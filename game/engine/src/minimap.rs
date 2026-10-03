//! Minimap + waypoints + full-screen map — backlog #6 (the JourneyMap gap).
//!
//! This module owns the **pure core** of the mapping feature: column sampling,
//! relief shading, colour quantisation and world→map projection. All of it is
//! free-function + plain-data so it unit-tests headless with no `World`, GPU or
//! egui dependency. The render/cache/UI layers (added in later increments) call
//! into these.
//!
//! Spec: `docs/foundations/2026-06-16-minimap-waypoints-map.md`.

use crate::block::BlockId;
use crate::world::World;
use ahash::AHashMap;
use std::cmp::Ordering;

/// Chunk-column footprint in blocks (16×16). Map tiles are one per column.
const TILE: i32 = crate::chunk::CHUNK_SIZE as i32;

/// One sampled map column: the topmost map-visible block and its world Y.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapCell {
    pub block: BlockId,
    pub height: i16,
}

/// Scan a single world column from `y_hi` down to `y_lo` (inclusive) for the
/// topmost block that should appear on the map. `get(y)` yields the block id at
/// height y; `is_surface(id)` decides whether a block forms a visible map
/// surface (the world layer passes "non-air", refined later to skip flimsy
/// decoration). Returns `(y, block)` of the first hit from the top, or `None`
/// if the whole column is empty/transparent.
pub fn topmost_map_block(
    y_hi: i32,
    y_lo: i32,
    get: impl Fn(i32) -> BlockId,
    is_surface: impl Fn(BlockId) -> bool,
) -> Option<(i32, BlockId)> {
    let mut y = y_hi;
    while y >= y_lo {
        let id = get(y);
        if is_surface(id) {
            return Some((y, id));
        }
        y -= 1;
    }
    None
}

/// Minecraft-style relief shading from a column's height versus the column to
/// its north (−Z). Higher than north → brighter; lower → darker; equal →
/// neutral. Returns a brightness multiplier.
pub fn relief_shade(height: i16, north_height: i16) -> f32 {
    match height.cmp(&north_height) {
        Ordering::Greater => 1.12,
        Ordering::Less => 0.84,
        Ordering::Equal => 1.0,
    }
}

/// Apply a shade multiplier to a 0..1 RGB block colour and quantise to `u8`
/// RGB, clamping into range so an over-bright relief value can never wrap.
pub fn cell_colour(base: [f32; 3], shade: f32) -> [u8; 3] {
    let q = |c: f32| ((c * shade).clamp(0.0, 1.0) * 255.0).round() as u8;
    [q(base[0]), q(base[1]), q(base[2])]
}

/// Project a world `(wx, wz)` onto map-pixel offsets from the map centre,
/// north-up. `blocks_per_pixel` > 0; +x → +px (east is right), +z → +py
/// (south is down — matches screen coordinates).
pub fn project_to_map(wx: f32, wz: f32, cx: f32, cz: f32, blocks_per_pixel: f32) -> (f32, f32) {
    ((wx - cx) / blocks_per_pixel, (wz - cz) / blocks_per_pixel)
}

/// Bake an `out_size`×`out_size` RGBA8 map image centred on world `(cx, cz)`,
/// `blocks_per_pixel` world blocks per pixel, north-up. `cell_at(wx, wz)` yields
/// the sampled column at a world block (`None` = unexplored → fully transparent
/// pixel); `colour_of(block)` yields its 0..1 base RGB. Each pixel is relief-
/// shaded by comparing its column height to the column one block north (−Z).
///
/// Pure: no `World`/GPU/egui — the cache layer supplies the two closures.
pub fn composite_rgba(
    out_size: usize,
    cx: f32,
    cz: f32,
    blocks_per_pixel: f32,
    cell_at: impl Fn(i32, i32) -> Option<MapCell>,
    colour_of: impl Fn(BlockId) -> [f32; 3],
) -> Vec<u8> {
    let mut buf = vec![0u8; out_size * out_size * 4]; // RGBA, default transparent
    let half = out_size as f32 / 2.0;
    for py in 0..out_size {
        for px in 0..out_size {
            // Pixel centre → the world block it samples (north-up).
            let wx = (cx + (px as f32 - half + 0.5) * blocks_per_pixel).floor() as i32;
            let wz = (cz + (py as f32 - half + 0.5) * blocks_per_pixel).floor() as i32;
            if let Some(cell) = cell_at(wx, wz) {
                // Relief vs the column one block north (−Z); fall back to this
                // column's own height (neutral) when the north column is unseen.
                let north = cell_at(wx, wz - 1).map(|c| c.height).unwrap_or(cell.height);
                let shade = relief_shade(cell.height, north);
                let [r, g, b] = cell_colour(colour_of(cell.block), shade);
                let i = (py * out_size + px) * 4;
                buf[i] = r;
                buf[i + 1] = g;
                buf[i + 2] = b;
                buf[i + 3] = 255;
            }
        }
    }
    buf
}

/// A baked map tile for one chunk-column `(cx, cz)`: 16×16 cells, row-major
/// (index = `lx + lz*16`), each the topmost map surface in that world column.
#[derive(Clone)]
pub struct MapTile {
    pub cells: [Option<MapCell>; 256],
}

/// Persistent map memory: one `MapTile` per explored chunk-column. Tiles are
/// built lazily and kept — explored terrain doesn't change once you leave — so
/// the whole explored map is the union of stored tiles. Near-player tiles are
/// rebuilt on a throttle so fresh edits show (see [`MinimapCache::update`]).
#[derive(Default)]
pub struct MinimapCache {
    tiles: AHashMap<(i32, i32), MapTile>,
}

/// Decide which chunk-columns to (re)build this refresh: every column within
/// `hot_radius` (Chebyshev) of `centre` — rebuilt even if cached, since the
/// player edits there — plus any *missing* column within `view_radius` (built
/// once). Returned nearest-first so a per-call build cap fills inward-out.
pub fn plan_refresh(
    centre: (i32, i32),
    view_radius: i32,
    hot_radius: i32,
    has_tile: impl Fn((i32, i32)) -> bool,
) -> Vec<(i32, i32)> {
    let (cx, cz) = centre;
    let mut out = Vec::new();
    for r in 0..=view_radius {
        // Walk ring `r` (Chebyshev), so the plan is nearest-first.
        for dx in -r..=r {
            for dz in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue; // interior of the square belongs to a smaller ring
                }
                let col = (cx + dx, cz + dz);
                if r <= hot_radius || !has_tile(col) {
                    out.push(col);
                }
            }
        }
    }
    out
}

/// Build a tile for chunk-column `(cx, cz)` from the live world. `None` when the
/// column is entirely air/unloaded, so an unloaded in-view column is retried on
/// a later refresh instead of being cached empty forever.
pub fn build_tile(world: &World, cx: i32, cz: i32) -> Option<MapTile> {
    let mut cells = [None; 256];
    let mut any = false;
    for lz in 0..TILE {
        for lx in 0..TILE {
            let wx = cx * TILE + lx;
            let wz = cz * TILE + lz;
            if let Some((y, b)) = world.highest_block(wx, wz) {
                cells[(lx + lz * TILE) as usize] = Some(MapCell {
                    block: b,
                    height: y as i16,
                });
                any = true;
            }
        }
    }
    if any {
        Some(MapTile { cells })
    } else {
        None
    }
}

impl MinimapCache {
    /// The sampled column at world block `(wx, wz)`, or `None` if unexplored.
    pub fn cell_at(&self, wx: i32, wz: i32) -> Option<MapCell> {
        let cc = (wx.div_euclid(TILE), wz.div_euclid(TILE));
        let lx = wx.rem_euclid(TILE) as usize;
        let lz = wz.rem_euclid(TILE) as usize;
        self.tiles.get(&cc).and_then(|t| t.cells[lx + lz * 16])
    }

    /// Number of explored tiles held. No production caller yet (the render/UI
    /// layer this feeds hasn't landed) — exercised by the tests below.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Refresh the cache around `centre` chunk-column from the live world.
    /// `cap` bounds how many *new* columns are built this call (hot rebuilds
    /// are always done) so first sight of a large area can't hitch a frame.
    pub fn update(
        &mut self,
        world: &World,
        centre: (i32, i32),
        view_radius: i32,
        hot_radius: i32,
        cap: usize,
    ) {
        let plan = plan_refresh(centre, view_radius, hot_radius, |cc| {
            self.tiles.contains_key(&cc)
        });
        let mut built_new = 0usize;
        for cc in plan {
            let is_new = !self.tiles.contains_key(&cc);
            if is_new {
                if built_new >= cap {
                    continue;
                }
                built_new += 1;
            }
            if let Some(tile) = build_tile(world, cc.0, cc.1) {
                self.tiles.insert(cc, tile);
            }
        }
    }
}

/// Minimap texture resolution (square, px) — the composited image, drawn scaled
/// to [`MINIMAP_DISPLAY_PTS`] on screen.
pub const MINIMAP_TEX_PX: usize = 160;
/// On-screen minimap size in egui points (square).
pub const MINIMAP_DISPLAY_PTS: f32 = 148.0;
/// Resample + re-upload the minimap at most this often (ticks; 20 TPS ⇒ ~0.5 s).
pub const MINIMAP_REFRESH_TICKS: u64 = 10;
/// New tiles built per refresh (bounds first-sight cost so a big unexplored area
/// can't hitch a frame; cached tiles persist, so it fills in over a few frames).
pub const MINIMAP_NEW_TILES_PER_REFRESH: usize = 24;

/// Per-session render state for the minimap, kept on `GameState`. The cache is
/// the persistent explored-map memory; `texture` is the last baked image; the
/// throttle fields gate how often we resample + re-upload.
#[derive(Default)]
pub struct MinimapView {
    pub cache: MinimapCache,
    pub texture: Option<egui::TextureHandle>,
    pub last_refresh_tick: u64,
    pub last_centre: Option<(i32, i32)>,
}

/// Full-screen map texture resolution (square, px).
pub const MAP_TEX_PX: usize = 512;
/// Default / min / max zoom for the full-screen map (world blocks per pixel) —
/// more zoomed out than the minimap so the explored region reads at a glance.
pub const MAP_DEFAULT_BPP: f32 = 3.0;
pub const MAP_MIN_BPP: f32 = 1.0;
pub const MAP_MAX_BPP: f32 = 12.0;

/// Full-screen explorable map state (#6). Centred on a world `(x, z)`, zoomable
/// (blocks per pixel) and pannable. The texture is recomposited only when
/// `dirty` (on open / pan / zoom), not every frame.
pub struct MapScreen {
    pub open: bool,
    pub centre: (f32, f32),
    pub bpp: f32,
    pub texture: Option<egui::TextureHandle>,
    pub dirty: bool,
}

impl Default for MapScreen {
    fn default() -> Self {
        Self {
            open: false,
            centre: (0.0, 0.0),
            bpp: MAP_DEFAULT_BPP,
            texture: None,
            dirty: false,
        }
    }
}

/// Clamp a proposed full-screen-map zoom into the allowed range.
pub fn clamp_map_bpp(bpp: f32) -> f32 {
    bpp.clamp(MAP_MIN_BPP, MAP_MAX_BPP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_map_bpp_bounds_zoom() {
        assert_eq!(clamp_map_bpp(0.1), MAP_MIN_BPP);
        assert_eq!(clamp_map_bpp(100.0), MAP_MAX_BPP);
        assert_eq!(clamp_map_bpp(4.0), 4.0);
    }

    // ---- topmost_map_block ----------------------------------------------

    #[test]
    fn topmost_returns_first_surface_from_the_top() {
        // Column: air above y=5, stone (id 1) at y=5 and below.
        let get = |y: i32| -> BlockId {
            if y <= 5 {
                1
            } else {
                0
            }
        };
        let hit = topmost_map_block(255, 0, get, |id| id != 0);
        assert_eq!(hit, Some((5, 1)), "should find the stone top at y=5");
    }

    #[test]
    fn topmost_returns_none_for_an_empty_column() {
        let hit = topmost_map_block(255, 0, |_| 0, |id| id != 0);
        assert_eq!(hit, None, "all-air column has no map surface");
    }

    #[test]
    fn topmost_respects_the_surface_predicate() {
        // A column that is leaves (id 9) on top of stone (id 1); the predicate
        // skips leaves, so the map should show the stone underneath at y=3.
        let get = |y: i32| -> BlockId {
            if y >= 4 && y <= 6 {
                9
            } else if y <= 3 {
                1
            } else {
                0
            }
        };
        let hit = topmost_map_block(255, 0, get, |id| id == 1);
        assert_eq!(hit, Some((3, 1)));
    }

    // ---- relief_shade ----------------------------------------------------

    #[test]
    fn relief_brightens_when_higher_than_north() {
        assert!(relief_shade(70, 64) > 1.0);
    }

    #[test]
    fn relief_darkens_when_lower_than_north() {
        assert!(relief_shade(60, 64) < 1.0);
    }

    #[test]
    fn relief_is_neutral_on_flat_ground() {
        assert_eq!(relief_shade(64, 64), 1.0);
    }

    // ---- cell_colour -----------------------------------------------------

    #[test]
    fn cell_colour_neutral_shade_quantises_midgrey() {
        // 0.5 * 1.0 * 255 = 127.5 → rounds to 128.
        assert_eq!(cell_colour([0.5, 0.5, 0.5], 1.0), [128, 128, 128]);
    }

    #[test]
    fn cell_colour_clamps_overbright_to_255() {
        assert_eq!(cell_colour([0.8, 0.8, 0.8], 4.0), [255, 255, 255]);
    }

    #[test]
    fn cell_colour_dark_shade_floors_at_zero() {
        assert_eq!(cell_colour([0.5, 0.5, 0.5], 0.0), [0, 0, 0]);
    }

    // ---- project_to_map --------------------------------------------------

    #[test]
    fn project_centre_maps_to_origin() {
        assert_eq!(project_to_map(100.0, 200.0, 100.0, 200.0, 2.0), (0.0, 0.0));
    }

    #[test]
    fn project_scales_by_blocks_per_pixel_and_keeps_axis_signs() {
        // 10 blocks east at 2 blocks/px → +5 px right; 8 blocks south → +4 px down.
        let (px, py) = project_to_map(110.0, 208.0, 100.0, 200.0, 2.0);
        assert_eq!((px, py), (5.0, 4.0));
    }

    // ---- composite_rgba --------------------------------------------------

    /// Alpha channel of pixel (px, py) in an RGBA buffer of width `w`.
    fn alpha(buf: &[u8], w: usize, px: usize, py: usize) -> u8 {
        buf[(py * w + px) * 4 + 3]
    }
    fn rgb(buf: &[u8], w: usize, px: usize, py: usize) -> [u8; 3] {
        let i = (py * w + px) * 4;
        [buf[i], buf[i + 1], buf[i + 2]]
    }

    #[test]
    fn composite_unexplored_is_fully_transparent() {
        let buf = composite_rgba(4, 0.0, 0.0, 1.0, |_, _| None, |_| [1.0, 1.0, 1.0]);
        assert_eq!(buf.len(), 4 * 4 * 4);
        for py in 0..4 {
            for px in 0..4 {
                assert_eq!(alpha(&buf, 4, px, py), 0, "unexplored pixel must be transparent");
            }
        }
    }

    #[test]
    fn composite_paints_known_cells_opaque_with_block_colour() {
        // Flat world of one block type (id 1), colour pure red, all height 64.
        // Every pixel should be opaque red at neutral shade (flat ⇒ shade 1.0).
        let buf = composite_rgba(
            3,
            0.0,
            0.0,
            1.0,
            |_, _| Some(MapCell { block: 1, height: 64 }),
            |_| [1.0, 0.0, 0.0],
        );
        for py in 0..3 {
            for px in 0..3 {
                assert_eq!(alpha(&buf, 3, px, py), 255);
                assert_eq!(rgb(&buf, 3, px, py), [255, 0, 0]);
            }
        }
    }

    #[test]
    fn composite_relief_brightens_a_ridge_above_its_north_neighbour() {
        // Column at wz = 0 sits one block higher than the column to its north
        // (wz = -1). With a grey base, the ridge pixel should be brighter than
        // a flat reference pixel. blocks_per_pixel = 1, out_size = 1 ⇒ the lone
        // pixel samples world (0,0); north is (0,-1).
        let ridge = composite_rgba(
            1,
            0.0,
            0.0,
            1.0,
            |_, wz| {
                let h = if wz >= 0 { 65 } else { 64 };
                Some(MapCell { block: 1, height: h })
            },
            |_| [0.5, 0.5, 0.5],
        );
        let flat = composite_rgba(
            1,
            0.0,
            0.0,
            1.0,
            |_, _| Some(MapCell { block: 1, height: 64 }),
            |_| [0.5, 0.5, 0.5],
        );
        assert!(
            ridge[0] > flat[0],
            "ridge ({}) should be brighter than flat ({})",
            ridge[0],
            flat[0]
        );
    }

    // ---- plan_refresh ----------------------------------------------------

    #[test]
    fn plan_hot_radius_zero_rebuilds_only_the_centre() {
        // hot_radius is an inclusive Chebyshev radius: 0 = the centre column is
        // hot (the player stands there and edits it, so it always rebuilds);
        // with everything else cached there is no further work.
        let plan = plan_refresh((0, 0), 2, 0, |_| true);
        assert_eq!(plan, vec![(0, 0)]);
    }

    #[test]
    fn plan_nothing_cached_builds_the_whole_view_square() {
        // view_radius 1 ⇒ 3×3 = 9 columns, all missing.
        let plan = plan_refresh((0, 0), 1, 0, |_| false);
        assert_eq!(plan.len(), 9);
        assert!(plan.contains(&(0, 0)));
        assert!(plan.contains(&(-1, -1)));
        assert!(plan.contains(&(1, 1)));
    }

    #[test]
    fn plan_rebuilds_hot_columns_even_when_cached() {
        // Everything cached, but hot_radius 1 ⇒ the 3×3 around centre rebuild.
        let plan = plan_refresh((5, 5), 3, 1, |_| true);
        assert_eq!(plan.len(), 9, "the hot 3×3 always rebuilds");
        assert!(plan.contains(&(5, 5)));
        assert!(plan.contains(&(6, 4)));
        assert!(!plan.contains(&(8, 5)), "outside hot + already cached ⇒ skip");
    }

    #[test]
    fn plan_is_nearest_first() {
        let plan = plan_refresh((0, 0), 2, 0, |_| false);
        assert_eq!(plan[0], (0, 0), "centre comes first");
        // The last entry must be on the outer ring (Chebyshev distance 2).
        let (lx, lz) = *plan.last().unwrap();
        assert_eq!(lx.abs().max(lz.abs()), 2);
    }

    // ---- cell_at ---------------------------------------------------------

    #[test]
    fn cell_at_reads_back_an_inserted_tile() {
        let mut cache = MinimapCache::default();
        let mut cells = [None; 256];
        // local (lx=1, lz=2) ⇒ index 1 + 2*16 = 33, in chunk-column (0,0).
        cells[33] = Some(MapCell { block: 7, height: 70 });
        cache.tiles.insert((0, 0), MapTile { cells });
        assert_eq!(cache.cell_at(1, 2), Some(MapCell { block: 7, height: 70 }));
        assert_eq!(cache.cell_at(0, 0), None, "an unset cell reads None");
        assert_eq!(cache.cell_at(999, 999), None, "no tile ⇒ None");
    }

    #[test]
    fn cell_at_handles_negative_world_coords() {
        let mut cache = MinimapCache::default();
        let mut cells = [None; 256];
        // World (-1, -1) ⇒ chunk-column (-1,-1), local (15,15) ⇒ index 255.
        cells[255] = Some(MapCell { block: 3, height: 64 });
        cache.tiles.insert((-1, -1), MapTile { cells });
        assert_eq!(cache.cell_at(-1, -1), Some(MapCell { block: 3, height: 64 }));
    }

    // ---- update (integration over a real World) --------------------------

    #[test]
    fn update_samples_the_world_into_tiles() {
        let mut world = World::new();
        // A grass pillar top at (3, 40, 5) over dirt — column (cx,cz)=(0,0).
        world.set_block(3, 39, 5, crate::block::DIRT);
        world.set_block(3, 40, 5, crate::block::GRASS);
        let mut cache = MinimapCache::default();
        cache.update(&world, (0, 0), 1, 0, 64);
        let cell = cache.cell_at(3, 5).expect("column (3,5) sampled");
        assert_eq!(cell.block, crate::block::GRASS);
        assert_eq!(cell.height, 40);
    }

    #[test]
    fn update_skips_entirely_empty_columns() {
        // An empty world has no blocks, so no tile should be cached (retry
        // later) rather than poisoning the cache with all-None tiles.
        let world = World::new();
        let mut cache = MinimapCache::default();
        cache.update(&world, (0, 0), 1, 0, 64);
        assert!(cache.is_empty(), "no surface anywhere ⇒ no tiles cached");
    }

    #[test]
    fn update_caps_new_tiles_per_call() {
        // Fill a wide area so many columns have surface, then cap new builds.
        let mut world = World::new();
        for x in -40..40 {
            for z in -40..40 {
                world.set_block(x, 30, z, crate::block::STONE);
            }
        }
        let mut cache = MinimapCache::default();
        cache.update(&world, (0, 0), 3, 0, 4); // view 7×7=49 columns, cap 4 new
        assert!(cache.len() <= 4, "at most `cap` new tiles built (got {})", cache.len());
    }
}

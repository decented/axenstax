//! Underworld C1 — Ravines.
//!
//! A ravine is a long, narrow canyon that splits open at the surface and cuts
//! deep into the stone, exposing ore in its walls and (for the deepest ones) a
//! thread of lava at the bottom. They're rare and dramatic: ~1 candidate per
//! 40×40 chunks, and under half of those actually spawn, so finding one is an
//! event.
//!
//! ## Generation model
//!
//! Mirrors the village / brigand-hideout placers: a deterministic virtual grid
//! (`RAVINE_GRID` chunks per cell) where each cell *may* host a ravine derived
//! purely from `(world_seed, gx, gz)`. `place_ravines_for_column` carves only
//! the slice of any nearby ravine that falls inside the current chunk-column,
//! so a ravine straddles chunk boundaries correctly with no global pre-pass and
//! identical results regardless of visit order.
//!
//! The geometry helpers ([`half_width_at`], [`project`], [`is_inside`]) are
//! pure and unit-tested without a `World`; `place_ravines_for_column` is the
//! thin block-setting shell over them.
//!
//! Eventual crate home: `genesis_worldgen`.

use crate::biome::{BiomeGenerator, SEA_LEVEL};
use crate::block::{AIR, LAVA};
use crate::chunk::CHUNK_SIZE;
use crate::world::World;

/// Grid cell size in chunks. One ravine *candidate* per 40×40 chunks.
pub const RAVINE_GRID: i32 = 40;

/// Of the candidate cells, this percentage actually host a ravine.
const RAVINE_SPAWN_PERCENT: u32 = 45;

/// Smallest floor a ravine bottoms out at — keeps clear of bedrock and the
/// lowest cave layer.
const MIN_FLOOR_Y: i32 = 8;

/// A concrete ravine instance, derived from a grid cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RavineLayout {
    /// World-space centre of the ravine (its midpoint along the axis).
    pub anchor_x: i32,
    pub anchor_z: i32,
    /// Orientation of the long axis in the xz plane (radians).
    pub angle: f32,
    /// Half the ravine length along the axis (blocks).
    pub half_len: f32,
    /// Widest half-width (at the rim), before tapering (blocks).
    pub max_half_width: f32,
    /// Reference top used for the vertical taper (≈ surface at the anchor).
    pub top_y: i32,
    /// Floor of the ravine.
    pub floor_y: i32,
    /// Whether a thread of lava pools along the very bottom.
    pub lava_floor: bool,
}

/// SplitMix-style 64→32 hash for deterministic, replay-stable layout.
fn hash3(seed: u32, gx: i32, gz: i32, salt: u32) -> u32 {
    let mut h = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (gx as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= (gz as i64 as u64).rotate_left(32).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= salt as u64;
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h & 0xFFFF_FFFF) as u32
}

fn unit(seed: u32, gx: i32, gz: i32, salt: u32) -> f32 {
    hash3(seed, gx, gz, salt) as f32 / u32::MAX as f32
}

/// Derive the ravine (if any) hosted by grid cell `(gx, gz)`. Pure +
/// deterministic.
pub fn layout_for_ravine_cell(
    seed: u32,
    gx: i32,
    gz: i32,
    biome_gen: &BiomeGenerator,
) -> Option<RavineLayout> {
    if hash3(seed, gx, gz, 1) % 100 >= RAVINE_SPAWN_PERCENT {
        return None;
    }
    let cell_blocks = RAVINE_GRID * CHUNK_SIZE as i32;
    let cell_origin_x = gx * cell_blocks;
    let cell_origin_z = gz * cell_blocks;
    // Keep the anchor inside the central half of the cell so the body rarely
    // overflows into a non-neighbour cell.
    let margin = cell_blocks / 4;
    let span = cell_blocks - 2 * margin;
    let anchor_x = cell_origin_x + margin + (hash3(seed, gx, gz, 2) % span as u32) as i32;
    let anchor_z = cell_origin_z + margin + (hash3(seed, gx, gz, 3) % span as u32) as i32;

    let surface = biome_gen.terrain_height(anchor_x, anchor_z);
    // Land only — keep ravines out of the ocean for v1 (no underwater carving).
    if surface < SEA_LEVEL + 2 {
        return None;
    }

    let angle = unit(seed, gx, gz, 4) * std::f32::consts::PI; // 0..π (axis is symmetric)
    let half_len = 28.0 + unit(seed, gx, gz, 5) * 28.0; // 28..56 blocks
    let max_half_width = 3.0 + unit(seed, gx, gz, 6) * 2.0; // 3..5 (width 6..10)
    let depth = 30 + (hash3(seed, gx, gz, 7) % 22) as i32; // 30..51 deep
    let floor_y = (surface - depth).max(MIN_FLOOR_Y);
    let lava_floor = floor_y <= 16 && hash3(seed, gx, gz, 8).is_multiple_of(2);

    Some(RavineLayout {
        anchor_x,
        anchor_z,
        angle,
        half_len,
        max_half_width,
        top_y: surface,
        floor_y,
        lava_floor,
    })
}

/// Project world `(wx, wz)` into ravine-local `(along, perp)` coordinates,
/// where `along` runs down the ravine's long axis and `perp` is the sideways
/// offset. Block centres are used.
pub fn project(layout: &RavineLayout, wx: i32, wz: i32) -> (f32, f32) {
    let dx = wx as f32 + 0.5 - layout.anchor_x as f32;
    let dz = wz as f32 + 0.5 - layout.anchor_z as f32;
    let (s, c) = layout.angle.sin_cos();
    let along = dx * c + dz * s;
    let perp = -dx * s + dz * c;
    (along, perp)
}

/// Half-width of the ravine slot at axis position `along` and height `y`.
/// Narrows toward the tips (`along` near ±half_len) and toward the floor, with
/// a gentle sine wobble so the walls aren't a flat slab. Returns 0 outside the
/// length.
pub fn half_width_at(layout: &RavineLayout, along: f32, y: i32) -> f32 {
    let a = along.abs();
    if a > layout.half_len {
        return 0.0;
    }
    // End taper — quadratic falloff to a point at each tip.
    let end = (1.0 - (a / layout.half_len).powi(2)).max(0.0);
    // Vertical taper — wide at the rim, ~⅓ width at the floor.
    let span = (layout.top_y - layout.floor_y).max(1) as f32;
    let frac = ((y - layout.floor_y) as f32 / span).clamp(0.0, 1.0);
    let vert = 0.35 + 0.65 * frac.sqrt();
    // Wobble — breaks up the straight walls.
    let wobble = 1.0 + 0.22 * (along * 0.17).sin();
    layout.max_half_width * end * vert * wobble
}

/// Whether the ravine slot includes world cell `(wx, wy, wz)`. No production
/// caller — `carve_column` below inlines the identical `project` +
/// `half_width_at` check rather than delegating here, so the two could drift.
/// Exercised by the tests below.
#[cfg_attr(not(test), allow(dead_code))]
pub fn is_inside(layout: &RavineLayout, wx: i32, wy: i32, wz: i32) -> bool {
    if wy < layout.floor_y || wy > layout.top_y {
        return false;
    }
    let (along, perp) = project(layout, wx, wz);
    perp.abs() <= half_width_at(layout, along, wy)
}

/// Carve every nearby ravine's contribution to this chunk-column.
pub fn place_ravines_for_column(
    world: &mut World,
    cx: i32,
    cz: i32,
    biome_gen: &BiomeGenerator,
    world_seed: u32,
) {
    let cs = CHUNK_SIZE as i32;
    let col_min_x = cx * cs;
    let col_min_z = cz * cs;
    let col_max_x = col_min_x + cs - 1;
    let col_max_z = col_min_z + cs - 1;

    let gx_centre = cx.div_euclid(RAVINE_GRID);
    let gz_centre = cz.div_euclid(RAVINE_GRID);

    for dgz in -1..=1 {
        for dgx in -1..=1 {
            let Some(layout) =
                layout_for_ravine_cell(world_seed, gx_centre + dgx, gz_centre + dgz, biome_gen)
            else {
                continue;
            };
            // Cull cells whose body can't reach this column.
            let reach = (layout.half_len + layout.max_half_width).ceil() as i32 + 1;
            if (layout.anchor_x + reach) < col_min_x
                || (layout.anchor_x - reach) > col_max_x
                || (layout.anchor_z + reach) < col_min_z
                || (layout.anchor_z - reach) > col_max_z
            {
                continue;
            }
            carve_column(world, &layout, biome_gen, col_min_x, col_min_z, col_max_x, col_max_z);
        }
    }
}

fn carve_column(
    world: &mut World,
    layout: &RavineLayout,
    biome_gen: &BiomeGenerator,
    col_min_x: i32,
    col_min_z: i32,
    col_max_x: i32,
    col_max_z: i32,
) {
    for wz in col_min_z..=col_max_z {
        for wx in col_min_x..=col_max_x {
            let (along, perp) = project(layout, wx, wz);
            if along.abs() > layout.half_len || perp.abs() > layout.max_half_width + 1.0 {
                continue; // fast reject — this column cell is outside the body
            }
            // Open the canyon at the LOCAL surface so its rim follows the
            // terrain instead of a flat anchor height.
            let local_top = biome_gen.terrain_height(wx, wz).min(layout.top_y);
            for wy in layout.floor_y..=local_top {
                if perp.abs() <= half_width_at(layout, along, wy) {
                    if layout.lava_floor && wy <= layout.floor_y + 1 {
                        world.set_block(wx, wy, wz, LAVA);
                    } else {
                        world.set_block(wx, wy, wz, AIR);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any_ravine(seed: u32) -> (RavineLayout, BiomeGenerator) {
        let bg = BiomeGenerator::new(seed);
        // Scan grid cells until we find one hosting a ravine.
        for gx in 0..200 {
            for gz in 0..3 {
                if let Some(l) = layout_for_ravine_cell(seed, gx, gz, &bg) {
                    return (l, bg);
                }
            }
        }
        panic!("no ravine found in 600 cells for seed {seed} — spawn rate too low?");
    }

    #[test]
    fn layout_is_deterministic_for_a_cell() {
        let bg = BiomeGenerator::new(7);
        let a = layout_for_ravine_cell(7, 3, 4, &bg);
        let b = layout_for_ravine_cell(7, 3, 4, &bg);
        assert_eq!(a, b, "same seed+cell must give the same ravine");
    }

    #[test]
    fn ravines_spawn_at_a_sensible_rate() {
        // Sample a 2-D patch (not one strip, which can be all ocean). The hash
        // gate admits 45% of cells; the land gate only *reduces* that, so the
        // observed rate must sit above zero (land cells host ravines) and at or
        // below the 45% ceiling.
        let bg = BiomeGenerator::new(123);
        let mut present = 0;
        let side = 40;
        let n = side * side;
        for gx in 0..side {
            for gz in 0..side {
                if layout_for_ravine_cell(123, gx, gz, &bg).is_some() {
                    present += 1;
                }
            }
        }
        assert!(present > n / 50, "ravines too rare: {present}/{n}");
        assert!(present <= (n * 47) / 100, "ravines exceed the spawn-gate ceiling: {present}/{n}");
    }

    #[test]
    fn centre_floor_is_carved_walls_are_not() {
        let (l, _bg) = any_ravine(20260621);
        // Dead centre at mid-height is inside the slot.
        let mid_y = (l.floor_y + l.top_y) / 2;
        assert!(is_inside(&l, l.anchor_x, mid_y, l.anchor_z), "centre should be hollow");
        // Far out to the side (beyond max width) is solid wall.
        let (s, c) = l.angle.sin_cos();
        // Step 20 blocks perpendicular to the axis from the anchor.
        let off_x = l.anchor_x + (-s * 20.0) as i32;
        let off_z = l.anchor_z + (c * 20.0) as i32;
        assert!(!is_inside(&l, off_x, mid_y, off_z), "20 blocks to the side is wall");
    }

    #[test]
    fn slot_narrows_toward_the_floor() {
        let (l, _bg) = any_ravine(987654);
        let wide = half_width_at(&l, 0.0, l.top_y);
        let narrow = half_width_at(&l, 0.0, l.floor_y);
        assert!(wide > narrow, "ravine should be wider at the rim ({wide}) than the floor ({narrow})");
    }

    #[test]
    fn nothing_carved_below_floor_or_above_top() {
        let (l, _bg) = any_ravine(555);
        assert!(!is_inside(&l, l.anchor_x, l.floor_y - 1, l.anchor_z));
        assert!(!is_inside(&l, l.anchor_x, l.top_y + 1, l.anchor_z));
    }

    #[test]
    fn carving_a_real_column_hollows_the_centre() {
        let seed = 424242;
        let (l, bg) = any_ravine(seed);
        let mut world = World::new();
        // Generate the chunk-columns the ravine spans so there's stone to carve.
        let cs = CHUNK_SIZE as i32;
        let cx0 = l.anchor_x.div_euclid(cs);
        let cz0 = l.anchor_z.div_euclid(cs);
        for dcx in -2..=2 {
            for dcz in -2..=2 {
                world.generate_column(cx0 + dcx, cz0 + dcz, &bg);
            }
        }
        // Re-run the ravine placer over the anchor's column (idempotent), then
        // assert the centre column is now air somewhere in the slot.
        place_ravines_for_column(&mut world, cx0, cz0, &bg, seed);
        let mid_y = (l.floor_y + l.top_y) / 2;
        // generate_column already invoked the placer via the worldgen path is
        // NOT wired in this unit test, so the explicit call above did the carve.
        let here = world.get_block(l.anchor_x, mid_y, l.anchor_z);
        assert_eq!(here, AIR, "ravine centre at y={mid_y} should be carved to air");
    }
}

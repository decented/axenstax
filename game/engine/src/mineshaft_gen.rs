//! Underworld C2 — Abandoned mineshafts.
//!
//! A buried network of timber-framed corridors radiating from a central
//! junction, with loot chests tucked along the arms. Like ravines they're
//! placed on a deterministic virtual grid and built column-by-column, so a
//! shaft straddles chunk boundaries cleanly and is identical regardless of
//! visit order.
//!
//! Each arm is an axis-aligned corridor: a 3-wide × 2-tall air tunnel with a
//! plank walkway, fence-post + plank-beam support frames every few blocks, and
//! a loot chest about two-thirds of the way along. Corridors sit underground
//! (well below sea level, above the lava-cave layer), so digging into one is a
//! genuine "what's down here?" discovery.
//!
//! The layout + loot helpers are pure and unit-tested; the column placer is
//! the block-setting shell.
//!
//! Eventual crate home: `genesis_worldgen`.

use crate::biome::BiomeGenerator;
use crate::block::{AIR, CHEST, FENCE_POST, OAK_PLANKS};
use crate::chest::ChestData;
use crate::chunk::CHUNK_SIZE;
use crate::item::{ItemStack, MaterialId};
use crate::world::World;

/// Grid cell size in chunks. One mineshaft *candidate* per 24×24 chunks.
pub const MINESHAFT_GRID: i32 = 24;
/// Percentage of candidate cells that actually host a shaft.
const MINESHAFT_SPAWN_PERCENT: u32 = 40;
/// Blocks between support frames along a corridor.
const SUPPORT_SPACING: i32 = 5;

/// The four axis-aligned arm directions.
const ARM_DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

#[derive(Clone, Debug, PartialEq)]
pub struct MineshaftLayout {
    /// Junction centre (world space). Corridors run at this y.
    pub ax: i32,
    pub ay: i32,
    pub az: i32,
    /// Arms: `(dir_index_into_ARM_DIRS, length_in_blocks)`.
    pub arms: Vec<(usize, i32)>,
}

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

/// The mineshaft (if any) hosted by grid cell `(gx, gz)`. Pure + deterministic.
pub fn layout_for_mineshaft_cell(
    seed: u32,
    gx: i32,
    gz: i32,
    biome_gen: &BiomeGenerator,
) -> Option<MineshaftLayout> {
    if hash3(seed, gx, gz, 1) % 100 >= MINESHAFT_SPAWN_PERCENT {
        return None;
    }
    let cell_blocks = MINESHAFT_GRID * CHUNK_SIZE as i32;
    let cell_origin_x = gx * cell_blocks;
    let cell_origin_z = gz * cell_blocks;
    let margin = cell_blocks / 4;
    let span = (cell_blocks - 2 * margin).max(1) as u32;
    let ax = cell_origin_x + margin + (hash3(seed, gx, gz, 2) % span) as i32;
    let az = cell_origin_z + margin + (hash3(seed, gx, gz, 3) % span) as i32;

    // Sit the junction underground: below the surface, above the lava-cave
    // band. Clamp so even a low surface keeps the shaft buried.
    let surface = biome_gen.terrain_height(ax, az);
    let ceiling = (surface - 12).min(40);
    if ceiling < 16 {
        return None; // surface too low (ocean / deep valley) to bury a shaft
    }
    let ay = 16 + (hash3(seed, gx, gz, 4) % (ceiling - 16 + 1) as u32) as i32;

    // 2–4 arms, each a distinct direction, 18–38 blocks long.
    let arm_count = 2 + (hash3(seed, gx, gz, 5) % 3) as usize; // 2..4
    let mut arms = Vec::with_capacity(arm_count);
    for (i, dir) in ARM_DIRS.iter().enumerate().take(arm_count) {
        let _ = dir;
        let len = 18 + (hash3(seed, gx, gz, 10 + i as u32) % 21) as i32; // 18..38
        arms.push((i, len));
    }

    Some(MineshaftLayout { ax, ay, az, arms })
}

/// Deterministic loot for a chest seeded by its world position. Returns a
/// 27-slot vector (the default chest size) with a handful of filled slots.
pub fn loot_for_chest(seed: u32) -> Vec<Option<ItemStack>> {
    // (item-builder, min, max). Closures keep the table compact.
    let pool: [(fn(u8) -> ItemStack, u8, u8); 10] = [
        (|n| ItemStack::new_block(OAK_PLANKS, n), 4, 16),
        (|n| ItemStack::new_material(MaterialId::Coal, n), 2, 8),
        (|n| ItemStack::new_material(MaterialId::RawIron, n), 1, 4),
        (|n| ItemStack::new_material(MaterialId::IronIngot, n), 1, 3),
        (|n| ItemStack::new_material(MaterialId::Bone, n), 1, 4),
        (|n| ItemStack::new_material(MaterialId::Bread, n), 1, 3),
        (|n| ItemStack::new_material(MaterialId::Stick, n), 2, 6),
        (|n| ItemStack::new_material(MaterialId::Arrow, n), 4, 12),
        (|n| ItemStack::new_material(MaterialId::Wheat, n), 1, 5),
        // Rare: a single diamond, only sometimes (handled below).
        (|n| ItemStack::new_material(MaterialId::Diamond, n), 1, 1),
    ];
    let mut slots: Vec<Option<ItemStack>> = vec![None; crate::chest::CHEST_SLOTS];
    // 3–5 stacks.
    let n_stacks = 3 + (hash3(seed, 0, 0, 100) % 3) as usize;
    for k in 0..n_stacks {
        let mut idx = (hash3(seed, 0, 0, 200 + k as u32) % pool.len() as u32) as usize;
        // Gate the diamond slot to ~1-in-6 so it stays a treat.
        if idx == pool.len() - 1 && !hash3(seed, 0, 0, 300 + k as u32).is_multiple_of(6) {
            idx = (hash3(seed, 0, 0, 400 + k as u32) % (pool.len() as u32 - 1)) as usize;
        }
        let (build, lo, hi) = pool[idx];
        let count = lo + (hash3(seed, 0, 0, 500 + k as u32) % (hi - lo + 1) as u32) as u8;
        // First-fit into an empty slot.
        if let Some(s) = slots.iter_mut().find(|s| s.is_none()) {
            *s = Some(build(count));
        }
    }
    slots
}

/// World position of the loot chest on a given arm (about 60% along), and the
/// per-chest loot seed. Pure — testable without a `World`.
pub fn chest_site(layout: &MineshaftLayout, arm_idx: usize) -> Option<(i32, i32, i32)> {
    let &(dir_i, len) = layout.arms.get(arm_idx)?;
    let (dx, dz) = ARM_DIRS[dir_i];
    let s = (len * 3) / 5;
    Some((layout.ax + dx * s, layout.ay, layout.az + dz * s))
}

/// Build every nearby mineshaft's contribution to this chunk-column.
pub fn place_mineshafts_for_column(
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

    let gx_centre = cx.div_euclid(MINESHAFT_GRID);
    let gz_centre = cz.div_euclid(MINESHAFT_GRID);

    for dgz in -1..=1 {
        for dgx in -1..=1 {
            let Some(layout) =
                layout_for_mineshaft_cell(world_seed, gx_centre + dgx, gz_centre + dgz, biome_gen)
            else {
                continue;
            };
            // Reach = longest arm + a block of slack.
            let reach = layout.arms.iter().map(|&(_, l)| l).max().unwrap_or(0) + 2;
            if (layout.ax + reach) < col_min_x
                || (layout.ax - reach) > col_max_x
                || (layout.az + reach) < col_min_z
                || (layout.az - reach) > col_max_z
            {
                continue;
            }
            build_in_column(
                world, &layout, col_min_x, col_min_z, col_max_x, col_max_z,
            );
        }
    }
}

/// Is `(x, z)` inside the current column's bounds?
fn in_col(x: i32, z: i32, xmin: i32, zmin: i32, xmax: i32, zmax: i32) -> bool {
    x >= xmin && x <= xmax && z >= zmin && z <= zmax
}

fn build_in_column(
    world: &mut World,
    layout: &MineshaftLayout,
    xmin: i32,
    zmin: i32,
    xmax: i32,
    zmax: i32,
) {
    let y = layout.ay;
    for (arm_idx, &(dir_i, len)) in layout.arms.iter().enumerate() {
        let (dx, dz) = ARM_DIRS[dir_i];
        // Perpendicular (width) axis for an axis-aligned arm.
        let (pdx, pdz) = (dz, dx);
        let chest = chest_site(layout, arm_idx);
        for s in 0..=len {
            let cx0 = layout.ax + dx * s;
            let cz0 = layout.az + dz * s;
            let on_support = s % SUPPORT_SPACING == 0 && s > 0;
            for w in -1..=1 {
                let x = cx0 + pdx * w;
                let z = cz0 + pdz * w;
                if !in_col(x, z, xmin, zmin, xmax, zmax) {
                    continue;
                }
                // Plank walkway under the whole 3-wide floor.
                world.set_block(x, y - 1, z, OAK_PLANKS);
                // Hollow the 2-tall tunnel.
                world.set_block(x, y, z, AIR);
                world.set_block(x, y + 1, z, AIR);
                if on_support {
                    if w == 0 {
                        // Plank beam across the ceiling at the centre.
                        world.set_block(x, y + 2, z, OAK_PLANKS);
                    } else {
                        // Fence posts frame the two sides, floor to ceiling.
                        world.set_block(x, y, z, FENCE_POST);
                        world.set_block(x, y + 1, z, FENCE_POST);
                        world.set_block(x, y + 2, z, OAK_PLANKS);
                    }
                }
            }
        }
        // Loot chest — placed once, in whichever column owns its cell. Don't
        // overwrite an existing chest (protects a looted chest from a re-gen).
        if let Some((cxp, cyp, czp)) = chest
            && in_col(cxp, czp, xmin, zmin, xmax, zmax)
                && world.chest_at((cxp, cyp, czp)).is_none()
            {
                world.set_block(cxp, cyp, czp, CHEST);
                let loot_seed = hash3((layout.ax as u32) ^ 0x5151, cxp, czp, arm_idx as u32);
                let data = ChestData {
                    slots: loot_for_chest(loot_seed),
                    tier: crate::chest::ChestTier::Wood,
                };
                world.insert_chest((cxp, cyp, czp), data);
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::SEA_LEVEL;

    fn any_mineshaft(seed: u32) -> (MineshaftLayout, BiomeGenerator) {
        let bg = BiomeGenerator::new(seed);
        for gx in 0..300 {
            for gz in 0..3 {
                if let Some(l) = layout_for_mineshaft_cell(seed, gx, gz, &bg) {
                    return (l, bg);
                }
            }
        }
        panic!("no mineshaft found for seed {seed}");
    }

    #[test]
    fn layout_is_deterministic() {
        let bg = BiomeGenerator::new(11);
        assert_eq!(
            layout_for_mineshaft_cell(11, 5, 2, &bg),
            layout_for_mineshaft_cell(11, 5, 2, &bg),
        );
    }

    #[test]
    fn shaft_sits_underground() {
        let (l, _bg) = any_mineshaft(2026);
        assert!(l.ay < SEA_LEVEL, "mineshaft junction should be underground, ay={}", l.ay);
        assert!(l.ay >= 16, "shaft must clear the lava-cave band, ay={}", l.ay);
        assert!((2..=4).contains(&l.arms.len()), "expected 2..4 arms, got {}", l.arms.len());
    }

    #[test]
    fn chest_sites_are_on_their_arms_and_deterministic() {
        let (l, _bg) = any_mineshaft(99);
        for (i, &(dir_i, len)) in l.arms.iter().enumerate() {
            let (dx, dz) = ARM_DIRS[dir_i];
            let site = chest_site(&l, i).unwrap();
            // Distance from the junction along the arm is within its length.
            let along = (site.0 - l.ax) * dx + (site.2 - l.az) * dz;
            assert!(along > 0 && along <= len, "chest off its arm: along={along} len={len}");
            assert_eq!(chest_site(&l, i), Some(site), "chest site must be deterministic");
        }
    }

    #[test]
    fn loot_is_nonempty_and_deterministic() {
        let a = loot_for_chest(777);
        let b = loot_for_chest(777);
        assert_eq!(a.len(), crate::chest::CHEST_SLOTS);
        assert!(a.iter().any(|s| s.is_some()), "a mineshaft chest should hold loot");
        // ItemStack isn't PartialEq; compare the Debug fingerprint for determinism.
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "same seed → same loot");
    }

    #[test]
    fn building_a_real_shaft_hollows_the_junction_and_places_a_chest() {
        let seed = 31337;
        let (l, bg) = any_mineshaft(seed);
        let mut world = World::new();
        let cs = CHUNK_SIZE as i32;
        let cx0 = l.ax.div_euclid(cs);
        let cz0 = l.az.div_euclid(cs);
        // Generate a wide enough patch to cover all arms, then build.
        for dcx in -4..=4 {
            for dcz in -4..=4 {
                world.generate_column(cx0 + dcx, cz0 + dcz, &bg);
            }
        }
        // The junction cell should be carved air with a plank floor.
        assert_eq!(world.get_block(l.ax, l.ay, l.az), AIR, "junction should be hollow");
        assert_eq!(world.get_block(l.ax, l.ay - 1, l.az), OAK_PLANKS, "walkway plank under junction");
        // At least one loot chest exists somewhere in the shaft.
        let chest_count = world.iter_chests().count();
        assert!(chest_count >= 1, "a mineshaft should leave at least one loot chest");
    }
}

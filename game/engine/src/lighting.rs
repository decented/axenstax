//! Spec 30 — voxel lighting via BFS flood-fill.
//!
//! Two channels: **block-light** (torches / campfires / lava / future
//! glowstone) and **sky-light** (open sky tracking down through air).
//! Each is a 4-bit per-voxel value packed into `Chunk.light`. The
//! renderer reads these per-vertex; mob-spawn + crop growth gate on
//! `World::effective_light_at`.
//!
//! ## Algorithm
//!
//! Standard Minecraft-style BFS. The propagate path enqueues
//! `(pos, level)` from sources, then for each cardinal neighbour
//! computes `new = level - 1 - absorption` and, if greater than the
//! neighbour's current light, writes + enqueues. Opaque blocks have
//! absorption 15 so they never propagate light.
//!
//! Removal is a two-step **darken-then-refill**: BFS from the removed
//! source darkening everything that was lit *by* it (lower-or-equal
//! to its old contribution), collecting boundary positions of
//! remaining lit cells along the way; then re-propagate from those
//! boundary positions to fill in any cells that had alternative paths.

use crate::block::{BlockId, BlockRegistry};
use crate::chunk::CHUNK_SIZE;
use crate::world::World;
use std::collections::VecDeque;

/// Maximum light level (4-bit storage). No consumer references this named
/// constant — the `0x0F` mask is used directly wherever light is packed/read.
#[allow(dead_code)]
pub const MAX_LIGHT: u8 = 15;

/// Run the full initial light pass (sky-light then block-light) for
/// the chunk-column at `(cx, cz)`. Convenience wrapper around the two
/// initial passes — used by `chunk_stream` and `server` after
/// `world.generate_column` so chunks arrive at the renderer with the
/// right light values from the very first frame.
pub fn run_initial_pass_for_column(
    world: &mut World,
    cx: i32,
    cz: i32,
    registry: &BlockRegistry,
) {
    let max_y = (crate::world::MAX_CHUNK_Y + 1) * crate::chunk::CHUNK_SIZE as i32 - 1;
    let y_range = (0, max_y);
    initial_sky_light_for_column(world, cx, cz, y_range, registry);
    initial_block_light_for_column(world, cx, cz, y_range, registry);
}

const NEIGHBOURS: [(i32, i32, i32); 6] = [
    (1, 0, 0), (-1, 0, 0),
    (0, 1, 0), (0, -1, 0),
    (0, 0, 1), (0, 0, -1),
];

/// Spec 30 — propagate block-light from a set of source positions.
/// Each source is read for its `light_emission` from the registry; if
/// > current value at that cell, the cell's block-light is set and
/// > the BFS enqueues. Idempotent — running twice with no changes is
/// > a no-op.
pub fn propagate_block_light_from(
    world: &mut World,
    sources: impl IntoIterator<Item = (i32, i32, i32)>,
    registry: &BlockRegistry,
) {
    let mut queue: VecDeque<((i32, i32, i32), u8)> = VecDeque::new();
    for pos in sources {
        let block = world.get_block(pos.0, pos.1, pos.2);
        let emission = registry.light_emission(block);
        if emission == 0 {
            continue;
        }
        if world.block_light_at(pos.0, pos.1, pos.2) < emission {
            world.set_block_light_at(pos.0, pos.1, pos.2, emission);
            queue.push_back((pos, emission));
        }
    }
    bfs_propagate(world, &mut queue, registry, /* block_light */ true);
}

/// Spec 30 — remove block-light at a position whose source has gone
/// dark (torch broken). `prev_emission` is what the block was
/// emitting before the break. Walks the affected neighbourhood,
/// darkens cells that were lit by this source (and not by anything
/// stronger nearby), then re-propagates from any remaining lit
/// boundary cells to fill in.
pub fn remove_block_light_at(
    world: &mut World,
    pos: (i32, i32, i32),
    prev_emission: u8,
    registry: &BlockRegistry,
) {
    if prev_emission == 0 {
        return;
    }
    // Darken phase: queue (pos, level_we_had). For each neighbour with
    // light <= level - 1, also darken it; if a neighbour is brighter,
    // it's a refill candidate.
    let mut darken: VecDeque<((i32, i32, i32), u8)> = VecDeque::new();
    let mut refill: Vec<(i32, i32, i32)> = Vec::new();
    world.set_block_light_at(pos.0, pos.1, pos.2, 0);
    darken.push_back((pos, prev_emission));
    while let Some(((x, y, z), level)) = darken.pop_front() {
        for &(dx, dy, dz) in &NEIGHBOURS {
            let np = (x + dx, y + dy, z + dz);
            let nl = world.block_light_at(np.0, np.1, np.2);
            if nl == 0 {
                continue;
            }
            // If the neighbour's light is consistent with being lit by
            // us (i.e. lower than our level), darken it; else it has
            // an independent light path — mark for refill.
            if nl != 0 && nl < level {
                world.set_block_light_at(np.0, np.1, np.2, 0);
                darken.push_back((np, nl));
            } else {
                refill.push(np);
            }
        }
    }
    // Refill phase: re-propagate from boundary positions still lit.
    let mut queue: VecDeque<((i32, i32, i32), u8)> = VecDeque::new();
    for r in refill {
        let level = world.block_light_at(r.0, r.1, r.2);
        if level > 0 {
            queue.push_back((r, level));
        }
    }
    bfs_propagate(world, &mut queue, registry, /* block_light */ true);
}

/// Compute the initial sky-light for a column. Top-down per (x, z):
/// air blocks above the highest opaque block get `sky_light = 15`.
/// Below opaque, sky_light = 0 (caves stay dark).
///
/// `y_range` is the inclusive (min_y, max_y) span to process. Outside
/// this span the function leaves cells untouched.
pub fn initial_sky_light_for_column(
    world: &mut World,
    cx: i32,
    cz: i32,
    y_range: (i32, i32),
    registry: &BlockRegistry,
) {
    let (min_y, max_y) = y_range;
    let cs = CHUNK_SIZE as i32;
    let world_x_min = cx * cs;
    let world_z_min = cz * cs;
    // Per-column top-down pass — quick "open sky" seed.
    let mut seeds: Vec<(i32, i32, i32)> = Vec::new();
    for lz in 0..cs {
        for lx in 0..cs {
            let wx = world_x_min + lx;
            let wz = world_z_min + lz;
            let mut sky_blocked = false;
            for y in (min_y..=max_y).rev() {
                let b = world.get_block(wx, y, wz);
                if !sky_blocked {
                    // Above (or in) the highest opaque block.
                    if registry.light_absorption(b) >= 15 {
                        sky_blocked = true;
                        // The opaque block itself absorbs sky; below
                        // stays dark until BFS spreads sideways from
                        // a still-open column.
                        continue;
                    }
                    world.set_sky_light_at(wx, y, wz, 15);
                    seeds.push((wx, y, wz));
                }
            }
        }
    }
    // BFS spread sideways so open sky reaches under overhangs.
    let mut queue: VecDeque<((i32, i32, i32), u8)> = VecDeque::new();
    for s in seeds {
        queue.push_back((s, 15));
    }
    bfs_propagate(world, &mut queue, registry, /* block_light */ false);
}

/// Compute initial block-light for a column. Walks every voxel in
/// the chunk-column span, enqueues emitters, BFS-propagates.
pub fn initial_block_light_for_column(
    world: &mut World,
    cx: i32,
    cz: i32,
    y_range: (i32, i32),
    registry: &BlockRegistry,
) {
    let (min_y, max_y) = y_range;
    let cs = CHUNK_SIZE as i32;
    let world_x_min = cx * cs;
    let world_z_min = cz * cs;
    let mut sources: Vec<(i32, i32, i32)> = Vec::new();
    for lz in 0..cs {
        for lx in 0..cs {
            let wx = world_x_min + lx;
            let wz = world_z_min + lz;
            for y in min_y..=max_y {
                let b = world.get_block(wx, y, wz);
                if registry.light_emission(b) > 0 {
                    sources.push((wx, y, wz));
                }
            }
        }
    }
    propagate_block_light_from(world, sources, registry);
}

/// Spec 30 Phase F — apply lighting updates for a block place/break.
/// Call after the world's block state has been mutated. Handles:
///   * block-light removal if `prev_block` was an emitter,
///   * block-light propagation if `new_block` is an emitter,
///   * sky-light re-pass for the column if the block's opacity flipped.
pub fn update_for_block_change(
    world: &mut World,
    pos: (i32, i32, i32),
    prev_block: BlockId,
    new_block: BlockId,
    registry: &BlockRegistry,
) {
    // Block-light: remove old emitter if any.
    let prev_emission = registry.light_emission(prev_block);
    if prev_emission > 0 {
        remove_block_light_at(world, pos, prev_emission, registry);
    }
    // Block-light: add new emitter if any.
    let new_emission = registry.light_emission(new_block);
    if new_emission > 0 {
        propagate_block_light_from(world, [pos], registry);
    }
    // Sky-light: if opacity flipped, re-run the column's sky pass.
    // Localised propagation would be faster but the column-wide
    // recompute is simple and correct; profile-driven optimisation
    // can land later.
    let prev_opaque = registry.light_absorption(prev_block) >= 15;
    let new_opaque = registry.light_absorption(new_block) >= 15;
    if prev_opaque != new_opaque {
        let cx = pos.0.div_euclid(CHUNK_SIZE as i32);
        let cz = pos.2.div_euclid(CHUNK_SIZE as i32);
        let max_y = (crate::world::MAX_CHUNK_Y + 1) * crate::chunk::CHUNK_SIZE as i32 - 1;
        // Clear sky-light in the column before re-running the seed pass
        // so the recompute doesn't see stale 15s above a newly-placed
        // opaque block.
        let cs = CHUNK_SIZE as i32;
        for lz in 0..cs {
            for lx in 0..cs {
                let wx = cx * cs + lx;
                let wz = cz * cs + lz;
                for y in 0..=max_y {
                    world.set_sky_light_at(wx, y, wz, 0);
                }
            }
        }
        initial_sky_light_for_column(world, cx, cz, (0, max_y), registry);
    }
}

/// Shared BFS step. `is_block_light` selects between block-light and
/// sky-light accessors.
fn bfs_propagate(
    world: &mut World,
    queue: &mut VecDeque<((i32, i32, i32), u8)>,
    registry: &BlockRegistry,
    is_block_light: bool,
) {
    while let Some(((x, y, z), level)) = queue.pop_front() {
        if level <= 1 {
            continue;
        }
        for &(dx, dy, dz) in &NEIGHBOURS {
            let np = (x + dx, y + dy, z + dz);
            // Spec 02 §7.5 — an evicted column is a barrier: its light writes
            // are dropped and its light reads 0, so entering it would re-queue
            // the same cells forever. Restore runs the column's own light pass.
            if world.is_evicted_at(np.0, np.2) {
                continue;
            }
            let block = world.get_block(np.0, np.1, np.2);
            let absorption = registry.light_absorption(block);
            if absorption >= 15 {
                // Opaque block — light doesn't enter (its own face
                // brightness is read from this same value, which stays
                // 0 unless the block IS the emitter).
                continue;
            }
            let new_level = level.saturating_sub(1).saturating_sub(absorption);
            if new_level == 0 {
                continue;
            }
            let current = if is_block_light {
                world.block_light_at(np.0, np.1, np.2)
            } else {
                world.sky_light_at(np.0, np.1, np.2)
            };
            if new_level > current {
                if is_block_light {
                    world.set_block_light_at(np.0, np.1, np.2, new_level);
                } else {
                    world.set_sky_light_at(np.0, np.1, np.2, new_level);
                }
                queue.push_back((np, new_level));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{self, BlockRegistry};
    use crate::world::World;

    fn fresh() -> (World, BlockRegistry) {
        let mut w = World::new();
        // Allocate chunk (0,0,0) so set_block_light_at doesn't need to
        // create one indirectly.
        w.set_block(0, 0, 0, block::AIR);
        let r = BlockRegistry::new();
        (w, r)
    }

    /// Spec 02 §7.5 — light writes into an evicted column are dropped and its
    /// light reads as 0, so a BFS that entered it saw every cell as still dark
    /// and re-queued its neighbours without end (exponential: an 8 GiB
    /// allocation when the dedicated server restored a column beside a still-
    /// evicted one holding lava, Phase B1). An evicted column is a light
    /// barrier, as it is for the fluid and fire sims; restore relights it.
    #[test]
    fn block_light_stops_at_an_evicted_column() {
        let r = BlockRegistry::new();
        let mut w = World::new();
        // Column (1, 0) holds an edit, so it is kept (evicted), not dropped.
        w.set_block(20, 40, 8, block::STONE);
        assert!(w.evict_column(1, 0));
        // A lava source on column (0, 0)'s border with it.
        w.set_block(15, 40, 8, block::LAVA);
        propagate_block_light_from(&mut w, [(15, 40, 8)], &r);
        assert_eq!(w.block_light_at(15, 40, 8), 15);
        assert_eq!(w.block_light_at(14, 40, 8), 14, "spreads inside the live column");
        assert!(!w.has_chunk(1, 2, 0), "never enters the evicted column");
        assert!(w.restore_column(1, 0), "the evicted column is intact");
        assert_eq!(w.get_block(20, 40, 8), block::STONE);
    }

    #[test]
    fn light_emission_table_known_values() {
        let r = BlockRegistry::new();
        assert_eq!(r.light_emission(block::TORCH), 14);
        assert_eq!(r.light_emission(block::CAMPFIRE), 14);
        assert_eq!(r.light_emission(block::FURNACE_LIT), 13);
        // P10 — lava is the brightest natural emitter (Minecraft 15).
        assert_eq!(r.light_emission(block::LAVA), 15);
        assert_eq!(r.light_emission(block::STONE), 0);
        assert_eq!(r.light_emission(block::AIR), 0);
        // Spec 48 (Electricity) — a lit Electric Lamp is a primary light source;
        // a running Steam Generator gives a warm combustion glow. Unlit variants
        // emit nothing.
        assert_eq!(r.light_emission(block::ELECTRIC_LAMP_LIT), 14);
        assert_eq!(r.light_emission(block::ELECTRIC_LAMP), 0);
        assert_eq!(r.light_emission(block::STEAM_GENERATOR_LIT), 11);
        assert_eq!(r.light_emission(block::STEAM_GENERATOR), 0);
    }

    #[test]
    fn light_absorption_air_is_zero_opaque_is_max() {
        let r = BlockRegistry::new();
        assert_eq!(r.light_absorption(block::AIR), 0);
        assert_eq!(r.light_absorption(block::STONE), 15);
        // Glass is solid + transparent — should not block light.
        assert_eq!(r.light_absorption(block::GLASS), 0);
    }

    #[test]
    fn propagate_torch_lights_neighbours_with_decay() {
        let (mut w, r) = fresh();
        w.set_block(5, 5, 5, block::TORCH);
        propagate_block_light_from(&mut w, [(5, 5, 5)], &r);
        // Torch itself: emission = 14.
        assert_eq!(w.block_light_at(5, 5, 5), 14);
        // Adjacent: 13. Two away: 12. Etc.
        assert_eq!(w.block_light_at(6, 5, 5), 13);
        assert_eq!(w.block_light_at(7, 5, 5), 12);
        // Diagonal — BFS only goes cardinal, but distance-2 via two
        // cardinal hops = 12 (one hop horizontal + one vertical).
        assert_eq!(w.block_light_at(6, 6, 5), 12);
        // 14 blocks away: light should have decayed to 0.
        assert_eq!(w.block_light_at(5 + 14, 5, 5), 0);
        // 13 blocks away: light should be 1 (14 - 13).
        assert_eq!(w.block_light_at(5 + 13, 5, 5), 1);
    }

    #[test]
    fn opaque_block_blocks_propagation_into_itself() {
        // A torch surrounded on all 6 cardinal faces by stone has the
        // torch cell lit (it's the emitter) but every neighbour stays
        // at 0 — opaque blocks have absorption 15 and BFS refuses to
        // enter them. This is the fundamental "light doesn't pass
        // through walls" invariant.
        let (mut w, r) = fresh();
        let (tx, ty, tz) = (5, 5, 5);
        for &(dx, dy, dz) in &NEIGHBOURS {
            w.set_block(tx + dx, ty + dy, tz + dz, block::STONE);
        }
        w.set_block(tx, ty, tz, block::TORCH);
        propagate_block_light_from(&mut w, [(tx, ty, tz)], &r);
        // Torch lit.
        assert_eq!(w.block_light_at(tx, ty, tz), 14);
        // Every surrounding opaque block is 0 — light didn't enter.
        for &(dx, dy, dz) in &NEIGHBOURS {
            let nl = w.block_light_at(tx + dx, ty + dy, tz + dz);
            assert_eq!(nl, 0, "opaque neighbour at +{dx},{dy},{dz} should be 0; got {nl}");
        }
    }

    #[test]
    fn remove_block_light_after_breaking_source_darkens_neighbourhood() {
        let (mut w, r) = fresh();
        w.set_block(5, 5, 5, block::TORCH);
        propagate_block_light_from(&mut w, [(5, 5, 5)], &r);
        assert_eq!(w.block_light_at(6, 5, 5), 13);
        // Break the torch.
        w.set_block(5, 5, 5, block::AIR);
        remove_block_light_at(&mut w, (5, 5, 5), 14, &r);
        assert_eq!(w.block_light_at(5, 5, 5), 0);
        assert_eq!(w.block_light_at(6, 5, 5), 0);
        assert_eq!(w.block_light_at(7, 5, 5), 0);
    }

    #[test]
    fn remove_block_light_with_alternative_source_partial_refill() {
        // Two torches close together — break one; some cells should
        // remain lit (covered by the surviving torch).
        let (mut w, r) = fresh();
        w.set_block(5, 5, 5, block::TORCH);
        w.set_block(5, 5, 10, block::TORCH);
        propagate_block_light_from(&mut w, [(5, 5, 5), (5, 5, 10)], &r);
        // Middle cell between the two torches, both contribute.
        let mid_before = w.block_light_at(5, 5, 7);
        assert!(mid_before > 0);
        // Break the first torch.
        w.set_block(5, 5, 5, block::AIR);
        remove_block_light_at(&mut w, (5, 5, 5), 14, &r);
        // Middle cell still lit by the second torch.
        let mid_after = w.block_light_at(5, 5, 7);
        assert!(mid_after > 0, "middle cell should retain light from torch 2");
        // Second torch itself still at 14.
        assert_eq!(w.block_light_at(5, 5, 10), 14);
    }

    #[test]
    fn initial_sky_light_for_column_lights_open_sky() {
        // Empty column — all sky-light should be 15 everywhere.
        let (mut w, r) = fresh();
        initial_sky_light_for_column(&mut w, 0, 0, (0, 15), &r);
        for y in 0..=15 {
            assert_eq!(w.sky_light_at(0, y, 0), 15, "y={y} should be open sky");
            assert_eq!(w.sky_light_at(8, y, 8), 15);
        }
    }

    #[test]
    fn initial_sky_light_for_column_blocks_under_opaque() {
        // Column with a stone slab at y=10. Above stays 15, at slab = 0.
        let (mut w, r) = fresh();
        w.set_block(5, 10, 5, block::STONE);
        initial_sky_light_for_column(&mut w, 0, 0, (0, 15), &r);
        assert_eq!(w.sky_light_at(5, 11, 5), 15, "above the slab is open sky");
        assert_eq!(w.sky_light_at(5, 10, 5), 0, "slab itself is opaque");
        // Below the slab: BFS may have spread from neighbours (within
        // chunk bounds). The cell directly below at (5,9,5) should be
        // <= 14 (BFS one step from the open neighbour above the slab).
        let below = w.sky_light_at(5, 9, 5);
        assert!(below <= 14, "below opaque, sky-light should be <15; got {below}");
    }

    #[test]
    fn effective_light_uses_block_or_sky_minus_4() {
        let (mut w, _r) = fresh();
        w.set_block_light_at(0, 5, 0, 10);
        w.set_sky_light_at(0, 5, 0, 15);
        // max(10, 15-4) = max(10, 11) = 11.
        assert_eq!(w.effective_light_at(0, 5, 0), 11);
        // Block-light wins if sky is low.
        w.set_sky_light_at(0, 5, 0, 4);
        assert_eq!(w.effective_light_at(0, 5, 0), 10);
        // Both low — total darkness.
        w.set_block_light_at(0, 5, 0, 0);
        w.set_sky_light_at(0, 5, 0, 4);
        assert_eq!(w.effective_light_at(0, 5, 0), 0);
    }
}

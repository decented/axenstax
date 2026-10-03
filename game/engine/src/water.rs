//! Water flow system — simple spread model.
//!
//! Source blocks spread to adjacent air (4 cardinal + down), up to 7 blocks
//! horizontally. All water renders full-height. Removing a source retracts
//! dependent water.

use std::collections::VecDeque;
use ahash::AHashSet;
use crate::block;
use crate::world::World;

/// Budget: max blocks processed per tick to avoid frame spikes.
const SPREAD_BUDGET: usize = 64;
const RETRACT_BUDGET: usize = 64;
const MAX_SPREAD_DIST: u8 = 7;

pub struct WaterSystem {
    /// All water source positions (world-gen + player-placed).
    sources: AHashSet<(i32, i32, i32)>,
    /// Pending spread: (x, y, z, distance_from_source).
    spread_queue: VecDeque<(i32, i32, i32, u8)>,
    /// Pending retraction checks after a source is removed.
    retract_queue: VecDeque<(i32, i32, i32)>,
}

impl WaterSystem {
    pub fn new() -> Self {
        Self {
            sources: AHashSet::new(),
            spread_queue: VecDeque::new(),
            retract_queue: VecDeque::new(),
        }
    }

    /// Register a water source (world-gen or player-placed).
    pub fn add_source(&mut self, x: i32, y: i32, z: i32) {
        if self.sources.insert((x, y, z)) {
            self.spread_queue.push_back((x, y, z, 0));
        }
    }

    /// Remove a water source and queue retraction.
    pub fn remove_source(&mut self, x: i32, y: i32, z: i32) {
        if self.sources.remove(&(x, y, z)) {
            self.retract_queue.push_back((x, y, z));
        }
    }

    /// Returns true if the position is a registered source.
    pub fn is_source(&self, x: i32, y: i32, z: i32) -> bool {
        self.sources.contains(&(x, y, z))
    }

    /// Called when a non-water block is broken adjacent to water.
    /// Queues spread from neighbouring water into the newly-open position.
    pub fn notify_block_removed(&mut self, x: i32, y: i32, z: i32, world: &World) {
        for &(dx, dy, dz) in &[(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)] {
            let nx = x + dx;
            let ny = y + dy;
            let nz = z + dz;
            if world.get_block(nx, ny, nz) == block::WATER {
                let dist = if self.is_source(nx, ny, nz) { 0 } else { MAX_SPREAD_DIST - 1 };
                self.spread_queue.push_back((nx, ny, nz, dist));
            }
        }
    }

    /// Campaign B — water quenches lava. When water arrives at `(x,y,z)`, any
    /// orthogonally-adjacent lava freezes to obsidian (the lava-side rule lives
    /// in `lava.rs`; together they cover whichever fluid moves last). Newly
    /// frozen cells are pushed to `dirty` for re-meshing.
    fn freeze_adjacent_lava(world: &mut World, x: i32, y: i32, z: i32, dirty: &mut Vec<(i32, i32, i32)>) {
        for &(dx, dy, dz) in &[(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            if world.get_block(nx, ny, nz) == block::LAVA && !world.is_evicted_at(nx, nz) {
                world.set_block(nx, ny, nz, block::OBSIDIAN);
                dirty.push((nx, ny, nz));
            }
        }
    }

    /// Run one tick of water spreading. Returns dirty block positions.
    pub fn tick_spread(&mut self, world: &mut World) -> Vec<(i32, i32, i32)> {
        let mut dirty: Vec<(i32, i32, i32)> = Vec::new();
        let mut processed = 0;

        while processed < SPREAD_BUDGET {
            let Some((x, y, z, dist)) = self.spread_queue.pop_front() else {
                break;
            };
            processed += 1;

            // Spec 02 §7.5 — an evicted column is a barrier (reads go through
            // to its real blocks, so without this the flow would continue
            // inside it). Drop the entry; restore re-registers its sources.
            if world.is_evicted_at(x, z) || world.get_block(x, y, z) != block::WATER {
                continue;
            }

            let below = world.get_block(x, y - 1, z);
            if below == block::AIR {
                world.set_block(x, y - 1, z, block::WATER);
                // Falling water is full-depth (level 0) — clear any stale meta
                // (set_block never touches the sparse meta map).
                world.set_meta((x, y - 1, z), 0);
                dirty.push((x, y - 1, z));
                Self::freeze_adjacent_lava(world, x, y - 1, z, &mut dirty);
                self.spread_queue.push_back((x, y - 1, z, 0));
                continue;
            }

            if dist < MAX_SPREAD_DIST {
                for &(dx, dz) in &[(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    let nx = x + dx;
                    let nz = z + dz;
                    if !world.is_evicted_at(nx, nz) && world.get_block(nx, y, nz) == block::AIR {
                        world.set_block(nx, y, nz, block::WATER);
                        dirty.push((nx, y, nz));
                        Self::freeze_adjacent_lava(world, nx, y, nz, &mut dirty);
                        // MC-parity infinite pool: a flow cell flanked by two
                        // (or more) horizontal sources becomes a source itself
                        // (level 0). Otherwise it carries a depth level that
                        // fades 1..=7 with distance (meta AUX field).
                        if self.horizontal_source_neighbours(nx, y, nz) >= 2 {
                            world.set_meta((nx, y, nz), 0);
                            self.add_source(nx, y, nz);
                        } else {
                            world.set_meta(
                                (nx, y, nz),
                                crate::meta::with_aux(0, (dist + 1).min(MAX_SPREAD_DIST)),
                            );
                            self.spread_queue.push_back((nx, y, nz, dist + 1));
                        }
                    }
                }
            }
        }

        dirty
    }

    /// Run one tick of water retraction after a source is removed.
    /// Returns dirty block positions.
    pub fn tick_retract(&mut self, world: &mut World) -> Vec<(i32, i32, i32)> {
        let mut dirty: Vec<(i32, i32, i32)> = Vec::new();

        let mut to_check: Vec<(i32, i32, i32)> = Vec::new();
        while let Some(pos) = self.retract_queue.pop_front() {
            if to_check.len() >= RETRACT_BUDGET {
                self.retract_queue.push_front(pos);
                break;
            }
            to_check.push(pos);
        }

        for (sx, sy, sz) in to_check {
            let mut visited: AHashSet<(i32, i32, i32)> = AHashSet::new();
            let mut frontier: VecDeque<(i32, i32, i32, u8)> = VecDeque::new();
            let mut orphans: Vec<(i32, i32, i32)> = Vec::new();

            for &(dx, dy, dz) in &[(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)] {
                let nx = sx + dx;
                let ny = sy + dy;
                let nz = sz + dz;
                if world.get_block(nx, ny, nz) == block::WATER && !self.is_source(nx, ny, nz)
                    && !world.is_evicted_at(nx, nz)
                    && visited.insert((nx, ny, nz)) {
                        frontier.push_back((nx, ny, nz, 0));
                    }
            }

            while let Some((x, y, z, dist)) = frontier.pop_front() {
                if self.can_reach_source(x, y, z, world) {
                    continue;
                }

                orphans.push((x, y, z));

                if dist < MAX_SPREAD_DIST {
                    for &(dx, dy, dz) in &[(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)] {
                        let nx = x + dx;
                        let ny = y + dy;
                        let nz = z + dz;
                        if world.get_block(nx, ny, nz) == block::WATER
                            && !self.is_source(nx, ny, nz)
                            && !world.is_evicted_at(nx, nz)
                            && visited.insert((nx, ny, nz))
                        {
                            frontier.push_back((nx, ny, nz, dist + 1));
                        }
                    }
                }
            }

            for (ox, oy, oz) in orphans {
                world.set_block(ox, oy, oz, block::AIR);
                world.set_meta((ox, oy, oz), 0); // clear the depth level
                dirty.push((ox, oy, oz));
            }
        }

        dirty
    }

    /// How many of the 4 horizontal neighbours of `(x, y, z)` are registered
    /// sources? Feeds the MC-parity infinite-pool rule (>= 2 promotes a flow
    /// cell into a source).
    fn horizontal_source_neighbours(&self, x: i32, y: i32, z: i32) -> u32 {
        [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .filter(|(dx, dz)| self.is_source(x + dx, y, z + dz))
            .count() as u32
    }

    /// BFS from a water block to check if it can trace a path to any source
    /// within MAX_SPREAD_DIST horizontal steps.
    fn can_reach_source(&self, x: i32, y: i32, z: i32, world: &World) -> bool {
        let mut visited: AHashSet<(i32, i32, i32)> = AHashSet::new();
        let mut frontier: VecDeque<(i32, i32, i32, u8)> = VecDeque::new();
        visited.insert((x, y, z));
        frontier.push_back((x, y, z, 0));

        while let Some((cx, cy, cz, dist)) = frontier.pop_front() {
            if self.is_source(cx, cy, cz) {
                return true;
            }
            if dist >= MAX_SPREAD_DIST {
                continue;
            }
            for &(dx, dy, dz) in &[(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)] {
                let nx = cx + dx;
                let ny = cy + dy;
                let nz = cz + dz;
                if world.get_block(nx, ny, nz) == block::WATER && visited.insert((nx, ny, nz)) {
                    frontier.push_back((nx, ny, nz, dist + 1));
                }
            }
        }
        false
    }

    /// Bulk-register all existing water blocks in a column as sources.
    /// Called after world generation for each column.
    pub fn register_column_sources(&mut self, cx: i32, cz: i32, world: &World) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        let max_y = 6 * cs;
        let base_x = cx * cs;
        let base_z = cz * cs;

        for lx in 0..cs {
            for lz in 0..cs {
                let wx = base_x + lx;
                let wz = base_z + lz;
                for y in 0..max_y {
                    if world.get_block(wx, y, wz) == block::WATER {
                        self.sources.insert((wx, y, wz));
                    }
                }
            }
        }
    }
}

/// Depth level (0..=7) of the water cell at `(x, y, z)`. 0 = source or full
/// (falling) water; 1..=7 fades out from the source. Reads the meta AUX field
/// — worldgen ocean cells carry no meta entry and therefore read 0 (full).
/// This is the level the mesher, [`flow_vector`] consumers, and the future
/// water wheel (Electricity Phase 3–4) all share.
pub fn level_at(world: &World, x: i32, y: i32, z: i32) -> u8 {
    crate::meta::aux(world.meta_at(x, y, z))
}

/// Horizontal push (blocks/tick²) a flowing current applies to entities and
/// swimming players each tick. Feel knob — tune on playtest.
pub const FLOW_PUSH: f32 = 0.02;

/// Render height of a water cell's surface for a given depth level (MC-like:
/// level 0 → 8/9 ≈ 0.889, fading to level 7 → 1/9 ≈ 0.111). Shared by the
/// mesher today and by anything that later needs "where is the water surface"
/// (boats, splash particles).
pub fn water_surface_height(level: u8) -> f32 {
    (8.0 - level.min(7) as f32) / 9.0
}

/// Weight an edge-drop (flow about to fall over a lip) contributes to the
/// flow direction, relative to one depth-level of gradient.
const EDGE_DROP_WEIGHT: f32 = 2.0;

/// The current at a water cell: a unit-length direction the water is flowing,
/// or `None` for still water (uniform pool / lone source). Pure — THE query
/// the entity/player push uses today and the water wheel (Electricity Phase
/// 3–4), boats, and item streams consume later.
///
/// Direction is derived from the depth-level gradient (toward shallower
/// neighbouring flow, away from deeper), pulled toward edge-drops (an AIR
/// neighbour with water below it), with a downward component while falling.
pub fn flow_vector(world: &World, x: i32, y: i32, z: i32) -> Option<[f32; 3]> {
    if world.get_block(x, y, z) != block::WATER {
        return None;
    }
    let level = level_at(world, x, y, z) as f32;
    let mut vx = 0.0f32;
    let mut vz = 0.0f32;
    for &(dx, dz) in &[(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
        let (nx, nz) = (x + dx, z + dz);
        let nb = world.get_block(nx, y, nz);
        if nb == block::WATER {
            let d = level_at(world, nx, y, nz) as f32 - level;
            vx += dx as f32 * d;
            vz += dz as f32 * d;
        } else if nb == block::AIR && world.get_block(nx, y - 1, nz) == block::WATER {
            vx += dx as f32 * EDGE_DROP_WEIGHT;
            vz += dz as f32 * EDGE_DROP_WEIGHT;
        }
    }
    let vy = if world.get_block(x, y - 1, z) == block::AIR { -EDGE_DROP_WEIGHT } else { 0.0 };
    let mag = (vx * vx + vy * vy + vz * vz).sqrt();
    if mag < 1e-4 {
        return None;
    }
    Some([vx / mag, vy / mag, vz / mag])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::world::World;

    /// Stone floor at y=9 over a comfortable area so spread stays horizontal.
    fn world_with_floor() -> World {
        let mut w = World::new();
        for x in -12..=12 {
            for z in -12..=12 {
                w.set_block(x, 9, z, block::STONE);
            }
        }
        w
    }

    fn settle(ws: &mut WaterSystem, w: &mut World) {
        for _ in 0..64 {
            ws.tick_spread(w);
        }
    }

    #[test]
    fn horizontal_spread_writes_depth_levels() {
        let mut w = world_with_floor();
        let mut ws = WaterSystem::new();
        w.set_block(0, 10, 0, block::WATER);
        ws.add_source(0, 10, 0);
        settle(&mut ws, &mut w);
        assert_eq!(w.get_block(3, 10, 0), block::WATER, "spread reached x=3");
        assert_eq!(level_at(&w, 0, 10, 0), 0, "source is level 0");
        assert_eq!(level_at(&w, 1, 10, 0), 1);
        assert_eq!(level_at(&w, 2, 10, 0), 2);
        assert_eq!(level_at(&w, 7, 10, 0), 7, "rim of the 7-block spread");
    }

    #[test]
    fn falling_water_is_full_depth() {
        let mut w = world_with_floor();
        // Open a shaft: remove floor under (1,10,0)'s neighbour so water falls.
        w.set_block(2, 9, 0, block::AIR);
        w.set_block(2, 8, 0, block::STONE); // land one below
        let mut ws = WaterSystem::new();
        w.set_block(0, 10, 0, block::WATER);
        ws.add_source(0, 10, 0);
        settle(&mut ws, &mut w);
        assert_eq!(w.get_block(2, 9, 0), block::WATER, "water fell into the shaft");
        assert_eq!(level_at(&w, 2, 9, 0), 0, "falling water lands full-depth");
    }

    #[test]
    fn retraction_clears_depth_meta() {
        let mut w = world_with_floor();
        let mut ws = WaterSystem::new();
        w.set_block(0, 10, 0, block::WATER);
        ws.add_source(0, 10, 0);
        settle(&mut ws, &mut w);
        assert_ne!(level_at(&w, 2, 10, 0), 0, "flow cell has a level before retract");
        w.set_block(0, 10, 0, block::AIR);
        ws.remove_source(0, 10, 0);
        for _ in 0..64 {
            ws.tick_retract(&mut w);
        }
        assert_eq!(w.get_block(2, 10, 0), block::AIR, "orphan retracted");
        assert_eq!(w.meta_at(2, 10, 0), 0, "orphan's depth meta cleared");
    }

    #[test]
    fn two_adjacent_sources_make_infinite_pool() {
        // MC parity: a flow cell with >= 2 horizontal source neighbours
        // becomes a source itself — the classic 2x2 infinite pool.
        let mut w = world_with_floor();
        let mut ws = WaterSystem::new();
        w.set_block(0, 10, 0, block::WATER);
        w.set_block(2, 10, 0, block::WATER);
        ws.add_source(0, 10, 0);
        ws.add_source(2, 10, 0);
        settle(&mut ws, &mut w);
        assert_eq!(w.get_block(1, 10, 0), block::WATER, "gap filled");
        assert!(ws.is_source(1, 10, 0), "gap cell promoted to source");
        assert_eq!(level_at(&w, 1, 10, 0), 0, "promoted source is full depth");
    }

    #[test]
    fn flow_vector_points_downstream() {
        let mut w = world_with_floor();
        let mut ws = WaterSystem::new();
        w.set_block(0, 10, 0, block::WATER);
        ws.add_source(0, 10, 0);
        settle(&mut ws, &mut w);
        let f = flow_vector(&w, 2, 10, 0).expect("flow cell has a current");
        assert!(f[0] > 0.5, "current points away from the source (+x): {f:?}");
        assert!(f[2].abs() < 0.5, "no sideways bias on the +x axis: {f:?}");
        // Symmetric check on the -x arm.
        let g = flow_vector(&w, -2, 10, 0).expect("flow cell has a current");
        assert!(g[0] < -0.5, "-x arm flows -x: {g:?}");
    }

    #[test]
    fn flow_vector_none_for_still_pool_and_non_water() {
        let mut w = world_with_floor();
        // A lone source with sources all around (mini pool of sources).
        for x in 0..2 {
            for z in 0..2 {
                w.set_block(x, 10, z, block::WATER);
            }
        }
        // All level 0 (no meta), fully enclosed by more water on every side?
        // Interior cell (0,10,0) has water at (1,..) and (0,..,1) at equal
        // level and AIR at (-1,..)/(..,-1) with no water below → edge pull
        // exists there. Wall it in so it is genuinely still:
        for &(x, z) in &[(-1, 0), (2, 0), (0, -1), (0, 2), (-1, 1), (2, 1), (1, -1), (1, 2)] {
            w.set_block(x, 10, z, block::STONE);
        }
        assert_eq!(flow_vector(&w, 0, 10, 0), None, "walled uniform pool is still");
        assert_eq!(flow_vector(&w, 5, 12, 5), None, "non-water cell has no flow");
    }

    #[test]
    fn flow_vector_pulls_toward_edge_drop_and_down_while_falling() {
        let mut w = world_with_floor();
        // Manual construction: a flow cell (level 3) beside a lip — the +x
        // neighbour is AIR with water already below it (the far side of a
        // waterfall edge).
        w.set_block(0, 10, 0, block::WATER);
        w.set_meta((0, 10, 0), crate::meta::with_aux(0, 3));
        w.set_block(1, 9, 0, block::WATER); // below the AIR neighbour at (1,10,0)
        let f = flow_vector(&w, 0, 10, 0).expect("lip cell is flowing");
        assert!(f[0] > 0.5, "current pulls toward the drop (+x): {f:?}");
        // Mid-fall: a water cell with AIR directly below points down.
        let mut w2 = World::new();
        w2.set_block(5, 20, 5, block::WATER);
        let g = flow_vector(&w2, 5, 20, 5).expect("mid-air water is falling");
        assert!(g[1] < -0.5, "falling cell's current points down: {g:?}");
    }

    #[test]
    fn water_surface_height_fades_monotonically() {
        assert!((water_surface_height(0) - 8.0 / 9.0).abs() < 1e-6, "source ≈ 0.889");
        assert!((water_surface_height(7) - 1.0 / 9.0).abs() < 1e-6, "rim ≈ 0.111");
        for l in 0..7u8 {
            assert!(
                water_surface_height(l) > water_surface_height(l + 1),
                "strictly decreasing at level {l}"
            );
        }
        assert_eq!(
            water_surface_height(9),
            water_surface_height(7),
            "out-of-range clamps"
        );
    }

    #[test]
    fn worldgen_column_registration_leaves_meta_untouched() {
        let mut w = World::new();
        w.set_block(4, 20, 4, block::WATER);
        let mut ws = WaterSystem::new();
        ws.register_column_sources(0, 0, &w);
        assert!(ws.is_source(4, 20, 4));
        assert_eq!(w.meta_at(4, 20, 4), 0, "ocean cells stay meta-free (level 0)");
    }

    /// Spec 02 §7.5 — read-through makes an evicted column read its real
    /// blocks, so water must treat it as a barrier: no spread into it, and
    /// queued cells inside it are dropped (re-registered on restore).
    #[test]
    fn water_does_not_spread_into_an_evicted_column() {
        let mut w = world_with_floor();
        // Floor for column (1,0) (x 16..32), then evict it (edited → kept).
        for x in 13..32 {
            for z in -12..16 {
                w.set_block(x, 9, z, block::STONE);
            }
        }
        assert!(w.evict_column(1, 0));
        let mut ws = WaterSystem::new();
        w.set_block(12, 10, 0, block::WATER);
        ws.add_source(12, 10, 0);
        settle(&mut ws, &mut w);
        assert_eq!(w.get_block(15, 10, 0), block::WATER, "spreads inside resident columns");
        assert_eq!(w.get_block(16, 10, 0), block::AIR, "stops at the evicted column");
        assert!(!w.has_chunk(1, 0, 0), "no stray chunk");
    }
}

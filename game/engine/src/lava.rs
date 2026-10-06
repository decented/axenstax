//! Lava flow system — Campaign B.
//!
//! Mirrors [`crate::water::WaterSystem`] but lava is **shorter-range** (spreads
//! 3 blocks, not 7) and **slower** (it only advances every third tick), and it
//! **solidifies on contact with water**: a flowing front that would move next
//! to water becomes obsidian instead, which is the classic "pour water on the
//! lava" obsidian farm. The reverse direction (water flowing *onto* lava) is
//! handled in `water.rs`, so the interaction works whichever fluid moves last.
//!
//! Eventual crate home: `genesis_sim`.

use std::collections::VecDeque;

use ahash::AHashSet;

use crate::block::{AIR, LAVA, OBSIDIAN, WATER};
use crate::world::World;

/// Max cells solidified/spread per active tick — lava is rarer than water so a
/// modest budget keeps it smooth.
const SPREAD_BUDGET: usize = 32;
const RETRACT_BUDGET: usize = 32;
/// Lava only reaches 3 blocks from a source (overworld parity).
const MAX_SPREAD_DIST: u8 = 3;
/// Lava advances every Nth `tick_spread` call — it crawls compared to water.
const SLOW_FACTOR: u8 = 3;

pub struct LavaSystem {
    sources: crate::fluids::SourceSet,
    spread_queue: VecDeque<(i32, i32, i32, u8)>,
    retract_queue: VecDeque<(i32, i32, i32)>,
    slow: u8,
}

impl LavaSystem {
    pub fn new() -> Self {
        Self {
            sources: crate::fluids::SourceSet::default(),
            spread_queue: VecDeque::new(),
            retract_queue: VecDeque::new(),
            slow: 0,
        }
    }

    pub fn add_source(&mut self, x: i32, y: i32, z: i32) {
        if self.sources.insert((x, y, z)) {
            self.spread_queue.push_back((x, y, z, 0));
        }
    }

    pub fn remove_source(&mut self, x: i32, y: i32, z: i32) {
        if self.sources.remove((x, y, z)) {
            self.retract_queue.push_back((x, y, z));
        }
    }

    pub fn is_source(&self, x: i32, y: i32, z: i32) -> bool {
        self.sources.contains((x, y, z))
    }

    /// A non-lava block was broken next to lava — re-wake the neighbouring lava
    /// so it can flow into the newly-opened space.
    pub fn notify_block_removed(&mut self, x: i32, y: i32, z: i32, world: &World) {
        for &(dx, dy, dz) in &[(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            if world.get_block(nx, ny, nz) == LAVA {
                let dist = if self.is_source(nx, ny, nz) { 0 } else { MAX_SPREAD_DIST - 1 };
                self.spread_queue.push_back((nx, ny, nz, dist));
            }
        }
    }

    /// True if any of the 6 neighbours of `(x,y,z)` is water — used to decide
    /// whether a flowing lava cell should freeze to obsidian.
    fn touches_water(world: &World, x: i32, y: i32, z: i32) -> bool {
        for &(dx, dy, dz) in &[(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            if world.get_block(x + dx, y + dy, z + dz) == WATER {
                return true;
            }
        }
        false
    }

    /// Place lava at a flow target — but if the target would sit next to water,
    /// freeze it to obsidian and report it as terminal (no further flow).
    /// Returns `true` if lava was placed (caller should enqueue it).
    fn place_flow(world: &mut World, x: i32, y: i32, z: i32, dirty: &mut Vec<(i32, i32, i32)>) -> bool {
        if Self::touches_water(world, x, y, z) {
            world.set_block(x, y, z, OBSIDIAN);
            dirty.push((x, y, z));
            false
        } else {
            world.set_block(x, y, z, LAVA);
            dirty.push((x, y, z));
            true
        }
    }

    /// One tick of lava spreading. Crawls (only acts every `SLOW_FACTOR` calls).
    pub fn tick_spread(&mut self, world: &mut World) -> Vec<(i32, i32, i32)> {
        let mut dirty: Vec<(i32, i32, i32)> = Vec::new();
        self.slow = (self.slow + 1) % SLOW_FACTOR;
        if self.slow != 0 {
            return dirty; // resting tick — lava is slow
        }

        let mut processed = 0;
        while processed < SPREAD_BUDGET {
            let Some((x, y, z, dist)) = self.spread_queue.pop_front() else {
                break;
            };
            processed += 1;

            // Spec 02 §7.5 — a column that is not present (evicted, dropped or
            // never loaded) is a barrier; drop the entry (its stream-in
            // re-registers the column's sources).
            if !world.is_column_present_at(x, z) || world.get_block(x, y, z) != LAVA {
                continue; // it solidified, was removed, or its column unloaded
            }

            // A source/flow next to water freezes in place (obsidian). If the
            // frozen cell was a *source*, drop it from `sources` — otherwise the
            // set keeps a phantom source on a now-solid obsidian cell, which
            // makes `is_source` lie, props up orphaned downstream flow that
            // should drain, and lets a re-placed lava cell there resurrect a
            // ghost source. `remove_source` is a no-op for non-source cells and
            // enqueues the now-orphaned flow for retraction.
            if Self::touches_water(world, x, y, z) {
                world.set_block(x, y, z, OBSIDIAN);
                self.remove_source(x, y, z);
                dirty.push((x, y, z));
                continue;
            }

            // Flow down first.
            if world.get_block(x, y - 1, z) == AIR {
                if Self::place_flow(world, x, y - 1, z, &mut dirty) {
                    self.spread_queue.push_back((x, y - 1, z, 0));
                }
                continue;
            }

            // Then spread sideways, up to the (short) range.
            if dist < MAX_SPREAD_DIST {
                for &(dx, dz) in &[(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, nz) = (x + dx, z + dz);
                    // Never into a column that is not present: `set_block`
                    // would conjure a chunk its generation then skips (Phase
                    // B1 review — cave lava at a loaded edge left the next
                    // column without bedrock).
                    if world.is_column_present_at(nx, nz)
                        && world.get_block(nx, y, nz) == AIR
                        && Self::place_flow(world, nx, y, nz, &mut dirty) {
                            self.spread_queue.push_back((nx, y, nz, dist + 1));
                        }
                }
            }
        }
        dirty
    }

    /// One tick of retraction after a lava source is removed — orphaned flow
    /// (no path back to a source) drains to air. Same algorithm as water.
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

            for &(dx, dy, dz) in &[(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let (nx, ny, nz) = (sx + dx, sy + dy, sz + dz);
                if world.get_block(nx, ny, nz) == LAVA && !self.is_source(nx, ny, nz) && world.is_column_present_at(nx, nz) && visited.insert((nx, ny, nz)) {
                    frontier.push_back((nx, ny, nz, 0));
                }
            }

            while let Some((x, y, z, dist)) = frontier.pop_front() {
                if self.can_reach_source(x, y, z, world) {
                    continue;
                }
                orphans.push((x, y, z));
                if dist < MAX_SPREAD_DIST {
                    for &(dx, dy, dz) in &[(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                        let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                        if world.get_block(nx, ny, nz) == LAVA && !self.is_source(nx, ny, nz) && world.is_column_present_at(nx, nz) && visited.insert((nx, ny, nz)) {
                            frontier.push_back((nx, ny, nz, dist + 1));
                        }
                    }
                }
            }

            for (ox, oy, oz) in orphans {
                world.set_block(ox, oy, oz, AIR);
                dirty.push((ox, oy, oz));
            }
        }
        dirty
    }

    fn can_reach_source(&self, x: i32, y: i32, z: i32, world: &World) -> bool {
        let mut visited: AHashSet<(i32, i32, i32)> = AHashSet::new();
        let mut frontier: VecDeque<(i32, i32, i32, u8)> = VecDeque::new();
        visited.insert((x, y, z));
        frontier.push_back((x, y, z, 0));
        while let Some((cx, cy, cz, dist)) = frontier.pop_front() {
            // Lava read through a column that is not present may be fed by a
            // source forgotten when it streamed out: assume it is (water does
            // the same).
            if self.is_source(cx, cy, cz) || !world.is_column_present_at(cx, cz) {
                return true;
            }
            if dist >= MAX_SPREAD_DIST {
                continue;
            }
            for &(dx, dy, dz) in &[(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let (nx, ny, nz) = (cx + dx, cy + dy, cz + dz);
                if world.get_block(nx, ny, nz) == LAVA && visited.insert((nx, ny, nz)) {
                    frontier.push_back((nx, ny, nz, dist + 1));
                }
            }
        }
        false
    }

    /// Register every existing lava block in a column as a source (post
    /// world-gen), so natural pools flow correctly into any air they border.
    pub fn register_column_sources(&mut self, cx: i32, cz: i32, world: &World) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        let max_y = 6 * cs;
        let base_x = cx * cs;
        let base_z = cz * cs;
        for lx in 0..cs {
            for lz in 0..cs {
                let (wx, wz) = (base_x + lx, base_z + lz);
                for y in 0..max_y {
                    if world.get_block(wx, y, wz) == LAVA
                        && self.sources.insert((wx, y, wz)) {
                            // Wake it once so a pool bordering open air flows.
                            self.spread_queue.push_back((wx, y, wz, 0));
                        }
                }
            }
        }
    }

    /// Forget the sources in chunk column `(cx, cz)` as it streams out (Phase
    /// B1 review — see `WaterSystem::forget_column`). No retraction;
    /// `register_column_sources` re-adds (and re-wakes) them on stream-in.
    pub fn forget_column(&mut self, cx: i32, cz: i32) {
        self.sources.forget_column(cx, cz);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_floor(world: &mut World, y: i32, r: i32) {
        for x in -r..=r {
            for z in -r..=r {
                world.set_block(x, y, z, crate::block::STONE);
            }
        }
    }

    #[test]
    fn lava_flows_downhill_into_air() {
        let mut world = World::new();
        solid_floor(&mut world, 60, 4);
        // A lava source floating one block above the floor.
        world.set_block(0, 64, 0, LAVA);
        let mut lava = LavaSystem::new();
        lava.add_source(0, 64, 0);
        // Lava is slow — give it several ticks.
        for _ in 0..12 {
            lava.tick_spread(&mut world);
        }
        // It should have dropped down toward the floor.
        assert_eq!(world.get_block(0, 63, 0), LAVA, "lava should pour down");
    }

    #[test]
    fn lava_spread_is_bounded_to_short_range() {
        let mut world = World::new();
        solid_floor(&mut world, 59, 12);
        // Source sitting on the floor so it can only spread sideways.
        world.set_block(0, 60, 0, LAVA);
        let mut lava = LavaSystem::new();
        lava.add_source(0, 60, 0);
        for _ in 0..40 {
            lava.tick_spread(&mut world);
        }
        // Within range it reaches; beyond MAX_SPREAD_DIST it must not.
        assert_eq!(world.get_block(3, 60, 0), LAVA, "lava reaches 3 blocks");
        assert_eq!(world.get_block(6, 60, 0), AIR, "lava must not reach 6 blocks");
    }

    #[test]
    fn lava_flowing_next_to_water_freezes_to_obsidian() {
        let mut world = World::new();
        solid_floor(&mut world, 59, 6);
        // Water sitting at (2,60,0); lava source at (0,60,0). As lava spreads
        // toward the water it should hit the cell next to it and freeze.
        world.set_block(2, 60, 0, WATER);
        world.set_block(0, 60, 0, LAVA);
        let mut lava = LavaSystem::new();
        lava.add_source(0, 60, 0);
        for _ in 0..30 {
            lava.tick_spread(&mut world);
        }
        // The cell at (1,60,0) borders the water → obsidian, not lava.
        assert_eq!(world.get_block(1, 60, 0), OBSIDIAN, "lava beside water makes obsidian");
    }

    #[test]
    fn freezing_a_source_in_place_removes_it_from_the_source_set() {
        let mut world = World::new();
        solid_floor(&mut world, 59, 6);
        // A lava SOURCE sitting directly beside water freezes in place. The set
        // must drop it — otherwise `is_source` keeps lying about a now-solid
        // obsidian cell (props up orphaned flow, resurrects a ghost source).
        world.set_block(1, 60, 0, WATER);
        world.set_block(0, 60, 0, LAVA);
        let mut lava = LavaSystem::new();
        lava.add_source(0, 60, 0);
        assert!(lava.is_source(0, 60, 0));
        for _ in 0..20 {
            lava.tick_spread(&mut world);
        }
        assert_eq!(world.get_block(0, 60, 0), OBSIDIAN, "the source froze to obsidian");
        assert!(!lava.is_source(0, 60, 0), "frozen source must leave the source set");
    }

    #[test]
    fn removing_a_source_drains_orphaned_flow() {
        let mut world = World::new();
        solid_floor(&mut world, 59, 8);
        world.set_block(0, 60, 0, LAVA);
        let mut lava = LavaSystem::new();
        lava.add_source(0, 60, 0);
        for _ in 0..40 {
            lava.tick_spread(&mut world);
        }
        assert_eq!(world.get_block(2, 60, 0), LAVA, "flow established before removal");
        lava.remove_source(0, 60, 0);
        world.set_block(0, 60, 0, AIR);
        for _ in 0..40 {
            lava.tick_retract(&mut world);
        }
        assert_eq!(world.get_block(2, 60, 0), AIR, "orphaned flow drains away");
    }
}

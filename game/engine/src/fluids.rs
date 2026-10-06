//! Cross-fluid coordination.
//!
//! [`WaterSystem`] and [`LavaSystem`] are independent and hold no reference to
//! each other, so the small amount of logic that must see BOTH lives here and is
//! called from every tick/edit site — the client `GameState` and the
//! authoritative `GameServer` — so the two paths can't drift. Before this module
//! existed, `GameServer` didn't simulate lava at all and the water-freeze path
//! leaked stale lava sources.

use ahash::{AHashMap, AHashSet};

use crate::block::{AIR, LAVA, OBSIDIAN, WATER};
use crate::lava::LavaSystem;
use crate::water::WaterSystem;
use crate::world::World;

/// Mirror a block edit into both fluid systems' source bookkeeping, matching the
/// client's inline place/break handling (see `game_loop.rs`). Covers three cases:
/// placing a fluid registers a source; digging a fluid drops its source; opening
/// a gap to air beside a fluid wakes it so it can flow in. `old_block` is the
/// block that was at `(x,y,z)` before the edit; `new_block` is what replaced it.
pub fn notify_block_edit(
    water: &mut WaterSystem,
    lava: &mut LavaSystem,
    world: &World,
    x: i32,
    y: i32,
    z: i32,
    old_block: u16,
    new_block: u16,
) {
    match new_block {
        WATER => water.add_source(x, y, z),
        LAVA => lava.add_source(x, y, z),
        _ => {
            // A fluid source that was overwritten (dug, or displaced by a solid)
            // must be dropped; otherwise a gap opening to air lets a neighbour
            // flow in. `remove_source` also enqueues retraction of orphaned flow.
            if old_block == WATER {
                water.remove_source(x, y, z);
            } else if new_block == AIR {
                water.notify_block_removed(x, y, z, world);
            }
            if old_block == LAVA {
                lava.remove_source(x, y, z);
            } else if new_block == AIR {
                lava.notify_block_removed(x, y, z, world);
            }
        }
    }
}

/// Reconcile lava sources after a water spread tick. When water spreads next to
/// lava it freezes that cell to obsidian directly on the voxel world (see
/// `WaterSystem::freeze_adjacent_lava`) but has no reference to the lava system,
/// so a frozen cell that was a registered lava source is left behind as a
/// phantom: `is_source` keeps lying about a now-solid obsidian cell, and lava
/// re-placed there is silently inert because `add_source`'s `insert` no-ops on
/// the stale entry. `water_dirty` is the water tick's dirty list; the freshly
/// frozen cells are exactly its `OBSIDIAN` entries (water only ever writes WATER
/// or OBSIDIAN). `remove_source` is a no-op for non-sources, so calling it on
/// every obsidian cell is safe and also drains any now-orphaned downstream flow.
pub fn reconcile_frozen_lava_sources(
    lava: &mut LavaSystem,
    world: &World,
    water_dirty: &[(i32, i32, i32)],
) {
    for &(x, y, z) in water_dirty {
        if world.get_block(x, y, z) == OBSIDIAN {
            lava.remove_source(x, y, z);
        }
    }
}

/// A fluid system's source cells, indexed by chunk column so a column that
/// streams out can be forgotten in one step (Phase B1 review). Water registers
/// every water block of a streamed-in column as a source, so a flat set grew
/// without bound as players roamed an ocean world, and pruning it by scan was
/// O(every source) per unload.
#[derive(Default)]
pub struct SourceSet {
    by_column: AHashMap<(i32, i32), AHashSet<(i32, i32, i32)>>,
}

impl SourceSet {
    fn column(x: i32, z: i32) -> (i32, i32) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        (x.div_euclid(cs), z.div_euclid(cs))
    }

    /// Add a source; true if it was new.
    pub fn insert(&mut self, (x, y, z): (i32, i32, i32)) -> bool {
        self.by_column.entry(Self::column(x, z)).or_default().insert((x, y, z))
    }

    /// Remove a source; true if it was there.
    pub fn remove(&mut self, (x, y, z): (i32, i32, i32)) -> bool {
        let col = Self::column(x, z);
        let Some(set) = self.by_column.get_mut(&col) else {
            return false;
        };
        let removed = set.remove(&(x, y, z));
        if set.is_empty() {
            self.by_column.remove(&col);
        }
        removed
    }

    pub fn contains(&self, (x, y, z): (i32, i32, i32)) -> bool {
        self.by_column.get(&Self::column(x, z)).is_some_and(|s| s.contains(&(x, y, z)))
    }

    /// Forget every source in chunk column `(cx, cz)` — no retraction: the
    /// blocks stay where they are, and the column's stream-in re-registers
    /// them. Returns how many were forgotten.
    pub fn forget_column(&mut self, cx: i32, cz: i32) -> usize {
        self.by_column.remove(&(cx, cz)).map_or(0, |s| s.len())
    }

    /// How many sources are held, across every column.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_column.values().map(|s| s.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::STONE;

    fn solid_floor(world: &mut World, y: i32, r: i32) {
        for x in -r..=r {
            for z in -r..=r {
                world.set_block(x, y, z, STONE);
            }
        }
    }

    #[test]
    fn placing_lava_registers_a_flowing_source() {
        let mut world = World::new();
        solid_floor(&mut world, 60, 4);
        let mut water = WaterSystem::new();
        let mut lava = LavaSystem::new();
        // Simulate a player emptying a lava bucket at (0,64,0).
        world.set_block(0, 64, 0, LAVA);
        notify_block_edit(&mut water, &mut lava, &world, 0, 64, 0, AIR, LAVA);
        assert!(lava.is_source(0, 64, 0), "placed lava is a source");
        for _ in 0..12 {
            lava.tick_spread(&mut world);
        }
        assert_eq!(world.get_block(0, 63, 0), LAVA, "the placed lava actually flows");
    }

    #[test]
    fn digging_lava_drops_the_source() {
        let mut world = World::new();
        world.set_block(0, 64, 0, LAVA);
        let mut water = WaterSystem::new();
        let mut lava = LavaSystem::new();
        notify_block_edit(&mut water, &mut lava, &world, 0, 64, 0, AIR, LAVA);
        assert!(lava.is_source(0, 64, 0));
        // Dig it: LAVA → AIR.
        world.set_block(0, 64, 0, AIR);
        notify_block_edit(&mut water, &mut lava, &world, 0, 64, 0, LAVA, AIR);
        assert!(!lava.is_source(0, 64, 0), "dug lava is no longer a source");
    }

    #[test]
    fn water_frozen_lava_source_is_reconciled_so_replacement_flows() {
        let mut world = World::new();
        solid_floor(&mut world, 59, 6);
        let mut water = WaterSystem::new();
        let mut lava = LavaSystem::new();

        // A lava source beside a water source. When water spreads onto/next to
        // the lava, the lava cell freezes to obsidian.
        world.set_block(0, 60, 0, LAVA);
        lava.add_source(0, 60, 0);
        world.set_block(2, 60, 0, WATER);
        water.add_source(2, 60, 0);

        for _ in 0..20 {
            let water_dirty = water.tick_spread(&mut world);
            reconcile_frozen_lava_sources(&mut lava, &world, &water_dirty);
            water.tick_retract(&mut world);
            lava.tick_spread(&mut world);
        }

        // Whatever cell froze, it must not still be a registered lava source.
        if world.get_block(0, 60, 0) == OBSIDIAN {
            assert!(
                !lava.is_source(0, 60, 0),
                "frozen lava source was reconciled out of the source set",
            );
        }

        // The load-bearing consequence: mine the obsidian and re-place lava —
        // it must flow, not sit inert on a phantom source.
        world.set_block(0, 60, 0, AIR);
        notify_block_edit(&mut water, &mut lava, &world, 0, 60, 0, OBSIDIAN, AIR);
        world.set_block(0, 61, 0, LAVA);
        notify_block_edit(&mut water, &mut lava, &world, 0, 61, 0, AIR, LAVA);
        assert!(lava.is_source(0, 61, 0), "re-placed lava registers as a live source");
    }

    /// Phase B1 review — the column index: forgetting a column drops exactly
    /// its sources (negative coordinates included), in one step.
    #[test]
    fn source_set_forgets_exactly_one_column() {
        let mut set = SourceSet::default();
        for p in [(0, 60, 0), (15, 60, 15), (16, 60, 0), (-1, 60, -1), (-16, 60, -16)] {
            assert!(set.insert(p));
        }
        assert!(!set.insert((0, 60, 0)), "no duplicates");
        assert_eq!(set.forget_column(0, 0), 2);
        assert!(!set.contains((0, 60, 0)) && !set.contains((15, 60, 15)));
        assert!(set.contains((16, 60, 0)) && set.contains((-1, 60, -1)));
        assert_eq!(set.forget_column(-1, -1), 2, "(-1, -1) and (-16, -16) share column (-1, -1)");
        assert!(set.remove((16, 60, 0)));
        assert!(!set.remove((16, 60, 0)));
        assert_eq!(set.len(), 0);
    }
}

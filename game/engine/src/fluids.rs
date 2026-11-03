//! Cross-fluid coordination.
//!
//! [`WaterSystem`] and [`LavaSystem`] are independent and hold no reference to
//! each other, so the small amount of logic that must see BOTH lives here and is
//! called from every tick/edit site — the client `GameState` and the
//! authoritative `GameServer` — so the two paths can't drift. Before this module
//! existed, `GameServer` didn't simulate lava at all and the water-freeze path
//! leaked stale lava sources.

use std::collections::VecDeque;

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

/// Sources, spread queue, retract queue, for `GameServer::sim_digest`.
#[cfg(test)]
pub(crate) type FluidDigestView = (Vec<(i32, i32, i32)>, Vec<(i32, i32, i32, u8)>, Vec<(i32, i32, i32)>);

/// Keep slice 2b — a fluid system as an authority checkpoint carries it: its
/// sources as the difference from a rescan of the loaded columns
/// (`SourceSet::delta_from_rescan`), and its pending fronts in queue order.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FluidCheckpoint {
    pub extra: Vec<(i32, i32, i32)>,
    pub missing: Vec<(i32, i32, i32)>,
    pub spread: Vec<(i32, i32, i32, u8)>,
    pub retract: Vec<(i32, i32, i32)>,
}

/// Keep slice 3 — lava as an authority checkpoint carries it. Its queues keep their
/// places in the stored pieces, which is what lets a drain leave those pieces as they are.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LavaCheckpoint {
    pub extra: Vec<(i32, i32, i32)>,
    pub missing: Vec<(i32, i32, i32)>,
    pub spread: PositionedQueue<(i32, i32, i32, u8)>,
    pub retract: PositionedQueue<(i32, i32, i32)>,
}

/// How many entries one stored piece of a [`PositionedQueue`] holds. Part of the
/// checkpoint's schema: changing it is a new schema.
pub(crate) const QUEUE_PIECE_ENTRIES: u64 = 1024;

/// A first-in, first-out queue that knows where each entry lies in the pieces a stored
/// authority holds it in, so that taking the front moves no other entry.
///
/// `head` counts all entries taken since the queue was created, so an entry's place
/// in that run gives its piece and its place in it, and neither ever changes. To write a
/// part-used piece back as it was, the queue keeps the entries of it already taken:
/// `drained`, at most `QUEUE_PIECE_ENTRIES - 1`. They are forgotten when the cursor reaches
/// the next piece. Emptying the queue preserves its cursor and any partial piece, so a
/// later push never reuses an earlier entry's place.
/// Pushing and popping give exactly what a `VecDeque` gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PositionedQueue<T> {
    head: u64,
    drained: Vec<T>,
    live: VecDeque<T>,
}

// By hand, as a derive would ask for `T: Default`.
impl<T> Default for PositionedQueue<T> {
    fn default() -> Self {
        Self { head: 0, drained: Vec::new(), live: VecDeque::new() }
    }
}

impl<T: Copy> PositionedQueue<T> {
    pub(crate) fn push_back(&mut self, entry: T) {
        self.live.push_back(entry);
    }

    pub(crate) fn pop_front(&mut self) -> Option<T> {
        let entry = self.live.pop_front()?;
        self.head += 1;
        if self.head.is_multiple_of(QUEUE_PIECE_ENTRIES) {
            self.drained.clear();
        } else {
            self.drained.push(entry);
        }
        Some(entry)
    }

    /// What is queued, front first.
    pub(crate) fn iter(&self) -> std::collections::vec_deque::Iter<'_, T> {
        self.live.iter()
    }

    /// The queue as a restore read it back. `head`, `drained` and `live` are in the one
    /// form [`Self::pop_front`] leaves, which the reader of the stored pieces checks.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn restored(head: u64, drained: Vec<T>, live: Vec<T>) -> Self {
        Self { head, drained, live: live.into() }
    }

    /// How many entries have been taken since the queue was created.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn head(&self) -> u64 {
        self.head
    }

    /// The entries of the first stored piece that lie behind the cursor.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn drained(&self) -> &[T] {
        &self.drained
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

    /// Keep slice 2b — this set against what a rescan of `loaded` would give
    /// (`register_column_sources`: every `fluid` cell in those columns), as
    /// `(sources the rescan would not find, cells it would add)`, each sorted.
    /// Water registers every water block of a column, so an ocean's set is far
    /// past a checkpoint's cap, while this difference is only what flowed or
    /// was placed since those columns loaded.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn delta_from_rescan(
        &self,
        world: &World,
        loaded: &AHashSet<(i32, i32)>,
        fluid: u16,
    ) -> (Vec<(i32, i32, i32)>, Vec<(i32, i32, i32)>) {
        let mut missing = Vec::new();
        for &(cx, cz) in loaded {
            world.for_each_cell_of(cx, cz, fluid, |cell| {
                if !self.contains(cell) {
                    missing.push(cell);
                }
            });
        }
        let mut extra = Vec::new();
        for (col, cells) in &self.by_column {
            let rescanned = loaded.contains(col);
            for &cell in cells {
                if !rescanned || !world.is_cell_of(col.0, col.1, cell, fluid) {
                    extra.push(cell);
                }
            }
        }
        extra.sort_unstable();
        missing.sort_unstable();
        (extra, missing)
    }

    /// Keep slice 2b — rebuild from a rescan of `loaded` and the difference
    /// [`Self::delta_from_rescan`] recorded.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn rebuild_from_rescan(
        &mut self,
        world: &World,
        loaded: &AHashSet<(i32, i32)>,
        fluid: u16,
        extra: &[(i32, i32, i32)],
        missing: &[(i32, i32, i32)],
    ) {
        self.by_column.clear();
        for &(cx, cz) in loaded {
            world.for_each_cell_of(cx, cz, fluid, |cell| {
                self.insert(cell);
            });
        }
        for &cell in missing {
            self.remove(cell);
        }
        for &cell in extra {
            self.insert(cell);
        }
    }

    /// Every source, sorted (`GameServer::sim_digest`).
    #[cfg(test)]
    pub fn sorted(&self) -> Vec<(i32, i32, i32)> {
        let mut all: Vec<_> = self.by_column.values().flatten().copied().collect();
        all.sort_unstable();
        all
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

    #[test]
    fn an_empty_positioned_queue_keeps_its_cursor_and_partial_piece() {
        let mut q = PositionedQueue::default();
        q.push_back(10u32);
        q.push_back(11);
        assert_eq!(q.pop_front(), Some(10));
        assert_eq!(q.pop_front(), Some(11));
        assert_eq!(q.head(), 2, "emptying the queue never reuses an entry's place");
        assert_eq!(q.drained(), &[10, 11]);
        assert_eq!(q.pop_front(), None);
        q.push_back(12);
        assert_eq!(q.head(), 2);
        assert_eq!(q.pop_front(), Some(12));
        assert_eq!(q.head(), 3);
        assert_eq!(q.drained(), &[10, 11, 12]);
        for i in 3..QUEUE_PIECE_ENTRIES {
            q.push_back(i as u32);
            assert_eq!(q.pop_front(), Some(i as u32));
        }
        assert_eq!(q.head(), QUEUE_PIECE_ENTRIES);
        assert!(q.drained().is_empty(), "only a completed piece's prefix is forgotten");
        assert!(q.iter().next().is_none());
    }

    /// The queue gives what a `VecDeque` gives, for a run of pushes and pops that crosses
    /// piece edges and empties, and `drained` is what its first piece holds behind the head.
    #[test]
    fn a_positioned_queue_pops_and_pushes_as_a_plain_queue_does() {
        let n = QUEUE_PIECE_ENTRIES as usize;
        let mut q = PositionedQueue::default();
        let mut plain = VecDeque::new();
        let mut next = 0u32;
        // (pushes, pops) past each edge there is: mid-piece, on an edge, emptying it.
        for (pushes, pops) in [(3, 1), (n * 2, 5), (0, n), (7, n - 3), (1, 1), (n + 2, n + 3), (5, 0), (0, 2), (n, n + 3), (9, 20)] {
            for _ in 0..pushes {
                q.push_back(next);
                plain.push_back(next);
                next += 1;
            }
            for _ in 0..pops {
                assert_eq!(q.pop_front(), plain.pop_front());
            }
            assert_eq!(q.iter().copied().collect::<Vec<_>>(), plain.iter().copied().collect::<Vec<_>>());
            assert_eq!(q.drained().len() as u64, q.head() % QUEUE_PIECE_ENTRIES, "the part of the first piece behind the head");
            let front = plain.front().copied().unwrap_or(next);
            assert!(q.drained().iter().rev().copied().eq((front - q.drained().len() as u32..front).rev()), "the entries just taken");
        }
        assert_eq!(q.pop_front(), None);
        assert_eq!(q.head(), u64::from(next), "emptying the queue keeps every entry's place");
        assert_eq!(q.drained().len() as u64, q.head() % QUEUE_PIECE_ENTRIES);
    }

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

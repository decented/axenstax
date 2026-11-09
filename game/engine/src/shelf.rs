//! The Commons Shelf (living world Tier 2, S6; spec §6 and §1 `s` and looted).
//!
//! A Commons Shelf is a chest-sized container anyone may put into or take from:
//! no owner, no lock (`chest::ChestTier::Shelf`, so it opens, saves, spills when
//! broken and takes a hopper's items exactly as a wood chest does). A shelf that
//! is kept stocked is a sign that a place is shared and tended, so bloom reads it.
//!
//! **The authority keeps one byte of state in the shelf's `block_meta`**: the low
//! seven bits are how full it is (0..=127, the share of slots holding anything,
//! rounded) and the top bit says it has been *looted*. Meta already travels to
//! every client by `BlockChange` and the chunk push, so each client's bloom reads
//! it with no new wire shape.
//!
//! Looted: if the fill falls by at least half within one world day while the
//! day began at least a quarter full, the shelf is marked looted. Restocking it to
//! half full or more clears the mark. A shelf marked looted dims its region's
//! target (x0.8) until then.
//!
//! Everything here is pure except [`tick`], the one sweep that keeps the byte
//! true. It runs from the shared machine phase (`sim_machines::step`), so the
//! dedicated server, a lending host and single-player share it, and it sees every
//! way a shelf's contents change (the container window, a joiner's window ops, a
//! hopper, a blast) with nothing to forget. The byte follows the contents
//! by at most one machine beat.

use crate::block;
use crate::item::ItemStack;
use crate::protocol::BlockChange;
use crate::world::World;

/// The most a fill can be: seven bits.
pub const FILL_MAX: u8 = 127;
/// The mask of the fill bits of the meta byte.
pub const FILL_MASK: u8 = 0x7F;
/// The top bit of the meta byte: the shelf has been looted.
pub const LOOTED_BIT: u8 = 0x80;
/// A day must have begun at least this full (a quarter, rounded up) for a fall
/// to count as looting.
pub const LOOTED_FROM: u8 = 32;
/// A looted shelf is restocked once it is at least this full (half, rounded up).
pub const RESTOCKED_AT: u8 = 64;
/// `s = SHELF_WEIGHT * fill` (fill 0..1) from the fullest shelf in a region.
pub const SHELF_WEIGHT: f32 = 0.5;
/// What a looted shelf in a region multiplies the region's bloom target by.
pub const LOOTED_TARGET_SCALE: f32 = 0.8;

/// The fill of a shelf holding `slots`: the share holding anything, scaled to
/// 0..=127 and rounded to nearest.
pub fn fill_of(slots: &[Option<ItemStack>]) -> u8 {
    if slots.is_empty() {
        return 0;
    }
    let held = slots.iter().filter(|s| s.as_ref().is_some_and(|st| st.count > 0)).count();
    let n = slots.len();
    ((held * usize::from(FILL_MAX) + n / 2) / n) as u8
}

/// The fill bits of a shelf's meta byte.
pub fn fill_of_meta(meta: u8) -> u8 {
    meta & FILL_MASK
}

/// Is the looted bit set in this meta byte?
pub fn is_looted(meta: u8) -> bool {
    meta & LOOTED_BIT != 0
}

/// The meta byte a shelf should carry now: `fill` (0..=127), and whether it is
/// looted. `prev_meta` is the byte it carries; `day_start_fill` is how full it
/// was when this world day began.
///
/// * Not looted: it becomes looted when the day began at least a quarter full
///   ([`LOOTED_FROM`]) and the fill is now half of that or less.
/// * Looted: it stays looted until the fill is back to half full or more
///   ([`RESTOCKED_AT`]).
pub fn next_meta(prev_meta: u8, day_start_fill: u8, fill: u8) -> u8 {
    let fill = fill.min(FILL_MAX);
    let looted = if is_looted(prev_meta) {
        fill < RESTOCKED_AT
    } else {
        day_start_fill >= LOOTED_FROM && u16::from(fill) * 2 <= u16::from(day_start_fill)
    };
    fill | if looted { LOOTED_BIT } else { 0 }
}

/// The runtime-only record of how full a shelf was when its world day began.
/// It is never saved: after a reload a day begins at whatever the shelf holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DayMark {
    pub day: u32,
    pub start_fill: u8,
}

/// One look at a shelf on world day `day`: updates its [`DayMark`] and returns
/// the meta byte it should now carry.
///
/// A new day starts from the current fill (the first sight that day). When a
/// restock clears the looted mark the day's baseline becomes the restocked fill,
/// so a shelf hovering around half full cannot flap in and out of looted.
pub fn observe(mark: &mut Option<DayMark>, day: u32, prev_meta: u8, fill: u8) -> u8 {
    let fill = fill.min(FILL_MAX);
    let m = mark.get_or_insert(DayMark { day, start_fill: fill });
    if m.day != day {
        *m = DayMark { day, start_fill: fill };
    }
    let next = next_meta(prev_meta, m.start_fill, fill);
    if is_looted(prev_meta) && !is_looted(next) {
        m.start_fill = fill;
    }
    next
}

/// The sweep: for every shelf in the loaded world, bring its meta
/// byte in line with its contents (0 when it has none yet), and push a broadcastable change for each byte that
/// moved. A byte that is already right is not written (no change, no write).
/// `day` is the world day (the day a tick falls in); order is by position, so the
/// changes are deterministic.
pub fn tick(world: &mut World, day: u32, changes: &mut Vec<BlockChange>) {
    for pos in world.shelf_cells() {
        let prev = world.meta_at(pos.0, pos.1, pos.2);
        // A shelf nobody has opened has no contents yet, so its byte is 0. Any
        // other byte is a stale one the cell held before the shelf arrived (or a
        // forged one): write it away (review SF1).
        let Some(fill) = world.chest_at(pos).map(|c| fill_of(&c.slots)) else {
            if prev != 0 {
                world.shelf_days.remove(&pos);
                world.set_meta(pos, 0);
                changes.push(crate::game_loop::broadcast_change(world, pos.0, pos.1, pos.2, block::COMMONS_SHELF));
            }
            continue;
        };
        let mut mark = world.shelf_days.get(&pos).copied();
        let next = observe(&mut mark, day, prev, fill);
        if let Some(m) = mark {
            world.shelf_days.insert(pos, m);
        }
        if next != prev {
            world.set_meta(pos, next);
            changes.push(crate::game_loop::broadcast_change(world, pos.0, pos.1, pos.2, block::COMMONS_SHELF));
        }
    }
}

/// The world day a machine tick falls in. Runtime only (it is never saved), so
/// the tick counter's own origin does not matter.
pub fn day_of_tick(tick: u64) -> u32 {
    (tick / u64::from(crate::sim_clock::DAY_TICKS)) as u32
}

/// What bloom reads of one region's shelves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShelfTerms {
    /// The fullest shelf's fill, 0..=1 (0 when the region has none).
    pub fullest: f32,
    /// A shelf in the region is marked looted.
    pub looted: bool,
}

impl ShelfTerms {
    /// `s`, the tended-shelf factor of `P`: `SHELF_WEIGHT` of the fullest fill.
    pub fn s(&self) -> f32 {
        SHELF_WEIGHT * self.fullest
    }

    /// The factor the region's target is multiplied by.
    pub fn target_scale(&self) -> f32 {
        if self.looted { LOOTED_TARGET_SCALE } else { 1.0 }
    }

    /// Fold one shelf's meta byte in: the fullest fill wins, any looted counts.
    pub fn note(&mut self, meta: u8) {
        self.fullest = self.fullest.max(f32::from(fill_of_meta(meta)) / f32::from(FILL_MAX));
        self.looted |= is_looted(meta);
    }
}

/// Read the shelves of region `key` (one chunk column) from its blocks and meta,
/// as every client can: only the sections that hold a shelf are walked.
pub fn region_terms(world: &World, key: (i32, i32)) -> ShelfTerms {
    let mut t = ShelfTerms::default();
    for cy in 0..=crate::world::MAX_CHUNK_Y {
        let Some(chunk) = world.get_chunk(key.0, cy, key.1).filter(|c| c.has_shelves()) else { continue };
        let (ox, oy, oz) = (key.0 * 16, cy * 16, key.1 * 16);
        chunk.for_each_cell_of(block::COMMONS_SHELF, |x, y, z| {
            t.note(world.meta_at(ox + x as i32, oy + y as i32, oz + z as i32));
        });
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chest::{ChestData, ChestTier};
    use crate::item::MaterialId;

    fn stacks(n: usize, of: usize) -> Vec<Option<ItemStack>> {
        (0..of).map(|i| (i < n).then(|| ItemStack::new_material(MaterialId::Stick, 1))).collect()
    }

    #[test]
    fn fill_is_the_share_of_slots_holding_anything_rounded_to_127() {
        assert_eq!(fill_of(&[]), 0);
        assert_eq!(fill_of(&stacks(0, 27)), 0);
        assert_eq!(fill_of(&stacks(27, 27)), 127);
        assert_eq!(fill_of(&stacks(14, 27)), 66, "14/27 of 127, rounded");
        assert_eq!(fill_of(&stacks(13, 27)), 61);
        assert_eq!(fill_of(&stacks(1, 27)), 5, "one slot is 4.7 -> 5");
        // A stack of nothing holds nothing.
        let mut v = stacks(0, 27);
        v[3] = Some(ItemStack { item: crate::item::Item::Material(MaterialId::Stick), count: 0 });
        assert_eq!(fill_of(&v), 0);
        // Monotone in the slots held.
        let mut last = 0;
        for n in 0..=27 {
            let f = fill_of(&stacks(n, 27));
            assert!(f >= last && f <= FILL_MAX);
            last = f;
        }
    }

    #[test]
    fn looted_when_a_day_that_began_a_quarter_full_falls_by_half() {
        // From exactly a quarter (32): half is 16.
        assert_eq!(next_meta(32, 32, 17), 17, "17 is more than half of 32");
        assert_eq!(next_meta(32, 32, 16), 16 | LOOTED_BIT, "16 is half: looted");
        assert_eq!(next_meta(32, 32, 0), LOOTED_BIT, "emptied: looted, fill 0");
        // From a full shelf: 63 is under half of 127 (63.5), 64 is not.
        assert_eq!(next_meta(127, 127, 63), 63 | LOOTED_BIT);
        assert_eq!(next_meta(127, 127, 64), 64);
        // From 20% (25/127): not a quarter, so never looted whatever the fall.
        let twenty = (0.20 * 127.0f32).round() as u8;
        assert!(twenty < LOOTED_FROM);
        assert_eq!(next_meta(twenty, twenty, 0), 0);
        // 31 is one short of a quarter.
        assert_eq!(next_meta(31, 31, 0), 0);
        // A small dip is not looting.
        assert_eq!(next_meta(100, 100, 80), 80);
    }

    #[test]
    fn a_looted_shelf_stays_looted_until_it_is_half_full_again() {
        for fill in 0..RESTOCKED_AT {
            let m = next_meta(fill | LOOTED_BIT, 127, fill);
            assert!(is_looted(m), "still looted at {fill}");
            assert_eq!(fill_of_meta(m), fill);
        }
        for fill in RESTOCKED_AT..=FILL_MAX {
            let m = next_meta(fill | LOOTED_BIT, 127, fill);
            assert!(!is_looted(m), "restocked at {fill}");
            assert_eq!(m, fill);
        }
        // Over the top of the range is clamped to 127.
        assert_eq!(next_meta(0, 0, 200), FILL_MAX);
    }

    #[test]
    fn the_looted_mark_does_not_flap_at_the_boundary() {
        // A full shelf is cleaned out to 61, restocked to 66, and one slot in or
        // out around the boundary afterwards must not bring the mark back.
        let mut mark = None;
        let mut meta = 127;
        let steps = [127, 61, 66, 61, 66, 61, 66, 61];
        let mut looted_flips = 0;
        for f in steps {
            let next = observe(&mut mark, 1, meta, f);
            if is_looted(next) != is_looted(meta) {
                looted_flips += 1;
            }
            meta = next;
        }
        // Looted once at 61, cleared once at 66, then it stays clear: the day's
        // baseline became the restocked fill. Two flips, not seven.
        assert_eq!(looted_flips, 2, "flips: {looted_flips}");
        assert!(!is_looted(meta));
        // A deeper fall from the restocked level is a fresh looting.
        let m = observe(&mut mark, 1, meta, 30);
        assert!(is_looted(m), "66 -> 30 is more than half off the restocked level");
    }

    #[test]
    fn a_new_day_starts_from_whatever_the_shelf_holds() {
        let mut mark = None;
        // Day 3 begins half empty: the baseline is 40, so 30 is no looting.
        assert_eq!(observe(&mut mark, 3, 40, 40), 40);
        assert_eq!(observe(&mut mark, 3, 40, 30), 30);
        assert_eq!(mark, Some(DayMark { day: 3, start_fill: 40 }));
        // The next day begins at 30 (its first sight), under a quarter: a fall to 5 is no looting.
        assert_eq!(observe(&mut mark, 4, 30, 30), 30);
        assert_eq!(observe(&mut mark, 4, 30, 5), 5);
        assert_eq!(mark, Some(DayMark { day: 4, start_fill: 30 }));
    }

    #[test]
    fn a_new_day_begins_a_looted_shelf_looted_if_it_is_still_empty() {
        let mut mark = Some(DayMark { day: 1, start_fill: 100 });
        let m = observe(&mut mark, 2, 5 | LOOTED_BIT, 5);
        assert!(is_looted(m), "a day does not heal a looted shelf; only stock does");
    }

    #[test]
    fn terms_pick_the_fullest_shelf_and_any_looted_one() {
        let mut t = ShelfTerms::default();
        assert_eq!((t.s(), t.target_scale()), (0.0, 1.0));
        t.note(127);
        assert!((t.s() - 0.5).abs() < 1e-6, "a full shelf: s = 0.5");
        assert_eq!(t.target_scale(), 1.0);
        t.note(30);
        assert!((t.s() - 0.5).abs() < 1e-6, "the fullest wins");
        t.note(3 | LOOTED_BIT);
        assert!((t.target_scale() - 0.8).abs() < 1e-6, "a looted one dims the target");
        assert!((t.s() - 0.5).abs() < 1e-6);
        let mut half = ShelfTerms::default();
        half.note(64);
        assert!((half.s() - 0.5 * 64.0 / 127.0).abs() < 1e-6);
    }

    #[test]
    fn the_sweep_writes_only_what_moved_and_broadcasts_it() {
        let mut w = World::new();
        let pos = (3, 20, 4);
        w.set_block(pos.0, pos.1, pos.2, block::COMMONS_SHELF);
        let mut changes = Vec::new();
        // A shelf nobody opened: no entity, nothing written.
        tick(&mut w, 0, &mut changes);
        assert!(changes.is_empty());
        let mut data = ChestData::for_tier(ChestTier::Shelf);
        for s in data.slots.iter_mut().take(20) {
            *s = Some(ItemStack::new_material(MaterialId::Stick, 1));
        }
        w.insert_chest(pos, data);
        tick(&mut w, 0, &mut changes);
        let want = fill_of(&w.chest_at(pos).unwrap().slots);
        assert_eq!(w.meta_at(pos.0, pos.1, pos.2), want);
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].x, changes[0].y, changes[0].z, changes[0].new_block, changes[0].meta), (3, 20, 4, block::COMMONS_SHELF, want));
        // Nothing moved: nothing written, nothing sent.
        tick(&mut w, 0, &mut changes);
        assert_eq!(changes.len(), 1, "no change, no write");
        // Take most of it out: the byte follows, and the day began at 20 slots, so it is looted.
        for s in w.chest_at_mut(pos).unwrap().slots.iter_mut().take(15) {
            *s = None;
        }
        tick(&mut w, 0, &mut changes);
        let m = w.meta_at(pos.0, pos.1, pos.2);
        assert!(is_looted(m), "20 slots of 27 down to 5 in one day");
        assert_eq!(fill_of_meta(m), fill_of(&w.chest_at(pos).unwrap().slots));
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn breaking_a_shelf_takes_its_meta_and_its_day_with_it() {
        let mut w = World::new();
        let pos = (1, 20, 1);
        w.set_block(pos.0, pos.1, pos.2, block::COMMONS_SHELF);
        w.insert_chest(pos, ChestData::for_tier(ChestTier::Shelf));
        w.chest_at_mut(pos).unwrap().slots[0] = Some(ItemStack::new_material(MaterialId::Stick, 1));
        let mut changes = Vec::new();
        tick(&mut w, 0, &mut changes);
        assert_ne!(w.meta_at(1, 20, 1), 0);
        assert!(w.shelf_days.contains_key(&pos));
        w.set_block(1, 20, 1, block::AIR);
        w.set_block(1, 20, 1, block::OAK_PLANKS);
        assert_eq!(w.meta_at(1, 20, 1), 0, "the next block here does not inherit a fill byte");
        assert!(!w.shelf_days.contains_key(&pos));
    }

    #[test]
    fn a_world_change_forgets_every_shelfs_day() {
        let mut w = World::new();
        let pos = (1, 20, 1);
        w.set_block(pos.0, pos.1, pos.2, block::COMMONS_SHELF);
        w.insert_chest(pos, ChestData::for_tier(ChestTier::Shelf));
        w.chest_at_mut(pos).unwrap().slots[0] = Some(ItemStack::new_material(MaterialId::Stick, 1));
        let mut changes = Vec::new();
        tick(&mut w, 0, &mut changes);
        assert!(w.shelf_days.contains_key(&pos));
        w.clear();
        assert!(w.shelf_days.is_empty(), "a new world's shelf must not start from the old world's day mark");
    }

    #[test]
    fn region_terms_read_blocks_and_meta_alone() {
        let mut w = World::new();
        // Two shelves in region (0, 0), one in region (1, 0), different heights.
        w.set_block(1, 5, 1, block::COMMONS_SHELF);
        w.set_block(2, 70, 3, block::COMMONS_SHELF);
        w.set_block(20, 5, 1, block::COMMONS_SHELF);
        w.set_meta((1, 5, 1), 40);
        w.set_meta((2, 70, 3), 90);
        w.set_meta((20, 5, 1), 5 | LOOTED_BIT);
        let a = region_terms(&w, (0, 0));
        assert!((a.fullest - 90.0 / 127.0).abs() < 1e-6);
        assert!(!a.looted);
        let b = region_terms(&w, (1, 0));
        assert!(b.looted);
        assert_eq!(region_terms(&w, (9, 9)), ShelfTerms::default());
    }
}

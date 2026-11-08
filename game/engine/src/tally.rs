//! Living world S5 (spec `docs/superpowers/specs/2026-11-08-living-world-tier1.md`
//! section 5) — the Tally: session counters for the grown-up panel.
//!
//! Runtime only. Nothing here is saved, sent on the wire or shown to a child:
//! the figures are about the world, never a person, and appear only behind the
//! "Grown-up panel (F3)" switch (decision D16). Counting is one line at the
//! existing seam, `GameState::fire_challenge`, which every mined block, placed
//! block, finished craft, finished smelt and settled vendor trade already
//! passes through; the counters reset with the world.
//!
//! **Stock** is the total item count in the containers (chests and the
//! dispenser family) of the loaded chunks. It is a walk over block entities,
//! so [`StockMeter`] recomputes it at most once a second, and only while the
//! panel asks for it.

use web_time::{Duration, Instant};

use crate::scenario::ChallengeEvent;
use crate::world::World;

/// How often the stock is recomputed while the panel is open.
const STOCK_EVERY: Duration = Duration::from_secs(1);

/// Counters since this world was opened. Saturating, so they never wrap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Blocks broken.
    pub mined: u64,
    /// Blocks placed.
    pub placed: u64,
    /// Items crafted (one per finished craft).
    pub crafted: u64,
    /// Smelts collected from a furnace.
    pub smelted: u64,
    /// Vendor trades settled (`VendorSale`; auctions fire no sale event yet).
    pub trades: u64,
}

impl Tally {
    /// Count a trial event, if it is one of the five the Tally keeps. Called
    /// once from `GameState::fire_challenge`.
    pub fn note_event(&mut self, ev: &ChallengeEvent) {
        let counter = match ev {
            ChallengeEvent::BreakBlock { .. } => &mut self.mined,
            ChallengeEvent::PlaceBlock { .. } => &mut self.placed,
            ChallengeEvent::CraftItem { .. } => &mut self.crafted,
            ChallengeEvent::SmeltItem => &mut self.smelted,
            ChallengeEvent::VendorSale => &mut self.trades,
            _ => return,
        };
        *counter = counter.saturating_add(1);
    }

    /// An accepted block edit on a server (a joiner's, judged by the
    /// authority): a cell that became air was mined, one that became something
    /// else was placed. Same-block edits count nothing.
    pub fn note_edit(&mut self, old: crate::block::BlockId, new: crate::block::BlockId) {
        use crate::block::AIR;
        if old == new {
            return;
        }
        if new == AIR {
            self.mined = self.mined.saturating_add(1);
        } else {
            self.placed = self.placed.saturating_add(1);
        }
    }

    /// Back to zero (the world changed).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The total item count in the containers of the loaded chunks: chests of
/// every tier and the dispenser / dropper family. Items in a furnace, a
/// vendor's stall or a player's hands are not stock.
pub fn stock_in(world: &World) -> u64 {
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let loaded = |p: (i32, i32, i32)| world.has_chunk(p.0.div_euclid(cs), p.1.div_euclid(cs), p.2.div_euclid(cs));
    let count = |chest: &crate::chest::ChestData| -> u64 {
        chest.slots.iter().flatten().map(|s| u64::from(s.count)).sum()
    };
    let chests = world.iter_chests().filter(|(p, _)| loaded(*p)).map(|(_, c)| count(c));
    let dispensers = world.iter_dispensers().filter(|(p, _)| loaded(*p)).map(|(_, d)| count(&d.chest));
    chests.chain(dispensers).sum()
}

/// [`stock_in`], remembered for a second.
#[derive(Clone, Copy, Debug, Default)]
pub struct StockMeter {
    value: u64,
    at: Option<Instant>,
}

impl StockMeter {
    /// The stock now, recomputed only if the last count is a second old (or
    /// there is none). `now` is passed in so a test can step time.
    pub fn read(&mut self, world: &World, now: Instant) -> u64 {
        if self.at.is_none_or(|t| now.saturating_duration_since(t) >= STOCK_EVERY) {
            self.value = stock_in(world);
            self.at = Some(now);
        }
        self.value
    }

    /// Forget the last count (the world changed).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::chest::ChestData;
    use crate::item::{ItemStack, MaterialId};

    fn event_counts(events: &[ChallengeEvent]) -> Tally {
        let mut t = Tally::default();
        for e in events {
            t.note_event(e);
        }
        t
    }

    #[test]
    fn the_five_events_each_move_their_own_counter() {
        let t = event_counts(&[
            ChallengeEvent::BreakBlock { block: Some(block::STONE) },
            ChallengeEvent::BreakBlock { block: None },
            ChallengeEvent::PlaceBlock { block: Some(block::DIRT) },
            ChallengeEvent::CraftItem { item: None },
            ChallengeEvent::CraftItem { item: None },
            ChallengeEvent::CraftItem { item: None },
            ChallengeEvent::SmeltItem,
            ChallengeEvent::VendorSale,
            ChallengeEvent::VendorSale,
        ]);
        assert_eq!(t, Tally { mined: 2, placed: 1, crafted: 3, smelted: 1, trades: 2 });
    }

    #[test]
    fn every_other_event_counts_nothing() {
        let t = event_counts(&[
            ChallengeEvent::EatFood,
            ChallengeEvent::HoldFeast,
            ChallengeEvent::KillMob,
            ChallengeEvent::ClaimCommons,
            ChallengeEvent::HarvestCrop,
        ]);
        assert_eq!(t, Tally::default());
    }

    #[test]
    fn an_accepted_edit_is_a_mine_or_a_placement_and_a_same_block_edit_is_neither() {
        let mut t = Tally::default();
        t.note_edit(block::STONE, block::AIR);
        t.note_edit(block::AIR, block::DIRT);
        t.note_edit(block::DIRT, block::STONE);
        t.note_edit(block::STONE, block::STONE);
        assert_eq!((t.mined, t.placed), (1, 2));
    }

    #[test]
    fn reset_zeroes_everything_and_counters_saturate() {
        let mut t = Tally { mined: u64::MAX, placed: 4, crafted: 5, smelted: 6, trades: 7 };
        t.note_event(&ChallengeEvent::BreakBlock { block: None });
        assert_eq!(t.mined, u64::MAX, "saturates, never wraps");
        t.reset();
        assert_eq!(t, Tally::default());
    }

    fn world_with_chests() -> World {
        let mut w = World::new();
        w.set_block(1, 64, 1, block::STONE); // loads the chunk
        let mut a = ChestData::default();
        a.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 7));
        a.slots[5] = Some(ItemStack::new_block(block::DIRT, 30));
        w.insert_chest((1, 65, 1), a);
        let mut b = ChestData::default();
        b.slots[2] = Some(ItemStack::new_material(MaterialId::Bread, 3));
        w.insert_chest((2, 65, 1), b);
        w
    }

    #[test]
    fn stock_is_the_item_count_over_every_chest() {
        let w = world_with_chests();
        assert_eq!(stock_in(&w), 40);
        assert_eq!(stock_in(&World::new()), 0);
    }

    #[test]
    fn a_container_in_an_unloaded_chunk_is_not_stock() {
        let mut w = world_with_chests();
        let mut far = ChestData::default();
        far.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 9));
        w.insert_chest((4000, 65, 4000), far); // no chunk loaded there
        assert_eq!(stock_in(&w), 40);
    }

    #[test]
    fn the_stock_is_recomputed_at_most_once_a_second() {
        let mut w = world_with_chests();
        let mut meter = StockMeter::default();
        let t0 = Instant::now();
        assert_eq!(meter.read(&w, t0), 40);
        w.chest_at_mut((2, 65, 1)).unwrap().slots[3] = Some(ItemStack::new_material(MaterialId::Bread, 10));
        assert_eq!(meter.read(&w, t0 + Duration::from_millis(400)), 40, "still the last count");
        assert_eq!(meter.read(&w, t0 + Duration::from_millis(1000)), 50, "a second on: counted again");
        meter.reset();
        assert_eq!(meter.read(&w, t0 + Duration::from_millis(1100)), 50, "after a reset it counts at once");
    }
}

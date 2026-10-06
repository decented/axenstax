//! Pure-function crop growth helpers (T1.5).
//!
//! ⚠️ **NOT THE LIVE CROP PATH.** The wired, ticking crop system is
//! `growth.rs` (`collect_crop_positions` + `advance_crops`, driven from
//! `game_loop`). This module has **zero live call sites** and its crop set
//! intentionally **diverges** from `growth.rs` — it's the unit-tested substrate
//! for the wider T1.5 crop family that will replace/extend `growth.rs` when the
//! tier-1.5 economy is wired (engine audit 2026-06-04, E3). **Do not edit this
//! module to change live crop behaviour — edit `growth.rs`.**
//!
//! Each crop family has its own growth-stage block sequence; this
//! module gives a single pure entry-point that takes (stage, water
//! adjacency, tick count, seed) and returns the next stage. Live
//! growth-tick wiring (calling this once per chunk-tick over the
//! plant blocks) is deferred to playtest; this module is the
//! unit-testable substrate it'll consume.
//!
//! Decoupling here lets us:
//! - Tune growth rates by editing a constant table, not the world tick
//! - Test edge cases (boundary conditions, max stage, water bonus)
//! - Reuse the same shape across crop families
//!
//! Spec: `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md`.

// Every pub item below carries an item-level `allow(dead_code)`: the module doc
// above is the reason (zero live call sites, kept as the tested substrate for
// the deferred T1.5 crop family). Anything NEW added here that gets no allow
// will warn — and should either be wired or deleted.

use crate::block::{self, BlockId};

/// Ticks between growth stages under nominal conditions (no water).
/// Matches the existing T1 crop growth (10s @ 20 TPS = 200 ticks).
/// Each crop family can override via `growth_period_ticks`.
#[allow(dead_code)] // dead by design — NOT the live crop path (see module doc); substrate for the deferred T1.5 wiring
pub const DEFAULT_GROWTH_PERIOD_TICKS: u64 = 200;

/// Water-adjacency speed multiplier. Crops next to water grow 2× as
/// fast — the period halves.
#[allow(dead_code)] // dead by design — NOT the live crop path (see module doc); substrate for the deferred T1.5 wiring
pub const WATER_BONUS_FACTOR: u64 = 2;

/// Crop families. Each maps to its 4-stage block-id sequence (or
/// 5-stage for Pumpkin Stem). The crop-block predicate `crop_family`
/// classifies a BlockId into one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // dead by design — NOT the live crop path (see module doc); substrate for the deferred T1.5 wiring
pub enum CropFamily {
    Wheat,
    Carrot,
    Potato,
    Corn,
    SugarBeet,
    Beetroot,
    PumpkinStem,
    BerryBush,
}

#[allow(dead_code)] // dead by design — NOT the live crop path (see module doc); substrate for the deferred T1.5 wiring
impl CropFamily {
    /// Block ids for this family's growth stages, lowest-first.
    pub fn stages(&self) -> &'static [BlockId] {
        match self {
            CropFamily::Wheat => &[
                block::WHEAT_STAGE_0,
                block::WHEAT_STAGE_1,
                block::WHEAT_STAGE_2,
                block::WHEAT_STAGE_3,
            ],
            CropFamily::Carrot => &[
                block::CARROT_STAGE_0,
                block::CARROT_STAGE_1,
                block::CARROT_STAGE_2,
                block::CARROT_STAGE_3,
            ],
            CropFamily::Potato => &[
                block::POTATO_STAGE_0,
                block::POTATO_STAGE_1,
                block::POTATO_STAGE_2,
                block::POTATO_STAGE_3,
            ],
            CropFamily::Corn => &[
                block::CORN_STAGE_0,
                block::CORN_STAGE_1,
                block::CORN_STAGE_2,
                block::CORN_STAGE_3,
            ],
            CropFamily::SugarBeet => &[
                block::SUGAR_BEET_STAGE_0,
                block::SUGAR_BEET_STAGE_1,
                block::SUGAR_BEET_STAGE_2,
                block::SUGAR_BEET_STAGE_3,
            ],
            CropFamily::Beetroot => &[
                block::BEETROOT_STAGE_0,
                block::BEETROOT_STAGE_1,
                block::BEETROOT_STAGE_2,
                block::BEETROOT_STAGE_3,
            ],
            CropFamily::PumpkinStem => &[
                block::PUMPKIN_STEM_0,
                block::PUMPKIN_STEM_1,
                block::PUMPKIN_STEM_2,
                block::PUMPKIN_STEM_3,
                block::PUMPKIN_STEM_4,
            ],
            CropFamily::BerryBush => &[
                block::BERRY_BUSH_0,
                block::BERRY_BUSH_1,
                block::BERRY_BUSH_2,
                block::BERRY_BUSH_3,
            ],
        }
    }

    /// Growth period (ticks per stage) under nominal conditions.
    /// Defaults to the standard 200 ticks; overridden for crops with
    /// longer real-world maturation (pumpkin's stem-to-fruit cycle
    /// takes longer).
    pub fn growth_period_ticks(&self) -> u64 {
        match self {
            CropFamily::Wheat
            | CropFamily::Carrot
            | CropFamily::Potato
            | CropFamily::Corn
            | CropFamily::SugarBeet
            | CropFamily::Beetroot => DEFAULT_GROWTH_PERIOD_TICKS,
            // Pumpkin stem matures slower in MC.
            CropFamily::PumpkinStem => 300,
            // Berry bush regrows quickly between harvests.
            CropFamily::BerryBush => 150,
        }
    }

    /// Mature stage id — the topmost in `stages()`.
    pub fn mature_stage(&self) -> BlockId {
        let s = self.stages();
        s[s.len() - 1]
    }

    /// Initial stage id — first plant.
    pub fn initial_stage(&self) -> BlockId {
        self.stages()[0]
    }
}

/// Classify a BlockId into its crop family + current stage index, if
/// any. Returns None for non-crop blocks.
#[allow(dead_code)] // dead by design — NOT the live crop path (see module doc); substrate for the deferred T1.5 wiring
pub fn crop_family(block_id: BlockId) -> Option<(CropFamily, usize)> {
    for family in [
        CropFamily::Wheat,
        CropFamily::Carrot,
        CropFamily::Potato,
        CropFamily::Corn,
        CropFamily::SugarBeet,
        CropFamily::Beetroot,
        CropFamily::PumpkinStem,
        CropFamily::BerryBush,
    ] {
        if let Some(idx) = family.stages().iter().position(|&id| id == block_id) {
            return Some((family, idx));
        }
    }
    None
}

/// Pure crop tick — does the crop at `block_id` advance to its next
/// stage given `(water_adjacent, ticks_since_plant, seed)`?
///
/// Returns the new block id for the crop position. A mature crop stays
/// at its mature stage (the player must harvest + replant).
///
/// Water adjacency: if true, growth-period halves.
/// Seed: per-position seed XORed with the tick count to add a small
/// stochastic jitter — two crops planted on the same tick will advance
/// on different ticks, so a row of wheat doesn't perfectly synchronise.
#[allow(dead_code)] // dead by design — NOT the live crop path (see module doc); substrate for the deferred T1.5 wiring
pub fn tick_crop(
    block_id: BlockId,
    water_adjacent: bool,
    ticks_since_plant: u64,
    seed: u64,
) -> BlockId {
    let Some((family, stage_idx)) = crop_family(block_id) else {
        return block_id; // Not a crop; no change.
    };
    let stages = family.stages();
    if stage_idx + 1 >= stages.len() {
        return block_id; // Already mature.
    }
    let period = family.growth_period_ticks();
    let effective_period = if water_adjacent { period / WATER_BONUS_FACTOR } else { period };
    // Per-position jitter: bias the boundary by ±10% of the period
    // using a hash of seed so adjacent crops desync.
    let jitter = ((seed ^ 0xA2A2_A2A2) % (effective_period / 5)) as i64
        - (effective_period / 10) as i64;
    let target = (stage_idx as u64 + 1) * effective_period;
    let target = (target as i64 + jitter).max(0) as u64;
    if ticks_since_plant >= target {
        stages[stage_idx + 1]
    } else {
        block_id
    }
}

/// Berry bush harvest — when the mature stage is right-click harvested,
/// it reverts to stage 2 (one-back-from-mature) so it regrows quickly.
/// Pure helper; the world tick reads this on right-click.
#[allow(dead_code)] // dead by design — NOT the live crop path (see module doc); substrate for the deferred T1.5 wiring
pub fn berry_bush_after_harvest() -> BlockId {
    block::BERRY_BUSH_2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_family_recognises_every_stage() {
        for family in [
            CropFamily::Wheat,
            CropFamily::Carrot,
            CropFamily::Potato,
            CropFamily::Corn,
            CropFamily::SugarBeet,
            CropFamily::Beetroot,
            CropFamily::PumpkinStem,
            CropFamily::BerryBush,
        ] {
            for (i, &id) in family.stages().iter().enumerate() {
                let (f, idx) = crop_family(id).expect("crop block");
                assert_eq!(f, family);
                assert_eq!(idx, i);
            }
        }
    }

    #[test]
    fn crop_family_returns_none_for_non_crop() {
        assert!(crop_family(block::STONE).is_none());
        assert!(crop_family(block::OAK_LOG).is_none());
        assert!(crop_family(block::AIR).is_none());
    }

    #[test]
    fn tick_crop_advances_after_growth_period() {
        // No water; default period 200 ticks. Mid-period (50 ticks):
        // shouldn't advance. Past period (300 ticks): should.
        let mid = tick_crop(block::WHEAT_STAGE_0, false, 50, 42);
        assert_eq!(mid, block::WHEAT_STAGE_0,
            "wheat shouldn't advance mid-period");
        // Use a seed where jitter doesn't push the boundary past 300.
        // Period 200, jitter range ±20; max boundary = 220. 300 > 220.
        let advanced = tick_crop(block::WHEAT_STAGE_0, false, 300, 42);
        assert_eq!(advanced, block::WHEAT_STAGE_1,
            "wheat should advance past 300 ticks");
    }

    #[test]
    fn tick_crop_water_doubles_speed() {
        // 100 ticks with water (effective period 100) should advance;
        // 100 ticks without water should not.
        let with_water = tick_crop(block::WHEAT_STAGE_0, true, 150, 42);
        let without_water = tick_crop(block::WHEAT_STAGE_0, false, 150, 42);
        assert_eq!(with_water, block::WHEAT_STAGE_1);
        assert_eq!(without_water, block::WHEAT_STAGE_0);
    }

    #[test]
    fn mature_crops_stay_mature() {
        // No more advancement once at the mature stage.
        let stays = tick_crop(block::WHEAT_STAGE_3, true, 99999, 42);
        assert_eq!(stays, block::WHEAT_STAGE_3);
    }

    #[test]
    fn pumpkin_stem_grows_through_five_stages() {
        let mut id = block::PUMPKIN_STEM_0;
        for _ in 0..5 {
            id = tick_crop(id, true, 99999, 42);
        }
        assert_eq!(id, block::PUMPKIN_STEM_4);
    }

    #[test]
    fn non_crop_block_unchanged() {
        let stone = tick_crop(block::STONE, true, 99999, 42);
        assert_eq!(stone, block::STONE);
    }

    #[test]
    fn berry_bush_harvest_drops_to_stage_two() {
        // After harvest, bush is at stage 2 — three ticks worth of
        // regrowth still to go before it's mature again.
        assert_eq!(berry_bush_after_harvest(), block::BERRY_BUSH_2);
        // Confirm crop_family agrees this is mid-stage.
        let (family, idx) = crop_family(berry_bush_after_harvest()).unwrap();
        assert_eq!(family, CropFamily::BerryBush);
        assert_eq!(idx, 2);
    }

    #[test]
    fn growth_jitter_keeps_adjacent_crops_desynced() {
        // Two crops planted on the same tick with different per-position
        // seeds should NOT advance on exactly the same tick. Check by
        // finding the tick each advances at.
        let mut tick_a: Option<u64> = None;
        let mut tick_b: Option<u64> = None;
        for t in 0..400 {
            if tick_a.is_none() {
                let id = tick_crop(block::WHEAT_STAGE_0, false, t, 0xAAAA);
                if id == block::WHEAT_STAGE_1 { tick_a = Some(t); }
            }
            if tick_b.is_none() {
                let id = tick_crop(block::WHEAT_STAGE_0, false, t, 0xBBBB);
                if id == block::WHEAT_STAGE_1 { tick_b = Some(t); }
            }
            if tick_a.is_some() && tick_b.is_some() { break; }
        }
        assert!(tick_a.is_some() && tick_b.is_some());
        assert_ne!(tick_a, tick_b,
            "adjacent crops with different seeds should desync");
    }

    #[test]
    fn each_family_has_correct_stage_count() {
        // Wheat/Carrot/Potato/Corn/SugarBeet/Beetroot/BerryBush: 4 stages.
        // PumpkinStem: 5 stages.
        assert_eq!(CropFamily::Wheat.stages().len(), 4);
        assert_eq!(CropFamily::PumpkinStem.stages().len(), 5);
        assert_eq!(CropFamily::BerryBush.stages().len(), 4);
    }

    #[test]
    fn mature_stage_helper_returns_topmost() {
        assert_eq!(CropFamily::Wheat.mature_stage(), block::WHEAT_STAGE_3);
        assert_eq!(CropFamily::PumpkinStem.mature_stage(), block::PUMPKIN_STEM_4);
        assert_eq!(CropFamily::BerryBush.mature_stage(), block::BERRY_BUSH_3);
    }

    #[test]
    fn initial_stage_helper_returns_lowest() {
        assert_eq!(CropFamily::Wheat.initial_stage(), block::WHEAT_STAGE_0);
        assert_eq!(CropFamily::SugarBeet.initial_stage(), block::SUGAR_BEET_STAGE_0);
    }

    #[test]
    fn pumpkin_stem_has_longer_period_than_wheat() {
        assert!(CropFamily::PumpkinStem.growth_period_ticks()
            > CropFamily::Wheat.growth_period_ticks());
    }

    #[test]
    fn berry_bush_has_shorter_period_than_wheat() {
        assert!(CropFamily::BerryBush.growth_period_ticks()
            < CropFamily::Wheat.growth_period_ticks());
    }
}

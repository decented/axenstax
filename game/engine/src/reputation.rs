//! Spec 19 phase 9 — per-player-per-village reputation ledger.
//!
//! Each player carries a [`Reputation`] map (village grid cell → i16 score).
//! Score is bounded to `[REP_MIN, REP_MAX]` and divides into five tiers:
//!
//! | tier      | range            | behaviour                              |
//! |-----------|------------------|----------------------------------------|
//! | Hostile   | ≤ -50            | dialogues refuse interaction           |
//! | Wary      | -50 .. -10       | rewards reduced 25 %                   |
//! | Neutral   | -10 .. 10        | baseline rewards                       |
//! | Friendly  | 10 .. 50         | rewards boosted 10 %                   |
//! | Beloved   | > 50             | bonus rare quests unlocked (Phase 10+) |
//!
//! **Killing a villager**: -25 with a 30 s per-player cooldown so a kid in
//! a rage-loop only loses rep once.
//! **Completing a quest**: + the quest's `reward.reputation` (5-20 typically).
//! **Natural decay**: ±1 per in-game day toward zero — kid-friendly, lets
//! past mistakes fade.

use ahash::AHashMap;

/// Compact id of a village. Matches Phase 4's `(grid_x, grid_z)` key.
pub type VillageId = (i32, i32);

pub const REP_MIN: i16 = -100;
pub const REP_MAX: i16 = 100;

/// Reputation tier the dialogue path can read to gate behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Hostile,
    Wary,
    Neutral,
    Friendly,
    Beloved,
}

impl Tier {
    pub fn from_score(score: i16) -> Tier {
        match score {
            v if v <= -50 => Tier::Hostile,
            v if v <= -10 => Tier::Wary,
            v if v < 10 => Tier::Neutral,
            v if v < 50 => Tier::Friendly,
            _ => Tier::Beloved,
        }
    }

    /// Multiplier applied to reward sats + items. Wary loses 25 %, Friendly
    /// gains 10 %, Beloved gains 25 %.
    pub fn reward_multiplier(self) -> f32 {
        match self {
            Tier::Hostile => 0.0,
            Tier::Wary => 0.75,
            Tier::Neutral => 1.0,
            Tier::Friendly => 1.10,
            Tier::Beloved => 1.25,
        }
    }
}

/// Per-player reputation across all villages they've interacted with.
/// Not directly Serializable — ahash::AHashMap has no serde feature; Phase 11
/// (save/load) flattens this to a `Vec<(VillageId, i16)>` for the wire.
#[derive(Clone, Debug, Default)]
pub struct Reputation {
    pub per_village: AHashMap<VillageId, i16>,
}

impl Reputation {
    /// Current score with the given village (defaults to 0 = Neutral).
    pub fn score(&self, vid: VillageId) -> i16 {
        *self.per_village.get(&vid).unwrap_or(&0)
    }

    /// Apply a delta, clamping to [REP_MIN, REP_MAX]. Returns the new score.
    pub fn adjust(&mut self, vid: VillageId, delta: i16) -> i16 {
        let cur = self.score(vid);
        let new = cur.saturating_add(delta).clamp(REP_MIN, REP_MAX);
        self.per_village.insert(vid, new);
        new
    }

    /// Tier the dialogue gates on.
    pub fn tier(&self, vid: VillageId) -> Tier {
        Tier::from_score(self.score(vid))
    }

    /// Natural decay step — every entry drifts one point toward zero.
    /// Callers throttle the cadence (Spec 19: once per in-game day).
    pub fn decay_once(&mut self) {
        for score in self.per_village.values_mut() {
            if *score > 0 {
                *score -= 1;
            } else if *score < 0 {
                *score += 1;
            }
        }
    }
}

/// Pick the village whose anchor is nearest to `world_pos`. Returns `None`
/// if there are no anchors loaded.
pub fn village_at_position(
    world: &crate::world::World,
    world_pos: glam::Vec3,
) -> Option<VillageId> {
    let mut best: Option<(VillageId, f32)> = None;
    for (&vid, &anchor) in &world.village_anchors {
        let v = glam::Vec3::new(anchor[0] as f32, anchor[1] as f32, anchor[2] as f32);
        let d = (v - world_pos).length();
        if best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((vid, d));
        }
    }
    best.map(|(v, _)| v)
}

/// Penalty (negative delta) for killing a villager.
pub const VILLAGER_KILL_PENALTY: i16 = -25;

/// 30-second cooldown after deducting villager-kill penalty (Spec 19) so
/// rapid kills only count once.
pub const VILLAGER_KILL_REP_COOLDOWN_TICKS: u64 = 600;

/// One Minecraft day at the engine's tick rate. Used by the natural-decay
/// timer. With `world_time_step = 4` (alpha-fast 5-min day) the value here
/// is the *engine* tick budget per natural day — 6000 ticks ≈ 5 min @ 20 TPS.
pub const REPUTATION_DECAY_INTERVAL_TICKS: u64 = 6000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_thresholds_match_spec() {
        assert_eq!(Tier::from_score(-100), Tier::Hostile);
        assert_eq!(Tier::from_score(-50), Tier::Hostile);
        assert_eq!(Tier::from_score(-49), Tier::Wary);
        assert_eq!(Tier::from_score(-10), Tier::Wary);
        assert_eq!(Tier::from_score(-9), Tier::Neutral);
        assert_eq!(Tier::from_score(0), Tier::Neutral);
        assert_eq!(Tier::from_score(9), Tier::Neutral);
        assert_eq!(Tier::from_score(10), Tier::Friendly);
        assert_eq!(Tier::from_score(49), Tier::Friendly);
        assert_eq!(Tier::from_score(50), Tier::Beloved);
        assert_eq!(Tier::from_score(100), Tier::Beloved);
    }

    #[test]
    fn adjust_clamps_to_range() {
        let mut r = Reputation::default();
        r.adjust((0, 0), 200);
        assert_eq!(r.score((0, 0)), REP_MAX);
        r.adjust((0, 0), -500);
        assert_eq!(r.score((0, 0)), REP_MIN);
    }

    #[test]
    fn villager_kill_pushes_score_below_neutral() {
        let mut r = Reputation::default();
        r.adjust((0, 0), VILLAGER_KILL_PENALTY);
        assert!(r.score((0, 0)) < 0);
    }

    #[test]
    fn decay_walks_back_toward_zero() {
        let mut r = Reputation::default();
        r.adjust((1, 2), 5);
        r.adjust((3, 4), -7);
        r.decay_once();
        assert_eq!(r.score((1, 2)), 4);
        assert_eq!(r.score((3, 4)), -6);
        // Zero stays zero (no thrashing).
        r.adjust((5, 5), 0);
        r.decay_once();
        assert_eq!(r.score((5, 5)), 0);
    }

    #[test]
    fn reward_multiplier_matches_tier() {
        assert_eq!(Tier::Hostile.reward_multiplier(), 0.0);
        assert_eq!(Tier::Wary.reward_multiplier(), 0.75);
        assert_eq!(Tier::Neutral.reward_multiplier(), 1.0);
        assert!((Tier::Friendly.reward_multiplier() - 1.10).abs() < 1e-4);
        assert_eq!(Tier::Beloved.reward_multiplier(), 1.25);
    }
}

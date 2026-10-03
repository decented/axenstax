//! Historical Pivot Sub-Foundation 2 (HP-2) — Hyena AI.
//!
//! Savanna-only pack hunter. State machine:
//!
//!  - `Lazy` (day) — sits or wanders slowly within ~10 blocks of pack
//!    centre. Ignores player unless attacked.
//!  - `Hunt` (night) — actively pathfinds toward nearest player or
//!    villager within 24 blocks; melee attack on contact.
//!  - `Aggro` — same as Bear; hit by player triggers a 30 s revenge
//!    window regardless of day/night.
//!
//! Pack-spawn: when a hyena spawn rolls, spawn 2-4 hyenas in a tight
//! cluster (within a 1-block radius). Pack-boost: when ≥3 hyenas are
//! within 6 blocks of one another, all hyenas in the cluster gain +1
//! damage (passive modifier — not a state).
//!
//! Pure-function shape: pack-boost detection takes a list of positions
//! and returns the boosted positions. Day/night transition takes a
//! `world_time` integer and emits the desired state.
//!
//! Spec: `docs/foundations/2026-05-22-historical-pivot-wild-animals.md`.

use serde::{Deserialize, Serialize};

/// Minimum pack size for a spawn cluster.
pub const PACK_SIZE_MIN: u32 = 2;
/// Maximum pack size for a spawn cluster.
pub const PACK_SIZE_MAX: u32 = 4;

/// Pack-boost activates when this many or more hyenas are within
/// `PACK_BOOST_RADIUS` of each other.
#[cfg_attr(not(test), allow(dead_code))]
pub const PACK_BOOST_MIN: usize = 3;

/// Cluster-detection radius for pack boost, in blocks.
#[cfg_attr(not(test), allow(dead_code))]
pub const PACK_BOOST_RADIUS: f32 = 6.0;

// BRIDGE: `PACK_BOOST_DAMAGE_BONUS` and `HUNT_SCAN_RADIUS` still have no
// consumer (Task 13, bug-hardening, 2026-07-07 wired up `on_hit_by_player` +
// `AGGRO_DURATION_TICKS` — see below — but left these two flagged). Contact
// damage against players is already applied generically for every
// `MobCategory::Hostile` mob by `combat::tick_mob_attacks`, which has no
// per-species damage hook to add a pack bonus onto, and `HUNT_SCAN_RADIUS`
// would need new nearby-hyena scanning infrastructure that doesn't exist
// yet (`dispatch_hyenas` targets the single nearest/attacking player, not a
// scanned set). Wire these in when that pack-scan infrastructure lands; see
// `pack_boosted_indices` and `on_hit_by_player` below, which at least have
// test coverage for the pure logic.
/// Bonus damage applied when pack-boost fires.
#[allow(dead_code)]
pub const PACK_BOOST_DAMAGE_BONUS: u8 = 1;

/// Hunt-state target scan radius.
#[allow(dead_code)]
pub const HUNT_SCAN_RADIUS: f32 = 24.0;

/// Aggro window after being hit. 30 s = 600 ticks.
pub const AGGRO_DURATION_TICKS: u32 = 600;

/// World time range for "night" — same threshold the spawner uses,
/// roughly midnight ± most of a night. Day if `world_time < 4500` or
/// `> 19500` in the 24000-tick day cycle (compute_sun mapping).
///
/// Implementation aligns with `camera::compute_sun`'s brightness < 0.3
/// threshold for night detection, derived empirically: at world_time
/// in [4500, 19500] the sun is high enough that brightness > 0.3.
pub fn is_night(world_time: u32) -> bool {
    let t = world_time % 24000;
    !(4500..=19500).contains(&t)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HyenaAiState {
    Lazy,
    Hunt,
    /// Hit by a player — chase `attacker_pidx` (the player-slot index that
    /// landed the hit) for `ticks_remaining` more ticks, day or night. Same
    /// slot-index rationale as `bear_ai::BearAiState::Aggro` — see there.
    Aggro { ticks_remaining: u32, attacker_pidx: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyenaData {
    pub state: HyenaAiState,
}

impl Default for HyenaData {
    fn default() -> Self {
        Self { state: HyenaAiState::Lazy }
    }
}

impl HyenaData {
    pub fn new() -> Self { Self::default() }

    /// Same story as `bear_ai::BearData::is_aggro` — `dispatch_hyenas`
    /// matches `HyenaAiState::Aggro { .. }` directly (it needs
    /// `attacker_pidx`), so this stays test-only for now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_aggro(&self) -> bool {
        matches!(self.state, HyenaAiState::Aggro { .. })
    }
}

/// Pack-spawn size — deterministic on the spawn seed. Returns a value in
/// `PACK_SIZE_MIN..=PACK_SIZE_MAX`.
pub fn pack_size_for_seed(seed: u32) -> u32 {
    PACK_SIZE_MIN + (seed % (PACK_SIZE_MAX - PACK_SIZE_MIN + 1))
}

/// Pack-spawn offsets — returns up to `count` (x,z) offsets within a
/// 1-block radius of the spawn origin. Tight cluster per the spec.
pub fn pack_offsets(count: u32) -> Vec<(i32, i32)> {
    let candidates = [
        (0, 0), (1, 0), (-1, 0), (0, 1), (0, -1),
        (1, 1), (-1, -1), (1, -1), (-1, 1),
    ];
    candidates.iter().take(count as usize).copied().collect()
}

/// Detect pack-boost clusters. Given a slice of hyena positions, return
/// the indices into that slice for hyenas in a cluster of ≥
/// `PACK_BOOST_MIN` within `PACK_BOOST_RADIUS` of one another.
///
/// Pure on `positions`. Cluster membership is "this hyena has at least
/// `PACK_BOOST_MIN - 1` other hyenas within radius" — the minimum
/// definition that captures the spec's "3 within 6 blocks of each
/// other" intent without doing graph traversal.
#[cfg_attr(not(test), allow(dead_code))]
pub fn pack_boosted_indices(positions: &[(f32, f32, f32)]) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, &(x, y, z)) in positions.iter().enumerate() {
        let mut neighbours = 0usize;
        for (j, &(x2, y2, z2)) in positions.iter().enumerate() {
            if i == j { continue; }
            let dx = x - x2;
            let dy = y - y2;
            let dz = z - z2;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            if dist_sq <= PACK_BOOST_RADIUS * PACK_BOOST_RADIUS {
                neighbours += 1;
            }
        }
        if neighbours + 1 >= PACK_BOOST_MIN {
            out.push(i);
        }
    }
    out
}

/// Day → Lazy, night → Hunt, hit → Aggro (timed). Pure on `HyenaData`.
pub fn tick(
    hyena: &mut HyenaData,
    world_time: u32,
) {
    match &mut hyena.state {
        HyenaAiState::Aggro { ticks_remaining, .. } => {
            if *ticks_remaining > 0 { *ticks_remaining -= 1; }
            if *ticks_remaining == 0 {
                hyena.state = if is_night(world_time) { HyenaAiState::Hunt } else { HyenaAiState::Lazy };
            }
        }
        _ => {
            hyena.state = if is_night(world_time) { HyenaAiState::Hunt } else { HyenaAiState::Lazy };
        }
    }
}

/// Player hit a hyena — enter Aggro for 30 s regardless of time of day,
/// targeting the hitting player's slot. Wired from `combat::player_attack`
/// (melee) and `entity::tick_projectiles` (arrows) via
/// `combat::notify_hit_bear_or_hyena`.
pub fn on_hit_by_player(hyena: &mut HyenaData, attacker_pidx: usize) {
    hyena.state = HyenaAiState::Aggro { ticks_remaining: AGGRO_DURATION_TICKS, attacker_pidx };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_lazy() {
        let h = HyenaData::new();
        assert!(matches!(h.state, HyenaAiState::Lazy));
        assert!(!h.is_aggro());
    }

    #[test]
    fn pack_size_in_two_to_four_range() {
        for seed in 0u32..200 {
            let n = pack_size_for_seed(seed);
            assert!((PACK_SIZE_MIN..=PACK_SIZE_MAX).contains(&n),
                "seed {seed}: size {n} out of range");
        }
    }

    #[test]
    fn pack_size_samples_every_value_across_seeds() {
        let mut seen = std::collections::HashSet::new();
        for seed in 0u32..200 {
            seen.insert(pack_size_for_seed(seed));
        }
        for n in PACK_SIZE_MIN..=PACK_SIZE_MAX {
            assert!(seen.contains(&n), "no seed produced pack-size {n}");
        }
    }

    #[test]
    fn pack_offsets_returns_requested_count() {
        let four = pack_offsets(4);
        assert_eq!(four.len(), 4);
        // First element is the origin.
        assert_eq!(four[0], (0, 0));
    }

    #[test]
    fn pack_offsets_capped_at_nine() {
        let huge = pack_offsets(100);
        assert!(huge.len() <= 9);
    }

    #[test]
    fn is_night_during_midnight() {
        assert!(is_night(0));
        assert!(is_night(1000));
        assert!(is_night(22000));
    }

    #[test]
    fn is_day_at_noon() {
        assert!(!is_night(12000));
        assert!(!is_night(10000));
        assert!(!is_night(15000));
    }

    #[test]
    fn day_tick_lazy_night_tick_hunt() {
        let mut h = HyenaData::new();
        tick(&mut h, 12000);
        assert!(matches!(h.state, HyenaAiState::Lazy));
        tick(&mut h, 0);
        assert!(matches!(h.state, HyenaAiState::Hunt));
    }

    #[test]
    fn on_hit_sets_aggro_window_and_holds_through_tick() {
        let mut h = HyenaData::new();
        on_hit_by_player(&mut h, 4);
        assert!(h.is_aggro());
        tick(&mut h, 12000); // day; aggro must NOT immediately revert.
        assert!(h.is_aggro());
        match h.state {
            HyenaAiState::Aggro { ticks_remaining, attacker_pidx } => {
                assert_eq!(ticks_remaining, AGGRO_DURATION_TICKS - 1);
                assert_eq!(attacker_pidx, 4, "should target the player slot that landed the hit");
            }
            _ => panic!("expected Aggro"),
        }
    }

    #[test]
    fn aggro_decays_then_reverts_to_day_or_night_state() {
        let mut h = HyenaData::new();
        h.state = HyenaAiState::Aggro { ticks_remaining: 1, attacker_pidx: 0 };
        // 1→0 and instant revert to day state (Lazy).
        tick(&mut h, 12000);
        assert!(matches!(h.state, HyenaAiState::Lazy));

        h.state = HyenaAiState::Aggro { ticks_remaining: 1, attacker_pidx: 0 };
        tick(&mut h, 0); // night → after timer expires, Hunt.
        assert!(matches!(h.state, HyenaAiState::Hunt));
    }

    #[test]
    fn aggro_preserves_attacker_pidx_across_ticks() {
        let mut h = HyenaData::new();
        on_hit_by_player(&mut h, 2);
        tick(&mut h, 12000);
        match h.state {
            HyenaAiState::Aggro { attacker_pidx, .. } => assert_eq!(attacker_pidx, 2),
            _ => panic!("expected Aggro state to persist"),
        }
    }

    #[test]
    fn pack_boost_fires_when_three_in_six_blocks() {
        let positions = vec![
            (0.0, 64.0, 0.0),
            (1.0, 64.0, 0.0),
            (0.0, 64.0, 1.0),
        ];
        let boosted = pack_boosted_indices(&positions);
        assert_eq!(boosted.len(), 3, "all three should be boosted");
    }

    #[test]
    fn pack_boost_skips_lone_hyena() {
        let positions = vec![
            (0.0, 64.0, 0.0),
            (100.0, 64.0, 100.0),
        ];
        let boosted = pack_boosted_indices(&positions);
        assert!(boosted.is_empty(), "two isolated hyenas: no boost");
    }

    #[test]
    fn pack_boost_skips_when_only_two_are_close() {
        let positions = vec![
            (0.0, 64.0, 0.0),
            (1.0, 64.0, 0.0),
        ];
        let boosted = pack_boosted_indices(&positions);
        assert!(boosted.is_empty(), "two hyenas don't make a pack boost");
    }

    #[test]
    fn pack_boost_skips_outliers_in_mixed_cluster() {
        // Three close + one far away. Boost fires on the three; the
        // outlier is not boosted (it has no neighbours within radius).
        let positions = vec![
            (0.0, 64.0, 0.0),
            (1.0, 64.0, 0.0),
            (0.0, 64.0, 1.0),
            (100.0, 64.0, 100.0),
        ];
        let boosted = pack_boosted_indices(&positions);
        assert_eq!(boosted.len(), 3);
        assert!(!boosted.contains(&3));
    }
}

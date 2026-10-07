//! Spec 28d.nostrich v2 — per-mob persistent state + AI state
//! machine + berry-feed taming + tamed behaviours.
//!
//! v1 (Spec 28d.nostrich) shipped wild Nostriches via a global tick
//! stagger for egg-laying. v2 lifts the per-mob state into a hecs
//! ECS component (`NostrichData`) attached to every Nostrich
//! entity, replacing the global stagger with per-entity lay +
//! feather timers, adding the Idle/Flee/Kick/Charge/Follow/Sit
//! state machine, and wiring berry-feed taming.
//!
//! Pure logic + tests live here; live ECS wiring lives in
//! `entity::spawn_mob` (component attach) and `game_loop::tick`
//! (per-tick state update + lay/feather emission).

use serde::{Deserialize, Serialize};

use crate::tameable::{self, OwnershipData, TameAttempt};
use crate::item::MaterialId;

/// Number of ticks between egg lays for a Nostrich. 24,000 ticks =
/// 1 in-game day @ 1× time. Both wild + tamed use the same rate;
/// difference is wild drops at the Nostrich's wandering position,
/// tamed drops at `home_pos` if set.
pub const LAY_PERIOD_TICKS: u32 = 24_000;

/// Number of ticks between passive feather sheds for a Nostrich.
/// 48,000 ticks = 2 in-game days. Only tamed Nostriches shed
/// passively — wild ones force the player into a kill path which
/// triggers the Vow.
pub const FEATHER_PERIOD_TICKS: u32 = 48_000;

/// Berry-feed taming success rate. 1-in-3 per feed, matching the
/// wolf bone-tame rate at the underlying primitive.
pub const TAME_SUCCESS_NUMER: u32 = 1;
pub const TAME_SUCCESS_DENOM: u32 = 3;

/// Maximum follow distance for a tamed Nostrich. When the owner
/// exceeds this, the AI flips to Follow state and the tick steers
/// the Nostrich toward the owner. Used inside `advance_state`; ticked
/// live by `species_ai::dispatch_nostriches` (wired 2026-07-11).
pub const FOLLOW_MAX_DISTANCE: f32 = 8.0;

/// Stop-following distance — the Nostrich halts following when it gets within
/// this. Same anti-oscillation pattern as wolves (see `wolf::FOLLOW_MIN_DISTANCE`
/// and its `dist > FOLLOW_MIN_DISTANCE` guard); enforced by the
/// `dispatch_nostriches` Follow arm.
pub const FOLLOW_MIN_DISTANCE: f32 = 4.0;

/// Flee duration after taking damage. 5 seconds at 20 TPS.
pub const FLEE_DURATION_TICKS: u32 = 100;

/// Kick window — after a kick lands, the Nostrich locks into Kick
/// state for this many ticks (one frame of dealing damage, then
/// pivot to Flee). 4 ticks = 0.2 s, just long enough to register
/// the hit visually.
pub const KICK_WINDOW_TICKS: u32 = 4;

/// Kick damage. 6 HP — meaningful but not lethal at full HP.
pub const KICK_DAMAGE: f32 = 6.0;

/// Per-mob persistent state for a Nostrich. Stored as a hecs ECS
/// component on every Nostrich entity at spawn time. Survives via
/// the entity's own lifecycle; not persisted across saves on v2
/// (alpha-ephemeral — see Spec doc Open Q #X).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NostrichData {
    /// Generic ownership + damage-tracking, reused from the wolf
    /// tameable framework.
    pub ownership: OwnershipData,
    pub state: NostrichAiState,
    /// Ticks remaining until the next egg lay. Counts down per
    /// tick; on hit 0, the game loop spawns an egg and resets.
    pub lay_ticks_remaining: u32,
    /// Ticks remaining until the next passive feather shed. Only
    /// tamed Nostriches actually shed (the tick handler gates on
    /// `is_tamed`). Wild ones still tick down for save-stability
    /// in case taming happens mid-cycle.
    pub feather_ticks_remaining: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NostrichAiState {
    /// Default — wander idly, peck the ground. Both wild and tamed
    /// start here until something changes.
    Idle,
    /// Flee from an attacker. `until_tick` is the absolute world
    /// tick when this state expires back to Idle.
    Flee { attacker_id: u64, until_tick: u64 },
    /// Brief one-shot retaliate-kick. Lasts `KICK_WINDOW_TICKS`
    /// before pivoting to Flee.
    Kick { target_id: u64, until_tick: u64 },
    /// Charging an attacker who cornered the Nostrich. Deals double
    /// damage on contact. Pivots to Flee on contact or timeout.
    Charge { target_id: u64, until_tick: u64 },
    /// Tamed: follow the owner toward their current position.
    Follow,
    /// Tamed: sit and stay. Right-click toggle between Follow/Sit.
    Sit,
}

impl NostrichData {
    /// New untamed Nostrich at spawn. Lay + feather timers start
    /// at their full period so a freshly-spawned Nostrich takes
    /// 1 in-game day before its first egg.
    pub fn untamed() -> Self {
        Self {
            ownership: OwnershipData::untamed(),
            state: NostrichAiState::Idle,
            lay_ticks_remaining: LAY_PERIOD_TICKS,
            feather_ticks_remaining: FEATHER_PERIOD_TICKS,
        }
    }

    pub fn is_tamed(&self) -> bool {
        self.ownership.is_tamed()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn owner_pubkey(&self) -> &str {
        &self.ownership.owner_pubkey
    }
}

/// Outcome of an `attempt_tame_with_berry` call. Drives the toast +
/// inventory-consume decision in the right-click handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BerryFeedOutcome {
    /// Tamed successfully — `owner_pubkey` was recorded.
    Tamed,
    /// Berry was consumed; tame roll failed; try again.
    BerryConsumed,
    /// Nostrich was already tamed — no berry consumed, no change.
    AlreadyTamed,
    /// Caller didn't hold a berry — no-op.
    NotAFeed,
}

/// Try to tame a Nostrich by hand-feeding one Mixed Berry.
/// `held_material` is what the player is currently holding;
/// `seed` is a deterministic per-(player, mob, tick) RNG seed.
/// Returns the outcome; caller handles berry-decrement +
/// toast + state-flip side effects.
pub fn attempt_tame_with_berry(
    nostrich: &mut NostrichData,
    held_material: Option<MaterialId>,
    owner_pubkey: &str,
    seed: u64,
) -> BerryFeedOutcome {
    if held_material != Some(MaterialId::Berries) {
        return BerryFeedOutcome::NotAFeed;
    }
    let attempt = tameable::attempt_tame_generic(
        &mut nostrich.ownership,
        owner_pubkey,
        seed,
        TAME_SUCCESS_NUMER,
        TAME_SUCCESS_DENOM,
    );
    match attempt {
        TameAttempt::Succeeded => {
            // On success, the tamed Nostrich shifts to Follow state
            // so it immediately starts trailing the new owner.
            nostrich.state = NostrichAiState::Follow;
            BerryFeedOutcome::Tamed
        }
        TameAttempt::Failed => BerryFeedOutcome::BerryConsumed,
        TameAttempt::AlreadyTamed => BerryFeedOutcome::AlreadyTamed,
    }
}

/// Toggle a tamed Nostrich's state between Follow and Sit. Caller
/// gates on `is_owned_by(player_pubkey)`. No-op on untamed.
pub fn toggle_sit_follow(nostrich: &mut NostrichData) {
    if !nostrich.is_tamed() {
        return;
    }
    nostrich.state = match nostrich.state {
        NostrichAiState::Sit => NostrichAiState::Follow,
        _ => NostrichAiState::Sit,
    };
}

/// Called when a Nostrich takes damage. Flips state to Flee and
/// records the attacker. If the attacker is within melee range,
/// also schedules a retaliate Kick (caller deals the damage on
/// the next tick if state == Kick).
///
/// Returns `Some(KICK_DAMAGE)` if the attacker should also receive
/// a retaliate-kick hit *this same tick*. Caller routes through
/// the existing combat path.
pub fn on_damaged(
    nostrich: &mut NostrichData,
    attacker_id: u64,
    attacker_in_melee_range: bool,
    current_tick: u64,
) -> Option<f32> {
    // Tamed Nostriches don't retaliate against their owner — that
    // gets handled by the ownership pubkey check upstream. Here we
    // assume the attacker is hostile (or a non-owner).
    if attacker_in_melee_range {
        nostrich.state = NostrichAiState::Kick {
            target_id: attacker_id,
            until_tick: current_tick + KICK_WINDOW_TICKS as u64,
        };
        Some(KICK_DAMAGE)
    } else {
        nostrich.state = NostrichAiState::Flee {
            attacker_id,
            until_tick: current_tick + FLEE_DURATION_TICKS as u64,
        };
        None
    }
}

/// Per-tick state-machine advance. Pure: takes the current state +
/// context, returns the next state. Caller (game_loop) applies
/// the resulting state and any side effects (movement, attacks).
/// Tested below; not yet ticked from anywhere (no `dispatch_nostrich`).
#[cfg_attr(not(test), allow(dead_code))]
pub fn advance_state(
    nostrich: &NostrichData,
    current_tick: u64,
    owner_distance: Option<f32>,
) -> NostrichAiState {
    match nostrich.state {
        NostrichAiState::Flee { until_tick, .. } if current_tick >= until_tick => {
            // Flee expired — return to Idle (or Follow if tamed).
            if nostrich.is_tamed() {
                NostrichAiState::Follow
            } else {
                NostrichAiState::Idle
            }
        }
        NostrichAiState::Kick { target_id, until_tick } if current_tick >= until_tick => {
            // Kick window closed — pivot to Flee from the same target.
            NostrichAiState::Flee {
                attacker_id: target_id,
                until_tick: current_tick + FLEE_DURATION_TICKS as u64,
            }
        }
        NostrichAiState::Charge { until_tick, .. } if current_tick >= until_tick => {
            // Charge timed out — back to Flee.
            if nostrich.is_tamed() {
                NostrichAiState::Follow
            } else {
                NostrichAiState::Idle
            }
        }
        NostrichAiState::Idle if nostrich.is_tamed() => {
            // Tamed Nostriches don't stay Idle; promote to Follow.
            NostrichAiState::Follow
        }
        NostrichAiState::Follow => {
            // If we're close enough to the owner, drop to Idle-like
            // wandering (still treated as Follow for the state-machine
            // semantics — distance check decides movement). If the
            // owner is missing entirely, return to Idle.
            match owner_distance {
                None => NostrichAiState::Idle,
                Some(d) if d > FOLLOW_MAX_DISTANCE * 3.0 => {
                    // Owner too far — stop following; the Nostrich
                    // reverts to wild behaviour.
                    NostrichAiState::Idle
                }
                _ => NostrichAiState::Follow,
            }
        }
        // Default — state unchanged this tick.
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> NostrichData {
        NostrichData::untamed()
    }

    #[test]
    fn untamed_starts_in_idle_with_full_timers() {
        let n = fresh();
        assert!(!n.is_tamed());
        assert_eq!(n.state, NostrichAiState::Idle);
        assert_eq!(n.lay_ticks_remaining, LAY_PERIOD_TICKS);
        assert_eq!(n.feather_ticks_remaining, FEATHER_PERIOD_TICKS);
    }

    #[test]
    fn berry_feed_with_no_berry_is_noop() {
        let mut n = fresh();
        let out = attempt_tame_with_berry(&mut n, None, "alice", 0);
        assert_eq!(out, BerryFeedOutcome::NotAFeed);
        assert!(!n.is_tamed());
    }

    #[test]
    fn berry_feed_with_wrong_material_is_noop() {
        let mut n = fresh();
        let out = attempt_tame_with_berry(
            &mut n,
            Some(MaterialId::Wheat),
            "alice",
            0,
        );
        assert_eq!(out, BerryFeedOutcome::NotAFeed);
        assert!(!n.is_tamed());
    }

    #[test]
    fn berry_feed_eventually_tames() {
        let mut n = fresh();
        let mut feeds = 0;
        let mut tamed = false;
        for seed in 0u64..200 {
            let out = attempt_tame_with_berry(
                &mut n,
                Some(MaterialId::Berries),
                "alice",
                seed,
            );
            feeds += 1;
            if out == BerryFeedOutcome::Tamed {
                tamed = true;
                break;
            }
        }
        assert!(tamed, "expected to tame within 200 seeds; took {feeds}");
        assert!(n.is_tamed());
        assert_eq!(n.owner_pubkey(), "alice");
        assert_eq!(n.state, NostrichAiState::Follow);
    }

    #[test]
    fn berry_feed_after_tame_is_alreadytamed() {
        let mut n = fresh();
        n.ownership.owner_pubkey = "alice".to_string();
        let out = attempt_tame_with_berry(
            &mut n,
            Some(MaterialId::Berries),
            "alice",
            0,
        );
        assert_eq!(out, BerryFeedOutcome::AlreadyTamed);
    }

    #[test]
    fn toggle_sit_follow_swaps_states() {
        let mut n = fresh();
        n.ownership.owner_pubkey = "alice".to_string();
        n.state = NostrichAiState::Follow;
        toggle_sit_follow(&mut n);
        assert_eq!(n.state, NostrichAiState::Sit);
        toggle_sit_follow(&mut n);
        assert_eq!(n.state, NostrichAiState::Follow);
    }

    #[test]
    fn toggle_sit_follow_from_idle_goes_to_sit() {
        let mut n = fresh();
        n.ownership.owner_pubkey = "alice".to_string();
        n.state = NostrichAiState::Idle;
        toggle_sit_follow(&mut n);
        assert_eq!(n.state, NostrichAiState::Sit);
    }

    #[test]
    fn toggle_sit_follow_untamed_is_noop() {
        let mut n = fresh();
        n.state = NostrichAiState::Idle;
        toggle_sit_follow(&mut n);
        assert_eq!(n.state, NostrichAiState::Idle);
    }

    #[test]
    fn damage_in_melee_returns_kick_damage_and_sets_kick_state() {
        let mut n = fresh();
        let dmg = on_damaged(&mut n, 42, true, 100);
        assert_eq!(dmg, Some(KICK_DAMAGE));
        match n.state {
            NostrichAiState::Kick { target_id, until_tick } => {
                assert_eq!(target_id, 42);
                assert_eq!(until_tick, 100 + KICK_WINDOW_TICKS as u64);
            }
            other => panic!("expected Kick, got {other:?}"),
        }
    }

    #[test]
    fn damage_out_of_melee_just_flees() {
        let mut n = fresh();
        let dmg = on_damaged(&mut n, 42, false, 100);
        assert_eq!(dmg, None);
        match n.state {
            NostrichAiState::Flee { attacker_id, until_tick } => {
                assert_eq!(attacker_id, 42);
                assert_eq!(until_tick, 100 + FLEE_DURATION_TICKS as u64);
            }
            other => panic!("expected Flee, got {other:?}"),
        }
    }

    #[test]
    fn flee_expires_to_idle_when_window_closes() {
        let mut n = fresh();
        n.state = NostrichAiState::Flee {
            attacker_id: 1,
            until_tick: 50,
        };
        let next = advance_state(&n, 51, None);
        assert_eq!(next, NostrichAiState::Idle);
    }

    #[test]
    fn flee_expires_to_follow_when_tamed() {
        let mut n = fresh();
        n.ownership.owner_pubkey = "alice".to_string();
        n.state = NostrichAiState::Flee {
            attacker_id: 1,
            until_tick: 50,
        };
        let next = advance_state(&n, 51, Some(5.0));
        assert_eq!(next, NostrichAiState::Follow);
    }

    #[test]
    fn kick_expires_to_flee() {
        let mut n = fresh();
        n.state = NostrichAiState::Kick {
            target_id: 42,
            until_tick: 50,
        };
        let next = advance_state(&n, 51, None);
        match next {
            NostrichAiState::Flee { attacker_id, .. } => {
                assert_eq!(attacker_id, 42);
            }
            other => panic!("expected Flee, got {other:?}"),
        }
    }

    #[test]
    fn tamed_idle_promotes_to_follow() {
        let mut n = fresh();
        n.ownership.owner_pubkey = "alice".to_string();
        n.state = NostrichAiState::Idle;
        let next = advance_state(&n, 100, Some(5.0));
        assert_eq!(next, NostrichAiState::Follow);
    }

    #[test]
    fn follow_with_no_owner_reverts_to_idle() {
        let mut n = fresh();
        n.ownership.owner_pubkey = "alice".to_string();
        n.state = NostrichAiState::Follow;
        let next = advance_state(&n, 100, None);
        assert_eq!(next, NostrichAiState::Idle);
    }

    #[test]
    fn follow_with_far_owner_reverts_to_idle() {
        let mut n = fresh();
        n.ownership.owner_pubkey = "alice".to_string();
        n.state = NostrichAiState::Follow;
        let next = advance_state(&n, 100, Some(FOLLOW_MAX_DISTANCE * 4.0));
        assert_eq!(next, NostrichAiState::Idle);
    }

    #[test]
    fn sit_state_unchanged_by_advance() {
        let mut n = fresh();
        n.ownership.owner_pubkey = "alice".to_string();
        n.state = NostrichAiState::Sit;
        let next = advance_state(&n, 100, Some(5.0));
        assert_eq!(next, NostrichAiState::Sit);
    }
}

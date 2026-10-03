//! Spec 28d chunk 4 — Goat AI tick (pure).
//!
//! Goats are passive-with-charge: they wander idly, but at random
//! intervals (1 in ~600 ticks ≈ once per 30 s) they charge a nearby
//! target — player or mob — head-first, knocking it back. After a
//! cooldown the goat returns to wandering. This makes Mountains feel
//! lively without making goats hostile in the Spec 19 sense (they
//! don't track you, they're just startling).
//!
//! State machine:
//! - `Idle` — standing; transitions to Wander on a coin flip (same
//!   shape as horse_ai).
//! - `Wander` — walking toward a wander target.
//! - `Charge { target_id, until_tick }` — full-speed at the target;
//!   the impact tick deals 1 HP + applies knockback; afterwards a
//!   ~120-tick cooldown via `Stunned`.
//! - `Stunned { until_tick }` — standing still after a charge,
//!   recovering. Reverts to Idle.

use serde::{Deserialize, Serialize};

/// Charge wind-up + commit duration (ticks). 30 ticks = 1.5 s — enough
/// time for the player to dodge if they spot the head lower.
pub const CHARGE_DURATION_TICKS: u64 = 30;

/// Stunned/cooldown duration after a charge. 120 ticks = 6 s.
pub const STUN_DURATION_TICKS: u64 = 120;

/// Maximum distance a goat will charge from (blocks). Outside this,
/// the goat just wanders.
pub const CHARGE_TRIGGER_DISTANCE: f32 = 6.0;

/// Charge damage on impact (HP). Knockback strength is multiplied
/// by this in the impact resolver upstream.
pub const CHARGE_DAMAGE: f32 = 1.0;

/// Re-target cadence in Wander (same as horse).
pub const WANDER_RETARGET_TICKS: u64 = 80;

/// Wander step radius.
pub const WANDER_RADIUS: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoatAiState {
    Idle,
    Wander,
    Charge { target_id: u64, until_tick: u64 },
    Stunned { until_tick: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoatData {
    pub state: GoatAiState,
    pub last_retarget_tick: u64,
    pub wander_target: Option<(f32, f32, f32)>,
}

impl GoatData {
    pub fn new() -> Self {
        Self {
            state: GoatAiState::Idle,
            last_retarget_tick: 0,
            wander_target: None,
        }
    }
}

impl Default for GoatData {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GoatAction {
    /// Wander/walk toward a target.
    MoveToward { x: f32, y: f32, z: f32 },
    /// Charge a specific entity (full-speed). Caller resolves the
    /// impact + knockback when the goat reaches the target.
    Charge { target_id: u64, target_x: f32, target_y: f32, target_z: f32 },
    /// Impact happens this tick — caller applies CHARGE_DAMAGE + knockback.
    Impact { target_id: u64 },
    NoOp,
}

/// One tick of Goat AI. `nearest_target` is the nearest player or
/// passive mob within sight range — when provided, the goat may decide
/// to charge it. Pure.
pub fn tick_goat(
    goat: &GoatData,
    goat_pos: (f32, f32, f32),
    nearest_target: Option<(u64, f32, f32, f32)>,
    current_tick: u64,
) -> (GoatData, GoatAction) {
    let mut next = goat.clone();

    match goat.state {
        GoatAiState::Stunned { until_tick } => {
            if current_tick >= until_tick {
                next.state = GoatAiState::Idle;
            }
            return (next, GoatAction::NoOp);
        }
        GoatAiState::Charge { target_id, until_tick } => {
            if current_tick >= until_tick {
                // Impact resolves this tick. Game-loop applies damage +
                // knockback in response to GoatAction::Impact.
                next.state = GoatAiState::Stunned {
                    until_tick: current_tick + STUN_DURATION_TICKS,
                };
                return (next, GoatAction::Impact { target_id });
            }
            // Still winding up the charge — keep heading toward last
            // known target position via the caller.
            if let Some((id, tx, ty, tz)) = nearest_target
                && id == target_id {
                    return (next, GoatAction::Charge {
                        target_id,
                        target_x: tx, target_y: ty, target_z: tz,
                    });
                }
            // Lost sight of target → drop the charge, become stunned.
            next.state = GoatAiState::Stunned {
                until_tick: current_tick + STUN_DURATION_TICKS,
            };
            return (next, GoatAction::NoOp);
        }
        _ => {}
    }

    // Idle / Wander branches. Check charge trigger first — Goats are
    // ornery, they'll interrupt their own wander to ram something
    // close.
    if let Some((id, tx, ty, tz)) = nearest_target {
        let dx = tx - goat_pos.0;
        let dz = tz - goat_pos.2;
        let dist = (dx * dx + dz * dz).sqrt();
        if dist <= CHARGE_TRIGGER_DISTANCE
            && seeded_roll(current_tick, 7).is_multiple_of(600)
        {
            // Charge!
            next.state = GoatAiState::Charge {
                target_id: id,
                until_tick: current_tick + CHARGE_DURATION_TICKS,
            };
            return (next, GoatAction::Charge {
                target_id: id, target_x: tx, target_y: ty, target_z: tz,
            });
        }
    }

    match next.state {
        GoatAiState::Idle => {
            if seeded_roll(current_tick, 11) % 100 < 4 {
                next.state = GoatAiState::Wander;
                let (dx, dz) = pick_wander(current_tick, 13);
                next.wander_target = Some((
                    goat_pos.0 + dx,
                    goat_pos.1,
                    goat_pos.2 + dz,
                ));
                next.last_retarget_tick = current_tick;
                if let Some((x, y, z)) = next.wander_target {
                    return (next, GoatAction::MoveToward { x, y, z });
                }
            }
            (next, GoatAction::NoOp)
        }
        GoatAiState::Wander => {
            if let Some((tx, ty, tz)) = next.wander_target {
                let dx = tx - goat_pos.0;
                let dz = tz - goat_pos.2;
                if (dx * dx + dz * dz) < 0.25 {
                    next.state = GoatAiState::Idle;
                    next.wander_target = None;
                    return (next, GoatAction::NoOp);
                }
                if current_tick.saturating_sub(next.last_retarget_tick) >= WANDER_RETARGET_TICKS {
                    let (ndx, ndz) = pick_wander(current_tick, 17);
                    next.wander_target = Some((
                        goat_pos.0 + ndx,
                        goat_pos.1,
                        goat_pos.2 + ndz,
                    ));
                    next.last_retarget_tick = current_tick;
                }
                let (tx, ty, tz) = next.wander_target.unwrap_or((tx, ty, tz));
                (next, GoatAction::MoveToward { x: tx, y: ty, z: tz })
            } else {
                next.state = GoatAiState::Idle;
                (next, GoatAction::NoOp)
            }
        }
        // Stunned / Charge handled at the top.
        _ => (next, GoatAction::NoOp),
    }
}

fn seeded_roll(tick: u64, salt: u32) -> u32 {
    let mut h = (tick as u32).wrapping_mul(2_654_435_761) ^ salt.wrapping_mul(1_597_334_677);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h.wrapping_mul(0xc2b2_ae35) ^ (h >> 16)
}

fn pick_wander(tick: u64, salt: u32) -> (f32, f32) {
    let h1 = seeded_roll(tick, salt) as f32 / u32::MAX as f32;
    let h2 = seeded_roll(tick.wrapping_add(1), salt) as f32 / u32::MAX as f32;
    let theta = h1 * std::f32::consts::TAU;
    let r = h2 * WANDER_RADIUS;
    (theta.cos() * r, theta.sin() * r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goat_starts_idle() {
        let g = GoatData::new();
        assert_eq!(g.state, GoatAiState::Idle);
    }

    #[test]
    fn idle_goat_eventually_wanders_with_no_target() {
        let mut g = GoatData::new();
        let mut found = false;
        for tick in 0..5000 {
            let (next, _) = tick_goat(&g, (0.0, 64.0, 0.0), None, tick);
            g = next;
            if matches!(g.state, GoatAiState::Wander) {
                found = true;
                break;
            }
        }
        assert!(found, "goat never woke into Wander");
    }

    #[test]
    fn goat_eventually_charges_a_nearby_target() {
        // With a target within trigger distance, the per-tick charge
        // chance fires roughly once per 600 ticks. Across 6000 ticks
        // we expect ~10 charges; assert at least one.
        let mut g = GoatData::new();
        let target = Some((42u64, 3.0, 64.0, 0.0));
        let mut charges = 0;
        for tick in 0..6000 {
            let (next, action) = tick_goat(&g, (0.0, 64.0, 0.0), target, tick);
            g = next;
            if matches!(action, GoatAction::Charge { .. }) {
                charges += 1;
            }
        }
        assert!(charges > 0, "goat never charged across 6000 ticks");
    }

    #[test]
    fn distant_target_does_not_trigger_charge() {
        let g = GoatData::new();
        // Target well outside trigger distance — no charge ever.
        let far = Some((42u64, 50.0, 64.0, 0.0));
        for tick in 0..2000 {
            let (next, action) = tick_goat(&g, (0.0, 64.0, 0.0), far, tick);
            assert!(!matches!(action, GoatAction::Charge { .. }),
                "distant target triggered charge at tick {tick}");
            // Don't update g — keep the goat Idle so the trigger
            // condition is the only variable.
            let _ = next;
        }
    }

    #[test]
    fn charge_resolves_to_impact_then_stunned() {
        let g = GoatData {
            state: GoatAiState::Charge {
                target_id: 99,
                until_tick: 100,
            },
            last_retarget_tick: 0,
            wander_target: None,
        };
        let target = Some((99u64, 0.5, 64.0, 0.0));
        let (next, action) = tick_goat(&g, (0.0, 64.0, 0.0), target, 100);
        assert!(matches!(action, GoatAction::Impact { target_id: 99 }));
        assert!(matches!(next.state, GoatAiState::Stunned { .. }));
    }

    #[test]
    fn stunned_goat_does_not_act() {
        let g = GoatData {
            state: GoatAiState::Stunned { until_tick: 500 },
            last_retarget_tick: 0,
            wander_target: None,
        };
        let (_, action) = tick_goat(&g, (0.0, 64.0, 0.0), None, 200);
        assert!(matches!(action, GoatAction::NoOp));
    }

    #[test]
    fn stun_expires_to_idle() {
        let g = GoatData {
            state: GoatAiState::Stunned { until_tick: 100 },
            last_retarget_tick: 0,
            wander_target: None,
        };
        let (next, _) = tick_goat(&g, (0.0, 64.0, 0.0), None, 200);
        assert_eq!(next.state, GoatAiState::Idle);
    }
}

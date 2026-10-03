//! Spec 28d chunk 6 — Squid AI tick (pure).
//!
//! Squid are aquatic passive mobs. They live in WATER blocks, drift
//! with slow current, and emit a Suffocate signal if pushed out of
//! water for too long. Drops an InkSac on death (Material already
//! reserved by 28c Phase 5).
//!
//! States:
//! - `Drift` — slow swim toward a wander target inside water.
//! - `Suffocating { until_tick }` — out-of-water timer; when expired
//!   the squid takes 1 HP/tick (caller resolves the damage).

use serde::{Deserialize, Serialize};

/// Ticks a squid can be out of water before suffocation damage starts.
pub const SUFFOCATE_GRACE_TICKS: u64 = 60; // 3 s

/// Drift retarget cadence.
pub const DRIFT_RETARGET_TICKS: u64 = 60;

/// Per-pick drift radius (blocks).
pub const DRIFT_RADIUS: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SquidAiState {
    Drift,
    Suffocating { until_tick: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SquidData {
    pub state: SquidAiState,
    pub last_retarget_tick: u64,
    pub drift_target: Option<(f32, f32, f32)>,
}

impl SquidData {
    pub fn new() -> Self {
        Self {
            state: SquidAiState::Drift,
            last_retarget_tick: 0,
            drift_target: None,
        }
    }
}

impl Default for SquidData {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SquidAction {
    /// Drift toward an in-water target.
    DriftToward { x: f32, y: f32, z: f32 },
    /// Take 1 HP per tick of suffocation damage.
    SuffocateTick,
    NoOp,
}

/// One tick of squid AI. `in_water_at` is a closure (so we don't have
/// to depend on World here — the squid module stays pure). Returns
/// `true` if the world cell at the given block coords is a water
/// block. Pure.
pub fn tick_squid<F: Fn(i32, i32, i32) -> bool>(
    squid: &SquidData,
    squid_pos: (f32, f32, f32),
    in_water_at: F,
    current_tick: u64,
) -> (SquidData, SquidAction) {
    let mut next = squid.clone();

    let here_x = squid_pos.0.floor() as i32;
    let here_y = squid_pos.1.floor() as i32;
    let here_z = squid_pos.2.floor() as i32;
    let in_water = in_water_at(here_x, here_y, here_z);

    if !in_water {
        match squid.state {
            SquidAiState::Suffocating { until_tick } => {
                if current_tick >= until_tick {
                    return (next, SquidAction::SuffocateTick);
                }
                return (next, SquidAction::NoOp);
            }
            _ => {
                next.state = SquidAiState::Suffocating {
                    until_tick: current_tick + SUFFOCATE_GRACE_TICKS,
                };
                return (next, SquidAction::NoOp);
            }
        }
    }

    // Back in water — clear suffocation state.
    if matches!(squid.state, SquidAiState::Suffocating { .. }) {
        next.state = SquidAiState::Drift;
        next.drift_target = None;
    }

    // Drift cycle.
    if current_tick.saturating_sub(next.last_retarget_tick) >= DRIFT_RETARGET_TICKS {
        let (dx, dy, dz) = pick_drift_offset(current_tick);
        next.drift_target = Some((
            squid_pos.0 + dx,
            (squid_pos.1 + dy).max(1.0),
            squid_pos.2 + dz,
        ));
        next.last_retarget_tick = current_tick;
        if let Some((x, y, z)) = next.drift_target {
            return (next, SquidAction::DriftToward { x, y, z });
        }
    }

    if let Some((x, y, z)) = next.drift_target {
        (next, SquidAction::DriftToward { x, y, z })
    } else {
        (next, SquidAction::NoOp)
    }
}

fn pick_drift_offset(tick: u64) -> (f32, f32, f32) {
    let h = |s: u32| {
        let mut h = (tick as u32).wrapping_mul(2_654_435_761) ^ s.wrapping_mul(1_597_334_677);
        h ^= h >> 16;
        h = h.wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
        (h.wrapping_mul(0xc2b2_ae35) ^ (h >> 16)) as f32 / u32::MAX as f32
    };
    let theta = h(1) * std::f32::consts::TAU;
    let r = h(2) * DRIFT_RADIUS;
    let dy = -1.0 + h(3) * 2.0; // ±1b vertical drift
    (theta.cos() * r, dy, theta.sin() * r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squid_starts_in_drift() {
        let s = SquidData::new();
        assert_eq!(s.state, SquidAiState::Drift);
    }

    #[test]
    fn squid_in_water_drifts() {
        let s = SquidData::new();
        let (next, action) = tick_squid(&s, (0.0, 64.0, 0.0), |_, _, _| true, 60);
        assert_eq!(next.state, SquidAiState::Drift);
        assert!(matches!(action, SquidAction::DriftToward { .. }));
    }

    #[test]
    fn squid_out_of_water_enters_suffocation_grace() {
        let s = SquidData::new();
        let (next, _) = tick_squid(&s, (0.0, 64.0, 0.0), |_, _, _| false, 10);
        assert!(matches!(next.state, SquidAiState::Suffocating { .. }));
    }

    #[test]
    fn squid_takes_damage_after_grace_expires() {
        let s = SquidData {
            state: SquidAiState::Suffocating { until_tick: 100 },
            last_retarget_tick: 0,
            drift_target: None,
        };
        let (_, action) = tick_squid(&s, (0.0, 64.0, 0.0), |_, _, _| false, 200);
        assert!(matches!(action, SquidAction::SuffocateTick));
    }

    #[test]
    fn squid_back_in_water_clears_suffocation() {
        let s = SquidData {
            state: SquidAiState::Suffocating { until_tick: 100 },
            last_retarget_tick: 0,
            drift_target: None,
        };
        let (next, _) = tick_squid(&s, (0.0, 64.0, 0.0), |_, _, _| true, 200);
        assert!(matches!(next.state, SquidAiState::Drift));
    }

    #[test]
    fn squid_grace_period_holds_off_damage() {
        // Just stepped out of water. Action should be NoOp until the
        // grace tick fires.
        let s = SquidData::new();
        let (next, action) = tick_squid(&s, (0.0, 64.0, 0.0), |_, _, _| false, 0);
        assert!(matches!(next.state, SquidAiState::Suffocating { .. }));
        assert!(matches!(action, SquidAction::NoOp));
    }
}

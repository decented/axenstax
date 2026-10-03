//! Spec 28d chunk 5 — Bee AI tick (pure).
//!
//! Bees are flying, neutral mobs. They wander around their home hive
//! (when one exists — Hive block-entity ships in chunk 8) and pollinate
//! crops. Stung-by-player turns them aggressive for a bounded window;
//! the sting drops the bee (consumes a stinger) so they can't grief
//! forever.
//!
//! Flight is distinct from ground-mob wander: target Y oscillates
//! between `hover_min_y` and `hover_max_y` so the bee bobs through
//! the air. The game_loop applies gravity-cancel + the AI's
//! per-axis vector.
//!
//! States:
//! - `Idle` → occasional Fly transition.
//! - `Fly` → drift toward a random nearby point (xz random; y bobs).
//! - `ReturnToHive` → if a `home_hive` is set + the bee is > 20b away
//!   from it, fly back. Triggered by night-fall + distance.
//! - `Sting { target_id, until_tick }` → angry-bee window. Caller
//!   resolves an attack on impact + then removes the bee (one-shot).

use serde::{Deserialize, Serialize};

pub const HIVE_RETURN_DISTANCE: f32 = 20.0;
pub const STING_WINDOW_TICKS: u64 = 60; // 3 s
pub const STING_TRIGGER_DISTANCE: f32 = 4.0;

/// Random-fly cadence (re-target every N ticks).
pub const FLY_RETARGET_TICKS: u64 = 40;

/// Flying mob bobs vertically between these heights relative to current y.
pub const HOVER_MIN_DY: f32 = -1.0;
pub const HOVER_MAX_DY: f32 = 1.5;

/// Horizontal wander radius per pick.
pub const FLY_RADIUS: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BeeAiState {
    Idle,
    Fly,
    ReturnToHive,
    Sting { target_id: u64, until_tick: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeeData {
    pub state: BeeAiState,
    /// Block position of the bee's home hive (set when the bee was
    /// spawned by a Hive). `None` for wild bees.
    pub home_hive: Option<(i32, i32, i32)>,
    /// Last tick the fly target was picked.
    pub last_retarget_tick: u64,
    pub fly_target: Option<(f32, f32, f32)>,
}

impl BeeData {
    pub fn new() -> Self {
        Self {
            state: BeeAiState::Idle,
            home_hive: None,
            last_retarget_tick: 0,
            fly_target: None,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_home_hive(hive: (i32, i32, i32)) -> Self {
        Self {
            home_hive: Some(hive),
            ..Self::new()
        }
    }
}

impl Default for BeeData {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BeeAction {
    /// Drift toward this point.
    FlyToward { x: f32, y: f32, z: f32 },
    /// Charge to sting this entity.
    StingChase { target_id: u64, target_x: f32, target_y: f32, target_z: f32 },
    /// Impact this tick — caller applies sting damage + then despawns
    /// the bee (Minecraft parity: bees die after stinging).
    StingImpact { target_id: u64 },
    NoOp,
}

/// One tick of bee AI. `recent_attacker` is the most recent attacker on
/// this bee within the sting trigger window — caller decides what's
/// "recent". Pure.
pub fn tick_bee(
    bee: &BeeData,
    bee_pos: (f32, f32, f32),
    recent_attacker: Option<(u64, f32, f32, f32)>,
    current_tick: u64,
) -> (BeeData, BeeAction) {
    let mut next = bee.clone();

    // Sting state resolves first.
    if let BeeAiState::Sting { target_id, until_tick } = bee.state {
        if current_tick >= until_tick {
            // Window expired; impact this tick + caller despawns.
            return (next, BeeAction::StingImpact { target_id });
        }
        if let Some((aid, ax, ay, az)) = recent_attacker
            && aid == target_id {
                return (next, BeeAction::StingChase {
                    target_id, target_x: ax, target_y: ay, target_z: az,
                });
            }
        // Lost sight — keep heading toward last known.
        return (next, BeeAction::NoOp);
    }

    // Recent attacker close → enter Sting.
    if let Some((aid, ax, ay, az)) = recent_attacker {
        let dx = ax - bee_pos.0;
        let dz = az - bee_pos.2;
        let dist = (dx * dx + dz * dz).sqrt();
        if dist <= STING_TRIGGER_DISTANCE {
            next.state = BeeAiState::Sting {
                target_id: aid,
                until_tick: current_tick + STING_WINDOW_TICKS,
            };
            return (next, BeeAction::StingChase {
                target_id: aid, target_x: ax, target_y: ay, target_z: az,
            });
        }
    }

    // Home-hive return triggers if we have one and we're far away.
    if let Some((hx, hy, hz)) = bee.home_hive {
        let dx = hx as f32 - bee_pos.0;
        let dz = hz as f32 - bee_pos.2;
        let dist = (dx * dx + dz * dz).sqrt();
        if dist > HIVE_RETURN_DISTANCE {
            next.state = BeeAiState::ReturnToHive;
            return (next, BeeAction::FlyToward {
                x: hx as f32 + 0.5, y: hy as f32 + 0.5, z: hz as f32 + 0.5,
            });
        }
    }

    // Normal fly cycle.
    if current_tick.saturating_sub(next.last_retarget_tick) >= FLY_RETARGET_TICKS {
        let (dx, dy, dz) = pick_fly_offset(current_tick);
        next.state = BeeAiState::Fly;
        next.fly_target = Some((
            bee_pos.0 + dx,
            (bee_pos.1 + dy).max(1.0),
            bee_pos.2 + dz,
        ));
        next.last_retarget_tick = current_tick;
        if let Some((x, y, z)) = next.fly_target {
            return (next, BeeAction::FlyToward { x, y, z });
        }
    }

    next.state = BeeAiState::Idle;
    (next, BeeAction::NoOp)
}

fn pick_fly_offset(tick: u64) -> (f32, f32, f32) {
    let h = |s: u32| {
        let mut h = (tick as u32).wrapping_mul(2_654_435_761) ^ s.wrapping_mul(1_597_334_677);
        h ^= h >> 16;
        h = h.wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
        (h.wrapping_mul(0xc2b2_ae35) ^ (h >> 16)) as f32 / u32::MAX as f32
    };
    let theta = h(1) * std::f32::consts::TAU;
    let r = h(2) * FLY_RADIUS;
    let dy = HOVER_MIN_DY + (HOVER_MAX_DY - HOVER_MIN_DY) * h(3);
    (theta.cos() * r, dy, theta.sin() * r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bee_starts_idle_without_hive() {
        let b = BeeData::new();
        assert_eq!(b.state, BeeAiState::Idle);
        assert!(b.home_hive.is_none());
    }

    #[test]
    fn bee_can_be_assigned_a_home_hive() {
        let b = BeeData::with_home_hive((10, 64, 5));
        assert_eq!(b.home_hive, Some((10, 64, 5)));
    }

    #[test]
    fn close_attacker_triggers_sting() {
        let b = BeeData::new();
        let attacker = Some((42u64, 2.0, 64.0, 0.0));
        let (next, action) = tick_bee(&b, (0.0, 64.0, 0.0), attacker, 10);
        assert!(matches!(next.state, BeeAiState::Sting { target_id: 42, .. }));
        assert!(matches!(action, BeeAction::StingChase { target_id: 42, .. }));
    }

    #[test]
    fn distant_attacker_does_not_trigger_sting() {
        let b = BeeData::new();
        let far = Some((42u64, 100.0, 64.0, 0.0));
        let (next, action) = tick_bee(&b, (0.0, 64.0, 0.0), far, 10);
        assert!(!matches!(next.state, BeeAiState::Sting { .. }));
        assert!(!matches!(action, BeeAction::StingChase { .. }));
    }

    #[test]
    fn sting_window_resolves_to_impact() {
        let b = BeeData {
            state: BeeAiState::Sting { target_id: 7, until_tick: 100 },
            home_hive: None,
            last_retarget_tick: 0,
            fly_target: None,
        };
        let (_, action) = tick_bee(&b, (0.0, 64.0, 0.0), None, 200);
        assert!(matches!(action, BeeAction::StingImpact { target_id: 7 }));
    }

    #[test]
    fn bee_returns_to_distant_hive() {
        let b = BeeData::with_home_hive((50, 64, 0));
        // Bee currently at origin → 50b from hive → > HIVE_RETURN_DISTANCE.
        let (next, action) = tick_bee(&b, (0.0, 64.0, 0.0), None, 10);
        assert_eq!(next.state, BeeAiState::ReturnToHive);
        assert!(matches!(action, BeeAction::FlyToward { x, .. } if x > 49.0));
    }

    #[test]
    fn bee_with_close_hive_does_normal_fly() {
        let b = BeeData::with_home_hive((2, 64, 2));
        // Within return distance → stays in fly cycle.
        for tick in 0..200 {
            let (next, _) = tick_bee(&b, (0.0, 64.0, 0.0), None, tick);
            assert!(!matches!(next.state, BeeAiState::ReturnToHive),
                "close-to-hive bee should not enter ReturnToHive (tick {tick})");
        }
    }

    #[test]
    fn fly_target_has_vertical_bob() {
        // Pick across many ticks; dy should range across HOVER_MIN/MAX.
        let mut min_dy = f32::MAX;
        let mut max_dy = f32::MIN;
        for tick in 0..1000 {
            let (_, dy, _) = pick_fly_offset(tick);
            min_dy = min_dy.min(dy);
            max_dy = max_dy.max(dy);
        }
        assert!(min_dy < 0.0, "fly path should sometimes dip below y; min={min_dy}");
        assert!(max_dy > 0.5, "fly path should sometimes rise; max={max_dy}");
    }
}

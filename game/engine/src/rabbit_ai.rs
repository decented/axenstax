//! Spec 28d chunk 3 — Rabbit AI tick (pure).
//!
//! Rabbits are small passive creatures with hop-style movement. AI
//! states:
//! - `Idle`: standing still, occasional ear-twitch (animation hook later).
//! - `Hop`: short jump in a random direction every ~30 ticks.
//! - `Flee`: triggered by player-proximity (within 4 b) — extra-fast hop
//!   in the away direction until the threat clears.
//!
//! Rabbits don't engage in combat (3 HP is one wolf bite). Live spawn
//! deferred; module ships here so ECS wire-up is a single import away.

use serde::{Deserialize, Serialize};

/// Minimum ticks between hops (Hop state).
pub const HOP_PERIOD_TICKS: u64 = 30;

/// Player proximity that triggers Flee (blocks).
pub const FLEE_DISTANCE_BLOCKS: f32 = 4.0;

/// Distance moved per hop (blocks). Faster than wolf wander.
pub const HOP_DISTANCE_BLOCKS: f32 = 1.5;

/// Rabbits flee until the player is this far away (blocks). Lower than
/// the trigger distance so we don't oscillate at the boundary.
pub const FLEE_CLEAR_DISTANCE_BLOCKS: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RabbitAiState {
    Idle,
    Hop,
    Flee,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RabbitData {
    pub state: RabbitAiState,
    pub last_hop_tick: u64,
}

impl RabbitData {
    pub fn new() -> Self {
        Self { state: RabbitAiState::Idle, last_hop_tick: 0 }
    }
}

impl Default for RabbitData {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RabbitAction {
    /// Hop toward this offset (relative to current pos).
    HopToward { dx: f32, dz: f32 },
    /// Flee from this player position.
    FleeFrom { x: f32, z: f32 },
    NoOp,
}

/// One tick of Rabbit AI. Pure. `nearest_player_dist` is `None` if no
/// player is nearby — in that case the rabbit stays in its passive
/// hop/idle loop.
pub fn tick_rabbit(
    rabbit: &RabbitData,
    rabbit_pos: (f32, f32, f32),
    nearest_player: Option<(f32, f32, f32)>,
    current_tick: u64,
) -> (RabbitData, RabbitAction) {
    let mut next = *rabbit;

    // Compute player distance once.
    let player_dist = nearest_player.map(|(px, _py, pz)| {
        let dx = px - rabbit_pos.0;
        let dz = pz - rabbit_pos.2;
        (dx * dx + dz * dz).sqrt()
    });

    // Player too close → enter / stay in Flee.
    if let Some(d) = player_dist
        && d < FLEE_DISTANCE_BLOCKS {
            next.state = RabbitAiState::Flee;
            if let Some((px, _, pz)) = nearest_player {
                next.last_hop_tick = current_tick;
                return (next, RabbitAction::FleeFrom { x: px, z: pz });
            }
        }

    // In Flee: stay there until the player clears.
    if matches!(rabbit.state, RabbitAiState::Flee) {
        match player_dist {
            Some(d) if d < FLEE_CLEAR_DISTANCE_BLOCKS => {
                if let Some((px, _, pz)) = nearest_player {
                    return (next, RabbitAction::FleeFrom { x: px, z: pz });
                }
            }
            _ => {
                // Player gone / out of range → revert to Idle.
                next.state = RabbitAiState::Idle;
            }
        }
    }

    // Passive hop cycle.
    if current_tick.saturating_sub(next.last_hop_tick) >= HOP_PERIOD_TICKS {
        let (dx, dz) = pick_hop_offset(current_tick);
        next.state = RabbitAiState::Hop;
        next.last_hop_tick = current_tick;
        return (next, RabbitAction::HopToward { dx, dz });
    }

    // In-between hops: idle.
    next.state = RabbitAiState::Idle;
    (next, RabbitAction::NoOp)
}

fn pick_hop_offset(tick: u64) -> (f32, f32) {
    // Same hash function as horse_ai for replay-stable per-tick
    // determinism.
    let mut h = (tick as u32).wrapping_mul(2_246_822_519) ^ 0x9E37_79B9;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    let unit = (h.wrapping_mul(0xc2b2_ae35) ^ (h >> 16)) as f32 / u32::MAX as f32;
    let theta = unit * std::f32::consts::TAU;
    (theta.cos() * HOP_DISTANCE_BLOCKS, theta.sin() * HOP_DISTANCE_BLOCKS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rabbit_starts_idle() {
        let r = RabbitData::new();
        assert_eq!(r.state, RabbitAiState::Idle);
        assert_eq!(r.last_hop_tick, 0);
    }

    #[test]
    fn player_within_flee_distance_triggers_flee() {
        let r = RabbitData::new();
        // Player at (3, 64, 0), rabbit at origin — distance 3 (< 4)
        let nearby = Some((3.0, 64.0, 0.0));
        let (next, action) = tick_rabbit(&r, (0.0, 64.0, 0.0), nearby, 10);
        assert_eq!(next.state, RabbitAiState::Flee);
        assert!(matches!(action, RabbitAction::FleeFrom { x: 3.0, z: 0.0 }));
    }

    #[test]
    fn player_far_away_leaves_rabbit_passive() {
        let r = RabbitData::new();
        let far_player = Some((100.0, 64.0, 100.0));
        let (next, action) = tick_rabbit(&r, (0.0, 64.0, 0.0), far_player, 5);
        // First tick, no hop period elapsed → idle/no-op.
        assert!(matches!(action, RabbitAction::NoOp));
        assert_eq!(next.state, RabbitAiState::Idle);
    }

    #[test]
    fn flee_clears_when_player_moves_away() {
        let r = RabbitData { state: RabbitAiState::Flee, last_hop_tick: 0 };
        // Player now at distance 20 (> FLEE_CLEAR_DISTANCE)
        let far_player = Some((20.0, 64.0, 0.0));
        let (next, _) = tick_rabbit(&r, (0.0, 64.0, 0.0), far_player, 50);
        // Should revert from Flee and continue with the hop cycle (50
        // ticks > HOP_PERIOD_TICKS so a hop fires).
        assert!(matches!(next.state, RabbitAiState::Idle | RabbitAiState::Hop));
    }

    #[test]
    fn passive_rabbit_hops_periodically() {
        let mut r = RabbitData::new();
        let mut hops = 0;
        for tick in 0..200 {
            let (next, action) = tick_rabbit(&r, (0.0, 64.0, 0.0), None, tick);
            r = next;
            if matches!(action, RabbitAction::HopToward { .. }) {
                hops += 1;
            }
        }
        // 200 ticks / 30 per hop = 6+ hops expected.
        assert!(hops >= 6, "expected at least 6 hops across 200 ticks, got {hops}");
    }

    #[test]
    fn flee_at_boundary_still_flees_inside() {
        // Player exactly at FLEE_DISTANCE − 0.1 → inside.
        let r = RabbitData::new();
        let inside = Some((FLEE_DISTANCE_BLOCKS - 0.1, 64.0, 0.0));
        let (next, _) = tick_rabbit(&r, (0.0, 64.0, 0.0), inside, 0);
        assert_eq!(next.state, RabbitAiState::Flee);
    }
}

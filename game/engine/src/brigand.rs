//! Historical Pivot Sub-Foundation 3 (HP-3) — Brigand tier framework.
//!
//! Three human-tier mob species (`Brigand`, `Marauder`, `Berserker`) share
//! the base `tick_mob_ai` dispatcher; tier-specific behaviour layers in
//! via this module's pre-pass `tick_brigand_overrides`.
//!
//! Per-tier knobs:
//! - **Detect range** — replaces `mob_ai::DETECT_RANGE` for tier mobs
//!   (16 / 24 / 32 blocks).
//! - **Flee gate** — Brigand-only; below 25 % HP they turn toward their
//!   home hideout anchor and walk away (Wander state, facing reset).
//! - **Day/night chase gating** (HP-polish, 2026-05-23) — Brigand +
//!   Marauder only chase at night (sun brightness < 0.3); Berserker
//!   always chases (day or night). `world_time` threads through the
//!   override pass; `is_night_at(world_time)` is the gate.
//!
//! Engine-generic shape: the `Tier` enum + the override pass are
//! plain-data and lift cross-game wherever "humans-with-stratified-AI"
//! content sits in a survival-style game.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::combat::Health;
use crate::entity::{MobKind, Position};
use crate::mob::MobType;
use crate::mob_ai::{AiState, MobAi};

/// Tier of a brigand-family mob. ECS component (one per brigand-family
/// entity). Inserted by the hideout spawner at spawn time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    Brigand,
    Marauder,
    Berserker,
}

impl Tier {
    /// Map a `MobType` to a `Tier`, or `None` if the mob isn't part of
    /// the brigand family. Pure.
    pub fn from_mob_type(kind: MobType) -> Option<Self> {
        match kind {
            MobType::Brigand => Some(Tier::Brigand),
            MobType::Marauder => Some(Tier::Marauder),
            MobType::Berserker => Some(Tier::Berserker),
            _ => None,
        }
    }

    /// Detect range in blocks. Overrides `mob_ai::DETECT_RANGE` for
    /// brigand-family mobs.
    pub fn detect_range(self) -> f32 {
        match self {
            Tier::Brigand => 16.0,
            Tier::Marauder => 24.0,
            Tier::Berserker => 32.0,
        }
    }

    /// HP ratio below which a Brigand flees toward home. None means the
    /// tier never flees.
    pub fn flee_threshold(self) -> Option<f32> {
        match self {
            Tier::Brigand => Some(0.25),
            Tier::Marauder => None,
            Tier::Berserker => None,
        }
    }

    /// HP-polish 2026-05-23 — does this tier chase players regardless of
    /// time of day? Berserker is always aggressive; Brigand + Marauder
    /// gate on night-time per the historical-pivot vision doc.
    pub fn always_aggressive(self) -> bool {
        matches!(self, Tier::Berserker)
    }
}

/// Sun-brightness threshold below which the world counts as "night" for
/// brigand chase-gating. Matches `spawning::tick_mob_spawning`'s
/// `is_night` test so the surface-hostile spawn pool and the brigand
/// AI agree on when night begins.
pub const NIGHT_BRIGHTNESS_THRESHOLD: f32 = 0.3;

/// Pure helper — is the world currently in its night-time band?
/// Reads `camera::compute_sun` for brightness; matches the spawn-time
/// `is_night` semantics so newly-spawned brigands stay consistent
/// with the AI override pass.
pub fn is_night_at(world_time: u32) -> bool {
    let (_, brightness) = crate::camera::compute_sun(world_time);
    brightness < NIGHT_BRIGHTNESS_THRESHOLD
}

/// ECS component carrying the tier for a brigand-family mob. Inserted on
/// spawn (brigand.rs, brigand_hideout_gen.rs) but never queried back
/// anywhere — the "tier-tunable AI hooks" this is meant to drive (per
/// mob.rs's comment) aren't reading it yet.
#[derive(Clone, Copy, Debug)]
pub struct BrigandTier {
    #[allow(dead_code)]
    pub tier: Tier,
}

/// ECS component recording the home hideout anchor for a brigand mob.
/// Used by the flee path (Brigand walks back toward this position when
/// wounded) and the kill-attribution path (population decrement on
/// death keyed by this anchor).
#[derive(Clone, Copy, Debug)]
pub struct HomeHideout {
    pub anchor: [i32; 3],
}

/// Pure helper: is this entity's health below the tier's flee
/// threshold? Returns false for tiers that never flee (Marauder /
/// Berserker) and for full-HP brigands.
pub fn should_flee(tier: Tier, health: &Health) -> bool {
    let Some(threshold) = tier.flee_threshold() else {
        return false;
    };
    if health.max <= 0.0 {
        return false;
    }
    health.current / health.max < threshold
}

/// Nearest player position to a mob — local copy of the mob_ai helper
/// (kept private there). Returns None if no players.
fn nearest_player(mob_pos: Vec3, player_positions: &[Vec3]) -> Option<(Vec3, f32)> {
    if player_positions.is_empty() {
        return None;
    }
    let mut best_pos = player_positions[0];
    let mut best_dist = (best_pos - mob_pos).length();
    for &pp in &player_positions[1..] {
        let d = (pp - mob_pos).length();
        if d < best_dist {
            best_dist = d;
            best_pos = pp;
        }
    }
    Some((best_pos, best_dist))
}

/// Pre-pass that runs once per tick BEFORE `tick_mob_ai`. Mutates the
/// `MobAi` state on brigand-family mobs to apply tier-specific
/// behaviour (extended detect range + Brigand flee gate + day/night
/// chase gate). The main dispatcher then runs its normal Chase /
/// Wander / Idle logic with the updated state.
///
/// Called from the hosted_server tick + the single-player game_loop
/// tick — the same dual-call shape used by `tick_mob_spawning` and
/// `tick_knight_spawn`. `world_time` drives the day/night gate;
/// Brigand + Marauder skip the chase-promotion arm during the day,
/// Berserker ignores time of day entirely.
pub fn tick_brigand_overrides(
    ecs: &mut hecs::World,
    player_positions: &[Vec3],
    world_time: u32,
) {
    if player_positions.is_empty() {
        return;
    }
    let is_night = is_night_at(world_time);

    // Collect transitions first to avoid borrow conflicts on the ECS.
    let mut transitions: Vec<(hecs::Entity, AiState, f32)> = Vec::new();

    for (id, (kind, pos, ai, health)) in ecs
        .query::<(&MobKind, &Position, &MobAi, &Health)>()
        .iter()
    {
        let Some(tier) = Tier::from_mob_type(kind.0) else {
            continue;
        };
        let Some((player_pos, player_dist)) = nearest_player(pos.0, player_positions) else {
            continue;
        };

        // Flee gate (Brigand only). Only fires when the mob has a
        // HomeHideout anchor — wave-spawn brigands (HP-5) have no
        // home and fall through to the regular detect-range override
        // so a wounded wave-Brigand keeps chasing rather than stalling.
        if should_flee(tier, health)
            && let Ok(home) = ecs.get::<&HomeHideout>(id) {
                let to_home = Vec3::new(
                    home.anchor[0] as f32 + 0.5,
                    pos.0.y,
                    home.anchor[2] as f32 + 0.5,
                ) - pos.0;
                let facing = to_home.z.atan2(to_home.x);
                // Set wander timer for 80 ticks (4 s) so the brigand
                // walks home for a noticeable beat before re-checking.
                transitions.push((id, AiState::Wander { timer: 80 }, facing));
                continue;
            }
            // No home — fall through to the normal chase override.

        // Day/night gate — Brigand + Marauder only chase at night.
        // Berserker bypasses the gate (always aggressive). HP-polish
        // 2026-05-23: vision-doc-spec'd as deferred v2 in HP-3, landed
        // here as the post-cutover polish round.
        if !is_night && !tier.always_aggressive() {
            continue;
        }

        // Detect-range override — bump Idle/Wander mobs into Chase if
        // the player is within the tier's range. `tick_mob_ai`'s
        // Chase arm then drives the actual pursuit.
        let in_range = player_dist < tier.detect_range();
        let in_idle_or_wander = matches!(ai.state, AiState::Idle { .. } | AiState::Wander { .. });
        if in_range && in_idle_or_wander {
            // Facing toward player so the immediate Chase tick lines up.
            let to_player = player_pos - pos.0;
            let facing = to_player.z.atan2(to_player.x);
            transitions.push((id, AiState::Chase, facing));
        }
    }

    for (id, new_state, new_facing) in transitions {
        if let Ok(mut ai) = ecs.get::<&mut MobAi>(id) {
            ai.state = new_state;
            // Only update facing when the override carried one (chase
            // toward player / flee toward home both set it). Default 0
            // is fine for the no-home fallback; that branch sits in
            // Idle so facing is irrelevant.
            ai.facing = new_facing;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::Health;
    use crate::entity::spawn_mob;
    use crate::mob_ai::MobAi;

    /// Night world_time used by every pre-existing override test —
    /// HP-polish (2026-05-23) introduced day/night gating; tests that
    /// pre-date it stay on the night-time branch so their semantics
    /// don't change.
    const NIGHT_TICK: u32 = 0;
    /// Noon world_time. `compute_sun(12000)` is peak brightness; the
    /// brigand chase gate skips Brigand + Marauder promotion here.
    const DAY_TICK: u32 = 12_000;

    fn make_health(current: f32, max: f32) -> Health {
        Health { current, max, flash_timer: 0, invincible_timer: 0 }
    }

    #[test]
    fn tier_from_mob_type_resolves_brigand_family() {
        assert_eq!(Tier::from_mob_type(MobType::Brigand), Some(Tier::Brigand));
        assert_eq!(Tier::from_mob_type(MobType::Marauder), Some(Tier::Marauder));
        assert_eq!(Tier::from_mob_type(MobType::Berserker), Some(Tier::Berserker));
        assert_eq!(Tier::from_mob_type(MobType::Cow), None);
        assert_eq!(Tier::from_mob_type(MobType::Bear), None);
    }

    #[test]
    fn detect_range_scales_with_tier() {
        assert!(Tier::Brigand.detect_range() < Tier::Marauder.detect_range());
        assert!(Tier::Marauder.detect_range() < Tier::Berserker.detect_range());
        assert_eq!(Tier::Brigand.detect_range(), 16.0);
        assert_eq!(Tier::Marauder.detect_range(), 24.0);
        assert_eq!(Tier::Berserker.detect_range(), 32.0);
    }

    #[test]
    fn flee_threshold_is_brigand_only() {
        assert!(Tier::Brigand.flee_threshold().is_some());
        assert!(Tier::Marauder.flee_threshold().is_none());
        assert!(Tier::Berserker.flee_threshold().is_none());
    }

    #[test]
    fn brigand_flees_below_quarter_hp() {
        let h = make_health(3.0, 16.0); // 0.1875 < 0.25
        assert!(should_flee(Tier::Brigand, &h));
    }

    #[test]
    fn brigand_does_not_flee_at_full_hp() {
        let h = make_health(16.0, 16.0);
        assert!(!should_flee(Tier::Brigand, &h));
    }

    #[test]
    fn marauder_does_not_flee_at_low_hp() {
        let h = make_health(1.0, 28.0); // 0.036 < 0.25
        assert!(!should_flee(Tier::Marauder, &h));
    }

    #[test]
    fn berserker_does_not_flee_at_low_hp() {
        let h = make_health(1.0, 45.0);
        assert!(!should_flee(Tier::Berserker, &h));
    }

    /// Spawn a single brigand mob with the given tier + home + health,
    /// at the given position. Returns its entity id.
    fn spawn_test_brigand(
        ecs: &mut hecs::World,
        tier: Tier,
        home: [i32; 3],
        pos: Vec3,
        health_current: f32,
    ) -> hecs::Entity {
        let kind = match tier {
            Tier::Brigand => MobType::Brigand,
            Tier::Marauder => MobType::Marauder,
            Tier::Berserker => MobType::Berserker,
        };
        let id = spawn_mob(ecs, kind, pos);
        let max = crate::mob::mob_def(kind).health as f32;
        ecs.insert(id, (
            Health { current: health_current, max, flash_timer: 0, invincible_timer: 0 },
            BrigandTier { tier },
            HomeHideout { anchor: home },
        )).unwrap();
        id
    }

    #[test]
    fn override_pass_promotes_marauder_to_chase_at_20_blocks() {
        // Plain DETECT_RANGE (16) wouldn't trigger at 20 blocks, but
        // Marauder's tier range (24) does.
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Marauder,
            [0, 64, 0],
            Vec3::new(0.0, 64.0, 0.0),
            28.0,
        );
        // Player at distance 20.
        tick_brigand_overrides(&mut ecs, &[Vec3::new(20.0, 64.0, 0.0)], NIGHT_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(matches!(ai.state, AiState::Chase),
            "Marauder should chase at 20 blocks (tier detect 24)");
    }

    #[test]
    fn override_pass_leaves_brigand_idle_at_20_blocks() {
        // Brigand's tier range is 16; 20 blocks is out of reach.
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Brigand,
            [0, 64, 0],
            Vec3::new(0.0, 64.0, 0.0),
            16.0,
        );
        tick_brigand_overrides(&mut ecs, &[Vec3::new(20.0, 64.0, 0.0)], NIGHT_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(matches!(ai.state, AiState::Idle { .. } | AiState::Wander { .. }),
            "Brigand should stay non-chasing at 20 blocks (tier detect 16)");
    }

    #[test]
    fn override_pass_promotes_berserker_to_chase_at_30_blocks() {
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Berserker,
            [0, 64, 0],
            Vec3::new(0.0, 64.0, 0.0),
            45.0,
        );
        tick_brigand_overrides(&mut ecs, &[Vec3::new(30.0, 64.0, 0.0)], NIGHT_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(matches!(ai.state, AiState::Chase),
            "Berserker should chase at 30 blocks (tier detect 32)");
    }

    #[test]
    fn override_pass_flees_wounded_brigand_toward_home() {
        // Brigand at (10, 64, 0) with home at (0, 64, 0) and HP 2/16
        // — should flip to Wander with facing toward home.
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Brigand,
            [0, 64, 0],
            Vec3::new(10.0, 64.0, 0.0),
            2.0,
        );
        // Player nearby to ensure the flee path overrides chase.
        tick_brigand_overrides(&mut ecs, &[Vec3::new(11.0, 64.0, 0.0)], NIGHT_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(matches!(ai.state, AiState::Wander { .. }), "wounded Brigand should flee (Wander)");
        // Facing: home is at -x from the mob → atan2(0, -10) = π.
        // Just check that facing points away from positive-x.
        assert!(ai.facing.cos() < 0.0,
            "Brigand facing should point toward home (negative-x), got cos = {}",
            ai.facing.cos());
    }

    #[test]
    fn override_pass_does_not_flee_marauder_at_low_hp() {
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Marauder,
            [0, 64, 0],
            Vec3::new(10.0, 64.0, 0.0),
            2.0,
        );
        tick_brigand_overrides(&mut ecs, &[Vec3::new(11.0, 64.0, 0.0)], NIGHT_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        // Marauder should be promoted to Chase, NOT flee.
        assert!(matches!(ai.state, AiState::Chase), "wounded Marauder should still chase");
    }

    #[test]
    fn override_pass_low_hp_brigand_without_home_still_chases() {
        // HP-5 wave-spawn case: a Brigand spawned by raid mechanics
        // has no HomeHideout. Even at low HP it should keep chasing
        // (not stall in Idle), because there's no home to flee to.
        let mut ecs = hecs::World::new();
        let kind = MobType::Brigand;
        let id = spawn_mob(&mut ecs, kind, Vec3::new(10.0, 64.0, 0.0));
        let max = crate::mob::mob_def(kind).health as f32;
        // Insert Health (low) + BrigandTier — NO HomeHideout.
        ecs.insert(id, (
            Health { current: 2.0, max, flash_timer: 0, invincible_timer: 0 },
            BrigandTier { tier: Tier::Brigand },
        )).unwrap();
        // Player within 10 blocks → Brigand's detect range (16) covers.
        tick_brigand_overrides(&mut ecs, &[Vec3::new(11.0, 64.0, 0.0)], NIGHT_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        // No HomeHideout → flee gate falls through; detect-range
        // override fires → Chase.
        assert!(matches!(ai.state, AiState::Chase),
            "wave-spawn Brigand without HomeHideout should chase even at low HP");
    }

    // HP-polish 2026-05-23 — day/night chase gating tests.

    #[test]
    fn is_night_at_zero_world_time() {
        // world_time = 0 is midnight by `compute_sun`'s phase.
        assert!(is_night_at(0));
    }

    #[test]
    fn is_night_at_noon_returns_false() {
        // world_time = 12000 is noon; brightness peaks at 1.0.
        assert!(!is_night_at(12_000));
    }

    #[test]
    fn brigand_does_not_chase_during_day() {
        // Player in detect range; Brigand should NOT promote to Chase
        // when world_time is daytime.
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Brigand,
            [0, 64, 0],
            Vec3::new(0.0, 64.0, 0.0),
            16.0,
        );
        // Player 8 blocks away — well within Brigand's 16-block range.
        tick_brigand_overrides(&mut ecs, &[Vec3::new(8.0, 64.0, 0.0)], DAY_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(matches!(ai.state, AiState::Idle { .. } | AiState::Wander { .. }),
            "Brigand should NOT chase during day, got {:?}", ai.state);
    }

    #[test]
    fn marauder_does_not_chase_during_day() {
        // Marauder shares the night-only gate with Brigand. Vision
        // doc explicitly names Marauder as "Night only" too.
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Marauder,
            [0, 64, 0],
            Vec3::new(0.0, 64.0, 0.0),
            28.0,
        );
        tick_brigand_overrides(&mut ecs, &[Vec3::new(15.0, 64.0, 0.0)], DAY_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(matches!(ai.state, AiState::Idle { .. } | AiState::Wander { .. }),
            "Marauder should NOT chase during day, got {:?}", ai.state);
    }

    #[test]
    fn berserker_chases_during_day_anyway() {
        // Berserker is the "always aggressive" tier per spec table.
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Berserker,
            [0, 64, 0],
            Vec3::new(0.0, 64.0, 0.0),
            45.0,
        );
        tick_brigand_overrides(&mut ecs, &[Vec3::new(20.0, 64.0, 0.0)], DAY_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(matches!(ai.state, AiState::Chase),
            "Berserker must chase day or night, got {:?}", ai.state);
    }

    #[test]
    fn brigand_resumes_chasing_at_night() {
        // Same scenario as the day test, but at NIGHT_TICK — Brigand
        // should resume the chase promotion.
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Brigand,
            [0, 64, 0],
            Vec3::new(0.0, 64.0, 0.0),
            16.0,
        );
        tick_brigand_overrides(&mut ecs, &[Vec3::new(8.0, 64.0, 0.0)], NIGHT_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(matches!(ai.state, AiState::Chase),
            "Brigand must chase at night, got {:?}", ai.state);
    }

    #[test]
    fn tier_always_aggressive_flag() {
        assert!(!Tier::Brigand.always_aggressive());
        assert!(!Tier::Marauder.always_aggressive());
        assert!(Tier::Berserker.always_aggressive());
    }

    #[test]
    fn override_pass_no_op_when_no_players() {
        let mut ecs = hecs::World::new();
        let id = spawn_test_brigand(
            &mut ecs,
            Tier::Brigand,
            [0, 64, 0],
            Vec3::new(0.0, 64.0, 0.0),
            16.0,
        );
        tick_brigand_overrides(&mut ecs, &[], NIGHT_TICK);
        let ai = ecs.get::<&MobAi>(id).unwrap();
        assert!(!matches!(ai.state, AiState::Chase));
    }
}

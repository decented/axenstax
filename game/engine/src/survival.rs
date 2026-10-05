//! Survival basics (audit wave W2) — fall damage, drowning, the difficulty
//! table, the hurt-vignette fade and the death-screen wording.
//!
//! Every rule here is a pure function so it is unit-tested without a world,
//! and so both simulation paths share ONE copy of each number:
//!
//! - **Local players** (single-player, split-screen, the host's own seat and a
//!   joined client's own body): `game_loop.rs` calls
//!   [`tick_player_survival`] per player in its combat pass, right after
//!   `PlayerCombat::tick`.
//! - **Server-simulated remote players**: `server.rs`
//!   `GameServer::tick_player_physics` calls the same [`tick_player_survival`]
//!   in its own pass over `server_simulated` players.
//!
//! (Single-player still bypasses `GameServer` — the CLAUDE.md "dual-sim" known
//! debt — which is exactly why the rules live here and not in either caller.)
//!
//! Spec: `docs/spec/05-gameplay-systems.md` §1.5 (fall damage), §1.5.1
//! (drowning), §8.5 (difficulty table), §6.7 (death screen).

use crate::block::{self, BlockId};
use crate::combat::PlayerCombat;
use crate::mob::MobType;
use crate::physics::Player;
use crate::play_mode::PlayMode;
use crate::world::World;

// ─── Fall damage (Spec 05 §1.5) ─────────────────────────────────────────────

/// Falls up to this many blocks are free.
pub const SAFE_FALL_BLOCKS: f32 = 3.0;
/// Float slack subtracted before `ceil`, so an exact 3-block drop (whose
/// accumulated per-tick descent can land a hair over 3.0) is still free, and
/// an exact 4-block drop is 1 HP, not 2.
const FALL_EPS: f32 = 1e-3;
/// Hay bale landing multiplier (−80%), Minecraft's `ceil((d − 3) · 0.2)`.
pub const HAY_FALL_MULTIPLIER: f32 = 0.2;

/// Who is falling. Only immunity today (Creative / Spectator); kept a struct so
/// a later rule (Feather Falling boots, a slow-fall effect) is a field, not a
/// signature change at both call sites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FallFlags {
    /// Creative / Spectator: no fall damage at all.
    pub immune: bool,
}

/// Landing-block damage multiplier. Water negates; hay bale −80%. A slime
/// block would be −100% but none exists in the block registry yet — add it
/// here when it ships (Spec 05 §1.5).
pub fn landing_multiplier(landing_block: BlockId) -> f32 {
    match landing_block {
        block::WATER => 0.0,
        block::HAY_BALE => HAY_FALL_MULTIPLIER,
        _ => 1.0,
    }
}

/// Fall damage in HP for a landing (Spec 05 §1.5): 3 blocks are safe, then
/// 1 HP per block beyond, rounded UP (`ceil`). Applies on every difficulty
/// (Peaceful included, as in Minecraft), is never scaled by difficulty and
/// bypasses armour — the caller applies it straight to `PlayerCombat`.
pub fn fall_damage(fall_distance: f32, landing_block: BlockId, flags: FallFlags) -> u32 {
    if flags.immune || !fall_distance.is_finite() {
        return 0;
    }
    let mult = landing_multiplier(landing_block);
    if mult <= 0.0 {
        return 0;
    }
    let over = fall_distance - SAFE_FALL_BLOCKS - FALL_EPS;
    if over <= 0.0 {
        return 0;
    }
    (over * mult).ceil() as u32
}

// ─── Drowning (Spec 05 §1.5.1) ──────────────────────────────────────────────

/// Air supply in ticks: 15 s at 20 TPS.
pub const MAX_AIR_TICKS: u16 = 300;
/// Air regained per tick with the head out of water: 300 / 8 → full in 38
/// ticks (~1.9 s).
pub const AIR_REFILL_PER_TICK: u16 = 8;
/// With no air left, one drowning hit every this many ticks.
pub const DROWN_INTERVAL_TICKS: u16 = 20;
/// HP per drowning hit (bypasses armour).
pub const DROWN_DAMAGE: u32 = 2;
/// Bubbles in the breath bar.
pub const BREATH_BUBBLES: u8 = 10;

/// Per-player breath state. Transient — never persisted (a reload starts with
/// full lungs), and reset to [`Breath::FULL`] on respawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Breath {
    /// Ticks of air left, `0..=MAX_AIR_TICKS`.
    pub air: u16,
    /// Ticks spent at zero air since the last drowning hit.
    pub drown_ticks: u16,
    /// Whether the head was in water on the last tick — drives the HUD bar.
    pub underwater: bool,
}

impl Breath {
    pub const FULL: Breath = Breath { air: MAX_AIR_TICKS, drown_ticks: 0, underwater: false };
}

impl Default for Breath {
    fn default() -> Self {
        Self::FULL
    }
}

/// One tick of breath. Returns the new state and the drowning damage (HP) due
/// this tick. Timeline from full lungs, head held under: air 300 → 0 over 300
/// ticks, then 2 HP on tick 320, 340, 360… Surfacing refills
/// [`AIR_REFILL_PER_TICK`] per tick and clears the drowning counter.
/// `creative` (Creative / Spectator) keeps the lungs full and never drowns.
pub fn tick_breath(state: Breath, head_in_water: bool, creative: bool) -> (Breath, u32) {
    if creative {
        return (Breath::FULL, 0);
    }
    if !head_in_water {
        let air = state.air.saturating_add(AIR_REFILL_PER_TICK).min(MAX_AIR_TICKS);
        return (Breath { air, drown_ticks: 0, underwater: false }, 0);
    }
    if state.air > 0 {
        return (Breath { air: state.air - 1, drown_ticks: 0, underwater: true }, 0);
    }
    let t = state.drown_ticks.saturating_add(1);
    if t >= DROWN_INTERVAL_TICKS {
        (Breath { air: 0, drown_ticks: 0, underwater: true }, DROWN_DAMAGE)
    } else {
        (Breath { air: 0, drown_ticks: t, underwater: true }, 0)
    }
}

/// Bubbles to draw for `air` ticks left: `ceil(air · 10 / 300)`, so the last
/// bubble only pops when the air is truly gone.
pub fn breath_bubbles(air: u16) -> u8 {
    let air = u32::from(air.min(MAX_AIR_TICKS));
    let n = (air * u32::from(BREATH_BUBBLES)).div_ceil(u32::from(MAX_AIR_TICKS));
    n as u8
}

/// The breath bar is shown only while underwater or while it is refilling.
pub fn show_breath_bar(state: &Breath) -> bool {
    state.underwater || state.air < MAX_AIR_TICKS
}

/// Is the player's head (eye cell) in water?
pub fn head_in_water(world: &World, player: &Player) -> bool {
    let eye = player.eye_pos();
    world.is_water(eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32)
}

// ─── Difficulty table (Spec 05 §8.5) ────────────────────────────────────────

/// World difficulty (`WorldMeta.difficulty`). The discriminant indexes
/// [`DIFFICULTY_TABLE`] — keep the order in step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Difficulty {
    Peaceful = 0,
    Easy = 1,
    #[default]
    Normal = 2,
    Hard = 3,
}

impl Difficulty {
    /// Parse the `WorldMeta.difficulty` string. Unknown / empty → Normal (the
    /// meta default).
    /// Allocation-free — it runs per player per tick.
    pub fn from_meta_str(s: &str) -> Self {
        let s = s.trim();
        if s.eq_ignore_ascii_case("peaceful") {
            Difficulty::Peaceful
        } else if s.eq_ignore_ascii_case("easy") {
            Difficulty::Easy
        } else if s.eq_ignore_ascii_case("hard") {
            Difficulty::Hard
        } else {
            Difficulty::Normal
        }
    }

    /// This difficulty's row of [`DIFFICULTY_TABLE`].
    pub fn rules(self) -> &'static DifficultyRules {
        &DIFFICULTY_TABLE[self as usize]
    }
}

/// How mob → player damage `d` scales.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MobDamageRule {
    /// Easy: `min(d / 2 + 1, d)` (Minecraft's Easy formula).
    HalfPlusOne,
    /// Multiply by the factor.
    Scale(f32),
}

/// One row of the difficulty table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DifficultyRules {
    /// The row's own key — lets a test prove the table is indexed in step
    /// with the enum discriminants.
    #[cfg_attr(not(test), allow(dead_code))]
    pub difficulty: Difficulty,
    /// Do hostile mobs attack the player at all? (Peaceful: no.)
    pub hostiles_attack: bool,
    /// Scaling for mob → player damage that does land.
    pub mob_damage: MobDamageRule,
    /// Starvation never takes health below this (HP). `0.0` = starvation can
    /// kill.
    pub starvation_floor: f32,
}

/// THE difficulty table — the one place these numbers live.
///
/// Peaceful is unchanged from before W2: hostiles don't attack, the neutral
/// mobs that still bite (bee, goat, shark) hit at ×1, and starvation keeps the
/// old half-heart floor.
pub static DIFFICULTY_TABLE: [DifficultyRules; 4] = [
    DifficultyRules {
        difficulty: Difficulty::Peaceful,
        hostiles_attack: false,
        mob_damage: MobDamageRule::Scale(1.0),
        starvation_floor: crate::combat::POISON_HEALTH_FLOOR,
    },
    DifficultyRules {
        difficulty: Difficulty::Easy,
        hostiles_attack: true,
        mob_damage: MobDamageRule::HalfPlusOne,
        starvation_floor: 10.0,
    },
    DifficultyRules {
        difficulty: Difficulty::Normal,
        hostiles_attack: true,
        mob_damage: MobDamageRule::Scale(1.0),
        starvation_floor: 1.0,
    },
    DifficultyRules {
        difficulty: Difficulty::Hard,
        hostiles_attack: true,
        mob_damage: MobDamageRule::Scale(1.5),
        starvation_floor: 0.0,
    },
];

/// Scale raw mob → player damage `d` for `difficulty` (before armour).
pub fn scale_mob_damage(d: f32, difficulty: Difficulty) -> f32 {
    match difficulty.rules().mob_damage {
        MobDamageRule::HalfPlusOne => (d / 2.0 + 1.0).min(d),
        MobDamageRule::Scale(k) => d * k,
    }
}

// ─── Damage cause + death screen (Spec 05 §6.7) ─────────────────────────────

/// What last hurt the player — read by the death screen. Set by
/// `PlayerCombat::take_damage_from` on every hit that lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DamageCause {
    /// Anything untagged (commands, unknown sources).
    #[default]
    Generic,
    Fall,
    Drowning,
    Starvation,
    Lava,
    Fire,
    Explosion,
    Mob(MobType),
}

/// "a Brigand" / "an Ocelot".
pub fn with_article(name: &str) -> String {
    let first = name.chars().next().map(|c| c.to_ascii_lowercase());
    if matches!(first, Some('a' | 'e' | 'i' | 'o' | 'u')) {
        format!("an {name}")
    } else {
        format!("a {name}")
    }
}

/// The death screen's cause line (UK English).
pub fn death_message(cause: DamageCause) -> String {
    match cause {
        DamageCause::Fall => "You fell from a high place".to_string(),
        DamageCause::Drowning => "You drowned".to_string(),
        DamageCause::Starvation => "You starved".to_string(),
        DamageCause::Lava => "You tried to swim in lava".to_string(),
        DamageCause::Fire => "You burned to death".to_string(),
        DamageCause::Explosion => "You were blown up".to_string(),
        DamageCause::Mob(kind) => {
            format!("Killed by {}", with_article(&crate::mob::mob_def(kind).name))
        }
        DamageCause::Generic => "You died".to_string(),
    }
}

/// The death screen's grave line.
pub fn grave_message(pos: [i32; 3]) -> String {
    format!("Your items are in a grave at {}, {}, {}", pos[0], pos[1], pos[2])
}

// ─── Hurt vignette ──────────────────────────────────────────────────────────

/// Peak alpha of the red screen-edge vignette (never a full-screen flash).
pub const HURT_VIGNETTE_MAX_ALPHA: f32 = 0.35;

/// Vignette alpha for a hurt flash with `ticks_left` of `total` remaining:
/// linear fade from [`HURT_VIGNETTE_MAX_ALPHA`] to 0.
pub fn hurt_vignette_alpha(ticks_left: u32, total: u32) -> f32 {
    if total == 0 || ticks_left == 0 {
        return 0.0;
    }
    HURT_VIGNETTE_MAX_ALPHA * (ticks_left.min(total) as f32 / total as f32)
}

// ─── The shared per-tick driver ─────────────────────────────────────────────

/// One tick of environmental survival for one player body: turns the physics
/// landing event (if any) into fall damage and advances breath / drowning.
/// Called by BOTH sim paths (see the module docs). Run after the physics tick.
pub fn tick_player_survival(
    player: &mut Player,
    combat: &mut PlayerCombat,
    world: &World,
    mode: PlayMode,
) {
    // Creative / Spectator take no environmental damage (Spec 05 §8.2).
    let immune = mode.flies();
    let landing = player.pending_landing.take();
    if combat.dead {
        return;
    }
    if let Some(l) = landing {
        let dmg = fall_damage(l.fall_distance, l.landing_block, FallFlags { immune });
        if dmg > 0 {
            combat.take_damage_from(dmg as f32, DamageCause::Fall);
        }
    }
    let (breath, drown) = tick_breath(combat.breath, head_in_water(world, player), immune);
    combat.breath = breath;
    if drown > 0 {
        combat.take_damage_from(drown as f32, DamageCause::Drowning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockRegistry;
    use crate::camera::Camera;
    use crate::player_intent::PlayerIntent;
    use glam::Vec3;

    const NONE: FallFlags = FallFlags { immune: false };

    // ── fall damage ──

    #[test]
    fn three_blocks_is_safe() {
        assert_eq!(fall_damage(3.0, block::STONE, NONE), 0);
        assert_eq!(fall_damage(2.0, block::STONE, NONE), 0);
        assert_eq!(fall_damage(0.0, block::STONE, NONE), 0);
        // Float slop over an exact 3-block drop is still free.
        assert_eq!(fall_damage(3.000_2, block::STONE, NONE), 0);
    }

    #[test]
    fn just_over_three_blocks_is_one_hp() {
        assert_eq!(fall_damage(3.1, block::STONE, NONE), 1);
    }

    #[test]
    fn four_blocks_is_one_hp_and_a_hair_more_is_two() {
        assert_eq!(fall_damage(4.0, block::STONE, NONE), 1);
        assert_eq!(fall_damage(4.1, block::STONE, NONE), 2);
        assert_eq!(fall_damage(10.0, block::STONE, NONE), 7);
    }

    #[test]
    fn lethal_from_above_22_blocks_at_20_hp() {
        assert_eq!(fall_damage(22.0, block::STONE, NONE), 19);
        assert_eq!(fall_damage(23.0, block::STONE, NONE), 20);
    }

    #[test]
    fn water_negates_fall_damage() {
        assert_eq!(fall_damage(50.0, block::WATER, NONE), 0);
    }

    #[test]
    fn hay_bale_cuts_eighty_percent() {
        // ceil((d − 3) · 0.2)
        assert_eq!(fall_damage(3.0, block::HAY_BALE, NONE), 0);
        assert_eq!(fall_damage(4.0, block::HAY_BALE, NONE), 1);
        assert_eq!(fall_damage(8.0, block::HAY_BALE, NONE), 1);
        assert_eq!(fall_damage(8.1, block::HAY_BALE, NONE), 2);
        assert_eq!(fall_damage(23.0, block::HAY_BALE, NONE), 4);
    }

    #[test]
    fn creative_is_immune() {
        assert_eq!(fall_damage(100.0, block::STONE, FallFlags { immune: true }), 0);
    }

    #[test]
    fn non_finite_distance_is_harmless() {
        assert_eq!(fall_damage(f32::NAN, block::STONE, NONE), 0);
        assert_eq!(fall_damage(f32::INFINITY, block::STONE, NONE), 0);
    }

    // ── fall tracking through the real physics ──

    fn floor_world(top_block: BlockId) -> (World, BlockRegistry) {
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -3..=3 {
            for z in -3..=3 {
                world.set_block(x, 63, z, top_block);
            }
        }
        (world, registry)
    }

    /// Drop a player from `height` blocks above the floor top (y = 64) and
    /// return the landing event.
    fn drop_from(height: f32, world: &World, registry: &BlockRegistry) -> Option<crate::physics::Landing> {
        let start = Vec3::new(0.5, 64.0 + height, 0.5);
        let cam = Camera::new(start, 1.0);
        let mut p = Player::new(start);
        let idle = PlayerIntent::default();
        let mut landing = None;
        for _ in 0..200 {
            p.tick(&idle, &cam, world, registry, PlayMode::Survival);
            if let Some(l) = p.pending_landing.take() {
                landing = Some(l);
            }
            if p.on_ground && landing.is_some() {
                break;
            }
        }
        landing
    }

    #[test]
    fn physics_reports_the_fall_distance_on_landing() {
        let (world, registry) = floor_world(block::STONE);
        let l = drop_from(10.0, &world, &registry).expect("must land");
        assert!((l.fall_distance - 10.0).abs() < 1e-2, "fell {}", l.fall_distance);
        assert_eq!(l.landing_block, block::STONE);
        assert_eq!(fall_damage(l.fall_distance, l.landing_block, NONE), 7);
    }

    #[test]
    fn physics_exact_three_block_drop_is_free() {
        let (world, registry) = floor_world(block::STONE);
        let l = drop_from(3.0, &world, &registry).expect("must land");
        assert_eq!(fall_damage(l.fall_distance, l.landing_block, NONE), 0, "fell {}", l.fall_distance);
    }

    #[test]
    fn physics_reports_hay_as_the_landing_block() {
        let (world, registry) = floor_world(block::HAY_BALE);
        let l = drop_from(10.0, &world, &registry).expect("must land");
        assert_eq!(l.landing_block, block::HAY_BALE);
        assert_eq!(fall_damage(l.fall_distance, l.landing_block, NONE), 2);
    }

    #[test]
    fn physics_splash_into_water_is_a_water_landing() {
        let (mut world, registry) = floor_world(block::STONE);
        // Two deep pool on top of the floor.
        for x in -3..=3 {
            for z in -3..=3 {
                world.set_block(x, 64, z, block::WATER);
                world.set_block(x, 65, z, block::WATER);
            }
        }
        let start = Vec3::new(0.5, 80.0, 0.5);
        let cam = Camera::new(start, 1.0);
        let mut p = Player::new(start);
        let idle = PlayerIntent::default();
        let mut worst = 0;
        for _ in 0..300 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
            if let Some(l) = p.pending_landing.take() {
                worst = worst.max(fall_damage(l.fall_distance, l.landing_block, NONE));
            }
        }
        assert_eq!(worst, 0, "a 14-block drop into water must do no damage");
    }

    #[test]
    fn a_ladder_resets_the_fall() {
        let (mut world, registry) = floor_world(block::STONE);
        // A ladder column from the floor up to y = 80.
        for y in 64..=80 {
            world.set_block(0, y, 0, block::LADDER);
        }
        let start = Vec3::new(0.5, 80.0, 0.5);
        let cam = Camera::new(start, 1.0);
        let mut p = Player::new(start);
        // Sneak = descend the ladder.
        let down = PlayerIntent { sneak: true, ..PlayerIntent::default() };
        let mut worst = 0;
        for _ in 0..400 {
            p.tick(&down, &cam, &world, &registry, PlayMode::Survival);
            if let Some(l) = p.pending_landing.take() {
                worst = worst.max(fall_damage(l.fall_distance, l.landing_block, NONE));
            }
        }
        assert_eq!(worst, 0, "climbing down a ladder is not a fall");
    }

    // ── breath ──

    #[test]
    fn breath_timeline_300_ticks_then_2hp_every_20() {
        let mut b = Breath::FULL;
        let mut hits = Vec::new();
        for tick in 1..=360u32 {
            let (nb, dmg) = tick_breath(b, true, false);
            b = nb;
            if tick == 300 {
                assert_eq!(b.air, 0, "air runs out on tick 300");
            }
            if tick < 300 {
                assert!(b.air > 0);
            }
            if dmg > 0 {
                assert_eq!(dmg, DROWN_DAMAGE);
                hits.push(tick);
            }
        }
        assert_eq!(hits, vec![320, 340, 360]);
    }

    #[test]
    fn breath_refills_in_about_two_seconds() {
        let mut b = Breath { air: 0, drown_ticks: 7, underwater: true };
        let mut ticks = 0;
        while b.air < MAX_AIR_TICKS {
            b = tick_breath(b, false, false).0;
            ticks += 1;
            assert_eq!(b.drown_ticks, 0, "surfacing clears the drowning counter");
            assert!(!b.underwater);
        }
        assert_eq!(ticks, 38, "300 / 8 per tick, ~1.9 s");
    }

    #[test]
    fn creative_never_drowns() {
        let mut b = Breath { air: 0, drown_ticks: 19, underwater: true };
        for _ in 0..100 {
            let (nb, dmg) = tick_breath(b, true, true);
            assert_eq!(dmg, 0);
            b = nb;
        }
        assert_eq!(b, Breath::FULL);
    }

    #[test]
    fn breath_bubbles_and_visibility() {
        assert_eq!(breath_bubbles(MAX_AIR_TICKS), 10);
        assert_eq!(breath_bubbles(299), 10);
        assert_eq!(breath_bubbles(270), 9);
        assert_eq!(breath_bubbles(1), 1);
        assert_eq!(breath_bubbles(0), 0);
        assert!(!show_breath_bar(&Breath::FULL), "hidden on dry land with full lungs");
        assert!(show_breath_bar(&Breath { air: MAX_AIR_TICKS, drown_ticks: 0, underwater: true }));
        assert!(show_breath_bar(&Breath { air: 120, drown_ticks: 0, underwater: false }), "shown while refilling");
    }

    // ── difficulty ──

    #[test]
    fn table_rows_are_indexed_by_discriminant() {
        for d in [Difficulty::Peaceful, Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
            assert_eq!(d.rules().difficulty, d);
        }
    }

    #[test]
    fn difficulty_parses_meta_strings() {
        assert_eq!(Difficulty::from_meta_str("peaceful"), Difficulty::Peaceful);
        assert_eq!(Difficulty::from_meta_str("Easy"), Difficulty::Easy);
        assert_eq!(Difficulty::from_meta_str("normal"), Difficulty::Normal);
        assert_eq!(Difficulty::from_meta_str("hard"), Difficulty::Hard);
        assert_eq!(Difficulty::from_meta_str(""), Difficulty::Normal);
        assert_eq!(Difficulty::from_meta_str("nonsense"), Difficulty::Normal);
    }

    #[test]
    fn mob_damage_scaling_per_difficulty() {
        // Easy: min(d/2 + 1, d)
        assert_eq!(scale_mob_damage(3.0, Difficulty::Easy), 2.5);
        assert_eq!(scale_mob_damage(6.0, Difficulty::Easy), 4.0);
        assert_eq!(scale_mob_damage(1.0, Difficulty::Easy), 1.0, "never more than d");
        // Normal ×1, Hard ×1.5
        assert_eq!(scale_mob_damage(3.0, Difficulty::Normal), 3.0);
        assert_eq!(scale_mob_damage(3.0, Difficulty::Hard), 4.5);
        // Peaceful: hostiles don't attack; neutral bites unchanged.
        assert!(!Difficulty::Peaceful.rules().hostiles_attack);
        assert_eq!(scale_mob_damage(3.0, Difficulty::Peaceful), 3.0);
        assert!(Difficulty::Easy.rules().hostiles_attack);
        assert!(Difficulty::Hard.rules().hostiles_attack);
    }

    fn starve(difficulty: Difficulty) -> PlayerCombat {
        let mut c = PlayerCombat::new();
        c.starvation_floor = difficulty.rules().starvation_floor;
        c.hunger = 0;
        for _ in 0..(crate::combat::STARVATION_INTERVAL_TICKS * 50) {
            // Keep hunger pinned at zero (the drain can't go lower anyway).
            c.tick();
            if c.dead {
                break;
            }
        }
        c
    }

    #[test]
    fn starvation_floor_easy_is_10_hp() {
        let c = starve(Difficulty::Easy);
        assert!(!c.dead);
        assert_eq!(c.health, 10.0);
    }

    #[test]
    fn starvation_floor_normal_is_1_hp() {
        let c = starve(Difficulty::Normal);
        assert!(!c.dead);
        assert_eq!(c.health, 1.0);
    }

    #[test]
    fn starvation_kills_on_hard() {
        let c = starve(Difficulty::Hard);
        assert!(c.dead, "Hard starvation can kill");
        assert_eq!(c.health, 0.0);
        assert_eq!(c.last_damage, DamageCause::Starvation);
    }

    #[test]
    fn starvation_peaceful_keeps_old_half_heart_floor() {
        let c = starve(Difficulty::Peaceful);
        assert!(!c.dead);
        assert_eq!(c.health, crate::combat::POISON_HEALTH_FLOOR);
    }

    // ── death screen wording ──

    #[test]
    fn death_messages() {
        assert_eq!(death_message(DamageCause::Fall), "You fell from a high place");
        assert_eq!(death_message(DamageCause::Drowning), "You drowned");
        assert_eq!(death_message(DamageCause::Starvation), "You starved");
        assert_eq!(death_message(DamageCause::Generic), "You died");
        assert_eq!(death_message(DamageCause::Mob(MobType::Brigand)), "Killed by a Brigand");
        assert_eq!(with_article("Ocelot"), "an Ocelot");
        assert_eq!(grave_message([1, -2, 3]), "Your items are in a grave at 1, -2, 3");
    }

    #[test]
    fn take_damage_from_records_the_cause() {
        let mut c = PlayerCombat::new();
        assert!(c.take_damage_from(2.0, DamageCause::Fall));
        assert_eq!(c.last_damage, DamageCause::Fall);
        // An i-framed hit doesn't land, so it doesn't overwrite the cause.
        assert!(!c.take_damage_from(2.0, DamageCause::Drowning));
        assert_eq!(c.last_damage, DamageCause::Fall);
    }

    // ── hurt vignette ──

    #[test]
    fn vignette_fades_and_never_exceeds_the_cap() {
        let total = crate::combat::PLAYER_HURT_FLASH_TICKS;
        assert_eq!(hurt_vignette_alpha(0, total), 0.0);
        assert!((hurt_vignette_alpha(total, total) - HURT_VIGNETTE_MAX_ALPHA).abs() < 1e-6);
        assert!(hurt_vignette_alpha(total + 5, total) <= HURT_VIGNETTE_MAX_ALPHA);
        assert!(hurt_vignette_alpha(total / 2, total) < hurt_vignette_alpha(total, total));
        // ~0.4 s at 20 TPS.
        assert_eq!(total, 8);
    }

    // ── the shared driver ──

    #[test]
    fn driver_applies_fall_damage_bypassing_nothing_but_creative() {
        let world = World::new();
        let mut p = Player::new(Vec3::new(0.5, 100.0, 0.5));
        let mut c = PlayerCombat::new();
        p.pending_landing = Some(crate::physics::Landing { fall_distance: 10.0, landing_block: block::STONE });
        tick_player_survival(&mut p, &mut c, &world, PlayMode::Survival);
        assert_eq!(c.health, 13.0);
        assert_eq!(c.last_damage, DamageCause::Fall);
        assert!(p.pending_landing.is_none(), "the landing is consumed");

        let mut c2 = PlayerCombat::new();
        p.pending_landing = Some(crate::physics::Landing { fall_distance: 50.0, landing_block: block::STONE });
        tick_player_survival(&mut p, &mut c2, &world, PlayMode::Creative);
        assert_eq!(c2.health, 20.0);
    }

    #[test]
    fn driver_drowns_a_submerged_player() {
        let mut world = World::new();
        for y in 60..=70 {
            world.set_block(0, y, 0, block::WATER);
        }
        let mut p = Player::new(Vec3::new(0.5, 62.0, 0.5));
        let mut c = PlayerCombat::new();
        for _ in 0..320 {
            tick_player_survival(&mut p, &mut c, &world, PlayMode::Survival);
        }
        assert_eq!(c.health, 18.0);
        assert_eq!(c.last_damage, DamageCause::Drowning);
    }
}

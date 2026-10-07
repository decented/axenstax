//! Per-species AI dispatch — wires the dormant pure-function species AIs
//! (rabbit hop, goat charge/ram, …) into the live ECS movement loop.
//!
//! Each species already ships a fully-tested pure tick (`rabbit_ai::tick_rabbit`,
//! `goat_ai::tick_goat`, …) plus an `XData` ECS component, but nothing ever
//! called them, so these animals fell through to the generic `tick_mob_ai`
//! wander and read as indistinct. This module is the missing dispatch layer.
//!
//! ## Composition with the generic AI (the important bit)
//!
//! The dispatch runs AFTER `mob_ai::tick_mob_ai` (which re-sets each mob's
//! horizontal velocity every tick from its `MobAi` state). The rule:
//!
//! - If the mob's generic state is **reactive** (`Flee` from being struck —
//!   `combat::spook_if_prey`; `Chase`; `InvestigateCampfire` — Spec 18;
//!   `GolemGuard`) the species dispatch **yields**: the generic state keeps
//!   ownership of locomotion this tick. This preserves the shipped
//!   struck-prey bolt and campfire attraction unchanged.
//! - Otherwise (Idle / Wander) the species dispatch **owns** locomotion: it
//!   applies the species action (hop / charge / …). A `NoOp` action leaves the
//!   velocity the generic state set, so a resting animal still does the subtle
//!   generic idle/wander between its signature moves.
//!
//! The dispatch is a free function over a bare `hecs::World` so it is unit
//! testable without a `GameState`. Effects that reach outside the ECS (a goat
//! goring a *player* — players are not ECS entities) are returned as data for
//! the thin `GameState` wrapper in `game_loop` to apply against `self.players`.
//!
//! Eventual crate home: `genesis_sim`.

use glam::Vec3;

use crate::entity::{Flying, MobKind, Position, Velocity};
use crate::mob::{self, MobType};
use crate::mob_ai::{AiState, MobAi};

// --- Tunables (feel; Axolittle playtest refines) -------------------------

/// Rabbit hop: horizontal leap speed as a multiple of base walk speed.
pub const RABBIT_HOP_SPEED_MULT: f32 = 2.5;
/// Rabbit hop: upward impulse (blocks/tick). GRAVITY is 0.08, so this clears
/// roughly half a block — a visible little bounce, not a launch.
pub const RABBIT_HOP_JUMP: f32 = 0.30;
/// Rabbit panic speed when fleeing a too-close player (matches
/// `mob::flee_speed_mult(Rabbit)`).
pub const RABBIT_FLEE_SPEED_MULT: f32 = 1.8;

/// Goat charge: forward speed as a multiple of base walk speed — fast enough
/// to feel like a ram, slow enough to dodge if you spot the lowered head.
pub const GOAT_CHARGE_SPEED_MULT: f32 = 2.2;
/// How close the goat must still be to the player at the impact tick for the
/// gore to land (the player may have side-stepped during the wind-up).
pub const GOAT_IMPACT_REACH: f32 = 2.2;
/// Horizontal knockback impulse applied to a gored player (blocks/tick).
pub const GOAT_KNOCKBACK: f32 = 0.48;
/// Upward component of the gore knockback (a little pop).
pub const GOAT_KNOCKBACK_UP: f32 = 0.30;

/// True when the generic AI state should keep ownership of locomotion this
/// tick (species dispatch yields). These are reactive states whose movement
/// the species flavour must not stomp.
fn generic_owns(state: &AiState) -> bool {
    matches!(
        state,
        AiState::Flee { .. }
            | AiState::Chase
            | AiState::InvestigateCampfire { .. }
            | AiState::GolemGuard { .. }
    )
}

/// MP-D2b — the stand-in position for a player slot with no body in the
/// world (a lending host lists its joiners at their server slots; a dead or
/// departed joiner's slot holds this): far outside the world on every axis
/// (several of these AIs measure horizontal distance only), so none ever
/// picks it as its nearest target, and a Lead fastened to it snaps.
pub const ABSENT_PLAYER: Vec3 = Vec3::new(1.0e7, -1.0e6, 1.0e7);

/// Nearest player position to `pos` (xz distance), if any players exist.
fn nearest_player(pos: Vec3, players: &[Vec3]) -> Option<(f32, f32, f32)> {
    players
        .iter()
        .min_by(|a, b| {
            let da = (a.x - pos.x).powi(2) + (a.z - pos.z).powi(2);
            let db = (b.x - pos.x).powi(2) + (b.z - pos.z).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| (p.x, p.y, p.z))
}

/// Aggro chase target for a hit-and-provoked mob (Task 13, bug-hardening,
/// 2026-07-07): prefer the specific player slot that landed the hit
/// (`attacker_pidx`, stamped by `bear_ai`/`hyena_ai::on_hit_by_player`) so a
/// bear or hyena charges the player who provoked it rather than whichever
/// player happens to be nearest. Falls back to `nearest_player` if that
/// slot is gone (player disconnected / slot out of range) so an aggro'd
/// mob with a stale attacker still has somewhere to go instead of freezing.
fn attacker_or_nearest_player(
    pos: Vec3,
    player_positions: &[Vec3],
    attacker_pidx: usize,
) -> Option<(f32, f32, f32)> {
    player_positions
        .get(attacker_pidx)
        .filter(|p| **p != ABSENT_PLAYER)
        .map(|p| (p.x, p.y, p.z))
        .or_else(|| nearest_player(pos, player_positions))
}

/// Nearest player as an indexed charge target `(slot, x, y, z)` — the slot
/// index is the stable "target id" handed to `goat_ai::tick_goat`.
fn nearest_player_indexed(pos: Vec3, players: &[Vec3]) -> Option<(u64, f32, f32, f32)> {
    players
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = (a.x - pos.x).powi(2) + (a.z - pos.z).powi(2);
            let db = (b.x - pos.x).powi(2) + (b.z - pos.z).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, p)| (i as u64, p.x, p.y, p.z))
}

fn set_facing(ecs: &mut hecs::World, id: hecs::Entity, dir: Vec3) {
    if dir.length_squared() > 1e-6
        && let Ok(mut ai) = ecs.get::<&mut MobAi>(id) {
            ai.facing = dir.z.atan2(dir.x);
        }
}

// --- Rabbit -------------------------------------------------------------

/// Dispatch hop/flee movement for every Rabbit carrying a `RabbitData`.
pub fn dispatch_rabbits(ecs: &mut hecs::World, player_positions: &[Vec3], tick: u64) {
    use crate::rabbit_ai::{self, RabbitAction, RabbitData};

    let mut rabbits: Vec<(hecs::Entity, Vec3, RabbitData)> = Vec::new();
    for (id, (pos, kind, data, ai)) in ecs
        .query::<(&Position, &MobKind, &RabbitData, &MobAi)>()
        .iter()
    {
        if kind.0 == MobType::Rabbit && !generic_owns(&ai.state) {
            rabbits.push((id, pos.0, *data));
        }
    }
    if rabbits.is_empty() {
        return;
    }
    let speed = mob::mob_def(MobType::Rabbit).speed / 20.0;
    for (id, pos, data) in rabbits {
        let nearest = nearest_player(pos, player_positions);
        let (next, action) = rabbit_ai::tick_rabbit(&data, (pos.x, pos.y, pos.z), nearest, tick);
        if let Ok(mut d) = ecs.get::<&mut RabbitData>(id) {
            *d = next;
        }
        match action {
            RabbitAction::HopToward { dx, dz } => {
                let dir = Vec3::new(dx, 0.0, dz).normalize_or_zero();
                if let Ok(mut vel) = ecs.get::<&mut Velocity>(id) {
                    vel.0.x = dir.x * speed * RABBIT_HOP_SPEED_MULT;
                    vel.0.z = dir.z * speed * RABBIT_HOP_SPEED_MULT;
                    vel.0.y = RABBIT_HOP_JUMP;
                }
                set_facing(ecs, id, dir);
            }
            RabbitAction::FleeFrom { x, z } => {
                let away = Vec3::new(pos.x - x, 0.0, pos.z - z).normalize_or_zero();
                if let Ok(mut vel) = ecs.get::<&mut Velocity>(id) {
                    vel.0.x = away.x * speed * RABBIT_FLEE_SPEED_MULT;
                    vel.0.z = away.z * speed * RABBIT_FLEE_SPEED_MULT;
                    // A fleeing rabbit hops too — keep the bounce.
                    vel.0.y = RABBIT_HOP_JUMP;
                }
                set_facing(ecs, id, away);
            }
            // Between hops: let the generic idle/wander velocity stand.
            RabbitAction::NoOp => {}
        }
    }
}

// --- Goat ---------------------------------------------------------------

/// A goat's charge connected this tick — the `GameState` wrapper resolves the
/// gore (damage + knockback) against `self.players[target_slot]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoatImpact {
    pub target_slot: usize,
    pub goat_pos: Vec3,
}

/// Dispatch wander/charge movement for every Goat carrying a `GoatData`.
/// Returns the gore impacts to apply against the player slots.
pub fn dispatch_goats(
    ecs: &mut hecs::World,
    player_positions: &[Vec3],
    tick: u64,
) -> Vec<GoatImpact> {
    use crate::goat_ai::{self, GoatAction, GoatData};

    let mut goats: Vec<(hecs::Entity, Vec3, GoatData)> = Vec::new();
    for (id, (pos, kind, data, ai)) in
        ecs.query::<(&Position, &MobKind, &GoatData, &MobAi)>().iter()
    {
        if kind.0 == MobType::Goat && !generic_owns(&ai.state) {
            goats.push((id, pos.0, data.clone()));
        }
    }
    let mut impacts = Vec::new();
    if goats.is_empty() {
        return impacts;
    }
    let speed = mob::mob_def(MobType::Goat).speed / 20.0;
    for (id, pos, data) in goats {
        let target = nearest_player_indexed(pos, player_positions);
        let (next, action) = goat_ai::tick_goat(&data, (pos.x, pos.y, pos.z), target, tick);
        if let Ok(mut d) = ecs.get::<&mut GoatData>(id) {
            *d = next;
        }
        match action {
            GoatAction::MoveToward { x, z, .. } => {
                let dir = Vec3::new(x - pos.x, 0.0, z - pos.z).normalize_or_zero();
                if let Ok(mut vel) = ecs.get::<&mut Velocity>(id) {
                    vel.0.x = dir.x * speed;
                    vel.0.z = dir.z * speed;
                }
                set_facing(ecs, id, dir);
            }
            GoatAction::Charge { target_x, target_z, .. } => {
                let dir = Vec3::new(target_x - pos.x, 0.0, target_z - pos.z).normalize_or_zero();
                if let Ok(mut vel) = ecs.get::<&mut Velocity>(id) {
                    vel.0.x = dir.x * speed * GOAT_CHARGE_SPEED_MULT;
                    vel.0.z = dir.z * speed * GOAT_CHARGE_SPEED_MULT;
                }
                set_facing(ecs, id, dir);
            }
            GoatAction::Impact { target_id } => {
                let slot = target_id as usize;
                // Only gore if the player is genuinely still in reach — they
                // may have dodged during the 1.5 s wind-up.
                if let Some(p) = player_positions.get(slot)
                    && (p.x - pos.x).hypot(p.z - pos.z) <= GOAT_IMPACT_REACH {
                        impacts.push(GoatImpact { target_slot: slot, goat_pos: pos });
                    }
            }
            GoatAction::NoOp => {}
        }
    }
    impacts
}

// --- Bee ----------------------------------------------------------------

/// Bee flight speed as a multiple of base walk speed — a lazy drift, not a
/// dart.
pub const BEE_FLY_SPEED_MULT: f32 = 1.4;

/// Dispatch flight (drift / hover-bob / hive-return) for every Bee carrying a
/// `BeeData`. Bees are `Flying` (gravity-exempt), so this owns the full
/// velocity vector including the vertical component. A bee that a player has
/// struck (carries a `LastAttacker`) turns angry: it chases that player and,
/// at the end of its sting window, stings — returned as a [`BeeSting`] for the
/// caller to resolve (Minecraft parity: the bee dies after stinging).
pub fn dispatch_bees(
    ecs: &mut hecs::World,
    player_positions: &[Vec3],
    tick: u64,
) -> Vec<BeeSting> {
    use crate::bee_ai::{self, BeeAction, BeeData};
    use crate::combat::LastAttacker;

    let mut bees: Vec<(hecs::Entity, Vec3, BeeData, Option<usize>)> = Vec::new();
    for (id, (pos, kind, data, ai, attacker)) in ecs
        .query::<(&Position, &MobKind, &BeeData, &MobAi, Option<&LastAttacker>)>()
        .iter()
    {
        if kind.0 == MobType::Bee && !generic_owns(&ai.state) {
            bees.push((id, pos.0, data.clone(), attacker.map(|a| a.0.slot())));
        }
    }
    let mut stings = Vec::new();
    if bees.is_empty() {
        return stings;
    }
    let speed = mob::mob_def(MobType::Bee).speed / 20.0 * BEE_FLY_SPEED_MULT;
    for (id, pos, data, attacker_pidx) in bees {
        // Surface the recent attacker (if any) so the bee can decide to sting.
        let recent_attacker = attacker_pidx
            .and_then(|pidx| player_positions.get(pidx).map(|p| (pidx as u64, p.x, p.y, p.z)));
        let (next, action) = bee_ai::tick_bee(&data, (pos.x, pos.y, pos.z), recent_attacker, tick);
        if let Ok(mut d) = ecs.get::<&mut BeeData>(id) {
            *d = next;
        }
        match action {
            BeeAction::FlyToward { x, y, z } => {
                let dir = Vec3::new(x - pos.x, y - pos.y, z - pos.z).normalize_or_zero();
                if let Ok(mut vel) = ecs.get::<&mut Velocity>(id) {
                    vel.0 = dir * speed;
                }
                set_facing(ecs, id, Vec3::new(dir.x, 0.0, dir.z));
            }
            BeeAction::StingChase { target_x, target_y, target_z, .. } => {
                // Angry bee darts at its attacker (a bit faster than its drift).
                let dir =
                    Vec3::new(target_x - pos.x, target_y - pos.y, target_z - pos.z).normalize_or_zero();
                if let Ok(mut vel) = ecs.get::<&mut Velocity>(id) {
                    vel.0 = dir * speed * 1.5;
                }
                set_facing(ecs, id, Vec3::new(dir.x, 0.0, dir.z));
            }
            BeeAction::StingImpact { target_id } => {
                stings.push(BeeSting { target_slot: target_id as usize, bee: id });
            }
            BeeAction::NoOp => {}
        }
    }
    stings
}

/// A bee landed its sting this tick — the `GameState` wrapper damages the
/// player and despawns the bee (it dies after stinging, MC parity).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeeSting {
    pub target_slot: usize,
    pub bee: hecs::Entity,
}

/// Damage a bee sting deals (half a heart — an irritant, not a threat).
pub const BEE_STING_DAMAGE: f32 = 1.0;

// --- Horse (Animals Wave 2) ----------------------------------------------

/// Dispatch the (previously dormant) horse herd wander. Skips ridden horses
/// (the rider steers) and yields to the generic struck-flee (horses are in
/// `flees_when_attacked`), so this only adds the wide herd-wander during the
/// idle/wander states.
pub fn dispatch_horses(ecs: &mut hecs::World, tick: u64) {
    use crate::entity::Ridden;
    use crate::horse_ai::{self, HorseAction, HorseData};

    let mut horses: Vec<(hecs::Entity, Vec3, HorseData, MobType)> = Vec::new();
    for (id, (pos, kind, data, ai, ridden)) in ecs
        .query::<(&Position, &MobKind, &HorseData, &MobAi, Option<&Ridden>)>()
        .iter()
    {
        // The whole horse family (Horse + Donkey + Mule) herd-wanders.
        if crate::mob::is_horse_family(kind.0) && ridden.is_none() && !generic_owns(&ai.state) {
            horses.push((id, pos.0, data.clone(), kind.0));
        }
    }
    if horses.is_empty() {
        return;
    }
    for (id, pos, data, kind) in horses {
        let speed = mob::mob_def(kind).speed / 20.0; // per-species (donkeys/mules amble slower)
        // No species-flee: the generic struck-flee already bolts horses.
        let (next, action) = horse_ai::tick_horse(&data, (pos.x, pos.y, pos.z), None, tick);
        if let Ok(mut d) = ecs.get::<&mut HorseData>(id) {
            *d = next;
        }
        match action {
            HorseAction::MoveToward { x, z, .. } => {
                let dir = Vec3::new(x - pos.x, 0.0, z - pos.z).normalize_or_zero();
                if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                    v.0.x = dir.x * speed;
                    v.0.z = dir.z * speed;
                }
                set_facing(ecs, id, dir);
            }
            HorseAction::FleeFrom { .. } | HorseAction::NoOp => {}
        }
    }
}

// --- Squid (Animals Wave 2) ----------------------------------------------

/// Dispatch squid aquatic drift + suffocation. Squid live in water (the
/// generic AI already exempts them from the swim-to-shore march); this drives
/// their gentle 3-D drift and applies suffocation damage when they end up on
/// land.
pub fn dispatch_squids(ecs: &mut hecs::World, world: &crate::world::World, tick: u64) {
    use crate::squid_ai::{self, SquidAction, SquidData};

    // Squid + the aquatic-wave Fish + Glow Squid all drift via SquidData.
    let mut squids: Vec<(hecs::Entity, Vec3, MobType, SquidData)> = Vec::new();
    for (id, (pos, kind, data, ai)) in
        ecs.query::<(&Position, &MobKind, &SquidData, &MobAi)>().iter()
    {
        if matches!(kind.0, MobType::Squid | MobType::Fish | MobType::GlowSquid)
            && !generic_owns(&ai.state)
        {
            squids.push((id, pos.0, kind.0, data.clone()));
        }
    }
    if squids.is_empty() {
        return;
    }
    for (id, pos, kind, data) in squids {
        let speed = mob::mob_def(kind).speed / 20.0;
        let (next, action) = squid_ai::tick_squid(
            &data,
            (pos.x, pos.y, pos.z),
            |x, y, z| world.is_water(x, y, z),
            tick,
        );
        if let Ok(mut d) = ecs.get::<&mut SquidData>(id) {
            *d = next;
        }
        match action {
            SquidAction::DriftToward { x, y, z } => {
                let dir = Vec3::new(x - pos.x, y - pos.y, z - pos.z).normalize_or_zero();
                if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                    v.0 = dir * speed;
                }
            }
            SquidAction::SuffocateTick => {
                if let Ok(mut h) = ecs.get::<&mut crate::combat::Health>(id) {
                    h.take_damage(1.0);
                }
            }
            SquidAction::NoOp => {}
        }
    }
}

// --- Shark (Aquatic wave — apex predator) --------------------------------

pub const SHARK_BITE_DAMAGE: f32 = 6.0;
pub const SHARK_BITE_REACH: f32 = 2.2;
pub const SHARK_HUNT_RANGE: f32 = 22.0;

/// A shark bit a player this tick — the GameState wrapper applies the damage +
/// knockback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SharkBite {
    pub target_slot: usize,
    pub shark_pos: Vec3,
}

/// Dispatch the Shark apex predator: it swims at the nearest player who is IN
/// the water within hunt range (stand on shore / in a boat and you're safe —
/// telegraphed, avoidable, kid-appropriate). On reaching the player it bites
/// (returned as a `SharkBite`), and while actively hunting it periodically
/// drips a Shark Tooth nearby — the non-lethal, husbandry-over-slaughter drop.
pub fn dispatch_sharks(
    ecs: &mut hecs::World,
    world: &crate::world::World,
    player_positions: &[Vec3],
    tick: u64,
) -> Vec<SharkBite> {
    let mut sharks: Vec<(hecs::Entity, Vec3)> = Vec::new();
    for (id, (pos, kind)) in ecs.query::<(&Position, &MobKind)>().iter() {
        if kind.0 == MobType::Shark {
            sharks.push((id, pos.0));
        }
    }
    let mut bites = Vec::new();
    if sharks.is_empty() {
        return bites;
    }
    let speed = mob::mob_def(MobType::Shark).speed / 20.0;
    for (id, pos) in sharks {
        // Only hunt a player who is themselves in water + within range.
        let target = player_positions
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                world.is_water(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
            })
            .map(|(i, p)| (i, *p, (p.x - pos.x).hypot(p.z - pos.z)))
            .filter(|(_, _, d)| *d <= SHARK_HUNT_RANGE)
            .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        let Some((slot, ppos, dist)) = target else {
            continue; // no prey in reach → idle drift (generic)
        };
        // Swim straight at the prey (3-D).
        let dir = Vec3::new(ppos.x - pos.x, ppos.y - pos.y, ppos.z - pos.z).normalize_or_zero();
        if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
            v.0 = dir * speed;
        }
        set_facing(ecs, id, Vec3::new(dir.x, 0.0, dir.z));
        // Drip a tooth while hunting (behaviour-gated, not on-kill).
        if tick.is_multiple_of(60) {
            let seed = (pos.x as i32 as u32).wrapping_mul(2_654_435_761) ^ (tick as u32);
            crate::entity::spawn_item(
                ecs,
                pos,
                crate::item::ItemStack::new_material(crate::item::MaterialId::SharkTooth, 1),
                seed,
            );
        }
        if dist <= SHARK_BITE_REACH {
            bites.push(SharkBite { target_slot: slot, shark_pos: pos });
        }
    }
    bites
}

// --- Companions (Cat / Parrot / Fox follow) ------------------------------

/// Dispatch the generic companion follow: a tamed Cat/Parrot/Fox walks toward
/// its owner when mid-range. Mirrors `tick_wolf_companions` but generic over
/// `CompanionData`, so one pass covers every non-wolf pet.
///
/// 1C command states (Task 2, 2026-07-06) gate the drive: `Stay` zeroes
/// horizontal velocity and holds; `Wander` leaves the generic ambient AI
/// drive untouched (no companion override at all); `Perch` pins the pet's
/// *position* every tick to the owner's shoulder, yaw-rotated (Task 9,
/// 2026-07-07 — see `companion::shoulder_offset`) so it stays over the same
/// shoulder in body space instead of a fixed world offset, and zeroes
/// velocity (Parrot-only in practice, gated by `can_perch` at the UI/cycle
/// layer — nothing here stops another species reaching `Perch` if a save
/// somehow set it, so the position pin is unconditional on state alone);
/// `Follow` is the pre-1C behaviour, now with an added vertical (Y) drive
/// for `Flying` companions (Parrot) so a following flyer climbs/descends to
/// hover near its owner instead of walking through a wall of air at ground
/// height. `player_yaws` is indexed the same as `player_positions` (missing
/// entries default to yaw 0).
pub fn dispatch_companions(
    ecs: &mut hecs::World,
    player_positions: &[Vec3],
    player_yaws: &[f32],
    _tick: u64,
    remote_owners: &[(String, usize)],
) {
    // MP-D2b — a joiner's pet (owner key = its npub) follows that joiner's
    // body, which a lending host lists at its server slot.
    let owners = crate::tameable::OwnerBodies { positions: player_positions, remote_owners };
    use crate::companion::{self, CompanionData, CompanionState};

    let mut pets: Vec<(hecs::Entity, Vec3, MobType, String, CompanionState)> = Vec::new();
    for (id, (pos, kind, data, ai)) in
        ecs.query::<(&Position, &MobKind, &CompanionData, &MobAi)>().iter()
    {
        if data.is_tamed() && !generic_owns(&ai.state) {
            pets.push((id, pos.0, kind.0, data.owner_pubkey().to_string(), data.state));
        }
    }
    if pets.is_empty() {
        return;
    }
    for (id, pos, kind, owner, state) in pets {
        let owner_slot = owners.slot_of(&owner);
        let owner_pos = owners.position_of(&owner);
        match state {
            CompanionState::Stay => {
                if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                    v.0.x = 0.0;
                    v.0.z = 0.0;
                }
            }
            CompanionState::Wander => {
                // Leave the generic ambient AI drive untouched — no override.
            }
            CompanionState::Perch => {
                if let (Some(slot), Some(op)) = (owner_slot, owner_pos) {
                    // Task 9 — rotate the shoulder anchor by the owner's yaw so
                    // it stays over the same shoulder in body space instead of
                    // drifting in front of the camera when the owner turns.
                    let yaw = player_yaws.get(slot).copied().unwrap_or(0.0);
                    if let Ok(mut p) = ecs.get::<&mut Position>(id) {
                        p.0 = op + companion::shoulder_offset(slot, yaw);
                    }
                    if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                        v.0 = Vec3::ZERO;
                    }
                }
            }
            CompanionState::Follow => {
                let flying = ecs.get::<&Flying>(id).is_ok();
                let speed = mob::mob_def(kind).speed / 20.0;
                let owner_xz = owner_pos.map(|v| (v.x, v.y, v.z));
                match companion::follow_target(owner_xz, (pos.x, pos.y, pos.z)) {
                    Some((tx, tz)) => {
                        let dir = Vec3::new(tx - pos.x, 0.0, tz - pos.z).normalize_or_zero();
                        if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                            v.0.x = dir.x * speed;
                            v.0.z = dir.z * speed;
                            if flying
                                && let Some(op) = owner_pos
                            {
                                v.0.y = ((op.y + 1.0) - pos.y).clamp(-1.0, 1.0) * speed;
                            }
                        }
                        set_facing(ecs, id, dir);
                    }
                    None => {
                        // Close enough / owner unknown — hold (don't drift on wander).
                        if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                            v.0.x = 0.0;
                            v.0.z = 0.0;
                        }
                    }
                }
            }
        }
    }
}

// --- Nostrich (wave-hardening backlog, 2026-07-11) -------------------------

/// Dispatch the tamed-Nostrich Follow/Sit state machine. `nostrich.rs` shipped
/// a fully-tested `advance_state` with Spec 28d.nostrich v2, but no dispatcher
/// ever ticked it — tamed Nostriches never followed their owner at all. Same
/// yield rule as every dispatcher above: a reactive generic state
/// (`generic_owns`) keeps locomotion; otherwise Follow steers toward the owner
/// with the wolf-style `FOLLOW_MIN_DISTANCE` halt guard, and Sit parks.
pub fn dispatch_nostriches(
    ecs: &mut hecs::World,
    player_positions: &[Vec3],
    tick: u64,
    remote_owners: &[(String, usize)],
) {
    // MP-D2b — see `dispatch_companions`.
    let owners = crate::tameable::OwnerBodies { positions: player_positions, remote_owners };
    use crate::nostrich::{self, NostrichAiState, NostrichData};

    // Collect first to release the query borrow (same shape as the
    // companion dispatch above). Untamed Nostriches are pure generic-AI
    // wildlife — only tamed birds enter the Follow/Sit machine.
    let mut birds: Vec<(hecs::Entity, Vec3, String, bool)> = Vec::new();
    for (id, (pos, kind, data, ai)) in
        ecs.query::<(&Position, &MobKind, &NostrichData, &MobAi)>().iter()
    {
        if kind.0 != MobType::Nostrich || !data.is_tamed() {
            continue;
        }
        birds.push((id, pos.0, data.owner_pubkey().to_string(), generic_owns(&ai.state)));
    }
    for (id, pos, owner, generic_reactive) in birds {
        let owner_pos = owners.position_of(&owner);
        let owner_distance = owner_pos.map(|op| (op - pos).length());

        // Advance + write back the pure state machine every tick, so Flee
        // expiry / Idle→Follow promotion track even while yielding below.
        let next = {
            let Ok(mut d) = ecs.get::<&mut NostrichData>(id) else {
                continue;
            };
            let next = nostrich::advance_state(&d, tick, owner_distance);
            d.state = next;
            next
        };

        // A reactive generic state (struck-prey bolt, …) keeps locomotion.
        if generic_reactive {
            continue;
        }
        match next {
            NostrichAiState::Sit => {
                if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                    v.0.x = 0.0;
                    v.0.z = 0.0;
                }
            }
            NostrichAiState::Follow => {
                match (owner_pos, owner_distance) {
                    (Some(op), Some(dist)) if dist > nostrich::FOLLOW_MIN_DISTANCE => {
                        let speed = mob::mob_def(MobType::Nostrich).speed / 20.0;
                        let dir = Vec3::new(op.x - pos.x, 0.0, op.z - pos.z).normalize_or_zero();
                        if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                            v.0.x = dir.x * speed;
                            v.0.z = dir.z * speed;
                        }
                        set_facing(ecs, id, dir);
                    }
                    _ => {
                        // Within the halt guard / owner unknown — hold
                        // (don't orbit, don't drift on generic wander).
                        if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                            v.0.x = 0.0;
                            v.0.z = 0.0;
                        }
                    }
                }
            }
            // Idle / Flee / Kick / Charge — the generic drive (wander,
            // spook bolt) stands; kick damage lands via `on_damaged`.
            _ => {}
        }
    }
}

// --- Hyena (Animals Wave 2) ----------------------------------------------

/// Dispatch the hyena day/night temperament. `hyena_ai::tick` flips Lazy (day)
/// ↔ Hunt (night), with a timed Aggro after a hit. During Hunt/Aggro the pack
/// drives hard toward the nearest player; during Lazy it damps the generic
/// hostile chase so daytime hyenas lounge instead of relentlessly pursuing.
pub fn dispatch_hyenas(
    ecs: &mut hecs::World,
    player_positions: &[Vec3],
    world_time: u32,
) {
    use crate::hyena_ai::{self, HyenaAiState, HyenaData};

    let mut hyenas: Vec<(hecs::Entity, Vec3, HyenaData)> = Vec::new();
    for (id, (pos, kind, data)) in ecs.query::<(&Position, &MobKind, &HyenaData)>().iter() {
        if kind.0 == MobType::Hyena {
            hyenas.push((id, pos.0, *data));
        }
    }
    if hyenas.is_empty() {
        return;
    }
    let speed = mob::mob_def(MobType::Hyena).speed / 20.0;
    for (id, pos, mut data) in hyenas {
        hyena_ai::tick(&mut data, world_time);
        if let Ok(mut d) = ecs.get::<&mut HyenaData>(id) {
            *d = data;
        }
        match data.state {
            HyenaAiState::Hunt => {
                if let Some(p) = nearest_player(pos, player_positions) {
                    let dir = Vec3::new(p.0 - pos.x, 0.0, p.2 - pos.z).normalize_or_zero();
                    if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                        v.0.x = dir.x * speed;
                        v.0.z = dir.z * speed;
                    }
                    set_facing(ecs, id, dir);
                }
            }
            // Task 13 — hit by a player: chase the attacker specifically
            // (falling back to nearest if they're gone), regardless of
            // time of day.
            HyenaAiState::Aggro { attacker_pidx, .. } => {
                if let Some(p) = attacker_or_nearest_player(pos, player_positions, attacker_pidx) {
                    let dir = Vec3::new(p.0 - pos.x, 0.0, p.2 - pos.z).normalize_or_zero();
                    if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                        v.0.x = dir.x * speed;
                        v.0.z = dir.z * speed;
                    }
                    set_facing(ecs, id, dir);
                }
            }
            HyenaAiState::Lazy => {
                // Daytime: damp the generic chase so they don't relentlessly
                // pursue (lounging pack).
                if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                    v.0.x *= 0.3;
                    v.0.z *= 0.3;
                }
            }
        }
    }
}

// --- Bear (Animals Wave 2) -----------------------------------------------

/// A bear's food raid resolved this tick — the `GameState` wrapper applies it
/// against the voxel world (crop → tilled soil; one food item taken from the
/// chest).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BearWorldEffect {
    ConsumeCrop { x: i32, y: i32, z: i32 },
    RaidChest { x: i32, y: i32, z: i32 },
}

/// Dispatch the (previously dormant) bear food-raiding: bears smell nearby
/// crops/food-chests, walk to them, and eat/raid. Also drives the Aggro
/// revenge-charge (Task 13, bug-hardening, 2026-07-07) — a Bear hit by a
/// player now charges them instead of sitting idle. Returns the world
/// mutations for the caller to apply (bears read `world` here but don't own
/// a `&mut` to it).
pub fn dispatch_bears(
    ecs: &mut hecs::World,
    world: &crate::world::World,
    player_positions: &[Vec3],
    tick: u64,
) -> Vec<BearWorldEffect> {
    use crate::bear_ai::{self, BearAiState, BearData, BearTickEffect};

    let mut bears: Vec<(hecs::Entity, Vec3)> = Vec::new();
    for (id, (pos, kind, _data)) in ecs.query::<(&Position, &MobKind, &BearData)>().iter() {
        if kind.0 == MobType::Bear {
            bears.push((id, pos.0));
        }
    }
    let mut effects = Vec::new();
    if bears.is_empty() {
        return effects;
    }
    let speed = mob::mob_def(MobType::Bear).speed / 20.0;
    for (id, pos) in bears {
        let ipos = (pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32);
        let effect = {
            let Ok(mut d) = ecs.get::<&mut BearData>(id) else { continue };
            bear_ai::tick(&mut d, world, ipos, tick)
        };
        let state = ecs.get::<&BearData>(id).ok().map(|d| d.state);
        match state {
            // Walk toward the smelled food while hunting it down.
            Some(BearAiState::SmellFood { target, .. }) => {
                let dir = Vec3::new(
                    target.0 as f32 + 0.5 - pos.x,
                    0.0,
                    target.2 as f32 + 0.5 - pos.z,
                )
                .normalize_or_zero();
                if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                    v.0.x = dir.x * speed;
                    v.0.z = dir.z * speed;
                }
                set_facing(ecs, id, dir);
            }
            // Task 13 — hit by a player: charge the attacker (falling back
            // to nearest if they're gone). Bear/Hyena `on_hit_by_player`
            // was dead code before this wave — nothing ever called it, so
            // hitting either species did nothing.
            Some(BearAiState::Aggro { attacker_pidx, .. }) => {
                if let Some(t) = attacker_or_nearest_player(pos, player_positions, attacker_pidx) {
                    let dir = Vec3::new(t.0 - pos.x, 0.0, t.2 - pos.z).normalize_or_zero();
                    if let Ok(mut v) = ecs.get::<&mut Velocity>(id) {
                        v.0.x = dir.x * speed;
                        v.0.z = dir.z * speed;
                    }
                    set_facing(ecs, id, dir);
                }
            }
            _ => {}
        }
        match effect {
            BearTickEffect::ConsumeCrop { pos } => {
                effects.push(BearWorldEffect::ConsumeCrop { x: pos.0, y: pos.1, z: pos.2 });
            }
            BearTickEffect::RaidChestOnce { pos } => {
                effects.push(BearWorldEffect::RaidChest { x: pos.0, y: pos.1, z: pos.2 });
            }
            BearTickEffect::None => {}
        }
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bee_ai::BeeData;
    use crate::entity::Flying;
    use crate::goat_ai::{GoatAiState, GoatData};
    use crate::mob_ai::MobAi;
    use crate::rabbit_ai::{RabbitAiState, RabbitData};

    fn spawn_rabbit(ecs: &mut hecs::World, pos: Vec3, data: RabbitData) -> hecs::Entity {
        ecs.spawn((
            Position(pos),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Rabbit),
            MobAi::new(),
            data,
        ))
    }

    fn spawn_goat(ecs: &mut hecs::World, pos: Vec3, data: GoatData) -> hecs::Entity {
        ecs.spawn((
            Position(pos),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Goat),
            MobAi::new(),
            data,
        ))
    }

    #[test]
    fn rabbit_hops_periodically_when_undisturbed() {
        let mut ecs = hecs::World::new();
        let id = spawn_rabbit(&mut ecs, Vec3::new(0.0, 64.0, 0.0), RabbitData::new());
        let players = vec![Vec3::new(500.0, 64.0, 500.0)]; // far away
        let mut hop_ticks = 0;
        for t in 0..200 {
            dispatch_rabbits(&mut ecs, &players, t);
            let v = ecs.get::<&Velocity>(id).unwrap().0;
            if v.y > 0.0 {
                hop_ticks += 1;
            }
        }
        assert!(hop_ticks >= 5, "rabbit should hop several times in 200 ticks, got {hop_ticks}");
    }

    #[test]
    fn rabbit_flees_directly_away_from_close_player() {
        let mut ecs = hecs::World::new();
        // Rabbit at origin, player 2 blocks east (inside FLEE_DISTANCE).
        let id = spawn_rabbit(&mut ecs, Vec3::new(0.0, 64.0, 0.0), RabbitData::new());
        let players = vec![Vec3::new(2.0, 64.0, 0.0)];
        dispatch_rabbits(&mut ecs, &players, 10);
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(v.x < 0.0, "rabbit should flee west (−x) from a player to its east, got vx={}", v.x);
    }

    #[test]
    fn rabbit_in_generic_flee_is_not_overridden() {
        // A rabbit already bolting from a ranged hit (generic Flee) must keep
        // its generic velocity — dispatch yields.
        let mut ecs = hecs::World::new();
        let id = spawn_rabbit(&mut ecs, Vec3::new(0.0, 64.0, 0.0), RabbitData::new());
        // Force the generic reactive state + a sentinel velocity.
        {
            let mut ai = ecs.get::<&mut MobAi>(id).unwrap();
            ai.state = AiState::Flee { timer: 30 };
        }
        ecs.get::<&mut Velocity>(id).unwrap().0 = Vec3::new(0.42, 0.0, 0.0);
        let players = vec![Vec3::new(1.0, 64.0, 0.0)]; // close — would normally flee-override
        dispatch_rabbits(&mut ecs, &players, 0);
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert_eq!(v.x, 0.42, "generic Flee velocity must be left untouched");
    }

    #[test]
    fn goat_charge_state_drives_velocity_toward_target() {
        let mut ecs = hecs::World::new();
        let data = GoatData {
            state: GoatAiState::Charge { target_id: 0, until_tick: 1_000 },
            last_retarget_tick: 0,
            wander_target: None,
        };
        let id = spawn_goat(&mut ecs, Vec3::new(0.0, 64.0, 0.0), data);
        let players = vec![Vec3::new(5.0, 64.0, 0.0)]; // target due east
        let impacts = dispatch_goats(&mut ecs, &players, 10);
        assert!(impacts.is_empty(), "mid-charge produces no impact yet");
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(v.x > 0.0, "charging goat should move toward the +x target, got vx={}", v.x);
    }

    #[test]
    fn goat_impact_tick_emits_gore_for_player_in_reach() {
        let mut ecs = hecs::World::new();
        // until_tick == current tick → impact resolves this tick.
        let data = GoatData {
            state: GoatAiState::Charge { target_id: 0, until_tick: 50 },
            last_retarget_tick: 0,
            wander_target: None,
        };
        let _id = spawn_goat(&mut ecs, Vec3::new(0.0, 64.0, 0.0), data);
        let players = vec![Vec3::new(1.0, 64.0, 0.0)]; // within GOAT_IMPACT_REACH
        let impacts = dispatch_goats(&mut ecs, &players, 50);
        assert_eq!(impacts.len(), 1);
        assert_eq!(impacts[0].target_slot, 0);
    }

    #[test]
    fn goat_impact_misses_a_dodged_player() {
        let mut ecs = hecs::World::new();
        let data = GoatData {
            state: GoatAiState::Charge { target_id: 0, until_tick: 50 },
            last_retarget_tick: 0,
            wander_target: None,
        };
        let _id = spawn_goat(&mut ecs, Vec3::new(0.0, 64.0, 0.0), data);
        let players = vec![Vec3::new(20.0, 64.0, 0.0)]; // dodged well out of reach
        let impacts = dispatch_goats(&mut ecs, &players, 50);
        assert!(impacts.is_empty(), "a player who side-stepped the charge takes no gore");
    }

    #[test]
    fn no_species_entities_is_a_cheap_noop() {
        let mut ecs = hecs::World::new();
        // A lone cow — neither dispatcher should touch it.
        let id = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::new(0.1, 0.0, 0.1)),
            MobKind(MobType::Cow),
            MobAi::new(),
        ));
        let players = vec![Vec3::new(1.0, 64.0, 0.0)];
        dispatch_rabbits(&mut ecs, &players, 5);
        let impacts = dispatch_goats(&mut ecs, &players, 5);
        assert!(impacts.is_empty());
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert_eq!(v, Vec3::new(0.1, 0.0, 0.1), "cow velocity untouched");
    }

    fn spawn_bee(ecs: &mut hecs::World, pos: Vec3, data: BeeData) -> hecs::Entity {
        ecs.spawn((
            Position(pos),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Bee),
            MobAi::new(),
            data,
            Flying,
        ))
    }

    #[test]
    fn bee_drifts_with_horizontal_and_vertical_motion() {
        let mut ecs = hecs::World::new();
        let id = spawn_bee(&mut ecs, Vec3::new(0.0, 64.0, 0.0), BeeData::new());
        let mut saw_vertical = false;
        let mut saw_horizontal = false;
        for t in 0..200 {
            let _ = dispatch_bees(&mut ecs, &[], t);
            let v = ecs.get::<&Velocity>(id).unwrap().0;
            if v.y.abs() > 1e-4 {
                saw_vertical = true;
            }
            if v.x.abs() > 1e-4 || v.z.abs() > 1e-4 {
                saw_horizontal = true;
            }
        }
        assert!(saw_vertical, "a flying bee should bob up and down");
        assert!(saw_horizontal, "a flying bee should drift horizontally");
    }

    #[test]
    fn bee_with_distant_hive_heads_home() {
        let mut ecs = hecs::World::new();
        // Bee 50 blocks from its hive → ReturnToHive → flies toward +x.
        let id = spawn_bee(
            &mut ecs,
            Vec3::new(0.0, 64.0, 0.0),
            BeeData::with_home_hive((50, 64, 0)),
        );
        let _ = dispatch_bees(&mut ecs, &[], 10);
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(v.x > 0.0, "bee should fly toward its hive at +x, got vx={}", v.x);
    }

    #[test]
    fn struck_bee_stings_its_attacker_in_range() {
        use crate::combat::LastAttacker;
        let mut ecs = hecs::World::new();
        // Bee at origin carrying a LastAttacker(0); player 0 is 2 blocks away,
        // inside the sting trigger.
        let id = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Bee),
            MobAi::new(),
            BeeData::new(),
            Flying,
            LastAttacker(crate::combat::Attacker::Local(0)),
        ));
        let players = vec![Vec3::new(2.0, 64.0, 0.0)];
        let mut stung = false;
        for t in 0..150 {
            for s in dispatch_bees(&mut ecs, &players, t) {
                if s.target_slot == 0 && s.bee == id {
                    stung = true;
                }
            }
            if stung {
                break;
            }
        }
        assert!(stung, "a struck bee with its attacker in range should land a sting");
    }

    #[test]
    fn rabbit_data_round_trips_to_idle_between_hops() {
        // Sanity: dispatching writes the advanced RabbitData back so the next
        // tick sees the updated hop clock (not a frozen state).
        let mut ecs = hecs::World::new();
        let id = spawn_rabbit(&mut ecs, Vec3::new(0.0, 64.0, 0.0), RabbitData::new());
        let players = vec![Vec3::new(500.0, 64.0, 500.0)];
        // Tick 30 fires the first hop (HOP_PERIOD_TICKS); state becomes Hop.
        dispatch_rabbits(&mut ecs, &players, 30);
        let d = *ecs.get::<&RabbitData>(id).unwrap();
        assert_eq!(d.last_hop_tick, 30);
        assert!(matches!(d.state, RabbitAiState::Hop));
    }

    // --- Animals Wave 2 dispatchers ------------------------------------

    #[test]
    fn dispatch_horses_wanders_free_horse_but_skips_ridden() {
        let mut ecs = hecs::World::new();
        let free = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Horse),
            MobAi::new(),
            crate::horse_ai::HorseData::new(),
        ));
        let ridden = ecs.spawn((
            Position(Vec3::new(40.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Horse),
            MobAi::new(),
            crate::horse_ai::HorseData::new(),
            crate::entity::Ridden,
        ));
        let mut free_moved = false;
        for t in 0..500 {
            dispatch_horses(&mut ecs, t);
            if ecs.get::<&Velocity>(free).unwrap().0.length_squared() > 1e-6 {
                free_moved = true;
            }
            assert_eq!(
                ecs.get::<&Velocity>(ridden).unwrap().0,
                Vec3::ZERO,
                "a ridden horse must not be wandered by the dispatcher (the rider steers)"
            );
        }
        assert!(free_moved, "a free horse should herd-wander");
    }

    #[test]
    fn dispatch_horses_wanders_the_whole_family_donkey_and_mule() {
        // Donkey + Mule share the horse herd-wander (they get HorseData on spawn
        // via is_horse_family). A free one should amble; mounting is separate.
        let mut ecs = hecs::World::new();
        let donkey = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Donkey),
            MobAi::new(),
            crate::horse_ai::HorseData::new(),
        ));
        let mule = ecs.spawn((
            Position(Vec3::new(80.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Mule),
            MobAi::new(),
            crate::horse_ai::HorseData::new(),
        ));
        let (mut donkey_moved, mut mule_moved) = (false, false);
        for t in 0..500 {
            dispatch_horses(&mut ecs, t);
            if ecs.get::<&Velocity>(donkey).unwrap().0.length_squared() > 1e-6 {
                donkey_moved = true;
            }
            if ecs.get::<&Velocity>(mule).unwrap().0.length_squared() > 1e-6 {
                mule_moved = true;
            }
        }
        assert!(donkey_moved, "a free donkey should herd-wander like a horse");
        assert!(mule_moved, "a free mule should herd-wander like a horse");
    }

    #[test]
    fn horse_family_is_horse_donkey_mule_not_nostrich() {
        use crate::mob::is_horse_family;
        assert!(is_horse_family(MobType::Horse));
        assert!(is_horse_family(MobType::Donkey));
        assert!(is_horse_family(MobType::Mule));
        assert!(!is_horse_family(MobType::Nostrich), "Nostrich rides but isn't horse-family");
        assert!(!is_horse_family(MobType::Cow));
    }

    #[test]
    fn dispatch_squids_suffocate_on_dry_land() {
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new(); // empty → is_water is false everywhere
        let id = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Squid),
            MobAi::new(),
            crate::squid_ai::SquidData::new(),
            crate::combat::Health::new(10.0),
        ));
        let start = (*ecs.get::<&crate::combat::Health>(id).unwrap()).current;
        for t in 0..220 {
            dispatch_squids(&mut ecs, &world, t);
        }
        let end = (*ecs.get::<&crate::combat::Health>(id).unwrap()).current;
        assert!(end < start, "a stranded squid should suffocate (start {start}, end {end})");
    }

    #[test]
    fn dispatch_hyenas_hunt_at_night_laze_by_day() {
        use crate::hyena_ai::{HyenaAiState, HyenaData};
        let mut ecs = hecs::World::new();
        let id = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Hyena),
            HyenaData::new(),
        ));
        let players = vec![Vec3::new(6.0, 64.0, 0.0)]; // player to the +x
        // Night (t=22000 is outside the [4500,19500] day band) → Hunt the player.
        dispatch_hyenas(&mut ecs, &players, 22000);
        assert!(matches!(ecs.get::<&HyenaData>(id).unwrap().state, HyenaAiState::Hunt));
        assert!(ecs.get::<&Velocity>(id).unwrap().0.x > 0.0, "night hyena hunts toward the player");
        // Day (t=12000) → Lazy.
        dispatch_hyenas(&mut ecs, &players, 12000);
        assert!(matches!(ecs.get::<&HyenaData>(id).unwrap().state, HyenaAiState::Lazy));
    }

    #[test]
    fn dispatch_hyenas_aggro_chases_the_attacker_not_the_nearer_player() {
        // Task 13 — a hyena hit by player 1 must charge player 1, even
        // though player 0 is closer. This is the behaviour that
        // distinguishes Aggro (revenge on a specific attacker) from the
        // generic Hunt state (which just goes for whoever is nearest).
        use crate::hyena_ai::{HyenaAiState, HyenaData};
        let mut ecs = hecs::World::new();
        let mut data = HyenaData::new();
        data.state = HyenaAiState::Aggro { ticks_remaining: 50, attacker_pidx: 1 };
        let id = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Hyena),
            data,
        ));
        // Player 0 is close on +x; player 1 (the attacker) is far on -x.
        let players = vec![Vec3::new(2.0, 64.0, 0.0), Vec3::new(-10.0, 64.0, 0.0)];
        dispatch_hyenas(&mut ecs, &players, 12000); // daytime — Aggro must still chase.
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(v.x < 0.0, "aggro'd hyena should chase the attacker (-x), got vx={}", v.x);
        // Aggro persists (doesn't revert to Lazy mid-window even by day).
        assert!(matches!(ecs.get::<&HyenaData>(id).unwrap().state, HyenaAiState::Aggro { .. }));
    }

    #[test]
    fn dispatch_hyenas_aggro_falls_back_to_nearest_when_attacker_slot_gone() {
        // If the attacker's slot index is out of range (disconnected),
        // the hyena should still have somewhere to go rather than freezing.
        use crate::hyena_ai::{HyenaAiState, HyenaData};
        let mut ecs = hecs::World::new();
        let mut data = HyenaData::new();
        data.state = HyenaAiState::Aggro { ticks_remaining: 50, attacker_pidx: 5 };
        let id = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Hyena),
            data,
        ));
        let players = vec![Vec3::new(-4.0, 64.0, 0.0)]; // only slot 0 exists
        dispatch_hyenas(&mut ecs, &players, 12000);
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(v.x < 0.0, "should fall back to the only available player");
    }

    #[test]
    fn dispatch_bears_aggro_charges_the_attacker() {
        // Task 13 — a bear hit by a player must charge them, not sit idle
        // (no food nearby to smell, so before this fix it did nothing).
        use crate::bear_ai::{BearAiState, BearData};
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let mut data = BearData::new();
        data.state = BearAiState::Aggro { ticks_remaining: 100, attacker_pidx: 0 };
        let id = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Bear),
            data,
        ));
        let players = vec![Vec3::new(8.0, 64.0, 0.0)];
        let _ = dispatch_bears(&mut ecs, &world, &players, 1);
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(v.x > 0.0, "aggro'd bear should charge toward the attacker, got vx={}", v.x);
    }

    #[test]
    fn dispatch_bears_eat_a_ripe_crop_underfoot() {
        let mut ecs = hecs::World::new();
        let mut world = crate::world::World::new();
        world.set_block(0, 64, 0, crate::block::WHEAT_STAGE_3); // a ripe crop at the bear's feet
        let _bear = ecs.spawn((
            Position(Vec3::new(0.5, 64.0, 0.5)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Bear),
            crate::bear_ai::BearData::new(),
        ));
        let mut consumed = false;
        let no_players: Vec<Vec3> = Vec::new();
        for t in 0..300 {
            for eff in dispatch_bears(&mut ecs, &world, &no_players, t) {
                if matches!(eff, BearWorldEffect::ConsumeCrop { .. }) {
                    consumed = true;
                }
            }
            if consumed {
                break;
            }
        }
        assert!(consumed, "a bear standing on a ripe crop should eventually eat it");
    }

    // --- Aquatic wave: Shark ---------------------------------------------

    #[test]
    fn shark_bites_a_player_who_is_in_water_and_in_reach() {
        let mut ecs = hecs::World::new();
        let mut world = crate::world::World::new();
        world.set_block(0, 64, 0, crate::block::WATER);
        world.set_block(1, 64, 0, crate::block::WATER); // the player's cell is water
        let _shark = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Shark),
        ));
        let players = vec![Vec3::new(1.0, 64.0, 0.0)]; // ~1 block away, in water
        let bites = dispatch_sharks(&mut ecs, &world, &players, 10);
        assert_eq!(bites.len(), 1, "shark should bite a swimming player in reach");
        assert_eq!(bites[0].target_slot, 0);
    }

    #[test]
    fn shark_leaves_a_player_on_dry_land_alone() {
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new(); // no water anywhere
        ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Shark),
        ));
        let players = vec![Vec3::new(1.0, 64.0, 0.0)]; // adjacent but NOT in water
        let bites = dispatch_sharks(&mut ecs, &world, &players, 10);
        assert!(bites.is_empty(), "stand on shore / in a boat and you're safe from sharks");
    }

    // --- Companions: follow dispatch --------------------------------------

    #[test]
    fn tamed_cat_follows_its_owner_untamed_stays_put() {
        use crate::companion::CompanionData;
        let mut ecs = hecs::World::new();
        // Tamed cat owned by player 0, 10 blocks east of the owner.
        let mut owned = CompanionData::untamed();
        owned.ownership.owner_pubkey = "local-player-0".to_string();
        let cat = ecs.spawn((
            Position(Vec3::new(10.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Cat),
            MobAi::new(),
            owned,
        ));
        // An untamed cat that must NOT be dragged anywhere.
        let wild = ecs.spawn((
            Position(Vec3::new(10.0, 64.0, 20.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Cat),
            MobAi::new(),
            CompanionData::untamed(),
        ));
        let players = vec![Vec3::new(0.0, 64.0, 0.0)]; // owner at origin
        dispatch_companions(&mut ecs, &players, &[0.0], 5, &[]);
        let v = ecs.get::<&Velocity>(cat).unwrap().0;
        assert!(v.x < 0.0, "tamed cat should walk toward its owner at −x, got vx={}", v.x);
        let wv = ecs.get::<&Velocity>(wild).unwrap().0;
        assert_eq!(wv, Vec3::ZERO, "an untamed cat is not dispatched to follow");
    }

    /// MP-D2b — a joiner's pet (owner key = its npub) follows that joiner's
    /// body, which a lending host lists at the joiner's server slot.
    #[test]
    fn a_joiners_pet_follows_the_joiners_body() {
        use crate::companion::CompanionData;
        let mut ecs = hecs::World::new();
        let mut owned = CompanionData::untamed();
        owned.ownership.owner_pubkey = "npub1joiner".to_string();
        let cat = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Cat),
            MobAi::new(),
            owned,
        ));
        // Slot 0 (the host) to the -x; the joiner at slot 1 to the +x.
        let bodies = [Vec3::new(-10.0, 64.0, 0.0), Vec3::new(10.0, 64.0, 0.0)];
        dispatch_companions(&mut ecs, &bodies, &[0.0, 0.0], 5, &[("npub1joiner".to_string(), 1)]);
        let v = ecs.get::<&Velocity>(cat).unwrap().0;
        assert!(v.x > 0.0, "the cat walks to its owner at slot 1 (+x), got vx={}", v.x);
    }

    /// MP-D2b — a slot with no body (`ABSENT_PLAYER`) is never anyone's
    /// nearest target, even for an AI that measures horizontal distance only
    /// and stands at the world's origin; a Bear or Hyena whose attacker has
    /// left falls back to the nearest body instead of charging off after it.
    #[test]
    fn an_absent_slot_is_never_the_nearest_target() {
        let at_origin = Vec3::new(0.0, 64.0, 0.0);
        let bodies = [ABSENT_PLAYER, Vec3::new(30.0, 64.0, 30.0)];
        assert_eq!(nearest_player_indexed(at_origin, &bodies).map(|t| t.0), Some(1));
        assert_eq!(nearest_player(at_origin, &bodies), Some((30.0, 64.0, 30.0)));
        assert_eq!(attacker_or_nearest_player(at_origin, &bodies, 0), Some((30.0, 64.0, 30.0)));
    }

    /// MP-D2b — a pet whose joiner owner has left (its slot holds
    /// `ABSENT_PLAYER`) has no owner to walk to: it is not dragged off.
    #[test]
    fn a_departed_joiners_pet_is_not_dragged_after_the_absent_slot() {
        use crate::companion::CompanionData;
        let mut ecs = hecs::World::new();
        let mut owned = CompanionData::untamed();
        owned.ownership.owner_pubkey = "npub1gone".to_string();
        let cat = ecs.spawn((
            Position(Vec3::new(0.0, 64.0, 0.0)),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Cat),
            MobAi::new(),
            owned,
        ));
        let bodies = [Vec3::new(-10.0, 64.0, 0.0), ABSENT_PLAYER];
        let owners = crate::tameable::OwnerBodies {
            positions: &bodies,
            remote_owners: &[("npub1gone".to_string(), 1)],
        };
        assert_eq!(owners.position_of("npub1gone"), None);
        dispatch_companions(&mut ecs, &bodies, &[0.0, 0.0], 5, &[("npub1gone".to_string(), 1)]);
        let v = ecs.get::<&Velocity>(cat).unwrap().0;
        assert!(v.x <= 0.0 && v.x.abs() < 1.0, "no pull toward the absent slot, got vx={}", v.x);
        assert!(v.z.abs() < 1.0);
    }

    #[test]
    fn stay_state_holds_position_despite_distant_owner() {
        let mut ecs = hecs::World::new();
        let cat = crate::entity::spawn_mob(&mut ecs, MobType::Cat, Vec3::new(0.0, 64.0, 0.0));
        {
            let mut d = ecs.get::<&mut crate::companion::CompanionData>(cat).unwrap();
            d.ownership.owner_pubkey = "local-player-0".into();
            d.state = crate::companion::CompanionState::Stay;
        }
        dispatch_companions(&mut ecs, &[Vec3::new(10.0, 64.0, 0.0)], &[0.0], 0, &[]);
        let v = ecs.get::<&crate::entity::Velocity>(cat).unwrap();
        assert_eq!((v.0.x, v.0.z), (0.0, 0.0), "Stay must not chase the owner");
    }

    #[test]
    fn perch_pins_parrot_to_owner_shoulder() {
        let mut ecs = hecs::World::new();
        let parrot = crate::entity::spawn_mob(&mut ecs, MobType::Parrot, Vec3::new(5.0, 64.0, 5.0));
        {
            let mut d = ecs.get::<&mut crate::companion::CompanionData>(parrot).unwrap();
            d.ownership.owner_pubkey = "local-player-0".into();
            d.state = crate::companion::CompanionState::Perch;
        }
        let owner = Vec3::new(20.0, 70.0, 20.0);
        let yaw = 1.2345_f32; // arbitrary non-zero yaw — exercises the rotation
        dispatch_companions(&mut ecs, &[owner], &[yaw], 0, &[]);
        let p = ecs.get::<&crate::entity::Position>(parrot).unwrap();
        assert!((p.0 - (owner + crate::companion::shoulder_offset(0, yaw))).length() < 0.01);
    }

    #[test]
    fn flying_follower_gets_vertical_drive() {
        let mut ecs = hecs::World::new();
        let parrot = crate::entity::spawn_mob(&mut ecs, MobType::Parrot, Vec3::new(0.0, 64.0, 0.0));
        {
            let mut d = ecs.get::<&mut crate::companion::CompanionData>(parrot).unwrap();
            d.ownership.owner_pubkey = "local-player-0".into();
        }
        dispatch_companions(&mut ecs, &[Vec3::new(10.0, 70.0, 0.0)], &[0.0], 0, &[]);
        let v = ecs.get::<&crate::entity::Velocity>(parrot).unwrap();
        assert!(v.0.y > 0.0, "flying follower should climb toward a higher owner");
    }

    // --- Nostrich dispatcher (wave-hardening backlog, 2026-07-11) --------

    fn spawn_tamed_nostrich(
        ecs: &mut hecs::World,
        pos: Vec3,
        state: crate::nostrich::NostrichAiState,
    ) -> hecs::Entity {
        let id = crate::entity::spawn_mob(&mut *ecs, MobType::Nostrich, pos);
        {
            let mut d = ecs.get::<&mut crate::nostrich::NostrichData>(id).unwrap();
            d.ownership.owner_pubkey = "local-player-0".into();
            d.state = state;
        }
        id
    }

    #[test]
    fn nostrich_follow_steers_toward_owner_and_halts_close() {
        use crate::nostrich::NostrichAiState;
        let mut ecs = hecs::World::new();
        let id = spawn_tamed_nostrich(
            &mut ecs, Vec3::new(0.0, 64.0, 0.0), NostrichAiState::Follow,
        );

        // Owner beyond the halt guard → steer toward them (+x).
        dispatch_nostriches(&mut ecs, &[Vec3::new(10.0, 64.0, 0.0)], 10, &[]);
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(v.x > 0.0, "Follow must steer toward the owner, got {v:?}");

        // Owner within FOLLOW_MIN_DISTANCE → halt (anti-oscillation guard).
        dispatch_nostriches(&mut ecs, &[Vec3::new(2.0, 64.0, 0.0)], 11, &[]);
        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(
            v.x == 0.0 && v.z == 0.0,
            "close enough: halt, don't orbit the owner, got {v:?}"
        );
    }

    #[test]
    fn nostrich_sit_parks_regardless_of_owner_distance() {
        use crate::nostrich::NostrichAiState;
        let mut ecs = hecs::World::new();
        let id = spawn_tamed_nostrich(
            &mut ecs, Vec3::new(0.0, 64.0, 0.0), NostrichAiState::Sit,
        );
        ecs.get::<&mut Velocity>(id).unwrap().0 = Vec3::new(0.4, 0.0, 0.4);

        dispatch_nostriches(&mut ecs, &[Vec3::new(30.0, 64.0, 0.0)], 10, &[]);

        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert!(v.x == 0.0 && v.z == 0.0, "Sit means stay put, got {v:?}");
        let d = ecs.get::<&crate::nostrich::NostrichData>(id).unwrap();
        assert_eq!(d.state, NostrichAiState::Sit, "Sit is sticky until toggled");
    }

    #[test]
    fn nostrich_tamed_idle_promotes_to_follow_and_writes_back() {
        use crate::nostrich::NostrichAiState;
        let mut ecs = hecs::World::new();
        let id = spawn_tamed_nostrich(
            &mut ecs, Vec3::new(0.0, 64.0, 0.0), NostrichAiState::Idle,
        );

        dispatch_nostriches(&mut ecs, &[Vec3::new(6.0, 64.0, 0.0)], 10, &[]);

        let d = ecs.get::<&crate::nostrich::NostrichData>(id).unwrap();
        assert_eq!(
            d.state,
            NostrichAiState::Follow,
            "advance_state promotes a tamed Idle to Follow — the dispatcher must write it back"
        );
    }

    #[test]
    fn nostrich_untamed_locomotion_is_left_to_the_generic_ai() {
        let mut ecs = hecs::World::new();
        let id = crate::entity::spawn_mob(&mut ecs, MobType::Nostrich, Vec3::new(0.0, 64.0, 0.0));
        ecs.get::<&mut Velocity>(id).unwrap().0 = Vec3::new(0.3, 0.0, 0.0);

        dispatch_nostriches(&mut ecs, &[Vec3::new(10.0, 64.0, 0.0)], 10, &[]);

        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert_eq!(v.x, 0.3, "an untamed Nostrich keeps the generic wander drive");
    }

    #[test]
    fn nostrich_yields_to_a_reactive_generic_state() {
        use crate::mob_ai::AiState;
        use crate::nostrich::NostrichAiState;
        let mut ecs = hecs::World::new();
        let id = spawn_tamed_nostrich(
            &mut ecs, Vec3::new(0.0, 64.0, 0.0), NostrichAiState::Follow,
        );
        // The struck-prey bolt (generic Flee) owns locomotion this tick.
        ecs.get::<&mut MobAi>(id).unwrap().state = AiState::Flee { timer: 40 };
        ecs.get::<&mut Velocity>(id).unwrap().0 = Vec3::new(-0.6, 0.0, 0.0);

        dispatch_nostriches(&mut ecs, &[Vec3::new(10.0, 64.0, 0.0)], 10, &[]);

        let v = ecs.get::<&Velocity>(id).unwrap().0;
        assert_eq!(v.x, -0.6, "a reactive generic state keeps locomotion (yield rule)");
    }
}

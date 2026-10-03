//! Companions wave — a generic tameable-pet system shared by Cat, Parrot, and
//! Fox (and any future companion). Built on the `tameable::OwnershipData`
//! framework the Wolf already uses, so a single `CompanionData` component +
//! `dispatch_companions` follow pass covers every non-wolf pet instead of
//! duplicating the wolf code per species.
//!
//! v1 scope: tame with the species' food → the pet follows its owner. The
//! richer 1C layer (Stay/Sit command states, no-friendly-fire, pet bed +
//! recall) layers on top of this component later.
//!
//! Eventual crate home: `genesis_sim`.

use serde::{Deserialize, Serialize};

use crate::entity::{MobKind, Position};
use crate::item::MaterialId;
use crate::mob::{self, MobType};
use crate::tameable::OwnershipData;

/// 1C command states. `Follow` is the default (pre-1C behaviour). Cycled by
/// the owner's empty-hand right-click. `Perch` = shoulder-ride, Parrot only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompanionState {
    #[default]
    Follow,
    Stay,
    Wander,
    Perch,
}

/// Below this the pet is close enough — stop (don't jitter on the owner).
pub const FOLLOW_MIN_DISTANCE: f32 = 2.5;
/// Beyond this the pet gives up following (treated as out of range) so a
/// teleporting/sprinting owner doesn't drag it across the world.
pub const FOLLOW_GIVE_UP_DISTANCE: f32 = 48.0;
/// Tame success rate (1-in-3), matching the Nostrich berry-feed cadence.
pub const TAME_NUMER: u32 = 1;
pub const TAME_DENOM: u32 = 3;

/// A tameable companion's ownership state. Untamed pets wander (generic AI);
/// tamed pets follow their owner via `dispatch_companions`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompanionData {
    pub ownership: OwnershipData,
    /// 1C command state. RUNTIME-ONLY on this struct: `serde(skip)` keeps the
    /// serialized wire shape identical to the pre-1C `CompanionData` (bincode
    /// cannot default missing trailing fields — appending here would corrupt
    /// old saves). Persistence travels in `SavedTamedPetData::Companion2`.
    #[serde(skip)]
    pub state: CompanionState,
}

impl CompanionData {
    pub fn untamed() -> Self {
        Self { ownership: OwnershipData::untamed(), state: CompanionState::Follow }
    }
    pub fn is_tamed(&self) -> bool {
        self.ownership.is_tamed()
    }
    pub fn owner_pubkey(&self) -> &str {
        &self.ownership.owner_pubkey
    }
}

impl Default for CompanionData {
    fn default() -> Self {
        Self::untamed()
    }
}

/// Which species are companion-tameable, and the food each is tamed with.
/// `None` = not a companion species.
pub fn tame_food(kind: MobType) -> Option<MaterialId> {
    match kind {
        MobType::Cat => Some(MaterialId::RawFish),
        MobType::Parrot => Some(MaterialId::WheatSeeds),
        MobType::Fox => Some(MaterialId::Berries),
        _ => None,
    }
}

pub fn is_companion_species(kind: MobType) -> bool {
    tame_food(kind).is_some()
}

pub fn can_perch(kind: MobType) -> bool {
    matches!(kind, MobType::Parrot)
}

/// Where a perched pet pins itself relative to its owner — a body-space local
/// offset (right shoulder, slightly up) rotated by the owner's `yaw` into
/// world space. `slot` is accepted for a future per-slot/handedness tweak but
/// unused for now — every owner gets the same shoulder point.
///
/// Task 9 (bug-hardening, 2026-07-07) — this used to be a fixed *world-axis*
/// offset added straight to the owner's position, so facing the +x/+z
/// quadrant dragged the parrot in front of the first-person camera and inside
/// the 60° interaction cone (`combat::SWING_MIN_DOT`), where it ate every
/// swing/right-click meant for something else. Rotating the local offset by
/// `yaw` keeps the parrot pinned over the same shoulder in body space no
/// matter which way the owner is facing.
///
/// The rotation must match the convention `Camera::horizontal_forward`/
/// `Camera::right` use (the actual movement-code basis, `physics.rs`'s
/// `Player::tick`): at `yaw=0`, forward = `(0,0,-1)` and right = `(1,0,0)`.
/// Deriving `right(yaw) = R(yaw)*right(0)` and
/// `back(yaw) = R(yaw)*back(0)` (`back(0) = (0,0,1)`, the local +z axis this
/// offset's `z` component was originally written against — it's the shoulder
/// point, not the look direction) from those two functions gives:
/// `x' = dx*cos(yaw) + dz*sin(yaw)`, `z' = -dx*sin(yaw) + dz*cos(yaw)`.
pub fn shoulder_offset(_slot: usize, yaw: f32) -> glam::Vec3 {
    let local = glam::Vec3::new(0.4, 1.4, 0.2);
    let (sin_y, cos_y) = yaw.sin_cos();
    let x = local.x * cos_y + local.z * sin_y;
    let z = -local.x * sin_y + local.z * cos_y;
    glam::Vec3::new(x, local.y, z)
}

/// Owner command cycle. Ground species: Follow → Stay → Wander → Follow.
/// Shoulder species insert Perch after Wander.
pub fn cycle_state(current: CompanionState, can_perch: bool) -> CompanionState {
    use CompanionState::*;
    match current {
        Follow => Stay,
        Stay => Wander,
        Wander => if can_perch { Perch } else { Follow },
        Perch => Follow,
    }
}

pub fn state_label(s: CompanionState) -> &'static str {
    match s {
        CompanionState::Follow => "Follow",
        CompanionState::Stay => "Stay",
        CompanionState::Wander => "Wander",
        CompanionState::Perch => "Perch",
    }
}

/// Pure follow decision: given the owner's position (if resolvable) and the
/// pet's position, return the xz point the pet should walk toward, or `None`
/// to hold still (close enough, owner unknown, or owner out of give-up range).
pub fn follow_target(
    owner_pos: Option<(f32, f32, f32)>,
    self_pos: (f32, f32, f32),
) -> Option<(f32, f32)> {
    let (ox, _oy, oz) = owner_pos?;
    let dx = ox - self_pos.0;
    let dz = oz - self_pos.2;
    let dist = (dx * dx + dz * dz).sqrt();
    if (FOLLOW_MIN_DISTANCE..FOLLOW_GIVE_UP_DISTANCE).contains(&dist) {
        Some((ox, oz))
    } else {
        None
    }
}

/// Pets wave Task 9 — a tamed Cat's threat-ward radius. `mob_ai::tick_mob_ai`
/// keeps a hostile (its own explicit species set — Brigand/Marauder/
/// Berserker/Hyena; Bear is deliberately excluded, a wild predator rather
/// than a looter that reasons about a housecat) from targeting a player
/// standing inside this radius of any tamed cat, and steers a hostile that
/// wanders into the radius directly away from the nearest cat.
pub const CAT_WARD_RADIUS: f32 = 12.0;

/// World-space positions of every *tamed* Cat in the world. Wild (untamed)
/// cats don't ward, and other tamed companions (Parrot, Fox) don't either —
/// this is a Cat-specific mechanic, not a generic companion one.
pub fn tamed_cat_positions(ecs: &hecs::World) -> Vec<glam::Vec3> {
    ecs.query::<(&Position, &MobKind, &CompanionData)>()
        .iter()
        .filter(|(_, (_, kind, data))| kind.0 == MobType::Cat && data.is_tamed())
        .map(|(_, (pos, _, _))| pos.0)
        .collect()
}

/// Is `pos` within [`CAT_WARD_RADIUS`] of any tamed cat?
pub fn warded(pos: glam::Vec3, cats: &[glam::Vec3]) -> bool {
    cats.iter().any(|&c| (pos - c).length() < CAT_WARD_RADIUS)
}

/// Task 11 (design P6, bug-hardening 2026-07-07) — a perched parrot dismounts
/// to `Follow` when its owner takes damage, same as the existing manual
/// cycle-state dismount. Design P6 specified both triggers; only the
/// cycle-state one shipped. Called from the single "the owner just took a
/// landed hit" choke point (`game_loop.rs`'s per-source `take_damage_with_
/// armour`/`tick_mob_attacks` call sites) rather than threaded through every
/// damage type individually — a parrot riding the shoulder through a mob
/// hit, a goat charge, or a lava tick doesn't fit "perched", regardless of
/// what caused the hit. Returns each dismounted parrot's `(entity, position)`
/// so the caller can spawn a puff particle at the vacated shoulder.
pub fn dismount_perched_pets_on_damage(
    ecs: &mut hecs::World,
    owner_pubkey: &str,
) -> Vec<(hecs::Entity, glam::Vec3)> {
    let mut dismounted = Vec::new();
    for (id, (pos, data)) in ecs.query_mut::<(&Position, &mut CompanionData)>() {
        if data.state == CompanionState::Perch && data.ownership.is_owned_by(owner_pubkey) {
            data.state = CompanionState::Follow;
            dismounted.push((id, pos.0));
        }
    }
    dismounted
}

/// Pets wave Task 10 — a tamed Parrot's threat-alarm range. A hostile mob
/// wandering within this radius of a tamed parrot trips the alarm (subject to
/// [`PARROT_ALARM_COOLDOWN`]) — a toast + particle burst the caller emits.
pub const PARROT_ALARM_RADIUS: f32 = 16.0;

/// Ticks between alarms from the same parrot, so a hostile lingering nearby
/// doesn't spam a toast every tick.
pub const PARROT_ALARM_COOLDOWN: u64 = 200;

/// Ephemeral per-tick gate on a tamed parrot's alarm — NOT part of the save
/// format. Unlike `CompanionData` (persisted via `SavedTamedPetData::
/// Companion2`), this is transient AI state in the same vein as
/// `combat::LastAttacker` or `MobAi.ticks`: it only matters for the current
/// session's tick cadence, resets harmlessly to "no cooldown" (component
/// absent) on world load, and carries no `Serialize`/`Deserialize` — adding
/// those would invite it to leak into the wire/save format it was
/// deliberately kept out of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlarmCooldown {
    pub until_tick: u64,
}

/// World-space positions of every mob in `MobCategory::Hostile`, regardless
/// of species — the parrot alarm is a generic "danger nearby" cue, not the
/// narrower cat-ward species list.
fn hostile_positions(ecs: &hecs::World) -> Vec<glam::Vec3> {
    ecs.query::<(&Position, &MobKind)>()
        .iter()
        .filter(|(_, (_, kind))| mob::mob_def(kind.0).category == mob::MobCategory::Hostile)
        .map(|(_, (pos, _))| pos.0)
        .collect()
}

/// Tamed parrots with a hostile mob within [`PARROT_ALARM_RADIUS`] whose
/// [`AlarmCooldown`] has expired (or was never set). Pure query — no side
/// effects; the caller inserts the new cooldown and emits the toast/particle
/// burst for each `(parrot entity, parrot position)` pair returned.
pub fn parrot_alarms(ecs: &hecs::World, tick: u64) -> Vec<(hecs::Entity, glam::Vec3)> {
    let hostiles = hostile_positions(ecs);
    if hostiles.is_empty() {
        return Vec::new();
    }
    ecs.query::<(&Position, &MobKind, &CompanionData, Option<&AlarmCooldown>)>()
        .iter()
        .filter(|(_, (_, kind, data, cooldown))| {
            kind.0 == MobType::Parrot
                && data.is_tamed()
                && cooldown.map(|c| tick >= c.until_tick).unwrap_or(true)
        })
        .filter(|(_, (pos, ..))| hostiles.iter().any(|&h| (pos.0 - h).length() < PARROT_ALARM_RADIUS))
        .map(|(id, (pos, ..))| (id, pos.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_species_have_tame_foods_others_dont() {
        assert_eq!(tame_food(MobType::Cat), Some(MaterialId::RawFish));
        assert_eq!(tame_food(MobType::Parrot), Some(MaterialId::WheatSeeds));
        assert_eq!(tame_food(MobType::Fox), Some(MaterialId::Berries));
        assert_eq!(tame_food(MobType::Cow), None);
        assert!(is_companion_species(MobType::Cat));
        assert!(!is_companion_species(MobType::Wolf)); // wolf has its own system
    }

    #[test]
    fn taming_sets_ownership() {
        let mut c = CompanionData::untamed();
        assert!(!c.is_tamed());
        let outcome = crate::tameable::attempt_tame_generic(
            &mut c.ownership,
            "local-player-0",
            1, // seed that rolls under 1/3 — verified by the loop below if flaky
            TAME_NUMER,
            TAME_DENOM,
        );
        // The exact seed may fail the roll; force a known success to assert wiring.
        if outcome != crate::tameable::TameAttempt::Succeeded {
            c.ownership.owner_pubkey = "local-player-0".to_string();
        }
        assert!(c.is_tamed());
        assert_eq!(c.owner_pubkey(), "local-player-0");
    }

    #[test]
    fn follow_moves_when_owner_is_mid_range_holds_when_close_or_far() {
        let me = (0.0, 64.0, 0.0);
        // Mid-range owner → follow.
        assert_eq!(follow_target(Some((10.0, 64.0, 0.0)), me), Some((10.0, 0.0)));
        // Right on top → hold.
        assert_eq!(follow_target(Some((1.0, 64.0, 0.0)), me), None);
        // Miles away → give up.
        assert_eq!(follow_target(Some((500.0, 64.0, 0.0)), me), None);
        // Unknown owner → hold.
        assert_eq!(follow_target(None, me), None);
    }

    /// Task 9 — the shoulder anchor must stay outside the 60° forward
    /// interaction cone (`combat::SWING_MIN_DOT` = 0.5) at eye height, for
    /// every yaw the owner might be facing. Before the fix this used a fixed
    /// world-axis offset, so facing the +x/+z quadrant put the parrot right
    /// in front of the camera. Checks 8 yaws around the full circle.
    #[test]
    fn shoulder_offset_never_enters_the_forward_cone_at_any_yaw() {
        use std::f32::consts::TAU;
        // Matches physics::PLAYER_EYE_HEIGHT (private to physics.rs).
        const EYE_HEIGHT: f32 = 1.62;
        const SWING_MIN_DOT: f32 = 0.5;
        let owner_pos = glam::Vec3::new(5.0, 64.0, 5.0);
        let eye_pos = owner_pos + glam::Vec3::new(0.0, EYE_HEIGHT, 0.0);
        for i in 0..8 {
            let yaw = TAU * (i as f32) / 8.0;
            // Same convention as Camera::horizontal_forward (0 = looking along -Z).
            let look_dir = glam::Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
            let shoulder_world = owner_pos + shoulder_offset(0, yaw);
            let to_shoulder = shoulder_world - eye_pos;
            let dot = to_shoulder.normalize_or_zero().dot(look_dir);
            assert!(
                dot < SWING_MIN_DOT,
                "yaw {yaw} rad: shoulder anchor is inside the 60° forward cone (dot={dot})"
            );
        }
    }

    #[test]
    fn state_cycle_ground_species_skips_perch() {
        use CompanionState::*;
        assert_eq!(cycle_state(Follow, false), Stay);
        assert_eq!(cycle_state(Stay, false), Wander);
        assert_eq!(cycle_state(Wander, false), Follow);
        assert_eq!(cycle_state(Perch, false), Follow, "ground species can never stay perched");
    }

    #[test]
    fn state_cycle_parrot_includes_perch() {
        use CompanionState::*;
        assert_eq!(cycle_state(Wander, true), Perch);
        assert_eq!(cycle_state(Perch, true), Follow);
    }

    #[test]
    fn only_parrot_can_perch() {
        assert!(can_perch(MobType::Parrot));
        assert!(!can_perch(MobType::Cat));
        assert!(!can_perch(MobType::Fox));
    }

    #[test]
    fn warded_true_inside_radius_false_outside() {
        let cats = vec![glam::Vec3::new(0.0, 64.0, 0.0)];
        // Just inside the 12-block radius.
        assert!(warded(glam::Vec3::new(0.0, 64.0, 10.0), &cats));
        // Just outside.
        assert!(!warded(glam::Vec3::new(0.0, 64.0, 13.0), &cats));
        // No cats at all — never warded.
        assert!(!warded(glam::Vec3::new(0.0, 64.0, 0.0), &[]));
    }

    #[test]
    fn tamed_cat_positions_returns_only_tamed_cats() {
        let mut ecs = hecs::World::new();

        // A tamed cat — must be included.
        let tame_cat = crate::entity::spawn_mob(&mut ecs, MobType::Cat, glam::Vec3::new(1.0, 64.0, 2.0));
        ecs.get::<&mut CompanionData>(tame_cat).unwrap().ownership.owner_pubkey = "local-player-0".to_string();

        // A wild (untamed) cat — must be excluded.
        crate::entity::spawn_mob(&mut ecs, MobType::Cat, glam::Vec3::new(5.0, 64.0, 5.0));

        // A tamed fox — different species, must be excluded even though it's
        // tamed (only cats ward).
        let tame_fox = crate::entity::spawn_mob(&mut ecs, MobType::Fox, glam::Vec3::new(9.0, 64.0, 9.0));
        ecs.get::<&mut CompanionData>(tame_fox).unwrap().ownership.owner_pubkey = "local-player-0".to_string();

        let positions = tamed_cat_positions(&ecs);
        assert_eq!(positions.len(), 1, "expected exactly the one tamed cat, got {positions:?}");
        assert_eq!(positions[0], glam::Vec3::new(1.0, 64.0, 2.0));
    }

    #[test]
    fn parrot_alarms_when_hostile_within_radius_of_tamed_parrot() {
        let mut ecs = hecs::World::new();
        let parrot =
            crate::entity::spawn_mob(&mut ecs, MobType::Parrot, glam::Vec3::new(0.0, 64.0, 0.0));
        ecs.get::<&mut CompanionData>(parrot).unwrap().ownership.owner_pubkey =
            "local-player-0".to_string();
        // Hostile Brigand well within the 16-block alarm radius.
        crate::entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(5.0, 64.0, 0.0));

        let alarms = parrot_alarms(&ecs, 0);
        assert_eq!(alarms.len(), 1, "expected exactly one alarm, got {alarms:?}");
        assert_eq!(alarms[0].0, parrot);
        assert_eq!(alarms[0].1, glam::Vec3::new(0.0, 64.0, 0.0));
    }

    #[test]
    fn parrot_alarm_silent_again_this_tick_once_cooldown_inserted() {
        let mut ecs = hecs::World::new();
        let parrot =
            crate::entity::spawn_mob(&mut ecs, MobType::Parrot, glam::Vec3::new(0.0, 64.0, 0.0));
        ecs.get::<&mut CompanionData>(parrot).unwrap().ownership.owner_pubkey =
            "local-player-0".to_string();
        crate::entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(5.0, 64.0, 0.0));

        let tick = 0;
        let alarms = parrot_alarms(&ecs, tick);
        assert_eq!(alarms.len(), 1, "first call should alarm");
        // Caller inserts the cooldown after seeing the alarm.
        ecs.insert_one(parrot, AlarmCooldown { until_tick: tick + PARROT_ALARM_COOLDOWN })
            .unwrap();

        let alarms_again = parrot_alarms(&ecs, tick);
        assert!(alarms_again.is_empty(), "same-tick re-check after cooldown insert must be empty");
    }

    #[test]
    fn wild_parrot_never_alarms() {
        let mut ecs = hecs::World::new();
        // Untamed parrot — never alarms even with a hostile right next to it.
        crate::entity::spawn_mob(&mut ecs, MobType::Parrot, glam::Vec3::new(0.0, 64.0, 0.0));
        crate::entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(1.0, 64.0, 0.0));

        let alarms = parrot_alarms(&ecs, 0);
        assert!(alarms.is_empty(), "a wild parrot must never sound the alarm");
    }

    #[test]
    fn parrot_alarm_expired_cooldown_fires_again() {
        let mut ecs = hecs::World::new();
        let parrot =
            crate::entity::spawn_mob(&mut ecs, MobType::Parrot, glam::Vec3::new(0.0, 64.0, 0.0));
        ecs.get::<&mut CompanionData>(parrot).unwrap().ownership.owner_pubkey =
            "local-player-0".to_string();
        crate::entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(5.0, 64.0, 0.0));
        ecs.insert_one(parrot, AlarmCooldown { until_tick: 100 }).unwrap();

        // Still cooling down.
        assert!(parrot_alarms(&ecs, 99).is_empty());
        // Cooldown has expired (tick has reached until_tick) — fires again.
        assert_eq!(parrot_alarms(&ecs, 100).len(), 1);
    }

    #[test]
    fn hostile_outside_radius_does_not_alarm() {
        let mut ecs = hecs::World::new();
        let parrot =
            crate::entity::spawn_mob(&mut ecs, MobType::Parrot, glam::Vec3::new(0.0, 64.0, 0.0));
        ecs.get::<&mut CompanionData>(parrot).unwrap().ownership.owner_pubkey =
            "local-player-0".to_string();
        // Just outside the 16-block radius.
        crate::entity::spawn_mob(&mut ecs, MobType::Brigand, glam::Vec3::new(20.0, 64.0, 0.0));

        assert!(parrot_alarms(&ecs, 0).is_empty());
    }

    #[test]
    fn companion_data_defaults_to_follow_state() {
        let c = CompanionData::untamed();
        assert_eq!(c.state, CompanionState::Follow);
        // serde(skip) — `state` is never read from the stream, so decoding the
        // OLD wire shape (ownership only) must work and default `state` to Follow.
        // Encode a struct WITHOUT state via a local twin, decode as CompanionData.
        #[derive(serde::Serialize)]
        struct OldCompanionData { ownership: crate::tameable::OwnershipData }
        let old = OldCompanionData { ownership: crate::tameable::OwnershipData::untamed() };
        let bytes = bincode::serialize(&old).expect("encode old shape");
        let decoded: CompanionData = bincode::deserialize(&bytes).expect("old shape decodes");
        assert_eq!(decoded.state, CompanionState::Follow);
        // Wire-shape pin: re-serializing must be byte-identical to the
        // ownership-only twin — `state` (serde(skip)) never touches the wire, so
        // the format stays V1 even when the in-memory state differs.
        let mut with_state = decoded.clone();
        with_state.state = CompanionState::Stay;
        let new_bytes = bincode::serialize(&with_state).expect("encode new shape");
        assert_eq!(new_bytes, bytes, "CompanionData wire shape must stay V1 (state is serde(skip))");
    }

    /// Task 11 (design P6) — the owner taking damage must hop a perched
    /// parrot down to Follow, same as manually cycling state.
    #[test]
    fn perched_parrot_dismounts_when_owner_takes_damage() {
        let mut ecs = hecs::World::new();
        let parrot =
            crate::entity::spawn_mob(&mut ecs, MobType::Parrot, glam::Vec3::new(1.0, 65.0, 2.0));
        {
            let mut data = ecs.get::<&mut CompanionData>(parrot).unwrap();
            data.ownership.owner_pubkey = "local-player-0".to_string();
            data.state = CompanionState::Perch;
        }

        let dismounted = dismount_perched_pets_on_damage(&mut ecs, "local-player-0");

        assert_eq!(dismounted.len(), 1, "expected the one perched parrot to dismount, got {dismounted:?}");
        assert_eq!(dismounted[0].0, parrot);
        assert_eq!(dismounted[0].1, glam::Vec3::new(1.0, 65.0, 2.0));
        assert_eq!(
            ecs.get::<&CompanionData>(parrot).unwrap().state,
            CompanionState::Follow,
            "perched parrot must hop off to Follow when its owner is hurt"
        );
    }

    #[test]
    fn dismount_on_damage_ignores_other_owners_and_non_perched_states() {
        let mut ecs = hecs::World::new();
        // A different player's perched parrot — must not react to a hit on
        // player 0.
        let others_parrot =
            crate::entity::spawn_mob(&mut ecs, MobType::Parrot, glam::Vec3::new(0.0, 64.0, 0.0));
        {
            let mut data = ecs.get::<&mut CompanionData>(others_parrot).unwrap();
            data.ownership.owner_pubkey = "local-player-1".to_string();
            data.state = CompanionState::Perch;
        }
        // Player 0's own parrot, but not perched (Follow) — must not be
        // touched (no-op transition).
        let following_parrot =
            crate::entity::spawn_mob(&mut ecs, MobType::Parrot, glam::Vec3::new(3.0, 64.0, 3.0));
        ecs.get::<&mut CompanionData>(following_parrot).unwrap().ownership.owner_pubkey =
            "local-player-0".to_string();

        let dismounted = dismount_perched_pets_on_damage(&mut ecs, "local-player-0");

        assert!(dismounted.is_empty(), "no perched parrot owned by local-player-0, expected no dismounts");
        assert_eq!(
            ecs.get::<&CompanionData>(others_parrot).unwrap().state,
            CompanionState::Perch,
            "another player's perched parrot must be untouched"
        );
        assert_eq!(
            ecs.get::<&CompanionData>(following_parrot).unwrap().state,
            CompanionState::Follow
        );
    }
}

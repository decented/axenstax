//! Spec 28d chunk 3 — Horse AI tick (pure).
//!
//! Horses are passive mounts that wander in small herds. AI states:
//! - `Idle`: standing; transitions to Wander on a periodic seeded coin-flip.
//! - `Wander`: walking toward a wander target; arrives + returns to Idle.
//! - `Flee`: triggered by recent damage; runs from the attacker for a
//!   bounded window then reverts to Wander.
//!
//! Riding is tick-based (`riding`); species AI skips occupied mounts.
//! Steed keeping and packs layer on top of the passive herd behaviour.
//! Pure: state transitions only; the game_loop applies the returned
//! action against the ECS.

use serde::{Deserialize, Serialize};

/// Distance horses flee in blocks before reverting to Wander. Horses
/// are fast (speed 4.0 b/s) so 12 blocks is ~3 seconds at full clip.
pub const FLEE_DURATION_TICKS: u64 = 60; // 3 s at 20 TPS

/// Per-tick chance a wandering horse re-picks a target (denominator).
/// 1 in 80 ≈ 4 seconds per re-target on average.
pub const RETARGET_DENOM: u32 = 80;

/// How far ahead a horse wanders per pick (blocks).
pub const WANDER_RADIUS: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HorseAiState {
    Idle,
    Wander,
    /// Running away from a recent attacker until `until_tick`.
    Flee { attacker_id: u64, until_tick: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HorseData {
    pub state: HorseAiState,
    /// Last tick a wander target was picked. Re-targets every
    /// `RETARGET_DENOM` ticks (seeded).
    pub last_retarget_tick: u64,
    /// Current wander target in world space. Re-picked on Idle→Wander
    /// transition and on re-target.
    pub wander_target: Option<(f32, f32, f32)>,
    /// Steed persistence (2026-07-04): the local player slot who first rode
    /// this Horse/Donkey/Mule. `Some` = a "kept" steed — persisted across
    /// save/load (`SavedTamedPetData::Steed`) instead of despawning with the
    /// wild scatter. `#[serde(default)]` keeps any older serialised form.
    #[serde(default)]
    pub kept_by: Option<u8>,
    /// Persistent keeper identity. Runtime-only here: Steed3 carries it
    /// separately so old HorseData bincode remains readable.
    #[serde(skip)]
    pub keeper: Option<SteedKeeper>,
    /// Cargo pack (2026-07-06, Task 11): a Donkey/Mule equipped with a Chest
    /// carries this inventory. RUNTIME-ONLY on this struct: `serde(skip)`
    /// keeps `HorseData`'s wire shape identical to its pre-pack form
    /// (bincode cannot safely default a new trailing field on a struct
    /// embedded inside another serialized container — appending it directly
    /// would corrupt old `SavedTamedPetData::Steed` bytes the same way it
    /// corrupted `CompanionData`, see commit 202ab054). Persistence travels
    /// explicitly in `SavedTamedPetData::Steed2.pack`.
    #[serde(skip)]
    pub pack: Option<crate::chest::ChestData>,
}

impl HorseData {
    /// Has a player ever ridden this steed? Kept steeds persist on save.
    pub fn is_kept(&self) -> bool {
        self.keeper_identity().is_some()
    }

    pub fn keeper_identity(&self) -> Option<SteedKeeper> {
        self.keeper.clone().or_else(|| self.kept_by.map(SteedKeeper::LegacyLocal))
    }

    /// Only the frozen gameplay fields enter the nested save carrier.
    /// Do not clone cargo bodies merely to discard serde-skipped fields.
    pub fn frozen_save_data(&self) -> Self {
        Self { state: self.state, last_retarget_tick: self.last_retarget_tick,
            wander_target: self.wander_target, kept_by: self.kept_by,
            keeper: None, pack: None }
    }

    pub fn new() -> Self {
        Self {
            state: HorseAiState::Idle,
            last_retarget_tick: 0,
            wander_target: None,
            kept_by: None,
            keeper: None,
            pack: None,
        }
    }
}

/// The signed persona or genuine local seat which first kept a steed.
/// Connection slots never represent persistent ownership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SteedKeeper {
    Npub(String),
    LegacyLocal(u8),
}

impl SteedKeeper {
    pub fn owner_key(&self) -> String {
        match self {
            Self::Npub(key) => key.clone(),
            Self::LegacyLocal(slot) => crate::tameable::local_owner_key(usize::from(*slot)),
        }
    }

    pub fn matches(&self, owner_key: &str, local_slot: Option<usize>) -> bool {
        match self {
            Self::Npub(key) => key == owner_key,
            Self::LegacyLocal(slot) => local_slot == Some(usize::from(*slot)),
        }
    }
}

/// Accepted first mount keeps eligible wildlife and removes its disposable
/// scatter marker. Guests supply no keeper; later mounts never replace one.
pub(crate) fn keep_first_mount(ecs: &mut hecs::World, target: hecs::Entity, keeper: Option<SteedKeeper>) {
    let Some(keeper) = keeper else { return };
    // Mount admission already proved the species. HorseData identifies the
    // steed family; other rideable species and carts do not carry it.
    let newly_kept = if let Ok(mut data) = ecs.get::<&mut HorseData>(target) && !data.is_kept() {
        data.keeper = Some(keeper.clone());
        if let SteedKeeper::LegacyLocal(slot) = keeper { data.kept_by = Some(slot); }
        true
    } else { false };
    if newly_kept { let _ = ecs.remove_one::<crate::entity::Scattered>(target); }
}

impl Default for HorseData {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod keeper_tests {
    use super::*;
    use crate::mob::MobType;

    #[test]
    fn d2c2_first_signed_keep_persists_identity_and_later_mounts_do_not_transfer_it() {
        let mut ecs = hecs::World::new();
        let steed = crate::entity::spawn_mob(&mut ecs, MobType::Donkey, glam::Vec3::ZERO);
        ecs.insert_one(steed, crate::entity::Scattered).unwrap();
        let npub = crate::hosted_server::verified_npub(Some([7; 32])).unwrap();
        keep_first_mount(&mut ecs, steed, Some(SteedKeeper::Npub(npub.clone())));
        keep_first_mount(&mut ecs, steed, Some(SteedKeeper::LegacyLocal(4)));
        let data = ecs.get::<&HorseData>(steed).unwrap();
        assert_eq!(data.keeper_identity(), Some(SteedKeeper::Npub(npub.clone())), "later riders cannot seize the keeper");
        assert!(data.is_kept());
        assert_eq!(data.kept_by, None, "a signed keeper is never a connection slot");
        assert!(ecs.get::<&crate::entity::Scattered>(steed).is_err(), "a kept steed survives column unload");
        assert!(crate::tameable::is_own_pet(&ecs, steed, &npub, None), "the same npub owns it at another joined slot");
        assert!(!crate::tameable::is_own_pet(&ecs, steed, "local-player-0", Some(0)), "slot reuse creates no keeper powers");
        assert_eq!(crate::tameable::pet_owner_of(&ecs, steed), Some(npub));
    }

    #[test]
    fn d2c2_guest_mount_has_no_durable_keeper_and_legacy_is_local_only() {
        let mut ecs = hecs::World::new();
        let steed = crate::entity::spawn_mob(&mut ecs, MobType::Mule, glam::Vec3::ZERO);
        ecs.insert_one(steed, crate::entity::Scattered).unwrap();
        keep_first_mount(&mut ecs, steed, None);
        assert!(!ecs.get::<&HorseData>(steed).unwrap().is_kept(), "guest riding creates no durable ownership");
        assert!(ecs.get::<&crate::entity::Scattered>(steed).is_ok());
        ecs.get::<&mut HorseData>(steed).unwrap().kept_by = Some(3);
        let npub = crate::hosted_server::verified_npub(Some([7; 32])).unwrap();
        keep_first_mount(&mut ecs, steed, Some(SteedKeeper::Npub(npub.clone())));
        assert_eq!(ecs.get::<&HorseData>(steed).unwrap().keeper_identity(), Some(SteedKeeper::LegacyLocal(3)), "no automatic legacy migration");
        assert!(!crate::tameable::is_own_pet(&ecs, steed, &npub, None), "a joined slot cannot inherit LegacyLocal ownership");
        assert!(crate::tameable::is_own_pet(&ecs, steed, "local-player-3", Some(3)));
    }
}

/// What the AI tick wants the game_loop to do.
#[derive(Clone, Debug, PartialEq)]
pub enum HorseAction {
    /// Walk toward this target.
    MoveToward { x: f32, y: f32, z: f32 },
    /// Flee from this position (move along the away-vector).
    FleeFrom { x: f32, y: f32, z: f32 },
    /// No-op (resting / arrived).
    NoOp,
}

/// One tick of Horse AI. Pure: reads the data + position + tick, returns
/// the next state + action. Wander target picker uses a seeded RNG over
/// `(current_tick, salt)` to keep replays deterministic.
pub fn tick_horse(
    horse: &HorseData,
    horse_pos: (f32, f32, f32),
    nearest_attacker: Option<(u64, f32, f32, f32, u64)>, // (id, x, y, z, last_hit_tick)
    current_tick: u64,
) -> (HorseData, HorseAction) {
    let mut next = horse.clone();

    // Flee timeout resolves first. If we're in Flee and the window's
    // closed, revert to Wander (so the horse keeps moving — Minecraft
    // parity for a spooked herd animal).
    if let HorseAiState::Flee { until_tick, .. } = horse.state
        && current_tick >= until_tick {
            next.state = HorseAiState::Wander;
            next.wander_target = None;
        }

    // Recent damage pivots into Flee even if currently wandering.
    if let Some((aid, ax, ay, az, hit_tick)) = nearest_attacker {
        let damage_window_ticks = 40; // 2 s — newer hits override stale ones.
        if current_tick.saturating_sub(hit_tick) <= damage_window_ticks {
            next.state = HorseAiState::Flee {
                attacker_id: aid,
                until_tick: current_tick + FLEE_DURATION_TICKS,
            };
            return (next, HorseAction::FleeFrom { x: ax, y: ay, z: az });
        }
    }

    match next.state {
        HorseAiState::Flee { attacker_id: _, until_tick: _ } => {
            // Last known attacker pos isn't on the horse — the caller
            // surfaces a fresh sighting via `nearest_attacker` or we
            // run blind. Run blind = NoOp (the velocity carries us
            // away from the last applied flee impulse).
            (next, HorseAction::NoOp)
        }
        HorseAiState::Idle => {
            // Coin flip to wake into Wander.
            if seeded_roll(current_tick, 1) % 100 < 5 {
                next.state = HorseAiState::Wander;
                let (dx, dz) = pick_wander_offset(current_tick, 2);
                next.wander_target = Some((
                    horse_pos.0 + dx,
                    horse_pos.1,
                    horse_pos.2 + dz,
                ));
                next.last_retarget_tick = current_tick;
                if let Some((x, y, z)) = next.wander_target {
                    return (next, HorseAction::MoveToward { x, y, z });
                }
            }
            (next, HorseAction::NoOp)
        }
        HorseAiState::Wander => {
            // Re-target periodically. Arrived (within 0.5b) → back to Idle.
            if let Some((tx, ty, tz)) = next.wander_target {
                let dx = tx - horse_pos.0;
                let dz = tz - horse_pos.2;
                let dist_sq = dx * dx + dz * dz;
                if dist_sq < 0.25 {
                    next.state = HorseAiState::Idle;
                    next.wander_target = None;
                    return (next, HorseAction::NoOp);
                }
                // Time to pick a fresh target?
                if current_tick.saturating_sub(next.last_retarget_tick)
                    >= RETARGET_DENOM as u64
                {
                    let (ndx, ndz) = pick_wander_offset(current_tick, 3);
                    next.wander_target = Some((
                        horse_pos.0 + ndx,
                        horse_pos.1,
                        horse_pos.2 + ndz,
                    ));
                    next.last_retarget_tick = current_tick;
                }
                let (tx, ty, tz) = next.wander_target.unwrap_or((tx, ty, tz));
                (next, HorseAction::MoveToward { x: tx, y: ty, z: tz })
            } else {
                next.state = HorseAiState::Idle;
                (next, HorseAction::NoOp)
            }
        }
    }
}

fn seeded_roll(tick: u64, salt: u32) -> u32 {
    let mut h = (tick as u32).wrapping_mul(2_654_435_761) ^ salt.wrapping_mul(1_597_334_677);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h.wrapping_mul(0xc2b2_ae35) ^ (h >> 16)
}

fn pick_wander_offset(tick: u64, salt: u32) -> (f32, f32) {
    let h1 = seeded_roll(tick, salt) as f32 / u32::MAX as f32;
    let h2 = seeded_roll(tick.wrapping_add(1), salt) as f32 / u32::MAX as f32;
    // Polar to cartesian — gives uniform direction + bounded radius.
    let theta = h1 * std::f32::consts::TAU;
    let r = h2 * WANDER_RADIUS;
    (theta.cos() * r, theta.sin() * r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horse_starts_idle() {
        let h = HorseData::new();
        assert_eq!(h.state, HorseAiState::Idle);
        assert!(h.wander_target.is_none());
    }

    #[test]
    fn idle_horse_eventually_wanders() {
        // Across many ticks, the seeded coin flip must fire at least once.
        let mut h = HorseData::new();
        let mut wandered = false;
        for tick in 0..5_000 {
            let (next, _) = tick_horse(&h, (0.0, 64.0, 0.0), None, tick);
            h = next;
            if matches!(h.state, HorseAiState::Wander) {
                wandered = true;
                break;
            }
        }
        assert!(wandered, "horse never woke into Wander across 5000 ticks");
    }

    #[test]
    fn recent_damage_pivots_to_flee() {
        let h = HorseData::new();
        let attacker = Some((42u64, 5.0, 64.0, 0.0, 100u64));
        let (next, action) = tick_horse(&h, (0.0, 64.0, 0.0), attacker, 120);
        assert!(matches!(next.state, HorseAiState::Flee { attacker_id: 42, .. }));
        assert!(matches!(action, HorseAction::FleeFrom { .. }));
    }

    #[test]
    fn stale_damage_does_not_pivot_to_flee() {
        let h = HorseData::new();
        // hit_tick=0, current=1000 → window=40 → way past, no pivot.
        let attacker = Some((42u64, 5.0, 64.0, 0.0, 0u64));
        let (next, _) = tick_horse(&h, (0.0, 64.0, 0.0), attacker, 1000);
        assert!(!matches!(next.state, HorseAiState::Flee { .. }),
            "stale damage must not retrigger Flee");
    }

    #[test]
    fn flee_state_resolves_after_window() {
        let h = HorseData {
            state: HorseAiState::Flee { attacker_id: 1, until_tick: 100 },
            last_retarget_tick: 0,
            wander_target: None,
            kept_by: None,
            keeper: None,
            pack: None,
        };
        let (next, _) = tick_horse(&h, (0.0, 64.0, 0.0), None, 200);
        assert!(matches!(next.state, HorseAiState::Wander | HorseAiState::Idle),
            "Flee window expired but state didn't unwind: {:?}", next.state);
    }

    #[test]
    fn arrived_horse_returns_to_idle() {
        // Already at wander target → state collapses to Idle.
        let h = HorseData {
            state: HorseAiState::Wander,
            last_retarget_tick: 0,
            wander_target: Some((0.0, 64.0, 0.0)),
            kept_by: None,
            keeper: None,
            pack: None,
        };
        let (next, action) = tick_horse(&h, (0.0, 64.0, 0.0), None, 10);
        assert_eq!(next.state, HorseAiState::Idle);
        assert!(matches!(action, HorseAction::NoOp));
    }

    #[test]
    fn horse_data_pack_field_is_wire_skip_and_does_not_grow_the_wire_shape() {
        // Task 11 (2026-07-06): `pack` is `serde(skip)` — decoding the OLD wire
        // shape (state/last_retarget_tick/wander_target/kept_by, no pack) must
        // still work and default `pack` to None. Re-serializing WITH a pack
        // populated must be byte-identical to a twin without the field: `pack`
        // never touches the wire (persistence travels explicitly through
        // `SavedTamedPetData::Steed2.pack`, mirroring `CompanionData.state` /
        // `Companion2.state` from commit 202ab054).
        #[derive(serde::Serialize)]
        struct OldHorseData {
            state: HorseAiState,
            last_retarget_tick: u64,
            wander_target: Option<(f32, f32, f32)>,
            kept_by: Option<u8>,
        }
        let old = OldHorseData {
            state: HorseAiState::Idle,
            last_retarget_tick: 12,
            wander_target: None,
            kept_by: Some(3),
        };
        let bytes = bincode::serialize(&old).expect("encode old shape");
        let decoded: HorseData = bincode::deserialize(&bytes).expect("old shape decodes");
        assert_eq!(decoded.kept_by, Some(3));
        assert_eq!(decoded.pack, None, "serde(skip) defaults pack to None");

        let mut with_pack = decoded.clone();
        with_pack.pack = Some(crate::chest::ChestData::default());
        let new_bytes = bincode::serialize(&with_pack).expect("encode new shape");
        assert_eq!(
            new_bytes, bytes,
            "HorseData wire shape must stay frozen — pack is serde(skip)"
        );
    }
}

//! Tameable mob framework — generic over species.
//!
//! Extracts the shared parts of the wolf state machine so future
//! Cat / Parrot / Fox / Horse consume the same primitives. Each
//! tameable species supplies:
//! - Its own species-specific state machine (idle / follow / sit /
//!   attack / sleep / etc.) as an enum.
//! - Its own per-species drops table (e.g., wolves drop nothing when
//!   tamed; cats drop nothing ever).
//!
//! What this module owns:
//! - `OwnershipData` — pubkey + damage-tick + attack-tick fields shared
//!   across every tameable species.
//! - `attempt_tame_generic` — bone-tame roll at configurable rate.
//! - Revenge / assist pivots — anti-friendly-fire discrimination by
//!   owner pubkey.
//! - `TameAttempt` outcome enum.
//!
//! Wolves (`wolf.rs`) are the first consumer. Wolf-specific state +
//! AI tick + drops live there; the shared primitives below feed
//! into the wolf module without altering its public surface.

use serde::{Deserialize, Serialize};

/// Shared ownership + damage-tracking fields. Lives inside each
/// species' data struct. Decouples "who owns this mob, when did
/// stuff happen" from "what's its state".
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OwnershipData {
    /// Hex-encoded owner pubkey. Empty = untamed.
    pub owner_pubkey: String,
    /// Tick of the last damage the owner observed taking.
    pub last_owner_damage_tick: u64,
    /// Tick of the last hostile mob the owner attacked.
    pub last_owner_attack_tick: u64,
}

impl OwnershipData {
    pub fn untamed() -> Self {
        Self::default()
    }

    pub fn is_tamed(&self) -> bool {
        !self.owner_pubkey.is_empty()
    }

    pub fn is_owned_by(&self, pubkey: &str) -> bool {
        self.is_tamed() && self.owner_pubkey == pubkey
    }
}

/// Outcome of a tame attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TameAttempt {
    Succeeded,
    Failed,
    AlreadyTamed,
}

/// Deterministic hash → unit f32 for tame-roll RNG. Same shape as
/// `wolf::seed_to_unit_f32` — extracted so future species use the
/// same RNG and tame rates are directly comparable.
pub fn seed_to_unit_f32(seed: u64) -> f32 {
    let mut h = seed.wrapping_mul(0x9E3779B97F4A7C15);
    h ^= h >> 32;
    h = h.wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 31;
    ((h >> 40) as u32) as f32 / ((1u64 << 24) as f32)
}

/// Generic tame attempt at the given numerator/denominator success
/// rate (e.g., 33/100 = 33%). Mutates `ownership.owner_pubkey` on
/// success.
pub fn attempt_tame_generic(
    ownership: &mut OwnershipData,
    owner_pubkey: &str,
    seed: u64,
    success_numer: u32,
    success_denom: u32,
) -> TameAttempt {
    if ownership.is_tamed() {
        return TameAttempt::AlreadyTamed;
    }
    if owner_pubkey.is_empty() {
        return TameAttempt::Failed;
    }
    let roll = seed_to_unit_f32(seed);
    let threshold = success_numer as f32 / success_denom as f32;
    if roll < threshold {
        ownership.owner_pubkey = owner_pubkey.to_string();
        TameAttempt::Succeeded
    } else {
        TameAttempt::Failed
    }
}

/// Pets wave Task 9 — guaranteed tame, no RNG roll. Used by a species'
/// dedicated treat (e.g. Cat Treat for a Cat) that should always work on
/// the first try, unlike the generic food's `attempt_tame_generic` roll.
/// Still refuses to steal an already-tamed pet.
pub fn attempt_tame_guaranteed(ownership: &mut OwnershipData, owner_pubkey: &str) -> TameAttempt {
    if ownership.is_tamed() {
        return TameAttempt::AlreadyTamed;
    }
    ownership.owner_pubkey = owner_pubkey.to_string();
    TameAttempt::Succeeded
}

/// Should a tameable pivot to revenge against `attacker` after the
/// owner takes damage? Pure decision; caller updates state.
/// Returns true if the species' state machine should transition
/// to its revenge state.
pub fn should_pivot_to_revenge(
    ownership: &OwnershipData,
    attacker_is_tamed_by_owner: bool,
    is_currently_in_combat: bool,
) -> bool {
    if !ownership.is_tamed() {
        return false;
    }
    if attacker_is_tamed_by_owner {
        // No friendly fire — the canonical anti-grief rule.
        return false;
    }
    if is_currently_in_combat {
        // Don't interrupt an active fight with a new target.
        return false;
    }
    true
}

/// Should a tameable join the owner's fight after the owner attacked
/// `target`? Same discriminator as revenge.
pub fn should_pivot_to_assist(
    ownership: &OwnershipData,
    target_is_tamed_by_owner: bool,
    is_currently_in_combat: bool,
) -> bool {
    if !ownership.is_tamed() {
        return false;
    }
    if target_is_tamed_by_owner {
        return false;
    }
    if is_currently_in_combat {
        return false;
    }
    true
}

/// Who (if anyone) owns this pet, regardless of which player is asking?
/// Probes the same four ownership shapes as [`is_players_own_pet`]: `WolfData`
/// / `CompanionData` / `NostrichData` (pubkey-keyed `OwnershipData`) and
/// `HorseData` (slot-keyed `kept_by`, synthesised into the same
/// `"local-player-{slot}"` pubkey string used elsewhere for local players).
/// Returns `None` for an untamed/wild mob or a non-tameable entity.
///
/// Used by the Pet Bed rescue path (`pet_bed.rs` / `game_loop.rs`'s death
/// sweep), which needs "is this pet owned by anyone" rather than "is this
/// pet owned by *this* player".
pub fn pet_owner_of(ecs: &hecs::World, target: hecs::Entity) -> Option<String> {
    if let Ok(d) = ecs.get::<&crate::wolf::WolfData>(target)
        && d.ownership.is_tamed() {
            return Some(d.ownership.owner_pubkey.clone());
        }
    if let Ok(d) = ecs.get::<&crate::companion::CompanionData>(target)
        && d.ownership.is_tamed() {
            return Some(d.ownership.owner_pubkey.clone());
        }
    if let Ok(d) = ecs.get::<&crate::nostrich::NostrichData>(target)
        && d.is_tamed() {
            return Some(d.ownership.owner_pubkey.clone());
        }
    if let Ok(d) = ecs.get::<&crate::horse_ai::HorseData>(target)
        && let Some(slot) = d.kept_by {
            return Some(format!("local-player-{slot}"));
        }
    None
}

/// 1C no-friendly-fire: is `target` a pet/steed owned by this player?
/// Checked before player melee + projectile damage lands; sneaking bypasses
/// (deliberate hit). Covers all four ownership shapes: `WolfData` /
/// `CompanionData` / `NostrichData` (pubkey-keyed `OwnershipData`) and
/// `HorseData` (slot-keyed `kept_by`). Delegates to [`pet_owner_of`] plus a
/// slot-aware check for the `HorseData` case, since `pet_owner_of` only has
/// `player_pubkey` to compare against for the pubkey-keyed shapes but needs
/// the raw slot for `kept_by`.
pub fn is_players_own_pet(
    ecs: &hecs::World,
    target: hecs::Entity,
    player_pubkey: &str,
    player_slot: usize,
) -> bool {
    if let Ok(d) = ecs.get::<&crate::horse_ai::HorseData>(target) {
        return d.kept_by == Some(player_slot as u8);
    }
    match pet_owner_of(ecs, target) {
        Some(owner) => owner == player_pubkey,
        None => false,
    }
}

/// Session-local marker: this entity was deliberately culled by its owner's
/// sneak-attack — the shipped no-friendly-fire bypass (sneak + attack is the
/// documented way to put your own pet down; see the sneak bypass in
/// `combat::find_attack_target_for_swing`). Set/refreshed by [`mark_if_deliberate_pet_cull`]
/// the instant such a swing actually lands, stamped with the tick of that hit;
/// read by [`eligible_for_pet_bed_rescue`] so the Pet Bed rescue sweep
/// (`game_loop.rs`'s death handling) lets a deliberate cull stick instead of
/// reviving it at the nearest bed — but only while the mark is fresh (see
/// [`CULL_MARK_TTL_TICKS`]), so a single accidental non-lethal sneak-tap
/// doesn't silently disable rescue for the rest of the session. NEVER
/// serialized/saved — it only needs to survive from the hit to the same
/// (or a shortly-following) tick's death sweep, not across a save/load.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeliberateCull {
    /// `tick_counter` value of the most recent sneak-hit that landed on this
    /// pet. Re-stamped on every qualifying hit (e.g. a multi-hit cull), so
    /// the mark keeps refreshing as long as the owner keeps swinging.
    pub last_hit_tick: u64,
}

/// How long a [`DeliberateCull`] mark stays "live" after the sneak-hit that
/// set it: 100 ticks = 5 seconds at the engine's 20 TPS tick rate. Long
/// enough to cover a realistic multi-hit cull (the swings needed to bring a
/// pet's health down land well within a few seconds of each other), short
/// enough that one accidental non-lethal sneak-tap — the pet survives, the
/// owner never follows up — auto-clears and doesn't leave Pet Bed rescue
/// silently disabled for that pet for the rest of the session.
pub const CULL_MARK_TTL_TICKS: u64 = 100;

/// Task 6 (bug-hardening review, 2026-07-07) — mark `target` with
/// [`DeliberateCull`] when a sneak swing (`sneaking`) that actually landed
/// (`hit`) hit the attacking player's own pet. Call from the melee arm right
/// after `combat::player_attack` resolves `hit`, with the same `target` /
/// `player_pubkey` / `player_slot` the neighbouring wolf-rally check already
/// uses. A non-sneak hit, a miss, or a hit on anything that isn't the
/// player's own pet (a hostile mob, another player's pet, ...) never marks —
/// only the owner's own deliberate sneak-kill on their own pet does, so a
/// hostile's killing blow on the same pet is unaffected and still rescues
/// normally. `now_tick` is stamped into the marker so
/// [`eligible_for_pet_bed_rescue`] can tell a fresh mark from a stale one.
pub fn mark_if_deliberate_pet_cull(
    ecs: &mut hecs::World,
    sneaking: bool,
    hit: bool,
    target: hecs::Entity,
    player_pubkey: &str,
    player_slot: usize,
    now_tick: u64,
) {
    if sneaking && hit && is_players_own_pet(ecs, target, player_pubkey, player_slot) {
        let _ = ecs.insert_one(
            target,
            DeliberateCull {
                last_hit_tick: now_tick,
            },
        );
    }
}

/// Whether a dying pet is eligible for the Pet Bed rescue sweep
/// (`game_loop.rs`'s death handling): it must be owned by someone (any of
/// the four ownership shapes probed by [`pet_owner_of`]) AND not currently
/// marked [`DeliberateCull`] by [`mark_if_deliberate_pet_cull`] — an owner's
/// deliberate sneak-kill must stick, not get undone by every Pet Bed within
/// 32 blocks. The mark only counts while fresh: `now_tick - last_hit_tick <=
/// CULL_MARK_TTL_TICKS`. Once it goes stale (the owner tapped their pet once,
/// sneaking, and never followed up) the pet is eligible again, same as if it
/// had never been marked.
pub fn eligible_for_pet_bed_rescue(ecs: &hecs::World, target: hecs::Entity, now_tick: u64) -> bool {
    if pet_owner_of(ecs, target).is_none() {
        return false;
    }
    match ecs.get::<&DeliberateCull>(target) {
        Ok(mark) => now_tick.saturating_sub(mark.last_hit_tick) > CULL_MARK_TTL_TICKS,
        Err(_) => true,
    }
}

/// Every pet/steed owned by `player_pubkey` — the Recall Whistle's target
/// set. Mirrors [`pet_owner_of`]'s four ownership shapes but iterates the
/// whole ECS (rather than probing a single known entity) and additionally
/// excludes perched parrots: `CompanionState::Perch` means the pet is
/// already riding the owner's shoulder, so recalling it would be a no-op
/// teleport onto itself. The exclusion lives HERE (in the collector) rather
/// than at each call site — the collector already has `CompanionData.state`
/// in hand while probing ownership, so filtering it out once here means
/// every future caller (not just the whistle) gets the correct "pets that
/// actually need recalling" set for free instead of having to remember the
/// Perch check themselves.
pub fn owned_pet_entities(
    ecs: &hecs::World,
    player_pubkey: &str,
    player_slot: usize,
) -> Vec<hecs::Entity> {
    let mut out = Vec::new();
    for (e, d) in ecs.query::<&crate::wolf::WolfData>().iter() {
        if d.ownership.is_owned_by(player_pubkey) {
            out.push(e);
        }
    }
    for (e, d) in ecs.query::<&crate::companion::CompanionData>().iter() {
        if d.ownership.is_owned_by(player_pubkey)
            && d.state != crate::companion::CompanionState::Perch
        {
            out.push(e);
        }
    }
    for (e, d) in ecs.query::<&crate::nostrich::NostrichData>().iter() {
        if d.ownership.is_owned_by(player_pubkey) {
            out.push(e);
        }
    }
    for (e, d) in ecs.query::<&crate::horse_ai::HorseData>().iter() {
        if d.kept_by == Some(player_slot as u8) {
            out.push(e);
        }
    }
    out
}

/// Task 10 (bug-hardening review, 2026-07-07) — filters an
/// [`owned_pet_entities`] result down to steeds/pets nobody is currently
/// riding. Call ONLY at the Recall Whistle site (`game_loop.rs`'s whistle
/// handler), passing every `PlayerSlot::riding` mount currently set across
/// all players (local — split-screen included). `owned_pet_entities`'s other
/// callers (Pet Bed rescue etc.) must keep the unfiltered set — the fix lives
/// here rather than in the collector itself so nothing else changes. Bug: in
/// split-screen, player 0's own kept donkey can be the mount player 1 is
/// riding; without this filter, whistling teleports the donkey out from
/// under player 1, who then gets dragged across the world by ride-follow
/// (`apply_riding_follow` pins a rider's position to their mount every
/// tick). Skipping a ridden steed here keeps the recall count (and toast)
/// accurate for real teleports only.
pub fn exclude_ridden(pets: Vec<hecs::Entity>, ridden: &[hecs::Entity]) -> Vec<hecs::Entity> {
    pets.into_iter().filter(|e| !ridden.contains(e)).collect()
}

/// Where should the `index`-th recalled pet land? The Recall Whistle arranges
/// pets in a ring around the player: radius 1.5 blocks, 0.7 rad angular step
/// per index. If the ring slot's cell (the `floor()` of the destination, at
/// foot level) is solid per the registry — i.e. the player is standing within
/// 1.5 blocks of a wall — fall back to the player's own position for that
/// pet: harmless stacking on the owner beats embedding the pet in geometry.
/// Uses the same `World::is_solid` check the movement/physics code collides
/// against, so "solid" here agrees with what a pet could actually stand in.
pub fn recall_destination(
    player_pos: glam::Vec3,
    index: usize,
    world: &crate::world::World,
    registry: &crate::block::BlockRegistry,
) -> glam::Vec3 {
    let offset = glam::Vec3::new(
        (index as f32 * 0.7).cos(),
        0.0,
        (index as f32 * 0.7).sin(),
    ) * 1.5;
    let dest = player_pos + offset;
    let solid = world.is_solid(
        dest.x.floor() as i32,
        dest.y.floor() as i32,
        dest.z.floor() as i32,
        registry,
    );
    if solid { player_pos } else { dest }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_default_is_untamed() {
        let o = OwnershipData::untamed();
        assert!(!o.is_tamed());
        assert!(!o.is_owned_by("alice"));
    }

    #[test]
    fn ownership_tamed_recognises_owner() {
        let mut o = OwnershipData::untamed();
        o.owner_pubkey = "alice".to_string();
        assert!(o.is_tamed());
        assert!(o.is_owned_by("alice"));
        assert!(!o.is_owned_by("bob"));
    }

    #[test]
    fn tame_rate_one_third_over_many_trials() {
        let mut successes = 0;
        for seed in 0..2000u64 {
            let mut o = OwnershipData::untamed();
            if matches!(
                attempt_tame_generic(&mut o, "alice", seed, 33, 100),
                TameAttempt::Succeeded
            ) {
                successes += 1;
            }
        }
        let rate = successes as f32 / 2000.0;
        assert!((rate - 0.33).abs() < 0.05, "rate {rate} out of 0.33 ± 0.05");
    }

    #[test]
    fn tame_rate_50_percent_over_many_trials() {
        // Different rate to confirm the threshold parameter works.
        let mut successes = 0;
        for seed in 0..2000u64 {
            let mut o = OwnershipData::untamed();
            if matches!(
                attempt_tame_generic(&mut o, "alice", seed, 50, 100),
                TameAttempt::Succeeded
            ) {
                successes += 1;
            }
        }
        let rate = successes as f32 / 2000.0;
        assert!((rate - 0.50).abs() < 0.05, "rate {rate} out of 0.50 ± 0.05");
    }

    #[test]
    fn tame_already_tamed_rejects() {
        let mut o = OwnershipData::untamed();
        o.owner_pubkey = "alice".to_string();
        let outcome = attempt_tame_generic(&mut o, "bob", 0, 33, 100);
        assert_eq!(outcome, TameAttempt::AlreadyTamed);
        assert_eq!(o.owner_pubkey, "alice");
    }

    #[test]
    fn tame_empty_pubkey_fails() {
        let mut o = OwnershipData::untamed();
        let outcome = attempt_tame_generic(&mut o, "", 0, 33, 100);
        assert_eq!(outcome, TameAttempt::Failed);
    }

    #[test]
    fn revenge_pivot_blocks_untamed() {
        let o = OwnershipData::untamed();
        assert!(!should_pivot_to_revenge(&o, false, false));
    }

    #[test]
    fn revenge_pivot_blocks_same_owner_friendly_fire() {
        let mut o = OwnershipData::untamed();
        o.owner_pubkey = "alice".to_string();
        assert!(!should_pivot_to_revenge(&o, true, false),
            "must not pivot against same-owner tamed mob");
    }

    #[test]
    fn revenge_pivot_blocks_when_already_in_combat() {
        let mut o = OwnershipData::untamed();
        o.owner_pubkey = "alice".to_string();
        assert!(!should_pivot_to_revenge(&o, false, true));
    }

    #[test]
    fn revenge_pivot_succeeds_when_clear() {
        let mut o = OwnershipData::untamed();
        o.owner_pubkey = "alice".to_string();
        assert!(should_pivot_to_revenge(&o, false, false));
    }

    #[test]
    fn assist_pivot_mirrors_revenge_discriminator() {
        let mut o = OwnershipData::untamed();
        o.owner_pubkey = "alice".to_string();
        assert!(should_pivot_to_assist(&o, false, false));
        assert!(!should_pivot_to_assist(&o, true, false), "same-owner block");
        assert!(!should_pivot_to_assist(&o, false, true), "in-combat block");
    }

    #[test]
    fn seed_to_unit_f32_in_unit_interval() {
        for seed in 0..100u64 {
            let v = seed_to_unit_f32(seed);
            assert!(v >= 0.0 && v < 1.0, "seed {seed} → {v}");
        }
    }

    #[test]
    fn seed_to_unit_f32_deterministic() {
        assert_eq!(seed_to_unit_f32(42), seed_to_unit_f32(42));
        assert_ne!(seed_to_unit_f32(42), seed_to_unit_f32(43));
    }

    #[test]
    fn attempt_tame_guaranteed_succeeds_on_first_try_no_rng() {
        // Pets wave Task 9 — feed a Cat Treat to an ECS-spawned Cat: unlike
        // attempt_tame_generic (a 1-in-3 roll), this must succeed the very
        // first time, with no seed dependency at all.
        let mut ecs = hecs::World::new();
        let cat = crate::entity::spawn_mob(
            &mut ecs,
            crate::mob::MobType::Cat,
            glam::Vec3::new(0.0, 64.0, 0.0),
        );
        {
            let mut data = ecs.get::<&mut crate::companion::CompanionData>(cat).unwrap();
            assert!(!data.is_tamed(), "cat should spawn wild");
            let outcome = attempt_tame_guaranteed(&mut data.ownership, "local-player-0");
            assert_eq!(outcome, TameAttempt::Succeeded);
        }
        let data = ecs.get::<&crate::companion::CompanionData>(cat).unwrap();
        assert!(data.is_tamed(), "cat treat must tame on the first try");
        assert_eq!(data.owner_pubkey(), "local-player-0");
    }

    #[test]
    fn attempt_tame_guaranteed_does_not_steal_an_already_tamed_pet() {
        let mut ownership = OwnershipData::untamed();
        ownership.owner_pubkey = "local-player-0".to_string();
        let outcome = attempt_tame_guaranteed(&mut ownership, "local-player-1");
        assert_eq!(outcome, TameAttempt::AlreadyTamed);
        assert_eq!(ownership.owner_pubkey, "local-player-0",
            "an already-tamed pet's owner must not change");
    }

    #[test]
    fn own_pets_are_recognised_across_all_ownership_shapes() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);
        assert!(is_players_own_pet(&ecs, wolf, "local-player-0", 0));
        assert!(!is_players_own_pet(&ecs, wolf, "local-player-1", 1));

        let cat = ecs.spawn(());
        let mut cd = crate::companion::CompanionData::untamed();
        cd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(cat, cd);
        assert!(is_players_own_pet(&ecs, cat, "local-player-0", 0));

        let steed = ecs.spawn(());
        let mut hd = crate::horse_ai::HorseData::new();
        hd.kept_by = Some(0);
        let _ = ecs.insert_one(steed, hd);
        assert!(is_players_own_pet(&ecs, steed, "local-player-0", 0));

        let wild = ecs.spawn(());
        assert!(!is_players_own_pet(&ecs, wild, "local-player-0", 0));
    }

    #[test]
    fn own_pets_recognised_for_nostrich_ownership_shape() {
        let mut ecs = hecs::World::new();
        let bird = ecs.spawn(());
        let mut nd = crate::nostrich::NostrichData::untamed();
        nd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(bird, nd);
        assert!(is_players_own_pet(&ecs, bird, "local-player-0", 0));
        assert!(!is_players_own_pet(&ecs, bird, "local-player-1", 1));
    }

    #[test]
    fn owned_pet_entities_collects_wolf_cat_steed_but_not_wild_or_other_player() {
        let mut ecs = hecs::World::new();

        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        let cat = ecs.spawn(());
        let mut cd = crate::companion::CompanionData::untamed();
        cd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(cat, cd);

        let steed = ecs.spawn(());
        let mut hd = crate::horse_ai::HorseData::new();
        hd.kept_by = Some(0);
        let _ = ecs.insert_one(steed, hd);

        // Wild mob — an untamed WolfData (never owned by anyone).
        let wild = ecs.spawn(());
        let _ = ecs.insert_one(wild, crate::wolf::WolfData::untamed());

        // Owned by a different player — must not be recalled by player 0.
        let others_cat = ecs.spawn(());
        let mut oc = crate::companion::CompanionData::untamed();
        oc.ownership.owner_pubkey = "local-player-1".into();
        let _ = ecs.insert_one(others_cat, oc);

        let got = owned_pet_entities(&ecs, "local-player-0", 0);
        assert_eq!(got.len(), 3, "expected exactly wolf+cat+steed, got {got:?}");
        assert!(got.contains(&wolf));
        assert!(got.contains(&cat));
        assert!(got.contains(&steed));
        assert!(!got.contains(&wild));
        assert!(!got.contains(&others_cat));
    }

    #[test]
    fn owned_pet_entities_excludes_perched_parrot() {
        let mut ecs = hecs::World::new();

        let parrot = ecs.spawn(());
        let mut pd = crate::companion::CompanionData::untamed();
        pd.ownership.owner_pubkey = "local-player-0".into();
        pd.state = crate::companion::CompanionState::Perch;
        let _ = ecs.insert_one(parrot, pd);

        let follower = ecs.spawn(());
        let mut fd = crate::companion::CompanionData::untamed();
        fd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(follower, fd);

        let got = owned_pet_entities(&ecs, "local-player-0", 0);
        assert_eq!(got, vec![follower], "perched parrot must be skipped — already on the owner");
    }

    #[test]
    fn exclude_ridden_skips_a_steed_someone_is_currently_riding() {
        // Task 10 — split-screen: player 0 kept-owns a donkey and a wolf.
        // Player 1 is riding the donkey. Player 0 blows the Recall Whistle:
        // the wolf must teleport, the ridden donkey must not (recalling it
        // would yank player 1 across the world via ride-follow).
        let mut ecs = hecs::World::new();

        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        let donkey = ecs.spawn(());
        let mut hd = crate::horse_ai::HorseData::new();
        hd.kept_by = Some(0);
        let _ = ecs.insert_one(donkey, hd);

        let pets = owned_pet_entities(&ecs, "local-player-0", 0);
        assert_eq!(pets.len(), 2, "sanity: both wolf and donkey are owned by player 0");

        // Player 1's PlayerSlot.riding == Some(donkey) in the real game;
        // here that's modelled as the `ridden` slice passed to the filter.
        let ridden = [donkey];
        let got = exclude_ridden(pets, &ridden);

        assert_eq!(got, vec![wolf], "ridden donkey must be excluded, unridden wolf must remain");
    }

    #[test]
    fn exclude_ridden_is_a_no_op_when_nothing_is_ridden() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        let pets = owned_pet_entities(&ecs, "local-player-0", 0);
        let got = exclude_ridden(pets.clone(), &[]);
        assert_eq!(got, pets, "no ridden mounts means nothing gets filtered out");
    }

    #[test]
    fn recall_destination_open_ring_slot_lands_on_the_ring() {
        let world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        let player = glam::Vec3::new(8.0, 64.0, 8.0);
        // Index 0's ring offset is (cos 0, 0, sin 0) * 1.5 = (1.5, 0, 0).
        let dest = recall_destination(player, 0, &world, &registry);
        let expected = player + glam::Vec3::new(1.5, 0.0, 0.0);
        assert!(
            (dest - expected).length() < 1e-5,
            "open cell → ring slot; got {dest:?}, want {expected:?}"
        );
    }

    #[test]
    fn recall_destination_solid_ring_slot_falls_back_to_player_pos() {
        let mut world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        let player = glam::Vec3::new(8.0, 64.0, 8.0);
        // Index 0's ring destination is (9.5, 64.0, 8.0) → cell (9, 64, 8).
        // Wall it off: the pet must land on the player instead of in stone.
        world.set_block(9, 64, 8, crate::block::STONE);
        let dest = recall_destination(player, 0, &world, &registry);
        assert_eq!(dest, player, "solid cell → fall back to the player's own position");
    }

    #[test]
    fn pet_owner_of_reports_owner_across_all_ownership_shapes_regardless_of_asker() {
        let mut ecs = hecs::World::new();

        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);
        assert_eq!(pet_owner_of(&ecs, wolf), Some("local-player-0".to_string()));

        let cat = ecs.spawn(());
        let mut cd = crate::companion::CompanionData::untamed();
        cd.ownership.owner_pubkey = "local-player-2".into();
        let _ = ecs.insert_one(cat, cd);
        assert_eq!(pet_owner_of(&ecs, cat), Some("local-player-2".to_string()));

        let bird = ecs.spawn(());
        let mut nd = crate::nostrich::NostrichData::untamed();
        nd.ownership.owner_pubkey = "local-player-1".into();
        let _ = ecs.insert_one(bird, nd);
        assert_eq!(pet_owner_of(&ecs, bird), Some("local-player-1".to_string()));

        let steed = ecs.spawn(());
        let mut hd = crate::horse_ai::HorseData::new();
        hd.kept_by = Some(3);
        let _ = ecs.insert_one(steed, hd);
        assert_eq!(pet_owner_of(&ecs, steed), Some("local-player-3".to_string()));

        let untamed_wolf = ecs.spawn(());
        let _ = ecs.insert_one(untamed_wolf, crate::wolf::WolfData::untamed());
        assert_eq!(pet_owner_of(&ecs, untamed_wolf), None);

        let wild = ecs.spawn(());
        assert_eq!(pet_owner_of(&ecs, wild), None);
    }

    // Task 6 (bug-hardening review, 2026-07-07) — Pet Bed rescue used to
    // revive EVERY dying owned pet in range, including one the owner just
    // deliberately sneak-killed via the 1C no-friendly-fire bypass, making
    // that bypass unusable within 32 blocks of any bed.

    #[test]
    fn sneak_hit_on_own_pet_marks_deliberate_cull() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        mark_if_deliberate_pet_cull(&mut ecs, true, true, wolf, "local-player-0", 0, 1_000);

        assert!(ecs.get::<&DeliberateCull>(wolf).is_ok(), "sneak-killed own pet must be marked");
    }

    #[test]
    fn non_sneak_hit_on_own_pet_does_not_mark() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        mark_if_deliberate_pet_cull(&mut ecs, false, true, wolf, "local-player-0", 0, 1_000);

        assert!(ecs.get::<&DeliberateCull>(wolf).is_err(), "a non-sneak swing never lands on your own pet (skipped in target selection) — must never mark");
    }

    #[test]
    fn sneak_hit_on_someone_elses_pet_does_not_mark() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-1".into();
        let _ = ecs.insert_one(wolf, wd);

        // player 0 sneak-hits player 1's wolf — not their own pet, so no mark.
        mark_if_deliberate_pet_cull(&mut ecs, true, true, wolf, "local-player-0", 0, 1_000);

        assert!(ecs.get::<&DeliberateCull>(wolf).is_err());
    }

    #[test]
    fn sneak_swing_that_missed_does_not_mark() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        mark_if_deliberate_pet_cull(&mut ecs, true, false, wolf, "local-player-0", 0, 1_000);

        assert!(ecs.get::<&DeliberateCull>(wolf).is_err(), "a miss must not mark");
    }

    #[test]
    fn deliberate_sneak_kill_skips_pet_bed_rescue() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        // The owner's sneak swing lands the killing blow on their own pet.
        mark_if_deliberate_pet_cull(&mut ecs, true, true, wolf, "local-player-0", 0, 1_000);

        // Death sweep runs the very same tick — well within the TTL.
        assert!(
            !eligible_for_pet_bed_rescue(&ecs, wolf, 1_000),
            "a deliberately sneak-killed pet must NOT be rescued"
        );
    }

    #[test]
    fn hostile_kill_still_eligible_for_pet_bed_rescue() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        // No sneak-cull mark — this pet died to a hostile mob, not its owner.
        assert!(
            eligible_for_pet_bed_rescue(&ecs, wolf, 1_000),
            "a pet killed by a hostile must still be rescued as before"
        );
    }

    #[test]
    fn unowned_mob_is_never_eligible_for_pet_bed_rescue() {
        let mut ecs = hecs::World::new();
        let wild = ecs.spawn(());
        assert!(!eligible_for_pet_bed_rescue(&ecs, wild, 1_000));
    }

    // Task 6 follow-up (bug-hardening review, 2026-07-07) — the mark never
    // expired: one accidental non-lethal sneak-tap silently disabled Pet Bed
    // rescue for that pet for the rest of the session. It must auto-clear
    // once stale (CULL_MARK_TTL_TICKS = 100 ticks = 5s at 20 TPS).
    #[test]
    fn stale_cull_mark_no_longer_blocks_pet_bed_rescue() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        // Accidental sneak-tap at tick 1_000 — pet survives (this call site
        // only ever fires on a landed hit, but the mark itself doesn't know
        // or care whether that hit was lethal).
        mark_if_deliberate_pet_cull(&mut ecs, true, true, wolf, "local-player-0", 0, 1_000);

        // Pet dies much later to something else entirely — well past the TTL.
        let later_tick = 1_000 + CULL_MARK_TTL_TICKS + 1;
        assert!(
            eligible_for_pet_bed_rescue(&ecs, wolf, later_tick),
            "a stale cull mark (older than the TTL) must not block rescue"
        );
    }

    #[test]
    fn cull_mark_exactly_at_ttl_boundary_still_blocks_rescue() {
        let mut ecs = hecs::World::new();
        let wolf = ecs.spawn(());
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = "local-player-0".into();
        let _ = ecs.insert_one(wolf, wd);

        mark_if_deliberate_pet_cull(&mut ecs, true, true, wolf, "local-player-0", 0, 1_000);

        // Exactly at the TTL boundary — still fresh (inclusive).
        assert!(
            !eligible_for_pet_bed_rescue(&ecs, wolf, 1_000 + CULL_MARK_TTL_TICKS),
            "a mark exactly at the TTL boundary must still count as fresh"
        );
    }
}

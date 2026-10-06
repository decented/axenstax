//! Combat system — health, melee damage, knockback, death.
//!
//! Spec 05 Section 6: Health, damage types, melee combat, death/respawn.
//! This implementation covers fist combat, mob contact damage, and basic death.

use glam::Vec3;
use crate::entity::{Hitbox, MobKind, Position, Velocity};
use crate::mob::{self, MobCategory};

/// Attack reach in blocks (Spec 05 Section 6.4).
pub const ATTACK_REACH: f32 = 3.0;
/// Attack cooldown in ticks (0.5 seconds at 20 TPS).
pub const ATTACK_COOLDOWN: u32 = 10;
/// Spec 19 — per-player rolling window during which a swing on a villager
/// actually damages instead of just warning. 30 s @ 20 TPS = 600 ticks.
pub const VILLAGER_GRACE_TICKS: u64 = 600;
/// Fist base damage. `crafting.rs`'s per-tool damage table hardcodes the same
/// `1.0` literal directly (its `_ => 1.0` fallback) rather than referencing
/// this constant, so it has no consumer.
#[allow(dead_code)]
const FIST_DAMAGE: f32 = 1.0;
/// Base knockback impulse in blocks.
const KNOCKBACK_BASE: f32 = 0.4;
/// Extra knockback when sprinting.
const KNOCKBACK_SPRINT: f32 = 0.4;
/// Critical hit multiplier (when falling).
const CRIT_MULTIPLIER: f32 = 1.5;
/// #23 — sweep-attack: a melee swing also hits other mobs in the swing arc for
/// this fraction of the primary damage (Minecraft-style sweep). PvP balance is
/// multiplayer-gated; this is the vs-mob feel.
const SWEEP_DAMAGE_FRACTION: f32 = 0.4;
/// #23 — swing-arc cone: a target counts if `to_target·look_dir >= this`
/// (≈ within 60° of where you're looking), the same cone as the primary pick.
const SWING_MIN_DOT: f32 = 0.5;
/// Baseline hostile-mob contact damage per hit.
pub(crate) const HOSTILE_MELEE_DAMAGE: f32 = 3.0;
/// Baseline hostile-mob attack cooldown (1 second).
#[allow(dead_code)]
const HOSTILE_MELEE_COOLDOWN: u32 = 20;
/// Baseline hostile-mob attack range (blocks).
const HOSTILE_MELEE_RANGE: f32 = 1.5;
/// Damage flash duration in ticks.
pub const DAMAGE_FLASH_TICKS: u32 = 6;
/// W2 — the PLAYER's hurt flash (drives the red screen-edge vignette): 8 ticks
/// = 0.4 s at 20 TPS. Separate from [`DAMAGE_FLASH_TICKS`], which the mob
/// model tint reads.
pub const PLAYER_HURT_FLASH_TICKS: u32 = 8;
/// Invincibility frames after being hit (ticks).
const INVINCIBILITY_TICKS: u32 = 10;

// --- Health component for mobs ---

/// ECS component stamped on a mob the last time a player damaged it.
/// Read by `despawn_dead` so kill-attribution can credit the actual
/// last-attacker rather than the nearest-player proximity fallback.
/// Absent on mobs no player has hit (e.g. a Cow killed by a Hyena);
/// the caller falls back to nearest-player in that case.
#[derive(Clone, Copy, Debug)]
pub struct LastAttacker(pub usize);

/// Health component for entities.
pub struct Health {
    pub current: f32,
    pub max: f32,
    /// Ticks remaining on damage flash (red tint).
    pub flash_timer: u32,
    /// Ticks remaining on invincibility after being hit.
    pub invincible_timer: u32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Self {
            current: max,
            max,
            flash_timer: 0,
            invincible_timer: 0,
        }
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }

    pub fn take_damage(&mut self, amount: f32) -> bool {
        if self.invincible_timer > 0 {
            return false;
        }
        self.current = (self.current - amount).max(0.0);
        self.flash_timer = DAMAGE_FLASH_TICKS;
        self.invincible_timer = INVINCIBILITY_TICKS;
        true
    }

    pub fn tick(&mut self) {
        if self.flash_timer > 0 {
            self.flash_timer -= 1;
        }
        if self.invincible_timer > 0 {
            self.invincible_timer -= 1;
        }
    }

    pub fn is_flashing(&self) -> bool {
        self.flash_timer > 0
    }
}

// --- Player combat state ---

pub struct PlayerCombat {
    pub health: f32,
    pub max_health: f32,
    pub attack_cooldown: u32,
    pub flash_timer: u32,
    pub invincible_timer: u32,
    /// Damage of the hit that started the current invulnerability window
    /// (post-armour). Minecraft rule: a hit arriving inside the window only
    /// applies the amount by which it exceeds this. Meaningful only while
    /// `invincible_timer > 0`.
    pub last_hit_damage: f32,
    /// Dead until respawned. Nothing respawns a player on a timer: the client
    /// waits for `respawn_requested` (W2 owner decision 2026-10-06 — the death
    /// screen stays up until the player chooses Respawn), and a server holds a
    /// joiner's copy dead until that joiner sends `PacketType::Respawn`
    /// (MP-A3, `GameServer::respawn_player`).
    pub dead: bool,
    /// Set by the death screen (Respawn button / Enter / pad A) via
    /// [`PlayerCombat::request_respawn`]; the client's death loop respawns the
    /// player on the next tick and `respawn()` clears it.
    pub respawn_requested: bool,
    /// One-shot flag: set on the tick the player transitions to dead, cleared
    /// after the death-side handler (e.g. inventory drop) consumes it.
    pub just_died: bool,
    /// Active poison timer (Wave 15). Drains health every
    /// POISON_DAMAGE_INTERVAL_TICKS while non-zero. Floors health at 0.5
    /// (poison can't kill — Minecraft parity).
    pub poison_ticks: u32,
    /// Hunger level (Wave 24). 0-20, like MC. Drains over time + every
    /// passive heart regen costs hunger. Eating restores it.
    pub hunger: u8,
    pub max_hunger: u8,
    /// Tick counter for the hunger drain step. When it hits
    /// HUNGER_DRAIN_INTERVAL_TICKS, decrement `hunger` by 1.
    pub hunger_drain_ticks: u32,
    /// Tick counter for passive health regeneration. When `hunger >=
    /// HUNGER_THRESHOLD_FOR_REGEN` and below max health, this counter
    /// climbs; at HEALTH_REGEN_INTERVAL_TICKS, restore 1 HP + cost 1
    /// hunger (MC charges hunger per regen tick).
    pub regen_ticks: u32,
    /// Tick counter for starvation damage. When `hunger == 0`, this
    /// counter climbs; at STARVATION_INTERVAL_TICKS, drain 1 HP (floored
    /// at `starvation_floor`).
    pub starvation_ticks: u32,
    /// W2 — starvation never takes health below this. Set from the difficulty
    /// table (`survival::Difficulty::rules().starvation_floor`) by the caller
    /// before each `tick`; defaults to Normal (1 HP). `0.0` = starvation kills.
    pub starvation_floor: f32,
    /// W2 — what last hurt this player (death-screen cause line).
    pub last_damage: crate::survival::DamageCause,
    /// W2 — air supply / drowning state. Transient; reset on respawn.
    pub breath: crate::survival::Breath,
}

/// How long between hunger ticks (Wave 24). 600 ticks = 30 seconds at
/// 20 TPS — gentler than MC's idle drain so the kid doesn't constantly
/// chase food.
pub const HUNGER_DRAIN_INTERVAL_TICKS: u32 = 600;
/// Hunger level above which passive health regen kicks in (Wave 24).
/// MC equivalent threshold.
pub const HUNGER_THRESHOLD_FOR_REGEN: u8 = 18;
/// Ticks between passive +1 HP regen events (4 seconds at 20 TPS).
pub const HEALTH_REGEN_INTERVAL_TICKS: u32 = 80;
/// Ticks between starvation -1 HP events when hunger is empty.
pub const STARVATION_INTERVAL_TICKS: u32 = 80;

/// Default poison duration (ticks) used to exercise the apply_poison mechanic.
#[cfg_attr(not(test), allow(dead_code))]
pub const POISON_DURATION_TICKS: u32 = 60; // 3s @ 20 TPS
/// Damage tick interval while poisoned.
pub const POISON_DAMAGE_INTERVAL_TICKS: u32 = 20; // 1s
/// HP lost per poison damage tick.
pub const POISON_DAMAGE_PER_TICK: f32 = 1.0;
/// Health floor under poison-only damage. Other sources can still kill.
pub const POISON_HEALTH_FLOOR: f32 = 0.5;

impl PlayerCombat {
    pub fn new() -> Self {
        Self {
            health: 20.0,
            max_health: 20.0,
            attack_cooldown: 0,
            flash_timer: 0,
            invincible_timer: 0,
            last_hit_damage: 0.0,
            dead: false,
            respawn_requested: false,
            just_died: false,
            poison_ticks: 0,
            hunger: 20,
            max_hunger: 20,
            hunger_drain_ticks: 0,
            regen_ticks: 0,
            starvation_ticks: 0,
            starvation_floor: crate::survival::Difficulty::Normal.rules().starvation_floor,
            last_damage: crate::survival::DamageCause::Generic,
            breath: crate::survival::Breath::FULL,
        }
    }

    /// Refill hunger by `amount`, clamped at `max_hunger`. Returns the
    /// amount actually added.
    pub fn feed(&mut self, amount: u8) -> u8 {
        if self.dead {
            return 0;
        }
        let before = self.hunger;
        self.hunger = (self.hunger.saturating_add(amount)).min(self.max_hunger);
        self.hunger - before
    }

    pub fn is_well_fed(&self) -> bool {
        self.hunger >= HUNGER_THRESHOLD_FOR_REGEN
    }

    /// One tick of everything: the hit/attack timers, then the metabolism.
    pub fn tick(&mut self) {
        self.tick_timers();
        self.tick_metabolism();
    }

    /// One tick of the hit and attack timers only — what a server runs for a
    /// joiner's body (MP-D2a): its metabolism (hunger, regen, starvation,
    /// poison) is its own client's, reported as `InputPacket.health_delta`.
    pub fn tick_timers(&mut self) {
        if self.attack_cooldown > 0 {
            self.attack_cooldown -= 1;
        }
        if self.flash_timer > 0 {
            self.flash_timer -= 1;
        }
        if self.invincible_timer > 0 {
            self.invincible_timer -= 1;
        }
    }

    /// One tick of poison, hunger drain, natural regen and starvation.
    pub fn tick_metabolism(&mut self) {
        if self.poison_ticks > 0 {
            self.poison_ticks -= 1;
            // Damage on every interval boundary. Drop health but never below
            // POISON_HEALTH_FLOOR — poison alone can't kill.
            if !self.dead && self.poison_ticks.is_multiple_of(POISON_DAMAGE_INTERVAL_TICKS) {
                self.health = (self.health - POISON_DAMAGE_PER_TICK).max(POISON_HEALTH_FLOOR);
            }
        }

        // --- Hunger drain (Wave 24) ---
        if !self.dead {
            self.hunger_drain_ticks = self.hunger_drain_ticks.saturating_add(1);
            if self.hunger_drain_ticks >= HUNGER_DRAIN_INTERVAL_TICKS {
                self.hunger_drain_ticks = 0;
                if self.hunger > 0 {
                    self.hunger -= 1;
                }
            }
        }

        // --- Passive health regen (Wave 24) ---
        if !self.dead && !self.is_poisoned()
            && self.is_well_fed()
            && self.health < self.max_health
        {
            self.regen_ticks = self.regen_ticks.saturating_add(1);
            if self.regen_ticks >= HEALTH_REGEN_INTERVAL_TICKS {
                self.regen_ticks = 0;
                self.health = (self.health + 1.0).min(self.max_health);
                // Each regen pulse costs hunger — matches MC's saturation
                // model loosely (no separate saturation buffer here).
                if self.hunger > 0 { self.hunger -= 1; }
            }
        } else {
            self.regen_ticks = 0;
        }

        // --- Starvation damage (Wave 24; W2 difficulty floor) ---
        // The floor comes from the difficulty table: Easy 10 HP, Normal 1 HP,
        // Hard 0 (starvation kills), Peaceful the old half heart.
        if !self.dead && self.hunger == 0 && self.health > self.starvation_floor {
            self.starvation_ticks = self.starvation_ticks.saturating_add(1);
            if self.starvation_ticks >= STARVATION_INTERVAL_TICKS {
                self.starvation_ticks = 0;
                let amount = (self.health - self.starvation_floor).min(1.0);
                if amount > 0.0 {
                    self.take_damage_from(amount, crate::survival::DamageCause::Starvation);
                }
            }
        } else {
            self.starvation_ticks = 0;
        }
    }

    pub fn can_attack(&self) -> bool {
        self.attack_cooldown == 0 && !self.dead
    }

    pub fn take_damage(&mut self, amount: f32) -> bool {
        self.take_damage_from(amount, crate::survival::DamageCause::Generic)
    }

    /// [`take_damage`](Self::take_damage), recording `cause` as the last
    /// damage (the death-screen line) when the hit lands. No armour here —
    /// armour-reduced sources go through
    /// `PlayerSlot::take_damage_with_armour_from`.
    pub fn take_damage_from(&mut self, amount: f32, cause: crate::survival::DamageCause) -> bool {
        if self.dead {
            return false;
        }
        let applied = if self.invincible_timer > 0 {
            // Minecraft rule: inside the hit-invulnerability window only the
            // amount exceeding the hit that started it applies (if larger);
            // otherwise the hit is ignored. The window is NOT restarted.
            if amount <= self.last_hit_damage {
                return false;
            }
            let extra = amount - self.last_hit_damage;
            self.last_hit_damage = amount;
            extra
        } else {
            self.last_hit_damage = amount;
            self.invincible_timer = INVINCIBILITY_TICKS;
            amount
        };
        self.last_damage = cause;
        self.health = (self.health - applied).max(0.0);
        self.flash_timer = PLAYER_HURT_FLASH_TICKS;
        if self.health <= 0.0 {
            self.mark_dead();
        }
        true
    }

    /// MP-D2a — apply a joiner's reported change to its own health (the
    /// sources its client still owns: eating, regen, poison, starvation,
    /// sleeping, `/heal`; `InputPacket.health_delta`). A heal is clamped to
    /// max health. A loss takes no hit-invulnerability window and records no
    /// cause (it is not a hit). Nothing for the dead: only a Respawn revives,
    /// and a report never kills twice. Returns whether this report killed
    /// the player — its client already knows (it reported the loss), so the
    /// caller clears the `just_died` one-shot rather than echo a `Died`.
    pub fn apply_reported_change(&mut self, delta: f32) -> bool {
        if self.dead || !delta.is_finite() || delta == 0.0 {
            return false;
        }
        self.health = (self.health + delta).clamp(0.0, self.max_health);
        if self.health <= 0.0 {
            self.mark_dead();
            return true;
        }
        false
    }

    /// Die now, whatever the health: the death a joiner's client takes when
    /// the server tells it its body died (`PlayerEventType::Died`), and the
    /// death a server records when a joiner's input reports zero health.
    /// Records `cause` for the death-screen line. No-op when already dead, so
    /// the `just_died` one-shot fires once per death.
    pub fn die(&mut self, cause: crate::survival::DamageCause) {
        if self.dead {
            return;
        }
        self.last_damage = cause;
        self.health = 0.0;
        self.mark_dead();
    }

    /// The death transition, shared by a lethal hit and [`Self::die`].
    fn mark_dead(&mut self) {
        self.dead = true;
        self.respawn_requested = false;
        self.just_died = true;
    }

    pub fn respawn(&mut self) {
        self.health = self.max_health;
        self.dead = false;
        self.flash_timer = 0;
        self.invincible_timer = 0;
        self.last_hit_damage = 0.0;
        self.respawn_requested = false;
        // Respawn restores hunger too (MC parity).
        self.hunger = self.max_hunger;
        self.hunger_drain_ticks = 0;
        self.regen_ticks = 0;
        self.starvation_ticks = 0;
        self.breath = crate::survival::Breath::FULL;
        // just_died stays false — only the death transition sets it.
    }

    /// The player chose Respawn on the death screen. No-op unless dead (a stale
    /// press can't queue a respawn for a later death). Returns whether the
    /// request was taken.
    pub fn request_respawn(&mut self) -> bool {
        if !self.dead {
            return false;
        }
        self.respawn_requested = true;
        true
    }

    /// Whether the client death loop should respawn this player now.
    pub fn should_respawn(&self) -> bool {
        self.dead && self.respawn_requested
    }

    /// Apply (or refresh) a poison effect for `ticks` duration. No-op when
    /// dead. If the player is already poisoned, takes the longer of the two.
    pub fn apply_poison(&mut self, ticks: u32) {
        if self.dead {
            return;
        }
        self.poison_ticks = self.poison_ticks.max(ticks);
    }

    /// Whether the player is currently under poison effect.
    pub fn is_poisoned(&self) -> bool {
        self.poison_ticks > 0
    }

    /// Restore health up to max_health. Returns the actual amount restored.
    /// No-op when dead — eat in time, not after.
    pub fn heal(&mut self, amount: f32) -> f32 {
        if self.dead || amount <= 0.0 {
            return 0.0;
        }
        let before = self.health;
        self.health = (self.health + amount).min(self.max_health);
        self.health - before
    }

    /// W2 — drives the red hurt vignette (`hud_ui::draw_hurt_vignette`).
    pub fn is_flashing(&self) -> bool {
        self.flash_timer > 0
    }

    /// W2 — the hurt vignette's alpha this frame (0 when not flashing).
    pub fn hurt_vignette_alpha(&self) -> f32 {
        if !self.is_flashing() {
            return 0.0;
        }
        crate::survival::hurt_vignette_alpha(self.flash_timer, PLAYER_HURT_FLASH_TICKS)
    }
}

// --- Player attacks entity ---

/// #23 — is a target within the swing's reach **and** inside the forward cone?
/// `to_target` is the vector from the player to the target's centre; `look_dir`
/// is the (normalised) look direction. Pure geometry, shared by the primary
/// target pick and the sweep.
pub fn in_swing_arc(to_target: Vec3, look_dir: Vec3, reach: f32, min_dot: f32) -> bool {
    let dist = to_target.length();
    dist > 0.0 && dist <= reach && to_target.normalize_or_zero().dot(look_dir) >= min_dot
}

/// Peek at which entity a player would attack from `player_pos` looking in
/// `look_dir`. Used by callers that need to gate the attack (Spec 19 villager
/// anti-grief warning, future quest objectives) before damage is applied, and
/// by the many right-click interaction pickers (mount/feed/lead/pack/tame/...)
/// that deliberately WANT to be able to pick the player's own tamed pet/steed
/// (that's the whole point of those interactions). No exclusion — see
/// [`find_attack_target_for_swing`] for the melee-swing variant that skips
/// the attacker's own pets.
/// Returns the entity id and its optional `MobType` (None for non-mob entities).
pub fn find_attack_target(
    ecs: &hecs::World,
    player_pos: Vec3,
    look_dir: Vec3,
) -> Option<(hecs::Entity, Option<crate::mob::MobType>)> {
    find_attack_target_impl(ecs, player_pos, look_dir, None)
}

/// Task 8 (bug-hardening, 2026-07-07) — same cone/reach pick as
/// [`find_attack_target`], but when `sneaking` is false it skips any entity
/// that is `player_pubkey`/`player_slot`'s own tamed pet or steed, falling
/// through to the next-nearest candidate in the cone instead. This is the
/// melee swing's target peek: previously the own-pet check ran AFTER
/// selection (`own_pet_shielded` in `game_loop.rs`'s melee arm) and, if the
/// nearest cone entity was the attacker's own pet, cancelled the whole swing
/// — with wolf-assist actively driving the wolf onto the real target, this
/// meant clicks silently fell through to block-breaking behind the enemy
/// while the player took damage. Moving the exclusion into selection means a
/// hostile standing behind your own body-blocking pet is now the target, not
/// a mined block. Sneaking is the deliberate-hit signal (mirrors the sweep
/// gate and `tameable::mark_if_deliberate_pet_cull`), so it keeps pets
/// targetable exactly as before.
pub fn find_attack_target_for_swing(
    ecs: &hecs::World,
    player_pos: Vec3,
    look_dir: Vec3,
    player_pubkey: &str,
    player_slot: usize,
    sneaking: bool,
) -> Option<(hecs::Entity, Option<crate::mob::MobType>)> {
    find_attack_target_impl(
        ecs,
        player_pos,
        look_dir,
        (!sneaking).then_some((player_pubkey, player_slot)),
    )
}

fn find_attack_target_impl(
    ecs: &hecs::World,
    player_pos: Vec3,
    look_dir: Vec3,
    exclude_own_pet: Option<(&str, usize)>,
) -> Option<(hecs::Entity, Option<crate::mob::MobType>)> {
    let mut best: Option<(hecs::Entity, f32)> = None;
    for (id, (pos, hitbox)) in ecs.query::<(&Position, &Hitbox)>().iter() {
        let to_entity = pos.0 + Vec3::new(0.0, hitbox.height * 0.5, 0.0) - player_pos;
        let dist = to_entity.length();
        if dist > ATTACK_REACH {
            continue;
        }
        let dot = to_entity.normalize_or_zero().dot(look_dir);
        if dot < 0.5 {
            continue;
        }
        // Task 9 (bug-hardening, 2026-07-07) — a perched parrot is cosmetic,
        // pinned to the owner's shoulder right at the camera; it must never
        // be a target for either selector, sneaking or not. Unlike the
        // `exclude_own_pet` gate below (swing-only, sneak-bypassable), this
        // applies unconditionally to both `find_attack_target` (the
        // unfiltered picker the mount/feed/lead/pack/tame interactions use)
        // and `find_attack_target_for_swing`.
        if let Ok(data) = ecs.get::<&crate::companion::CompanionData>(id)
            && data.state == crate::companion::CompanionState::Perch
        {
            continue;
        }
        if let Some((pubkey, slot)) = exclude_own_pet
            && crate::tameable::is_players_own_pet(ecs, id, pubkey, slot)
        {
            continue;
        }
        if best.is_none() || dist < best.unwrap().1 {
            best = Some((id, dist));
        }
    }
    best.map(|(id, _)| {
        let kind = ecs.get::<&MobKind>(id).map(|k| k.0).ok();
        (id, kind)
    })
}

/// Try to attack the entity closest to the player's crosshair.
/// Returns true if an entity was hit.
///
/// `attacker_pidx` is the player slot index of the attacker, stamped
/// on the target as a `LastAttacker` component so kill-attribution
/// can credit the actual last-attacker on death (rather than the
/// nearest-player proximity fallback).
///
/// `sneaking` is the attacker's live sneak state — 1C no-friendly-fire:
/// the #23 sweep skips the attacker's own tamed pets/steeds in the arc
/// unless sneaking (a deliberate hit). Task 8 (bug-hardening, 2026-07-07)
/// moved the PRIMARY target's own-pet gate INTO selection
/// ([`find_attack_target_for_swing`]) so a body-blocking own pet is skipped
/// in favour of the next-nearest cone candidate rather than cancelling the
/// whole swing (`game_loop`'s melee arm peeks with the same function so its
/// villager-grace / wolf-assist / pet-cull logic sees the same target).
pub fn player_attack(
    ecs: &mut hecs::World,
    player_pos: Vec3,
    look_dir: Vec3,
    on_ground: bool,
    sprinting: bool,
    sneaking: bool,
    combat: &mut PlayerCombat,
    base_damage: f32,
    attacker_pidx: usize,
) -> bool {
    if !combat.can_attack() {
        return false;
    }

    let attacker_key = format!("local-player-{attacker_pidx}");
    let Some((target_id, _kind)) = find_attack_target_for_swing(
        ecs,
        player_pos,
        look_dir,
        &attacker_key,
        attacker_pidx,
        sneaking,
    ) else {
        return false;
    };

    // Calculate damage
    let mut damage = base_damage;
    let is_crit = !on_ground;
    if is_crit {
        damage *= CRIT_MULTIPLIER;
    }

    // Cooldown scaling (always full for now since we check can_attack)
    combat.attack_cooldown = ATTACK_COOLDOWN;

    // Apply damage and knockback
    let mut damage_landed = false;
    if let Ok((pos, vel, health)) = ecs.query_one_mut::<(&Position, &mut Velocity, &mut Health)>(target_id)
        && health.take_damage(damage) {
            damage_landed = true;
            // Knockback direction: from player toward entity
            let kb_dir = (pos.0 - player_pos).normalize_or_zero();
            let kb_dir = Vec3::new(kb_dir.x, 0.0, kb_dir.z).normalize_or_zero();
            let mut kb = KNOCKBACK_BASE;
            if sprinting {
                kb += KNOCKBACK_SPRINT;
            }
            vel.0.x += kb_dir.x * kb;
            vel.0.y += 0.3; // Small upward pop
            vel.0.z += kb_dir.z * kb;
        }
    // Stamp/refresh LastAttacker so despawn_dead can credit the kill
    // to this player on the next tick. Only stamp on actual damage
    // (i_frames + dead targets skip). Insert is idempotent — replaces
    // any prior LastAttacker so the most recent damaging hit wins.
    if damage_landed {
        let _ = ecs.insert_one(target_id, LastAttacker(attacker_pidx));
        spook_if_prey(ecs, target_id);
        notify_hit_bear_or_hyena(ecs, target_id, attacker_pidx);
    }

    // #23 — sweep attack: every OTHER entity in the swing arc takes reduced
    // damage + a light knockback (the Minecraft-style sweep). Collect the ids
    // under an immutable query, then apply mutably (borrow-checker dance).
    if damage_landed {
        let sweep_dmg = damage * SWEEP_DAMAGE_FRACTION;
        let others: Vec<hecs::Entity> = ecs
            .query::<(&Position, &Hitbox)>()
            .iter()
            .filter(|(id, (pos, hb))| {
                *id != target_id
                    && in_swing_arc(
                        pos.0 + Vec3::new(0.0, hb.height * 0.5, 0.0) - player_pos,
                        look_dir,
                        ATTACK_REACH,
                        SWING_MIN_DOT,
                    )
            })
            .map(|(id, _)| id)
            .collect();
        for oid in others {
            // 1C no-friendly-fire — the sweep passes over the attacker's
            // own tamed pet/steed (no damage, no LastAttacker) unless
            // they're sneaking. Mirrors the primary-target selection gate
            // above (`find_attack_target_for_swing`).
            if !sneaking
                && crate::tameable::is_players_own_pet(ecs, oid, &attacker_key, attacker_pidx)
            {
                continue;
            }
            let mut landed = false;
            if let Ok((pos, vel, health)) =
                ecs.query_one_mut::<(&Position, &mut Velocity, &mut Health)>(oid)
                && health.take_damage(sweep_dmg) {
                    landed = true;
                    let kb_dir = (pos.0 - player_pos).normalize_or_zero();
                    vel.0.x += kb_dir.x * KNOCKBACK_BASE * 0.5;
                    vel.0.z += kb_dir.z * KNOCKBACK_BASE * 0.5;
                }
            if landed {
                let _ = ecs.insert_one(oid, LastAttacker(attacker_pidx));
                spook_if_prey(ecs, oid);
                notify_hit_bear_or_hyena(ecs, oid, attacker_pidx);
            }
        }
    }

    true
}

/// P2 — when a struck entity is a prey animal ([`mob::flees_when_attacked`]),
/// flip its AI into [`AiState::Flee`] so it bolts away from the player. Hostiles
/// (which retaliate), village defenders, the tamed Wolf companion, and NPCs are
/// left untouched. No-op for anything without a `MobKind` + `MobAi`.
fn spook_if_prey(ecs: &mut hecs::World, id: hecs::Entity) {
    let is_prey = ecs
        .get::<&crate::entity::MobKind>(id)
        .map(|k| crate::mob::flees_when_attacked(k.0))
        .unwrap_or(false);
    if is_prey
        && let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(id) {
            ai.state = crate::mob_ai::AiState::Flee {
                timer: crate::mob_ai::FLEE_TICKS,
            };
        }
}

/// Task 13 (bug-hardening, 2026-07-07) — a player hit landing on a Bear or
/// Hyena provokes it: `bear_ai`/`hyena_ai::on_hit_by_player` flip the
/// species AI into its `Aggro` state, which was dead code before this fix
/// (the state existed and ticked, but nothing ever called the function that
/// reaches it). Mirrors `spook_if_prey` above — a per-hit species-reaction
/// lookup shared by the melee path here and the arrow-hit path in
/// `entity::tick_projectiles`. No-op for any other species.
pub(crate) fn notify_hit_bear_or_hyena(ecs: &mut hecs::World, id: hecs::Entity, attacker_pidx: usize) {
    if let Ok(mut bear) = ecs.get::<&mut crate::bear_ai::BearData>(id) {
        crate::bear_ai::on_hit_by_player(&mut bear, attacker_pidx);
        return;
    }
    if let Ok(mut hyena) = ecs.get::<&mut crate::hyena_ai::HyenaData>(id) {
        crate::hyena_ai::on_hit_by_player(&mut hyena, attacker_pidx);
    }
}

/// Remove dead entities from the ECS world. Returns
/// `(MobType, position, Option<attacker_pidx>)` for every mob that
/// died this tick, so the caller can:
///   1. spawn drop entities via `mob::drops_for` + `entity::spawn_item`,
///   2. credit the kill to the last-attacker pidx (or fall back to
///      nearest-player proximity if no player ever damaged the mob).
///
/// Non-mob dead entities (anything with `Health` but no `MobKind`) are
/// still despawned but produce no tuples.
pub fn despawn_dead(ecs: &mut hecs::World) -> Vec<(crate::mob::MobType, Vec3, Option<usize>)> {
    // First pass: find dead entities, capture (MobType, Vec3, attacker)
    // where present.
    let mut dead_with_drops: Vec<(hecs::Entity, crate::mob::MobType, Vec3, Option<usize>)> =
        Vec::new();
    let mut dead_other: Vec<hecs::Entity> = Vec::new();
    for (id, h) in ecs.query::<&Health>().iter() {
        if !h.is_dead() {
            continue;
        }
        // Try to fetch MobKind + Position + LastAttacker alongside.
        let kind = ecs.get::<&MobKind>(id).map(|k| k.0).ok();
        let pos = ecs.get::<&Position>(id).map(|p| p.0).ok();
        let attacker = ecs.get::<&LastAttacker>(id).map(|la| la.0).ok();
        match (kind, pos) {
            (Some(k), Some(p)) => dead_with_drops.push((id, k, p, attacker)),
            _ => dead_other.push(id),
        }
    }

    let mut out: Vec<(crate::mob::MobType, Vec3, Option<usize>)> =
        Vec::with_capacity(dead_with_drops.len());
    for (id, k, p, attacker) in dead_with_drops {
        out.push((k, p, attacker));
        let _ = ecs.despawn(id);
    }
    for id in dead_other {
        let _ = ecs.despawn(id);
    }
    out
}

/// Hostile mob contact damage on one player body — every
/// `MobCategory::Hostile` mob within melee reach of `player_pos` hits for the
/// baseline melee damage, scaled by the difficulty table (W2), reduced by
/// `armour_points` (Spec 28e), and knocks the body back.
///
/// The one rule for both sides (MP-D2a): the client runs it on its local
/// players through [`tick_mob_attacks`] (which also wears their armour), and
/// `GameServer::tick` runs it on every joiner's server-held body, with the
/// armour points the joiner's input reports. Returns the mobs whose hit
/// LANDED (not merely in reach — the i-frame window can refuse one).
pub fn hostile_melee_tick(
    ecs: &hecs::World,
    player_pos: Vec3,
    player_vel: &mut Vec3,
    combat: &mut PlayerCombat,
    armour_points: u8,
    difficulty: crate::survival::Difficulty,
) -> Vec<hecs::Entity> {
    let mut landed: Vec<hecs::Entity> = Vec::new();
    if combat.dead {
        return landed;
    }
    let damage = crate::armour::damage_after_armour(
        crate::survival::scale_mob_damage(HOSTILE_MELEE_DAMAGE, difficulty),
        armour_points,
    );
    for (id, (pos, kind, _hitbox)) in ecs.query::<(&Position, &MobKind, &Hitbox)>().iter() {
        if mob::mob_def(kind.0).category != MobCategory::Hostile {
            continue;
        }
        let to_player = player_pos - pos.0;
        let horiz_dist = Vec3::new(to_player.x, 0.0, to_player.z).length();
        let vert_overlap = to_player.y >= 0.0 && to_player.y < 1.8; // Player height
        if horiz_dist < HOSTILE_MELEE_RANGE
            && vert_overlap
            && combat.take_damage_from(damage, crate::survival::DamageCause::Mob(kind.0))
        {
            // Knockback away from the attacker.
            let kb_dir = Vec3::new(to_player.x, 0.0, to_player.z).normalize_or_zero();
            player_vel.x += kb_dir.x * KNOCKBACK_BASE;
            player_vel.y += 0.3;
            player_vel.z += kb_dir.z * KNOCKBACK_BASE;
            landed.push(id);
        }
    }
    landed
}

/// [`hostile_melee_tick`] on a local player: their equipped armour soaks the
/// hit and wears one durability per landed hit (Spec 28e).
///
/// Returns the mobs that actually LANDED damage this call so the caller can
/// feed downstream "the player was attacked by M" events — Task 8b uses
/// this to rally the player's tamed wolves into their revenge pivot.
pub fn tick_mob_attacks(
    ecs: &hecs::World,
    slot: &mut crate::player_slot::PlayerSlot,
    difficulty: crate::survival::Difficulty,
) -> Vec<hecs::Entity> {
    let points = slot.total_armour_points();
    let pos = slot.player.pos;
    let landed =
        hostile_melee_tick(ecs, pos, &mut slot.player.velocity, &mut slot.combat, points, difficulty);
    for _ in &landed {
        slot.wear_armour();
    }
    landed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{self, MobKind, Position};
    use crate::mob::MobType;

    // --- Health ---

    #[test]
    fn health_take_damage_reduces_and_flags_flash() {
        let mut h = Health::new(20.0);
        let hit = h.take_damage(5.0);
        assert!(hit);
        assert_eq!(h.current, 15.0);
        assert!(h.is_flashing());
        assert!(!h.is_dead());
    }

    #[test]
    fn health_invulnerability_blocks_second_hit() {
        let mut h = Health::new(20.0);
        assert!(h.take_damage(3.0));
        // While invulnerable, a follow-up hit must be ignored.
        assert!(!h.take_damage(5.0));
        assert_eq!(h.current, 17.0);
    }

    // --- Player i-frames: Minecraft rule (W2 review) ---

    #[test]
    fn iframe_larger_hit_applies_only_the_excess() {
        // 4 dmg mob hit, then a 10 dmg fall inside the window: only the 6
        // beyond the first hit applies (total 10 lost).
        let mut c = PlayerCombat::new();
        assert!(c.take_damage_from(4.0, crate::survival::DamageCause::Generic));
        assert!((c.health - 16.0).abs() < 1e-4);
        assert!(c.take_damage_from(10.0, crate::survival::DamageCause::Fall));
        assert!((c.health - 10.0).abs() < 1e-4, "6 extra applied, got {}", c.health);
        assert_eq!(c.last_damage, crate::survival::DamageCause::Fall);
        // The window was not restarted, and a third equal hit is ignored.
        assert!(!c.take_damage_from(10.0, crate::survival::DamageCause::Fall));
        assert!((c.health - 10.0).abs() < 1e-4);
    }

    #[test]
    fn iframe_smaller_or_equal_hit_is_ignored() {
        let mut c = PlayerCombat::new();
        assert!(c.take_damage_from(4.0, crate::survival::DamageCause::Generic));
        assert!(!c.take_damage_from(3.0, crate::survival::DamageCause::Drowning));
        assert!(!c.take_damage_from(4.0, crate::survival::DamageCause::Fall));
        assert!((c.health - 16.0).abs() < 1e-4, "nothing extra applied, got {}", c.health);
        assert_eq!(c.last_damage, crate::survival::DamageCause::Generic);
    }

    #[test]
    fn iframe_window_expiry_allows_a_fresh_full_hit() {
        let mut c = PlayerCombat::new();
        c.take_damage_from(4.0, crate::survival::DamageCause::Generic);
        for _ in 0..INVINCIBILITY_TICKS {
            c.tick();
        }
        assert!(c.take_damage_from(3.0, crate::survival::DamageCause::Fall));
        assert!((c.health - 13.0).abs() < 1e-4);
    }

    #[test]
    fn iframe_excess_hit_can_kill() {
        let mut c = PlayerCombat::new();
        c.take_damage_from(4.0, crate::survival::DamageCause::Generic);
        assert!(c.take_damage_from(30.0, crate::survival::DamageCause::Fall));
        assert!(c.dead && c.just_died);
    }

    #[test]
    fn health_invulnerability_expires_after_frames() {
        let mut h = Health::new(20.0);
        h.take_damage(3.0);
        for _ in 0..INVINCIBILITY_TICKS {
            h.tick();
        }
        // Next tick clears the timer; a fresh hit should land now.
        assert!(h.take_damage(2.0));
        assert_eq!(h.current, 15.0);
    }

    #[test]
    fn health_clamps_at_zero_and_flags_dead() {
        let mut h = Health::new(4.0);
        h.take_damage(10.0);
        assert_eq!(h.current, 0.0);
        assert!(h.is_dead());
    }

    // --- PlayerCombat ---

    #[test]
    fn player_combat_death_sets_the_just_died_one_shot() {
        let mut p = PlayerCombat::new();
        p.take_damage(25.0);
        assert!(p.dead);
        assert_eq!(p.health, 0.0);
        assert!(p.just_died, "death tick must set the just_died one-shot");
    }

    #[test]
    fn die_kills_outright_once_and_records_the_cause() {
        use crate::survival::DamageCause;
        let mut p = PlayerCombat::new();
        p.invincible_timer = 5; // mid-invulnerability makes no difference
        p.die(DamageCause::Fall);
        assert!(p.dead && p.just_died);
        assert_eq!(p.health, 0.0);
        assert_eq!(p.last_damage, DamageCause::Fall);
        p.just_died = false; // consumed
        p.die(DamageCause::Drowning);
        assert!(!p.just_died, "already dead: no second death transition");
        assert_eq!(p.last_damage, DamageCause::Fall, "nor a rewritten cause");
    }

    #[test]
    fn player_combat_just_died_clears_after_consumer_flips_it() {
        // The just_died flag is a one-shot: callers consume + clear.
        let mut p = PlayerCombat::new();
        p.take_damage(25.0);
        assert!(p.just_died);
        p.just_died = false; // consumed
        // Subsequent ticks of the dead state must not re-set it.
        for _ in 0..10 {
            p.tick();
        }
        assert!(!p.just_died);
        assert!(p.dead);
    }

    #[test]
    fn death_screen_waits_for_respawn_request() {
        // W2 owner decision 2026-10-06 — no auto-respawn: however long the
        // player sits on the death screen, they stay dead until they choose.
        let mut p = PlayerCombat::new();
        assert!(!p.request_respawn(), "alive: a press must not queue a respawn");
        assert!(!p.respawn_requested);
        p.take_damage(25.0);
        for _ in 0..2000 {
            p.tick();
        }
        assert!(p.dead);
        assert!(!p.should_respawn(), "time alone must never respawn the player");
        assert!(p.request_respawn());
        assert!(p.should_respawn());
        p.respawn();
        assert!(!p.dead);
        assert!(!p.respawn_requested, "respawn() consumes the request");
        assert!(!p.should_respawn());
    }

    #[test]
    fn player_combat_respawn_restores_full_health() {
        let mut p = PlayerCombat::new();
        p.take_damage(25.0);
        p.respawn();
        assert!(!p.dead);
        assert_eq!(p.health, p.max_health);
    }

    #[test]
    fn heal_clamps_to_max_health() {
        let mut p = PlayerCombat::new();
        p.take_damage(8.0);
        assert_eq!(p.health, 12.0);
        let restored = p.heal(20.0);
        assert_eq!(restored, 8.0);
        assert_eq!(p.health, p.max_health);
    }

    #[test]
    fn poison_drains_health_at_intervals_with_floor() {
        let mut p = PlayerCombat::new();
        p.apply_poison(POISON_DURATION_TICKS);
        assert!(p.is_poisoned());
        // Run the full poison duration. Health should drop but never below
        // POISON_HEALTH_FLOOR.
        for _ in 0..POISON_DURATION_TICKS {
            p.tick();
        }
        assert!(!p.is_poisoned());
        assert!(p.health >= POISON_HEALTH_FLOOR);
        assert!(p.health < p.max_health, "poison should have done some damage");
    }

    #[test]
    fn poison_cannot_kill() {
        let mut p = PlayerCombat::new();
        // Start with very low health.
        p.health = 1.0;
        p.apply_poison(POISON_DURATION_TICKS * 10);
        for _ in 0..(POISON_DURATION_TICKS * 10) {
            p.tick();
        }
        assert!(!p.dead, "poison alone must not kill");
        assert!(p.health >= POISON_HEALTH_FLOOR);
    }

    #[test]
    fn poison_apply_takes_longer_of_two_durations() {
        let mut p = PlayerCombat::new();
        p.apply_poison(60);
        p.apply_poison(20);
        assert_eq!(p.poison_ticks, 60, "shorter follow-up must not cut existing");
        p.apply_poison(120);
        assert_eq!(p.poison_ticks, 120, "longer follow-up extends");
    }

    #[test]
    fn poison_apply_no_op_when_dead() {
        let mut p = PlayerCombat::new();
        p.take_damage(25.0);
        assert!(p.dead);
        p.apply_poison(60);
        assert!(!p.is_poisoned());
    }

    #[test]
    fn heal_no_op_when_dead() {
        let mut p = PlayerCombat::new();
        p.take_damage(25.0);
        assert!(p.dead);
        let restored = p.heal(5.0);
        assert_eq!(restored, 0.0);
        assert_eq!(p.health, 0.0);
    }

    #[test]
    fn player_combat_attack_cooldown_expires() {
        let mut p = PlayerCombat::new();
        p.attack_cooldown = ATTACK_COOLDOWN;
        assert!(!p.can_attack());
        for _ in 0..ATTACK_COOLDOWN {
            p.tick();
        }
        assert!(p.can_attack());
    }

    // --- despawn_dead ---

    #[test]
    fn despawn_dead_removes_entities_with_zero_health() {
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 64.0, 0.0));
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(1.0, 64.0, 0.0));
        // Kill the first one directly.
        let first: hecs::Entity = ecs
            .query::<&Position>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        {
            let mut h = ecs.get::<&mut Health>(first).unwrap();
            h.current = 0.0;
        }
        let deaths = despawn_dead(&mut ecs);
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].0, MobType::Brigand);
        assert_eq!(ecs.query::<&MobKind>().iter().count(), 1);
    }

    #[test]
    fn despawn_dead_returns_position_of_each_dead_mob() {
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(7.0, 64.0, 3.0));
        entity::spawn_mob(&mut ecs, MobType::Pig, Vec3::new(-1.0, 64.0, 9.0));
        // Kill both.
        let ids: Vec<hecs::Entity> = ecs
            .query::<&Position>()
            .iter()
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            ecs.get::<&mut Health>(id).unwrap().current = 0.0;
        }
        let deaths = despawn_dead(&mut ecs);
        assert_eq!(deaths.len(), 2);
        let kinds: Vec<MobType> = deaths.iter().map(|(k, _, _)| *k).collect();
        assert!(kinds.contains(&MobType::Cow));
        assert!(kinds.contains(&MobType::Pig));
        // Positions preserved.
        let positions: Vec<Vec3> = deaths.iter().map(|(_, p, _)| *p).collect();
        assert!(positions.iter().any(|p| (p.x - 7.0).abs() < 1e-3));
        assert!(positions.iter().any(|p| (p.x + 1.0).abs() < 1e-3));
    }

    #[test]
    fn despawn_dead_includes_last_attacker_when_stamped() {
        // A Cow stamped with LastAttacker(3) should surface that pidx
        // in the despawn_dead tuple. Drives the bounty-board
        // attribution path.
        let mut ecs = hecs::World::new();
        let cow = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        let _ = ecs.insert_one(cow, LastAttacker(3));
        ecs.get::<&mut Health>(cow).unwrap().current = 0.0;
        let deaths = despawn_dead(&mut ecs);
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].2, Some(3), "LastAttacker pidx should propagate");
    }

    #[test]
    fn despawn_dead_no_attacker_returns_none() {
        // No-player-damage death (mob-on-mob, suffocation, fall) →
        // None. Caller falls back to nearest-player proximity.
        let mut ecs = hecs::World::new();
        let pig = entity::spawn_mob(&mut ecs, MobType::Pig, Vec3::new(0.0, 64.0, 0.0));
        ecs.get::<&mut Health>(pig).unwrap().current = 0.0;
        let deaths = despawn_dead(&mut ecs);
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].2, None);
    }

    #[test]
    fn player_attack_stamps_last_attacker_on_damage() {
        // Setting up a mob in front of a player; a successful melee
        // hit should land a LastAttacker(7) component.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let mob_pos = Vec3::new(0.0, 64.0, 2.0);
        let cow = entity::spawn_mob(&mut ecs, MobType::Cow, mob_pos);

        let mut combat = PlayerCombat::new();
        let hit = player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 7,
        );
        assert!(hit, "should have hit the cow in front");
        let attacker = ecs.get::<&LastAttacker>(cow).expect("LastAttacker should be stamped");
        assert_eq!(attacker.0, 7);
    }

    #[test]
    fn player_attack_spooks_prey_into_fleeing() {
        // P2 — striking a cow flips it into Flee so it bolts away.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let cow = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 2.0));
        let mut combat = PlayerCombat::new();
        assert!(player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 1,
        ));
        let ai = ecs.get::<&crate::mob_ai::MobAi>(cow).expect("cow has MobAi");
        assert!(
            matches!(ai.state, crate::mob_ai::AiState::Flee { .. }),
            "a struck prey animal should flee"
        );
    }

    #[test]
    fn player_attack_provokes_bear_into_aggro() {
        // Task 13 — a melee hit on a Bear must enter Aggro targeting the
        // attacking player's slot. Before this fix, `on_hit_by_player` was
        // never called, so hitting a Bear did nothing.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let bear = entity::spawn_mob(&mut ecs, MobType::Bear, Vec3::new(0.0, 64.0, 2.0));
        let mut combat = PlayerCombat::new();
        assert!(player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 9,
        ));
        let data = ecs.get::<&crate::bear_ai::BearData>(bear).expect("bear has BearData");
        match data.state {
            crate::bear_ai::BearAiState::Aggro { ticks_remaining, attacker_pidx } => {
                assert_eq!(ticks_remaining, crate::bear_ai::AGGRO_DURATION_TICKS);
                assert_eq!(attacker_pidx, 9);
            }
            other => panic!("expected Aggro after being hit, got {other:?}"),
        }
    }

    #[test]
    fn player_attack_provokes_hyena_into_aggro() {
        // Same wiring, Hyena side.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let hyena = entity::spawn_mob(&mut ecs, MobType::Hyena, Vec3::new(0.0, 64.0, 2.0));
        let mut combat = PlayerCombat::new();
        assert!(player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 2,
        ));
        let data = ecs.get::<&crate::hyena_ai::HyenaData>(hyena).expect("hyena has HyenaData");
        match data.state {
            crate::hyena_ai::HyenaAiState::Aggro { ticks_remaining, attacker_pidx } => {
                assert_eq!(ticks_remaining, crate::hyena_ai::AGGRO_DURATION_TICKS);
                assert_eq!(attacker_pidx, 2);
            }
            other => panic!("expected Aggro after being hit, got {other:?}"),
        }
    }

    #[test]
    fn player_attack_does_not_spook_non_prey() {
        // A Bear is neutral-aggressive, not prey — it must NOT flee when hit.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let bear = entity::spawn_mob(&mut ecs, MobType::Bear, Vec3::new(0.0, 64.0, 2.0));
        let mut combat = PlayerCombat::new();
        assert!(player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 1,
        ));
        let ai = ecs.get::<&crate::mob_ai::MobAi>(bear).expect("bear has MobAi");
        assert!(
            !matches!(ai.state, crate::mob_ai::AiState::Flee { .. }),
            "a non-prey mob should not flee"
        );
    }

    #[test]
    fn in_swing_arc_respects_reach_and_cone() {
        let look = Vec3::new(0.0, 0.0, 1.0);
        assert!(in_swing_arc(Vec3::new(0.0, 0.0, 2.0), look, 3.0, 0.5), "in front, in reach");
        assert!(!in_swing_arc(Vec3::new(0.0, 0.0, 5.0), look, 3.0, 0.5), "too far");
        assert!(!in_swing_arc(Vec3::new(0.0, 0.0, -2.0), look, 3.0, 0.5), "behind");
        assert!(!in_swing_arc(Vec3::new(3.0, 0.0, 0.1), look, 3.0, 0.5), "off to the side");
    }

    #[test]
    fn sweep_attack_also_hits_other_mobs_in_the_arc() {
        // #23 — two cows in front within the swing arc; the closer is the
        // primary, the other should take sweep damage (both end up stamped).
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let near = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 1.5));
        let far = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.3, 64.0, 2.0));

        let mut combat = PlayerCombat::new();
        let hit = player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 7,
        );
        assert!(hit, "primary hit lands");
        assert!(ecs.get::<&LastAttacker>(near).is_ok(), "primary stamped");
        assert!(
            ecs.get::<&LastAttacker>(far).is_ok(),
            "the second mob in the arc took sweep damage too"
        );
    }

    #[test]
    fn sweep_spares_a_mob_behind_the_player() {
        // A mob behind the player must NOT be swept (cone gate).
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let front = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 1.5));
        let behind = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, -1.5));
        let mut combat = PlayerCombat::new();
        player_attack(&mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 7);
        assert!(ecs.get::<&LastAttacker>(front).is_ok(), "front cow hit");
        assert!(ecs.get::<&LastAttacker>(behind).is_err(), "cow behind is spared");
    }

    /// Spawn a Wolf owned by `owner` at `pos`. `spawn_mob` auto-attaches an
    /// untamed `WolfData`; overwrite it with a tamed one.
    fn spawn_owned_wolf(ecs: &mut hecs::World, pos: Vec3, owner: &str) -> hecs::Entity {
        let wolf = entity::spawn_mob(ecs, MobType::Wolf, pos);
        let mut wd = crate::wolf::WolfData::untamed();
        wd.ownership.owner_pubkey = owner.into();
        let _ = ecs.insert_one(wolf, wd);
        wolf
    }

    #[test]
    fn sweep_spares_the_attackers_own_pet_in_the_arc() {
        // 1C no-friendly-fire — an assisting pet standing beside a legitimate
        // primary target must NOT take sweep damage (nor be LastAttacker-
        // stamped) while its owner isn't sneaking. Geometry mirrors
        // `sweep_attack_also_hits_other_mobs_in_the_arc`: the cow at z=1.5 is
        // the closer primary, the pet at (0.3, 2.0) is inside the sweep arc.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let cow = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 1.5));
        let pet = spawn_owned_wolf(&mut ecs, Vec3::new(0.3, 64.0, 2.0), "local-player-7");

        let pet_hp = ecs.get::<&Health>(pet).unwrap().current;
        let mut combat = PlayerCombat::new();
        let hit = player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 7,
        );
        assert!(hit, "primary hit on the cow still lands");
        assert!(ecs.get::<&LastAttacker>(cow).is_ok(), "primary stamped");
        assert_eq!(
            ecs.get::<&Health>(pet).unwrap().current,
            pet_hp,
            "own pet in the sweep arc must take no sweep damage while not sneaking"
        );
        assert!(
            ecs.get::<&LastAttacker>(pet).is_err(),
            "a shielded sweep must not stamp LastAttacker on the pet"
        );
    }

    #[test]
    fn sweep_hits_own_pet_when_sneaking() {
        // Sneaking is the deliberate-hit bypass — the sweep must land on the
        // attacker's own pet too.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let _cow = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 1.5));
        let pet = spawn_owned_wolf(&mut ecs, Vec3::new(0.3, 64.0, 2.0), "local-player-7");

        let pet_hp = ecs.get::<&Health>(pet).unwrap().current;
        let mut combat = PlayerCombat::new();
        let hit = player_attack(
            &mut ecs, player_pos, look_dir, true, false, true, &mut combat, 5.0, 7,
        );
        assert!(hit, "primary hit lands");
        assert!(
            ecs.get::<&Health>(pet).unwrap().current < pet_hp,
            "sneaking bypasses the shield — sweep damage lands on the own pet"
        );
    }

    #[test]
    fn sweep_still_hits_another_players_pet_in_the_arc() {
        // Only the ATTACKER'S own pets are shielded — a pet owned by a
        // different player takes normal sweep damage.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let _cow = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 1.5));
        let pet = spawn_owned_wolf(&mut ecs, Vec3::new(0.3, 64.0, 2.0), "local-player-9");

        let pet_hp = ecs.get::<&Health>(pet).unwrap().current;
        let mut combat = PlayerCombat::new();
        // Attacker is player 7, not sneaking; the wolf belongs to player 9.
        let hit = player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 7,
        );
        assert!(hit, "primary hit lands");
        assert!(
            ecs.get::<&Health>(pet).unwrap().current < pet_hp,
            "another player's pet in the arc still takes sweep damage"
        );
        assert!(
            ecs.get::<&LastAttacker>(pet).is_ok(),
            "the swept non-own pet is stamped normally"
        );
    }

    // --- Task 8 (bug-hardening, 2026-07-07) — own pets body-block attacks:
    // retarget instead of cancelling. `find_attack_target_for_swing` must
    // skip the attacker's own tamed pet/steed when it's the NEAREST cone
    // candidate, picking the next-nearest (the actual hostile) instead of
    // returning nothing (the old `own_pet_shielded` cancel-the-swing bug).

    #[test]
    fn swing_target_skips_own_pet_for_the_hostile_behind_it_when_not_sneaking() {
        // The wolf body-blocks at 1.8 blocks (nearer than the hostile just
        // inside reach at 2.9 — `ATTACK_REACH` is 3.0, and the pick's 3D
        // distance includes half the hostile's hitbox height, so 2.9 keeps it
        // just inside), both in the same forward cone. Not sneaking → the
        // hostile is picked.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let hostile = entity::spawn_mob(&mut ecs, MobType::Hyena, Vec3::new(0.0, 64.0, 2.9));
        let _pet = spawn_owned_wolf(&mut ecs, Vec3::new(0.0, 64.0, 1.8), "local-player-7");

        let picked = find_attack_target_for_swing(
            &ecs, player_pos, look_dir, "local-player-7", 7, false,
        );
        assert_eq!(
            picked.map(|(id, _)| id),
            Some(hostile),
            "own pet body-blocking the hostile must be skipped, not cancel the swing"
        );
    }

    #[test]
    fn swing_target_picks_own_pet_when_sneaking() {
        // Sneaking is the deliberate-hit signal — the nearer own pet stays
        // targetable, same as before.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let _hostile = entity::spawn_mob(&mut ecs, MobType::Hyena, Vec3::new(0.0, 64.0, 2.9));
        let pet = spawn_owned_wolf(&mut ecs, Vec3::new(0.0, 64.0, 1.8), "local-player-7");

        let picked = find_attack_target_for_swing(
            &ecs, player_pos, look_dir, "local-player-7", 7, true,
        );
        assert_eq!(
            picked.map(|(id, _)| id),
            Some(pet),
            "sneaking keeps the own pet targetable (preserves the deliberate-cull path)"
        );
    }

    #[test]
    fn swing_target_still_prefers_own_pet_when_it_is_the_only_candidate() {
        // No hostile in the cone at all — not sneaking still means no
        // target (never falls through to some entity outside reach/cone),
        // matching the old shield's net effect for this specific case.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let _pet = spawn_owned_wolf(&mut ecs, Vec3::new(0.0, 64.0, 1.8), "local-player-7");

        let picked = find_attack_target_for_swing(
            &ecs, player_pos, look_dir, "local-player-7", 7, false,
        );
        assert!(picked.is_none(), "no other candidate in the cone — no target");
    }

    // --- Task 9 (bug-hardening, 2026-07-07) — a perched parrot is cosmetic
    // (pinned to the owner's shoulder) and must never be picked by either
    // selector, sneaking or not — unlike the Task 8 own-pet gate above, which
    // sneaking deliberately bypasses.

    fn spawn_perched_parrot(ecs: &mut hecs::World, pos: Vec3, owner: &str) -> hecs::Entity {
        let parrot = entity::spawn_mob(ecs, MobType::Parrot, pos);
        let mut data = crate::companion::CompanionData::untamed();
        data.ownership.owner_pubkey = owner.into();
        data.state = crate::companion::CompanionState::Perch;
        let _ = ecs.insert_one(parrot, data);
        parrot
    }

    #[test]
    fn swing_target_never_picks_a_perched_parrot_even_when_sneaking() {
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        // Right at the camera, well inside reach and the cone — exactly where
        // the unrotated shoulder offset used to put it.
        let _parrot = spawn_perched_parrot(&mut ecs, Vec3::new(0.0, 64.0, 0.5), "local-player-7");

        let not_sneaking = find_attack_target_for_swing(
            &ecs, player_pos, look_dir, "local-player-7", 7, false,
        );
        assert!(not_sneaking.is_none(), "perched parrot must not be swung at while not sneaking");

        let sneaking = find_attack_target_for_swing(
            &ecs, player_pos, look_dir, "local-player-7", 7, true,
        );
        assert!(
            sneaking.is_none(),
            "sneaking bypasses the Task 8 own-pet gate but must NOT surface a perched parrot"
        );
    }

    #[test]
    fn interaction_picker_never_picks_a_perched_parrot() {
        // `find_attack_target` is the unfiltered picker the right-click
        // interactions (mount/feed/lead/pack/tame) use — it deliberately DOES
        // want the owner's own pets, but a perched parrot is cosmetic and
        // must still be excluded.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let _parrot = spawn_perched_parrot(&mut ecs, Vec3::new(0.0, 64.0, 0.5), "local-player-7");

        let picked = find_attack_target(&ecs, player_pos, look_dir);
        assert!(picked.is_none(), "perched parrot must be invisible to the interaction picker too");
    }

    #[test]
    fn swing_target_falls_through_a_perched_parrot_to_the_hostile_behind_it() {
        // The perched parrot sits nearer than the hostile — same body-block
        // shape as the Task 8 own-pet case — but must be skipped unconditionally.
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let hostile = entity::spawn_mob(&mut ecs, MobType::Hyena, Vec3::new(0.0, 64.0, 2.9));
        let _parrot = spawn_perched_parrot(&mut ecs, Vec3::new(0.0, 64.0, 0.5), "local-player-7");

        let picked = find_attack_target_for_swing(
            &ecs, player_pos, look_dir, "local-player-7", 7, true,
        );
        assert_eq!(
            picked.map(|(id, _)| id),
            Some(hostile),
            "perched parrot must never body-block a swing, even sneaking"
        );
    }

    #[test]
    fn own_pet_body_block_no_longer_falls_through_to_block_break_via_player_attack() {
        // End-to-end via `player_attack` (what the melee arm actually calls):
        // damage lands on the hostile behind the body-blocking pet, and the
        // pet is untouched, instead of `player_attack` returning false (which
        // used to make the melee arm fall through to mining the block behind
        // the enemy).
        let mut ecs = hecs::World::new();
        let player_pos = Vec3::new(0.0, 64.0, 0.0);
        let look_dir = Vec3::new(0.0, 0.0, 1.0);
        let hostile = entity::spawn_mob(&mut ecs, MobType::Hyena, Vec3::new(0.0, 64.0, 2.9));
        let pet = spawn_owned_wolf(&mut ecs, Vec3::new(0.0, 64.0, 1.8), "local-player-7");
        let hostile_hp = ecs.get::<&Health>(hostile).unwrap().current;
        let pet_hp = ecs.get::<&Health>(pet).unwrap().current;

        let mut combat = PlayerCombat::new();
        let hit = player_attack(
            &mut ecs, player_pos, look_dir, true, false, false, &mut combat, 5.0, 7,
        );

        assert!(hit, "the swing lands on the hostile instead of being cancelled");
        assert!(
            ecs.get::<&Health>(hostile).unwrap().current < hostile_hp,
            "damage lands on the hostile behind the body-blocking pet"
        );
        assert_eq!(
            ecs.get::<&Health>(pet).unwrap().current,
            pet_hp,
            "the body-blocking pet itself takes no damage while not sneaking"
        );
    }

    // --- tick_mob_attacks ---

    // --- Hunger tests (Wave 24) ---

    #[test]
    fn hunger_drains_at_interval() {
        let mut combat = PlayerCombat::new();
        assert_eq!(combat.hunger, 20);
        // Tick exactly HUNGER_DRAIN_INTERVAL_TICKS times.
        for _ in 0..HUNGER_DRAIN_INTERVAL_TICKS {
            combat.tick();
        }
        assert_eq!(combat.hunger, 19, "hunger should drop by 1 after one full interval");
    }

    #[test]
    fn hunger_no_drain_when_dead() {
        let mut combat = PlayerCombat::new();
        combat.dead = true;
        for _ in 0..(HUNGER_DRAIN_INTERVAL_TICKS * 2) {
            combat.tick();
        }
        assert_eq!(combat.hunger, 20, "dead players don't get hungry");
    }

    #[test]
    fn passive_regen_costs_hunger() {
        let mut combat = PlayerCombat::new();
        combat.health = 10.0;
        // Hunger = 20 (well-fed), health below max → regen kicks in.
        for _ in 0..HEALTH_REGEN_INTERVAL_TICKS {
            combat.tick();
        }
        assert_eq!(combat.health, 11.0, "regen should restore 1 HP");
        assert_eq!(combat.hunger, 19, "regen should cost 1 hunger");
    }

    #[test]
    fn passive_regen_skipped_when_hungry() {
        let mut combat = PlayerCombat::new();
        combat.health = 10.0;
        combat.hunger = 10; // below threshold
        for _ in 0..(HEALTH_REGEN_INTERVAL_TICKS * 2) {
            combat.tick();
        }
        assert_eq!(combat.health, 10.0, "no regen when below hunger threshold");
    }

    #[test]
    fn passive_regen_skipped_when_poisoned() {
        let mut combat = PlayerCombat::new();
        combat.health = 10.0;
        combat.apply_poison(POISON_DAMAGE_INTERVAL_TICKS * 4);
        for _ in 0..HEALTH_REGEN_INTERVAL_TICKS {
            combat.tick();
        }
        // Poison drains; regen suppressed.
        assert!(combat.health <= 10.0, "regen must not run while poisoned");
    }

    #[test]
    fn starvation_damages_at_zero_hunger_with_floor() {
        let mut combat = PlayerCombat::new();
        combat.hunger = 0;
        // Tick long enough to dip below the floor without it.
        for _ in 0..(STARVATION_INTERVAL_TICKS * 50) {
            combat.tick();
        }
        // W2 — default floor is Normal's 1 HP (difficulty table); the
        // per-difficulty floors are covered in `survival::tests`.
        assert_eq!(combat.health, 1.0,
            "starvation must floor at Normal's 1 HP by default (cannot kill)");
        assert!(!combat.dead);
    }

    #[test]
    fn starvation_no_damage_when_fed() {
        let mut combat = PlayerCombat::new();
        combat.hunger = 5; // above zero, below regen threshold
        let start = combat.health;
        for _ in 0..(STARVATION_INTERVAL_TICKS * 5) {
            combat.tick();
        }
        assert_eq!(combat.health, start, "no starvation damage when hunger > 0");
    }

    #[test]
    fn feed_clamps_to_max_hunger() {
        let mut combat = PlayerCombat::new();
        combat.hunger = 18;
        let added = combat.feed(10);
        assert_eq!(added, 2, "should only add up to max_hunger");
        assert_eq!(combat.hunger, 20);
    }

    #[test]
    fn feed_no_op_when_dead() {
        let mut combat = PlayerCombat::new();
        combat.hunger = 5;
        combat.dead = true;
        let added = combat.feed(10);
        assert_eq!(added, 0);
        assert_eq!(combat.hunger, 5);
    }

    #[test]
    fn respawn_restores_hunger() {
        let mut combat = PlayerCombat::new();
        combat.hunger = 0;
        combat.hunger_drain_ticks = 100;
        combat.starvation_ticks = 30;
        combat.dead = true;
        combat.respawn();
        assert_eq!(combat.hunger, combat.max_hunger);
        assert_eq!(combat.hunger_drain_ticks, 0);
        assert_eq!(combat.starvation_ticks, 0);
    }

    #[test]
    fn is_well_fed_threshold() {
        let mut combat = PlayerCombat::new();
        combat.hunger = 17;
        assert!(!combat.is_well_fed());
        combat.hunger = 18;
        assert!(combat.is_well_fed());
        combat.hunger = 20;
        assert!(combat.is_well_fed());
    }

    #[test]
    fn tick_mob_attacks_skips_when_player_dead() {
        let mut ecs = hecs::World::new();
        entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(0.0, 64.0, 0.0));
        let mut slot = crate::player_slot::PlayerSlot::new(
            0, Vec3::new(0.0, 64.0, 0.0), 1.0,
        );
        slot.combat.dead = true;
        let start_health = slot.combat.health;
        let landed = tick_mob_attacks(&ecs, &mut slot, crate::survival::Difficulty::Normal);
        assert_eq!(slot.combat.health, start_health, "dead players take no damage");
        assert_eq!(slot.player.velocity, Vec3::ZERO);
        assert!(landed.is_empty(), "no attackers reported against a dead player");
    }

    /// Task 8b — the revenge-pivot wiring needs to know WHICH mob landed
    /// contact damage; `tick_mob_attacks` reports exactly the hostiles that
    /// connected (in range + armour gate passed), not merely proximate ones.
    #[test]
    fn tick_mob_attacks_reports_attackers_that_landed() {
        let mut ecs = hecs::World::new();
        let near =
            entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(1.0, 64.0, 0.0));
        let _far =
            entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(10.0, 64.0, 0.0));
        let mut slot = crate::player_slot::PlayerSlot::new(
            0, Vec3::new(0.0, 64.0, 0.0), 1.0,
        );
        let start_health = slot.combat.health;
        let landed = tick_mob_attacks(&ecs, &mut slot, crate::survival::Difficulty::Normal);
        assert!(slot.combat.health < start_health, "the near brigand's hit lands");
        assert_eq!(landed, vec![near], "only the mob that landed damage is reported");
        // Immediately again: i-frames block the hit → nothing reported.
        let landed_again = tick_mob_attacks(&ecs, &mut slot, crate::survival::Difficulty::Normal);
        assert!(landed_again.is_empty(), "i-frame-blocked hits are not reported");
    }

    // --- Spec 28e — armour damage reduction ---

    #[test]
    fn full_iron_set_blunts_damage_to_40_percent() {
        // Full Iron set = 15 points → 60% reduction → 40% of raw damage
        // lands. An 8.0 raw hit should deal 3.2 through the armour.
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        let mut slot = crate::player_slot::PlayerSlot::new(0, Vec3::ZERO, 1.0);
        slot.armour_slots = [
            Some(ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron)),
            Some(ArmourItem::new(ArmourSlot::Chestplate, ArmourMaterial::Iron)),
            Some(ArmourItem::new(ArmourSlot::Leggings, ArmourMaterial::Iron)),
            Some(ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Iron)),
        ];
        let before = slot.combat.health;
        slot.take_damage_with_armour_from(8.0, crate::survival::DamageCause::Generic);
        let dealt = before - slot.combat.health;
        let expected = 8.0 * 0.40;
        assert!((dealt - expected).abs() < 1e-3,
            "iron set should land {expected:.2} hp, got {dealt:.2}");
    }

    #[test]
    fn armour_hit_wears_one_durability_per_piece() {
        // Each landed hit takes a single durability point off every
        // non-broken equipped piece (Minecraft per-piece behaviour).
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot, max_durability};
        let mut slot = crate::player_slot::PlayerSlot::new(0, Vec3::ZERO, 1.0);
        slot.armour_slots = [
            Some(ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron)),
            Some(ArmourItem::new(ArmourSlot::Chestplate, ArmourMaterial::Iron)),
            Some(ArmourItem::new(ArmourSlot::Leggings, ArmourMaterial::Iron)),
            Some(ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Iron)),
        ];
        slot.take_damage_with_armour_from(8.0, crate::survival::DamageCause::Generic);
        for s in slot.armour_slots.iter() {
            let piece = s.as_ref().expect("piece still equipped after one hit");
            assert_eq!(
                piece.durability,
                max_durability(piece.slot, piece.material) - 1,
                "{:?} should have lost exactly 1 durability", piece.slot,
            );
        }
    }

    #[test]
    fn broken_piece_unequips_silently_after_final_hit() {
        // A piece at durability 1 wears to 0 on the next hit and is
        // dropped from the slot so it stops contributing.
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        let mut helmet = ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Leather);
        helmet.durability = 1;
        let mut slot = crate::player_slot::PlayerSlot::new(0, Vec3::ZERO, 1.0);
        slot.armour_slots[ArmourSlot::Helmet as usize] = Some(helmet);
        assert_eq!(slot.total_armour_points(), 1);
        let landed = slot.take_damage_with_armour_from(2.0, crate::survival::DamageCause::Generic);
        assert!(landed);
        assert!(slot.armour_slots[ArmourSlot::Helmet as usize].is_none(),
            "broken helmet must auto-unequip");
        assert_eq!(slot.total_armour_points(), 0);
    }
}

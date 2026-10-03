//! Spec 28d.wolves — tameable companion mob.
//!
//! Five AI states + bone-taming (33% per click) + owner pubkey on
//! `WolfData` + pet list on PlayerSlot + emotional-loss drops
//! (tamed wolves drop nothing).
//!
//! Pure-function shape: state transitions and taming roll are seeded
//! by `(mob_id, tick)` so behaviour is testable + replay-stable.
//!
//! Live integration landed in R5 (2026-05-28): `WolfData` is wired into
//! `spawn_mob` (every Wolf carries one) and the bone-feed taming path. The
//! follow/attack *movement* AI remains dormant — taming toggles flip state but
//! don't yet drive movement (audit E2). This module is the data layer + pure
//! logic + tests.
//!
//! Spec: `docs/foundations/2026-05-20-wolves-tameable-companion.md`.

use serde::{Deserialize, Serialize};

use crate::item::{ItemStack, MaterialId};

/// Maximum follow distance (blocks). When the owner exceeds this, a
/// tamed wolf in `FollowOwner` state starts pathing toward them.
pub const FOLLOW_MAX_DISTANCE: f32 = 8.0;

/// Minimum follow distance (blocks). When the wolf gets closer than
/// this, follow-pathing stops — prevents the oscillation that would
/// happen if the wolf walked exactly to the player's position and then
/// the player's next tick moved them, restarting the chase.
pub const FOLLOW_MIN_DISTANCE: f32 = 2.0;

/// Distance beyond which a following wolf gives up entirely rather than
/// pathing forever toward an owner who has gone offline, teleported, or
/// otherwise crossed into "unreachable" territory. 4x `FOLLOW_MAX_DISTANCE`
/// — comfortably past the ordinary follow band, but not so tight that a
/// player briefly out-running their wolf strands it. Matches
/// `WolfAction::GiveUpFollow`'s documented "owner crossed despawn
/// distance" intent (Task 14). The resume threshold is deliberately NOT
/// this constant — a gave-up wolf only starts following again once the
/// owner is back within `FOLLOW_MAX_DISTANCE` (hysteresis, same pattern
/// as the FOLLOW_MAX/FOLLOW_MIN pair), so hovering near the give-up
/// radius can't flap the wolf between giving up and chasing.
pub const FOLLOW_GIVE_UP_DISTANCE: f32 = FOLLOW_MAX_DISTANCE * 4.0;

/// After the owner takes damage, the wolf has this many ticks to chase
/// the attacker. 90 ticks = 4.5 seconds at 20 TPS — long enough to
/// commit to a target, short enough that wandering "I got hit ages
/// ago" claims don't trigger surprise attacks later.
pub const REVENGE_WINDOW_TICKS: u64 = 90;

/// After the owner attacks something, the wolf has this many ticks to
/// pile on to the same target. Shorter than the revenge window because
/// "owner is currently fighting X" is a stronger signal than "owner
/// got hit by X recently".
pub const ASSIST_WINDOW_TICKS: u64 = 30;

/// Taming success probability per bone-click. 33% mirrors the
/// Minecraft baseline.
pub const TAME_SUCCESS_NUMER: u32 = 33;
pub const TAME_SUCCESS_DENOM: u32 = 100;

/// Contact damage a companion wolf lands on its `AttackTarget` per hit
/// (Task 8 — combat-assist movement). Matches `combat::Health::take_damage`'s
/// `f32` contract rather than the integer sketch in the task brief; the
/// consumer (`block_interact::tick_wolf_attack`) throttles hits to once per
/// 20 ticks, and `Health`'s own invincibility-frame timer provides a second,
/// independent floor on hit rate.
pub const WOLF_ASSIST_DAMAGE: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WolfAiState {
    Idle,
    FollowOwner,
    Sit,
    AttackHostile {
        target_id: u64,
        until_tick: u64,
    },
    AttackRecentAttacker {
        target_id: u64,
        until_tick: u64,
    },
}

/// Spec 28d chunk 16 — Wolf consumes the generic `tameable::
/// OwnershipData` for owner/damage/attack tracking. Convenience
/// accessors (`owner_pubkey`, `last_owner_damage_tick`, etc.) are
/// preserved as methods so external call-sites don't need to know
/// about the indirection. Internal AI tick + tame logic delegate
/// to the generic primitives in `tameable.rs`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WolfData {
    pub state: WolfAiState,
    pub ownership: crate::tameable::OwnershipData,
}

impl WolfData {
    /// New untamed wolf — wanders the world, gets the regular drop
    /// table on death.
    pub fn untamed() -> Self {
        Self {
            state: WolfAiState::Idle,
            ownership: crate::tameable::OwnershipData::untamed(),
        }
    }

    pub fn is_tamed(&self) -> bool {
        self.ownership.is_tamed()
    }

    /// Hex-encoded owner pubkey. Empty = untamed.
    pub fn owner_pubkey(&self) -> &str {
        &self.ownership.owner_pubkey
    }

    /// Convenience wrapper — live call sites (block_interact.rs, save.rs)
    /// read `.ownership.last_owner_damage_tick` directly instead.
    #[allow(dead_code)]
    pub fn last_owner_damage_tick(&self) -> u64 {
        self.ownership.last_owner_damage_tick
    }

    /// Same story as `last_owner_damage_tick` above.
    #[allow(dead_code)]
    pub fn last_owner_attack_tick(&self) -> u64 {
        self.ownership.last_owner_attack_tick
    }

    /// Owner-only command — toggle the sit/stand of a tamed wolf.
    /// Returns the new state, or None if the wolf isn't tamed by the
    /// caller (call site should toast "Not your wolf").
    pub fn try_toggle_sit(&mut self, caller_pubkey: &str) -> Option<WolfAiState> {
        if !self.ownership.is_owned_by(caller_pubkey) {
            return None;
        }
        self.state = match self.state {
            WolfAiState::Sit => WolfAiState::FollowOwner,
            // Any non-sit state — sitting overrides combat (Minecraft
            // pattern; the owner is making a deliberate "stay here"
            // call).
            _ => WolfAiState::Sit,
        };
        Some(self.state)
    }

    /// Strip a persistence-unsafe attack state before it's written to (or
    /// after it's read back from) a save.
    ///
    /// `AttackHostile`/`AttackRecentAttacker` carry a raw hecs entity id
    /// (`target_id`) and a `until_tick` measured against the *current*
    /// session's tick counter. Neither survives a reload: `target_id` is
    /// just bits, and `Entity::from_bits` will happily resolve it to
    /// whatever unrelated entity (a villager, a cow, another pet) picks up
    /// that id in the fresh ECS, with no ownership/hostility re-check —
    /// `tick_wolf_attack` just re-attacks it. And a stale `until_tick`
    /// compared against a restarted tick counter can make the state last
    /// arbitrarily long (or never expire).
    ///
    /// Called both when writing a save (`tamed_mobs_to_saved_impl`) and
    /// when decoding one back into a live `WolfData` (`chunk_stream.rs`
    /// restore + the WASM resume path in `game_loop.rs`), so saves written
    /// before this fix are covered too. Mirrors the natural timeout-revert
    /// mapping already used in `tick_wolf_attack` above: tamed wolves fall
    /// back to `FollowOwner`, untamed ones to `Idle`.
    pub fn sanitize_attack_state_for_persistence(&mut self) {
        if matches!(self.state, WolfAiState::AttackHostile { .. } | WolfAiState::AttackRecentAttacker { .. }) {
            self.state = if self.is_tamed() { WolfAiState::FollowOwner } else { WolfAiState::Idle };
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TameOutcome {
    /// 33% roll succeeded — wolf is now bonded to the caller.
    Succeeded,
    /// 33% roll failed — try again. Bone is still consumed (Minecraft
    /// pattern; the cost makes the success feel earned).
    Failed,
    /// Wolf is already tamed (by anyone).
    AlreadyTamed,
}

/// Hash a 64-bit seed to a unit-interval f32 deterministically.
/// Re-exported through `tameable::seed_to_unit_f32` — kept as a thin
/// re-export so wolf.rs's call-sites don't need to import tameable.
fn seed_to_unit_f32(seed: u64) -> f32 {
    crate::tameable::seed_to_unit_f32(seed)
}

/// Attempt to tame a wolf with a bone. Delegates to
/// `tameable::attempt_tame_generic` so the rate-roll logic is shared
/// across every tameable species. On success, also flips the
/// AI state to FollowOwner (species-specific).
pub fn attempt_tame(wolf: &mut WolfData, owner_pubkey: &str, seed: u64) -> TameOutcome {
    use crate::tameable::{TameAttempt, attempt_tame_generic};
    match attempt_tame_generic(
        &mut wolf.ownership,
        owner_pubkey,
        seed,
        TAME_SUCCESS_NUMER,
        TAME_SUCCESS_DENOM,
    ) {
        TameAttempt::Succeeded => {
            wolf.state = WolfAiState::FollowOwner;
            TameOutcome::Succeeded
        }
        TameAttempt::Failed => TameOutcome::Failed,
        TameAttempt::AlreadyTamed => TameOutcome::AlreadyTamed,
    }
}

/// Drop table for a wolf — emotional-loss semantics. Tamed wolves
/// drop nothing (killing a companion is loss, not loot). Untamed
/// wolves drop leather + 1-2 bones.
pub fn drops_for_wolf(wolf: &WolfData, seed: u64) -> Vec<ItemStack> {
    if wolf.is_tamed() {
        return Vec::new();
    }
    let bone_count = ((seed_to_unit_f32(seed) * 2.0) as u8) + 1;
    vec![
        ItemStack::new_material(MaterialId::Leather, 1),
        ItemStack::new_material(MaterialId::Bone, bone_count.clamp(1, 2)),
    ]
}

/// Mutation that a wolf AI tick wants to perform on the world.
/// Pure — the wolf module doesn't touch the ECS directly; the
/// game_loop consumes these and applies them.
#[derive(Clone, Debug, PartialEq)]
pub enum WolfAction {
    /// Walk toward this world-space target.
    MoveToward { x: f32, y: f32, z: f32 },
    /// Attack this entity.
    AttackTarget { entity_id: u64 },
    /// No action this tick (idle, sit, or already at target).
    NoOp,
    /// Wolf is too far from owner — give up follow, return to Idle.
    /// Owner crossed `FOLLOW_GIVE_UP_DISTANCE`. Produced by `tick_wolf`'s
    /// tamed-wolf branch (Task 14). Resume is hysteretic: the gave-up wolf
    /// idles at its spot and only re-enters `FollowOwner` once the owner
    /// comes back within `FOLLOW_MAX_DISTANCE` — not merely back under the
    /// give-up radius — so an owner hovering near the boundary can't flap
    /// it. Applied in block_interact.rs alongside `NoOp` (stand still
    /// rather than drift on stale wander velocity).
    GiveUpFollow,
}

/// Parse a local-player owner pubkey (`"local-player-{slot}"`, the shape
/// `attempt_tame` is given at the tame site) back to its player-slot index.
/// Returns `None` for any other shape (e.g. a real npub on a dedicated
/// server), in which case the companion pass leaves the wolf at rest until a
/// richer pubkey→player map exists. P3 gap-closure.
pub fn owner_slot_from_pubkey(pubkey: &str) -> Option<usize> {
    pubkey.strip_prefix("local-player-")?.parse::<usize>().ok()
}

/// One tick of wolf AI. Pure: reads world-shaped inputs, returns
/// the next state + the action the game_loop should apply.
///
/// `owner_pos` is None if the owner is offline / out of range; the
/// wolf falls back to Idle wandering.
pub fn tick_wolf(
    wolf: &WolfData,
    wolf_pos: (f32, f32, f32),
    owner_pos: Option<(f32, f32, f32)>,
    current_tick: u64,
) -> (WolfData, WolfAction) {
    let mut next = wolf.clone();

    // Sit short-circuits everything except a transition from Sit (only
    // the owner's right-click can toggle that — happens outside this
    // function).
    if matches!(wolf.state, WolfAiState::Sit) {
        return (next, WolfAction::NoOp);
    }

    // Resolve active combat target first — attack states have a hard
    // timeout, after which the wolf reverts.
    let combat_action = match wolf.state {
        WolfAiState::AttackHostile { target_id, until_tick }
        | WolfAiState::AttackRecentAttacker { target_id, until_tick } => {
            if current_tick >= until_tick {
                // Window closed; revert to follow-owner (if tamed) or idle.
                next.state = if wolf.is_tamed() {
                    WolfAiState::FollowOwner
                } else {
                    WolfAiState::Idle
                };
                None
            } else {
                Some(WolfAction::AttackTarget { entity_id: target_id })
            }
        }
        _ => None,
    };
    if let Some(action) = combat_action {
        return (next, action);
    }

    // Tamed wolves with an owner pos: follow logic.
    if wolf.is_tamed() {
        if let Some((ox, oy, oz)) = owner_pos {
            let dx = ox - wolf_pos.0;
            let dy = oy - wolf_pos.1;
            let dz = oz - wolf_pos.2;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            let dist = dist_sq.sqrt();
            // Hysteresis on the give-up boundary (same class of guard as
            // the FOLLOW_MAX/FOLLOW_MIN pair at the top of the file): a
            // tamed wolf in Idle has given up — Idle is only reachable for
            // a tamed wolf via the give-up branch below, since taming,
            // sit-toggle and attack-expiry all land tamed wolves in
            // FollowOwner/Sit. It idles at its spot until the owner
            // genuinely comes back to it (within FOLLOW_MAX_DISTANCE),
            // rather than resuming the instant dist dips back under
            // FOLLOW_GIVE_UP_DISTANCE — otherwise an owner hovering near
            // the give-up radius would flap the wolf between give-up and
            // follow every tick. Checked FIRST so a gave-up wolf that's
            // still at extreme range emits NoOp, not GiveUpFollow again —
            // the give-up action fires exactly once per give-up.
            if matches!(wolf.state, WolfAiState::Idle) {
                if dist <= FOLLOW_MAX_DISTANCE {
                    // Owner came back — resume following from here on.
                    next.state = WolfAiState::FollowOwner;
                }
                return (next, WolfAction::NoOp);
            }
            // Extreme range: the owner is unreachable (offline, teleported,
            // another part of the world) — stop pathing toward them rather
            // than chasing forever. Checked before the ordinary follow band
            // so it wins over the FollowOwner re-entry below.
            if dist > FOLLOW_GIVE_UP_DISTANCE {
                next.state = WolfAiState::Idle;
                return (next, WolfAction::GiveUpFollow);
            }
            // Anti-oscillation: only path if outside the min radius.
            if dist > FOLLOW_MAX_DISTANCE {
                next.state = WolfAiState::FollowOwner;
                return (next, WolfAction::MoveToward { x: ox, y: oy, z: oz });
            }
            if dist > FOLLOW_MIN_DISTANCE && matches!(wolf.state, WolfAiState::FollowOwner) {
                // Still in the follow band — keep moving but don't
                // re-enter the state.
                return (next, WolfAction::MoveToward { x: ox, y: oy, z: oz });
            }
            // Inside min distance or already idle/follow at rest.
            next.state = WolfAiState::FollowOwner;
            return (next, WolfAction::NoOp);
        }
        // Owner offline / out of range. NoOp + stay in current state.
        return (next, WolfAction::NoOp);
    }

    // Untamed wolves just wander idly. The actual wander vector lives
    // in the existing mob-AI wander logic; this returns NoOp so the
    // default wander path applies.
    (next, WolfAction::NoOp)
}

/// Helper for the game_loop — given the owner's most-recent damage
/// event, decide whether the wolf should pivot into attack-revenge.
/// Pure. Returns the new state if a pivot is appropriate; None if no
/// change.
pub fn maybe_pivot_to_revenge(
    wolf: &WolfData,
    attacker_id: u64,
    attacker_is_tamed_by_owner: bool,
    current_tick: u64,
) -> Option<WolfAiState> {
    // Sit is a deliberate owner "stay here" command (see `try_toggle_sit`,
    // where sitting deliberately overrides combat) — a sitting wolf never
    // self-pivots into a fight; only the owner's sit toggle releases it.
    // Wolf-specific state, so the gate lives here rather than in the
    // species-agnostic `tameable::should_pivot_*` predicates.
    if matches!(wolf.state, WolfAiState::Sit) {
        return None;
    }
    let in_combat = wolf_in_combat(wolf, current_tick);
    if !crate::tameable::should_pivot_to_revenge(
        &wolf.ownership,
        attacker_is_tamed_by_owner,
        in_combat,
    ) {
        return None;
    }
    Some(WolfAiState::AttackRecentAttacker {
        target_id: attacker_id,
        until_tick: current_tick + REVENGE_WINDOW_TICKS,
    })
}

/// Helper for the game_loop — the owner attacked an entity, wolf
/// should join the fight. Same discrimination as revenge: never
/// target a tamed wolf of the same owner.
pub fn maybe_pivot_to_assist(
    wolf: &WolfData,
    target_id: u64,
    target_is_tamed_by_owner: bool,
    current_tick: u64,
) -> Option<WolfAiState> {
    // Same Sit gate as revenge — see the comment there.
    if matches!(wolf.state, WolfAiState::Sit) {
        return None;
    }
    let in_combat = wolf_in_combat(wolf, current_tick);
    if !crate::tameable::should_pivot_to_assist(
        &wolf.ownership,
        target_is_tamed_by_owner,
        in_combat,
    ) {
        return None;
    }
    Some(WolfAiState::AttackHostile {
        target_id,
        until_tick: current_tick + ASSIST_WINDOW_TICKS,
    })
}

/// Helper: is this wolf currently in an active combat state whose
/// window hasn't closed yet?
fn wolf_in_combat(wolf: &WolfData, current_tick: u64) -> bool {
    matches!(
        wolf.state,
        WolfAiState::AttackHostile { until_tick, .. }
        | WolfAiState::AttackRecentAttacker { until_tick, .. }
        if current_tick < until_tick
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(pos: (f32, f32, f32)) -> (f32, f32, f32) {
        pos
    }

    #[test]
    fn owner_slot_parses_local_player_pubkey() {
        assert_eq!(owner_slot_from_pubkey("local-player-0"), Some(0));
        assert_eq!(owner_slot_from_pubkey("local-player-3"), Some(3));
        // Real npubs (dedicated server) and junk → None (companion rests).
        assert_eq!(owner_slot_from_pubkey("npub1abcdef"), None);
        assert_eq!(owner_slot_from_pubkey("local-player-"), None);
        assert_eq!(owner_slot_from_pubkey(""), None);
    }

    #[test]
    fn tamed_wolf_far_from_owner_moves_toward_them() {
        // A tamed wolf well outside FOLLOW_MAX_DISTANCE walks toward the owner.
        let mut wolf = WolfData::untamed();
        // Tame deterministically by retrying seeds until the 33% roll lands.
        for seed in 0..100u64 {
            if matches!(
                attempt_tame(&mut wolf, "local-player-0", seed),
                TameOutcome::Succeeded
            ) {
                break;
            }
        }
        assert!(wolf.is_tamed(), "should tame within 100 attempts");
        wolf.state = WolfAiState::FollowOwner;
        let (_next, action) =
            tick_wolf(&wolf, at((0.0, 0.0, 0.0)), Some((20.0, 0.0, 0.0)), 5);
        assert!(matches!(action, WolfAction::MoveToward { .. }),
            "a far tamed wolf should move toward its owner, got {action:?}");
    }

    #[test]
    fn untamed_wolf_idles_and_does_not_attack() {
        let wolf = WolfData::untamed();
        let (next, action) = tick_wolf(&wolf, at((0.0, 0.0, 0.0)), Some((10.0, 0.0, 0.0)), 0);
        assert_eq!(next.state, WolfAiState::Idle);
        assert_eq!(action, WolfAction::NoOp);
    }

    #[test]
    fn tamed_wolf_follows_owner_outside_max_distance() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let (_, action) = tick_wolf(&wolf, at((0.0, 0.0, 0.0)), Some((20.0, 0.0, 0.0)), 0);
        match action {
            WolfAction::MoveToward { x, .. } => assert!(x > 0.0),
            _ => panic!("wolf should chase owner at >8 blocks; got {action:?}"),
        }
    }

    #[test]
    fn tamed_wolf_does_not_chase_owner_inside_min_distance() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        // Owner is at the wolf's feet.
        let (_, action) = tick_wolf(&wolf, at((0.0, 0.0, 0.0)), Some((0.5, 0.0, 0.5)), 0);
        assert_eq!(
            action,
            WolfAction::NoOp,
            "wolf shouldn't oscillate when owner is right next to it"
        );
    }

    #[test]
    fn tamed_wolf_at_follow_boundary_does_not_oscillate() {
        // The wolf is at exactly FOLLOW_MAX_DISTANCE; we tick 50 times with
        // a stationary owner and assert the wolf doesn't bounce between
        // chasing and idling.
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let owner = (FOLLOW_MAX_DISTANCE, 0.0, 0.0);
        let mut wolf_pos = (0.0, 0.0, 0.0);
        let mut action_kinds = Vec::new();
        for tick in 0..50u64 {
            let (next, action) = tick_wolf(&wolf, wolf_pos, Some(owner), tick);
            wolf = next;
            // Apply a unit-step move toward the target if we got one.
            if let WolfAction::MoveToward { x, y, z } = action {
                let dx = x - wolf_pos.0;
                let dy = y - wolf_pos.1;
                let dz = z - wolf_pos.2;
                let len = (dx * dx + dy * dy + dz * dz).sqrt().max(0.001);
                wolf_pos.0 += dx / len;
                wolf_pos.1 += dy / len;
                wolf_pos.2 += dz / len;
            }
            action_kinds.push(std::mem::discriminant(&action));
        }
        // After settling, the wolf should be NoOp-ing (close enough).
        // Count transitions — should be a small finite number, not 50.
        let mut transitions = 0;
        for w in action_kinds.windows(2) {
            if w[0] != w[1] {
                transitions += 1;
            }
        }
        assert!(
            transitions <= 4,
            "wolf oscillated {transitions} times at follow boundary; expected ≤ 4"
        );
    }

    #[test]
    fn tamed_wolf_gives_up_follow_at_extreme_distance() {
        // Task 14 — GiveUpFollow was documented but never produced. A wolf
        // whose owner is ~200 blocks away (well past FOLLOW_GIVE_UP_DISTANCE)
        // should stop trying to path to them.
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let (next, action) =
            tick_wolf(&wolf, at((0.0, 0.0, 0.0)), Some((200.0, 0.0, 0.0)), 0);
        assert_eq!(
            action,
            WolfAction::GiveUpFollow,
            "wolf should give up chasing an owner 200 blocks away, got {action:?}"
        );
        assert_eq!(
            next.state,
            WolfAiState::Idle,
            "giving up follow should drop the wolf back to Idle"
        );
    }

    #[test]
    fn tamed_wolf_resumes_follow_once_owner_returns_in_range() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;

        // Owner wanders far enough away that the wolf gives up.
        let (gave_up, action) =
            tick_wolf(&wolf, at((0.0, 0.0, 0.0)), Some((200.0, 0.0, 0.0)), 0);
        assert_eq!(action, WolfAction::GiveUpFollow);
        assert_eq!(gave_up.state, WolfAiState::Idle);

        // Owner merely dips back under the give-up radius (20 blocks —
        // inside FOLLOW_GIVE_UP_DISTANCE but outside FOLLOW_MAX_DISTANCE):
        // hysteresis holds, the wolf keeps idling at its spot.
        let (still_idle, held) =
            tick_wolf(&gave_up, at((0.0, 0.0, 0.0)), Some((20.0, 0.0, 0.0)), 1);
        assert_eq!(
            still_idle.state,
            WolfAiState::Idle,
            "a gave-up wolf must not resume until the owner is back within \
             FOLLOW_MAX_DISTANCE (hysteresis)"
        );
        assert_eq!(held, WolfAction::NoOp);

        // Owner genuinely returns to the wolf (within FOLLOW_MAX_DISTANCE) —
        // the wolf resumes FollowOwner rather than staying idle forever.
        let (resumed, action2) =
            tick_wolf(&still_idle, at((0.0, 0.0, 0.0)), Some((5.0, 0.0, 0.0)), 2);
        assert_eq!(
            resumed.state,
            WolfAiState::FollowOwner,
            "wolf should resume FollowOwner once the owner is back in range"
        );
        assert_eq!(action2, WolfAction::NoOp, "resume tick itself holds position");

        // And from then on it follows normally again.
        let (_, action3) =
            tick_wolf(&resumed, at((0.0, 0.0, 0.0)), Some((20.0, 0.0, 0.0)), 3);
        assert!(
            matches!(action3, WolfAction::MoveToward { .. }),
            "a resumed wolf should chase its owner normally, got {action3:?}"
        );
    }

    #[test]
    fn tamed_wolf_at_give_up_boundary_does_not_oscillate() {
        // Mirror of tamed_wolf_at_follow_boundary_does_not_oscillate for the
        // give-up threshold: the owner hovers around FOLLOW_GIVE_UP_DISTANCE
        // (alternating one block either side of it, worst case for a
        // bufferless boundary) while the wolf stays put. With hysteresis the
        // wolf gives up once and then holds NoOp — it must not flap between
        // GiveUpFollow and MoveToward every tick.
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let wolf_pos = (0.0, 0.0, 0.0);
        let mut give_ups = 0;
        let mut moves = 0;
        for tick in 0..50u64 {
            let hover = if tick % 2 == 0 { 1.0 } else { -1.0 };
            let owner = (FOLLOW_GIVE_UP_DISTANCE + hover, 0.0, 0.0);
            let (next, action) = tick_wolf(&wolf, wolf_pos, Some(owner), tick);
            wolf = next;
            match action {
                WolfAction::GiveUpFollow => give_ups += 1,
                WolfAction::MoveToward { .. } => moves += 1,
                _ => {}
            }
        }
        assert_eq!(
            give_ups, 1,
            "wolf should give up exactly once at the boundary, not flap ({give_ups} give-ups)"
        );
        assert_eq!(
            moves, 0,
            "a gave-up wolf must not resume chasing while the owner hovers \
             at the give-up radius ({moves} moves)"
        );
        assert_eq!(wolf.state, WolfAiState::Idle, "wolf should still be idling");
    }

    #[test]
    fn untamed_wolf_drops_leather_and_one_or_two_bones() {
        let wolf = WolfData::untamed();
        let mut saw_one = false;
        let mut saw_two = false;
        for seed in 0..200u64 {
            let drops = drops_for_wolf(&wolf, seed);
            assert_eq!(drops.len(), 2);
            assert!(matches!(drops[0].item, crate::item::Item::Material(MaterialId::Leather)));
            match &drops[1].item {
                crate::item::Item::Material(MaterialId::Bone) => {}
                _ => panic!("second drop must be Bone"),
            }
            let bone_count = drops[1].count;
            assert!(bone_count == 1 || bone_count == 2);
            if bone_count == 1 { saw_one = true; }
            if bone_count == 2 { saw_two = true; }
        }
        assert!(saw_one && saw_two, "bone count should vary across seeds");
    }

    #[test]
    fn tamed_wolf_drops_nothing_emotional_loss_rule() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        let drops = drops_for_wolf(&wolf, 12345);
        assert!(drops.is_empty(), "tamed wolves must drop nothing (emotional loss rule)");
    }

    #[test]
    fn taming_succeeds_roughly_one_third_of_the_time() {
        let mut successes = 0;
        let attempts = 2000u64;
        for seed in 0..attempts {
            let mut wolf = WolfData::untamed();
            if matches!(attempt_tame(&mut wolf, "owner_pub", seed), TameOutcome::Succeeded) {
                successes += 1;
            }
        }
        let rate = successes as f32 / attempts as f32;
        assert!(
            (rate - 0.33).abs() < 0.05,
            "taming rate {rate} out of expected 0.33 ± 0.05"
        );
    }

    #[test]
    fn taming_rejected_when_already_tamed() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "first_owner".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let outcome = attempt_tame(&mut wolf, "second_owner", 0);
        assert_eq!(outcome, TameOutcome::AlreadyTamed);
        assert_eq!(wolf.ownership.owner_pubkey, "first_owner");
    }

    #[test]
    fn taming_rejected_when_owner_empty() {
        let mut wolf = WolfData::untamed();
        let outcome = attempt_tame(&mut wolf, "", 0);
        assert_eq!(outcome, TameOutcome::Failed);
        assert!(!wolf.is_tamed());
    }

    #[test]
    fn revenge_pivot_targets_attacker() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let new_state = maybe_pivot_to_revenge(&wolf, 42, false, 100);
        match new_state {
            Some(WolfAiState::AttackRecentAttacker { target_id, until_tick }) => {
                assert_eq!(target_id, 42);
                assert_eq!(until_tick, 100 + REVENGE_WINDOW_TICKS);
            }
            _ => panic!("expected AttackRecentAttacker, got {new_state:?}"),
        }
    }

    #[test]
    fn revenge_ignores_same_owner_tamed_wolf() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let new_state = maybe_pivot_to_revenge(&wolf, 42, true, 100);
        assert!(
            new_state.is_none(),
            "wolf must not attack another tamed wolf of the same owner"
        );
    }

    #[test]
    fn revenge_does_not_override_active_combat() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::AttackHostile {
            target_id: 7,
            until_tick: 200,
        };
        let new_state = maybe_pivot_to_revenge(&wolf, 42, false, 100);
        assert!(
            new_state.is_none(),
            "active combat should not be interrupted by a new revenge target"
        );
    }

    #[test]
    fn assist_pivot_joins_owner_fight() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let new_state = maybe_pivot_to_assist(&wolf, 42, false, 100);
        match new_state {
            Some(WolfAiState::AttackHostile { target_id, until_tick }) => {
                assert_eq!(target_id, 42);
                assert_eq!(until_tick, 100 + ASSIST_WINDOW_TICKS);
            }
            _ => panic!("expected AttackHostile, got {new_state:?}"),
        }
    }

    #[test]
    fn assist_ignores_same_owner_tamed_wolf() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::FollowOwner;
        let new_state = maybe_pivot_to_assist(&wolf, 42, true, 100);
        assert!(
            new_state.is_none(),
            "wolf must not attack another tamed wolf of the same owner even on assist"
        );
    }

    #[test]
    fn untamed_wolf_does_not_pivot_to_anything() {
        let wolf = WolfData::untamed();
        assert!(maybe_pivot_to_revenge(&wolf, 42, false, 100).is_none());
        assert!(maybe_pivot_to_assist(&wolf, 42, false, 100).is_none());
    }

    #[test]
    fn sitting_wolf_refuses_both_combat_pivots() {
        // Sit = a deliberate owner "stay here" command; a sitting wolf
        // must not self-pivot into revenge or assist (Task 8b wiring
        // relies on this gate living in the pure API).
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::Sit;
        assert!(maybe_pivot_to_revenge(&wolf, 42, false, 100).is_none());
        assert!(maybe_pivot_to_assist(&wolf, 42, false, 100).is_none());
    }

    #[test]
    fn sit_command_owner_only() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "alice".to_string();
        wolf.state = WolfAiState::FollowOwner;
        // Bob isn't the owner.
        assert!(wolf.try_toggle_sit("bob").is_none());
        assert_eq!(wolf.state, WolfAiState::FollowOwner);
        // Alice toggles to sit.
        let new_state = wolf.try_toggle_sit("alice");
        assert_eq!(new_state, Some(WolfAiState::Sit));
        // Alice toggles back.
        let back = wolf.try_toggle_sit("alice");
        assert_eq!(back, Some(WolfAiState::FollowOwner));
    }

    #[test]
    fn sit_command_rejected_on_untamed_wolf() {
        let mut wolf = WolfData::untamed();
        assert!(wolf.try_toggle_sit("anyone").is_none());
    }

    #[test]
    fn attack_window_expires_and_state_reverts() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::AttackHostile {
            target_id: 7,
            until_tick: 50,
        };
        // Tick after the window expires.
        let (next, action) = tick_wolf(&wolf, (0.0, 0.0, 0.0), Some((0.0, 0.0, 0.0)), 60);
        // Reverts to FollowOwner because the wolf is tamed.
        assert_eq!(next.state, WolfAiState::FollowOwner);
        // No attack action emitted on the expiry tick — wolf is back
        // in follow mode.
        assert!(!matches!(action, WolfAction::AttackTarget { .. }));
    }

    #[test]
    fn attack_window_active_yields_attack_action() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::AttackHostile {
            target_id: 7,
            until_tick: 100,
        };
        let (_, action) = tick_wolf(&wolf, (0.0, 0.0, 0.0), Some((10.0, 0.0, 0.0)), 50);
        assert_eq!(action, WolfAction::AttackTarget { entity_id: 7 });
    }

    #[test]
    fn sit_state_short_circuits_to_no_action() {
        let mut wolf = WolfData::untamed();
        wolf.ownership.owner_pubkey = "abc".to_string();
        wolf.state = WolfAiState::Sit;
        let (next, action) = tick_wolf(&wolf, (0.0, 0.0, 0.0), Some((50.0, 0.0, 50.0)), 0);
        assert_eq!(next.state, WolfAiState::Sit, "sit holds even if owner is far");
        assert_eq!(action, WolfAction::NoOp);
    }
}

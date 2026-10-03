# Wolves — Tameable Companion Mob — Spec 28d.wolves

**Status:** DELIVERED as of 2026-05-21 on main. `MobType::Wolf` + `wolf.rs` (~607 LOC) — 5-state AI (idle / follow / sit / attack-hostile / attack-recent-attacker) + bone-taming + owner pubkey + pet list on `PlayerSlot` + emotional-loss drops (tamed: nothing). Drove the `tameable.rs` framework extraction (PR #49) for future Cat/Parrot/etc. Phase 9 (Axolittle playtest) remains the validation gate.
**Branch (when building):** `feat/wolves-build` off `main`.
**Trigger:** Sub-foundation of [Spec 28 Minecraft-parity content surface](2026-05-20-minecraft-parity-content-surface.md) §4 — sister to the seven species in [Spec 28d Mob Roster](2026-05-20-mob-roster-expansion.md). Builds out the **tameable mob framework** that future companion species (Cat, Parrot, Fox, future pets) consume.

---

## TL;DR

Add **wolves** as a tameable companion mob with five AI states (idle / follow-owner / sit / attack-hostile-mobs / attack-recent-attacker). Right-click with bone tames (33% per click, hearts on success). Tamed wolves follow the owner within 8 blocks, sit on right-click, attack what the owner attacks, and attack mobs that hit the owner. Untamed wolves drop leather + 1-2 bones on death; **tamed wolves drop nothing** (emotional weight — killing a tamed wolf is loss, not loot).

Owner identity is carried by pubkey on the mob component so a wolf knows whose it is across save/load and across player sessions. The PlayerSlot maintains a small pet-list for quick lookup.

**Scope:** ~1,500 LOC + ~2 textures across 9 phases. Phase 9 is the Axolittle playtest gate (wolf "feel" is the failure surface — does follow-distance feel right, do attacks discriminate correctly).

---

## Why this lives here

- **Wolves are the canonical first pet.** Every Minecrafter expects them; the gap is felt immediately on first contact with the mob roster.
- **Tameable-mob mechanics are a missing primitive.** The engine has hostile (zombie, skeleton, creeper), passive (cow, pig, sheep), and neutral (placeholder), but no tameable state. Wolves are the consumer that justifies extracting `tameable.rs` for future Cat / Parrot / Horse.
- **Emotional-loss design choice:** untamed wolves drop loot; tamed wolves don't. This isn't accidental — it's the rule that makes wolves feel like companions instead of mobile inventory chests. Document loudly in the spec so a future tweak doesn't quietly revert it.
- **Cross-game lift:** the tameable-state pattern (right-click-to-interact mob, owner pubkey, follow-distance, attack-discrimination) is engine-generic. Any Decented game with companion mechanics consumes the same framework.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/mob.rs` — append `MobType::Wolf` (positional bincode; append-only).
- `game/engine/src/mob_ai.rs` — per-species AI dispatch; add `tick_wolf` case routing to the five states.
- `game/engine/src/entity_model.rs` — wolf `ModelParts` (body + head + 4 legs + tail).
- `game/engine/src/spawning.rs` — biome-weight lookup (when Spec 28a Biomes lands; until then, spawn rate is flat).
- `game/engine/src/player_slot.rs` — new `tamed_pets: Vec<PetRef>` field (small list, owner-pubkey-keyed mob ids).
- `game/engine/src/save.rs` — serialise the wolf's `owner_pubkey` + `ai_state` + tamed flag on save; restore on load.
- `game/engine/src/block_interact.rs` (or wherever right-click is dispatched) — bone-on-wolf triggers taming attempt.

### New modules

- `game/engine/src/wolf.rs` — wolf-specific AI + taming + drop helpers. Pure functions where possible.
- (Framework extract, **deferred to its own PR after wolves ships**) `game/engine/src/tameable.rs` — abstracts the wolf state machine for future Cat/Parrot/Horse.

### Related specs

- 28d Mob Roster — sister spec.
- Spec 5 §4.2 — mob spawn rules; wolves use the biome-aware spawn pool once 28a lands.
- Spec 5 §3 — combat damage path; wolf attack damage flows through here.

### Memory pointers

- uk english naming — "Wolf" not "Doggo"; "Tamed Wolf" the inventory hover label.
- shared infra strategy — extract tameable framework after the live consumer (wolves) works.
- autonomy to playtest boundary — wolf AI feel (follow / attack discrimination) is the playtest gate; deliver everything testable, stop at the visual sign-off.

---

## Phasing

| # | Phase | Files | LOC | Solo? |
|---|---|---|---|---|
| 1 | **This spec** | this doc | — | — |
| 2 | `MobType::Wolf` enum entry + `wolf.rs` skeleton (`AiState` enum, `WolfData` struct, `tick_wolf` signature) | `mob.rs`, `wolf.rs` | ~120 | ✓ |
| 3 | Per-state AI logic — `tick_idle` (random wander, 10% chance per second), `tick_follow_owner` (path toward owner if > 8 blocks), `tick_sit` (no-op, only the right-click pet/unpet toggles), `tick_attack_hostile` (move toward + within reach attack at 2.0 HP/hit), `tick_attack_recent_attacker` (90-tick window after owner takes damage) | `wolf.rs` | ~350 | ✓ |
| 4 | Taming — bone-on-wolf right-click path. Pure function `attempt_tame(wolf, rng_seed) -> TameOutcome` with 33% success per attempt. On success: set owner_pubkey, swap AiState to FollowOwner, emit hearts particle event. On fail: emit smoke particles. | `wolf.rs`, `block_interact.rs` | ~200 | ✓ |
| 5 | PlayerSlot pet-list — `tamed_pets: Vec<PetRef { mob_id: u64, name: String }>`. Save/load round-trip. | `player_slot.rs`, `save.rs` | ~150 | ✓ |
| 6 | Right-click-on-tamed-wolf toggles sit/stand. Owner-only — non-owners get a "not your wolf" toast. | `wolf.rs`, `block_interact.rs` | ~120 | ✓ |
| 7 | Drops — `drops_for(MobType::Wolf, tamed) -> Vec<ItemStack>`. Untamed: leather + 1-2 bones (seeded). Tamed: empty. | `mob.rs::drops_for` (extended signature) | ~80 | ✓ |
| 8 | Tests — AI doesn't infinite-loop, follow stays ≤ 8 blocks, taming succeeds at ~33% over many trials (seed-deterministic), attack-discrimination ignores tamed wolves of the same owner, drops table empty for tamed | `wolf.rs::tests` | ~280 | ✓ |
| 9 | Axolittle playtest — spawn wolves in a Forest, tame one with bones, follow on a walk, find a hostile mob, observe the wolf attacks it; attack the wolf with your sword, observe owner doesn't get auto-attacked back | — | — | playtest gate |

**Total Phases 2-8:** ~1,300 LOC + tests. Phase 9 is the gate.

**Phase F (post-PR, separate branch):** Extract `tameable.rs` framework from `wolf.rs` once wolf behaviour is signed off in playtest. Refactor wolf to consume the framework. Adds Cat/Parrot/Horse as future trivial additions.

---

## §2 — Wolf data & state machine

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WolfAiState {
    Idle,
    FollowOwner,
    Sit,
    AttackHostile { target_id: u64, until_tick: u64 },
    AttackRecentAttacker { target_id: u64, until_tick: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WolfData {
    pub state: WolfAiState,
    /// Hex-encoded pubkey of the owner. Empty string = untamed.
    pub owner_pubkey: String,
    /// Last tick the owner took damage; drives the 90-tick attack-attacker window.
    pub last_owner_damage_tick: u64,
    /// Last tick the owner used a sword to hit a target; drives attack-hostile.
    pub last_owner_attack_tick: u64,
}

impl WolfData {
    pub fn untamed() -> Self { ... }
    pub fn is_tamed(&self) -> bool { !self.owner_pubkey.is_empty() }
}
```

State transitions:
- Idle → FollowOwner: tamed, owner > 4 blocks away.
- FollowOwner → Idle: tamed but Sit toggled or owner < 1 block.
- Idle → Sit: tamed, right-click by owner.
- Any → AttackHostile: tamed, owner attacked a non-tamed hostile in last 30 ticks → wolf joins.
- Any → AttackRecentAttacker: tamed, owner just took damage from `entity_id` → wolf chases that entity.

Untamed wolves: only Idle (random wander), with no Sit/Follow/Attack transitions.

---

## §3 — AI tick (deterministic)

Each state's tick is a pure function: `(WolfData, Mob, &World, &[PlayerSlot]) -> (WolfData, Vec<MobAction>)`. `MobAction` covers `Move(Vec3)`, `AttackTarget(u64)`, `EmitParticles(ParticleKind, Vec3)`, `NoOp`.

Determinism: state transitions seeded by `(mob_id, tick)` — same world state → same wolf actions. Lets us test attack-discrimination + follow-distance precisely.

Anti-loop test: `tick_wolf_does_not_oscillate_at_8_block_boundary` — spawn owner at exactly 8 blocks from wolf, tick 100 times, assert position deltas don't bounce > 1 block per tick on average.

---

## §4 — Taming

```rust
pub fn attempt_tame(wolf: &mut WolfData, owner_pubkey: &str, seed: u64) -> TameOutcome {
    if wolf.is_tamed() { return TameOutcome::AlreadyTamed; }
    let roll = hash_seed_to_unit_f32(seed);
    if roll < 0.33 {
        wolf.owner_pubkey = owner_pubkey.to_string();
        wolf.state = WolfAiState::FollowOwner;
        TameOutcome::Succeeded
    } else {
        TameOutcome::Failed
    }
}
```

Consumed bone is decremented from the player's hotbar regardless of outcome (matches Minecraft — failed taming still uses the bone, that's the cost).

Particles: hearts (success) / smoke (failure) emitted from the wolf's head position. Particle system already exists in the engine; reuse.

---

## §5 — PlayerSlot pet list

```rust
pub struct PetRef {
    pub mob_id: u64,
    /// Per-player display name. Defaults to "Wolf"; future renaming UI.
    pub name: String,
}

// On PlayerSlot:
pub tamed_pets: Vec<PetRef>,
```

Save/load: serialise via existing bincode-positional pattern. When a tamed wolf despawns or dies, prune from the list.

Capped at 8 pets per player initially (anti-clutter; UI can show 8 max in a future Pets panel).

---

## §6 — Right-click handler

When the player right-clicks a wolf:

1. Is it tamed AND not owned by you? → toast "Not your wolf" (UK English), no state change.
2. Is it tamed AND owned by you? → toggle Sit ↔ FollowOwner. Emit a subtle "ack" sound.
3. Is it untamed? → if you're holding a bone, run `attempt_tame`. Otherwise no-op.

---

## §7 — Drops

```rust
// Extended drops_for signature
pub fn drops_for(mob_type: MobType, tamed: bool, seed: u64) -> Vec<ItemStack> {
    match (mob_type, tamed) {
        (MobType::Wolf, true) => Vec::new(),
        (MobType::Wolf, false) => {
            let mut out = vec![ItemStack::new_material(MaterialId::Leather, 1)];
            let bone_count = (hash_seed_to_u8(seed) % 2) + 1;  // 1 or 2
            out.push(ItemStack::new_material(MaterialId::Bone, bone_count));
            out
        }
        _ => existing_table(mob_type),
    }
}
```

Existing call sites pass `tamed = false` by default; only wolves carry the tamed-aware branch. Other tameable species (Cat/Parrot) will follow the same shape when they land via the framework extract.

---

## §8 — Tests

```rust
#[test] fn untamed_wolf_idles_doesnt_seek_player() { ... }
#[test] fn tamed_wolf_follows_owner_within_8_blocks() { ... }
#[test] fn tamed_wolf_does_not_oscillate_at_follow_boundary() { ... }
#[test] fn taming_success_rate_is_one_third_over_1000_trials() { ... }
#[test] fn taming_consumes_bone_even_on_failure() { ... }
#[test] fn tamed_wolf_attacks_recent_owner_attacker_within_window() { ... }
#[test] fn tamed_wolf_attacks_hostile_owner_just_hit() { ... }
#[test] fn tamed_wolf_does_not_attack_other_tamed_wolf_same_owner() { ... }
#[test] fn tamed_wolf_drops_nothing_on_death() { ... }
#[test] fn untamed_wolf_drops_leather_plus_one_to_two_bones() { ... }
#[test] fn pet_list_persists_across_save_load() { ... }
#[test] fn non_owner_right_click_shows_not_your_wolf_toast() { ... }
```

---

## §9 — Axolittle playtest

1. `/give bone` x 10.
2. `/tp` to a Forest (or wait for biome spawn).
3. Find a wolf (Forest weight 2.0).
4. Right-click with bone. Repeat until hearts appear (~3 tries on average).
5. Walk 20 blocks. Confirm the wolf follows but doesn't crowd.
6. Approach a zombie with sword drawn. Hit it. Wolf joins the fight.
7. Let a creeper hit you. Wolf chases the creeper for ~90 ticks.
8. Right-click the wolf. Wolf sits. Walk away. Confirm wolf stays.
9. Right-click again. Wolf rejoins.
10. Kill the wolf with sword. Confirm: no drops, name removed from pet list (`/tameable list` if implemented).

Questions:
- Does the follow distance feel right (8 blocks) or too close / too far?
- Does the wolf attack-discriminate correctly (ignore my other tamed wolves)?
- Does the no-drops-on-tamed feel like loss, or just annoying?
- Heart particles big enough to confirm taming succeeded?

---

## Acceptance — sub-foundation 28d.wolves overall

- `./check.sh` ALL GREEN.
- All Phase 8 tests pass.
- Manual playtest checks above succeed.
- Player Guide page added (`tools/sites/docs/content/player-guide/wolves.md`).
- Foundations README updated.
- Spec 28d master spec + Spec 28 master spec updated to remove the "wolves deferred to v2" callout.
- Memory note (wolves no longer deferred) written.
- Tameable-mob framework extraction (`tameable.rs`) queued as a follow-up PR after playtest.

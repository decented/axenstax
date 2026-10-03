# Historical Pivot — Sub-Foundation 4: Knights

**Status:** READY TO BUILD (sequenced after HP-3). **Prerequisite:** HP-3 (Brigand Hideouts) merged so brigand-family mobs exist as the Knight's primary engagement target; HP-2 + Spec 19 already provide the Iron-Golem precedent the Knight AI lifts.
**Branch:** `feat/historical-pivot-sub4-knights` off `main`.
**Trigger:** Sub-foundation **4** of the [Historical Pivot Long-Run](../vision/historical-pivot-long-run.md). Replaces the Iron Golem's role as village defender — Knight ships alongside the Iron Golem in HP-4; HP-6 cutover removes the golem.

---

## TL;DR

One new `MobType::Knight` village defender mob. Auto-spawns when a village meets the existing Spec 19 trigger (≥ 3 claimed villagers + ≥ 5 houses), in addition to the Iron Golem during the HP-3→HP-6 transition. Reuses the existing `AiState::GolemGuard` state + `tick_golem_combat` dispatcher with a small tier-aware extension — same patrol-radius / defend-radius / melee-cooldown contract, different damage value and entity model.

All-enemies-die-on-HP-0 per the historical pivot. Puff-of-smoke on defeat. No new combat plumbing; the Knight is mechanically a re-skinned Golem with human bipedal proportions + a tighter HP wall (60 vs 100). HP-6 cutover retires the Iron Golem; from then on the Knight is the sole village defender.

---

## Why this lives here

- **Closes the human-defender side of the historical pivot.** With brigands attacking villages (HP-5) we need human defenders, not iron constructs.
- **Lifts the GolemGuard primitive cross-game.** The existing `AiState::GolemGuard` + `tick_golem_combat` was already engine-generic — HP-4 makes it explicit by giving it a second consumer with different stats. Future sibling games can plug in their own equivalent without further engine work.
- **Coexists cleanly with the Iron Golem.** Both spawn in the same village; players see "Knights and Golems are both defending today." HP-6 quietly drops the golem; the village's defensive feel doesn't crater.

---

## What this PR ships

### 1. `MobType::Knight`

| Stat | Value |
|---|---|
| Category | Passive (matches Iron Golem — friendly to players, hostile to hostile mobs) |
| HP | 60 |
| Damage / hit | 8 |
| Speed | 4.0 b/s (human walk) |
| Width / Height | 0.65 / 1.95 |
| Colour | `[0.80, 0.80, 0.82]` (steel grey) |
| Drops on defeat | 2-3 IronIngot + 0-1 Leather |

Defined in `data/mobs/knight.toml`. Appended after Berserker in `MobType` + `DEFS` so wire indices stay stable.

### 2. AI — reuses `AiState::GolemGuard`

Knight spawns with `AiState::GolemGuard { home_x, home_z }` where home is the village anchor — same as Iron Golem. `tick_mob_ai`'s `GolemGuard` arm needs no changes; the patrol-toward-home + plod-with-turn logic is mob-kind-agnostic.

### 3. Combat — extend `tick_golem_combat`

`mob_ai::tick_golem_combat` currently filters by `MobType::IronGolem`. HP-4 generalises this:

```rust
fn is_village_defender(kind: MobType) -> bool {
    matches!(kind, MobType::IronGolem | MobType::Knight)
}
```

The function loops every defender, finds the nearest hostile mob (any `MobCategory::Hostile`) within `GOLEM_DEFEND_RADIUS`, applies `GOLEM_DAMAGE` if within `GOLEM_MELEE_REACH`. Knight uses a tier-specific damage value (8 instead of 7) — the function reads damage off a per-mob helper `defender_damage(kind: MobType) -> f32` so future defender tiers (e.g. Captain Knight at v2) only need to add an arm.

### 4. Auto-spawn — new `tick_knight_spawn`

Mirrors `village_gen::tick_iron_golem_spawn` exactly:

- For each village with the population threshold met (Spec 19's existing rule: ≥ 3 claimed villagers + ≥ 5 houses), check if a Knight already exists within the village radius. If not, spawn one at the village campfire position.
- Cap: `max(1, villagers / 10)` knights per village — same scaling as Iron Golem cap so large villages can hold multiple.
- During the HP-4→HP-6 window, **both** spawn-tick passes run. A 5-house village can have one Iron Golem AND one Knight simultaneously. This is intentional; HP-6 removes the golem half.

Wired into `game_loop.rs` + `server.rs` alongside the existing `tick_iron_golem_spawn` call (same 20-tick cadence).

### 5. Wire format

- `EntityKind::Knight = 25` appended to `protocol.rs::EntityKind`.
- `PROTOCOL_VERSION: 26 → 27` bump.
- `MobType::Knight → EntityKind::Knight` mapping added to `hosted_server.rs`.

### 6. Entity model

Reuses the villager mesh (bipedal human) with `MobDef.color = steel grey`. Same shortcut HP-3 took for brigand tiers. Dedicated knight mesh (with visible chainmail) deferred to post-playtest polish.

---

## What this PR does NOT do

- Doesn't remove the Iron Golem. Sub 6.
- Doesn't change Spec 19's village population threshold rules.
- Doesn't add a Knight equipment ladder (Iron sword vs Steel sword vs Greatsword tiers tied to village reputation tier was floated in the vision doc's "Open design questions". v2.)
- Doesn't add Knight-vs-brigand-tier matchup advantages — damage is flat regardless of target's tier.
- Doesn't add anti-friendly-fire (a Knight will still hit a sneaky Villager if some weird AI bug routes them into melee range; same risk Iron Golem already carries).
- Doesn't add new sats payouts. Iron from kills routes through existing vendor / inventory; the existing reputation-loss-on-defender-kill is what gates farming.
- Doesn't introduce a Knight-helmet item drop. Killing a Knight gives iron + leather; no Knight-specific armour piece v1.

---

## Phasing

| # | Phase | Files | LOC est |
|---|---|---|---|
| 1 | **This spec** | — | — |
| 2 | New mob TOML + `MobType::Knight` + `drops_for` + DEFS + tests | `mob.rs`, `data/mobs/knight.toml` | ~80 |
| 3 | Wire-format: `EntityKind::Knight = 25`, PROTOCOL_VERSION 26→27, hosted_server mapping | `protocol.rs`, `hosted_server.rs`, `test_integration/handshake.rs` | ~40 |
| 4 | Combat — extract `is_village_defender` + `defender_damage`; loop generalised in `tick_golem_combat` | `mob_ai.rs` | ~70 |
| 5 | New `tick_knight_spawn` in `village_gen.rs` + wire into game_loop + server | `village_gen.rs`, `game_loop.rs`, `server.rs` | ~110 |
| 6 | Entity model — Knight maps to `villager_model` with steel-grey tint via TOML | `entity_model.rs` | ~10 |
| 7 | Tests (mob def + drops + defender filter + spawn at threshold + multi-spawn cap + Knight-takes-hostile-damage in golem-combat) | various `*::tests` | ~150 |
| 8 | Docs (HP-4 spec → DELIVERED, README flip, Spec 5 §3 mobs append) | docs | ~20 |
| 9 | Axolittle playtest | — | — |

**Total**: ~480 LOC. Single PR. Light on engine surface — most of the work is the spawn-tick + the combat-function generalisation.

---

## Tests

```rust
#[test] fn knight_baseline_stats_match_spec() { /* HP=60, damage check via defender_damage */ }
#[test] fn knight_is_passive_with_human_proportions() { /* category, h/w */ }
#[test] fn knight_drops_iron_and_leather_in_range() {
    for seed in 0..200 {
        let drops = drops_for(MobType::Knight, seed);
        let iron: u8 = ... ; assert!(iron >= 2 && iron <= 3);
        let leather: u8 = ... ; assert!(leather <= 1);
    }
}

#[test] fn defender_damage_iron_golem_is_seven() { ... }
#[test] fn defender_damage_knight_is_eight() { ... }
#[test] fn is_village_defender_covers_both() { ... }
#[test] fn tick_golem_combat_damages_hostile_when_knight_in_range() {
    // place Knight + Brigand in melee reach; tick; assert Brigand HP dropped by 8.
}

#[test] fn knight_spawns_when_threshold_met_and_none_present() { /* matches Iron Golem test pattern */ }
#[test] fn knight_does_not_spawn_below_threshold() { /* < 3 villagers OR < 5 houses */ }
#[test] fn knight_and_iron_golem_coexist_in_same_village() { /* both spawn; one of each */ }
#[test] fn knight_spawn_cap_scales_with_villager_count() { /* 10 villagers → 1 knight; 20 → 2 */ }
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- All Phase 2-7 tests pass.
- In a test world with a 5-house village, `tick_knight_spawn` produces exactly one Knight on its first qualifying tick.
- Adding a second Iron Golem doesn't block Knight spawning (and vice versa).
- A Knight in melee range of a Brigand applies 8 damage per `GOLEM_ATTACK_COOLDOWN`.
- A Knight ignores Villagers + players (passive category).
- Phase 9 (playtest): Axolittle confirms the village feels defended; Knight is visually distinct from Iron Golem and from Villager.

---

## Memory-rule check

- ✓ uk english naming — "Knight", "armour", "defender" all UK English.
- ✓ bitcoin parent controlled — orthogonal; no new sats payouts.
- ✓ shared infra strategy — `is_village_defender` + `defender_damage` are engine-generic; the Knight is a second consumer of the existing GolemGuard primitive, validating the cross-game-lift design.
- ✓ merge to main preauthorised — healthy-gate merge when `check.sh` is green.
- ✓ engine commands — orthogonal; no command-system changes.

---

## Out of scope (deferred to later subs or v2)

- Iron Golem removal — Sub 6 (HP-6 migration cutover).
- Knight equipment ladder (sword tier scaling with village reputation) — v2.
- Knight visual mesh with chainmail + helmet detail — post-playtest polish.
- Knight-vs-brigand-tier matchup damage scaling — v2.
- Knight-helmet item drop — v2.
- "Captain Knight" elite tier — v2.

---

## Open questions

None. All design decisions resolved during HP-4 brainstorming (autonomous-mode call).

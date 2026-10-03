# Mob Roster Expansion — Spec 28d

**Status:** PARTIALLY DELIVERED. Wolves shipped 2026-05-21 (see `2026-05-20-wolves-tameable-companion.md`). **2026-05-27:** Horse, Goat, Rabbit, Bee, Squid (plus Nostrich, which already had AI) had spawn logic + AI but **no render model**, so they spawned invisible — a playtest found only cow/pig/chicken were ever seen. All seven now have box models in `entity_model.rs` (reusing existing coat textures; bespoke per-species textures — esp. bee-yellow + squid-blue — are a deferred polish pass). Guarded by `every_passive_spawn_mob_has_a_model`. **Remaining:** hostile additions (Skeleton archer / Wither Skeleton) + the texture polish pass + per-species AI tuning after Axolittle's playtest.
**Branch (when building):** one PR per mob species — `feat/spec-28d-{species}` off `main`.
**Trigger:** Sub-foundation of [Spec 28 Minecraft-parity content surface](2026-05-20-minecraft-parity-content-surface.md) §4. Independent of 28a-c at the type-system level; depends on 28a for biome-specific spawn weighting.

---

## TL;DR

Add 7 new mob species to fill the roster gap:

- **Passive — terrestrial**: Horse (rideable post-alpha; spawnable now without saddle), Goat (mountain-spawn — needs taller terrain; alpha-OK on Plains), Rabbit (small + fast — tests "small entity" feel).
- **Neutral**: Bee (pollinates flowers; produces Honeycomb when raided — 28c Honeycomb material).
- **Hostile — terrestrial**: Skeleton archer (existing? — verify; if absent, add), Wither Skeleton (cave-only at depth + on bone tile).
- **Aquatic**: Squid (water-only spawn; drops Ink Sac).

**Wolves lifted from v2-deferral 2026-05-20** — spec'd separately at [`28d.wolves`](2026-05-20-wolves-tameable-companion.md). Wolves drive the tameable-mob framework that future Cat / Parrot / Horse companions consume. Still deferred to v2: Cat, Llama, Iron Golem (Cat lands once `tameable.rs` is extracted from the wolf build; Llama and Iron Golem need building/breeding lore not on alpha scope).

**Scope:** ~2,800 LOC across 14 phases (2 per species). Each species is one PR. Playtest gate per species, not per spec.

---

## Why this lives here

- Axolittle's check: "Minecraft has ~30 mobs; AxeNStax has ~12". The roster gap is felt every time the player goes for a walk and sees the same 4 things. This is gameplay-affecting, not cosmetic.
- Per-species PRs because mob AI is the most playtest-sensitive surface — bee pollination feels nothing like rabbit hopping; we don't want to bundle them.
- Cross-game lift: the per-species AI pattern via `MobAi::tick(mob, world, players) -> MobAction` is engine-generic. Sister games consume by registering different species in their MobRegistry.

---

## Context pointers

### Existing surfaces

- `mob_ai.rs` — per-species AI dispatch. Add cases for each new species.
- `mob.rs::MobType` — enum; append-only (positional bincode).
- `mob.rs::drops_for` — drop table; one row per new species.
- `entity_model.rs` — per-species cached `ModelParts` via LazyLock. Add model definitions.
- `spawning.rs::tick_mob_spawning` — biome weight lookup (via 28a) chooses which mobs to attempt to spawn at a candidate location.

### New modules

None — additive into existing.

### Related specs

- 28a Biomes §3 — biome-weighted spawn pool.
- 28c Materials — Honeycomb, Egg, Ink Sac drops_for entries fire here.
- Spec 5 §4 — mob spawn cap, despawn distance.

### Memory pointers

- Wolves explicitly deferred per Spec 28 master spec (page §4 of `2026-05-20-minecraft-parity-content-surface.md`) — do not include in 28d.
- UK English: "Skeleton archer" not "skellie"; "Rabbit" not "Bunny".

---

## Phasing — per species

Each species follows the same shape; here's the template, applied 7 times:

| # | Phase | Files | LOC | Solo? |
|---|---|---|---|---|
| A | `MobType::<NewSpecies>` enum entry | `mob.rs` | +5 | ✓ |
| B | `ModelParts` definition (parts + bones + dimensions) | `entity_model.rs` | +80 | ✓ |
| C | AI dispatch — per-tick `MobAi::tick_<species>` | `mob_ai.rs` | +120 | ⚠ playtest |
| D | drops_for + loot table | `mob.rs::drops_for` | +20 | ✓ |
| E | Biome spawn weights — update 28a's table to include the new species | `biome.rs::BIOMES` | +15 | ✓ |
| F | Tests — spawning produces this species in the expected biome, drops fire, AI doesn't infinite-loop | `*.rs::tests` | +80 | ✓ |
| G | Axolittle playtest — find one in the wild, observe behaviour, kill one, confirm drops | — | — | playtest gate |

**Total per species:** ~320 LOC + playtest. Build slow; playtest-gate each one before moving to the next.

---

## §2 — Species-specific notes

### Horse

- **Passive**, idle wanders + breaks into trot if player approaches (~10-block trigger).
- Drops: nothing on alpha (Leather drop after death — already in MaterialId). Saddle ride post-alpha.
- Biome: Plains weight 1.5, Savanna 1.5, others 0.
- Model: Horse-body (rectangular), 4 thin legs, head with mane.

### Goat

- **Passive but aggressive** — random chance to charge a player if within 3 blocks. Kicks them back ~4 blocks.
- Drops: Mutton (existing) + 5-15% Horn (new MaterialId — defer to v2).
- Biome: Snowy Tundra 2.0, Savanna 0.5, others 0.
- Model: small body, distinctive curled horns.

### Rabbit

- **Passive**, hop-locomotion (jump every 4 ticks while moving). Despawns if not within 64 blocks of player after 5 minutes.
- Drops: Raw Rabbit (new MaterialId — add) + Rabbit Hide (new MaterialId).
- Biome: Plains 1.0, Desert 1.5, Snowy Tundra 1.0.
- Model: small body, long ears, short tail.

### Bee

- **Neutral**, pursues flowers (block ID `tall_grass` placeholder until proper flower blocks). Attacks if hive disturbed.
- Drops: Honey Bottle (after Bee dies; 50%) — uses 28c's `MaterialId::Honey`.
- Hive integration: Honeycomb material in 28c only fires when player breaks a Bee Hive (new block — deferred to Phase H below).
- Biome: Plains 1.0, Forest 1.5, Jungle 0.5.
- Model: small flying body with wings; flight AI distinct from walking — uses entity Y-position not on_ground.

### Skeleton archer

- **Hostile night-only**. Existing — verify; if absent, add.
- Drops: Bone (existing), Arrow (existing).
- Biome: all (default 1.0).

### Wither Skeleton

- **Hostile**, deepslate-cave-only. Stronger than Skeleton. Drops: Bone + 1-2% Wither Skull (new MaterialId — defer if needed).
- Biome: deep underground only — spawn condition is `y < -32 && light < 8`, biome weight ignored.

### Squid

- **Passive aquatic**. Spawns inside `WATER` block. Sink/swim AI.
- Drops: Ink Sac (28c material).
- Biome: any with surface water.

---

## §3 — Bee Hive block (sub-phase H)

Bees + their drop loop need a Hive block. Phases:

H1: Add `BEE_HIVE` BlockId. Place near a Bee swarm spawn (chance 5% in Plains/Forest).
H2: Right-click Bee Hive with empty hand + Shears (new tool — defer) → drops Honeycomb. Right-click with Glass Bottle → drops Honey Bottle.
H3: Bees periodically deposit honey into nearby hives (~once per 3 minutes per active bee within 16 blocks of a hive).

H is the most-cut sub-phase of 28d — Bee + Hive is ~600 LOC by itself. Bee species can land without Hive (Bee just wanders + dies + drops Honeycomb directly on kill); Hive system layers in.

---

## §4 — Tests

```rust
#[test]
fn each_new_mob_has_model_parts() {
    for mob_type in [MobType::Horse, MobType::Goat, ...] {
        let parts = ModelParts::for_mob(mob_type);
        assert!(!parts.parts.is_empty(), "Mob {:?} has no model", mob_type);
    }
}

#[test]
fn drops_for_each_new_mob_is_non_empty() { /* skeleton verify */ }

#[test]
fn squid_only_spawns_in_water() {
    let attempts = simulate_spawn_attempts(MobType::Squid, &test_world, 1000);
    assert!(attempts.iter().all(|pos| is_water(*pos)));
}
```

---

## §5 — Axolittle playtest checklist (per species)

For each new species:
1. `/tp` to its preferred biome.
2. Wait 3 minutes; confirm at least one spawns.
3. Observe behaviour for 30 seconds. Does it walk / hop / fly / swim correctly? Does it pursue / flee / pollinate?
4. Approach. Trigger the appropriate response (attack, flee, charge).
5. Kill it. Confirm drops appear in the inventory.

Questions per species:
- Does the AI feel right or buggy?
- Is the model too big / too small relative to existing mobs?
- Are drops at the right rate?

---

## Acceptance — sub-foundation 28d overall

- `./check.sh` ALL GREEN for each species' PR.
- Per-species playtest gate satisfied.
- Player Guide pages added per species.
- Foundations README updated incrementally.
- Wolves explicitly NOT added (v2 task).

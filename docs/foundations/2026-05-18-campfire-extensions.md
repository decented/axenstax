# Foundation — Campfire Extensions: Smoke Signal + Baking + Heat-Aware Mobs

**Status:** READY TO BUILD
**Date:** 2026-05-18
**Author:** Staxolottle (design) + Claude (implementation)
**Parent:** Spec 17 — [Campfire](2026-05-18-campfire.md) (delivered)

## TL;DR

Three independent extensions to the campfire that landed yesterday:

1. **Smoke signal** — burning leaves emit a tall vertical smoke column visible from far away. Use case: "where's my friend?" — a quick visual ping across the world. Made of a new non-solid block (`CAMPFIRE_SMOKE`, id 45); spawned + cleared by the campfire tick.
2. **Basic baking** — Potato + Carrot + Corn → Baked variants at the campfire. Closes the loop between Wave 26 farming and the cooking station. **Corn is a new crop** added in this spec (4 growth stages, matches the Wheat/Carrot/Potato pattern; single-block-tall first pass — the "on-the-cob" experience is in the harvest+bake loop, not plant height).
3. **Heat-aware mobs** — mobs see the smoke + glow and walk toward the campfire, but stop at a heat-radius proportional to fuel level. Hostile mobs still chase a player who passes nearer than the campfire. Replaces the lazy "fire deters mobs" Minecraft trope with a beacon-attracts-but-heat-holds-them-back mechanic the player can use tactically.

## Narrative

A campfire isn't a panic button — it's a beacon. Light a fire, you can cook and see at night, but you've also told the world where you are. Sheep walk over to inspect. Zombies amble in slowly. The fire keeps them at a distance, but they're there, watching. **More fuel = more heat = wider safe radius, but also brighter smoke = bigger draw.** The player decides between a small kindling fire (quiet, no draw, no protection) and a big roaring fire (visible, but holds a real ring of clear ground).

Leaves are the cheapest smoke fuel — short burn, big plume. Useful as a deliberate signal when you want friends to spot you. Coal is the heat fuel — long burn, wide safe radius, but the smoke is the same volume.

## Phasing

| Phase | What | Files | Approx LOC |
|-------|------|-------|------------|
| 1 | Foundation spec (this doc) | new | – |
| 2 | `CAMPFIRE_SMOKE` block (id 45) — non-solid + transparent + grey + texture | `block.rs`, `texture_gen.rs` | ~30 |
| 3 | `CampfireData.smoke_ticks` + leaves-grant-smoke + tick decrement | `campfire.rs`, `save.rs` | ~50 |
| 4 | Smoke column lifecycle — spawn/clear in `game_loop.rs` tick | `game_loop.rs` | ~80 |
| 5a | Corn crop — 4 stage blocks + materials (CornSeeds, Corn) + growth | `block.rs`, `growth.rs`, `item.rs`, `texture_gen.rs` | ~120 |
| 5b | Baked Potato + Baked Carrot + Baked Corn materials + food values | `item.rs` | ~40 |
| 6 | Campfire cooking: extend `cooked_variant` + `is_raw_cookable` (3 new mappings) | `campfire.rs` | ~10 |
| 7 | `AiState::InvestigateCampfire` + nearest-lit-campfire scan | `mob_ai.rs`, `campfire.rs` | ~80 |
| 8 | Heat-radius gating + player-priority override | `mob_ai.rs` | ~60 |
| 9 | `/give` extensions + Spec 5 update + foundations README | various | ~30 |
| 10 | Axolittle playtest gate | – | – |

**Total:** ~370 LOC across ~9 build phases.

## Phase-by-phase

### Phase 2 — `CAMPFIRE_SMOKE` block

- New block id `45`. PROTOCOL_VERSION 12 → 13.
- Non-solid, transparent, light grey colour `[0.7, 0.7, 0.7]`.
- One new texture layer (grey wisp with alpha). `TEX_CAMPFIRE_SMOKE = 136`.
- `mine_drop` returns nothing — players can't pick smoke up. Realised as `ItemStack::empty()`.
- `is_solid = false`, `transparent = true`, `gravity = false`.

### Phase 3 — `CampfireData.smoke_ticks`

- Field: `smoke_ticks: u32` (defaults to 0, `#[serde(default)]` for forward compat).
- When a player adds OAK_LEAVES as fuel: also bumps `smoke_ticks += 60` (~3 s) per leaf added. (Leaves burn 1s as fuel; smoke lingers a bit longer so the signal is actually readable.)
- `tick_one`: decrement smoke_ticks by 1 if > 0, regardless of fuel state.
- Smoke is only **rendered** when `smoke_ticks > 0 AND fuel_ticks > 0` (the campfire is lit). An unfueled campfire with residual smoke_ticks shows no smoke.

### Phase 4 — Smoke column lifecycle

The tick outcome gains:

```rust
pub struct CampfireTickOutcome {
    pub block_change: Option<BlockId>,
    pub smoke_state: SmokeState,
}
pub enum SmokeState {
    Off,   // clear any existing column
    On,    // ensure column exists above
    NoChange,
}
```

`game_loop.rs` processes `smoke_state` after the campfire tick:
- **On** → place `CAMPFIRE_SMOKE` blocks at `(x, y+1..=y+6, z)` if the cell is AIR.
- **Off** → clear any `CAMPFIRE_SMOKE` blocks at `(x, y+1..=y+6, z)`.

Smoke blocks live in chunk data (same as TALL_GRASS or TORCH). They survive save/load alongside their parent campfire — when the world loads, the next campfire tick re-evaluates and either keeps or clears them.

**Cleanup on campfire-break**: extend the BUG-1 fix (yesterday's patch) to also clear smoke blocks when the campfire is destroyed. Helper `campfire::cleanup_campfire(&mut World, pos)`.

### Phase 5a — Corn as a new crop

Block ids 46-49 (after CAMPFIRE_SMOKE = 45):
- `CORN_STAGE_0` = 46 (sprout — short green stalk)
- `CORN_STAGE_1` = 47 (knee-high — taller, still green)
- `CORN_STAGE_2` = 48 (waist-high — with developing tassels)
- `CORN_STAGE_3` = 49 (mature — full stalk with yellow cob)

Materials (appended to `MaterialId` after `Flint`):
- `CornSeeds` — seed item, plants on tilled soil
- `Corn` — raw harvest (the ear, kernels still on the cob)

Growth pipeline (`growth.rs`):
- Extend `next_stage`, `is_crop`, `crop_break` with the four corn stages.
- Mature corn drops: 1 `Corn` + 1-3 `CornSeeds` (matches wheat's seed-on-harvest pattern).
- Same 200-tick-per-stage timing as the other crops.

Textures: 4 new layers (one per stage). Simple yellow-green palette progression.

### Phase 5b — Baked materials

Append to `MaterialId` (after `Corn`):
- `BakedPotato` — display "Baked Potato"
- `BakedCarrot` — display "Baked Carrot"
- `BakedCorn` — display "Corn on the Cob" (kid-friendly name)

Food values (`item.rs`):
- `Potato`: 1.0 → unchanged (raw is barely food)
- `Carrot`: 3.0 → unchanged (raw is OK)
- `Corn`: **2.0** (new — raw is moderate)
- `BakedPotato`: **5.0** (5x the raw)
- `BakedCarrot`: **4.0** (modest bump — carrot was already OK raw)
- `BakedCorn`: **5.0** (matches baked potato; corn on the cob is a proper meal)

Names + colours appended.

### Phase 6 — Extend cooking

`campfire::cooked_variant`:
- `Potato → Some(BakedPotato)`
- `Carrot → Some(BakedCarrot)`
- `Corn → Some(BakedCorn)`

`campfire::is_raw_cookable` automatically includes these via the `cooked_variant.is_some()` predicate — no extra work.

### Phase 7 — InvestigateCampfire state

New `AiState::InvestigateCampfire { target: [i32; 3], pause_ticks: u32 }`.

`mob_ai.rs` during `Idle { timer: 0 }`:
1. Existing: check for nearest player in DETECT_RANGE.
2. **New**: if no player in range, check for nearest LIT campfire in CAMPFIRE_DETECT_RANGE (12 blocks). If found, transition to `InvestigateCampfire { target, pause_ticks: 0 }`.

Helper `campfire::nearest_lit_campfire(world, mob_pos, range) -> Option<(pos, heat_radius)>`:
- Scans `world.block_entities` for entries with `is_lit()`.
- Returns the nearest within `range`.
- `heat_radius` = `((fuel_ticks as f32).log2() - 5.0).max(2.0).min(6.0)` floor:
  - 200 fuel → ~3 blocks
  - 1200 fuel → ~5 blocks
  - 4800 fuel → ~7 → clamped to 6 blocks
  - <200 fuel → 2 blocks

### Phase 8 — Heat-radius gating

In `InvestigateCampfire`:
- Compute distance to target.
- If `dist > heat_radius + 0.5`: walk toward target.
- If `dist <= heat_radius + 0.5`: stop. Hold position. Increment `pause_ticks`.
- If `pause_ticks > 200` (10 s holding): transition back to Idle (mob loses interest).
- **Hostile-mob override**: every SCAN_INTERVAL, check nearest_player. If player is closer than the campfire AND within DETECT_RANGE: transition to Chase (player priority).

The "mob can't enter heat zone" is enforced purely by their AI — they walk to the boundary and stop. No physics-level heat damage; the spec deliberately avoids damage-on-stand (matches Spec 17's "future polish" deferral).

### Phase 9 — `/give` + docs

- `/give campfire_smoke` — debug-only, mostly for testing.
- `/give baked_potato`, `/give baked_carrot`.
- Spec 5 §3.10 (campfire) — add "Smoke" subsection + the new InvestigateCampfire mob behavior.
- Spec 5 §3.11 (items added) — add BakedPotato + BakedCarrot.
- Spec 5 §6.2.1 — add the two baked food values.
- Foundations README — mark Spec 18 DELIVERED.

### Phase 10 — Axolittle playtest

Open questions for Axolittle to answer:
- Does the smoke pillar feel readable as a "find me!" signal? Is 6 blocks tall enough?
- Is the heat-radius mechanic legible — can you "see" why mobs hold at the edge?
- Big-fire vs small-fire: does the trade-off actually feel like a real choice, or is "always make a big fire" the obvious winner?
- Baked potato + baked carrot — useful additions, or do they feel like padding next to cooked meat + bread?

## Cross-game lift

- **Smoke pillar pattern** — any Decented game with a hearth could use a "smoke signal = look here" beacon. The block + tick pattern lifts cleanly.
- **Heat-radius mob behaviour** — any game with mobs and a heat source. Predator-prey games (a future Decented title?) would use exactly this.
- **Bake-vs-cook food ladder** — generic cooking-method-affects-yield ladder; useful anywhere food is a system.

Don't hardcode "AxeNStax meat" assumptions into `campfire::cooked_variant` — it's already material-based via the `MaterialId` enum, which is engine-generic.

## Memory-rule check

- ✓ axenstax has farming — extends T1 farming's payoff (baked potato is a farming product).
- ✓ shared infra strategy — smoke + heat patterns lift to other games.
- ✓ uk english naming — "Baked Potato" and "Baked Carrot" are UK English (no clash; same in US English here).

## Acceptance criteria

- [ ] Lit campfire with leaves added shows a 6-block smoke column above for ~3s per leaf.
- [ ] Unfueled campfire with residual smoke_ticks shows no smoke (rule: lit AND smoke_ticks > 0).
- [ ] Breaking the campfire clears the smoke column (no orphan smoke blocks).
- [ ] Save + reload preserves smoke_ticks alongside fuel_ticks.
- [ ] Potato + Carrot placed on a lit campfire cook in 10s, yielding BakedPotato + BakedCarrot.
- [ ] BakedPotato heals 5.0 HP; BakedCarrot heals 4.0 HP.
- [ ] A mob within 12 blocks of a lit campfire (and no player in range) walks toward it.
- [ ] The mob stops at a distance proportional to the campfire's fuel level.
- [ ] If a player comes nearer than the campfire while a hostile mob is investigating, the mob switches to Chase.
- [ ] All tests pass; check.sh ALL GREEN; bundle still under 5 MiB brotli.

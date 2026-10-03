# Farming Tier 1.5 — Processed Economy Base

**Status**: DELIVERED on main (2026-05-21). Workstation framework (`workstation.rs`, PR #50) shipped as the generic processing-block primitive; Furnace (blocks 59/60) + Mill (108) + Oven (109) + AgingRack carry their state via `BlockEntityData`. All sourcing crops shipped (Sugarcane, SugarBeet, Beetroot, Pumpkin, Berries) + animal products (Egg, MilkBucket via Bucket). ~20 new recipes wired in `crafting.rs`. `complexity_tier` + `food_value` + `trade_value` live on `Item`. Spec 6 §13 (`ServerEconomyConfig`) shipped as `server_economy.rs` (PR #45). Phase 13 (Axolittle UX playtest) remains the validation gate.
**Date**: 2026-05-14
**Branch**: `feat/farming-tier-1.5-economy` once started; off `main` (after Tier 1 lands).
**Session**: Fresh — implementer should treat this doc as the only brief.
**Prerequisite**: Tier 1 (`docs/foundations/2026-05-14-farming-system.md`) delivered — Tier 1.5 reuses tilled-soil, the growth-tick, the seed→plant→harvest loop, and the Hoe-tier ladder.
**Trigger**: Axolittle 2026-05-14 — "let's go with farming and a base for an economy, so we want to be able to grow more different items and bake/cook/process them to make higher value items." This spec banks that expansion while the design intent is fresh; it sits between Tier 1 (basic farming) and Tier 2 (plough + draft animals) in the foundations queue.

---

## TL;DR

Tier 1 gave you a tilled-soil + 3-crop + bread loop. Tier 1.5 turns that into an **economy**: more inputs, processing stations, multi-step recipe chains, and a value ladder that runs from raw crops (~1 hunger) up to peak-crafted dishes (~15-20 hunger + saturation bonus). It also delivers the **Furnace** that Spec 5 §4.5 has been designing-without-building since Wave 6, and lays in a **trade-value hook** on items that adds a new Spec 6 §13 ("Server Economy Modes / Item Trade-Value") — the stale parenthetical in Spec 6 §3 table-row line 297 that points at a non-existent §6 "server economy modes" gets retargeted to §13 as part of Phase 12.

**Three pillars**:

1. **Sourcing expansion** — 5 new crops (sugarcane, sugar beet, beetroot, pumpkin, berries) + 2 animal products (eggs from chickens, milk from cows via a new Bucket item). **Note:** sugar beet (pale, sugar-extraction cultivar) and beetroot (dark-red salad/cooking root) are deliberately separate items — UK English distinguishes them and they have distinct in-game roles (sugar beet → Mill → Sugar; beetroot → Beetroot Soup, not a sugar source).
2. **Processing workstations** — 4 block-entity stations on a shared workstation framework: **Furnace** (canonical smelting, replaces the Wave-6 grid hack), **Mill** (grinds: wheat→flour, sugarcane→sugar), **Oven** (multi-input baked goods: bread variants, cake, pie), **Aging Rack** (real-time recipes: butter, cheese — the "set and come back later" depth mechanic).
3. **Recipe ladder + economy hook** — ~20 new food and intermediate items, each tagged with a `complexity_tier: u8` and `food_value` scaling. Adds `trade_value: Option<u64>` to `Item` metadata, hand-tuned per recipe, server-operator overridable, ready for Spec 6's Bitcoin layer to consume on opt-in servers.

Total scope: **~2400 lines** (spec → workstation framework → 4 stations → 6 new sourced items → 15-20 recipes → trade-value plumbing → Spec 5/6 updates). 12 autonomous phases + 1 playtest gate.

---

## Why this lives between Tier 1 and Tier 2

- **Tier 1** is a complete shippable farming loop on its own (hoe → till → plant → grow → harvest → bread). Tier 1.5 is **depth on top of breadth**, not a missing piece. Tier 1 must ship and get a playtest first.
- **Tier 2** is automation (plough + draft animals + breeding + fences). Tier 1.5 is **handcrafted depth** — every recipe is a player action, every workstation is a player-operated block. Tier 1.5 makes Tier 2's automation *valuable* by establishing what's being automated.
- **Tier 1.5 is independent of Tier 2.** Different files, different mechanics. They can be parallelised after Tier 1 lands, though serialising them is cleaner for Axolittle's playtest sequencing.
- **Cross-game lift, per shared infra strategy**:
  - The **workstation framework** (block-entity with input/fuel/output slots + tick-based processing + UI hook) lifts directly to any game that wants stations of any kind — cooking, egg incubation, artifact-restoration benches in other games on the same primitives.
  - The **trade-value hook** is generic item-economy plumbing. Same field works for any inventory in any game.
  - The **multi-step recipe chain** + **complexity-tier** pattern is generic crafting design — any game with a recipe ladder benefits.
  - The **animal-product extraction** (laying eggs, milking) is generic mob-behaviour-extension and lifts to any game with farm animals.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/block.rs` — block ID registry. Tier 1.5 adds **~27 new IDs total** across phases: Phase 2/3/5/8/9 each add 1 workstation block (Furnace, Mill, Oven, Aging Rack = 4), Phase 4 adds 1 (Sugarcane), Phase 10 adds 18 (4 sugar-beet stages, 4 beetroot stages, 1 pumpkin, 4 stem stages + 1 mature-stem, 4 berry-bush stages). Picks up after Tier 1's `WHEAT_STAGE_3 = ?` (TBD — depends on Tier 1's exact ID assignments).
- `game/engine/src/item.rs` — `MaterialId` enum. Adds: `Bucket`, `MilkBucket`, `Egg`, `Sugar`, `Flour`, `Dough`, `Cream`, `Butter`, `Cheese`, `SweetBread`, `Cake`, `PumpkinPie`, `BerryPie`, `Cookie`, `Pancakes`, `BakedPotato`, `LoadedBakedPotato`, `Stew`, `BeetrootSoup`, `Bowl`, `Pumpkin` (food version, separate from block), `SugarBeet`, `SugarBeetSeeds`, `Beetroot`, `BeetrootSeeds`, `Berries`. (~25 new variants, plus a few related; appended to preserve bincode order.)
- `game/engine/src/crafting.rs` — `ToolType` no change. New module/file for **workstation recipes** since they aren't grid-shaped: `workstation_recipes.rs`. Existing smelting recipes (`raw_beef_over_coal → cooked_beef` and friends) migrate from `crafting.rs::match_recipe` to the new furnace recipe table; the grid versions get a deprecation comment + retained for migration window (one full release before deletion).
- `game/engine/src/block_interact.rs` — right-click on a workstation block opens that station's UI (new entry-point). Right-click on a cow with a Bucket in hand → empty bucket → MilkBucket. New chicken behaviour: time-driven egg drop at the chicken's feet (no player interaction needed).
- New module **`game/engine/src/workstation.rs`** — generic block-entity pattern: input slot(s), fuel slot (optional), output slot, current-recipe-in-progress tracker, tick-driven progress. Furnace/Mill/Oven/Aging Rack all instantiate this with different recipe tables + tick rates.
- New module **`game/engine/src/workstation_ui.rs`** — egui panel that renders the station's slots + progress bar. One panel template, parameterised per station type.
- `game/engine/src/mob.rs` — chickens get a `next_egg_tick: u64` field; cows get a `last_milked_tick: u64` cooldown field. Drop tables unchanged (these are alive-extraction mechanics).
- `game/engine/src/save.rs` — workstation block-entities round-trip via the existing block-entity save path (which Tier 1 doesn't add either; this is the first true block-entity in the engine). Save format gains a `BlockEntities` map keyed by `BlockPos`. Bumps `SAVE_VERSION`.
- `game/engine/src/protocol.rs` — bumps `PROTOCOL_VERSION` (after Tier 1's bump): adds workstation UI open/close packets + workstation slot sync packets. Versioning entry per phase.
- `game/engine/src/texture_gen.rs` — heavy additions: 4 workstation block textures (multi-face), 4 crop families' growth-stage textures, ~20 new item textures.
- `game/engine/src/commands/builtins/give.rs` — `/give` extensions for every new material + block.

### Related specs

- **Spec 5 §4.5 (Furnace / Smelting)** — fully describes the furnace block-entity: 3 slots (input, fuel, output), `smelt_time` per recipe, `Furnace`/`BlastFurnace`/`Smoker` variants. Tier 1.5 **delivers §4.5 as written**. The "smelt-in-grid via coal-adjacency" path in current `crafting.rs` was always temporary (`// Wave 6 — Furnace + UI is a future wave`). Tier 1.5 ends that wave.
- **Spec 5 §3.7 (Container Inventories)** — describes the contract for non-player inventories (chests, furnaces, hoppers) as ECS components on block entities. Workstation framework implements this for the first time. Pattern then lifts trivially to chests (future spec).
- **Spec 5 §6.2 (Hunger / Food values)** — existing table has raw + cooked meats + bread. Tier 1.5 extends this table substantially (Phase 12). Recommend a **complexity-tier column** that documents the value ladder logic per item.
- **Spec 6 §2.2 (Proof of Play)** — orthogonal. Cooking has no proof-of-play hook in Tier 1.5 (no hashing of furnace burns; not a mining action). Stays orthogonal in future tiers too — proof-of-play is the *Bitcoin sourcing* mechanic; trade-value is the *Bitcoin spending* mechanic.
- **Spec 6 §13 (NEW — Server Economy Modes / Item Trade-Value)** — Tier 1.5 introduces this new section. `Item::trade_value()` returns `Option<u64>`. Server economy config exposes a per-item override map + a global "sats per trade-unit" conversion. **Note**: the existing stale parenthetical at Spec 6 line 297 ("see Spec 6 §6 (server economy modes, TBD)") points at the wrong section — §6 is "Player Payment Flows", a fully written section. Phase 12 creates §13 and retargets line 297 to it.
- **Spec 6 §10.3 (Parent-controlled Bitcoin)** — preserved. Trade-value is an internal economy unit. Sats conversion is the parent-controlled gate. Default servers run trade-value as internal-score-only.
- **`docs/foundations/2026-05-14-farming-system.md`** (Tier 1) — this spec layers on top of Tier 1. Tier 1 ships first.

### Memory pointers

- axenstax has farming — base design Tier 1 captures. This spec extends that by Axolittle's 2026-05-14 economy ask; that memory will be updated to record the extension.
- shared infra strategy — workstation framework + trade-value hook + recipe-chain pattern are explicitly cross-game-friendly. Keep AxeNStax flavour (specific recipes, food names) out of the framework code; live in data tables.
- bitcoin parent controlled — trade-value lives at the item-metadata layer (always present). Sats conversion lives at the server-economy layer (parent-controlled, off by default).
- pretest check — before starting each phase, verify the code-state claims in this doc against actual current code; values like "next free block ID" will have moved since this was written.
- proof of play is proof of work — Proof of Play is a mining-side concept; Tier 1.5 (cooking/baking/processing) intentionally does not invoke it. Don't add hash-on-bake or similar.

### What does NOT exist yet (and Tier 1.5 needs to add or defer)

- **Block entities.** The engine has no first-class block-entity system today — chests aren't real, furnaces aren't real. Tier 1.5 introduces the block-entity pattern (Phase 2). This is load-bearing for the whole expansion. **Spec 5 §3.7 + §4.5 design it; Tier 1.5 builds it.**
- **Workstation UI panels.** Inventory has egui UI today (`inventory_ui.rs`). Workstation panels reuse the egui infrastructure (Phase 2 wires the first one).
- **Time-of-day-or-wall-clock-driven processes.** Tier 1 has tick-driven growth. The Aging Rack needs *long* timescales (real cheese takes weeks). Two implementation options: scale game-time aggressively (4× already in single-player; could be 240× for an Aging Rack), or use in-game-day counts (cheese ages over 3 in-game days regardless of speed). **Recommended: in-game-day counts.** Decouples from world-time-speed tuning.
- **Fluid items in inventory.** Bucket and MilkBucket are the first inventory-bound fluids. Bucket stacks to 16 (Minecraft baseline); MilkBucket stacks to 1 (consume-and-empty). Pattern: a `Bucket` material with a `contents: Option<BucketContents>` discriminant. Or two separate variants (`Bucket`, `MilkBucket`). **Recommended: separate variants** for save-compat simplicity; collapse into discriminated variant later if water/lava buckets ever land.
- **Bowl as a container item.** Stew recipes need a bowl. Wooden Bowl crafted from 3 planks in a V pattern (Minecraft baseline). Adds 1 material + 1 recipe.
- **Recipe-discovery UI.** Spec 5 §4.6 describes a recipe-book UI (placeholder). Tier 1.5 doesn't deliver §4.6 — recipe discovery happens via `/help recipes` console command (extending the engine commands system) + the workstation UI auto-suggesting valid recipes from current inputs. Full recipe-book UI is a future polish wave.
- **Recipe categories / filters in creative inventory.** Spec 5 mentions Food/Tools/etc. tabs (§3 search refs). Tier 1.5 can skip (creative inventory is one-window today; categorisation is a UI polish wave).

---

## Scope

| # | Phase | Files | Est. lines | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md` | ~900 | ✓ |
| 2 | **Workstation framework + block-entity pattern + save round-trip** | new `workstation.rs`, new `workstation_ui.rs`, `save.rs` extension (BlockEntities map), `block.rs` (block-entity flag), `protocol.rs` (PROTOCOL_VERSION bump + UI open/close packets), tests | ~400 | ✓ |
| 3 | **Furnace** as canonical workstation — deliver Spec 5 §4.5 | `block.rs` (FURNACE block), workstation_recipes.rs (smelting recipes migrate here from crafting.rs), `block_interact.rs` (right-click opens UI), `texture_gen.rs` (furnace block + lit-front overlay), Spec 5 §4.5 status flip | ~250 | ✓ |
| 4 | **Sugarcane** crop (water-adjacent growth, multi-block-tall like Minecraft), `Sugar` material | `block.rs` (SUGARCANE block, 3-stage vertical growth), `growth.rs` (Tier 1's growth tick gets a vertical-stack case), `item.rs` (Sugar), `texture_gen.rs`, tests | ~200 | ✓ |
| 5 | **Mill** workstation + grinding recipes (wheat → flour, sugarcane → sugar, sugar beet → sugar at lower yield) | `block.rs` (MILL), `workstation_recipes.rs` (mill recipe table), `item.rs` (Flour), Spec 5 §3.x note, texture_gen.rs, tests | ~250 | ✓ |
| 6 | **Bucket** item + **cow milking** (right-click cow with bucket → milk bucket) | `item.rs` (Bucket, MilkBucket — both as `MaterialId` variants), `block_interact.rs` (cow interaction path, cooldown via `last_milked_tick` on Cow mob), `mob.rs` (Cow gains cooldown field), `crafting.rs` (Bucket recipe — 3 iron in V pattern), tests | ~250 | ✓ |
| 7 | **Chicken laying** — chickens passively drop eggs on a cooldown; **Egg** material | `mob.rs` (Chicken gains `next_egg_tick`, AI tick checks and drops egg item at feet), `item.rs` (Egg), `entity.rs` (item entity for ground-spawned items if not already there — Tier 1 covers harvest drops, this is similar), tests | ~180 | ✓ |
| 8 | **Aging Rack** + slow-time recipes (cream → butter, milk → cheese via cream-then-aging chain) | `block.rs` (AGING_RACK), `workstation.rs` (in-game-day timer support added to framework), `workstation_recipes.rs` (aging table), `item.rs` (Cream, Butter, Cheese), tests | ~300 | ✓ |
| 9 | **Oven** + advanced baked recipes (Dough from flour+water+egg in grid, Bread keeps the Tier 1 grid recipe but a richer Sweet Bread + Cake via Oven, Cookies, Pancakes) | `block.rs` (OVEN), `workstation_recipes.rs` (oven recipes), `crafting.rs` (Dough is a grid recipe), `item.rs` (Dough, SweetBread, Cake, Cookie, Pancakes), tests | ~350 | ✓ |
| 10 | **Sugar Beet + Beetroot + Pumpkin + Berries** — four more sources. Sugar beet is a wheat-clone whose harvest mills to Sugar (lower yield than sugarcane). Beetroot is a separate wheat-clone whose harvest is the salad/cooking root — eaten raw (low) or made into Beetroot Soup via Furnace. Pumpkin grows on a stem like Minecraft. Berries are a replenishing bush (right-click harvests without breaking the plant). Pumpkin Pie + Berry Pie via Oven. Stew (cooking pot omitted — Stew is a Furnace recipe via Bowl + ingredients to keep the workstation count manageable) | `block.rs` (SUGAR_BEET_STAGE_0..3, BEETROOT_STAGE_0..3, PUMPKIN, PUMPKIN_STEM_*, BERRY_BUSH_*), `growth.rs` (pumpkin-stem-produces-pumpkin logic; berry-bush-refills logic), `item.rs` (SugarBeet, SugarBeetSeeds, Beetroot, BeetrootSeeds, Pumpkin (food), Berries, Bowl, Stew, BeetrootSoup, PumpkinPie, BerryPie, BakedPotato, LoadedBakedPotato), `texture_gen.rs`, tests | ~500 | ✓ |
| 11 | **`Item::complexity_tier()` + `Item::trade_value()`** — both data-driven from a single per-item annotation table. Defaults sensibly. Hand-tune per-item per the value ladder | `item.rs` (annotation table + accessor methods), tests asserting the value-ladder invariants | ~120 | ✓ |
| 12 | **Spec 5 + Spec 6 updates** — (a) Spec 5: §4.5 marked DELIVERED (Furnace); **also renumber the duplicate §4.5 "Slash Commands" at Spec 5 line 1267 onwards** — that section collides with the Furnace one and needs to move to §4.7+. §6.2 food table expanded with new entries + complexity-tier column. New §3.x "Farming Tier 1.5" subsection. (b) Spec 6: create new §13 "Server Economy Modes / Item Trade-Value" describing trade-value → sats conversion semantics on Bitcoin-enabled servers (with parent-control gate per §10.3); retarget the stale parenthetical at line 297 from "§6" to "§13". | `docs/spec/05-gameplay-systems.md`, `docs/spec/06-bitcoin-integration.md` | ~200 | ✓ |
| 13 | **Axolittle playtest** — value-ladder feel; station ergonomics; recipe discovery; trade-value tuning | n/a | 0 | ✗ blocked |

**Total**: ~2400 lines spec + code + tests for Tier 1.5. Phase 13 is the playtest gate.

**Recommended order**: 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12. Phases 6 and 7 are parallel-safe (different files). Phase 11 cleanly slots after 10 since all the items it annotates exist by then.

---

## Phase 1 — This spec

You're reading it. ✓ Move on.

---

## Phase 2 — Workstation framework + block-entity pattern

### Goal

Deliver the **first real block-entity** in the engine. Workstation framework supports: input slot(s), optional fuel slot, output slot(s), tick-driven or day-driven progress, recipe matching, persistence. Furnace/Mill/Oven/Aging Rack all instantiate this.

### Design

A workstation is identified by its block ID. When a player right-clicks a workstation block, the server (or local sim) looks up the matching block-entity record by `BlockPos`. If absent, creates a default one with empty slots. The block-entity tracks:

```rust
pub struct Workstation {
    pub kind: WorkstationKind,           // Furnace | Mill | Oven | AgingRack
    pub input:  [Option<ItemStack>; N],  // N varies per kind: Furnace=1, Mill=1, Oven=3, AgingRack=1
    pub fuel:   Option<ItemStack>,       // None for Mill + AgingRack (no fuel needed)
    pub output: Option<ItemStack>,
    pub progress: u32,                   // ticks elapsed on current recipe (or in-game-days × 24000 for AgingRack)
    pub current_recipe: Option<RecipeId>,
}
```

`WorkstationKind` encodes slot counts + tick model (`Ticks(u32)` or `InGameDays(u32)`). Each kind looks up recipes in a per-kind `&'static RECIPE_TABLE`.

Block entities are stored in `World` as a `HashMap<BlockPos, BlockEntity>` (a new field). Save round-trips them via a new `BlockEntities` section in the chunk-save format.

### Changes

- New module `game/engine/src/workstation.rs`:
  - `Workstation` struct + `WorkstationKind` enum (only `Furnace` populated in this phase; Mill/Oven/AgingRack land in their respective phases as no-op `kind` variants).
  - `Workstation::tick(&mut self, dt_ticks: u32, registry: &WorkstationRecipes)` — advances progress, completes recipes, emits a `BlockChange`-equivalent for output-slot updates.
  - Recipe matching is per-kind, table-driven.
- New module `game/engine/src/workstation_ui.rs`:
  - egui panel: input slot(s), fuel slot (if present), output slot, progress bar, recipe-hint text.
  - One template; per-kind parameters (slot count, fuel slot visible/hidden, "Burning" vs "Grinding" vs "Baking" vs "Aging" label).
- `game/engine/src/world.rs` (or wherever the world struct lives):
  - Add `block_entities: HashMap<BlockPos, BlockEntity>` field.
  - `World::get_block_entity_mut(pos) -> Option<&mut BlockEntity>` accessor.
- `game/engine/src/save.rs`:
  - Save format gains a `block_entities` section per chunk. Bincode-versioned. Bumps `SAVE_VERSION` (current TBD — check before bumping).
  - Load path: missing block_entities section → empty map (backward-compatible).
- `game/engine/src/protocol.rs`:
  - New packets: `OpenWorkstationPacket { pos }`, `CloseWorkstationPacket`, `WorkstationStateUpdate { pos, state }`. Bump PROTOCOL_VERSION.
- `game/engine/src/block_interact.rs`:
  - Right-click handler dispatches to `try_open_workstation(player, world, pos)` when the targeted block is a workstation block. Sends `OpenWorkstationPacket`.

### Tests

- `workstation::tests::furnace_smelts_raw_beef_in_200_ticks` — set up a furnace block-entity, put RawBeef in input + Coal in fuel, tick 200 times, expect CookedBeef in output + 1 coal consumed.
- `workstation::tests::no_fuel_no_progress` — same setup minus fuel → progress stays at 0.
- `workstation::tests::output_blocks_when_full` — output slot full of CookedBeef (stack of 64), trying to smelt another → progress stalls.
- `workstation::tests::wrong_input_no_match` — Stone in input → no recipe matches → idle.
- `save::tests::workstation_round_trips` — save a world with a half-progress furnace, load it, expect identical state.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Block-entity pattern is generic enough that Mill/Oven/Aging Rack in later phases need only a recipe table + `WorkstationKind` variant; no framework code touched.

### Save compat

`SAVE_VERSION` bump. Old saves load with empty `block_entities` map (no furnaces yet anyway in old saves). Bincode-additive on chunk format.

---

## Phase 3 — Furnace (canonical workstation)

### Goal

Real Furnace block + UI + recipe table, replacing the Wave-6 "smelt-in-grid via coal adjacency" hack. Delivers Spec 5 §4.5.

### Changes

- `game/engine/src/block.rs`:
  - Add `FURNACE: BlockId` (next free after Tier 1's last block — verify before assigning).
  - Block-entity flag set in `BlockDef`.
  - Textures: front (lit/unlit two variants), top, sides.
- `game/engine/src/workstation_recipes.rs` (new):
  - `FURNACE_RECIPES` table: RawBeef→CookedBeef, RawPorkchop→CookedPorkchop, RawChicken→CookedChicken, RawMutton→CookedMutton, RawIron→IronIngot, Sand→Glass (existing Wave-16 grid recipe migrates here), Potato→BakedPotato, Bowl+vegetables→Stew (vegetables defined as: any of Carrot/Potato/Beetroot/Pumpkin, ≥1 with Bowl).
  - Default smelt time: 200 ticks (10 seconds @ 20 TPS), matching Minecraft.
  - Default fuel: Coal (8 smelts per coal). Future fuels: charcoal, lava bucket, planks (lower-yield).
- `game/engine/src/crafting.rs`:
  - Existing smelting recipes (raw_beef_over_coal pattern in `match_recipe`) get a `#[deprecated(note = "Use Furnace as of Wave 26 Tier 1.5; grid-smelt retained for one release for migration")]` markers. Code path stays for backward-compat but the workstation furnace becomes the canonical route.
  - Add Furnace recipe (8 cobblestone in a ring around an empty centre, Minecraft pattern) → 1 Furnace block.
- `game/engine/src/block_interact.rs`:
  - Right-click on a Furnace block opens the Furnace UI (handled by the workstation framework from Phase 2).
- `game/engine/src/texture_gen.rs`:
  - Furnace textures: 4 layers (front_unlit, front_lit, top, side). Cobblestone-coloured base + black furnace mouth + orange glow on lit-front.
- `docs/spec/05-gameplay-systems.md` §4.5:
  - Status note added at top: "DELIVERED Wave 26 / Tier 1.5 / commit `<TBD>`."
  - Any drift discovered during implementation propagated back into §4.5.

### Tests

- `crafting::tests::furnace_recipe_matches` — 8 cobblestone ring → 1 Furnace block.
- `workstation::tests::furnace_baked_potato` — Potato in input + Coal in fuel → BakedPotato after 200 ticks.
- `workstation::tests::furnace_stew_recipe` — Bowl + Carrot + Potato in input → Stew after 200 ticks.
- `block_interact::tests::right_click_furnace_opens_ui` — right-click sends OpenWorkstationPacket.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Manual: craft a Furnace, place it, right-click → UI opens with 3 slots + progress bar. Drop RawBeef in input + Coal in fuel → smelting visibly progresses → CookedBeef in output.
- Spec 5 §4.5 status reflects delivery.

### Save compat

New block ID. Save format change covered by Phase 2's block-entities section.

---

## Phase 4 — Sugarcane

### Goal

A new crop type with vertical-stack growth (1-3 blocks tall, top is the only one that grows), water adjacency required. Right-click harvest like Minecraft (preserves the bottom block, harvests anything above).

### Design

Sugarcane is a 3-stage *vertical* crop — different from Tier 1's wheat (4-stage in place). Cell grows up to 3 blocks tall over time. Player breaks the bottom to drop everything; player can also right-click the top to harvest only the top and let it regrow.

### Changes

- `game/engine/src/block.rs`:
  - `SUGARCANE: BlockId` — single block ID; the stack height is implicit from how many vertically adjacent SUGARCANE blocks there are.
  - `BlockDef`: transparent (you can walk through), no collision.
  - Drops: 1 Sugarcane material per block on break.
- `game/engine/src/item.rs`:
  - Add `Sugarcane` material (the harvested item).
- `game/engine/src/growth.rs`:
  - Extend the growth tick to handle vertical-stack crops. On each tick window: if a SUGARCANE block has air above + water within 4 horizontal blocks + the column is shorter than 3 blocks → place another SUGARCANE block above.
- `game/engine/src/block_interact.rs`:
  - Plant action: right-click dirt (NOT tilled soil; sugarcane plants on plain dirt adjacent to water in Minecraft) with a Sugarcane material → place SUGARCANE on top of the dirt. Requires water within 4 blocks horizontally; refuse otherwise with a chat-warning ("Sugarcane needs water nearby").
  - Right-click the top of a sugarcane stack with empty hand → harvest top block (drops 1 Sugarcane), preserving lower stack.
- `game/engine/src/texture_gen.rs`:
  - Sugarcane texture (green vertical stalk with subtle horizontal banding).

### Tests

- `block_interact::tests::sugarcane_plants_on_dirt_near_water` — dirt with water 2 blocks away + sugarcane in hand → places sugarcane.
- `block_interact::tests::sugarcane_refuses_without_water` — no water within 4 → no plant, chat warning fired.
- `growth::tests::sugarcane_grows_up_to_3_blocks` — single block, tick many times → grows to 3, then stops.
- `growth::tests::sugarcane_refuses_to_grow_when_water_removed` — break the adjacent water → growth stops.
- `block_interact::tests::right_click_top_sugarcane_harvests_one` — 3-tall stack + right-click top → drops 1, leaves 2-tall stack.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Manual: place water adjacent to dirt, plant sugarcane, wait, watch it grow vertically, harvest top.

### Save compat

New block ID. No format change beyond Phase 2.

---

## Phase 5 — Mill + grinding recipes

### Goal

Second workstation. Grinds wheat → flour, sugarcane → sugar + bagasse (two-stream output), sugar beet → sugar (lower yield, since Phase 10 adds sugar beet), bagasse → pulp paper. No fuel needed (hand-cranked; visible animation in UI as a small turning gear). **UK English note:** sugar beet (pale, sugar-extraction cultivar) is the sugar-source crop; beetroot (the dark-red salad root, also added in Phase 10) is a separate food crop and is **not** milled here.

**Pulp Paper** is the T1.5 paper grade — used as the high-yield Plan Tile input for Spec 24 Build Schematics. Sugarcane milling produces sugar AND bagasse (the fibrous residue) as separate output stacks; bagasse mills separately into Pulp Paper. This mirrors the real-world sugarcane-bagasse → paper industry and ties Spec 24's plan-paper-cost curve into the T1.5 economy (see `docs/vision/build-schematics-long-run.md` §3.4 + Spec 24 Phase 3).

### Changes

- `game/engine/src/block.rs`:
  - `MILL: BlockId`. Block-entity flag set.
  - Textures: wooden frame + millstone face + grain-chute on one side.
- `game/engine/src/workstation.rs`:
  - `WorkstationKind::Mill` variant. 1 input slot, no fuel slot, 1 output slot. Tick-based.
- `game/engine/src/workstation_recipes.rs`:
  - `MILL_RECIPES` table:
    - Wheat → Flour (1:1, 100 ticks = 5s).
    - **Sugarcane → Sugar + Bagasse (1:1:1, 100 ticks)** — two-stream output. Requires extending the Mill workstation to support multi-output recipes (or a parallel `secondary_output` slot). The Sugar goes into the primary output slot; Bagasse stacks into a secondary output slot or auto-ejects if the secondary is full.
    - SugarBeet → Sugar (3:1, 150 ticks — sugar-from-beet is less efficient than sugar-from-cane; gives sugar beet purpose as a backup sugar source for biomes/areas where sugarcane is hard to get).
    - **Bagasse → PulpPaper (2:1, 150 ticks)** — paper-press recipe. Two bagasse units press into one Pulp Paper sheet. Consumed by Spec 24 Plan Tile recipe `1 stick + 1 PulpPaper → 25 PLAN_TILEs`.
- `game/engine/src/crafting.rs`:
  - Mill recipe: 2 cobblestone bottom row + 2 sticks middle column + 1 plank centre. Output: 1 Mill block.
- `game/engine/src/item.rs`:
  - `Flour`, `Sugar`, **`Bagasse`**, **`PulpPaper`** material variants. PulpPaper joins the `is_paperish_slot` predicate alongside PapyrusSheet (Spec 23).
- `game/engine/src/texture_gen.rs`:
  - Mill block textures + Flour item texture (pale beige) + Sugar item texture (white granular) + **Bagasse texture (light brown fibrous wad)** + **PulpPaper texture (cleaner cream than papyrus, even fibre)**.

### Tests

- `crafting::tests::mill_recipe_matches` — 5-block recipe → Mill.
- `workstation::tests::mill_grinds_wheat_to_flour` — Wheat in input, 100 ticks, Flour in output.
- `workstation::tests::mill_grinds_sugarcane_to_sugar_and_bagasse` — single Sugarcane input, 100 ticks, both Sugar (primary) AND Bagasse (secondary) outputs populated.
- `workstation::tests::mill_grinds_sugar_beet_to_sugar_at_3_to_1` — assert input consumed in 3s, output is 1 Sugar (Phase 10 makes SugarBeet input real; gated by Phase 10 delivery but the recipe-table entry lands here).
- `workstation::tests::mill_grinds_bagasse_to_pulp_paper_at_2_to_1` — 2 Bagasse in, 150 ticks, 1 PulpPaper out.
- `workstation::tests::mill_does_not_grind_beetroot` — Beetroot in input → no recipe matches → idle. Guards against the sugar-beet/beetroot conflation.
- `workstation::tests::mill_no_fuel_slot` — UI shows 2 slots not 3.
- `crafting::tests::pulp_paper_unlocks_plan_tile_25_recipe` — Spec 24 follow-on: with PulpPaper in inventory + stick, crafting yields 25 PLAN_TILEs (vs papyrus arm's 9).

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Manual: craft Mill, place, mill wheat to flour, mill sugarcane to sugar.

### Save compat

New block ID + new material variants. No format change.

---

## Phase 6 — Bucket + cow milking

### Goal

Iron Bucket craftable. Right-click a cow with empty bucket → empty bucket consumed, MilkBucket added. Cow has a milking cooldown (~5 in-game minutes — 6000 ticks at 20 TPS, or ~1 in-game day at world_time_step=4) to prevent infinite milk farms.

### Changes

- `game/engine/src/item.rs`:
  - `Bucket` material variant. Stacks to 16 (Minecraft baseline).
  - `MilkBucket` material variant. Stacks to 1 (since it has "contents").
- `game/engine/src/crafting.rs`:
  - Bucket recipe: 3 IronIngot in V pattern (Minecraft). Output: 1 Bucket.
- `game/engine/src/mob.rs`:
  - `Cow` struct adds `last_milked_tick: u64` (default 0).
- `game/engine/src/block_interact.rs`:
  - Right-click on a cow with Bucket in hand: if `current_tick - cow.last_milked_tick >= MILKING_COOLDOWN_TICKS` (set to 6000), consume bucket, give MilkBucket, set `cow.last_milked_tick = current_tick`. Otherwise show "Cow needs time to refill" message.
  - **Drinking MilkBucket**: hold right-click → eat animation → restore 6 hunger + clears all status effects (Minecraft parity). Empty Bucket returned to inventory.
- `game/engine/src/texture_gen.rs`:
  - Bucket item texture (grey cylindrical metal) + MilkBucket variant (white-topped).

### Tests

- `crafting::tests::bucket_recipe_matches` — 3 iron V → Bucket.
- `block_interact::tests::cow_milking_works_first_time` — cow with `last_milked_tick=0` + bucket → MilkBucket in inventory.
- `block_interact::tests::cow_milking_cooldown` — milking right after another milking → no MilkBucket, warning shown.
- `block_interact::tests::milk_bucket_drink_restores_hunger_and_returns_empty_bucket`.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Manual: craft Bucket, find cow, right-click → MilkBucket. Drink it. Try to milk same cow immediately → blocked.

### Save compat

2 new material variants appended.

---

## Phase 7 — Chicken laying + Egg

### Goal

Chickens passively drop an egg item at their feet on a cooldown (~5-10 minutes IRL, randomised; matches Minecraft). Player picks up the egg like any other item drop.

### Changes

- `game/engine/src/item.rs`:
  - `Egg` material variant.
- `game/engine/src/mob.rs`:
  - `Chicken` struct adds `next_egg_tick: u64` field. On chicken spawn, initialised to `current_tick + rand_range(6000..12000)` (~5-10 minutes at 20 TPS).
  - Chicken AI tick: if `current_tick >= next_egg_tick`, spawn an item entity (Egg) at chicken's position; reset `next_egg_tick = current_tick + rand_range(6000..12000)`.
- `game/engine/src/entity.rs`:
  - Item entities — if Tier 1's harvest drops already use the item-entity path, no change. If not, this phase adds it (lightweight: position + ItemStack + pickup-on-player-proximity).
- `game/engine/src/texture_gen.rs`:
  - Egg item texture (white ovoid with subtle speckling).

### Tests

- `mob::tests::chicken_lays_egg_after_cooldown` — chicken with `next_egg_tick = current_tick + 1`, tick once, expect an Egg item entity spawned at chicken position.
- `mob::tests::chicken_does_not_lay_before_cooldown`.
- `mob::tests::egg_cooldown_resets_after_lay`.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Manual: spawn chicken, wait ~5 minutes, pick up egg.

### Save compat

1 new material variant. Chicken struct field appended; existing saves load with `next_egg_tick = 0` (chicken lays on first tick after load — minor harmless edge).

---

## Phase 8 — Aging Rack + cheese/butter chain

### Goal

Third workstation: in-game-day-timer-driven, no fuel, single input + single output. All milk→dairy processing runs through this single station — no detour via the Mill (the Mill is for grain/sugar grinding, not dairy). Three recipes:

- **MilkBucket → Cream + empty Bucket** (½ in-game day)
- **Cream → Butter** (1 in-game day)
- **Cream → Cheese** (3 in-game days)

Recipe selection via a radio button in the UI: when Cream is in the input slot, the player picks "Aging for: Butter / Cheese". This avoids needing a second input ingredient (Salt is deferred; rennet substitution is deferred) and keeps the framework one-in-one-out.

### Changes

- `game/engine/src/block.rs`:
  - `AGING_RACK: BlockId`. Block-entity flag set.
  - Textures: wooden frame + storage slots + a brass tag.
- `game/engine/src/workstation.rs`:
  - `WorkstationKind::AgingRack` variant. Adds `InGameDays(u32)` to the `ProcessTime` enum.
  - Tick logic: AgingRack progress is derived from the existing `world_time: u64` field on `GameState` (a tick counter; one in-game day = 24000 ticks at the standard 20 TPS / `world_time_step: 1` configuration; with the alpha-default `world_time_step: 4`, a day is 6000 ticks of real time). On recipe-start, store `start_day = world_time / 24000` on the Workstation; each tick, check `(world_time / 24000) - start_day >= required_days`. **No new field needed** — derived from `world_time`. Visible UI bar shows "Day 1 of 3" instead of progress bar.
- `game/engine/src/workstation_recipes.rs`:
  - `AGING_RACK_RECIPES` table:
    - MilkBucket → Cream + Bucket (½ in-game day)
    - Cream → Butter (1 in-game day) — selected via radio button when Cream is in input
    - Cream → Cheese (3 in-game days) — selected via radio button when Cream is in input
  - Butter is the faster-cheaper output; Cheese is the patient-aged high-value output. Same input, player picks the timer/output via the UI selector.
- `game/engine/src/crafting.rs`:
  - Aging Rack recipe: 4 planks corners + 4 sticks edges + 1 iron centre (Minecraft-rack-like; not in Minecraft directly, AxeNStax-native).
- `game/engine/src/item.rs`:
  - `Cream`, `Butter`, `Cheese` material variants. **Note**: `Cheese` here is the basic Aging-Rack cow-milk cheese item (3 in-game days). Named premium cheeses (`Cheddar`, `Brie`, `Parmesan`, `Feta`, etc.) are distinct `MaterialId` variants added at T7 when the Cheese Cave workstation lands — do not collapse those into `Cheese` now; they're separate products with different aging times and trade values per the vision §6.3 dairy ladder.
- `game/engine/src/texture_gen.rs`:
  - Aging Rack block + Cream/Butter/Cheese item textures.

### Tests

- `workstation::tests::aging_rack_cream_to_butter_one_day`.
- `workstation::tests::aging_rack_cream_to_cheese_three_days`.
- `workstation::tests::aging_rack_uses_day_counter_not_ticks` — fast-forward day counter via `/time` and assert progress.
- `workstation::tests::aging_rack_recipe_selector_works`.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Manual: get MilkBucket → put in Aging Rack → wait ½ day → Cream + empty Bucket. Re-insert Cream → pick "Cheese" → wait 3 days (`/time set day` 3x to fast-forward) → Cheese.

### Save compat

New block ID + 3 new materials. In-game-day fields on workstation already handled by Phase 2.

---

## Phase 9 — Oven + advanced baked recipes

### Goal

Fourth workstation: multi-input baker. 3-slot input grid (small recipe area inside the station), fuel slot, output slot. Recipes: Dough (from grid, NOT oven — clarified below), Bread variants from Oven, Cake from Oven, Cookies, Pancakes.

### Design clarification: Dough

Dough is a grid-craft (Flour + 1 Egg + 1 MilkBucket → 2 Dough; bucket returns empty), not an Oven recipe. Oven recipes start *with* Dough. This matches the "grid combines, station processes" pattern.

### Changes

- `game/engine/src/block.rs`:
  - `OVEN: BlockId`. Block-entity flag set.
  - Textures: stone hearth body + iron-grille front + roof chimney.
- `game/engine/src/workstation.rs`:
  - `WorkstationKind::Oven` variant. 3 input slots + 1 fuel slot + 1 output slot.
- `game/engine/src/workstation_recipes.rs`:
  - `OVEN_RECIPES` table (each consumes the listed items from input slots; recipe order in slots irrelevant for these):
    - Dough → Bread (200 ticks, same as Furnace baked goods)
    - Dough + Sugar → Sweet Bread (200 ticks)
    - Dough + Sugar + Egg → Cake (300 ticks)
    - Dough + Sugar + Berries → Berry Pie (300 ticks)
    - Dough + Sugar + Pumpkin → Pumpkin Pie (300 ticks)
    - Flour + Sugar + Egg → 8 Cookies (200 ticks)
    - Flour + MilkBucket + Egg → 4 Pancakes + Bucket (200 ticks)
- `game/engine/src/crafting.rs`:
  - **Dough** grid recipe: Flour + Egg + MilkBucket in any cell → 2 Dough + 1 empty Bucket.
  - **Oven** grid recipe: 6 cobblestone in U shape + 1 furnace inside (yes, the oven *contains* a furnace). Reuses Phase 3's furnace.
  - **Bread keeps two recipes side-by-side**:
    - Tier 1's primitive grid recipe (3 wheat horizontal → 1 bread) — preserved as the early-game low-yield path. `food_value = Some(5.0)`.
    - Tier 1.5's Oven path (1 Dough → 1 Bread via Oven) — the upgraded chain. `food_value = Some(8.0)` (better saturation).
  - This is intentional: the primitive path is the kid-friendly bootstrapping route; the Oven path is the upgrade reward for building out the processing chain. Document the dual recipe in Spec 5 §4 update (Phase 12).
- `game/engine/src/item.rs`:
  - `Dough`, `SweetBread`, `Cake`, `Cookie`, `Pancakes` (plural — single item, multiple bites? or a stack of 4?). Recommend: `Pancakes` is a single item that restores 6 hunger; you don't get 4 separate items, you get 1 "Pancakes" item. Or simpler: 4 separate Pancake items. Either's fine — **call it a single Pancakes item for simplicity** (matches Cake-as-single-item pattern in Minecraft).
- `game/engine/src/texture_gen.rs`:
  - Oven block textures + 5 new item textures (Dough, SweetBread, Cake, Cookie, Pancakes).

### Tests

- `crafting::tests::dough_recipe_returns_two_dough_and_bucket`.
- `workstation::tests::oven_bread_from_dough`.
- `workstation::tests::oven_cake_recipe`.
- `workstation::tests::oven_berry_pie_recipe`.
- `workstation::tests::oven_cookie_recipe_outputs_8_cookies`.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Manual: full chain — grow wheat → mill to flour → mill sugarcane to sugar → bucket cow → chicken lays egg → craft Dough in grid → Oven → Cake. Eat Cake → big hunger restore.

### Save compat

1 new block ID + 5 new materials. No format change beyond Phase 2.

---

## Phase 10 — Sugar Beet + Beetroot + Pumpkin + Berries

### Goal

Four more crops to round out the sourcing tier, with their respective harvest/use patterns and any new finished recipes they unlock.

### Sugar Beet (UK English — the pale sugar-extraction cultivar)

- Wheat-clone behaviourally: 4 stages, tills soil, harvest drops 1 sugar beet + 1-3 sugar beet seeds.
- **Not eaten directly** — the sugar beet's purpose is the Mill recipe added in Phase 5 (SugarBeet → Sugar at 3:1 ratio). Eating raw is allowed for safety-net flavour but trivial (`food_value = Some(1.0)`, lower than carrot).
- Visual: pale-cream-and-white root with green leafy top, distinct from beetroot's dark-red colouration.

### Beetroot (UK English — the dark-red salad/cooking root)

- Wheat-clone behaviourally: 4 stages, tills soil, harvest drops 1 beetroot + 1-3 beetroot seeds.
- Beetroot the **food item** restores 2 hunger raw (a step up from sugar beet — beetroot is meant to be eaten).
- **Beetroot Soup** is the headline cooked recipe: Beetroot + Bowl + Furnace → Beetroot Soup (6 hunger, 7.2 saturation). Restores Bowl on consumption.
- **Not millable to sugar** — that's sugar beet's job. The Phase 5 mill-doesn't-grind-beetroot test guards against confusion.
- Visual: distinctively dark-red bulbous root with red-veined leafy top.

### Pumpkin

- Stem-grown like Minecraft: plant Pumpkin Seeds on tilled soil → grows a stem (4 stages) → mature stem spawns a Pumpkin BLOCK in an adjacent air tile.
- Pumpkin block breaks to drop 1 Pumpkin item.
- Pumpkin Pie recipe from Phase 9 consumes it.
- Pumpkin item also placeable back as a block (decoration / Halloween mob lure later).

### Berries

- Berry Bush block: no seed-and-grow loop; placed directly as a bush (4 stages of ripeness, all visually).
- Stage 0 = young, stage 1-2 = leafy, stage 3 = ripe with visible berries.
- Right-click a stage-3 bush → drops 1-3 Berries; bush resets to stage 1 (not stage 0).
- Berry bushes naturally spawn in world-gen (worldgen note: add to plains/forest biome generators; small density). For Tier 1.5 it's enough to spawn them via `/give` and treat worldgen-integration as a follow-on.
- Walking through a stage-3 bush damages player by 0.5 (Minecraft parity, sweet bush). Optional polish — defer if collision system doesn't support partial-damage volumes; in that case bushes are walk-through-safe and gameplay marginally different.

### Changes

Big phase — see the spec phase table for total line estimate. Block IDs assigned: `SUGAR_BEET_STAGE_0..3` (+4), `BEETROOT_STAGE_0..3` (+4), `PUMPKIN` (+1), `PUMPKIN_STEM_0..3` + `MATURE_STEM` (+5), `BERRY_BUSH_0..3` (+4) — **18 new IDs**.

New materials: `SugarBeet`, `SugarBeetSeeds`, `Beetroot`, `BeetrootSeeds`, `Pumpkin` (food, distinct from block — `Item::Block(PUMPKIN)` for the block, `Item::Material(Pumpkin)` for the cooked-ingredient form? **Simpler**: Pumpkin the block, when in inventory, IS the ingredient; no separate food item. Cooking recipes accept `Item::Block(PUMPKIN)` as input.), `Berries`, `Bowl`, `Stew` (generic vegetable stew), `BeetrootSoup`, `BakedPotato`, `LoadedBakedPotato` (baked potato + butter + cheese in grid).

(Adjust the materials list as you go — keep what's needed, drop what isn't.)

### Tests

- One growth + harvest + use test per crop. Pattern follows Tier 1.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.
- Manual: each crop's full loop works.

### Save compat

18 new block IDs + materials. No format change.

---

## Phase 11 — Trade-value annotations + complexity tier

### Goal

Every item in the game gets `complexity_tier() -> u8` and `trade_value() -> Option<u64>`. The value ladder is asserted by tests so future drift is caught immediately.

### Design — the value ladder

| Tier | Description | Example items | `food_value` | `trade_value` (trade-units) |
|:---:|---|---|:---:|:---:|
| 0 | **Raw** — grown, harvested, mob-dropped | Wheat, Carrot, Potato, SugarBeet, Beetroot, Berries, Pumpkin, Sugarcane, RawBeef, MilkBucket, Egg, Stick | 0-2 | 1-3 |
| 1 | **Single-process** — one workstation step | Flour, Sugar, CookedBeef, BakedPotato, Cream, Bread (primitive), BeetrootSoup | 3-6 | 4-10 |
| 2 | **Two-process** — chained workstations | Bread (Oven path), Butter, SweetBread | 6-9 | 12-25 |
| 3 | **Multi-input** — recipe combines 3+ tier 0/1 inputs | Cake, BerryPie, PumpkinPie, Cookie, Pancakes, Stew, Cheese | 10-14 | 30-60 |
| 4 | **Peak crafted** — top-tier recipes with rare or aged inputs | LoadedBakedPotato (with cheese + butter) | 15-20 + saturation bonus | 75-150 |
| 5 | **Reserved** — golden-carrot-style with temporary buffs | (none in Tier 1.5; reserved for future Bitcoin-economy hooks like Satori-laced food?) | TBD | TBD |

`trade_value` units are abstract trade-units, NOT sats. Spec 6 §13 defines conversion to sats per server-config on Bitcoin-enabled servers (Phase 12).

### Changes

- `game/engine/src/item.rs`:
  - `Item::complexity_tier(&self) -> u8`: match against all known items + blocks + tools. Tier 0 default, override per item.
  - `Item::trade_value(&self) -> Option<u64>`: match against all known items, returning the trade-unit value. None for items with no trade-value (e.g. Bedrock as a creative-only block).
- New module `game/engine/src/economy.rs`:
  - Per-item annotation table (consolidates `food_value`, `complexity_tier`, `trade_value` in one place for maintenance — avoids drift between three separate match arms).
  - `Annotation { food: Option<f32>, tier: u8, trade: Option<u64> }` struct.
  - `pub fn annotation_for(item: &Item) -> Annotation`.
  - Refactor `Item::food_value`, `Item::complexity_tier`, `Item::trade_value` to delegate.

### Tests

- `economy::tests::value_ladder_monotonic` — for every item, `trade_value` (when Some) is consistent with `complexity_tier` per the table above. Tier 0 items have trade_value ≤ 3; Tier 1 items have 4 ≤ trade_value ≤ 10; etc. Catches manual annotation mistakes.
- `economy::tests::all_food_items_have_a_tier` — every item with a `food_value` also has a non-zero `complexity_tier`, and vice versa for non-zero food.
- `economy::tests::trade_value_is_set_for_all_player_craftable_items` — items reachable via any recipe (grid, furnace, mill, oven, aging rack) all have `Some` trade_value. Creative-only blocks (Bedrock) can have None.

### Acceptance

- Tests pass. `./check.sh` ALL GREEN.

### Save compat

No format change. Annotations are runtime-derived from item identity.

---

## Phase 12 — Spec 5 + Spec 6 updates

### Goal

Bring `docs/spec/05-gameplay-systems.md` and `docs/spec/06-bitcoin-integration.md` in line with shipped.

### Spec 5 changes

- §3.7 Container Inventories — status note "Block-entity framework DELIVERED Wave 26 / Tier 1.5 / commit `<TBD>`; first implementations: Furnace, Mill, Oven, Aging Rack."
- §4.5 Furnace — status note "DELIVERED Wave 26 / commit `<TBD>`." Wave-6 grid-smelting hack noted as deprecated.
- **Renumber the second §4.5** — there are currently TWO §4.5 headings in Spec 5: line 531 "Furnace / Smelting" (the one Tier 1.5 delivers) and line 1267 "Slash Commands" (introduced by the engine-commands feature). The second collides with the first. Move "Slash Commands" to §4.7 (after the new Workstations section); cascade any §4.x references in Spec 5 or other docs that pointed at the old numbering. Drift audit catches anything missed.
- New §4.6.1 or §4.7 **Workstations** — describes the generic workstation framework + the 4 station types + recipe tables (high-level). Section number depends on the renumbering above.
- §6.2 Food Values — table expanded with all Tier 1.5 entries. New column "Complexity Tier" added. Bread row clarifies dual-path (primitive grid: 5 hunger; Oven: 8 hunger).
- New §3.x **Farming Tier 1.5** — references this foundation doc; high-level recipe-chain diagram.

### Spec 6 changes

- **Create new §13 — Server Economy Modes / Item Trade-Value** (Spec 6 currently runs §1–§12; §13 is the next slot):
  - **Item Trade Value** — abstract internal currency. Every item annotated with `trade_value: Option<u64>`. Player trades, server-shop transactions, and (on Bitcoin-enabled servers) Lightning payouts consume this annotation.
  - **Conversion to sats** — Bitcoin-enabled servers (per §10.3, parent-controlled) configure a `sats_per_trade_unit: f64` ratio in their economy config. Default 0.0 (off).
  - **Player-to-player trade** — out of scope for Tier 1.5; trade-value annotation enables it as a future spec.
  - **Server-shop / vendor blocks** — out of scope for Tier 1.5; same.
  - **Worked example** — a player on a Bitcoin-enabled server who bakes 10 Cakes (Tier 3, ~40 trade-units each) accumulates 400 trade-units; at `sats_per_trade_unit = 0.1`, that's 40 sats redeemable via the server's payout API (which is the same payout endpoint used for proof-of-play). Parent's per-day cap (§10.3) still applies.
- **Retarget the stale parenthetical at Spec 6 line 297** — currently reads "see Spec 6 §6 (server economy modes, TBD)", which is wrong because §6 is "Player Payment Flows". Update to "see Spec 6 §13 (Server Economy Modes / Item Trade-Value)".
- §10.3 — note that trade-value-derived sats are subject to the same parent-controlled gate as proof-of-play-derived sats.

### Acceptance

- Spec 5 + 6 read as complete descriptions of shipped systems.
- Drift audit re-run finds nothing new.

---

## Phase 13 — Axolittle playtest (BLOCKED on his time)

### What he's evaluating

- **Value ladder feel** — does climbing from raw wheat to Cake feel rewarding? Or grindy? Tune `complexity_tier` thresholds.
- **Station ergonomics** — is having 4 distinct workstations too many to juggle? Or is the variety part of the fun? Compare to Minecraft's furnace-and-grid simplicity.
- **Recipe discovery** — without a recipe book UI, can he figure out new recipes? Should the Oven UI auto-suggest valid recipes from current inputs?
- **Aging Rack pacing** — 3 in-game days for cheese — too slow? Too fast? Wall-clock vs in-game-day pacing — which felt right?
- **Trade-value tuning** — are the numbers in the ladder table balanced? Especially for the eventual Bitcoin-conversion layer (Spec 6 §13) — is 40 trade-units per Cake "feels worth the work"?
- **Animal product cooldowns** — egg cooldown (5-10 min) and cow milking cooldown (5 in-game min) — too long? Too short?
- **Sugarcane water requirement** — does requiring water adjacency feel natural, or annoying?
- **Berry-bush damage** — is walking-through-damage fun or just annoying? Skip if collision system can't do partial volumes anyway.
- **What's missing** — any obvious recipe he expected that isn't there? (Honey? Cocoa? Tea? Bread upgrades?)
- **Tier 2 / Tier 3 readiness** — with Tier 1.5 in hand, does Tier 2 (plough + animals) still feel like the right next step, or has something else surfaced as more pressing?

### Outputs

- Memory entries capturing his calls.
- Spec 5 + Spec 6 + this foundation doc updated as needed.
- New foundation specs queued for whatever surfaces (e.g., Tier 2, NPC vendors, Bitcoin-conversion polish).

---

## Polish, fuel, and edge-case detail

A first-pass gap-audit (2026-05-14) flagged these items as under-specified. Filling them here so the implementer doesn't have to redesign mid-build.

### P.1 Furnace fuel table

Coal alone isn't enough — early-game players won't have coal yet. Full fuel table for Phase 3:

| Fuel item | Smelts per unit | Notes |
|---|:---:|---|
| **Coal** | 8 | Primary mid-tier fuel; matches Minecraft |
| **Charcoal** | 8 | Same as coal; produced by smelting Wood Logs in a Furnace (Phase 3 adds the recipe) |
| **Planks** | 1.5 | Early-game low-tier fallback; rounds up to 2 smelts per 2 planks |
| **Wood Logs** | 1.5 | Same as planks (planks-equivalent yield) |
| **Sticks** | 0.5 | Emergency-tier; 2 sticks = 1 smelt |
| **Lava Bucket** | 100 | Late-game; bucket consumed (returns empty after last smelt? Or destroyed? Recommend: returns empty bucket after the 100th smelt) |

**Phase 3 must add**: Wood Log → Charcoal furnace recipe (200 ticks, 1:1) so charcoal is bootable from the first tree. This is the primary fuel chain before coal mining.

### P.2 Workstation break behaviour

When a workstation block is broken (by any tool) while it has items in slots:
- **Output slot contents** drop as item entities at the block position.
- **Input slot contents** drop the same way.
- **Fuel slot contents** drop the same way (including partially-burned fuel — see edge case below).
- **In-progress recipe** is **lost** (progress not refunded). The decision is "don't break workstations mid-cook"; teach via test, not refund.
- **Partially-burned fuel** — if fuel has ticked some smelts already (e.g., coal mid-burn after 3 of its 8 smelts used), the remaining burn ticks are reset on next placement. Track per-block-entity, not per-item-stack. Simpler: don't track partial burns; once a coal is "consumed" it's gone; the next coal starts fresh.

**Test**: `block_interact::tests::breaking_furnace_drops_all_slot_contents`.

### P.3 Workstation block hardness / tool tiers

| Block | Hardness | Required tool | Drop |
|---|:---:|---|---|
| **Furnace** | Stone-tier (Minecraft baseline) | Stone pickaxe or better | 1 Furnace block (intact); contents drop separately per P.2 |
| **Mill** | Wood-tier | Any pickaxe | 1 Mill block; contents drop separately |
| **Oven** | Stone-tier | Stone pickaxe or better | 1 Oven block; contents drop separately |
| **Aging Rack** | Wood-tier | Any pickaxe (mostly wooden frame) | 1 Aging Rack block; contents drop separately |

Break times follow the existing per-tool-per-material formula from Spec 5 §2.1.

### P.4 Stew recipe formalised

**Recipe**: Bowl + 3 vegetables (any of: Carrot, Potato, Beetroot, Pumpkin, Sugar Beet — any combination, at least 3 total, mixed or same) → Stew. Cooked in Furnace, 200 ticks, fuel required.

**Output**: 1 Stew item.

**Food value**: 6 hunger, 7.2 saturation. **Complexity tier**: 3 (multi-input — combines 3+ tier-0 inputs in one workstation step; matches the §8.1 vision-doc Tier 3 definition). **Trade value**: 35.

**Bowl returns**: NO — the Bowl is consumed (per Minecraft). Eating Stew returns the empty Bowl to inventory.

**Beetroot Soup** is a distinct recipe (same workstation): Bowl + Beetroot (single ingredient) → Beetroot Soup. 6 hunger, 7.2 saturation. Bowl returned on eating.

**Test**: `workstation::tests::stew_recipe_accepts_three_vegetables` + `workstation::tests::stew_recipe_rejects_two_vegetables` + `workstation::tests::beetroot_soup_distinct_recipe`.

### P.5 Loaded Baked Potato recipe formalised

**Recipe** (grid, not workstation): BakedPotato + Butter + Cheese → 1 LoadedBakedPotato. Any 3-cell arrangement (shapeless).

**Food value**: 14 hunger, 16 saturation. **Complexity tier**: 4. **Trade value**: 90. Top-tier achievable food in T1.5 (peak of the ladder).

**Test**: `crafting::tests::loaded_baked_potato_recipe`.

### P.6 Pumpkin stem mechanics (Phase 10)

A mature Pumpkin Stem (stage 4, `MATURE_STEM`) attempts each growth tick to spawn a `PUMPKIN` block in one of the 4 cardinally-adjacent tiles to the stem:

1. Randomly pick one of [N, E, S, W] (deterministic via per-block-position RNG seeded by world seed + position).
2. If the picked tile is AIR and the tile *below* it is DIRT, GRASS, or TILLED_SOIL → place a `PUMPKIN` block there.
3. If the picked tile fails (blocked, no soil below) → no pumpkin this tick. Try again next tick with a different RNG pick.
4. If a `PUMPKIN` block exists adjacent to the stem → don't spawn another until that pumpkin is harvested. (One pumpkin per stem at a time.)

**Edge cases**:
- All 4 adjacent tiles blocked / no soil → stem idles indefinitely. Player's responsibility to clear space.
- Harvesting the stem destroys it; harvesting the pumpkin leaves the stem to grow another.

**Test**: `growth::tests::mature_pumpkin_stem_spawns_pumpkin` + `growth::tests::pumpkin_stem_idles_when_all_adjacent_blocked` + `growth::tests::pumpkin_stem_one_pumpkin_at_a_time`.

### P.7 Berry bush regrowth and damage

**Right-click harvest from stage 3** → 1-3 Berries dropped to inventory; bush resets to **stage 2** (not stage 0 — partial regrowth, faster cycle).

**Regrowth cadence**: same growth tick as Tier 1's crops (200 ticks per stage). Stage 2 → 3 in 200 ticks (~10s at 20 TPS). No water-adjacency requirement.

**Breaking the bush** (via any tool / break action) → destroys it permanently. Drops: stage 0-1 → nothing; stage 2-3 → 0-1 Berries (random) + nothing else. Encourages right-click harvest.

**Walk-through damage** — flagged in Phase 10 as "defer if collision can't do partial volumes". For Tier 1.5: **defer it.** Bushes are walk-through-safe; reintroduce damage when collision system supports partial-AABB-damage volumes (likely T7 polish).

**Worldgen integration** — Tier 1.5 doesn't wire worldgen. Bushes are `/give`-only initially. **Workaround for playtesting**: a `/spawn_berries <count>` debug command (extends Phase 9) that scatters berry bushes in a radius around the player on grass tiles. Real worldgen integration is a future polish wave.

**Test**: `block_interact::tests::berry_bush_right_click_drops_berries_and_resets_to_stage_2` + `growth::tests::berry_bush_regrows_stage_2_to_3` + `block_interact::tests::breaking_immature_bush_drops_nothing`.

### P.8 Seed sources for new crops

Tier 1.5 introduces 5 new sourced items (sugarcane, sugar beet, beetroot, pumpkin, berries). The first three are seed-planted; pumpkin starts seed-planted on tilled soil per the T1 model but also generates as world blocks; berries are bush-grown / wild-spawned only.

| Crop | First source | Mechanism |
|---|---|---|
| **Sugarcane** | Worldgen along water-adjacent tiles (rivers, lake shores) | Spawn in initial worldgen at low density |
| **Sugar Beet** | Tall-grass drops (similar to wheat seeds in Minecraft) | Breaking tall-grass yields, at low %, Sugar Beet Seeds |
| **Beetroot** | Tall-grass drops, same mechanism, distinct % | Same as Sugar Beet — drops from grass at low % |
| **Pumpkin** | Worldgen, surface pumpkins in plains/savanna | Spawn as full pumpkins in worldgen at very low density (~1 per chunk in eligible biomes) |
| **Berries (Berry Bush)** | Worldgen, forest edges | Spawn at very low density in forest biomes |

**Tall-grass drop mechanism — verify before extending.** T1.5 P.4/P.10 extend a tall-grass break path that yields seeds. **Re-check the T1 spec before starting Phase 4** — if T1's deliverable includes "break tall grass → wheat seeds drop at low %", Phase 4 extends it; if T1 didn't add that mechanism (it may only ship the till-and-plant-from-known-seeds loop), Phase 4 of T1.5 adds the tall-grass-drops path from scratch as a small pre-requisite. Either way, the worldgen-integration files (`worldgen.rs` and the tall-grass break path) are touched in T1.5 — add them to Phase 4's file list when starting that phase.

**Worldgen integration scope** — Tier 1.5 needs to add worldgen entries for these. Phase 4 (sugarcane) and Phase 10 (others) each include a worldgen-pass extension. Specifics:
- Sugarcane: in `worldgen.rs::populate_chunk`, after lake/river generation, scan for water-adjacent dirt/sand tiles and spawn sugarcane at 5% chance per eligible tile.
- Tall-grass drops: extend the tall-grass break path (verify per above) to also yield Sugar Beet Seeds at 5% and Beetroot Seeds at 5%.
- Pumpkin: in `populate_chunk`, after surface generation, spawn 1-2 pumpkins per chunk at 30% chance, on grass tiles.
- Berry Bush: in `populate_chunk`, 0-3 bushes per chunk in forest-adjacent areas, on grass tiles.

**Phase 4 file list addendum**: when starting Phase 4, add `worldgen.rs` (sugarcane worldgen + verifying tall-grass drops) to the file list shown in the scope table. Same for Phase 10.

### P.9 MilkBucket drink timing

Hold right-click on a MilkBucket in hand → eat-animation. Use the standard food-eat duration referenced in Spec 5's audio/animation tables (`Eat food: Crunching (4 bites over 1.6 seconds)` — that's 32 ticks at 20 TPS). At eat-completion:
- Restore 6 hunger + 7.2 saturation (matches Minecraft milk).
- Clear all active status effects (Minecraft parity).
- Replace MilkBucket in hand with empty Bucket.

**Test**: `block_interact::tests::milk_bucket_drink_restores_hunger_and_clears_effects_and_returns_empty_bucket`.

### P.10 Hopper / automated input hook

Tier 1.5 doesn't add hoppers, but the workstation framework should expose `pub fn try_insert(&mut self, slot: SlotKind, stack: ItemStack) -> Option<ItemStack>` and `pub fn try_extract(&mut self, slot: SlotKind) -> Option<ItemStack>` as public API on `Workstation`. A future hopper spec wires these. **Tier 1.5 ships the methods even though no caller uses them yet** — costs ~20 lines, saves a refactor later.

`SlotKind` enum: `Input(usize)`, `Fuel`, `Output(usize)`.

### P.11 Workstation UI panel layout (sketch)

For the egui panels — not pixel-perfect, but enough that the implementer doesn't redesign from scratch:

**Furnace panel**:
```
┌─────────────────────────────┐
│       FURNACE               │
│                             │
│   ┌──┐                      │
│   │  │ ← Input slot         │
│   └──┘                      │
│    ↓                        │
│  ████░░░░ Burning           │
│    ↓                        │
│   ┌──┐                      │
│   │  │ ← Output slot        │
│   └──┘                      │
│                             │
│   ┌──┐                      │
│   │🔥│ ← Fuel slot          │
│   └──┘                      │
│                             │
│   ── Player inventory ──    │
│   [ … existing inv UI … ]   │
└─────────────────────────────┘
```

**Mill panel**: same as Furnace but **no fuel slot** and "Grinding" label.

**Oven panel**: 3 input slots in a horizontal row (small recipe grid), plus fuel + output + progress bar. "Baking" label. Optional recipe-hint text below progress bar shows the matched recipe ("Now baking: Berry Pie").

**Aging Rack panel**: 1 input + 1 output + recipe-radio-selector (Butter/Cheese) + day counter ("Day 2 of 3"). No fuel. "Aging" label.

Implementer can iterate from these wireframes; precision tuning is a polish concern.

### P.12 Sound effects (deferred but listed for completeness)

| Action | Sound | Tier deferred to |
|---|---|---|
| Furnace burning loop | Soft crackle | T1.5 polish wave |
| Mill grinding | Stone-on-stone | T1.5 polish wave |
| Oven baking | Soft hum + occasional crackle | T1.5 polish wave |
| Aging Rack tick | (silent — slow-time) | n/a |
| Cow milking | Squeaky "milking" sfx | T1.5 polish wave |
| Chicken laying egg | Cluck + plop | T1.5 polish wave |
| Right-click harvest berry | Soft rustle | T1.5 polish wave |

All deferrable; none load-bearing for gameplay. Audio engineer (or texture-gen-style procedural audio) can pick up at the end.

---

## Open design questions (for Axolittle, in priority order)

These are intentionally left as design TBDs for Phase 13. Implementer should ship a sensible default per recommendation; playtest overrides.

1. **Should the Oven UI auto-suggest valid recipes from current inputs?** (Recommended yes — usability win, kid-friendly.)
2. **Should the Aging Rack support multiple parallel recipes (e.g., two cheeses aging at different start dates)?** (Recommended no for Tier 1.5; one recipe per station; place multiple stations.)
3. **Should sweet bread / bread / cake stack to 64, 16, or 1?** (Recommended 64 / 64 / 1 — Cake is a place-and-eat-from-block in Minecraft; for Tier 1.5, Cake is just a high-value food item that stacks to 1 to match its complexity, no cake-block.)
4. **Should Berries replenish for any tool, or only by right-click (no break)?** (Recommended right-click only — encourages the "tend a garden" loop. Breaking the block destroys the bush + drops 0-1 berries.)
5. **Carrots-on-a-stick (Minecraft has them as pig-riding tool)?** Defer to Tier 2 — riding-mob mechanic is Tier 2.
6. **Honey / Cocoa / other tropical-or-niche ingredients?** Defer to a Tier 1.5b polish spec if Axolittle wants them after playtest.
7. **Composter (food waste → bone meal)?** Defer. Bone meal currently has no farming-acceleration mechanic anyway (Tier 1 didn't add Minecraft's bonemeal-as-instant-grow); when it does, a Composter station fits naturally as a 5th workstation.
8. **Salt as an ingredient?** Defer. Either as a Mill recipe on sea-water/salt-block, or as a sourced ore — both add a new sourcing branch (mining for food).
9. **Spoilage?** Defer. Adds depth + grind; not requested.

---

## Acceptance — overall (Tier 1.5)

- All 13 phases complete or explicitly blocked (Phase 13 = playtest gate).
- `./check.sh` ALL GREEN throughout — no phase ships with regressions.
- Spec 5 reflects what shipped; Spec 6 §13 "TBD" is replaced with substance; drift audit re-run finds nothing new.
- Axolittle's gameplay loop works end-to-end on a single dev box: grow wheat + sugarcane → mill to flour + sugar → milk cow → wait for egg → bake Dough → Oven → Cake → eat (big hunger restore). Then re-run with Cheese: wait 3 in-game days, harvest Cheese, make Loaded Baked Potato.
- Memory entry axenstax has farming updated with "Tier 1.5 DELIVERED `<commit>`" line; Tier 2 status confirmed or revised based on playtest.
- This foundation doc gets a status flip from READY TO BUILD → DELIVERED at the top with commit links.
- `docs/foundations/README.md` queue table updated.

---

## Spec maintenance

Per CLAUDE.md "Spec Maintenance" rule:
- All design decisions in this doc that change during implementation get reflected back into Spec 5 and Spec 6 — not just in git history.
- The Tier 2 sketch in `docs/foundations/2026-05-14-farming-system.md` gets refreshed after Tier 1.5 ships and Axolittle's playtest feedback lands.
- The axenstax has farming memory file is the long-term home for Axolittle's design calls. Any new design input during Tier 1.5 implementation goes there first, then propagates to spec.
- Workstation framework + trade-value hook are flagged as cross-game-friendly in shared infra strategy — when other AxeNStax-internal games (other games on the same primitives) start, lift the framework rather than duplicating.

---

## Memory-rule check

- **signet boundary**: N/A. Tier 1.5 touches no Signet/identity.
- **shared infra strategy**: ✓ — workstation framework, trade-value hook, multi-step-recipe pattern, animal-product extraction are all cross-game-friendly. Keep recipe tables in data, not in framework code.
- **pretest check**: implementer should re-verify each phase's claims against actual code state before starting. Block-ID counts, texture-layer counts, variant indices, save-version, protocol-version — all shift between Tier 1 delivery and Tier 1.5 start.
- **axenstax has farming**: this spec is the implementation contract for Axolittle's 2026-05-14 economy ask. Keep them in sync; update the memory when this spec changes.
- **alpha launch posture**: Tier 1.5 is **post-alpha-launch** in priority, same posture as Tier 1. Doesn't gate the alpha. Spec'd now while the design intent is fresh; built when it floats to the top of the queue.
- **bitcoin parent controlled**: ✓ — trade-value lives at the item layer (always present); sats conversion lives at the server-economy layer (parent-controlled, off by default).
- **proof of play is proof of work**: ✓ — Tier 1.5 intentionally does NOT invoke proof-of-play. Cooking is not mining.

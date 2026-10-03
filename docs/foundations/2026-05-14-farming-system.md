# Farming system — Tier 1 (Minecraft-baseline) + Tier 2 sketch (AxeNStax-native plough + draft animals)

**Status**: Tier 1 DELIVERED on main (2026-05-18). Tilled Soil (id 30) + 12 crop-stage blocks (ids 31-42) + Hoe across all 5 tiers + WheatSeeds/Wheat/Bread/Carrot/Potato materials + till/plant/growth (10s per stage, water-adjacency 2x) + harvest + bread recipe + /give parser extensions. Spec 5 §3.9 + §6.2 updated. Phase 11 (Axolittle UX playtest) remains the validation gate. Tier 2 (plough + draft animals) still DESIGN-LOCKED, future spec.
**Date**: 2026-05-14
**Branch**: `feat/farming-tier-1` once started; off `main`.
**Session**: Fresh — implementer should treat this doc as the only brief.
**Trigger**: Spec-code drift audit on 2026-05-14 surfaced the Hoe in Spec 5 §3.8 + §5.2 with no `ToolType::Hoe` in code. Asked Axolittle whether to delete-from-spec or build-it; he picked build-it, then expanded the design substantially over the same session. Memory: axenstax has farming.

---

## TL;DR

AxeNStax has farming. Two tiers, distinct in feel and ambition:

**Tier 1 — Minecraft-baseline** (this spec, fully detailed): Hoes till dirt into tilled soil. Seeds plant in tilled soil. Crops grow over time. Harvested crops yield seeds + food items. Water adjacency speeds growth. Three crop families to start: **wheat, carrot, potato** (the canonical Minecraft starting trio). Hoes ladder across all five tool tiers (Wood → Stone → Iron → Diamond → **Satori**), matching the existing pickaxe/sword/axe/shovel pattern. The Satori Hoe slot already exists in Spec 5 §3.8's recipe table — Tier 1 makes that recipe real.

**Tier 2 — AxeNStax-native plough** (sketched at the end of this doc; needs its own foundation spec): Craftable plough item, drawn by one of four draft animals. Cow + horse run at full speed; donkey + mule at half speed. Mule is bred-only via Minecraft-style donkey × horse cross. Two work modes: **mounted** (player rides, controls direction, ploughs as it walks) and **fenced autonomy** (animal in a fenced area with plough attached works the field unattended — the fence is the spec, no pathing system needed). Tier 2 adds three new mobs (horse, donkey, mule), a generic breeding mechanic, a fence block (Minecraft-baseline but absent from current code), and the plough/yoke/ride-while-attached/fence-autonomy machinery.

Total scope **for this spec (Tier 1 only)**: ~1900 lines (spec → blocks → tools → items → growth → harvest → tests + Spec 5 update). 10 autonomous phases + 1 playtest gate.

---

## Why this lives here

- **Axolittle's design call is fresh and substantive** (2026-05-14). The window for capturing it accurately is now; if not specced, it fades and the design has to be re-extracted later. Per memory pretest check and the spec-maintenance rule in CLAUDE.md ("the specs are what survive rebuilds"), banked design > remembered design.
- **Resolves a documented spec-code drift cleanly.** The Hoe references in Spec 5 §3.8 + §5.2 were flagged as drift in the 2026-05-14 audit. Axolittle's "yes, full farming" turned them from drift into intentional anticipation. This spec makes the spec true again.
- **Cross-game lift, per shared infra strategy**:
  - The growth-tick-with-stages pattern lifts to any game that wants procedural progression (planet evolution, egg incubation, etc).
  - The crop-as-block + harvest-yields-item pattern is the standard voxel-farming shape; reusable.
  - The food-value extension to Spec 5 §6 is AxeNStax-only but mechanically generic.
  - The breeding system planned for Tier 2 is wholly cross-game.
- **Extends the food chain.** Today Spec 5 §6 lists raw + cooked meats from mobs. Crops add a parallel grain/vegetable pillar. Bread (3 wheat → 1) gives a craftable food path independent of mob-killing — the kid-friendly survival arc.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/block.rs` — block ID registry (currently 30 IDs: AIR=0..SATORI_BLOCK=29). Tier 1 adds ~6 new IDs (tilled soil + 3 crops × growth-stages-as-separate-IDs OR 1-ID-with-metadata; design decision below).
- `game/engine/src/item.rs` — `MaterialId` enum (currently ~26 variants ending in `Satori`). Adds seed materials (`WheatSeeds`, `Carrot` doubles as seed + food, `Potato` doubles too) + flour/bread items.
- `game/engine/src/crafting.rs` — `ToolType` enum (currently `Pickaxe`, `Axe`, `Sword`, `Shovel`, `Bow`). Adds `Hoe`. `Tool::attack_damage` and related match arms gain Hoe rows. Recipes added for 5 hoe tiers + bread.
- `game/engine/src/block_interact.rs` — block break + place handling. Adds two new actions: **till** (right-click dirt with hoe) and **plant** (right-click tilled soil with seed). Plus harvest is just a regular break with the seed-yielding drop logic.
- `game/engine/src/game_loop.rs` or new `growth.rs` — per-tick crop growth processing. Loops over chunks containing crop blocks, advances stages on a configurable interval.
- `game/engine/src/texture_gen.rs` — pixel art for tilled soil + 3 crop families × multiple growth stages + seed/grain/vegetable item textures + hoe item textures (5 tiers × matches existing pickaxe/sword pattern).
- `game/engine/src/audio.rs` — small additions: till sound, plant sound, harvest sound. Optional polish.
- `game/engine/src/save.rs` — crop blocks need to round-trip in saves (they're just block IDs, so this is automatic if added correctly to the registry).

### Related specs

- `docs/spec/05-gameplay-systems.md §3.8` — Satori Hoe recipe (already in spec, currently unbuildable; this spec makes it real).
- `docs/spec/05-gameplay-systems.md §5.2` — Tool Types table lists Hoe (already in spec, no enum variant; this spec adds it).
- `docs/spec/05-gameplay-systems.md §6` — Food/hunger system. Spec edits in Phase 10 add bread + cooked-from-crop variants to the food table.
- `docs/spec/06-bitcoin-integration.md §2.2` — Proof of Play. Tier 1 farming has no proof-of-play hook (you don't "mine" dirt with a hoe in a way that should yield Bitcoin). Tier 2's plough on tilled soil is a future open question (see open questions section).
- `docs/foundations/2026-05-12-proof-of-play-clarification.md` — useful style reference for design-heavy spec writeups.

### Memory pointers

- axenstax has farming — full design from Axolittle, including Tier 2 four-animal table.
- signet boundary — N/A here, but a reminder: nothing in farming touches Signet/identity.

### What does NOT exist yet (and Tier 1 doesn't need)

- **Fence blocks.** Tier 1 doesn't need them (manual hoe doesn't care about boundaries). Tier 2 needs them — fence-autonomy depends on the player being able to enclose an area. Add as part of Tier 2 spec.
- **Horse, donkey, mule mobs.** Tier 1 doesn't need them. Tier 2 prerequisite.
- **Generic breeding system.** Tier 2 prerequisite.
- **Riding mechanic.** Tier 2 prerequisite (and probably wider — pigs, cows, etc. could be saddleable too; that's the open question in memory).
- **Light-level system.** Crop growth in Minecraft is light-gated (crops won't grow in pitch dark). The engine has a `block_light` / `sky_light` field on `ChunkSection` per Spec 2 §2.6, so the data is there — but whether the *propagation* is wired up enough for crops to read meaningful light values is a code check Phase 6 needs to do. **If light propagation isn't viable yet, drop the light gate for Tier 1** and growth becomes time-only. Don't block farming on the lighting system; revisit when lighting is mature.

---

## Scope

| # | Phase | Files | Est. lines | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-14-farming-system.md` | ~700 | ✓ |
| 2 | Tilled-soil block + ledger | `block.rs`, `texture_gen.rs`, `protocol.rs` (PROTOCOL_VERSION bump) | ~150 | ✓ |
| 3 | `ToolType::Hoe` + Wood-tier hoe recipe + till action (right-click dirt → tilled soil) | `crafting.rs`, `block_interact.rs`, `texture_gen.rs`, tests | ~250 | ✓ |
| 4 | Seed/food items (`WheatSeeds`, `Wheat`, `Bread`; carrot + potato as dual-purpose seed/food) + textures | `item.rs`, `texture_gen.rs`, `entity_model.rs`, tests | ~200 | ✓ |
| 5 | Crop blocks (wheat / carrot / potato) — single block ID per crop with growth stage stored in block-state metadata, OR 4 IDs per crop (one per stage). **Spec proposes 1-ID-with-state** but Phase 5 may flip if state-bit support doesn't exist yet | `block.rs`, `chunk.rs` (state bits if needed), `texture_gen.rs`, tests | ~350 | ✓ |
| 6 | Plant action + crop-growth tick (advance stage every N ticks; respects light if available, water adjacency multiplier) | `block_interact.rs`, new `growth.rs` module, `game_loop.rs` integration, tests | ~300 | ✓ |
| 7 | Harvest action — break a mature crop, yields seed(s) + food item; reset to tilled soil. Bread crafting recipe (3 wheat → 1 bread) | `block_interact.rs`, `crafting.rs`, food values in `item.rs`, tests | ~200 | ✓ |
| 8 | Per-tier Hoe ladder (Stone, Iron, Diamond, Satori). Durability + till-speed differentiation. Recipes follow existing pickaxe pattern | `crafting.rs`, tests | ~150 | ✓ |
| 9 | `/give` command extensions for new items (seeds, crops, bread, hoes) so playtesters can spawn-and-test without survival grind | `commands/builtins/give.rs`, tests | ~80 | ✓ |
| 10 | Spec 5 update — add tilled soil + crop blocks to block-list section; flesh out food table with bread + crop entries; deferred markers updated; tool tier table notes Hoe lacks attack damage (it's a utility tool) | `docs/spec/05-gameplay-systems.md` | ~80 | ✓ |
| 11 | Axolittle playtest — feel of growth pacing, harvest reward, recipe values | n/a | 0 | ✗ blocked |

**Total**: ~1900 lines spec + code + tests for Tier 1. Phase 11 is the playtest gate — tune from there.

Phases 2–10 are independently testable (each has its own unit tests). Recommended order: 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10. Phase 8 (per-tier hoes) can land in parallel with Phase 9 (give command extensions); both depend on Phase 3.

---

## Phase 1 — This spec

You're reading it. ✓ Move on.

---

## Phase 2 — Tilled-soil block

### Goal

Add a new block type representing tilled soil — the substrate crops plant into. No interaction with hoes yet (Phase 3 wires that). Phase 2 just makes the block exist + serialise + render.

### Changes

- `game/engine/src/block.rs`:
  - Add `pub const TILLED_SOIL: BlockId = 30;` (next free ID after `SATORI_BLOCK = 29`).
  - Add `pub const TEX_TILLED_SOIL: u32 = 112;` (next free texture-array layer after the Satori-item texture at 111).
  - Register `BlockDef { name: "genesis:tilled_soil", solid: true, transparent: false, gravity: false, color: [0.45, 0.30, 0.18], tex_top: TEX_TILLED_SOIL, tex_bottom: TEX_DIRT, tex_side: TEX_DIRT }` in `BlockRegistry::new()`.
  - Add to `is_known` test, add a tilled-soil-specific test asserting registry presence and basic properties.
- `game/engine/src/texture_gen.rs`:
  - Add `gen_tilled_soil()` returning a 16×16 pixel texture — dark brown base with subtle furrow lines (procedural seed-driven noise like the other dirt-family textures). Push to layer 112.
  - Update `texture_count()` to 113.
  - Update the inline doc-comment counting layers.
- `game/engine/src/protocol.rs`:
  - Bump `PROTOCOL_VERSION: u32 = 9` with a new entry in the version-history doc-comment: `v9 (2026-05-XX): Wave 26 begins farming — adds TILLED_SOIL = id 30`. (The bump prevents stale clients from rendering tilled soil as AIR like the Wave-25 deepslate-bump rationale.)

### Acceptance

- `cargo test --bin axenstax-engine` green (new test passes; existing tests unaffected).
- `./check.sh` ALL GREEN.
- Manual: in-game, `/give satori_pickaxe 1` then break a dirt block, place a tilled-soil block via `/give tilled_soil 64` once Phase 9's give support is added — but for Phase 2 just verify the block constant compiles and registry knows about it.

### Save compat

New block ID. Existing saves won't have any tilled-soil blocks; loading them is a no-op. No save format change.

---

## Phase 3 — Hoe tool + till action

### Goal

`ToolType::Hoe` exists. Wooden hoe recipe works. Right-clicking a dirt block with a wooden hoe in hand converts it to tilled soil.

### Changes

- `game/engine/src/crafting.rs`:
  - Add `Hoe` to the `ToolType` enum (after `Shovel`, before `Bow` to keep the bow-as-special pattern at the end).
  - Add Wood-tier Hoe to recipe matching (`match_recipe`): pattern is two materials top row (wood-tier or higher) + 1 stick centre + 1 stick bottom-centre. Same as Minecraft hoe pattern. Output: `Tool { tool_type: Hoe, material: Wood, durability: max_durability(Wood) }`.
  - `Tool::attack_damage` for `(Hoe, _)`: returns `1.0` (hoes are utility, not weapons; matches Minecraft).
  - `Tool::name` for `(Hoe, Wood)`: `"Wooden Hoe"`. (Other tiers added in Phase 8.)
  - `mining_speed` is already a per-material function, so Hoe inherits the standard tier multipliers (a Wooden Hoe mines blocks at Wood-tier speed when used). No special action there.
  - Add a `is_hoe()` helper or a `try_till(...)` predicate for use in `block_interact.rs`.
- `game/engine/src/block_interact.rs`:
  - Extend the right-click handler to dispatch to a new `try_till_block(player, world, pos)` path when the held item is a `Hoe` and the targeted block is `DIRT` (id 1) or `GRASS` (id 3).
  - On successful till: `world.set_block(pos, TILLED_SOIL)`, decrement hoe durability via `inventory.use_hotbar_tool`, play till sound (Phase 7 polish — for Phase 3 just leave a TODO comment).
  - Hoes do **NOT** till the SIDES of dirt blocks — only the top face. Top-face check via the raycast hit-face.
- `game/engine/src/texture_gen.rs`:
  - Add `gen_item_hoe_wood()` returning a 16×16 pixel texture — wooden handle + flat angled head. Match the pickaxe-item pattern at `gen_item_tool_wood` for visual consistency.
  - Push at layer 113 (post-tilled-soil).
  - Update `texture_count()` to 114, update inline counting comment.
- `game/engine/src/entity_model.rs`:
  - `tool_texture` match arm for `(Hoe, Wood)` returns the new layer index. Other tiers placeholder until Phase 8.

### Tests

- `crafting::tests::wooden_hoe_recipe_matches` — 2 planks + 2 sticks in hoe pattern → `Tool::Hoe(Wood)`.
- `crafting::tests::hoe_attack_damage_is_one` — non-weapon utility.
- `block_interact::tests::hoe_tills_dirt_top_face` — raycast hit on dirt top face with hoe in hand → block becomes tilled soil + durability decrements.
- `block_interact::tests::hoe_does_not_till_dirt_side_face` — same setup but hit on side face → no change.
- `block_interact::tests::hoe_tills_grass_top_face` — grass also tillable (Minecraft parity).
- `block_interact::tests::pickaxe_does_not_till_dirt` — wrong tool type → no change.

### Acceptance

- All new tests pass.
- `./check.sh` ALL GREEN.
- Manual: `/give wood_hoe 1`, find dirt or grass, right-click top face → block becomes tilled soil. Right-click stops working when durability hits zero.

### Save compat

`ToolType::Hoe` added to the enum. **Variant order matters for bincode.** Add `Hoe` between `Shovel` and `Bow`, NOT at the end — wait, actually adding *anywhere* shifts subsequent indices. Verify: `Shovel` is currently variant 3 (Pickaxe=0, Axe=1, Sword=2, Shovel=3, Bow=4). Adding `Hoe` after `Shovel` makes Bow shift from 4 → 5. **Existing saves with bows would deserialize as Hoes.** Two safe options:
1. **Add `Hoe` at the end** (after `Bow`). Variant index = 5. Existing saves unaffected. Cosmetically less-clean (bows-as-last-special is broken) but safe.
2. **Use `#[serde(rename = ...)]` or explicit discriminants** to lock variant ordering. More involved.

**Recommended**: Option 1 (append). Cosmetic ordering is a documentation concern (a doc-comment on the enum can explain "Hoe is last for bincode-compat reasons; see foundation 2026-05-14"). Save compat is the load-bearing concern.

---

## Phase 4 — Seed + food items

### Goal

Seed and food materials exist as items, displayable in inventory + droppable from `/give`. No planting yet (Phase 6) — Phase 4 just makes the items real.

### Changes

- `game/engine/src/item.rs`:
  - Add to `MaterialId` enum (at end, preserving bincode index of existing variants):
    - `WheatSeeds`
    - `Wheat`
    - `Bread`
    - **(Carrot doubles as seed + food)** — already absent from current enum, add `Carrot`.
    - **(Potato similarly)** — add `Potato`.
  - `Item::name` arms for each.
  - `Item::color` arms for each (wheat-seeds tan, wheat golden, bread brown, carrot orange, potato light-brown).
  - `food_value` for `Wheat`: `None` (raw wheat is not a food per Minecraft; you bake bread). For `Bread`: `Some(5.0)`. For `Carrot`: `Some(3.0)`. For `Potato`: `Some(1.0)` raw — baked-potato is a future polish wave (skip for Tier 1).
- `game/engine/src/texture_gen.rs`:
  - 5 new gen_item_X() functions for the 5 new items.
  - Push at layers 114..118.
  - Update `texture_count()` to 119, inline counting comment.
- `game/engine/src/entity_model.rs`:
  - `material_texture` arms for the 5 new items, mapping to their texture layer indices.

### Tests

- `item::tests::wheat_seeds_displays_correctly` — name + colour + food_value (None) + max_stack (64).
- `item::tests::bread_is_food` — `food_value() == Some(5.0)`.
- `item::tests::carrot_is_food_and_seed_via_planting` — food_value Some(3.0); planting test deferred to Phase 6.

### Acceptance

- New tests pass.
- `./check.sh` ALL GREEN.

### Save compat

5 new MaterialId variants appended at the end. Bincode-compatible. Existing saves with no carrots/wheat/etc. load unchanged.

---

## Phase 5 — Crop blocks

### Goal

Wheat, carrot, and potato crops exist as blocks with growth stages. They're placeable + breakable + serialisable. Phase 5 doesn't add the planting action (that's Phase 6 because plant requires the seed + tilled-soil interaction); it just makes the blocks exist.

### Design call: stage representation

Two viable approaches:

**Option A — One block ID per stage (12 new IDs total):** `WHEAT_STAGE_0`, `WHEAT_STAGE_1`, ..., `POTATO_STAGE_3`. Simple. Direct registry lookup. But pollutes the registry with 12 tightly-related IDs.

**Option B — One ID per crop, growth stage in block-state metadata (3 new IDs total):** `WHEAT`, `CARROT`, `POTATO` block IDs, with a 2-bit growth-stage field stored in chunk-side state. Requires the chunk format to support per-block state bits — Spec 2 §2.x calls for it but verify code support before relying on it.

**Recommended**: **Option A for Tier 1.** State-bit support in the chunk format isn't load-bearing-tested yet, and 12 extra IDs is harmless (block ID is `u16`; we're at ~30/65535 used). When/if a state-bit system lands, a future migration can collapse.

### Changes

- `game/engine/src/block.rs`:
  - Add 12 new BlockIds: `WHEAT_STAGE_0..WHEAT_STAGE_3`, `CARROT_STAGE_0..3`, `POTATO_STAGE_0..3`. IDs 31..42.
  - Register each in `BlockRegistry::new()`. All transparent (you can walk through them), all `gravity: false`, all `solid: false` (no collision). Texture indices wired to per-stage textures (Phase 5 generates these).
  - When BROKEN with no tool / wrong tool, crops drop nothing if not mature (stage < 3). At maturity (stage 3), break drops handled in Phase 7.
- `game/engine/src/texture_gen.rs`:
  - 12 new texture-gen functions covering the 12 stages. Visual progression: stage 0 = sprout (1-2 pixels at base), stage 3 = full plant (fills the block bounds, distinctive per crop family).
  - Push at layers 119..130.
  - Update `texture_count()` to 131.
- `game/engine/src/protocol.rs`:
  - Bump PROTOCOL_VERSION to 10 with a Wave-26 doc-comment entry covering tilled soil + crop blocks.

### Tests

- `block::tests::all_crop_stages_known_to_registry` — 12 IDs all return a valid `BlockDef`.
- `block::tests::crops_are_transparent_and_walkable` — collision + transparency invariants.

### Acceptance

- Tests pass. `./check.sh` green.
- Manual: `/give wheat_stage_3 1`, place it, walk through it (no collision). Visual-check the texture progression by giving stages 0/1/2/3 next to each other.

### Save compat

12 new block IDs. Existing saves don't have them; unchanged. PROTOCOL_VERSION bump forces stale clients to reject (no half-rendered crops).

---

## Phase 6 — Plant action + growth tick

### Goal

Right-clicking a tilled-soil block with seeds in hand plants a stage-0 crop. Crops advance stage every N ticks. Water-adjacent crops grow faster.

### Changes

- `game/engine/src/block_interact.rs`:
  - Extend the right-click handler: when held item is a `Material(WheatSeeds | Carrot | Potato)` AND target block is `TILLED_SOIL`, call `try_plant(player, world, pos, seed_kind)`.
  - On successful plant: `world.set_block(pos.above(), WHEAT_STAGE_0 | CARROT_STAGE_0 | POTATO_STAGE_0)`. Decrement seed count by 1.
  - Plant requires the block ABOVE the tilled soil to be AIR — refuse if blocked.
- New module `game/engine/src/growth.rs`:
  - `pub fn tick_crop_growth(world: &mut World, tick_counter: u64) -> Vec<BlockChange>` — pure free function (matches the `spawning::tick_mob_spawning` pattern for cross-server-side reuse).
  - Iterates loaded chunks. For each crop block found, computes whether to advance:
    - Base growth interval: 200 ticks per stage (10 seconds at 20 TPS) — cumulative 600 ticks (~30s) from sprout to mature. Tunable constant `CROP_GROWTH_TICKS_PER_STAGE: u64 = 200`.
    - Water-adjacency multiplier: scan the 8 horizontal neighbours within 4 blocks for `WATER` (id 11). If any found, growth interval halved (100 ticks per stage = ~15s sprout-to-mature). Matches Minecraft.
    - Light gate (only if block_light system is wired in code — Phase 6 starts with a quick check; if light propagation isn't reliable enough yet, **drop the gate** and growth is time-only). When implemented: stage advance only if `block_light + sky_light >= 9`.
  - Returns a `Vec<BlockChange>` for the broadcast path (matches the pattern from `falling_blocks::tick_falling_blocks`).
- `game/engine/src/game_loop.rs`:
  - Call `growth::tick_crop_growth` once per tick (or less frequently — a 20-tick interval gives 1Hz growth processing which is plenty since per-stage interval is 200 ticks). Apply returned `BlockChange`s.
- `game/engine/src/hosted_server.rs`:
  - Mirror the call in the server tick loop so multiplayer growth is server-authoritative.

### Tests

- `block_interact::tests::seeds_plant_on_tilled_soil` — wheat seeds + tilled soil → wheat stage 0 placed above; seed count decrements.
- `block_interact::tests::seeds_refuse_on_wrong_block` — wheat seeds + plain dirt → no change.
- `block_interact::tests::seeds_refuse_when_above_blocked` — air above must be free.
- `growth::tests::stage_advances_every_200_ticks` — set up a wheat stage 0, tick 200 times, expect stage 1.
- `growth::tests::water_adjacent_grows_faster` — same setup with water within 4 blocks → 100 ticks per stage.
- `growth::tests::mature_crop_does_not_advance` — stage 3 stays stage 3 (no stage 4).
- `growth::tests::all_three_crops_grow_independently` — wheat, carrot, potato all use the same tick logic.

### Acceptance

- Tests pass. `./check.sh` green.
- Manual: `/give wheat_seeds 64`, till some dirt, plant seeds, wait 30 seconds (or `/time` if engine commands let you fast-forward). Crop stages should visibly progress. Place water nearby — adjacent crops grow faster.

### Save compat

No new blocks since Phase 5 covered crop IDs. No format change.

---

## Phase 7 — Harvest + bread crafting

### Goal

Breaking a mature crop (stage 3) yields seeds + the food item. Reset to tilled soil so the player can re-plant. Bread can be crafted from 3 wheat (Minecraft pattern).

### Changes

- `game/engine/src/block_interact.rs` (in the existing block-break handling):
  - When breaking a crop block:
    - **Stage < 3**: drop nothing. Block becomes air. (Minecraft: immature crops drop nothing.)
    - **Stage 3 (mature)**:
      - **Wheat**: drop 1 wheat + 1-3 wheat seeds (random, e.g. uniform 1..=3). Reset to tilled soil (NOT air — so player can immediately re-plant).
      - **Carrot**: drop 1-4 carrots (random uniform 1..=4). No separate seed; carrot doubles as seed.
      - **Potato**: drop 1-4 potatoes. No separate seed; potato doubles as seed.
- `game/engine/src/crafting.rs`:
  - Add bread recipe to `match_recipe`: 3 wheat in a horizontal line (any row) → 1 bread.
- `game/engine/src/registry.rs` or wherever block→drop logic lives:
  - Wire crop drop tables into `mine_drop` so the standard break path produces the right items.

### Tests

- `block_interact::tests::mature_wheat_drops_wheat_and_seeds_resets_to_tilled_soil` — break stage-3 wheat → inventory gains wheat + seeds; block becomes tilled soil.
- `block_interact::tests::immature_wheat_drops_nothing` — break stage-1 wheat → no drops; block becomes air.
- `block_interact::tests::mature_carrot_drops_carrots_only` — no seed-as-separate-item.
- `block_interact::tests::mature_potato_drops_potatoes_only`.
- `crafting::tests::three_wheat_makes_one_bread` — recipe matches; output is 1 bread.

### Acceptance

- Tests pass. `./check.sh` green.
- Manual: full loop — till → plant → grow → harvest → eat (bread restores hunger).

### Save compat

No format change.

---

## Phase 8 — Per-tier Hoe ladder

### Goal

Stone, Iron, Diamond, and Satori hoes exist with the same tier ladder as pickaxes (durability + colour differentiation). Recipes follow the standard hoe pattern with the appropriate head material.

### Changes

- `game/engine/src/crafting.rs`:
  - Recipes for Stone, Iron, Diamond, Satori hoes (same hoe pattern, swap head material).
  - `Tool::name` arms: "Stone Hoe", "Iron Hoe", "Diamond Hoe", "Satori Hoe".
  - All tiers inherit the per-material `mining_speed` and `attack_damage` (which is 1.0 for hoes regardless of tier — a Satori Hoe is not a weapon).
  - Durability tracks the per-material `max_durability` already defined.
- `game/engine/src/texture_gen.rs`:
  - 4 new gen_item_hoe_X() functions (stone, iron, diamond, satori). Match the existing per-tier pickaxe colour palette (e.g., diamond gets the cyan accent; satori gets the orange).
  - Push at layers 131..134, update count to 135.
- `game/engine/src/entity_model.rs`:
  - `tool_texture` arms for `(Hoe, Stone)`, `(Hoe, Iron)`, `(Hoe, Diamond)`, `(Hoe, Satori)`.

### Tests

- `crafting::tests::all_five_hoe_tiers_recipe_matches` — parametrised across tiers.
- `crafting::tests::satori_hoe_durability_matches_satori_pickaxe` — both 2031.
- `crafting::tests::all_hoes_have_attack_damage_one` — explicit assertion that hoe damage doesn't tier up.

### Acceptance

- Tests pass. `./check.sh` green.
- Manual: craft each hoe tier, verify visual + durability + name.

### Save compat

No format change (variants already exist from Phase 3).

---

## Phase 9 — `/give` extensions

### Goal

`/give` accepts the new items by sensible names so playtesters can spawn-and-test without grinding.

### Changes

- `game/engine/src/commands/builtins/give.rs`:
  - Material parser: add `"wheat_seeds" | "wheatseeds" | "wseeds" => Some(M::WheatSeeds)`, `"wheat" => Some(M::Wheat)`, `"bread" => Some(M::Bread)`, `"carrot" => Some(M::Carrot)`, `"potato" => Some(M::Potato)`.
  - Tool parser: extend material match to accept `"wood"|"stone"|"iron"|"diamond"|"satori"` × `"hoe"` → returns `(ToolType::Hoe, ToolMaterial::X)`.
  - Block parser: add tilled-soil + the 12 crop-stage IDs by name (`tilled_soil`, `wheat_stage_0`, etc., plus convenience aliases `wheat_mature` → stage 3).

### Tests

- `commands::tests::give_seeds_works`, `give_bread_works`, `give_satori_hoe_works`, `give_tilled_soil_works`.

### Acceptance

- Tests pass.
- Manual: `/help give` shows the new options (or at least `/give satori_hoe 1` parses and lands in inventory).

### Save compat

No format change.

---

## Phase 10 — Spec 5 update

### Goal

Bring `docs/spec/05-gameplay-systems.md` in line with what shipped. The Hoe references that were drift become real; the food table extends; deferred markers update.

### Changes

- §5.2 Tool Types table: confirm Hoe row matches what shipped (utility tool, no attack damage, tills dirt/grass).
- §3.8 Satori recipe table: confirm Satori Hoe row matches shipped recipe (was already present from a prior wave; this cycle just makes the recipe real).
- §6 Food values: add Wheat (None — not edible), Bread (5.0 HP), Carrot (3.0 HP), Potato (1.0 HP).
- New section §3.x **Farming** (or append to §3.8 if logically adjacent): describe tilled-soil + crops + growth + water + harvest. Reference this foundation doc.
- §5.1 Tool tier table: no changes (hoes use the existing per-material durability/speed; no special row needed).

### Acceptance

- Spec reads as a complete description of the system that just shipped.
- Drift audit re-run on Spec 5 returns zero new drifts in farming-touched sections.

---

## Phase 11 — Axolittle playtest (BLOCKED on his time)

### What he's evaluating

- **Growth pacing**: 30 seconds sprout-to-mature feels fun? Too slow? Too fast? (Tunable via `CROP_GROWTH_TICKS_PER_STAGE`.)
- **Yield balance**: 1-3 wheat seeds + 1 wheat per harvest — grindy or generous? 1-4 carrots/potatoes — same question.
- **Hoe tier feel**: does a Satori Hoe feel meaningfully better than a Wooden Hoe, or is it cosmetic since attack damage doesn't tier?
- **Bread** as a food: does the 3-wheat → 1-bread ratio feel right for the survival arc?
- **Visual progression** of crop stages: is the per-stage texture differentiation clear enough to tell at a glance?
- **Recipe set**: anything missing for Tier 1? (E.g., baked potato? Carrot-on-a-stick? Wheat → hay bale storage block?)
- **Tier 2 readiness**: with Tier 1 in hand, is the next move drafting the Tier 2 spec, or does Tier 1 expose unanticipated design decisions that need to be answered first?

### Outputs

Memory entries capturing his calls. Spec 5 + this foundation doc updated as needed. New foundation spec for Tier 2 if green-lit.

---

## Tier 2 sketch (future foundation spec)

**Status**: DESIGN-LOCKED at the high level (per Axolittle 2026-05-14). Needs its own foundation spec covering the substantial new mechanics. Not in this spec's implementation scope.

### What Tier 2 adds

Per memory axenstax has farming:

| Mob | Spawn | Speed | Code status |
|---|---|---:|---|
| Cow | Natural | Full | ✅ exists |
| Horse | Natural | Full | ❌ new mob |
| Donkey | Natural | Half | ❌ new mob |
| Mule | Bred (Donkey × Horse, Minecraft cross) | Half | ❌ new mob |

- **Plough** — craftable item. Recipe tier TBC (likely iron-tier per Minecraft "lots of utility tools are iron"). Attaches to a draft animal via yoke/harness mechanic (separate item or built-in TBC).
- **Two work modes**:
  - **Mounted**: player rides the animal; ploughs as it walks; tills the strip the animal traverses.
  - **Fenced autonomy**: animal in a fenced area with plough attached works the field unattended. The fence is the spec — animal wanders the fenced area, ploughing as it walks. No pathing/priority system required.
- **Fence block** — Minecraft-baseline but absent from current code (`grep -n FENCE block.rs` returns nothing). Tier 2 adds it. Standard 4-way auto-connecting fence shape.
- **Generic breeding system** — Mule = Donkey × Horse is the headline use case but the system should generalise (cows breed cows, horses breed horses, etc.). Pattern: feed two adult animals of compatible types → spawn baby of the appropriate output type. Cross-breeding rules in a small lookup table.
- **Riding mechanic** — required for the mounted work mode. May extend beyond draft animals to other rideable mobs (pigs, etc.) — open question.

### Cart + `lubrication: f32` data laydown

Per `docs/vision/farming-economy-long-run.md` §10.7, T2 ships an animal-drawn cart entity with a hidden `lubrication: f32` field (range `0.0..=1.0`). At T2 the field is a **data-only no-op** — no speed multiplier, no wear modifier, no oil consumption. Initial value `1.0`. Persists in save format.

Why lay it down now: T5 introduces the Oil Press + Olive Oil + Peanut Oil; T5's oil-mechanic foundation spec wires the consumer (oil → refills `lubrication`, lubrication → speed/wear modifiers). Laying the field at T2 avoids a save-format migration at T5. T8's Industrial Mill / Power Oven re-use the same field on workstation entities.

Behaviour gates (all activate at T5+, not T2):
- Above 0.5 → +20% cart top speed, normal wear.
- Below 0.2 → -30% top speed, +50% wear (parts wear faster).
- Empty (0.0) → cart works but slow + noisy + accelerated breakdown.

T2 ships with the field always at `1.0` (or a default-fill) so carts behave as if fully lubricated until T5 activates the drain + oil-application path. No UI surface at T2.

### Tier 2 dependencies on Tier 1

Tier 2 needs Tier 1's tilled-soil block and crop-system to mean anything (a plough that tills nothing wouldn't make sense). Other than that, the dependencies are weak — Tier 2's mob/breeding/plough/fence work is mostly orthogonal to Tier 1's hoe/seed/crop work.

### Tier 2 open questions (also in memory)

- Plough recipe tier (single iron-tier? Per-tier ladder?)
- Cow vs horse mechanical difference beyond speed (strip width? Carry?)
- Fence-autonomy edge cases (incomplete enclosure, no soil left, multiple animals)
- Manual plough usable without animal? (E.g., player drags it for slow one-block work.)
- Bitcoin-economy hooks (gem-fertilised soil, proof-of-play crop variants?)
- Interaction with meteor / Bitcoin Golem must-haves
- Manual hoe stays viable post-plough?
- Other rideable mobs (pigs etc.)?

### When to spec Tier 2

Recommended order: **deliver Tier 1 first**, get Axolittle's playtest feedback, THEN draft Tier 2. Reasons:
- Axolittle's Tier-2 design might shift after he tries Tier 1 (he might want different yields, faster growth, etc., that change Tier 2's balance).
- The breeding system + new mobs are substantial work; better to bank Tier 1 as a complete shippable unit first.
- Tier 2 has many open design questions that benefit from Tier-1-in-hand context.

---

## Acceptance — overall (Tier 1)

- All 11 phases complete or explicitly blocked (Phase 11 = playtest gate).
- `./check.sh` ALL GREEN throughout — no phase ships with regressions.
- Spec 5 reflects what shipped; drift audit re-run finds nothing new.
- Axolittle's gameplay loop works end-to-end on a single dev box: till → plant → wait → harvest → bake bread → eat.
- Memory entry axenstax has farming updated with "Tier 1 DELIVERED <commit>" status line; Open Questions narrowed.
- This foundation doc gets a status flip from READY TO BUILD → DELIVERED at the top with commit links.

---

## Spec maintenance

Per CLAUDE.md "Spec Maintenance" rule:
- All design decisions in this doc that change during implementation get reflected back into Spec 5 (and any other relevant spec) — not just in git history.
- The Tier 2 sketch in this doc gets promoted to its own foundation spec when Tier 1 ships and Axolittle green-lights starting Tier 2 work. This doc updates with a "Tier 2 spec: <link>" pointer at that time.
- The axenstax has farming memory file is the long-term home for Axolittle's design calls. Any new design input from him during implementation goes there first, then propagates to spec.

---

## Memory-rule check

- **signet boundary**: N/A. Farming touches no Signet/identity.
- **shared infra strategy**: ✓ — growth-tick + breeding + crop-as-block are cross-game-friendly patterns. Don't hard-code AxeNStax assumptions into the growth tick (e.g., "bread restores 5 HP" lives in `food_value`, not in growth).
- **pretest check**: implementer should re-verify each phase's claims against actual code state before starting (the existing block-ID count, the texture-layer count, the variant indices) — these may have shifted since this spec was written.
- **axenstax has farming**: this foundation spec is the implementation contract for the design banked there. Keep them in sync.
- **alpha launch posture**: farming is **post-alpha-launch** in priority. Doesn't gate the alpha. Spec'd now while Axolittle's design is fresh; built when Tier 1 floats to the top of the queue.

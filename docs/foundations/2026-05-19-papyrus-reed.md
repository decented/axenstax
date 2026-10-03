# Papyrus Reed — water-adjacent T1 plant + first paper source

**Status:** Phases 2-9 DELIVERED 2026-05-19 on `feat/papyrus-reed`. Phase 10 = Axolittle playtest gate (blocked on his time).
**Branch:** `feat/papyrus-reed` off `main`.
**Trigger:** Foundation **A** of the Build Schematics economy (`docs/vision/build-schematics-long-run.md`). Spec 24 (Build Schematics Core) needs **paper** for the Plan Tile recipe; this spec ships the first paper source (Papyrus Sheet) plus the T1 plant that produces it. Sequenced ahead of every other Build Schematics foundation per the vision doc §10 ordering rule.

---

## TL;DR

Papyrus Reed is a 4-stage water-adjacent plant — drop a `PapyrusReed` material on a sand / dirt / grass tile with water within one block and it sprouts a `PAPYRUS_STAGE_0` block; it advances through three further stages on the standard crop-growth cadence; mining a mature `PAPYRUS_STAGE_3` block drops 1–2 reeds and *replants itself* (sugarcane-family pattern — the root keeps growing). Three reeds laid horizontally at a crafting table produce three **Papyrus Sheets** — the first **paper** material in the engine. The whole loop lifts cross-game (every Decented voxel game eventually wants a paper supply chain).

**One block family, one material family, one recipe, one new craft-slot predicate**:

| New piece | Where it lives | Value |
|---|---|---|
| `MaterialId::PapyrusReed` | `item.rs` | Plant material — places as PAPYRUS_STAGE_0 on a water-adjacent sand/dirt/grass tile; harvested 1-2 at a time from mature blocks |
| `MaterialId::PapyrusSheet` | `item.rs` | The crafted paper — Plan Tile recipe input (Spec 24) + future books / maps / scrolls / charter clauses |
| `PAPYRUS_STAGE_0..3` | `block.rs` (ids 52-55) | Four-stage growth ladder mirroring wheat. Stage 3 is mature + harvestable |
| `is_paperish_slot(slot)` | `crafting.rs` | Recipe predicate covering every paper-tier (just PapyrusSheet for now; Pulp Paper joins post-T1.5). Mirrors `is_logish_slot` |
| Placement rule | `papyrus.rs` (new) | Block-below must be sand / dirt / grass; **water within 1 block** horizontally OR directly below |

Reeds don't have a "tilled" prerequisite (papyrus grows wild — that's the design choice for an earliest-tier plant). The water-adjacency rule is the entire placement gate.

### Growth + harvest

| Stage | Visual | Behaviour |
|---|---|---|
| `PAPYRUS_STAGE_0` | small sprout, mostly soil-coloured | Just planted; advances via `growth::advance_crops` |
| `PAPYRUS_STAGE_1` | short blade emerging | … |
| `PAPYRUS_STAGE_2` | mid-height stalk | … |
| `PAPYRUS_STAGE_3` | tall papyrus head, harvestable | Mining drops 1-2 `PapyrusReed` materials; **replaces with `PAPYRUS_STAGE_0`** so the planted root keeps growing |

Growth uses the existing `crop_growth_ticks_per_stage = 200` (10 s per stage @ 20 TPS, 40 s sprout → mature) with the same water-radius-4 ×2 boost. Papyrus is *always* water-adjacent at placement time so the boost almost always fires — sprout → mature is ~20 s in practice. Realistic-feeling pace for the alpha's first walk-around-and-harvest loop.

### Recipe

```
[reed][reed][reed]   →   3× PapyrusSheet
```

Mirrors Minecraft's sugarcane → paper. Three sheets per craft keeps Plan Tile pricing reasonable (1 stick + 1 sheet → 4 tiles per Spec 24).

### Save compat + protocol

- `MaterialId` enum appended (bincode-stable variant indices preserved).
- 4 new block ids — `PAPYRUS_STAGE_0..3` = 52..=55. Old saves load fine (the registry sees them as unknown until the engine version with the registry entries is loaded — but old saves can't have them in chunks anyway).
- `PROTOCOL_VERSION 15 → 16` so a stale client connecting to a v16 server with papyrus blocks in chunks gets a clean reject instead of silent AIR fallback.

Scope: ~400 LOC including tests. 10 phases. Independent of every other foundation spec (no shared files with Specs 1, 2, 12, 19, 20, 21).

---

## Why this lives here

- **Unblocks Spec 24 Build Schematics Core.** Plan Tile recipe is `1 stick + 1 paper → 4 tiles`. Without a paper material in the engine, Spec 24 can't ship. Papyrus is the smallest possible paper-supply primitive — one plant, one recipe, no workstation needed.
- **Day-1 paper for everyone.** Players don't need iron, mills, or animals to make their first sheet — find a river, harvest reeds, lay them at a crafting table. Matches the kid-friendly "first day, you can start a build journal" intuition.
- **Reusable cross-systems.** Books (future), maps (future), quest scrolls (future Spec 19 v2), charter clauses (future Spec 10 mechanism B+) all need paper. PapyrusSheet is the alpha tier; Pulp Paper (T1.5 — sugarcane bagasse) layers on top via the same `is_paperish_slot` predicate. The vision doc §3.4 makes this an explicit two-tier paper-tech tree.
- **Cross-game lift, per shared infra strategy.** Water-adjacent plant placement gate, sugarcane-family auto-regrow harvest pattern, `is_paperish_slot` recipe predicate, paper-as-material primitive — all engine-generic. another game's recipe stations want paper-as-recipe-card; another game's "trade notes" want paper-as-token. Game-specific data (the reed plant identity, biome distribution) lives in tables.
- **Tonally honest.** Reeds growing in a riverbank are universal — every kid who's seen a river has seen reeds. Papyrus specifically anchors the "paper has a history" beat that the vision doc §3.4 calls out.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/block.rs`
  - 4 new block IDs `PAPYRUS_STAGE_0..3 = 52..=55` (after `DRYING_RACK = 51`).
  - 4 new texture-layer indices `TEX_PAPYRUS_STAGE_0..3 = 160..=163` (after `TEX_DRYING_RACK_SIDE = 159`).
  - 2 new item-drop texture layers `TEX_ITEM_PAPYRUS_REED = 164` + `TEX_ITEM_PAPYRUS_SHEET = 165`.
  - `BlockRegistry::new()` — push 4 `BlockDef` entries. All four are `solid: false, transparent: true, gravity: false` matching every other crop stage (walk-through reeds — you can wade between them as you harvest).
  - Per-stage colour: green-cast getting more golden-papyrus as it matures.
  - `mine_drop_with_seed(PAPYRUS_STAGE_3, seed)` — handled in `growth::crop_break` extension rather than here (that's the per-position-seeded path mature crops already use).
- `game/engine/src/item.rs`
  - Append `MaterialId::PapyrusReed`, `MaterialId::PapyrusSheet` (in that order, after `KilnDriedLog`). Bincode-positional — old saves load fine.
  - Display names + colours in `Item::name` + `Item::color`.
  - Hotbar/UI palette: PapyrusReed = green-tan (matches stage-2 stalk); PapyrusSheet = parchment-cream.
- `game/engine/src/growth.rs`
  - Extend `next_stage`, `is_crop`, `crop_break` matches to cover the 4 new stage IDs.
  - `crop_break(PAPYRUS_STAGE_3, seed)` returns `replacement: PAPYRUS_STAGE_0` (sugarcane-style auto-regrow) + drops 1-2 `PapyrusReed` materials. This is the **only** crop family in the engine whose break-replacement is itself rather than `TILLED_SOIL` or `AIR` — see §"Why sugarcane-style regrowth" below.
- New module `game/engine/src/papyrus.rs` (~80 LOC)
  - `is_water_adjacent(world, x, y, z) -> bool` — checks 4 horizontal neighbours of `(x, y, z)` plus the cell directly below for `block::WATER`.
  - `is_valid_planting_base(world, x, y, z) -> bool` — block at `(x, y, z)` is one of `DIRT`/`GRASS`/`SAND` AND `world.get_block(x, y+1, z) == AIR` AND `is_water_adjacent(world, x, y, z)`.
  - Pure free functions. Tests cover each rule independently + the composite predicate.
- `game/engine/src/game_loop.rs`
  - Right-click-place handler: extend the existing `Item::Material(...)` branch to dispatch on PapyrusReed — `papyrus::is_valid_planting_base` gate, consume 1 material, place `PAPYRUS_STAGE_0` at `(target_x, target_y+1, target_z)`. Mirror the existing place-block path.
  - Mining a PAPYRUS_STAGE_3 already routes through `crop_break` once the new match arm lands.
- `game/engine/src/crafting.rs`
  - New helper `is_paperish_slot(slot: CraftSlot) -> bool` — mirrors `is_logish_slot`. Returns true for any paper-tier material. v1 covers just `MaterialId::PapyrusSheet`; T1.5's Pulp Paper joins later.
  - New recipe arm: 3 reeds horizontal (any single row) → 3 PapyrusSheet. Mirrors the bread recipe (3 wheat horizontal).
- `game/engine/src/commands/builtins/give.rs`
  - Aliases: `papyrus_reed | papyrus`, `papyrus_sheet | papyrus_paper | paper`, plus block aliases `papyrus_stage_0 | papyrus_stage_1 | papyrus_stage_2 | papyrus_stage_3` for tests.
- `game/engine/src/protocol.rs`
  - `PROTOCOL_VERSION 15 → 16`. Version-history entry below.
- `game/engine/src/texture_gen.rs`
  - 6 new texture layers — 4 stages + 2 item-drop icons. Procedural — green-yellow palette, sugarcane-style vertical stalk for the block, flat horizontal reed for the item icon, parchment grid pattern for the sheet.
- `game/engine/src/entity_model.rs`
  - Material-icon entries for the 2 new item drops.

### New module

- `game/engine/src/papyrus.rs` — ~80 LOC. Three pure functions:
  - `is_water_adjacent(world, x, y, z) -> bool`
  - `is_valid_planting_base(world, x, y, z) -> bool`
  - `try_plant_papyrus(world, x, y, z) -> bool` — consolidated convenience for the place-handler.
  - Plus `#[cfg(test)]` coverage for each predicate.

### Related specs

- `docs/vision/build-schematics-long-run.md` §3.4 (Paper — historical tech tree). This spec is the T1 row of that table.
- `docs/foundations/2026-05-19-build-schematics-core.md` (Spec 24, written next on `feat/build-schematics-core`). Consumes `MaterialId::PapyrusSheet` via `is_paperish_slot` in the Plan Tile recipe.
- `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md` (Spec 12 — T1.5 deferred). Will add Pulp Paper as a second `is_paperish_slot` match — zero-touch from this spec's PoV.
- `docs/spec/05-gameplay-systems.md §3.9` Farming. Touch lightly — add a "Papyrus Reed" sub-bullet under the crops list noting the water-adjacency rule + cross-reference Spec 24 for the paper-tech-tree context.

### Memory pointers

- uk english naming — "papyrus reed" + "papyrus sheet" (both noun phrases, lower-case in prose). "Reed" not "rush" or "cane". No transatlantic divergences.
- pretest check — implementer should grep current state before starting: block IDs in use (50, 51 taken), PROTOCOL_VERSION (15), texture layers (last in use = 159), MaterialId tail (KilnDriedLog). Confirmed at spec-write time 2026-05-19.
- autonomy to playtest boundary — Phases 2-9 autonomous; Phase 10 = playtest gate.
- shared infra strategy — the water-adjacency placement gate + `is_paperish_slot` predicate are engine-generic; reed-as-AxeNStax-papyrus is game-specific data.
- axenstax has farming — coherent with the farming arc. Papyrus is the kid-friendly first-paper plant; pulp paper from sugarcane bagasse lands in T1.5.

### What does NOT exist yet (and papyrus does NOT need)

- **Tilled-soil for non-crop plants.** Sugarcane / reeds grow on plain dirt/grass/sand without tilling — kid-intuitive and matches Minecraft.
- **Multi-block-tall plants.** Each stage is a single block. Visual height comes from per-stage textures. Vertical-stack mechanics (Spec 11 v2-ish — multi-block sugarcane stacks) are out of scope; one-block-tall reeds are sufficient for the v1 paper supply chain.
- **Biome-restricted spawning.** Spec 11 v2 may seed reeds along river spawns at world-gen; this spec ships the plant + the harvest loop. World-gen integration is a follow-on.
- **Bonemeal interaction.** Wheat / carrot / potato bonemeal land in their own polish pass; papyrus mirrors whatever shape they end up with. Out of scope here.
- **Pulp Paper.** T1.5 (Spec 12) ships sugarcane → bagasse → pulp paper. This spec defines `is_paperish_slot` so T1.5's addition is a one-arm extension.

---

## Why sugarcane-style regrowth (and not wheat-style replant-after-harvest)

Two patterns were on the table:

1. **Wheat-style** — mature crop breaks to `TILLED_SOIL` + drops material + seeds. Player must collect seeds + replant. (`crop_break(WHEAT_STAGE_3) → replacement: TILLED_SOIL`.)
2. **Sugarcane-style** — mature crop breaks to STAGE_0 + drops material. The root keeps growing. No "seed" item.

Papyrus picks **sugarcane-style** because:

- **No "seed" friction** — papyrus reed material *is* the seed; eating the seed/material distinction adds nothing for an earliest-tier plant.
- **River-walk harvest pattern** — a kid who finds a river of papyrus can walk along it, whacking each mature block in turn; they each regrow on their own. No "after-harvest, click each one to replant" chore.
- **Matches Minecraft sugarcane intuition** — anyone who's played Minecraft expects this mechanic for water-edge plants.
- **First-planting friction is the meaningful one** — finding a water-adjacent tile + harvesting your first reed feels like the discovery moment. After that, the loop should be loose.

Wheat / carrot / potato remain wheat-style — they're tilled-soil crops where the tilling is the work investment. Papyrus is wild — no work to maintain.

---

## Scope

| # | Phase | Files | Est. LOC | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-19-papyrus-reed.md` | ~450 | ✓ |
| 2 | MaterialIds + names + colours + four PAPYRUS_STAGE blocks + textures + registry | `item.rs`, `block.rs`, `texture_gen.rs`, tests | ~150 | ✓ |
| 3 | `papyrus.rs` placement predicates + right-click placement handler | new `papyrus.rs`, `game_loop.rs`, tests | ~100 | ✓ |
| 4 | Growth-tick integration | `growth.rs` (extend `next_stage` + `is_crop`), tests | ~40 | ✓ |
| 5 | Harvest — `crop_break` extension (mature stage drops 1-2 reeds + replaces with stage 0) | `growth.rs`, tests | ~40 | ✓ |
| 6 | Crafting recipe — `is_paperish_slot` helper + 3 reeds horizontal → 3 sheets | `crafting.rs`, tests | ~60 | ✓ |
| 7 | `/give` aliases for materials + block stages | `commands/builtins/give.rs`, tests | ~30 | ✓ |
| 8 | `PROTOCOL_VERSION` bump 15 → 16 + history note | `protocol.rs`, tests | ~10 | ✓ |
| 9 | Spec 5 §3.9 touch + README flip (Spec 23 status to DELIVERED) + economies/vision cross-refs | `docs/spec/05-gameplay-systems.md`, `docs/foundations/README.md` | ~30 | ✓ |
| 10 | Axolittle playtest — water-adjacency intuition, harvest pacing, recipe yield feel, paper-as-foundational-material moment | n/a | 0 | ✗ blocked |

**Total**: ~450 LOC spec + code + tests. Phases 2-9 autonomous. Phase 10 is the playtest gate.

Recommended order: 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9. Phase 3 depends on 2's materials existing. Phase 4 depends on 2's block IDs being registered. Phase 5 depends on 4 (growth must reach STAGE_3 before crop_break has anything to do). Phase 6 depends on 2's PapyrusSheet material existing. Phase 7 is independent once 2 is done. Phase 8 is the final wire bump. Phase 9 is docs-only.

---

## Phase 1 — This spec

You're reading it. ✓

---

## Phase 2 — Materials + blocks + textures + registry

### Goal

Two new `MaterialId` variants and four new block IDs exist with display names, colours, textures, and registry entries. No gameplay yet — pure data wiring + a single positive registry test per piece.

### Changes

- `item.rs`:
  - Append to `MaterialId` enum, *after* `KilnDriedLog`:
    ```rust
    // Papyrus Reed (Spec 23, 2026-05-19). Water-adjacent T1 plant +
    // first paper source. Reed is the plantable / harvested material;
    // Sheet is the crafted paper (3 reeds → 3 sheets). Appended after
    // the Wave 29 log-seasoning materials to preserve bincode variant
    // indices. See `docs/foundations/2026-05-19-papyrus-reed.md`.
    PapyrusReed,
    PapyrusSheet,
    ```
  - `Item::name` arms:
    - `PapyrusReed → "Papyrus Reed"`
    - `PapyrusSheet → "Papyrus Sheet"`
  - `Item::color` arms:
    - `PapyrusReed → [0.55, 0.68, 0.32]` (green-tan riverbank stalk)
    - `PapyrusSheet → [0.92, 0.86, 0.62]` (parchment cream)
- `block.rs`:
  - Append constants after `DRYING_RACK = 51`:
    ```rust
    // Papyrus Reed crop stages (Spec 23 — Foundation A of Build
    // Schematics). 4-stage growth ladder mirroring wheat. Placement
    // requires water within 1 block; harvest regrows in place
    // (sugarcane-style). All four are transparent + non-solid so the
    // player can wade through riverbank reeds while harvesting.
    pub const PAPYRUS_STAGE_0: BlockId = 52;
    pub const PAPYRUS_STAGE_1: BlockId = 53;
    pub const PAPYRUS_STAGE_2: BlockId = 54;
    pub const PAPYRUS_STAGE_3: BlockId = 55;
    ```
  - Append texture layer indices after `TEX_DRYING_RACK_SIDE = 159`:
    ```rust
    // Papyrus stage textures (Wave 30 — layers 160-163) + item drop
    // textures (164-165).
    pub const TEX_PAPYRUS_STAGE_0: u32 = 160;
    pub const TEX_PAPYRUS_STAGE_1: u32 = 161;
    pub const TEX_PAPYRUS_STAGE_2: u32 = 162;
    pub const TEX_PAPYRUS_STAGE_3: u32 = 163;
    pub const TEX_ITEM_PAPYRUS_REED: u32 = 164;
    pub const TEX_ITEM_PAPYRUS_SHEET: u32 = 165;
    ```
  - In `BlockRegistry::new()`, after the DRYING_RACK entry, push four `BlockDef`s mirroring the crop-stage loop pattern from §"31..=34 / …":
    ```rust
    let papyrus_stages = [
        ("genesis:papyrus_stage_0", TEX_PAPYRUS_STAGE_0, [0.55, 0.65, 0.30]),
        ("genesis:papyrus_stage_1", TEX_PAPYRUS_STAGE_1, [0.60, 0.70, 0.32]),
        ("genesis:papyrus_stage_2", TEX_PAPYRUS_STAGE_2, [0.68, 0.75, 0.32]),
        ("genesis:papyrus_stage_3", TEX_PAPYRUS_STAGE_3, [0.85, 0.82, 0.45]),
    ];
    for (name, tex, color) in papyrus_stages {
        blocks.push(BlockDef {
            name,
            solid: false,
            transparent: true,
            gravity: false,
            color,
            tex_top: tex, tex_bottom: tex, tex_side: tex,
        });
    }
    ```
- `texture_gen.rs`:
  - 6 new texture layers — papyrus stages 0..3 + reed item icon + sheet item icon. Procedural generation: vertical stalk getting taller per stage on a transparent background; mature stage shows the wedge-shaped papyrus crown at the top.
- `entity_model.rs`:
  - Material-texture mapping for `PapyrusReed → TEX_ITEM_PAPYRUS_REED` + `PapyrusSheet → TEX_ITEM_PAPYRUS_SHEET`.

### Tests

- `item::tests::papyrus_materials_have_names_and_unique_colours`
- `block::tests::papyrus_stage_blocks_registered` — all four IDs known to registry; correct textures; transparent/non-solid/non-gravity invariants
- `item::tests::papyrus_materials_stack_to_64`

### Acceptance

- `cargo build` clean (host + WASM via check.sh).
- All new tests pass.
- `check.sh` ALL GREEN.

### Save compat

`MaterialId` enum appended at the end — bincode indices for prior variants unchanged. 4 new block IDs appended. Old saves load fine. Protocol bump arrives in Phase 8.

---

## Phase 3 — Placement predicates + right-click handler

### Goal

Players holding `PapyrusReed` material can right-click sand / dirt / grass blocks that have water within 1 block to plant a `PAPYRUS_STAGE_0` on top of them, consuming 1 reed.

### Changes

- New `game/engine/src/papyrus.rs`:
  ```rust
  //! Papyrus Reed — water-adjacent placement predicates.
  //!
  //! Per Spec 23 / `docs/foundations/2026-05-19-papyrus-reed.md`. Three
  //! pure free functions matching the `growth.rs` precedent so the
  //! place-handler and tests share one source of truth.

  use crate::block::{self, BlockId};
  use crate::world::World;

  /// Search radius for water adjacency, in blocks. Reeds need water
  /// touching the placement tile — 1-block Chebyshev distance covers
  /// the four horizontal neighbours and the cell directly below
  /// (riverbed water seeping up).
  pub const WATER_ADJACENCY_RADIUS: i32 = 1;

  /// Is there `block::WATER` within [`WATER_ADJACENCY_RADIUS`] of
  /// `(x, y, z)`? Checks the 4 horizontal neighbours at the same Y
  /// plus the cell directly below. Tight on purpose — papyrus
  /// shouldn't sprout from a tile two squares away from a river.
  pub fn is_water_adjacent(world: &World, x: i32, y: i32, z: i32) -> bool {
      for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
          if world.get_block(x + dx, y, z + dz) == block::WATER {
              return true;
          }
      }
      world.get_block(x, y - 1, z) == block::WATER
  }

  /// Is `(x, y, z)` a valid base for planting a papyrus stem?
  /// Three conjunctive conditions:
  ///   1. Block at (x, y, z) is one of DIRT / GRASS / SAND.
  ///   2. Block directly above is AIR.
  ///   3. Water touches the base tile (see [`is_water_adjacent`]).
  pub fn is_valid_planting_base(world: &World, x: i32, y: i32, z: i32) -> bool {
      let base = world.get_block(x, y, z);
      let base_ok = matches!(base, block::DIRT | block::GRASS | block::SAND);
      base_ok
          && world.get_block(x, y + 1, z) == block::AIR
          && is_water_adjacent(world, x, y, z)
  }

  /// Attempt to plant a papyrus stem on top of `(x, y, z)`. Returns
  /// `true` if planted; `false` if any rule failed. Idempotent on
  /// failure — leaves the world untouched.
  pub fn try_plant_papyrus(world: &mut World, x: i32, y: i32, z: i32) -> bool {
      if !is_valid_planting_base(world, x, y, z) {
          return false;
      }
      world.set_block(x, y + 1, z, block::PAPYRUS_STAGE_0);
      true
  }
  ```
- `game_loop.rs` (right-click-place handler):
  - Locate the existing `Item::Material(...)` arm that handles `material_as_placeable_block` (Wave 29 log path).
  - Add a parallel arm for `PapyrusReed`: instead of routing through `material_as_placeable_block` (which would place an OAK_LOG), call `papyrus::try_plant_papyrus(&mut world, target_x, target_y, target_z)`. On `true`: consume 1 reed; play_place audio; respect the existing cooldown.
  - On `false`: fall through to the default no-op. No toast in v1 — the wireframe already tells the player they're aimed at a place they can't plant. (Add a toast in Phase 10 if Axolittle's playtest reveals confusion.)

### Tests (in `papyrus.rs`)

- `is_water_adjacent_detects_horizontal_neighbour` — water at (x+1, y, z) trips the predicate
- `is_water_adjacent_detects_below_tile` — water at (x, y-1, z) trips the predicate
- `is_water_adjacent_false_for_dry_tile` — no water in neighbourhood → false
- `is_water_adjacent_does_not_check_y_plus_1` — water above doesn't count (papyrus grows up, not down)
- `is_valid_planting_base_requires_dirt_grass_or_sand` — stone base → false; dirt → true
- `is_valid_planting_base_requires_air_above` — block above is stone → false
- `is_valid_planting_base_requires_water_nearby` — dry tile → false even with valid base + air above
- `try_plant_papyrus_writes_stage_0` — happy-path: planted block reads back as PAPYRUS_STAGE_0
- `try_plant_papyrus_idempotent_on_failure` — invalid placement leaves world unchanged

### Acceptance

- All tests green; `check.sh` clean.
- Manual: place a row of dirt next to water, hold reeds, right-click each tile → row of stage-0 reeds appears. Try the same on a dirt tile not touching water → nothing happens.

### Save compat

No format change.

---

## Phase 4 — Growth-tick integration

### Goal

Planted reeds advance through stages 0 → 1 → 2 → 3 on the standard crop-growth cadence (200 ticks per stage, halved if water-adjacent — which they always are at plant time, so effectively 100 ticks per stage).

### Changes

- `growth.rs`:
  - Extend `next_stage` match to cover the four papyrus stages:
    ```rust
    PAPYRUS_STAGE_0 => Some(PAPYRUS_STAGE_1),
    PAPYRUS_STAGE_1 => Some(PAPYRUS_STAGE_2),
    PAPYRUS_STAGE_2 => Some(PAPYRUS_STAGE_3),
    ```
    (Note: `PAPYRUS_STAGE_3 → None` is implicit via the existing `_ => None` arm.)
  - Extend `is_crop` to recognise the four new stages.
  - Update the top-of-file `use` list to include `PAPYRUS_STAGE_0..PAPYRUS_STAGE_3` from `crate::block`.
  - **No change** to `advance_crops` — it iterates over arbitrary positions and reads `next_stage` + `water_within_radius`. The papyrus path automatically rides the existing `water_within_radius` check (papyrus is always water-adjacent by placement rule).
- The chunk-iteration code in `collect_crop_positions` automatically picks up papyrus tiles once `is_crop` recognises them.

### Tests (in `growth.rs`)

- `next_stage_progresses_papyrus` — every stage advances; mature stays
- `is_crop_recognises_papyrus_stages`
- `papyrus_advances_when_water_adjacent` — TestHost-style: plant a papyrus near water, run `advance_crops` at the water-adjacent fast-fire tick (100), assert it advanced
- `papyrus_advances_at_base_interval_when_no_water_nearby` — synthetic case where a papyrus block sits on a tile that *was* water-adjacent at placement but the water was later removed; growth slows to the base 200-tick interval. Documents that water-adjacency is checked *at every growth tick*, not just at placement.

### Acceptance

- All tests green; `check.sh` clean.
- Manual: plant a fresh reed → walk away 2 minutes → return to find it harvestable. (Faster in single-player at 20 TPS — actually ~20 s.)

### Save compat

No format change.

---

## Phase 5 — Harvest (crop_break extension)

### Goal

Mining a mature `PAPYRUS_STAGE_3` drops 1-2 `PapyrusReed` materials and replaces the block with `PAPYRUS_STAGE_0` so the root keeps growing.

### Changes

- `growth.rs`:
  - Add to `crop_break`:
    ```rust
    PAPYRUS_STAGE_3 => Some(CropBreakResult {
        // Sugarcane-style auto-regrow — root stays planted, top
        // resets to stage 0. The kid walks along a riverbank
        // harvesting reed crowns; each tile keeps growing on its
        // own. Distinct from wheat/carrot/potato which break to
        // TILLED_SOIL (those are "field crops" that need replanting).
        // See spec §"Why sugarcane-style regrowth".
        replacement: block::PAPYRUS_STAGE_0,
        drops: vec![ItemStack::new_material(MaterialId::PapyrusReed, roll(rng_seed, 1, 2))],
    }),
    // Immature papyrus drops nothing — wasted growth. Replaces with
    // AIR (NOT stage 0 — accidentally clear-cutting an immature
    // reed deserves the lost root, otherwise there's no penalty for
    // harvesting too early).
    PAPYRUS_STAGE_0 | PAPYRUS_STAGE_1 | PAPYRUS_STAGE_2 => Some(CropBreakResult {
        replacement: block::AIR,
        drops: Vec::new(),
    }),
    ```
  - The existing `_ => None` arm covers non-papyrus.

### Tests (in `growth.rs`)

- `mature_papyrus_drops_one_or_two_reeds_and_replants_in_place` — replacement is PAPYRUS_STAGE_0; drop count in [1, 2]
- `papyrus_drop_count_is_deterministic_for_same_seed`
- `immature_papyrus_breaks_to_air_with_no_drops` — three stages × no drops × replacement AIR

### Acceptance

- All tests green; `check.sh` clean.
- Manual: plant + grow + mine a mature reed → get 1-2 reeds in inventory; the block is back at stage 0; wait a moment → it grows again.

### Save compat

No format change.

---

## Phase 6 — Crafting recipe + `is_paperish_slot` helper

### Goal

Three reeds laid horizontally at a crafting table produce 3 Papyrus Sheets. A new `is_paperish_slot` recipe predicate lets future paper tiers (Pulp Paper in T1.5) join the same recipe with zero changes.

### Changes

- `crafting.rs`:
  - Add helper near `is_logish_slot` (~line 323):
    ```rust
    /// Recipe-predicate covering every paper-tier material. Mirrors
    /// `is_logish_slot` — recipes that "want any paper" can match this
    /// without caring about the tier (Papyrus Sheet now; Pulp Paper
    /// post-T1.5; future Bamboo Paper, …).
    ///
    /// Cross-game-generic — the predicate is engine-neutral; the
    /// specific MaterialIds it recognises are AxeNStax data.
    pub fn is_paperish_slot(slot: CraftSlot) -> bool {
        matches!(
            slot,
            CraftSlot::Material(MaterialId::PapyrusSheet),
        )
    }
    ```
  - Add the reed-row recipe near the bread recipe (search for the existing 3-wheat-horizontal arm — bread recipe). The shape is identical: any single row of 3 reeds → 3 sheets, output as `ItemStack::new_material(MaterialId::PapyrusSheet, 3)`.
- The Plan Tile recipe in Spec 24 will consume `is_paperish_slot` against the input pattern `[stick][paper]`. This spec doesn't ship the Plan Tile recipe itself.

### Tests (in `crafting.rs`)

- `is_paperish_slot_covers_papyrus_sheet_only_for_now`
- `is_paperish_slot_rejects_non_paper_materials`
- `three_reeds_horizontal_makes_three_sheets` — every row position (top / middle / bottom) of the 3×3 grid produces 3 sheets
- `mixed_row_does_not_match` — 2 reeds + 1 stick row → no match

### Acceptance

- All tests green; `check.sh` clean.
- Manual: 3 reeds in a row at a crafting table → output slot shows 3 Papyrus Sheets.

### Save compat

No format change.

---

## Phase 7 — `/give` aliases

### Goal

`/give papyrus_reed`, `/give papyrus_sheet`, and the four `/give papyrus_stage_N` aliases all work. Convenience aliases `papyrus` (→ reed) and `paper` (→ sheet) for casual chat use.

### Changes

- `commands/builtins/give.rs`:
  - Add to the material match:
    ```rust
    "papyrus_reed" | "papyrus" | "reed"     => Some(Item::Material(MaterialId::PapyrusReed)),
    "papyrus_sheet" | "papyrus_paper" | "paper" => Some(Item::Material(MaterialId::PapyrusSheet)),
    ```
  - Add to the block match (for testing/debug — the stages are not normally `/give`-able, but it's useful for the engine team to debug-place a particular stage):
    ```rust
    "papyrus_stage_0" => Some(Item::Block(block::PAPYRUS_STAGE_0)),
    "papyrus_stage_1" => Some(Item::Block(block::PAPYRUS_STAGE_1)),
    "papyrus_stage_2" => Some(Item::Block(block::PAPYRUS_STAGE_2)),
    "papyrus_stage_3" => Some(Item::Block(block::PAPYRUS_STAGE_3)),
    ```

### Tests (in `commands/builtins/give.rs`)

- `give_papyrus_reed_works` + alias variants
- `give_papyrus_sheet_works` + alias variants
- `give_papyrus_stage_blocks_work` — all 4 stages individually

### Acceptance

- Tests green; `check.sh` clean.
- Manual: `/give paper 64` → 64 Papyrus Sheets in inventory.

### Save compat

No format change.

---

## Phase 8 — PROTOCOL_VERSION bump

### Goal

`PROTOCOL_VERSION` bumps from 15 to 16. Stale clients (≤v15) get a clean reject on connect rather than seeing AIR where papyrus blocks should be.

### Changes

- `protocol.rs`:
  - Bump the constant.
  - Append a version-history entry:
    ```text
    /// - v16 (2026-05-19): Spec 23 Papyrus Reed — adds 4 new block ids
    ///   (PAPYRUS_STAGE_0..3 = 52..=55), 2 new materials
    ///   (PapyrusReed / PapyrusSheet appended to MaterialId), and the
    ///   `is_paperish_slot` crafting predicate. Same registry-fallback
    ///   rationale as previous block-id bumps. See foundation
    ///   `2026-05-19-papyrus-reed.md`.
    ```
  - Update the `protocol_version_pinned` test to assert 16.

### Tests

- `protocol_version_pinned` — passes at v16.

### Acceptance

- Tests green; `check.sh` clean.

### Save compat

PROTOCOL_VERSION change forces stale-client rejection on multiplayer connect. No on-disk format change.

---

## Phase 9 — Docs touch + README flip

### Goal

`docs/spec/05-gameplay-systems.md` mentions papyrus in the farming section. `docs/foundations/README.md` flips Spec 23's status to DELIVERED with the merge commit hash. Optional: a small cross-ref in `docs/vision/build-schematics-long-run.md` confirming Foundation A is shipped.

### Changes

- `docs/spec/05-gameplay-systems.md §3.9` Farming:
  - Sub-bullet: "**Papyrus Reed** — water-adjacent plant; place on sand/dirt/grass within 1 block of water. 4 growth stages; mature drops 1-2 reeds + auto-regrows in place. 3 reeds → 3 Papyrus Sheets (first paper material). Spec 23."
- `docs/foundations/README.md`:
  - In the Specs 23-27 table, flip Spec 23's status line from "READY TO SPEC" to "DELIVERED 2026-05-19 on `feat/papyrus-reed`" with the merge commit hash filled in post-merge.
- `docs/vision/build-schematics-long-run.md` §10:
  - Update Foundation A's row to "DELIVERED" + commit hash.
  - No content change to §3.4 — the design contract is unchanged.

### Acceptance

- Spec 5 §3.9 has the papyrus sub-bullet.
- Foundations README flipped.
- Drift-audit re-read finds no contradictions.

---

## Phase 10 — Axolittle playtest (BLOCKED on his time)

### What he's evaluating

- **Water-adjacency intuition** — does the "needs water nearby" rule feel obvious from the placement failure / success pattern? Or does he need a toast saying "needs water"?
- **Harvest pacing** — 20 s sprout → mature feels right? Too fast (no real loop)? Too slow (boring)?
- **Recipe yield** — 3 reeds → 3 sheets at the crafting table. Does this feel like enough for the Spec 24 Plan Tile recipe (which then converts 1 sheet → 4 tiles)?
- **Paper-as-foundational moment** — when he crafts his first sheet, does it land as "I just made paper" or does it feel like just another material?
- **Replanting friction** — sugarcane-style auto-regrow vs wheat-style replant. Does the difference register, or does he expect wheat-style?
- **Riverbank harvest loop** — running along a river chopping reeds. Does it feel natural? Smooth movement? Or do the non-solid plant tiles cause weird collision feedback?
- **Visual progression** — can he tell stage 0 from stage 3 at a glance from a few blocks away?

### Outputs

Memory entries on his calls. Tuning patch with adjusted constants if needed (growth rate, drop count range, harvest yield, recipe yield). Spec 5 amended for any design changes.

---

## Future spec hooks (designed-for, deferred)

### Pulp Paper (T1.5, Spec 12)

- `MaterialId::PulpPaper` appended.
- `is_paperish_slot` extended with `| CraftSlot::Material(MaterialId::PulpPaper)` — one line.
- Mill workstation produces Pulp Paper from sugarcane bagasse (T1.5 supply chain).
- Plan Tile recipe + future book / map / scroll recipes all accept Pulp Paper automatically via `is_paperish_slot`.

### World-gen river-edge spawning

- Future world-gen pass seeds papyrus stalks along river edges so the player has a visible "here be reeds" hook from the start.
- This spec ships the plant + the harvest loop; world-gen integration is a follow-on (Spec 11 v2 territory).

### Books / Maps / Scrolls / Charter clauses

- All four future systems will use `is_paperish_slot` as the recipe predicate. No this-spec wiring needed.

### Reed multi-block stacks (sugarcane-tall)

- Out of scope for v1 — one-block-tall reeds suffice. Multi-block stacking (Minecraft sugarcane mechanic — break the top, the lower stays; can grow up to N tall) is polish for a later wave.

---

## Memory rule check

- ✓ signet boundary — N/A. Papyrus touches no Signet/identity.
- ✓ uk english naming — "papyrus reed", "papyrus sheet", "sugarcane-style", "riverbank" — all UK-standard. No transatlantic divergences.
- ✓ pretest check — implementer should grep current state: block IDs 52+ available (51 = DRYING_RACK is the last in use); PROTOCOL_VERSION = 15; texture layers 160+ available; MaterialId tail = KilnDriedLog. Confirmed at spec-write time 2026-05-19.
- ✓ autonomy to playtest boundary — Phases 2-9 autonomous, Phase 10 = playtest gate.
- ✓ shared infra strategy — water-adjacency placement predicate + `is_paperish_slot` recipe predicate + sugarcane-style auto-regrow are engine-generic primitives. The specific plant identity (papyrus), drop counts, growth rate are AxeNStax-specific data.
- ✓ axenstax has farming — papyrus is sequenced alongside the T1 farming work. Coherent with the survival arc; papyrus is the kid-friendly first-paper plant.
- ✓ alpha launch posture — post-alpha-priority but useful enough to ship before alpha if cycles allow. Doesn't gate alpha; unlocks Spec 24 Build Schematics Core which doesn't gate alpha either.
- ✓ proof of play is proof of work — orthogonal; no hashing in this spec.
- ✓ genesis block singular per world — orthogonal; this spec touches no Satori.

---

## Acceptance — overall

- 10 phases complete or explicitly blocked (Phase 10 = playtest).
- `./check.sh` ALL GREEN throughout.
- Two new materials (`PapyrusReed`, `PapyrusSheet`) in `MaterialId`.
- Four new block IDs (`PAPYRUS_STAGE_0..3` = 52..=55) registered + textured.
- New `papyrus.rs` module with placement predicates + tests.
- Growth + harvest wired through `growth.rs` (`next_stage` + `is_crop` + `crop_break`).
- 3-reeds-horizontal → 3-sheets crafting recipe + `is_paperish_slot` helper.
- `/give` aliases.
- `PROTOCOL_VERSION` 15 → 16 with history note.
- Spec 5 §3.9 amended; foundations README + vision doc cross-refs flipped.
- This foundation doc's status line: READY TO BUILD → DELIVERED at the top.

---

## Out of scope (explicitly)

- Pulp Paper (T1.5 — Spec 12).
- World-gen river-edge papyrus spawning (future world-gen pass).
- Multi-block-tall sugarcane-style reed stacks (future polish).
- Bonemeal interaction (mirrors whatever wheat/carrot/potato end up with).
- Biome-restricted spawning (river / wetland biome work — future world-gen spec).
- Books / maps / quest scrolls / charter clauses (all consume PapyrusSheet via `is_paperish_slot`, all are their own future specs).
- Plan Tile + Plan item (Spec 24 — Foundation B, ships next on `feat/build-schematics-core`).
- Multi-player rack-style ownership rules — single-player owns this spec end-to-end; multi-player rules deferred.

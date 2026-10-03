# Log Seasoning — three-tier wood economy + Drying Rack workstation

**Status:** READY TO BUILD as of 2026-05-19. Phases 2-8 are autonomous; Phase 9 = Axolittle playtest gate.
**Branch:** `feat/log-seasoning` off `main`.
**Trigger:** 2026-05-19 chat — Axolittle floated "season your logs for the furnace and campfires." Designed up from a one-liner into a full deep-economy lane: green wood burns poorly + smokes, seasoned wood burns clean, kiln-dried (future) burns premium. New block-entity workstation (Drying Rack) closes the loop.

---

## TL;DR

Logs aren't a single uniform fuel anymore. Fresh-cut wood is **green** — short burn, lots of smoke, mob-drawing pillar. Stack it on a **Drying Rack** for ~5 real minutes (game-day-scale) under open sky and it becomes **seasoned** — clean burn, double the burn time, no smoke beacon. A future **Kiln** workstation (post-Spec 20 Furnace) bakes seasoned logs into **kiln-dried** — premium fuel at 4× green burn.

The seasoning ladder ties directly into:
- **Campfire fuel economy** (`campfire::fuel_value` — three new entries).
- **Existing Wave 28 smoke-pillar mechanic** (green-log burn pillars; seasoned/kiln burn clean — survival progression baked in).
- **Future Furnace smelt budgets** (Spec 20 inherits the fuel-value table; no extra work this spec).
- **The "first night → I built shelter" arc** — building a drying rack is the kid's first "I made my workshop better" moment.

Scope: ~700 LOC including tests. 9 phases. Independent of Specs 1+2 (no shared files). Cross-game-generic in the block-entity + dryness-state-machine sense.

### Three log materials

| Material | Burn (s @ 20 TPS) | Smoke pillar? | Friction ignition | Source |
|---|---:|:---:|:---:|---|
| `GreenLog` | 30 (600 ticks) | **yes** (2× burn-time) | 70% (current — defer rate-by-fuel-mix to future polish) | Drops 1× from any OAK_LOG mine (tree-trunk or placed) |
| `SeasonedLog` | 60 (1200 ticks — current OAK_LOG block fuel) | no | 70% | Drying-Rack output after ~5 real min |
| `KilnDriedLog` | 120 (2400 ticks) | no | 70% | **Defined-but-unproductionable** this spec; `/give` only. Kiln workstation lands post-Spec 20. |
| `Item::Block(OAK_LOG)` (back-compat) | 60 | no | 70% | Old saves with logs-as-blocks in inventory; new `/give oak_log` for backwards-compat tests |

### Drying Rack — new block-entity workstation

| Property | Value |
|---|---|
| Block ID | `DRYING_RACK = 51` (next available after Wave 26-28 + Village Bell) |
| Recipe | 4 sticks in a 2×2 → 1 Drying Rack (cheap, day-1 buildable) |
| Capacity | 8 log slots, parallel seasoning |
| Operating gate | Block directly above must be `AIR` (open sky / under an overhang with airflow) |
| Seasoning time | 6000 ticks per slot (~5 real min at 20 TPS) |
| Right-click with GreenLog | Adds to first empty slot |
| Right-click empty hand | Withdraws first fully-seasoned slot as 1× SeasonedLog material; toasts "Not ready yet" if no slot is mature |
| Mining the rack | Drops the rack item + spills any logs as GreenLog material (seasoning progress lost — Minecraft-style "you mined a workstation" honesty) |
| Save/load | Parallel `World::drying_racks: AHashMap<(i32,i32,i32), DryingRackData>` mirroring campfire pattern; folds into Spec 20's BlockEntityData enum when that ships |

### Material-placeable extension

Small engine change so log materials act as placeable OAK_LOG block-items: the right-click-to-place handler checks if the held item is `Item::Material(GreenLog | SeasonedLog | KilnDriedLog)` and, if so, places `block::OAK_LOG` while consuming 1 of the material. This avoids forcing players through a 1:1 crafting conversion just to build with logs. Cross-game-generic via a small lookup function `item::material_as_placeable_block(m) -> Option<BlockId>`.

---

## Why this lives here

- **Closes a deep-economy lane Axolittle reached for.** "Season your logs" is the natural next half-step in a survival kid's mental model. Land it now while the fuel system is fresh (post-campfire + pre-furnace) — there's only one fuel-ladder caller to migrate.
- **Survival-progression smoke beacon.** Wave 28 already ships the smoke-pillar mechanic for leaves. Green logs riding the same primitive ties day-1 fires to a visible drawback (mob-drawing smoke + visible-from-far) and seasoning becomes a tangible upgrade. Re-uses code, deepens gameplay, costs no extra renderer work.
- **Lifts cross-game.** The dryness-state-machine pattern (raw → cured-via-time → premium-via-workstation) generalises. aging fish at the smoker, curing dried herbs — same primitive, wherever it's used. The block-entity tick + air-above-gate is engine-generic. Game-specific data (fuel values, recipe yields) lives in tables.
- **Pre-Spec-20 placement.** Spec 20 Furnace is the next major block-entity ship and introduces the `BlockEntityData` enum framework. This spec ships its drying-rack state as a parallel map (campfire pattern); when Spec 20 lands, the migration is a one-variant fold-in. Doing seasoning *before* Spec 20 keeps the furnace spec from being yet another "while we're here" omnibus.
- **Removes a tonal hand-wave.** Today every log burns the same. A kid who tries to light a fresh-from-the-tree branch IRL knows it won't burn. Seasoning makes the game tonally honest without adding a "real life" lecture — it just behaves the way intuition expects.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/block.rs`
  - `OAK_LOG` (id 7) `mine_drop` — switch from `ItemStack::new_block(OAK_LOG, 1)` to `ItemStack::new_material(MaterialId::GreenLog, 1)`. **The block ID stays valid for placement + world-gen; only the mine drop changes.**
  - Add `DRYING_RACK: BlockId = 51` + `BlockDef` (solid, non-transparent, no gravity, custom textures).
  - Add `TEX_DRYING_RACK_TOP` + `TEX_DRYING_RACK_SIDE` to the texture atlas (next layer after Wave 28's corn textures).
- `game/engine/src/item.rs`
  - Append `MaterialId::GreenLog`, `MaterialId::SeasonedLog`, `MaterialId::KilnDriedLog` (in that order, after `BakedCarrot` — preserve bincode indices).
  - Display names + colour tints in `Item::name` and `Item::color`.
  - New helper `material_as_placeable_block(m: MaterialId) -> Option<BlockId>` — returns `Some(block::OAK_LOG)` for all three log materials; `None` for everything else.
- `game/engine/src/campfire.rs`
  - `fuel_value` — add three new arms (GreenLog 600, SeasonedLog 1200, KilnDriedLog 2400 ticks). Keep the existing `block::OAK_LOG` arm at 1200 for back-compat.
  - Add `is_smoky_fuel(material, block) -> bool` returning true ONLY for GreenLog and OAK_LEAVES. Caller (game_loop right-click branch) keys off this.
  - New constant `SMOKE_TICKS_PER_GREEN_LOG = 1200` (2× the 600-tick burn so smoke outlives the burn — matches the leaves precedent of smoke-outlasts-burn).
- `game/engine/src/game_loop.rs`
  - Right-click-fuel-add branch at `game_loop.rs:2196-2228` — extend the `if fuel_block == Some(block::OAK_LEAVES)` smoke bump to use `is_smoky_fuel` and pick `SMOKE_TICKS_PER_LEAF` vs `SMOKE_TICKS_PER_GREEN_LOG` based on material/block.
  - Right-click-place-block branch — handle `Item::Material(GreenLog | SeasonedLog | KilnDriedLog)` as a placement of `block::OAK_LOG`. Search for the existing place-block code path to thread the material-as-block hook (the spec implementer should grep for `Item::Block` near placement; the analogous handler for materials lands right before the no-op fall-through).
  - Drying-Rack right-click branch — new branch *before* the campfire branch (so a player holding a green log right-clicking a drying rack adds to the rack, not the (nonexistent) campfire underneath). Mirrors the campfire branch shape.
  - New `tick_drying_racks` per-tick call placed alongside the existing `crate::campfire::tick_one` sweep in `game_loop.rs`'s tick (search for `block_entities.iter_mut`).
- `game/engine/src/crafting.rs`
  - Plank recipe (`crafting.rs:334-336`) — accept any of `block::OAK_LOG`, `MaterialId::GreenLog`, `MaterialId::SeasonedLog`, `MaterialId::KilnDriedLog`. Uniform yield: 4 planks.
  - Drying Rack recipe — 4 sticks in 2×2 → 1 DryingRack block.
- `game/engine/src/world.rs`
  - New field `pub drying_racks: AHashMap<(i32,i32,i32), DryingRackData>`. Initialised empty.
- `game/engine/src/save.rs`
  - `WorldSave` gains `#[serde(default)] pub drying_racks: Vec<SavedDryingRack>` for round-trip + back-compat with pre-Wave-29 saves.
- `game/engine/src/protocol.rs`
  - `PROTOCOL_VERSION 14 → 15`. Version-history entry: `v15 (2026-05-19): Wave 29 — log seasoning. Three new log materials (Green/Seasoned/KilnDried); DRYING_RACK block (id 51); per-rack state in world.drying_racks; OAK_LOG mine-drop changed from block to GreenLog material.`
- `game/engine/src/texture_gen.rs`
  - 2 new texture layers (drying rack top + side).
- `game/engine/src/commands/builtins/give.rs`
  - Aliases for `green_log`, `seasoned_log`, `kiln_dried_log`, `drying_rack`.
- `game/engine/src/entity_model.rs`
  - Material textures for the three new log items (small log-section icons with colour tint per stage).
- `game/engine/src/audio.rs`
  - Reuse `play_place` / `play_pickup` for rack interactions — no new sounds.

### New module

- `game/engine/src/drying_rack.rs` — new file, ~250 LOC. Mirrors `campfire.rs`'s shape:
  - `DryingRackData` struct (8 slots × `(seasoning_ticks, source_kind)`).
  - `RackSlot` newtype if helpful.
  - `tick_one(data, world, x, y, z) -> RackTickOutcome` — advances every non-empty slot's seasoning counter by 1 if the air-above gate passes. Returns a list of slot indices that just matured (UI / audio hooks can read these later).
  - Pure-function tests for the air-gate, the per-slot advancement, the maturity threshold.
- Plus the new entries in `block.rs`, `item.rs`, `crafting.rs`, etc — all touch existing files, not new ones.

### Related specs

- `docs/foundations/2026-05-18-campfire.md` (Spec 17) — fuel-value table being extended; smoke-tick mechanism being reused.
- `docs/foundations/2026-05-18-campfire-extensions.md` (Spec 18) — smoke-pillar mechanic that green-log burns plug into.
- `docs/foundations/2026-05-18-furnace.md` (Spec 20) — future workstation that will inherit the new fuel-value table; **block-entity enum refactor** that this spec's parallel `drying_racks` map will fold into.
- `docs/spec/05-gameplay-systems.md §3.10` (Campfire) + `§5.5` (Tool-Specific Actions) — update to reflect seasoning's fuel-quality ladder; new `§3.11 Drying Rack` subsection.
- `docs/spec/05-gameplay-systems.md §6.2.1` (Food Values and Cooking Risk) — note that seasoned/kiln-dried fuels cook faster *will be* a future polish; not enabled in this spec.

### Memory pointers

- signet boundary — N/A; seasoning touches no Signet/identity.
- uk english naming — "Seasoning" is UK-standard; "Drying Rack" not "Dryer"; "Kiln" not "Furnace" (those are different workstations).
- autonomy to playtest boundary — Phases 2-8 autonomous, Phase 9 = playtest gate.
- shared infra strategy — block-entity tick + dryness state-machine + air-above operating gate are engine-generic primitives. Material data tables (burn times, smoke flags) are game-specific.
- axenstax has farming — coherent with the farming + cooking + survival arc.
- pretest check — implementer should grep current state before starting (block IDs 50+; PROTOCOL_VERSION 14; current `OAK_LOG.mine_drop` flow).

### What does NOT exist yet (and seasoning does NOT need)

- **Generic block-entity framework.** Spec 20 introduces it; this spec ships a campfire-style parallel map. One-variant fold-in later.
- **Per-stack metadata on materials.** Sidestepped by having three separate `MaterialId` variants; stacking rules unchanged.
- **Rain.** No weather system yet; air-above operating gate is enough. Future weather spec might add "rain-protection canopy required" for a small additional design beat.
- **Furnace.** Spec 20 will inherit the fuel-value table; this spec doesn't touch furnaces.
- **Tree species variation.** Oak only for alpha (every world-gen log is OAK_LOG today). Species-specific seasoning curves land if/when birch/spruce/Satori-adjacent exotics land.
- **Friction-rate-by-fuel-mix.** Future polish. The 70% baseline stays for this spec — even green-log primary fires light at 70%, the player just notices the burn is shorter and smokier.
- **Kiln workstation.** Defined as the future production path for KilnDriedLog; lands post-Spec 20. This spec ships the material + `/give` so kiln-dried fuel is testable.

---

## Scope

| # | Phase | Files | Est. LOC | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-19-log-seasoning.md` | ~700 | ✓ |
| 2 | Three log materials + tree-fell drop change + tooltip names/colours | `item.rs`, `block.rs` (mine_drop), `entity_model.rs` (material textures), tests | ~120 | ✓ |
| 3 | Fuel ladder + smoke economy + planks-from-any-log | `campfire.rs` (fuel_value + is_smoky_fuel + SMOKE_TICKS_PER_GREEN_LOG), `crafting.rs` (plank recipe accepts any-log), `game_loop.rs` (smoke-add wiring), tests | ~150 | ✓ |
| 4 | Material-placeable extension | `item.rs` (`material_as_placeable_block`), `game_loop.rs` (place-block hook), tests | ~80 | ✓ |
| 5 | DRYING_RACK block + DryingRackData + per-tick advancement | `block.rs` (BlockDef + texture), `texture_gen.rs`, new `drying_rack.rs`, `world.rs` (field), `game_loop.rs` (tick wiring), tests | ~250 | ✓ |
| 6 | DryingRack right-click interactions + recipe | `game_loop.rs` (right-click branch), `crafting.rs` (4-sticks recipe), tests | ~100 | ✓ |
| 7 | Save/load + `/give` aliases + protocol bump + spill-on-mine | `save.rs` (SavedDryingRack), `commands/builtins/give.rs`, `block.rs` (DRYING_RACK mine_drop spills slots), `protocol.rs`, tests | ~100 | ✓ |
| 8 | Spec 5 update — new §3.11 Drying Rack + §3.10 Campfire fuel-ladder amendment + §5.5 hint about seasoning | `docs/spec/05-gameplay-systems.md` | ~80 | ✓ |
| 9 | Axolittle playtest — first-night feel, drying-rack-build moment, seasoning-pacing, smoke-pillar drawback intuition, kiln-dried curiosity | n/a | 0 | ✗ blocked |

**Total**: ~700 LOC including tests. Phases 2-8 autonomous. Phase 9 is the playtest gate.

Recommended build order: 2 → 3 → 4 → 5 → 6 → 7 → 8. Phase 3 depends on Phase 2's materials. Phase 4 (material-placeable) depends on Phase 2's materials existing. Phase 5 (rack) depends on Phase 4 (otherwise a fresh-from-rack seasoned-log can't be re-placed as a building log without going through the table). Phase 6 (interactions) depends on Phase 5 (state must exist before clicks can change it). Phase 7 (persistence) depends on Phase 6 (we save state that has gameplay-meaning). Phase 8 is docs-only.

---

## Phase 1 — This spec

You're reading it. ✓

---

## Phase 2 — Three log materials + tree-fell drop change

### Goal

The three log materials exist in the item registry with names/colours; mining an OAK_LOG block (any source) drops a GreenLog material. Existing OAK_LOG block-items in old saves stay as OAK_LOG block-items (back-compat).

### Changes

- `item.rs`:
  - Append to `MaterialId` enum:
    ```rust
    // Log Seasoning (Wave 29, 2026-05-19). Three-tier wood economy.
    // GreenLog: drops from any OAK_LOG mine, burns 30 s + emits smoke
    // pillar. SeasonedLog: drying-rack output, burns 60 s clean.
    // KilnDriedLog: defined for future Kiln workstation (post-Spec 20);
    // /give only this spec.
    GreenLog,
    SeasonedLog,
    KilnDriedLog,
    ```
  - `Item::name` arms:
    - `GreenLog` → `"Green Log"`
    - `SeasonedLog` → `"Seasoned Log"`
    - `KilnDriedLog` → `"Kiln-Dried Log"`
  - `Item::color` arms:
    - `GreenLog` → `[0.45, 0.52, 0.22]` (greenish-brown, conveys "moss / fresh-cut")
    - `SeasonedLog` → `[0.55, 0.38, 0.18]` (warm amber-brown — current oak feel)
    - `KilnDriedLog` → `[0.38, 0.26, 0.14]` (darker, slightly charred — premium)
- `entity_model.rs`:
  - Add material-texture entries for the three new logs (use the existing `TEX_OAK_LOG_SIDE` as a base + colour-tint at render time, OR add three new texture layers if the colour-tint path is too coarse). First cut: re-use `TEX_OAK_LOG_SIDE` and rely on the per-material `Item::color` tint for visual differentiation.
- `block.rs`:
  - `BlockRegistry::mine_drop` for `OAK_LOG` — change to `ItemStack::new_material(MaterialId::GreenLog, 1)`.
  - **The `OAK_LOG` block itself stays placeable.** Mining changes the drop only.
  - Old saves with `Item::Block(OAK_LOG)` stacks in inventory keep working — those stacks are still placeable + still fuel (60 s); no migration needed.

### Tests

- `item::tests::three_new_log_materials_have_names_and_colours`
- `block::tests::oak_log_mine_drops_green_log_material`
- `block::tests::existing_oak_log_block_item_remains_placeable` — sanity check the back-compat path

### Acceptance

- `cargo test` green. `check.sh` clean.
- Manual: spawn into a fresh world, chop a tree — inventory shows "Green Log" stack instead of "Oak Log" block.
- Old save loads cleanly; OAK_LOG block-items already in inventory render correctly.

### Save compat

`MaterialId` enum appended at the end — bincode indices for prior variants unchanged. Old saves load fine. New saves include the new variants where they exist.

---

## Phase 3 — Fuel ladder + smoke economy + planks-from-any-log

### Goal

The campfire `fuel_value` table covers the three new materials. Burning a green log spawns a smoke pillar (re-using Wave 28's mechanic). The plank recipe accepts any log type for uniform 4-plank yield, so structural building isn't gated on seasoning.

### Changes

- `campfire.rs`:
  - Extend `fuel_value`:
    ```rust
    if let Some(m) = material {
        return match m {
            MaterialId::Stick => Some(2 * 20),    // 2 s
            MaterialId::Coal => Some(240 * 20),   // 240 s
            MaterialId::GreenLog => Some(30 * 20),     // 30 s — Wave 29
            MaterialId::SeasonedLog => Some(60 * 20),  // 60 s — Wave 29
            MaterialId::KilnDriedLog => Some(120 * 20),// 120 s — Wave 29
            _ => None,
        };
    }
    ```
  - New helper:
    ```rust
    /// Whether burning this fuel emits a smoke pillar (Wave 28 mechanic).
    /// Green logs + oak leaves are the two smoky fuels; everything else
    /// burns clean. Used by the right-click-fuel-add branch to bump the
    /// campfire's smoke_ticks counter.
    pub fn is_smoky_fuel(material: Option<MaterialId>, block: Option<BlockId>) -> bool {
        matches!(block, Some(block::OAK_LEAVES))
            || matches!(material, Some(MaterialId::GreenLog))
    }

    /// Smoke ticks added per GreenLog burn. 1200 ticks = 60 s, 2× the
    /// log's 30 s burn — smoke outlives the flame so the signal is
    /// readable mid-burn AND for a beat after the log's gone, matching
    /// the leaves precedent.
    pub const SMOKE_TICKS_PER_GREEN_LOG: u32 = 1200;
    ```
- `game_loop.rs` (fuel-add right-click branch at `game_loop.rs:2196-2228`):
  - Replace the `if fuel_block == Some(block::OAK_LEAVES)` smoke bump with `if crate::campfire::is_smoky_fuel(fuel_material, fuel_block)`.
  - When the smoky-fuel branch fires, pick the smoke-tick value:
    ```rust
    let smoke_bump = match (fuel_material, fuel_block) {
        (_, Some(block::OAK_LEAVES)) => crate::campfire::SMOKE_TICKS_PER_LEAF,
        (Some(crate::item::MaterialId::GreenLog), _) => crate::campfire::SMOKE_TICKS_PER_GREEN_LOG,
        _ => 0,
    };
    cf.smoke_ticks = cf.smoke_ticks.saturating_add(smoke_bump);
    ```
- `crafting.rs` (plank recipe near `crafting.rs:334`):
  - Currently the recipe is `slot == CraftSlot::Block(block::OAK_LOG) → 4 planks`.
  - Generalise: a small helper `is_logish(slot: CraftSlot) -> bool` returns true for any of `Block(OAK_LOG)`, `Material(GreenLog)`, `Material(SeasonedLog)`, `Material(KilnDriedLog)`.
  - Plank recipe matches `is_logish(slot)` and returns `ItemStack::new_block(OAK_PLANKS, 4)`.
- Note: green logs still produce 4 planks. Structural plank-builds aren't gated on seasoning — only fuel quality + smoke beacon is. This is deliberate.

### Tests

- `campfire::tests::fuel_value_for_three_log_materials`
- `campfire::tests::is_smoky_fuel_green_log_and_leaves_only`
- `crafting::tests::plank_recipe_accepts_any_log_variant` — four input variants, uniform 4-plank output
- `campfire::tests::burning_green_log_bumps_smoke_ticks` — integration-style: simulate the right-click fuel-add path
- `campfire::tests::burning_seasoned_log_does_not_bump_smoke_ticks`

### Acceptance

- All tests pass; `check.sh` clean.
- Manual: place a campfire, add a green log → smoke pillar appears. Add a seasoned log instead → no pillar (fire just burns longer).
- Crafting table: 1 green log → 4 planks; 1 seasoned log → 4 planks; 1 kiln-dried log → 4 planks.

### Save compat

No format change in this phase — only behaviour.

---

## Phase 4 — Material-placeable extension

### Goal

Log materials act as placeable OAK_LOG blocks via right-click. Players who chop a tree can immediately right-click the ground with a green log to place an oak-log block (consuming 1 material). This avoids a degenerate "must convert via crafting table to build with logs" UX.

### Changes

- `item.rs`:
  ```rust
  /// If this material can be placed as a block (e.g. log materials place
  /// as OAK_LOG), return the block id to place. Returns None for
  /// non-placeable materials (sticks, coal, food, etc).
  ///
  /// Cross-game-generic primitive: any voxel game with "this material
  /// represents a placeable block variant" semantics uses the same hook.
  /// AxeNStax-specific data lives in the match below.
  pub fn material_as_placeable_block(m: MaterialId) -> Option<BlockId> {
      match m {
          MaterialId::GreenLog | MaterialId::SeasonedLog | MaterialId::KilnDriedLog => {
              Some(crate::block::OAK_LOG)
          }
          _ => None,
      }
  }
  ```
- `game_loop.rs` (right-click-to-place handler):
  - Grep the file for the existing place-block path (where it checks `Item::Block(b)` from the held stack). Add a parallel arm: if held is `Item::Material(m)` AND `material_as_placeable_block(m).is_some()`, place that block id and consume 1 material from the hotbar slot.
  - Cooldown + audio mirror the existing block-place path.

### Tests

- `item::tests::material_as_placeable_block_covers_three_logs`
- `item::tests::material_as_placeable_block_returns_none_for_non_log`
- `block_interact::tests::right_click_green_log_places_oak_log_block` — TestHost-driven integration test
- `block_interact::tests::right_click_consumes_one_material_per_place`

### Acceptance

- All tests pass; `check.sh` clean.
- Manual: chop a tree → green-log material in inventory → right-click an air block → places OAK_LOG (looks identical to the old block-placement); material count decrements by 1.

### Save compat

No format change.

---

## Phase 5 — DRYING_RACK block + DryingRackData + per-tick advancement

### Goal

The Drying Rack block exists; placing it creates a `DryingRackData` entry in the world's parallel map; the engine's tick advances every slot's seasoning counter by 1 each tick, gated by an air-above check.

### Changes

- `block.rs`:
  - `DRYING_RACK: BlockId = 51`.
  - `BlockDef` for it: solid, opaque (will revisit if it looks bad with the open-frame texture — alpha-priority), no gravity, custom textures (top: side-by-side log-stack icon; side: vertical post-and-rail frame).
  - `mine_drop` for DRYING_RACK: drops the DRYING_RACK block-item itself. **Spilling logs from slots on mine** is handled in Phase 7 (needs save knowledge).
- `texture_gen.rs`:
  - 2 new texture layers (drying-rack top + side). First-cut: stylised log-stack on top; vertical bars on sides.
- `world.rs`:
  - `pub drying_racks: AHashMap<(i32,i32,i32), DryingRackData>` field on `World`. Initialised empty in `World::new`.
- New `game/engine/src/drying_rack.rs`:
  ```rust
  //! Drying Rack — block-entity state + seasoning-progress tick.
  //!
  //! Per Spec 29 / `docs/foundations/2026-05-19-log-seasoning.md`. Mirrors
  //! the campfire pattern: state in a parallel World field; pure tick
  //! function reads world + advances state.
  //!
  //! Folds into Spec 20's BlockEntityData enum when that ships — the
  //! field name + struct shape are deliberately picked to make that a
  //! one-variant rename.

  use serde::{Deserialize, Serialize};

  use crate::block::{self, BlockId};
  use crate::item::MaterialId;

  /// Number of seasoning slots per rack.
  pub const RACK_SLOTS: usize = 8;

  /// Ticks of seasoning required for a slot to be fully mature.
  /// 6000 = 5 real minutes @ 20 TPS. Tunable in Phase 9 playtest.
  pub const SEASON_TICKS: u32 = 6000;

  /// What kind of log is in this slot. Tracks the source so a future
  /// kiln-eligible variant (e.g. dried_alder, dried_birch) can mature
  /// into a species-specific seasoned output. Alpha is OAK only.
  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
  pub enum LogSpecies {
      #[default]
      Oak,
  }

  /// A single rack slot — either empty or holding a green log with its
  /// current seasoning progress (in ticks).
  #[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
  pub struct RackSlot {
      pub species: Option<LogSpecies>,
      pub seasoning_ticks: u32,
  }

  impl RackSlot {
      pub fn is_empty(&self) -> bool { self.species.is_none() }
      pub fn is_mature(&self) -> bool {
          self.species.is_some() && self.seasoning_ticks >= SEASON_TICKS
      }
  }

  /// State stored per-rack in `World::drying_racks`.
  #[derive(Clone, Debug, Default, Serialize, Deserialize)]
  pub struct DryingRackData {
      pub slots: [RackSlot; RACK_SLOTS],
  }

  impl DryingRackData {
      pub fn empty_slot(&self) -> Option<usize> {
          self.slots.iter().position(RackSlot::is_empty)
      }

      /// First mature slot — used by the empty-hand right-click to
      /// withdraw a seasoned log.
      pub fn first_mature_slot(&self) -> Option<usize> {
          self.slots.iter().position(RackSlot::is_mature)
      }

      pub fn try_place_green(&mut self, species: LogSpecies) -> bool {
          if let Some(idx) = self.empty_slot() {
              self.slots[idx] = RackSlot { species: Some(species), seasoning_ticks: 0 };
              true
          } else {
              false
          }
      }

      /// Take the first mature slot's contents as a (species, count=1).
      /// Returns None if no mature slot exists. Empties the slot.
      pub fn take_mature(&mut self) -> Option<LogSpecies> {
          let idx = self.first_mature_slot()?;
          let species = self.slots[idx].species?;
          self.slots[idx] = RackSlot::default();
          Some(species)
      }
  }

  /// Whether this rack is currently "operating" — has the air-above gate
  /// passed? Block directly above must be AIR for the rack to advance
  /// seasoning. Sheltered (block-above) racks just don't progress (no
  /// damage, no loss — just a stall until the player opens the airspace).
  pub fn is_operating(world: &crate::world::World, x: i32, y: i32, z: i32) -> bool {
      world.get_block(x, y + 1, z) == block::AIR
  }

  /// Outcome of advancing one rack by one tick. Captures slots that
  /// just matured so the caller can fire a "rack ready" toast.
  #[derive(Debug, Clone, PartialEq, Eq, Default)]
  pub struct RackTickOutcome {
      pub newly_mature_slots: Vec<usize>,
  }

  /// Advance one rack's state by one tick. Pure-ish: takes data mutably,
  /// reads world for the air-above gate, returns just-matured slot
  /// indices. Cap seasoning_ticks at SEASON_TICKS so a long-sitting slot
  /// doesn't keep climbing into u32::MAX territory.
  pub fn tick_one(
      data: &mut DryingRackData,
      world: &crate::world::World,
      x: i32, y: i32, z: i32,
  ) -> RackTickOutcome {
      let mut newly_mature = Vec::new();
      if !is_operating(world, x, y, z) {
          return RackTickOutcome::default();
      }
      for (i, slot) in data.slots.iter_mut().enumerate() {
          if slot.species.is_none() { continue; }
          if slot.seasoning_ticks < SEASON_TICKS {
              slot.seasoning_ticks += 1;
              if slot.seasoning_ticks == SEASON_TICKS {
                  newly_mature.push(i);
              }
          }
      }
      RackTickOutcome { newly_mature_slots: newly_mature }
  }

  /// Map a species + maturity to its withdrawal material.
  /// (Alpha: Oak only → SeasonedLog. Future species → species-specific
  /// seasoned variants.)
  pub fn mature_output(species: LogSpecies) -> MaterialId {
      match species {
          LogSpecies::Oak => MaterialId::SeasonedLog,
      }
  }
  ```
- `game_loop.rs` (tick body):
  - Add a sweep that iterates `world.drying_racks` and calls `drying_rack::tick_one` for each entry. Mirrors the campfire sweep. Collect newly-mature slots → optional toast (deferred to Phase 6).

### Tests

- `drying_rack::tests::is_operating_requires_air_above`
- `drying_rack::tests::tick_advances_seasoning_when_operating`
- `drying_rack::tests::tick_does_not_advance_when_sheltered`
- `drying_rack::tests::tick_caps_at_season_threshold`
- `drying_rack::tests::tick_reports_newly_mature_slots`
- `drying_rack::tests::try_place_green_uses_first_empty_slot`
- `drying_rack::tests::take_mature_returns_first_mature_and_clears_slot`
- `block::tests::drying_rack_block_registered`

### Acceptance

- All tests pass; `check.sh` clean.
- Manual: place a Drying Rack, drop a green log (via right-click — Phase 6 wires this), wait 5 real min — slot matures. Place a block above the rack → seasoning stalls (manual log-line check or `/seed`-style debug toast).

### Save compat

`World::drying_racks` is a new field but it's not in `WorldSave` yet — Phase 7 adds the save-side glue. For this phase, racks exist only in-memory and don't survive saves. This is fine since Phase 5 is the engine-side groundwork; Phase 7 closes persistence before any user-facing testing.

---

## Phase 6 — DryingRack right-click interactions + recipe

### Goal

Right-click a Drying Rack with a green log → adds to first empty slot. Empty-hand right-click → withdraws first mature slot as 1× SeasonedLog material. Craftable from 4 sticks. Toasts on each action.

### Changes

- `game_loop.rs` (right-click handler):
  - New branch *before* the campfire branch (drying rack has its own block id, so the dispatch is `if target_blk == block::DRYING_RACK`):
    - If held is `Item::Material(GreenLog)`:
      - If `world.drying_racks.entry(pos).or_default().try_place_green(Oak)` succeeds → consume 1 material from hotbar; toast `"Log added — drying"`; play_place.
      - Else (rack full) → toast `"Rack full"`.
    - Else if hand is empty:
      - If rack has a mature slot → take it (returns `LogSpecies`); award 1× `mature_output(species)` SeasonedLog to inventory; toast `"Seasoned Log!"`; play_pickup.
      - Else → toast `"Not ready yet"` (with the most-mature slot's progress as a percentage in alpha — `"Not ready yet — 47%"`).
    - Else → fall through.
- `crafting.rs`:
  - Drying Rack recipe — 4 sticks arranged 2×2 in the crafting grid → 1 `block::DRYING_RACK` block-item. The 2×2 pattern matches the existing planks-from-logs recipe shape so it's not visually surprising.

### Tests

- `drying_rack::tests::right_click_green_log_places_into_rack` — TestHost
- `drying_rack::tests::right_click_empty_hand_withdraws_mature_log`
- `drying_rack::tests::right_click_empty_hand_on_immature_rack_does_nothing`
- `drying_rack::tests::right_click_with_seasoned_log_falls_through` — only green logs go in
- `crafting::tests::drying_rack_recipe_four_sticks` — 2×2 sticks → DRYING_RACK

### Acceptance

- All tests pass; `check.sh` clean.
- Manual flow: chop tree → 4 green logs → 1 green log → 4 planks → 4 sticks → craft drying rack → place → load all 4 remaining green logs → wait 5 min → withdraw 4 seasoned logs. Build a campfire with one seasoned log; burn time confirms 60 s.

### Save compat

No format change yet (Phase 7).

---

## Phase 7 — Save/load + `/give` aliases + protocol bump + spill-on-mine

### Goal

Drying-rack state survives save/load. `/give` covers all the new items. Protocol bumps. Mining a Drying Rack spills its current slot contents (Minecraft-style) as green logs at any seasoning level — partial seasoning is lost as a tax on un-placing a workstation.

### Changes

- `save.rs`:
  - New struct:
    ```rust
    #[derive(Clone, Serialize, Deserialize)]
    pub struct SavedDryingRack {
        pub x: i32, pub y: i32, pub z: i32,
        pub data: DryingRackData,
    }
    ```
  - `WorldSave` gains `#[serde(default)] pub drying_racks: Vec<SavedDryingRack>` (appended; old saves load with empty vec).
  - Save path collects from `world.drying_racks`. Load path rehydrates into `world.drying_racks`. Mirror the campfire pattern exactly.
- `commands/builtins/give.rs`:
  - `"green_log" | "greenlog"` → `MaterialId::GreenLog`.
  - `"seasoned_log" | "seasonedlog"` → `MaterialId::SeasonedLog`.
  - `"kiln_dried_log" | "kilndriedlog" | "kiln_log"` → `MaterialId::KilnDriedLog`.
  - `"drying_rack" | "dryingrack" | "rack"` → `block::DRYING_RACK`.
- `protocol.rs`:
  - `PROTOCOL_VERSION 14 → 15`.
  - Version-history entry: `v15 (2026-05-19): Wave 29 — log seasoning. Three new log materials (Green/Seasoned/KilnDried); DRYING_RACK block (id 51); per-rack state in world.drying_racks; OAK_LOG mine-drop changed from block to GreenLog material.`
- `block.rs`:
  - `mine_drop_with_seed` for `DRYING_RACK` — drops the DRYING_RACK block-item itself + ALSO spills any slot contents as GreenLog material (single-stack count = number of non-empty slots). Caller in the mining path needs to consult the world's `drying_racks` map; the cleanest API is a separate helper `drying_rack::spill_on_mine(world, pos) -> ItemStack` that the mining code calls *in addition to* `mine_drop`, then the entry is removed from `world.drying_racks`.

### Tests

- `save::tests::drying_rack_round_trip_through_save`
- `save::tests::pre_wave_29_save_loads_with_empty_drying_racks`
- `commands::builtins::give::tests::give_green_log_works`
- `commands::builtins::give::tests::give_seasoned_log_works`
- `commands::builtins::give::tests::give_kiln_dried_log_works`
- `commands::builtins::give::tests::give_drying_rack_works`
- `drying_rack::tests::spill_on_mine_returns_one_green_per_filled_slot`
- `protocol::tests::protocol_version_is_15`

### Acceptance

- `cargo test` green. `check.sh` ALL GREEN.
- Manual: save world with a partially-stocked drying rack; reload; rack still has its logs at the same seasoning levels.
- Manual: `/give kiln_dried_log 4` + place 4 in a fresh campfire — burns 480 s total (4 × 120 s) without smoke pillar.

### Save compat

`WorldSave::drying_racks` is a new appended field with `#[serde(default)]`. Old saves load with an empty Vec. PROTOCOL_VERSION bump forces stale-client rejection on multiplayer connect.

---

## Phase 8 — Spec 5 update

### Goal

Spec 5 reflects what shipped. New §3.11 Drying Rack. §3.10 Campfire amended with the fuel-quality ladder. §5.5 mentions seasoning's relationship to fire-starting reliability (future polish, but call out the hook).

### Changes

- New §3.11 **Drying Rack** subsection (after §3.10 Campfire):
  - Description: 8-slot block-entity workstation; air-above operating gate; 5-real-min seasoning per slot.
  - Recipe: 4 sticks 2×2.
  - Interactions: right-click with green log adds, empty-hand withdraws mature; toast feedback.
  - Save/load: parallel `World::drying_racks` map, will fold into BlockEntityData enum when Spec 20 ships.
- Update §3.10 to amend the fuel-ladder table:
  - GreenLog 30 s + smoke pillar.
  - SeasonedLog 60 s clean.
  - KilnDriedLog 120 s clean (future production via Kiln).
  - `Item::Block(OAK_LOG)` remains 60 s clean for back-compat with old saves.
- §5.5 Tool-Specific Actions: add a hint that seasoning quality *will* affect friction-ignition success rate (currently 70% uniform; future polish lands when fuel-by-fuel-mix tracking exists). Note this is sequenced *after* this spec, so don't lean on it for any Phase 9 playtest acceptance.
- §6.2.1 Food Values and Cooking Risk: tiny note that fuel quality *may* affect cook speed (future polish; uniform now).
- §7.3 Block Drops: amend the `OAK_LOG → GreenLog material (1 per mine)` entry.

### Acceptance

- Spec 5 §3.10, §3.11, §5.5, §6.2.1, §7.3 amended accurately reflect shipped behaviour.
- Drift-audit-style re-read finds no contradictions.

---

## Phase 9 — Axolittle playtest (BLOCKED on his time)

### What he's evaluating

- **First-night feel** — chop tree → green logs → friction-fire with stick → burn 30 s green-log fires + smoke pillar visible across the map. Does it feel like real-life campfire intuition?
- **Smoke-pillar drawback** — does the green-log smoke beacon meaningfully draw mobs and feel like a survival drawback? Or is it just visual noise?
- **Drying-rack-build moment** — does building the first drying rack feel like a "I made my workshop better" beat? Or does the 4-sticks recipe feel too cheap to matter?
- **Seasoning pacing** — 5 real min per slot, 8 slots in parallel. Walk-away friendly. Too fast (trivialises the loop)? Too slow (frustrating)? Tune at 6000 ticks ± 50%.
- **Air-above gate** — does the spatial decision (where to put your rack so it gets airflow) feel meaningful? Or do players just put it anywhere and ignore?
- **Kiln-dried curiosity** — when he `/give kiln_dried_log` and burns one, does the 2× burn time feel premium-enough to want a kiln workstation built next?
- **Back-compat smoothness** — old save with OAK_LOG blocks in inventory loads cleanly; existing log-as-fuel paths still work.
- **Plank recipe** — do plank yields feel unchanged from his POV? (They should — uniform 4 yield regardless of log type.)
- **Building** — right-click with green log places an oak-log block, feels identical to old "right-click with oak log block" placement.

### Outputs

Memory entries on his calls. Tuning patch with adjusted constants if needed (seasoning time, smoke-tick scale, fuel-burn ratios). Spec 5 amended for any design changes.

---

## Future spec hooks (deferred, but designed-for)

### Kiln workstation (post-Spec 20 Furnace)

- New block-entity workstation that takes seasoned logs + coal as fuel → produces kiln-dried logs over a longer tick budget.
- Lands as a one-variant addition to Spec 20's BlockEntityData enum.
- Recipe: 8 cobblestone + 1 furnace (or similar — TBD when Spec 20 ships).
- This spec defines KilnDriedLog's burn value (120 s) so when the Kiln lands, the fuel-side wiring is zero-touch.

### Species variation

- Birch, spruce, Satori-adjacent exotic species (post-alpha world-gen work) all get their own LogSpecies variants.
- DryingRackData already tracks species per slot — adding birch/spruce is a one-variant enum extension.
- Species-specific fuel curves (birch burns faster, oak medium, exotic premium) drop into `fuel_value` cleanly.

### Friction-rate-by-fuel-mix

- The campfire tracks "last fuel kind added" or "dominant fuel mix" → friction ignition success rate skews accordingly.
- Adds a `last_fuel_added: Option<FuelKind>` field to CampfireData.
- Out of scope this spec — current 70% baseline stays.

### Wood Shed (Tier 2)

- Bigger rack-cousin (32 slots, faster seasoning).
- Recipe scales the rack pattern (multiple sticks + planks).
- Lands when economy depth needs it; not blocking.

### Furnace-side seasoning bonuses (Spec 20 integration)

- When Spec 20 Furnace ships, its smelt-tick reads the fuel-value table the same way campfire does. Kiln-dried logs auto-give more smelts per unit. No code change in this spec needed; the fuel-value table is the integration point.

---

## Memory rule check

- ✓ signet boundary — N/A. Seasoning touches no Signet/identity.
- ✓ uk english naming — "Seasoning" (UK-standard); "Drying Rack" (closed compound); "Kiln" (not "furnace"). No transatlantic divergences.
- ✓ axenstax has farming — coherent with the survival/cooking arc. Seasoning is the wood-fuel parallel to crop-growing's time-investment loop.
- ✓ shared infra strategy — block-entity tick + dryness state-machine + air-above operating gate + `material_as_placeable_block` helper are engine-generic primitives. Burn-time tables and species enum are AxeNStax-specific data.
- ✓ alpha launch posture — post-alpha-priority but useful enough to ship before alpha if cycles allow. Doesn't gate alpha; removes a tonal hand-wave from the existing fuel system.
- ✓ pretest check — implementer should grep current state before starting (block IDs 50+; PROTOCOL_VERSION 14; current `OAK_LOG.mine_drop` flow; existing campfire fuel branch in game_loop.rs).
- ✓ autonomy to playtest boundary — Phases 2-8 autonomous, Phase 9 = playtest gate.
- ✓ proof of play is proof of work — orthogonal; no hashing in this spec.

---

## Acceptance — overall

- 9 phases complete or explicitly blocked (Phase 9 = playtest).
- `./check.sh` ALL GREEN throughout.
- Three new log materials in `MaterialId`; tree-fell drops GreenLog; placed-log mining drops GreenLog (back-compat materials stay).
- Drying Rack block + workstation tick + 4-sticks recipe + right-click interactions.
- Save/load preserves rack state across sessions.
- Smoke pillar fires from green-log burns (Wave 28 mechanic re-used).
- Spec 5 §3.10, §3.11, §5.5, §6.2.1, §7.3 amended.
- axenstax has farming memory gets a small note: "Log seasoning shipped 2026-05-19 as the wood-fuel parallel to the farming/cooking arc. Three materials (Green/Seasoned/KilnDried), Drying Rack workstation, green-log smoke beacon, kiln-dried path deferred until post-Spec-20."
- This foundation doc gets a status flip READY TO BUILD → DELIVERED at the top.

---

## Out of scope (explicitly)

- Kiln workstation (post-Spec 20).
- Wood Shed (Tier 2 future).
- Species variation beyond Oak (depends on world-gen species work).
- Friction-rate-by-fuel-mix (future polish).
- Rain / weather interaction (no weather system yet).
- Per-stack metadata on materials (sidestepped by three separate `MaterialId` variants).
- Furnace-side seasoning bonuses (Spec 20 inherits the fuel-value table automatically).
- Generic block-entity framework (Spec 20).
- Multi-player rack authority (single-player owns the tick until HostedServer routing lands — BRIDGE).
- Cook-time-by-fuel-quality (future polish).
- Light-emission from drying racks (they're not on fire).

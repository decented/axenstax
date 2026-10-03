# Furnace UX refinement — Axolittle playtest feedback 2026-05-21

**Status**: **DELIVERED 2026-05-21** on `feat/spec29-furnace-ux-refinement`. All three asks shipped: 'E'/Esc closes (no Close button), three clickable slot-buttons with Single/Stack click modes, ore-only smelting (raw-meat arms removed). Legacy meat in v1 saves ejects into `world.pending_legacy_meat_drops` and spawns as ItemEntities on first post-load tick (drain wired into both `GameState::tick` and `GameServer::tick`). Phase 8 (Axolittle playtest gate) is the remaining gate.
**Date**: 2026-05-21
**Parent spec**: `docs/foundations/2026-05-18-furnace.md` (Spec 20, DELIVERED 2026-05-20)
**Sibling spec**: `docs/foundations/2026-05-18-campfire.md` (Spec 17 — cooking primitive stays here)
**Tester**: Axolittle (live playtest 2026-05-21)

---

## TL;DR

Axolittle has played the v1 furnace and made three calls that bring it closer to Minecraft's mental model:

1. **'E' closes the furnace UI** (not a Close button). Match inventory close-key behaviour.
2. **Slot-based drag/drop UI** when right-clicking the furnace — three slots (input / fuel / output), exactly like Minecraft's furnace screen. Replace the current "right-click with item held to insert" pattern.
3. **Furnace smelts ore only, not food.** Cooking food belongs at the Campfire (Spec 17). The grid-deprecation arms (raw meat → cooked meat) that Spec 20 Phase 6 migrated *to* the furnace get removed from the furnace recipe table.

Net effect: the furnace becomes Minecraft-recognisable on three dimensions a kid notices on first contact — keypress, screen, role.

---

## Why this matters

Spec 20 v1 chose right-click-to-insert because slot-drag UI is a meatier engineering job and the v1 ship pressure favoured a working furnace over a polished one. The deprecation arms (food cooking on furnace) were a transitional convenience — Spec 17 added the proper Campfire cooking primitive but Spec 20 still accepted raw meat to keep the grid-smelt migration smooth.

Both shortcuts are now visible to the player and clash with the Minecraft mental model Axolittle expects. The campfire IS the cook-station; the furnace IS the smelter. Splitting them cleanly clarifies the workstation roster and frees the furnace UI to be a proper menu.

---

## Scope (what's in)

### A. Close-key parity with inventory

- Remove the **"Close" button** at `game/engine/src/furnace_ui.rs:121-123`.
- Press **'E'** while the furnace UI is open → emits `FurnaceUiOutcome::Closed` (mirroring how 'E' toggles the inventory).
- **Esc** still works as a secondary close, preserving the existing `:127-130` Esc handler.
- Update the dialog-modal input-gating in `game_loop.rs` so 'E' is consumed by the furnace UI (not toggled-through to inventory).

### B. Slot-based drag/drop UI

Three labelled, interactive slots in the existing egui panel:

```
   [ INPUT  ]   →[progress arrow]→   [ OUTPUT ]
   [ FUEL   ]
```

- **Click a slot with an item held** → place 1 (or shift-click for full stack) into the slot. Item moves from cursor stack to slot stack.
- **Click a slot with empty hand** → pick up the slot's stack into cursor stack (shift-click moves to inventory).
- **Slot acceptance rules** baked into the click handler:
  - Input slot accepts only items with a `smelt_recipe_for_input(input)` match → see §C for the narrowed table.
  - Fuel slot accepts only items with a `fuel_burn_ticks(item) > 0` match (coal / plank / log / stick — existing helper).
  - Output slot is read-only on insert; only Take is allowed (matches Minecraft).
- The existing tooltip at `furnace_ui.rs:111-115` is replaced with a one-line hint: "Drag ore into Input, fuel into Fuel. Take the smelted ingot from Output."
- The progress-arrow widget (already drawn at `:85-93` via egui ProgressBar) is unchanged; just relocated visually between input and output for the Minecraft layout.

The right-click-with-item-to-insert path in `game_loop.rs` (Spec 20's v1 insertion pattern) is **removed for furnaces**. Right-clicking a furnace now just opens the UI; the slot-click flow handles all insertion. Campfire's right-click-to-insert pattern stays — campfires don't have a UI, so right-click is the only path.

### C. Ore-only smelting (remove food cooking)

Narrow `furnace::smelt_recipe_for_input` in `game/engine/src/furnace.rs:99` to ore inputs only:

| Input | Output | Status |
|---|---|---|
| `RawIron` | `IronIngot` | KEEP |
| `Copper` | `CopperIngot` | KEEP |
| `Tin` | `TinIngot` | KEEP |
| `RawBeef` | `CookedBeef` | **REMOVE** — campfire cooks meat now |
| `RawChicken` | `CookedChicken` | **REMOVE** |
| `RawMutton` | `CookedMutton` | **REMOVE** |
| `RawPorkchop` | `CookedPorkchop` | **REMOVE** |

The unit tests at `furnace.rs:359-371` get trimmed to match — `assert_eq!` lines for the cooked-meat arms become `assert_eq!(smelt_recipe_for_input(MaterialId::RawBeef), None)` etc.

The campfire (`campfire.rs::tick_one`) already cooks all four raw meats — no migration needed for cooking content.

**Migration note for existing saves:** A furnace with raw meat in its input slot at load-time should drop the meat back as a loose item on the ground next to the furnace block. Implement as a one-shot save-load hook in `save.rs::load_world` after furnaces deserialize: if `input.item == RawBeef|RawChicken|RawMutton|RawPorkchop`, eject it. No save-format version bump needed (the slot just goes empty).

### D. Spec 5 + Spec 20 doc updates

- `docs/spec/05-gameplay-systems.md` §3.14 (or wherever furnace surface is described) — note furnace is ore-only.
- `docs/foundations/2026-05-18-furnace.md` — add a "v2 — UX refinement" status section at the top pointing to this spec.
- `docs/foundations/2026-05-18-campfire.md` — note campfire is the canonical cooking station; furnace does not duplicate.

---

## Scope (what's out — deferred)

- **Shift-click chains** (shift-click from inventory → auto-route to correct furnace slot). Nice-to-have but not blocking. Phase 2 if Axolittle asks.
- **Hopper-style auto-insertion** from adjacent inventory blocks. Future T2+ farming/transport spec.
- **Multi-slot inputs/fuels** (Minecraft has one of each; we match). Not deferred to "future" — explicitly not adding.
- **Furnace v2 textures** — the v1 lit/unlit pair stays.
- **Touch / gamepad slot interaction** — keep the v1 path (gamepad cursor) working; specialised touch-drag UX deferred.

---

## Phasing

| # | Phase | Files | LOC | Solo? |
|:-:|---|---|--:|:-:|
| 1 | Tests for the new behaviour (TDD). Adjust the four cooked-meat tests at `furnace.rs:359-371` to assert `None`. Add a new test confirming `RawIron → IronIngot` still works. Add an integration test in `test_integration/` that drives the slot-click flow via TestHost. | `furnace.rs`, `test_integration/furnace_ui.rs` (new) | ~150 | ✓ |
| 2 | Narrow `smelt_recipe_for_input` to ore arms only. Cook-meat arms deleted. Re-run tests — Phase 1's failing tests pass. | `furnace.rs` | ~30 | ✓ |
| 3 | Save-load eject hook for legacy raw-meat-in-furnace saves. Test: save a furnace with RawBeef in input, run through Phase 2's narrowed table, confirm load drops the meat as a loose entity. | `save.rs`, `test_integration/save_load.rs` | ~80 | ✓ |
| 4 | Slot-click UI in `furnace_ui.rs`. Three clickable slots with cursor-stack interaction. Remove the Close button. Remove the right-click-tooltip text. Add hint line. Wire 'E' close handler. Update `game_loop.rs` to gate 'E' input through the furnace UI when open. | `furnace_ui.rs`, `game_loop.rs` | ~250 | ✓ |
| 5 | Remove the right-click-with-item-to-insert handler for furnaces in `game_loop.rs` (campfire's stays). Test: right-clicking a furnace with raw iron held opens the UI but does NOT insert. Test: slot-click insertion still works. | `game_loop.rs`, `test_integration/furnace_ui.rs` | ~100 | ✓ |
| 6 | Spec doc updates per §D above. | `docs/spec/05-gameplay-systems.md`, `docs/foundations/2026-05-18-furnace.md`, `docs/foundations/2026-05-18-campfire.md` | ~50 | ✓ |
| 7 | `./check.sh` ALL GREEN. Texture-gen audit: confirm the furnace textures still match the v1 lit/unlit pair. | – | – | ✓ |
| 8 | Axolittle playtest gate (next session). Open a furnace, drop iron ore into Input via click-drag, watch it smelt to Iron Ingot, take the ingot out, press 'E' to close. Try raw beef on the furnace — refuses. Take the beef back to the campfire — cooks fine. | – | – | playtest |

**Total Phases 1-7:** ~660 LOC (mostly tests + UI plumbing; recipe-table narrowing is tiny).

---

## Acceptance — overall

- All 7 phases complete or explicitly blocked (Phase 8 = playtest gate).
- `./check.sh` ALL GREEN throughout.
- Furnace v1 saves load cleanly (raw meat ejected as loose entity, no panic).
- Spec 20 doc has a v2 status section pointing at this spec.
- Spec 5 furnace surface description reads "ore only" — no raw-meat references.
- Axolittle confirms: 'E' closes the furnace, slot-click works, food won't smelt.

---

## Open design questions

1. **Slot-stack-size limit**: Minecraft caps the input slot at 64. AxeNStax engine stack-sizes are already 64-ceil — inherit that, no special handling needed. Confirm during Phase 4.
2. **Empty-slot click with empty hand**: no-op or play a UI sound? No-op for v1; sound feedback is a polish pass.
3. **What about shift-clicking the OUTPUT slot to auto-move to inventory?** Minecraft does this. Add to Phase 4 if it's a 5-line addition; defer otherwise.
4. **Shift-click into a furnace that's already smelting**: should the new input join the queue or refuse? Minecraft: stacks compatibly (if input slot has 8 RawIron and you shift-click another 12, becomes 20). Match that.
5. **Should the campfire spec doc gain a "this is the cooking station, not the furnace" line?** Yes — §D covers it.

---

## Cross-game lift

The slot-click + cursor-stack interaction pattern is engine-generic. Once it lands here, the Vendor Block's existing slot UI can be normalised to the same widget. Spec 25 (Plan Trade) and any future Chest UI inherit the same handler.

---

## Memory-rule check

- ✓ axenstax has farming — clarifies the workstation roster (Campfire = cook, Furnace = smelt) without re-litigating it.
- ✓ uk english naming — no terminology changes.
- ✓ shared infra strategy — slot-click widget is reusable; explicitly called out under "Cross-game lift".
- N/A charter phase4 shared gap — no Signet surface.
- N/A pretest check — this IS the post-test follow-up; verifies live behaviour matched the spec, finds gaps, drafts the fix.

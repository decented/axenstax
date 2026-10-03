# Gamepad UI Navigation — slot cursor, menu focus, in-world stubs

**Status: BUILT 2026-06-12** (merged to main same day). Feel/UX = Axolittle
playtest boundary — test sheet at `docs/test-sheets/2026-06-12-gamepad-ui.md`.

## Problem

The engine's *in-world* gamepad layer was already mature and cross-platform
(gilrs native / `navigator.getGamepads()` web, one `GamepadState` →
`PlayerIntent`), but every **UI surface** was 100% pointer-driven: the
inventory/crafting panel, the lobby world list, and the dialogs all acted on
egui `.clicked()` only. Opening the inventory with B on a pad left the player
stranded — "many things are rendered useless like the inventory" (owner,
2026-06-12).

## Console pattern adopted (Minecraft Bedrock console editions)

- A **slot cursor**, not a fake mouse pointer: d-pad moves a highlighted
  slot; face buttons act on it. A = take/place whole stack, X = split/half /
  place-one, B = close.
- **On-screen button legend** (no hover tooltips on a pad).
- **Seeded focus** on menus; B always backs out.
- R3 = sneak toggle, D-pad-down = drop one item.

## What shipped (one implementation, web + native — shared egui code)

1. **In-world stubs** (`gamepad.rs`, `gamepad/native.rs`): R3 toggles sneak
   (mirrors L3 sprint), D-pad-down = `drop_item`. The menu-open intent gate
   in `game_loop.rs` now also zeroes `camera_cycle` (X is the slot-cursor's
   split button; it must not cycle the camera behind the panel).
2. **Crafting/inventory slot cursor** (`craft_ui.rs`): `PadSlot` +
   `pad_move` (pure, clamped, exhaustively unit-tested) navigate armour
   column | grid | result | main rows | hotbar with column-mapped vertical
   seams. `pad_activate` emits the **same `ClickTarget` actions as mouse
   clicks** — the lossless click logic is reused untouched. Cursor is `None`
   until the first d-pad press (mouse users never see it); gold focus ring;
   carried item anchors beside the focused slot; footer legend swaps to
   button hints.
3. **game_loop wiring**: per-frame pre-pass maps each player's UI pad via
   `local_join::ui_pad_index` (= `expected_controller_index` + the solo
   KB+M fallback to unowned pad 0). **Press-A-to-join is suppressed while
   any crafting UI is open** — solo P1's UI pad is exactly the pad the join
   rule watches.
4. **Menu focus navigation** (`inject_gamepad_nav`): **bug found** — the old
   implementation pushed key events via `ctx.input_mut` *after*
   `begin_pass`, but egui consumes Arrow/Tab focus keys from the `RawInput`
   *at* pass start, so the d-pad never moved menu focus (only A=Enter /
   B=Escape reached `key_pressed()`). Now injected into the RawInput
   pre-pass; first press seeds focus with a synthetic Tab, then arrows ride
   egui 0.34's directional `FocusDirection` navigation. Gold focus
   visibility: global `active.bg_stroke` (egui renders `has_focus` with the
   "active" widget style) + `menu::focus_ring` for custom-styled buttons
   and collapsed world cards. egui fake-clicks the focused `Sense::click()`
   widget on Enter/Space, so A activates cards/buttons natively.

Spec 05 §12.1 updated (button table had drifted from the code — B/X/Select
rows were wrong; corrected against `state_to_intent`).

## Deliberately deferred (don't stack on bridges)

- **Recipe book** (console crafting pattern): blocked on a **data-driven
  recipe registry** — `crafting::match_recipe` is a ~1,300-line procedural
  matcher, so a "list of craftable outputs" can't be derived without
  re-encoding recipes. Registry refactor is its own foundations spec; the
  slot cursor makes the manual grid pad-usable meanwhile.
- **Chest / furnace / vendor / explorer panels**: same PadSlot pattern,
  per-panel geometry. Next consumers once the inventory feel is validated.
- **Left-stick slot navigation + held-d-pad key repeat** (Bedrock supports
  both; v1 is d-pad edges only).
- **D-pad direct hotbar select in-world** (LB/RB scroll already covers it).
- **Workshop chords** (eyedropper, symmetry, pin, gallery) — still
  KB+M-only, BRIDGE comments in `gamepad.rs`.
- **Rumble**: the one genuine web/native divergence (gilrs force feedback
  vs patchy browser support). Optional polish, not navigation.

## Verification

- 2,534 engine tests green (`cargo test --bin axenstax-engine`), incl. 16
  new `pad_move`/`pad_activate` tests with an exhaustive stays-in-bounds
  sweep, 4 new gamepad-mapping tests, 3 new `ui_pad_index` tests.
- `check.sh` green pre-merge.
- `--shot-lobby` headless render confirmed no lobby rendering regression.
- **Not solo-verifiable**: actual pad-in-hand feel (focus ring visibility at
  TV distance, d-pad travel speed, X-split discoverability) — Axolittle
  playtest, web + native.

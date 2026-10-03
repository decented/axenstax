# Inventory QoL — sort, quick-stack, drag-distribute, scroll-transfer, auto-refill

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-alpha-qol-building-blocks`, goal `2026-06-16-alpha-qol-and-building-blocks`). Shipped: P1 pure `sort_slots` + locked slots (Alt+click) + Sort button; P2 Dump/Restock/Take-all + Sort on the chest dialog; #28 trash slot (bins the held cursor item); P4 opt-in auto-refill (default on, stackables only). **P3 + persistence landed 2026-06-19** (goal `2026-06-19-solo-buildout-wave-2`, Wave 3): **scroll-wheel transfer** in the container dialog (`chest_ui::deposit_to_chest`/`withdraw_chest_slot`, one item per scroll tick over a slot, shift = whole stack; count-conserving) and **locked-slot persistence** (`WorldSave.locked_slots`, append-only; `Inventory::{locked_indices,set_locked_from}`; player 0 only — split-screen + dedicated-server locks not persisted). **RMB-drag-distribute + LMB-drag-gather BUILT 2026-06-19** (same goal, Session 2) — Mouse-Tweaks parity now complete: RMB-drag across slots deposits one carried item per slot, LMB-drag gathers matching items into the cursor (grid + inventory), count-conserving, painted once per gesture via a visited set, `click_and_drag` sense so a paint doesn't also fire a single-slot click. `check.sh` green (3086 tests). *Feel = Test Session 2 playtest.* Backlog **#2** from `docs/research/2026-06-15-native-bake-in-feature-backlog.md` (spec B of the 2026-06-15 build-now QoL sweep).
**Date**: 2026-06-15
**Branch (when built)**: TBD (`qol/inventory`).
**Owner decisions captured (2026-06-15)**: **buttons-first / discoverable** for the kid audience — sort and quick-stack are **on-screen buttons** (not hidden keys), which also **resolves the R-key collision** (JEI uses R for "show recipe", Inventory Profiles Next uses R for "sort"; here sort is a button, recipe/uses is right-click — see spec C). Power-user mouse behaviours (drag, scroll-transfer) and an optional R shortcut layer on top. Build-now-able: single-player / PWA, no multiplayer gate.

---

## TL;DR

Inventory interaction polish — the **#2 most-installed mod category in Minecraft** (Mouse Tweaks ~455M, Inventory Profiles Next ~30M). The inventory data model exists; this is **interaction on top of it**. Concretely:

- **Sort** — a visible Sort button in the player inventory and any open container, grouping/stacking items deterministically. **Locked slots** are excluded from sorting.
- **Quick-stack / dump / restock** — buttons on a container: "Dump matching" (move all stacks the container already holds), "Restock" (refill from the container), and "Take all".
- **Mouse behaviours** (Mouse Tweaks parity) — **RMB-drag distribute** (drag across slots with RMB held → one item into each), **LMB-drag gather** same-type, **scroll-wheel transfer** between inventory ↔ container (down = move one out, up = fill a stack), Shift = act on all matching.
- **Auto-refill** (opt-in) — when a hotbar tool/stack runs out, refill it from the inventory; per-slot toggle.

Reference behaviour (researched 2026-06-15): [Mouse Tweaks](https://modrinth.com/mod/mouse-tweaks) (RMB-drag distribute, two LMB-drag mechanics, scroll-wheel transfer; purely client-side) and [Inventory Profiles Next](https://www.curseforge.com/minecraft/mc-mods/inventory-profiles-next) (sort, locked slots, auto-refill with per-slot icon, gear sets, container overlay buttons).

---

## Why this lives here

- Per the backlog: inventory UX is the single most-installed QoL class after the recipe viewer — **expected** behaviour, not a nicety. Small surface, outsized daily payoff.
- Per the 2026-05-21 playtest lesson (kids miss non-obvious affordances): the headline actions are **buttons**, not hotkeys. Mouse-drag/scroll are additive power features, documented but not required to discover.
- Per gamepad ui navigation shipped: there is now a real slot cursor + menu focus nav — the Sort/quick-stack buttons must be reachable by the **gamepad cursor** too, not mouse-only.
- Per shared infra strategy + uk english naming: generic engine inventory polish, UK English.

---

## The real seam (grounded)

`game/engine/src/`:
```text
inventory.rs        Inventory { slots: [Option<ItemStack>; 36], open }
                    slots 0–8 hotbar, 9–35 main; slot(), set_slot(), add_item()
item.rs             ItemStack { item: Item, count: u32 }; Item = Block|Tool|Material|Plan|Armour
inventory_explorer.rs   full inventory browse screen (B key — input.rs:key_pressed)
chest_ui.rs:26      show_chest_dialog(ctx, chest, inventory, pos)
                    :52  ui.input(|i| i.modifiers.shift)   ← shift-modifier already read
                    click = move one item; Shift-click = move stack (withdraw_chest_slot)
chest.rs:31         ChestData { slots: Vec<Option<ItemStack>> } (27 slots, try_insert)
input.rs:57         scroll_delta: f32; right_held/left_held; right_click/left_click (edge);
                    end_frame() (:236) clears click/scroll each tick
```
**What exists:** the click-to-move-one / shift-to-move-stack transfer, the shift modifier read, and raw scroll/drag input state. **What's missing:** the sort algorithm, locked slots, the quick-stack/dump/restock buttons, the drag-distribute/gather **state machine**, scroll-transfer routing, and auto-refill. The drag/scroll state machine should live in a focused new module (`inventory_interaction.rs`) consumed by both `inventory_explorer.rs` and `chest_ui.rs`, rather than duplicated.

---

## Scope (phased)

### Phase 1 — Sort + locked slots
- A **pure** `sort_slots(&[Option<ItemStack>], &locked) -> Vec<Option<ItemStack>>` (merge partial stacks, then order by a stable item key: category → id → count desc). Lives in `inventory.rs` (or `inventory_interaction.rs`), fully unit-testable.
- A **Sort button** drawn in the player inventory and in `show_chest_dialog` (container sorts its own contents). Gamepad-cursor reachable.
- **Locked slots**: right-click a slot to toggle a lock; locked slots keep their item index through a sort. Lock state persists with the inventory (per-player).
- Optional: bind **R** as a sort shortcut while an inventory is open (additive; the button is primary).

### Phase 2 — Quick-stack / dump / restock
- Container overlay buttons: **Dump matching** (move every player stack whose item already exists in the container), **Restock** (pull matching stacks back), **Take all**. Backbone = "act on all matching", reused by the shift behaviours in Phase 3.

### Phase 3 — Mouse behaviours (Mouse Tweaks parity)
- `inventory_interaction.rs` drag/scroll state machine driven by `input.rs` (`right_held`, `left_held`, `scroll_delta`, edge clicks):
  - **RMB-drag distribute**: hold RMB, drag across empty/like slots → deposit one carried item per slot.
  - **LMB-drag gather**: hold LMB over an item, drag → pick up like items; Shift+drag → gather all matching.
  - **Scroll-transfer**: hovering a slot with a container open, scroll down → move one item to the other inventory; scroll up → fill a stack from the other inventory; Shift → whole stack.

### Phase 4 — Auto-refill (opt-in)
- Setting `auto_refill: bool` (default on for tools) in the player/settings model; per-slot toggle with a small icon (IPN-style). When a held/hotbar stack hits zero, pull an identical stack from the main inventory.

### Deferred (in-spec)
- **Gear sets** (save/equip loadouts) — IPN power feature, separate later spec.
- **Custom sort-rule editor** — ship one good default order; configurable rules later.

---

## Acceptance criteria

- **P1:** `sort_slots` is a pure function with unit tests (merges partials, stable order, never duplicates/loses items, respects locked indices). Sort button sorts player inventory and an open container; locked slots stay put; reachable by mouse, touch, and the gamepad cursor.
- **P2:** Dump/Restock/Take-all move exactly the matching stacks with correct overflow handling (reuse `ChestData::try_insert` semantics); no item duplication (test: total item count invariant before/after).
- **P3:** RMB-drag distributes one-per-slot; LMB-drag gathers; scroll-transfer moves the right quantity in the right direction; Shift escalates to whole-stack/all-matching. State machine resets cleanly on mouse-up / inventory close (no "stuck carrying" state).
- **P4:** Auto-refill (when enabled) refills an emptied hotbar tool from inventory; per-slot toggle works; default off-for-non-tools so it never surprises.
- No item duplication or loss anywhere (the load-bearing invariant) — covered by count-conservation tests. `./check.sh` green.

## Memory-rule check
- **Concrete, not cards**: a shared `inventory_interaction.rs` (one responsibility) consumed by both inventory and chest UIs — not duplicated logic, not a god-file. Sort is a pure function. No bridge code.
- **Multiplayer-ready**: Mouse Tweaks is client-side in MC and this is too — pure UI over the existing local inventory model; nothing here assumes single-player in a way that blocks a future server-authoritative inventory.
- **Spec maintenance**: on build, update Spec 05 §Inventory with the sort order, locked-slot model, and auto-refill rule.
- **No build authorised** beyond this queue entry — graduates on "build inventory QoL" / "add #2".

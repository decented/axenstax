# Recipe book → JEI parity — reverse "what uses this" + always-open browser

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-alpha-qol-building-blocks`, goal `2026-06-16-alpha-qol-and-building-blocks`). P1 reverse "uses" index shipped: `crafting_catalogue::recipes_using(item)` backed by a `LazyLock<HashMap<(u8,u32), Vec<usize>>>` keyed by `Item::sort_key` — derived from the same `RecipeCard`s, no second source of truth; unit-tested == brute-force scan; the catalogue→matcher consistency test still passes. Entry point: **hover an inventory item + press U** → opens the book filtered to those recipes (`book_uses_filter` on `CraftingUi`, honoured by `visible_indices`; cleared on Close / tab-switch). P2: the book is already always-open from the inventory's "📖 Recipe Book" button (no workbench needed) — the standalone main-menu browser is unnecessary for the alpha. `check.sh` green (2605 tests). Backlog **#3** from `docs/research/2026-06-15-native-bake-in-feature-backlog.md` (spec C of the 2026-06-15 build-now QoL sweep).
**Date**: 2026-06-15
**Branch (when built)**: TBD (`qol/recipe-parity`).
**Owner decisions captured (2026-06-15)**: **buttons-first / discoverable** — the reverse lookup is **right-click an item → "what can I make with this"**, and the always-open browser opens from an on-screen button (with optional U-key for power users). This deliberately keeps **R** free of a collision (see spec B: sort is a button, not R).
**Scope correction (verified 2026-06-15)**: the backlog estimated #3 at "~90% done / remaining 15%". Code inspection confirms the **type-to-filter search box already shipped** (`recipe_book_ui.rs:44-61 visible_indices`, drawn `:125-141`). So this spec is **just the two genuine gaps**: the reverse "uses" index and the always-open browser. Smaller than the backlog implied.

---

## TL;DR

The shipped recipe book (Spec 43, 2026-06-12) already does JEI's core: a card catalogue, the drill-down "how to make it", grid autofill, a **search box**, and mouse/touch/gamepad-Y input. Two JEI behaviours remain:

1. **Reverse "uses" index (#3a)** — JEI's **U key** ("show uses"): given an item, *what recipes consume it*. We only have the **forward** map today (`recipe_index_for_output`, output → recipe). Add `recipes_using(item)` built from `RecipeCard.ingredients`, surfaced as **right-click an item → "What can I make with this"**, opening the book filtered to those cards.
2. **Always-open browser (#3b)** — JEI is browsable any time, not only from a crafting grid. Add a recipe-browser panel openable independent of the crafting UI, reusing the existing card list + search + drill-down.

Reference behaviour (researched 2026-06-15): [JEI](https://www.curseforge.com/minecraft/mc-mods/jei) — **R = show recipe** (how to make the hovered item), **U = show uses** (every recipe that consumes it), plus bookmarks; the item-list overlay is always available.

---

## Why this lives here

- Per the backlog: JEI/REI is the **#1 utility-mod class by installs (~56.7M)**; we are most of the way there, so the remaining effort is small for outsized polish.
- Per recipe catalogue and book: the catalogue **delegates to `match_recipe`** and a consistency test asserts every card's grid → matcher == output. The reverse index must preserve that contract — it indexes the same `RecipeCard`s, it does not introduce a second source of truth.
- Per the 2026-05-21 playtest lesson: the reverse lookup is a **right-click action on an item the player is already looking at** — more discoverable than a hidden U key (which we add only as an optional shortcut).
- Per shared infra strategy + uk english naming: generic, UK English.

---

## The real seam (grounded)

`game/engine/src/`:
```text
recipe_book_ui.rs:65   draw_recipe_book(...)  → RecipeBookAction { Close, SetCategory, Fill(idx) }
              :44-61   visible_indices(category, search)  ← SEARCH SHIPPED (name substring, all categories)
              :125-141 search box drawn
crafting_catalogue.rs:955  static CATALOGUE: LazyLock<Vec<RecipeCard>>
              :967  all_cards()   :972 cards_in(category)   :977 search(query)
              :1034 recipe_index_for_output(item)   ← FORWARD only (output → recipe)
              RecipeCard { name, output, ingredients: Vec<RecipeIngredient>, example_grid, category, station }
craft_ui.rs:59   CraftingUi { grid, result, cursor_item, book_open, book_category,
                              book_search, book_focus, pinned_recipe_stack, ... }
              :99  open_book()      :203 autofill_from_example()  (drill-down already works)
game_loop.rs:8428  gamepad Y opens book      :11388-11404 book action dispatch
```
**What exists:** search, forward output→recipe map, drill-down + autofill, the `pinned_recipe_stack` (a bookmark primitive already in the struct). **What's missing:** a reverse ingredient→recipes map, a right-click "uses" entry point, and a grid-independent browser panel.

---

## Scope (phased)

### Phase 1 — Reverse "uses" index (#3a)
- Add `recipes_using(item: &Item) -> Vec<usize>` to `crafting_catalogue.rs`, backed by a `LazyLock<HashMap<ItemKey, Vec<usize>>>` built once at startup by iterating `all_cards()` and indexing each `RecipeCard.ingredients`. (`ItemKey` = a normalised hashable key over the `Item` enum — add a small `fn key(&self)` if `Item` isn't already `Hash`.)
- Entry point: **right-click an item** in the inventory/hotbar/explorer → context action **"What can I make with this"** → opens the book with `visible_indices` constrained to `recipes_using(item)`. Optional **U** shortcut while hovering, matching JEI muscle memory.
- A new `RecipeBookAction`/filter state (`book_filter: Option<UsesOf(Item)>`) so the book can show "uses of X" alongside the existing category/search filters.

### Phase 2 — Always-open browser (#3b)
- A recipe-browser panel openable **independent of the crafting grid**: a button in the inventory screen + a menu entry (and optional hotkey / T long-press). Reuses `all_cards()`, `search()`, `visible_indices`, and the card drill-down — no new card-drawing code.
- State lives next to `book_open` but does not require an open `CraftingUi` grid (so it works from the inventory, not just the workbench). Selecting a card still drives the existing drill-down ("how to make it"); autofill is only offered when a grid is actually open.

### Phase 3 (optional) — Bookmarks rail
- Promote `pinned_recipe_stack` into a small **bookmarks** rail (JEI-style): pin a card to keep it on screen across category switches. Low cost given the field already exists.

---

## Acceptance criteria

- **P1:** `recipes_using` returns exactly the cards whose `ingredients` include the item; right-clicking an item opens the book filtered to those cards; the result set matches a brute-force scan of `all_cards()` (unit test). The catalogue→matcher consistency test still passes (no second source of truth introduced).
- **P2:** The browser opens from the inventory (button) with no crafting grid present, supports search + category + drill-down, and reachable via mouse, touch, and the gamepad cursor. Opening from a workbench still offers autofill; opening standalone does not error.
- **P3 (if built):** Pinned cards persist across category/search changes within a session.
- `./check.sh` green (including the catalogue consistency test).

## Memory-rule check
- **Concrete, not cards**: the reverse index is derived from the same `RecipeCard`s as the forward map — single source of truth, delegating to `match_recipe` as before. No duplicate recipe data.
- **Spec maintenance**: on build, update the recipe-catalogue spec (`docs/superpowers/specs/.../recipe-*` / Spec 43 notes) to record the reverse index + browser entry points, and the controls doc for the right-click "uses" action.
- **No build authorised** beyond this queue entry — graduates on "build recipe parity" / "add #3".

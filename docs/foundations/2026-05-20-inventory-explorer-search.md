# Inventory Explorer with Search — Spec 28f

**Status:** DELIVERED 2026-05-20 on main (PR #25). `inventory_explorer.rs` (~805 LOC) — full-screen overlay, case-insensitive substring search, category filter, give-to-hotbar in creative, "×N" survival count. Reads from `enumerate_all_items()` so it picks up new content automatically.
**Branch:** `feat/inventory-explorer-search` off `main`.
**Trigger:** Sub-foundation of [Spec 28 Minecraft-parity content surface](2026-05-20-minecraft-parity-content-surface.md) §6. Standalone — doesn't depend on the other content sub-foundations because it enumerates whatever content the engine has at build time.

---

## TL;DR

New full-screen pane separate from the hotbar + crafting inventory that shows **every item, block, and tool** registered in the engine. Filter by text (substring search) + by category. Click any entry for a tooltip (name, type, source, food value if any, damage if tool). In **creative**, click an entry to add one to the hotbar (an in-UI `/give`). In **survival**, the entry shows "You have: N" so you can quickly check what's in your inventory without scrolling.

Opens via the **`B` key** (currently unbound). Esc closes.

**Scope:** ~700 LOC across 6 phases. Phase 7 is the Axolittle playtest.

---

## Why this lives here

- The user explicitly asked for "a way to explore all these in the inventory with a search option" alongside the Spec 28 content roster.
- Standalone — no dependency on the other sub-foundations. The explorer enumerates whatever's in the engine when it runs; as Spec 28a-e land new content, the explorer picks them up for free.
- Cross-game lift: the registry-of-all-content + search overlay is engine-generic. Any Decented game with a varied item surface adopts the same pattern.
- Educational: a kid playing AxeNStax can browse the full surface of what's in the game without trial-and-error /give commands.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/block.rs` — `BlockRegistry::blocks` is already a `Vec<BlockDef>`; iterate for "all blocks".
- `game/engine/src/item.rs::MaterialId` — derive an iterator (no built-in; need a small helper).
- `game/engine/src/crafting.rs` — `Tool::new(tool_type, material)` to construct every (ToolType, ToolMaterial) combo.
- `game/engine/src/inventory.rs` — read-only access to slot contents for the survival "you have N" overlay.
- `game/engine/src/player_slot.rs::PlayerSlot` — new `open_explorer: bool` field.
- `game/engine/src/game_loop.rs` — `B` key handler + render hook + click handlers.
- `game/engine/src/input.rs` + `player_intent.rs` — new `toggle_inventory_explorer: bool` intent (separate from the existing `toggle_inventory` E-key which opens crafting).

### New modules

- `game/engine/src/inventory_explorer.rs` (~400 LOC) — `ExplorerState`, `enumerate_all_items()`, search + category filter logic, egui draw function, click outcome enum.

### Related specs

- [Spec 28 master](2026-05-20-minecraft-parity-content-surface.md) §6 — design discussion.
- Spec 5 §3.6 — egui rendering pitfalls. The explorer follows the layered-painter / Background-order overlay / pinned-width Middle-order panel pattern.

### Memory pointers

- uk english naming — UK English throughout ("Inventory", "Colour", "Defence", "Armour").
- shared infra strategy — `enumerate_all_items` + search filter + drag-to-hotbar are engine-generic patterns. Per-game flavours (which items exist) come for free.

---

## Phasing

| # | Phase | Files | LOC | Solo? |
|---|---|---|---|---|
| 1 | **This spec** | this doc | — | — |
| 2 | `inventory_explorer.rs` skeleton — `ExplorerState`, `ExplorerEntry`, `ExplorerCategory` enum, `enumerate_all_items() -> Vec<ExplorerEntry>` pure function | new `inventory_explorer.rs` | ~150 | ✓ |
| 3 | Filter logic — search (case-insensitive substring on display name) + category radio. Pure function `filter_entries(entries, query, category) -> Vec<&ExplorerEntry>`. | `inventory_explorer.rs` | ~80 | ✓ |
| 4 | egui draw — modal overlay, search field, category buttons, scrollable icon grid with hover tooltips. Returns `ExplorerClickOutcome`. | `inventory_explorer.rs` | ~250 | ✓ |
| 5 | Wire-up — `B` key intent, `PlayerSlot.open_explorer` field, render hook in `game_loop.rs`, menu-open gate. Esc closes. | `input.rs`, `player_intent.rs`, `player_slot.rs`, `game_loop.rs` | ~120 | ✓ |
| 6 | Click handlers — Creative: `/give 1 of clicked entry` to hotbar. Survival: tooltip shows "You have: N" with N counted from `inventory.slots_iter()`. | `game_loop.rs`, `inventory.rs` (read-only helper) | ~80 | ✓ |
| 7 | Tests — `enumerate_all_items` count, filter substring match, filter category exclusion, ExplorerCategory exhaustiveness. | `inventory_explorer.rs::tests` | ~80 | ✓ |
| 8 | Axolittle playtest — open explorer with B, search "iron" → see iron ore + ingot + tools + block; click a Diamond Pickaxe in creative → it's in your hotbar; in survival open explorer → confirm "you have N" overlay shows correct counts. | — | — | playtest gate |

**Total Phases 2-7:** ~760 LOC. Phase 8 is the playtest gate.

---

## §2 — Skeleton

```rust
pub struct ExplorerEntry {
    pub label: String,             // "Iron Pickaxe", "Cooked Beef", "Oak Planks"
    pub category: ExplorerCategory,
    pub source: ExplorerSource,    // discriminated for click handling
    pub tooltip_lines: Vec<String>,
}

pub enum ExplorerCategory {
    Blocks,
    Tools,
    Materials,
    Plans,
}

pub enum ExplorerSource {
    Block(BlockId),
    Material(MaterialId),
    Tool(ToolType, ToolMaterial),
    Plan,  // Plans are per-instance; the explorer only shows "this is a plan slot"
}

pub fn enumerate_all_items(registry: &BlockRegistry) -> Vec<ExplorerEntry> {
    let mut entries = Vec::new();
    // Blocks
    for (id, def) in registry.blocks.iter().enumerate() {
        if def.name == "genesis:air" { continue; }
        entries.push(ExplorerEntry {
            label: human_name_for_block(id as BlockId),
            category: ExplorerCategory::Blocks,
            source: ExplorerSource::Block(id as BlockId),
            tooltip_lines: vec![format!("Block id {}", id), def.name.to_string()],
        });
    }
    // Materials — enumerate via a const-table since MaterialId has no derive(Iter)
    for m in ALL_MATERIAL_IDS {
        entries.push(...);
    }
    // Tools — Cartesian product
    for tool_type in ALL_TOOL_TYPES {
        for tool_material in compatible_materials_for(tool_type) {
            entries.push(...);
        }
    }
    entries
}
```

`ALL_MATERIAL_IDS` is a const slice of every `MaterialId` variant — generated by hand on alpha (~50 entries) and locked by a test that compares the count to `mem::variant_count::<MaterialId>()` (or, since that's unstable, a manual count assertion).

### Acceptance

- `enumerate_all_items` returns one entry per registered block (skipping AIR), one per MaterialId, and one per valid (ToolType, ToolMaterial) combo (~30 tool combos).
- Total entry count test ≈ 65 blocks + 44 materials + 30 tools = ~139 entries.

---

## §3 — Filter logic

```rust
pub fn filter_entries<'a>(
    entries: &'a [ExplorerEntry],
    query: &str,
    category: Option<ExplorerCategory>,
) -> Vec<&'a ExplorerEntry> {
    let needle = query.to_lowercase();
    entries.iter()
        .filter(|e| category.map_or(true, |c| e.category == c))
        .filter(|e| needle.is_empty() || e.label.to_lowercase().contains(&needle))
        .collect()
}
```

### Acceptance

- Empty query + None category → all entries returned.
- Query "iron" → matches "Iron Pickaxe", "Iron Ore", "Iron Ingot", "Iron Block".
- Category Blocks → only entries with `category == Blocks`.

---

## §4 — egui draw

Standard Spec 5 §3.6 pattern:
- Background-order overlay (dims the world).
- Middle-order pinned-width panel.
- Search input field with `egui::TextEdit::singleline`.
- Category radio buttons (All / Blocks / Tools / Materials).
- Scrollable grid via `egui::Grid` + `egui::ScrollArea::vertical`.
- Each entry: icon + label. Hover shows tooltip.
- Returns `ExplorerClickOutcome::ItemClicked(usize)` (index into filtered) or `Closed`.

### Acceptance

- Modal renders centred in viewport.
- Search field has keyboard focus when explorer opens.
- Esc closes.

---

## §5 — Wire-up

- `PlayerIntent::toggle_inventory_explorer: bool` (new field).
- `InputState::key_pressed(KeyCode::KeyB)` sets it.
- `game_loop` per-frame: if intent.toggle_inventory_explorer && !menu_open → flip `players[pidx].open_explorer`. Cursor releases on open + recaptures on close.
- `menu_open` gate includes `open_explorer`.
- Render hook in the dialog-render section, between plaque and inspect (or another sensible slot).

### Acceptance

- B toggles the explorer.
- Movement/break/place suppressed while explorer is open.
- Esc closes; cursor re-captured.

---

## §6 — Click handlers

- **Creative**: clicked entry → `inventory.add_item(stack_from_source(source))`. Toast "Added {label} to inventory."
- **Survival**: clicked entry → tooltip stays open with extra line "You have: N". Count via `inventory.slots_iter()` matching the entry's source.

For Plans: skip — they're per-instance and don't have a "default exemplar" that the explorer can spawn.

### Acceptance

- Creative click on a block entry puts one of that block in the player's hotbar/inventory.
- Survival click highlights "You have: N" — non-mutating.

---

## §7 — Tests

```rust
#[test]
fn enumerate_returns_at_least_current_content() {
    let registry = BlockRegistry::new();
    let entries = enumerate_all_items(&registry);
    // 65 blocks (registry length minus AIR) + 44 MaterialIds + ~30 valid (ToolType, ToolMaterial) combos
    assert!(entries.len() >= 130);
}

#[test]
fn filter_search_is_case_insensitive() {
    let registry = BlockRegistry::new();
    let entries = enumerate_all_items(&registry);
    let lo = filter_entries(&entries, "iron", None);
    let up = filter_entries(&entries, "IRON", None);
    assert_eq!(lo.len(), up.len());
    assert!(lo.iter().any(|e| e.label.contains("Iron Pickaxe")));
}

#[test]
fn filter_category_restricts_correctly() {
    let registry = BlockRegistry::new();
    let entries = enumerate_all_items(&registry);
    let only_blocks = filter_entries(&entries, "", Some(ExplorerCategory::Blocks));
    assert!(only_blocks.iter().all(|e| e.category == ExplorerCategory::Blocks));
}

#[test]
fn empty_query_returns_all_in_category() {
    let registry = BlockRegistry::new();
    let entries = enumerate_all_items(&registry);
    let only_tools = filter_entries(&entries, "", Some(ExplorerCategory::Tools));
    // Tools = (5 tool tiers × 5 standard tool types) + Bow (wood) + FlintAndSteel (iron) ≈ 27
    assert!(only_tools.len() >= 25 && only_tools.len() <= 35);
}
```

---

## §8 — Axolittle playtest

Run-through:

1. Press B → explorer opens, search field has focus.
2. Type "iron" → list filters to iron-related items.
3. Click All → list resets to all categories.
4. Click Tools → list narrows to tools only.
5. (Creative) Click an Iron Pickaxe in the list → check hotbar, it's there.
6. Close (Esc) → explorer closes, cursor recaptured.
7. (Survival) Open explorer with B → hover over Cobblestone → tooltip shows "You have: N" where N matches the inventory count.

Questions:
- Does B feel like the right key, or is it stepping on something?
- Is the modal too big / too small?
- Is the search responsive enough (no lag while typing)?
- Should categories be tabs instead of buttons?
- Should the explorer remember the last search/category between sessions? (Probably yes — small state on PlayerSlot.)

---

## Acceptance — sub-foundation 28f overall

- `./check.sh` ALL GREEN.
- All Phase 2-7 tests pass.
- Manual: explorer opens with B, search works, category filters work, creative spawns items, survival shows counts.
- Foundations README + Player Guide updated when delivered.

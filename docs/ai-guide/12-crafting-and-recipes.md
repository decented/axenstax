<!-- SOURCE: game/engine/src/crafting.rs (match_recipe — the recipe matcher, source of truth), craft_ui.rs (2x2 vs 3x3 grid), crafting_catalogue.rs (recipe-book cards + consistency test), recipe_book_ui.rs (book panel), workstation.rs (workstation framework), furnace.rs (smelting), campfire.rs (cooking + fuel ladder), item.rs | Verified against code 2026-06-22 -->

# 12 — Crafting & Recipes

**Purpose:** the verified facts about how crafting and smelting work, plus a representative, code-traced set of real recipes. These are EXAMPLES — the full list lives in the in-game recipe book. Never invent a recipe or import one from Minecraft; if it isn't traceable in `match_recipe`, treat it as not craftable.

> **The matcher is the source of truth.** Every recipe is decided by one function, `crafting::match_recipe`, which reads the 3×3 grid and returns the output. The recipe book (`crafting_catalogue.rs`) is a list of cards that **delegate to the matcher** — a test (`every_card_matches_the_live_matcher`) feeds each card's grid back through `match_recipe` and fails the build if a card disagrees. So a card can never promise something the engine wouldn't actually craft.

---

## 1. How crafting works

### The grid
- Crafting uses a **3×3 grid** internally. There are two ways to reach it:
  - **Press `E`** (open inventory) → a **2×2 crafting grid** (only the top-left 2×2 cells are usable). This is the always-available "hand crafting" — enough for planks, sticks, a crafting table, torches, simple tools.
  - **Right-click a placed Crafting Table block** → the **full 3×3 grid**. Needed for anything bigger than 2×2 (most tools, furnaces, armour, beds, chests, etc.).
- So the rule of thumb: *small recipes anywhere; big recipes need a Crafting Table.*

### Shaped vs shapeless
- Most recipes are **shaped** — the items must be in the right pattern (e.g. a pickaxe is a row of 3 heads with two sticks straight down the middle).
- The grid is **position-independent within itself**: the matcher trims empty rows/columns to a bounding box first, so a recipe works wherever you place it in the grid (top-left, centre, etc.) as long as the *shape* is right.
- A few recipes are **shapeless / order-independent** — the inputs can be in any arrangement in their row (e.g. the Black Powder mix and some dye mixes). The Salt-curing recipes accept the salt on either side.

### The recipe book (auto-fill helper)
- Open the recipe book from the crafting screen (mouse/touch click, or **gamepad Y**). It shows cards grouped into tabs: **Tools, Combat, Building, Stations, Decoration, Food, Materials, Transport**. You can search by name, or ask "what can I make with X".
- **The book never crafts for you.** Selecting a card **auto-fills the grid** with that recipe's example layout (pulling the exact items from your inventory). The real `match_recipe` then produces the result, exactly as if you'd laid it out by hand. If you don't have the ingredients, it can't fill.
- Use the book to *discover* and *lay out* recipes; it is the complete, always-up-to-date list. This page only spot-checks a cross-section.

---

## 2. Example recipes (verified from the matcher)

These are confirmed in `match_recipe`. **`M` = the tool/armour material tier** (see §2.4). Output counts are exact.

### 2.1 Wood, basics, and your first crafting table
| Recipe (shape) | Output |
|---|---|
| 1 Log (any wood) | **4 Planks** |
| 2 Planks stacked vertically | **4 Sticks** |
| 4 Planks in a 2×2 | **1 Crafting Table** |
| 4 Sticks in a 2×2 | **1 Drying Rack** |
| Coal on top of a Stick (vertical) | **4 Torches** |
| 1 Stone (single cell) | **1 Button** |
| 2 Stone in a row | **1 Pressure Plate** |
| Stick on top of Cobblestone (vertical) | **1 Lever** |

*Notes:* a "log" can be the old oak-log block or the newer log materials (Green/Seasoned/Kiln-Dried) — all give 4 oak planks; specific species logs give that species' planks.

### 2.2 Tools (the standard shapes)
The tool head material `M` sits on top; **sticks** form the handle. Material tiers: **Planks = Wood, Cobblestone = Stone, Iron Ingot = Iron, Diamond = Diamond, Satori = Satori** (top tier).

| Tool | Shape (M = head, S = Stick) | Output |
|---|---|---|
| **Pickaxe** | `M M M` / `· S ·` / `· S ·` | 1 Pickaxe (of that tier) |
| **Axe** | `M M` / `M S` / `· S` (mirror works too) | 1 Axe |
| **Shovel** | `M` / `S` / `S` (1-wide column) | 1 Shovel |
| **Sword** | `M` / `M` / `S` (1-wide column) | 1 Sword |
| **Hoe** | `M M` / `· S` / `· S` (mirror works too) | 1 Hoe |

So a **Wooden Pickaxe** = 3 Planks across the top + 2 Sticks down the middle. Swap the planks for Cobblestone/Iron Ingot/Diamond/Satori to make the higher tiers.

### 2.3 Other verified tools & weapons
| Recipe | Output |
|---|---|
| Flint on top of an Iron Ingot (vertical) | **1 Flint and Steel** |
| Two Iron Ingots stacked vertically | **1 Shears** |
| Bow curve: `· S Str` / `S · Str` / `· S Str` (S = Stick, Str = String; centre cell empty = Wood Bow, or Cobble/Iron/Diamond/Satori for higher tiers) | **1 Bow** |
| Stick on top of a Feather (vertical) | **4 Arrows** |
| Fishing Rod (sticks on a diagonal + string down the right column) | **1 Fishing Rod** |

### 2.4 Armour (worn for protection)
Armour uses a material `M` from: **Leather, Iron Ingot, Diamond, Satori** (and **Rubber** for boots only). Same material in every filled cell.
| Piece | Shape | Output |
|---|---|---|
| **Helmet** | `M M M` / `M · M` | 1 Helmet |
| **Chestplate** | `M · M` / `M M M` / `M M M` | 1 Chestplate |
| **Leggings** | `M M M` / `M · M` / `M · M` | 1 Leggings |
| **Boots** | `M · M` / `M · M` | 1 Boots |

### 2.5 Building & storage blocks
| Recipe | Output |
|---|---|
| 3 Stone in a row | **6 Stone Slabs** |
| 6 Stone in a staircase (either direction) | **4 Stone Stairs** |
| 4 Sand in a 2×2 | **4 Glass** |
| 6 Glass filling a 2×3 | **16 Glass Panes** |
| 6 Iron Ingots filling a 2×3 | **16 Iron Bars** |
| 8 Planks ringing an empty centre (any wood) | **1 Chest** |
| 9 Coal / 9 Raw Iron / 9 Diamond / 9 Satori (full 3×3, all same) | **1 Coal/Iron/Diamond/Satori storage Block** (reverses 1→9) |

*Fences & gates (oak shown):* Fence Post = `P S P` / `P S P`; Fence Gate = `S P S` / `S P S`; Trapdoor = 6 planks in a 2×3 → 2 Trapdoors. (`P` = Planks, `S` = Stick.)

### 2.6 Workstation & utility blocks
| Recipe | Output |
|---|---|
| 8 Cobblestone in a ring, empty centre (3×3) | **1 Furnace** |
| 3 Sticks / 3 Logs / 3 Sticks (3×3 rows) | **1 Campfire** (unlit — you light it after) |
| 4 Sticks in a 2×2 | **1 Drying Rack** |
| Paper + 3 Planks in a 2×2 | **1 Drafting Table** |
| 5 Iron Ingots in a V around a Chest (3×3) | **1 Hopper** |

### 2.7 A few foods
| Recipe | Output |
|---|---|
| 3 Wheat in a row | **1 Bread** |
| Salt + a raw meat (1×2, either order) | **Salt-Cured** version of that meat (e.g. Salt + Raw Beef → Salt-Cured Beef) |

> Most food is **cooked**, not crafted — see smelting/cooking below. The recipe book's **Food** tab is the complete food list.

---

## 3. Smelting & cooking

There are **two** heat stations, and they do different jobs:

### 3.1 Furnace — smelts ORE into INGOTS
- Craft a Furnace (8 cobblestone ring). Place it, then open it: an **input slot**, a **fuel slot**, and an **output slot**.
- Drop ore in the input and **fuel** in the fuel slot. It burns fuel to make progress; one smelt takes ~200 ticks (about 10 seconds) of burning.
- **Verified furnace smelts:** Raw Iron → **Iron Ingot**; Copper → **Copper Ingot**; Tin → **Tin Ingot**. *(Those raw ore items are named "Raw Iron", "Copper", and "Tin" in your inventory — not "Raw Copper"/"Raw Tin". That's the whole furnace recipe list right now — the furnace does ore, not food.)*

### 3.2 Campfire — cooks FOOD
- Place a (lit) Campfire and put raw food on it. **Verified cooking:** Raw Beef → Cooked Beef; Raw Porkchop → Cooked Porkchop; Raw Chicken → Cooked Chicken; Raw Mutton → Cooked Mutton; Raw Fish → Cooked Fish; Potato → Baked Potato; Carrot → Baked Carrot; Corn → Corn on the Cob.
- **Cooked food restores more hunger and is safer than raw** — cooking is the main way to make meat worth eating.

### 3.3 Fuel (works in both Furnace and Campfire)
The fuel ladder is shared. Burn time (longer = better fuel):
| Fuel | Burn time |
|---|---|
| Oak Leaves | ~1 s (fastest kindling) |
| Stick | ~2 s |
| Planks | ~12 s |
| Green Log | ~30 s |
| Oak Log / Seasoned Log | ~60 s |
| Kiln-Dried Log | ~120 s |
| **Coal** | **~240 s (best common fuel)** |

So **Coal** is the most efficient everyday fuel; logs and planks work in a pinch.

---

## 4. Workstations (which ones actually exist)

Live, working stations you can craft and use:
- **Crafting Table** — the 3×3 crafting grid (see §1).
- **Furnace** — smelts ore → ingots (§3.1).
- **Campfire** — cooks/bakes food (§3.2).
- **Drying Rack** — passive station (4 sticks) for seasoning/drying over time.
- **Drafting Table** — for blueprints/schematics (see the building/Workshop pages).
- **Vendor Block** — a player-run shop block (Sell/Barter — see the economy page).

There is a generic workstation framework in the code that also names **Mill, Oven, and Aging Rack** — these are **scaffolding for future tiers, not live stations**. There's no working way to mill flour, bake in an Oven, or age food in normal play yet. Don't send a player to use a Mill/Oven/Aging Rack.

---

## Deferred / not yet (do NOT present as available)
- **Flour → so Cake / Pancakes / Nostrich Omelette are unreachable.** A Nostrich Omelette recipe (Egg + Flour + Wheat) exists in the matcher, **but there is no way to make or obtain Flour in normal play** (no Mill is live; Flour only appears via the creative item explorer). So **treat Flour, Cake, Pancakes, and the Omelette as NOT craftable** — they're gated out of quests. Don't send a player to craft them.
- **Beetroot Soup** and similar multi-ingredient cooked dishes — not built; not craftable.
- **Mill / Oven / Aging Rack** — framework only, no live recipes (see §4). The processed-food tier they belong to is future work.
- **Recycling armour/tools back into materials** — not a thing; armour and tools can't be used as crafting ingredients.
- **Charcoal** — referenced in code as a *future* fuel but not yet a real material; don't promise it.
- **Anything not in `match_recipe` / the in-game recipe book.** If you can't find it in the book, it isn't craftable — say so kindly and offer what *is*.

### A note for the assistant
When a player asks "how do I craft X", the safest move is: name the recipe from this page if it's here, otherwise point them at the **in-game recipe book** (it's the full, live list and it lays the recipe out for them). Never reconstruct a recipe from memory or from Minecraft — the shapes here are deliberately AxeNStax's own and a wrong recipe just frustrates a kid who then can't make the thing.

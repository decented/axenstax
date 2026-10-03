# Crafting

Crafting is how you turn the stuff you mine and gather into **tools, blocks, food, and gear**. You place items into a crafting grid in the right shape, then click the result to take it.

This page tells you **what every live recipe is**. If you want a step-by-step walkthrough of your very first crafts, follow [Make your first workbench](https://learn.axenstax.com/docs/journey/learn-journey/first-workbench.md).

## The two grids

There are **two** crafting grids, and they share one recipe matcher.

| Grid | Where | What fits |
|---|---|---|
| **2×2 inventory grid** | Always there — open your inventory (**E**). Only the top-left 2×2 is usable. | Any recipe whose shape fits in a 2×2 square. |
| **3×3 Workbench grid** | Right-click a **Crafting Table** block. The window is titled **Workbench**. | Everything — including the 2×2 recipes. |

A recipe works in the small grid **only if its shape is 2×2 or smaller**. Bigger recipes (like a pickaxe) need the Workbench.

### Shaped vs shapeless

Most recipes are **shaped** — the items have to sit in the right pattern relative to each other. The good news: only the *shape* matters, not *where* in the grid you put it. A torch works in any corner, as long as the coal is directly above the stick.

A few recipes are **shapeless** — you just need the right items, in any slots, any order. These are: the salt-cure pairs, magnesium, dye mixes, and dyed-paper pairs.

In every grid below, `[ ]` means an **empty cell**. The legend under each grid tells you what the letters mean.

## The gateway chain (start here)

This is the bootstrap. With nothing but a log and the 2×2 inventory grid, you can build your way up to a Workbench.

```
Planks (1×1 → ×4)
[L]
L = any Log
```

One log becomes **4 Planks**. (Each tree species gives its own colour of planks. The neutral Green Log gives Oak planks.)

```
Sticks (1×2 → ×4)
[P]
[P]
P = Oak Planks only
```

Two planks stacked make **4 Sticks**.

```
Crafting Table (2×2 → ×1)
[P][P]
[P][P]
P = Oak Planks only
```

Four planks filling the 2×2 make a **Crafting Table**. You can make this in the inventory grid — that's the whole point. Place it down, right-click it, and the 3×3 Workbench opens.

> The whole early bootstrap chain — Sticks, Crafting Table, and Wood-tier tools — checks for **Oak Planks specifically**, not any species. If your first log wasn't a neutral Green Log (which gives Oak planks), chop an oak tree before you start.

> Full walkthrough: [Make your first workbench](https://learn.axenstax.com/docs/journey/learn-journey/first-workbench.md).

## Basic blocks & light

```
Glass (2×2 → ×4)
[Sa][Sa]
[Sa][Sa]
Sa = Sand
```

```
Torch (1×2 → ×4)
[Co]
[S]
Co = Coal   S = Stick
```

## Tools

Most tools share a pattern: a **head material** sits on a **stick handle**. Swap the head material for a higher tier — the shape stays the same.

> **M = head material:** Planks (Wood) → Cobblestone (Stone) → Iron Ingot → Diamond → Satori.
> **S = Stick.** All of these are 3×3 (Workbench) recipes and make **×1**.

For durability, damage, and mining-speed numbers, see [Tools Reference](tools-reference.md).

```
Pickaxe
[M][M][M]
[ ][S][ ]
[ ][S][ ]
```

```
Axe (heads make an L)
[M][M]
[M][S]
[ ][S]
```

```
Sword
[M]
[M]
[S]
```

```
Shovel
[M]
[S]
[S]
```

```
Hoe
[M][M]
[ ][S]
[ ][S]
```

```
Bow (centre R sets the tier)
[ ][S][St]
[S][R ][St]
[ ][S][St]
S = Stick   St = String
R = empty→Wood, Cobblestone→Stone, Iron Ingot→Iron, Diamond→Diamond, Satori→Satori
```

### Single-tier tools

These don't have a material ladder — there's just one version of each.

```
Flint & Steel (1×2 → ×1)
[Fl]
[I ]
Fl = Flint (on top)   I = Iron Ingot (bottom)
```

✅ Flint **on top**, Iron **below** — a straight up-and-down column. (Not a diagonal, not "any arrangement".)

```
Shears (1×2 → ×1)
[I]
[I]
I = Iron Ingot
```

✅ Two Iron Ingots **stacked in one column**. (Not a diagonal.)

```
Fishing Rod (Wood-tier only, 3×3 → ×1)
[ ][ ][S ]
[ ][S][St]
[S][ ][St]
S = Stick   St = String
```

> **Coming soon:** the Fishing Rod crafts now, but casting and catching fish aren't live yet.

```
Slingshot (3×3 → ×1)
[S][   ][S]
[ ][Ru ][ ]
[ ][S  ][ ]
S = Stick   Ru = Rubber
```

```
Eraser (1×2 → ×1)
[Ru]
[S ]
Ru = Rubber (on top)   S = Stick (bottom)
```

```
Drafting Stamp (1×2 → ×1, Wood tier)
[I ]
[Bp]
I = Iron Ingot (top)   Bp = Blueprint Paper (bottom)
```

All twelve tool types exist: **Pickaxe, Axe, Sword, Shovel, Hoe, Bow, Flint & Steel, Shears, Fishing Rod, Slingshot, Eraser, Drafting Stamp**.

## Storage & compressed blocks

Pack nine of a material into a 3×3 to squeeze it into a block — handy for storage and trading. Every one of these is **reversible**: put the block back in a 1×1 to get your materials out again.

```
Compressed block (3×3 → ×1)
[X][X][X]
[X][X][X]
[X][X][X]
X = Coal / Raw Iron / Diamond / Satori / Bone / Wheat / Salt
```

- 9 Coal → **Coal Block**
- 9 Raw Iron → **Iron Block** (raw ore, not the smelted Iron Ingot)
- 9 Diamond → **Diamond Block**
- 9 Satori → **Satori Block**
- 9 Bone → **Bone Block**
- 9 Wheat → **Hay Bale**
- 9 Salt → **Salt Block**

```
Amethyst Block (2×2 → ×1)
[A][A]
[A][A]
A = Amethyst
```

Put any of these blocks in a 1×1 to get back its 9 materials (or 4 for Amethyst). See [Storage Blocks](storage-blocks.md).

## Workstations

Most workstations are an **8-cell ring of Planks or Cobblestone** with something in the middle. All are 3×3 → ×1.

```
Furnace
[Cb][Cb][Cb]
[Cb][  ][Cb]
[Cb][Cb][Cb]
Cb = Cobblestone (empty centre)
```

```
Chest
[P][P][P]
[P][ ][P]
[P][P][P]
P = Planks (any species, empty centre)
```

```
Vendor Block
[P][P][P]
[P][I][P]
[P][P][P]
P = Planks   I = Iron Ingot
```

```
Bazaar Block
[P][P][P]
[P][D][P]
[P][P][P]
P = Planks   D = Diamond
```

```
Auction Block
[I][P][I]
[P][P][P]
[I][P][I]
I = Iron Ingot   P = Planks
```

```
Plot Marker
[I][ ][I]
[ ][P][ ]
[I][ ][I]
I = Iron Ingot   P = Planks
```

```
Tip Jar (two Iron stacked in the centre column)
[P][I][P]
[P][I][P]
[P][P][P]
P = Planks   I = Iron Ingot
```

```
Bounty Board
[P][P ][P]
[P][Pa][P]
[P][P ][P]
P = Planks   Pa = Papyrus Sheet
```

```
Repair Bench
[I ][I ][I ]
[  ][St][  ]
[St][St][St]
I = Iron Ingot   St = Stone
```

```
Drying Rack (2×2)
[S][S]
[S][S]
S = Stick
```

```
Drafting Table (2×2)
[Pa][P]
[P ][P]
Pa = Papyrus Sheet   P = Planks
```

```
Campfire (unlit)
[S][S][S]
[L][L][L]
[S][S][S]
S = Stick   L = Log
```

```
Village Bell (1×3 column)
[I]
[S]
[P]
I = Iron Ingot   S = Stick   P = Plank
```

```
Market Bell (1×3 column)
[I]
[I]
[P]
I = Iron Ingot   P = Plank
```

The Furnace doesn't craft food — it **smelts** it. Smelting works differently from crafting (input + fuel slots, not a grid); see [Smelting](smelting.md). The Village Bell claims a village zone — see [Village Bell](village-bell.md).

## Food

```
Bread (1×3 → ×1)
[W][W][W]
W = Wheat
```

```
Nostrich Omelette (1×3 → ×1)
[E][Fl][W]
E = Nostrich Egg   Fl = Flour   W = Wheat
```

```
Salt-cured meat (1×2, any order → ×1)
[Sa][Rm]
Sa = Salt   Rm = any raw meat
```

```
Seasoned cooked food (1×2, any order → ×1)
[Sa][Cf]
Sa = Salt   Cf = any cooked/baked item
```

Turning **raw** meat into **cooked** meat happens at the Campfire, not in a grid — see [Food & Cooking](food-and-cooking.md).

## Fibre & cordage

```
String (1×1 → ×2)
[Ct]
Ct = Cotton
```

```
Rope (1×3 column → ×1)
[H]
[H]
[H]
H = Hemp Fibre
```

```
Lead (1×3 column → ×1)
[Ro]
[St]
[St]
Ro = Rope   St = String
```

```
Cloth (2×2 → ×1)
[Ct][Ct]
[Ct][Ct]
Ct = Cotton
```

```
Canvas (2×2 → ×1)
[H][H]
[H][H]
H = Hemp Fibre
```

```
Bed (2×3 → ×1)
[Wo][Wo][Wo]
[P ][P ][P ]
Wo = Wool   P = Planks
```

```
Tent (2×3 → ×1)
[Cn][Cn][Cn]
[S ][  ][S ]
Cn = Canvas   S = Stick
```

## Arrows & misc

```
Arrow (1×2 → ×4)
[S]
[Fe]
S = Stick   Fe = Feather
```

```
Nostrich Arrow (1×3 column → ×6)
[S ]
[Fl]
[Nf]
S = Stick   Fl = Flint   Nf = Nostrich Feather
```

```
Rubber Ball (1×1 → ×4)
[Ru]
Ru = Rubber
```

```
Bonemeal (1×1 → ×3)
[Bo]
Bo = Bone
```

```
Bronze Ingot (1×2 → ×1)
[Cu]
[Tn]
Cu = Copper Ingot   Tn = Tin Ingot
```

```
Papyrus Sheet (1×3 → ×3)
[Pr][Pr][Pr]
Pr = Papyrus Reed
```

```
Blueprint Paper (1×3 column → ×3)
[Pa]
[I ]
[Sa]
Pa = Papyrus Sheet   I = Iron Ingot   Sa = Salt
```

## Armour

Armour follows the head-material idea too. **M = Leather, Iron Ingot, Diamond, or Satori** (Rubber makes **Boots only**). Chainmail can't be crafted — it's a drop. Full stats and the protection table are in [Armour](armour.md); the four shapes:

```
Helmet (2×3)
[M][M][M]
[M][ ][M]
M = armour material
```

```
Chestplate (3×3)
[M][ ][M]
[M][M][M]
[M][M][M]
```

```
Leggings (3×3)
[M][M][M]
[M][ ][M]
[M][ ][M]
```

```
Boots (2×3)
[M][ ][M]
[M][ ][M]
```

## Dyes & décor

This is a big system on its own — flowers and other items become dyes, dyes mix into more colours, and dyes paint a whole range of décor (Wallpaper, Bunting, Paper Lanterns, Kites, Banners, Sails) in 16 colours. The quick version:

- **Flowers → dye:** Cornflower → Blue, Field Poppy → Red, Buttercup → Yellow (1×1 each).
- **Ink Sac → Black**, **Bonemeal → White**.
- **Two dyes side by side (1×2, ×2)** mix into Orange, Green, Purple, Pink, Lime, Light Blue, Grey, Light Grey, Cyan, and Magenta.

For the full colour chart and every décor recipe, see [Dyes, Fibre & Magnesium](dyes-fibre-and-magnesium.md).

## Tips

- Keep **planks and sticks** in your bag — they're the backbone of nearly every recipe.
- A **spare Crafting Table** lets you craft anywhere: place it, use it, mine it back.
- Only *shape* matters for shaped recipes, so a recipe works in any corner of the grid.
- The result only crafts when you **click it** — pick it up each time you want another.

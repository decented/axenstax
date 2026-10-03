# Storage & Workstation Blocks

A quick tour of the placeable blocks that **hold things** in
Axe'n'Stax — what they look like, where the inputs and outputs go,
and which materials they handle.

## Furnace

Smelts **ore** into ingots (iron, copper, tin). Needs **fuel** (any
log or coal) in the bottom slot and **input** (ore) in the top slot.
Output appears in the output slot after the smelt timer. Raw meats
aren't smelted here — cook those at the **Campfire** instead (see
[Food & Cooking](food-and-cooking.md)).

| Slot | What goes there |
|---|---|
| Top | The ore to smelt (iron ore → iron ingot, copper ore → copper ingot, tin ore → tin ingot) |
| Bottom | Fuel — leaves (1s), sticks (2s), planks (12s), green log (30s), logs (60s), seasoned log (60s), kiln-dried log (120s), coal (240s) |
| Output | The smelt result. Pick it up to clear the slot. |

A lit furnace shows flames + a slight glow; an unlit one is grey.
Right-click to open the dialog.

## Drying Rack

Wave-29 log seasoning. Place green logs (the new tree-mine drop) on
the rack and they slowly mature into **seasoned logs** over ~5 real
minutes per slot. Seasoned logs make better tools.

8 slots per rack. The block above must be air for the slot to mature
(seasoning needs airflow).

## Mill, Oven & Aging Rack (not yet functional)

**Mill**, **Oven**, and **Aging Rack** are placeable now — you can craft
them, put them down, and they'll even get captured by a build
schematic — but they don't do anything yet. No recipes work at them,
there's no grind/bake/age timer, and nothing you put in comes back
out. Think of them as reserved spots for a future farming-depth
update (Mill grinding crops into intermediates, Oven baking multi-
input goods, Aging Rack doing slow "come back later" recipes like
milk → cheese). Until that lands, keep cooking at the **Campfire** and
smelting at the **Furnace**.

## Vendor Block

Player-run shop. Right-click an unowned Vendor to claim it; you can
list one item at a time per Vendor. Modes:

- **Sell** — buyer pays sats for your item
- **Buy** — you pay sats for buyer's item
- **Barter** — item-for-item; no sats; always available
- **Bulk** — sell your item in fixed lots of **8, 16, 32 or 64** for sats. Pick Bulk in the mode picker, then choose a lot size; the buyer pays for the whole lot in one click (lot size × your per-unit price). Handy for shifting big stacks of cobblestone or planks — price each unit a little cheaper than singles so buyers get a bargain. If your stock drops below one full lot, the Buy button greys out until you restock.

Buyer right-clicks to open the dialog and complete the trade. If you
break someone else's Vendor by accident you get a toast and nothing
else happens (anti-grief); the owner can break their own to retrieve
the listed item + escrowed sats.

## Bee Hive (chunk-8)

Stores up to **5 honey levels** + **3 bees** sheltered. Bees go out
to pollinate crops and come back, raising the honey level. Drain a
full hive:

| Tool | Output |
|---|---|
| Bucket | 1 Honey Bottle (–1 honey) |
| Shears | 3 Honeycomb (–1 honey, –1 shear durability) |
| Empty hand | Nothing — just inspects. Don't anger the bees. |

The hive only gives something while honey ≥ 1.

## Chest

Right-click to open. (Chest UI ships with the inventory-explorer
surface; same drag-and-drop rules as the player inventory.) There are
five chest tiers, each its own block with more room than the last:

| Tier | Slots | Auto-collect nearby drops? |
|---|---:|---|
| Wood | 27 | No |
| Copper | 36 | No |
| Iron | 45 | No |
| Diamond | 54 | Yes |
| Satori | 72 | Yes |

The top two tiers — **Diamond** and **Satori** — automatically pull in
dropped items from nearby, so a Diamond or Satori chest by your
furnace or farm scoops up drops without you having to walk over and
pick them up.

## Plaque

Not a storage block — a **memorial**. The Build Schematics system
places one when you complete a build from a plan; it records the
plan's architect chain so anyone right-clicking the plaque can see
who designed the original. See `build-schematics.md`.

## Workbench

The 3×3 crafting surface. You'll see this everywhere — it's the
gateway block. Place planks in a 2×2 in your inventory to make one,
or pick one up from a village house's furniture set. (Older builds
called it the Crafting Table.)

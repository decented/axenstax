# Smelting (the Furnace)

Smelting turns raw ore into shiny metal **ingots**. You do it in a **Furnace** — and here's the most important thing to get straight:

**The Furnace is NOT the crafting grid.** The crafting grid is where you arrange shapes to make things. The Furnace is a separate block that heats stuff up on its own. You don't place ore into a recipe shape — you load it into the Furnace and wait.

## Getting a Furnace

First you have to *craft* the Furnace block itself, in the crafting grid:

- **8 Cobblestone** in a ring, with the centre left empty.

(See [Crafting](crafting.md) for how to lay that out.)

Then:

1. **Place** the Furnace down like any block.
2. **Right-click** it to open it.
3. You'll see **three slots**: **Input**, **Fuel**, and **Output**.

## How it works

| Slot | What goes in it |
|------|-----------------|
| **Input** | The raw ore you want to smelt |
| **Fuel** | Something that burns (see the fuels table) |
| **Output** | The finished ingots appear here |

Load the **Input** with ore and the **Fuel** with something that burns, and the Furnace **lights itself**. The block flips to a glowing "lit" texture so you can see it's working. ✅

You do **not** need a **Flint & Steel** or any kind of match. The Furnace is self-lighting — that's different from the campfire, which you have to light by hand.

## What the Furnace smelts

The Furnace only smelts **ores into ingots**. That's the whole list:

| Input | Output |
|-------|--------|
| **Raw Iron** | **Iron Ingot** |
| **Copper** | **Copper Ingot** |
| **Tin** | **Tin Ingot** |

That's it. ⚠️ Raw meat does **not** go in the Furnace — food is cooked on the **Campfire** instead (see [Food and cooking](food-and-cooking.md)).

And **Bronze** is *not* smelted — you make it in the crafting grid from a **Copper Ingot** and a **Tin Ingot**. See [Crafting](crafting.md).

## How long it takes

- **10 seconds** per item (200 ticks — there are 20 ticks in a second).
- The **Output** slot holds up to **64** ingots.
- ✅ If your fuel runs out partway through smelting an item, the **progress is saved**. Pop more fuel in and it carries on from where it stopped — you don't lose that item.

## Fuels

The Furnace and the Campfire burn the **same fuels**. Each one burns for a set number of seconds, and since each item takes 10 seconds, you can work out how many items one bit of fuel will smelt:

| Fuel | Burns for | Items it smelts |
|------|-----------|-----------------|
| **Oak Leaves** | 1s | (not even one) |
| **Stick** | 2s | (not even one) |
| **Oak Planks** | 12s | 1 |
| **Green Log** | 30s | 3 |
| **Seasoned Log** | 60s | 6 |
| **Oak Log** | 60s | 6 |
| **Kiln-Dried Log** | 120s | 12 |
| **Coal** | 240s | 24 |

So **1 Coal** keeps the Furnace going for **240 seconds** — that's **24 items** smelted from a single lump. Coal is your best friend for a big smelting job.

## Smelting vs cooking vs crafting

These three are easy to mix up, so here's the difference side by side:

| | **Furnace** (smelting) | **Campfire** (cooking) | **Crafting grid** |
|---|---|---|---|
| What it does | Ores → ingots | Raw food → cooked food | Arranges shapes into items |
| Lights itself? | ✅ Yes, automatically | ⚠️ No — you light it by hand | Not a fire at all |
| How you light it | Just add fuel + input | **Flint & Steel**, a **Magnesium Firestarter**, or hold a **Stick** ~5s | — |
| Slots | 3 (Input / Fuel / Output) | 4 | 9-square grid |

- **Furnace** = metal. Self-lighting. Covered on this page.
- **Campfire** = food. You must light it yourself. Full details in [Food and cooking](food-and-cooking.md).
- **Crafting grid** = shapes and recipes (like the Furnace block itself, or Bronze). See [Crafting](crafting.md).

> **Coming soon:** more ores and metals to smelt as new blocks are added.

Want to try it hands-on? **[Cook food on a campfire](https://learn.axenstax.com/docs/journey/learn-journey/cook-on-a-campfire.md)** is the closest tutorial — it shows the fire-and-fuel idea in action.

## See also

- [Crafting](crafting.md) — make the Furnace block and craft Bronze
- [Food and cooking](food-and-cooking.md) — the Campfire, for cooking instead of smelting
- [Blocks and mining](blocks-and-mining.md) — where to dig up Raw Iron, Copper and Tin
- [Inventory and tools](inventory-and-tools.md) — managing what you've smelted

# Blocks & Mining

Everything in the world is made of blocks, and you mine them with tools. This page is your full block reference: what each one is, where to find it, how to get it, and what it's for.

New to all this? Start with the tutorial [Your first blocks](https://learn.axenstax.com/docs/journey/learn-journey/first-blocks.md), then come back here to look things up.

## How mining works

**Left-click and hold** on a block. A crack pattern grows across it. When the crack peaks, the block shatters and drops something you can pick up.

Three things decide how long that takes — and whether you get anything at all.

### Hardness → break time

Every block has a **hardness**: the number of seconds it takes to break bare-fisted with no tool. **Cobblestone** (hardness 2.0) takes about 2 seconds with your fist; **Stone** (hardness 10.0) takes ten. A worse tool still breaks the block — it just takes longer.

### Best tool → speed

Each block has a *best* tool. Use it and you break far faster:

| Block kind | Best tool |
|---|---|
| Stone, ore, deepslate, storage blocks | **Pickaxe** |
| Dirt, sand, gravel, snow | **Shovel** |
| Logs, planks, crafting table | **Axe** |

The better the tool material, the bigger the speed boost:

| Tool material | Speed |
|---|---|
| **Wood** | ×2 |
| **Stone** | ×4 |
| **Iron** | ×6 |
| **Diamond** | ×8 |
| **Satori** | ×9 |

Only the *matching* tool gives the speed boost. A diamond shovel won't mine stone any faster than your fist.

### Tool-tier gate → do you get a drop?

Some blocks are **tier-gated**: they drop **nothing** unless you mine them with a **pickaxe** of at least the right material.

| Tier | Material |
|---|---|
| 0 | Wood |
| 1 | Stone |
| 2 | Iron |
| 3 | Diamond |
| 4 | Satori |

If you swing a **wooden pickaxe** at **Iron Ore**, the ore breaks but **nothing drops** — the block, and your tool durability, are just gone. Wrong tool type (a shovel on ore) does the same: destroyed, no drop. Always carry the right tier.

> ⚠️ **In Creative mode** none of the timing or gating applies — every block breaks instantly in one click and always behaves as if you have the perfect tool.

### Where things live

| Marker | Y level |
|---|---|
| Sea level | **Y 62** |
| Caves carve through | **Y 2–54** |
| Deepslate begins | **Y 30** and below |
| World floor (bedrock) | **Y 0** |

## Natural & terrain blocks

| Block | What & where | How to get it | Drops |
|---|---|---|---|
| **Stone** | The bulk of the underground, everywhere below the dirt. | Pickaxe, Wood+ | **Cobblestone** (not itself) |
| **Cobblestone** | What stone gives you. The cheapest building + crafting block. | Pickaxe | Itself |
| **Dirt** | Below grass. | Shovel | Itself |
| **Grass** | The green surface of most biomes. Till it with a hoe to make **Tilled Soil** for crops. | Shovel | Itself |
| **Sand** | Deserts and beaches. **Falls** when nothing holds it up. | Shovel | Itself |
| **Sandstone** | Packed sand, found under desert sand. | Pickaxe (any tier) | Itself |
| **Gravel** | Patches around the world. **Falls** when unsupported. | Shovel | 15% chance of **Flint**, otherwise itself |
| **Snow** | Mountain tops above **Y 85** and the snowy tundra. | Shovel | Itself |
| **Bedrock** | The world floor at **Y 0**. | — | **Unbreakable**, even in Creative |
| **Water** | Oceans, caves, village wells, up to **Y 62**. You can swim and you can drown. | — | Non-solid — you swim through it |
| **Tall Grass** | Wispy grass on grassland. | Any tool / fist | Itself |
| **Glass** | See-through solid. Made by smelting sand. | Pickaxe | (see [Smelting](smelting.md)) |
| **Torch** | A placed light source (light level 14). | Any | (see [Crafting](crafting.md)) |
| **Bed** | Right-click at **night** to sleep, skip to morning, and set your spawn. | Any | (3 wool + 3 planks — see [Crafting](crafting.md)) |

> ⚠️ Watch your head under gravel and sand — break the bottom of a tall column and the whole thing drops on you.

## Ores & deepslate

Ores are buried treasure inside stone. Each is **tier-gated** — bring the right pickaxe or you get nothing.

### Ores in stone

| Ore | Where | Pickaxe needed | Drops |
|---|---|---|---|
| **Coal Ore** | Anywhere in stone (~6% of it). | Wood+ | **Coal** |
| **Iron Ore** | Below **Y 50** (~3%). | Stone+ | **Raw Iron** → smelt to **Iron Ingot** (see [Smelting](smelting.md)) |
| **Diamond Ore** | Below **Y 15** (~0.4%) — the rarest, deepest ore. | Iron+ | **Diamond** |
| **Magnesium Ore** | **Y 30–47** (~2%). Stone only — no deepslate version. | Stone+ | **Magnesium** |
| **Rock Salt** | Mountains and deserts only, **Y 60 up to the surface** (~5%, in clustered veins). | Any tool | **Salt** (~50% chance of ×2) |
| **Brimstone** | **Y 30–47** (~3%). Yellow-green crystal ore. | Stone+ | **Sulphur** (1–2) — see [Explosives](explosives.md) |
| **Nitre Ore** | **Y 30–62** (~2.8%). Pale white-grey ore. | Stone+ | **Saltpetre** — see [Explosives](explosives.md) |
| **Copper Ore** | **Y 30–69** (~3%), mid-depth stone band. Stone only — no deepslate version. | Stone+ | **Copper** → smelt to **Copper Ingot** (see [Smelting](smelting.md)) |

### Deepslate (below Y 30)

Below **Y 30** the base rock becomes **Pure Deepslate** — darker and much tougher. Ores down here have deepslate versions that **drop the same materials**, just take longer to break.

| Block | Hardness | Pickaxe needed | Drops |
|---|---|---|---|
| **Pure Deepslate** | 15.0 | Stone+ | Itself |
| **Deepslate Coal Ore** | 4.5 | Stone+ | Coal |
| **Deepslate Iron Ore** | 4.5 | Stone+ | Raw Iron |
| **Deepslate Diamond Ore** | 4.5 | Iron+ | Diamond |

Pure Deepslate comes in three looks — thin, healthy, and fat — that hint at how rich the rock is. They **mine identically** and all drop the same pure-deepslate item; the look is just a signal.

## ⭐ Satori — the Bitcoin gem

**Satori** is the top material in the game, above diamond. The important thing to know: **there is no Satori ore block.** Satori is a *material* you win by mining **Pure Deepslate** inside a hidden "vein". It never comes from deepslate ore blocks — only from the plain deepslate itself.

Every one of these has to be true for a strike to drop Satori:

| Condition | What it means |
|---|---|
| Block | Must be **Pure Deepslate** (not an ore variant). |
| Depth | **Y 9 or below.** |
| Pickaxe | **Diamond or Satori** pickaxe. A weaker pickaxe gives nothing. |
| Hidden vein | The block is a secret "vein member" (about 1 in 100,000, and richer — more common — the closer you get to Y 0). |
| Fresh exposure | You must have **freshly re-exposed the block by your own digging.** Deepslate that's just sitting open in a cave is "decayed" and never drops — mine the rock next to it to refresh it, then strike. |

When all of that lines up, you get **exactly 1 Satori**. Satori makes tier-5 tools and armour.

> ✅ The **Satori Block** (orange) is the only Satori *block* — pure storage. **9 Satori ↔ 1 block**, hardness 5.0, needs an **Iron+** pickaxe to mine.

### The Genesis Block

The **Genesis Block** is **not a block you place** — it's a one-per-world event. The **first Satori ever mined** in a world (by any player) claims it, with a fanfare and a permanent flag. Every Satori after that is a normal pickup. The flag persists across saves.

Full story on both: **[Bitcoin & Sats](bitcoin-and-sats.md)**.

## Wood & trees

Six tree species grow in the world. Mining **any** log drops a species-neutral **Green Log**, which feeds the **Drying Rack** seasoning chain. One log crafts into **4 planks** of the same species (see [Crafting](crafting.md)). **Leaves** (hardness 0.2) drop themselves and decay when no log is nearby.

Leaves also have a small chance to drop a **Sapling**. Plant one on grass or dirt and, on a light-gated timer (about 5 minutes on average), it grows into a real tree of its species.

| Species | Where it grows |
|---|---|
| **Oak** | Plains, Forest, mountain edges |
| **Birch** | Forest, Birch Forest |
| **Spruce** | Taiga, Snowy Tundra |
| **Jungle** | Jungle |
| **Acacia** | Savanna |
| **Rubber** | Jungle — **tappable**: tap a rubber log and it gives **rubber** (for slingshots, balls, erasers), then returns to normal after a while. |

## Storage & compressed blocks

Dense storage forms of materials — **9 material ↔ 1 block**, reversible, hardness 5.0, tier-gated like the ore they're made of.

| Block | Made from | Pickaxe to mine |
|---|---|---|
| **Coal Block** | 9 coal | Wood+ |
| **Iron Block** | 9 iron | Stone+ |
| **Diamond Block** | 9 diamond | Iron+ |
| **Satori Block** | 9 Satori | Iron+ |
| **Salt Block** | 9 salt | Any |
| **Bone Block** | 9 bone | Any |
| **Hay Bale** | 9 wheat | Any — also generates inside **Brigand Hideout** structures |

More on storage: **[Storage Blocks](storage-blocks.md)**.

## Workstations

Placeable blocks you interact with to make, store, or trade things.

| Block | What it's for |
|---|---|
| **Crafting Table** | Right-click for the 3×3 crafting grid. (The UI calls it the **Workbench**.) See [Crafting](crafting.md). |
| **Furnace** | Smelts **ore** into ingots (raw iron / copper / tin → ingots). Self-lights with fuel + input. See [Smelting](smelting.md). |
| **Campfire** | Cooks food (meats, baked potato/carrot/corn). Light it with flint & steel or a 5-second friction stick. Gives light and scares hostile mobs. See [Food & Cooking](food-and-cooking.md). |
| **Mill** | Placeable now, not yet functional — reserved for a future grinder recipe. See [Storage Blocks](storage-blocks.md). |
| **Oven** | Placeable now, not yet functional — reserved for a future baking recipe. See [Storage Blocks](storage-blocks.md). |
| **Aging Rack** | Placeable now, not yet functional — reserved for a future slow-fermenting recipe. See [Storage Blocks](storage-blocks.md). |
| **Drying Rack** | Seasons up to 8 green logs into seasoned logs (~5 min per slot). Needs open air above it. |
| **Bee Hive** | Houses bees and stores honey. |
| **Chest** | Storage — five tiers from 27 up to 72 slots. Mining it spills the contents. See [Storage Blocks](storage-blocks.md). |
| **Vendor Block** | Your own shop — sell, buy, or barter at prices you set. The first peer-to-peer Bitcoin path. See [Bitcoin & Sats](bitcoin-and-sats.md). |
| **Drafting Table** | Gives a villager the **Builder** profession. |
| **Tip Jar** | Lets others tip you 1 / 5 / 25 / 100 sats. |
| **Repair Bench** | Repairs a damaged tool with tier material plus a small sats tax. |
| **Plot Marker** | Claims a 32×32 region as yours — anti-grief. |
| **Market Bell** | Marks out a 32-block Market Hub. |
| **Auction Block** | Runs a timed auction. |
| **Bazaar Block** | Sells your held stack for its trade value in sats — the market of last resort. |
| **Bounty Board** | Posts a daily mob bounty. |
| **Village Bell** | Claims a "potential-village" zone that wandering villagers path toward. See [Village Bell](village-bell.md). |
| **Tilled Soil** | Hoe-tilled dirt/grass that crops plant into. See [Farming](farming.md). |

## Crops & plants

These live on their own page — see **[Farming](farming.md)** for stages, planting, and harvests. Live now: **wheat, carrot, potato, corn** (4 growth stages each), **papyrus reed** (by water), **cotton & hemp** (wild plant and farmable crop), the three dye flowers (**cornflower / field poppy / buttercup**), and the wild **berry bush**.

## Decorative & schematic blocks

All craft-only — these don't generate in the world.

| Block | What |
|---|---|
| **Salt Lick** | Husbandry aura (decorative + animal care). |
| **Salt Lamp** | Light source. |
| **Salt Path** | Suppresses snowfall over it. |
| **Paper Lantern** | Light source, 16 colours. |
| **Wallpaper / Bunting / Kite / Banner / Sail** | Décor, 16 colours each. See [Dyes, Fibre & Magnesium](dyes-fibre-and-magnesium.md). |
| **Tent / Fence Posts / Trophy Wall** | Build décor (fence posts come per wood species). |
| **Brigand Hideout Banner** | A worldgen marker at hideout gates. Drops nothing. |
| **Cyanotype Print** | Wall art made from a developed plan. See [Build Schematics](build-schematics.md). |

The **schematic** flow uses **Blueprint Paper** (lay it flat and right-click to capture a build), a **Latent Print** (develops a captured plan under open sky), the **Cyanotype Print** (hang a developed plan as décor), and the engine-placed **Construction Anchor** + **Architect Plaque**. Full flow: **[Build Schematics](build-schematics.md)**.

## Coming soon

These exist in the game's data but aren't live yet. You can't find them in the world.

> **Coming soon:** **Dark Oak** trees — the wood exists, but dark oak doesn't grow in the world yet.

> **Coming soon:** Decorative stone variants **Limestone, Marble, Granite, Slate** — defined but they don't generate in the world.

> **Coming soon:** **Amethyst** as a terrain block — the **Amethyst Block** (4 amethyst, 2×2) exists, but amethyst doesn't appear in the world yet.

> **Coming soon:** **Full bee behaviour** — honey already accrues (1 honey level roughly every 60 seconds) whenever a bee is within 8 blocks of a hive, but the full fly-out-and-pollinate-and-return animation loop isn't built yet.

> **Coming soon:** Crop blocks **sugar beet, beetroot, pumpkin** (and the pumpkin-stem stages) — defined but not live.

## Tips

- Always carry one tier of pickaxe more than you think you need, in case it breaks deep underground.
- Cobblestone is everywhere — it's your cheapest building block.
- Hunt ore in **cave walls** instead of digging blind. Exposed ore is the real deal.
- For Satori, remember the freshness rule: mine **into** new deepslate, never the stuff already sitting open in a cave.

## See also

- [Crafting](crafting.md)
- [Smelting](smelting.md)
- [Storage Blocks](storage-blocks.md)
- [Farming](farming.md)
- [Build Schematics](build-schematics.md)
- [Bitcoin & Sats](bitcoin-and-sats.md)
- [Dyes, Fibre & Magnesium](dyes-fibre-and-magnesium.md)
- Tutorial: [Your first blocks](https://learn.axenstax.com/docs/journey/learn-journey/first-blocks.md)

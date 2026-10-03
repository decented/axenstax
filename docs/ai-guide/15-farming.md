<!-- SOURCE: game/engine/src/{growth.rs (LIVE crop path), game_loop.rs (planting/tilling/bonemeal/composter/animal-product wiring), block.rs (farmland + crop stage block ids + mine_drop), composter.rs, drying_rack.rs, salt_lick.rs, animal_products.rs, workstation.rs, rubber.rs, item.rs (seed/crop material ids), papyrus.rs}; sapling.rs + crop_growth.rs read and confirmed DEAD (no live call sites) | Verified against code 2026-06-22 -->

# 15 — Farming

**Purpose:** the exact, verified list of crops a player can actually plant and grow, how growth and harvesting work, and the food-prep workstations that really exist — so the assistant never sends a player to grow or craft something the engine can't do. **The live crop system is `growth.rs`** (driven from `game_loop.rs`). A second file, `crop_growth.rs`, looks like a crop list but is **dead code with no live call sites** — ignore everything only in that file (see "Defined but not growable" below).

> **Honesty rule for this page:** A crop is only growable if it is wired in **both** the live planting handler **and** the live `growth.rs` advance step. Several crop blocks and seeds exist in the data but are *not* wired — they are listed under "Defined / not yet" and must **never** be offered as plantable.

---

## 1. The farming loop (verified, start to finish)

1. **Till the ground.** Hold a **Hoe**, right-click the **top face** of **Dirt** or **Grass** → it becomes **Tilled Soil**. Only the top face tills; the hoe loses 1 durability per till. (Sand cannot be tilled.)
2. **Plant a seed.** With a seed in hand, right-click the **top face of Tilled Soil** → a stage-0 crop sprouts in the air block directly above. The soil must have an empty (air) block above it.
3. **Wait for it to grow.** Crops advance through stages over time (see §3). Growing needs enough **light** and grows faster near **water** or in the **rain**.
4. **Harvest.** Break the **mature** crop (mine/left-click it) → it drops produce (and usually seeds) and the tile resets to **Tilled Soil**, ready to replant.

Breaking an **immature** crop wastes it — no drops, and it reverts to plain air (you'll have to till again).

---

## 2. Crops you can actually plant and grow

These ten are wired end-to-end (plantable **and** they grow **and** they harvest). Field crops plant on **Tilled Soil**; papyrus is the one exception (see its row).

| Crop | Plant with (seed/item) | Grows on | Stages | Harvest (mature) drops |
|------|------------------------|----------|--------|------------------------|
| **Wheat** | Wheat Seeds | Tilled Soil | 4 (0→3) | 1 Wheat + 1–3 Wheat Seeds |
| **Carrot** | a Carrot (the carrot *is* the seed) | Tilled Soil | 4 | 1–4 Carrots |
| **Potato** | a Potato (the potato *is* the seed) | Tilled Soil | 4 | 1–4 Potatoes |
| **Corn** | Corn Seeds | Tilled Soil | 4 | 1 Corn + 1–3 Corn Seeds |
| **Cotton** | Cotton Seeds | Tilled Soil | 4 | 1 Cotton + 1–2 Cotton Seeds |
| **Hemp** | Hemp Seeds | Tilled Soil | 4 | 1 Hemp Fibre + 1–2 Hemp Seeds |
| **Cornflower** | Cornflower Seeds | Tilled Soil | 4 | 1 Cornflower + 1–2 Cornflower Seeds |
| **Field Poppy** | Field Poppy Seeds | Tilled Soil | 4 | 1 Field Poppy + 1–2 Field Poppy Seeds |
| **Buttercup** | Buttercup Seeds | Tilled Soil | 4 | 1 Buttercup + 1–2 Buttercup Seeds |
| **Papyrus Reed** | Papyrus Reed | **Dirt / Grass / Sand top face, next to water** | 4 | 1–2 Papyrus Reeds; the root stays and regrows (sugarcane-style) |

Notes:
- **Carrot and Potato have no separate "seed" item** — you replant the food item itself.
- The three flowers (Cornflower, Field Poppy, Buttercup) are the *same* block whether grown by you or found wild — so breaking a **mature** flower (wild *or* farmed) drops the flower block **plus 1–2 of its seeds**. (A farmed flower on tilled soil resets the tile to Tilled Soil; a wild one on grass just leaves clean grass.)
- **Papyrus is different from field crops:** it plants directly on dirt/grass/sand (not tilled soil) and only where there's water nearby. When harvested it leaves its root and regrows itself, so you don't replant it.

### Berry bushes (forage, not farmable)
**Berry Bushes** grow **only as wild world-generation decoration** — there is **no way to plant one** in the current build. A mature berry bush, when broken, drops **1 Berries**. Treat berries as something you **find and forage**, never something you "grow your own patch of". (Berries are also used to tame nostriches — see page 13/14.)

---

## 3. How growth works (timing, light, water, rain)

All values from `growth.rs`:

- **Base speed:** **200 ticks per stage** (about 10 seconds at 20 ticks/second). A crop with 4 stages takes roughly **30 seconds** from sprout to mature under good conditions.
- **Water bonus:** any **water within 4 blocks** (horizontally, or one block below the crop) **halves** the time — **100 ticks per stage**. Dig an irrigation channel near your field to speed everything up.
- **Rain:** while it's **raining**, every crop is watered automatically (same fast speed as having water nearby) — as long as it still has enough light.
- **Light:** a crop needs an effective light level of **at least 9** to advance. Crops in open daylight grow fine, and sky-lit crops keep growing through the night. Crops underground or under a roof need a **torch nearby**, or they won't grow.
- Mature crops stop growing — they wait for you to harvest them.

Crops advance on a shared growth cadence rather than each on its own private timer, so a whole row planted at the same time (under the same water/rain conditions) tends to ripen together — handy for a single big harvest.

---

## 4. Bonemeal — instant growth

**Bonemeal works** and is the fast accelerator. Hold Bonemeal and **right-click a growing crop** → it jumps **1–2 stages** at once (the second stage lands some of the time, so it feels strong but isn't a guaranteed instant-grow). One Bonemeal is consumed per use.

- Using it on a crop that's already **mature does nothing and wastes nothing** (it won't be consumed if there's no stage left to advance).
- **Where Bonemeal comes from:** craft it from a **Bone** — **1 Bone → 3 Bonemeal** in the crafting grid. (Bones drop from some mobs.) The composter does **not** make bonemeal (see §6 — that's a common Minecraft assumption that's wrong here).
- There is also a separate, weaker accelerator called **Fertiliser** (right-click a crop for a flat **+1 stage**). Bonemeal is the stronger of the two.
- Bonemeal also has a second use: right-click **Grass** with it to sprout **tall grass** in the air above.

---

## 5. Trees & saplings

**Saplings cannot be planted or grown into trees in the current build.** Saplings (Oak, Birch, Spruce, Jungle, Acacia, Dark Oak, Rubber) **exist as items**, but there is **no live "plant a sapling, watch it grow into a tree" mechanic** yet. Trees come from world generation. Don't tell a player to plant a sapling and wait for a tree.

**Rubber tapping** is the one live tree-product loop: a **Rubber Log** can be **tapped** (right-click) to collect Rubber (latex). After tapping, that log shows as "tapped" and recovers on a cooldown (~1 in-game day) before it can be tapped again.

---

## 6. Processing & food-prep workstations (what's actually built)

### Composter — makes Compost and Saltpetre (NOT bonemeal)
The **Composter** ages organic waste. Right-click it with a compostable item to load one in; it processes over time, and you collect the result:

- **Organic matter → Compost** (~30 s): wheat, berries, seeds (wheat/corn/sugar-beet/beetroot/cotton/hemp/cornflower/field-poppy/buttercup seeds), and saplings (oak/birch/spruce/jungle/acacia/dark-oak/rubber) all compost.
- **Compost → Saltpetre** (~60 s): re-insert the Compost to age it further into Saltpetre (used in explosives chemistry).

> **Accuracy flag:** unlike Minecraft, the composter here does **not** produce bonemeal. Its outputs are **Compost** and **Saltpetre**. Bonemeal comes from a **Bone** (see §4).

### Drying Rack — seasons logs into fuel/material
The **Drying Rack** matures **Green Logs** into **Seasoned Logs** over ~5 real minutes per slot (8 slots). It only makes progress when there's **air directly above it** (place it outdoors or under an open window — a buried rack just stalls, with no loss). Mining a rack with logs in it spills them back as Green Logs (the seasoning is lost). *(Alpha seasons Oak only.)*

### Salt Lick — livestock husbandry (a block you place near animals)
A **Salt Lick** placed near livestock (**Cow, Sheep, Horse, Pig, Goat**, within 8 blocks) makes them heal faster, drift toward it, and drop one extra of their main product when killed inside its range. It does nothing for carnivores or exotic animals. This is animal-keeping, not crop-farming, but it lives in the farming toolkit.

### Mill / Oven / Aging Rack — framework only, not usable
A generic workstation framework names a **Mill**, **Oven**, and **Aging Rack**, but these are **not built as usable workstations** — there are no live blocks or recipes for them. Don't offer them.

---

## 7. Animal products as "farming" (cross-link to page 14)

These livestock loops are live and count as part of farming. Full detail is on **page 14 (Animals & Mobs)**:

- **Chicken eggs** — a chicken periodically lays an **Egg** at its feet; just pick it up.
- **Cow milk** — right-click a cow with an empty **Bucket** → **Milk Bucket** (with a cooldown before that cow can be milked again).
- **Sheep wool** — right-click a sheep with **Shears** → **Wool** (a well-bred sheep yields more).
- **Tamed nostrich eggs** — a tamed nostrich lays a **Nostrich Egg** about once per in-game day.

---

## Deferred / not yet (never present these as available)

**Crops that exist in the data but you CANNOT plant or grow** (their blocks/seeds/drops are defined, but they are **absent from the live planting handler and the live `growth.rs` step** — only the dead `crop_growth.rs` references them). Do **not** tell a player they can grow these:

- **Sugar Beet** — Sugar Beet Seeds item exists and stage blocks exist, but it is **not plantable and does not grow** in the live build.
- **Beetroot** — same: Beetroot Seeds and stage blocks exist, but it is **not plantable and does not grow**. *(UK English reminder: in this game "Sugar Beet" and "Beetroot" are two different defined-but-unwired crops; neither is growable.)*
- **Pumpkin / Pumpkin Stem** — stem stage blocks exist (and a mature stem's drop is defined), but there is **no way to plant a pumpkin seed and no live stem growth**.
- **Berry Bush as a crop** — you cannot plant berry bushes; they're wild-only forage (see §2).

**Other deferred farming features:**
- **Planting saplings / growing trees from saplings** — not wired (see §5). Saplings are items only.
- **Mill, Oven, Aging Rack workstations** — framework only, no usable block or recipes (see §6).
- **Composter → bonemeal** — does not happen; the composter makes Compost and Saltpetre (see §6).
- **Craftable Flour, Cake, Pancakes, Beetroot Soup** and similar processed foods — their workstations/recipes are not built, so they are gated out (also noted in `99-accuracy-and-deferred.md`). Don't send a player to craft them.

If a player insists they grew one of the "deferred" crops, believe them and flag it (the corpus may be stale) — but don't promise it to the next player until it's confirmed.

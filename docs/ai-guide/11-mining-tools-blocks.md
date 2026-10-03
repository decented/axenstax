<!-- SOURCE: game/engine/src/crafting.rs, item.rs, block.rs, proof_of_play.rs, game_loop.rs (maybe_gem_drop + survival break path), block_interact.rs, crafting_catalogue.rs | Verified against code 2026-06-22 -->

# 11 — Mining, Tools & Blocks

Verified facts about tools, the "right tool for the block" rule, the blocks you can mine, and what the mining hash (Proof-of-Play) really does. Only describe what is in this page. UK English.

---

## 1. Tool materials (the tier ladder)

There are **five tool materials** (`ToolMaterial` in `crafting.rs`), low to high:

| # | Material | In-game name prefix | Notes |
|---|----------|--------------------|-------|
| 0 | Wood | "Wooden …" | Starter tier |
| 1 | Stone | "Stone …" | |
| 2 | Iron | "Iron …" | |
| 3 | Diamond | "Diamond …" | |
| 4 | **Satori** | "Satori …" | Top tier — the orange gem, above diamond. Mined deep from pure-deepslate veins. (Code identifier is `Satori`.) |

Higher material = faster mining, more durability, more attack damage, and can harvest more block types. There is **no gold tier** in the engine — do not mention gold tools.

### Per-material stats (verified)

| Material | Mining speed | Tool durability (pick/axe/shovel/sword/hoe) | Sword damage | Axe damage |
|----------|-------------|----------------------------------------------|--------------|-----------|
| Wood | 2.0 | 59 | 4.0 | 7.0 |
| Stone | 4.0 | 131 | 5.0 | 7.0 |
| Iron | 6.0 | 250 | 6.0 | 7.0 |
| Diamond | 8.0 | 1561 | 7.0 | 9.0 |
| Satori | 9.0 | 2031 | 8.0 | 10.0 |

(Mining speed divides a block's hardness to give the break time. Faster tool = quicker break.)

## 2. Tool types (what tools exist)

The mineable/combat tools are: **Pickaxe, Axe, Shovel, Sword, Hoe** — each comes in all five materials. (The Hoe exists in all five tiers but is always weak as a weapon — attack damage 1.0 regardless of tier; a Satori Hoe is not a weapon. It tills dirt/grass into soil for farming.)

Other tools exist but are **single-tier** (one fixed material, no ladder):

| Tool | Tier | What it does |
|------|------|--------------|
| Bow | recipe makes Wood; per-tier exists | Ranged weapon (fires arrows) |
| Flint and Steel | single (Iron) | Right-click an unlit campfire to light it |
| Shears | single (Iron) | Shears a sheep for wool |
| Fishing Rod | single (Wood) | Right-click aimed at water to cast and fish |
| Slingshot | single (Wood) | Fires Rubber Ball ammo; can stun passive/neutral mobs (at half charge or more; carnivores and brigands shrug it off) |
| Eraser | single (Wood) | Right-click a Blueprint Paper block to revert it to a Papyrus Sheet |
| Drafting Stamp | single (Wood) | Captures a built structure as a blueprint Plan item |

For mining and digging, only **Pickaxe / Axe / Shovel** matter — see below.

## 3. The "right tool" rule

Each block has a **best tool** and (for harder blocks) a **minimum pickaxe tier** required to actually get a drop.

- **Best tool** = the tool type that mines that block fastest (`best_tool_for`). Using it applies the tool's mining speed; using the wrong tool (or bare hands) mines at speed 1.0 (much slower).
  - **Pickaxe** → stone, cobblestone, sandstone, coal/iron/diamond ore, brimstone, nitre ore, ore-storage blocks (coal/iron/diamond blocks), deepslate + deepslate ores, Satori block. (Copper Ore and Magnesium Ore have no "best tool" in code — they mine at speed 1.0 with anything — but you still need a stone pickaxe to get a drop, per the tier table below.)
  - **Shovel** → dirt, grass, sand, gravel, snow.
  - **Axe** → logs, planks, crafting table.
- **Tier gate (`min_tool_tier` + `can_harvest`)**: harder blocks only **drop an item** if you hold a **pickaxe of at least the required material**. Mine them with the wrong tool or too low a tier and the block still breaks, but **nothing drops** — wasted effort. The exact ladder:

| Block | Minimum tool to get a drop |
|-------|----------------------------|
| Stone, Cobblestone, Sandstone, Coal Ore, Coal Block | **Wood pickaxe** or better |
| Iron Ore, Iron Block, Copper Ore, Magnesium Ore, Brimstone, Nitre Ore | **Stone pickaxe** or better |
| Deepslate (pure), Deepslate Coal Ore, Deepslate Iron Ore | **Stone pickaxe** or better |
| Diamond Ore, Diamond Block, Deepslate Diamond Ore, Satori Block | **Iron pickaxe** or better |

- Anything **not** in that table (dirt, sand, gravel, wood, leaves, glass, snow, beds, crops, workstation blocks, …) drops with **any tool or bare hands**. The right tool just makes it faster.
- **Rule of thumb to teach a player:** to harvest a block you must use a **pickaxe**, and its material must be **at least** the tier shown. A wood pickaxe gets you stone and coal; you need stone-tier to mine iron; iron-tier to mine diamond and Satori.

### Block hardness (how long to mine, bare-hand seconds)

Hardness ÷ tool mining-speed = break time. Sample values from `block_hardness`:

| Block | Bare-hand seconds |
|-------|------------------|
| Leaves, Snow | 0.2 |
| Sand | 0.5 / Gravel 0.6 / Sandstone 0.8 |
| Dirt, Grass | 2.5 |
| Cobblestone, Planks, Crafting Table | 2.0 |
| Coal/Iron/Diamond Ore, Brimstone, Nitre | 3.0 |
| Oak Log | 4.5 |
| Deepslate ores | 4.5 / Satori Block 5.0 / storage blocks 5.0 |
| **Stone** | 10.0 |
| **Pure deepslate** | 15.0 (hardest common block — "commit to depth") |

So stone takes ~10s by hand but ~5s with a wood pickaxe, ~1.25s with diamond. Pure deepslate is the slowest natural block to dig.

## 4. Block families & what they drop

| Family | Examples | Drop when harvested |
|--------|----------|--------------------|
| Stone | Stone | drops **Cobblestone** (not stone) |
| Soil/loose | Dirt, Grass, Sand, Gravel, Snow | drop themselves; **Gravel has a 15% chance to drop Flint instead** |
| Wood | Oak Log, Planks | drop themselves |
| Ores | Coal, Iron, Diamond, Copper, Magnesium, Brimstone(sulphur), Nitre(saltpetre), Rock Salt | drop their raw material |
| Deepslate | Pure Deepslate + deepslate ore variants | deepslate ores drop the same raw material as the stone ore; pure deepslate drops itself |
| Special | Satori Block, Satori (gem) | top-tier material |

### Ores — guaranteed drops with the right tool

With a pickaxe of the required tier (table in §3), these ores are **guaranteed** to drop, one per block:

| Ore (block id) | Drops |
|----------------|-------|
| Coal Ore / Deepslate Coal Ore | 1 Coal |
| Iron Ore / Deepslate Iron Ore | 1 Raw Iron (smelt to an iron ingot) |
| Diamond Ore / Deepslate Diamond Ore | 1 Diamond |
| Copper Ore | 1 Copper (smelts to ingot) |
| Brimstone (sulphur ore) | 1–2 Sulphur |
| Nitre Ore | 1–2 Saltpetre |
| Rock Salt | 1–2 Salt |

There is **no random "did the ore drop?" roll** — meet the tier and the ore drops every time. (Only side things like gravel→flint and the 1-vs-2 salt/sulphur amount are seeded chance.)

> **Anti-farming:** breaking a **block the player placed themselves** drops the item back but counts as **no work** and yields **no hash-driven gem** — you cannot place-and-rebreak to farm. This is normal and worth knowing if a player asks why re-mining their own wall "does nothing special".

## 5. Satori (the deep gem)

Satori is the rarest material, found by mining **pure deepslate deep underground** with a **diamond (or better) pickaxe**. It does not sit in obvious ore blocks — it is hidden inside ordinary pure-deepslate and revealed by a deterministic vein algorithm at depth (`maybe_gem_drop` in `game_loop.rs`, using the vein helpers in `proof_of_play.rs`). Conditions, all required:

1. The block is **pure deepslate** and **deep enough** (below the depth gate).
2. You hold a **diamond+ pickaxe**.
3. The block is part of a Satori vein (deterministic per world).
4. The block was **freshly exposed by your own digging** — naturally cave-exposed deepslate has "decayed" and won't yield (you must mine adjacent deepslate to refresh it).

The **first Satori ever found in a world** (by anyone) is the **Genesis Block** — a one-off celebration, singular per world. Every later Satori is a normal pickup.

If a player asks "how do I find Satori?": dig **deep**, bring a **diamond pickaxe**, and mine **into** fresh pure deepslate (not just grab exposed bits in caves). It is meant to be rare.

## 6. Proof-of-Play — the mining hash (educational, NOT earning)

When a deep pure-deepslate block is checked for Satori, the game runs a **real HMAC-SHA256 hash** of the block's position with a server-held secret (`proof_hash` in `proof_of_play.rs`). This runs **behind the scenes** — it is **not shown on screen** (see Deferred below). What to teach about it:

- **It is the same kind of cryptographic proof-of-work that secures Bitcoin** — a real primitive, used here as an honest **fairness foundation**, not a flashy on-screen demo. The player is **not** a miner of Bitcoin.
- **It is deterministic, not chance.** The same block in the same world always produces the same hash. It is **not** a gamble, **not** a slot machine, **not** random luck.
- **It is NOT earning.** It is **not** a payout, **not** sats, **not** money. There is nothing to cash out. (Real-money payouts are **deferred** and switched off — see `99-accuracy-and-deferred.md`.) **Never** frame mining as a way to earn or "get sats".
- What it actually does today: it deterministically drives the **Satori vein** (a fair, world-fixed gem layout) — that's its one live job. Its *design* role also includes (1) **education** about proof-of-work, and (2) on a hypothetical Bitcoin-enabled server, gating effort-based payouts via a **work-meter** (effort accumulation), **never** a chance threshold. Neither of those is active or on-screen for players today; only the vein layout is.

### How to talk about it (if a curious player asks)
"When you mine deep, the game quietly does a real cryptography puzzle — the same kind that keeps Bitcoin honest — to decide where the rarest gems hide. You don't see it happen; it's working in the background to keep things fair. It's not a way to earn money. And it always gives the same answer for the same block, so it's fair, never luck."

## Deferred / not yet — do not present as available

- **A visible per-strike hash readout in the HUD.** The design intends the hash to be surfaced (truncated) on every pickaxe strike, but in the current build the hash is only computed during the deep Satori-vein check and is **not shown on a normal strike**. Do **not** tell a player they'll see a hash pop up every time they mine.
- **Hash on *every* block / "even on grass".** In the current code the Proof-of-Play hash runs only for the deep Satori evaluation on pure deepslate — **not** on grass, dirt, or ordinary stone. Describe the hash as a **deep-mining / Satori** thing, not an every-block thing.
- **Earning, spending, holding, or withdrawing real money / sats from mining.** Entirely deferred and off by default; any future version is parent-controlled and opt-in. Never imply mining earns money.
- **Anti-X-ray chunk obfuscation** (hiding buried ore from the client). This is a stated architectural goal but is **not implemented** in the current engine — don't claim the game hides ore from cheaters yet.
- **Gold tools / a gold tier.** Not in the engine. Five tiers only: Wood, Stone, Iron, Diamond, Satori.

---

*Verify any "you can mine X with Y" claim against §3's tier table before stating it. When unsure, say "let's find out in-game" rather than guess.*

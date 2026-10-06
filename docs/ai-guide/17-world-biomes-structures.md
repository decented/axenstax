<!-- SOURCE: game/engine/src/biome.rs, world.rs, village_gen.rs, mineshaft_gen.rs, ravine_gen.rs, brigand_hideout_gen.rs, snowfall.rs, save.rs, menu.rs, game_loop.rs, main.rs, commands/builtins/seed.rs | Verified against code 2026-06-22 -->

# 17 — World, Biomes & Structures

Purpose: the verified ground truth on how an AxeNStax world is shaped — its biomes, terrain, day/night, weather, the structures you can find, how a world is created (including Blank Canvas), and the Genesis Block lore event.

This page describes the **current build only**. If it isn't listed here, treat it as not in the game.

---

## 1. Biomes

A world is divided into biomes by a temperature/humidity + elevation noise field. There are **ten** biome types in the code (`biome.rs` `enum Biome`). Two are "terrain-shape" biomes chosen by elevation (Ocean = low, Mountains = high); the other eight are "climate" biomes chosen by a Whittaker temperature/humidity grid on mid-elevation land.

| Biome | Surface | Terrain | Trees / vegetation | Notes |
|---|---|---|---|---|
| **Plains** | Grass over dirt | Gentle, low rolling land; small lakes | Sparse oak trees | The friendly default. Villages can spawn here. |
| **Forest** | Grass over dirt | Rolling land with lakes | Dense **oak + birch** trees | Villages can spawn here. |
| **Birch Forest** | Grass over dirt | Like Forest | Dense **birch** | A cooler, lighter-wood forest. |
| **Taiga** | Grass over dirt | Slightly higher land | **Spruce** (dense) | Cold conifer forest. |
| **Jungle** | Grass over dirt | Lifted slightly so it isn't drowned | **Densest** trees — jungle wood + rubber trees | Warm and wet. Rubber trees are tappable here. |
| **Savanna** | Grass over dirt | Low, dry rolling land | Sparse **acacia** | Hot and dry. |
| **Desert** | **Sand** over sandstone | Low dunes, no lakes | **No trees** | Hot and arid. Rock Salt can occur in the rock below. |
| **Snowy Tundra** | **Snow** over dirt | Low land | Sparse **spruce** | Cold; snow can drift onto the ground over time (see §3). |
| **Mountains** | Stone, **snow caps above y≈85** | Tall, steep massifs | None | High elevation; exposes more rock and ore. Snow can drift here too. |
| **Ocean** | Gravel seabed | Deep basins below sea level | None | Filled with water up to sea level (y=62). |

Notes verified from code:
- **Sea level is y=62** (`SEA_LEVEL`). Ocean basins and low dips below this fill with water.
- Mountains gain a **snow cap** on their surface when the surface is above y=85.
- **Display names** use UK-English spacing: "Birch Forest", "Snowy Tundra".
- Which mobs/animals live where is covered on the mobs/animals pages, not here. What this page can confirm: **Villagers** appear in villages (Plains/Forest only — §4.1), and **brigands** appear at brigand hideouts in temperate biomes (§4.4).

---

## 2. Terrain, caves, ores, and the underground

- **World height:** the alpha world is a relatively short column. **Bedrock is the unbreakable floor at y=0.** Sea level is y=62.
- **Surface → underground layering** (a normal column): the biome surface block on top (grass/sand/snow), a few blocks of dirt/sandstone beneath, then **stone**, and below that **deepslate** (a darker, deeper rock that begins replacing stone around y=30 and is solid deepslate by y≈22).
- **Ores** are depth-banded and deterministic (the same world seed always puts them in the same place):
  - **Coal** — common, found at any depth in stone.
  - **Iron** — below y=50.
  - **Diamond** — rare, only **below y=15** (and that deep, it appears as the deepslate variant).
  - Other minerals exist deeper/in specific biomes (e.g. Rock Salt in Mountains/Desert rock). Mining detail belongs on the mining/blocks page.
- **Caves** are carved through the underground (roughly between y=2 and y=55). Deep caves are dangerous: **caves at or below y=10 pool with lava**, and caves below sea level can fill with water. Reaching diamonds means braving the deep, lava-lined band — that risk is intentional.

---

## 3. Day/night cycle and weather

### Day/night
- The world runs a **day/night cycle**. A full day is **24000 ticks**, which at 20 ticks/second is a **20-minute real-world day**. New worlds start at world-time 7500 (morning).
- A **Blank Canvas** world can **lock the time** to always-day, always-night, or the normal cycle (see §5).

### Rain
- **Rain** occurs in normal play. While the sky is clear, roughly once per in-game minute there is about a **22% chance** rain begins, lasting **1 to 2.5 minutes**. It's deterministic from the world clock, not random luck.
- Rain **waters crops** (helps them grow) and draws a rain overlay on the HUD.

### Snow
- **Snow drifts onto the ground in cold biomes** — specifically **Snowy Tundra and Mountains** (`snowfall.rs`). Slowly, a snow block is placed on top of exposed grass/dirt surfaces in those biomes. It does **not** snow in warm biomes.
- (This snow-drift is a separate system from rain — rain doesn't visually turn into snowfall; the snow painter is what makes cold biomes go white over time.)

---

## 4. Structures

Structures generate deterministically from the world seed on a virtual grid, so the same seed always places them in the same spot, and they straddle chunk boundaries cleanly. Finding one is meant to feel like a real discovery.

### 4.1 Villages
- **Where:** **Plains or Forest only**, on dry land above sea level. About one per 32×32-chunk cell (roughly 80% of cells host one).
- **What's in it:** a cluster of **3–6 small houses** around a central anchor, a **water well** (cobblestone rim around a water source), a **lit campfire** on a cobblestone hearth, and **workshop buildings** on an outer ring (one per village trade).
- **Villagers:** villages are populated with **villagers** (about one per house). Villagers can claim a workshop and take a **profession** — e.g. Farmer, Cook, Carpenter, Blacksmith, Scribe, Miller, Baker, Brewer.
- **Defenders:** a large, well-populated village (≥5 houses and several settled villagers) auto-spawns a **Knight** defender to guard it.
- **Village Bell** (player-founded villages): a player can place a **Village Bell** block. Wandering villagers will migrate toward a bell and, on arrival, settle there — founding a new village anchor that gets all the same village systems. This is how you grow your own village.

### 4.2 Mineshafts (underground)
- **Where:** **underground**, well below the surface and above the lava layer — about one candidate per 24×24 chunks, ~40% of which actually spawn. Digging into one is a "what's down here?" moment.
- **What's in it:** a central junction with **2–4 timber corridors** radiating out — each a 3-wide × 2-tall tunnel with an **oak-plank walkway**, **fence-post + plank support frames** every few blocks, and a **wooden loot chest** about 60% of the way along each arm.
- **Loot (per chest, deterministic):** a few stacks drawn from: oak planks, coal, raw iron, iron ingots, bone, bread, sticks, arrows, wheat — and, rarely (about 1 in 6 of the slots that roll it), a single **diamond**.

### 4.3 Ravines (rare canyons)
- **Where:** carved on land above sea level — about one candidate per 40×40 chunks, ~45% of which spawn. Rare and dramatic.
- **What it is:** a long, narrow canyon that splits open at the surface and cuts deep into the stone (typically 30–50 blocks deep), **exposing ore in its walls**. The deepest ones (bottoming out near bedrock) have a **thread of lava** along the floor. Ravines carry no chest or loot table — the reward is the exposed ore and the spectacle.

### 4.4 Brigand hideouts
- **Where:** **temperate biomes only** — Plains, Forest, Savanna, or Taiga — on dry land, and kept **at least 128 blocks away from any village**. About one per 64×64 chunks (sparser than villages), ~60% of those cells.
- **What's in it:** a **wooden palisade ring** (oak logs, ~6-block radius, 3 tall) with a south gate, a **cobblestone hearth with an unlit campfire**, a central **stolen-goods chest**, four **corner torches** on the palisade, a **Brigand Hideout banner** at the gate, and **3–4 small huts** (oak-plank floor + a hay-bale "bed").
- **Loot (stockpile chest):** wheat (2–4), bread (1–2), iron ingot (1–2), wool (1–2).
- **Who's there:** the hideout is **guarded by hostile mobs** — a captain (Marauder) plus several Brigands (typically 4 total). About 1 in 5 hideouts is a "rare" hideout that also holds a tougher **Berserker**. This is a combat encounter, not a free-loot building.
- After a player clears and loots a hideout, it goes dormant for about two in-game days and then re-stocks. (Combat/AI details live on the mobs page.)

---

## 5. World creation, spawn, and Blank Canvas

When you make a **New World**, you choose a **world type** and a **game mode** (covered on the gameplay page). Worlds get a random world seed at creation (you can also type one in).

### World spawn
- In a **Normal** world you spawn on the generated terrain surface.
- In a **Blank Canvas / Workshop** world you spawn at **y=80**, standing on the flat floor.

### Blank Canvas (the "flat / empty" world type)
Choosing **"Blank Canvas"** in the create dialog (internally `world_type = "flat"`) makes a **flat, empty world** for building undisturbed: a single flat floor plane over an unbreakable bedrock base, with **no terrain, no ores, no caves, no trees, no villages, no hideouts, and no mobs spawned by world-gen**. Options surfaced in the create dialog:

- **Ground** (the floor surface): **None** (just bedrock), **Grass**, **Sand**, **Stone**, **Dirt**, **Snow**, or **Water**.
  - "Water" lets you pick a **water depth of 1–8 blocks** over a sand-and-bedrock base.
- **Time:** **Cycle** (normal day/night), **Day** (locked daytime), or **Night** (locked night).
- **Mobs:** **ON** or **OFF**. Selecting Blank Canvas defaults **mobs to OFF** so you can build in peace. (Blank Canvas worlds also keep your inventory on death.)

(A **Normal** world uses the full terrain generator described above — biomes, caves, ores, trees, villages, hideouts, ravines, mineshafts.)

### The `/seed` command
- Typing **`/seed`** in chat shows you the **world seed** (e.g. "Seed: 12345"). It's read-only, available to everyone (not a cheat), and doesn't change anything. The seed is what makes terrain and structures reproducible. (Chat commands must be enabled for the world; they're ON by default.)

---

## 6. The Genesis Block (lore / event — NOT earning)

The **Genesis Block** is a one-time, **one-per-world founding event** in the world's lore. It is recorded as a flag on the world's data (`WorldMeta.genesis_block_found`).

- It triggers the **first** time anyone in a world finds a **Satori** (the rarest, deepest material — the in-world "best money"). That first find gets a special **"Genesis Block!" celebration** on screen; every later Satori is a normal pickup.
- It is **singular**: like the very first block of a chain, **there is only ever one Genesis Block per world**. Once found, the flag stays set.
- It is a **founding-moment / discovery moment in the world's story** — a milestone you can be proud of. It is **not** a way to earn money, **not** a payout, and **not** a gamble. There is nothing to cash out. Frame it as lore and a personal achievement, never as earning.

---

## Deferred / not yet
- **More biome variety / per-species tree systems.** The code has extra biome groundwork (a full Whittaker climate classifier, per-biome tree-species tables, biome blending) that is **not yet the live world-gen path** — it's prepared but not wired in. Don't promise biomes or tree behaviours beyond what §1 lists as currently generating.
- **Snow as falling weather.** Snow appears by **drifting onto the ground in cold biomes over time** (§3), not as a visible falling-snow storm. Don't describe a snowstorm.
- **Structures beyond the four listed.** Only **villages, mineshafts, ravines, and brigand hideouts** generate. There are no other dungeons/temples/strongholds in the current build — don't invent them.
- **Ocean structures / underwater ruins** — none. Ravines stay on land; oceans are water over a gravel seabed.

# World Creation & Spawn

The flow from "main menu" to "I'm standing in the world". This page covers world creation, the spawn-location picker, save / load, and the world-list options around them.

## The main menu

After Axe'n'Stax launches (and signed in, if on a Bitcoin-enabled server), you reach the main menu. You'll see:

- A **list of worlds** you've created (empty on first run).
- A **Spawn dropdown** above the list — applies to the next world you load.
- A **Create New World** button.
- For each world, **Play / Host / Edit / Fork / Delete** options.

## Creating a new world

Click **Create Your First World** (or **+** if you already have worlds). The Create dialog opens:

- **World name** — display name. Leave blank for "New World 1, 2, …" auto-naming.
- **Seed** — optional. Determines world layout (terrain, ores, village placement). Blank = random.
- **Game mode** — **Survival** (default) or **Creative**. See **[Game Modes](game-modes.md)**.

Click **Create & Play**. The world generates, the menu closes, and you spawn in.

> Your typed seed is honoured — numeric or text (Minecraft-style hashing) — and drives real terrain generation, saved per world so re-entering a world always regenerates the same terrain from its own seed.

## The Spawn dropdown

Above the world list. **Applies to whichever world you load next**, including a fresh Create. It's a per-session pick — survives world-list refreshes but resets to Default the next time the app starts.

Nine options:

| Option | Behaviour |
|---|---|
| **Default (saved position)** | No override. Existing worlds: spawn at your saved position. New worlds: spawn at world origin (0, surface, 0). |
| **Near a village** | Search outward for the nearest village anchor (within ~32 grid cells / ~1 km). Teleport there. |
| **Wilderness (no village in cell)** | Search outward for a grid cell that has no village in it. Plain wilderness drop. |
| **In open plains** | Search for a Plains-biome tile. |
| **In a forest** | Search for a Forest-biome tile. |
| **In the mountains** | Search for a Mountains-biome tile. |
| **In the desert** | Search for a Desert-biome tile. |
| **On a beach** | Search for a Plains/Desert tile near sea level adjacent to an Ocean tile. |
| **At origin (0, 0)** | Drop at (0.5, surface, 0.5) — useful for debugging or starting over. |

### How the search works

For "Near a village", the engine reads the **deterministic village layout** from `village_gen` and finds the closest anchor. **No chunks have to be generated** — village positions are a pure function of the world seed, so the search is fast.

For biome searches, the engine reads `biome_at(x, z)` (also seed-deterministic) in a spiral outward from the reference position, stride'd for efficiency. Mountain biome is rarer than Plains, so its stride is wider + radius is larger.

For "Wilderness", the engine looks for a grid cell whose hash says "no village here". On alpha seeds, this is about a 1-in-5 chance per cell, so a hit comes quickly.

### If the search fails

If the engine can't find the requested biome within the search budget, it **falls back to the saved/default position** silently. So a "find me a desert!" search in a tiny world with no desert biome won't trap you — it'll just spawn you at the default.

## Loading an existing world

Click **Play** on any world card (or **double-click** the card). The Spawn dropdown still applies — so a kid can pick **Near a village** on a save where they wandered far away from one, teleport back, and pick up where they left off.

Saved player state restores:

- Position, look direction (unless you used the spawn override).
- Health + hunger.
- Inventory + hotbar slot.
- Reputation with every village (as of the recent persistence work).
- Village anchors, populated villages, placed Village Bells.
- Campfire block-entity state (fuel + cook progress).

The world chunks themselves load lazily — you'll see them stream in as the player position settles. The 5×5 chunks around the spawn pre-load so you don't drop into void.

## Saving

Three triggers save the world:

1. **Auto-save** every 5 minutes while you play.
2. **Pause menu → "Save"** when you want to commit a checkpoint before risky play.
3. **Pause menu → "Save and Quit"** when you're done.

Worlds save to `worlds/<folder>/` as a `world.dat` (bincode-serialized player + village state) plus a `chunks/` directory of per-chunk binary files. Only **modified or non-empty chunks** are written.

On WASM (the PWA), worlds save to IndexedDB, addressed by your Signet pubkey.

## Editing world details

Click **Edit** on a world card. You can change:

- **Display name** (the title shown in the list — the on-disk folder doesn't rename).
- **Description** (a longer text shown in tooltips).

These are saved into `worlds/<folder>/meta.json` along with the game mode + creation date.

## Forking a world

Click **Fork** on a world card. Creates a copy of the world with a new name. Useful for "let me try something risky on a copy" or "let me play with my friend on this exact world without overwriting my single-player save".

## Deleting a world

Click **Delete** on a world card → confirmation dialog → type the world name to confirm → delete. There's no undo. Worlds you delete are gone.

## Hosting + joining (multiplayer)

- **Host** on any world card opens it for LAN play. Other players on the same network see it in their world list under a "remote" badge.
- **Join Direct** in the menu lets you connect to a friend's IP address.
- **Host online** opens the world to a friend in another house, by invite — a different
  setup from LAN. See **[Play with a friend online](play-with-a-friend-online.md)**.

> These are **native desktop app** features only — they're hidden on the web/PWA build, which shows a "get the desktop app" prompt in their place instead.

Multiplayer is still in active development — single-player + split-screen are the rock-solid paths. LAN multiplayer works but has a longer technical-debt list. See `docs/spec/04-networking.md` for the design.

## Tips

- **Save before risky play.** Mining into bedrock-adjacent territory, exploring a deep cave with low health — these are good moments to manually save.
- **The Spawn dropdown resets to Default each app launch.** If you always want to spawn near a village, pick it each time.
- **Two players on a split-screen save share a world.** Each player has their own slot — position, inventory, reputation are per-player; the world (blocks, villages) is shared.
- **Fork your save before trying anything you might regret.** Forks are cheap; rebuilds are not.

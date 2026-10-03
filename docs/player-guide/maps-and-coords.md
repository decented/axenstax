# Maps & Coordinates

Where you are in the world, how to find your way back, and the
commands that help.

## The coordinate system

Three axes:

- **X** — east(+) / west(–)
- **Y** — up(+) / down(–). Sea level is **Y=62**; bedrock is roughly
  **Y=0**.
- **Z** — south(+) / north(–)

The world is **infinite** on X and Z — you can walk forever in any
horizontal direction. The playable column is 6 chunks tall, so Y is
clamped between bedrock at the bottom and the sky cap (Y ≈ 96).

## F3 debug overlay

Press **F3** to toggle the debug overlay. It shows:

- Current XYZ + facing direction
- Frame timing + tick timing (perf samples)
- Reserve richness (the Bitcoin Reserve gauge)
- Mining-rate text — sats-per-hour estimate while mining
- Biome at the player's current column

F3 is local-only — it doesn't affect gameplay or other players, and
it isn't a cheat. Leave it on while learning the world.

## Useful commands

Chat (press **T**) and type:

| Command | What it does |
|---|---|
| `/seed` | Print the world seed — useful if you want to recreate the same map |
| `/biome` | Print the biome at your current XZ |
| `/biome 100 -200` | Print the biome at any XZ you name |
| `/biomes` | List every biome the engine knows + its surface block |
| `/tp <x> <y> <z>` | Teleport (op-only / creative) |
| `/spawnpoint` | Set your bed-respawn point to your current position |
| `/waypoint add\|list\|remove\|tp <name>` | Manage map waypoints (see below) |

## Setting a Spawn Point

The standard way to mark "home": **right-click a bed**. Your respawn
point becomes the bed; on death you'll wake up there.

You can also use `/spawnpoint` to set the respawn at exactly your
current position, no bed required.

## Village Bells

Every Village Bell (craftable + placed by you, or auto-placed by
procgen) acts as a navigation beacon for **Wandering Villagers** —
they pathfind toward unclaimed bells over a 64-block radius. They're
also useful as a personal "I built something here" marker on the
map. (In-game flavour text may still say "Wandering Villager" even
though the mob is internally the "Peddler" — just a naming leftover.)

## Minimap & the full map (M key)

A small **minimap** sits in a corner of the HUD at all times, showing
your immediate surroundings.

Press **M** to open a **full-screen map** — zoomable and pannable,
separate from the Workshop. It shows:

- **Waypoint markers** — including any you've placed.
- A **"📌 Pin map centre"** button to drop a waypoint at wherever the
  map is currently centred on.
- **Teleport-to-waypoint** (creative mode only).
- **Auto-placed death waypoints** — die, and a waypoint marking the
  spot appears automatically so you can walk back for your dropped
  items.

Manage waypoints from chat with `/waypoint`:

- `/waypoint add <name>` — save a waypoint at your current position.
- `/waypoint list` — list your saved waypoints.
- `/waypoint remove <name>` — delete one.
- `/waypoint tp <name>` — teleport to one (creative mode only).

## What's NOT here yet

- **Maps as items** — paper Map item that records your wanderings.
  Spec'd in `whats-coming.md`; not in alpha.
- **Compass** — points to your spawn. Same status.

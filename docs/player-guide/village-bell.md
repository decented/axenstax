# Village Bell

You can found your own village. Craft a Village Bell, place it in the wilderness, and over a few in-game days **Wandering Villagers** will migrate toward it and settle. Once enough of them arrive, the place becomes a proper village with quests, reputation, the lot.

## The Bell block

A **Village Bell** is a single block — iron block top, cobblestone post — that doesn't do anything mechanical on its own. It's a **marker**. The engine reads it as "the player wants a village here."

### Recipe

At a **Workbench** (3×3 grid), vertical column:

```
I        (Iron Ingot)
S        (Stick)
P        (Oak Plank)
```

That's it: 1 iron + 1 stick + 1 plank → 1 Village Bell.

Iron is the bottleneck. You'll need a Stone Pickaxe minimum to mine iron ore, then smelt it in a Furnace — or skip the smelting and pick up iron ingots as drops from the bandit family (Brigand/Marauder/Berserker) or as quest rewards.

### Placing it

Right-click any solid block (top side) while holding the bell. It places like any normal block.

A toast confirms: *"Village Bell placed — wandering villagers may find their way here."*

The bell now sits in the world. Don't break it — the migration logic reads its position from the world side-table every tick.

### The Houses tab

Right-click an **already-placed** bell (rather than one in your hand) and you'll get a **Houses tab** — a list of the nearby architect-attributed houses in that village, one row per architect. Each row has a **Tip** button (sends sats to that architect's npub) plus a **Tip all** option to spread a payment across every listed architect at once.

## Wandering Villagers

Once you have a bell placed, the engine starts spawning **Wandering Villagers** around it.

They look like regular villagers but with **purple-hooded heads** instead of brown. They're passive and slow. They only spawn once at least one Village Bell exists anywhere in the world — with no bell placed, none appear. Once a bell is in range, they steer toward it.

### Spawn rate

Every **30 seconds** of in-game time, the engine checks every Village Bell. If there's no Wandering Villager within ~32 blocks, one spawns about 12 blocks away from the bell. So you'll see them appear in pairs and trios as time passes.

### Migration

A Wandering Villager within **64 blocks** of an unclaimed bell pathfinds toward it (well — the AI just turns its facing toward the bell; the wander state does the walking). It's not a precise A* path — they bumble in the right direction, getting stuck on terrain sometimes.

When a Wandering Villager gets within **2.5 blocks** of the bell, it **converts**:

1. The Wanderer despawns.
2. A regular **Villager** spawns at the bell's air column.
3. The bell's cell is registered as a **village anchor** — this happens on the very **first** conversion, not the third. From this point onward, workstation claims, quest dialogue, and reputation are all live, even with just one villager. The **Knight defender** is the one system that specifically waits for ≥3 claimed villagers + ≥5 houses (see below) — everything else starts at villager #1.

A toast confirms each conversion (currently silent in alpha; toasts may be added later).

### One villager is enough to start, three unlocks a defender

Your very **first** villager conversion registers the village anchor — from that point on it's a real village:

- The villager will claim a profession when they wander near a workstation (Workbench, Campfire, Tilled Soil, and so on).
- They'll start offering quests.
- Your reputation with this new village starts at Neutral.

The one thing that needs more than a single villager is the **Knight defender** — a Knight auto-spawns only once you have ≥3 claimed villagers + ≥5 houses in range (build the houses yourself or the village stays without a defender).

## How long does it actually take?

Realistically, **a few in-game days** of patience. Each migration cycle is 30 seconds; pathing takes a minute or two; conversions trickle in.

You can speed this up by:

- Standing near the bell so the chunks stay loaded.
- Building cobblestone "houses" around the bell (the engine doesn't currently check house quality for migration, but it'll matter for Knight auto-spawn).
- Placing workstations early so the wanderers convert into professional villagers immediately.

## Why bother?

A few good reasons:

1. **Your own village.** Place it next to your base, your mine, your farm. Quest-givers in walking distance forever.
2. **A second source of sats + reputation.** If your starting village is hostile (or just far away), found a new one in friendlier territory.
3. **Pure construction.** Building a village from scratch is its own gameplay loop — placing houses, putting in a well, lighting a campfire.

## Limits + caveats

- **Placing two bells close together doesn't merge into one bigger village** — the engine doesn't currently dedupe nearby bells, so both register and both can pull in wanderers. Stick to one bell per village site to avoid splitting your migration.
- Bells **can't be broken by other players** unless you ship a multiplayer ownership system (not in alpha). For now, treat any bell on your single-player world as yours.

## Tips

- **Place the bell next to existing structures** — a mine entrance, a farm, a base — so the village grows around your stuff.
- **Build a starter house or two before the wanderers arrive.** Cobblestone walls + planks roof + door + bed = a functioning house. Five of those + 3 villagers + a Knight will materialise within minutes.
- **Don't break your own bell while wanderers are migrating.** They'll lose their target and just wander aimlessly again until a new bell goes up.

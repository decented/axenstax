# Cheats & Op Commands

Some commands are "**cheats**" — they break the survival contract,
so the engine marks them in the **World Integrity Ledger** the first
time they fire. The ledger is a per-world flag; it never resets,
even if you later play strictly straight. The world is permanently
flagged as "this player used cheats."

## Why the Ledger?

So players can prove their accomplishments. A diamond pickaxe in a
ledger-clean world is **earned**; the same pickaxe in a ledger-
flagged world might be a `/give`. The flag doesn't disable any
gameplay — it's signal, not punishment.

## Op level

Commands have a `min_op_level`. The default is **Op** — most commands
need it. A handful of purely informational commands (`/help`, `/seed`,
`/biome`, `/biomes`, `/mobs`, `/tools`, `/armour`, `/reach`,
`/timestep`, `/recipes`, `/complexitytier`, `/tradevalue`, `/mailbox` — the last only when tester feedback is switched on)
explicitly opt out to **None**, so anyone can run them. Non-op players
who try an Op-only command get an "unauthorised" toast.

On a single-player world, you're always Op. On a multiplayer LAN
server, the host is Op by default; other players need an `/op
<name>` from the host (coming post-alpha).

## Cheat commands (mark the ledger)

| Command | What |
|---|---|
| `/time set <day\|noon\|night\|midnight\|0..23999>` | Set the world clock |
| `/time speed <1..64>` | Set time-of-day speed (4 = alpha default; 1 = real-time) |
| `/gamemode <survival\|creative\|spectator>` | Switch mode |
| `/tp <x> <y> <z>` | Teleport to coords |
| `/clear` | Empty your inventory |
| `/clearitems` | Like `/clear` but only for materials |
| `/give <item> [count]` | Add an item to your inventory |
| `/heal` | Restore your HP to full |
| `/kill` | Kill yourself (useful for testing respawn) |
| `/killall` | Kill every mob in the loaded chunks |
| `/spawn <mob>` | Spawn one mob next to you (cow, pig, sheep, chicken, brigand, marauder, berserker, wolf, horse, rabbit, goat, bee, squid) |
| `/spawnpoint` (also `/sp`) | Set your bed-respawn to your current position |
| `/waypoint add\|list\|remove\|tp <name>` | Manage map waypoints |

## Non-cheat commands (free for everyone)

| Command | What |
|---|---|
| `/help` | List commands you have access to |
| `/help <cmd>` | Show usage for a specific command |
| `/seed` | Print the world seed |
| `/biome [x] [z]` | Print the biome at your position or coords |
| `/biomes` | List every biome the engine knows |
| `/mobs` | List every mob type + stats |
| `/tools` | List every tool tier × type + stats |
| `/armour` (or `/armor`) | List every armour piece × tier |
| `/reach` | Print the reach + attack-reach constants |
| `/timestep` | Print the current world-time step + day length |
| `/recipes [filter]` | List known recipes (substring filter) |
| `/complexitytier` | Print the complexity tier of the held item |
| `/tradevalue` | Print the default trade value of the held item |
| `/mailbox` | Alpha-tester feedback only (off by default: Settings in the lobby or pause menu, tap the version line 7 times). See the status of your `/bug` / `/idea` reports: Sent, Received, Fixed in a version, or Won't fix (native app only). We never message players. |

## Newer non-cheat commands that still need Op

`/online` (show/copy your invite link) and `/room` (attach world chat to an outside room) are **not** cheats — but unlike the table above, they need **Op** to run, because they're hosting decisions, not information anyone should be able to twiddle. Both are **native app only**. See **[Play with a friend online](play-with-a-friend-online.md)** and **[Chat & Commands](chat-and-commands.md)**.

## The "pure-survival broken" flag

There's a stricter version of the cheat flag called **pure-survival
broken**. This fires the first time the player ever:

- Switches to **Creative** mode (Spectator does **not** break it)
- Uses `/clearitems`, `/killall`, `/gamemode` (switching to creative),
  `/kill`, `/heal`, `/give`, or `/spawn`

`/clear`, `/tp`, and `/time` adjustments mark cheats but don't break
pure-survival (they're sandbox-y enough). Future game modes might
gate awards on the pure-survival flag staying clean.

## How to inspect

Both flags live in `WorldMeta` and round-trip through save/load:

- `cheats_used_marker` (bool)
- `pure_survival_broken_marker` (bool)

The lobby world card displays a small badge if either is set. Future
post-alpha: a dedicated `/ledger` command to inspect the world's
history.

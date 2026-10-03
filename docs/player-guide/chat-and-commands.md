# Chat & Commands

Press **T** in-game to open the chat overlay. Type a message (or a `/command`) and hit **Enter** to send. This page covers what the overlay looks like, the slash-commands available, and a few tips on using it.

## Opening + closing chat

| Input | What |
|---|---|
| **T** | Open chat with an empty input field. |
| **/** | Open chat with `/` already typed (for fast slash-command entry). |
| **Enter** | Submit the current input. |
| **Esc** | Close chat without submitting. |
| **↑ / ↓** | Scroll through your previous commands (history). |

While chat is open:

- Movement is **disabled** — WASD won't walk you off a cliff.
- Mouse clicks **don't break or place** blocks.
- The cursor is **freed** so you can click the chat window.
- Pressing **Esc** closes chat first; a second Esc opens the pause menu.

When chat closes, any held movement keys re-engage — if you were holding W when you opened chat, you don't stay frozen.

## The chat overlay

Bottom-left of the screen. Three regions:

1. **Log** — past messages + command outputs. Scrolls upward as new ones arrive.
2. **Input** — the text field where you type.
3. **Hint** (when relevant) — `/help` shows a list of commands; some commands show usage hints inline.

**Single-player** (no server at all): chat is local — only you see it, same as always.

**Multiplayer** (hosting or joined, **native desktop app only**): a plain line you type is real **world chat** now — it travels to the other players over the game connection itself, not through us. Two rules gate it:

- **You need a verified sign-in.** If you're playing without signing in with Signet, your lines don't go anywhere — no verified identity, no chat, on either end.
- **Recognition levels decide who hears what.** Family (**Kin**) and mutually-added friends (**Kith**) can talk to you and you to them. Someone you've merely recognised (**Ken** — you pinned them, they haven't pinned you back) can be *heard* but can't be *spoken to* — recognising someone doesn't hand them a channel to you. Everyone else is a stranger: no chat either way.

There's **no in-game voice chat** — this is text only. The **web taster has no chat at all**, in either direction — it's the anonymous local sandbox, so there's no server-side identity for chat to hang off. See **[Multiplayer (LAN)](multiplayer-lan.md)** and **[Play with a friend online](play-with-a-friend-online.md)** for how the connection itself works.

## Slash commands

Type `/` (or use the `/` shortcut). The engine has a small set of built-in commands.

### `/help`

Lists all available commands with one-line descriptions. The starting point.

### `/time <verb> [value]`

Controls the world day/night cycle.

- `/time set day` — jump to midday.
- `/time set night` — jump to midnight.
- `/time set <0-24000>` — jump to a specific tick of the in-game day.
- `/time speed <1-64>` — set the day-length multiplier. **Default 4** (alpha-fast 5-minute day). `1` = 20-minute Minecraft-standard day; `64` = blink-and-it's-gone.

### `/gamemode <survival|creative|adventure|spectator>` (also `/gm <s|c|a|sp>`)

Switch your game mode mid-session. Survival → Creative gives you invulnerability + flight; Creative → Survival drops you back into the survival rules. Adventure and Spectator are the other two modes.

The world tracks "has ever been Creative" — so a kid who switches to Creative once leaves a mark on the World Integrity Ledger that survival-purist achievements (future) will check.

### `/tp <x> <y> <z>` (also `/teleport`)

Teleport to a specific coordinate. It needs exactly three numbers — there's no player-name form (`/tp Alice` isn't supported).

- `/tp 100 80 -50` — teleport to (100, 80, -50).
- `/tp 0 64 0` — teleport to world origin at surface y=64.

### `/give <item> [count]`

Spawn an item into your inventory.

- `/give iron_pickaxe` — one iron pickaxe.
- `/give wheat 64` — a full stack of wheat.
- `/give campfire` — one campfire block.

A "cheats used" marker is added to the World Integrity Ledger when `/give` runs. Survival-purist achievements account for it.

### `/clear`

Empties your inventory — all 36 slots. It does not touch the chat log.

### `/clearitems` (also `/clearground`)

Despawns every floating item entity in the world. Useful when a death spilled a hundred items and you want a clean slate.

### `/heal`

Restores your health to full. Doesn't affect hunger.

### `/kill` (also `/suicide`)

Kills yourself instantly. Triggers the normal death + respawn loop — you drop your inventory at the death location and respawn at your spawn point.

### `/killall`

Kills every mob in the world. Useful for testing. Mobs drop their normal loot.

### `/seed`

Prints the world's seed to chat. Useful when you want to share a world layout with a friend.

### `/spawn <mob>`

Spawns a mob at your position. Recognised mobs: `cow`, `pig`, `sheep`, `chicken`, `brigand`, `marauder`, `berserker`, `wolf`, `horse`, `rabbit`, `goat`, `bee`, `squid`. Villager/Knight aren't on the command list — they spawn through their natural village mechanic.

### `/spawnpoint [x y z]` (also `/sp`)

Sets your respawn point. With no args, sets it to your current position. With three coordinates, sets it to that block.

### Other commands

A handful of other registered commands aren't detailed here — see `/help` for the full, current list. Worth knowing about:

- **`/bug`** and **`/idea`** — the game's player-feedback mechanism, an **alpha-tester feature that is off by default**. To switch it on: open **Settings** (the button at the top of the world list, or the pause menu in a world) and tap the version line ("AxeNStax v…") 7 times in a row; a **Tester feedback** checkbox then appears, and unticking it turns feedback off again. Until then these are unknown commands and don't appear in `/help`. Once on, use them to send a bug report or an idea straight to the devs (**native app only** — the browser version has no feedback channel, so on the web these are always unknown commands). Reports are end-to-end encrypted and sent from a one-time key with no account or name attached, and they carry the game's build version automatically so the makers can tell whether a bug is already fixed in a newer build. We never reply to or message players.
- **`/mailbox`** — (tester feedback only, see above) "Your reports": each `/bug` / `/idea` you sent from this computer, with its status — Sent, Received, Fixed in vX, or Won't fix (**native app only**). Nobody messages you: the game checks a public list of scrambled ticket numbers that only your own game can recognise. The list is kept on your computer for 180 days.
- **`/online`** (also `/online copy`) — **native app only.** Shows this world's invite link (or copies it) so a friend in another house can join it. See **[Play with a friend online](play-with-a-friend-online.md)**.
- **`/room`** — **native app only.** Attaches this world's chat to an outside KithMoot room so it also reaches, say, a parent's phone. `/room join <link>`, `/room leave`, `/room invite`.
- **`/trial`** — powers the **Challenge Board** (the **J** key, see **[Controls](controls.md)**).
- **`/waypoint`** — manage map waypoints, see **[Maps & Coordinates](maps-and-coords.md)**.
- **`/buildguide`** — the build-along guide. `/buildguide <plan> [block|layer|whole]` projects a saved blueprint as a ghost to build along with, `/buildguide mode <block|layer|whole>` switches how an active guide steps you through it, and `/buildguide off` clears it. See **[Build Schematics](build-schematics.md)**.
- Also registered: `/keepinventory`, `/we` (also `/worldedit`), `/import` (also `/importschem`), `/exhibit`, `/place`, `/spawncart`, `/market`, `/scenario` (also `/experience`), `/ws` (also `/workshop`).

When new gameplay systems ship, new commands often appear. **`/help` is always the source of truth** — run it after every update to see what's new.

## Cheats marker

Some commands (`/give`, `/gamemode creative`, `/time set`, `/tp`) flip a `cheats_used` flag in the World Integrity Ledger. The ledger is a metadata record on the world that says "this save has used cheats". It's there for kid-honesty reasons — survival-purist achievements (future) check the flag.

You can't un-flag a world once cheats have been used. Fork the world first if you want a clean copy to try things on.

## Tips

- **`/help` is your best friend.** Run it any time you forget what's available.
- **`/time set day` is the easiest "I died and now it's pitch black" rescue.** Day, mob spawns slow.
- **World chat needs the desktop app and a sign-in.** No verified identity, no chat — and the web taster doesn't have chat at all.
- **The slash-command parser is forgiving** — typos in command names will get a "command not found" line in the log rather than crashing. Try `/help` then look.
- **Past commands are reachable with ↑ / ↓**. Re-running the same `/give` four times is just four presses of ↑ + Enter.

# Split-Screen (Couch Co-op)

Up to **4 players** on one screen, each with their own viewport, their own inventory, their own reputation. Plug in controllers; press A; they're in.

## How players join

1. The host (player 1, on keyboard + mouse) starts a world normally.
2. A second player picks up a **controller** and **presses A** (Xbox) or **X** (PlayStation).
3. The screen splits — player 1 keeps their view; player 2 gets a new viewport.

A 3rd player presses A → 2×2 grid layout. A 4th → fills the bottom-right quadrant.

If a controller disconnects mid-game, that player freezes in place. Reconnect the same controller → they resume. The split layout stays.

## Layouts

- **1 player** — full-screen viewport.
- **2 players** — horizontal split (top / bottom).
- **3 players** — 2×2 grid; the bottom-right quadrant shows a "Press A to join" prompt.
- **4 players** — 2×2 grid.

The crafting UI + dialogue UI fit fine in any quadrant down to 600×360.

## Per-player state

Each kid has their own:

- **Position + camera + look**.
- **Inventory + hotbar**.
- **Health + hunger**.
- **Reputation with every village**.
- **Quest progress + active quests**.
- **Kill counter**.

They share:

- **The world** — blocks, mobs, villages, weather.
- **Day/night cycle + world time**.
- **Saved state** — saving + reloading keeps everyone's slot data.

## Controllers vs keyboard

- **Player 1** always controls the keyboard + mouse. Always.
- **Players 2–4** use controllers.
- If player 1 also plugs in a controller, the controller input merges into their keyboard input automatically — useful for "I want to look around with the stick but still use 1–9 keys for hotbar". There's no settings toggle for this; it just kicks in whenever you're playing solo with an unclaimed gamepad plugged in.

## Performance

Split-screen is rendering 2–4 viewports per frame. On the Intel Iris Xe (the dev machine), 1080p quad-split is comfortably 60 FPS. Older hardware may need to drop window resolution.

The HUD elements shrink to **0.75× scale** when the per-player viewport is narrower than 720 px, so 4-player 1080p windows still look clean.

## Save / load with split-screen

A 2-player save can be loaded as a 2-player session OR as a 1-player session (player 2's slot data is preserved but they don't appear in the world until they press A to rejoin). The world remembers everyone's last position + inventory across loads.

## Cooperating

- **Q-drop is the easy way to share items.** One player drops, the other picks up — instant for the receiver (1.5 s pickup delay only blocks the dropper).
- **Reputation is per-player.** If you trash a village, your friend keeps their Beloved standing. Plan accordingly.
- **Quest items are per-player.** Two players questing the same villager can both accept the same quest in parallel — each turns it in independently.

## Future

- Player names on top of player heads (currently shows the entity ID).
- Per-player toasts (right now toasts share one widget — only the most recent shows).
- Cross-player damage / friendly fire toggle.
- Per-player chat / whisper.
- More than 4 players (over LAN — not local split-screen).

## Tips

- **Pair a young kid (P2) with a parent (P1).** Parent runs the keyboard for menus + crafting; kid runs a controller for movement + interact.
- **Don't kill villagers in front of your co-op partner's village.** Reputation hits the killing player, not the partner — but the village's villager-count drops for everyone.
- **Quadrant assignment is deterministic by join order**. P1 = top-left, P2 = top-right, P3 = bottom-left, P4 = bottom-right. Know your corner.

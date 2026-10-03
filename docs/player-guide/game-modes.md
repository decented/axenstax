# Game Modes

When you create a world, you pick a game mode. It changes how the game plays at a fundamental level. There are four modes — Survival, Creative, Adventure, and Spectator — plus a difficulty knob you can change later. This page covers Survival and Creative in detail, since those are what most kids play day to day. The other two: **Adventure** locks the world read-only (no breaking/placing blocks, but doors/mobs/trade still work) — it's built for scenarios, challenges, and tutorials where you don't want a build wrecked. **Spectator** is a no-clip fly-through observer mode with no interaction at all.

## Survival (default)

The full game. Recommended for everyone the first few times.

- You have **10 hearts** (20 health points). Lose them all → death + respawn at your spawn point + items drop at the death site.
- **Hunger ticks down** as you play. Eat food to keep it up. Below half hunger, no health regeneration.
- **Tools have durability.** They break after enough uses.
- **Blocks take time to mine.** A wooden pickaxe on stone is several seconds; a diamond pickaxe is fast.
- **Hostile mobs** appear at night and in dark places. Bandits (Brigands, Marauders, Berserkers) raid; Bears and Hyenas hunt — and Hyenas come in packs.
- **Inventory starts empty.** You start with nothing — every item you have, you earned.
- **No fall damage yet.** Fall damage is a planned feature but there's no code for it in the engine today — drop from any height and you'll land fine.

## Creative

For building, exploring, or just learning the controls. No survival pressure.

- **Invulnerable.** Nothing damages you. No hunger drain. Hearts and drumsticks stay full.
- **Fly.** Double-tap **Space** in mid-air to enter / exit flight. Space = up, Shift = down.
- **Instant break.** One left-click breaks any block (except bedrock).
- **Infinite hotbar.** Selected blocks don't decrement when you place them.
- **Starter set of blocks** auto-fills your hotbar so you can start building immediately.

Switching from Survival to Creative isn't a permanent commitment — you can change it in the pause menu (Esc → Game Mode toggle). But:

- The game tracks whether you've **ever been Creative on a world** in the World Integrity Ledger. Survival-purist achievements (if/when they ship) won't apply once you've Creative'd, even back in Survival.

## Difficulty (Survival only)

In Survival, there's a difficulty knob — accessed via the pause menu while playing. Four levels exist, but today the engine really only distinguishes **Peaceful** from everything else:

| Difficulty | Hostile mob attacks | Notes |
|---|---|---|
| **Peaceful** | Suppressed — mobs can't damage you | The one difficulty that actually changes anything right now. |
| **Easy** | Standard | Functionally identical to Normal/Hard today. |
| **Normal** (default) | Standard | Functionally identical to Easy/Hard today. |
| **Hard** | Standard | Functionally identical to Easy/Normal today. |

Two things worth knowing: **mob spawning isn't controlled by difficulty at all** — it's gated by separate world-settings flags, so Peaceful doesn't stop mobs from appearing, it just stops them hurting you. And **Easy/Normal/Hard don't yet scale damage differently** — that per-difficulty damage tuning is planned but not built. Peaceful is genuinely a low-stress mode; the other three levels are the same experience under different names for now.

You can change difficulty mid-world via the pause menu (Esc). The world remembers your choice.

## Picking your first mode

- **Kid playing for the first time:** Survival on Normal. The threat is what makes the game feel like a game.
- **Kid who wants to build a castle without worrying about mobs:** Creative. Get out the planks and have at it.
- **Kid getting stressed by hostile mobs:** Switch to Peaceful from the pause menu. Easy fix.
- **Kid showing off to a friend:** Survival on Hard, fully kitted out. Brag rights.

## Game Mode in multiplayer

When a host opens a world for LAN play, the game mode applies to **all players** — the host's setting. Sats are switched off for everyone regardless — see [Bitcoin & Sats](bitcoin-and-sats.md).

Since Easy/Normal/Hard are functionally the same today, the difficulty pick mostly matters for whether it's Peaceful or not. Pick something everyone enjoys.

## Things that don't change

Regardless of mode:

- **Bitcoin / sats.** Proof of Play hashes still run. Quest sats still pay. The Charter parent flag + server policy still apply.
- **Villages.** Villagers, quests, reputation all work in both modes. Killing a villager still costs reputation in Creative.
- **Saves.** Everything persists across save/load in both modes.

## Tips

- **Switch to Creative briefly to learn block recipes.** With instant-break + infinite hotbar you can try every recipe in 5 minutes, then switch back to Survival to play "for real".
- **Peaceful is great for first-time farmers.** No interruptions while you set up your wheat field.
- **Hard doesn't currently hit harder than Normal or Easy.** They're functionally identical today — the only difficulty setting that changes anything is Peaceful, which turns off mob attack damage entirely.

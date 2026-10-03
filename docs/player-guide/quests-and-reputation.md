# Quests & Reputation

Right-clicking a villager opens a dialogue. Inside it: a quest offer + your current reputation with that village. This page explains exactly how the quest loop works and how reputation shifts your rewards.

## The three quest flavours

Every quest belongs to one of three types:

| Flavour | What | Example |
|---|---|---|
| **Fetch** | Bring N of an item to the villager | "Bring me 5 × Wheat" |
| **Make** | Craft N of an item (currently same as Fetch — checks your inventory) | "Make me 3 × Bread" |
| **Kill** | Slay N of a mob type | "Kill 3 × Brigand" |

The villager's profession determines the **pool** of quests they draw from:

- **Farmer** — Fetch Wheat/Carrot/Potato, Kill Brigand.
- **Cook** — Fetch Raw Beef/Wheat, Make Bread.
- **Carpenter** — Fetch Sticks/Wool, Kill Brigand.
- **Blacksmith** — Fetch Iron Ingot/Coal.
- **Scribe** (when bookshelves ship) — Fetch Feathers, Kill Marauder.
- **Miller**, **Baker**, **Brewer** — profession-specific quest pools also exist, though their workstations (Mill, Oven, Aging Rack) aren't functional yet.

## The dialogue UI

When you right-click a villager, the screen darkens with a black overlay and a 480×220 panel appears in the middle. Three modes:

### Offer mode (you haven't accepted yet)

- Villager name + profession.
- Quest summary: **"Bring me 5 × Wheat. Reward: 12 sats + +5 rep."**
- Three buttons: **Accept** / **Decline** / **Close**.
- The italicised line below is the villager's gossip.

### In Progress mode (you've accepted the quest)

- Same villager info.
- Quest summary + **progress**: **"Bring me 5 × Wheat. Reward: 12 sats + +5 rep.  Progress: 3 / 5."**
- One button: **Close**.

The progress updates live — go gather the wheat, come back, the line shows your current count.

### Ready to Turn In (quest complete)

- Two buttons: **Turn in** / **Close**.
- Hit **Turn in** → resources consume, rewards pay, dialogue closes.

## Accepting

Click **Accept**. A toast confirms: *"Quest accepted. Come back when you've finished."* The quest is now bound to you for that villager.

You can have **one active quest per (player, villager) pair**. If you talk to a different villager, you can accept their quest separately. So a kid with three villagers can have three active quests in parallel.

## Declining

Click **Decline**. The villager won't offer you a quest again for **5 minutes** (in-game time, real-time roughly — 6000 ticks at 20 TPS). It's a polite "no thanks" gate so you don't accidentally lock yourself out, but you can't spam-reject either.

## Tracking progress

### Fetch + Make quests

Progress = how many of the target item are in your inventory. Carry it around; the dialogue's progress line reads your current count when you open it.

### Kill quests

When you accept the quest, the game snapshots your current kill count for the target mob. Future kills add to the delta. So **kill 3 Brigands after accepting** = quest done.

Other players helping you with kills currently don't share credit — each player's `kill_counter` is separate, and the kill is attributed to whichever player is closest when the mob dies.

## Turn in (and what happens)

When you have the items (or kills) the quest asks for, talk to the **same villager** again. The dialogue switches to **Ready to Turn In** mode.

**Click Turn in.** The engine:

1. **Consumes the resources.** For Fetch/Make, it removes exactly the required count from your inventory (no wasted overage). For Kill, nothing to consume.
2. **Pays the items.** Most quests pay sats + rep only, but if a quest specifies item rewards (e.g. "Receive 1 Bread"), they go into your bag. If your bag is full, the items drop at the villager's feet.
3. **Pays the sats.** See "Reputation × sats" below for the multiplier. The amount appears in the completion toast: *"Quest complete! +5 rep + 12 sats (sandbox)"*.
4. **Adjusts reputation.** Your standing with this village increases by the quest's rep value.
5. **Clears the active quest** so you can accept the next one.

## Reputation

Each player has a reputation score with each village, from -100 to +100. The HUD panel in the bottom-left shows it when you're in range of a village.

### Tiers

| Tier | Range | Sats × | Other effects |
|---|---|---|---|
| **Hostile** | ≤ -50 | 0.00 | Villagers refuse. |
| **Wary** | -49 to -10 | 0.75 | Reduced payouts. |
| **Neutral** | -9 to 9 | 1.00 | Baseline. |
| **Friendly** | 10 to 49 | 1.10 | +10 % payouts. |
| **Beloved** | ≥ 50 | 1.25 | +25 % + bonus quests (future). |

### Reputation × sats

The multiplier applies to the **sats reward** of every quest — but that's not the last step. After the reputation multiplier, the village takes a **20% treasury skim** off the top, so the number that actually lands in your wallet is lower than a naive tier × base calculation. A 12-sat base quest at:

- **Wary** pays 8 sats.
- **Neutral** pays 10.
- **Beloved** pays 12.

The multiplier is read **before** the rep gain from the current quest — so completing a Friendly-tier quest pays the Friendly rate (minus the treasury skim), then your rep ticks up (possibly into Beloved for the next one).

### Gaining reputation

- Complete a quest → **+5 to +17 reputation**, depending on the quest (the Brewer's Bread quest tops the range).
- Higher-effort quests (Kill, Make) pay more rep than simple Fetch.

### Losing reputation

- Kill a villager → **-25 reputation**, one penalty per 30 seconds. A rage-loop costs once.
- Natural decay → **1 point toward zero per in-game day**. Your past mistakes (and triumphs) fade.

### Hostile

If you fall below -50 with a village, the dialogue won't open at all. The villager turns away. You'd need to wait for the daily decay to climb back to Wary, then start questing to rebuild trust.

### Persistence

Reputation **persists across save/load**. The 20 minutes you spent grinding to Beloved aren't wiped on quit.

## What sats are

Quest rewards are scored in "sats" — but today, on every server, that's just an in-game number. Nothing converts to real money, there's no wallet, and no controls exist yet to change that. See **[Bitcoin & Sats](bitcoin-and-sats.md)** for the kid-friendly version and what's still to come.

Either way the **items + reputation always pay out**. Even a Barter-mode kid gets the full gameplay reward — the sats line is just suppressed.

## Tips

- **The Carpenter's stick quest is the easiest first quest.** 16 sticks for 10 sats + 5 rep. You'll have sticks coming out of your ears anyway.
- **Don't accept three Fetch quests for the same item from three villagers** — the quest consumes the items at Turn In, so the third villager will be left short. Stagger them.
- **Building reputation pays compound interest.** Friendly → +10 %. Beloved → +25 %. A few extra quests to get there pays for itself.
- **Don't kill villagers.** 25 rep takes a handful of quests to earn back, and you've wasted an NPC who would've paid you.
- **Check the HUD tier colour** before quest-grinding a village. If you're Wary or below, your payouts are reduced.

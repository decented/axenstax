# Nostrich — the purple-ostrich mascot

A purple ostrich that lives on the Savanna. It is the **Nostr mascot**, embedded in the world. It lays the biggest eggs in the game, sheds beautiful purple feathers — and **it does not like being hunted**.

> **Status:** Nostrich v2 shipped. Wild Nostriches spawn in Savanna, lay eggs, and can be **tamed by hand-feeding berries**. Tamed Nostriches sit/stand on command, shed feathers passively, lay eggs, and are exempt from the Vow. **Riding is already live and needs no saddle** — there's no saddle item in the game, and no taming step required either; right-click *any* Nostrich, wild or tame, to mount and ride it. Taming/berries are for keeping and feeding a bird, not a prerequisite for riding it. The **walk-and-follow trail is live** (2026-07-11): a tamed, standing Nostrich pads after you across the map.

## Finding a Nostrich

Nostriches spawn in **Savanna** only, in family groups of 2–3. They're rare — about 1 in 19 Savanna passives (horses still dominate). Look for the purple silhouette against the tan grass.

They run faster than you sprint (8 blocks/s vs your ~6 b/s). If they get startled, you won't catch them.

## Riding

Right-click a Nostrich — **wild or tame, no taming needed first** — to mount and ride it instantly. There's no saddle item anywhere in the game; mounting only checks that the mob is a rideable type. Full ride physics apply: speed builds up as you ride, it drifts a little on turns, and it can skim across water. Taming with berries is about *keeping* a Nostrich (feathers, eggs, following, exemption from the Vow) — it's not a gate on riding.

## What they drop

| Item | When | Quantity |
|---|---|---|
| **Nostrich Egg** | Wild Nostriches drop one every ~20 real-minutes (1 in-game day @ 1× time speed) onto the ground near them | 1 |
| **Nostrich Feather** | On kill (and from molt in v2) | 1–3 |
| **Raw Nostrich Meat** | On kill | 0–1 |

The **Nostrich Egg** is the game's most valuable egg. It's worth **21 chicken eggs** by food yield — a deliberate Bitcoin nod (21 million BTC cap). Eating one raw heals 21 hunger; cooking turns it into a much better recipe (see below).

## The Nostrich's Vow — don't kill them

The Nostrich is the Nostr mascot. Killing one (or eating Nostrich meat) triggers **The Nostrich's Vow** — a 20-real-minute curse that:

- **Blocks Vendor Block trades** — vendors refuse to deal with you ("The Nostriches will not aid your dealings.")
- **Zeroes village reputation** — villagers turn hostile until the vow lifts
- **Pauses sats payouts** — on Bitcoin-enabled servers, every payout (Proof-of-Play, quests, vendor sales, plaque tips) is suppressed. Mining still happens; settlement is held.

When the vow triggers, you'll see: *"The Nostriches will remember this…"*

### Lifting the vow

Two ways:

1. **Wait it out** — 20 real-minutes (24,000 ticks @ 20 TPS).
2. **Build a Memorial** — place a 3×3 of OAK_PLANKS in the world. Any player who completes the pattern lifts their own vow instantly. You'll see *"The Nostriches' Vow is lifted. Welcome back."*

> When the dye system ships, the memorial pattern upgrades to purple wool + feathers + an egg in the centre. For now, oak planks works as a placeholder.

### Eating Nostrich meat

If you eat raw or cooked Nostrich meat, the vow triggers *and* you get a different flavour toast: *"The meat is sour. The Nostriches notice."* The meat only heals 1 hunger — it's tough and sour by design. Don't eat the mascot.

## Egg recipes

### Nostrich Omelette

`NostrichEgg + Flour + Wheat` (horizontal, in any row) → **1 Nostrich Omelette**.

Heals 12 hunger. The premium breakfast. High trade-value (T4 tier).

### Future recipes (Pavlova, Custard)

Royal Pavlova and Nostrich Custard are spec'd but not yet recipe-wired. They land when the dye system + the rest of the high-tier dessert ladder ships.

## Feather recipes

### Nostrich Arrow

`Stick (top) + Flint (middle) + NostrichFeather (bottom)` vertical → **6 Nostrich Arrows**.

> **Coming soon:** the Nostrich Arrow crafts today, but its planned +20% range / +20% damage bonus over a regular Arrow isn't wired up yet — a Bow only ever loads a plain **Arrow** stack, so Nostrich Arrows currently just sit in your bag. Craft them for the collection, not the buff.

### Purple Banner

`Wool (top) + NostrichFeather (bottom)` vertical → **1 Purple Banner**.

Decorative. Hang one on the side of a Vendor Block to mark "Nostr-friendly" trade. Trade-value 25 (T2 tier).

## Taming a Nostrich (v2)

Right-click a Nostrich while holding **Berries** in your active hotbar slot. Each feed has a **1-in-3** chance to tame. Failed feeds still consume the berry — like wolf bone-taming, the friction is the cost. Expected ~3-7 berries on average; up to ~21 on the unlucky tail.

You'll see one of these toasts:

| Toast | What |
|---|---|
| `"The Nostrich eyes you carefully…"` | Berry consumed; tame roll failed. Try again. |
| `"Tamed! The Nostrich is yours."` | Bonded. The Nostrich flips to Follow mode. |
| `"This Nostrich is already tame."` | Already tamed (by anyone). No berry consumed. |

Hold onto a stack of ~10 berries before approaching to give yourself a comfortable buffer.

### Tamed behaviour

Once tamed:

- **Sit / Stand** — right-click your tamed Nostrich with an **empty hand** to toggle. A sitting Nostrich stays put no matter how far you walk; right-click again to stand it up.
- **Follow mode** — a tamed, standing Nostrich is in Follow mode and **physically pads after you**: it starts walking when you're more than ~4 blocks away and stops when it catches up (so it won't orbit you). Wander more than ~24 blocks away and it gives up and goes back to wild-style idling until you return.
- **Passive feather shed** — every ~40 real-minutes (2 in-game days), a tamed Nostrich drops a NostrichFeather where it's standing. Free purple feathers, no killing required.
- **Egg-laying** — same rate as wild (1 per 20 real-minutes). A tamed bird lays at its home spot rather than wandering off to random Savanna corners. Easy to scoop.
- **Vow-immune** — if a tamed Nostrich is killed (yours or someone else's), it does NOT trigger the Nostrich's Vow on the killer. Tamed birds are "personal" not "mascot". You still lose the bird though; emotional loss is its own punishment.

### Behaviour against attackers

When a wild (untamed) Nostrich is attacked:

- If you're in **melee range** (~2.5 blocks), it kicks back for **6 damage** + flips to Flee state.
- If you're out of range (you shot it with an arrow), it just flees.

Don't underestimate the kick. A tier-1 wood-sword build can be one-shotted by a Nostrich + a single Brigand at night.

## HUD vow badge

When you have an active Vow, a purple-feather badge appears in the **top-left** of your viewport showing the remaining time (minutes + seconds). Disappears when the vow expires or a memorial purifies it.

## Coming in v3

- ~~**Walk-and-follow trail**~~ — **shipped 2026-07-11**; see Tamed behaviour above
- **Tribute path** — donate sats on Bitcoin-enabled servers to purify the vow instantly
- **Cross-world slayer tag** — your kill follows you across worlds via Nostr
- **Royal Pavlova + Nostrich Custard recipes** — luxury T5 desserts

## Why a Nostrich?

The game runs on Nostr — Signet for sign-in, Charter for the parent-permission layer, the relay for cross-server messaging. The Nostrich is the Nostr mascot. Embedding it in the world (with a curse for harming it) is a thematic statement: respect the protocol that powers the playtime.

# Wolves

Wolves are the first tameable companion mob. Find one, feed it bones, and it's yours — it sits or stands on command, and it's the proof-of-concept for every companion mob to come.

> **Status:** Wolves are **fully live**. Spawning, the 33% bone-tame, the owner bond, and the **Sit / Stand** toggle all work in-game, and so does the companion *movement*: a tamed, standing wolf physically follows you, joins you when you pick a fight, and chases down whatever hits you. Follow + combat-assist shipped across the pets-debt-water wave (2026-07-06) and the bug-hardening wave (2026-07-07).

## Finding a wolf

Wolves spawn in:

| Biome | Spawn weight |
|---|---|
| **Taiga** | High |
| **Forest** | High |
| **Snowy Tundra** | Medium |
| Plains / Desert / Jungle | None or low |

Untamed wolves wander around like other passive mobs. They have **20 HP** — same as a player — and a 0.85m height (shorter than you).

## Taming

Hold a **Bone** (drops from Bears, Hyenas, and livestock; 0–2 per kill) and **right-click** the wolf. Each click has a **33% chance** of succeeding. The bone is consumed whether or not it works — the cost is the friction.

On success you'll see *"Tamed! The wolf is yours."* and it's bonded to you. The wolf's owner-pubkey is recorded so save/load + future multiplayer keep the bond intact.

## Sit / Stand

Right-click your own wolf with an **empty hand** to toggle **Sit / Stand**. A sitting wolf stays put no matter how far you walk; stand it back up to bring it along. This toggle works in-game today.

## What a tamed wolf will do (the companion behaviours)

A tamed wolf runs **five behaviours**, and all five — state machine, ownership, and movement alike — are live:

1. **Idle** — sits in place, looks around.
2. **Sit** — toggle Sit/Stand; a sitting wolf stays put no matter how far you walk.
3. **Follow** — when you're more than 8 blocks away, the wolf paths toward you every tick, stopping short so it doesn't bounce at your feet.
4. **Attack hostile** — when you attack a hostile mob, your wolf joins for ~1.5 seconds (30 ticks), landing real contact damage (3.0 HP).
5. **Attack recent attacker** — when something hits you, your wolf chases that attacker for ~4.5 seconds (90 ticks), also landing real contact damage (3.0 HP).

By design a wolf **never attacks another tamed wolf of the same owner** — no friendly fire.

## Drops

| State | Drops |
|---|---|
| **Untamed wolf killed** | 1 Leather + 1–2 Bone |
| **Tamed wolf killed** | **Nothing** |

This is deliberate. Killing your wolf is **loss, not loot**. The emotional weight is the point.

## Where things stand

Spawning, taming, the owner bond, the **Sit / Stand** toggle, and the companion **movement** — follow, and fighting at your side — all work in-game today. The **Inventory Explorer (press B)** already lists "Wolf" with its drop info, and you'll find them wandering any Forest, Taiga, or Snowy Tundra biome.

## Recall Whistle

Craft a **Recall Whistle** (Bone + String) and right-click anywhere to call every pet and mount you own — wolves included — to your side in a small ring around you. It doesn't get used up, just a short cooldown between blows. Handy for gathering a scattered pack before you set off.

## The bigger picture

Wolves were the first consumer of the **tameable-mob framework**. That framework has since been extracted into a generic `tameable.rs` module, and **Cat, Parrot, and Fox** now run on it too as their own tameable companions — see [Combat & Mobs](combat-and-mobs.md) for how to tame them. Wolves were the proof-of-concept; they're not the only companion any more.

See `docs/foundations/2026-05-20-wolves-tameable-companion.md` for the full spec.

# Explosives — Black Powder & the Blasting Keg

The **demolition tier**. You make **Black Powder** the real way — from
**Sulphur**, **Coal**, and **Saltpetre** — pack it into a wooden
**Blasting Keg**, light the fuse, and get clear. Light it by **hand** or wire it
to **Electricity** for remote and trap detonations.

> **New to it?** Do the hands-on lesson first:
> **[Light a Blasting Keg](https://learn.axenstax.com/docs/journey/learn-journey/light-a-blasting-keg.md)**. This page is
> for *looking things up*.

## The one rule: demolition, not mining

A blast **breaks** blocks but gives you **no drops** from them — and **no
Proof-of-Play hash**. The careful **pickaxe** strike is still the *only* way to
actually collect ore and run the Proof-of-Play hash on it. So you can blow a tunnel,
but you **forfeit** whatever was in the wall. **Ore never drops from a blast.**

## The supply chain — three ingredients

Black Powder is the real ~75/15/10 gunpowder recipe. Two of the three ingredients
you already know; this tier adds the third and finally gives **Sulphur** a home.

| Ingredient | Where it comes from |
|---|---|
| **Sulphur** | Mine **Brimstone** (yellow-green crystal ore) with a **stone pickaxe or better** → 1–2 Sulphur. |
| **Coal** | You already have it — mined the normal way. (Charcoal is planned but doesn't exist as an item yet.) |
| **Saltpetre** | Two ways (pick either): mine a **Nitre** deposit (pale white-grey ore, stone pickaxe+), **or** farm it in a **Composter** (below). |

### Saltpetre the farming way — the Composter

A **Composter** is a slatted wooden bin. **Right-click it with plant/food waste**
(seeds, crops, saplings) a few times to fill it; wait, then **right-click with an
empty hand** to collect **Compost**. Feed the **Compost** back in, wait again, and
it ages into **Saltpetre** — the real "nitre bed" chemistry. A miner and a farmer
both reach the keg by their own route.

## The recipes

Open the **recipe book** (it shows both), or craft by hand:

| Make | Recipe | Yield |
|---|---|---|
| **Black Powder** | **Sulphur + Coal + Saltpetre** in a row (any order) | 3 |
| **Blasting Keg** | **8 Planks** ringing **1 Black Powder** in the middle (any wood) | 1 |

> The Keg is **our own barrel** — never a red lettered cube. The middle ingredient
> is what tells it apart from a Chest (empty middle = Chest).

Get any of these for testing with `/give <name>` in chat: `brimstone`,
`nitre_ore`, `composter`, `blasting_keg`, `sulphur`, `saltpetre`, `black_powder`,
`compost`, `magnesium_firestarter`.

## Lighting the fuse

A lit keg burns a **~4-second fuse**, then detonates in place — enough time to get
clear. There are two ways to light it:

1. **By hand** — right-click the keg with the **Magnesium Firestarter**.
2. **By Electricity** — the keg is a power **sink**: a fresh power signal lights
   the same fuse (see below).

A keg caught inside *another* blast **chain-ignites** — line a few up for a
cascade.

## Wiring it to Electricity

The keg plugs straight into the [Electricity](electricity.md) tier — it's just a
**consumer** on the network, lit by a **rising edge** (an off→on pulse). It fires
**once** per pulse, not continuously, so a held-on wire won't keep re-triggering.

- **Plunger Detonator** — the hero input: a boxed T-handle. **Push it** → a pulse
  runs down the cable → every wired keg lights. Craft it from **1 Iron Ingot +
  1 Copper Cable (the material, not the placeable Cable block) + 1 Oak Plank**
  in a horizontal row (iron casing, cable contact, plank base — "a boxed switch").
- **Cables fan out** — one plunger pulse can light **many kegs at once** for
  synchronised demolition. No wire length limit.
- **Any Spec 48 trigger works** — a **Pressure Plate** (step-on trap), a **Beam
  Sensor** tripwire (walk through the beam), a **Motion Sensor** (proximity), a
  **Lever** or **Button**. The keg is a standard sink, so they all just work.
- **Arming (safety)** — wire `Lever AND trigger → keg` through a **Logic Gate
  (AND)**: the charge only fires when your **arm lever is on** *and* the trigger
  trips. Real circuit design, no special keg setting.

## The blast

- **Radius** — a spherical **~4 blocks** (tunable). Soft terrain, stone, wood and
  ore all go.
- **Blast-proof blocks** — **Bedrock** and **Satori** are immune; **deepslate** and
  packed metal/gem blocks shrug off a single keg. Your world floor and the toughest
  blocks hold.
- **Damage** — players and mobs in range take damage that **falls off with
  distance**, and **hiding behind a wall** (line of sight) softens it. Stand back,
  or duck behind cover. **Armour helps too** — blast damage is reduced by your
  equipped armour like any other hit (and wears it accordingly).
- **No fire spread** — detonation doesn't start spreading fire (kid-friendly +
  anti-grief).

## Where it won't go off

Explosives are a **trigger, not a bypass** — the no-drop rule holds however a keg
is fired, and a couple of switches turn it off entirely:

- **`explosives_enabled = false`** — a per-world toggle (off→ a keg just fizzles,
  breaks nothing, hand-lit *or* electrical).
- **Adventure / Spectator** — read-only modes: a keg can't break blocks.
- **Creative** — place + detonate freely.

> Want the deep version (how it's built, the industrial ANFO route, fireworks)?
> That's the spec: `docs/foundations/2026-06-20-explosives-blasting-keg.md`.

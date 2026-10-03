# Light a Blasting Keg

> **Goal:** By the end of this you'll **craft a Blasting Keg, light the fuse, and
> blow a crater** — by hand, then with a plunger and cables like a proper
> demolition crew.

Explosives in Axe'n'Stax are made the **real way** — from chemistry, not from a
monster. One rule to remember: a blast **breaks** blocks but gives you **no
drops**. It's **demolition, not mining** — the pickaxe is still the only way to
*collect* ore.

## 1. Get the parts

Easiest while you're learning — open the chat (**T** or **/**) and type:

```
/give blasting_keg
/give magnesium_firestarter
```

(You can also make them: mine **Brimstone** for **Sulphur** and **Nitre** for
**Saltpetre**, craft **Sulphur + Coal + Saltpetre** into **Black Powder**, then
ring **8 planks** around the powder for a **Blasting Keg**. The recipe book shows
both — search "powder" and "keg".)

## 2. Place the keg somewhere open

Put a **Blasting Keg** down on flat ground, away from anything you care about.
It's a harmless barrel until you light it.

## 3. Light the fuse

**Right-click the keg with the Magnesium Firestarter.** The fuse starts hissing —
you've got about **4 seconds**.

## 4. Get clear!

**Run.** Stand a few blocks back, ideally behind a wall.

*Boom.* The keg blows a **crater about 4 blocks across** — and notice you got
**no blocks back** from it. That's the rule: blasting destroys, it doesn't harvest.

> 🎉 **You did it!** Try blasting next to **bedrock** — the bedrock survives. Some
> blocks (bedrock, Satori, deepslate) are just too tough to break.

## 5. Now do it with electricity

This is the fun part — detonate from a distance.

```
/give blasting_keg
/give plunger
/give cable
```

1. Place a **Blasting Keg**.
2. Run a line of **Cable** from the keg to where you'll stand.
3. Place a **Plunger Detonator** touching the far end of the cable.
4. **Push the plunger.** A pulse runs down the wire and the keg's fuse lights —
   *boom*, from safety.

## Now try these

- **Synchronised demolition.** Run cables to **two or three kegs** from **one
  plunger** — a single push sets them **all** off together.
- **A trap.** Swap the plunger for a **Pressure Plate** or a **Beam Sensor**
  tripwire — now walking into it triggers the blast.
- **A safety switch.** Wire `Lever AND trigger → keg` through a **Logic Gate
  (AND)** so it only fires when your **arm lever** is on *and* the trigger trips.
- **Farm your own powder.** Feed plant scraps into a **Composter** to make
  **Compost**, then compost the Compost into **Saltpetre** — no mining needed.

> **Want the full list** — every block, the recipes, the blast rules? See the
> **[Explosives guide](../../player-guide/player-guide/explosives.md)**.

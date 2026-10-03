# Your First Circuit

> **Goal:** By the end of this you'll have a **lamp you can switch on and off**
> with a lever — your first electrical circuit. The same idea (source → cable →
> thing) powers everything in the Electricity tier.

This is the "redstone" of Axe'n'Stax, but simpler: a wire is just **on** or
**off**, and there's **no length limit** — your cable can run as far as you like.

## 1. Get the parts

Easiest while you're learning: open the chat (press **T** or **/**) and type:

```
/give lever
/give cable
/give lamp
/give crank
```

(You can also **craft** them — open the recipe book and search the names. A
**Lever** is a Stick on Cobblestone, a **Lamp** is Glass / Copper / Glass, and
so on.)

## 2. Lay a line of cable

Place a row of **Cable** blocks in a line — say **4 in a row**. This is your
wire. It looks dark now because no power is flowing yet.

## 3. Put a lamp at the end

Place an **Electric Lamp** touching one end of the cable run.

## 4. Put a switch at the other end

Place a **Lever** touching the **other** end of the cable.

Your line now reads: **Lever → Cable → Cable → Cable → Cable → Lamp.**

## 5. Flip the switch

**Right-click the lever.** The whole cable run lights up (you'll see the copper
core glow) and the **lamp switches on** — and it actually lights up the area
around it, like a torch you can turn off.

**Right-click the lever again** → everything goes dark.

> 🎉 **You did it!** That's a complete circuit: a **source** (the lever) sending
> power down a **wire** (cable) to a **consumer** (the lamp).

## Now try these

- **Swap the switch.** Replace the lever with a **Button** (a quick pulse that
  pops back off) or a **Pressure Plate** (on while you stand on it).
- **No-fuel power.** Put a **Hand Crank** instead and right-click it — it drives
  the circuit for a few seconds per crank. Great for testing.
- **Hands-free power.** A **Steam Generator** runs on fuel: place it, then
  right-click it **with coal or logs in your hand** to load fuel. It powers the
  circuit while it's burning.
- **Two switches, one lamp.** Feed two levers into a **Logic Gate**, then the
  gate into the lamp. By default it's an **AND** — the lamp only lights when
  **both** levers are on.

When you're ready for the really fun one, do
**[Light-Beam Tripwire](light-beam-tripwire.md)** — invisible-ish triplines you
walk through to trigger things.

> **Want the full list** of every power block and what it does? See the
> **[Electricity guide](../../player-guide/player-guide/electricity.md)**.

# Electricity — Power & Logic

The **wired power tier** — Axe'n'Stax's take on redstone. You make
power, run it down **Cable**, and use it to switch lamps, speed up
carts, and build contraptions. It's binary for now: a wire is either
**on** or **off**.

> **New to it?** The hands-on lessons are easier than this reference:
> **[Switch On a Lamp](https://learn.axenstax.com/docs/journey/learn-journey/switch-on-a-lamp.md)** (lamp on a
> switch). The **light-beam tripwire** — the cool one — is covered in the sensors
> section below. This page is for *looking things up*.

## How it works (the one rule)

A run of connected **Cable** lights up whenever **any source touching
it is on**. Touch a lit cable (or a source) to a **consumer** and the
consumer switches on. That's the whole game:

> **source → cable → consumer**

Cables have **no length limit** (unlike Minecraft redstone, which dies
after 15 blocks) — a circuit lights at any distance. Lit cable shows a
glowing copper core; unlit is dark.

Get any of these with `/give <name>` in chat, or craft them (open the
**recipe book** and search the name).

## Sources — they *send* power

| Block | How it sends power | Get it |
|---|---|---|
| **Lever** | Right-click to **latch** on/off (stays where you leave it). | Stick on Cobblestone |
| **Button** | Right-click for a **short pulse**, then it pops back off. | 1 Stone |
| **Pressure Plate** | On **while something stands on it**. | 2 Stone in a row |
| **Hand Crank** | Right-click to drive power for a few seconds (re-crank to keep going). The easy starter source. | Stick / Copper / Plank stacked |
| **Steam Generator** | Burns **fuel** (right-click it with coal/logs in hand) to send power continuously while lit. | 8 Iron ringing 1 Copper |
| **Battery** | Stores power: charges from an adjacent powered cable, then keeps the circuit on for a few seconds after the source stops. | Copper / Coal / Copper stacked |
| **Water Wheel** | Turns whenever **moving** water touches it — next to it, or running underneath — and sends power for as long as the water keeps running. No fuel, no winding. **A pond won't do it:** still water just sits there. Dig a channel from a higher pool so the water has somewhere to run *to*, and put the wheel beside the flow. | 8 Planks ringing 1 Copper Ingot |
| **Windmill** | Turns in the **wind** — needs **open sky** above it (a roof, leaves or even glass blocks it) and clear air on at least two sides. A sea-level mill is intermittent — it comes and goes with the breeze. Build it **high** (a hilltop, a tower) and it runs far more reliably; up near the world's ceiling it barely stops. Storms blow harder than a clear day. No fuel, no winding. | Canvas / Plank / Stick ringing Copper + Iron |

### Wind

Look at a Windmill (or check the **F3** debug overlay) and you'll see a wind
word: **calm**, **light**, **fresh**, **strong** or **gale**, plus a compass
direction. The wind is real weather — it picks up in rain, more in a storm —
and it gets stronger the higher up you build. A Windmill needs at least a
**fresh** breeze to start turning, and only stops once it drops back to
**light**, so it doesn't flicker on and off in a lull.

> ⚠️ **Copper is currently a dead end in Survival.** Cable and almost every
> device on this page needs Copper Ingot somewhere in its recipe (directly,
> or via Cable/Copper Cable) — but Copper Ore doesn't generate underground
> yet, so there's no mining, mob-drop, or `/give` route to raw Copper in a
> normal playthrough. Lever, Button, and Pressure Plate are the only sources
> you can craft from scratch right now. In **Creative**, press **B** and
> click Copper Ingot in the Inventory Explorer to unblock everything else.

## Consumers — they *use* power

| Block | What power does | Get it |
|---|---|---|
| **Electric Lamp** | Lights up (and **actually lights the area**, like a switchable torch) while powered. | Glass / Copper / Glass in a row |
| **Powered Rail** | A normal **Track** with a powered cable beside it makes a passing **cart go ~2× faster**. (No new block — just power the track.) | run Cable next to Track |
| **Piston** | **Shoves the blocks in front of it** one step when powered, and lets go when the power stops. The first machine that *moves the world*. | Planks ×3 on top / Cobble · Iron · Cobble / Cobble · **Cable** · Cobble |
| **Sticky Piston** | Same push as a Piston, but when the power stops it **pulls the block back with it** instead of leaving it out. | Place **Rubber** on top of an already-placed Piston |
| **Dispenser** | 9-slot container. On a power **rising edge**, it ejects one item from its slots — arrows fire as real projectiles, everything else is tossed out. | Iron / Copper shape ringing a **Cable**, mirroring the Piston |
| **Dropper** | Same as the Dispenser (9-slot container, fires on a rising edge), but every item is just tossed out rather than launched as a projectile. | Iron / Copper shape ringing a **Cable**, mirroring the Piston |

## Pistons — the machine that moves blocks

A **Piston** pushes. Power it and an arm shoots out, shoving the line of
blocks in front of it forward by one. Cut the power and the arm pulls back
in (the blocks it pushed stay where they are — that's a plain piston).

- **It faces the way you're looking when you place it** — aim straight ahead
  for a sideways push, look up or down to make it push up or down.
- **Power it like any consumer** — a Lever, Button, Pressure Plate, sensor,
  or Logic Gate next to it (or a Cable carrying their signal) sets it off.
- **What it can push:** plain solid blocks — stone, dirt, planks, logs, wool,
  glass and the like. It **won't** budge bedrock, Satori, or anything with
  stuff inside it (chests, furnaces, other pistons) — those stay put so your
  storage is always safe. If the line of blocks has nowhere to go (a wall
  behind them), the piston just sits still.
- **Build idea:** a hidden door — power a piston to slide a block out of your
  wall, walk through, let go to seal it back up.

**Sticky Piston** works the same way, except when the power cuts and the arm
retracts, it **pulls the pushed block back with it** instead of leaving it
sitting out — handy for a door that closes itself flush, or a block that
needs to come back on cue. Craft one by placing **Rubber** on top of a
Piston you've already placed.

## Logic — the **Logic Gate**

A **Logic Gate** combines inputs like a switchboard relay. It reads the
cables feeding into it and drives its output cable based on its mode:

- **AND** — output on only when **all** inputs are on (the default).
- **OR** — on when **any** input is on.
- **NOT** — flips it: on when the input is **off**.
- **XOR** — on when inputs **differ**.

Craft with Iron / Copper-Cable / Iron in a row.

## Light beams & motion (the sensor blocks)

These are tripwires — perfect for "cross the line to open the door"
traps and parkour gates.

### Beam Sensor

Place one and it shoots a faint **red beam** out its front. The beam
**arms** when it reaches a valid end:

- a **second Beam Sensor** facing back at it (a *through-beam*), or
- a **Mirror** that bounces it back to the sensor (a *retroreflector*).

Once armed, **anything that crosses the beam triggers it** — the sensor
sends power (like a momentary switch) while the beam is broken. Wire it
to a lamp, a door, a Logic Gate — anything. A beam that hits a wall or
just runs off into the open **doesn't arm**, so crossing it does nothing.

Craft with Glass / Copper / Iron in a row.

### Mirror

Bounces beams. **Right-click to cycle** its 5 settings: bounce straight
back (retroreflect), or turn the beam **90°** toward North / East /
South / West. Use a 90° turn to route a beam **around a corner** to a
sensor or another mirror. You'll see the red line follow your routing.

Craft with Iron / Glass / Iron in a row.

### Motion Sensor

No beam — it just trips while **anything is within ~3 blocks**. Good for
auto-lights and "someone's near" alarms.

Craft with Copper / Glass / Copper in a row.

## Tips

- Look at any device to see what it's doing — a label pops up under the
  crosshair showing fuel left, charge, turning/still, or on/off.
- The **Hand Crank** is the fastest way to test a circuit — no fuel needed.
- A **Steam Generator** runs hands-free once you feed it coal.
- Circuits and switch positions **survive saving** — your build comes
  back the way you left it.
- **Copper now comes straight out of the ground** — it generates as ore in
  mid-depth stone, so a full circuit (dig → smelt → wire it up) is a normal
  Survival playthrough. The **Copper Rush** trial on the Challenge Board (J)
  walks through the dig.
- A friend's own switches (their Lever, Button, Hand Crank, Plunger Detonator
  or Mirror) now work on your hosted world, not just their own screen.
- A **Battery** no longer keeps itself topped up off its own wire — it runs
  down for real once the source that was charging it stops. (It still holds
  the circuit for a while after a source cuts, as before.)

> Want the deep version (how it's built)? That's the engineering spec:
> `docs/foundations/2026-06-17-electricity-power-logic.md`.

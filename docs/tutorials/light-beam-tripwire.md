# Light-Beam Tripwire

> **Goal:** Build a **beam you walk through to trigger something** — a light
> that comes on when you cross a doorway, a trap, or a parkour "cross the line"
> gate. By the end you'll know how to aim beams and bounce them off mirrors.

This builds on **[Your First Circuit](your-first-circuit.md)** — do that one
first so "source → cable → lamp" makes sense.

## 1. Get the parts

In chat (**T** or **/**):

```
/give beam
/give mirror
/give motion
/give cable
/give lamp
```

(Or craft them — search the recipe book: **Beam Sensor** = Glass / Copper /
Iron, **Mirror** = Iron / Glass / Iron, **Motion Sensor** = Copper / Glass /
Copper.)

## 2. The simplest tripwire — sensor + mirror

1. Place a **Beam Sensor** on the ground, facing across a gap (it faces the way
   you're looking when you place it). A faint **red beam** shoots out the front.
2. A few blocks ahead, place a **Mirror** so the beam hits it. The beam should
   **bounce straight back** to the sensor — you'll see the red line go out and
   come back. That means the tripwire is **armed**.
3. Wire the sensor to a lamp: run **Cable** from the sensor to an **Electric
   Lamp** (or put the lamp right next to the sensor).

## 3. Walk through it

Step **through the red beam**. The lamp **lights up** — you broke the beam, and
that's the trigger. Step out of the beam and it goes off again.

> 🎉 **You did it!** That's a tripwire: an armed beam that fires whenever
> something crosses it. Wire it to a door, a trap, a Logic Gate — anything.

## 4. Bend the beam around a corner

Mirrors can **turn** the beam, not just bounce it back:

- **Right-click the Mirror** to cycle its setting: bounce-straight-back → turn
  **North** → **East** → **South** → **West** → back to bounce.
- Pick a 90° turn and the beam leaves the mirror in that direction. Put a
  **second mirror** (or a second Beam Sensor) where the turned beam lands to keep
  routing it. Watch the red line follow your setup.

This lets you guard an L-shaped corridor, or send a beam the long way around a
wall.

## 5. The no-aim version — Motion Sensor

Don't want to line up a beam? Place a **Motion Sensor** and wire it to a lamp.
It trips whenever **anything comes within about 3 blocks** — no beam to aim.
Great for an auto-light that comes on as you approach.

## Build ideas

- **Auto-door:** beam across a doorway → wire to the thing you want to open.
- **Parkour gate:** "cross the beam to light the path" timing challenge.
- **Alarm:** Motion Sensor in a room → lamp that flicks on when someone enters.

> **Heads up:** a beam that hits a wall, or just runs off into open air, **does
> not arm** — so it won't trigger. Aim it at a mirror or a second sensor. Full
> reference: **[Electricity guide](../../player-guide/player-guide/electricity.md)**.

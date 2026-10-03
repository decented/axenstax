# The Nostrich Mount — wacky, drifty, water-skimming (Mario-Kart fun)

**Date:** 2026-06-25 · **Status:** Built + shipped (v0.2.9) · **Owner spec:** "Make Nostriches
rideable. Super fast, builds up. At full speed it's so fast it runs over water; if it slows it
sinks and you fall off. The faster it goes the more it drifts. Mario-Kart fun."

## What it is

Nostriches are now a **rideable mount** alongside the horse family — but where the horse is a
steady ~6 b/s gallop, the Nostrich is an **arcade racer**: it winds up to a blazing top speed,
**drifts** harder the faster it goes, and at full tilt **runs across water**. Slow down over
water and it sinks — you fall off with a splash.

Right-click a Nostrich (empty hand) to mount, **hold forward** to wind up, **hold crouch to drift**
(slide at lower speeds), and **crouch when stopped to hop off**.

## The physics (`src/nostrich_ride.rs`, pure + unit-tested)

A pure `step(speed, heading, input_dir, over_water, dt) -> RideStep` runs every tick; `game_loop`
owns the ECS glue (reads input + the water cell under the mount, applies the returned velocity,
pins the bird to the water surface while skimming, dismounts on a sink). Per-ride state
(`ride_speed`, `ride_heading`) lives on the `PlayerSlot`, reset to 0 on mount + dismount.

| Knob | Value | Feel |
|------|-------|------|
| `BASE_SPEED` | 5.0 b/s | brisk trot off the line |
| `MAX_SPEED` | 19.0 b/s | flat-out — ~3.4× the player sprint |
| `ACCEL` / `DECEL` | 7 / 16 b/s² | winds up over ~2s; bleeds off faster (stopping is committal) |
| `WATER_RUN_SPEED` | 14.0 b/s | the threshold to skim water |
| `TURN_FAST` / `TURN_SLOW` | 9 / 2.2 rad/s | snappy at a standstill, **drifty** at speed |
| `DRIFT_HOLD_BOOST` | 0.5 | crouch shifts the drift curve down → slide at lower speeds |
| `DISMOUNT_SPEED` | 1.5 b/s | crouch only hops you off below this (else it drifts) |

- **Speed build-up:** holding any direction kicks to `BASE` then climbs to `MAX`; releasing
  bleeds speed to 0.
- **Crouch to drift:** holding crouch boosts the drift factor (`+DRIFT_HOLD_BOOST`) so the slide
  onsets at much lower speeds — Mario-Kart hold-to-drift. Crouch only *dismounts* below
  `DISMOUNT_SPEED`, so you can't bail mid-corner.
- **Drift:** the heading turns toward the stick with authority that *falls* with speed, so fast
  turns carry you in a wide arc (the heading lags the input → Mario-Kart slide).
- **Water-running:** `over_water && speed >= WATER_RUN_SPEED` → the bird is pinned to the water
  surface (`Position.y` snapped up, `vel.y = 0`) and skims across. `over_water && speed <
  WATER_RUN_SPEED` → **dismount** (the rider falls into the water; normal swim physics take over).
- Over land it's a normal grounded mount (gravity + a Space jump).

## Engine wiring

- `mob::is_rideable` now includes `Nostrich`; `mob::is_nostrich_mount` distinguishes the arcade
  path. The generic `mob_ai` already skips ridden mobs (`if ridden.is_some() { continue }`), so
  the Nostrich's own wander AI yields to the rider — no extra gating needed.
- `game_loop` steering block branches: Nostrich → `nostrich_ride::step` + water handling;
  Horse/Donkey/Mule → the existing steady gallop (unchanged).
- HUD: `hud_ui::draw_nostrich_speed` — a top-centre wind-up bar that flashes "💧 SKIM!" once
  you're fast enough to hit water.

## Showcase trial

**Roadrunner** (`explorer-roadrunner.json`, survival/flat) — a Nostrich on a flat runway with a
lake to skim. Objective is just `RideEntity` (mount it); the water-skimming is the emergent fun,
guided by the how-to + Satoshi's voice. Listed with the other Trials.

## Tests

`nostrich_ride::tests` — speed builds-up-to-cap, coasting bleeds to zero, runs-on-water-only-when-
fast-else-sinks, drift-stronger-at-speed, velocity-follows-heading×speed. The trial rides the
usual bundled-challenge invariants (parses, names resolve, has Satoshi voice, no money words).

## Deferred / ideas

- Tune-by-feel pass (Axo playtest): speed cap, drift amount, water threshold, the surface-skim
  snap (currently a hard pin; could be a softer spring for a bouncier bob).
- A Nostrich-only "drift gate" slalom or a water-crossing race trial once it feels right.
- Mount only a *tamed* Nostrich? (Today any Nostrich is mountable, matching horses.)

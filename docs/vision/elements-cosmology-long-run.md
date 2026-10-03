# The Six Elements & the Grounded-Frontier Principle — Long-Run Cosmology

**Status:** CANON (design contract — sits above foundation specs)
**Date:** 2026-06-09
**Owner:** Staxolottle (worldbuilding) + Claude (structure)
**Scope note:** Written in AxeNStax `docs/vision/` because AxeNStax is the live product
and drives the canon. **Intended as Decented-platform canon** — promote to a
platform-level doc once a second product consumes it. Glimkin is owned by the same
owner and currently conceptual, so it conforms to *this*, not the reverse.

---

## The principle (the one rule)

> **The unexplained, never the supernatural.**
> Advanced phenomena are allowed if and only if they are framed as *real-but-incompletely-
> understood science* — period-plausible or historically real — and never as magic.

This is Clarke's Third Law **inverted**: not "technology so advanced it looks like
magic," but "phenomena that *look* like magic, always revealed as physics we haven't
finished understanding." It is what lets AxeNStax reach past Newtonian/medieval tech
(electricity, fields, telecommunication) **without** betraying the historical pivot
([[historical-pivot-long-run]]). Anything we add must be grounded in a structure we can
justify.

## Why this isn't a fantasy magic system — it's the actual history of science

The cosmology *is* the real Aristotelian/alchemical model, extended along the real arc
of physics. None of it is invented:

| Element | Historical grounding | In-world era |
|---|---|---|
| **Earth** | Classical element (Empedocles/Aristotle) | medieval — present now |
| **Water** | Classical element | medieval — present now |
| **Air** | Classical element | medieval — present now |
| **Fire** | Classical element | medieval — present now |
| **Electricity** (5th) | Studied seriously from ~1600 (Gilbert) → Volta's pile (1800). A *newly discovered force*. | Enlightenment — **future** |
| **Aether** (6th) | Aristotle's *quintessence*, then the **luminiferous aether** — the genuine 19th-c. physics concept for the unseen medium carrying light/EM waves, refined later by relativity & quantum theory. | frontier — **future** |

So the player isn't learning a spell list — they're walking the real path of natural
philosophy: four classical elements → the discovery of electricity → the aether that
physics itself once believed carried light and signals.

## The wired / wireless split

The two new elements have crisp, non-overlapping jobs:

- **Electricity = wired.** Copper + rubber **cabling** carries power and logic. This is
  the engine's **redstone replacement** — devices, gadgets, automation, logic — grounded
  in real Victorian electrical engineering. (No redstone; cabling instead.)
- **Aether = wireless.** The unseen medium: **telecommunication, signalling, telemetry,
  sensing, fields.** Its first concrete jobs are *information*, not transport — alarms,
  tracking, remote signalling (see the rail spec's Aether security). Radio and telegraphy
  literally rode on what physicists called the aether.

**Teleportation is explicitly excluded.** Moving a body instantly is the hardest thing
to ground even in frontier-physics terms, and it breaks shared-world time coherence. If
true aether-transport ever exists, it is a costly far-endgame with real time/energy/risk
cost — never a free blink.

## Spelling & the Glimkin relationship

- AxeNStax spells the sixth element **"Aether"** (the historical form — Aristotle's
  quintessence, the luminiferous aether). Use this spelling everywhere in AxeNStax.
- Glimkin's **"Ether"** (the veil to another realm) is the **same platform element**,
  expressed differently. **One element, two faces:**
  - **Glimkin** → the **veil**: sacred, observational, *witness-don't-hoard*, gentle.
    Reaching *across* the unseen medium to glimpse another realm.
  - **AxeNStax** → **telecommunication/fields**: utilitarian. Putting the unseen medium
    to *work*.
  - Same underlying idea — *the connective medium we don't fully understand* — wearing
    two cultural faces. Water is sacred in a font and turns a mill; nobody's confused.
- **The rule:** share the element *vocabulary*; do **not** cross-wire the *mechanics or
  tone*. AxeNStax's Aether-tech must never reuse Glimkin's veil mechanic or its sacred
  voice. Keeping the two distinct is a deliberate creative choice (now that the owner
  controls both), not a workaround.

## Player-facing surface

The elements are **named and documented in the wiki** — one article per element, told as
in-world natural philosophy ("Aether: the medium we don't fully understand, that carries
light and signal across the unseen"). The cosmology is felt in mechanics and *explained*
in the wiki, never dumped as a tutorial.

## The tech ladder (what derives from this)

This cosmology is the justifying structure beneath everything past medieval tech:

1. **Now — the four classical elements.** Present in the world's materials and forces
   (stone/ore, water, air, fire/heat). No formal "element system" surfaced yet.
2. **Electricity (wired).** Copper/rubber cabling → power, devices, logic, automation.
   The redstone replacement. *(Future foundation spec — not yet written.)*
3. **Aether (wireless).** Telecom/signalling/telemetry → alarms, tracking, remote
   signalling, eventually wireless logic. *(Future foundation spec — not yet written.)*

Concrete first consumers already in design:
- **Rail Freight & Logistics** ([../foundations/2026-06-09-rail-freight-logistics.md])
  ladders across all three: muscle/gravity carts now → **electric powered rail** when the
  electricity tier lands → **Aether freight security** (alarm/track/flag) when the aether
  tier lands. Rail is the showcase of the whole progression.

## Electricity tier — component glossary + first mechanics

**Status:** names + first mechanics **LOCKED 2026-06-16** (owner: Staxolottle). The
foundation spec is still to be written; its **hard prerequisite** is a
**neighbour-update / scheduled-tick architecture** — the engine today has no
"block A changes → notify block B" mechanism (see
`../research/2026-06-16-engine-content-and-electricity-audit.md`).

**Naming convention — Hybrid:** plain, kid-clear primary name at the hotbar; the
period-accurate term carried in the wiki article ("felt in mechanics, explained in
the wiki"). The element stays **Electricity** (wired); **Aether** is the later
wireless tier. **Power is *generated and stored*, never mined** — there is no
redstone-style magic dust; that grounded "generate-and-store" model is the
differentiator.

### Components

| In-game name | Wiki / period term | Role |
|---|---|---|
| **Copper Wire** | copper conductor | crafted intermediate (copper ingot → wire) |
| **Cable** | insulated cable | copper wire + rubber; the placeable power conductor (`CopperCable` seed already exists) |
| **Hand Crank** | hand dynamo | manual bootstrap generation |
| **Steam Generator** | dynamo | burn fuel → boiler → power; first real source (reuses the furnace fuel pattern) |
| **Water Wheel** | water dynamo | flowing water → power ✅ *(shipped 2026-09-06 — turns beside or above a current; a still pond drives nothing)* |
| **Windmill** | wind dynamo | wind → power *(needs a wind mechanic; later, Air-element)* |
| **Battery** | accumulator | stores charge; buffers intermittent sources |
| **Electric Lamp** | filament / arc lamp | light when powered |
| **Powered Rail** | electrified rail | the rail-spec Phase 2 consumer |
| **Electric Motor** | motor | rotational output for machines (future) |
| **Electric Furnace** | electric furnace | fuel-free faster smelting (later) |
| **Lever** | knife switch | manual on/off toggle (also a generic building block) |
| **Button** | push switch | momentary pulse |
| **Pressure Plate** | treadle | triggers when stepped on |
| **Logic Gate** | relay | AND / OR / NOT logic from wired inputs |
| **Beam Sensor** | photoelectric sensor (retroreflective / through-beam) | emits **and** detects an infrared beam — see mechanics below |
| **Mirror** | reflector | rotatable beam router; also decorative |
| **Motion Sensor** | PIR — passive infrared | proximity / area trigger, no beam |

### Beam Sensor — first mechanics (locked)

- One **Beam Sensor** both **emits and detects** a faint, slightly-visible infrared
  beam from the **centre** of the block (lore: "you catch the glow"; a fully-hidden
  beam may be a later upgrade).
- Two arming modes (the real modes of a photoelectric sensor):
  - **Retroreflective** *(primary)* — one Beam Sensor + a **Mirror** terminal reflects
    the beam back to it.
  - **Through-beam** — a Beam Sensor at **each end**; the beam runs between them.
- **Mirrors are rotatable**: reflect the beam **straight back (180°)** or turn it
  **90°**, so beams route into paths. A beam that dead-ends (no mirror/sensor
  terminal) does **not** arm.
- The beam is **non-destructive** — it passes harmlessly (parkour-safe). An entity
  crossing any segment **breaks** the beam → **trigger**, feeding a Cable / Logic Gate
  like any wired input.
- **Use cases:** security triplines, puzzle routing, and **parkour** (timing
  obstacles, or "cross the beam to open the door" switches) — strong synergy with the
  blank-canvas / parkour direction.
- **Motion Sensor (PIR)** is the beamless sibling: proximity/area trigger only.

### Build order (when the tier is specced)
1. **Neighbour-update / scheduled-tick architecture** — the enabler; nothing works without it.
2. **Cable** + a power-level field on blocks + inputs (**Lever / Button / Pressure Plate**) + **Logic Gate**.
3. **Generation + storage** — **Steam Generator + Battery** (most self-contained), **Hand Crank** bootstrap.
4. **Sensors** — **Beam Sensor + Mirror**, **Motion Sensor**.
5. **Consumers** — **Electric Lamp**, **Powered Rail**, …
6. ✅ **Water Wheel** — shipped 2026-09-06 once directional water flow existed.
7. **Later** — **Windmill** (needs wind), then the **Aether** (wireless) tier.

## Related

[[project_economies_vision]] · [[project_axenstax_has_farming]] ·
[[historical-pivot-long-run]] · the Glimkin mod
(`../foundations/2026-05-19-glimkin-mod.md`).

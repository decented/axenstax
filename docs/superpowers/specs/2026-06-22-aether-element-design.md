# Aether — the connective (wireless) element — design

**Date:** 2026-06-22
**Status:** DESIGN ONLY (not built; do not build yet). Captures a design conversation so nothing is
lost. The **six-elements cosmology canon** is the parent; this spec proposes folding the definitions
below into it (a fold-in is **pending owner go** — see §12).
**Owner ask:** make Aether cohere now that it does several things (wireless signalling, travel,
mind/sense), without it becoming "the element that does any spooky thing," and without breaking
"unexplained, not supernatural" or the rail-over-waystones stance.

**Relationships:**
- Parent canon: the six-elements doc (4 classical + **Electricity = wired** + **Aether = wireless**).
- Consumer: `2026-06-22-multi-world-hub-and-portals-design.md` (portals/travel reference *this* spec).
- Neighbours: the farming/biome vision (foraging), rail logistics (rail-replaces-waystones).

---

## 1. Core axiom

> **Electricity = connection *by* a wire. Aether = connection *without* a wire.**

That single axis is the whole element. "Connection" just carries different **payloads**:
- **signal** — information without a wire → wireless sensors / transmitters
- **presence** — matter/you without a path → travel (portals)
- **mind / sense** — perception without the body moving → astral, creature-sense

This keeps Aether one idea, not a grab-bag. It stays **"unexplained, not supernatural"** the way wifi
is to a child: clearly a real force with rules, not a spell. The defence against "it's just magic" is
always: it is **built, powered, paired, and ruled**, and it is grounded in real phenomena (radio
waves; mycelial signalling; animal magnetoreception).

---

## 2. The two faces (a clean 2×2)

Every Aether mechanic has an **engineered** face and a **living/wild** face, and **they
interoperate** (see §4). This mirrors the engineered-vs-wild duality used for travel (gates vs fungi).

| | **engineered** | **living / wild** |
|---|---|---|
| **Electricity (wired)** | circuits / redstone | bioelectricity (electric eels — optional niche) |
| **Aether (wireless)** | radio / wireless sensors / transmitters | cultivated mycelial networks; animals as free sensors; astral |

**Decision (owner, 2026-06-22): keep BOTH faces of Aether.** Do not drop the technological wireless
in favour of an all-organic Aether; do not drop the organic in favour of pure tech. They coexist and
combine.

---

## 3. Aether as a resource

Aether is a **chargeable resource** — think battery/fuel, **not** mana:
- **Gathered** from Aether-dense sources (nodes / crystals / Aether-fungi — exact sources TBD, §12).
- **Stored** in cores / cells.
- **Spent** to power wireless devices, open gates, etc.
- **Recharged** from a source → a real **gather → charge → use** gameplay loop. This is what turns a
  docs-only element into a mechanic.

---

## 4. Attunement — the universal pairing primitive

Everything Aether is **tuned to a channel/partner** ("attunement"), because wireless needs a
frequency:
- a **gate** is attuned to its partner gate;
- a **sensor** is attuned to its **receiver**;
- a befriended **companion** is attuned to you.

One concept reused everywhere — and it maps **1:1 onto the engineering** (the cross-world *handoff
token* in the portal spec *is* attunement). **Interoperability falls out of this:** because organic
and technical parts share the one attunement protocol, they **mix freely** — a living fungus sensor
can trip an engineered Aether lamp; an engineered button can set off an organic reaction. Builders
pick an aesthetic (clean tech / living garden / a blend); the system doesn't care.

---

## 5. Face A — Engineered Aether (wireless tech)

The wireless cousin of redstone wires: **wireless sensors, signal links, transmitters/receivers.**
Build a device, **power** it (Aether charge), **attune** it to its partner. Examples: a wireless
button → distant lamp; a remote tripwire → a switch across the map with no wire run. Most
real-world-grounded face (radio is real and invisible).

---

## 6. Face B — Living Aether (organic) + the humane rule

The biological face, grounded in real science so it stays non-supernatural:
- **Backbone = cultivated fungi / mycelial networks.** Mycelium genuinely passes signals between
  organisms in nature (the "wood wide web"). You **cultivate** a network (it's farming) that senses
  disturbance and transmits to a paired fruiting body. No creature confined.
- **Animals are free companions, never components.** A bird that alarm-calls at an intruder is using
  its *natural* gift; you've befriended it (it comes and goes) and you *read* its behaviour. You
  never *need* to trap one — the fungal network does the core sensing; the animal is a bonus,
  willingly given. Ties into the tameable-companion-as-friend framework.
- **"Telepathy" is reframed as Aether-attunement**, not mind-reading — two living things tuned to the
  same channel (animal senses / mycelial signalling), which keeps it on the non-supernatural side.

> **Humane rule (owner, 2026-06-22): the living path *partners with* nature, it never cages it.**
> *"Don't cage a bird and feed it mushrooms to make a sensor."* Receivers are reacting fungi/plants
> or an engineered relay — **never an animal pinned to a switch.**

Worked example (humane "bird + mushroom" sensor): a **cultivated mushroom patch senses** an
approach, optionally **a free bird's alarm** you've learned to read, triggering **a reacting fungus
or an Aether relay** — humane end to end, and any part swappable for its technical equivalent.

---

## 7. Travel via Aether (the between-worlds role)

Full navigation/graph design lives in the multi-world/portals spec; **the element rules are here.**

- **Principle (keeps rail-over-waystones intact):** **physical travel *within* a world**
  (walk/rail/cart); **Aether bridges what physical space cannot connect** (between separate worlds, +
  pocket interiors). **Aether never shortcuts a route you could walk.** Waystones were removed for
  skipping walkable distance *inside* a world; portals don't, because separate worlds share no
  physical space — there is nothing to skip.
- **Engineered travel = Aether gates.** Cost model:
  - **attunement** (one-time, expensive — establishing the link),
  - **per-trip** (each crossing burns core charge),
  - **upkeep** (optional trickle to keep a gate open/listening),
  - **scope scaling** (pocket interior cheap → cross-Place expensive).
  - Recharge from an Aether source. **Failure is SAFE:** out of charge ⇒ the gate won't open —
    inconvenient, never harmful.
- **Wild travel = astral, via Aether-fungi** (see §8). **Astral = spirit travels, body stays** (and
  is *vulnerable*): scouting/reach, not logistics — so it does **not** undercut rail. Gateless,
  personal, **risky**.
- **The loop tying wild → engineered:** **forage (risky) → astral-scout a new place → attune it →
  build + power a gate.** Mushroom = the explorer's tool; gate = the settler's tool.
- **Layer split:** the meta **menu/launcher** (picking a world) needs no lore; **in-world gates**
  (lobby gates, doors, the TARDIS pocket-interior pattern) are diegetic Aether tech.

---

## 8. Foraging & the mushroom lookalike danger (educational subsystem)

Aether-fungi are the wild Aether source and the astral consumable — and the vehicle for a genuine
real-world lesson (deadly lookalikes kill foragers every year).

- **Where:** Aether-fungi grow in Aether-dense biomes (the "good" one rarer / one-biome; toxic
  lookalikes common + widespread).
- **Lookalike pairs**, told apart by **learnable cues**: cap **spot pattern/markings**,
  **biome/habitat**, **bruising/colour-change on picking**, maybe a stem/ring tell. They *mimic* each
  other, like reality.
- **Graduated danger** (so mistakes teach, not merely punish):
  - **Hallucinate** — see things that aren't there (phantom mobs/blocks, vision distortion). The
    standout mechanic; thematically tied to altered perception/astral. Calibrate **whimsical-spooky,
    not horror.**
  - **Really ill** — recoverable debuff with a cost.
  - **Deadly** — the death-cap analogue: you die, drop your stuff.
- A **field guide / mentor** teaches identification — doubling as the meta-lesson (*learn to ID, use a
  guide, ask an expert*).

**Child-safety framing — HOLD THIS LINE (design rule, not a blocker):** a mechanic that rewards
*eating a found mushroom with powers* can send a young child the opposite of the safety message.
Keep the lesson pointing right:
- **caution dominant** (deadly lookalikes common + clearly signposted-risky);
- **preparation by knowledge, not raw eating** (brew/prepare the *identified* fungus — models "never
  eat what you find");
- **hallucination whimsical + age-calibrated** (an *Aether trance*, not a drug trip; respect
  parental/age settings);
- optional **real-world safety beat** ("real mushrooms can be deadly — never eat wild ones"), in the
  same educational spirit as proof-of-play.

(Content-safety scanning of *images* is a separate concern, parked — see the portal spec's assets
section.)

---

## 9. Gameplay payoff

Aether stops being docs-only lore and becomes a system: **gather → charge → attune → build.** It
gives players a **wireless toolkit** (engineered) *and* a **living/organic toolkit** (cultivated),
plus travel and scouting — and it grounds the most sci-fi things in the game (portals, astral) in the
project's own cosmology rather than imported magic.

---

## 10. Invariants (the rules that keep it coherent)

1. **Aether never shortcuts within-world walkable travel** — protects rail; travel role is
   between-worlds (+ pocket interiors) only. (A brutally-costed within-world endgame jump is possible
   but discouraged — recommend *not*.)
2. **Unexplained, not supernatural** — every Aether thing is built/powered/paired/ruled and grounded
   in a real phenomenon (radio, mycelium, animal senses).
3. **Humane** — the living face partners with nature; no caging/forcing/exploiting animals.
4. **"Telepathy" = attunement**, not mind-reading.
5. **Engineered failure is safe; wild failure is risky** — the deliberate contrast (a dead gate just
   won't open; a wrong mushroom can kill you).
6. **Keep both faces** (engineered + living) and **let them interoperate** via one attunement protocol.

---

## 11. Phasing (when/if built — for reference, not a commitment)

- **Aether resource + engineered wireless** (sensors/links) — a self-contained first slice.
- **Aether gates + cost model** — needs the portal/handoff work in the multi-world spec (its Phase 2).
- **Living Aether** (mycelial networks, companion sensors) — ties to farming/biome + tameable mobs.
- **Foraging + astral** — likely its own build, gated behind the child-safety framing.
Sequencing is owner's call; this spec just keeps the pieces consistent.

---

## 12. Open questions / decisions

1. **Aether source(s):** nodes / crystals / fungi / something else? What does "gathering Aether" look
   like?
2. **Astral reach:** can you astral-scout *anywhere*, or only places you've already attuned / a bounded
   radius? (Affects how it pairs with gate-building.)
3. **Wireless-tech toolkit breadth:** how big is the engineered wireless kit (just sensors+links, or a
   fuller wireless-redstone suite)?
4. **Within-world Aether jump:** allow a costed endgame exception, or hard-forbid to protect rail?
   (Recommend hard-forbid.)
5. **Hallucination intensity & age settings:** how strong, and how does it respect parental/age
   controls?
6. **Canon fold-in:** approve folding §1–§2 (the axiom + 2×2 + keep-both) into the six-elements
   cosmology doc so canon and this spec agree. **(Pending owner go.)**

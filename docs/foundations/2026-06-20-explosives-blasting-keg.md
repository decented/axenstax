# Explosives — Black Powder & the Blasting Keg

**Date:** 2026-06-20
**Status:** **DELIVERED 2026-06-20** (solo, to the Axolittle playtest boundary).
Built on branch `worktree-spec-49-explosives` across 11 phases, `check.sh` green
(native + WASM) at every commit; **3156 tests**. Protocol **54 → 55**. Remaining
= the **Axolittle "boom" feel playtest** (test sheet
`docs/test-sheets/2026-06-20-explosives.md`) + a native rebuild/deploy (owner —
metered). **What shipped vs. spec** (deviations, all small):
- **Compost is a real `MaterialId`** (158) — the Composter's intermediate output;
  the spec named "Compost" as the farming output and it earned its own id.
- **Chain-ignition** landed with the **blast** (P8 `explosion::resolve_blast`),
  not the bare fuse phase — "caught inside another blast" needs the radius to mean
  something.
- **Obsidian** is immune (wired 2026-09-27 — the block exists now; before that fix
  it fell through to the soft default and a keg levelled obsidian vaults).
- **Block-entities in a blast (2026-09-27 audit fix).** Economy blocks — Vendor,
  Tip Jar, Auction, Plot Marker, Market Bell — are **blast-immune**: the block and
  its stock/escrow/claim stay put, so a keg lit beside someone's shop or plot
  can't wipe it. Containers (chest, grave, furnace, dispenser/dropper, campfire,
  drying rack, composter, item frame, latent print, face attachments) stay
  blastable but **spill their contents as item entities**
  (`explosion::spill_block_contents`) — the "drop nothing" rule covers the
  blasted block itself, never what it stored.
- **Fluids in a blast (2026-09-27 audit fix).** Water and lava stay blastable,
  but every blasted cell goes through `fluids::notify_block_edit(old, AIR)` —
  the same path as a pickaxe or bucket removal — so blasted sources are dropped
  (no phantom source that makes a re-poured bucket inert) and the crater's
  neighbours wake and flow back in.
- **Blast particles** are deferred — the engine has **no particle framework** yet,
  so detonation ships with the **boom sound** (`audio::play_explosion`) + the
  crater; a richer particle pass is a follow-on when the framework lands.

Owner-approved direction (2026-06-20, "go with your recommendations"): **~4-block
/ ~4 s** blast feel (tunable consts in `explosion.rs` / `power.rs`), blasted blocks
**drop nothing**, saltpetre farmed from **plant/food waste** (manure deferred).
Mine **and** farm the saltpetre in v1; the industrial (ANFO) route is a later
tier. PWA/WASM + native.
**Driver:** Staxolottle (`/remote-control`, 2026-06-20). Came out of the "where
are we at with TNT?" question. Axolittle's note was the seed: in Minecraft TNT
traces back to **creeper** gunpowder — and **we removed creepers** in the
fantasy-roster excision (2026-05-24), taking gunpowder and the whole explosion
code path with them. So explosives need their own honest, mineable supply chain
that doesn't lean on a Minecraft-distinctive mob drop. Text-only design.

---

## TL;DR

Reopen explosives the **real-chemistry** way the rest of the project teaches:

1. **Black Powder** ← `Sulphur + Charcoal + Saltpetre` (the real ~75/15/10
   gunpowder formula, the three ingredients laid out as a recipe that *is* the
   chemistry lesson).
2. **Blasting Keg** ← `Black Powder + a wooden container` — **our own
   identity (a barrel), never the red "TNT" letter-cube** (Minecraft trade
   dress; we just did an IP-cleanup pass). Light the fuse with the existing
   **Magnesium Firestarter**, get clear, *boom* — **or wire it to Electricity
   (Spec 48):** a **Plunger Detonator**, cables fanned to one keg or many, and
   any Spec 48 sensor (beam tripwire, PIR/motion, pressure plate, logic-gated
   arming) fires the same fuse. See §"Electricity integration".
3. Two of the three ingredients already exist (**Sulphur** as an orphan
   material with no source yet; **Charcoal/Coal** mineable today). This spec
   **finally gives Sulphur a home** (a Brimstone ore) and adds the missing
   **Saltpetre**, sourced **two ways**: mine a **Nitre** deposit *or* farm it in
   a **Composter** (the nitre-bed the farming docs keep deferring — it earns its
   slot here).
4. The **industrial route (ANFO = ammonium-nitrate fertiliser + oil)** is a
   flagged later tier — it ties explosives into the farming/fertiliser and
   oil-as-lubricant chains we already have.

The one load-bearing design rule: **blasting is demolition, not mining.**
Blasted blocks drop **nothing** and run **no Proof-of-Play hash**, so explosives
can never shortcut the pickaxe-strike reward loop. See §"Proof-of-Play
integrity".

---

## Why this earns a slot (and what it teaches)

- **Closes a real gap honestly.** Players expect explosives; we can't borrow the
  creeper path; the real-world supply chain is *better* — it's a teachable
  chemistry ladder (saltpetre + charcoal + sulphur = black powder), exactly the
  register the project already uses (Epsom-salt fertiliser, magnesium
  flame-tests, brimstone = "burning stone").
- **Finally sources Sulphur.** `MaterialId::Sulphur` has sat in the registry
  since the materials-expansion with *no acquisition path* — flagged "future
  gunpowder upgrade — recipe TBD". This is that future. (`item.rs:106`; no
  `SULPHUR_ORE` exists in `block.rs`/`biome.rs` today.)
- **Gives the Composter a reason to exist now.** The farming vision parks a
  Composter at T7 (`farming-economy-long-run.md` §10.4). The "farm saltpetre"
  path pulls a minimal version forward as the nitre-bed, tying explosives to the
  farming economy the owner asked for.
- **A clean echo of Proof-of-Play.** You *can* blast a tunnel — but you forfeit
  the reward. The careful pickaxe strike (the HMAC work) is still the only thing
  that pays. The mechanic itself teaches "there's no shortcut around the work."

---

## The supply chain

### Ingredient 1 — Sulphur (finally mineable)

- **Brimstone** — a new ore block. "Brimstone" literally means *burning stone*
  = sulphur; evocative and teachable. Mines with a **stone pickaxe or better**,
  drops 1–2 **Sulphur** (the existing material — no new `MaterialId`).
- **Where:** deep + **volcanic** — a low band near lava / the deepslate layer,
  with a secondary weighting alongside the existing **Rock Salt** / **Magnesium**
  evaporite veins in Desert + Mountains (real sulphur occurs both volcanically
  and in sedimentary salt/gypsum settings). Exact band + rarity tuned at build
  (start ~ iron-band rarity). Reuses the `biome.rs` ore-placement path.

### Ingredient 2 — Charcoal (already in the game)

- **Charcoal** (furnace: wood → charcoal) or **Coal** (`MaterialId::Coal`,
  mineable today). Either satisfies the carbon leg of the recipe. No new
  content; the recipe accepts either (matcher fuzzy-category, like other
  either/or inputs).

### Ingredient 3 — Saltpetre (the missing piece — mine **and** farm)

UK spelling: the refined material is **Saltpetre**; the mineral deposit is
**Nitre**.

- **Mine it — `NITRE_ORE`.** A new ore (real niter is a cave-crust / arid-soil
  evaporite — honest). Mines with **stone pickaxe+**, drops 1–2 **Saltpetre**.
  Worldgen weighting alongside salt/brimstone (Desert + cave walls). Keeps
  explosives inside the mining loop for players who don't farm.
- **Farm it — the Composter (nitre bed).** A new **Composter** workstation
  block. Feed it plant trimmings / food waste (and, later, manure for flavour);
  over time it yields **Compost** (the farming output) and, aged further,
  **Saltpetre** (real nitre beds were aged compost heaps). Minimal v1: a
  block-entity with an input buffer + a timed conversion, mirroring the furnace
  fuel-burn state machine. This is the "earn it through farming" path the owner
  asked for, and it doubles as the long-deferred farming Composter.

> The two saltpetre paths are deliberately parallel, not gated — a miner and a
> farmer both reach the Blasting Keg by their own route, which is the owner's
> explicit call (mine **and** farm in v1).

---

## The craft ladder

| # | Pattern | Inputs | Output |
|---|---------|--------|--------|
| 1 | shapeless 3 | 1 × `Sulphur` + 1 × `Charcoal`/`Coal` + 1 × `Saltpetre` | 1 × `BlackPowder` (×N tuned) |
| 2 | keg | 1 × `BlackPowder` + 1 × wooden container (Planks ring / Barrel) | 1 × `BLASTING_KEG` block |
| 3 | (optional) bigger charge | 4 × `BlackPowder` + container | 1 × stronger keg variant *(deferred — see Out of scope)* |

`BlackPowder` is a new `MaterialId` — also the **propellant** the Magnesium spec
flagged as missing, so it **unblocks launching fireworks** as a follow-on (that
spec explicitly deferred them "blocked on a propellant"). Black Powder is **not**
the old `Gunpowder` variant (excised) — new name, new identity, no Minecraft mob
lineage.

---

## The Blasting Keg — behaviour

- **Placement.** A barrel-look block (`BLASTING_KEG`). Inert until lit. **Never**
  the red TNT cube.
- **Ignition (v1).** Right-click with the **Magnesium Firestarter** (exists —
  `magnesium-mineral` spec) → lights a **fuse**. A fuse timer runs on a
  block-entity (~**4 s / 80 ticks**, tunable) so the player can get clear, then
  it detonates in place. A keg caught inside another blast **chain-ignites**
  (fun + teaches sympathetic detonation).
- **Electrical ignition.** The keg is also a first-class **Electricity sink** —
  a rising-edge power signal lights the same fuse. Full wiring design (Plunger
  Detonator, cables, beam tripwires, PIR/motion, logic-gate arming) is its own
  section below: §"Electricity integration".
- **Blast.** On detonation: a spherical radius (~**4 blocks**, tunable const),
  destroys blocks whose **blast resistance** is below the keg's power; damages
  entities + players with **distance falloff + line-of-sight blast reduction**
  (Spec 05 §6.3 already declares `Explosion` damage "Yes — with blast
  reduction"). Smoke + debris particles + a boom sound — both already declared
  in Spec 05 §11 (`Explosion` particle + sound hooks exist, unimplemented).
- **Blast resistance.** A per-block `blast_resistance` lookup (pure helper /
  table, not necessarily a new `BlockDef` field — mirror how `camera_occlusion`
  was *derived* not stored). **Bedrock, Obsidian, and Satori are immune.**
  Water/lava behave sensibly (water dampens — defer the full fluid interaction
  if it's not cheap).
- **No fire spread (v1).** Kid-friendly + anti-grief: detonation does **not**
  start spreading fire. Reconsider as an option later.

---

## Electricity integration — plunger, cables, sensors (Spec 48)

Explosives are the first Electricity **actuator with real consequence**: the
powered rail was Spec 48's first *consumer*, the Blasting Keg is the first
*output that does something dramatic*. Everything here rides on primitives Spec
48 already shipped (Phases 1 + 2, delivered 2026-06-17) — so this is **wiring,
not new infrastructure**.

**The keg is a power sink.** It's a `BlockEntityData::PowerDevice` sink on the
`power.rs` network. On a **rising edge** (incoming power unpowered → powered) it
lights the same ~4 s fuse the Firestarter uses. **Edge-triggered, not level** —
a held-on wire fires it once, not every tick; a single pulse is enough. A keg
that detonates still **chain-ignites** neighbouring kegs (sympathetic
detonation), wired or not.

**Run cables to it.** Spec 48 **Cable** carries the signal from any source to
one keg or — fanned out — to **many kegs at once** for synchronised demolition.
It's just the existing power flood; the keg is an ordinary sink on it. No wire
distance-decay (Spec 48's deliberate differentiator), so a long run out to a
remote charge Just Works.

**The Plunger Detonator** (new input block) — the classic T-handle detonator
box, and the themed centrepiece. Interact with it → it emits a momentary power
**pulse** onto the connected cable network. Mechanically it's the Spec 48
**Button** momentary-source path in a dramatic box (reuses shipped code);
thematically it earns its own block, look, and sound (the *cha-CHUNK*). Recipe:
`Iron Ingot + Cable + Planks` (a boxed switch). Push plunger → pulse → cable →
keg fuse → *boom*.

**Every Spec 48 trigger already works** — the keg is a standard sink, so there's
no per-sensor wiring code to write:

| Trigger (already shipped in Spec 48) | Use |
|---|---|
| **Plunger** (new) / **Lever** / **Button** | manual hand-detonation (plunger is the hero) |
| **Pressure Plate** | step-on / weight trap |
| **Beam Sensor + Mirror** (Phase 2) | photoelectric **tripwire** — break the beam, boom |
| **Motion Sensor / PIR** (Phase 2) | **proximity** trigger — something moves nearby, boom |
| **Logic Gate** (AND/OR/NOT/XOR) | compose conditions — see arming below |

**Arming is a circuit, not new state (recommended pattern).** To stop a stray
signal levelling your base, wire the trigger through a **Logic AND gate**:
`AND(arm-lever, trigger) → keg`. The charge only fires when you've thrown the arm
lever **and** the trigger fires — real circuit design from parts that already
exist, no bespoke keg safety flag. (A per-keg "manual-only / ignores
electricity" meta-bit is a possible later toggle for purely decorative kegs.)

**Gating still applies.** `explosives_enabled = false` no-ops an electrical
detonation exactly as it no-ops a hand-lit one, and Adventure still blocks the
block destruction. Electricity is a **trigger, not a bypass** — the
drop-nothing / no-Proof-of-Play rule below holds however the keg is fired.

---

## Proof-of-Play integrity (the load-bearing rule)

Explosives must not become an X-ray-free shortcut around the reward economy.

- **Blasted blocks drop nothing** by default — it's demolition, not harvesting.
  No item drops, and crucially **no Proof-of-Play HMAC strike** is generated for
  a blasted block. The pickaxe strike (the actual work) stays the *only* path to
  ore drops and, on Bitcoin-enabled servers, to sats. This closes the "blast a
  vein, collect ore with zero strikes" exploit and keeps the
  `server_secret`-anchored reward layer intact (Proof-of-Play clarification doc;
  `reference_proof_of_play_is_proof_of_work`).
- A cosmetic **rubble** drop for common terrain (dirt/stone) is a *possible*
  later tweak — but **ore never drops from a blast**, ever. Flagged, not v1.

---

## Operator + PlayMode gating

- **Per-server / per-region toggle.** `explosives_enabled` (world-meta /
  `ServerEconomyConfig`-adjacent). This is exactly the per-region TNT on/off the
  land-claim backlog item (#25, WorldGuard-class) wants — design the flag here so
  the region system inherits it. Default: **on** in single-player + Creative;
  operator-toggleable on multiplayer survival.
- **PlayMode respect.** **Adventure** = read-only world, so a keg cannot break
  blocks (inherits the existing Adventure block-edit gate — guard-test it).
  **Spectator** = can't place. **Creative** = place + detonate freely, no
  ingredient cost.

---

## The industrial route (deferred — later tier)

The real "what miners actually use today" path, kept as a flagged follow-on per
the owner's "industrial is the later route" call:

- **ANFO** = **Ammonium Nitrate** (a nitrogen *fertiliser*) + **Oil** (the
  `oil-as-lubricant` material). Bigger, cheaper blast at the cost of an advanced
  production chain — ties explosives into the farming/fertiliser economy
  (`project_axenstax_has_farming`) and the pressed-oil line
  (`project_oil_lubricant_mechanic`). The same real-world nitrogen chemistry
  feeds both fertiliser *and* explosives — a strong teachable hook for the
  higher tier. Its own spec post-Blasting-Keg playtest.

---

## Data model (build notes)

- **Blocks (append from next free `BlockId` at build — coordinate the texture
  index range with concurrent specs):** `BRIMSTONE` (sulphur ore),
  `NITRE_ORE`, `COMPOSTER`, `BLASTING_KEG`, `PLUNGER_DETONATOR`. Worldgen for
  Brimstone + Nitre via the `biome.rs` ore path. Each gets a procedural texture
  in `texture_gen.rs` (brimstone = yellow-green crystalline on dark rock; nitre
  = pale white-grey crust; composter = slatted wood bin; blasting keg = banded
  barrel with a visible fuse, **not** red, **no lettering**; plunger detonator =
  boxed switch with a T-handle, `directional` via Spec 48's `block_meta` so the
  handle faces the player).
- **Electricity wiring (Spec 48, no new infra):** `BLASTING_KEG` registers as a
  `PowerDevice` **sink** (rising-edge → fuse); `PLUNGER_DETONATOR` registers as a
  momentary **source** (reuses the Button pulse path). Both hook the existing
  `power.rs` flood + `UpdateScheduler`; no changes to Cable / sensors / gates.
- **Items (`MaterialId`, append-only, positional bincode):** `Saltpetre`,
  `BlackPowder`. **`Sulphur` already exists** — just wire its ore drop + the
  Black Powder recipe arm. Extend the `TryFrom<u16>` map + name/colour/
  texture/complexity arms + `ALL_MATERIAL_IDS` + `/give` aliases.
- **Block-entities (`BlockEntityData`):** a keg **fuse** state (countdown, like
  the Cyanotype `LatentPrint` per-tick driver) and a **Composter** state (input
  buffer + timed conversion, like the furnace fuel-burn machine). Reuse Spec
  48's `block_meta` byte for keg lit/unlit where it fits.
- **Save (`WorldSave`, append-only — new fields LAST, after the current final
  field):** primed-keg fuses + composter contents persist (mirror
  `SavedTipJar` / `SavedLatentPrint`). Block placement is canonical; derived
  indices `#[serde(skip)]` + rebuilt on load.
- **Protocol:** one `PROTOCOL_VERSION` bump (new BlockIds + MaterialIds +
  block-entity variant; `BlockChange` already carries `meta` post-Spec-48).
  Confirm the current version at build (it has moved well past the salt-era 29).
- **Recipes (`crafting.rs` + `crafting_catalogue.rs`):** add the Black Powder +
  Blasting Keg arms to `match_recipe`, **and a `RecipeCard` each** so the recipe
  book shows them (Spec 43 consistency test `every_card_matches_the_live_matcher`
  must stay green).

---

## Phases (indicative — finalise at build)

1. Branch off main + spec read.
2. New BlockIds + procedural textures + BlockDef entries (`block.rs`,
   `texture_gen.rs`). PROTOCOL bump here.
3. `Saltpetre` + `BlackPowder` MaterialIds + sprites + name/colour tables; wire
   **Sulphur** ore drop (`item.rs`, `block.rs`).
4. Worldgen — Brimstone + Nitre vein placement (`biome.rs`).
5. Recipes — Black Powder + Blasting Keg, matcher arms + RecipeCards
   (`crafting.rs`, `crafting_catalogue.rs`).
6. Composter block-entity — input + timed Compost/Saltpetre conversion (new
   module, save round-trip).
7. Blasting Keg — fuse block-entity, Magnesium-Firestarter ignition,
   chain-ignite (`block_interact.rs`, scheduler hook).
7b. **Electricity integration (Spec 48)** — keg as `PowerDevice` **sink**
   (rising-edge → fuse) + **Plunger Detonator** block (momentary source, Button
   pulse path) + recipe; verify cable fan-out to multiple kegs and every shipped
   trigger (Plunger / Lever / Button / Pressure Plate / Beam Sensor / Motion
   Sensor-PIR / Logic Gate). Document the `AND(arm, trigger)` arming pattern.
8. **Blast resolution** — radius, blast-resistance table, entity/player damage
   with falloff + LoS reduction, **drop-nothing + no-Proof-of-Play guard**,
   particles + sound (new `explosion.rs` pure helpers, `combat.rs` damage path).
9. Operator/PlayMode gating — `explosives_enabled` + Adventure/Spectator/Creative
   behaviour, all guard-tested.
10. Save/load fields + round-trip integration test.
11. Test sweep — pure helpers + integration (`test_integration/explosives.rs`).
12. **Axolittle playtest gate** (feel: fuse length, blast size, "boom").

---

## Verification & boundary

- **Unit-testable:** Brimstone→Sulphur + Nitre→Saltpetre drops; Black Powder +
  Blasting Keg recipes match (incl. the RecipeCard consistency test); Composter
  yields Saltpetre after N ticks; fuse counts down + detonates; blast destroys
  ≤blast-resistance blocks and **skips** bedrock/obsidian/Satori; **blasted
  blocks drop nothing AND emit no Proof-of-Play strike** (the key guard test);
  Adventure mode blocks keg destruction; `explosives_enabled=false` no-ops a
  detonation (hand-lit **and** electrical); a **rising-edge** power signal lights
  the fuse **exactly once** (edge not level — a held-on wire doesn't re-fire); a
  Plunger pulse detonates a cable-wired keg; one pulse **fans out** to multiple
  kegs; save/load round-trip with a primed keg + a loaded composter.
- **Playtest boundary (Axolittle):** fuse length feel, blast radius feel, the
  audio/particle "boom", how findable Brimstone/Nitre are, whether the Composter
  path feels worth it vs mining.
- `check.sh` ALL GREEN at every phase commit; engine + WASM build.

---

## Memory-rule check

- ✓ `reference_proof_of_play_is_proof_of_work` — blasting yields **no** HMAC
  strike + **no** ore drop; the pickaxe-strike work stays the sole reward path.
  Anti-X-ray reward-layer defence is preserved.
- ✓ `project_bitcoin_parent_controlled` — no new sats path; on Bitcoin servers
  explosives never pay (they can't, by the drop-nothing rule). Operator toggle
  sits with the existing per-server policy.
- ✓ `feedback_uk_english_naming` — Saltpetre, Nitre, Sulphur, Brimstone,
  Composter, Black Powder, Blasting Keg. No Mojang coinage; the red-"TNT"-cube
  trade dress is explicitly rejected.
- ✓ `project_axenstax_has_farming` — the farm-saltpetre path pulls the deferred
  Composter forward; the industrial (ANFO) tier ties to fertiliser.
- ✓ `project_oil_lubricant_mechanic` — oil gets a second industrial-tier use
  (ANFO) on the deferred route.
- ✓ `project_shared_infra_strategy` — blast resolution + blast-resistance table +
  the demolition-vs-harvest reward rule are engine-generic; lift cross-game.
- ✓ `feedback_merge_to_main_preauthorised` — healthy-gate merge when green
  (once built; nothing built yet).
- Sibling to **Spec 37 Magnesium** (Black Powder unblocks its deferred launching
  fireworks; Firestarter is the keg's igniter) and **Spec 48 Electricity** (the
  keg is an Electricity **sink** + adds the **Plunger Detonator** input; every
  shipped Spec 48 trigger — beam / PIR / pressure plate / lever / button / logic
  gate — fires it; reuses the scheduler + `block_meta` + Button-pulse path, no
  new power infra).

---

## Out of scope (deferred to follow-on specs)

- **Launchable primed keg entity** (a fizzing barrel you can push/fling like
  Minecraft's primed TNT, with its own physics + `EntityKind`). v1 detonates in
  place. The Cart is the engine's first vehicle entity; a thrown keg can follow
  the same entity path later.
- **Industrial / ANFO route** — its own tier + spec (fertiliser + oil), per the
  owner's "industrial is the later route".
- **Bigger-charge keg variant** (recipe row 3) + directional/shaped charges.
- **Rubble drops** from blasted common terrain — possible later; **ore never
  drops from a blast**.
- **Fire spread** from detonation — off in v1.
- **Launching fireworks** — now *unblocked* by Black Powder, but is the Magnesium
  spec's follow-on, not this one.
- **TNT-mining ore for yield** — explicitly **never** (would break Proof-of-Play).

---

## Open questions

**None — resolved 2026-06-20** (owner: "go with your recommendations"):

1. **Blast radius + fuse feel** → **~4-block spherical radius, ~4 s (80-tick)
   fuse** for v1. Both stay tunable consts; Axolittle's playtest tunes the final
   numbers (this is the feel gate, not a build blocker).
2. **Blasted-block drops** → blasted blocks **drop nothing** (protects
   Proof-of-Play). Ore never drops from a blast regardless. Cosmetic rubble is a
   possible later tweak, not v1.
3. **Saltpetre farming inputs** → **plant trimmings / food waste → Compost →
   Saltpetre.** Manure deferred (no livestock manure drop exists yet; adding one
   is its own content).

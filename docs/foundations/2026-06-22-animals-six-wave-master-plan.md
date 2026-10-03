# Animals — Six-Wave Master Plan (spec + build order)

**Status:** SPEC + IN-BUILD 2026-06-22. Branch `feature/animals-six-waves`.
**Origin:** Deep-research report (`/workflows` deep-research, 2026-06-22 — Alex's
Mobs / Naturalist / Untamed Wilds / Genetic Animals / Domestication Innovation
download + design evidence, mob-vote-backlash analysis) consolidated against the
live 19-mob roster and the planned-but-unbuilt list. Owner direction: build all
six waves; pull the aquatic wave up and make the **Shark** its flagship (multiple
kids asked for sharks; research independently ranks sharks a top real-fauna draw
with a ready non-lethal drop mechanic).

## The 5 design laws (every animal obeys these)
1. **A purpose, not a presence** — each animal feeds a crafting/economy loop or a
   utility. No window-dressing mobs ("concrete, not cards").
2. **Reward interaction, not slaughter** — best products come from the animal
   doing something *while alive* (husbandry). This is the natural skin for
   Proof-of-Play + sats.
3. **Ecosystem participant** — predator/prey, herds, foraging, day/night.
4. **Feeds the economy** — products are Vendor SKUs; rarer/bred ones worth more.
5. **Tameables stay useful** — command states, no friendly-fire, no permanent loss.

> No-fantasy constraint holds throughout: real-world or AxeNStax-original
> creatures only (the ~14M-download Untamed Wilds proves real fauna is a
> strength). Automation maps to the **Electricity** grid, never redstone.

---

## WAVE 1 — Foundational systems (build FIRST; upgrade all 19 existing animals)

### 1A · Breeding genetics (inheritance)
- **Data:** a `Genetics` ECS component on breedable animals: `size: f32`
  (0.8–1.2), `tint: [u8;3]` (subtle coat variation), `yield_q: u8` (0–255 drop
  quality/quantity multiplier), `speed_q: u8`. Append-only `WorldSave` field
  `animal_genetics: Vec<SavedGenetics>` keyed by persisted-mob id (rides the
  Wave-2 `saved_mobs` infra; only persisted/tamed/bred animals carry it long-term).
- **Mechanics:** a baby's genes = per-gene `coin-flip(parentA, parentB)` + small
  bounded mutation. Pure free fn `genetics::breed(a, b, seed) -> Genetics`
  (fully unit-testable, deterministic).
- **Hook into existing breeding** (`breeding::tick_breeding`): when a baby spawns,
  derive + attach its `Genetics`. Render reads `size`/`tint` (scale + tint).
- **Benefits:** player attachment + an economy engine (breed up `yield_q`).
- **Unlocks:** champion bloodlines; premium Vendor stock; Mule (Wave 5) as the
  flagship cross-breed; Creator-Gallery "prize animal" exhibits.
- **Kid guard:** lightweight — visible traits + loot quality only; NO gender/
  gestation friction in v1.

### 1B · Husbandry products — non-lethal, behaviour-gated drops → sats/PoP
- **Mechanics:** generalise the egg/milk/wool cadence into a `husbandry` module:
  a content, fed, un-stressed animal periodically yields its product (richer if
  `yield_q` is high). Stress (recent damage, no food nearby) suppresses yield.
- **Proof-of-Play hook:** husbandry yields run the PoP hash like mining strikes —
  caring for animals is an earning loop on Bitcoin-enabled servers (server
  operator translates effort → sats; engine holds a score, not a balance).
- **Benefits:** ranch-as-a-business; rewards keeping animals alive (kid-friendly).
- **Unlocks:** the cleanest animal→sats hook; pairs with 1A (bred quality → value).

### 1C · Pet framework v2 (and it finishes Wolf + Nostrich) — **DELIVERED 2026-07-06** (pets-debt-water wave, Tasks 1-8b)
Command states (`CompanionState::{Follow,Stay,Wander,Perch}`, empty-hand cycle,
wolf sit toggle), no-friendly-fire, Pet Bed rescue, Recall Whistle, and the wolf
combat-assist movement all shipped. See `docs/spec/05-gameplay-systems.md §9.7`
+ `docs/superpowers/specs/2026-07-06-pets-debt-water-wave-design.md`.
- **Mechanics:** a `Companion` component with `state: Wander|Stay|Follow`
  (right-click cycles), `owner` pubkey (reuse `tameable::OwnershipData`).
  - **Command states** drive movement (Follow = the wolf/nostrich follow already
    half-built; Stay = hold; Wander = generic).
  - **No friendly-fire:** player melee/projectiles pass through own tamed pets
    unless sneaking (combat target filter).
  - **Anti-loss:** a **Pet Bed** block (respawn point for a downed pet next
    morning) + a **Recall Whistle** item (teleport-home a follow-pet on unload).
- **Finishes deferred work:** wolf combat-assist *movement* (resolve
  `WolfAction::AttackTarget` entity → move toward it) + tamed-nostrich Follow.
- **Benefits:** kills "useless after taming"; persistence (shipped) is the base.
- **Unlocks:** every Wave-2 companion becomes trivial; combat-pet + explorer-buddy
  playstyles.

---

## WAVE 2 (build order #4) — Aquatic flagship: "the Ocean comes alive"

> Pulled UP to be the first new-creature wave. Predator needs prey → Fish + Shark
> + Glow Squid ship together.

### 🐟 Fish (schooling, swimming) — *closes the "nothing swims" gap*
- **Behaviour:** schools that wander water; **panic-flee** (propagates to
  school-mates) from predators/players. Catchable by hand/net in shallows (not
  only rod-RNG).
- **Drops/products:** RawFish (already an item) on catch; live fish for aquariums.
- **Unlocks:** **fish farms/ponds** (aquaculture, farming T8 seed), visible
  renewable food, the prey layer the Shark needs.

### 🦈 Shark (apex predator) — FLAGSHIP
- **Behaviour:** patrols **deep ocean only** (telegraphed + avoidable — fun-scary,
  not cheap; kid-appropriate), hunts Fish + **wounded** prey, territorial.
- **Non-lethal drop:** **Shark Tooth** drips when it *attacks prey* (research
  template), not on death — husbandry-over-slaughter, no gore/finning.
- **New items/recipes:** `SharkTooth` material → **Serrated Blade** (high-tier
  knife/sword), **tipped arrows**, and a prized **Vendor trade good** (rare =
  risky to farm → sats hook).
- **Unlocks:** Ocean biome *stakes* (boats + coastal bases matter), deep-sea
  risk/reward, **shark-in-an-aquarium** Creator-Gallery showpiece.

### ✨ Glow Squid
- **Behaviour:** deep-water/cave drifter (extends Squid AI); non-lethal **Glow
  Ink** when calm.
- **New items/recipes:** `GlowInk` → **glow signs**, **glow item-frames**, soft
  **bio-lamp** light blocks (ties to decoration AND the Electricity light family).
- **Unlocks:** glow-decoration economy; aquarium ambiance.

---

## WAVE 3 (build order #3) — Companions (exploit 1C)

- **🐈 Cat** — tame with fish; companion states; passive **threat-ward** aura
  (deters brigands/hyenas near home). Unlocks "safe homestead"; cat-treat recipe;
  tradeable tamed cats. **DELIVERED 2026-07-06** (pets-debt-water wave, Task 9) —
  ward aura + guaranteed-tame Cat Treat; placeholder mesh (wolf-shared) noted as
  known, not a bug.
- **🦊 Fox** — predator/prey (hunts Rabbit + Chicken); steal/fetch behaviour;
  tame with berries; nocturnal. Unlocks real food-web tension (fence your hens).
- **🦜 Parrot** — shoulder-ride; **alarms on nearby threats**. This is the *first
  animal→signal bridge* → feeds Wave 4. **DELIVERED 2026-07-06** (pets-debt-water
  wave, Task 10) — flight (`Flying` marker) + `CompanionState::Perch`
  shoulder-ride + threat-alarm on cooldown; placeholder mesh known. The
  animal→Electricity signal bridge itself stays deferred with Wave 4/Aether.

## WAVE 4 (build order #5) — Electricity bridge (the differentiator)

- **Animal Sensor block** — emits an Electricity signal when an animal of type X
  is within range (livestock pressure-plate).
- **Animal-as-signal-source** — a perched Parrot's alarm, a charging Goat as a
  kinetic input, an animal treadwheel that generates grid power.
- **Unlocks:** mob-powered automation (auto-doors for your wolf, alarms,
  animal-count farm logic) — a combined animals+Electricity identity nothing with
  redstone can copy. *(No fantasy golem; trigger is a real animal or a sensor block.)*

## WAVE 5 (build order #6) — Logistics & build utility

- **🫏 Donkey + 🐴→🦓 Mule** — Donkey carries chests (mobile storage) + hitches to
  carts; **Mule = the genetics showcase** (sterile Donkey×Horse cross via 1A).
  Unlocks caravans hitched to rail freight; overland trade routes; farming-T2 draft.
  **DELIVERED 2026-07-06** (pets-debt-water wave, Tasks 11-12) — cargo pack
  (equip a Chest, sneak-click to open the 27-slot container) + Horse×Donkey→Mule
  cross-breeding (blended genetics, Mule sterile).
- **🦀 Crab** — coastal; drops a **claw → reach-extension build tool** (place
  blocks farther; the community-loved utility). Unlocks large-build ergonomics +
  beach biome life. **DELIVERED 2026-07-06** (pets-debt-water wave, Task 13) —
  coastal/sand spawn, Crab Claw drop, Reach Claw tool (+2 reach, both client and
  server-side anti-cheat honour it); placeholder mesh known.

## WAVE 6 (build order #7) — Economy fauna

- **🦆 Duck / 🪿 Goose / 🦃 Turkey** — premium egg/meat/**down/fat** (farming T6);
  net-new Vendor SKUs above chicken. Unlocks premium baking, **down bedding/
  insulation**, festive foods.
- **🐻‍❄️ Polar Bear** — tundra apex threat (extends Bear AI). **🦌 Reindeer** —
  cold-biome **draft/sled** animal + tundra signature. Unlocks sled transport on
  snow/ice; a reason to go north.

---

## New blocks / items / recipes (rolling registry — assigned at build time)
| Wave | New BlockIds | New MaterialIds | Recipes / experiences |
|------|--------------|------------------|------------------------|
| 1C | Pet Bed | Recall Whistle | pet-bed craft; whistle craft |
| 4-aq | (aquarium glass tank?) | SharkTooth, GlowInk, (live-fish bucket) | Serrated Blade, tipped arrows, glow signs/frames, bio-lamp |
| 2-comp | — | Cat Treat | cat-treat; (fox/parrot tame foods reuse berries/seeds) |
| 3-elec | Animal Sensor, Treadwheel | — | sensor + treadwheel craft; grid wiring |
| 5-log | — | Crab Claw (tool) | reach-tool; donkey chest = reuse chest |
| 6-econ | Down Bedding | DuckEgg, GooseEgg, Down, GooseFat, TurkeyMeat | premium baking, insulation |

## Save-format discipline
Each wave appends ONLY new `WorldSave` fields, LAST, `#[serde(default)]`, in build
order (current last field = `saved_mobs`). New per-mob component state
(`Genetics`, `Companion`) rides the `saved_mobs` persistence infra. Update the
`is_known` block-id boundary test + the byte-layout decode tests on every new
block / save field (per the Wave-2 lesson: +N trailing bytes shifts those tests).

## Testing + boundary
Pure-function cores (genetics breed, husbandry cadence, schooling/panic, shark
target-selection, sensor logic) are unit-tested off a bare `hecs::World`.
`check.sh` (native + wasm + bundle) green before every merge. Hand-authored
creature **models + feel-tuning (danger levels, cadences, yields) = Axolittle
playtest boundary**; ship clean proc-model placeholders so each wave is testable.
Per-wave test sheets under `docs/test-sheets/`.

# Animals Wave 2 — persistence · dormant-AI dispatch · product loop

**Status:** DELIVERED 2026-06-22 (autonomous build from the full animal audit) —
`check.sh` green.
**Branch:** `feature/animals-wave2`.
**Trigger:** A full animal audit (see chat 2026-06-22) found the mob system was
deep but had two systemic gaps: a lot of per-species AI was *written but never
dispatched*, and tamed/bred animals didn't survive save/load. The owner picked
all four prioritised fixes.

## #1 — Tamed-pet persistence (the biggest felt gap)

Tamed wolves + nostriches now survive save/load instead of vanishing each
session. Implementation mirrors the proven `carts` plumbing:

- New world-level `WorldSave.saved_mobs: Vec<SavedTamedPet>` (append-only LAST
  field; bincode is positional). `SavedTamedPetData` gained a `Nostrich`
  variant + a `mob_type()` helper.
- `save::tamed_mobs_to_saved(ecs)` snapshots tamed Wolf/Nostrich entities. The
  owner pubkey + AI/tame state travel **verbatim inside** `WolfData`/
  `NostrichData`, so there's no owner→slot remap on restore.
- Threaded through `save_world` / `autosave_world` (native + wasm); restored in
  `chunk_stream::initial_load` (spawned NOT `Scattered`, so they stick).
- **Within-session leak also fixed:** taming now drops the `Scattered` tag, so a
  tamed pet survives chunk-unload (previously it despawned the moment you walked
  away, before any save).
- Untamed wildlife is deliberately **not** persisted — it re-scatters
  deterministically.

Decode tests that hard-code the serialized tail length were updated for the
+8-byte field.

## #2 — Dispatch the four dormant species AIs

Horse, Squid, Bear, Hyena each shipped a complete, unit-tested pure-fn AI that
was **never called**. Now dispatched via `species_ai` (mirrors the Wave-1
rabbit/goat/bee dispatchers), with their data components attached at
`spawn_mob`:

- **Horse** — wide herd-wander; skips ridden horses; generic struck-flee still
  owns the bolt.
- **Squid** — aquatic 3-D drift; suffocation damage when stranded on land.
- **Hyena** — `hyena_ai::tick` flips Lazy (day) ↔ Hunt (night), Aggro after a
  hit. Hunt/Aggro drive hard at the nearest player; Lazy damps the generic
  hostile chase so daytime hyenas lounge.
- **Bear** — food-raiding: smells nearby ripe crops + food-chests, walks to
  them, eats/raids. World mutations (crop→tilled soil; take one food) returned
  as `BearWorldEffect`s for the `GameState` wrapper to apply.

## #3 — Finish the half-built interactions

The `animal_products` cadence (`should_lay_egg` / `can_milk`) shipped long ago
but was never called.

- **Chicken eggs** — each chicken carries an `AnimalProductState`; a per-tick
  pass drops an Egg at its feet on cadence (mirrors the nostrich egg pass).
- **Cow milking** — right-click a cow with a Bucket → Milk Bucket, gated by the
  milk cooldown.
- **Bee sting-on-attack** — `dispatch_bees` now reads each bee's `LastAttacker`
  and surfaces it as the recent attacker; a struck bee darts at the player and
  stings (half-heart + the bee dies, MC parity).

**Deferred (companion/combat-movement polish, genuinely more plumbing):**
- **Wolf combat-assist movement** — `WolfAction::AttackTarget { entity_id: u64 }`
  carries an ambiguous id that needs resolving to a live entity position before
  the wolf can chase a target; today it holds position by its owner.
- **Tamed-nostrich follow** — the Nostrich has a `Follow` state but no clean
  tick/action to dispatch; the follow-movement would be written from scratch
  (like `tick_wolf_companions`).

## #4 — Sheep shearing

Right-click a sheep with Shears → 1–3 Wool, gated by a regrow cooldown (reuses
the `AnimalProductState` cadence). Previously wool was death-drop only.

## Components added at spawn
`HorseData`, `SquidData`, `BearData`, `HyenaData` (dormant-AI dispatch) +
`AnimalProductState` on Chicken/Cow/Sheep (product cadence).

## Verification
`check.sh` green (native + wasm + tests). New tests: persistence extraction +
Nostrich wire round-trip (2), the four dispatchers (4), bee sting (1) = 7, plus
the existing `animal_products` / `*_ai` unit suites. Feel-tuning (hyena
aggression, bear raid frequency, egg/milk/wool cadences) is the playtest
boundary — test sheet: `docs/test-sheets/2026-06-22-animals-wave2.md`.

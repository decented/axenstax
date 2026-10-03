# Authoring a Trial (Explorer Challenge)

This is a copy-pasteable recipe for adding **one new Explorer Challenge trial**
to AxeNStax. It is written so a weak/cheap model can follow it mechanically
and still ship something correct — every step names the exact file, the exact
thing to add, and what breaks (silently, or loudly) if you get it wrong.

A trial today is **one JSON file plus five small Rust edits** scattered across
two files. There is a lint suite (`trials_lint`, see the very last section)
that turns almost every mistake into a loud, specific `cargo test` failure —
but only if you actually run it. **Always finish by running it.**

Background/design docs (read only if you want the "why", not required to
author a trial): `docs/foundations/2026-06-06-feature-coverage-challenges.md`
(the event/objective system) and `docs/foundations/2026-06-24-trials-fun-redesign.md`
(the fun-first voice/arena redesign these trials follow).

All file paths below are relative to `game/engine/`.

---

## 1. The annotated `ScenarioDef` schema

A trial's JSON deserialises into `ScenarioDef` (defined in `src/scenario.rs`).
Fields with no default are **required** — omit them and `load_scenario_def`
returns a parse error. Fields with a default are optional.

```jsonc
{
  // REQUIRED. For a hand-authored trial this is always the literal string
  // "Challenge" (the other ScenarioKind values — HashDash/SatoriRush/
  // Test — are the engine's own built-in game modes, not something you author).
  "kind": "Challenge",

  // REQUIRED. The one-line premise shown in the Trials list, e.g.
  // "Quarry Run — break twenty blocks and feel a mine take shape".
  "display_name": "Title — one-line premise",

  // GOTCHA: this exact string is ALSO the reverse-lookup key. The running
  // scenario only carries its display_name at runtime (not your JSON's
  // filename or token) — so `challenge_help_for_display`, `trial_satoshi_for_display`,
  // and `trial_recipe_hints_for_display` all match the LIVE display_name
  // against `challenge_listing()`'s (token, display_name) pairs to find your
  // authored help/voice/hints. If display_name here doesn't byte-for-byte
  // match what challenge_listing() reports (i.e. what's actually in this same
  // JSON file), those reverse-lookups silently return ("", "") / &[] and the
  // in-game help panel goes blank. In practice: just don't hand-edit
  // display_name in two places that could drift — there IS only one place
  // (this field), so this only bites if you literally paste an old
  // display_name string somewhere by mistake.

  // OPTIONAL (default: empty). Items placed in EVERY local player's hotbar
  // when the trial starts. `name` is a `/give`-vocabulary token (snake_case,
  // hyphens also accepted — normalised to underscore) — see §4 "Valid names".
  "kit": [
    { "name": "stone_pickaxe", "count": 1 }
  ],

  // REQUIRED. What ends the trial. See §2 "Objective shapes" below — for a
  // new Explorer Challenge this is almost always `Action`, `Sequence`, or
  // `Checklist` (Timed/FirstSatori/FreeRoam are for the built-in games, not
  // Explorer Challenges).
  "objective": {
    "Action": {
      "event": { "BreakBlock": { "block": null } },
      "count": 20
    }
  },

  // OPTIONAL (default: "None" — no running score; the trial's HUD is the
  // objective checklist, not a score readout). Every current Explorer
  // Challenge either omits this or doesn't need it — leave it out unless
  // you're building a score-attack (Scavenger's InventoryVariety is the one
  // exception among Challenges).
  // "scoring": "None",

  // OPTIONAL (default: null). A fixed world seed for a fair repeatable map.
  // Explorer Challenges never set this — omit it (each trial's arena is
  // placed relative to wherever the player spawns, not a specific seed).

  // OPTIONAL (default: false). Hides "Switch to Creative" in the pause menu
  // during the run. Explorer Challenges leave this unset/false.

  // OPTIONAL (default: "Reuse"). World lifecycle across replays. Explorer
  // Challenges leave this unset (they overlay on the player's CURRENT world,
  // not a dedicated arena world — see `provision_all_players`'s fair_start
  // logic: only a Timed Challenge, like Scavenger, wipes the inventory).

  // OPTIONAL (default: null each). Fresh-arena world shape, applied only when
  // the Trials board LAUNCHES a fresh world for this challenge (not when a
  // player runs `/scenario <token>` in their own world). Every current
  // Explorer Challenge sets `game_mode` and `time_lock` (below) plus
  // `mobs_enabled` (further below) to a fixed value; `weather_lock` (also
  // below, between them) is genuinely optional — only one trial uses it so
  // far (see its own note).
  "game_mode": "survival",   // or "creative" (only if the trial genuinely
                             // needs flight/instant-break/infinite blocks —
                             // e.g. "build", "lava-floor", any Tech trial
                             // wiring cable/pistons/logic).
  "time_lock": "day",       // or "night" (only three trials that want the
                             // ambient dark currently use "night" —
                             // "power"/"logic-gate"/"generator", the Tech
                             // trials whose glow/circuit effect reads best
                             // after dark; most trials use "day").

  // OPTIONAL (default: null). Pins the LOCAL weather window for as long as the
  // trial runs: exactly "clear" | "rain" | "storm". Unlike the three fields
  // above (world-shape values baked into the fresh arena's WorldMeta), this is
  // re-applied every tick from the RUNNING scenario, so `/scenario <token>` in
  // your own world pins the weather too — and it lands before the tick samples
  // the wind, which is what lets "catch-the-wind" guarantee a turning windmill.
  // An unrecognised value is IGNORED at runtime (forward-compatible defs) and
  // fails `trials_lint`'s `every_bundled_weather_lock_is_a_value_the_engine_honours`
  // at test time. A Workshop world still never has weather, lock or no lock.
  "weather_lock": "storm",
  "mobs_enabled": false,    // Turns off AMBIENT/ecosystem mob spawning so
                             // nothing except your authored `arena.mobs`
                             // exists. Every current Explorer Challenge sets
                             // this to false — arena-placed mobs spawn
                             // regardless of this flag (see `apply_arena_setup`
                             // in game_loop.rs), so this only suppresses
                             // UNWANTED extra wildlife wandering in.

  // OPTIONAL (default: null). Only set on the three ⚡ Race scenario wrappers
  // (sprint/cross-country/marathon) — never on an Explorer Challenge. Leave
  // it out.

  // OPTIONAL (default: null). Blocks + mobs planted next to the player's
  // spawn so the challenge is completable SOLO. `at` is a `[dx, dy, dz]`
  // offset from the player's spawn position (NOT world coordinates) —
  // applied by `GameState::apply_arena_setup` in game_loop.rs.
  "arena": {
    "blocks": [
      { "at": [3, 0, 0], "block": "stone" }
    ],
    "mobs": [
      { "at": [3, 0, 0], "mob": "wolf", "count": 1 }  // "count" optional, default 1
    ]
  }
}
```

**Unknown-field guard (test-time, not runtime):** at RUNTIME,
`load_scenario_def` is deliberately **tolerant** — a typo'd/unknown field name
(e.g. `"displayname"` instead of `"display_name"`, or `"arena_seeed"`) is
**silently ignored**, not a parse error, and the real field just takes its
default. This tolerance is on purpose and must stay: `ScenarioDef` is also
persisted into a world's save (`WorldMeta.scenario_def`, re-parsed on load for
resumable Satori Rush) and deserialised from community-published open-stash
mods authored against other client versions — making unknown fields fatal
would break a stale save or a forward-published mod (the project's
tolerant-old-decode philosophy). A genuinely **missing required field**
(`kind` / `display_name` / `objective`, or a required nested field like
`arena.blocks[].block`) *does* still fail `load_scenario_def` loudly, as before.

**BUT for a BUNDLED trial you still get a loud failure** — the strictness lives
in `trials_lint` at test time. `every_bundled_field_is_recognised` (in
`src/test_integration/trials_lint.rs`) parses each bundled JSON both literally
and through `ScenarioDef`, then asserts every key you wrote survives the
round-trip; a dropped (typo'd) key fails the test, naming the trial and the
exact key path (e.g. `arena.mobs[0].conut`) at any depth. So: **always run
`cargo test trials_lint`** — that, not the parser, is what catches a field-name
typo in a trial you're authoring.

---

## 2. Objective shapes — and the one rule that matters most

```rust
Objective::Action { event: ChallengeEvent, count: u32 }       // "do X, N times"
Objective::Sequence { steps: Vec<Objective> }                  // ORDERED: step 0 must finish before step 1 counts
Objective::Checklist { items: Vec<Objective> }                 // UNORDERED: any order, all must finish
Objective::Timed { ticks: u32 }                                // built-in games only — not for Explorer Challenges
Objective::FirstSatori | Objective::FreeRoam                   // built-in games only — not for Explorer Challenges
```

**THE RULE: every leaf inside a `Sequence` or `Checklist` must be a plain
`Action`.** Nesting a `Sequence` inside a `Sequence`, or a `Timed` inside a
`Checklist`, *parses fine* — but `ScenarioState::objective_is_complete` only
recognises `Action` leaves, so a non-`Action` leaf can **never complete** and
the trial silently stalls forever for every player who reaches it. This is
exactly what `trials_lint`'s `sequence_and_checklist_leaves_are_actions` check
catches — if you break this rule, that test fails and names the exact step
index.

JSON shape reminders (confirmed against the bundled JSON):
- A unit-variant event (no fields) serialises as a bare string: `"event": "TameMob"`.
- A struct-variant event serialises as `{"Variant": {field: value}}`:
  `"event": { "BreakBlock": { "block": null } }` (or `{"block": 5}` for a
  specific block id — but you'll almost always use a **name**, not a raw id,
  when hand-authoring; see §4).
- `Sequence`/`Checklist` wrap a `steps`/`items` array of `Action` objects — see
  `onboarding.json` (Sequence) and `explorer-funny-farm.json` (Checklist) for
  real examples.

---

## 3. The event palette

Every `ChallengeEvent` variant, what gameplay action fires it (verified fire
sites in `src/game_loop.rs`), and whether it carries a per-target filter.
"Unfiltered" means the objective can only ever say "any", never "this
specific one" — see §6 for what that implies for your arena.

| Variant | Fires when… | Filter |
|---|---|---|
| `BreakBlock { block }` | A block-break completes (survival AND creative/instant breaks — the latter fire with 0 work). | `Some(id)` = that block only; `None` = any block. |
| `PlaceBlock { block }` | A block-place commits. | `Some(id)` = that block only; `None` = any block. |
| `CraftItem` | A craft completes at the `ResultSlot` click (any recipe) — fires the same for BOTH the 2×2 personal-inventory grid and the 3×3 crafting-table grid, so a `CraftItem` objective is satisfiable without a table. | Unfiltered (v1 has no per-recipe filter). |
| `CookAtCampfire` | A cooked item is taken OFF a campfire. | Unfiltered. |
| `TameMob` | A companion tame (cat/parrot/fox, `companion::tame_food`) OR a wolf bone-tame succeeds — two separate fire sites, same event. | Unfiltered — no species filter (see below). |
| `VendorSale` | A Vendor trade/barter settles (either the plan-buy path or the direct swap path). | Unfiltered. |
| `WorkshopPublish` | The player confirms publishing a Workshop reskin/reshape. | Unfiltered. |
| `ClaimPlot` | A plot claim marker is planted. | Unfiltered. |
| `GainMaterial { material }` | A `MaterialId` (e.g. a Satori gem) enters the inventory. | `material` is a required, always-matched filter (no `None` option) — but no bundled Explorer Challenge currently uses this event; see the JSON shape below before you author the first one. |
| `KillMob` | The player lands the attributed killing blow on a mob. | Unfiltered — genuinely ANY mob death counts (this is intentional, not a gap). |
| `EatFood` | The player successfully eats a held food item (hunger/health gate applies — eating while full is a no-op, no event). | Unfiltered. |
| `HarvestCrop` | A block break happens on a crop block (`growth::is_crop`) — fires *in addition to* `BreakBlock` on that same break. | Unfiltered. |
| `SmeltItem` | A finished smelt is taken from a furnace's OUTPUT slot. | Unfiltered. |
| `CatchFish` | A fishing line successfully lands a catch. | Unfiltered. |
| `RideEntity` | The player mounts a rideable mob (`mob::is_rideable`) OR a minecart. | Unfiltered — species not distinguished (cart vs. horse vs. Nostrich all fire the same event). |
| `PowerDevice` | An electric lamp transitions to lit. | Unfiltered. |
| `UsePiston` | A piston (regular or sticky) actually moves a block this tick. | Unfiltered. |
| `Detonate` | A blasting keg's fuse reaches 0 and it explodes. | Unfiltered. |
| `UseBucket` | A bucket is filled OR emptied (BOTH ends fire — a "fill then empty" objective needs 2 counted). | Unfiltered. |
| `ShearOrMilk` | A sheep is sheared OR a cow is milked (both fire the same event). | Unfiltered — species not distinguished. |
| `BreedAnimals { offspring }` | Two adults pair and produce a baby. | `Some(kind)` = only when the baby is that species (e.g. `Mule` — the one cross-species hybrid, Horse×Donkey); `None` = any offspring. This is the ONE mob event with a real filter. |
| `SourceTurned { kind }` | A self-driving power source goes idle→**turning**: a Water Wheel catching a current, a Windmill catching the wind. Fires ONCE per transition (never while it keeps turning, never when it stops). Distinct from `PowerDevice`, which is the far end of the circuit. | `kind` is a required `TurningSource` filter — `"WaterWheel"` or `"Windmill"`, no "any" option (the two are different lessons). |
| `CompleteBuildGuide` | A laid build-along guide finishes — every cell of the plan verifies `Correct`. | Unfiltered (no per-plan filter). |
| `SaveSkin` | A painted avatar skin is pinned in the Workshop: saved to the wardrobe **and** worn. | Unfiltered. |
| `SpawnRig` | A rig authored in Rig Studio is spawned into the world. | Unfiltered. |
| `SendFeedback` | A `/bug` or `/idea` report is queued for the makers (native outbox only — the web build has no feedback channel). Local only: nothing about the report reaches the scenario. The only trial using it (`suggestion-box`) is hidden unless tester feedback is switched on (`scenario::FEEDBACK_TRIAL`). | Unfiltered. |

**`SourceTurned` JSON encoding:**
```json
"event": { "SourceTurned": { "kind": "Windmill" } }
```

**Arena-shaped gotcha for the two turning sources:** the reachability check (§6)
counts `arena.blocks` only, so a `PlaceBlock { block: Some(<wheel|mill>) }` step
whose block comes from the **kit** needs a `REACHABILITY_MANUALLY_VERIFIED`
entry — see `mill-race` / `catch-the-wind` for the pattern. And a windmill needs
`weather_lock: "storm"` (or real altitude) if the trial is to be reliably
completable: a clear day at sea level is intermittent by design.

**`GainMaterial` JSON encoding (untested by any bundled trial — verified against
the enum, not copied from a real JSON):** `MaterialId` (`src/item.rs`) is a
plain unit-variant enum (`Stick`, `Coal`, `Diamond`, `Satori`, …), so it
serialises as a bare string — same pattern as `BreedAnimals { offspring }`'s
`MobType` filter. The full event is:
```json
"event": { "GainMaterial": { "material": "Satori" } }
```
(swap `"Satori"` for any other `MaterialId` variant name, e.g. `"Coal"`,
`"Diamond"`). There is no `None`/any-material option — `GainMaterial` always
names one specific material.

**Species-gated events — the trap the lint fixes:** `TameMob`, `ShearOrMilk`,
and `RideEntity` are unfiltered in the enum, but in REAL gameplay only certain
species can ever fire them:
- `TameMob` fires only for a companion species (cat/parrot/fox — whichever has
  `companion::tame_food`) or the Wolf.
- `ShearOrMilk` fires only for Sheep (shear) or Cow (milk).
- `RideEntity` fires only for `mob::is_rideable` species: Horse, Donkey, Mule,
  Nostrich (or the kit-provided cart).
- `KillMob` really is any mob — no species restriction.

If your arena's `arena.mobs` doesn't include a mob of the RIGHT species for
these three events, the trial is **unwinnable** and `trials_lint`'s
`arena_provides_objective_targets` check will fail, naming the trial and the
species it needs. This is exactly the bug class the harness exists to catch
(a `TameMob` objective over a cow-only arena used to pass silently before the
lint existed).

**Unfiltered-over-multiple-species is a DIFFERENT, separate check:** even
with the right species present, if your arena stocks **more than one
distinct species** for `KillMob`/`TameMob`/`ShearOrMilk`/`RideEntity`, the
lint's `unfiltered_event_trials_are_intentional` check demands you
consciously acknowledge "any of these is fine" by adding an entry to the
`UNFILTERED_EVENT_INTENTIONAL` allowlist in
`src/test_integration/trials_lint.rs` (with a one-line reason). If your
arena only ever stocks ONE species for these events (the normal case), you
need no allowlist entry at all.

---

## 4. Valid names — only use names that resolve

Everything you type as a name in a trial's JSON gets resolved through a real
registry at parse/apply time. An unresolvable name doesn't error loudly by
itself in every path — it's `trials_lint`'s
`every_challenge_kit_and_arena_name_resolves`-style test (in `scenario.rs`)
that catches it, so **only use names from these sources**:

- **`kit[].name` and `arena.blocks[].block`** — resolved via
  `commands::builtins::give::resolve_item` in
  `src/commands/builtins/give.rs`. This is the exact `/give <name>` vocabulary
  (hyphens accepted, normalised to underscore). It covers placeable blocks,
  tools (`<material>_<tool>`, e.g. `stone_pickaxe`, `wooden_sword`,
  `iron_hoe`), single-tier tools (`bow`, `flint_and_steel`), and materials
  (`stick`, `bone`, `raw_fish`, `wheat_seeds`, `coal`, …). If the name you
  want doesn't exist yet, add an alias to `block_by_name` / `tool_by_name` /
  `material_by_name` in that file — this is the "conditional /give alias"
  step, only needed for a genuinely new item, never for an existing one.
- **`arena.mobs[].mob`** — resolved via `mob::MobType::from_name` in
  `src/mob.rs`. Valid snake_case ids (verified, ~30 total): `cow`, `chicken`,
  `pig`, `sheep`, `villager`, `peddler`, `wolf`, `horse`, `rabbit`, `goat`,
  `bee`, `squid`, `nostrich`, `bear`, `hyena`, `brigand`, `marauder`,
  `berserker`, `knight`, `fish`, `shark`, `glow_squid` (or `glowsquid`),
  `fox`, `polar_bear` (or `polarbear`), `reindeer`, `cat`, `parrot`,
  `donkey`, `mule`, `crab`.
- **`trial_recipe_hints(token)` entries** (only if you add one — see §5 step
  6b) — must resolve via `resolve_item` AND have a real recipe card, checked
  via `crafting_catalogue::recipe_index_for_output`. Only add a hint for
  something the player actually crafts as part of the objective; it just
  auto-pins the recipe in the inventory UI, it isn't required.

If any of these names typo or don't exist, the relevant test fails naming the
challenge, the field, and the bad name — fix the spelling or (for a genuinely
new item) add the alias.

---

## 5. The places to touch — in order, with failure modes

Adding one new trial with token `<token>` (kebab-case, e.g. `"torch-relay"`)
touches these files. Do them in this order.

### (1) The JSON file — REQUIRED
**File:** `game/engine/assets/scenarios/explorer-<token>.json`
Author the full `ScenarioDef` per §1. **Failure mode if skipped/wrong:** the
file doesn't exist → step (2)'s `include_bytes!` is a **compile error**
(missing file). If the JSON is malformed or missing a required field, every
test that parses bundled defs panics with a clear serde error naming the file.

### (2) `EXPLORER_CHALLENGES` — REQUIRED
**File:** `game/engine/src/scenario.rs`, the `EXPLORER_CHALLENGES` const
(currently 45 entries, alphabetically-unordered — just append a line).
**Add:** `("<token>", include_bytes!("../assets/scenarios/explorer-<token>.json")),`
**Failure mode if skipped:** the trial is **completely inert** — it doesn't
show up in `challenge_listing()`, `/scenario <token>` doesn't resolve it,
`named_builtin_def` returns `None` for it, no test even looks at it. No error,
no crash — it just silently doesn't exist. (This is also exactly why the
`_skeleton.example.json` template shipped alongside this guide is safe to
leave in the repo unregistered — it's inert until you add this line.)

### (3) `TRIAL_ORDER` — REQUIRED
**File:** `game/engine/src/trials.rs`, the `TRIAL_ORDER` const.
**Add:** `(TrialType::<Make|Tend|Brave|Tech|Race>, TrialRef::Challenge("<token>")),`
at a reasonable position (roughly: easier/foundational trials first, deeper
systems later — look at neighbouring entries for a similar trial and slot
near it). Pick the `TrialType` that best matches the trial's flavour (see the
existing entries for examples: `Make` = building/crafting/production, `Tend`
= animals/farming/trade, `Brave` = combat/danger, `Tech` = electricity/
logic/mechanisms, `Race` = movement).
**Failure mode if skipped:** the test `trial_order_covers_every_trial_once`
(in `trials.rs`) **fails** with `"every Challenge must appear in TRIAL_ORDER"`
— your trial exists (from step 2) but never appears in the in-game Trials
lobby list.

### (4) `trial_task_labels` — REQUIRED (or the trial reads with generic wording)
**File:** `game/engine/src/scenario.rs`, the `trial_task_labels` function.
**Add:** `"<token>" => &["<one label per objective leaf, in order>"],`
The **number of labels must exactly match the number of objective leaves**
(1 for a plain `Action`, N for a Sequence/Checklist with N steps).
**Failure mode if wrong count:** `authored_task_labels_align_with_objective_steps`
(test, in `scenario.rs`) **fails**, naming the token and both counts.
**Failure mode if omitted entirely:** no failure — `trial_task_labels`
returns `&[]` for an unknown token and the UI falls back to generic
auto-derived wording (e.g. "Break 20 blocks") instead of your specific
phrasing ("Chop down the oak tree"). Not a bug, just worse UX — always add
this.

### (5) `challenge_help` — REQUIRED
**File:** `game/engine/src/scenario.rs`, the `challenge_help` function.
**Add:** `"<token>" => ("<one-line tagline>", "<detailed how-to>"),`
**Failure mode if omitted:** `every_bundled_challenge_has_authored_help` (and
`challenge_help_resolves_by_display_name`) **fail**, asserting the tagline/
how-to are non-empty for every entry in `challenge_listing()`.

### (6) `trial_satoshi` — REQUIRED
**File:** `game/engine/src/scenario.rs`, the `trial_satoshi` function.
**Add:** `"<token>" => ("<Satoshi's intro line>", "<Satoshi's hint line>"),`
**Failure mode if omitted:** `every_trial_has_satoshi_voice` (test) **fails**
— every bundled challenge token needs a non-empty intro + hint.
**COMPLIANCE (checked by this same surface, and by `trials_lint`):** never
use money/earning words in this text (or anywhere else authored) — see §7.

### Conditional extras (only when they genuinely apply)
- **(6b) `trial_recipe_hints`** (`scenario.rs`) — only if your objective
  involves crafting something specific and you want its recipe auto-pinned in
  the inventory UI. Optional; `&[]` (the default, via the `_ => &[]` arm) is
  fine and most trials use it.
- **A `/give` alias** in `src/commands/builtins/give.rs` — only if a kit item
  or arena block/mob name you need doesn't already resolve (see §4). Adding
  one is additive and safe; don't rename or remove an existing alias (other
  trials/tests may depend on it).
- **A new `ChallengeEvent` variant** (`scenario.rs`'s enum + its `matches`
  method + `event_task_label`'s match + a real `fire_challenge`/`on_event`
  call site in `game_loop.rs` at the actual gameplay action) — only if no
  existing event (see §3's full palette) fits what you want to track. This is
  rare; check the palette twice before doing this. If you do add one, you
  MUST also teach `trials_lint`'s `event_variant_name` exhaustive match (in
  `src/test_integration/trials_lint.rs`) about it — it's written as an
  exhaustive match specifically so a missed variant is a **compile error**,
  forcing you to update the lint alongside the enum.

---

## 6. Reachability — the arena must actually supply what the objective needs

If your objective names a concrete arena resource (a mob-count event, or a
block-count event with a specific block filter, or `HarvestCrop`, or
`BreedAnimals`), `trials_lint`'s `arena_provides_objective_targets` check
verifies your `arena` actually supplies enough of it — see §3's species-gating
note. If it doesn't, you get a clear failure naming the trial, the event, and
the shortfall.

**Known limitation:** for a filtered `BreakBlock { block: Some(id) }` or
`PlaceBlock { block: Some(id) }` objective, the check counts ONLY
`arena.blocks` entries — it never looks at `kit`. A trial that hands the
target block to the player via `kit` (rather than placing it in the world via
`arena.blocks`) will fail this check even though a player could genuinely
complete it. Either add a matching `arena.blocks` entry for the filtered
block, or (if the block is only ever obtained some other way, e.g. crafted or
kit-provided) add the trial to `REACHABILITY_MANUALLY_VERIFIED` as described
below.

**If a trial's completion genuinely can't come from a statically-countable
arena resource** — e.g. a block that's *formed* by a world reaction (lava +
water → obsidian) rather than authored directly, or a kit-provided cart
rather than an arena mob — the check can't see that, and you must add an
entry to `const REACHABILITY_MANUALLY_VERIFIED` in
`src/test_integration/trials_lint.rs`: `("<token>", "<one-line reason>"),`.
**Keep this list minimal** — there's a companion test
(`reachability_allowlist_entries_are_actually_needed`) that fails if you add
an entry for a trial that would actually pass the static check anyway (i.e.
you don't need the bypass). Only reach for this when the static rule
genuinely can't express your trial's reachability — most trials never need it
(only 4 of the 45 current ones do).

---

## 7. Compliance — no money/earning words, anywhere

**Never** use money/earning framing in any authored text: `display_name`,
task labels, kit item names, `challenge_help`, or `trial_satoshi`. This is a
regulatory red line (see the project's `CLAUDE.md` "Regulatory Red Lines"),
not a style preference. `trials_lint`'s `compliance_banlist_covers_all_text_surfaces`
check (plus the pre-existing `trial_text_has_no_money_or_earning_words` test)
scans every one of those surfaces for whole-word matches against:

```
sats, bitcoin, btc, earn, earning, earned, payout, payouts, wallet, money,
cash, cashback, prize, prizes, sell, buy, lightning, wages, salary
```

**Use instead:** "trade", "barter", "swap" (a Vendor trial is a fair barter,
never a sale). "Trade your spare stone with the vendor" is fine; "Sell your
spare stone to the vendor" is not.

---

## 8. Finish: run the checks

After authoring, run (from `game/engine/`):

```bash
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=<your build dir> cargo test --lib trials_lint
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=<your build dir> cargo test --lib scenario
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=<your build dir> cargo test --lib trials
```

**ALL of these must be green** before the trial is correct. Every failure
message is written to tell you exactly what's wrong and where — read it, it
names your trial's token, the field, and the fix. Don't hand-wave past a
failure; each one maps to a real way the trial would ship broken or
un-completable. Finish with a full `cargo test --lib` (or
`./check.sh`) before considering the trial done — the whole suite must stay
green, not just the trial-specific tests.

---

## 9. Worked example — adding "Sandcastle" end to end

A minimal trial: place 10 blocks of sand. Deliberately simple — a single
unfiltered `Action` objective needs no arena at all (no species-gating, no
reachability allowlist, no unfiltered-mob allowlist).

**(1) `game/engine/assets/scenarios/explorer-sandcastle.json`:**
```json
{
  "kind": "Challenge",
  "display_name": "Sandcastle — pile up ten blocks of sand into your own castle on the beach",
  "kit": [
    { "name": "sand", "count": 10 }
  ],
  "objective": {
    "Action": {
      "event": { "PlaceBlock": { "block": null } },
      "count": 10
    }
  },
  "game_mode": "creative",
  "time_lock": "day",
  "mobs_enabled": false
}
```

**(2) `scenario.rs`, `EXPLORER_CHALLENGES`** — append:
```rust
("sandcastle", include_bytes!("../assets/scenarios/explorer-sandcastle.json")),
```

**(3) `trials.rs`, `TRIAL_ORDER`** — add near other `Make`-flavoured building
trials (e.g. next to `"build"` or `"splash-zone"`):
```rust
(TrialType::Make, TrialRef::Challenge("sandcastle")),
```

**(4) `scenario.rs`, `trial_task_labels`** — one label (one leaf, a plain `Action`):
```rust
"sandcastle" => &["Place 10 blocks of sand to build your castle"],
```

**(5) `scenario.rs`, `challenge_help`:**
```rust
"sandcastle" => (
    "Pile up ten blocks of sand into your very own beach castle.",
    "You've got a stack of sand. RIGHT-CLICK to place it down, block by \
     block, wherever you like on the beach. Stack it up, spread it out — \
     any shape counts. Place 10 blocks in all to finish your castle.",
),
```

**(6) `scenario.rs`, `trial_satoshi`:**
```rust
"sandcastle" => (
    "Nothing beats an afternoon on the beach with a pile of sand and no plan \
     at all. Stack it however you like — there's no wrong castle.",
    "Just keep placing sand — ten blocks down and your castle's done, in \
     whatever shape you fancy.",
),
```

No conditional steps needed: `sand` already resolves via `/give`, `PlaceBlock`
already exists as an event, and no recipe hint applies (nothing is crafted).

Then run the checks in §8. If every one is green, "Sandcastle" is a
correctly-wired trial.

---

## The skeleton

`game/engine/assets/scenarios/_skeleton.example.json` is a minimal, valid,
**unregistered** `ScenarioDef` — copy it as the starting point for your
`explorer-<token>.json`, then fill in `display_name`/`kit`/`objective`/`arena`
for your trial. It is deliberately **not** listed in `EXPLORER_CHALLENGES`, so
it's inert: nothing loads it, no test scans it, `/scenario` can't launch it.
It only becomes live once you copy-and-rename it and complete step (2) above.

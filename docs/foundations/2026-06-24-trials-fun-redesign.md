# Trials — Fun-First Redesign (every system, a mini-adventure, with Satoshi)

**Date:** 2026-06-24 · **Status:** Built + shipped (v0.2.4) · **Owner ask:** "Review all the
trials. Each needs a drop-down description and clear guidance, a pop-up, and dialogue + help
from Satoshi. Review every aspect of the game and make sure there's a trial that tests each
one — right location, right tools, right materials (they may need to dig or craft them). The
emphasis is on **fun**."

This doc is the source of truth for the Trials content + the Satoshi-on-trials integration.
The trials themselves are authored as **data** (`game/engine/assets/scenarios/*.json`) so they
survive an engine rebuild; the prose + Satoshi voice live in `scenario.rs`
(`challenge_help`, `trial_satoshi`) and the race text in `trials.rs::CATALOG`.

## What a Trial is (and where the player meets it)

Trials are surfaced in the lobby's **Trials column** (`menu.rs::draw_trials_column`) in two
families, each row a **drop-down**:

- **⚡ Races** (`trials.rs::CATALOG`) — ghost time-trials. Drop-down `blurb` + a "What to do?"
  modal (`how_to`).
- **🎯 Challenges** (`scenario.rs`, the `EXPLORER_CHALLENGES` + onboarding defs) — guided
  mini-adventures. Drop-down `blurb`/tagline + "What to do?" modal (`how_to`).

In-world, every trial has:

- the **H objective pop-up** (`hud_ui::draw_objective_panel`) — title + step-by-step `how_to`
  + live progress (`done / total`) + **Satoshi's tip** (his `hint`), appended under the steps;
- **Satoshi's spoken intro card** (`hud_ui::draw_satoshi_brief`) — a warm "Satoshi — <title>"
  speech bubble that greets the player as the arena loads, then self-dismisses (or clears the
  moment they press H). Set in `game_loop::start_scenario` (challenges) and at race start
  (races), from `scenario::trial_satoshi`.

So **every** trial now carries: a drop-down description, clear how-to guidance, an in-game
pop-up, and Satoshi dialogue + a hint.

## Design principles (what makes a guided trial fun for kids ~8–13)

Synthesised from Minecraft community map design (parkour, build battle, escape rooms, redstone
puzzles), flow/level-design theory, and onboarding research:

1. One **visible** goal you can see, not read.
2. A reward **within seconds** of acting (a lamp lights, a wall slides, a cart rolls, a BOOM).
3. Teach **by doing** in a safe sandbox — no text walls.
4. Escalate in small, honest steps; gate progress behind the skill just taught.
5. A strong **theme** frames every prop ("light the lab", "hidden door", "raiders at the gate").
6. Pace as **tension → breather** waves; make failure cheap.
7. Leave room for the player's own solution; hide a little surprise.
8. End on a **payoff** worth showing off.
9. Respect the player — no baby-talk, no hand-holding past the lesson.

Anti-patterns avoided: walls of text, repeating one input, one "correct" solution, far
checkpoints, invisible feedback, patronising tone.

> **Follow-up (v0.2.5–0.2.6, 2026-06-25):** a skeptical second-pass review fixed
> two trials whose objectives couldn't complete in-engine (the build tests cover
> parse/resolve/`on_event`, not the event WIRING): **The Plumber** (emptying a
> bucket didn't fire `UseBucket` — fixed so it does, water + lava) and **Bring
> the Wall Down** (a lone brigand wasn't penned → ordered sequence soft-locked;
> now a Checklist). Then added **4 new challenges** for the cleanly-buildable
> coverage gaps: **The Champion** (boss combat vs a berserker), **Power Plant**
> (light a lamp from a generator, no lever), **Forged in Fire** (lava + water →
> obsidian), **Colour Lab** (mix dyes + paint wallpaper). The dye trial needed
> the 16 dye names registered in `give::material_by_name` (also enables `/give
> red_dye`). Then added **The Scavenger** (v0.2.7, owner request): a **3-minute timed run
scored by inventory VARIETY** — survival, from nothing, grab or craft as many
DIFFERENT things as you can (one of each kind counts; quantity doesn't). This
introduced a new `Scoring::InventoryVariety` mode — CUMULATIVE: each tick the
player's current item-kind keys (`Inventory::item_kind_keys`) fold into a run-long
"ever held" set and the score is its size, so it only ever GOES UP (using an item
up or crafting it away never lowers it). Shown on the timed badge as "🎒 N different"
and on the end-card as "N different things collected!", and a rule that a
timed-objective Challenge clears the inventory on launch so it always "starts
with nothing" (every replay, since Reuse arenas wipe + regenerate).
>
> Then a **Wacky pack** (v0.2.8, owner: "think wacky, nothing too edgy, full kits
> so they hit the ground running, normal scoring, all about fun") — 5 silly,
> generously-kitted challenges: **The Floor Is Lava!** (creative — the whole floor
> is a lava sea, build islands across; needed `lava` registered as a placeable
> block in `give`, mirroring `water`), **Kaboomtown** (creative — stack 16 kegs and
> chain-detonate a tower), **Monster Mash** (survival — bonk a 6-brigand "mosh pit"
> with sword + bow + a pile of bread), **Funny Farm** (survival — a tame/breed/ride/
> shear menagerie checklist), and **Splash Zone** (creative — pour water buckets to
> flood a walled pool into a water park). All built from proven events + valid
> names. Total is now **36 challenges + 3 races**. Remaining coverage gaps
> (beds/sleep, eggs/honey, schematic build-along, Rig Studio, cart depots,
> auctions/bazaar, gated movement) need NEW engine events before they can be
> gated — a separate instrumentation pass.

## The catalogue — 30 challenges + 3 races

Each challenge ships with the **right kit + a small pre-built arena** (blocks + mobs placed at
spawn offsets) so it's completable **solo** in a fresh arena. Engineering/building trials run
in **creative** (free blocks + flight to experiment on a flat canvas); survival / combat /
farming / gathering trials run in **survival** (so digging, crafting and hunger matter). The
objective is built only from the engine's **detectable events**, using ordered **Sequences**
(gather → craft → use) and unordered **Checklists** for variety.

### ⚡ Races (movement) — `trials.rs::CATALOG`

| id | name | covers |
|----|------|--------|
| `sprint` | The Sprint | reaction, flat-out speed (60 blocks) |
| `cross-country` | Cross Country | terrain navigation, route-finding (300 blocks) |
| `marathon` | Marathon | endurance, one clean line (~1000 blocks) |

### 🎯 Challenges

| id | title | drive | mode | world | objective |
|----|-------|-------|------|-------|-----------|
| `mine` | Quarry Run | MAKE | survival | normal | BreakBlock×20 |
| `craft` | First Workbench | MAKE | survival | normal | BreakBlock×3 → CraftItem → PlaceBlock → CraftItem×2 |
| `smelt` | The Forge | MAKE | survival | normal | PlaceBlock → SmeltItem → CraftItem |
| `cook-three` | Camp Cook | MAKE | survival | normal | CookAtCampfire×3 |
| `build` | Skybox Studio | MAKE | creative | flat | PlaceBlock×30 |
| `power` | Light the Lab | MAKE | creative | flat | PlaceBlock×5 → PowerDevice |
| `logic-gate` | Sensor Vault | MAKE | creative | flat | PlaceBlock×4 → PowerDevice |
| `piston` | Hidden Door | MAKE | creative | flat | PlaceBlock×3 → UsePiston |
| `rail-rider` | Rail Rider | MAKE | creative | flat | PlaceBlock×3 → RideEntity |
| `booby-trap` | Booby Trap | MAKE | creative | flat | PlaceBlock×3 → Detonate |
| `bucket` | The Plumber | MAKE | survival | normal | UseBucket×2 |
| `workshop-publish` | Open Your Workshop | MAKE | creative | flat | CraftItem → WorkshopPublish |
| `kill` | Raiders at the Gate | BRAVE | survival | normal | KillMob×3 |
| `armour-up` | Suit Up | BRAVE | survival | normal | SmeltItem + CraftItem×4 + KillMob×2 (checklist) |
| `breach-and-clear` | Bring the Wall Down | BRAVE | survival | normal | Detonate → KillMob |
| `eat` | Campfire Cook | BRAVE | survival | normal | CookAtCampfire → EatFood |
| `harvest` | Green Thumb | CRACK | survival | normal | HarvestCrop×4 |
| `farmhand` | Barn Chores | CRACK | survival | normal | ShearOrMilk×2 |
| `rancher` | Two of Every Kind | CRACK | survival | normal | BreedAnimals |
| `tame-wolf` | Best Friend | CRACK | survival | normal | TameMob |
| `fish` | Master Angler | CRACK | survival | normal | CatchFish×2 |
| `ride` | Saddle Up | CRACK | survival | normal | RideEntity |
| `fish-feast` | Catch & Cook | CRACK | survival | normal | CatchFish → CookAtCampfire → EatFood |
| `vendor-sale` | Market Day | ECONOMY | survival | normal | VendorSale |
| `claim-plot` | Stake Your Claim | ECONOMY | survival | normal | ClaimPlot → PlaceBlock |
| `onboarding` | First Light | EXPLORE | survival | normal | BreakBlock×4 → CraftItem → PlaceBlock×6 → CookAtCampfire |

## Coverage matrix — every game aspect has a trial

| Aspect | Trial(s) |
|--------|----------|
| Movement / traversal | sprint, cross-country, marathon |
| Mining | mine, craft (chop), onboarding |
| Building / placement | build, claim-plot, onboarding |
| Crafting | craft, smelt, armour-up, onboarding |
| Smelting | smelt, armour-up |
| Cooking | cook-three, eat, fish-feast, onboarding |
| Electricity / wiring | power |
| Logic / sensors / automation | logic-gate |
| Pistons / mechanisms | piston |
| Rails + carts | rail-rider |
| Booby traps | booby-trap |
| Explosives | booby-trap, breach-and-clear |
| Combat | kill, armour-up, breach-and-clear |
| Armour | armour-up |
| Farming / crops / bonemeal | harvest |
| Animals — shear/milk | farmhand |
| Breeding | rancher |
| Taming | tame-wolf |
| Fishing | fish, fish-feast |
| Riding | ride, rail-rider |
| Buckets / fluids | bucket |
| Hunger / eating | eat, fish-feast |
| Plots / land | claim-plot |
| Vendor / trade | vendor-sale |
| Workshop / reskins | workshop-publish |
| First-session onboarding | onboarding |

## Technical notes (what keeps these robust)

- **Events only.** Objectives are built from the 21 detectable `ChallengeEvent`s, all already
  firing in `game_loop`. `BreakBlock`/`PlaceBlock` filters are left unfiltered (any block) so a
  trial never depends on a brittle block-id match.
- **Arena = additive only.** `apply_arena_setup` *sets* blocks; it cannot carve air/negative
  space. So pure-traversal "climb / swim / bridge" challenges (which need a real gap and can't
  be gated on a position event) are intentionally **not** challenges — movement is owned by the
  ⚡ races, whose finish-beacon win condition *is* position-gated.
- **No "kill-during-prep" deadlock.** An ordered Sequence ending in `KillMob` can soft-lock if
  the mob is killed in self-defence before the prep steps complete. `armour-up` is therefore a
  **Checklist** (order-independent); `breach-and-clear` is safe as a Sequence because the stone
  wall pens the brigand until the keg blows it open.
- **Cooking needs fuel + ignition.** A placed campfire is unlit with 0 fuel, so cooking trials
  ship `flint_and_steel` + sticks (fuel).
- **`mobs_enabled: false` everywhere.** It gates only *ambient* spawning; arena mobs spawn
  directly (with full AI), so the arenas stay clean and controlled.
- **Vendor barter wants `stone`.** Mining stone drops *cobblestone*, so `vendor-sale` gives the
  player `stone` to trade rather than asking them to mine it.
- **Farming guaranteed.** `harvest` pre-grows four `wheat_mature` crops (instant, robust
  `HarvestCrop`) plus a hoe/seeds/bonemeal kit so the kid also learns the full grow loop.
- **Armour is expensive.** A full iron set is ~24 ingots, so `armour-up` reframes to "four
  pieces of iron **gear**" (armour or iron tools) and gives generous iron + sticks so the four
  crafts always land.

## Compliance

No trial text (tagline / how-to / Satoshi intro+hint) may frame play as money/earning. Trading
is a "swap / barter / trade", never "buy / sell for money". Enforced by the
`trial_text_has_no_money_or_earning_words` test (whole-word scan over the whole Trials corpus,
including the races). Mirrors `satoshi.rs`'s corpus guard.

## Tests (the safety net)

In `scenario.rs::tests`: every challenge parses + round-trips; every kit/arena/mob name
resolves (a typo ships a broken kit); every challenge has a tagline + how-to; every trial
(challenge **and** race) has a Satoshi intro + hint; the onboarding arc completes when driven
in order; the no-money-words corpus scan. `cargo test --bin axenstax-engine` (via `check.sh`)
is the gate.

## Deliberately dropped (and why)

- `climb-the-tower`, `deep-crossing`, `sky-bridge` — movement challenges whose only gate would
  be an unfiltered Break/Place (trivially completed without doing the climb/swim/bridge) and
  which need carved negative space the arena can't make. Movement is covered by the races.
- `logic-lock` — a pre-wired AND-gate puzzle too finicky to ship without a live playtest;
  redundant with `power` + `logic-gate`.
- `boss-stand` — a 5-step night boss fight (berserker) where prep-under-fire is unfair/fragile
  for a first-timer; its coverage is redundant.
- `explore-gallery-walk` — a `BreakBlock` objective in **adventure** mode (which forbids
  breaking) → can never complete. Exploration is covered by the standalone **Gallery**
  experience.

## Deferred / future

- Position-gated objectives (reach-a-place, stand-on) would unlock real parkour/escape-room
  trials and let the dropped traversal trials return.
- Block-filtered Break/Place would let multi-step builds gate on the *specific* block (e.g. the
  door vs the wall in `build`).
- Satoshi could give trials directly in the Test Lab flow (today he hands out test-board
  missions there); unifying the two is a later pass.
- Per-trial "show-off" capture (screenshot the finished build/contraption) for the Showcase IA.

## Update 2026-06-25 — lobby IA + pause + onboarding fixes (from native playtest)

Owner playtest feedback (`docs/test-sheets/2026-06-25-native-playtest-bugs.md`) drove
three changes to the trials experience:

- **One ordered Trials list, no Races/Challenges split.** The lobby Trials column no
  longer renders separate "Races" and "Challenges" sections. It now renders a single
  progression — easiest/first-session at the top, deepest systems at the bottom —
  interleaving Races and Challenges. Source of truth is `trials::TRIAL_ORDER`
  (a `&[(TrialType, TrialRef)]`), guarded by `trial_order_covers_every_trial_once`.
- **Five trial styles, each with an icon.** `trials::TrialType` = 🏁 Race · 🔨 Make ·
  ⚡ Tech · ⚔ Brave · 🌱 Tend. Every row shows `{icon} {name}`; the expanded body
  opens with `{icon} {Type} trial` so the description names the style. (Supersedes the
  old MAKE/BRAVE/CRACK/ECONOMY/EXPLORE drive labels for the *lobby UI*; those remain a
  fine internal coverage vocabulary.)
- **Trials can't be saved.** The in-trial pause menu (Esc) collapses to **Resume /
  ↻ Try Again / ← Leave** — no Save, Save & Quit, difficulty or Switch-to-Creative
  (`menu::draw_pause_menu(is_trial=true)`). A trial is ephemeral + retryable, so
  there's nothing to save; `game_loop` detects a live trial via `active_trial` or
  `scenario.kind == Challenge`.
- **`craft` ("First Workbench") no longer pre-gives the bench.** Its `kit` was
  `[crafting_table]`, which let the player skip crafting the workbench the trial is
  named for. Kit is now empty: chop the oak tree (the arena's 3 logs) → craft planks →
  craft + place a workbench → craft tools. The loose `Sequence` (break×3 → craft → place
  → craft×2) still completes because non-matching events don't reset step progress.

### Clarity pass (2026-06-25, second batch)

A follow-up playtest drove a "what to do is clearer" pass:

- **Colour icons.** `TrialType::color_rgb()` gives each style a signature colour
  (egui renders emoji as single-colour glyphs, so the icon is *tinted*, not
  multi-colour art). Lobby rows render `arrow + coloured icon + title` via a
  `LayoutJob`; the popup shows the same icon large (size 40) + coloured.
- **"What to do" popup = task list, not prose.** `scenario::objective_tasks(def)`
  derives the list straight from the objective — **Sequence → numbered**, **Checklist
  → bullets**, Action → one line, Timed/FreeRoam → a one-liner. Labels come from
  `event_task_label(event, count)` (e.g. "Break 3 blocks", "Defeat 3 enemies") and are
  deliberately **control-free**. The popup no longer shows the `how_to` prose.
- **In-game = the specific "how".** The detailed, control-specific `how_to` prose
  stays in the in-game objective panel (H), which now **auto-opens** at trial start
  (`show_objective = true` in `start_trial` / `start_scenario`).
- **Recipe on the inventory's right.** `scenario::trial_recipe_hints(token)` lists the
  items a crafting trial wants made; on opening the inventory during such a trial (and
  only if nothing is already pinned), the engine resolves the first hint via
  `give::resolve_item` → `crafting_catalogue::recipe_index_for_output` and pins it into
  `crafting_ui.pinned_recipe_stack` — so the recipe shows in the existing right-side
  placement card with no recipe book. The popup footer names those recipes.
  Tests guard that every hint resolves to a real catalogue card and every challenge
  yields a non-empty task list.

### Communication-clarity pass (2026-06-25, third batch)

Playtest: the auto-derived task lists were accurate but too generic ("Place 1 block"
for *place the furnace*), the in-game help still showed prose, and a race showed a
ticking clock with **no distance** ("how many of the 60 blocks have I run?"). Fix —
make *goal · steps · where am I · what's next* clear across every channel:

- **Authored task labels for all 40 trials** — `scenario::trial_task_labels(token)`,
  one clear, specific label per objective step ("Chop the oak tree → Build a workbench
  → Make a pickaxe & sword"). `objective_tasks(def, authored)` prefers them, auto-derive
  is the fallback. `authored_task_labels_align_with_objective_steps` (test) guards the
  count matches the objective's steps for every trial.
- **Per-step live progress** — `ScenarioState::step_status()` → `(done, total)` per
  step, and `current_step()` → first incomplete. Drives every readout below.
- **In-game H panel = the task list**, not prose: a `✓ done / ➜ now / • next`
  checklist (numbered when ordered) with a `(2/3)` count on multi-count steps, plus
  Satoshi's tip. Auto-opens at trial start.
- **Top badge** names the current step for stepped trials:
  `🎯 First Workbench · 1/4 · Build a workbench` (Action trials keep `12 / 20`).
- **Race distance** — `draw_trial_clock` now shows a progress bar + `38 / 60 blocks`
  (computed by `game_loop::race_distance` from the start→finish line), fixing the
  "counter just kept counting" bug.
- **Step-completion nudges** — `fire_challenge` fires a green toast
  `✓ Step done! Next: …` whenever a Sequence/Checklist step finishes (single-action
  trials are excluded so they don't spam).

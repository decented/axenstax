# The Challenge Engine — fun-first, coverage-harvested, made to scale

**Date:** 2026-06-23
**Status:** VISION + ARCHITECTURE (write-up first, owner-requested). Not built.
**Supersedes the framing of:** `2026-06-23-satoshi-test-lab-missions-spec.md`
(the Test Lab was the seed; this generalises it and **removes the "test"
framing** entirely — see §6).

Owner brief (2026-06-23, paraphrased): don't limit to build or speed; boil the
player types down to a small **core of motivations**; build a *system* that can
create **loads** of challenges (not a hand-authored few), ideally lettings
**others create them** too; kids pick a challenge once they see what it is; and
rethink where this lives in the app — **get rid of the "test" column**, give
**experiences** their own discovery flow, and give **challenges** their own
column with a **better name than "Lab."**

Builds on the validated signal: **Hash Dash got Axo the most engaged** of
anything we've shipped — un-coerced. Speed works. ([[project_play_modes_shipped]],
the `scenario` system, and [[feedback_axolittle_coerced_not_fun_signal]]: the
only real fun-signal is unprompted replay, which Hash Dash got.)

---

## 1. Thesis

**The game is the test harness.** The best QA is players who can't stop playing;
every run fuzzes the engine. So we never *ask* a kid to test — we make genuinely
fun challenges that, as a by-product, exercise the systems we need exercised,
and we **harvest** the signal (completions, crashes, timings, frame-time, which
features got touched) rather than demand a verdict. Testing becomes exhaust, not
homework. This is what stops it burning Axo out and lets us pull the **fun
lever** to bring other kids in — most of whom don't care about sovereignty yet
because they haven't been burned. Fun is the acquisition lever; **ownership** is
the retention lever; **sovereignty** is the meaning that reveals itself once
they're invested (the fun-floor / meaning-ceiling of
[[project_genesis_founding_myth_moonshot]]).

## 2. The core motivations — four drives + one connector

The earlier list (7+ motivations) was too many. Boiled to the core: **four
things a kid wants to DO, and one thing they want from each other.** Each "doing"
drive maps to a challenge family, is legible as a one-word tag a kid instantly
gets, and — not by coincidence — exercises a distinct slice of the engine (so
designing for player variety *is* designing for coverage variety).

| Drive | "I want to…" | Challenge family (tag) | Engine it exercises |
|---|---|---|---|
| **MASTER** | get good, go fast, win | ⚡ **Race** — time trials, skill runs, beat-your-ghost | movement, physics, collision, fixed worlds |
| **MAKE** | build something that's *mine* | 🔨 **Build** — themed prompts, constraints | placement, schematics, build-along, blocks |
| **BRAVE** | get a thrill, survive, cause chaos | ⚔️ **Survive** — hordes, demolition, obstacle gauntlets | combat, mobs, falling blocks, explosions, **load/perf** |
| **CRACK** | figure it out, discover | 🧩 **Solve** — puzzles, escapes, mazes, logic rooms | electricity/logic, traversal, the feature-under-test |

**The connector — SHOW ("be seen / belong").** This is *not* a fifth family;
it's the **engagement layer** that sits on top of all four: share it, vote on
it, chase someone's ghost, get featured. Recognising SHOW as *orthogonal* is
what keeps the type-list short **and** answers "people find fun differently"
without multiplying challenge types. It's also the growth + UGC engine, because
it lets **one kid's MAKE become another kid's BRAVE or CRACK** — your booby trap
is my obstacle; your puzzle-house is my escape room. Creations become content.

> Why four and not three: BRAVE (twitch/thrill) and CRACK (think/solve) are
> genuinely different kids; collapsing them loses the chaos-fun that doubles as
> our load test. Why not more: anything else (collect, explore, compete) is a
> *flavour* of one of these four, or lives in the SHOW layer.

## 3. Challenges are tagged; kids choose

Every challenge carries its family tag (⚡/🔨/⚔️/🧩) + difficulty + optional
SHOW affordances (shareable? votable? has a ghost?). The browser filters by tag,
so a kid self-selects by mood ("I want to race" / "I want to build"). No one is
*assigned* anything. If a feature isn't getting exercised, that means its
challenges aren't fun enough — itself a useful signal (interesting ≠ fun,
[[feedback_pop_educational_not_earning_now]]).

## 4. The creation system — loads, not a few

The owner's hard requirement: a *system* that produces **loads** of challenges,
not a hand-authored handful. The substrate already exists — **`ScenarioDef`**
(`game/engine/src/scenario.rs`) is data-driven JSON with `kit` (inventory),
`objective` (incl. **`Timed`**), `arena_seed` (fixed fair map), `scoring`,
`lock_creative`, and world-shape overrides. We extend it minimally and layer a
factory on top:

- **`ChallengeDef` = `ScenarioDef` + `{ family, difficulty, judging, author,
  share }`.** `family` is the tag; `judging` is how a result is recognised
  (best-time / completion / **vote** / "got me"); `author` + `share` carry the
  npub + discovery info for UGC.
- **Archetype templates (the scale lever).** Each family has a few
  parameterised recipes; a recipe + params = a concrete `ChallengeDef`:
  - ⚡ Race: `{arena_seed | layout, start, finish, par_time, kit}` → vary seed +
    par → **infinite fair maps, near-free.**
  - ⚔️ Survive: `{arena, threat (waves/timer/avalanche), win}` → vary threat
    intensity → many; the chaos ones *are* the load test.
  - 🧩 Solve: `{authored layout (schematic/.axeworld), win-condition}`.
  - 🔨 Build: `{prompt, kit, constraints, judging = vote}` — cheapest to author
    (mostly text + a kit), richest in SHOW.
- **Procedural where cheap, authored where it matters.** Race/Survive lean on
  seeds (loads, free); Build/Solve lean on prompts/layouts (authored, fewer but
  deeper). I can author the bundled launch set as JSON files; a small
  templating pass stamps variants.
- **A catalogue + filters** (extends the existing recipe/scenario catalogue
  pattern, cf. [[project_recipe_catalogue_and_book]]) holds the bundled set and
  merges in remote/UGC challenges, browsable by family.

## 5. UGC — players make them (designed-for, build deferred)

The real scale engine is **players authoring challenges**, which is itself a
MAKE challenge ("design a challenge" → publish). We have every piece:
`.axeworld` interchange + schematics (capture a layout), the **Workshop** (an
authoring space), the **stash** (self-custody), **Beacon** (discovery/relay).
The flow: build/capture an environment → pick a win-condition + family tag →
publish to your stash + announce on Beacon → others discover and play. The
"author a challenge" in-game UX is **deferred** (owner: "leave that for later"),
but the format (`ChallengeDef` JSON) and transports are designed for it now so
nothing has to be re-cut later.

## 6. App / lobby IA — where it lives ("how we put it in")

Three player-facing homes; the **"test" framing disappears**:

1. **Experiences** — *whole worlds / games you enter.* The current three (Hash
   Dash, Satori Rush, Gallery) are the seed set. Given their own **discovery
   flow**, not a fixed column: a **"Find an Experience"** action — paste an
   **npub** or browse the **AxeNStax directory** — that pulls published
   experiences from **Beacon** (and your stash for ones you've made/saved), then
   you download + enter. Mirrors "get it from a stash," labelled **Experience**.
2. **Trials** *(working name — see naming below)* — *the challenge engine.* This
   **replaces the current games/experiences column AND absorbs the Test Board
   column** — "get rid of the test." A browsable, tag-filtered list (⚡🔨⚔️🧩) of
   bite-size challenges you play in place. Coverage is harvested invisibly
   underneath; the old Community Test Board becomes an **internal dev coverage
   dashboard**, no longer shown to kids.
3. **Showcase** — *the SHOW payoff.* Winners' gallery: top times/ghosts,
   most-voted builds, "your booby trap got 12 people." Reuses the delivered
   **Exhibit/Showcase** primitive ([[project_creator_gallery_delivered]]).

**Naming** (owner: "better name than Lab"). Recommend **"Trials"** — fits the
worthiness-not-riches moonshot theme, kid-legible, and avoids clashing with the
existing in-game *quests* and *challenge board*. Alternatives: "Proving Grounds",
"The Gauntlet", "Arcade", plain "Challenges". **Owner's call** — flagged in the
summary.

## 7. The SHOW layer without chat (and the safety stance)

- **Engagement is structured, never written:** chase a ghost (race a recording —
  async, no live netcode, no comms), or cast a **structured vote** (heart /
  top-3 / "😂 got me" / "🛠 didn't fire"). No free text ⇒ **no moderation
  surface, ever.**
- **Fair tallies reuse the test board's exact maths** — count *distinct* npubs,
  not raw taps (`test_board::board_status` already does this for good/broken). So
  "most-loved build" and "feature works" are the same code.
- **Safety stance (owner 2026-06-23, "no target on our head"):** at alpha, with
  a handful of known kids, we do **not** gold-plate moderation. Crucially, the
  no-chat / structured-vote design means we're **safe by construction** — so
  this is *not* trading safety for speed; it still holds at scale. Anything that
  leaves the device (ghosts, builds, votes) stays **opt-in + self-custodied**,
  never an accidental broadcast (privacy posture from
  [[project_uk_compliance_landscape]] / [[project_identity_default_nonpublic]]),
  but we build *zero* extra moderation tooling now.

## 8. Sovereignty, felt not told

A kid whose ghost three friends are chasing, whose booby trap "got" 20 people,
whose challenge others play — owns a reputation made of **things in their own
stash: signed, portable across servers, un-deletable by any operator.** They
feel ownership before they have the word. The challenge engine is the vehicle;
we never lecture it.

## 9. Honest guardrails

- **Validate ONE loop before the cathedral.** Metric = unprompted replay /
  share, not "challenges completed."
- **Leaderboards are forgeable** (sybil votes, faked times) — known hard problem
  ([[project_identity_default_nonpublic]]). Ghost-as-*fun* (race a visual
  replay) is cheap and fine; ghost/score-as-*truth* needs anti-cheat — **defer**,
  don't gate the fun on it.
- **Timing is a mode, not a mandate** — same challenge, "relaxed" or "timed" —
  respecting kids who find the clock stressful, not motivating.
- **Telemetry is privacy-careful** — aggregate, no-PII, opt-in for anything that
  leaves the device.

## 10. Roadmap

- **Phase 1 — prove the loop (smallest fun thing).** A ⚡ Race trial on the
  existing `ScenarioDef`: fixed Adventure arena, par-time, **personal best +
  your-own-ghost**. No sharing, no votes, no verdict. Ship it; watch one
  non-Axo kid. If they replay to shave a second, the thesis holds.
  - **P1 COMPLETE 2026-06-23** (origin/main `df66efcc`): `/trial [list|<id>|off]`
    launches a Race — teleports you to the start, plants a bright finish beacon,
    times the run, saves a **personal best that persists per-device** (native
    `profile/trials.json` / web localStorage) + across worlds, with a race-clock
    HUD. **And you chase your ghost** — a translucent cyan wireframe runner
    (dedicated `trial_ghost` render channel) replayed at your best run's
    transform sampled at your current elapsed time. **Full suite: 9 escalating
    Race trials** (First Steps 30 → Marathon ~1000), surface-snapped to work in
    any world. Next: richer families (Build / Survive / Solve) per §2 + the SHOW
    rail (share / vote / chase others' ghosts) per Phase 2.
  - **P1 CORE LANDED 2026-06-23** (`game/engine/src/trials.rs`, tested + green):
    the pure primitives — `GhostRecording` (record/sample, holds on last frame),
    `TrialBests` (fastest-only + JSON round-trip), `RaceRun` (counts up between
    start/finish, records a ghost frame per tick), `in_volume` start/finish
    trigger, `format_time`. **Remaining for the playable loop:** a race objective
    on `scenario` (start→timer→finish via `in_volume`), the local PB store
    (native file / web), the **ghost-avatar render** (replay `sample(elapsed)` as
    a translucent avatar), a couple of bundled Race trial defs, and a minimal
    Trials launch. Name locked: **Trials**.
  - **GHOST SHARE BUILT 2026-07-05** (deferred-lists wave): the *smallest*
    share primitive, deliberately FILE-based and rail-free — "Export ghost" /
    "Import ghost" on each lobby race row writes/reads a versioned `.axeghost`
    JSON file (`trials::GhostShareFile`; native rfd dialogs, web
    download/file-picker bridges). The import becomes that race's **rival**
    (`TrialBests.rivals`, one per race, persisted beside the bests) and races
    alongside your PB — PB cyan, rival ORANGE — with a finish-panel verdict
    ("You beat <label>'s ghost!"). No relay, no upload, nothing collected;
    the exporter writes `author: None` by default (caption is untrusted free
    text, shown as "Friend" when absent). This neither builds nor pre-empts
    the Phase-2 publish/vote rail decision — the file format stands alone and
    any future rail can carry the same `GhostShareFile`.
- **Phase 2 — the SHOW rail + first social family.** Publish-an-artifact pipe
  (ghost → race; build → vote), `ChallengeDef` + tags + the **Trials** browser,
  one 🔨 Build challenge with structured voting + a Showcase. Fold the Test
  Board column into Trials; the dev coverage dashboard goes internal.
- **Phase 3 — the factory + breadth.** Archetype templates per family; author
  *loads* of bundled challenges (seeded Race/Survive + authored Build/Solve);
  tag filters.
- **Phase 4 — Experiences discovery.** "Find an Experience" via npub / AxeNStax
  directory / Beacon; download + enter.
- **Phase 5 — UGC (deferred).** In-game "author a challenge" → publish to
  stash/Beacon; "create a challenge" as a MAKE meta-challenge.

## 10a. Trials must test ALL of the engine — the coverage matrix

Owner correction (2026-06-23): *"they need to test all aspects of the engine and
gameplay. no trial should be too similar."* A trial is only worth its slot if it
exercises a system **no other trial does**. So the suite is built as a coverage
matrix, not a pile of near-identical races. A trial can verify a system only if
the engine **detects** the player doing it — today that's the `scenario`
`ChallengeEvent` palette + the Race mechanic.

**Covered now** (each a distinct trial; live):

| System | Trial | Detection |
|---|---|---|
| Movement (speed) | ⚡ `sprint` | Race finish |
| Movement (terrain nav) | ⚡ `cross-country` | Race finish |
| Movement (endurance/route) | ⚡ `marathon` | Race finish |
| Mining / digging | `mine` | `BreakBlock` |
| Building / placing | `build` | `PlaceBlock` |
| Crafting | `craft` | `CraftItem` |
| Cooking | `cook-three` | `CookAtCampfire` |
| Taming | `tame-wolf` | `TameMob` |
| Trading | `vendor-sale` | `VendorSale` |
| Plot claiming | `claim-plot` | `ClaimPlot` |
| Workshop authoring | `workshop-publish` | `WorkshopPublish` |
| First-session flow | `onboarding` | ordered `Sequence` |
| Genesis mining | `satori-rush` | `FirstSatori` |

**Detection extension DONE 2026-06-24** — 12 new `ChallengeEvent` variants +
fire-hooks + a trial each, so these systems are now covered too:

| System | Trial | Event (fire site) |
|---|---|---|
| Combat | `kill` | `KillMob` (kill_counter increment) |
| Eating / hunger | `eat` | `EatFood` (try_eat_hotbar) |
| Farming / harvest | `harvest` | `HarvestCrop` (break a crop block) |
| Smelting | `smelt` | `SmeltItem` (take furnace output) |
| Fishing | `fish` | `CatchFish` (reel a hooked line) |
| Mounts / carts | `ride` | `RideEntity` (`riding = Some`) |
| Electricity | `power` | `PowerDevice` (a lamp lit in `power_tick`) |
| Pistons | `piston` | `UsePiston` (`tick_pistons` moved one) |
| Explosives | `detonate` | `Detonate` (keg fuse hit 0) |
| Buckets | `bucket` | `UseBucket` (bucket fill) |
| Shear / milk | `farmhand` | `ShearOrMilk` (shear / milk success) |
| Breeding | `rancher` | `BreedAnimals` (`tick_breeding` bore a baby) |

Fired via the one-line `GameState::fire_challenge(ev)` helper. **Still uncovered**
(niche, later): bonemeal use · fluid flow reaching a cell · specific tool/armour
use. The pattern is set — each is one more event + fire + JSON.

## 11. What we reuse (almost nothing is new plumbing)

`ScenarioDef` (kit/objective/timer/arena/scoring) · `test_board` distinct-npub
tally (→ votes) + its column (→ Trials) · Exhibit/Showcase (→ Showcase) ·
`.axeworld` + schematics + Workshop (→ authored layouts + UGC) · stash + Beacon
(→ discovery/self-custody) · gamestr (→ times/leaderboards). The new code is the
`ChallengeDef` wrapper, the ghost recorder/replayer, the Trials browser, and the
publish/vote rail.

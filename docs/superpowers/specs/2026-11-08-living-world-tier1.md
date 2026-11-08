# Living World, Tier 1: bloom, the Feast, Commons, the Grey Quarry, the Tally

Status: BUILD SPEC, 2026-11-08. The owner ratified the living-world direction on 2026-11-08 and asked for Tier 0 and the whole
of Tier 1 in one playable build for Axolittle. Breadth over polish: every piece present and playable; tuning follows the
playtest.

Sources (internal repo, private): `docs/vision/living-world-long-run.md` §3, §4, §5, §9, §11, §14–§16, and
`docs/vision/trials-two-acts.md` Part B. This spec is the public, as-built-intent version. When a slice lands, the relevant
numbered spec (03, 05, 06) gets the as-built text too.

## 0. Decisions taken in this build (owner said: take the call, write it down)

| # | Call | Taken | Why |
|---|---|---|---|
| D1 | Names | Player-facing verb is **bloom** ("make it bloom"); the word never appears as a number. Code module `bloom`. Adult figures: **the Tally** and **world bloom**. | Vision §4 working names; one name in code and copy avoids drift. Rename is a find-and-replace later. |
| D2 | Work-meter sats path | **Retired** (spec edit only, done in Tier 0). | Vision §14 "retire or demote"; it was never built. |
| D3 | Beacon | **Kept** as the Plan/knowledge rail; nothing changes in this build. | Vision §10/§16 Q5 position. |
| D4 | `game_loop.rs` | **Not split now**; split before Tier 2. New code lives in new modules; `game_loop.rs` gets call sites only. | Vision §14: the split pays for itself before Tier 2. |
| D5 | Proof of Play layer 1 | **Not built.** CLAUDE.md now says what is built (Tier 0). | Not in the owner's scope list. |
| D6 | Bloom is **presentation, computed on each client** from what that client can see | No wire packet, no WorldSave field for bloom. | Tier 1 bloom has no gameplay consequence (consequences are Tier 3). Every client already has the world, the mobs and the other players' positions. Small differences between clients are harmless. |
| D7 | Bloom state across reloads | The slow-moving **level** is runtime only; on load it starts at the **target** worked out from the world. | "No save change if possible." The place's state is real (blocks, mobs, markers), so a reloaded quarry is still grey. Feast and co-presence surges fade on reload; the Feast marker's lasting lift survives because the marker is a block. |
| D8 | New blocks | Feast Table, Feast Marker, Commons Stone: three new block ids (373–375). `SAVE_LAYOUT_REVISION` 4→5 (save version 60→61). `PROTOCOL_VERSION` 96→97. No new WorldSave field. | New ids need the layout bump so older builds refuse the world instead of reading air. The protocol bump stops an old client drawing them as air. |
| D9 | Texture budget (511/512) | Feast Table takes the last free layer (the floor test is `texture_count() <= 512`; the builder confirms `texture_count()` is 511 before using it, else the table reuses a layer too). Feast Marker and Commons Stone **reuse existing layers** (builder picks distinctive ones, e.g. the marker reuses the table texture; the stone reuses a mossy/flowered stone). No change to the 512 floor. | Raising 512 means editing website files; the owner said leave the website alone. |
| D10 | Commons at Tier 1 | A Commons Stone declares the same 33×33 footprint as a Plot. Anyone may build there (as on open land). **Break-protection of other people's work waits for block authorship (Tier 2).** A Commons and a Plot may not overlap, either way round. The Stone can be broken by anyone. | "Nobody may break another's work" needs authorship, which is Tier 2. Without it, Tier 1 Commons is a declaration plus a bloom lift. |
| D11 | Feast guests | Guests = other players present within 48 blocks + villagers within 32 blocks (villagers capped at 12). The host is not a guest. No guests: nothing happens and a toast says the feast needs guests. | Vision B.6: "solo, the village's people are the guests". An empty-world feast with no-one to give to is honestly not a feast. |
| D12 | Feast name | "The Feast of <Name>", where Name is chosen deterministically from short word lists by the table's position (e.g. "Long Hollow", "Birch Rise"). | Vision example "the Feast of the Long Bridge". There is no place-naming system yet. |
| D13 | Act-two shelf | A sixth `TrialType`, label **"Bloom"** (placeholder, vision B.2). It appears (the Grey Quarry unlocks) **on the first time a place in the player's own world goes grey, or after Power Plant** (`generator`). | B.2 proposal. It also makes "dig out a big patch till it goes grey" a playtest of drain itself. |
| D14 | Trial modifiers + play-along code (B.4) | **Deferred.** | A Trials-wide feature, not needed for one playable act-two trial. |
| D15 | End card before/after pictures | Built **only if** `world_picture.rs` can render from a fixed camera cheaply; otherwise deferred and the card shows the flower and the sentence. | Breadth over polish. |
| D16 | Adult panel placement | Tally + world bloom replace the Deepslate Reserve panel's slot under **F3**, shown only when a new Settings switch **"Grown-up panel (F3)"** is on (default off). The sats part of the old gauge keeps its own sats gate. Dedicated servers add the same figures to `ConsoleSnapshot`; the console *site* is not changed. | Vision §9 says the adult numbers live on the learn site, docs site and server console, *never in the game*. The owner named the Reserve gauge seam, which is in-game F3, so **the owner's seam wins for Tier 1**; the §9 home (web/console pages only) is the later answer. A default-off switch is a soft boundary, enough for Tier 1 because the figures are about the world, never a person. |
| D17 | Satoshi's gardener lines | Added: the Grey Quarry intro, halfway and end lines (B.5 drafts), plus three ambient gardener lines that his greeting can use when the player stands in a grey or a blooming place. | Vision §8 and §14 Tier 0. |
| D18 | Grey Quarry kit | B.5 kit **plus a hoe** (sowing needs tilled soil). | Without a hoe the seeds route is blocked. |
| D19 | Scope of act-two trials | Grey Quarry only. The Feast trial, Seed the Island and After the Raid are next. | The owner's scope names the Grey Quarry. `HoldFeast` is added as an event now, so the Feast trial is data later. |

## 1. Bloom: what is worked out, and from what

### 1.1 Regions

A **region** is one chunk column: 16×16 blocks in x/z, all heights. Key `(cx, cz)`. A client keeps a `BloomField` with an
entry for every region within 8 chunks of any of its local players (about 289 regions). Entries further away are dropped.

### 1.2 The life term ("eco"), from blocks and mobs, every client, no save

For each of the region's 256 columns, take the highest non-air block (`World::highest_block`) and classify it:

| Class | Value | Blocks |
|---|---|---|
| LIVING | 1.0 | grass, tall grass, any flower (wild or farmed stage), any crop stage, any sapling, any leaves, water, bee hive, cactus/dead bush if they exist. Logs count as LIVING (stumps, trunks). |
| NATURAL | 0.8 | the surface that biome naturally has: sand in Desert/Beach; snow/ice in SnowyTundra/Taiga; stone/gravel in Mountains. The biome comes from `BiomeGenerator::biome_at(x,z)`, cached per region (the region's centre column is enough). |
| SOIL | 0.4 | dirt, tilled soil with nothing on it |
| BUILT | excluded | any non-living top block with the placed bit set (`World::is_placed`) |
| BARE | 0.0 | everything else: stone, cobble, gravel, deepslate and so on outside a biome where they are natural |

**Natural terrain is never BARE.** A natural ravine, cave mouth, cliff or rocky bank in an untouched world must not read grey.
As built (S1): the natural top of a column is found by **regenerating the region's chunk column into a throwaway twin of the
world** (`World::generation_twin` + `generate_column`, the `chunk_verdict` trick) and keeping each column's top height and
block (`bloom/natural.rs`, 256 entries per region, cached for the session). That is exact for every generator (v2, v3, v4,
flat) and includes trees, villages and ravines. Classification of a non-living, non-placed top block:
- it **matches the natural top** (same block at the same height), or is snow cover: NATURAL (0.8);
- otherwise dirt or tilled soil: SOIL (0.4);
- otherwise BARE (0).

The twin is needed only for a **touched** region, one with any section edited this session or loaded from a save
(`Chunk::persist`). A region fresh from worldgen is its own natural top, so every non-placed block there is NATURAL and no
twin is built. At most one regeneration per tick. Measured about 1 to 2 ms per column in a debug-profile test build for v3
(estimate well under 1 ms in release); a fully touched 289-region field fills in over about 15 s. The biome table is not
used. The calibration tests below gate S1: a touched natural-stone, desert, forest and mountain region reads the same as
untouched (within 0.02).

- `living_frac` = mean value over non-BUILT columns (if every column is BUILT, use 0.5).
- `cover = 1 − exp(−3 · living_frac) / normalised so that living_frac = 1 → 1.0` (i.e. `cover = (1 − e^{−3f}) / (1 − e^{−3})`). It rises fast: a place does not need every column planted.
- `kinds` = the number of distinct living kinds in the region:
  - each leaf species, log species and sapling species;
  - each crop family and each flower kind;
  - grass, tall grass and water, counting once each;
  - plus each distinct **passive** mob species standing in the region. Villagers and peddlers do not count: they are people. Fish, squid and similar count.
  - Kinds are collected by walking each column down from its top through non-solid and living blocks until the first solid non-living block, so a tree's leaves and trunk both count.
- `div = 1 − exp(−kinds / 4)`.
- **Monoculture:** if the region has ≥ 24 crop cells and one family is ≥ 85 % of them, `mono = 0.6`; otherwise `mono = 1`.
- **Walls everywhere:** `built_frac` = BUILT columns / 256. `wall = 1 − 1.5 · max(0, built_frac − 0.6)`.
- `eco = cover^0.7 · (0.5 + 0.5 · div) · wall · mono`, clamped to 0..1. (S1 moved the base from 0.45/0.55 to 0.5/0.5 so that
  a purely natural place with no green, such as bare desert or mountain rock, stays at or above 0.45 instead of 0.44.)

Intended feel. These bands are targets; S1's calibration tests pin them, and the formula's constants move to meet the tests,
not the other way round:
- Untouched plains or forest: about 0.8 (a forest measures 0.94 to 0.98; grass-only plains with no trees or animals 0.57;
  with a few animals about 0.75).
- Untouched desert, beach, mountain or ocean: about 0.5 (0.49 to 0.57; normal colour, see §2).
- A first mine, one chopped tree or a house: the region stays in the normal band.
- A stripped quarry floor: about 0.
- One sapling kind planted 40 times in a quarry: below 0.35 (measures 0.32).
- A quarry with three tree kinds, three crops, water and two animal kinds, on 64 of the 256 columns: 0.6 or more (measures 0.63).
- A monoculture (one crop family on 64 columns) falls to under 0.8 of the same ground with two families.

### 1.3 The people-together term, runtime only

Per region, three parts, each 0..1:

- **Co-presence `c`.** Each tick, count the distinct players whose position is inside the region or within 8 blocks of it.
  - "Players" means local split-screen seats plus every other player this client knows the position of.
  - If two or more are present: `c += (1 − c) · (n − 1) · dt / 180 s`.
  - Otherwise `c −= c · dt / 1800 s`.
  - In a Commons area the rise rate doubles.
- **Feast `f`.**
  - When a Feast happens in the region (a Feast Table turns into a Feast Marker, seen locally or through a block change), set `surge = max(surge, 1 − exp(−g/3))` with `g` = the marker's guest count. The surge fades: `surge −= surge · dt / 1200 s`.
  - Each Feast Marker in the region also gives a lasting floor of `0.15 · (1 − exp(−g/3))`, summed and capped at 0.3.
  - `f = max(surge, floor)`.
- **Commons `k`.** 0.1 if any Commons area overlaps the region, else 0.

Combined: `P = 1 − (1 − c)(1 − f)(1 − k)`. A solo player with no Feast and no Commons has `P = 0`. That is absent, never a penalty.

S1 inputs for slice S3: the field is fed `set_feast_markers(&[(x, z, guests)])` (the lasting floors, whole list replaced),
`set_commons(&[CommonsArea])` (whole list replaced; `CommonsArea::from_marker(x, z)` is the 33 x 33 footprint) and
`note_feast(x, z, guests)` (the surge, region and probes). "Present" players are the local seats plus `remote_players`;
every player counts in the four regions within 8 blocks of them. Co-presence is real time, never scaled.

### 1.4 Target, level and tempo

- `target = 1 − (1 − eco) · (1 − 0.7 · P)`. People can lift even a grey place, to at most 0.7.
- In **creative mode**: `target = max(target, 0.6)`. Creative is vivid and neutral.
- **Level** eases toward target, scaled by the world's `time_scale` (1.0 normally; trials set it):
  - falling: `level += (target − level) · dt · scale / τ_down`, with τ_down = 2400 s;
  - rising: the same with τ_up = 360 s.
  - "Slow drift, fast recovery." Newly tracked regions start at `level = target`.
- **Scan budget.** Four regions are re-scanned per tick, oldest first. A full sweep takes about 4 s at 20 TPS. The passive-mob census is one ECS pass every 20 ticks, bucketed by region.

S1 constants as built (`bloom/level.rs`): `TAU_DOWN` 2400 s, `TAU_UP` 360 s, people lift 0.7, creative floor 0.6, unknown
places read 0.6. Regions tracked: 289 round every local seat (remote players do not extend tracking). A region that is not
loaded costs no scan.

### 1.5 Probes (for trials)

`BloomField` also holds named **probes**: an axis-aligned box in world coordinates with its own target, level and terms,
computed over exactly the box's columns rather than region-aligned. The people-together part of a probe uses the same three
parts restricted to the box. Probes re-scan once per second. The trial `Place` objective reads a probe.
As built: `add_probe(id, min, max)`, `set_probe_goal(id, at_least, hold_secs)` (default 0.55 and 20 s),
`remove_probe`, `probe(id) -> Option<ProbeState { level, target, eco, people, stage, held_secs, scanned }>` (`scanned` is
false until the first scan). `set_initial_level(min, max, level)` applies to regions and probes overlapping the box, now or
when first scanned.

### 1.6 Stages (words, never numbers)

| Level | Phrase (default) |
|---|---|
| < 0.25 | "Grey and still" |
| < 0.45 | "Something's stirring" |
| < the trial's `at_least` | "Waking up" |
| ≥ `at_least` and held long enough | "Alive" |

### 1.7 "A place went grey" (unlock trigger)

In a non-trial world, the first time any tracked region's **level** falls below 0.25 while its `eco` is below 0.25, fire
`ChallengeEvent::PlaceWentGrey` once (through `fire_challenge`, so it records in `TrialBests.events_seen`). A world whose
regions start grey on load fires it too, after a 60-second grace period so that loading never counts as an event.
As built: reported by `BloomField::tick` and fired from `game_loop.rs`; never on a joined client, in a flat or Workshop world, or
while a scenario is running or about to launch (trial arenas). It is one-shot per session and, if suppressed, not used up.

### 1.8 Where it runs

- **Single-player, host and joiner clients:** `GameState` owns a `BloomField`, ticked from `GameState::tick`.
  - It is ticked on joiners too. It reads the client's own world copy and mob mirror.
  - It is not part of the sim-lend machinery: it is presentation and writes nothing to the world.
- **Dedicated server:** `GameServer` owns a `BloomField` round its present players, used only for the `ConsoleSnapshot` figures (§5).
- **Web build:** the same as single-player. No native-only code.

## 2. The colour, sound and life layer (presentation)

### 2.1 Colour

- `CameraUniform` gains `bloom_origin: [f32;4]` and `bloom_grid: [[f32;4]; 64]`. Each player viewport carries it, so split-screen works.
  - `bloom_origin` holds the grid origin x and z, the cell size (16) and an enabled flag.
  - `bloom_grid` is a 16×16 grid of region levels centred on the camera, packed four to a vec4.
  - Cells with no data are 0.6.
- **Shader:** at every lit output (next to `apply_user_brightness`), sample the grid bilinearly at the fragment's world x/z and grade the colour.
  - level ≥ 0.45: normal colour.
  - Below 0.45: desaturate toward luminance, down to 15 % saturation at level 0, with a slight cool grey cast.
  - Above 0.75: up to 15 % extra saturation and a slight warm lift.
  - Outside the grid, use the value at the camera.
- **Sky:** sky and fog horizon colour take the same grade, using the level at the camera.
- **Grass sway:** plant vertices (`fs_plant` path) sway gently, with amplitude scaled by the local level (none when grey). This needs a time value; add it to `bloom_origin.w` or a spare lane.

### 2.2 Sound

New procedural sounds in `audio.rs`, synthesised like the others, with no audio files:
- `Birdsong`: two or three short chirp variants;
- `Insects`: a soft cricket trill.

The ambient loop schedules one-shots round each local player:
- birdsong in daytime and insects at night;
- the interval is shortest at high level and none at all below 0.3.

Grey places are **still**: no birdsong.

### 2.3 Life

New particle emitters in `particles.rs`, both using `TEX_PARTICLE_SOFT`:
- **pollen**: daytime, slow drifting motes;
- **fireflies**: night, blinking yellow-green motes.

They are spawned from the existing ambient loop round each local player. The rate is scaled by the local level and is zero below 0.45.

## 3. The Feast

### 3.1 Blocks

- `FEAST_TABLE` (373): solid, wood-like. It takes the last free texture layer: a cloth-covered table top.
  - Recipe: 3 planks over 2 sticks, plus 1 bread.
  - Also in the creative inventory and as a `/give` alias.
- `FEAST_MARKER` (374): what the table becomes after the Feast. It reuses the table texture and drops a Feast Table when broken.
  - Its `block_meta` byte holds the guest count (0–255).
  - The guest count therefore reaches joiners through `BlockChange.meta` and late joiners through the B2a chunk push (which already carries `block_meta`), and it is saved with the existing block-meta store. The builder must check that no generic code reads the meta byte of an unknown block as orientation or water level.

### 3.2 Holding a Feast (shared rule, all modes)

Use (right-click) a Feast Table while holding a stack:

1. Find the guests (D11). With none: a toast, "A feast needs guests. Bring friends, or find a village." Nothing is used up.
2. The whole held stack is the goods. Split it evenly over the guests; any remainder goes to the first guests by distance.
3. Each player guest gets their share dropped as a real item at their feet.
4. Each villager guest's share is used up there. A confetti burst plays at the table (the client sees the block change).
5. The table becomes `FEAST_MARKER` with meta = guest count (players + villagers). The change is broadcast like any block change.
6. Fire `ChallengeEvent::HoldFeast` for the local player who held it. Joiners do not fire trial events.
7. A toast to the holder: "The Feast of <Name>, N guests."

Use a Feast Marker: a toast with its name and guest count.

**Guest plumbing (one interface, both paths).** The shared rule takes the guests from its **caller**:
`guests: &[Guest { kind: GuestKind::Player(slot or player id) | GuestKind::Villager(entity), pos }]`.
- The local path (single-player, and the host's own seats on a lent world) gathers them from `GameState.players`, the positions of remote players it knows and the ECS villagers.
- The authority (lent host serving a joiner, dedicated server) gathers them from `GameServer.players` (present and alive) plus its ECS.
- **The authority spawns the item drops for every player guest**, so a lent host and a dedicated server behave the same. The local single-player path spawns them in its own world.

**Mode wiring:** follow the Composter `block_use` pattern. It is one shared rule in `block_use.rs`:
- the local path in single-player and host;
- `send_block_use` and the authority's existing UseBlock handling for joiners.

A joiner's held stack is believed within the existing believed-deposit bound (`window_ops::believe_pay`), like other UseBlock payments. That is an accepted Tier 1 limit.

## 4. Commons

- `COMMONS_STONE` (375): solid, stone-like, reusing an existing texture (D9).
  - Recipe: 8 cobblestone round 1 sapling (any).
  - Creative inventory and a `/give` alias.
- **Footprint:** the same as a Plot, marker ±16 in x/z.
  - Commons areas are found by scanning loaded chunks for the block. Chunks keep a maintained "has commons" counter so the scan can skip, like `has_hoppers`.
  - They are not stored anywhere else.
- **Rules:**
  - Placing a Commons Stone whose footprint overlaps any Plot is refused: "This land is claimed." The same applies to a Plot marker whose footprint overlaps a Commons area.
  - These are enforced on the local path and on the authority (`remote_may_touch` / the plot-claim path).
  - Building inside a Commons area is open to everyone, as on unclaimed land. Breaking protection waits for Tier 2 (D10).
- **Bloom:** the `k` term and the doubled co-presence rate (§1.3).
- **Event:** `ChallengeEvent::ClaimCommons`, fired locally when a Commons Stone is placed.

## 5. The adult layer: the Tally and world bloom

- **Tally** (runtime, this session, per world). Counters kept in a small `tally` module and incremented at the existing seams:
  - blocks mined;
  - blocks placed;
  - items crafted;
  - items smelted;
  - vendor and auction trades.
  - Plus **stock**: the total item count in containers in the loaded chunks, computed when the panel is open, at most once a second.
- **World bloom:** the mean `level` over tracked regions, with its terms:
  - life = mean `eco`;
  - people together = mean `P`;
  - ideas and time are shown as "not yet measured".
- **Where:**
  - The F3 overlay, in the Deepslate Reserve panel's slot, only when Settings → "Grown-up panel (F3)" is on (D16). The sats rows of the old gauge stay behind their existing sats gate.
  - `ConsoleSnapshot` on dedicated servers gains the same figures.
- **Copy:** passes `copy_lint`. No money words. The panel says what each figure counts in plain words.

## 6. The Grey Quarry (act-two place trial)

### 6.1 Engine additions

- `Objective::Place { area: { from, to }, measure: "Bloom", at_least: f32, hold_seconds: u32 }`.
  - The area is offsets from spawn, like `ArenaSetup`.
  - It is allowed as a leaf in `Sequence` and `Checklist`; the lint's allowed leaves become `Action` or `Place`.
  - The running scenario registers a bloom probe on the area and completes when the probe's level has stayed ≥ `at_least` for `hold_seconds`.
  - The task list shows the stage phrase (§1.6), never a number.
- **Time scale:** `ScenarioTuning.bloom_time_scale: f32` (default 1.0).
  - It is applied to the `BloomField` every tick while a scenario runs, **only in a fresh trial arena world**.
  - The in-world `/scenario` overlay never changes the player's own world's tempo.
- **Grey start:** `ArenaSetup.drained: [{ area, level }]` sets the initial level of the probe and of the overlapping regions, on the fresh-world launch only.
- **Fills:** `ArenaSetup.fills: [{ from, to, block }]`, a box fill, so a 24×24 terraced quarry does not need thousands of block rows. The lint's recognised-field list is updated.
- **New events:** `PlaceWentGrey`, `HoldFeast` and `ClaimCommons`. Each goes on the full event checklist: the wire mirror, `matches`, `event_task_label`, the lint variant name and a real fire site.
- **`TrialType::Bloom`:**
  - icon 🌸 (U+1F338), label "Bloom", a colour of its own;
  - TRIAL_ORDER entry after the front shelf;
  - unlock `{ on: [PlaceWentGrey], after: ["generator"], hint: "Somewhere grey is waiting." }`.
  - The builder confirms that `on` and `after` mean "either". If they mean "both", it adds an any-of form.
- **End card:**
  - a four-petal flower: life, ideas, people and time;
  - each petal's size is that term's share of the recovery, measured from the probe's start terms to its end terms;
  - the ideas and time petals are drawn closed;
  - one sentence: "You brought it back mostly through life." (or "...through people together", or "...through life and people together");
  - before/after pictures per D15.
  - `TrialBests.ways_done` records "life" and/or "people" for the trial.

### 6.2 The trial

- **File:** `explorer-grey-quarry.json`, token `grey-quarry`. Display name: "The Grey Quarry: bring a stripped quarry back to life".
- **Arena** (`ground: flat` or similar, plus fills):
  - a terraced stone quarry about 24×24, inside the area;
  - some dirt patches on the terraces and a few log stumps;
  - a dry stream bed running from a spring: a water source behind 2–3 cobblestone blocks at the top terrace;
  - no animals inside;
  - a fenced grass meadow outside the area holding 2 sheep, 1 cow and 2 rabbits (if rabbits spawn by name; otherwise pigs);
  - a chest at spawn.
- **drained:** the quarry area at level 0.05.
- **Kit:**
  - oak, birch and spruce saplings, 4 each;
  - seeds of three crops (wheat, carrot, potato), 8 each;
  - a bucket, a shovel and a hoe (D18);
  - bone meal ×8, bread ×16, wheat ×8 and **Leads ×3**. There is no "animals follow held food" behaviour in the engine; animals are led in on a Lead (`tether.rs`).
- **Objective:** `Place { area: the quarry, measure: "Bloom", at_least: 0.55, hold_seconds: 20 }`.
- **Time scale:** `bloom_time_scale: 12`. Tuned so a full recovery takes about 8–15 minutes of play. Playtest owns the number.
- **Task label:** "Bring the quarry back to life". Its state line is the stage phrase.
- **Help** (tagline / how-to): "It was alive once. What lives in places like this?" It never lists the routes. After one completion it lists the ways the player has done it.
- **Satoshi** (`trial_satoshi`):
  - intro: "Somebody took everything this place had and walked away. It's still here, though. Let's see if it remembers how to be alive."
  - hint (halfway): "Hear that? Something moved in."
  - end line (end card): "Look at it. Nobody owns that colour. It's just there now, for whoever comes next."

## 6.3 Known limits at Tier 1 (for the things-to-try note)

- Worlds saved by this build do not open in 0.2.29 (layout revision 5).
- Bloom is computed per client: two players may see slightly different shades at the edges of what they have loaded.
- Feast and co-presence surges fade on reload; the Feast Marker's lasting lift stays.
- Commons do not yet stop anyone breaking things.

## 7. Build slices (one at a time, in worktree `../AxeNStax-living-world`, branch `living-world`)

| Slice | Content | Rung |
|---|---|---|
| S1 | `bloom` module: §1 math, BloomField, region scan, mob census, probes, stages, creative floor, PlaceWentGrey; wired into `GameState::tick` (all client modes). Calibration tests for §1.2's bands. | build (sonnet) |
| S2 | §2 colour, sound and life layer. | build (sonnet) |
| S3 | §3 Feast and §4 Commons: blocks, recipes, shared use rule, authority wiring, layout and protocol bumps, bloom hooks, events. | build (sonnet), then **independent review** (saves + multiplayer) |
| S4 | §6 Grey Quarry and the engine additions, lint, spec and authoring docs. | build (sonnet) |
| S5 | §5 Tally and world bloom (F3 panel, settings switch, ConsoleSnapshot). | build (sonnet) |

Every slice:
- builds with its own `CARGO_TARGET_DIR` (never the shared main one), `CARGO_INCREMENTAL=0` and `TMPDIR` on the main disk, one cargo job at a time;
- puts every `fire_challenge(ChallengeEvent::…)` site for a new event in `game_loop.rs` or `machine_client.rs` (the trials lint only reads those two files);
- keeps `./check.sh` green (TMPDIR on the main disk);
- updates the relevant numbered spec;
- makes no edits under `tools/sites/` (website untouched);
- commits on `living-world` with no AI attribution.

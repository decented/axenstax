# Wind, Copper & Electricity wave — design

**Date:** 2026-09-07 · **Status:** APPROVED (owner: "crack on and make it work") · **Branch:** `feat/wind-copper-electricity`
**Owner brief:** "Copper Ore never generates underground — fix all that. Make sure the whole electricity thing works properly, the wind turbines work and the water wheel works. Get it into the AppImage. Make sure the trials cover the new functionality so people test it."
**Builds on:** Spec 48 (`docs/foundations/2026-06-17-electricity-power-logic.md`, Phases 1, 2 and the Water Wheel half of 4 delivered), weather sync (protocol v59), `water::flow_vector`.

## 0. TL;DR

Four deliverables, built in this order, one cargo job at a time:

1. **Copper Ore generates** in a mid-depth stone band, with a real ore texture, anti-X-ray disguise, and a `/give` route. Closes the "Electricity is a Survival dead end" gap.
2. **A wind mechanic** (`wind.rs`, derived, never saved, never synced) and the **Windmill** — the last Phase-4 source. Turns in a breeze, always in a storm, only with open sky.
3. **Electricity end-to-end audit**: a `TestHost` integration module that walks the Survival chain (ore → ingot → cable → every device → save/load), fixing whatever breaks.
4. **Seven new Trials** covering copper, water wheel, windmill, build-along, skin painter, rig clips and the feedback mailbox, plus the events and the scenario `weather_lock` they need.

Then docs/spec updates, version **0.2.26**, `check.sh`, push, AppImage.

Not in scope (YAGNI, unchanged): analog signal strength / energy economy (Spec 48 Phase 3 stays gated on playtest), Aether, wind affecting anything but the Windmill and the F3 readout, syncing `PowerDeviceData` to remote clients (remote hover labels stay a known gap).

## 1. Copper Ore generation

- `biome.rs::ore_at`: add a Copper band **after Nitre, before Coal** (own hash seed `self.seed.wrapping_add(4902)`, so it never biases existing bands):
  `(Y_DP..(SEA_LEVEL + 8)).contains(&y) && (hc % 1000) < 30` → `COPPER_ORE` (~3%, iron-like). Stone-only — the band sits above the deepslate threshold, so `ore_variant_matches_substrate` keeps holding. No deepslate copper variant.
- `anti_xray.rs::host_rock`: `COPPER_ORE => STONE` (buried copper hides like iron). Magnesium/Brimstone/Nitre are left as they are (pre-existing decision, not this wave's).
- Texture: `TEX_COPPER_ORE` — procedural stone base with copper-orange flecks in `texture_gen.rs`, same family as the iron-ore generator. `block.rs` copper-ore def gets `tex_*` set. **Layer budget:** 508 used before this wave, hard floor 512; copper takes one, Windmill takes at most two. If the count would pass 511, the Windmill's turning state shares the idle side texture.
- `commands/builtins/give.rs`: `copper` / `copper_ore` → the ore block, `raw_copper` → `MaterialId::Copper`, `copper_ingot` → ingot (only where an alias is missing).
- Tests (`biome.rs`): `copper_appears_in_mid_band` (≥1 hit scanning the band), `copper_only_inside_band` (none below `Y_DP` or above `SEA_LEVEL+8`), `copper_is_stone_only_never_in_deepslate`, and `coal_appears_at_some_density` still within bounds. `anti_xray` test: buried copper is disguised as stone, exposed copper stays.

## 2. Wind and the Windmill

### 2.1 `wind.rs` — the mechanic

Pure, deterministic, derived from state every side already agrees on (tick + synced weather + world seed). Nothing saved, no protocol change.

```rust
pub struct WindSample { pub speed: f32 /* 0.0..=1.0 */, pub direction: u8 /* 0..8, 0 = north, clockwise */ }
pub fn sample(tick: u64, weather: Weather, seed: u64, y: i32) -> WindSample
```

- **Base breeze:** smooth value noise over `tick` (two octaves, periods ≈ 2400 and 9000 ticks, so gusts change over 2–7 real minutes), mapped to `0.10..0.60`.
- **Weather:** `+0.25` while raining, `+0.50` while storming (storm ⊆ rain, so a storm reads `+0.50` total, not `+0.75`). Clamp to 1.0.
- **Altitude:** `+ ((y - SEA_LEVEL).max(0) as f32 * 0.010).min(0.30)` — a windmill on a hill is reliable, one at sea level is intermittent. The gain is set so the `+0.30` cap lands 30 blocks up, just under the world ceiling at `SEA_LEVEL + 33`: at the first tuning (`0.004`) the cap needed +75 blocks and no legal build could reach it, so "build it high" was not a real lever (retuned in Task 2b).
- **Direction:** slow noise over tick, seeded, quantised to 8 points; changes every few minutes. Exposed for the F3 readout and future sails/kites; the Windmill ignores it in this wave.
- Tests: determinism; `0 ≤ speed ≤ 1`; storm ≥ rain ≥ clear at the same tick; altitude monotone; over a 24 000-tick clear-sky sample at `SEA_LEVEL`, the fraction above the Windmill "turn on" threshold lies in `0.25..0.75` (intermittent by design); at `SEA_LEVEL + 75` it is above `0.85`.

### 2.2 The Windmill block + device

- Blocks `WINDMILL` / `WINDMILL_TURNING` appended at the next free ids (idle/active twin, like `WATER_WHEEL` 316/317). `is_power_block`, `mine_drop` (both → Windmill item), no light emission, solid.
- `PowerDeviceKind::Windmill` **appended last** in the enum (bincode-positional; saved).
- Textures: `TEX_WINDMILL` (sail cross on planks) and `TEX_WINDMILL_TURNING` (blurred sails); top/bottom reuse planks. See the layer-budget rule in §1.
- **Turn rule** (in `tick_devices`, mirroring the Water Wheel arm): each tick sample `wind` (passed in — see 2.3). The mill is **exposed** if the cell above is not solid AND at least two of its four horizontal neighbours are not solid AND no solid block sits in the 8 cells above it (a cheap sky check — use the chunk heightmap if `World` exposes one, else the 8-cell scan). Hysteresis: turn **on** when exposed and `speed ≥ 0.35`, turn **off** when not exposed or `speed < 0.30`. Swap `WINDMILL ↔ WINDMILL_TURNING` on transitions only. `is_active_source` → `d.on`.
- Recipe (placeholder, Axolittle confirms feel): 3×3 — top row `Canvas / Canvas / Canvas`, middle `Plank / Copper Ingot / Plank`, bottom `Stick / Iron Ingot / Stick` → 1 Windmill. Canvas already exists (fibre economy). Add to the catalogue + the `electricity_blocks_are_craftable` guard + the catalogue↔matcher consistency test.
- Placement: the `game_loop.rs` place-arm's exhaustive `PowerDeviceKind` match gains `WINDMILL`; break-arm removes the device + `notify_neighbours` (the ghost-source regression test pattern).
- Hover label (`power_ui.rs`): `Windmill — turning (wind: fresh)` / `Windmill — still (wind: light)` / `Windmill — blocked (needs open sky)`; wind words: `<0.20 calm`, `<0.35 light`, `<0.60 fresh`, `<0.85 strong`, else `gale`.
- F3 debug line gains `wind: <word> <speed:.2> <compass>`.
- Tests (`power.rs`): turning in a storm when exposed; idle when calm; idle when enclosed (block above / three neighbours solid); hysteresis (0.33 does not flip a turning mill off, 0.29 does); on→off swaps the block exactly once; `reseed_on_load` re-drives a saved turning mill.

### 2.3 Plumbing

- `power_tick(world, now, entity_positions)` becomes `power_tick(world, now, entity_positions, wind: WindSample)`. Two call sites (`server.rs` ~950, `game_loop.rs` ~5825) compute `wind::sample(tick, weather, seed, y)` — `y` for the sample passed in is `SEA_LEVEL`; the Windmill arm re-applies the altitude term for its own `y` via `wind::with_altitude(sample, y)` so one sample per tick serves every mill.
- Server weather lives in `GameServer` (authoritative window, v59); single-player in `game_loop` state. Both already have the tick and seed.
- Spec 48 `docs/foundations/...electricity-power-logic.md` Phase 4: Windmill → DELIVERED with the rule above. `docs/spec/05-gameplay-systems.md`: a "Wind" subsection under weather. `docs/spec/02-world-format.md`: copper band in the ore table.

## 3. Electricity end-to-end audit

New `src/test_integration/electricity.rs` (registered in `mod.rs`), driving `TestHost` (extend `test_harness.rs` with helpers where needed — place power block with device data, tick N, read block, load fuel). Cases:

1. **Survival chain**: world with the copper band → mine copper (drop is `Copper`), furnace smelt → `CopperIngot`, craft `Rubber/Copper/Rubber` → 3 Cable, craft Lamp, Lever. No `/give`.
2. Lever → cable run (20 blocks) → lamp: lit after one tick; off after toggle; cutting the cable mid-run kills the lamp; ghost-device regression on break.
3. Hand Crank runs `CRANK_RUN_TICKS` then stops; Battery holds a lamp for its buffer after the source stops; Steam Generator lit only while fuelled, `try_load_generator_fuel` accepts coal and refuses cobblestone.
4. Logic gate truth (AND/OR/NOT/XOR) through cable, settles in one extra tick; Button pulse releases after 10 ticks; Pressure Plate on while an entity stands on it.
5. Piston pushes, sticky retracts; Beam Sensor + Mirror; Motion Sensor within `MOTION_RADIUS`.
6. **Water Wheel with real water**: build a 3-step slope, place a water source at the top, tick the water sim until `flow_vector` is `Some` beside the wheel → turning; remove the source, tick until dry → idle. A still pond beside a wheel never turns it.
7. **Windmill**: storm + exposed → turning; roof it → idle; clear-sky at `SEA_LEVEL+80` → turning within 200 ticks.
8. Save → load → `reseed_on_load`: a lit lamp network comes back lit, a turning wheel/mill keeps its device data.
9. Multiplayer: a `TestHost` remote joiner receives the `CABLE_LIT`/`ELECTRIC_LAMP_LIT` block changes after the host toggles a lever (block-change broadcast path).

Any failure is a bug to fix in this wave (not to document around). Fixes go with their own unit test.

## 4. Trials for the new functionality

Per `docs/authoring/trials.md` (6 registration points + `trials_lint` green). All `Tech` unless noted; all use kits/arenas so they are completable in one sitting, and every text surface passes the money-word guard.

| Token | Title — premise | Objective (Sequence) | Arena / kit | New event |
|---|---|---|---|---|
| `copper-rush` | Copper Rush — dig copper, smelt it, draw your first cable | `BreakBlock{copper_ore}` ×3 → `SmeltItem` ×1 → `CraftItem` ×1 | mine face with ≥6 copper ore; kit: stone pickaxe, furnace, coal, rubber ×2 | — |
| `mill-race` | Mill Race — set a water wheel turning on a stream and light a lamp | `PlaceBlock{water_wheel}` → `SourceTurned{WaterWheel}` → `PowerDevice` | a stepped channel with a water source at the top already flowing; kit: water wheel, cable ×8, lamp, bucket | `SourceTurned { kind }` |
| `catch-the-wind` | Catch the Wind — raise a windmill up high and wait for a gust | `PlaceBlock{windmill}` → `SourceTurned{Windmill}` → `PowerDevice` | hilltop platform; `weather_lock: "storm"`; kit: windmill, cable ×8, lamp | `SourceTurned`, `weather_lock` |
| `follow-the-plan` (Make) | Follow the Plan — build a house block by block with the guide | `CompleteBuildGuide` ×1 | flat plot; kit: a developed Plan (small house), its full materials | `CompleteBuildGuide` |
| `fresh-coat` (Make) | Fresh Coat — paint your own skin and wear it | `SaveSkin` ×1 | Workshop-style arena; help text: Tab opens the paint panel, R separates limbs | `SaveSkin` |
| `bouncer` (Make) | Bouncer — build a rig in Rig Studio and give it the Bounce clip | `SpawnRig` ×1 | flat plot; kit: a few block types; help: Y opens Rig Studio | `SpawnRig` |
| `suggestion-box` | Suggestion Box — tell the makers one idea | `SendFeedback` ×1 | any; help: `T` then `/idea <text>` | `SendFeedback` |

- New `ChallengeEvent` variants fire from: `power.rs` (wheel/mill idle→turning transition, via the same channel `PowerDevice` uses in `game_loop.rs`), the build guide's completion path, the Workshop skin studio save/equip path, `rig_studio` Spawn, and the mailbox `/idea` + `/bug` send path (web and native; the event is local, the report is unchanged). `trials_lint`'s exhaustive event match and its "every event has a fire site" check are updated.
- `ScenarioDef.weather_lock: Option<String>` (`"clear" | "rain" | "storm"`), mirroring `time_lock`: while the trial runs, the local weather window is pinned. `trials_lint` learns the field and its valid values. Unknown values fail lint.
- Existing `power`/`generator`/`logic-gate` trials: help text mentions that copper now comes from the ground.

### As built — deviations from §4 above (Task 3, `aa80cbf2`)

- **`copper-rush`'s arena** is a 6×`copper_ore` face (3×2), not a bare "≥6" count; the kit swaps in a **stone** pickaxe (a wooden one just crumbles copper ore, per the tier gate) plus rubber ×4 and a crafting table, and carries `trial_recipe_hints: ["cable"]` to pin the Cable card in the recipe book.
- **`mill-race`'s arena** is a 4-high stone pillar with a `water` source on top rather than a pre-built stepped channel — the spill off all four sides gives every neighbour of the fall a flow vector, which is simpler to author and still teaches "wheel needs a stream."
- **`catch-the-wind`'s arena** is a 4-high pillar under a 3×3 stone platform (not just "hilltop platform") so the mill lands ~5 blocks up with its full `WINDMILL_SKY_SCAN` column clear inside the 96-block world.
- **`follow-the-plan`'s kit** builds a new **`PlanData::small_hut`** (23 cells: doorway, two windows, a roof) rather than reusing an existing "small house" Plan — none existed at the right size — plus a new `/give hut_plan` alias.
- **`fresh-coat`'s arena** needs a new `ScenarioDef` field, **`world_type: "workshop"`**, so the trial's arena is a real Workshop (void floor, avatar mannequin, starter Bellows — all keyed off `World.is_workshop`, not the `world_type` string). Without it the only route into the painter is Esc → Your look → Edit, which leaves the world and ends the trial (`reset_for_world_change` clears `self.scenario`).
- **`SendFeedback`** fires via a new **`CommandResult::FeedbackQueued`**, returned by `/bug`/`/idea` after the native/web hand-off, rather than a new `CommandContext` marker — the alternative would have touched ~40 call sites for no gain. **A bare `/idea` (no text) returns `Silent`, not `FeedbackQueued`, on both targets** — on web it only opens the JS mailbox composer (`wasm_feedback::open_mailbox`), whose own send happens later in JS, outside this command's return path; on native it just prints the usage hint. Only the **inline** `/idea <text>` form queues a message and fires the event, so `suggestion-box` is completable only that way. Native fires the event on every inline send regardless of whether the enqueue itself logged a failure, matching the existing "player sees Queued either way" contract (`commands/builtins/feedback.rs`).
- **`weather_lock` is driven per-tick from the running scenario, not `WorldMeta`** (a deviation from "mirror `time_lock`'s plumbing"): applied every tick immediately after the one place `weather_rain_until`/`weather_storm_until` are written, so it (a) also pins the window for `/scenario <token>` run in your own world, not only a fresh Trials-board arena, (b) needs no save-format field for a value that's explicitly ephemeral, and (c) clears on release without a save-format field. `has_weather` still wins, so a trial can't conjure a storm inside a Workshop. **Correction (final fix wave):** (c) was not true as first built. The pin is re-stamped `LOCK_HORIZON` (1200 ticks — a full in-game minute) ahead every tick so it cannot elapse mid-run, which meant the LAST stamp before the trial ended was still a minute in the future: the player walked out of a finished storm trial into up to 60 s of that storm. The loop now remembers the lock it applied on the previous tick (`GameState.weather_lock_applied`) and `weather::apply_lock` drops the residue back to `Weather::CLEAR` the tick the lock goes away or names a different sky, handing the window back to `weather::advance` at once. The lightning roll reads the same post-lock window, so a released lock also stops throwing bolts on the tick it is released.
- Final count: **38 → 45** Explorer Challenges (46 challenges incl. onboarding, + 3 races = 49 `TRIAL_ORDER` entries).
- **Not verified solo** (playtest items, per the task-3 report): the four UI-path trials (`fresh-coat`, `follow-the-plan`, `bouncer`, `suggestion-box`) fire from flows no headless harness reaches, and the two authored arenas (`mill-race`'s waterfall, `copper-rush`'s ore face) have no test that walks a player through the geometry.

## 5. Docs (source-of-truth rule)

Player guide `electricity.md` (drop the copper warning; add the Windmill + wind words; Water Wheel unchanged), `blocks-and-mining.md` (copper band), `whats-coming.md`, `minigames.md` (trial count), learn `switch-on-a-lamp.md` (one line). Spec 02 ore table, Spec 05 wind subsection, Spec 48 Phase 4 status. Test sheet `docs/test-sheets/2026-09-07-wind-copper-electricity.md` for Axolittle.

## 6. Release

`game/engine/Cargo.toml` + `tools/packaging/packager.toml` → **0.2.26**; `./check.sh` foreground, `-j 4`, ALL GREEN; commit; merge to main; dispatch `native-packages.yml` (`linux_only`, pre-authorised). The web deploy and the installer sync to the box are parked on the SSH failure — the AppImage is collected from the workflow artefact / release.

## 7. Memory-rule check

No public directory, no AxeNStax-operated service, no data collection, no money words (the `suggestion-box` trial sends the existing burner/device-key mailbox report, nothing new). Proof of Play untouched.

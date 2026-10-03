# Deferred-lists wave — night-lit entities, dispenser arms, ghost share, painter polish

**Date:** 2026-07-05
**Status:** design, autonomous build session (owner: "build whatever you can solo")
**Branch:** `feature/deferred-lists-wave`
**Scope tier:** medium ×4, all solo-verifiable (tests + headless `--shot-*` screenshots), no owner boundary

## Why this wave

The active backlog is clear (particles c1198cb4, gap-fill wave, rails, Trials
Phase 1 all shipped). What remains solo-buildable is the **documented deferred
list** from those waves, plus one small step on the Trials loop that stays on
the safe side of the Phase-2 owner decision. Four campaigns:

| # | Campaign | Deferred-from |
|---|----------|---------------|
| N | Night-light completion — entities/plants/micro dim at night | 2026-07-04 polish campaign ("entity/plant/micro night dimming — still full combined light") |
| D | Dispenser behaviours — bucket / bonemeal / ignite arms | 2026-07-04 dispensers campaign (`dispenser.rs` doc header: "future dispense behaviours … are new arms HERE") |
| G | Ghost export/import — race a friend's ghost from a file | Trials vision §Phase 2 (the *smallest* share primitive; see red-lines note below) |
| S | Skin painter redo + finer colour picker | Skin studio ship (documented NEXT items) |

**Explicitly deferred again (with reasons):** weather sync (server already rolls
the *same deterministic tick-hash formula* as the client — `server.rs:619-633` —
so it is synced by construction; revisit only if a real divergence is observed);
water step-side seams (cosmetic mesh work, low value); fire burn-tiers/afterburn
(feel-tuning, wants a playtest first); piston arm animation (GPU boundary).

**Red-lines check (all four):** no networking, discovery, identity, hosting or
data-collection changes. Campaign G is deliberately **file-based only** — a kid
exports a ghost file and hands it to a friend however they like (USB, their own
chat, whatever). No relay, no upload, no AxeNStax-operated channel, nothing
collected. It extends the shipped chase-your-own-ghost loop and does **not**
pre-empt the open Phase-2 decision (ghost-vs-build-vote *publish rail*); if the
owner later picks build-vote first, this file format still stands alone.

---

## Campaign N — Night-light completion

**Problem.** The 2026-07-04 polish campaign split sky/block light so nights get
dark and torches matter — but only for terrain. Mobs, dropped items,
projectiles, carts, player avatars and plant/grass micro-instances still render
full-bright (`entity_model.rs` bakes `Vertex::FULL_BRIGHT, sky_light: 0.0`), so
a zombie glows like a lamp at midnight and grass stays daylit. This visibly
undercuts the shipped feature.

**Design.** Three sub-paths, matching how each is rendered today:

1. **Entity pass** (mobs, dropped items, projectiles, carts, rigs — all go
   through `fs_main`, which already computes `max(light, sky_light * sun.w)`):
   sample world light once per entity at its occupied cell
   (`world.block_light_at` / `sky_light_at`, mid-height of the entity so we
   don't sample the ground block) and bake `light = block/15`,
   `sky_light = sky/15` into the freshly built verts. The builders in
   `entity_model.rs` gain a `(f32, f32)` light argument threaded from the
   `game_loop.rs` call sites (~:13595-13640), where `self.world` is in scope.
   A small pure helper `entity_light_at(world, pos) -> (f32, f32)` is the
   testable unit.
   - **Emissive exemptions** stay full-bright: Satoshi's `>1.5` glow sentinel is
     stamped *after* the light bake (already ordered that way at
     `entity_model.rs:934-937`); GlowSquid gets an explicit exemption (it is
     the game's living lamp). Hurt-flash (normal overwrite) is orthogonal.
2. **Avatar pass** (third-person self + remote players, `fs_avatar`): the
   per-vertex `light` channel is already repurposed as the screen-door fade
   alpha, but `sky_light` is baked `0.0` and unused. Avatars rebuild verts every
   frame, so bake the **CPU-resolved final scalar**
   `max(block/15, sky/15 * sky_brightness)` into `sky_light` and have
   `fs_avatar` multiply rgb by `max(in.sky_light, 0.08)` (same floor as
   terrain). Both sides of that contract change in one commit. The first-person
   viewmodel (arm via avatar pipeline, held item via entity pipeline) samples at
   the player's eye cell so your own hand dims in a cave too.
3. **Micro-instances + plants** (`vs_micro` attr 9 `inst_light`, `vs_plant`
   attr 5 `light` — both currently a *combined* snapshot with
   `out.sky_light = 0.0`, the documented "P1 follow-up"): split the snapshot.
   Add one instance attribute each (`MicroInstance` attr 10 = `inst_sky`,
   `PlantInstance` attr 6 = `sky`), bake block and sky separately at mesh time
   (`mesh.rs` already reads both), and set `out.sky_light` in the vertex
   shaders so the existing `fs_main` night formula applies. Chunks re-mesh on
   load/change, so no save impact.

**Not in scope:** per-face entity lighting (one sample per entity is the
Minecraft-parity look), dynamic re-light of stationary chunk meshes (already
handled by the light system), particles (already dim via cave-dimming; their
single-scalar path is fine).

**Verification:** unit tests on `entity_light_at` (dark cave → floor, torch-lit
→ block channel, midday surface → sky channel); vertex-layout/naga shader
guard tests updated for the new attributes; visual: `--shot-3p` day vs night
screenshots (extend the shot harness with a time-of-day override if it lacks
one).

---

## Campaign D — Dispenser behaviours

**Problem.** Dispensers eject-or-shoot only (`Arrow` special-cased); the module
header reserves the seam for bucket/bonemeal/fire arms.

**Design.** New `EjectKind` variants decided in `eject_decision`
(`dispenser.rs:93`), realized in the existing caller loop
(`game_loop.rs:3789-3815`) using the established
`set_block → pending_block_changes.push(BlockChange) → rebuild_chunk_at` idiom:

- **`PlaceLiquid`** — `WaterBucket`(163)/`LavaBucket`(164): if the facing cell
  is AIR (reuse `bucket::empty_result` rules), place the source block,
  register it with the fluid sim exactly as the right-click bucket path does,
  and put an **empty Bucket back into the dispenser's inventory** (Minecraft
  parity). If blocked: return the stack unconsumed, no-op.
- **`Bonemeal`** — Bonemeal(10): apply `growth::bonemeal_advance` to the facing
  block with the same `tick ^ position-hash` seed idiom as the right-click path
  (`game_loop.rs:12130-12160`). On `None` (not a crop / mature): return the
  stack, no-op. Same semantics as hand-use — no new growth rules.
- **`Ignite`** — FlintAndSteel: ignite the facing cell through the same seam
  the right-click flint-and-steel uses (fire.rs ignition; respects
  `fire_spread_enabled`). The tool stays in the dispenser and takes the same
  durability cost as a hand ignition.

A shared `return_stack` helper puts failed/leftover items back in the
dispenser's embedded chest (never dropped on the floor). `DispenserData` /
`PowerDeviceData` layouts are **not touched** (hard rule from the dispensers
ship). Hoppers already feed dispensers, so bonemeal-farms and water-timers
compose for free.

**Verification:** unit tests on the new `eject_decision` arms + `return_stack`;
integration test if the dispenser tick is reachable from `TestHost`, else
world-level tests on the extracted apply helpers. Behaviour parity asserted
against the right-click paths (same functions, not copies).

---

## Campaign G — Ghost export/import (race a friend's ghost)

**Problem.** The Trials loop ends at chase-your-own-ghost. The single cheapest
fun multiplier — "beat my ghost" between two kids on two machines — needs only
a file in and out. This is also the primitive any future SHOW rail would reuse.

**Design.**

- **File format** `.axeghost` (JSON, serde): `GhostShareFile { version: 1,
  trial_id, trial_name, ticks, author: Option<String>, ghost: GhostRecording }`.
  ~24 KB for a 60 s run (frames are already capped at 12 000). `author` is a
  free-text label defaulting to the local display handle — written by the
  exporter, trusted by nobody, purely a HUD label.
- **Export**: in the lobby Trials accordion, an expanded race row with a best
  gains an **"Export ghost"** button (`menu.rs:1495-1507` button row): native →
  new `SaveGhost` arm in `native_file_dialog.rs` (copy of `SaveSkin`); web →
  new `ghost_download_wasm` bridge mirroring `skin_download_wasm` (+ the JS
  side in the game site, mirroring `axenstax_skin_download`).
- **Import**: an **"Import ghost"** button on the same row: native `OpenGhost`
  arm / web `pick_ghost_wasm` (+ JS mirroring `axenstax_pick_skin_file`).
  Parse-validate; if the file's `trial_id` doesn't match the row, route it to
  the matching race instead and say so in a toast (a kid double-clicking the
  wrong row shouldn't lose the import). Imported ghost is stored as the race's
  **rival**: `TrialBests` gains `#[serde(default)] rivals:
  BTreeMap<String, RivalGhost { label, ticks, ghost }>` — one rival per race,
  import replaces, persists in the existing `profile/trials.json` /
  localStorage store (tolerant JSON keeps old files loading).
- **Racing it**: `ActiveTrial` gains `rival: Option<...>` armed at
  `start_trial`. `refresh_trial_ghost` samples both recordings and pushes both
  box sets into the one `set_trial_ghost` call (the renderer channel is
  already arbitrary-length) — own PB stays cyan, rival renders **orange**.
  The finish panel adds one line when a rival is set: beat their time →
  "You beat <label>'s ghost!", else "<label>'s ghost: <time>".
  Lobby row shows the rival label + time under the best-time badge.
- **Glyph rule**: all new UI text uses renderable-glyph-safe strings (no ✓/▸ —
  known egui font gap).

**Not in scope:** any publish/relay/vote rail, leaderboards
(ghost-as-truth needs anti-cheat — stays deferred), multi-rival grids.

**Verification:** serde round-trip tests (share file, rivals map, tolerant
load of pre-rival JSON), trial-id mismatch routing test, `sample`-both-ghosts
logic test; `--shot-lobby` with a seeded rival in `profile/trials.json` to
screenshot the new row state.

---

## Campaign S — Skin painter redo + finer colour picker

**Problem.** Undo (Z) exists (24-snapshot stack); redo doesn't. Colour comes
only from held dyes or the G eyedropper — 16 dyes is coarse for skin work.

**Design.**

- **Redo**: `redo: Vec<Vec<u8>>` beside `undo` on `SkinPaintSession`. Undo pops
  onto redo; a new stroke clears redo; redo pops back onto undo. Same 24 cap.
  Bound to **X** (review fix: the drafted Shift+Z chord collided with Shift =
  fly-descend, held while painting mid-air, stealing undos; X sits next to Z).
  The transition logic lives in small methods on the session with unit tests.
- **Finer colour picker**: while a paint session is active, a small egui window
  (toggle **O** — review fix: the drafted C collided with hold-to-zoom, which
  a kid uses for mannequin detail work) with `color_edit_button_srgba` wired
  to `session.picked_color` — the plumbing already prefers `picked_color` over
  the held dye, so this is UI-only. Includes a "back to dye" clear button and
  the current-colour swatch. Workshop *face*-painter palette is untouched
  (separate surface, not in the NEXT list).

**Verification:** unit tests on undo/redo transitions (undo→redo→undo
round-trip, stroke-clears-redo, caps); `--shot-painter` screenshot with the
picker open.

---

## Ship plan

Branch `feature/deferred-lists-wave`, one commit per campaign (N, D, G, S) plus
this doc. `check.sh` green before merge; `/code-review` on the full diff; merge
ff to `main`, push (web auto-deploys). **Native AppImage not rebuilt** (metered
CI — owner call). Test sheet → `AxeNStax-internal/docs/test-sheets/`, focused
on the feel knobs: night entity visibility vs difficulty, dispenser arm timing,
ghost import UX with a real second kid, picker discoverability.

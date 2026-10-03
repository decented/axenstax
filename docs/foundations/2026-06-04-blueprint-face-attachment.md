# Blueprint as a Face Attachment — paper-thin blueprints on a generalised surface store

**Status: ✅ BUILT + merged to main** (`blueprint_attach.rs`; commits 48f45ef/6df135a/c093d48/3634d21). Reconciled 2026-06-19. Supersedes the
deferred "blueprint-as-a-face-attribute" idea that was **explicitly dropped** on 2026-06-03
(`docs/backlog/owner-inbox.md` §#1/#2/#3; `docs/foundations/2026-06-03-face-overlay-wallpaper.md:5-9`).
The deferral assumed the owner's face-overlay desire was "really about décor"; the owner has
since confirmed the face model **is** required for blueprints — because a laid blueprint must
have **zero physical height** or every captured build ends up a block too tall. This doc
reverses that deferral with the owner's explicit go-ahead.

**Branch:** `docs/blueprint-face-attachment-design` (this doc) → implementation branch off
`main` when we get there.

---

## The problem (owner, 2026-06-04)

A laid blueprint today is a **full solid cube**. `LATENT_PRINT` (id 158, `block.rs:2170`) is
placed at `target + face` — the air cell on top of the ground (`game_loop.rs:4914-4961`), so it
sits **one whole block proud** of the surrounding terrain. Build on top of it and the structure
is a block too high; the only workaround is digging the plot down a block ("foundations"), which
the owner explicitly does not want.

The owner's mental model is **paper**: "one thin layer that you can see but has no physical
height to it." Lay it down, build what you want on top, lock it into the blueprint, then remove
it or leave it — doesn't matter which. And the same surface-attachment idea must eventually work
on **walls** (murals) and **ceilings** (chandeliers), not just floors.

## What already exists (verified against the tree 2026-06-04)

The **face-overlay wallpaper** system shipped 2026-06-03 and is exactly the paper-thin primitive
needed, but only wallpaper consumes it:

- **Store:** `World.face_overlays: AHashMap<(i32,i32,i32), FaceOverlays>` where
  `FaceOverlays = [Option<OverlayData>; 6]` and `OverlayData { block: BlockId }`
  (`world.rs:74, 329`). Accessors `set_face_overlay`/`face_overlay_at`/`remove_face_overlay`/
  `remove_face_overlays_at`/`iter_face_overlays` (`world.rs:670-700`).
- **Render:** `emit_face_decals` (`mesh.rs:564`) + `emit_decal_quad` (`mesh.rs:616`) — one
  textured quad per painted face, inflated `e = 0.003` **outward along the face normal**
  (`mesh.rs:625-639`), so the decal always sits on the *room* side of the face (a ceiling
  overlay hangs *below* the block, facing down). Depth-biased + alpha-blended, reusing the
  crack-overlay technique. **No collision** — nothing in `physics.rs`/`player.rs`/`raycast.rs`
  reads `face_overlays`; it is render-only, zero height. All six faces already supported.
- **Persistence:** `WorldSave.face_overlays: Vec<SavedFaceOverlay>` (`save.rs:162`),
  `SavedFaceOverlay { x, y, z, face, block }` (`save.rs:209`), `face_overlays_to_saved`
  (`save.rs:220`) + `restore_face_overlays`.
- **Interaction:** paint branch in the place handler + two-stage peel (first strike peels the
  overlay, second breaks the block) + clear-all-overlays-on-destroy, all recoverable.

The blueprint capture system is **cell-based and shipped**: `PlanData` (`plan.rs:167`),
`DevelopState` (`plan.rs:44`, `Latent { exposure_ticks } → Developed`), sun-development tick
(`latent_print.rs`), flood-fill capture + animated build (`plan.rs`). Its Axolittle playtest is
still the only open gate, which is why 2026-06-03 cautioned against re-architecting it.

## Scope decision (owner, 2026-06-04)

**Option 1 on a generalised store.** Build flat blueprints + flat décor (murals) now on a
*generalised* face-attachment store; defer 3D-on-a-face objects (chandeliers) behind a single
future enum variant. Capture is **floor-only** for now. This kills the height bug, delivers
wall murals nearly free (the decal already does all six faces), and leaves a clean one-variant
seam for the 3D work — without re-touching storage/lifecycle later.

---

## Design

### 1. Generalise the attachment payload — `FaceAttachment`

Widen the per-face payload from a bare block id to a typed attachment. **Storage, peel/clear/
persist lifecycle, and face targeting are written once and shared by every kind.**

```rust
enum FaceAttachment {
    Wallpaper(BlockId),               // shipped — migrates onto this verbatim
    Blueprint(Box<BlueprintData>),    // this design — boxed; payload is large
    // Model(MicroModelId, Orientation),  // DEFERRED — chandeliers, one variant + one render branch
}
```

- `World.face_overlays` becomes `AHashMap<(i32,i32,i32), [Option<FaceAttachment>; 6]>`. The
  Blueprint payload is **boxed** so the six-slot per-face array stays cheap (a wallpaper face
  must not pay a `PlanData`-sized slot).
- `BlueprintData` carries the captured plan + its develop state + provenance:
  ```rust
  struct BlueprintData {
      plan: PlanData,            // reuse the shipped capture struct
      develop: DevelopState,     // Latent { exposure_ticks } | Developed
      provenance: Provenance,    // see §4
  }
  ```
- Existing `OverlayData { block }` is folded into `FaceAttachment::Wallpaper(block)`. Accessor
  signatures generalise from `OverlayData` to `FaceAttachment`; the wallpaper call sites change
  only at the construction/match points.

**Why a shared store, not a blueprint-specific field:** adding chandeliers later becomes one
enum variant + one render branch + one collision decision — **no migration, no rework**. Bolting
blueprint fields onto `OverlayData` would force exactly that rework (the bridge-debt CLAUDE.md
warns against). We replace the bridge once, properly, and stack both blueprints and future
models on it.

### 2. Blueprint lifecycle (the height fix)

- **Lay:** hold Blueprint Paper (`BLUEPRINT_PAPER`, id 56), right-click the **top face of a
  floor block** → set a `FaceAttachment::Blueprint` on that face (mirrors the wallpaper paint
  branch). Paper-thin, zero collision, zero height — the build above sits at correct world
  height. **This is the fix.**
- **Build:** place the structure in the cells above the papered floor tiles, normally.
- **Lock in:** right-click a papered floor tile with an **empty hand** → capture. Flood-fill the
  connected papered top-faces (4-connected in XZ) and the build volume sitting above them into a
  `PlanData`, exactly as the shipped capture does (`plan.rs` flood-fill + volume sweep), then
  produce a portable `Item::Plan` — the reusable artefact blueprints exist for.
- **After:** the paper **persists** as a peelable attachment. First mining strike on the papered
  face peels the blueprint (recover it / pick the Plan back up); second strike breaks the floor
  block; destroying the block clears the attachment and returns it — same two-stage rule as
  wallpaper. Leave it as a marker or peel it; either is fine.

### 3. Capture speed — mode-split (resolves the develop-state question)

We keep **both** the instant and the sun-developed paths, gated by game mode — consistent with
Spec 24's already-mode-aware build (creative lays from thin air, survival locks real materials):

- **Survival:** slow + real. Lock-in yields a **Latent** (pale) blueprint that must accumulate
  ~`DEVELOP_THRESHOLD_TICKS` of direct daytime sky-light before it flips to **Developed** (blue)
  and becomes a usable Plan. Drives the existing develop tick (`latent_print::tick_develop`,
  `is_developing_condition`), now walking floor Blueprint attachments instead of `LATENT_PRINT`
  blocks. Real materials locked per Spec 24.
- **Creative:** instant. Lock-in produces a finished **Developed** (blue) Plan immediately — no
  sunlight gate, no material lock.

### 4. Provenance (groundwork — honest about trust)

At capture, stamp `BlueprintData.provenance`:

```rust
struct Provenance {
    made_in: GameMode,        // Survival | Creative
    integrity: IntegrityTag,  // snapshot of the World Integrity Ledger at capture
    trust: TrustLevel,        // SelfAsserted (today) — upgradeable later
}
```

- **Integrity snapshot:** the engine already keeps a **World Integrity Ledger** that flags
  `/give`, `/gamemode`, and other cheats and persists them on `WorldMeta`
  (`game_loop.rs:9114`; `save.rs:2676`). Capturing in a clean world is meaningfully different
  from capturing in a flagged one — tamper-evidence, not proof.
- **Trust tiers** (design-stated, only tier 1 buildable now):
  1. **SelfAsserted** — a flag in the Plan. Forgeable (edit the save, toggle `/gamemode`).
     Records intent; proves nothing. *This is what we ship.*
  2. **Tamper-evident** — the integrity snapshot above. Lightweight, exists today.
  3. **Attested** — server-witnessed or Signet-signed at capture. The real "survival-forged,
     verified" guarantee. **Deferred** — blocked on server-authoritative save + the Phase 4
     signing bridge (both in CLAUDE.md known-debt).

We **carry the field now** so the seam exists; the rarity/value of "survival-forged" only lights
up once tier 3 lands. We do not fake a guarantee we cannot keep. The actual *payoff* (trading
survival blueprints, sats for them) is further downstream, behind the **parked** settlement model
(`project_settlement_model_decision_parked`) — this is groundwork, not payday.

### 5. Flat décor / murals

Already delivered by the shipped wallpaper decal — works on all six faces today. In scope here
only as **confirmed coverage** of "murals on walls": placing flat art on any face is the existing
`FaceAttachment::Wallpaper` path. **No** paint-your-own-image authoring tool in this design.

### 6. Rendering

A Blueprint attachment renders through the existing decal path (`emit_face_decals` /
`emit_decal_quad`) as a flat quad on the floor's top face, paper-thin, lit by the room. Colour is
driven by develop state via the shipped `develop_state_color` (`plan.rs:114`): **pale Latent →
blue Developed** (`[0.16, 0.32, 0.62]`). The render dispatches per `FaceAttachment` kind —
Wallpaper/Blueprint → decal quad; future Model → baked micro-model.

### 7. Migration & compatibility

- `LATENT_PRINT` (158) and `CYANOTYPE_PRINT` (172) full-cube blocks are **retired** in favour of
  attachments. On load, **auto-convert** each such block in an existing save into a floor
  Blueprint attachment on the block beneath it (carrying its `LatentPrintData`/develop state),
  then clear the cube. Old worlds keep working; no build is lost.
- **Save-format bump:** `SavedFaceOverlay` generalises to carry the attachment kind + payload
  (wallpaper rows decode as `Wallpaper`; a new blueprint row variant carries the boxed
  `BlueprintData`). Legacy saves with the old `{ block }` shape decode as `Wallpaper`
  (`#[serde(default)]` + a tagged enum, mirroring the existing back-compat tests at
  `save.rs:2715`).
- Cell-based capture/build internals (`plan.rs` flood-fill, animated builder, Architect's
  Plaque) are **unchanged** — only the *laid-paper representation* moves from a block to an
  attachment. This keeps the 2026-06-03 caution satisfied: we are not re-architecting the
  capture data model, only where the paper lives in the world.

---

## Phased scope (build order)

| # | Phase | Touches |
|---|-------|---------|
| A | `FaceAttachment` enum + generalise the store/accessors; fold wallpaper onto it | `world.rs` |
| B | Save-format generalisation + back-compat decode + round-trip tests | `save.rs` |
| C | Render dispatch per kind; blueprint decal colour from `develop_state_color` | `mesh.rs`, `shader.wgsl` |
| D | Lay / lock-in / peel handlers for blueprints (replace the `LATENT_PRINT` block path) | `game_loop.rs` |
| E | Mode-split capture (survival sun-develop via `tick_develop` on attachments; creative instant) | `game_loop.rs`, `latent_print.rs` |
| F | `Provenance` stamp (mode + integrity snapshot, `SelfAsserted`) at capture | `plan.rs`, `game_loop.rs` |
| G | Migration: auto-convert saved `LATENT_PRINT`/`CYANOTYPE_PRINT` cubes on load | `save.rs` |
| H | Spec maintenance: fold into Spec 05 (block interaction) + Spec 03 (rendering) + retire the cube blocks in Spec 38 | `docs/spec/*` |

## Testing posture

- **Pure-function / TestHost units:** store set/get/remove/clear over the generalised enum;
  save→load round-trip for each attachment kind; legacy-save decode as `Wallpaper`; the
  cube→attachment migration; the develop tick advancing a floor Blueprint attachment under
  sky-light; provenance stamping (mode + integrity tag) at capture; two-stage peel + recover.
- **Playtest gate (Axolittle):** the *visual/feel* — does it read like paper, does the build sit
  flush with surrounding terrain, does survival's sun-develop feel right. Not solo-verifiable.

## Explicitly deferred

- **Chandeliers / 3D-on-a-face** (`FaceAttachment::Model`) — one variant + a micro-model render
  branch + a collision decision, later.
- **Capturing builds off walls / ceilings** — floor-only for now (matches v1 flat-ground rule).
- **Paint-your-own-image mural authoring** — murals are the existing wallpaper decal.
- **Cryptographic / server-witnessed provenance** (trust tier 3) — blocked on server-authoritative
  save + Phase 4 signing bridge.
- **Trading / sats for survival-forged blueprints** — blocked on the parked settlement model.

## Risk / confidence

**Medium.** Storage, render, and persistence all generalise proven shipped patterns (the
wallpaper face-overlay, the cell-based capture, the develop tick) — the riskiest mechanical part,
the paper-thin zero-height render, is already shipped and verified render-only. The real risks:
(1) the save-format generalisation must keep wallpaper saves decoding cleanly (locked by a
back-compat test); (2) the cube→attachment migration must not drop a develop state mid-flight
(locked by a migration round-trip test); (3) retiring blocks 158/172 touches the live Spec-38
path, so the migration and the lay/lock/peel handlers must fully replace the old block path in
one go. Visual/feel is the only non-solo-verifiable part → playtest gate.

## Memory-rule check

- `feedback_merge_to_main_preauthorised` — merge when `check.sh` is ALL GREEN, no ask.
- `project_settlement_model_decision_parked` — provenance is groundwork only; the trading/sats
  payoff stays parked. Do not design Bitcoin settlement here.
- `project_charter_phase4_shared_gap` — trust tier 3 (attested provenance) shares the deferred
  signing-bridge gap; do not promise it as live.
- `feedback_uk_english_naming` — UK English throughout.
- `feedback_autonomy_to_playtest_boundary` — build to the playtest boundary, stop cleanly.
- Spec maintenance (`CLAUDE.md`) — fold the attachment path into Spec 03 + Spec 05 and retire the
  cube blocks in Spec 38 once shipped (Phase H).

---

## Implementation outcome (2026-06-04)

**Status: BUILT — engine tests + WASM build green. Awaiting Axolittle playtest.**

### What shipped vs the design

**Data model.** The generalised store shipped exactly as designed, with one naming
refinement: the blank draughting paper is its own variant rather than a
`Blueprint(Latent)` with no data:

```rust
enum FaceAttachment {
    Wallpaper(BlockId),
    BlueprintBlank,                    // cream paper laid down, no plan yet
    Blueprint(Box<plan::PlanData>),    // captured plan (Latent or Developed)
}
```

`World.face_attachments` replaces the old `World.face_overlays`; the three
serialisation rows are `SavedFaceOverlay`, `SavedFaceBlueprint`, and
`SavedFaceBlankPaper`, all `#[serde(default)]` so existing saves load safely.

**Primary capture surface — `BLUEPRINT_PAPER`.** The design text leant on
`LATENT_PRINT` as the laid-paper; the actual PRIMARY capture surface is the
cream `BLUEPRINT_PAPER` (block id 56) draughting sheet. Holding Blueprint Paper
and right-clicking a floor's **TOP face** paints a `BlueprintBlank` attachment —
paper-thin, zero height, zero collision. A held Blueprint Paper never places a
solid block-56 cube anymore; non-floor faces are no-ops. The Latent blueprint
re-lay (see below) is the complementary *survival* path, not the primary path.

**`LATENT_PRINT` re-lay — survival develop path.** After capture, the player
holds an `Item::Plan` (Latent state). Laying it on a floor (`blueprint_attach::
lay_blueprint_on_floor`) creates a `Blueprint(Latent)` attachment. The new tick
function `latent_print::tick_develop_attachments` advances it under full
daytime sky-light (threshold: `DEVELOP_THRESHOLD_TICKS` = 1800 ticks ≈ 90 s) →
flips to Developed. The player then peels the finished plan. This is the
survival sun-develop loop for *re-placing a captured Latent plan*, distinct from
the initial lay-and-capture.

**Capture rework.** `plan::capture` and `flood_fill_tiles` now read
`BlueprintBlank` attachments instead of scanning for `BLUEPRINT_PAPER` solid
blocks. `check_flat_ground`'s foundation check was moved to the tile cell itself.
Empty-hand right-click on a `BlueprintBlank`-papered floor triggers the capture:
flood-fill finds all connected blank tiles, sweeps the build volume above, and
produces a `PlanData`.

> **Capture model superseded (2026-06-04):** The capture algorithm described above
> (empty-hand right-click + air-flood/shell sweep) was **subsequently reworked** to
> the **column-capture (connectivity flood) + Drafting Stamp** model. The live
> semantics are in `docs/foundations/2026-06-04-blueprint-column-capture.md` and
> Spec 05 §2.9. The lay / render / persist / peel / sun-develop / mode-split
> described elsewhere in this document remain accurate and are unchanged.

**Commit keeps the floor (bug fix).** The old capture code set tiles to `AIR`,
destroying the floor beneath the captured build. Fixed: `remove_face_attachment`
consumes the `BlueprintBlank` attachment; the underlying floor block is preserved.
Attachment changes are **not** broadcast as `BlockChange`s (same posture as
wallpaper).

**Mode split.** Creative capture → `develop_state = Developed` (instant). Survival
capture → `Latent { exposure_ticks: 0 }`. Consistent with Spec 08's World Integrity
Ledger; `authored_in` (creative/survival) is stamped on every captured plan.

**Two-stage peel.** First mining strike on any face attachment peels it and returns
the right item: `Wallpaper` → `Item::Block(block_id)`, `BlueprintBlank` →
`Item::Block(BLUEPRINT_PAPER)`, `Blueprint(plan)` → `Item::Plan(plan)`. Second
strike breaks the underlying block. Destroying a block recovers all its attachments.
Unified via `blueprint_attach::recovered_item_for`.

### Phases shipped

| Phase | Shipped |
|-------|---------|
| A | `FaceAttachment` enum; generalised store; wallpaper folded on |
| B | Save-format + `#[serde(default)]` back-compat; round-trip tests |
| C | Decal dispatch per kind; Blueprint colour from develop state |
| D | Lay / lock-in / peel handlers (replace `LATENT_PRINT` block path) |
| E | Mode-split capture; survival sun-develop via `tick_develop_attachments` |
| F (partial) | `authored_in` (creative/survival) stamped at capture |
| H | Spec maintenance (this section + Spec 03/05 updates) |

### Deferred phases

**Phase G — migration of old saved full-cube blocks.** Auto-converting saved
`LATENT_PRINT` (158) / `CYANOTYPE_PRINT` (172) / `BLUEPRINT_PAPER` (56) solid
cubes in pre-existing worlds to face attachments is **NOT done**. Block ids and
definitions are kept for decode; the block-based `latent_print::tick_develop` still
exists. Old cubes in legacy saves remain as-is. Deferred because no live alpha saves
exist yet and the migration needs a targeted test session.

**Phase F (provenance integrity snapshot) — NOT done.** The `Provenance` struct
carrying the World Integrity Ledger snapshot (`integrity: IntegrityTag`) and the full
`trust: TrustLevel` ladder was not stamped. `authored_in` is recorded (creative vs
survival), giving tier-1 self-asserted provenance. The integrity snapshot + tier-2
tamper-evidence are deferred until a session that validates the ledger wire-up and
adds the round-trip test.

### Contradictions with the original design, corrected

- The design assumed `LATENT_PRINT` (158) was the primary laid-paper object. The
  implementation uses `BLUEPRINT_PAPER` (56) as the primary capture surface;
  `LATENT_PRINT` re-lay is the survival develop path for a *previously captured* plan.
- `BlueprintBlank` is a standalone variant, not a `Blueprint(Latent)` placeholder —
  the blank paper has no `PlanData` until lock-in.
- Migration (Phase G) did **not** ship; old full-cube block ids are kept, not retired.

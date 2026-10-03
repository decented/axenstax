# Face-Overlay Wallpaper — paint one face of a block, paper-thin

**Status: READY TO BUILD → BUILDING 2026-06-03.** Graduated from owner-inbox item
**#1/#2/#3** (`docs/backlog/owner-inbox.md` §#1/#2/#3, `PLANNED`) under the
2026-06-03 Tier-4 reprioritisation **SPLIT**: build the genuinely-new face-overlay
**wallpaper** primitive; **DROP/defer** the blueprint-as-a-face-attribute rewrite (the
shipped cell-based blueprint — Specs 24/38 — already works and is unplaytested; do not
re-architect it on spec). This doc is the buildable design for the wallpaper half only,
grounded in the real engine code with exact `file:line` anchors.

**The idea (owner):** a décor surface that coats **one face** of an existing block
without occupying a cell or adding a layer — paper-thin so builds never shift, removable
independently of the block, and the two sides of a wall paint separately.

**Mining rule (owner):** crosshair on an overlaid face → **first strike peels just the
overlay** (recoverable item), **second strike** breaks the block (hard blocks take their
normal multi-strike); hitting a **bare** face breaks the block directly; **destroying a
block clears all its overlays** — every removed overlay is recoverable.

**Branch:** `feat/face-overlay-wallpaper` off `main`. One branch, one merge when
`./check.sh` is ALL GREEN. The visual/feel confirmation (does it *look* like wallpaper,
do two sides read independently) is a **playtest-boundary** check Axolittle runs — see
the test sheet.

---

## TL;DR

Today "wallpaper" is **16 solid opaque décor blocks** (`WALLPAPER_*`, ids 145-157 +
159-161, `block.rs:265-286`; textures `TEX_WALLPAPER_*` 257-269 + 273-275,
`block.rs:944-964`) crafted from Papyrus Sheet + dye. None of the paper-thin behaviour
exists. This build adds an **additive face-overlay path alongside** those solid blocks —
the solid blocks are **untouched**.

A wallpaper overlay is a **per-(block, face) decal**: a single textured quad sitting
~0.003 proud of the block face, alpha-blended with a negative depth bias so it hugs the
surface without z-fighting — exactly the render technique the **block-break crack overlay**
already uses (`make_crack_pipeline` renderer.rs:3097, `build_crack_cube` renderer.rs:3149,
`fs_crack` shader.wgsl:239). The overlay **reuses the wallpaper block's own texture layer**
(`registry.tex_side(id)`), so painting a face needs **zero new texture layers** — important
given the native texture-array layer-budget caution (`project_player_cosmetics_plan` memory,
cosmetics-opportunities note 2026-06-02).

**Storage:** a new sparse side-table on `World`, keyed by block position, holding one
optional wallpaper id per face. It is **persisted** (it has no block to rebuild from — see
the key decision below) by mirroring the `block_entities`/`SavedCampfire` serialise pattern
(`save.rs:176-194`), **not** the `salt_licks` derived-cache pattern.

**Interaction:** a paint branch in the place handler (modelled on the Blueprint-Paper
capture-art branch, `game_loop.rs:5698-5749`) and a two-stage peel + clear-on-destroy in
the break handler (`game_loop.rs:3608` guard + both commit arms). All client-side; the
server is an unchanged BlockChange validator.

---

## Key architecture decision — per-chunk geometry, NOT a per-player buffer

The owner-inbox note and a first reading point at "a decal pass modelled on the crack
overlay" + "per-player `decal_buffer` mirroring `crack_buffer`". **We deliberately do not
store the geometry per-player.** Five-agent code research (2026-06-03) surfaced the reason:

- The **crack overlay** is per-player and **transient** — it is the *one* block the player
  is actively mining, rebuilt **every frame** from `breaking_pos`. 36 verts, fine.
- A **wallpaper overlay is persistent world geometry**: identical from every viewport,
  changing only when a face is painted/peeled, and there may be a whole base's worth of it.
  Re-emitting all of it per frame per viewport (the crack pattern) is the one thing the
  `#18` micro-model research also flags as the cardinal sin — wasteful and unscalable.

So we use the **per-chunk mesh pattern the engine already uses for plants**
(`PlantInstance` / `GpuPlantInstances`, built in `build_chunk_meshes`, uploaded via
`upload_chunk`, rebuilt only on the chunk dirty gate). This rides **all** existing
plumbing for free: chunk streaming load/unload, the `dirty_mesh_chunks` gate +
`rebuild_chunk_at` + budgeted `process_dirty_meshes`, frustum culling (`chunk_aabb`), and
lighting remesh. We **keep the goal's render technique** (single-face depth-biased
alpha-blended quad, crack-pipeline config) and only change *where the geometry lives* to
the correct persistent lifetime. This is the "build for the moon / reads like the
surrounding code" choice per `CLAUDE.md`. Cost: more than the ~250 LOC the inbox estimated
(~400-450 incl. tests), but it is the concrete, scalable shape.

(If a future feature needs a *transient per-player* face decal — e.g. a paint preview under
the crosshair — that is the moment to add a `decal_buffer` mirroring `crack_buffer`; it is
not needed for persistent wallpaper.)

---

## Context pointers (verified against the tree 2026-06-03)

**Storage / serialisation (mirror this):**
- `World.block_entities: AHashMap<(i32,i32,i32), BlockEntityData>` — `world.rs:178`. THE
  serialise-mirror sparse map. Accessors `insert_campfire`/`campfire_at`/`iter_campfires`/
  `remove_block_entity` at `world.rs:439-478, 587-591`.
- `World.salt_licks: AHashSet<...>` (`world.rs:270`) + `tapped_rubber_logs` (`world.rs:277`)
  are **derived caches**, `#[serde(skip)]`, rebuilt from a block scan on load
  (`rebuild_salt_lick_index` world.rs:334). **Do NOT model wallpaper on these** — a face
  overlay has no block to rebuild from.
- `SavedCampfire { x, y, z, data }` — `save.rs:176-183`; `WorldSave` block-entity Vec fields
  each `#[serde(default)]` (`save.rs:60-116`); struct tail at `save.rs:155-157`.
- Save loops at **three** sites: native `save_world` (`save.rs:660-663` build, `:726` field),
  WASM `save_world` (`:823-826`, `:882`), native `autosave_world` (`:1608-1611`, `:1667`).
  Load loop at native `load_world` only (`save.rs:1014-1016`; WASM `load_world` is a stub,
  `save.rs:1132-1138`). Forward-compat test `missing_block_entity_vecs_deserialise_as_empty`
  (`save.rs:2616`).

**Render (reuse the technique):**
- `make_crack_pipeline` — `renderer.rs:3097-3143`: chunk pipeline layout (camera grp0 +
  texture grp1), `vs_main` + `fs_crack`, `ALPHA_BLENDING`, `cull_mode:None`,
  `depth_write_enabled:false`, `depth_compare:LessEqual`, `DepthBiasState{constant:-2,
  slope_scale:-1.0}`.
- `build_crack_cube` — `renderer.rs:3149-3186`: the six per-face CCW outward windings to
  lift one-at-a-time. `e = 0.003` outward inflate.
- `fs_crack` — `shader.wgsl:239-241`: raw unlit texture-array sample. The decal wants this
  **plus** a `*in.light` multiply (and an `a<0.5` discard for future translucent wallpaper).
- Plant per-chunk pattern: `ChunkMeshes` (`mesh.rs:146-150`), `build_chunk_meshes`
  (`mesh.rs:188-243`), `emit_non_solid_blocks` (`mesh.rs:326-405`, the model for
  `emit_face_decals`), `upload_plant_instances` (`renderer.rs:1011-1034`), `upload_chunk`
  (`renderer.rs:1036-1043`), plant draw pass (`renderer.rs:2461-2501`), `plant_meshes`
  AHashMap (`renderer.rs:183-212`). `mesh::Vertex` (`mesh.rs:18-50`).

**Faces / raycast:**
- `mesh::Face { Top,Bottom,North,South,East,West }` with `normal()`/`offset()` —
  `mesh.rs:152-185` (currently private). Iterated in the order
  `[Top,Bottom,North,South,East,West]` (`mesh.rs:209`) — **index order is load-bearing**.
- `RayHit.face_normal: [i32;3]` (`raycast.rs:46`), pointing back toward the eye
  (`raycast.rs:122-137`), stored each frame into `PlayerSlot.target_face` (`player_slot.rs:39`,
  set `game_loop.rs:3131`).

**Interaction:**
- Place flow + the precedent face-intercept branch (Blueprint-Paper capture-art) —
  `game_loop.rs:5696-5813` (generic place at 5751+, capture-art branch 5698-5749).
- Break handler: shared anti-grief guard `if !..._blocked && blk != BEDROCK {` ~`game_loop.rs:3608`;
  creative commit arm `3622-3744`; survival commit arm `3745-3995`; side-table clears at
  `3693-3695` (creative) and `3826-3830` (survival).
- Items: `Item::Block(BlockId)` (`item.rs:508-524`); consume helpers
  `take_one_from_hotbar` (`inventory.rs:214`), `take_placeable_from_hotbar` (`inventory.rs:179`);
  ground-item spawn `entity::spawn_item` (drop-when-full model `game_loop.rs:670-677`).

---

## Phased scope

### Phase A — storage + face plumbing

1. **`mesh::Face` → `pub`** (`mesh.rs:154`) and add, keeping the discriminant order stable:
   - `pub fn from_normal(n: [i32;3]) -> Option<Face>` — matches the six unit normals.
   - `pub fn index(self) -> usize` — stable `0..6` (`Top=0 … West=5`), the overlay-array slot.
   `from_normal([0,0,0])` → `None` (no target).
2. **`OverlayData`** — minimal, extensible value (`world.rs`, beside `BlockEntityData`):
   `#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)] pub struct OverlayData
   { pub block: BlockId }` (the wallpaper block id; texture layer derived at mesh time via
   `registry.tex_side(block)`; recovered item is `Item::Block(block)` — byte-identical
   round-trip, no new item type).
3. **`World.face_overlays: AHashMap<(i32,i32,i32), [Option<OverlayData>; 6]>`** — declare
   beside `block_entities` (`world.rs:178`), init in `World::new` (`world.rs:~325`). Helpers:
   `set_face_overlay(pos, face_idx, data)`, `face_overlay_at(pos, face_idx) -> Option<OverlayData>`,
   `remove_face_overlay(pos, face_idx)` (one face), `remove_face_overlays_at(pos) -> [Option<OverlayData>;6]`
   (whole entry, returns what was there for drop), `iter_face_overlays()`.
4. **`block::is_wallpaper(id) -> bool`** (`block.rs`, beside `is_pure_deepslate_family`):
   `(145..=157).contains(&id) || (159..=161).contains(&id)` — **must skip 158 (LATENT_PRINT)**.

**Acceptance A:** unit tests — `Face::from_normal`/`index` round-trip (lock the index map);
`is_wallpaper` true for all 16 ids, false for 158/144/162; overlay set/get/remove + whole-cell
clear returns the painted set.

### Phase B — serialisation

5. **`SavedFaceOverlay { x, y, z, face: u8, block: BlockId }`** — `save.rs` beside
   `SavedFurnace`. `face` = `Face::index()`. (Flat per-face entries, matching every existing
   `Saved*`; the runtime `[_;6]` array expands to per-face entries on save.)
6. **`#[serde(default)] pub face_overlays: Vec<SavedFaceOverlay>`** on `WorldSave` (before the
   struct close, `save.rs:156`).
7. **Save** at all three construction sites: build
   `let face_overlays = world.iter_face_overlays().flat_map(|(pos, faces)| faces.iter().enumerate()
   .filter_map(...).map(|(fi, d)| SavedFaceOverlay{ x:pos.0, y:pos.1, z:pos.2, face:fi as u8, block:d.block }))
   .collect()` + add the field. **Load** (native `load_world` only): `for fo in &save.face_overlays
   { world.set_face_overlay((fo.x,fo.y,fo.z), fo.face as usize, OverlayData{block: fo.block}); }`.
8. Add a `face_overlays.is_empty()` assertion to `missing_block_entity_vecs_deserialise_as_empty`
   + a save→load round-trip test (model: the every-block-entity round-trip test).

**Acceptance B:** round-trip test paints two faces of a block, saves, loads, asserts both
overlays restored with the right ids; legacy-save test (no field) loads with empty overlays.

### Phase C — render (per-chunk decal geometry)

9. **`ChunkMeshes.decals: Vec<Vertex>`** (`mesh.rs:146-150`); include `decals: vec![]` in the
   early-return empty (`mesh.rs:198`) and the final return (`mesh.rs:242`).
10. **`emit_face_decals(chunk, origin_{x,y,z}, world, registry, &mut decals)`** (`mesh.rs`,
    modelled on `emit_non_solid_blocks`): for each `world.face_overlays` entry whose block
    position is in this chunk, for each `Some(face)`: emit **one** quad via `emit_decal_quad`
    — corners lifted from the matching `build_crack_cube` face, inflated `e=0.003` outward
    along the face normal, uv `0..1`, `tex_layer = registry.tex_side(data.block)`, `light`
    sampled from the **adjacent** cell (`pos + Face::offset()`) so the paper is lit by the
    room, not the inside of the wall (fall back to the block's own cell / sky light when the
    neighbour is cross-chunk-unavailable). Call it at the end of `build_chunk_meshes`.
11. **`fs_decal`** (`shader.wgsl`): `let c = textureSample(...); if (c.a < 0.5) { discard; }
    return vec4(c.rgb * in.light, c.a);` — lit + alpha-cutout.
12. **`decal_pipeline`** — generalise `make_crack_pipeline` into `make_overlay_pipeline(...,
    fs_entry, label)` (or a sibling); decal uses `fs_decal`, otherwise crack config verbatim.
13. **`decal_meshes: AHashMap<(i32,i32,i32), GpuDecalMesh{ buffer, vertex_count }>`** on
    `Renderer` (beside `plant_meshes`); init in both constructors. `upload_chunk_decals(pos,
    &[Vertex])` (empty → remove key, else `create_buffer_init` VERTEX), called from
    `upload_chunk`. **Decal draw pass** after the opaque chunk pass, cloned from the plant
    pass: `LoadOp::Load` colour + read-only depth, `decal_pipeline`, bind camera(0)+texture(1),
    iterate `decal_meshes` with the same `chunk_aabb` frustum cull, `draw(0..vertex_count)`.

**Acceptance C:** `emit_decal_quad` unit test (6 verts, correct normal, outward offset per
face). `check.sh` native + WASM build green. Visual = playtest.

### Phase D — interaction (paint, peel, clear-on-destroy)

14. **Paint branch** in the place handler, immediately after the capture-art branch (before
    the generic `place_x = pos+face`, `game_loop.rs:~5751`): if the held hotbar item is
    `Item::Block(b)` with `block::is_wallpaper(b)` **and** the targeted block at `pos`
    (`target_block`) is solid (`registry.is_solid`), then `world.set_face_overlay(pos,
    Face::from_normal(target_face)?.index(), OverlayData{ block: b })`; consume one item in
    survival (`take_one_from_hotbar`), none in creative (mirror the `is_creative` split at
    5803-5807); `audio.play_place()`; `place_cooldown = 8`; `rebuild_chunk_at(pos…)`;
    `continue`. The generic solid-place path is untouched (right-clicking still places a solid
    wallpaper block in the adjacent air gap — preserved as the fallthrough when the targeted
    face already carries that overlay, or when not holding wallpaper).
15. **Two-stage peel** as the first action inside the `!..._blocked && blk != BEDROCK` guard
    (`game_loop.rs:~3608`), before the creative/survival split: let `fi =
    Face::from_normal(target_face).map(Face::index)`; if `Some(fi)` and
    `world.face_overlay_at(pos, fi)` is `Some(d)` → peel only: `remove_face_overlay(pos, fi)`,
    give back `Item::Block(d.block)` (inventory, else `spawn_item`), `rebuild_chunk_at`,
    `play_break`, reset `breaking_pos=None` + `break_progress=0` so the **next** swing is
    strike 2, and skip the break (no `BlockChange`). Bare face → fall through unchanged.
16. **Clear-on-destroy** in **both** commit arms (creative `~3693`, survival `~3826`): after
    the block is set, `for d in world.remove_face_overlays_at(pos) { if let Some(d)=d { give
    Item::Block(d.block) } }` so every painted face is recovered when the block is destroyed.

**Acceptance D:** TestHost integration (if the harness supports place/break intents) — paint a
face → overlay present; peel → overlay gone + item back, block intact; second strike →
block breaks; break a block with 2 painted faces → all overlays gone + 2 items recovered. Else
pure-function tests on the peel/clear helpers + a manual test-sheet step.

---

## Out of scope / deferred

- **Blueprint-as-a-face-attribute (Phase C of the inbox note).** Explicitly dropped — the
  cell-based blueprint (Specs 24/38) is shipped + unplaytested; do not rewrite on spec.
- **Multiplayer overlay sync.** A peel emits no `BlockChange` (`protocol.rs:239-246` has no
  face/overlay field) and paint isn't a block change, so a second client won't see overlays.
  Matches the project's "networked two browsers is platform-blocked" posture — defer behind
  Spec 1 Phase 4; a new protocol delta, not an overloaded `BlockChange`, is the eventual fix.
- **Distance LOD / impostors** for decals — not needed at wallpaper scale; the `#18`
  micro-model doc owns the general LOD story.
- **Translucent / paper-edge wallpaper textures** — `fs_decal` already discards `a<0.5`, so
  authoring transparent `gen_wallpaper_overlay` layers later is a drop-in; not needed for v1
  (existing `TEX_WALLPAPER_*` are opaque, which fully covers the painted face).
- **Re-papering a covered face, hard-block overlay nuances, recovered-drop form tuning** —
  open gameplay calls, deferred per the inbox note.

## Risk / confidence

**Medium-high.** Storage + serialisation + render all mirror proven, shipped patterns
(`block_entities`, plants, crack overlay) with exact anchors. The two real risks: (1) the
**three save sites + two break arms** must each be touched — a miss silently drops overlays
or leaks them (locked by the round-trip test + a clear-on-destroy test); (2) decal **lighting**
across chunk boundaries (sampling the adjacent cell) — falls back to sky/full-bright safely.
No new texture layers (sidesteps the native layer-budget caution). Visual/feel is the only
non-solo-verifiable part → playtest gate.

## Memory-rule check

- `feedback_merge_to_main_preauthorised` — merge when `check.sh` is ALL GREEN, no ask.
- `project_player_cosmetics_plan` / cosmetics-opportunities note — native texture-array layer
  budget caution; this build adds **zero** layers (reuses `TEX_WALLPAPER_*`).
- `feedback_uk_english_naming` — UK English throughout ("colour", "neighbour").
- `feedback_autonomy_to_playtest_boundary` — build to the playtest boundary, stop cleanly.
- Spec maintenance (`CLAUDE.md`) — fold the overlay path into Spec 05 (block interaction) +
  Spec 03 (rendering) once shipped.

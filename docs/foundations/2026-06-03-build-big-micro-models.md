# Community Asset Authoring — build-big in-game, bake to a sub-voxel micro-model

**Status: ✅ DELIVERED 2026-06-03 — Phases A–D merged to `main`.** Flowers
(CORNFLOWER/FIELD_POPPY/BUTTERCUP) now render as 3D sub-voxel shells instead of flat
cross-billboards; test sheet `docs/test-sheets/2026-06-03-flower-micro-models.md`
(Axolittle playtest = open gate — the *look* is the only piece not solo-verifiable,
though it was headlessly rendered + confirmed). Graduated from owner-inbox item **#18**
(`docs/backlog/owner-inbox.md` §#18). This is the concrete, buildable expansion of that
note: the bake-to-micro-model pipeline designed against the **real** chunk/mesh/capture
format with exact `file:line` anchors.

**As-built deltas vs this design** (folded back per CLAUDE.md spec-maintenance): bake lives
in `micro_model.rs`, not `mesh.rs` (no-god-files — mesh.rs was already ~1300 lines), and
replicates `mesh::greedy_face` since it's module-private; built-in flowers are **procedural
Rust builders** (`micro_model_assets.rs`), not committed JSON, because the in-engine
build→capture→bake authoring needs an interactive GPU session — the JSON loader + registry
remain the path for community/Stash assets. `MicroInstance` instance attrs sit at vertex
**locations 5–8** (the shell `Vertex` uses 0–4). A single-voxel bake is 24 verts / 36 indices
(a cube), not the "12 indices" first written here.

**Hardening pass (2026-06-03, post-ship audit)** fixed: (1) the flower **centre eye** was
fully buried inside the bloom → culled to zero faces; the centre now pokes one cell above the
bloom top so it bakes (guarded by `flower_centre_is_visible`). (2) Phase-D far-LOD now emits a
**dedicated inset billboard** (`ChunkMeshes.micro_billboards`, the block's original
`non_solid_shape_for` cross) instead of reusing the shell buffer as a full-block cross — distant
flowers keep their footprint. (3) `bake_micro_model` refuses scales other than 8/16 (DoS guard
on the Stash path). (4) registering a *solid* block warns (no hook on the solid path yet). Plus
a flower save/load round-trip test and a `content_hash` doc correction (no cache exists yet).
Deferred micro-opt: Back-face culling on the micro pipeline (winding is verified, but the perf
win is negligible for flowers and `cull_mode: None` is two-sided-safe).

The do-ocracy curation / asset-show / competition governance + the in-engine creator UI are
**explicitly deferred** (owner: "good problem to have, not now") and are **not** designed in
this doc.

**Trigger / the prize:** some static assets — flowers first, then decorative blocks,
plant/leaf looks — "need polish", but that polish is **3D work that can't be written
in text; it has to be built in 3D**. Rather than ship a separate modelling tool
(Blender, Blockbench), **the game itself becomes the asset editor**: a creator builds
a giant version of the prop out of ordinary blocks in creative, captures it with the
build-schematics system we already shipped, and the engine **bakes** that captured
volume down into a compact sub-voxel **micro-model** that renders in place of the
block's default cube or cross-billboard. The engine ships the **tool**; the community
authors the **assets**. This is the structural fix for "flowers look like flat 2D
sprites" — flowers upgrade **first**.

**Branches:** fork off `main` → `feat/micro-model-core` (Phase A) → `feat/micro-model-render`
(Phase B) → `feat/flower-micro-models` (Phase C) → `feat/micro-model-lod` (Phase D).
Each phase is independently shippable + `check.sh`-gated; the visual confirmation is a
playtest-boundary check the owner runs.

---

## TL;DR

Flowers, grass, crops and fibre plants render today as **cross-billboards** — two flat
quads crossed in an X, painted with a 16×16 see-through sprite (`mesh.rs::plant_unit_cross`,
lines 121-141; the species' AABB comes from `non_solid_shape_for`, `mesh.rs:262`). They
are flagged `solid:false, transparent:true` (`block.rs:2079-2096`) so the greedy mesher
skips them and the non-solid pass emits them as instanced billboards. You can see straight
through the gaps — they are **2D sprites, not 3D objects**. That is the "spaces in the
centre" the owner reported.

The fix reuses three things we already own end-to-end:

1. **Cell-based capture** — `plan::capture()` (`plan.rs:544`) already grabs a bounded
   built volume into `PlanData { width, depth, height, cells: Vec<CapturedCell> }`
   (`plan.rs:148-205`), each `CapturedCell { rx, ry, rz: u8, block_id }`. **No new
   capture tool is needed.**
2. **The greedy chunk mesher** — `build_chunk_meshes` / `greedy_face` (`mesh.rs:188-243`)
   already turns a 16³ block grid into a merged, interior-culled shell mesh.
3. **Instanced draw** — a chunk's plants are drawn with **one** instanced draw against
   shared geometry (`renderer.rs:2498`, `pass.draw_indexed(0..plant_geo_index_count, 0, 0..plants.count)`),
   the same cost class we want for micro-models.

The pipeline: **capture a built volume → reinterpret it at sub-block scale (1/8 or 1/16
per asset) → bake ONCE per asset type into a greedy-meshed compact shell mesh (interior
micro-voxels culled) → INSTANCE per placement (one draw per type) → a `block_id → micro-model`
override table makes a registered block render its micro-model instead of its default
cube/cross → LOD back to a billboard/impostor at distance.** The **1/16** grid lines up
exactly with the 16×16 texture resolution (`texture_gen.rs:7`, `const SIZE: u32 = 16`), so
the colour comes from the blocks you built with — **paint-with-blocks**.

**The one failure mode** is drawing raw micro-cubes (4096 cubes × N flowers → billions of
triangles). The whole design exists to **never do that**: bake + cull + instance, always.

**Scope: static props only.** Animated assets (mobs, swaying plants, machines) need
rigging — that is owner-inbox item **#19**, a separate future foundation doc, and is
**out of scope** here.

---

## Context pointers (verified against the tree 2026-06-03)

- **Design authority:** `docs/backlog/owner-inbox.md` §#18 — the owner+Claude design
  conversation this doc expands. Shares the schematic lineage with §#1/#2/#3 (paper-thin
  surface) and §#19 (animated authoring). The do-ocracy framing sits in
  `docs/vision/economies-long-run.md` §9 (Knowledge & Creator economy) + §9.4 (custom
  textures/resource packs).
- **Capture (reused wholesale):**
  - `CapturedCell { rx, ry, rz: u8, block_id: BlockId }` — `plan.rs:148-153`.
  - `PlanData { version, name, width: u8, depth: u8, height: u8, cells: Vec<CapturedCell>, kind: PlanKind, … }` — `plan.rs:157-205`. Only non-air cells are stored.
  - `pub fn capture(world, click_pos, authored_in) -> Result<PlanCaptureCandidate, CaptureRefusal>` — `plan.rs:544-610`. 3D flood-fill above blueprint-paper tiles; computes the relative-coord cells at `plan.rs:579-587`.
  - `pub fn capture_art(...)` — `plan.rs:633` — the 2D wall-slice sibling (Art-kind plans, `depth=1`).
  - `flood_fill_volume` (`plan.rs:487-539`) — the 6-connected shell capture.
  - Envelope guards: `MAX_FOOTPRINT = 32`, `MAX_HEIGHT = 32` (`plan.rs:427-428`).
  - `PlanRegistry` — content-hash-keyed registry of plans loaded from `assets/registered_plans/*.plan.json` via `include_str!` (`plan_registry.rs:141`, asset list at `plan_registry.rs:266+`). The model for a built-in micro-model asset pack.
- **Chunk + block storage:**
  - 16³ sub-chunks, flat `[BlockId; CHUNK_VOLUME]`, index `x + z*16 + y*256` (`chunk.rs:9-39`). `CHUNK_SIZE = 16`, `CHUNK_VOLUME = 4096`.
  - `BlockDef { name, solid, transparent, gravity, color: [f32;3], tex_top, tex_bottom, tex_side }` — `block.rs:1111-1124`. A block id selects its render path via `solid`/`transparent`: opaque-solid → greedy mesh; non-solid+transparent → `emit_non_solid_blocks`.
  - Flowers (CORNFLOWER 131 / FIELD_POPPY 132 / BUTTERCUP 133) declared `solid:false, transparent:true` at `block.rs:2079-2096`. Registry predicates `is_solid` (`block.rs:2585`), `is_transparent` (`block.rs:2589`), `tex_side` (`block.rs:2608`).
- **Mesher + plant render:**
  - `build_chunk_meshes(cx,cy,cz,world,registry) -> ChunkMeshes { opaque, water, plants }` — `mesh.rs:188-243`. `greedy_face` runs the 6-direction merge (`mesh.rs:209-221`).
  - `plant_unit_cross()` — the shared two-quad X-billboard, **8 verts / 12 indices**, `cull_mode: None`, drawn from every direction (`mesh.rs:121-141`).
  - `PlantInstance { pos: [f32;3], size: [f32;3], tex_layer: u32, light: f32 }` — the **32-byte** per-plant record (`mesh.rs:66-91`); `emit_non_solid_blocks` pushes one per plant cell (`mesh.rs:389-401`).
  - `non_solid_shape_for(block_id)` — per-species AABB; flowers are `([0.30,0,0.30],[0.70,0.65,0.70])` (`mesh.rs:262-263`).
  - `Vertex { position, normal, tex_layer: u32, uv: [f32;2], light: f32 }` — 44-byte chunk-mesh vertex (`mesh.rs:18-26`).
  - `emit_small_cube(min,max,tex_layer,light,…)` — the 6-faced baked-cube helper used by TORCH/SMOKE (`mesh.rs:410-479`); the **proof that sub-block baked geometry already flows through the chunk mesh**.
- **Renderer:**
  - `GpuChunkMesh { vertex_buffer, index_buffer, index_count }` and `GpuPlantInstances { buffer, count }` (`renderer.rs:14-25`).
  - `create_plant_pipeline` — vertex buffer 0 = shared geometry, buffer 1 = per-instance data, `vs_plant`/`fs_plant`, `cull_mode: None`, depth-write on (`renderer.rs:30-75`).
  - `create_plant_geo` — uploads `plant_unit_cross` once into shared buffers (`renderer.rs:79-93`).
  - `upload_chunk(pos, &ChunkMeshes)` — single entry point: opaque + water + plants (`renderer.rs:1039-1042`); `upload_plant_instances` (`renderer.rs:1011`).
  - **The instanced plant pass** — frustum-culled per chunk, **one draw per chunk-of-plants**: `renderer.rs:2491-2500`, the load-bearing line being `pass.draw_indexed(0..plant_geo_index_count, 0, 0..plants.count)` (`renderer.rs:2498`). This pass is **already** frustum-tested (`renderer.rs:2493`) — micro-models inherit that for free.
  - `upload_chunk` call sites: `chunk_stream.rs:102,115,316,357` + `game_loop.rs:975,1037,1490,8952` (both the single-player client path and the server-driven path go through the same call).
- **Shaders:** `block_textures: texture_2d_array<f32>` (`shader.wgsl:20`); `vs_plant`/`fs_plant` (`shader.wgsl:79-113`) with alpha-cutout `if (tex_color.a < 0.5) { discard; }` (`shader.wgsl:97`); chunk `vs_main`/`fs_main` (`shader.wgsl:53,131`).
- **Texture resolution:** `const SIZE: u32 = 16` (`texture_gen.rs:7`), 16×16 RGBA per layer, uploaded as a 2D texture array. This is why a **1/16 authoring grid lines up exactly** — one micro-voxel ↔ one source texel.

Spec cross-refs: Spec 03 (`docs/spec/03-rendering.md`) — greedy meshing §2.3, instancing,
LOD §5.4 (currently unimplemented; this work adds a first concrete LOD consumer). Spec 02
(`docs/spec/02-world-format.md`) — chunk format. Build-schematics: Specs 24/38 +
`docs/vision/build-schematics-long-run.md` (the capture system this reuses). The
build-schematics review note in the inbox (2026-06-03) confirms #18 **aligns with** the
shipped cell-based capture rather than fighting it (unlike #1/#2/#3's blueprint half).

---

## The problem (precisely)

A flower is one block id. Its `BlockDef` is `solid:false, transparent:true`
(`block.rs:2081`). At mesh-build time the greedy mesher skips it (it only meshes solid
or opaque faces), and `emit_non_solid_blocks` (`mesh.rs:326-405`) routes it — because it
is **not** TORCH/SMOKE (`mesh.rs:386`) — into the `plants` vector as a single
`PlantInstance`. The renderer then draws every plant in the chunk against the shared
two-quad cross (`mesh.rs:121`, `plant_unit_cross`), alpha-cut by `fs_plant`
(`shader.wgsl:92-99`).

That cross is **8 vertices**. Viewed off-axis you see the X; viewed along a quad's plane
you see a thin sliver; you can see **through** the centre. There is no volume, no petals,
no stem geometry. The art is doing all the work and the art is a flat 16×16 sprite.

Two things follow:

1. **No amount of texture work makes a billboard read as 3D.** The polish the owner wants
   is *geometry*, and geometry of organic shapes (a flower head, layered petals, a curved
   stem) is exactly the thing you cannot describe in a data file by hand — you have to
   **build** it.
2. **We already have the machinery to build-and-bake.** `emit_small_cube` (`mesh.rs:410`)
   proves sub-block baked geometry flows through the chunk mesh today (torch, smoke). The
   capture system proves we can grab a bounded built volume. Nothing in the pipeline below
   is new *technology* — it is new *plumbing* between systems we own.

The naive thing — let a block render its captured volume as literal sub-cubes — would be
catastrophic: a 1/16 flower is 16³ = 4096 cells; a field of 1000 flowers naively drawn is
~4 billion cube-faces. **The entire design is the discipline that avoids this**: bake the
shell once, cull the interior, instance per placement, LOD at distance.

---

## Design — the bake-to-micro-model pipeline

Six concrete steps, each grounded in the real format.

### D1. Build big (creative) and capture — reuse the schematic capture verbatim

The author builds the giant prop out of ordinary blocks in creative, lays blueprint paper,
and captures exactly as today (`plan.rs:544` `capture`, or `plan.rs:633` `capture_art` for a
flat wall-slice). The output is a `PlanData` with `cells: Vec<CapturedCell>` and a
`width × depth × height` envelope (≤ 32 each, `plan.rs:427`). **No capture-path change.**
A "giant flower" built at 16× scale is, say, a 16×16×16 build that fits comfortably inside
the 32-cube envelope.

The micro-model authoring entry point is a **new derive step on an existing developed
Plan**, not a new capture flow: "Bake this plan as a micro-model for block X at 1/N scale."

### D2. Reinterpret at sub-block scale — a `MicroModelData` type (Phase A)

A captured volume is just a relative-coordinate point cloud of `(rx, ry, rz, block_id)`.
"Bake to micro-model" reinterprets that volume as occupying **one** block's footprint at a
finer grid:

- **1/8** → an 8×8×8 micro-grid (512 cells max) — cheaper, for chunky props.
- **1/16** → a 16×16×16 micro-grid (4096 cells max) — fine detail; **lines up 1:1 with the
  16×16 texture resolution** (`texture_gen.rs:7`) so one micro-voxel maps to one source
  texel. Recommended for flowers and fine décor.

The scale is **per-asset** (a field on the micro-model), chosen by the author. The captured
`width/depth/height` is rescaled to fit the chosen micro-grid (downsample-by-majority, or
require the author to have built at exactly N³ and reject otherwise — see open questions). A
micro-voxel inherits its **colour/texture** from the source block it came from — `tex_side`
of `block_id` (`block.rs:2608`) — which is the **paint-with-blocks** mechanic.

New type (Phase A, in a new `micro_model.rs`):

```
/// A baked sub-voxel static prop. `scale` is the micro-grid resolution
/// (8 or 16); a micro-voxel at (mx,my,mz) sits at world offset
/// (mx,my,mz)/scale inside the host block's unit cube. Only occupied
/// micro-voxels are stored. Wire-stable / serde — this is shareable
/// content (Stash, registry), so version it like PlanData.
struct MicroModelData {
    version: u8,
    scale: u8,                 // 8 or 16
    voxels: Vec<MicroVoxel>,   // occupied cells only
    // provenance for do-ocracy attribution (mirrors PlanData)
    author_npub: String,
    derivation_chain: Vec<DerivationLink>,  // reuse plan::DerivationLink
}
struct MicroVoxel { mx: u8, my: u8, mz: u8, block_id: BlockId }
```

A pure `from_plan(plan: &PlanData, scale: u8) -> Result<MicroModelData, BakeRefusal>`
converts a captured `PlanData` into a `MicroModelData` (rescale + occupancy). **Pure,
unit-testable, no GPU, no `World`.**

### D3. Bake ONCE per asset type into a compact greedy-meshed shell (Phase A)

This is the crux. A `MicroModelData` is meshed **exactly like a chunk** — reuse the greedy
mesher's machinery rather than reinventing it:

- The micro-grid is a `scale³` occupancy grid — structurally identical to a sub-chunk
  (`chunk.rs`), just `scale` instead of 16. Build a transient occupancy buffer and run the
  same 6-direction greedy face-merge logic that `greedy_face` (`mesh.rs:209`) runs on a
  chunk.
- **Interior micro-voxels are culled** — the mesher only emits a face when the neighbour
  micro-voxel is empty (the same adjacency test `greedy_face` already does). A solid
  interior contributes **zero** triangles; only the shell survives.
- Faces are emitted at `1/scale` world size, offset into the unit cube, in the existing
  `Vertex` format (`mesh.rs:18`) so the output is a bog-standard `ChunkMesh`
  (`mesh.rs:53`) the renderer already knows how to upload.
- **Bake once, cache by content hash.** The baked mesh is keyed by the micro-model's content
  hash (mirror `plan::content_hash`, `plan.rs:270`) so identical assets share one baked mesh.

A pure `bake_micro_model(&MicroModelData, &BlockRegistry) -> ChunkMesh` (in `micro_model.rs`,
alongside `from_plan` — **not** `mesh.rs`, which is already ~1300 lines, per the CLAUDE.md
no-god-files rule; it *replicates* `mesh::greedy_face`'s two-phase merge because `Face` and
`greedy_face` are module-private) is the deliverable. **A flower bakes to ~100–400 triangles**
(a few petals + a stem shell) versus a billboard's 4 — and that cost is paid **once per
asset type**, not per placement.

Acceptance maths to enforce in tests: a fully-solid `scale³` micro-model bakes to exactly
the **outer shell** (6 × scale² faces at most after merge, far fewer in practice), never
`scale³` cubes.

### D4. INSTANCE per placement — never draw raw micro-cubes (Phase B)

A placed micro-model prop must cost the **same class** as a billboard does today: one record
per placement, **one draw per type per chunk**. This mirrors the existing plant path exactly:

- The baked `ChunkMesh` for a micro-model type is its **shared geometry** (analogous to
  `plant_geo_vbuf`/`plant_geo_ibuf` uploaded once by `create_plant_geo`, `renderer.rs:79`).
- Each placement contributes a per-instance record — reuse/extend `PlantInstance`
  (`mesh.rs:66`): `{ pos, size, tex_layer, light }`. For paint-with-blocks colouring the
  texture is baked **into** the shared mesh's per-vertex `tex_layer`, so the per-instance
  record can be even smaller (pos + light); start by reusing `PlantInstance` to minimise new
  surface.
- A new instanced pass (mirror `create_plant_pipeline`, `renderer.rs:30`, and the plant pass
  at `renderer.rs:2457-2501`) draws all instances of one micro-model type against its shared
  baked geometry with **one** `draw_indexed(0..type_index_count, 0, 0..instances.count)` —
  the micro-model analogue of `renderer.rs:2498`. It is **already inside a frustum-culled
  loop** (`renderer.rs:2493`), so distant/off-screen instances cost nothing.

**Cost sanity (from the inbox note, now grounded):** 1000 flowers × ~250 tris = ~250k
tris/frame — trivial — at **unchanged draw-call count** (one instanced draw per type per
chunk, exactly like plants today). Bonus: opaque micro-cubes can use depth-write + no
alpha-blend (the plant pipeline already uses `BlendState::REPLACE`, `renderer.rs:54`, with
alpha-cutout in `fs_plant`), **avoiding the alpha-blend overdraw billboards pay**.

> **Hard rule, restate in code comments:** the only failure mode is naively drawing
> `scale³` cubes per placement. The pipeline **always** bakes (D3) + instances (D4). There
> is never a path that emits raw micro-cubes per placement.

### D5. `block_id → micro-model` override table (Phase B)

A registered block renders its micro-model **instead of** its default cube/cross. This is
the plug-in point and it is small:

- A `MicroModelRegistry` (mirror `PlanRegistry`, `plan_registry.rs:141`): `AHashMap<BlockId, BakedMicroModel>`
  where `BakedMicroModel` holds the baked `ChunkMesh` + its uploaded shared GPU buffers + the
  source `MicroModelData`. Built-in assets ship as `assets/micro_models/*.json` loaded via
  `include_str!` (the `plan_registry.rs:266` pattern); player/community assets load from Stash.
- In `emit_non_solid_blocks` (`mesh.rs:326`) — **before** falling through
  to billboard/cube emission, check `micro_registry.contains(block_id)`. If present, push a
  micro-model instance into `ChunkMeshes.micro_instances` instead of a `PlantInstance`.
  **As built (v1): the hook lives ONLY in the non-solid path** — flowers are non-solid, so
  this covers the shipped consumer. The solid `greedy_face` path has NO hook yet; registering
  a *solid* block logs a warning + has no visual effect (deferred to the first solid-décor
  consumer). Everything else (light sampling at `mesh.rs:364-368`,
  per-cell tint) is reused unchanged.
- `upload_chunk` (`renderer.rs:1039`) gains an `upload_micro_instances` call alongside
  `upload_plant_instances` (`renderer.rs:1011`). All eight `upload_chunk` call sites
  (`chunk_stream.rs:102,115,316,357`; `game_loop.rs:975,1037,1490,8952`) are reached through
  the single entry point — **no call-site changes**.

**Upgrade existing assets first; new placeable props later.** The override table means a
flower's block id is unchanged — only its render path flips. Saves, inventory, crafting,
crop-growth all keep working untouched.

### D6. Distance LOD back to billboards/impostors (Phase D)

Near the camera, draw the baked micro-model shell. Beyond a distance threshold, fall back to
the cheap billboard the block already has (its `tex_side`, `block.rs:2608`) — or a flat
"impostor" quad — so a horizon-spanning field of flowers never pays even the modest
micro-model tri cost at distance. The per-chunk frustum loop (`renderer.rs:2491`) is the
natural place: pick the LOD per chunk by camera distance (Spec 03 §5.4 describes LOD but it
is currently unimplemented — this is its **first concrete consumer**). Because both
representations already exist (baked mesh from D3, billboard from the current plant path),
LOD is a per-chunk **selection**, not new geometry.

This **also fixes flowers looking like 2D sprites**: up close you get the 3D micro-model;
only far away (where you can't tell anyway) do they fall back to the billboard.

---

## Why flowers go first

- They are the **reported** problem ("spaces in the centre", inbox §#18).
- They already flow through the **instanced** path (`mesh.rs:389`, `renderer.rs:2491`), so
  Phase C is a *retarget* of an existing instanced draw, not a new system.
- Three flowers share one AABB and differ only by colour/texture (`mesh.rs:262`,
  `block.rs:2079-2096`) — a clean, bounded first asset set.
- A flower is the **smallest** useful micro-model (a stem + a head), so it exercises the
  whole pipeline (capture → bake → instance → LOD) with the least authoring effort and the
  least risk.

---

## Phased scope

### Phase A — micro-model data type + bake step (pure, TDD, no GPU)

Deliverables, all pure and unit-tested:

- New `micro_model.rs`: `MicroModelData`, `MicroVoxel`, `from_plan(plan, scale)`,
  `content_hash` (mirror `plan.rs:270`).
- New `bake_micro_model(&MicroModelData, &BlockRegistry) -> ChunkMesh` in `micro_model.rs`
  (alongside `from_plan`, **not** `mesh.rs` — CLAUDE.md no-god-files rule), replicating
  `mesh::greedy_face`'s merge (which is module-private) over a `scale³` grid.
- Asset format `assets/micro_models/*.json` + a loader skeleton (the `plan_registry.rs:266`
  `include_str!` pattern), not yet wired to render.

**Acceptance:**
- `from_plan` of a known `PlanData` (e.g. `PlanData::debug_3x3_stone`, `plan.rs:212`)
  produces the expected occupied-voxel set at scale 8 and 16; out-of-envelope or empty input
  refuses with a typed `BakeRefusal`.
- `bake_micro_model` of a **fully-solid** `scale³` model emits **only the outer shell**
  (assert tri-count ≪ `scale³`, and that no interior face is present).
- `bake_micro_model` of a single occupied micro-voxel emits exactly 6 faces = 12 triangles
  (**24 vertices / 36 indices**). (A *billboard* cross is 12 indices; a baked *cube* is 36 —
  do not conflate the two counts.)
- Baked output is a valid `ChunkMesh` (`mesh.rs:53`) — vertices in `Vertex` format
  (`mesh.rs:18`), indices reference real vertices.
- Content hash is stable across reorderings of `voxels` (sort before hashing, like
  `plan::content_hash`).
- `check.sh` green (clippy clean, native + WASM build, all tests, `trunk build`, bundle < 5 MiB).

### Phase B — render path + `block_id → micro-model` override table

Deliverables:

- `MicroModelRegistry` (mirror `PlanRegistry`, `plan_registry.rs:141`) — `AHashMap<BlockId, BakedMicroModel>`.
- `MicroInstance` (extend or alias `PlantInstance`, `mesh.rs:66`) + a `micro_instances: Vec<MicroInstance>` field on `ChunkMeshes` (`mesh.rs:146`).
- Override hook in `emit_non_solid_blocks` (`mesh.rs:326`) — registry lookup **before**
  billboard fall-through (`mesh.rs:386-401`); push a `MicroInstance` when registered.
- `micro_model_pipeline` + shared-geometry upload (mirror `create_plant_pipeline`
  `renderer.rs:30` + `create_plant_geo` `renderer.rs:79`); a new instanced pass after
  chunks/before water (mirror `renderer.rs:2457-2501`), frustum-culled per chunk
  (`renderer.rs:2493`), **one draw per type per chunk**.
- `upload_micro_instances` wired into `upload_chunk` (`renderer.rs:1039`); shaders — reuse
  `vs_plant`/`fs_plant` (`shader.wgsl:79-113`) or add `vs_micro`/`fs_micro` if the
  per-instance transform differs (micro-models are not camera-facing, so a fixed model
  transform, unlike the billboard).

**Acceptance:**
- A test-only block id with a registered single-voxel micro-model renders via the micro path
  (assert the chunk's `micro_instances` is populated and `plants` is not, for that block).
- Draw-call count for a chunk full of one micro-model type is **one** (instanced), verified
  via the F3 draw-call counter (added in Spec 39, surfaced in the perf overlay).
- A block with **no** registered micro-model is unaffected (still billboard/cube).
- Saves/inventory/crop-growth unaffected (block id unchanged — override is render-only).
- `check.sh` green.

### Phase C — flower upgrade (first real consumer)

Deliverables:

- An **original** baked micro-model for CORNFLOWER / FIELD_POPPY / BUTTERCUP (131/132/133,
  `block.rs:2079-2096`), authored in-engine (build-big → capture → bake) and committed as
  `assets/micro_models/*.json` (our own art — see §IP).
- Register the three in `MicroModelRegistry` at startup.

**Acceptance:**
- In-world flowers render as 3D micro-models, not crossed billboards (visual — owner's eyes).
- Mining/placing/growing a flower still works (the crop-break path `growth.rs` and the
  flower-on-tilled fix from inbox §#16 are untouched — render-only change).
- Draw-call count for a flower field is unchanged vs the billboard baseline (instanced).
- `check.sh` green. This is the **playtest-boundary** phase: I build + ship; the owner
  confirms the 3D look.

### Phase D — distance LOD to billboards/impostors

Deliverables:

- Per-chunk (or per-instance-batch) LOD selection in the micro-model pass
  (`renderer.rs:2491` loop): near → baked shell, far → the block's existing billboard
  (`tex_side`, `block.rs:2608`) or a flat impostor. Threshold derived from the live render
  distance (`GraphicsSettings`, Spec 39) so it tracks the player's setting.
- Spec 03 §5.4 (LOD) updated to record this first concrete LOD consumer.

**Acceptance:**
- A large flower field shows micro-models near the camera and billboards past the threshold
  (visual — owner's eyes). The far billboard is the block's ORIGINAL inset cross-billboard
  (correct footprint), so the seam is a 3D-shell↔2D-sprite *style* change, not a size jump;
  a pop-free cross-fade remains deferred polish.
- The LOD's effect is observable in the F3 overlay's **draw-call count** (Spec 39) — near
  chunks issue per-type shell draws, far chunks one billboard draw — plus the visual. (A
  dedicated F3 *triangle* counter isn't wired yet; noted as a future perf-stat add.)
- `check.sh` green.

---

## Out of scope / deferred (explicit)

- **Animated / dynamic assets (rigging, pivots, deformation)** — owner-inbox **#19**, its own
  XL foundation doc. Micro-models here are **static props only** (owner agreed). Phase D's
  output is a fixed mesh; no joints, no per-frame transform beyond placement.
- **The do-ocracy curation / asset-show / competition governance** — owner: "good problem to
  have, not now." This doc designs the **pipeline**, not the social layer. When prioritised it
  gets its own doc; it will lean on `docs/vision/economies-long-run.md` §9 (Knowledge economy,
  attribution chains, optional sats prize) + §1.3 (no chance-based gambling — a flower-show is
  skill/judging, not a lottery). The **provenance plumbing** (author npub + derivation chain on
  `MicroModelData`) is included now so the governance layer is a drop-in, not a rewrite — same
  approach the cosmetics doc took with its descriptor seam.
- **Re-baking at runtime on the client** — bake is an authoring/offline step + a one-time
  load-time bake from the asset JSON; the hot path only ever **instances** a pre-baked mesh.
- **Sub-voxel collision / hitbox changes** — micro-models are **visual only**; the block's
  collision stays its unit cube (or its existing non-solid no-collision). This mirrors the
  cosmetics doc's hitbox/visual separation principle and keeps the server authoritative.
- **Editing the working cell-based blueprint into a face/voxel attribute** — that is inbox
  §#1/#2/#3's contested half and is **not** touched here; #18 *reuses* the cell-based capture
  cleanly (per the 2026-06-03 schematic-build review note).
- **Texture authoring / higher-res texture packs** — colour comes from existing 16×16 block
  textures (paint-with-blocks). A custom-texture marketplace is `economies-long-run.md` §9.4,
  separate.

---

## Risk / confidence

**Confidence: Medium-High on feasibility, Medium on authoring ergonomics.**

- **Render + bake are well-grounded.** Every piece reuses a system that exists and is on
  main: capture (`plan.rs:544`), greedy meshing (`mesh.rs:188`), sub-block baked geometry
  (`emit_small_cube`, `mesh.rs:410`), instanced per-chunk draw (`renderer.rs:2498`),
  content-hash registry (`plan_registry.rs:141`). The micro-model mesher is **a denser
  cousin of the chunk mesher**, not new tech.
- **The one real risk is the naive-cube footgun** — mitigated structurally: there is never a
  per-placement raw-cube path; bake (D3) + instance (D4) is the only route, restated in code
  comments and enforced by the Phase A shell-only tri-count test.
- **Authoring ergonomics are the soft spot.** "Build at exactly 16×, capture, bake" needs a
  clear in-engine flow and good refusal messages (wrong scale, too large, empty). The capture
  envelope is 32³ (`plan.rs:427`), comfortably larger than a 16³ build, so 1/16 props fit;
  1/8 props are tiny. Open question below on downsample-vs-exact.
- **Greedy-merge on a 16³ micro-grid** is 4096 cells — trivial at bake time; baking is
  off-hot-path (load-time / authoring), so even an unoptimised merge is fine.
- **WASM:** the bake is pure CPU + the render path mirrors the existing plant path, which
  already runs on WASM — no native-only deps, no threads. Same cross-platform posture as the
  crack-overlay and cosmetics work.
- **LOD pop (Phase D)** is the usual LOD-seam risk; mitigated by tying the threshold to render
  distance and (if needed) a short cross-fade — but that polish is deferrable.

---

## Open questions (resolve during build, not blocking)

1. **Downsample vs exact-build.** Does `from_plan` downsample an arbitrary capture to the
   target micro-grid (majority-vote per micro-cell), or require the author to have built at
   exactly `scale³` and refuse otherwise? Exact-build is simpler + predictable (recommended
   for v1); downsample is friendlier but lossy. Start exact, add downsample later.
2. **Per-instance vs baked-in colour.** Bake `tex_layer` into the shared mesh's vertices
   (one mesh per colour variant) vs carry it per-instance. Baking-in is cleaner for
   paint-with-blocks (each cornflower is the same mesh) but means one shared mesh per distinct
   colourway. For the three flowers (each a single colour) baking-in is fine.
3. **Where the author triggers the bake.** A new option on the developed-Plan UI
   (`plan_ui.rs`), or a `/give`-style debug command first (`commands/`). Debug command first
   keeps Phase A/B testable without UI.
4. **Community asset delivery.** Built-ins via `include_str!` (`plan_registry.rs:266`); player
   assets via Stash (a new `kind:"micro_model"` — the cosmetics doc's `kind` pattern). The
   override table reads from both. Stash delivery is Phase B+ and can follow the cosmetics
   precedent exactly.
5. **Light sampling for a multi-cell micro-model.** Today a plant samples one cell's light
   (`mesh.rs:364`). A micro-model occupies one host block, so sampling the host cell's light
   (as plants do) is the obvious v1; per-micro-voxel lighting is a later refinement.

---

## File-touch map

| File | Phase | What |
|------|------|------|
| `game/engine/src/micro_model.rs` (new) | A | `MicroModelData`, `MicroVoxel`, `from_plan`, `content_hash`, `BakeRefusal`, **`bake_micro_model` + the micro greedy mesher** (replicates `mesh::greedy_face`), `BundledMicroModel` + `parse_micro_model_json` loader skeleton + tests |
| `game/engine/src/mesh.rs` | B | `MicroInstance` (byte-identical to `PlantInstance`, but instance attrs at **locations 5+** — the shell `Vertex` occupies 0-4, unlike the plant `PlantGeoVertex` which uses only 0-1); `micro_instances` on `ChunkMeshes` (NB: now 5 fields — `decals` was added since drafting; two struct literals to update); override hook in `emit_non_solid_blocks` (which already takes `&World`) |
| `game/engine/src/micro_model_registry.rs` (new) | B | `MicroModelRegistry` `AHashMap<BlockId, BakedMicroModel>` (mirror `plan_registry.rs:141`); built-in loader (`include_str!` pattern :266) |
| `game/engine/assets/micro_models/*.json` (new) | A,C | Baked micro-model asset definitions (mirror `assets/registered_plans/`); the three flowers in C |
| `game/engine/src/renderer.rs` | B,D | `micro_model_pipeline` (mirror `create_plant_pipeline` :30); shared-geometry upload (mirror `create_plant_geo` :79); micro pass after chunks (mirror plant pass :2457-2501, one instanced draw :2498, frustum-culled :2493); `upload_micro_instances` into `upload_chunk` :1039; LOD selection in the per-chunk loop :2491 (D) |
| `game/engine/src/shader.wgsl` | B | reuse `vs_plant`/`fs_plant` :79-113 or add `vs_micro`/`fs_micro` (fixed model transform, not camera-facing) |
| `game/engine/src/block.rs` | C | (no def change — render-only override; flowers stay 131/132/133 :2079) |
| `game/engine/src/plan_ui.rs` / `commands/` | B (open Q3) | author trigger: "bake plan as micro-model for block X at 1/N" |
| `docs/spec/03-rendering.md` | A–D | record the micro-model bake + instanced pass; mark LOD §5.4 as having a first consumer (Phase D) |

---

## Memory-rule check

- **`project_shared_infra_strategy` (cross-game lift):** the bake step, micro-model registry,
  instanced micro-model pass, and `block_id → micro-model` override table are **engine-generic** —
  any Decented voxel game gets richer static props from the same primitive. The *assets* (which
  flower, which prop) are AxeNStax-flavoured; the *pipeline* is shared infra. Kept out of
  AxeNStax-specific assumptions. ✅
- **`project_axenstax_has_farming`:** flowers/crops/fibre plants are farming content; upgrading
  their look serves the farming economy without touching its mechanics (render-only). ✅
- **`reference_proof_of_play_is_proof_of_work` + `economies-long-run.md` §1.3:** the deferred
  do-ocracy reward (asset-show / optional sats prize) is **skill/judging-based**, not
  chance-based — consistent with the no-gambling rule. Not designed here, but flagged so the
  later governance doc inherits the constraint. ✅
- **`feedback_uk_english_naming`:** UK English throughout (colour, not color, in prose). ✅
- **`feedback_autonomy_to_playtest_boundary`:** Phases A–B and the bake are solo-buildable and
  fully `check.sh`-gated; Phases C–D end at a **visual** confirmation the owner runs (3D flowers
  look right, LOD seam is clean) — build + ship + stop at that boundary. ✅
- **`feedback_merge_to_main_preauthorised`:** merge to main on a healthy gate (`check.sh` green)
  once built — pre-authorised, scoped to decented/axenstax. ✅
- **No build yet:** this is the design/spec only, per the project rule "do NOT build anything
  unless explicitly asked." The owner says when to action it. ✅
- **No Bitcoin/Signet/economy-block surface touched** in the core pipeline; the only economy
  touch-point (the deferred reward) is explicitly out of scope. No owner-convergence concern. ✅
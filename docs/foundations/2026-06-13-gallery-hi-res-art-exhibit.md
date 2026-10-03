# The Gallery — hi-res art on low-res walls (Adventure mode)

**Status:** Built + shipped 2026-06-13. **RETIRED from the engine 2026-09-29** —
the Gallery is now an optional external world pack (a `.axeworld` of Wall
exhibits with the images inside, loaded via normal world import); `gallery.rs`,
`world_type "gallery"`, `/scenario gallery`, the embedded/static art and the web
art bridge are gone. A save still marked `world_type "gallery"` loads as a flat
world. Kept below as the historical design.

## Concept

A walk-through art exhibition you enter from the lobby, alongside Hash Dash and
Satori Rush. A simple maze of **light-coloured, 3-block-tall walls** with
**5-wide corridors** under **open sky locked to day**, explored in **Adventure
mode** (read-only — visitors can't grief). Real Bitcoin artists' work hangs on
the walls as **high-resolution paintings**. The deliberate contrast — crisp
hi-res art mounted on the chunky 16×16 voxel walls — *is* the aesthetic.

Artists (used **with permission**, owner confirmed in person 2026-06-13):
**Rebel Money** (`rebelmoney.art`), **ChiefMonkey** (`hodlr.rocks`),
**Fractal Encrypt** (`timechainartifacts.com`). 12 pieces, 4 per artist.

## Why it was cheap (and where the real work was)

The mechanics fell out of existing systems: a resolution-agnostic renderer,
Adventure mode (`PlayMode::Adventure`), the scenario-card launch lifecycle, and
the arbitrary-image load path proven by skin upload. The genuinely **new** work
was the **hi-res painting renderer** — a textured quad with its **own**
native-resolution texture, mounted on world geometry, separate from the 16×16
block texture array.

## Architecture

### `game/engine/src/gallery.rs` — the pure, deterministic core (source of truth)

Owns the maze layout **and** the painting placements so world-gen and the
renderer agree exactly. Fully unit-tested (7 tests: span, spawn-is-open,
perimeter walled, maze fully connected, one placement per artwork, paintings
hang on real walls facing corridors, registry/embedded-bytes agree).

- `build_layout()` / `layout()` — recursive-backtracker maze over a 5×5 cell
  grid (cells = 5-wide corridors, walls = 1 thick), expanded to a 31×31 block
  `solid` grid. Deterministic LCG seed → identical on every machine/run.
- Painting placement: **every** cell side whose bordering wall is solid
  (interior unconnected wall or perimeter) gets a painting — the maze is hung
  densely, not sparsely — cycling the catalogue so each piece recurs a few
  times. Each `Placement` carries the painting centre + outward normal (toward
  the viewer) and exposes `plaque_center()` (same wall, just below). Painting
  height is `ART_HEIGHT` (1.7) at `ART_CENTER_Y`; the plaque is `PLAQUE_HEIGHT`
  (0.42) at `PLAQUE_CENTER_Y` — both inside the 3-tall wall band, clear of the
  floor. Widths come from baked image aspects.
- `ART: &[ArtMeta]` — the curated registry (slug / title / artist + baked pixel
  `w`,`h` so quads build before textures load). `art_bytes(slug)` embeds the
  artwork **and** `<slug>-plaque` JPEGs via `include_bytes!` on **native**.
- Constants: `WORLD_TYPE = "gallery"`, floor block `STONE` at y79 (bedrock y78),
  walls `SANDSTONE` (light, warm, non-white) at y80–82.

### Renderer — `renderer.rs` + `shader.wgsl`

- New `fs_painting` fragment + `painting_tex`/`painting_samp` at **group 2,
  bindings 2/3** (every other group/binding pair was taken; 2.2/2.3 are free and
  conflict-free with the avatar skin at 2.0/2.1). Reuses `vs_main`.
- `create_painting_resources()` builds a dedicated pipeline (layout
  `[camera, block-texture, painting]`; opaque, depth-writing, no back-face cull),
  a linear sampler (smooth photographic art, unlike pixel skins), and the
  group-2 layout.
- **Texture reuse:** `upload_painting_image(tex_id, w, h, rgba)` uploads each
  unique image ONCE into `painting_textures[tex_id]`; `add_painting_quad(tex_id,
  center, normal, height, aspect)` hangs a quad that references it by id
  (`right = up × normal` for an unmirrored frame). A densely-hung maze (≈60
  walls × 2 quads) therefore holds only ~24 textures (12 art + 12 plaque), not
  120. Painting tex id = `ART` index; plaque tex id = `1000 + index`.
- New **Pass 1.4** (between decals and water) draws each quad with per-quad
  frustum culling, skipping any whose texture hasn't uploaded yet (async pop-in).
  Empty in every non-gallery world → zero cost.
- `clear_paintings()` drops quads + textures on world change.

### Plaques — pre-baked title/artist panels

Rather than add a font/text rasteriser to the engine, the 12 title+artist
plaques are **pre-generated as 600×150 images** (PIL, DejaVu) and hung as small
quads below each painting — just more textures through the same pipeline. Served
as static assets / embedded on native exactly like the art, so zero bundle cost.
`<slug>-plaque.jpg`.

### Art delivery — static assets, NOT in the WASM bundle

The 12 JPEGs are ~2 MiB; already-compressed, so brotli can't shrink them. The
WASM bundle gate is 5 MiB brotli with only ~0.85 MiB headroom, so **baking the
art into the binary is a non-starter**. Instead:

- **Web:** served from `tools/sites/game/static/gallery/<slug>.jpg`, fetched at
  runtime by `window.axenstax_load_gallery_art` (`world_store.js`) →
  `js_load_gallery_art` bridge. Bundle stays at 4.13 MiB.
- **Native:** `include_bytes!` (no size gate on the native binary).

Decoded pixels reach the GPU via the slot/drain pattern (mirrors skin load):
`GameState::tick_gallery_art()` requests once per visit (native decodes inline;
WASM spawns per-painting fetches that push into `gallery_art_slot`), then drains
decoded paintings into `Renderer::add_painting` each frame.

### Launch — a data-driven scenario

The Gallery is a `ScenarioDef` (reuses the world-create/seed/enter/menu-card
lifecycle) with a new honest `Objective::FreeRoam` (never ends, no objective
HUD, no score). New **optional** `ScenarioDef` world fields —
`world_type` / `game_mode` / `time_lock` / `mobs_enabled` — drive the fresh
arena's `WorldMeta`. Hash Dash and Satori Rush leave them `None` (unchanged).
Def: `assets/scenarios/gallery-maze.json` (`world_type:"gallery"`,
`game_mode:"adventure"`, `time_lock:"day"`, `mobs_enabled:false`). Card added to
`menu::official_stash_items()` as the third official experience.

## Verification

`check.sh`: clippy no errors, native build green, **wasm build green**, bundle
**4.13 MiB < 5 MiB gate**, WGSL `wgsl_shader_validates` green (validates
`fs_painting` + the new bindings in naga). All 8 new unit tests pass; full suite
2553/0 single-threaded. (One pre-existing parallel-test flake —
`import_picked_world_garbage_bytes_fails_gracefully`, a shared-`./worlds/` race
unrelated to this work — surfaces only under parallel `cargo test`.)

## Deferred / playtest boundary

- **Feel + visuals** (lighting, painting size, maze flow, queasiness) — Axolittle
  playtest; tunable via the constants in `gallery.rs` + `fs_painting`.
- Per-painting **title/artist plaques** in-world (metadata already in `ART`).
- Mip-mapping the painting textures (currently 1 mip; linear-filtered).
- Larger / themed maze layouts; more pieces per artist.
- Painting placement uses one candidate per artwork spread across the maze;
  could weight toward better sightlines.

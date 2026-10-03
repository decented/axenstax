# Minimap + waypoints + full-screen map — the most-demanded QoL gap

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-community-features`, branch `worktree-community-features`, goal `2026-06-16-community-features-solo-buildout`). Phase 1 feature **#6** — the highest external demand of anything not already covered (replaces JourneyMap, **~336M CurseForge**, the single most-demanded QoL gap; and Xaero's).

**What shipped** (4 increments, each `check.sh`-green): (1) pure map core (`minimap.rs`: `topmost_map_block`/`relief_shade`/`cell_colour`/`project_to_map`/`composite_rgba`, 14 tests) + `World::highest_block`; (2) `MinimapCache` (persistent per-chunk-column explored memory, `plan_refresh`/`build_tile`/`update`, throttled near-recompute) + corner minimap HUD (`hud_ui::draw_minimap`, north-up + heading arrow, player 0) + `GraphicsSettings.minimap_enabled`/`minimap_zoom` (settings-panel checkbox + slider); (3) waypoints (`waypoint.rs`: `Waypoint`/`WaypointKind` + pure ops, persisted `WorldSave.waypoints` append-only serde-default + round-trip test, auto **death markers** on the `just_died` one-shot, `/waypoint add|list|remove|tp` command `OpLevel::None`, minimap waypoint dots); (4) full-screen map (`map_ui.rs` + `MapScreen`, **M** toggle outside the Workshop, scroll-zoom/drag-pan/recentre, pin-centre, waypoint list with TP-creative/remove, input-gated while open). **Remaining = Axo playtest** (minimap feel/size/cadence, rotate-vs-north-up, full-map ergonomics, palette legibility). Deferred items unchanged (below).
**Date**: 2026-06-16
**Backlog**: `docs/research/2026-06-15-native-bake-in-feature-backlog.md` §6.

---

## TL;DR

A real-time **top-down minimap** rendered from loaded chunk data (HUD corner, north-up, player-arrow heading), a **full-screen explorable map** (pan + zoom over explored area), and a **waypoint** system (create/label/colour, teleport-in-creative, auto **death markers**). Works fully in single-player / PWA. The web-map (multiplayer Dynmap surface) is the deferred Tier-3 #12 add — out of scope here.

## Design (concrete, not cards)

### Data model — a per-chunk-column tile cache (the "explored map memory")

The map is built from **tiles**, one per chunk-column `(cx, cz)`. A tile is a `16×16` grid of `MapCell { block: BlockId, height: i16 }` — the topmost non-air block of each world column and its Y (for relief shading). Tiles are computed lazily on first sight and **persist** once explored (terrain you've left doesn't change), so the full map is just the union of seen tiles. Near-player columns are recomputed on a throttle so freshly-placed/mined blocks update; a precise per-block dirty-event hook is a later optimisation (the throttled recompute is *functionally correct*, just not maximally efficient — a real TODO, not a placeholder).

- **Pure core** (`minimap.rs`, fully unit-tested, no `World`/GPU dependency):
  - `topmost_map_block(y_hi, y_lo, get, is_opaque) -> Option<(i32, BlockId)>` — column scan.
  - `relief_shade(height, north_height) -> f32` — Minecraft-style N-neighbour relief multiplier (higher-than-north = brighter, lower = darker, equal = flat).
  - `cell_colour(base_rgb, shade) -> [u8; 3]` — apply shade, clamp.
  - `project_to_map(world_xz, centre_xz, blocks_per_pixel) -> (px, py)` — world→map-pixel projection (shared by minimap + full map + waypoint dots).
  - `MapTile` build from a column accessor.
- **World-facing** (`minimap.rs`): `MinimapCache` keyed by `(cx, cz)` → `MapTile`; `ensure_near(world, registry, centre_chunk, radius)` throttled refresh; `composite(centre, radius_px) -> ColorImage-ready RGBA buffer`.

### Render — egui texture, not thousands of rects

Each cache refresh bakes the visible region into one RGBA buffer → a single egui `TextureHandle`, drawn in the HUD corner (square frame, north-up) with a rotated player arrow + waypoint/death dots on top. One texture upload per refresh, not per frame. The full-screen map reuses the same compositing at a coarser `blocks_per_pixel`.

### Waypoints — per-world, in the save (append-only)

`Waypoint { id: u32, name: String, pos: [i32; 3], colour: [u8; 3], kind: WaypointKind }` where `WaypointKind = Manual | Death`. Stored in a new `WorldSave.waypoints: Vec<Waypoint>` (serde `#[serde(default)]`, append-only — same pattern as `WorldSave.graves`). Runtime mirror on `World` / `GameState`.

- **Create**: `/waypoint add <name>` (+ colour arg) at player position; full-map "drop here" button.
- **Death markers**: auto-create a `Death` waypoint at the death column where the grave is placed (the graves spec flagged this pairing). Capped/rolling so they don't accumulate unbounded.
- **Teleport-in-creative**: `/waypoint tp <name>` (creative only — reuses the `tp` command's set-pos), full-map click-to-teleport in creative.
- **List/remove**: `/waypoint list`, `/waypoint remove <name>`.

### Controls (v1)

- Minimap visibility = persisted setting (default on). `GraphicsSettings.minimap_*` (size, enabled). North-up default.
- `M` opens/closes the full-screen map.
- Waypoint create/manage via commands + full-map UI (avoids new gameplay-key conflicts in v1).
- Gamepad/touch parity for the map screen = playtest polish.

## Solo boundary → playtest gate

Build solo: the sampler, tile cache, minimap render, waypoint model + persistence, death markers, full-screen map, commands — all unit-/integration-testable headless. **Playtest** (owner): minimap *feel* (size, rotate-with-player vs north-up, update cadence), full-map pan/zoom ergonomics, colour palette legibility, marker density.

## Deferred (named so the omission is deliberate)

- Per-block dirty-event map invalidation (throttled near-recompute covers correctness now).
- Web/multiplayer map surface (Tier-3 #12 — needs an authoritative server).
- Cave/underground layer view (v1 shows surface-topmost only).
- Rotating ("player-up") minimap mode — a feel toggle, decide at playtest.
- Gamepad/touch map navigation.

## Spec maintenance

Spec 03 (Rendering) gains a Minimap/Map section; Spec 05 (Gameplay) gains the waypoint/teleport note. Updated as the increments land.

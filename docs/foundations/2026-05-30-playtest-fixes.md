# Playtest fixes — 2026-05-30 (Axolittle)

**Status**: DELIVERED on branch `fix/playtest-2026-05-30-batch`. `check.sh` ALL GREEN; 1854 engine tests pass (0 fail). Two items need Axolittle's eyes next playtest (flagged below).
**Source**: Axolittle's playtest feedback list (lag, creative HUD, keyboard trap, worldgen, mobs, `/time`, world delete).

These are the conventions + root causes that must survive an engine rebuild. Each fix also carries a dated comment at its code site.

## 1. Lag — "too many flowers", "too laggy to play nicely"
Two levers, both in the chunk-mesh / worldgen path (no new GPU pipeline):
- **Density trim** (`world.rs::place_vegetation`): flower roll 4% → **2%**, tall grass 12% → **6%** (`FLOWER_PERCENT` / `TALL_GRASS_PERCENT` consts). Regression-guarded by `flower_density_is_trimmed_for_playtest_lag` (flower coverage of grassy surface ≤ 3%).
- **Cross-billboard plants** (`mesh.rs::emit_cross_billboard`): plant blocks (flowers, tall grass, crops, berries) now emit a 2-plane cross — **8 verts / 24 indices, double-sided** — instead of a 6-faced small cube (24 / 36). ~67% fewer vertices on the densest meadow geometry, and reads as a crossed stem rather than a chunky cube. Volumetric non-solids (`TORCH`, `CAMPFIRE_SMOKE`) keep the cube. The cross is double-sided via reversed-winding indices because the chunk pipeline has `cull_mode: Back`; normal is `(0,1,0)` so both faces catch overhead sun and never self-shadow to black.

**Full GPU instancing — BUILT on branch `feat/plant-instancing` (pending visual verify).** The renderer had no instancing infrastructure (every pipeline was `VertexStepMode::Vertex`), so this added one from scratch: `PlantInstance` (32 bytes: world-min corner, AABB size, tex layer, light), a shared unit cross-billboard, a `plant_pipeline` (cull none, alpha-cutout `fs_plant`), per-chunk instance buffers, and a dedicated plant pass after chunks/before water. Plants are collected into `ChunkMeshes.plants` instead of baked into the opaque mesh; `Renderer::upload_chunk(pos, &ChunkMeshes)` is the single upload entry point (so no site forgets plants). Verified as far as possible without a GPU: native + WASM build, all unit tests (instance data, geometry), **and a naga static validation of `shader.wgsl`** so a WGSL error can't reach a device. Irreducibly GPU-gated: the on-screen look + the runtime pipeline-creation binding check — hence it sits on a branch until a playtest confirms it. The already-merged density trim + (now-superseded) cross-billboard combo had cut plant geometry ~83%, so the acute lag was addressed before instancing.

## 2 & 8. Trees without leaves + trees in water — `world.rs::place_trees`
Rewrote placement around a pure `tree_at_column_cell()` (eligibility from `biome_gen` noise only — never `self.get_block`):
- **Canopy completeness**: a column now places tree blocks from itself **and its 8 neighbours**, keeping only blocks that land inside it. Because eligibility + shape are pure/deterministic, every leaf is placed exactly once by whichever column it falls in, regardless of generation order. Replaces the legacy clip-at-boundary placer that dropped cross-chunk leaves (bare half-canopies). The trunk inset was removed (it was the clipping workaround). **Invariant: no canopy may extend ≥ `CHUNK_SIZE` horizontally** (guarded by a `tree_shapes` test) so immediate-neighbour consultation is complete.
- **Water guard**: trees require `surface > SEA_LEVEL`. The grass-biome column returns `GRASS` at `y == surface` even when submerged, so the old surface-block match let trees root on a lakebed. `place_vegetation` already gated on this; trees did not.

**Side observation (not fixed):** jungle terrain (`SEA_LEVEL + scale_lake(14.0)`) sits below sea level in ~60–80% of cells, so jungles are sparsely treed. Pre-existing terrain-tuning issue, out of scope — flagged for later.

## 4. Creative still shows hearts + hunger — `hud_ui.rs::draw_hud`
Added `is_creative` param; hearts + hunger bars are skipped in creative (no damage, no hunger there). Armour readout is self-gating (0 in creative → draws nothing).

## 5. Creative-search keyboard trap (E/T/B leak while typing) — `main.rs` keyboard handler
The mouse path guarded on `egui_consumed`; the keyboard path did not, so keystrokes typed into the inventory-explorer search box also fired gameplay/menu hotkeys. Added the same `egui_consumed` guard the `chat.open` block uses: when an egui widget has keyboard focus, skip gameplay key processing — but always let key *releases* through (so a movement key held when a widget grabs focus doesn't latch). The explorer closes via egui's own Escape handler + a Close button, so the guard doesn't trap the user.

## 6. `/time set day`/`night` swapped
See the 2026-05-30 addendum in `docs/foundations/2026-05-07-engine-commands.md`. **Engine clock: `0 = midnight, 6000 = sunrise, 12000 = noon, 18000 = sunset`.**

## 7. Unable to delete worlds (PWA / WASM) — `save.rs` + `menu.rs`
The WASM `delete_world` was a silent no-op (`Ok(())`), so the menu's Delete button did nothing on the only alpha target. `delete_world_wasm` (IndexedDB) already existed. Now: `save.rs::delete_world` fires the real delete (+ drops cached meta); the menu's WASM path uses `kick_off_local_delete`, which **sequences delete → re-list in one async chain** so the refreshed world list can't race ahead of the IndexedDB delete commit. Native path unchanged.

## 10. Animals in / walking into water — `mob_ai.rs` + `entity.rs`
- **`can_walk_to` rejects water**: water is non-solid so it never tripped the obstruction check; a step whose body cell is water is now refused. Mobs avoid water by choice in every AI state. (The night spawner already rejects water *surfaces*; the only real spawner is `tick_mob_spawning` — there is no passive-animal world spawner, so "animals in water" was mobs walking in.)
- **Swim-to-shore**: when a mob's feet are in water, the AI tick skips the normal state machine and steers toward the nearest dry, standable land (`swim_dir_to_land`, 8-block search), setting velocity directly (bypassing the can_walk_to water veto).
- **Buoyancy** (`entity::tick_entities`): a submerged entity cancels gravity + gains a small capped lift (`BUOYANCY`, `MAX_RISE`) so it floats to the surface and swims out instead of drowning at the lakebed.

**Needs playtest:** the swim *feel* (speed, how reliably mobs reach shore) and the cross-billboard *appearance* can only be judged in-engine.

# Spawn-proof overlay — light up to stop the mobs

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-community-features`, goal `2026-06-16-community-features-solo-buildout`). Phase 1 feature **#8** (replaces light-level / mob-spawn-prevention overlay mods).
**Date**: 2026-06-16
**Backlog**: `docs/research/2026-06-15-native-bake-in-feature-backlog.md` §8. *Demand caveat: the "JEI-tier popularity" claim for this category was **refuted** — a valued builder tool, not a universal must-have. Built after the headline features accordingly.*

---

## TL;DR

An **F7** toggle that paints red wireframe markers around every surface cell where a hostile mob can spawn at night, so builders can light-proof an area. Reuses the world-space overlay-buffer mechanism (a dedicated `spawn_marker_buffer` parallel to the ghost-placement wireframe).

## Engine rule is binary (not Minecraft graded)

`spawning::tick_mob_spawning` spawns a mob on a solid, non-water surface only where the **block-light** one cell above is exactly **0** (sky-light is handled by the day/night gate elsewhere). There's no graded 0-7 light scale, so the overlay is **one colour** — red = "will spawn at night". No yellow "spawnable only in deeper dark" tier exists to distinguish. (`is_spawnable_surface` mirrors that gate exactly.)

## Design (concrete, not cards)

- **`spawn_overlay.rs`** — `is_spawnable_surface(surface, block_light_above)` (the pure rule) + `scan_spawnable(world, centre, radius)` (collect spawn cells around the player, reusing `World::highest_block` + `block_light_at`). Headless unit-tested.
- **Render** — `renderer::set_spawn_markers(player, &cells)` builds red wireframe cubes (reusing `build_wireframe_cube_colored_t`) into a dedicated `spawn_marker_buffer` on `PlayerGpuResources`, drawn in its own overlay pass (Pass 2.5b) — coexists with ghost placement + the targeting outline.
- **Toggle + refresh** — `GameState.spawn_overlay` flipped by **F7** (main.rs event loop, like F3); `GameState::refresh_spawn_overlay` rescans (throttled ~1.3 Hz, since the scan is O(radius²) columns) and re-uploads, or clears when off. Player 0 / local, `SPAWN_OVERLAY_RADIUS = 20` columns.

## Solo boundary → playtest gate

Solo: the spawn rule + scan are unit-tested; the toggle + render compile + run. **Playtest** (owner, needs a display): the markers' visual read (cube outline vs a flatter top-face marker), the radius/throttle feel, and whether a second "near-dark" tier is worth faking despite the binary engine rule.

## Deferred

- Flatter **top-face marker** geometry (vs full cube outlines) if the cubes read as cluttered — a render-geometry swap on the same buffer.
- A **settings-panel toggle** in addition to F7 (discoverability), mirroring `minimap_enabled`.
- Multi-player / split-screen markers (player 0 only in v1).

## Spec maintenance

Spec 03 (Rendering) overlay-pass note + Spec 05 builder-tools note reference the F7 overlay and the binary spawn rule.

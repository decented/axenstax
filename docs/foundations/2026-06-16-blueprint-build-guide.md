# Blueprint build-guide — Litematica-style ghost build-along

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-community-features`, goal `2026-06-16-community-features-solo-buildout`). Phase 1 feature **#9** (replaces Litematica's ghost-hologram build-along + material list + Schematic Verifier).
**Date**: 2026-06-16
**Backlog**: `docs/research/2026-06-15-native-bake-in-feature-backlog.md` §9. The **reverse** of plan capture — reuses the shipped `PlanData` blueprint format.

---

## TL;DR

Project a captured blueprint (`PlanData` from `world.plan_registry`) as a **build-along ghost**: `/buildguide <plan>` anchors it at your feet and paints **white** wireframe ghosts where blocks are still missing (**red** where a wrong block sits). Ghosts vanish as you place correctly. A HUD panel shows **progress** (done/total) + the **material list** (blocks still needed, by name). `/buildguide off` clears it; `/buildguide list` lists plans.

## Design (concrete, not cards)

- **`build_guide.rs`** — pure core, headless-tested (5 tests): `material_list` (counts by block), `verify_cell`/`verify` (per-cell Correct/Missing/Wrong vs the live world), `summarize` (counts), `remaining_materials` (what's still needed). `BuildGuide { name, origin, cells }` is the active state on `GameState`.
- **Render** — `renderer::set_build_guide_markers(player, &[(pos, colour)])` builds **per-cell-coloured** wireframe cubes into a dedicated `build_guide_buffer` (Pass 2.5c, parallel to the spawn overlay + ghost placement). `GameState::refresh_build_guide` verifies (throttled) and paints **only** the not-yet-Correct cells, so the ghosts disappear block-by-block as you build.
- **Command** — `/buildguide <plan>|off|list` (`OpLevel::None`, not a cheat — it places nothing). Validates the plan exists, computes the origin (player foot block), returns `CommandResult::StartBuildGuide { name, origin }`; the game loop clones the plan's cells and arms the guide. 3 command tests.
- **Panel** — `hud_ui::draw_build_guide_panel` (top-left): progress + the named material list (resolves block names via the block registry).

## Solo boundary → playtest gate

Solo: the verifier + material list + command are unit-tested; the ghost render + panel compile + run. **Playtest** (owner, needs a display): the ghost read (cube outlines vs translucent block previews), origin/anchoring ergonomics (a placement handle vs the foot-block anchor), panel placement.

## Deferred

- **Anchor handle** — a way to nudge/rotate the projection origin (v1 anchors at the player's foot block, SW corner).
- **Translucent block previews** instead of wireframe outlines (a render-geometry swap on the same buffer).
- **Schematic-verifier report** to chat/file; per-layer build mode; auto-place in creative.

## Spec maintenance

Spec 05 (Gameplay) creative-tools note references `/buildguide`; the render pass is noted alongside the #8 overlay in Spec 03.

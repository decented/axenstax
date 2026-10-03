# Cinematic Camera Director — Phase 1 (live, native)

**Date:** 2026-06-26 · **Status:** built (Phase 1 of the Cinematic Camera & Replay design).
Design: `docs/superpowers/specs/2026-06-26-cinematic-camera-and-replay-design.md`.
Plan: `docs/superpowers/specs/2026-06-26-cinematic-camera-and-replay-plan.md`.

## What it is

A **render-only camera rig** detached from the avatar, for the primary viewport
(slot 0), with six modes + keyframe paths + a hide-HUD toggle. The freecam, but
clean — because we own the renderer, the camera is "just a coordinate," no frozen
packets / fake entities. Native-only (the web bundle is untouched).

## The load-bearing decision: override the *render camera*, not the body sync

The gameplay `slot.camera` is **never** mutated. We do **not** conditionalise
`slot.camera.position = slot.player.eye_pos()` (`game_loop.rs`). Instead, at the
render-camera binding (`game_loop.rs`, the `for screen in &screens` loop) we swap
in a director-built `Camera` for slot 0 when active:

```rust
let director_cam = (self.director.active && pidx == 0)
    .then(|| self.director.as_render_camera(screen.viewport.aspect(), self.graphics.fov_y));
let camera = director_cam.as_ref().unwrap_or(&self.players[pidx].camera);
```

This keeps every invariant intact:
- **Eye-anchored aim** — mining/place/combat rays use `slot.camera` (untouched).
- **Third-person** — `render_eye()` path untouched; the director rig is built
  `FirstPerson` so `render_eye() == position` (no pull-back leak).
- **Anti-X-ray** — chunk streaming stays keyed on the **body** (`chunk_stream.rs`
  on `slot.player.pos`). The director adds **no** camera-keyed streaming, so it
  renders only already-loaded chunks; fly past the loaded region → empty space.
  The freecam therefore cannot scout (the X-ray guarantee, by omission).
- **Multiplayer** — no protocol change; native-local only.
- **Save/load** — director state is transient, never serialized.

The body is frozen + interaction is gated while the director flies. This needs
**two** gates, because slot-0 input flows through two intent vectors:
- the **20 TPS tick intent** (drives `Player::tick` physics) — `director_active()`
  is added to the existing `menu_open` gate (`game_loop.rs` ~2942), zeroing
  movement / jump / flight / camera_cycle so the avatar can't walk off a cliff;
- the **60 Hz frame intent** (`intent_local`, drives block break / place / attack,
  item drop, mount steering, and panel/camera toggles) — a sibling clause right
  after the per-player `intent_local` clone zeroes break / place / drop /
  toggle_inventory / toggle_explorer / toggle_debug / camera_cycle / move / jump.
  **This second gate is load-bearing:** block interaction does NOT read the tick
  intent, so without it clicking while filming would mine/place at the frozen
  body's aim (Spec: "block interaction gated off while the camera is detached").

Modal-opening keys that live outside the per-player loop (M map, J board, H help,
Y rig-studio) are additionally guarded with `&& !self.director_active()` so a panel
can't be opened over a hidden-HUD cinematic frame.

`tick_director` runs at the **top** of the per-player loop (before the dead/riding
`continue`s and before the body mouse-look), so F6 toggles even while mounted / on a
cart / dead, and the body camera doesn't double-consume the entry frame's mouse delta.

The cursor stays **captured** (mouselook) — deliberately NOT added to
`p0_ui_modal_open`, which releases the cursor.

## The six modes (`director.rs` + `camera_path.rs`)

| Mode | Behaviour |
|------|-----------|
| **Free-fly** | WASD + mouse noclip flight (reuses the spectator fly math). |
| **Path** | Plays a keyframed dolly — Catmull-Rom position/FOV, shortest-arc yaw/pitch. |
| **Tripod** | Fixed position; aim only. |
| **Follow** | Chase a target player at a yaw-rotated offset; free aim. |
| **Look-at** | Orbit a target, aim auto-locked to it. |
| **POV** | Render from a target player's eye + yaw + pitch. |

`camera_path.rs` (pure, 8 unit tests) does the interpolation; `director.rs`
(pure, 9 unit tests) the per-mode update + `as_render_camera`. Both run under
`cargo test --bin`.

## Keybinds (Phase 1 — placeholder, tunable)

All confirmed free of existing handlers; edge-triggered in `input.rs`, consumed
only while the director is active.

| Key | Action |
|-----|--------|
| **F6** | Toggle the Director (seamless enter from the current view / exit) |
| **F9** | Cycle mode (free-fly → path → tripod → follow → look-at → POV) |
| **F8** | Toggle hide-HUD |
| **F10** | Cycle follow/look-at/POV target |
| **Enter** | Drop a camera keyframe at the current pose |
| **F12** | Play / stop the keyframe path |
| **Backspace** | Clear the keyframe path |
| WASD / Space / Shift | Move / up / down · **Ctrl** boost · mouse looks |

## Deferred (documented follow-ups)

- **Multi-local director** (split-screen) — Phase 1 is slot-0 only.
- **Esc-to-exit** the director first (currently F6 exits) — wire into the pause
  path later.
- **Remote-player targeting** for follow/look-at/POV (Phase 1 targets local slots).
- **On-screen panel** showing mode / target / fly-speed (minimal in Phase 1).
- **More HUD surfaces** under hide-HUD (Phase 1 gates the main HUD + crosshair +
  rain; toasts / scenario badges still show).
- Feel tuning (FOV / fly-speed / sprint / easing) — Axolittle playtest.

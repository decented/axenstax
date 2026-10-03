# Third-Person Camera — the showcase camera (over-the-shoulder, avatar-on-self)

**Status**: ✅ BUILT — Phases 1–6 merged + deployed 2026-06-10 (`camera.rs` `CameraMode`, opt-in F5). Remaining = Axolittle feel-tuning playtest + polish (profile context-selection, gamepad/touch free-look, held-item fade, front-facing mode). Reconciled 2026-06-19.
**Date**: 2026-06-09 (drafted)
**Branch (when built)**: `feat/third-person-camera` off `main` (in a worktree —
camera/render files overlap with concurrent Workshop work; see
concurrent agent use worktree).
**Research**: `docs/research/2026-06-09-third-person-camera.md` (verified findings).
**Owner ask**: "create a spec of what will add the most value and opportunity for
AxeNStax." This spec answers that by reframing third-person from a camera option into
the **payoff screen for our customization economy**, then laying a value-ordered
phase ladder where **Phase 1 alone delivers ~80% of the value**.

---

## TL;DR

We're first-person only. We've also invested heavily in **skins** (upload-your-own),
**cosmetics**, and the **Workshop** (reskin/reshape blocks) — and in first-person the
player *never sees their own avatar or, fully, their own builds from the outside*.
A third-person camera is the screen where all of that customization finally pays off,
which is exactly why it's the highest-value-per-effort camera feature we can ship.

The research is unambiguous: players don't want one thing, they want a **bundle** of
small configurable behaviours that Minecraft forces them to bolt on with mods (the
top mod, Shoulder Surfing, has 56.5M+ downloads and is five features at once). We ship
that bundle as native, data-driven defaults.

The build is unusually cheap because **the hard parts already exist**:

- The **3rd-person avatar mesh builder is already written** (`entity_model.rs:1066`
  `build_player_avatar_vertices`, literally commented "the 3rd-person avatar") and
  already renders *remote* players — third-person reuses it for *self*.
- **Orbit-around-avatar** math already works (`renderer.rs:1463` skin preview).
- **Aim is already eye-anchored** (`player.eye_pos()` + `camera.forward()`), not
  camera-anchored — so the render camera can move back with **zero changes to mining,
  combat, or block-placement aim**. This is the #1 third-person trap and we already
  avoid it.
- **One clean camera seam** (`camera.rs`), a **u16 block registry** for per-block
  collision data (`block.rs:1123`), **`raycast.rs`** for camera collision, **per-slot
  cameras** for split-screen, and a **settings panel** (`graphics_settings.rs:141`).

---

## Why this lives here

- Per player cosmetics plan + workshop spec40 built +
  workshop blowup phase2: we built avatars, skin upload, and Workshop
  reskin/reshape — third-person is the missing surface that makes a player's *own*
  customization visible to *them*. Highest-leverage tie-in we have.
- Per launch scope axenstax only + alpha launch posture:
  AxeNStax is live and tested by kids; "see my guy" is a primary delight/identity hook
  and a low-risk, high-delight alpha addition.
- Per shared infra strategy: a `CameraMode` enum, a per-block
  `camera_occlusion` field, and an avatar-fade render path are **engine-generic**
  primitives that lift to every Decented game. Nothing here hard-codes AxeNStax.
- Per autonomy to playtest boundary: Phases 1–6 are solo-buildable up to
  the point where *feel* must be judged in-game; the final phase is an Axolittle
  playtest gate. The camera "feel" cannot be solo-verified.
- Per uk english naming: UK English throughout (e.g. "centre",
  "behaviour").
- Targets: **PWA-first / Chromium** plus native; inputs are **mouse-keyboard, gamepad
  (`gamepad.rs`), and touch (`touch_input.rs`)** — the perspective toggle must exist on
  all three.

---

## The core seam (grounded in real code)

`game/engine/src/camera.rs` today — the camera **is** the eye:

```rust
pub struct Camera {
    pub position: Vec3,   // currently set straight from player.eye_pos() (game_loop.rs:1335)
    pub yaw: f32, pub pitch: f32,
    pub fov_y: f32, pub aspect: f32, pub near: f32, pub far: f32,
}
fn view_matrix(&self) -> Mat4 {
    let target = self.position + self.forward();
    Mat4::look_at_rh(self.position, target, Vec3::Y)   // <- the only place the eye is the camera
}
```

There is **no concept of a camera offset or distance from the body**. Adding one is
the whole job. The critical invariant: **`position`/`forward()` keep their meaning for
*aim* (eye-origin ray); only the *render* view origin moves back.**

### The invariant we must not lose (write it into the spec, not just the code)

> **Aim stays eye-anchored. The render camera moves; the interaction ray does not.**

Block-interact, combat, and item-drops all already ray from `player.eye_pos()` +
`camera.forward()` (`game_loop.rs` ~3551/3629/3896/4839/5006/5068). In third-person the
*view matrix* uses a pulled-back **render eye**; every gameplay raycast keeps using the
true eye + look direction so the crosshair still means what it shows. A future rebuild
must preserve this split — it's the difference between third-person that feels right and
third-person that feels broken.

---

## Phased scope (value-ordered — Phase 1 is the MVP)

### Phase 1 — The showcase MVP *(the 80%)*

The deliverable: **press the perspective key → see your customized self, over the
shoulder → mining/combat/placing still work perfectly.**

1. **`CameraMode` enum** on the camera/player-slot:
   `FirstPerson | OverShoulder { shoulder: Side, offset: Vec3 } | OrbitBehind { distance }`.
   Default `FirstPerson`. Over-the-shoulder is the *default third-person* (the proven
   56.5M-download fix), not centred-behind.
2. **Render eye derivation** — a pure helper `render_eye(eye, forward, right, mode)`
   returning the view-matrix origin. First-person → `eye`. Third-person →
   `eye - forward*distance + shoulder_offset`. Unit-tested (eye unchanged in FP;
   pulled back + offset in TP; aim ray untouched).
3. **Render self-avatar** — call the existing `build_player_avatar_vertices`
   (`entity_model.rs:1066`) for the local slot when mode ≠ FirstPerson, applying the
   player's skin/cosmetics (already wired for remote players via
   `renderer.write_avatar_skin`). **Hide the first-person viewmodel** (`viewmodel.rs`
   `SHOW_VIEWMODEL` / per-slot gate) in third-person.
4. **Toggle binding on all three inputs** — F5-style cycle on keyboard, a gamepad
   button, and a **touch perspective button** (HUD). Cycles FP → over-shoulder
   (→ front-facing in a later phase).
5. **Per-slot** — third-person is per-player so split-screen Just Works (each
   `PlayerSlot` already owns its `Camera`).

*Acceptance:* in single-player you can flip to over-the-shoulder, see your own skin,
walk/mine/place/attack with the crosshair still accurate, and flip back. No regression
to first-person. `check.sh` green; new pure-helper tests pass.

*Explicitly deferred out of Phase 1:* collision (camera may clip walls — acceptable
for the MVP), occlusion fade, profiles, accessibility sliders.

### Phase 2 — No-snap camera collision

- Cast `raycast.rs` from the true eye toward the desired render eye; clamp the render
  eye to just before the first solid hit so the camera never sits inside geometry.
- **Pull in fast, lerp back out on a timer** once clear (the documented Vintage Story
  failure mode is "punches in and never returns"). Smoothing constants are data, not
  magic numbers.
- *Acceptance:* camera no longer enters walls; recovery is smooth, never a snap.

### Phase 3 — Avatar-fade-on-occlusion *(the better fix)*

- When the self-avatar would occlude the crosshair target (or the camera is mid-pull),
  **fade the avatar to transparent** instead of (or alongside) pulling in — the
  research's strongest-adopted occlusion handler. Needs a **per-entity alpha** on the
  self-avatar draw (toggleable for shader compatibility).
- *Acceptance:* you can always see what you're aiming at; fade is smooth; can be
  disabled.

### Phase 4 — Per-block camera-collision as registry data

- Add `camera_occlusion: CameraOcclusion` to `BlockDef` (`block.rs:1123`):
  `Squeeze | PassThrough | RotateAround | FadeOnly`. Default derived from `solid` +
  `transparent` so existing blocks behave sensibly with no per-block authoring.
- Day-one fix for MC's Glass-vs-Barrier inconsistency (MC-189617/175927) — we key off
  *registry intent*, not a visual-box accident.
- **Interaction note (decide, don't stumble into):** camera-collision raycasts run
  against the **client-obfuscated** world (buried ore → stone). That's correct — the
  camera should collide with what's rendered — but it's a conscious decision; record it
  in the spec.
- *Acceptance:* glass/leaves/fences behave per their registry tag, not per visual box.

#### Phase 4 — as built (2026-06-10)

Built on branch `feat/third-person-camera`. Decisions made at build time:

1. **Derived registry accessor, not a stored `BlockDef` field.** `BlockDef` is ~200
   hand-written struct literals, and the spec requires the default need **"no per-block
   authoring."** So `CameraOcclusion` is a derived value, not a field on every literal:
   `BlockRegistry::camera_occlusion(id)` returns `CameraOcclusion::default_for(solid,
   transparent)`, and `camera_occludes(id)` is the raycast predicate. The method body is
   the **override seam** — an authored exception (an invisible barrier that still blocks
   the camera; fences that rotate-around) branches on `id` there before the derived
   fallthrough. This is strictly more maintainable than a redundant field 99% derivable
   from existing flags, and it satisfies "no per-block authoring" exactly.
   (`block.rs`: `enum CameraOcclusion { Squeeze | PassThrough | RotateAround | FadeOnly }`,
   `default_for`, `collides`; `RotateAround`/`FadeOnly` are reserved tags the derived
   default never emits yet.)

2. **Default derivation:** opaque solid (`solid && !transparent`) → `Squeeze` (camera
   collides); everything you can see through (`transparent`) or walk through (`!solid`)
   → `PassThrough` (camera passes). The camera-collision raycast (`raycast::cast_ray_camera`,
   which replaced Phase 2's `cast_ray_solid`) stops only where `camera_occludes` is true.

3. **Leaves are opaque-solid in *this* registry** (`transparent: false`), so the derived
   default makes them `Squeeze` — they behave per their registry intent. Making leaves
   see-through for the camera would be an authored `PassThrough` override on the seam, not
   a change to the derived default (and would want to track the rendering side too). Glass
   (`solid: true, transparent: true`) is the canonical `PassThrough` case and is the L3
   differentiator.

4. **Chunk-stream-obfuscation interaction — RESOLVED (safe).** The camera-collision
   raycast reads `World::get_block`, i.e. the **client-obfuscated** world (buried ore →
   stone, per `docs/foundations/2026-05-12-proof-of-play-clarification.md`). This is
   correct (the camera should collide with what's *rendered*) and, crucially, **harmless
   to the anti-X-ray guarantee**: obfuscation only swaps *opaque solid* ore for *opaque
   solid* stone — both derive to `Squeeze`, so the camera behaves identically whether a
   cell is real ore or obfuscated stone. The camera therefore leaks **no** information
   about hidden ore (it would clamp the same way regardless), and `server_secret` /
   reward logic are untouched (Phase 1–4 are client-render only). No PROTOCOL bump.

### Phase 5 — Feel: profiles, decouple, free-look, auto-recenter

- **Pitch→distance and pitch→FOV curves** (data-driven): look down → pull back + widen
  (build overview); look up → come in close (combat framing).
- **Per-mode camera profiles**: a **Build** profile (further back, look-down bias —
  serves Workshop/blueprints/farming/villages) and a **Combat** profile (closer, tighter
  — serves Hash Dash/Satori Rush/raids). Profiles are data; the engine picks by context.
- **Decoupled camera + free-look** — strafe without turning the character; a free-look
  modifier to look one way while moving another.
- **Velocity-gated auto-recenter** — when the player isn't steering and is moving, blend
  azimuth back behind + pitch to default. **Player input always wins**, with a
  **post-input cooldown** before auto-behaviour resumes.
- *Acceptance:* building and combat each feel right with their profile; auto-recenter
  never fights deliberate orbiting.

#### Phase 5 — as built (2026-06-10)

Built on branch `feat/third-person-camera`. The guiding constraint: the owner **approved
the Phase-1 feel** ("felt right — proceed"), so **nothing in Phase 5 changes the default
feel**. Every lever defaults to a no-op; the *enablement* (turning a profile/free-look on
and tuning its numbers) is the Phase-6 Axolittle playtest.

- **All decoupling is render-only — the eye-anchored invariant holds.** Free-look moves a
  new *render* orbit offset (`Camera::orbit_yaw_offset`/`orbit_pitch_offset`); `yaw`/`pitch`
  and `forward()` (the aim) are never touched. `view_matrix` now looks along `orbit_forward()`
  (which equals `forward()` when the offset is 0) and `desired_render_eye` pulls back along
  the orbit; `projection_matrix` uses `effective_fov_y()`. Under the defaults
  (offset 0, Neutral profile) all three are **byte-identical to Phases 1–4** — verified by
  the unchanged `--shot-3p` PNGs and the full camera/`third_person` test suite.

- **Profiles are data (`CameraProfile`): `NEUTRAL` (default, no-op) / `BUILD` (further back,
  wider, look-down overview bias) / `COMBAT` (closer, tighter).** The profile *data* and the
  pitch→distance / pitch→FOV curves are **live** (read every third-person frame), but under
  `NEUTRAL` (`distance_mul 1`, `fov_offset 0`, curve `k` 0) they are identity. `pitch_distance_scale`
  / `pitch_fov_offset` are pure + L1-tested.

- **Free-look + auto-recenter are a complete, L1/L3-tested *mechanism* (camera methods
  `apply_free_look` / `tick_auto_recenter` + pure `should_auto_recenter`), shipped as
  Phase-6 *enablement API*** (marked `#[allow(dead_code)]`). They are **not yet wired to a
  live input** because the binding (which key/stick/touch modifier) and the *feel* (recenter
  speed, cooldown, profile-by-context selection) are exactly the things that need a real
  player — the irreducible Phase-6 gate. The mechanism is concrete and regression-guarded;
  the input/selection layer is the deferred piece, not a temporary bridge.

- **Test coverage matches the plan:** L1 — `pitch_distance_scale`, `pitch_fov_offset`,
  profile ordering, `should_auto_recenter` predicate, cooldown countdown, `apply_free_look`
  arms cooldown; "L3" (recenter only when idle+moving+off-cooldown; deliberate steering and
  player input always win; free-look moves render but never aim) is covered as camera-state
  integration tests driving the real methods (no world needed). The TestHost
  `crosshair_target` invariant tests still re-run every phase and guard the aim path.

- **Feel values are guesses, flagged for the Phase-6 playtest:** profile distances/FOV,
  pitch-curve strengths, recenter speed/cooldown, orbit-pitch clamp. No PROTOCOL bump
  (client-render + input only).

### Phase 6 — Accessibility + persistence *(then the playtest gate)*

- Persist `CameraMode` per player; **1st/3rd toggle**, **auto-centre delay** slider,
  **auto-centre speed** slider, FOV (already present) — added to the existing settings
  panel (`graphics_settings.rs`), honouring XAG 117.
- **Phase 6 is the Axolittle playtest gate** — orbit distance, shoulder offset,
  recenter delay/speed, and per-mode profile values are *feel* numbers that can only be
  tuned in-game with a real player (and with the kid who'll tell us if it makes him
  queasy). Ship Phases 1–5 solo; tune here.

#### Phase 6 — as built (2026-06-10)

Built solo (Axolittle not available to test). The principle: **wire the mechanism +
expose the knobs as live in-game dials, leaving the actual numbers as today's flagged
placeholders** so the playtest is a slider-drag, not a rebuild. Still default-safe — the
approved feel is unchanged out of the box.

- **Persistence (no save-format change).** The camera mode is remembered as
  `GraphicsSettings.default_camera_mode` (the device-global settings JSON, `#[serde(default)]`
  so old files load clean). The F5 toggle writes it + saves; `chunk_stream::initial_load`
  applies it to every player on world entry (new *and* loaded), so your last perspective is
  restored. Per-slot toggling during play stays independent. `CameraMode` gained
  `Serialize`/`Deserialize`.
- **Live-tunable feel → `GraphicsSettings` + sliders** (panel `menu.rs::draw_settings_panel`,
  "Third-person camera" section): **Camera distance** (`third_person_distance`, a pull-back
  zoom multiplier read in `desired_render_eye`), **avatar fade** toggle (`avatar_fade`),
  **free-look** on/off (`third_person_freelook`) + **sensitivity** + **auto-centre delay/speed**
  sliders (shown only when free-look is on). FOV already existed. Each value is pushed onto
  the camera every tick; `RECENTER_SPEED`/`RECENTER_COOLDOWN` consts are now the slider
  defaults. L1: defaults, clamps, serde round-trip, old-file fallback, and the camera reading
  the live values.
- **Free-look input bound** (`game_loop.rs`): with the setting on + third-person, **holding
  Alt** routes the mouse to `apply_free_look` (orbit the render camera, mirroring `rotate`'s
  sign) instead of `rotate` (aim) — so you look around without re-aiming; release → the
  velocity-gated `tick_auto_recenter` eases the orbit back behind you. **Aim (`yaw`/`pitch`)
  is never touched** — the eye-anchored invariant holds under free-look (camera-level test +
  the TestHost `crosshair_target` guards). Default-off ⇒ the coupled-camera feel is unchanged;
  `is_held(AltLeft)` reuses the existing input key-state. `orbit_forward` clamps combined pitch
  to ±89° so free-look can't degenerate the view matrix.
- **Still deferred (genuine Phase-6+ / playtest):** the **profile context-selection**
  (which scenario auto-picks Build vs Combat) — `BUILD_PROFILE`/`COMBAT_PROFILE`/`set_profile`
  remain tested forward-API (the distance + FOV sliders cover most of that feel meanwhile);
  gamepad/touch free-look bindings; the held-item-fade follow-up. And of course the **number
  tuning + queasiness check with Axolittle** — the irreducible human gate.
- check.sh green; no PROTOCOL bump (client-render + input + device-settings only).

---

## What each phase touches (file map)

| Phase | Primary files |
|---|---|
| 1 | `camera.rs` (CameraMode + render_eye), `game_loop.rs` (view build + self-avatar draw + viewmodel gate), `renderer.rs` (self-avatar pass), `viewmodel.rs` (per-slot hide), `input.rs` + `gamepad.rs` + `touch_input.rs` (toggle), `player_slot.rs` (mode field), `hud_ui.rs` (touch button) |
| 2 | `camera.rs`, `raycast.rs`, `game_loop.rs` |
| 3 | `renderer.rs` (per-entity alpha), `entity_model.rs`, `camera.rs` |
| 4 | `block.rs` (BlockDef field), `camera.rs`, registry init |
| 5 | `camera.rs` (curves/profiles/recenter), `game_loop.rs` (input intent), data assets |
| 6 | `graphics_settings.rs`, `save.rs` (persist mode), settings UI |

No overlap with Specs 1/2 (`hosted_server.rs`) in Phases 1–4. Phase 1 is purely
client-render + input — **no PROTOCOL_VERSION bump** (other players already broadcast
the data needed to draw avatars). Confirm before locking.

---

## Acceptance criteria (whole spec)

- First-person is unchanged and remains the default.
- Over-the-shoulder shows the player's **own skin/cosmetics**; the viewmodel hides.
- **Crosshair aim is pixel-accurate in third-person** (eye-anchored ray invariant holds)
  — mining, placing, combat, and item-drops all behave exactly as in first-person.
- Camera never sits inside solid geometry; recovery is a smooth lerp, never a snap.
- You can always see your aim target (avatar fades or camera adjusts).
- Works per-player in split-screen and on touch/gamepad/mouse-keyboard.
- `check.sh` green throughout; each phase adds unit tests (render_eye, collision clamp,
  recenter gating, per-block occlusion mapping).
- Spec 03 (Rendering) and Spec 05 (Gameplay) updated with the camera-mode model and the
  **eye-anchored-aim invariant** so a rebuild can't lose it.

---

## Memory-rule check

- merge to main preauthorised — healthy-gate merges (check.sh green) are
  pre-authorised, scoped to decented/axenstax. Applies once built.
- concurrent agent use worktree — build in a dedicated worktree; camera /
  renderer / game_loop overlap with concurrent Workshop work.
- autonomy to playtest boundary — proceed through Phase 5 solo; stop at
  the Phase 6 playtest gate.
- shared infra strategy — keep `CameraMode`, `camera_occlusion`, and the
  avatar-fade path engine-generic (cross-game lift).
- uk english naming — UK English.
- **No build yet** — per CLAUDE.md "Do NOT build anything unless explicitly asked,"
  this is a spec only. Building Phase 1 needs an explicit "build it."

---

## Open questions (resolve at build time or with a focused 2nd research pass)

1. **Front-facing mode** — keep MC's third state (camera in front, mirror view) as a
   third cycle stop, or drop it? (Low cost to keep; nice for admiring your skin.)
2. **Default orbit distance / shoulder offset / recenter delay+speed** — start from
   MC's 4-block distance and Sea of Thieves' 2.0s / 180 defaults, then tune in the
   Phase 6 playtest. Should defaults differ per input device (touch vs mouse) and per
   mode (build vs combat)?
3. **Bedrock/Better-Third-Person/Luanti/Hytale/Trove** went unverified in the research
   (§7 of the research doc) — worth a focused second pass before locking Phase 5's feel
   defaults, since those are the games most likely to show touch/controller patterns.
4. **Avatar-fade vs camera-pull as the default occlusion handler** — research supports
   fade as the strongest-adopted; confirm it reads well with our lighting/shaders, else
   default to pull-in with fade as an option.

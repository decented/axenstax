# SPEC ADDITION — Web spectate via weblink

> Folds into `docs/superpowers/specs/2026-06-26-cinematic-camera-and-replay-design.md`. Defines the read-only browser-spectator path. Stacks on two unbuilt foundations (P1 remote chunk send-path, P2 anti-X-ray obfuscation) — see the Implementation Plan, Phase 3.

## Flow at a glance

```
Operator mints link ──▶ self-describing URL ──▶ /watch dumb-shell (game-site) ──▶ WASM boots
   (admin cmd /            https://axenstax.app/watch        sets AXENSTAX_DEDICATED_WS         spectator mode
    console button)        #e=<wss-endpoint>&t=<token>       + AXENSTAX_SPECTATE_TOKEN          (PlayMode::Spectator)
                                                                                                       │
   operator's dedicated server  ◀──── wss:// JoinRequest{spectate_token, auth_event?} ───────────────┘
   validates token + ladder, marks slot is_spectator, streams FILTERED StateUpdate + OBFUSCATED ChunkData
```

The **dedicated server is the sole token authority**. The platform game-site (`axenstax.app`) never validates, never stores, never sees the token — it lives in the URL **fragment** (`#…`), which browsers never send in the HTTP request. This preserves the "platform holds nothing / self-host = operator liability" keystone: the operator's box is the only thing that can authorise a viewer.

## 1. Minting the link (server, native-only)

A volatile `SpectateToken` held in `HostedServer` (`Arc<Mutex<HashMap<String, SpectateToken>>>`), cleared on restart, mirroring the existing `.identity/` volatile-identity pattern:

```rust
struct SpectateToken {
    token: String,              // URL-safe random bearer secret
    world_name: String,
    audience: SpectateAudience, // ladder tier
    valid_until: u64,           // unix epoch — checked at join AND each tick
    max_viewers: u16,
    current_viewers: AtomicUsize,
    minted_by: String,          // operator npub, for console attribution
}
enum SpectateAudience { Owner, Whitelist, Participants, Public }
```

Two existing signed operator surfaces mint it — no new auth primitive:
- **Admin command** (reuse the Heartwood Track-5 signed `--admin` path, ~5 s sentinel reload): `mint-spectate <world> <audience> <max> <ttl>` → returns the assembled URL.
- **Operator console** (`tools/sites/console`): a "Share a viewer link" button writes the token into `.identity/spectate-tokens.json`; the engine polls `.identity/` and loads it (same sentinel pattern as `server.json` / `restart`). Gated by the existing `app.py:_require_operator`.

Spectators get a **separate `max_spectators` cap** — they must never consume play slots in `max_remote_players`.

## 2. The access ladder — who may connect

The token's `audience` decides whether the bearer link alone suffices or whether a Signet identity proof is also required. Reuses the Heartwood Track-4 access policy (`require_signin` + allowlist + blocklist) already inside `resolve_join_identity`:

| Tier | Who | Join requirement |
|------|-----|------------------|
| **Owner** | operator only | valid token **+** `auth_event`, npub == operator |
| **Whitelist** | named npubs | valid token **+** `auth_event`, npub ∈ allowlist |
| **Participants** | anyone currently playing this world | valid token **+** `auth_event`, npub ∈ present verified players |
| **Public** | anyone with the link | valid token only — anonymous, no sign-in |

`resolve_join_identity` gains a spectate branch: a valid token **short-circuits `require_signin`** for the Public tier (the token *is* the credential); for the other three it verifies `auth_event` against the ladder set instead of the play-allowlist. Blocklist applies to all four tiers. Expiry and viewer-cap are re-checked at join and on every tick (expired/over-cap → disconnect on next tick).

## 3. Privacy / identity model — who is *shown*

- **Spectators are anonymous to the world.** `is_spectator` on `ServerPlayer`: render **no avatar**, and exclude from mob targeting (mob-AI targets nearest *player* — must filter spectators out), collision, economy/ownership (`verified_pubkey` unused for blocks), and the save path.
- **Participants see only a count** ("👁 3 watching"), never spectator identities. Individual spectator npubs (gated tiers) are visible **only to the operator** in the console. The npub inspect view (`hud_ui::draw_player_inspect`) is **disabled in spectator mode** — a public viewer never gets the copyable-npub roster.
- **`StateUpdatePacket` is filtered for spectators** — strip reserve-richness, `last_acked_input`, and per-player inventory/economy fields before send. Spectators receive positions/animations/block-changes only.
- **Read-only spectate sits outside the economy/age perimeter.** No earning, placing, chat-send, or Bitcoin interaction → no HEAA/Signet age gate is triggered by *watching*. To keep it there, **chat is default-off for spectators** (operator opt-in only), keeping passive viewing clear of the OSA/UGC concerns.

## 4. Anti-X-ray — the spectator gets the *same* obfuscated stream

Non-negotiable, and the reason P2 blocks this feature. A flying noclip spectator is the highest-risk X-ray consumer in the game.
- Obfuscation lives in the **shared chunk send-path** (the P1 path), applied server-side **before bytes hit any transport**. Regular remote players and spectators flow through it — implemented once, covers both.
- Buried ore (no surface-reachable air-facing neighbour, Spec 8 §5.2.2) → **stone in the wire data**. Cave-wall / ravine / cliff / surface-exposed ore stays visible.
- **Noclip does not leak:** flying *into* solid rock shows obfuscated stone (buried = stone in the data the client ever held); flying *through a cave* shows exposed ore, same as a walking player. The client never holds unobfuscated ore positions → no client-side reveal.
- `server_secret` and reserve richness never reach the spectator (the §3 StateUpdate filter). The reward layer's architectural defence is unchanged.

## 5. The `/watch` route (game-site, dumb shell)

New route in `tools/sites/game/app.py`, modelled on `/game` (`app.py:469`) + the kiosk-globals injection pattern:
- `GET /watch` (no path token — the secret is in the fragment): serve the same WASM `index.html` with `_build_play_headers()`, plus injected `<script>window.AXENSTAX_SPECTATE_MODE = "1";</script>`.
- A tiny inline boot script reads `location.hash`, parses `e=` / `t=`, sets `window.AXENSTAX_DEDICATED_WS = <endpoint>` and `window.AXENSTAX_SPECTATE_TOKEN = <token>` **before** `start_engine()`.
- **No token store at the platform** — it never sees the token and performs no validation. Reuses the existing static WASM mount (`app.py:559`) and the `dedicated_server_url()` global-reading path (`ws_transport_web.rs:56-72`) unchanged. Operators running their own console host can serve `/watch` themselves for a fully self-contained deployment.

## 6. Where WASM fits in the spectator tiers (Spec 04 §8)

| Tier | Cap | Transport to viewer | Status for web spectate |
|------|-----|---------------------|-------------------------|
| **Close** | ≤ ~50 | full remote path — filtered `StateUpdate` + obfuscated `ChunkData`, direct-to-shard `wss` | **Alpha target.** Highest fidelity, least new code. The weblink-friend case. |
| **Standard** | ≤ ~500 | `SpectatorSnapshot` (0x1C) — reduced precision (1/16 block, 16-dir yaw), 4/s | New (Spec 04 §8.3). **Phase B / deferred.** |
| **Mass** | 10k+ | game-server → relay → N viewers | New (Spec 04 §8.2/§8.4). **Deferred** — out of scope for weblink-friend. |

The WASM client is a **Close-tier** consumer: it reuses the full remote-player rendering path (greedy-meshed obfuscated chunks + remote-avatar/entity rendering), `PlayMode::Spectator` flight/noclip (`play_mode.rs:65-71`, `physics.rs:132`), camera-follows-body (`game_loop.rs:2984` — **no camera detachment needed**), the modal input-gate to suppress break/place/inventory/chat, and the `show_hud:false` HUD-hide. The server streams chunks around the **spectator's own body position** (self-only, client-trustable for streaming; X-ray handled by obfuscation, not by hiding chunks), bounded render distance + per-spectator rate-limit.

**Reuse-vs-new vs Spec 04 §8:** §8.1 concept = reuse · §8.2 Close = reuse remote path / Standard+Mass = new (deferred) · §8.3 `SpectatorSnapshot` 0x1C = new (Phase B) · §8.4 LOD/no-chunk-stream = new (deferred; alpha uses real obfuscated chunks) · handshake = reuse + one new `JoinRequestPacket.spectate_token: Option<String>` field.

---

# Implementation plan

Four phases. Phase 1 is fully self-contained and green-able unattended. Each phase ends at `./check.sh green`. Every step is tagged **[SOLO]** (compile + `check.sh`, no human/hardware) or **[BOUNDARY]** (needs live 2-machine MP, a real browser, a real GPU, an age credential, or CI). Run with `CARGO_INCREMENTAL=0` if the LLVM linker flakes (per memory). Reference design: `docs/superpowers/specs/2026-06-26-cinematic-camera-and-replay-design.md`.

## Build status (updated 2026-06-27)

- **Phase 1 — Live Director: BUILT + reviewed + fixed.** An adversarial pre-merge review found 4 real integration bugs (block break/place not frozen while filming; F6 swallowed while mounted; split-screen HUD blanked for player 2; F8 didn't hide the crosshair) — all fixed (commit `4f4e32fe`). Anti-X-ray / eye-anchored-aim / WASM-untouched / save-load invariants verified.
- **Phase 2 — Format + recorder: 2.1–2.4 BUILT (`35fa5945`, `8452c780`, `ec7b34a6`).** `.axereplay` format, `ReplayRecorder`, native store, `ReplayPlayer` (load/seek/advance/sample_targets), and the **single-player recorder tee** (F4, one frame per sim tick from `network_send_input`). Latent cursor tick-vs-index bug found in review + fixed (TDD). Tested end-to-end (pack→record→load→reconstruct + backward-seek).
  - **Remaining [BOUNDARY]:** the in-engine Director playback UI (2.5–2.9 — timeline scrub, two-clock speed, world render-swap) is a **GPU/feel-playtest boundary** — the state-reconstruction core is tested, but the live viewing path can't be verified without a GPU. Entity/mob replay also deferred (recorder captures blocks + player transforms).
- **Phase 3 — `anti_xray.rs` obfuscation filter BUILT as a tested foundation** (Spec 8 §5.2.2 buried-ore→host-rock; pure, 6 unit tests). **Unwired** — the remote chunk send-path (3.1) it plugs into doesn't exist yet; this is intentional foundation, like the replay modules were. The rest of Phase 3 (remote send-path, server token/ladder, WASM spectator, `/watch`) is **[BOUNDARY]** (2-machine + browser).
- **Phase 4 —** not started ([BOUNDARY]: live age credential + metered CI, both owner-gated).

---

## Phase 1 — Live Cinematic Camera (Driver A, NATIVE client)

**Goal:** A native, single-machine "director" rig the renderer views the scene through, detached from the avatar, with six modes (FreeFly, Path, Tripod, Follow, LookAt, Pov), keyframe paths, hide-HUD, and a clean cinematic frame. **The gameplay `slot.camera` is never mutated** — detach happens at *render-camera selection*, not at the body-sync line. Native-gated throughout so the web bundle and the spectator/anti-X-ray surface are untouched.

**Load-bearing decision:** do **not** conditionalise `slot.camera.position = slot.player.eye_pos()` (`game_loop.rs:2984`). Keep it unconditional → third-person `render_eye()`, eye-anchored mining/place/combat rays, and body-keyed chunk streaming all stay correct. Override only the **render-camera binding** at `game_loop.rs:11722-11724`, which feeds both the GPU view-projection (`update_camera` → `gpu.last_view_proj`) and the CPU frustum cull (`renderer.rs:2405 Frustum::from_view_proj`). One seam redirects both.

### Step 1.1 — Spec the feature **[SOLO]**
- **Create** `docs/foundations/2026-06-26-cinematic-camera-director.md`: six modes, the render-override detach decision + why `2984` stays unconditional, the anti-X-ray "renders only already-loaded chunks (no camera-keyed streaming)" guarantee, keybinds, deferreds (multi-local director, remote-player targeting). Specs are source of truth.

### Step 1.2 — Pure module `camera_path.rs` (TDD) **[SOLO]**
- **Create** `game/engine/src/camera_path.rs` (no egui/winit/wgpu). Types: `CameraPose { position: Vec3, yaw, pitch, fov }`, `Keyframe { t: f32, pose }`, `CameraPath { keys: Vec<Keyframe> }` (invariant: sorted by `t`). Methods: `push_at(t, pose)`, `duration()`, `sample(t) -> Option<CameraPose>`.
- `sample`: clamp `t` to `[0, duration]`; locate segment; **uniform Catmull-Rom** for position + fov over `p0..p3` (clamp ends); **shortest-arc angle-lerp** for yaw/pitch (unwrap each control delta into `(-π, π]` so 350°→10° travels +20°). Single key → constant; empty → `None`. (Document centripetal Catmull-Rom + arc-length reparam as a Phase-2 refinement.)
- **Write tests FIRST** (`#[cfg(test)] mod tests`): (1) sample-at-keyframe passes through control point; (2) midpoint of a straight 2-key path lies on the segment & finite; (3) single-key path constant; (4) empty → `None`; (5) yaw 350°→10° crosses +20°; (6) `duration` == last key `t`; (7) clamp outside range (t<0 → first, t>dur → last); (8) dense-sweep smoke: no NaN, just-before vs just-after an interior key are close.

### Step 1.3 — Runtime module `director.rs` (TDD) **[SOLO]**
- **Create** `game/engine/src/director.rs`. Types: `DirectorMode { FreeFly, Path, Tripod, Follow, LookAt, Pov }` with `next()` (closed 6-cycle); `TargetSnapshot { eye, yaw, pitch }`; `DirectorInputs { look_dx, look_dy, fwd, right, up, sprint }`; `DirectorCamera { active, mode, pose, fly_speed (reuse physics::FLY_SPEED=10.89), sprint_mult (~2.0), target_index, follow_offset, orbit_radius/yaw/pitch, path: CameraPath, playback: Option<f32>, playback_speed, loop_path, hide_hud (default true), last_update: Option<web_time::Instant> }`.
- Pure methods: `update(&mut self, dt, &DirectorInputs, &[TargetSnapshot])` (per-mode logic: FreeFly = mouse→pose yaw/pitch clamp ±89°, position += `(fwd*forward + right*right + up*Y).normalize_or_zero() * speed*dt`; Path = advance `playback` by `dt*speed`, loop/clamp, `pose = path.sample()`; Tripod = aim only; Follow = `pose.position = tg.eye + yaw_rotate(follow_offset, tg.yaw)`, free-aim; LookAt = WASD adjusts orbit, position on orbit sphere, look-at target; Pov = `pose = {tg.eye, tg.yaw, tg.pitch}`); `as_render_camera(aspect, fallback_fov) -> Camera` (builds `Camera::new(pose.position, aspect)`, sets yaw/pitch/fov_y, **`mode = FirstPerson`** so `render_eye() == position` — no third-person pull-back leaks); `drop_keyframe(t)`; `seed_from(&Camera)`.
- **Write tests FIRST:** (9) FreeFly fwd=1 → moves `fly_speed*dt` along `forward()`; (10) up=±1 → ±Y only; (11) sprint scales speed; (12) LookAt: `normalize(target.eye − pose.position)·forward ≈ 1`; (13) Pov == target snapshot; (14) Follow holds yaw-rotated offset; (15) `mode.next()` cycles all six and wraps; (16) `drop_keyframe` appends increasing `t`; (17) `as_render_camera().render_eye() == position` (eye-anchored / no-pull-back guard).

### Step 1.4 — Register modules **[SOLO]**
- **Edit** `game/engine/src/main.rs`: add `mod camera_path;` and `mod director;` alongside the other `mod` lines (~200-290), native-gated (`#[cfg(not(target_arch = "wasm32"))]`) to match the field.

### Step 1.5 — Input fields **[SOLO]**
- **Edit** `game/engine/src/input.rs`: add 7 edge-triggered `director_*` fields following the documented 4-step keybind pattern (field decl ~L75-98 → `key_pressed` map L136+ → `new()` init L101+ → `end_frame` clear L291+), native-gated: `director_toggle`→F6, `director_mode_next`→F7, `director_hud_toggle`→F8, `director_keyframe_drop`→Enter/K, `director_path_play`→P, `director_clear_path`→Backspace, `director_target_next`→Tab. Continuous movement reads raw `is_held(KeyW/A/S/D/Space/ShiftLeft)` + mouse `mouse_dx/dy` (the modal gate does **not** clear `keys_held`, so the director moves while the body is frozen).

### Step 1.6 — GameState field **[SOLO]**
- **Edit** `game/engine/src/game_loop.rs` GameState struct: add native-gated `pub director: crate::director::DirectorCamera` + init in `new`/`Default` (`active = false`). Phase-1 scope = one director on the primary viewport (slot 0); multi-local is a documented follow-up.

### Step 1.7 — Body freeze + interaction gate (reuse modal mechanism) **[SOLO]**
- **Edit** `game_loop.rs:2913-2937`: add one native-gated clause `|| (i == 0 && self.director.active)` to the `menu_open` OR-chain. This zeroes slot-0 `move_forward/right`, `look_dx/dy`, `break_block`, `place_block`, jump, `toggle_flight`, `camera_cycle`, `toggle_debug` (2938-2957) — avatar freezes, **block break (6843) + place (8009) are gated with zero extra clauses**, F5/F3 can't fight the director.

### Step 1.8 — Mouse routing **[SOLO]**
- **Edit** `game_loop.rs:6196-6217`: skip the slot-0 `cam.rotate(dx, dy, sensitivity)` at 6215 when `self.director.active` (the director consumes the same `mouse_dx/dy` instead — no double-consume; `end_frame` clears them as today).

### Step 1.9 — Director update call **[SOLO]**
- **Edit** `game_loop.rs`: add `GameState::update_director(&mut self, dt)` and call it **once per frame** (native-gated, `if self.director.active`), immediately after the per-player input loop (mouse delta still un-cleared). It: gathers `DirectorInputs` (raw held WASD/Space/Shift + `mouse_dx/dy * sensitivity` + sprint); snapshots `targets` from local `players[i].player.eye_pos()` + `camera.yaw/pitch`; computes `dt` from its own `last_update: web_time::Instant`; handles the edge keys (toggle/mode/hud/keyframe/play/clear/target via `self.input.director_*`); calls `self.director.update(dt, &inp, &targets)`. `director_target_next` cycles `target_index` over `0..players.len()` (local slots only; remote ghost-entity targeting is a follow-up).

### Step 1.10 — Toggle / enter / exit / Esc **[SOLO]**
- **Edit** `game_loop.rs` near the camera_cycle consume (~8064), native-gated: consume `director_toggle` (F6) — ignore while a slot-0 UI modal is open; on **enter** call `self.director.seed_from(&self.players[0].camera)` (seamless start), `mode=FreeFly`, `hide_hud=true`, keep cursor captured; on **exit** set `active=false` (next frame renders from the untouched `slot.camera`; HUD returns; body resumes from its frozen position). While active, let **Esc** exit the director first (consume `pause` before it reaches the pause menu).

### Step 1.11 — Render override **[SOLO]**
- **Edit** `game_loop.rs:11719-11743`, before the `let camera = …` binding, native-gated:
  ```rust
  let director_cam: Option<crate::camera::Camera> = (self.director.active && pidx == 0)
      .then(|| self.director.as_render_camera(screen.viewport.aspect(), self.graphics.fov_y));
  let camera = director_cam.as_ref().unwrap_or(&self.players[pidx].camera);
  ```
  (`#[cfg(target_arch = "wasm32")]` variant binds `None`.) Everything downstream — `CameraUniform::from_camera`, fog, brightness, `update_camera` → frustum cull — flows from the director pose. `slot.camera` untouched. Add a code comment: *the director must never drive chunk streaming.*

### Step 1.12 — Hide-HUD **[SOLO]**
- **Edit** `game_loop.rs`: add `fn hud_suppressed(&self) -> bool` (native: `director.active && director.hide_hud`; wasm: `false`). Guard the `draw_hud(...)` call (14828) and the rain overlay (14851) with `if !self.hud_suppressed()`. Grep the crosshair render path; gate it too. `director_hud_toggle` (F8) flips `director.hide_hud`.

### Step 1.13 — Anti-X-ray non-task assertion **[SOLO]**
- Confirm `chunk_stream.rs:18-19` still keys on `slot.player.pos` (BODY). Add **no** camera-keyed streaming. Director renders only already-loaded chunks; fly past the loaded region → empty space (the X-ray guarantee, achieved by omission).

### Step 1.14 — Gate **[SOLO]**
- Run `./check.sh`: clippy + build + `cargo test --bin axenstax-engine` (new pure tests 1-17 included) + `trunk build` (web compiles; director cfg'd out) + bundle-size gate (no web growth since native-gated). **Land green.** Optional stretch: a `TestHost` integration test asserting slot-0 `player.pos` is unchanged across N ticks while "forward" is fed with the director active (proves body-freeze) — mark deferred if it widens scope.

**Invariants preserved:** eye-anchored-aim (gameplay `slot.camera` never mutated; director rig is FirstPerson) • third-person (2984 + `render_eye` untouched) • spectator (unrelated `PlayMode`; not conflated) • anti-X-ray (body-keyed streaming unchanged) • multiplayer (no protocol change; native-local) • save/load (director state transient, never serialized).

**Phase 1 files:** `camera_path.rs` (new), `director.rs` (new), `main.rs`, `input.rs`, `game_loop.rs` (2913-2957, 6196-6217, ~8064, 11719-11743, 14828-14851), `camera.rs` (`as_render_camera` builds on `Camera::new`/`render_eye`), `docs/foundations/2026-06-26-cinematic-camera-director.md` (new).

**Boundary:** none required to land. **[BOUNDARY]** Axolittle feel-playtest of camera modes / FOV / fly-speed (tuning only, post-merge).

---

## Phase 2 — Director playback + recorders (replay format, NATIVE, single-player)

**Goal:** A `.axereplay` format (a generalisation of the Trials ghost from one runner to the whole scene — *re-apply recorded state, never re-simulate*), a single-player recorder, and a Director that plays it back with a two-clock scrub, slow-mo interpolation, and the Phase-1 detached camera as its viewpoint. The on-disk frame is `StateUpdatePacket` minus server-bookkeeping — reuse existing serde types; net-new serde is only the 3 container structs.

### Step 2.1 — Format module `replay.rs` (TDD) **[SOLO]**
- **Create** `game/engine/src/replay.rs`: `ReplayHeader` (magic `AXEREPL1`, format_version, `ReplaySource { SinglePlayer | MpClientTee | MpServerSession }`, tick_rate=20, world_seed, world_name, start_world_time, recorded_at, duration_ticks, keyframe_interval_ticks, `keyframe_index: Vec<(tick, byte_offset)>`, perspective_hints), `InitialSnapshot { world_archive_blob (reuse pack_world()), players: Vec<PlayerState>, entities: Vec<EntitySpawn> }`, `ReplayFrame { tick, players, block_changes, entity_spawns, entity_updates, entity_despawns, world_time }` (all inner types reused as-is from `protocol.rs`), `ReplayFile`. LZ4 per N-frame block (matching `ChunkDataPacket` convention). Periodic `KeyframeFrame` = full players+entities snapshot + accumulated block-change set since last keyframe. `max_duration_ticks` cap (reuse `MAX_GHOST_FRAMES` discipline).
- **Write tests FIRST:** encode→decode round-trip byte-identical; keyframe-index seek lands on nearest preceding keyframe; delta-apply-forward reconstructs block state at an arbitrary tick; `max_duration_ticks` guard truncates; `EntityKind` append-only forward-compat smoke.

### Step 2.2 — `ReplayRecorder` (TDD) **[SOLO]**
- **Add to** `replay.rs`: `ReplayRecorder` — append-only frame buffer with `keyframe_interval`, `max_duration_ticks` cap, `finalize() -> bytes`. One type, three future call sites. Unit-test: N pushes → finalize → reload → frame count + keyframe stride correct.

### Step 2.3 — Replay store **[SOLO]**
- **Add** native file persistence (`.axereplay` under a replays dir) + web stub, mirroring the `TrialBests`/`graphics_settings` dual-target pattern (`trials.rs:121`, `graphics_settings.rs:442`).

### Step 2.4 — Single-player recorder tee **[SOLO]**
- **Edit** `game_loop.rs` next to the ghost-record block (~2274): each sim tick snapshot local players (from `PlayerSlot.player`/`camera`) + render-ECS mobs + the drained `pending_block_changes` into a `ReplayFrame`; initial snapshot via `pack_world()`. No network → inherently safe. Produces a real `.axereplay` from a solo session.

### Step 2.5 — Playback core `replay_player.rs` **[SOLO]**
- **Create** `game/engine/src/replay_player.rs`: load a `ReplayFile` into a playback world (reuse `world_archive` unpack) + render-only ECS; `seek(tick)` (binary-search `keyframe_index` → restore nearest keyframe → fast-apply deltas forward, O(keyframe_interval)); `advance(dt, speed)`; apply each frame's `block_changes` to the world and **set** player/entity transforms (insert on spawn, remove on despawn). No `Player::tick`, no `mob_ai`, no economy. Unit-test: load the file from 2.4, assert state at sampled ticks matches.

### Step 2.6 — Director UI `director_ui.rs` (two clocks + scrub) **[SOLO]**
- **Create** `game/engine/src/director_ui.rs` (state-struct + outcome-enum panel, per the build-map UI pattern): timeline/scrub bar reading `keyframe_index`, play/pause, speed dial (0.0=pause, 0.25=slow-mo, 1.0=realtime, >1 fast, negative=reverse), camera-mode picker, follow-player selector. Sim clock (recording tick axis) + wall clock (playback-speed multiplier). Keybind + entry point via `input.rs`/`game_loop.rs`.

### Step 2.7 — Inter-tick interpolation + slow-mo **[SOLO]**
- **Edit** `replay_player.rs`: between adjacent `ReplayFrame`s lerp position + slerp yaw/pitch by the fractional wall-clock phase (reuse the lerped-Cart interpolation model, `protocol.rs:386`). No new data — render-time only. Makes 0.25× fluid not steppy.

### Step 2.8 — Detached camera driver for playback **[SOLO]**
- The Director consumes the Phase-1 `DirectorCamera` directly: FreeFly (noclip), Follow-cam (lock to a `perspective_hints` player eye = `pos + 1.62`), Path (`CameraKeyframePath`). The replay Director is the first real consumer of the cinematic rig. Camera keyframe paths reuse `camera_path.rs` from Phase 1.

### Step 2.9 — Gate **[SOLO]**
- `./check.sh` green. Record a SP session → load → scrub → drive the cinematic camera over it, all unattended.

**Phase 2 boundary:** **[BOUNDARY]** feel/UX of slow-mo + cinematic paths (Axolittle playtest); a real GPU only matters for *feel*, not for green.

---

## Phase 3 — Networked spectator + web-spectate-via-link (NATIVE + SERVER + WASM)

**Goal:** A browser joins a live dedicated server as a read-only spectator via a shareable link. **Two unbuilt foundations (P1, P2) are the real cost and must be built first** — they also unblock real remote-multiplayer world-rendering. Never expose a chunk stream to a browser spectator without obfuscation in place.

### Step 3.1 — P1: remote chunk send-path **[SOLO build / BOUNDARY verify]**
- **Edit** `hosted_server.rs`: per-remote-client chunk streaming around the client's body position — **construct + send** `ChunkDataPacket` (currently defined `protocol.rs:472-482` but never sent). Reuse `chunk.as_bytes()` + LZ4. **[SOLO]** unit tests (construct→compress→decode round-trip; streaming set around a position). **[BOUNDARY]** live-verify a native remote client actually renders the streamed world.

### Step 3.2 — P2: anti-X-ray obfuscation **[SOLO build / BOUNDARY verify]**
- **Implement** Spec 8 §5.2.2 buried-ore→stone filter **inside** the 3.1 send-path, server-side, before bytes hit any transport. **[SOLO]** unit tests: buried block → stone in serialized bytes; exposed ore unchanged; re-mesh on mutation flips a newly-exposed vein to real. **[BOUNDARY]** the live X-ray proof (fly a known ore field, confirm buried veins read as stone) — can't be fully validated solo.

### Step 3.3 — Protocol field **[SOLO]**
- **Edit** `protocol.rs`: add `spectate_token: Option<String>` to `JoinRequestPacket`. Round-trip encode/decode test.

### Step 3.4 — Server token + ladder + privacy **[SOLO]**
- **Edit** `hosted_server.rs` / `server_identity` / `server.rs`: `SpectateToken` store (`Arc<Mutex<HashMap>>`, volatile), minting via admin cmd + console-sentinel, `SpectateAudience` ladder, `max_spectators` (separate from `max_remote_players`), expiry + viewer-cap recheck each tick. Spectate branch in `resolve_join_identity` (ladder check; `require_signin` short-circuit for Public; blocklist still applies). `is_spectator` on `ServerPlayer` → exclude from mob targeting, collision, economy/ownership, save. Per-spectator `StateUpdate` filter (strip reserve-richness, `last_acked_input`, inventory/economy). Tests: token lifecycle (mint→valid→expired-reject→cap-reject→revoke); ladder logic (Public accepts anonymous; Whitelist/Owner/Participants reject wrong/absent npub); `is_spectator` exclusions; StateUpdate filter strips the right fields; spectator absent from `max_remote_players` count.

### Step 3.5 — WASM spectator mode **[SOLO]**
- **Edit** `web_main.rs`, `game_loop.rs`, `hud_ui.rs`, `input.rs`: read `AXENSTAX_SPECTATE_MODE` / `AXENSTAX_SPECTATE_TOKEN` globals; pass token into `JoinRequest` (+ optional `auth_event` for gated tiers, already supported). Suppress all gameplay intent via the modal-gating pattern (`game_loop.rs:2915-2957`) — add `is_spectator` to the OR-chain; `&& !is_spectator` on break (6843) / place (8009). Hide HUD via a `show_hud: bool` param on `hud_ui::draw_hud` (674), `!is_spectator` at the call site (14828); draw a minimal "Spectating <world> · 👁 N watching" overlay. Reuse `PlayMode::Spectator` flight/noclip (`play_mode.rs:65-71`, `physics.rs:132`), camera-follows-body (`2984` — **no detachment**), and the now-obfuscated chunk-mesh + remote-avatar/entity rendering. Disable the npub inspect view in spectator mode.

### Step 3.6 — Game-site `/watch` + console mint UI **[SOLO]**
- **Edit** `tools/sites/game/app.py`: `GET /watch` dumb-shell (serve WASM `index.html` + `_build_play_headers()` + injected `AXENSTAX_SPECTATE_MODE`) + an inline fragment-reading boot script (`location.hash` → `e=`/`t=` → set `AXENSTAX_DEDICATED_WS` + `AXENSTAX_SPECTATE_TOKEN` before `start_engine()`). No token store/validation at the platform. **Edit** `tools/sites/console`: "Share a viewer link" button → writes `.identity/spectate-tokens.json` + assembles the `#e=…&t=…` URL.

### Step 3.7 — Gate **[SOLO]**
- `./check.sh` green (clippy, build, all new unit tests, trunk, bundle-size). Web bundle now includes spectator code — watch the 5 MiB brotli gate.

### Step 3.8 — Live validation **[BOUNDARY]**
- Operator box runs the dedicated server (wss via Caddy 8443), mints a link; a **second machine's browser** opens `axenstax.app/watch#e=…&t=…`, connects, and renders the live world + players moving. **X-ray live check** (3.2 proof). Gated tiers with a real Signet phone sign-in (Whitelist/Participants/Owner). Bandwidth/feel at Close tier with several concurrent viewers; chunk-stream rate under a fast-flying spectator. Privacy: participants see only a count, spectator avatar truly absent, chat correctly withheld.

**Phase 3 sequence rule:** ship 3.1→3.2 as the foundation and verify them on their own (they pay for themselves by unblocking remote-multiplayer world-rendering), then 3.3→3.6 solo, then gate on the single live test 3.8. Do not implement 3.5 before 3.2.

**Phase 3 MP server-side replay (folds in here):** **[BOUNDARY]** tee at `broadcast_state()` (`hosted_server.rs:992`) into a server-side `ReplayRecorder` (`ReplaySource::MpServerSession`); run the same buried-ore filter before any export (server replays hold *real* ore positions — operator-only, obfuscate before sharing). The **MP client-side tee** (`RemoteClient::poll`, `remote_client.rs:~400`, `ReplaySource::MpClientTee`) is **[SOLO]** — record against a local HostedServer on loopback; add a `TestHost` integration test that ticks a HostedServer, tees N frames, and asserts they re-apply to byte-identical transforms. A client-tee replay is only as complete as the obfuscated stream it received; the same `.axereplay` type differs only in `ReplaySource` + completeness.

**Deferred (not part of weblink-friend):** `SpectatorSnapshot` 0x1C + Standard tier (Spec 04 §8.3); Mass-tier relay / 10k viewers (§8.2/§8.4).

---

## Phase 4 — Billing / access / age-safety

**Goal:** Production hardening of the spectate access surface — operator economics, abuse limits, and the compliance perimeter. Mostly policy + plumbing on top of Phase 3 seams.

### Step 4.1 — Per-spectator bandwidth metering + rate-limit **[SOLO]**
- **Edit** `hosted_server.rs`: bound chunk-send rate + render distance per spectator; cap total spectator egress per session. Anti-abuse for a fast-flying viewer. Unit-test the rate-limiter logic.

### Step 4.2 — Viewer-cap economics + operator console surfacing **[SOLO]**
- **Edit** `tools/sites/console`: operator-facing live viewer count, per-token revoke, token TTL controls, and (if the operator runs a paid world) the hook where viewer caps tie to the operator's own billing — platform holds nothing; this is operator-side bookkeeping only. Unit-test token revoke + cap enforcement.

### Step 4.3 — Age-safety perimeter assertions **[SOLO build / BOUNDARY sign-off]**
- **[SOLO]** Encode the invariants as tests: spectate triggers **no** earning/placing/chat-send/Bitcoin path → no HEAA/Signet age gate; chat default-off for spectators; gated-tier npubs operator-only. **[BOUNDARY]** A real Signet **age credential** is only needed if/when a future tier grants a spectator any economy-touching capability — out of scope while spectate stays strictly read-only; flag for counsel review (the two open compliance items: skill-vs-chance + payments perimeter) before any spectator earn/interact surface ships.

### Step 4.4 — CI / release **[BOUNDARY]**
- **[BOUNDARY]** Repo is private → Actions minutes metered; **ask before** dispatching `native-packages.yml` or any macOS/Windows CI. Web deploy is auto via `deploy.yml` on push to main. Follow the release runbook (bump 2 version files → `check.sh` → push main).

### Step 4.5 — Gate **[SOLO]**
- `./check.sh` green; the full spectate + replay + cinematic surface ships behind operator opt-in, read-only, obfuscated, and outside the age/economy perimeter.

**Phase 4 boundary summary:** the only hard boundaries are a live age-credential sign-off (4.3, deferred while read-only) and metered CI (4.4, owner-gated). Everything else is solo-verifiable.
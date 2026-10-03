# Quad split-screen on WASM — lift the web's solo-only clamp

**Status**: READY TO BUILD, **depends on #14 landing first**. Phases 2–6 are autonomous solo work. Phase 7 (Axolittle 4-player PWA playtest) blocks on his time.
**Date**: 2026-05-16
**Branch**: `feat/quad-split-screen-wasm` once started; off `main`, **after** #14's `feat/quad-split-screen` has merged.
**Session**: Fresh — implementer should treat this doc and #14 as the only briefs.
**Trigger**: 2026-05-16 chat. After speccing #14 (native quad), the question "what's actually stopping the PWA from doing the same?" surfaced. Answer: gilrs is native-only, but the browser has a perfectly capable Gamepad API the engine doesn't use. This spec is the lift.

---

## TL;DR

The web has had a Gamepad API in stable Chromium since 2014. The engine has just never plumbed it through, because gilrs (the gamepad crate the native build uses) doesn't compile on `wasm32-unknown-unknown`. Add a web-backed `GamepadSystem` behind the same `gamepad.rs` interface, walk the ten-ish `#[cfg(not(target_arch = "wasm32"))]` gates around split-screen, lift the ones that were only native-only because gamepads were, and the PWA can do the same 1-4 player local split as native.

Scope ~750 lines. The big piece is the gamepad backend (~300 lines + tests); the rest is `cfg` cleanup, a perf-verify checkpoint, and a playtest. **Hard dependency on #14** — this spec is purely "make the same feature work on WASM."

Per signet boundary and shared infra strategy, the gamepad-on-web work lifts engine-wide — other games on the same primitives and any future Decented web game inherit gamepad support automatically.

---

## Why this lives here

- **PWA is the alpha target.** Per pwa priority, Chromium-only PWA is the alpha. "PWA" today means single-player local — for AxeNStax that's fine *until* Axolittle wants to play with siblings/friends on a laptop. Two kids on one MacBook with two USB controllers is a real, requested couch-co-op shape the native build supports today and the PWA cannot.
- **One library swap unblocks an entire input axis.** `gamepad.rs` is the only file that pulls `gilrs`. Replace the gilrs-backed `GamepadSystem` with a `cfg`-split backend (gilrs on native, web-sys on WASM) behind the same public surface, and every consumer (`game_loop.rs`, the menu, the join rule) is target-agnostic.
- **Cross-game lift.** A web Gamepad backend is one of those primitives that pays off across every Decented title. Couch-co-op for any web game becomes a layout decision, not an input-system rebuild.
- **Closes the "WASM is solo" footnote.** #14's spec, `chunk_stream.rs:151`, and the 2026-04-02 wasm-web-build-design doc all carry the qualifier "split-screen stays native-only." This spec deletes the qualifier — or, more precisely, replaces it with "split-screen on web requires gamepads, which we now have."

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/gamepad.rs` — currently `use gilrs::...` at the top. The whole file is `gilrs::Gilrs`-driven. Needs to split into:
  - A `trait GamepadBackend` (or equivalent module shape) with a uniform interface (`poll(&mut self) -> &[GamepadState]`, hot-plug events, etc.)
  - `#[cfg(not(target_arch = "wasm32"))] mod native_backend` — current gilrs code, lifted into a backend impl.
  - `#[cfg(target_arch = "wasm32")] mod web_backend` — new, `web_sys::Gamepad`-driven impl.
  - The public `GamepadSystem` type wraps whichever backend the target picks. All call sites stay identical.

- `game/engine/Cargo.toml`:
  - `gilrs = "0.11"` (line 45, under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`) — unchanged.
  - `web-sys` (line 69, under `[target.'cfg(target_arch = "wasm32")'.dependencies]`) — add the gamepad feature set: `"Gamepad"`, `"GamepadButton"`, `"GamepadMappingType"`, `"GamepadEvent"`, `"Navigator"`. These are stable web-sys features.

- `game/engine/src/main.rs:227-228` — the `#[cfg(not(target_arch = "wasm32"))] pub(crate) gamepad: ...` field. Drop the gate; `GamepadSystem` becomes cross-platform with internal `cfg`-switched backends.

- `game/engine/src/game_loop.rs:1028` — `#[cfg(not(target_arch = "wasm32"))]` on the join rule. Drop the gate; the join rule becomes target-agnostic once `GamepadSystem` works on both. Native still routes keyboard to P1; WASM does too — same code.

- `game/engine/src/chunk_stream.rs:153` — `#[cfg(not(target_arch = "wasm32"))]` on the multi-player save pre-allocate loop. Drop the gate; the loop becomes cross-platform.

- `game/engine/src/touch_input.rs` — WASM's touch-input path. Stays as-is. Touch is **only** for solo P1 on a phone/tablet. If a touchscreen device has no gamepads connected, the game is single-player as today. If gamepads are plugged into a touchscreen device (Steam Deck, a tablet with USB-C hub), P2-P4 can still join via gamepad while P1 stays on touch.

- `game/engine/src/wasm_save.rs` — IndexedDB save backend. Already shape-agnostic (it serialises whatever `SaveData` holds; the format is already `Vec<PlayerSaveData>`). No code change. Add an integration test that round-trips a 4-player WASM save.

- `game/engine/src/wasm_feedback.rs` — perf-snapshot path. The existing `frame_samples` / `tick_samples` rolling windows are already set up. Phase 5 of this spec uses them to verify 4-viewport WASM rendering meets a frame-time budget.

### Native-only gates inventory (decide on each)

A grep of the engine's bin sources turned up **~105 `#[cfg(not(target_arch = "wasm32"))]` gates** across `gamepad.rs`, `main.rs`, `game_loop.rs`, and `chunk_stream.rs` alone. The vast majority are *correctly* native-only (HostedServer/QUIC, rodio audio, env_logger, pollster, …). The handful this spec lifts:

| File | Line | What it gates | Action |
|------|------|---------------|--------|
| `main.rs` | ~227 | `gamepad: GamepadSystem` field | **Lift** — gamepad system is now cross-platform |
| `main.rs` | ~349 | `gamepad.connect_first_controller()` etc. | **Lift** — same |
| `game_loop.rs` | 1028 | "Press A to join" block | **Lift** — depends on `gamepad`, now cross-platform |
| `chunk_stream.rs` | 153 | Save pre-allocate loop for multi-player saves | **Lift** — no reason this was native-only beyond gamepad gate |
| `chunk_stream.rs` (renderer hook) | within initial_load | per-player GPU resource alloc | **Lift** if present |

What stays native-only:

- HostedServer / QUIC / LAN broadcaster — still native-only (no real LAN sockets in the browser; web multiplayer needs the WebRTC transport, separate spec).
- gilrs itself — native-only by `Cargo.toml`.
- Rodio audio sink — native-only.
- env_logger, pollster, tokio, quinn, rcgen, rustls, secp256k1 — already correctly fenced.

Phase 2 of this spec is a careful audit of every one of the ~105 gates, classified as "lift / keep / unrelated", with a small diff per file that drops only the lift-class ones. Don't go on a deletion spree — read each one.

### Related specs

- **`docs/foundations/2026-05-16-quad-split-screen.md` (#14) — hard prerequisite.** The layout solver, the join cap lift, the input routing table for 1-4, the HUD-fit work — all of that lands first. This spec is "now make the same thing work on WASM."
- `docs/superpowers/specs/2026-04-01-split-screen-design.md` — parent design. §3 ("Input System") describes the per-player gamepad routing pattern, target-agnostic at the spec level.
- `docs/superpowers/specs/2026-04-02-wasm-web-build-design.md` — the wasm port. §267 ("Single player only (no split-screen, no multiplayer on web yet)") is the line this spec retires.
- `docs/spec/01-engine-architecture.md` — engine target list. No protocol change.

### Memory pointers

- pwa priority — PWA is alpha priority. Quad-on-PWA is a couch-co-op feature for that exact platform.
- shared infra strategy — gamepad-on-web is reusable for every Decented web game; keep the gamepad backend module engine-generic.
- signet boundary — this is a generic engine upgrade (gamepad backend + cfg cleanup), not an AxeNStax-only patch. The reusable bit is the backend trait + the web impl.
- uk english naming — user-facing strings UK English.
- autonomy to playtest boundary — Phases 2-6 autonomous, Phase 7 is the playtest gate.
- pretest check — Phase 6 (perf verify) must produce real numbers before claiming "WASM quad works."

### What does NOT exist yet (and this spec does NOT need)

- **WebRTC voxel multiplayer** — separate spec, much larger. This spec is local split-screen on web, no network.
- **Audio per-screen** — impossible on web in the foreseeable future (one audio context per tab). Couch co-op shares speakers. Not a feature loss.
- **Multi-window dual-screen on web** — browsers can fullscreen on one monitor only; out of scope, and probably never possible in a sandbox.
- **Touch multi-player** — a single touchscreen can't drive 4 players (no per-finger ownership). P1 may be touch on a tablet, but P2-P4 are always gamepads on web. Same routing rule as native, just with touch swapped in for KB+M when no keyboard is detected.

---

## Scope

| # | Phase | Files | Est. lines | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-16-quad-split-screen-wasm.md` | ~750 | ✓ |
| 2 | `GamepadSystem` backend split — extract a trait, port gilrs path to the native backend behind it, add a stub web backend that returns "no controllers" | `gamepad.rs`, `Cargo.toml` (web-sys features) | ~200 | ✓ |
| 3 | Web Gamepad backend — `web_sys::Navigator::get_gamepads()` polling, button/axis mapping, hot-plug events, sticks-with-deadzone, same `GamepadState` shape as native | `gamepad.rs` (web backend module), tests | ~300 | ✓ |
| 4 | Lift native-only `#[cfg]` gates around the gamepad + split-screen path (audit every gate, lift only the lift-class ones) | `main.rs`, `game_loop.rs`, `chunk_stream.rs`, possibly `menu.rs` | ~80 (mostly deletions) | ✓ |
| 5 | WASM-side wiring: join toast on web; touch+gamepad coexistence (P1 = touch or KB, P2-P4 = gamepads); fullscreen behaviour for 4-viewport rendering | `game_loop.rs`, `touch_input.rs`, possibly `wasm_auth.rs` if cursor-pointer interactions overlap | ~80 | ✓ |
| 6 | Perf verification on WASM — render 4 viewports at 1920×1080 in Chromium, measure tick + frame times via `wasm_feedback`'s existing samples, document the budget result | `wasm_feedback.rs` (verify only), playtest harness, `docs/foundations/2026-05-16-quad-split-screen-wasm.md` perf-results appendix | ~50 (verify + doc) | ✓ |
| 7 | Axolittle 4-player PWA playtest — does it feel good with two laptops + 4 controllers on a Chromebook? Or one laptop + 4 USB-C gamepads via a hub? | n/a | 0 | ✗ blocked |

**Total**: ~750 lines spec + code + tests. Phase 7 is the playtest gate.

Phases 2 → 3 → 4 → 5 → 6 are serial (each builds on the prior). Phase 3 is the bulk of the work; the rest are short.

**Hard prerequisite**: #14 (native quad) must land first. Without #14, the layout solver still panics for `num_local_players > 2` and the join cap is still 2 — there's no native path to compare against and no shared layout primitive to reuse.

---

## Phase 1 — This spec

You're reading it. ✓ Move on.

---

## Phase 2 — Backend split

### Goal

Refactor `gamepad.rs` so the public surface (`GamepadSystem::new()`, `poll()`, `to_intent(idx, dt)`, `gamepads: &[GamepadState]`, hot-plug events) is identical on both targets, with two `cfg`-switched backends underneath.

### Design

Two options:

**Option A: trait + dyn dispatch.** A `trait GamepadBackend { fn poll(&mut self); fn states(&self) -> &[GamepadState]; }` with `NativeBackend` and `WebBackend` impls; `GamepadSystem` holds a `Box<dyn GamepadBackend>`.

**Option B: cfg-switched concrete type.** `GamepadSystem` is the public type; under the hood:
```rust
#[cfg(not(target_arch = "wasm32"))]
struct GamepadSystem { inner: native::NativeBackend, ... }
#[cfg(target_arch = "wasm32")]
struct GamepadSystem { inner: web::WebBackend, ... }
```

**Pick Option B.** No virtual dispatch overhead, no trait-object lifetime weirdness, and the two backends genuinely never coexist in one binary. The mod-private modules + `cfg`-attribute-on-impls pattern is idiomatic in this codebase already (see `wasm_save.rs` etc.).

### Implementation outline

1. Move the existing gilrs code into a `mod native { use gilrs::...; pub struct NativeBackend { ... } impl NativeBackend { ... } }` block, gated `#[cfg(not(target_arch = "wasm32"))]`. The current `GamepadState` struct stays public at the module top — it's the shared shape.
2. Add a stub `#[cfg(target_arch = "wasm32")] mod web { pub struct WebBackend { gamepads: Vec<GamepadState> } impl WebBackend { pub fn new() -> Self { Self { gamepads: vec![] } } pub fn poll(&mut self) {} pub fn states(&self) -> &[GamepadState] { &self.gamepads } } }`. Stub returns "no controllers" for now — Phase 3 fills it in.
3. The public `GamepadSystem` struct (top of file) holds an `inner: native::NativeBackend` on native, `inner: web::WebBackend` on WASM. All its public methods (`poll`, `to_intent`, `gamepads`, etc.) delegate to `inner`.

### Tests

Existing gilrs-flavoured tests stay green (they live under `#[cfg(not(target_arch = "wasm32"))]` already or move there if they don't).

Add a `#[cfg(target_arch = "wasm32")]` test (under `wasm-bindgen-test`) that constructs a `GamepadSystem` and confirms `states().is_empty()`. Confirms the stub compiles and runs in a `cargo test --target wasm32-unknown-unknown` pass.

### Acceptance

- `cargo build` native — clean.
- `cargo build --target wasm32-unknown-unknown` — clean.
- `cargo test --bin axenstax-engine` — all green.
- No public-API change visible to callers.
- `check.sh` clean.

---

## Phase 3 — Web Gamepad backend

### Goal

Make the WASM stub backend a real implementation that surfaces connected browser-side gamepads to the engine.

### Browser API

The Gamepad API is poll-based:

```rust
let nav: web_sys::Navigator = web_sys::window().unwrap().navigator();
let pads: js_sys::Array = nav.get_gamepads().unwrap();
// pads is an array of 4 slots; each is either null or a `web_sys::Gamepad`.
for i in 0..pads.length() {
    let pad = pads.get(i);
    if pad.is_null() { continue; }
    let pad: web_sys::Gamepad = pad.dyn_into().unwrap();
    if !pad.connected() { continue; }
    let id = pad.index() as usize;          // 0..3 (browser slot)
    let buttons = pad.buttons();             // GamepadButton list
    let axes = pad.axes();                   // f64 list, length 4 typical (LX, LY, RX, RY)
    let mapping = pad.mapping();             // "standard" if Xbox/PS mapping
    // ...
}
```

### Button mapping (standard mapping)

Browser's "standard" mapping is well-defined ([w3c spec](https://w3c.github.io/gamepad/#remapping)):

| Browser button index | Xbox name | Engine field on `GamepadState` |
|---:|---|---|
| 0 | A | `a_pressed` |
| 1 | B | `b_pressed` |
| 2 | X | (not currently used) |
| 3 | Y | `y_pressed` |
| 4 | LB | `lb_pressed` |
| 5 | RB | `rb_pressed` |
| 6 | LT | `left_trigger` (analog: `pad.buttons()[6].value()`) |
| 7 | RT | `right_trigger` (analog: `pad.buttons()[7].value()`) |
| 8 | Back/Share | (not currently used) |
| 9 | Start | `start_pressed` |
| 10 | L3 (left stick click) | `l3_pressed` |
| 11 | R3 (right stick click) | (not currently used) |
| 12 | D-pad Up | `dpad_up` |
| 13 | D-pad Down | `dpad_down` |
| 14 | D-pad Left | `dpad_left` |
| 15 | D-pad Right | `dpad_right` |

Axes:
- `axes[0]` = left stick X → `left_x` (deadzone + scale matches native)
- `axes[1]` = left stick Y → `left_y` (note: browser axis Y is +down per spec; native gilrs same — verify in Phase 3 unit test)
- `axes[2]` = right stick X → `right_x`
- `axes[3]` = right stick Y → `right_y`

Non-standard mappings (`pad.mapping() != "standard"`) are rare on modern Chromium with Xbox/PS controllers but exist for older/weird hardware. **Strategy**: if mapping is not "standard", log a warn once and don't expose that controller. Phase 7 playtest will reveal whether any of Axolittle's hardware needs a remapping fallback.

### Hot-plug

`web_sys::Window::add_event_listener_with_callback` for `"gamepadconnected"` and `"gamepaddisconnected"`. Both fire `GamepadEvent`s with a `gamepad: Gamepad` field. Set internal flags on connect/disconnect; the `poll()` method's per-frame walk picks up state changes.

The browser only surfaces a gamepad **after** the user presses a button on it (anti-fingerprinting). This is fine — the engine's "press A to join" rule means the first poll that sees the controller fires on the same frame the user pressed A. Matches the native flow.

### Sticks: deadzone and normalisation

`STICK_DEADZONE_INNER = 0.15` and `STICK_DEADZONE_OUTER = 0.95` — same constants as the native backend (top of `gamepad.rs`). The web backend reads `axes[N]` as `f64`, casts to `f32`, applies the same deadzone math. Behaviour parity is the goal.

### Double-tap A and sprint toggle

`last_a_time: Option<Instant>` and `toggle_flight: bool` — same fields, same logic. **But `Instant`** on `wasm32-unknown-unknown` panics on construction historically; verify with Phase 2's existing `last_frame_instant` pattern (`main.rs:260` already uses `Instant` on WASM, so it's evidently working in this engine's current toolchain — `web-time` crate or similar shim). Reuse whatever pattern the rest of the engine uses.

### Implementation outline

1. Add `web-sys` features in `Cargo.toml`: `"Gamepad"`, `"GamepadButton"`, `"GamepadMappingType"`, `"GamepadEvent"`, `"Navigator"`.
2. In `gamepad.rs`'s `#[cfg(target_arch = "wasm32")] mod web`:
   - `struct WebBackend { gamepads: Vec<GamepadState>, /* hot-plug flags */ }`.
   - `pub fn new() -> Self` — installs the `gamepadconnected`/`gamepaddisconnected` listeners (or skips and relies on poll-only; the spec says poll is sufficient).
   - `pub fn poll(&mut self)` — calls `nav.get_gamepads()`, walks the slots, fills `GamepadState` entries. Resets per-frame button presses at the top of `poll()` to match the native pattern.
   - `pub fn to_intent(&self, idx: usize, dt: f32) -> Option<PlayerIntent>` — identical to native's version since it operates on `GamepadState`. Lift to module-level if it's currently on `NativeBackend`.
   - `pub fn states(&self) -> &[GamepadState]`.

### Tests

Unit tests (under `#[cfg(target_arch = "wasm32")] mod tests` with `wasm-bindgen-test`):

- `web_backend_no_gamepads` — fresh `WebBackend::new()`, poll, assert `states().is_empty()` (or all-disconnected).
- `web_backend_button_mapping` — mock the `get_gamepads()` array (via a JS test fixture or by feeding `GamepadState` directly through a test-only constructor), verify A maps to `a_pressed`, etc.
- `web_backend_deadzone` — feed stick values 0.10, 0.20, 0.50, 0.99 into the analog path, assert deadzone applied correctly. Numerically identical to native.

Manual smoke (Phase 6 / 7) — a real controller in Chromium triggers `a_pressed = true` when A is pressed, sticks track in-engine camera as expected.

### Acceptance

- All unit tests green (`cargo test --target wasm32-unknown-unknown`).
- Manual: WASM build, plug in a controller, press A — the existing "Press A to join" rule (lifted in Phase 4) seats Player 2 on the PWA.

---

## Phase 4 — Lift the native-only `#[cfg]` gates

### Goal

Audit every `#[cfg(not(target_arch = "wasm32"))]` gate in `main.rs`, `game_loop.rs`, `chunk_stream.rs`, and (if applicable) `menu.rs` / `hud_ui.rs`. Classify each. Lift only the ones that were gated *because of gamepads*.

### Method

1. `grep -n 'cfg(not(target_arch = "wasm32"))'` across the four files.
2. For each hit, read the surrounding 10 lines, write a one-line classification in a comment in this spec (or a separate audit doc). Don't lift in bulk.
3. Classes:
   - **LIFT** — was only native-only because gamepads were native-only.
   - **KEEP** — depends on something genuinely native-only (HostedServer, QUIC, rodio audio, env_logger, pollster).
   - **REVIEW** — unclear; ask before lifting.

### Expected lifts (from the inventory above)

- `main.rs:227-228` — the `gamepad` field. Lift.
- `main.rs:349-360` — gamepad init calls. Lift.
- `game_loop.rs:1028` — the join rule. Lift.
- `chunk_stream.rs:153` — save pre-allocate loop. Lift.

### Expected keeps

- `main.rs:240-250` (HostedServer + RemoteClient + remote_players + pending_block_changes) — keep, all QUIC-based.
- All `#[cfg(target_arch = "wasm32")]` blocks for touch input, frame samples, last_frame_instant — keep, WASM-specific.

### Tests

After lifts:
- `cargo build` native — clean.
- `cargo build --target wasm32-unknown-unknown` — clean.
- All existing integration tests green on native.
- New: a WASM-side integration test (under `wasm-bindgen-test`) constructing a `GameState` and asserting `players.len() == 1` initially and that pushing a second `PlayerSlot` + recomputing screens yields a valid 2-screen layout. (Doesn't need a real gamepad — exercises the layout solver, which already works on both targets but was gated indirectly by gamepad gating.)

### Acceptance

- Both build targets clean.
- All existing tests green.
- Diff inspection: every removed `#[cfg]` annotated in the commit message with its class.
- `check.sh` clean.

---

## Phase 5 — WASM-side wiring

### Goal

Make the lifted code actually work end-to-end on WASM: join flow, touch+gamepad coexistence, fullscreen for 4 viewports.

### Join flow on WASM

The native join flow (`game_loop.rs:1029-1068`, post-#14 raised to `< 4`) calls into pure functions for spawn position + GPU resource creation + screen layout recompute + toast. None of this is target-specific. After Phase 4 lifts the outer `#[cfg]`, the join flow runs on WASM unchanged — *if* the gamepad backend is real (Phase 3).

The one WASM-specific bit: the join toast string. The existing toast renders via egui, which works on both targets. No change.

### Touch + gamepad coexistence

The existing `touch_input.rs` provides P1 input on touch devices (virtual joystick, action buttons). On a touchscreen-only device, the game is touch-driven P1. If gamepads connect to the same device:

| P1 input | P2-P4 input | Scenario |
|----------|-------------|----------|
| Touch | — | Phone/tablet, no controllers |
| Touch | Gamepad[0..2] | Tablet + USB-C hub + gamepads (rare but supported) |
| KB+M | — | Laptop, no controllers |
| KB+M | Gamepad[0..2] | Laptop, couch co-op via Chromecast / external display |
| KB+M + Gamepad[0] merged | Gamepad[1..3] | Same as native: P1 has KB+M, also gets Gamepad[0] inputs merged |

The merging logic from #14 Phase 4 (native intent routing) works as-written on WASM once `GamepadSystem` is real. No change in `game_loop.rs`'s intent collector. Touch is its own input source for P1 specifically — it doesn't merge with gamepads (since holding a controller and touching the screen is a weird combo that's not worth designing for).

### Fullscreen behaviour

When the user clicks "Play" in the PWA, the existing fullscreen request fires (via `web_sys::Element::request_fullscreen`). For 4-viewport rendering, fullscreen is strongly recommended — 1920×1080 windowed shrinks each viewport to ~480×270, the lower bound. Phase 5 of #14 sets the recommended minimum.

If the user enters 4-player mode in a windowed PWA tab smaller than 1280×720, show a toast: *"For 4-player split-screen, try fullscreen (F11) or a larger window."* Non-blocking — the game still renders, just at the floor. No new code path; the toast is a one-liner inserted at the same place the join completes.

### Tests

- A WASM integration test (under `wasm-bindgen-test`) that spawns a 4-player `GameState` and asserts the screen layout is 2×2 quad. This exercises the entire lifted path end-to-end at the unit level.
- Manual: real PWA in Chromium, plug in 3 controllers, press A on each — each joins. Quad layout appears at the 4-player step.

### Acceptance

- WASM integration test green.
- Manual: PWA quad join flow works.

---

## Phase 6 — WASM perf verification

### Goal

Confirm 4-viewport rendering on WASM hits an acceptable frame-time budget on representative hardware.

### Target budgets

Per `docs/research/2026-04-01-multiscreen-layout.md` and the 20 TPS engine spec:

| Hardware | Resolution | Target frame time | Target tick time |
|----------|-----------|---:|---:|
| M1 MacBook Air, Chromium | 1920×1080 fullscreen | < 8 ms (= 120 FPS headroom; vsync caps at 60) | < 5 ms |
| Chromebook (Intel N100) | 1920×1080 fullscreen | < 16 ms (= 60 FPS sustained) | < 10 ms |
| Pi 5 + Chromium | 1280×720 fullscreen | < 33 ms (= 30 FPS sustained, acceptable) | < 15 ms |

These are budgets, not promises. If a target is missed, document it and decide: optimise, document the floor, or recommend a higher spec.

### Measurement

The engine already has `frame_samples` and `tick_samples` rolling windows on WASM (`main.rs:254-265`), fed by `update_and_render` and `tick` respectively, consumed by `wasm_feedback::build_snapshot`.

Add a debug overlay (gated behind an existing debug key like F3) that prints the rolling-mean frame time and tick time over the last 5 seconds. Use `frame_samples` and `tick_samples` directly. The overlay sits in one viewport's corner.

For each hardware target above:
1. Boot the PWA, fullscreen, join 3 controllers (4 players total).
2. Walk around for 60 seconds in a moderately complex area (chunk streaming firing, mob AI firing).
3. Read the rolling means.
4. Record in this spec's appendix below.

### Phase 6 wiring (delivered 2026-05-17)

Wired the existing rolling-window samples into the F3 debug overlay.
`hud_ui::PerfSamples` carries the mean frame time, worst-case frame
time, and mean tick time over the rolling window (300 frame samples
≈ 5 s at 60 FPS, 100 tick samples = 5 s at 20 TPS). The samples are
populated on WASM only — native still shows the un-augmented overlay.

Toggle the overlay with the existing F3 debug key. The perf line
appears below the existing XYZ / Time / Target lines in player 0's
viewport (every viewport gets the same numbers; they're engine-wide).

### Perf-results appendix (Phase 7 fills in)

The actual measurements happen during the playtest because they need
a real browser session with 4 controllers + 60 seconds of in-game
walking in a chunk-streaming area. Numbers go below.

| Hardware | Resolution | Mean frame time | Worst frame time | Mean tick time | Pass? |
|----------|-----------|---:|---:|---:|:---:|
| (fill in during Phase 7) | | | | | |

### Acceptance

- Numbers recorded for at least the dev's primary hardware.
- If any target misses, an entry in "open questions" below captures next steps.

---

## Phase 7 — Axolittle 4-player PWA playtest

### Goal

Real couch co-op. Two laptops + 4 controllers, or one laptop + 4 controllers via a USB hub. Find what breaks.

### Setup

- A laptop running Chromium + the PWA fullscreen.
- 4 paired gamepads (USB or BT).
- Axolittle + 1-3 friends/siblings.
- An area in a saved world with enough variety to fill 30 minutes (build, fight, explore, craft).

### What to watch for

- Gamepad disconnect/reconnect behaviour during a session.
- "Press A to join" discoverability on PWA (does the empty-quadrant prompt from #14 render correctly via WebGPU?).
- Frame pacing during chunk streaming with 4 players spread out.
- Crafting UI in a quadrant at the laptop's native resolution.
- Browser-specific weirdness: tab focus loss, fullscreen escape via Esc (which kills pointer lock).
- Comparison to native 4-player from #14's Phase 6 — feel parity, or worse on web?

### Outcome

Tuning notes commit. Likely follow-ups: pointer-lock recovery, gamepad-disconnect UX, perf tweaks if a Chromebook is the playtest target.

---

## Open questions / future work

- **Non-standard gamepad mapping.** Phase 3 's "log warn + ignore" stance may surface real hardware (cheap third-party controllers) that needs a remapping layer. If Phase 7 hits this, write a tiny mapping table for known offenders.
- **Touch+gamepad in the same session.** Phase 5 declares P1=touch is exclusive (no merge with gamepads), but a touchscreen laptop is a real shape. Validate in Phase 7 if available.
- **`Instant` semantics on WASM.** Verified to work elsewhere in the engine (frame samples), but if it turns out the existing pattern is a `web-time` shim, the gamepad backend should use the same shim, not raw `std::time::Instant`. Phase 3 checks.
- **WebRTC multiplayer + WASM split-screen.** Composes — same model as native (HostedServer is N-shaped). Lands when the WebRTC transport spec lands. Out of scope here.
- **Per-player audio output** — impossible on web. Documented limitation; not a follow-up.

---

## Memory rule check

- uk english naming — toasts ("For 4-player split-screen, try fullscreen (F11) or a larger window."), all UK English. ✓
- autonomy to playtest boundary — Phases 2-6 solo-verifiable, Phase 7 = explicit playtest gate. ✓
- pretest check — Phase 6 produces real numbers; no "should be fine" hand-waves. ✓
- signet boundary — gamepad-on-web backend is engine-generic, not AxeNStax-specific. The reusable bit (web `GamepadBackend`) lifts to other Decented web games unchanged. ✓
- pwa priority — PWA is the alpha target; this spec is squarely on it. ✓
- shared infra strategy — `gamepad.rs::web` module + the trait/concrete split are engine-generic. ✓

---

## Out of scope (explicitly)

- WebRTC voxel multiplayer (separate spec).
- Per-screen audio routing on web (browser limitation).
- Multi-window / multi-monitor on web (browser limitation).
- Touch routing for P2-P4 (not feasible with one touch surface).
- Mobile Chromium quad play (phone screens are too small; tablet+gamepads is the smallest reasonable shape).
- Non-Chromium browser support (Firefox/Safari are out of scope per ADR-003).

---

## Dependency note

This spec **must not land before #14**. The layout solver, join cap, intent routing, HUD legibility, and 4-player save round-trip test all come from #14 and are pure cross-platform Rust — that work is identical on native and WASM. This spec exists *only* to lift the WASM gates and add the missing web Gamepad backend. If #14 is in flight, hold this one in the queue.

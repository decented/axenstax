# WASM/WebGPU Build — Design Spec

**Date**: 2026-04-02
**Status**: Design
**Phase**: Originally Phase 4, proposed to pull forward
**Trigger**: Alpha tester on ChromeOS touchscreen, no Crostini support — web build is the only path

---

## Cart Before Horse Assessment

### Honest risks of doing this now

1. **The game might change significantly.** Axolittle hasn't tested split-screen yet. His feedback could trigger big refactors. Porting to web before the gameplay stabilises means porting twice.

2. **Maintenance burden doubles.** Every engine change needs testing on both native and web. Touch controls are a separate input path that needs maintaining.

3. **WebGPU is still maturing.** Chrome supports it, but edge cases exist. Chromebook GPU drivers vary wildly. Some will work, some won't.

4. **It's not the critical path.** Phase 1 (multiplayer) and Phase 2 (hosted worlds) are what make the platform work. The web build is distribution, not foundation.

### Honest reasons to do it now anyway

1. **The alpha tester can't play.** Her Chromebook doesn't support Crostini. Without the web build, she has no access. Broken feedback loop for half the test team.

2. **The codebase is surprisingly web-ready.** After auditing:
   - Filesystem access is ONLY in `save.rs` — one file to stub
   - Audio is ONLY in `audio.rs` — already has `new_silent()` fallback
   - Gamepad (gilrs) is ONLY in `gamepad.rs` — compile-gate it, touch replaces it
   - Networking (tokio, quinn, rcgen, rustls) is ONLY in `network.rs`, `hosted_server.rs`, `remote_client.rs`, `discovery.rs` — all compile-gated out for web
   - wgpu supports WebGPU backend natively
   - winit 0.30 supports web canvas target
   - egui supports web
   - Textures are procedurally generated — NO asset pipeline needed
   - No filesystem texture loading means no HTTP asset fetching
   - `pollster::block_on` is only in `main.rs` — replace with `wasm_bindgen_futures::spawn_local`

3. **It's a clean separation.** The web build is a compile target, not a fork. Same game loop, same renderer, same physics. Only the edges change (input, audio, save, window creation).

4. **It proves the platform story early.** "Play in a browser, no install" is a massive distribution advantage.

### Verdict: Do it, but MINIMAL viable web build

Don't build the full Phase 4 web client (asset streaming, WebRTC multiplayer, PWA, offline mode). Build the minimum that gets her playing:

- Engine renders in Chrome via WebGPU
- Touch controls work (move, look, break, place, jump)
- Single player only (no networking on web yet)
- No save/load (LocalStorage can come later)
- No audio (silent — already supported)
- No gamepad on web (touch is the input)

This is a weekend of work, not weeks. The heavy stuff (multiplayer, save/load, audio, PWA) stays in Phase 4.

---

## What Changes

### Files that need `#[cfg]` gating

| File | Native | Web | Change |
|------|--------|-----|--------|
| `main.rs` | `pollster::block_on`, `EventLoop` | `wasm_bindgen_futures`, web canvas | Platform-specific entry point |
| `audio.rs` | `rodio` | Silent (`new_silent()`) | `#[cfg(not(target_arch = "wasm32"))]` on rodio imports |
| `gamepad.rs` | `gilrs` | Disabled (touch replaces it) | `#[cfg(not(target_arch = "wasm32"))]` on whole module |
| `save.rs` | `std::fs` | Stubbed (no save on web MVP) | `#[cfg(not(target_arch = "wasm32"))]` on fs operations |
| `network.rs` | `quinn`, `tokio` | Disabled | `#[cfg(not(target_arch = "wasm32"))]` on whole module |
| `hosted_server.rs` | Server thread | Disabled | Same |
| `remote_client.rs` | QUIC client | Disabled | Same |
| `discovery.rs` | UDP broadcast | Disabled | Same |

### Files that work as-is (no changes)

| File | Why it works |
|------|-------------|
| `renderer.rs` | wgpu targets WebGPU automatically when compiled to WASM |
| `game_loop.rs` | Pure game logic, no platform deps |
| `camera.rs` | Pure math |
| `physics.rs` | Pure math |
| `world.rs` | Pure data structures |
| `chunk.rs` | Pure data |
| `mesh.rs` | Pure geometry |
| `block.rs` | Pure data |
| `inventory.rs` | Pure data |
| `combat.rs` | Pure logic |
| `crafting.rs` | Pure logic |
| `craft_ui.rs` | egui (web-compatible) |
| `hud_ui.rs` | egui (web-compatible) |
| `menu.rs` | egui (web-compatible) |
| `splash_ui.rs` | egui (web-compatible) |
| `screen.rs` | Pure data |
| `player_intent.rs` | Pure data |
| `player_slot.rs` | Pure data |
| `raycast.rs` | Pure math |
| `texture_gen.rs` | Pure procedural generation |
| `biome.rs` | Pure math |
| `entity.rs` | Pure data |
| `mob.rs` | Pure data |
| `mob_ai.rs` | Pure logic |
| `entity_model.rs` | Pure geometry |
| `block_interact.rs` | Pure logic |
| `spawning.rs` | Pure logic |
| `chunk_stream.rs` | Pure logic |
| `water.rs` | Pure logic |
| `leaf_decay.rs` | Pure logic |
| `egui_integration.rs` | egui-wgpu supports web |
| `protocol.rs` | Pure data + serde |

**That's 30+ files that work unchanged.** Only 8 files need gating, and most of those are just "disable the whole module on web."

### New files for web

| File | Purpose | Lines |
|------|---------|-------|
| `web_main.rs` | WASM entry point, canvas setup, event loop | ~80 |
| `touch_input.rs` | Touch event handling → PlayerIntent | ~200 |
| `index.html` | Host page with canvas + touch overlay | ~60 |

---

## Touch Controls Design

### Layout (portrait-ish Chromebook, landscape works too)

```
┌─────────────────────────────────────────────┐
│                                             │
│              GAME WORLD                     │
│           (wgpu canvas)                     │
│                                             │
│                                 [JUMP]      │
│   [JOYSTICK]                                │
│   (move)         [LOOK AREA]    [BREAK]     │
│                  (drag = look)  [PLACE]     │
│                                             │
│         [1][2][3][4][5][6][7][8][9]         │
│              (hotbar)                        │
└─────────────────────────────────────────────┘
```

### Touch zones

| Zone | Area | Action | Mechanic |
|------|------|--------|----------|
| **Left joystick** | Bottom-left 25% | Move (WASD equivalent) | Virtual joystick: thumb drags from centre, distance = speed |
| **Look area** | Centre + right 60% | Camera look | Drag = rotate camera (like Minecraft PE) |
| **Jump button** | Right side, above break | Jump / toggle flight | Tap = jump, double-tap = toggle flight |
| **Break button** | Bottom-right | Break block | Tap = single hit, hold = continuous break |
| **Place button** | Bottom-right, below break | Place block | Tap = place |
| **Hotbar** | Bottom strip | Select block | Tap slot 1-9 |
| **Pause** | Top-right corner | Pause menu | Tap |
| **Inventory** | Above hotbar, small icon | Open inventory | Tap (opens egui inventory) |

### Touch → PlayerIntent mapping

```rust
pub struct TouchInput {
    // Virtual joystick
    joystick_active: bool,
    joystick_origin: (f32, f32),  // where thumb first touched
    joystick_current: (f32, f32), // where thumb is now
    
    // Look
    look_active: bool,
    look_prev: (f32, f32),
    
    // Buttons
    jump_pressed: bool,
    break_held: bool,
    place_pressed: bool,
    hotbar_select: Option<usize>,
    pause_pressed: bool,
    inventory_pressed: bool,
}

impl TouchInput {
    pub fn to_intent(&self) -> PlayerIntent {
        let dx = self.joystick_current.0 - self.joystick_origin.0;
        let dy = self.joystick_current.1 - self.joystick_origin.1;
        let max_radius = 60.0; // pixels
        
        PlayerIntent {
            move_forward: (-dy / max_radius).clamp(-1.0, 1.0),
            move_right: (dx / max_radius).clamp(-1.0, 1.0),
            look_dx: /* from look drag delta */ ,
            look_dy: /* from look drag delta */ ,
            jump_pressed: self.jump_pressed,
            break_block: self.break_held,
            place_block: self.place_pressed,
            hotbar_select: self.hotbar_select,
            pause: self.pause_pressed,
            toggle_inventory: self.inventory_pressed,
            ..Default::default()
        }
    }
}
```

### Touch rendering

The virtual joystick and buttons are drawn as egui overlays on top of the game — same as the HUD. Semi-transparent circles and icons. They render in the egui pass, after the world.

---

## Cargo.toml Changes

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
rodio = { version = "0.20", default-features = false, features = ["vorbis"] }
gilrs = "0.11"
tokio = { version = "1", features = ["rt-multi-thread", "net", "sync", "time", "macros"] }
quinn = "0.11"
rcgen = "0.13"
rustls = { version = "0.23", default-features = false, features = ["ring", "std"] }
pollster = "0.4"

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
web-sys = { version = "0.3", features = [
    "Window", "Document", "HtmlCanvasElement", "Element",
    "TouchEvent", "TouchList", "Touch",
    "console",
] }
console_error_panic_hook = "0.1"
```

The key: native-only deps (`rodio`, `gilrs`, `tokio`, `quinn`, etc.) move behind `cfg(not(wasm32))`. Web-only deps (`wasm-bindgen`, `web-sys`) are `cfg(wasm32)`.

---

## Build Process

### Native (unchanged)
```bash
cargo run --release
```

### Web
```bash
# Install trunk (WASM build tool)
cargo install trunk

# Build and serve
trunk serve --release
# Opens http://localhost:8080 with hot-reload
```

Or build and deploy to the website:
```bash
trunk build --release
# Output: dist/index.html + dist/axenstax-engine_bg.wasm
# Copy to website static dir and serve
```

The website at `https://192.168.1.10:8094` could serve the web build directly — add a `/play` route that loads the WASM.

---

## What She Gets (MVP)

- Opens Chrome on her Chromebook
- Goes to `https://192.168.1.10:8094/play`
- Game loads in the browser (no install)
- Touch controls: virtual joystick to move, drag to look, tap to break/place
- Full game: mining, building, crafting, mobs, combat, day/night
- Single player only (no split-screen, no multiplayer on web yet)
- No save (refreshing the page loses the world)
- No audio (silent)

That's enough to play, test, and give feedback. Save, audio, and multiplayer come later.

---

## Effort Estimate

| Task | Lines | Sessions |
|------|-------|----------|
| Cargo.toml cfg gating | ~20 | 0.5 |
| `#[cfg]` gates on 8 files | ~40 | 0.5 |
| `web_main.rs` (WASM entry point) | ~80 | 1 |
| `touch_input.rs` (touch → PlayerIntent) | ~200 | 1 |
| Touch overlay UI (egui) | ~150 | 1 |
| `index.html` host page | ~60 | 0.5 |
| `save.rs` stub for web (no-op) | ~20 | 0.5 |
| Testing + fixes | — | 2 |
| **Total** | **~570** | **~6** |

~570 new/changed lines. Maybe 6 sessions. The vast majority of the engine (30+ files, 10,000+ lines) compiles unchanged.

---

## What This Does NOT Do (Phase 4 Proper)

- Asset streaming over HTTP (not needed — procedural textures)
- WebRTC multiplayer (Phase 4)
- PWA / offline / installable (Phase 4)
- Save/load via LocalStorage or IndexedDB (Phase 4)
- Audio via Web Audio API (Phase 4)
- Gamepad via Gamepad API (Phase 4)
- Mobile-optimised touch (Phase 4 — this MVP is functional, not polished)
- Performance tuning for low-end Chromebook GPUs (Phase 4)

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| WebGPU not available on her Chrome | Low (Chrome 113+ has it) | Blocks entirely | Check `chrome://gpu` first |
| Chromebook GPU too weak | Medium | Poor framerate | Reduce render distance, lower resolution |
| Touch controls feel bad | Medium | She won't play | Iterate based on her feedback — she's the tester |
| wgpu WASM build has bugs | Low-Medium | Build fails | wgpu WASM is well-tested, large community |
| Game changes break web build | Low | Re-test needed | CI can run `cargo check --target wasm32-unknown-unknown` |

### Pre-flight check (before building)

On her Chromebook, open Chrome and go to `chrome://gpu`. Look for:
- **WebGPU**: should say "Enabled" or "Hardware accelerated"
- **Graphics Backend**: should show the GPU model

If WebGPU is disabled, try `chrome://flags/#enable-unsafe-webgpu` and enable it.

If the GPU is listed as "Software only" — the Chromebook's GPU is too weak and the web build won't perform well enough. This is the kill signal for the port.

---

## Recommendation

**Do the MVP web build.** It's ~570 lines, 6 sessions, and it unblocks the alpha tester. The engine is already 95% web-compatible — only the edges need gating. Don't build Phase 4 features (PWA, save, audio, multiplayer). Just get the game rendering in Chrome with touch controls.

**But do it AFTER Axolittle tests split-screen.** His feedback might change the game loop, input system, or HUD — all of which affect the web build. Let him test first (native, on his Linux Mint), incorporate his feedback, then port to web for her.

**Sequence:**
1. Axolittle tests split-screen on Linux Mint (now)
2. Fix anything he finds (next session)
3. Build WASM MVP (session after that)
4. She tests on Chromebook (immediately after)
5. Iterate on touch controls based on her feedback

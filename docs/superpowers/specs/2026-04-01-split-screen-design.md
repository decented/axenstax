# Split-Screen & Multi-Screen Design

**Date**: 2026-04-01
**Status**: Design
**Phase**: 1 (Multiplayer Core)

## Vision

Two players on one screen (split-screen), two players on two screens (dual-screen), or one player plus a companion screen (sidecar: tutorial, stream, spectator camera). All three share the same rendering abstraction. The architecture also supports a future where each screen runs a different Decented game — e.g., two kids in a car, each with their own headrest display, each playing their own game on one box.

### Use Cases

| Config | Screens | Content | When |
|--------|---------|---------|------|
| Single player | 1 full window | LocalPlayer | Now (unchanged) |
| Split-screen | 2 viewports, 1 window | 2x LocalPlayer | Phase 1 |
| Dual-screen, 2 players | 2 windows, 2 monitors | 2x LocalPlayer | Phase 2 |
| Dual-screen, player + sidecar | 2 windows, 2 monitors | LocalPlayer + Companion | Phase 2+ |
| Car mode | 2 windows, 2 HDMI outputs | 2x any game | Future platform |

### Hardware Targets

- **Couch co-op**: TV + 2 controllers. Split-screen.
- **Desk setup**: PC + 2 monitors + 2 controllers. Each player gets full screen.
- **Pi 5 / N100 console box**: Dual HDMI + 2 controllers. Either split on one TV or one player per TV.
- **Car rig**: N100 mini PC under seat, dual HDMI to headrest screens, 2 controllers, 2 USB/BT headphones.

---

## 1. Core Abstractions

### ViewportRect

Where a screen renders — a pixel region within a window's surface.

```rust
#[derive(Clone, Copy, Debug)]
pub struct ViewportRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl ViewportRect {
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }

    pub fn full(window_width: u32, window_height: u32) -> Self {
        Self { x: 0, y: 0, width: window_width, height: window_height }
    }
}
```

### ScreenContent

What feeds a screen. Only `LocalPlayer` exists in Phase 1; the enum is the extension point.

```rust
pub enum ScreenContent {
    /// A local player's first-person view into the shared world.
    /// Index into GameServer.players.
    LocalPlayer(usize),

    // Future variants:
    // Spectator(SpectatorConnection),  // Phase 2: read-only view of remote server
    // Companion(CompanionKind),        // Phase 3: tutorial, map, recipe book
    // WebView(url),                    // Phase 3: embedded stream, wiki
}
```

### Screen

A viewport rect paired with its content source.

```rust
pub struct Screen {
    pub viewport: ViewportRect,
    pub content: ScreenContent,
}
```

### Layout Computation

Replaces the existing `viewports()` function in `player_slot.rs`. Called on window resize or player join/leave.

```rust
pub fn compute_screen_layout(
    num_local_players: usize,
    window_width: u32,
    window_height: u32,
) -> Vec<Screen> {
    match num_local_players {
        1 => vec![Screen {
            viewport: ViewportRect::full(window_width, window_height),
            content: ScreenContent::LocalPlayer(0),
        }],
        2 => {
            let half_w = window_width / 2;
            vec![
                Screen {
                    viewport: ViewportRect { x: 0, y: 0, width: half_w, height: window_height },
                    content: ScreenContent::LocalPlayer(0),
                },
                Screen {
                    viewport: ViewportRect { x: half_w, y: 0, width: window_width - half_w, height: window_height },
                    content: ScreenContent::LocalPlayer(1),
                },
            ]
        }
        _ => unimplemented!("3-4 player layouts: Phase 2"),
    }
}
```

**Split orientation**: Side-by-side (vertical divider) for 2 players. Horizontal split (top/bottom) wastes peripheral vision in a first-person game. Side-by-side gives each player a tall, narrow view which better matches natural FOV.

**Note**: The existing `Viewport` struct and `viewports()` in `player_slot.rs` will be replaced by `ViewportRect`, `Screen`, and `compute_screen_layout`. The NDC coordinates in the old struct are not needed — `wgpu::RenderPass::set_viewport()` takes pixel coordinates directly.

---

## 2. Renderer Refactor

### Per-Player GPU Resources

Each local player needs their own camera uniform buffer and bind group. Created once when a player joins, destroyed when they leave.

```rust
pub struct PlayerGpuResources {
    pub camera_buffer: wgpu::Buffer,
    pub camera_bind_group: wgpu::BindGroup,

    // Per-player frame data (updated each frame)
    pub entity_buffer: Option<wgpu::Buffer>,
    pub entity_vertex_count: u32,
    pub wire_buffer: Option<wgpu::Buffer>,
    pub wire_vertex_count: u32,
}
```

The `Renderer` gains:
- `player_gpu: Vec<PlayerGpuResources>` — indexed by player index
- `create_player_resources(&mut self) -> PlayerGpuResources`
- `remove_player_resources(&mut self, index: usize)`

### Shared Resources (No Change)

These remain single-instance, shared across all viewports:
- `chunk_meshes` / `water_meshes` — world geometry, uploaded once
- `texture_bind_group` — block texture array
- All pipelines (`render_pipeline`, `water_pipeline`, `entity_pipeline`, `wire_pipeline`, `crosshair_pipeline`)
- `depth_texture_view` — one depth buffer for the whole framebuffer, partitioned by viewport rect

### render_world_viewport()

New method that renders the world from one camera into one viewport rect. This is the core of the refactor — the existing `render()` method's world-rendering code moves here.

```rust
fn render_world_viewport(
    &self,
    encoder: &mut wgpu::CommandEncoder,
    target_view: &wgpu::TextureView,
    gpu: &PlayerGpuResources,
    viewport: &ViewportRect,
    sky_color: [f32; 3],
    is_first_viewport: bool,  // first viewport clears, rest load
) {
    // Each viewport gets its own render passes with set_viewport + set_scissor_rect.
    //
    // Pass 1: Opaque chunks
    //   - LoadOp: Clear if first viewport, Load otherwise
    //   - set_viewport(x, y, w, h, 0.0, 1.0)
    //   - set_scissor_rect(x, y, w, h)
    //   - Bind gpu.camera_bind_group (this player's camera)
    //   - Draw all chunk_meshes
    //
    // Pass 1.5: Water (same viewport/scissor)
    // Pass 1.7: Entities (gpu.entity_buffer — this player's visible mobs)
    // Pass 2: Wireframe highlight (gpu.wire_buffer — this player's target block)
    // Pass 3: Crosshair (centred in this viewport, not screen centre)
}
```

### Updated render()

The top-level `render()` becomes an orchestrator:

```rust
fn render(
    &mut self,
    window: &Window,
    screens: &[Screen],
) {
    let output = self.surface.get_current_texture();
    let view = output.texture.create_view(&Default::default());
    let mut encoder = self.device.create_command_encoder(&Default::default());

    for (i, screen) in screens.iter().enumerate() {
        match &screen.content {
            ScreenContent::LocalPlayer(player_idx) => {
                let gpu = &self.player_gpu[*player_idx];
                self.render_world_viewport(
                    &mut encoder,
                    &view,
                    gpu,
                    &screen.viewport,
                    self.sky_color,
                    i == 0,
                );
            }
        }
    }

    // egui pass: renders all per-viewport HUDs in one pass (see Section 4)
    self.egui.end_frame(&self.device, &self.queue, &mut encoder, &view, self.width, self.height);

    self.queue.submit(std::iter::once(encoder.finish()));
    output.present();
}
```

### Crosshair Per-Viewport

The crosshair is currently a fixed buffer created at init, centred on the screen. For split-screen, each viewport needs a crosshair centred in its own rect. Two approaches:

**Option A**: Generate crosshair vertices per viewport each frame (cheap — 12 vertices).
**Option B**: Use a shader uniform for viewport offset and render the same buffer.

**Decision**: Option A. 12 vertices is negligible. The crosshair pipeline already exists; just compute NDC centre relative to the viewport rect.

### Depth Buffer

One depth texture at full window resolution. Each viewport's `set_viewport()` maps its NDC depth to its pixel region. No interference between viewports because they don't overlap. The first viewport's passes use `LoadOp::Clear(1.0)`, subsequent viewports use `LoadOp::Load` — the clear already set the entire depth buffer to 1.0 (far plane).

### Performance

| Metric | 1 Player | 2 Players Split |
|--------|----------|-----------------|
| Chunk draw calls | N | 2N (same meshes, 2 cameras) |
| GPU mesh memory | M | M (shared) |
| Fragment count | W*H | W*H (each viewport is half area) |
| Camera uniforms | 1 buffer | 2 buffers (~256 bytes each) |
| Entity buffers | 1 | 2 (different visibility per camera) |
| Net GPU load | 1.0x | ~1.0-1.2x |

The dominant cost is draw calls (2x), but each call processes half the fragments. Net GPU load is roughly equivalent to single player.

---

## 3. Input System

### Multi-Gamepad

The current `GamepadSystem` tracks only one active controller (`active: Option<GamepadId>`). This must become multi-controller.

```rust
pub struct GamepadSystem {
    gilrs: Gilrs,
    /// All connected controllers, ordered by connection time.
    /// Index 0 = first controller connected = Player 1's gamepad (if keyboard is not P1).
    gamepads: Vec<GamepadState>,
}

pub struct GamepadState {
    pub id: GamepadId,
    pub connected: bool,
    // All per-controller state that currently lives in GamepadSystem:
    pub right_x: f32, pub right_y: f32,
    pub left_x: f32, pub left_y: f32,
    pub a_pressed: bool, pub b_pressed: bool, // ... etc
    pub dpad_up: bool, pub dpad_down: bool, // ... etc
    pub left_trigger: f32, pub right_trigger: f32,
    pub last_a_time: Option<Instant>,
    pub toggle_flight: bool,
    pub sprint_on: bool,
    pub disconnected_this_frame: bool,
}
```

### Player-to-Input Routing

Player 0 always gets keyboard+mouse. If a gamepad is also connected, Player 0 merges keyboard + gamepad 0 (current behaviour when only 1 player). When a second controller connects, it becomes Player 1's exclusive input.

| Players | Player 0 Input | Player 1 Input |
|---------|---------------|----------------|
| 1 (no gamepad) | Keyboard + Mouse | — |
| 1 (gamepad) | Keyboard + Mouse + Gamepad 0 (merged) | — |
| 2 (1 gamepad) | Keyboard + Mouse | Gamepad 0 |
| 2 (2 gamepads) | Keyboard + Mouse + Gamepad 0 (merged) | Gamepad 1 |

In the 2-gamepad case, Player 0 still has keyboard+mouse as a fallback (useful if they set down the controller). Keyboard always routes to Player 0 regardless of gamepad count.

### Player 2 Join Flow

When a second gamepad connects:

1. If in menu: the world creation screen gains a "2 Players" option. Both players load into the new world.
2. If mid-game (single player): the game pauses and shows a prompt: "Player 2 wants to join. Start split-screen?" If accepted, Player 2 spawns near Player 1 (3 blocks offset, same Y), the screen layout switches to side-by-side, and the game resumes.
3. If a gamepad disconnects mid-game: that player's half shows "Controller disconnected — reconnect to continue" and the other player can keep playing. If Player 0's keyboard+mouse is still active, they are unaffected.

When loading a world saved with 2 players, both spawn at their saved positions regardless of how many controllers are connected — Player 1 can play solo in a 2-player world.

### Intent Generation

Each player gets their own `PlayerIntent` per tick:

```rust
fn collect_intents(
    input: &InputState,
    gamepads: &GamepadSystem,
    num_players: usize,
    dt: f32,
) -> Vec<PlayerIntent> {
    match num_players {
        1 => {
            // Current behaviour: merge keyboard + first gamepad
            let mut intent = input.to_intent();
            if let Some(gp) = gamepads.to_intent(0, dt) {
                intent.merge(gp);
            }
            vec![intent]
        }
        2 => {
            // Player 0: keyboard+mouse (or gamepad 0 if 2 gamepads)
            // Player 1: gamepad 0 (or gamepad 1 if 2 gamepads)
            // ... routing logic per table above
        }
        _ => unimplemented!(),
    }
}
```

### Mouse Ownership

In split-screen on one window, the mouse is always Player 0's. Player 1 uses a controller. Cursor capture applies to the whole window as before — Player 0's look-around still works via raw mouse delta.

For future dual-screen (two windows), each window could own its own mouse — but this is Phase 2 and requires OS-level input routing.

---

## 4. Per-Viewport egui HUD

### Strategy

One `egui::Context`, one `egui_wgpu::Renderer`, one frame per tick — but HUD elements are drawn with explicit position offsets so each player's HUD sits inside their viewport.

The HUD drawing functions (`draw_hotbar`, `draw_hearts`, `draw_block_name`, `draw_debug_overlay`) currently use `egui::Area` with anchors like `CENTER_BOTTOM`. These anchors are relative to the full egui screen. For split-screen, each function gains a `viewport: ViewportRect` parameter and positions elements relative to that rect instead of the full screen.

```rust
pub fn draw_hud(
    ctx: &egui::Context,
    viewport: &ViewportRect,  // NEW: which region this HUD belongs to
    player: &PlayerSlot,
    block_textures: &[egui::TextureId],
    debug_on: bool,
) {
    draw_hotbar(ctx, viewport, &player.inventory, player.hotbar_slot, block_textures);
    draw_hearts(ctx, viewport, player.combat.health);
    draw_block_name(ctx, viewport, &player.inventory, player.hotbar_slot);
    if debug_on {
        draw_debug_overlay(ctx, viewport, &player.player, &player.camera);
    }
}
```

### Positioning

Instead of `Anchor::CENTER_BOTTOM`, use `egui::Area::new(...).fixed_pos(pos)` where `pos` is computed from the viewport rect:

```rust
// Example: hotbar at bottom-centre of viewport
let hotbar_x = viewport.x as f32 + viewport.width as f32 / 2.0 - hotbar_width / 2.0;
let hotbar_y = viewport.y as f32 + viewport.height as f32 - hotbar_height - 16.0;
egui::Area::new(egui::Id::new(("hotbar", player_index)))
    .fixed_pos(egui::pos2(hotbar_x, hotbar_y))
    // ...
```

Each player's HUD elements get a unique `egui::Id` incorporating the player index to avoid ID collisions.

### Menu / Pause Screens

Menus (splash, world select, pause) remain full-screen — they're not per-player. When any player presses pause, the game pauses for all players and the pause menu renders over the full window. This matches console game convention (one player pausing pauses for everyone).

### Crafting UI

Crafting UI opens per-player. When Player 1 opens their crafting table, it renders centred in their viewport. Player 0 continues playing. The crafting UI functions gain the same `viewport` parameter as the HUD.

---

## 5. Game Loop Changes

### GameState Changes

```rust
pub struct GameState {
    // World (shared)
    pub server: GameServer,          // owns World, ECS, water, leaf decay
    pub world_time: u32,

    // Players (per-player)
    pub players: Vec<PlayerSlot>,    // position, camera, inventory, combat
    pub screens: Vec<Screen>,        // viewport layout

    // Input (refactored)
    pub input: InputState,           // keyboard+mouse (always Player 0)
    pub gamepads: GamepadSystem,     // all controllers

    // Rendering
    pub renderer: Renderer,          // owns player_gpu Vec
    pub window: Arc<Window>,

    // Mode
    pub mode: GameMode,              // Splash, Menu, Playing, Paused
}
```

### Tick Loop (20 TPS)

```
tick():
    1. Advance world_time
    2. Autosave check
    3. Mob spawning
    4. Collect intents: Vec<PlayerIntent> from keyboard + gamepads
    5. For each player:
        a. Apply intent to player physics (movement, collision, jumping)
        b. Camera follows player: camera.position = player.eye_pos()
    6. Water/leaf decay simulation
    7. Chunk mesh rebuilds (dirty chunks)
    8. Mob AI, entity physics
    9. For each player:
        a. Combat (melee range check per player)
```

### Frame Loop (vsync)

```
update_and_render():
    1. Poll gamepads
    2. Handle mode (splash/menu/pause/playing)
    3. If playing:
        a. Stream chunks (union of all players' positions)
        b. Run tick accumulator
        c. For each player:
            - Mouse look / gamepad look (per-frame interpolation)
            - Hotbar selection
            - Block raycast from this player's camera
            - Block break/place, entity combat
            - Update camera uniform
            - Upload entity vertices (visible from this camera)
            - Set block highlight wireframe
        d. Build egui frame:
            - For each player: draw_hud(ctx, viewport, player)
            - Or: draw_crafting_ui for players with crafting open
        e. renderer.render(window, screens)
```

### Chunk Streaming

Currently chunks stream around a single player position. With split-screen, the streaming centre becomes the union of all player positions — load chunks that are within render distance of *any* player.

```rust
fn compute_stream_columns(players: &[PlayerSlot], render_distance: i32) -> AHashSet<(i32, i32)> {
    let mut columns = AHashSet::new();
    for player in players {
        let cx = (player.player.pos[0] / 16.0).floor() as i32;
        let cz = (player.player.pos[2] / 16.0).floor() as i32;
        for dx in -render_distance..=render_distance {
            for dz in -render_distance..=render_distance {
                columns.insert((cx + dx, cz + dz));
            }
        }
    }
    columns
}
```

When players are near each other, this is roughly the same as single-player. When far apart, more chunks are loaded — this is expected and correct.

---

## 6. Save/Load

### Multi-Player Save

The current save format stores one player. Extend to store N players:

```rust
pub struct SaveData {
    pub world: WorldData,
    pub players: Vec<PlayerSaveData>,  // was: single player fields
    pub world_time: u32,
    pub integrity: IntegrityLedger,
    pub difficulty: Difficulty,
    pub game_mode: GameModeType,
}

pub struct PlayerSaveData {
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub inventory: InventorySaveData,
    pub health: u32,
    pub hotbar_slot: usize,
}
```

When loading a split-screen world that was saved with 1 player, spawn Player 2 at a default offset from Player 1. When loading with fewer players than saved, keep all save data but only activate the present players.

---

## 7. Sidecar Foundation (Future)

The `ScreenContent` enum and `Screen` abstraction are the sidecar foundation. No sidecar code is built in Phase 1, but the architecture supports it without renderer changes:

### Phase 2: Multi-Window

- winit supports creating multiple windows from the same event loop
- Each window gets its own `wgpu::Surface`
- The `render()` method checks if screens span one surface or multiple
- Input routing maps each window to a player or sidecar

### Phase 2: Spectator

- `ScreenContent::Spectator(connection)` — a lightweight client that receives world state from a remote GameServer over the network transport
- Renders with the same `render_world_viewport()` — just fed by a remote camera position and remote chunk data instead of local
- Read-only: no input sent to remote server (or limited to camera control)

### Phase 3: Companion Content

- `ScreenContent::Companion(kind)` where kind is Tutorial, Map, RecipeBook, etc.
- Rendered by egui (full-viewport UI, no 3D world) or a WebView
- Tutorial content could be markdown rendered by egui, or HTML via embedded browser

### Future: Per-Screen Audio

Each Screen can own an audio output device. The engine creates one audio context per screen and routes to the assigned hardware output. This enables:
- Split-screen: shared audio (default) or headphone-per-player
- Dual-screen car mode: each kid hears their own game through their own headphones
- Player + stream sidecar: game audio in speakers, stream audio in headphones (or vice versa)

Audio routing is not implemented in Phase 1 but the per-screen model naturally supports it when audio is built (Phase 3 roadmap).

---

## 8. What Changes vs What's New

### Modified Files

| File | Changes |
|------|---------|
| `renderer.rs` | Add `player_gpu: Vec<PlayerGpuResources>`, refactor `render()` into loop over screens calling `render_world_viewport()`, per-viewport crosshair |
| `game_loop.rs` | Loop over players for tick + render, multi-intent collection, per-viewport egui HUD, union chunk streaming |
| `gamepad.rs` | Replace `active: Option<GamepadId>` with `gamepads: Vec<GamepadState>`, per-controller state tracking |
| `hud_ui.rs` | Add `viewport` parameter to all draw functions, absolute positioning instead of anchors, unique IDs per player |
| `craft_ui.rs` | Add `viewport` parameter, centre crafting UI in player's viewport |
| `player_slot.rs` | Replace `Viewport`/`viewports()` with `ViewportRect`/`Screen`/`compute_screen_layout()` |
| `input.rs` | No structural changes — already produces PlayerIntent. Minor: expose intent merge as a method |
| `save.rs` | Extend save format to `Vec<PlayerSaveData>` |
| `chunk_stream.rs` | Union of player positions for stream centre |
| `main.rs` | Pass `screens` to render, handle second-controller-join event |
| `camera.rs` | No changes — Camera is already per-instance ready |

### New Files

| File | Purpose |
|------|---------|
| `screen.rs` | `ViewportRect`, `ScreenContent`, `Screen`, `compute_screen_layout()` |

### Removed / Replaced

| Item | Replacement |
|------|-------------|
| `player_slot.rs::Viewport` (NDC-based) | `screen.rs::ViewportRect` (pixel-based) |
| `player_slot.rs::viewports()` | `screen.rs::compute_screen_layout()` |
| Single `camera_buffer` in Renderer | `Vec<PlayerGpuResources>` with per-player camera buffers |
| Single `entity_buffer` / `wire_buffer` | Per-player buffers in `PlayerGpuResources` |

---

## 9. Phase 1 Scope

What gets built now:

- `Screen`, `ViewportRect`, `ScreenContent::LocalPlayer`
- `compute_screen_layout()` for 1-2 players (side-by-side split)
- `PlayerGpuResources` and `create_player_resources()`
- `render_world_viewport()` — the per-viewport render method
- Refactored `render()` that loops over screens
- Multi-gamepad tracking (`Vec<GamepadState>`)
- Player-to-input routing (keyboard=P0, gamepads=P1+)
- Per-player tick (physics, combat, block interaction)
- Per-viewport HUD positioning
- Per-viewport crafting UI
- Union chunk streaming
- Multi-player save/load
- Second-controller-join flow (pause menu prompt)

What is explicitly NOT built now:

- Multi-window / multi-surface (Phase 2)
- Monitor detection / assignment UI (Phase 2)
- Spectator client (Phase 2)
- Companion/tutorial screen content (Phase 3)
- Per-screen audio routing (Phase 3)
- 3-4 player layouts (Phase 2)
- Peripheral vision / ultra-wide FOV (deprioritized)

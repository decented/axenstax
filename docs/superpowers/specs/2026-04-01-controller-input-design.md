# Controller & Input Abstraction — Spec

## Goal

Add gamepad (Xbox/PlayStation) support as a first-class input method. Refactor input into a `PlayerIntent` abstraction so keyboard, gamepad, and future touch controls all produce the same signals. Enable controller-only play (no keyboard/mouse required) including menu navigation.

## Core Use Cases

1. **Laptop on TV** — Bluetooth Xbox controller, no keyboard, no mouse. Must handle menus, world creation, and gameplay entirely with controller.
2. **Split-screen** (Phase 1 follow-up) — Player 1 on keyboard/mouse, Player 2 on controller. Or both on controllers.
3. **Console-native alpha testers** — coming from Xbox/PlayStation, expect standard button mapping and feel.
4. **Future: Touch** (Phase 4) — same abstraction, touch input produces PlayerIntent.

## Architecture

### PlayerIntent

The game loop reads `PlayerIntent`, never raw keys or buttons. Each input source produces one.

```
                    ┌─────────────┐
  Keyboard/Mouse ──>│             │
                    │   Input     │──> PlayerIntent
  Gamepad ─────────>│  Abstraction│   { move_dir, look_delta,
                    │   Layer     │     jump, break_block, ... }
  Touch (Phase 4) ─>│             │
                    └─────────────┘
```

```rust
pub struct PlayerIntent {
    /// Movement direction (normalized XZ). Length 0.0-1.0 (analog stick or WASD).
    pub move_dir: glam::Vec2,
    /// Look delta (pitch, yaw) in radians this frame.
    pub look_delta: glam::Vec2,
    /// Sprint modifier (analog: 0.0-1.0 from trigger, or 1.0 from Ctrl)
    pub sprint: f32,
    /// Per-frame actions (consumed once)
    pub jump: bool,
    pub sneak: bool,
    pub break_block: bool,
    pub place_block: bool,
    pub toggle_flight: bool,
    pub toggle_inventory: bool,
    pub pause: bool,
    pub hotbar_select: Option<usize>,
    pub hotbar_next: bool,
    pub hotbar_prev: bool,
    pub scroll_delta: f32,
}
```

### Input Sources

**Keyboard/Mouse** (refactor of existing `InputState`):
- WASD → move_dir (digital: 0 or 1 per axis, normalized)
- Mouse delta → look_delta
- Space → jump, Ctrl → sprint, Shift → sneak
- Left click → break_block, Right click → place_block
- Double-tap space → toggle_flight
- E → toggle_inventory, Escape → pause
- 1-9 → hotbar_select, scroll → scroll_delta

**Gamepad** (new, via `gilrs` crate):
- Left stick → move_dir (analog 0.0-1.0)
- Right stick → look_delta (with sensitivity + deadzone)
- A → jump
- Left trigger → break_block (hold to mine)
- Right trigger → place_block
- Left bumper → hotbar_prev, Right bumper → hotbar_next
- B → toggle_inventory (or back in menus)
- Y → toggle_flight (or: double-tap A)
- Left stick click → sprint
- Right stick click → sneak (toggle)
- Start → pause
- D-pad → menu navigation (not gameplay)

### Gamepad Tuning

| Parameter | Default | Notes |
|-----------|---------|-------|
| Right stick sensitivity | 2.5 | Multiplier on raw stick input |
| Deadzone (inner) | 0.15 | Below this, stick reads as 0 |
| Deadzone (outer) | 0.95 | Above this, stick reads as 1 |
| Look acceleration | None (linear) | Phase 3: add curve options |

### Menu Navigation (Controller)

egui doesn't natively support gamepad navigation. We synthesize keyboard events from gamepad:

| Gamepad | Synthesized | Menu Action |
|---------|-------------|-------------|
| D-pad Up/Down | Arrow Up/Down | Move focus |
| D-pad Left/Right | Arrow Left/Right | Move focus |
| A button | Enter | Confirm/click |
| B button | Escape | Back/cancel |
| Start | Escape | Pause/unpause |

This is injected into egui's raw input before `begin_pass()`. The existing egui UI works with keyboard navigation — we just need to feed it the right events.

### Text Input Without Keyboard

For Phase 1, avoid the on-screen keyboard complexity:
- World creation: if no keyboard detected, auto-generate a name ("New World", "New World 2", etc.)
- World edit/rename: disabled without keyboard (greyed out)
- Phase 3: proper on-screen keyboard with controller navigation

### Cursor Handling

- **Keyboard/mouse player**: existing cursor capture/release (mouse look in gameplay, free cursor in menus)
- **Controller player**: cursor is always hidden. No mouse capture needed. Look comes from right stick.
- **Mixed**: if gamepad is active and mouse hasn't moved recently, hide cursor. If mouse moves, show cursor. Auto-detect which is "active" based on last input event.

### Controller Hot-Plug

`gilrs` handles hot-plug events. When a controller connects:
- If in single-player: controller becomes an alternative input for Player 1
- If split-screen: controller becomes Player 2

When a controller disconnects mid-game:
- Pause the game automatically
- Show "Controller disconnected" message

## What's In Scope

- `PlayerIntent` abstraction struct
- Refactor `InputState` to produce `PlayerIntent`
- `GamepadState` using `gilrs` to produce `PlayerIntent`
- Gamepad → egui menu navigation (synthesized key events)
- Auto-generated world names for controller-only users
- Deadzone + sensitivity tuning (hardcoded constants, not UI)
- Controller disconnect → auto-pause

## What's NOT In Scope

- Button remapping UI (Phase 3)
- On-screen keyboard (Phase 3)
- Touch controls (Phase 4)
- Vibration/haptics (Phase 3)
- Multiple controllers for single-player (one controller per player)
- Split-screen rendering (separate spec, depends on this)

## Dependency

```toml
gilrs = "0.11"
```

## Build Status

**DONE (2026-04-01):**
- PlayerIntent abstraction struct (`player_intent.rs`)
- InputState refactored to produce PlayerIntent via `to_intent()`
- Physics reads PlayerIntent, not raw InputState
- GamepadSystem (`gamepad.rs`) via gilrs — Xbox/PS controller support
- Stick deadzone + sensitivity tuning
- Gamepad intent merged with keyboard intent in game loop
- D-pad/A/B synthesized as egui key events for menu navigation
- Controller disconnect → auto-pause
- Start button → pause/resume
- Auto-generated world names for controller-only users

**Phase 3 (future):**
- Button remapping UI
- On-screen keyboard with D-pad navigation
- Vibration/haptics
- Right stick click → sneak toggle
- Multiple controller profiles

**Phase 4 (future — touch controls):**
- Virtual joysticks (left=move, right=look) overlaid on screen
- Tap to break/place with separate toggle
- Touch → PlayerIntent (same abstraction)
- Auto-detect input method (keyboard vs controller vs touch)

## Rebuild Notes

**If rebuilding the input system from scratch:**
1. `PlayerIntent` is the core abstraction — game loop and physics never read raw input
2. Each input source produces a `PlayerIntent` independently
3. For single-player: merge all sources (keyboard OR-ed with gamepad)
4. For split-screen: each player gets their own source and intent
5. gilrs handles all controller brands (Xbox, PS, Switch Pro, generic) — don't write platform-specific code
6. Deadzone must be configurable — different controllers have different stick drift
7. egui doesn't natively support gamepad nav — synthesize keyboard events from D-pad
8. Camera look from gamepad stick needs dt-scaling (stick deflection × sensitivity × frame_time)


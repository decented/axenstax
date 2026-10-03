//! Cross-platform gamepad input.
//!
//! The public surface (`GamepadSystem`, `GamepadState`, `to_intent`) is
//! target-agnostic. Native builds wrap gilrs; WASM builds wrap a web-sys
//! `Navigator::get_gamepads()` polling loop. Both backends fill the same
//! `GamepadState` shape so every consumer (`game_loop.rs`, the menu, the
//! "press A to join" rule) is uniform.
//!
//! Button mapping matches the Xbox layout (A=jump, B=inventory, RT=break,
//! LT=place). USB and Bluetooth controllers both work.

use crate::player_intent::PlayerIntent;
use web_time::Instant;

// Tuning constants — both backends honour these.
const STICK_DEADZONE_INNER: f32 = 0.15;
const STICK_DEADZONE_OUTER: f32 = 0.95;
const LOOK_SENSITIVITY: f32 = 2.5;
/// Maximum gap between two A presses to register as a double-tap → flight
/// toggle. Tuned from the 2026-06-16 playtest ("double-tap space too
/// finicky" — widened 300 → 450ms for the keyboard flight toggle in
/// `input.rs`; the gamepad window was left at 300ms). Feel-tunable — pinned
/// by `is_double_tap`'s tests so a future edit doesn't drift it silently.
const DOUBLE_TAP_MS: u128 = 300;

/// Per-controller state. Both backends fill this; consumers never know
/// which one produced it.
pub struct GamepadState {
    /// Whether this controller is still connected.
    pub connected: bool,
    /// Whether this controller disconnected this frame.
    pub disconnected_this_frame: bool,
    /// Right stick raw values (for look).
    pub(crate) right_x: f32,
    pub(crate) right_y: f32,
    /// Left stick raw values (for movement).
    pub(crate) left_x: f32,
    pub(crate) left_y: f32,
    /// Per-frame button presses (consumed each frame).
    pub a_pressed: bool,
    pub b_pressed: bool,
    /// X / West — third-person camera perspective toggle (Phase 1).
    pub x_pressed: bool,
    pub y_pressed: bool,
    pub start_pressed: bool,
    pub(crate) lb_pressed: bool,
    pub(crate) rb_pressed: bool,
    /// Whether A (South) is currently held — separate from the per-frame
    /// edge `a_pressed`, used so `jump_held` stays true while A is held
    /// rather than firing once per frame.
    pub(crate) a_held: bool,
    /// Trigger values (0.0-1.0).
    pub(crate) left_trigger: f32,
    pub(crate) right_trigger: f32,
    /// Left stick click (sprint toggle).
    pub(crate) l3_pressed: bool,
    /// D-pad per-frame presses (for menu navigation).
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    /// Double-tap A detection for flight toggle.
    pub(crate) last_a_time: Option<Instant>,
    pub(crate) toggle_flight: bool,
    /// Sprint toggle (left stick click).
    pub(crate) sprint_on: bool,
    /// Sneak toggle (right stick click — Bedrock console standard).
    pub(crate) sneak_on: bool,
}

impl GamepadState {
    /// A fresh-but-connected state. Backends call this when they see a
    /// new controller.
    pub(crate) fn new_connected() -> Self {
        Self {
            connected: true,
            disconnected_this_frame: false,
            right_x: 0.0,
            right_y: 0.0,
            left_x: 0.0,
            left_y: 0.0,
            a_pressed: false,
            b_pressed: false,
            x_pressed: false,
            y_pressed: false,
            start_pressed: false,
            lb_pressed: false,
            rb_pressed: false,
            a_held: false,
            left_trigger: 0.0,
            right_trigger: 0.0,
            l3_pressed: false,
            dpad_up: false,
            dpad_down: false,
            dpad_left: false,
            dpad_right: false,
            last_a_time: None,
            toggle_flight: false,
            sprint_on: false,
            sneak_on: false,
        }
    }

    /// Reset all per-frame flags. Called by both backends at the top of
    /// their poll/update.
    pub(crate) fn reset_frame_flags(&mut self) {
        self.a_pressed = false;
        self.b_pressed = false;
        self.x_pressed = false;
        self.y_pressed = false;
        self.start_pressed = false;
        self.lb_pressed = false;
        self.rb_pressed = false;
        self.l3_pressed = false;
        self.dpad_up = false;
        self.dpad_down = false;
        self.dpad_left = false;
        self.dpad_right = false;
        self.toggle_flight = false;
        self.disconnected_this_frame = false;
    }

    /// Update the double-tap-A → flight-toggle state and per-frame
    /// `a_pressed` edge. Shared between backends so the timing rule lives
    /// in one place. The actual window check is the pure [`is_double_tap`]
    /// helper — this method's only job is to read the clock and drive it.
    pub(crate) fn on_a_pressed(&mut self) {
        self.a_pressed = true;
        let now = Instant::now();
        if let Some(last) = self.last_a_time
            && is_double_tap(now.duration_since(last).as_millis()) {
                self.toggle_flight = true;
            }
        self.last_a_time = Some(now);
    }
}

/// Pure double-tap decision: `true` when `elapsed_ms` (the gap since the
/// previous A press) falls inside the double-tap window. Factored out of
/// `on_a_pressed` so the tuned timing constant is testable without depending
/// on wall-clock `Instant::now()`.
fn is_double_tap(elapsed_ms: u128) -> bool {
    elapsed_ms < DOUBLE_TAP_MS
}

/// Apply deadzone to a stick axis value. Returns 0.0 inside the inner
/// deadzone, clamps to ±1.0 outside the outer deadzone, linearly scales
/// in between. Both backends use this — keeping numerical parity is
/// important so a player switching from a native build to the PWA
/// doesn't feel the sticks shift.
pub(crate) fn apply_deadzone(val: f32) -> f32 {
    let abs = val.abs();
    if abs < STICK_DEADZONE_INNER {
        0.0
    } else if abs > STICK_DEADZONE_OUTER {
        val.signum()
    } else {
        let scaled = (abs - STICK_DEADZONE_INNER) / (STICK_DEADZONE_OUTER - STICK_DEADZONE_INNER);
        scaled * val.signum()
    }
}

/// Apply a press event for a standard-mapping button index to a
/// [`GamepadState`]. Used by the web backend (poll-based) and exposed
/// here so its mapping table is testable on native.
///
/// References the W3C "standard" mapping:
/// `0=A, 1=B, 2=X, 3=Y, 4=LB, 5=RB, 6=LT, 7=RT, 8=Back, 9=Start,
///  10=L3, 11=R3, 12-15=DpadUp/Down/Left/Right`.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub(crate) fn apply_standard_button_press(state: &mut GamepadState, idx: usize) {
    match idx {
        0 => {
            state.on_a_pressed();
            state.a_held = true;
        }
        1 => state.b_pressed = true,
        2 => state.x_pressed = true,
        3 => state.y_pressed = true,
        4 => state.lb_pressed = true,
        5 => state.rb_pressed = true,
        9 => state.start_pressed = true,
        10 => {
            state.l3_pressed = true;
            state.sprint_on = !state.sprint_on;
        }
        11 => state.sneak_on = !state.sneak_on, // R3 — sneak toggle
        12 => state.dpad_up = true,
        13 => state.dpad_down = true,
        14 => state.dpad_left = true,
        15 => state.dpad_right = true,
        _ => {}
    }
}

/// Apply the held-state value for a standard-mapping button — for the
/// triggers (analog values) and the A button's `a_held` field.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub(crate) fn apply_standard_button_value(
    state: &mut GamepadState,
    idx: usize,
    pressed: bool,
    value: f32,
) {
    match idx {
        0 => state.a_held = pressed,
        6 => state.left_trigger = value,  // LT analog
        7 => state.right_trigger = value, // RT analog
        _ => {}
    }
}

/// Build a [`PlayerIntent`] from a [`GamepadState`]. Backend-agnostic —
/// operates purely on the shared state fields.
pub fn state_to_intent(gp: &GamepadState, dt: f32) -> Option<PlayerIntent> {
    if !gp.connected {
        return None;
    }

    let move_x = apply_deadzone(gp.left_x);
    let move_y = apply_deadzone(gp.left_y);
    let look_x = apply_deadzone(gp.right_x);
    let look_y = apply_deadzone(gp.right_y);

    Some(PlayerIntent {
        move_forward: move_y,
        move_right: move_x,
        look_dx: (look_x * LOOK_SENSITIVITY * dt) as f64,
        look_dy: (-look_y * LOOK_SENSITIVITY * dt) as f64, // Inverted Y
        sprint: gp.sprint_on,
        sneak: gp.sneak_on, // R3 toggle (Bedrock console standard)
        jump_held: gp.a_pressed || gp.a_held,
        jump_pressed: gp.a_pressed,
        toggle_flight: gp.toggle_flight,
        break_block: gp.right_trigger > 0.5, // RT = break/punch (Minecraft standard)
        place_block: gp.left_trigger > 0.5,  // LT = place block
        toggle_inventory: gp.b_pressed,
        drop_item: gp.dpad_down, // D-pad down = drop one (Bedrock standard); in
        // menus the d-pad drives `inject_gamepad_nav` instead, and the
        // crafting-open gate zeroes drop_item, so there's no double-meaning.
        pause: gp.start_pressed,
        hotbar_select: None, // D-pad hotbar selection is Phase 3
        scroll_delta: if gp.lb_pressed { 1.0 } else if gp.rb_pressed { -1.0 } else { 0.0 },
        toggle_debug: gp.y_pressed,
        cursor_captured: true, // Controller always acts as if cursor is captured
        rotate_ghost_ccw: false, // BRIDGE: gamepad rotate-ghost chord deferred
        rotate_ghost_cw: false,
        toggle_explorer: false, // BRIDGE: gamepad explorer chord deferred
        workshop_eyedropper: false,    // BRIDGE: no gamepad chord wired yet
        workshop_cycle_symmetry: false, // BRIDGE: no gamepad chord wired yet
        workshop_pin: false,           // BRIDGE: no gamepad chord wired yet
        workshop_gallery: false,       // BRIDGE: no gamepad chord wired yet
        workshop_toggle_mode: false,   // BRIDGE: no gamepad chord wired yet
        workshop_undo: false,          // BRIDGE: no gamepad chord wired yet
        workshop_redo: false,          // BRIDGE: ditto (Campaign S)
        workshop_picker: false,        // BRIDGE: ditto
            workshop_paint_panel: false, // BRIDGE: ditto
            workshop_limbs: false,       // BRIDGE: ditto
            workshop_tool_cycle: false,  // BRIDGE: ditto
            workshop_fill: false,        // BRIDGE: ditto
            workshop_grid: false,        // BRIDGE: ditto
            shift_down: false,           // BRIDGE: no gamepad modifier chord
        camera_cycle: gp.x_pressed,    // X / West → cycle third-person perspective
    })
}

// ── Backend modules ───────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
mod web;

/// Cross-platform gamepad system. Public methods delegate to whichever
/// backend the target compiled with.
pub struct GamepadSystem {
    #[cfg(not(target_arch = "wasm32"))]
    inner: native::NativeBackend,
    #[cfg(target_arch = "wasm32")]
    inner: web::WebBackend,
}

impl GamepadSystem {
    pub fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            inner: native::NativeBackend::new(),
            #[cfg(target_arch = "wasm32")]
            inner: web::WebBackend::new(),
        }
    }

    /// Poll the underlying backend for new events / state. Call once per
    /// frame.
    pub fn update(&mut self) {
        self.inner.update();
    }

    /// All known gamepads (connected and recently disconnected). Slots
    /// are stable so `gamepads[0]` is always the same physical controller
    /// across frames.
    pub fn gamepads(&self) -> &[GamepadState] {
        self.inner.states()
    }

    /// Returns true if at least one gamepad is connected.
    pub fn any_connected(&self) -> bool {
        self.gamepads().iter().any(|g| g.connected)
    }

    /// Returns true if any gamepad disconnected this frame.
    pub fn any_disconnected_this_frame(&self) -> bool {
        self.gamepads().iter().any(|g| g.disconnected_this_frame)
    }

    /// Number of currently connected gamepads.
    pub fn connected_count(&self) -> usize {
        self.gamepads().iter().filter(|g| g.connected).count()
    }

    /// D-pad state of the first connected gamepad (up, down, left, right).
    pub fn first_dpad(&self) -> (bool, bool, bool, bool) {
        if let Some(g) = self.gamepads().iter().find(|g| g.connected) {
            (g.dpad_up, g.dpad_down, g.dpad_left, g.dpad_right)
        } else {
            (false, false, false, false)
        }
    }

    /// Produce a [`PlayerIntent`] from the gamepad at `index`. Returns
    /// `None` if the index is out of bounds or the controller is
    /// disconnected.
    pub fn to_intent(&self, index: usize, dt: f32) -> Option<PlayerIntent> {
        state_to_intent(self.gamepads().get(index)?, dt)
    }
}

// ── Compatibility shim ────────────────────────────────────────────────────

impl GamepadSystem {
    /// Borrowed slice of gamepad state. Existing callers iterate this
    /// directly via `.gamepads.iter()` — preserve the field-style access
    /// without exposing the backend type.
    #[allow(dead_code)]
    pub(crate) fn gamepads_slice(&self) -> &[GamepadState] {
        self.gamepads()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_inside_returns_zero() {
        assert_eq!(apply_deadzone(0.0), 0.0);
        assert_eq!(apply_deadzone(0.10), 0.0);
        assert_eq!(apply_deadzone(-0.14), 0.0);
    }

    #[test]
    fn deadzone_outside_outer_saturates() {
        assert_eq!(apply_deadzone(0.99), 1.0);
        assert_eq!(apply_deadzone(-0.99), -1.0);
        assert_eq!(apply_deadzone(1.0), 1.0);
    }

    #[test]
    fn deadzone_scales_linearly_between() {
        // At the inner edge the output should be ~0; at the outer edge ~1.
        // Test the midpoint of the active range maps to ~0.5.
        let mid = (STICK_DEADZONE_INNER + STICK_DEADZONE_OUTER) / 2.0;
        let scaled = apply_deadzone(mid);
        assert!((scaled - 0.5).abs() < 0.001, "midpoint {} scaled to {}", mid, scaled);
    }

    #[test]
    fn deadzone_preserves_sign() {
        assert!(apply_deadzone(0.5) > 0.0);
        assert!(apply_deadzone(-0.5) < 0.0);
    }

    #[test]
    fn state_to_intent_returns_none_when_disconnected() {
        let mut gp = GamepadState::new_connected();
        gp.connected = false;
        assert!(state_to_intent(&gp, 1.0 / 60.0).is_none());
    }

    #[test]
    fn state_to_intent_jump_held_reflects_either_pressed_or_held() {
        // a_pressed (per-frame edge) without a_held still produces
        // jump_held = true so the player can do a tap jump that lasts a
        // frame; a_held alone (continuing to hold from a previous frame)
        // also keeps jump_held true.
        let mut gp = GamepadState::new_connected();
        gp.a_pressed = true;
        let intent = state_to_intent(&gp, 1.0 / 60.0).unwrap();
        assert!(intent.jump_held);
        assert!(intent.jump_pressed);

        gp.a_pressed = false;
        gp.a_held = true;
        let intent = state_to_intent(&gp, 1.0 / 60.0).unwrap();
        assert!(intent.jump_held);
        assert!(!intent.jump_pressed);
    }

    #[test]
    fn state_to_intent_triggers_map_to_break_place() {
        let mut gp = GamepadState::new_connected();
        gp.right_trigger = 0.6;
        gp.left_trigger = 0.0;
        let intent = state_to_intent(&gp, 1.0 / 60.0).unwrap();
        assert!(intent.break_block);
        assert!(!intent.place_block);

        gp.right_trigger = 0.0;
        gp.left_trigger = 0.6;
        let intent = state_to_intent(&gp, 1.0 / 60.0).unwrap();
        assert!(!intent.break_block);
        assert!(intent.place_block);
    }

    #[test]
    fn state_to_intent_scroll_from_bumpers() {
        let mut gp = GamepadState::new_connected();
        gp.lb_pressed = true;
        assert_eq!(state_to_intent(&gp, 1.0 / 60.0).unwrap().scroll_delta, 1.0);
        gp.lb_pressed = false;
        gp.rb_pressed = true;
        assert_eq!(state_to_intent(&gp, 1.0 / 60.0).unwrap().scroll_delta, -1.0);
    }

    #[test]
    fn reset_frame_flags_clears_per_frame_state() {
        let mut gp = GamepadState::new_connected();
        gp.a_pressed = true;
        gp.b_pressed = true;
        gp.dpad_up = true;
        gp.toggle_flight = true;
        gp.disconnected_this_frame = true;
        gp.a_held = true; // a_held is held-state, NOT per-frame — must survive

        gp.reset_frame_flags();
        assert!(!gp.a_pressed);
        assert!(!gp.b_pressed);
        assert!(!gp.dpad_up);
        assert!(!gp.toggle_flight);
        assert!(!gp.disconnected_this_frame);
        assert!(gp.a_held, "a_held is held-state, not per-frame; reset must NOT clear it");
    }

    #[test]
    fn is_double_tap_true_for_250ms_gap() {
        // Inside the 300ms window → counts as a double tap.
        assert!(is_double_tap(250));
    }

    #[test]
    fn is_double_tap_false_for_350ms_gap() {
        // Outside the 300ms window → does not count as a double tap.
        assert!(!is_double_tap(350));
    }

    #[test]
    fn double_tap_a_within_window_sets_toggle_flight() {
        let mut gp = GamepadState::new_connected();
        gp.on_a_pressed();
        // Manually back-date the first press so the second is inside the window.
        gp.last_a_time = Some(Instant::now() - std::time::Duration::from_millis(100));
        gp.on_a_pressed();
        assert!(gp.toggle_flight);
    }

    // ── Standard-mapping button table (web backend uses this) ─────────

    #[test]
    fn apply_press_a_fires_a_pressed_and_held() {
        let mut state = GamepadState::new_connected();
        apply_standard_button_press(&mut state, 0);
        assert!(state.a_pressed);
        assert!(state.a_held);
    }

    #[test]
    fn apply_press_maps_standard_indices() {
        let cases: &[(usize, fn(&GamepadState) -> bool)] = &[
            (1, |s| s.b_pressed),
            (2, |s| s.x_pressed),
            (3, |s| s.y_pressed),
            (4, |s| s.lb_pressed),
            (5, |s| s.rb_pressed),
            (9, |s| s.start_pressed),
            (10, |s| s.l3_pressed),
            (12, |s| s.dpad_up),
            (13, |s| s.dpad_down),
            (14, |s| s.dpad_left),
            (15, |s| s.dpad_right),
        ];
        for (idx, getter) in cases {
            let mut state = GamepadState::new_connected();
            apply_standard_button_press(&mut state, *idx);
            assert!(getter(&state), "button {} should set its field", idx);
        }
    }

    #[test]
    fn r3_toggles_sneak() {
        let mut state = GamepadState::new_connected();
        assert!(!state.sneak_on);
        apply_standard_button_press(&mut state, 11);
        assert!(state.sneak_on, "first R3 press toggles sneak on");
        apply_standard_button_press(&mut state, 11);
        assert!(!state.sneak_on, "second R3 press toggles sneak off");
    }

    #[test]
    fn sneak_toggle_feeds_intent() {
        let mut gp = GamepadState::new_connected();
        gp.sneak_on = true;
        let intent = state_to_intent(&gp, 1.0 / 60.0).unwrap();
        assert!(intent.sneak, "sneak_on toggle must surface as intent.sneak");
    }

    #[test]
    fn dpad_down_maps_to_drop_item() {
        let mut gp = GamepadState::new_connected();
        gp.dpad_down = true;
        let intent = state_to_intent(&gp, 1.0 / 60.0).unwrap();
        assert!(intent.drop_item, "D-pad down = drop one item (Bedrock standard)");
        gp.dpad_down = false;
        let intent = state_to_intent(&gp, 1.0 / 60.0).unwrap();
        assert!(!intent.drop_item);
    }

    #[test]
    fn reset_frame_flags_preserves_sneak_toggle() {
        let mut gp = GamepadState::new_connected();
        gp.sneak_on = true;
        gp.reset_frame_flags();
        assert!(gp.sneak_on, "sneak_on is toggle-state, not per-frame; reset must NOT clear it");
    }

    #[test]
    fn apply_press_ignores_unmapped_indices() {
        // Back (8) — currently unused. Pressing it must not flip any state.
        // (X/2 is the perspective toggle, R3/11 is the sneak toggle — see
        // their mapping tests.)
        for idx in [8, 99] {
            let mut state = GamepadState::new_connected();
            apply_standard_button_press(&mut state, idx);
            assert!(!state.a_pressed);
            assert!(!state.b_pressed);
            assert!(!state.x_pressed);
            assert!(!state.dpad_up);
        }
    }

    #[test]
    fn x_button_toggles_third_person_perspective() {
        let mut state = GamepadState::new_connected();
        apply_standard_button_press(&mut state, 2); // X / West
        assert!(state.x_pressed);
        let intent = state_to_intent(&state, 0.016).expect("connected → Some intent");
        assert!(intent.camera_cycle, "X maps to the camera perspective toggle");
    }

    #[test]
    fn apply_value_updates_triggers() {
        let mut state = GamepadState::new_connected();
        apply_standard_button_value(&mut state, 6, true, 0.42); // LT
        apply_standard_button_value(&mut state, 7, true, 0.99); // RT
        assert!((state.left_trigger - 0.42).abs() < 1e-6);
        assert!((state.right_trigger - 0.99).abs() < 1e-6);
    }

    #[test]
    fn apply_value_a_tracks_held_release() {
        let mut state = GamepadState::new_connected();
        state.a_held = true;
        apply_standard_button_value(&mut state, 0, false, 0.0);
        assert!(!state.a_held, "release should clear a_held");
    }

    #[test]
    fn l3_toggles_sprint() {
        let mut state = GamepadState::new_connected();
        assert!(!state.sprint_on);
        apply_standard_button_press(&mut state, 10);
        assert!(state.sprint_on);
        apply_standard_button_press(&mut state, 10);
        assert!(!state.sprint_on, "second L3 press should toggle sprint off");
    }

    #[test]
    fn double_tap_a_via_standard_press_path() {
        // The on_a_pressed helper is shared with native; this test
        // confirms the standard-mapping dispatch hits it correctly.
        let mut state = GamepadState::new_connected();
        apply_standard_button_press(&mut state, 0);
        state.last_a_time = Some(Instant::now() - std::time::Duration::from_millis(100));
        apply_standard_button_press(&mut state, 0);
        assert!(state.toggle_flight);
    }
}

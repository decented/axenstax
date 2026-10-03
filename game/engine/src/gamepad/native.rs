//! Native gamepad backend — wraps gilrs (USB/Bluetooth, hot-plug, Xbox
//! and PlayStation layouts).
//!
//! The previous lift of this code lived directly on `GamepadSystem`; the
//! Phase 2 refactor moves it behind an `inner` field so a parallel
//! `web::WebBackend` can share the same public surface on WASM.

use gilrs::{Axis, Button, Event, EventType, GamepadId, Gilrs};

use super::GamepadState;

/// Native (gilrs-backed) implementation of the gamepad backend.
pub struct NativeBackend {
    gilrs: Gilrs,
    /// All known gamepads (connected and recently disconnected). The
    /// indices here match the slot indices the rest of the engine uses.
    states: Vec<GamepadState>,
    /// Parallel-indexed gilrs IDs for the entries in `states`. Kept on
    /// the backend (not on `GamepadState`) so the shared state stays
    /// target-agnostic.
    ids: Vec<GamepadId>,
}

impl NativeBackend {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => g,
            Err(e) => {
                log::warn!("Failed to init gamepad system: {e}");
                // Partial failures still return a usable instance via GilrsBuilder.
                gilrs::GilrsBuilder::new()
                    .build()
                    .unwrap_or_else(|e2| panic!("Gamepad system completely failed: {e2}"))
            }
        };

        let mut states = Vec::new();
        let mut ids = Vec::new();
        for (id, gp) in gilrs.gamepads() {
            log::info!("Gamepad detected: {} ({})", gp.name(), id);
            states.push(GamepadState::new_connected());
            ids.push(id);
        }

        Self { gilrs, states, ids }
    }

    /// Find or create a slot for `id`, returning its index.
    fn find_or_create(&mut self, id: GamepadId) -> usize {
        if let Some(pos) = self.ids.iter().position(|&existing| existing == id) {
            return pos;
        }
        self.states.push(GamepadState::new_connected());
        self.ids.push(id);
        self.states.len() - 1
    }

    /// Find an existing connected slot for `id`.
    fn find_connected(&self, id: GamepadId) -> Option<usize> {
        self.ids
            .iter()
            .position(|&existing| existing == id)
            .filter(|&idx| self.states[idx].connected)
    }

    fn handle_button_press(&mut self, idx: usize, btn: Button) {
        let gp = &mut self.states[idx];
        match btn {
            Button::South => {
                // A on Xbox — jump and double-tap → flight toggle. The
                // double-tap timing rule lives in `GamepadState::on_a_pressed`
                // so it's shared with the web backend.
                gp.on_a_pressed();
                gp.a_held = true;
            }
            Button::East => gp.b_pressed = true, // B on Xbox
            Button::West => gp.x_pressed = true, // X on Xbox — perspective toggle
            Button::North => gp.y_pressed = true, // Y on Xbox
            Button::Start => gp.start_pressed = true,
            Button::LeftTrigger => gp.lb_pressed = true, // LB
            Button::RightTrigger => gp.rb_pressed = true, // RB
            Button::LeftThumb => {
                gp.l3_pressed = true;
                gp.sprint_on = !gp.sprint_on; // Toggle sprint
            }
            Button::RightThumb => gp.sneak_on = !gp.sneak_on, // R3 — toggle sneak
            Button::DPadUp => gp.dpad_up = true,
            Button::DPadDown => gp.dpad_down = true,
            Button::DPadLeft => gp.dpad_left = true,
            Button::DPadRight => gp.dpad_right = true,
            _ => {}
        }
    }

    fn handle_button_release(&mut self, idx: usize, btn: Button) {
        let gp = &mut self.states[idx];
        if let Button::South = btn {
            gp.a_held = false;
        }
    }

    fn handle_axis_change(&mut self, idx: usize, axis: Axis, value: f32) {
        let gp = &mut self.states[idx];
        match axis {
            Axis::LeftStickX => gp.left_x = value,
            Axis::LeftStickY => gp.left_y = value,
            Axis::RightStickX => gp.right_x = value,
            Axis::RightStickY => gp.right_y = value,
            _ => {}
        }
    }

    fn handle_trigger_change(&mut self, idx: usize, btn: Button, value: f32) {
        let gp = &mut self.states[idx];
        match btn {
            Button::LeftTrigger2 => gp.left_trigger = value, // LT (analog)
            Button::RightTrigger2 => gp.right_trigger = value, // RT (analog)
            _ => {}
        }
    }

    /// Poll gilrs events. Called once per frame by the public
    /// [`super::GamepadSystem::update`].
    pub fn update(&mut self) {
        for gp in &mut self.states {
            gp.reset_frame_flags();
        }

        while let Some(Event { id, event, .. }) = self.gilrs.next_event() {
            match event {
                EventType::Connected => {
                    let name = self.gilrs.gamepad(id).name().to_string();
                    let idx = self.find_or_create(id);
                    self.states[idx].connected = true;
                    self.states[idx].disconnected_this_frame = false;
                    log::info!("Gamepad connected: {name} (slot {idx})");
                }
                EventType::Disconnected => {
                    if let Some(pos) = self.ids.iter().position(|&existing| existing == id) {
                        self.states[pos].connected = false;
                        self.states[pos].disconnected_this_frame = true;
                        self.states[pos].a_held = false;
                        log::info!("Gamepad disconnected (slot {pos})");
                    }
                }
                EventType::ButtonPressed(btn, _) => {
                    if let Some(idx) = self.find_connected(id) {
                        self.handle_button_press(idx, btn);
                    }
                }
                EventType::ButtonReleased(btn, _) => {
                    if let Some(idx) = self.find_connected(id) {
                        self.handle_button_release(idx, btn);
                    }
                }
                EventType::AxisChanged(axis, val, _) => {
                    if let Some(idx) = self.find_connected(id) {
                        self.handle_axis_change(idx, axis, val);
                    }
                }
                EventType::ButtonChanged(btn, val, _) => {
                    if let Some(idx) = self.find_connected(id) {
                        self.handle_trigger_change(idx, btn, val);
                    }
                }
                _ => {}
            }
        }
    }

    /// Immutable slice of all known gamepad states.
    pub fn states(&self) -> &[GamepadState] {
        &self.states
    }
}

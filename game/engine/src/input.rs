//! Input state tracking.
//!
//! Tracks which keys are held, mouse movement deltas, mouse button clicks,
//! and hotbar selection per frame.

use winit::keyboard::KeyCode;
use winit::event::MouseButton;

/// True if a DOM overlay wants keyboard focus (the voice-feedback modal, or the
/// gamestr leaderboard opt-in shown at a scenario's end). The engine suppresses
/// ALL gameplay input — including movement — while this is set, so typing a
/// handle into the overlay's text field can't also drive the avatar. Native
/// builds always return false — no DOM, no overlay.
#[cfg(target_arch = "wasm32")]
pub fn dom_overlay_blocks_input() -> bool {
    use js_sys::Reflect;
    use wasm_bindgen::JsValue;
    let window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };
    for flag in ["__axenstax_feedback_overlay_open", "__axenstax_gamestr_overlay_open"] {
        if let Ok(val) = Reflect::get(&window, &JsValue::from_str(flag)) {
            if val.as_bool().unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn dom_overlay_blocks_input() -> bool {
    false
}

pub struct InputState {
    keys_held: ahash::AHashSet<KeyCode>,
    pub mouse_dx: f64,
    pub mouse_dy: f64,
    pub cursor_captured: bool,
    /// For double-tap space detection (creative flight toggle).
    last_space_press: Option<web_time::Instant>,
    pub toggle_flight: bool,
    /// For double-tap W detection (sprint toggle).
    last_w_press: Option<web_time::Instant>,
    /// Sprint active (toggled by double-tap W, cleared when W released).
    pub sprinting: bool,
    /// Mouse button clicks (consumed per frame — true only on press frame).
    pub left_click: bool,
    pub right_click: bool,
    /// Mouse button held state (true while button is down).
    pub left_held: bool,
    pub right_held: bool,
    /// Hotbar slot selection (0-8), None if no change this frame.
    pub hotbar_select: Option<usize>,
    /// Scroll wheel delta (positive = scroll up, negative = scroll down).
    pub scroll_delta: f32,
    /// Inventory toggle (E key), consumed per frame.
    pub inventory_toggle: bool,
    /// Drop one of the held hotbar item (Q key), consumed per frame.
    pub drop_item: bool,
    /// Spec 24 Phase 8 — Q press → rotate active plan ghost 90° CCW.
    /// Set in parallel with `drop_item`; the game_loop ghost handler
    /// consumes this AND suppresses `drop_item` when ghost is active.
    pub rotate_ghost_ccw: bool,
    /// Spec 24 Phase 8 — E press → rotate active plan ghost 90° CW.
    /// Set in parallel with `inventory_toggle`; same context-overload.
    pub rotate_ghost_cw: bool,
    /// Spec 28f — B press → toggle the Inventory Explorer overlay.
    pub explorer_toggle: bool,
    /// Workshop Phase 3: G press → eyedropper (grab colour off crosshair cell).
    pub workshop_eyedropper: bool,
    /// Workshop Phase 3: M press → cycle edit-symmetry mode.
    pub workshop_cycle_symmetry: bool,
    /// Workshop Phase 4: P press → pin the locked working copy.
    pub workshop_pin: bool,
    /// Workshop Phase 5 Task 6: K press → open/close the Wardrobe panel.
    pub workshop_gallery: bool,
    /// Workshop §6 (2026-06-18): V press → toggle Paint/Sculpt edit mode.
    pub workshop_toggle_mode: bool,
    /// Skin-paint (2026-06-30): Z press → undo the last avatar paint stroke.
    /// Edge-triggered. Only acted on while an avatar paint session is open.
    pub workshop_undo: bool,
    /// Campaign S (2026-07-05): Shift+Z press → redo an undone paint stroke.
    /// Edge-triggered; only acted on while an avatar paint session is open.
    pub workshop_redo: bool,
    /// Campaign S: O press → toggle the skin-paint colour picker window (the
    /// legacy alias for the Tab panel). NOT C — C is the global hold-to-zoom.
    /// Edge-triggered; only acted on while an avatar paint session is open.
    pub workshop_picker: bool,
    /// Tab press → toggle the skin paint panel (colour wheel + every option).
    /// Edge-triggered. Consumed ONLY while an avatar paint session is open, so
    /// Tab stays free for a server player list everywhere else.
    pub workshop_paint_panel: bool,
    /// R press → separate / rejoin the mannequin's limbs so the inner faces can
    /// be reached. Edge-triggered; avatar paint session only.
    pub workshop_limbs: bool,
    /// B press → cycle the skin-paint shading brushes (Brush → Lighten →
    /// Darken → Noise). Edge-triggered. Shares B with the Inventory Explorer
    /// (`explorer_toggle`): mutually-exclusive contexts, and game_loop
    /// suppresses the Explorer while a paint session is open.
    pub workshop_tool_cycle: bool,
    /// F press → toggle the skin-paint Fill tool on/off (back to Brush).
    /// Edge-triggered; avatar paint session only. F is otherwise unbound.
    pub workshop_fill: bool,
    /// L press → show/hide the skin painter's texel grid + hover footprint.
    /// Edge-triggered; consumed ONLY while an avatar paint session is open, so
    /// L stays free everywhere else. (G would read better but is the
    /// eyedropper; L is the nearest unbound letter.)
    pub workshop_grid: bool,
    /// Third-person camera (Phase 1): F5 press → cycle perspective. Edge-triggered.
    pub camera_cycle: bool,
    /// #6: M press → toggle the full-screen map. Edge-triggered. Overloads the
    /// Workshop "cycle symmetry" M (mutually-exclusive contexts — game_loop only
    /// acts on this outside the Workshop).
    pub map_toggle: bool,
    /// Wave 6: J press → toggle the challenge board (feature-coverage Phase 6).
    /// Edge-triggered.
    pub challenge_board_toggle: bool,
    /// #19: Y press → toggle the Rig Studio. Edge-triggered.
    pub rig_studio_toggle: bool,
    /// N (for Nakamoto) press → summon Satoshi to you (walks over) / send him
    /// home. Edge-triggered.
    pub satoshi_summon: bool,
    /// H press → toggle the in-game objective / help pop-up. Edge-triggered.
    pub help_panel: bool,
    // --- Cinematic Director (Phase 1) — edge-triggered. Read directly by
    // game_loop (native); on web nothing consumes them. Keys confirmed free of
    // any existing handler. ---
    /// F6 → toggle the Director rig on/off.
    pub director_toggle: bool,
    /// F9 → cycle Director mode (free-fly/path/tripod/follow/look-at/POV).
    pub director_mode_next: bool,
    /// F8 → toggle the Director's hide-HUD.
    pub director_hud_toggle: bool,
    /// Enter → drop a camera keyframe at the current pose.
    pub director_keyframe_drop: bool,
    /// F12 → play/stop the keyframe path.
    pub director_path_play: bool,
    /// Backspace → clear the keyframe path.
    pub director_clear_path: bool,
    /// F10 → cycle the follow/look-at/POV target.
    pub director_target_next: bool,
    /// F4 → toggle single-player `.axereplay` recording (Phase 2c). Edge-triggered.
    pub replay_record_toggle: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            keys_held: ahash::AHashSet::new(),
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            cursor_captured: false,
            last_space_press: None,
            toggle_flight: false,
            last_w_press: None,
            sprinting: false,
            left_click: false,
            right_click: false,
            left_held: false,
            right_held: false,
            hotbar_select: None,
            scroll_delta: 0.0,
            satoshi_summon: false,
            inventory_toggle: false,
            drop_item: false,
            rotate_ghost_ccw: false,
            rotate_ghost_cw: false,
            explorer_toggle: false,
            workshop_eyedropper: false,
            workshop_cycle_symmetry: false,
            workshop_pin: false,
            workshop_gallery: false,
            workshop_toggle_mode: false,
            workshop_undo: false,
            workshop_redo: false,
            workshop_picker: false,
            workshop_paint_panel: false,
            workshop_limbs: false,
            workshop_tool_cycle: false,
            workshop_fill: false,
            workshop_grid: false,
            camera_cycle: false,
            map_toggle: false,
            challenge_board_toggle: false,
            rig_studio_toggle: false,
            help_panel: false,
            director_toggle: false,
            director_mode_next: false,
            director_hud_toggle: false,
            director_keyframe_drop: false,
            director_path_play: false,
            director_clear_path: false,
            director_target_next: false,
            replay_record_toggle: false,
        }
    }

    pub fn key_pressed(&mut self, key: KeyCode) {
        self.keys_held.insert(key);

        // Double-tap space detection for creative flight (Spec 05 Section 1.7).
        // Window widened 300 → 450 ms (2026-06-16 playtest: Axolittle "to come out
        // of flying you have to double-tap space really quickly") — easier for a
        // kid to trigger the toggle without fumbling the timing.
        if key == KeyCode::Space {
            let now = web_time::Instant::now();
            if let Some(last) = self.last_space_press
                && now.duration_since(last).as_millis() < 450 {
                    self.toggle_flight = true;
                }
            self.last_space_press = Some(now);
        }

        // Double-tap W to sprint (like Minecraft)
        if key == KeyCode::KeyW {
            let now = web_time::Instant::now();
            if let Some(last) = self.last_w_press
                && now.duration_since(last).as_millis() < 300 {
                    self.sprinting = true;
                }
            self.last_w_press = Some(now);
        }

        // Inventory toggle (also Spec 24 Phase 8 ghost-rotate CW — context
        // overload resolved by `ghost_state.is_some()` in game_loop).
        if key == KeyCode::KeyE {
            self.inventory_toggle = true;
            self.rotate_ghost_cw = true;
        }

        // Spec 28f — Inventory Explorer overlay toggle. Skin painter (2026-09-06)
        // — B also walks the shading brushes. Context-overload in the same shape
        // as E (inventory / ghost-rotate): the Explorer is suppressed while an
        // avatar paint session is open, so exactly one of the two ever fires.
        if key == KeyCode::KeyB {
            self.explorer_toggle = true;
            self.workshop_tool_cycle = true;
        }

        // Skin painter (2026-09-06) — F toggles the Fill tool. Consumed only
        // while an avatar paint session is open; F is free of any other handler
        // (F5/F6/F8-F12 are function keys, not KeyF).
        if key == KeyCode::KeyF {
            self.workshop_fill = true;
        }

        // Skin painter (2026-09-06) — L shows/hides the texel grid + the hover
        // footprint box. Consumed only while an avatar paint session is open;
        // L has no other handler anywhere in the game.
        if key == KeyCode::KeyL {
            self.workshop_grid = true;
        }

        // Workshop Phase 3 — eyedropper (grab colour off crosshair cell).
        if key == KeyCode::KeyG {
            self.workshop_eyedropper = true;
        }

        // Workshop Phase 3 — cycle edit-symmetry mode. #6 — also flags the
        // full-screen map toggle; game_loop picks one by context (Workshop vs not).
        if key == KeyCode::KeyM {
            self.workshop_cycle_symmetry = true;
            self.map_toggle = true;
        }

        // Wave 6 — J toggles the challenge board (feature-coverage Phase 6).
        if key == KeyCode::KeyJ {
            self.challenge_board_toggle = true;
        }

        // Satoshi — N (for Nakamoto) summons the guide to you (he walks over);
        // press again to send him home. Pull-first co-presence.
        if key == KeyCode::KeyN {
            self.satoshi_summon = true;
        }

        // H toggles the in-game objective / help pop-up (what to do + progress).
        if key == KeyCode::KeyH {
            self.help_panel = true;
        }

        // #19 — Y toggles the Rig Studio.
        if key == KeyCode::KeyY {
            self.rig_studio_toggle = true;
        }

        // Workshop Phase 4 — pin the locked working copy (P).
        if key == KeyCode::KeyP {
            self.workshop_pin = true;
        }

        // Workshop Phase 5 Task 6 — open/close the Wardrobe panel (K).
        if key == KeyCode::KeyK {
            self.workshop_gallery = true;
        }

        // Workshop §6 (2026-06-18) — toggle Paint/Sculpt edit mode (V). Paint
        // is the safe default; you must press V to enter Sculpt before any
        // carving can happen, so a textured block can't be chipped by accident.
        if key == KeyCode::KeyV {
            self.workshop_toggle_mode = true;
        }

        // Skin-paint (2026-06-30) — Z undoes the last avatar paint stroke;
        // X redoes it (Campaign S). Z stays UNCONDITIONAL — Shift is the
        // fly-descend key, naturally held while painting mid-air, so a
        // Shift-modified chord would steal undos (review fix). X sits right
        // next to Z and is free of any existing handler.
        if key == KeyCode::KeyZ {
            self.workshop_undo = true;
        }
        if key == KeyCode::KeyX {
            self.workshop_redo = true;
        }

        // Campaign S — O toggles the skin-paint colour picker (only consumed
        // while an avatar paint session is open; free otherwise). NOT C —
        // C is the global hold-to-zoom (#44), which a kid uses for detail
        // work on the mannequin (review fix).
        if key == KeyCode::KeyO {
            self.workshop_picker = true;
        }

        // Skin paint panel (2026-07-25) — Tab opens the colour wheel + options.
        // game_loop consumes this ONLY while a skin-paint session is open, so a
        // future hold-Tab server scoreboard needs no rebinding.
        if key == KeyCode::Tab {
            self.workshop_paint_panel = true;
        }
        // R separates the mannequin's limbs (8 of 36 faces are otherwise
        // unreachable — see skin_pose). Avatar paint session only.
        if key == KeyCode::KeyR {
            self.workshop_limbs = true;
        }

        // Third-person camera (Phase 1) — F5 cycles the perspective.
        if key == KeyCode::F5 {
            self.camera_cycle = true;
        }

        // Cinematic Director (Phase 1) — edge keys (game_loop only acts on these
        // while the Director is active, so they don't disturb normal play).
        match key {
            KeyCode::F6 => self.director_toggle = true,
            KeyCode::F9 => self.director_mode_next = true,
            KeyCode::F8 => self.director_hud_toggle = true,
            KeyCode::Enter => self.director_keyframe_drop = true,
            KeyCode::F12 => self.director_path_play = true,
            KeyCode::Backspace => self.director_clear_path = true,
            KeyCode::F10 => self.director_target_next = true,
            KeyCode::F4 => self.replay_record_toggle = true,
            _ => {}
        }

        // Drop one of the held hotbar item (Minecraft Q convention; also
        // Spec 24 Phase 8 ghost-rotate CCW under the same context overload).
        if key == KeyCode::KeyQ {
            self.drop_item = true;
            self.rotate_ghost_ccw = true;
        }

        // Number keys for hotbar selection
        match key {
            KeyCode::Digit1 => self.hotbar_select = Some(0),
            KeyCode::Digit2 => self.hotbar_select = Some(1),
            KeyCode::Digit3 => self.hotbar_select = Some(2),
            KeyCode::Digit4 => self.hotbar_select = Some(3),
            KeyCode::Digit5 => self.hotbar_select = Some(4),
            KeyCode::Digit6 => self.hotbar_select = Some(5),
            KeyCode::Digit7 => self.hotbar_select = Some(6),
            KeyCode::Digit8 => self.hotbar_select = Some(7),
            KeyCode::Digit9 => self.hotbar_select = Some(8),
            _ => {}
        }
    }

    pub fn key_released(&mut self, key: KeyCode) {
        self.keys_held.remove(&key);
        // Stop sprinting when W is released
        if key == KeyCode::KeyW {
            self.sprinting = false;
        }
    }

    pub fn is_held(&self, key: KeyCode) -> bool {
        self.keys_held.contains(&key)
    }

    pub fn mouse_button_pressed(&mut self, button: MouseButton) {
        match button {
            MouseButton::Left => { self.left_click = true; self.left_held = true; }
            MouseButton::Right => { self.right_click = true; self.right_held = true; }
            _ => {}
        }
    }

    pub fn mouse_button_released(&mut self, button: MouseButton) {
        match button {
            MouseButton::Left => self.left_held = false,
            MouseButton::Right => self.right_held = false,
            _ => {}
        }
    }

    pub fn mouse_moved(&mut self, dx: f64, dy: f64) {
        self.mouse_dx += dx;
        self.mouse_dy += dy;
    }

    pub fn scroll(&mut self, delta: f32) {
        self.scroll_delta += delta;
    }

    /// Reset per-frame accumulators. Call at the end of each frame.
    pub fn end_frame(&mut self) {
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;
        self.toggle_flight = false;
        self.left_click = false;
        self.right_click = false;
        self.hotbar_select = None;
        self.scroll_delta = 0.0;
        self.inventory_toggle = false;
        self.drop_item = false;
        self.rotate_ghost_ccw = false;
        self.rotate_ghost_cw = false;
        self.explorer_toggle = false;
        self.workshop_eyedropper = false;
        self.workshop_cycle_symmetry = false;
        self.workshop_pin = false;
        self.workshop_gallery = false;
        self.workshop_toggle_mode = false;
        self.workshop_undo = false;
        self.workshop_redo = false;
        self.workshop_picker = false;
        self.workshop_paint_panel = false;
        self.workshop_limbs = false;
        self.workshop_tool_cycle = false;
        self.workshop_fill = false;
        self.workshop_grid = false;
        self.map_toggle = false;
        self.challenge_board_toggle = false;
        self.rig_studio_toggle = false;
        self.satoshi_summon = false;
        self.help_panel = false;
        self.camera_cycle = false;
        self.director_toggle = false;
        self.director_mode_next = false;
        self.director_hud_toggle = false;
        self.director_keyframe_drop = false;
        self.director_path_play = false;
        self.director_clear_path = false;
        self.director_target_next = false;
        self.replay_record_toggle = false;
    }

    /// Drop every held / continuous input. Called when the window loses focus:
    /// the OS (and the browser, on web) stops delivering key-up and pointer
    /// events while backgrounded, so without this a held movement key stays
    /// latched in `keys_held` — plus the sprint latch and mouse-button held
    /// state — and the avatar keeps walking when you tab back in. Per-frame edge
    /// flags (left_click, hotbar_select, …) are left alone: they're consumed
    /// each frame and can't be spuriously set while the window is unfocused.
    pub fn release_all_inputs(&mut self) {
        self.keys_held.clear();
        self.sprinting = false;
        self.left_held = false;
        self.right_held = false;
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;
    }

    // Convenience methods for movement directions
    pub fn forward(&self) -> bool {
        self.is_held(KeyCode::KeyW)
    }

    pub fn backward(&self) -> bool {
        self.is_held(KeyCode::KeyS)
    }

    pub fn left(&self) -> bool {
        self.is_held(KeyCode::KeyA)
    }

    pub fn right_key(&self) -> bool {
        self.is_held(KeyCode::KeyD)
    }

    pub fn jump(&self) -> bool {
        self.is_held(KeyCode::Space)
    }

    pub fn sneak(&self) -> bool {
        self.is_held(KeyCode::ShiftLeft)
    }

    /// Either Shift held — the skin painter's straight-line modifier
    /// (Shift+click rules a line from the last painted texel). Separate from
    /// [`InputState::sneak`] so the two meanings can diverge: sneak is a
    /// gameplay verb that a gamepad/touch pad can also assert, this is
    /// specifically "the keyboard modifier key is down".
    pub fn shift_held(&self) -> bool {
        self.is_held(KeyCode::ShiftLeft) || self.is_held(KeyCode::ShiftRight)
    }

    pub fn sprint(&self) -> bool {
        self.sprinting || self.is_held(KeyCode::ControlLeft)
    }

    /// Produce a PlayerIntent from the current keyboard/mouse state.
    pub fn to_intent(&self) -> crate::player_intent::PlayerIntent {
        let mut fwd: f32 = 0.0;
        let mut right: f32 = 0.0;
        if self.forward() { fwd += 1.0; }
        if self.backward() { fwd -= 1.0; }
        if self.right_key() { right += 1.0; }
        if self.left() { right -= 1.0; }

        crate::player_intent::PlayerIntent {
            move_forward: fwd,
            move_right: right,
            look_dx: self.mouse_dx,
            look_dy: self.mouse_dy,
            sprint: self.sprint(),
            sneak: self.sneak(),
            jump_held: self.jump(),
            jump_pressed: self.jump(), // BRIDGE: no separate pressed tracking yet
            toggle_flight: self.toggle_flight,
            break_block: self.left_held,
            place_block: self.right_held,
            toggle_inventory: self.inventory_toggle,
            drop_item: self.drop_item,
            pause: false, // Escape is handled in main.rs event loop, not here
            hotbar_select: self.hotbar_select,
            scroll_delta: self.scroll_delta,
            toggle_debug: false, // F3 handled in main.rs event loop
            cursor_captured: self.cursor_captured,
            rotate_ghost_ccw: self.rotate_ghost_ccw,
            rotate_ghost_cw: self.rotate_ghost_cw,
            toggle_explorer: self.explorer_toggle,
            workshop_eyedropper: self.workshop_eyedropper,
            workshop_cycle_symmetry: self.workshop_cycle_symmetry,
            workshop_pin: self.workshop_pin,
            workshop_gallery: self.workshop_gallery,
            workshop_toggle_mode: self.workshop_toggle_mode,
            workshop_undo: self.workshop_undo,
            workshop_redo: self.workshop_redo,
            workshop_picker: self.workshop_picker,
            workshop_paint_panel: self.workshop_paint_panel,
            workshop_limbs: self.workshop_limbs,
            workshop_tool_cycle: self.workshop_tool_cycle,
            workshop_fill: self.workshop_fill,
            workshop_grid: self.workshop_grid,
            shift_down: self.shift_held(),
            camera_cycle: self.camera_cycle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f5_sets_camera_cycle_edge_and_clears_each_frame() {
        let mut input = InputState::new();
        assert!(!input.to_intent().camera_cycle, "idle → no perspective cycle");
        input.key_pressed(KeyCode::F5);
        assert!(input.to_intent().camera_cycle, "F5 cycles the camera perspective");
        input.end_frame();
        assert!(!input.to_intent().camera_cycle, "edge cleared at end of frame");
    }

    #[test]
    fn tab_sets_the_paint_panel_edge_and_clears_each_frame() {
        let mut input = InputState::new();
        assert!(!input.to_intent().workshop_paint_panel, "idle → no panel toggle");
        input.key_pressed(KeyCode::Tab);
        assert!(input.to_intent().workshop_paint_panel, "Tab → panel toggle");
        input.end_frame();
        assert!(!input.to_intent().workshop_paint_panel, "edge-triggered, one frame only");
    }

    #[test]
    fn r_sets_the_limbs_toggle() {
        let mut input = InputState::new();
        assert!(!input.to_intent().workshop_limbs);
        input.key_pressed(KeyCode::KeyR);
        assert!(input.to_intent().workshop_limbs);
        input.end_frame();
        assert!(!input.to_intent().workshop_limbs);
    }

    #[test]
    fn b_sets_the_paint_tool_cycle_edge_alongside_the_explorer() {
        // Both flags ride the same key; game_loop picks by context (a paint
        // session open → tool cycle, otherwise → Inventory Explorer).
        let mut input = InputState::new();
        assert!(!input.to_intent().workshop_tool_cycle);
        input.key_pressed(KeyCode::KeyB);
        let i = input.to_intent();
        assert!(i.workshop_tool_cycle, "B walks the shading brushes");
        assert!(i.toggle_explorer, "B still opens the Explorer outside the painter");
        input.end_frame();
        assert!(!input.to_intent().workshop_tool_cycle, "edge-triggered, one frame only");
    }

    #[test]
    fn f_sets_the_fill_edge_and_touches_nothing_else() {
        let mut input = InputState::new();
        assert!(!input.to_intent().workshop_fill);
        input.key_pressed(KeyCode::KeyF);
        let i = input.to_intent();
        assert!(i.workshop_fill, "F toggles the Fill tool");
        assert!(!i.toggle_explorer);
        assert!(!i.toggle_inventory);
        assert!(!i.workshop_tool_cycle);
        assert!(!i.workshop_paint_panel);
        input.end_frame();
        assert!(!input.to_intent().workshop_fill, "edge-triggered, one frame only");
    }

    #[test]
    fn l_sets_the_grid_edge_and_touches_nothing_else() {
        // L is the painter's grid/footprint toggle and has no other handler in
        // the game, so pressing it must set exactly one flag.
        let mut input = InputState::new();
        assert!(!input.to_intent().workshop_grid);
        input.key_pressed(KeyCode::KeyL);
        let i = input.to_intent();
        assert!(i.workshop_grid, "L shows/hides the texel grid");
        assert!(!i.workshop_fill);
        assert!(!i.workshop_tool_cycle);
        assert!(!i.toggle_explorer);
        assert!(!i.toggle_inventory);
        assert!(!i.workshop_paint_panel);
        assert!(!i.workshop_limbs);
        input.end_frame();
        assert!(!input.to_intent().workshop_grid, "edge-triggered, one frame only");
    }

    #[test]
    fn shift_is_reported_for_the_straight_line_modifier() {
        let mut input = InputState::new();
        assert!(!input.to_intent().shift_down);
        input.key_pressed(KeyCode::ShiftRight);
        assert!(input.to_intent().shift_down, "either Shift arms the line modifier");
        input.key_released(KeyCode::ShiftRight);
        input.key_pressed(KeyCode::ShiftLeft);
        assert!(input.to_intent().shift_down);
        input.key_released(KeyCode::ShiftLeft);
        assert!(!input.to_intent().shift_down);
    }

    #[test]
    fn tab_touches_nothing_else() {
        // Tab is the conventional server player-list key. It must carry exactly
        // one meaning here and never be swallowed on behalf of another feature,
        // so a hold-Tab scoreboard can land later without a rebinding exercise.
        let mut input = InputState::new();
        input.key_pressed(KeyCode::Tab);
        let i = input.to_intent();
        assert!(i.workshop_paint_panel);
        assert!(!i.workshop_picker);
        assert!(!i.workshop_pin);
        assert!(!i.workshop_gallery);
        assert!(!i.workshop_toggle_mode);
        assert!(!i.workshop_undo);
        assert!(!i.workshop_redo);
        assert!(!i.workshop_limbs);
        assert!(!i.workshop_tool_cycle);
        assert!(!i.workshop_fill);
        assert!(!i.workshop_grid);
        assert!(!i.camera_cycle);
        // Keys that live on the input state rather than the intent.
        assert!(!input.map_toggle);
        assert!(!input.inventory_toggle);
        assert!(!input.explorer_toggle);
        assert!(!input.rig_studio_toggle);
        assert!(!input.help_panel);
        assert!(!input.drop_item);
    }

    #[test]
    fn release_all_inputs_drops_held_and_continuous_state() {
        // Player mid-action: holding W, sprint latched, both mouse buttons down
        // (mining + placing), with look-delta accumulated this frame. This is the
        // state when the window loses focus (alt-tab); the OS/browser stops
        // delivering key-up + pointer events, so without an explicit reset the
        // movement key stays latched and the avatar keeps walking on return.
        let mut input = InputState::new();
        input.key_pressed(KeyCode::KeyW);
        input.sprinting = true;
        input.mouse_button_pressed(MouseButton::Left);
        input.mouse_button_pressed(MouseButton::Right);
        input.mouse_moved(12.0, -8.0);
        assert!(input.forward(), "precondition: W held");
        assert!(input.left_held && input.right_held, "precondition: buttons held");

        input.release_all_inputs();

        assert!(!input.forward(), "held movement key dropped on focus loss");
        assert!(!input.is_held(KeyCode::KeyW), "keys_held cleared");
        assert!(!input.sprinting, "sprint latch cleared");
        assert!(!input.left_held, "left button held state cleared");
        assert!(!input.right_held, "right button held state cleared");
        assert_eq!(input.mouse_dx, 0.0, "accumulated look-delta discarded");
        assert_eq!(input.mouse_dy, 0.0, "accumulated look-delta discarded");
    }
}

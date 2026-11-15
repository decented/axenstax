//! Input state tracking.
//!
//! Tracks which keys are held, mouse movement deltas, mouse button clicks,
//! and hotbar selection per frame.

use crate::keybindings::{Action, ActiveContexts, Binding, Bindings};
use std::collections::BTreeMap;
use std::sync::Arc;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

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
    for flag in ["__axenstax_gamestr_overlay_open"] {
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
    /// Every key and mouse button down now. Held actions ask whether their
    /// binding is in here, so a held action can be a key or a mouse button.
    held: ahash::AHashSet<Binding>,
    /// Which non-gameplay contexts are live (set by the game loop each frame): a
    /// press one of them owns does not also fire the Gameplay action on that key.
    active: ActiveContexts,
    /// The live bindings (Settings → Controls). Every press, release and held
    /// read goes through them; the defaults are the shipped controls.
    bindings: Arc<Bindings>,
    /// The stored overrides `bindings` was built from (see [`InputState::apply_overrides`]).
    overrides_applied: Option<BTreeMap<String, String>>,
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
    /// Attack / Use presses (consumed per frame — true only on the press
    /// frame). Named for their default mouse buttons; they follow the Attack
    /// and Use bindings, a key or any mouse button.
    pub left_click: bool,
    pub right_click: bool,
    /// Attack / Use held (true while the binding is down).
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
    /// F press → swap the main-hand item with the off-hand (`hands.rs`).
    /// Edge-triggered, one frame. F is shared with the skin painter's Fill
    /// tool (`workshop_fill`): the paint session consumes that one, and
    /// `game_loop` ignores this one while a paint session is open.
    pub swap_hands: bool,
    /// L press → show/hide the skin painter's texel grid + hover footprint.
    /// Edge-triggered; consumed ONLY while an avatar paint session is open, so
    /// L stays free everywhere else. (G would read better but is the
    /// eyedropper; L is the nearest unbound letter.)
    pub workshop_grid: bool,
    /// Third-person camera (Phase 1): F5 press → cycle perspective. Edge-triggered.
    pub camera_cycle: bool,
    /// Pick block: middle-click → put the aimed block's item in hand.
    /// Edge-triggered (once/press); client-local, never over the wire.
    pub pick_block: bool,
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
            held: ahash::AHashSet::new(),
            active: ActiveContexts::default(),
            bindings: Arc::new(Bindings::defaults()),
            overrides_applied: None,
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
            swap_hands: false,
            workshop_grid: false,
            camera_cycle: false,
            pick_block: false,
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

    /// Tell the input which non-gameplay contexts are live now (once a frame).
    pub fn set_active_contexts(&mut self, active: ActiveContexts) {
        self.active = active;
    }

    /// The live bindings.
    pub fn bindings(&self) -> &Arc<Bindings> {
        &self.bindings
    }

    /// Use these bindings from now on (tests; the game goes through
    /// [`InputState::apply_overrides`]).
    #[cfg(test)]
    pub fn set_bindings(&mut self, bindings: Arc<Bindings>) {
        self.bindings = bindings;
    }

    /// Rebuild the bindings from the stored overrides
    /// (`GraphicsSettings::key_bindings`) when they differ from the ones in
    /// use. Returns what the stored map had that was ignored (for one log
    /// line) when it rebuilt, `None` when nothing changed.
    pub fn apply_overrides(&mut self, overrides: &BTreeMap<String, String>) -> Option<Vec<String>> {
        if self.overrides_applied.as_ref() == Some(overrides) {
            return None;
        }
        let (bindings, ignored) = Bindings::from_overrides(overrides);
        self.bindings = Arc::new(bindings);
        self.overrides_applied = Some(overrides.clone());
        Some(ignored)
    }

    /// A physical key went down.
    pub fn key_pressed(&mut self, key: KeyCode) {
        self.press(Binding::Key(key));
    }

    /// A physical key came up.
    pub fn key_released(&mut self, key: KeyCode) {
        self.release(Binding::Key(key));
    }

    /// Is this physical key down? (Bindings ask [`InputState::action_held`].)
    pub fn is_held(&self, key: KeyCode) -> bool {
        self.held.contains(&Binding::Key(key))
    }

    /// Is the action's binding down now?
    pub fn action_held(&self, action: Action) -> bool {
        self.held.contains(&self.bindings.binding_for(action))
    }

    pub fn mouse_button_pressed(&mut self, button: MouseButton) {
        if let Some(b) = Binding::from_mouse(button) {
            self.press(b);
        }
    }

    pub fn mouse_button_released(&mut self, button: MouseButton) {
        if let Some(b) = Binding::from_mouse(button) {
            self.release(b);
        }
    }

    /// A key or mouse button went down: every context's action bound to it
    /// sets its flag; `game_loop` picks the one its context wants (E opens the
    /// inventory, or turns a plan ghost while one is up). A live non-gameplay
    /// context owns its own binding: the Gameplay action on the same key stays
    /// quiet (`Bindings::actions_firing`), except for the shipped overlaps.
    fn press(&mut self, binding: Binding) {
        self.held.insert(binding);
        let bindings = Arc::clone(&self.bindings);
        for action in bindings.actions_firing(binding, self.active) {
            self.press_action(action);
        }
    }

    fn release(&mut self, binding: Binding) {
        self.held.remove(&binding);
        let bindings = Arc::clone(&self.bindings);
        for action in bindings.actions_for(binding) {
            match action {
                // Sprinting stops when Forward is let go.
                Action::Forward => self.sprinting = false,
                Action::Attack => self.left_held = false,
                Action::Use => self.right_held = false,
                _ => {}
            }
        }
    }

    fn press_action(&mut self, action: Action) {
        if let Some(slot) = action.hotbar_slot() {
            self.hotbar_select = Some(slot);
            return;
        }
        match action {
            // Double-tap Jump toggles creative flight (Spec 05 Section 1.7).
            // Window widened 300 → 450 ms (2026-06-16 playtest: Axolittle "to come out
            // of flying you have to double-tap space really quickly") — easier for a
            // kid to trigger the toggle without fumbling the timing.
            Action::Jump => {
                let now = web_time::Instant::now();
                if let Some(last) = self.last_space_press
                    && now.duration_since(last).as_millis() < 450 {
                        self.toggle_flight = true;
                    }
                self.last_space_press = Some(now);
            }
            // Double-tap Forward to sprint (like Minecraft).
            Action::Forward => {
                let now = web_time::Instant::now();
                if let Some(last) = self.last_w_press
                    && now.duration_since(last).as_millis() < 300 {
                        self.sprinting = true;
                    }
                self.last_w_press = Some(now);
            }
            Action::Attack => {
                self.left_click = true;
                self.left_held = true;
            }
            Action::Use => {
                self.right_click = true;
                self.right_held = true;
            }
            Action::PickBlock => self.pick_block = true,
            // Inventory (E): shares its default key with the plan ghost's
            // clockwise turn (Spec 24 Phase 8); `ghost_state.is_some()` in
            // game_loop picks one.
            Action::Inventory => self.inventory_toggle = true,
            Action::PlanRotateCw => self.rotate_ghost_cw = true,
            // Drop one of the held hotbar item (Minecraft Q convention); Q also
            // turns a plan ghost anticlockwise under the same context overload.
            Action::Drop => self.drop_item = true,
            Action::PlanRotateCcw => self.rotate_ghost_ccw = true,
            // Spec 28f — Inventory Explorer. Shares B with the skin painter's
            // brush cycle; the Explorer is suppressed while an avatar paint
            // session is open, so exactly one of the two ever fires.
            Action::Explorer => self.explorer_toggle = true,
            Action::PaintTool => self.workshop_tool_cycle = true,
            // F: swap hands, or the skin painter's Fill (the paint session
            // consumes Fill; game_loop ignores the swap while one is open).
            Action::SwapHands => self.swap_hands = true,
            Action::PaintFill => self.workshop_fill = true,
            // L: the painter's texel grid + hover footprint.
            Action::PaintGrid => self.workshop_grid = true,
            // Workshop Phase 3 — eyedropper (grab colour off crosshair cell).
            Action::WorkshopPickColour => self.workshop_eyedropper = true,
            // Workshop Phase 3 — cycle edit-symmetry mode. #6 — M is also the
            // full-screen map; game_loop picks one by context (Workshop or not).
            Action::WorkshopMirror => self.workshop_cycle_symmetry = true,
            Action::Map => self.map_toggle = true,
            // Wave 6 — the challenge board (feature-coverage Phase 6).
            Action::Challenges => self.challenge_board_toggle = true,
            // Satoshi — N (for Nakamoto) summons the guide to you (he walks
            // over); press again to send him home. Pull-first co-presence.
            Action::Satoshi => self.satoshi_summon = true,
            // The in-game objective / help pop-up (what to do + progress).
            Action::Help => self.help_panel = true,
            // #19 — the Rig Studio.
            Action::RigStudio => self.rig_studio_toggle = true,
            // Workshop Phase 4 — pin the locked working copy.
            Action::WorkshopPin => self.workshop_pin = true,
            // Workshop Phase 5 Task 6 — open/close the Wardrobe panel.
            Action::WorkshopWardrobe => self.workshop_gallery = true,
            // Workshop §6 (2026-06-18) — toggle Paint/Sculpt edit mode. Paint is
            // the safe default; Sculpt needs this press first, so a textured
            // block can't be chipped by accident.
            Action::WorkshopMode => self.workshop_toggle_mode = true,
            // Skin-paint — undo / redo the last avatar paint stroke. Undo has
            // no Shift chord: Shift is the fly-descend key, naturally held
            // while painting mid-air (review fix).
            Action::PaintUndo => self.workshop_undo = true,
            Action::PaintRedo => self.workshop_redo = true,
            // Campaign S — the skin-paint colour picker (the legacy alias for
            // the paint panel). Not C — C is the global hold-to-zoom.
            Action::PaintPicker => self.workshop_picker = true,
            // The skin paint panel (colour wheel + every option). Consumed
            // ONLY while a skin-paint session is open.
            Action::PaintPanel => self.workshop_paint_panel = true,
            // Separate the mannequin's limbs (8 of 36 faces are otherwise
            // unreachable — see skin_pose). Avatar paint session only.
            Action::PaintLimbs => self.workshop_limbs = true,
            // Third-person camera (Phase 1) — F5 cycles the perspective.
            Action::CameraView => self.camera_cycle = true,
            // Cinematic Director (Phase 1) — game_loop only acts on these
            // while the Director is active, so they don't disturb normal play.
            Action::DirectorToggle => self.director_toggle = true,
            Action::DirectorMode => self.director_mode_next = true,
            Action::DirectorHud => self.director_hud_toggle = true,
            Action::DirectorKeyframe => self.director_keyframe_drop = true,
            Action::DirectorPlay => self.director_path_play = true,
            Action::DirectorClearPath => self.director_clear_path = true,
            Action::DirectorTarget => self.director_target_next = true,
            Action::ReplayRecord => self.replay_record_toggle = true,
            // Held actions are read through `action_held`; chat, Esc, F3, F7
            // and F11 are handled by `lib.rs` before the press reaches here.
            Action::Back
            | Action::Left
            | Action::Right
            | Action::Sneak
            | Action::Sprint
            | Action::Zoom
            | Action::FreeLook
            | Action::PaintLine
            | Action::Chat
            | Action::Command
            | Action::Pause
            | Action::DebugOverlay
            | Action::SpawnOverlay
            | Action::Fullscreen => {}
            Action::Hotbar1
            | Action::Hotbar2
            | Action::Hotbar3
            | Action::Hotbar4
            | Action::Hotbar5
            | Action::Hotbar6
            | Action::Hotbar7
            | Action::Hotbar8
            | Action::Hotbar9 => {}
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
        self.swap_hands = false;
        self.workshop_grid = false;
        self.map_toggle = false;
        self.challenge_board_toggle = false;
        self.rig_studio_toggle = false;
        self.satoshi_summon = false;
        self.help_panel = false;
        self.camera_cycle = false;
        self.pick_block = false;
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
    /// latched in `held` — plus the sprint latch and mouse-button held
    /// state — and the avatar keeps walking when you tab back in. Per-frame edge
    /// flags (left_click, hotbar_select, …) are left alone: they're consumed
    /// each frame and can't be spuriously set while the window is unfocused.
    pub fn release_all_inputs(&mut self) {
        self.held.clear();
        self.sprinting = false;
        self.left_held = false;
        self.right_held = false;
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;
    }

    // Convenience methods for movement directions (the bound keys, held).
    pub fn forward(&self) -> bool {
        self.action_held(Action::Forward)
    }

    pub fn backward(&self) -> bool {
        self.action_held(Action::Back)
    }

    pub fn left(&self) -> bool {
        self.action_held(Action::Left)
    }

    pub fn right_key(&self) -> bool {
        self.action_held(Action::Right)
    }

    pub fn jump(&self) -> bool {
        self.action_held(Action::Jump)
    }

    pub fn sneak(&self) -> bool {
        self.action_held(Action::Sneak)
    }

    /// The skin painter's straight-line modifier (Shift+click rules a line
    /// from the last painted texel): the Workshop's "Straight line" binding
    /// held. While that is Left Shift (the default), Right Shift works too, as
    /// it always has. Separate from [`InputState::sneak`] so the two meanings
    /// can diverge: sneak is a gameplay verb that a gamepad/touch pad can also
    /// assert, this is specifically the painter's keyboard modifier.
    pub fn shift_held(&self) -> bool {
        self.action_held(Action::PaintLine)
            || (self.bindings.binding_for(Action::PaintLine) == Binding::Key(KeyCode::ShiftLeft)
                && self.is_held(KeyCode::ShiftRight))
    }

    pub fn sprint(&self) -> bool {
        self.sprinting || self.action_held(Action::Sprint)
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
            // Mouse: `place_block` is already a hold; `use_held` is touch's. Not guarding yet
            // (the game loop sets it each tick from the raise).
            use_held: false,
            guarding: false,
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
            swap_hands: self.swap_hands,
            workshop_grid: self.workshop_grid,
            shift_down: self.shift_held(),
            camera_cycle: self.camera_cycle,
            pick_block: self.pick_block,
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
    fn middle_click_sets_the_pick_block_edge_and_clears_each_frame() {
        let mut input = InputState::new();
        assert!(!input.to_intent().pick_block, "idle → no pick");
        input.mouse_button_pressed(MouseButton::Middle);
        let i = input.to_intent();
        assert!(i.pick_block, "middle click picks the aimed block");
        assert!(!i.break_block && !i.place_block, "and neither breaks nor places");
        assert!(!input.left_click && !input.right_click);
        input.mouse_button_released(MouseButton::Middle);
        assert!(input.to_intent().pick_block, "release does not clear the edge");
        input.end_frame();
        assert!(!input.to_intent().pick_block, "edge-triggered, one frame only");
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

    // ── Remapping (Settings → Controls, Spec 05 §1.9) ──────────────────────

    use crate::keybindings::{Action, Binding, Bindings, Rebind};

    /// An InputState whose bindings have `action` moved to `to`.
    fn remapped(action: Action, to: Binding) -> InputState {
        let mut b = Bindings::defaults();
        assert!(matches!(b.rebind(action, to), Rebind::Bound | Rebind::Swapped { .. }));
        let mut input = InputState::new();
        input.set_bindings(std::sync::Arc::new(b));
        input
    }

    use crate::keybindings::ActiveContexts;

    #[test]
    fn a_live_painter_owns_its_rebound_key_and_the_dye_is_not_dropped() {
        // The painter's Grid on Q, painting with a dye held: Q toggles the grid only.
        let mut input = remapped(Action::PaintGrid, Binding::Key(KeyCode::KeyQ));
        input.set_active_contexts(ActiveContexts { workshop: true, avatar_paint: true, ..Default::default() });
        input.key_pressed(KeyCode::KeyQ);
        assert!(input.workshop_grid, "Q is the grid now");
        assert!(!input.drop_item, "and does not also drop the dye");
        // Outside a paint session the painter owns nothing: Q drops as Q always did.
        let mut input = remapped(Action::PaintGrid, Binding::Key(KeyCode::KeyQ));
        input.set_active_contexts(ActiveContexts::default());
        input.key_pressed(KeyCode::KeyQ);
        assert!(input.drop_item, "no paint session: Q still drops");
    }

    #[test]
    fn a_gameplay_action_rebound_onto_a_live_contexts_key_stays_quiet_there() {
        // Drop on G (the Workshop's pick-colour key): in the Workshop G picks a colour only.
        let mut input = remapped(Action::Drop, Binding::Key(KeyCode::KeyG));
        input.set_active_contexts(ActiveContexts { workshop: true, ..Default::default() });
        input.key_pressed(KeyCode::KeyG);
        assert!(input.workshop_eyedropper && !input.drop_item);
        input.end_frame();
        input.set_active_contexts(ActiveContexts::default());
        input.key_pressed(KeyCode::KeyG);
        assert!(input.drop_item, "outside the Workshop G drops");
    }

    #[test]
    fn the_shipped_overlaps_still_fire_together_with_every_context_live() {
        // 0.2.40 set both flags of E, Q, B, F and M and `game_loop` told them apart.
        let mut input = InputState::new();
        input.set_active_contexts(ActiveContexts::ALL_LIVE);
        input.key_pressed(KeyCode::KeyE);
        assert!(input.inventory_toggle && input.rotate_ghost_cw);
        input.key_pressed(KeyCode::KeyQ);
        assert!(input.drop_item && input.rotate_ghost_ccw);
        input.key_pressed(KeyCode::KeyB);
        assert!(input.explorer_toggle && input.workshop_tool_cycle);
        input.key_pressed(KeyCode::KeyF);
        assert!(input.swap_hands && input.workshop_fill);
        input.key_pressed(KeyCode::KeyM);
        assert!(input.map_toggle && input.workshop_cycle_symmetry);
    }

    #[test]
    fn a_remapped_hotbar_key_selects_its_slot_and_the_old_key_does_not() {
        let mut input = remapped(Action::Hotbar1, Binding::Key(KeyCode::KeyZ));
        input.key_pressed(KeyCode::KeyZ);
        assert_eq!(input.hotbar_select, Some(0), "Z now selects the first slot");
        input.end_frame();
        input.key_pressed(KeyCode::Digit1);
        assert_eq!(input.hotbar_select, None, "1 no longer does");
        input.key_pressed(KeyCode::Digit2);
        assert_eq!(input.hotbar_select, Some(1), "the other slots keep their keys");
        // A hotbar slot can be a mouse button too.
        let mut input = remapped(Action::Hotbar1, Binding::Mouse(MouseButton::Back));
        input.mouse_button_pressed(MouseButton::Back);
        assert_eq!(input.hotbar_select, Some(0), "Mouse 4 selects the first slot");
    }

    #[test]
    fn remapped_forward_moves_the_player() {
        let mut input = remapped(Action::Forward, Binding::Key(KeyCode::ArrowUp));
        input.key_pressed(KeyCode::KeyW);
        assert!(!input.forward(), "W no longer walks");
        assert_eq!(input.to_intent().move_forward, 0.0);
        input.key_released(KeyCode::KeyW);
        input.key_pressed(KeyCode::ArrowUp);
        assert!(input.forward(), "Up walks");
        let intent = input.to_intent();
        assert_eq!(intent.move_forward, 1.0);
        // The body really walks: a stone floor, a second of the held key.
        let mut world = crate::world::World::new();
        let registry = crate::block::BlockRegistry::new();
        for x in -8..=8 {
            for z in -8..=8 {
                world.set_block(x, 63, z, crate::block::STONE);
            }
        }
        let cam = crate::camera::Camera::new(glam::Vec3::new(0.0, 64.0, 0.0), 1.0);
        let mut body = crate::physics::Player::new(glam::Vec3::new(0.0, 64.0, 0.0));
        for _ in 0..20 {
            body.tick(&intent, &cam, &world, &registry, crate::play_mode::PlayMode::Survival);
        }
        assert!(body.pos.z < -1.0, "held Up walks forward (-Z at yaw 0): {:?}", body.pos);
        // Double-tap sprint follows the new key, and letting it go stops it.
        let mut input = remapped(Action::Forward, Binding::Key(KeyCode::ArrowUp));
        input.key_pressed(KeyCode::ArrowUp);
        input.key_released(KeyCode::ArrowUp);
        input.key_pressed(KeyCode::ArrowUp);
        assert!(input.sprinting, "double-tap Up sprints");
        input.key_released(KeyCode::ArrowUp);
        assert!(!input.sprinting);
    }

    #[test]
    fn a_mouse_button_can_attack_a_key_can_use_and_any_action_takes_a_button() {
        let mut input = remapped(Action::Attack, Binding::Mouse(MouseButton::Back));
        input.mouse_button_pressed(MouseButton::Left);
        assert!(!input.to_intent().break_block, "left click no longer attacks");
        input.mouse_button_released(MouseButton::Left);
        input.mouse_button_pressed(MouseButton::Back);
        assert!(input.left_click && input.to_intent().break_block, "Mouse 4 attacks");
        input.mouse_button_released(MouseButton::Back);
        assert!(!input.to_intent().break_block, "letting go stops the hold");

        let mut input = remapped(Action::Use, Binding::Key(KeyCode::KeyU));
        input.key_pressed(KeyCode::KeyU);
        assert!(input.right_click && input.to_intent().place_block, "U uses");
        input.key_released(KeyCode::KeyU);
        assert!(!input.right_held);

        let mut input = remapped(Action::Inventory, Binding::Mouse(MouseButton::Forward));
        input.mouse_button_pressed(MouseButton::Forward);
        assert!(input.inventory_toggle, "Mouse 5 opens the inventory");
    }

    #[test]
    fn held_reads_and_focus_loss_follow_the_bindings() {
        let mut input = remapped(Action::Sneak, Binding::Mouse(MouseButton::Back));
        input.mouse_button_pressed(MouseButton::Back);
        assert!(input.sneak() && input.action_held(Action::Sneak));
        input.release_all_inputs();
        assert!(!input.sneak(), "focus loss drops a held mouse binding too");

        let mut input = remapped(Action::Zoom, Binding::Key(KeyCode::KeyV));
        input.key_pressed(KeyCode::KeyC);
        assert!(!input.action_held(Action::Zoom), "C no longer zooms");
        input.key_pressed(KeyCode::KeyV);
        assert!(input.action_held(Action::Zoom), "V zooms");
    }

    #[test]
    fn a_remap_in_one_context_leaves_the_other_contexts_alone() {
        // Drop moves to G: Q still turns a plan ghost anticlockwise, and G still
        // picks a Workshop colour as well as dropping.
        let mut input = remapped(Action::Drop, Binding::Key(KeyCode::KeyG));
        input.key_pressed(KeyCode::KeyQ);
        assert!(!input.drop_item && input.rotate_ghost_ccw);
        input.end_frame();
        input.key_pressed(KeyCode::KeyG);
        assert!(input.drop_item && input.workshop_eyedropper && !input.rotate_ghost_ccw);
    }

    #[test]
    fn stored_overrides_rebuild_the_bindings_once() {
        let mut input = InputState::new();
        let mut map = std::collections::BTreeMap::new();
        map.insert("drop".to_string(), "KeyG".to_string());
        map.insert("no_such_control".to_string(), "KeyP".to_string());
        let ignored = input.apply_overrides(&map).expect("a new map rebuilds");
        assert_eq!(ignored.len(), 1, "the unknown id is reported: {ignored:?}");
        assert!(input.apply_overrides(&map).is_none(), "the same map again does nothing");
        input.key_pressed(KeyCode::KeyG);
        assert!(input.drop_item);
    }
}

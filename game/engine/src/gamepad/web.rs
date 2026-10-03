//! Web gamepad backend — `web_sys::Navigator::get_gamepads()` polling.
//!
//! Lifts the engine's solo-only clamp on WASM. The Browser Gamepad API
//! is poll-based and returns up to 4 controller slots per call; each
//! slot is null or a [`web_sys::Gamepad`]. We map browser slot index
//! directly to engine slot index, so `states[i]` is whatever the
//! browser exposes at `navigator.getGamepads()[i]`.
//!
//! Standard mapping (W3C spec, `pad.mapping() == "standard"`) is the
//! only layout we accept — third-party controllers with non-standard
//! mappings are logged and ignored. Phase 7 playtest will surface
//! whether any of the dev's hardware needs a remapping fallback.
//!
//! The browser only exposes a controller *after* the user presses a
//! button on it (anti-fingerprinting). That dovetails with the engine's
//! "press A to join" rule: the first poll that sees a new controller
//! fires on the same frame the user pressed A.

use wasm_bindgen::JsCast;
use web_sys::{Gamepad, GamepadButton, GamepadMappingType};

use super::{apply_standard_button_press, apply_standard_button_value, GamepadState};

/// Browser slot count — `navigator.getGamepads()` returns up to this
/// many slots (per the W3C spec; Chromium honours it).
const MAX_BROWSER_SLOTS: usize = 4;

/// Standard-mapping button index → field setter.
///
/// References the W3C "standard" mapping table:
/// 0=A, 1=B, 2=X (unused), 3=Y, 4=LB, 5=RB, 6=LT (analog), 7=RT (analog),
/// 8=Back (unused), 9=Start, 10=L3, 11=R3 (unused), 12=DpadUp, 13=DpadDown,
/// 14=DpadLeft, 15=DpadRight.
const NUM_TRACKED_BUTTONS: usize = 16;

/// Per-slot bookkeeping the engine state doesn't carry — used to detect
/// press/release edges between polls.
#[derive(Clone, Copy, Default)]
struct SlotBookkeeping {
    /// Whether each button was held at the end of the previous poll. The
    /// rising edge (`pressed && !prev`) fires the engine's per-frame
    /// `*_pressed` flags + the double-tap-A check.
    prev_pressed: [bool; NUM_TRACKED_BUTTONS],
    /// Whether we've ever seen this slot occupied. Used so a slot that
    /// goes from null → null (e.g. never connected) doesn't fire a
    /// spurious "disconnected" event.
    ever_occupied: bool,
}

pub struct WebBackend {
    /// One state per browser slot (lazily grown up to [`MAX_BROWSER_SLOTS`]).
    states: Vec<GamepadState>,
    /// Per-slot edge-detection state.
    bookkeeping: Vec<SlotBookkeeping>,
}

impl WebBackend {
    pub fn new() -> Self {
        Self {
            states: Vec::new(),
            bookkeeping: Vec::new(),
        }
    }

    /// Ensure `states` and `bookkeeping` have at least `len` entries —
    /// pads with disconnected slots so the engine sees a stable index
    /// space.
    fn ensure_capacity(&mut self, len: usize) {
        while self.states.len() < len {
            let mut state = GamepadState::new_connected();
            state.connected = false; // not yet seen
            self.states.push(state);
            self.bookkeeping.push(SlotBookkeeping::default());
        }
    }

    /// Map a `web_sys::Gamepad` slot into the engine state. Called from
    /// `update` for each connected slot. The standard-mapping button
    /// dispatch lives in the parent module so it can be unit-tested on
    /// native (`apply_standard_button_press`, `apply_standard_button_value`).
    fn ingest_pad(
        state: &mut GamepadState,
        book: &mut SlotBookkeeping,
        pad: &Gamepad,
    ) {
        // Sticks → raw axes; `state_to_intent` applies the deadzone.
        let axes = pad.axes();
        let axis = |i: u32| -> f32 {
            axes.get(i).as_f64().unwrap_or(0.0) as f32
        };
        state.left_x = axis(0);
        state.left_y = axis(1);
        state.right_x = axis(2);
        state.right_y = axis(3);

        // Buttons → edge events + held state.
        let buttons = pad.buttons();
        let len = buttons.length() as usize;
        for i in 0..NUM_TRACKED_BUTTONS.min(len) {
            let btn_val = buttons.get(i as u32);
            // Each entry is either a GamepadButton or a number (legacy).
            // The standard surface in modern Chromium is GamepadButton.
            let btn: Option<GamepadButton> = btn_val.dyn_into().ok();
            let (pressed, value) = match btn {
                Some(b) => (b.pressed(), b.value() as f32),
                None => continue,
            };

            // Held analog state (triggers, a_held).
            apply_standard_button_value(state, i, pressed, value);

            // Rising-edge events.
            if pressed && !book.prev_pressed[i] {
                apply_standard_button_press(state, i);
            }

            book.prev_pressed[i] = pressed;
        }
    }

    /// Poll `navigator.getGamepads()`. Called once per frame by
    /// [`super::GamepadSystem::update`].
    pub fn update(&mut self) {
        // Reset per-frame flags before reading new edges in.
        for gp in &mut self.states {
            gp.reset_frame_flags();
        }

        // Pull the gamepad list from the browser. If `window()` or
        // `navigator.getGamepads()` is unavailable, just leave state
        // alone — every slot stays disconnected.
        let Some(window) = web_sys::window() else { return };
        let nav = window.navigator();
        let Ok(pads_array) = nav.get_gamepads() else { return };

        let pads_len = pads_array.length() as usize;
        self.ensure_capacity(pads_len.min(MAX_BROWSER_SLOTS));

        for slot in 0..pads_len.min(MAX_BROWSER_SLOTS) {
            let entry = pads_array.get(slot as u32);
            if entry.is_null() || entry.is_undefined() {
                // Slot empty this frame. If it was previously occupied,
                // mark a disconnect; otherwise just leave it alone.
                if let Some(state) = self.states.get_mut(slot) {
                    if state.connected {
                        state.connected = false;
                        state.disconnected_this_frame = true;
                        state.a_held = false;
                        log::info!("Gamepad disconnected (slot {})", slot);
                    }
                }
                if let Some(book) = self.bookkeeping.get_mut(slot) {
                    book.prev_pressed = [false; NUM_TRACKED_BUTTONS];
                }
                continue;
            }

            let pad: Gamepad = match entry.dyn_into() {
                Ok(p) => p,
                Err(_) => continue, // not a Gamepad — unexpected, skip
            };

            if !pad.connected() {
                continue;
            }

            // Standard mapping only. Non-standard controllers stay
            // invisible to the engine; log once per slot so a real
            // hardware report can be filed.
            if pad.mapping() != GamepadMappingType::Standard {
                let book = &mut self.bookkeeping[slot];
                if !book.ever_occupied {
                    log::warn!(
                        "Gamepad in slot {} reports non-standard mapping ({:?}); ignoring. \
                         Re-test with a different controller or file a remapping issue.",
                        slot,
                        pad.mapping(),
                    );
                    book.ever_occupied = true;
                }
                continue;
            }

            let state = &mut self.states[slot];
            let book = &mut self.bookkeeping[slot];

            // New-connection edge.
            if !state.connected {
                state.connected = true;
                state.disconnected_this_frame = false;
                book.ever_occupied = true;
                log::info!("Gamepad connected (slot {}): {}", slot, pad.id());
            }

            Self::ingest_pad(state, book, &pad);
        }
    }

    pub fn states(&self) -> &[GamepadState] {
        &self.states
    }
}

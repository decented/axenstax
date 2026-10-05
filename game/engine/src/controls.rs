//! Controls reference — the ONE source of truth for "what do the keys do?".
//!
//! Three surfaces read the tables below, so they can never drift apart:
//!   - the first-spawn **controls card** (shown once per device, persisted via
//!     `GraphicsSettings::controls_card_seen`),
//!   - the pause menu's **Controls** button (reopens the same card),
//!   - the free-play **H help sheet** ([`help_sheet_text`]).
//!
//! Every row cites the real binding it documents in `source` (a file + symbol,
//! not a line number — lines drift). **Do not add a row without a binding**: the
//! unit tests below press the documented keys on a real [`crate::input::InputState`]
//! and assert the resulting flag, and a text guard keeps feedback / gamepad
//! wording out (the web build has no feedback channel, native `/bug` is off by
//! default behind a hidden tester unlock, and gamepad support is parked — no
//! parity is claimed).
//!
//! Platform: [`ControlsLayout::current`] picks the touch table on a touch
//! device (`touch_input::is_touch_device`, set from `navigator.maxTouchPoints`
//! on the web and unconditionally on Android) and the keyboard table otherwise.
//! `TOUCH_PLATFORM` is deliberately NOT used: it is true for every web build,
//! including a desktop with a mouse.

use egui::RichText;

/// Which table to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlsLayout {
    Keyboard,
    Touch,
}

impl ControlsLayout {
    /// Touch on a touch device (phone / tablet / touchscreen Chromebook /
    /// Android), keyboard + mouse everywhere else.
    pub fn current() -> Self {
        if crate::touch_input::is_touch_device() {
            ControlsLayout::Touch
        } else {
            ControlsLayout::Keyboard
        }
    }
}

/// One line of the reference.
#[derive(Clone, Copy, Debug)]
pub struct ControlRow {
    /// What the player presses / touches.
    pub keys: &'static str,
    /// What it does.
    pub action: &'static str,
    /// The real binding this row documents (file + symbol). Read by the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub source: &'static str,
    /// Hidden on the web build (e.g. F11 — the browser owns fullscreen).
    pub native_only: bool,
}

/// A titled group of rows.
#[derive(Clone, Copy, Debug)]
pub struct ControlSection {
    pub title: &'static str,
    /// Included in the H help sheet (the card always shows every section).
    pub in_help_sheet: bool,
    pub rows: &'static [ControlRow],
}

const fn row(keys: &'static str, action: &'static str, source: &'static str) -> ControlRow {
    ControlRow { keys, action, source, native_only: false }
}

const fn native_row(keys: &'static str, action: &'static str, source: &'static str) -> ControlRow {
    ControlRow { keys, action, source, native_only: true }
}

/// Mouse + keyboard. Bindings: `input.rs` (`InputState::key_pressed`, the
/// movement helpers), `lib.rs` (window-event key handling: T / slash, Esc, F3,
/// F11, mouse wheel), `game_loop.rs` (hold-to-zoom), `physics.rs`.
pub const KEYBOARD_SECTIONS: &[ControlSection] = &[
    ControlSection {
        title: "Moving",
        in_help_sheet: true,
        rows: &[
            row("W A S D", "Walk", "input.rs InputState::forward/backward/left/right_key"),
            row("Mouse", "Look around", "input.rs InputState::mouse_moved"),
            row("Space", "Jump (swim up in water)", "input.rs InputState::jump; physics.rs swim"),
            row("Shift", "Sneak: creep to a ledge without falling (dive in water)", "input.rs InputState::sneak; physics.rs swim"),
            row("Double-tap W, or hold Ctrl", "Sprint", "input.rs key_pressed KeyW double-tap; InputState::sprint"),
            row("Double-tap Space", "Fly on / off (Creative worlds)", "input.rs key_pressed Space double-tap; physics.rs toggle_flight"),
            row("Hold C", "Zoom in", "game_loop.rs hold-to-zoom (KeyC)"),
            row("F5", "Third-person camera", "input.rs key_pressed F5 -> camera_cycle"),
        ],
    },
    ControlSection {
        title: "Building and items",
        in_help_sheet: true,
        rows: &[
            row("Left click (hold)", "Break blocks: hold until it breaks", "input.rs mouse_button_pressed Left -> break_block"),
            row("Right click", "Place a block, use what you hold, open things", "input.rs mouse_button_pressed Right -> place_block"),
            row("1 to 9, or scroll wheel", "Choose the item in your hand", "input.rs key_pressed Digit1..9; lib.rs MouseWheel -> scroll"),
            row("E", "Open your inventory and crafting", "input.rs key_pressed KeyE -> inventory_toggle"),
            row("Q", "Drop one of the item in your hand", "input.rs key_pressed KeyQ -> drop_item"),
            row("B", "Search every item (Creative: click one to take it)", "input.rs key_pressed KeyB -> explorer_toggle"),
        ],
    },
    ControlSection {
        title: "Menus and maps",
        in_help_sheet: true,
        rows: &[
            row("Esc", "Pause menu (also closes an open panel)", "lib.rs KeyCode::Escape"),
            row("H", "This help / your current objective", "input.rs key_pressed KeyH -> help_panel"),
            row("M", "Full-screen map", "input.rs key_pressed KeyM -> map_toggle"),
            row("J", "Challenges (Trials)", "input.rs key_pressed KeyJ -> challenge_board_toggle"),
            row("N", "Call Satoshi over (press again to send him home)", "input.rs key_pressed KeyN -> satoshi_summon"),
            row("T or /", "Chat and commands (in worlds with commands on)", "lib.rs KeyCode::KeyT / KeyCode::Slash, is_commands_enabled"),
        ],
    },
    ControlSection {
        title: "Extras",
        in_help_sheet: false,
        rows: &[
            row("F3", "Show / hide the debug overlay", "lib.rs KeyCode::F3"),
            native_row("F11", "Fullscreen on / off", "lib.rs KeyCode::F11 (native only)"),
        ],
    },
];

/// Touch layout. Bindings: `touch_input.rs` (`layout_buttons`, `ButtonKind`,
/// `TouchInput::on_touch_start`, the joystick sprint threshold).
pub const TOUCH_SECTIONS: &[ControlSection] = &[
    ControlSection {
        title: "Moving",
        in_help_sheet: true,
        rows: &[
            row("Left thumb stick", "Walk: push it to the edge to sprint", "touch_input.rs joystick (sprint at mag > 0.92)"),
            row("Drag anywhere else", "Look around", "touch_input.rs TouchZone::Look"),
            row("Jump button", "Jump (swim up in water)", "touch_input.rs ButtonKind::Jump"),
            row("Sneak button (hold)", "Sneak: creep to a ledge without falling", "touch_input.rs ButtonKind::Sneak"),
            row("View button", "Third-person camera", "touch_input.rs ButtonKind::Perspective -> camera_cycle"),
            row("Zoom button (hold)", "Zoom in", "touch_input.rs ButtonKind::Zoom"),
        ],
    },
    ControlSection {
        title: "Building and items",
        in_help_sheet: true,
        rows: &[
            row("Break button (hold)", "Break blocks: hold until it breaks", "touch_input.rs ButtonKind::Break"),
            row("Place button", "Place a block, use what you hold, open things", "touch_input.rs ButtonKind::Place"),
            row("Number slots along the bottom", "Choose the item in your hand", "touch_input.rs TouchZone::Hotbar"),
            row("Inventory button", "Open your inventory and crafting", "touch_input.rs ButtonKind::Inventory"),
        ],
    },
    ControlSection {
        title: "Menus",
        in_help_sheet: true,
        rows: &[
            row("Pause button (top left)", "Pause menu", "touch_input.rs ButtonKind::Pause"),
            row("Chat button (top left)", "Chat and commands (in worlds with commands on)", "touch_input.rs ButtonKind::Chat"),
        ],
    },
];

/// Extra free-play lines for the H help sheet, keyboard only. Verified against
/// `companion.rs` (Follow / Stay / Wander cycle) and `breeding.rs` +
/// `tameable.rs` (sneak-gated feed / cull) when they were first written.
const HELP_SHEET_EXTRAS: &[&str] = &[
    "Empty hand, right-click a tamed pet: Follow / Stay / Wander",
    "Sneak: careful actions (breeding a mount, culling your own pet)",
];

/// The table for a layout.
pub fn sections(layout: ControlsLayout) -> &'static [ControlSection] {
    match layout {
        ControlsLayout::Keyboard => KEYBOARD_SECTIONS,
        ControlsLayout::Touch => TOUCH_SECTIONS,
    }
}

fn row_visible(r: &ControlRow) -> bool {
    !r.native_only || cfg!(not(target_arch = "wasm32"))
}

/// The H help sheet's free-play body: every help-sheet row as a bullet line
/// ("keys - action"), plus the pet / sneak notes on the keyboard layout.
pub fn help_sheet_text(layout: ControlsLayout) -> String {
    let mut out = String::new();
    for section in sections(layout).iter().filter(|s| s.in_help_sheet) {
        for r in section.rows.iter().filter(|r| row_visible(r)) {
            out.push_str(&format!("\u{2022} {} \u{2014} {}\n", r.keys, r.action));
        }
    }
    if layout == ControlsLayout::Keyboard {
        for line in HELP_SHEET_EXTRAS {
            out.push_str(&format!("\u{2022} {line}\n"));
        }
    }
    out.trim_end().to_string()
}

/// One-line intro under the card title.
fn intro(layout: ControlsLayout) -> &'static str {
    match layout {
        ControlsLayout::Keyboard => {
            "Mouse and keyboard. You can open this again any time: Esc, then Controls."
        }
        ControlsLayout::Touch => {
            "Touch controls. You can open this again any time: Pause button, then Controls."
        }
    }
}

/// Draw the controls card. Returns `true` on the frame the player dismisses it
/// ("Got it" or Enter). Esc is handled by the main loop's panel-closing chain
/// (`GameState::close_topmost_ui_panel`), which is why it is not read here.
pub fn show_controls_card(ctx: &egui::Context, layout: ControlsLayout) -> bool {
    let gold = egui::Color32::from_rgb(255, 210, 130);
    let mut close = false;
    // Keep the card inside small (phone) screens: scroll the table, pin the button.
    let max_h = (ctx.content_rect().height() * 0.65).max(220.0);

    egui::Window::new("Controls")
        .id(egui::Id::new("controls_card"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(380.0);
            ui.label(RichText::new(intro(layout)).size(12.0).weak());
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(max_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (si, section) in sections(layout).iter().enumerate() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(section.title).size(14.0).strong().color(gold));
                        egui::Grid::new(("controls_card_grid", si))
                            .num_columns(2)
                            .spacing([18.0, 4.0])
                            .show(ui, |ui| {
                                for r in section.rows.iter().filter(|r| row_visible(r)) {
                                    ui.label(RichText::new(r.keys).strong().monospace());
                                    ui.label(r.action);
                                    ui.end_row();
                                }
                            });
                    }
                });
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                let btn = egui::Button::new(RichText::new("Got it").size(16.0))
                    .min_size(egui::vec2(160.0, 36.0));
                if ui.add(btn).clicked() {
                    close = true;
                }
            });
        });

    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        close = true;
    }
    close
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::InputState;
    use winit::keyboard::KeyCode;

    fn all_rows(layout: ControlsLayout) -> impl Iterator<Item = &'static ControlRow> {
        sections(layout).iter().flat_map(|s| s.rows.iter())
    }

    #[test]
    fn every_row_is_filled_in_and_cites_a_binding() {
        for layout in [ControlsLayout::Keyboard, ControlsLayout::Touch] {
            assert!(!sections(layout).is_empty());
            for s in sections(layout) {
                assert!(!s.title.is_empty());
                assert!(!s.rows.is_empty(), "empty section '{}'", s.title);
            }
            for r in all_rows(layout) {
                assert!(!r.keys.trim().is_empty());
                assert!(!r.action.trim().is_empty());
                assert!(
                    r.source.contains(".rs"),
                    "row '{}' must cite the source file of its binding",
                    r.keys
                );
            }
        }
    }

    #[test]
    fn touch_rows_cite_touch_input_and_keyboard_rows_do_not() {
        for r in all_rows(ControlsLayout::Touch) {
            assert!(r.source.starts_with("touch_input.rs"), "{}", r.keys);
        }
        for r in all_rows(ControlsLayout::Keyboard) {
            assert!(!r.source.starts_with("touch_input.rs"), "{}", r.keys);
        }
    }

    #[test]
    fn no_row_promises_feedback_gamepad_or_money() {
        // Web has no feedback channel; native /bug is off behind a hidden
        // tester unlock; gamepad is parked; the play surface never says money.
        let banned = [
            "feedback", "/bug", "/idea", "tell us", "report", "gamepad", "controller",
            "bitcoin", "sats", "earn",
        ];
        let mut texts: Vec<String> = Vec::new();
        for layout in [ControlsLayout::Keyboard, ControlsLayout::Touch] {
            for r in all_rows(layout) {
                texts.push(format!("{} {}", r.keys, r.action).to_lowercase());
            }
            texts.push(intro(layout).to_lowercase());
            texts.push(help_sheet_text(layout).to_lowercase());
        }
        for t in &texts {
            for b in banned {
                assert!(!t.contains(b), "controls text '{t}' contains '{b}'");
            }
        }
    }

    #[test]
    fn keyboard_keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for r in all_rows(ControlsLayout::Keyboard) {
            assert!(seen.insert(r.keys), "duplicate key row '{}'", r.keys);
        }
    }

    #[test]
    fn documented_keyboard_bindings_really_do_what_the_card_says() {
        // Press the documented keys on a real InputState.
        let mut i = InputState::new();
        i.key_pressed(KeyCode::Digit1);
        assert_eq!(i.hotbar_select, Some(0), "1 selects slot 1");
        i.key_pressed(KeyCode::Digit9);
        assert_eq!(i.hotbar_select, Some(8), "9 selects slot 9");

        let mut i = InputState::new();
        i.key_pressed(KeyCode::KeyE);
        assert!(i.inventory_toggle, "E opens the inventory");
        i.key_pressed(KeyCode::KeyQ);
        assert!(i.drop_item, "Q drops");
        i.key_pressed(KeyCode::KeyB);
        assert!(i.explorer_toggle, "B opens the item search");
        i.key_pressed(KeyCode::KeyH);
        assert!(i.help_panel, "H opens help");
        i.key_pressed(KeyCode::KeyM);
        assert!(i.map_toggle, "M opens the map");
        i.key_pressed(KeyCode::KeyJ);
        assert!(i.challenge_board_toggle, "J opens challenges");
        i.key_pressed(KeyCode::KeyN);
        assert!(i.satoshi_summon, "N calls Satoshi");
        i.key_pressed(KeyCode::F5);
        assert!(i.camera_cycle, "F5 cycles the camera");

        // Movement + modifiers.
        let mut i = InputState::new();
        for k in [KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD] {
            i.key_pressed(k);
        }
        assert!(i.forward() && i.left() && i.backward() && i.right_key(), "WASD");
        i.key_pressed(KeyCode::Space);
        assert!(i.jump(), "Space jumps");
        i.key_pressed(KeyCode::ShiftLeft);
        assert!(i.sneak(), "Shift sneaks");
        i.key_pressed(KeyCode::ControlLeft);
        assert!(i.sprint(), "Ctrl sprints");

        // Double-tap W sprints; double-tap Space asks to toggle flight.
        let mut i = InputState::new();
        i.key_pressed(KeyCode::KeyW);
        i.key_released(KeyCode::KeyW);
        i.key_pressed(KeyCode::KeyW);
        assert!(i.sprinting, "double-tap W sprints");
        i.key_pressed(KeyCode::Space);
        i.key_released(KeyCode::Space);
        i.key_pressed(KeyCode::Space);
        assert!(i.toggle_flight, "double-tap Space toggles flight");
    }

    #[test]
    fn help_sheet_has_the_movement_basics() {
        let kb = help_sheet_text(ControlsLayout::Keyboard);
        for needle in ["W A S D", "Space", "Shift", "1 to 9", "E", "Q", "T or /", "Esc"] {
            assert!(kb.contains(needle), "keyboard help sheet lacks '{needle}'");
        }
        // The old free-play cheat-sheet lines survive.
        assert!(kb.contains("tamed pet"));
        let touch = help_sheet_text(ControlsLayout::Touch);
        assert!(touch.contains("stick"));
        assert!(!touch.contains("W A S D"));
        // Debug / fullscreen stay out of the short sheet.
        assert!(!kb.contains("F3"));
    }
}

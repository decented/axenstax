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
//! wording out (the game has no feedback channel, and gamepad support is parked
//! — no parity is claimed).
//!
//! Keys follow the live bindings (Settings → Controls, `keybindings.rs`): a row
//! names the [`Action`]s it documents, shows its static text while they all have
//! their shipped binding, and builds its text from their live labels once the
//! player has changed one ([`ControlRow::keys_text`]).
//!
//! Platform: [`ControlsLayout::current`] picks the touch table on a touch
//! device (`touch_input::is_touch_device`, set from `navigator.maxTouchPoints`
//! on the web and unconditionally on Android) and the keyboard table otherwise.
//! `TOUCH_PLATFORM` is deliberately NOT used: it is true for every web build,
//! including a desktop with a mouse.

use crate::keybindings::{Action, Bindings};
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
    /// What the player presses / touches, with the shipped bindings.
    pub keys: &'static str,
    /// What it does.
    pub action: &'static str,
    /// The real binding this row documents (file + symbol). Read by the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub source: &'static str,
    /// Hidden on the web build (e.g. F11 — the browser owns fullscreen).
    pub native_only: bool,
    /// The remappable actions this row names, in the order `compose` takes
    /// their labels. Empty for fixed input (Esc, an F key, the mouse's look,
    /// a touch control).
    pub actions: &'static [Action],
    /// Builds the keys text from those actions' live labels.
    pub compose: fn(&[String]) -> String,
}

impl ControlRow {
    /// The keys as the player has them bound: the static text while every
    /// action is at its default (so the shipped card is unchanged), otherwise
    /// built from the live labels.
    pub fn keys_text(&self, bindings: &Bindings) -> String {
        if self.actions.iter().all(|a| bindings.is_default(*a)) {
            return self.keys.to_string();
        }
        let labels: Vec<String> = self.actions.iter().map(|a| bindings.hint(*a)).collect();
        (self.compose)(&labels)
    }
}

fn first(l: &[String]) -> String {
    l[0].clone()
}
fn spaced(l: &[String]) -> String {
    l.join(" ")
}
fn hold(l: &[String]) -> String {
    format!("Hold {}", l[0])
}
fn held(l: &[String]) -> String {
    format!("{} (hold)", l[0])
}
fn double_tap(l: &[String]) -> String {
    format!("Double-tap {}", l[0])
}
fn sprint_keys(l: &[String]) -> String {
    format!("Double-tap {}, or hold {}", l[0], l[1])
}
fn either(l: &[String]) -> String {
    format!("{} or {}", l[0], l[1])
}
fn hotbar_keys(l: &[String]) -> String {
    format!("{}, or scroll wheel", l.join(" "))
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
    ControlRow { keys, action, source, native_only: false, actions: &[], compose: first }
}

const fn native_row(keys: &'static str, action: &'static str, source: &'static str) -> ControlRow {
    ControlRow { keys, action, source, native_only: true, actions: &[], compose: first }
}

/// A row naming remappable actions.
const fn bound(
    keys: &'static str,
    action: &'static str,
    source: &'static str,
    actions: &'static [Action],
    compose: fn(&[String]) -> String,
) -> ControlRow {
    ControlRow { keys, action, source, native_only: false, actions, compose }
}

const HOTBAR: &[Action] = &[
    Action::Hotbar1,
    Action::Hotbar2,
    Action::Hotbar3,
    Action::Hotbar4,
    Action::Hotbar5,
    Action::Hotbar6,
    Action::Hotbar7,
    Action::Hotbar8,
    Action::Hotbar9,
];

/// Mouse + keyboard. Bindings: `input.rs` (`InputState::key_pressed`, the
/// movement helpers), `lib.rs` (window-event key handling: T / slash, Esc, F3,
/// F11, mouse wheel), `game_loop.rs` (hold-to-zoom), `physics.rs`.
pub const KEYBOARD_SECTIONS: &[ControlSection] = &[
    ControlSection {
        title: "Moving",
        in_help_sheet: true,
        rows: &[
            bound("W A S D", "Walk", "input.rs InputState::forward/backward/left/right_key", &[Action::Forward, Action::Left, Action::Back, Action::Right], spaced),
            row("Mouse", "Look around", "input.rs InputState::mouse_moved"),
            bound("Space", "Jump (swim up in water)", "input.rs InputState::jump; physics.rs swim", &[Action::Jump], first),
            bound("Shift", "Sneak: creep to a ledge without falling (dive in water)", "input.rs InputState::sneak; physics.rs swim", &[Action::Sneak], first),
            bound("Double-tap W, or hold Ctrl", "Sprint", "input.rs press_action Forward double-tap; InputState::sprint", &[Action::Forward, Action::Sprint], sprint_keys),
            bound("Double-tap Space", "Fly on / off (Creative worlds)", "input.rs press_action Jump double-tap; physics.rs toggle_flight", &[Action::Jump], double_tap),
            bound("Hold C", "Look through a spyglass (one in either hand)", "game_loop.rs spyglass look (Action::Zoom, spyglass::look_frame)", &[Action::Zoom], hold),
            row("F5", "Third-person camera", "input.rs press_action CameraView (F5, fixed) -> camera_cycle"),
        ],
    },
    ControlSection {
        title: "Building and items",
        in_help_sheet: true,
        rows: &[
            bound("Left click (hold)", "Break blocks: hold until it breaks", "input.rs press_action Attack -> break_block", &[Action::Attack], held),
            bound("Right click", "Place a block, use what you hold, open things (hold it to raise a shield or look through a spyglass)", "input.rs press_action Use -> place_block", &[Action::Use], first),
            bound("Middle click", "Pick the block you're looking at into your hand (in Survival, only if you're carrying it)", "input.rs press_action PickBlock -> pick_block", &[Action::PickBlock], first),
            bound("1 to 9, or scroll wheel", "Choose the item in your hand", "input.rs press_action Hotbar1..9; lib.rs MouseWheel -> scroll", HOTBAR, hotbar_keys),
            bound("E", "Open your inventory and crafting", "input.rs press_action Inventory -> inventory_toggle", &[Action::Inventory], first),
            bound("Q", "Drop one of the item in your hand", "input.rs press_action Drop -> drop_item", &[Action::Drop], first),
            bound("F", "Swap the item in your hand with your off-hand (right click uses the off-hand when your main hand has no use)", "input.rs press_action SwapHands -> swap_hands", &[Action::SwapHands], first),
            bound("B", "Search every item (Creative: click one to take it)", "input.rs press_action Explorer -> explorer_toggle", &[Action::Explorer], first),
        ],
    },
    ControlSection {
        title: "Menus and maps",
        in_help_sheet: true,
        rows: &[
            row("Esc", "Pause menu (also closes an open panel)", "lib.rs KeyCode::Escape"),
            bound("H", "This help / your current objective", "input.rs press_action Help -> help_panel", &[Action::Help], first),
            bound("M", "Full-screen map", "input.rs press_action Map -> map_toggle", &[Action::Map], first),
            bound("J", "Challenges (Trials)", "input.rs press_action Challenges -> challenge_board_toggle", &[Action::Challenges], first),
            bound("N", "Call Satoshi over (press again to send him home)", "input.rs press_action Satoshi -> satoshi_summon", &[Action::Satoshi], first),
            bound("T or /", "Chat and commands (in worlds with commands on)", "lib.rs chat_prefix (Action::Chat / Action::Command), is_commands_enabled", &[Action::Chat, Action::Command], either),
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
            row("Zoom button (hold)", "Look through a spyglass (one in either hand)", "touch_input.rs ButtonKind::Zoom"),
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
            row("Off-hand box (left of the number slots)", "Tap to swap it with the item in your hand", "touch_input.rs TouchZone::Offhand"),
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
/// ("keys - action", the keys as bound), plus the pet / sneak notes on the
/// keyboard layout.
pub fn help_sheet_text(layout: ControlsLayout, bindings: &Bindings) -> String {
    let mut out = String::new();
    for section in sections(layout).iter().filter(|s| s.in_help_sheet) {
        for r in section.rows.iter().filter(|r| row_visible(r)) {
            out.push_str(&format!("\u{2022} {} \u{2014} {}\n", r.keys_text(bindings), r.action));
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

/// Draw the controls card, its keys as bound. Returns `true` on the frame the
/// player dismisses it ("Got it" or Enter). Esc is handled by the main loop's
/// panel-closing chain (`GameState::close_topmost_ui_panel`), which is why it
/// is not read here.
pub fn show_controls_card(ctx: &egui::Context, layout: ControlsLayout, bindings: &Bindings) -> bool {
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
                                    ui.label(RichText::new(r.keys_text(bindings)).strong().monospace());
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
        // The game has no feedback channel; gamepad is parked; the play
        // surface never says money.
        let banned = [
            "feedback", "/bug", "/idea", "tell us", "report", "gamepad", "controller",
        ];
        let mut texts: Vec<String> = Vec::new();
        for layout in [ControlsLayout::Keyboard, ControlsLayout::Touch] {
            for r in all_rows(layout) {
                texts.push(format!("{} {}", r.keys, r.action).to_lowercase());
            }
            texts.push(intro(layout).to_lowercase());
            texts.push(help_sheet_text(layout, &Bindings::defaults()).to_lowercase());
        }
        for t in &texts {
            for b in banned {
                assert!(!t.contains(b), "controls text '{t}' contains '{b}'");
            }
            // Money words: the shared `copy_lint::BANNED_MONEY_WORDS`.
            let money = crate::copy_lint::banned_in(t, crate::copy_lint::BANNED_MONEY_WORDS);
            assert!(money.is_empty(), "controls text '{t}' contains {money:?}");
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
        i.key_pressed(KeyCode::KeyF);
        assert!(i.swap_hands, "F swaps the hands");
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
        i.mouse_button_pressed(winit::event::MouseButton::Middle);
        assert!(i.pick_block, "middle click picks the aimed block");

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
        let kb = help_sheet_text(ControlsLayout::Keyboard, &Bindings::defaults());
        for needle in ["W A S D", "Space", "Shift", "1 to 9", "E", "Q", "T or /", "Esc"] {
            assert!(kb.contains(needle), "keyboard help sheet lacks '{needle}'");
        }
        // The old free-play cheat-sheet lines survive.
        assert!(kb.contains("tamed pet"));
        let touch = help_sheet_text(ControlsLayout::Touch, &Bindings::defaults());
        assert!(touch.contains("stick"));
        assert!(!touch.contains("W A S D"));
        // Debug / fullscreen stay out of the short sheet.
        assert!(!kb.contains("F3"));
    }

    /// The card reads the live bindings: with the shipped ones every row is its
    /// static text (the card is unchanged); a rebound action's row shows the
    /// new key, and the help sheet with it.
    #[test]
    fn the_card_and_help_sheet_follow_the_live_bindings() {
        use crate::keybindings::Binding;
        let d = Bindings::defaults();
        for layout in [ControlsLayout::Keyboard, ControlsLayout::Touch] {
            for r in all_rows(layout) {
                assert_eq!(r.keys_text(&d), r.keys, "the shipped card is unchanged");
            }
        }
        let mut b = Bindings::defaults();
        b.rebind(Action::Drop, Binding::Key(KeyCode::KeyG));
        b.rebind(Action::Hotbar1, Binding::Key(KeyCode::KeyZ));
        b.rebind(Action::Forward, Binding::Key(KeyCode::ArrowUp));
        b.rebind(Action::Attack, Binding::Mouse(winit::event::MouseButton::Back));
        let rows: Vec<String> = all_rows(ControlsLayout::Keyboard).map(|r| r.keys_text(&b)).collect();
        assert!(rows.iter().any(|t| t == "G"), "the Drop row shows G: {rows:?}");
        assert!(!rows.iter().any(|t| t == "Q"), "and no longer Q: {rows:?}");
        assert!(rows.iter().any(|t| t == "Up A S D"), "{rows:?}");
        assert!(rows.iter().any(|t| t == "Double-tap Up, or hold Ctrl"), "{rows:?}");
        assert!(rows.iter().any(|t| t == "Z 2 3 4 5 6 7 8 9, or scroll wheel"), "{rows:?}");
        assert!(rows.iter().any(|t| t == "Mouse 4 (hold)"), "{rows:?}");
        let sheet = help_sheet_text(ControlsLayout::Keyboard, &b);
        assert!(sheet.contains("\u{2022} G \u{2014} Drop"), "{sheet}");
        // And the row's binding really does what it says on a real InputState.
        let mut i = InputState::new();
        i.set_bindings(std::sync::Arc::new(b));
        i.key_pressed(KeyCode::KeyG);
        assert!(i.drop_item, "G drops, as the card says");
    }

    /// Every row that names an action shows that action's binding while it is
    /// at its default, so the static text is the table's (no row can name a
    /// key the table doesn't bind).
    #[test]
    fn each_rows_static_text_names_its_actions_default_keys() {
        let d = Bindings::defaults();
        for r in all_rows(ControlsLayout::Keyboard) {
            for a in r.actions {
                let label = d.hint(*a);
                let named = r.keys.contains(&label) || (a.hotbar_slot().is_some() && r.keys.starts_with("1 to 9"));
                assert!(named, "row '{}' doesn't name {} ({label})", r.keys, a.id());
            }
        }
    }
}

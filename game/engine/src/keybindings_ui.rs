//! Settings → Controls: every action with its binding and a **Change** button,
//! the capture of the next key or mouse button, and **Reset all to defaults**.
//!
//! Drawn inside the shared Settings panel (`menu::draw_settings_panel`), so it is
//! reachable from the lobby and the pause menu, and on a phone it scrolls with
//! the rest of Settings (the panel's own drag-to-scroll area).
//!
//! A capture takes the next press BEFORE egui or the game sees it: `lib.rs`
//! asks [`KeyBindingsUi::capture_active`] at the top of its window-event
//! handler and, while it is true, hands the press to
//! [`KeyBindingsUi::take_press`] and returns. So the captured key never acts in
//! the game or in the menu (Enter can't re-press the focused Change button, Esc
//! cancels the capture instead of closing Settings). Touch is never captured:
//! a phone has no keys, and its Cancel button must stay tappable.

use crate::graphics_settings::GraphicsSettings;
use crate::keybindings::{Action, Binding, Bindings, Context, Rebind};
use egui::RichText;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// What the capturing row says.
pub const CAPTURE_PROMPT: &str = "Press a key or mouse button (Esc to cancel)";

/// A press a capture can take. `Key(Escape)` (and the Android Back key, which
/// `lib.rs` folds onto Escape) cancels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturePress {
    Key(KeyCode),
    Mouse(MouseButton),
}

/// What a press did while a capture was active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureOutcome {
    /// The press was taken by the capture: nothing else may see it.
    pub swallowed: bool,
    /// The stored overrides changed (save them; the game rebuilds its bindings).
    pub changed: bool,
}

/// The Controls list's own state (lives in `SettingsPanelState`).
#[derive(Default)]
pub struct KeyBindingsUi {
    /// The action waiting for its new key while its row shows [`CAPTURE_PROMPT`].
    pub capturing: Option<Action>,
    /// The egui pass the list was last drawn in. A capture only takes presses
    /// while the list is on screen, so a capture left behind by a closed panel
    /// can never eat a key in play.
    drawn_pass: Option<u64>,
    /// The last outcome line ("Drop moved to G", a refusal).
    pub notice: Option<String>,
    /// The Reset all confirm is up.
    confirm_reset: bool,
    /// Give the confirm's Cancel the keyboard focus on its first frame.
    focus_cancel: bool,
}

impl KeyBindingsUi {
    /// True while a capture should take the next press: one is armed and the
    /// list was drawn in the latest egui pass (`current_pass` is
    /// `Context::cumulative_pass_nr`, which counts finished passes).
    pub fn capture_active(&self, current_pass: u64) -> bool {
        self.capturing.is_some() && self.drawn_pass.is_some_and(|p| current_pass <= p + 1)
    }

    /// Take a press while a capture is armed. Esc cancels; any other key or
    /// mouse button is offered as the new binding (a fixed or unnamed one is
    /// refused with a line; one used in the same context is swapped). The
    /// capture ends either way. Not armed: nothing is taken.
    pub fn take_press(&mut self, gfx: &mut GraphicsSettings, press: CapturePress) -> CaptureOutcome {
        let Some(action) = self.capturing.take() else {
            return CaptureOutcome::default();
        };
        let binding = match press {
            CapturePress::Key(KeyCode::Escape) => {
                self.notice = None;
                return CaptureOutcome { swallowed: true, changed: false };
            }
            CapturePress::Key(k) => Some(Binding::Key(k)),
            CapturePress::Mouse(m) => Binding::from_mouse(m),
        };
        let (mut bindings, _) = Bindings::from_overrides(&gfx.key_bindings);
        let result = match binding {
            Some(b) => bindings.rebind(action, b),
            None => Rebind::Unsupported,
        };
        self.notice = result.message();
        let changed = matches!(result, Rebind::Bound | Rebind::Swapped { .. });
        if changed {
            gfx.key_bindings = bindings.overrides();
        }
        CaptureOutcome { swallowed: true, changed }
    }

    /// Drop any capture or open confirm (Settings closed).
    pub fn cancel(&mut self) {
        self.capturing = None;
        self.confirm_reset = false;
        self.focus_cancel = false;
    }
}

/// The Controls section: a collapsed header that opens onto the list. Returns
/// true when Reset all changed the stored overrides (a capture's change comes
/// through [`KeyBindingsUi::take_press`] instead). Collapsing it drops a capture.
pub fn draw(ui: &mut egui::Ui, st: &mut KeyBindingsUi, gfx: &mut GraphicsSettings, ts: f32) -> bool {
    let mut changed = false;
    let shown = egui::CollapsingHeader::new(
        RichText::new("Controls").size(14.0 * ts).color(crate::brand::COPPER),
    )
    .id_salt("settings_controls")
    .show(ui, |ui| {
        changed = draw_list(ui, st, gfx, ts);
    });
    if shown.body_returned.is_none() && (st.capturing.is_some() || st.confirm_reset) {
        st.cancel();
    }
    changed
}

/// The list itself: one row per action, grouped by context.
pub fn draw_list(ui: &mut egui::Ui, st: &mut KeyBindingsUi, gfx: &mut GraphicsSettings, ts: f32) -> bool {
    st.drawn_pass = Some(ui.ctx().cumulative_pass_nr());
    let (bindings, _) = Bindings::from_overrides(&gfx.key_bindings);
    let mut changed = false;
    let dim = crate::brand::MUTED_TEXT;
    ui.label(
        RichText::new("Change a key: press Change, then the key or mouse button you want. Esc and F1 to F12 stay as they are.")
            .size(11.0 * ts)
            .color(dim),
    );
    if let Some(note) = &st.notice {
        ui.label(RichText::new(note).size(12.0 * ts).color(crate::brand::COPPER));
    }
    for context in Context::ALL {
        ui.add_space(6.0);
        ui.label(RichText::new(context.title()).size(13.0 * ts).strong());
        egui::Grid::new(("settings_controls_grid", context))
            .num_columns(3)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                for action in Action::ALL.into_iter().filter(|a| a.context() == context) {
                    ui.label(RichText::new(action.name()).size(12.0 * ts));
                    if st.capturing == Some(action) {
                        ui.label(RichText::new(CAPTURE_PROMPT).size(12.0 * ts).color(crate::brand::COPPER));
                        // Touch is never captured, so a phone (whose Back is
                        // Escape anyway) also gets a button to back out.
                        if crate::touch_input::is_touch_device() {
                            if ui.button("Cancel").clicked() {
                                st.capturing = None;
                            }
                        } else {
                            ui.label("");
                        }
                    } else {
                        ui.label(RichText::new(bindings.label(action)).size(12.0 * ts).strong().monospace());
                        if action.is_fixed() {
                            ui.label(RichText::new("fixed").size(11.0 * ts).color(dim));
                        } else if ui.button(RichText::new("Change").size(12.0 * ts)).clicked() {
                            st.capturing = Some(action);
                            st.notice = None;
                            st.confirm_reset = false;
                        }
                    }
                    ui.end_row();
                }
            });
    }
    ui.add_space(8.0);
    if !st.confirm_reset {
        if ui.button(RichText::new("Reset all to defaults").size(12.0 * ts)).clicked() {
            st.confirm_reset = true;
            st.focus_cancel = true;
            st.capturing = None;
        }
    } else {
        ui.label(RichText::new("Put every control back to its default?").size(12.0 * ts));
        ui.horizontal(|ui| {
            let cancel = ui.button(RichText::new("Cancel").size(12.0 * ts));
            if st.focus_cancel {
                cancel.request_focus();
                st.focus_cancel = false;
            }
            let reset = ui.button(RichText::new("Reset").size(12.0 * ts));
            if cancel.clicked() {
                st.confirm_reset = false;
            } else if reset.clicked_by(egui::PointerButton::Primary) {
                // A pointer click only: `clicked()` would also take Enter or
                // Space on a focused button, and Enter must never reset.
                st.confirm_reset = false;
                if !gfx.key_bindings.is_empty() {
                    gfx.key_bindings.clear();
                    changed = true;
                }
                st.notice = Some("Every control is back to its default.".to_string());
            }
        });
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armed(action: Action) -> KeyBindingsUi {
        KeyBindingsUi { capturing: Some(action), drawn_pass: Some(10), ..Default::default() }
    }

    #[test]
    fn a_capture_swallows_the_key_and_rebinds_the_action() {
        let mut st = armed(Action::Drop);
        let mut gfx = GraphicsSettings::default();
        assert!(st.capture_active(11), "the list was drawn in the latest pass");
        let out = st.take_press(&mut gfx, CapturePress::Key(KeyCode::KeyG));
        assert_eq!(out, CaptureOutcome { swallowed: true, changed: true });
        assert_eq!(gfx.key_bindings.get("drop").map(String::as_str), Some("KeyG"));
        assert_eq!(st.capturing, None, "one press, one capture");
        assert!(!st.capture_active(11));
        // Not armed: a press is not taken (it goes on to egui and the game).
        let out = st.take_press(&mut gfx, CapturePress::Key(KeyCode::KeyH));
        assert_eq!(out, CaptureOutcome::default());
    }

    #[test]
    fn esc_cancels_a_fixed_key_is_refused_and_a_mouse_button_binds() {
        let mut gfx = GraphicsSettings::default();
        let mut st = armed(Action::Drop);
        let out = st.take_press(&mut gfx, CapturePress::Key(KeyCode::Escape));
        assert_eq!(out, CaptureOutcome { swallowed: true, changed: false }, "Esc cancels, and is eaten");
        assert!(gfx.key_bindings.is_empty());

        let mut st = armed(Action::Drop);
        let out = st.take_press(&mut gfx, CapturePress::Key(KeyCode::F5));
        assert_eq!(out, CaptureOutcome { swallowed: true, changed: false });
        assert!(st.notice.as_deref().is_some_and(|n| n.contains("F1 to F12")), "{:?}", st.notice);
        assert!(gfx.key_bindings.is_empty());

        let mut st = armed(Action::Attack);
        let out = st.take_press(&mut gfx, CapturePress::Mouse(MouseButton::Back));
        assert_eq!(out, CaptureOutcome { swallowed: true, changed: true });
        assert_eq!(gfx.key_bindings.get("attack").map(String::as_str), Some("MouseBack"));

        let mut st = armed(Action::Drop);
        let out = st.take_press(&mut gfx, CapturePress::Mouse(MouseButton::Other(9)));
        assert_eq!(out, CaptureOutcome { swallowed: true, changed: false }, "an unnamed button is refused");
    }

    #[test]
    fn a_clash_swaps_and_says_so() {
        let mut gfx = GraphicsSettings::default();
        let mut st = armed(Action::Drop);
        st.take_press(&mut gfx, CapturePress::Key(KeyCode::KeyE));
        assert_eq!(st.notice.as_deref(), Some("Inventory moved to Q"));
        assert_eq!(gfx.key_bindings.get("drop").map(String::as_str), Some("KeyE"));
        assert_eq!(gfx.key_bindings.get("inventory").map(String::as_str), Some("KeyQ"));
    }

    #[test]
    fn a_capture_left_behind_by_a_closed_list_takes_nothing() {
        let st = armed(Action::Drop);
        assert!(st.capture_active(11));
        assert!(!st.capture_active(12), "not drawn in the latest pass: the list is closed");
        let mut st = armed(Action::Drop);
        st.cancel();
        assert!(!st.capture_active(11));
    }

    // ── The list, drawn headless ────────────────────────────────────────────

    struct Rig {
        ctx: egui::Context,
        st: KeyBindingsUi,
        gfx: GraphicsSettings,
        out: egui::FullOutput,
        changed: bool,
    }

    fn placed(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
        fn walk(shape: &egui::epaint::Shape, label: &str, out: &mut Option<egui::Rect>) {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == label && out.is_none() => {
                    *out = Some(t.galley.rect.translate(t.pos.to_vec2()));
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, label, out)),
                _ => {}
            }
        }
        let mut out = None;
        output.shapes.iter().for_each(|c| walk(&c.shape, label, &mut out));
        out
    }

    impl Rig {
        fn new() -> Rig {
            let mut gfx = GraphicsSettings::default();
            gfx.key_bindings.insert("drop".to_string(), "KeyG".to_string());
            let mut rig = Rig { ctx: egui::Context::default(), st: KeyBindingsUi::default(), gfx, out: Default::default(), changed: false };
            for _ in 0..3 {
                rig.frame(vec![]);
            }
            rig
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 3000.0))),
                events,
                ..Default::default()
            };
            let (st, gfx, changed) = (&mut self.st, &mut self.gfx, &mut self.changed);
            self.out = self.ctx.run_ui(input, |ui| {
                if draw_list(ui, st, gfx, 1.0) {
                    *changed = true;
                }
            });
        }

        fn click(&mut self, label: &str) {
            let at = placed(&self.out, label).unwrap_or_else(|| panic!("no {label:?} drawn")).center();
            let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(vec![]);
        }

        fn enter(&mut self) {
            let key = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
            self.frame(vec![key(true)]);
            self.frame(vec![key(false)]);
            self.frame(vec![]);
        }
    }

    #[test]
    fn the_list_shows_every_action_its_label_and_the_fixed_keys() {
        let rig = Rig::new();
        for a in Action::ALL {
            assert!(placed(&rig.out, a.name()).is_some(), "no row for {}", a.id());
        }
        assert!(placed(&rig.out, "G").is_some(), "Drop shows its override");
        assert!(placed(&rig.out, "Esc").is_some() && placed(&rig.out, "F5").is_some());
        assert!(placed(&rig.out, "fixed").is_some());
    }

    #[test]
    fn change_arms_a_capture_for_that_row() {
        let mut rig = Rig::new();
        // The first Change button is the first row's (Walk forward).
        rig.click("Change");
        assert_eq!(rig.st.capturing, Some(Action::Forward));
        assert!(placed(&rig.out, CAPTURE_PROMPT).is_some());
        let pass = rig.ctx.cumulative_pass_nr();
        assert!(rig.st.capture_active(pass), "the list is on screen, so the next press is captured");
    }

    #[test]
    fn enter_on_the_reset_confirm_does_not_reset() {
        let mut rig = Rig::new();
        rig.click("Reset all to defaults");
        assert!(placed(&rig.out, "Put every control back to its default?").is_some(), "the confirm is up");
        // Enter, three times over: the confirm's focus is Cancel, so the first
        // closes it; nothing ever resets.
        for _ in 0..3 {
            rig.enter();
            assert_eq!(rig.gfx.key_bindings.get("drop").map(String::as_str), Some("KeyG"), "Enter reset the controls");
            assert!(!rig.changed);
        }
        assert!(placed(&rig.out, "Put every control back to its default?").is_none(), "Enter took Cancel");
        // A real click on Reset does reset.
        rig.click("Reset all to defaults");
        rig.click("Reset");
        assert!(rig.gfx.key_bindings.is_empty());
        assert!(rig.changed);
        assert_eq!(rig.st.notice.as_deref(), Some("Every control is back to its default."));
    }
}

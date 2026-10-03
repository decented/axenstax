//! Sign text editor (Solo Buildout Wave 2c).
//!
//! A small egui dialog opened by right-clicking (or placing) a Sign. It edits
//! the `SignData.text` in place, clamped to `sign::SIGN_MAX_CHARS`. Mirrors the
//! `chest_ui` pattern: pure render returning a small result the caller commits
//! by closing `open_sign`.

use egui::RichText;

use crate::sign::{SignData, SIGN_MAX_CHARS};

#[derive(Default)]
pub struct SignUiResult {
    pub close_requested: bool,
}

/// Render the sign editor. `sign.text` is mutated live as the player types
/// (clamped to the char cap). Returns `close_requested` when the player clicks
/// Done / closes the window.
pub fn show_sign_dialog(
    ctx: &egui::Context,
    sign: &mut SignData,
    pos: (i32, i32, i32),
) -> SignUiResult {
    let mut result = SignUiResult::default();
    let mut window_open = true;

    egui::Window::new(format!("Sign ({}, {}, {})", pos.0, pos.1, pos.2))
        .open(&mut window_open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(RichText::new("Edit sign text").size(16.0).strong());
            ui.add_space(4.0);
            ui.label("Type your message. Enter for a new line.");
            ui.add_space(8.0);

            // Edit a local buffer, then commit through `set_text` so the cap is
            // enforced no matter how the text arrived (typing or paste).
            let mut buf = sign.text.clone();
            let resp = ui.add(
                egui::TextEdit::multiline(&mut buf)
                    .desired_rows(4)
                    .desired_width(220.0)
                    .char_limit(SIGN_MAX_CHARS),
            );
            if resp.changed() {
                sign.set_text(&buf);
            }

            let used = sign.text.chars().count();
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!("{used}/{SIGN_MAX_CHARS}"))
                    .weak()
                    .size(11.0),
            );

            ui.add_space(8.0);
            if ui.button("Done").clicked() {
                result.close_requested = true;
            }
        });

    if !window_open {
        result.close_requested = true;
    }
    result
}

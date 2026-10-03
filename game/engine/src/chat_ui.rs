//! In-game chat overlay (egui). Renders the rolling output log and, when
//! `state.open == true`, the input field with focus.
//!
//! Key handling for opening / closing / history is done in `main.rs` window
//! event loop, not here — that lets us cleanly suppress gameplay input
//! (movement, hotbar) while chat is open without fighting egui's text input.

use crate::commands::{ChatLine, ChatLineKind};

const MAX_LOG_LINES_VISIBLE: usize = 8;
const FADE_TICKS: u64 = 160; // 8s at 20 TPS — match Minecraft's chat fade.
const HISTORY_CAP: usize = 64;

pub struct ChatState {
    pub open: bool,
    pub input: String,
    pub log: Vec<ChatLine>,
    /// Submitted commands, oldest first. Capped at HISTORY_CAP.
    pub history: Vec<String>,
    /// `Some(i)` while navigating with ↑/↓; `None` after a fresh edit.
    pub history_cursor: Option<usize>,
    /// Set true on `open()`; consumed by the draw call to set egui focus once.
    pub focus_pending: bool,
    /// A line acquired out-of-band (the touch Chat button → OS soft keyboard,
    /// which has no egui text field). The next `draw_chat` returns it as
    /// `ChatAction::Submit` so it flows through the same dispatch as a typed line.
    pub pending_submit: Option<String>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            open: false,
            input: String::new(),
            log: Vec::new(),
            history: Vec::new(),
            history_cursor: None,
            focus_pending: false,
            pending_submit: None,
        }
    }

    /// Open with an optional prefix already filled (e.g. `/` for slash-key
    /// open). Cursor sits at end.
    pub fn open_with_prefix(&mut self, prefix: &str) {
        self.open = true;
        self.input.clear();
        self.input.push_str(prefix);
        self.history_cursor = None;
        self.focus_pending = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.input.clear();
        self.history_cursor = None;
        self.focus_pending = false;
    }

    pub fn push_history(&mut self, line: String) {
        if line.is_empty() {
            return;
        }
        // Don't duplicate the most recent entry.
        if self.history.last().map(|s| s.as_str()) == Some(line.as_str()) {
            return;
        }
        self.history.push(line);
        let len = self.history.len();
        if len > HISTORY_CAP {
            self.history.drain(0..(len - HISTORY_CAP));
        }
    }

    /// Move cursor backward (older). Returns the entry to display, if any.
    pub fn history_prev(&mut self) -> Option<&str> {
        if self.history.is_empty() {
            return None;
        }
        let next = match self.history_cursor {
            None => self.history.len().saturating_sub(1),
            Some(0) => 0,
            Some(i) => i.saturating_sub(1),
        };
        self.history_cursor = Some(next);
        self.history.get(next).map(|s| s.as_str())
    }

    /// Move cursor forward (newer). Returns next entry, or None if past the end.
    pub fn history_next(&mut self) -> Option<&str> {
        let cur = self.history_cursor?;
        if cur + 1 >= self.history.len() {
            self.history_cursor = None;
            return None;
        }
        let next = cur + 1;
        self.history_cursor = Some(next);
        self.history.get(next).map(|s| s.as_str())
    }

    pub fn push_log(&mut self, line: ChatLine) {
        self.log.push(line);
        // Cap log growth.
        let len = self.log.len();
        if len > 256 {
            self.log.drain(0..(len - 256));
        }
    }

    pub fn extend_log(&mut self, lines: impl IntoIterator<Item = ChatLine>) {
        for line in lines {
            self.push_log(line);
        }
    }
}

#[derive(Clone, Debug)]
pub enum ChatAction {
    None,
    Submit(String),
    Cancel,
}

fn line_color(kind: ChatLineKind) -> egui::Color32 {
    match kind {
        ChatLineKind::Info => egui::Color32::from_rgb(196, 196, 212),
        ChatLineKind::Echo => egui::Color32::from_rgb(150, 150, 165),
        ChatLineKind::Success => egui::Color32::from_rgb(140, 220, 140),
        ChatLineKind::Error => egui::Color32::from_rgb(232, 120, 120),
        ChatLineKind::System => egui::Color32::from_rgb(212, 160, 68),
        // World chat (Phase 2). Player: ordinary readable text — this is
        // what most chat looks like, so it stays close to white/Info rather
        // than drawing attention to itself.
        ChatLineKind::Player => egui::Color32::from_rgb(225, 225, 225),
        // Room: a cool tint so a line relayed from the attached room (a
        // guardian's phone, via KithMoot) visibly reads as from outside the
        // world, not from another player in-game.
        ChatLineKind::Room => egui::Color32::from_rgb(140, 190, 220),
    }
}

/// Draw the chat overlay (log + optional input). Returns the next action.
pub fn draw_chat(
    ctx: &egui::Context,
    state: &mut ChatState,
    current_tick: u64,
) -> ChatAction {
    let mut action = ChatAction::None;

    // A line acquired out-of-band (touch Chat button → OS soft keyboard) is
    // submitted on the next frame, before any egui work, so it runs through the
    // exact same dispatch as a typed line.
    if let Some(line) = state.pending_submit.take() {
        return ChatAction::Submit(line);
    }

    // Visible-line filter: keep lines submitted within FADE_TICKS, OR all of
    // them if chat is open (so the user can read what they missed). Clone
    // out so we don't hold an immutable borrow on `state.log` into the
    // closure — the input field below needs `&mut state`.
    let mut visible: Vec<ChatLine> = state
        .log
        .iter()
        .filter(|l| state.open || current_tick.saturating_sub(l.at_tick) <= FADE_TICKS)
        .cloned()
        .collect();
    let total = visible.len();
    if total > MAX_LOG_LINES_VISIBLE {
        visible.drain(0..(total - MAX_LOG_LINES_VISIBLE));
    }

    if visible.is_empty() && !state.open {
        return action;
    }

    egui::Area::new(egui::Id::new("engine_chat_overlay"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -120.0))
        .order(egui::Order::Foreground)
        // Task 16 — the log lingers for FADE_TICKS (~8 s) after chat closes;
        // during that fade the Area must not hit-test, or a cursor resting
        // bottom-left trips the `wants_pointer_input()` gameplay gate. The
        // input field only exists while open, so open ⇔ interactable keeps
        // typing working (chat.open already gates gameplay input anyway).
        .interactable(state.open)
        .show(ctx, |ui| {
            ui.set_max_width(560.0);
            // Output log
            for line in &visible {
                let alpha = if state.open {
                    255
                } else {
                    let age = current_tick.saturating_sub(line.at_tick);
                    let remaining = FADE_TICKS.saturating_sub(age);
                    ((remaining as f32 / FADE_TICKS as f32) * 255.0).clamp(40.0, 255.0) as u8
                };
                let mut col = line_color(line.kind);
                col = egui::Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), alpha);
                let bg = egui::Color32::from_rgba_unmultiplied(15, 18, 28, alpha.saturating_sub(30));
                let frame = egui::Frame::new()
                    .fill(bg)
                    .inner_margin(egui::Margin::symmetric(8, 2));
                frame.show(ui, |ui| {
                    ui.label(egui::RichText::new(&line.text).color(col).monospace());
                });
            }

            // Input field (only when open)
            if state.open {
                ui.add_space(4.0);
                let frame = egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(8, 10, 18, 240))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(60, 70, 100)))
                    .inner_margin(egui::Margin::symmetric(10, 6));
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(">")
                                .color(egui::Color32::from_rgb(212, 160, 68))
                                .monospace(),
                        );
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut state.input)
                                .desired_width(520.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        if state.focus_pending {
                            resp.request_focus();
                            state.focus_pending = false;
                        }
                        // Enter submits.
                        if resp.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            let line = std::mem::take(&mut state.input);
                            action = if line.trim().is_empty() {
                                ChatAction::Cancel
                            } else {
                                ChatAction::Submit(line)
                            };
                        }
                        // Esc cancels.
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            action = ChatAction::Cancel;
                        }
                        // History navigation.
                        if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::ArrowUp))
                            && let Some(s) = state.history_prev() {
                                state.input = s.to_string();
                            }
                        if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                            match state.history_next() {
                                Some(s) => state.input = s.to_string(),
                                None => state.input.clear(),
                            }
                        }
                    });
                });
            }
        });

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_with_prefix_sets_input() {
        let mut s = ChatState::new();
        s.open_with_prefix("/");
        assert!(s.open);
        assert_eq!(s.input, "/");
        assert!(s.focus_pending);
    }

    #[test]
    fn close_resets_state() {
        let mut s = ChatState::new();
        s.open_with_prefix("/he");
        s.close();
        assert!(!s.open);
        assert!(s.input.is_empty());
        assert!(!s.focus_pending);
    }

    #[test]
    fn history_dedupes_consecutive() {
        let mut s = ChatState::new();
        s.push_history("/help".to_string());
        s.push_history("/help".to_string());
        s.push_history("/time get".to_string());
        s.push_history("/time get".to_string());
        assert_eq!(s.history.len(), 2);
    }

    #[test]
    fn history_caps_at_limit() {
        let mut s = ChatState::new();
        for i in 0..(HISTORY_CAP + 10) {
            s.push_history(format!("/cmd {i}"));
        }
        assert_eq!(s.history.len(), HISTORY_CAP);
        // Oldest entries dropped.
        assert!(s.history[0].contains("10"));
    }

    #[test]
    fn history_prev_walks_backward() {
        let mut s = ChatState::new();
        s.push_history("a".to_string());
        s.push_history("b".to_string());
        s.push_history("c".to_string());
        assert_eq!(s.history_prev(), Some("c"));
        assert_eq!(s.history_prev(), Some("b"));
        assert_eq!(s.history_prev(), Some("a"));
        assert_eq!(s.history_prev(), Some("a")); // saturates at 0
    }

    #[test]
    fn history_next_after_prev() {
        let mut s = ChatState::new();
        s.push_history("a".to_string());
        s.push_history("b".to_string());
        s.push_history("c".to_string());
        s.history_prev(); // c
        s.history_prev(); // b
        assert_eq!(s.history_next(), Some("c"));
        assert_eq!(s.history_next(), None); // past end clears cursor
    }

    #[test]
    fn empty_history_navigation_safe() {
        let mut s = ChatState::new();
        assert_eq!(s.history_prev(), None);
        assert_eq!(s.history_next(), None);
    }

    #[test]
    fn pending_submit_returns_submit_and_clears() {
        // The touch Chat button stages a line in `pending_submit`; the next
        // draw_chat must return it as Submit (before any egui frame work, so a
        // default Context is safe) and clear the slot so it fires exactly once.
        let ctx = egui::Context::default();
        let mut s = ChatState::new();
        s.pending_submit = Some("/time set day".to_string());
        match draw_chat(&ctx, &mut s, 0) {
            ChatAction::Submit(line) => assert_eq!(line, "/time set day"),
            other => panic!("expected Submit, got {other:?}"),
        }
        assert!(s.pending_submit.is_none(), "pending_submit consumed once");
    }

    #[test]
    fn push_log_caps_growth() {
        let mut s = ChatState::new();
        for i in 0..300 {
            s.push_log(ChatLine::info(format!("line {i}"), 0));
        }
        assert!(s.log.len() <= 256);
    }
}

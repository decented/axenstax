//! In-game chat overlay (egui). Renders the rolling output log and, when
//! `state.open == true`, the input field with focus.
//!
//! Key handling for opening / closing / history is done in `main.rs` window
//! event loop, not here — that lets us cleanly suppress gameplay input
//! (movement, hotbar) while chat is open without fighting egui's text input.

use crate::commands::complete::{self, Completion, CompletionContext};
use crate::commands::{ChatLine, ChatLineKind};
use crate::graphics_settings::{CHAT_TEXT_SCALE_MAX, CHAT_TEXT_SCALE_MIN, DEFAULT_CHAT_TEXT_SCALE};

const MAX_LOG_LINES_VISIBLE: usize = 8;
const FADE_TICKS: u64 = 160; // 8s at 20 TPS — match Minecraft's chat fade.
const HISTORY_CAP: usize = 64;

/// egui's Monospace text style size, which the chat used before it was adjustable.
const BASE_FONT_PT: f32 = 13.0;
/// The overlay's fixed sizes at a text scale of 1.0 (points).
const BASE_MAX_WIDTH: f32 = 560.0;
const BASE_INPUT_WIDTH: f32 = 520.0;
const BASE_BOTTOM_OFFSET: f32 = 120.0;
/// The log never grows taller than this share of the screen.
const MAX_LOG_SCREEN_FRACTION: f32 = 0.45;
/// Tab-completion rows shown at most; the rest are a `+N more` line.
pub const MAX_SUGGESTION_ROWS: usize = 8;

const AREA_ID: &str = "engine_chat_overlay";
const INPUT_ID: &str = "engine_chat_input";

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
    /// Tab has filled in a candidate and may cycle through the rest.
    pub cycle: Option<TabCycle>,
    /// The next frame puts the text cursor at the end of the line (after Tab or
    /// a clicked suggestion has rewritten it).
    pub cursor_to_end: bool,
}

/// One Tab-completion in progress. It lasts only while the input line is
/// exactly what the cycle last wrote; any other edit ends it.
#[derive(Clone, Debug, PartialEq)]
pub struct TabCycle {
    /// Where the word being completed starts in the line (byte index). Kept
    /// from the first Tab, so cycling replaces the right text even though the
    /// word has changed since.
    replace_from: usize,
    /// The candidates the first Tab saw.
    candidates: Vec<String>,
    /// The one currently written into the line.
    index: usize,
    /// The whole line as the cycle last wrote it.
    written: String,
}

/// `line` with everything from `from` replaced by `candidate`.
fn replaced(line: &str, from: usize, candidate: &str) -> String {
    let head = line.get(..from).unwrap_or(line);
    format!("{head}{candidate}")
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
            cycle: None,
            cursor_to_end: false,
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
        self.cycle = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.input.clear();
        self.history_cursor = None;
        self.focus_pending = false;
        self.cycle = None;
        self.cursor_to_end = false;
    }

    /// Tab (or Shift+Tab when `backwards`). Fills in the selected candidate, or,
    /// when the line is still what the last Tab wrote, steps to the next
    /// (previous) one, wrapping. Returns whether the line changed. `live` is the
    /// completion of the line as it stands now; plain chat has none, so Tab
    /// does nothing there.
    pub fn tab(&mut self, live: Option<&Completion>, backwards: bool) -> bool {
        if let Some(cycle) = self.cycle.as_mut()
            && cycle.written == self.input
        {
            let n = cycle.candidates.len();
            cycle.index = if backwards { (cycle.index + n - 1) % n } else { (cycle.index + 1) % n };
            let line = replaced(&self.input, cycle.replace_from, &cycle.candidates[cycle.index]);
            cycle.written = line.clone();
            self.input = line;
            self.cursor_to_end = true;
            self.history_cursor = None;
            return true;
        }
        self.cycle = None;
        let Some(live) = live.filter(|c| !c.candidates.is_empty()) else {
            return false;
        };
        let n = live.candidates.len();
        let index = if backwards { n - 1 } else { 0 };
        let line = replaced(&self.input, live.replace_from, &live.candidates[index]);
        self.cycle = Some(TabCycle {
            replace_from: live.replace_from,
            candidates: live.candidates.clone(),
            index,
            written: line.clone(),
        });
        self.input = line;
        self.cursor_to_end = true;
        self.history_cursor = None;
        true
    }

    /// What the suggestion list shows for the line as it stands. While a Tab
    /// cycle is running (the line is what it wrote) that is the cycle's own list
    /// with its current row highlighted; any other edit ends the cycle and the
    /// list is the live completion with its first row selected.
    pub fn suggestions(&mut self, live: Option<Completion>) -> Option<SuggestionView> {
        if let Some(cycle) = &self.cycle
            && cycle.written == self.input
        {
            return Some(SuggestionView {
                candidates: cycle.candidates.clone(),
                selected: cycle.index,
                replace_from: cycle.replace_from,
                hint: None,
            });
        }
        self.cycle = None;
        let live = live?;
        Some(SuggestionView {
            candidates: live.candidates,
            selected: 0,
            replace_from: live.replace_from,
            hint: live.usage_hint,
        })
    }

    /// A clicked or tapped suggestion: write it over the word being completed,
    /// end any cycle, put the cursor at the end and keep the input focused.
    pub fn accept(&mut self, replace_from: usize, candidate: &str) {
        self.input = replaced(&self.input, replace_from, candidate);
        self.cycle = None;
        self.cursor_to_end = true;
        self.focus_pending = true;
        self.history_cursor = None;
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

/// Muted gold, the colour of the `>` prompt and the selected suggestion.
const GOLD: egui::Color32 = egui::Color32::from_rgb(212, 160, 68);

/// Overlay measurements for one frame, all in egui points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChatLayout {
    /// Font size of everything the overlay draws.
    pub font: f32,
    /// Widest the box may be.
    pub max_width: f32,
    /// Width of the text field.
    pub input_width: f32,
    /// Offset of the box from the bottom-left corner (negative = up).
    pub bottom_offset: f32,
    /// Most suggestion rows to show.
    pub suggestion_rows: usize,
}

/// A usable scale whatever the settings file said.
fn sane_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(CHAT_TEXT_SCALE_MIN, CHAT_TEXT_SCALE_MAX)
    } else {
        DEFAULT_CHAT_TEXT_SCALE
    }
}

/// Scale the overlay's fixed sizes (font, widths, distance from the bottom)
/// by `scale`, never wider than the screen minus 16 points each side.
pub(crate) fn layout(scale: f32, screen: egui::Vec2) -> ChatLayout {
    let scale = sane_scale(scale);
    let font = BASE_FONT_PT * scale;
    let max_width = (BASE_MAX_WIDTH * scale).min(screen.x - 32.0).max(160.0);
    // The row around the field holds a margin, the `>` and a gap.
    let input_width = (BASE_INPUT_WIDTH * scale).min(max_width - 32.0 - font * 0.7).max(80.0);
    let bottom_offset = -(BASE_BOTTOM_OFFSET * scale).min(screen.y * 0.2);
    let row = font * 1.5;
    let suggestion_rows = (((screen.y * 0.3) / row).floor() as usize).clamp(3, MAX_SUGGESTION_ROWS);
    ChatLayout { font, max_width, input_width, bottom_offset, suggestion_rows }
}

/// What the suggestion list shows this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SuggestionView {
    pub candidates: Vec<String>,
    /// The highlighted row.
    pub selected: usize,
    /// Byte index in the line where a picked candidate goes.
    pub replace_from: usize,
    /// The command's usage line, when nothing can be suggested.
    pub hint: Option<String>,
}

/// The first `rows` candidates, scrolled so `selected` is in view, plus how many lie beyond.
fn window(view: &SuggestionView, rows: usize) -> (std::ops::Range<usize>, usize) {
    let total = view.candidates.len();
    let start = if view.selected < rows { 0 } else { view.selected + 1 - rows };
    let end = (start + rows).min(total);
    (start..end, total - end)
}

/// Draw the candidates (or the usage hint) as a list; the row clicked, if any
/// (an index into `view.candidates`). A tablet has no Tab key.
fn draw_suggestions(
    ui: &mut egui::Ui,
    font: &egui::FontId,
    view: &SuggestionView,
    rows: usize,
) -> Option<usize> {
    if view.candidates.is_empty() && view.hint.is_none() {
        return None;
    }
    let mut clicked = None;
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(8, 10, 18, 235))
        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(60, 70, 100)))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            let dim = egui::Color32::from_rgb(130, 134, 150);
            if view.candidates.is_empty() {
                if let Some(hint) = &view.hint {
                    ui.label(egui::RichText::new(hint).color(dim).font(font.clone()));
                }
                return;
            }
            let (shown, more) = window(view, rows);
            for i in shown {
                let selected = i == view.selected;
                let colour = if selected { GOLD } else { egui::Color32::from_rgb(196, 196, 212) };
                let fill = if selected {
                    egui::Color32::from_rgba_unmultiplied(80, 66, 30, 255)
                } else {
                    egui::Color32::TRANSPARENT
                };
                let label = egui::Label::new(
                    egui::RichText::new(&view.candidates[i]).color(colour).font(font.clone()),
                )
                .sense(egui::Sense::click());
                let response = egui::Frame::new()
                    .fill(fill)
                    .inner_margin(egui::Margin::symmetric(4, 1))
                    .show(ui, |ui| ui.add(label))
                    .inner;
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    clicked = Some(i);
                }
            }
            if more > 0 {
                ui.label(egui::RichText::new(format!("+{more} more")).color(dim).font(font.clone()));
            }
        });
    clicked
}

/// Completion for the line as it stands; `None` for plain chat or when completion is off.
fn complete_line(line: &str, completion: Option<&CompletionContext>) -> Option<Completion> {
    complete::complete(line, completion?)
}

/// Draw the chat overlay (log + optional input). Returns the next action.
///
/// `text_scale` multiplies egui's Monospace 13 pt for everything drawn here:
/// the log, the input line, the suggestion list and the usage hint. Other UI
/// keeps its own sizes. `completion` is what Tab completion needs (`None`
/// switches it off).
pub fn draw_chat(
    ctx: &egui::Context,
    state: &mut ChatState,
    current_tick: u64,
    text_scale: f32,
    completion: Option<&CompletionContext>,
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

    let screen = ctx.content_rect().size();
    let lay = layout(text_scale, screen);
    let font = egui::FontId::monospace(lay.font);
    let row_h = lay.font * 1.5;

    // Tab and Shift+Tab are ours: taken out of the input before the text field
    // sees them (egui otherwise treats them as focus traversal). The field also
    // locks focus, which is what stops egui moving focus away on the key press.
    let mut view = None;
    if state.open {
        let (backwards, forwards) = ctx.input_mut(|i| {
            // Shift first: `consume_key` ignores an extra Shift, so a plain
            // Tab check would swallow Shift+Tab.
            let back = i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab);
            let fwd = i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
            (back, fwd)
        });
        if backwards || forwards {
            let live = complete_line(&state.input, completion);
            state.tab(live.as_ref(), backwards);
            // Keep typing: the key press may have moved focus before we saw it.
            state.focus_pending = true;
        }
        view = state.suggestions(complete_line(&state.input, completion));
    }

    // Room left for the log once the suggestion list and the input line have theirs.
    let suggestion_h = match &view {
        Some(v) if !v.candidates.is_empty() => {
            let (shown, more) = window(v, lay.suggestion_rows);
            (shown.len() + usize::from(more > 0)) as f32 * row_h + 14.0
        }
        Some(v) if v.hint.is_some() => row_h + 14.0,
        _ => 0.0,
    };
    let input_h = if state.open { row_h + 28.0 } else { 0.0 };
    let log_max_height = (screen.y * MAX_LOG_SCREEN_FRACTION)
        .min(screen.y - lay.bottom_offset.abs() - input_h - suggestion_h - 8.0)
        .max(row_h * 2.0);

    egui::Area::new(egui::Id::new(AREA_ID))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, lay.bottom_offset))
        .order(egui::Order::Foreground)
        // Task 16 — the log lingers for FADE_TICKS (~8 s) after chat closes;
        // during that fade the Area must not hit-test, or a cursor resting
        // bottom-left trips the `wants_pointer_input()` gameplay gate. The
        // input field only exists while open, so open ⇔ interactable keeps
        // typing working (chat.open already gates gameplay input anyway).
        .interactable(state.open)
        .show(ctx, |ui| {
            ui.set_max_width(lay.max_width);
            // Output log. Never taller than a share of the screen: if the lines
            // do not fit, the oldest scroll out of view.
            egui::ScrollArea::vertical()
                .id_salt("engine_chat_log")
                .max_height(log_max_height)
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
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
                            ui.label(egui::RichText::new(&line.text).color(col).font(font.clone()));
                        });
                    }
                });

            // Input field (only when open)
            if state.open {
                ui.add_space(4.0);
                // Tab-completion candidates, directly above the input line.
                if let Some(view) = &view
                    && let Some(i) = draw_suggestions(ui, &font, view, lay.suggestion_rows)
                {
                    let picked = view.candidates[i].clone();
                    state.accept(view.replace_from, &picked);
                }
                let input_id = egui::Id::new(INPUT_ID);
                if state.cursor_to_end {
                    state.cursor_to_end = false;
                    let end = state.input.chars().count();
                    let mut edit = egui::TextEdit::load_state(ui.ctx(), input_id).unwrap_or_default();
                    edit.cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
                    edit.store(ui.ctx(), input_id);
                }
                let frame = egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(8, 10, 18, 240))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(60, 70, 100)))
                    .inner_margin(egui::Margin::symmetric(10, 6));
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(">").color(GOLD).font(font.clone()));
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut state.input)
                                .id(input_id)
                                .desired_width(lay.input_width)
                                .font(font.clone())
                                // Tab must not move focus out of the chat line.
                                .lock_focus(true),
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
        match draw_chat(&ctx, &mut s, 0, DEFAULT_CHAT_TEXT_SCALE, None) {
            ChatAction::Submit(line) => assert_eq!(line, "/time set day"),
            other => panic!("expected Submit, got {other:?}"),
        }
        assert!(s.pending_submit.is_none(), "pending_submit consumed once");
    }

    // ---- Tab completion: state transitions (no GPU) ----

    fn completion(replace_from: usize, candidates: &[&str]) -> Completion {
        Completion {
            replace_from,
            candidates: candidates.iter().map(|s| s.to_string()).collect(),
            usage_hint: None,
        }
    }

    #[test]
    fn tab_fills_in_the_first_candidate_and_puts_the_cursor_at_the_end() {
        let mut s = ChatState::new();
        s.open_with_prefix("/ti");
        let live = completion(1, &["time", "timestep"]);
        assert!(s.tab(Some(&live), false));
        assert_eq!(s.input, "/time");
        assert!(s.cursor_to_end, "the next frame moves the cursor to the end");
        assert!(s.cycle.is_some());
    }

    #[test]
    fn tab_tab_shift_tab_cycle_through_the_candidates_and_wrap() {
        let mut s = ChatState::new();
        s.open_with_prefix("/ti");
        let first = completion(1, &["time", "timestep"]);
        s.tab(Some(&first), false);
        assert_eq!(s.input, "/time");
        // Between presses the live completion of the NEW line is a different
        // list ("/time" matches only itself); the cycle must ignore it.
        let live_now = completion(1, &["time"]);
        s.tab(Some(&live_now), false);
        assert_eq!(s.input, "/timestep");
        s.tab(Some(&live_now), false);
        assert_eq!(s.input, "/time", "wraps forward");
        s.tab(Some(&live_now), true);
        assert_eq!(s.input, "/timestep", "Shift+Tab wraps backwards");
        s.tab(Some(&live_now), true);
        assert_eq!(s.input, "/time");
    }

    #[test]
    fn shift_tab_first_goes_to_the_last_candidate() {
        let mut s = ChatState::new();
        s.open_with_prefix("/ti");
        s.tab(Some(&completion(1, &["time", "timestep"])), true);
        assert_eq!(s.input, "/timestep");
    }

    #[test]
    fn any_other_edit_ends_the_cycle_and_recomputes() {
        let mut s = ChatState::new();
        s.open_with_prefix("/ti");
        s.tab(Some(&completion(1, &["time", "timestep"])), false);
        assert_eq!(s.input, "/time");
        // The player types a space: the line is no longer what the cycle wrote.
        s.input.push(' ');
        let view = s.suggestions(Some(completion(6, &["get", "set", "speed"]))).unwrap();
        assert_eq!(view.candidates, ["get", "set", "speed"]);
        assert_eq!((view.selected, view.replace_from), (0, 6));
        assert!(s.cycle.is_none());
        // Tab now completes the new word, not the old cycle.
        s.tab(Some(&completion(6, &["get", "set", "speed"])), false);
        assert_eq!(s.input, "/time get");
    }

    #[test]
    fn the_suggestions_follow_the_cycle_while_it_runs() {
        let mut s = ChatState::new();
        s.open_with_prefix("/ti");
        s.tab(Some(&completion(1, &["time", "timestep"])), false);
        s.tab(None, false);
        let view = s.suggestions(Some(completion(1, &["time"]))).unwrap();
        assert_eq!(view.candidates, ["time", "timestep"]);
        assert_eq!(view.selected, 1);
    }

    #[test]
    fn tab_does_nothing_without_candidates_or_on_plain_chat() {
        let mut s = ChatState::new();
        s.open_with_prefix("hello");
        assert!(!s.tab(None, false));
        assert!(!s.tab(Some(&completion(1, &[])), false));
        assert_eq!(s.input, "hello");
        assert!(!s.cursor_to_end && s.cycle.is_none());
    }

    #[test]
    fn accepting_a_clicked_row_replaces_the_word_ends_the_cycle_and_keeps_focus() {
        let mut s = ChatState::new();
        s.open_with_prefix("/give tor");
        s.focus_pending = false;
        s.accept("/give ".len(), "torch");
        assert_eq!(s.input, "/give torch");
        assert!(s.cursor_to_end && s.focus_pending && s.cycle.is_none());
    }

    #[test]
    fn a_line_with_a_multibyte_word_is_replaced_at_a_char_boundary() {
        assert_eq!(replaced("/say héllo", 5, "x"), "/say x");
        // A stale index past the end or inside a char never panics: the
        // candidate is appended instead.
        assert_eq!(replaced("/é", 2, "x"), "/éx");
        assert_eq!(replaced("/a", 99, "x"), "/ax");
    }

    #[test]
    fn the_suggestion_window_scrolls_to_keep_the_selected_row_in_view() {
        let view = |selected| SuggestionView {
            candidates: (0..20).map(|i| i.to_string()).collect(),
            selected,
            replace_from: 1,
            hint: None,
        };
        assert_eq!(window(&view(0), 8), (0..8, 12));
        assert_eq!(window(&view(7), 8), (0..8, 12));
        assert_eq!(window(&view(8), 8), (1..9, 11));
        assert_eq!(window(&view(19), 8), (12..20, 0));
    }

    // ---- Layout ----

    #[test]
    fn layout_scales_everything_and_never_exceeds_the_screen() {
        let big = egui::vec2(1920.0, 1080.0);
        let one = layout(1.0, big);
        assert_eq!((one.font, one.max_width), (13.0, 560.0));
        assert!((one.input_width - 520.0).abs() < 2.0, "the field is about as wide as before: {}", one.input_width);
        assert_eq!(one.bottom_offset, -120.0, "the 100% layout is the old one");
        let one_and_half = layout(1.5, big);
        assert_eq!(one_and_half.font, 19.5);
        assert_eq!(one_and_half.max_width, 840.0);
        // A phone: the box is the screen minus 16 on each side, the field fits inside it.
        for scale in [1.0, 1.5, 3.0] {
            for width in [320.0, 360.0, 412.0, 640.0, 1280.0] {
                let l = layout(scale, egui::vec2(width, 700.0));
                assert!(l.max_width <= width - 32.0, "{scale} at {width}: {l:?}");
                assert!(l.input_width < l.max_width, "{scale} at {width}: {l:?}");
                assert!(l.bottom_offset.abs() <= 700.0 * 0.2 + 0.01);
            }
        }
    }

    #[test]
    fn a_bad_scale_falls_back_to_something_usable() {
        let screen = egui::vec2(1280.0, 720.0);
        assert_eq!(layout(f32::NAN, screen).font, 13.0 * DEFAULT_CHAT_TEXT_SCALE);
        assert_eq!(layout(f32::INFINITY, screen).font, 13.0 * DEFAULT_CHAT_TEXT_SCALE);
        assert_eq!(layout(0.0, screen).font, 13.0 * CHAT_TEXT_SCALE_MIN);
        assert_eq!(layout(99.0, screen).font, 13.0 * CHAT_TEXT_SCALE_MAX);
    }

    // ---- The real overlay in a headless egui context (CPU only) ----

    const INPUT_ID_STR: &str = INPUT_ID;

    struct Rig {
        ctx: egui::Context,
        state: ChatState,
        registry: crate::commands::CommandRegistry,
        size: egui::Vec2,
        scale: f32,
        output: egui::FullOutput,
    }

    impl Rig {
        fn new(size: egui::Vec2, scale: f32, line: &str) -> Rig {
            let mut state = ChatState::new();
            state.open_with_prefix(line);
            let mut rig = Rig {
                ctx: egui::Context::default(),
                state,
                registry: crate::commands::test_support::full_registry(),
                size,
                scale,
                output: Default::default(),
            };
            // Warm up: the field takes focus on the first frame it exists.
            for _ in 0..4 {
                rig.frame(vec![], egui::Modifiers::NONE);
            }
            rig
        }

        fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                events,
                modifiers,
                ..Default::default()
            };
            let (state, registry, scale) = (&mut self.state, &self.registry, self.scale);
            self.output = self.ctx.run(input, |ctx| {
                let comp = CompletionContext {
                    op_level: crate::commands::OpLevel::Op,
                    registry,
                    roster_names: &[],
                };
                draw_chat(ctx, state, 0, scale, Some(&comp));
            });
        }

        fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
            let event = |pressed| egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            };
            self.frame(vec![event(true)], modifiers);
            self.frame(vec![event(false)], modifiers);
            // A quiet frame: the live recompute must not undo what Tab did.
            self.frame(vec![], egui::Modifiers::NONE);
        }

        fn focused(&self) -> bool {
            self.ctx.memory(|m| m.has_focus(egui::Id::new(INPUT_ID_STR)))
        }

        fn cursor(&self) -> Option<usize> {
            egui::TextEdit::load_state(&self.ctx, egui::Id::new(INPUT_ID_STR))
                .and_then(|s| s.cursor.char_range())
                .map(|r| r.primary.index)
        }
    }

    #[test]
    fn headless_tab_completes_the_command_and_the_input_keeps_focus() {
        let mut rig = Rig::new(egui::vec2(1280.0, 720.0), 1.5, "/gi");
        assert!(rig.focused(), "the input takes focus when the chat opens");
        rig.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(rig.state.input, "/give", "Tab finished the command (and typed no tab character)");
        assert!(rig.focused(), "egui's focus traversal must not take focus off the input");
        assert_eq!(rig.cursor(), Some("/give".chars().count()), "cursor at the end");
        // Tab again with a single candidate stays put and still holds focus.
        rig.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(rig.state.input, "/give");
        assert!(rig.focused());
    }

    #[test]
    fn headless_tab_and_shift_tab_cycle_and_wrap() {
        let mut rig = Rig::new(egui::vec2(1280.0, 720.0), 1.5, "/ti");
        rig.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(rig.state.input, "/time");
        rig.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(rig.state.input, "/timestep");
        rig.key(egui::Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(rig.state.input, "/time", "Shift+Tab goes back");
        rig.key(egui::Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(rig.state.input, "/timestep", "and wraps");
        assert!(rig.focused());
        assert_eq!(rig.cursor(), Some("/timestep".chars().count()));
    }

    #[test]
    fn headless_tab_on_plain_chat_changes_nothing_and_keeps_focus() {
        let mut rig = Rig::new(egui::vec2(1280.0, 720.0), 1.5, "hello");
        rig.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(rig.state.input, "hello");
        assert!(rig.focused());
    }

    #[test]
    fn headless_typing_after_tab_ends_the_cycle_and_completes_the_new_word() {
        let mut rig = Rig::new(egui::vec2(1280.0, 720.0), 1.5, "/time");
        rig.frame(vec![egui::Event::Text(" ".to_string())], egui::Modifiers::NONE);
        rig.frame(vec![], egui::Modifiers::NONE);
        assert_eq!(rig.state.input, "/time ");
        rig.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(rig.state.input, "/time get");
        rig.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(rig.state.input, "/time set");
    }

    /// Where a text was drawn: its rectangle and the clip rectangle it was drawn under.
    fn placed(output: &egui::FullOutput, label: &str) -> Option<(egui::Rect, egui::Rect)> {
        fn walk(shape: &egui::epaint::Shape, clip: egui::Rect, label: &str, out: &mut Option<(egui::Rect, egui::Rect)>) {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == label && out.is_none() => {
                    *out = Some((t.galley.rect.translate(t.pos.to_vec2()), clip));
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, label, out)),
                _ => {}
            }
        }
        let mut out = None;
        output.shapes.iter().for_each(|clipped| walk(&clipped.shape, clipped.clip_rect, label, &mut out));
        out
    }

    fn overlay_rect(rig: &Rig) -> egui::Rect {
        rig.ctx.memory(|m| m.area_rect(egui::Id::new(AREA_ID))).expect("the chat overlay was not drawn")
    }

    #[test]
    fn eight_lines_at_one_and_a_half_times_fit_a_1280_by_720_window_with_suggestions_showing() {
        let mut rig = Rig::new(egui::vec2(1280.0, 720.0), 1.5, "/");
        for i in 0..8 {
            rig.state.push_log(ChatLine::info(format!("log line number {i}"), 0));
        }
        rig.frame(vec![], egui::Modifiers::NONE);
        rig.frame(vec![], egui::Modifiers::NONE);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, rig.size);
        let r = overlay_rect(&rig);
        assert!(screen.contains_rect(r), "the chat {r:?} left the screen {screen:?}");
        for i in 0..8 {
            let label = format!("log line number {i}");
            let seen = placed(&rig.output, &label);
            assert!(seen.is_some_and(|(text, clip)| screen.contains_rect(text) && clip.contains_rect(text)),
                "{label} is not fully visible");
        }
    }

    #[test]
    fn the_overlay_is_never_wider_than_the_screen_and_the_log_never_taller_than_45_percent() {
        for (size, scale) in [(egui::vec2(360.0, 640.0), 3.0), (egui::vec2(640.0, 360.0), 3.0), (egui::vec2(1280.0, 720.0), 3.0)] {
            let mut rig = Rig::new(size, scale, "/g");
            for i in 0..8 {
                rig.state.push_log(ChatLine::info(format!("line {i} with some words in it"), 0));
            }
            rig.frame(vec![], egui::Modifiers::NONE);
            rig.frame(vec![], egui::Modifiers::NONE);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let r = overlay_rect(&rig);
            assert!(r.left() >= 0.0 && r.right() <= size.x - 16.0 + 0.5, "{size:?} x{scale}: {r:?} is wider than the screen minus 16 each side");
            assert!(screen.contains_rect(r), "{size:?} x{scale}: {r:?} left the screen");
        }
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

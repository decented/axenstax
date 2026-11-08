//! The lobby's pages (home page overhaul spec §2-§5, §7): which page shows,
//! the TV-safe area they draw in, Escape stepping back, the keyboard focus
//! seed, and the buttons and headers the pages share.
//!
//! `menu::draw_main_menu` draws the notice, the dialogs and the Trials gallery
//! first; when none of them owns the screen it calls [`draw`]. Home is
//! `home_ui`, Your worlds is `worlds_ui` (+ `world_detail_ui`). Play together
//! and More to play host the existing Friends column and official-games
//! column under a `‹ Back` header for now.

use crate::brand;
use crate::menu::{LobbyPage, MenuAction, MenuState};
use crate::ui_safe_area::{self, Metrics};

/// Whether "Host for friends online" can be pressed this frame, and the line
/// under it (read once per frame by `draw_main_menu`).
#[derive(Clone, Copy)]
pub(crate) struct HostOnline {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub enabled: bool,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub note: Option<&'static str>,
}

/// What a page needs to know about this frame's keyboard.
#[derive(Clone, Copy)]
pub(crate) struct Keys {
    /// An arrow key was pressed with nothing focused: the page focuses its
    /// first item, so the arrows have somewhere to start from.
    pub seed_focus: bool,
}

/// Draw the current lobby page for one frame.
pub(crate) fn draw(ctx: &egui::Context, state: &mut MenuState, host: HostOnline) -> MenuAction {
    let m = Metrics::of(ctx);

    // Escape (and a pad's B) on an inner page goes back one page and never
    // quits: `lib.rs` leaves every page but a bare Home to this. A menu that
    // was open last frame takes the Escape itself (it closes).
    let spent = std::mem::take(&mut state.escape_spent);
    let popup_was_open = egui::Popup::is_any_open(ctx);
    if state.page != LobbyPage::Home
        && !popup_was_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        && !spent
    {
        state.step_back();
    }

    let nothing_focused = ctx.memory(|mem| mem.focused().is_none());
    let arrow = ctx.input(|i| {
        [egui::Key::ArrowUp, egui::Key::ArrowDown, egui::Key::ArrowLeft, egui::Key::ArrowRight]
            .iter()
            .any(|k| i.key_pressed(*k))
    });
    let keys = Keys { seed_focus: nothing_focused && arrow };

    let mut action = MenuAction::None;
    #[allow(deprecated)]
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(brand::DEEP_FRONTIER))
        .show(ctx, |ui| {
            // Labels never take a click meant for the card under them, and a
            // scroll bar is always drawn where the page can scroll (kids miss
            // a bar that only appears on hover: 2026-05-21 playtest).
            ui.style_mut().interaction.selectable_labels = false;
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
            ui_safe_area::inside(ui, |ui| {
                action = match state.page {
                    LobbyPage::Home => crate::home_ui::draw(ui, state, &m, keys),
                    LobbyPage::Worlds => crate::worlds_ui::draw(ui, state, &m, host, keys),
                    LobbyPage::PlayTogether => draw_play_together(ui, state, &m),
                    LobbyPage::Stash => draw_more_to_play(ui, state, &m),
                };
            });
        });
    state.popup_open = egui::Popup::is_any_open(ctx);
    ui_safe_area::paint_focus_ring(ctx);
    action
}

// ---------------------------------------------------------------------------
// Shared pieces
// ---------------------------------------------------------------------------

/// A lobby button's look.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The one main thing on a page (green).
    Primary,
    /// Everything else.
    Secondary,
}

/// A lobby button at least `HIT` tall, styled by `kind`.
pub(crate) fn button<'a>(m: &Metrics, label: impl Into<String>, kind: Kind) -> egui::Button<'a> {
    let (text, fill, stroke) = match kind {
        Kind::Primary => (
            egui::Color32::from_rgb(214, 244, 214),
            egui::Color32::from_rgb(45, 107, 45),
            egui::Color32::from_rgb(70, 140, 70),
        ),
        Kind::Secondary => (brand::STONE, brand::SURFACE, brand::BORDER),
    };
    let rich = egui::RichText::new(label.into()).size(m.body()).color(text);
    let rich = if kind == Kind::Primary { rich.strong() } else { rich };
    egui::Button::new(rich)
        .min_size(egui::vec2(0.0, m.hit()))
        .fill(fill)
        .stroke(egui::Stroke::new(1.0_f32, stroke))
        .corner_radius(egui::CornerRadius::same(8))
}

/// A full-width entry inside a ⋯ / ▾ menu, at least `HIT` tall.
pub(crate) fn menu_entry<'a>(m: &Metrics, label: impl Into<String>, colour: egui::Color32) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(label.into()).size(m.body()).color(colour))
        .min_size(egui::vec2(m.px(280.0), m.hit()))
        .frame(false)
}

/// A menu button (⋯ or ▾) that opens `add` under it. Menu entries get the
/// lobby's hit height.
pub(crate) fn menu_button<R>(
    ui: &mut egui::Ui,
    m: &Metrics,
    label: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    let m = *m;
    let (resp, _) = egui::containers::menu::MenuButton::from_button(button(&m, label, Kind::Secondary))
        .ui(ui, |ui| {
            ui.spacing_mut().item_spacing.y = m.px(2.0);
            add(ui)
        });
    resp
}

/// The `‹ Back` + title row every inner page starts with. `right` adds the
/// page's own buttons at the right-hand end. Returns true when Back was pressed.
pub(crate) fn page_header(
    ui: &mut egui::Ui,
    m: &Metrics,
    title: &str,
    right: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut back = false;
    let narrow = ui.available_width() < m.px(640.0);
    let mut right = Some(right);
    ui.horizontal(|ui| {
        if ui.add(button(m, "\u{2039}  Back", Kind::Secondary)).clicked() {
            back = true;
        }
        ui.add_space(m.px(10.0));
        ui.label(egui::RichText::new(title).size(m.page_title()).strong().color(brand::COPPER));
        if !narrow && let Some(r) = right.take() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), r);
        }
    });
    if let Some(r) = right.take() {
        // A phone: the page's buttons wrap onto their own row under the title.
        ui.add_space(m.px(6.0));
        ui.horizontal_wrapped(r);
    }
    back
}

/// A status line (an import or export result, a handover waiting).
pub(crate) fn status_line(ui: &mut egui::Ui, m: &Metrics, text: &str) {
    ui.label(egui::RichText::new(text).size(m.body()).color(egui::Color32::from_rgb(170, 190, 150)));
}

// ---------------------------------------------------------------------------
// Play together and More to play (hosting the existing columns for now)
// ---------------------------------------------------------------------------

fn draw_play_together(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics) -> MenuAction {
    #[allow(unused_mut)]
    let mut action = MenuAction::None;
    if page_header(ui, m, "Play together", |_| {}) {
        state.go(LobbyPage::Home);
    }
    ui.add_space(m.px(12.0));
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The Signet files can't be read: the way out (Disconnect) is here.
        if crate::signet_contacts::view().unreadable {
            egui::Frame::new()
                .fill(brand::SURFACE)
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    if crate::signet_contacts::ui::draw_unreadable_strip(ui)
                        && state.notice.as_deref().is_some_and(crate::signet_contacts::ui::is_host_refusal)
                    {
                        state.notice = None;
                    }
                });
            ui.add_space(m.px(10.0));
        }
        if ui.add(button(m, "Join a game…", Kind::Primary)).clicked() {
            state.dialog = crate::menu::MenuDialog::JoinDirect { address: String::new() };
        }
        ui.add_space(m.px(12.0));
        let w = ui.available_width().min(m.px(560.0));
        ui.allocate_ui(egui::vec2(w, ui.available_height()), |ui| {
            ui.set_width(w);
            let a = crate::friends_ui::draw_friends_column(ui, state);
            if !matches!(a, MenuAction::None) {
                action = a;
            }
        });
    }
    action
}

fn draw_more_to_play(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics) -> MenuAction {
    let mut action = MenuAction::None;
    if page_header(ui, m, crate::worlds_ui::words::MORE_TO_PLAY, |_| {}) {
        state.go(LobbyPage::Home);
    }
    ui.add_space(m.px(12.0));
    let w = ui.available_width().min(m.px(560.0));
    ui.allocate_ui(egui::vec2(w, ui.available_height()), |ui| {
        ui.set_width(w);
        egui::ScrollArea::vertical().id_salt("more_to_play").auto_shrink([false, false]).show(ui, |ui| {
            let a = crate::menu::draw_stash_column(ui, state);
            if !matches!(a, MenuAction::None) {
                action = a;
            }
        });
    });
    action
}

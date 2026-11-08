//! The lobby's pages (home page overhaul spec §2-§5, §7): which page shows,
//! the TV-safe area they draw in, Escape stepping back, the keyboard focus
//! seed, and the buttons and headers the pages share.
//!
//! `menu::draw_main_menu` draws the notice, the dialogs and the Trials gallery
//! first; when none of them owns the screen it calls [`draw`]. Home is
//! `home_ui`, Your worlds is `worlds_ui` (+ `world_detail_ui`), Play together
//! is `play_together_ui` (native) and More to play is `stash_page_ui`.

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
                    #[cfg(not(target_arch = "wasm32"))]
                    LobbyPage::PlayTogether => crate::play_together_ui::draw(ui, state, &m, keys),
                    // The web build has no multiplayer: its Play together tile
                    // explains the desktop app, so this page is never reached.
                    #[cfg(target_arch = "wasm32")]
                    LobbyPage::PlayTogether => {
                        state.go(LobbyPage::Home);
                        MenuAction::None
                    }
                    LobbyPage::Stash => crate::stash_page_ui::draw(ui, state, &m, keys),
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    const SIZES: [(&str, f32, f32); 4] =
        [("tv", 1920.0, 1080.0), ("desktop", 1280.0, 800.0), ("tablet", 834.0, 1112.0), ("phone", 390.0, 844.0)];

    /// Every text the frame drew: its rect and the smallest font size in it.
    fn texts_with_size(output: &egui::FullOutput) -> Vec<(String, egui::Rect, f32)> {
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Rect, f32)>) {
            match shape {
                egui::epaint::Shape::Text(t) => {
                    let size = t.galley.job.sections.iter().map(|s| s.format.font_id.size).fold(f32::MAX, f32::min);
                    out.push((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2()), size));
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        output.shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        out
    }

    /// The page on a screen of this size, a few frames in (egui measures first).
    fn draw_page(page: LobbyPage, w: f32, h: f32, with_fixture: bool) -> (egui::FullOutput, egui::Rect) {
        let ctx = egui::Context::default();
        let mut state = MenuState::new();
        if with_fixture {
            crate::fixture_together(&mut state);
        }
        state.go(page);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
        let mut out = Default::default();
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            out = ctx.run(input, |ctx| {
                let _ = crate::menu::draw_main_menu(ctx, &mut state);
            });
        }
        (out, crate::ui_safe_area::safe_rect(screen))
    }

    #[test]
    fn play_together_and_more_to_play_use_body_text_of_15_or_more_and_stay_in_the_safe_width() {
        for page in [LobbyPage::PlayTogether, LobbyPage::Stash] {
            for (name, w, h) in SIZES {
                let (out, safe) = draw_page(page, w, h, true);
                let seen = texts_with_size(&out);
                assert!(seen.len() > 5, "{page:?} on {name} drew almost nothing: {seen:?}");
                for (text, rect, size) in &seen {
                    assert!(*size >= 15.0, "{page:?} on {name}: {text:?} is {size} pt");
                    assert!(
                        rect.min.x >= safe.min.x - 1.0 && rect.max.x <= safe.max.x + 1.0,
                        "{page:?} on {name}: {text:?} at {rect:?} crosses the safe width {safe:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn play_together_has_its_sections_and_more_to_play_its_two_games() {
        let names = |page: LobbyPage| {
            let (out, _) = draw_page(page, 1920.0, 1080.0, true);
            texts_with_size(&out).into_iter().map(|(t, _, _)| t).collect::<Vec<_>>()
        };
        let together = names(LobbyPage::PlayTogether);
        for want in ["Play together", "Join a game", "Games on this network", "Room code", "Connect", "Friends", "Your address", "My Servers"] {
            assert!(together.iter().any(|t| t == want), "no {want:?} on Play together: {together:?}");
        }
        let more = names(LobbyPage::Stash);
        for want in ["More to play", "AxeNStax Official", "Hash Dash", "Satori Rush"] {
            assert!(more.iter().any(|t| t == want), "no {want:?} on More to play: {more:?}");
        }
        // Look up a creator is web only.
        assert!(!more.iter().any(|t| t.contains("Look up")), "{more:?}");
    }

    #[test]
    fn the_two_pages_step_back_to_home() {
        for page in [LobbyPage::PlayTogether, LobbyPage::Stash] {
            let mut menu = MenuState::new();
            menu.go(page);
            menu.step_back();
            assert_eq!(menu.page, LobbyPage::Home);
        }
    }

    #[test]
    fn the_lan_browser_lives_only_while_something_that_lists_games_is_up() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut state = MenuState::new();
        let mut frame = |state: &mut MenuState| {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                let _ = crate::menu::draw_main_menu(ctx, state);
            });
        };
        state.go(LobbyPage::PlayTogether);
        frame(&mut state);
        assert!(state.lan.is_some(), "Play together lists the games on this network");
        state.go(LobbyPage::Home);
        frame(&mut state);
        assert!(state.lan.is_none(), "leaving the page frees the beacon port");
    }
}

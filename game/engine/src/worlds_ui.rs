//! Your worlds (home page overhaul spec §4): the page header (+ New world,
//! Import ▾ or the web file buttons), the status lines, and the grid of world
//! cards. Selecting a card opens its detail (`world_detail_ui`): on the right
//! on a wide screen, as the whole page on a narrow one.
//!
//! The world list itself never holds Trial arenas (`exp-*`) or the Workshop
//! (`save::list_world_entries` and the web list leave them out): they are
//! reached from Trials and the Workshop tile.

use crate::brand;
use crate::lobby_pages_ui::{button, page_header, status_line, HostOnline, Keys, Kind};
#[cfg(not(target_arch = "wasm32"))]
use crate::lobby_pages_ui::{menu_button, menu_entry};
use crate::menu::{CardAction, MenuAction, MenuState};
use crate::ui_safe_area::Metrics;

/// Every player-facing label for the Stash toggle and the online host, in one
/// place: the Keep work is expected to rename them ("Host on my Keep", "Back
/// up to my Keep"), and that should be one edit here.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) mod words {
    /// The home tile and page that hold the official games and a creator look-up.
    pub const MORE_TO_PLAY: &str = "More to play";
    pub const MORE_TO_PLAY_LEAD: &str = "Official games and things people share";
    /// The web-only look-up section on that page, and its field hint.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub const LOOK_UP_CREATOR: &str = "Look up a creator";
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub const LOOK_UP_HINT: &str = "Their address (npub1…)";

    /// The official games on that page.
    pub const OFFICIAL: &str = "AxeNStax Official";
    pub const OFFICIAL_LEAD: &str = "Always here, even offline.";
    pub const PLAY: &str = "Play";
    /// One plain line per official game (no scores, no counts).
    pub const HASH_DASH_LINE: &str = "Mine as much as you can in three minutes.";
    pub const SATORI_RUSH_LINE: &str = "Race to mine your first Satori. Leave any time and carry on later.";
    /// What the web page says under the look-up field before anything is found.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub const LOOK_UP_LEAD: &str = "Play the experiences they share and try on their skins.";

    /// Play together (native only).
    pub const PLAY_TOGETHER: &str = "Play together";
    pub const JOIN_A_GAME: &str = "Join a game";
    pub const GAMES_ON_THIS_NETWORK: &str = "Games on this network";
    pub const ROOM_CODE: &str = "Room code";
    pub const CONNECT: &str = "Connect";
    pub const FRIENDS: &str = "Friends";
    pub const ADD_A_FRIEND: &str = "Add a friend";
    pub const YOUR_ADDRESS: &str = "Your address";

    /// Hosting from a world's page.
    pub const HOST_LAN: &str = "Host on this network";
    pub const HOST_ONLINE: &str = "Host for friends online";

    /// The per-world Stash toggle in More ⋯ (off / on, not yet confirmed / on).
    pub const KEEP_OFF: &str = "Keep in your Stash: Off";
    pub const KEEP_ON_UNCONFIRMED: &str = "Keep in your Stash: On…";
    pub const KEEP_ON: &str = "Keep in your Stash: On";
    pub const KEEP_OFF_HINT: &str = "Off — this world stays on this computer. Click to start stashing it.";
    pub const KEEP_ON_HINT: &str = "On — this world is in your Stash; open it on any computer.";
    pub const KEEP_ON_UNCONFIRMED_HINT: &str =
        "On — not in your Stash yet. Save the world (with a Stash sign-in) to upload it.";
}

/// From this safe width, world detail sits beside the grid instead of over it.
const SIDE_DETAIL_WIDTH: f32 = 1000.0;
const GAP: f32 = 16.0;

/// Draw Your worlds for one frame.
pub(crate) fn draw(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, host: HostOnline, keys: Keys) -> MenuAction {
    let mut action = MenuAction::None;
    if state.selected.is_some_and(|i| i >= state.worlds.len()) {
        state.selected = None;
    }
    let back = page_header(ui, m, "Your worlds", |ui| header_buttons(ui, state, m));
    if back {
        state.step_back();
        return action;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(s) = state.transfer_status.clone() {
            status_line(ui, m, &s);
        }
        for line in state.awaiting_imports.clone() {
            status_line(ui, m, &line);
        }
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = state.import_status.clone() {
        status_line(ui, m, &s);
    }
    ui.add_space(m.px(12.0));

    let icons = crate::egui_integration::block_texture_handles(ui.ctx());
    let area = ui.available_rect_before_wrap();
    let side = area.width() >= m.px(SIDE_DETAIL_WIDTH);
    match state.selected {
        Some(idx) if !side => {
            // A narrow screen: the world's page is the whole page.
            action = crate::world_detail_ui::draw(ui, state, idx, m, host, keys, &icons);
        }
        Some(idx) => {
            let detail_w = (area.width() * 0.4).clamp(m.px(380.0), m.px(560.0));
            let grid = egui::Rect::from_min_max(area.min, egui::pos2(area.max.x - detail_w - m.px(GAP), area.max.y));
            let detail = egui::Rect::from_min_max(egui::pos2(area.max.x - detail_w, area.min.y), area.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(grid), |ui| {
                let a = draw_grid(ui, state, m, Keys { seed_focus: false }, &icons);
                if !matches!(a, MenuAction::None) {
                    action = a;
                }
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(detail), |ui| {
                let a = crate::world_detail_ui::draw(ui, state, idx, m, host, keys, &icons);
                if !matches!(a, MenuAction::None) {
                    action = a;
                }
            });
        }
        None => action = draw_grid(ui, state, m, keys, &icons),
    }
    action
}

/// The header's right-hand buttons: + New world, then the import ones.
fn header_buttons(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics) {
    if ui.add(button(m, "+ New world", Kind::Primary)).clicked() {
        state.dialog = crate::menu::new_create_dialog();
    }
    #[cfg(not(target_arch = "wasm32"))]
    import_menu(ui, state, m);
    #[cfg(target_arch = "wasm32")]
    {
        if ui.add(button(m, "Restore from a file", Kind::Secondary)).clicked() {
            state.start_web_import();
        }
        let busy = state.profile_export.is_some();
        if ui
            .add_enabled(!busy, button(m, "Save everything to a file", Kind::Secondary))
            .on_hover_text(
                "Take your worlds to the desktop app: saves one .axeprofile file with every world in this browser plus your Trials records.",
            )
            .clicked()
        {
            state.start_web_profile_export();
        }
    }
}

/// Import ▾ (native). The two file-picker entries are left out on Android,
/// where a picker can't open a world file yet (they only failed there).
#[cfg(not(target_arch = "wasm32"))]
fn import_menu(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics) {
    let busy = state.dialog_pending();
    let android = cfg!(target_os = "android");
    menu_button(ui, m, "Import \u{23F7}", |ui| {
        if !android
            && ui
                .add_enabled(!busy, menu_entry(m, "Open a world file…", brand::STONE))
                .on_hover_text("Open a single .axeworld file from anywhere on your computer")
                .clicked()
        {
            state.open_file_dialog(crate::native_file_dialog::FileDialogRequest::OpenWorld);
            ui.close();
        }
        if ui
            .add_enabled(!busy, menu_entry(m, "Bring in everything from the transfer folder", brand::STONE))
            .on_hover_text("Reads every *.axeworld file in the world-transfer/ folder and imports each one")
            .clicked()
        {
            state.import_transfer_folder();
            ui.close();
        }
        if !android
            && ui
                .add_enabled(!busy, menu_entry(m, "Bring in your worlds from the web game…", brand::STONE))
                .on_hover_text(
                    "Open an .axeprofile file — every world you saved in the browser, plus your Trials records. Worlds already here are kept: a copy arrives as \"(web)\".",
                )
                .clicked()
        {
            state.open_file_dialog(crate::native_file_dialog::FileDialogRequest::OpenProfile);
            ui.close();
        }
    });
}

/// The grid of world cards, newest first.
fn draw_grid(
    ui: &mut egui::Ui,
    state: &mut MenuState,
    m: &Metrics,
    keys: Keys,
    icons: &[egui::TextureId],
) -> MenuAction {
    let mut action = MenuAction::None;
    #[cfg(not(target_arch = "wasm32"))]
    let nothing_awaiting = state.awaiting_imports.is_empty();
    #[cfg(target_arch = "wasm32")]
    let nothing_awaiting = true;
    if state.worlds.is_empty() && nothing_awaiting {
        ui.add_space(m.px(40.0));
        ui.vertical_centered(|ui| {
            brand::show_mark(ui, m.px(96.0), 255, 0.0);
            ui.add_space(m.px(16.0));
            ui.label(egui::RichText::new("No worlds yet").size(m.tile_title()).color(brand::STONE).strong());
            ui.add_space(m.px(10.0));
            let r = ui.add(button(m, "+ New world", Kind::Primary));
            if keys.seed_focus {
                r.request_focus();
            }
            if r.clicked() {
                state.dialog = crate::menu::new_create_dialog();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                ui.add_space(m.px(10.0));
                ui.label(
                    egui::RichText::new("Played in the browser? Import \u{23F7} brings your web worlds here.")
                        .size(m.body())
                        .color(brand::MUTED_TEXT),
                );
            }
        });
        return action;
    }
    egui::ScrollArea::vertical().id_salt("world_grid").auto_shrink([false, false]).show(ui, |ui| {
        let w = ui.available_width() - m.px(14.0);
        let gap = m.px(GAP);
        let min_card = m.px(260.0);
        let cols = (((w + gap) / (min_card + gap)).floor() as usize).clamp(1, 4);
        let card_w = ((w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        let n = state.worlds.len();
        let mut i = 0;
        while i < n {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for idx in i..(i + cols).min(n) {
                    let selected = state.selected == Some(idx);
                    let folder = state.worlds[idx].folder_name.clone();
                    let resp = ui
                        .push_id(("world_card", folder), |ui| card(ui, &state.worlds[idx], selected, card_w, m, icons))
                        .inner;
                    if keys.seed_focus && idx == 0 {
                        resp.request_focus();
                    }
                    // A card holds no buttons, so its whole-card click is unambiguous:
                    // one click opens the world's page, a double click plays it.
                    let ca = if resp.double_clicked() {
                        CardAction::DoubleClick
                    } else if resp.clicked() {
                        CardAction::Select
                    } else {
                        CardAction::None
                    };
                    let a = crate::menu::handle_card_action(state, idx, ca);
                    if !matches!(a, MenuAction::None) {
                        action = a;
                    }
                }
            });
            ui.add_space(gap);
            i += cols;
        }
    });
    action
}

/// One world card: its picture, name, mode, last played and size.
fn card(
    ui: &mut egui::Ui,
    entry: &crate::save::WorldEntry,
    selected: bool,
    w: f32,
    m: &Metrics,
    icons: &[egui::TextureId],
) -> egui::Response {
    let pic_h = (w * 9.0 / 16.0).floor();
    let text_h = m.px(84.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, pic_h + text_h), egui::Sense::click());
    let p = ui.painter().clone();
    let cr = egui::CornerRadius::same(12);
    let hot = resp.hovered() || resp.has_focus();
    let fill = if selected {
        brand::SURFACE_SELECTED
    } else if hot {
        brand::SURFACE_HOVER
    } else {
        brand::SURFACE
    };
    p.rect_filled(rect, cr, fill);
    let pic = egui::Rect::from_min_size(rect.min, egui::vec2(w, pic_h));
    crate::world_picture::paint(&p, pic, [12.0, 12.0, 0.0, 0.0], &entry.meta, icons);
    let pad = m.px(12.0);
    let x = rect.min.x + pad;
    let tw = w - 2.0 * pad;
    crate::home_ui::text(&p, egui::pos2(x, pic.max.y + pad * 0.7), &entry.meta.display_name, m.px(19.0), brand::STONE, tw);
    let badge_y = pic.max.y + pad * 0.7 + m.px(30.0);
    let mut bx = mode_badge(&p, egui::pos2(x, badge_y), &entry.meta.game_mode, m);
    if let Some((label, bg, fg)) = publish_badge(entry.meta.publish_badge()) {
        bx = pill(&p, egui::pos2(bx + m.px(6.0), badge_y), label, bg, fg, m);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let facts = format!(
        "{} \u{00B7} {}",
        crate::save::format_relative_time(entry.last_played),
        crate::save::format_size(entry.size_bytes)
    );
    #[cfg(target_arch = "wasm32")]
    let facts = crate::save::format_size(entry.size_bytes);
    crate::home_ui::text(&p, egui::pos2(bx + m.px(10.0), badge_y), &facts, m.body(), brand::MUTED_TEXT, rect.max.x - pad - bx - m.px(10.0));
    let stroke = if selected {
        egui::Stroke::new(2.0_f32, brand::COPPER)
    } else {
        egui::Stroke::new(1.0_f32, brand::BORDER)
    };
    p.rect_stroke(rect, cr, stroke, egui::StrokeKind::Inside);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// A small rounded label; returns its right edge.
pub(crate) fn pill(
    p: &egui::Painter,
    at: egui::Pos2,
    label: &str,
    bg: egui::Color32,
    fg: egui::Color32,
    m: &Metrics,
) -> f32 {
    let font = egui::FontId::proportional(m.px(12.5));
    let g = p.layout_no_wrap(label.to_string(), font, fg);
    let size = g.size() + egui::vec2(m.px(16.0), m.px(6.0));
    let r = egui::Rect::from_min_size(at, size);
    p.rect_filled(r, size.y / 2.0, bg);
    p.galley(r.center() - g.size() / 2.0, g, fg);
    r.max.x
}

/// The mode badge: the world's real mode (Adventure and Spectator used to read SURVIVAL).
pub(crate) fn mode_badge(p: &egui::Painter, at: egui::Pos2, mode: &str, m: &Metrics) -> f32 {
    let c = crate::world_picture::mode_colour(mode);
    let bg = crate::trials_diorama_ui::lerp_color(brand::DEEP_FRONTIER, c, 0.55);
    let fg = crate::trials_diorama_ui::lerp_color(c, egui::Color32::WHITE, 0.6);
    pill(p, at, &crate::world_picture::mode_label(mode).to_uppercase(), bg, fg, m)
}

/// The publish badge's words and colours, or `None` for a world that was never published.
pub(crate) fn publish_badge(badge: crate::save::PublishBadge) -> Option<(&'static str, egui::Color32, egui::Color32)> {
    use crate::save::PublishBadge;
    match badge {
        PublishBadge::NotPublished => None,
        PublishBadge::UpToDate => Some(("PUBLISHED", egui::Color32::from_rgb(28, 64, 38), egui::Color32::from_rgb(150, 224, 160))),
        PublishBadge::UnpublishedChanges => {
            Some(("EDITS PENDING", egui::Color32::from_rgb(74, 60, 24), egui::Color32::from_rgb(232, 206, 130)))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_keep_and_host_labels_live_here_and_carry_no_component_names() {
        use super::words::*;
        for w in [
            MORE_TO_PLAY,
            MORE_TO_PLAY_LEAD,
            OFFICIAL,
            OFFICIAL_LEAD,
            PLAY,
            HASH_DASH_LINE,
            SATORI_RUSH_LINE,
            LOOK_UP_LEAD,
            PLAY_TOGETHER,
            JOIN_A_GAME,
            GAMES_ON_THIS_NETWORK,
            ROOM_CODE,
            CONNECT,
            FRIENDS,
            ADD_A_FRIEND,
            YOUR_ADDRESS,
            LOOK_UP_CREATOR,
            LOOK_UP_HINT,
            HOST_LAN,
            HOST_ONLINE,
            KEEP_OFF,
            KEEP_ON_UNCONFIRMED,
            KEEP_ON,
            KEEP_OFF_HINT,
            KEEP_ON_HINT,
            KEEP_ON_UNCONFIRMED_HINT,
        ] {
            for name in ["Fray", "Ingle", "Bothy", "WildBloom"] {
                assert!(!w.contains(name), "{w:?} names {name}");
            }
        }
        assert!(!MORE_TO_PLAY.contains("Stash") && !MORE_TO_PLAY_LEAD.contains("Stash"));
    }

    #[test]
    fn no_lobby_page_shows_a_component_name() {
        for src in [
            include_str!("home_ui.rs"),
            include_str!("worlds_ui.rs"),
            include_str!("world_detail_ui.rs"),
            include_str!("lobby_pages_ui.rs"),
            include_str!("play_together_ui.rs"),
            include_str!("stash_page_ui.rs"),
        ] {
            let body = src.split("#[cfg(test)]").next().unwrap();
            for lit in body.split('"').skip(1).step_by(2) {
                for name in ["Fray", "Ingle", "Bothy", "WildBloom"] {
                    assert!(!lit.contains(name), "{lit:?} names {name}");
                }
            }
        }
    }
}

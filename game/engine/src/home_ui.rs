//! Home: the lobby's first page (home page overhaul spec §3). The biggest
//! thing on it is "carry on where you left off"; beside it the Trials tile;
//! under them four big destinations. The top bar holds the wordmark, the
//! profile chip, Settings and Quit; the footer holds the version line and the
//! last import or export message.

use crate::brand;
use crate::lobby_pages_ui::{button, menu_button, menu_entry, Keys, Kind};
use crate::menu::{CardAction, LobbyPage, MenuAction, MenuDialog, MenuState};
use crate::ui_safe_area::Metrics;

/// How the page lays out at this safe width (the same order at every step).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    /// Hero and Trials side by side, then the four tiles in one row.
    Wide,
    /// Hero, then Trials, full width; the tiles 2 x 2.
    Tablet,
    /// Everything in one column.
    Phone,
}

impl Layout {
    pub(crate) fn for_width(safe_width: f32, scale: f32) -> Self {
        if safe_width >= 900.0 * scale {
            Layout::Wide
        } else if safe_width >= 520.0 {
            Layout::Tablet
        } else {
            Layout::Phone
        }
    }
}

const GAP: f32 = 18.0;

/// Draw Home for one frame.
pub(crate) fn draw(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, keys: Keys) -> MenuAction {
    let mut action = MenuAction::None;
    let icons = crate::egui_integration::block_texture_handles(ui.ctx());

    // The footer is pinned to the bottom of the safe area; the rest scrolls as one.
    let foot_h = m.px(30.0);
    let full = ui.available_rect_before_wrap();
    let body = egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, full.max.y - foot_h - m.px(6.0)));
    let foot = egui::Rect::from_min_max(egui::pos2(full.min.x, full.max.y - foot_h), full.max);

    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::vertical().id_salt("home_page").auto_shrink([false, false]).show(ui, |ui| {
            top_bar(ui, state, m, &mut action);
            ui.add_space(m.px(GAP));
            let w = ui.available_width();
            let layout = Layout::for_width(w, m.scale);
            let gap = m.px(GAP);
            let tile_h = match layout {
                Layout::Phone => m.px(92.0),
                _ => m.px(124.0),
            };
            match layout {
                Layout::Wide => {
                    let hero_w = ((w - gap) * 0.62).floor();
                    let trials_w = w - gap - hero_w;
                    // The hero takes what the page's height leaves after the top bar and tiles.
                    let left = body.height() - m.px(64.0) - tile_h - 3.0 * gap;
                    let pic_h = (left - m.px(104.0)).clamp(m.px(150.0), hero_w * 9.0 / 16.0);
                    let card_h = pic_h + m.px(104.0);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        hero(ui, state, m, &icons, egui::vec2(hero_w, card_h), pic_h, keys, &mut action);
                        if crate::trials_gallery_ui::draw_trials_tile(ui, egui::vec2(trials_w, card_h), m.scale) {
                            state.trials_gallery.open();
                        }
                    });
                }
                Layout::Tablet | Layout::Phone => {
                    let pic_h = (w * 9.0 / 16.0).min(body.height() * 0.42);
                    // A phone stacks Play under the name, so the name gets the whole width.
                    let text_h = if w < m.px(480.0) { m.px(170.0) } else { m.px(104.0) };
                    hero(ui, state, m, &icons, egui::vec2(w, pic_h + text_h), pic_h, keys, &mut action);
                    ui.add_space(gap);
                    let th = if layout == Layout::Phone { m.px(150.0) } else { m.px(210.0) };
                    if crate::trials_gallery_ui::draw_trials_tile(ui, egui::vec2(w, th), m.scale) {
                        state.trials_gallery.open();
                    }
                }
            }
            ui.add_space(gap);
            let per_row = match layout {
                Layout::Wide => 4,
                Layout::Tablet => 2,
                Layout::Phone => 1,
            };
            let tile_w = ((w - gap * (per_row as f32 - 1.0)) / per_row as f32).floor();
            let tiles = Dest::ALL;
            for row in tiles.chunks(per_row) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for d in row {
                        destination(ui, state, m, &icons, *d, egui::vec2(tile_w, tile_h), &mut action);
                    }
                });
                ui.add_space(gap);
            }
        });
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(foot), |ui| footer(ui, state, m, &mut action));
    action
}

// ---------------------------------------------------------------------------
// Top bar
// ---------------------------------------------------------------------------

fn top_bar(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, action: &mut MenuAction) {
    let narrow = ui.available_width() < m.px(620.0);
    let wordmark_w = if narrow { 170.0 } else { m.px(220.0) };
    let mut buttons = |ui: &mut egui::Ui| {
        // Right to left: the first added is rightmost.
        #[cfg(target_arch = "wasm32")]
        if ui.add(button(m, "Exit", Kind::Secondary)).on_hover_text("Leave the game, back to the home page").clicked() {
            crate::wasm_auth::exit_to_lobby();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if ui.add(button(m, "Quit", Kind::Secondary)).clicked() {
            *action = MenuAction::Quit;
        }
        if ui.add(button(m, "Settings", Kind::Secondary)).on_hover_text("Graphics, your relays and more").clicked() {
            state.show_settings = true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        profile_chip(ui, state, m);
        #[cfg(target_arch = "wasm32")]
        let _ = &action;
    };
    if narrow {
        brand::show_wordmark(ui, wordmark_w);
        ui.add_space(m.px(8.0));
        let w = ui.available_width();
        ui.allocate_ui_with_layout(egui::vec2(w, m.hit()), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            buttons(ui)
        });
    } else {
        ui.horizontal(|ui| {
            brand::show_wordmark(ui, wordmark_w);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| buttons(ui));
        });
    }
}

/// The profile chip: "Guest" (opens the sign-in dialog) or the short npub
/// with "Your address" and "Sign out" under it. Native only: the web build has
/// no sign-in.
#[cfg(not(target_arch = "wasm32"))]
fn profile_chip(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics) {
    match crate::signet::native_signer::current_owner_pubkey() {
        None => {
            if ui
                .add(button(m, "Guest", Kind::Secondary))
                .on_hover_text("Sign in to play with friends.")
                .clicked()
            {
                crate::native_signin::reset();
                state.dialog = MenuDialog::SignIn { paste_uri: String::new(), show_paste: false };
            }
        }
        Some(hex) => {
            let npub = crate::signet::native_signer::NativeIdentity::SignedIn { pubkey_hex: hex }
                .npub()
                .unwrap_or_default();
            let label = format!("{} \u{23F7}", crate::plan::short_npub(&npub, 16));
            menu_button(ui, m, &label, |ui| {
                ui.set_max_width(m.px(300.0));
                crate::friends_ui::draw_your_address(ui, Some(&npub));
                ui.separator();
                if ui
                    .add(menu_entry(m, "Sign out", brand::STONE))
                    .on_hover_text("Forget this device's Signet sign-in")
                    .clicked()
                {
                    crate::native_signin::sign_out();
                    ui.close();
                }
            })
            .on_hover_text("Signed in with Signet");
        }
    }
}

// ---------------------------------------------------------------------------
// Continue (the hero)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn hero(
    ui: &mut egui::Ui,
    state: &mut MenuState,
    m: &Metrics,
    icons: &[egui::TextureId],
    size: egui::Vec2,
    pic_h: f32,
    keys: Keys,
    action: &mut MenuAction,
) {
    let (rect, card) = ui.allocate_exact_size(size, egui::Sense::click());
    let pad = m.px(14.0);
    let cr = egui::CornerRadius::same(16);
    let p = ui.painter().clone();
    let hot = !state.worlds.is_empty() && (card.hovered() || card.has_focus());
    p.rect_filled(rect, cr, if hot { brand::SURFACE_HOVER } else { brand::SURFACE });
    p.rect_stroke(rect, cr, egui::Stroke::new(1.0_f32, brand::BORDER), egui::StrokeKind::Inside);
    let pic = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), pic_h));
    let text_rect = egui::Rect::from_min_max(egui::pos2(rect.min.x + pad, pic.max.y + pad * 0.8), rect.max - egui::vec2(pad, pad));
    // Play is added after the card, so it sits on top of the card's own click.
    let stacked = rect.width() < m.px(480.0);
    let (play_rect, play_w) = if stacked {
        let r = egui::Rect::from_min_max(egui::pos2(text_rect.min.x, text_rect.max.y - m.px(56.0)), text_rect.max);
        (r, -pad)
    } else {
        let pw = m.px(150.0).min(text_rect.width() * 0.5);
        let r = egui::Rect::from_min_size(
            egui::pos2(text_rect.max.x - pw, text_rect.center().y - m.px(28.0)),
            egui::vec2(pw, m.px(56.0)),
        );
        (r, pw)
    };

    #[cfg(target_arch = "wasm32")]
    let loading = !state.local_loaded;
    #[cfg(not(target_arch = "wasm32"))]
    let loading = false;

    let Some(entry) = state.worlds.first() else {
        crate::world_picture::paint_scene(&p, pic, [16.0, 16.0, 0.0, 0.0], crate::world_picture::Scene::Meadow, "survival", icons);
        let title = if loading { "Loading your worlds…" } else { "Start your first world" };
        text(&p, text_rect.left_top(), title, m.tile_title(), brand::STONE, text_rect.width() - play_w - pad);
        if !loading {
            text(&p, text_rect.left_top() + egui::vec2(0.0, m.px(34.0)), "Your own place to build.", m.body(), brand::MUTED_TEXT, text_rect.width() - play_w - pad);
            let r = ui.put(play_rect, button(m, "Create a world", Kind::Primary));
            if keys.seed_focus {
                r.request_focus();
            }
            if r.clicked() {
                state.dialog = crate::menu::new_create_dialog();
            }
        }
        return;
    };

    crate::world_picture::paint(&p, pic, [16.0, 16.0, 0.0, 0.0], &entry.meta, icons);
    let name_w = text_rect.width() - play_w - pad;
    text(&p, text_rect.left_top(), &entry.meta.display_name, m.tile_title(), brand::STONE, name_w);
    let mode = crate::world_picture::mode_label(&entry.meta.game_mode);
    #[cfg(not(target_arch = "wasm32"))]
    let line = format!("{mode} \u{00B7} Played {}", crate::save::format_relative_time(entry.last_played).to_lowercase());
    #[cfg(target_arch = "wasm32")]
    let line = mode.to_string();
    text(&p, text_rect.left_top() + egui::vec2(0.0, m.px(34.0)), &line, m.body(), brand::MUTED_TEXT, name_w);

    let play = ui.put(play_rect, button(m, "\u{25B6}  Play", Kind::Primary));
    if keys.seed_focus {
        play.request_focus();
    }
    if play.clicked() {
        *action = crate::menu::handle_card_action(state, 0, CardAction::Play);
    } else if card.clicked() {
        state.go(LobbyPage::Worlds);
        state.selected = Some(0);
    }
    if card.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

/// One line of text, cut short with "…" past `max_w`.
pub(crate) fn text(p: &egui::Painter, at: egui::Pos2, s: &str, size: f32, colour: egui::Color32, max_w: f32) {
    let mut job = egui::text::LayoutJob::simple_singleline(s.to_string(), egui::FontId::proportional(size), colour);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(10.0));
    let g = p.layout_job(job);
    p.galley(at, g, colour);
}

// ---------------------------------------------------------------------------
// The four destinations
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dest {
    Worlds,
    Workshop,
    Together,
    MoreToPlay,
}

impl Dest {
    const ALL: [Dest; 4] = [Dest::Worlds, Dest::Workshop, Dest::Together, Dest::MoreToPlay];

    fn title(self) -> &'static str {
        match self {
            Dest::Worlds => "Your worlds",
            Dest::Workshop => "The Workshop",
            Dest::Together => "Play together",
            Dest::MoreToPlay => crate::worlds_ui::words::MORE_TO_PLAY,
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Dest::Worlds => "Every world you've made.",
            Dest::Workshop => "Redesign blocks, mobs and your skin.",
            Dest::Together => "Friends, invites and games nearby.",
            Dest::MoreToPlay => crate::worlds_ui::words::MORE_TO_PLAY_LEAD,
        }
    }

    /// The block drawn as the tile's icon (`/give` name).
    fn icon(self) -> &'static str {
        match self {
            Dest::Worlds => "grass",
            Dest::Workshop => "crafting_table",
            Dest::Together => "campfire_lit",
            Dest::MoreToPlay => "chest",
        }
    }

    fn colour(self) -> egui::Color32 {
        match self {
            Dest::Worlds => egui::Color32::from_rgb(84, 156, 64),
            Dest::Workshop => egui::Color32::from_rgb(190, 120, 40),
            Dest::Together => egui::Color32::from_rgb(64, 132, 196),
            Dest::MoreToPlay => egui::Color32::from_rgb(170, 96, 160),
        }
    }
}

/// "5 worlds" / "1 world" / "None yet" for the Your worlds tile.
pub(crate) fn world_count_line(n: usize) -> String {
    match n {
        0 => "None yet".to_string(),
        1 => "1 world".to_string(),
        n => format!("{n} worlds"),
    }
}

fn destination(
    ui: &mut egui::Ui,
    state: &mut MenuState,
    m: &Metrics,
    icons: &[egui::TextureId],
    d: Dest,
    size: egui::Vec2,
    action: &mut MenuAction,
) {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter().clone();
    let cr = egui::CornerRadius::same(14);
    let hot = resp.hovered() || resp.has_focus();
    p.rect_filled(rect, cr, if hot { brand::SURFACE_HOVER } else { brand::SURFACE });
    // The colour band down the left edge, as on the Trials shelves.
    let band = egui::Rect::from_min_size(rect.min, egui::vec2(m.px(8.0), rect.height()));
    p.rect_filled(band, egui::CornerRadius { nw: 14, sw: 14, ne: 0, se: 0 }, d.colour());
    p.rect_stroke(rect, cr, egui::Stroke::new(1.0_f32, brand::BORDER), egui::StrokeKind::Inside);
    let icon = m.px(42.0).min(rect.height() * 0.5);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.min.x + m.px(18.0) + icon / 2.0, rect.center().y),
        egui::vec2(icon, icon),
    );
    if let Some(cube) = crate::trials_gallery::resolve_cube(d.icon(), [0, 0, 0]) {
        crate::trials_diorama_ui::paint_diorama(&p, icon_rect, &[cube], icons, 0.5);
    }
    let tx = icon_rect.max.x + m.px(12.0);
    let tw = rect.max.x - tx - m.px(14.0);
    let title_y = rect.center().y - m.px(26.0);
    let title_room = if d == Dest::Workshop { tw - m.hit() + m.px(8.0) } else { tw };
    text(&p, egui::pos2(tx, title_y), d.title(), m.px(22.0), brand::STONE, title_room);
    let sub = if d == Dest::Worlds {
        format!("{} {}", world_count_line(state.worlds.len()), "\u{00B7} Every world you've made.")
    } else {
        d.subtitle().to_string()
    };
    let mut job = egui::text::LayoutJob::single_section(sub, egui::TextFormat::simple(egui::FontId::proportional(m.body()), brand::MUTED_TEXT));
    job.wrap = egui::text::TextWrapping { max_width: tw.max(10.0), max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
    let g = p.layout_job(job);
    p.galley(egui::pos2(tx, title_y + m.px(34.0)), g, brand::MUTED_TEXT);

    // The Workshop's own ⋯ menu, added after the tile so it takes its clicks.
    if d == Dest::Workshop {
        let s = m.hit();
        let r = egui::Rect::from_min_size(egui::pos2(rect.max.x - s - m.px(6.0), rect.min.y + m.px(6.0)), egui::vec2(s, s));
        ui.put(r, |ui: &mut egui::Ui| {
            menu_button(ui, m, "\u{2026}", |ui| {
                if ui
                    .add(menu_entry(m, "Clear the Workshop floor…", brand::STONE))
                    .on_hover_text("Clear everything built in The Workshop, back to an empty floor. Keeps your saved skins.")
                    .clicked()
                {
                    state.dialog = MenuDialog::ResetWorkshop;
                    ui.close();
                }
            })
        });
    }

    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if !resp.clicked() {
        return;
    }
    match d {
        Dest::Worlds => {
            state.selected = None;
            state.go(LobbyPage::Worlds);
        }
        Dest::Workshop => *action = MenuAction::EnterWorkshop,
        Dest::Together => {
            #[cfg(not(target_arch = "wasm32"))]
            state.go(LobbyPage::PlayTogether);
            // The web build has no multiplayer: the tile explains the desktop app.
            #[cfg(target_arch = "wasm32")]
            {
                state.dialog = MenuDialog::DesktopOnly(crate::menu::AppFeature::Join);
            }
        }
        Dest::MoreToPlay => state.go(LobbyPage::Stash),
    }
}

// ---------------------------------------------------------------------------
// Footer
// ---------------------------------------------------------------------------

fn footer(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, action: &mut MenuAction) {
    ui.horizontal(|ui| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut restart = false;
            crate::menu::draw_version_line(ui, &mut restart);
            if restart {
                *action = MenuAction::RestartForUpdate;
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = &action;
        #[cfg(not(target_arch = "wasm32"))]
        let status = state.transfer_status.clone();
        #[cfg(target_arch = "wasm32")]
        let status = state.import_status.clone();
        if let Some(s) = status {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(s).size(m.body()).color(egui::Color32::from_rgb(170, 190, 150)))
                        .truncate(),
                );
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_steps_down_by_width() {
        assert_eq!(Layout::for_width(1728.0, 1.25), Layout::Wide, "a television");
        assert_eq!(Layout::for_width(1152.0, 1.0), Layout::Wide, "a desktop");
        assert_eq!(Layout::for_width(750.0, 1.0), Layout::Tablet, "a tablet");
        assert_eq!(Layout::for_width(358.0, 1.0), Layout::Phone, "a phone");
    }

    #[test]
    fn the_world_count_reads_naturally() {
        assert_eq!(world_count_line(0), "None yet");
        assert_eq!(world_count_line(1), "1 world");
        assert_eq!(world_count_line(5), "5 worlds");
    }

    #[test]
    fn every_tile_icon_is_a_block() {
        for d in Dest::ALL {
            assert!(crate::trials_gallery::resolve_cube(d.icon(), [0, 0, 0]).is_some(), "{d:?}");
        }
    }

    #[test]
    fn the_home_copy_never_says_stash() {
        let body = include_str!("home_ui.rs").split("#[cfg(test)]").next().unwrap();
        assert!(!body.contains("\"The Stash\""));
        for d in Dest::ALL {
            assert!(!d.title().contains("Stash") && !d.subtitle().contains("Stash"), "{d:?}");
        }
    }
}

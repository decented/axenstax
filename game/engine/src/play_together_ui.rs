//! Play together (home page overhaul spec §5, native): one page for
//! everything about other people. It hosts the existing pieces, each in a
//! card: "Join a game" (the Join Game dialog's content inline), Friends (the
//! contact list with Join, Add a friend and the Signet contacts row), Your
//! address, My Servers and My Keeps.
//!
//! One column on a narrow screen, two on a wide one. Nothing here changes how
//! joining, admission, invites or storage work: every button emits the same
//! `MenuAction` the old column and dialog did. Hosting stays on a world's page.
//!
//! Red lines: people you already know, no directory, no social framing.
#![cfg(not(target_arch = "wasm32"))]

use crate::brand;
use crate::lobby_pages_ui::{button, page_header, Keys, Kind};
use crate::menu::{MenuAction, MenuState};
use crate::ui_safe_area::Metrics;
use crate::worlds_ui::words;

/// From this safe width (before scaling) the cards sit in two columns.
const TWO_COLUMNS_FROM: f32 = 980.0;
const GAP: f32 = 18.0;

/// Draw Play together for one frame.
pub(crate) fn draw(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, keys: Keys) -> MenuAction {
    let mut action = MenuAction::None;
    if page_header(ui, m, words::PLAY_TOGETHER, |_| {}) {
        state.step_back();
        return action;
    }
    ui.add_space(m.px(12.0));
    // The Signet files can't be read: the way out (Disconnect) is here.
    if crate::signet_contacts::view().unreadable {
        egui::Frame::new()
            .fill(brand::SURFACE)
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::same(m.px(10.0) as i8))
            .show(ui, |ui| {
                if crate::signet_contacts::ui::draw_unreadable_strip(ui, m)
                    && state.notice.as_deref().is_some_and(crate::signet_contacts::ui::is_host_refusal)
                {
                    state.notice = None;
                }
            });
        ui.add_space(m.px(10.0));
    }

    egui::ScrollArea::vertical().id_salt("play_together").auto_shrink([false, false]).show(ui, |ui| {
        let w = ui.available_width();
        let gap = m.px(GAP);
        if w >= m.px(TWO_COLUMNS_FROM) {
            let col_w = ((w - gap) / 2.0).floor();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui_with_layout(
                    egui::vec2(col_w, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(col_w);
                        join_a_game(ui, state, m, keys, &mut action);
                        friends(ui, state, m, &mut action);
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(col_w, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(col_w);
                        your_address(ui, m);
                        my_servers_and_keeps(ui, state, m, &mut action);
                    },
                );
            });
        } else {
            join_a_game(ui, state, m, keys, &mut action);
            friends(ui, state, m, &mut action);
            your_address(ui, m);
            my_servers_and_keeps(ui, state, m, &mut action);
        }
    });
    action
}

/// A card: the lobby surface, rounded, with an optional heading and lead line.
fn section(ui: &mut egui::Ui, m: &Metrics, title: Option<(&str, Option<&str>)>, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(brand::SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::same(m.px(16.0) as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some((heading, lead)) = title {
                ui.label(egui::RichText::new(heading).size(m.px(20.0)).strong().color(brand::COPPER));
                if let Some(lead) = lead {
                    ui.label(egui::RichText::new(lead).size(m.body()).color(brand::MUTED_TEXT));
                }
                ui.add_space(m.px(8.0));
            }
            add(ui);
        });
    ui.add_space(m.px(GAP));
}

fn sub_heading(ui: &mut egui::Ui, m: &Metrics, text: &str) {
    ui.label(egui::RichText::new(text).size(m.body()).strong().color(brand::STONE));
    ui.add_space(m.px(4.0));
}

/// Join a game: the Join Game dialog's content inline, "Games on this
/// network" (heard on this network only) and the room-code field.
fn join_a_game(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, keys: Keys, action: &mut MenuAction) {
    section(ui, m, Some((words::JOIN_A_GAME, None)), |ui| {
        sub_heading(ui, m, words::GAMES_ON_THIS_NETWORK);
        let games = crate::lan_ui::poll_games(&mut state.lan);
        // A click on a heard room joins it straight away.
        if let Some(room) = crate::lan_ui::draw_lan_rows(ui, &games, Some(m)) {
            state.lan = None;
            *action = MenuAction::JoinLanRoom(room);
            return;
        }
        ui.add_space(m.px(14.0));
        sub_heading(ui, m, words::ROOM_CODE);
        let mut confirmed = false;
        ui.horizontal(|ui| {
            let connect_w = m.px(130.0);
            let field_w = (ui.available_width() - connect_w - ui.spacing().item_spacing.x).max(m.px(120.0));
            let field = crate::menu::menu_text_field_tall(
                ui,
                &mut state.join_code,
                words::ROOM_CODE,
                crate::lan_ui::CODE_HINT,
                field_w,
                m.body(),
                m.hit(),
            );
            // Enter in the field connects, as the Connect button does.
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                confirmed = true;
            }
            let connect = ui.add_sized(egui::vec2(connect_w, m.hit()), button(m, words::CONNECT, Kind::Primary));
            if keys.seed_focus {
                connect.request_focus();
            }
            if connect.clicked() {
                confirmed = true;
            }
        });
        ui.add_space(m.px(4.0));
        ui.label(egui::RichText::new(crate::lan_ui::CODE_HELP).size(m.body()).color(brand::MUTED_TEXT));
        if confirmed {
            // Free the beacon port now: the lobby is not drawn once a join starts.
            state.lan = None;
            *action = MenuAction::JoinGame(std::mem::take(&mut state.join_code));
        }
    });
}

/// Friends: the contact list with Join, Add a friend and the Signet contacts row.
fn friends(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, action: &mut MenuAction) {
    section(ui, m, Some((words::FRIENDS, Some(crate::friends_ui::friends_lead()))), |ui| {
        let a = crate::friends_ui::draw_friends_section(ui, state, m);
        if !matches!(a, MenuAction::None) {
            *action = a;
        }
    });
}

/// Your address: the QR and Copy when signed in, the sign-in prompt when signed out.
fn your_address(ui: &mut egui::Ui, m: &Metrics) {
    section(ui, m, Some((words::YOUR_ADDRESS, None)), |ui| {
        let identity = crate::signet::native_signer::load_identity();
        crate::friends_ui::draw_your_address(ui, identity.npub().as_deref(), Some(m));
    });
}

/// My Servers and My Keeps: the rows as they were (My Keeps stays hidden while empty).
fn my_servers_and_keeps(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, action: &mut MenuAction) {
    section(ui, m, None, |ui| {
        let a = crate::menu::draw_my_servers_column_inner(ui, state, m);
        if !matches!(a, MenuAction::None) {
            *action = a;
        }
    });
    if !state.keeps.is_empty() {
        section(ui, m, None, |ui| {
            let a = crate::menu::draw_my_keeps_column_inner(ui, state, m);
            if !matches!(a, MenuAction::None) {
                *action = a;
            }
        });
    }
}

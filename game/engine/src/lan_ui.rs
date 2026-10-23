//! LAN play UI (gap-audit T2-10): the "Games on this network" list in the Join
//! Game dialog and the "you are hosting" panel in the pause menu.
//!
//! The wording and ordering rules are pure and unit-tested; the egui half only
//! lays them out. The list is Fray's LAN beacon (`decented_net::lan_browse`),
//! heard on this network only: no directory, relay or internet lookup of any
//! kind — a central, browsable list of player-run games would make AxeNStax
//! the platform (red line 1). A room code reaches further, but only because the
//! host handed it out, and only over the relays the players chose themselves.
//!
//! NATIVE-ONLY: the web build has no multiplayer.
#![cfg(not(target_arch = "wasm32"))]

use decented_net::{LanBrowser, LanRoom};

use crate::fray_play::{CodeStatus, RoomView};
use crate::lan_host::HostAccess;

pub const GAMES_HEADING: &str = "GAMES ON THIS NETWORK";
pub const GAMES_SEARCHING: &str =
    "Looking for games on your Wi-Fi or Ethernet. Ask a friend to press Host on their \
     world, or type their room code below.";
pub const GAMES_NOT_LISTENING: &str =
    "Can't listen for games on this network. You can still type a room code below.";

/// The Join dialog's field: a room code (or a server address).
pub const CODE_HEADING: &str = "ROOM CODE";
pub const CODE_HINT: &str = "KMT RWX";
pub const CODE_HELP: &str =
    "The host's code is in their pause menu. A server address (axenstax://...) works here too.";

/// Said when the Join field holds an `ip:port`, which no longer joins anything.
pub const ADDRESS_JOIN_RETIRED: &str =
    "Joining by address has been replaced by room codes. Ask the host for the code in \
     their pause menu, or pick their game from the list.";
/// Said when the Join field holds neither a code nor a server address.
pub const NOT_A_CODE: &str =
    "That isn't a room code. A code is 6 letters, like KMT RWX, and the host finds it in \
     their pause menu.";

/// The lobby's line while a join looks for its host.
pub fn joining_notice(label: &str) -> String {
    format!("Joining {label}...")
}

/// What the Join field says when it holds no code and no server address.
pub fn bad_code_notice(text: &str) -> &'static str {
    if text.trim().parse::<std::net::SocketAddr>().is_ok() {
        ADDRESS_JOIN_RETIRED
    } else {
        NOT_A_CODE
    }
}

/// Why a heard room cannot be joined, shown in place of the click.
pub fn unavailable_reason(r: &LanRoom) -> Option<&'static str> {
    if r.max_players > 0 && r.player_count >= r.max_players {
        Some("full")
    } else {
        None
    }
}

/// A room's name as the lobby shows it.
pub fn room_label(r: &LanRoom) -> String {
    if r.name.is_empty() { "Unnamed world".to_string() } else { r.name.clone() }
}

/// The names the lobby shows for `rooms`, in the order given. Every Fray room
/// announces the same neutral name ([`crate::fray_play::ROOM_NAME`]), so rooms
/// of one name are told apart by a number: "AxeNStax world", "AxeNStax world (2)".
/// The room code is not shown here: it works over the internet, and this list
/// is everyone on the network.
pub fn row_names(rooms: &[LanRoom]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    rooms
        .iter()
        .map(|r| {
            let label = room_label(r);
            let n = seen.entry(label.to_lowercase()).or_insert(0);
            *n += 1;
            if *n == 1 { label } else { format!("{label} ({n})") }
        })
        .collect()
}

/// One row's text: `World name  ·  2 of 5 players`, with the reason appended
/// when the room cannot be joined.
pub fn entry_label(name: &str, r: &LanRoom) -> String {
    let mut out = format!("{name}  \u{b7}  {} of {} players", r.player_count, r.max_players);
    if let Some(why) = unavailable_reason(r) {
        out.push_str(&format!("  \u{b7}  {why}"));
    }
    out
}

/// Stable display order: joinable rooms first, then by name, then code — so
/// the list does not reshuffle as beacons arrive.
pub fn ordered(rooms: &[LanRoom]) -> Vec<LanRoom> {
    let mut v = rooms.to_vec();
    v.sort_by(|a, b| {
        (unavailable_reason(a).is_some(), a.name.to_lowercase(), &a.code)
            .cmp(&(unavailable_reason(b).is_some(), b.name.to_lowercase(), &b.code))
    });
    v
}

/// A frame's view of the LAN: who is announcing, and whether we can hear at all.
pub struct LanGames {
    pub rooms: Vec<LanRoom>,
    pub listening: bool,
}

/// Open the browser on first use, drain it, and snapshot the list. The caller
/// keeps the `Option` on its state and drops it (freeing the beacon port) when
/// the dialog closes.
pub fn poll_games(browser: &mut Option<LanBrowser>) -> LanGames {
    let b = browser.get_or_insert_with(|| decented_net::lan_browse(&crate::fray_play::play_config(&[])));
    let rooms = b.poll();
    LanGames { rooms: ordered(&rooms), listening: b.is_listening() }
}

/// The "Games on this network" section. Returns the room the player clicked,
/// if any.
pub fn draw_lan_games(ui: &mut egui::Ui, games: &LanGames) -> Option<LanRoom> {
    let mut picked = None;
    ui.label(
        egui::RichText::new(GAMES_HEADING)
            .size(11.0)
            .color(egui::Color32::from_rgb(136, 136, 153))
            .strong(),
    );
    ui.add_space(4.0);
    if !games.listening {
        ui.label(egui::RichText::new(GAMES_NOT_LISTENING).size(12.0).color(egui::Color32::from_rgb(220, 170, 120)));
    } else if games.rooms.is_empty() {
        ui.label(egui::RichText::new(GAMES_SEARCHING).size(12.0).color(egui::Color32::from_rgb(150, 150, 160)));
    } else {
        egui::ScrollArea::vertical().max_height(150.0).auto_shrink([true, true]).show(ui, |ui| {
            let names = row_names(&games.rooms);
            for (r, name) in games.rooms.iter().zip(&names) {
                let joinable = unavailable_reason(r).is_none();
                let btn = egui::Button::new(egui::RichText::new(entry_label(name, r)).size(13.0))
                    .min_size(egui::vec2(360.0, 28.0))
                    .corner_radius(egui::CornerRadius::same(6));
                let resp = ui.add_enabled(joinable, btn);
                if resp.clicked() {
                    picked = Some(r.clone());
                }
                ui.add_space(3.0);
            }
        });
    }
    ui.add_space(12.0);
    picked
}

/// The status line under the room code.
pub fn code_status_line(status: CodeStatus) -> String {
    match status {
        CodeStatus::Published { up, total } => format!(
            "Friends on this network pick this world under Join Game. Friends anywhere else \
             type the code (it is on {up} of your {total} relays)."
        ),
        CodeStatus::Publishing => "Putting the code on your relays... Friends on this network \
                                   can already pick this world under Join Game."
            .to_string(),
        CodeStatus::LanOnly { tried: 0 } => "You have no relays set, so the code can't be used: \
                                             only friends on this network can join, from Join \
                                             Game."
            .to_string(),
        CodeStatus::LanOnly { .. } => "Couldn't reach any of your relays, so the code doesn't \
                                       work yet: only friends on this network can join, from \
                                       Join Game. Still trying."
            .to_string(),
    }
}

/// The toast shown once, when the code first reaches a relay.
pub fn code_live_toast(code: &str) -> String {
    format!("Room code {code} is live: friends can type it under Join Game.")
}

/// The pause-menu panel shown while this machine hosts: the room code, whether
/// it is published, how full the world is, and a copy button.
pub fn draw_host_panel(ctx: &egui::Context, access: &HostAccess, room: &RoomView, players: (usize, usize)) {
    egui::Area::new(egui::Id::new("lan_host_panel"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -28.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(420.0);
                ui.label(egui::RichText::new("Hosting on your network").strong().size(15.0));
                ui.label(
                    egui::RichText::new(format!("{} of {} players in this world", players.0, players.1))
                        .size(12.0)
                        .color(egui::Color32::from_rgb(150, 150, 160)),
                );
                ui.add_space(4.0);
                if access.addresses.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "No network connection found. Connect to Wi-Fi or Ethernet so friends can join.",
                        )
                        .color(egui::Color32::from_rgb(230, 150, 150)),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label("Room code");
                    ui.label(egui::RichText::new(&room.code).monospace().size(18.0).strong());
                    if ui.small_button("Copy").clicked() {
                        ui.ctx().copy_text(room.code.clone());
                    }
                });
                ui.label(egui::RichText::new(code_status_line(room.status)).size(12.0));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn room(name: &str, code: &str, last_octet: u8, players: u8, max: u8) -> LanRoom {
        LanRoom {
            name: name.to_string(),
            code: code.to_string(),
            player_count: players,
            max_players: max,
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, last_octet)), 40_000),
            token: "00".repeat(16),
        }
    }

    #[test]
    fn a_row_reads_name_and_players_and_never_the_room_code() {
        let r = room("Axo's World", "KMTRWX", 23, 2, 5);
        let label = entry_label(&room_label(&r), &r);
        assert_eq!(label, "Axo's World  \u{b7}  2 of 5 players");
        // The code works over the internet; the LAN list is everyone nearby.
        for rooms in [vec![r.clone()], vec![room("a", "KMTRWX", 1, 5, 5)]] {
            for (r, name) in rooms.iter().zip(row_names(&rooms)) {
                let shown = entry_label(&name, r);
                let code = decented_net::room::display_code(&r.code);
                assert!(!shown.contains(&r.code) && !shown.contains(&code), "{shown}");
                assert!(!shown.to_lowercase().contains("code"), "{shown}");
            }
        }
    }

    #[test]
    fn rooms_with_one_name_are_numbered_in_list_order() {
        let list = ordered(&[
            room(crate::fray_play::ROOM_NAME, "BBBBBB", 2, 1, 5),
            room(crate::fray_play::ROOM_NAME, "AAAAAA", 1, 1, 5),
            room(&crate::fray_play::ROOM_NAME.to_uppercase(), "CCCCCC", 3, 1, 5),
        ]);
        assert_eq!(
            row_names(&list),
            ["AxeNStax world", "AxeNStax world (2)", "AXENSTAX WORLD (3)"]
        );
        assert_eq!(row_names(&[]), Vec::<String>::new());
    }

    #[test]
    fn an_unnamed_world_still_gets_a_row_label() {
        let r = room("", "KMTRWX", 1, 0, 5);
        assert!(entry_label(&room_label(&r), &r).starts_with("Unnamed world"));
    }

    #[test]
    fn a_full_room_says_why_it_cannot_be_joined() {
        assert_eq!(unavailable_reason(&room("a", "KMTRWX", 1, 5, 5)), Some("full"));
        assert_eq!(unavailable_reason(&room("a", "KMTRWX", 1, 1, 5)), None);
        let r = room("a", "KMTRWX", 1, 5, 5);
        assert!(entry_label(&room_label(&r), &r).ends_with("full"));
    }

    #[test]
    fn an_address_in_the_code_field_is_told_codes_replaced_it() {
        assert_eq!(bad_code_notice("192.0.2.10:7700"), ADDRESS_JOIN_RETIRED);
        assert_eq!(bad_code_notice(" 198.51.100.2:40000 "), ADDRESS_JOIN_RETIRED);
        assert_eq!(bad_code_notice("hello"), NOT_A_CODE);
        assert_eq!(bad_code_notice(""), NOT_A_CODE);
    }

    #[test]
    fn every_code_status_has_its_own_line() {
        let lines = [
            code_status_line(CodeStatus::Publishing),
            code_status_line(CodeStatus::Published { up: 2, total: 3 }),
            code_status_line(CodeStatus::LanOnly { tried: 0 }),
            code_status_line(CodeStatus::LanOnly { tried: 3 }),
        ];
        assert!(lines[1].contains("2 of your 3 relays"));
        assert!(lines[3].contains("Couldn't reach any of your relays"));
        for (i, a) in lines.iter().enumerate() {
            for b in &lines[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    /// Run one headless egui pass over `ui_fn` inside an Area (no GPU needed).
    fn run_pass<R>(ui_fn: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
            ..Default::default()
        };
        ctx.begin_pass(raw);
        let out = egui::Area::new(egui::Id::new("lan_ui_test")).show(&ctx, ui_fn).inner;
        let _ = ctx.end_pass();
        out
    }

    #[test]
    fn the_games_section_draws_in_every_state_without_a_phantom_click() {
        let populated = LanGames {
            rooms: ordered(&[room("Axo", "KMTRWX", 3, 1, 5), room("Full", "PQRSTU", 5, 5, 5)]),
            listening: true,
        };
        let empty = LanGames { rooms: vec![], listening: true };
        let deaf = LanGames { rooms: vec![], listening: false };
        for games in [&populated, &empty, &deaf] {
            assert_eq!(run_pass(|ui| draw_lan_games(ui, games)), None, "nothing clicked");
        }
    }

    #[test]
    fn the_host_panel_draws_with_and_without_an_address_in_every_status() {
        let ctx = egui::Context::default();
        let statuses = [
            CodeStatus::Publishing,
            CodeStatus::Published { up: 1, total: 3 },
            CodeStatus::LanOnly { tried: 3 },
        ];
        for access in [
            HostAccess { addresses: vec![Ipv4Addr::new(192, 0, 2, 23)] },
            HostAccess { addresses: vec![] },
        ] {
            for status in statuses {
                ctx.begin_pass(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
                    ..Default::default()
                });
                draw_host_panel(&ctx, &access, &RoomView { code: "KMT RWX".into(), status }, (1, 5));
                let _ = ctx.end_pass();
            }
        }
    }

    #[test]
    fn order_puts_joinable_first_then_name_then_code() {
        let list = vec![
            room("Zed", "ZZZZZZ", 5, 1, 5),
            room("full", "FFFFFF", 4, 5, 5),
            room("alpha", "BBBBBB", 9, 1, 5),
            room("alpha", "AAAAAA", 3, 1, 5),
        ];
        let got: Vec<String> = ordered(&list).iter().map(|r| r.code.clone()).collect();
        assert_eq!(got, ["AAAAAA", "BBBBBB", "ZZZZZZ", "FFFFFF"]);
    }
}

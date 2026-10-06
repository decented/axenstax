//! LAN play UI (gap-audit T2-10): the "Games on this network" list in the Join
//! Game dialog and the "you are hosting" panel in the pause menu.
//!
//! The wording and ordering rules are pure and unit-tested; the egui half only
//! lays them out. Discovery is **LAN broadcast only** (`discovery.rs`, UDP
//! 7705): no directory, relay or internet lookup of any kind — a central,
//! browsable list of player-run games would make AxeNStax the platform
//! (red line 1).
//!
//! NATIVE-ONLY: the web build has no multiplayer.
#![cfg(not(target_arch = "wasm32"))]

use crate::discovery::{DiscoveredServer, ServerListener};
use crate::lan_host::HostAccess;

pub const GAMES_HEADING: &str = "GAMES ON THIS NETWORK";
pub const GAMES_SEARCHING: &str =
    "Looking for games on your Wi-Fi or Ethernet. Ask a friend to press Host on their \
     world, or type their address below.";
pub const GAMES_NOT_LISTENING: &str =
    "Can't listen for games on this network (another copy of Axe'n'Stax on this computer is \
     already listening). You can still type an address below.";

/// Why a discovered game cannot be joined, shown in place of the click.
pub fn unavailable_reason(s: &DiscoveredServer) -> Option<&'static str> {
    if s.protocol_version != crate::protocol::PROTOCOL_VERSION {
        Some("different version")
    } else if s.max_players > 0 && s.player_count >= s.max_players {
        Some("full")
    } else {
        None
    }
}

/// One row's text: `World name  ·  2 of 5 players  ·  Creative`, with the
/// reason appended when the game cannot be joined.
pub fn entry_label(s: &DiscoveredServer) -> String {
    let name = if s.server_name.is_empty() { "Unnamed world" } else { s.server_name.as_str() };
    let mode = if s.is_creative { "Creative" } else { "Survival" };
    let mut out = format!("{name}  \u{b7}  {} of {} players  \u{b7}  {mode}", s.player_count, s.max_players);
    if let Some(why) = unavailable_reason(s) {
        out.push_str(&format!("  \u{b7}  {why}"));
    }
    out
}

/// The address a click joins, exactly as the manual box would hold it.
pub fn entry_address(s: &DiscoveredServer) -> String {
    s.addr.to_string()
}

/// Stable display order: joinable games first, then by name, then address — so
/// the list does not reshuffle as announcements arrive.
pub fn ordered(servers: &[DiscoveredServer]) -> Vec<DiscoveredServer> {
    let mut v = servers.to_vec();
    v.sort_by(|a, b| {
        (unavailable_reason(a).is_some(), a.server_name.to_lowercase(), a.addr)
            .cmp(&(unavailable_reason(b).is_some(), b.server_name.to_lowercase(), b.addr))
    });
    v
}

/// A frame's view of the LAN: who is announcing, and whether we can hear at all.
pub struct LanGames {
    pub servers: Vec<DiscoveredServer>,
    pub listening: bool,
}

/// Open the listener on first use, drain it, and snapshot the list. The caller
/// keeps the `Option` on its state and drops it (freeing UDP 7705) when the
/// dialog closes.
pub fn poll_games(listener: &mut Option<ServerListener>) -> LanGames {
    let l = listener.get_or_insert_with(ServerListener::new);
    l.poll();
    LanGames { servers: ordered(l.servers()), listening: l.is_listening() }
}

/// The "Games on this network" section. Returns the address of the entry the
/// player clicked, if any.
pub fn draw_lan_games(ui: &mut egui::Ui, games: &LanGames) -> Option<String> {
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
    } else if games.servers.is_empty() {
        ui.label(egui::RichText::new(GAMES_SEARCHING).size(12.0).color(egui::Color32::from_rgb(150, 150, 160)));
    } else {
        egui::ScrollArea::vertical().max_height(150.0).auto_shrink([true, true]).show(ui, |ui| {
            for s in &games.servers {
                let joinable = unavailable_reason(s).is_none();
                let text = format!("{}\n{}", entry_label(s), entry_address(s));
                let btn = egui::Button::new(egui::RichText::new(text).size(13.0))
                    .min_size(egui::vec2(360.0, 40.0))
                    .corner_radius(egui::CornerRadius::same(6));
                let resp = ui.add_enabled(joinable, btn);
                if resp.clicked() {
                    picked = Some(entry_address(s));
                }
                ui.add_space(3.0);
            }
        });
    }
    ui.add_space(12.0);
    picked
}

/// The pause-menu panel shown while this machine hosts a LAN game: where
/// friends connect, how full the world is, and a copy button.
pub fn draw_host_panel(ctx: &egui::Context, access: &HostAccess, players: (usize, usize)) {
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
                let addrs = access.join_addresses();
                if addrs.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "No network connection found. Connect to Wi-Fi or Ethernet so friends can join.",
                        )
                        .color(egui::Color32::from_rgb(230, 150, 150)),
                    );
                } else {
                    ui.label("Friends on the same network: Join Game, then pick this world or type:");
                    for a in &addrs {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(a).monospace().size(15.0).strong());
                            if ui.small_button("Copy").clicked() {
                                ui.ctx().copy_text(a.clone());
                            }
                        });
                    }
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::Instant;

    fn server(name: &str, last_octet: u8, players: u8, max: u8, version: u32) -> DiscoveredServer {
        DiscoveredServer {
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, last_octet)), 7700),
            server_name: name.to_string(),
            player_count: players,
            max_players: max,
            is_creative: false,
            protocol_version: version,
            last_seen: Instant::now(),
        }
    }

    const V: u32 = crate::protocol::PROTOCOL_VERSION;

    #[test]
    fn a_row_reads_name_players_and_mode() {
        let s = server("Axo's World", 23, 2, 5, V);
        assert_eq!(entry_label(&s), "Axo's World  \u{b7}  2 of 5 players  \u{b7}  Survival");
        assert_eq!(entry_address(&s), "192.168.1.23:7700");
        let mut c = server("Build", 24, 1, 5, V);
        c.is_creative = true;
        assert!(entry_label(&c).ends_with("Creative"));
    }

    #[test]
    fn an_unnamed_world_still_gets_a_row_label() {
        assert!(entry_label(&server("", 1, 0, 5, V)).starts_with("Unnamed world"));
    }

    #[test]
    fn full_and_mismatched_games_say_why_they_cannot_be_joined() {
        assert_eq!(unavailable_reason(&server("a", 1, 5, 5, V)), Some("full"));
        assert_eq!(unavailable_reason(&server("a", 1, 1, 5, V + 1)), Some("different version"));
        assert_eq!(unavailable_reason(&server("a", 1, 1, 5, V)), None);
        assert!(entry_label(&server("a", 1, 5, 5, V)).ends_with("full"));
        assert!(entry_label(&server("a", 1, 1, 5, V + 1)).ends_with("different version"));
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
            servers: ordered(&[server("Axo", 3, 1, 5, V), server("Old", 4, 1, 5, V + 1), server("Full", 5, 5, 5, V)]),
            listening: true,
        };
        let empty = LanGames { servers: vec![], listening: true };
        let deaf = LanGames { servers: vec![], listening: false };
        for games in [&populated, &empty, &deaf] {
            assert_eq!(run_pass(|ui| draw_lan_games(ui, games)), None, "nothing clicked");
        }
    }

    #[test]
    fn the_host_panel_draws_with_and_without_an_address() {
        let ctx = egui::Context::default();
        for access in [
            HostAccess { addresses: vec![Ipv4Addr::new(192, 168, 1, 23)], port: 7700 },
            HostAccess { addresses: vec![], port: 7700 },
        ] {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
                ..Default::default()
            });
            draw_host_panel(&ctx, &access, (1, 5));
            let _ = ctx.end_pass();
        }
    }

    #[test]
    fn order_puts_joinable_first_then_name_then_address() {
        let list = vec![
            server("Zed", 5, 1, 5, V),
            server("full", 4, 5, 5, V),
            server("alpha", 9, 1, 5, V),
            server("alpha", 3, 1, 5, V),
        ];
        let got: Vec<String> = ordered(&list).iter().map(|s| entry_address(s)).collect();
        assert_eq!(
            got,
            ["192.168.1.3:7700", "192.168.1.9:7700", "192.168.1.5:7700", "192.168.1.4:7700"]
        );
    }
}

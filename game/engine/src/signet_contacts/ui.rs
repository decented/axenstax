//! The "Signet contacts" row in the Friends column (spec §5, D6, D12, D13).
//! Copy is plain and carries no social-network framing (red line 4); every
//! string is pinned by a test below.

use super::pairing_flow::Phase;
use super::View;

pub const HEADING: &str = "Signet contacts";
pub const CONNECT_LEAD: &str = "Use the people you already know in Signet here. Signet stays in charge of the list.";
pub const CONNECT_BUTTON: &str = "Connect Signet contacts";
pub const SCAN_LINE: &str = "Scan this with your phone, or open the link on it.";
pub const COPY_LINK: &str = "Copy link";
pub const WAITING: &str = "Waiting for Signet…";
pub const CODE_LINE: &str =
    "Type this code into Signet. Press Continue only when Signet says the code matches. If it doesn't, press Cancel.";
pub const CONTINUE: &str = "Continue";
pub const CANCEL: &str = "Cancel";
pub const TIMED_OUT: &str =
    "No answer from Signet. If you've connected lots of apps, Signet may be at its limit (10).";
pub const TRY_AGAIN: &str = "Try again";
pub const SYNC_NOW: &str = "Sync now";
pub const DISCONNECT: &str = "Disconnect";
pub const DISCONNECT_HINT: &str = "Also remove \"Axe'n'Stax\" in Signet.";
pub const TRUNCATED: &str = "Your Signet list may be incomplete.";
pub const STALE: &str = "Your Signet list is out of date, so it isn't used until it syncs.";
pub const REVOKED_NOTICE: &str = "Signet disconnected this game.";
pub const UNREADABLE_SHORT: &str = "Your Signet contacts can't be read.";
pub const UNREADABLE: &str =
    "Your Signet contacts can't be read. Press Disconnect, then connect Signet again.";
/// Why a world can't be hosted. Disconnect is in the lobby at any width: in the
/// Friends column when it is drawn, and in a strip across the top when it isn't.
pub const HOST_REFUSED: &str =
    "Your Signet contacts can't be read. Press Disconnect in the lobby, then connect Signet again.";
/// What a host is told when the Signet files become unreadable mid-game.
pub const HOST_LOST_SIGNET_FRIENDS: &str =
    "Your Signet contacts can't be read, so friends who were only on your Signet list were removed from this game. Press Disconnect in the lobby, then connect Signet again.";
/// The button beside the lobby notice that says so.
pub const DISCONNECT_SIGNET: &str = "Disconnect Signet";

/// The narrowest lobby that has room for the Friends column, and so for the row
/// that holds Disconnect.
pub const FRIENDS_COLUMN_MIN_WIDTH: f32 = 1360.0;

/// Whether the lobby has to draw its own way out of an unreadable Signet: the
/// Friends column holds Disconnect, and below the width that has room for it
/// nothing else would.
pub fn needs_strip(lobby_width: f32, unreadable: bool) -> bool {
    unreadable && lobby_width < FRIENDS_COLUMN_MIN_WIDTH
}

/// Whether a lobby notice is the refusal that Disconnect answers.
pub fn is_host_refusal(notice: &str) -> bool {
    notice == HOST_REFUSED
}

/// A narrow lobby's way out of an unreadable Signet. Returns true once the player
/// has pressed Disconnect.
pub fn draw_unreadable_strip(ui: &mut egui::Ui) -> bool {
    let mut pressed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(dim(UNREADABLE_SHORT));
        if ui.small_button(DISCONNECT).clicked() {
            super::disconnect();
            pressed = true;
        }
    });
    pressed
}

/// "synced 2 min ago" — coarse on purpose.
pub fn ago(then: u64, now: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// The connected status line.
pub fn connected_line(people: usize, last_synced: Option<u64>, now: u64) -> String {
    let who = if people == 1 { "1 contact".to_owned() } else { format!("{people} contacts") };
    match last_synced {
        Some(t) => format!("Connected · {who} · synced {}", ago(t, now)),
        None => format!("Connected · {who} · not synced yet"),
    }
}

/// Whether the row offers Disconnect. A grant that can't be read isn't connected
/// in any useful sense, but Disconnect is the way out of that.
pub fn offers_disconnect(connected: bool, unreadable: bool) -> bool {
    connected || unreadable
}

fn dim(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(11.0).color(egui::Color32::from_rgb(150, 150, 160))
}

/// Draw the row. Calls straight into the service; never blocks.
/// `player_relays` is the player's "Your relays" list — a new pairing asks
/// Signet to answer on its first relay (D4).
pub fn draw(ui: &mut egui::Ui, player_relays: &[String]) {
    super::note_column_drawn();
    let view: View = super::view();
    let now = super::unix_now();

    ui.label(egui::RichText::new(HEADING).size(14.0).strong());
    if let Some(n) = view.notice {
        ui.horizontal(|ui| {
            ui.label(dim(n));
            if ui.small_button("OK").clicked() {
                super::dismiss_notice();
            }
        });
    }

    match view.phase.unwrap_or(Phase::Idle) {
        Phase::Waiting { carrier } => {
            ui.label(dim(SCAN_LINE));
            crate::menu::draw_qr(ui, &carrier, 200.0);
            ui.horizontal(|ui| {
                if ui.small_button(COPY_LINK).clicked() {
                    ui.ctx().copy_text(carrier.clone());
                }
                if ui.small_button(CANCEL).clicked() {
                    super::cancel_pairing();
                }
            });
            ui.label(dim(WAITING));
        }
        Phase::Code { code } => {
            ui.label(egui::RichText::new(code).size(30.0).strong().monospace());
            ui.label(dim(CODE_LINE));
            ui.horizontal(|ui| {
                if ui.button(CONTINUE).clicked() {
                    super::continue_pairing();
                }
                if ui.small_button(CANCEL).clicked() {
                    super::cancel_pairing();
                }
            });
        }
        Phase::TimedOut => {
            ui.label(dim(TIMED_OUT));
            ui.horizontal(|ui| {
                if ui.small_button(TRY_AGAIN).clicked() {
                    super::start_pairing(player_relays);
                }
                if ui.small_button(CANCEL).clicked() {
                    super::cancel_pairing();
                }
            });
        }
        Phase::Idle if offers_disconnect(view.connected, view.unreadable) => {
            if view.connected {
                ui.label(dim(&connected_line(view.people, view.last_synced, now)));
            }
            if view.unreadable {
                ui.label(dim(UNREADABLE));
            }
            if view.stale {
                ui.label(dim(STALE));
            }
            if view.truncated {
                ui.label(dim(TRUNCATED));
            }
            ui.horizontal(|ui| {
                if view.connected && ui.small_button(SYNC_NOW).clicked() {
                    super::sync_now();
                }
                if ui.small_button(DISCONNECT).clicked() {
                    super::disconnect();
                }
            });
            ui.label(dim(DISCONNECT_HINT));
        }
        Phase::Idle => {
            ui.label(dim(CONNECT_LEAD));
            if ui.button(CONNECT_BUTTON).clicked() {
                super::start_pairing(player_relays);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&str; 24] = [
        HEADING, CONNECT_LEAD, CONNECT_BUTTON, SCAN_LINE, COPY_LINK, WAITING, CODE_LINE, CONTINUE,
        CANCEL, TIMED_OUT, TRY_AGAIN, SYNC_NOW, DISCONNECT, DISCONNECT_HINT, TRUNCATED, STALE,
        REVOKED_NOTICE, UNREADABLE, HOST_REFUSED, "Connected", "contacts", UNREADABLE_SHORT,
        DISCONNECT_SIGNET, HOST_LOST_SIGNET_FRIENDS,
    ];

    #[test]
    fn the_copy_carries_no_social_discovery_or_money_words() {
        let banned = [
            "social", "network", "follow", "discover", "browse", "find friends", "meet", "chat",
            "public", "stranger",
        ];
        for s in ALL {
            let l = s.to_lowercase();
            for b in banned {
                assert!(!l.contains(b), "{s:?} contains {b:?}");
            }
            // Money words: the shared `copy_lint::BANNED_MONEY_WORDS`.
            let money = crate::copy_lint::banned_in(s, crate::copy_lint::BANNED_MONEY_WORDS);
            assert!(money.is_empty(), "{s:?} contains {money:?}");
        }
    }

    #[test]
    fn an_unreadable_signet_always_has_a_way_out_in_the_lobby() {
        // Below the Friends column's width the strip is drawn; at or above it the
        // column's own row is. There is no width with neither.
        for width in [320.0, 800.0, 1280.0, 1359.9, FRIENDS_COLUMN_MIN_WIDTH, 1920.0, 3840.0] {
            let column = width >= FRIENDS_COLUMN_MIN_WIDTH;
            assert!(
                column || needs_strip(width, true),
                "no Disconnect at {width} wide"
            );
            assert!(!(column && needs_strip(width, true)), "two at {width} wide");
            assert!(!needs_strip(width, false), "a readable Signet needs no strip at {width}");
        }
    }

    #[test]
    fn the_host_refusal_is_the_notice_that_offers_disconnect() {
        assert!(is_host_refusal(HOST_REFUSED));
        assert!(!is_host_refusal("The connection was closed."));
        // It must not point at a column that may not be drawn.
        assert!(!HOST_REFUSED.contains("Friends"));
    }

    #[test]
    fn the_d13_lines_are_the_spec_wording() {
        assert_eq!(TIMED_OUT, "No answer from Signet. If you've connected lots of apps, Signet may be at its limit (10).");
        assert_eq!(TRUNCATED, "Your Signet list may be incomplete.");
        assert_eq!(
            CODE_LINE,
            "Type this code into Signet. Press Continue only when Signet says the code matches. If it doesn't, press Cancel."
        );
    }

    #[test]
    fn the_connected_line_reads_plainly() {
        assert_eq!(connected_line(3, Some(1000), 1130), "Connected · 3 contacts · synced 2 min ago");
        assert_eq!(connected_line(1, Some(1000), 1010), "Connected · 1 contact · synced just now");
        assert_eq!(connected_line(0, None, 5), "Connected · 0 contacts · not synced yet");
        assert_eq!(ago(0, 7200), "2 h ago");
        assert_eq!(ago(0, 3 * 86_400), "3 days ago");
    }

    #[test]
    fn column_open_sync_is_debounced_to_a_minute() {
        assert!(crate::signet_contacts::column_sync_due(None, 100));
        assert!(!crate::signet_contacts::column_sync_due(Some(100), 159));
        assert!(crate::signet_contacts::column_sync_due(Some(100), 160));
    }
}

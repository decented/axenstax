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
        Phase::Idle if view.connected => {
            ui.label(dim(&connected_line(view.people, view.last_synced, now)));
            if view.stale {
                ui.label(dim(STALE));
            }
            if view.truncated {
                ui.label(dim(TRUNCATED));
            }
            ui.horizontal(|ui| {
                if ui.small_button(SYNC_NOW).clicked() {
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

    const ALL: [&str; 19] = [
        HEADING, CONNECT_LEAD, CONNECT_BUTTON, SCAN_LINE, COPY_LINK, WAITING, CODE_LINE, CONTINUE,
        CANCEL, TIMED_OUT, TRY_AGAIN, SYNC_NOW, DISCONNECT, DISCONNECT_HINT, TRUNCATED, STALE,
        REVOKED_NOTICE, "Connected", "contacts",
    ];

    #[test]
    fn the_copy_carries_no_social_discovery_or_money_words() {
        let banned = [
            "social", "network", "follow", "discover", "browse", "find friends", "meet", "chat",
            "public", "earn", "bitcoin", "sats", "money", "stranger",
        ];
        for s in ALL {
            let l = s.to_lowercase();
            for b in banned {
                assert!(!l.contains(b), "{s:?} contains {b:?}");
            }
        }
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

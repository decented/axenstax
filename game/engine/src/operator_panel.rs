#![cfg(not(target_arch = "wasm32"))]
//! In-game Operator panel (Spec B task 7) — renders a received `ConsoleSnapshot`.
//!
//! Native (the desktop operator connected to their own server); the web-operator
//! path is a follow-up. **Owner boundary:** the live 2-machine snapshot delivery
//! and the visual feel are verified by the owner — this is a minimal, honest
//! render of the read-model.

use crate::console_snapshot::ConsoleSnapshot;

/// Draw the Operator panel for the latest received snapshot.
pub fn draw_operator_panel(ui: &mut egui::Ui, snap: &ConsoleSnapshot) {
    ui.heading("Operator Console");
    ui.label(format!(
        "{}  ({}/{} players)",
        snap.server_name, snap.players_cur, snap.players_max
    ));
    ui.separator();
    ui.label(format!(
        "Unique today: {}    Peak: {}    Sessions: {}",
        snap.unique_today, snap.peak_today, snap.total_sessions
    ));
    ui.label(format!(
        "Allowlist: {}    Blocklist: {}    Sign-in required: {}",
        allowlist_text(snap),
        snap.blocklist_count,
        snap.require_signin
    ));
    ui.label(format!(
        "Announcing: {}    Privacy: {}",
        snap.announce,
        if snap.privacy_level.is_empty() {
            "—"
        } else {
            snap.privacy_level.as_str()
        }
    ));
    ui.separator();
    ui.label("Connected:");
    if snap.roster.is_empty() {
        ui.label("  (nobody right now)");
    }
    for p in &snap.roster {
        let who = if p.handle.is_empty() {
            p.npub.clone()
        } else {
            format!("{} ({})", p.handle, p.npub)
        };
        ui.label(format!("  {who}"));
    }
}

/// How the panel words the allowlist: an open server and a closed one both list
/// nobody, so the count alone can't say which.
fn allowlist_text(snap: &ConsoleSnapshot) -> String {
    match (snap.allowlist_on, snap.allowlist_count) {
        (false, _) => "off (open)".to_string(),
        (true, 0) => "on, nobody listed (nobody can join)".to_string(),
        (true, n) => format!("on, {n} listed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(on: bool, count: u32) -> ConsoleSnapshot {
        ConsoleSnapshot { allowlist_on: on, allowlist_count: count, ..Default::default() }
    }

    #[test]
    fn the_panel_tells_an_open_server_from_a_closed_one() {
        assert_eq!(allowlist_text(&snap(false, 0)), "off (open)");
        assert_eq!(allowlist_text(&snap(true, 0)), "on, nobody listed (nobody can join)");
        assert_eq!(allowlist_text(&snap(true, 3)), "on, 3 listed");
    }
}

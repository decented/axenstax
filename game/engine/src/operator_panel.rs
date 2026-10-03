#![cfg(not(target_arch = "wasm32"))]
#![allow(dead_code)] // rendered by the in-game HUD once the snapshot route lands (B-7a)
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
        snap.allowlist_count, snap.blocklist_count, snap.require_signin
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

//! Challenge board (feature-coverage Phase 6). Solo Buildout Wave 6.
//!
//! The in-game **menu** for the already-built challenge system (`scenario.rs`):
//! the J-key board lists every bundled challenge (onboarding arc + explorer
//! cards) and starts one on click — the graphical successor to the `/scenario
//! list` chat stopgap. Pure render: it returns which challenge to start (by its
//! `scenario` name) + a close request; the game loop owns the provisioning
//! (`GameState::start_scenario`) so the board and the `/scenario` command can't
//! drift.

use egui::RichText;

/// What the board is asking the game loop to do this frame.
#[derive(Default)]
pub struct ChallengeBoardResult {
    /// The challenge `name` (the `/scenario <name>` key) the player chose to
    /// start, if any.
    pub start: Option<String>,
    pub close_requested: bool,
}

/// One row of the board, as the caller supplies it.
pub struct ChallengeRow {
    /// The `/scenario <name>` key (stable id used to start it).
    pub name: &'static str,
    /// The human display name.
    pub display: String,
}

/// Render the challenge board.
///
/// `rows` is the bundled challenge listing (`scenario::challenge_listing`).
/// `active` describes the running challenge, if one is live:
/// `(display_name, (done, total) leaves, elapsed_seconds)`.
pub fn show_challenge_board(
    ctx: &egui::Context,
    rows: &[ChallengeRow],
    active: Option<(&str, (u32, u32), u32)>,
) -> ChallengeBoardResult {
    let mut result = ChallengeBoardResult::default();
    let mut window_open = true;

    egui::Window::new("Challenges")
        .id(egui::Id::new("challenge_board"))
        .open(&mut window_open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(RichText::new("Pick a challenge").size(16.0).strong());
            ui.add_space(2.0);
            ui.label(RichText::new("Press J to close. Progress shows for the active one.").weak().size(11.0));
            ui.add_space(8.0);

            // Active challenge banner (if any).
            if let Some((name, (done, total), secs)) = active {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new(format!("▶ Active: {name}")).strong().color(GOLD));
                    if total > 0 {
                        ui.label(format!("Progress: {done} / {total}"));
                    }
                    ui.label(RichText::new(format!("{secs}s elapsed")).weak().size(11.0));
                });
                ui.add_space(8.0);
            }

            let active_display = active.map(|(n, _, _)| n);
            for row in rows {
                ui.horizontal(|ui| {
                    let is_active = active_display == Some(row.display.as_str());
                    ui.label(RichText::new(&row.display).strong());
                    // Push the control to the right.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if is_active {
                            ui.label(RichText::new("● running").color(GOLD).size(12.0));
                        } else if ui.button("Start").clicked() {
                            result.start = Some(row.name.to_string());
                        }
                    });
                });
                ui.separator();
            }
        });

    if !window_open {
        result.close_requested = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        result.close_requested = true;
    }
    result
}

const GOLD: egui::Color32 = egui::Color32::from_rgb(255, 210, 120);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_build_from_the_scenario_listing() {
        // The board's rows are exactly the bundled challenge listing — onboarding
        // first, then the explorer cards, each with a startable name.
        let listing = crate::scenario::challenge_listing();
        assert!(!listing.is_empty());
        let rows: Vec<ChallengeRow> = listing
            .iter()
            .map(|(name, display)| ChallengeRow { name, display: display.clone() })
            .collect();
        assert_eq!(rows.len(), listing.len());
        // Every row's name resolves to a real def (the Start path won't dead-end).
        for r in &rows {
            assert!(
                crate::scenario::named_builtin_def(r.name).is_some(),
                "challenge row '{}' must resolve to a def",
                r.name
            );
        }
    }
}

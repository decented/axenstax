//! Spec 33 Mob Bounty Board — claim dialog.
//!
//! Right-click on a BOUNTY_BOARD opens this overlay. One row per
//! active bounty: label, kills/required, payout, Claim button. The
//! Claim button is enabled iff the player has accumulated enough
//! kills AND hasn't already claimed the bounty id this rotation.
//!
//! Pure render — no state mutation. The caller (game_loop) reads the
//! outcome and applies side-effects via `bounty::try_claim` + the
//! sats payout pipeline.

use crate::bounty::ActiveBounty;
use crate::screen::ViewportRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BountyUiOutcome {
    InProgress,
    Closed,
    /// Player clicked Claim on the bounty with this id. Caller routes
    /// through `bounty::try_claim` + the payout pipeline.
    Claim { bounty_id: u32 },
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOUR: egui::Color32 = egui::Color32::from_rgb(255, 195, 100);
const DIM_COLOUR: egui::Color32 = egui::Color32::LIGHT_GRAY;
const DISABLED_COLOUR: egui::Color32 = egui::Color32::from_rgb(120, 120, 120);

/// Draw the bounty dialog for one player. Returns the outcome of the
/// player's interaction this frame.
pub fn draw_bounty_ui(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    bounties: &[ActiveBounty],
    player_kill_counter: &ahash::AHashMap<crate::mob::MobType, u32>,
    player_claimed: &ahash::AHashMap<u32, u32>,
    bitcoin_enabled: bool,
    charter_allows_sats: bool,
) -> BountyUiOutcome {
    let mut outcome = BountyUiOutcome::InProgress;

    // Dim background.
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("bounty_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let window_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 220.0,
        viewport.y as f32 + viewport.height as f32 / 4.0,
    );

    egui::Area::new(egui::Id::new(("bounty_window", player_index)))
        .fixed_pos(window_origin)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(20, 16, 12, 240))
                .show(ui, |ui| {
                    ui.set_min_width(440.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Bounty Board")
                                .size(20.0)
                                .color(TITLE_COLOUR)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new("Refreshes at dawn")
                                .size(12.0)
                                .italics()
                                .color(DIM_COLOUR),
                        );
                        ui.add_space(8.0);

                        if bounties.is_empty() {
                            ui.label(
                                egui::RichText::new("(No bounties posted — check back at dawn)")
                                    .color(DIM_COLOUR),
                            );
                        } else {
                            for bounty in bounties {
                                let Some(template) = bounty.template() else { continue };
                                let kills = *player_kill_counter
                                    .get(&template.mob_kind)
                                    .unwrap_or(&0);
                                let already_claimed = player_claimed.contains_key(&bounty.id);
                                let can_claim =
                                    !already_claimed && kills >= template.required_count;
                                ui.horizontal(|ui| {
                                    // Bounty label + counter.
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(template.label)
                                                .size(15.0)
                                                .color(TITLE_COLOUR)
                                                .strong(),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "Progress: {kills} / {}",
                                                template.required_count
                                            ))
                                            .size(12.0)
                                            .color(if kills >= template.required_count {
                                                TITLE_COLOUR
                                            } else {
                                                DIM_COLOUR
                                            }),
                                        );
                                    });
                                    ui.add_space(20.0);
                                    // Payout (sats OR rep, depending on policy).
                                    let payout_label =
                                        if bitcoin_enabled && charter_allows_sats {
                                            format!("{} sats", template.payout_sats)
                                        } else {
                                            format!("+{} village rep", template.fallback_rep)
                                        };
                                    ui.label(
                                        egui::RichText::new(payout_label)
                                            .size(13.0)
                                            .color(DIM_COLOUR),
                                    );
                                    ui.add_space(20.0);
                                    // Claim button — disabled when not eligible.
                                    let button_text = if already_claimed {
                                        "Claimed"
                                    } else if !can_claim {
                                        "Locked"
                                    } else {
                                        "Claim"
                                    };
                                    let mut button = egui::Button::new(
                                        egui::RichText::new(button_text)
                                            .color(if can_claim {
                                                egui::Color32::WHITE
                                            } else {
                                                DISABLED_COLOUR
                                            }),
                                    );
                                    if !can_claim {
                                        button = button.fill(egui::Color32::from_gray(40));
                                    }
                                    let resp = ui.add_enabled(can_claim, button);
                                    if resp.clicked() && can_claim {
                                        outcome = BountyUiOutcome::Claim { bounty_id: bounty.id };
                                    }
                                });
                                ui.separator();
                            }
                        }

                        ui.add_space(6.0);
                        // Close button (Esc / E also closes — handled
                        // by the game-loop dialog-close gate).
                        if ui.button("Close").clicked() {
                            outcome = BountyUiOutcome::Closed;
                        }
                    });
                });
        });

    outcome
}

#[cfg(test)]
mod tests {
    // The UI is render-only; behaviour-level coverage lives in the
    // pure `bounty::try_claim` helper + the integration tests in
    // test_integration/bounty.rs. No tests here.
}

//! Spec 39 Bazaar — sell dialog.
//!
//! Right-click a Bazaar Block → shows the held stack's trade-value
//! floor + a Sell button. Pure render; the game-loop applies the sell
//! (consume stack + payout).

use crate::screen::ViewportRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BazaarUiOutcome {
    InProgress,
    Closed,
    /// Player clicked Sell on the held stack.
    Sell,
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOUR: egui::Color32 = egui::Color32::from_rgb(110, 200, 170);
const DIM_COLOUR: egui::Color32 = egui::Color32::LIGHT_GRAY;
const DISABLED_COLOUR: egui::Color32 = egui::Color32::from_rgb(120, 120, 120);

pub fn draw_bazaar_ui(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    held_label: Option<&str>,
    held_count: u8,
    quote: Option<u64>,
    bitcoin_enabled: bool,
    charter_allows_sats: bool,
    vow_suppresses: bool,
) -> BazaarUiOutcome {
    let mut outcome = BazaarUiOutcome::InProgress;

    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("bazaar_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 200.0,
        viewport.y as f32 + viewport.height as f32 / 3.0,
    );

    egui::Area::new(egui::Id::new(("bazaar_window", player_index)))
        .fixed_pos(origin)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(14, 22, 18, 240))
                .show(ui, |ui| {
                    ui.set_min_width(400.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Bazaar")
                                .size(20.0).color(TITLE_COLOUR).strong(),
                        );
                        ui.label(
                            egui::RichText::new("Sell anything at the trade-value floor")
                                .size(12.0).italics().color(DIM_COLOUR),
                        );
                        ui.add_space(8.0);

                        let tips_off = !bitcoin_enabled || !charter_allows_sats || vow_suppresses;
                        if tips_off {
                            let why = if !bitcoin_enabled {
                                "Bitcoin disabled on this server."
                            } else if !charter_allows_sats {
                                "Selling requires Bitcoin enabled for this account."
                            } else {
                                "The Nostrich's Vow suppresses sats."
                            };
                            ui.label(egui::RichText::new(why).size(12.0).italics().color(DIM_COLOUR));
                            ui.add_space(4.0);
                        }

                        match (held_label, quote) {
                            (Some(name), Some(q)) => {
                                ui.label(
                                    egui::RichText::new(format!("{name} ×{held_count}"))
                                        .size(15.0).color(TITLE_COLOUR),
                                );
                                ui.label(
                                    egui::RichText::new(format!("Sells for {q} sats"))
                                        .size(13.0).color(DIM_COLOUR),
                                );
                                ui.add_space(6.0);
                                let can_sell = !tips_off;
                                let btn = egui::Button::new(
                                    egui::RichText::new("Sell").color(
                                        if can_sell { egui::Color32::WHITE } else { DISABLED_COLOUR },
                                    ),
                                );
                                if ui.add_enabled(can_sell, btn).clicked() {
                                    outcome = BazaarUiOutcome::Sell;
                                }
                            }
                            _ => {
                                ui.label(
                                    egui::RichText::new(
                                        "Hold a stack to sell it (anything tradeable).",
                                    )
                                    .size(13.0).color(DIM_COLOUR),
                                );
                            }
                        }

                        ui.add_space(6.0);
                        if ui.button("Close").clicked() {
                            outcome = BazaarUiOutcome::Closed;
                        }
                    });
                });
        });

    outcome
}

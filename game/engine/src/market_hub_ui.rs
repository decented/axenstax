//! Spec 37 Market Hub — directory panel.
//!
//! Right-click a Market Bell → a read-only list of every Vendor Block
//! within the hub radius: item, price, owner, stock. v1 doesn't buy
//! remotely (walk to the vendor); the panel is pure discovery.
//!
//! Pure render — no state mutation.

use crate::market_hub::VendorListing;
use crate::screen::ViewportRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketHubUiOutcome {
    InProgress,
    Closed,
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOUR: egui::Color32 = egui::Color32::from_rgb(220, 175, 80);
const DIM_COLOUR: egui::Color32 = egui::Color32::LIGHT_GRAY;

/// Draw the market directory. `listings` is pre-computed by the caller
/// via `market_hub::vendors_in_hub`. `bitcoin_enabled` toggles whether
/// prices show as sats or as a barter hint.
pub fn draw_market_hub_ui(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    listings: &[VendorListing],
    bitcoin_enabled: bool,
) -> MarketHubUiOutcome {
    let mut outcome = MarketHubUiOutcome::InProgress;

    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("market_hub_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let window_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 240.0,
        viewport.y as f32 + viewport.height as f32 / 5.0,
    );

    egui::Area::new(egui::Id::new(("market_hub_window", player_index)))
        .fixed_pos(window_origin)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(22, 18, 12, 240))
                .show(ui, |ui| {
                    ui.set_min_width(480.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Market Hub — Directory")
                                .size(20.0)
                                .color(TITLE_COLOUR)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "{} vendor(s) in range — walk over to buy",
                                listings.len(),
                            ))
                            .size(12.0)
                            .italics()
                            .color(DIM_COLOUR),
                        );
                        ui.add_space(8.0);

                        if listings.is_empty() {
                            ui.label(
                                egui::RichText::new(
                                    "(No vendors here yet — place Vendor Blocks nearby.)",
                                )
                                .color(DIM_COLOUR),
                            );
                        } else {
                            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                for l in listings {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(&l.item_name)
                                                .size(14.0)
                                                .color(TITLE_COLOUR),
                                        );
                                        ui.add_space(12.0);
                                        let price = if bitcoin_enabled {
                                            format!("{} sats", l.price_sats)
                                        } else {
                                            "barter".to_string()
                                        };
                                        ui.label(
                                            egui::RichText::new(price).size(13.0).color(DIM_COLOUR),
                                        );
                                        ui.add_space(12.0);
                                        ui.label(
                                            egui::RichText::new(format!("x{}", l.stock))
                                                .size(13.0)
                                                .color(DIM_COLOUR),
                                        );
                                        ui.add_space(12.0);
                                        ui.label(
                                            egui::RichText::new(&l.owner_label)
                                                .size(12.0)
                                                .color(DIM_COLOUR),
                                        );
                                    });
                                    ui.separator();
                                }
                            });
                        }

                        ui.add_space(6.0);
                        if ui.button("Close").clicked() {
                            outcome = MarketHubUiOutcome::Closed;
                        }
                    });
                });
        });

    outcome
}

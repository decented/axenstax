//! Spec 38 Auction — configure / bid / status dialog.
//!
//! Three views chosen by ownership + state:
//! - Owner, un-started: Set Lot (from held) + reserve +/- + Start.
//! - Owner, active/settled: status (high bid, time left) + Withdraw.
//! - Bidder: high bid + time left + preset Bid buttons.
//!
//! Pure render — no state mutation. The game-loop applies the outcome.

use crate::auction::AuctionData;
use crate::screen::ViewportRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuctionUiOutcome {
    InProgress,
    Closed,
    /// Owner: move the held item into the lot slot.
    SetLot,
    /// Owner: adjust the reserve by this delta (clamped ≥ 0 by caller).
    ReserveAdjust(i64),
    /// Owner: start the countdown (default duration).
    Start,
    /// Bidder: place this bid.
    Bid { amount: u64 },
    /// Owner: withdraw accrued escrow proceeds.
    Withdraw,
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOUR: egui::Color32 = egui::Color32::from_rgb(210, 175, 110);
const DIM_COLOUR: egui::Color32 = egui::Color32::LIGHT_GRAY;
const DISABLED_COLOUR: egui::Color32 = egui::Color32::from_rgb(120, 120, 120);

/// Bid presets, as increments over the current high bid.
const BID_STEPS: &[u64] = &[1, 5, 25, 100];
/// Reserve adjust step.
const RESERVE_STEP: i64 = 10;

pub fn draw_auction_ui(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    is_owner: bool,
    data: &AuctionData,
    lot_label: Option<&str>,
    held_label: Option<&str>,
    time_left_secs: u64,
    bitcoin_enabled: bool,
    charter_allows_sats: bool,
    vow_suppresses: bool,
) -> AuctionUiOutcome {
    let mut outcome = AuctionUiOutcome::InProgress;

    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("auction_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 210.0,
        viewport.y as f32 + viewport.height as f32 / 4.0,
    );

    egui::Area::new(egui::Id::new(("auction_window", player_index)))
        .fixed_pos(origin)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(20, 17, 12, 240))
                .show(ui, |ui| {
                    ui.set_min_width(420.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Auction")
                                .size(20.0)
                                .color(TITLE_COLOUR)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let lot_str = lot_label.unwrap_or("(no lot set)");
                        ui.label(
                            egui::RichText::new(format!("Lot: {lot_str}"))
                                .size(14.0)
                                .color(TITLE_COLOUR),
                        );

                        if is_owner && !data.started {
                            // ── Owner config view ──
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                let held = held_label.unwrap_or("(nothing held)");
                                if ui.button(format!("Set lot from held: {held}")).clicked()
                                    && held_label.is_some()
                                {
                                    outcome = AuctionUiOutcome::SetLot;
                                }
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("Reserve: {} sats", data.reserve_sats))
                                        .color(DIM_COLOUR),
                                );
                                if ui.button(format!("-{RESERVE_STEP}")).clicked() {
                                    outcome = AuctionUiOutcome::ReserveAdjust(-RESERVE_STEP);
                                }
                                if ui.button(format!("+{RESERVE_STEP}")).clicked() {
                                    outcome = AuctionUiOutcome::ReserveAdjust(RESERVE_STEP);
                                }
                            });
                            ui.add_space(6.0);
                            let can_start = data.lot.is_some();
                            let start_btn = egui::Button::new(
                                egui::RichText::new("Start auction").color(
                                    if can_start { egui::Color32::WHITE } else { DISABLED_COLOUR },
                                ),
                            );
                            if ui.add_enabled(can_start, start_btn).clicked() {
                                outcome = AuctionUiOutcome::Start;
                            }
                            if !can_start {
                                ui.label(
                                    egui::RichText::new("Set a lot before starting.")
                                        .size(11.0).italics().color(DIM_COLOUR),
                                );
                            }
                        } else {
                            // ── Active / settled status (shared) ──
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "Reserve: {} sats", data.reserve_sats,
                                ))
                                .size(12.0).color(DIM_COLOUR),
                            );
                            ui.label(
                                egui::RichText::new(if data.high_bidder.is_some() {
                                    format!("High bid: {} sats", data.high_bid)
                                } else {
                                    "No bids yet".to_string()
                                })
                                .size(14.0).color(TITLE_COLOUR),
                            );
                            if data.settled {
                                ui.label(
                                    egui::RichText::new("Auction ended.")
                                        .size(13.0).italics().color(DIM_COLOUR),
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new(format!("Time left: {time_left_secs}s"))
                                        .size(13.0).color(DIM_COLOUR),
                                );
                            }
                            ui.add_space(6.0);

                            if is_owner {
                                // Owner: withdraw proceeds.
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Proceeds: {} sats", data.escrow_sats,
                                    ))
                                    .size(12.0).color(DIM_COLOUR),
                                );
                                let can_wd = data.escrow_sats > 0;
                                let wd = egui::Button::new(
                                    egui::RichText::new("Withdraw").color(
                                        if can_wd { egui::Color32::WHITE } else { DISABLED_COLOUR },
                                    ),
                                );
                                if ui.add_enabled(can_wd, wd).clicked() {
                                    outcome = AuctionUiOutcome::Withdraw;
                                }
                            } else if !data.settled {
                                // Bidder: preset bids over the current high.
                                let tips_off = !bitcoin_enabled || !charter_allows_sats || vow_suppresses;
                                if tips_off {
                                    ui.label(
                                        egui::RichText::new(
                                            "Bidding requires Bitcoin enabled for this account.",
                                        )
                                        .size(12.0).italics().color(DIM_COLOUR),
                                    );
                                }
                                ui.horizontal(|ui| {
                                    let base = data.high_bid.max(data.reserve_sats);
                                    for &step in BID_STEPS {
                                        let amount = base + step;
                                        let btn = egui::Button::new(
                                            egui::RichText::new(format!("Bid {amount}")).color(
                                                if tips_off { DISABLED_COLOUR } else { egui::Color32::WHITE },
                                            ),
                                        );
                                        if ui.add_enabled(!tips_off, btn).clicked() {
                                            outcome = AuctionUiOutcome::Bid { amount };
                                        }
                                    }
                                });
                            }
                        }

                        ui.add_space(6.0);
                        if ui.button("Close").clicked() {
                            outcome = AuctionUiOutcome::Closed;
                        }
                    });
                });
        });

    outcome
}

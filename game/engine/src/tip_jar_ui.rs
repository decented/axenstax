//! Spec 34 Tip Jar — tip + withdraw dialog.
//!
//! Two views, chosen by `is_owner`:
//! - Non-owner: preset-amount buttons (1 / 5 / 25 / 100 sats) +
//!   Charter / Bitcoin gating tooltip.
//! - Owner: lifetime total + escrow display + Withdraw button.
//!
//! Pure render — no state mutation. The caller (game_loop) reads
//! the outcome and applies side-effects via `tip_jar::try_tip` +
//! `tip_jar::try_withdraw` + the sats payout pipeline.

use crate::screen::ViewportRect;
use crate::tip_jar::TipJarData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipJarUiOutcome {
    InProgress,
    Closed,
    /// Non-owner clicked a preset-amount tip button. Caller routes
    /// through `tip_jar::try_tip` + `economy::apply_sats_payout`.
    Tip { amount_sats: u64 },
    /// Owner clicked Withdraw. Caller calls `tip_jar::try_withdraw`
    /// and credits the returned amount to the player's sats balance.
    Withdraw,
}

/// v1 preset amounts — small enough that a kid can tip from the
/// proof-of-play floor (a few stone strikes' worth of sats).
const PRESET_AMOUNTS: &[u64] = &[1, 5, 25, 100];

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOUR: egui::Color32 = egui::Color32::from_rgb(220, 175, 30); // gold rim
const DIM_COLOUR: egui::Color32 = egui::Color32::LIGHT_GRAY;
const DISABLED_COLOUR: egui::Color32 = egui::Color32::from_rgb(120, 120, 120);

/// Owner label for the dialog header. For `LocalPlayer(0)` returns
/// "Player 1"; for an Npub owner returns the truncated npub. None
/// (shouldn't happen — place-time stamps it) becomes a placeholder.
fn owner_label(data: &TipJarData) -> String {
    match &data.owner {
        Some(crate::tip_jar::TipJarOwner::LocalPlayer(p)) => {
            format!("Player {}", p + 1)
        }
        Some(crate::tip_jar::TipJarOwner::Npub(n)) => {
            // Truncate to first 10 chars of the bech32 string.
            let head: String = n.chars().take(10).collect();
            format!("{head}…")
        }
        None => "(no owner)".to_string(),
    }
}

/// The tipper's balance line — `None` (no line at all) when the balance
/// is unknown. There is no real wallet on alpha, so the caller passes
/// `None`; a sentinel is never shown as a balance (audit 2026-09-27).
pub fn balance_line(balance: Option<u64>) -> Option<String> {
    balance.map(|b| format!("Your balance: {b} sats"))
}

/// Is a preset tip button live? Never when tipping is disabled; with a
/// known balance, only when it covers the amount.
pub fn tip_button_enabled(tips_disabled: bool, balance: Option<u64>, amount: u64) -> bool {
    !tips_disabled && balance.is_none_or(|b| b >= amount)
}

/// Render the Tip Jar dialog. Returns the outcome of the player's
/// interaction this frame.
pub fn draw_tip_jar_ui(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    is_owner: bool,
    data: &TipJarData,
    player_sats_balance: Option<u64>,
    bitcoin_enabled: bool,
    charter_allows_sats: bool,
    vow_suppresses_sats: bool,
) -> TipJarUiOutcome {
    let mut outcome = TipJarUiOutcome::InProgress;

    // Dim background.
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("tip_jar_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let window_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 200.0,
        viewport.y as f32 + viewport.height as f32 / 3.0,
    );

    let header_label = if is_owner {
        "Your Tip Jar".to_string()
    } else {
        format!("Tip Jar — {}", owner_label(data))
    };

    egui::Area::new(egui::Id::new(("tip_jar_window", player_index)))
        .fixed_pos(window_origin)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(20, 16, 12, 240))
                .show(ui, |ui| {
                    ui.set_min_width(400.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(header_label)
                                .size(20.0)
                                .color(TITLE_COLOUR)
                                .strong(),
                        );
                        ui.add_space(8.0);

                        if is_owner {
                            // Owner view: counters + Withdraw button.
                            ui.label(
                                egui::RichText::new(format!(
                                    "In jar: {} sats", data.escrow_sats,
                                ))
                                .size(15.0)
                                .color(TITLE_COLOUR),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "Lifetime received: {} sats",
                                    data.lifetime_tips_received,
                                ))
                                .size(12.0)
                                .color(DIM_COLOUR),
                            );
                            ui.add_space(10.0);
                            let can_withdraw = data.escrow_sats > 0;
                            let mut btn = egui::Button::new(
                                egui::RichText::new("Withdraw").color(
                                    if can_withdraw {
                                        egui::Color32::WHITE
                                    } else {
                                        DISABLED_COLOUR
                                    },
                                ),
                            );
                            if !can_withdraw {
                                btn = btn.fill(egui::Color32::from_gray(40));
                            }
                            if ui.add_enabled(can_withdraw, btn).clicked() {
                                outcome = TipJarUiOutcome::Withdraw;
                            }
                        } else {
                            // Tipper view: preset amount buttons + status line.
                            let tips_disabled =
                                !bitcoin_enabled || !charter_allows_sats || vow_suppresses_sats;
                            let status_line: Option<&str> = if !bitcoin_enabled {
                                Some("Bitcoin disabled on this server.")
                            } else if !charter_allows_sats {
                                Some("Tipping requires Bitcoin enabled for this account.")
                            } else if vow_suppresses_sats {
                                Some("The Nostrich's Vow suppresses sats outflows.")
                            } else {
                                None
                            };
                            if let Some(line) = status_line {
                                ui.label(
                                    egui::RichText::new(line)
                                        .size(13.0)
                                        .color(DIM_COLOUR)
                                        .italics(),
                                );
                                ui.add_space(4.0);
                            }
                            // An unknown balance is hidden, never faked.
                            if let Some(line) = balance_line(player_sats_balance) {
                                ui.label(egui::RichText::new(line).size(12.0).color(DIM_COLOUR));
                            }
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                for &amount in PRESET_AMOUNTS {
                                    let can_pay =
                                        tip_button_enabled(tips_disabled, player_sats_balance, amount);
                                    let mut btn = egui::Button::new(
                                        egui::RichText::new(format!("{amount} sats")).color(
                                            if can_pay {
                                                egui::Color32::WHITE
                                            } else {
                                                DISABLED_COLOUR
                                            },
                                        ),
                                    );
                                    if !can_pay {
                                        btn = btn.fill(egui::Color32::from_gray(40));
                                    }
                                    if ui.add_enabled(can_pay, btn).clicked() {
                                        outcome = TipJarUiOutcome::Tip { amount_sats: amount };
                                    }
                                }
                            });
                        }
                        ui.add_space(6.0);
                        if ui.button("Close").clicked() {
                            outcome = TipJarUiOutcome::Closed;
                        }
                    });
                });
        });

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tip_jar::{TipJarData, TipJarOwner};

    #[test]
    fn owner_label_formats_local_player() {
        let data = TipJarData {
            owner: Some(TipJarOwner::LocalPlayer(0)),
            ..Default::default()
        };
        assert_eq!(owner_label(&data), "Player 1");
        let data2 = TipJarData {
            owner: Some(TipJarOwner::LocalPlayer(3)),
            ..Default::default()
        };
        assert_eq!(owner_label(&data2), "Player 4");
    }

    #[test]
    fn owner_label_truncates_npub() {
        let data = TipJarData {
            owner: Some(TipJarOwner::Npub("npub1abc123def456".to_string())),
            ..Default::default()
        };
        let label = owner_label(&data);
        assert!(label.starts_with("npub1abc12"));
        assert!(label.ends_with("…"));
    }

    #[test]
    fn an_unknown_balance_is_hidden_never_a_sentinel() {
        assert_eq!(balance_line(None), None, "no wallet → no balance line");
        assert_eq!(balance_line(Some(250)).as_deref(), Some("Your balance: 250 sats"));
        assert!(!tip_button_enabled(true, None, 10), "disabled tipping stays disabled");
        assert!(tip_button_enabled(false, None, 10));
        assert!(!tip_button_enabled(false, Some(5), 10), "a known short balance can't pay");
    }

    #[test]
    fn owner_label_handles_no_owner() {
        let data = TipJarData::default();
        assert_eq!(owner_label(&data), "(no owner)");
    }
}

//! Spec 35 Repair Bench — repair dialog.
//!
//! v1 repairs the currently-held hotbar tool (no slot-deposit
//! interaction — keeps the UI to a single read-only quote + a
//! Repair button). The caller computes the quote from the held tool
//! + the player's material count and passes the display data in;
//!   this module renders + returns the click outcome.
//!
//! Pure render — no state mutation.

use crate::screen::ViewportRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairUiOutcome {
    InProgress,
    Closed,
    /// Player clicked Repair. Caller applies the quote (apply_repair +
    /// material decrement + sats payout).
    Repair,
}

/// Display payload for the dialog. The caller assembles this from the
/// held tool + inventory; the UI is pure presentation.
pub struct RepairView<'a> {
    /// Tool display name, or None if the player isn't holding a
    /// repairable tool.
    pub tool_name: Option<&'a str>,
    /// Current / max durability of the held tool.
    pub current: u16,
    pub max: u16,
    /// Repair material display name (e.g. "Iron Ingot").
    pub material_name: &'a str,
    /// How many units of the material the player has.
    pub material_have: u16,
    /// Quote: durability that would be restored.
    pub durability_restored: u16,
    /// Quote: material units consumed.
    pub material_consumed: u16,
    /// Quote: sats tax (0 on Bitcoin-disabled / Charter-off).
    pub sats_tax: u64,
    /// Whether sats are chargeable (Bitcoin enabled + Charter on +
    /// no Vow). When false, repair is free (material cost only).
    pub sats_chargeable: bool,
    /// Why the tool can't be repaired, if applicable (already full,
    /// no material, not a repairable tool). None = repairable.
    pub blocked_reason: Option<&'a str>,
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOUR: egui::Color32 = egui::Color32::from_rgb(180, 200, 230);
const DIM_COLOUR: egui::Color32 = egui::Color32::LIGHT_GRAY;
const DISABLED_COLOUR: egui::Color32 = egui::Color32::from_rgb(120, 120, 120);

pub fn draw_repair_ui(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    view: &RepairView,
) -> RepairUiOutcome {
    let mut outcome = RepairUiOutcome::InProgress;

    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("repair_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let window_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 200.0,
        viewport.y as f32 + viewport.height as f32 / 3.0,
    );

    egui::Area::new(egui::Id::new(("repair_window", player_index)))
        .fixed_pos(window_origin)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(18, 20, 24, 240))
                .show(ui, |ui| {
                    ui.set_min_width(400.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Repair Bench")
                                .size(20.0)
                                .color(TITLE_COLOUR)
                                .strong(),
                        );
                        ui.add_space(8.0);

                        match view.tool_name {
                            None => {
                                ui.label(
                                    egui::RichText::new(
                                        "Hold a damaged tool (iron, diamond, or satori) to repair it.",
                                    )
                                    .size(13.0)
                                    .color(DIM_COLOUR),
                                );
                            }
                            Some(name) => {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{name}  ·  {} / {} durability",
                                        view.current, view.max,
                                    ))
                                    .size(15.0)
                                    .color(TITLE_COLOUR),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}: {} available",
                                        view.material_name, view.material_have,
                                    ))
                                    .size(12.0)
                                    .color(DIM_COLOUR),
                                );
                                ui.add_space(6.0);

                                if let Some(reason) = view.blocked_reason {
                                    ui.label(
                                        egui::RichText::new(reason)
                                            .size(13.0)
                                            .italics()
                                            .color(DIM_COLOUR),
                                    );
                                } else {
                                    // Live quote line.
                                    let cost_str = if view.sats_chargeable {
                                        format!("{} sats", view.sats_tax)
                                    } else {
                                        "free".to_string()
                                    };
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Restore {} durability  ·  {} {}  ·  {}",
                                            view.durability_restored,
                                            view.material_consumed,
                                            view.material_name,
                                            cost_str,
                                        ))
                                        .size(13.0)
                                        .color(TITLE_COLOUR),
                                    );
                                    ui.add_space(6.0);
                                    if ui.button(
                                        egui::RichText::new("Repair").color(egui::Color32::WHITE),
                                    ).clicked() {
                                        outcome = RepairUiOutcome::Repair;
                                    }
                                }
                            }
                        }

                        ui.add_space(6.0);
                        if ui.button("Close").clicked() {
                            outcome = RepairUiOutcome::Closed;
                        }
                        let _ = DISABLED_COLOUR;
                    });
                });
        });

    outcome
}

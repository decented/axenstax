//! Spec 20 Phase 5 — Furnace egui overlay. **Spec 29 (2026-05-21)**
//! reworked the surface following Axolittle's playtest:
//!
//! - Three slots are now **clickable buttons** with Minecraft-style
//!   semantics. Left-click moves 1 unit; shift-click moves the whole
//!   stack. Empty hand on a slot = take from slot. Held item on Input
//!   = place if smeltable. Held item on Fuel = place if has fuel
//!   value. Output is take-only (held item is ignored).
//! - The **Close button is gone**. 'E' (matching inventory) and Esc
//!   close the UI.
//! - The slot-click event handler lives in `furnace::apply_slot_click`
//!   so the logic is testable without the egui frame loop.

use crate::block::BlockRegistry;
use crate::egui_integration::EguiIntegration;
use crate::furnace::{FurnaceData, SMELT_TICKS_PER_ITEM, SlotKind, ClickMode};
use crate::item::{Item, ItemStack};

/// UI outcome. Spec 29 expanded this enum: in addition to Closed and
/// InProgress, the UI can emit a SlotClick event the caller routes
/// through `furnace::apply_slot_click` on its own inventory state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FurnaceUiOutcome {
    /// UI is still open this frame, no click events.
    InProgress,
    /// Player closed the UI (Esc, 'E', or click outside).
    Closed,
    /// Player clicked a slot. Caller invokes
    /// `furnace::apply_slot_click(..)` with the player's inventory.
    SlotClick { kind: SlotKind, mode: ClickMode },
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 195, 100);

/// Draw the furnace UI. Returns the outcome — caller (game_loop)
/// clears the player's `open_furnace` field on `Closed`.
pub fn draw_furnace_ui(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    data: &FurnaceData,
    registry: &BlockRegistry,
    _egui_integration: &EguiIntegration,
) -> FurnaceUiOutcome {
    let mut outcome = FurnaceUiOutcome::InProgress;

    // Dim background covering the player's viewport rect.
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("furnace_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 180.0,
        viewport.y as f32 + 80.0,
    );
    egui::Area::new(egui::Id::new(("furnace_panel", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.set_max_width(360.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("Furnace")
                        .size(24.0)
                        .color(TITLE_COLOR)
                        .strong(),
                );
                ui.add_space(12.0);

                let shift_held = ctx.input(|i| i.modifiers.shift);
                let mode = if shift_held { ClickMode::Stack } else { ClickMode::Single };

                ui.horizontal(|ui| {
                    if slot_button(ui, "Input", data.input.as_ref(), registry).clicked() {
                        outcome = FurnaceUiOutcome::SlotClick { kind: SlotKind::Input, mode };
                    }
                    ui.add_space(8.0);
                    if slot_button(ui, "Fuel", data.fuel.as_ref(), registry).clicked() {
                        outcome = FurnaceUiOutcome::SlotClick { kind: SlotKind::Fuel, mode };
                    }
                });
                ui.add_space(8.0);

                // Progress bar — fills from 0 to 1 across SMELT_TICKS_PER_ITEM.
                let progress = if data.smelt_total > 0 {
                    (data.smelt_progress as f32) / (data.smelt_total as f32)
                } else {
                    0.0
                };
                let progress = progress.clamp(0.0, 1.0);
                ui.add(egui::ProgressBar::new(progress)
                    .desired_width(220.0)
                    .text(if data.lit { "smelting…" } else { "idle" }));

                // Fuel-burn indicator — separate bar so the player can
                // tell at a glance whether the furnace is starved.
                let fuel_frac = if data.fuel_ticks_remaining > 0 {
                    // Cap visual scale at one coal-worth of burn (4800 ticks)
                    // so a leaf-burn doesn't look like a full reserve.
                    (data.fuel_ticks_remaining as f32 / 4800.0).min(1.0)
                } else { 0.0 };
                ui.add(egui::ProgressBar::new(fuel_frac)
                    .desired_width(220.0)
                    .text(format!("fuel: {} ticks", data.fuel_ticks_remaining)));

                ui.add_space(8.0);
                if slot_button(ui, "Output", data.output.as_ref(), registry).clicked() {
                    outcome = FurnaceUiOutcome::SlotClick { kind: SlotKind::Output, mode };
                }

                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new(
                        "Click Input to load ore (raw iron / copper / tin). \
                         Click Fuel to load coal / planks / logs. \
                         Click Output to take ingots. \
                         Shift-click moves a whole stack. Press E or Esc to close.",
                    )
                    .size(13.0)
                    .color(egui::Color32::LIGHT_GRAY),
                );
            });
        });

    // Esc + 'E' close — Spec 29: 'E' closes the furnace UI exactly
    // like it toggles the inventory, so the kid uses one key.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::E)) {
        outcome = FurnaceUiOutcome::Closed;
    }

    let _ = SMELT_TICKS_PER_ITEM; // Touch the const so cross-spec readers find it.
    outcome
}

/// Spec 29 — slot rendered as a clickable button so the player can
/// drag/drop / single-click without right-clicking the world block.
/// The returned `Response` lets the caller emit a SlotClick outcome.
fn slot_button(
    ui: &mut egui::Ui,
    name: &str,
    stack: Option<&ItemStack>,
    registry: &BlockRegistry,
) -> egui::Response {
    let label = match stack {
        None => format!("{name}\nempty"),
        Some(s) => {
            let item_label = match &s.item {
                Item::Material(m) => format!("{:?}", m),
                Item::Block(b) => registry.get(*b).name.to_string(),
                Item::Tool(_) => "tool".to_string(),
                Item::Plan(_) => "plan".to_string(),
                Item::Armour(_) => "armour".to_string(),
            };
            if s.count > 1 {
                format!("{name}\n{item_label} ×{}", s.count)
            } else {
                format!("{name}\n{item_label}")
            }
        }
    };
    ui.add_sized([110.0, 56.0], egui::Button::new(label))
}

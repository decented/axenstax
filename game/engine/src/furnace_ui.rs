//! Spec 20 Phase 5 — Furnace egui overlay. **Spec 29 (2026-05-21)**
//! reworked the surface following the tester's playtest:
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
//! - A click that moves nothing says why on the screen (`furnace::click_refusal`), and the slots
//!   carry the item's plain name, not its Rust debug name.

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
const NOTICE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 140, 110);
/// How long a refusal stays on the furnace screen, in seconds.
const NOTICE_SECONDS: f64 = 3.0;

/// Draw the furnace UI. Returns the outcome — caller (game_loop)
/// clears the player's `open_furnace` field on `Closed`. `inventory_pressed` is
/// this frame's Inventory press (`InputState::inventory_toggle`, player 0 only):
/// the inventory key closes the furnace as it opens the inventory, whatever it
/// is bound to; `keys` names it in the hint. `held` is the item selected on the hotbar, the one a
/// click on Input or Fuel loads: a click that can do nothing with it says why under the slots.
pub fn draw_furnace_ui(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    data: &FurnaceData,
    registry: &BlockRegistry,
    _egui_integration: &EguiIntegration,
    keys: &crate::keybindings::Bindings,
    inventory_pressed: bool,
    held: Option<&ItemStack>,
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
                let notice_id = egui::Id::new(("furnace_notice", player_index));

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

                // A click that moves nothing says why, for a few seconds.
                let now = ctx.input(|i| i.time);
                if let FurnaceUiOutcome::SlotClick { kind, .. } = outcome
                    && let Some(why) = crate::furnace::click_refusal(data, held, kind)
                {
                    ctx.data_mut(|d| d.insert_temp(notice_id, (why.message(), now + NOTICE_SECONDS)));
                }
                let notice = ctx
                    .data(|d| d.get_temp::<(&'static str, f64)>(notice_id))
                    .filter(|(_, until)| *until > now);
                ui.add_space(8.0);
                ui.label(match notice {
                    Some((text, _)) => egui::RichText::new(text).size(15.0).color(NOTICE_COLOR).strong(),
                    None => egui::RichText::new(match held {
                        Some(stack) => format!("Selected on your hotbar: {}", stack.item.name(registry)),
                        None => "Nothing selected on your hotbar".to_string(),
                    })
                    .size(13.0)
                    .color(egui::Color32::GRAY),
                });

                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Click Input or Fuel to load the item selected on your hotbar: \
                         raw iron, copper or tin goes in Input; coal, planks, logs and sticks go in Fuel. \
                         Click Output to take ingots. \
                         Shift-click moves a whole stack. Press {} or Esc to close.",
                        keys.hint(crate::keybindings::Action::Inventory),
                    ))
                    .size(13.0)
                    .color(egui::Color32::LIGHT_GRAY),
                );
            });
        });

    // Esc + the inventory key close — Spec 29: the inventory key (E unless
    // rebound) closes the furnace UI exactly like it toggles the inventory,
    // so the kid uses one key.
    if inventory_pressed || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = FurnaceUiOutcome::Closed;
    }

    let _ = SMELT_TICKS_PER_ITEM; // Touch the const so cross-spec readers find it.
    outcome
}

/// What a furnace slot calls the item in it: the same plain name the inventory shows
/// (`Raw Iron`, `Oak Planks`), not a Rust debug name or a registry id.
fn slot_item_label(item: &Item, registry: &BlockRegistry) -> String {
    item.name(registry)
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
            let item_label = slot_item_label(&s.item, registry);
            if s.count > 1 {
                format!("{name}\n{item_label} ×{}", s.count)
            } else {
                format!("{name}\n{item_label}")
            }
        }
    };
    ui.add_sized([110.0, 56.0], egui::Button::new(label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::MaterialId;

    #[test]
    fn slots_carry_plain_names_not_debug_names() {
        let reg = BlockRegistry::new();
        assert_eq!(slot_item_label(&Item::Material(MaterialId::RawIron), &reg), "Raw Iron");
        assert_eq!(slot_item_label(&Item::Material(MaterialId::IronIngot), &reg), "Iron Ingot");
        assert_eq!(slot_item_label(&Item::Block(crate::block::OAK_PLANKS), &reg), "Oak Planks");
        let tool = Item::Tool(crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Iron));
        let label = slot_item_label(&tool, &reg);
        assert_ne!(label, "tool");
        assert!(!label.contains("genesis:"));
    }
}

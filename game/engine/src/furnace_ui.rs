//! Spec 20 Phase 5 — Furnace egui overlay. **Spec 29 (2026-05-21)** made the
//! three furnace slots clickable; **pt1124 (2026-11-25)** showed the player's
//! whole inventory beside them, the way the chest screen does, after a tester
//! could not get fuel and ore in (the old screen loaded only the item selected
//! on the hotbar, and said nothing when it refused):
//!
//! - The inventory (bag and hotbar) is drawn under the furnace's slots, in the
//!   same slots the inventory and chest screens draw (`craft_ui::draw_item_slot`:
//!   the item's icon, its count, its durability), all four rows always on the
//!   screen: the panel is as tall as the window and scrolls when the window is
//!   shorter than the panel. Click
//!   an item to send it where it belongs: a smeltable (raw iron, copper, tin)
//!   to Input, fuel (coal, sticks, planks, logs, leaves) to Fuel
//!   (`furnace::route_stack`, the one routing rule). A click moves one;
//!   Shift-click, or the "Move whole stacks" switch (touch and controller have
//!   no Shift), moves the whole stack.
//! - Click Input, Fuel or Output to take their contents back.
//! - A click that moves nothing says why on the screen (`furnace::click_refusal`),
//!   and every slot carries the item's plain name on hover.
//! - 'E' (the inventory key) and Esc close; there is no Close button.
//! - The click rules live in `furnace::apply_slot_click`, reached through
//!   `container_window::ContainerClick::Furnace` (host, solo and joined play
//!   all use the same window path).

use crate::block::BlockRegistry;
use crate::craft_ui::{draw_item_slot, IconSource};
use crate::furnace::{ClickMode, FurnaceData, SlotKind, SMELT_TICKS_PER_ITEM, TAKE_BACK};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};

/// UI outcome: the screen is still open, closed, or a slot was clicked and the
/// caller routes it through the window (`ContainerClick::Furnace`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FurnaceUiOutcome {
    /// UI is still open this frame, no click events.
    InProgress,
    /// Player closed the UI (Esc or the inventory key).
    Closed,
    /// Player clicked: `kind` is the furnace slot the move is for (already
    /// routed for an inventory click), `from` the inventory slot it loads
    /// from or `furnace::TAKE_BACK`.
    SlotClick { kind: SlotKind, mode: ClickMode, from: usize },
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 195, 100);
const NOTICE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 140, 110);
/// The edge colours of the inventory stacks the furnace takes: smeltable (blue) and
/// fuel (orange).
const SMELT_TINT: egui::Color32 = egui::Color32::from_rgb(90, 150, 235);
const FUEL_TINT: egui::Color32 = egui::Color32::from_rgb(240, 150, 60);
/// How long a refusal stays on the furnace screen, in seconds.
const NOTICE_SECONDS: f64 = 3.0;
/// An inventory slot's side, and the gap between slots.
const INV_SLOT: f32 = 46.0;
const INV_GAP: f32 = 4.0;
/// One of the furnace's own slots' side.
const FURNACE_SLOT: f32 = 54.0;
/// Panel width: nine inventory columns.
const PANEL_WIDTH: f32 = 9.0 * INV_SLOT + 8.0 * INV_GAP + 8.0;

/// Draw the furnace UI. Returns the outcome — caller (game_loop)
/// clears the player's `open_furnace` field on `Closed`. `inventory_pressed` is
/// this frame's Inventory press (`InputState::inventory_toggle`, player 0 only):
/// the inventory key closes the furnace as it opens the inventory, whatever it
/// is bound to; `keys` names it in the hint. `inventory` is the player's own
/// (bag and hotbar), drawn under the furnace's slots.
pub fn draw_furnace_ui(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    data: &FurnaceData,
    registry: &BlockRegistry,
    icons: &dyn IconSource,
    keys: &crate::keybindings::Bindings,
    inventory_pressed: bool,
    inventory: &Inventory,
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
        viewport.x as f32 + (viewport.width as f32 - PANEL_WIDTH) / 2.0,
        viewport.y as f32 + 16.0,
    );
    let notice_id = egui::Id::new(("furnace_notice", player_index));
    let whole_id = egui::Id::new(("furnace_whole_stacks", player_index));
    egui::Area::new(egui::Id::new(("furnace_panel", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(PANEL_WIDTH);
            ui.set_max_width(PANEL_WIDTH);
            // The panel is as tall as the window (less its margin) whatever its
            // content measured last frame, so the hotbar row is never cut off by an
            // earlier, shorter measurement; a window shorter than the panel scrolls.
            egui::ScrollArea::vertical()
                .max_height((viewport.height as f32 - 32.0).max(120.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("Furnace").size(24.0).color(TITLE_COLOR).strong());
                        ui.add_space(8.0);

                        // Shift, or the switch for a screen with no Shift (touch, controller).
                        let mut whole = ctx.data(|d| d.get_temp::<bool>(whole_id)).unwrap_or(false);
                        if ui.checkbox(&mut whole, "Move whole stacks").changed() {
                            ctx.data_mut(|d| d.insert_temp(whole_id, whole));
                        }
                        let mode = if whole || ctx.input(|i| i.modifiers.shift) { ClickMode::Stack } else { ClickMode::Single };
                        ui.add_space(6.0);

                        // The furnace's own three slots: click to take back.
                        ui.horizontal(|ui| {
                            ui.add_space((PANEL_WIDTH - 3.0 * FURNACE_COLUMN - 2.0 * ui.spacing().item_spacing.x) / 2.0);
                            if furnace_slot(ui, "Input", data.input.as_ref(), registry, icons).clicked() {
                                outcome = FurnaceUiOutcome::SlotClick { kind: SlotKind::Input, mode, from: TAKE_BACK };
                            }
                            if furnace_slot(ui, "Fuel", data.fuel.as_ref(), registry, icons).clicked() {
                                outcome = FurnaceUiOutcome::SlotClick { kind: SlotKind::Fuel, mode, from: TAKE_BACK };
                            }
                            if furnace_slot(ui, "Output", data.output.as_ref(), registry, icons).clicked() {
                                outcome = FurnaceUiOutcome::SlotClick { kind: SlotKind::Output, mode, from: TAKE_BACK };
                            }
                        });
                        ui.add_space(6.0);

                        // Progress bar — fills from 0 to 1 across SMELT_TICKS_PER_ITEM.
                        let progress = if data.smelt_total > 0 {
                            (data.smelt_progress as f32) / (data.smelt_total as f32)
                        } else {
                            0.0
                        };
                        ui.add(egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                            .desired_width(280.0)
                            .text(if data.lit { "smelting…" } else { "idle" }));
                        // Fuel-burn indicator — separate bar so the player can tell at a glance
                        // whether the furnace is starved. Visual scale is one coal's burn (4800
                        // ticks); the words are seconds (20 ticks a second).
                        let fuel_frac = if data.fuel_ticks_remaining > 0 {
                            (data.fuel_ticks_remaining as f32 / 4800.0).min(1.0)
                        } else {
                            0.0
                        };
                        ui.add(egui::ProgressBar::new(fuel_frac)
                            .desired_width(280.0)
                            .text(if data.fuel_ticks_remaining > 0 {
                                format!("burning: {} s left", data.fuel_ticks_remaining.div_ceil(20))
                            } else {
                                "not burning".to_string()
                            }));

                        // The player's whole inventory: bag on top, hotbar underneath.
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new("Your inventory").strong());
                        ui.spacing_mut().item_spacing = egui::vec2(INV_GAP, INV_GAP);
                        for row in [1, 2, 3, 0] {
                            if row == 0 {
                                ui.add_space(4.0);
                            }
                            ui.horizontal(|ui| {
                                for col in 0..9 {
                                    let idx = row * 9 + col;
                                    let stack = inventory.slot(idx);
                                    if inventory_slot(ui, stack, data, registry, icons).clicked()
                                        && let Some(stack) = stack
                                    {
                                        match crate::furnace::route_stack(data, stack) {
                                            Ok(kind) => outcome = FurnaceUiOutcome::SlotClick { kind, mode, from: idx },
                                            Err(why) => notice(ctx, notice_id, why.message()),
                                        }
                                    }
                                }
                            });
                        }
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 3.0);

                        // A click that moves nothing says why, for a few seconds, and is not sent.
                        if let FurnaceUiOutcome::SlotClick { kind, from, .. } = outcome
                            && let Some(why) = crate::furnace::click_refusal(data, inventory, from, kind)
                        {
                            notice(ctx, notice_id, why.message());
                            outcome = FurnaceUiOutcome::InProgress;
                        }
                        let now = ctx.input(|i| i.time);
                        let shown = ctx
                            .data(|d| d.get_temp::<(&'static str, f64)>(notice_id))
                            .filter(|(_, until)| *until > now);
                        ui.add_space(8.0);
                        ui.label(match shown {
                            Some((text, _)) => egui::RichText::new(text).size(15.0).color(NOTICE_COLOR).strong(),
                            None => egui::RichText::new(" ").size(15.0),
                        });
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Click an item to load it: a blue edge goes in Input (raw iron, copper, tin), \
                                 an orange edge goes in Fuel (coal, planks, logs, sticks). \
                                 Click Input, Fuel or Output to take things out. \
                                 Shift-click moves a whole stack. Press {} or Esc to close.",
                                keys.hint(crate::keybindings::Action::Inventory),
                            ))
                            .size(13.0)
                            .color(egui::Color32::LIGHT_GRAY),
                        );
                    });
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

/// Show `text` on the furnace screen for a few seconds.
fn notice(ctx: &egui::Context, id: egui::Id, text: &'static str) {
    let until = ctx.input(|i| i.time) + NOTICE_SECONDS;
    ctx.data_mut(|d| d.insert_temp(id, (text, until)));
}

/// What a furnace slot calls the item in it: the same plain name the inventory shows
/// (`Raw Iron`, `Oak Planks`), not a Rust debug name or a registry id.
fn slot_item_label(item: &Item, registry: &BlockRegistry) -> String {
    item.name(registry)
}

/// The width of one of the furnace's own slots' column (the slot and its name under it).
const FURNACE_COLUMN: f32 = 110.0;

/// Spec 29 — one of the furnace's own slots, drawn as the inventory's are (the
/// item's icon and count) with its name above and the item's plain name under it;
/// click to take its contents out. The returned `Response` lets the caller emit a
/// SlotClick outcome.
fn furnace_slot(
    ui: &mut egui::Ui,
    name: &str,
    stack: Option<&ItemStack>,
    registry: &BlockRegistry,
    icons: &dyn IconSource,
) -> egui::Response {
    ui.allocate_ui(egui::vec2(FURNACE_COLUMN, FURNACE_SLOT + 40.0), |ui| {
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(name).strong());
            let response = draw_item_slot(ui, &stack.cloned(), registry, icons, FURNACE_SLOT, false, false, false);
            ui.label(egui::RichText::new(match stack {
                None => "empty".to_string(),
                Some(s) => slot_item_label(&s.item, registry),
            }).size(11.0));
            if stack.is_some() { response.on_hover_text("Click to take it out") } else { response }
        })
        .inner
    })
    .inner
}

/// One inventory slot, drawn as the inventory screen's are (the icon, the count, the
/// durability bar), with an edge in blue when the furnace would smelt it and in orange
/// when it would burn it (the edge is `route_stack`'s answer).
fn inventory_slot(
    ui: &mut egui::Ui,
    stack: Option<&ItemStack>,
    data: &FurnaceData,
    registry: &BlockRegistry,
    icons: &dyn IconSource,
) -> egui::Response {
    let response = draw_item_slot(ui, &stack.cloned(), registry, icons, INV_SLOT, false, false, false);
    if let Some(stack) = stack
        && let Ok(kind) = crate::furnace::route_stack(data, stack)
    {
        let edge = if kind == SlotKind::Input { SMELT_TINT } else { FUEL_TINT };
        ui.painter().rect_stroke(response.rect, 3.0, egui::Stroke::new(2.0_f32, edge), egui::StrokeKind::Inside);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::craft_ui::NoIcons;
    use crate::item::MaterialId;

    /// The rows of inventory slots a pass at window height `h` puts on the screen:
    /// the distinct top and bottom of every slot-sized square it paints.
    fn slot_rows(h: u32) -> Vec<(f32, f32)> {
        let ctx = egui::Context::default();
        let reg = BlockRegistry::new();
        let mut inv = Inventory::new();
        for i in 0..36 {
            inv.set_slot(i, Some(ItemStack::new_material(MaterialId::Coal, 3)));
        }
        let keys = crate::keybindings::Bindings::default();
        let vp = crate::screen::ViewportRect { x: 0, y: 0, width: 1280, height: h };
        let mut rows: Vec<(f32, f32)> = Vec::new();
        // Several passes: egui settles its sizing on the first.
        for _ in 0..4 {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, h as f32)));
            let out = ctx.run(input, |ctx| {
                let _ = draw_furnace_ui(ctx, &vp, 0, &FurnaceData::default(), &reg, &NoIcons, &keys, false, &inv);
            });
            rows.clear();
            for clipped in &out.shapes {
                if let egui::Shape::Rect(r) = &clipped.shape
                    && (r.rect.width() - INV_SLOT).abs() < 0.01
                    && (r.rect.height() - INV_SLOT).abs() < 0.01
                    && !rows.iter().any(|(top, _)| (*top - r.rect.top()).abs() < 0.5)
                {
                    rows.push((r.rect.top(), r.rect.bottom()));
                }
            }
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows
    }

    #[test]
    fn all_four_inventory_rows_are_on_a_720p_screen() {
        let rows = slot_rows(720);
        assert_eq!(rows.len(), 4, "bag rows and the hotbar: {rows:?}");
        assert!(rows.iter().all(|(_, bottom)| *bottom <= 720.0), "the hotbar row is on the screen: {rows:?}");
    }

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

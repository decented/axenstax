//! The inventory window on a small screen (a phone), drawn whole.
//!
//! The desktop window is a fixed 600 x 650 point panel of 48-point slots. On a landscape phone
//! (about 800 x 360 points) most of it is off the screen, and a touch screen cannot scroll it
//! (a drag on an inventory slot is a tap or a move, not a scroll), so nothing past the top row
//! of the bag could be moved. This lays the same window out to fit the screen instead: every
//! slot (armour, off-hand, the crafting grid and its output, the 27-slot bag, the hotbar) and
//! the Sort, Trash and Recipe Book buttons at once, with slots as large as the screen allows
//! (at most the desktop's 48 points, at least [`MIN_SLOT`]).
//!
//! The geometry is pure (sizes in, sizes out) so it is unit-tested. A screen the desktop panel
//! fits on gets `None` from [`compact_plan`] and keeps the layout it always had.
use super::*;

/// The smallest a slot is drawn, in points: a thumb-sized target.
pub(super) const MIN_SLOT: f32 = 28.0;
/// The largest, the desktop's.
const MAX_SLOT: f32 = 48.0;
/// The margin kept clear on every side of the screen.
const MARGIN: f32 = 8.0;
/// A screen at least this big (in points) keeps the desktop layout.
const DESKTOP_SCREEN: egui::Vec2 = egui::vec2(640.0, 700.0);
const TITLE_H: f32 = 22.0;
const ROW_GAP: f32 = 6.0;
/// The space between the bag and the hotbar.
const HOTBAR_GAP: f32 = 8.0;
/// The space between the crafting block and the bag when they sit side by side.
const SIDE_GAP: f32 = 16.0;
const ARROW_W: f32 = 16.0;
const BUTTONS_H: f32 = 40.0;
const BUTTON_WIDTHS: [f32; 3] = [84.0, 92.0, 118.0];
const BUTTON_GAP: f32 = 6.0;

fn buttons_width() -> f32 {
    BUTTON_WIDTHS.iter().sum::<f32>() + BUTTON_GAP * (BUTTON_WIDTHS.len() as f32 - 1.0)
}

/// Where everything goes, for one slot size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CompactPlan {
    pub slot: f32,
    pub gap: f32,
    /// Crafting on the left and the bag on the right, instead of crafting above the bag.
    pub side_by_side: bool,
    /// The whole window, in points.
    pub size: egui::Vec2,
}

fn gap_for(slot: f32) -> f32 {
    (slot * 0.08).round().max(2.0)
}

/// The crafting block: armour column, off-hand column, grid, arrow, output.
fn crafting_size(slot: f32, gap: f32, grid: usize, offhand: bool) -> egui::Vec2 {
    let col = slot + gap;
    let grid_w = grid as f32 * slot + (grid as f32 - 1.0) * gap;
    let width = col + if offhand { col } else { 0.0 } + 10.0 + grid_w + 8.0 + ARROW_W + 8.0 + slot;
    egui::vec2(width, 4.0 * slot + 3.0 * gap)
}

/// The bag's three rows and the hotbar under them.
fn bag_size(slot: f32, gap: f32) -> egui::Vec2 {
    egui::vec2(9.0 * slot + 8.0 * gap, 4.0 * slot + 2.0 * gap + HOTBAR_GAP)
}

fn plan_size(slot: f32, gap: f32, side_by_side: bool, grid: usize, offhand: bool) -> egui::Vec2 {
    let craft = crafting_size(slot, gap, grid, offhand);
    let bag = bag_size(slot, gap);
    if side_by_side {
        let left_w = craft.x.max(buttons_width());
        let left_h = TITLE_H + ROW_GAP + craft.y + ROW_GAP + BUTTONS_H;
        let right_h = TITLE_H + ROW_GAP + bag.y;
        egui::vec2(left_w + SIDE_GAP + bag.x, left_h.max(right_h))
    } else {
        egui::vec2(
            craft.x.max(bag.x).max(buttons_width()),
            TITLE_H + ROW_GAP + craft.y + ROW_GAP + bag.y + ROW_GAP + BUTTONS_H,
        )
    }
}

/// The compact layout for a screen of `screen` points, or `None` where the desktop panel fits.
///
/// The largest slot that fits wins, whichever arrangement it takes: crafting above the bag
/// (what the desktop does, and the only one a portrait phone has width for) or crafting
/// beside it (what a landscape phone has room for, at a bigger slot). At equal size, above.
/// If even [`MIN_SLOT`] does not fit, it is used anyway in the arrangement that overflows less.
pub(super) fn compact_plan(screen: egui::Vec2, grid: usize, offhand: bool) -> Option<CompactPlan> {
    if screen.x >= DESKTOP_SCREEN.x && screen.y >= DESKTOP_SCREEN.y {
        return None;
    }
    let room = screen - egui::Vec2::splat(2.0 * MARGIN);
    let fits = |size: egui::Vec2| size.x <= room.x && size.y <= room.y;
    let mut slot = MAX_SLOT;
    while slot >= MIN_SLOT {
        let gap = gap_for(slot);
        for side_by_side in [false, true] {
            let size = plan_size(slot, gap, side_by_side, grid, offhand);
            if fits(size) {
                return Some(CompactPlan { slot, gap, side_by_side, size });
            }
        }
        slot -= 1.0;
    }
    let gap = gap_for(MIN_SLOT);
    let overflow = |size: egui::Vec2| (size.x - room.x).max(0.0) + (size.y - room.y).max(0.0);
    let (above, beside) = (plan_size(MIN_SLOT, gap, false, grid, offhand), plan_size(MIN_SLOT, gap, true, grid, offhand));
    let side_by_side = overflow(beside) < overflow(above);
    Some(CompactPlan { slot: MIN_SLOT, gap, side_by_side, size: if side_by_side { beside } else { above } })
}

/// What a compact draw found out, for the caller to use as the desktop draw's.
pub(super) struct CompactOut {
    pub click: ClickTarget,
    pub focused_rect: Option<egui::Rect>,
    pub result_rect: Option<egui::Rect>,
}

/// One child of the panel, placed exactly in `rect`.
fn at<R>(ui: &mut egui::Ui, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), add).inner
}

fn slot_rect(origin: egui::Pos2, col: usize, row: usize, slot: f32, gap: f32) -> egui::Rect {
    egui::Rect::from_min_size(origin + egui::vec2(col as f32 * (slot + gap), row as f32 * (slot + gap)), egui::Vec2::splat(slot))
}

/// What a click or drag on one bag or hotbar slot means: the window transition it asks for.
/// Shared by the desktop and the compact draw.
pub(super) fn inventory_slot_target(
    ui: &egui::Ui,
    response: &egui::Response,
    ui_state: &mut CraftingUi,
    slot_idx: usize,
    has_item: bool,
    drag_secondary: bool,
) -> Option<ClickTarget> {
    let mut target = None;
    // #45 — Alt+click toggles the slot lock instead of moving.
    let alt = ui.input(|i| i.modifiers.alt);
    if response.clicked() {
        target = Some(if alt { ClickTarget::ToggleLock(slot_idx) } else { ClickTarget::InventorySlot(slot_idx, false) });
    } else if response.secondary_clicked() {
        target = Some(ClickTarget::InventorySlot(slot_idx, true));
    } else if response.hovered() && has_item && ui.input(|i| i.key_pressed(egui::Key::U)) {
        // #46 — JEI "show uses" on the hovered item.
        target = Some(ClickTarget::ShowUses(slot_idx));
    }
    // #45 P3 — drag paint over inventory slots (ids 0..36).
    let id = slot_idx as u32;
    if response.drag_started() {
        ui_state.drag_paint = Some(drag_secondary);
        ui_state.drag_visited.clear();
        ui_state.drag_visited.insert(id);
        target = Some(ClickTarget::DragInventory(slot_idx, drag_secondary));
    } else if let Some(distribute) = ui_state.drag_paint
        && response.hovered()
        && ui_state.drag_visited.insert(id)
    {
        target = Some(ClickTarget::DragInventory(slot_idx, distribute));
    }
    target
}

/// What a click or drag on one crafting-grid cell means.
pub(super) fn grid_slot_target(
    response: &egui::Response,
    ui_state: &mut CraftingUi,
    (r, c): (usize, usize),
    grid_size: usize,
    drag_secondary: bool,
) -> Option<ClickTarget> {
    let mut target = None;
    if response.clicked() {
        target = Some(ClickTarget::GridSlot(r, c, false));
    } else if response.secondary_clicked() {
        target = Some(ClickTarget::GridSlot(r, c, true));
    }
    // #45 P3 — drag paint over grid cells.
    let gid = (100 + r * grid_size + c) as u32;
    if response.drag_started() {
        ui_state.drag_paint = Some(drag_secondary);
        ui_state.drag_visited.clear();
        ui_state.drag_visited.insert(gid);
        target = Some(ClickTarget::DragGrid(r, c, drag_secondary));
    } else if let Some(distribute) = ui_state.drag_paint
        && response.hovered()
        && ui_state.drag_visited.insert(gid)
    {
        target = Some(ClickTarget::DragGrid(r, c, distribute));
    }
    target
}

/// Draw the whole inventory window inside `area` (the screen, in points) as `plan` lays it out.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_compact(
    ctx: &egui::Context,
    area: egui::Rect,
    plan: CompactPlan,
    player_index: usize,
    ui_state: &mut CraftingUi,
    inventory: &Inventory,
    armour_slots: &[Option<ArmourItem>; 4],
    offhand: crate::hud_ui::OffhandBox,
    registry: &BlockRegistry,
    icons: &dyn IconSource,
    touch: bool,
) -> CompactOut {
    let mut out = CompactOut { click: ClickTarget::None, focused_rect: None, result_rect: None };
    let (_, drag_secondary) = ctx.input(|i| (i.pointer.any_down(), i.pointer.secondary_down()));
    let grid_size = ui_state.view_grid_size();
    let station_size = ui_state.station().grid_size();
    let has_offhand = !matches!(offhand, crate::hud_ui::OffhandBox::Hidden);
    let pad_focus = ui_state.pad_focus;
    let (slot, gap) = (plan.slot, plan.gap);
    // A finger taps; the mouse-tweaks drag paint is a mouse's gesture, and a finger that slid a
    // few points while tapping would otherwise start one.
    let sense_drag = !touch;

    let origin = (area.center() - plan.size / 2.0).round();
    let craft = crafting_size(slot, gap, grid_size, has_offhand);
    let bag = bag_size(slot, gap);
    let (title_rect, craft_rect, bag_rect, buttons_rect);
    if plan.side_by_side {
        let left_w = craft.x.max(buttons_width());
        title_rect = egui::Rect::from_min_size(origin, egui::vec2(left_w, TITLE_H));
        craft_rect = egui::Rect::from_min_size(origin + egui::vec2((left_w - craft.x) / 2.0, TITLE_H + ROW_GAP), craft);
        buttons_rect = egui::Rect::from_min_size(
            egui::pos2(origin.x + (left_w - buttons_width()) / 2.0, craft_rect.bottom() + ROW_GAP),
            egui::vec2(buttons_width(), BUTTONS_H),
        );
        bag_rect = egui::Rect::from_min_size(origin + egui::vec2(left_w + SIDE_GAP, TITLE_H + ROW_GAP), bag);
    } else {
        title_rect = egui::Rect::from_min_size(origin, egui::vec2(plan.size.x, TITLE_H));
        craft_rect = egui::Rect::from_min_size(origin + egui::vec2((plan.size.x - craft.x) / 2.0, TITLE_H + ROW_GAP), craft);
        bag_rect = egui::Rect::from_min_size(egui::pos2(origin.x + (plan.size.x - bag.x) / 2.0, craft_rect.bottom() + ROW_GAP), bag);
        buttons_rect = egui::Rect::from_min_size(
            egui::pos2(origin.x + (plan.size.x - buttons_width()) / 2.0, bag_rect.bottom() + ROW_GAP),
            egui::vec2(buttons_width(), BUTTONS_H),
        );
    }

    egui::Area::new(egui::Id::new(("craft_panel", player_index)))
        .fixed_pos(origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_min_size(plan.size);

            // Title.
            let title = if ui_state.is_table() { "Workbench" } else { "Crafting" };
            at(ui, title_rect, |ui| {
                let align = if plan.side_by_side { egui::Align::Min } else { egui::Align::Center };
                ui.with_layout(egui::Layout::top_down(align), |ui| {
                    ui.label(egui::RichText::new(title).size(16.0).color(TITLE_COLOR).strong());
                });
            });

            // Crafting block: armour, off-hand, grid, arrow, output.
            let col = slot + gap;
            for (i, piece) in armour_slots.iter().enumerate() {
                let pad_focused = pad_focus == Some(PadSlot::Armour(i));
                let rect = slot_rect(craft_rect.min, 0, i, slot, gap);
                let response = at(ui, rect, |ui| draw_armour_slot(ui, i, piece.as_ref(), icons, slot, pad_focused));
                if pad_focused {
                    out.focused_rect = Some(response.rect);
                }
                if response.clicked() {
                    out.click = ClickTarget::ArmourSlot(i);
                }
            }
            if has_offhand {
                // Level with the boots, as Minecraft puts it.
                let rect = slot_rect(craft_rect.min, 1, 3, slot, gap);
                let held = match offhand {
                    crate::hud_ui::OffhandBox::Holding(stack) => Some(stack.clone()),
                    _ => None,
                };
                let response = at(ui, rect, |ui| draw_item_slot(ui, &held, registry, icons, slot, false, false, false));
                if held.is_none() {
                    paint_shield_silhouette(ui.painter(), response.rect);
                }
                if response.clicked() {
                    out.click = ClickTarget::OffhandSlot(false);
                } else if response.secondary_clicked() {
                    out.click = ClickTarget::OffhandSlot(true);
                }
                response.on_hover_text("Off-hand (F swaps it with your hand)");
            }
            let grid_x = craft_rect.left() + col + if has_offhand { col } else { 0.0 } + 10.0;
            let grid_h = grid_size as f32 * slot + (grid_size as f32 - 1.0) * gap;
            let grid_origin = egui::pos2(grid_x, craft_rect.top() + (craft.y - grid_h) / 2.0);
            for r in 0..grid_size {
                for c in 0..grid_size {
                    let pad_focused = pad_focus == Some(PadSlot::Grid(r, c));
                    let rect = slot_rect(grid_origin, c, r, slot, gap);
                    let response = at(ui, rect, |ui| draw_item_slot(ui, &ui_state.grid[r][c], registry, icons, slot, false, pad_focused, sense_drag));
                    if pad_focused {
                        out.focused_rect = Some(response.rect);
                    }
                    if r >= station_size || c >= station_size {
                        continue; // shown, never clicked (LOW-A)
                    }
                    if let Some(target) = grid_slot_target(&response, ui_state, (r, c), grid_size, drag_secondary) {
                        out.click = target;
                    }
                }
            }
            let arrow_x = grid_origin.x + grid_size as f32 * slot + (grid_size as f32 - 1.0) * gap + 8.0;
            let arrow_rect = egui::Rect::from_center_size(egui::pos2(arrow_x + ARROW_W / 2.0, craft_rect.center().y), egui::vec2(ARROW_W, slot));
            at(ui, arrow_rect, |ui| {
                ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                    ui.label(egui::RichText::new("\u{2192}").size(20.0).color(LABEL_COLOR));
                });
            });
            let result_rect = egui::Rect::from_min_size(egui::pos2(arrow_x + ARROW_W + 8.0, craft_rect.center().y - slot / 2.0), egui::Vec2::splat(slot));
            let has_result = ui_state.result.is_some();
            let pad_focused = pad_focus == Some(PadSlot::Result);
            let response = at(ui, result_rect, |ui| draw_item_slot(ui, &ui_state.result, registry, icons, slot, has_result, pad_focused, false));
            if pad_focused {
                out.focused_rect = Some(response.rect);
            }
            out.result_rect = Some(response.rect);
            if response.clicked() {
                out.click = ClickTarget::ResultSlot;
            }

            // The bag (slots 9..36), then the hotbar (0..9) under it.
            for row in 0..4 {
                for column in 0..9 {
                    let (slot_idx, rect) = if row < 3 {
                        (9 + row * 9 + column, slot_rect(bag_rect.min, column, row, slot, gap))
                    } else {
                        let y = 3.0 * (slot + gap) + HOTBAR_GAP - gap;
                        (column, egui::Rect::from_min_size(bag_rect.min + egui::vec2(column as f32 * (slot + gap), y), egui::Vec2::splat(slot)))
                    };
                    let stack = inventory.slot(slot_idx);
                    let pad_focused = pad_focus == Some(PadSlot::Inv(slot_idx));
                    let response = at(ui, rect, |ui| draw_item_slot(ui, &stack.cloned(), registry, icons, slot, false, pad_focused, sense_drag));
                    if inventory.is_locked(slot_idx) {
                        mark_locked(ui, response.rect);
                    }
                    if pad_focused {
                        out.focused_rect = Some(response.rect);
                    }
                    if let Some(target) = inventory_slot_target(ui, &response, ui_state, slot_idx, stack.is_some(), drag_secondary) {
                        out.click = target;
                    }
                }
            }

            // Sort, Trash, Recipe Book.
            let mut x = buttons_rect.left();
            let button_rect = |i: usize, x: f32| egui::Rect::from_min_size(egui::pos2(x, buttons_rect.top()), egui::vec2(BUTTON_WIDTHS[i], BUTTONS_H));
            let sort = egui::Button::new(egui::RichText::new("\u{2195} Sort").size(13.0).color(egui::Color32::from_rgb(200, 210, 230)))
                .fill(egui::Color32::from_rgb(30, 36, 48))
                .corner_radius(egui::CornerRadius::same(6))
                .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 82, 110)));
            if ui.put(button_rect(0, x), sort).clicked() {
                out.click = ClickTarget::SortInventory;
            }
            x += BUTTON_WIDTHS[0] + BUTTON_GAP;
            let trash_armed = ui_state.cursor_item.is_some();
            let trash = egui::Button::new(egui::RichText::new("\u{1F5D1} Trash").size(13.0).color(if trash_armed {
                egui::Color32::from_rgb(230, 150, 150)
            } else {
                egui::Color32::from_rgb(120, 100, 100)
            }))
            .fill(egui::Color32::from_rgb(44, 28, 28))
            .corner_radius(egui::CornerRadius::same(6))
            .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 50, 50)));
            if ui.put(button_rect(1, x), trash).clicked() {
                out.click = ClickTarget::TrashCursor;
            }
            x += BUTTON_WIDTHS[1] + BUTTON_GAP;
            let book = egui::Button::new(egui::RichText::new("\u{1F4D6} Recipe Book").size(13.0).color(egui::Color32::from_rgb(220, 200, 120)))
                .fill(egui::Color32::from_rgb(48, 42, 28))
                .corner_radius(egui::CornerRadius::same(6))
                .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 78, 44)));
            if ui.put(button_rect(2, x), book).clicked() {
                out.click = ClickTarget::OpenBook;
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONEPLUS: egui::Vec2 = egui::vec2(800.0, 360.0);
    const PIXEL: egui::Vec2 = egui::vec2(914.0, 411.0);
    const PIXEL_PORTRAIT: egui::Vec2 = egui::vec2(412.0, 915.0);

    #[test]
    fn a_desktop_screen_keeps_the_desktop_layout() {
        assert_eq!(compact_plan(egui::vec2(1280.0, 720.0), 2, true), None);
        assert_eq!(compact_plan(egui::vec2(1920.0, 1080.0), 3, true), None);
        assert!(compact_plan(egui::vec2(1280.0, 600.0), 2, true).is_some(), "a short desktop window cannot hold the 650-point panel");
    }

    #[test]
    fn a_landscape_phone_puts_crafting_beside_the_bag_with_near_full_size_slots() {
        for screen in [ONEPLUS, PIXEL] {
            let plan = compact_plan(screen, 2, true).unwrap();
            assert!(plan.side_by_side, "{screen:?}: {plan:?}");
            assert!(plan.slot >= 44.0, "{screen:?}: {plan:?}");
            assert!(plan.size.x <= screen.x - 2.0 * MARGIN && plan.size.y <= screen.y - 2.0 * MARGIN, "{screen:?}: {plan:?}");
        }
    }

    #[test]
    fn a_portrait_phone_stacks_crafting_over_the_bag_in_the_width_it_has() {
        let plan = compact_plan(PIXEL_PORTRAIT, 2, true).unwrap();
        assert!(!plan.side_by_side, "{plan:?}");
        assert!(plan.slot >= MIN_SLOT && plan.slot < MAX_SLOT, "{plan:?}");
        assert!(plan.size.x <= PIXEL_PORTRAIT.x - 2.0 * MARGIN, "{plan:?}");
    }

    #[test]
    fn a_crafting_table_on_a_phone_still_fits_with_thumb_sized_slots() {
        for screen in [ONEPLUS, PIXEL, egui::vec2(640.0, 360.0), PIXEL_PORTRAIT] {
            let plan = compact_plan(screen, 3, true).unwrap();
            assert!(plan.slot >= MIN_SLOT, "{screen:?}: {plan:?}");
            assert!(plan.size.x <= screen.x - 2.0 * MARGIN && plan.size.y <= screen.y - 2.0 * MARGIN, "{screen:?}: {plan:?}");
        }
    }

    #[test]
    fn a_screen_too_small_for_the_smallest_slot_uses_it_anyway() {
        let plan = compact_plan(egui::vec2(300.0, 200.0), 3, true).unwrap();
        assert_eq!(plan.slot, MIN_SLOT);
    }
}

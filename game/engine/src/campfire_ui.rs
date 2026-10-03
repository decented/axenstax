//! Spec 30 — Campfire hover-label UI.
//!
//! When the player's crosshair targets a campfire (lit or unlit) and
//! the campfire is within strike distance, this overlay shows the
//! campfire's current state: fuel ticks remaining, lit/smouldering/
//! cold mode, smoke output, heat radius, and emission. Pure render —
//! no state mutation; reads `CampfireData` through the world.

use crate::block::BlockRegistry;
use crate::campfire::{self, CampfireData, COOK_TICKS_PER_ITEM, heat_radius_blocks};
use crate::screen::ViewportRect;

/// Pure: compute the campfire hover panel's title + body lines from state.
/// Separated from the egui draw so it's unit-testable. `lit_block` is true
/// when the targeted block id is `CAMPFIRE` (vs `CAMPFIRE_UNLIT`).
///
/// Four states, each guiding the next step of the "add fuel → light → cook"
/// arc:
/// - **Lit** (lit block + fuel): fuel left, heat, smoke, + cooking lines.
/// - **Ready to light** (unlit but fuelled): the state players got stuck in —
///   now it shows the fuel and tells you how to light it, instead of the old
///   "Cold" with the fuel hidden.
/// - **Smouldering**: warm-window countdown + "drop fuel to relight".
/// - **Cold** (no fuel): "add fuel first, then light".
///
/// Cooking lines are appended in every state so meat sitting on an unlit fire
/// is never invisible.
pub fn campfire_hover_text(
    cf: &CampfireData,
    lit_block: bool,
    registry: &BlockRegistry,
) -> (&'static str, Vec<String>) {
    let title;
    let mut body: Vec<String> = Vec::new();
    let actively_cooking = lit_block && cf.fuel_ticks > 0;
    if actively_cooking {
        title = "Campfire — Lit";
        body.push(format!("Fuel: {} s left", cf.fuel_ticks / 20));
        body.push(format!("Heat: radius {:.0} blocks", heat_radius_blocks(cf.fuel_ticks)));
        if cf.smoke_ticks > 0 {
            body.push("Smoke: on".to_string());
        }
    } else if cf.fuel_ticks > 0 {
        // Unlit but fuelled — ready to light.
        title = "Campfire — Ready to light";
        body.push(format!("Fuel: {} s", cf.fuel_ticks / 20));
        body.push("Strike with a stick, or use flint & steel".to_string());
    } else if cf.is_smouldering() {
        title = "Campfire — Smouldering";
        body.push(format!("Still warm for {} s", cf.smoulder_ticks / 20));
        body.push("Drop fuel to relight".to_string());
    } else {
        title = "Campfire — Cold";
        body.push("Add fuel (wood), then light with a stick or flint & steel".to_string());
    }
    body.extend(cook_lines(cf, registry, actively_cooking));
    (title, body)
}

/// Pure: one line per occupied cook slot — progress % while cooking, or a
/// "ready" prompt once mature. When the fire is actively cooking but every
/// slot is empty, a single hint nudges the player to add raw food.
fn cook_lines(cf: &CampfireData, registry: &BlockRegistry, actively_cooking: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut any_occupied = false;
    for slot in cf.slots.iter() {
        let Some(raw) = slot.item else { continue };
        any_occupied = true;
        let raw_name = crate::item::Item::Material(raw).name(registry);
        if slot.progress_ticks >= COOK_TICKS_PER_ITEM
            && let Some(cooked) = campfire::cooked_variant(raw) {
                let cooked_name = crate::item::Item::Material(cooked).name(registry);
                lines.push(format!("Ready: {cooked_name} (empty hand to take)"));
                continue;
            }
        let pct = (slot.progress_ticks.saturating_mul(100) / COOK_TICKS_PER_ITEM).min(100);
        lines.push(format!("Cooking: {raw_name} — {pct}%"));
    }
    if actively_cooking && !any_occupied {
        lines.push("Right-click raw meat to cook".to_string());
    }
    lines
}

/// Draw the campfire hover label for one player. Caller passes the
/// current `target_block` (from raycast) and the campfire data; this
/// function decides whether to render and how to format the text.
pub fn draw_campfire_hover_label(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    cf: &CampfireData,
    lit_block: bool,
    registry: &BlockRegistry,
) {
    let (title, body) = campfire_hover_text(cf, lit_block, registry);

    // Floating panel anchored near the top-centre of the player's
    // viewport. egui doesn't easily project a world-space point to
    // screen here without the camera plumbing, so we anchor in
    // viewport coordinates as a v1 pragmatism — visible without
    // needing to look up.
    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 150.0,
        viewport.y as f32 + viewport.height as f32 / 3.0,
    );
    egui::Area::new(egui::Id::new(("campfire_hover", player_index)))
        .fixed_pos(panel_origin)
        .interactable(false)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(15, 12, 8, 220))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(280.0);
                        ui.label(
                            egui::RichText::new(title)
                                .size(18.0)
                                .color(egui::Color32::from_rgb(255, 195, 100))
                                .strong(),
                        );
                        ui.add_space(4.0);
                        for line in &body {
                            ui.label(
                                egui::RichText::new(line)
                                    .size(13.0)
                                    .color(egui::Color32::LIGHT_GRAY),
                            );
                        }
                    });
                });
        });

    // Avoid the "unused if no other emissive consumers" warning when
    // future polish removes the helper-touch above.
    let _ = campfire::SMOULDER_TICKS;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockRegistry;
    use crate::campfire::{CampfireData, CookSlot, COOK_TICKS_PER_ITEM};
    use crate::item::MaterialId;

    #[test]
    fn hover_unlit_but_fueled_reads_ready_and_shows_fuel() {
        // The exact state the player got stuck in: fuel was added but the
        // fire is unlit. Old UI said "Cold" and hid the fuel; now it says
        // "Ready to light", shows the fuel, and tells you how to light it.
        let reg = BlockRegistry::new();
        let mut cf = CampfireData::default();
        cf.fuel_ticks = 60 * 20; // 60 s
        let (title, body) = campfire_hover_text(&cf, false, &reg);
        assert_eq!(title, "Campfire — Ready to light");
        assert!(
            body.iter().any(|l| l.contains("Fuel") && l.contains("60")),
            "shows fuel seconds: {body:?}"
        );
        assert!(
            body.iter().any(|l| {
                let l = l.to_lowercase();
                l.contains("stick") || l.contains("flint")
            }),
            "tells the player how to light it: {body:?}"
        );
    }

    #[test]
    fn hover_cold_guides_add_fuel_first() {
        let reg = BlockRegistry::new();
        let cf = CampfireData::default(); // no fuel, no smoulder
        let (title, body) = campfire_hover_text(&cf, false, &reg);
        assert_eq!(title, "Campfire — Cold");
        assert!(
            body.iter().any(|l| l.to_lowercase().contains("add fuel")),
            "guides 'add fuel': {body:?}"
        );
    }

    #[test]
    fn hover_lit_shows_cooking_progress() {
        let reg = BlockRegistry::new();
        let mut cf = CampfireData::default();
        cf.fuel_ticks = 100;
        cf.slots[0] = CookSlot {
            item: Some(MaterialId::RawBeef),
            progress_ticks: COOK_TICKS_PER_ITEM / 2,
        };
        let (title, body) = campfire_hover_text(&cf, true, &reg);
        assert_eq!(title, "Campfire — Lit");
        assert!(
            body.iter().any(|l| l.contains("Raw Beef") && l.contains("50")),
            "cooking line with progress %: {body:?}"
        );
    }

    #[test]
    fn hover_shows_ready_cooked_item() {
        let reg = BlockRegistry::new();
        let mut cf = CampfireData::default();
        cf.fuel_ticks = 100;
        cf.slots[0] = CookSlot {
            item: Some(MaterialId::RawBeef),
            progress_ticks: COOK_TICKS_PER_ITEM,
        };
        let (_t, body) = campfire_hover_text(&cf, true, &reg);
        assert!(
            body.iter().any(|l| l.contains("Cooked Beef") && l.to_lowercase().contains("ready")),
            "shows the cooked item as ready: {body:?}"
        );
    }

    #[test]
    fn hover_cooking_line_visible_even_when_cold() {
        // Meat placed on an unlit fire used to be invisible — now it's listed
        // so "I chucked meat on it and nothing happened" can't recur silently.
        let reg = BlockRegistry::new();
        let mut cf = CampfireData::default(); // cold
        cf.slots[0] = CookSlot {
            item: Some(MaterialId::RawBeef),
            progress_ticks: 0,
        };
        let (_t, body) = campfire_hover_text(&cf, false, &reg);
        assert!(
            body.iter().any(|l| l.contains("Raw Beef")),
            "raw item is shown even when the fire is cold: {body:?}"
        );
    }
}

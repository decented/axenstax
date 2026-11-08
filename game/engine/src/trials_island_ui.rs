//! The Trials island's doorway card, drawn: the gallery's own hero view for
//! the trial (`trials_gallery_ui::draw_hero`), in a panel at the side of the
//! screen so the doorway stays in view and the player can step back out.
//! Play, or Enter, launches the trial; nothing else in the card acts in the
//! world (the ghost buttons and "How it works" belong to the lobby).

use crate::brand;
use crate::menu::MenuAction;
use crate::trials_gallery::Card;

/// Draw the card for one frame. Returns true when Play (or Enter) was pressed.
pub(crate) fn draw_card(ctx: &egui::Context, card: &Card, postcard: Option<egui::TextureId>) -> bool {
    // Inside the TV-safe area (home page overhaul spec §2), never larger than it.
    let safe = crate::ui_safe_area::rect(ctx);
    const MARGIN: f32 = 16.0;
    let w = (safe.width() * 0.45).clamp(300.0, 520.0).min(safe.width() - 2.0 * MARGIN).max(0.0);
    let h = (safe.height() - 2.0 * MARGIN).clamp(0.0, 720.0);
    let icons = crate::egui_integration::block_texture_handles(ctx);
    let mut play = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let fill = egui::Color32::from_rgba_unmultiplied(
        brand::DEEP_FRONTIER.r(),
        brand::DEEP_FRONTIER.g(),
        brand::DEEP_FRONTIER.b(),
        238,
    );
    egui::Area::new(egui::Id::new("trials_island_card"))
        .pivot(egui::Align2::RIGHT_CENTER)
        .fixed_pos(egui::pos2(safe.max.x, safe.center().y))
        .constrain_to(safe)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(fill)
                .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
                .corner_radius(egui::CornerRadius::same(16))
                .inner_margin(egui::Margin::same(MARGIN as i8))
                .show(ui, |ui| {
                    ui.set_width(w);
                    ui.set_height(h);
                    ui.label(
                        egui::RichText::new("Step back out of the doorway to close this.")
                            .size(12.5)
                            .color(brand::MUTED_TEXT),
                    );
                    ui.add_space(6.0);
                    let hero = crate::trials_gallery_ui::draw_hero(ui, card, &icons, false, postcard, false);
                    if matches!(hero.action, Some(MenuAction::PlayScenario { .. })) {
                        play = true;
                    }
                });
        });
    play
}

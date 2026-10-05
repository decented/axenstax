//! The terminal showcase exit screen — the kiosk's dead-end (Spec 2026-06-19
//! §8/§10 Phase 2). Full-screen, no path back to the lobby. Renders the visitor's
//! basket as the configured exit action (a board / CTA for now). Structure
//! mirrors `loading_screen::draw_loading_screen` (bg Area + transparent
//! CentralPanel + voxel-hero brand moment).

use crate::brand;
use crate::showcase::{Basket, ExitAction};

/// The lines the exit screen shows for `action`, derived from the basket. Pure
/// so the content is unit-testable without egui.
pub fn summary_lines(action: ExitAction, basket: &Basket) -> Vec<String> {
    match action {
        ExitAction::Board => {
            if basket.is_empty() {
                vec!["Thanks for visiting the gallery.".to_string()]
            } else {
                let mut lines = vec![format!("You saved {} piece(s):", basket.len())];
                for item in &basket.items {
                    let label = if item.label.trim().is_empty() {
                        item.image_ref.clone()
                    } else {
                        item.label.clone()
                    };
                    lines.push(format!("\u{2022} {label}"));
                }
                lines
            }
        }
    }
}

/// Draw the full-screen terminal exit screen. No interactive return to the lobby
/// — this is the dead-end. (Auto-loop, if configured, resets the session on a
/// timer driven by the caller.)
pub fn draw_exit_screen(ctx: &egui::Context, action: ExitAction, basket: &Basket) {
    ctx.request_repaint();
    const BG: egui::Color32 = brand::DEEP_FRONTIER;
    egui::Area::new(egui::Id::new("showcase_exit_bg"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.painter().rect_filled(ui.max_rect(), 0.0, BG);
        });
    // `.show(ctx, ..)` is deprecated in favour of `.show_inside(ui, ..)`, but this
    // is a genuine top-level panel (no enclosing Ui) — egui 0.34 has no
    // non-deprecated top-level entry point for CentralPanel.
    #[allow(deprecated)]
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
        .show(ctx, |ui| {
            let available = ui.available_size();
            ui.vertical_centered(|ui| {
                ui.add_space((available.y / 2.0 - 140.0).max(20.0));
                ui.label(
                    egui::RichText::new("AXE'N'STAX")
                        .size(40.0)
                        .color(brand::STONE)
                        .strong()
                        .extra_letter_spacing(3.0),
                );
                ui.add_space(18.0);
                for line in summary_lines(action, basket) {
                    ui.label(
                        egui::RichText::new(line)
                            .size(16.0)
                            .color(brand::STONE),
                    );
                    ui.add_space(4.0);
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::showcase::BasketItem;

    fn item(label: &str, image: &str) -> BasketItem {
        BasketItem {
            image_ref: image.to_string(),
            label: label.to_string(),
            link: None,
            sku: None,
            price: None,
        }
    }

    #[test]
    fn board_shows_thanks_when_empty() {
        let lines = summary_lines(ExitAction::Board, &Basket::new());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_lowercase().contains("thanks"));
    }

    #[test]
    fn board_lists_collected_labels() {
        let mut b = Basket::new();
        b.add(item("Sunrise", "a.png"));
        b.add(item("", "b.png")); // empty label → falls back to image_ref
        let lines = summary_lines(ExitAction::Board, &b);
        assert!(lines[0].contains("2 piece"));
        assert!(lines.iter().any(|l| l.contains("Sunrise")));
        assert!(lines.iter().any(|l| l.contains("b.png")), "empty label falls back to ref");
    }
}

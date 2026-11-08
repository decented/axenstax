//! The end card's flower for a `Place` trial (living world spec §6.1): four
//! petals, life, ideas, people and time, drawn with egui shapes. A petal's size
//! is that term's share of the recovery; ideas and time are not built yet, so
//! they are drawn closed (a small, muted bud). One sentence and Satoshi's end
//! line go under it.

use crate::place_trial::Recovery;

/// Overall size of the flower's canvas.
const CANVAS: f32 = 150.0;
/// An open petal's length at a share of 0 and at a share of 1.
const OPEN_MIN: f32 = 16.0;
const OPEN_MAX: f32 = 62.0;
/// A closed petal (a bud).
const BUD_LEN: f32 = 14.0;
const BUD_WIDTH: f32 = 5.0;

/// The four directions, clockwise from the top: life, ideas, people, time.
const DIRS: [(f32, f32); 4] = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)];

/// The outline of one petal rooted at `root`, pointing along the unit vector
/// `dir`: an ellipse `len` long and `width` wide at its fullest.
pub fn petal_outline(root: egui::Pos2, dir: (f32, f32), len: f32, width: f32) -> Vec<egui::Pos2> {
    const STEPS: usize = 28;
    let (dx, dy) = dir;
    let (nx, ny) = (-dy, dx);
    (0..STEPS)
        .map(|i| {
            let t = std::f32::consts::TAU * i as f32 / STEPS as f32;
            // Along the petal 0..len, across it -width/2..width/2.
            let along = len * 0.5 * (1.0 + t.cos());
            let across = width * 0.5 * t.sin();
            egui::pos2(root.x + dx * along + nx * across, root.y + dy * along + ny * across)
        })
        .collect()
}

/// The length of an open petal at `share` (0..=1).
pub fn open_petal_len(share: f32) -> f32 {
    OPEN_MIN + (OPEN_MAX - OPEN_MIN) * share.clamp(0.0, 1.0)
}

/// Draw the flower, then the sentence and Satoshi's line, centred in `ui`.
pub fn draw_recovery(ui: &mut egui::Ui, recovery: &Recovery, satoshi_line: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(CANVAS, CANVAS), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let centre = rect.center();
    let sizes = recovery.petals();
    // Life and people open; ideas and time closed.
    let open_colours = [
        egui::Color32::from_rgb(236, 138, 170),
        egui::Color32::TRANSPARENT,
        egui::Color32::from_rgb(244, 190, 120),
        egui::Color32::TRANSPARENT,
    ];
    let closed = egui::Color32::from_rgba_unmultiplied(150, 140, 165, 190);
    for (i, dir) in DIRS.iter().enumerate() {
        let is_open = i == 0 || i == 2;
        if is_open {
            let len = open_petal_len(sizes[i]);
            painter.add(egui::Shape::convex_polygon(
                petal_outline(centre, *dir, len, len * 0.62),
                open_colours[i],
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 120)),
            ));
        } else {
            painter.add(egui::Shape::convex_polygon(
                petal_outline(centre, *dir, BUD_LEN, BUD_WIDTH),
                closed,
                egui::Stroke::NONE,
            ));
        }
    }
    painter.circle_filled(centre, 7.0, egui::Color32::from_rgb(252, 226, 130));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(recovery.sentence()).size(17.0).color(egui::Color32::from_rgb(170, 230, 170)));
    if !satoshi_line.is_empty() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(satoshi_line).size(15.0).italics().color(egui::Color32::from_rgb(210, 220, 255)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place_trial::{recovery, Terms};

    #[test]
    fn a_petal_runs_from_its_root_out_along_its_direction() {
        let root = egui::pos2(10.0, 10.0);
        let pts = petal_outline(root, (0.0, -1.0), 40.0, 20.0);
        let top = pts.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let bottom = pts.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        assert!((bottom - 10.0).abs() < 1e-3, "rooted at the centre: {bottom}");
        assert!((top - (-30.0)).abs() < 1e-3, "40 long: {top}");
        let left = pts.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let right = pts.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!(((right - left) - 20.0).abs() < 0.6, "about 20 wide");
    }

    #[test]
    fn bigger_share_means_a_longer_open_petal_and_buds_stay_small() {
        assert!(open_petal_len(0.8) > open_petal_len(0.2));
        assert!(open_petal_len(0.0) > BUD_LEN, "even the smallest open petal outgrows a bud");
        assert_eq!(open_petal_len(7.0), open_petal_len(1.0), "clamped");
        let r = recovery(Terms { life: 0.0, people: 0.0 }, Terms { life: 0.6, people: 0.0 });
        let p = r.petals();
        assert!(p[0] > p[2] && p[1] == 0.0 && p[3] == 0.0);
    }

    #[test]
    fn the_flower_draws_without_panicking_in_a_headless_egui_pass() {
        let ctx = egui::Context::default();
        let r = recovery(Terms { life: 0.05, people: 0.0 }, Terms { life: 0.7, people: 0.4 });
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| draw_recovery(ui, &r, "Look at it."));
        });
    }
}

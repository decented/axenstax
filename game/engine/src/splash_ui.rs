//! Splash screen — shows on launch as a brand moment.
//!
//! Draws a Deep Frontier full-screen background, the Copperline mark
//! (`brand::show_mark`), the "AXE'N'STAX" title over a soft copper glow, the
//! "PROOF OF PLAY" tagline and a thin loading bar. Colours are the brand tokens
//! from `brand.rs` (`brand/BRAND-GUIDELINES.md`).

use crate::brand;
use web_time::{Duration, Instant};

/// How long the splash screen displays before transitioning to menu.
const SPLASH_DURATION: Duration = Duration::from_millis(2000);

/// Theme: Deep Frontier `#0D1B1E` (matches the HTML boot screen, so no flash).
const BG_COLOR: egui::Color32 = brand::DEEP_FRONTIER;
/// Width of the mark, px (the web boot screen uses 168).
const MARK_WIDTH: f32 = 168.0;

/// Splash screen state.
pub struct SplashState {
    pub started_at: Instant,
}

impl SplashState {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }

    /// Returns true when the splash should transition to menu.
    pub fn is_done(&self) -> bool {
        self.started_at.elapsed() >= SPLASH_DURATION
    }

    /// Progress 0.0 to 1.0 through the splash duration.
    pub fn progress(&self) -> f32 {
        let elapsed = self.started_at.elapsed().as_secs_f32();
        let total = SPLASH_DURATION.as_secs_f32();
        (elapsed / total).min(1.0)
    }
}

/// Draw the splash screen. Returns true when ready to transition.
pub fn draw_splash(ctx: &egui::Context, state: &SplashState) -> bool {
    let progress = state.progress();

    // Full-screen dark background
    egui::Area::new(egui::Id::new("splash_bg"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .interactable(false)
        .show(ctx, |ui| {
            let screen = ui.max_rect();
            ui.painter().rect_filled(screen, 0.0, BG_COLOR);
        });

    // `.show(ctx, ..)` is deprecated in favour of `.show_inside(ui, ..)`, but this
    // is a genuine top-level panel (no enclosing Ui) — egui 0.34 has no
    // non-deprecated top-level entry point for CentralPanel.
    #[allow(deprecated)]
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
        .show(ctx, |ui| {
            let available = ui.available_size();
            let center_y = available.y / 2.0;

            ui.vertical_centered(|ui| {
                // Mark (~123 px tall) + title + tagline + bar ≈ 260 px of content.
                ui.add_space((center_y - 150.0).max(8.0));

                // Mark — fades in with the title.
                let title_alpha = ((progress / 0.3).min(1.0) * 255.0) as u8;
                brand::show_mark(ui, MARK_WIDTH, title_alpha, 0.0);
                ui.add_space(8.0);

                // Subtle copper glow behind title (painted directly)
                let title_center = ui.cursor().left_top() + egui::vec2(available.x / 2.0, 0.0);
                ui.painter().rect_filled(
                    egui::Rect::from_center_size(
                        title_center + egui::vec2(0.0, 10.0),
                        egui::vec2(400.0, 100.0),
                    ),
                    50.0,
                    brand::with_alpha(brand::COPPER, 14),
                );

                // Title — fade in over first 0.5s
                ui.label(
                    egui::RichText::new("AXE'N'STAX")
                        .size(56.0)
                        .color(brand::with_alpha(brand::STONE, title_alpha))
                        .strong()
                        .extra_letter_spacing(4.0),
                );

                ui.add_space(10.0);

                // Tagline — fade in over 0.3-0.6s
                let tagline_alpha = (((progress - 0.2) / 0.3).clamp(0.0, 1.0) * 255.0) as u8;
                ui.label(
                    egui::RichText::new("PROOF  OF  PLAY")
                        .size(14.0)
                        .color(brand::with_alpha(brand::COPPER, tagline_alpha))
                        .extra_letter_spacing(8.0),
                );

                ui.add_space(44.0);

                // Loading bar — appears after 0.4s, fills over remaining time
                let bar_alpha = (((progress - 0.3) / 0.2).clamp(0.0, 1.0) * 255.0) as u8;
                if bar_alpha > 0 {
                    let bar_progress = ((progress - 0.4) / 0.6).clamp(0.0, 1.0);
                    let bar_width = 240.0;
                    let bar_height = 3.0;

                    let bar_rect = egui::Rect::from_center_size(
                        ui.cursor().left_top() + egui::vec2(available.x / 2.0, 0.0),
                        egui::vec2(bar_width, bar_height),
                    );

                    // Background
                    ui.painter().rect_filled(
                        bar_rect,
                        2.0,
                        egui::Color32::from_rgba_unmultiplied(43, 43, 43, bar_alpha), // Deep Rock #2B2B2B
                    );

                    // Fill
                    let fill_rect = egui::Rect::from_min_size(
                        bar_rect.left_top(),
                        egui::vec2(bar_width * bar_progress, bar_height),
                    );
                    ui.painter().rect_filled(
                        fill_rect,
                        2.0,
                        brand::with_alpha(brand::COPPER, bar_alpha),
                    );
                }
            });
        });

    // Request continuous repaint for animation
    ctx.request_repaint();

    state.is_done()
}


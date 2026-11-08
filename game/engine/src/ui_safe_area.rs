//! The TV-safe area every lobby and Trials page draws inside (home page
//! overhaul spec §2). A television can crop the edges of the picture, and a
//! sofa is far from the screen, so:
//!
//! * content sits in a rect inset by `max(16 pt, 5%)` of the screen on each
//!   side (backgrounds still fill the whole screen);
//! * on a large screen (safe width at least [`LARGE_SAFE_WIDTH`]) lobby type
//!   and tiles grow by [`LARGE_SCALE`];
//! * dialogs are centred in the safe rect and never larger than it;
//! * the keyboard-focused item wears the gold ring ([`paint_focus_ring`]).
//!
//! The geometry is pure (`Rect` in, `Rect` out) so it is unit-tested; the few
//! egui helpers below only apply it.

use crate::brand;

/// Smallest inset on any side, in points.
pub const MIN_INSET: f32 = 16.0;
/// Inset as a share of the screen's width (left and right) or height (top and bottom).
pub const INSET_FRACTION: f32 = 0.05;
/// From this safe width up, lobby type and tiles scale by [`LARGE_SCALE`].
pub const LARGE_SAFE_WIDTH: f32 = 1500.0;
/// The large-screen scale (a 1920 x 1080 television).
pub const LARGE_SCALE: f32 = 1.25;

/// Body text, before scaling (spec: at least 15 pt).
pub const BODY: f32 = 15.0;
/// Tile titles, before scaling (spec: 22-28 pt).
pub const TILE_TITLE: f32 = 24.0;
/// Page titles, before scaling (spec: 30-36 pt).
pub const PAGE_TITLE: f32 = 32.0;
/// The smallest hit target height, before scaling (spec: 44 pt).
pub const HIT: f32 = 44.0;

/// The horizontal and vertical inset for a screen of this size.
pub fn insets(screen: egui::Vec2) -> egui::Vec2 {
    egui::vec2(
        (screen.x * INSET_FRACTION).max(MIN_INSET),
        (screen.y * INSET_FRACTION).max(MIN_INSET),
    )
}

/// The safe rect inside `screen`. A screen too small for the insets gives a
/// zero-size rect at its centre, never an inverted one.
pub fn safe_rect(screen: egui::Rect) -> egui::Rect {
    let i = insets(screen.size());
    let c = screen.center();
    let (mut x0, mut x1) = (screen.min.x + i.x, screen.max.x - i.x);
    let (mut y0, mut y1) = (screen.min.y + i.y, screen.max.y - i.y);
    if x0 > x1 {
        (x0, x1) = (c.x, c.x);
    }
    if y0 > y1 {
        (y0, y1) = (c.y, c.y);
    }
    egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
}

/// The lobby's size scale for a safe rect of this width.
pub fn scale_for(safe_width: f32) -> f32 {
    if safe_width >= LARGE_SAFE_WIDTH { LARGE_SCALE } else { 1.0 }
}

/// The safe rect for this frame (from the content rect: on a phone it already
/// leaves out the notch and navigation bar).
pub fn rect(ctx: &egui::Context) -> egui::Rect {
    safe_rect(ctx.content_rect())
}

/// Sizes for one frame of a lobby page, already scaled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub safe: egui::Rect,
    pub scale: f32,
}

impl Metrics {
    pub fn of(ctx: &egui::Context) -> Self {
        let safe = rect(ctx);
        Self { safe, scale: scale_for(safe.width()) }
    }
    /// A size in points, scaled.
    pub fn px(&self, pt: f32) -> f32 {
        pt * self.scale
    }
    pub fn body(&self) -> f32 {
        self.px(BODY)
    }
    pub fn tile_title(&self) -> f32 {
        self.px(TILE_TITLE)
    }
    pub fn page_title(&self) -> f32 {
        self.px(PAGE_TITLE)
    }
    pub fn hit(&self) -> f32 {
        self.px(HIT)
    }
}

/// Run `add` in a child Ui bounded to the safe rect. Use inside a full-screen
/// panel whose frame paints the background.
pub fn inside<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let safe = rect(ui.ctx()).intersect(ui.max_rect());
    ui.scope_builder(egui::UiBuilder::new().max_rect(safe), |ui| {
        ui.set_clip_rect(safe.expand(4.0).intersect(ui.ctx().content_rect()));
        add(ui)
    })
    .inner
}

/// Paint the gold ring round whatever widget has keyboard focus this frame,
/// on top of everything (one call per lobby frame covers every page, so custom
/// tiles and plain buttons ring the same way). Nothing focused paints nothing.
pub fn paint_focus_ring(ctx: &egui::Context) {
    let Some(id) = ctx.memory(|m| m.focused()) else { return };
    let Some(resp) = ctx.read_response(id) else { return };
    let rect = resp.interact_rect.expand(3.0).intersect(ctx.content_rect());
    if !rect.is_positive() {
        return;
    }
    ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("lobby_focus_ring")))
        .rect_stroke(rect, 9.0, egui::Stroke::new(2.5_f32, brand::LANTERN), egui::StrokeKind::Outside);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))
    }

    #[test]
    fn a_television_is_inset_five_percent_and_scaled_up() {
        let s = safe_rect(screen(1920.0, 1080.0));
        assert_eq!(s.min, egui::pos2(96.0, 54.0));
        assert_eq!(s.max, egui::pos2(1824.0, 1026.0));
        assert_eq!(scale_for(s.width()), LARGE_SCALE);
    }

    #[test]
    fn a_desktop_and_a_tablet_are_inset_but_not_scaled() {
        for (w, h) in [(1280.0, 800.0), (834.0, 1112.0)] {
            let s = safe_rect(screen(w, h));
            assert!((s.min.x - w * 0.05).abs() < 1e-3 && (s.min.y - h * 0.05).abs() < 1e-3, "{w}x{h}: {s:?}");
            assert_eq!(scale_for(s.width()), 1.0, "{w}x{h}");
        }
    }

    #[test]
    fn a_phone_is_inset_five_percent_and_not_scaled() {
        let s = safe_rect(screen(390.0, 844.0));
        assert!((s.min.x - 19.5).abs() < 1e-3 && (s.min.y - 42.2).abs() < 1e-3, "{s:?}");
        assert_eq!(scale_for(s.width()), 1.0);
        // A narrower phone keeps the 16-point floor across.
        assert_eq!(safe_rect(screen(300.0, 640.0)).min.x, MIN_INSET);
    }

    #[test]
    fn the_floor_wins_on_a_small_window_and_the_rect_never_inverts() {
        let s = safe_rect(screen(200.0, 120.0));
        assert_eq!(s.min, egui::pos2(16.0, 16.0));
        let tiny = safe_rect(screen(20.0, 10.0));
        assert!(tiny.width() >= 0.0 && tiny.height() >= 0.0, "{tiny:?}");
        assert!(screen(20.0, 10.0).contains_rect(tiny));
    }

    #[test]
    fn an_offset_screen_keeps_its_origin() {
        let r = egui::Rect::from_min_size(egui::pos2(0.0, 30.0), egui::vec2(1000.0, 600.0));
        let s = safe_rect(r);
        assert_eq!(s.min, egui::pos2(50.0, 60.0));
        assert!(r.contains_rect(s));
    }

    #[test]
    fn the_scale_steps_at_the_large_width() {
        assert_eq!(scale_for(LARGE_SAFE_WIDTH - 1.0), 1.0);
        assert_eq!(scale_for(LARGE_SAFE_WIDTH), LARGE_SCALE);
    }

    #[test]
    fn the_spec_sizes_hold_before_scaling() {
        assert!(BODY >= 15.0);
        assert!((22.0..=28.0).contains(&TILE_TITLE));
        assert!((30.0..=36.0).contains(&PAGE_TITLE));
        assert!(HIT >= 44.0);
    }
}

//! Copperline brand assets for the native + WASM engine.
//!
//! One shared loader for the logo mark (so the splash screen, the world-load
//! screen and anything else that wants it share a single GPU texture), the
//! brand colour tokens the engine screens draw with, and the native window icon.
//!
//! Source of truth for tokens: `brand/BRAND-GUIDELINES.md`. The PNGs are
//! rendered from the SVGs in `brand/svg/` by `node brand/render-rasters.mjs`
//! (see the "Rust engine" block at the top of that script) — never hand-edit
//! them. The mark is an egui texture, **separate from the block texture array**,
//! so the 256-layer array limit / WebGPU crash guard does not apply.

use egui::{Color32, ColorImage, Context, Id, TextureHandle, TextureOptions};

/// Deep Frontier `#0D1B1E` — primary dark / UI background.
pub const DEEP_FRONTIER: Color32 = Color32::from_rgb(13, 27, 30);
/// Deep Rock `#2B2B2B` — neutral dark (widget fills, outlines).
pub const DEEP_ROCK: Color32 = Color32::from_rgb(43, 43, 43);
/// Forest Green `#2E6B43` — primary green (selection fill).
pub const FOREST: Color32 = Color32::from_rgb(46, 107, 67);
/// Lantern `#F4C16F` — warm highlight (focus ring).
pub const LANTERN: Color32 = Color32::from_rgb(244, 193, 111);
/// Sky Blue `#3F7FBF` — informational / selected-card outline.
pub const SKY: Color32 = Color32::from_rgb(63, 127, 191);
/// Mountain Stone `#E6E0D1` — primary light text.
pub const STONE: Color32 = Color32::from_rgb(230, 224, 209);
/// Copper `#D27B3E` — primary warm accent (path, selected states, bars).
pub const COPPER: Color32 = Color32::from_rgb(210, 123, 62);

/// `mark.png` is rendered at 336x246 (axenstax-mark-flat.svg, transparent).
const MARK_PNG: &[u8] = include_bytes!("../assets/brand/mark.png");
const MARK_ASPECT: f32 = 336.0 / 246.0;

/// `c` with its alpha replaced by `a` (straight, i.e. unmultiplied, alpha).
pub fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// The Copperline mark as an egui texture. Decoded and uploaded once per
/// `egui::Context`, then served from the context's temp data. `None` only if
/// the embedded PNG somehow fails to decode (callers just skip drawing it).
pub fn mark_texture(ctx: &Context) -> Option<TextureHandle> {
    let id = Id::new("axenstax_brand_mark_texture");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(tex);
    }
    let img = image::load_from_memory(MARK_PNG).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let tex = ctx.load_texture(
        "axenstax_brand_mark",
        ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
        TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// Lay out the mark `width` px wide in the current (centred) column and paint
/// it, faded by `alpha` (0..=255) and shifted down by `y_offset` (for a bob
/// animation; the layout slot does not move).
pub fn show_mark(ui: &mut egui::Ui, width: f32, alpha: u8, y_offset: f32) {
    let size = egui::vec2(width, width / MARK_ASPECT);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if let Some(tex) = mark_texture(ui.ctx()) {
        ui.painter().image(
            tex.id(),
            rect.translate(egui::vec2(0.0, y_offset)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::from_white_alpha(alpha),
        );
    }
}

/// The window/taskbar icon (128px rounded Copperline tile). Native desktop
/// only: the web build has no window icon and Android uses the launcher icon,
/// so the PNG is not even embedded there.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub fn window_icon() -> Option<winit::window::Icon> {
    const APP_ICON_PNG: &[u8] = include_bytes!("../assets/brand/app-icon-128.png");
    let img = image::load_from_memory(APP_ICON_PNG).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_png_decodes_at_expected_size() {
        let img = image::load_from_memory(MARK_PNG).expect("mark.png decodes").to_rgba8();
        assert_eq!(img.dimensions(), (336, 246));
        // Transparent canvas: the corner is fully see-through.
        assert_eq!(img.get_pixel(0, 0).0[3], 0);
    }

    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    #[test]
    fn window_icon_builds() {
        assert!(window_icon().is_some());
    }

    #[test]
    fn mark_texture_is_cached_per_context() {
        let ctx = Context::default();
        let a = mark_texture(&ctx).expect("texture");
        let b = mark_texture(&ctx).expect("texture");
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn with_alpha_keeps_rgb() {
        let c = with_alpha(COPPER, 255);
        assert_eq!((c.r(), c.g(), c.b(), c.a()), (210, 123, 62, 255));
    }
}

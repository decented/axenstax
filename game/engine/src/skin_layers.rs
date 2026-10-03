//! Does a skin's clothes (overlay) layer have anything painted on it?
//!
//! Drives the Workshop's clothes-on/off default: a skin with a bare overlay
//! opens on the body (so there is no invisible shell to be confused by), and a
//! skin with so much as one painted overlay pixel — an imported Minecraft hoodie,
//! say — opens showing the clothes, ready to edit.

/// True if any pixel inside the 36 overlay face rects has alpha > 0.
///
/// Scans exactly the rects from `skin_uv::overlay_faces()`, never the whole
/// atlas — the unused regions of a 64x64 skin are not clothes. Returns false for
/// a buffer that is not 64x64 RGBA.
///
/// Deliberately scans the CLASSIC rects regardless of the skin's arm model.
/// Every slim arm rect lies inside the union of that arm's classic rects
/// (`skin_uv::slim_arm_rects_stay_inside_the_classic_arm_island` pins that), so
/// the classic scan is a strict superset: a painted slim sleeve is always
/// found. The reverse — a stray pixel in the 1-px column a slim arm gives up —
/// would also count as clothes, which is the right way round to be wrong: it
/// opens the painter showing the clothes, never hides painted ones. Takes no
/// `ArmModel` for that reason, so no caller has to thread one just to ask
/// "does this skin have a second layer?".
pub fn clothes_layer_has_content(buffer: &[u8]) -> bool {
    const SIDE: usize = 64;
    if buffer.len() != SIDE * SIDE * 4 {
        return false;
    }
    for part in crate::skin_uv::overlay_faces(crate::skin_uv::ArmModel::Classic) {
        for rect in part {
            // UvRects are 0..1 atlas space; back to pixels.
            let x0 = (rect[0] * SIDE as f32).round() as usize;
            let y0 = (rect[1] * SIDE as f32).round() as usize;
            let x1 = (rect[2] * SIDE as f32).round() as usize;
            let y1 = (rect[3] * SIDE as f32).round() as usize;
            for y in y0..y1.min(SIDE) {
                for x in x0..x1.min(SIDE) {
                    if buffer[(y * SIDE + x) * 4 + 3] > 0 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Vec<u8> {
        vec![0u8; 64 * 64 * 4]
    }

    /// Set one pixel's alpha in a 64x64 RGBA buffer.
    fn set_alpha(buf: &mut [u8], x: u32, y: u32, a: u8) {
        let i = ((y * 64 + x) * 4) as usize;
        buf[i + 3] = a;
    }

    #[test]
    fn empty_overlay_has_no_content() {
        assert!(!clothes_layer_has_content(&blank()));
    }

    #[test]
    fn one_painted_overlay_pixel_counts() {
        // (32, 8) is inside the hat rect px(32,8,8,8).
        let mut b = blank();
        set_alpha(&mut b, 32, 8, 255);
        assert!(clothes_layer_has_content(&b), "a single painted pixel turns clothes on");
    }

    #[test]
    fn base_layer_pixels_do_not_count() {
        // (8, 8) is the head FRONT on the BASE layer — px(8,8,8,8) in base_faces.
        // A fully-opaque body must not be mistaken for clothes.
        let mut b = blank();
        set_alpha(&mut b, 8, 8, 255);
        assert!(!clothes_layer_has_content(&b), "base pixels are not clothes");
    }

    #[test]
    fn a_fully_opaque_default_skin_has_no_clothes() {
        // The stock skin paints the body solid and leaves the overlay clear.
        let def = crate::texture_gen::default_skin_rgba();
        assert_eq!(def.len(), 64 * 64 * 4);
        assert!(
            !clothes_layer_has_content(&def),
            "the default look must open on the body, not on an invisible shell"
        );
    }

    #[test]
    fn a_sleeve_pixel_counts() {
        // (48, 52) is inside the left-arm sleeve rect px(48,52,4,12).
        let mut b = blank();
        set_alpha(&mut b, 48, 52, 200);
        assert!(clothes_layer_has_content(&b));
    }

    #[test]
    fn a_slim_sleeve_pixel_still_counts() {
        // The slim left sleeve's front tile starts at (52,52) — inside the
        // classic rect the scan uses, which is exactly why the classic-superset
        // shortcut is safe. Painted slim clothes must never read as "bare".
        let (x0, y0, _, _) = crate::skin_uv::face_rect_px(
            2,
            5,
            crate::skin_uv::SkinLayer::Overlay,
            crate::skin_uv::ArmModel::Slim,
        )
        .unwrap();
        let mut b = blank();
        set_alpha(&mut b, x0, y0, 200);
        assert!(clothes_layer_has_content(&b), "a slim sleeve pixel is clothes too");
    }

    #[test]
    fn wrong_sized_buffer_is_treated_as_empty() {
        assert!(!clothes_layer_has_content(&[0u8; 16]));
    }
}

//! Exhibit data model — a placed, sized 2D art surface: either flush on a wall
//! or a standing Y-axis billboard on a pedestal. Pure data + helpers only;
//! rendering lives in the painting pipeline, persistence in `save::WorldSave`.
//!
//! Spec: docs/superpowers/specs/2026-06-19-creator-gallery-showcase-design.md (§5).
//!
//! Forward-compat (spec §13 #5): `image_ref` + `presentation` is the *image-only
//! special case* of a content-generic exhibit. Audio / text-panel / voxel-build-
//! adopt presentations slot onto the same `Vec<Exhibit>` + render dispatch later
//! without a rebuild; v1 implements image-only as locked by the Phase 1 plan.

use serde::{Deserialize, Serialize};
use std::f32::consts::{FRAC_PI_2, PI};

/// How an exhibit mounts in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presentation {
    /// Flat quad flush on a wall; fixed facing; single-sided (back = wall).
    Wall,
    /// Y-axis billboard standing on/above a pedestal; always faces the viewer;
    /// rendered from a transparent (cut-out) PNG.
    Standing,
}

/// A single placed art piece.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exhibit {
    /// World-space anchor: the mount point (Wall) or pedestal-top (Standing).
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub presentation: Presentation,
    /// Filename of the image within the world's exhibit image set (no path).
    pub image_ref: String,
    /// Display size in blocks (width, height). Must be > 0 to render.
    pub width: f32,
    pub height: f32,
    /// Facing, radians. Wall: the wall's outward normal. Standing: a starting
    /// yaw, overridden each frame by billboarding at render time (1b).
    pub yaw: f32,
    /// Plaque text (title / artist). May be empty.
    pub label: String,
    // --- Reserved payload, consumed by later phases (collect/basket, commerce).
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub sku: Option<String>,
    #[serde(default)]
    pub price: Option<u64>,
}

impl Exhibit {
    /// True when renderable: a non-empty image ref and a positive size.
    pub fn is_valid(&self) -> bool {
        !self.image_ref.trim().is_empty() && self.width > 0.0 && self.height > 0.0
    }

    /// Aspect ratio (width / height); 1.0 when height is non-positive (guard).
    pub fn aspect(&self) -> f32 {
        if self.height > 0.0 {
            self.width / self.height
        } else {
            1.0
        }
    }
}

/// Fit an image of aspect `image_aspect` (= pixel width / height) inside the
/// artist's `(box_w, box_h)` size box **without distortion**, returning the
/// rendered `(width, height)` in blocks. The image is scaled to touch the box on
/// its limiting axis and centred on the other — the artist picks the *size*
/// (the box), the image keeps its true *shape*. This is the fix for art being
/// stretched into a hand-typed square: `width/height` are no longer the literal
/// quad size but its bounding frame. Non-positive inputs collapse to `(0, 0)` so
/// a half-edited exhibit never panics or renders inverted.
pub fn fit_within(box_w: f32, box_h: f32, image_aspect: f32) -> (f32, f32) {
    if box_w <= 0.0 || box_h <= 0.0 || image_aspect <= 0.0 {
        return (0.0, 0.0);
    }
    let box_aspect = box_w / box_h;
    if image_aspect >= box_aspect {
        // Wider (per unit height) than the box → width-limited.
        (box_w, box_w / image_aspect)
    } else {
        // Taller than the box → height-limited.
        (box_h * image_aspect, box_h)
    }
}

/// The horizontal outward normal for a facing `yaw` (radians). Yaw 0 faces +Z;
/// increasing yaw rotates toward +X (right-handed about +Y). Y is always 0 — a
/// quad built from this normal stays vertical (never tilts).
pub fn yaw_to_normal(yaw: f32) -> [f32; 3] {
    [yaw.sin(), 0.0, yaw.cos()]
}

/// Y-axis-only billboard facing: the yaw whose outward normal points horizontally
/// from `anchor` toward `eye`. The vertical component is dropped, so a viewer
/// looking down on the piece still sees it upright (Spec §5: "never tilts"). When
/// the eye is directly above/below the anchor (degenerate horizontal delta) the
/// facing is held at yaw 0 so the quad never collapses or flips.
pub fn billboard_yaw(anchor: [f32; 3], eye: [f32; 3]) -> f32 {
    let dx = eye[0] - anchor[0];
    let dz = eye[2] - anchor[2];
    if dx * dx + dz * dz < 1e-8 {
        return 0.0;
    }
    dx.atan2(dz)
}

/// How far a Wall exhibit's quad sits off the wall face, in blocks — just enough
/// to beat z-fighting with the wall block behind it.
pub const WALL_INSET: f32 = 0.03;

/// World-space centre of a **Wall** exhibit's quad. The anchor `(x, y, z)` is the
/// air cell in front of the wall (placement = hit block + face), and `yaw`'s
/// outward normal points from the wall into that cell; so the quad is pushed
/// from the cell centre back toward the wall until it sits [`WALL_INSET`] off
/// the face — flush, not floating half a block out. `disp_h` is the rendered
/// height; the quad's bottom edge sits on the anchor's floor (`y`).
pub fn wall_quad_center(x: i32, y: i32, z: i32, yaw: f32, disp_h: f32) -> [f32; 3] {
    let n = yaw_to_normal(yaw);
    let back = 0.5 - WALL_INSET;
    [
        x as f32 + 0.5 - n[0] * back,
        y as f32 + disp_h * 0.5,
        z as f32 + 0.5 - n[2] * back,
    ]
}

// ---------------------------------------------------------------------------
// Curation logic (Phase 1c) — pure free functions over the exhibit list. The
// `/exhibit` command (commands/builtins/exhibit.rs) is a thin parser over these.
// ---------------------------------------------------------------------------

/// Where a freshly-placed exhibit anchors, and the yaw it should face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedAnchor {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub yaw: f32,
}

/// What can go wrong editing the exhibit list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The artist isn't looking at a block (no raycast target).
    NoTarget,
    /// The given list index doesn't exist.
    BadIndex,
    /// An empty / whitespace-only image reference was supplied.
    EmptyImageRef,
    /// A width or height that is not strictly positive.
    NonPositiveSize,
    /// A Wall exhibit was placed against a floor/ceiling face (needs a vertical wall).
    WallNeedsWallFace,
}

/// Convert a hit-face unit normal into the yaw a Wall exhibit should face — the
/// yaw whose `yaw_to_normal` equals that face normal, so the piece faces the room
/// (toward the player who struck the face). Returns `None` for the floor/ceiling
/// normals (a wall piece can't mount flat). The mapping is the inverse of
/// `yaw_to_normal` (yaw 0 → +Z), NOT the camera-forward convention — the renderer
/// (`build_exhibit_quads` → `yaw_to_normal`) is the source of truth for what a
/// yaw means.
pub fn yaw_from_face(face: [i32; 3]) -> Option<f32> {
    match face {
        [0, 0, 1] => Some(0.0),         // +Z face → outward normal +Z (yaw 0)
        [0, 0, -1] => Some(PI),         // -Z face → outward normal -Z (yaw π)
        [1, 0, 0] => Some(FRAC_PI_2),   // +X face → outward normal +X (yaw +π/2)
        [-1, 0, 0] => Some(-FRAC_PI_2), // -X face → outward normal -X (yaw -π/2)
        _ => None,                      // [0,±1,0] = floor/ceiling: no wall mount
    }
}

/// Resolve the anchor + yaw for a new exhibit from the player's raycast and
/// facing. Wall: anchors in the cell adjacent to the hit face (target + face),
/// yaw derived from which wall face was struck — rejecting floor/ceiling.
/// Standing: anchors on top of the hit block (target + Y), yaw = the artist's
/// current camera yaw rounded to the nearest 45° so billboards start sensibly
/// aligned (the 1b billboard pass overrides this each frame at render).
pub fn resolve_placement(
    target_block: Option<[i32; 3]>,
    target_face: [i32; 3],
    camera_yaw: f32,
    presentation: Presentation,
) -> Result<PlacedAnchor, EditError> {
    let t = target_block.ok_or(EditError::NoTarget)?;
    match presentation {
        Presentation::Wall => {
            let yaw = yaw_from_face(target_face).ok_or(EditError::WallNeedsWallFace)?;
            Ok(PlacedAnchor {
                x: t[0] + target_face[0],
                y: t[1] + target_face[1],
                z: t[2] + target_face[2],
                yaw,
            })
        }
        Presentation::Standing => Ok(PlacedAnchor {
            x: t[0],
            y: t[1] + 1, // stands on top of the looked-at block (the pedestal)
            z: t[2],
            yaw: snap_to_45(camera_yaw),
        }),
    }
}

/// Round a yaw (radians) to the nearest 45° (π/4) step.
fn snap_to_45(yaw: f32) -> f32 {
    let step = PI / 4.0;
    (yaw / step).round() * step
}

fn at_mut(list: &mut [Exhibit], index: usize) -> Result<&mut Exhibit, EditError> {
    list.get_mut(index).ok_or(EditError::BadIndex)
}

pub fn apply_resize(
    list: &mut [Exhibit],
    index: usize,
    width: f32,
    height: f32,
) -> Result<(), EditError> {
    if width <= 0.0 || height <= 0.0 {
        return Err(EditError::NonPositiveSize);
    }
    let e = at_mut(list, index)?;
    e.width = width;
    e.height = height;
    Ok(())
}

pub fn apply_yaw(list: &mut [Exhibit], index: usize, yaw: f32) -> Result<(), EditError> {
    at_mut(list, index)?.yaw = yaw;
    Ok(())
}

pub fn apply_move(list: &mut [Exhibit], index: usize, x: i32, y: i32, z: i32) -> Result<(), EditError> {
    let e = at_mut(list, index)?;
    e.x = x;
    e.y = y;
    e.z = z;
    Ok(())
}

pub fn apply_set_image(
    list: &mut [Exhibit],
    index: usize,
    image_ref: String,
) -> Result<(), EditError> {
    if image_ref.trim().is_empty() {
        return Err(EditError::EmptyImageRef);
    }
    at_mut(list, index)?.image_ref = image_ref;
    Ok(())
}

pub fn apply_set_label(list: &mut [Exhibit], index: usize, label: String) -> Result<(), EditError> {
    at_mut(list, index)?.label = label;
    Ok(())
}

pub fn apply_delete(list: &mut Vec<Exhibit>, index: usize) -> Result<Exhibit, EditError> {
    if index >= list.len() {
        return Err(EditError::BadIndex);
    }
    Ok(list.remove(index))
}

#[cfg(test)]
mod tests {

    #[test]
    fn wall_quad_sits_flush_on_the_wall_behind_its_anchor() {
        // Anchor cell (4, 81, 7) in front of a wall on its -X side: the face
        // normal is +X (yaw +pi/2), the wall face is the plane x = 4.
        let c = wall_quad_center(4, 81, 7, FRAC_PI_2, 1.7);
        assert!((c[0] - (4.0 + WALL_INSET)).abs() < 1e-5, "not flush: {c:?}");
        assert!((c[2] - 7.5).abs() < 1e-5, "must stay centred along the wall");
        assert!((c[1] - (81.0 + 0.85)).abs() < 1e-5, "bottom edge on the anchor floor");
        // Wall on the +Z side of the anchor: normal -Z (yaw pi), face plane z = 8.
        let c = wall_quad_center(0, 80, 7, PI, 1.0);
        assert!((c[2] - (8.0 - WALL_INSET)).abs() < 1e-5, "not flush: {c:?}");
        assert!((c[0] - 0.5).abs() < 1e-5);
    }
    use super::*;

    fn sample(presentation: Presentation) -> Exhibit {
        Exhibit {
            x: 1,
            y: 64,
            z: -3,
            presentation,
            image_ref: "piece.png".to_string(),
            width: 2.0,
            height: 1.0,
            yaw: 0.0,
            label: "Untitled".to_string(),
            link: None,
            sku: None,
            price: None,
        }
    }

    #[test]
    fn valid_exhibit_passes() {
        assert!(sample(Presentation::Wall).is_valid());
        assert!(sample(Presentation::Standing).is_valid());
    }

    #[test]
    fn empty_image_ref_is_invalid() {
        let mut e = sample(Presentation::Wall);
        e.image_ref = "   ".to_string();
        assert!(!e.is_valid());
    }

    #[test]
    fn zero_or_negative_size_is_invalid() {
        let mut e = sample(Presentation::Standing);
        e.width = 0.0;
        assert!(!e.is_valid());
        e.width = 2.0;
        e.height = -1.0;
        assert!(!e.is_valid());
    }

    #[test]
    fn aspect_is_width_over_height_with_guard() {
        let e = sample(Presentation::Wall); // 2.0 / 1.0
        assert!((e.aspect() - 2.0).abs() < 1e-6);
        let mut z = sample(Presentation::Wall);
        z.height = 0.0;
        assert!((z.aspect() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn yaw_to_normal_is_unit_and_horizontal() {
        for yaw in [0.0_f32, 0.5, 1.5707963, 3.14159, -2.0, 5.0] {
            let n = yaw_to_normal(yaw);
            assert!((n[1]).abs() < 1e-6, "billboard normal must be horizontal");
            let len = (n[0] * n[0] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "normal must be unit length");
        }
        // Yaw 0 faces +Z by convention.
        let n0 = yaw_to_normal(0.0);
        assert!((n0[0]).abs() < 1e-6 && (n0[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn billboard_faces_the_eye_ignoring_height() {
        let anchor = [10.0, 64.0, 10.0];
        // Eye due +Z of the anchor, far above it. Normal must point toward +Z
        // (yaw 0) regardless of the height difference.
        let yaw = billboard_yaw(anchor, [10.0, 200.0, 25.0]);
        let n = yaw_to_normal(yaw);
        assert!((n[2] - 1.0).abs() < 1e-4, "should face +Z toward the eye");
        assert!((n[1]).abs() < 1e-6, "facing must never tilt with height");

        // Eye due +X: normal points toward +X (yaw = +pi/2).
        let yaw_x = billboard_yaw(anchor, [99.0, 64.0, 10.0]);
        let nx = yaw_to_normal(yaw_x);
        assert!((nx[0] - 1.0).abs() < 1e-4, "should face +X toward the eye");
    }

    #[test]
    fn billboard_eye_directly_overhead_is_stable() {
        // Degenerate: eye straight above the anchor. Must NOT NaN/collapse.
        let yaw = billboard_yaw([5.0, 64.0, 5.0], [5.0, 120.0, 5.0]);
        assert_eq!(yaw, 0.0);
        let n = yaw_to_normal(yaw);
        assert!(n.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn yaw_from_wall_faces_only() {
        // Inverse of yaw_to_normal (yaw 0 → +Z): a struck face maps to the yaw
        // whose outward normal equals that face.
        assert_eq!(yaw_from_face([0, 0, 1]), Some(0.0));
        assert_eq!(yaw_from_face([0, 0, -1]), Some(PI));
        assert_eq!(yaw_from_face([1, 0, 0]), Some(FRAC_PI_2));
        assert_eq!(yaw_from_face([-1, 0, 0]), Some(-FRAC_PI_2));
        // Floor / ceiling have no wall to mount against:
        assert_eq!(yaw_from_face([0, 1, 0]), None);
        assert_eq!(yaw_from_face([0, -1, 0]), None);
    }

    #[test]
    fn wall_placement_anchors_in_front_of_wall_and_faces_the_room() {
        // The +Z face of the looked-at block is struck; the piece sits one cell
        // in +Z and faces the room (yaw 0). Crucially, yaw_to_normal(yaw) equals
        // the struck face — the exhibit faces back toward the player (the 1b/1c
        // yaw-convention reconciliation).
        let a = resolve_placement(Some([5, 64, 10]), [0, 0, 1], 1.23, Presentation::Wall).unwrap();
        assert_eq!((a.x, a.y, a.z), (5, 64, 11));
        assert!((a.yaw - 0.0).abs() < 1e-6);
        let n = yaw_to_normal(a.yaw);
        assert!((n[2] - 1.0).abs() < 1e-6, "wall normal must match the struck face (+Z)");
    }

    #[test]
    fn wall_placement_rejects_floor_and_ceiling() {
        assert_eq!(
            resolve_placement(Some([0, 0, 0]), [0, 1, 0], 0.0, Presentation::Wall),
            Err(EditError::WallNeedsWallFace)
        );
    }

    #[test]
    fn standing_placement_sits_on_top_and_snaps_yaw() {
        // Camera yaw slightly off 90° snaps to FRAC_PI_2; anchor is one above.
        let a = resolve_placement(Some([2, 30, -4]), [0, 1, 0], FRAC_PI_2 + 0.1, Presentation::Standing)
            .unwrap();
        assert_eq!((a.x, a.y, a.z), (2, 31, -4));
        assert!((a.yaw - FRAC_PI_2).abs() < 1e-6, "yaw snapped to nearest 45°");
    }

    #[test]
    fn placement_without_target_errors() {
        assert_eq!(
            resolve_placement(None, [0, 0, -1], 0.0, Presentation::Wall),
            Err(EditError::NoTarget)
        );
    }

    #[test]
    fn resize_validates_positive_size_and_index() {
        let mut list = vec![sample(Presentation::Wall)];
        assert!(apply_resize(&mut list, 0, 3.0, 2.0).is_ok());
        assert!((list[0].width - 3.0).abs() < 1e-6 && (list[0].height - 2.0).abs() < 1e-6);
        assert_eq!(apply_resize(&mut list, 0, 0.0, 2.0), Err(EditError::NonPositiveSize));
        assert_eq!(apply_resize(&mut list, 9, 1.0, 1.0), Err(EditError::BadIndex));
    }

    #[test]
    fn set_image_rejects_blank_and_bad_index() {
        let mut list = vec![sample(Presentation::Standing)];
        assert!(apply_set_image(&mut list, 0, "new.png".to_string()).is_ok());
        assert_eq!(list[0].image_ref, "new.png");
        assert_eq!(
            apply_set_image(&mut list, 0, "   ".to_string()),
            Err(EditError::EmptyImageRef)
        );
        assert_eq!(
            apply_set_image(&mut list, 5, "x.png".to_string()),
            Err(EditError::BadIndex)
        );
    }

    #[test]
    fn move_yaw_label_mutate_in_place() {
        let mut list = vec![sample(Presentation::Wall)];
        assert!(apply_move(&mut list, 0, 7, 8, 9).is_ok());
        assert_eq!((list[0].x, list[0].y, list[0].z), (7, 8, 9));
        assert!(apply_yaw(&mut list, 0, 1.5).is_ok());
        assert!((list[0].yaw - 1.5).abs() < 1e-6);
        assert!(apply_set_label(&mut list, 0, "Renamed".to_string()).is_ok());
        assert_eq!(list[0].label, "Renamed");
    }

    #[test]
    fn delete_removes_and_returns_the_piece_and_guards_index() {
        let mut list = vec![sample(Presentation::Wall), sample(Presentation::Standing)];
        let removed = apply_delete(&mut list, 0).unwrap();
        assert_eq!(removed.presentation, Presentation::Wall);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].presentation, Presentation::Standing);
        assert_eq!(apply_delete(&mut list, 9), Err(EditError::BadIndex));
    }

    fn approx(a: (f32, f32), b: (f32, f32)) {
        assert!((a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5, "{a:?} != {b:?}");
    }

    #[test]
    fn fit_within_landscape_image_in_square_box_fills_width() {
        // A 2:1 landscape image in a 2×2 box → 2 wide × 1 tall (width-limited),
        // never the stretched 2×2. The artist picks the box; the shape is the image's.
        approx(fit_within(2.0, 2.0, 2.0), (2.0, 1.0));
    }

    #[test]
    fn fit_within_portrait_image_in_square_box_fills_height() {
        // A 1:2 portrait image in a 2×2 box → 1 wide × 2 tall (height-limited).
        approx(fit_within(2.0, 2.0, 0.5), (1.0, 2.0));
    }

    #[test]
    fn fit_within_square_image_unchanged_in_square_box() {
        approx(fit_within(2.0, 2.0, 1.0), (2.0, 2.0));
    }

    #[test]
    fn fit_within_image_aspect_equal_to_box_fills_exactly() {
        // 16:9 image in a 16:9-shaped box fills the whole box.
        approx(fit_within(4.0, 2.25, 16.0 / 9.0), (4.0, 2.25));
    }

    #[test]
    fn fit_within_square_image_in_wide_box_is_height_limited() {
        // A square image in a 4×2 (2:1) box → 2×2, not stretched to 4×2.
        approx(fit_within(4.0, 2.0, 1.0), (2.0, 2.0));
    }

    #[test]
    fn fit_within_guards_nonpositive_inputs() {
        // Degenerate inputs never panic / never produce negatives.
        approx(fit_within(0.0, 2.0, 1.5), (0.0, 0.0));
        approx(fit_within(2.0, 0.0, 1.5), (0.0, 0.0));
        approx(fit_within(2.0, 2.0, 0.0), (0.0, 0.0));
        approx(fit_within(2.0, 2.0, -1.0), (0.0, 0.0));
    }
}

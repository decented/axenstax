//! Pure ray → avatar-skin hit-test for the Skin Studio painter (Phase 1b).
//! Given a ray in model space, returns which body part + cube face it hits and
//! the fractional position on that face — fed straight into
//! `skin_uv::texel_for` to land on a 64×64 pixel. Geometry and per-face UV
//! winding are copied verbatim from `entity_model::player_model()` +
//! `push_skin_quad`, so a click lands exactly where it's drawn.

use crate::skin_uv::{ArmModel, SkinLayer};

/// A resolved paint target on the avatar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvatarHit {
    pub part: usize,
    pub face: usize,
    pub frac_u: f32,
    pub frac_v: f32,
}

/// Result of one ray-vs-box slab test.
struct BoxHit {
    t: f32,
    axis: usize, // 0=x, 1=y, 2=z — which slab the ray entered through
}

/// Slab ray-AABB. Returns the entry `t` (>= 0) and the entry axis, or `None`.
fn ray_box(origin: [f32; 3], dir: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Option<BoxHit> {
    let mut t_enter = f32::NEG_INFINITY;
    let mut t_exit = f32::INFINITY;
    let mut enter_axis = 0usize;
    for a in 0..3 {
        if dir[a].abs() < 1e-9 {
            // Ray parallel to this slab: miss if the origin is outside it.
            if origin[a] < min[a] || origin[a] > max[a] {
                return None;
            }
        } else {
            let inv = 1.0 / dir[a];
            let mut t1 = (min[a] - origin[a]) * inv;
            let mut t2 = (max[a] - origin[a]) * inv;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            if t1 > t_enter {
                t_enter = t1;
                enter_axis = a;
            }
            if t2 < t_exit {
                t_exit = t2;
            }
            if t_enter > t_exit {
                return None;
            }
        }
    }
    if t_exit < 0.0 {
        return None; // box entirely behind the ray
    }
    let t = if t_enter >= 0.0 { t_enter } else { return None }; // require entering from outside
    Some(BoxHit { t, axis: enter_axis })
}

/// Map an entry axis + ray direction to the face index in `[+x,-x,+y,-y,+z,-z]`.
fn face_of(axis: usize, dir: [f32; 3]) -> usize {
    match axis {
        0 => if dir[0] > 0.0 { 1 } else { 0 }, // entered x_min (going +x) → -X face
        1 => if dir[1] > 0.0 { 3 } else { 2 },
        _ => if dir[2] > 0.0 { 5 } else { 4 },
    }
}

/// Fractional `(u, v)` on a face — the exact inverse of what
/// `entity_model::push_skin_quad` drew, so the brush lands under the crosshair.
///
/// u follows the Minecraft box unwrap, in which the head band runs
/// `right | front | left | back` across atlas x=0..32 and neighbouring tiles
/// share real box edges. Walking forward around the model therefore walks
/// forward along the atlas: on the +X (right) face u runs back→front; on the
/// -Z (front) face it runs from the character's right to their left; and the
/// top/bottom tiles keep the front face's u direction because they are stacked
/// above it. Flip any of these and the renderer and the painter disagree.
fn frac_uv(face: usize, p: [f32; 3], min: [f32; 3], max: [f32; 3]) -> (f32, f32) {
    let (x, y, z) = (p[0], p[1], p[2]);
    let (xn, xx) = (min[0], max[0]);
    let (yn, yx) = (min[1], max[1]);
    let (zn, zx) = (min[2], max[2]);
    let dx = xx - xn;
    let dy = yx - yn;
    let dz = zx - zn;
    match face {
        0 => ((zx - z) / dz, (yx - y) / dy),       // +X: u runs back → front
        1 => ((z - zn) / dz, (yx - y) / dy),       // -X: u runs front → back
        2 => ((xx - x) / dx, (zx - z) / dz),       // +Y
        3 => ((xx - x) / dx, (z - zn) / dz),       // -Y
        4 => ((x - xn) / dx, (yx - y) / dy),       // +Z: u runs left → right
        _ => ((xx - x) / dx, (yx - y) / dy),       // -Z: u runs right → left
    }
}

/// Ray-cast against the avatar; nearest forward hit as `(part, face, frac_u, frac_v)`.
///
/// `t` is the eased limbs-apart factor and `edit` selects the exaggerated Workshop
/// overlay inflate — BOTH must match what the renderer drew, or the crosshair lands
/// somewhere other than the limb. See [`crate::skin_pose`]. `arm` likewise:
/// aiming at a slim avatar with the classic boxes would let the crosshair land
/// on a pixel-wide sliver of arm that isn't drawn.
pub fn ray_hit_avatar(
    origin: [f32; 3],
    dir: [f32; 3],
    layer: SkinLayer,
    t: f32,
    edit: bool,
    arm: ArmModel,
) -> Option<AvatarHit> {
    let mut best: Option<(f32, usize, BoxHit)> = None;
    for part in 0..crate::skin_pose::PART_COUNT {
        let (min, max) = crate::skin_pose::part_box(part, layer, t, edit, arm);
        if let Some(bh) = ray_box(origin, dir, min, max)
            && best.as_ref().is_none_or(|(bt, _, _)| bh.t < *bt) {
                best = Some((bh.t, part, bh));
            }
    }
    let (t_hit, part, bh) = best?;
    let (min, max) = crate::skin_pose::part_box(part, layer, t, edit, arm);
    let p = [
        origin[0] + dir[0] * t_hit,
        origin[1] + dir[1] * t_hit,
        origin[2] + dir[2] * t_hit,
    ];
    let face = face_of(bh.axis, dir);
    let (frac_u, frac_v) = frac_uv(face, p, min, max);
    Some(AvatarHit {
        part,
        face,
        frac_u: frac_u.clamp(0.0, 1.0),
        frac_v: frac_v.clamp(0.0, 1.0),
    })
}

/// Unproject a cursor in normalized device coordinates (`ndc_x`/`ndc_y` ∈ [-1,1],
/// y up — egui's y-down must be flipped by the caller) into a model-space ray
/// `(origin, dir)`. `view_proj` is the SAME matrix the avatar was rendered with
/// (`renderer::skin_preview_view_proj(..)`), so the ray matches what's on screen.
/// Uses wgpu depth (near z=0, far z=1) and `project_point3` for the w-divide.
///
/// Currently exercised only by this module's tests + the wardrobe-thumbnail
/// camera: the in-world Workshop painter transforms the world camera ray with
/// `workshop::world_ray_to_avatar_model` instead.
#[allow(dead_code)]
pub fn cursor_ray(view_proj: glam::Mat4, ndc_x: f32, ndc_y: f32) -> ([f32; 3], [f32; 3]) {
    let inv = view_proj.inverse();
    let near = inv.project_point3(glam::Vec3::new(ndc_x, ndc_y, 0.0));
    let far = inv.project_point3(glam::Vec3::new(ndc_x, ndc_y, 1.0));
    let dir = (far - near).normalize_or_zero();
    ([near.x, near.y, near.z], [dir.x, dir.y, dir.z])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin_uv::{ArmModel, SkinLayer};

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
    }

    #[test]
    fn ray_into_head_front_centre() {
        // From in front of the face (-Z side), looking +Z toward the head centre.
        let hit = ray_hit_avatar([0.0, 1.575, -5.0], [0.0, 0.0, 1.0], SkinLayer::Base, 0.0, false, ArmModel::Classic)
            .expect("ray should hit the head");
        assert_eq!(hit.part, 0, "head");
        assert_eq!(hit.face, 5, "front (-Z) face");
        approx(hit.frac_u, 0.5);
        approx(hit.frac_v, 0.5);
    }

    #[test]
    fn ray_onto_head_top_centre() {
        // From above, looking straight down onto the head's +Y top.
        let hit = ray_hit_avatar([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], SkinLayer::Base, 0.0, false, ArmModel::Classic)
            .expect("ray should hit the head top");
        assert_eq!(hit.part, 0);
        assert_eq!(hit.face, 2, "top (+Y) face");
        approx(hit.frac_u, 0.5);
        approx(hit.frac_v, 0.5);
    }

    #[test]
    fn ray_front_upper_left_of_face_maps_corner() {
        // Aim at the head FRONT, at the character's-right/top corner region.
        // Head front spans x in [-0.25, 0.25], y in [1.325, 1.825]. Pick x=+0.20,
        // y=1.80 (near top, +X side). On the front tile u0 sits at the character's
        // RIGHT (the edge shared with the right-side tile), so a +X hit is a LOW
        // frac_u: frac_u = (x_max - x)/0.5; frac_v = (y_max - y)/0.5.
        let hit = ray_hit_avatar([0.20, 1.80, -5.0], [0.0, 0.0, 1.0], SkinLayer::Base, 0.0, false, ArmModel::Classic)
            .expect("hit");
        assert_eq!((hit.part, hit.face), (0, 5));
        approx(hit.frac_u, (0.25 - 0.20) / 0.5); // 0.1
        approx(hit.frac_v, (1.825 - 1.80) / 0.5); // 0.05
    }

    #[test]
    fn right_side_of_head_maps_u_toward_the_front() {
        // The painter must agree with what the renderer drew. On the right-side
        // tile the u1 edge is the FRONT of the head (it joins the front tile in the
        // unrolled band), so clicking the front half of the right side has to land
        // at a HIGH frac_u. Get this backwards and the brush paints on the wrong
        // half of the skull.
        // Head spans z in [-0.25, 0.25] with -Z the front; y in [1.325, 1.825].
        let hit = ray_hit_avatar([5.0, 1.575, -0.20], [-1.0, 0.0, 0.0], SkinLayer::Base, 0.0, false, ArmModel::Classic)
            .expect("ray from the right should hit the head");
        assert_eq!((hit.part, hit.face), (0, 0), "head, +X (right side) face");
        // u runs back → front, so a hit 0.05 short of the front face is u = 0.9.
        approx(hit.frac_u, (0.25 - -0.20) / 0.5); // 0.9
    }

    #[test]
    fn ray_hits_right_arm_not_body() {
        // Right arm front spans x in [0.25, 0.5]. Aim at x=0.375 (arm centre), which
        // is outside the body's x-range [-0.25, 0.25].
        let hit = ray_hit_avatar([0.375, 1.05, -5.0], [0.0, 0.0, 1.0], SkinLayer::Base, 0.0, false, ArmModel::Classic)
            .expect("hit");
        assert_eq!(hit.part, 3, "right arm");
        assert_eq!(hit.face, 5);
    }

    #[test]
    fn ray_misses_returns_none() {
        // Well to the side of everything, looking +Z.
        assert!(ray_hit_avatar([5.0, 1.0, -5.0], [0.0, 0.0, 1.0], SkinLayer::Base, 0.0, false, ArmModel::Classic).is_none());
    }

    #[test]
    fn centre_cursor_hits_avatar_front() {
        // The exact view-proj the studio renders with: camera at yaw=0 sits at
        // world +Z (skin_preview_eye(0,0,2.6) → (0, 0.95, 2.6)). The ray goes
        // in the -Z direction, entering the body box through its +Z face (face 4).
        // skin_hit BOXES are in unrotated box-space; the visible face from a +Z
        // camera is always face 4 (+Z entry).
        let vp = crate::renderer::skin_preview_view_proj(
            0.0, 0.0, 2.6, 256.0 / 384.0,
        );
        // Centre of the image (ndc 0,0) → a ray that hits the avatar body.
        let (o, d) = cursor_ray(vp, 0.0, 0.0);
        let hit = ray_hit_avatar(o, d, SkinLayer::Base, 0.0, false, ArmModel::Classic)
            .expect("centre cursor should hit the avatar");
        assert_eq!(hit.face, 4, "+Z face: camera at yaw=0 is on the +Z side");
        // Camera targets the chest (y=0.95) → centre lands on the body.
        assert_eq!(hit.part, 1, "body");
        // Direction is unit length and generally points into the scene (-Z).
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-3, "dir normalized");
        assert!(d[2] < 0.0, "ray points toward the avatar (-Z), got {d:?}");
    }

    #[test]
    fn off_centre_cursor_moves_the_hit() {
        let vp = crate::renderer::skin_preview_view_proj(
            0.0, 0.0, 2.6, 256.0 / 384.0,
        );
        // A cursor left of centre and a cursor right of centre hit different
        // u-fractions on the same front face (the unprojection is not degenerate).
        let left = ray_hit_avatar(cursor_ray(vp, -0.3, 0.0).0, cursor_ray(vp, -0.3, 0.0).1, SkinLayer::Base, 0.0, false, ArmModel::Classic);
        let right = ray_hit_avatar(cursor_ray(vp, 0.3, 0.0).0, cursor_ray(vp, 0.3, 0.0).1, SkinLayer::Base, 0.0, false, ArmModel::Classic);
        let (l, r) = (left.expect("left hit"), right.expect("right hit"));
        assert!((l.frac_u - r.frac_u).abs() > 0.05, "horizontal cursor move changes frac_u");
    }

    #[test]
    fn overlay_shell_is_larger_than_base() {
        // A ray grazing 0.02 above the head top (1.825): misses the base box
        // (top at 1.825) but hits the overlay shell (top at 1.85625).
        let o = [0.0, 1.845, -5.0];
        let d = [0.0, 0.0, 1.0];
        assert!(ray_hit_avatar(o, d, SkinLayer::Base, 0.0, false, ArmModel::Classic).is_none(), "above base head");
        let hit = ray_hit_avatar(o, d, SkinLayer::Overlay, 0.0, false, ArmModel::Classic).expect("overlay shell is taller");
        assert_eq!(hit.part, 0);
    }

    #[test]
    fn clothes_off_cannot_hit_the_overlay_shell() {
        // The "cannot paint blind" invariant. A ray that grazes only the clothes
        // shell must MISS when the body layer is selected, so a click can never
        // land on something that is not being drawn.
        let o = [0.0, 1.84, -5.0]; // above the base head top (1.825)
        let d = [0.0, 0.0, 1.0];
        assert!(
            ray_hit_avatar(o, d, SkinLayer::Base, 0.0, true, ArmModel::Classic).is_none(),
            "body layer must not pick up the clothes shell"
        );
        assert!(
            ray_hit_avatar(o, d, SkinLayer::Overlay, 0.0, true, ArmModel::Classic).is_some(),
            "the clothes shell is hittable when clothes are selected"
        );
    }

    #[test]
    fn edit_inflate_widens_the_clothes_target() {
        // The exaggerated Workshop shell must be genuinely easier to hit than the
        // Minecraft-exact worn one — that is the whole point of it. Head base top
        // 1.825, worn overlay top 1.85625, edit overlay top 1.8875.
        let o = [0.0, 1.87, -5.0];
        let d = [0.0, 0.0, 1.0];
        assert!(ray_hit_avatar(o, d, SkinLayer::Overlay, 0.0, false, ArmModel::Classic).is_none());
        assert!(ray_hit_avatar(o, d, SkinLayer::Overlay, 0.0, true, ArmModel::Classic).is_some());
    }

    /// Cast a dense sphere of rays at the avatar and collect every `(part, face)`
    /// the crosshair can actually land on from outside the model.
    fn reachable_faces(t: f32, arm: ArmModel) -> std::collections::HashSet<(usize, usize)> {
        let mut found = std::collections::HashSet::new();
        let r = 3.0f32;
        let n = 24;
        for i in 0..n {
            for j in 0..n {
                let yaw = std::f32::consts::TAU * i as f32 / n as f32;
                let pitch = -std::f32::consts::FRAC_PI_2
                    + std::f32::consts::PI * (j as f32 + 0.5) / n as f32;
                let eye = [
                    r * pitch.cos() * yaw.cos(),
                    0.9 + r * pitch.sin(),
                    r * pitch.cos() * yaw.sin(),
                ];
                // Aim at a grid of points spanning the whole model.
                for tx in 0..17 {
                    for ty in 0..17 {
                        for tz in 0..9 {
                            let target = [
                                -0.8 + 1.6 * tx as f32 / 16.0,
                                2.1 * ty as f32 / 16.0,
                                -0.5 + 1.0 * tz as f32 / 8.0,
                            ];
                            let d = [
                                target[0] - eye[0],
                                target[1] - eye[1],
                                target[2] - eye[2],
                            ];
                            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                            if len < 1e-6 {
                                continue;
                            }
                            let dir = [d[0] / len, d[1] / len, d[2] / len];
                            if let Some(h) = ray_hit_avatar(eye, dir, SkinLayer::Base, t, false, arm) {
                                found.insert((h.part, h.face));
                            }
                        }
                    }
                }
            }
        }
        found
    }

    #[test]
    fn apart_pose_reaches_every_face() {
        // THE guard. With the limbs separated, all 6 parts × 6 faces must be
        // paintable. Axolittle hit a wall here: in the resting pose 8 of these 36
        // are occluded by a neighbouring part and can never be painted at all.
        for arm in [ArmModel::Classic, ArmModel::Slim] {
            let found = reachable_faces(1.0, arm);
            let mut missing: Vec<(usize, usize)> = Vec::new();
            for part in 0..crate::skin_pose::PART_COUNT {
                for face in 0..6 {
                    if !found.contains(&(part, face)) {
                        missing.push((part, face));
                    }
                }
            }
            assert!(
                missing.is_empty(),
                "{arm:?}: these (part, face) pairs are unpaintable even with limbs apart: {missing:?}"
            );
        }
    }

    #[test]
    fn together_pose_occludes_the_faces_apart_exists_to_expose() {
        // Documents WHY the apart pose exists, and fails if the resting geometry
        // silently changes. Inner arms, inner legs, body top/bottom and leg tops
        // are buried when the parts rest flush against each other.
        let found = reachable_faces(0.0, ArmModel::Classic);
        for (part, face, what) in [
            (1usize, 2usize, "body top (head sits on it)"),
            (1, 3, "body bottom (legs sit under it)"),
            (2, 0, "left arm inner face"),
            (3, 1, "right arm inner face"),
            (4, 2, "left leg top"),
            (5, 2, "right leg top"),
        ] {
            assert!(
                !found.contains(&(part, face)),
                "{what} is reachable in the resting pose — the geometry changed, \
                 re-check whether the apart pose is still needed"
            );
        }
    }

    // ── Slim ("Alex") arms ───────────────────────────────────────────────────

    #[test]
    fn ray_hits_the_slim_right_arm_front_at_its_centre() {
        // The slim right arm spans x 0.25..0.4375 — centre 0.34375. A ray down
        // that line must strike the arm's FRONT face dead centre; a stale
        // classic box would put the centre at 0.375 and report frac_u ≈ 0.33.
        let hit = ray_hit_avatar(
            [0.34375, 1.0, -5.0],
            [0.0, 0.0, 1.0],
            SkinLayer::Base,
            0.0,
            false,
            ArmModel::Slim,
        )
        .expect("ray should hit the slim right arm");
        assert_eq!((hit.part, hit.face), (3, 5), "right arm, front (-Z) face");
        approx(hit.frac_u, 0.5);
    }

    #[test]
    fn a_ray_down_the_pixel_slim_gave_up_misses_the_arm() {
        // x = 0.47 sits inside the CLASSIC right arm (0.25..0.5) but outside the
        // slim one (0.25..0.4375). On slim nothing is drawn there, so nothing
        // may be hit there — otherwise the crosshair paints thin air.
        let o = [0.47, 1.0, -5.0];
        let d = [0.0, 0.0, 1.0];
        assert_eq!(
            ray_hit_avatar(o, d, SkinLayer::Base, 0.0, false, ArmModel::Classic)
                .map(|h| h.part),
            Some(3),
            "classic: that column IS arm"
        );
        assert!(
            ray_hit_avatar(o, d, SkinLayer::Base, 0.0, false, ArmModel::Slim).is_none(),
            "slim: that column is empty air"
        );
    }

    #[test]
    fn the_slim_arms_inner_edge_still_meets_the_torso() {
        // Grazing just inside the shared plane (x = 0.25) must still land on the
        // arm, not slip between arm and body — the inner edge did not move.
        let hit = ray_hit_avatar(
            [0.26, 1.0, -5.0],
            [0.0, 0.0, 1.0],
            SkinLayer::Base,
            0.0,
            false,
            ArmModel::Slim,
        )
        .expect("hit");
        assert_eq!(hit.part, 3, "still the right arm, hard against the torso");
    }

    #[test]
    fn slim_head_and_body_hits_are_identical_to_classic() {
        // Only the arms move; a head or chest click must resolve the same way on
        // both models (same texel, same face) or every non-arm click would drift.
        for (o, d) in [
            ([0.10f32, 1.70, -5.0], [0.0f32, 0.0, 1.0]),
            ([0.0, 1.0, -5.0], [0.0, 0.0, 1.0]),
            ([0.0, 5.0, 0.0], [0.0, -1.0, 0.0]),
        ] {
            let c = ray_hit_avatar(o, d, SkinLayer::Base, 0.0, false, ArmModel::Classic);
            let s = ray_hit_avatar(o, d, SkinLayer::Base, 0.0, false, ArmModel::Slim);
            assert_eq!(c, s, "non-arm ray {o:?} must resolve identically on both models");
        }
    }
}

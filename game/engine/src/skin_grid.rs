//! Precision aids for the in-world avatar skin painter: the **texel grid**
//! drawn over the blown-up Workshop mannequin, and the **hover footprint** that
//! outlines exactly which texel(s) the next click will paint.
//!
//! Everything here is pure line geometry — world-space endpoint pairs the
//! renderer thickens into wire quads (`renderer::set_paint_aid_lines`). No GPU,
//! no egui, so the maths that decides "the box you see is the pixel you get" is
//! unit-tested without a display.
//!
//! Three sources of truth are reused verbatim rather than re-derived, because a
//! grid that disagrees with any of them is worse than no grid at all:
//!
//! - [`crate::skin_pose::part_box`] — the shell the lines are laid on (base body
//!   or the exaggerated Workshop clothes shell), at the current limbs-apart
//!   separation. Same call the renderer and [`crate::skin_hit`] make.
//! - [`crate::skin_uv::face_rect_px`] — how many texels ride each face, so the
//!   grid pitch is the real atlas pitch, not a guess.
//! - [`crate::skin_paint::pencil`] — the brush footprint rule (`brush/2` top-left
//!   bias, clamped to the face rect), so the yellow box is exactly the pixel set
//!   the click writes.
//!
//! [`avatar_model_to_world`] is the forward half of the mannequin transform; its
//! inverse is [`crate::workshop::world_ray_to_avatar_model`] (the paint ray).
//! Both the vertex builder ([`crate::entity_model::build_workshop_avatar_vertices`])
//! and these aids go through the forward function, so there is one transform, not
//! three copies of one.

use crate::skin_pose::PART_COUNT;
use crate::skin_uv::{ArmModel, SkinLayer};

/// A world-space line: `(start, end)`.
pub type Segment = ([f32; 3], [f32; 3]);

/// How far the grid lines float off the shell, in WORLD units, so they never
/// z-fight the painted face. Small enough that they still read as lying on the
/// skin at the ×4 blow-up.
pub const GRID_EPS_WORLD: f32 = 0.004;

/// The footprint outline floats a little further out than the grid, so where the
/// two coincide (they share texel edges) the yellow box wins.
pub const FOOTPRINT_EPS_WORLD: f32 = 0.012;

/// Model → world for the blown-up Workshop mannequin: rotate about Y by
/// `yaw + π/2`, scale uniformly about the feet anchor, translate to `anchor`.
///
/// `yaw` is the avatar's model yaw (e.g. [`crate::workshop::WORKSHOP_MANNEQUIN_YAW`]),
/// NOT the pre-rotated `yaw + π/2` the vertex builder works in — the same
/// convention [`crate::workshop::world_ray_to_avatar_model`] takes, and this is
/// its exact inverse. Model space is the one
/// [`crate::skin_pose::part_box`] / [`crate::skin_hit::ray_hit_avatar`] use:
/// feet at the origin, front on −Z, limbs-apart offsets already applied.
pub fn avatar_model_to_world(p: [f32; 3], anchor: [f32; 3], yaw: f32, scale: f32) -> [f32; 3] {
    let theta = yaw + std::f32::consts::FRAC_PI_2;
    let (c, s) = (theta.cos(), theta.sin());
    [
        anchor[0] + (p[0] * c - p[2] * s) * scale,
        anchor[1] + p[1] * scale,
        anchor[2] + (p[0] * s + p[2] * c) * scale,
    ]
}

/// Which skin layer the mannequin is currently SHOWING — and therefore the only
/// one a click can land on. Clothes visible IS clothes selected (the painter's
/// "you cannot paint blind" rule); mirrors `SkinPaintSession::layer`.
pub fn visible_layer(clothes_on: bool) -> SkinLayer {
    if clothes_on {
        SkinLayer::Overlay
    } else {
        SkinLayer::Base
    }
}

/// The box currently drawn for `part` on `layer` at separation `t`. Always the
/// Workshop `edit` inflate — that flag only widens the OVERLAY shell, so passing
/// it unconditionally is correct for the base layer too (see `skin_pose`).
fn shell_box(part: usize, layer: SkinLayer, t: f32, arm: ArmModel) -> ([f32; 3], [f32; 3]) {
    crate::skin_pose::part_box(part, layer, t, true, arm)
}

/// Outward unit normal of a cube face, in the `[+x,-x,+y,-y,+z,-z]` order the
/// whole skin stack uses. Out-of-range faces get `+Y` (never reached: every
/// caller iterates `0..6`).
fn face_normal(face: usize) -> [f32; 3] {
    match face {
        0 => [1.0, 0.0, 0.0],
        1 => [-1.0, 0.0, 0.0],
        2 => [0.0, 1.0, 0.0],
        3 => [0.0, -1.0, 0.0],
        4 => [0.0, 0.0, 1.0],
        5 => [0.0, 0.0, -1.0],
        _ => [0.0, 1.0, 0.0],
    }
}

/// The model-space point at fractional `(fu, fv)` on `face` of the box
/// `(min, max)` — the exact INVERSE of `skin_hit::frac_uv`, which is itself the
/// inverse of what `entity_model::push_skin_quad` drew. Flip any row here and
/// the grid stops lining up with the texels the brush actually writes.
pub fn face_point(face: usize, min: [f32; 3], max: [f32; 3], fu: f32, fv: f32) -> [f32; 3] {
    let (xn, yn, zn) = (min[0], min[1], min[2]);
    let (xx, yx, zx) = (max[0], max[1], max[2]);
    let (dx, dy, dz) = (xx - xn, yx - yn, zx - zn);
    match face {
        // +X: u runs back → front (z high → low), v runs top → bottom.
        0 => [xx, yx - fv * dy, zx - fu * dz],
        // −X: u runs front → back.
        1 => [xn, yx - fv * dy, zn + fu * dz],
        // +Y (top): u runs +x → −x, v runs back → front.
        2 => [xx - fu * dx, yx, zx - fv * dz],
        // −Y (bottom): u runs +x → −x, v runs front → back.
        3 => [xx - fu * dx, yn, zn + fv * dz],
        // +Z (back): u runs −x → +x.
        4 => [xn + fu * dx, yx - fv * dy, zx],
        // −Z (front): u runs +x → −x.
        _ => [xx - fu * dx, yx - fv * dy, zn],
    }
}

/// One model-space face point, pushed `eps` along the face normal and taken to
/// world space.
fn lifted(
    face: usize,
    min: [f32; 3],
    max: [f32; 3],
    fu: f32,
    fv: f32,
    eps: f32,
    anchor: [f32; 3],
    yaw: f32,
    scale: f32,
) -> [f32; 3] {
    let p = face_point(face, min, max, fu, fv);
    let n = face_normal(face);
    avatar_model_to_world(
        [p[0] + n[0] * eps, p[1] + n[1] * eps, p[2] + n[2] * eps],
        anchor,
        yaw,
        scale,
    )
}

/// Every texel gridline on the CURRENTLY VISIBLE shell of the mannequin, in
/// world space.
///
/// For each of the 6 parts × 6 faces: `w + 1` lines across and `h + 1` down,
/// where `w × h` is the face's atlas rect (so a head face is 8×8 → 18 lines).
/// `clothes_on` picks the shell — the clothes shell when they're on (and
/// therefore what clicks land on), the bare body when they're off.
///
/// ~584 segments for the standard model. Rebuilding is cheap but not free, so
/// the caller memoises with [`GridAid`]; the pose only changes while the R
/// limbs-apart ease is running.
pub fn grid_segments(
    limb_frac: f32,
    clothes_on: bool,
    arm: ArmModel,
    anchor: [f32; 3],
    yaw: f32,
    scale: f32,
) -> Vec<Segment> {
    let layer = visible_layer(clothes_on);
    // The epsilon is specified in WORLD units, so undo the blow-up to get the
    // model-space push that lands there.
    let eps = GRID_EPS_WORLD / scale.max(1e-6);
    let mut out = Vec::with_capacity(600);
    for part in 0..PART_COUNT {
        let (min, max) = shell_box(part, layer, limb_frac, arm);
        for face in 0..6 {
            let Some((x0, y0, x1, y1)) = crate::skin_uv::face_rect_px(part, face, layer, arm) else {
                continue;
            };
            let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
            if w == 0 || h == 0 {
                continue;
            }
            for i in 0..=w {
                let u = i as f32 / w as f32;
                out.push((
                    lifted(face, min, max, u, 0.0, eps, anchor, yaw, scale),
                    lifted(face, min, max, u, 1.0, eps, anchor, yaw, scale),
                ));
            }
            for j in 0..=h {
                let v = j as f32 / h as f32;
                out.push((
                    lifted(face, min, max, 0.0, v, eps, anchor, yaw, scale),
                    lifted(face, min, max, 1.0, v, eps, anchor, yaw, scale),
                ));
            }
        }
    }
    out
}

/// The half-open atlas-pixel rect a `brush`-wide click centred on
/// `(texel_x, texel_y)` would actually write on `(part, face, layer)` — the
/// footprint of [`crate::skin_paint::pencil`], clamped to the face's UV island
/// exactly as `pencil` clamps it. `None` when the brush lands entirely off the
/// face (or the part/face is out of range).
///
/// Kept as its own function so the test suite can compare it against the pixels
/// `pencil` really touches, rather than trusting two copies of the same rule.
#[allow(clippy::too_many_arguments)]
pub fn footprint_rect(
    part: usize,
    face: usize,
    layer: SkinLayer,
    arm: ArmModel,
    texel_x: u32,
    texel_y: u32,
    brush: u32,
) -> Option<(u32, u32, u32, u32)> {
    let (x0, y0, x1, y1) = crate::skin_uv::face_rect_px(part, face, layer, arm)?;
    // `pencil`: side = brush.max(1), top-left corner at centre − side/2.
    let side = brush.max(1) as i64;
    let half = side / 2;
    let lo_x = (texel_x as i64 - half).max(x0 as i64);
    let hi_x = (texel_x as i64 - half + side).min(x1 as i64);
    let lo_y = (texel_y as i64 - half).max(y0 as i64);
    let hi_y = (texel_y as i64 - half + side).min(y1 as i64);
    if hi_x <= lo_x || hi_y <= lo_y {
        return None;
    }
    Some((lo_x as u32, lo_y as u32, hi_x as u32, hi_y as u32))
}

/// The 4-segment outline of the brush footprint, on the same visible shell as
/// the grid and floated a little further out so it sits on top of it.
///
/// `layer` must be the layer the mannequin is SHOWING (see [`visible_layer`]) —
/// the painter never lets a click land on a hidden layer, so drawing the hover
/// box on one would be a lie. Empty when the footprint is entirely clamped away.
#[allow(clippy::too_many_arguments)]
pub fn footprint_segments(
    part: usize,
    face: usize,
    layer: SkinLayer,
    arm: ArmModel,
    texel_x: u32,
    texel_y: u32,
    brush: u32,
    limb_frac: f32,
    anchor: [f32; 3],
    yaw: f32,
    scale: f32,
) -> Vec<Segment> {
    if part >= PART_COUNT || face >= 6 {
        return Vec::new();
    }
    let Some((fx0, fy0, fx1, fy1)) =
        footprint_rect(part, face, layer, arm, texel_x, texel_y, brush)
    else {
        return Vec::new();
    };
    let Some((x0, y0, x1, y1)) = crate::skin_uv::face_rect_px(part, face, layer, arm) else {
        return Vec::new();
    };
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let (min, max) = shell_box(part, layer, limb_frac, arm);
    let eps = FOOTPRINT_EPS_WORLD / scale.max(1e-6);
    let u0 = (fx0 - x0) as f32 / w as f32;
    let u1 = (fx1 - x0) as f32 / w as f32;
    let v0 = (fy0 - y0) as f32 / h as f32;
    let v1 = (fy1 - y0) as f32 / h as f32;
    let corner = |fu: f32, fv: f32| lifted(face, min, max, fu, fv, eps, anchor, yaw, scale);
    let a = corner(u0, v0);
    let b = corner(u1, v0);
    let c = corner(u1, v1);
    let d = corner(u0, v1);
    vec![(a, b), (b, c), (c, d), (d, a)]
}

/// Session-lived state for the paint aids: the on/off toggle plus a memo of the
/// last grid built, so the ~584 segments are only recomputed when the pose or
/// the visible shell actually changes (the R ease animates for ~0.2 s; the rest
/// of the time the key is unchanged and the cached `Vec` is handed straight
/// back).
#[derive(Clone, Debug)]
pub struct GridAid {
    /// Draw the texel grid? On by default — the aid exists because texels are
    /// hard to judge by eye, and a kid should not have to find a toggle first.
    pub on: bool,
    /// `(limb_frac bits, clothes_on, arm model)` the cache was built for.
    key: Option<(u32, bool, ArmModel)>,
    cached: Vec<Segment>,
}

impl Default for GridAid {
    fn default() -> Self {
        Self { on: true, key: None, cached: Vec::new() }
    }
}

impl GridAid {
    /// The grid for this pose, rebuilding only on a change of pose or shell.
    /// Returns an empty slice when the grid is switched off.
    pub fn segments(
        &mut self,
        limb_frac: f32,
        clothes_on: bool,
        arm: ArmModel,
        anchor: [f32; 3],
        yaw: f32,
        scale: f32,
    ) -> &[Segment] {
        if !self.on {
            return &[];
        }
        let key = (limb_frac.to_bits(), clothes_on, arm);
        if self.key != Some(key) {
            self.cached = grid_segments(limb_frac, clothes_on, arm, anchor, yaw, scale);
            self.key = Some(key);
        }
        &self.cached
    }

    /// Drop the memo (e.g. when the session closes) so a stale pose can never be
    /// handed back to a later session.
    pub fn invalidate(&mut self) {
        self.key = None;
        self.cached = Vec::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin_uv::{ArmModel, SkinLayer, face_rect_px};

    const ANCHOR: [f32; 3] = crate::workshop::WORKSHOP_MANNEQUIN_POS;
    const YAW: f32 = crate::workshop::WORKSHOP_MANNEQUIN_YAW;
    const SCALE: f32 = crate::workshop::AVATAR_BLOW_UP_SCALE;
    /// The yaw at which the builder's `yaw + π/2` is zero, so model space and
    /// world space share axes — used by the tests that measure "how far out"
    /// along a named model axis and would otherwise be reading a rotated one.
    const IDENT_YAW: f32 = -std::f32::consts::FRAC_PI_2;
    /// Every legacy test predates the slim model and speaks about the classic one.
    const CLASSIC: ArmModel = ArmModel::Classic;

    fn approx(a: f32, b: f32, tol: f32) {
        assert!((a - b).abs() < tol, "expected {b}, got {a}");
    }

    // ── the transform pairing ────────────────────────────────────────────────

    #[test]
    fn model_to_world_round_trips_through_the_paint_ray_inverse() {
        // THE pairing: the painter turns a world ray into model space with
        // `world_ray_to_avatar_model`; the aids turn model points back into
        // world space with `avatar_model_to_world`. If they ever stop being
        // exact inverses, the grid drifts off the texels the brush writes.
        for &yaw in &[0.0f32, YAW, 1.234, -2.7, std::f32::consts::PI] {
            for &scale in &[1.0f32, SCALE, 7.5] {
                for &p in &[
                    [0.0f32, 0.0, 0.0],
                    [0.25, 1.575, -0.25],
                    [-0.75, 2.075, 0.125],
                    [0.375, 1.05, 0.0],
                ] {
                    let w = avatar_model_to_world(p, ANCHOR, yaw, scale);
                    let (back, _) = crate::workshop::world_ray_to_avatar_model(
                        w,
                        [0.0, 0.0, 1.0],
                        ANCHOR,
                        yaw,
                        scale,
                    );
                    for a in 0..3 {
                        approx(back[a], p[a], 1e-4);
                    }
                }
            }
        }
    }

    #[test]
    fn model_to_world_directions_round_trip_too() {
        // The ray's DIRECTION is unscaled by the same rotation, so a model-space
        // delta must map to a world delta of `scale ×` its length.
        let a = avatar_model_to_world([0.0, 0.0, 0.0], ANCHOR, YAW, SCALE);
        let b = avatar_model_to_world([0.5, 0.0, 0.0], ANCHOR, YAW, SCALE);
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        approx(len, 0.5 * SCALE, 1e-4);
    }

    #[test]
    fn model_to_world_at_the_workshop_yaw_is_a_half_turn() {
        // WORKSHOP_MANNEQUIN_YAW is π/2, so the builder's yaw is π — a half turn
        // about Y, which negates x and z. Pinned so a change to the mannequin's
        // facing is a deliberate, visible edit rather than a silent drift.
        let w = avatar_model_to_world([0.25, 1.575, 0.125], ANCHOR, YAW, SCALE);
        approx(w[0], ANCHOR[0] - 0.25 * SCALE, 1e-4);
        approx(w[1], ANCHOR[1] + 1.575 * SCALE, 1e-4);
        approx(w[2], ANCHOR[2] - 0.125 * SCALE, 1e-4);
    }

    // ── face_point: the inverse of skin_hit::frac_uv ─────────────────────────

    #[test]
    fn face_point_inverts_the_hit_tests_frac_uv_on_all_six_faces() {
        // Cast a real paint ray at a known OFF-CENTRE point on each of the head
        // box's six faces, then rebuild that point from the hit's (frac_u,
        // frac_v). This is the strongest available statement that the grid lies
        // on the texels the brush writes: it pins `face_point` against the very
        // `frac_uv` the painter runs, on every face and off both axes (so a
        // transposed or flipped row cannot slip through).
        //
        // Origins are chosen to reach the HEAD without a limb or the torso
        // getting in first — notably the −Y probe, which comes up at z = 0.2,
        // clear of the legs and body (both only ±0.125 deep).
        let (min, max) = shell_box(0, SkinLayer::Base, 0.0, CLASSIC); // head, together
        let probes: [(usize, [f32; 3], [f32; 3], [f32; 3]); 6] = [
            (0, [5.0, 1.6, 0.1], [-1.0, 0.0, 0.0], [max[0], 1.6, 0.1]),
            (1, [-5.0, 1.6, 0.1], [1.0, 0.0, 0.0], [min[0], 1.6, 0.1]),
            (2, [0.1, 5.0, -0.1], [0.0, -1.0, 0.0], [0.1, max[1], -0.1]),
            (3, [0.1, -5.0, 0.2], [0.0, 1.0, 0.0], [0.1, min[1], 0.2]),
            (4, [0.1, 1.6, 5.0], [0.0, 0.0, -1.0], [0.1, 1.6, max[2]]),
            (5, [0.1, 1.6, -5.0], [0.0, 0.0, 1.0], [0.1, 1.6, min[2]]),
        ];
        for (face, origin, dir, want) in probes {
            let hit =
                crate::skin_hit::ray_hit_avatar(origin, dir, SkinLayer::Base, 0.0, false, CLASSIC)
                .unwrap_or_else(|| panic!("probe for face {face} should hit the head"));
            assert_eq!(hit.part, 0, "probe for face {face} must reach the head");
            assert_eq!(hit.face, face, "probe entered face {} not {face}", hit.face);
            let p = face_point(hit.face, min, max, hit.frac_u, hit.frac_v);
            for a in 0..3 {
                approx(p[a], want[a], 1e-3);
            }
        }
    }

    // ── grid_segments ────────────────────────────────────────────────────────

    fn expected_segment_count(clothes_on: bool, arm: ArmModel) -> usize {
        let layer = visible_layer(clothes_on);
        let mut n = 0usize;
        for part in 0..PART_COUNT {
            for face in 0..6 {
                let (x0, y0, x1, y1) = face_rect_px(part, face, layer, arm).unwrap();
                n += (x1 - x0) as usize + 1 + (y1 - y0) as usize + 1;
            }
        }
        n
    }

    #[test]
    fn grid_has_one_line_per_texel_boundary_on_every_face() {
        for arm in [ArmModel::Classic, ArmModel::Slim] {
            for clothes_on in [false, true] {
                let segs = grid_segments(0.0, clothes_on, arm, ANCHOR, YAW, SCALE);
                assert_eq!(
                    segs.len(),
                    expected_segment_count(clothes_on, arm),
                    "{arm:?} clothes_on={clothes_on}: one line per texel boundary, (w+1)+(h+1) per face"
                );
            }
        }
    }

    #[test]
    fn head_front_face_contributes_eighteen_segments() {
        // The 8×8 head front is the reference case: 9 lines one way, 9 the
        // other. Counted out of the REAL grid by finding the segments that lie
        // on the head-front plane, so this checks placement, not just arithmetic.
        let (x0, y0, x1, y1) = face_rect_px(0, 5, SkinLayer::Base, CLASSIC).unwrap();
        assert_eq!((x1 - x0, y1 - y0), (8, 8), "head front is 8×8 texels");

        let eps = GRID_EPS_WORLD / SCALE;
        let (min, max) = shell_box(0, SkinLayer::Base, 0.0, CLASSIC);
        let plane_z = min[2] - eps; // front face is −Z, pushed outward
        let on_head_front = |w: &[f32; 3]| {
            let (m, _) =
                crate::workshop::world_ray_to_avatar_model(*w, [0.0, 0.0, 1.0], ANCHOR, YAW, SCALE);
            // Tolerance well under `eps`: the neighbouring faces' edge lines sit
            // at z = min.z (pushed out along ±X/±Y instead), exactly `eps` away,
            // and must NOT be counted as front-face lines.
            (m[2] - plane_z).abs() < 1e-4
                && m[0] >= min[0] - 1e-3
                && m[0] <= max[0] + 1e-3
                && m[1] >= min[1] - 1e-3
                && m[1] <= max[1] + 1e-3
        };
        let n = grid_segments(0.0, false, CLASSIC, ANCHOR, YAW, SCALE)
            .iter()
            .filter(|(a, b)| on_head_front(a) && on_head_front(b))
            .count();
        assert_eq!(n, 18, "9 lines across + 9 down on the 8×8 head front");
    }

    #[test]
    fn every_grid_endpoint_sits_just_off_its_face_plane() {
        // Inverse-transform each endpoint back to model space and confirm it
        // lies on the face plane of SOME part, pushed outward by exactly the
        // grid epsilon — i.e. the lines hug the shell instead of floating in
        // space or sinking into it.
        let eps = GRID_EPS_WORLD / SCALE;
        for clothes_on in [false, true] {
            let layer = visible_layer(clothes_on);
            let segs = grid_segments(0.0, clothes_on, CLASSIC, ANCHOR, YAW, SCALE);
            for (a, b) in &segs {
                for w in [a, b] {
                    let (m, _) = crate::workshop::world_ray_to_avatar_model(
                        *w,
                        [0.0, 0.0, 1.0],
                        ANCHOR,
                        YAW,
                        SCALE,
                    );
                    let on_a_face = (0..PART_COUNT).any(|part| {
                        let (lo, hi) = shell_box(part, layer, 0.0, CLASSIC);
                        (0..3).any(|ax| {
                            (m[ax] - (hi[ax] + eps)).abs() < 1e-3
                                || (m[ax] - (lo[ax] - eps)).abs() < 1e-3
                        }) && (0..3).all(|ax| {
                            m[ax] >= lo[ax] - eps - 1e-3 && m[ax] <= hi[ax] + eps + 1e-3
                        })
                    });
                    assert!(on_a_face, "endpoint {w:?} (model {m:?}) is not on any face plane");
                }
            }
        }
    }

    #[test]
    fn grid_lines_span_a_whole_texel_on_the_head_front() {
        // Pitch check: consecutive constant-u lines on the 8-texel-wide head
        // front must be one texel apart in world units, i.e. the head's 0.5
        // model width / 8 texels × the blow-up scale.
        let eps = GRID_EPS_WORLD / SCALE;
        let (min, max) = shell_box(0, SkinLayer::Base, 0.0, CLASSIC);
        let p0 = lifted(5, min, max, 0.0, 0.5, eps, ANCHOR, YAW, SCALE);
        let p1 = lifted(5, min, max, 1.0 / 8.0, 0.5, eps, ANCHOR, YAW, SCALE);
        let d = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        approx(len, (0.5 / 8.0) * SCALE, 1e-4);
    }

    #[test]
    fn clothes_shell_grid_sits_outside_the_body_grid() {
        // The clothes shell is proud of the body by EDIT_INFLATE, so its grid
        // must be further from the model centre — the visible shell really is
        // the one being outlined.
        let body = grid_segments(0.0, false, CLASSIC, [0.0, 0.0, 0.0], IDENT_YAW, 1.0);
        let clothes = grid_segments(0.0, true, CLASSIC, [0.0, 0.0, 0.0], IDENT_YAW, 1.0);
        let widest = |segs: &[Segment]| {
            segs.iter()
                .flat_map(|(a, b)| [a[0].abs(), b[0].abs()])
                .fold(0.0f32, f32::max)
        };
        assert!(
            widest(&clothes) > widest(&body) + 0.05,
            "clothes grid ({}) should be proud of the body grid ({})",
            widest(&clothes),
            widest(&body)
        );
    }

    #[test]
    fn grid_follows_the_limbs_apart_pose() {
        // The arms swing out with R; the grid must travel with them or it stops
        // describing the thing under the crosshair.
        let together = grid_segments(0.0, false, CLASSIC, [0.0, 0.0, 0.0], IDENT_YAW, 1.0);
        let apart = grid_segments(1.0, false, CLASSIC, [0.0, 0.0, 0.0], IDENT_YAW, 1.0);
        assert_eq!(together.len(), apart.len(), "same line count either way");
        let widest = |segs: &[Segment]| {
            segs.iter()
                .flat_map(|(a, b)| [a[0].abs(), b[0].abs()])
                .fold(0.0f32, f32::max)
        };
        assert!(
            widest(&apart) > widest(&together) + 0.2,
            "apart grid ({}) must reach further than together ({})",
            widest(&apart),
            widest(&together)
        );
    }

    // ── footprint ────────────────────────────────────────────────────────────

    /// The bounding box of the pixels `skin_paint::pencil` actually writes —
    /// the ground truth the footprint outline has to match.
    #[allow(clippy::too_many_arguments)]
    fn pencil_bounds(
        part: usize,
        face: usize,
        layer: SkinLayer,
        arm: ArmModel,
        cx: u32,
        cy: u32,
        brush: u32,
    ) -> Option<(u32, u32, u32, u32)> {
        let (x0, y0, x1, y1) = face_rect_px(part, face, layer, arm)?;
        let mut buf = vec![0u8; 64 * 64 * 4];
        let rect = crate::skin_paint::PixelRect { x0, y0, x1, y1 };
        crate::skin_paint::pencil(&mut buf, cx, cy, brush, [255, 0, 0, 255], &rect);
        let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut any = false;
        for y in 0..64u32 {
            for x in 0..64u32 {
                if crate::skin_paint::get_pixel(&buf, x, y)[3] != 0 {
                    any = true;
                    lo_x = lo_x.min(x);
                    lo_y = lo_y.min(y);
                    hi_x = hi_x.max(x + 1);
                    hi_y = hi_y.max(y + 1);
                }
            }
        }
        any.then_some((lo_x, lo_y, hi_x, hi_y))
    }

    #[test]
    fn footprint_rect_matches_what_pencil_actually_paints() {
        // Every brush size, every face, and the awkward positions: centres,
        // corners and edges, where clamping decides the answer.
        for arm in [ArmModel::Classic, ArmModel::Slim] {
        for layer in [SkinLayer::Base, SkinLayer::Overlay] {
            for part in 0..PART_COUNT {
                for face in 0..6 {
                    let (x0, y0, x1, y1) = face_rect_px(part, face, layer, arm).unwrap();
                    let probes = [
                        (x0, y0),
                        (x1 - 1, y0),
                        (x0, y1 - 1),
                        (x1 - 1, y1 - 1),
                        ((x0 + x1) / 2, (y0 + y1) / 2),
                    ];
                    for (cx, cy) in probes {
                        for brush in 1u32..=3 {
                            assert_eq!(
                                footprint_rect(part, face, layer, arm, cx, cy, brush),
                                pencil_bounds(part, face, layer, arm, cx, cy, brush),
                                "{arm:?} part {part} face {face} {layer:?} at ({cx},{cy}) brush {brush}"
                            );
                        }
                    }
                }
            }
        }
        }
    }

    #[test]
    fn brush_one_outlines_exactly_one_texel() {
        let (x0, y0, _, _) = face_rect_px(0, 5, SkinLayer::Base, CLASSIC).unwrap();
        let r = footprint_rect(0, 5, SkinLayer::Base, CLASSIC, x0 + 3, y0 + 4, 1).unwrap();
        assert_eq!(r, (x0 + 3, y0 + 4, x0 + 4, y0 + 5));
        let segs = footprint_segments(
            0,
            5,
            SkinLayer::Base,
            CLASSIC,
            x0 + 3,
            y0 + 4,
            1,
            0.0,
            ANCHOR,
            YAW,
            SCALE,
        );
        assert_eq!(segs.len(), 4, "a rectangle outline is four segments");
        // Each side is one texel long: the head front is 0.5 model units over
        // 8 texels, blown up by SCALE.
        let texel = (0.5 / 8.0) * SCALE;
        for (a, b) in &segs {
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            approx(len, texel, 1e-4);
        }
    }

    #[test]
    fn brush_three_at_a_face_corner_shrinks_to_what_is_paintable() {
        // A 3×3 centred on the top-left texel spills 1 texel off two edges;
        // `pencil` clamps it to 2×2, so the outline must shrink to match rather
        // than promising paint that never lands.
        let (x0, y0, _, _) = face_rect_px(0, 5, SkinLayer::Base, CLASSIC).unwrap();
        let r = footprint_rect(0, 5, SkinLayer::Base, CLASSIC, x0, y0, 3).unwrap();
        assert_eq!(r, (x0, y0, x0 + 2, y0 + 2), "clamped to the 2×2 that is on-face");
        let segs = footprint_segments(0, 5, SkinLayer::Base, CLASSIC, x0, y0, 3, 0.0, ANCHOR, YAW, SCALE);
        let texel = (0.5 / 8.0) * SCALE;
        for (a, b) in &segs {
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            approx(len, 2.0 * texel, 1e-3);
        }
    }

    #[test]
    fn footprint_sits_further_out_than_the_grid() {
        // Where the brush box shares an edge with a gridline, the yellow box
        // must win — so it floats further from the shell.
        assert!(FOOTPRINT_EPS_WORLD > GRID_EPS_WORLD);
        let (x0, y0, _, _) = face_rect_px(0, 5, SkinLayer::Base, CLASSIC).unwrap();
        let (min, _max) = shell_box(0, SkinLayer::Base, 0.0, CLASSIC);
        let segs = footprint_segments(0, 5, SkinLayer::Base, CLASSIC, x0, y0, 1, 0.0, ANCHOR, YAW, SCALE);
        let (m, _) = crate::workshop::world_ray_to_avatar_model(
            segs[0].0,
            [0.0, 0.0, 1.0],
            ANCHOR,
            YAW,
            SCALE,
        );
        // Front face is −Z: the outline is pushed to z < min.z by the footprint
        // epsilon, which is bigger than the grid's.
        approx(m[2], min[2] - FOOTPRINT_EPS_WORLD / SCALE, 1e-4);
    }

    #[test]
    fn footprint_is_empty_for_an_out_of_range_part_or_face() {
        assert!(
            footprint_segments(9, 5, SkinLayer::Base, CLASSIC, 8, 8, 1, 0.0, ANCHOR, YAW, SCALE).is_empty()
        );
        assert!(
            footprint_segments(0, 9, SkinLayer::Base, CLASSIC, 8, 8, 1, 0.0, ANCHOR, YAW, SCALE).is_empty()
        );
        assert_eq!(footprint_rect(9, 5, SkinLayer::Base, CLASSIC, 8, 8, 1), None);
    }

    #[test]
    fn footprint_lands_on_the_texel_the_crosshair_is_over() {
        // End-to-end: ray → skin_hit → texel_for → footprint. The outlined box
        // must contain the point the ray actually struck.
        let (min, max) = shell_box(0, SkinLayer::Base, 0.0, CLASSIC);
        let hit = crate::skin_hit::ray_hit_avatar(
            [0.10, 1.70, -5.0],
            [0.0, 0.0, 1.0],
            SkinLayer::Base,
            0.0,
            false,
            CLASSIC,
        )
        .expect("hit the head front");
        let (tx, ty) = crate::skin_uv::texel_for(
            hit.part,
            hit.face,
            hit.frac_u,
            hit.frac_v,
            SkinLayer::Base,
            CLASSIC,
        )
        .unwrap();
        let (fx0, fy0, fx1, fy1) =
            footprint_rect(hit.part, hit.face, SkinLayer::Base, CLASSIC, tx, ty, 1).unwrap();
        assert!(tx >= fx0 && tx < fx1 && ty >= fy0 && ty < fy1);
        // And the world-space box straddles the struck point.
        let segs = footprint_segments(
            hit.part,
            hit.face,
            SkinLayer::Base,
            CLASSIC,
            tx,
            ty,
            1,
            0.0,
            ANCHOR,
            YAW,
            SCALE,
        );
        let hit_world = avatar_model_to_world(
            face_point(hit.face, min, max, hit.frac_u, hit.frac_v),
            ANCHOR,
            YAW,
            SCALE,
        );
        let texel = (0.5 / 8.0) * SCALE;
        let near = segs.iter().any(|(a, _b)| {
            let d = [a[0] - hit_world[0], a[1] - hit_world[1], a[2] - hit_world[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < texel * 1.5
        });
        assert!(near, "the outline corners must be within a texel of the hit point");
    }

    // ── GridAid memo ─────────────────────────────────────────────────────────

    #[test]
    fn grid_aid_defaults_on() {
        assert!(GridAid::default().on, "the grid is an aid, not an easter egg");
    }

    #[test]
    fn grid_aid_serves_the_cache_until_the_pose_changes() {
        let mut aid = GridAid::default();
        let n = aid.segments(0.0, false, CLASSIC, ANCHOR, YAW, SCALE).len();
        assert!(n > 0);
        assert_eq!(aid.key, Some((0.0f32.to_bits(), false, CLASSIC)));
        // Same key → same cached vector, no rebuild.
        let first = aid.cached.clone();
        assert_eq!(aid.segments(0.0, false, CLASSIC, ANCHOR, YAW, SCALE).len(), n);
        assert_eq!(aid.cached, first);
        // Pose change → rebuild (the arms move, so the geometry differs).
        aid.segments(1.0, false, CLASSIC, ANCHOR, YAW, SCALE);
        assert_eq!(aid.key, Some((1.0f32.to_bits(), false, CLASSIC)));
        assert_ne!(aid.cached, first);
        // Shell change → rebuild too.
        aid.segments(1.0, true, CLASSIC, ANCHOR, YAW, SCALE);
        assert_eq!(aid.key, Some((1.0f32.to_bits(), true, CLASSIC)));
    }

    #[test]
    fn grid_aid_off_yields_nothing() {
        let mut aid = GridAid::default();
        aid.on = false;
        assert!(aid.segments(0.0, false, CLASSIC, ANCHOR, YAW, SCALE).is_empty());
    }

    #[test]
    fn grid_aid_invalidate_drops_the_memo() {
        let mut aid = GridAid::default();
        aid.segments(0.0, false, CLASSIC, ANCHOR, YAW, SCALE);
        aid.invalidate();
        assert_eq!(aid.key, None);
        assert!(aid.cached.is_empty());
    }

    // ── Slim ("Alex") arms ───────────────────────────────────────────────────

    /// Count the gridlines lying on one face's plane, the way
    /// `head_front_face_contributes_eighteen_segments` does — placement, not
    /// arithmetic.
    fn lines_on_front_face(part: usize, arm: ArmModel) -> usize {
        let eps = GRID_EPS_WORLD / SCALE;
        let (min, max) = shell_box(part, SkinLayer::Base, 0.0, arm);
        let plane_z = min[2] - eps;
        let on_face = |w: &[f32; 3]| {
            let (m, _) =
                crate::workshop::world_ray_to_avatar_model(*w, [0.0, 0.0, 1.0], ANCHOR, YAW, SCALE);
            (m[2] - plane_z).abs() < 1e-4
                && m[0] >= min[0] - 1e-3
                && m[0] <= max[0] + 1e-3
                && m[1] >= min[1] - 1e-3
                && m[1] <= max[1] + 1e-3
        };
        grid_segments(0.0, false, arm, ANCHOR, YAW, SCALE)
            .iter()
            .filter(|(a, b)| on_face(a) && on_face(b))
            .count()
    }

    #[test]
    fn head_front_grid_is_unchanged_by_the_arm_model() {
        // The head is untouched by slim, so its 8×8 face must still be 18 lines
        // on both models — the guard that "slim" never leaks past the arms.
        assert_eq!(lines_on_front_face(0, ArmModel::Classic), 18);
        assert_eq!(lines_on_front_face(0, ArmModel::Slim), 18);
    }

    #[test]
    fn slim_arm_front_grid_has_four_vertical_lines() {
        // 3 texels across → 4 constant-u lines (plus 13 constant-v lines for
        // the 12 rows) = 17 segments, against classic's 5 + 13 = 18.
        let (x0, _, x1, _) = face_rect_px(3, 5, SkinLayer::Base, ArmModel::Slim).unwrap();
        assert_eq!(x1 - x0, 3, "slim arm front is 3 texels wide");
        assert_eq!(lines_on_front_face(3, ArmModel::Slim), 17, "4 across + 13 down");
        assert_eq!(lines_on_front_face(3, ArmModel::Classic), 18, "5 across + 13 down");
    }

    #[test]
    fn slim_grid_lines_span_one_slim_texel() {
        // Pitch: the slim arm front is 3/16 of a block over 3 texels, so a
        // texel is 1/16 — the SAME world pitch as classic. If the box shrank
        // but the rect didn't (or vice versa) this is where it shows.
        let eps = GRID_EPS_WORLD / SCALE;
        for arm in [ArmModel::Classic, ArmModel::Slim] {
            let (min, max) = shell_box(3, SkinLayer::Base, 0.0, arm);
            let (x0, _, x1, _) = face_rect_px(3, 5, SkinLayer::Base, arm).unwrap();
            let w = (x1 - x0) as f32;
            let p0 = lifted(5, min, max, 0.0, 0.5, eps, ANCHOR, YAW, SCALE);
            let p1 = lifted(5, min, max, 1.0 / w, 0.5, eps, ANCHOR, YAW, SCALE);
            let d = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            // The edit shell is inflated, so the per-texel pitch is the box
            // width over the texel count — compare against that, not a constant.
            approx(len, ((max[0] - min[0]) / w) * SCALE, 1e-4);
        }
    }

    #[test]
    fn slim_footprint_lands_on_the_texel_the_crosshair_is_over() {
        // End-to-end on a slim arm: ray → skin_hit → texel_for → footprint. The
        // outlined box must contain the struck texel, with the SLIM rects and
        // the SLIM box on every hop.
        let arm = ArmModel::Slim;
        let (min, max) = shell_box(3, SkinLayer::Base, 0.0, arm);
        let mid_x = (min[0] + max[0]) * 0.5;
        let hit = crate::skin_hit::ray_hit_avatar(
            [mid_x, 1.0, -5.0],
            [0.0, 0.0, 1.0],
            SkinLayer::Base,
            0.0,
            true,
            arm,
        )
        .expect("hit the slim right arm front");
        assert_eq!((hit.part, hit.face), (3, 5));
        let (tx, ty) =
            crate::skin_uv::texel_for(hit.part, hit.face, hit.frac_u, hit.frac_v, SkinLayer::Base, arm)
                .unwrap();
        let (fx0, fy0, fx1, fy1) =
            footprint_rect(hit.part, hit.face, SkinLayer::Base, arm, tx, ty, 1).unwrap();
        assert!(tx >= fx0 && tx < fx1 && ty >= fy0 && ty < fy1);
        // The struck texel must be inside the SLIM front rect (x 44..47), not
        // the classic one it replaced.
        assert!((44..47).contains(&tx), "slim front texel x={tx} must be in 44..47");
        let segs = footprint_segments(
            hit.part,
            hit.face,
            SkinLayer::Base,
            arm,
            tx,
            ty,
            1,
            0.0,
            ANCHOR,
            YAW,
            SCALE,
        );
        assert_eq!(segs.len(), 4, "a rectangle outline is four segments");
    }

    #[test]
    fn grid_aid_rebuilds_when_the_arm_model_changes() {
        // The mannequin follows the Arms toggle live, so a memo keyed only on
        // pose/shell would leave a classic grid draped over a slim arm.
        let mut aid = GridAid::default();
        let classic = aid.segments(0.0, false, ArmModel::Classic, ANCHOR, YAW, SCALE).to_vec();
        let slim = aid.segments(0.0, false, ArmModel::Slim, ANCHOR, YAW, SCALE).to_vec();
        assert_eq!(aid.key, Some((0.0f32.to_bits(), false, ArmModel::Slim)));
        assert_ne!(classic.len(), slim.len(), "slim has fewer arm gridlines");
    }
}

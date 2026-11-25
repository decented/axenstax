//! A cast fishing line, drawn: a small red-and-white bobber floating on the water
//! the cast found, and a thin line sagging from the rod's tip to it (Spec 05
//! "Fishing"; Spec 03 §5.10).
//!
//! **Presentation only.** The rules (`fishing`) never read any of this. The client
//! keeps, with the line, the water point the cast found ([`FishingLine::target`])
//! and the tick it went out ([`FishingLine::cast_tick`]); this module turns those,
//! the rod's tip and the clock into vertices for the entity pipeline, next to the
//! held items. A bite (`FishingLine::hooked`) pulls the bobber under and keeps it
//! jittering until the line is reeled in.
//!
//! **The rod's tip** comes from the hand's own transform, so the line leaves the
//! very rod that was drawn: in third person from the avatar builder
//! (`entity_model::build_player_avatar_vertices_framed`), in first person from the
//! viewmodel placement (`viewmodel::rod_tip_view`) carried through to the world
//! pass by [`unproject_tip`] (the viewmodel is drawn in its own view space with its
//! own projection, so its tip is not a world point; the line starts on the same
//! screen spot, a little in front of the eye).
//!
//! **While a line is out** the rod in hand is drawn bare
//! (`held_tool_model::tool_model_with`, `HeldItem::line_out`), so the baked line,
//! float and hook are not drawn twice; when the line is reeled in the rigged rod
//! comes back on its own, because nothing here is stored.
//!
//! No texture layers: every box is one flat texel of a tile that already exists
//! (string for the line, red dye and wool for the bobber), as the held tools are.

use glam::{Mat3, Mat4, Vec3};

use crate::crafting::ToolType;
use crate::fishing::FishingLine;
use crate::held_tool_model::{centre_paint, Paint};
use crate::mesh::Vertex;
use crate::viewmodel::HandSide;

/// Ticks the bobber takes to fly from the rod's tip to the water (20 TPS).
pub const FLY_TICKS: f32 = 6.0;
/// The line is this many straight boxes along its sag.
const LINE_SEGMENTS: usize = 12;
/// Side of the line's square section, in blocks (a bit thick: at 12 blocks it is
/// still a pixel or two on a 720p screen).
const LINE_THICKNESS: f32 = 0.028;
/// The bobber's white lower half and red upper half (width, height, depth).
const WHITE_SIZE: Vec3 = Vec3::new(0.14, 0.10, 0.14);
const RED_SIZE: Vec3 = Vec3::new(0.11, 0.10, 0.11);
/// How far in front of the eye (along the ray through the rod tip's screen spot)
/// a first-person line starts.
pub const FIRST_PERSON_LINE_START: f32 = 0.6;

/// What to draw for a line that is out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CastView {
    /// The hand whose rod the line leaves.
    pub side: HandSide,
    /// The water point the bobber floats on.
    pub target: Vec3,
    /// Client ticks since the cast, with the fraction of the tick in progress.
    pub age_ticks: f32,
    pub hooked: bool,
}

/// The hand side of the hand a cast was made with.
pub fn side_of(hand: crate::hands::Hand) -> HandSide {
    match hand {
        crate::hands::Hand::Main => HandSide::Right,
        crate::hands::Hand::Off => HandSide::Left,
    }
}

/// The view of `line` at client tick `now` (`frac` of the next tick done), with
/// the tools in the two hands and the selected hotbar slot. `None` when there is
/// nothing to draw: no line, no water point (a joiner whose own world shows none),
/// or the rod the line was cast with is no longer where it was cast from (a second
/// rod selected, or the rod put away): the line is drawn from THAT rod only.
pub fn cast_view(
    line: Option<FishingLine>,
    hotbar_slot: usize,
    main: Option<ToolType>,
    off: Option<ToolType>,
    now: u64,
    frac: f32,
) -> Option<CastView> {
    let line = line?;
    let target = line.target?;
    if !line.rod_in_place(hotbar_slot, main == Some(ToolType::FishingRod), off == Some(ToolType::FishingRod)) {
        return None;
    }
    Some(CastView {
        side: side_of(line.hand),
        target,
        age_ticks: now.saturating_sub(line.cast_tick) as f32 + frac.clamp(0.0, 1.0),
        hooked: line.hooked,
    })
}

/// Seconds the bobber has bobbed for (it bobs from the cast).
fn seconds(view: &CastView) -> f32 {
    view.age_ticks / 20.0
}

/// The bobber's middle (the waterline between its white and red halves) once it
/// has landed: floating a little proud of the surface with a gentle bob, or, on a
/// bite, pulled under with a quick jitter.
pub fn settled_centre(view: &CastView) -> Vec3 {
    let t = seconds(view);
    let lift = if view.hooked { -0.07 + 0.04 * (t * 22.0).sin() } else { 0.05 + 0.012 * (t * 3.0).sin() };
    Vec3::new(view.target.x, view.target.y + lift, view.target.z)
}

/// The bobber's middle now: flying out from the rod's `tip` on an arc for
/// [`FLY_TICKS`], then [`settled_centre`].
pub fn bobber_centre(tip: Vec3, view: &CastView) -> Vec3 {
    let rest = settled_centre(view);
    if view.age_ticks >= FLY_TICKS {
        return rest;
    }
    let f = (view.age_ticks / FLY_TICKS).clamp(0.0, 1.0);
    let arc = (0.12 * tip.distance(rest)).clamp(0.3, 1.5) * (std::f32::consts::PI * f).sin();
    tip.lerp(rest, f) + Vec3::Y * arc
}

/// Where the line is tied: the top of the bobber's red half.
pub fn line_end(centre: Vec3) -> Vec3 {
    centre + Vec3::Y * RED_SIZE.y
}

/// How far the line sags below the straight line between its ends: more when it
/// is long and slack, little when a bite pulls it taut.
pub fn sag(length: f32, hooked: bool) -> f32 {
    let slack = (0.06 * length).clamp(0.08, 0.7);
    if hooked { slack * 0.4 } else { slack }
}

/// `n + 1` points from `a` to `b` along a parabola sagging `sag` below the chord.
pub fn line_points(a: Vec3, b: Vec3, sag: f32, n: usize) -> Vec<Vec3> {
    (0..=n)
        .map(|i| {
            let s = i as f32 / n as f32;
            a.lerp(b, s) - Vec3::Y * (sag * 4.0 * s * (1.0 - s))
        })
        .collect()
}

/// An oriented box: its centre, the frame its axes lie along (columns x, y, z; a
/// rotation) and its size, one flat texel on every face, wound like
/// `held_tool_model::push_part`.
fn push_box(verts: &mut Vec<Vertex>, centre: Vec3, frame: Mat3, size: Vec3, paint: Paint) {
    let h = size * 0.5;
    let corner = |sx: f32, sy: f32, sz: f32| centre + frame * Vec3::new(sx * h.x, sy * h.y, sz * h.z);
    let c = [
        corner(-1.0, -1.0, -1.0),
        corner(1.0, -1.0, -1.0),
        corner(1.0, 1.0, -1.0),
        corner(-1.0, 1.0, -1.0),
        corner(-1.0, -1.0, 1.0),
        corner(1.0, -1.0, 1.0),
        corner(1.0, 1.0, 1.0),
        corner(-1.0, 1.0, 1.0),
    ];
    let faces: [([f32; 3], [usize; 4]); 6] = [
        ([1.0, 0.0, 0.0], [1, 5, 6, 2]),
        ([-1.0, 0.0, 0.0], [4, 0, 3, 7]),
        ([0.0, 1.0, 0.0], [3, 2, 6, 7]),
        ([0.0, -1.0, 0.0], [4, 5, 1, 0]),
        ([0.0, 0.0, 1.0], [5, 4, 7, 6]),
        ([0.0, 0.0, -1.0], [0, 1, 2, 3]),
    ];
    let uv = paint.uv();
    for (n, idx) in faces {
        let normal = (frame * Vec3::from(n)).to_array();
        let [a, b, cc, d] = idx.map(|i| c[i]);
        for q in [a, b, cc, a, cc, d] {
            verts.push(Vertex {
                position: q.to_array(),
                normal,
                tex_layer: paint.layer,
                uv,
                light: Vertex::FULL_BRIGHT,
                sky_light: 0.0,
            });
        }
    }
}

/// A box along the segment `a` to `b`, `LINE_THICKNESS` square.
fn push_segment(verts: &mut Vec<Vertex>, a: Vec3, b: Vec3, paint: Paint) {
    let along = b - a;
    let len = along.length();
    if len < 1e-5 {
        return;
    }
    let y = along / len;
    let helper = if y.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
    let x = helper.cross(y).normalize();
    let z = x.cross(y);
    push_box(verts, (a + b) * 0.5, Mat3::from_cols(x, y, z), Vec3::new(LINE_THICKNESS, len, LINE_THICKNESS), paint);
}

/// The line from `tip` to the bobber and the bobber, as entity-pipeline vertices
/// at full brightness (the caller lights them at the bobber, `apply_light_channels`).
pub fn build_vertices(tip: Vec3, view: &CastView) -> Vec<Vertex> {
    let centre = bobber_centre(tip, view);
    let end = line_end(centre);
    let mut verts = Vec::with_capacity((LINE_SEGMENTS + 2) * 36);
    let string = centre_paint(crate::entity_model::TEX_ITEM_STRING);
    let points = line_points(tip, end, sag(tip.distance(end), view.hooked), LINE_SEGMENTS);
    for pair in points.windows(2) {
        push_segment(&mut verts, pair[0], pair[1], string);
    }
    let white = centre_paint(crate::entity_model::TEX_ITEM_WOOL);
    let red = centre_paint(crate::entity_model::TEX_ITEM_RED_DYE);
    push_box(&mut verts, centre - Vec3::Y * (WHITE_SIZE.y * 0.5), Mat3::IDENTITY, WHITE_SIZE, white);
    push_box(&mut verts, centre + Vec3::Y * (RED_SIZE.y * 0.5), Mat3::IDENTITY, RED_SIZE, red);
    verts
}

/// A first-person rod tip in the world pass. `view_tip` is the tip in the
/// viewmodel's view space, `viewmodel_proj` that pass's projection,
/// `inv_world_view_proj` the inverse of the world camera's view-projection and
/// `eye` its origin: the point `depth` blocks from the eye along the world ray
/// through the same screen spot, so the line starts where the rod's tip is seen.
pub fn unproject_tip(view_tip: Vec3, viewmodel_proj: Mat4, inv_world_view_proj: Mat4, eye: Vec3, depth: f32) -> Vec3 {
    let c = viewmodel_proj * view_tip.extend(1.0);
    let (x, y) = (c.x / c.w, c.y / c.w);
    let at = |z: f32| {
        let p = inv_world_view_proj * glam::Vec4::new(x, y, z, 1.0);
        p.truncate() / p.w
    };
    let (a, b) = (at(0.0), at(1.0));
    // Whichever end of the ray is farther from the eye is the way into the world.
    let dir = if a.distance_squared(eye) <= b.distance_squared(eye) { b - a } else { a - b };
    eye + dir.normalize_or_zero() * depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_model::{TEX_ITEM_RED_DYE, TEX_ITEM_STRING, TEX_ITEM_WOOL};

    fn view(age: f32, hooked: bool) -> CastView {
        CastView { side: HandSide::Right, target: Vec3::new(2.0, 10.889, -6.0), age_ticks: age, hooked }
    }

    fn painted(verts: &[Vertex], layer: u32) -> Vec<Vec3> {
        let p = centre_paint(layer);
        verts.iter().filter(|v| v.tex_layer == p.layer && v.uv == p.uv()).map(|v| Vec3::from(v.position)).collect()
    }

    #[test]
    fn the_line_leaves_the_hand_that_cast_it() {
        use crate::hands::Hand;
        let rod = Some(ToolType::FishingRod);
        let water = Some(Vec3::new(1.0, 2.0, 3.0));
        let main = FishingLine::cast(100, 300, water).with_rod(Hand::Main, 2);
        let off = FishingLine::cast(100, 300, water).with_rod(Hand::Off, 2);
        assert_eq!(cast_view(Some(main), 2, rod, None, 103, 0.0).unwrap().side, HandSide::Right);
        assert_eq!(cast_view(Some(off), 2, None, rod, 103, 0.0).unwrap().side, HandSide::Left);
        // An off-hand cast with a second rod now in the main hand stays on the off-hand's rod.
        assert_eq!(cast_view(Some(off), 5, rod, rod, 103, 0.0).unwrap().side, HandSide::Left);
        // A main-hand cast does not jump to a second rod on another slot, nor to the off-hand's.
        assert!(cast_view(Some(main), 5, rod, None, 103, 0.0).is_none(), "another slot's rod");
        assert!(cast_view(Some(main), 2, None, rod, 103, 0.0).is_none(), "the off-hand's rod");
    }

    #[test]
    fn nothing_is_drawn_without_a_line_a_water_point_or_a_rod() {
        let line = FishingLine::cast(100, 300, Some(Vec3::new(1.0, 2.0, 3.0)));
        let rod = Some(ToolType::FishingRod);
        let v = cast_view(Some(line), 0, rod, None, 103, 0.5).expect("a cast line with a rod in hand");
        assert_eq!(v.target, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(v.side, HandSide::Right);
        assert!((v.age_ticks - 3.5).abs() < 1e-5, "{}", v.age_ticks);
        assert!(!v.hooked);
        assert!(cast_view(None, 0, rod, None, 103, 0.0).is_none(), "no line");
        assert!(cast_view(Some(FishingLine::cast(100, 300, None)), 0, rod, None, 103, 0.0).is_none(), "no water point");
        assert!(cast_view(Some(line), 0, Some(ToolType::Sword), None, 103, 0.0).is_none(), "rod put away");
        // A tick counter that reads before the cast (a clock reset) is a line at age 0, not a panic.
        assert_eq!(cast_view(Some(line), 0, rod, None, 50, 0.0).unwrap().age_ticks, 0.0);
    }

    #[test]
    fn the_line_runs_from_the_tip_to_the_bobber_and_sags_between() {
        let (a, b) = (Vec3::new(0.0, 12.0, 0.0), Vec3::new(0.0, 11.0, -6.0));
        let pts = line_points(a, b, 0.4, 12);
        assert_eq!(pts.len(), 13);
        assert!((pts[0] - a).length() < 1e-6, "starts at the tip");
        assert!((pts[12] - b).length() < 1e-6, "ends at the bobber");
        let mid = pts[6];
        let chord = a.lerp(b, 0.5);
        assert!((chord.y - mid.y - 0.4).abs() < 1e-5, "the middle hangs by the sag: {mid:?} vs {chord:?}");
        assert!(pts.windows(2).all(|w| (w[1] - w[0]).length() > 0.1), "every box has a length");
    }

    #[test]
    fn the_built_line_touches_the_tip_and_the_bobber() {
        let tip = Vec3::new(0.3, 12.0, 0.2);
        let v = view(40.0, false);
        let verts = build_vertices(tip, &v);
        assert!(!verts.is_empty() && verts.len() % 36 == 0, "whole boxes: {}", verts.len());
        assert!(verts.iter().all(|q| q.position.iter().all(|c| c.is_finite())));
        let line = painted(&verts, TEX_ITEM_STRING);
        assert_eq!(line.len(), LINE_SEGMENTS * 36, "a box per segment");
        let end = line_end(bobber_centre(tip, &v));
        let near = |p: Vec3| line.iter().map(|q| q.distance(p)).fold(f32::MAX, f32::min);
        assert!(near(tip) < LINE_THICKNESS, "the line starts at the tip ({})", near(tip));
        assert!(near(end) < LINE_THICKNESS, "and ends at the top of the bobber ({})", near(end));
    }

    #[test]
    fn the_bobber_is_red_over_white_and_floats_on_the_target() {
        let tip = Vec3::new(0.0, 12.0, 0.0);
        let v = view(40.0, false);
        let verts = build_vertices(tip, &v);
        let (red, white) = (painted(&verts, TEX_ITEM_RED_DYE), painted(&verts, TEX_ITEM_WOOL));
        assert_eq!(red.len(), 36);
        assert_eq!(white.len(), 36);
        let low = |v: &[Vec3]| v.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert!(low(&red) > low(&white), "the red half is the upper one");
        let c = bobber_centre(tip, &v);
        assert!((c.x - v.target.x).abs() < 1e-5 && (c.z - v.target.z).abs() < 1e-5, "over the water point");
        assert!((c.y - v.target.y).abs() < 0.07, "at the surface, not in the air or the depths: {c:?}");
        assert!(red.iter().chain(&white).all(|p| (p.x - c.x).abs() <= 0.08 && (p.z - c.z).abs() <= 0.08), "a small bobber");
    }

    #[test]
    fn a_bite_pulls_the_bobber_under() {
        for age in [30.0_f32, 31.0, 32.3, 40.0, 57.7, 100.0] {
            let calm = settled_centre(&view(age, false));
            for other in [30.0_f32, 31.0, 32.3, 40.0, 57.7, 100.0] {
                let bitten = settled_centre(&view(other, true));
                assert!(bitten.y < calm.y - 0.05, "hooked {bitten:?} at {other} vs calm {calm:?} at {age}");
            }
        }
        let calm = settled_centre(&view(40.0, false));
        assert!(calm.y > view(0.0, false).target.y, "a calm bobber rides above the surface");
        let bitten = settled_centre(&view(40.0, true));
        assert!(bitten.y < view(0.0, false).target.y, "a bitten one is under it");
        assert!(sag(8.0, true) < sag(8.0, false), "a bite takes the slack out");
    }

    #[test]
    fn the_bobber_flies_from_the_tip_to_the_water_over_a_few_ticks() {
        let tip = Vec3::new(0.0, 12.0, 0.0);
        let start = bobber_centre(tip, &view(0.0, false));
        assert!((start - tip).length() < 1e-5, "leaves the tip: {start:?}");
        let mid = bobber_centre(tip, &view(FLY_TICKS * 0.5, false));
        let straight = tip.lerp(settled_centre(&view(FLY_TICKS * 0.5, false)), 0.5);
        assert!(mid.y > straight.y + 0.2, "in an arc, above the straight path");
        let landed = bobber_centre(tip, &view(FLY_TICKS, false));
        assert!((landed - settled_centre(&view(FLY_TICKS, false))).length() < 1e-5, "lands on the water");
        let mut last = tip;
        for i in 0..=12 {
            let age = FLY_TICKS * i as f32 / 12.0;
            let p = bobber_centre(tip, &view(age, false));
            assert!((p - last).length() < 2.0, "no jump between steps ({age})");
            last = p;
        }
    }

    #[test]
    fn a_first_person_tip_starts_the_line_on_its_screen_spot_in_front_of_the_eye() {
        let eye = Vec3::new(4.0, 70.0, -2.0);
        let (yaw, pitch) = (0.6_f32, -0.2_f32);
        let mut cam = crate::camera::Camera::new(eye, 16.0 / 9.0);
        cam.yaw = yaw;
        cam.pitch = pitch;
        let inv = cam.view_projection_matrix().inverse();
        let vm = crate::viewmodel::viewmodel_projection(16.0 / 9.0);
        // Dead ahead in the viewmodel is dead ahead in the world.
        let ahead = unproject_tip(Vec3::new(0.0, 0.0, -1.5), vm, inv, cam.render_eye(), 0.6);
        let want = cam.render_eye() + cam.forward() * 0.6;
        assert!((ahead - want).length() < 1e-3, "{ahead:?} vs {want:?}");
        // A tip low and right in the viewmodel is low and right of the look, at the depth asked.
        let tip = unproject_tip(Vec3::new(0.5, -0.4, -1.5), vm, inv, cam.render_eye(), 0.6);
        let off = tip - cam.render_eye();
        assert!((off.length() - 0.6).abs() < 1e-3, "{}", off.length());
        assert!(off.dot(cam.forward()) > 0.4, "in front of the eye");
        assert!(off.dot(cam.right()) > 0.05, "to the right of the look");
        assert!(off.dot(cam.right().cross(cam.forward())) < -0.02, "below the look");
        // And it projects back to the spot it was taken from.
        let c = cam.view_projection_matrix() * tip.extend(1.0);
        let want = vm * Vec3::new(0.5, -0.4, -1.5).extend(1.0);
        assert!((c.x / c.w - want.x / want.w).abs() < 1e-3 && (c.y / c.w - want.y / want.w).abs() < 1e-3);
    }
}

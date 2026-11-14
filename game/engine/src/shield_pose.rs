//! The shield's poses (off-hand release, slice 5; Spec 03 §5.10 "The shield").
//!
//! The shield model (`held_tool_model`, `ToolType::Shield`) is a board in the
//! model's X-Y plane with its boss on the +Z face, centred on the origin. Every
//! view puts that one mesh where it belongs:
//!
//! * **First person, lowered:** the generic upright placement (`viewmodel`
//!   `held_item_placement`): the board faces the camera in the hand.
//! * **First person, raised** ([`raise_first_person`]): the shield and the arm
//!   that holds it come in toward the middle of the view and up, the board turned
//!   a little toward the centre and leaning in, so it covers the lower-middle of
//!   the view the way Minecraft's raised shield does. Its edge nearest the
//!   centre is held [`CENTRE_CLEAR_NDC`] clear of the crosshair at every aspect
//!   (the viewmodel layout spec keeps the centre +-5% clear).
//! * **Third person, lowered** ([`avatar_transform`]): worn on the outside of the
//!   forearm, the boss facing out.
//! * **Third person, raised:** the arm lifts forward ([`GUARD_ARM_ANGLE`]) and
//!   the board stands in front of the hand, boss forward.
//!
//! The avatar model is in `entity_model`'s model space: arms at +-X, the front
//! toward -Z, y up, before the yaw and the foot position are applied.

use glam::{Mat4, Vec3};

use crate::mesh::Vertex;
use crate::viewmodel::HandSide;

/// How far the arm holding a raised shield lifts forward about its shoulder
/// (radians; positive swings the hand toward the front, `entity_model::swing_angle`).
pub const GUARD_ARM_ANGLE: f32 = 1.25;

/// First person, raised: the board turns this far toward the centre (degrees
/// about the view y axis) ...
pub const RAISED_YAW_DEG: f32 = 18.0;
/// ... and leans in this far (degrees about the view z axis, the top toward the centre).
pub const RAISED_ROLL_DEG: f32 = 8.0;
/// First person, raised: the board's edge nearest the centre of the screen sits at
/// least this far from it (NDC, 0 = the middle, 1 = the screen edge). The
/// layout spec's clear box is 0.05; this keeps a margin for the lean and the bob.
pub const CENTRE_CLEAR_NDC: f32 = 0.12;
/// First person, raised: where the board's middle sits vertically (NDC, up positive).
pub const RAISED_CENTRE_Y_NDC: f32 = -0.10;
/// First person, raised: the board is at least this tall on screen (NDC, 2 = the
/// whole view; the layout spec wants 30% of the view's height, 0.60). A fixed-size
/// board in a narrow (portrait) view is short against the view's height and cannot
/// grow wider (its outer edge is on screen), so it is stretched upward only.
pub const RAISED_MIN_HEIGHT_NDC: f32 = 0.64;
/// The most the board is stretched taller to reach [`RAISED_MIN_HEIGHT_NDC`].
const RAISED_MAX_STRETCH: f32 = 1.6;

/// Third person: the board's centre on the outside of a lowered forearm, away from
/// the arm's own centre by half the arm and a little more.
const LOWERED_OUT: f32 = 0.17;
/// Third person, raised: the board's centre in front of the hand, and toward the
/// body's centre line by this fraction of the arm's own x.
const RAISED_FRONT: f32 = 0.06;
const RAISED_IN: f32 = 0.45;

/// Model-space transform of the shield mesh for an avatar's hand: `arm` is the
/// arm part (`entity_model::ModelPart`) of the hand `side`, `raised` the guard pose
/// (the arm is lifted by [`GUARD_ARM_ANGLE`]). Outputs are in model space, before
/// the yaw and the foot position.
pub fn avatar_transform(arm_origin: Vec3, arm_size: Vec3, arm_pivot: Vec3, side: HandSide, raised: bool) -> Mat4 {
    let s = side.sign();
    if raised {
        // Where the arm's bottom end lands when the arm swings forward about its
        // pivot (the rotation `build_part_vertices` makes).
        let hand = Vec3::new(arm_origin.x, arm_origin.y - arm_size.y * 0.5, arm_origin.z);
        let (dy, dz) = (hand.y - arm_pivot.y, hand.z - arm_pivot.z);
        let (sin, cos) = GUARD_ARM_ANGLE.sin_cos();
        let end = Vec3::new(hand.x, arm_pivot.y + dy * cos - dz * sin, arm_pivot.z + dy * sin + dz * cos);
        let centre = Vec3::new(arm_origin.x * RAISED_IN, end.y - 0.02, end.z - RAISED_FRONT - 0.05);
        // Boss to the front (-Z): turn the mesh half a turn about y.
        Mat4::from_translation(centre) * Mat4::from_rotation_y(std::f32::consts::PI)
    } else {
        let centre = Vec3::new(arm_origin.x + s * LOWERED_OUT, arm_origin.y - 0.10, arm_origin.z);
        // Boss outward: +Z turned to +X for the right hand, to -X for the left.
        Mat4::from_translation(centre) * Mat4::from_rotation_y(s * std::f32::consts::FRAC_PI_2)
    }
}

/// Apply `m` to a vertex list: positions fully, normals by the rotation.
pub fn transform_vertices(verts: &mut [Vertex], m: &Mat4) {
    let rot = Mat4::from_mat3(glam::Mat3::from_mat4(*m));
    for v in verts {
        v.position = m.transform_point3(Vec3::from(v.position)).to_array();
        v.normal = rot.transform_vector3(Vec3::from(v.normal)).normalize_or_zero().to_array();
    }
}

fn centre_of(verts: &[Vertex]) -> Vec3 {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for v in verts {
        let p = Vec3::from(v.position);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo + hi) * 0.5
}

/// NDC x of a view-space point.
fn ndc_x(p: Vec3, aspect: f32) -> f32 {
    let tan_half = (crate::viewmodel::VIEWMODEL_FOV_Y.to_radians() * 0.5).tan();
    p.x / (-p.z * tan_half * aspect)
}

fn ndc_y(p: Vec3) -> f32 {
    let tan_half = (crate::viewmodel::VIEWMODEL_FOV_Y.to_radians() * 0.5).tan();
    p.y / (-p.z * tan_half)
}

/// Raise the first-person shield: `shield` (view space, as `build_viewmodel_vertices`
/// leaves it) turns a little toward the centre and leans in, then it and the `arm`
/// holding it move toward the middle of the view and up until the board's edge
/// nearest the centre is [`CENTRE_CLEAR_NDC`] from it and its middle is at
/// [`RAISED_CENTRE_Y_NDC`]. `t` (0..=1) is how far the raise has come: the rotation
/// and the move are scaled by it, so the pose eases in and out with the count of
/// raised ticks.
pub fn raise_first_person(arm: &mut [Vertex], shield: &mut [Vertex], side: HandSide, aspect: f32, t: f32) {
    if shield.is_empty() || t <= 0.0 {
        return;
    }
    let t = t.min(1.0);
    let s = side.sign();
    let turn = |t: f32, c: Vec3| {
        Mat4::from_translation(c)
            * Mat4::from_rotation_y(-s * RAISED_YAW_DEG.to_radians() * t)
            * Mat4::from_rotation_z(s * RAISED_ROLL_DEG.to_radians() * t)
            * Mat4::from_translation(-c)
    };
    let c0 = centre_of(shield);
    // The full pose's shift (t = 1), found on a copy: rotate, then move the
    // board's inner edge to the clear distance and its middle to the target height.
    let mut full: Vec<Vertex> = shield.to_vec();
    transform_vertices(&mut full, &turn(1.0, c0));
    let mut shift = Vec3::ZERO;
    for _ in 0..2 {
        let moved: Vec<Vec3> = full.iter().map(|v| Vec3::from(v.position) + shift).collect();
        let inner = moved.iter().map(|p| s * ndc_x(*p, aspect)).fold(f32::INFINITY, f32::min);
        let depth = moved.iter().map(|p| -p.z).sum::<f32>() / moved.len() as f32;
        let tan_half = (crate::viewmodel::VIEWMODEL_FOV_Y.to_radians() * 0.5).tan();
        shift.x += s * (CENTRE_CLEAR_NDC - inner) * tan_half * aspect * depth;
        let mid = centre_of(&full) + shift;
        shift.y += (RAISED_CENTRE_Y_NDC - ndc_y(mid)) * tan_half * depth;
    }
    // How much taller the full pose must be to reach the minimum height on screen.
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for v in &full {
        let y = ndc_y(Vec3::from(v.position) + shift);
        lo = lo.min(y);
        hi = hi.max(y);
    }
    let stretch = (RAISED_MIN_HEIGHT_NDC / (hi - lo).max(1e-4)).clamp(1.0, RAISED_MAX_STRETCH);
    transform_vertices(shield, &(Mat4::from_translation(shift * t) * turn(t, c0)));
    let (pivot_y, k) = (centre_of(shield).y, 1.0 + (stretch - 1.0) * t);
    for v in shield.iter_mut() {
        v.position[1] = pivot_y + (v.position[1] - pivot_y) * k;
    }
    transform_vertices(arm, &Mat4::from_translation(shift * t));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm() -> (Vec3, Vec3, Vec3) {
        // The classic left arm (`entity_model::PLAYER_MODEL`).
        (Vec3::new(-0.375, 1.05, 0.0), Vec3::new(0.25, 0.7, 0.25), Vec3::new(-0.375, 1.4, 0.0))
    }

    #[test]
    fn a_lowered_shield_is_outside_the_arm_a_raised_one_in_front_of_the_hand() {
        let (o, sz, pv) = arm();
        let lowered = avatar_transform(o, sz, pv, HandSide::Left, false).transform_point3(Vec3::ZERO);
        assert!(lowered.x < o.x - 0.12, "the left arm's shield is on its outside: {lowered:?}");
        assert!((lowered.z - o.z).abs() < 1e-4, "level with the arm front to back");
        let right = avatar_transform(Vec3::new(0.375, 1.05, 0.0), sz, Vec3::new(0.375, 1.4, 0.0), HandSide::Right, false)
            .transform_point3(Vec3::ZERO);
        assert!(right.x > 0.375 + 0.12, "the right arm's is on its outside: {right:?}");
        let raised = avatar_transform(o, sz, pv, HandSide::Left, true).transform_point3(Vec3::ZERO);
        assert!(raised.z < -0.5, "raised, the board is well in front (the front is -Z): {raised:?}");
        assert!(raised.y > 0.9 && raised.y < 1.4, "at chest height: {raised:?}");
        assert!(raised.x.abs() < o.x.abs(), "toward the middle of the body");
    }

    #[test]
    fn the_boss_faces_out_lowered_and_forward_raised() {
        let (o, sz, pv) = arm();
        let boss = Vec3::Z; // the mesh's boss face
        let lowered_left = avatar_transform(o, sz, pv, HandSide::Left, false).transform_vector3(boss);
        assert!(lowered_left.x < -0.99, "left, lowered: boss to -X ({lowered_left:?})");
        let lowered_right = avatar_transform(Vec3::new(0.375, 1.05, 0.0), sz, Vec3::new(0.375, 1.4, 0.0), HandSide::Right, false)
            .transform_vector3(boss);
        assert!(lowered_right.x > 0.99, "right, lowered: boss to +X ({lowered_right:?})");
        let raised = avatar_transform(o, sz, pv, HandSide::Left, true).transform_vector3(boss);
        assert!(raised.z < -0.99, "raised: boss to the front, -Z ({raised:?})");
    }
}

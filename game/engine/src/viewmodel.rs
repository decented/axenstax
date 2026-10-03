//! First-person held-tool viewmodel (Phase 7).
//!
//! Renders the LOCAL player's own right arm + equipped item, lower-right of
//! the screen, bobbing while walking and swinging on mine/place. This is the
//! player seeing their own tool in hand — the counterpart to the remote-player
//! avatars (Phase 6), which reuse the same `player_model()` right arm and the
//! same `held_item_model::held_item_mesh` so the in-hand item always agrees
//! with what other players see.
//!
//! Geometry is assembled in the right arm's LOCAL/model space, then
//! transformed by [`viewmodel_transform`] straight into VIEW space (x right,
//! y up, -z forward) on the CPU. The render pass then draws it with an
//! identity view matrix and a narrow-FOV perspective projection, so the world
//! camera's yaw/pitch never touch the viewmodel — it stays glued to the
//! lower-right of the screen.

use glam::{Mat4, Vec3};

use crate::block::BlockRegistry;
use crate::entity_model::{self, PartPose};
use crate::mesh::Vertex;
use crate::protocol::ItemRef;

/// Index of the RIGHT ARM `ModelPart` in `entity_model::player_model()`.
/// (head, body, left arm, right arm, left leg, right leg.)
pub const RIGHT_ARM_INDEX: usize = 3;

/// Walk-bob amplitude (view-space units). Small so it reads as a gentle sway,
/// not a seasick lurch. Visual feel unverified — tune in playtest.
pub const BOB_AMP: f32 = 0.03;

/// Kill-switch for the whole viewmodel draw. Flip to `false` to disable.
pub const SHOW_VIEWMODEL: bool = true;

/// Narrow vertical FOV (degrees) for the viewmodel projection. Narrower than
/// the world camera (70°) so the arm/tool don't fish-eye at the screen edge.
pub const VIEWMODEL_FOV_Y: f32 = 55.0;

/// Model→view-space transform placing the viewmodel lower-right, just in front
/// of the near plane, with `bob` and `swing` baked in.
///
/// - `bob`: a pre-computed sinusoidal offset (caller passes
///   `(walk_phase).sin() * BOB_AMP`); applied to y (and a quarter of it to x)
///   so the hand sways as the player walks.
///   (Swing is applied separately — about the shoulder — in
///   [`build_viewmodel_vertices`], not here.)
///
/// View-space convention: +x right, +y up, -z forward. The base translation
/// `(+0.58, -0.60, -0.9)` sits the model lower-right and ~0.9 units in front of
/// the camera (inside the world far plane, beyond the 0.1 near plane). A small
/// base rotation angles the arm/tool naturally toward the centre.
pub fn viewmodel_transform(bob: f32) -> Mat4 {
    // Base slot: lower-right, in front of the camera. #15 — the previous
    // (+0.45, -0.32) put the arm's shoulder near screen-centre, so the whole
    // upper arm read as "too far forward / in front of the face". Dropping y to
    // -0.60 and pushing x to +0.58 tucks the shoulder off-screen so only the
    // hand/forearm peeks from the lower-right corner, the way a held tool
    // should. (-0.32 itself replaced an even-lower -0.5 that hid the fist;
    // -0.60 stays above that floor — the held item still lands on-screen.)
    let base_offset = Vec3::new(0.58 + bob * 0.25, -0.60 + bob, -0.9);
    let translate = Mat4::from_translation(base_offset);

    // Fixed tilt so the arm/tool reads as hanging at the side and angling in
    // toward the crosshair rather than standing bolt upright. #15 bumped these
    // from 12°/-10° to ~22° yaw (toward centre) + ~16° pitch back.
    let base_rot = Mat4::from_rotation_y(22.0_f32.to_radians())
        * Mat4::from_rotation_x(-16.0_f32.to_radians());

    translate * base_rot
}

/// Build the LOCAL player's viewmodel mesh in VIEW space, split by render path.
///
/// Returns `(arm_skin_verts, held_verts)`:
/// - **`arm_skin_verts`** — the right-arm part box-unwrapped onto the 64x64
///   skin atlas: the solid **base** layer (`base_faces()`, inflate 0.0) plus the
///   **sleeve overlay** layer (`overlay_faces()`, inflate `0.03` so it sits just
///   outside the base and the alpha-cutout shader shows the arm through
///   transparent overlay pixels without z-fighting). Rendered through the
///   renderer's `avatar_pipeline` against the dedicated skin texture — so an
///   uploaded skin (Phase 2) shows on the player's OWN hand, matching what
///   remote peers see via `build_player_avatar_vertices`.
/// - **`held_verts`** — the equipped item at the hand-end, block/atlas-textured.
///   Rendered through the ordinary `entity_pipeline` (block texture array)
///   because it is a world block, not part of the skin. Empty when no item.
///
/// BOTH meshes get the SAME shoulder swing then [`viewmodel_transform`] (bob),
/// so the arm and the item arc together and stay glued lower-right. Vertices are
/// ready to render with an identity view matrix and a narrow-FOV projection (see
/// the viewmodel render pass), which draws the arm and the item in one
/// depth-cleared pass so they self-occlude correctly.
pub fn build_viewmodel_vertices(
    held: ItemRef,
    bob: f32,
    swing: f32,
    skin_layer: u32,
    registry: &BlockRegistry,
    arm_model: crate::skin_uv::ArmModel,
) -> (Vec<Vertex>, Vec<Vertex>) {
    // The hand you see is YOUR hand: a slim ("Alex") skin gets the 3-px arm and
    // the 3-px sleeve rects here too, not just on the third-person body.
    let arm = &entity_model::player_model_for(arm_model)[RIGHT_ARM_INDEX];

    // Right-arm part in local/model space, centred on its own origin so the
    // whole thing sits at the view-space slot rather than the avatar's body
    // offset. Built UN-swung: the whole arm+item assembly is swung once about
    // the shoulder below. (The old code swung the arm here via arm_override AND
    // again in viewmodel_transform about the arm's CENTRE — a double rotation
    // with the wrong pivot, which read as the arm pivoting from the wrong end.)
    //
    // Skin path: base body layer + sleeve overlay, the same two boxes (and the
    // same OVERLAY_INFLATE) the remote-avatar builder emits per part.
    let pose = PartPose {
        walk_swing: 0.0,
        head_pitch: 0.0,
        arm_override: Some(0.0),
        ..Default::default()
    };
    let base_uv = crate::skin_uv::base_faces(arm_model);
    let overlay_uv = crate::skin_uv::overlay_faces(arm_model);
    let mut arm_verts =
        entity_model::build_skin_part_local(arm, &base_uv[RIGHT_ARM_INDEX], 0.0, pose, skin_layer);
    arm_verts.extend(entity_model::build_skin_part_local(
        arm,
        &overlay_uv[RIGHT_ARM_INDEX],
        crate::entity_model::RIGHT_ARM_OVERLAY_INFLATE,
        pose,
        skin_layer,
    ));

    // Equipped item, anchored at the hand-end of the arm (bottom of the arm
    // cuboid). The arm part is recentred on its origin, so the hand sits at
    // (0, -size.y/2, 0) in the recentred local frame. Built into its OWN vec so
    // it can render through the block-texture pipeline, not the skin one.
    let mut held_verts = Vec::new();
    if !matches!(held, ItemRef::Empty) {
        let hand_anchor = Vec3::new(0.0, -arm.size.y * 0.5, 0.0);
        let mut item_verts = crate::held_item_model::held_item_mesh(held, registry);
        for v in &mut item_verts {
            let p = Vec3::from(v.position) + hand_anchor;
            v.position = p.to_array();
        }
        held_verts.extend(item_verts);
    }

    // Swing once, about the shoulder (top of the recentred arm, +size.y/2), so
    // the hand + tool arc down-and-forward together and pivot at the shoulder
    // rather than the arm's centre.
    let shoulder = Vec3::new(0.0, arm.size.y * 0.5, 0.0);
    let swing_about_shoulder = Mat4::from_translation(shoulder)
        * Mat4::from_rotation_x(swing)
        * Mat4::from_translation(-shoulder);

    // Transform the whole assembly into view space. Positions go through the
    // full matrix; normals through its rotation only. Viewmodel lighting is
    // full-bright (set per-vertex by the part/item builders) so we don't need
    // accurate normals for shading — but we still rotate them to keep the data
    // sane for any future lit-viewmodel pass. The SAME matrices apply to the arm
    // and the held item so they move together.
    let xform = viewmodel_transform(bob) * swing_about_shoulder;
    // correct for rotation-only xform; if scale is added, use transpose(inverse) for normals.
    let normal_rot = Mat4::from_mat3(glam::Mat3::from_mat4(xform));
    for v in arm_verts.iter_mut().chain(held_verts.iter_mut()) {
        let p = xform.transform_point3(Vec3::from(v.position));
        v.position = p.to_array();
        let n = normal_rot.transform_vector3(Vec3::from(v.normal)).normalize_or_zero();
        v.normal = n.to_array();
    }

    (arm_verts, held_verts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_transform_places_lower_right_and_in_front() {
        // The model origin (0,0,0) should map to +x (right), -y (down),
        // -z (in front of the camera) at rest.
        let m = viewmodel_transform(0.0);
        let origin = m.transform_point3(Vec3::ZERO);
        assert!(origin.x > 0.3, "viewmodel should sit well right of centre: {origin:?}");
        // #15 — anchor lowered so only the hand/forearm shows; lock it clearly
        // below centre (not just <0) to guard against drifting back up.
        assert!(origin.y < -0.4, "viewmodel should sit low, off the face: {origin:?}");
        assert!(origin.z < 0.0, "viewmodel should sit in front of camera: {origin:?}");
    }

    #[test]
    fn swing_changes_the_built_vertices() {
        // Swing now lives in build_viewmodel_vertices (about the shoulder), not
        // in viewmodel_transform. A non-zero swing must move the geometry. Both
        // the arm-skin mesh (.0) and the held-item mesh (.1) get the same swing.
        let reg = BlockRegistry::new();
        let (rest_arm, rest_held) = build_viewmodel_vertices(ItemRef::Tool(2), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic);
        let (swung_arm, swung_held) = build_viewmodel_vertices(ItemRef::Tool(2), 0.0, 0.6, 0, &reg, crate::skin_uv::ArmModel::Classic);
        assert_eq!(rest_arm.len(), swung_arm.len(), "swing must not change arm vertex count");
        assert_eq!(rest_held.len(), swung_held.len(), "swing must not change held vertex count");
        let arm_moved = rest_arm.iter().zip(&swung_arm).any(|(a, b)| a.position != b.position);
        let held_moved = rest_held.iter().zip(&swung_held).any(|(a, b)| a.position != b.position);
        assert!(arm_moved, "a non-zero swing must move the arm geometry");
        assert!(held_moved, "a non-zero swing must move the held-item geometry");
    }

    #[test]
    fn bob_shifts_the_origin_vertically() {
        let down = viewmodel_transform(-BOB_AMP).transform_point3(Vec3::ZERO);
        let up = viewmodel_transform(BOB_AMP).transform_point3(Vec3::ZERO);
        assert!(up.y > down.y, "positive bob should raise the viewmodel");
    }

    #[test]
    fn empty_hand_builds_just_the_arm() {
        let reg = BlockRegistry::new();
        let (arm_empty, held_empty) = build_viewmodel_vertices(ItemRef::Empty, 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic);
        let (arm_block, held_block) = build_viewmodel_vertices(ItemRef::Block(1), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic);
        // Arm = base + sleeve overlay = 2 boxes × 36 = 72 verts, in .0; it is
        // identical whether or not an item is held (the item never touches .0).
        assert_eq!(arm_empty.len(), 72, "arm = base + overlay = 72 verts");
        assert_eq!(
            arm_empty.iter().map(|v| v.position).collect::<Vec<_>>(),
            arm_block.iter().map(|v| v.position).collect::<Vec<_>>(),
            "the held item must not change the arm-skin mesh",
        );
        // The held block is a 36-vert cuboid, and lands ONLY in .1.
        assert!(held_empty.is_empty(), "empty hand -> no held mesh");
        assert_eq!(held_block.len(), 36, "held block is a 36-vert cuboid in .1");
    }

    #[test]
    fn slim_hand_is_narrower_than_the_classic_one() {
        // Your own hand follows your skin's Arms setting. Same vertex count
        // (same two boxes), narrower on the model's x axis before the view
        // transform tilts it — measured as the spread of the arm's widest pair
        // of vertices, which the 22° yaw preserves ordering on.
        let reg = BlockRegistry::new();
        let width = |arm: crate::skin_uv::ArmModel| {
            let (v, _) = build_viewmodel_vertices(ItemRef::Empty, 0.0, 0.0, 0, &reg, arm);
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for p in v.iter().map(|v| v.position) {
                lo = lo.min(p[0]);
                hi = hi.max(p[0]);
            }
            (v.len(), hi - lo)
        };
        let (nc, wc) = width(crate::skin_uv::ArmModel::Classic);
        let (ns, ws) = width(crate::skin_uv::ArmModel::Slim);
        assert_eq!(nc, ns, "same geometry, only narrower");
        assert!(ws < wc, "slim hand ({ws}) must be narrower than classic ({wc})");
    }

    #[test]
    fn vertices_land_in_front_of_camera() {
        // Every viewmodel vertex — arm (.0) AND held item (.1) — must sit in
        // front of the camera (-z) so it's not clipped behind the near plane /
        // culled to the back.
        let reg = BlockRegistry::new();
        let (arm, held) = build_viewmodel_vertices(ItemRef::Tool(2), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(!arm.is_empty());
        assert!(!held.is_empty());
        for v in arm.iter().chain(&held) {
            assert!(v.position[2] < 0.0, "vertex behind camera: {:?}", v.position);
        }
    }
}

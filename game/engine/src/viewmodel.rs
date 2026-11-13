//! First-person held-tool viewmodel (Phase 7).
//!
//! Renders the LOCAL player's own right arm + equipped item in the lower right
//! of the screen, bobbing while walking and swinging on mine/place. This is the
//! player seeing their own tool in hand — the counterpart to the remote-player
//! avatars (Phase 6), which reuse the same `player_model()` right arm and the
//! same `item_model::held_item_mesh` so the in-hand item always agrees
//! with what other players see.
//!
//! Geometry is assembled in the right arm's LOCAL/model space, then
//! transformed by [`arm_transform`] straight into VIEW space (x right,
//! y up, -z forward) on the CPU. The render pass then draws it with an
//! identity view matrix and a narrow-FOV perspective projection
//! ([`viewmodel_projection`]), so the world camera's yaw/pitch never touch the
//! viewmodel — it stays glued to the lower right of the screen.
//!
//! # The layout (Minecraft-like; Spec 03 "first-person viewmodel")
//!
//! The arm points away from the player, in toward the centre, like Minecraft's
//! first-person arm. The fist is a fixed place in the lower-right quadrant
//! ([`FIST_NDC`]) and the held item stands upright above it, leaning a little
//! in. Seen end-on, the arm's shoulder end is near the camera and far below and
//! right of the fist, so it is off the screen.
//!
//! History, and the guard that keeps it. #15 moved the first slot (+0.45, -0.32)
//! lower-right because the shoulder sat near the screen centre and the upper arm
//! read as "too far forward / in front of the face". The slot it chose
//! (+0.58, -0.60) went too far: the hand end fell below the bottom edge and a
//! held item was never on screen at rest. This layout keeps #15's rule (no
//! shoulder in view) and puts the fist and the item on screen. The tests in
//! `layout_tests` project the vertices through [`viewmodel_projection`] and
//! check both, at 16:9, 16:10, 4:3 and phone portrait.
//!
//! The slot depends on the viewport's aspect ratio ([`slot_depth`]) and on the
//! hand side ([`HandSide`]; the off-hand is the same layout mirrored in x, not
//! drawn yet).

use glam::{Mat4, Vec3};

use crate::block::BlockRegistry;
use crate::entity_model::{self, PartPose};
use crate::item_model;
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

/// The viewmodel pass's projection for a viewport of `aspect` (width / height):
/// the narrow [`VIEWMODEL_FOV_Y`] perspective the renderer multiplies the
/// (identity-view) viewmodel vertices by. One function, so the renderer and
/// the layout tests project through exactly the same matrix.
pub fn viewmodel_projection(aspect: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective(
        VIEWMODEL_FOV_Y.to_radians(),
        aspect.max(0.0001),
        0.01,
        10.0,
    )
}

/// Which hand a viewmodel is. Only the right hand is drawn today; Spec 05 has an
/// off-hand that is not built. Its slot is this layout mirrored in x, so every
/// part of the slot below takes the side instead of assuming the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandSide {
    Right,
    // BRIDGE: the off-hand viewmodel (Spec 05's off-hand, not built) — remove the
    // allow when a left hand is drawn. Only tests build a left slot today.
    #[allow(dead_code)]
    Left,
}

impl HandSide {
    /// +1 for the right hand, -1 for the left: the x mirror.
    pub fn sign(self) -> f32 {
        match self {
            HandSide::Right => 1.0,
            HandSide::Left => -1.0,
        }
    }
}

/// The aspect the slot is laid out for (16:9). A narrower viewport pushes the
/// slot deeper, see [`slot_depth`].
pub const REF_ASPECT: f32 = 16.0 / 9.0;
/// Where the fist (the middle of the arm's hand end) sits at [`REF_ASPECT`], in
/// NDC (x right, y up, -1..1 across the viewport): well into the lower-right
/// quadrant, clear of the crosshair, with the forearm running off the bottom
/// and right edges.
pub const FIST_NDC: (f32, f32) = (0.57, -0.63);
/// View-space depth of the fist at [`REF_ASPECT`].
pub const SLOT_DEPTH: f32 = 1.57;
/// How fast the slot recedes as the viewport narrows (see [`slot_depth`]).
pub const SLOT_DEPTH_ASPECT_EXPONENT: f32 = 0.58;
/// The arm points away from the player, like Minecraft's first-person arm: this
/// far in toward the centre (yaw, degrees) ...
pub const ARM_YAW_DEG: f32 = 35.0;
/// ... and pitched so the hand end is almost straight ahead of the shoulder
/// (degrees about x; 90 points the arm along -z). Seen end-on, the shoulder is
/// near the camera and far below and right of the fist, so it is off screen.
pub const ARM_PITCH_DEG: f32 = 91.0;
/// How far the held item leans in toward the centre from upright (degrees,
/// about the view z axis).
pub const ITEM_LEAN_DEG: f32 = 12.0;

/// View-space depth of the slot for a viewport of `aspect` (width / height).
///
/// The fist stays at the same place on screen at every aspect (the slot is
/// placed from [`FIST_NDC`]), but the model's size does not change with the
/// aspect, so on a narrow viewport the arm and item would be too wide for it.
/// Below [`REF_ASPECT`] the slot moves away from the camera, by
/// `(REF_ASPECT / aspect)^SLOT_DEPTH_ASPECT_EXPONENT`: less than the full
/// `1 / aspect` that would hold the width on screen, so a phone in portrait
/// still gets an item a sixth of the screen tall rather than a speck.
pub fn slot_depth(aspect: f32) -> f32 {
    SLOT_DEPTH * (REF_ASPECT / aspect.max(0.0001)).max(1.0).powf(SLOT_DEPTH_ASPECT_EXPONENT)
}

/// The arm's rotation in view space: yawed in toward the centre (mirrored for
/// the left hand) and pitched forward. Rotating about y and z negates under an
/// x mirror, rotating about x does not.
fn arm_rotation(side: HandSide) -> Mat4 {
    Mat4::from_rotation_y(side.sign() * ARM_YAW_DEG.to_radians())
        * Mat4::from_rotation_x(ARM_PITCH_DEG.to_radians())
}

/// Half the length of the arm box, from its centre to the hand end.
fn arm_half_length() -> f32 {
    entity_model::player_model()[RIGHT_ARM_INDEX].size.y * 0.5
}

/// Model-to-view slot of the arm at rest: the arm box recentred on its own
/// origin, placed so its hand end lands at [`FIST_NDC`] (mirrored for the left
/// hand) at [`slot_depth`] for `aspect`, pointing away from the player.
///
/// View space is +x right, +y up, -z forward. `bob` is the walk-bob offset
/// (`(walk_phase).sin() * BOB_AMP`), applied to y and a quarter of it to x.
/// The swing is applied separately, about the shoulder, in [`arm_transform`].
pub fn arm_slot(aspect: f32, side: HandSide, bob: f32) -> Mat4 {
    let depth = slot_depth(aspect);
    let half_height = (VIEWMODEL_FOV_Y.to_radians() * 0.5).tan() * depth;
    let fist = Vec3::new(
        side.sign() * FIST_NDC.0 * half_height * aspect + bob * 0.25,
        FIST_NDC.1 * half_height + bob,
        -depth,
    );
    let rotation = arm_rotation(side);
    // The hand end sits at (0, -half length, 0) in the recentred arm frame.
    let hand_end = rotation.transform_point3(Vec3::new(0.0, -arm_half_length(), 0.0));
    Mat4::from_translation(fist - hand_end) * rotation
}

/// Model-to-view transform of the arm with the swing: [`arm_slot`] after a
/// rotation about the shoulder (the top of the recentred arm), so the hand and
/// the tool arc together and pivot at the shoulder. Swung toward the centre
/// and up the screen: at the swing peak the hand is near the middle of the
/// lower half and the item is still in view.
pub fn arm_transform(aspect: f32, side: HandSide, bob: f32, swing: f32) -> Mat4 {
    let shoulder = Vec3::new(0.0, arm_half_length(), 0.0);
    arm_slot(aspect, side, bob)
        * Mat4::from_translation(shoulder)
        * Mat4::from_rotation_x(swing)
        * Mat4::from_translation(-shoulder)
}

/// How far a held item's middle is lifted above the fist in the first-person
/// view, so the grip is in the hand and the rest of the item stands above it
/// (view space, upright). `item_model` builds a tool centred on the hand and a
/// cube centred on the hand, for the third-person arm that hangs down; held
/// upright in front of the player the same thing would sink into the fist.
fn item_lift(held: ItemRef, registry: &BlockRegistry) -> f32 {
    match held {
        ItemRef::Tool(_) => 0.2,
        ItemRef::Block(id)
            if matches!(item_model::block_look(id, registry), item_model::ItemLook::Sprite { .. }) =>
        {
            0.2
        }
        _ => 0.15,
    }
}

/// Model-space placement of the held item for the first-person view: at the
/// hand end, turned back by the arm's own rotation so it stands upright in the
/// view (not along the arm, as in the third-person hand), then leaned in toward
/// the centre by [`ITEM_LEAN_DEG`], with [`item_lift`] above the fist. A
/// first-person-only adjustment: `item_model` and the third-person hand are
/// untouched.
fn held_item_placement(held: ItemRef, registry: &BlockRegistry, side: HandSide) -> Mat4 {
    let hand_anchor = Vec3::new(0.0, -arm_half_length(), 0.0);
    let upright = arm_rotation(side).inverse();
    let lean = Mat4::from_rotation_z(side.sign() * ITEM_LEAN_DEG.to_radians());
    Mat4::from_translation(hand_anchor)
        * upright
        * lean
        * Mat4::from_translation(Vec3::new(0.0, item_lift(held, registry), 0.0))
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
/// BOTH meshes get the SAME shoulder swing then the slot (bob), so the arm and
/// the item arc together. `aspect` is the viewport's width / height: the slot
/// depends on it (see [`slot_depth`]). Vertices are ready to render with an
/// identity view matrix and the [`viewmodel_projection`], which draws the arm and
/// the item in one depth-cleared pass so they self-occlude correctly.
pub fn build_viewmodel_vertices(
    held: ItemRef,
    bob: f32,
    swing: f32,
    skin_layer: u32,
    registry: &BlockRegistry,
    arm_model: crate::skin_uv::ArmModel,
    aspect: f32,
) -> (Vec<Vertex>, Vec<Vertex>) {
    let side = HandSide::Right;
    // The hand you see is YOUR hand: a slim ("Alex") skin gets the 3-px arm and
    // the 3-px sleeve rects here too, not just on the third-person body.
    let arm = &entity_model::player_model_for(arm_model)[RIGHT_ARM_INDEX];

    // Right-arm part in local/model space, centred on its own origin so the
    // whole thing sits at the view-space slot rather than the avatar's body
    // offset. Built UN-swung: the whole arm+item assembly is swung once about
    // the shoulder in `arm_transform`.
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

    // Equipped item, placed at the hand-end of the arm by `held_item_placement`.
    // Built into its OWN vec so it can render through the block-texture
    // pipeline, not the skin one.
    let mut held_verts = Vec::new();
    if !matches!(held, ItemRef::Empty) {
        let placement = held_item_placement(held, registry, side);
        // `item_model` puts a held sprite beside the fist (HELD_SPRITE_OFFSET) so
        // its flame clears the hanging third-person arm. Held upright in front of
        // the player that offset would push it off to the side, so take it out.
        let is_sprite = matches!(held, ItemRef::Block(id)
            if matches!(item_model::block_look(id, registry), item_model::ItemLook::Sprite { .. }));
        let mut item_verts = item_model::held_item_mesh(held, registry);
        for v in &mut item_verts {
            let mut p = Vec3::from(v.position);
            if is_sprite {
                p -= item_model::HELD_SPRITE_OFFSET;
            }
            v.position = placement.transform_point3(p).to_array();
            v.normal = placement.transform_vector3(Vec3::from(v.normal)).normalize_or_zero().to_array();
        }
        held_verts.extend(item_verts);
    }

    // Transform the whole assembly into view space. Positions go through the
    // full matrix; normals through its rotation only. The caller lights both
    // with the eye cell's (block, sky) channels under the shared light rule
    // (Spec 03 §4.7), which shades by these view-space normals: the sun term
    // against the viewmodel camera's fixed sun, the block term by face. The
    // SAME matrix applies to the arm and the held item so they move together.
    let xform = arm_transform(aspect, side, bob, swing);
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
        // The model origin (0,0,0), the middle of the arm, should map to +x
        // (right), -y (down), -z (in front of the camera) at rest, at any aspect.
        for aspect in [16.0 / 9.0, 4.0 / 3.0, 9.0 / 16.0] {
            let origin = arm_slot(aspect, HandSide::Right, 0.0).transform_point3(Vec3::ZERO);
            assert!(origin.x > 0.0, "viewmodel should sit right of centre @{aspect}: {origin:?}");
            assert!(origin.y < 0.0, "viewmodel should sit low @{aspect}: {origin:?}");
            assert!(origin.z < 0.0, "viewmodel should sit in front of camera @{aspect}: {origin:?}");
        }
    }

    #[test]
    fn the_slot_gets_deeper_as_the_viewport_narrows() {
        let depth = |a: f32| slot_depth(a);
        assert_eq!(depth(REF_ASPECT), SLOT_DEPTH);
        assert_eq!(depth(21.0 / 9.0), SLOT_DEPTH, "wider than 16:9 keeps the reference depth");
        assert!(depth(16.0 / 10.0) > depth(16.0 / 9.0));
        assert!(depth(4.0 / 3.0) > depth(16.0 / 10.0));
        assert!(depth(9.0 / 16.0) > depth(4.0 / 3.0));
    }

    #[test]
    fn the_fist_lands_on_its_ndc_target_at_every_aspect() {
        let half = arm_half_length();
        for aspect in [16.0 / 9.0, 16.0 / 10.0, 4.0 / 3.0, 9.0 / 16.0] {
            let fist = arm_slot(aspect, HandSide::Right, 0.0).transform_point3(Vec3::new(0.0, -half, 0.0));
            let c = viewmodel_projection(aspect) * fist.extend(1.0);
            let (x, y) = (c.x / c.w, c.y / c.w);
            assert!((x - FIST_NDC.0).abs() < 1e-3 && (y - FIST_NDC.1).abs() < 1e-3, "@{aspect}: fist at ({x}, {y})");
        }
    }

    #[test]
    fn the_left_hand_is_the_right_hand_mirrored_in_x() {
        // The off-hand's slot: every point of the arm (and of an upright item
        // placed on it) is the right hand's with x negated.
        let reg = BlockRegistry::new();
        for aspect in [16.0 / 9.0, 9.0 / 16.0] {
            let r = arm_transform(aspect, HandSide::Right, 0.0, 0.4);
            let l = arm_transform(aspect, HandSide::Left, 0.0, 0.4);
            for p in [Vec3::new(0.1, 0.3, -0.05), Vec3::new(-0.2, -0.35, 0.1), Vec3::ZERO] {
                let (a, b) = (r.transform_point3(p), l.transform_point3(Vec3::new(-p.x, p.y, p.z)));
                assert!((a - Vec3::new(-b.x, b.y, b.z)).length() < 1e-4, "{p:?}: {a:?} vs {b:?}");
            }
            for held in [ItemRef::Tool(2), ItemRef::Block(crate::block::TORCH)] {
                let (pr, pl) = (held_item_placement(held, &reg, HandSide::Right), held_item_placement(held, &reg, HandSide::Left));
                let p = Vec3::new(0.15, 0.2, 0.03);
                let (a, b) = (pr.transform_point3(p), pl.transform_point3(Vec3::new(-p.x, p.y, p.z)));
                assert!((a - Vec3::new(-b.x, b.y, b.z)).length() < 1e-4, "{held:?}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn swing_changes_the_built_vertices() {
        // Swing now lives in build_viewmodel_vertices (about the shoulder), not
        // in viewmodel_transform. A non-zero swing must move the geometry. Both
        // the arm-skin mesh (.0) and the held-item mesh (.1) get the same swing.
        let reg = BlockRegistry::new();
        let (rest_arm, rest_held) = build_viewmodel_vertices(ItemRef::Tool(2), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
        let (swung_arm, swung_held) = build_viewmodel_vertices(ItemRef::Tool(2), 0.0, 0.6, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
        assert_eq!(rest_arm.len(), swung_arm.len(), "swing must not change arm vertex count");
        assert_eq!(rest_held.len(), swung_held.len(), "swing must not change held vertex count");
        let arm_moved = rest_arm.iter().zip(&swung_arm).any(|(a, b)| a.position != b.position);
        let held_moved = rest_held.iter().zip(&swung_held).any(|(a, b)| a.position != b.position);
        assert!(arm_moved, "a non-zero swing must move the arm geometry");
        assert!(held_moved, "a non-zero swing must move the held-item geometry");
    }

    #[test]
    fn bob_shifts_the_origin_vertically() {
        let down = arm_slot(16.0 / 9.0, HandSide::Right, -BOB_AMP).transform_point3(Vec3::ZERO);
        let up = arm_slot(16.0 / 9.0, HandSide::Right, BOB_AMP).transform_point3(Vec3::ZERO);
        assert!(up.y > down.y, "positive bob should raise the viewmodel");
    }

    #[test]
    fn empty_hand_builds_just_the_arm() {
        let reg = BlockRegistry::new();
        let (arm_empty, held_empty) = build_viewmodel_vertices(ItemRef::Empty, 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
        let (arm_block, held_block) = build_viewmodel_vertices(ItemRef::Block(1), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
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
            let (v, _) = build_viewmodel_vertices(ItemRef::Empty, 0.0, 0.0, 0, &reg, arm, 16.0 / 9.0);
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
        let (arm, held) = build_viewmodel_vertices(ItemRef::Tool(2), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
        assert!(!arm.is_empty());
        assert!(!held.is_empty());
        for v in arm.iter().chain(&held) {
            assert!(v.position[2] < 0.0, "vertex behind camera: {:?}", v.position);
        }
    }
}

/// Where the first-person hand and its item land on screen (spec 2026-11-13,
/// "held items visible in first person"). CPU projection tests: the vertices
/// go through [`viewmodel_projection`], the matrix the render pass uses, with an
/// identity view, at rest (bob 0, swing 0) unless a test says it is at the peak.
#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::block;
    use crate::skin_uv::ArmModel;

    /// The aspect ratios people play at: 16:9, 16:10, 4:3 and phone portrait.
    const ASPECTS: [f32; 4] = [16.0 / 9.0, 16.0 / 10.0, 4.0 / 3.0, 9.0 / 16.0];

    /// What is held, with the least height its projected box must have (as a
    /// fraction of the viewport height: the spec's 12% and, for the tall items,
    /// 15%).
    fn items() -> Vec<(&'static str, ItemRef, f32)> {
        vec![
            ("torch", ItemRef::Block(block::TORCH), 0.15),
            ("iron sword", ItemRef::Tool(2), 0.15),
            ("stone block", ItemRef::Block(block::STONE), 0.12),
            ("oak sapling", ItemRef::Block(block::SAPLING_OAK), 0.15),
        ]
    }

    /// The largest swing angle `entity_model::swing_angle` reaches.
    fn swing_peak() -> f32 {
        (0..=1000)
            .map(|i| crate::entity_model::swing_angle(i as f32 / 1000.0))
            .fold(0.0_f32, f32::max)
    }

    /// View-space point to NDC (x right, y up, both -1..1 across the viewport).
    fn ndc(p: Vec3, aspect: f32) -> glam::Vec2 {
        let c = viewmodel_projection(aspect) * p.extend(1.0);
        assert!(c.w > 0.0, "{p:?} is behind the camera");
        glam::Vec2::new(c.x / c.w, c.y / c.w)
    }

    fn view_positions(v: &[Vertex]) -> Vec<Vec3> {
        v.iter().map(|v| Vec3::from(v.position)).collect()
    }

    fn build(held: ItemRef, aspect: f32, swing: f32, arm: ArmModel) -> (Vec<Vec3>, Vec<Vec3>) {
        let (a, h) = build_viewmodel_vertices(held, 0.0, swing, 0, &BlockRegistry::new(), arm, aspect);
        (view_positions(&a), view_positions(&h))
    }

    /// The four corners of the arm box's hand end and of its shoulder end, in
    /// view space.
    fn arm_ends(aspect: f32, swing: f32, arm: ArmModel) -> ([Vec3; 4], [Vec3; 4]) {
        let size = entity_model::player_model_for(arm)[RIGHT_ARM_INDEX].size;
        let m = arm_transform(aspect, HandSide::Right, 0.0, swing);
        let ends = |y: f32| {
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .map(|(sx, sz)| m.transform_point3(Vec3::new(sx * size.x * 0.5, y, sz * size.z * 0.5)))
        };
        (ends(-size.y * 0.5), ends(size.y * 0.5))
    }

    /// NDC bounding box of a point set: (min, max).
    fn bbox(pts: &[Vec3], aspect: f32) -> (glam::Vec2, glam::Vec2) {
        let n: Vec<_> = pts.iter().map(|p| ndc(*p, aspect)).collect();
        let lo = n.iter().fold(glam::Vec2::splat(f32::INFINITY), |a, b| a.min(*b));
        let hi = n.iter().fold(glam::Vec2::splat(f32::NEG_INFINITY), |a, b| a.max(*b));
        (lo, hi)
    }

    fn inside_viewport(p: glam::Vec2, limit: f32) -> bool {
        p.x.abs() <= limit && p.y.abs() <= limit
    }

    /// The shoulder end is out of view when every corner is beyond the right or
    /// the bottom edge.
    fn shoulder_outside(shoulder: &[Vec3; 4], aspect: f32) -> bool {
        shoulder.iter().all(|p| {
            let n = ndc(*p, aspect);
            n.x > 1.0 || n.y < -1.0
        })
    }

    /// Does any part of the triangle list (view space) cover the centre box
    /// `+-half` (NDC)? Tests every vertex and a barycentric grid across each
    /// triangle, so a large face spanning the centre is caught too.
    fn covers_centre(verts: &[Vec3], aspect: f32, half: f32) -> bool {
        const N: u32 = 24;
        for t in verts.chunks_exact(3) {
            let n = [ndc(t[0], aspect), ndc(t[1], aspect), ndc(t[2], aspect)];
            for i in 0..=N {
                for j in 0..=(N - i) {
                    let (u, v) = (i as f32 / N as f32, j as f32 / N as f32);
                    let p = n[0] * (1.0 - u - v) + n[1] * u + n[2] * v;
                    if p.x.abs() < half && p.y.abs() < half {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn describe(name: &str, aspect: f32, pts: &[Vec3]) -> String {
        let (lo, hi) = bbox(pts, aspect);
        let c = (lo + hi) * 0.5;
        format!(
            "{name} @{aspect:.2}: ndc x {:.2}..{:.2}, y {:.2}..{:.2}, height {:.0}%, centre ({:.2}, {:.2})",
            lo.x, hi.x, lo.y, hi.y, (hi.y - lo.y) * 50.0, c.x, c.y
        )
    }

    /// Every numeric the spec measures, printed for the before/after record.
    /// Always passes; run with --nocapture.
    #[test]
    fn viewmodel_layout_numbers() {
        for aspect in ASPECTS {
            let (hand, shoulder) = arm_ends(aspect, 0.0, ArmModel::Classic);
            let sh: Vec<_> = shoulder.iter().map(|p| ndc(*p, aspect)).collect();
            let hn: Vec<_> = hand.iter().map(|p| ndc(*p, aspect)).collect();
            eprintln!(
                "VMLAYOUT aspect {aspect:.2}: fist ndc {hn:.2?}  shoulder ndc {sh:.2?}  shoulder_outside={}",
                shoulder_outside(&shoulder, aspect)
            );
            for (name, held, _) in items() {
                for (label, swing) in [("rest", 0.0), ("peak", swing_peak())] {
                    let (arm, item) = build(held, aspect, swing, ArmModel::Classic);
                    eprintln!("VMLAYOUT {label} {}", describe(name, aspect, &item));
                    let _ = arm;
                }
            }
        }
    }

    // 1. Every vertex of the held item is inside the viewport.
    #[test]
    fn spec1_every_held_vertex_is_in_the_viewport() {
        for aspect in ASPECTS {
            for (name, held, _) in items() {
                let (_, item) = build(held, aspect, 0.0, ArmModel::Classic);
                assert!(item.iter().all(|p| p.z < -0.02), "{name}: vertex too close to the camera");
                let outside: Vec<_> = item
                    .iter()
                    .map(|p| ndc(*p, aspect))
                    .filter(|n| !inside_viewport(*n, 0.98))
                    .collect();
                assert!(outside.is_empty(), "{}; {} of {} vertices outside +-0.98", describe(name, aspect, &item), outside.len(), item.len());
            }
        }
    }

    // 2. Big enough, and in the lower-right quadrant.
    #[test]
    fn spec2_item_box_is_tall_enough_and_lower_right() {
        for aspect in ASPECTS {
            for (name, held, min_h) in items() {
                let (_, item) = build(held, aspect, 0.0, ArmModel::Classic);
                let (lo, hi) = bbox(&item, aspect);
                let c = (lo + hi) * 0.5;
                // The part of the box that is on screen: an item that mostly hangs
                // below the bottom edge is not "tall" in view.
                let seen_h = (hi.y.min(1.0) - lo.y.max(-1.0)).max(0.0) * 0.5;
                assert!(seen_h >= min_h, "{}: needs >= {:.0}% tall on screen, has {:.0}%", describe(name, aspect, &item), min_h * 100.0, seen_h * 100.0);
                assert!(c.x > 0.15 && c.y < -0.15, "{}: centre must be lower-right (x > 0.15, y < -0.15)", describe(name, aspect, &item));
                assert!(inside_viewport(c, 1.0), "{}: centre must be on screen", describe(name, aspect, &item));
            }
        }
    }

    // 3. The fist (the hand end of the arm box) is in the viewport.
    #[test]
    fn spec3_fist_is_in_the_viewport() {
        for aspect in ASPECTS {
            for arm in [ArmModel::Classic, ArmModel::Slim] {
                let (hand, _) = arm_ends(aspect, 0.0, arm);
                for p in hand {
                    let n = ndc(p, aspect);
                    assert!(inside_viewport(n, 1.0), "{arm:?} fist corner at ndc ({:.2}, {:.2}) @{aspect:.2} is outside the viewport", n.x, n.y);
                }
            }
        }
    }

    // 4. The shoulder end is out of the viewport (#15's guard).
    #[test]
    fn spec4_shoulder_stays_off_screen() {
        for aspect in ASPECTS {
            for arm in [ArmModel::Classic, ArmModel::Slim] {
                let (_, shoulder) = arm_ends(aspect, 0.0, arm);
                assert!(shoulder.iter().all(|p| p.z < -0.05), "{arm:?} shoulder too close to the camera: {shoulder:?}");
                let n: Vec<_> = shoulder.iter().map(|p| ndc(*p, aspect)).collect();
                assert!(shoulder_outside(&shoulder, aspect), "{arm:?} shoulder corners at ndc {n:.2?} @{aspect:.2} are not all beyond the right or bottom edge");
            }
        }
    }

    /// The guard must bite: the pre-#15 slot (+0.45, -0.32) puts the shoulder in view.
    #[test]
    fn spec4_guard_fails_for_the_pre_15_slot() {
        let old = Mat4::from_translation(Vec3::new(0.45, -0.32, -0.9))
            * Mat4::from_rotation_y(22.0_f32.to_radians())
            * Mat4::from_rotation_x(-16.0_f32.to_radians());
        let size = entity_model::player_model_for(ArmModel::Classic)[RIGHT_ARM_INDEX].size;
        let top = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(sx, sz)| old.transform_point3(Vec3::new(sx * size.x * 0.5, size.y * 0.5, sz * size.z * 0.5)));
        assert!(!shoulder_outside(&top, 16.0 / 9.0), "the old slot's shoulder is on screen, so the guard must report it");
    }

    // 5. Nothing of the arm or item covers the centre +-5% of the viewport.
    #[test]
    fn spec5_the_crosshair_stays_clear() {
        // +-5% of the viewport either side of the centre is +-0.10 in NDC.
        const HALF: f32 = 0.10;
        for aspect in ASPECTS {
            for (name, held, _) in items() {
                let (arm, item) = build(held, aspect, 0.0, ArmModel::Classic);
                assert!(!covers_centre(&arm, aspect, HALF), "arm covers the centre with {name} @{aspect:.2}");
                assert!(!covers_centre(&item, aspect, HALF), "{name} covers the centre @{aspect:.2}");
            }
            let (arm, _) = build(ItemRef::Empty, aspect, 0.0, ArmModel::Classic);
            assert!(!covers_centre(&arm, aspect, HALF), "the empty hand's arm covers the centre @{aspect:.2}");
        }
    }

    // 6. At the swing peak some of the item is still in view.
    #[test]
    fn spec6_item_stays_partly_in_view_at_the_swing_peak() {
        for aspect in ASPECTS {
            for (name, held, _) in items() {
                let (_, item) = build(held, aspect, swing_peak(), ArmModel::Classic);
                let (lo, hi) = bbox(&item, aspect);
                let overlaps = lo.x < 1.0 && hi.x > -1.0 && lo.y < 1.0 && hi.y > -1.0;
                assert!(overlaps, "{} at the swing peak is entirely outside the viewport", describe(name, aspect, &item));
            }
        }
    }
}

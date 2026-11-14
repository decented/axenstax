//! First-person held-tool viewmodel (Phase 7).
//!
//! Renders the LOCAL player's own right arm + equipped item in the lower right
//! of the screen, bobbing while walking and swinging on mine/place, and, when
//! the off-hand holds something, the left arm + that item in the lower left (the
//! same layout mirrored in x; an empty off-hand draws nothing on the left). This
//! is the player seeing their own tool in hand — the counterpart to the
//! remote-player avatars (Phase 6), which reuse the same `player_model()` arms
//! and the same `item_model::held_item_mesh` so the in-hand item always agrees
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
//! hand side ([`HandSide`]; the off-hand's left arm is the same layout mirrored
//! in x, built from the left arm part and mirrored translations and tilts, never
//! a negative scale, which would flip the winding and the normals).
//!
//! The two hands swing on their own timers ([`build_viewmodel_hands`]): a
//! right-click that routes to the off-hand (`hands::use_hand`) swings the left
//! hand only; mining, attacking and a main-hand use swing the right.
//!
//! A held tool draws as its own shape (`held_tool_model`). Its one-sided feature
//! (an axe's blade) points in toward the centre of the screen in first person, so
//! the item is built for the opposite hand's side than the hand it is in; in the
//! third-person hand the feature points outward.

use glam::{Mat4, Vec3};

use crate::block::BlockRegistry;
use crate::entity_model::{self, PartPose};
use crate::item_model::{self, HeldItem};
use crate::mesh::Vertex;
use crate::protocol::ItemRef;

/// Index of the RIGHT ARM `ModelPart` in `entity_model::player_model()`.
/// (head, body, left arm, right arm, left leg, right leg.)
pub const RIGHT_ARM_INDEX: usize = 3;
/// Index of the LEFT ARM `ModelPart` in `entity_model::player_model()`.
pub const LEFT_ARM_INDEX: usize = 2;

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

/// Which hand a viewmodel (or an avatar's held item) is: the main hand is the
/// right, the off-hand the left. The left slot is the right layout mirrored in
/// x, so every part of the slot below takes the side instead of assuming the
/// right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandSide {
    Right,
    Left,
}

impl HandSide {
    /// The other hand.
    pub fn opposite(self) -> HandSide {
        match self {
            HandSide::Right => HandSide::Left,
            HandSide::Left => HandSide::Right,
        }
    }

    /// The arm part of `entity_model::player_model()` this hand is.
    pub fn arm_index(self) -> usize {
        match self {
            HandSide::Right => RIGHT_ARM_INDEX,
            HandSide::Left => LEFT_ARM_INDEX,
        }
    }

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
        // The hand lantern stands up out of the fist so its glowing core clears the arm.
        ItemRef::Block(id) if id == crate::block::LANTERN => 0.32,
        other if item_model::held_is_sprite(other, registry) => 0.2,
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

/// Build one hand's viewmodel mesh in VIEW space, split by render path.
///
/// Returns `(arm_skin_verts, held_verts)`:
/// - **`arm_skin_verts`** — the arm part of `side` box-unwrapped onto the 64x64
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
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn build_viewmodel_vertices(
    side: HandSide,
    held: HeldItem,
    bob: f32,
    swing: f32,
    skin_layer: u32,
    registry: &BlockRegistry,
    arm_model: crate::skin_uv::ArmModel,
    aspect: f32,
) -> (Vec<Vertex>, Vec<Vertex>) {
    build_hand(side, held, bob, swing, 0.0, skin_layer, registry, arm_model, aspect)
}

/// [`build_viewmodel_vertices`] with `guard`: how far a shield in this hand has
/// been raised (0 lowered, 1 up; `shield_pose::raise_first_person`). Ignored for
/// any other item.
#[allow(clippy::too_many_arguments)]
fn build_hand(
    side: HandSide,
    held: HeldItem,
    bob: f32,
    swing: f32,
    guard: f32,
    skin_layer: u32,
    registry: &BlockRegistry,
    arm_model: crate::skin_uv::ArmModel,
    aspect: f32,
) -> (Vec<Vertex>, Vec<Vertex>) {
    // The hand you see is YOUR hand: a slim ("Alex") skin gets the 3-px arm and
    // the 3-px sleeve rects here too, not just on the third-person body.
    let arm_index = side.arm_index();
    let arm = &entity_model::player_model_for(arm_model)[arm_index];

    // The arm part in local/model space, centred on its own origin so the
    // whole thing sits at the view-space slot rather than the avatar's body
    // offset. Built UN-swung: the whole arm+item assembly is swung once about
    // the shoulder in `arm_transform`.
    //
    // Skin path: base body layer + sleeve overlay, the same two boxes (and the
    // same overlay inflate) the remote-avatar builder emits per part.
    let pose = PartPose {
        walk_swing: 0.0,
        head_pitch: 0.0,
        arm_override: Some(0.0),
        ..Default::default()
    };
    let base_uv = crate::skin_uv::base_faces(arm_model);
    let overlay_uv = crate::skin_uv::overlay_faces(arm_model);
    let mut arm_verts =
        entity_model::build_skin_part_local(arm, &base_uv[arm_index], 0.0, pose, skin_layer);
    arm_verts.extend(entity_model::build_skin_part_local(
        arm,
        &overlay_uv[arm_index],
        crate::skin_pose::overlay_inflate(arm_index),
        pose,
        skin_layer,
    ));

    // Equipped item, placed at the hand-end of the arm by `held_item_placement`.
    // Built into its OWN vec so it can render through the block-texture
    // pipeline, not the skin one.
    let mut held_verts = Vec::new();
    if !held.is_empty() {
        let placement = held_item_placement(held.item, registry, side);
        // `item_model` puts a held sprite beside the fist (HELD_SPRITE_OFFSET) so
        // its flame clears the hanging third-person arm, and a one-sided tool
        // reaches out to its hand's own side. Held upright in front of the player
        // the sprite offset would push it off to the side, so take it out; and a
        // tool is built for the OTHER hand so its feature points in toward the
        // centre of the screen.
        let mesh_side = side.opposite();
        let is_sprite = item_model::held_is_sprite(held.item, registry);
        let offset = item_model::sprite_offset(mesh_side);
        let mut item_verts = item_model::held_item_mesh(held, registry, mesh_side);
        for v in &mut item_verts {
            let mut p = Vec3::from(v.position);
            if is_sprite {
                p -= offset;
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

    // Off-hand release (slice 5) — a raised shield and the arm that holds it come
    // in toward the middle of the view, clear of the crosshair.
    if guard > 0.0 && held.tool == Some(crate::crafting::ToolType::Shield) {
        crate::shield_pose::raise_first_person(&mut arm_verts, &mut held_verts, side, aspect, guard);
    }

    (arm_verts, held_verts)
}

/// Both hands of the first-person view: the right arm with the main-hand item
/// (a bare fist when `main` is empty) and, only when the off-hand holds an item,
/// the left arm with that item. `swing_main` and `swing_off` are the hands' own
/// swing angles (radians). Returns `(arm_skin_verts, held_verts)` for both hands
/// together, the right hand's first, ready for the same two uploads. `guard` says
/// which hand's shield is raised and how far (0..=1, `shield_pose`); a raised
/// shield swings no hand.
#[allow(clippy::too_many_arguments)]
pub fn build_viewmodel_hands(
    main: HeldItem,
    off: Option<HeldItem>,
    bob: f32,
    swing_main: f32,
    swing_off: f32,
    guard: Option<(HandSide, f32)>,
    skin_layer: u32,
    registry: &BlockRegistry,
    arm_model: crate::skin_uv::ArmModel,
    aspect: f32,
) -> (Vec<Vertex>, Vec<Vertex>) {
    let raised = |side: HandSide| guard.filter(|(s, _)| *s == side).map_or(0.0, |(_, t)| t);
    let (mut arm, mut held) =
        build_hand(HandSide::Right, main, bob, swing_main, raised(HandSide::Right), skin_layer, registry, arm_model, aspect);
    if let Some(off) = off.filter(|o| !o.is_empty()) {
        let (left_arm, left_held) =
            build_hand(HandSide::Left, off, bob, swing_off, raised(HandSide::Left), skin_layer, registry, arm_model, aspect);
        arm.extend(left_arm);
        held.extend(left_held);
    }
    (arm, held)
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
        let (rest_arm, rest_held) = build_viewmodel_vertices(HandSide::Right, ItemRef::Tool(2).into(), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
        let (swung_arm, swung_held) = build_viewmodel_vertices(HandSide::Right, ItemRef::Tool(2).into(), 0.0, 0.6, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
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
        let (arm_empty, held_empty) = build_viewmodel_vertices(HandSide::Right, ItemRef::Empty.into(), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
        let (arm_block, held_block) = build_viewmodel_vertices(HandSide::Right, ItemRef::Block(1).into(), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
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
            let (v, _) = build_viewmodel_vertices(HandSide::Right, ItemRef::Empty.into(), 0.0, 0.0, 0, &reg, arm, 16.0 / 9.0);
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
        let (arm, held) = build_viewmodel_vertices(HandSide::Right, ItemRef::Tool(2).into(), 0.0, 0.0, 0, &reg, crate::skin_uv::ArmModel::Classic, 16.0 / 9.0);
        assert!(!arm.is_empty());
        assert!(!held.is_empty());
        for v in arm.iter().chain(&held) {
            assert!(v.position[2] < 0.0, "vertex behind camera: {:?}", v.position);
        }
    }
}

/// Where the first-person hand and its item land on screen (spec 2026-11-13,
/// "held items visible in first person", and the off-hand's mirror of it). CPU
/// projection tests: the vertices go through [`viewmodel_projection`], the matrix
/// the render pass uses, with an identity view, at rest (bob 0, swing 0) unless a
/// test says it is at the peak. Every test runs for both hands: the left hand's
/// numbers are the right's with x negated (`side.sign()`).
#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::block;
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::skin_uv::ArmModel;

    /// The aspect ratios people play at: 16:9, 16:10, 4:3 and phone portrait.
    const ASPECTS: [f32; 4] = [16.0 / 9.0, 16.0 / 10.0, 4.0 / 3.0, 9.0 / 16.0];
    const SIDES: [HandSide; 2] = [HandSide::Right, HandSide::Left];

    /// What is held, with the least height its projected box must have (as a
    /// fraction of the viewport height: the spec's 12% and, for the tall items,
    /// 15%). Every tool type is here, in iron.
    fn items() -> Vec<(String, HeldItem, f32)> {
        let mut v: Vec<(String, HeldItem, f32)> = vec![
            ("torch".into(), ItemRef::Block(block::TORCH).into(), 0.15),
            ("iron slab".into(), ItemRef::Tool(2).into(), 0.15),
            ("stone block".into(), ItemRef::Block(block::STONE).into(), 0.12),
            ("oak sapling".into(), ItemRef::Block(block::SAPLING_OAK).into(), 0.15),
            // Off-hand slice 4: the hand lantern, a small model, not a sprite.
            ("lantern".into(), ItemRef::Block(block::LANTERN).into(), 0.12),
        ];
        for t in crate::held_tool_model::tests::all_tool_types() {
            let item = crate::item::Item::Tool(Tool::new(t, ToolMaterial::Iron));
            v.push((format!("iron {t:?}"), HeldItem::of(&item), 0.15));
        }
        v
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

    fn build(side: HandSide, held: HeldItem, aspect: f32, swing: f32, arm: ArmModel) -> (Vec<Vec3>, Vec<Vec3>) {
        let (a, h) = build_viewmodel_vertices(side, held, 0.0, swing, 0, &BlockRegistry::new(), arm, aspect);
        (view_positions(&a), view_positions(&h))
    }

    /// The four corners of the arm box's hand end and of its shoulder end, in
    /// view space.
    fn arm_ends(side: HandSide, aspect: f32, swing: f32, arm: ArmModel) -> ([Vec3; 4], [Vec3; 4]) {
        let size = entity_model::player_model_for(arm)[side.arm_index()].size;
        let m = arm_transform(aspect, side, 0.0, swing);
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

    /// The shoulder end is out of view when every corner is beyond the hand's
    /// own side edge or the bottom edge.
    fn shoulder_outside(side: HandSide, shoulder: &[Vec3; 4], aspect: f32) -> bool {
        shoulder.iter().all(|p| {
            let n = ndc(*p, aspect);
            n.x * side.sign() > 1.0 || n.y < -1.0
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

    fn describe(name: &str, side: HandSide, aspect: f32, pts: &[Vec3]) -> String {
        let (lo, hi) = bbox(pts, aspect);
        let c = (lo + hi) * 0.5;
        format!(
            "{side:?} {name} @{aspect:.2}: ndc x {:.2}..{:.2}, y {:.2}..{:.2}, height {:.0}%, centre ({:.2}, {:.2})",
            lo.x, hi.x, lo.y, hi.y, (hi.y - lo.y) * 50.0, c.x, c.y
        )
    }

    /// Every numeric the spec measures, printed for the before/after record.
    /// Always passes; run with --nocapture.
    #[test]
    fn viewmodel_layout_numbers() {
        for side in SIDES {
            for aspect in ASPECTS {
                let (hand, shoulder) = arm_ends(side, aspect, 0.0, ArmModel::Classic);
                let sh: Vec<_> = shoulder.iter().map(|p| ndc(*p, aspect)).collect();
                let hn: Vec<_> = hand.iter().map(|p| ndc(*p, aspect)).collect();
                eprintln!(
                    "VMLAYOUT {side:?} aspect {aspect:.2}: fist ndc {hn:.2?}  shoulder ndc {sh:.2?}  shoulder_outside={}",
                    shoulder_outside(side, &shoulder, aspect)
                );
                for (name, held, _) in items() {
                    for (label, swing) in [("rest", 0.0), ("peak", swing_peak())] {
                        let (_, item) = build(side, held, aspect, swing, ArmModel::Classic);
                        eprintln!("VMLAYOUT {label} {}", describe(&name, side, aspect, &item));
                    }
                }
            }
        }
    }

    // 1. Every vertex of the held item is inside the viewport.
    #[test]
    fn spec1_every_held_vertex_is_in_the_viewport() {
        for side in SIDES {
            for aspect in ASPECTS {
                for (name, held, _) in items() {
                    let (_, item) = build(side, held, aspect, 0.0, ArmModel::Classic);
                    assert!(item.iter().all(|p| p.z < -0.02), "{name}: vertex too close to the camera");
                    let outside: Vec<_> = item
                        .iter()
                        .map(|p| ndc(*p, aspect))
                        .filter(|n| !inside_viewport(*n, 0.98))
                        .collect();
                    assert!(outside.is_empty(), "{}; {} of {} vertices outside +-0.98", describe(&name, side, aspect, &item), outside.len(), item.len());
                }
            }
        }
    }

    // 2. Big enough, and in the lower quadrant on the hand's own side.
    #[test]
    fn spec2_item_box_is_tall_enough_and_in_the_hands_lower_quadrant() {
        for side in SIDES {
            for aspect in ASPECTS {
                for (name, held, min_h) in items() {
                    let (_, item) = build(side, held, aspect, 0.0, ArmModel::Classic);
                    let (lo, hi) = bbox(&item, aspect);
                    let c = (lo + hi) * 0.5;
                    // The part of the box that is on screen: an item that mostly hangs
                    // below the bottom edge is not "tall" in view.
                    let seen_h = (hi.y.min(1.0) - lo.y.max(-1.0)).max(0.0) * 0.5;
                    assert!(seen_h >= min_h, "{}: needs >= {:.0}% tall on screen, has {:.0}%", describe(&name, side, aspect, &item), min_h * 100.0, seen_h * 100.0);
                    assert!(c.x * side.sign() > 0.15 && c.y < -0.15, "{}: centre must be lower-{side:?} (|x| > 0.15 on the hand's side, y < -0.15)", describe(&name, side, aspect, &item));
                    assert!(inside_viewport(c, 1.0), "{}: centre must be on screen", describe(&name, side, aspect, &item));
                }
            }
        }
    }

    // 3. The fist (the hand end of the arm box) is in the viewport.
    #[test]
    fn spec3_fist_is_in_the_viewport() {
        for side in SIDES {
            for aspect in ASPECTS {
                for arm in [ArmModel::Classic, ArmModel::Slim] {
                    let (hand, _) = arm_ends(side, aspect, 0.0, arm);
                    for p in hand {
                        let n = ndc(p, aspect);
                        assert!(inside_viewport(n, 1.0), "{side:?} {arm:?} fist corner at ndc ({:.2}, {:.2}) @{aspect:.2} is outside the viewport", n.x, n.y);
                    }
                }
            }
        }
    }

    // 4. The shoulder end is out of the viewport (#15's guard).
    #[test]
    fn spec4_shoulder_stays_off_screen() {
        for side in SIDES {
            for aspect in ASPECTS {
                for arm in [ArmModel::Classic, ArmModel::Slim] {
                    let (_, shoulder) = arm_ends(side, aspect, 0.0, arm);
                    assert!(shoulder.iter().all(|p| p.z < -0.05), "{side:?} {arm:?} shoulder too close to the camera: {shoulder:?}");
                    let n: Vec<_> = shoulder.iter().map(|p| ndc(*p, aspect)).collect();
                    assert!(shoulder_outside(side, &shoulder, aspect), "{side:?} {arm:?} shoulder corners at ndc {n:.2?} @{aspect:.2} are not all beyond the side or bottom edge");
                }
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
        assert!(!shoulder_outside(HandSide::Right, &top, 16.0 / 9.0), "the old slot's shoulder is on screen, so the guard must report it");
        // The same slot mirrored is the left hand's old place: the guard bites there too.
        let mirrored = top.map(|p| Vec3::new(-p.x, p.y, p.z));
        assert!(!shoulder_outside(HandSide::Left, &mirrored, 16.0 / 9.0));
    }

    // 5. Nothing of the arm or item covers the centre +-5% of the viewport.
    #[test]
    fn spec5_the_crosshair_stays_clear() {
        // +-5% of the viewport either side of the centre is +-0.10 in NDC.
        const HALF: f32 = 0.10;
        for side in SIDES {
            for aspect in ASPECTS {
                for (name, held, _) in items() {
                    let (arm, item) = build(side, held, aspect, 0.0, ArmModel::Classic);
                    assert!(!covers_centre(&arm, aspect, HALF), "{side:?} arm covers the centre with {name} @{aspect:.2}");
                    assert!(!covers_centre(&item, aspect, HALF), "{side:?} {name} covers the centre @{aspect:.2}");
                }
                let (arm, _) = build(side, ItemRef::Empty.into(), aspect, 0.0, ArmModel::Classic);
                assert!(!covers_centre(&arm, aspect, HALF), "the empty {side:?} hand's arm covers the centre @{aspect:.2}");
            }
        }
    }

    // 6. At the swing peak some of the item is still in view.
    #[test]
    fn spec6_item_stays_partly_in_view_at_the_swing_peak() {
        for side in SIDES {
            for aspect in ASPECTS {
                for (name, held, _) in items() {
                    let (_, item) = build(side, held, aspect, swing_peak(), ArmModel::Classic);
                    let (lo, hi) = bbox(&item, aspect);
                    let overlaps = lo.x < 1.0 && hi.x > -1.0 && lo.y < 1.0 && hi.y > -1.0;
                    assert!(overlaps, "{} at the swing peak is entirely outside the viewport", describe(&name, side, aspect, &item));
                }
            }
        }
    }

    // Both hands at once.

    fn both(aspect: f32, swing_main: f32, swing_off: f32) -> (Vec<Vertex>, Vec<Vertex>) {
        let pick = HeldItem::of(&crate::item::Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron)));
        let torch: HeldItem = ItemRef::Block(block::TORCH).into();
        build_viewmodel_hands(pick, Some(torch), 0.0, swing_main, swing_off, None, 0, &BlockRegistry::new(), ArmModel::Classic, aspect)
    }

    #[test]
    fn with_both_hands_full_neither_enters_the_centre_box() {
        for aspect in ASPECTS {
            let (arm, held) = both(aspect, 0.0, 0.0);
            let (arm, held) = (view_positions(&arm), view_positions(&held));
            assert!(!covers_centre(&arm, aspect, 0.05), "an arm covers the centre +-5% @{aspect:.2}");
            assert!(!covers_centre(&held, aspect, 0.05), "an item covers the centre +-5% @{aspect:.2}");
        }
    }

    #[test]
    fn the_left_item_is_in_the_lower_left_and_the_main_item_stays_lower_right() {
        for aspect in ASPECTS {
            let (_, held) = both(aspect, 0.0, 0.0);
            let (left, right): (Vec<_>, Vec<_>) = view_positions(&held).into_iter().partition(|p| p.x < 0.0);
            assert!(!left.is_empty() && !right.is_empty(), "@{aspect:.2}: an item in each hand");
            let (lo, hi) = bbox(&left, aspect);
            let c = (lo + hi) * 0.5;
            assert!(c.x < -0.15 && c.y < -0.15, "left item centre must be lower-left @{aspect:.2}: {c:?}");
            let (lo, hi) = bbox(&right, aspect);
            let c = (lo + hi) * 0.5;
            assert!(c.x > 0.15 && c.y < -0.15, "main item centre must stay lower-right @{aspect:.2}: {c:?}");
        }
    }

    #[test]
    fn an_empty_off_hand_draws_nothing_on_the_left() {
        let reg = BlockRegistry::new();
        let pick: HeldItem = ItemRef::Tool(2).into();
        let alone = build_viewmodel_hands(pick, None, 0.0, 0.0, 0.0, None, 0, &reg, ArmModel::Classic, 16.0 / 9.0);
        let right_only = build_viewmodel_vertices(HandSide::Right, pick, 0.0, 0.0, 0, &reg, ArmModel::Classic, 16.0 / 9.0);
        assert_eq!(alone.0.len(), right_only.0.len(), "no left arm");
        assert_eq!(alone.1.len(), right_only.1.len(), "no left item");
        assert!(alone.0.iter().chain(&alone.1).all(|v| v.position[0] > 0.0), "everything is on the right half");
        // An off-hand slot that holds an empty ref is empty too.
        let still = build_viewmodel_hands(pick, Some(HeldItem::EMPTY), 0.0, 0.0, 0.0, None, 0, &reg, ArmModel::Classic, 16.0 / 9.0);
        assert_eq!(still.0.len(), right_only.0.len());
        // A full off-hand adds a left arm (two boxes) and its item.
        let (arm, held) = both(16.0 / 9.0, 0.0, 0.0);
        assert_eq!(arm.len(), 72 * 2, "two arms");
        assert!(held.len() > right_only.1.len(), "and two items");
    }

    #[test]
    fn an_off_hand_swing_moves_the_left_hand_only() {
        for aspect in [16.0 / 9.0, 9.0 / 16.0] {
            let (rest_arm, rest_held) = both(aspect, 0.0, 0.0);
            let (swung_arm, swung_held) = both(aspect, 0.0, 0.6);
            assert_eq!(rest_arm.len(), swung_arm.len());
            let moved = |a: &[Vertex], b: &[Vertex], left: bool| {
                a.iter().zip(b).filter(|(x, _)| (x.position[0] < 0.0) == left).any(|(x, y)| x.position != y.position)
            };
            assert!(moved(&rest_arm, &swung_arm, true) && moved(&rest_held, &swung_held, true), "the left hand swings");
            assert!(!moved(&rest_arm, &swung_arm, false) && !moved(&rest_held, &swung_held, false), "the right hand stays");
            // And the main swing moves the right hand only.
            let (m_arm, m_held) = both(aspect, 0.6, 0.0);
            assert!(moved(&rest_arm, &m_arm, false) && moved(&rest_held, &m_held, false));
            assert!(!moved(&rest_arm, &m_arm, true) && !moved(&rest_held, &m_held, true), "the left hand stays");
        }
    }

    // Off-hand release (slice 5): a raised shield.

    fn shield() -> HeldItem {
        HeldItem::of(&crate::item::Item::Tool(Tool::new(ToolType::Shield, ToolMaterial::Wood)))
    }

    #[test]
    fn a_raised_shield_covers_part_of_the_view_and_keeps_the_crosshair_clear() {
        for side in SIDES {
            for aspect in ASPECTS {
                let reg = BlockRegistry::new();
                let (arm, held) = build_hand(side, shield(), 0.0, 0.0, 1.0, 0, &reg, ArmModel::Classic, aspect);
                let (arm, held) = (view_positions(&arm), view_positions(&held));
                assert!(!covers_centre(&held, aspect, 0.05), "{side:?} shield covers the centre +-5% @{aspect:.2}");
                assert!(!covers_centre(&arm, aspect, 0.05), "{side:?} arm covers the centre +-5% @{aspect:.2}");
                assert!(held.iter().chain(&arm).all(|p| p.z < 0.0), "in front of the camera");
                let (lo, hi) = bbox(&held, aspect);
                let height = (hi.y - lo.y) * 0.5;
                assert!(height >= 0.30, "{}: a raised shield covers at least 30% of the view's height", describe("raised shield", side, aspect, &held));
                let (inner, outer) = if side == HandSide::Right { (lo.x, hi.x) } else { (-hi.x, -lo.x) };
                assert!(inner >= 0.05 && outer <= 1.0, "{}: on its own side of the centre and on screen", describe("raised shield", side, aspect, &held));
                // It comes in toward the centre and up from where the lowered one sits.
                let (_, rest) = build_hand(side, shield(), 0.0, 0.0, 0.0, 0, &reg, ArmModel::Classic, aspect);
                let (rlo, rhi) = bbox(&view_positions(&rest), aspect);
                let (rc, c) = ((rlo + rhi) * 0.5, (lo + hi) * 0.5);
                assert!(c.x * side.sign() < rc.x * side.sign(), "{side:?} @{aspect:.2}: nearer the centre than lowered ({c:?} vs {rc:?})");
                assert!(c.y > rc.y, "{side:?} @{aspect:.2}: higher than lowered ({c:?} vs {rc:?})");
            }
        }
    }

    #[test]
    fn the_raise_eases_in_with_the_count() {
        let reg = BlockRegistry::new();
        let centre_x = |t: f32| {
            let (_, h) = build_hand(HandSide::Right, shield(), 0.0, 0.0, t, 0, &reg, ArmModel::Classic, 16.0 / 9.0);
            let (lo, hi) = bbox(&view_positions(&h), 16.0 / 9.0);
            (lo.x + hi.x) * 0.5
        };
        let (a, b, c) = (centre_x(0.0), centre_x(0.5), centre_x(1.0));
        assert!(a > b && b > c, "each step comes further in: {a} {b} {c}");
    }

    #[test]
    fn the_shield_hand_is_the_hand_the_guard_names_and_the_other_hand_is_unmoved() {
        let reg = BlockRegistry::new();
        let aspect = 16.0 / 9.0;
        let sword = HeldItem::of(&crate::item::Item::Tool(Tool::new(ToolType::Sword, ToolMaterial::Iron)));
        let rest = build_viewmodel_hands(sword, Some(shield()), 0.0, 0.0, 0.0, None, 0, &reg, ArmModel::Classic, aspect);
        let up = build_viewmodel_hands(sword, Some(shield()), 0.0, 0.0, 0.0, Some((HandSide::Left, 1.0)), 0, &reg, ArmModel::Classic, aspect);
        let moved = |a: &[Vertex], b: &[Vertex], left: bool| a.iter().zip(b).filter(|(x, _)| (x.position[0] < 0.0) == left).any(|(x, y)| x.position != y.position);
        assert!(moved(&rest.0, &up.0, true) && moved(&rest.1, &up.1, true), "the left arm and shield rose");
        assert!(!moved(&rest.0, &up.0, false) && !moved(&rest.1, &up.1, false), "the right hand's sword stays put");
        // A guard named for a hand that holds no shield changes nothing.
        let none = build_viewmodel_hands(sword, Some(shield()), 0.0, 0.0, 0.0, Some((HandSide::Right, 1.0)), 0, &reg, ArmModel::Classic, aspect);
        assert!(!moved(&rest.0, &none.0, true) && !moved(&rest.1, &none.1, true) && !moved(&rest.1, &none.1, false), "a sword is not raised");
    }
}

//! Single source of truth for avatar-skin box geometry: the part boxes, the
//! Workshop "limbs apart" offsets, and the per-part overlay inflate.
//!
//! The renderer (`entity_model`) and the paint ray hit-test (`skin_hit`) BOTH
//! derive from this module. That pairing is load-bearing: if they disagree, the
//! crosshair lands somewhere other than where the limb is drawn. Never hardcode
//! a box, an offset or an inflate anywhere else.

use crate::skin_uv::{ArmModel, SkinLayer};

/// Avatar parts, in `entity_model::player_model()` order.
pub const PART_COUNT: usize = 6;

/// Part boxes as `(centre, half-extents)`, verbatim from `player_model()`.
/// Order: `[head, body, arm_l, arm_r, leg_l, leg_r]`. CLASSIC (4-px) arms —
/// [`SLIM_ARM_L`] / [`SLIM_ARM_R`] replace rows 2 and 3 on the slim model.
const BOXES: [([f32; 3], [f32; 3]); PART_COUNT] = [
    ([0.0, 1.575, 0.0], [0.25, 0.25, 0.25]),         // head
    ([0.0, 0.975, 0.0], [0.25, 0.35, 0.125]),        // body
    ([-0.375, 1.05, 0.0], [0.125, 0.35, 0.125]),     // arm_l
    ([0.375, 1.05, 0.0], [0.125, 0.35, 0.125]),      // arm_r
    ([-0.125, 0.3125, 0.0], [0.125, 0.3125, 0.125]), // leg_l
    ([0.125, 0.3125, 0.0], [0.125, 0.3125, 0.125]),  // leg_r
];

/// How far Minecraft's slim ("Alex") model drops the whole arm — box AND pivot
/// together — relative to classic: 0.5 skin pixel, i.e. 0.5/16 of a block.
/// (Java `PlayerModel`: the slim arm's rotation point is y 2.5 where classic is
/// y 2, and +y is DOWN in that space, so the arm hangs half a pixel lower.)
pub const SLIM_ARM_DROP: f32 = 0.03125;

/// Half the width a slim arm loses: the box goes 4 px → 3 px, all of it off the
/// OUTER edge, so the centre shifts inward by half a pixel and the half-extent
/// shrinks by half a pixel. One skin pixel = 1/16 block.
const SLIM_HALF_PX: f32 = 0.03125;

/// Slim LEFT arm box. Classic is centre (-0.375, 1.05, 0), half (0.125, …):
/// x spans -0.5 (outer) .. -0.25 (inner, flush with the torso). Slim keeps the
/// INNER edge at -0.25 and pulls the outer edge in to -0.4375.
const SLIM_ARM_L: ([f32; 3], [f32; 3]) = (
    [-0.375 + SLIM_HALF_PX, 1.05 - SLIM_ARM_DROP, 0.0],
    [0.125 - SLIM_HALF_PX, 0.35, 0.125],
);

/// Slim RIGHT arm box — the mirror of [`SLIM_ARM_L`]: the inner edge stays at
/// +0.25, the outer comes in from +0.5 to +0.4375.
const SLIM_ARM_R: ([f32; 3], [f32; 3]) = (
    [0.375 - SLIM_HALF_PX, 1.05 - SLIM_ARM_DROP, 0.0],
    [0.125 - SLIM_HALF_PX, 0.35, 0.125],
);

/// The `(centre, half-extents)` of `part` on `arm`. The single place the slim
/// arm boxes are substituted — `part_box` and `avatar_aabb` both come through
/// here, so they can never disagree about how wide an arm is.
fn box_of(part: usize, arm: ArmModel) -> ([f32; 3], [f32; 3]) {
    match (arm, part) {
        (ArmModel::Slim, 2) => SLIM_ARM_L,
        (ArmModel::Slim, 3) => SLIM_ARM_R,
        _ => BOXES.get(part).copied().unwrap_or(([0.0; 3], [0.0; 3])),
    }
}

/// Fully-separated offsets for the Workshop "limbs apart" pose, in model units.
///
/// The body is the anchor and never moves. Sizes are chosen for comfortable
/// mouse aim, not bare geometric possibility — a gap as small as 0.05 already
/// exposes every face, but at the ×4 blow-up these give a full block of daylight
/// between an arm and the torso. `apart_pose_reaches_every_face` in `skin_hit`
/// is the test that holds this honest.
const APART: [[f32; 3]; PART_COUNT] = [
    [0.0, 0.25, 0.0],     // head lifts, exposing body-top and head-bottom
    [0.0, 0.0, 0.0],      // body — the anchor
    [-0.25, 0.0, 0.0],    // arm_l swings clear of the torso
    [0.25, 0.0, 0.0],     // arm_r
    [-0.125, -0.25, 0.0], // leg_l drops and parts
    [0.125, -0.25, 0.0],  // leg_r
];

/// Overlay inflate per side for the head, matching Minecraft's hat layer:
/// 1 px total growth = 0.5 px per side, and one skin pixel is 1/16 of a block.
const HEAD_INFLATE: f32 = 0.03125;

/// Overlay inflate per side for body/arms/legs, matching Minecraft's
/// jacket / sleeve / trouser layers: 0.5 px total growth = 0.25 px per side.
const LIMB_INFLATE: f32 = 0.015625;

/// Overlay inflate for the blown-up Workshop mannequin ONLY — a full skin pixel
/// per side, so the clothes shell is visibly proud of the body while you paint
/// it instead of being merely technically offset. Never used on a worn avatar,
/// which must stay Minecraft-exact so skins look the same here and there.
pub const EDIT_INFLATE: f32 = 0.0625;

/// How far `part` is displaced from its resting position, where `t` is the eased
/// separation factor (0 = together, 1 = fully apart). Render and hit-test must
/// pass the SAME `t`. Out-of-range parts do not move.
pub fn part_offset(part: usize, t: f32) -> [f32; 3] {
    let Some(a) = APART.get(part) else {
        return [0.0, 0.0, 0.0];
    };
    [a[0] * t, a[1] * t, a[2] * t]
}

/// Minecraft-exact overlay inflate for `part`. Out-of-range parts get the limb
/// value (the common case) rather than panicking on a render path.
pub fn overlay_inflate(part: usize) -> f32 {
    if part == 0 { HEAD_INFLATE } else { LIMB_INFLATE }
}

/// The axis-aligned box for `part` on `layer` as `(min, max)`, displaced by the
/// separation factor `t`. `edit` swaps the worn overlay inflate for the
/// exaggerated Workshop one; it never affects the base layer.
pub fn part_box(
    part: usize,
    layer: SkinLayer,
    t: f32,
    edit: bool,
    arm: ArmModel,
) -> ([f32; 3], [f32; 3]) {
    let (centre, half) = box_of(part, arm);
    let inflate = match layer {
        SkinLayer::Base => 0.0,
        SkinLayer::Overlay => {
            if edit {
                EDIT_INFLATE
            } else {
                overlay_inflate(part)
            }
        }
    };
    let off = part_offset(part, t);
    let mut min = [0.0f32; 3];
    let mut max = [0.0f32; 3];
    for a in 0..3 {
        min[a] = centre[a] + off[a] - half[a] - inflate;
        max[a] = centre[a] + off[a] + half[a] + inflate;
    }
    (min, max)
}

/// The union bounding box (min, max) of all 6 avatar parts at separation `t`,
/// with `inflate` applied to every part's half-extents on every axis. Derived
/// from the SAME `BOXES` / `part_offset` tables `part_box` (and so
/// `ray_hit_avatar`) use — never hand-retype a number here, or the coarse aim
/// box (Workshop Bellows highlight / nearest-vs-block compare, `workshop::
/// pick_avatar_mannequin`) can silently stop matching the real per-part paint
/// hit-test, especially once the limbs-apart pose (R) is in play.
///
/// Takes no [`ArmModel`]: it is the CLASSIC union, and every slim part box is
/// contained in it (a slim arm is narrower on x and only half a skin pixel
/// lower on y, far inside the legs' reach), so the coarse aim box stays a
/// correct — merely slightly generous — bound for a slim avatar too.
/// `avatar_aabb_contains_every_part_box_when_fully_apart` proves that for both.
pub fn avatar_aabb(t: f32, inflate: f32) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for (part, &(centre, half)) in BOXES.iter().enumerate() {
        let off = part_offset(part, t);
        for a in 0..3 {
            let a_lo = centre[a] + off[a] - half[a] - inflate;
            let a_hi = centre[a] + off[a] + half[a] + inflate;
            lo[a] = lo[a].min(a_lo);
            hi[a] = hi[a].max(a_hi);
        }
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin_uv::{ArmModel, SkinLayer};

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
    }

    #[test]
    fn overlay_inflate_matches_minecraft() {
        // Minecraft grows the hat box by 1px total = 0.5px per side; the
        // jacket/sleeves/pants by 0.5px total = 0.25px per side. One skin pixel
        // is 1/16 of a block, so 0.5px = 0.03125 and 0.25px = 0.015625.
        approx(overlay_inflate(0), 0.03125); // head
        for part in 1..PART_COUNT {
            approx(overlay_inflate(part), 0.015625); // body, arms, legs
        }
    }

    #[test]
    fn edit_inflate_is_one_pixel_per_side() {
        // A full skin pixel per side, so the shell is visibly proud of the body
        // on the blown-up mannequin rather than only technically offset.
        approx(EDIT_INFLATE, 0.0625);
    }

    #[test]
    fn offsets_are_zero_when_together() {
        for part in 0..PART_COUNT {
            assert_eq!(part_offset(part, 0.0), [0.0, 0.0, 0.0], "part {part}");
        }
    }

    #[test]
    fn offsets_separate_limbs_when_apart() {
        approx(part_offset(0, 1.0)[1], 0.25); // head lifts
        assert_eq!(part_offset(1, 1.0), [0.0, 0.0, 0.0], "body is the anchor");
        approx(part_offset(2, 1.0)[0], -0.25); // arm_L out -X
        approx(part_offset(3, 1.0)[0], 0.25); // arm_R out +X
        approx(part_offset(4, 1.0)[0], -0.125); // leg_L out -X
        approx(part_offset(4, 1.0)[1], -0.25); // legs drop
        approx(part_offset(5, 1.0)[0], 0.125); // leg_R out +X
        approx(part_offset(5, 1.0)[1], -0.25);
    }

    #[test]
    fn offsets_lerp_linearly() {
        // The eased separation factor drives render and hit-test identically,
        // so a half-open pose must be expressible.
        approx(part_offset(2, 0.5)[0], -0.125);
        approx(part_offset(0, 0.5)[1], 0.125);
    }

    #[test]
    fn out_of_range_part_has_no_offset() {
        assert_eq!(part_offset(99, 1.0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn base_box_matches_the_player_model() {
        // Head: centre (0, 1.575, 0), half-extent 0.25 on every axis.
        let (min, max) = part_box(0, SkinLayer::Base, 0.0, false, ArmModel::Classic);
        approx(min[0], -0.25);
        approx(max[0], 0.25);
        approx(min[1], 1.325);
        approx(max[1], 1.825);
    }

    #[test]
    fn overlay_box_is_inflated_by_its_part_value() {
        let (bmin, bmax) = part_box(2, SkinLayer::Base, 0.0, false, ArmModel::Classic);
        let (omin, omax) = part_box(2, SkinLayer::Overlay, 0.0, false, ArmModel::Classic);
        approx(bmin[0] - omin[0], 0.015625); // arm uses the limb value
        approx(omax[0] - bmax[0], 0.015625);
    }

    #[test]
    fn edit_box_is_inflated_more_than_the_worn_box() {
        let (_, worn) = part_box(2, SkinLayer::Overlay, 0.0, false, ArmModel::Classic);
        let (_, edit) = part_box(2, SkinLayer::Overlay, 0.0, true, ArmModel::Classic);
        assert!(edit[0] > worn[0], "edit shell must be visibly proud");
        approx(edit[0] - worn[0], 0.0625 - 0.015625);
    }

    #[test]
    fn edit_flag_does_not_inflate_the_base_layer() {
        // Only the clothes shell is exaggerated; the body keeps its true size.
        let (a, b) = part_box(1, SkinLayer::Base, 0.0, false, ArmModel::Classic);
        let (c, d) = part_box(1, SkinLayer::Base, 0.0, true, ArmModel::Classic);
        assert_eq!((a, b), (c, d));
    }

    #[test]
    fn apart_pose_moves_the_box() {
        let (together, _) = part_box(3, SkinLayer::Base, 0.0, false, ArmModel::Classic);
        let (apart, _) = part_box(3, SkinLayer::Base, 1.0, false, ArmModel::Classic);
        approx(apart[0] - together[0], 0.25);
    }

    // ── avatar_aabb: coarse aim box for the Workshop Bellows (2026-09-03) ────

    #[test]
    fn avatar_aabb_matches_the_old_hand_written_constants_when_together() {
        // At t=0 / inflate=0 this must land close to the box that used to be
        // hand-typed as `workshop::AVATAR_AABB_MIN/MAX` ([-0.5,0,-0.25] ..
        // [0.5,1.86,0.25]): x and z match exactly (arm width / head depth); the
        // real head-top (centre 1.575 + half 0.25 = 1.825) is 0.035 BELOW the
        // old 1.86 — that old constant carried some extra rounded-up headroom
        // that was never tied to a real box. This function reports the true
        // value instead.
        let (lo, hi) = avatar_aabb(0.0, 0.0);
        approx(lo[0], -0.5); // computed lo.x = -0.5
        approx(hi[0], 0.5); // computed hi.x = 0.5
        approx(lo[1], 0.0); // computed lo.y = 0.0
        assert!(
            (hi[1] - 1.825).abs() < 0.02,
            "computed hi.y = {} (expected ~1.825, the real head-top)",
            hi[1]
        );
        approx(lo[2], -0.25); // computed lo.z = -0.25
        approx(hi[2], 0.25); // computed hi.z = 0.25
    }

    #[test]
    fn avatar_aabb_contains_every_part_box_when_fully_apart() {
        // EDIT_INFLATE is the largest inflate any part_box call ever uses (the
        // Workshop paint ray always passes edit=true), so avatar_aabb at that
        // inflate must contain every part's box at every layer/edit combo.
        let (lo, hi) = avatar_aabb(1.0, EDIT_INFLATE);
        for arm in [ArmModel::Classic, ArmModel::Slim] {
        for part in 0..PART_COUNT {
            for &(layer, edit) in &[
                (SkinLayer::Base, false),
                (SkinLayer::Overlay, false),
                (SkinLayer::Overlay, true),
            ] {
                let (pmin, pmax) = part_box(part, layer, 1.0, edit, arm);
                for a in 0..3 {
                    assert!(
                        pmin[a] >= lo[a] - 1e-6,
                        "{arm:?} part {part} {layer:?}/{edit} min[{a}]={} not covered by aabb lo[{a}]={}",
                        pmin[a],
                        lo[a]
                    );
                    assert!(
                        pmax[a] <= hi[a] + 1e-6,
                        "{arm:?} part {part} {layer:?}/{edit} max[{a}]={} not covered by aabb hi[{a}]={}",
                        pmax[a],
                        hi[a]
                    );
                }
            }
        }
        }
    }

    #[test]
    fn avatar_aabb_reaches_the_separated_arms_and_raised_head() {
        // Fully apart: arms swing to x=±0.75 before inflate, head top to
        // y=2.075 before inflate (see skin_pose module docs / APART table).
        let (lo, hi) = avatar_aabb(1.0, EDIT_INFLATE);
        assert!(lo[0] <= -0.75, "computed lo.x = {} (expected <= -0.75)", lo[0]);
        assert!(hi[0] >= 0.75, "computed hi.x = {} (expected >= 0.75)", hi[0]);
        assert!(hi[1] >= 2.0, "computed hi.y = {} (expected >= 2.0)", hi[1]);
    }

    // ── Slim ("Alex") arm geometry ───────────────────────────────────────────

    #[test]
    fn slim_arm_is_three_pixels_wide_with_the_inner_edge_unchanged() {
        // THE geometric rule: 4 px → 3 px, the whole pixel coming off the OUTER
        // edge. So the edge against the torso is identical to classic and the
        // arm cannot float away from (or sink into) the body.
        for (part, inner_sign) in [(2usize, 1.0f32), (3, -1.0)] {
            let (cmin, cmax) = part_box(part, SkinLayer::Base, 0.0, false, ArmModel::Classic);
            let (smin, smax) = part_box(part, SkinLayer::Base, 0.0, false, ArmModel::Slim);
            approx(cmax[0] - cmin[0], 0.25); // classic = 4/16
            approx(smax[0] - smin[0], 0.1875); // slim = 3/16
            // The inner edge is the one nearer x = 0.
            let (c_inner, s_inner) = if inner_sign > 0.0 { (cmax[0], smax[0]) } else { (cmin[0], smin[0]) };
            approx(s_inner, c_inner);
            // …and the outer edge has moved in by exactly one skin pixel.
            let (c_outer, s_outer) = if inner_sign > 0.0 { (cmin[0], smin[0]) } else { (cmax[0], smax[0]) };
            approx((s_outer - c_outer).abs(), 0.0625);
        }
        // Left arm inner edge is -0.25, right arm inner edge is +0.25 — flush
        // with the torso's ±0.25 half-width either way.
        let (lmin, lmax) = part_box(2, SkinLayer::Base, 0.0, false, ArmModel::Slim);
        approx(lmax[0], -0.25);
        approx(lmin[0], -0.4375);
        let (rmin, rmax) = part_box(3, SkinLayer::Base, 0.0, false, ArmModel::Slim);
        approx(rmin[0], 0.25);
        approx(rmax[0], 0.4375);
    }

    #[test]
    fn slim_arm_hangs_half_a_pixel_lower() {
        // Java `PlayerModel` sets the slim arm's rotation point 0.5 px lower
        // (y 2.5 vs 2, +y down), and the box moves with it. Both edges shift
        // together — the arm drops, it does not stretch.
        for part in [2usize, 3] {
            let (cmin, cmax) = part_box(part, SkinLayer::Base, 0.0, false, ArmModel::Classic);
            let (smin, smax) = part_box(part, SkinLayer::Base, 0.0, false, ArmModel::Slim);
            approx(cmin[1] - smin[1], SLIM_ARM_DROP);
            approx(cmax[1] - smax[1], SLIM_ARM_DROP);
            approx(smax[1] - smin[1], cmax[1] - cmin[1]);
        }
        approx(SLIM_ARM_DROP, 0.03125);
    }

    #[test]
    fn slim_leaves_head_body_and_legs_alone() {
        for part in [0usize, 1, 4, 5] {
            for layer in [SkinLayer::Base, SkinLayer::Overlay] {
                assert_eq!(
                    part_box(part, layer, 0.0, false, ArmModel::Classic),
                    part_box(part, layer, 0.0, false, ArmModel::Slim),
                    "part {part} must not change with the arm model"
                );
            }
        }
    }

    #[test]
    fn slim_arm_still_inflates_and_separates_like_classic() {
        // The overlay inflate and the limbs-apart offset are model-independent;
        // a regression here would break the sleeve or the R pose on slim only.
        let (bmin, bmax) = part_box(3, SkinLayer::Base, 0.0, false, ArmModel::Slim);
        let (omin, omax) = part_box(3, SkinLayer::Overlay, 0.0, false, ArmModel::Slim);
        approx(bmin[0] - omin[0], 0.015625);
        approx(omax[0] - bmax[0], 0.015625);
        let (apart, _) = part_box(3, SkinLayer::Base, 1.0, false, ArmModel::Slim);
        approx(apart[0] - bmin[0], 0.25);
    }

    #[test]
    fn out_of_range_part_is_a_zero_box_on_both_models() {
        for arm in [ArmModel::Classic, ArmModel::Slim] {
            let (min, max) = part_box(99, SkinLayer::Base, 0.0, false, arm);
            assert_eq!((min, max), ([0.0; 3], [0.0; 3]));
        }
    }
}

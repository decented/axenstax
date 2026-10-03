//! Standard Minecraft 64x64 "classic" skin box-unwrap UV table.
//!
//! Each player ModelPart face maps to a sub-rectangle of the 64x64 skin atlas.
//! Coordinates here are returned in 0..1 (atlas space) for direct use as vertex
//! UVs. `base` = the solid body layer; `overlay` = the second hat/jacket layer.
//!
//! Face index order matches ModelPart.tex_faces: [+x,-x,+y,-y,+z,-z].
//! GROUND TRUTH (PLAYER_MODEL head, entity_model.rs): the avatar FACE is on
//! -Z = index 5, the BACK on +Z = index 4. So index 5 = the "front" tile and
//! index 4 = the "back" tile for every part. +X (index 0) = character's right.

/// A UV rectangle in 0..1 atlas space: [u0, v0, u1, v1].
pub type UvRect = [f32; 4];

const ATLAS: f32 = 64.0;

/// Which player arm shape a skin is drawn on — Minecraft's two standard models.
///
/// `Classic` ("Steve") has 4-px-wide arms; `Slim` ("Alex") has 3-px-wide arms.
/// The difference is ONE pixel taken off the OUTER edge of each arm (the inner
/// edge stays flush against the torso), plus a 0.5-px drop of the whole arm —
/// and, on the atlas, narrower top/bottom/front/back arm tiles with the tiles
/// after the front one shifted 1 px left. Everything else — head, body, legs —
/// is byte-for-byte identical, which is why a skin PNG is valid for both and
/// only the model choice differs.
///
/// Threaded explicitly through every geometry/UV entry point rather than held
/// in a global: the wardrobe stores it per entry, so two avatars on screen can
/// legitimately disagree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ArmModel {
    /// 4-px arms — Minecraft's "Steve"/Classic model. The default everywhere.
    #[default]
    Classic,
    /// 3-px arms — Minecraft's "Alex"/Slim model.
    Slim,
}

impl ArmModel {
    /// `true` for the 3-px-arm model. Reads better than a `matches!` at call sites.
    pub fn is_slim(self) -> bool {
        matches!(self, ArmModel::Slim)
    }

    /// The name Minecraft's own skin picker uses, so the export help can tell a
    /// kid exactly which button to press on minecraft.net.
    pub fn label(self) -> &'static str {
        match self {
            ArmModel::Classic => "Classic",
            ArmModel::Slim => "Slim",
        }
    }

    /// From the Mojang profile's `metadata.model` flag (`mc_import`'s `slim`).
    pub fn from_slim_flag(slim: bool) -> Self {
        if slim { ArmModel::Slim } else { ArmModel::Classic }
    }
}

/// Arm-part indices in the PLAYER_MODEL order shared by this whole stack.
const ARM_L: usize = 2;
const ARM_R: usize = 3;

/// Slim (3-px) arm face rects, base layer, in the same
/// `[+X, -X, +Y top, -Y bottom, +Z back, -Z front]` order as `base_faces`.
///
/// Derived from Minecraft's box unwrap for a `w=3, d=4, h=12` cuboid at the
/// arm's atlas origin: the strip runs `side(d) | front(w) | side(d) | back(w)`,
/// so narrowing `w` from 4 to 3 shrinks the front/back/top/bottom tiles to 3
/// and pulls everything after the front tile 1 px left. The two ±X side faces
/// stay 4 wide (they show the arm's DEPTH, which is unchanged).
///
/// Left arm origin (32,48); right arm origin (40,16).
const SLIM_ARM_L_BASE: [UvRect; 6] = [
    // +X (inner, against the torso) — 4 wide, origin unmoved.
    px(32, 52, 4, 12),
    // -X (outer) — 4 wide, pulled 1 px left (40 → 39).
    px(39, 52, 4, 12),
    px(36, 48, 3, 4),  // +Y top — 3 wide
    px(39, 48, 3, 4),  // -Y bottom — 3 wide, 1 px left (40 → 39)
    px(43, 52, 3, 12), // +Z back — 3 wide, 1 px left (44 → 43)
    px(36, 52, 3, 12), // -Z front — 3 wide, origin unmoved
];
const SLIM_ARM_R_BASE: [UvRect; 6] = [
    px(40, 20, 4, 12), // +X (outer) — 4 wide, origin unmoved
    px(47, 20, 4, 12), // -X (inner) — 4 wide, 1 px left (48 → 47)
    px(44, 16, 3, 4),  // +Y top
    px(47, 16, 3, 4),  // -Y bottom — 1 px left (48 → 47)
    px(51, 20, 3, 12), // +Z back — 1 px left (52 → 51)
    px(44, 20, 3, 12), // -Z front
];

/// Shift a face rect by `(dx, dy)` atlas pixels — how the overlay tables are
/// built from the base ones (left sleeve +x16, right sleeve +y16), so the slim
/// sleeve rects can never drift from the slim body rects.
const fn shifted(r: UvRect, dx: f32, dy: f32) -> UvRect {
    [
        r[0] + dx / ATLAS,
        r[1] + dy / ATLAS,
        r[2] + dx / ATLAS,
        r[3] + dy / ATLAS,
    ]
}

/// Convert a pixel rect (x,y,w,h) on the 64x64 atlas to a 0..1 UvRect.
const fn px(x: u32, y: u32, w: u32, h: u32) -> UvRect {
    [
        x as f32 / ATLAS,
        y as f32 / ATLAS,
        (x + w) as f32 / ATLAS,
        (y + h) as f32 / ATLAS,
    ]
}

/// The 6 part-groups in PLAYER_MODEL order: head, body, arm_l, arm_r, leg_l, leg_r.
/// Each entry is the 6 face rects in tex_faces order [+x,-x,+y,-y,+z,-z].
/// Classic 64x64 layout. (8x8x8 head; 8x12x4 body; 4x12x4 limbs.)
pub fn base_faces(arm: ArmModel) -> [[UvRect; 6]; 6] {
    let mut t = classic_base_faces();
    if arm.is_slim() {
        t[ARM_L] = SLIM_ARM_L_BASE;
        t[ARM_R] = SLIM_ARM_R_BASE;
    }
    t
}

/// The classic (4-px arm) base table — the historical hand-written rects. Slim
/// patches only the two arm rows on top of this, so head/body/legs have exactly
/// one definition.
fn classic_base_faces() -> [[UvRect; 6]; 6] {
    // Order per part = [+X right, -X left, +Y top, -Y bottom, +Z BACK, -Z FRONT].
    // (-Z is the face — ground truth from PLAYER_MODEL head tex_faces.)
    [
        // Head — block (0,0), 8x8x8.
        // R(0,8) L(16,8) T(8,0) B(16,0) back(24,8) front(8,8)
        [px(0,8,8,8), px(16,8,8,8), px(8,0,8,8), px(16,0,8,8), px(24,8,8,8), px(8,8,8,8)],
        // Body — block (16,16), 8w x 12h x 4d.
        // R(16,20) L(28,20) T(20,16) B(28,16) back(32,20) front(20,20)
        [px(16,20,4,12), px(28,20,4,12), px(20,16,8,4), px(28,16,8,4), px(32,20,8,12), px(20,20,8,12)],
        // Left arm — block (32,48), 4x12x4.
        // R(32,52) L(40,52) T(36,48) B(40,48) back(44,52) front(36,52)
        [px(32,52,4,12), px(40,52,4,12), px(36,48,4,4), px(40,48,4,4), px(44,52,4,12), px(36,52,4,12)],
        // Right arm — block (40,16), 4x12x4.
        // R(40,20) L(48,20) T(44,16) B(48,16) back(52,20) front(44,20)
        [px(40,20,4,12), px(48,20,4,12), px(44,16,4,4), px(48,16,4,4), px(52,20,4,12), px(44,20,4,12)],
        // Left leg — block (16,48), 4x12x4.
        // R(16,52) L(24,52) T(20,48) B(24,48) back(28,52) front(20,52)
        [px(16,52,4,12), px(24,52,4,12), px(20,48,4,4), px(24,48,4,4), px(28,52,4,12), px(20,52,4,12)],
        // Right leg — block (0,16), 4x12x4.
        // R(0,20) L(8,20) T(4,16) B(8,16) back(12,20) front(4,20)
        [px(0,20,4,12), px(8,20,4,12), px(4,16,4,4), px(8,16,4,4), px(12,20,4,12), px(4,20,4,12)],
    ]
}

/// Overlay (hat/jacket/sleeve/leg2) face rects, same order.
pub fn overlay_faces(arm: ArmModel) -> [[UvRect; 6]; 6] {
    let mut t = classic_overlay_faces();
    if arm.is_slim() {
        // Same shift the classic table applies: left sleeve = left-arm base
        // +x16, right sleeve = right-arm base +y16.
        t[ARM_L] = SLIM_ARM_L_BASE.map(|r| shifted(r, 16.0, 0.0));
        t[ARM_R] = SLIM_ARM_R_BASE.map(|r| shifted(r, 0.0, 16.0));
    }
    t
}

fn classic_overlay_faces() -> [[UvRect; 6]; 6] {
    // Same face order as base_faces. Each part's overlay box is its base box
    // shifted to the 2nd-layer region: head +x32 (hat); body +y16 (jacket);
    // right-arm +y16; left-arm +x16; right-leg +y16; left-leg -x16.
    [
        // Head hat — head base +x32.
        [px(32,8,8,8), px(48,8,8,8), px(40,0,8,8), px(48,0,8,8), px(56,8,8,8), px(40,8,8,8)],
        // Body jacket — body base +y16.
        [px(16,36,4,12), px(28,36,4,12), px(20,32,8,4), px(28,32,8,4), px(32,36,8,12), px(20,36,8,12)],
        // Left-arm sleeve — left-arm base +x16.
        [px(48,52,4,12), px(56,52,4,12), px(52,48,4,4), px(56,48,4,4), px(60,52,4,12), px(52,52,4,12)],
        // Right-arm sleeve — right-arm base +y16.
        [px(40,36,4,12), px(48,36,4,12), px(44,32,4,4), px(48,32,4,4), px(52,36,4,12), px(44,36,4,12)],
        // Left-leg layer — left-leg base -x16.
        [px(0,52,4,12), px(8,52,4,12), px(4,48,4,4), px(8,48,4,4), px(12,52,4,12), px(4,52,4,12)],
        // Right-leg layer — right-leg base +y16.
        [px(0,36,4,12), px(8,36,4,12), px(4,32,4,4), px(8,32,4,4), px(12,36,4,12), px(4,36,4,12)],
    ]
}

/// Which skin layer a paint operation targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkinLayer {
    Base,
    Overlay,
}

/// Reverse of the box-unwrap: a fractional position `(frac_u, frac_v)` in 0..1
/// across a part's box-face maps to the exact `(x, y)` texel in the 64×64 atlas
/// for the chosen layer. `None` when `part` or `face` is out of range.
///
/// This is the per-pixel hit-mapping the 3D painter uses: ray → (part, face,
/// fractional u/v) → here → a single pixel in the skin buffer.
pub fn texel_for(
    part: usize,
    face: usize,
    frac_u: f32,
    frac_v: f32,
    layer: SkinLayer,
    arm: ArmModel,
) -> Option<(u32, u32)> {
    if part >= 6 || face >= 6 {
        return None;
    }
    let table = match layer {
        SkinLayer::Base => base_faces(arm),
        SkinLayer::Overlay => overlay_faces(arm),
    };
    let [u0, v0, u1, v1] = table[part][face];
    let fu = frac_u.clamp(0.0, 1.0);
    let fv = frac_v.clamp(0.0, 1.0);
    let ax = (u0 + fu * (u1 - u0)) * ATLAS;
    let ay = (v0 + fv * (v1 - v0)) * ATLAS;
    // Clamp to the rect's last interior pixel so frac==1.0 never spills over.
    let x = (ax as u32).min((u1 * ATLAS) as u32 - 1);
    let y = (ay as u32).min((v1 * ATLAS) as u32 - 1);
    Some((x, y))
}

/// The half-open pixel bounds `(x0, y0, x1, y1)` of a face's atlas rect — the
/// region a brush/fill must stay inside so it can't bleed into a neighbouring
/// UV island. Mirrors `texel_for`'s rect; `x1`/`y1` are exclusive. `None` when
/// `part` or `face` is out of range.
pub fn face_rect_px(
    part: usize,
    face: usize,
    layer: SkinLayer,
    arm: ArmModel,
) -> Option<(u32, u32, u32, u32)> {
    if part >= 6 || face >= 6 {
        return None;
    }
    let table = match layer {
        SkinLayer::Base => base_faces(arm),
        SkinLayer::Overlay => overlay_faces(arm),
    };
    let [u0, v0, u1, v1] = table[part][face];
    Some((
        (u0 * ATLAS) as u32,
        (v0 * ATLAS) as u32,
        (u1 * ATLAS) as u32,
        (v1 * ATLAS) as u32,
    ))
}

/// Mirror a hit across the avatar's left/right (X=0) plane for symmetric
/// painting: swap left/right limbs, swap the +X/−X faces, and flip `frac_u`
/// (front/back/top/bottom faces keep their face index, u flips). `frac_v`
/// is unchanged by an X-mirror, so the caller passes it through. Pure + tested.
///
/// Takes no [`ArmModel`]: this is pure index/fraction arithmetic, and the two
/// arms are mirror images of each other under BOTH models — the left arm's +X
/// face and the right arm's −X face are 4×12 on classic AND on slim, the two
/// fronts are 4×12 on classic and 3×12 on slim, and so on down the six faces.
/// The mapping is therefore identical and the mirrored fraction lands inside
/// the mirrored rect either way. `mirror_hit_stays_inside_the_slim_rect` is the
/// test that holds that honest rather than leaving it as a claim.
pub fn mirror_hit(part: usize, face: usize, frac_u: f32) -> (usize, usize, f32) {
    let part_m = match part {
        2 => 3,
        3 => 2, // arm_l ↔ arm_r
        4 => 5,
        5 => 4, // leg_l ↔ leg_r
        other => other, // head/body onto themselves
    };
    let face_m = match face {
        0 => 1,
        1 => 0, // +X ↔ −X
        other => other, // +Y/−Y/+Z/−Z unchanged under an X-mirror
    };
    (part_m, face_m, (1.0 - frac_u).clamp(0.0, 1.0))
}

/// One-shot migration primitive (v0.2.19 box-unwrap-fix migration): mirror
/// every base + overlay face rect of a 64×64 RGBA skin buffer horizontally
/// (reverse the texel columns *inside* each rect; rows are untouched). This
/// undoes the pre-v0.2.18 backwards-u box-unwrap on a skin that was painted
/// under it, without touching pixels outside every rect (the atlas has a few
/// unused corners, e.g. (0,0)..(8,8)).
///
/// No-op if `buf` isn't exactly 64×64×4 bytes. Applying this function twice
/// is the identity (each rect-row is reversed in place, and reversing twice
/// restores the original order).
///
/// CLASSIC rects only, deliberately: this is the one-shot v1 → v2 migration for
/// skins painted before v0.2.18, and slim arms did not exist then. Every slim
/// arm rect lies inside the union of the classic arm rects anyway, so nothing a
/// slim skin uses falls outside the region scanned here.
pub fn mirror_every_face(buf: &mut [u8]) {
    const SIDE: usize = 64;
    if buf.len() != SIDE * SIDE * 4 {
        return;
    }
    for table in [base_faces(ArmModel::Classic), overlay_faces(ArmModel::Classic)] {
        for part in table {
            for rect in part {
                let x0 = (rect[0] * ATLAS).round() as usize;
                let y0 = (rect[1] * ATLAS).round() as usize;
                let x1 = ((rect[2] * ATLAS).round() as usize).min(SIDE);
                let y1 = ((rect[3] * ATLAS).round() as usize).min(SIDE);
                for y in y0..y1 {
                    let mut left = x0;
                    let mut right = x1.saturating_sub(1);
                    while left < right {
                        let li = (y * SIDE + left) * 4;
                        let ri = (y * SIDE + right) * 4;
                        for c in 0..4 {
                            buf.swap(li + c, ri + c);
                        }
                        left += 1;
                        right -= 1;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_rects_within_unit_square() {
        for arm in [ArmModel::Classic, ArmModel::Slim] {
        for table in [base_faces(arm), overlay_faces(arm)] {
            for part in table {
                for r in part {
                    assert!(r[0] >= 0.0 && r[1] >= 0.0 && r[2] <= 1.0 && r[3] <= 1.0, "rect {r:?} out of 0..1");
                    assert!(r[2] > r[0] && r[3] > r[1], "rect {r:?} has non-positive area");
                }
            }
        }
        }
    }

    #[test]
    fn six_parts_six_faces() {
        assert_eq!(base_faces(ArmModel::Classic).len(), 6);
        assert_eq!(base_faces(ArmModel::Classic)[0].len(), 6);
        assert_eq!(overlay_faces(ArmModel::Classic).len(), 6);
    }

    #[test]
    fn head_face_is_minus_z_index5() {
        // GROUND TRUTH: PLAYER_MODEL puts the face (HEAD_FRONT) on -Z = index 5.
        assert_eq!(base_faces(ArmModel::Classic)[0][5], [8.0 / 64.0, 8.0 / 64.0, 16.0 / 64.0, 16.0 / 64.0], "head front must be -Z (index 5) = (8,8) tile");
        assert_eq!(base_faces(ArmModel::Classic)[0][4], [24.0 / 64.0, 8.0 / 64.0, 32.0 / 64.0, 16.0 / 64.0], "head back must be +Z (index 4) = (24,8) tile");
        assert_eq!(base_faces(ArmModel::Classic)[1][5], [20.0 / 64.0, 20.0 / 64.0, 28.0 / 64.0, 32.0 / 64.0], "body front must be -Z (index 5) = (20,20) chest tile");
    }

    #[test]
    fn overlay_rects_within_bounds() {
        for part in overlay_faces(ArmModel::Classic) {
            for r in part {
                assert!(r[0] >= 0.0 && r[1] >= 0.0 && r[2] <= 1.0 && r[3] <= 1.0, "overlay rect {r:?} out of 0..1");
                assert!(r[2] > r[0] && r[3] > r[1], "overlay rect {r:?} has non-positive area");
            }
        }
    }

    #[test]
    fn texel_for_head_front_center() {
        // Head (part 0) front is -Z = face index 5, base rect (8,8,8,8) → pixels
        // x in 8..16, y in 8..16. The face centre maps to (12,12).
        let (x, y) = texel_for(0, 5, 0.5, 0.5, SkinLayer::Base, ArmModel::Classic).expect("valid part/face");
        assert_eq!((x, y), (12, 12));
    }

    #[test]
    fn texel_for_lands_inside_every_rect() {
        for arm in [ArmModel::Classic, ArmModel::Slim] {
        for (layer, table) in [
            (SkinLayer::Base, base_faces(arm)),
            (SkinLayer::Overlay, overlay_faces(arm)),
        ] {
            for part in 0..6 {
                for face in 0..6 {
                    let r = table[part][face];
                    let (x0, y0) = ((r[0] * ATLAS) as u32, (r[1] * ATLAS) as u32);
                    let (x1, y1) = ((r[2] * ATLAS) as u32, (r[3] * ATLAS) as u32);
                    // Sample the four corners + centre; all must fall in [x0,x1)×[y0,y1).
                    for &(fu, fv) in &[(0.0, 0.0), (0.999, 0.0), (0.0, 0.999), (0.999, 0.999), (0.5, 0.5)] {
                        let (x, y) = texel_for(part, face, fu, fv, layer, arm).unwrap();
                        assert!(x >= x0 && x < x1, "{arm:?} part {part} face {face} x {x} not in [{x0},{x1})");
                        assert!(y >= y0 && y < y1, "{arm:?} part {part} face {face} y {y} not in [{y0},{y1})");
                    }
                }
            }
        }
        }
    }

    #[test]
    fn texel_for_rejects_out_of_range() {
        assert!(texel_for(6, 0, 0.5, 0.5, SkinLayer::Base, ArmModel::Classic).is_none());
        assert!(texel_for(0, 6, 0.5, 0.5, SkinLayer::Base, ArmModel::Classic).is_none());
    }

    #[test]
    fn face_rect_px_head_front_base() {
        // Head (part 0) front (-Z = face 5) base rect is pixel (8,8) size 8x8 →
        // half-open bounds (8, 8, 16, 16).
        assert_eq!(face_rect_px(0, 5, SkinLayer::Base, ArmModel::Classic), Some((8, 8, 16, 16)));
    }

    #[test]
    fn face_rect_px_contains_texel_for() {
        // Every texel texel_for produces for a face must lie inside that face's
        // face_rect_px bounds — they describe the same rect.
        for arm in [ArmModel::Classic, ArmModel::Slim] {
        for layer in [SkinLayer::Base, SkinLayer::Overlay] {
            for part in 0..6 {
                for face in 0..6 {
                    let (x0, y0, x1, y1) = face_rect_px(part, face, layer, arm).unwrap();
                    for &(fu, fv) in &[(0.0, 0.0), (0.999, 0.999), (0.5, 0.5)] {
                        let (x, y) = texel_for(part, face, fu, fv, layer, arm).unwrap();
                        assert!(x >= x0 && x < x1, "{arm:?} part {part} face {face}: x {x} not in [{x0},{x1})");
                        assert!(y >= y0 && y < y1, "{arm:?} part {part} face {face}: y {y} not in [{y0},{y1})");
                    }
                }
            }
        }
        }
    }

    #[test]
    fn face_rect_px_rejects_out_of_range() {
        assert_eq!(face_rect_px(6, 0, SkinLayer::Base, ArmModel::Classic), None);
        assert_eq!(face_rect_px(0, 6, SkinLayer::Base, ArmModel::Classic), None);
    }

    #[test]
    fn mirror_swaps_left_right_limbs_and_flips_u() {
        // arm_l(2) ↔ arm_r(3), leg_l(4) ↔ leg_r(5); front face stays a front face;
        // u flips (1 − u). Head(0)/body(1) mirror onto themselves with u flipped.
        let (p, f, u) = mirror_hit(2, 5, 0.2);
        assert_eq!((p, f), (3, 5));
        assert!((u - 0.8).abs() < 1e-6);
        let (p, _f, _u) = mirror_hit(0, 5, 0.5);
        assert_eq!(p, 0, "head mirrors onto itself");
        // +X face ↔ −X face when mirroring across the X plane.
        let (_p, f, _u) = mirror_hit(1, 0, 0.3);
        assert_eq!(f, 1, "+X (0) becomes −X (1)");
    }

    /// A 64×64 RGBA buffer with a distinct value at every byte, so any
    /// mispositioned swap during flip/flip-back would show up.
    fn varied_buffer() -> Vec<u8> {
        (0..64 * 64 * 4).map(|i| (i % 256) as u8).collect()
    }

    #[test]
    fn mirror_every_face_twice_is_identity() {
        let original = varied_buffer();
        let mut buf = original.clone();
        mirror_every_face(&mut buf);
        assert_ne!(buf, original, "a single flip must actually change a varied buffer");
        mirror_every_face(&mut buf);
        assert_eq!(buf, original, "flipping twice restores the original buffer");
    }

    #[test]
    fn mirror_every_face_swaps_rect_left_edge_to_right_edge_base() {
        // Base head front (part 0, face 5) is px(8,8,8,8): x0=8, x1=16, y in [8,16).
        let (x0, y0, x1, _y1) = face_rect_px(0, 5, SkinLayer::Base, ArmModel::Classic).unwrap();
        let mut buf = vec![0u8; 64 * 64 * 4];
        let idx = |x: usize, y: usize| (y * 64 + x) * 4;
        let src = idx(x0 as usize, y0 as usize);
        buf[src..src + 4].copy_from_slice(&[9, 8, 7, 6]);

        mirror_every_face(&mut buf);

        let dst = idx(x1 as usize - 1, y0 as usize);
        assert_eq!(&buf[dst..dst + 4], &[9, 8, 7, 6], "left-edge marker lands on the right edge");
        assert_eq!(&buf[src..src + 4], &[0, 0, 0, 0], "left edge no longer holds the marker");
    }

    #[test]
    fn mirror_every_face_swaps_rect_left_edge_to_right_edge_overlay() {
        // Overlay head hat (part 0, face 0) is px(32,8,8,8): x0=32, x1=40, y in [8,16).
        let (x0, y0, x1, _y1) = face_rect_px(0, 0, SkinLayer::Overlay, ArmModel::Classic).unwrap();
        let mut buf = vec![0u8; 64 * 64 * 4];
        let idx = |x: usize, y: usize| (y * 64 + x) * 4;
        let src = idx(x0 as usize, y0 as usize);
        buf[src..src + 4].copy_from_slice(&[1, 2, 3, 4]);

        mirror_every_face(&mut buf);

        let dst = idx(x1 as usize - 1, y0 as usize);
        assert_eq!(&buf[dst..dst + 4], &[1, 2, 3, 4], "left-edge marker lands on the right edge");
    }

    #[test]
    fn mirror_every_face_leaves_unused_corner_untouched() {
        // (0,0) must fall outside every base + overlay rect — confirm that,
        // don't assume it.
        for layer in [SkinLayer::Base, SkinLayer::Overlay] {
            let table = match layer {
                SkinLayer::Base => base_faces(ArmModel::Classic),
                SkinLayer::Overlay => overlay_faces(ArmModel::Classic),
            };
            for part in table {
                for r in part {
                    let (x0, y0, x1, y1) = (
                        (r[0] * ATLAS).round() as u32,
                        (r[1] * ATLAS).round() as u32,
                        (r[2] * ATLAS).round() as u32,
                        (r[3] * ATLAS).round() as u32,
                    );
                    assert!(!(x0 <= 0 && 0 < x1 && y0 <= 0 && 0 < y1), "expected (0,0) outside rect {r:?}");
                }
            }
        }

        let mut buf = vec![0u8; 64 * 64 * 4];
        buf[0..4].copy_from_slice(&[11, 22, 33, 44]);
        mirror_every_face(&mut buf);
        assert_eq!(&buf[0..4], &[11, 22, 33, 44], "texel outside every rect must be untouched");
    }

    #[test]
    fn mirror_every_face_does_not_change_clothes_layer_flag() {
        let def = crate::texture_gen::default_skin_rgba();
        let before = crate::skin_layers::clothes_layer_has_content(&def);
        let mut flipped = def.clone();
        mirror_every_face(&mut flipped);
        let after = crate::skin_layers::clothes_layer_has_content(&flipped);
        assert_eq!(before, after, "flipping must not change whether the overlay layer has content");
    }

    #[test]
    fn mirror_every_face_is_noop_on_wrong_sized_buffer() {
        let mut buf = vec![7u8; 16];
        mirror_every_face(&mut buf);
        assert_eq!(buf, vec![7u8; 16]);
    }

    // ── Slim ("Alex", 3-px arms) ─────────────────────────────────────────────

    /// Pixel rect `(x, y, w, h)` of a face, from the 0..1 table.
    fn rect_px(arm: ArmModel, layer: SkinLayer, part: usize, face: usize) -> (u32, u32, u32, u32) {
        let (x0, y0, x1, y1) = face_rect_px(part, face, layer, arm).unwrap();
        (x0, y0, x1 - x0, y1 - y0)
    }

    #[test]
    fn arm_model_defaults_to_classic() {
        assert_eq!(ArmModel::default(), ArmModel::Classic);
        assert!(!ArmModel::Classic.is_slim());
        assert!(ArmModel::Slim.is_slim());
        assert_eq!(ArmModel::Classic.label(), "Classic");
        assert_eq!(ArmModel::Slim.label(), "Slim");
        assert_eq!(ArmModel::from_slim_flag(true), ArmModel::Slim);
        assert_eq!(ArmModel::from_slim_flag(false), ArmModel::Classic);
    }

    #[test]
    fn slim_right_arm_front_is_three_wide_at_x44() {
        // THE headline difference. The right arm's front (-Z = face 5) is the
        // tile a kid actually paints; on slim it is 3 texels wide, still
        // starting at atlas x=44 (the front tile's origin never moves — the
        // pixel comes off the far side).
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 3, 5), (44, 20, 3, 12));
        assert_eq!(rect_px(ArmModel::Classic, SkinLayer::Base, 3, 5), (44, 20, 4, 12));
        // Left arm front likewise: 3 wide, origin unmoved at x=36.
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 2, 5), (36, 52, 3, 12));
        assert_eq!(rect_px(ArmModel::Classic, SkinLayer::Base, 2, 5), (36, 52, 4, 12));
    }

    #[test]
    fn slim_inner_face_origin_is_47_and_39() {
        // The 4-wide side tiles keep their WIDTH (they show the arm's unchanged
        // depth) but the one AFTER the front tile slides 1 px left because the
        // front tile shrank. Right arm: -X (inner) 48 → 47. Left arm: -X
        // (outer) 40 → 39. The other side tile is before the front and doesn't move.
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 3, 1), (47, 20, 4, 12));
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 3, 0), (40, 20, 4, 12));
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 2, 1), (39, 52, 4, 12));
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 2, 0), (32, 52, 4, 12));
    }

    #[test]
    fn slim_top_bottom_and_back_arm_tiles() {
        // Right arm (origin 40,16): top (44,16,3,4), bottom (47,16,3,4),
        // back (51,20,3,12). Left arm (origin 32,48): top (36,48,3,4),
        // bottom (39,48,3,4), back (43,52,3,12).
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 3, 2), (44, 16, 3, 4));
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 3, 3), (47, 16, 3, 4));
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 3, 4), (51, 20, 3, 12));
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 2, 2), (36, 48, 3, 4));
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 2, 3), (39, 48, 3, 4));
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Base, 2, 4), (43, 52, 3, 12));
    }

    #[test]
    fn slim_sleeves_are_the_slim_arm_rects_shifted_like_the_classic_ones() {
        // Right sleeve = right-arm base +y16; left sleeve = left-arm base +x16.
        // Same shifts the classic table uses, so a slim sleeve can never drift
        // from the slim arm under it.
        for face in 0..6 {
            let (bx, by, bw, bh) = rect_px(ArmModel::Slim, SkinLayer::Base, 3, face);
            assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Overlay, 3, face), (bx, by + 16, bw, bh));
            let (lx, ly, lw, lh) = rect_px(ArmModel::Slim, SkinLayer::Base, 2, face);
            assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Overlay, 2, face), (lx + 16, ly, lw, lh));
        }
        // Pinned corner: the left sleeve's back tile is the rightmost thing on
        // the atlas and must still fit inside 64 px (43+16=59, +3 = 62).
        assert_eq!(rect_px(ArmModel::Slim, SkinLayer::Overlay, 2, 4), (59, 52, 3, 12));
    }

    #[test]
    fn slim_changes_only_the_arms() {
        // Head, body and both legs are byte-identical between the models —
        // which is why one PNG serves both and only the model choice differs.
        let (cb, sb) = (base_faces(ArmModel::Classic), base_faces(ArmModel::Slim));
        let (co, so) = (overlay_faces(ArmModel::Classic), overlay_faces(ArmModel::Slim));
        for part in [0usize, 1, 4, 5] {
            assert_eq!(cb[part], sb[part], "part {part} base must not change with the arm model");
            assert_eq!(co[part], so[part], "part {part} overlay must not change with the arm model");
        }
        assert_ne!(cb[2], sb[2], "left arm must differ");
        assert_ne!(cb[3], sb[3], "right arm must differ");
    }

    #[test]
    fn texel_for_never_leaves_the_slim_front_rect() {
        // The 3-wide front tiles are where a rounding slip would show first:
        // sweep the full fraction range on both arms, both layers.
        for part in [2usize, 3] {
            for layer in [SkinLayer::Base, SkinLayer::Overlay] {
                let (x0, y0, x1, y1) = face_rect_px(part, 5, layer, ArmModel::Slim).unwrap();
                assert_eq!(x1 - x0, 3, "slim front is 3 texels wide");
                for i in 0..=200 {
                    let f = i as f32 / 200.0;
                    let (x, y) = texel_for(part, 5, f, f, layer, ArmModel::Slim).unwrap();
                    assert!(x >= x0 && x < x1, "u={f} landed on x={x}, outside [{x0},{x1})");
                    assert!(y >= y0 && y < y1, "v={f} landed on y={y}, outside [{y0},{y1})");
                }
            }
        }
    }

    #[test]
    fn mirror_hit_stays_inside_the_slim_rect() {
        // Symmetric painting on a slim skin: every face of one arm must mirror
        // onto the matching face of the other, and the mirrored texel must land
        // inside THAT face's (slim) rect — not the classic one it replaced.
        for face in 0..6 {
            for layer in [SkinLayer::Base, SkinLayer::Overlay] {
                for &u in &[0.0f32, 0.01, 0.5, 0.99, 1.0] {
                    let (mp, mf, mu) = mirror_hit(3, face, u);
                    assert_eq!(mp, 2, "right arm mirrors to the left arm");
                    assert_eq!(mf, if face == 0 { 1 } else if face == 1 { 0 } else { face });
                    let (x0, y0, x1, y1) = face_rect_px(mp, mf, layer, ArmModel::Slim).unwrap();
                    let (x, y) = texel_for(mp, mf, mu, 0.5, layer, ArmModel::Slim).unwrap();
                    assert!(x >= x0 && x < x1, "mirrored u={u} → x={x} outside [{x0},{x1})");
                    assert!(y >= y0 && y < y1);
                }
            }
            // …and back again: mirroring twice is the identity.
            let (p2, f2, u2) = mirror_hit(3, face, 0.2);
            let (p3, f3, u3) = mirror_hit(p2, f2, u2);
            assert_eq!((p3, f3), (3, face));
            assert!((u3 - 0.2).abs() < 1e-6);
        }
    }

    #[test]
    fn slim_arm_rects_stay_inside_the_classic_arm_island() {
        // Load-bearing for the passes that still scan CLASSIC rects on purpose
        // (`skin_layers::clothes_layer_has_content`, `skin_paint::heal_base_opacity`,
        // `mirror_every_face`): every slim arm texel must be covered by the
        // union of that arm's classic rects, so those scans are a superset and
        // never miss a slim skin's pixels.
        for layer in [SkinLayer::Base, SkinLayer::Overlay] {
            for part in [2usize, 3] {
                let classic: Vec<_> = (0..6)
                    .map(|f| face_rect_px(part, f, layer, ArmModel::Classic).unwrap())
                    .collect();
                for face in 0..6 {
                    let (x0, y0, x1, y1) = face_rect_px(part, face, layer, ArmModel::Slim).unwrap();
                    for y in y0..y1 {
                        for x in x0..x1 {
                            assert!(
                                classic.iter().any(|&(cx0, cy0, cx1, cy1)| {
                                    x >= cx0 && x < cx1 && y >= cy0 && y < cy1
                                }),
                                "slim texel ({x},{y}) on part {part} face {face} is outside every classic rect"
                            );
                        }
                    }
                }
            }
        }
    }
}

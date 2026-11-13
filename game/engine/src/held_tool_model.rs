//! Held tool shapes: every [`ToolType`] drawn as its own small box model,
//! with the tier's material on the head or the blade and a stick handle
//! (Spec 03 §5.10, "Held tools").
//!
//! Why boxes and not Minecraft's extruded icon: a tool has no per-type icon
//! layer. Its tier layer (`entity_model::tool_texture`) is one generic tile for
//! every type (a diagonal handle and a wedge on a flat ground), and the
//! inventory and hotbar draw a tool as that tile plus a letter. There is nothing
//! per-type to extrude, and the web build's texture array is exactly full (512
//! layers), so there is no room for new art. The shapes are built from existing
//! layers only: the tier's own tile for the head, the stick for the handle, and
//! a few ingredient tiles (string, flint, leather, wool).
//!
//! **Flat colour.** Each box face carries ONE texel of a layer: the same uv on
//! all four corners, so the fragment's uv derivatives are zero (mip level 0)
//! and the face is that texel's colour, whatever the sampler's filter. The
//! tier's head colour is the wedge texel of the tier tile ([`tier_paint`]); the
//! others are the opaque texel nearest a tile's centre ([`centre_paint`]), which
//! the tests pin as opaque (a transparent texel would draw as a dark box).
//!
//! **Frame.** The caller's frame is `item_model`'s: the grip is the origin, +Y
//! is up the shaft, every model spans about y -0.31 .. 0.34 and x -0.25 .. 0.25
//! (the old slab was 0.6 tall). A model's one-sided feature (an axe's blade, a
//! hoe's head) points to +X for [`HandSide::Right`] and to -X for
//! [`HandSide::Left`]: built mirrored from the box centres and tilts, never by a
//! negative scale (which would flip the winding and the normals).
//!
//! **Where the shield goes.** [`tool_model`] is an exhaustive `match` with no
//! wildcard, so adding `ToolType::Shield` (slice 5 of the off-hand plan) fails to
//! compile here until it has a shape: a board from the planks tile with an iron
//! rim and boss, held by its back, raised in front of the arm, so it wants its own
//! pose in `entity_model` (the left arm's raised angle) and in `viewmodel` as well
//! as a model here.

use glam::{Mat3, Vec3};

use crate::crafting::{ToolMaterial, ToolType};
use crate::entity_model::{
    TEX_ITEM_LEATHER, TEX_ITEM_STICK, TEX_ITEM_STRING, TEX_ITEM_TOOL_DIAMOND, TEX_ITEM_TOOL_IRON,
    TEX_ITEM_TOOL_STONE, TEX_ITEM_TOOL_WOOD, TEX_ITEM_WOOL,
};
use crate::item_model::layer_mask;
use crate::mesh::Vertex;
use crate::viewmodel::HandSide;

/// Texels along a side of a tile.
const TILE: u32 = 16;

/// A flat colour: one texel of one texture layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paint {
    pub layer: u32,
    /// Texel (column, row) in the tile.
    pub texel: (u32, u32),
}

impl Paint {
    /// The uv at the middle of the texel.
    fn uv(self) -> [f32; 2] {
        [(self.texel.0 as f32 + 0.5) / TILE as f32, (self.texel.1 as f32 + 0.5) / TILE as f32]
    }
}

/// The opaque texel of `layer` nearest the middle of the tile (texel (8, 8) if the
/// layer is unknown): a representative colour of the layer's art.
pub fn centre_paint(layer: u32) -> Paint {
    let mask = layer_mask(layer);
    let mut best = ((8, 8), f32::INFINITY);
    for y in 0..TILE {
        for x in 0..TILE {
            if (mask[y as usize] >> x) & 1 == 1 {
                let d = (x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2);
                if d < best.1 {
                    best = ((x, y), d);
                }
            }
        }
    }
    Paint { layer, texel: best.0 }
}

/// The tier's head colour: the wedge texel of its tool tile (every tier tile
/// draws a diagonal handle and a head wedge in the tier's own colour on a darker
/// ground; texel (11, 5) is the wedge, `texture_gen::gen_tool_for_tier`). Satori
/// tools reuse the Satori gem icon, so the middle of that.
pub fn tier_paint(material: ToolMaterial) -> Paint {
    let tile = |layer| Paint { layer, texel: (11, 5) };
    match material {
        ToolMaterial::Wood => tile(TEX_ITEM_TOOL_WOOD),
        ToolMaterial::Stone => tile(TEX_ITEM_TOOL_STONE),
        ToolMaterial::Iron => tile(TEX_ITEM_TOOL_IRON),
        ToolMaterial::Diamond => tile(TEX_ITEM_TOOL_DIAMOND),
        ToolMaterial::Satori => centre_paint(crate::block::TEX_ITEM_SATORI),
    }
}

/// The handle: the stick tile's plain ground.
fn grip() -> Paint {
    Paint { layer: TEX_ITEM_STICK, texel: (3, 8) }
}

/// Steel: the iron tier's colour, whatever the tool's own tier (shears and a
/// flint-and-steel are iron by recipe, the sword's guard is metal).
fn metal() -> Paint {
    tier_paint(ToolMaterial::Iron)
}

/// One axis-aligned box, tilted about Z, before the hand's mirror.
struct Part {
    centre: (f32, f32),
    size: Vec3,
    /// Tilt about the box's own centre, degrees, counter-clockwise seen from +Z
    /// (the top moves toward -X for a positive angle, on the right hand).
    tilt_deg: f32,
    paint: Paint,
}

fn part(cx: f32, cy: f32, w: f32, h: f32, d: f32, tilt_deg: f32, paint: Paint) -> Part {
    Part { centre: (cx, cy), size: Vec3::new(w, h, d), tilt_deg, paint }
}

/// Emit one part, mirrored for `side`.
fn push_part(verts: &mut Vec<Vertex>, p: &Part, side: HandSide) {
    let s = side.sign();
    let rot = Mat3::from_rotation_z((p.tilt_deg * s).to_radians());
    let centre = Vec3::new(p.centre.0 * s, p.centre.1, 0.0);
    let h = p.size * 0.5;
    let corner = |sx: f32, sy: f32, sz: f32| centre + rot * Vec3::new(sx * h.x, sy * h.y, sz * h.z);
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
    // Same faces and winding as `item_model::push_cuboid`.
    let faces: [([f32; 3], [usize; 4]); 6] = [
        ([1.0, 0.0, 0.0], [1, 5, 6, 2]),
        ([-1.0, 0.0, 0.0], [4, 0, 3, 7]),
        ([0.0, 1.0, 0.0], [3, 2, 6, 7]),
        ([0.0, -1.0, 0.0], [4, 5, 1, 0]),
        ([0.0, 0.0, 1.0], [5, 4, 7, 6]),
        ([0.0, 0.0, -1.0], [0, 1, 2, 3]),
    ];
    let uv = p.paint.uv();
    for (n, idx) in faces {
        let normal = (rot * Vec3::from(n)).to_array();
        let [a, b, cc, d] = idx.map(|i| c[i]);
        for q in [a, b, cc, a, cc, d] {
            verts.push(Vertex {
                position: q.to_array(),
                normal,
                tex_layer: p.paint.layer,
                uv,
                light: Vertex::FULL_BRIGHT,
                sky_light: 0.0,
            });
        }
    }
}

/// The wooden shaft, grip end at the bottom.
fn shaft(len: f32, cy: f32) -> Part {
    part(0.0, cy, 0.07, len, 0.07, 0.0, grip())
}

/// The parts of `tool` in `material`, for the right hand (the caller mirrors the
/// left). Exhaustive: see the module doc for the shield.
fn parts(tool: ToolType, material: ToolMaterial) -> Vec<Part> {
    let head = tier_paint(material);
    let steel = metal();
    match tool {
        // A crossbar with the two tips bent down.
        ToolType::Pickaxe => vec![
            shaft(0.56, -0.03),
            part(0.0, 0.24, 0.46, 0.09, 0.10, 0.0, head),
            part(-0.205, 0.18, 0.07, 0.12, 0.10, 0.0, head),
            part(0.205, 0.18, 0.07, 0.12, 0.10, 0.0, head),
        ],
        // A block on the shaft with a long blade edge on one side.
        ToolType::Axe => vec![
            shaft(0.56, -0.03),
            part(0.10, 0.22, 0.20, 0.20, 0.10, 0.0, head),
            part(0.21, 0.21, 0.05, 0.26, 0.10, 0.0, head),
        ],
        // A long, narrow, pointed blade (three steps taper to the tip), a
        // crossguard wider than the blade, and a short grip. The blade fills
        // the hand envelope's height: grip -0.34..-0.235, guard to -0.195,
        // point at 0.35.
        ToolType::Sword => vec![
            part(0.0, -0.285, 0.06, 0.10, 0.06, 0.0, grip()),
            part(0.0, -0.215, 0.27, 0.04, 0.08, 0.0, steel),
            part(0.0, -0.005, 0.075, 0.38, 0.04, 0.0, head),
            part(0.0, 0.255, 0.055, 0.14, 0.035, 0.0, head),
            part(0.0, 0.335, 0.03, 0.02, 0.03, 0.0, head),
        ],
        // A broad flat blade on a long shaft with a T grip.
        ToolType::Shovel => vec![
            part(0.0, -0.06, 0.06, 0.50, 0.06, 0.0, grip()),
            part(0.0, -0.31, 0.15, 0.04, 0.06, 0.0, grip()),
            part(0.0, 0.20, 0.16, 0.24, 0.05, 0.0, head),
            part(0.0, 0.325, 0.11, 0.05, 0.05, 0.0, head),
        ],
        // A thin head reaching out to one side, with a short blade turned down.
        ToolType::Hoe => vec![
            shaft(0.56, -0.03),
            part(0.12, 0.27, 0.30, 0.06, 0.08, 0.0, head),
            part(0.245, 0.20, 0.06, 0.16, 0.08, 0.0, head),
        ],
        // A bent stave with a string and a grip wrap.
        ToolType::Bow => vec![
            part(0.06, 0.0, 0.05, 0.22, 0.06, 0.0, grip()),
            part(0.02, 0.205, 0.05, 0.22, 0.06, 21.0, grip()),
            part(0.02, -0.205, 0.05, 0.22, 0.06, -21.0, grip()),
            part(0.06, 0.0, 0.075, 0.09, 0.08, 0.0, Paint { layer: TEX_ITEM_LEATHER, texel: (8, 8) }),
            part(-0.03, 0.0, 0.015, 0.60, 0.015, 0.0, centre_paint(TEX_ITEM_STRING)),
        ],
        // A long, slightly leaning rod, a reel, and a line hanging from the
        // tip with a small red float and a hook below it.
        ToolType::FishingRod => {
            let tilt: f32 = 9.0;
            let (sin, cos) = tilt.to_radians().sin_cos();
            // The rod's tip (half its length from its centre, along the tilt).
            let tip = (0.0 - 0.32 * sin, 0.02 + 0.32 * cos);
            let line_len = 0.22;
            let line_cy = tip.1 - line_len / 2.0;
            vec![
                part(0.0, 0.02, 0.04, 0.66, 0.04, tilt, grip()),
                part(0.0, -0.19, 0.075, 0.15, 0.075, tilt, Paint { layer: TEX_ITEM_LEATHER, texel: (8, 8) }),
                part(0.065, -0.10, 0.075, 0.075, 0.075, 0.0, steel),
                part(tip.0, line_cy, 0.022, line_len, 0.022, 0.0, centre_paint(TEX_ITEM_STRING)),
                part(tip.0, tip.1 - 0.15, 0.05, 0.05, 0.05, 0.0, centre_paint(crate::entity_model::TEX_ITEM_RED_DYE)),
                part(tip.0, tip.1 - line_len - 0.015, 0.03, 0.03, 0.03, 0.0, steel),
            ]
        }
        // A fork, a grip and the band between the prongs.
        ToolType::Slingshot => vec![
            part(0.0, -0.13, 0.07, 0.30, 0.07, 0.0, grip()),
            part(-0.07, 0.135, 0.06, 0.26, 0.06, 20.0, grip()),
            part(0.07, 0.135, 0.06, 0.26, 0.06, -20.0, grip()),
            part(0.0, 0.215, 0.20, 0.035, 0.035, 0.0, Paint { layer: TEX_ITEM_LEATHER, texel: (8, 8) }),
            part(0.0, 0.17, 0.07, 0.06, 0.04, 0.0, Paint { layer: TEX_ITEM_LEATHER, texel: (8, 8) }),
        ],
        // Two crossed blades and two finger loops.
        ToolType::Shears => vec![
            part(0.0, 0.0, 0.065, 0.60, 0.03, 11.0, steel),
            part(0.0, 0.0, 0.065, 0.60, 0.03, -11.0, steel),
            part(0.075, -0.26, 0.09, 0.075, 0.045, 0.0, Paint { layer: TEX_ITEM_LEATHER, texel: (8, 8) }),
            part(-0.075, -0.26, 0.09, 0.075, 0.045, 0.0, Paint { layer: TEX_ITEM_LEATHER, texel: (8, 8) }),
        ],
        // A steel loop with a chunk of flint held above it.
        ToolType::FlintAndSteel => {
            let flint = centre_paint(crate::block::TEX_ITEM_FLINT);
            vec![
                part(0.0, -0.20, 0.27, 0.045, 0.06, 0.0, steel),
                part(0.115, -0.115, 0.045, 0.19, 0.06, 0.0, steel),
                part(-0.115, -0.115, 0.045, 0.19, 0.06, 0.0, steel),
                part(0.0, -0.02, 0.19, 0.045, 0.06, 0.0, steel),
                part(0.0, 0.11, 0.15, 0.21, 0.11, 0.0, flint),
                part(0.0, 0.25, 0.09, 0.07, 0.07, 0.0, flint),
            ]
        }
        // A tall block of rubber in a metal band.
        ToolType::Eraser => vec![
            part(0.0, 0.0, 0.15, 0.44, 0.10, 0.0, centre_paint(TEX_ITEM_WOOL)),
            part(0.0, 0.0, 0.17, 0.07, 0.12, 0.0, steel),
            part(0.0, -0.20, 0.17, 0.04, 0.12, 0.0, steel),
        ],
        // Off-hand release (slice 5) — a board of four planks (the oak planks tile's
        // light and dark texels), an iron rim round it and an iron boss on the +Z
        // face. The board lies in the X-Y plane, centred on the origin; the views
        // place it (`shield_pose`): upright and facing the camera in a first-person
        // hand, turned out on the forearm in a lowered third-person one, lifted in
        // front of the body when raised.
        ToolType::Shield => {
            let plank_light = Paint { layer: crate::block::TEX_OAK_PLANKS, texel: (8, 6) };
            let plank_dark = Paint { layer: crate::block::TEX_OAK_PLANKS, texel: (8, 8) };
            let mut v = Vec::new();
            for (i, y) in [-0.2325_f32, -0.0775, 0.0775, 0.2325].into_iter().enumerate() {
                v.push(part(0.0, y, 0.46, 0.155, 0.06, 0.0, if i % 2 == 0 { plank_light } else { plank_dark }));
            }
            // The rim: top and bottom bars and the two sides, a touch deeper than the board.
            v.push(part(0.0, 0.31, 0.52, 0.04, 0.08, 0.0, steel));
            v.push(part(0.0, -0.31, 0.52, 0.04, 0.08, 0.0, steel));
            v.push(part(0.25, 0.0, 0.04, 0.58, 0.08, 0.0, steel));
            v.push(part(-0.25, 0.0, 0.04, 0.58, 0.08, 0.0, steel));
            // The boss.
            v.push(part(0.0, 0.0, 0.14, 0.14, 0.11, 0.0, steel));
            v
        }
        // A knob and a stem on a wide flat base.
        ToolType::DraftingStamp => vec![
            part(0.0, 0.30, 0.13, 0.08, 0.13, 0.0, grip()),
            part(0.0, 0.10, 0.06, 0.34, 0.06, 0.0, grip()),
            part(0.0, -0.10, 0.28, 0.07, 0.28, 0.0, head),
            part(0.0, -0.155, 0.30, 0.04, 0.30, 0.0, steel),
        ],
    }
}

/// The mesh of `tool` in `material` for the hand `side`: a triangle list, the grip
/// at the origin (see the module doc).
pub fn tool_model(tool: ToolType, material: ToolMaterial, side: HandSide) -> Vec<Vertex> {
    let parts = parts(tool, material);
    let mut verts = Vec::with_capacity(parts.len() * 36);
    for p in &parts {
        push_part(&mut verts, p, side);
    }
    verts
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every `ToolType` this build can put on the wire, in wire order: the
    /// wire table is the one place that lists them all, and a variant appended
    /// there joins every test below.
    pub(crate) fn all_tool_types() -> Vec<ToolType> {
        (0..=u8::MAX).filter_map(crate::inventory::tool_type_from_held_wire).collect()
    }

    fn positions(v: &[Vertex]) -> Vec<[i32; 3]> {
        // Millimetres, so float noise doesn't matter; sorted, so order doesn't.
        let mut p: Vec<[i32; 3]> =
            v.iter().map(|v| v.position.map(|c| (c * 1000.0).round() as i32)).collect();
        p.sort_unstable();
        p.dedup();
        p
    }

    #[test]
    fn the_wire_table_lists_twelve_tool_types() {
        assert_eq!(all_tool_types().len(), 12, "a new tool type needs a shape in held_tool_model::parts");
    }

    #[test]
    fn every_tool_type_has_a_shape_that_differs_from_every_other() {
        let types = all_tool_types();
        for mat in [ToolMaterial::Wood, ToolMaterial::Iron, ToolMaterial::Satori] {
            let shapes: Vec<_> = types.iter().map(|t| positions(&tool_model(*t, mat, HandSide::Right))).collect();
            for (i, a) in shapes.iter().enumerate() {
                assert!(!a.is_empty(), "{:?} draws nothing", types[i]);
                for (j, b) in shapes.iter().enumerate().skip(i + 1) {
                    assert_ne!(a, b, "{:?} and {:?} have the same shape", types[i], types[j]);
                }
            }
        }
    }

    #[test]
    fn every_tool_type_differs_from_the_old_slab() {
        let reg = crate::block::BlockRegistry::new();
        let slab = positions(&crate::item_model::held_item_mesh(
            crate::protocol::ItemRef::Tool(2).into(),
            &reg,
            HandSide::Right,
        ));
        for t in all_tool_types() {
            assert_ne!(positions(&tool_model(t, ToolMaterial::Iron, HandSide::Right)), slab, "{t:?} is still the slab");
        }
    }

    #[test]
    fn the_left_hand_model_is_the_right_mirrored_in_x() {
        for t in all_tool_types() {
            let r = positions(&tool_model(t, ToolMaterial::Iron, HandSide::Right));
            let mut mirrored: Vec<[i32; 3]> = r.iter().map(|p| [-p[0], p[1], p[2]]).collect();
            mirrored.sort_unstable();
            let l = positions(&tool_model(t, ToolMaterial::Iron, HandSide::Left));
            assert_eq!(l.len(), mirrored.len(), "{t:?}");
            // A tilted box mirrors to the corner set of the mirrored box: allow a
            // millimetre of rounding on each coordinate.
            for (a, b) in l.iter().zip(&mirrored) {
                for k in 0..3 {
                    assert!((a[k] - b[k]).abs() <= 1, "{t:?}: {a:?} vs {b:?}");
                }
            }
        }
    }

    #[test]
    fn a_one_sided_tool_reaches_out_on_the_hands_own_side() {
        for (t, name) in [(ToolType::Axe, "axe"), (ToolType::Hoe, "hoe")] {
            let right = tool_model(t, ToolMaterial::Iron, HandSide::Right);
            let left = tool_model(t, ToolMaterial::Iron, HandSide::Left);
            let max_x = |v: &[Vertex]| v.iter().map(|v| v.position[0]).fold(f32::MIN, f32::max);
            let min_x = |v: &[Vertex]| v.iter().map(|v| v.position[0]).fold(f32::MAX, f32::min);
            assert!(max_x(&right) > 0.2 && min_x(&right) > -0.05, "{name}: right-hand feature points to +X");
            assert!(min_x(&left) < -0.2 && max_x(&left) < 0.05, "{name}: left-hand feature points to -X");
        }
    }

    #[test]
    fn the_sword_is_long_narrow_and_pointed_with_a_wide_guard() {
        // Wood, so the blade (the tier's wood tile) and the guard (iron) differ in
        // paint and can be measured apart.
        let v = tool_model(ToolType::Sword, ToolMaterial::Wood, HandSide::Right);
        let blade = tier_paint(ToolMaterial::Wood);
        let guard = metal();
        let on = |paint: Paint| -> Vec<[f32; 3]> {
            v.iter().filter(|q| q.tex_layer == paint.layer && q.uv == paint.uv()).map(|q| q.position).collect()
        };
        let width = |pts: &[[f32; 3]], lo: f32, hi: f32| -> f32 {
            pts.iter().filter(|p| (lo..hi).contains(&p[1])).map(|p| p[0].abs()).fold(0.0f32, f32::max) * 2.0
        };
        let blade_pts = on(blade);
        let top = blade_pts.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        let bottom = blade_pts.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!(top >= 0.34 && top < 0.35, "the point reaches the top of the hand envelope, inside it ({top})");
        assert!(top - bottom >= 0.5, "the blade is long ({}); it was 0.41", top - bottom);
        let low = width(&blade_pts, -0.2, 0.19);
        let high = width(&blade_pts, 0.19, 0.33);
        let tip = width(&blade_pts, 0.33, 0.36);
        assert!(low <= 0.08, "the blade is narrow ({low}); it was 0.13");
        assert!(high < low && tip < high && tip <= 0.035, "the blade tapers to a point: {low} {high} {tip}");
        let crossguard = width(&on(guard), -0.24, -0.19);
        assert!(crossguard > low * 3.0, "the guard ({crossguard}) is wider than the blade ({low})");
    }

    #[test]
    fn the_fishing_rod_has_a_line_a_float_and_a_hook() {
        let v = tool_model(ToolType::FishingRod, ToolMaterial::Iron, HandSide::Right);
        let float = centre_paint(crate::entity_model::TEX_ITEM_RED_DYE);
        let line = centre_paint(TEX_ITEM_STRING);
        let ys = |paint: Paint| -> Vec<f32> { v.iter().filter(|q| q.tex_layer == paint.layer && q.uv == paint.uv()).map(|q| q.position[1]).collect() };
        assert!(!ys(float).is_empty(), "a small float");
        let line_ys = ys(line);
        assert!(!line_ys.is_empty(), "a line");
        let line_len = line_ys.iter().cloned().fold(f32::MIN, f32::max) - line_ys.iter().cloned().fold(f32::MAX, f32::min);
        assert!(line_len >= 0.2, "the line hangs a good way ({line_len})");
        let float_top = ys(float).iter().cloned().fold(f32::MIN, f32::max);
        let line_top = line_ys.iter().cloned().fold(f32::MIN, f32::max);
        assert!(float_top < line_top, "the float hangs on the line, below the tip");
    }

    #[test]
    fn every_model_stays_inside_the_hand_envelope() {
        for t in all_tool_types() {
            for side in [HandSide::Right, HandSide::Left] {
                for v in tool_model(t, ToolMaterial::Iron, side) {
                    let [x, y, z] = v.position;
                    assert!(x.abs() <= 0.28 && (-0.34..=0.35).contains(&y) && z.abs() <= 0.17, "{t:?} {side:?} vertex {:?} leaves the envelope", v.position);
                }
            }
        }
    }

    #[test]
    fn every_paint_is_an_opaque_texel() {
        // A transparent texel would draw as a dark box (no cutout bit on a box
        // face). The head tile is checked for every tier.
        let mut paints = vec![grip(), metal(), centre_paint(TEX_ITEM_STRING), centre_paint(TEX_ITEM_WOOL), centre_paint(crate::block::TEX_ITEM_FLINT)];
        paints.push(Paint { layer: TEX_ITEM_LEATHER, texel: (8, 8) });
        for m in [ToolMaterial::Wood, ToolMaterial::Stone, ToolMaterial::Iron, ToolMaterial::Diamond, ToolMaterial::Satori] {
            paints.push(tier_paint(m));
        }
        for p in paints {
            assert!((layer_mask(p.layer)[p.texel.1 as usize] >> p.texel.0) & 1 == 1, "layer {} texel {:?} is transparent", p.layer, p.texel);
        }
    }

    #[test]
    fn a_tool_faces_carry_one_uv_so_they_sample_one_texel() {
        let v = tool_model(ToolType::Pickaxe, ToolMaterial::Iron, HandSide::Right);
        for quad in v.chunks_exact(6) {
            assert!(quad.iter().all(|q| q.uv == quad[0].uv && q.tex_layer == quad[0].tex_layer));
        }
    }

    #[test]
    fn the_triangles_wind_like_the_slab_and_every_other_entity_box() {
        // The entity pass's winding (`item_model::push_cuboid`, shared by the
        // slab, the cubes and the mobs): the cross product of a triangle's edges
        // points against its normal. A tilted or mirrored box must keep it.
        let reg = crate::block::BlockRegistry::new();
        let slab = crate::item_model::held_item_mesh(crate::protocol::ItemRef::Tool(2).into(), &reg, HandSide::Right);
        let sign = |v: &[Vertex]| -> Vec<bool> {
            v.chunks_exact(3)
                .map(|t| {
                    let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(t[i].position));
                    (b - a).cross(c - a).dot(Vec3::from(t[0].normal)) < 0.0
                })
                .collect()
        };
        assert!(sign(&slab).iter().all(|s| *s), "the slab's winding is the reference");
        for t in all_tool_types() {
            for side in [HandSide::Right, HandSide::Left] {
                assert!(sign(&tool_model(t, ToolMaterial::Iron, side)).iter().all(|s| *s), "{t:?} {side:?}: a face winds differently from the slab");
            }
        }
    }

    /// Off-hand release (slice 5) — the shield is a board of planks texels with an iron rim
    /// and boss, from layers that already exist (no new texture layer: the web budget is full).
    #[test]
    fn the_shield_is_a_planks_board_with_an_iron_rim_and_boss_from_existing_layers() {
        let v = tool_model(ToolType::Shield, ToolMaterial::Wood, HandSide::Right);
        let planks: Vec<_> = v.iter().filter(|q| q.tex_layer == crate::block::TEX_OAK_PLANKS).collect();
        let iron = metal();
        let rim: Vec<_> = v.iter().filter(|q| q.tex_layer == iron.layer && q.uv == iron.uv()).collect();
        assert!(!planks.is_empty() && !rim.is_empty(), "planks board and iron fittings");
        assert_eq!(planks.len() + rim.len(), v.len(), "nothing else is drawn");
        let extent = |pts: &[&Vertex], axis: usize| {
            let lo = pts.iter().map(|q| q.position[axis]).fold(f32::MAX, f32::min);
            let hi = pts.iter().map(|q| q.position[axis]).fold(f32::MIN, f32::max);
            hi - lo
        };
        assert!(extent(&planks, 0) >= 0.44 && extent(&planks, 1) >= 0.6, "a board about half a block wide and tall");
        assert!(extent(&planks, 2) <= 0.08, "and thin");
        // The boss stands out of the +Z face, further than the board.
        let board_front = planks.iter().map(|q| q.position[2]).fold(f32::MIN, f32::max);
        let boss_front = rim.iter().map(|q| q.position[2]).fold(f32::MIN, f32::max);
        assert!(boss_front > board_front, "the boss stands proud of the board ({boss_front} > {board_front})");
        // A planks texel is a light plank or a dark seam, both opaque.
        for texel in [(8u32, 6u32), (8, 8)] {
            assert_eq!(crate::item_model::layer_mask(crate::block::TEX_OAK_PLANKS)[texel.1 as usize] >> texel.0 & 1, 1, "{texel:?}");
        }
        assert_eq!(crate::texture_gen::texture_count(), 512, "the layer count is exactly the web budget: no layer was added");
    }

    #[test]
    fn the_shield_is_symmetric_so_both_hands_hold_the_same_board() {
        let l = tool_model(ToolType::Shield, ToolMaterial::Wood, HandSide::Left);
        let r = tool_model(ToolType::Shield, ToolMaterial::Wood, HandSide::Right);
        let set = |v: &[Vertex]| {
            let mut p: Vec<[i32; 3]> = v.iter().map(|q| q.position.map(|c| (c * 1000.0).round() as i32)).collect();
            p.sort();
            p.dedup();
            p
        };
        assert_eq!(set(&l), set(&r));
    }
}

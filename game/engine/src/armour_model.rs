//! Armour as small box models (Spec 03 §7.5a, "Armour").
//!
//! One armour table serves three looks, so a piece cannot look one way worn and
//! another in the hand:
//! - **worn** on the avatar: boxes laid over the player's own parts (head, body,
//!   arms, legs), inflated past the clothes overlay so nothing z-fights, and carried
//!   through the part's own swing angle ([`push_worn_part`]);
//! - **held** in a hand: a small model of the piece, centred on the hand like a held
//!   block ([`held_mesh`]);
//! - **dropped** on the ground: the same small model, a little smaller
//!   ([`drop_mesh`]).
//!
//! No new art: the block texture array is full, so each box face carries ONE texel of
//! an existing layer (the same flat-colour trick as `held_tool_model`: the same uv on
//! all four corners, so the face is that texel's colour whatever the sampler's
//! filter). A material's three paints ([`palette`]) are the opaque texels of a few
//! item layers nearest its inventory swatch ([`swatch`], the colour `Item::color`
//! gives the piece in the inventory), its darker neighbour and its lighter one.
//!
//! GPU-free and pure: everything here returns vertices, so it is unit-tested.

use std::sync::OnceLock;

use glam::Vec3;

use crate::armour::{ArmourMaterial, ArmourSlot};
use crate::entity_model::{
    ModelPart, TEX_ITEM_IRON_INGOT, TEX_ITEM_LEATHER, TEX_ITEM_STICK, TEX_ITEM_TOOL_DIAMOND, TEX_ITEM_TOOL_IRON,
    TEX_ITEM_TOOL_STONE, TEX_ITEM_TOOL_WOOD,
};
use crate::held_tool_model::Paint;
use crate::mesh::Vertex;

/// What an avatar wears, by [`ArmourSlot`] index (helmet, chestplate, leggings,
/// boots): the material of each worn piece, `None` for a bare slot.
pub type WornArmour = [Option<ArmourMaterial>; 4];

/// Nothing worn.
pub const NO_ARMOUR: WornArmour = [None; 4];

/// The player model's part indices (`entity_model::player_model_for`).
const HEAD: usize = 0;
const BODY: usize = 1;
const LEFT_ARM: usize = 2;
const RIGHT_ARM: usize = 3;
const LEFT_LEG: usize = 4;
const RIGHT_LEG: usize = 5;

/// Which of a material's three paints a box wears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// The plate itself.
    Base,
    /// Trim that sits darker: a visor, a belt, a toe cap.
    Dark,
    /// Trim that catches the light: a crest, an emblem, a cuff.
    Light,
}

/// A material's flat colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub base: Paint,
    pub dark: Paint,
    pub light: Paint,
}

impl Palette {
    /// The paint of `tone`.
    pub fn tone(self, tone: Tone) -> Paint {
        match tone {
            Tone::Base => self.base,
            Tone::Dark => self.dark,
            Tone::Light => self.light,
        }
    }
}

/// The inventory swatch of a material (`Item::color` for its armour; a test pins
/// that they agree).
pub fn swatch(material: ArmourMaterial) -> [f32; 3] {
    match material {
        ArmourMaterial::Leather => [0.55, 0.35, 0.18],
        ArmourMaterial::Iron => [0.75, 0.75, 0.78],
        ArmourMaterial::Diamond => [0.40, 0.92, 0.90],
        ArmourMaterial::Satori => [0.95, 0.55, 0.18],
        ArmourMaterial::Chainmail => [0.55, 0.55, 0.60],
        ArmourMaterial::Rubber => [0.42, 0.30, 0.22],
    }
}

/// Every material, in the order [`palettes`] holds them.
const MATERIALS: [ArmourMaterial; 6] = [
    ArmourMaterial::Leather,
    ArmourMaterial::Iron,
    ArmourMaterial::Diamond,
    ArmourMaterial::Satori,
    ArmourMaterial::Chainmail,
    ArmourMaterial::Rubber,
];

fn material_index(material: ArmourMaterial) -> usize {
    match material {
        ArmourMaterial::Leather => 0,
        ArmourMaterial::Iron => 1,
        ArmourMaterial::Diamond => 2,
        ArmourMaterial::Satori => 3,
        ArmourMaterial::Chainmail => 4,
        ArmourMaterial::Rubber => 5,
    }
}

/// The existing item layers a material's colours are picked from.
fn pool(material: ArmourMaterial) -> &'static [u32] {
    match material {
        ArmourMaterial::Leather => &[TEX_ITEM_LEATHER, TEX_ITEM_TOOL_WOOD, TEX_ITEM_STICK],
        ArmourMaterial::Iron => &[TEX_ITEM_IRON_INGOT, TEX_ITEM_TOOL_IRON],
        ArmourMaterial::Diamond => &[TEX_ITEM_TOOL_DIAMOND, TEX_ITEM_TOOL_IRON],
        ArmourMaterial::Satori => &[crate::block::TEX_ITEM_SATORI, TEX_ITEM_TOOL_WOOD],
        ArmourMaterial::Chainmail => &[TEX_ITEM_TOOL_STONE, crate::block::TEX_STONE],
        ArmourMaterial::Rubber => &[crate::block::TEX_RUBBER_LOG, crate::block::TEX_RUBBER_PLANKS],
    }
}

fn luminance(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

/// The colour of a texel the palette may pick, with where it is.
struct Texel {
    paint: Paint,
    colour: [f32; 3],
}

/// The opaque texels of the material's pool layers (an unknown or odd-sized layer
/// adds none).
fn pool_texels(textures: &[Vec<u8>], material: ArmourMaterial) -> Vec<Texel> {
    let mut out = Vec::new();
    for &layer in pool(material) {
        let Some(rgba) = textures.get(layer as usize).filter(|t| t.len() == 16 * 16 * 4) else { continue };
        for y in 0..16u32 {
            for x in 0..16u32 {
                let i = ((y * 16 + x) * 4) as usize;
                if rgba[i + 3] < 128 {
                    continue;
                }
                let colour = [rgba[i] as f32 / 255.0, rgba[i + 1] as f32 / 255.0, rgba[i + 2] as f32 / 255.0];
                out.push(Texel { paint: Paint { layer, texel: (x, y) }, colour });
            }
        }
    }
    out
}

fn palette_from(textures: &[Vec<u8>], material: ArmourMaterial) -> Palette {
    let texels = pool_texels(textures, material);
    let goal = swatch(material);
    let nearest = |to: [f32; 3], keep: &dyn Fn(f32) -> bool| {
        texels
            .iter()
            .filter(|t| keep(luminance(t.colour)))
            .min_by(|a, b| distance_squared(a.colour, to).total_cmp(&distance_squared(b.colour, to)))
    };
    // A material whose layers are all missing (a build without them) is a flat grey
    // card of its first layer: opaque by the stock set's own rule, never a hole.
    let fallback = Paint { layer: pool(material)[0], texel: (8, 8) };
    let Some(base) = nearest(goal, &|_| true) else {
        return Palette { base: fallback, dark: fallback, light: fallback };
    };
    let base_lum = luminance(base.colour);
    let dark_goal = goal.map(|c| c * 0.62);
    let light_goal = goal.map(|c| (c * 1.2).min(1.0));
    let dark = nearest(dark_goal, &|l| l < base_lum - 0.04).map_or(base.paint, |t| t.paint);
    let light = nearest(light_goal, &|l| l > base_lum + 0.04).map_or(base.paint, |t| t.paint);
    Palette { base: base.paint, dark, light }
}

fn palettes() -> &'static [Palette; 6] {
    static PALETTES: OnceLock<[Palette; 6]> = OnceLock::new();
    PALETTES.get_or_init(|| {
        let textures = crate::texture_gen::generate_textures();
        MATERIALS.map(|m| palette_from(&textures, m))
    })
}

/// The flat colours of `material`'s armour.
pub fn palette(material: ArmourMaterial) -> Palette {
    palettes()[material_index(material)]
}

// ── Boxes ────────────────────────────────────────────────────────────────────

/// One axis-aligned box of an armour piece, in local space, with the paint tone it
/// wears.
struct Block {
    min: Vec3,
    max: Vec3,
    tone: Tone,
}

/// Emit `block`'s six faces in `paint`, every corner and normal passed through
/// `place` / `turn`. Same faces and winding as `held_tool_model::push_part`.
fn push_box(
    verts: &mut Vec<Vertex>,
    (min, max): (Vec3, Vec3),
    paint: Paint,
    place: &dyn Fn(Vec3) -> Vec3,
    turn: &dyn Fn(Vec3) -> Vec3,
) {
    let corner = |sx: bool, sy: bool, sz: bool| {
        place(Vec3::new(if sx { max.x } else { min.x }, if sy { max.y } else { min.y }, if sz { max.z } else { min.z }))
    };
    let c = [
        corner(false, false, false),
        corner(true, false, false),
        corner(true, true, false),
        corner(false, true, false),
        corner(false, false, true),
        corner(true, false, true),
        corner(true, true, true),
        corner(false, true, true),
    ];
    let faces: [([f32; 3], [usize; 4]); 6] = [
        ([1.0, 0.0, 0.0], [1, 5, 6, 2]),
        ([-1.0, 0.0, 0.0], [4, 0, 3, 7]),
        ([0.0, 1.0, 0.0], [3, 2, 6, 7]),
        ([0.0, -1.0, 0.0], [4, 5, 1, 0]),
        ([0.0, 0.0, 1.0], [5, 4, 7, 6]),
        ([0.0, 0.0, -1.0], [0, 1, 2, 3]),
    ];
    let uv = {
        let p = paint;
        [(p.texel.0 as f32 + 0.5) / 16.0, (p.texel.1 as f32 + 0.5) / 16.0]
    };
    for (n, idx) in faces {
        let normal = turn(Vec3::from(n)).to_array();
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

// ── Worn ─────────────────────────────────────────────────────────────────────

/// One box of a worn piece: a span of one player part (`lo`..`hi`, each a fraction
/// 0..1 of the part's box on x, y, z), pushed out by `grow` on each axis. The span
/// being a fraction of the part is what makes the same table fit the slim arm.
struct WornBox {
    part: usize,
    lo: [f32; 3],
    hi: [f32; 3],
    grow: [f32; 3],
    tone: Tone,
}

const fn worn(part: usize, lo: [f32; 3], hi: [f32; 3], grow: [f32; 3], tone: Tone) -> WornBox {
    WornBox { part, lo, hi, grow, tone }
}

/// How far each piece stands off the skin. The clothes overlay is 0.03125 out on
/// the head and 0.015625 on the rest (`skin_pose::overlay_inflate`); every plate's
/// faces clear that by more than 0.01 (a test pins it), and trim stands further out
/// than the plate it sits on.
const HELM: f32 = 0.05;
const CHEST: f32 = 0.04;
const LEGGING: f32 = 0.03;
const BOOT: f32 = 0.045;

const FULL_LO: [f32; 3] = [0.0, 0.0, 0.0];
const FULL_HI: [f32; 3] = [1.0, 1.0, 1.0];

// The head's front is -Z (its face), so a helmet's back plate is at high z.

/// A cap with a back, two cheek plates, a brow and a nose guard: the face stays
/// open.
const HELMET_BOXES: [WornBox; 6] = [
    worn(HEAD, [0.0, 0.78, 0.0], FULL_HI, [HELM; 3], Tone::Base),
    worn(HEAD, [0.0, 0.12, 0.78], [1.0, 0.78, 1.0], [HELM; 3], Tone::Base),
    worn(HEAD, [0.0, 0.12, 0.45], [0.08, 0.78, 1.0], [HELM; 3], Tone::Base),
    worn(HEAD, [0.92, 0.12, 0.45], [1.0, 0.78, 1.0], [HELM; 3], Tone::Base),
    worn(HEAD, [0.0, 0.62, 0.0], [1.0, 0.78, 0.14], [HELM + 0.02; 3], Tone::Dark),
    worn(HEAD, [0.44, 0.3, 0.0], [0.56, 0.78, 0.1], [0.012, 0.0, HELM + 0.02], Tone::Dark),
];

/// The torso and the top of both arms, with a shoulder cap on each and a plate of
/// trim on the chest.
const CHESTPLATE_BOXES: [WornBox; 6] = [
    worn(BODY, [0.0, 0.2, 0.0], FULL_HI, [CHEST; 3], Tone::Base),
    worn(BODY, [0.3, 0.45, 0.0], [0.7, 0.85, 0.0], [0.0, 0.0, CHEST + 0.02], Tone::Light),
    worn(LEFT_ARM, [0.0, 0.4, 0.0], FULL_HI, [CHEST; 3], Tone::Base),
    worn(RIGHT_ARM, [0.0, 0.4, 0.0], FULL_HI, [CHEST; 3], Tone::Base),
    worn(LEFT_ARM, [0.0, 0.82, 0.0], FULL_HI, [CHEST + 0.02; 3], Tone::Dark),
    worn(RIGHT_ARM, [0.0, 0.82, 0.0], FULL_HI, [CHEST + 0.02; 3], Tone::Dark),
];

/// Both legs from the knee up and a belt round the hips.
const LEGGINGS_BOXES: [WornBox; 3] = [
    worn(LEFT_LEG, [0.0, 0.3, 0.0], FULL_HI, [LEGGING; 3], Tone::Base),
    worn(RIGHT_LEG, [0.0, 0.3, 0.0], FULL_HI, [LEGGING; 3], Tone::Base),
    worn(BODY, FULL_LO, [1.0, 0.2, 1.0], [0.06; 3], Tone::Dark),
];

/// Both feet and ankles, with a toe cap and a cuff. The sole stays a hair above the
/// ground.
const BOOTS_BOXES: [WornBox; 6] = [
    worn(LEFT_LEG, [0.0, 0.006, 0.0], [1.0, 0.3, 1.0], [BOOT, 0.0, BOOT], Tone::Base),
    worn(RIGHT_LEG, [0.0, 0.006, 0.0], [1.0, 0.3, 1.0], [BOOT, 0.0, BOOT], Tone::Base),
    worn(LEFT_LEG, [0.05, 0.006, 0.0], [0.95, 0.16, 0.35], [0.0, 0.0, BOOT + 0.025], Tone::Dark),
    worn(RIGHT_LEG, [0.05, 0.006, 0.0], [0.95, 0.16, 0.35], [0.0, 0.0, BOOT + 0.025], Tone::Dark),
    worn(LEFT_LEG, [0.0, 0.27, 0.0], [1.0, 0.32, 1.0], [BOOT + 0.015, 0.0, BOOT + 0.015], Tone::Light),
    worn(RIGHT_LEG, [0.0, 0.27, 0.0], [1.0, 0.32, 1.0], [BOOT + 0.015, 0.0, BOOT + 0.015], Tone::Light),
];

/// The boxes of a worn `slot`.
fn worn_boxes(slot: ArmourSlot) -> &'static [WornBox] {
    match slot {
        ArmourSlot::Helmet => &HELMET_BOXES,
        ArmourSlot::Chestplate => &CHESTPLATE_BOXES,
        ArmourSlot::Leggings => &LEGGINGS_BOXES,
        ArmourSlot::Boots => &BOOTS_BOXES,
    }
}

const SLOTS: [ArmourSlot; 4] = [ArmourSlot::Helmet, ArmourSlot::Chestplate, ArmourSlot::Leggings, ArmourSlot::Boots];

/// A worn box in the model's own space (the part's origin and size, before any
/// swing): `(min, max)`.
fn worn_extent(b: &WornBox, part: &ModelPart) -> (Vec3, Vec3) {
    let base = part.origin - part.size * 0.5;
    let grow = Vec3::from(b.grow);
    (base + Vec3::from(b.lo) * part.size - grow, base + Vec3::from(b.hi) * part.size + grow)
}

/// `p` turned about the X axis through `pivot` by `a` radians (the rotation the
/// avatar's part builder gives an arm, a leg or a nodding head).
fn rot_x(p: Vec3, pivot: Vec3, a: f32) -> Vec3 {
    if a.abs() <= 0.001 {
        return p;
    }
    let (s, c) = a.sin_cos();
    let (dy, dz) = (p.y - pivot.y, p.z - pivot.z);
    Vec3::new(p.x, pivot.y + dy * c - dz * s, pivot.z + dy * s + dz * c)
}

/// The worn armour of player part `part_index` (`part`: its box in the model's own
/// space), turned about the part's pivot by `x_rot` (the part's own effective
/// angle, the one the skin part is built with), then about the vertical axis through
/// the pivot by `arm_yaw` (an eating arm's swing in), then about Y by `yaw`, then moved
/// to `entity_pos`: the same transform `entity_model::build_skin_part_vertices`
/// gives the part's skin, so the armour never lags a swinging limb.
#[allow(clippy::too_many_arguments)]
pub fn push_worn_part(
    verts: &mut Vec<Vertex>,
    part_index: usize,
    part: &ModelPart,
    worn_set: &WornArmour,
    x_rot: f32,
    arm_yaw: f32,
    yaw: f32,
    entity_pos: Vec3,
) {
    let (sin_y, cos_y) = yaw.sin_cos();
    let turn_y = |p: Vec3| Vec3::new(p.x * cos_y - p.z * sin_y, p.y, p.x * sin_y + p.z * cos_y);
    let swing_in = |p: Vec3, about: Vec3| crate::entity_model::rot_y_about(p, about, arm_yaw);
    let place = |p: Vec3| turn_y(swing_in(rot_x(p, part.pivot, x_rot), part.pivot)) + entity_pos;
    let turn = |n: Vec3| turn_y(swing_in(rot_x(n, Vec3::ZERO, x_rot), Vec3::ZERO));
    for (slot, material) in SLOTS.iter().zip(worn_set.iter()) {
        let Some(material) = material else { continue };
        let colours = palette(*material);
        for b in worn_boxes(*slot).iter().filter(|b| b.part == part_index) {
            push_box(verts, worn_extent(b, part), colours.tone(b.tone), &place, &turn);
        }
    }
}

// ── Held and dropped ─────────────────────────────────────────────────────────

const fn solid(cx: f32, cy: f32, cz: f32, w: f32, h: f32, d: f32, tone: Tone) -> Block {
    Block {
        min: Vec3::new(cx - w * 0.5, cy - h * 0.5, cz - d * 0.5),
        max: Vec3::new(cx + w * 0.5, cy + h * 0.5, cz + d * 0.5),
        tone,
    }
}

// The small model of a piece, centred on the hand like a held block (about 0.4
// across), its open side toward +Z (the viewer in first person).

const HELMET_HAND: [Block; 7] = [
    solid(0.0, 0.07, 0.0, 0.30, 0.13, 0.28, Tone::Base),
    solid(0.0, -0.04, -0.11, 0.30, 0.14, 0.06, Tone::Base),
    solid(-0.12, -0.05, 0.0, 0.06, 0.14, 0.28, Tone::Base),
    solid(0.12, -0.05, 0.0, 0.06, 0.14, 0.28, Tone::Base),
    solid(0.0, 0.0, 0.125, 0.30, 0.05, 0.04, Tone::Dark),
    solid(0.0, -0.06, 0.13, 0.04, 0.10, 0.03, Tone::Dark),
    solid(0.0, 0.15, 0.0, 0.04, 0.04, 0.28, Tone::Light),
];

const CHESTPLATE_HAND: [Block; 7] = [
    solid(0.0, 0.0, 0.0, 0.28, 0.32, 0.12, Tone::Base),
    solid(0.0, 0.03, 0.07, 0.12, 0.14, 0.03, Tone::Light),
    solid(-0.19, 0.11, 0.0, 0.10, 0.10, 0.12, Tone::Base),
    solid(0.19, 0.11, 0.0, 0.10, 0.10, 0.12, Tone::Base),
    solid(-0.19, 0.17, 0.0, 0.11, 0.03, 0.13, Tone::Dark),
    solid(0.19, 0.17, 0.0, 0.11, 0.03, 0.13, Tone::Dark),
    solid(0.0, -0.15, 0.0, 0.30, 0.04, 0.13, Tone::Dark),
];

const LEGGINGS_HAND: [Block; 5] = [
    solid(0.0, 0.15, 0.0, 0.28, 0.05, 0.12, Tone::Dark),
    solid(-0.07, -0.02, 0.0, 0.12, 0.30, 0.11, Tone::Base),
    solid(0.07, -0.02, 0.0, 0.12, 0.30, 0.11, Tone::Base),
    solid(-0.07, -0.02, 0.065, 0.08, 0.06, 0.02, Tone::Light),
    solid(0.07, -0.02, 0.065, 0.08, 0.06, 0.02, Tone::Light),
];

const BOOTS_HAND: [Block; 8] = [
    solid(-0.09, -0.05, 0.0, 0.13, 0.10, 0.20, Tone::Base),
    solid(0.09, -0.05, 0.0, 0.13, 0.10, 0.20, Tone::Base),
    solid(-0.09, 0.05, -0.04, 0.12, 0.12, 0.11, Tone::Base),
    solid(0.09, 0.05, -0.04, 0.12, 0.12, 0.11, Tone::Base),
    solid(-0.09, -0.07, 0.09, 0.13, 0.05, 0.07, Tone::Dark),
    solid(0.09, -0.07, 0.09, 0.13, 0.05, 0.07, Tone::Dark),
    solid(-0.09, 0.11, -0.04, 0.13, 0.03, 0.12, Tone::Light),
    solid(0.09, 0.11, -0.04, 0.13, 0.03, 0.12, Tone::Light),
];

fn hand_blocks(slot: ArmourSlot) -> &'static [Block] {
    match slot {
        ArmourSlot::Helmet => &HELMET_HAND,
        ArmourSlot::Chestplate => &CHESTPLATE_HAND,
        ArmourSlot::Leggings => &LEGGINGS_HAND,
        ArmourSlot::Boots => &BOOTS_HAND,
    }
}

/// The hand-sized model of `slot` in `material`, centred on the origin, a triangle
/// list (36 vertices a box). The held item mesh and, scaled, the dropped one.
pub fn held_mesh(slot: ArmourSlot, material: ArmourMaterial) -> Vec<Vertex> {
    let colours = palette(material);
    let mut verts = Vec::new();
    for b in hand_blocks(slot) {
        push_box(&mut verts, (b.min, b.max), colours.tone(b.tone), &|p: Vec3| p, &|n: Vec3| n);
    }
    verts
}

/// A dropped piece is the held model at this scale (the drop cube is 0.2 across).
pub const DROP_SCALE: f32 = 0.7;

/// The dropped model of `slot` in `material`: [`held_mesh`] at [`DROP_SCALE`],
/// shifted so its lowest point is at the origin (the caller lifts it off the
/// ground and spins it about Y).
pub fn drop_mesh(slot: ArmourSlot, material: ArmourMaterial) -> Vec<Vertex> {
    let mut verts = held_mesh(slot, material);
    let lowest = verts.iter().map(|v| v.position[1]).fold(f32::INFINITY, f32::min);
    for v in &mut verts {
        v.position = [v.position[0] * DROP_SCALE, (v.position[1] - lowest) * DROP_SCALE, v.position[2] * DROP_SCALE];
    }
    verts
}

thread_local! {
    /// The dropped models already built, by slot and material: a pile of identical
    /// helmets used to allocate, build and scan the same seven-box mesh for every
    /// helmet in every viewport every frame.
    static DROP_MESHES: std::cell::RefCell<std::collections::HashMap<(usize, usize), Vec<Vertex>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// Test hook: how many dropped models were actually built on this thread.
    #[cfg(test)]
    static DROP_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Run `f` on [`drop_mesh`] of `slot` in `material`, built once and kept: the
/// caller appends its transformed vertices straight into its own buffer instead of
/// taking a copy of the mesh to turn in place.
pub fn with_drop_mesh<R>(slot: ArmourSlot, material: ArmourMaterial, f: impl FnOnce(&[Vertex]) -> R) -> R {
    DROP_MESHES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let mesh = cache.entry((slot as usize, material as usize)).or_insert_with(|| {
            #[cfg(test)]
            DROP_BUILDS.with(|n| n.set(n.get() + 1));
            drop_mesh(slot, material)
        });
        f(mesh)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armour::all_armour_combos;
    use crate::entity_model::player_model_for;
    use crate::skin_uv::ArmModel;

    fn full(material: ArmourMaterial) -> WornArmour {
        [Some(material); 4]
    }

    /// The worn boxes of `set` over the model `arm`: `(part, min, max)` in the
    /// model's own space.
    fn worn_extents(set: &WornArmour, arm: ArmModel) -> Vec<(usize, Vec3, Vec3)> {
        let parts = player_model_for(arm);
        let mut out = Vec::new();
        for (slot, material) in SLOTS.iter().zip(set.iter()) {
            if material.is_none() {
                continue;
            }
            for b in worn_boxes(*slot) {
                let (lo, hi) = worn_extent(b, &parts[b.part]);
                out.push((b.part, lo, hi));
            }
        }
        out
    }

    #[test]
    fn a_pile_of_identical_drops_builds_its_model_once() {
        let builds = || DROP_BUILDS.with(std::cell::Cell::get);
        let want = drop_mesh(ArmourSlot::Helmet, ArmourMaterial::Iron);
        let bytes = |v: &[Vertex]| bytemuck::cast_slice::<Vertex, u8>(v).to_vec();
        for _ in 0..40 {
            with_drop_mesh(ArmourSlot::Helmet, ArmourMaterial::Iron, |m| assert_eq!(bytes(m), bytes(&want)));
        }
        assert_eq!(builds(), 1, "forty helmets, one build");
        with_drop_mesh(ArmourSlot::Helmet, ArmourMaterial::Diamond, |m| assert_eq!(m.len(), want.len()));
        with_drop_mesh(ArmourSlot::Boots, ArmourMaterial::Iron, |_| ());
        assert_eq!(builds(), 3, "one per slot and material");
        for (slot, material) in all_armour_combos() {
            with_drop_mesh(slot, material, |m| assert_eq!(bytes(m), bytes(&drop_mesh(slot, material))));
        }
    }

    #[test]
    fn every_armour_combo_has_a_held_and_a_dropped_model() {
        for (slot, material) in all_armour_combos() {
            let held = held_mesh(slot, material);
            assert!(!held.is_empty(), "{slot:?} {material:?} has no held mesh");
            assert_eq!(held.len() % 36, 0, "{slot:?} {material:?}: whole boxes");
            let dropped = drop_mesh(slot, material);
            assert_eq!(dropped.len(), held.len());
            let lowest = dropped.iter().map(|v| v.position[1]).fold(f32::INFINITY, f32::min);
            assert!(lowest.abs() < 1e-5, "{slot:?}: a dropped piece rests on its foot, not {lowest}");
            let (top, spread) = dropped.iter().fold((0.0_f32, 0.0_f32), |(t, s), v| {
                (t.max(v.position[1]), s.max(v.position[0].abs()).max(v.position[2].abs()))
            });
            assert!(top < 0.35 && spread < 0.2, "{slot:?}: a dropped piece is about a drop cube's size ({top}, {spread})");
        }
    }

    #[test]
    fn a_held_piece_is_hand_sized_and_finite() {
        for (slot, material) in all_armour_combos() {
            for v in held_mesh(slot, material) {
                assert!(v.position.iter().all(|c| c.is_finite() && c.abs() <= 0.26), "{slot:?}: {:?}", v.position);
                assert!((0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1]));
                let n = Vec3::from(v.normal);
                assert!((n.length() - 1.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn every_paint_is_an_opaque_texel_of_a_real_layer() {
        for material in MATERIALS {
            let p = palette(material);
            for paint in [p.base, p.dark, p.light] {
                let mask = crate::item_model::layer_mask(paint.layer);
                assert_eq!(
                    (mask[paint.texel.1 as usize] >> paint.texel.0) & 1,
                    1,
                    "{material:?}: texel {:?} of layer {} is transparent (it would draw as a dark box)",
                    paint.texel,
                    paint.layer
                );
            }
        }
    }

    #[test]
    fn the_swatch_is_the_inventory_colour() {
        let reg = crate::block::BlockRegistry::new();
        for (slot, material) in all_armour_combos() {
            let item = crate::item::Item::Armour(crate::armour::ArmourItem::new(slot, material));
            assert_eq!(item.color(&reg), swatch(material), "{slot:?} {material:?}");
        }
    }

    #[test]
    fn a_material_wears_its_own_colour_with_a_darker_and_a_lighter_trim() {
        let textures = crate::texture_gen::generate_textures();
        let colour = |p: Paint| {
            let i = ((p.texel.1 * 16 + p.texel.0) * 4) as usize;
            let t = &textures[p.layer as usize];
            [t[i] as f32 / 255.0, t[i + 1] as f32 / 255.0, t[i + 2] as f32 / 255.0]
        };
        let mut failures = Vec::new();
        let mut bases = Vec::new();
        for material in MATERIALS {
            let (p, goal) = (palette(material), swatch(material));
            let (base, dark, light) = (colour(p.base), colour(p.dark), colour(p.light));
            eprintln!("ARMOUR palette {material:?}: base {base:?} dark {dark:?} light {light:?} (swatch {goal:?}) {p:?}");
            for c in 0..3 {
                if (base[c] - goal[c]).abs() > 0.3 {
                    failures.push(format!("{material:?}: the plate's colour {base:?} is far from its swatch {goal:?}"));
                    break;
                }
            }
            if !(luminance(dark) <= luminance(base) && luminance(base) <= luminance(light)) {
                failures.push(format!("{material:?}: trim is not darker/lighter ({dark:?} {base:?} {light:?})"));
            }
            bases.push((material, base));
        }
        for (i, a) in bases.iter().enumerate() {
            for b in &bases[i + 1..] {
                if distance_squared(a.1, b.1).sqrt() < 0.08 {
                    failures.push(format!("{:?} and {:?} wear the same colour ({:?} {:?})", a.0, b.0, a.1, b.1));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn worn_plates_stand_clear_of_the_clothes_overlay() {
        // A face within 0.01 of an overlay face would z-fight with it.
        for arm in [ArmModel::Classic, ArmModel::Slim] {
            let parts = player_model_for(arm);
            for (part, lo, hi) in worn_extents(&full(ArmourMaterial::Iron), arm) {
                let inflate = crate::skin_pose::overlay_inflate(part);
                let half = parts[part].size * 0.5 + Vec3::splat(inflate);
                let (olo, ohi) = (parts[part].origin - half, parts[part].origin + half);
                for a in 0..3 {
                    for (face, overlay) in [(lo[a], olo[a]), (lo[a], ohi[a]), (hi[a], olo[a]), (hi[a], ohi[a])] {
                        assert!(
                            (face - overlay).abs() >= 0.01,
                            "{arm:?} part {part} axis {a}: a plate face at {face} is within 0.01 of the overlay at {overlay}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_full_set_covers_the_body_and_never_floats() {
        for arm in [ArmModel::Classic, ArmModel::Slim] {
            let extents = worn_extents(&full(ArmourMaterial::Diamond), arm);
            for part in 0..6 {
                assert!(extents.iter().any(|e| e.0 == part), "{arm:?}: part {part} wears nothing");
            }
            let (lo, hi) = crate::skin_pose::avatar_aabb(0.0, crate::skin_pose::overlay_inflate(1));
            let top = extents.iter().map(|e| e.2.y).fold(f32::NEG_INFINITY, f32::max);
            let bottom = extents.iter().map(|e| e.1.y).fold(f32::INFINITY, f32::min);
            let side = extents.iter().map(|e| e.2.x).fold(f32::NEG_INFINITY, f32::max);
            assert!(top > hi[1], "the helmet is over the hat: {top} vs {}", hi[1]);
            let arm = &player_model_for(arm)[RIGHT_ARM];
            let sleeve = arm.origin.x + arm.size.x * 0.5 + crate::skin_pose::overlay_inflate(RIGHT_ARM);
            assert!(side > sleeve, "{side} the shoulder caps are outside the sleeves ({sleeve})");
            assert!(bottom >= 0.0 && bottom < 0.02, "the boots stand on the ground, not {bottom} (overlay {})", lo[1]);
        }
    }

    #[test]
    fn a_worn_part_turns_with_its_limb() {
        let parts = player_model_for(ArmModel::Classic);
        let set = full(ArmourMaterial::Iron);
        for i in 0..6 {
            let mut still = Vec::new();
            push_worn_part(&mut still, i, &parts[i], &set, 0.0, 0.0, 0.0, Vec3::ZERO);
            assert!(!still.is_empty(), "part {i}");
            let angle = 0.4;
            let mut turned = Vec::new();
            push_worn_part(&mut turned, i, &parts[i], &set, angle, 0.0, 0.0, Vec3::ZERO);
            assert_eq!(still.len(), turned.len());
            for (a, b) in still.iter().zip(&turned) {
                let want = rot_x(Vec3::from(a.position), parts[i].pivot, angle);
                assert!((want - Vec3::from(b.position)).length() < 1e-5, "part {i}");
                let n = rot_x(Vec3::from(a.normal), Vec3::ZERO, angle);
                assert!((n - Vec3::from(b.normal)).length() < 1e-5, "part {i}: normals turn too");
            }
        }
    }

    #[test]
    fn a_bare_slot_wears_nothing() {
        let parts = player_model_for(ArmModel::Classic);
        let mut verts = Vec::new();
        for i in 0..6 {
            push_worn_part(&mut verts, i, &parts[i], &NO_ARMOUR, 0.0, 0.0, 0.0, Vec3::ZERO);
        }
        assert!(verts.is_empty());
        // Boots alone touch only the legs.
        let mut boots = Vec::new();
        let set: WornArmour = [None, None, None, Some(ArmourMaterial::Rubber)];
        for i in 0..6 {
            let before = boots.len();
            push_worn_part(&mut boots, i, &parts[i], &set, 0.0, 0.0, 0.0, Vec3::ZERO);
            assert_eq!(boots.len() > before, i == LEFT_LEG || i == RIGHT_LEG, "part {i}");
        }
    }
}

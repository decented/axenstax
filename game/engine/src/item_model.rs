//! One item model for the item frame, the hand and the ground (Spec 03 §5.10).
//!
//! An item looks like the block it places, by the chunk mesher's own rule
//! ([`crate::mesh::render_class`]), so a block can't look one way in the world
//! and another as an item:
//! - a block the mesher draws as a full cube, a shaped block (stairs, buttons,
//!   plates, ...) and the campfire smoke keep the small cube they have always
//!   had ([`ItemLook::Cube`]);
//! - a block it draws as a cross billboard (crops, flowers, saplings, tall
//!   grass, ...), a flat rail or cable, or the torch pole is a flat extruded
//!   sprite of its inventory icon, the block's `tex_top` layer
//!   ([`ItemLook::Sprite`], Minecraft's item look): the front and back show
//!   the icon, the thickness is 1/16 of the width, and the edges are the
//!   1-texel border strips of the icon's opaque texels, merged into runs.
//!   Every sprite vertex carries [`crate::mesh::CUTOUT_FLAG`], so `fs_main`
//!   drops the icon's transparent texels.
//!
//! Tools, materials, plans and armour keep their slab and cube looks.
//!
//! Users: the item frame (`mesh`, a sprite lies flat on the plate), the held
//! item (the first-person `viewmodel` and the third-person avatar hand in
//! `entity_model`) and the dropped item (`entity_model`, local and remote).
//! Each builder returns local-space geometry centred at the origin; the caller
//! places it.

use std::sync::OnceLock;

use glam::Vec3;

use crate::block::{BlockId, BlockRegistry};
use crate::crafting::ToolMaterial;
use crate::entity_model::{
    material_texture, push_textured_quad, tool_texture, TEX_ITEM_IRON_INGOT, TEX_ITEM_STICK,
};
use crate::item::{Item, MaterialId};
use crate::mesh::{RenderClass, Vertex, CUTOUT_FLAG};
use crate::protocol::ItemRef;

/// How an item is drawn in a frame, in the hand and on the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemLook {
    /// A small cube: the `top` layer on +Y, `bottom` on -Y, `side` on the four
    /// sides, each face the whole texture.
    Cube { top: u32, side: u32, bottom: u32 },
    /// A flat extruded sprite of one texture layer (the inventory icon).
    Sprite { layer: u32 },
}

impl ItemLook {
    /// One texture on every face of the cube.
    pub const fn cube(tex: u32) -> ItemLook {
        ItemLook::Cube { top: tex, side: tex, bottom: tex }
    }
}

/// A sprite's thickness as a fraction of its width: one texel of its 16-texel
/// icon.
pub const SPRITE_DEPTH: f32 = 1.0 / 16.0;
/// Width of a sprite held in the hand (a held block is a 0.4 cube).
pub const HELD_SPRITE_WIDTH: f32 = 0.6;
/// Where a held sprite's centre sits from the hand (the hand end of the arm;
/// +X is outward from the body, +Y up the arm): beside the fist on the
/// outside and a little up, so a torch's art clears the arm (0.25 wide plus
/// its sleeve) instead of hiding its flame inside it. A held cube stays
/// centred on the hand.
pub const HELD_SPRITE_OFFSET: Vec3 = Vec3::new(0.25, 0.1, 0.0);
/// Width of a dropped sprite (a dropped block is a 0.2 cube).
pub const DROP_SPRITE_WIDTH: f32 = 0.4;
/// Width of a framed sprite: the half-cell square a framed block fills.
pub const FRAME_SPRITE_WIDTH: f32 = 0.5;

/// The look of block `id` as an item: a sprite when the mesher draws the block
/// as a cross billboard, a flat rail or cable, or the torch; otherwise the
/// small cube with the block's own top, side and bottom layers. An unknown id
/// is the cube of the registry's fallback, as before.
pub fn block_look(id: BlockId, registry: &BlockRegistry) -> ItemLook {
    let def = registry.get(id);
    let cube = ItemLook::Cube { top: def.tex_top, side: def.tex_side, bottom: def.tex_bottom };
    if !registry.is_known(id) {
        return cube;
    }
    match crate::mesh::render_class(id, registry) {
        RenderClass::Cross | RenderClass::Flat | RenderClass::Torch => ItemLook::Sprite { layer: def.tex_top },
        RenderClass::None | RenderClass::Cube | RenderClass::Shaped | RenderClass::Smoke => cube,
    }
}

/// The look of an inventory `Item`: a dropped one (local `ItemEntity`s and a
/// server-broadcast item with a full payload) and a framed block.
pub fn item_look(item: &Item, registry: &BlockRegistry) -> ItemLook {
    match item {
        Item::Block(id) => block_look(*id, registry),
        Item::Material(mid) => ItemLook::cube(material_texture(*mid)),
        Item::Tool(t) => ItemLook::cube(tool_texture(t.material)),
        // Spec 24 — a ground-dropped Plan: a small parchment scroll cube, the
        // Blueprint Paper top for the parchment faces and the plaque side for
        // the wood-rolled edges.
        Item::Plan(_) => ItemLook::Cube {
            top: crate::block::TEX_BLUEPRINT_PAPER_TOP,
            side: crate::block::TEX_ARCHITECT_PLAQUE_SIDE,
            bottom: crate::block::TEX_BLUEPRINT_PAPER_TOP,
        },
        // Spec 28e — ground-dropped armour. BRIDGE: reuse the iron-ingot
        // texture for all tiers; per-slot icons land with the live armour HUD
        // work.
        Item::Armour(_) => ItemLook::cube(TEX_ITEM_IRON_INGOT),
    }
}

/// The look of a wire `ItemRef` (a server-broadcast drop without a full
/// payload). `None` = nothing sound to draw: an unknown or hostile block id
/// (the registry's AIR fallback would draw a ghost cube), an unknown material
/// or tool tier, `Empty`.
pub fn item_ref_look(item: ItemRef, registry: &BlockRegistry) -> Option<ItemLook> {
    match item {
        ItemRef::Block(id) if registry.is_known(id) => Some(block_look(id, registry)),
        ItemRef::Material(id) => MaterialId::try_from(id).ok().map(|m| ItemLook::cube(material_texture(m))),
        ItemRef::Tool(tier) => {
            let mat = match tier {
                0 => ToolMaterial::Wood,
                1 => ToolMaterial::Stone,
                2 => ToolMaterial::Iron,
                3 => ToolMaterial::Diamond,
                4 => ToolMaterial::Satori,
                _ => return None,
            };
            Some(ItemLook::cube(tool_texture(mat)))
        }
        _ => None,
    }
}

/// Local-space mesh (about unit scale, the hand at the origin) for a held
/// item, a triangle list. The caller moves it to the hand anchor or the
/// first-person viewmodel slot. A cuboid is centred on the hand, 6 faces x 2
/// triangles x 3 vertices = **36 vertices**; a block sprite is
/// [`sprite_triangles`] moved to [`HELD_SPRITE_OFFSET`].
pub fn held_item_mesh(item: ItemRef, registry: &BlockRegistry) -> Vec<Vertex> {
    match item {
        ItemRef::Empty => Vec::new(),
        ItemRef::Block(id) => match block_look(id, registry) {
            ItemLook::Sprite { layer } => {
                let mut verts = sprite_triangles(layer, HELD_SPRITE_WIDTH);
                for v in &mut verts {
                    v.position = (Vec3::from(v.position) + HELD_SPRITE_OFFSET).to_array();
                }
                verts
            }
            ItemLook::Cube { top, side, bottom } => {
                let mut verts = Vec::with_capacity(36);
                push_cuboid(&mut verts, Vec3::ZERO, Vec3::splat(0.4), top, side, bottom);
                verts
            }
        },
        ItemRef::Tool(tier) => {
            // Map the wire tier back to a ToolMaterial, then defer to the
            // authoritative texture lookup so a held tool always matches its
            // dropped-item icon (e.g. Satori has its own texture, not diamond).
            let mat = match tier {
                0 => ToolMaterial::Wood,
                1 => ToolMaterial::Stone,
                2 => ToolMaterial::Iron,
                3 => ToolMaterial::Diamond,
                _ => ToolMaterial::Satori, // tier 4 (Satori) + any future tier
            };
            let tex = tool_texture(mat);
            // Thin slab oriented along Y so it reads as a tool shaft when the
            // caller tilts it into the hand.
            let mut verts = Vec::with_capacity(36);
            push_cuboid(&mut verts, Vec3::ZERO, Vec3::new(0.12, 0.6, 0.12), tex, tex, tex);
            verts
        }
        ItemRef::Material(id) => {
            // ItemRef::Material carries the `MaterialId as u16` discriminant
            // index (see inventory::item_to_ref). Recover the MaterialId via
            // its TryFrom inverse and use the authoritative per-material item
            // texture. An unknown id (e.g. a newer peer's material this build
            // doesn't know) falls back to the stick icon.
            let tex = MaterialId::try_from(id).ok().map(material_texture).unwrap_or(TEX_ITEM_STICK);
            let mut verts = Vec::with_capacity(36);
            push_cuboid(&mut verts, Vec3::ZERO, Vec3::splat(0.3), tex, tex, tex);
            verts
        }
    }
}

/// Emit a textured axis-aligned cuboid centred at `centre` with full size
/// `size` (so it spans `centre +/- size/2` on each axis). `tex_top`,
/// `tex_side`, `tex_bottom` are the texture array layers for the +Y face,
/// the four side faces, and the -Y face respectively. Faces are wound to
/// match the entity pass (`push_textured_quad`). Always emits 36 vertices.
fn push_cuboid(
    verts: &mut Vec<Vertex>,
    centre: Vec3,
    size: Vec3,
    tex_top: u32,
    tex_side: u32,
    tex_bottom: u32,
) {
    let h = size * 0.5;
    let c = [
        centre + Vec3::new(-h.x, -h.y, -h.z),
        centre + Vec3::new(h.x, -h.y, -h.z),
        centre + Vec3::new(h.x, h.y, -h.z),
        centre + Vec3::new(-h.x, h.y, -h.z),
        centre + Vec3::new(-h.x, -h.y, h.z),
        centre + Vec3::new(h.x, -h.y, h.z),
        centre + Vec3::new(h.x, h.y, h.z),
        centre + Vec3::new(-h.x, h.y, h.z),
    ];

    push_textured_quad(verts, tex_side, [1.0, 0.0, 0.0], c[1], c[5], c[6], c[2]); // +X
    push_textured_quad(verts, tex_side, [-1.0, 0.0, 0.0], c[4], c[0], c[3], c[7]); // -X
    push_textured_quad(verts, tex_top, [0.0, 1.0, 0.0], c[3], c[2], c[6], c[7]); // +Y
    push_textured_quad(verts, tex_bottom, [0.0, -1.0, 0.0], c[4], c[5], c[1], c[0]); // -Y
    push_textured_quad(verts, tex_side, [0.0, 0.0, 1.0], c[5], c[4], c[7], c[6]); // +Z
    push_textured_quad(verts, tex_side, [0.0, 0.0, -1.0], c[0], c[1], c[2], c[3]); // -Z
}

// ── The sprite ───────────────────────────────────────────────────────────────

/// One face of a sprite in its own space: the picture lies in the XY plane,
/// centred on the origin, +X to its right and +Y its top, the front facing +Z.
/// The corners wind counter-clockwise seen from the side `normal` points to
/// (the chunk pipeline culls back faces).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteQuad {
    pub corners: [Vec3; 4],
    pub uvs: [[f32; 2]; 4],
    pub normal: Vec3,
}

/// Texels along a side of an icon (the stock textures are 16x16).
const ICON: usize = 16;

/// How far (in texels) an edge strip's UVs are pulled in from its texel's
/// edges, so nearest sampling never reads the transparent texel beside it.
const EDGE_INSET: f32 = 0.02;

/// Opaque texels of an icon: bit `x` of `mask[y]` is texel (column `x`, row
/// `y`, row 0 at the top) with alpha >= 128, the cut `fs_main` makes.
type Mask = [u16; ICON];

/// Every texel opaque: a plain square card.
const SOLID: Mask = [0xFFFF; ICON];

fn mask_of(rgba: &[u8]) -> Mask {
    if rgba.len() != ICON * ICON * 4 {
        return SOLID;
    }
    let mut m = [0u16; ICON];
    for (y, row) in m.iter_mut().enumerate() {
        for x in 0..ICON {
            if rgba[(y * ICON + x) * 4 + 3] >= 128 {
                *row |= 1 << x;
            }
        }
    }
    m
}

/// The opaque texels of every stock texture layer (`texture_gen`, the set the
/// GPU array starts from), worked out once.
fn stock_masks() -> &'static [Mask] {
    static MASKS: OnceLock<Vec<Mask>> = OnceLock::new();
    MASKS.get_or_init(|| crate::texture_gen::generate_textures().iter().map(|t| mask_of(t.as_slice())).collect())
}

/// The opaque texels of layer `layer`. A layer past the stock set (one a
/// Workshop design or a pack appends at run time) is a plain card; a pack that
/// redraws a stock layer keeps the stock outline, and the cutout bit on every
/// sprite face drops whatever of it the new art leaves transparent.
fn layer_mask(layer: u32) -> Mask {
    stock_masks().get(layer as usize).copied().unwrap_or(SOLID)
}

fn opaque(mask: &Mask, x: i32, y: i32) -> bool {
    (0..ICON as i32).contains(&x) && (0..ICON as i32).contains(&y) && (mask[y as usize] >> x) & 1 == 1
}

/// The runs `[start, end)` of consecutive `i` in `0..ICON` for which `on(i)`.
fn runs(on: impl Fn(usize) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for i in 0..=ICON {
        match (i < ICON && on(i), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// A quad wound counter-clockwise about `normal`: the corner order is
/// reversed if it isn't.
fn quad(corners: [Vec3; 4], uvs: [[f32; 2]; 4], normal: Vec3) -> SpriteQuad {
    let [a, b, c, d] = corners;
    if (b - a).cross(c - a).dot(normal) >= 0.0 {
        SpriteQuad { corners, uvs, normal }
    } else {
        SpriteQuad { corners: [a, d, c, b], uvs: [uvs[0], uvs[3], uvs[2], uvs[1]], normal }
    }
}

/// The faces of layer `layer`'s sprite, `width` across, in sprite space (see
/// [`SpriteQuad`]). The front and back show the whole icon; each edge is one
/// quad per run of opaque texels whose neighbour across that edge is
/// transparent (or off the icon), showing those texels' 1-texel strip across
/// the thickness. The back shows each texel behind its front twin, so seen
/// from behind the picture is mirrored, like a cut-out.
pub fn sprite_quads(layer: u32, width: f32) -> Vec<SpriteQuad> {
    let mask = layer_mask(layer);
    let n = ICON as f32;
    let t = width / n; // one texel
    let h = width * 0.5;
    let d = width * SPRITE_DEPTH * 0.5;
    // The left edge of column x and the top edge of row y, in sprite space.
    let xe = |x: usize| -h + x as f32 * t;
    let ye = |y: usize| h - y as f32 * t;
    // A texel span `lo..hi` (whole texels) as a UV span, pulled in a hair.
    let span = |lo: usize, hi: usize| ((lo as f32 + EDGE_INSET) / n, (hi as f32 - EDGE_INSET) / n);
    let v3 = Vec3::new;

    let mut quads = vec![
        quad(
            [v3(-h, -h, d), v3(h, -h, d), v3(h, h, d), v3(-h, h, d)],
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            Vec3::Z,
        ),
        quad(
            [v3(h, -h, -d), v3(-h, -h, -d), v3(-h, h, -d), v3(h, h, -d)],
            [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
            Vec3::NEG_Z,
        ),
    ];
    for x in 0..ICON {
        let xi = x as i32;
        let (u0, u1) = span(x, x + 1);
        // Left edges (-X) of column x, then right edges (+X).
        for (dir, side_x) in [(-1, xe(x)), (1, xe(x + 1))] {
            for (y0, y1) in runs(|y| opaque(&mask, xi, y as i32) && !opaque(&mask, xi + dir, y as i32)) {
                let (vt, vb) = span(y0, y1);
                let (yt, yb) = (ye(y0), ye(y1));
                quads.push(quad(
                    [v3(side_x, yb, -d), v3(side_x, yb, d), v3(side_x, yt, d), v3(side_x, yt, -d)],
                    [[u0, vb], [u1, vb], [u1, vt], [u0, vt]],
                    Vec3::X * dir as f32,
                ));
            }
        }
    }
    for y in 0..ICON {
        let yi = y as i32;
        let (v0, v1) = span(y, y + 1);
        // Top edges (+Y) of row y, then bottom edges (-Y).
        for (dir, side_y) in [(-1, ye(y)), (1, ye(y + 1))] {
            for (x0, x1) in runs(|x| opaque(&mask, x as i32, yi) && !opaque(&mask, x as i32, yi + dir)) {
                let (ul, ur) = span(x0, x1);
                let (xl, xr) = (xe(x0), xe(x1));
                quads.push(quad(
                    [v3(xl, side_y, d), v3(xr, side_y, d), v3(xr, side_y, -d), v3(xl, side_y, -d)],
                    [[ul, v1], [ur, v1], [ur, v0], [ul, v0]],
                    // Row y - 1 is above row y: a top edge faces +Y.
                    Vec3::Y * -dir as f32,
                ));
            }
        }
    }
    quads
}

/// [`sprite_quads`] as a triangle list for the entity passes (held and
/// dropped items): every vertex on layer `layer` with [`CUTOUT_FLAG`] set and
/// the placeholder light the caller stamps.
pub fn sprite_triangles(layer: u32, width: f32) -> Vec<Vertex> {
    let quads = sprite_quads(layer, width);
    let mut verts = Vec::with_capacity(quads.len() * 6);
    for q in &quads {
        for i in [0, 1, 2, 0, 2, 3] {
            verts.push(Vertex {
                position: q.corners[i].to_array(),
                normal: q.normal.to_array(),
                tex_layer: layer | CUTOUT_FLAG,
                uv: q.uvs[i],
                light: Vertex::FULL_BRIGHT,
                sky_light: 0.0,
            });
        }
    }
    verts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::mesh::{self, build_chunk_meshes};
    use crate::protocol::ItemRef;
    use crate::world::World;

    fn test_registry() -> BlockRegistry {
        BlockRegistry::new()
    }

    #[test]
    fn empty_has_no_verts() {
        assert!(held_item_mesh(ItemRef::Empty, &test_registry()).is_empty());
    }

    #[test]
    fn block_makes_a_cube() {
        assert_eq!(held_item_mesh(ItemRef::Block(1), &test_registry()).len(), 36);
    }

    #[test]
    fn tool_makes_nonempty_slab() {
        assert_eq!(held_item_mesh(ItemRef::Tool(2), &test_registry()).len(), 36);
    }

    #[test]
    fn satori_tier_falls_back_not_panics() {
        let _ = held_item_mesh(ItemRef::Tool(4), &test_registry());
    }

    #[test]
    fn material_makes_nonempty_cube() {
        assert_eq!(held_item_mesh(ItemRef::Material(0), &test_registry()).len(), 36);
    }

    #[test]
    fn tool_tier_uses_authoritative_texture() {
        // The in-hand tool texture must match the dropped-item lookup, so a
        // Satori tool (tier 4) is NOT the diamond layer. This guards against
        // the in-hand vs on-ground mismatch the review caught.
        assert_eq!(tool_texture(ToolMaterial::Satori), crate::block::TEX_ITEM_SATORI);
        assert_ne!(tool_texture(ToolMaterial::Satori), tool_texture(ToolMaterial::Diamond));
    }

    #[test]
    fn material_resolves_to_its_own_texture() {
        // A known non-stick material (Diamond) must resolve to its own item
        // texture, not the stick fallback — proving the u16 -> MaterialId
        // path is wired up rather than always returning the placeholder.
        let diamond_id = MaterialId::Diamond as u16;
        assert_eq!(
            MaterialId::try_from(diamond_id).map(material_texture),
            Ok(material_texture(MaterialId::Diamond))
        );
        assert_ne!(material_texture(MaterialId::Diamond), TEX_ITEM_STICK);
    }

    #[test]
    fn a_torch_is_a_sprite_and_stone_is_a_cube() {
        let reg = test_registry();
        assert_eq!(block_look(block::TORCH, &reg), ItemLook::Sprite { layer: reg.tex_top(block::TORCH) });
        assert_eq!(
            block_look(block::STONE, &reg),
            ItemLook::Cube {
                top: reg.tex_top(block::STONE),
                side: reg.tex_side(block::STONE),
                bottom: reg.tex_bottom(block::STONE),
            }
        );
        // The held stone is still the 36-vertex cube; the held torch is not.
        assert_eq!(held_item_mesh(ItemRef::Block(block::STONE), &reg).len(), 36);
        let torch = held_item_mesh(ItemRef::Block(block::TORCH), &reg);
        assert!(!torch.is_empty() && torch.len() != 36, "a held torch is a sprite, not a cube");
    }

    #[test]
    fn a_sprite_is_a_sixteenth_of_its_width_deep() {
        for width in [HELD_SPRITE_WIDTH, DROP_SPRITE_WIDTH, FRAME_SPRITE_WIDTH] {
            let quads = sprite_quads(block::TEX_TORCH, width);
            let mut lo = Vec3::splat(f32::MAX);
            let mut hi = Vec3::splat(f32::MIN);
            for p in quads.iter().flat_map(|q| q.corners) {
                lo = lo.min(p);
                hi = hi.max(p);
            }
            let size = hi - lo;
            assert!((size.x - width).abs() < 1e-6 && (size.y - width).abs() < 1e-6, "{width} across: {size}");
            assert!((size.z - width / 16.0).abs() < 1e-6, "{width} wide: depth {} is not 1/16 of it", size.z);
            assert!((lo + hi).length() < 1e-6, "centred on the origin: {lo} .. {hi}");
        }
    }

    /// Texel `(x, y)` of a texture layer.
    fn texel(tex: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * 16 + x) * 4;
        [tex[i], tex[i + 1], tex[i + 2], tex[i + 3]]
    }

    #[test]
    fn a_torch_sprite_is_cut_from_its_opaque_texels() {
        let textures = crate::texture_gen::generate_textures();
        let tex = &textures[block::TEX_TORCH as usize];
        let quads = sprite_quads(block::TEX_TORCH, 1.0);
        // Front and back, and the merged edge runs round the flame and the
        // handle (12 quads by hand count), well below one quad per texel.
        assert!(quads.len() <= 16, "{} quads", quads.len());
        for q in &quads[2..] {
            // An edge strip stays inside opaque texels: every texel its UV
            // rect touches is opaque, so its colour is the torch's own.
            let (u0, u1) = q.uvs.iter().fold((f32::MAX, f32::MIN), |a, uv| (a.0.min(uv[0]), a.1.max(uv[0])));
            let (v0, v1) = q.uvs.iter().fold((f32::MAX, f32::MIN), |a, uv| (a.0.min(uv[1]), a.1.max(uv[1])));
            let cols = (u0 * 16.0).floor() as usize..=((u1 * 16.0).ceil() as usize - 1);
            let rows = (v0 * 16.0).floor() as usize..=((v1 * 16.0).ceil() as usize - 1);
            assert!(
                cols.clone().count() == 1 || rows.clone().count() == 1,
                "an edge strip is one texel across: cols {cols:?} rows {rows:?}"
            );
            for y in rows.clone() {
                for x in cols.clone() {
                    assert_eq!(texel(tex, x, y)[3], 255, "edge {:?} samples transparent texel ({x}, {y})", q.normal);
                }
            }
        }
    }

    #[test]
    fn a_held_torch_clears_the_arm() {
        // The avatar's right arm is 0.25 wide round the hand anchor, with its
        // sleeve overlay a hair outside that. The torch's art (its edge quads
        // trace the outline) lies wholly outside the arm, on the outer side, so
        // its flame isn't hidden inside the forearm.
        let arm = &crate::entity_model::player_model_for(crate::skin_uv::ArmModel::Classic)[3];
        let clear = arm.size.x * 0.5 + 0.04;
        let verts = held_item_mesh(ItemRef::Block(block::TORCH), &test_registry());
        // Six vertices a quad; the first two quads are the front and back squares.
        assert!(verts.len() > 12);
        for v in &verts[12..] {
            assert!(v.position[0] > clear, "torch art at {:?} is inside the arm (half width {clear})", v.position);
        }
    }

    #[test]
    fn sprite_faces_wind_outward() {
        // The chunk pipeline culls back faces: each triangle's winding (from
        // its positions) must face the way its normal says.
        let reg = test_registry();
        let verts = held_item_mesh(ItemRef::Block(block::TORCH), &reg);
        assert!(verts.len().is_multiple_of(3));
        for tri in verts.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|v| Vec3::from(v.position));
            let n = Vec3::from(tri[0].normal);
            assert!((b - a).cross(c - a).dot(n) > 0.0, "a triangle winds against its normal {n}");
        }
    }

    #[test]
    fn only_sprite_vertices_carry_the_cutout_bit() {
        let reg = test_registry();
        let cut = |v: &Vertex| v.tex_layer & CUTOUT_FLAG != 0;
        // Held.
        let torch = held_item_mesh(ItemRef::Block(block::TORCH), &reg);
        assert!(torch.iter().all(cut), "every held-torch vertex is cut out");
        assert_eq!(torch[0].tex_layer & 0xFFFF, block::TEX_TORCH);
        for item in [ItemRef::Block(block::STONE), ItemRef::Tool(2), ItemRef::Material(0)] {
            assert!(!held_item_mesh(item, &reg).iter().any(cut), "{item:?} carries no cutout bit");
        }
        // Dropped.
        let mut ecs = hecs::World::new();
        crate::entity::spawn_item(&mut ecs, Vec3::new(0.0, 65.0, 0.0), crate::item::ItemStack::new_block(block::TORCH, 1), 0);
        let torch_drop = crate::entity_model::build_item_entity_vertices(&ecs, &reg, 0, Vec3::new(9.0, 70.0, 9.0), |_| (1.0, 0.0));
        assert!(!torch_drop.is_empty() && torch_drop.iter().all(cut), "a dropped torch is a cut-out sprite");
        let mut ecs = hecs::World::new();
        crate::entity::spawn_item(&mut ecs, Vec3::new(0.0, 65.0, 0.0), crate::item::ItemStack::new_block(block::STONE, 1), 0);
        let stone_drop = crate::entity_model::build_item_entity_vertices(&ecs, &reg, 0, Vec3::new(9.0, 70.0, 9.0), |_| (1.0, 0.0));
        assert_eq!(stone_drop.len(), 36, "a dropped stone block is still the drop cube");
        assert!(!stone_drop.iter().any(cut));
        // In the world: placed blocks never carry it, a framed stone neither;
        // a framed torch's sprite does.
        let mut w = World::new();
        for (i, id) in [block::TORCH, block::WHEAT_STAGE_3, block::STONE, block::GLASS, crate::rail::TRACK].into_iter().enumerate() {
            w.set_block(2 + 2 * i as i32, 3, 2, id);
        }
        let framed = |w: &mut World, at: (i32, i32, i32), id: BlockId| {
            w.set_block(at.0, at.1, at.2, block::ITEM_FRAME);
            w.set_meta(at, crate::meta::with_facing(0, crate::meta::Facing::South));
            let mut f = crate::item_frame::ItemFrameData::new();
            assert!(f.try_insert(crate::item::ItemStack::new_block(id, 1)));
            w.insert_item_frame(at, f);
        };
        framed(&mut w, (2, 6, 2), block::STONE);
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);
        assert!(!m.opaque.vertices.iter().chain(&m.transparent.vertices).any(cut), "no placed block is cut out");
        framed(&mut w, (6, 6, 2), block::TORCH);
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);
        let flagged: Vec<&Vertex> = m.opaque.vertices.iter().filter(|&v| cut(v)).collect();
        assert_eq!(flagged.len(), 4 * sprite_quads(block::TEX_TORCH, FRAME_SPRITE_WIDTH).len(), "the framed torch's sprite");
        assert!(flagged.iter().all(|v| (6.0..=7.0).contains(&v.position[0]) && (6.0..=7.0).contains(&v.position[1])));
    }

    /// Whether the mesher draws one `id` at (5, 5, 5) as something an item
    /// shows as a sprite: a cross billboard, a flat rail or cable (every face
    /// up), or the torch's emissive pole.
    fn meshed_as_sprite(id: BlockId, reg: &BlockRegistry) -> bool {
        let mut w = World::new();
        w.set_block(0, 0, 0, block::AIR);
        w.set_block(5, 5, 5, id);
        let m = build_chunk_meshes(0, 0, 0, &w, reg);
        let o = &m.opaque.vertices;
        !m.plants.is_empty()
            || (!o.is_empty() && o.iter().all(|v| v.normal == [0.0, 1.0, 0.0]))
            || o.iter().any(|v| v.light > 1.5)
    }

    #[test]
    fn every_block_item_follows_how_the_mesher_draws_its_block() {
        // The shape rule covers every registered id: the item is a sprite
        // exactly when the mesher draws the block as a billboard, a flat rail
        // or the torch, checked against what the mesher really emits.
        let reg = test_registry();
        let mut sprites = Vec::new();
        for id in 0..reg.len() as BlockId {
            let look = block_look(id, &reg);
            let class = mesh::render_class(id, &reg);
            let sprite = matches!(look, ItemLook::Sprite { .. });
            assert_eq!(sprite, meshed_as_sprite(id, &reg), "block {id} ({}) is {class:?}, item {look:?}", reg.get(id).name);
            if let ItemLook::Sprite { layer } = look {
                assert_eq!(layer, reg.tex_top(id), "a sprite shows the inventory icon");
                sprites.push(id);
            }
        }
        for id in [block::TORCH, block::TALL_GRASS, block::WHEAT_STAGE_3, block::CORNFLOWER, block::LADDER, crate::rail::TRACK, block::CABLE] {
            assert!(sprites.contains(&id), "block {id} ({}) is a sprite", reg.get(id).name);
        }
        for id in [block::STONE, block::GLASS, block::OAK_LEAVES, block::ITEM_FRAME, block::CAMPFIRE_SMOKE] {
            assert!(!sprites.contains(&id), "block {id} ({}) is a cube", reg.get(id).name);
        }
    }

    #[test]
    fn sprite_sizes_stay_bounded() {
        // Every sprite-class block's sprite, edges merged into runs, stays well
        // under one quad per texel. Also lists the icons that have no
        // transparent texel at all (they show as a square card).
        let reg = test_registry();
        let mut most = (0usize, 0 as BlockId);
        let mut cards = Vec::new();
        for id in 0..reg.len() as BlockId {
            if let ItemLook::Sprite { layer } = block_look(id, &reg) {
                let n = sprite_quads(layer, 1.0).len();
                if n > most.0 {
                    most = (n, id);
                }
                if layer_mask(layer) == SOLID {
                    cards.push(reg.get(id).name);
                }
            }
        }
        eprintln!("ITEMMODEL most quads: {} ({}); square-card icons: {cards:?}", most.0, reg.get(most.1).name);
        assert!(most.0 <= 256, "{} quads for {}", most.0, reg.get(most.1).name);
        assert!(!cards.contains(&reg.get(block::TORCH).name), "the torch icon has a transparent background");
    }

    #[test]
    fn a_layer_past_the_stock_set_is_a_plain_card() {
        let past = crate::texture_gen::texture_count() + 7;
        let quads = sprite_quads(past, 1.0);
        assert_eq!(quads.len(), 6, "front, back and the four border edges");
    }
}

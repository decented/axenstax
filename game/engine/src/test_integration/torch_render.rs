//! Small cubes show the right part of their texture: the torch, the button,
//! the pressure plate, the lever's handle and a framed block.
//!
//! A small cube is a box smaller than its block, so each face samples only a
//! window of the 16x16 texture. These tests mesh one block with the real
//! mesher, take each face's UV rect and read the texels under it from the real
//! generated textures, so a window on the wrong part of the art fails: a torch
//! drawn as its dark background, a button showing its stone rim instead of its
//! pad, a plate missing half its bevel, a grey lever handle, a framed block
//! showing a quarter of its texture, or a side face with its art upside down.

use crate::block::{self, BlockId, BlockRegistry};
use crate::block_shape::{self, Aabb, BlockShape};
use crate::item::ItemStack;
use crate::item_frame::ItemFrameData;
use crate::mesh::{self, Vertex};
use crate::meta::{self, Facing};
use crate::texture_gen;
use crate::world::World;

/// Where the block under test sits (inside chunk (0, 0, 0)).
const AT: (i32, i32, i32) = (5, 3, 5);
/// Texels along one side of a block texture.
const TEXELS: f32 = 16.0;
/// A UV span shows a texel only if it overlaps it by more than this many
/// texels (so float noise at an exact texel edge doesn't count as bleed).
const HAIR: f32 = 1e-3;
/// UV and position tolerance.
const EPS: f32 = 1e-5;

type Quad = [Vertex; 4];

/// A world holding `id` with meta `m` at `AT`, and nothing else.
fn world_with(id: BlockId, m: u8) -> World {
    let mut w = World::new();
    w.set_block(AT.0, AT.1, AT.2, id);
    w.set_meta(AT, m);
    w
}

/// The opaque mesh of chunk (0, 0, 0), four vertices to a face.
fn faces_of(world: &World) -> Vec<Quad> {
    let m = mesh::build_chunk_meshes(0, 0, 0, world, &BlockRegistry::new());
    let v = &m.opaque.vertices;
    assert!(!v.is_empty() && v.len().is_multiple_of(4), "whole quads expected, got {} vertices", v.len());
    v.chunks_exact(4).map(|q| [q[0], q[1], q[2], q[3]]).collect()
}

/// A face's UV rect: `[u_min, v_min, u_max, v_max]`.
fn uv_rect(f: &Quad) -> [f32; 4] {
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for v in f {
        r[0] = r[0].min(v.uv[0]);
        r[1] = r[1].min(v.uv[1]);
        r[2] = r[2].max(v.uv[0]);
        r[3] = r[3].max(v.uv[1]);
    }
    r
}

/// The texel columns (or rows) a UV span `lo..hi` shows under nearest
/// sampling: every texel it overlaps by more than a hair.
fn span(lo: f32, hi: f32) -> std::ops::RangeInclusive<i32> {
    let first = (lo * TEXELS + HAIR).floor() as i32;
    let last = (hi * TEXELS - HAIR).ceil() as i32 - 1;
    first..=last
}

/// Every texel a face shows, `(x, y, rgb)`, read at its centre from the face's
/// own texture layer (texel `(x, y)` is row `y`, column `x`, as `texture_gen`
/// writes it). Asserts the face shows at least one, so "no background" or
/// "only wood" can't pass on an empty window.
fn shown(textures: &[Vec<u8>], f: &Quad) -> Vec<(i32, i32, [u8; 3])> {
    let layer = &textures[f[0].tex_layer as usize];
    let [u0, v0, u1, v1] = uv_rect(f);
    let mut out = Vec::new();
    for y in span(v0, v1) {
        for x in span(u0, u1) {
            let i = ((y.rem_euclid(16) * 16 + x.rem_euclid(16)) * 4) as usize;
            out.push((x, y, [layer[i], layer[i + 1], layer[i + 2]]));
        }
    }
    assert!(!out.is_empty(), "face {:?} (uv {:?}) shows no texel", f[0].normal, uv_rect(f));
    out
}

/// Position relative to the block's min corner.
fn local(v: &Vertex) -> [f32; 3] {
    [v.position[0] - AT.0 as f32, v.position[1] - AT.1 as f32, v.position[2] - AT.2 as f32]
}

/// Whether every corner of `f` lies in box `b` (block-local).
fn inside(f: &Quad, b: Aabb) -> bool {
    f.iter().all(|v| {
        let p = local(v);
        (0..3).all(|a| p[a] >= b.min[a] - EPS && p[a] <= b.max[a] + EPS)
    })
}

/// The face with outward normal `n`.
fn face(faces: &[Quad], n: [f32; 3]) -> Quad {
    *faces.iter().find(|f| f[0].normal == n).unwrap_or_else(|| panic!("no face with normal {n:?}"))
}

/// Anchored UVs: the texture laid over the whole block, so each corner samples
/// the texel under it. Top and bottom: u = local x, v = local z. Side faces:
/// u = local x (Z faces) or local z (X faces), v = 1 - local y, so the top edge
/// of a side samples the top of the texture and upside-down art fails here.
fn assert_anchored(what: &str, faces: &[Quad]) {
    for f in faces {
        let n = f[0].normal;
        for v in f {
            let l = local(v);
            let want = if n[1] != 0.0 {
                [l[0], l[2]]
            } else if n[2] != 0.0 {
                [l[0], 1.0 - l[1]]
            } else {
                [l[2], 1.0 - l[1]]
            };
            assert!(
                (v.uv[0] - want[0]).abs() < EPS && (v.uv[1] - want[1]).abs() < EPS,
                "{what}: face {n:?} corner at local {l:?} has uv {:?}, anchored wants {want:?}",
                v.uv,
            );
        }
    }
}

#[test]
fn torch_faces_show_the_flame_and_handle_not_the_background() {
    // `gen_torch`: a dark (18, 14, 10) background, a (110, 75, 35) handle in
    // columns 7..9 rows 8..16, a flame in rows 3..8 and a hot core at columns
    // 7..9 rows 4..6. A torch drawn from the texture's corner shows only the
    // background: a black stick.
    let textures = texture_gen::generate_textures();
    let faces = faces_of(&world_with(block::TORCH, 0));
    assert_eq!(faces.len(), 6, "a torch is one six-faced small cube");
    let flame = |c: [u8; 3]| c[0] >= 200 && c[1] >= 150;
    let handle = |c: [u8; 3]| c == [110, 75, 35];
    let background = |c: [u8; 3]| c == [18, 14, 10];
    for f in &faces {
        assert_eq!(f[0].tex_layer, block::TEX_TORCH, "a torch face samples the torch texture");
        let n = f[0].normal;
        let px = shown(&textures, f);
        let rect = uv_rect(f);
        if n[1] == 0.0 {
            assert!(px.iter().any(|t| flame(t.2)), "side {n:?} (uv {rect:?}) shows no flame: {px:?}");
            assert!(px.iter().any(|t| handle(t.2)), "side {n:?} (uv {rect:?}) shows no handle: {px:?}");
            assert!(
                !px.iter().any(|t| background(t.2)),
                "side {n:?} (uv {rect:?}) shows the dark background: {px:?}",
            );
        } else if n[1] > 0.0 {
            assert!(px.iter().all(|t| flame(t.2)), "the top (uv {rect:?}) is not all flame: {px:?}");
        } else {
            assert!(px.iter().all(|t| handle(t.2)), "the bottom (uv {rect:?}) is not all handle: {px:?}");
        }
    }
}

#[test]
fn button_top_shows_its_raised_pad() {
    // `gen_button`: the raised pad is texels 5..=10 in both axes, exactly the
    // button's 6/16 footprint (5/16..11/16). A floor button's top must show
    // the whole pad and nothing else.
    let textures = texture_gen::generate_textures();
    let faces = faces_of(&world_with(block::BUTTON, meta::with_facing(0, Facing::Up)));
    assert_eq!(faces.len(), 6, "a button is one six-faced box");
    let top = face(&faces, [0.0, 1.0, 0.0]);
    let [u0, v0, u1, v1] = uv_rect(&top);
    assert_eq!(span(u0, u1), 5..=10, "the top shows the pad's columns (uv {:?})", uv_rect(&top));
    assert_eq!(span(v0, v1), 5..=10, "the top shows the pad's rows (uv {:?})", uv_rect(&top));
    for (x, y, c) in shown(&textures, &top) {
        // Pad red 146..=154; the stone rim 108..=116.
        assert!(c[0] >= 130, "texel ({x}, {y}) on the button's top is rim, not pad: {c:?}");
    }
    assert_anchored("button", &faces);
}

#[test]
fn pressure_plate_top_shows_its_bevel_on_all_four_edges() {
    // `gen_pressure_plate`: a bright bevel lip two texels wide round all four
    // edges, a sunken centre. The plate spans 1/16..15/16, so its top shows
    // texels 1..=14, whose outer ring is all lip.
    let textures = texture_gen::generate_textures();
    let faces = faces_of(&world_with(block::PRESSURE_PLATE, 0));
    assert_eq!(faces.len(), 6, "a pressure plate is one six-faced box");
    let top = face(&faces, [0.0, 1.0, 0.0]);
    let [u0, v0, u1, v1] = uv_rect(&top);
    let (xs, ys) = (span(u0, u1), span(v0, v1));
    // Lip red 136..=150; the sunken centre 100..=114.
    let lip = |c: [u8; 3]| c[0] >= 125;
    let px = shown(&textures, &top);
    for &(x, y, c) in &px {
        let edge = x == *xs.start() || x == *xs.end() || y == *ys.start() || y == *ys.end();
        if edge {
            assert!(lip(c), "edge texel ({x}, {y}) of the plate's top (uv {:?}) is not lip: {c:?}", uv_rect(&top));
        }
    }
    assert!(px.iter().any(|t| !lip(t.2)), "the plate's sunken centre shows too");
    assert_anchored("pressure plate", &faces);
}

#[test]
fn lever_handle_shows_only_wood() {
    // `gen_lever`: a grey plate crossed by a wooden diagonal, |x - (15 - y)| <= 1,
    // with a red knob in the top-right corner. The handle stub (box 1 of the
    // lever) is wood; drawn from the texture's corner it shows grey plate.
    let textures = texture_gen::generate_textures();
    let m = meta::with_facing(0, Facing::Up);
    let faces = faces_of(&world_with(block::LEVER, m));
    let handle = block_shape::render_cuboids(BlockShape::Lever, m)[1];
    let on_handle: Vec<&Quad> = faces.iter().filter(|f| inside(f, handle)).collect();
    assert_eq!(on_handle.len(), 6, "the handle is a six-faced box");
    // Wood is (96, 64, 34) plus the same noise on every channel; grey has
    // r ~ b and the knob g ~ b.
    let wood = |c: [u8; 3]| c[0] >= c[1] + 20 && c[1] >= c[2] + 20;
    for f in on_handle {
        for (x, y, c) in shown(&textures, f) {
            assert!(
                wood(c),
                "texel ({x}, {y}) on the handle's {:?} face (uv {:?}) is not wood: {c:?}",
                f[0].normal,
                uv_rect(f),
            );
        }
    }
}

#[test]
fn framed_block_shows_its_whole_texture_on_every_face() {
    // A framed block is a half-block cube off the frame's plate. Like a held or
    // dropped block it shows its whole texture on every face; drawn from the
    // texture's corner it shows the top-left quarter (a framed torch is black).
    let m = meta::with_facing(0, Facing::South);
    let mut w = world_with(block::ITEM_FRAME, m);
    let mut frame = ItemFrameData::new();
    assert!(frame.try_insert(ItemStack::new_block(block::STONE, 1)), "an empty frame takes a block");
    w.insert_item_frame(AT, frame);
    let faces = faces_of(&w);
    let cube = block_shape::item_frame_item_cube(Facing::South);
    let framed: Vec<&Quad> = faces.iter().filter(|f| inside(f, cube)).collect();
    assert_eq!(framed.len(), 6, "the framed block is a six-faced cube off the plate");
    for f in framed {
        let r = uv_rect(f);
        let whole = [0.0_f32, 0.0, 1.0, 1.0];
        assert!(
            (0..4).all(|i| (r[i] - whole[i]).abs() < EPS),
            "the framed block's {:?} face shows uv {r:?}, not the whole texture",
            f[0].normal,
        );
    }
}

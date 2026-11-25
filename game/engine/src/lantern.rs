//! The hand lantern (off-hand plan, slice 4; Spec 05 §4): one block, `LANTERN`
//! (id 380), that is held, placed and hung.
//!
//! - **One id, three uses.** The same `Item::Block(LANTERN)` sits in a hand, stands
//!   on the top of a block, or hangs from the underside of one. Light 15 when
//!   placed (`BlockRegistry::light_emission`); torches stay 14 and paper lanterns
//!   12. It needs a support (pt1124, `support.rs`): a standing lantern the block
//!   below it, a hanging one the block or lantern above it; lose it and it drops.
//! - **Hanging is a state bit.** Placing it against the underside of a block
//!   (`face == [0, -1, 0]`) stamps [`HANGING_STATE`] into the meta byte
//!   ([`placement_meta`], called from `placement_meta::placement_meta`), the
//!   mesher reads it ([`model_key`]) and draws the hanging model.
//! - **Look.** A small sub-voxel model (a micro-model, `micro_model.rs`), painted
//!   from EXISTING texture layers only: the dark wallpaper-black frame, a grey cap
//!   and a warm yellow core. The web build's texture array is exactly full (512
//!   layers), so there is no new layer. The standing and the hanging model are two
//!   registered shells: the standing one under the block's own id, the hanging one
//!   under [`HANGING_MODEL_KEY`] (a key that is not a block id). In the hand
//!   ([`held_mesh`]) the standing model is drawn small, not a sprite.
//! - **On the ground** ([`drop_mesh`]) it is the same model, smaller
//!   ([`DROP_SCALE`]), not a cube; the inventory slot shows a code-drawn icon
//!   (`hud_icons::lantern`), because its top face is a plain fill.
//! - **Not built here:** collision (non-solid, like a torch) and the far-LOD
//!   billboard (the generic one, from the block's own layer).

use std::sync::OnceLock;

use crate::block::{self, BlockId, BlockRegistry, LANTERN};
use crate::mesh::Vertex;
use crate::meta;
use crate::micro_model::{bake_micro_model, MicroModelData, MicroVoxel, MICRO_MODEL_VERSION, MICRO_SCALE_16};

/// The state bit (`meta::state`) of a hanging lantern.
pub const HANGING_STATE: u8 = 0b01;

/// Light a placed lantern gives (`light_emission`).
pub const LIGHT_LEVEL: u8 = 15;

/// The micro-model registry key of the hanging model. Not a block id (ids stop
/// at 380 and the registry rejects anything unknown): the standing model is under
/// `LANTERN` itself, so `micro_registry.contains(LANTERN)` stays the one question
/// the mesher asks about a lantern cell.
pub const HANGING_MODEL_KEY: BlockId = 0x8000 | LANTERN;

/// Whether the meta byte of a lantern says it hangs.
pub fn is_hanging(meta_byte: u8) -> bool {
    meta::state(meta_byte) & HANGING_STATE != 0
}

/// The meta a lantern placed on `face` (the clicked face's outward normal)
/// stamps: hanging when the click was on the underside of a block, standing on
/// any other face.
pub fn placement_meta(face: [i32; 3]) -> u8 {
    if face == [0, -1, 0] {
        meta::with_state(0, HANGING_STATE)
    } else {
        0
    }
}

/// The micro-model registry key the mesher draws a lantern cell with (`meta` is
/// the cell's meta byte).
pub fn model_key(meta_byte: u8) -> BlockId {
    if is_hanging(meta_byte) {
        HANGING_MODEL_KEY
    } else {
        LANTERN
    }
}

fn fill(v: &mut Vec<MicroVoxel>, x: (u8, u8), y: (u8, u8), z: (u8, u8), id: BlockId) {
    for mx in x.0..x.1 {
        for my in y.0..y.1 {
            for mz in z.0..z.1 {
                v.push(MicroVoxel { mx, my, mz, block_id: id });
            }
        }
    }
}

/// The lantern body and bail, `lift` sixteenths up from the cell floor.
///
/// Eight sixteenths across; a base plate, four thin corner posts round a yellow core
/// six across, a top plate, a grey cap under a black one, and a bail (two uprights
/// and a bar). Thirteen sixteenths tall.
fn body(lift: u8) -> Vec<MicroVoxel> {
    let frame = block::WALLPAPER_BLACK;
    let cap = block::WALLPAPER_GREY;
    let core = block::WALLPAPER_YELLOW;
    let mut v = Vec::new();
    let y = |a: u8, b: u8| (a + lift, b + lift);
    // Base plate and the glowing core between four corner posts.
    fill(&mut v, (4, 12), y(0, 1), (4, 12), frame);
    for (px, pz) in [(4, 4), (11, 4), (4, 11), (11, 11)] {
        fill(&mut v, (px, px + 1), y(1, 7), (pz, pz + 1), frame);
    }
    fill(&mut v, (5, 11), y(1, 7), (5, 11), core);
    // Top plate, grey cap, black cap.
    fill(&mut v, (4, 12), y(7, 8), (4, 12), frame);
    fill(&mut v, (5, 11), y(8, 9), (5, 11), cap);
    fill(&mut v, (6, 10), y(9, 10), (6, 10), frame);
    // The bail: two uprights and a bar across them.
    fill(&mut v, (6, 7), y(10, 12), (7, 9), frame);
    fill(&mut v, (9, 10), y(10, 12), (7, 9), frame);
    fill(&mut v, (6, 10), y(12, 13), (7, 9), frame);
    v
}

fn model(voxels: Vec<MicroVoxel>) -> MicroModelData {
    MicroModelData {
        version: MICRO_MODEL_VERSION,
        scale: MICRO_SCALE_16,
        voxels,
        author_npub: "genesis".to_string(),
        derivation_chain: Vec::new(),
    }
}

/// The standing lantern: on the floor of its cell.
pub fn standing_model() -> MicroModelData {
    model(body(0))
}

/// The hanging lantern: the body lifted one sixteenth, on a short chain of black
/// links from the bail bar up to the top of the cell.
pub fn hanging_model() -> MicroModelData {
    let mut v = body(1);
    fill(&mut v, (7, 9), (14, 16), (7, 9), block::WALLPAPER_BLACK);
    model(v)
}

/// Register the lantern's two shells (standing under `LANTERN`, hanging under
/// [`HANGING_MODEL_KEY`]).
pub fn register_models(reg: &mut crate::micro_model_registry::MicroModelRegistry, blocks: &BlockRegistry) {
    reg.register(LANTERN, standing_model(), blocks);
    reg.register(HANGING_MODEL_KEY, hanging_model(), blocks);
}

/// How big the held lantern is: the model is a block across (at most 1.0); the
/// hand holds it about half that.
pub const HELD_SCALE: f32 = 0.55;

/// How big a dropped lantern is: the same model as the hand's, smaller. A dropped
/// non-cube block is about 0.4 across (`item_model::DROP_SPRITE_WIDTH`); this one is
/// about 0.37 tall and 0.22 wide.
pub const DROP_SCALE: f32 = 0.45;

/// The model's own middle, in the baked model's units (the cell is 1 across, the
/// body 13 sixteenths tall): held and dropped meshes are centred on it.
const MESH_CENTRE: glam::Vec3 = glam::Vec3::new(0.5, 6.5 / 16.0, 0.5);

/// The lantern in the hand: the standing model baked once, scaled by
/// [`HELD_SCALE`] and centred on its own middle, as a triangle list for the
/// entity passes (the caller stamps the light). No cut-out bit: every texel it
/// samples is opaque.
pub fn held_mesh(registry: &BlockRegistry) -> Vec<Vertex> {
    static MESH: OnceLock<Vec<Vertex>> = OnceLock::new();
    MESH.get_or_init(|| {
        let baked = bake_micro_model(&standing_model(), registry);
        baked
            .indices
            .iter()
            .map(|&i| {
                let mut v = baked.vertices[i as usize];
                v.position = ((glam::Vec3::from(v.position) - MESH_CENTRE) * HELD_SCALE).to_array();
                v
            })
            .collect()
    })
    .clone()
}

/// How far the top bar of the held model (the grip of its bail) sits above the
/// mesh's own middle: the hanging pose puts this at the fist
/// (`hold_pose::HoldPose::Hanging`). 12.5 sixteenths is the bar's middle.
pub const HANDLE_HEIGHT: f32 = (12.5 / 16.0 - MESH_CENTRE.y) * HELD_SCALE;

/// The dropped lantern's mesh: the hand's model at [`DROP_SCALE`], centred on itself
/// (`item_model::ItemLook::Lantern`; the caller spins and lifts it).
pub fn drop_mesh(registry: &BlockRegistry) -> Vec<Vertex> {
    let k = DROP_SCALE / HELD_SCALE;
    held_mesh(registry)
        .into_iter()
        .map(|mut v| {
            v.position = (glam::Vec3::from(v.position) * k).to_array();
            v
        })
        .collect()
}

/// Half the dropped lantern's height: how far its middle is above its foot.
pub const DROP_HALF_HEIGHT: f32 = MESH_CENTRE.y * DROP_SCALE;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::micro_model_registry::MicroModelRegistry;

    #[test]
    fn a_lantern_gives_15_light_and_the_others_stay_put() {
        let reg = BlockRegistry::new();
        assert_eq!(reg.light_emission(LANTERN), 15);
        assert_eq!(reg.light_emission(LANTERN), LIGHT_LEVEL);
        assert_eq!(reg.light_emission(block::TORCH), 14);
        assert_eq!(reg.light_emission(block::PAPER_LANTERN_YELLOW), 12);
    }

    #[test]
    fn clicking_the_underside_of_a_block_hangs_it_and_any_other_face_stands_it() {
        assert!(is_hanging(placement_meta([0, -1, 0])));
        for face in [[0, 1, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]] {
            assert!(!is_hanging(placement_meta(face)), "face {face:?} stands");
        }
        assert_eq!(placement_meta([0, 1, 0]), 0);
    }

    #[test]
    fn the_mesher_key_follows_the_hanging_bit() {
        assert_eq!(model_key(0), LANTERN);
        assert_eq!(model_key(placement_meta([0, -1, 0])), HANGING_MODEL_KEY);
        assert_ne!(HANGING_MODEL_KEY, LANTERN);
        assert!(!BlockRegistry::new().is_known(HANGING_MODEL_KEY), "the key is not a block id");
    }

    #[test]
    fn both_models_bake_and_the_hanging_one_reaches_the_ceiling_and_the_standing_one_the_floor() {
        let reg = BlockRegistry::new();
        for (name, data) in [("standing", standing_model()), ("hanging", hanging_model())] {
            let mesh = bake_micro_model(&data, &reg);
            assert!(!mesh.indices.is_empty(), "{name} bakes nothing");
            assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.vertices.len()));
            let ys = || mesh.vertices.iter().map(|v| v.position[1]);
            let (lo, hi) = (ys().fold(f32::MAX, f32::min), ys().fold(f32::MIN, f32::max));
            let inside = mesh.vertices.iter().all(|v| v.position.iter().all(|c| (0.0..=1.0).contains(c)));
            assert!(inside, "{name} leaves the cell");
            match name {
                "standing" => assert!(lo.abs() < 1e-4 && hi < 0.9, "stands on the floor: {lo}..{hi}"),
                _ => assert!((hi - 1.0).abs() < 1e-4 && lo > 0.05, "hangs from the ceiling: {lo}..{hi}"),
            }
        }
        assert_ne!(standing_model().content_hash(), hanging_model().content_hash());
    }

    #[test]
    fn the_look_is_painted_from_existing_layers_only() {
        // No new texture layer: every voxel is a block that already exists, and
        // each paints with a layer the stock set already holds.
        let reg = BlockRegistry::new();
        let layers = crate::texture_gen::texture_count() as u32;
        for data in [standing_model(), hanging_model()] {
            let mesh = bake_micro_model(&data, &reg);
            assert!(mesh.vertices.iter().all(|v| v.tex_layer < layers), "a baked face uses a layer past the stock set");
            for v in &data.voxels {
                assert!(reg.is_known(v.block_id));
            }
        }
        assert_eq!(crate::texture_gen::texture_count(), 512, "the lantern adds no layer");
    }

    #[test]
    fn the_core_glows_and_the_frame_is_dark() {
        let reg = BlockRegistry::new();
        let frame = block::wallpaper_solid_layer(block::WALLPAPER_BLACK).unwrap();
        let core = block::wallpaper_solid_layer(block::WALLPAPER_YELLOW).unwrap();
        let mesh = bake_micro_model(&standing_model(), &reg);
        assert!(mesh.vertices.iter().any(|v| v.tex_layer == frame), "a dark frame");
        assert!(mesh.vertices.iter().any(|v| v.tex_layer == core), "the warm core shows between the posts");
    }

    #[test]
    fn the_registry_holds_both_shells_and_the_block_is_one_the_mesher_routes_to_them() {
        let blocks = BlockRegistry::new();
        let mut reg = MicroModelRegistry::new();
        register_models(&mut reg, &blocks);
        assert!(reg.contains(LANTERN));
        assert!(reg.contains(HANGING_MODEL_KEY));
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn the_held_lantern_is_a_small_model_and_not_a_sprite() {
        let reg = BlockRegistry::new();
        let mesh = held_mesh(&reg);
        assert!(!mesh.is_empty() && mesh.len() % 3 == 0);
        assert!(mesh.iter().all(|v| v.tex_layer & crate::mesh::CUTOUT_FLAG == 0), "no cut-out bit: it is a model");
        let (mut lo, mut hi) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
        for v in &mesh {
            lo = lo.min(glam::Vec3::from(v.position));
            hi = hi.max(glam::Vec3::from(v.position));
        }
        let size = hi - lo;
        assert!(size.y > 0.3 && size.y < 0.6, "about half a block tall: {size}");
        assert!(size.x < 0.35 && size.z < 0.35, "narrow: {size}");
        assert!(((lo + hi) * 0.5).abs().max_element() < 0.05, "centred on the hand: {lo} {hi}");
    }
}

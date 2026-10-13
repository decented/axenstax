//! Actual GPU occlusion controls for opaque, cutout and translucent solids.
#![cfg(not(target_arch = "wasm32"))]

use crate::{block, camera, mesh, renderer::Renderer, world::World};

/// A green transparent-solid block in front of a blue water wall. Use a dark
/// sky and sample away from the crosshair, so colour identifies the visible
/// surface rather than fog, background or an overlay.
fn water_behind_solid(near: block::BlockId, alpha: u8, tag: &str) -> Vec<[u8; 4]> {
    let registry = block::BlockRegistry::new();
    let mut textures = crate::texture_gen::generate_textures();
    for texture in &mut textures {
        for pixel in texture.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[20, 240, 30, alpha]);
        }
    }
    for pixel in textures[registry.tex_top(block::WATER) as usize].chunks_exact_mut(4) {
        pixel.copy_from_slice(&[20, 30, 255, 255]);
    }
    let mut world = World::new();
    world.set_block(0, 0, 1, near);
    for x in -1..=1 {
        for y in -1..=1 {
            world.set_block(x, y, -1, block::WATER);
        }
    }
    let mut renderer = pollster::block_on(Renderer::new_headless(128, 128, &textures));
    renderer.sky_color = [0.001; 3];
    for (x, y, z) in world.chunk_positions() {
        renderer.upload_chunk(
            (x, y, z),
            &mesh::build_chunk_meshes(x, y, z, &world, &registry),
        );
    }
    let camera = camera::Camera::new(glam::Vec3::new(0.5, 0.5, 4.0), 1.0);
    renderer.update_camera(0, &camera::CameraUniform::from_camera(&camera));
    let path =
        std::env::temp_dir().join(format!("axenstax-depth-{}-{tag}.png", std::process::id()));
    renderer.render_to_png(path.to_str().expect("test path"));
    let image = image::open(&path).expect("rendered scene").to_rgba8();
    std::fs::remove_file(path).expect("remove owned render fixture");
    [56, 72]
        .into_iter()
        .map(|x| image.get_pixel(x, 64).0)
        .collect()
}

#[test]
#[ignore = "requires an adapter supporting the full block texture array"]
fn game_harness_opaque_leaf_texels_occlude_water_behind_them() {
    for pixel in water_behind_solid(block::OAK_LEAVES, 255, "opaque") {
        assert!(
            pixel[1] > pixel[2],
            "foreground green foliage must occlude blue water: {pixel:?}"
        );
    }
}

#[test]
#[ignore = "requires an adapter supporting the full block texture array"]
fn game_harness_leaf_cutout_holes_reveal_water_behind_them() {
    for pixel in water_behind_solid(block::OAK_LEAVES, 0, "holes") {
        assert!(
            pixel[2] > pixel[1],
            "cutout holes must reveal blue water: {pixel:?}"
        );
    }
}

#[test]
#[ignore = "requires an adapter supporting the full block texture array"]
fn game_harness_translucent_glass_does_not_occlude_water_behind_it() {
    for pixel in water_behind_solid(block::GLASS, 64, "glass") {
        assert!(
            pixel[2] > pixel[1],
            "translucent glass must retain blue water behind it: {pixel:?}"
        );
    }
}

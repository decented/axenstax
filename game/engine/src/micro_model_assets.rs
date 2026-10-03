//! Engine built-in micro-model assets (owner-inbox #18, Phase C) — the first
//! real consumers of the bake-to-micro-model pipeline: the three wild dye
//! flowers (CORNFLOWER / FIELD_POPPY / BUTTERCUP) upgrade from flat cross-
//! billboards ("spaces in the centre") to 3D sub-voxel shells.
//!
//! **Why procedural builders, not committed JSON:** the intended authoring UX
//! is in-engine — build a giant flower out of blocks in creative, capture it
//! with the schematic system, bake it (`docs/foundations/2026-06-03-build-big-
//! micro-models.md`). That flow needs an interactive GPU session, so the *first*
//! built-ins are authored here as readable, parameterised Rust (a green stem +
//! a flattened colour disc + a contrasting centre) rather than a hand-written
//! ~140-voxel JSON blob. The JSON loader + registry (Phase A/B) stay the path
//! for community / Stash assets and the future in-engine capture output.
//!
//! **Paint-with-blocks:** each micro-voxel's colour is the source block's
//! texture. The near-solid `WALLPAPER_*` colours are the palette
//! (`texture_gen::gen_wallpaper`): cornflower blue, poppy red + black centre,
//! buttercup yellow, green stems — chosen to match each flower's `BlockDef.color`.

use crate::block::{self, BlockId, BlockRegistry};
use crate::micro_model::{MicroModelData, MicroVoxel, MICRO_MODEL_VERSION, MICRO_SCALE_16};
use crate::micro_model_registry::MicroModelRegistry;

/// Build a generic flower micro-model at 1/16 scale: a 2×2 green stem column up
/// the centre, a flattened ellipsoid bloom of `petal` blocks atop it, and a
/// small contrasting `center` core. Footprint (~0.44 wide, ~0.85 tall) sits
/// inside the host block's unit cube, comparable to the old billboard AABB but
/// with real 3D volume.
fn flower_model(stem: BlockId, petal: BlockId, center: BlockId) -> MicroModelData {
    let mut voxels = Vec::new();

    // Stem: a 2×2 column up the centre, y 0..10.
    for y in 0..10u8 {
        for x in 7..9u8 {
            for z in 7..9u8 {
                voxels.push(MicroVoxel { mx: x, my: y, mz: z, block_id: stem });
            }
        }
    }

    // Bloom: a flattened disc (ellipsoid rx=rz=3.5, ry=2) centred above the stem.
    let (cx, cy, cz) = (8.0f32, 11.0, 8.0);
    let (rx, ry, rz) = (3.5f32, 2.0, 3.5);
    for y in 9..14u8 {
        for x in 0..16u8 {
            for z in 0..16u8 {
                let dx = (x as f32 + 0.5 - cx) / rx;
                let dy = (y as f32 + 0.5 - cy) / ry;
                let dz = (z as f32 + 0.5 - cz) / rz;
                if dx * dx + dy * dy + dz * dz <= 1.0 {
                    voxels.push(MicroVoxel { mx: x, my: y, mz: z, block_id: petal });
                }
            }
        }
    }

    // Centre eye: a small contrasting core in the bloom centre that POKES one cell
    // ABOVE the bloom top (the ellipsoid tops out at y=12) so its cap is exposed to
    // AIR and actually bakes. A fully-surrounded centre is culled to zero faces —
    // the audit-found bug (2026-06-03), guarded by `flower_centre_is_visible`.
    // Pushed last so it wins the shared centre cells (last-write).
    for y in 11..14u8 {
        for x in 7..9u8 {
            for z in 7..9u8 {
                voxels.push(MicroVoxel { mx: x, my: y, mz: z, block_id: center });
            }
        }
    }

    MicroModelData {
        version: MICRO_MODEL_VERSION,
        scale: MICRO_SCALE_16,
        voxels,
        author_npub: "genesis".to_string(),
        derivation_chain: Vec::new(),
    }
}

/// Cornflower — blue bloom, purple centre, green stem.
pub fn cornflower_model() -> MicroModelData {
    flower_model(block::WALLPAPER_GREEN, block::WALLPAPER_BLUE, block::WALLPAPER_PURPLE)
}

/// Field poppy — red bloom, black centre (poppies' signature dark eye), green stem.
pub fn field_poppy_model() -> MicroModelData {
    flower_model(block::WALLPAPER_GREEN, block::WALLPAPER_RED, block::WALLPAPER_BLACK)
}

/// Buttercup — yellow bloom, warm orange centre, green stem.
pub fn buttercup_model() -> MicroModelData {
    flower_model(block::WALLPAPER_GREEN, block::WALLPAPER_YELLOW, block::WALLPAPER_ORANGE)
}

/// Register the engine's built-in micro-model overrides. Called from
/// `World::load_bundled_micro_models` at client world init. Each flower's block
/// id (unchanged — render-only override) now renders its baked 3D shell.
pub fn register_builtin_micro_models(reg: &mut MicroModelRegistry, blocks: &BlockRegistry) {
    reg.register(block::CORNFLOWER, cornflower_model(), blocks);
    reg.register(block::FIELD_POPPY, field_poppy_model(), blocks);
    reg.register(block::BUTTERCUP, buttercup_model(), blocks);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::micro_model::bake_micro_model;

    #[test]
    fn each_flower_bakes_to_a_nonempty_culled_shell() {
        let blocks = BlockRegistry::new();
        for model in [cornflower_model(), field_poppy_model(), buttercup_model()] {
            assert_eq!(model.scale, 16);
            assert!(model.voxels.len() > 40, "flower should have stem + bloom + centre");
            let mesh = bake_micro_model(&model, &blocks);
            assert!(!mesh.indices.is_empty(), "baked flower mesh is empty");
            // Interior culled: far fewer tris than naive cubes (voxels × 36 indices).
            assert!(
                mesh.indices.len() < model.voxels.len() * 36,
                "interior faces not culled"
            );
            // Valid index buffer.
            let n = mesh.vertices.len() as u32;
            assert!(mesh.indices.iter().all(|&i| i < n));
            assert_eq!(mesh.indices.len() % 3, 0);
        }
    }

    #[test]
    fn flower_centre_is_visible() {
        // Regression guard (audit 2026-06-03): the contrasting centre must bake to
        // real geometry — if it's fully surrounded by petals the greedy mesher culls
        // every face and the eye renders zero triangles (the bug this guards).
        let blocks = BlockRegistry::new();
        for (model, centre) in [
            (cornflower_model(), block::WALLPAPER_PURPLE),
            (field_poppy_model(), block::WALLPAPER_BLACK),
            (buttercup_model(), block::WALLPAPER_ORANGE),
        ] {
            let mesh = bake_micro_model(&model, &blocks);
            // The centre is a wallpaper colour block; the bake renders wallpaper
            // as its flat SOLID-colour paint layer (Workshop-paint fix), so the
            // visible-centre guard checks that layer, not the patterned tex_side.
            let centre_tex = crate::block::wallpaper_solid_layer(centre)
                .unwrap_or_else(|| blocks.tex_side(centre));
            assert!(
                mesh.vertices.iter().any(|v| v.tex_layer == centre_tex),
                "flower centre (tex layer {centre_tex}) baked ZERO faces — buried inside the bloom"
            );
        }
    }

    #[test]
    fn register_builtins_adds_the_three_flowers() {
        let blocks = BlockRegistry::new();
        let mut reg = MicroModelRegistry::new();
        register_builtin_micro_models(&mut reg, &blocks);
        assert_eq!(reg.len(), 3);
        assert!(reg.contains(block::CORNFLOWER));
        assert!(reg.contains(block::FIELD_POPPY));
        assert!(reg.contains(block::BUTTERCUP));
    }

    #[test]
    fn flowers_are_visually_distinct() {
        // Different petal/centre colours → different baked content (so the
        // Phase-B per-type cache keeps three distinct shells, not one shared).
        let c = cornflower_model().content_hash();
        let p = field_poppy_model().content_hash();
        let b = buttercup_model().content_hash();
        assert_ne!(c, p);
        assert_ne!(p, b);
        assert_ne!(c, b);
    }
}

//! The `block_id → micro-model` override table (owner-inbox #18, Phase B).
//!
//! A registered block renders its baked micro-model shell instead of its default
//! cube / cross-billboard. This is the small plug-in point that lets flowers
//! (Phase C) — and later any décor block or community asset — upgrade their look
//! with zero change to saves, inventory, crafting or crop-growth: only the render
//! path flips, keyed by an unchanged `BlockId`.
//!
//! Mirrors the role of `PlanRegistry`, but keyed by `BlockId` in an `AHashMap`
//! (O(1) lookup in the hot mesh-build path) rather than `PlanRegistry`'s flat
//! `Vec` (linear scan is fine for procgen sampling but not per-voxel meshing).
//! Lives as a `World` field (`world.micro_registry`), like `plan_registry`, so
//! the mesher can consult it via the `&World` it already receives, and so
//! community assets (Stash, deferred) can populate a non-static registry later.
//! The baked **CPU** mesh lives here; the **GPU** buffers live in the renderer
//! (`Renderer::sync_micro_models` uploads one shared geometry per type), keeping
//! this struct platform-agnostic.

use crate::block::{BlockId, BlockRegistry};
use crate::mesh::ChunkMesh;
use crate::micro_model::{bake_micro_model, MicroModelData};
use ahash::AHashMap;

/// A registered micro-model: the source data + its baked, interior-culled shell
/// mesh (CPU). One per registered `BlockId`.
#[derive(Clone, Debug)]
pub struct BakedMicroModel {
    #[allow(dead_code)] // carries the source model for the deferred Stash/re-bake layer; read only by tests today
    pub data: MicroModelData,
    pub mesh: ChunkMesh,
}

/// `block_id → baked micro-model` override table.
#[derive(Clone, Debug, Default)]
pub struct MicroModelRegistry {
    models: AHashMap<BlockId, BakedMicroModel>,
}

impl MicroModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bake `data` (once) and register it as the render override for `block_id`.
    /// Re-registering a `block_id` replaces the previous entry. Solid AND non-solid
    /// blocks render their shell: the chunk mesher skips a registered block in the
    /// greedy pass and emits a micro-instance for it (Workshop Phase 4 / "Phase G").
    pub fn register(&mut self, block_id: BlockId, data: MicroModelData, registry: &BlockRegistry) {
        let mesh = bake_micro_model(&data, registry);
        self.models.insert(block_id, BakedMicroModel { data, mesh });
    }

    /// The baked override for `block_id`, if any.
    pub fn get(&self, block_id: BlockId) -> Option<&BakedMicroModel> {
        self.models.get(&block_id)
    }

    /// Whether `block_id` has a registered micro-model (the hot-path predicate
    /// the mesher uses to decide routing).
    pub fn contains(&self, block_id: BlockId) -> bool {
        self.models.contains_key(&block_id)
    }

    #[allow(dead_code)] // rounds out the type (clippy len_without_is_empty); tested only
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    #[allow(dead_code)] // rounds out the type; tested only
    pub fn len(&self) -> usize {
        self.models.len()
    }

    /// Iterate `(block_id, baked)` — used by the renderer to upload one shared
    /// GPU geometry per registered type.
    pub fn iter(&self) -> impl Iterator<Item = (&BlockId, &BakedMicroModel)> {
        self.models.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::STONE;
    use crate::micro_model::{MicroModelData, MicroVoxel, MICRO_MODEL_VERSION};

    fn tiny_model() -> MicroModelData {
        MicroModelData {
            version: MICRO_MODEL_VERSION,
            scale: 8,
            voxels: vec![MicroVoxel { mx: 0, my: 0, mz: 0, block_id: STONE }],
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        }
    }

    #[test]
    fn new_registry_is_empty() {
        let r = MicroModelRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(!r.contains(131));
        assert!(r.get(131).is_none());
    }

    #[test]
    fn register_bakes_and_stores() {
        let reg = BlockRegistry::new();
        let mut r = MicroModelRegistry::new();
        r.register(131, tiny_model(), &reg);
        assert!(r.contains(131));
        assert_eq!(r.len(), 1);
        let baked = r.get(131).expect("registered");
        assert_eq!(baked.data.scale, 8);
        // single voxel → a cube shell: 24 verts / 36 indices
        assert_eq!(baked.mesh.vertices.len(), 24);
        assert_eq!(baked.mesh.indices.len(), 36);
    }

    #[test]
    fn unregistered_block_absent() {
        let reg = BlockRegistry::new();
        let mut r = MicroModelRegistry::new();
        r.register(131, tiny_model(), &reg);
        assert!(!r.contains(132));
        assert!(r.get(132).is_none());
    }

    #[test]
    fn reregister_replaces() {
        let reg = BlockRegistry::new();
        let mut r = MicroModelRegistry::new();
        r.register(131, tiny_model(), &reg);
        let mut two = tiny_model();
        two.voxels.push(MicroVoxel { mx: 1, my: 0, mz: 0, block_id: STONE });
        r.register(131, two, &reg);
        assert_eq!(r.len(), 1, "re-register should replace, not add");
        assert_eq!(r.get(131).unwrap().data.voxels.len(), 2);
    }
}

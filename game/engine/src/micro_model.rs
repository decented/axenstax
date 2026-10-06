//! Sub-voxel static-prop micro-models (owner-inbox #18, foundation
//! `docs/foundations/2026-06-03-build-big-micro-models.md`).
//!
//! A creator builds a giant prop out of ordinary blocks in creative, captures
//! it with the build-schematics system (`plan.rs`), and the engine **bakes**
//! that captured volume down into a compact sub-voxel shell mesh that renders
//! in place of a block's default cube / cross-billboard. The game itself is the
//! asset editor; the community authors the assets, the engine ships the tool.
//!
//! This module is **Phase A**: the pure, GPU-free data type + bake step. It is
//! **WIP / not user-reachable yet** — `#![allow(dead_code)]`, no renderer wiring
//! (Phase B) and no authoring trigger, so nothing calls it at runtime (engine
//! audit 2026-06-04, E7). It's the tested substrate, not a live feature.
//!   * [`MicroModelData`] — a baked sub-voxel prop (occupied micro-voxels only),
//!     wire-stable / serde so it can travel through Stash + a built-in registry.
//!   * [`MicroModelData::from_plan`] — reinterpret a captured `PlanData` at a
//!     finer (1/8 or 1/16) grid. Exact-build for v1 (open question 1 in the
//!     foundation doc): the captured volume must fit inside `scale³`.
//!   * [`bake_micro_model`] — mesh a `MicroModelData` into a greedy-merged,
//!     interior-culled shell [`ChunkMesh`], mirroring the chunk mesher's proven
//!     two-phase merge (`mesh.rs::greedy_face`). This lives here rather than in
//!     `mesh.rs` because that file is already ~1300 lines (CLAUDE.md no-god-files
//!     rule); it reuses the mesher's machinery by importing [`Vertex`]/[`ChunkMesh`]
//!     and replicating the merge algorithm over a `scale³` grid.
//!
//! **The one failure mode** the whole design avoids: drawing raw `scale³` cubes
//! per placement (a 1/16 flower is 4096 cells; a field of them is billions of
//! tris). Baking culls the interior and merges the shell ONCE per asset type;
//! the render path (Phase B) only ever *instances* the pre-baked mesh.

// BRIDGE: Phase A ships the pure data + bake layer; most of the surface is live
// (the registry, mesher and renderer consume it). The few items Phase B/C will
// call carry item-level `allow(dead_code)` — delete each as it gets a caller.

use crate::block::{BlockId, BlockRegistry, AIR};
use crate::mesh::{ChunkMesh, Vertex};
use crate::plan::{DerivationLink, PlanData};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Coarser micro-grid: 8×8×8 (512 cells max). Cheaper, for chunky props.
pub const MICRO_SCALE_8: u8 = 8;
/// Fine micro-grid: 16×16×16 (4096 cells max). Lines up 1:1 with the 16×16
/// texture resolution (`texture_gen.rs`), so one micro-voxel ↔ one source texel.
pub const MICRO_SCALE_16: u8 = 16;

/// Current `MicroModelData` wire version. Bump + migrate like `PlanData`.
pub const MICRO_MODEL_VERSION: u8 = 1;

/// One occupied micro-voxel: a cell coordinate in the `scale³` grid + the block
/// it was built from (its texture is the "paint" — see `bake_micro_model`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MicroVoxel {
    pub mx: u8,
    pub my: u8,
    pub mz: u8,
    pub block_id: BlockId,
}

/// A baked sub-voxel static prop. A micro-voxel at `(mx,my,mz)` sits at world
/// offset `(mx,my,mz)/scale` inside the host block's unit cube. Only occupied
/// micro-voxels are stored. Wire-stable / serde — this is shareable content
/// (Stash, registry), so it is versioned like `PlanData`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MicroModelData {
    pub version: u8,
    /// Micro-grid resolution: [`MICRO_SCALE_8`] or [`MICRO_SCALE_16`].
    pub scale: u8,
    /// Occupied cells only (interior + shell; the bake culls interior faces).
    pub voxels: Vec<MicroVoxel>,
    /// Provenance for the deferred do-ocracy attribution layer (mirrors
    /// `PlanData`). Carried now so the governance doc is a drop-in, not a rewrite.
    pub author_npub: String,
    pub derivation_chain: Vec<DerivationLink>,
}

/// Why a bake / `from_plan` was refused. Typed so callers can show a clear
/// in-engine message (authoring ergonomics — foundation doc risk section).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeRefusal {
    /// `scale` was neither 8 nor 16.
    BadScale,
    /// The source had no cells / no occupied voxels.
    EmptyVolume,
    /// The captured envelope exceeds `scale` in some dimension (exact-build, v1).
    TooLarge,
    /// A voxel coordinate landed outside the `scale³` grid (defensive).
    OutOfBounds,
}

impl MicroModelData {
    /// Reinterpret a captured [`PlanData`] as a micro-model at `scale` (8 or 16).
    ///
    /// **Exact-build (v1, foundation doc open-question 1):** the captured volume
    /// must already fit inside `scale³` — `width/depth/height ≤ scale`. Each
    /// captured cell maps 1:1 to a micro-voxel (`rx→mx`, `ry→my`, `rz→mz`).
    /// Downsample-by-majority for arbitrary captures is deferred.
    ///
    /// Provenance (`author_npub`, `derivation_chain`) is inherited from the plan
    /// so attribution survives the bake.
    pub fn from_plan(plan: &PlanData, scale: u8) -> Result<MicroModelData, BakeRefusal> {
        if scale != MICRO_SCALE_8 && scale != MICRO_SCALE_16 {
            return Err(BakeRefusal::BadScale);
        }
        if plan.cells.is_empty() {
            return Err(BakeRefusal::EmptyVolume);
        }
        // Exact-build (v1): the captured envelope must fit inside `scale³`.
        // `width↔rx (x)`, `height↔ry (y)`, `depth↔rz (z)` (the capture axis
        // convention, see `plan::debug_3x3_stone`).
        if plan.width > scale || plan.depth > scale || plan.height > scale {
            return Err(BakeRefusal::TooLarge);
        }
        let mut voxels = Vec::with_capacity(plan.cells.len());
        for c in &plan.cells {
            if c.rx >= scale || c.ry >= scale || c.rz >= scale {
                return Err(BakeRefusal::OutOfBounds);
            }
            voxels.push(MicroVoxel { mx: c.rx, my: c.ry, mz: c.rz, block_id: c.block_id });
        }
        Ok(MicroModelData {
            version: MICRO_MODEL_VERSION,
            scale,
            voxels,
            author_npub: plan.author_npub.clone(),
            derivation_chain: plan.derivation_chain.clone(),
        })
    }

    /// Content hash over the *identity* of the model (version + scale + voxels),
    /// excluding provenance — mirrors `plan::content_hash`'s
    /// canonicalise-then-hash approach. **Voxels are sorted before hashing** so
    /// the hash is stable across reorderings. NOTE: provided for a FUTURE
    /// content-hash dedup / Stash cache — the current `MicroModelRegistry` keys by
    /// `block_id` and bakes per block, so nothing keys on this hash in production yet.
    #[allow(dead_code)] // provided for a future content-hash dedup / Stash cache; tested only
    pub fn content_hash(&self) -> [u8; 32] {
        // Canonicalise to the SAME grid `bake_micro_model` builds: collapse
        // duplicate-coordinate voxels (last-write-wins, matching the bake's grid
        // write order), then hash the sorted (coord, block_id) entries plus
        // version + scale. This makes hash-identity == bake-identity — two models
        // that bake to the same mesh hash the same, regardless of voxel order OR
        // duplicates — the property a future Stash/dedup cache would rely on (no
        // such cache exists yet; see the fn doc-comment).
        // Provenance is excluded (mirrors `plan::content_hash`). Hashed explicitly
        // (not via bincode) so the byte layout is obvious + stable across toolchains.
        let mut grid: std::collections::BTreeMap<(u8, u8, u8), BlockId> =
            std::collections::BTreeMap::new();
        for v in &self.voxels {
            grid.insert((v.mx, v.my, v.mz), v.block_id);
        }
        let mut hasher = Sha256::new();
        hasher.update([self.version, self.scale]);
        for ((mx, my, mz), block_id) in &grid {
            hasher.update([*mx, *my, *mz]);
            hasher.update(block_id.to_le_bytes());
        }
        hasher.finalize().into()
    }
}

/// Parse a built-in micro-model asset (`assets/micro_models/*.json`). Mirrors
/// the `plan_registry::BundledPlan` wrapper convention: the on-disk shape is
/// `{ "micro_model": { <MicroModelData fields> } }`, not a bare struct.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)] // Phase B/C micro-model render path is not wired; tested only (see the BRIDGE note)
pub struct BundledMicroModel {
    pub micro_model: MicroModelData,
}

/// Parse one `assets/micro_models/*.json` source string into [`MicroModelData`].
#[allow(dead_code)] // Phase B/C micro-model render path is not wired; tested only (see the BRIDGE note)
pub fn parse_micro_model_json(source: &str) -> Result<MicroModelData, serde_json::Error> {
    Ok(serde_json::from_str::<BundledMicroModel>(source)?.micro_model)
}

/// Engine-bundled micro-model assets, embedded at compile time via `include_str!`
/// — mirrors `plan_registry::bundled_plan_sources`. Adding an asset is a two-step
/// append: drop the file in `assets/micro_models/` and add a
/// `(name, include_str!("../assets/micro_models/<file>.json"))` row here. Kept an
/// explicit hand-maintained list (not a build-script scan) so the bundle surface
/// is auditable and the binary stays self-contained on WASM. `example_cube` is a
/// format reference + loader canary; the real flower assets land in Phase C.
#[allow(dead_code)] // Phase B/C micro-model render path is not wired; tested only (see the BRIDGE note)
fn bundled_micro_model_sources() -> &'static [(&'static str, &'static str)] {
    &[(
        "example_cube",
        include_str!("../assets/micro_models/example_cube.json"),
    )]
}

/// Parse every engine-bundled micro-model. Parse failures are non-fatal
/// (warn-and-skip), mirroring `plan_registry::load_bundled`. The Phase-B
/// `MicroModelRegistry` builds its `block_id → baked mesh` table on top of this.
#[allow(dead_code)] // only the Phase B registry loader will call it (tests exercise it today)
pub fn load_bundled_micro_models() -> Vec<MicroModelData> {
    let mut out = Vec::new();
    for (name, source) in bundled_micro_model_sources() {
        match parse_micro_model_json(source) {
            Ok(m) => out.push(m),
            Err(e) => log::warn!("micro_model: failed to parse bundled '{name}': {e}"),
        }
    }
    out
}

/// Bake a [`MicroModelData`] into a compact, interior-culled, greedy-merged
/// shell [`ChunkMesh`]. Pure: no GPU, no `World`. Baked ONCE per asset type
/// (the render path caches by content hash and only instances the result).
///
/// The micro-grid is a `scale³` occupancy grid — structurally a denser sub-chunk
/// — so this runs the same 6-direction greedy face-merge that the chunk mesher
/// runs (`mesh.rs::greedy_face`): a face is emitted only when the neighbouring
/// micro-voxel is empty, so a solid interior contributes **zero** triangles.
/// Each shell face's texture layer comes from the source block it was built
/// from (`tex_top/bottom/side`) — the "paint-with-blocks" mechanic. Per-vertex
/// `light` is baked at `FULL_BRIGHT`; placement light is applied per-instance at
/// render time (Phase B), exactly as plants do.
pub fn bake_micro_model(model: &MicroModelData, registry: &BlockRegistry) -> ChunkMesh {
    let scale = model.scale as usize;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    // Guard the grid allocation: only 8 or 16 are valid micro scales. Anything
    // larger (e.g. a corrupt/hostile Stash asset with scale 255 → 16M-cell grid)
    // is refused here so the loader can't DoS the bake. Smaller-but-odd scales
    // also bail rather than bake a non-spec shell.
    if (scale != MICRO_SCALE_8 as usize && scale != MICRO_SCALE_16 as usize)
        || model.voxels.is_empty()
    {
        return ChunkMesh { vertices, indices };
    }
    // Dense occupancy grid (AIR = empty). Out-of-range voxels are dropped
    // defensively (from_plan already guarantees in-range, but a hand-authored
    // asset might not).
    let mut grid = vec![AIR; scale * scale * scale];
    for v in &model.voxels {
        let (x, y, z) = (v.mx as usize, v.my as usize, v.mz as usize);
        if x < scale && y < scale && z < scale {
            grid[grid_idx(x, y, z, scale)] = v.block_id;
        }
    }
    for &face in &MICRO_FACES {
        greedy_micro_face(&grid, scale, face, registry, &mut vertices, &mut indices);
    }
    ChunkMesh { vertices, indices }
}

/// Greedy-merge one face direction across the whole `scale³` micro-grid — the
/// same two-phase merge `mesh::greedy_face` runs on a chunk, but parameterised
/// on `scale` and with no cross-chunk/world light lookups. A face is emitted
/// only when the neighbouring micro-voxel is empty (or off-grid), so a solid
/// interior contributes zero triangles.
fn greedy_micro_face(
    grid: &[BlockId],
    scale: usize,
    face: MicroFace,
    registry: &BlockRegistry,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let (ndx, ndy, ndz) = face.offset();
    let normal = face.normal();
    for layer in 0..scale {
        // mask[v*scale + u] = Some(block) when the cell is occupied AND its
        // neighbour in the face direction is empty (exposed shell face).
        let mut mask: Vec<Option<BlockId>> = vec![None; scale * scale];
        for v in 0..scale {
            for u in 0..scale {
                let (x, y, z) = face.local_to_xyz(layer, u, v);
                let block = grid[grid_idx(x, y, z, scale)];
                if block == AIR {
                    continue;
                }
                let nx = x as i32 + ndx;
                let ny = y as i32 + ndy;
                let nz = z as i32 + ndz;
                let exposed = nx < 0
                    || ny < 0
                    || nz < 0
                    || nx >= scale as i32
                    || ny >= scale as i32
                    || nz >= scale as i32
                    || grid[grid_idx(nx as usize, ny as usize, nz as usize, scale)] == AIR;
                if exposed {
                    mask[v * scale + u] = Some(block);
                }
            }
        }

        // Two-phase greedy merge: extend width along u, then height along v
        // requiring the whole w-strip to match (mirrors mesh::greedy_face).
        let mut visited = vec![false; scale * scale];
        for v in 0..scale {
            for u in 0..scale {
                let cell = v * scale + u;
                if visited[cell] || mask[cell].is_none() {
                    continue;
                }
                let entry = mask[cell];
                let block = entry.unwrap();

                let mut w = 1;
                while u + w < scale && !visited[v * scale + u + w] && mask[v * scale + u + w] == entry {
                    w += 1;
                }

                let mut h = 1;
                'outer: while v + h < scale {
                    for du in 0..w {
                        if visited[(v + h) * scale + u + du] || mask[(v + h) * scale + u + du] != entry {
                            break 'outer;
                        }
                    }
                    h += 1;
                }

                for dv in 0..h {
                    for du in 0..w {
                        visited[(v + dv) * scale + u + du] = true;
                    }
                }

                // P-bugfix (Workshop paint): a dye-painted cell stores a wallpaper
                // block whose texture is a decorative PATTERN. For paint we want a
                // flat SOLID colour, so map it to its solid-colour layer; every
                // other block keeps its normal face texture.
                let tex_layer = crate::block::wallpaper_solid_layer(block)
                    .unwrap_or_else(|| face.tex_layer(block, registry));
                emit_micro_quad(face, layer, u, v, w, h, scale, tex_layer, normal, vertices, indices);
            }
        }
    }
}

/// Emit one merged shell quad in model space (`0..1` inside the host block).
/// Winding, normals and UV tiling mirror `mesh::emit_small_cube`; UVs tile by
/// cell count (`w × h`) so each micro-cell shows one full source-block texture
/// — the paint-with-blocks look.
#[allow(clippy::too_many_arguments)]
fn emit_micro_quad(
    face: MicroFace,
    layer: usize,
    u: usize,
    v: usize,
    w: usize,
    h: usize,
    scale: usize,
    tex_layer: u32,
    normal: [f32; 3],
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let inv = 1.0 / scale as f32;
    let wf = w as f32;
    let hf = h as f32;
    let (corners, uvs): ([[f32; 3]; 4], [[f32; 2]; 4]) = match face {
        MicroFace::Top => {
            let (x0, x1) = (u as f32 * inv, (u + w) as f32 * inv);
            let (z0, z1) = (v as f32 * inv, (v + h) as f32 * inv);
            let y = (layer + 1) as f32 * inv;
            (
                [[x0, y, z0], [x0, y, z1], [x1, y, z1], [x1, y, z0]],
                [[0.0, 0.0], [0.0, hf], [wf, hf], [wf, 0.0]],
            )
        }
        MicroFace::Bottom => {
            let (x0, x1) = (u as f32 * inv, (u + w) as f32 * inv);
            let (z0, z1) = (v as f32 * inv, (v + h) as f32 * inv);
            let y = layer as f32 * inv;
            (
                [[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]],
                [[0.0, 0.0], [wf, 0.0], [wf, hf], [0.0, hf]],
            )
        }
        MicroFace::North => {
            let (x0, x1) = (u as f32 * inv, (u + w) as f32 * inv);
            let (y0, y1) = (v as f32 * inv, (v + h) as f32 * inv);
            let z = layer as f32 * inv;
            (
                [[x1, y0, z], [x0, y0, z], [x0, y1, z], [x1, y1, z]],
                [[wf, hf], [0.0, hf], [0.0, 0.0], [wf, 0.0]],
            )
        }
        MicroFace::South => {
            let (x0, x1) = (u as f32 * inv, (u + w) as f32 * inv);
            let (y0, y1) = (v as f32 * inv, (v + h) as f32 * inv);
            let z = (layer + 1) as f32 * inv;
            (
                [[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]],
                [[0.0, hf], [wf, hf], [wf, 0.0], [0.0, 0.0]],
            )
        }
        MicroFace::East => {
            // local_to_xyz East → (layer, v, u): u sweeps z, v sweeps y.
            let (z0, z1) = (u as f32 * inv, (u + w) as f32 * inv);
            let (y0, y1) = (v as f32 * inv, (v + h) as f32 * inv);
            let x = (layer + 1) as f32 * inv;
            (
                [[x, y0, z1], [x, y0, z0], [x, y1, z0], [x, y1, z1]],
                [[wf, hf], [0.0, hf], [0.0, 0.0], [wf, 0.0]],
            )
        }
        MicroFace::West => {
            let (z0, z1) = (u as f32 * inv, (u + w) as f32 * inv);
            let (y0, y1) = (v as f32 * inv, (v + h) as f32 * inv);
            let x = layer as f32 * inv;
            (
                [[x, y0, z0], [x, y0, z1], [x, y1, z1], [x, y1, z0]],
                [[0.0, hf], [wf, hf], [wf, 0.0], [0.0, 0.0]],
            )
        }
    };
    let base = vertices.len() as u32;
    for i in 0..4 {
        vertices.push(Vertex {
            position: corners[i],
            normal,
            tex_layer,
            uv: uvs[i],
            light: Vertex::FULL_BRIGHT,
            sky_light: 0.0,
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

// ---------------------------------------------------------------------------
// Internal: the micro-grid greedy mesher (a denser cousin of mesh.rs::greedy_face)
// ---------------------------------------------------------------------------

#[inline]
fn grid_idx(x: usize, y: usize, z: usize, scale: usize) -> usize {
    // Same flat layout as `chunk::index`: x + z*scale + y*scale².
    x + z * scale + y * scale * scale
}

/// The six axis-aligned face directions, mirroring `mesh::Face` but local to
/// this module (keeps the micro mesher self-contained — `mesh::Face` need not
/// be public). Winding/normals/UV-orientation match `emit_small_cube`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MicroFace {
    Top,
    Bottom,
    North,
    South,
    East,
    West,
}

const MICRO_FACES: [MicroFace; 6] = [
    MicroFace::Top,
    MicroFace::Bottom,
    MicroFace::North,
    MicroFace::South,
    MicroFace::East,
    MicroFace::West,
];

impl MicroFace {
    /// Outward normal / neighbour-offset direction.
    fn offset(self) -> (i32, i32, i32) {
        match self {
            MicroFace::Top => (0, 1, 0),
            MicroFace::Bottom => (0, -1, 0),
            MicroFace::North => (0, 0, -1),
            MicroFace::South => (0, 0, 1),
            MicroFace::East => (1, 0, 0),
            MicroFace::West => (-1, 0, 0),
        }
    }

    fn normal(self) -> [f32; 3] {
        let (x, y, z) = self.offset();
        [x as f32, y as f32, z as f32]
    }

    /// Map slab coords `(layer, u, v)` to grid `(x, y, z)` — matches
    /// `mesh::face_local_to_xyz`.
    fn local_to_xyz(self, layer: usize, u: usize, v: usize) -> (usize, usize, usize) {
        match self {
            MicroFace::Top | MicroFace::Bottom => (u, layer, v),
            MicroFace::North | MicroFace::South => (u, v, layer),
            MicroFace::East | MicroFace::West => (layer, v, u),
        }
    }

    fn tex_layer(self, block: BlockId, registry: &BlockRegistry) -> u32 {
        match self {
            MicroFace::Top => registry.tex_top(block),
            MicroFace::Bottom => registry.tex_bottom(block),
            _ => registry.tex_side(block),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::STONE;

    fn solid_model(scale: u8, block: BlockId) -> MicroModelData {
        let s = scale;
        let mut voxels = Vec::new();
        for y in 0..s {
            for z in 0..s {
                for x in 0..s {
                    voxels.push(MicroVoxel { mx: x, my: y, mz: z, block_id: block });
                }
            }
        }
        MicroModelData {
            version: MICRO_MODEL_VERSION,
            scale,
            voxels,
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        }
    }

    fn one_voxel(scale: u8, x: u8, y: u8, z: u8) -> MicroModelData {
        MicroModelData {
            version: MICRO_MODEL_VERSION,
            scale,
            voxels: vec![MicroVoxel { mx: x, my: y, mz: z, block_id: STONE }],
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        }
    }

    // ---- from_plan ----

    #[test]
    fn from_plan_maps_debug_plan_at_scale_8() {
        let plan = PlanData::debug_3x3_stone();
        let m = MicroModelData::from_plan(&plan, MICRO_SCALE_8).unwrap();
        assert_eq!(m.scale, 8);
        assert_eq!(m.voxels.len(), 9); // 3×3 footprint, height 1
        // every captured cell (rx,0,rz) for rx,rz in 0..3, all STONE
        for vx in &m.voxels {
            assert_eq!(vx.my, 0);
            assert!(vx.mx < 3 && vx.mz < 3);
            assert_eq!(vx.block_id, STONE);
        }
    }

    #[test]
    fn from_plan_maps_debug_plan_at_scale_16() {
        let plan = PlanData::debug_3x3_stone();
        let m = MicroModelData::from_plan(&plan, MICRO_SCALE_16).unwrap();
        assert_eq!(m.scale, 16);
        assert_eq!(m.voxels.len(), 9);
    }

    #[test]
    fn from_plan_rejects_bad_scale() {
        let plan = PlanData::debug_3x3_stone();
        assert_eq!(MicroModelData::from_plan(&plan, 7).unwrap_err(), BakeRefusal::BadScale);
        assert_eq!(MicroModelData::from_plan(&plan, 32).unwrap_err(), BakeRefusal::BadScale);
        assert_eq!(MicroModelData::from_plan(&plan, 0).unwrap_err(), BakeRefusal::BadScale);
    }

    #[test]
    fn from_plan_rejects_empty() {
        let mut plan = PlanData::debug_3x3_stone();
        plan.cells.clear();
        assert_eq!(
            MicroModelData::from_plan(&plan, MICRO_SCALE_16).unwrap_err(),
            BakeRefusal::EmptyVolume
        );
    }

    #[test]
    fn from_plan_rejects_too_large() {
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 20; // exceeds scale 16
        assert_eq!(
            MicroModelData::from_plan(&plan, MICRO_SCALE_16).unwrap_err(),
            BakeRefusal::TooLarge
        );
    }

    #[test]
    fn from_plan_inherits_provenance() {
        let mut plan = PlanData::debug_3x3_stone();
        plan.author_npub = "npub1example".to_string();
        let chain_len = plan.derivation_chain.len();
        let m = MicroModelData::from_plan(&plan, MICRO_SCALE_16).unwrap();
        assert_eq!(m.author_npub, "npub1example");
        assert_eq!(m.derivation_chain.len(), chain_len);
    }

    // ---- content_hash ----

    #[test]
    fn content_hash_stable_across_voxel_reorder() {
        let a = solid_model(4, STONE);
        let mut b = a.clone();
        b.voxels.reverse();
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_differs_on_different_voxels() {
        let a = one_voxel(8, 0, 0, 0);
        let b = one_voxel(8, 1, 0, 0);
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_differs_on_scale() {
        let a = one_voxel(8, 0, 0, 0);
        let b = one_voxel(16, 0, 0, 0);
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_ignores_provenance() {
        let a = one_voxel(8, 0, 0, 0);
        let mut b = a.clone();
        b.author_npub = "npub1someoneelse".to_string();
        assert_eq!(a.content_hash(), b.content_hash());
    }

    // ---- bake_micro_model ----

    #[test]
    fn bake_single_voxel_is_one_cube() {
        let reg = BlockRegistry::new();
        let mesh = bake_micro_model(&one_voxel(8, 0, 0, 0), &reg);
        // 6 faces × 4 verts = 24 verts; 6 faces × 2 tris × 3 = 36 indices.
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
        // a cell at (0,0,0) of an 8-grid spans [0, 0.125] on each axis
        for v in &mesh.vertices {
            for c in v.position {
                assert!((0.0..=0.125 + 1e-6).contains(&c), "vert {c} outside cell");
            }
        }
    }

    #[test]
    fn bake_fully_solid_is_shell_only_scale_8() {
        let reg = BlockRegistry::new();
        let mesh = bake_micro_model(&solid_model(8, STONE), &reg);
        // A fully-solid cube greedy-merges each of 6 faces to ONE quad.
        // 6 quads → 24 verts, 36 indices — NOT 8³ cubes (512×36 = 18432 idx).
        assert_eq!(mesh.indices.len(), 36, "fully-solid should bake to 6 merged faces");
        assert_eq!(mesh.vertices.len(), 24);
        assert!(mesh.indices.len() < 8 * 8 * 8 * 36);
    }

    #[test]
    fn bake_fully_solid_is_shell_only_scale_16() {
        let reg = BlockRegistry::new();
        let mesh = bake_micro_model(&solid_model(16, STONE), &reg);
        assert_eq!(mesh.indices.len(), 36);
        assert_eq!(mesh.vertices.len(), 24);
    }

    #[test]
    fn bake_culls_shared_interior_face() {
        let reg = BlockRegistry::new();
        // Two voxels adjacent along x: the shared face between them is culled,
        // and the 4 side faces merge across both cells → 6 quads total (36 idx),
        // NOT 12 faces (72 idx) as two independent cubes would give.
        let model = MicroModelData {
            version: MICRO_MODEL_VERSION,
            scale: 8,
            voxels: vec![
                MicroVoxel { mx: 0, my: 0, mz: 0, block_id: STONE },
                MicroVoxel { mx: 1, my: 0, mz: 0, block_id: STONE },
            ],
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        };
        let mesh = bake_micro_model(&model, &reg);
        assert_eq!(mesh.indices.len(), 36, "interior face should be culled + sides merged");
    }

    #[test]
    fn bake_empty_model_is_empty_mesh() {
        let reg = BlockRegistry::new();
        let model = MicroModelData {
            version: MICRO_MODEL_VERSION,
            scale: 8,
            voxels: Vec::new(),
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        };
        let mesh = bake_micro_model(&model, &reg);
        assert!(mesh.vertices.is_empty());
        assert!(mesh.indices.is_empty());
    }

    #[test]
    fn bake_produces_valid_index_buffer() {
        let reg = BlockRegistry::new();
        // a non-trivial shape: an L of stone
        let model = MicroModelData {
            version: MICRO_MODEL_VERSION,
            scale: 8,
            voxels: vec![
                MicroVoxel { mx: 0, my: 0, mz: 0, block_id: STONE },
                MicroVoxel { mx: 1, my: 0, mz: 0, block_id: STONE },
                MicroVoxel { mx: 0, my: 1, mz: 0, block_id: STONE },
            ],
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        };
        let mesh = bake_micro_model(&model, &reg);
        let n = mesh.vertices.len() as u32;
        assert!(!mesh.indices.is_empty());
        assert!(mesh.indices.iter().all(|&i| i < n), "index out of range");
        assert_eq!(mesh.indices.len() % 3, 0, "indices must be whole triangles");
    }

    // ---- parse_micro_model_json (loader skeleton) ----

    #[test]
    fn parse_micro_model_json_reads_wrapper() {
        let json = r#"{
            "micro_model": {
                "version": 1,
                "scale": 16,
                "voxels": [{ "mx": 0, "my": 0, "mz": 0, "block_id": 1 }],
                "author_npub": "",
                "derivation_chain": []
            }
        }"#;
        let m = parse_micro_model_json(json).expect("parse");
        assert_eq!(m.scale, 16);
        assert_eq!(m.voxels.len(), 1);
        assert_eq!(m.voxels[0].block_id, STONE);
    }

    #[test]
    fn parse_micro_model_json_round_trips() {
        let original = solid_model(8, STONE);
        let wrapped = BundledMicroModel { micro_model: original.clone() };
        let s = serde_json::to_string(&wrapped).unwrap();
        let parsed = parse_micro_model_json(&s).unwrap();
        assert_eq!(parsed.content_hash(), original.content_hash());
    }

    // ---- bake winding guard (review finding: lock the doc-comment's claim) ----

    #[test]
    fn bake_single_voxel_faces_wind_outward() {
        let reg = BlockRegistry::new();
        let mesh = bake_micro_model(&one_voxel(8, 0, 0, 0), &reg);
        assert_eq!(mesh.indices.len(), 36);
        // For each of the 6 quads, the geometric normal cross(p1-p0, p2-p0) must
        // point the SAME way as the declared per-vertex normal — i.e. CCW /
        // outward under FrontFace::Ccw. A swapped corner (back-face winding) or a
        // flipped declared normal would make the face invisible yet still pass the
        // count/bbox assertions, so this is the real regression guard.
        let mut normals = Vec::new();
        for q in 0..6 {
            let b = q * 4;
            let p0 = mesh.vertices[b].position;
            let p1 = mesh.vertices[b + 1].position;
            let p2 = mesh.vertices[b + 2].position;
            let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
            let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
            let cross = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let n = mesh.vertices[b].normal;
            let dot = cross[0] * n[0] + cross[1] * n[1] + cross[2] * n[2];
            assert!(dot > 0.0, "quad {q} winds inward (dot={dot}) — would be culled");
            for k in 0..4 {
                assert_eq!(mesh.vertices[b + k].normal, n, "quad {q} has mixed normals");
            }
            normals.push(n);
        }
        // the 6 faces cover the 6 axis directions exactly
        for dir in [
            [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0], [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0], [0.0, 0.0, -1.0],
        ] {
            assert!(normals.contains(&dir), "missing face normal {dir:?}");
        }
    }

    // ---- from_plan OutOfBounds (review finding: untested defensive path) ----

    #[test]
    fn from_plan_rejects_out_of_bounds_cell() {
        // Declared envelope stays small (width 3) so TooLarge does NOT fire, but a
        // cell coordinate exceeds the chosen scale — only the per-cell guard
        // catches it. This is the envelope-inconsistent (corrupt/hand-authored)
        // case OutOfBounds exists for.
        let mut plan = PlanData::debug_3x3_stone();
        plan.cells.push(crate::plan::CapturedCell { rx: 20, ry: 0, rz: 0, block_id: STONE });
        assert_eq!(
            MicroModelData::from_plan(&plan, MICRO_SCALE_16).unwrap_err(),
            BakeRefusal::OutOfBounds
        );
    }

    // ---- content_hash dedup (review finding: hash-identity == bake-identity) ----

    #[test]
    fn content_hash_dedups_duplicate_voxels() {
        // A duplicate voxel collapses to the same grid cell at bake time, so the
        // two models bake to an identical mesh — they MUST hash identically, or the
        // Phase-B content-hash cache stores two entries for one visual asset.
        let base = one_voxel(8, 2, 2, 2);
        let mut dup = base.clone();
        dup.voxels.push(MicroVoxel { mx: 2, my: 2, mz: 2, block_id: STONE });
        assert_eq!(base.content_hash(), dup.content_hash());
        let reg = BlockRegistry::new();
        assert_eq!(
            bake_micro_model(&base, &reg).indices.len(),
            bake_micro_model(&dup, &reg).indices.len()
        );
    }

    // ---- bundled loader skeleton (review finding: prove include_str! pipeline) ----

    #[test]
    fn bundled_example_loads_and_bakes() {
        let models = load_bundled_micro_models();
        assert!(!models.is_empty(), "the bundled example asset should load");
        let reg = BlockRegistry::new();
        for m in &models {
            let mesh = bake_micro_model(m, &reg);
            assert!(!mesh.indices.is_empty(), "bundled asset baked to empty mesh");
            let n = mesh.vertices.len() as u32;
            assert!(mesh.indices.iter().all(|&i| i < n), "bundled asset bad index");
        }
    }
}

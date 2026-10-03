//! Greedy meshing for chunk rendering.
//!
//! Spec 03 Section 2.3: Greedy meshing merges adjacent faces with identical attributes
//! into larger quads, reducing vertex count by 60-80% for natural terrain.
//!
//! Vertex format: position + normal + texture layer + UV (tiled for merged faces).

use crate::block::{self, BlockId, BlockRegistry, AIR, WATER};
use crate::chunk::{Chunk, CHUNK_SIZE};
use crate::world::World;

/// Vertex format for chunk meshes: position, normal, texture array
/// layer, UV, and **per-vertex light** (Spec 30, 0.0–1.0; 1.0 = fully
/// bright). The light value is sampled at mesh-build time from the
/// adjacent block's chunk light data; the fragment shader multiplies
/// the texture-lit colour by this factor so dark corners actually
/// darken, while bright areas (sky / near a torch) stay full.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tex_layer: u32,
    pub uv: [f32; 2],
    /// Block-light channel (torch/lava/lamp), 0..=1 (+ the >1.5 emissive
    /// sentinel). For non-terrain paths this is the FINAL brightness and
    /// `sky_light` is 0.
    pub light: f32,
    /// Sky-light channel, 0..=1 — scaled by time-of-day in the shader
    /// (`sky * sun_brightness`), so nights actually darken sky-lit faces
    /// while torch-light keeps working (P1, 2026-07-04). Non-terrain
    /// paths pass 0 (their `light` already holds the combined value).
    pub sky_light: f32,
}

impl Vertex {
    /// Full brightness — used by the small-cube path, water mesh,
    /// and entity meshes while lighting BFS isn't sampling them.
    /// Spec 30 Phase E swaps this out for `chunk.light_at(adj)`
    /// inside `greedy_face`.
    pub const FULL_BRIGHT: f32 = 1.0;

    const ATTRIBS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3,  // position
        1 => Float32x3,  // normal
        2 => Uint32,     // tex_layer
        3 => Float32x2,  // uv
        4 => Float32,    // block light
        5 => Float32,    // sky light (0 on non-terrain paths)
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// A built mesh ready for upload to GPU.
#[derive(Clone, Debug, Default)]
pub struct ChunkMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// One instanced plant (flower / tall grass / crop / berry bush). The
/// renderer draws all of a chunk's plants with a single instanced draw of a
/// shared unit cross-billboard, so each plant costs this 32-byte record
/// instead of 8 baked vertices (~288 bytes) in the chunk mesh. `pos` is the
/// world-space min corner of the plant's AABB; `size` is `max - min`; the
/// unit cross (local coords in `[0,1]³`) is scaled by `size` and offset by
/// `pos` in the vertex shader, preserving the per-block footprint that
/// `non_solid_shape_for` gives each species.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PlantInstance {
    pub pos: [f32; 3],
    pub size: [f32; 3],
    pub tex_layer: u32,
    pub light: f32,
    /// Campaign N — sky-light channel (0..=1), split from `light` so the
    /// shader's `max(block, sky * sun.w)` dims plants at night.
    pub sky: f32,
}

impl PlantInstance {
    const ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        // Continue location numbering after the unit-cross vertex (0,1).
        2 => Float32x3, // instance pos
        3 => Float32x3, // instance size
        4 => Uint32,    // tex_layer
        5 => Float32,   // block light
        6 => Float32,   // sky light
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// A placed micro-model instance (owner-inbox #18). Byte-identical to
/// `PlantInstance` (`pos` = host block min corner, `size` = host extent, usually
/// `[1,1,1]`; `tex_layer` is unused by the near micro shader but kept so the
/// Phase-D far-LOD can reuse the *plant* pipeline + this very buffer for a
/// billboard fallback; `light` is the placement light). The bake-time per-vertex
/// texture/normal live in the shared shell geometry, not here.
///
/// Its `layout()` puts the instance attrs at **locations 5-8**, because a micro
/// pipeline pairs this with the baked shell's full `Vertex` (locations 0-4) —
/// unlike the plant pipeline, whose `PlantGeoVertex` uses only 0-1 and so leaves
/// 2-5 free for `PlantInstance`. Same 32-byte layout as `PlantInstance`, so the
/// two buffers are interchangeable at the byte level.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MicroInstance {
    pub pos: [f32; 3],
    pub size: [f32; 3],
    pub tex_layer: u32,
    pub light: f32,
    /// Campaign N — sky-light channel (0..=1); see `PlantInstance.sky`.
    pub sky: f32,
}

impl MicroInstance {
    const ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        // After the baked shell `Vertex` (locations 0-5, incl. sky_light).
        6 => Float32x3, // instance pos
        7 => Float32x3, // instance size
        8 => Uint32,    // tex_layer (far-LOD billboard only; near uses the shell's)
        9 => Float32,   // placement block light
        10 => Float32,  // placement sky light
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Group a chunk's micro-model instances by block type — one batch per distinct
/// type. The renderer uploads one GPU instance buffer per batch and issues one
/// instanced draw per batch, so the **batch count == the draw-call count** for
/// the chunk. Pure (no GPU) so it's unit-testable as a regression proxy for the
/// "one draw per micro-model type" cost guarantee (the doc's Phase-B acceptance).
pub fn group_micro_instances(
    instances: &[(BlockId, MicroInstance)],
) -> Vec<(BlockId, Vec<MicroInstance>)> {
    let mut by_type: ahash::AHashMap<BlockId, Vec<MicroInstance>> = ahash::AHashMap::new();
    for (block_id, inst) in instances {
        by_type.entry(*block_id).or_default().push(*inst);
    }
    by_type.into_iter().collect()
}

/// Per-particle GPU instance (2026-07-05 particle framework). Expanded into a
/// camera-facing quad by `vs_particle` using `CameraUniform.cam_right/cam_up`
/// (per-player → correct in split-screen). 48 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleInstance {
    pub pos: [f32; 3],
    /// World-space half-extent of the billboard.
    pub size: f32,
    pub color: [f32; 4],
    pub tex_layer: u32,
    pub _pad: [u32; 3],
}

impl ParticleInstance {
    const ATTRIBS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        // Continue location numbering after the unit-quad vertex (0,1).
        2 => Float32x4, // pos.xyz + size in w
        3 => Float32x4, // color rgba
        4 => Uint32,    // tex_layer
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// The shared unit quad for particle billboards: local xy in [-0.5, 0.5],
/// z = 0, uv 0..1. Reuses [`PlantGeoVertex`] (local + uv) as the vertex type.
pub fn particle_unit_quad() -> (Vec<PlantGeoVertex>, Vec<u32>) {
    let verts = vec![
        PlantGeoVertex { local: [-0.5, -0.5, 0.0], uv: [0.0, 1.0] },
        PlantGeoVertex { local: [0.5, -0.5, 0.0], uv: [1.0, 1.0] },
        PlantGeoVertex { local: [0.5, 0.5, 0.0], uv: [1.0, 0.0] },
        PlantGeoVertex { local: [-0.5, 0.5, 0.0], uv: [0.0, 0.0] },
    ];
    (verts, vec![0, 1, 2, 0, 2, 3])
}

/// One vertex of the shared unit cross-billboard (local space `[0,1]³`).
/// Position + uv only; lighting/texture come from the per-instance data.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PlantGeoVertex {
    pub local: [f32; 3],
    pub uv: [f32; 2],
}

impl PlantGeoVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
        0 => Float32x3, // local position
        1 => Float32x2, // uv
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// The shared cross-billboard geometry: two perpendicular quads in local
/// `[0,1]³` space. Drawn with `cull_mode: None` so each quad shows from both
/// sides — a cross visible from every direction with just 8 verts / 12
/// indices, scaled + positioned per instance in the vertex shader.
pub fn plant_unit_cross() -> (Vec<PlantGeoVertex>, Vec<u32>) {
    let v = |x: f32, y: f32, z: f32, u: f32, w: f32| PlantGeoVertex {
        local: [x, y, z],
        uv: [u, w],
    };
    // uv (0,1)=bottom-left … (0,0)=top-left so the full texture maps once.
    let verts = vec![
        // Plane A: (0,0)→(1,1) diagonal.
        v(0.0, 0.0, 0.0, 0.0, 1.0),
        v(1.0, 0.0, 1.0, 1.0, 1.0),
        v(1.0, 1.0, 1.0, 1.0, 0.0),
        v(0.0, 1.0, 0.0, 0.0, 0.0),
        // Plane B: (0,1)→(1,0) diagonal.
        v(0.0, 0.0, 1.0, 0.0, 1.0),
        v(1.0, 0.0, 0.0, 1.0, 1.0),
        v(1.0, 1.0, 0.0, 1.0, 0.0),
        v(0.0, 1.0, 1.0, 0.0, 0.0),
    ];
    let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
    (verts, indices)
}

/// Dual mesh output: opaque geometry + transparent water geometry, plus the
/// chunk's instanced plants (flowers/grass/crops — drawn as cross billboards
/// in a dedicated instanced pass rather than baked into `opaque`).
pub struct ChunkMeshes {
    pub opaque: ChunkMesh,
    pub water: ChunkMesh,
    /// #131 — solid + `transparent` blocks (glass, and future see-through
    /// leaves). The opaque greedy pass skips `is_transparent` cells and the
    /// small-cube pass skips solids, so without this bucket they emit no
    /// geometry and render invisible. Drawn in the alpha-blended, depth-read-only
    /// transparent pass alongside water (back-to-front), so glass shows as glass.
    pub transparent: ChunkMesh,
    pub plants: Vec<PlantInstance>,
    /// Owner-inbox #1/2/3 — wallpaper face-overlay decal quads for this chunk.
    /// One quad (6 verts) per painted face, drawn in a dedicated alpha-blended,
    /// depth-biased pass (the crack-overlay render technique). Rebuilt only on
    /// the chunk dirty gate, like `plants`.
    pub decals: Vec<Vertex>,
    /// Owner-inbox #18 — placed micro-models in this chunk, tagged by `block_id`
    /// so the renderer can draw one instanced batch per type against that type's
    /// shared baked shell geometry. A block routes here (instead of `plants`)
    /// when it has a registered micro-model (`World::micro_registry`).
    pub micro_instances: Vec<(BlockId, MicroInstance)>,
    /// Owner-inbox #18 Phase D — the far-LOD fallback billboard for each placed
    /// micro-model: the SAME cross-billboard the block would have had without a
    /// micro-model (inset species AABB from `non_solid_shape_for` + `tex_side`
    /// sprite), so distant flowers match their pre-#18 footprint instead of a
    /// full-block cross. Drawn (via the plant pipeline) only for chunks past the
    /// LOD threshold; near chunks draw `micro_instances` (the 3D shell) instead.
    pub micro_billboards: Vec<PlantInstance>,
}

/// The 6 face directions. `pub` so the block-interaction + storage layers can
/// map a raycast `face_normal` to a stable face index (Top=0 … West=5) for the
/// wallpaper face-overlay table (owner-inbox #1/2/3). The discriminant order is
/// load-bearing: `build_chunk_meshes` iterates `[Top,Bottom,North,South,East,West]`
/// and `index()` + the save format both depend on it — do not reorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Top,    // +Y
    Bottom, // -Y
    North,  // -Z
    South,  // +Z
    East,   // +X
    West,   // -X
}

impl Face {
    /// Map an integer face normal (e.g. `RayHit.face_normal`) to its face.
    /// Returns `None` for the zero normal or any non-unit-axis normal.
    pub fn from_normal(n: [i32; 3]) -> Option<Face> {
        match n {
            [0, 1, 0] => Some(Face::Top),
            [0, -1, 0] => Some(Face::Bottom),
            [0, 0, -1] => Some(Face::North),
            [0, 0, 1] => Some(Face::South),
            [1, 0, 0] => Some(Face::East),
            [-1, 0, 0] => Some(Face::West),
            _ => None,
        }
    }

    /// Stable 0..6 index (Top=0, Bottom=1, North=2, South=3, East=4, West=5)
    /// used as the per-face slot in `World.face_attachments` and the save format.
    pub fn index(self) -> usize {
        match self {
            Face::Top => 0,
            Face::Bottom => 1,
            Face::North => 2,
            Face::South => 3,
            Face::East => 4,
            Face::West => 5,
        }
    }

    /// Inverse of `index()` — map a 0..6 slot back to its face.
    pub fn from_index(i: usize) -> Option<Face> {
        match i {
            0 => Some(Face::Top),
            1 => Some(Face::Bottom),
            2 => Some(Face::North),
            3 => Some(Face::South),
            4 => Some(Face::East),
            5 => Some(Face::West),
            _ => None,
        }
    }

    fn normal(&self) -> [f32; 3] {
        match self {
            Face::Top => [0.0, 1.0, 0.0],
            Face::Bottom => [0.0, -1.0, 0.0],
            Face::North => [0.0, 0.0, -1.0],
            Face::South => [0.0, 0.0, 1.0],
            Face::East => [1.0, 0.0, 0.0],
            Face::West => [-1.0, 0.0, 0.0],
        }
    }

    fn offset(&self) -> (i32, i32, i32) {
        match self {
            Face::Top => (0, 1, 0),
            Face::Bottom => (0, -1, 0),
            Face::North => (0, 0, -1),
            Face::South => (0, 0, 1),
            Face::East => (1, 0, 0),
            Face::West => (-1, 0, 0),
        }
    }
}

/// Build opaque + water meshes for a single sub-chunk.
pub fn build_chunk_meshes(
    cx: i32,
    cy: i32,
    cz: i32,
    world: &World,
    registry: &BlockRegistry,
) -> ChunkMeshes {
    let empty = || ChunkMesh { vertices: vec![], indices: vec![] };
    let chunk = match world.get_chunk(cx, cy, cz) {
        Some(c) => c,
        None => return ChunkMeshes { opaque: empty(), water: empty(), transparent: empty(), plants: vec![], decals: vec![], micro_instances: vec![], micro_billboards: vec![] },
    };

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut plants = Vec::new();
    let mut micro_instances = Vec::new();
    let mut micro_billboards = Vec::new();

    let origin_x = cx * CHUNK_SIZE as i32;
    let origin_y = cy * CHUNK_SIZE as i32;
    let origin_z = cz * CHUNK_SIZE as i32;

    for face in [Face::Top, Face::Bottom, Face::North, Face::South, Face::East, Face::West] {
        greedy_face(
            chunk,
            face,
            origin_x,
            origin_y,
            origin_z,
            world,
            registry,
            &mut vertices,
            &mut indices,
        );
    }

    // Spec 30 Phase A — small-cube emit path for non-solid + transparent
    // blocks (TORCH, TALL_GRASS, CAMPFIRE_SMOKE, future flowers/lanterns).
    // The greedy mesher above skips them entirely because they're flagged
    // `transparent=true`, so without this pass they'd be invisible.
    emit_non_solid_blocks(
        chunk,
        origin_x,
        origin_y,
        origin_z,
        world,
        registry,
        &mut vertices,
        &mut indices,
        &mut plants,
        &mut micro_instances,
        &mut micro_billboards,
    );

    let opaque = ChunkMesh { vertices, indices };
    let water = build_water_mesh(chunk, origin_x, origin_y, origin_z, world, registry);
    // #131 — solid + transparent blocks (glass, …) in their own bucket.
    let transparent = build_transparent_mesh(chunk, origin_x, origin_y, origin_z, world, registry);

    // Owner-inbox #1/2/3 — wallpaper face overlays for blocks in this chunk.
    let mut decals = Vec::new();
    emit_face_decals(origin_x, origin_y, origin_z, world, registry, &mut decals);

    ChunkMeshes { opaque, water, transparent, plants, decals, micro_instances, micro_billboards }
}

/// Spec 30 Phase A — returns the per-block min/max offsets (each in
/// 0.0..=1.0 inside the block) for non-solid transparent blocks that
/// need bespoke geometry. Returns `None` for blocks that don't get
/// the small-cube treatment (AIR, water, etc.).
fn non_solid_shape_for(block_id: BlockId) -> Option<([f32; 3], [f32; 3])> {
    match block_id {
        // Thin tall pole — narrow XZ, ~half block tall, base at y=0.
        block::TORCH => Some(([0.4, 0.0, 0.4], [0.6, 0.6, 0.6])),
        // Wider but shorter — fills most of the X-Z footprint.
        block::TALL_GRASS => Some(([0.15, 0.0, 0.15], [0.85, 0.85, 0.85])),
        // Wild dye flowers (Spec 35). Bespoke shape so they don't fall
        // through to the catch-all default, which made them read as
        // chunky full-height cubes (playtest 2026-05-28: "the flowers
        // are too big"). Identical AABB across the three flowers — they
        // distinguish by colour/texture, not silhouette — clearly
        // narrower in XZ *and* shorter in Y than tall grass so they
        // read as individual stems.
        block::CORNFLOWER | block::FIELD_POPPY | block::BUTTERCUP =>
            Some(([0.30, 0.0, 0.30], [0.70, 0.65, 0.70])),
        // Spec 18 / 22 — smoke pillar. Near-full block with a slight
        // inset so neighbouring smoke cells don't z-fight at the seams
        // and the column reads as a hazy stack rather than a hard cube.
        block::CAMPFIRE_SMOKE => Some(([0.05, 0.0, 0.05], [0.95, 1.0, 0.95])),
        // P10 — Lava. A full-footprint fluid block, surface set slightly below
        // the top (0.9) so it reads as a pooled liquid rather than a hard cube.
        block::LAVA => Some(([0.0, 0.0, 0.0], [1.0, 0.9, 1.0])),
        // Fire (2026-07-04) — full-footprint flame, tongues reach ~0.7 high so
        // it reads as a fire on the block, not a burning cube.
        block::FIRE => Some(([0.0, 0.0, 0.0], [1.0, 0.7, 1.0])),
        // #30 — Ladder. A thin full-height back-panel against the +Z face. Per-
        // wall orientation waits on the block-state foundation; a centred-rear
        // panel is the sensible default for now.
        block::LADDER => Some(([0.0, 0.0, 0.0], [1.0, 1.0, 0.12])),
        // #30 — Carpet. Full XZ footprint, a thin (~1/16) decorative top layer
        // sitting at the floor; you walk on the block beneath it.
        block::CARPET => Some(([0.0, 0.0, 0.0], [1.0, 0.0625, 1.0])),
        // Default for every OTHER non-solid + transparent block — i.e. all
        // crops, flowers, fibre plants, saplings, etc. (2026-05-27 fix). The
        // caller has already filtered to non-solid + transparent + non-air/
        // water, so anything reaching here is a plant-like decoration that
        // should render as a near-full-height small cross-cube. Before this,
        // `None` skipped them → they were INVISIBLE in-game (the same class
        // of bug as the missing mob models): every crop (wheat/carrot/potato/
        // corn/papyrus), berry bush, and the new dye-flowers + fibre crops
        // were never emitted. This is the "future flowers/lanterns" hook the
        // Spec 30 small-cube path was built for.
        _ => Some(([0.12, 0.0, 0.12], [0.88, 1.0, 0.88])),
    }
}

/// Spec 22 Phase 7 — walk down from a CAMPFIRE_SMOKE cell to find the
/// source campfire (CAMPFIRE or CAMPFIRE_UNLIT) and read its
/// `raid_warning_active` flag. Returns `false` if no campfire is
/// reachable within [`crate::campfire::SMOKE_PILLAR_HEIGHT`] cells
/// below (orphan smoke after the campfire was broken — handle
/// defensively; `cleanup_campfire` normally clears these). Walk stops
/// at the first non-smoke, non-campfire cell so the lookup doesn't
/// leak into unrelated columns.
fn smoke_cell_warning_active(world: &World, wx: i32, wy: i32, wz: i32) -> bool {
    let max = crate::campfire::SMOKE_PILLAR_HEIGHT;
    for dy in 1..=max {
        let py = wy - dy;
        let b = world.get_block(wx, py, wz);
        if b == block::CAMPFIRE || b == block::CAMPFIRE_UNLIT {
            return world
                .campfire_at((wx, py, wz))
                .map(|cf| cf.raid_warning_active)
                .unwrap_or(false);
        }
        if b != block::CAMPFIRE_SMOKE {
            return false;
        }
    }
    false
}

/// Spec 22 Phase 7 — pick the texture layer for a CAMPFIRE_SMOKE cell
/// at (wx, wy, wz). Returns the warning-tinted layer when the source
/// campfire below has `raid_warning_active = true`, otherwise the
/// normal grey smoke layer.
fn campfire_smoke_tex_layer(world: &World, wx: i32, wy: i32, wz: i32) -> u32 {
    if smoke_cell_warning_active(world, wx, wy, wz) {
        block::TEX_CAMPFIRE_SMOKE_WARNING
    } else {
        block::TEX_CAMPFIRE_SMOKE
    }
}

/// Spec 30 Phase A — iterate the chunk and emit a small textured cube
/// for each TORCH / TALL_GRASS / CAMPFIRE_SMOKE / future non-solid
/// transparent block. Reads per-block light from the chunk so the
/// small cube respects the lighting BFS pass. `world` is consulted
/// for per-cell tint decisions (Spec 22 Phase 7 smoke red-shift —
/// walks down to the source campfire).
fn emit_non_solid_blocks(
    chunk: &Chunk,
    origin_x: i32,
    origin_y: i32,
    origin_z: i32,
    world: &World,
    registry: &BlockRegistry,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    plants: &mut Vec<PlantInstance>,
    micro_instances: &mut Vec<(BlockId, MicroInstance)>,
    micro_billboards: &mut Vec<PlantInstance>,
) {
    let cs = CHUNK_SIZE;
    for y in 0..cs {
        for z in 0..cs {
            for x in 0..cs {
                let block_id = chunk.get(x, y, z);
                if block_id == AIR || block_id == WATER {
                    continue;
                }
                // Spec 40 §5 — a render-hidden cell (blown up in the Workshop)
                // emits nothing here either, so transparent / micro-model origin
                // blocks vanish just like opaque ones do in the greedy pass.
                if world.is_render_hidden(
                    origin_x + x as i32,
                    origin_y + y as i32,
                    origin_z + z as i32,
                ) {
                    continue;
                }
                // Spec 40 Phase 4 / "Phase G" — a block (SOLID or non-solid) with a
                // registered micro-model renders its baked 3D shell (near) + a
                // billboard fallback (far). Solid blocks are skipped by the greedy
                // pass (above), so this is their only emit. A solid cell's own light
                // is 0 (solid blocks block light), so sample the brightest face
                // neighbour; non-solid cells keep their own propagated light.
                if world.micro_registry.contains(block_id) {
                    let (wx, wy, wz) = (origin_x + x as i32, origin_y + y as i32, origin_z + z as i32);
                    // Campaign N — split channels so the shader's night
                    // formula applies (emission joins BLOCK).
                    let (light_f, sky_f) =
                        split_cell_light(chunk, world, registry, block_id, (x, y, z), (wx, wy, wz));
                    let bx = wx as f32;
                    let by = wy as f32;
                    let bz = wz as f32;
                    let tex_layer = registry.tex_side(block_id); // far-LOD billboard tint only
                    micro_instances.push((
                        block_id,
                        MicroInstance { pos: [bx, by, bz], size: [1.0, 1.0, 1.0], tex_layer, light: light_f, sky: sky_f },
                    ));
                    // `non_solid_shape_for` is total (catch-all `_`), so the default
                    // is unreachable-by-construction — kept defensive. v1: a SOLID
                    // sculpt borrows the plant-footprint billboard for its far-LOD;
                    // a full-cube far-LOD billboard is deferred polish.
                    let (mn, mx) = non_solid_shape_for(block_id).unwrap_or(([0.1, 0.0, 0.1], [0.9, 1.0, 0.9]));
                    micro_billboards.push(PlantInstance {
                        pos: [bx + mn[0], by + mn[1], bz + mn[2]],
                        size: [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]],
                        tex_layer,
                        light: light_f,
                        sky: sky_f,
                    });
                    continue;
                }
                // F1 — a shaped block (slab/stairs/…) emits its authored
                // sub-cuboids with per-face textures. Handled BEFORE the
                // solid-skip below so it works whether solid or not. Light:
                // a solid cell's own light is 0, so sample the brightest
                // face-neighbour (same as a solid micro-model).
                let shape = crate::block_shape::shape_of(block_id);
                if shape != crate::block_shape::BlockShape::FullCube {
                    let (wx, wy, wz) =
                        (origin_x + x as i32, origin_y + y as i32, origin_z + z as i32);
                    // Campaign N — split channels (shared helper).
                    let (light_f, sky_f) =
                        split_cell_light(chunk, world, registry, block_id, (x, y, z), (wx, wy, wz));
                    // Connecting shapes (Wall, Pane) read a live neighbour mask;
                    // all other shapes read their stored meta byte.
                    let m = if crate::block_shape::is_connecting(shape) {
                        world.connection_mask(wx, wy, wz, shape, registry)
                    } else {
                        world.meta_at(wx, wy, wz)
                    };
                    let (obx, oby, obz) = (wx as f32, wy as f32, wz as f32);
                    let tex_top = registry.tex_top(block_id);
                    let tex_side = registry.tex_side(block_id);
                    let tex_bottom = registry.tex_bottom(block_id);
                    for cuboid in crate::block_shape::render_cuboids(shape, m) {
                        let mn = [
                            obx + cuboid.min[0],
                            oby + cuboid.min[1],
                            obz + cuboid.min[2],
                        ];
                        let mx = [
                            obx + cuboid.max[0],
                            oby + cuboid.max[1],
                            obz + cuboid.max[2],
                        ];
                        emit_small_cube_textured(
                            mn, mx, tex_top, tex_side, tex_bottom, light_f, sky_f, vertices, indices,
                        );
                    }
                    // Wave 2c — a filled Item Frame draws its framed *block* item
                    // as a small cube off the plate (non-block items show only on
                    // the read-on-look HUD). The block's own textures identify it.
                    if shape == crate::block_shape::BlockShape::ItemFrame
                        && let Some(crate::item::Item::Block(fb)) = world
                            .item_frame_at((wx, wy, wz))
                            .and_then(|f| f.item.as_ref().map(|s| s.item.clone()))
                        {
                            let cube = crate::block_shape::item_frame_item_cube(
                                crate::meta::facing(m),
                            );
                            emit_small_cube_textured(
                                [obx + cube.min[0], oby + cube.min[1], obz + cube.min[2]],
                                [obx + cube.max[0], oby + cube.max[1], obz + cube.max[2]],
                                registry.tex_top(fb),
                                registry.tex_side(fb),
                                registry.tex_bottom(fb),
                                light_f,
                                sky_f,
                                vertices,
                                indices,
                            );
                        }
                    continue;
                }
                // Rail freight + Electricity — auto-connecting flat rails/cables.
                // Shape derives live from same-family cardinal neighbours (like
                // Wall/Pane): straights, curved bends, T-junctions, crosses. Clean
                // solid-colour geometry (no per-shape texture). Carts unchanged.
                if block_id == crate::rail::TRACK
                    || block_id == block::CABLE
                    || block_id == block::CABLE_LIT
                {
                    let (wx, wy, wz) =
                        (origin_x + x as i32, origin_y + y as i32, origin_z + z as i32);
                    let is_track = block_id == crate::rail::TRACK;
                    let (offsets, hw, tex): (&[f32], f32, u32) = if is_track {
                        (&[-RAIL_GAP, RAIL_GAP], 0.045, block::TEX_RAIL_STEEL)
                    } else if block_id == block::CABLE_LIT {
                        (&[0.0], 0.07, block::TEX_CABLE_COPPER_LIT)
                    } else {
                        (&[0.0], 0.07, block::TEX_CABLE_COPPER)
                    };
                    let member = |b: block::BlockId| {
                        if is_track {
                            b == crate::rail::TRACK
                        } else {
                            b == block::CABLE || b == block::CABLE_LIT
                        }
                    };
                    let bl = chunk.block_light_at(x, y, z);
                    let sl = chunk.sky_light_at(x, y, z);
                    let light_f = (bl.max(sl) as f32) / 15.0;
                    // Track gets a sleeper/ballast bed under the rails; cables don't.
                    let base_tex = if is_track { Some(block::TEX_RAIL_BASE) } else { None };
                    // Track: a rail with a rail one block up-and-over ramps toward
                    // it (45° ascending / descending). Cables stay flat — their
                    // vertical story is surface-mounting, not ramps.
                    let ascend = if is_track {
                        crate::rail::rail_ascend_dir(
                            |c| member(world.get_block(c.0, c.1, c.2)),
                            (wx, wy, wz),
                        )
                    } else {
                        None
                    };
                    if let Some(dir) = ascend {
                        emit_ramp_rail(
                            wx as f32, wy as f32, wz as f32, dir, offsets, hw, tex, base_tex,
                            light_f, vertices, indices,
                        );
                    } else {
                        let mask =
                            crate::rail::neighbour_link_mask(world, wx, wy, wz, member);
                        emit_connected_rail(
                            wx as f32, wy as f32, wz as f32, mask, offsets, hw, tex, base_tex,
                            light_f, vertices, indices,
                        );
                    }
                    continue;
                }
                // Only candidates: non-solid + transparent. Solid
                // transparent (e.g. glass) keeps the greedy path.
                if registry.is_solid(block_id) || !registry.is_transparent(block_id) {
                    continue;
                }
                let (min_off, max_off) = match non_solid_shape_for(block_id) {
                    Some(t) => t,
                    None => continue,
                };
                let bx = (origin_x + x as i32) as f32;
                let by = (origin_y + y as i32) as f32;
                let bz = (origin_z + z as i32) as f32;
                let min = [bx + min_off[0], by + min_off[1], bz + min_off[2]];
                let max = [bx + max_off[0], by + max_off[1], bz + max_off[2]];
                // Light: use the cell's own light. For emitters (torch)
                // this is the emission value; for tall grass etc. it's
                // whatever sky/block light propagated to that cell.
                // Emitters also get a min-floor so a torch's own faces
                // always show fully bright even before BFS runs.
                let bl = chunk.block_light_at(x, y, z);
                let sl = chunk.sky_light_at(x, y, z);
                let emission = registry.light_emission(block_id);
                // Campaign N — emission joins the BLOCK channel; sky rides its
                // own so the shader dims these at night.
                let light_f = (bl.max(emission) as f32) / 15.0;
                let sky_f = (sl as f32) / 15.0;
                // Spec 22 Phase 7 — smoke cells consult the source
                // campfire's raid_warning_active flag to pick the
                // red-tinted layer when a raid is incoming.
                let tex_layer = if block_id == block::CAMPFIRE_SMOKE {
                    campfire_smoke_tex_layer(
                        world,
                        origin_x + x as i32,
                        origin_y + y as i32,
                        origin_z + z as i32,
                    )
                } else {
                    registry.tex_side(block_id)
                };
                // Plants render as instanced cross billboards (collected
                // here, drawn in the renderer's dedicated plant pass); only
                // the genuinely volumetric non-solids (torch pole, smoke
                // column) keep the 6-faced small cube baked into the mesh.
                if matches!(block_id, block::TORCH | block::CAMPFIRE_SMOKE) {
                    emit_small_cube(min, max, tex_layer, light_f, sky_f, vertices, indices);
                } else {
                    plants.push(PlantInstance {
                        pos: min,
                        // Size from the raw offsets, not world-space min/max,
                        // so it's exact (no `+block_origin` rounding).
                        size: [
                            max_off[0] - min_off[0],
                            max_off[1] - min_off[1],
                            max_off[2] - min_off[2],
                        ],
                        tex_layer,
                        light: light_f,
                        sky: sky_f,
                    });
                }
            }
        }
    }
}

/// Owner-inbox #1/2/3 — emit one wallpaper decal quad per painted face whose
/// block lives in this chunk. Light is sampled from the cell ADJACENT to the
/// painted face (the room side) so the paper darkens in dim rooms instead of
/// glowing; `world.*_light_at` take world coords and resolve across chunk
/// boundaries (same idiom as `greedy_face`).
fn emit_face_decals(
    origin_x: i32,
    origin_y: i32,
    origin_z: i32,
    world: &World,
    registry: &BlockRegistry,
    decals: &mut Vec<Vertex>,
) {
    // Fast path: most chunks (and whole worlds) have no painted faces, so skip
    // the map scan entirely. PERF: when overlays DO exist this still scans the
    // global overlay map once per chunk (O(total painted), filtered by chunk
    // AABB below) — fine at alpha scale (sparse + dirty-gated). If a base ever
    // carries tens of thousands of painted faces, add a chunk-keyed index of
    // painted positions (maintained in set/remove_face_attachment + restore_
    // face_overlays + World::clear) and iterate only this chunk's bucket here.
    if world.face_attachments.is_empty() {
        return;
    }
    let cs = CHUNK_SIZE as i32;
    for ((bx, by, bz), faces) in world.iter_face_attachments() {
        // Only blocks whose cell lives in THIS chunk.
        if bx < origin_x
            || bx >= origin_x + cs
            || by < origin_y
            || by >= origin_y + cs
            || bz < origin_z
            || bz >= origin_z + cs
        {
            continue;
        }
        // Defence-in-depth: don't paint a face whose block is gone. The break
        // path clears overlays on destroy, so this should not normally fire.
        if !registry.is_solid(world.get_block(bx, by, bz)) {
            continue;
        }
        for (face_idx, slot) in faces.iter().enumerate() {
            let Some(att) = slot else { continue };
            let Some(face) = Face::from_index(face_idx) else { continue };
            let (ox, oy, oz) = face.offset();
            let (ax, ay, az) = (bx + ox, by + oy, bz + oz);
            // Campaign N — two channels so wallpaper/blueprints dim at night.
            let light = world.block_light_at(ax, ay, az) as f32 / 15.0;
            let sky = world.sky_light_at(ax, ay, az) as f32 / 15.0;
            let tex_layer = match att {
                crate::world::FaceAttachment::Wallpaper(b) => registry.tex_side(*b),
                // Task C1 — render develop state via texture swap (no shader change).
                // Developed = blueprint-blue cyanotype; Latent = pale parchment.
                crate::world::FaceAttachment::Blueprint(plan) => {
                    if plan.develop_state.is_developed() {
                        crate::block::TEX_CYANOTYPE_PRINT
                    } else {
                        crate::block::TEX_BLUEPRINT_PAPER_TOP
                    }
                }
                // Laid blank cream draughting paper, pre-capture — cream top tex.
                crate::world::FaceAttachment::BlueprintBlank => {
                    crate::block::TEX_BLUEPRINT_PAPER_TOP
                }
            };
            emit_decal_quad(face, bx, by, bz, tex_layer, light, sky, decals);
        }
    }
}

/// Owner-inbox #1/2/3 — emit one wallpaper decal quad (two triangles) for the
/// given face of the block at (bx,by,bz). Per-face windings copied from
/// `build_crack_cube` (CCW, outward) so it shares the proven inflate (`e`) +
/// the renderer's negative depth bias for anti-z-fight. `light` baked per-vertex.
fn emit_decal_quad(
    face: Face,
    bx: i32,
    by: i32,
    bz: i32,
    tex_layer: u32,
    light: f32,
    sky: f32,
    decals: &mut Vec<Vertex>,
) {
    let e = 0.003_f32;
    let x0 = bx as f32 - e;
    let y0 = by as f32 - e;
    let z0 = bz as f32 - e;
    let x1 = bx as f32 + 1.0 + e;
    let y1 = by as f32 + 1.0 + e;
    let z1 = bz as f32 + 1.0 + e;
    let (corners, normal): ([[f32; 3]; 4], [f32; 3]) = match face {
        Face::Top => ([[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]], [0.0, 1.0, 0.0]),
        Face::Bottom => ([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], [0.0, -1.0, 0.0]),
        Face::South => ([[x0, y1, z1], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1]], [0.0, 0.0, 1.0]),
        Face::North => ([[x1, y1, z0], [x1, y0, z0], [x0, y0, z0], [x0, y1, z0]], [0.0, 0.0, -1.0]),
        Face::East => ([[x1, y1, z1], [x1, y0, z1], [x1, y0, z0], [x1, y1, z0]], [1.0, 0.0, 0.0]),
        Face::West => ([[x0, y1, z0], [x0, y0, z0], [x0, y0, z1], [x0, y1, z1]], [-1.0, 0.0, 0.0]),
    };
    let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let mk = |i: usize| Vertex {
        position: corners[i],
        normal,
        tex_layer,
        uv: uvs[i],
        light,
        sky_light: sky,
    };
    decals.extend([mk(0), mk(1), mk(2), mk(0), mk(2), mk(3)]);
}

/// Campaign N — sample a cell's split `(block, sky)` light for a bake-time
/// snapshot: SOLID blocks (own cell light 0) take the brightest face-neighbour
/// per channel; non-solids take their own cell with emission folded into the
/// BLOCK channel. Shared by the micro-model and shaped-block emit paths.
fn split_cell_light(
    chunk: &Chunk,
    world: &World,
    registry: &BlockRegistry,
    block_id: BlockId,
    (x, y, z): (usize, usize, usize),
    (wx, wy, wz): (i32, i32, i32),
) -> (f32, f32) {
    let (bl, sl) = if registry.is_solid(block_id) {
        let (mut bb, mut bs) = (chunk.block_light_at(x, y, z), chunk.sky_light_at(x, y, z));
        for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            bb = bb.max(world.block_light_at(wx + dx, wy + dy, wz + dz));
            bs = bs.max(world.sky_light_at(wx + dx, wy + dy, wz + dz));
        }
        (bb, bs)
    } else {
        (
            chunk.block_light_at(x, y, z).max(registry.light_emission(block_id)),
            chunk.sky_light_at(x, y, z),
        )
    };
    ((bl as f32) / 15.0, (sl as f32) / 15.0)
}

/// Emit a 6-faced small cube with the given world-space min/max.
/// Used for sub-block geometry (torch pole, tall grass, future
/// flowers). Winding matches `emit_quad` for consistent culling.
fn emit_small_cube(
    min: [f32; 3],
    max: [f32; 3],
    tex_layer: u32,
    light: f32,
    sky: f32,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let dz = z1 - z0;

    let mut emit_face = |corners: [[f32; 3]; 4], uvs: [[f32; 2]; 4], normal: [f32; 3]| {
        let base = vertices.len() as u32;
        for i in 0..4 {
            vertices.push(Vertex {
                position: corners[i],
                normal,
                tex_layer,
                uv: uvs[i],
                light,
                sky_light: sky,
            });
        }
        indices.push(base);
        indices.push(base + 1);
        indices.push(base + 2);
        indices.push(base);
        indices.push(base + 2);
        indices.push(base + 3);
    };

    // Top (+Y)
    emit_face(
        [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        [[0.0, 0.0], [0.0, dz], [dx, dz], [dx, 0.0]],
        [0.0, 1.0, 0.0],
    );
    // Bottom (-Y)
    emit_face(
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        [[0.0, 0.0], [dx, 0.0], [dx, dz], [0.0, dz]],
        [0.0, -1.0, 0.0],
    );
    // North (-Z)
    emit_face(
        [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
        [[dx, dy], [0.0, dy], [0.0, 0.0], [dx, 0.0]],
        [0.0, 0.0, -1.0],
    );
    // South (+Z)
    emit_face(
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        [[0.0, dy], [dx, dy], [dx, 0.0], [0.0, 0.0]],
        [0.0, 0.0, 1.0],
    );
    // East (+X)
    emit_face(
        [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]],
        [[dz, dy], [0.0, dy], [0.0, 0.0], [dz, 0.0]],
        [1.0, 0.0, 0.0],
    );
    // West (-X)
    emit_face(
        [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        [[0.0, dy], [dz, dy], [dz, 0.0], [0.0, 0.0]],
        [-1.0, 0.0, 0.0],
    );
}

// --- Rail/cable connecting-arms geometry (v2) ---------------------------------
// Clean flat rails drawn from a centre toward each connected neighbour. All
// pieces are flat, double-sided quads at a small height (visible from any angle,
// no back-face-cull surprises). Straights/T/cross/stub are axis-aligned bars;
// corners are a curved ribbon. Colour comes from a solid texture layer.
const RAIL_Y: f32 = 0.07; // height above the cell floor
const RAIL_GAP: f32 = 0.20; // track: each of the two rails offset ± this from centre

/// One flat, double-sided quad from four XZ corners (world space), up normal.
fn emit_flat_quad(
    c: [[f32; 3]; 4],
    tex_layer: u32,
    light: f32,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let base = vertices.len() as u32;
    let uvs = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    for i in 0..4 {
        vertices.push(Vertex {
            position: c[i],
            normal: [0.0, 1.0, 0.0],
            tex_layer,
            uv: uvs[i],
            light,
            sky_light: 0.0,
        });
    }
    // front + back windings so the floor ribbon shows from above and below.
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
}

/// A curved rail ribbon: a fan of flat quads following an arc of radius `r`
/// (centre `cx,cz` local to the cell), half-width `hw`, sweeping `a0`→`a1` deg.
#[allow(clippy::too_many_arguments)]
fn emit_ribbon_arc(
    bx: f32, bz: f32, y: f32,
    cx: f32, cz: f32, r: f32, hw: f32,
    a0: f32, a1: f32,
    tex: u32, light: f32,
    vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>,
) {
    let steps = 8;
    let pt = |rr: f32, a_deg: f32| {
        let a = a_deg.to_radians();
        [bx + cx + rr * a.cos(), y, bz + cz + rr * a.sin()]
    };
    let (mut po, mut pi) = (pt(r + hw, a0), pt(r - hw, a0));
    for k in 1..=steps {
        let a = a0 + (a1 - a0) * (k as f32) / (steps as f32);
        let (co, ci) = (pt(r + hw, a), pt(r - hw, a));
        emit_flat_quad([po, co, ci, pi], tex, light, vertices, indices);
        po = co;
        pi = ci;
    }
}

/// Emit a rail/cable cell from its neighbour mask. `offsets` are the perpendicular
/// rail offsets from the centreline (track: `[-RAIL_GAP, RAIL_GAP]`, cable: `[0.0]`);
/// `hw` is the bar/wire half-width.
#[allow(clippy::too_many_arguments)]
fn emit_connected_rail(
    bx: f32, by: f32, bz: f32,
    mask: u8,
    offsets: &[f32], hw: f32,
    tex: u32, base_tex: Option<u32>, light: f32,
    vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>,
) {
    use crate::block_shape::{CONN_E, CONN_N, CONN_S, CONN_W};
    let y = by + RAIL_Y;
    let (n, s, w, e) = (
        mask & CONN_N != 0, mask & CONN_S != 0,
        mask & CONN_W != 0, mask & CONN_E != 0,
    );
    let count = [n, s, w, e].iter().filter(|b| **b).count();

    // Sleeper/ballast bed under the rails (track only — cables are bare wire).
    // A full-cell flat pad sitting just below the rails, so a run reads as
    // laid on a bed and adjacent cells tile into a continuous strip.
    if let Some(bt) = base_tex {
        let yb = by + 0.03;
        emit_flat_quad(
            [[bx, yb, bz], [bx, yb, bz + 1.0], [bx + 1.0, yb, bz + 1.0], [bx + 1.0, yb, bz]],
            bt, light, vertices, indices,
        );
    }

    let rect = |x0: f32, z0: f32, x1: f32, z1: f32, v: &mut Vec<Vertex>, i: &mut Vec<u32>| {
        emit_flat_quad(
            [[bx + x0, y, bz + z0], [bx + x0, y, bz + z1], [bx + x1, y, bz + z1], [bx + x1, y, bz + z0]],
            tex, light, v, i,
        );
    };

    // Corner: exactly two perpendicular neighbours -> curved ribbon per rail.
    let perp = count == 2 && !((n && s) || (w && e));
    if perp {
        // Arc centre = the shared cell corner; sweep so the ends land on the
        // straight-rail offsets (0.5 ± RAIL_GAP) at each edge.
        let (cx, cz, a0, a1) = if n && e {
            (1.0f32, 0.0f32, 180.0f32, 90.0f32)
        } else if n && w {
            (0.0, 0.0, 0.0, 90.0)
        } else if s && e {
            (1.0, 1.0, 270.0, 180.0)
        } else {
            (0.0, 1.0, 270.0, 360.0) // s && w
        };
        for &off in offsets {
            emit_ribbon_arc(bx, bz, y, cx, cz, 0.5 + off, hw, a0, a1, tex, light, vertices, indices);
        }
        return;
    }

    // Lone piece (0 neighbours) or a dead-end (1 neighbour): a FULL straight bar
    // along the axis, so the run ends flush at the block edge — no half-tile
    // stub. A single E/W neighbour runs east-west; otherwise (N/S or none)
    // north-south. The cart rolls to the edge and stops (no auto buffer-stop).
    if count <= 1 {
        let along_ew = e || w;
        for &off in offsets {
            let c = 0.5 + off;
            if along_ew {
                rect(0.0, c - hw, 1.0, c + hw, vertices, indices);
            } else {
                rect(c - hw, 0.0, c + hw, 1.0, vertices, indices);
            }
        }
        return;
    }

    // T-junction (3 neighbours): the present opposite pair is the straight
    // through-line; the lone perpendicular branch curves into it BOTH ways (a
    // symmetric turnout) rather than meeting it at a square stub.
    if count == 3 {
        let straight_ns = n && s;
        for &off in offsets {
            let c = 0.5 + off;
            if straight_ns {
                rect(c - hw, 0.0, c + hw, 1.0, vertices, indices);
            } else {
                rect(0.0, c - hw, 1.0, c + hw, vertices, indices);
            }
        }
        // The two corner arcs the branch curves along into each end of the
        // straight — same arc centres as the corner shapes: (cx, cz, a0, a1).
        let arcs: [(f32, f32, f32, f32); 2] = if straight_ns {
            if e {
                [(1.0, 0.0, 180.0, 90.0), (1.0, 1.0, 270.0, 180.0)] // E->N, E->S
            } else {
                [(0.0, 0.0, 0.0, 90.0), (0.0, 1.0, 270.0, 360.0)] // W->N, W->S
            }
        } else if n {
            [(1.0, 0.0, 180.0, 90.0), (0.0, 0.0, 0.0, 90.0)] // N->E, N->W
        } else {
            [(1.0, 1.0, 270.0, 180.0), (0.0, 1.0, 270.0, 360.0)] // S->E, S->W
        };
        for (cx, cz, a0, a1) in arcs {
            for &off in offsets {
                emit_ribbon_arc(bx, bz, y, cx, cz, 0.5 + off, hw, a0, a1, tex, light, vertices, indices);
            }
        }
        return;
    }

    // Straight (2 opposite) or cross (4-way): a bar from centre to each
    // connected edge. (2-opposite arms meet into a full straight; 4 make a +.)
    for &off in offsets {
        let c = 0.5 + off;
        if n { rect(c - hw, 0.0, c + hw, 0.5 + hw, vertices, indices); }
        if s { rect(c - hw, 0.5 - hw, c + hw, 1.0, vertices, indices); }
        if w { rect(0.0, c - hw, 0.5 + hw, c + hw, vertices, indices); }
        if e { rect(0.5 - hw, c - hw, 1.0, c + hw, vertices, indices); }
    }
}

/// A 45° ascending rail: the rails (and the bed) tilt from the low `-dir` edge
/// (this cell's floor) up to the high `+dir` edge (one block higher), so a
/// staircase of rails reads as a continuous slope and the cart rolls up/down it.
/// `dir` is the up-slope horizontal direction `(dx, dz)`.
#[allow(clippy::too_many_arguments)]
fn emit_ramp_rail(
    bx: f32, by: f32, bz: f32,
    dir: (i32, i32),
    offsets: &[f32], hw: f32,
    tex: u32, base_tex: Option<u32>, light: f32,
    vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>,
) {
    let ylo = by + RAIL_Y;
    let yhi = by + 1.0 + RAIL_Y;
    // Height climbs from the low (-dir) edge to the high (+dir) edge — matching
    // the flat rail at the bottom and the next step's rail at the top exactly.
    let h_at = |x: f32, z: f32| -> f32 {
        let frac = match dir {
            (1, 0) => x,        // ascend east:  high at x=1
            (-1, 0) => 1.0 - x, // ascend west:  high at x=0
            (0, 1) => z,        // ascend south: high at z=1
            _ => 1.0 - z,       // ascend north: high at z=0
        };
        ylo + frac * (yhi - ylo)
    };
    // Sleeper bed, tilted, a touch below the rails.
    if let Some(bt) = base_tex {
        let drop = RAIL_Y - 0.03;
        emit_flat_quad(
            [
                [bx, h_at(0.0, 0.0) - drop, bz],
                [bx, h_at(0.0, 1.0) - drop, bz + 1.0],
                [bx + 1.0, h_at(1.0, 1.0) - drop, bz + 1.0],
                [bx + 1.0, h_at(1.0, 0.0) - drop, bz],
            ],
            bt, light, vertices, indices,
        );
    }
    // Two rails running along the slope axis, tilted low -> high.
    let along_x = dir.1 == 0;
    for &off in offsets {
        let c = 0.5 + off;
        let (x0, z0, x1, z1) = if along_x {
            (0.0, c - hw, 1.0, c + hw)
        } else {
            (c - hw, 0.0, c + hw, 1.0)
        };
        emit_flat_quad(
            [
                [bx + x0, h_at(x0, z0), bz + z0],
                [bx + x0, h_at(x0, z1), bz + z1],
                [bx + x1, h_at(x1, z1), bz + z1],
                [bx + x1, h_at(x1, z0), bz + z0],
            ],
            tex, light, vertices, indices,
        );
    }
}

/// F1 — like `emit_small_cube` but textures the top (+Y), bottom (-Y) and the
/// four sides independently, so a slab/stair sub-cuboid shows its block's real
/// top vs side face. Winding matches `emit_small_cube`/`emit_quad`.
#[allow(clippy::too_many_arguments)]
fn emit_small_cube_textured(
    min: [f32; 3],
    max: [f32; 3],
    tex_top: u32,
    tex_side: u32,
    tex_bottom: u32,
    light: f32,
    sky: f32,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let dz = z1 - z0;

    let mut emit_face =
        |corners: [[f32; 3]; 4], uvs: [[f32; 2]; 4], normal: [f32; 3], tex_layer: u32| {
            let base = vertices.len() as u32;
            for i in 0..4 {
                vertices.push(Vertex {
                    position: corners[i],
                    normal,
                    tex_layer,
                    uv: uvs[i],
                    light,
                    sky_light: sky,
                });
            }
            indices.push(base);
            indices.push(base + 1);
            indices.push(base + 2);
            indices.push(base);
            indices.push(base + 2);
            indices.push(base + 3);
        };

    // Top (+Y)
    emit_face(
        [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        [[0.0, 0.0], [0.0, dz], [dx, dz], [dx, 0.0]],
        [0.0, 1.0, 0.0],
        tex_top,
    );
    // Bottom (-Y)
    emit_face(
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        [[0.0, 0.0], [dx, 0.0], [dx, dz], [0.0, dz]],
        [0.0, -1.0, 0.0],
        tex_bottom,
    );
    // North (-Z)
    emit_face(
        [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
        [[dx, dy], [0.0, dy], [0.0, 0.0], [dx, 0.0]],
        [0.0, 0.0, -1.0],
        tex_side,
    );
    // South (+Z)
    emit_face(
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        [[0.0, dy], [dx, dy], [dx, 0.0], [0.0, 0.0]],
        [0.0, 0.0, 1.0],
        tex_side,
    );
    // East (+X)
    emit_face(
        [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]],
        [[dz, dy], [0.0, dy], [0.0, 0.0], [dz, 0.0]],
        [1.0, 0.0, 0.0],
        tex_side,
    );
    // West (-X)
    emit_face(
        [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        [[0.0, dy], [dz, dy], [dz, 0.0], [0.0, 0.0]],
        [-1.0, 0.0, 0.0],
        tex_side,
    );
}

/// Build mesh for water blocks only.
/// Emits faces where the adjacent block is AIR (not other water, not solid).
fn build_water_mesh(
    chunk: &Chunk,
    origin_x: i32,
    origin_y: i32,
    origin_z: i32,
    world: &World,
    registry: &BlockRegistry,
) -> ChunkMesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Spec 39 A5 — greedy-merge water faces, the same way opaque blocks merge.
    // A flat 16×16 lake surface was 256 quads; merging makes it 1. Faces only
    // emit against AIR (unchanged rule) and merge across equal light.
    for face in [Face::Top, Face::Bottom, Face::North, Face::South, Face::East, Face::West] {
        greedy_water_face(
            chunk, face, origin_x, origin_y, origin_z, world, registry,
            &mut vertices, &mut indices,
        );
    }

    ChunkMesh { vertices, indices }
}

/// Greedy-merge one face direction of the water in a chunk (Spec 39 A5). Mirrors
/// [`greedy_face`] but with the water rule: a face exists only where the water
/// cell's neighbour in the face direction is AIR. The mask keys on light alone
/// (the block is always WATER), so cells merge across equal light, exactly like
/// the opaque mesher — producing identical geometry to the old per-cell version,
/// just with far fewer quads.
#[allow(clippy::too_many_arguments)]
fn greedy_water_face(
    chunk: &Chunk,
    face: Face,
    origin_x: i32,
    origin_y: i32,
    origin_z: i32,
    world: &World,
    registry: &BlockRegistry,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let normal = face.normal();
    let (dx, dy, dz) = face.offset();
    let cs = CHUNK_SIZE;

    for layer in 0..cs {
        // mask[v][u] = Some((light, height_key)) where a water face is visible
        // (neighbour AIR). height_key is the cell's depth level (0..=7) when
        // the cell is the top of its column, or FULL_COLUMN when more water
        // sits above it (the column continues — render the full cell). Depth
        // is in the merge key, so a merged rect is height-uniform; worldgen
        // oceans are all level 0 and keep merging into single quads.
        const FULL_COLUMN: u8 = 8;
        let mut mask: [[Option<(u8, u8, u8)>; CHUNK_SIZE]; CHUNK_SIZE] =
            [[None; CHUNK_SIZE]; CHUNK_SIZE];

        for (v, row) in mask.iter_mut().enumerate() {
            for (u, cell) in row.iter_mut().enumerate() {
                let (lx, ly, lz) = face_local_to_xyz(face, layer, u, v);
                if chunk.get(lx, ly, lz) != WATER {
                    continue;
                }
                let cell_wx = origin_x + lx as i32;
                let cell_wy = origin_y + ly as i32;
                let cell_wz = origin_z + lz as i32;
                let wx = cell_wx + dx;
                let wy = cell_wy + dy;
                let wz = cell_wz + dz;
                if world.get_block(wx, wy, wz) == AIR {
                    // Spec 30 — water is transparent; light reads from the
                    // air-side cell where the face is visible.
                    let bl = world.block_light_at(wx, wy, wz);
                    let sl = world.sky_light_at(wx, wy, wz);
                    let hkey = match face {
                        Face::Bottom => FULL_COLUMN,
                        _ => {
                            if world.get_block(cell_wx, cell_wy + 1, cell_wz) == WATER {
                                FULL_COLUMN
                            } else {
                                crate::water::level_at(world, cell_wx, cell_wy, cell_wz)
                            }
                        }
                    };
                    *cell = Some((bl, sl, hkey));
                } else if world.get_block(wx, wy, wz) == WATER
                    && !matches!(face, Face::Top | Face::Bottom)
                {
                    // Step-side strips (2026-07-06, Task 16): where two
                    // surface cells of different flow level meet, the taller
                    // cell's exposed band [neighbour_surface, my_surface] was
                    // previously unmeshed (faces only emitted against AIR —
                    // the "accepted for v1" seam). Emit it per-cell — pair-
                    // specific heights defeat greedy merging, and shorelines
                    // are sparse, so this bypasses the mask/merge path and
                    // emits directly.
                    let my_full = world.get_block(cell_wx, cell_wy + 1, cell_wz) == WATER;
                    let nb_full = world.get_block(wx, wy + 1, wz) == WATER;
                    if !my_full {
                        let my_surf = crate::water::water_surface_height(crate::water::level_at(
                            world, cell_wx, cell_wy, cell_wz,
                        ));
                        let nb_surf = if nb_full {
                            1.0
                        } else {
                            crate::water::water_surface_height(crate::water::level_at(
                                world, wx, wy, wz,
                            ))
                        };
                        if my_surf > nb_surf + 0.001 {
                            let bl = world.block_light_at(wx, wy, wz);
                            let sl = world.sky_light_at(wx, wy, wz);
                            emit_side_strip(
                                face,
                                layer,
                                u,
                                v,
                                origin_x as f32,
                                origin_y as f32,
                                origin_z as f32,
                                registry.tex_side(WATER),
                                normal,
                                bl as f32 / 15.0,
                                sl as f32 / 15.0,
                                nb_surf,
                                my_surf,
                                vertices,
                                indices,
                            );
                        }
                    }
                }
            }
        }

        let mut visited = [[false; CHUNK_SIZE]; CHUNK_SIZE];

        for v in 0..cs {
            for u in 0..cs {
                if visited[v][u] || mask[v][u].is_none() {
                    continue;
                }
                let entry = mask[v][u];

                // Extend width along u, then height along v (rectangular merge).
                let mut w = 1;
                while u + w < cs && !visited[v][u + w] && mask[v][u + w] == entry {
                    w += 1;
                }
                let mut h = 1;
                'outer: while v + h < cs {
                    for du in 0..w {
                        if visited[v + h][u + du] || mask[v + h][u + du] != entry {
                            break 'outer;
                        }
                    }
                    h += 1;
                }
                for dv in 0..h {
                    for du in 0..w {
                        visited[v + dv][u + du] = true;
                    }
                }

                let tex_layer = match face {
                    Face::Top => registry.tex_top(WATER),
                    Face::Bottom => registry.tex_bottom(WATER),
                    _ => registry.tex_side(WATER),
                };
                let (bl, sl, hkey) = entry.unwrap();
                let (bl_f, sl_f) = (bl as f32 / 15.0, sl as f32 / 15.0);
                // Depth level → lowered surface; a continued column (or a
                // bottom face) renders the full cell. Where two flow cells of
                // different level touch, the taller one's exposed step-side
                // band is meshed separately (Task 16) in the mask-build loop
                // above via `emit_side_strip` — see the comment there.
                let surface_frac = if hkey == FULL_COLUMN {
                    1.0
                } else {
                    crate::water::water_surface_height(hkey)
                };
                emit_quad_frac(
                    face, layer, u, v, w, h,
                    origin_x as f32, origin_y as f32, origin_z as f32,
                    tex_layer, normal, bl_f, sl_f, surface_frac, vertices, indices,
                );
            }
        }
    }
}

/// Build the solid + `transparent` block mesh for a chunk (#131): glass and any
/// other block flagged `solid: true, transparent: true`. Drawn in the alpha-
/// blended transparent pass. See [`greedy_transparent_face`] for the rule.
fn build_transparent_mesh(
    chunk: &Chunk,
    origin_x: i32,
    origin_y: i32,
    origin_z: i32,
    world: &World,
    registry: &BlockRegistry,
) -> ChunkMesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for face in [Face::Top, Face::Bottom, Face::North, Face::South, Face::East, Face::West] {
        greedy_transparent_face(
            chunk, face, origin_x, origin_y, origin_z, world, registry,
            &mut vertices, &mut indices,
        );
    }
    ChunkMesh { vertices, indices }
}

/// Greedy-merge one face of the SOLID + `transparent` blocks in a chunk (#131).
/// Implements Spec 03 §2.2 rule 1 + rule 3 for transparent solids: a face is
/// emitted UNLESS the neighbour is (1) a fully-opaque solid [hidden behind it]
/// or (3) the SAME block type [the glass-next-to-glass merge]. So a glass cube
/// shows against air / water / a different transparent block, but not where it
/// abuts stone or more of its own kind. Shaped (panes/slabs), micro-model and
/// render-hidden cells render via their own paths and are skipped here. The mask
/// keys on (block, light) so only same-block-same-light cells merge.
#[allow(clippy::too_many_arguments)]
fn greedy_transparent_face(
    chunk: &Chunk,
    face: Face,
    origin_x: i32,
    origin_y: i32,
    origin_z: i32,
    world: &World,
    registry: &BlockRegistry,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let normal = face.normal();
    let (dx, dy, dz) = face.offset();
    let cs = CHUNK_SIZE;

    for layer in 0..cs {
        let mut mask: [[Option<(BlockId, u8, u8)>; CHUNK_SIZE]; CHUNK_SIZE] =
            [[None; CHUNK_SIZE]; CHUNK_SIZE];

        for (v, row) in mask.iter_mut().enumerate() {
            for (u, cell) in row.iter_mut().enumerate() {
                let (lx, ly, lz) = face_local_to_xyz(face, layer, u, v);
                let block = chunk.get(lx, ly, lz);
                let cwx = origin_x + lx as i32;
                let cwy = origin_y + ly as i32;
                let cwz = origin_z + lz as i32;
                if block == AIR
                    || !(registry.is_solid(block) && registry.is_transparent(block))
                    || crate::block_shape::is_shaped(block)
                    || world.micro_registry.contains(block)
                    || world.is_render_hidden(cwx, cwy, cwz)
                {
                    continue;
                }
                let wx = origin_x + lx as i32 + dx;
                let wy = origin_y + ly as i32 + dy;
                let wz = origin_z + lz as i32 + dz;
                let adj = world.get_block(wx, wy, wz);
                let same_type = adj == block;
                let occludes = registry.is_solid(adj)
                    && !registry.is_transparent(adj)
                    && !world.is_render_hidden(wx, wy, wz);
                if !same_type && !occludes {
                    let bl = world.block_light_at(wx, wy, wz);
                    let sl = world.sky_light_at(wx, wy, wz);
                    *cell = Some((block, bl, sl));
                }
            }
        }

        let mut visited = [[false; CHUNK_SIZE]; CHUNK_SIZE];
        for v in 0..cs {
            for u in 0..cs {
                if visited[v][u] || mask[v][u].is_none() {
                    continue;
                }
                let entry = mask[v][u];
                let mut w = 1;
                while u + w < cs && !visited[v][u + w] && mask[v][u + w] == entry {
                    w += 1;
                }
                let mut h = 1;
                'outer: while v + h < cs {
                    for du in 0..w {
                        if visited[v + h][u + du] || mask[v + h][u + du] != entry {
                            break 'outer;
                        }
                    }
                    h += 1;
                }
                for dv in 0..h {
                    for du in 0..w {
                        visited[v + dv][u + du] = true;
                    }
                }
                let (block, bl, sl) = entry.unwrap();
                let tex_layer = match face {
                    Face::Top => registry.tex_top(block),
                    Face::Bottom => registry.tex_bottom(block),
                    _ => registry.tex_side(block),
                };
                let (bl_f, sl_f) = (bl as f32 / 15.0, sl as f32 / 15.0);
                emit_quad(
                    face, layer, u, v, w, h,
                    origin_x as f32, origin_y as f32, origin_z as f32,
                    tex_layer, normal, bl_f, sl_f, vertices, indices,
                );
            }
        }
    }
}

/// Greedy meshing for one face direction across the entire chunk.
fn greedy_face(
    chunk: &Chunk,
    face: Face,
    origin_x: i32,
    origin_y: i32,
    origin_z: i32,
    world: &World,
    registry: &BlockRegistry,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let normal = face.normal();
    let (dx, dy, dz) = face.offset();
    let cs = CHUNK_SIZE;

    for layer in 0..cs {
        // Spec 30 — mask stores (block_id, packed_light) so the greedy
        // merge respects light boundaries; two adjacent cells with
        // different light levels become separate quads. Light is read
        // from the air-side adjacent block (where the face is visible),
        // packed as max(block_light, sky_light) in 0..=15.
        let mut mask: [[Option<(u16, u8, u8)>; CHUNK_SIZE]; CHUNK_SIZE] = [[None; CHUNK_SIZE]; CHUNK_SIZE];

        for (v, row) in mask.iter_mut().enumerate() {
            for (u, cell) in row.iter_mut().enumerate() {
                let (lx, ly, lz) = face_local_to_xyz(face, layer, u, v);
                let block = chunk.get(lx, ly, lz);

                // World coords of THIS cell (the neighbour offset dx/dy/dz is
                // added separately below for the adjacent cell).
                let cwx = origin_x + lx as i32;
                let cwy = origin_y + ly as i32;
                let cwz = origin_z + lz as i32;

                // Spec 40 Phase 4 / "Phase G" — a block with a registered
                // micro-model renders its baked shell via emit_non_solid_blocks
                // (even when solid), so the greedy pass must skip it here.
                // Spec 40 §5 — a render-hidden cell (a block currently blown up
                // in the Workshop) meshes as if it were AIR, so its original
                // texture doesn't show through / z-fight the inflated copy.
                // F1 — shaped blocks (slabs/stairs) render their sub-cuboids in
                // emit_non_solid_blocks, so the greedy pass skips them too (same
                // as micro-models). They're also `transparent` today, but key
                // off the shape so the binding doesn't depend on that flag.
                if block == AIR
                    || registry.is_transparent(block)
                    || crate::block_shape::is_shaped(block)
                    || world.micro_registry.contains(block)
                    || world.is_render_hidden(cwx, cwy, cwz)
                {
                    continue;
                }

                let wx = origin_x + lx as i32 + dx;
                let wy = origin_y + ly as i32 + dy;
                let wz = origin_z + lz as i32 + dz;

                let adj = world.get_block(wx, wy, wz);

                // A neighbour that's a micro-model block doesn't fully occlude
                // (its sculpt may have gaps), so keep our face toward it. A
                // render-hidden neighbour (§5) is treated as AIR too, so the
                // faces around a blown-up block stay visible.
                if adj == AIR
                    || registry.is_transparent(adj)
                    || world.micro_registry.contains(adj)
                    || world.is_render_hidden(wx, wy, wz)
                {
                    let bl = world.block_light_at(wx, wy, wz);
                    let sl = world.sky_light_at(wx, wy, wz);
                    *cell = Some((block, bl, sl));
                }
            }
        }

        let mut visited = [[false; CHUNK_SIZE]; CHUNK_SIZE];

        for v in 0..cs {
            for u in 0..cs {
                if visited[v][u] || mask[v][u].is_none() {
                    continue;
                }

                let (block, bl, sl) = mask[v][u].unwrap();
                let entry = Some((block, bl, sl));

                let mut w = 1;
                while u + w < cs && !visited[v][u + w] && mask[v][u + w] == entry {
                    w += 1;
                }

                let mut h = 1;
                'outer: while v + h < cs {
                    for du in 0..w {
                        if visited[v + h][u + du] || mask[v + h][u + du] != entry {
                            break 'outer;
                        }
                    }
                    h += 1;
                }

                for dv in 0..h {
                    for du in 0..w {
                        visited[v + dv][u + du] = true;
                    }
                }

                // Get texture layer for this face direction. Spec 40 (The
                // Workshop): consult the per-asset appearance override layer
                // FIRST — a registered reskin replaces the default texture for
                // every instance of this block. Falls through to the default
                // top/bottom/side texture when unoverridden (byte-identical).
                // The greedy merge stays valid because the override is keyed by
                // block id, so all merged cells (same block id) share it.
                let tex_layer = world
                    .override_registry
                    .block_face_layer(block, face.index() as u8)
                    .unwrap_or_else(|| match face {
                        Face::Top => registry.tex_top(block),
                        Face::Bottom => registry.tex_bottom(block),
                        _ => registry.tex_side(block),
                    });

                let (bl_f, sl_f) = (bl as f32 / 15.0, sl as f32 / 15.0);
                emit_quad(
                    face, layer, u, v, w, h,
                    origin_x as f32, origin_y as f32, origin_z as f32,
                    tex_layer, normal, bl_f, sl_f, vertices, indices,
                );
            }
        }
    }
}

/// Map face-local coordinates (layer, u, v) to chunk-local (x, y, z).
fn face_local_to_xyz(face: Face, layer: usize, u: usize, v: usize) -> (usize, usize, usize) {
    match face {
        Face::Top | Face::Bottom => (u, layer, v),
        Face::North | Face::South => (u, v, layer),
        Face::East | Face::West => (layer, v, u),
    }
}

/// Emit a quad (two triangles) for a greedy-merged face with texture UVs.
fn emit_quad(
    face: Face,
    layer: usize,
    u: usize,
    v: usize,
    w: usize,
    h: usize,
    ox: f32,
    oy: f32,
    oz: f32,
    tex_layer: u32,
    normal: [f32; 3],
    block_light: f32,
    sky_light: f32,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    emit_quad_frac(
        face, layer, u, v, w, h, ox, oy, oz, tex_layer, normal, block_light, sky_light, 1.0,
        vertices, indices,
    );
}

/// [`emit_quad`] with a vertical surface fraction (water depth levels): the
/// TOP plane sits at `layer + surface_frac` instead of `layer + 1`, and side
/// faces stop at the same reduced surface. `surface_frac == 1.0` is byte-
/// identical to the plain quad. Only the water mesher passes < 1.0; a merged
/// water rect is level-uniform (level is in the merge key), and a partial-
/// height side quad is always one cell tall (a water cell with water above it
/// keys as full-height instead).
#[allow(clippy::too_many_arguments)]
fn emit_quad_frac(
    face: Face,
    layer: usize,
    u: usize,
    v: usize,
    w: usize,
    h: usize,
    ox: f32,
    oy: f32,
    oz: f32,
    tex_layer: u32,
    normal: [f32; 3],
    block_light: f32,
    sky_light: f32,
    surface_frac: f32,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let base_idx = vertices.len() as u32;

    let offset = match face {
        Face::Top => surface_frac,
        Face::South | Face::East => 1.0,
        Face::Bottom | Face::North | Face::West => 0.0,
    };

    let (u_f, v_f, w_f, layer_f) = (u as f32, v as f32, w as f32, layer as f32 + offset);
    // Side faces rise `h` cells minus the missing sliver of the top cell.
    let h_f = match face {
        Face::Top | Face::Bottom => h as f32,
        _ => h as f32 - 1.0 + surface_frac,
    };

    let corners: [[f32; 3]; 4] = match face {
        Face::Top => [
            [ox + u_f, oy + layer_f, oz + v_f],
            [ox + u_f, oy + layer_f, oz + v_f + h_f],
            [ox + u_f + w_f, oy + layer_f, oz + v_f + h_f],
            [ox + u_f + w_f, oy + layer_f, oz + v_f],
        ],
        Face::Bottom => [
            [ox + u_f, oy + layer_f, oz + v_f],
            [ox + u_f + w_f, oy + layer_f, oz + v_f],
            [ox + u_f + w_f, oy + layer_f, oz + v_f + h_f],
            [ox + u_f, oy + layer_f, oz + v_f + h_f],
        ],
        Face::North => [
            [ox + u_f + w_f, oy + v_f, oz + layer_f],
            [ox + u_f, oy + v_f, oz + layer_f],
            [ox + u_f, oy + v_f + h_f, oz + layer_f],
            [ox + u_f + w_f, oy + v_f + h_f, oz + layer_f],
        ],
        Face::South => [
            [ox + u_f, oy + v_f, oz + layer_f],
            [ox + u_f + w_f, oy + v_f, oz + layer_f],
            [ox + u_f + w_f, oy + v_f + h_f, oz + layer_f],
            [ox + u_f, oy + v_f + h_f, oz + layer_f],
        ],
        Face::East => [
            [ox + layer_f, oy + v_f, oz + u_f + w_f],
            [ox + layer_f, oy + v_f, oz + u_f],
            [ox + layer_f, oy + v_f + h_f, oz + u_f],
            [ox + layer_f, oy + v_f + h_f, oz + u_f + w_f],
        ],
        Face::West => [
            [ox + layer_f, oy + v_f, oz + u_f],
            [ox + layer_f, oy + v_f, oz + u_f + w_f],
            [ox + layer_f, oy + v_f + h_f, oz + u_f + w_f],
            [ox + layer_f, oy + v_f + h_f, oz + u_f],
        ],
    };

    // UV corners: V=0 at top of face (high Y) for side faces,
    // simple (0,0)→(w,h) mapping for top/bottom.
    let uvs: [[f32; 2]; 4] = match face {
        Face::Top => [
            [0.0, 0.0], [0.0, h_f], [w_f, h_f], [w_f, 0.0],
        ],
        Face::Bottom => [
            [0.0, 0.0], [w_f, 0.0], [w_f, h_f], [0.0, h_f],
        ],
        Face::North | Face::East => [
            [w_f, h_f], [0.0, h_f], [0.0, 0.0], [w_f, 0.0],
        ],
        Face::South | Face::West => [
            [0.0, h_f], [w_f, h_f], [w_f, 0.0], [0.0, 0.0],
        ],
    };

    for i in 0..4 {
        vertices.push(Vertex {
            position: corners[i],
            normal,
            tex_layer,
            uv: uvs[i],
            light: block_light,
            sky_light,
        });
    }

    indices.push(base_idx);
    indices.push(base_idx + 1);
    indices.push(base_idx + 2);
    indices.push(base_idx);
    indices.push(base_idx + 2);
    indices.push(base_idx + 3);
}

/// Emit a 1-cell-wide vertical strip on a side face (North/South/East/West
/// only), spanning `[layer's cell + y_lo_frac, layer's cell + y_hi_frac]` in
/// world Y — a *floating* band `emit_quad_frac` can't produce, since that
/// helper always anchors the bottom of a side quad to the cell floor.
///
/// Task 16 (2026-07-06): fills the water step-side seam — where two adjacent
/// water cells have different flow levels, the taller cell's exposed vertical
/// band toward its shorter neighbour was previously left unmeshed. The corner
/// order/winding for each face is copied verbatim from `emit_quad_frac`'s
/// side-face arm, only substituting the two Y coordinates for `y_lo`/`y_hi` —
/// this keeps winding consistent with the surrounding faces so the strip
/// isn't back-face culled.
#[allow(clippy::too_many_arguments)]
fn emit_side_strip(
    face: Face,
    layer: usize,
    u: usize,
    v: usize,
    ox: f32,
    oy: f32,
    oz: f32,
    tex_layer: u32,
    normal: [f32; 3],
    block_light: f32,
    sky_light: f32,
    y_lo_frac: f32,
    y_hi_frac: f32,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let base_idx = vertices.len() as u32;

    let offset = match face {
        Face::South | Face::East => 1.0,
        Face::North | Face::West => 0.0,
        _ => unreachable!("emit_side_strip is for side faces only"),
    };

    let u_f = u as f32;
    let w_f = 1.0;
    let layer_f = layer as f32 + offset;
    let y_lo = oy + v as f32 + y_lo_frac;
    let y_hi = oy + v as f32 + y_hi_frac;
    let h_f = y_hi_frac - y_lo_frac;

    let corners: [[f32; 3]; 4] = match face {
        Face::North => [
            [ox + u_f + w_f, y_lo, oz + layer_f],
            [ox + u_f, y_lo, oz + layer_f],
            [ox + u_f, y_hi, oz + layer_f],
            [ox + u_f + w_f, y_hi, oz + layer_f],
        ],
        Face::South => [
            [ox + u_f, y_lo, oz + layer_f],
            [ox + u_f + w_f, y_lo, oz + layer_f],
            [ox + u_f + w_f, y_hi, oz + layer_f],
            [ox + u_f, y_hi, oz + layer_f],
        ],
        Face::East => [
            [ox + layer_f, y_lo, oz + u_f + w_f],
            [ox + layer_f, y_lo, oz + u_f],
            [ox + layer_f, y_hi, oz + u_f],
            [ox + layer_f, y_hi, oz + u_f + w_f],
        ],
        Face::West => [
            [ox + layer_f, y_lo, oz + u_f],
            [ox + layer_f, y_lo, oz + u_f + w_f],
            [ox + layer_f, y_hi, oz + u_f + w_f],
            [ox + layer_f, y_hi, oz + u_f],
        ],
        _ => unreachable!("emit_side_strip is for side faces only"),
    };

    let uvs: [[f32; 2]; 4] = match face {
        Face::North | Face::East => [[w_f, h_f], [0.0, h_f], [0.0, 0.0], [w_f, 0.0]],
        Face::South | Face::West => [[0.0, h_f], [w_f, h_f], [w_f, 0.0], [0.0, 0.0]],
        _ => unreachable!("emit_side_strip is for side faces only"),
    };

    for i in 0..4 {
        vertices.push(Vertex {
            position: corners[i],
            normal,
            tex_layer,
            uv: uvs[i],
            light: block_light,
            sky_light,
        });
    }

    indices.push(base_idx);
    indices.push(base_idx + 1);
    indices.push(base_idx + 2);
    indices.push(base_idx);
    indices.push(base_idx + 2);
    indices.push(base_idx + 3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{self, BlockRegistry};
    use crate::world::World;

    #[test]
    fn plant_instances_carry_split_light_channels() {
        // Campaign N (2026-07-05) — plants still freeze a bake-time light
        // snapshot, but now in TWO channels, so the shader's
        // `max(block, sky * sun.w)` formula dims them at night like terrain.
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(3, 3, 3, block::TALL_GRASS);
        w.set_block_light_at(3, 3, 3, 12);
        w.set_sky_light_at(3, 3, 3, 15);
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);
        assert_eq!(m.plants.len(), 1, "one grass = one plant instance");
        assert!((m.plants[0].light - 12.0 / 15.0).abs() < 1e-6, "block channel");
        assert!((m.plants[0].sky - 1.0).abs() < 1e-6, "sky channel");
    }

    #[test]
    fn torch_small_cube_splits_emission_from_sky() {
        // The torch pole keeps its emission in the BLOCK channel (still
        // bright at night); the sky contribution rides the sky channel.
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(5, 3, 5, block::TORCH);
        w.set_sky_light_at(5, 3, 5, 15);
        let emission = reg.light_emission(block::TORCH) as f32 / 15.0;
        assert!(emission > 0.0, "torch emits light");
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);
        assert!(!m.opaque.vertices.is_empty(), "torch pole small cube emitted");
        for v in &m.opaque.vertices {
            assert!((v.light - emission).abs() < 1e-6, "emission in block channel");
            assert!((v.sky_light - 1.0).abs() < 1e-6, "sky rides its own channel");
        }
    }

    #[test]
    fn water_depth_level_lowers_rendered_surface() {
        // A flow cell at level 4 renders its top face at the reduced surface
        // height, not the full cell top.
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(2, 2, 2, block::WATER);
        w.set_meta((2, 2, 2), crate::meta::with_aux(0, 4));
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);
        let max_y = m
            .water
            .vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::MIN, f32::max);
        let expected = 2.0 + crate::water::water_surface_height(4);
        assert!(
            (max_y - expected).abs() < 1e-5,
            "level-4 water surface at {expected}, got {max_y}"
        );
    }

    #[test]
    fn uniform_water_sheet_still_greedy_merges() {
        // Worldgen oceans are all level 0 (no meta) — the depth key must not
        // break the one-quad-per-face merge (Spec 39 A5 perf guarantee).
        let reg = BlockRegistry::new();
        let mut w = World::new();
        for x in 0..16 {
            for z in 0..16 {
                w.set_block(x, 2, z, block::WATER);
            }
        }
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);
        // 1 top + 1 bottom + 4 side quads = 6 quads = 24 vertices.
        assert_eq!(m.water.vertices.len(), 24, "sheet merges into 6 quads");
        // And the merged top sits at the source surface height (8/9).
        let max_y = m
            .water
            .vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::MIN, f32::max);
        let expected = 2.0 + crate::water::water_surface_height(0);
        assert!((max_y - expected).abs() < 1e-5, "ocean surface at {expected}, got {max_y}");
    }

    #[test]
    fn submerged_water_column_renders_full_cells() {
        // A cell with water above it keys FULL_COLUMN: its exposed side face
        // spans the whole cell so the column has no horizontal slits.
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(3, 2, 3, block::WATER);
        w.set_block(3, 3, 3, block::WATER); // column of two; lower is submerged
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);
        // The lower cell's side faces must reach y=3.0 exactly (full cell) —
        // i.e. some side vertex sits at 3.0 (the seam), and the top of the
        // whole column is the upper cell's reduced surface.
        let ys: Vec<f32> = m.water.vertices.iter().map(|v| v.position[1]).collect();
        assert!(
            ys.iter().any(|&y| (y - 3.0).abs() < 1e-5),
            "lower cell renders full height to the seam: {ys:?}"
        );
        let max_y = ys.iter().copied().fold(f32::MIN, f32::max);
        let expected = 3.0 + crate::water::water_surface_height(0);
        assert!((max_y - expected).abs() < 1e-5, "column top at {expected}, got {max_y}");
    }

    /// Vertices of the water mesh that sit on the given X boundary plane and
    /// carry the East face normal (+X) — isolates a step-side strip quad from
    /// unrelated top/bottom-face vertices that happen to share the same X.
    fn east_strip_ys(m: &ChunkMesh, boundary_x: f32) -> Vec<f32> {
        m.vertices
            .iter()
            .filter(|v| {
                (v.position[0] - boundary_x).abs() < 1e-4
                    && (v.normal[0] - 1.0).abs() < 1e-4
                    && v.normal[1].abs() < 1e-4
                    && v.normal[2].abs() < 1e-4
            })
            .map(|v| v.position[1])
            .collect()
    }

    #[test]
    fn water_step_between_levels_emits_side_strip() {
        // Task 16 — Cell A level 2 (surface ~0.667) beside cell B level 5
        // (surface ~0.333) on the +X side: A's East face toward B must now
        // emit a strip covering the exposed band [B's surface, A's surface].
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(2, 2, 2, block::WATER);
        w.set_meta((2, 2, 2), crate::meta::with_aux(0, 2));
        w.set_block(3, 2, 2, block::WATER);
        w.set_meta((3, 2, 2), crate::meta::with_aux(0, 5));
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);

        let ys = east_strip_ys(&m.water, 3.0);
        assert!(!ys.is_empty(), "expected a step-side strip quad at the A/B boundary");
        let min_y = ys.iter().copied().fold(f32::MAX, f32::min);
        let max_y = ys.iter().copied().fold(f32::MIN, f32::max);
        let expected_lo = 2.0 + crate::water::water_surface_height(5);
        let expected_hi = 2.0 + crate::water::water_surface_height(2);
        assert!((min_y - expected_lo).abs() < 0.01, "strip bottom at {expected_lo}, got {min_y}");
        assert!((max_y - expected_hi).abs() < 0.01, "strip top at {expected_hi}, got {max_y}");
    }

    #[test]
    fn water_equal_levels_emit_no_side_strip() {
        // Same two-cell setup, but both cells share level 2 — no step, so no
        // strip should appear at the boundary (vertex count is unaffected by
        // the new feature; equal-level water still merges/faces as before).
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(2, 2, 2, block::WATER);
        w.set_meta((2, 2, 2), crate::meta::with_aux(0, 2));
        w.set_block(3, 2, 2, block::WATER);
        w.set_meta((3, 2, 2), crate::meta::with_aux(0, 2));
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);

        assert!(
            east_strip_ys(&m.water, 3.0).is_empty(),
            "equal levels must not emit a step-side strip"
        );
    }

    #[test]
    fn water_neighbour_full_column_emits_no_side_strip() {
        // Cell A level 2 beside cell B, but B has water above it (FULL_COLUMN
        // — B is submerged, not the top of its column). B's effective surface
        // is the full cell height, so there is no exposed band toward A and no
        // strip should be emitted.
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(2, 2, 2, block::WATER);
        w.set_meta((2, 2, 2), crate::meta::with_aux(0, 2));
        w.set_block(3, 2, 2, block::WATER);
        w.set_meta((3, 2, 2), crate::meta::with_aux(0, 5));
        w.set_block(3, 3, 2, block::WATER); // water above B -> B is FULL_COLUMN
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);

        assert!(
            east_strip_ys(&m.water, 3.0).is_empty(),
            "a full-column neighbour must not emit a step-side strip"
        );
    }

    #[test]
    fn glass_emits_into_transparent_bucket_with_rule1_and_rule3_culling() {
        // #131 — solid+transparent blocks (glass) must emit into the NEW
        // transparent bucket (never the opaque mesh), culling against opaque
        // neighbours (rule 1) and same-type neighbours (rule 3).
        let reg = BlockRegistry::new();

        // Lone glass cube in air → all 6 faces, none mergeable → 24 verts.
        let mut w = World::new();
        w.set_block(2, 2, 2, block::GLASS);
        let m = build_chunk_meshes(0, 0, 0, &w, &reg);
        assert_eq!(m.transparent.vertices.len(), 24, "lone glass emits 6 faces");
        assert!(m.opaque.vertices.is_empty(), "glass must NOT be in the opaque mesh");

        // Glass with a STONE neighbour on +X → that face hides (rule 1) → 5 faces.
        let mut w2 = World::new();
        w2.set_block(2, 2, 2, block::GLASS);
        w2.set_block(3, 2, 2, block::STONE);
        let m2 = build_chunk_meshes(0, 0, 0, &w2, &reg);
        assert_eq!(m2.transparent.vertices.len(), 20, "glass face vs opaque stone is culled");

        // Two adjacent glass → the shared faces cull (rule 3); the outer shell is
        // all that remains (fewer verts than two independent cubes = 48).
        let mut w3 = World::new();
        w3.set_block(2, 2, 2, block::GLASS);
        w3.set_block(3, 2, 2, block::GLASS);
        let m3 = build_chunk_meshes(0, 0, 0, &w3, &reg);
        assert!(
            (24..48).contains(&m3.transparent.vertices.len()),
            "glass-glass shared faces culled, outer shell kept: got {}",
            m3.transparent.vertices.len()
        );
    }

    fn empty_world_with_chunk() -> (World, BlockRegistry) {
        let mut world = World::new();
        // Ensure chunk (0,0,0) is allocated so build_chunk_meshes doesn't bail.
        world.set_block(0, 0, 0, block::AIR);
        let registry = BlockRegistry::new();
        (world, registry)
    }

    fn single_voxel_micro() -> crate::micro_model::MicroModelData {
        crate::micro_model::MicroModelData {
            version: crate::micro_model::MICRO_MODEL_VERSION,
            scale: 8,
            voxels: vec![crate::micro_model::MicroVoxel {
                mx: 0, my: 0, mz: 0, block_id: block::STONE,
            }],
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        }
    }

    #[test]
    fn render_hidden_block_emits_no_faces() {
        // Spec 40 §5 (2026-06-18) — a render-hidden cell (a block blown up in
        // the Workshop) meshes as if it were AIR, so its original texture can't
        // show through the inflated copy. Hiding is render-only: the block data
        // is untouched (still STONE), only the mesh omits it.
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(1, 1, 1, block::STONE);
        let visible = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            !visible.opaque.vertices.is_empty(),
            "a lone stone block should emit faces when not hidden"
        );

        world.render_hidden.insert((1, 1, 1));
        let hidden = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            hidden.opaque.vertices.is_empty(),
            "a render-hidden block must emit no opaque faces"
        );
        // Data is untouched — raycast / charge / collapse / save still see it.
        assert_eq!(world.get_block(1, 1, 1), block::STONE);

        // Un-hiding restores the faces (the collapse/pin path).
        world.render_hidden.remove(&(1, 1, 1));
        let restored = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert_eq!(
            restored.opaque.vertices.len(),
            visible.opaque.vertices.len(),
            "restoring visibility re-emits exactly the original faces"
        );
    }

    #[test]
    fn registered_block_routes_to_micro_not_plants() {
        // Owner-inbox #18 — a block with a registered micro-model is collected
        // into `micro_instances` (drawn as an instanced shell), NOT into `plants`
        // (the cross-billboard). CORNFLOWER is non-solid+transparent so it reaches
        // emit_non_solid_blocks; registering an override flips its render path.
        let (mut world, registry) = empty_world_with_chunk();
        world
            .micro_registry
            .register(block::CORNFLOWER, single_voxel_micro(), &registry);
        world.set_block(1, 1, 1, block::CORNFLOWER);

        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert_eq!(meshes.micro_instances.len(), 1, "registered flower → micro path");
        assert_eq!(meshes.micro_instances[0].0, block::CORNFLOWER);
        assert!(
            meshes.plants.is_empty(),
            "a registered micro-model must NOT stay in the normal plant billboard list"
        );
        // Phase D far-LOD: the block's ORIGINAL inset billboard is emitted separately
        // (the flower's species AABB, ~0.4 wide — NOT a full-block 1.0 cross).
        assert_eq!(meshes.micro_billboards.len(), 1, "far-LOD billboard emitted");
        let bw = meshes.micro_billboards[0].size[0];
        assert!((bw - 0.4).abs() < 1e-5, "far billboard width {bw} should be the inset 0.4, not 1.0");
    }

    #[test]
    fn group_micro_instances_one_batch_per_type() {
        // CPU-side proxy for the "one instanced draw per micro-model type per
        // chunk" guarantee: the renderer makes one buffer + one draw per batch.
        let mi = |tl: u32| MicroInstance { pos: [0.0; 3], size: [1.0; 3], tex_layer: tl, light: 1.0, sky: 1.0 };
        let insts = vec![
            (131u16, mi(1)), (131, mi(1)), (132, mi(2)), (131, mi(1)), (132, mi(2)),
        ];
        let groups = group_micro_instances(&insts);
        assert_eq!(groups.len(), 2, "one batch per distinct block type");
        let mut counts: Vec<usize> = groups.iter().map(|(_, v)| v.len()).collect();
        counts.sort_unstable();
        assert_eq!(counts, vec![2, 3]);
        // a single-type chunk → exactly one batch == one draw call
        let one = group_micro_instances(&[(131, mi(1)), (131, mi(1))]);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].1.len(), 2);
        // empty → no batches
        assert!(group_micro_instances(&[]).is_empty());
    }

    #[test]
    fn block_appearance_override_retextures_solid_faces() {
        // Spec 40 (The Workshop) Phase A — a registered per-asset appearance
        // override re-textures EVERY face of that block id at the mesher seam,
        // while leaving a block id with no override byte-identical.
        use crate::override_registry::AuthoredFaces;
        let base = crate::texture_gen::texture_count();

        let (mut world, registry) = empty_world_with_chunk();
        // A lone solid block: all 6 faces border air, so all 6 are emitted.
        world.set_block(1, 1, 1, block::STONE);

        // Baseline: no override → every face uses a default stone layer, all of
        // which are below `texture_count()` (the appended-override region).
        let before = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            !before.opaque.vertices.is_empty(),
            "lone stone block should emit faces"
        );
        assert!(
            before.opaque.vertices.iter().all(|v| v.tex_layer < base),
            "unoverridden block must use stock layers (< texture_count())"
        );

        // Register a uniform reskin of STONE. Solid ⇒ all six faces dedup to one
        // appended layer at exactly `base` (= texture_count()).
        world.override_registry.add_block_design(
            block::STONE,
            crate::override_registry::NamedDesign {
                id: 0,
                name: "test".into(),
                faces: Some(AuthoredFaces::solid([200, 40, 40, 255])),
                micro_model: None,
                author_npub: String::new(),
                derivation_chain: vec![],
            },
            base,
        );
        assert_eq!(world.override_registry.appended_len(), 1);

        let after = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            after.opaque.vertices.iter().all(|v| v.tex_layer == base),
            "every stone face must now emit the override layer ({base})"
        );

        // A DIFFERENT block id stays on its stock layers — overrides are per-asset.
        world.set_block(2, 1, 1, block::DIRT);
        let mixed = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            mixed.opaque.vertices.iter().any(|v| v.tex_layer < base),
            "the unoverridden dirt block must still use a stock layer"
        );
        assert!(
            mixed.opaque.vertices.iter().any(|v| v.tex_layer == base),
            "the overridden stone block must still use the override layer"
        );
    }

    #[test]
    fn unregistered_flower_routes_to_plants() {
        // With no registered override, the flower keeps its existing cross-billboard
        // path — micro-models are strictly opt-in per block id.
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(1, 1, 1, block::CORNFLOWER);

        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(meshes.micro_instances.is_empty(), "no override registered");
        assert_eq!(meshes.plants.len(), 1, "unregistered flower stays a billboard");
    }

    #[test]
    fn bundled_flowers_route_to_micro_after_load() {
        // Phase C — once the built-in flower micro-models are registered at world
        // init, a placed flower renders as a 3D shell, not a flat billboard.
        let (mut world, registry) = empty_world_with_chunk();
        world.load_bundled_micro_models(&registry);
        world.set_block(1, 1, 1, block::CORNFLOWER);
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert_eq!(meshes.micro_instances.len(), 1, "loaded flower → micro path");
        assert_eq!(meshes.micro_instances[0].0, block::CORNFLOWER);
        assert!(meshes.plants.is_empty(), "flower no longer a billboard");
    }

    #[test]
    fn solid_block_with_micro_model_routes_to_micro_path() {
        // Spec 40 Phase 4 / "Phase G" — a SOLID block with a registered
        // micro-model is skipped by the greedy pass and emitted through the
        // micro-instance path (near shell + far billboard), exactly like the
        // non-solid flowers above. Without this hook a Workshop sculpt of stone
        // (the common target) had NO visual effect.
        use crate::micro_model::{MicroModelData, MicroVoxel, MICRO_MODEL_VERSION, MICRO_SCALE_8};
        let (mut world, reg) = empty_world_with_chunk();
        // Two solid STONE blocks side by side in chunk (0,0,0). With an override
        // registered, both should leave the greedy pass empty.
        world.set_block(0, 0, 0, block::STONE);
        world.set_block(1, 0, 0, block::STONE);
        let model = MicroModelData {
            version: MICRO_MODEL_VERSION,
            scale: MICRO_SCALE_8,
            voxels: vec![MicroVoxel { mx: 0, my: 0, mz: 0, block_id: block::WALLPAPER_RED }],
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        };
        world.micro_registry.register(block::STONE, model, &reg);

        let meshes = build_chunk_meshes(0, 0, 0, &world, &reg);
        assert_eq!(meshes.micro_instances.len(), 2, "both solid stone → micro instances");
        assert!(meshes.micro_instances.iter().all(|(b, _)| *b == block::STONE));
        assert!(meshes.opaque.vertices.is_empty(), "registered solid block is not greedy-meshed");
        assert_eq!(meshes.micro_billboards.len(), 2, "far-LOD fallback present");
    }

    #[test]
    fn face_from_normal_and_index_round_trip() {
        // Owner-inbox #1/2/3 — each unit face normal maps to its Face, and
        // index() is the stable 0..6 slot the overlay table + save format use.
        let cases = [
            ([0, 1, 0], Face::Top, 0usize),
            ([0, -1, 0], Face::Bottom, 1),
            ([0, 0, -1], Face::North, 2),
            ([0, 0, 1], Face::South, 3),
            ([1, 0, 0], Face::East, 4),
            ([-1, 0, 0], Face::West, 5),
        ];
        for (normal, face, idx) in cases {
            assert_eq!(Face::from_normal(normal), Some(face), "normal {normal:?}");
            assert_eq!(face.index(), idx, "index of {face:?}");
            assert_eq!(Face::from_index(idx), Some(face), "from_index {idx}");
        }
        // Zero / non-unit normals have no face.
        assert!(Face::from_normal([0, 0, 0]).is_none());
        assert!(Face::from_normal([1, 1, 0]).is_none());
        assert!(Face::from_normal([2, 0, 0]).is_none());
        assert!(Face::from_index(6).is_none());
    }

    #[test]
    fn painted_face_emits_one_decal_quad() {
        // Owner-inbox #1/2/3 — a painted face produces exactly one decal quad
        // (6 verts) carrying the wallpaper texture layer + the face normal; an
        // unpainted chunk produces none.
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(2, 2, 2, block::STONE);
        world.set_face_attachment(
            (2, 2, 2),
            Face::Top.index(),
            crate::world::FaceAttachment::Wallpaper(block::WALLPAPER_RED),
        );
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert_eq!(meshes.decals.len(), 6, "one painted face = one quad = 6 verts");
        let want_layer = registry.tex_side(block::WALLPAPER_RED);
        for v in &meshes.decals {
            assert_eq!(v.tex_layer, want_layer, "decal samples the wallpaper layer");
            assert_eq!(v.normal, [0.0, 1.0, 0.0], "Top face normal");
        }

        let (world2, registry2) = empty_world_with_chunk();
        assert!(
            build_chunk_meshes(0, 0, 0, &world2, &registry2).decals.is_empty(),
            "no overlays => no decals"
        );
    }

    #[test]
    fn torch_in_otherwise_empty_chunk_emits_geometry() {
        // Spec 30 Phase A — torches are flagged transparent + non-solid;
        // the greedy mesher skipped them, leaving placed torches invisible.
        // After this fix a chunk containing a TORCH produces non-empty
        // geometry via the small-cube non-greedy emit path.
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(5, 5, 5, block::TORCH);
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            !meshes.opaque.vertices.is_empty(),
            "TORCH at (5,5,5) should produce mesh geometry, got 0 vertices",
        );
    }

    #[test]
    fn water_surface_greedy_merges_into_few_quads() {
        // Spec 39 A5 — a full 16×16 water layer with air above used to emit one
        // quad per face (256 top quads = 1024 verts). Greedy merging collapses
        // the flat surface to a handful of quads.
        let (mut world, registry) = empty_world_with_chunk();
        for x in 0..16 {
            for z in 0..16 {
                world.set_block(x, 0, z, block::WATER);
            }
        }
        let m = build_chunk_meshes(0, 0, 0, &world, &registry);
        let verts = m.water.vertices.len();
        assert!(verts > 0, "water layer should emit geometry");
        // Per-cell would be ≥ 1024 verts for the top face alone; greedy gives the
        // top + bottom + 4 edge strips ≈ 6 quads = 24 verts. Well under 256.
        assert!(
            verts < 256,
            "greedy water merge should be far below the per-cell count, got {verts}",
        );
    }

    #[test]
    fn tall_grass_in_otherwise_empty_chunk_emits_geometry() {
        // Spec 30 Phase A — TALL_GRASS (transparent + non-solid) must render.
        // Since 2026-05-30 it renders as an instanced plant rather than baked
        // chunk geometry, so it shows up in `plants`, not `opaque`.
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(3, 4, 7, block::TALL_GRASS);
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert_eq!(
            meshes.plants.len(),
            1,
            "TALL_GRASS at (3,4,7) should produce one plant instance",
        );
    }

    #[test]
    fn plants_are_instanced_not_baked() {
        // 2026-05-30 lag fix ("too many flowers = lots of lag"): plant blocks
        // become 32-byte instances drawn against a shared unit cross, instead
        // of baking 8 verts each into the chunk mesh. The instance carries the
        // block's world-min corner, its AABB size, texture layer and light.
        // Volumetric non-solids (torch, smoke) stay baked cubes.
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(5, 6, 5, block::TALL_GRASS);
        let m = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(m.opaque.vertices.is_empty(), "plant must not bake into opaque mesh");
        assert_eq!(m.plants.len(), 1, "one plant instance expected");
        let p = m.plants[0];
        let (min, max) = non_solid_shape_for(block::TALL_GRASS).unwrap();
        assert_eq!(p.pos, [5.0 + min[0], 6.0 + min[1], 5.0 + min[2]], "instance world-min corner");
        assert_eq!(
            p.size,
            [max[0] - min[0], max[1] - min[1], max[2] - min[2]],
            "instance size = AABB extent",
        );

        let (mut world2, registry2) = empty_world_with_chunk();
        world2.set_block(5, 5, 5, block::TORCH);
        let mt = build_chunk_meshes(0, 0, 0, &world2, &registry2);
        assert_eq!(mt.opaque.vertices.len(), 24, "torch stays a 6-faced baked cube");
        assert!(mt.plants.is_empty(), "torch is not a plant instance");
    }

    #[test]
    fn connected_rail_geometry_scales_with_connections() {
        // Straight (N+S) draws bars; a corner (N+E) draws a curved ribbon
        // (tessellated finer than a straight bar for a smooth bend); a cross
        // (N+S+E+W) draws more bar geometry than a straight. All non-empty.
        use crate::block_shape::{CONN_E, CONN_N, CONN_S, CONN_W};
        let count = |mask: u8, offsets: &[f32]| {
            let (mut v, mut i) = (Vec::new(), Vec::new());
            super::emit_connected_rail(0.0, 0.0, 0.0, mask, offsets, 0.045, 0, None, 1.0, &mut v, &mut i);
            v.len()
        };
        let straight = count(CONN_N | CONN_S, &[-0.2, 0.2]);
        let corner = count(CONN_N | CONN_E, &[-0.2, 0.2]);
        let cross = count(CONN_N | CONN_S | CONN_E | CONN_W, &[-0.2, 0.2]);
        assert!(straight > 0 && corner > 0 && cross > 0, "all shapes emit geometry");
        assert!(corner > straight, "curved corner has more quads than a straight");
        assert!(cross > straight, "cross draws more bar geometry than a straight");
        // T-junction (N+S+E) = the straight through-line PLUS two curved branch
        // arcs, so it has far more geometry than a plain straight.
        let t_junction = count(CONN_N | CONN_S | CONN_E, &[-0.2, 0.2]);
        assert!(t_junction > straight, "T-junction adds curved branch arcs");
        // Cable (single wire) emits fewer verts than track (two rails) for the same shape.
        let cable_straight = count(CONN_N | CONN_S, &[0.0]);
        assert!(cable_straight < straight, "one wire < two rails");
    }

    #[test]
    fn plant_unit_cross_is_two_quads() {
        // The shared instanced geometry: 8 verts / 12 indices (two quads,
        // single-sided — the plant pipeline uses cull_mode None so each shows
        // from both faces). Local coords stay within the unit cube.
        let (verts, indices) = plant_unit_cross();
        assert_eq!(verts.len(), 8);
        assert_eq!(indices.len(), 12);
        for v in &verts {
            for c in v.local {
                assert!((0.0..=1.0).contains(&c), "local coord {c} out of unit cube");
            }
        }
    }

    #[test]
    fn empty_chunk_emits_no_geometry() {
        // Sanity baseline — an all-air chunk produces no opaque mesh.
        let (world, registry) = empty_world_with_chunk();
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(meshes.opaque.vertices.is_empty());
    }

    #[test]
    fn crops_and_plants_emit_geometry() {
        // 2026-05-27 fix — every non-solid transparent block (crops,
        // flowers, fibre plants) must render via the small-cube path, not
        // be silently skipped. Regression guard against the invisible-plant
        // bug that also hid every existing crop.
        for b in [
            block::WHEAT_STAGE_0,
            block::CARROT_STAGE_3,
            block::PAPYRUS_STAGE_3,
            block::BERRY_BUSH_3,
            block::CORNFLOWER,
            block::COTTON_PLANT,
            block::COTTON_STAGE_2,
            block::HEMP_STAGE_3,
        ] {
            let (mut world, registry) = empty_world_with_chunk();
            world.set_block(5, 5, 5, b);
            let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
            assert!(
                !meshes.plants.is_empty(),
                "block {b} should render as a plant instance, got none",
            );
        }
    }

    #[test]
    fn wild_flowers_have_compact_shape() {
        // 2026-05-28 — wild flowers (CORNFLOWER/FIELD_POPPY/BUTTERCUP)
        // were falling through `non_solid_shape_for` to the catch-all
        // default `([0.12, 0.0, 0.12], [0.88, 1.0, 0.88])` — a 0.76×1.0×0.76
        // cube *wider and taller* than TALL_GRASS. Playtest feedback
        // (2026-05-28): "the flowers are too big." This test pins the
        // bespoke flower AABB: clearly narrower in X/Z and shorter in Y
        // than tall grass, so they read as individual stems rather than
        // chunky cubes.
        let (_, tg_max) = non_solid_shape_for(block::TALL_GRASS).unwrap();
        let (tg_min, _) = non_solid_shape_for(block::TALL_GRASS).unwrap();
        let tg_height = tg_max[1] - tg_min[1];
        let tg_xz = tg_max[0] - tg_min[0];

        for plant in [block::CORNFLOWER, block::FIELD_POPPY, block::BUTTERCUP] {
            let (min, max) = non_solid_shape_for(plant)
                .unwrap_or_else(|| panic!("plant {plant} missing shape"));
            let height = max[1] - min[1];
            let xz = max[0] - min[0];
            assert!(
                xz < tg_xz,
                "plant {plant} XZ footprint {xz} must be narrower than \
                 TALL_GRASS {tg_xz} (flowers are stems, not bushes)",
            );
            assert!(
                height < tg_height,
                "plant {plant} height {height} must be shorter than \
                 TALL_GRASS {tg_height} (flowers are low to the ground)",
            );
            // Sanity — still rooted at the ground (min Y = 0).
            assert_eq!(
                min[1], 0.0,
                "plant {plant} must sit on the floor of the cell",
            );
            // Sanity — square footprint, centred (min X == min Z, max X == max Z).
            assert_eq!(min[0], min[2], "plant {plant} XZ footprint not square");
            assert_eq!(max[0], max[2], "plant {plant} XZ footprint not square");
        }
    }

    /// Spec 22 Phase 7 — helper that returns true iff any vertex in
    /// the supplied mesh references the warning-tinted smoke layer.
    /// Used by the red-shift tests to confirm the per-cell texture
    /// pick actually changed.
    fn mesh_uses_warning_smoke(mesh: &ChunkMesh) -> bool {
        mesh.vertices
            .iter()
            .any(|v| v.tex_layer == block::TEX_CAMPFIRE_SMOKE_WARNING)
    }

    /// Spec 22 Phase 7 — same idea for the grey-smoke layer.
    fn mesh_uses_normal_smoke(mesh: &ChunkMesh) -> bool {
        mesh.vertices
            .iter()
            .any(|v| v.tex_layer == block::TEX_CAMPFIRE_SMOKE)
    }

    /// Set up a campfire at (cx, cy, cz) with a single smoke cell
    /// directly above. Useful for the red-shift unit tests.
    fn world_with_smoke_pillar(
        cx: i32,
        cy: i32,
        cz: i32,
        warning: bool,
    ) -> (World, BlockRegistry) {
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(cx, cy, cz, block::CAMPFIRE);
        let mut cf = crate::campfire::CampfireData::default();
        cf.raid_warning_active = warning;
        world.insert_campfire((cx, cy, cz), cf);
        world.set_block(cx, cy + 1, cz, block::CAMPFIRE_SMOKE);
        (world, registry)
    }

    #[test]
    fn smoke_above_warning_campfire_emits_warning_layer() {
        // Spec 22 Phase 7 — when the source campfire has
        // raid_warning_active = true, the smoke cell's mesh face uses
        // TEX_CAMPFIRE_SMOKE_WARNING, not the grey layer.
        let (world, registry) = world_with_smoke_pillar(5, 5, 5, true);
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            mesh_uses_warning_smoke(&meshes.opaque),
            "smoke above a warning campfire must use the warning texture layer ({})",
            block::TEX_CAMPFIRE_SMOKE_WARNING,
        );
        assert!(
            !mesh_uses_normal_smoke(&meshes.opaque),
            "warning state must NOT also emit grey-smoke faces (would dilute the signal)",
        );
    }

    #[test]
    fn smoke_above_normal_campfire_emits_grey_layer() {
        // Spec 22 Phase 7 — baseline: a campfire without the warning
        // flag produces the regular grey smoke.
        let (world, registry) = world_with_smoke_pillar(5, 5, 5, false);
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            mesh_uses_normal_smoke(&meshes.opaque),
            "smoke above a non-warning campfire must use the grey texture layer ({})",
            block::TEX_CAMPFIRE_SMOKE,
        );
        assert!(
            !mesh_uses_warning_smoke(&meshes.opaque),
            "non-warning state must NOT emit red-tinted faces",
        );
    }

    #[test]
    fn orphan_smoke_cell_falls_back_to_grey() {
        // Spec 22 Phase 7 — defensive case. An orphan smoke cell with
        // no campfire underneath (e.g. raced cleanup) must not panic
        // and must default to the normal grey layer.
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(5, 5, 5, block::CAMPFIRE_SMOKE);
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            mesh_uses_normal_smoke(&meshes.opaque),
            "orphan smoke cell must emit grey-layer faces, got {:?}",
            meshes
                .opaque
                .vertices
                .iter()
                .map(|v| v.tex_layer)
                .collect::<Vec<_>>(),
        );
        assert!(
            !mesh_uses_warning_smoke(&meshes.opaque),
            "orphan smoke cell must NOT default to the warning layer",
        );
    }

    #[test]
    fn smoke_walk_terminates_above_pillar_height() {
        // Spec 22 Phase 7 — a smoke cell more than SMOKE_PILLAR_HEIGHT
        // cells above any campfire shouldn't find the campfire (walk
        // is bounded) and falls back to grey. Guards against a stray
        // floating smoke block adopting a far-below campfire's flag.
        let (mut world, registry) = empty_world_with_chunk();
        // Warning-active campfire well below the smoke cell.
        world.set_block(5, 1, 5, block::CAMPFIRE);
        let mut cf = crate::campfire::CampfireData::default();
        cf.raid_warning_active = true;
        world.insert_campfire((5, 1, 5), cf);
        // Smoke cell SMOKE_PILLAR_HEIGHT + 2 cells above the campfire,
        // with air in between (so the column-walk hits AIR and bails
        // before reaching the campfire).
        let smoke_y = 1 + crate::campfire::SMOKE_PILLAR_HEIGHT + 2;
        world.set_block(5, smoke_y, 5, block::CAMPFIRE_SMOKE);
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(
            mesh_uses_normal_smoke(&meshes.opaque),
            "far-floating smoke must NOT pick up a distant campfire's warning flag",
        );
        assert!(!mesh_uses_warning_smoke(&meshes.opaque));
    }

    #[test]
    fn developed_blueprint_attachment_emits_cyanotype_decal() {
        // Task C1 — a developed blueprint face-attachment must emit a decal quad
        // using TEX_CYANOTYPE_PRINT (the blueprint-blue texture layer).
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(0, 0, 0, block::DIRT);
        let mut plan = crate::plan::PlanData::debug_3x3_stone();
        plan.develop_state = crate::plan::DevelopState::Developed;
        world.set_face_attachment(
            (0, 0, 0),
            Face::Top.index(),
            crate::world::FaceAttachment::Blueprint(Box::new(plan)),
        );
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(!meshes.decals.is_empty(), "developed blueprint emits a decal");
        assert!(
            meshes.decals.iter().all(|v| v.tex_layer == block::TEX_CYANOTYPE_PRINT),
            "developed blueprint decal uses the cyanotype-blue texture layer",
        );
    }

    #[test]
    fn latent_blueprint_attachment_emits_pale_decal() {
        // Task C1 — a latent (still-developing) blueprint face-attachment must emit
        // a decal quad using TEX_BLUEPRINT_PAPER_TOP (the pale parchment layer).
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(0, 0, 0, block::DIRT);
        let mut plan = crate::plan::PlanData::debug_3x3_stone();
        plan.develop_state = crate::plan::DevelopState::Latent { exposure_ticks: 0 };
        world.set_face_attachment(
            (0, 0, 0),
            Face::Top.index(),
            crate::world::FaceAttachment::Blueprint(Box::new(plan)),
        );
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(!meshes.decals.is_empty(), "latent blueprint emits a decal");
        assert!(
            meshes.decals.iter().all(|v| v.tex_layer == block::TEX_BLUEPRINT_PAPER_TOP),
            "latent blueprint decal uses the pale blueprint-paper texture layer",
        );
    }

    #[test]
    fn blank_blueprint_attachment_emits_cream_decal() {
        // Task R1 — a laid blank cream draughting-paper face-attachment (pre-capture,
        // no plan yet) must emit a decal quad using TEX_BLUEPRINT_PAPER_TOP (cream).
        let (mut world, registry) = empty_world_with_chunk();
        world.set_block(0, 0, 0, block::DIRT);
        world.set_face_attachment(
            (0, 0, 0),
            Face::Top.index(),
            crate::world::FaceAttachment::BlueprintBlank,
        );
        let meshes = build_chunk_meshes(0, 0, 0, &world, &registry);
        assert!(!meshes.decals.is_empty(), "blank draughting paper emits a decal");
        assert!(
            meshes.decals.iter().all(|v| v.tex_layer == block::TEX_BLUEPRINT_PAPER_TOP),
            "blank draughting-paper decal uses the cream blueprint-paper texture layer",
        );
    }
}

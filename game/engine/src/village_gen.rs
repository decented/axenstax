//! Spec 19 phase 4 — procedural village placement.
//!
//! A village is a cluster of 3-6 small houses + a lit campfire + a well,
//! dropped onto Plains or Forest terrain. Each village's existence + layout
//! is a pure function of (world_seed, chunk_x, chunk_z) so save/load + replay
//! give bit-identical placements without ever serialising the layout.
//!
//! Hooked into `world::generate_column` as a third decoration pass after
//! terrain + trees. The structure-builder is column-aware: when a column is
//! generated, every nearby anchor whose footprint overlaps it gets a partial
//! build for the overlapping blocks only. Means villages straddle chunk
//! boundaries correctly without needing a global pre-pass.
//!
//! **Density**: ~1 village per 32×32 chunks (`VILLAGE_GRID` below).
//! Anchors live on a virtual grid; only one anchor per grid cell, deterministic.

use std::collections::BTreeMap;

use crate::biome::{Biome, BiomeGenerator};
use crate::block;
use glam::Vec3;

use crate::chunk::CHUNK_SIZE;
use crate::plan_registry::{PlanCategory, PlanRegistry, SlotDims};
use crate::villager::Profession;
use crate::world::World;

/// Grid cell size in chunks — one village per cell, ~512 blocks apart.
pub const VILLAGE_GRID: i32 = 32;

/// Radius (chunks) around a column from which an anchor's footprint can
/// reach in. Houses extend at most ~20 blocks from the anchor (3-6 houses
/// + well + campfire); 2 chunks of 16 blocks = 32 block reach.
pub const VILLAGE_CHUNK_REACH: i32 = 2;

/// Houses per village. Picked deterministically per anchor.
pub const HOUSES_MIN: u32 = 3;
pub const HOUSES_MAX: u32 = 6;

#[derive(Clone, Debug)]
pub struct VillageLayout {
    pub anchor_world: [i32; 3],
    pub houses: Vec<HouseSpec>,
    pub well: [i32; 3],
    pub campfire: [i32; 3],
    /// Spec 27 Phase 7 — one workshop slot per claimed profession.
    /// Placed on a wider ring than the houses so the village reads as
    /// "homes nearer the centre, workshops on the outside".
    pub workshops: Vec<WorkshopSpec>,
}

#[derive(Clone, Debug)]
pub struct HouseSpec {
    /// Origin = bottom-NW corner of the 3×3 footprint at floor level.
    pub origin: [i32; 3],
    /// Cardinal direction the door faces. 0=North, 1=East, 2=South, 3=West.
    pub door_facing: u8,
}

/// Spec 27 Phase 7 — workshop placement record. One per claimed
/// profession in the village.
#[derive(Clone, Debug)]
pub struct WorkshopSpec {
    pub origin: [i32; 3],
    pub profession: Profession,
}

/// Deterministic 32-bit hash of the (world_seed, cell_x, cell_z) triple.
/// Mixed enough that adjacent cells don't share patterns.
fn anchor_hash(world_seed: u32, gx: i32, gz: i32) -> u32 {
    let mut h = world_seed.wrapping_mul(0x9E3779B1);
    h ^= (gx as u32).wrapping_mul(0x85EBCA77);
    h = h.rotate_left(13).wrapping_mul(0xC2B2AE3D);
    h ^= (gz as u32).wrapping_mul(0x27D4EB2F);
    h = h.rotate_left(7).wrapping_mul(0x165667B1);
    h ^ (h >> 16)
}

/// Should the grid cell `(gx, gz)` host a village? About 80% of cells do —
/// the other 20% are intentional empty stretches so the world doesn't feel
/// villager-saturated. (Will be tuned post-Axolittle-playtest.)
fn cell_has_village(world_seed: u32, gx: i32, gz: i32) -> bool {
    !anchor_hash(world_seed, gx, gz).is_multiple_of(5)
}

/// World-space block position the anchor lives at — offset *within* the
/// grid cell by a deterministic amount so two adjacent cells don't put their
/// anchors on a perfect grid.
fn anchor_world_xz(world_seed: u32, gx: i32, gz: i32) -> (i32, i32) {
    let h = anchor_hash(world_seed, gx, gz);
    // Cell origin in blocks.
    let cell_origin_x = gx * VILLAGE_GRID * CHUNK_SIZE as i32;
    let cell_origin_z = gz * VILLAGE_GRID * CHUNK_SIZE as i32;
    // Random offset within the cell, kept away from the cell edges so the
    // village doesn't clip the boundary.
    let cell_blocks = VILLAGE_GRID * CHUNK_SIZE as i32;
    let margin = 40; // keep 40 blocks from the cell edge
    let span = cell_blocks - 2 * margin;
    let off_x = (h % span as u32) as i32 + margin;
    let off_z = ((h >> 12) % span as u32) as i32 + margin;
    (cell_origin_x + off_x, cell_origin_z + off_z)
}

/// The village site of a grid cell — its anchor `[x, surface_y, z]` — or
/// `None` if the cell has no village. This is the whole "does a village exist
/// here" decision (cell roll, biome gate, sea-level gate) and nothing else.
/// Pure: seed + biome sampling only, never world state, so anything that must
/// know where villages are (the Brigand Hideout village-distance gate) asks
/// this instead of `World::village_anchors`, which only fills as village
/// columns generate (Phase B0 worldgen purity).
pub fn village_site(
    world_seed: u32,
    gx: i32,
    gz: i32,
    biome_gen: &BiomeGenerator,
) -> Option<[i32; 3]> {
    if !cell_has_village(world_seed, gx, gz) {
        return None;
    }
    let (ax, az) = anchor_world_xz(world_seed, gx, gz);

    // Biome gate — villages only on Plains or Forest. Reject early.
    let biome = biome_gen.biome_at(ax, az);
    if !matches!(biome, Biome::Plains | Biome::Forest) {
        return None;
    }

    // Bugfix 2026-05-21 — reject villages whose anchor sits at or
    // below sea level, otherwise the village renders as a flooded
    // pit when the player spawns one near a lake. The 4× lake-depth
    // boost shipped the same day made this case very visible.
    let surface = biome_gen.terrain_height(ax, az);
    if surface < crate::biome::SEA_LEVEL + 2 {
        return None;
    }
    Some([ax, surface, az])
}

/// Is there a village anchor strictly closer than `dist` blocks (Chebyshev,
/// XZ) to `(x, z)`? Scans every village grid cell an anchor that close could
/// sit in, via [`village_site`]. Pure, so the answer does not depend on which
/// columns have been generated or in what order.
pub fn village_site_within(
    world_seed: u32,
    x: i32,
    z: i32,
    dist: i32,
    biome_gen: &BiomeGenerator,
) -> bool {
    let cell = VILLAGE_GRID * CHUNK_SIZE as i32;
    let r = dist - 1;
    for gx in (x - r).div_euclid(cell)..=(x + r).div_euclid(cell) {
        for gz in (z - r).div_euclid(cell)..=(z + r).div_euclid(cell) {
            if let Some([vx, _, vz]) = village_site(world_seed, gx, gz, biome_gen)
                && (vx - x).abs().max((vz - z).abs()) < dist
            {
                return true;
            }
        }
    }
    false
}

/// Compute the full village layout for a cell, or `None` if this cell has no
/// village. Pure — no world reads, no allocation outside the returned `Vec`.
///
/// Ring positions use `libm::cosf`/`sinf`, not the platform `f32::cos`/`sin`:
/// the result is `.round()`ed to a cell, and platform libms (glibc, Android,
/// macOS, the WASM build) may differ by an ULP, enough to flip a cell between a
/// host and a joiner (Phase B0).
pub fn layout_for_cell(
    world_seed: u32,
    gx: i32,
    gz: i32,
    biome_gen: &BiomeGenerator,
) -> Option<VillageLayout> {
    let [ax, surface, az] = village_site(world_seed, gx, gz, biome_gen)?;
    let h = anchor_hash(world_seed, gx, gz);

    // House count: 3-6 inclusive.
    let n_houses = HOUSES_MIN + (h.rotate_left(3) % (HOUSES_MAX - HOUSES_MIN + 1));
    let mut houses = Vec::with_capacity(n_houses as usize);

    // Lay houses on a coarse ring around the anchor at distinct angles. Each
    // house gets a deterministic angular offset + radial jitter.
    for i in 0..n_houses {
        let h_i = h.wrapping_mul(73856093).wrapping_add(i.wrapping_mul(83492791));
        // Angle slice (i / n) * 2π, plus a small randomised offset so houses
        // don't form a perfect ring.
        let angle_base = (i as f32) / (n_houses as f32) * std::f32::consts::TAU;
        let angle_jitter = (h_i % 30) as f32 / 100.0; // up to 0.3 rad ≈ 17°
        let angle = angle_base + angle_jitter;
        // Radius: 8-14 blocks from the anchor.
        let radius = 8.0 + (h_i.rotate_left(5) % 7) as f32;
        let dx = (radius * libm::cosf(angle)).round() as i32;
        let dz = (radius * libm::sinf(angle)).round() as i32;
        let hx = ax + dx;
        let hz = az + dz;
        // House surface independently — terrain dips don't all match anchor.
        let h_surface = biome_gen.terrain_height(hx, hz);
        // Door faces inward (back toward the anchor).
        let door_facing = inward_facing(dx, dz);
        houses.push(HouseSpec {
            origin: [hx, h_surface, hz],
            door_facing,
        });
    }

    // Spec 27 Phase 7 — workshop slots. One per profession in the
    // alpha set, placed on a wider ring than the houses so the
    // village reads as "homes inside, workshops outside". The count
    // = min(n_houses, 5) so small hamlets only get a couple, while
    // bigger villages cover the full profession set.
    let alpha_professions = [
        Profession::Farmer,
        Profession::Cook,
        Profession::Carpenter,
        Profession::Blacksmith,
        Profession::Scribe,
        // Historical Pivot Sub 7 (2026-05-23) — T1.5 trades. Workshop
        // slots get a Mill / Oven / Aging Rack via the workstation
        // table below so the corresponding villagers can claim them
        // and read as Miller / Baker / Brewer.
        Profession::Miller,
        Profession::Baker,
        Profession::Brewer,
    ];
    let n_workshops = (n_houses as usize).min(alpha_professions.len());
    let mut workshops = Vec::with_capacity(n_workshops);
    // Rotate the profession order per village so adjacent villages
    // don't all lead with Farmer.
    let prof_offset = (h.rotate_left(11) as usize) % alpha_professions.len();
    for i in 0..n_workshops {
        let prof = alpha_professions[(prof_offset + i) % alpha_professions.len()];
        let h_i = h.wrapping_mul(40503).wrapping_add((i as u32).wrapping_mul(6151));
        let angle = (i as f32 / n_workshops as f32) * std::f32::consts::TAU
            + (h_i % 30) as f32 / 100.0;
        // Workshops sit ~18-22 blocks out — outside the house ring.
        let radius = 18.0 + (h_i.rotate_left(3) % 5) as f32;
        let dx = (radius * libm::cosf(angle)).round() as i32;
        let dz = (radius * libm::sinf(angle)).round() as i32;
        let wx = ax + dx;
        let wz = az + dz;
        let w_surface = biome_gen.terrain_height(wx, wz);
        workshops.push(WorkshopSpec {
            origin: [wx, w_surface, wz],
            profession: prof,
        });
    }

    Some(VillageLayout {
        anchor_world: [ax, surface, az],
        houses,
        // Well + campfire flank the anchor on opposite sides.
        well: [ax - 3, surface, az],
        campfire: [ax + 3, surface, az],
        workshops,
    })
}

fn inward_facing(dx: i32, dz: i32) -> u8 {
    // Pick the cardinal direction that points back toward the anchor.
    if dx.abs() > dz.abs() {
        if dx > 0 { 3 } else { 1 } // door on the inward side
    } else {
        if dz > 0 { 0 } else { 2 }
    }
}

/// Decoration pass — called from `world::generate_column` after trees.
/// Places any village blocks that fall inside this column.
///
/// Returns the set of village anchors whose blocks were touched this call so
/// the caller can register them in a side-table for later villager-spawning.
///
/// Houses and workshops sample [`PlanRegistry::bundled`], never
/// `World::plan_registry`: the world's registry also holds runtime additions
/// (`/importschem`), which must not change what a seed generates (Phase B0).
/// The bundled registry's content is folded into `world::worldgen_fingerprint`.
pub fn place_villages_for_column(
    world: &mut World,
    cx: i32,
    cz: i32,
    biome_gen: &BiomeGenerator,
    world_seed: u32,
    placed: &mut BTreeMap<(i32, i32), [i32; 3]>,
) {
    place_villages_for_column_with_plans(
        world, cx, cz, biome_gen, world_seed, placed, PlanRegistry::bundled(),
    );
}

/// [`place_villages_for_column`] with an explicit plan registry. Generation
/// always passes the bundled one; tests pass an empty registry to exercise the
/// hardcoded fallback shapes.
pub(crate) fn place_villages_for_column_with_plans(
    world: &mut World,
    cx: i32,
    cz: i32,
    biome_gen: &BiomeGenerator,
    world_seed: u32,
    placed: &mut BTreeMap<(i32, i32), [i32; 3]>,
    plans: &PlanRegistry,
) {
    let cs = CHUNK_SIZE as i32;
    let col_min_x = cx * cs;
    let col_min_z = cz * cs;
    let col_max_x = col_min_x + cs - 1;
    let col_max_z = col_min_z + cs - 1;

    // Find the grid cell this column is in, then check this + neighbouring
    // cells whose villages could reach into us.
    let gx_centre = cx.div_euclid(VILLAGE_GRID);
    let gz_centre = cz.div_euclid(VILLAGE_GRID);
    for dgz in -1..=1 {
        for dgx in -1..=1 {
            let gx = gx_centre + dgx;
            let gz = gz_centre + dgz;
            let Some(layout) = layout_for_cell(world_seed, gx, gz, biome_gen) else {
                continue;
            };
            // Anchor world reach — early reject if even the farthest block
            // can't touch this column.
            let ax = layout.anchor_world[0];
            let az = layout.anchor_world[2];
            let reach = (VILLAGE_CHUNK_REACH + 1) * cs;
            if (ax + reach) < col_min_x || (ax - reach) > col_max_x
                || (az + reach) < col_min_z || (az - reach) > col_max_z
            {
                continue;
            }
            // Seed the registry sampler from the village's anchor
            // hash so each village's house variants are stable
            // across loads.
            let village_seed = (anchor_hash(world_seed, gx, gz) as u64).wrapping_mul(0x9E3779B97F4A7C15);
            apply_layout_to_column(
                world,
                &layout,
                col_min_x, col_min_z, col_max_x, col_max_z,
                village_seed,
                plans,
            );
            placed.insert((gx, gz), layout.anchor_world);
        }
    }
}

fn apply_layout_to_column(
    world: &mut World,
    layout: &VillageLayout,
    col_min_x: i32,
    col_min_z: i32,
    col_max_x: i32,
    col_max_z: i32,
    village_seed: u64,
    plans: &PlanRegistry,
) {
    let bounds = ColumnBounds {
        min_x: col_min_x, min_z: col_min_z, max_x: col_max_x, max_z: col_max_z,
    };

    // Houses. Per-house seed derived from village seed + index so a
    // village's house variants are stable but distinct from each
    // other.
    for (idx, house) in layout.houses.iter().enumerate() {
        let seed = village_seed.wrapping_mul(31).wrapping_add(idx as u64);
        build_house(world, house, &bounds, seed, plans);
    }
    // Workshops — Spec 27 Phase 7.
    for (idx, ws) in layout.workshops.iter().enumerate() {
        let seed = village_seed.wrapping_mul(37).wrapping_add(idx as u64 + 1000);
        build_workshop(world, ws, &bounds, seed, plans);
    }
    // Well.
    build_well(world, layout.well, &bounds);
    // Lit campfire on a cobblestone base — Spec 17 says lit campfire id 43.
    build_campfire(world, layout.campfire, &bounds);
}

/// Helper: column-XZ bounds for the partial-build pass. Builders use
/// `try_set` to skip writes outside the active column without having
/// to reason about chunk math at each call.
#[derive(Clone, Copy)]
struct ColumnBounds {
    min_x: i32,
    min_z: i32,
    max_x: i32,
    max_z: i32,
}

impl ColumnBounds {
    fn contains(&self, x: i32, z: i32) -> bool {
        x >= self.min_x && x <= self.max_x && z >= self.min_z && z <= self.max_z
    }
}

fn try_set(world: &mut World, bounds: &ColumnBounds, x: i32, y: i32, z: i32, b: u16) {
    if bounds.contains(x, z) {
        world.set_block(x, y, z, b);
    }
}

/// Spec 27 Phase 6 — try the plan registry first. On a hit, instantiate
/// the sampled plan + drop the procgen Plaque next to the building. On
/// a miss (empty registry, no matching plan), fall back to the legacy
/// hardcoded shape so worlds without bundled content still build.
fn build_house(
    world: &mut World,
    house: &HouseSpec,
    bounds: &ColumnBounds,
    seed: u64,
    plans: &PlanRegistry,
) {
    let [ox, oy, oz] = house.origin;
    let slot = SlotDims { width: 8, depth: 8 };
    if let Some(plan) = plans.sample_plan_for_slot(PlanCategory::SmallHouse, slot, seed) {
        place_plan_via_procgen(world, plan, ox, oy, oz, bounds);
        return;
    }
    build_hardcoded_house(world, house, bounds);
}

/// Spec 27 Phase 7 — sample a Workshop plan for the workshop's
/// profession. Falls back to a small hardcoded shape (just the
/// distinguishing workstation block on a floor patch) when the
/// registry can't supply a fit.
fn build_workshop(
    world: &mut World,
    ws: &WorkshopSpec,
    bounds: &ColumnBounds,
    seed: u64,
    plans: &PlanRegistry,
) {
    let [ox, oy, oz] = ws.origin;
    let slot = SlotDims { width: 10, depth: 10 };
    if let Some(plan) = plans.sample_plan_for_slot(PlanCategory::Workshop(ws.profession), slot, seed) {
        place_plan_via_procgen(world, plan, ox, oy, oz, bounds);
        return;
    }
    build_hardcoded_workshop(world, ws, bounds);
}

/// Spec 27 Phase 6 + 8 — instantiate a sampled plan at the given
/// anchor + drop an Architect's Plaque tagged as procgen-sourced. The
/// plan's cells go in via `try_set` (column-filter aware); the Plaque
/// goes in via `try_set` too so a plan that straddles a chunk
/// boundary still places its plaque on the column-pass that contains
/// the plaque cell. The plaque metadata write happens unconditionally
/// — `World::architect_plaques` is keyed by world position, not by
/// chunk, so duplicating it across column passes is idempotent.
fn place_plan_via_procgen(
    world: &mut World,
    plan: &crate::plan::PlanData,
    ox: i32,
    oy: i32,
    oz: i32,
    bounds: &ColumnBounds,
) {
    // Plan cells sit on top of the foundation, so the floor (ry=0)
    // lands at y=oy and the rest climbs up.
    for c in &plan.cells {
        let wx = ox + c.rx as i32;
        let wy = oy + c.ry as i32;
        let wz = oz + c.rz as i32;
        try_set(world, bounds, wx, wy, wz, c.block_id);
    }
    // Architect Plaque at the building's NW corner on the floor's
    // outside (oz - 1) so it doesn't overlap a cell. Falls back to
    // the centre if the outside is out-of-bounds.
    let plaque_x = ox;
    let plaque_y = oy + 1;
    let plaque_z = oz.saturating_sub(1);
    try_set(world, bounds, plaque_x, plaque_y, plaque_z, block::ARCHITECT_PLAQUE);
    world.architect_plaques.insert(
        (plaque_x, plaque_y, plaque_z),
        crate::plan::ArchitectPlaqueData::from_plan(plan),
    );
    world.procgen_plaque_sources.insert((plaque_x, plaque_y, plaque_z));
}

/// Legacy hardcoded house shape — the fallback when the registry has no
/// plan for the slot (generation passes the bundled registry, so in
/// practice only tests passing an empty one reach it). Removed once Spec
/// 27 Phase 11 playtest signs off on the registry-only path.
fn build_hardcoded_house(world: &mut World, house: &HouseSpec, bounds: &ColumnBounds) {
    let [ox, oy, oz] = house.origin;
    let w: i32 = 3;
    let d: i32 = 3;
    let h: i32 = 4;
    for dz in 0..d {
        for dx in 0..w {
            try_set(world, bounds, ox + dx, oy, oz + dz, block::OAK_PLANKS);
        }
    }
    for dy in 1..(h - 1) {
        for dz in 0..d {
            for dx in 0..w {
                let on_edge = dx == 0 || dx == w - 1 || dz == 0 || dz == d - 1;
                if !on_edge {
                    continue;
                }
                try_set(world, bounds, ox + dx, oy + dy, oz + dz, block::COBBLESTONE);
            }
        }
    }
    for dz in 0..d {
        for dx in 0..w {
            try_set(world, bounds, ox + dx, oy + h - 1, oz + dz, block::OAK_PLANKS);
        }
    }
    let (door_dx, door_dz) = match house.door_facing {
        0 => (1, 0),
        1 => (w - 1, 1),
        2 => (1, d - 1),
        _ => (0, 1),
    };
    try_set(world, bounds, ox + door_dx, oy + 1, oz + door_dz, block::AIR);
    try_set(world, bounds, ox + door_dx, oy + 2, oz + door_dz, block::AIR);
    let (bed_dx, bed_dz) = match house.door_facing {
        0 => (1, d - 2),
        1 => (1, 1),
        2 => (1, 1),
        _ => (w - 2, 1),
    };
    try_set(world, bounds, ox + bed_dx, oy + 1, oz + bed_dz, block::BED);
    try_set(world, bounds, ox + 1, oy + h - 2, oz + 1, block::TORCH);
}

/// Legacy hardcoded workshop — registry fallback. Just a planks floor
/// + the distinguishing workstation block at the centre, so the
///   villager-claim system still has something to attach to.
fn build_hardcoded_workshop(world: &mut World, ws: &WorkshopSpec, bounds: &ColumnBounds) {
    let [ox, oy, oz] = ws.origin;
    let w: i32 = 3;
    let d: i32 = 3;
    for dz in 0..d {
        for dx in 0..w {
            try_set(world, bounds, ox + dx, oy, oz + dz, block::OAK_PLANKS);
        }
    }
    let workstation = match ws.profession {
        Profession::Farmer => block::TILLED_SOIL,
        Profession::Cook => block::CAMPFIRE,
        Profession::Carpenter => block::CRAFTING_TABLE,
        Profession::Blacksmith => block::FURNACE,
        // Historical Pivot Sub 7 — T1.5 workstation blocks so villagers
        // standing in these workshops claim Miller / Baker / Brewer.
        Profession::Miller => block::MILL,
        Profession::Baker => block::OVEN,
        Profession::Brewer => block::AGING_RACK,
        // Scribe has no distinguishing block yet — drop a torch
        // so the slot reads as a claimed-but-furnitureless room.
        _ => block::TORCH,
    };
    try_set(world, bounds, ox + 1, oy + 1, oz + 1, workstation);
}

fn build_well(world: &mut World, pos: [i32; 3], bounds: &ColumnBounds) {
    let [x, y, z] = pos;
    // 3×3 cobblestone rim around a 1×1 water source. Water sits at surface
    // level so it's flush with the ground; rim sticks up one block.
    for dz in -1..=1 {
        for dx in -1..=1 {
            // The centre block: water source.
            if dx == 0 && dz == 0 {
                try_set(world, bounds, x, y, z, block::WATER);
            } else {
                try_set(world, bounds, x + dx, y, z + dz, block::COBBLESTONE);
            }
        }
    }
    // Rim — 4 cobblestone blocks one level up forming the well wall.
    try_set(world, bounds, x - 1, y + 1, z, block::COBBLESTONE);
    try_set(world, bounds, x + 1, y + 1, z, block::COBBLESTONE);
    try_set(world, bounds, x, y + 1, z - 1, block::COBBLESTONE);
    try_set(world, bounds, x, y + 1, z + 1, block::COBBLESTONE);
}

fn build_campfire(world: &mut World, pos: [i32; 3], bounds: &ColumnBounds) {
    let [x, y, z] = pos;
    // Small cobblestone hearth (cross pattern) + a lit campfire on top.
    try_set(world, bounds, x, y, z, block::COBBLESTONE);
    try_set(world, bounds, x - 1, y, z, block::COBBLESTONE);
    try_set(world, bounds, x + 1, y, z, block::COBBLESTONE);
    try_set(world, bounds, x, y, z - 1, block::COBBLESTONE);
    try_set(world, bounds, x, y, z + 1, block::COBBLESTONE);
    try_set(world, bounds, x, y + 1, z, block::CAMPFIRE);
}

/// Spawn the initial villager cohort for each village whose blocks have been
/// placed but whose villagers haven't yet been instantiated. One villager
/// near each house. Idempotent — adds the cell to `populated_villages` so a
/// second call is a no-op.
///
/// Should be called from the game loop on any cadence that's "more often
/// than the player can leave-and-rejoin a village area" — once per second
/// (every 20 ticks) is plenty.
pub fn spawn_initial_villagers(
    world: &mut World,
    biome_gen: &BiomeGenerator,
    ecs: &mut hecs::World,
) -> u32 {
    let world_seed = biome_gen.seed;
    let mut spawned = 0u32;
    let mut to_mark: Vec<(i32, i32)> = Vec::new();

    // Iterate by key clone so we don't borrow village_anchors while mutating
    // populated_villages below.
    let anchors: Vec<((i32, i32), [i32; 3])> = world
        .village_anchors
        .iter()
        .map(|(&k, &v)| (k, v))
        .collect();

    for ((gx, gz), _anchor) in anchors {
        if world.populated_villages.contains(&(gx, gz)) {
            continue;
        }
        let Some(layout) = layout_for_cell(world_seed, gx, gz, biome_gen) else {
            continue;
        };
        // Anchor chunk must actually be loaded — don't spawn into the void.
        let cs = CHUNK_SIZE as i32;
        let cx = layout.anchor_world[0].div_euclid(cs);
        let cz = layout.anchor_world[2].div_euclid(cs);
        let cy = layout.anchor_world[1].div_euclid(cs);
        if !world.has_chunk(cx, cy, cz) {
            continue;
        }
        // One villager per house, spawned 1 block above the floor of each
        // house so they stand on the planks rather than clipping into them.
        for house in &layout.houses {
            let [hx, hy, hz] = house.origin;
            let pos = glam::Vec3::new(hx as f32 + 1.5, hy as f32 + 1.0, hz as f32 + 1.5);
            crate::entity::spawn_mob(ecs, crate::mob::MobType::Villager, pos);
            spawned += 1;
        }
        to_mark.push((gx, gz));
    }
    for k in to_mark {
        world.populated_villages.insert(k);
    }
    spawned
}

/// HP-4 — auto-spawn Knight village defenders for every village whose
/// population meets the Spec 19 threshold (≥ 3 claimed villagers + ≥ 5
/// houses); per-village cap of `max(1, villagers / 10)`. HP-6 retired the
/// Iron Golem, leaving the Knight as the sole village defender (the fantasy
/// roster was excised entirely in the open-source IP cleanup).
///
/// `tick_counter` throttle is the caller's job — invoke at most once
/// per second (every 20 ticks) per the existing village-tick cadence.
pub fn tick_knight_spawn(
    world: &World,
    biome_gen: &BiomeGenerator,
    ecs: &mut hecs::World,
) -> u32 {
    use crate::entity::{MobKind, Position};
    use crate::mob::MobType;
    use crate::villager::{is_villager_kind, VillagerComponent};

    let world_seed = biome_gen.seed;
    let villager_pos_claims: Vec<(Vec3, bool)> = ecs
        .query::<(&Position, &MobKind, &VillagerComponent)>()
        .iter()
        .filter(|(_, (_, k, _))| is_villager_kind(k.0))
        .map(|(_, (p, _, vc))| (p.0, vc.profession != crate::villager::Profession::None))
        .collect();
    let knight_positions: Vec<Vec3> = ecs
        .query::<(&Position, &MobKind)>()
        .iter()
        .filter(|(_, (_, k))| k.0 == MobType::Knight)
        .map(|(_, (p, _))| p.0)
        .collect();

    let mut spawned = 0u32;
    for (&(gx, gz), &anchor) in &world.village_anchors {
        let Some(layout) = layout_for_cell(world_seed, gx, gz, biome_gen) else {
            continue;
        };
        let n_houses = layout.houses.len() as u32;
        if n_houses < 5 {
            continue;
        }
        let anchor_pos = Vec3::new(
            anchor[0] as f32 + 0.5,
            anchor[1] as f32 + 0.5,
            anchor[2] as f32 + 0.5,
        );
        let claimed_count = villager_pos_claims
            .iter()
            .filter(|(p, claimed)| {
                *claimed && (*p - anchor_pos).length() < 24.0
            })
            .count() as u32;
        if claimed_count < 3 {
            continue;
        }
        let existing = knight_positions
            .iter()
            .filter(|p| (**p - anchor_pos).length() < 32.0)
            .count() as u32;
        let cap = (claimed_count / 10).max(1);
        if existing >= cap {
            continue;
        }
        let spawn_pos = Vec3::new(
            anchor_pos.x + 1.5, // offset a bit off the anchor block
            anchor_pos.y + 1.0,
            anchor_pos.z,
        );
        let id = crate::entity::spawn_mob(ecs, MobType::Knight, spawn_pos);
        if let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(id) {
            ai.state = crate::mob_ai::AiState::GolemGuard {
                home_x: anchor[0],
                home_z: anchor[2],
            };
        }
        spawned += 1;
    }
    spawned
}

/// Spec 19 phase 10 — Village Bell migration tick.
///
/// For each Village Bell in the world: occasionally spawn a Wandering
/// Villager nearby (one at a time); for each Wandering Villager near a bell,
/// steer them toward it; on arrival (< 2.5 blocks) despawn the wanderer +
/// spawn a normal Villager at the bell + register the bell as a village
/// anchor. The player-founded village then benefits from every later phase
/// (workstation claims, quests, iron-golem spawn) on the same code paths
/// procgen villages do.
///
/// `tick` is the engine tick counter — used to throttle the spawn cadence so
/// wanderers don't pile up.
pub fn tick_wanderer_migration(
    world: &mut World,
    ecs: &mut hecs::World,
    tick: u64,
) -> u32 {
    use crate::entity::{MobKind, Position};
    use crate::mob::MobType;

    // Filter to bells that still exist as blocks.
    let bells: Vec<[i32; 3]> = world
        .village_bells
        .iter()
        .copied()
        .filter(|p| world.get_block(p[0], p[1], p[2]) == crate::block::VILLAGE_BELL)
        .collect();
    if bells.is_empty() {
        return 0;
    }

    // Snapshot wanderer positions + ids.
    let wanderers: Vec<(hecs::Entity, Vec3)> = ecs
        .query::<(&Position, &MobKind)>()
        .iter()
        .filter(|(_, (_, k))| k.0 == MobType::Peddler)
        .map(|(id, (p, _))| (id, p.0))
        .collect();

    let mut converted = 0u32;
    let mut to_despawn: Vec<hecs::Entity> = Vec::new();
    let mut to_spawn: Vec<Vec3> = Vec::new();

    for (id, wpos) in &wanderers {
        let mut best: Option<([i32; 3], f32)> = None;
        for b in &bells {
            let bv = Vec3::new(b[0] as f32 + 0.5, b[1] as f32 + 0.5, b[2] as f32 + 0.5);
            let d = (bv - *wpos).length();
            if d <= 64.0 && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((*b, d));
            }
        }
        let Some((bell, dist)) = best else { continue };
        if dist < 2.5 {
            // Convert. Spawn replacement villager just above the bell base.
            to_despawn.push(*id);
            let villager_pos = Vec3::new(
                bell[0] as f32 + 0.5,
                bell[1] as f32 + 1.0,
                bell[2] as f32 + 0.5,
            );
            to_spawn.push(villager_pos);
            // Register the bell as a village anchor (one cell key per bell).
            let cs = CHUNK_SIZE as i32;
            let gx = bell[0].div_euclid(VILLAGE_GRID * cs);
            let gz = bell[2].div_euclid(VILLAGE_GRID * cs);
            world.village_anchors.insert((gx, gz), bell);
            world.populated_villages.insert((gx, gz));
            converted += 1;
        } else {
            // Steer the wanderer's facing toward the bell so the Wander
            // state walks them that way. Exact pathfinding is future polish.
            let bv = Vec3::new(bell[0] as f32 + 0.5, wpos.y, bell[2] as f32 + 0.5);
            let to_bell = bv - *wpos;
            if let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(*id) {
                ai.facing = to_bell.z.atan2(to_bell.x);
            }
        }
    }

    // Apply despawn + spawn after the iteration loop releases its borrows.
    for id in to_despawn {
        let _ = ecs.despawn(id);
    }
    for pos in to_spawn {
        crate::entity::spawn_mob(ecs, MobType::Villager, pos);
    }

    // Throttled wanderer spawn — every 30 s, for any bell without a nearby
    // wanderer, spawn one ~12 blocks out so they can walk in. Skip tick 0
    // so an immediate-after-conversion call doesn't reseed the bell with
    // a fresh wanderer.
    if tick > 0 && tick.is_multiple_of(600) {
        for bell in &bells {
            let bv = Vec3::new(
                bell[0] as f32 + 0.5,
                bell[1] as f32 + 1.0,
                bell[2] as f32 + 0.5,
            );
            let has_nearby = ecs
                .query::<(&Position, &MobKind)>()
                .iter()
                .any(|(_, (p, k))| {
                    k.0 == MobType::Peddler && (p.0 - bv).length() < 32.0
                });
            if !has_nearby {
                let dx = ((tick.wrapping_mul(73856093) % 24) as f32) - 12.0;
                let dz = ((tick.wrapping_mul(83492791) % 24) as f32) - 12.0;
                let spawn_pos = Vec3::new(bv.x + dx, bv.y, bv.z + dz);
                crate::entity::spawn_mob(ecs, MobType::Peddler, spawn_pos);
            }
        }
    }

    converted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::BiomeGenerator;

    fn world_seed() -> u32 { 42 }

    #[test]
    fn anchor_hash_is_deterministic_and_position_dependent() {
        let a = anchor_hash(world_seed(), 0, 0);
        let b = anchor_hash(world_seed(), 0, 0);
        assert_eq!(a, b, "hash must be deterministic");
        let c = anchor_hash(world_seed(), 1, 0);
        assert_ne!(a, c, "neighbouring cells must hash differently");
    }

    #[test]
    fn approximately_80_percent_of_cells_have_villages() {
        let mut hits = 0;
        for gx in 0..100 {
            for gz in 0..100 {
                if cell_has_village(world_seed(), gx, gz) {
                    hits += 1;
                }
            }
        }
        // ~8000 with the (1 in 5 empty) rule; allow ±5% wiggle for hash variance.
        assert!(hits >= 7500 && hits <= 8500, "village density off: {hits}/10000");
    }

    #[test]
    fn layout_only_built_in_plains_or_forest() {
        let bg = BiomeGenerator::new(world_seed());
        // Sample many cells; for every cell with a layout, the biome at the
        // anchor must be Plains or Forest.
        for gx in -10..10 {
            for gz in -10..10 {
                if let Some(layout) = layout_for_cell(world_seed(), gx, gz, &bg) {
                    let [ax, _, az] = layout.anchor_world;
                    let biome = bg.biome_at(ax, az);
                    assert!(
                        matches!(biome, Biome::Plains | Biome::Forest),
                        "village placed in non-grass biome {biome:?} at ({ax},{az})"
                    );
                }
            }
        }
    }

    #[test]
    fn layout_has_three_to_six_houses() {
        let bg = BiomeGenerator::new(world_seed());
        let mut min_h = u32::MAX;
        let mut max_h = 0u32;
        let mut seen = 0;
        for gx in -20..20 {
            for gz in -20..20 {
                if let Some(layout) = layout_for_cell(world_seed(), gx, gz, &bg) {
                    seen += 1;
                    let n = layout.houses.len() as u32;
                    if n < min_h { min_h = n; }
                    if n > max_h { max_h = n; }
                    assert!(n >= HOUSES_MIN && n <= HOUSES_MAX,
                        "house count {n} outside spec range {HOUSES_MIN}..={HOUSES_MAX}");
                }
            }
        }
        assert!(seen > 50, "test bailed without enough samples");
        assert_eq!(min_h, HOUSES_MIN, "expected at least one min-house village");
        assert_eq!(max_h, HOUSES_MAX, "expected at least one max-house village");
    }

    #[test]
    fn layout_anchor_well_and_campfire_are_distinct() {
        let bg = BiomeGenerator::new(world_seed());
        for gx in -10..10 {
            for gz in -10..10 {
                if let Some(layout) = layout_for_cell(world_seed(), gx, gz, &bg) {
                    assert_ne!(layout.well, layout.campfire,
                        "well and campfire must not share a tile");
                    assert_ne!(layout.well, layout.anchor_world);
                    assert_ne!(layout.campfire, layout.anchor_world);
                }
            }
        }
    }

    #[test]
    fn place_villages_writes_at_least_campfire_when_anchor_is_in_column() {
        let bg = BiomeGenerator::new(world_seed());
        // Find an actual village.
        let mut found: Option<VillageLayout> = None;
        'search: for gx in -10..10 {
            for gz in -10..10 {
                if let Some(layout) = layout_for_cell(world_seed(), gx, gz, &bg) {
                    found = Some(layout);
                    break 'search;
                }
            }
        }
        let layout = found.expect("expected at least one village within the sample range");

        let mut world = World::new();
        let [ax, _ay, az] = layout.anchor_world;
        let cx = ax.div_euclid(CHUNK_SIZE as i32);
        let cz = az.div_euclid(CHUNK_SIZE as i32);
        // Generate the centre column + neighbours so the village footprint lands.
        for dcx in -3..=3 {
            for dcz in -3..=3 {
                world.generate_column(cx + dcx, cz + dcz, &bg);
            }
        }

        // Re-run the village pass via the standalone API too (idempotency check).
        let mut placed: BTreeMap<(i32, i32), [i32; 3]> = BTreeMap::new();
        for dcx in -3..=3 {
            for dcz in -3..=3 {
                place_villages_for_column(
                    &mut world, cx + dcx, cz + dcz, &bg, world_seed(), &mut placed,
                );
            }
        }
        // The campfire block (id 43) should exist exactly where the layout
        // said it would.
        let [fx, fy, fz] = layout.campfire;
        assert_eq!(
            world.get_block(fx, fy + 1, fz),
            block::CAMPFIRE,
            "expected lit campfire at layout-specified position"
        );
    }
}

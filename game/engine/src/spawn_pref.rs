//! Spawn-preference — applied when a world loads (new or existing).
//!
//! `Default` preserves the current behaviour (saved position for existing
//! worlds, the engine's world-spawn point for new worlds). `NearVillage`
//! scans the deterministic village layout from `village_gen` and teleports
//! the player to the nearest village anchor. `AtOrigin` drops them at
//! `(0.5, surface, 0.5)` — useful for debugging.
//!
//! Lives in its own module so the menu UI + chunk-load path can both depend
//! on it without circular imports.

use glam::Vec3;

use crate::biome::{Biome, BiomeGenerator, SEA_LEVEL};
use crate::chunk::CHUNK_SIZE;
use crate::village_gen;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
pub enum SpawnPref {
    /// Use the saved player position (existing world) or the engine's
    /// default spawn point (new world). No override applied.
    #[default]
    Default,
    /// Teleport to the nearest village anchor.
    NearVillage,
    /// Teleport to a wilderness cell — one whose own grid cell has no
    /// village (neighbour cells may; with ~80 % village density on alpha
    /// seeds, finding a fully-isolated cell isn't statistically feasible).
    FarFromVillages,
    /// Teleport to a Plains-biome surface position.
    InPlains,
    /// Teleport to a Forest-biome surface position.
    InForest,
    /// Teleport to a Mountains-biome surface position.
    InMountains,
    /// Teleport to a Desert-biome surface position.
    InDesert,
    /// Teleport to a beach — Plains/Desert biome adjacent to an Ocean
    /// tile at sea level.
    OnBeach,
    /// Drop at `(0.5, surface, 0.5)` — debug + "I want to start fresh"
    /// option.
    AtOrigin,
}

impl SpawnPref {
    /// Human-readable label for the menu dropdown.
    pub fn label(self) -> &'static str {
        match self {
            SpawnPref::Default => "Default (saved position)",
            SpawnPref::NearVillage => "Near a village",
            SpawnPref::FarFromVillages => "Wilderness (no village in cell)",
            SpawnPref::InPlains => "In open plains",
            SpawnPref::InForest => "In a forest",
            SpawnPref::InMountains => "In the mountains",
            SpawnPref::InDesert => "In the desert",
            SpawnPref::OnBeach => "On a beach",
            SpawnPref::AtOrigin => "At origin (0, 0)",
        }
    }

    /// Iterate over the variants in display order.
    pub fn all() -> &'static [SpawnPref] {
        &[
            SpawnPref::Default,
            SpawnPref::NearVillage,
            SpawnPref::FarFromVillages,
            SpawnPref::InPlains,
            SpawnPref::InForest,
            SpawnPref::InMountains,
            SpawnPref::InDesert,
            SpawnPref::OnBeach,
            SpawnPref::AtOrigin,
        ]
    }
}


/// Find the world-position of the nearest village anchor, searching outward
/// from `reference` in a spiral over `village_gen`'s deterministic cells.
/// Returns `None` if no village is reachable within `max_radius_cells`.
///
/// Pure — no world reads, no allocation outside the spiral cursor. Safe to
/// call before any chunks have been generated (the layout is computed from
/// the seed, not the world state).
pub fn nearest_village_anchor(
    seed: u32,
    biome_gen: &BiomeGenerator,
    reference: Vec3,
    max_radius_cells: i32,
) -> Option<Vec3> {
    let cs = CHUNK_SIZE as i32;
    let stride = village_gen::VILLAGE_GRID * cs;
    let ref_gx = (reference.x as i32).div_euclid(stride);
    let ref_gz = (reference.z as i32).div_euclid(stride);

    // Spiral by rings: r = 0, 1, 2, …. For each r, scan the ring of cells
    // at Chebyshev distance r from (ref_gx, ref_gz).
    for r in 0..=max_radius_cells {
        if r == 0 {
            if let Some(layout) = village_gen::layout_for_cell(seed, ref_gx, ref_gz, biome_gen) {
                return Some(anchor_world(&layout));
            }
            continue;
        }
        // Walk the ring's perimeter clockwise from top-left.
        for dgx in -r..=r {
            let gx = ref_gx + dgx;
            let gz_top = ref_gz - r;
            let gz_bot = ref_gz + r;
            for gz in [gz_top, gz_bot] {
                if let Some(layout) = village_gen::layout_for_cell(seed, gx, gz, biome_gen) {
                    return Some(anchor_world(&layout));
                }
            }
        }
        for dgz in (-r + 1)..r {
            let gz = ref_gz + dgz;
            let gx_left = ref_gx - r;
            let gx_right = ref_gx + r;
            for gx in [gx_left, gx_right] {
                if let Some(layout) = village_gen::layout_for_cell(seed, gx, gz, biome_gen) {
                    return Some(anchor_world(&layout));
                }
            }
        }
    }
    None
}

fn anchor_world(layout: &village_gen::VillageLayout) -> Vec3 {
    let [ax, ay, az] = layout.anchor_world;
    // Spawn slightly above the anchor block so the player drops onto the
    // hearth + doesn't suffocate inside terrain.
    Vec3::new(ax as f32 + 0.5, ay as f32 + 2.0, az as f32 + 0.5)
}

/// Search outward from a reference world position on a per-block spiral
/// for a coordinate satisfying `predicate(x, z)`. Returns the world
/// position (centred + surface + 2 above for safe spawn) of the first
/// hit. Step `stride` blocks at a time so the search covers ground
/// quickly. Bails after `max_radius_blocks` and returns `None`.
fn find_position_matching<F>(
    biome_gen: &BiomeGenerator,
    reference: Vec3,
    stride: i32,
    max_radius_blocks: i32,
    predicate: F,
) -> Option<Vec3>
where
    F: Fn(i32, i32) -> bool,
{
    let ref_x = reference.x as i32;
    let ref_z = reference.z as i32;
    if predicate(ref_x, ref_z) {
        let y = biome_gen.terrain_height(ref_x, ref_z);
        return Some(Vec3::new(ref_x as f32 + 0.5, y as f32 + 2.0, ref_z as f32 + 0.5));
    }
    let rings = max_radius_blocks / stride;
    for r in 1..=rings {
        let edge = r * stride;
        for dx in (-edge..=edge).step_by(stride as usize) {
            for dz in [-edge, edge] {
                let x = ref_x + dx;
                let z = ref_z + dz;
                if predicate(x, z) {
                    let y = biome_gen.terrain_height(x, z);
                    return Some(Vec3::new(x as f32 + 0.5, y as f32 + 2.0, z as f32 + 0.5));
                }
            }
        }
        for dz in (-edge + stride..edge).step_by(stride as usize) {
            for dx in [-edge, edge] {
                let x = ref_x + dx;
                let z = ref_z + dz;
                if predicate(x, z) {
                    let y = biome_gen.terrain_height(x, z);
                    return Some(Vec3::new(x as f32 + 0.5, y as f32 + 2.0, z as f32 + 0.5));
                }
            }
        }
    }
    None
}

/// True if no village anchor falls within `cell_radius` grid cells of
/// the (x, z) block position. Used by the "Far from villages" search.
fn is_far_from_villages(seed: u32, biome_gen: &BiomeGenerator, x: i32, z: i32, cell_radius: i32) -> bool {
    let cs = CHUNK_SIZE as i32;
    let stride = village_gen::VILLAGE_GRID * cs;
    let gx = x.div_euclid(stride);
    let gz = z.div_euclid(stride);
    for dgx in -cell_radius..=cell_radius {
        for dgz in -cell_radius..=cell_radius {
            if village_gen::layout_for_cell(seed, gx + dgx, gz + dgz, biome_gen).is_some() {
                return false;
            }
        }
    }
    true
}

/// True if (x, z) is a beach — biome is Plains or Desert AND any of the 4
/// orthogonal neighbours one step out is an Ocean tile. Surface y must be
/// close to sea level so the player spawns on actual sand, not a cliff
/// edge above the sea.
fn is_beach(biome_gen: &BiomeGenerator, x: i32, z: i32) -> bool {
    let here = biome_gen.biome_at(x, z);
    if !matches!(here, Biome::Plains | Biome::Desert) {
        return false;
    }
    let y = biome_gen.terrain_height(x, z);
    if (y - SEA_LEVEL).abs() > 4 {
        return false;
    }
    for (dx, dz) in [(8, 0), (-8, 0), (0, 8), (0, -8)] {
        if biome_gen.biome_at(x + dx, z + dz) == Biome::Ocean {
            return true;
        }
    }
    false
}

/// Compute the spawn position implied by `pref`, given the world seed +
/// biome generator + the saved-position fallback. The caller applies the
/// result to `players[0].player.pos` after `chunk_stream::initial_load`.
pub fn resolve_spawn(
    pref: SpawnPref,
    seed: u32,
    biome_gen: &BiomeGenerator,
    saved_position: Vec3,
) -> Vec3 {
    match pref {
        SpawnPref::Default => saved_position,
        SpawnPref::NearVillage => nearest_village_anchor(seed, biome_gen, saved_position, 32)
            .unwrap_or(saved_position),
        SpawnPref::AtOrigin => {
            let surface = biome_gen.terrain_height(0, 0);
            Vec3::new(0.5, surface as f32 + 2.0, 0.5)
        }
        SpawnPref::FarFromVillages => find_position_matching(
            biome_gen, saved_position, 32, 2048,
            // cell_radius=0 — just require the spawn cell itself to have
            // no village. With ~80 % village density per cell, finding a
            // 3×3-cell isolated patch is ~5e-7 probability and a 5×5 patch
            // is statistically unreachable on alpha seeds. Restricting to
            // "spawn cell has no village" is the practical interpretation
            // of "wilderness" that actually finds a hit in the search budget.
            |x, z| is_far_from_villages(seed, biome_gen, x, z, 0),
        )
        .unwrap_or(saved_position),
        SpawnPref::InPlains => find_position_matching(
            biome_gen, saved_position, 16, 2048,
            |x, z| biome_gen.biome_at(x, z) == Biome::Plains,
        )
        .unwrap_or(saved_position),
        SpawnPref::InForest => find_position_matching(
            biome_gen, saved_position, 16, 2048,
            |x, z| biome_gen.biome_at(x, z) == Biome::Forest,
        )
        .unwrap_or(saved_position),
        SpawnPref::InMountains => find_position_matching(
            biome_gen, saved_position, 32, 4096,
            |x, z| biome_gen.biome_at(x, z) == Biome::Mountains,
        )
        .unwrap_or(saved_position),
        SpawnPref::InDesert => find_position_matching(
            biome_gen, saved_position, 32, 4096,
            |x, z| biome_gen.biome_at(x, z) == Biome::Desert,
        )
        .unwrap_or(saved_position),
        SpawnPref::OnBeach => find_position_matching(
            biome_gen, saved_position, 8, 2048,
            |x, z| is_beach(biome_gen, x, z),
        )
        .unwrap_or(saved_position),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_village_returns_some_for_seed_42() {
        let bg = BiomeGenerator::new(42);
        let pos = nearest_village_anchor(42, &bg, Vec3::ZERO, 32);
        assert!(pos.is_some(), "expected at least one village within 32 cells of origin");
    }

    #[test]
    fn resolve_default_returns_saved_position() {
        let bg = BiomeGenerator::new(42);
        let saved = Vec3::new(123.0, 80.0, 456.0);
        let resolved = resolve_spawn(SpawnPref::Default, 42, &bg, saved);
        assert_eq!(resolved, saved);
    }

    #[test]
    fn resolve_at_origin_returns_origin_at_surface() {
        let bg = BiomeGenerator::new(42);
        let resolved = resolve_spawn(SpawnPref::AtOrigin, 42, &bg, Vec3::new(999.0, 999.0, 999.0));
        assert_eq!(resolved.x, 0.5);
        assert_eq!(resolved.z, 0.5);
        assert!(resolved.y > 0.0 && resolved.y < 200.0, "surface y looks plausible: {}", resolved.y);
    }

    #[test]
    fn resolve_near_village_finds_a_village_position() {
        let bg = BiomeGenerator::new(42);
        let saved = Vec3::new(0.0, 80.0, 0.0);
        let resolved = resolve_spawn(SpawnPref::NearVillage, 42, &bg, saved);
        // If the search worked, resolved should differ from saved (or
        // happen to coincide if a village is literally at 0,0 — unlikely).
        // Either way the y is now a village anchor y, not 80 unless by chance.
        assert_ne!(resolved, saved, "expected the spawn override to move us");
    }

    #[test]
    fn label_strings_are_distinct() {
        let labels: Vec<&str> = SpawnPref::all().iter().map(|p| p.label()).collect();
        let unique: std::collections::HashSet<&&str> = labels.iter().collect();
        assert_eq!(labels.len(), unique.len(), "all spawn-pref labels must be unique");
    }

    #[test]
    fn biome_searches_find_matching_terrain() {
        let bg = BiomeGenerator::new(42);
        // For each biome-keyed pref, the resolved position's biome should
        // match. Search budget is generous so we expect a hit for each
        // major biome within reach of origin on seed 42.
        for (pref, biome) in [
            (SpawnPref::InPlains, Biome::Plains),
            (SpawnPref::InForest, Biome::Forest),
            (SpawnPref::InDesert, Biome::Desert),
            (SpawnPref::InMountains, Biome::Mountains),
        ] {
            let resolved = resolve_spawn(pref, 42, &bg, Vec3::ZERO);
            // If the search fell back to saved (no match), the y will be 0
            // and the biome assertion may fail — that's OK to surface here.
            let biome_at = bg.biome_at(resolved.x as i32, resolved.z as i32);
            assert_eq!(biome_at, biome,
                "{:?}: resolved ({:.0}, {:.0}) but biome there is {:?}, expected {:?}",
                pref, resolved.x, resolved.z, biome_at, biome,
            );
        }
    }

    #[test]
    fn far_from_villages_actually_finds_a_village_free_cell() {
        let bg = BiomeGenerator::new(42);
        let resolved = resolve_spawn(SpawnPref::FarFromVillages, 42, &bg, Vec3::ZERO);
        // Predicate is cell_radius=0 — spawn cell itself has no village
        // (neighbour cells may; alpha village density makes stricter
        // patches statistically unfindable). Match the test.
        let cs = CHUNK_SIZE as i32;
        let stride = village_gen::VILLAGE_GRID * cs;
        let gx = (resolved.x as i32).div_euclid(stride);
        let gz = (resolved.z as i32).div_euclid(stride);
        assert!(
            village_gen::layout_for_cell(42, gx, gz, &bg).is_none(),
            "FarFromVillages spawn cell ({}, {}) has a village", gx, gz,
        );
        // Sanity: shouldn't have fallen back to saved_position — village
        // exists in cell (0, 0)'s range on seed 42, but a nearby village-
        // free cell does exist.
        assert_ne!(resolved, Vec3::ZERO, "FarFromVillages search fell back to saved position");
    }
}

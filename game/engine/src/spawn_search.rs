//! The world spawn column of a v3 (or later) world: terrain design §10, Spec 02
//! §5.3.8.
//!
//! A v3 world's origin is often wet or on a steep face (every climate field's
//! first octave is zero there, so it sits in the deepest valley: a third to a
//! half of seeds start on a lake or sea floor), so the old +-6 block search
//! round the origin ([`crate::chunk_stream`]'s `find_surface_spawn`) is not
//! enough. This module spirals outward from the origin, up to
//! [`SEARCH_CHUNKS`] chunks, to the first column that is dry land, not
//! Ocean / DeepOcean / River / Beach, with a top in [`TOP_RANGE`] and a slope
//! under [`MAX_SLOPE`].
//!
//! It decides on **pure queries** only (`biome_at`, `terrain_height`,
//! `surface_y`, the lattice's slope), so it generates nothing and is the same
//! for a seed on every machine. The caller generates the chosen column's 3x3
//! chunks and only then runs `ground_with_headroom` there, which reads blocks.
//! v2 worlds do not use it: their spawn is the +-6 block search at the origin,
//! unchanged.

use crate::biome::{Biome, BiomeGenerator, Generator};
use crate::chunk::CHUNK_SIZE;

/// How far the search reaches, in chunks.
pub const SEARCH_CHUNKS: i32 = 64;
/// Candidate spacing in blocks (a coarse grid keeps the worst case cheap).
pub const STEP: i32 = 8;
/// The accepted height of a spawn column's top solid block (design §10).
pub const TOP_RANGE: std::ops::RangeInclusive<i32> = 63..=110;
/// Slopes at or over this are refused (the largest 4-neighbour height step).
pub const MAX_SLOPE: f64 = 2.0;
/// The lowest top solid block that is above the sea (water fills to y 62).
const DRY_TOP: i32 = 63;

/// How good the chosen column is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    /// Met every rule.
    Preferred,
    /// Nothing met every rule within the reach: the nearest dry, non-water,
    /// non-river column (it may be steep, high or a beach).
    DryOnly,
    /// Nothing dry within the reach (or a v2 world): the origin, with the old
    /// +-6 block search.
    Origin,
}

/// The chosen world spawn column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnColumn {
    pub x: i32,
    pub z: i32,
    pub kind: SpawnKind,
}

impl SpawnColumn {
    /// The chunk column holding it.
    pub fn chunk(&self) -> (i32, i32) {
        let cs = CHUNK_SIZE as i32;
        (self.x.div_euclid(cs), self.z.div_euclid(cs))
    }

    /// Chebyshev distance from the origin in chunks, rounded up.
    pub fn distance_chunks(&self) -> i32 {
        let cs = CHUNK_SIZE as i32;
        (self.x.abs().max(self.z.abs()) + cs - 1) / cs
    }
}

/// Dry land that is not a river or sea: the [`SpawnKind::DryOnly`] rule.
fn dry_land(bg: &BiomeGenerator, x: i32, z: i32) -> bool {
    bg.terrain_height(x, z) >= DRY_TOP
        && bg.surface_y(x, z) >= DRY_TOP
        && !matches!(bg.biome_at(x, z), Biome::Ocean | Biome::DeepOcean | Biome::River)
}

/// Whether column `(x, z)` meets every rule of design §10 (pure).
pub fn is_preferred(bg: &BiomeGenerator, x: i32, z: i32) -> bool {
    let top = bg.terrain_height(x, z);
    if !TOP_RANGE.contains(&top) {
        return false;
    }
    let biome = bg.biome_at(x, z);
    if matches!(biome, Biome::Ocean | Biome::DeepOcean | Biome::River | Biome::Beach) {
        return false;
    }
    // The real top (3D noise included) must be dry land too, not far above the
    // 2D estimate (an overhang or a cliff).
    let real = bg.surface_y(x, z);
    if real < DRY_TOP || real > *TOP_RANGE.end() + 8 {
        return false;
    }
    match bg.v3() {
        Some(t) => t.lattice.slope(x, z, t.height2d(x, z)) < MAX_SLOPE,
        // Another generator: the same rule on its integer heights.
        None => [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .all(|&(dx, dz)| ((bg.terrain_height(x + dx, z + dz) - top).abs() as f64) < MAX_SLOPE),
    }
}

/// Search for the world spawn column of a non-v2 world, uncached. Spirals in
/// square rings of [`STEP`] blocks from the origin; the order is fixed, so the
/// result is deterministic for a seed. A v2 world returns the origin.
pub fn search(bg: &BiomeGenerator) -> SpawnColumn {
    let origin = SpawnColumn { x: 0, z: 0, kind: SpawnKind::Origin };
    if matches!(bg.generator(), Generator::V2) {
        return origin;
    }
    let cs = CHUNK_SIZE as i32;
    let rings = SEARCH_CHUNKS * cs / STEP;
    let mut dry: Option<SpawnColumn> = None;
    for ring in 0..=rings {
        let edge = ring * STEP;
        for dz in (-edge..=edge).step_by(STEP as usize) {
            for dx in (-edge..=edge).step_by(STEP as usize) {
                // The perimeter of this ring only (inner rings were tried).
                if dx.abs().max(dz.abs()) != edge {
                    continue;
                }
                if is_preferred(bg, dx, dz) {
                    return SpawnColumn { x: dx, z: dz, kind: SpawnKind::Preferred };
                }
                if dry.is_none() && dry_land(bg, dx, dz) {
                    dry = Some(SpawnColumn { x: dx, z: dz, kind: SpawnKind::DryOnly });
                }
            }
        }
    }
    match dry {
        Some(found) => {
            log::warn!(
                "no ideal spawn within {SEARCH_CHUNKS} chunks of seed {}: using the nearest dry column ({}, {})",
                bg.seed, found.x, found.z
            );
            found
        }
        None => {
            log::warn!(
                "no dry spawn within {SEARCH_CHUNKS} chunks of seed {}: spawning at the origin",
                bg.seed
            );
            origin
        }
    }
}

/// [`search`], computed once per generator.
pub fn world_spawn_column(bg: &BiomeGenerator) -> SpawnColumn {
    *bg.spawn_search.get_or_init(|| search(bg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::chunk_stream::{snap_spawn_to_ground, spawn_pregen_columns, world_spawn_point};
    use crate::spawn_pref::{resolve_spawn, SpawnPref};
    use crate::world::World;

    fn v3(seed: u32) -> BiomeGenerator {
        BiomeGenerator::new(seed, Generator::V3)
    }

    /// Generate the spawn columns the way `begin_load` and `GameServer::world_spawn` do.
    fn world_with_spawn_columns(bg: &BiomeGenerator) -> World {
        let mut world = World::new();
        for (cx, cz) in spawn_pregen_columns(&world, bg) {
            world.generate_column(cx, cz, bg);
        }
        world
    }

    /// The player at `pos` stands in two air cells on ground that is neither
    /// air, water nor lava, with the cell under the feet and the cells round
    /// the body dry.
    fn assert_standing(world: &World, pos: glam::Vec3, what: &str) {
        let (x, y, z) = (pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32);
        let at = |dy: i32| world.get_block(x, y + dy, z);
        assert_eq!(at(0), block::AIR, "{what}: feet cell is not air at {pos:?}");
        assert_eq!(at(1), block::AIR, "{what}: head cell is not air at {pos:?}");
        let ground = at(-1);
        assert!(
            ground != block::AIR && ground != block::WATER && ground != block::LAVA,
            "{what}: ground under the feet is {ground} at {pos:?}"
        );
        for dy in 0..=2 {
            let b = at(dy);
            assert!(b != block::WATER && b != block::LAVA, "{what}: liquid {b} at {pos:?}+{dy}");
        }
    }

    /// Pure sweep: every seed finds a preferred column within the reach, and it
    /// really meets every rule of design §10.
    #[test]
    fn every_v3_seed_finds_a_preferred_spawn_column_within_64_chunks() {
        for seed in 0..150u32 {
            let bg = v3(seed.wrapping_mul(2_654_435_761));
            let col = world_spawn_column(&bg);
            assert_eq!(col.kind, SpawnKind::Preferred, "seed {seed}: {col:?}");
            assert!(col.distance_chunks() <= SEARCH_CHUNKS, "seed {seed}: {col:?}");
            assert!(is_preferred(&bg, col.x, col.z));
            let top = bg.terrain_height(col.x, col.z);
            assert!(TOP_RANGE.contains(&top), "seed {seed}: top {top}");
            assert!(!matches!(
                bg.biome_at(col.x, col.z),
                Biome::Ocean | Biome::DeepOcean | Biome::River | Biome::Beach
            ));
        }
    }

    #[test]
    fn the_spawn_search_is_deterministic_and_cached() {
        let a = search(&v3(7));
        assert_eq!(a, search(&v3(7)));
        let bg = v3(7);
        assert_eq!(world_spawn_column(&bg), a);
        assert_eq!(world_spawn_column(&bg), a);
    }

    /// End to end on a handful of seeds: the spawn is dry, in air, with headroom.
    #[test]
    fn v3_world_spawn_is_dry_with_headroom() {
        for seed in [1u32, 2, 3, 42, 777, 2024, 9_999_991, 0xDEAD_BEEF] {
            let bg = v3(seed);
            let world = world_with_spawn_columns(&bg);
            let spawn = world_spawn_point(&world, &bg);
            assert_standing(&world, spawn, &format!("seed {seed}"));
        }
    }

    /// "In the mountains" lands on the surface with headroom, not in the rock.
    #[test]
    fn in_mountains_on_v3_lands_on_the_surface_not_in_rock() {
        let mut checked = 0;
        for seed in [1u32, 2, 3, 42, 777, 2024] {
            let bg = v3(seed);
            let saved = glam::Vec3::new(8.5, 80.0, 8.5);
            let pos = resolve_spawn(SpawnPref::InMountains, seed, &bg, saved);
            if pos == saved {
                continue;
            }
            let mut world = World::new();
            let (cx, cz) = ((pos.x.floor() as i32).div_euclid(16), (pos.z.floor() as i32).div_euclid(16));
            for dx in -2..=2 {
                for dz in -2..=2 {
                    world.generate_column(cx + dx, cz + dz, &bg);
                }
            }
            let snapped = snap_spawn_to_ground(&world, &bg, pos);
            assert_standing(&world, snapped, &format!("mountains seed {seed}"));
            checked += 1;
        }
        assert!(checked >= 3, "too few seeds found mountains: {checked}");
    }

    /// v2 worlds keep their exact spawn (the +-6 block search at the origin).
    #[test]
    fn v2_world_spawn_is_unchanged() {
        for (seed, expected) in V2_SPAWNS {
            let bg = BiomeGenerator::new(seed, Generator::V2);
            let world = world_with_spawn_columns(&bg);
            let cols = spawn_pregen_columns(&world, &bg);
            assert_eq!(cols[0], (-1, -1), "v2 pregens the 3x3 round the origin");
            let spawn = world_spawn_point(&world, &bg);
            assert_eq!([spawn.x, spawn.y, spawn.z], expected, "v2 seed {seed}");
            assert_eq!(world_spawn_column(&bg).kind, SpawnKind::Origin);
        }
    }

    /// Pinned from the unmodified v2 spawn (seed, [x, feet y, z]).
    const V2_SPAWNS: [(u32, [f32; 3]); 3] = [(1, [0.5, 63.0, 0.5]), (42, [0.5, 63.0, 0.5]), (777, [0.5, 63.0, 0.5])];
}

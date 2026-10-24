//! v3 terrain: purity (pure queries equal the column fill), the generator
//! ceiling, and shape sanity. Metrics, the map tool and the bench live in
//! `metrics.rs`; the golden in `test_integration/worldgen_v3_golden.rs`.

use super::*;
use crate::biome::{BiomeGenerator, Generator};
use crate::world::{terrain_block_at, World, V2_WORLD_TOP};

fn v3(seed: u32) -> BiomeGenerator {
    BiomeGenerator::new(seed, Generator::V3)
}

/// A chunk column whose 2D height peaks above `min_h`, searched outward.
pub(super) fn find_column_above(t: &Terrain, min_h: f64) -> (i32, i32) {
    for r in 0..400i32 {
        for cx in -r..=r {
            for cz in [-r, r] {
                for (x, z) in [(cx, cz), (cz, cx)] {
                    if t.height2d(x * 16 + 8, z * 16 + 8) > min_h {
                        return (x, z);
                    }
                }
            }
        }
    }
    panic!("no column above {min_h} within 400 chunks");
}

#[test]
fn pure_block_at_equals_the_column_fill() {
    // The `terrain_block_at` purity contract (world.rs): a decoration pass
    // asking about a neighbour column before it generates must get exactly
    // what the column fill will put there. Every cell of a few columns
    // (plains, a mountain, the coast), plus scattered cells of many more.
    for seed in [1, 42, 1337] {
        let bg = v3(seed);
        let t = bg.v3().unwrap();
        let mut cols = vec![(0, 0), (-3, 5), find_column_above(t, 150.0)];
        for i in 0..24 {
            cols.push((i * 37 - 400, (i * i * 13) % 500 - 250));
        }
        for (k, &(cx, cz)) in cols.iter().enumerate() {
            let blocks = fill::column_blocks(&bg, t, cx, cz);
            let every = if k < 3 { 1 } else { 7 };
            for (i, &b) in blocks.iter().enumerate().step_by(every) {
                let y = (i % fill::HEIGHT) as i32;
                let c = (i / fill::HEIGHT) as i32;
                let (x, z) = (cx * 16 + c % 16, cz * 16 + c / 16);
                assert_eq!(
                    terrain_block_at(&bg, x, y, z),
                    b,
                    "seed {seed}: pure vs column at ({x},{y},{z})"
                );
            }
        }
    }
}

#[test]
fn pure_solid_and_height_agree_with_the_fill() {
    // solid_at / surface_y / height2d are what T2b/T3 build on.
    let bg = v3(42);
    let t = bg.v3().unwrap();
    let (cx, cz) = find_column_above(t, 140.0);
    let blocks = fill::column_blocks(&bg, t, cx, cz);
    for c in 0..256 {
        let (x, z) = (cx * 16 + c % 16, cz * 16 + c / 16);
        let col = &blocks[c as usize * fill::HEIGHT..(c as usize + 1) * fill::HEIGHT];
        let top = (0..fill::HEIGHT).rev().find(|&y| col[y] != crate::block::AIR && col[y] != crate::block::WATER);
        assert_eq!(top, Some(t.surface_y(x, z) as usize), "surface_y at ({x},{z})");
        for y in 1..fill::HEIGHT as i32 {
            let filled = !matches!(col[y as usize], crate::block::AIR | crate::block::WATER);
            if y > 60 {
                // above the v2 cave band every non-fluid cell is density-solid
                assert_eq!(t.solid_at(x, y, z), filled, "solid_at ({x},{y},{z})");
            }
        }
        assert_eq!(bg.terrain_height(x, z), lattice::top_of(t.height2d(x, z)));
    }
}

#[test]
fn a_v3_mountain_column_really_holds_blocks_above_the_old_roof() {
    // `World::worldgen_top` clips world-gen writes to the generator's
    // `max_generated_height`; v2's is 95. A v3 peak must survive the clip.
    let bg = v3(1337);
    let t = bg.v3().unwrap();
    let (cx, cz) = find_column_above(t, 150.0);
    let mut w = World::new();
    w.generate_column(cx, cz, &bg);
    let high = (0..16)
        .flat_map(|x| (0..16).map(move |z| (cx * 16 + x, cz * 16 + z)))
        .filter(|&(x, z)| (V2_WORLD_TOP + 1..256).any(|y| w.get_block(x, y, z) != crate::block::AIR))
        .count();
    assert!(high > 128, "only {high} of 256 columns hold a block above y 95");
    assert!(w.has_chunk(cx, 9, cz), "a section above y 150 exists");
}

#[test]
fn v2_generators_carry_no_v3_terrain() {
    assert!(BiomeGenerator::new(7, Generator::V2).v3().is_none());
    assert!(v3(7).v3().is_some());
    // And v2 never labels the v3-only biomes.
    let g = BiomeGenerator::new(42, Generator::V2);
    for x in (-3000..3000).step_by(64) {
        for z in (-3000..3000).step_by(64) {
            assert!(!matches!(
                g.biome_at(x, z),
                Biome::DeepOcean | Biome::River | Biome::Beach
            ));
        }
    }
}

#[test]
fn lattice_points_are_exact_between_paths() {
    // At a lattice point the bilinear blend returns the point itself.
    let t = Terrain::new(9);
    for (ix, iz) in [(0, 0), (-5, 3), (100, -77)] {
        let p = t.lattice.point(ix, iz);
        assert_eq!(t.sample(ix * lattice::STEP, iz * lattice::STEP), p);
        assert_eq!(lattice::compute(&t.lattice.fields, ix, iz), p, "memo returns what compute does");
    }
}

#[test]
fn rivers_carve_to_their_bed_and_oceans_are_deep() {
    let t = Terrain::new(42);
    let (mut river, mut deep) = (0, 0);
    for x in (-4096..4096).step_by(16) {
        for z in (-4096..4096).step_by(16) {
            match t.biome_at(x, z) {
                Biome::River => {
                    river += 1;
                    assert!(t.terrain_height(x, z) < 63);
                }
                Biome::DeepOcean => {
                    deep += 1;
                    assert!(t.terrain_height(x, z) <= 45, "deep ocean floor at {}", t.terrain_height(x, z));
                }
                _ => {}
            }
        }
    }
    assert!(river > 50, "rivers: {river}");
    assert!(deep > 50, "deep ocean: {deep}");
}

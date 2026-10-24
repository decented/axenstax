//! v4 = v3 + the dry caves (design §7): v4 changes nothing but the caves, and
//! v3 builds no cave noise. The caves' own acceptance numbers are in
//! `cave_metrics.rs`, the pinned output in `test_integration/worldgen_v4_golden.rs`.

use super::*;
use crate::biome::{BiomeGenerator, Generator};
use crate::block;
use crate::world::{terrain_block_at, worldgen_fingerprint_for};

fn bgen(seed: u32, g: Generator) -> BiomeGenerator {
    BiomeGenerator::new(seed, g)
}

#[test]
fn only_v4_builds_cave_noise() {
    assert!(bgen(5, Generator::V3).v3().unwrap().caves().is_none(), "v3 must build no cave noise");
    assert!(bgen(5, Generator::V4).v3().unwrap().caves().is_some());
    assert!(bgen(5, Generator::V2).v3().is_none());
    // A terrain without caves carves nothing, whatever the cell.
    let t3 = Terrain::new(5);
    assert!(!caves::is_carved(&t3, 10, 30, 10));
}

#[test]
fn v4_is_a_known_generator_new_worlds_do_not_get() {
    assert_eq!(Generator::V4.version(), 4);
    assert_eq!(Generator::from_version(4), Ok(Generator::V4));
    assert_eq!(crate::biome::NEW_WORLD_GENERATOR, 3, "v4 is not stamped on new worlds yet");
    assert_eq!(Generator::for_new_world(), Generator::V3);
    let next = Generator::ALL.iter().map(|g| g.version()).max().unwrap() + 1;
    assert!(Generator::from_version(next).is_err());
    assert_eq!(crate::biome::V3_REVISION, 1, "v3 is frozen at revision 1");
    assert_eq!(crate::biome::V4_REVISION, 0);
}

#[test]
fn v2_v3_and_v4_fingerprints_are_distinct() {
    let fp: Vec<u32> = Generator::ALL.iter().map(|&g| worldgen_fingerprint_for(g)).collect();
    assert_eq!(fp.len(), 3);
    for (i, a) in fp.iter().enumerate() {
        assert_ne!(*a, 0);
        for b in &fp[i + 1..] {
            assert_ne!(a, b, "two generators share a fingerprint: {fp:x?}");
        }
    }
    // v2's value is the one every build since protocol v65 exchanged.
    assert_eq!(worldgen_fingerprint_for(Generator::V2), crate::world::worldgen_fingerprint());
    // The revision is folded in.
    assert_ne!(crate::world::v4_output_version(0), crate::world::v4_output_version(1));
    assert_ne!(crate::world::v4_output_version(0), crate::world::v3_output_version(0));
    assert_ne!(crate::world::v4_output_version(1), crate::world::v3_output_version(1));
}

#[test]
fn v4_shares_v3s_shape_surface_labels_and_height() {
    for seed in [1, 42, 1337] {
        let (a, b) = (bgen(seed, Generator::V3), bgen(seed, Generator::V4));
        assert_eq!(a.max_generated_height(), b.max_generated_height());
        for x in (-300..300).step_by(37) {
            for z in (-300..300).step_by(41) {
                assert_eq!(a.surface_y(x, z), b.surface_y(x, z), "surface_y seed {seed} ({x},{z})");
                assert_eq!(a.terrain_height(x, z), b.terrain_height(x, z));
                assert_eq!(a.biome_at(x, z), b.biome_at(x, z));
            }
        }
    }
}

/// Every cell where v4 differs from v3 is either a cave v4 carved or a v2
/// flooded cave v3 had; nowhere else.
#[test]
fn v4_differs_from_v3_only_by_caves() {
    let mut carved_seen = 0u64;
    for seed in [1, 42, 1337] {
        let (b3, b4) = (bgen(seed, Generator::V3), bgen(seed, Generator::V4));
        let (t3, t4) = (b3.v3().unwrap(), b4.v3().unwrap());
        for &(cx, cz) in &[(0, 0), (-3, 5), (7, -2), (20, 20)] {
            let c3 = fill::column_blocks(&b3, t3, cx, cz);
            let c4 = fill::column_blocks(&b4, t4, cx, cz);
            for (i, (&a, &b)) in c3.iter().zip(&c4).enumerate() {
                if a == b {
                    continue;
                }
                let y = (i % fill::HEIGHT) as i32;
                let c = (i / fill::HEIGHT) as i32;
                let (x, z) = (cx * 16 + c % 16, cz * 16 + c / 16);
                let carved = caves::is_carved(t4, x, y, z);
                if carved {
                    carved_seen += 1;
                    assert!(matches!(b, block::AIR | block::LAVA), "carved cell holds {b} at ({x},{y},{z})");
                } else {
                    assert!(b3.is_cave(x, y, z), "({x},{y},{z}): v4 {b} v3 {a}, neither a v4 cave nor a v2 cave");
                }
            }
        }
    }
    assert!(carved_seen > 0, "no v4 cave in the sampled columns");
}

#[test]
fn v4_pure_block_at_equals_the_column_fill_and_is_cave_agrees() {
    // The `terrain_block_at` purity contract, with caves in play.
    let mut carved = 0u64;
    for seed in [1, 42, 1337] {
        let bg = bgen(seed, Generator::V4);
        let t = bg.v3().unwrap();
        let mut cols = vec![(0, 0), (-3, 5), tests::find_column_above(t, 150.0)];
        for i in 0..24 {
            cols.push((i * 37 - 400, (i * i * 13) % 500 - 250));
        }
        for (k, &(cx, cz)) in cols.iter().enumerate() {
            let blocks = fill::column_blocks(&bg, t, cx, cz);
            let every = if k < 3 { 1 } else { 5 };
            for (i, &b) in blocks.iter().enumerate().step_by(every) {
                let y = (i % fill::HEIGHT) as i32;
                let c = (i / fill::HEIGHT) as i32;
                let (x, z) = (cx * 16 + c % 16, cz * 16 + c / 16);
                assert_eq!(terrain_block_at(&bg, x, y, z), b, "seed {seed}: pure vs column at ({x},{y},{z})");
                // `is_cave` is the fill's own decision, never v2's.
                let is_cave = bg.is_cave(x, y, z);
                assert_eq!(is_cave, caves::is_carved(t, x, y, z));
                if is_cave {
                    carved += 1;
                    assert!(matches!(b, block::AIR | block::LAVA), "({x},{y},{z}) holds {b}");
                }
            }
        }
    }
    assert!(carved > 0);
}

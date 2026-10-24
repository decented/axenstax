//! Terrain v3 golden + order independence (the v2 pattern in
//! `worldgen_golden.rs`, which stays pinned to v2).
//!
//! A joiner regenerates a v3 host's untouched columns from the seed, so v3
//! output must be a pure function of the seed: the same blocks in any column
//! order, on every machine. The golden hashes every cell y 0..=255 of a
//! normal-terrain patch at the origin plus a mountain patch (peaks above y
//! 150, so the 3D overhangs and the tall sections are covered).
//!
//! NOTE for lane T2b (v3 caves) and T3 (trees, structures): your lanes change
//! v3 output, so re-pin `V3_GOLDEN` (and bump `V3_GOLDEN_REVISION`).

use crate::biome::{BiomeGenerator, Generator};
use crate::chunk::CHUNK_SIZE;
use crate::world::{World, WORLD_HEIGHT};

const SEED: u32 = 20_261_024;

/// Chunk `(-21, 21)`: the nearest chunk to the origin (spiral search) whose
/// centre's 2D height exceeds 150 for `SEED` (found with
/// `worldgen_v3::tests::find_column_above`).
const MOUNTAIN: (i32, i32) = (-21, 21);

fn columns() -> Vec<(i32, i32)> {
    let mut cols = Vec::new();
    for cx in -2..2 {
        for cz in -2..2 {
            cols.push((cx, cz));
        }
    }
    for cx in MOUNTAIN.0 - 1..=MOUNTAIN.0 + 1 {
        for cz in MOUNTAIN.1 - 1..=MOUNTAIN.1 + 1 {
            cols.push((cx, cz));
        }
    }
    cols
}

fn orders(cols: &[(i32, i32)]) -> Vec<(&'static str, Vec<(i32, i32)>)> {
    let mut reversed = cols.to_vec();
    reversed.reverse();
    let mut scattered = cols.to_vec();
    scattered.sort_by_key(|&(x, z)| {
        (x as u32).wrapping_mul(0x9E37_79B9) ^ (z as u32).wrapping_mul(0x85EB_CA6B)
    });
    vec![("row-major", cols.to_vec()), ("reversed", reversed), ("scattered", scattered)]
}

fn generate(order: &[(i32, i32)]) -> World {
    let bg = BiomeGenerator::new(SEED, Generator::V3);
    let mut w = World::new();
    for &(cx, cz) in order {
        w.generate_column(cx, cz, &bg);
    }
    w
}

fn fnv(h: &mut u64, b: u8) {
    *h ^= b as u64;
    *h = h.wrapping_mul(0x0000_0100_0000_01B3);
}

/// FNV-1a over every cell of `cols` (fixed coordinate order): block id, meta,
/// then the side tables' keys.
fn hash(w: &World, cols: &[(i32, i32)]) -> u64 {
    let cs = CHUNK_SIZE as i32;
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &(cx, cz) in cols {
        for x in cx * cs..(cx + 1) * cs {
            for z in cz * cs..(cz + 1) * cs {
                for y in 0..WORLD_HEIGHT {
                    for b in w.get_block(x, y, z).to_le_bytes() {
                        fnv(&mut h, b);
                    }
                    fnv(&mut h, w.block_meta.get(&(x, y, z)).copied().unwrap_or(0));
                }
            }
        }
    }
    let mut keys: Vec<String> = w.block_entities.keys().map(|k| format!("be{k:?}")).collect();
    keys.extend(w.brigand_hideouts.keys().map(|k| format!("hideout{k:?}")));
    keys.extend(w.village_anchors.iter().map(|(k, v)| format!("village{k:?}{v:?}")));
    keys.sort();
    for k in keys {
        for b in k.bytes() {
            fnv(&mut h, b);
        }
    }
    h
}

/// The pinned v3 output. When the v3 golden changes, bump this AND
/// biome::V3_REVISION to match. The session wires the equality assert after
/// merging.
const V3_GOLDEN: u64 = 0;
const V3_GOLDEN_REVISION: u32 = 1;

#[test]
fn v3_terrain_does_not_depend_on_column_order() {
    let cols = columns();
    let mut reference: Option<World> = None;
    for (name, order) in orders(&cols) {
        let w = generate(&order);
        match &reference {
            None => reference = Some(w),
            Some(r) => {
                let cs = CHUNK_SIZE as i32;
                for &(cx, cz) in &cols {
                    for x in cx * cs..(cx + 1) * cs {
                        for z in cz * cs..(cz + 1) * cs {
                            for y in 0..WORLD_HEIGHT {
                                assert_eq!(
                                    r.get_block(x, y, z),
                                    w.get_block(x, y, z),
                                    "{name} vs row-major at ({x},{y},{z})"
                                );
                            }
                        }
                    }
                }
                assert_eq!(hash(r, &cols), hash(&w, &cols), "{name}: side tables differ");
            }
        }
    }
}

#[test]
fn v3_golden_set_reaches_the_mountains() {
    // The premise: the mountain patch really is tall (else re-find MOUNTAIN).
    let w = generate(&columns());
    let cs = CHUNK_SIZE as i32;
    let tall = (MOUNTAIN.0 * cs..(MOUNTAIN.0 + 1) * cs)
        .flat_map(|x| (MOUNTAIN.1 * cs..(MOUNTAIN.1 + 1) * cs).map(move |z| (x, z)))
        .filter(|&(x, z)| (150..WORLD_HEIGHT).any(|y| w.get_block(x, y, z) != crate::block::AIR))
        .count();
    if tall <= 32 {
        // Re-find: the nearest chunk (rings around the origin) whose centre's
        // 2D height exceeds 150.
        let t = BiomeGenerator::new(SEED, Generator::V3);
        let t = t.v3().unwrap();
        let found = (0..400i32).find_map(|r| {
            (-r..=r).flat_map(|a| [(a, -r), (a, r), (-r, a), (r, a)]).find(|&(x, z)| t.height2d(x * 16 + 8, z * 16 + 8) > 150.0)
        });
        panic!("only {tall} columns of the mountain chunk reach y 150; nearest tall chunk: {found:?}");
    }
}

#[test]
fn v3_worldgen_output_matches_golden() {
    let got = hash(&generate(&columns()), &columns());
    assert_eq!(got, hash(&generate(&columns()), &columns()), "v3 is not deterministic within a process");
    assert_eq!(
        got, V3_GOLDEN,
        "v3 worldgen output changed (revision {V3_GOLDEN_REVISION}): new hash {got:#018x}. Update \
         V3_GOLDEN, bump V3_GOLDEN_REVISION and biome::V3_REVISION"
    );
}

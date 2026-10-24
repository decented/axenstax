//! Terrain v4 golden + order independence (the v2/v3 pattern in
//! `worldgen_golden.rs` and `worldgen_v4_golden.rs`, which stay pinned to v2
//! and v3).
//!
//! v4 = v3 + the dry caves of design §7. A joiner regenerates a v4 host's
//! untouched columns from the seed, so v4 output must be a pure function of the
//! seed: the same blocks in any column
//! order, on every machine. The golden hashes every cell y 0..=255 of a
//! normal-terrain patch at the origin plus a mountain patch (peaks above y
//! 150, so the 3D overhangs and the tall sections are covered).
//!
//! v4 is UNFROZEN and unreleased (`biome::V4_REVISION` 0): v4 will also get the
//! §9-§10 trees and spawn before it ships. Any change to v4 output bumps
//! `V4_REVISION` and `V4_GOLDEN_REVISION` and re-pins `V4_GOLDEN`. (Re-pinning
//! the v2 or v3 golden is never the answer: those generators are frozen, and
//! `v4_matches_v4_wherever_no_cave_is_carved` shows what a v4 cave changes.)

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
    let bg = BiomeGenerator::new(SEED, Generator::V4);
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

/// The pinned v4 output. When the v4 golden changes, bump this AND
/// biome::V4_REVISION to match (`v4_golden_revision_matches_the_fingerprint_revision`).
const V4_GOLDEN: u64 = 0x1c60_e89f_de18_ad0a;
const V4_GOLDEN_REVISION: u32 = 0;

#[test]
fn v4_terrain_does_not_depend_on_column_order() {
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
fn v4_golden_set_reaches_the_mountains() {
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
        let t = BiomeGenerator::new(SEED, Generator::V4);
        let t = t.v3().unwrap();
        let found = (0..400i32).find_map(|r| {
            (-r..=r).flat_map(|a| [(a, -r), (a, r), (-r, a), (r, a)]).find(|&(x, z)| t.height2d(x * 16 + 8, z * 16 + 8) > 150.0)
        });
        panic!("only {tall} columns of the mountain chunk reach y 150; nearest tall chunk: {found:?}");
    }
}

/// A re-pinned golden without a `V4_REVISION` bump would let peers with
/// different v4 output trust each other's terrain (Spec 02 §5.2).
#[test]
fn v4_golden_revision_matches_the_fingerprint_revision() {
    assert_eq!(
        crate::biome::V4_REVISION,
        V4_GOLDEN_REVISION,
        "bump biome::V4_REVISION and V4_GOLDEN_REVISION together"
    );
}

#[test]
fn v4_worldgen_output_matches_golden() {
    let got = hash(&generate(&columns()), &columns());
    assert_eq!(got, hash(&generate(&columns()), &columns()), "v4 is not deterministic within a process");
    assert_eq!(
        got, V4_GOLDEN,
        "v4 worldgen output changed (revision {V4_GOLDEN_REVISION}): new hash {got:#018x}. Update \
         V4_GOLDEN, bump V4_GOLDEN_REVISION and biome::V4_REVISION"
    );
}

//! Worldgen golden + determinism (gap-audit T2-9).
//!
//! A joiner generates the host's terrain locally from the seed and the world
//! flags in `JoinAcceptPacket`, so the generator must give the same blocks for
//! the same seed + flags on every machine, in every run, in any column order.
//! `WORLDGEN_VERSION` (world.rs) names the generator's output; the golden hash
//! below pins it, so a change to generation output fails here until the
//! version is bumped (joiners on another version then get a toast, Spec 02).
//!
//! Scope: the golden set is generated in a FIXED order. Worldgen is not yet a
//! pure function of seed + flags (Phase B0 — see `WORLDGEN_VERSION`'s doc:
//! reserve richness, hideouts reading `village_anchors` in generation order,
//! the bundled plan registry, cross-platform trig rounding). The column-order
//! test below covers terrain, caves, ore, trees and vegetation; the set is not
//! chosen to include a village or hideout, so it does not prove structure
//! placement is order-independent.

use crate::biome::BiomeGenerator;
use crate::chunk::CHUNK_SIZE;
use crate::world::{World, MAX_CHUNK_Y, WORLDGEN_VERSION};

const SEED: u32 = 20_261_006;

/// Normal-terrain columns. Wide enough that every column carries caves and
/// ore under its surface; the far pair reaches a different biome.
fn normal_columns() -> Vec<(i32, i32)> {
    let mut cols = Vec::new();
    for cx in -2..2 {
        for cz in -2..2 {
            cols.push((cx, cz));
        }
    }
    cols.push((40, -37));
    cols.push((41, -37));
    cols
}

/// FNV-1a over every cell of `cols`, in a fixed coordinate order (never the
/// chunk map's iteration order): block id, then the per-cell meta byte.
fn hash_columns(w: &World, cols: &[(i32, i32)], h: &mut u64) {
    let cs = CHUNK_SIZE as i32;
    let top = (MAX_CHUNK_Y + 1) * cs;
    let mut eat = |b: u8| {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x0000_0100_0000_01B3);
    };
    for &(cx, cz) in cols {
        for x in cx * cs..(cx + 1) * cs {
            for z in cz * cs..(cz + 1) * cs {
                for y in 0..top {
                    let id = w.get_block(x, y, z);
                    for b in id.to_le_bytes() {
                        eat(b);
                    }
                    eat(w.block_meta.get(&(x, y, z)).copied().unwrap_or(0));
                }
            }
        }
    }
}

fn generate(w: &mut World, cols: &[(i32, i32)], bg: &BiomeGenerator) {
    for &(cx, cz) in cols {
        w.generate_column(cx, cz, bg);
    }
}

/// The whole golden set: normal terrain plus the flag-driven presets a joiner
/// must reproduce (flat grass, flat water, the Workshop void).
fn golden_hash(reverse: bool) -> u64 {
    let bg = BiomeGenerator::new(SEED);
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;

    let mut cols = normal_columns();
    if reverse {
        cols.reverse();
    }
    let mut normal = World::new();
    generate(&mut normal, &cols, &bg);
    hash_columns(&normal, &normal_columns(), &mut h);

    let flat_cols = [(0, 0), (3, -1)];
    let mut flat = World::new();
    flat.world_type = "flat".to_string();
    flat.ground = "grass".to_string();
    generate(&mut flat, &flat_cols, &bg);
    hash_columns(&flat, &flat_cols, &mut h);

    let mut water = World::new();
    water.world_type = "flat".to_string();
    water.ground = "water".to_string();
    water.water_depth = 3;
    generate(&mut water, &flat_cols, &bg);
    hash_columns(&water, &flat_cols, &mut h);

    let mut void = World::new();
    void.is_workshop = true;
    generate(&mut void, &flat_cols, &bg);
    hash_columns(&void, &flat_cols, &mut h);

    h
}

/// The pinned output of `WORLDGEN_VERSION`. Update ONLY together with a bump
/// of `WORLDGEN_VERSION` (and a line in Spec 02's worldgen version log).
const GOLDEN: u64 = 0x59ac_c87a_fa22_759c;
const GOLDEN_VERSION: u32 = 1;

#[test]
fn worldgen_is_deterministic_within_a_process() {
    assert_eq!(
        golden_hash(false),
        golden_hash(false),
        "worldgen is NOT deterministic: the same seed + flags gave different blocks twice"
    );
}

#[test]
fn worldgen_does_not_depend_on_column_order() {
    // Host and joiner stream columns in different orders (each from its own
    // spawn), so the order must not change a single block.
    assert_eq!(
        golden_hash(false),
        golden_hash(true),
        "worldgen output depends on the order columns are generated in"
    );
}

#[test]
fn worldgen_output_matches_golden_for_this_version() {
    assert_eq!(
        WORLDGEN_VERSION, GOLDEN_VERSION,
        "WORLDGEN_VERSION changed: regenerate GOLDEN for the new version"
    );
    let got = golden_hash(false);
    assert_eq!(
        got, GOLDEN,
        "worldgen output changed: bump WORLDGEN_VERSION and update this hash \
         (new hash {got:#018x})"
    );
}

//! Worldgen golden + purity (gap-audit T2-9, Phase B0).
//!
//! A joiner generates the host's untouched terrain locally from the seed and
//! the world flags in `JoinAcceptPacket`, so generation must be a pure
//! function of (seed, world flags, `worldgen_fingerprint()`): the same blocks
//! on every machine, in every run, in any column order. `WORLDGEN_VERSION`
//! (world.rs) names the generator's output; the golden hash below pins it, so
//! a change to generation output fails here until the version is bumped
//! (joiners on another fingerprint then get a toast, Spec 02 §5.2).
//!
//! Two column sets, each generated in three orders (row-major, reversed,
//! scattered) into fresh `World`s and compared cell by cell (block id + meta)
//! plus every side table generation writes:
//! - **terrain** (seed `SEED`): caves, ore, trees and vegetation across two
//!   biomes, plus the flat-grass / flat-water / Workshop-void presets in the
//!   golden hash;
//! - **structures** (seed `STRUCTURE_SEED`): a village V, a Brigand Hideout H
//!   that places, and a hideout candidate R whose anchor is 108 blocks from V,
//!   so the village-distance gate must reject it. Before B0 the gate read
//!   `World::village_anchors`, which only fills as village columns generate:
//!   generating R's columns before V's placed R (183 cells differed).

use crate::biome::BiomeGenerator;
use crate::chunk::CHUNK_SIZE;
use crate::plan_registry::PlanRegistry;
use crate::world::{worldgen_fingerprint, worldgen_fingerprint_of, World, MAX_CHUNK_Y, WORLDGEN_VERSION};

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

/// Found once by `find_seed_with_village_and_hideouts` (below, `#[ignore]`),
/// which lists seeds in 0..3000 with a village in cells -2..=2, a hideout site
/// in cells -1..=1 between 80 and 127 blocks (Chebyshev) from that village,
/// and another hideout that places. 2042 was picked from its hits because all
/// three patches sit within ~1000 blocks of the origin. Seed 2042:
/// - village V, cell (0, -1), anchor (301, 64, -224), chunk (18, -14);
/// - hideout candidate R, cell (0, -1), anchor (315, 64, -332), 108 blocks
///   from V, so it must NOT place; chunk (19, -21);
/// - hideout H, cell (0, 0), anchor (931, 64, 22), places; chunk (58, 1).
const STRUCTURE_SEED: u32 = 2042;
const V_CHUNK: (i32, i32) = (18, -14);
const V_CELL: (i32, i32) = (0, -1);
const R_CHUNK: (i32, i32) = (19, -21);
const R_CELL: (i32, i32) = (0, -1);
const H_CHUNK: (i32, i32) = (58, 1);
const H_CELL: (i32, i32) = (0, 0);

fn patch(centre: (i32, i32), r: i32, out: &mut Vec<(i32, i32)>) {
    for cx in centre.0 - r..=centre.0 + r {
        for cz in centre.1 - r..=centre.1 + r {
            if !out.contains(&(cx, cz)) {
                out.push((cx, cz));
            }
        }
    }
}

/// V's whole footprint (houses reach ~22 blocks, workshops ~32, from the
/// anchor), then R's and H's (palisade + banner reach 7 blocks). R comes
/// after V, so the reversed order generates R before any column of V.
fn structure_columns() -> Vec<(i32, i32)> {
    let mut cols = Vec::new();
    patch(V_CHUNK, 3, &mut cols);
    patch(R_CHUNK, 1, &mut cols);
    patch(H_CHUNK, 1, &mut cols);
    cols
}

/// The orders a column set is generated in: as listed, reversed, and
/// scattered (sorted by a coordinate hash, so neighbours are far apart in time).
fn orders(cols: &[(i32, i32)]) -> Vec<(&'static str, Vec<(i32, i32)>)> {
    let mut reversed = cols.to_vec();
    reversed.reverse();
    let mut scattered = cols.to_vec();
    scattered.sort_by_key(|&(x, z)| {
        (x as u32).wrapping_mul(0x9E37_79B9) ^ (z as u32).wrapping_mul(0x85EB_CA6B)
    });
    vec![("row-major", cols.to_vec()), ("reversed", reversed), ("scattered", scattered)]
}

fn generate(w: &mut World, cols: &[(i32, i32)], bg: &BiomeGenerator) {
    for &(cx, cz) in cols {
        w.generate_column(cx, cz, bg);
    }
}

fn sorted<K: std::fmt::Debug, V: std::fmt::Debug>(it: impl Iterator<Item = (K, V)>) -> Vec<String> {
    let mut v: Vec<String> = it.map(|(k, v)| format!("{k:?}={v:?}")).collect();
    v.sort();
    v
}

/// Every `World` side table generation writes, in a canonical (sorted) form.
fn side_tables(w: &World) -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("block_meta", sorted(w.block_meta.iter())),
        ("block_entities", sorted(w.block_entities.iter())),
        ("architect_plaques", sorted(w.architect_plaques.iter())),
        ("procgen_plaque_sources", sorted(w.procgen_plaque_sources.iter().map(|p| (p, ())))),
        ("brigand_hideouts", sorted(w.brigand_hideouts.iter())),
        ("village_anchors", sorted(w.village_anchors.iter())),
    ]
}

/// Bit-identical blocks over `cols` and identical side tables, or a failure
/// naming the first differing cell / table.
fn assert_same_world(a: &World, b: &World, cols: &[(i32, i32)], what: &str) {
    let cs = CHUNK_SIZE as i32;
    let top = (MAX_CHUNK_Y + 1) * cs;
    let mut diffs = 0usize;
    let mut first = None;
    for &(cx, cz) in cols {
        for x in cx * cs..(cx + 1) * cs {
            for z in cz * cs..(cz + 1) * cs {
                for y in 0..top {
                    let (ia, ib) = (a.get_block(x, y, z), b.get_block(x, y, z));
                    if ia != ib {
                        diffs += 1;
                        first.get_or_insert((x, y, z, ia, ib));
                    }
                }
            }
        }
    }
    assert_eq!(diffs, 0, "{what}: {diffs} cells differ; first (x, y, z, a, b) = {first:?}");
    for ((name, ta), (_, tb)) in side_tables(a).into_iter().zip(side_tables(b)) {
        assert_eq!(ta, tb, "{what}: World::{name} differs");
    }
}

/// Generate `cols` in every order into fresh worlds; all must match row-major.
fn assert_order_independent(cols: &[(i32, i32)], bg: &BiomeGenerator) {
    let mut reference = None;
    for (name, order) in orders(cols) {
        let mut w = World::new();
        generate(&mut w, &order, bg);
        match &reference {
            None => reference = Some(w),
            Some(r) => assert_same_world(r, &w, cols, &format!("{name} order vs row-major")),
        }
    }
}

/// FNV-1a over every cell of `cols`, in a fixed coordinate order (never the
/// chunk map's iteration order): block id, then the per-cell meta byte.
fn hash_columns(w: &World, cols: &[(i32, i32)], h: &mut u64) {
    let cs = CHUNK_SIZE as i32;
    let top = (MAX_CHUNK_Y + 1) * cs;
    for &(cx, cz) in cols {
        for x in cx * cs..(cx + 1) * cs {
            for z in cz * cs..(cz + 1) * cs {
                for y in 0..top {
                    let id = w.get_block(x, y, z);
                    for b in id.to_le_bytes() {
                        fnv(h, b);
                    }
                    fnv(h, w.block_meta.get(&(x, y, z)).copied().unwrap_or(0));
                }
            }
        }
    }
}

fn fnv(h: &mut u64, b: u8) {
    *h ^= b as u64;
    *h = h.wrapping_mul(0x0000_0100_0000_01B3);
}

/// Positions (not contents) of the structure side tables: which block
/// entities, plaques, hideouts and villages exist where. Contents are covered
/// by the order tests; hashing their `Debug` would tie the golden to unrelated
/// refactors of e.g. `ItemStack`.
fn hash_structure_positions(w: &World, h: &mut u64) {
    let mut keys: Vec<String> = Vec::new();
    keys.extend(w.block_entities.keys().map(|k| format!("be{k:?}")));
    keys.extend(w.architect_plaques.keys().map(|k| format!("plaque{k:?}")));
    keys.extend(w.brigand_hideouts.keys().map(|k| format!("hideout{k:?}")));
    keys.extend(w.village_anchors.iter().map(|(k, v)| format!("village{k:?}{v:?}")));
    keys.sort();
    for k in keys {
        for b in k.bytes() {
            fnv(h, b);
        }
    }
}

/// The whole golden set: normal terrain, the flag-driven presets a joiner
/// must reproduce (flat grass, flat water, the Workshop void), and the
/// village + hideout structure set.
fn golden_hash() -> u64 {
    let bg = BiomeGenerator::new(SEED);
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;

    let mut normal = World::new();
    generate(&mut normal, &normal_columns(), &bg);
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

    let sbg = BiomeGenerator::new(STRUCTURE_SEED);
    let mut structures = World::new();
    generate(&mut structures, &structure_columns(), &sbg);
    hash_columns(&structures, &structure_columns(), &mut h);
    hash_structure_positions(&structures, &mut h);

    h
}

/// First 8 bytes (LE) of the bundled plan registry's content hash.
fn plans_hash() -> u64 {
    let d = PlanRegistry::bundled().content_hash();
    u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
}

/// The pinned output of `WORLDGEN_VERSION` with the bundled plans hashed as
/// `GOLDEN_PLANS`. Update ONLY together with a bump of `WORLDGEN_VERSION` (and
/// a row in Spec 02 §5.2's version log) — or, when only the bundled plans
/// changed, together with `GOLDEN_PLANS` (the fingerprint covers that).
const GOLDEN: u64 = 0x0f88_b2cc_a767_ee72;
const GOLDEN_VERSION: u32 = 2;
const GOLDEN_PLANS: u64 = 0x0e0e_c249_b9fa_f9b8;

#[test]
fn worldgen_is_deterministic_within_a_process() {
    assert_eq!(
        golden_hash(),
        golden_hash(),
        "worldgen is NOT deterministic: the same seed + flags gave different blocks twice"
    );
}

#[test]
fn terrain_does_not_depend_on_column_order() {
    // Host and joiner stream columns in different orders (each from its own
    // spawn), so the order must not change a single block.
    assert_order_independent(&normal_columns(), &BiomeGenerator::new(SEED));
}

#[test]
fn structures_do_not_depend_on_column_order() {
    assert_order_independent(&structure_columns(), &BiomeGenerator::new(STRUCTURE_SEED));
}

#[test]
fn structure_set_holds_a_village_a_placed_hideout_and_a_rejected_one() {
    // The premise the order test relies on. If generation changes so this
    // seed no longer has them, re-run `find_seed_with_village_and_hideouts`
    // and re-pin the constants above.
    let mut w = World::new();
    generate(&mut w, &structure_columns(), &BiomeGenerator::new(STRUCTURE_SEED));
    assert!(w.village_anchors.contains_key(&V_CELL), "village V missing");
    assert!(w.brigand_hideouts.contains_key(&H_CELL), "hideout H missing");
    assert!(
        !w.brigand_hideouts.contains_key(&R_CELL),
        "hideout R sits 108 blocks from village V and must be rejected"
    );
    assert!(!w.procgen_plaque_sources.is_empty(), "V's houses should come from bundled plans");
}

#[test]
fn worldgen_ignores_the_worlds_runtime_plan_registry() {
    // `/importschem` adds plans to `World::plan_registry` at runtime; village
    // generation must not see them (it samples `PlanRegistry::bundled()`).
    let bg = BiomeGenerator::new(STRUCTURE_SEED);
    let mut cols = Vec::new();
    patch(V_CHUNK, 3, &mut cols);
    let mut bundled = World::new();
    bundled.load_bundled_plans();
    generate(&mut bundled, &cols, &bg);
    let mut emptied = World::new();
    emptied.plan_registry = PlanRegistry::new();
    generate(&mut emptied, &cols, &bg);
    assert_same_world(&bundled, &emptied, &cols, "runtime plan registry");
}

#[test]
fn worldgen_fingerprint_folds_version_and_bundled_plans() {
    let plans = PlanRegistry::bundled().content_hash();
    assert_eq!(worldgen_fingerprint(), worldgen_fingerprint_of(WORLDGEN_VERSION, &plans));
    assert_ne!(
        worldgen_fingerprint_of(WORLDGEN_VERSION, &plans),
        worldgen_fingerprint_of(WORLDGEN_VERSION + 1, &plans),
        "a generator version bump must change the fingerprint"
    );
    let mut other = plans;
    other[0] ^= 1;
    assert_ne!(
        worldgen_fingerprint_of(WORLDGEN_VERSION, &plans),
        worldgen_fingerprint_of(WORLDGEN_VERSION, &other),
        "a bundled-plan change must change the fingerprint"
    );
    assert_ne!(worldgen_fingerprint(), 0, "0 is what a peer that sent nothing decodes as");
}

#[test]
fn worldgen_output_matches_golden_for_this_version() {
    assert_eq!(
        WORLDGEN_VERSION, GOLDEN_VERSION,
        "WORLDGEN_VERSION changed: regenerate GOLDEN (and GOLDEN_PLANS) for the new version"
    );
    let got = golden_hash();
    let plans = plans_hash();
    assert_eq!(
        plans, GOLDEN_PLANS,
        "the bundled plan registry changed (plans hash {plans:#018x}). worldgen_fingerprint() \
         already covers that, so no WORLDGEN_VERSION bump for it: update GOLDEN_PLANS and \
         GOLDEN (new hash {got:#018x}). Bump the version too only if generator code changed."
    );
    assert_eq!(
        got, GOLDEN,
        "worldgen output changed with the same bundled plans: bump WORLDGEN_VERSION, add a \
         Spec 02 §5.2 version-log row, and update this hash (new hash {got:#018x})"
    );
}

/// One-off seed search (B0) that found `STRUCTURE_SEED`: a village V in cells
/// -2..=2, a hideout site R in cells -1..=1 between 80 and 127 blocks from V
/// (far enough that R's patch doesn't overlap V's), and a hideout H that
/// places. Run with `cargo test --lib find_seed_with_village_and_hideouts --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn find_seed_with_village_and_hideouts() {
    use crate::brigand_hideout_gen::{hideout_site, layout_for_hideout_cell};
    use crate::village_gen::village_site;
    for seed in 0u32..3000 {
        let bg = BiomeGenerator::new(seed);
        let mut villages = Vec::new();
        for gx in -2..=2 {
            for gz in -2..=2 {
                if let Some(v) = village_site(seed, gx, gz, &bg) {
                    villages.push(((gx, gz), v));
                }
            }
        }
        let mut r = None;
        let mut h = None;
        for hx in -1..=1 {
            for hz in -1..=1 {
                let Some([ax, _, az]) = hideout_site(seed, hx, hz, &bg) else { continue };
                let near = villages
                    .iter()
                    .map(|(c, v)| (c, v, (v[0] - ax).abs().max((v[2] - az).abs())))
                    .filter(|&(_, _, d)| (80..128).contains(&d))
                    .min_by_key(|&(_, _, d)| d);
                if let Some((vc, v, d)) = near
                    && r.is_none()
                {
                    r = Some(((hx, hz), [ax, az], *vc, *v, d));
                }
                if h.is_none() && layout_for_hideout_cell(seed, hx, hz, &bg).is_some() {
                    h = Some(((hx, hz), [ax, az]));
                }
            }
        }
        if let (Some(r), Some(h)) = (r, h) {
            println!(
                "seed {seed}: R cell {:?} anchor {:?}, {} blocks from village cell {:?} anchor {:?}; \
                 H cell {:?} anchor {:?}",
                r.0, r.1, r.4, r.2, r.3, h.0, h.1
            );
        }
    }
}

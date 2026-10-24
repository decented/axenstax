//! Terrain T1 (2026-10-23) — the world is 256 blocks tall (sections cy
//! 0..=15) and every world records its terrain generator. An old (v2) column
//! still holds and costs only its old six sections; a block built high
//! survives everything a block does: set/get, light, save → load, and a push
//! to a joiner. Spec 02 §3.1 / §5.2.

#![cfg(not(target_arch = "wasm32"))]

use crate::biome::{BiomeGenerator, Generator};
use crate::block::{self, BlockRegistry};
use crate::chunk_intake::ChunkIntake;
use crate::protocol;
use crate::save;
use crate::world::{World, MAX_CHUNK_Y, V2_WORLD_TOP, WORLD_HEIGHT};

const HIGH: i32 = 200; // cy 12

/// A generated, lit v2 column (0, 0), as single-player streams it.
fn lit_v2_column(seed: u32) -> (World, BlockRegistry) {
    let reg = BlockRegistry::new();
    let mut w = World::new();
    w.generate_column(0, 0, &BiomeGenerator::new(seed, Generator::V2));
    crate::lighting::run_initial_pass_for_column(&mut w, 0, 0, &reg);
    (w, reg)
}

fn sections(w: &World, cx: i32, cz: i32) -> Vec<i32> {
    (0..=MAX_CHUNK_Y).filter(|&cy| w.has_chunk(cx, cy, cz)).collect()
}

#[test]
fn the_world_is_256_tall() {
    assert_eq!(MAX_CHUNK_Y, 15);
    assert_eq!(WORLD_HEIGHT, 256);
}

#[test]
fn a_lit_v2_column_holds_only_the_old_six_sections() {
    // Generation never fills above the generator's height, and the light pass
    // conjures nothing above the old roof: an old world costs what it did.
    for seed in [7, 42, 1337] {
        let (w, _) = lit_v2_column(seed);
        let s = sections(&w, 0, 0);
        assert!(s.iter().all(|&cy| cy <= V2_WORLD_TOP / 16), "seed {seed}: {s:?}");
        assert_eq!(w.light_top(0, 0), V2_WORLD_TOP);
        assert_eq!(w.sky_light_at(3, HIGH, 3), 15, "absent sections are open sky");
    }
}

#[test]
fn a_block_at_y200_is_set_lit_and_read_back() {
    let (mut w, reg) = lit_v2_column(7);
    w.place_player_block(3, HIGH, 3, block::STONE);
    crate::lighting::update_for_block_change(&mut w, (3, HIGH, 3), block::AIR, block::STONE, &reg);
    assert_eq!(w.get_block(3, HIGH, 3), block::STONE);
    assert!(w.is_placed(3, HIGH, 3));
    assert!(w.has_chunk(0, 12, 0), "its section exists");
    assert_eq!(w.column_top_y(3, 3), 207, "the column's top follows the block");
    assert_eq!(w.light_top(0, 0), 207);
    assert_eq!(w.sky_light_at(3, HIGH + 1, 3), 15, "open sky above it");
    assert_eq!(w.sky_light_at(4, HIGH, 3), 15, "and beside it");
    assert!(w.sky_light_at(3, HIGH - 1, 3) < 15, "its shadow");
    assert!(w.sky_light_at(3, HIGH - 1, 3) > 0, "lit from the side");
    // Breaking it restores open sky.
    w.set_block(3, HIGH, 3, block::AIR);
    crate::lighting::update_for_block_change(&mut w, (3, HIGH, 3), block::STONE, block::AIR, &reg);
    assert_eq!(w.sky_light_at(3, HIGH - 1, 3), 15);
}

#[test]
fn a_torch_in_open_air_above_the_terrain_is_not_in_a_dark_box() {
    // The section a write conjures above a column's sections starts as open
    // sky (what it read as a moment before), so a non-opaque block placed
    // there — which runs no sky pass — leaves its surroundings lit.
    let (mut w, reg) = lit_v2_column(7);
    w.set_block(5, 150, 5, block::TORCH);
    crate::lighting::update_for_block_change(&mut w, (5, 150, 5), block::AIR, block::TORCH, &reg);
    assert!(w.has_chunk(0, 9, 0));
    assert_eq!(w.sky_light_at(6, 150, 5), 15);
    assert_eq!(w.sky_light_at(5, 145, 5), 15);
    assert!(w.block_light_at(6, 150, 5) > 0, "the torch lights its neighbour");
    // Its block light spilling into the absent section above conjures that
    // one open-sky too.
    assert_eq!(w.sky_light_at(5, 160, 5), 15);
}

#[test]
fn a_platform_built_above_the_terrain_shades_what_is_under_it() {
    // Sections conjured above the column start as open sky; the column's
    // light pass must still darken what an opaque roof covers there.
    let (mut w, reg) = lit_v2_column(7);
    for x in 4..=12 {
        for z in 4..=12 {
            w.set_block(x, 150, z, block::STONE);
        }
    }
    crate::lighting::run_initial_pass_for_column(&mut w, 0, 0, &reg);
    assert_eq!(w.sky_light_at(8, 151, 8), 15, "open sky on top");
    let under = w.sky_light_at(8, 149, 8);
    assert!(under < 15 && under > 0, "shaded, lit from the sides: {under}");
}

#[test]
fn a_block_at_y200_survives_save_and_load_with_the_worlds_generator() {
    let name = format!("__t1_tall_save_{}__", std::process::id());
    let _ = save::delete_world(&name);
    let (mut w, _) = lit_v2_column(7);
    w.generator = Generator::V3;
    w.place_player_block(3, HIGH, 3, block::STONE);
    let slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(3.5, 201.0, 3.5), 1.0);
    save::save_world(&name, &w, &[slot], 7, &[], &[], None).expect("save_world");

    let mut loaded = World::new();
    let (saved, _) = save::load_world(&name, &mut loaded).expect("load_world");
    assert_eq!(loaded.get_block(3, HIGH, 3), block::STONE);
    assert!(loaded.is_placed(3, HIGH, 3));
    assert_eq!(saved.worldgen_version, Some(3), "world.dat records the generator");
    let _ = save::delete_world(&name);
}

#[test]
fn a_block_at_y200_is_pushed_to_a_joiner() {
    let (mut host, reg) = lit_v2_column(7);
    host.place_player_block(3, HIGH, 3, block::DIAMOND_BLOCK);
    let mut joiner = World::new();
    let mut loaded = ahash::AHashSet::new();
    let mut intake = ChunkIntake::default();
    for cy in 0..=MAX_CHUNK_Y {
        for raw in crate::chunk_push::build_chunk_packets(&host, (0, cy, 0)) {
            let (_, payload) = protocol::deserialize_header(&raw).unwrap();
            let pkt: protocol::ChunkDataPacket = protocol::safe_deserialize(payload).unwrap();
            intake.apply(&mut joiner, &mut loaded, &reg, &pkt);
        }
    }
    assert!(loaded.contains(&(0, 0)), "the whole 16-chunk column arrived");
    assert_eq!(joiner.get_block(3, HIGH, 3), block::DIAMOND_BLOCK, "cy 12 reached the joiner");
    assert!(joiner.has_chunk(0, 12, 0));
    // The all-air sections between are left absent, not stored empty.
    assert!(!joiner.has_chunk(0, 8, 0));
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..WORLD_HEIGHT {
                assert_eq!(joiner.get_block(x, y, z), host.get_block(x, y, z), "({x},{y},{z})");
            }
        }
    }
}

/// Terrain T1 review S3: a world that lost its `world_meta.json` takes its
/// generator from the one `world.dat` recorded, not a hard-coded v2.
#[test]
fn a_world_with_no_meta_takes_its_generator_from_world_dat() {
    let _root = save::WorldsRootGuard::new("t1-metaless-generator");
    let make = |name: &str, recorded: Option<u32>| {
        let mut dat = save::minimal_world_save_for_tests(5);
        dat.worldgen_version = recorded;
        save::write_world_folder(name, &save::WorldMeta::new(name), &dat, &World::new()).unwrap();
        std::fs::remove_file(save::world_dir(name).join("world_meta.json")).unwrap();
    };

    make("recorded-v3", Some(3));
    assert_eq!(save::peek_world_meta("recorded-v3").unwrap().generator(), Ok(Generator::V3));
    assert_eq!(save::load_world_meta("recorded-v3").generator(), Ok(Generator::V3));

    make("recorded-v2", Some(2));
    assert_eq!(save::peek_world_meta("recorded-v2").unwrap().generator(), Ok(Generator::V2));

    make("pre-field", None);
    assert_eq!(save::peek_world_meta("pre-field").unwrap().generator(), Ok(Generator::V2), "an old save is v2");

    // A generator this build lacks refuses the world: no guessing v2.
    // (One past the newest generator, so it stays unknown as generators are added.)
    let unknown = Generator::ALL.iter().map(|g| g.version()).max().unwrap() + 1;
    let name = format!("recorded-v{unknown}");
    make(&name, Some(unknown));
    let err = save::peek_world_meta(&name).expect_err("an unknown generator must refuse");
    assert!(err.contains(&format!("generator v{unknown}")), "{err}");
}

#[test]
fn a_new_world_is_v3_and_an_old_one_v2() {
    let fresh = save::WorldMeta::new("fresh");
    assert_eq!(fresh.worldgen_version, crate::biome::NEW_WORLD_GENERATOR);
    assert_eq!(fresh.generator(), Ok(Generator::V3));
    // A meta written before the field.
    let mut json = serde_json::to_value(&fresh).unwrap();
    json.as_object_mut().unwrap().remove("worldgen_version");
    let old: save::WorldMeta = serde_json::from_value(json).unwrap();
    assert_eq!(old.worldgen_version, 2);
    assert_eq!(old.generator(), Ok(Generator::V2));
}

#[test]
fn a_world_on_an_unknown_generator_is_refused_at_load() {
    let name = format!("__t1_unknown_gen_{}__", std::process::id());
    let _ = save::delete_world(&name);
    let mut meta = save::WorldMeta::new(&name);
    meta.worldgen_version = 99;
    save::save_world_meta(&name, &meta).expect("meta written");
    let why = save::peek_world_meta(&name).expect_err("refused, not defaulted");
    assert!(why.contains("v99"), "{why}");
    let mut w = World::new();
    assert!(save::load_world(&name, &mut w).is_err(), "the world does not open");
    let _ = save::delete_world(&name);
}

/// Column (0, 0) roofed at y 150 over x 8..=15 with a wall at x 8, so the
/// space under the roof opens only onto column (1, 0) across x = 15 | 16.
fn roof_over_the_border(w: &mut World) {
    for z in 0..16 {
        for x in 8..16 {
            w.set_block(x, 150, z, block::STONE);
        }
        for y in V2_WORLD_TOP + 1..150 {
            w.set_block(8, y, z, block::STONE);
        }
    }
}

#[test]
fn sky_reaches_an_overhang_above_the_old_roof_from_a_lower_neighbour() {
    // Terrain v3 lighting gap: the neighbour's sections stop at y 95, so the
    // open sky beside the overhang is absent sections — read as sky, never a
    // light source — and the overhang stayed dark. Both lighting orders.
    let reg = BlockRegistry::new();
    let bg = BiomeGenerator::new(7, Generator::V2);
    for neighbour_first in [true, false] {
        let mut w = World::new();
        w.generate_column(0, 0, &bg);
        if neighbour_first {
            w.generate_column(1, 0, &bg);
            crate::lighting::run_initial_pass_for_column(&mut w, 1, 0, &reg);
        }
        roof_over_the_border(&mut w);
        crate::lighting::run_initial_pass_for_column(&mut w, 0, 0, &reg);
        if !neighbour_first {
            assert_eq!(w.sky_light_at(15, 120, 8), 0, "no lit neighbour yet: dark under the roof");
            w.generate_column(1, 0, &bg);
            crate::lighting::run_initial_pass_for_column(&mut w, 1, 0, &reg);
        }
        assert_eq!(w.sky_light_at(16, 120, 8), 15, "the neighbour's open sky");
        assert_eq!(w.sky_light_at(15, 120, 8), 14, "neighbour first: {neighbour_first}");
        assert_eq!(w.sky_light_at(12, 120, 8), 11, "and on inwards");
        assert_eq!(w.sky_light_at(15, 151, 8), 15, "open sky on the roof");
        assert!(!w.has_chunk(1, 7, 0), "nothing conjured in the neighbour");
    }
}

#[test]
fn an_ungenerated_neighbour_lends_no_sky() {
    // An absent neighbour column may yet generate solid there.
    let reg = BlockRegistry::new();
    let mut w = World::new();
    w.generate_column(0, 0, &BiomeGenerator::new(7, Generator::V2));
    roof_over_the_border(&mut w);
    crate::lighting::run_initial_pass_for_column(&mut w, 0, 0, &reg);
    assert_eq!(w.sky_light_at(15, 120, 8), 0);
}

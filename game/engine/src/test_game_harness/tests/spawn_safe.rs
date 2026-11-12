//! A new world on a seed whose origin is open sea, through the real game
//! (ignored: needs a GPU adapter holding the atlas): the player's start is the
//! respawn point, a death at sea respawns on dry ground, a save with the
//! legacy default spawn loads repaired, and Satoshi's house is built on dry
//! land and meshed the moment it is built.
//! Run: cargo test --lib -- --ignored game_harness_spawn
use super::*;
use crate::block;
use crate::spawn_safety::{is_dry_standing, LEGACY_DEFAULT_SPAWN};
use glam::Vec3;

/// A headless game whose next world, `name`, is a new v3 world on `seed` (the
/// seed the lobby would have read from the world's meta), Satoshi `satoshi`.
/// Each test names its own world: they share one saves folder, and a world a
/// sibling saved is a saved world, not a new one.
fn boot_new_world(name: &str, seed: u32, satoshi: bool) -> HeadlessGame {
    let _ = crate::save::delete_world(name);
    let state = pollster::block_on(crate::GameState::new_headless_routed(
        1280,
        720,
        crate::local_routing::LocalRouting::Standalone,
    ));
    let mut hg = HeadlessGame { state };
    let generator = crate::biome::Generator::V3;
    hg.state.biome_gen = crate::biome::BiomeGenerator::new(seed, generator);
    hg.state.world.generator = generator;
    hg.state.world.satoshi.enabled = satoshi;
    hg.enter_world(name);
    hg
}

fn settle(hg: &mut HeadlessGame) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        hg.frames(3);
        if hg.state.dirty_mesh_chunks.is_empty() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "meshes never settled");
    }
    hg.frames(3);
}

fn dry(hg: &HeadlessGame, p: Vec3) -> bool {
    is_dry_standing(&hg.state.world, &hg.state.registry, p)
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness"]
fn game_harness_spawn_a_sea_origin_world_starts_and_respawns_on_dry_ground() {
    super::isolate_saves();
    let seed = crate::spawn_search::sea_origin_seed();
    let mut hg = boot_new_world("harness-spawn-respawn", seed, false);

    // The start is the respawn point, and it is dry ground, not the legacy default.
    let start = hg.state.players[0].player.pos;
    assert_eq!(hg.state.players[0].spawn_pos, start, "seed {seed}: the new world's spawn point is its start");
    assert_ne!(hg.state.players[0].spawn_pos.to_array(), LEGACY_DEFAULT_SPAWN);
    assert!(dry(&hg, start), "seed {seed}: the start {start:?} is dry standing ground");

    // Die out at sea over the origin, then choose Respawn: back on the dry start.
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    hg.state.players[0].player.pos = Vec3::new(0.5, 70.0, 0.5);
    hg.state.players[0].combat.die(crate::survival::DamageCause::Generic);
    hg.ticks(2);
    assert!(hg.state.players[0].combat.request_respawn());
    hg.ticks(2);
    assert!(!hg.state.players[0].combat.dead, "respawned");
    let at = hg.state.players[0].player.pos;
    assert!(dry(&hg, at), "seed {seed}: respawned at {at:?}, not on dry ground");
    assert_eq!(at, start, "a no-bed respawn is the new world's start");
    assert_eq!(hg.state.players[0].spawn_pos, start, "and the saved spawn point is left as it was");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness"]
fn game_harness_spawn_a_legacy_default_spawn_is_repaired_on_load_and_a_chosen_one_is_not() {
    super::isolate_saves();
    let seed = crate::spawn_search::sea_origin_seed();
    let name = "harness-spawn-repair";
    let mut hg = boot_new_world(name, seed, false);
    let start = hg.state.players[0].player.pos;
    // Save it the way an old build did (the default), then as a bed sets it.
    for (spawn, repaired) in [
        (Vec3::from_array(LEGACY_DEFAULT_SPAWN), true),
        (Vec3::new(start.x + 7.0, start.y, start.z - 3.0), false),
    ] {
        hg.state.players[0].spawn_pos = spawn;
        assert!(hg.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
        // The reopen reads the world's seed and generator from its meta, which a
        // harness world has not been given: name them, or it opens another world.
        let mut meta = crate::save::load_world_meta(name);
        meta.seed = seed;
        meta.worldgen_version = crate::biome::Generator::V3.version();
        crate::save::save_world_meta(name, &meta).expect("meta written");
        hg.reopen_world(name);
        assert_eq!(hg.state.biome_gen.seed, seed, "control: the reopened world is the same world");
        let got = hg.state.players[0].spawn_pos;
        if repaired {
            assert_eq!(got, start, "the legacy default loads as the world spawn");
        } else {
            assert_eq!(got, spawn, "a chosen spawn loads untouched");
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness"]
fn game_harness_spawn_satoshis_house_stands_on_dry_land_and_is_meshed_straight_away() {
    super::isolate_saves();
    let seed = crate::spawn_search::sea_origin_seed();
    // Satoshi off while the world loads and settles: the house is then built
    // after every section round it has its mesh, the order that used to leave
    // it invisible.
    let mut hg = boot_new_world("harness-spawn-house", seed, false);
    settle(&mut hg);
    hg.state.world.satoshi.enabled = true;
    hg.state.world.satoshi.home = None;
    for _ in 0..8 {
        hg.ticks(10);
        if hg.state.world.satoshi.home.is_some() {
            break;
        }
    }
    let [ox, oy, oz] = hg.state.world.satoshi.home.expect("the house was placed");
    settle(&mut hg);

    let reg = &hg.state.registry;
    for dz in 0..5 {
        for dx in 0..5 {
            let under = hg.state.world.get_block(ox + dx, oy - 1, oz + dz);
            assert!(reg.is_solid(under) && !block::is_fluid(under), "seed {seed}: floor ({dx},{dz}) on block {under}");
        }
    }
    // Every section the house touches holds exactly the mesh the world now has.
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let sections: std::collections::BTreeSet<(i32, i32, i32)> = (ox..ox + 5)
        .flat_map(|x| (oy - 2..=oy + 4).flat_map(move |y| (oz..oz + 5).map(move |z| (x, y, z))))
        .map(|(x, y, z)| (x.div_euclid(cs), y.div_euclid(cs), z.div_euclid(cs)))
        .collect();
    for sec in sections {
        let have = hg.state.renderer.chunk_meshes.get(&sec).map_or(0, |m| m.index_count) as usize;
        let want = crate::mesh::build_chunk_meshes(sec.0, sec.1, sec.2, &hg.state.world, &hg.state.registry)
            .opaque
            .indices
            .len();
        assert_eq!(have, want, "section {sec:?}: the drawn mesh is not the world's");
    }
}

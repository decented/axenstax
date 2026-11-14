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
    boot_new_world_with(name, seed, satoshi, crate::spawn_pref::SpawnPref::Default)
}

/// [`boot_new_world`] with a spawn preference chosen on the new-world screen.
fn boot_new_world_with(name: &str, seed: u32, satoshi: bool, pref: crate::spawn_pref::SpawnPref) -> HeadlessGame {
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
    hg.state.pending_spawn_pref = pref;
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

/// A new world started with a spawn preference ("Near village", a biome): the
/// start the player is finally given, after the preference has moved them off
/// the world spawn, is the respawn point. Before, `spawn_pos` kept the world
/// spawn, which can be hundreds of blocks away.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness"]
fn game_harness_spawn_a_spawn_preference_start_is_the_respawn_point() {
    super::isolate_saves();
    use crate::spawn_pref::SpawnPref;
    let seed = crate::spawn_search::sea_origin_seed();
    let mut moved_any = false;
    for (i, pref) in [SpawnPref::NearVillage, SpawnPref::InForest, SpawnPref::InPlains, SpawnPref::FarFromVillages]
        .into_iter()
        .enumerate()
    {
        let hg = boot_new_world_with(&format!("harness-spawn-pref-{i}"), seed, false, pref);
        let slot = &hg.state.players[0];
        let world_spawn = crate::chunk_stream::world_spawn_point(&hg.state.world, &hg.state.biome_gen);
        assert_eq!(slot.spawn_pos, slot.player.pos, "{pref:?}: the start is the respawn point");
        assert_ne!(slot.spawn_pos.to_array(), LEGACY_DEFAULT_SPAWN, "{pref:?}");
        moved_any |= (slot.player.pos.x - world_spawn.x).hypot(slot.player.pos.z - world_spawn.z) > 8.0;
    }
    assert!(moved_any, "control: at least one preference started the player away from the world spawn");
}

/// The client's own respawn rule, with a spawn point that is NOT dry (the old
/// default over the sea origin; a bed's far spot nobody has loaded): the
/// `GameState` impl of the rule falls back to the world spawn and loads the
/// columns it reads (blocks and light, meshes queued through the streaming
/// path), then the body stands on dry ground.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness"]
fn game_harness_spawn_a_client_whose_spawn_point_is_in_the_sea_respawns_on_dry_ground() {
    super::isolate_saves();
    let seed = crate::spawn_search::sea_origin_seed();
    let mut hg = boot_new_world("harness-spawn-sea-spawn", seed, false);
    let start = hg.state.players[0].player.pos;
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    let die_and_respawn = |hg: &mut HeadlessGame| -> Vec3 {
        hg.state.players[0].player.pos = Vec3::new(0.5, 70.0, 0.5);
        hg.state.players[0].combat.die(crate::survival::DamageCause::Generic);
        hg.ticks(2);
        assert!(hg.state.players[0].combat.request_respawn());
        hg.ticks(2);
        assert!(!hg.state.players[0].combat.dead, "respawned");
        hg.state.players[0].player.pos
    };

    // 1. A respawn point over the sea origin (the default a new world used to
    //    save): the fallback is the world spawn, which a new world has loaded.
    let sea = Vec3::new(0.5, 80.0, 0.5);
    assert!(!dry(&hg, sea), "control: seed {seed}: the origin is not dry standing ground");
    hg.state.players[0].spawn_pos = sea;
    let at = die_and_respawn(&mut hg);
    assert!(dry(&hg, at), "seed {seed}: respawned at {at:?}, not on dry ground");
    assert_eq!(at, start, "a sea spawn point falls back to the world spawn");
    assert_eq!(hg.state.players[0].spawn_pos, sea, "and the saved spawn point is left alone");

    // 2. A respawn point on LAND in columns nobody has loaded (found on the
    //    generator's own heights): only a rule that loads them can stand the
    //    body there, so the body ends up at that spot, not back at the start.
    //    The meshes are queued through the streaming path and drain.
    let (fx, fz) = (0..400)
        .map(|i| (1600 + 16 * i, -1800))
        .find(|&(x, z)| {
            let bg = &hg.state.biome_gen;
            bg.surface_y(x, z) >= 66 && bg.terrain_height(x, z) >= 66
        })
        .expect("land in a fixed scan east of the origin");
    let far = Vec3::new(fx as f32 + 0.5, 110.0, fz as f32 + 0.5);
    let far_col = crate::chunk_stream::column_of(far);
    assert!(!hg.state.loaded_columns.contains(&far_col), "control: the far column is not loaded");
    hg.state.players[0].spawn_pos = far;
    let at = die_and_respawn(&mut hg);
    assert!(dry(&hg, at), "seed {seed}: respawned at {at:?}, not on dry ground");
    assert!(
        (at.x - far.x).hypot(at.z - far.z) <= 8.0,
        "seed {seed}: respawned at {at:?}, not near the loaded far spawn {far:?} (the rule did not load its column)"
    );
    settle(&mut hg);
    assert!(hg.state.dirty_mesh_chunks.is_empty(), "the queued meshes drained");
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

// ── Does a new world that is really ticking get Satoshi's house? ──────────────
//
// `satoshi::site_touches_player_work` refuses a site whose sections were edited
// since generation (`World::section_edited`), and every non-worldgen
// `set_block` marks its chunk. These tests enter a NEW world the way the lobby
// does (meta on disk, then the queued `LoadWorld`), run one frame and one tick
// at a time for 60 game seconds (streaming, random ticks, fluids, leaf decay,
// the once-a-second population pass), and count the houses. Many seeds, land and
// sea; and a world saved and reloaded before its house exists.

/// Seeds spread over the range, land and sea origins; the first is the sea
/// origin the other spawn tests use.
fn house_seeds() -> Vec<u32> {
    let mut seeds = vec![crate::spawn_search::sea_origin_seed()];
    seeds.extend([1, 7, 42, 1234, 31337, 99991, 271828, 314159, 8675309, 123456789, 987654321, 0xDEAD_BEEF, 0x1234_5678, 4_000_000_000]);
    seeds
}

const HOUSE_TICKS: u64 = 1200;

/// Write a new normal Survival world's meta as the lobby's Create does, then open it
/// through the lobby's own "Play".
fn new_world_via_lobby(name: &str, seed: u32) -> HeadlessGame {
    let _ = crate::save::delete_world(name);
    let mut meta = crate::save::WorldMeta::new(name);
    meta.seed = seed;
    meta.worldgen_version = crate::biome::Generator::V3.version();
    meta.satoshi_enabled = meta.world_type == "normal" && meta.game_mode == "survival";
    crate::save::save_world_meta(name, &meta).expect("meta written");
    let state = pollster::block_on(crate::GameState::new_headless_routed(
        1280,
        720,
        crate::local_routing::LocalRouting::Standalone,
    ));
    let mut hg = HeadlessGame { state };
    // A fresh boot shows the splash; the lobby is where a world is opened from.
    hg.state.mode = crate::GameMode::Menu(Box::new(crate::menu::MenuState::new()));
    hg.reopen_world(name);
    assert!(hg.state.world.satoshi.enabled, "control: the new world hosts Satoshi");
    assert_eq!(hg.state.biome_gen.seed, seed, "control: the lobby's seed is the world's");
    hg
}

/// What a run of ticks ended with.
#[derive(Debug, Clone, PartialEq)]
enum HouseOutcome {
    /// Built, this many ticks after the run began.
    Built(u64),
    /// The search ran and found no site in reach.
    NoSite,
    /// Never found its ground loaded.
    Waiting,
}

/// Run `ticks` logical ticks, a real frame (chunk streaming, meshing, UI) before
/// each tick, and say what became of the house.
fn tick_for_house(hg: &mut HeadlessGame, ticks: u64) -> HouseOutcome {
    let t0 = hg.state.tick_counter;
    let mut built = None;
    while hg.state.tick_counter - t0 < ticks {
        hg.frames(1);
        if built.is_none() && hg.state.world.satoshi.home.is_some() {
            built = Some(hg.state.tick_counter - t0);
        }
        hg.ticks(1);
        if built.is_none() && hg.state.world.satoshi.home.is_some() {
            built = Some(hg.state.tick_counter - t0);
        }
    }
    match built {
        Some(t) => HouseOutcome::Built(t),
        None if hg.state.world.satoshi.no_site.is_some() => HouseOutcome::NoSite,
        None => HouseOutcome::Waiting,
    }
}

/// Whether the world's origin is open sea or land, read the way `sea_origin_seed` reads it.
fn origin_kind(bg: &crate::biome::BiomeGenerator) -> &'static str {
    let sea = (-3..=3).all(|i| (-3..=3).all(|j| bg.surface_y(i * 4, j * 4) < 63 && bg.terrain_height(i * 4, j * 4) < 63));
    if sea { "sea" } else if bg.terrain_height(0, 0) < 63 { "coast" } else { "land" }
}

/// Which sections near the house's first try are marked edited, for a failure message.
fn edited_sections_near_spawn(hg: &HeadlessGame) -> Vec<(i32, i32, i32)> {
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let (sx, sz) = crate::chunk_stream::world_spawn_xz(&hg.state.world, &hg.state.biome_gen);
    let (c0x, c0z) = ((sx - 28).div_euclid(cs), (sz - 28).div_euclid(cs));
    let (c1x, c1z) = ((sx + 32).div_euclid(cs), (sz + 32).div_euclid(cs));
    let mut out = Vec::new();
    for cx in c0x..=c1x {
        for cz in c0z..=c1z {
            for cy in 0..=crate::world::MAX_CHUNK_Y {
                if hg.state.world.section_edited(cx, cy, cz) {
                    out.push((cx, cy, cz));
                }
            }
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness_spawn"]
fn game_harness_spawn_every_new_world_gets_satoshis_house_within_a_minute() {
    super::isolate_saves();
    let mut failures = Vec::new();
    for (i, seed) in house_seeds().into_iter().enumerate() {
        let name = format!("harness-spawn-house-seed-{i}");
        let mut hg = new_world_via_lobby(&name, seed);
        let col = crate::spawn_search::world_spawn_column(&hg.state.biome_gen);
        let origin = origin_kind(&hg.state.biome_gen);
        // Where the world stands the moment it is playable, before one tick of ours.
        let (t_entry, home_at_entry) = (hg.state.tick_counter, hg.state.world.satoshi.home.is_some());
        let outcome = tick_for_house(&mut hg, HOUSE_TICKS);
        println!(
            "HOUSE seed={seed} origin={origin} spawn=({},{}) kind={:?} tick_at_entry={t_entry} house_at_entry={home_at_entry} -> {outcome:?} final_tick={}",
            col.x, col.z, col.kind, hg.state.tick_counter
        );
        if !matches!(outcome, HouseOutcome::Built(_)) {
            let edited = edited_sections_near_spawn(&hg);
            println!("HOUSE seed={seed} edited sections near the house site: {edited:?}");
            failures.push(format!("seed {seed}: {outcome:?}"));
        }
        let _ = hg.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby);
        let _ = crate::save::delete_world(&name);
    }
    assert!(failures.is_empty(), "new worlds without a house after {HOUSE_TICKS} ticks: {failures:?}");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness_spawn"]
fn game_harness_spawn_a_new_world_saved_and_reloaded_before_its_house_still_gets_it() {
    super::isolate_saves();
    let seeds = house_seeds();
    let mut failures = Vec::new();
    for (i, seed) in [seeds[0], seeds[2], seeds[6]].into_iter().enumerate() {
        let name = format!("harness-spawn-house-reload-{i}");
        let mut hg = new_world_via_lobby(&name, seed);
        // The player quits the moment the world is playable.
        let built_early = hg.state.world.satoshi.home.is_some();
        println!("RELOAD seed={seed} at first entry: tick={} house={built_early}", hg.state.tick_counter);
        assert!(
            !built_early,
            "seed {seed}: the house is already built when the world first becomes playable (tick {}), so no save before it is reachable",
            hg.state.tick_counter
        );
        assert!(
            hg.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby),
            "the world saves"
        );
        let meta = crate::save::load_world_meta(&name);
        assert!(meta.satoshi_enabled && meta.seed == seed, "control: the saved meta is still this new world's");
        hg.reopen_world(&name);
        assert_eq!(hg.state.biome_gen.seed, seed, "control: the reloaded world is the same world");
        let had_house_on_reload = hg.state.world.satoshi.home.is_some();
        println!(
            "RELOAD seed={seed} after reload: tick={} house={had_house_on_reload} no_site={} edited near site: {:?}",
            hg.state.tick_counter,
            hg.state.world.satoshi.no_site.is_some(),
            edited_sections_near_spawn(&hg)
        );
        let outcome = tick_for_house(&mut hg, HOUSE_TICKS);
        let ok = had_house_on_reload || matches!(outcome, HouseOutcome::Built(_));
        println!("RELOAD seed={seed} built_before_quit={built_early} home_after_reload={had_house_on_reload} -> {outcome:?}");
        if !ok {
            println!("RELOAD seed={seed} edited sections near the house site: {:?}", edited_sections_near_spawn(&hg));
            failures.push(format!("seed {seed}: {outcome:?}"));
        }
        let _ = hg.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby);
        let _ = crate::save::delete_world(&name);
    }
    assert!(failures.is_empty(), "reloaded new worlds without a house: {failures:?}");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness_spawn"]
fn game_harness_spawn_a_house_sited_a_minute_late_still_finds_a_site() {
    super::isolate_saves();
    let mut failures = Vec::new();
    for (i, seed) in house_seeds().into_iter().enumerate() {
        let name = format!("harness-spawn-house-late-{i}");
        let mut hg = new_world_via_lobby(&name, seed);
        // Satoshi sits out the first minute (a world whose ground was slow to
        // load), so the world has ticked for real before any site is chosen.
        hg.state.world.satoshi.enabled = false;
        let _ = tick_for_house(&mut hg, HOUSE_TICKS);
        let edited_before = edited_sections_near_spawn(&hg);
        hg.state.world.satoshi.enabled = true;
        let outcome = tick_for_house(&mut hg, 100);
        println!("LATE seed={seed} edited sections near the site before siting: {edited_before:?} -> {outcome:?}");
        if !matches!(outcome, HouseOutcome::Built(_)) {
            failures.push(format!("seed {seed}: {outcome:?}"));
        }
        let _ = hg.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby);
        let _ = crate::save::delete_world(&name);
    }
    assert!(failures.is_empty(), "worlds that ticked a minute first and then got no house: {failures:?}");
}

/// Creative never cancels a death that has happened. Dead in Survival, away from the
/// spawn point, `/gamemode creative` is refused (the reply says to press Respawn first):
/// the mode stays Survival, the death screen stays up and the body stays where it fell
/// (it used to stand up there, alive, with its items left in the grave). After Respawn the
/// switch works, and a lethal hit does not kill a living Creative player.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness_spawn"]
fn game_harness_spawn_creative_never_revives_a_dead_player() {
    use crate::play_mode::PlayMode;
    super::isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-creative-dead");
    hg.state.set_play_mode(PlayMode::Survival);
    let feet = clear_pad(&mut hg, None);
    hg.state.players[0].player.pos = Vec3::new(feet[0] as f32 + 3.5, feet[1] as f32, feet[2] as f32 + 0.5);
    hg.ticks(2);
    hg.state.players[0].combat.take_damage(1000.0);
    hg.ticks(2);
    assert!(hg.state.players[0].combat.dead, "dead");
    let fell = hg.state.players[0].player.pos;
    assert!(fell.distance(hg.state.players[0].spawn_pos) > 2.0, "the body fell away from the spawn point");

    hg.state.submit_command_line("/gamemode creative".to_string());
    hg.ticks(3);
    hg.frames(2);
    let p = &hg.state.players[0];
    assert!(p.combat.dead, "still dead: the death screen stays up");
    assert!(p.player.pos.distance(fell) < 0.01, "the body stays where it fell: {:?} -> {:?}", fell, p.player.pos);
    assert_eq!(hg.state.play_mode, PlayMode::Survival, "the mode did not change");
    assert!(
        hg.state.chat.log.iter().any(|l| l.text.contains("Respawn")),
        "the reply says to press Respawn first"
    );

    // Respawned, the switch works, and a living Creative player takes no lethal hit.
    assert!(hg.state.players[0].combat.request_respawn());
    hg.ticks(2);
    assert!(!hg.state.players[0].combat.dead, "respawned");
    hg.state.submit_command_line("/gamemode creative".to_string());
    assert_eq!(hg.state.play_mode, PlayMode::Creative);
    hg.state.players[0].combat.take_damage(1000.0);
    hg.ticks(2);
    assert!(!hg.state.players[0].combat.dead, "a living Creative player is not killed");
}

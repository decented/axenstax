//! D3 — `--solo-hosted` through the REAL game: single-player entered from the
//! lobby's load flow runs through a local, transport-less `HostedServer`
//! lending the client's one world, with the main loop's one fixed tick
//! sequence. Default routing stays standalone (no server at all).
use super::*;
use crate::local_routing::{LocalRouting, SessionMode};

fn unique(tag: &str) -> String {
    format!("solo-{tag}-{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "-")
}

fn solo(tag: &str) -> (String, HeadlessGame) {
    isolate_saves();
    let name = unique(tag);
    let _ = std::fs::remove_dir_all(crate::save::world_dir(&name));
    let game = HeadlessGame::boot_into_world_routed(&name, LocalRouting::SoloHosted);
    (name, game)
}

/// Every lent tick ran each shared sim system exactly once (the D1 tripwire).
fn assert_one_sim(reports: &[crate::game_loop::LentTickReport]) {
    for (i, r) in reports.iter().enumerate() {
        assert!(r.faults.is_empty(), "tick {i}: a shared system ran the wrong number of times: {:?}", r.faults);
    }
}

fn stone() -> crate::item::Item {
    crate::item::ItemStack::new_block(crate::block::STONE, 1).item
}

fn stone_in(inv: &crate::inventory::Inventory) -> u32 {
    inv.slots_iter().flatten().filter(|s| s.item == stone()).map(|s| u32::from(s.count)).sum()
}

fn server_tick(game: &HeadlessGame) -> u64 {
    game.state.hosted_server.as_ref().expect("a local authority").server_tick_for_test()
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_default_single_player_runs_no_server() {
    isolate_saves();
    let name = unique("standalone");
    let mut game = HeadlessGame::boot_into_world(&name);
    assert_eq!(game.state.local_routing, LocalRouting::Standalone);
    assert_eq!(game.state.session_mode(), SessionMode::LocalStandalone);
    assert!(game.state.hosted_server.is_none());
    assert!(game.logical_ticks(3).is_empty(), "standalone runs the client tick alone");
    assert!(game.state.hosted_server.is_none());
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_solo_hosted_enters_through_a_network_silent_local_authority() {
    let (_name, mut game) = solo("entry");
    assert_eq!(game.state.session_mode(), SessionMode::LocalAuthority);
    assert!(!game.state.networked());
    assert!(game.state.network_host().is_none(), "no room or hosting command reaches it");
    let hs = game.state.hosted_server.as_ref().expect("the entry started a local authority");
    assert!(hs.local_only() && hs.lends_host_world());
    assert!(!hs.has_remote_clients());
    assert!(game.state.live_world.is_some());
    assert_eq!(server_tick(&game), 0, "it starts before the first tick");

    // One whole logical tick is the client's half plus the lent server's.
    let (c0, s0) = (game.state.tick_counter, server_tick(&game));
    let reports = game.logical_ticks(3);
    assert_eq!(reports.len(), 3);
    assert_one_sim(&reports);
    assert_eq!(game.state.tick_counter, c0 + 3);
    assert_eq!(server_tick(&game), s0 + 3);
    assert!(game.state.gameplay_preparation.is_none(), "no join-admission preparation for a local-only server");
    assert!(game.state.hosted_server.as_ref().unwrap().local_gameplay_preparation.is_none());
}

fn save_and_reopen(seats: usize) {
    let (name, mut game) = solo(&format!("seats{seats}"));
    let p0 = game.state.players[0].player.pos;
    for i in 1..seats {
        assert_eq!(game.state.add_local_seat(p0 + glam::Vec3::new(i as f32 * 2.0, 0.0, 0.0)), i);
    }
    for i in 0..seats {
        assert!(game.state.players[i].inventory.add_item(crate::item::ItemStack::new_block(crate::block::STONE, (i + 1) as u8)).is_none());
    }
    assert_one_sim(&game.logical_ticks(5));
    {
        let hs = game.state.hosted_server.as_ref().unwrap();
        assert_eq!(hs.server.players.len(), seats, "every seat has a server slot");
        assert_eq!(hs.local_transports.len(), 1, "only seat 0 has a loopback");
    }
    let before: Vec<u32> = game.state.players.iter().map(|p| stone_in(&p.inventory)).collect();
    assert_eq!(before, (1..=seats as u32).collect::<Vec<_>>());

    assert!(game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    assert!(game.state.hosted_server.is_none(), "leaving drops the authority");
    let dir = crate::save::world_dir(&name);
    assert!(!crate::handover::live::personal_adapter::has_any_bound_records(&dir).unwrap(), "no joiner sidecar for a local seat");

    game.reopen_world(&name);
    assert_eq!(game.state.players.len(), seats, "every seat is restored");
    let after: Vec<u32> = game.state.players.iter().map(|p| stone_in(&p.inventory)).collect();
    assert_eq!(after, before);
    assert_eq!(game.state.session_mode(), SessionMode::LocalAuthority);
    assert_eq!(server_tick(&game), 0, "a reopen gets fresh authority state");
    assert_one_sim(&game.logical_ticks(2));
    assert_eq!(game.state.hosted_server.as_ref().unwrap().server.players.len(), seats);
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_solo_hosted_two_seats_save_and_reopen() {
    save_and_reopen(2);
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_solo_hosted_four_seats_save_and_reopen() {
    save_and_reopen(4);
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_solo_hosted_seats_come_and_go_keeping_indices_and_clearing_rides() {
    let (_name, mut game) = solo("dynamic");
    let p0 = game.state.players[0].player.pos;
    game.state.add_local_seat(p0 + glam::Vec3::new(2.0, 0.0, 0.0));
    game.state.add_local_seat(p0 + glam::Vec3::new(4.0, 0.0, 0.0));
    assert_one_sim(&game.logical_ticks(1));
    assert_eq!(game.state.hosted_server.as_ref().unwrap().server.players.len(), 3);

    // Seat 1 sits in a cart. (Not ticked while seated: a cart off a rail
    // lets its rider off at once.) The server hears which seats ride the
    // way every lent tick tells it.
    let at = game.state.players[1].player.pos;
    let cart = crate::cart::spawn_cart(&mut game.state.ecs, (at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32));
    crate::riding::mount(&mut game.state.ecs, cart, &mut game.state.players[1].ride);
    game.state.players[1].riding = Some(cart);
    let seated: Vec<bool> = game.state.players.iter().map(crate::player_slot::PlayerSlot::seated).collect();
    assert_eq!(seated, [false, true, false]);
    game.state.hosted_server.as_mut().unwrap().sync_local_seats(&seated);
    assert!(game.state.hosted_server.as_ref().unwrap().server.players[1].local_seated);

    // Seat 1 leaves while riding.
    game.state.remove_local_seat(1);
    assert_eq!(game.state.players.len(), 2);
    assert!(game.state.ecs.get::<&crate::entity::Ridden>(cart).is_err(), "the cart is free again");
    assert!(crate::riding::may_mount(&game.state.ecs, cart).is_ok());
    assert_one_sim(&game.logical_ticks(1));
    let moved = game.state.players[1].player.pos;
    let hs = game.state.hosted_server.as_ref().unwrap();
    assert_eq!(hs.server.players.len(), 3, "server slot indices are kept");
    assert!(hs.server.players[1].connected && !hs.server.players[2].connected);
    assert!(!hs.server.players[1].local_seated && !hs.server.players[2].local_seated, "no stale ride");
    assert_eq!(hs.server.players[1].player.pos, moved, "slot 1 follows the seat now at index 1");

    // A new seat fills the free slot again.
    game.state.add_local_seat(p0 + glam::Vec3::new(6.0, 0.0, 0.0));
    assert_one_sim(&game.logical_ticks(1));
    let hs = game.state.hosted_server.as_ref().unwrap();
    assert_eq!(hs.server.players.len(), 3);
    assert!(hs.server.players[2].connected);
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_one_death_makes_one_grave_in_both_routings() {
    for routing in [LocalRouting::Standalone, LocalRouting::SoloHosted] {
        isolate_saves();
        let name = unique(&format!("death-{routing:?}"));
        let _ = std::fs::remove_dir_all(crate::save::world_dir(&name));
        let mut game = HeadlessGame::boot_into_world_routed(&name, routing);
        assert_eq!(game.state.hosted_server.is_some(), routing == LocalRouting::SoloHosted);
        game.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        game.state.world.keep_inventory = false;
        assert_eq!(game.state.world.iter_graves().count(), 0);
        assert!(game.state.players[0].inventory.add_item(crate::item::ItemStack::new_block(crate::block::STONE, 7)).is_none());
        assert!(game.state.players[0].combat.take_damage_from(1000.0, crate::survival::DamageCause::Generic));
        let reports = game.logical_ticks(5);
        assert_one_sim(&reports);
        assert_eq!(game.state.world.iter_graves().count(), 1, "{routing:?}: one death, one grave");
        let in_graves: u32 = game
            .state
            .world
            .iter_graves()
            .flat_map(|(_, g)| g.slots.iter().flatten())
            .filter(|s| s.item == stone())
            .map(|s| u32::from(s.count))
            .sum();
        assert_eq!(in_graves, 7, "{routing:?}: the stack moved into the grave once");
        assert_eq!(stone_in(&game.state.players[0].inventory), 0, "{routing:?}");
        if let Some(hs) = game.state.hosted_server.as_ref() {
            assert!(hs.server.players.iter().all(|sp| !sp.server_simulated), "no joiner body to die again");
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_solo_hosted_pause_stops_both_halves_and_catch_up_counts_logical_ticks() {
    let (_name, mut game) = solo("pause");
    assert_one_sim(&game.logical_ticks(1));
    let (c0, s0) = (game.state.tick_counter, server_tick(&game));
    game.state.mode = GameMode::Paused { confirm_quit: false, confirm_creative: false };
    game.state.tick_accumulator = crate::TICK_DURATION * 4;
    game.frames(5);
    assert_eq!(game.state.tick_counter, c0, "paused: no client tick");
    assert_eq!(server_tick(&game), s0, "paused: no server tick");
    // D4 slice 2 — nor any clock step on either half.
    let (w0, a0) = (game.state.world_time, authority(&game).tick_counter);
    game.frames(2);
    assert_eq!((game.state.world_time, authority(&game).tick_counter), (w0, a0), "paused: the clock holds");

    game.state.mode = GameMode::Playing;
    game.state.last_tick = web_time::Instant::now();
    game.state.tick_accumulator = crate::TICK_DURATION * 3;
    game.frames(1);
    let client = game.state.tick_counter - c0;
    let server = server_tick(&game) - s0;
    assert!((1..=crate::tick_catchup::MAX_BACKLOG_TICKS as u64 + 1).contains(&client), "caught up {client} ticks");
    assert_eq!(server, client, "each caught-up tick is one whole logical tick");
    assert_eq!(authority(&game).tick_counter, a0 + client, "the authority's clock stepped once per logical tick");
    assert_eq!(game.state.world_time, (w0 + client as u32) % 24_000, "and so did the day clock");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_solo_hosted_failed_save_keeps_playing_then_a_retry_leaves() {
    let (name, mut game) = solo("failsave");
    assert_one_sim(&game.logical_ticks(2));
    let meta_path = crate::save::world_dir(&name).join("world_meta.json");
    let good = std::fs::read(&meta_path).expect("the world has a meta");
    std::fs::write(&meta_path, b"{ torn").unwrap();
    assert!(
        !game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby),
        "a save that fails ends nothing"
    );
    assert!(matches!(game.state.mode, GameMode::Playing));
    assert!(game.state.live_world.is_some());
    assert!(game.state.hosted_server.as_ref().is_some_and(|hs| hs.local_only()), "the authority stays");
    let s0 = server_tick(&game);
    assert_one_sim(&game.logical_ticks(3));
    assert_eq!(server_tick(&game), s0 + 3, "still playable");

    std::fs::write(&meta_path, &good).unwrap();
    assert!(game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    assert!(matches!(game.state.mode, GameMode::Menu(_)));
    assert!(game.state.hosted_server.is_none(), "a landed save drops the authority");
    assert!(game.state.live_world.is_none());
}

// ─── D4 slice 2: the clock and weather ─────────────────────────────────────

/// The authority's clock in a solo-hosted game.
fn authority(game: &HeadlessGame) -> &crate::server::GameServer {
    &game.state.hosted_server.as_ref().expect("a local authority").server
}

/// One logical tick steps the counter and the day clock exactly once, in
/// both routings: standalone by the client's own pieces, solo-hosted by the
/// server's phase (read back), each tallied once on the one world.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_one_logical_tick_steps_the_clock_once_in_both_routings() {
    isolate_saves();
    let name = unique("clock-standalone");
    let _ = std::fs::remove_dir_all(crate::save::world_dir(&name));
    let standalone = HeadlessGame::boot_into_world(&name);
    let (_name, solo) = solo("clock");
    for (mode, mut game) in [("standalone", standalone), ("solo-hosted", solo)] {
        for _ in 0..3 {
            let (t, w) = (game.state.tick_counter, game.state.world_time);
            let tally = game.state.world.sim_tally;
            let reports = game.logical_ticks(1);
            assert_one_sim(&reports);
            assert_eq!(game.state.tick_counter, t + 1, "{mode}: one step of the counter");
            assert_eq!(game.state.world_time, (w + 1) % 24_000, "{mode}: one step of the day clock");
            let after = game.state.world.sim_tally;
            for s in [crate::sim_lend::SimSystem::Clock, crate::sim_lend::SimSystem::Weather] {
                assert_eq!(after.get(s) - tally.get(s), 1, "{mode}: {s:?} ran once");
            }
            if game.state.hosted_server.is_some() {
                let server = authority(&game);
                assert_eq!((server.tick_counter, server.world_time), (game.state.tick_counter, game.state.world_time), "{mode}: read back");
            }
        }
    }
}

/// `/time set` and `/time speed` (and the bed) write the clock through one
/// path (`write_clock`): solo-hosted, the authority's; the next tick steps
/// from it and the client reads it back.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_solo_hosted_time_set_and_speed_write_the_authority() {
    let (_name, mut game) = solo("time");
    assert_one_sim(&game.logical_ticks(1));
    game.state.write_clock(18_000, 8);
    assert_eq!((authority(&game).world_time, authority(&game).world_time_step), (18_000, 8));
    assert_one_sim(&game.logical_ticks(1));
    assert_eq!(game.state.world_time, 18_008, "the next tick steps from the written clock");
    assert_eq!(authority(&game).world_time, 18_008);
    assert_one_sim(&game.logical_ticks(1));
    assert_eq!(game.state.world_time, 18_016, "at the written speed");
    // A stale client copy is never copied over the authority.
    game.state.world_time = 3;
    assert_one_sim(&game.logical_ticks(1));
    assert_eq!(game.state.world_time, 18_024, "the authority's clock, read back");
}

/// The local bed on a solo-hosted world: at night it sets the morning (7000)
/// on the authority, the bed spawn and a full heal, once; by day it refuses.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_solo_hosted_bed_sets_morning_spawn_and_heal_once() {
    let (_name, mut game) = solo("bed");
    game.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    let feet = clear_pad(&mut game, None);
    let bed = [feet[0], feet[1], feet[2] - 2];
    game.state.world.set_block(bed[0], bed[1], bed[2], crate::block::BED);
    assert_one_sim(&game.logical_ticks(1));
    game.state.write_clock(0, 1);
    assert_one_sim(&game.logical_ticks(1));
    game.state.players[0].combat.health = 6.0;
    aim_at(&mut game, glam::Vec3::new(bed[0] as f32 + 0.5, bed[1] as f32 + 0.5, bed[2] as f32 + 0.5));
    game.state.tick_accumulator = std::time::Duration::ZERO;
    right_click(&mut game);
    assert_eq!(authority(&game).world_time, 7000, "morning on the authority");
    assert_eq!(game.state.world_time, 7000);
    assert_eq!(game.state.players[0].spawn_pos, crate::item_actions::bed_spawn(bed));
    assert_eq!(game.state.players[0].combat.health, game.state.players[0].combat.max_health);
    assert_one_sim(&game.logical_ticks(1));
    assert_eq!(authority(&game).world_time, 7001, "the morning holds: no second skip");
    game.state.players[0].combat.health = 6.0;
    right_click(&mut game);
    assert_eq!(game.state.players[0].combat.health, 6.0, "by day the bed refuses");
    assert_eq!(authority(&game).world_time, 7001);
}

/// A storm's bolt on a solo-hosted world is the authority's: rolled in its
/// phase, landed in its world, presented by the client (the flash) — and the
/// client rolls none of its own. Standalone keeps rolling and landing its
/// own, unchanged.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_a_storm_strikes_once_in_both_routings() {
    let storm_tick = |from: u64, storm: crate::weather::Weather| {
        (from..from + 100_000).find(|&t| crate::weather::strike_roll(storm, t, true).is_some()).unwrap()
    };
    // Solo-hosted: the authority's storm.
    let (_name, mut game) = solo("storm");
    assert_one_sim(&game.logical_ticks(1));
    let base = authority(&game).tick_counter;
    let storm = crate::weather::Weather { rain_until: base + 100_000, storm_until: base + 100_000 };
    let t = storm_tick(base + 1, storm);
    {
        let server = &mut game.state.hosted_server.as_mut().unwrap().server;
        server.weather = storm;
        server.tick_counter = t - 1;
    }
    game.state.lightning_flash = 0.0;
    assert_one_sim(&game.logical_ticks(1));
    assert_eq!(game.state.tick_counter, t);
    assert_eq!(game.state.lightning_flash, 1.0, "the client presented the authority's bolt");
    assert_eq!(game.state.weather_storm_until, storm.storm_until, "and reads the authority's window");
    // Standalone: its own roll, its own fire.
    isolate_saves();
    let name = unique("storm-standalone");
    let _ = std::fs::remove_dir_all(crate::save::world_dir(&name));
    let mut game = HeadlessGame::boot_into_world(&name);
    let base = game.state.tick_counter;
    let storm = crate::weather::Weather { rain_until: base + 100_000, storm_until: base + 100_000 };
    let t = storm_tick(base + 1, storm);
    game.state.weather_rain_until = storm.rain_until;
    game.state.weather_storm_until = storm.storm_until;
    game.state.tick_counter = t - 1;
    game.state.lightning_flash = 0.0;
    game.ticks(1);
    assert_eq!(game.state.lightning_flash, 1.0, "standalone rolls and presents its own bolt");
}

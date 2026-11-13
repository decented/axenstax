//! Keep slice 2a: what a restart must carry on with. Saved deadlines, bounties,
//! sleep gates and joiners' positions continue as they were, and saves from
//! before the field (footer 59, sidecar v4) load as they always did.
//!
//! The servers here are real: a "restart" is the old server dropped and a new
//! one started on the same folder, as a process restart would do.
use crate::auction::{AuctionData, AuctionOwner};
use crate::hosted_server::tests::send_signed_join;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item_actions::ItemNote;
use crate::joiner_save::JoinerSave;
use crate::save::WorldsRootGuard;
use crate::server::GameServer;
use crate::transport::{ChannelClientTransport, ClientTransport as _};
use glam::Vec3;

/// `world.dat` as the last build before this field wrote it: footer 59, 55
/// fields, tick 50,000 at the save (not recorded), one auction.
const WORLD_FOOTER_59: &[u8] = include_bytes!("clock_save_fixtures/world_footer59.bin");
/// A joiner record as the last build before the position wrote it: v4.
const SIDECAR_V4: &[u8] = include_bytes!("clock_save_fixtures/sidecar_v4.bin");
const FIXTURE_AUCTION_DEADLINE: u64 = 51_200;
const KEY: [u8; 32] = [66; 32];

fn started_auction(deadline_tick: u64) -> AuctionData {
    let mut a = AuctionData::new(AuctionOwner::Npub("npub1owner".into()));
    a.started = true;
    a.deadline_tick = deadline_tick;
    a
}

fn start(world: &str) -> HostedServer {
    let mut hs = HostedServer::start(0, world.to_string(), 42, 0, RemoteTransport::WebSocket { port: 0 })
        .expect("the server starts");
    hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    // Wildlife is not what these tests are about, and it can shove a body.
    hs.server.ecs.clear();
    hs
}

/// The joiner joins; returns its transport and slot.
fn join(hs: &mut HostedServer, binding: u8) -> (ChannelClientTransport, usize) {
    let client = hs.attach_test_remote_with_binding(Some([binding; 32]));
    send_signed_join(&client, KEY, Some([binding; 32]));
    hs.tick();
    let i = hs.server.players.len() - 1;
    assert!(hs.server.players[i].verified_pubkey.is_some(), "the join was accepted");
    (client, i)
}

fn accept_spawn(client: &ChannelClientTransport) -> Vec3 {
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((crate::protocol::PacketType::JoinAccept, bytes)) = crate::protocol::deserialize_header(&pkt) {
            let accept = crate::protocol::safe_deserialize::<crate::protocol::JoinAcceptPacket>(bytes).unwrap();
            return Vec3::new(accept.spawn_x, accept.spawn_y, accept.spawn_z);
        }
    }
    panic!("no JoinAccept");
}

fn record_path(world: &str, pk: [u8; 32]) -> std::path::PathBuf {
    crate::save::world_dir(world)
        .join("joiners")
        .join(format!("{}.dat", crate::hosted_server::pubkey_to_npub(&pk)))
}

/// A spot away from the spawn that a body can stand on.
fn standing_spot(hs: &mut HostedServer) -> Vec3 {
    let p = hs.server.safe_spawn_loading(Vec3::new(30.5, 80.0, -12.5), true);
    assert_eq!(hs.server.safe_spawn_loading(p, true), p, "the check passes there");
    p
}

#[test]
fn an_auction_keeps_its_remaining_ticks_across_a_save_and_load() {
    let _g = WorldsRootGuard::new("clock-save-auction");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.tick_counter = 90_000;
    s.world.insert_auction((3, 64, 5), started_auction(91_200));
    s.try_save().unwrap();

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    let (_, loaded) = back.world.iter_auctions().next().expect("the auction loaded");
    assert_eq!(loaded.deadline_tick - back.tick_counter, 1_200, "the remaining ticks are unchanged");
}

#[test]
fn a_bounty_is_not_rerolled_across_a_save_and_load() {
    let _g = WorldsRootGuard::new("clock-save-bounty");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.tick_counter = crate::bounty::BOUNTY_REFRESH_TICKS * 3 + 5;
    assert!(crate::bounty::tick_bounty_refresh(&mut s.world, s.tick_counter, s.biome_gen.seed), "today's roll");
    let rolled: Vec<u32> = s.world.bounties.iter().map(|b| b.id).collect();
    assert!(!rolled.is_empty());
    s.try_save().unwrap();

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert!(
        !crate::bounty::tick_bounty_refresh(&mut back.world, back.tick_counter, back.biome_gen.seed),
        "the same day after a restart: nothing to roll"
    );
    let after: Vec<u32> = back.world.bounties.iter().map(|b| b.id).collect();
    assert_eq!(after, rolled);
}

#[test]
fn a_second_sleep_in_the_same_night_across_a_restart_is_refused() {
    let _g = WorldsRootGuard::new("clock-save-sleep");
    let name = "sleep";
    let mut hs = start(name);
    let (client, i) = join(&mut hs, 21);
    let at = standing_spot(&mut hs);
    let bed = [at.x.floor() as i32 + 1, at.y.floor() as i32, at.z.floor() as i32];
    hs.server.world.set_block(bed[0], bed[1], bed[2], crate::block::BED);
    hs.server.players[i].player.pos = at;
    hs.server.world_time = 0; // midnight
    hs.server.night_calendar.observe(23_999);
    hs.server.night_calendar.observe(0);
    let tonight = hs.server.night_calendar.tonight();
    {
        let server = &mut hs.server;
        crate::item_actions::serve_sleep(&mut server.players[i], &mut server.world, server.world_time, tonight, bed)
            .expect("the first sleep of the night");
    }
    assert_eq!(hs.server.players[i].slept_night, Some(tonight));
    hs.server.save_joiner(i).unwrap();
    hs.server.try_save().unwrap();
    drop(client);
    drop(hs);

    let mut hs = start(name);
    let (_client, i) = join(&mut hs, 22);
    let server = &mut hs.server;
    server.night_calendar.observe(server.world_time);
    let tonight = server.night_calendar.tonight();
    assert_eq!(
        crate::item_actions::serve_sleep(&mut server.players[i], &mut server.world, server.world_time, tonight, bed),
        Err(ItemNote::SleptTonight),
        "the same night, after the restart"
    );
}

#[test]
fn a_joiner_who_left_at_a_spot_rejoins_there_after_a_restart() {
    let _g = WorldsRootGuard::new("clock-save-position");
    let name = "position";
    let mut hs = start(name);
    let (client, i) = join(&mut hs, 23);
    let spawn = hs.server.players[i].spawn_pos;
    let p = standing_spot(&mut hs);
    assert!(p.distance(spawn) > 8.0, "the spot is not the spawn: {p} {spawn}");
    hs.server.players[i].player.pos = p;
    hs.server.players[i].yaw = 1.25;
    hs.server.players[i].pitch = -0.5;
    drop(client);
    hs.tick(); // the leave: its save
    hs.server.try_save().unwrap();
    drop(hs);

    let mut hs = start(name);
    let (client, i) = join(&mut hs, 24);
    let sp = &hs.server.players[i];
    assert!(sp.player.pos.distance(p) < 0.01, "back where it left: {} not {p}", sp.player.pos);
    assert_eq!(sp.spawn_pos, spawn, "its respawn point is not the spot it left");
    assert_eq!((sp.yaw, sp.pitch), (1.25, -0.5));
    assert!(accept_spawn(&client).distance(p) < 0.01, "the client starts where the body stands");
}

#[test]
fn a_joiner_who_left_at_an_unsafe_spot_rejoins_at_its_spawn() {
    let _g = WorldsRootGuard::new("clock-save-unsafe");
    let name = "unsafe";
    let mut hs = start(name);
    let (client, i) = join(&mut hs, 25);
    let pk = hs.server.players[i].verified_pubkey.unwrap();
    let spawn = hs.server.players[i].spawn_pos;
    // Deep in the ground: no body can stand there.
    let p = Vec3::new(30.5, 3.0, -12.5);
    assert_ne!(hs.server.safe_spawn_loading(p, true), p, "the check fails there");
    hs.server.players[i].player.pos = p;
    hs.server.save_joiner(i).unwrap();
    hs.server.try_save().unwrap();
    assert_eq!(std::fs::read(record_path(name, pk)).unwrap()[0], 5, "the record carries the position");
    drop(client);
    drop(hs);

    let mut hs = start(name);
    let (client, i) = join(&mut hs, 26);
    let expected = hs.server.safe_spawn_loading(spawn, true);
    assert_eq!(hs.server.players[i].player.pos, expected, "at the spawn, as a v4 record would");
    assert_eq!(accept_spawn(&client), expected);
}

/// Loading a footer-59 save has always worked, so this test's only way to fail
/// on the old build is its statement that 59 is now an OLDER format. The rest
/// pins that nothing about such a load changes.
#[test]
fn an_old_format_save_fixture_loads_with_todays_defaults() {
    let _g = WorldsRootGuard::new("clock-save-footer59");
    let bytes = WORLD_FOOTER_59;
    assert_eq!(crate::save_format::split_save_footer(bytes).1, Some(59));
    // 71 since the off-hand slot (the `player_offhands` field, layout revision 8): 59 is an older format still.
    assert_eq!(crate::save_format::SAVE_FORMAT_VERSION, 71, "59 is an older format than this one");
    let (save, damaged) = crate::save::read_world_save_reporting(bytes).expect("the old save is read");
    assert!(!damaged);
    assert_eq!(save.seed, 42);

    std::fs::create_dir_all(crate::save::world_dir("old")).unwrap();
    crate::save::save_world_meta("old", &crate::save::WorldMeta::new("old")).unwrap();
    std::fs::write(crate::save::world_dir("old").join("world.dat"), bytes).unwrap();
    let mut s = GameServer::new(0, "old".into(), 42);
    s.initial_load().unwrap();
    assert_eq!(s.tick_counter, 0);
    assert_eq!(s.world_time, 6000);
    assert_eq!(s.weather, crate::weather::Weather::CLEAR);
    assert_eq!(s.night_calendar.tonight(), 0);
    assert_eq!(s.reputation_day.digest_view(), (0, None));
    let (_, auction) = s.world.iter_auctions().next().expect("the auction loaded");
    assert_eq!(auction.deadline_tick, FIXTURE_AUCTION_DEADLINE, "the saved value, stretched as it always was");
}

/// As above: a v4 record has always loaded, so the old build fails this only
/// on its statement that the writer has moved on from v4.
#[test]
fn a_v4_sidecar_still_loads_and_its_joiner_rejoins_at_spawn() {
    let _g = WorldsRootGuard::new("clock-save-v4");
    let bytes = SIDECAR_V4;
    assert_eq!(bytes[0], 4);
    let saved = JoinerSave::decode(bytes).expect("the old record is read");
    assert_eq!(saved.encode().unwrap()[0], 5, "its next write is the new layout");

    let name = "v4";
    let mut hs = start(name);
    let (client, i) = join(&mut hs, 27);
    let pk = hs.server.players[i].verified_pubkey.unwrap();
    hs.server.try_save().unwrap();
    drop(client);
    hs.tick();
    drop(hs);
    let path = record_path(name, pk);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();

    let mut hs = start(name);
    let (client, i) = join(&mut hs, 28);
    let spawn = Vec3::from_array(saved.spawn);
    let expected = hs.server.safe_spawn_loading(spawn, true);
    let sp = &hs.server.players[i];
    assert_eq!(sp.spawn_pos, spawn);
    assert_eq!(sp.player.pos, expected);
    assert_eq!(sp.combat.hunger, 17);
    assert_eq!(sp.slept_night, None, "a night number from another clock means nothing");
    assert_eq!(accept_spawn(&client), expected);
}

fn full_clock() -> crate::sim_clock::SavedClock {
    crate::sim_clock::SavedClock {
        tick_counter: 90_000,
        world_time: 15_000,
        rain_until: 91_000,
        storm_until: 90_500,
        clear_until: 0,
        weather_lock_applied: Some("storm".into()),
        nights: 12,
        was_night: Some(true),
        calendar_id: [7; 16],
        day_remainder: 15_000,
        day_last_tick: Some(90_000),
    }
}

/// The dedicated path keeps every piece, a new server starts from them, and its
/// next phase carries on rather than repeating or skipping.
#[test]
fn a_servers_whole_clock_survives_a_save_and_load() {
    let _g = WorldsRootGuard::new("clock-save-whole");
    let saved = full_clock();
    let mut s = GameServer::new(0, "w".into(), 42);
    s.tick_counter = saved.tick_counter;
    s.world_time = saved.world_time;
    s.weather = crate::weather::Weather { rain_until: saved.rain_until, storm_until: saved.storm_until, clear_until: 0 };
    s.weather_lock_applied = saved.weather_lock_applied.clone();
    s.night_calendar = crate::item_actions::NightCalendar::from_saved(saved.nights, saved.was_night);
    s.calendar_id = saved.calendar_id;
    s.reputation_day.resume(saved.day_remainder, saved.day_last_tick);
    s.try_save().unwrap();
    let dat = std::fs::read(crate::save::world_dir("w").join("world.dat")).unwrap();
    assert_eq!(crate::save::read_world_save(&dat).unwrap().authority_clock, Some(saved.clone()));

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.tick_counter, saved.tick_counter);
    assert_eq!(back.world_time, saved.world_time);
    assert_eq!(back.weather, s.weather);
    assert_eq!(back.weather_lock_applied, saved.weather_lock_applied);
    assert_eq!(back.night_calendar.saved(), (12, Some(true)));
    assert_eq!(back.calendar_id, saved.calendar_id);
    assert_eq!(back.reputation_day.saved(), (15_000, Some(90_000)));
    assert_ne!(back.run_id, s.run_id, "a run is still a run: only the clock carries over");
    back.tick();
    assert_eq!(back.tick_counter, saved.tick_counter + 1);
    assert_eq!(back.world_time, saved.world_time + 1);
}

/// The client's save paths (single-player has no server) write what they are
/// given, each to the file it writes.
#[test]
fn the_client_save_paths_write_the_clock() {
    let _g = WorldsRootGuard::new("clock-save-client");
    let saved = full_clock();
    let world = crate::world::World::new();
    let slot = crate::player_slot::PlayerSlot::new(0, Vec3::new(0.5, 80.0, 0.5), 1.0);
    let seats = std::slice::from_ref(&slot);
    crate::save::autosave_world("w", &world, seats, 1, &[], &[], None, Some(&saved)).unwrap();
    let autosave = std::fs::read(crate::save::world_dir("w").join("autosave/world.dat")).unwrap();
    assert_eq!(crate::save::read_world_save(&autosave).unwrap().clock(), Some(&saved));
    crate::save::save_world_superseding_autosave("w", &world, seats, 1, &[], &[], None, Some(&saved)).unwrap();
    let primary = std::fs::read(crate::save::world_dir("w").join("world.dat")).unwrap();
    assert_eq!(crate::save::read_world_save(&primary).unwrap().clock(), Some(&saved));
}

/// A clock no build could have written is not trusted: the world loads as a
/// save without one does.
#[test]
fn an_out_of_range_saved_clock_is_not_restored() {
    let good = full_clock();
    assert!(good.is_valid());
    let furthest = good.tick_counter + (1 << 32);
    assert!(
        crate::sim_clock::SavedClock { rain_until: furthest, storm_until: furthest, ..good.clone() }.is_valid(),
        "a window as far ahead as the bound allows is real"
    );
    for bad in [
        crate::sim_clock::SavedClock { world_time: crate::sim_clock::DAY_TICKS, ..good.clone() },
        crate::sim_clock::SavedClock { tick_counter: u64::MAX, day_last_tick: None, ..good.clone() },
        crate::sim_clock::SavedClock { day_remainder: 24_000, ..good.clone() },
        crate::sim_clock::SavedClock { day_last_tick: Some(good.tick_counter + 1), ..good.clone() },
        crate::sim_clock::SavedClock { weather_lock_applied: Some("x".repeat(65)), ..good.clone() },
        // A window that never ends: rain with no re-roll, for good.
        crate::sim_clock::SavedClock { rain_until: u64::MAX, ..good.clone() },
        crate::sim_clock::SavedClock { storm_until: good.tick_counter + (1 << 32) + 1, ..good.clone() },
        // A `/weather clear` hold that never ends would hold the sky clear for good.
        crate::sim_clock::SavedClock { clear_until: good.tick_counter + (1 << 32) + 1, ..good.clone() },
    ] {
        let mut save = crate::save::minimal_world_save_for_tests(1);
        save.authority_clock = Some(bad.clone());
        assert_eq!(save.clock(), None, "{bad:?}");
    }
}

/// `/weather clear`'s hold rides the appended field 62 (`weather_clear_until`),
/// not the clock record: a new save carries it round trip through every save
/// path and a server restart; a save from before the field loads with no hold;
/// a damaged hold is dropped rather than holding the sky clear for good.
#[test]
fn a_clear_hold_round_trips_and_an_old_save_loads_with_none() {
    let _g = WorldsRootGuard::new("clock-save-hold");
    let held = crate::sim_clock::SavedClock { clear_until: 102_000, ..full_clock() };
    assert!(held.is_valid());

    // The dedicated path: server -> file -> new server.
    let mut s = GameServer::new(0, "w".into(), 42);
    s.restore_saved_clock(Some(&held));
    assert_eq!(s.weather.clear_until, 102_000);
    s.try_save().unwrap();
    let dat = std::fs::read(crate::save::world_dir("w").join("world.dat")).unwrap();
    let read = crate::save::read_world_save(&dat).unwrap();
    assert_eq!(read.weather_clear_until, Some(102_000));
    assert_eq!(read.clock().map(|c| c.clear_until), Some(102_000));
    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.weather.clear_until, 102_000, "a restart keeps the hold");

    // The client's save paths.
    let world = crate::world::World::new();
    let slot = crate::player_slot::PlayerSlot::new(0, Vec3::new(0.5, 80.0, 0.5), 1.0);
    let seats = std::slice::from_ref(&slot);
    crate::save::autosave_world("w2", &world, seats, 1, &[], &[], None, Some(&held)).unwrap();
    let autosave = std::fs::read(crate::save::world_dir("w2").join("autosave/world.dat")).unwrap();
    assert_eq!(crate::save::read_world_save(&autosave).unwrap().clock(), Some(&held));
    crate::save::save_world_superseding_autosave("w2", &world, seats, 1, &[], &[], None, Some(&held)).unwrap();
    let primary = std::fs::read(crate::save::world_dir("w2").join("world.dat")).unwrap();
    assert_eq!(crate::save::read_world_save(&primary).unwrap().clock(), Some(&held));

    // No hold is written as the one `None` byte, and loads as no hold.
    let none = crate::sim_clock::SavedClock { clear_until: 0, ..held.clone() };
    assert_eq!(none.clear_hold_for_save(), None);
    crate::save::save_world_superseding_autosave("w3", &world, seats, 1, &[], &[], None, Some(&none)).unwrap();
    let plain = std::fs::read(crate::save::world_dir("w3").join("world.dat")).unwrap();
    assert_eq!(crate::save::read_world_save(&plain).unwrap().clock(), Some(&none));

    // An OLD save (footer 69: 61 fields, no `weather_clear_until`) holding a clock loads with the
    // hold at 0 and is not reported damaged.
    let mut file = crate::save::minimal_world_save_for_tests(3);
    file.authority_clock = Some(none.clone());
    let mut body = bincode::serialize(&file).unwrap();
    assert_eq!(body.pop(), Some(0), "the newest field (the off-hand column) is one `None` byte");
    assert_eq!(body.pop(), Some(0), "and the weather hold before it");
    body.extend_from_slice(&crate::save_format::footer_bytes(69));
    let (old, damaged) = crate::save::read_world_save_reporting(&body).expect("a footer-69 save still loads");
    assert!(!damaged);
    assert_eq!(old.weather_clear_until, None);
    assert_eq!(old.clock().map(|c| (c.clear_until, c.rain_until, c.tick_counter)), Some((0, 91_000, 90_000)));

    // A hold further ahead than a window can be is dropped, the rest of the clock kept.
    let mut file = crate::save::minimal_world_save_for_tests(3);
    file.authority_clock = Some(none.clone());
    file.weather_clear_until = Some(none.tick_counter + (1 << 32) + 1);
    let bytes = crate::save_format::encode_world_save(&file).unwrap();
    let read = crate::save::read_world_save(&bytes).unwrap();
    assert_eq!(read.clock().map(|c| (c.clear_until, c.tick_counter)), Some((0, 90_000)));
}

/// A restored tick must not expire every rubber tap at once: the cooldown still
/// restarts at the load, as it did when the tick restarted at 0.
#[test]
fn a_tapped_rubber_log_keeps_a_full_cooldown_across_a_load() {
    let _g = WorldsRootGuard::new("clock-save-rubber");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.tick_counter = 1_000_000;
    s.world.set_block(3, 64, 5, crate::block::RUBBER_LOG_TAPPED);
    s.try_save().unwrap();
    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.tapped_rubber_logs.get(&(3, 64, 5)), Some(&1_000_000));
    assert!(crate::rubber::restore_expired_taps(&mut back.world, back.tick_counter).is_empty());
}

#[test]
fn the_wire_tick_carries_on_from_the_saved_clock() {
    let _g = WorldsRootGuard::new("clock-save-wire");
    let name = "wire";
    let mut hs = start(name);
    hs.server.tick_counter = 700_000;
    hs.server.try_save().unwrap();
    drop(hs);
    let mut hs = start(name);
    assert_eq!(hs.server.tick_counter, 700_000);
    hs.tick();
    assert_eq!(hs.server_tick_for_test(), 700_001);
    assert_eq!(hs.server.tick_counter, 700_001);
}

/// A slot that attached before the jump keeps its age: the pre-auth timeout
/// must not fire because the clock moved.
#[test]
fn a_joiner_mid_handshake_is_not_timed_out_by_the_wire_tick_moving() {
    let _g = WorldsRootGuard::new("clock-save-attached");
    let mut hs = start("attached");
    let client = hs.attach_test_remote_with_binding(Some([29; 32]));
    hs.server.wire_tick_seed = Some(5_000_000);
    hs.tick();
    assert_eq!(hs.server_tick_for_test(), 5_000_001);
    send_signed_join(&client, KEY, Some([29; 32]));
    hs.tick();
    let i = hs.server.players.len() - 1;
    assert!(hs.server.players[i].verified_pubkey.is_some(), "still in its handshake, and now joined");
}

/// A lent world's server takes the saved calendar with the rest of the clock
/// from its host client, and its wire tick follows.
#[test]
fn a_lending_server_takes_the_calendar_and_wire_tick_from_its_host() {
    let _g = WorldsRootGuard::new("clock-save-lent");
    let (mut hs, mut host) = super::lent_world::start_lent("clock-save");
    host.clock.tick_counter = 400_000;
    host.clock.world_time = 0;
    host.clock.night_calendar = crate::item_actions::NightCalendar::from_saved(12, Some(true));
    host.clock.calendar_id = [7; 16];
    hs.server.forget_host_clock();
    host.lend_tick(&mut hs);
    assert_eq!(hs.server.night_calendar.tonight(), 12);
    assert_eq!(hs.server.calendar_id, [7; 16]);
    assert_eq!(hs.server_tick_for_test(), 400_001, "the host's tick, then the window's own step");
}

// ---- Review fixes (2a-f): where a joiner may stand, and the paths the first
// tests did not reach. ----

/// A flat stone floor of the test's own at y = 119 (x 26..=32, z -16..=-9),
/// open air over it, high above any terrain; the columns round it are loaded.
/// Cell (30, 120, -13) is where each case puts its block.
fn test_floor(hs: &mut HostedServer) {
    hs.server.safe_spawn_loading(Vec3::new(30.5, 130.0, -12.5), true);
    let w = &mut hs.server.world;
    for x in 24..=36 {
        for z in -18..=-7 {
            for y in 119..=124 {
                let id = if y == 119 && (26..=32).contains(&x) && (-16..=-9).contains(&z) {
                    crate::block::STONE
                } else {
                    crate::block::AIR
                };
                w.set_block(x, y, z, id);
            }
        }
    }
}

const CELL: (i32, i32, i32) = (30, 120, -13);

/// The joiner leaves standing at `pos` on the floor `build` makes and joins
/// again in the same run; returns where its body stands and what its spawn
/// resolves to.
fn leave_and_rejoin(name: &str, bindings: (u8, u8), build: &dyn Fn(&mut HostedServer), pos: Vec3) -> (Vec3, Vec3) {
    let mut hs = start(name);
    test_floor(&mut hs);
    build(&mut hs);
    let (client, i) = join(&mut hs, bindings.0);
    let spawn = hs.server.players[i].spawn_pos;
    hs.server.players[i].player.pos = pos;
    drop(client);
    hs.tick(); // the leave: its save
    let (_client, i) = join(&mut hs, bindings.1);
    let body = hs.server.players[i].player.pos;
    let at_spawn = hs.server.safe_spawn_loading(spawn, true);
    (body, at_spawn)
}

/// Ordinary standing places that are not a whole block: a joiner who left
/// there goes back there, not to spawn.
#[test]
fn a_joiner_who_left_on_a_partial_block_rejoins_there() {
    let _g = WorldsRootGuard::new("clock-save-partial");
    use crate::meta::{Facing, with_aux, with_facing};
    let (x, y, z) = CELL;
    let put = |id: crate::block::BlockId, meta: u8| {
        move |hs: &mut HostedServer| {
            hs.server.world.set_block(x, y, z, id);
            hs.server.world.set_meta((x, y, z), meta);
        }
    };
    let none = |_: &mut HostedServer| {};
    let cases: Vec<(&str, Box<dyn Fn(&mut HostedServer)>, Vec3)> = vec![
        ("the plain floor", Box::new(none), Vec3::new(30.5, 120.0, -12.5)),
        ("a bottom slab", Box::new(put(crate::block::STONE_SLAB, with_facing(0, Facing::Down))), Vec3::new(30.5, 120.5, -12.5)),
        ("a snow layer", Box::new(put(crate::block::SNOW_LAYER, with_aux(0, 0))), Vec3::new(30.5, 120.125, -12.5)),
        // The step's tall half is on the north side; the body is on the low half.
        ("the low step of a stair", Box::new(put(crate::block::STONE_STAIRS, with_facing(0, Facing::North))), Vec3::new(30.5, 120.5, -12.2)),
        ("a bottom trapdoor", Box::new(put(crate::block::OAK_TRAPDOOR, with_facing(0, Facing::North))), Vec3::new(30.5, 120.1875, -12.5)),
        // The floor ends at x = 33: the body's centre is over air.
        ("a ledge edge", Box::new(none), Vec3::new(33.2, 120.0, -12.5)),
    ];
    for (n, (what, build, pos)) in cases.iter().enumerate() {
        let (body, spawn) = leave_and_rejoin(&format!("partial{n}"), (40 + 2 * n as u8, 41 + 2 * n as u8), build.as_ref(), *pos);
        assert_ne!(body, spawn, "{what}: not at spawn");
        assert!(body.distance(*pos) < 0.001, "{what}: back at {pos}, not {body}");
    }
}

/// And the places no body can stand still go to spawn.
#[test]
fn a_joiner_who_left_where_nothing_can_stand_rejoins_at_its_spawn() {
    let _g = WorldsRootGuard::new("clock-save-nostand");
    let (x, y, z) = CELL;
    let put = |id: crate::block::BlockId, at_y: i32| {
        move |hs: &mut HostedServer| hs.server.world.set_block(x, at_y, z, id)
    };
    let none = |_: &mut HostedServer| {};
    let cases: Vec<(&str, Box<dyn Fn(&mut HostedServer)>, Vec3)> = vec![
        ("over a drop", Box::new(none), Vec3::new(30.5, 121.0, -12.5)),
        ("in lava", Box::new(put(crate::block::LAVA, y)), Vec3::new(30.5, 120.0, -12.5)),
        ("in fire", Box::new(put(crate::block::FIRE, y)), Vec3::new(30.5, 120.0, -12.5)),
        ("inside a block", Box::new(put(crate::block::STONE, y)), Vec3::new(30.5, 120.5, -12.5)),
        ("under a low ceiling", Box::new(put(crate::block::STONE, y + 1)), Vec3::new(30.5, 120.0, -12.5)),
    ];
    for (n, (what, build, pos)) in cases.iter().enumerate() {
        let (body, spawn) = leave_and_rejoin(&format!("nostand{n}"), (60 + 2 * n as u8, 61 + 2 * n as u8), build.as_ref(), *pos);
        assert_eq!(body, spawn, "{what}: at spawn");
    }
}

/// A night mark ahead of a calendar that has not reached it yet is not held
/// against the body; one it has reached is.
#[test]
fn a_sleep_mark_from_a_night_not_yet_reached_is_dropped_on_rejoin() {
    let _g = WorldsRootGuard::new("clock-save-future-night");
    let mut hs = start("future-night");
    hs.server.night_calendar.observe(23_999);
    hs.server.night_calendar.observe(0);
    let tonight = hs.server.night_calendar.tonight();
    // A rejoin can take a lower slot than the last one in the list: find the
    // body that is in the world.
    let seat = |hs: &HostedServer| {
        hs.server.players.iter().position(|p| p.server_simulated && p.is_in_world()).expect("a joiner in the world")
    };
    for (n, (mark, kept)) in [(tonight, true), (tonight + 5, false)].into_iter().enumerate() {
        let (client, _) = join(&mut hs, 80 + 2 * n as u8);
        let i = seat(&hs);
        hs.server.players[i].slept_night = Some(mark);
        hs.server.save_joiner(i).unwrap(); // the record, under this calendar
        drop(client);
        hs.tick(); // the leave
        let (client, _) = join(&mut hs, 81 + 2 * n as u8);
        let i = seat(&hs);
        let expected = kept.then_some(mark);
        assert_eq!(hs.server.players[i].slept_night, expected, "mark {mark} at tonight {tonight}");
        drop(client);
        hs.tick(); // settle this leave before the next join
    }
}

/// A lending host's world is the host's: a spot in a column it holds is read
/// and the joiner goes back there; one it does not hold can't be checked, and
/// the joiner goes to spawn.
#[test]
fn a_lent_hosts_rejoin_checks_only_the_columns_it_holds() {
    use super::handover_live::signed_join_lent;
    let _g = WorldsRootGuard::new("clock-save-lent-rejoin");
    let (mut hs, mut host) = super::lent_world::start_prepared_lent("rejoin-spot");
    let seckey = [0x71; 32];
    let (client, slot, _, _) = signed_join_lent(&mut hs, &mut host, seckey);
    let spawn = hs.server.players[slot].spawn_pos;
    // A loaded column near the spawn with ground to stand on.
    let held = (-6..=6)
        .flat_map(|dx| (-6..=6).map(move |dz| (spawn.x.floor() as i32 + dx, spawn.z.floor() as i32 + dz)))
        .filter(|&(x, z)| (x - spawn.x.floor() as i32).abs() + (z - spawn.z.floor() as i32).abs() > 3)
        .find_map(|(x, z)| {
            crate::spawn_safety::surface_feet(&host.world, &hs.server.registry, x, z)
                .map(|y| Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5))
        })
        .expect("a held column with ground near the spawn");
    // Far outside anything the host loads.
    let unheld = Vec3::new(held.x + 5_000.0, held.y, held.z + 5_000.0);
    assert!(
        crate::spawn_safety::surface_feet(&host.world, &hs.server.registry, unheld.x.floor() as i32, unheld.z.floor() as i32)
            .is_none(),
        "the host holds nothing there"
    );

    let mut client = Some(client);
    let mut slot = slot;
    for (what, pos, expect_there) in [("a held column", held, true), ("a column the host does not hold", unheld, false)] {
        hs.server.players[slot].player.pos = pos;
        drop(client.take());
        host.lend_tick(&mut hs);
        host.lend_tick(&mut hs); // the leave: its save
        let (c, s, _, _) = signed_join_lent(&mut hs, &mut host, seckey);
        client = Some(c);
        slot = s;
        let body = hs.server.players[slot].player.pos;
        if expect_there {
            assert!(body.distance(pos) < 0.001, "{what}: back at {pos}, not {body}");
        } else {
            assert!(body.distance(pos) > 1_000.0, "{what}: not at {pos}, but {body}");
            let expected = crate::spawn_safety::fix_up(&host.world, &hs.server.registry, spawn);
            assert_eq!(body, expected, "{what}: at the spawn");
        }
    }
}

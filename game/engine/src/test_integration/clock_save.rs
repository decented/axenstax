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
        crate::item_actions::serve_sleep(&mut server.players[i], &server.world, server.world_time, tonight, bed)
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
        crate::item_actions::serve_sleep(&mut server.players[i], &server.world, server.world_time, tonight, bed),
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
    assert_eq!(crate::save_format::SAVE_FORMAT_VERSION, 60, "59 is the format before this one");
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
    s.weather = crate::weather::Weather { rain_until: saved.rain_until, storm_until: saved.storm_until };
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
    for bad in [
        crate::sim_clock::SavedClock { world_time: crate::sim_clock::DAY_TICKS, ..good.clone() },
        crate::sim_clock::SavedClock { tick_counter: u64::MAX, day_last_tick: None, ..good.clone() },
        crate::sim_clock::SavedClock { day_remainder: 24_000, ..good.clone() },
        crate::sim_clock::SavedClock { day_last_tick: Some(good.tick_counter + 1), ..good.clone() },
        crate::sim_clock::SavedClock { weather_lock_applied: Some("x".repeat(65)), ..good.clone() },
    ] {
        let mut save = crate::save::minimal_world_save_for_tests(1);
        save.authority_clock = Some(bad.clone());
        assert_eq!(save.clock(), None, "{bad:?}");
    }
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

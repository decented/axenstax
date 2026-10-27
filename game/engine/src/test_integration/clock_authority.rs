//! D4 slice 2 — the clock and weather have one owner per world (`sim_clock`).
//!
//! A lent world's (LAN / online host, `--solo-hosted`) clock is its server's:
//! the clock phase runs once per logical tick, before the lend window
//! (`GameServer::run_clock_phase`, as `OwnedSimParts::lend_tick` and the game
//! loop run it), the world phase in the window does not step it again, and
//! the host client reads it back. A dedicated server runs the same phase
//! inline. Standalone and `--solo-hosted` through the real game loop are
//! covered by the GPU harness (`test_game_harness/tests/solo_hosted.rs`).

use super::joiner_authority::join_guest;
use super::lent_world::{join_guest_lent, start_lent};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::sim_lend::{OwnedSimParts, SimSystem};
use crate::weather::Weather;

fn dedicated(tag: &str) -> HostedServer {
    HostedServer::start(
        0,
        format!("clock-authority-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts")
}

/// A storm over every tick of the test, and the first tick at or after
/// `from` on which it throws a bolt.
fn storm_and_strike_tick(from: u64) -> (Weather, u64) {
    let storm = Weather { rain_until: from + 100_000, storm_until: from + 100_000 };
    let t = (from..from + 100_000)
        .find(|&t| crate::weather::strike_roll(storm, t, true).is_some())
        .expect("a storm throws a bolt within minutes");
    (storm, t)
}

fn clock_runs(tally: &crate::sim_lend::SimTally) -> (u64, u64) {
    (tally.get(SimSystem::Clock), tally.get(SimSystem::Weather))
}

#[test]
fn one_dedicated_tick_steps_the_clock_and_counter_once_and_spawn_protection_once() {
    let mut hs = dedicated("once");
    let (_client, slot) = join_guest(&mut hs, "Once");
    crate::spawn_protection::arm(&mut hs.server.players[slot]);
    for _ in 0..3 {
        let (t, w, p) = (hs.server.tick_counter, hs.server.world_time, hs.server.players[slot].spawn_protection);
        let before = clock_runs(&hs.server.world.sim_tally);
        hs.tick();
        assert_eq!(hs.server.tick_counter, t + 1);
        assert_eq!(hs.server.world_time, (w + 1) % 24_000);
        assert_eq!(clock_runs(&hs.server.world.sim_tally), (before.0 + 1, before.1 + 1));
        assert_eq!(hs.server.players[slot].spawn_protection, p - 1, "the protection counts server ticks, once");
    }
}

#[test]
fn one_lent_tick_steps_the_authority_once_and_the_host_reads_it_back() {
    let (mut hs, mut host) = start_lent("once");
    let (_client, slot) = join_guest_lent(&mut hs, &mut host, "Once");
    crate::spawn_protection::arm(&mut hs.server.players[slot]);
    for _ in 0..3 {
        let (t, w, p) = (hs.server.tick_counter, hs.server.world_time, hs.server.players[slot].spawn_protection);
        let before = host.world.sim_tally;
        host.lend_tick(&mut hs);
        assert_eq!(hs.server.tick_counter, t + 1, "one logical tick, one step");
        assert_eq!(hs.server.world_time, (w + 1) % 24_000);
        assert_eq!((host.clock.tick_counter, host.clock.world_time), (hs.server.tick_counter, hs.server.world_time));
        let faults = host.world.sim_tally.one_tick_faults(&before);
        assert!(
            faults.iter().all(|(s, _)| !matches!(s, SimSystem::Clock | SimSystem::Weather)),
            "the clock and weather ran once on the host's world: {faults:?}"
        );
        assert_eq!(clock_runs(&host.world.sim_tally), (clock_runs(&before).0 + 1, clock_runs(&before).1 + 1));
        assert_eq!(hs.server.players[slot].spawn_protection, p - 1, "the protection counts server ticks, once");
    }
}

/// `/time set` and `/time speed` on a lent world write the authority
/// (`GameState::write_clock` writes these fields); the next phase steps from
/// them and the host's copy follows.
#[test]
fn time_set_and_speed_on_a_lent_world_land_on_the_authority() {
    let (mut hs, mut host) = start_lent("time");
    host.lend_tick(&mut hs);
    hs.server.world_time = 18_000;
    hs.server.world_time_step = 8;
    host.lend_tick(&mut hs);
    assert_eq!(hs.server.world_time, 18_008);
    assert_eq!(host.clock.world_time, 18_008, "the client sees it the next tick");
    host.lend_tick(&mut hs);
    assert_eq!(hs.server.world_time, 18_016, "the step holds");
}

/// A trial's weather lock reaches the authority through its phase, the
/// release clears the pinned window at once, and the Workshop never rains.
#[test]
fn a_weather_lock_pins_and_releases_on_the_authority_and_the_workshop_never_rains() {
    let (mut hs, mut host) = start_lent("lock");
    host.lend_tick(&mut hs);
    let mut tally = host.world.sim_tally;
    let seed = |h: &OwnedSimParts| crate::sim_clock::ClockSeed {
        world_time: h.clock.world_time,
        world_time_step: 1,
        tick_counter: h.clock.tick_counter,
        weather: h.clock.weather,
        weather_lock_applied: None,
    };
    let phase = |hs: &mut HostedServer, host: &mut OwnedSimParts, tally: &mut crate::sim_lend::SimTally, has_weather: bool, lock: Option<&'static str>| {
        let back = hs.server.run_clock_phase(&seed(host), has_weather, lock, tally).expect("one phase");
        let clock = host.clock;
        crate::sim_lend::LentSim::lend(hs, host.parts(), clock).tick();
        back
    };
    let pinned = phase(&mut hs, &mut host, &mut tally, true, Some("storm"));
    assert!(pinned.tick_counter < pinned.weather.storm_until, "the storm is pinned");
    assert_eq!(hs.server.weather, pinned.weather);
    let again = phase(&mut hs, &mut host, &mut tally, true, Some("storm"));
    assert!(again.weather.storm_until > pinned.weather.storm_until, "re-stamped each tick");
    let released = phase(&mut hs, &mut host, &mut tally, true, None);
    assert_eq!(released.weather, Weather::CLEAR, "released at once");
    assert_eq!(released.weather_lock_applied, None);
    // The Workshop: no weather at all, whatever the window held.
    hs.server.weather = Weather { rain_until: u64::MAX, storm_until: u64::MAX };
    for _ in 0..3 {
        let back = phase(&mut hs, &mut host, &mut tally, false, None);
        assert_eq!(back.weather, Weather::CLEAR);
        assert!(hs.server.take_lent_strike().is_none(), "no weather, no bolts");
    }
}

/// One lightning roll per tick per world on a lent world: the server rolls
/// it in its phase and lands it in its world phase round its slot 0 (the
/// host's own body); the host only gets the event to present.
#[test]
fn a_lent_worlds_strike_is_the_servers_landed_round_the_host() {
    let (mut hs, mut host) = start_lent("strike");
    host.lend_tick(&mut hs);
    let (storm, t) = storm_and_strike_tick(hs.server.tick_counter + 1);
    hs.server.weather = storm;
    hs.server.tick_counter = t - 1;
    let before = host.world.sim_tally;
    host.lend_tick(&mut hs);
    assert_eq!(hs.server.tick_counter, t);
    assert_eq!(clock_runs(&host.world.sim_tally), (clock_runs(&before).0 + 1, clock_runs(&before).1 + 1), "one roll");
    let strike = hs.server.take_lent_strike().expect("the server landed this tick's bolt");
    assert!(hs.server.take_lent_strike().is_none(), "taken once");
    let anchor = hs.server.players[0].player.eye_pos();
    let h = crate::weather::strike_roll(storm, t, true).unwrap();
    assert_eq!(strike.hash, h);
    // The column the hash picks round the host's own body (read after the
    // strike, so a fire that took now tops it).
    let (cell, _) = crate::sim_clock::strike_ground(&host.world, anchor, h);
    assert_eq!((strike.cell.0, strike.cell.2), (cell.0, cell.2), "round the host's own body");
    assert_eq!(strike.ground.y, strike.cell.1 as f32, "the bolt's foot is the fire cell");
    if strike.ignited {
        let at = strike.cell;
        assert_eq!(host.world.get_block(at.0, at.1, at.2), crate::block::FIRE, "the fire is in the host's world");
    }
    // The next tick (no strike on it) presents nothing.
    host.lend_tick(&mut hs);
    assert!(hs.server.take_lent_strike().is_none());
}

/// A dedicated server has no host: its bolts land round the lowest-index
/// body in the world, or the world-spawn column when nobody is in it —
/// deterministically (two servers, the same strike).
#[test]
fn a_dedicated_servers_strike_anchor_is_deterministic() {
    let strike_cell = |with_joiner: bool, tag: &str| {
        let mut hs = dedicated(tag);
        let joiner = with_joiner.then(|| join_guest(&mut hs, "Anchor"));
        let (storm, t) = storm_and_strike_tick(hs.server.tick_counter + 1);
        hs.server.weather = storm;
        hs.server.tick_counter = t - 1;
        let fire_before = hs.server.fire.active();
        hs.tick();
        let h = crate::weather::strike_roll(storm, t, true).unwrap();
        let anchor = match &joiner {
            Some((_, slot)) => hs.server.players[*slot].player.eye_pos(),
            None => crate::sim_clock::strike_anchor(std::iter::empty(), hs.server.spawn_column),
        };
        // Read after the strike: a fire that took now tops the column, one
        // below the cell `strike_ground` names.
        let (cell, _) = crate::sim_clock::strike_ground(&hs.server.world, anchor, h);
        let lit = hs.server.world.get_block(cell.0, cell.1 - 1, cell.2) == crate::block::FIRE;
        let cell = (cell.0, cell.2);
        assert!(hs.server.take_lent_strike().is_none(), "a dedicated server presents to nobody");
        (cell, lit, hs.server.fire.active() > fire_before)
    };
    let a = strike_cell(true, "anchor-a");
    let b = strike_cell(true, "anchor-b");
    assert_eq!(a.0, b.0, "the same world, the same body, the same bolt");
    assert_eq!(a.1, a.2, "a lit cell is a fire the server tracks");
    let none = strike_cell(false, "anchor-none");
    let none2 = strike_cell(false, "anchor-none2");
    assert_eq!(none.0, none2.0, "nobody in the world: the spawn column, deterministically");
    assert_eq!(none.1, none.2);
    assert!(a.2 && none.2, "the dedicated server lit both bolts: {a:?} {none:?}");
}

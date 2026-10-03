//! P9 weather sync — the server owns the rain/storm window (`weather.rs`'s
//! `advance`, ticked once per `GameServer::tick()` in `server.rs`) and puts
//! it on the wire as ticks-remaining (`Weather::ticks_left`) on every
//! `StateUpdatePacket`. These tests drive the real `GameServer` tick loop
//! through `TestHost` and assert what a broadcast would carry, via
//! `TestHost::weather_ticks_left` (the harness bypasses the transport layer
//! `hosted_server.rs` builds packets through — see `test_harness.rs`).
//!
//! Tick 2400 is not arbitrary: it's the deterministic tick at which
//! `weather::advance`'s hash first rolls rain (and, at this tick, a storm
//! too) starting from a fresh `Weather::CLEAR` — found by walking the same
//! formula `weather.rs::advance` uses. Picking a known tick keeps this test
//! fast (a few thousand real `GameServer::tick()` calls, not the 200_000
//! the pure-function tests in `weather.rs` run) and deterministic.

use crate::test_harness::{TestConfig, TestHost};

#[test]
fn a_hosted_world_broadcasts_a_counting_down_rain_window() {
    let mut host = TestHost::start_with(TestConfig::default());
    // Before the tick-2400 roll, the sky is clear — nothing to broadcast.
    host.tick(2399);
    assert_eq!(
        host.weather_ticks_left(),
        (0, 0),
        "must stay clear right up to the deterministic roll tick"
    );

    // Tick 2400: the deterministic roll opens a rain (+ storm) window running
    // to tick 3600 (see module doc for how this tick was found).
    host.tick(1);
    let (rain_at_open, storm_at_open) = host.weather_ticks_left();
    assert!(rain_at_open > 0, "the server must broadcast a positive rain window once it rains");
    assert!(storm_at_open > 0, "this particular roll also upgrades to a storm");

    // Ticking further must count DOWN, not re-roll or jump — the server
    // advances the SAME window every tick (`GameServer::tick`), it doesn't
    // draw a fresh one.
    host.tick(400);
    let (rain_later, storm_later) = host.weather_ticks_left();
    assert!(
        rain_later < rain_at_open,
        "rain_ticks_left must count down tick over tick: {rain_at_open} -> {rain_later}"
    );
    assert!(
        storm_later < storm_at_open,
        "storm_ticks_left must count down tick over tick: {storm_at_open} -> {storm_later}"
    );

    // And once the window's fully elapsed (tick 3600), it's back to zero —
    // the server's fire-dousing rain and the broadcast agree the sky cleared.
    host.tick(800); // now at tick 3600
    assert_eq!(
        host.weather_ticks_left(),
        (0, 0),
        "ticks-left must reach zero once the rolled window has fully elapsed"
    );
}

#[test]
fn a_workshop_world_always_broadcasts_zero_weather() {
    let mut host = TestHost::start_with(TestConfig::default());
    // The Workshop is a hard exclusion (`weather::world_has_weather`) — set it
    // directly on the authoritative world, same as `server.rs::initial_load`
    // does from `WorldMeta.is_workshop`.
    host.server.world.is_workshop = true;

    // Run well past the tick-2400 roll point that produces rain in a normal
    // world (proven by the sibling test above) — a Workshop must never.
    host.tick(4000);
    assert_eq!(
        host.weather_ticks_left(),
        (0, 0),
        "a Workshop must broadcast permanently-clear weather, even past a normal world's roll tick"
    );
}

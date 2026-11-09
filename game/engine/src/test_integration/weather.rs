//! P9 weather sync — the server owns the rain/storm window (`weather.rs`'s
//! `advance`, ticked once per `GameServer::tick()` in `server.rs`) and puts
//! it on the wire as ticks-remaining (`Weather::ticks_left`) on every
//! `StateUpdatePacket`. These tests drive the real `GameServer` tick loop
//! through `TestHost` and assert what a broadcast would carry, via
//! `TestHost::weather_ticks_left` (the harness bypasses the transport layer
//! `hosted_server.rs` builds packets through — see `test_harness.rs`).
//!
//! The tick at which the first rain opens is not arbitrary: it is where
//! `weather::advance`'s hash first rolls rain from a fresh `Weather::CLEAR` in
//! the host's world (the roll mixes in the world seed, so each world has its
//! own). `first_rain` walks the same function to find it. Picking a known tick
//! keeps these tests fast (a few thousand real `GameServer::tick()` calls, not
//! the long runs the pure-function tests in `weather.rs` do) and deterministic.

use crate::test_harness::{TestConfig, TestHost};
use crate::weather::{advance, Weather};

/// The tick the first rain opens in the world with `seed`, and how long it
/// lasts (a plain rain or a storm: either way a window).
fn first_rain(seed: u32) -> (u64, u64) {
    let mut w = Weather::CLEAR;
    for t in 1..200_000u64 {
        w = advance(w, t, true, seed);
        if t < w.rain_until {
            return (t, w.rain_until - t);
        }
    }
    panic!("seed {seed} never rained");
}

#[test]
fn a_hosted_world_broadcasts_a_counting_down_rain_window() {
    let mut host = TestHost::start_with(TestConfig::default());
    let (open, len) = first_rain(host.server.biome_gen.seed);
    // Before the deterministic roll, the sky is clear: nothing to broadcast.
    host.tick((open - 1) as u32);
    assert_eq!(
        host.weather_ticks_left(),
        (0, 0),
        "must stay clear right up to the deterministic roll tick ({open})"
    );

    // The roll tick opens a rain window of `len` ticks.
    host.tick(1);
    let (rain_at_open, _storm_at_open) = host.weather_ticks_left();
    assert_eq!(rain_at_open as u64, len, "the server broadcasts the whole window once it rains");

    // Ticking further must count DOWN, not re-roll or jump: the server
    // advances the SAME window every tick (`GameServer::tick`), it doesn't
    // draw a fresh one.
    host.tick(400);
    let (rain_later, _) = host.weather_ticks_left();
    assert!(
        rain_later < rain_at_open,
        "rain_ticks_left must count down tick over tick: {rain_at_open} -> {rain_later}"
    );

    // And once the window's fully elapsed, it's back to zero: the server's
    // fire-dousing rain and the broadcast agree the sky cleared.
    host.tick((len - 400) as u32);
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

    // Run well past the roll point that produces rain in a normal world
    // (proven by the sibling test above): a Workshop must never.
    let (open, len) = first_rain(host.server.biome_gen.seed);
    host.tick((open + len + 400) as u32);
    assert_eq!(
        host.weather_ticks_left(),
        (0, 0),
        "a Workshop must broadcast permanently-clear weather, even past a normal world's roll tick"
    );
}

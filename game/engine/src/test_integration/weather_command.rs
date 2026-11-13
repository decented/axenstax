//! `/weather` on a lent world: the command's window is written through the
//! authority (`GameServer::write_weather`, as `GameState::write_weather` does
//! next to the client's own copy), and it has to survive the server's clock
//! phase and the host's read-back, because that phase owns the weather.

use super::lent_world::{join_guest_lent, start_lent};
use crate::hosted_server::HostedServer;
use crate::sim_lend::OwnedSimParts;
use crate::transport::ChannelClientTransport;
use crate::weather::{advance, command_window, Sky, Weather};

/// What `GameState::write_weather` does on a lent world: the authority's window
/// and the host client's copy.
fn write_weather(hs: &mut HostedServer, host: &mut OwnedSimParts, w: Weather) {
    hs.server.write_weather(w);
    host.clock.weather = w;
}

/// Every tick at or after `from`, on a minute, where `advance` would roll rain
/// out of a clear sky in the world with `seed`.
fn would_roll(seed: u32, from: u64) -> impl Iterator<Item = u64> {
    (from.div_ceil(1200)..).map(|k| k * 1200).filter(move |&t| t > 0 && advance(Weather::CLEAR, t, true, seed).rain_until > t)
}

/// Put the server at `tick - 1` so the next lent tick is `tick`.
fn jump_to(hs: &mut HostedServer, tick: u64) {
    hs.server.tick_counter = tick - 1;
}

#[test]
fn clear_holds_the_sky_across_a_tick_that_would_roll_rain_and_the_read_back_keeps_the_hold() {
    // Control: ending the rain alone (a clear window with no hold) does NOT keep
    // the sky clear: the server rolls rain at the next would-roll minute.
    let (mut hs, mut host) = start_lent("clear-control");
    host.lend_tick(&mut hs);
    let seed = hs.server.biome_gen.seed;
    let t = would_roll(seed, hs.server.tick_counter + 2).next().unwrap();
    write_weather(&mut hs, &mut host, Weather::CLEAR);
    jump_to(&mut hs, t);
    host.lend_tick(&mut hs);
    assert!(hs.server.weather.rain_until > t, "without the hold the tick rolls rain");

    // `/weather clear 600` at the tick before: held for 12,000 ticks.
    let (mut hs, mut host) = start_lent("clear-hold");
    host.lend_tick(&mut hs);
    let seed = hs.server.biome_gen.seed;
    let t = would_roll(seed, hs.server.tick_counter + 2).next().unwrap();
    let held = command_window(Sky::Clear, 600, t - 1);
    assert_eq!(held.clear_until, t - 1 + 12_000);
    write_weather(&mut hs, &mut host, held);
    jump_to(&mut hs, t);
    host.lend_tick(&mut hs);
    assert_eq!(hs.server.weather, held, "the phase rolled nothing and kept the hold");
    assert_eq!(host.clock.weather, held, "and the host's read-back kept it too");

    // Every would-roll minute inside the hold stays dry...
    let hold_ends = held.clear_until;
    for u in would_roll(seed, t + 1).take_while(|&u| u < hold_ends) {
        jump_to(&mut hs, u);
        host.lend_tick(&mut hs);
        assert_eq!(hs.server.weather.rain_until, 0, "tick {u}: dry inside the hold");
        assert_eq!(host.clock.weather.clear_until, hold_ends, "tick {u}: the read-back keeps the hold");
    }
    // ...and then the natural cycle resumes.
    let after = would_roll(seed, hold_ends).next().unwrap();
    jump_to(&mut hs, after);
    host.lend_tick(&mut hs);
    assert!(hs.server.weather.rain_until > after, "the cycle resumes once the hold is over");
}

#[test]
fn rain_during_a_storm_ends_the_thunder_on_the_authority() {
    let (mut hs, mut host) = start_lent("rain-in-storm");
    host.lend_tick(&mut hs);
    let now = hs.server.tick_counter;
    let seed = hs.server.biome_gen.seed;
    let storm = command_window(Sky::Storm, 600, now);
    write_weather(&mut hs, &mut host, storm);
    host.lend_tick(&mut hs);
    assert!(hs.server.weather.storm_until > hs.server.tick_counter + 10_000, "a long storm");
    let now = hs.server.tick_counter;
    let rain = command_window(Sky::Rain, 60, now);
    assert_eq!(rain.storm_until, 0, "`rain` zeroes the storm window");
    write_weather(&mut hs, &mut host, rain);
    host.lend_tick(&mut hs);
    assert_eq!(hs.server.weather.storm_until, 0, "and the phase leaves it zero");
    assert_eq!(host.clock.weather.storm_until, 0);
    assert!(hs.server.weather.rain_until > hs.server.tick_counter, "still raining");
    // No bolt can come from it: the old storm window would have thrown some.
    let t = hs.server.tick_counter;
    assert!((t + 1..t + 1200).all(|u| crate::weather::strike_roll(hs.server.weather, u, true, seed).is_none()));
}

/// The last weather a joiner was told: `(rain_ticks_left, storm_ticks_left)`.
fn weather_told(client: &ChannelClientTransport) -> Option<(u32, u32)> {
    use crate::transport::ClientTransport;
    let mut last = None;
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = crate::protocol::deserialize_header(&pkt)
            && ptype == crate::protocol::PacketType::StateUpdate
            && let Ok(s) = crate::protocol::safe_deserialize::<crate::protocol::StateUpdatePacket>(payload)
        {
            last = Some((s.rain_ticks_left, s.storm_ticks_left));
        }
    }
    last
}

#[test]
fn a_storm_reaches_a_joiners_synced_weather_with_no_hold_and_a_clear_sky_ends_it() {
    let (mut hs, mut host) = start_lent("joiner-sync");
    let (client, _slot) = join_guest_lent(&mut hs, &mut host, "Watcher");
    let _ = weather_told(&client);
    let now = hs.server.tick_counter;
    write_weather(&mut hs, &mut host, command_window(Sky::Storm, 60, now));
    let mut told = None;
    for _ in 0..40 {
        host.lend_tick(&mut hs);
        told = weather_told(&client).or(told);
    }
    let (rain, storm) = told.expect("the joiner was sent a state update");
    assert!(rain > 0 && storm > 0 && rain <= 1200 && storm <= 1200, "a minute's storm, told as ticks left: {rain}, {storm}");
    // The joiner rebuilds the window from the duration (`network_receive`): a
    // storm, and no clear hold (the wire never carries it).
    let synced = Weather::from_ticks_left(500, rain, storm);
    assert!(500 < synced.storm_until && synced.clear_until == 0);

    // `/weather clear`: the joiner is told the sky is empty, and still holds no hold.
    let now = hs.server.tick_counter;
    write_weather(&mut hs, &mut host, command_window(Sky::Clear, 600, now));
    let mut told = None;
    for _ in 0..40 {
        host.lend_tick(&mut hs);
        told = weather_told(&client).or(told);
    }
    assert_eq!(told, Some((0, 0)), "a clear sky is told as no rain and no storm");
    assert!(hs.server.weather.clear_until > hs.server.tick_counter, "the authority still holds the sky");
}

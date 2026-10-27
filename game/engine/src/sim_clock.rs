//! D4 slice 2 — the clock and weather phase, one implementation for every
//! world.
//!
//! A world's clock is four pieces of state that advance together once per
//! logical tick: the monotonic `tick_counter`, the cyclic day clock
//! `world_time` (stepped by `world_time_step`, `/time speed`), the rain /
//! storm window (`weather::advance`, then a scenario's `weather_lock`) and,
//! on a server, the [`NightCalendar`] a joiner's sleep is judged by. A storm
//! then rolls for a lightning strike from the post-lock window, and a strike
//! picks its ground and starts a fire.
//!
//! **Who runs it.** Exactly one owner per world:
//! - standalone single-player (the default, and the web build): the client,
//!   from `GameState::tick` — [`next_tick`] where it always stepped its
//!   counter, then [`next_world_time`], [`step_weather`] and the strike at the
//!   day-clock site further down, so its cadence is unchanged;
//! - a lent world (LAN / online host, `--solo-hosted`): the embedded
//!   `GameServer`. Its host client asks for the phase at the point of its
//!   fixed tick where it used to step its own counter
//!   (`GameServer::run_clock_phase`, guarded by a phase token), reads the
//!   authority's clock and weather back, and the server's later world phase
//!   (`GameServer::tick`, inside the lend) does not step it again; it only
//!   lands the strike the phase rolled ([`strike_ground`], [`ignite_strike`]).
//! - a dedicated (or `--no-lend`) server: `GameServer::tick`, inline, as
//!   before ([`phase`]).
//!
//! Everything here is pure (no `GameState`, no `GameServer`), so one set of
//! functions serves all three.

use crate::item_actions::NightCalendar;
use crate::weather::Weather;
use crate::world::World;

/// Ticks in one day of `world_time`.
pub const DAY_TICKS: u32 = 24_000;

/// Radius (blocks, each axis) round the anchor a strike lands in.
const STRIKE_SPREAD: u64 = 49;

/// The monotonic counter's next value.
pub const fn next_tick(tick: u64) -> u64 {
    tick.wrapping_add(1)
}

/// The day clock's next value at `step` per tick (`/time speed`). A step of
/// 0 (tests freezing the sky) holds the clock.
pub const fn next_world_time(world_time: u32, step: u32) -> u32 {
    ((world_time as u64 + step as u64) % DAY_TICKS as u64) as u32
}

/// What a tick's weather step may do.
#[derive(Clone, Copy, Debug)]
pub struct WeatherRules<'a> {
    /// The world has weather at all (`weather::world_has_weather`: never in
    /// the Workshop).
    pub has_weather: bool,
    /// Roll the window (`weather::advance`). A joined client does not: its
    /// window is the server's, synced from each `StateUpdate`.
    pub roll: bool,
    /// The scenario's `weather_lock` in force this tick, already dropped by
    /// the caller where the world has no weather.
    pub lock: Option<&'a str>,
}

/// Advance the weather window one tick: the roll (if this side rolls), then
/// the scenario lock, re-stamped every tick and released the tick it goes
/// away (`weather::apply_lock`). `lock_applied` is the lock pinned last tick,
/// updated here.
pub fn step_weather(
    current: Weather,
    tick: u64,
    rules: WeatherRules<'_>,
    lock_applied: &mut Option<String>,
) -> Weather {
    let rolled = if rules.roll {
        crate::weather::advance(current, tick, rules.has_weather)
    } else {
        current
    };
    let lock_changed = rules.lock != lock_applied.as_deref();
    let after = crate::weather::apply_lock(rolled, rules.lock, lock_changed, tick);
    if lock_changed {
        *lock_applied = rules.lock.map(str::to_string);
    }
    after
}

/// The roll for a lightning strike this tick, from the post-lock window
/// (`weather::strike_roll`): the strike hash, or `None`.
pub fn strike_roll(weather: Weather, tick: u64, has_weather: bool) -> Option<u64> {
    crate::weather::strike_roll(weather, tick, has_weather)
}

/// A host client's clock, handed to its lending server so the server can
/// adopt it the first time (`GameServer::adopt_host_clock`). Never copied
/// over a clock the server already owns.
#[derive(Clone, Debug)]
pub struct ClockSeed {
    pub world_time: u32,
    pub world_time_step: u32,
    pub tick_counter: u64,
    pub weather: Weather,
    pub weather_lock_applied: Option<String>,
}

/// The authority's clock after a lent world's phase, read back into its host
/// client's copy (presentation and the client's own rules read it).
#[derive(Clone, Debug)]
pub struct ClockReadback {
    pub world_time: u32,
    pub world_time_step: u32,
    pub tick_counter: u64,
    pub weather: Weather,
    pub weather_lock_applied: Option<String>,
    /// The day-clock step the phase applied.
    pub stepped: u32,
}

/// A server's clock state, borrowed field by field from `GameServer` for one
/// [`phase`].
pub struct ClockView<'a> {
    pub world_time: &'a mut u32,
    pub world_time_step: u32,
    pub tick_counter: &'a mut u64,
    pub weather: &'a mut Weather,
    pub weather_lock_applied: &'a mut Option<String>,
    pub night_calendar: &'a mut NightCalendar,
}

/// What one [`phase`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseOutcome {
    /// The day-clock step applied (`world_time_step`): what a reputation day
    /// counts.
    pub stepped: u32,
    /// This tick's lightning strike hash, if the storm threw one.
    pub strike: Option<u64>,
}

/// One whole clock phase, in the order the standalone client runs its
/// pieces: the counter, the day clock, the night calendar (a dusk starts the
/// next night), the weather window, the strike roll.
pub fn phase(view: ClockView<'_>, rules: WeatherRules<'_>) -> PhaseOutcome {
    *view.tick_counter = next_tick(*view.tick_counter);
    *view.world_time = next_world_time(*view.world_time, view.world_time_step);
    view.night_calendar.observe(*view.world_time);
    *view.weather = step_weather(*view.weather, *view.tick_counter, rules, view.weather_lock_applied);
    PhaseOutcome {
        stepped: view.world_time_step,
        strike: strike_roll(*view.weather, *view.tick_counter, rules.has_weather),
    }
}

/// Where a strike rolled by a server lands: round the lowest-index player
/// body in the world (dead or alive; on a hosting server slot 0 is the host's
/// own player, so a host's strikes land round the host as they always did).
/// `None` — nobody in the world — throws no bolt at all: an empty dedicated
/// server must not burn its spawn overnight. `bodies`: `(in_world, eye
/// position)` by slot.
pub fn strike_anchor(bodies: impl IntoIterator<Item = (bool, glam::Vec3)>) -> Option<glam::Vec3> {
    bodies.into_iter().find_map(|(in_world, eye)| in_world.then_some(eye))
}

/// A strike's ground: the column the hash picks round `anchor` and its top
/// solid block. Returns the cell the fire goes in (one above the ground) and
/// the bolt's foot.
pub fn strike_ground(world: &World, anchor: glam::Vec3, h: u64) -> ((i32, i32, i32), glam::Vec3) {
    let half = (STRIKE_SPREAD / 2) as f32;
    let sx = anchor.x + (((h >> 8) % STRIKE_SPREAD) as f32 - half);
    let sz = anchor.z + (((h >> 16) % STRIKE_SPREAD) as f32 - half);
    let (bx, bz) = (sx.floor() as i32, sz.floor() as i32);
    let mut surface_y = 0;
    for y in (0..=world.column_top_y(bx, bz)).rev() {
        if world.get_block(bx, y, bz) != crate::block::AIR {
            surface_y = y;
            break;
        }
    }
    let ground = glam::Vec3::new(bx as f32 + 0.5, surface_y as f32 + 1.0, bz as f32 + 0.5);
    ((bx, surface_y + 1, bz), ground)
}

/// A strike starts a fire (MC parity) through the normal fire path, so the
/// world toggle, burnout and cap all apply, and relights the cell. `true`
/// when the fire took.
pub fn ignite_strike(
    world: &mut World,
    fire: &mut crate::fire::FireSystem,
    registry: &crate::block::BlockRegistry,
    cell: (i32, i32, i32),
    tick: u64,
) -> bool {
    let (x, y, z) = cell;
    if !fire.ignite(world, x, y, z, tick) {
        return false;
    }
    crate::lighting::update_for_block_change(world, cell, crate::block::AIR, crate::block::FIRE, registry);
    true
}

/// A strike a server landed on its host client's lent world, for the host to
/// present (flash, bolt, thunder) and to remesh round when the fire took. A
/// presentation record only: the roll, the ground and the fire were the
/// server's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrikeEvent {
    pub hash: u64,
    pub ground: glam::Vec3,
    pub cell: (i32, i32, i32),
    pub ignited: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storm(until: u64) -> Weather {
        Weather { rain_until: until, storm_until: until }
    }

    #[test]
    fn the_day_clock_wraps_and_a_zero_step_holds_it() {
        assert_eq!(next_world_time(23_999, 1), 0);
        assert_eq!(next_world_time(23_990, 64), 54);
        assert_eq!(next_world_time(500, 0), 500);
        assert_eq!(next_tick(u64::MAX), 0);
    }

    #[test]
    fn one_phase_steps_every_piece_exactly_once() {
        let (mut wt, mut tick, mut weather, mut applied, mut night) =
            (7500u32, 41u64, Weather::CLEAR, None, NightCalendar::default());
        let out = phase(
            ClockView {
                world_time: &mut wt,
                world_time_step: 3,
                tick_counter: &mut tick,
                weather: &mut weather,
                weather_lock_applied: &mut applied,
                night_calendar: &mut night,
            },
            WeatherRules { has_weather: true, roll: true, lock: None },
        );
        assert_eq!((wt, tick), (7503, 42));
        assert_eq!(out.stepped, 3);
    }

    /// The phase and the standalone client's pieces are the same steps in
    /// the same order: over a long run (rain windows, storms, strikes, a lock
    /// taken and released) they agree tick for tick.
    #[test]
    fn the_server_phase_matches_the_standalone_pieces_tick_for_tick() {
        let (mut wt, mut tick, mut weather, mut applied, mut night) =
            (7500u32, 0u64, Weather::CLEAR, None, NightCalendar::default());
        let (mut c_wt, mut c_tick, mut c_weather, mut c_applied) = (7500u32, 0u64, Weather::CLEAR, None);
        let mut strikes = 0;
        for i in 0..200_000u64 {
            let lock = (50_000..60_000).contains(&i).then_some("storm");
            let rules = WeatherRules { has_weather: true, roll: true, lock };
            let out = phase(
                ClockView {
                    world_time: &mut wt,
                    world_time_step: 1,
                    tick_counter: &mut tick,
                    weather: &mut weather,
                    weather_lock_applied: &mut applied,
                    night_calendar: &mut night,
                },
                rules,
            );
            c_tick = next_tick(c_tick);
            c_wt = next_world_time(c_wt, 1);
            c_weather = step_weather(c_weather, c_tick, rules, &mut c_applied);
            let c_strike = strike_roll(c_weather, c_tick, true);
            assert_eq!((wt, tick, weather, out.strike), (c_wt, c_tick, c_weather, c_strike), "tick {i}");
            strikes += usize::from(out.strike.is_some());
        }
        assert!(strikes > 10, "the run saw storms and strikes ({strikes})");
    }

    #[test]
    fn the_workshop_never_rains_even_under_a_storm_lock_the_caller_dropped() {
        let mut applied = None;
        let mut w = storm(5_000);
        for t in 1..=4_000 {
            w = step_weather(w, t, WeatherRules { has_weather: false, roll: true, lock: None }, &mut applied);
            assert_eq!(w, Weather::CLEAR);
            assert_eq!(strike_roll(w, t, false), None);
        }
    }

    #[test]
    fn a_lock_pins_the_window_and_its_release_clears_it_at_once() {
        let mut applied = None;
        let rules = |lock| WeatherRules { has_weather: true, roll: true, lock };
        let pinned = step_weather(Weather::CLEAR, 10, rules(Some("storm")), &mut applied);
        assert!(pinned.storm_until > 10 && applied.as_deref() == Some("storm"));
        let released = step_weather(pinned, 11, rules(None), &mut applied);
        assert_eq!(released, Weather::CLEAR, "the pinned residue goes the tick the lock does");
        assert_eq!(applied, None);
    }

    #[test]
    fn the_anchor_is_the_lowest_slot_in_the_world_and_nobody_means_no_bolt() {
        let a = glam::Vec3::new(1.0, 70.0, 2.0);
        let b = glam::Vec3::new(-40.0, 64.0, 9.0);
        assert_eq!(strike_anchor([(false, a), (true, b)]), Some(b), "slot 0 left: slot 1");
        assert_eq!(strike_anchor([(true, a), (true, b)]), Some(a));
        assert_eq!(strike_anchor([(false, a)]), None, "nobody in the world");
        assert_eq!(strike_anchor(std::iter::empty()), None);
    }

    #[test]
    fn a_strike_lands_on_the_top_block_and_lights_a_fire() {
        let mut world = World::new();
        let registry = crate::block::BlockRegistry::new();
        let mut fire = crate::fire::FireSystem::new();
        for x in -30..30 {
            for z in -30..30 {
                world.set_block(x, 40, z, crate::block::STONE);
            }
        }
        let (cell, ground) = strike_ground(&world, glam::Vec3::new(0.5, 80.0, 0.5), 0xABCD_1234);
        assert_eq!(cell.1, 41);
        assert_eq!(ground.y, 41.0);
        assert!(ignite_strike(&mut world, &mut fire, &registry, cell, 100));
        assert_eq!(world.get_block(cell.0, cell.1, cell.2), crate::block::FIRE);
        assert!(!ignite_strike(&mut world, &mut fire, &registry, cell, 100), "a burning cell takes no second fire");
    }
}

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
//!
//! **What a save keeps** ([`SavedClock`], Keep slice 2a, Spec 02 §8.4): all of
//! the above except a pending lightning strike, which is rolled and landed in
//! one tick so none outlives it. The `/time speed` step is a runtime override
//! and goes back to 1 on every world entry, as it always has.

use serde::{Deserialize, Serialize};

use crate::item_actions::NightCalendar;
use crate::weather::Weather;
use crate::world::World;

/// Ticks in one day of `world_time`.
pub const DAY_TICKS: u32 = 24_000;

/// The largest `tick_counter` a save may restore: years past anything a world
/// reaches at 20 TPS, and far enough from `u64::MAX` that nothing keyed on the
/// tick can overflow.
const MAX_SAVED_TICK: u64 = 1 << 56;

/// How far past the saved tick a weather window may end: weather rolls windows
/// of minutes, so a longer one is a damaged file, and one that never ends would
/// rain for good (the window is only re-rolled once it is over).
const MAX_SAVED_WEATHER_AHEAD: u64 = 1 << 32;

/// The longest trial weather lock a save may restore (a scenario's lock name).
const MAX_SAVED_LOCK_LEN: usize = 64;

/// A world's authority clock as `world.dat` carries it
/// (`WorldSave.authority_clock`). Positional bincode inside an append-only
/// field: a change to the encoded layout here moves
/// `save_format::SAVE_LAYOUT_REVISION` and misreads every older save's clock,
/// so a new clock field rides its own appended `WorldSave` field instead
/// (`clear_until`, below).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedClock {
    pub tick_counter: u64,
    pub world_time: u32,
    pub rain_until: u64,
    pub storm_until: u64,
    /// The `/weather clear` hold (`Weather::clear_until`). NOT part of this
    /// record's encoding (`serde(skip)`): it is saved as the appended
    /// `WorldSave.weather_clear_until` field: written from here by
    /// [`SavedClock::clear_hold_for_save`] and folded back in by the reader with
    /// [`SavedClock::fold_clear_hold`]. `0` = no hold, and what a save from
    /// before the field loads.
    #[serde(skip)]
    pub clear_until: u64,
    pub weather_lock_applied: Option<String>,
    pub nights: u32,
    pub was_night: Option<bool>,
    /// Which run of [`NightCalendar`] `nights` counts in. A joiner record's
    /// night number means something only under the same id
    /// (`JoinerSave::restore_vitals`): a world whose calendar restarted (an
    /// old save) has a new one, so old night numbers are ignored.
    pub calendar_id: [u8; 16],
    /// The reputation day's progress (`reputation_day::DayClock`); its owner
    /// is per run and is drawn afresh.
    pub day_remainder: u32,
    pub day_last_tick: Option<u64>,
}

impl SavedClock {
    /// The appended `WorldSave.weather_clear_until` value for this clock: `None`
    /// (one byte) when there is no hold, so a world that never used `/weather`
    /// saves canonically.
    pub fn clear_hold_for_save(&self) -> Option<u64> {
        (self.clear_until != 0).then_some(self.clear_until)
    }

    /// Fold a loaded `WorldSave.weather_clear_until` into the clock it belongs
    /// to. `None` (a save from before the field) is no hold. A hold further
    /// past the clock's tick than a window can ever be is a damaged file and is
    /// dropped, never kept: it would hold the sky clear for good (the same
    /// bound as the rain and storm windows, [`MAX_SAVED_WEATHER_AHEAD`]).
    pub fn fold_clear_hold(&mut self, saved: Option<u64>) {
        let hold = saved.unwrap_or(0);
        self.clear_until =
            if hold <= self.tick_counter.saturating_add(MAX_SAVED_WEATHER_AHEAD) { hold } else { 0 };
    }

    /// Could a build have written this? A clock that fails (a damaged or
    /// hostile file) is not restored: the world loads as an old save does.
    pub fn is_valid(&self) -> bool {
        self.tick_counter <= MAX_SAVED_TICK
            && self.rain_until <= self.tick_counter + MAX_SAVED_WEATHER_AHEAD
            && self.storm_until <= self.tick_counter + MAX_SAVED_WEATHER_AHEAD
            && self.clear_until <= self.tick_counter + MAX_SAVED_WEATHER_AHEAD
            && self.world_time < DAY_TICKS
            && self.day_remainder < crate::reputation_day::SOLAR_UNITS_PER_DAY
            && self.day_last_tick.is_none_or(|t| t <= self.tick_counter)
            && self.weather_lock_applied.as_ref().is_none_or(|l| l.len() <= MAX_SAVED_LOCK_LEN)
    }
}

/// The clock fields a client holds that [`SavedClock`] covers (`GameState` has
/// no server of its own in single-player). The reputation day is kept as its
/// saved progress: its owner is per run and is drawn afresh.
#[derive(Clone, Debug)]
pub struct ClientClock {
    pub tick_counter: u64,
    pub world_time: u32,
    pub rain_until: u64,
    pub storm_until: u64,
    pub clear_until: u64,
    pub weather_lock_applied: Option<String>,
    pub night_calendar: crate::item_actions::NightCalendar,
    pub calendar_id: [u8; 16],
    pub day_remainder: u32,
    pub day_last_tick: Option<u64>,
}

/// Keep slice 2a — a client's clock as its save carries it. `host_calendar`:
/// the calendar and id of a hosting server that has taken this client's clock;
/// joiners sleep by that one, so it is the one kept. Everything else is the
/// client's.
pub fn clock_to_save(
    clock: &ClientClock,
    host_calendar: Option<(crate::item_actions::NightCalendar, [u8; 16])>,
) -> SavedClock {
    let (calendar, calendar_id) = host_calendar.unwrap_or((clock.night_calendar, clock.calendar_id));
    let (nights, was_night) = calendar.saved();
    SavedClock {
        tick_counter: clock.tick_counter,
        world_time: clock.world_time,
        rain_until: clock.rain_until,
        storm_until: clock.storm_until,
        clear_until: clock.clear_until,
        weather_lock_applied: clock.weather_lock_applied.clone(),
        nights,
        was_night,
        calendar_id,
        day_remainder: clock.day_remainder,
        day_last_tick: clock.day_last_tick,
    }
}

/// Keep slice 2a — carry on from a loaded save's clock. `None` (a save from
/// before the field) leaves the clock as it is. Every field of [`SavedClock`]
/// is named, so a field added to the save fails to compile until it is
/// restored here.
pub fn restore_saved_clock(clock: &mut ClientClock, saved: Option<&SavedClock>) {
    let Some(saved) = saved else { return };
    let SavedClock {
        tick_counter,
        world_time,
        rain_until,
        storm_until,
        clear_until,
        weather_lock_applied,
        nights,
        was_night,
        calendar_id,
        day_remainder,
        day_last_tick,
    } = saved;
    clock.tick_counter = *tick_counter;
    clock.world_time = *world_time;
    clock.rain_until = *rain_until;
    clock.storm_until = *storm_until;
    clock.clear_until = *clear_until;
    clock.weather_lock_applied.clone_from(weather_lock_applied);
    clock.night_calendar = crate::item_actions::NightCalendar::from_saved(*nights, *was_night);
    clock.calendar_id = *calendar_id;
    clock.day_remainder = *day_remainder;
    clock.day_last_tick = *day_last_tick;
}

/// A new calendar's identity.
pub fn fresh_calendar_id() -> [u8; 16] {
    let mut id = [0; 16];
    getrandom::fill(&mut id).expect("calendar randomness unavailable");
    id
}

/// Radius (blocks, each axis) round the anchor a strike lands in.
const STRIKE_SPREAD: u64 = 49;

/// The monotonic counter's next value.
pub const fn next_tick(tick: u64) -> u64 {
    tick.wrapping_add(1)
}

/// How often the machine phase (growth, saplings, blight, furnaces, ...) and
/// the falling-block/fluid beat run, in ticks.
pub const MACHINE_BEAT: u64 = 4;

/// The value the beat counter (`falling_tick_counter`, which gates that phase
/// on every [`MACHINE_BEAT`]th step) must hold when the world clock reads
/// `tick_counter`, so the two keep one phase. A new world starts both at 0. A
/// loaded one restores the clock mid-count while the beat counter restarts at 0;
/// left alone, a clock restored off the beat never lands a tick that is a
/// multiple of 100 on a machine beat, and crops, saplings and blight never
/// advance (the growth cadence is a multiple of 100 and 100 is a multiple of
/// the beat only from an aligned clock). Call this wherever the clock is restored.
pub const fn beat_counter_for(tick_counter: u64) -> u32 {
    (tick_counter % MACHINE_BEAT) as u32
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
    /// The world's seed, which the weather and strike rolls mix in so every
    /// world has its own timeline. Every side that rolls passes the same one.
    pub seed: u32,
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
        crate::weather::advance(current, tick, rules.has_weather, rules.seed)
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
pub fn strike_roll(weather: Weather, tick: u64, has_weather: bool, seed: u32) -> Option<u64> {
    crate::weather::strike_roll(weather, tick, has_weather, seed)
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
    pub night_calendar: NightCalendar,
    pub calendar_id: [u8; 16],
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
    pub night_calendar: NightCalendar,
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
        strike: strike_roll(*view.weather, *view.tick_counter, rules.has_weather, rules.seed),
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

    fn saved_example() -> ClientClock {
        ClientClock {
            tick_counter: 90_000,
            world_time: 15_000,
            rain_until: 91_000,
            storm_until: 90_500,
            clear_until: 95_000,
            weather_lock_applied: Some("storm".into()),
            night_calendar: crate::item_actions::NightCalendar::from_saved(12, Some(true)),
            calendar_id: [7; 16],
            day_remainder: 15_000,
            day_last_tick: Some(90_000),
        }
    }

    fn blank() -> ClientClock {
        ClientClock {
            tick_counter: 0,
            world_time: 6_000,
            rain_until: 0,
            storm_until: 0,
            clear_until: 0,
            weather_lock_applied: None,
            night_calendar: crate::item_actions::NightCalendar::default(),
            calendar_id: [0; 16],
            day_remainder: 0,
            day_last_tick: None,
        }
    }

    /// Every field of the single-player clock survives a save and a restore on
    /// a client that has just reset for the world (`blank`).
    #[test]
    fn a_clients_whole_clock_round_trips_through_its_save() {
        let before = saved_example();
        let saved = clock_to_save(&before, None);
        assert!(saved.is_valid());
        assert_eq!(saved.clear_until, 95_000);
        // Through the file's own encoding, not just the struct: the hold rides
        // the appended `WorldSave.weather_clear_until`, not the clock record
        // (`clear_until` is `serde(skip)`), so the round trip is a whole save.
        let mut file = crate::save::minimal_world_save_for_tests(1);
        file.weather_clear_until = saved.clear_hold_for_save();
        file.authority_clock = Some(saved.clone());
        let bytes = crate::save_format::encode_world_save(&file).unwrap();
        let read = crate::save::read_world_save(&bytes).unwrap();
        let read = read.clock().expect("a valid saved clock").clone();
        assert_eq!(read, saved, "the whole clock, hold included");
        let mut after = blank();
        restore_saved_clock(&mut after, Some(&read));
        assert_eq!(after.clear_until, before.clear_until);
        assert_eq!(after.tick_counter, before.tick_counter);
        assert_eq!(after.world_time, before.world_time);
        assert_eq!(after.rain_until, before.rain_until);
        assert_eq!(after.storm_until, before.storm_until);
        assert_eq!(after.weather_lock_applied, before.weather_lock_applied);
        assert_eq!(after.night_calendar.saved(), before.night_calendar.saved());
        assert_eq!(after.calendar_id, before.calendar_id);
        assert_eq!(after.day_remainder, before.day_remainder);
        assert_eq!(after.day_last_tick, before.day_last_tick);
    }

    /// A hosting server's calendar is the one kept; nothing else of the
    /// client's is replaced. A save with no clock leaves the reset alone.
    #[test]
    fn the_hosts_calendar_is_saved_and_a_save_without_a_clock_changes_nothing() {
        let client = saved_example();
        let host = (crate::item_actions::NightCalendar::from_saved(40, Some(false)), [9; 16]);
        let saved = clock_to_save(&client, Some(host));
        assert_eq!((saved.nights, saved.was_night, saved.calendar_id), (40, Some(false), [9; 16]));
        assert_eq!((saved.tick_counter, saved.world_time, saved.rain_until), (90_000, 15_000, 91_000));

        let mut reset = blank();
        restore_saved_clock(&mut reset, None);
        assert_eq!((reset.tick_counter, reset.world_time, reset.calendar_id), (0, 6_000, [0; 16]));
        assert_eq!(reset.night_calendar.tonight(), 0);
    }

    fn storm(until: u64) -> Weather {
        Weather { rain_until: until, storm_until: until, clear_until: 0 }
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
            WeatherRules { has_weather: true, roll: true, lock: None, seed: 42 },
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
            let rules = WeatherRules { has_weather: true, roll: true, lock, seed: 42 };
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
            let c_strike = strike_roll(c_weather, c_tick, true, 42);
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
            w = step_weather(w, t, WeatherRules { has_weather: false, roll: true, lock: None, seed: 42 }, &mut applied);
            assert_eq!(w, Weather::CLEAR);
            assert_eq!(strike_roll(w, t, false, 42), None);
        }
    }

    #[test]
    fn a_lock_pins_the_window_and_its_release_clears_it_at_once() {
        let mut applied = None;
        let rules = |lock| WeatherRules { has_weather: true, roll: true, lock, seed: 42 };
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

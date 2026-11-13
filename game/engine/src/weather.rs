//! Weather rolls as pure functions — the rain window and the lightning strike.
//!
//! Extracted from `game_loop` so the "which worlds get weather" decision is
//! stated once, in one testable place, instead of living inside a 20-line block
//! in the middle of the client tick. Both rolls are derived deterministically
//! from the tick counter and the world's seed (no RNG dependency), so every
//! world has its own weather timeline and every side that rolls (the server, a
//! lending host, a standalone client, a joiner's presentation-only bolts) agrees.
//!
//! **The Workshop never has weather.** It's an indoor authoring space on a void
//! platform: rain over it is nonsense, and a lightning strike is actively
//! destructive — the bolt ignites through `FireSystem` like any other fire, so a
//! storm could set light to parked, half-finished projects the player left
//! standing there. Same hard `is_workshop` guard the mob spawner
//! (`entity.rs`) and Satoshi (`satoshi.rs`) already use, and for the same
//! reason: it keys off the runtime flag, not a saved meta field, so existing
//! Workshop saves are fixed without a migration.

/// The current weather window, in `tick_counter` units. Saved with the world's
/// clock (`sim_clock::SavedClock`), so a storm carries on after a restart; a
/// world without a saved clock loads clear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weather {
    /// It is raining while `tick < rain_until`.
    pub rain_until: u64,
    /// The rain is a thunderstorm while `tick < storm_until` (always a subset of
    /// the rain window).
    pub storm_until: u64,
    /// `/weather clear` holds the sky clear until this tick: [`advance`] rolls
    /// no new rain while `tick < clear_until`. Ending the rain alone does not do
    /// that (`advance` rolls again the next minute), so the hold is its own
    /// field. Saved with the clock (`WorldSave.weather_clear_until`) but NOT on
    /// the wire: a joiner never rolls, so its presentation-only copy
    /// ([`Weather::from_ticks_left`]) has none. `0` = no hold.
    pub clear_until: u64,
}

impl Weather {
    /// A clear sky.
    pub const CLEAR: Weather = Weather { rain_until: 0, storm_until: 0, clear_until: 0 };

    /// Ticks left of the `/weather clear` hold, as of `tick` (saturating). Not on
    /// the wire; used where a window is translated into another tick frame
    /// (a `--no-lend` host's owning server).
    pub fn clear_ticks_left(self, tick: u64) -> u32 {
        self.clear_until.saturating_sub(tick).min(u64::from(u32::MAX)) as u32
    }

    /// Ticks remaining in the rain and storm windows, as of `tick`. This is
    /// how the server puts its weather on the wire (`StateUpdatePacket`):
    /// a *duration*, not the absolute `rain_until`/`storm_until` tick, so the
    /// sync is correct even when the server's and a client's `tick_counter`s
    /// have drifted apart (different join times, wrapping, etc). Zero when
    /// clear or once the window has already elapsed (saturating).
    pub fn ticks_left(self, tick: u64) -> (u32, u32) {
        let rain = self.rain_until.saturating_sub(tick).min(u64::from(u32::MAX)) as u32;
        let storm = self.storm_until.saturating_sub(tick).min(u64::from(u32::MAX)) as u32;
        (rain, storm)
    }

    /// Inverse of [`Weather::ticks_left`] — reconstruct a `Weather` window
    /// (in the *receiver's* absolute-tick space) from a
    /// `(rain_ticks_left, storm_ticks_left)` pair read off a `StateUpdatePacket`,
    /// relative to the receiver's own current `tick`.
    ///
    /// The result has NO `/weather clear` hold (`clear_until` 0): the wire
    /// carries only rain and storm, because the authority decides the weather
    /// and a joiner rolls none of its own. A caller that must keep the hold
    /// (a `--no-lend` host's translated window) adds it back with
    /// [`Weather::with_clear_ticks_left`].
    pub fn from_ticks_left(tick: u64, rain_ticks_left: u32, storm_ticks_left: u32) -> Weather {
        Weather {
            rain_until: tick + u64::from(rain_ticks_left),
            storm_until: tick + u64::from(storm_ticks_left),
            clear_until: 0,
        }
    }

    /// This window with a `/weather clear` hold of `ticks_left` ticks from `tick`.
    pub fn with_clear_ticks_left(self, tick: u64, ticks_left: u32) -> Weather {
        Weather { clear_until: tick + u64::from(ticks_left), ..self }
    }
}

/// Which worlds run weather at all. The Workshop is the one exclusion.
pub const fn world_has_weather(is_workshop: bool) -> bool {
    !is_workshop
}

/// The values a scenario's `weather_lock` may name. Authoring guardrail: the
/// bundled-trial lint checks every def's `weather_lock` against this list, so a
/// typo ("stormy") fails a test instead of silently rolling normal weather.
/// (Test-surface only — the runtime path just asks [`locked_window`], which is
/// deliberately tolerant of a value it doesn't know.)
#[cfg_attr(not(test), allow(dead_code))]
pub const WEATHER_LOCKS: &[&str] = &["clear", "rain", "storm"];

/// How far ahead of `tick` a pinned window is set. Re-applied every tick for as
/// long as the trial runs, so it can never elapse mid-run — the figure only has
/// to outlast a single tick, and a minute of headroom keeps any
/// "ticks remaining" read-out (the wire's `ticks_left`) sane.
const LOCK_HORIZON: u64 = 1200;

/// The weather window a scenario's `weather_lock` pins at `tick`, or `None` for
/// "leave the world's own roll alone" (no lock, or a value this build doesn't
/// recognise — a forward-published def must never brick a world's weather).
///
/// Wind, Copper & Electricity wave §4. Pure, so the mapping is unit-testable
/// without a game loop; the loop applies the result at the ONE place the live
/// window is written, which is before the tick samples the wind.
pub fn locked_window(lock: Option<&str>, tick: u64) -> Option<Weather> {
    let ahead = tick + LOCK_HORIZON;
    match lock? {
        "clear" => Some(Weather::CLEAR),
        "rain" => Some(Weather { rain_until: ahead, storm_until: 0, clear_until: 0 }),
        "storm" => Some(Weather { rain_until: ahead, storm_until: ahead, clear_until: 0 }),
        _ => None,
    }
}

/// Fold a scenario's `weather_lock` into the live window for this tick.
///
/// `current` is the window [`advance`] just produced; `lock` is the lock in
/// force now (`None` once the trial ends); `lock_changed` says whether that
/// differs from the lock the caller applied on the PREVIOUS tick.
///
/// A pinned window is re-stamped [`LOCK_HORIZON`] ticks ahead every tick, so it
/// cannot elapse mid-run — which also means that when the lock is *released* the
/// last stamp is still a full in-game minute in the future. Left alone, a
/// storm-locked trial ended with the player standing in someone else's rain for
/// up to 60 s. So the moment the lock goes away — or names a different sky — the
/// residue is dropped back to [`Weather::CLEAR`] and `advance` is in charge
/// again from the next tick.
pub fn apply_lock(current: Weather, lock: Option<&str>, lock_changed: bool, tick: u64) -> Weather {
    if let Some(pinned) = locked_window(lock, tick) {
        return pinned;
    }
    if lock_changed {
        // We were pinning a window and are not any more: clear the residue
        // rather than letting the horizon run itself down.
        return Weather::CLEAR;
    }
    current
}

// ── The `/weather` command ──────────────────────────────────────────────

/// The sky a `/weather` command asks for. `thunder` (the Minecraft word) is
/// [`Sky::Storm`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sky {
    Clear,
    Rain,
    Storm,
}

/// A parsed `/weather <sky> [seconds]`: what the command hands the game loop,
/// which applies it through the authority (`GameState::apply_weather_request`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeatherRequest {
    pub sky: Sky,
    /// How long it lasts, 1 to [`MAX_COMMAND_SECONDS`] (already capped).
    pub seconds: u32,
    /// The number the player typed, when it was above the cap.
    pub capped_from: Option<u64>,
}

/// Game ticks per real second.
const TICKS_PER_SECOND: u64 = 20;

/// The longest `/weather` duration, in seconds. More is capped, as `/give` caps a count.
pub const MAX_COMMAND_SECONDS: u64 = 3600;

impl Sky {
    /// The duration used when the command gives none: a clear sky lasts 20
    /// minutes, rain and storms 5.
    pub const fn default_seconds(self) -> u32 {
        match self {
            Sky::Clear => 1200,
            Sky::Rain | Sky::Storm => 300,
        }
    }
}

/// The window `/weather` sets at `now`, replacing the whole current window:
/// - `Clear`: no rain, no storm, and a hold that stops `advance` rolling new rain;
/// - `Rain`: rain for `seconds`, and NO storm (so `/weather rain` during a storm
///   ends the thunder, and the storm window stays a subset of the rain window);
/// - `Storm`: rain and storm for `seconds`.
///
/// Rain and storm clear the hold. After the window ends the natural cycle resumes.
pub fn command_window(sky: Sky, seconds: u32, now: u64) -> Weather {
    let until = now + u64::from(seconds) * TICKS_PER_SECOND;
    match sky {
        Sky::Clear => Weather { rain_until: 0, storm_until: 0, clear_until: until },
        Sky::Rain => Weather { rain_until: until, storm_until: 0, clear_until: 0 },
        Sky::Storm => Weather { rain_until: until, storm_until: until, clear_until: 0 },
    }
}

/// Why `/weather` must change nothing here, if it must: the Workshop has no
/// weather, and a trial's `weather_lock` is re-pinned every tick, so a command
/// would be undone the next tick. A lock this build does not recognise pins
/// nothing ([`locked_window`]), so it does not refuse.
pub fn command_refusal(has_weather: bool, lock: Option<&str>) -> Option<&'static str> {
    if !has_weather {
        return Some("The Workshop has no weather");
    }
    locked_window(lock, 0).map(|_| "This trial sets the weather")
}

/// The window a `/weather` request makes at `now` in a world, or why not.
pub fn plan_command(
    request: WeatherRequest,
    now: u64,
    has_weather: bool,
    lock: Option<&str>,
) -> Result<Weather, &'static str> {
    match command_refusal(has_weather, lock) {
        Some(why) => Err(why),
        None => Ok(command_window(request.sky, request.seconds, now)),
    }
}

/// The sentence `/weather` answers with.
pub fn command_reply(request: WeatherRequest) -> String {
    let secs = request.seconds;
    let how_long = if secs.is_multiple_of(60) {
        let m = secs / 60;
        format!("{m} minute{}", if m == 1 { "" } else { "s" })
    } else if secs < 60 {
        format!("{secs} second{}", if secs == 1 { "" } else { "s" })
    } else {
        let (m, r) = (secs / 60, secs % 60);
        format!(
            "{m} minute{} {r} second{}",
            if m == 1 { "" } else { "s" },
            if r == 1 { "" } else { "s" }
        )
    };
    let what = match request.sky {
        Sky::Clear => "Clear skies",
        Sky::Rain => "Rain",
        Sky::Storm => "A thunderstorm",
    };
    match request.capped_from {
        Some(n) => format!("{what} for {how_long} (capped from {n} seconds)"),
        None => format!("{what} for {how_long}"),
    }
}

/// The roll's hash: splitmix64's finaliser over the tick and the world seed.
/// `salt` keeps the rain roll and the strike roll independent of each other.
/// (Until the v0.2.32 review the roll hashed the tick alone, so every world had
/// the same timeline, and a Weyl-style multiply clustered the storms.)
fn roll_hash(tick: u64, seed: u32, salt: u64) -> u64 {
    let mut z = (tick ^ (u64::from(seed) << 32) ^ salt).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Advance the weather window for `tick`, in the world with `seed`.
///
/// While the sky is clear, once an in-game minute (1200 ticks) there is a ~22%
/// chance of rain lasting 1–2.5 minutes, and ~1 rain in 7 upgrades to a
/// thunderstorm for the same window (1 in 3 until the v0.2.31 playtest:
/// "lightning happens too often").
///
/// When `has_weather` is false the result is [`Weather::CLEAR`] — which also
/// cancels a window carried in from the world you just left, so walking from a
/// downpour into the Workshop stops the rain at the door.
pub fn advance(current: Weather, tick: u64, has_weather: bool, seed: u32) -> Weather {
    if !has_weather {
        return Weather::CLEAR;
    }
    let mut out = current;
    // `/weather clear` holds the sky: no new rain is rolled until the hold ends.
    if tick >= current.rain_until
        && tick >= current.clear_until
        && tick > 0
        && tick.is_multiple_of(1200)
    {
        let h = roll_hash(tick, seed, 0x5241_494E) >> 33;
        if h % 100 < 22 {
            let dur = 1200 + (h % 4) * 600;
            out.rain_until = tick + dur;
            // Of the rain rolls (h % 100 < 22), the ones at 0, 10 and 20.
            if h.is_multiple_of(10) {
                out.storm_until = out.rain_until;
            }
        }
    }
    out
}

/// Roll for a lightning strike on `tick`, returning the strike hash (the caller
/// derives the ground position from it) or `None` for no bolt.
///
/// Strikes are rolled every 8 s while storming, and land ~55% of those rolls,
/// so a storm's bolts come bunched together and then the sky clears. Storms
/// themselves are rare (see [`advance`]); the playtest ask was "rarer, and
/// bunched up in a storm", not fewer bolts per storm.
/// Ticks between strike rolls while storming (8 s at 20 TPS).
pub const STRIKE_EVERY_TICKS: u64 = 160;

pub fn strike_roll(current: Weather, tick: u64, has_weather: bool, seed: u32) -> Option<u64> {
    if !has_weather {
        return None;
    }
    if tick == 0 || tick >= current.storm_until || !tick.is_multiple_of(STRIKE_EVERY_TICKS) {
        return None;
    }
    let h = roll_hash(tick, seed, 0x424F_4C54) >> 31;
    (h % 100 < 55).then_some(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world seed for the tests that do not care which.
    const SEED: u32 = 42;

    /// Run the window forward over `ticks` and report whether it ever rained.
    fn ever_rains(has_weather: bool, ticks: u64) -> bool {
        let mut w = Weather::CLEAR;
        for t in 1..=ticks {
            w = advance(w, t, has_weather, SEED);
            if t < w.rain_until {
                return true;
            }
        }
        false
    }

    /// Run the window forward and report whether lightning ever struck.
    fn ever_strikes(has_weather: bool, ticks: u64) -> bool {
        let mut w = Weather::CLEAR;
        for t in 1..=ticks {
            w = advance(w, t, has_weather, SEED);
            if strike_roll(w, t, has_weather, SEED).is_some() {
                return true;
            }
        }
        false
    }

    /// Control: a normal world DOES get weather over a long enough run. Without
    /// this the Workshop assertions below could pass for the wrong reason.
    #[test]
    fn a_normal_world_gets_rain() {
        assert!(ever_rains(true, 200_000), "a normal world must still get rain");
    }

    /// Control: a normal world DOES get lightning.
    #[test]
    fn a_normal_world_gets_lightning() {
        assert!(ever_strikes(true, 200_000), "a normal world must still get storms");
    }

    /// THE guard — Axolittle 2026-07-31. No rain in the Workshop, ever.
    #[test]
    fn the_workshop_never_rains() {
        assert!(!ever_rains(false, 200_000), "the Workshop must stay clear");
    }

    /// THE guard — no lightning in the Workshop. A bolt ignites through
    /// `FireSystem`, so a strike could burn parked projects.
    #[test]
    fn the_workshop_is_never_struck_by_lightning() {
        assert!(!ever_strikes(false, 200_000), "no bolts over the Workshop");
    }

    /// Walking out of a downpour into the Workshop stops the rain at the door —
    /// the window carried in from the previous world is cancelled, not waited out.
    #[test]
    fn entering_the_workshop_cancels_weather_carried_in() {
        let storming = Weather { rain_until: 9_000, storm_until: 9_000, clear_until: 0 };
        assert_eq!(
            advance(storming, 1_000, false, SEED),
            Weather::CLEAR,
            "a rain window from the world you just left must not follow you in"
        );
    }

    /// Even mid-storm, the Workshop rolls no strike.
    #[test]
    fn a_carried_in_storm_still_cannot_strike_the_workshop() {
        let storming = Weather { rain_until: 9_000, storm_until: 9_000, clear_until: 0 };
        for t in (160..9_000).step_by(160) {
            assert!(
                strike_roll(storming, t, false, SEED).is_none(),
                "tick {t} rolled a bolt over the Workshop"
            );
        }
    }

    /// Rain without a storm never produces lightning — the storm tier is what
    /// unlocks strikes, and plain rain must stay quiet.
    #[test]
    fn plain_rain_produces_no_lightning() {
        let raining = Weather { rain_until: 9_000, storm_until: 0, clear_until: 0 };
        for t in (160..9_000).step_by(160) {
            assert!(strike_roll(raining, t, true, SEED).is_none(), "plain rain must not strike");
        }
    }

    /// The storm window is always inside the rain window — a storm cannot
    /// outlast the rain that carries it.
    #[test]
    fn a_storm_never_outlasts_its_rain() {
        let mut w = Weather::CLEAR;
        for t in 1..200_000 {
            w = advance(w, t, true, SEED);
            assert!(w.storm_until <= w.rain_until, "storm outran its rain at tick {t}");
        }
    }

    // ── ticks_left / from_ticks_left (P9 weather sync wire encoding) ──────

    /// A clear sky reports zero ticks left, whatever the current tick.
    #[test]
    fn ticks_left_is_zero_when_clear() {
        assert_eq!(Weather::CLEAR.ticks_left(0), (0, 0));
        assert_eq!(Weather::CLEAR.ticks_left(50_000), (0, 0));
    }

    /// A window that has already elapsed reports zero, not an underflow wrap —
    /// `tick` can run past `rain_until`/`storm_until` for a tick or two before
    /// the next `advance()` call clears them.
    #[test]
    fn ticks_left_saturates_past_the_window() {
        let w = Weather { rain_until: 50, storm_until: 40, clear_until: 0 };
        assert_eq!(w.ticks_left(100), (0, 0), "an elapsed window must not underflow");
    }

    /// `ticks_left` and `from_ticks_left` are exact inverses at the same tick —
    /// this is the property the wire encoding relies on: whatever the server
    /// measured relative to its own tick, the receiver reconstructs the same
    /// absolute window relative to ITS tick.
    #[test]
    fn ticks_left_round_trips_through_from_ticks_left() {
        let w = Weather { rain_until: 150, storm_until: 130, clear_until: 0 };
        let (rain, storm) = w.ticks_left(100);
        assert_eq!((rain, storm), (50, 30));
        assert_eq!(Weather::from_ticks_left(100, rain, storm), w);
    }

    /// Plain rain (no storm, `storm_until` already behind `tick`) reports a
    /// zero storm ticks-left, and reconstructing from that zero yields a
    /// `Weather` that is — like the original — NOT storming at `tick` (even
    /// though the absolute `storm_until` differs: 0 vs `tick` are both "no
    /// storm now"). Only the rain side round-trips exactly here, since
    /// `storm_until` (0) is behind `tick` (8000) to begin with.
    #[test]
    fn ticks_left_reports_plain_rain_with_no_storm() {
        let w = Weather { rain_until: 9_000, storm_until: 0, clear_until: 0 };
        let (rain, storm) = w.ticks_left(8_000);
        assert_eq!((rain, storm), (1_000, 0));
        let back = Weather::from_ticks_left(8_000, rain, storm);
        assert_eq!(back.rain_until, w.rain_until, "the active rain window round-trips exactly");
        assert!(8_000 >= back.storm_until, "reconstructed window must not be storming either");
    }

    // ── weather_lock (Wind, Copper & Electricity wave §4) ────────────────────

    #[test]
    fn no_lock_and_an_unknown_lock_both_leave_the_world_alone() {
        assert_eq!(locked_window(None, 500), None);
        assert_eq!(locked_window(Some("stormy"), 500), None, "a typo must not pin anything");
        assert_eq!(locked_window(Some(""), 500), None);
    }

    #[test]
    fn each_lock_names_the_window_it_says_it_does() {
        let tick = 7_000;
        let clear = locked_window(Some("clear"), tick).expect("clear is a lock");
        assert_eq!(clear, Weather::CLEAR);

        let rain = locked_window(Some("rain"), tick).expect("rain is a lock");
        assert!(tick < rain.rain_until, "a rain lock is raining right now");
        assert!(tick >= rain.storm_until, "…but it is not a thunderstorm");

        let storm = locked_window(Some("storm"), tick).expect("storm is a lock");
        assert!(tick < storm.storm_until, "a storm lock is storming right now");
        assert!(
            storm.storm_until <= storm.rain_until,
            "a storm is always inside the rain window (Weather's invariant)"
        );
    }

    #[test]
    fn a_lock_re_applied_every_tick_never_elapses() {
        // The loop re-applies the window each tick, so the horizon only has to
        // outlast one tick — but prove it never lapses over a long run either.
        for t in (0..40_000).step_by(97) {
            let w = locked_window(Some("storm"), t).expect("storm is a lock");
            assert!(t < w.storm_until, "the pinned storm must still be live at tick {t}");
        }
    }

    #[test]
    fn every_advertised_lock_value_actually_pins_a_window() {
        for name in WEATHER_LOCKS {
            assert!(
                locked_window(Some(name), 1_234).is_some(),
                "WEATHER_LOCKS advertises {name:?} but locked_window does not honour it"
            );
        }
    }

    #[test]
    fn releasing_a_lock_clears_the_pinned_window_at_once() {
        // Whole-branch review, minor. The lock is re-stamped LOCK_HORIZON ahead
        // every tick, so the LAST stamp before a trial ends is a full in-game
        // minute in the future. Pre-fix nothing dropped it: the trial finished
        // and the player stood in the trial's storm for up to 60 s afterwards.
        let tick = 5_000;
        let pinned = apply_lock(Weather::CLEAR, Some("storm"), true, tick);
        assert!(tick < pinned.storm_until, "the lock pins a live storm");

        // Same lock next tick: still pinned, re-stamped further ahead.
        let held = apply_lock(pinned, Some("storm"), false, tick + 1);
        assert!(tick + 1 < held.storm_until);

        // Trial over — the residue goes NOW, not in a minute's time.
        let released = apply_lock(held, None, true, tick + 2);
        assert_eq!(
            released,
            Weather::CLEAR,
            "a released lock must not leave its window running down"
        );
    }

    #[test]
    fn changing_the_lock_value_replaces_the_window_rather_than_layering() {
        // "storm" → "clear" is a lock CHANGE, not a release: the new value wins
        // immediately (and "clear" is itself a pin, so it holds the sky clear
        // for as long as that trial runs).
        let tick = 9_000;
        let storm = apply_lock(Weather::CLEAR, Some("storm"), true, tick);
        assert!(tick < storm.rain_until);
        let clear = apply_lock(storm, Some("clear"), true, tick + 1);
        assert_eq!(clear, Weather::CLEAR);
    }

    #[test]
    fn an_unlocked_world_keeps_its_own_rolled_window() {
        // The no-lock path must not touch the world's own weather. `advance`
        // owns that window; `apply_lock` only ever steps in for a trial.
        let own = Weather { rain_until: 12_000, storm_until: 11_000, clear_until: 0 };
        assert_eq!(apply_lock(own, None, false, 10_000), own);
    }

    #[test]
    fn an_unrecognised_lock_still_releases_a_previous_one() {
        // A forward-published def naming a sky this build doesn't know pins
        // nothing (`locked_window` returns None) — but it is still a CHANGE, so
        // the window the previous lock left behind is dropped rather than
        // lingering under a lock that isn't doing anything.
        let tick = 700;
        let storm = apply_lock(Weather::CLEAR, Some("storm"), true, tick);
        assert!(tick < storm.rain_until);
        assert_eq!(apply_lock(storm, Some("blizzard"), true, tick + 1), Weather::CLEAR);
    }

    // ── The world seed (v0.2.32 review) ──────────────────────────────────

    /// Ticks at which a rain / a storm opens, in `ticks`, in the world with `seed`.
    fn openings(seed: u32, ticks: u64) -> (Vec<u64>, Vec<u64>) {
        let (mut rains, mut storms) = (Vec::new(), Vec::new());
        let mut w = Weather::CLEAR;
        for t in (1200..=ticks).step_by(1200) {
            let before = w;
            w = advance(w, t, true, seed);
            if w.rain_until != before.rain_until {
                rains.push(t);
                if w.storm_until == w.rain_until {
                    storms.push(t);
                }
            }
        }
        (rains, storms)
    }

    #[test]
    fn two_worlds_have_different_weather_and_different_bolts() {
        let (r1, s1) = openings(1, 2_000_000);
        let (r2, s2) = openings(2, 2_000_000);
        assert_ne!(r1, r2, "two seeds, one rain timeline");
        assert_ne!(s1, s2, "two seeds, one storm timeline");
        // Every one of 16 seeds has its own timeline.
        let mut all: Vec<Vec<u64>> = (0..16).map(|s| openings(s, 600_000).1).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 16, "no two of 16 worlds share a storm timeline");

        // And the bolts of a storm differ by world too.
        let storm = Weather { rain_until: u64::MAX, storm_until: u64::MAX, clear_until: 0 };
        let bolts = |seed| -> Vec<Option<u64>> { (1..400).map(|k| strike_roll(storm, k * STRIKE_EVERY_TICKS, true, seed)).collect() };
        assert_ne!(bolts(1), bolts(2));
        assert_eq!(bolts(7), bolts(7), "but one world always rolls the same");
    }

    #[test]
    fn storms_stay_about_one_rain_in_seven_over_a_long_run() {
        let (mut rains, mut storms) = (0usize, 0usize);
        for seed in 0..8 {
            let (r, s) = openings(seed, 10_000_000);
            rains += r.len();
            storms += s.len();
        }
        assert!(rains > 1000, "{rains} rains is too few to judge");
        let share = storms as f64 / rains as f64;
        assert!((0.11..0.17).contains(&share), "{storms} storms in {rains} rains = {share:.3}, want about 1/7");
    }

    #[test]
    fn a_new_worlds_first_forty_minutes_do_not_always_hold_three_storms() {
        const FORTY_MINUTES: u64 = 40 * 1200;
        let counts: Vec<usize> = (0..200).map(|seed| openings(seed, FORTY_MINUTES).1.len()).collect();
        let three = counts.iter().filter(|&&n| n == 3).count();
        assert!(three < counts.len() / 4, "{three} of {} new worlds open with exactly three storms", counts.len());
        let mut kinds = counts.clone();
        kinds.sort_unstable();
        kinds.dedup();
        assert!(kinds.len() >= 3, "storm counts in 40 minutes should vary by world: {kinds:?}");
        let mean = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
        assert!(mean < 2.0, "a storm every 20 minutes on average is too many: {mean}");
    }

    // ── `/weather` (2026-11-13) ──────────────────────────────────────────

    /// The first minute from `from` on where `advance` rolls rain out of a clear sky.
    fn first_roll_from(seed: u32, from: u64) -> u64 {
        (from.div_ceil(1200)..)
            .map(|k| k * 1200)
            .find(|&t| t > 0 && advance(Weather::CLEAR, t, true, seed).rain_until > t)
            .unwrap()
    }

    #[test]
    fn a_clear_hold_stops_advance_rolling_rain_and_the_cycle_resumes_after_it() {
        let t = first_roll_from(SEED, 1200);
        // Control: the same tick, no hold, rolls rain.
        assert!(advance(Weather::CLEAR, t, true, SEED).rain_until > t);
        // `/weather clear 600` set the tick before: 12,000 ticks of hold.
        let held = command_window(Sky::Clear, 600, t - 1);
        assert_eq!(held.clear_until, t - 1 + 12_000, "600 s is 12,000 ticks");
        let mut w = held;
        let mut rolls_inside = 0;
        for tick in t..held.clear_until {
            w = advance(w, tick, true, SEED);
            assert!(tick >= w.rain_until, "tick {tick}: no rain inside the hold");
            if tick % 1200 == 0 && advance(Weather::CLEAR, tick, true, SEED).rain_until > tick {
                rolls_inside += 1;
            }
        }
        assert!(rolls_inside >= 1, "the hold crossed at least one tick that would have rolled rain");
        assert_eq!(w.clear_until, held.clear_until, "advance keeps the hold");
        // Then the natural cycle resumes: a later would-roll minute rolls rain again.
        let later = first_roll_from(SEED, held.clear_until);
        let w = advance(w, later, true, SEED);
        assert!(w.rain_until > later, "the cycle resumes after the hold");
    }

    #[test]
    fn a_hold_ends_exactly_at_clear_until() {
        let t = first_roll_from(SEED, 1200);
        let w = Weather { clear_until: t, ..Weather::CLEAR };
        assert!(advance(w, t, true, SEED).rain_until > t, "at `clear_until` the roll is live again");
        let w = Weather { clear_until: t + 1, ..Weather::CLEAR };
        assert_eq!(advance(w, t, true, SEED).rain_until, 0, "one tick short, it is held");
    }

    #[test]
    fn rain_and_storm_set_their_windows_and_clear_their_hold() {
        let now = 5_000;
        let rain = command_window(Sky::Rain, 60, now);
        assert_eq!(rain, Weather { rain_until: now + 1_200, storm_until: 0, clear_until: 0 });
        let storm = command_window(Sky::Storm, 300, now);
        assert_eq!(storm, Weather { rain_until: now + 6_000, storm_until: now + 6_000, clear_until: 0 });
        let clear = command_window(Sky::Clear, 1200, now);
        assert_eq!(clear, Weather { rain_until: 0, storm_until: 0, clear_until: now + 24_000 });
        // The storm window is a subset of the rain window.
        for w in [rain, storm, clear] {
            assert!(w.storm_until <= w.rain_until);
        }
    }

    #[test]
    fn rain_during_a_storm_with_more_storm_left_zeroes_the_storm() {
        let now = 9_000;
        let storm = command_window(Sky::Storm, 3600, now);
        assert!(storm.storm_until > now + 50_000);
        let rain = command_window(Sky::Rain, 60, now);
        assert_eq!(rain.storm_until, 0, "storm_until is 0 afterwards");
        assert!((now..now + 60_000).all(|t| strike_roll(rain, t, true, SEED).is_none()), "no thunder from the old window");
    }

    #[test]
    fn the_hold_is_not_on_the_wire_and_the_translated_window_keeps_it() {
        let w = Weather { rain_until: 150, storm_until: 130, clear_until: 5_000 };
        // The wire carries rain and storm only.
        let (rain, storm) = w.ticks_left(100);
        let joiner = Weather::from_ticks_left(100, rain, storm);
        assert_eq!(joiner, Weather { rain_until: 150, storm_until: 130, clear_until: 0 }, "a joiner's copy has no hold");
        // A `--no-lend` host's translation adds it back.
        let kept = Weather::from_ticks_left(7, rain, storm).with_clear_ticks_left(7, w.clear_ticks_left(100));
        assert_eq!(kept.clear_until, 7 + 4_900);
        assert_eq!(Weather::CLEAR.clear_ticks_left(10), 0);
        assert_eq!(w.clear_ticks_left(9_999), 0, "saturating once the hold is over");
    }

    #[test]
    fn a_trial_lock_pins_its_own_window_and_a_command_is_refused_under_it() {
        for lock in ["clear", "rain", "storm"] {
            assert_eq!(command_refusal(true, Some(lock)), Some("This trial sets the weather"), "{lock}");
        }
        assert_eq!(command_refusal(true, None), None);
        // A lock this build does not know pins nothing, so it does not refuse.
        assert_eq!(command_refusal(true, Some("blizzard")), None);
        // The Workshop wins, with its own words.
        assert_eq!(command_refusal(false, None), Some("The Workshop has no weather"));
        assert_eq!(command_refusal(false, Some("storm")), Some("The Workshop has no weather"));
        // The locked windows are never holds.
        for lock in ["clear", "rain", "storm"] {
            assert_eq!(locked_window(Some(lock), 10).unwrap().clear_until, 0);
        }
        let req = WeatherRequest { sky: Sky::Rain, seconds: 60, capped_from: None };
        assert_eq!(plan_command(req, 100, true, Some("storm")), Err("This trial sets the weather"));
        assert_eq!(plan_command(req, 100, false, None), Err("The Workshop has no weather"));
        assert_eq!(plan_command(req, 100, true, None), Ok(command_window(Sky::Rain, 60, 100)));
    }

    #[test]
    fn the_reply_is_a_plain_sentence() {
        let r = |sky, seconds, capped_from| command_reply(WeatherRequest { sky, seconds, capped_from });
        assert_eq!(r(Sky::Clear, 1200, None), "Clear skies for 20 minutes");
        assert_eq!(r(Sky::Rain, 300, None), "Rain for 5 minutes");
        assert_eq!(r(Sky::Storm, 60, None), "A thunderstorm for 1 minute");
        assert_eq!(r(Sky::Rain, 45, None), "Rain for 45 seconds");
        assert_eq!(r(Sky::Rain, 1, None), "Rain for 1 second");
        assert_eq!(r(Sky::Rain, 90, None), "Rain for 1 minute 30 seconds");
        assert_eq!(r(Sky::Clear, 3600, Some(9_000)), "Clear skies for 60 minutes (capped from 9000 seconds)");
    }
}

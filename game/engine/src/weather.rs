//! Weather rolls as pure functions — the rain window and the lightning strike.
//!
//! Extracted from `game_loop` so the "which worlds get weather" decision is
//! stated once, in one testable place, instead of living inside a 20-line block
//! in the middle of the client tick. Both rolls are derived deterministically
//! from the tick counter (no RNG dependency), exactly as before.
//!
//! **The Workshop never has weather.** It's an indoor authoring space on a void
//! platform: rain over it is nonsense, and a lightning strike is actively
//! destructive — the bolt ignites through `FireSystem` like any other fire, so a
//! storm could set light to parked, half-finished projects the player left
//! standing there. Same hard `is_workshop` guard the mob spawner
//! (`entity.rs`) and Satoshi (`satoshi.rs`) already use, and for the same
//! reason: it keys off the runtime flag, not a saved meta field, so existing
//! Workshop saves are fixed without a migration.

/// The current weather window, in `tick_counter` units. Both fields are
/// ephemeral — deliberately not saved, so a world always loads clear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weather {
    /// It is raining while `tick < rain_until`.
    pub rain_until: u64,
    /// The rain is a thunderstorm while `tick < storm_until` (always a subset of
    /// the rain window).
    pub storm_until: u64,
}

impl Weather {
    /// A clear sky.
    pub const CLEAR: Weather = Weather { rain_until: 0, storm_until: 0 };

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
    pub fn from_ticks_left(tick: u64, rain_ticks_left: u32, storm_ticks_left: u32) -> Weather {
        Weather {
            rain_until: tick + u64::from(rain_ticks_left),
            storm_until: tick + u64::from(storm_ticks_left),
        }
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
        "rain" => Some(Weather { rain_until: ahead, storm_until: 0 }),
        "storm" => Some(Weather { rain_until: ahead, storm_until: ahead }),
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

/// Advance the weather window for `tick`.
///
/// While the sky is clear, once an in-game minute (1200 ticks) there is a ~22%
/// chance of rain lasting 1–2.5 minutes, and ~1 rain in 3 upgrades to a
/// thunderstorm for the same window.
///
/// When `has_weather` is false the result is [`Weather::CLEAR`] — which also
/// cancels a window carried in from the world you just left, so walking from a
/// downpour into the Workshop stops the rain at the door.
pub fn advance(current: Weather, tick: u64, has_weather: bool) -> Weather {
    if !has_weather {
        return Weather::CLEAR;
    }
    let mut out = current;
    if tick >= current.rain_until && tick > 0 && tick.is_multiple_of(1200) {
        let h = tick.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 33;
        if h % 100 < 22 {
            let dur = 1200 + (h % 4) * 600;
            out.rain_until = tick + dur;
            if h.is_multiple_of(3) {
                out.storm_until = out.rain_until;
            }
        }
    }
    out
}

/// Roll for a lightning strike on `tick`, returning the strike hash (the caller
/// derives the ground position from it) or `None` for no bolt.
///
/// Strikes are rolled every 8 s while storming, and land ~55% of those rolls.
pub fn strike_roll(current: Weather, tick: u64, has_weather: bool) -> Option<u64> {
    if !has_weather {
        return None;
    }
    if tick == 0 || tick >= current.storm_until || !tick.is_multiple_of(160) {
        return None;
    }
    let h = tick.wrapping_mul(0xC4CE_B9FE_1A85_EC53) >> 31;
    (h % 100 < 55).then_some(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the window forward over `ticks` and report whether it ever rained.
    fn ever_rains(has_weather: bool, ticks: u64) -> bool {
        let mut w = Weather::CLEAR;
        for t in 1..=ticks {
            w = advance(w, t, has_weather);
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
            w = advance(w, t, has_weather);
            if strike_roll(w, t, has_weather).is_some() {
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
        let storming = Weather { rain_until: 9_000, storm_until: 9_000 };
        assert_eq!(
            advance(storming, 1_000, false),
            Weather::CLEAR,
            "a rain window from the world you just left must not follow you in"
        );
    }

    /// Even mid-storm, the Workshop rolls no strike.
    #[test]
    fn a_carried_in_storm_still_cannot_strike_the_workshop() {
        let storming = Weather { rain_until: 9_000, storm_until: 9_000 };
        for t in (160..9_000).step_by(160) {
            assert!(
                strike_roll(storming, t, false).is_none(),
                "tick {t} rolled a bolt over the Workshop"
            );
        }
    }

    /// Rain without a storm never produces lightning — the storm tier is what
    /// unlocks strikes, and plain rain must stay quiet.
    #[test]
    fn plain_rain_produces_no_lightning() {
        let raining = Weather { rain_until: 9_000, storm_until: 0 };
        for t in (160..9_000).step_by(160) {
            assert!(strike_roll(raining, t, true).is_none(), "plain rain must not strike");
        }
    }

    /// The storm window is always inside the rain window — a storm cannot
    /// outlast the rain that carries it.
    #[test]
    fn a_storm_never_outlasts_its_rain() {
        let mut w = Weather::CLEAR;
        for t in 1..200_000 {
            w = advance(w, t, true);
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
        let w = Weather { rain_until: 50, storm_until: 40 };
        assert_eq!(w.ticks_left(100), (0, 0), "an elapsed window must not underflow");
    }

    /// `ticks_left` and `from_ticks_left` are exact inverses at the same tick —
    /// this is the property the wire encoding relies on: whatever the server
    /// measured relative to its own tick, the receiver reconstructs the same
    /// absolute window relative to ITS tick.
    #[test]
    fn ticks_left_round_trips_through_from_ticks_left() {
        let w = Weather { rain_until: 150, storm_until: 130 };
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
        let w = Weather { rain_until: 9_000, storm_until: 0 };
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
        let own = Weather { rain_until: 12_000, storm_until: 11_000 };
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
}

//! Wind — a derived, deterministic breeze (Wind/Copper/Electricity wave §2.1).
//!
//! The Windmill needs something to turn it, and the cheapest honest source of
//! "something" is state every side of the wire ALREADY agrees on: the tick
//! counter, the synced weather window (protocol v59) and the world seed. So the
//! wind is a pure function of those three plus altitude — never saved, never
//! synced, no protocol change, and a hosted world's mills behave the same on
//! the server as they look on a client.
//!
//! Shape of the breeze:
//! * **Base** — two octaves of smooth value noise over `tick` (periods ≈ 2400
//!   and 9000 ticks), so a gust builds and dies over 2–7 real minutes rather
//!   than flickering frame to frame. Mapped to `0.10..=0.60`.
//! * **Weather** — `+0.25` while it rains, `+0.50` in a thunderstorm (a storm
//!   is a subset of the rain window, so a storm reads `+0.50`, not `+0.75`).
//! * **Altitude** — `+0.010` per block above sea level, capped at `+0.30` — so
//!   the cap lands 30 blocks up, which is inside the world (the ceiling is
//!   `SEA_LEVEL + 33`). A mill on a hilltop is reliable; one at sea level is
//!   intermittent, which is the teaching point ("build it high"). The first
//!   tuning put the gain at `+0.004`, needing +75 blocks for the cap — higher
//!   than any legal build can stand, so "build it high" was not really a lever
//!   at all.
//!
//! Direction is slow seeded noise quantised to the eight compass points. It
//! drives the F3 read-out (and, later, sails/kites); the Windmill itself is
//! omnidirectional in this wave.

use crate::biome::SEA_LEVEL;
use crate::weather::Weather;

/// The wind at one place and time. `speed` is `0.0..=1.0` (a fraction of a
/// gale, not a real-world m/s); `direction` is `0..8`, 0 = north, clockwise.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindSample {
    pub speed: f32,
    pub direction: u8,
}

impl WindSample {
    /// Dead air. The `power_tick` default for tests and for any caller that
    /// has no weather/seed to hand — a Windmill never turns in it.
    pub const CALM: WindSample = WindSample { speed: 0.0, direction: 0 };
}

/// Gust octave periods, in ticks (20 TPS → 2 min and 7.5 min).
const FAST_PERIOD: u64 = 2400;
const SLOW_PERIOD: u64 = 9000;
/// How often the direction wanders round the compass (~5 min).
const DIR_PERIOD: u64 = 6000;

/// Base-breeze band before weather and altitude.
const BASE_MIN: f32 = 0.10;
const BASE_MAX: f32 = 0.60;

/// Octave weights. The fast gust dominates so a mill visibly starts and stops
/// within a play session; the slow octave gives calm and blustery afternoons.
const FAST_WEIGHT: f32 = 0.65;
const SLOW_WEIGHT: f32 = 0.35;

/// Speed added while it is raining, and while it is a thunderstorm. A storm is
/// inside the rain window, so these do NOT stack — the storm figure is total.
const RAIN_BONUS: f32 = 0.25;
const STORM_BONUS: f32 = 0.50;

/// Speed gained per block above [`SEA_LEVEL`], and the cap on that gain. The
/// two are chosen together: at `0.010` per block the cap is reached 30 blocks
/// up, just under the world ceiling at `SEA_LEVEL + 33`, so the full altitude
/// bonus is something a player can actually build to.
const ALTITUDE_PER_BLOCK: f32 = 0.010;
const ALTITUDE_CAP: f32 = 0.30;

/// 64-bit avalanche → a `0.0..1.0` float. Seeded and salted so the direction
/// noise never correlates with the speed noise.
fn hash01(i: u64, seed: u32, salt: u64) -> f32 {
    let mut h = i
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ u64::from(seed).wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ salt;
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 29;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 32;
    // Top 24 bits → [0,1). Plenty of resolution for a 0..1 wind speed.
    (h >> 40) as f32 / 16_777_216.0
}

/// Smoothstep — kills the corners a linear interpolation would leave at each
/// period boundary, so the gust curve has no visible kinks.
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// One octave of smooth value noise over `tick`, in `0.0..1.0`.
fn value_noise(tick: u64, period: u64, seed: u32, salt: u64) -> f32 {
    let i = tick / period;
    let t = (tick % period) as f32 / period as f32;
    let a = hash01(i, seed, salt);
    let b = hash01(i + 1, seed, salt);
    a + (b - a) * smooth(t)
}

/// The two-octave base breeze at `tick`, in `BASE_MIN..=BASE_MAX`.
fn base_breeze(tick: u64, seed: u32) -> f32 {
    let fast = value_noise(tick, FAST_PERIOD, seed, 0x57_1D_10);
    let slow = value_noise(tick, SLOW_PERIOD, seed, 0x9E_A7_53);
    let mix = FAST_WEIGHT * fast + SLOW_WEIGHT * slow;
    BASE_MIN + (BASE_MAX - BASE_MIN) * mix
}

/// The weather term at `tick`. Storm supersedes rain (never stacks).
fn weather_bonus(tick: u64, weather: Weather) -> f32 {
    if tick < weather.storm_until {
        STORM_BONUS
    } else if tick < weather.rain_until {
        RAIN_BONUS
    } else {
        0.0
    }
}

/// Extra speed for standing `y` blocks up. Zero at or below sea level.
pub fn altitude_bonus(y: i32) -> f32 {
    ((y - SEA_LEVEL).max(0) as f32 * ALTITUDE_PER_BLOCK).min(ALTITUDE_CAP)
}

/// The wind at `tick`, under `weather`, in world `seed`, at height `y`.
/// Deterministic and side-effect free — the same four inputs always give the
/// same sample, on any machine, on either side of the wire.
pub fn sample(tick: u64, weather: Weather, seed: u32, y: i32) -> WindSample {
    let speed = (base_breeze(tick, seed) + weather_bonus(tick, weather) + altitude_bonus(y))
        .clamp(0.0, 1.0);
    let d = value_noise(tick, DIR_PERIOD, seed, 0xD1_2E_C7);
    WindSample {
        speed,
        direction: ((d * 8.0) as u8).min(7),
    }
}

/// Re-apply the altitude term to a sample taken at [`SEA_LEVEL`].
///
/// The tick loop takes ONE sea-level sample per tick and every Windmill lifts
/// it to its own height with this — cheaper than re-running the noise per mill,
/// and exactly equal to `sample(.., y)` because `altitude_bonus(SEA_LEVEL)` is
/// zero. Passing a sample that already carries an altitude term would
/// double-count it, so only ever feed this a sea-level sample.
pub fn with_altitude(s: WindSample, y: i32) -> WindSample {
    WindSample {
        speed: (s.speed + altitude_bonus(y)).clamp(0.0, 1.0),
        direction: s.direction,
    }
}

/// The player-facing word for a speed (hover labels + the F3 read-out).
pub fn word(speed: f32) -> &'static str {
    if speed < 0.20 {
        "calm"
    } else if speed < 0.35 {
        "light"
    } else if speed < 0.60 {
        "fresh"
    } else if speed < 0.85 {
        "strong"
    } else {
        "gale"
    }
}

/// The eight-point compass label for a `direction` (0 = north, clockwise).
pub fn compass(direction: u8) -> &'static str {
    match direction % 8 {
        0 => "N",
        1 => "NE",
        2 => "E",
        3 => "SE",
        4 => "S",
        5 => "SW",
        6 => "W",
        _ => "NW",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Windmill's "turn on" threshold, restated here so the duty-cycle
    /// tests read on their own (the live const is `power::WINDMILL_START`).
    const START: f32 = 0.35;

    fn storm(until: u64) -> Weather {
        Weather { rain_until: until, storm_until: until }
    }
    fn rain(until: u64) -> Weather {
        Weather { rain_until: until, storm_until: 0 }
    }

    #[test]
    fn sample_is_deterministic() {
        for t in [0u64, 1, 999, 24_000, 1_234_567] {
            let a = sample(t, Weather::CLEAR, 7, 70);
            let b = sample(t, Weather::CLEAR, 7, 70);
            assert_eq!(a, b, "same inputs must give the same sample at tick {t}");
        }
        // A different seed gives a different world's weather somewhere in a
        // day's worth of ticks (not necessarily at every single tick).
        let differs = (0..24_000u64).step_by(97).any(|t| {
            sample(t, Weather::CLEAR, 1, SEA_LEVEL).speed
                != sample(t, Weather::CLEAR, 2, SEA_LEVEL).speed
        });
        assert!(differs, "the seed must actually change the breeze");
    }

    #[test]
    fn speed_stays_in_range_and_direction_in_the_compass() {
        for seed in [0u32, 42, 12_345] {
            for t in (0..60_000u64).step_by(37) {
                for (w, y) in [
                    (Weather::CLEAR, -40),
                    (rain(u64::MAX), SEA_LEVEL),
                    (storm(u64::MAX), 250),
                ] {
                    let s = sample(t, w, seed, y);
                    assert!(
                        (0.0..=1.0).contains(&s.speed),
                        "speed {} out of range (t={t} seed={seed})",
                        s.speed
                    );
                    assert!(s.direction < 8, "direction {} off the compass", s.direction);
                }
            }
        }
    }

    #[test]
    fn storm_is_windier_than_rain_is_windier_than_clear() {
        for t in (0..24_000u64).step_by(211) {
            let clear = sample(t, Weather::CLEAR, 9, SEA_LEVEL).speed;
            let wet = sample(t, rain(u64::MAX), 9, SEA_LEVEL).speed;
            let wild = sample(t, storm(u64::MAX), 9, SEA_LEVEL).speed;
            assert!(wet >= clear, "rain must be at least as windy as clear (t={t})");
            assert!(wild >= wet, "a storm must be at least as windy as rain (t={t})");
        }
    }

    #[test]
    fn wind_never_falls_off_with_height() {
        for t in (0..24_000u64).step_by(613) {
            let mut prev = 0.0f32;
            for y in (SEA_LEVEL - 20..=SEA_LEVEL + 120).step_by(5) {
                let s = sample(t, Weather::CLEAR, 3, y).speed;
                assert!(s >= prev - f32::EPSILON, "altitude must be monotone (t={t} y={y})");
                prev = s;
            }
        }
        // …and the cap is real, and REACHABLE: +30 blocks is worth the full
        // +0.30, and the world ceiling is SEA_LEVEL + 33.
        assert!((altitude_bonus(SEA_LEVEL + 30) - 0.30).abs() < 1e-6);
        assert!(
            altitude_bonus(SEA_LEVEL + 29) < 0.30,
            "one block short of the cap must still be short of it"
        );
        assert_eq!(altitude_bonus(SEA_LEVEL - 30), 0.0, "no penalty below sea level");
    }

    #[test]
    fn sea_level_clear_sky_is_intermittent() {
        // The design point: a sea-level mill on a clear day works *sometimes*.
        // Over five days the fraction of ticks above the turn-on threshold has
        // to sit well inside 0.25..0.75 for every seed we check.
        for seed in [0u32, 1, 42, 2026, 999_983] {
            let hits = (0..120_000u64)
                .filter(|&t| sample(t, Weather::CLEAR, seed, SEA_LEVEL).speed >= START)
                .count();
            let frac = hits as f32 / 120_000.0;
            assert!(
                (0.25..=0.75).contains(&frac),
                "seed {seed}: sea-level duty cycle {frac} outside 0.25..0.75"
            );
        }
    }

    #[test]
    fn high_ground_is_reliable() {
        // 30 blocks up — hilltop height in a 96-block world — the altitude term
        // alone carries the base breeze over the threshold nearly all the time:
        // "build it on the hill".
        for seed in [0u32, 1, 42, 2026, 999_983] {
            let hits = (0..120_000u64)
                .filter(|&t| sample(t, Weather::CLEAR, seed, SEA_LEVEL + 30).speed >= START)
                .count();
            let frac = hits as f32 / 120_000.0;
            assert!(frac > 0.85, "seed {seed}: hilltop duty cycle {frac} must exceed 0.85");
        }
    }

    #[test]
    fn with_altitude_matches_a_direct_sample() {
        for t in (0..12_000u64).step_by(311) {
            for y in [SEA_LEVEL, SEA_LEVEL + 1, SEA_LEVEL + 40, SEA_LEVEL + 200] {
                let direct = sample(t, Weather::CLEAR, 77, y);
                let lifted = with_altitude(sample(t, Weather::CLEAR, 77, SEA_LEVEL), y);
                assert!(
                    (direct.speed - lifted.speed).abs() < 1e-6,
                    "lifting a sea-level sample must equal sampling at y (t={t} y={y})"
                );
                assert_eq!(direct.direction, lifted.direction);
            }
        }
    }

    #[test]
    fn wind_words_cover_the_whole_range() {
        assert_eq!(word(0.0), "calm");
        assert_eq!(word(0.19), "calm");
        assert_eq!(word(0.20), "light");
        assert_eq!(word(0.34), "light");
        assert_eq!(word(0.35), "fresh");
        assert_eq!(word(0.59), "fresh");
        assert_eq!(word(0.60), "strong");
        assert_eq!(word(0.84), "strong");
        assert_eq!(word(0.85), "gale");
        assert_eq!(word(1.0), "gale");
    }

    #[test]
    fn compass_runs_clockwise_from_north() {
        let want = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
        for (i, w) in want.iter().enumerate() {
            assert_eq!(compass(i as u8), *w);
        }
        assert_eq!(compass(8), "N", "the compass wraps");
    }

    #[test]
    fn gusts_are_slow_not_flickery() {
        // A gust must not change materially inside a second (20 ticks) — the
        // whole point of interpolating the noise rather than hashing per tick.
        for t in (0..24_000u64).step_by(53) {
            let a = sample(t, Weather::CLEAR, 5, SEA_LEVEL).speed;
            let b = sample(t + 20, Weather::CLEAR, 5, SEA_LEVEL).speed;
            assert!((a - b).abs() < 0.05, "wind jumped {} in one second at t={t}", (a - b).abs());
        }
    }
}

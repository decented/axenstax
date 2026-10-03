//! Nostrich ride physics — the wacky, drifty, water-skimming mount (owner spec:
//! "super fast, builds up; at full speed it runs over water; slow down and it
//! sinks + you fall off; the faster it goes the more it drifts — Mario-Kart fun").
//!
//! Pure + deterministic so the feel is unit-testable. `game_loop` owns the ECS
//! glue (reads input + water, applies the returned velocity to the mount, snaps
//! it to the water surface while skimming, and dismounts the rider on a sink).
//! Per-ride state (`speed`, `heading`) lives on the `PlayerSlot`.

use glam::Vec2;

/// b/s the Nostrich kicks off at the moment you start moving (a brisk trot).
pub const BASE_SPEED: f32 = 5.0;
/// b/s flat-out — a blazing ~3.4× the player sprint (5.6 b/s).
pub const MAX_SPEED: f32 = 19.0;
/// b/s² wind-up while holding a direction (base → max in ~2s).
pub const ACCEL: f32 = 7.0;
/// b/s² bleed-off when you let go (faster than it builds, so stopping is committal).
pub const DECEL: f32 = 16.0;
/// b/s you must be doing to run across water. Below this over water → sink.
pub const WATER_RUN_SPEED: f32 = 14.0;
/// rad/s steering authority at a standstill — snappy, turns on a dime.
pub const TURN_FAST: f32 = 9.0;
/// rad/s steering authority at top speed — drifty: the heading slides wide of
/// the stick, so fast turns carry you in an arc (the Mario-Kart slide).
pub const TURN_SLOW: f32 = 2.2;
/// Holding crouch shifts the drift curve DOWN by this fraction of the speed
/// range, so you slide at much lower speeds (the hold-to-drift button).
pub const DRIFT_HOLD_BOOST: f32 = 0.5;
/// Crouch hops you off only below this speed (b/s); above it, crouch DRIFTS
/// instead of dismounting, so you can't accidentally bail mid-corner.
pub const DISMOUNT_SPEED: f32 = 1.5;

/// The outcome of one 20-TPS step of ride physics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RideStep {
    /// New forward speed (b/s) — persist back onto the player.
    pub speed: f32,
    /// New drifted heading (unit-ish, `(x, z)`) — persist back onto the player.
    pub heading: Vec2,
    /// Horizontal velocity to apply this tick, in **b/s** (`heading * speed`).
    pub vel_xz: Vec2,
    /// Skimming the water surface this tick (fast enough over water).
    pub water_run: bool,
    /// Sank — too slow over water. The rider falls off (dismount).
    pub dismount: bool,
}

/// Advance the ride one tick. `input_dir` is the desired horizontal direction
/// from WASD-relative-to-camera (`(x, z)`, zero if no input). `drift` = crouch
/// held (drift onsets at lower speeds). `over_water` = there is a water column
/// under the mount's feet. `dt` = seconds per tick.
pub fn step(
    mut speed: f32,
    mut heading: Vec2,
    input_dir: Vec2,
    drift: bool,
    over_water: bool,
    dt: f32,
) -> RideStep {
    let throttle = input_dir.length_squared() > 1e-6;
    // Speed: wind up while throttling (with an initial kick off the line),
    // bleed off when coasting.
    if throttle {
        if speed < BASE_SPEED {
            speed = BASE_SPEED;
        }
        speed = (speed + ACCEL * dt).min(MAX_SPEED);
    } else {
        speed = (speed - DECEL * dt).max(0.0);
    }

    // Heading drift: turn toward the input, but with less authority the faster
    // you go — so at speed the heading lags the stick and you slide wide.
    let in_dir = input_dir.normalize_or_zero();
    if heading.length_squared() < 1e-6 {
        heading = in_dir; // first frame off the line: snap to input
    } else if in_dir.length_squared() > 1e-6 {
        let mut t = (speed / MAX_SPEED).clamp(0.0, 1.0);
        if drift {
            // Crouch: shift the drift curve down so you slide at lower speeds.
            t = (t + DRIFT_HOLD_BOOST).min(1.0);
        }
        let turn_rate = TURN_FAST + (TURN_SLOW - TURN_FAST) * t; // rad/s, ↓ with speed/drift
        heading = rotate_toward(heading.normalize_or_zero(), in_dir, turn_rate * dt);
    }

    let vel_xz = heading.normalize_or_zero() * speed;
    let water_run = over_water && speed >= WATER_RUN_SPEED;
    let dismount = over_water && speed < WATER_RUN_SPEED;
    RideStep { speed, heading, vel_xz, water_run, dismount }
}

/// Rotate unit vector `from` toward unit `to` by at most `max_rad` radians
/// (shortest way round). Returns a unit vector.
fn rotate_toward(from: Vec2, to: Vec2, max_rad: f32) -> Vec2 {
    let a0 = from.y.atan2(from.x);
    let a1 = to.y.atan2(to.x);
    let mut d = a1 - a0;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    let a = a0 + d.clamp(-max_rad, max_rad);
    Vec2::new(a.cos(), a.sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    const DT: f32 = 1.0 / 20.0;
    fn fwd() -> Vec2 {
        Vec2::new(1.0, 0.0)
    }

    fn across() -> Vec2 {
        Vec2::new(0.0, 1.0)
    }
    fn turned(h: Vec2) -> f32 {
        let h = h.normalize_or_zero();
        h.y.atan2(h.x).abs()
    }

    #[test]
    fn speed_builds_up_from_a_kick_to_the_cap() {
        // First throttled frame kicks to BASE, then climbs toward MAX.
        let s0 = step(0.0, Vec2::ZERO, fwd(), false, false, DT);
        assert!(s0.speed >= BASE_SPEED && s0.speed < MAX_SPEED, "kicks off then climbs: {}", s0.speed);
        let mut speed = s0.speed;
        let mut heading = s0.heading;
        for _ in 0..200 {
            let s = step(speed, heading, fwd(), false, false, DT);
            speed = s.speed;
            heading = s.heading;
        }
        assert!((speed - MAX_SPEED).abs() < 1e-3, "holds at the cap: {speed}");
    }

    #[test]
    fn coasting_bleeds_speed_to_zero() {
        let mut speed = MAX_SPEED;
        let mut heading = fwd();
        for _ in 0..200 {
            let s = step(speed, heading, Vec2::ZERO, false, false, DT);
            speed = s.speed;
            heading = s.heading;
        }
        assert_eq!(speed, 0.0, "no throttle → coasts to a stop");
    }

    #[test]
    fn runs_on_water_only_when_fast_else_sinks() {
        let fast = step(WATER_RUN_SPEED + 1.0, fwd(), fwd(), false, true, DT);
        assert!(fast.water_run && !fast.dismount, "fast over water skims");
        let slow = step(WATER_RUN_SPEED - 1.0, fwd(), fwd(), false, true, DT);
        assert!(!slow.water_run && slow.dismount, "slow over water sinks");
        let land = step(2.0, fwd(), fwd(), false, false, DT);
        assert!(!land.water_run && !land.dismount, "land never sinks");
    }

    #[test]
    fn drift_is_stronger_at_speed() {
        // Same 90° input: at low speed the heading turns MORE per tick than at
        // high speed (high speed = drifty, turns slower).
        let slow = step(BASE_SPEED, fwd(), across(), false, false, DT);
        let fast = step(MAX_SPEED, fwd(), across(), false, false, DT);
        assert!(
            turned(slow.heading) > turned(fast.heading),
            "low speed turns harder ({}) than high speed ({})",
            turned(slow.heading),
            turned(fast.heading)
        );
    }

    #[test]
    fn crouch_drift_starts_drift_at_lower_speeds() {
        // At the SAME low-ish speed, holding crouch turns LESS (drifts more) —
        // the drift onset moves down the speed range.
        let grip = step(BASE_SPEED, fwd(), across(), false, false, DT);
        let drift = step(BASE_SPEED, fwd(), across(), true, false, DT);
        assert!(
            turned(drift.heading) < turned(grip.heading),
            "crouch drifts at low speed: drift {} < grip {}",
            turned(drift.heading),
            turned(grip.heading)
        );
    }

    #[test]
    fn velocity_follows_heading_and_speed() {
        let s = step(10.0, fwd(), fwd(), false, false, DT);
        assert!((s.vel_xz.length() - s.speed).abs() < 1e-3, "|vel| == speed");
    }
}

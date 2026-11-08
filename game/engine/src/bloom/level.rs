//! Target, level and tempo (living-world spec §1.4), and the stage words (§1.6).

/// Seconds for the level to close the gap when it is falling.
pub const TAU_DOWN: f32 = 2400.0;
/// Seconds for the level to close the gap when it is rising.
pub const TAU_UP: f32 = 360.0;
/// People can lift even a grey place, but only so far.
pub const PEOPLE_LIFT: f32 = 0.7;
/// Creative mode is vivid and neutral.
pub const CREATIVE_FLOOR: f32 = 0.6;
/// What a place we know nothing about reads as.
pub const UNKNOWN_LEVEL: f32 = 0.6;
/// Below this level (and eco) a place has gone grey.
pub const GREY_BELOW: f32 = 0.25;
/// Stage thresholds.
const STIRRING_FROM: f32 = 0.25;
const WAKING_FROM: f32 = 0.45;

/// Where the level is heading: `1 - (1 - eco)(1 - 0.7 P)`, floored at 0.6 in
/// creative mode.
pub fn target(eco: f32, people: f32, creative: bool) -> f32 {
    let t = 1.0 - (1.0 - eco) * (1.0 - PEOPLE_LIFT * people);
    let t = if creative { t.max(CREATIVE_FLOOR) } else { t };
    t.clamp(0.0, 1.0)
}

/// One step of easing: slow drift down, fast recovery up. `scale` is the
/// world's bloom time scale (1.0 normally; trials set it).
pub fn ease(level: f32, target: f32, dt: f32, scale: f32) -> f32 {
    let tau = if target < level { TAU_DOWN } else { TAU_UP };
    level + (target - level) * (dt * scale / tau).clamp(0.0, 1.0)
}

/// The words for a level. Never a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    GreyAndStill,
    Stirring,
    WakingUp,
    Alive,
}

impl Stage {
    /// `at_least` is the trial's goal and `held`/`hold` how long the level has
    /// stayed at or above it and how long it must (seconds).
    pub fn of(level: f32, at_least: f32, held: f32, hold: f32) -> Self {
        if level >= at_least && held >= hold {
            Stage::Alive
        } else if level < STIRRING_FROM {
            Stage::GreyAndStill
        } else if level < WAKING_FROM.min(at_least) {
            Stage::Stirring
        } else {
            Stage::WakingUp
        }
    }

    pub fn phrase(self) -> &'static str {
        match self {
            Stage::GreyAndStill => "Grey and still",
            Stage::Stirring => "Something's stirring",
            Stage::WakingUp => "Waking up",
            Stage::Alive => "Alive",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_lift_a_grey_place_to_at_most_seven_tenths() {
        assert_eq!(target(0.0, 0.0, false), 0.0);
        assert!((target(0.0, 1.0, false) - 0.7).abs() < 1e-6);
        assert!((target(0.5, 1.0, false) - 0.85).abs() < 1e-6);
        assert_eq!(target(1.0, 0.0, false), 1.0);
    }

    #[test]
    fn creative_floors_the_target_but_never_lowers_it() {
        assert_eq!(target(0.0, 0.0, true), CREATIVE_FLOOR);
        assert_eq!(target(0.9, 0.0, true), 0.9);
    }

    #[test]
    fn falling_is_slower_than_rising_and_time_scale_speeds_both() {
        let step = |from: f32, to: f32, scale: f32| {
            let mut l = from;
            for _ in 0..(60 * 20) {
                l = ease(l, to, 0.05, scale);
            }
            (l - from).abs()
        };
        let fall = step(1.0, 0.0, 1.0);
        let rise = step(0.0, 1.0, 1.0);
        assert!(rise > fall * 4.0, "rise {rise} fall {fall}");
        assert!(step(1.0, 0.0, 12.0) > fall * 8.0);
        assert!(step(0.0, 1.0, 12.0) > rise);
        // One enormous step lands on the target, never past it.
        assert_eq!(ease(0.0, 1.0, 1e6, 1.0), 1.0);
    }

    #[test]
    fn stages_read_in_words() {
        assert_eq!(Stage::of(0.1, 0.55, 0.0, 20.0), Stage::GreyAndStill);
        assert_eq!(Stage::of(0.3, 0.55, 0.0, 20.0), Stage::Stirring);
        assert_eq!(Stage::of(0.5, 0.55, 0.0, 20.0), Stage::WakingUp);
        assert_eq!(Stage::of(0.6, 0.55, 5.0, 20.0), Stage::WakingUp, "not yet held long enough");
        assert_eq!(Stage::of(0.6, 0.55, 20.0, 20.0), Stage::Alive);
        assert_eq!(Stage::Stirring.phrase(), "Something's stirring");
    }
}

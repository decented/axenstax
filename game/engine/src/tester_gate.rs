//! The hidden "tester feedback" unlock — an Android-"developer mode"-style tap
//! counter for the Settings version line. Pure logic: time is passed in (egui's
//! frame clock in the UI, plain numbers in tests), no clock is read here.
//!
//! Spec: docs/foundations/2026-06-07-lobby-mailbox-feedback.md ("Tester gate").
//! The gate itself is `native_mailbox::feedback_enabled`; this module only
//! decides when seven taps have happened.
//!
//! NATIVE ONLY: the web build has no feedback channel to unlock.
#![cfg(not(target_arch = "wasm32"))]

/// Taps needed on the version line.
pub const TAPS_REQUIRED: u32 = 7;
/// Each tap must land within this many seconds of the previous one.
pub const TAP_WINDOW_SECS: f64 = 3.0;
/// The countdown hint starts from this tap.
pub const HINT_FROM_TAP: u32 = 3;
/// How long the "Tester feedback on" confirmation stays up.
pub const CONFIRM_SECS: f64 = 6.0;

/// The line shown in Settings; the thing being tapped.
pub fn version_line() -> String {
    format!("AxeNStax v{}", env!("CARGO_PKG_VERSION"))
}

/// The label of the checkbox that appears once tester feedback is on.
pub const CHECKBOX_LABEL: &str = "Tester feedback (/bug, /idea, /mailbox)";

/// Shown at the seventh tap.
pub const UNLOCKED_MESSAGE: &str = "Tester feedback on \u{2014} /bug and /idea are now available";

/// Result of one tap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapOutcome {
    /// Counted; not far enough along to say anything (taps 1 and 2).
    Counting,
    /// Counted; this many taps still to go (taps 3..=6).
    Hint { remaining: u32 },
    /// The seventh tap inside the window: switch tester feedback on.
    Unlocked,
}

/// Counts consecutive taps, each within [`TAP_WINDOW_SECS`] of the last.
#[derive(Clone, Debug, Default)]
pub struct TapCounter {
    count: u32,
    last_tap: Option<f64>,
    unlocked_at: Option<f64>,
}

impl TapCounter {
    /// Register one tap at time `now` (seconds, any monotonic clock).
    pub fn tap(&mut self, now: f64) -> TapOutcome {
        if let Some(last) = self.last_tap
            && (now - last) > TAP_WINDOW_SECS
        {
            self.count = 0;
        }
        self.last_tap = Some(now);
        self.count += 1;
        if self.count >= TAPS_REQUIRED {
            self.count = 0;
            self.last_tap = None;
            self.unlocked_at = Some(now);
            return TapOutcome::Unlocked;
        }
        if self.count >= HINT_FROM_TAP {
            TapOutcome::Hint { remaining: TAPS_REQUIRED - self.count }
        } else {
            TapOutcome::Counting
        }
    }

    /// Forget any partial count (e.g. the player turned the feature off).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The small line to show under the version at time `now`, if any: the
    /// countdown while a run of taps is live, or the confirmation for a few
    /// seconds after the unlock.
    pub fn message(&self, now: f64) -> Option<String> {
        if let Some(at) = self.unlocked_at
            && now - at <= CONFIRM_SECS
        {
            return Some(UNLOCKED_MESSAGE.to_string());
        }
        let last = self.last_tap?;
        if self.count >= HINT_FROM_TAP && now - last <= TAP_WINDOW_SECS {
            return Some(hint_text(TAPS_REQUIRED - self.count));
        }
        None
    }
}

/// "N more taps to turn on tester feedback" (singular at 1).
pub fn hint_text(remaining: u32) -> String {
    let s = if remaining == 1 { "" } else { "s" };
    format!("{remaining} more tap{s} to turn on tester feedback")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_quick_taps_unlock_on_the_seventh_and_not_before() {
        let mut c = TapCounter::default();
        for i in 0..6 {
            assert_ne!(c.tap(i as f64 * 0.5), TapOutcome::Unlocked, "tap {}", i + 1);
        }
        assert_eq!(c.tap(3.0), TapOutcome::Unlocked);
    }

    #[test]
    fn taps_exactly_at_the_window_edge_still_count() {
        let mut c = TapCounter::default();
        let mut out = TapOutcome::Counting;
        for i in 0..7 {
            out = c.tap(i as f64 * TAP_WINDOW_SECS);
        }
        assert_eq!(out, TapOutcome::Unlocked, "a gap of exactly 3 s is within the window");
    }

    #[test]
    fn a_gap_over_three_seconds_resets_the_count() {
        let mut c = TapCounter::default();
        for i in 0..5 {
            c.tap(i as f64 * 0.2);
        }
        // 3.01 s after the fifth tap: the run is dead, this is tap 1 again.
        assert_eq!(c.tap(0.8 + 3.01), TapOutcome::Counting);
        // It then takes 6 more quick taps (not 2) to unlock.
        let t0 = 0.8 + 3.01;
        for i in 1..6 {
            assert_ne!(c.tap(t0 + i as f64 * 0.2), TapOutcome::Unlocked);
        }
        assert_eq!(c.tap(t0 + 1.2), TapOutcome::Unlocked);
    }

    #[test]
    fn hints_start_at_tap_three_and_count_down() {
        let mut c = TapCounter::default();
        assert_eq!(c.tap(0.0), TapOutcome::Counting);
        assert_eq!(c.tap(0.1), TapOutcome::Counting);
        assert_eq!(c.tap(0.2), TapOutcome::Hint { remaining: 4 });
        assert_eq!(c.tap(0.3), TapOutcome::Hint { remaining: 3 });
        assert_eq!(c.tap(0.4), TapOutcome::Hint { remaining: 2 });
        assert_eq!(c.tap(0.5), TapOutcome::Hint { remaining: 1 });
        assert_eq!(c.tap(0.6), TapOutcome::Unlocked);
    }

    #[test]
    fn hint_text_reads_naturally() {
        assert_eq!(hint_text(4), "4 more taps to turn on tester feedback");
        assert_eq!(hint_text(1), "1 more tap to turn on tester feedback");
    }

    #[test]
    fn message_shows_the_countdown_only_while_the_run_is_live() {
        let mut c = TapCounter::default();
        assert_eq!(c.message(0.0), None);
        c.tap(0.0);
        c.tap(0.1);
        assert_eq!(c.message(0.1), None, "nothing is said for taps 1-2");
        c.tap(0.2);
        assert_eq!(c.message(0.3).as_deref(), Some("4 more taps to turn on tester feedback"));
        assert_eq!(c.message(0.2 + 3.01), None, "the hint disappears with the window");
    }

    #[test]
    fn unlocking_shows_the_confirmation_then_it_fades() {
        let mut c = TapCounter::default();
        for i in 0..7 {
            c.tap(i as f64 * 0.1);
        }
        assert_eq!(c.message(0.7).as_deref(), Some(UNLOCKED_MESSAGE));
        assert!(UNLOCKED_MESSAGE.contains("/bug and /idea are now available"));
        assert_eq!(c.message(0.6 + CONFIRM_SECS + 0.1), None);
    }

    #[test]
    fn reset_clears_a_partial_run_and_the_confirmation() {
        let mut c = TapCounter::default();
        for i in 0..4 {
            c.tap(i as f64 * 0.1);
        }
        c.reset();
        assert_eq!(c.message(0.4), None);
        assert_eq!(c.tap(0.5), TapOutcome::Counting, "starts again from tap 1");
    }

    #[test]
    fn the_version_line_carries_the_crate_version() {
        let v = version_line();
        assert!(v.starts_with("AxeNStax v"));
        assert!(v.ends_with(env!("CARGO_PKG_VERSION")));
    }
}

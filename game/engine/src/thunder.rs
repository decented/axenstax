//! Thunder presentation — the pure rules for how a lightning strike is HEARD:
//! how loud by distance, how long after the flash, and the small tick-based
//! schedule that holds a rumble back until then. Presentation only: it rolls
//! nothing and touches no world state. The sound itself is the recipe in
//! `audio.rs` ([`crate::audio::THUNDER_LAYERS`]).
//!
//! **The delay is deliberately not physical.** Real thunder trails a flash by
//! about 3 s per kilometre, so a bolt 30 blocks away would be heard within a
//! hundredth of a second. We hold it back `0.25 s + 0.02 s per block` instead
//! (about 0.3 s close, about 0.95 s at the farthest 34-block bolt), so the
//! flash always warns the player before the rumble starts. Spec 05 "Thunderstorms + lightning" and §11.
//!
//! Every strike the client presents (standalone, a lending host's server
//! strike, a joiner's presentation-only bolt) goes through
//! `GameState::present_strike`, which calls [`ThunderQueue::schedule`].

/// Logic ticks per second (the client's fixed step).
pub(crate) const TICKS_PER_SECOND: f32 = 20.0;

/// Distance (blocks) up to which a strike is "right here": full gain.
pub(crate) const NEAR_BLOCKS: f32 = 3.0;
/// Fall-off scale (blocks): the gain is `1 / (1 + (d - NEAR) / this)`, so it
/// is 1.0 at 3 blocks, 0.4 at 30 and about 0.37 at the 34-block maximum.
pub(crate) const FALLOFF_BLOCKS: f32 = 18.0;

/// Base delay between flash and rumble, seconds.
pub(crate) const DELAY_BASE_SECS: f32 = 0.25;
/// Extra delay per block of distance, seconds.
pub(crate) const DELAY_PER_BLOCK_SECS: f32 = 0.02;

/// A strike lands within 24 blocks sideways and a few up or down; anything
/// farther is clamped so a junk distance cannot schedule a rumble minutes away.
const MAX_DISTANCE_BLOCKS: f32 = 100.0;

/// More rumbles than this cannot be pending (strikes are 8 s apart and the
/// longest delay is about 1 s, so this is only a safety bound).
const MAX_PENDING: usize = 8;

fn clean_distance(d: f32) -> f32 {
    if d.is_finite() { d.clamp(0.0, MAX_DISTANCE_BLOCKS) } else { 0.0 }
}

/// Loudness factor (`0..=1`) of a strike `d` blocks from the listener's eye:
/// 1.0 up to 3 blocks, then falling monotonically; `gain(30) = 0.4`.
pub(crate) fn gain(d: f32) -> f32 {
    let d = clean_distance(d);
    1.0 / (1.0 + (d - NEAR_BLOCKS).max(0.0) / FALLOFF_BLOCKS)
}

/// Seconds between the flash and the start of the rumble for a strike `d`
/// blocks away (deliberately not the speed of sound, see the module doc).
pub(crate) fn delay_secs(d: f32) -> f32 {
    DELAY_BASE_SECS + DELAY_PER_BLOCK_SECS * clean_distance(d)
}

/// [`delay_secs`] in whole logic ticks (at least 1).
pub(crate) fn delay_ticks(d: f32) -> u32 {
    ((delay_secs(d) * TICKS_PER_SECOND).round() as u32).max(1)
}

struct Pending {
    ticks_left: u32,
    gain: f32,
}

/// Rumbles waiting for their delay to pass. A countdown per entry, stepped
/// once per logic tick by the client, so it needs no clock of its own (the
/// day clock can be rewritten under it; a countdown cannot be).
#[derive(Default)]
pub(crate) struct ThunderQueue {
    pending: Vec<Pending>,
}

impl ThunderQueue {
    /// Hold back a rumble for a strike `d` blocks from the listener.
    pub(crate) fn schedule(&mut self, d: f32) {
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
        }
        self.pending.push(Pending { ticks_left: delay_ticks(d), gain: gain(d) });
    }

    /// Step one logic tick. Returns the distance gains of the rumbles that
    /// are due now (an entry scheduled with a delay of `n` ticks is returned
    /// by the `n`th call after it).
    pub(crate) fn tick(&mut self) -> Vec<f32> {
        let mut due = Vec::new();
        self.pending.retain_mut(|p| {
            p.ticks_left = p.ticks_left.saturating_sub(1);
            if p.ticks_left == 0 {
                due.push(p.gain);
                false
            } else {
                true
            }
        });
        due
    }

    /// Drop everything pending (world change, entering a weather-free world).
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_is_one_when_close_and_falls_monotonically() {
        assert_eq!(gain(0.0), 1.0);
        assert_eq!(gain(3.0), 1.0);
        let mut prev = gain(3.0);
        let mut d = 3.0;
        while d <= 40.0 {
            d += 0.5;
            let g = gain(d);
            assert!(g < prev, "gain must fall with distance: gain({d}) = {g} >= {prev}");
            prev = g;
        }
        assert!((0.0..=1.0).contains(&gain(1000.0)));
        // The sound a laptop hears at 30 blocks is at most 45% of the one at 3.
        assert!(gain(30.0) / gain(3.0) <= 0.45, "gain(30)/gain(3) = {}", gain(30.0) / gain(3.0));
        assert!(gain(30.0) > 0.3, "a far bolt stays audible");
    }

    #[test]
    fn gain_survives_junk_distances() {
        assert_eq!(gain(f32::NAN), 1.0);
        assert_eq!(gain(-5.0), 1.0);
        assert!(gain(f32::INFINITY) > 0.0);
    }

    #[test]
    fn delay_is_a_quarter_second_plus_two_hundredths_a_block() {
        assert!((delay_secs(0.0) - 0.25).abs() < 1e-6);
        assert!((delay_secs(3.0) - 0.31).abs() < 1e-6);
        assert!((delay_secs(34.0) - 0.93).abs() < 1e-6);
        // Whole ticks at 20 TPS: about 0.3 s close, about 1 s at 34 blocks.
        assert_eq!(delay_ticks(0.0), 5);
        assert_eq!(delay_ticks(3.0), 6);
        assert_eq!(delay_ticks(34.0), 19);
        assert!(delay_ticks(f32::NAN) >= 1);
        // The flash always comes first: even point blank the rumble waits.
        assert!(delay_ticks(0.0) >= 5);
    }

    #[test]
    fn a_scheduled_rumble_is_due_exactly_its_delay_later() {
        let mut q = ThunderQueue::default();
        q.schedule(30.0);
        let n = delay_ticks(30.0);
        for i in 1..n {
            assert!(q.tick().is_empty(), "not due at tick {i} of {n}");
        }
        let due = q.tick();
        assert_eq!(due, vec![gain(30.0)], "due on tick {n}");
        assert!(q.tick().is_empty(), "plays once");
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn two_bolts_keep_their_own_distance_and_clear_drops_the_rest() {
        let mut q = ThunderQueue::default();
        q.schedule(3.0);
        q.schedule(30.0);
        let mut heard = Vec::new();
        for _ in 0..40 {
            heard.extend(q.tick());
        }
        assert_eq!(heard, vec![gain(3.0), gain(30.0)], "near first, far later");
        q.schedule(10.0);
        q.clear();
        for _ in 0..40 {
            assert!(q.tick().is_empty());
        }
    }

    #[test]
    fn the_queue_is_bounded() {
        let mut q = ThunderQueue::default();
        for _ in 0..50 {
            q.schedule(5.0);
        }
        assert!(q.len() <= 8);
    }
}

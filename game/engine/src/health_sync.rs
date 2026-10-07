//! A joiner's own health (Spec 04 §5.3.2, MP-D2a): the server's copy wins.
//!
//! The server lands the world's hits on a joiner's body — fall, drowning,
//! hostile melee, lava and fire, keg blasts — on the body it simulates, and
//! sends the result back in its `PlayerState.health`. Since C2a it also runs
//! the joiner's metabolism (hunger, regen, starvation, poison) and its eating
//! and sleeping (`item_actions`), so every heal is the server's too. What the
//! client still changes itself is reported as `InputPacket.health_delta`,
//! applied by the server when it simulates that input — a LOSS only: the
//! server counts a reported heal as nothing, so one the client shows itself
//! (an op's `/heal` on its own view) lasts only until the input carrying it
//! is acknowledged. Spec 04 §5.3.2 has the full table (which sources reach a
//! joiner at all).
//!
//! [`OwnHealth`] is the bookkeeping that keeps the bar steady across the
//! round trip, the health-shaped twin of the position prediction in
//! `prediction`: every reported change is kept, under the input sequence it
//! went out with, until a `StateUpdate` acknowledges that input
//! (`last_acked_input`); the bar shows the server's value plus the changes
//! not yet acknowledged, plus any change made since the last send.
//! Without it, a loss would flicker back for a round trip, and a change made
//! between a send and the next server apply would be lost.

use std::collections::VecDeque;

/// Changes smaller than this (HP) are noise, not a hit.
const HURT_EPSILON: f32 = 0.01;

/// What applying a server health value did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Applied {
    /// The health this client should now show (and hold).
    pub health: f32,
    /// The server's value came in below what this client was showing for it:
    /// a hit the server landed (flash the screen).
    pub hurt: bool,
    /// The health to show is zero for a living client: the server holds the
    /// body lower than this client knew (a hit it landed is still in flight
    /// to us), and our own changes not yet applied there take it the rest of
    /// the way. Zero is dead: the caller enters the death screen
    /// (`PlayerCombat::die`) — the server's copy dies too, from those same
    /// changes (with a `Died`) or from our next input reporting zero health,
    /// and holds the body dead until our Respawn (MP-A3; review D2a HIGH-2).
    pub died: bool,
}

/// One joiner's own-health bookkeeping. Reset when a session starts or ends
/// and on every death.
#[derive(Debug, Default)]
pub struct OwnHealth {
    /// The health this client last accounted for (at its last send or server
    /// apply): `None` until the first send, and after a death.
    baseline: Option<f32>,
    /// Client-owned changes sent but not yet acknowledged: `(seq, delta)`.
    unacked: VecDeque<(u64, f32)>,
}

impl OwnHealth {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything: a new session, or a death (the server's respawn
    /// sets the health; nothing in flight still applies).
    pub fn reset(&mut self) {
        self.baseline = None;
        self.unacked.clear();
    }

    /// The client-owned change to report with the next input: what the
    /// health did since it was last accounted for. Zero before the first
    /// send and while dead.
    pub fn pending(&self, current: f32, dead: bool) -> f32 {
        if dead {
            return 0.0;
        }
        self.baseline.map_or(0.0, |b| current - b)
    }

    /// The input carrying `delta` (from [`Self::pending`]) went out under
    /// sequence `seq`; `current` is the health it was computed from. A dead
    /// player reports nothing and starts afresh after its respawn.
    pub fn sent(&mut self, seq: u64, delta: f32, current: f32, dead: bool) {
        if dead {
            self.reset();
            return;
        }
        if delta != 0.0 {
            self.unacked.push_back((seq, delta));
        }
        self.baseline = Some(current);
    }

    /// The server holds the body at `server_health`, with every input up to
    /// `acked` applied. Returns the health to show: the server's value plus
    /// the changes it hasn't applied yet, plus anything this client changed
    /// since its last send (`current` − the baseline), clamped to
    /// `0..=max`. Nothing changes while `dead`, and a server value of zero is
    /// never applied: the server's own deaths and revivals arrive as its
    /// `Died` / `Respawned` events, never as a health value. (A body the
    /// server still holds dead reads zero for the round trip after this
    /// client chose Respawn; taking it would put a living player at zero
    /// health, and its next input would report a death and kill the
    /// respawned body.) A positive server value whose sum comes to zero is
    /// a death, though: [`Applied::died`]. A living client never holds zero
    /// health without dying, so the zero its next input reports is always a
    /// death it knows about.
    pub fn apply_server(
        &mut self,
        server_health: f32,
        acked: u64,
        current: f32,
        max: f32,
        dead: bool,
    ) -> Applied {
        if dead || !server_health.is_finite() || server_health <= 0.0 {
            return Applied { health: current, hurt: false, died: false };
        }
        while self.unacked.front().is_some_and(|&(seq, _)| seq <= acked) {
            self.unacked.pop_front();
        }
        let in_flight: f32 = self.unacked.iter().map(|&(_, d)| d).sum();
        let unsent = self.baseline.map_or(0.0, |b| current - b);
        let shown = (server_health + in_flight).clamp(0.0, max);
        let hurt = self.baseline.is_some_and(|b| shown < b - HURT_EPSILON);
        self.baseline = Some(shown);
        let health = (shown + unsent).clamp(0.0, max);
        Applied { health, hurt, died: health <= 0.0 }
    }

    /// Changes still waiting for the server (test hook).
    #[cfg(test)]
    fn in_flight(&self) -> usize {
        self.unacked.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: f32 = 20.0;

    #[test]
    fn the_first_send_establishes_the_baseline_and_reports_nothing() {
        let mut h = OwnHealth::new();
        assert_eq!(h.pending(17.0, false), 0.0);
        h.sent(1, 0.0, 17.0, false);
        assert_eq!(h.pending(17.0, false), 0.0);
        assert_eq!(h.pending(18.0, false), 1.0, "a regen pulse is reported");
    }

    #[test]
    fn the_servers_value_wins_for_a_hit() {
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 20.0, false);
        let a = h.apply_server(14.0, 1, 20.0, MAX, false);
        assert_eq!(a, Applied { health: 14.0, hurt: true, died: false });
        assert_eq!(h.pending(14.0, false), 0.0, "the server's hit is not echoed back");
    }

    #[test]
    fn a_heal_does_not_flicker_while_the_server_catches_up() {
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 12.0, false);
        // Eat: +4 locally, reported with input 2.
        let d = h.pending(16.0, false);
        assert_eq!(d, 4.0);
        h.sent(2, d, 16.0, false);
        // A StateUpdate that has only applied input 1 still shows 16.
        let a = h.apply_server(12.0, 1, 16.0, MAX, false);
        assert_eq!(a, Applied { health: 16.0, hurt: false, died: false });
        assert_eq!(h.in_flight(), 1);
        // Once input 2 is applied the server's own value is 16: no change.
        let a = h.apply_server(16.0, 2, 16.0, MAX, false);
        assert_eq!(a, Applied { health: 16.0, hurt: false, died: false });
        assert_eq!(h.in_flight(), 0);
    }

    #[test]
    fn a_change_made_after_the_last_send_survives_a_server_apply() {
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 10.0, false);
        // A frame-time action (eating) after this tick's send…
        let current = 13.0;
        // …then the next frame's StateUpdate arrives first.
        let a = h.apply_server(10.0, 1, current, MAX, false);
        assert_eq!(a.health, 13.0, "the unsent heal is kept");
        // …and is reported with the next input, exactly once.
        assert_eq!(h.pending(a.health, false), 3.0);
    }

    #[test]
    fn a_hit_and_an_unacknowledged_heal_combine() {
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 10.0, false);
        h.sent(2, 2.0, 12.0, false);
        // The server bit us for 5 before applying input 2.
        let a = h.apply_server(5.0, 1, 12.0, MAX, false);
        assert_eq!(a, Applied { health: 7.0, hurt: true, died: false });
    }

    #[test]
    fn clamped_to_max() {
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 19.0, false);
        h.sent(2, 5.0, 24.0, false);
        let a = h.apply_server(19.0, 1, 24.0, MAX, false);
        assert_eq!(a.health, MAX);
        assert!(!a.died);
    }

    /// Review D2a HIGH-2. The server holds the body at 1 (a zombie hit we
    /// haven't heard about yet), and our own poison tick (−1, in flight)
    /// takes the rest: the sum is zero, and a living client at zero is a
    /// dead one — never a player standing at 0 HP with no death screen.
    #[test]
    fn a_sum_that_reaches_zero_is_a_death() {
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 4.0, false);
        h.sent(2, -1.0, 3.0, false);
        let a = h.apply_server(1.0, 1, 3.0, MAX, false);
        assert_eq!(a, Applied { health: 0.0, hurt: true, died: true });
        // An unsent loss can take it there too.
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 3.0, false);
        let a = h.apply_server(1.0, 1, 2.0, MAX, false);
        assert!(a.died && a.health == 0.0);
        // Above zero is no death.
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 3.0, false);
        assert!(!h.apply_server(1.5, 1, 2.0, MAX, false).died);
    }

    #[test]
    fn a_zero_from_the_server_is_never_applied() {
        // Respawned locally while the server still holds the body dead: its
        // zero must not land (the next input would report a death).
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 20.0, false);
        let a = h.apply_server(0.0, 1, 20.0, MAX, false);
        assert_eq!(a, Applied { health: 20.0, hurt: false, died: false });
    }

    #[test]
    fn nothing_moves_while_dead_and_a_respawn_is_never_reported() {
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 6.0, false);
        // Dead: the server's zero doesn't touch us, and nothing is reported.
        assert_eq!(h.apply_server(0.0, 1, 0.0, MAX, true).health, 0.0);
        assert_eq!(h.pending(0.0, true), 0.0);
        h.sent(2, 0.0, 0.0, true);
        // Respawned locally to full: that jump is the server's respawn, not a heal.
        assert_eq!(h.pending(20.0, false), 0.0);
        h.sent(3, 0.0, 20.0, false);
        assert_eq!(
            h.apply_server(20.0, 3, 20.0, MAX, false),
            Applied { health: 20.0, hurt: false, died: false }
        );
    }

    #[test]
    fn a_non_finite_server_value_is_ignored() {
        let mut h = OwnHealth::new();
        h.sent(1, 0.0, 9.0, false);
        assert_eq!(h.apply_server(f32::NAN, 1, 9.0, MAX, false).health, 9.0);
    }
}

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

use crate::combat::PlayerCombat;

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

// ─── The joined slot's two C2a rules (pure, so non-GPU tests can pin them) ──

/// One fixed tick of a slot's combat state. A JOINED slot runs the hit and
/// attack timers only: its metabolism (hunger, regen, starvation, poison) is
/// the server's (C2a), which sends the hunger back as `own_hunger`. Every
/// other slot — single-player, a host's own — runs the whole of it.
pub fn tick_slot_combat(combat: &mut PlayerCombat, joined: bool) {
    if joined {
        combat.tick_timers();
    } else {
        combat.tick();
    }
}

/// A `StateUpdate`'s `own_hunger` lands on a joined slot's body (clamped to
/// its maximum); on any other slot it is ignored (the host's and
/// single-player's hunger is their own).
pub fn apply_own_hunger(combat: &mut PlayerCombat, joined: bool, own_hunger: u8) {
    if joined {
        combat.hunger = own_hunger.min(combat.max_hunger);
    }
}

/// May a slot take (or, joined, ask for) a bite now? Its eating cooldown
/// (fixed ticks, `PlayerSlot::eat_cooldown`) must be spent, and a JOINED
/// slot has one request in flight at a time: no new `Eat` while the last is
/// unanswered (`JoinerActions::eat_in_flight`). C2a verify M1.
pub fn may_eat_now(eat_cooldown: u32, joined: bool, eat_in_flight: bool) -> bool {
    eat_cooldown == 0 && !(joined && eat_in_flight)
}

/// What a right-click does when the held item might be a meal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EatClick {
    /// Take (or, joined, ask for) a bite.
    Eat,
    /// Food in hand and the body wants to eat, but a bite isn't due (the
    /// cooldown, or a joiner's request in flight): the click is swallowed, so
    /// holding right-click stays a meal and never falls through to place,
    /// open or sleep. C2b verify M2.
    Swallow,
    /// Not a meal: the click goes on to the rest of the chain (planting a
    /// carrot on a full stomach, a chest, a bed).
    Pass,
}

/// Classify a right-click: `is_food` in hand, `wants_to_eat` (hungry, or hurt
/// where eating heals), the slot's `eat_cooldown` (fixed ticks) and, joined,
/// whether an `Eat` is in flight.
pub fn eat_click(is_food: bool, wants_to_eat: bool, eat_cooldown: u32, joined: bool, eat_in_flight: bool) -> EatClick {
    if !is_food || !wants_to_eat {
        EatClick::Pass
    } else if may_eat_now(eat_cooldown, joined, eat_in_flight) {
        EatClick::Eat
    } else {
        EatClick::Swallow
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

    /// A hungry body that has run for a long time, so the drain, regen and
    /// starvation steps all have something to do if they run.
    fn drained() -> PlayerCombat {
        let mut c = PlayerCombat::new();
        c.hunger = 5;
        c.poison_ticks = 40;
        c
    }

    #[test]
    fn a_joined_slot_runs_its_timers_only() {
        let mut c = drained();
        c.attack_cooldown = 5;
        for _ in 0..crate::combat::HUNGER_DRAIN_INTERVAL_TICKS * 2 {
            tick_slot_combat(&mut c, true);
        }
        assert_eq!(c.hunger, 5, "no metabolism: the server's hunger is not drained here");
        assert_eq!(c.poison_ticks, 40, "poison is the server's too");
        assert_eq!(c.attack_cooldown, 0, "the timers still run");
    }

    #[test]
    fn any_other_slot_runs_the_whole_tick() {
        let mut c = drained();
        for _ in 0..crate::combat::HUNGER_DRAIN_INTERVAL_TICKS * 2 {
            tick_slot_combat(&mut c, false);
        }
        assert!(c.hunger < 5, "hunger drains");
        assert!(c.poison_ticks < 40, "poison ticks down");
    }

    #[test]
    fn a_joined_client_sends_no_second_eat_while_one_is_in_flight() {
        use crate::joiner_actions::{Asked, JoinerActions, Pending};
        let eat = || Pending { kind: Asked::Eat, mob: None, hotbar_slot: 0, held: None };
        let mut ja = JoinerActions::default();
        assert!(!ja.eat_in_flight());
        assert!(may_eat_now(0, true, ja.eat_in_flight()));
        // The first Eat goes out (input 5 is the next to be sent).
        let seq = ja.record(eat(), 5);
        assert!(ja.eat_in_flight());
        assert!(!may_eat_now(0, true, ja.eat_in_flight()), "one in flight: no second");
        assert!(may_eat_now(0, false, ja.eat_in_flight()), "a local slot has no server to wait on");
        assert!(!may_eat_now(1, false, false), "the cooldown still gates every path");
        // Answered: free again.
        assert!(ja.take(seq).is_some());
        assert!(may_eat_now(0, true, ja.eat_in_flight()));
        // Skipped for good (the server acknowledged the input after it, its
        // answer never came): it stops blocking, so a lost request can't stop
        // eating for good.
        ja.record(eat(), 9);
        assert!(ja.eat_in_flight());
        ja.acknowledged(9);
        assert!(!ja.eat_in_flight());
    }

    #[test]
    fn a_state_updates_own_hunger_lands_on_a_joined_slot_only() {
        let mut c = PlayerCombat::new();
        apply_own_hunger(&mut c, true, 7);
        assert_eq!(c.hunger, 7);
        apply_own_hunger(&mut c, false, 3);
        assert_eq!(c.hunger, 7, "not joined: ignored");
        apply_own_hunger(&mut c, true, 250);
        assert_eq!(c.hunger, c.max_hunger, "clamped to the body's maximum");
    }

    /// C2b verify M2 — a held click between bites is swallowed, never passed
    /// on to the place / open / sleep chain; a full stomach or a non-food
    /// item still passes.
    #[test]
    fn a_held_click_between_bites_is_swallowed_not_passed_to_place() {
        use EatClick::*;
        assert_eq!(eat_click(true, true, 0, false, false), Eat);
        assert_eq!(eat_click(true, true, 7, false, false), Swallow, "mid-cooldown: still a meal");
        assert_eq!(eat_click(true, true, 7, true, false), Swallow);
        assert_eq!(eat_click(true, true, 0, true, true), Swallow, "a joiner's bite in flight");
        assert_eq!(eat_click(true, true, 0, false, true), Eat, "a local slot has no server to wait on");
        assert_eq!(eat_click(true, false, 0, false, false), Pass, "full and unhurt: plant the carrot");
        assert_eq!(eat_click(false, true, 0, false, false), Pass, "not food");
    }
}

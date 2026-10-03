//! Neighbour-update / scheduled-tick scheduler (Spec 48 — Primitive #1).
//!
//! The engine has no "block A changes → notify block B" mechanism: every
//! block-entity ticks in isolation, and water only spreads because it
//! hand-rolls its own neighbour-notify. This generalises that into an
//! engine-generic scheduler the power layer (and, later, Aether or a water
//! migration) reuses.
//!
//! Two queues:
//! * `pending` — positions needing re-evaluation THIS tick (deduped).
//! * `scheduled` — delayed updates keyed by ABSOLUTE target tick (gate settle,
//!   button release, plate release), drained when their tick comes due.
//!
//! The scheduler is transient: it is never serialised. On world load the power
//! layer re-seeds it by enqueuing every source/switch, so `energised` state is
//! rederived rather than saved.

use ahash::AHashSet;
use std::collections::{BTreeMap, VecDeque};

pub type Pos = (i32, i32, i32);

/// Why a delayed update was scheduled. Lets the drain caller act on the
/// specific event (e.g. flip a button off) as well as re-flood the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleKind {
    /// Plain power re-evaluation at a later tick. No production scheduler
    /// call constructs this yet — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    PowerReeval,
    /// A logic gate's output applies one tick after its inputs settle.
    GateSettle,
    /// A momentary button returns to off.
    ButtonRelease,
    /// A pressure plate clears after the last entity steps off. Never
    /// actually scheduled — pressure-plate clearing apparently isn't wired
    /// through this scheduler (matched defensively in power.rs's drain, but
    /// nothing constructs it, not even a test).
    #[allow(dead_code)]
    PlateRelease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduledUpdate {
    pub pos: Pos,
    pub kind: ScheduleKind,
}

#[derive(Default)]
pub struct UpdateScheduler {
    pending: VecDeque<Pos>,
    in_pending: AHashSet<Pos>,
    scheduled: BTreeMap<u64, Vec<ScheduledUpdate>>,
}

impl UpdateScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a position for re-evaluation this tick. Deduped — a position
    /// already pending is not added twice.
    pub fn enqueue(&mut self, pos: Pos) {
        if self.in_pending.insert(pos) {
            self.pending.push_back(pos);
        }
    }

    /// Enqueue the six cardinal neighbours of `pos`.
    pub fn enqueue_neighbours(&mut self, pos: Pos) {
        for f in crate::meta::Facing::ALL {
            let (dx, dy, dz) = f.offset();
            self.enqueue((pos.0 + dx, pos.1 + dy, pos.2 + dz));
        }
    }

    /// Schedule `pos` for re-evaluation at absolute tick `due`.
    pub fn schedule(&mut self, pos: Pos, due: u64, kind: ScheduleKind) {
        self.scheduled
            .entry(due)
            .or_default()
            .push(ScheduledUpdate { pos, kind });
    }

    /// Move every scheduled update due at or before `now` into the pending
    /// queue, returning the drained updates in tick order (so the caller can
    /// act on GateSettle / ButtonRelease / …).
    pub fn drain_due(&mut self, now: u64) -> Vec<ScheduledUpdate> {
        // `split_off(&k)` leaves keys < k in self and returns keys >= k.
        let future = self.scheduled.split_off(&(now + 1));
        let ready = std::mem::replace(&mut self.scheduled, future);
        let mut drained = Vec::new();
        for (_tick, ups) in ready {
            for u in ups {
                self.enqueue(u.pos);
                drained.push(u);
            }
        }
        drained
    }

    /// Pop up to `budget` pending positions for evaluation this tick. Overflow
    /// stays queued for next tick (water's bounded-budget pattern).
    pub fn take_pending(&mut self, budget: usize) -> Vec<Pos> {
        let mut out = Vec::with_capacity(budget.min(self.pending.len()));
        while out.len() < budget {
            match self.pending.pop_front() {
                Some(p) => {
                    self.in_pending.remove(&p);
                    out.push(p);
                }
                None => break,
            }
        }
        out
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// True while any pending or scheduled work remains.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn has_work(&self) -> bool {
        !self.pending.is_empty() || !self.scheduled.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_dedupes() {
        let mut s = UpdateScheduler::new();
        s.enqueue((1, 2, 3));
        s.enqueue((1, 2, 3));
        s.enqueue((4, 5, 6));
        assert_eq!(s.pending_len(), 2);
    }

    #[test]
    fn take_pending_is_fifo_and_bounded() {
        let mut s = UpdateScheduler::new();
        for i in 0..5 {
            s.enqueue((i, 0, 0));
        }
        let first = s.take_pending(3);
        assert_eq!(first, vec![(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
        // Remaining two stay queued.
        assert_eq!(s.pending_len(), 2);
        let rest = s.take_pending(10);
        assert_eq!(rest, vec![(3, 0, 0), (4, 0, 0)]);
        assert_eq!(s.pending_len(), 0);
    }

    #[test]
    fn taken_positions_can_be_re_enqueued() {
        // After a position is taken, the dedupe guard must let it back in.
        let mut s = UpdateScheduler::new();
        s.enqueue((7, 7, 7));
        let _ = s.take_pending(1);
        s.enqueue((7, 7, 7));
        assert_eq!(s.pending_len(), 1);
    }

    #[test]
    fn enqueue_neighbours_adds_exactly_six_cardinals() {
        let mut s = UpdateScheduler::new();
        s.enqueue_neighbours((0, 0, 0));
        let got = s.take_pending(100);
        assert_eq!(got.len(), 6);
        for f in crate::meta::Facing::ALL {
            assert!(got.contains(&f.offset()), "missing neighbour {f:?}");
        }
    }

    #[test]
    fn scheduled_not_yet_due_stays() {
        let mut s = UpdateScheduler::new();
        s.schedule((1, 1, 1), 5, ScheduleKind::GateSettle);
        let drained = s.drain_due(4);
        assert!(drained.is_empty());
        assert_eq!(s.pending_len(), 0);
        assert!(s.has_work()); // still scheduled for tick 5
    }

    #[test]
    fn scheduled_due_moves_to_pending_and_returns_kind() {
        let mut s = UpdateScheduler::new();
        s.schedule((1, 1, 1), 5, ScheduleKind::ButtonRelease);
        let drained = s.drain_due(5);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].kind, ScheduleKind::ButtonRelease);
        assert_eq!(drained[0].pos, (1, 1, 1));
        assert_eq!(s.pending_len(), 1);
    }

    #[test]
    fn drain_collects_all_past_due_ticks_in_order() {
        let mut s = UpdateScheduler::new();
        s.schedule((0, 0, 3), 3, ScheduleKind::PowerReeval);
        s.schedule((0, 0, 1), 1, ScheduleKind::PowerReeval);
        s.schedule((0, 0, 9), 9, ScheduleKind::PowerReeval);
        let drained = s.drain_due(5); // ticks 1 and 3 are due; 9 is not
        let positions: Vec<Pos> = drained.iter().map(|u| u.pos).collect();
        assert_eq!(positions, vec![(0, 0, 1), (0, 0, 3)]);
        assert!(s.has_work()); // tick 9 still pending
    }
}

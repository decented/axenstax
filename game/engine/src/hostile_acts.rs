//! World hostile-act ledger (Rail Freight P3 — transit robbery). Solo Buildout
//! Wave 5.
//!
//! The engine has a *cheat* ledger (World Integrity) but no record of in-world
//! **hostile acts** — robbing a freight cart, and (later) other griefing. This
//! is that ledger: a small, append-only, serialised record the robbery mechanic
//! writes to, and that reputation / bounty consequences (and, with multiplayer
//! identity, the victim + perpetrator) read from later.
//!
//! Design intent (`docs/foundations/2026-06-09-rail-freight-logistics.md` P3):
//! robbery is **spice, not tax** — rare, costly, transit-only. Only a cart
//! breached **while in transit** records here; breaking a *parked* cart at a
//! depot is the owner reclaiming it (no hostile act).

use serde::{Deserialize, Serialize};

/// What kind of hostile act was recorded. One variant today; extensible (the
/// reason it's an enum rather than a bool) for future griefing categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostileActKind {
    /// A freight cart was breached open while rolling (in transit), spilling
    /// its cargo — the rail-freight robbery.
    CartRobbery,
}

/// One recorded hostile act: what, where, and when (the world tick). The
/// perpetrator/victim identity is deliberately absent — it arrives with the
/// multiplayer-identity work (Spec 1 Phase 4); in single-player the act is the
/// player's own and the record exists to drive consequences + the audit trail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostileAct {
    pub kind: HostileActKind,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub tick: u64,
}

/// The world's hostile-act ledger. A thin wrapper over a `Vec` so the record /
/// query surface is one type (and future caps / pruning land in one place).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HostileActLedger {
    acts: Vec<HostileAct>,
}

impl HostileActLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a hostile act. Returns the new total count.
    pub fn record(&mut self, kind: HostileActKind, pos: (i32, i32, i32), tick: u64) -> usize {
        self.acts.push(HostileAct { kind, x: pos.0, y: pos.1, z: pos.2, tick });
        self.acts.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.acts.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.acts.is_empty()
    }

    /// How many recorded acts are of `kind`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn count_of(&self, kind: HostileActKind) -> usize {
        self.acts.iter().filter(|a| a.kind == kind).count()
    }

    /// The most recently recorded act, if any.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn last(&self) -> Option<&HostileAct> {
        self.acts.last()
    }

    /// Replace the ledger contents (load path).
    pub fn set_acts(&mut self, acts: Vec<HostileAct>) {
        self.acts = acts;
    }

    /// Borrow the raw acts (save path).
    pub fn acts(&self) -> &[HostileAct] {
        &self.acts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_appends_and_counts_by_kind() {
        let mut led = HostileActLedger::new();
        assert!(led.is_empty());
        assert_eq!(led.record(HostileActKind::CartRobbery, (10, 64, -3), 100), 1);
        assert_eq!(led.record(HostileActKind::CartRobbery, (11, 64, -3), 140), 2);
        assert_eq!(led.len(), 2);
        assert_eq!(led.count_of(HostileActKind::CartRobbery), 2);
        let last = led.last().unwrap();
        assert_eq!((last.x, last.y, last.z), (11, 64, -3));
        assert_eq!(last.tick, 140);
    }

    #[test]
    fn ledger_round_trips_through_serde() {
        let mut led = HostileActLedger::new();
        led.record(HostileActKind::CartRobbery, (1, 2, 3), 7);
        let json = serde_json::to_string(&led).unwrap();
        let back: HostileActLedger = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.last().unwrap().kind, HostileActKind::CartRobbery);
    }

    #[test]
    fn set_acts_replaces_contents() {
        let mut led = HostileActLedger::new();
        led.record(HostileActKind::CartRobbery, (0, 0, 0), 1);
        led.set_acts(vec![HostileAct {
            kind: HostileActKind::CartRobbery,
            x: 5,
            y: 5,
            z: 5,
            tick: 9,
        }]);
        assert_eq!(led.len(), 1);
        assert_eq!(led.last().unwrap().x, 5);
    }
}

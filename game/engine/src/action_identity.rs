//! C3d action order, independent of movement acknowledgement and window events.
//! Admission owns no inventory: callers stage a rule before terminal commit.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InventoryMode {
    Mirror,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionHeader {
    pub session: [u8; 16],
    pub epoch: u32,
    pub seq: u64,
    pub life: u32,
    pub events_applied: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionFate {
    Accepted,
    Refused,
    NormalisedAcceptedNoop,
    CancelledNoop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionAdmission {
    Expected,
    TerminalDuplicate,
    Superseded,
    RecoverGap,
    Paused,
    WrongSession,
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCut {
    pub session: [u8; 16],
    pub id: u64,
    pub epoch: u32,
    pub life: u32,
    pub through: u64,
}

/// A terminal frontier advances only after a complete staged external commit.
/// Uncertainty pauses BEFORE the current action; definitive refusal includes it.
#[derive(Debug)]
pub struct ActionFrontier {
    session: [u8; 16],
    epoch: u32,
    life: u32,
    through: u64,
    recovery: Option<RecoveryCut>,
    last_recovery: u64,
}

impl ActionFrontier {
    pub fn new(session: [u8; 16], life: u32) -> Self {
        Self { session, epoch: 0, life, through: 0, recovery: None, last_recovery: 0 }
    }

    pub fn header(&self, seq: u64, events_applied: u32) -> ActionHeader {
        ActionHeader { session: self.session, epoch: self.epoch, seq, life: self.life, events_applied }
    }

    pub fn through(&self) -> u64 { self.through }
    pub fn recovery(&self) -> Option<RecoveryCut> { self.recovery }

    pub fn admit(&self, action: ActionHeader) -> ActionAdmission {
        if action.session != self.session { return ActionAdmission::WrongSession; }
        if action.epoch != self.epoch || action.life != self.life { return ActionAdmission::Superseded; }
        if self.recovery.is_some() { return ActionAdmission::Paused; }
        if action.seq == 0 { return ActionAdmission::RecoverGap; }
        if action.seq <= self.through { return ActionAdmission::TerminalDuplicate; }
        let Some(next) = self.through.checked_add(1) else { return ActionAdmission::Exhausted; };
        if action.seq == next { ActionAdmission::Expected } else { ActionAdmission::RecoverGap }
    }

    /// Call after the complete effect and its immediate events are registered.
    pub fn terminal(&mut self, action: ActionHeader, _fate: ActionFate) -> Result<(), ActionAdmission> {
        let admission = self.admit(action);
        if admission != ActionAdmission::Expected { return Err(admission); }
        self.through = action.seq;
        Ok(())
    }

    pub fn prepare(&mut self) -> Result<RecoveryCut, ActionAdmission> {
        if let Some(cut) = self.recovery { return Ok(cut); }
        let epoch = self.epoch.checked_add(1).ok_or(ActionAdmission::Exhausted)?;
        let id = self.last_recovery.checked_add(1).ok_or(ActionAdmission::Exhausted)?;
        let cut = RecoveryCut { session: self.session, id, epoch, life: self.life, through: self.through };
        self.epoch = epoch;
        self.last_recovery = id;
        self.recovery = Some(cut);
        Ok(cut)
    }

    /// The transport ACK must name the full cut, not merely its newer epoch.
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d recovery milestone (RebasePrepare, Describe, capture, applied fence and replay)"))]
    pub fn acknowledge(&mut self, cut: RecoveryCut) -> bool {
        if self.recovery != Some(cut) { return false; }
        self.recovery = None;
        true
    }

    /// Death supersedes generic recovery. The bounded death describe names the
    /// highest allocated old-life action, including unsent predictions. Its
    /// validated cancellation frontier closes that suffix before the new life.
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d death/life-change milestone (old-life suffix cancellation)"))]
    pub fn change_life(&mut self, life: u32, cancelled_through: u64) -> Result<(), ActionAdmission> {
        if life <= self.life || cancelled_through < self.through { return Err(ActionAdmission::Superseded); }
        self.epoch = self.epoch.checked_add(1).ok_or(ActionAdmission::Exhausted)?;
        self.life = life;
        self.through = cancelled_through;
        self.recovery = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c3d_partial_edits_are_not_retired_by_movement() {
        let mut frontier = ActionFrontier::new([17; 16], 1);
        // Ten edits share an input: only four admitted edits have completed.
        for seq in 1..=4 {
            frontier.terminal(frontier.header(seq, 0), ActionFate::Accepted).unwrap();
        }
        let _movement_acked_input = 900;
        let uncertain = frontier.header(5, 0);
        let cut = frontier.prepare().unwrap();
        assert_eq!(cut.through, 4);
        assert_eq!(frontier.admit(uncertain), ActionAdmission::Superseded);
        assert_eq!(frontier.admit(frontier.header(5, 0)), ActionAdmission::Paused);
        assert!(!frontier.acknowledge(RecoveryCut { session: [99; 16], ..cut }));
        assert!(frontier.acknowledge(cut));
        assert_eq!(frontier.admit(frontier.header(5, 0)), ActionAdmission::Expected);
    }

    #[test]
    fn c3d_every_mixed_cut_commits_external_effects_once() {
        // Edits, chest deposit and projectile share ONE sequence. A cut never
        // turns a committed external effect into an intent to send again.
        let effects = [1, 1, 100, 1, 1000, 1];
        for cut_at in 0..=effects.len() {
            let mut frontier = ActionFrontier::new([31; 16], 1);
            let mut total = 0;
            for (index, effect) in effects.iter().enumerate().take(cut_at) {
                frontier.terminal(frontier.header(index as u64 + 1, 0), ActionFate::Accepted).unwrap();
                total += effect;
            }
            let cut = frontier.prepare().unwrap();
            assert_eq!(cut.through, cut_at as u64);
            assert!(frontier.acknowledge(cut));
            for (index, effect) in effects.iter().enumerate() {
                let action = frontier.header(index as u64 + 1, 0);
                if frontier.admit(action) == ActionAdmission::Expected {
                    frontier.terminal(action, ActionFate::Accepted).unwrap();
                    total += effect;
                }
            }
            assert_eq!(total, effects.iter().sum::<i32>());
        }
    }

    #[test]
    fn c3d_refusal_and_cancellation_close_exact_sequence_positions() {
        let mut frontier = ActionFrontier::new([7; 16], 1);
        frontier.terminal(frontier.header(1, 0), ActionFate::Refused).unwrap();
        let cut = frontier.prepare().unwrap();
        assert_eq!(cut.through, 1);
        assert!(frontier.acknowledge(cut));
        frontier.terminal(frontier.header(2, 0), ActionFate::CancelledNoop).unwrap();
        assert_eq!(frontier.admit(frontier.header(2, 0)), ActionAdmission::TerminalDuplicate);
        assert_eq!(frontier.admit(frontier.header(4, 0)), ActionAdmission::RecoverGap);
        assert_eq!(frontier.admit(frontier.header(3, 0)), ActionAdmission::Expected);
    }

    #[test]
    fn c3d_death_and_namespace_supersede_incomplete_recovery() {
        let mut frontier = ActionFrontier::new([3; 16], 1);
        let old = frontier.header(1, 0);
        let cut = frontier.prepare().unwrap();
        frontier.change_life(2, 10).unwrap();
        assert_eq!(frontier.admit(frontier.header(11, 0)), ActionAdmission::Expected);
        assert!(!frontier.acknowledge(cut));
        assert_eq!(frontier.admit(old), ActionAdmission::Superseded);
        let other = ActionHeader { session: [4; 16], ..frontier.header(1, 0) };
        assert_eq!(frontier.admit(other), ActionAdmission::WrongSession);
        frontier.epoch = u32::MAX;
        assert_eq!(frontier.prepare(), Err(ActionAdmission::Exhausted));
        assert_eq!(frontier.epoch, u32::MAX);
    }
}

//! Bounded retained action bytes. Sending and movement ACKs retire nothing.
//! Boxed allocations make retained byte accounting independent of Vec capacity.

use std::collections::VecDeque;
use crate::action_identity::{ActionHeader, RecoveryCut};
use crate::action_wire::ActionReceipt;

pub const MAX_ACTIONS: usize = 2048;
pub const MAX_RETAINED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_INTENT_BYTES: usize = 32 * 1024;
pub const MAX_EVENT_ENTRIES: usize = 1024;
pub const MAX_CONTEXT_CELLS: usize = 8192;
pub const MAX_CONTEXT_BYTES: usize = 512 * 1024;
pub const MAX_BUNDLE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PAGE_BYTES: usize = 32 * 1024;
pub const MAX_RESULT_ENTRIES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalError { IntentTooLarge, CountLimit, ByteLimit, SequenceExhausted, InvalidFrontier }

#[derive(Debug)]
pub struct RetainedAction {
    pub header: ActionHeader,
    pub intent: Box<[u8]>,
    /// Encoded pure prediction/semantic preconditions, never Plan body bytes.
    pub recipe: Box<[u8]>,
    /// Touched cells include dependencies and unsent overlays.
    pub cells: Box<[[i32; 3]]>,
    pub sent: bool,
}

impl RetainedAction {
    fn bytes(&self) -> Result<usize, JournalError> {
        self.intent.len().checked_add(self.recipe.len())

            .and_then(|n| self.cells.len().checked_mul(std::mem::size_of::<[i32; 3]>()).and_then(|cells| n.checked_add(cells)))
            .ok_or(JournalError::ByteLimit)
    }
}

/// Retains the whole logical suffix until an applied authoritative boundary.
#[derive(Debug)]
pub struct ActionJournal {
    session: [u8; 16],
    epoch: u32,
    life: u32,
    next: u64,
    retired_through: u64,
    /// Included base events, distinct from later applied world-stream events.
    included_events: u32,
    /// Conservative sum: duplicate cells also reserve complete Describe room.
    context_cells: usize,
    bytes: usize,
    actions: VecDeque<RetainedAction>,
}

impl ActionJournal {
    pub fn new(session: [u8; 16], life: u32) -> Self {
        let actions = VecDeque::with_capacity(MAX_ACTIONS);
        let bytes = std::mem::size_of::<Self>() + actions.capacity() * std::mem::size_of::<RetainedAction>();
        Self { session, epoch: 0, life, next: 1, retired_through: 0, included_events: 0, context_cells: 0, bytes, actions }
    }

    pub fn len(&self) -> usize { self.actions.len() }
    pub fn is_empty(&self) -> bool { self.actions.is_empty() }
    pub fn retained_bytes(&self) -> usize { self.bytes }
    pub fn actions(&self) -> impl Iterator<Item = &RetainedAction> { self.actions.iter() }
    pub fn next_header(&self, events_applied: u32) -> Result<ActionHeader, JournalError> {
        self.next.checked_add(1).ok_or(JournalError::SequenceExhausted)?;
        Ok(ActionHeader { session: self.session, epoch: self.epoch, seq: self.next, life: self.life, events_applied })
    }
    pub fn accepts_cut(&self, cut: RecoveryCut) -> bool {
        cut.session == self.session && cut.life == self.life && cut.epoch > self.epoch
            && cut.through >= self.retired_through && cut.through < self.next
    }

    /// Allocation succeeds before the caller is permitted to predict anything.
    pub fn append(&mut self, intent: Box<[u8]>, recipe: Box<[u8]>, cells: Box<[[i32; 3]]>, events_applied: u32) -> Result<ActionHeader, JournalError> {
        if intent.len() > MAX_INTENT_BYTES { return Err(JournalError::IntentTooLarge); }
        if self.actions.len() == MAX_ACTIONS { return Err(JournalError::CountLimit); }
        if events_applied < self.included_events { return Err(JournalError::InvalidFrontier); }
        let context_cells = self.context_cells.checked_add(cells.len()).ok_or(JournalError::ByteLimit)?;
        if context_cells > MAX_CONTEXT_CELLS { return Err(JournalError::ByteLimit); }
        let next = self.next.checked_add(1).ok_or(JournalError::SequenceExhausted)?;
        let header = ActionHeader { session: self.session, epoch: self.epoch, seq: self.next, life: self.life, events_applied };
        let action = RetainedAction { header, intent, recipe, cells, sent: false };
        let bytes = action.bytes()?;
        let total = self.bytes.checked_add(bytes).ok_or(JournalError::ByteLimit)?;
        if total > MAX_RETAINED_BYTES { return Err(JournalError::ByteLimit); }
        self.actions.push_back(action);
        self.bytes = total;
        self.context_cells = context_cells;
        self.next = next;
        Ok(header)
    }

    pub fn mark_sent(&mut self, seq: u64) -> bool {
        let Some(action) = self.actions.iter_mut().find(|a| a.header.seq == seq) else { return false };
        action.sent = true;
        true
    }

    /// Ordinary receipts retire only after their real world-stream fence and
    /// included window events apply. Older receipts cannot regress the prefix.
    pub fn retire_receipt(&mut self, receipt: ActionReceipt, applied_events: u32, applied_through: u64) -> Result<(), JournalError> {
        let action = receipt.action;
        if action.session != self.session || action.epoch != self.epoch || action.life != self.life
            || action.seq < self.retired_through || action.seq >= self.next || receipt.included_events < self.included_events
            || applied_events < receipt.included_events || applied_through < action.seq {
            return Err(JournalError::InvalidFrontier);
        }
        while self.actions.front().is_some_and(|a| a.header.seq <= action.seq) {
            let action = self.actions.pop_front().expect("front was present");
            self.bytes -= action.bytes()?;
            self.context_cells -= action.cells.len();
        }
        self.retired_through = action.seq;
        self.included_events = receipt.included_events;
        Ok(())
    }

    /// Call only after old body keys are durably retained and death's window
    /// applies. Cancel through the exact highest allocated old-life sequence.
    pub fn change_life(&mut self, life: u32, epoch: u32) -> Result<u64, JournalError> {
        if life <= self.life || epoch <= self.epoch { return Err(JournalError::InvalidFrontier); }
        let through = self.next - 1;
        self.actions.clear();
        self.context_cells = 0;
        self.bytes = std::mem::size_of::<Self>() + self.actions.capacity() * std::mem::size_of::<RetainedAction>();
        self.life = life;
        self.epoch = epoch;
        self.retired_through = through;
        Ok(through)
    }

    /// Caller supplies proof from actual world intake AFTER complete assembly,
    /// never socket reception. Events included in the base have also applied.
    pub fn activate(&mut self, cut: RecoveryCut, included_events: u32, applied_events: u32, applied_fence: Option<u64>) -> Result<(), JournalError> {
        if cut.session != self.session || cut.life != self.life || cut.epoch <= self.epoch || cut.through >= self.next || cut.through < self.retired_through
            || included_events < self.included_events || applied_events < included_events || applied_fence != Some(cut.id) {
            return Err(JournalError::InvalidFrontier);
        }
        while self.actions.front().is_some_and(|a| a.header.seq <= cut.through) {
            let action = self.actions.pop_front().expect("front was present");
            self.bytes -= action.bytes()?;
            self.context_cells -= action.cells.len();
        }
        self.epoch = cut.epoch;
        self.retired_through = cut.through;
        self.included_events = included_events;
        for action in &mut self.actions {
            action.header.epoch = cut.epoch;
            action.header.events_applied = included_events;
            action.sent = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn append(journal: &mut ActionJournal, bytes: usize) -> Result<ActionHeader, JournalError> {
        journal.append(vec![0; bytes].into_boxed_slice(), Box::new([]), Box::new([]), 0)
    }
    fn receipt(action: ActionHeader) -> ActionReceipt {
        ActionReceipt { action, fate: crate::action_identity::ActionFate::Accepted, included_events: action.events_applied }
    }

    #[test]
    fn c3d_sending_and_socket_receipt_do_not_free_the_suffix() {
        let mut journal = ActionJournal::new([8; 16], 1);
        for _ in 0..10 { append(&mut journal, 100).unwrap(); }
        assert!(journal.mark_sent(1));
        assert_eq!(journal.len(), 10);
        let cut = RecoveryCut { session: [8; 16], id: 4, epoch: 1, life: 1, through: 4 };
        assert_eq!(journal.activate(cut, 9, 9, None), Err(JournalError::InvalidFrontier));
        assert_eq!(journal.activate(cut, 9, 8, Some(4)), Err(JournalError::InvalidFrontier));
        assert_eq!(journal.len(), 10);
        assert_eq!(journal.activate(RecoveryCut { session: [9; 16], ..cut }, 9, 9, Some(4)), Err(JournalError::InvalidFrontier));
        journal.activate(cut, 9, 9, Some(4)).unwrap();
        assert_eq!(journal.len(), 6);
        assert_eq!(journal.actions().map(|a| a.header.seq).collect::<Vec<_>>(), (5..=10).collect::<Vec<_>>());
        assert!(journal.actions().all(|a| !a.sent && a.header.epoch == 1 && a.header.events_applied == 9));
    }

    #[test]
    fn c3d_count_and_real_retained_bytes_stop_before_prediction() {
        let mut count = ActionJournal::new([1; 16], 1);
        for _ in 0..MAX_ACTIONS { append(&mut count, 0).unwrap(); }
        let before = count.retained_bytes();
        assert_eq!(append(&mut count, 0), Err(JournalError::CountLimit));
        assert_eq!(count.retained_bytes(), before);
        assert_eq!(count.len(), MAX_ACTIONS);
        let mut bytes = ActionJournal::new([2; 16], 1);
        while append(&mut bytes, MAX_INTENT_BYTES).is_ok() {}
        assert!(bytes.len() < MAX_ACTIONS);
        assert!(bytes.retained_bytes() <= MAX_RETAINED_BYTES);
        let before = bytes.retained_bytes();
        assert_eq!(append(&mut bytes, MAX_INTENT_BYTES), Err(JournalError::ByteLimit));
        assert_eq!(bytes.retained_bytes(), before);
        assert_eq!(append(&mut bytes, MAX_INTENT_BYTES + 1), Err(JournalError::IntentTooLarge));
    }

    #[test]
    fn c3d_recipe_and_unsent_context_are_accounted_and_exhaustion_never_wraps() {
        let mut journal = ActionJournal::new([5; 16], 1);
        journal.append(Box::new([1]), vec![2; 4096].into_boxed_slice(), vec![[3, 4, 5]; 100].into_boxed_slice(), 0).unwrap();
        assert_eq!(journal.retained_bytes(), std::mem::size_of::<ActionJournal>() + journal.actions.capacity() * std::mem::size_of::<RetainedAction>() + 1 + 4096 + 1200);
        journal.next = u64::MAX;
        assert_eq!(append(&mut journal, 0), Err(JournalError::SequenceExhausted));
        assert_eq!(journal.len(), 1);
        assert_eq!(journal.next, u64::MAX);
    }

    #[test]
    fn c3d_newer_epoch_cannot_regress_a_retired_terminal_cut() {
        let mut journal = ActionJournal::new([8; 16], 1);
        for _ in 0..10 { append(&mut journal, 0).unwrap(); }
        let received = ActionHeader { session: [8; 16], epoch: 0, life: 1, seq: 4, events_applied: 9 };
        assert_eq!(journal.retire_receipt(receipt(received), 9, 3), Err(JournalError::InvalidFrontier));
        journal.retire_receipt(receipt(received), 9, 4).unwrap();
        assert_eq!(journal.len(), 6);
        let regression = RecoveryCut { session: [8; 16], id: 1, epoch: 1, life: 1, through: 2 };
        assert_eq!(journal.activate(regression, 9, 9, Some(1)), Err(JournalError::InvalidFrontier));
        assert_eq!(journal.len(), 6);
        assert_eq!(journal.retire_receipt(receipt(ActionHeader { seq: 3, ..received }), 9, 4), Err(JournalError::InvalidFrontier));
    }

    #[test]
    fn c3d_included_event_base_never_regresses_after_receipt_or_recovery() {
        let mut journal = ActionJournal::new([8; 16], 1);
        for _ in 0..10 { append(&mut journal, 0).unwrap(); }
        let received = ActionHeader { session: [8; 16], epoch: 0, life: 1, seq: 4, events_applied: 10 };
        journal.retire_receipt(receipt(received), 10, 4).unwrap();
        let cut = RecoveryCut { session: [8; 16], id: 1, epoch: 1, life: 1, through: 4 };
        assert_eq!(journal.activate(cut, 5, 10, Some(1)), Err(JournalError::InvalidFrontier));
        assert_eq!(journal.retire_receipt(receipt(ActionHeader { seq: 5, events_applied: 9, ..received }), 10, 5), Err(JournalError::InvalidFrontier));
        journal.activate(cut, 10, 15, Some(1)).unwrap();
        let later = RecoveryCut { id: 2, epoch: 2, ..cut };
        assert_eq!(journal.activate(later, 9, 15, Some(2)), Err(JournalError::InvalidFrontier));
        // Applied post-fence events do not force the captured base up to 15.
        journal.activate(later, 10, 15, Some(2)).unwrap();
        assert!(journal.actions().all(|a| a.header.events_applied == 10));
    }

    #[test]
    fn c3d_retained_context_fits_one_complete_describe_and_releases_at_cuts() {
        let mut journal = ActionJournal::new([8; 16], 1);
        let cells = |offset| (0..MAX_CONTEXT_CELLS).map(|n| [n as i32, offset, 0]).collect::<Vec<_>>().into_boxed_slice();
        let first = journal.append(Box::new([]), Box::new([]), cells(0), 0).unwrap();
        let before = journal.retained_bytes();
        assert_eq!(journal.append(Box::new([]), Box::new([]), cells(1), 0), Err(JournalError::ByteLimit));
        assert_eq!(journal.len(), 1);
        assert_eq!(journal.retained_bytes(), before);
        journal.retire_receipt(receipt(first), 0, 1).unwrap();
        let second = journal.append(Box::new([]), Box::new([]), cells(1), 0).unwrap();
        assert_eq!(second.seq, 2, "failed reservation never allocates a sequence");
        let cut = RecoveryCut { session: [8; 16], id: 1, epoch: 1, life: 1, through: 2 };
        journal.activate(cut, 0, 0, Some(1)).unwrap();
        journal.append(Box::new([]), Box::new([]), cells(2), 0).unwrap();
        journal.change_life(2, 2).unwrap();
        journal.append(Box::new([]), Box::new([]), cells(3), 0).unwrap();
    }

    #[test]
    fn c3d_death_closes_unsent_old_life_suffix_without_a_new_namespace() {
        let mut journal = ActionJournal::new([8; 16], 1);
        let mut server = crate::action_identity::ActionFrontier::new([8; 16], 1);
        for _ in 0..10 { append(&mut journal, 0).unwrap(); }
        for seq in 1..=4 { server.terminal(server.header(seq, 0), crate::action_identity::ActionFate::Accepted).unwrap(); }
        let through = journal.change_life(2, 1).unwrap();
        assert_eq!(through, 10, "the unsent old-life actions are explicitly cancelled");
        server.change_life(2, through).unwrap();
        let next = append(&mut journal, 0).unwrap();
        assert_eq!(next.seq, 11);
        assert_eq!(next.session, [8; 16]);
        assert_eq!(server.admit(next), crate::action_identity::ActionAdmission::Expected);
    }
}

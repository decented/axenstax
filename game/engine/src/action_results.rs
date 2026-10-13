//! Immutable bounded server terminal results. Receipt acknowledgement is metadata.
use std::collections::VecDeque;
use crate::action_identity::ActionHeader;
use crate::action_journal::{MAX_ACTIONS, MAX_RETAINED_BYTES};
use crate::action_wire::{ActionReceipt, AppliedActionAck};

struct ResultRow { receipt: ActionReceipt, cells: Box<[[i32; 3]]>, needs_send: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultError { Namespace, Sequence, Capacity, InvalidAck }

pub struct ActionResults {
    owner: ActionHeader,
    terminal: u64,
    acked: u64,
    acked_events: u32,
    rows: VecDeque<ResultRow>,
    cell_count: usize,
}

pub struct PreparedResult { receipt: ActionReceipt, cells: Box<[[i32; 3]]>, expected_terminal: u64 }

impl ActionResults {
    pub fn new(owner: ActionHeader) -> Self {
        Self { owner, terminal: 0, acked: 0, acked_events: 0, rows: VecDeque::new(), cell_count: 0 }
    }
    fn owns(&self, action: ActionHeader) -> bool {
        action.session == self.owner.session && action.epoch == self.owner.epoch && action.life == self.owner.life
    }
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d recovery milestone: the server-known suffix footprint unions these cells into RebasePrepare"))]
    pub fn retained_cells(&self) -> impl Iterator<Item = &[i32; 3]> { self.rows.iter().flat_map(|r| r.cells.iter()) }
    pub fn retained_bytes(&self) -> usize { std::mem::size_of::<Self>() + self.rows.capacity() * std::mem::size_of::<ResultRow>() + self.cell_count * std::mem::size_of::<[i32; 3]>() }
    pub fn prepare(&mut self, receipt: ActionReceipt, cells: Box<[[i32; 3]]>) -> Result<PreparedResult, ResultError> {
        let cell_count = self.cell_count.checked_add(cells.len()).ok_or(ResultError::Capacity)?;
        if cell_count > crate::action_journal::MAX_CONTEXT_CELLS { return Err(ResultError::Capacity); }
        if !self.owns(receipt.action) { return Err(ResultError::Namespace); }
        if Some(receipt.action.seq) != self.terminal.checked_add(1) { return Err(ResultError::Sequence); }
        if self.rows.len() == MAX_ACTIONS { return Err(ResultError::Capacity); }
        let estimated = self.rows.capacity().max(self.rows.len() + 1).checked_mul(std::mem::size_of::<ResultRow>()).and_then(|n| n.checked_add(std::mem::size_of::<Self>())).ok_or(ResultError::Capacity)?;
        if estimated.checked_add(cell_count * std::mem::size_of::<[i32; 3]>()).is_none_or(|n| n > MAX_RETAINED_BYTES) { return Err(ResultError::Capacity); }
        let capacity_before = self.rows.capacity();
        self.rows.try_reserve_exact(1).map_err(|_| ResultError::Capacity)?;
        if self.retained_bytes().checked_add(cells.len() * std::mem::size_of::<[i32; 3]>()).is_none_or(|n| n > MAX_RETAINED_BYTES) {
            // Give back exactly what the reservation took: no capacity leaks.
            self.rows.shrink_to(capacity_before);
            return Err(ResultError::Capacity);
        }
        Ok(PreparedResult { receipt, cells, expected_terminal: self.terminal })
    }
    pub fn commit(&mut self, prepared: PreparedResult) {
        assert_eq!(self.terminal, prepared.expected_terminal, "terminal result reservation changed before commit");
        self.terminal = prepared.receipt.action.seq;
        self.cell_count += prepared.cells.len();
        self.rows.push_back(ResultRow { receipt: prepared.receipt, cells: prepared.cells, needs_send: true });
    }
    /// No row means no exact immutable result remains. The caller recovers or
    /// tears down; it must never invent a synthetic success/refusal/noop.
    pub fn repeat(&mut self, action: ActionHeader) -> Option<ActionReceipt> {
        if !self.owns(action) { return None; }
        let row = self.rows.iter_mut().find(|r| r.receipt.action.seq == action.seq)?;
        row.needs_send = true;
        Some(row.receipt)
    }
    /// Tests: the retained row for `action`, read without re-arming its send
    /// (a real client would hear a second receipt from [`Self::repeat`]).
    #[cfg(test)]
    pub fn peek(&self, action: ActionHeader) -> Option<ActionReceipt> {
        if !self.owns(action) { return None; }
        self.rows.iter().find(|r| r.receipt.action.seq == action.seq).map(|r| r.receipt)
    }
    pub fn take_unsent(&mut self) -> Vec<ActionReceipt> {
        self.rows.iter_mut().filter_map(|r| {
            if !r.needs_send { return None; }
            r.needs_send = false;
            Some(r.receipt)
        }).collect()
    }
    /// Every terminal result must be applied before a handover capture.
    pub fn is_quiescent(&self) -> bool { self.rows.is_empty() && self.acked == self.terminal }

    pub fn acknowledge(&mut self, ack: AppliedActionAck, authoritative_events: u32) -> Result<(), ResultError> {
        if ack.session != self.owner.session || ack.epoch != self.owner.epoch || ack.life != self.owner.life { return Err(ResultError::Namespace); }
        if ack.through == 0 || ack.through > self.terminal || ack.included_events > authoritative_events { return Err(ResultError::InvalidAck); }
        if ack.through <= self.acked {
            return if ack.included_events <= self.acked_events { Ok(()) } else { Err(ResultError::InvalidAck) };
        }
        if ack.included_events < self.acked_events { return Err(ResultError::InvalidAck); }
        if !self.rows.iter().any(|r| r.receipt.action.seq == ack.through) { return Err(ResultError::InvalidAck); }
        let required = self.rows.iter().filter(|r| r.receipt.action.seq <= ack.through).map(|r| r.receipt.included_events).max().unwrap_or(self.acked_events);
        if ack.included_events < required { return Err(ResultError::InvalidAck); }
        while self.rows.front().is_some_and(|r| r.receipt.action.seq <= ack.through) {
            let row = self.rows.pop_front().expect("retained terminal row");
            self.cell_count -= row.cells.len();
        }
        self.acked = ack.through;
        self.acked_events = ack.included_events;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_identity::ActionFate;

    fn header(seq: u64) -> ActionHeader {
        ActionHeader { session: [7; 16], epoch: 0, life: 1, seq, events_applied: 0 }
    }
    fn receipt(seq: u64, included_events: u32) -> ActionReceipt {
        ActionReceipt { action: header(seq), fate: ActionFate::Accepted, included_events }
    }
    fn ack(through: u64, included_events: u32) -> AppliedActionAck {
        AppliedActionAck { session: [7; 16], epoch: 0, life: 1, through, included_events }
    }
    fn append(results: &mut ActionResults, seq: u64, events: u32) {
        let prepared = results.prepare(receipt(seq, events), vec![[seq as i32, 0, 0]].into_boxed_slice()).unwrap();
        results.commit(prepared);
    }

    #[test]
    fn unacknowledged_capacity_refuses_before_effect_and_keeps_original_result() {
        let mut results = ActionResults::new(header(0));
        let mut effects = 0;
        for seq in 1..=MAX_ACTIONS as u64 {
            let prepared = results.prepare(receipt(seq, 3), Box::new([])).unwrap();
            effects += 1;
            results.commit(prepared);
        }
        let before = results.retained_bytes();
        if let Ok(prepared) = results.prepare(receipt(MAX_ACTIONS as u64 + 1, 3), Box::new([])) {
            effects += 1;
            results.commit(prepared);
        }
        assert_eq!(effects, MAX_ACTIONS, "full unacknowledged history admits no external effect");
        assert_eq!(results.retained_bytes(), before);
        assert_eq!(results.repeat(header(1)), Some(receipt(1, 3)));
        assert_eq!(results.rows.len(), MAX_ACTIONS, "no silent result eviction");
    }

    #[test]
    fn applied_ack_requires_all_removed_events_and_exact_retained_boundary() {
        let mut results = ActionResults::new(header(0));
        append(&mut results, 1, 10);
        append(&mut results, 2, 5);
        assert_eq!(results.acknowledge(ack(2, 5), 10), Err(ResultError::InvalidAck));
        assert_eq!(results.acknowledge(ack(3, 10), 10), Err(ResultError::InvalidAck));
        assert_eq!(results.acknowledge(ack(2, 11), 10), Err(ResultError::InvalidAck));
        let mut foreign = ack(2, 10); foreign.session = [8; 16];
        assert_eq!(results.acknowledge(foreign, 10), Err(ResultError::Namespace));
        assert_eq!(results.rows.len(), 2);
        results.acknowledge(ack(2, 10), 10).unwrap();
        assert!(results.rows.is_empty());
        assert_eq!(results.cell_count, 0);
        results.acknowledge(ack(1, 5), 10).unwrap();
        assert_eq!((results.acked, results.acked_events), (2, 10), "old metadata is inert");
        assert_eq!(results.acknowledge(ack(1, 11), 11), Err(ResultError::InvalidAck));
    }

    #[test]
    fn lost_receipt_repeat_is_exact_then_retired_repeat_has_no_invented_fate() {
        let mut results = ActionResults::new(header(0));
        let original = ActionReceipt { action: header(1), fate: ActionFate::Refused, included_events: 9 };
        let prepared = results.prepare(original, vec![[1, 2, 3]].into_boxed_slice()).unwrap();
        results.commit(prepared);
        assert_eq!(results.take_unsent(), vec![original]);
        assert_eq!(results.repeat(header(1)), Some(original));
        assert_eq!(results.take_unsent(), vec![original]);
        results.acknowledge(ack(1, 9), 9).unwrap();
        assert_eq!(results.repeat(header(1)), None, "caller must recover, never fabricate a new terminal fate");
        assert!(results.take_unsent().is_empty());
        assert_eq!(results.terminal, 1);
    }

    #[test]
    fn more_than_two_result_caps_remain_live_with_real_applied_metadata() {
        let mut results = ActionResults::new(header(0));
        let mut effects = 0;
        for seq in 1..=(MAX_ACTIONS as u64 * 2 + 17) {
            let prepared = results.prepare(receipt(seq, seq as u32), vec![[0, 0, 0]].into_boxed_slice()).unwrap();
            effects += 1;
            results.commit(prepared);
            assert_eq!(results.take_unsent(), vec![receipt(seq, seq as u32)]);
            results.acknowledge(ack(seq, seq as u32), seq as u32).unwrap();
            assert!(results.rows.is_empty());
            assert_eq!(results.cell_count, 0);
            assert!(results.retained_bytes() <= MAX_RETAINED_BYTES);
        }
        assert_eq!(effects, MAX_ACTIONS as u64 * 2 + 17);
        assert_eq!(results.terminal, effects);
    }
}

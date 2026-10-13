//! Immutable bounded server terminal results. Receipt acknowledgement is metadata.
use std::collections::VecDeque;
use crate::action_identity::ActionHeader;
use crate::action_journal::{MAX_ACTIONS, MAX_RETAINED_BYTES};
use crate::action_wire::{ActionReceipt, AppliedActionAck};

struct ResultRow { receipt: ActionReceipt, cells: Box<[[i32; 3]]>, carrier: Option<Box<[u8]>>, needs_send: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultError { Namespace, Sequence, Capacity, InvalidAck }

pub struct ActionResults {
    owner: ActionHeader,
    terminal: u64,
    acked: u64,
    acked_events: u32,
    rows: VecDeque<ResultRow>,
    cell_count: usize,
    carrier_bytes: usize,
}

pub struct PreparedResult { receipt: ActionReceipt, cells: Box<[[i32; 3]]>, carrier: Option<Box<[u8]>>, expected_terminal: u64 }

impl ActionResults {
    pub fn new(owner: ActionHeader) -> Self {
        Self { owner, terminal: 0, acked: 0, acked_events: 0, rows: VecDeque::new(), cell_count: 0, carrier_bytes: 0 }
    }
    fn owns(&self, action: ActionHeader) -> bool {
        action.session == self.owner.session && action.epoch == self.owner.epoch && action.life == self.owner.life
    }
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d recovery milestone: the server-known suffix footprint unions these cells into RebasePrepare"))]
    pub fn retained_cells(&self) -> impl Iterator<Item = &[i32; 3]> { self.rows.iter().flat_map(|r| r.cells.iter()) }
    pub fn retained_bytes(&self) -> usize { std::mem::size_of::<Self>() + self.rows.capacity() * std::mem::size_of::<ResultRow>() + self.cell_count * std::mem::size_of::<[i32; 3]>() + self.carrier_bytes }
    pub fn prepare(&mut self, receipt: ActionReceipt, cells: Box<[[i32; 3]]>) -> Result<PreparedResult, ResultError> {
        self.prepare_inner(receipt, cells, None)
    }
    /// An unpredicted pack keeps its exact bounded response with the existing
    /// terminal row. A duplicate never rereads live stock or repeats an effect.
    pub fn prepare_with_carrier(&mut self, receipt: ActionReceipt, cells: Box<[[i32; 3]]>, carrier: Box<[u8]>) -> Result<PreparedResult, ResultError> {
        if carrier.len() > crate::action_journal::MAX_PAGE_BYTES + 1 || carrier.first() != Some(&(crate::protocol::PacketType::PackResult as u8)) { return Err(ResultError::Capacity); }
        self.prepare_inner(receipt, cells, Some(carrier))
    }
    fn prepare_inner(&mut self, receipt: ActionReceipt, cells: Box<[[i32; 3]]>, carrier: Option<Box<[u8]>>) -> Result<PreparedResult, ResultError> {
        let added_carrier = carrier.as_ref().map_or(0, |bytes| bytes.len());
        let carrier_bytes = self.carrier_bytes.checked_add(added_carrier).ok_or(ResultError::Capacity)?;
        let cell_count = self.cell_count.checked_add(cells.len()).ok_or(ResultError::Capacity)?;
        if cell_count > crate::action_journal::MAX_CONTEXT_CELLS { return Err(ResultError::Capacity); }
        if !self.owns(receipt.action) { return Err(ResultError::Namespace); }
        if Some(receipt.action.seq) != self.terminal.checked_add(1) { return Err(ResultError::Sequence); }
        if self.rows.len() == MAX_ACTIONS { return Err(ResultError::Capacity); }
        let estimated = self.rows.capacity().max(self.rows.len() + 1).checked_mul(std::mem::size_of::<ResultRow>()).and_then(|n| n.checked_add(std::mem::size_of::<Self>())).ok_or(ResultError::Capacity)?;
        if estimated.checked_add(cell_count * std::mem::size_of::<[i32; 3]>()).and_then(|n| n.checked_add(carrier_bytes)).is_none_or(|n| n > MAX_RETAINED_BYTES) { return Err(ResultError::Capacity); }
        let capacity_before = self.rows.capacity();
        self.rows.try_reserve_exact(1).map_err(|_| ResultError::Capacity)?;
        if self.retained_bytes().checked_add(cells.len() * std::mem::size_of::<[i32; 3]>()).and_then(|n| n.checked_add(added_carrier)).is_none_or(|n| n > MAX_RETAINED_BYTES) {
            // Give back exactly what the reservation took: no capacity leaks.
            self.rows.shrink_to(capacity_before);
            return Err(ResultError::Capacity);
        }
        Ok(PreparedResult { receipt, cells, carrier, expected_terminal: self.terminal })
    }
    pub fn commit(&mut self, prepared: PreparedResult) {
        assert_eq!(self.terminal, prepared.expected_terminal, "terminal result reservation changed before commit");
        self.terminal = prepared.receipt.action.seq;
        self.cell_count += prepared.cells.len();
        self.carrier_bytes += prepared.carrier.as_ref().map_or(0, |bytes| bytes.len());
        self.rows.push_back(ResultRow { receipt: prepared.receipt, cells: prepared.cells, carrier: prepared.carrier, needs_send: true });
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
    #[cfg(test)]
    pub fn take_unsent(&mut self) -> Vec<ActionReceipt> {
        self.take_unsent_with_carriers().into_iter().map(|(receipt, _)| receipt).collect()
    }
    pub fn take_unsent_with_carriers(&mut self) -> Vec<(ActionReceipt, Option<Box<[u8]>>)> {
        self.rows.iter_mut().filter_map(|r| {
            if !r.needs_send { return None; }
            r.needs_send = false;
            Some((r.receipt, r.carrier.clone()))
        }).collect()
    }
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
            self.carrier_bytes -= row.carrier.as_ref().map_or(0, |bytes| bytes.len());
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
    fn d2c2_pack_result_duplicate_resends_immutable_carrier_and_ack_releases_its_bytes() {
        let mut results = ActionResults::new(header(0));
        let carrier = vec![crate::protocol::PacketType::PackResult as u8, 7, 8, 9].into_boxed_slice();
        let prepared = results.prepare_with_carrier(receipt(1, 4), Box::new([]), carrier.clone()).unwrap();
        results.commit(prepared);
        let bytes_before_ack = results.retained_bytes();
        assert_eq!(results.take_unsent_with_carriers(), vec![(receipt(1, 4), Some(carrier.clone()))]);
        assert_eq!(results.repeat(header(1)), Some(receipt(1, 4)));
        assert_eq!(results.take_unsent_with_carriers(), vec![(receipt(1, 4), Some(carrier))], "duplicate does not rebuild a view from current stock");
        assert_eq!(results.acknowledge(ack(1, 3), 4), Err(ResultError::InvalidAck));
        assert_eq!(results.retained_bytes(), bytes_before_ack);
        results.acknowledge(ack(1, 4), 4).unwrap();
        assert_eq!(results.carrier_bytes, 0);
        assert!(results.rows.is_empty());
    }

    #[test]
    fn d2c2_pack_result_carrier_limit_refuses_before_any_terminal_commit() {
        let mut results = ActionResults::new(header(0));
        let before = results.retained_bytes();
        let mut carrier = vec![0; crate::action_journal::MAX_PAGE_BYTES + 2];
        carrier[0] = crate::protocol::PacketType::PackResult as u8;
        assert!(matches!(results.prepare_with_carrier(receipt(1, 4), Box::new([]), carrier.into_boxed_slice()), Err(ResultError::Capacity)));
        assert_eq!(results.retained_bytes(), before);
        assert_eq!(results.terminal, 0);
        assert!(results.rows.is_empty());
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

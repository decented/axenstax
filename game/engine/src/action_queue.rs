//! Bounded unified server action queue and conflicting-duplicate detection.

use std::collections::VecDeque;
use sha2::{Digest, Sha256};
use crate::action_identity::{ActionFrontier, ActionHeader};
use crate::action_journal::{MAX_ACTIONS, MAX_INTENT_BYTES, MAX_RETAINED_BYTES};

#[derive(Debug)]
pub struct QueuedAction {
    pub header: ActionHeader,
    pub payload: Box<[u8]>,
    hash: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueResult { Queued, QueuedDuplicate, TerminalDuplicate }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError { Namespace, Paused, Gap, Conflict, Count, Bytes, Exhausted }

pub struct ActionQueue {
    entries: VecDeque<QueuedAction>,
    bytes: usize,
}

impl Default for ActionQueue {
    fn default() -> Self {
        let entries = VecDeque::with_capacity(MAX_ACTIONS);
        let bytes = std::mem::size_of::<Self>() + entries.capacity() * std::mem::size_of::<QueuedAction>();
        Self { entries, bytes }
    }
}

impl ActionQueue {
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d recovery milestone (RebasePrepare, Describe, capture, applied fence and replay)"))]
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d recovery milestone (RebasePrepare, Describe, capture, applied fence and replay)"))]
    pub fn retained_bytes(&self) -> usize { self.bytes }
    pub fn front(&self) -> Option<&QueuedAction> { self.entries.front() }

    pub fn push(&mut self, frontier: &ActionFrontier, header: ActionHeader, payload: Box<[u8]>) -> Result<QueueResult, QueueError> {
        if header != frontier.header(header.seq, header.events_applied) { return Err(QueueError::Namespace); }
        if frontier.recovery().is_some() { return Err(QueueError::Paused); }
        if payload.len() > MAX_INTENT_BYTES { return Err(QueueError::Bytes); }
        if header.seq == 0 { return Err(QueueError::Gap); }
        if header.seq <= frontier.through() { return Ok(QueueResult::TerminalDuplicate); }
        let hash: [u8; 32] = Sha256::digest(&payload).into();
        if let Some(old) = self.entries.iter().find(|a| a.header.seq == header.seq) {
            return if old.header == header && old.hash == hash { Ok(QueueResult::QueuedDuplicate) } else { Err(QueueError::Conflict) };
        }
        let latest = self.entries.back().map_or(frontier.through(), |a| a.header.seq);
        let next = latest.checked_add(1).ok_or(QueueError::Exhausted)?;
        if header.seq != next { return Err(QueueError::Gap); }
        if self.entries.len() == MAX_ACTIONS { return Err(QueueError::Count); }
        let bytes = self.bytes.checked_add(payload.len()).ok_or(QueueError::Bytes)?;
        if bytes > MAX_RETAINED_BYTES { return Err(QueueError::Bytes); }
        self.entries.push_back(QueuedAction { header, payload, hash });
        self.bytes = bytes;
        Ok(QueueResult::Queued)
    }

    /// Complete mutation/event registration first; pop only the exact terminal
    /// head. A partial-budget action remains at the front until it commits.
    pub fn pop_terminal(&mut self, frontier: &ActionFrontier) -> Option<QueuedAction> {
        if self.entries.front().is_none_or(|a| a.header.seq != frontier.through()) { return None; }
        let action = self.entries.pop_front()?;
        self.bytes -= action.payload.len();
        Some(action)
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "C3d recovery milestone (RebasePrepare, Describe, capture, applied fence and replay)"))]
    pub fn supersede(&mut self) {
        self.entries.clear();
        self.bytes = std::mem::size_of::<Self>() + self.entries.capacity() * std::mem::size_of::<QueuedAction>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_identity::ActionFate;
    #[test]
    fn c3d_queued_duplicates_match_payload_and_terminal_duplicates_mutate_nothing() {
        let mut frontier = ActionFrontier::new([6; 16], 1);
        let mut queue = ActionQueue::default();
        let one = frontier.header(1, 0);
        assert_eq!(queue.push(&frontier, one, Box::new([1, 2])), Ok(QueueResult::Queued));
        assert_eq!(queue.push(&frontier, one, Box::new([1, 2])), Ok(QueueResult::QueuedDuplicate));
        assert_eq!(queue.push(&frontier, one, Box::new([1, 3])), Err(QueueError::Conflict));
        assert!(queue.pop_terminal(&frontier).is_none());
        frontier.terminal(one, ActionFate::Accepted).unwrap();
        assert!(queue.pop_terminal(&frontier).is_some());
        assert_eq!(queue.push(&frontier, one, Box::new([1, 3])), Ok(QueueResult::TerminalDuplicate));
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.push(&frontier, frontier.header(3, 0), Box::new([])), Err(QueueError::Gap));
        assert_eq!(queue.push(&frontier, frontier.header(2, 0), Box::new([])), Ok(QueueResult::Queued));
        frontier.prepare().unwrap();
        assert_eq!(queue.push(&frontier, frontier.header(3, 0), Box::new([])), Err(QueueError::Paused));
        queue.supersede();
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn c3d_server_queue_count_and_byte_limits_preserve_existing_history() {
        let frontier = ActionFrontier::new([6; 16], 1);
        let mut count = ActionQueue::default();
        for seq in 1..=MAX_ACTIONS { count.push(&frontier, frontier.header(seq as u64, 0), Box::new([])).unwrap(); }
        assert_eq!(count.push(&frontier, frontier.header(MAX_ACTIONS as u64 + 1, 0), Box::new([])), Err(QueueError::Count));
        assert_eq!(count.len(), MAX_ACTIONS);
        let mut bytes = ActionQueue::default();
        let mut seq = 1;
        while bytes.push(&frontier, frontier.header(seq, 0), vec![0; MAX_INTENT_BYTES].into_boxed_slice()).is_ok() { seq += 1; }
        assert!(bytes.len() < MAX_ACTIONS);
        assert!(bytes.retained_bytes() <= MAX_RETAINED_BYTES);
        assert_eq!(bytes.len() as u64, seq - 1);
        assert_eq!(bytes.front().unwrap().header.seq, 1);
    }
}

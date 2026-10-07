//! FU3 (FU1 verify N3) — a client's block edits waiting past the server's
//! per-tick edit budget (`hosted_server::MAX_BLOCK_CHANGES_PER_TICK`).
//!
//! The server processes at most that many of a client's edits a tick. The
//! rest used to be refused and sent back, so an honest client lost every edit
//! past the fourth of a tick: a campfire break or light (one edit plus a
//! six-cell smoke pillar, before the server derived the smoke), a keg blast,
//! or the inputs a catch-up reads in one tick after a stall. Now they WAIT
//! here, per client, in arrival order, and go first on the next tick.
//!
//! **Kept with their input.** An edit is queued with the rest of its input's
//! edits, that input's `mined` tags and the hand its input reported, so it is
//! judged exactly as it would have been in its own tick: a tag still pairs
//! with its own edit (the oldest unused tag of its cell, `HostedServer::
//! classify_joiner_edit`), and a placement is classified by what was in hand
//! when it was made, not by a later input's hand. Reach and plot rules are
//! checked when the edit is processed (the body the server holds then).
//!
//! **A hard cap no honest client reaches** ([`MAX_DEFERRED_EDITS`]): past it
//! an edit is refused and sent back, as before, and the server logs a warning.

use std::collections::VecDeque;

use crate::protocol::{BlockChange, MinedBlock};

/// Most edits one client may have waiting. 16,384 — the client's own bound on
/// edits it holds back unsent (`remote_client::INPUT_CARRY_OVER_MAX_CHANGES`),
/// about 256 KiB of `BlockChange`s.
///
/// **The honest maximum.** Edits only pile up here when they arrive faster
/// than the budget drains them (4 a tick), which takes bunched inputs: a
/// catch-up after a stall reads up to `hosted_server::CATCH_UP_PACKETS_PER_TICK`
/// inputs a tick. Per client tick an honest player makes at most one break
/// (survival's floor is one tick a break, `crafting::break_time_ticks`;
/// creative's cooldown is `player_slot::BLOCK_BREAK_COOLDOWN_TICKS` = 5) and
/// at most one placement (a held button repeats every 10 ticks, a fresh click
/// is immediate), each at most two cells (a door or a bed): four edits a tick,
/// eighty a second, at the absolute ceiling — which nobody sustains (building
/// runs at a few a second). A T-second stall therefore queues at most 80·T
/// edits: the cap is a 3½-minute stall at that ceiling, hours of real
/// building, and beyond the half-hour the inbound byte bound
/// (`transport::MAX_INBOUND_BYTES`) admits only at a realistic rate. The
/// largest single action is a keg blast: every cell within
/// `explosion::BLAST_RADIUS` (4) of it, about 260; the cap holds sixty.
pub const MAX_DEFERRED_EDITS: usize = 16_384;

/// The edits of one input that have not been processed yet, with what they
/// are judged by.
pub struct EditGroup {
    /// In the order the client made them.
    pub edits: VecDeque<BlockChange>,
    /// The input's `mined` tags (its first `protocol::MAX_MINED_PER_INPUT`),
    /// each used by at most one edit; `None` once used.
    pub tags: Vec<Option<MinedBlock>>,
    /// What the input said was in hand (`InputPacket.held_kind` / `held_id`).
    pub held_kind: u8,
    pub held_id: u16,
}

impl EditGroup {
    /// Use up the tag of `bc` when `bc` empties its cell and is refused: the
    /// tag the edit would have taken (the oldest unused one of its cell — the
    /// edits of that cell before it took theirs), so it can't pair with a
    /// later edit of the cell that a later tick accepts. A refused edit's tag
    /// yields nothing and is gone.
    pub fn spend_tag_of_refused(&mut self, bc: &BlockChange) {
        if bc.new_block != crate::block::AIR {
            return;
        }
        let cell = (bc.x, bc.y, bc.z);
        if let Some(tag) = self.tags.iter_mut().find(|t| t.is_some_and(|m| (m.x, m.y, m.z) == cell)) {
            *tag = None;
        }
    }
}

/// One client's waiting edits, oldest first.
#[derive(Default)]
pub struct EditQueue {
    groups: VecDeque<EditGroup>,
    /// Edits waiting, across every group.
    len: usize,
    /// Whether the cap's warning was logged since the queue was last empty.
    cap_warned: bool,
}

impl EditQueue {
    /// Edits waiting.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Queue `group`'s edits behind everything waiting, as far as
    /// [`MAX_DEFERRED_EDITS`] allows. Returns the edits past the cap, newest
    /// last, which the caller refuses (their emptying edits' tags spent).
    pub fn push_back(&mut self, mut group: EditGroup) -> Vec<BlockChange> {
        let room = MAX_DEFERRED_EDITS.saturating_sub(self.len);
        let refused: Vec<BlockChange> = if group.edits.len() > room {
            group.edits.split_off(room).into()
        } else {
            Vec::new()
        };
        for bc in &refused {
            group.spend_tag_of_refused(bc);
        }
        if !group.edits.is_empty() {
            self.len += group.edits.len();
            self.groups.push_back(group);
        }
        refused
    }

    /// Take the oldest group, to process its edits.
    pub fn pop_front(&mut self) -> Option<EditGroup> {
        let group = self.groups.pop_front()?;
        self.len -= group.edits.len();
        if self.len == 0 {
            self.cap_warned = false;
        }
        Some(group)
    }

    /// Whether to log the cap's warning now: once until the queue next
    /// empties, so a client pinned at the cap can't flood the log.
    pub fn take_cap_warning(&mut self) -> bool {
        !std::mem::replace(&mut self.cap_warned, true)
    }

    /// Put back a group taken with [`Self::pop_front`] whose edits the budget
    /// did not reach, ahead of everything else.
    pub fn push_front(&mut self, group: EditGroup) {
        if !group.edits.is_empty() {
            self.len += group.edits.len();
            self.groups.push_front(group);
        }
    }

    /// Forget everything waiting (the connection ended).
    pub fn clear(&mut self) {
        self.groups.clear();
        self.len = 0;
        self.cap_warned = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::WireItem;

    fn edit(x: i32, new_block: crate::block::BlockId) -> BlockChange {
        BlockChange { x, y: 70, z: 0, new_block, meta: 0 }
    }

    fn tag(x: i32) -> Option<MinedBlock> {
        Some(MinedBlock { x, y: 70, z: 0, tool: WireItem::None })
    }

    fn group(edits: &[BlockChange], tags: Vec<Option<MinedBlock>>) -> EditGroup {
        EditGroup { edits: edits.iter().cloned().collect(), tags, held_kind: 0, held_id: 0 }
    }

    #[test]
    fn groups_wait_in_arrival_order_and_a_part_put_back_goes_first() {
        let mut q = EditQueue::default();
        assert!(q.push_back(group(&[edit(1, 1), edit(2, 1)], vec![])).is_empty());
        assert!(q.push_back(group(&[edit(3, 1)], vec![])).is_empty());
        assert!(q.push_back(group(&[], vec![tag(9)])).is_empty(), "an empty group is not kept");
        assert_eq!(q.len(), 3);
        let mut first = q.pop_front().expect("oldest");
        assert_eq!(first.edits.pop_front().map(|e| e.x), Some(1));
        q.push_front(first);
        assert_eq!(q.len(), 2);
        let order: Vec<i32> = std::iter::from_fn(|| q.pop_front()).flat_map(|g| g.edits).map(|e| e.x).collect();
        assert_eq!(order, vec![2, 3], "the rest of the first input, then the next");
        assert!(q.is_empty());
    }

    #[test]
    fn past_the_cap_the_newest_edits_are_refused_and_their_tags_spent() {
        let mut q = EditQueue::default();
        let filler: Vec<BlockChange> = (0..MAX_DEFERRED_EDITS as i32 - 1).map(|x| edit(x, 1)).collect();
        assert!(q.push_back(group(&filler, vec![])).is_empty());
        // One more fits; the two behind it (a mine of x = -5 among them) don't.
        let over = [edit(-1, 1), edit(-5, crate::block::AIR), edit(-6, 1)];
        let refused = q.push_back(group(&over, vec![tag(-5)]));
        assert_eq!(refused.iter().map(|e| e.x).collect::<Vec<_>>(), vec![-5, -6]);
        assert_eq!(q.len(), MAX_DEFERRED_EDITS, "full, never past it");
        let _ = q.pop_front();
        let kept = q.pop_front().expect("the part that fitted");
        assert_eq!(kept.edits.len(), 1);
        assert_eq!(kept.tags, vec![None], "the refused mine's tag is spent");
        q.clear();
        assert!(q.is_empty() && q.len() == 0);
    }

    #[test]
    fn a_refused_placement_keeps_the_tags_and_a_refused_mine_spends_only_its_cells_oldest() {
        let mut g = group(&[], vec![tag(1), tag(2), tag(1)]);
        g.spend_tag_of_refused(&edit(1, crate::block::STONE));
        assert_eq!(g.tags, vec![tag(1), tag(2), tag(1)], "a placement takes no tag");
        g.spend_tag_of_refused(&edit(1, crate::block::AIR));
        assert_eq!(g.tags, vec![None, tag(2), tag(1)], "the oldest of its cell");
    }
}

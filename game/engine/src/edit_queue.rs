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
//! edits, the hand its input reported and the life it was made in, so it is
//! judged exactly as it would have been in its own tick: a placement is
//! classified by what was in hand when it was made, not by a later input's
//! hand, and an edit made before its joiner died and respawned is sent back,
//! not applied (FU4a, FU3 verify L1). Reach and plot rules are checked when
//! the edit is processed (the body the server holds then).
//!
//! **Each edit owns its tag (FU4a, FU3 verify L2).** The input's `mined` tags
//! are paired with its edits when the input is read ([`EditGroup::new`]), and
//! from then on a tag goes wherever its edit goes: processed with it, refused
//! with it, dropped with it at the cap. A refused crop harvest takes its tag
//! with it, so a later edit of its cell can't yield the crop.
//!
//! **A hard cap no honest client reaches** ([`MAX_DEFERRED_EDITS`]): past it
//! an edit is dropped, with nothing sent back (FU4a, FU3 verify M2), and the
//! server logs one warning until the queue next empties.

use std::collections::VecDeque;

use crate::block::BlockId;
use crate::protocol::{BlockChange, MinedBlock};

/// Most edits one client may have waiting. 16,384 — the client's own bound on
/// edits it holds back unsent (`remote_client::INPUT_CARRY_OVER_MAX_CHANGES`).
///
/// **Memory (FU4a, FU3 verify M1).** A capped input keeps only the edits that
/// fit, in a buffer shrunk to them (FU3 kept each capped input's whole
/// deserialized buffer, about 69 KB, for the four edits that fitted: a modified
/// joiner sending one full input a tick at the cap pinned about 280 MB). The
/// worst case is now the cap's edits (16 B each), as many group headers (a
/// group holds at least one edit: 72 B each), a tag for each edit at most
/// (24 B), and the front group's processed slack (one input's edits at most,
/// about 4,370): **under 2 MB of capacity per client** (about 1.9 MB), plus
/// the allocator's own overhead of a few dozen bytes for each of a group's
/// one or two allocations. FU3's "about 256 KiB" counted the edits alone.
/// Pinned by
/// `tests::the_queue_at_the_cap_stays_within_its_memory_bound`.
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
    edits: VecDeque<BlockChange>,
    /// FU4a (L2) — the input's `mined` tags, each paired with its own edit
    /// (by the edit's place in the input, 0 its first), newest place first so
    /// the front edit's tag is the last. At most
    /// `protocol::MAX_MINED_PER_INPUT`, each with a distinct waiting edit.
    tags: Vec<(u32, MinedBlock)>,
    /// The place in the input of the front of `edits`.
    next: u32,
    /// What the input said was in hand (`InputPacket.held_kind` / `held_id`).
    pub held_kind: u8,
    pub held_id: u16,
    /// FU4a (L1) — the joiner's life when the input was read
    /// (`server::ServerPlayer::respawns`): a group from an earlier life is
    /// sent back, not applied.
    pub life: u32,
}

impl EditGroup {
    /// One input's edits, oldest first: the first `keep` of them (the rest
    /// are dropped, their count returned — before their tags are paired, so
    /// a flood at the cap costs nothing more), each paired with its own tag
    /// from `tags` (the input's first `protocol::MAX_MINED_PER_INPUT`, its
    /// DoS guard; [`pair_tags`]). `block_at` reads the world, for the first
    /// edit of a tagged cell.
    pub fn new(
        mut edits: Vec<BlockChange>,
        keep: usize,
        tags: &[MinedBlock],
        (held_kind, held_id): (u8, u16),
        life: u32,
        block_at: impl FnMut(i32, i32, i32) -> BlockId,
    ) -> (Self, usize) {
        let dropped = edits.len().saturating_sub(keep);
        if dropped > 0 {
            edits.truncate(keep);
            edits.shrink_to_fit();
        }
        let tags = &tags[..tags.len().min(crate::protocol::MAX_MINED_PER_INPUT)];
        let tags = pair_tags(&edits, tags, block_at);
        (Self { edits: edits.into(), tags, next: 0, held_kind, held_id, life }, dropped)
    }

    /// Edits still waiting.
    pub fn len(&self) -> usize {
        self.edits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Take the next edit, with its own tag.
    pub fn pop_front(&mut self) -> Option<(BlockChange, Option<MinedBlock>)> {
        let bc = self.edits.pop_front()?;
        let tag = match self.tags.last() {
            Some(&(at, _)) if at == self.next => self.tags.pop().map(|(_, m)| m),
            _ => None,
        };
        self.next += 1;
        Some((bc, tag))
    }

    /// Take every edit left, their tags gone with them (a dead joiner's or
    /// an earlier life's: sent back, never applied).
    pub fn take_edits(&mut self) -> VecDeque<BlockChange> {
        self.tags = Vec::new();
        std::mem::take(&mut self.edits)
    }

    /// FU4a (M1) — keep the first `keep` edits waiting, with their tags, and
    /// free the room the rest held. Returns how many were dropped.
    fn truncate(&mut self, keep: usize) -> usize {
        let dropped = self.edits.len().saturating_sub(keep);
        if dropped > 0 {
            self.edits.truncate(keep);
            self.edits.shrink_to_fit();
            let end = self.next + keep as u32;
            self.tags.retain(|&(at, _)| at < end);
            self.tags.shrink_to_fit();
        }
        dropped
    }
}

/// FU4a (FU3 verify L2) — pair each tag with its own edit, as the input is
/// read: walking the edits in order, an edit of a cell with a tag still
/// unpaired takes the cell's next tag if it is one a tag is sent for
/// ([`takes_tag`]: an emptying of the cell or a crop harvest, judged from what
/// the cell holds by then — the input's own earlier edits of it, else the
/// world). That is the client's pairing for every input it sends (it tags the
/// break or harvest its break arm made, and holds back a tagged edit behind an
/// untagged one of its cell, `remote_client::hold_back_for_tags`), and a fill
/// never takes a tag, so a place-then-break of one cell pairs the tag with the
/// break. A tag no edit takes yields nothing and is dropped. Returns the pairs
/// newest place first.
fn pair_tags(
    edits: &[BlockChange],
    tags: &[MinedBlock],
    mut block_at: impl FnMut(i32, i32, i32) -> BlockId,
) -> Vec<(u32, MinedBlock)> {
    let mut unpaired: Vec<MinedBlock> = tags.to_vec();
    let mut paired = Vec::with_capacity(unpaired.len());
    // What each tagged cell holds as the input's edits run.
    let mut holds: Vec<((i32, i32, i32), BlockId)> = Vec::new();
    for (at, bc) in edits.iter().enumerate() {
        if unpaired.is_empty() {
            break;
        }
        let cell = (bc.x, bc.y, bc.z);
        let Some(k) = unpaired.iter().position(|m| (m.x, m.y, m.z) == cell) else {
            continue;
        };
        let old = match holds.iter_mut().find(|(c, _)| *c == cell) {
            Some((_, held)) => std::mem::replace(held, bc.new_block),
            None => {
                holds.push((cell, bc.new_block));
                block_at(cell.0, cell.1, cell.2)
            }
        };
        if takes_tag(old, bc.new_block) {
            paired.push((at as u32, unpaired.remove(k)));
        }
    }
    paired.reverse();
    paired.shrink_to_fit();
    paired
}

/// Whether an edit `old → new` of a tagged cell is one a tag is sent for: an
/// emptying of the cell (a break, or dug lava or fire, which yield nothing) or
/// a crop harvest (its replacement left behind). The only edits
/// `joiner_inventory::classify` calls a `Break` with a tag are among these.
fn takes_tag(old: BlockId, new: BlockId) -> bool {
    new == crate::block::AIR || (crate::growth::is_crop(old) && new != old)
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

    /// How many more edits fit under [`MAX_DEFERRED_EDITS`].
    pub fn room(&self) -> usize {
        MAX_DEFERRED_EDITS.saturating_sub(self.len)
    }

    /// FU4a (M2) — at the cap: what this client sends is dropped.
    pub fn is_full(&self) -> bool {
        self.room() == 0
    }

    /// Queue `group`'s edits behind everything waiting, as far as
    /// [`MAX_DEFERRED_EDITS`] allows. Returns how many were past the cap:
    /// dropped, with their tags (FU4a, M2: nothing is sent back for them).
    pub fn push_back(&mut self, mut group: EditGroup) -> usize {
        let dropped = group.truncate(self.room());
        if !group.is_empty() {
            self.len += group.len();
            self.groups.push_back(group);
        }
        dropped
    }

    /// Take the oldest group, to process its edits.
    pub fn pop_front(&mut self) -> Option<EditGroup> {
        let group = self.groups.pop_front()?;
        self.len -= group.len();
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
        if !group.is_empty() {
            self.len += group.len();
            self.groups.push_front(group);
        }
    }

    /// Forget everything waiting (the connection ended), and free its room.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Test-only: the bytes the queue holds allocated (every collection's
    /// capacity), the figure [`MAX_DEFERRED_EDITS`]'s memory bound is about.
    #[cfg(test)]
    pub fn allocated_bytes(&self) -> usize {
        use std::mem::size_of;
        self.groups.capacity() * size_of::<EditGroup>()
            + self
                .groups
                .iter()
                .map(|g| {
                    g.edits.capacity() * size_of::<BlockChange>()
                        + g.tags.capacity() * size_of::<(u32, MinedBlock)>()
                })
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{AIR, STONE, TILLED_SOIL, WHEAT_STAGE_3};
    use crate::protocol::WireItem;

    fn edit(x: i32, new_block: BlockId) -> BlockChange {
        BlockChange { x, y: 70, z: 0, new_block, meta: 0 }
    }

    fn tag(x: i32) -> MinedBlock {
        MinedBlock { x, y: 70, z: 0, tool: WireItem::None }
    }

    fn tool_tag(x: i32, durability: u16) -> MinedBlock {
        MinedBlock { x, y: 70, z: 0, tool: WireItem::Tool { tool_type: 0, material: 0, durability } }
    }

    /// A group of `edits` read against a world of stone.
    fn group(edits: &[BlockChange], tags: &[MinedBlock]) -> EditGroup {
        group_in(edits, tags, |_, _, _| STONE)
    }

    fn group_in(edits: &[BlockChange], tags: &[MinedBlock], world: impl FnMut(i32, i32, i32) -> BlockId) -> EditGroup {
        EditGroup::new(edits.to_vec(), usize::MAX, tags, (0, 0), 0, world).0
    }

    /// Each edit with its tag, in order.
    fn drain(mut g: EditGroup) -> Vec<(i32, Option<MinedBlock>)> {
        std::iter::from_fn(|| g.pop_front()).map(|(bc, t)| (bc.x, t)).collect()
    }

    #[test]
    fn groups_wait_in_arrival_order_and_a_part_put_back_goes_first() {
        let mut q = EditQueue::default();
        assert_eq!(q.push_back(group(&[edit(1, 1), edit(2, 1)], &[])), 0);
        assert_eq!(q.push_back(group(&[edit(3, 1)], &[])), 0);
        assert_eq!(q.push_back(group(&[], &[tag(9)])), 0, "an empty group is not kept");
        assert_eq!(q.len(), 3);
        let mut first = q.pop_front().expect("oldest");
        assert_eq!(first.pop_front().map(|(e, _)| e.x), Some(1));
        q.push_front(first);
        assert_eq!(q.len(), 2);
        let order: Vec<i32> =
            std::iter::from_fn(|| q.pop_front()).flat_map(drain).map(|(x, _)| x).collect();
        assert_eq!(order, vec![2, 3], "the rest of the first input, then the next");
        assert!(q.is_empty());
    }

    /// FU4a (M2) — past the cap the newest edits are dropped (counted, never
    /// returned to be sent back), and their tags with them; the kept part's
    /// tags still pair with their own edits.
    #[test]
    fn past_the_cap_the_newest_edits_are_dropped_with_their_tags() {
        let mut q = EditQueue::default();
        let filler: Vec<BlockChange> = (0..MAX_DEFERRED_EDITS as i32 - 2).map(|x| edit(x, 1)).collect();
        assert_eq!(q.push_back(group(&filler, &[])), 0);
        // Two fit (a mine of x = -5 among them); the two behind (a mine of
        // x = -6 among them) don't.
        let over = [edit(-5, AIR), edit(-1, 1), edit(-6, AIR), edit(-7, 1)];
        assert_eq!(q.push_back(group(&over, &[tag(-5), tag(-6)])), 2, "two dropped");
        assert_eq!(q.len(), MAX_DEFERRED_EDITS, "full, never past it");
        assert!(q.is_full() && q.room() == 0);
        assert_eq!(q.push_back(group(&[edit(-8, 1)], &[])), 1, "at the cap everything is dropped");
        let _ = q.pop_front();
        let kept = q.pop_front().expect("the part that fitted");
        assert_eq!(kept.tags.len(), 1, "the dropped mine's tag went with it");
        assert_eq!(drain(kept), vec![(-5, Some(tag(-5))), (-1, None)]);
        q.clear();
        assert!(q.is_empty() && q.len() == 0 && !q.is_full());
    }

    /// FU4a (FU3 verify L2) — each tag is paired with its own edit when the
    /// input is read, and goes where that edit goes.
    #[test]
    fn each_tag_pairs_with_its_own_edit_when_the_input_is_read() {
        // A mine, then a refill and a second mine of the same cell, two tags:
        // each mine takes its own (the refill, a fill, takes none).
        let g = group(&[edit(1, AIR), edit(1, STONE), edit(1, AIR)], &[tool_tag(1, 7), tool_tag(1, 9)]);
        assert_eq!(drain(g), vec![(1, Some(tool_tag(1, 7))), (1, None), (1, Some(tool_tag(1, 9)))]);
        // A place-then-break of an empty cell, one tag: the break's.
        let g = group_in(&[edit(2, STONE), edit(2, AIR)], &[tag(2)], |_, _, _| AIR);
        assert_eq!(drain(g), vec![(2, None), (2, Some(tag(2)))]);
        // A mine, then an untagged Eraser of a refill: the tag is the mine's.
        let g = group(&[edit(3, AIR), edit(3, STONE), edit(3, AIR)], &[tag(3)]);
        assert_eq!(drain(g), vec![(3, Some(tag(3))), (3, None), (3, None)]);
        // A crop harvest, then a later emptying of its cell (carried over
        // from a later tick): the tag is the harvest's, not the emptying's.
        let g = group_in(&[edit(4, TILLED_SOIL), edit(4, AIR)], &[tag(4)], |_, _, _| WHEAT_STAGE_3);
        assert_eq!(drain(g), vec![(4, Some(tag(4))), (4, None)]);
        // A tag on a plain fill, or on a cell no edit touches: nobody's.
        let g = group_in(&[edit(5, STONE), edit(6, AIR)], &[tag(5), tag(7)], |_, _, _| AIR);
        assert!(g.tags.is_empty());
        assert_eq!(drain(g), vec![(5, None), (6, None)]);
        // Only the input's first MAX_MINED_PER_INPUT tags are read.
        let many: Vec<BlockChange> = (0..20).map(|x| edit(x, AIR)).collect();
        let tags: Vec<MinedBlock> = (0..20).map(tag).collect();
        let g = group(&many, &tags);
        assert_eq!(g.tags.len(), crate::protocol::MAX_MINED_PER_INPUT);
    }

    /// FU4a (FU3 verify M1) — the M1 scenario: the queue at the cap, then
    /// one full input a tick while the budget drains four edits a tick. FU3
    /// kept each input's whole deserialized buffer for the four edits that
    /// fitted (about 280 MB after 4,096 ticks); now the queue's allocated
    /// capacity stays within the bound [`MAX_DEFERRED_EDITS`] states — on
    /// both ways an input is capped (truncated as it is read, or by
    /// `push_back`).
    #[test]
    fn the_queue_at_the_cap_stays_within_its_memory_bound() {
        use std::mem::size_of;
        // The most edits one input carries: its packet is at most
        // MAX_WIRE_PACKET_LEN bytes, each edit at least its bincode size.
        let per_edit = bincode::serialized_size(&edit(0, 1)).expect("sizes") as usize;
        let max_input = crate::protocol::MAX_WIRE_PACKET_LEN / per_edit;
        let bound = MAX_DEFERRED_EDITS * size_of::<EditGroup>()
            + (MAX_DEFERRED_EDITS + max_input) * size_of::<BlockChange>()
            + (MAX_DEFERRED_EDITS + crate::protocol::MAX_MINED_PER_INPUT) * size_of::<(u32, MinedBlock)>();
        // A full input, its buffer exactly its length (as bincode leaves it),
        // with its tags on cells it mines.
        let full_input = |t: i32| -> (Vec<BlockChange>, Vec<MinedBlock>) {
            let mut v = Vec::with_capacity(max_input);
            v.extend((0..max_input as i32).map(|k| edit(t * 10_000 + k, AIR)));
            let tags = (0..crate::protocol::MAX_MINED_PER_INPUT as i32).map(|k| tag(t * 10_000 + k)).collect();
            (v, tags)
        };
        for truncated_as_read in [true, false] {
            let mut q = EditQueue::default();
            let mut t = 0;
            while !q.is_full() {
                let (v, tags) = full_input(t);
                q.push_back(EditGroup::new(v, usize::MAX, &tags, (0, 0), 0, |_, _, _| STONE).0);
                t += 1;
            }
            let mut peak = 0;
            // Past the point where the original full groups have all drained
            // and the queue is nothing but the small kept parts.
            for _ in 0..MAX_DEFERRED_EDITS / 4 + 200 {
                // The budget drains four edits from the front
                // (`HostedServer::process_waiting_edits`).
                let mut drained = 0;
                while drained < 4 {
                    let mut front = q.pop_front().expect("waiting");
                    while drained < 4 && front.pop_front().is_some() {
                        drained += 1;
                    }
                    q.push_front(front);
                }
                let (v, tags) = full_input(t);
                t += 1;
                let keep = if truncated_as_read { q.room() } else { usize::MAX };
                let (g, dropped) = EditGroup::new(v, keep, &tags, (0, 0), 0, |_, _, _| STONE);
                let dropped = dropped + q.push_back(g);
                assert_eq!(dropped + q.len(), MAX_DEFERRED_EDITS - 4 + max_input, "only what fits is kept");
                peak = peak.max(q.allocated_bytes());
                assert!(peak <= bound, "{peak} bytes allocated, past the bound of {bound}");
            }
            println!(
                "edit queue at the cap: peak {peak} bytes allocated (bound {bound}; {} B a group, \
                 {} B an edit, {} B a tag, {max_input} edits an input)",
                size_of::<EditGroup>(),
                size_of::<BlockChange>(),
                size_of::<(u32, MinedBlock)>(),
            );
            assert!(bound < 2_000_000, "the bound the docs state: under 2 MB ({bound})");
        }
    }
}

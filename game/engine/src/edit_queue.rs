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
//! edits, the hand and hotbar slot it was made with and the life it was made
//! in, so it is judged exactly as it would have been in its own tick: a
//! placement is classified by what was in hand when it was made, and charged
//! to the slot it was made at (C3a-2b), not a later input's hand or slot, and
//! an edit made before its joiner died and respawned is sent back,
//! not applied (FU4a, FU3 verify L1). Reach and plot rules are checked when
//! the edit is processed (the body the server holds then). C3a-fix-1 (C-M1)
//! — the hand and slot are each EDIT's own (`InputPacket.edit_hands`), so a
//! scroll after a placement within one send window charges the placement to
//! the slot it came from; an edit the list doesn't cover takes its input's.
//! The group also keeps the window events its client had applied when it
//! made the edits (`InputPacket.events_applied`): the server applies them
//! just before it processes the group (`window_events`).
//!
//! **Each edit owns its tag (FU4a, FU3 verify L2).** The input's `mined` tags
//! are paired with its edits when the input is read ([`EditGroup::new`]), and
//! from then on a tag goes wherever its edit goes: processed with it, refused
//! with it, dropped with it at the cap. A refused crop harvest takes its tag
//! with it, so a later edit of its cell can't yield the crop. C3c-1 — so do
//! the input's use tags (`InputPacket.use_tags`), each paired with the LAST
//! edit of its cell in the input (the client sends no edit of a cell behind
//! a use of it in one input), before the mined tags are paired among the
//! rest; one input's tags of both kinds count against one limit
//! (`protocol::MAX_MINED_PER_INPUT`), the mined ones read first.
//!
//! **A hard cap no honest client reaches** ([`MAX_DEFERRED_EDITS`]): past it
//! an edit is dropped, with nothing sent back (FU4a, FU3 verify M2), and the
//! server logs one warning until the queue next empties.

use std::collections::VecDeque;

use crate::block::BlockId;
use crate::protocol::{BlockChange, EditHand, EditTag, MinedBlock, UseTag};

/// Most edits one client may have waiting. 16,384 — the client's own bound on
/// edits it holds back unsent (`remote_client::INPUT_CARRY_OVER_MAX_CHANGES`).
///
/// **Memory (FU4a, FU3 verify M1).** A capped input keeps only the edits that
/// fit, in a buffer shrunk to them (FU3 kept each capped input's whole
/// deserialized buffer, about 69 KB, for the four edits that fitted: a modified
/// joiner sending one full input a tick at the cap pinned about 280 MB). The
/// worst case is now the cap's edits (20 B each since C3a-fix-1: the block
/// change and the four-byte hand it was made with), as many group headers (a
/// group holds at least one edit: 72 B each), a tag for each edit at most
/// (C3c-1: a mined or a use tag, `(u32, EditTag)`, 40 B since a use tag
/// carries the stack it used and the tool it wore; 24 B before), and the
/// front group's processed slack (one input's edits at most, about 4,370):
/// **under 2.5 MB of capacity per client** (about 2.25 MB), plus
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

/// What one input says about the hands its edits were made with.
#[derive(Clone, Debug, Default)]
pub struct InputHand {
    /// What the input said was in hand (`InputPacket.held_kind` / `held_id`).
    pub held: (u8, u16),
    /// The input's hotbar slot (`InputPacket.hotbar_slot`).
    pub hotbar_slot: Option<u8>,
    /// C3a-fix-1 — each edit's own slot and hand, parallel to the edits
    /// (`InputPacket.edit_hands`); may be short or empty.
    pub edit_hands: Vec<EditHand>,
    /// C3a-fix-1 — `InputPacket.events_applied`.
    pub events_applied: u32,
    /// C3c-1 — `InputPacket.use_tags`: the hand before each of the input's
    /// uses, paired with their edits as the input is read.
    pub use_tags: Vec<UseTag>,
}

/// The hand one edit was made with: four bytes, kept beside each waiting
/// edit (the queue's memory bound counts them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hand {
    /// The hotbar slot (below 9), or [`Hand::LATEST`].
    slot: u8,
    /// The held item's wire pair.
    kind: u8,
    id: u16,
}

impl Hand {
    /// No slot known: the latest input's at processing.
    const LATEST: u8 = u8::MAX;

    /// Edit `k`'s hand by `input`: its own entry in `edit_hands` (a slot of 9
    /// or more falls back to the input's), else the input's slot and hand.
    fn of(input: &InputHand, k: usize) -> Self {
        let input_slot = input.hotbar_slot.filter(|&s| s < 9).unwrap_or(Self::LATEST);
        match input.edit_hands.get(k) {
            Some(&(slot, kind, id)) => Hand { slot: if slot < 9 { slot } else { input_slot }, kind, id },
            None => Hand { slot: input_slot, kind: input.held.0, id: input.held.1 },
        }
    }

    /// The hotbar slot, below 9; `None` = the latest input's.
    pub fn slot(self) -> Option<u8> {
        (self.slot < 9).then_some(self.slot)
    }

    /// The held item's wire pair (`item_kind`, id).
    pub fn held(self) -> (u8, u16) {
        (self.kind, self.id)
    }
}

/// The edits of one input that have not been processed yet, with what they
/// are judged by.
pub struct EditGroup {
    /// In the order the client made them, each with the hand it was made
    /// with (C3a-fix-1).
    edits: VecDeque<(BlockChange, Hand)>,
    /// FU4a (L2) — the input's `mined` tags (C3c-1: and its use tags), each
    /// paired with its own edit (by the edit's place in the input, 0 its
    /// first), newest place first so the front edit's tag is the last. At
    /// most `protocol::MAX_MINED_PER_INPUT` in all, each with a distinct
    /// waiting edit.
    tags: Vec<(u32, EditTag)>,
    /// The place in the input of the front of `edits`.
    next: u32,
    /// FU4a (L1) — the joiner's life when the input was read
    /// (`server::ServerPlayer::respawns`): a group from an earlier life is
    /// sent back, not applied.
    pub life: u32,
    /// C3a-fix-1 — the window events the client had applied when it made
    /// these edits (`InputPacket.events_applied`).
    pub events_applied: u32,
}

impl EditGroup {
    /// One input's edits, oldest first: the first `keep` of them (the rest
    /// are dropped, their count returned — before their tags are paired, so
    /// a flood at the cap costs nothing more), each paired with its own tag
    /// from `tags` (the input's first `protocol::MAX_MINED_PER_INPUT`, its
    /// DoS guard; [`pair_tags`]) or (C3c-1) from `input.use_tags` (as many
    /// more as that limit leaves; [`pair_use_tags`]) and its own hand by
    /// `input` ([`Hand::of`]). `block_at` reads the world, for the first
    /// edit of a tagged cell.
    pub fn new(
        mut edits: Vec<BlockChange>,
        keep: usize,
        tags: &[MinedBlock],
        input: InputHand,
        life: u32,
        block_at: impl FnMut(i32, i32, i32) -> BlockId,
    ) -> (Self, usize) {
        let dropped = edits.len().saturating_sub(keep);
        if dropped > 0 {
            edits.truncate(keep);
        }
        let tags = &tags[..tags.len().min(crate::protocol::MAX_MINED_PER_INPUT)];
        let room = crate::protocol::MAX_MINED_PER_INPUT - tags.len();
        let uses = &input.use_tags[..input.use_tags.len().min(room)];
        let used = pair_use_tags(&edits, uses);
        let mut paired: Vec<(u32, EditTag)> = pair_tags(&edits, tags, &used, block_at)
            .into_iter()
            .map(|(at, m)| (at, EditTag::Mined(m)))
            .chain(used.into_iter().map(|(at, u)| (at, EditTag::Use(u))))
            .collect();
        paired.sort_by_key(|&(at, _)| std::cmp::Reverse(at));
        paired.shrink_to_fit();
        let mut handed = VecDeque::with_capacity(edits.len());
        handed.extend(edits.into_iter().enumerate().map(|(k, bc)| (bc, Hand::of(&input, k))));
        (Self { edits: handed, tags: paired, next: 0, life, events_applied: input.events_applied }, dropped)
    }

    /// Edits still waiting.
    pub fn len(&self) -> usize {
        self.edits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Take the next edit, with its own tag and hand.
    pub fn pop_front(&mut self) -> Option<(BlockChange, Option<EditTag>, Hand)> {
        let (bc, hand) = self.edits.pop_front()?;
        let tag = match self.tags.last() {
            Some(&(at, _)) if at == self.next => self.tags.pop().map(|(_, m)| m),
            _ => None,
        };
        self.next += 1;
        Some((bc, tag, hand))
    }

    /// Take every edit left, their tags gone with them (a dead joiner's or
    /// an earlier life's: sent back, never applied).
    pub fn take_edits(&mut self) -> Vec<BlockChange> {
        self.tags = Vec::new();
        std::mem::take(&mut self.edits).into_iter().map(|(bc, _)| bc).collect()
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
/// break. A tag no edit takes yields nothing and is dropped. C3c-1 — an edit
/// a use tag was paired with (`used`, [`pair_use_tags`]) takes no mined tag.
/// Returns the pairs newest place first.
fn pair_tags(
    edits: &[BlockChange],
    tags: &[MinedBlock],
    used: &[(u32, UseTag)],
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
        if takes_tag(old, bc.new_block) && !used.iter().any(|&(u, _)| u == at as u32) {
            paired.push((at as u32, unpaired.remove(k)));
        }
    }
    paired.reverse();
    paired.shrink_to_fit();
    paired
}

/// C3c-1 — pair each use tag with the LAST edit of its cell in the input not
/// already paired with one (the client's own pairing: it sends no edit of a
/// cell behind a use-tagged edit of that cell in one input,
/// `remote_client::tag_cut`). A use tag with no edit of its cell is dropped.
fn pair_use_tags(edits: &[BlockChange], uses: &[UseTag]) -> Vec<(u32, UseTag)> {
    let mut paired: Vec<(u32, UseTag)> = Vec::with_capacity(uses.len());
    for tag in uses.iter().rev() {
        let cell = (tag.x, tag.y, tag.z);
        let at = edits
            .iter()
            .enumerate()
            .rev()
            .find(|&(k, bc)| (bc.x, bc.y, bc.z) == cell && !paired.iter().any(|&(p, _)| p == k as u32))
            .map(|(k, _)| k as u32);
        match at {
            Some(at) => paired.push((at, tag.clone())),
            None => log::debug!("a use tag for {cell:?} with no edit of its cell: dropped"),
        }
    }
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
                    g.edits.capacity() * size_of::<(BlockChange, Hand)>()
                        + g.tags.capacity() * size_of::<(u32, EditTag)>()
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
        EditGroup::new(edits.to_vec(), usize::MAX, tags, InputHand::default(), 0, world).0
    }

    /// Each edit with its mined tag, in order.
    fn drain(mut g: EditGroup) -> Vec<(i32, Option<MinedBlock>)> {
        std::iter::from_fn(|| g.pop_front()).map(|(bc, t, _)| (bc.x, t.and_then(|t| t.mined().copied()))).collect()
    }

    fn use_tag(x: i32) -> UseTag {
        UseTag { x, y: 70, z: 0, kind: 2, slot: 0, used: None, tool: WireItem::None }
    }

    /// C3c-1 — each edit with its tag of either kind, in order.
    fn drain_tags(mut g: EditGroup) -> Vec<(i32, Option<EditTag>)> {
        std::iter::from_fn(|| g.pop_front()).map(|(bc, t, _)| (bc.x, t)).collect()
    }

    /// C3c-1 — a use tag pairs with the last edit of its cell, and a mined
    /// tag never takes a use's edit (a bucket fill empties its cell too).
    #[test]
    fn a_use_tag_pairs_with_the_last_edit_of_its_cell_and_a_mined_tag_never_takes_it() {
        let input = InputHand { use_tags: vec![use_tag(1), use_tag(9)], ..Default::default() };
        // A mine of cell 1 (a tall grass), then a bucket fill there… the use
        // is the last edit of its cell.
        let edits = [edit(1, AIR), edit(2, STONE), edit(1, AIR)];
        let (g, _) = EditGroup::new(edits.to_vec(), usize::MAX, &[tag(1)], input, 0, |_, _, _| STONE);
        assert_eq!(
            drain_tags(g),
            vec![(1, Some(EditTag::Mined(tag(1)))), (2, None), (1, Some(EditTag::Use(use_tag(1))))],
        );
        // A mined tag alone for a use's edit: the use keeps it.
        let input = InputHand { use_tags: vec![use_tag(3)], ..Default::default() };
        let (g, _) = EditGroup::new(vec![edit(3, AIR)], usize::MAX, &[tag(3)], input, 0, |_, _, _| STONE);
        assert_eq!(drain_tags(g), vec![(3, Some(EditTag::Use(use_tag(3))))]);
    }

    /// C3c-1 — mined and use tags count against one limit: the mined ones
    /// first, then use tags up to it.
    #[test]
    fn mined_and_use_tags_share_one_limit() {
        let max = crate::protocol::MAX_MINED_PER_INPUT;
        let edits: Vec<BlockChange> = (0..30).map(|x| edit(x, AIR)).collect();
        let mined: Vec<MinedBlock> = (0..10).map(tag).collect();
        let input = InputHand { use_tags: (10..30).map(use_tag).collect(), ..Default::default() };
        let (g, _) = EditGroup::new(edits, usize::MAX, &mined, input, 0, |_, _, _| STONE);
        assert_eq!(g.tags.len(), max);
        let uses = g.tags.iter().filter(|(_, t)| t.use_tag().is_some()).count();
        assert_eq!(uses, max - 10);
    }

    #[test]
    fn groups_wait_in_arrival_order_and_a_part_put_back_goes_first() {
        let mut q = EditQueue::default();
        assert_eq!(q.push_back(group(&[edit(1, 1), edit(2, 1)], &[])), 0);
        assert_eq!(q.push_back(group(&[edit(3, 1)], &[])), 0);
        assert_eq!(q.push_back(group(&[], &[tag(9)])), 0, "an empty group is not kept");
        assert_eq!(q.len(), 3);
        let mut first = q.pop_front().expect("oldest");
        assert_eq!(first.pop_front().map(|(e, _, _)| e.x), Some(1));
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

    /// C3a-fix-1 (C-M1) — each edit keeps its own slot and hand from the
    /// input's `edit_hands`; one past the list, or with a slot of 9 or more,
    /// takes the input's.
    #[test]
    fn each_edit_keeps_its_own_slot_and_hand() {
        let input = InputHand {
            held: (4, 40),
            hotbar_slot: Some(5),
            edit_hands: vec![(2, 1, 10), (11, 1, 11)],
            events_applied: 7,
            use_tags: Vec::new(),
        };
        let edits = [edit(1, STONE), edit(2, STONE), edit(3, STONE)];
        let (mut g, _) = EditGroup::new(edits.to_vec(), usize::MAX, &[], input, 0, |_, _, _| AIR);
        assert_eq!(g.events_applied, 7);
        let hands: Vec<(Option<u8>, (u8, u16))> =
            std::iter::from_fn(|| g.pop_front()).map(|(_, _, h)| (h.slot(), h.held())).collect();
        assert_eq!(hands, vec![(Some(2), (1, 10)), (Some(5), (1, 11)), (Some(5), (4, 40))]);
        // No input slot either: the latest input's, at processing.
        let (mut g, _) = EditGroup::new(vec![edit(1, STONE)], usize::MAX, &[], InputHand::default(), 0, |_, _, _| AIR);
        assert_eq!(g.pop_front().map(|(_, _, h)| h.slot()), Some(None));
        assert_eq!(std::mem::size_of::<Hand>(), 4, "four bytes beside each waiting edit");
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
            + (MAX_DEFERRED_EDITS + max_input) * size_of::<(BlockChange, Hand)>()
            + (MAX_DEFERRED_EDITS + crate::protocol::MAX_MINED_PER_INPUT) * size_of::<(u32, EditTag)>();
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
                q.push_back(EditGroup::new(v, usize::MAX, &tags, InputHand::default(), 0, |_, _, _| STONE).0);
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
                let (g, dropped) = EditGroup::new(v, keep, &tags, InputHand::default(), 0, |_, _, _| STONE);
                let dropped = dropped + q.push_back(g);
                assert_eq!(dropped + q.len(), MAX_DEFERRED_EDITS - 4 + max_input, "only what fits is kept");
                peak = peak.max(q.allocated_bytes());
                assert!(peak <= bound, "{peak} bytes allocated, past the bound of {bound}");
            }
            println!(
                "edit queue at the cap: peak {peak} bytes allocated (bound {bound}; {} B a group, \
                 {} B an edit, {} B a tag, {max_input} edits an input)",
                size_of::<EditGroup>(),
                size_of::<(BlockChange, Hand)>(),
                size_of::<(u32, EditTag)>(),
            );
            assert!(bound < 2_500_000, "the bound the docs state: under 2.5 MB ({bound})");
        }
    }
}

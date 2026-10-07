//! Per-client outbound state queue — bounded `StateUpdate`s (gap-audit T1-5).
//!
//! `HostedServer` used to put every block change and entity event of a tick
//! into ONE `StateUpdate`. A crop field, a piston array, a `/we` edit or a
//! late-join backfill could push it past `protocol::MAX_PACKET_SIZE` (64 KiB,
//! about 4,300 block changes), and every client's `safe_deserialize` then
//! dropped the whole packet — blocks, spawns, despawns and player positions.
//!
//! Each joined client now owns a [`ClientOutbox`]:
//!
//! - **Reliable deltas** — block changes, entity spawns, entity despawns — go
//!   into one FIFO, in the order the server produced them, with each entry's
//!   exact bincode size beside it.
//! - **Entity updates are not queued.** Only the latest state of an entity
//!   matters, so they sit in a latest-per-id map that every tick overwrites.
//!   An update is only sent for an entity whose spawn has already gone out to
//!   this client (or goes out earlier in the same packet) — the client ignores
//!   updates for ids it does not know.
//! - **Every packet is measured, not estimated**: a packet's size is the
//!   measured size of its snapshot fields plus the measured size of each item
//!   (bincode 1's fixed-int encoding is positional, so the sum is exact), and
//!   no packet exceeds [`STATE_UPDATE_MAX_BYTES`].
//! - **A remote client gets at most [`CLIENT_TICK_BUDGET_BYTES`] a tick**; the
//!   rest waits, in order, for later ticks. A slice of that budget
//!   ([`ENTITY_UPDATE_RESERVE_BYTES`]) goes to entity updates first, so mobs
//!   keep moving on the joiner's screen while a block backlog drains.
//! - **A backlog coalesces.** Once the queued block changes are more than one
//!   tick's budget, a further change to a cell that already has a queued change
//!   overwrites that change in place (latest wins, the cell keeps its place in
//!   line). Below that, the produced sequence is delivered exactly. Not when
//!   either change involves a block with apply-side effects (a power device, a
//!   container or economy block, a plot marker —
//!   [`crate::world::block_has_remote_apply_effects`]): the joiner's
//!   `apply_remote_block_change` only resets that block's entity when it sees
//!   the block change, so `chest → air → chest` has to arrive as written. Such
//!   a change is appended as its own entry and the cell's coalescing index
//!   points at it.
//! - **Overflow resyncs.** Past [`CLIENT_QUEUE_MAX_BYTES`] the queued block
//!   changes are dropped and their chunks recorded in a "needs resync" set,
//!   read through [`ClientOutbox::take_chunk_resync_requests`]; the chunk
//!   push (`chunk_push`, Phase B2a) sends each of them whole again.
//! - **Chunk pushes share the queue** (Phase B2a). A pushed chunk is a whole
//!   `ChunkData` packet (at most [`CHUNK_PACKET_MAX_BYTES`]) queued in line
//!   with the deltas ([`ClientOutbox::push_chunk`]) and sent as its own
//!   packet, inside the same per-tick budget, so changes queued after it
//!   apply on top of it on the client. It is a snapshot at its place in line,
//!   so no later change may be folded into one queued before it: a queued
//!   chunk is a coalescing barrier for its own cells. Overflow keeps queued
//!   chunks (their bytes are bounded by the push's credit window, not counted
//!   against the bound). **Nothing at the head of the queue can wedge it**
//!   (B2a review HIGH-1): when a chunk is next in line, the tick's first
//!   packet leaves it room (entity updates beyond the reserve wait), and a
//!   chunk that is at the head when the tick's drain starts always goes, even
//!   past the budget — so a tick sends at most the budget, or its first packet
//!   plus one chunk packet.
//!
//! The host's own in-process loopback (a local slot) has no wire to protect:
//! its outbox is unbudgeted — it drains in full every tick, split under the
//! packet cap, never coalesces and never overflows. Spec 04, "Bounded
//! StateUpdates".

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use crate::protocol::{self, BlockChange, EntitySpawn, EntityUpdate, StateUpdatePacket};

/// Largest `StateUpdate` (type tag + payload) the server will build. 8 KiB of
/// headroom under [`protocol::MAX_WIRE_PACKET_LEN`], the hard decode cap.
pub const STATE_UPDATE_MAX_BYTES: usize = 56 * 1024;

const _: () = assert!(STATE_UPDATE_MAX_BYTES + 8 * 1024 <= protocol::MAX_WIRE_PACKET_LEN);

/// Most `StateUpdate` bytes a REMOTE client is sent in one tick: about 1 MB/s
/// at 20 TPS. Counts whole packets, snapshot fields included. The first packet
/// of a tick always goes, whatever its size — it carries the player positions.
pub const CLIENT_TICK_BUDGET_BYTES: usize = 48 * 1024;

const _: () = assert!(CLIENT_TICK_BUDGET_BYTES <= STATE_UPDATE_MAX_BYTES);

/// The slice of a remote client's tick budget offered to entity updates
/// BEFORE the reliable queue, so a block backlog never freezes the mobs on
/// its screen. About 240 updates (34 bytes each since v68's velocity and
/// flags); whatever they don't use goes to the queue. Updates are
/// changed-only (`entity_broadcast`), so an idle herd takes none of it.
pub const ENTITY_UPDATE_RESERVE_BYTES: usize = 8 * 1024;

/// Largest chunk-push packet (`ChunkData`, tag included) the server builds
/// (`chunk_push::build_chunk_packets` splits a chunk's side data into
/// continuations under it). Well under one tick's budget: after the tick's
/// first packet (snapshot fields plus the entity-update reserve) a queued
/// chunk packet always fits, so one at the head of the queue never waits more
/// than a tick (B2a review HIGH-1).
pub const CHUNK_PACKET_MAX_BYTES: usize = 32 * 1024;

const _: () = assert!(
    CHUNK_PACKET_MAX_BYTES + ENTITY_UPDATE_RESERVE_BYTES + 8 * 1024 <= CLIENT_TICK_BUDGET_BYTES
);

/// Hard bound on a remote client's queued reliable bytes. Past it the queued
/// block changes are dropped and their chunks marked for resync (about
/// 140,000 changes — roughly 43 ticks of budget, so a client this far behind
/// is better served by whole chunks than by the history).
pub const CLIENT_QUEUE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// Queued block-change bytes above which a remote client counts as
/// backlogged and repeated edits to one cell coalesce. One tick's budget:
/// anything below it goes out this tick anyway.
pub const COALESCE_BACKLOG_BYTES: usize = CLIENT_TICK_BUDGET_BYTES;

/// At most one overflow warning per client per this many ticks (5 s).
const OVERFLOW_LOG_INTERVAL_TICKS: u64 = 100;

/// A chunk coordinate, `(cx, cy, cz)` — `ChunkDataPacket`'s addressing.
pub type ChunkCoord = (i32, i32, i32);

/// Chunk holding block `(x, y, z)`.
pub(crate) fn chunk_of_cell((x, y, z): (i32, i32, i32)) -> ChunkCoord {
    let cs = crate::chunk::CHUNK_SIZE as i32;
    (x.div_euclid(cs), y.div_euclid(cs), z.div_euclid(cs))
}

/// Chunk a block change lands in.
pub(crate) fn chunk_of(b: &BlockChange) -> ChunkCoord {
    chunk_of_cell((b.x, b.y, b.z))
}

/// May `newer` overwrite the still-queued `queued` change to the same cell?
///
/// Not when either carries a block entity or other side effect on the joiner's
/// apply ([`crate::world::block_has_remote_apply_effects`]). `A → B → A′`
/// folded to `A′` leaves `World::apply_remote_block_change` looking at a block
/// that never changed, so the power device / container entity / plot claim the
/// break and the replacement would have reset stays stale. Plain blocks carry
/// nothing but their id and meta, where the last value is the whole story.
fn can_coalesce(queued: &BlockChange, newer: &BlockChange) -> bool {
    use crate::world::block_has_remote_apply_effects as effects;
    !(effects(queued.new_block) || effects(newer.new_block))
}

/// Exact bincode size of one wire item — the same encoding `serialize_packet`
/// uses, so per-item sizes sum to the packet's real size.
fn wire_size<T: serde::Serialize>(item: &T) -> usize {
    bincode::serialized_size(item).expect("wire items always size") as usize
}

/// One reliable delta, in server order.
#[derive(Debug)]
enum Delta {
    Block(BlockChange),
    Spawn(EntitySpawn),
    Despawn(u32),
    /// One whole serialized `ChunkData` packet (tag included) for `coord`.
    Chunk { coord: ChunkCoord, packet: Vec<u8> },
}

#[derive(Debug)]
struct Entry {
    /// Push order; strictly increasing along the queue (binary-searchable).
    seq: u64,
    /// Exact serialized size of `delta`.
    size: usize,
    delta: Delta,
}

/// One client's outbound `StateUpdate` state. See the module docs.
#[derive(Debug)]
pub struct ClientOutbox {
    /// `None` = unbudgeted (an in-process local slot).
    tick_budget: Option<usize>,
    queue: VecDeque<Entry>,
    next_seq: u64,
    /// Exact serialized bytes of everything in `queue`.
    queued_bytes: usize,
    /// The block-change share of `queued_bytes`.
    queued_block_bytes: usize,
    /// The chunk-push share of `queued_bytes` (bounded by the push's credit
    /// window; never counted against [`CLIENT_QUEUE_MAX_BYTES`]).
    queued_chunk_bytes: usize,
    /// Backlog mode: cell → seq of its (single) queued change. `Some` only
    /// while backlogged; every queued block change is indexed while it is.
    coalesce_index: Option<HashMap<(i32, i32, i32), u64>>,
    /// Latest unsent update per entity id.
    updates: BTreeMap<u32, EntityUpdate>,
    /// Where the next update fill starts, so ids take turns when the budget
    /// can't carry every entity in one tick.
    update_cursor: u32,
    /// Ids whose spawn this client has been sent and whose despawn it hasn't.
    delivered: HashSet<u32>,
    /// Ids with a spawn still in `queue`, and how many: an entity that
    /// leaves a joiner's interest and comes back (`entity_broadcast`) is
    /// spawned again, possibly before its first spawn has drained.
    spawn_queued: HashMap<u32, u32>,
    /// Ids with a despawn still in `queue` → that entry's `seq`. A re-entry
    /// spawn cancels it (see [`Self::push_spawns`]).
    despawn_queued: HashMap<u32, u64>,
    /// Chunks whose block changes were dropped on overflow.
    resync: BTreeSet<ChunkCoord>,
    /// Overflow log rate limit.
    next_overflow_log: u64,
    dropped_since_log: usize,
}

impl ClientOutbox {
    /// A remote client's outbox (`remote = true`: budgeted, coalescing,
    /// bounded) or an in-process local slot's (`false`: drains in full).
    pub fn new(remote: bool) -> Self {
        ClientOutbox {
            tick_budget: remote.then_some(CLIENT_TICK_BUDGET_BYTES),
            queue: VecDeque::new(),
            next_seq: 0,
            queued_bytes: 0,
            queued_block_bytes: 0,
            queued_chunk_bytes: 0,
            coalesce_index: None,
            updates: BTreeMap::new(),
            update_cursor: 0,
            delivered: HashSet::new(),
            spawn_queued: HashMap::new(),
            despawn_queued: HashMap::new(),
            resync: BTreeSet::new(),
            next_overflow_log: 0,
            dropped_since_log: 0,
        }
    }

    /// Serialized bytes of reliable deltas (and chunk pushes) waiting to go out.
    pub fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }

    /// Queue entity spawns (the late-join backfill, or a tick's new entities).
    ///
    /// A spawn for an id whose despawn is still queued — it left this
    /// client's interest and came back before the withdrawal went out —
    /// cancels that despawn (review D2a MEDIUM-2). The client's spawn
    /// replaces any copy it already holds, so the withdrawal is moot; sent,
    /// it could land in the same frame as the re-entry and delete the new
    /// copy, after which this outbox (counting the id as delivered) would
    /// feed it updates the client drops.
    pub fn push_spawns(&mut self, spawns: &[EntitySpawn]) {
        for s in spawns {
            if let Some(seq) = self.despawn_queued.remove(&s.id) {
                self.cancel_entry(seq);
            }
            *self.spawn_queued.entry(s.id).or_insert(0) += 1;
            self.push_entry(Delta::Spawn(s.clone()));
        }
    }

    /// Will this client hold entity `id` once everything queued has gone
    /// out? Only then is an update for it worth keeping.
    fn will_hold(&self, id: u32) -> bool {
        !self.despawn_queued.contains_key(&id)
            && (self.delivered.contains(&id) || self.spawn_queued.contains_key(&id))
    }

    /// Drop the queued entry `seq` (a reliable delta that no longer needs
    /// sending).
    fn cancel_entry(&mut self, seq: u64) {
        let Ok(at) = self.queue.binary_search_by_key(&seq, |e| e.seq) else {
            return;
        };
        if let Some(e) = self.queue.remove(at) {
            self.queued_bytes -= e.size;
            match e.delta {
                Delta::Block(_) => self.queued_block_bytes -= e.size,
                Delta::Chunk { .. } => self.queued_chunk_bytes -= e.size,
                Delta::Spawn(_) | Delta::Despawn(_) => {}
            }
        }
    }

    /// Queue one tick of the server's diff. Spawns go in ahead of despawns
    /// (the order the diff implies), block changes after; updates fold into
    /// the latest-per-id map. `now` is the server tick (for the rate-limited
    /// overflow log).
    pub fn push_tick(
        &mut self,
        now: u64,
        spawns: &[EntitySpawn],
        despawns: &[u32],
        blocks: &[BlockChange],
        updates: &[EntityUpdate],
    ) {
        self.push_spawns(spawns);
        for &id in despawns {
            // Only an entity this client knows about (or is about to) needs
            // telling it is gone; its pending update is moot either way.
            self.updates.remove(&id);
            if self.will_hold(id) {
                self.push_entry(Delta::Despawn(id));
                self.despawn_queued.insert(id, self.next_seq - 1);
            }
        }
        for b in blocks {
            self.push_block(b);
            if self.tick_budget.is_some()
                && self.queued_bytes - self.queued_chunk_bytes > CLIENT_QUEUE_MAX_BYTES
            {
                self.overflow(now);
            }
        }
        for u in updates {
            if self.will_hold(u.id) {
                self.updates.insert(u.id, u.clone());
            }
        }
    }

    /// Chunks whose queued block changes were dropped because this client
    /// fell more than [`CLIENT_QUEUE_MAX_BYTES`] behind, sorted, and cleared.
    /// The chunk push (`chunk_push`) sends each listed chunk whole again, on
    /// this same FIFO, so block changes queued after it still apply on top of
    /// the snapshot, in order.
    pub fn take_chunk_resync_requests(&mut self) -> Vec<ChunkCoord> {
        std::mem::take(&mut self.resync).into_iter().collect()
    }

    /// Queue a chunk push: the serialized `ChunkData` packet(s) of chunk
    /// `coord`, in line after everything queued so far.
    ///
    /// **The Phase B rule.** A chunk push is a snapshot at its place in the
    /// queue, so no later change may be folded into a change queued *before*
    /// it — that would apply the newer value ahead of the snapshot, and the
    /// snapshot would then overwrite it with older state. So the chunk's
    /// cells leave the coalescing index here: a newer change to one of them
    /// appends after the snapshot instead.
    pub fn push_chunk(&mut self, coord: ChunkCoord, packets: Vec<Vec<u8>>) {
        if let Some(index) = &mut self.coalesce_index {
            index.retain(|&cell, _| chunk_of_cell(cell) != coord);
        }
        for packet in packets {
            self.push_entry(Delta::Chunk { coord, packet });
        }
    }

    /// Queue a "column is local" note (`ColumnLocal`, Phase B2b) for column
    /// `col`, in line like a chunk push and counted with them: it is part of
    /// the numbered chunk stream. No coalescing barrier is needed beyond the
    /// one `push_chunk` sets: a note goes only for a column none of whose
    /// chunks was sent, so no change to it is queued before it.
    pub fn push_column_note(&mut self, col: (i32, i32), note: Vec<u8>) {
        self.push_chunk((col.0, 0, col.1), vec![note]);
    }

    /// Serialized chunk-push bytes waiting to go out. Test-only.
    #[cfg(test)]
    pub fn queued_chunk_bytes(&self) -> usize {
        self.queued_chunk_bytes
    }

    fn push_entry(&mut self, delta: Delta) {
        let size = match &delta {
            Delta::Block(b) => wire_size(b),
            Delta::Spawn(s) => wire_size(s),
            Delta::Despawn(id) => wire_size(id),
            Delta::Chunk { packet, .. } => packet.len(),
        };
        match delta {
            Delta::Block(_) => self.queued_block_bytes += size,
            Delta::Chunk { .. } => self.queued_chunk_bytes += size,
            _ => {}
        }
        self.queued_bytes += size;
        self.queue.push_back(Entry { seq: self.next_seq, size, delta });
        self.next_seq += 1;
    }

    fn push_block(&mut self, b: &BlockChange) {
        let cell = (b.x, b.y, b.z);
        if let Some(index) = &self.coalesce_index
            && let Some(&seq) = index.get(&cell)
            && let Ok(at) = self.queue.binary_search_by_key(&seq, |e| e.seq)
            && let Delta::Block(queued) = &self.queue[at].delta
            && can_coalesce(queued, b)
        {
            // Latest wins, in place: same cell, same size, same slot in line.
            self.queue[at].delta = Delta::Block(b.clone());
            return;
        }
        // Not coalescable (or nothing queued at this cell): a new entry, and
        // the index points at it — it is now the cell's newest queued change,
        // the one a later repeat is compared against.
        self.push_entry(Delta::Block(b.clone()));
        let seq = self.next_seq - 1;
        if let Some(index) = &mut self.coalesce_index {
            index.insert(cell, seq);
        } else if self.tick_budget.is_some() && self.queued_block_bytes > COALESCE_BACKLOG_BYTES {
            self.enter_coalesce_mode();
        }
    }

    /// Backlogged: fold every already-queued repeat of a cell into its first
    /// queued change (latest value wins) and index what's left, so later
    /// pushes coalesce in O(log n). A repeat that [`can_coalesce`] refuses stays
    /// queued as its own entry and becomes the cell's newest — the one the next
    /// repeat is compared against, and the one the index ends up pointing at.
    fn enter_coalesce_mode(&mut self) {
        let mut newest: HashMap<(i32, i32, i32), usize> = HashMap::new();
        let mut latest: HashMap<usize, BlockChange> = HashMap::new();
        let mut dead = vec![false; self.queue.len()];
        for (i, e) in self.queue.iter().enumerate() {
            // A queued chunk push is a barrier for its cells: nothing after
            // it folds into a change before it (the Phase B rule).
            if let Delta::Chunk { coord, .. } = &e.delta {
                newest.retain(|&cell, _| chunk_of_cell(cell) != *coord);
                continue;
            }
            if let Delta::Block(b) = &e.delta {
                let cell = (b.x, b.y, b.z);
                match newest.get(&cell) {
                    // Entries are only ever folded into when coalescable, so the
                    // original at `j` stands in for its folded value here.
                    Some(&j)
                        if matches!(&self.queue[j].delta, Delta::Block(q) if can_coalesce(q, b)) =>
                    {
                        latest.insert(j, b.clone());
                        dead[i] = true;
                    }
                    _ => {
                        newest.insert(cell, i);
                    }
                }
            }
        }
        for (j, b) in latest {
            self.queue[j].delta = Delta::Block(b);
        }
        let mut i = 0;
        let mut freed = 0;
        self.queue.retain(|e| {
            let keep = !dead[i];
            i += 1;
            if !keep {
                freed += e.size;
            }
            keep
        });
        self.queued_bytes -= freed;
        self.queued_block_bytes -= freed;
        // Walked in order, so each cell maps to its NEWEST queued change —
        // unless a chunk push of its chunk is queued after that change: then
        // it maps to nothing, and a repeat appends after the snapshot.
        let mut index = HashMap::new();
        for e in &self.queue {
            match &e.delta {
                Delta::Block(b) => {
                    index.insert((b.x, b.y, b.z), e.seq);
                }
                Delta::Chunk { coord, .. } => {
                    index.retain(|&cell, _| chunk_of_cell(cell) != *coord);
                }
                _ => {}
            }
        }
        self.coalesce_index = Some(index);
    }

    /// Too far behind: drop every queued block change, remember its chunk.
    /// Spawns, despawns and chunk pushes stay — the client needs them in
    /// order, and they are bounded by the entity population and the push's
    /// credit window, not by block churn.
    fn overflow(&mut self, now: u64) {
        let mut dropped = 0usize;
        let mut freed = 0usize;
        let resync = &mut self.resync;
        self.queue.retain(|e| match &e.delta {
            Delta::Block(b) => {
                resync.insert(chunk_of(b));
                dropped += 1;
                freed += e.size;
                false
            }
            _ => true,
        });
        self.queued_bytes -= freed;
        self.queued_block_bytes = 0;
        self.coalesce_index = None;
        self.dropped_since_log += dropped;
        if now >= self.next_overflow_log {
            log::warn!(
                "StateUpdate queue for a client passed {CLIENT_QUEUE_MAX_BYTES} bytes: dropped \
                 {} block change(s); {} chunk(s) will be pushed again whole",
                self.dropped_since_log,
                self.resync.len()
            );
            self.dropped_since_log = 0;
            self.next_overflow_log = now + OVERFLOW_LOG_INTERVAL_TICKS;
        }
    }

    /// Build this tick's `StateUpdate`s for this client. `template` carries
    /// the tick's snapshot fields (tick, players, world time, reserve,
    /// weather) with EMPTY delta vectors; every packet repeats it. Always at
    /// least one packet. Each is at most [`STATE_UPDATE_MAX_BYTES`]; a remote
    /// client's total is at most [`CLIENT_TICK_BUDGET_BYTES`] — except that
    /// the first packet always goes (the snapshot alone may be bigger), and
    /// so does a chunk push at the head of the queue when the drain starts
    /// (nothing at the head can wedge the queue; see the module docs).
    pub fn drain_packets(&mut self, template: &StateUpdatePacket) -> Vec<Vec<u8>> {
        debug_assert!(
            template.block_changes.is_empty()
                && template.entity_spawns.is_empty()
                && template.entity_updates.is_empty()
                && template.entity_despawns.is_empty(),
            "the template carries snapshot fields only"
        );
        let base = 1 + wire_size(template);
        let budget = self.tick_budget.unwrap_or(usize::MAX);
        let mut spent = 0usize;
        let mut out: Vec<Vec<u8>> = Vec::new();
        // Has anything left the reliable queue this tick?
        let mut moved = false;
        loop {
            // A queued chunk push goes as its own packet, in line — never
            // first (the first packet of a tick carries the player positions)
            // and past the budget only when it was at the head of the queue
            // as the drain started: then nothing can hold it back for good
            // (B2a review HIGH-1). Otherwise it waits for the next tick.
            if !out.is_empty()
                && let Some(Entry { size, delta: Delta::Chunk { .. }, .. }) = self.queue.front()
            {
                if moved && spent + size > budget {
                    break;
                }
                moved = true;
                let Some(Entry { size, delta: Delta::Chunk { packet, .. }, .. }) =
                    self.queue.pop_front()
                else {
                    unreachable!("the front is a chunk push");
                };
                self.queued_bytes -= size;
                self.queued_chunk_bytes -= size;
                spent += size;
                out.push(packet);
                continue;
            }
            let room = STATE_UPDATE_MAX_BYTES.min(budget.saturating_sub(spent));
            if !out.is_empty() && room <= base {
                break;
            }
            let room = room.max(base);
            let mut pkt = template.clone();
            let mut size = base;
            if self.tick_budget.is_some() {
                let reserve = room.min(base + ENTITY_UPDATE_RESERVE_BYTES);
                self.fill_updates(&mut pkt, &mut size, reserve);
            }
            let queue_drained = self.fill_reliable(&mut pkt, &mut size, room);
            moved |= !(pkt.block_changes.is_empty()
                && pkt.entity_spawns.is_empty()
                && pkt.entity_despawns.is_empty());
            // A chunk push next in line keeps its room: entity updates past
            // the reserve take only what it leaves (else a heavy entity load
            // would crowd it, and everything behind it, out tick after tick).
            let update_limit = match self.queue.front() {
                Some(Entry { size: chunk, delta: Delta::Chunk { .. }, .. }) => {
                    room.min(budget.saturating_sub(spent.saturating_add(*chunk))).max(size)
                }
                _ => room,
            };
            let updates_drained = self.fill_updates(&mut pkt, &mut size, update_limit);
            let carried = !(pkt.block_changes.is_empty()
                && pkt.entity_spawns.is_empty()
                && pkt.entity_updates.is_empty()
                && pkt.entity_despawns.is_empty());
            if !out.is_empty() && !carried {
                break;
            }
            let bytes = protocol::serialize_packet(protocol::PacketType::StateUpdate, &pkt);
            debug_assert_eq!(bytes.len(), size, "measured size is the real size");
            spent += bytes.len();
            out.push(bytes);
            if matches!(self.queue.front(), Some(Entry { delta: Delta::Chunk { .. }, .. })) {
                continue;
            }
            if (queue_drained && updates_drained) || !carried {
                break;
            }
        }
        out
    }

    /// Move reliable deltas, oldest first, into `pkt` while they fit under
    /// `limit`, stopping at a queued chunk push (it goes as its own packet).
    /// Returns whether the queue is now empty. Leaves backlog mode once no
    /// block change is left queued.
    fn fill_reliable(&mut self, pkt: &mut StateUpdatePacket, size: &mut usize, limit: usize) -> bool {
        while let Some(front) = self.queue.front() {
            if *size + front.size > limit || matches!(front.delta, Delta::Chunk { .. }) {
                break;
            }
            let e = self.queue.pop_front().expect("front exists");
            *size += e.size;
            self.queued_bytes -= e.size;
            match e.delta {
                Delta::Block(b) => {
                    self.queued_block_bytes -= e.size;
                    if let Some(index) = &mut self.coalesce_index
                        && index.get(&(b.x, b.y, b.z)) == Some(&e.seq)
                    {
                        index.remove(&(b.x, b.y, b.z));
                    }
                    pkt.block_changes.push(b);
                }
                Delta::Spawn(s) => {
                    if let Some(n) = self.spawn_queued.get_mut(&s.id) {
                        *n -= 1;
                        if *n == 0 {
                            self.spawn_queued.remove(&s.id);
                        }
                    }
                    self.delivered.insert(s.id);
                    pkt.entity_spawns.push(s);
                }
                Delta::Despawn(id) => {
                    self.delivered.remove(&id);
                    self.updates.remove(&id);
                    self.despawn_queued.remove(&id);
                    pkt.entity_despawns.push(id);
                }
                Delta::Chunk { .. } => unreachable!("a chunk push is never folded into a StateUpdate"),
            }
        }
        if self.queued_block_bytes == 0 {
            // No block changes left queued: back to exact sequences.
            self.coalesce_index = None;
        }
        self.queue.is_empty()
    }

    /// Move pending updates for delivered entities into `pkt` while they fit
    /// under `limit`, taking ids in turn from the cursor. Returns whether
    /// every deliverable update went. An update waits while a spawn for its
    /// id is still queued: it belongs to the newest incarnation, and sent
    /// ahead of that spawn it would land on the copy the spawn then
    /// replaces, leaving the new one with no velocity or flags.
    fn fill_updates(&mut self, pkt: &mut StateUpdatePacket, size: &mut usize, limit: usize) -> bool {
        let cursor = self.update_cursor;
        let mut taken: Vec<u32> = Vec::new();
        let mut drained = true;
        for (&id, u) in self.updates.range(cursor..).chain(self.updates.range(..cursor)) {
            if !self.delivered.contains(&id) || self.spawn_queued.contains_key(&id) {
                continue;
            }
            let s = wire_size(u);
            if *size + s > limit {
                drained = false;
                break;
            }
            *size += s;
            taken.push(id);
        }
        for id in &taken {
            if let Some(u) = self.updates.remove(id) {
                pkt.entity_updates.push(u);
            }
        }
        if let Some(&last) = taken.last() {
            self.update_cursor = last.wrapping_add(1);
        }
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> StateUpdatePacket {
        StateUpdatePacket {
            tick: 7,
            players: Vec::new(),
            block_changes: Vec::new(),
            world_time: 1000,
            last_acked_input: 0,
            entity_spawns: Vec::new(),
            entity_updates: Vec::new(),
            entity_despawns: Vec::new(),
            reserve_richness: 1.0,
            reserve_target_sats: 0,
            reserve_current_sats: 0,
            rain_ticks_left: 0,
            storm_ticks_left: 0,
            own_hunger: 0,
        }
    }

    fn bc(x: i32, block: u16) -> BlockChange {
        BlockChange::with_meta(x, 0, 0, block, 0)
    }

    fn spawn(id: u32) -> EntitySpawn {
        EntitySpawn {
            id,
            kind: protocol::EntityKind::Item,
            x: 0.0,
            y: 64.0,
            z: 0.0,
            yaw: 0.0,
            health: 0,
            item_kind: protocol::item_kind::MATERIAL,
            item_id: 4,
            item_count: 1,
            full_item: protocol::WireItem::None,
        }
    }

    fn update(id: u32, x: f32) -> EntityUpdate {
        EntityUpdate { id, x, y: 64.0, z: 0.0, yaw: 0.0, state: 0, ..Default::default() }
    }

    fn decode(pkt: &[u8]) -> StateUpdatePacket {
        let (_, payload) = protocol::deserialize_header(pkt).unwrap();
        protocol::safe_deserialize(payload).unwrap()
    }

    /// Drain one tick, checking the per-packet cap and (remote) the budget.
    fn tick(ob: &mut ClientOutbox, remote: bool) -> Vec<StateUpdatePacket> {
        let raw = ob.drain_packets(&template());
        assert!(!raw.is_empty());
        for p in &raw {
            assert!(p.len() <= STATE_UPDATE_MAX_BYTES, "{} bytes", p.len());
        }
        if remote {
            assert!(raw.iter().map(Vec::len).sum::<usize>() <= CLIENT_TICK_BUDGET_BYTES);
        }
        raw.iter().map(|p| decode(p)).collect()
    }

    #[test]
    fn block_change_and_update_sizes_match_the_documented_wire_sizes() {
        assert_eq!(wire_size(&bc(1, 1)), 15);
        assert_eq!(wire_size(&update(1, 0.0)), 34);
        assert_eq!(wire_size(&3u32), 4);
    }

    #[test]
    fn an_idle_tick_still_sends_one_snapshot_packet() {
        let mut ob = ClientOutbox::new(true);
        let states = tick(&mut ob, true);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].world_time, 1000);
    }

    #[test]
    fn a_remote_burst_drains_in_order_within_the_budget() {
        let mut ob = ClientOutbox::new(true);
        let blocks: Vec<BlockChange> = (0..10_000).map(|i| bc(i, 1)).collect();
        ob.push_tick(0, &[], &[], &blocks, &[]);
        let mut got = Vec::new();
        let mut ticks = 0;
        while got.len() < blocks.len() {
            ticks += 1;
            assert!(ticks < 20, "drains in bounded time");
            for s in tick(&mut ob, true) {
                got.extend(s.block_changes.iter().map(|b| b.x));
            }
        }
        assert_eq!(got, (0..10_000).collect::<Vec<_>>());
        assert!(ticks >= 3, "150 KB over a 48 KiB budget takes several ticks");
        assert_eq!(ob.queued_bytes(), 0);
    }

    #[test]
    fn a_local_burst_drains_in_one_tick_across_several_capped_packets() {
        let mut ob = ClientOutbox::new(false);
        let blocks: Vec<BlockChange> = (0..10_000).map(|i| bc(i, 1)).collect();
        ob.push_tick(0, &[], &[], &blocks, &[]);
        let states = tick(&mut ob, false);
        assert!(states.len() >= 3);
        let got: Vec<i32> = states.iter().flat_map(|s| s.block_changes.iter().map(|b| b.x)).collect();
        assert_eq!(got, (0..10_000).collect::<Vec<_>>());
    }

    #[test]
    fn unbacklogged_repeats_are_delivered_exactly() {
        let mut ob = ClientOutbox::new(true);
        ob.push_tick(0, &[], &[], &[bc(5, 1), bc(5, 2), bc(5, 3)], &[]);
        let got: Vec<u16> = tick(&mut ob, true)
            .iter()
            .flat_map(|s| s.block_changes.iter().map(|b| b.new_block))
            .collect();
        assert_eq!(got, vec![1, 2, 3]);
    }

    #[test]
    fn a_backlog_coalesces_latest_wins_in_place() {
        let mut ob = ClientOutbox::new(true);
        // 5 then the filler then 5 again: entering backlog mode folds the
        // repeat into the first slot.
        let mut blocks = vec![bc(-1, 10)];
        blocks.extend((0..5_000).map(|i| bc(i, 1)));
        blocks.push(bc(-1, 11));
        ob.push_tick(0, &[], &[], &blocks, &[]);
        // More repeats while backlogged coalesce in place too.
        ob.push_tick(1, &[], &[], &[bc(-1, 12), bc(-1, 13)], &[]);
        let mut got = Vec::new();
        for _ in 0..10 {
            for s in tick(&mut ob, true) {
                got.extend(s.block_changes.iter().map(|b| (b.x, b.new_block)));
            }
        }
        let at: Vec<u16> = got.iter().filter(|(x, _)| *x == -1).map(|(_, v)| *v).collect();
        assert_eq!(at, vec![13], "one change for the cell, carrying its latest value");
        assert_eq!(got.first(), Some(&(-1, 13)), "the cell kept its first place in line");
        assert_eq!(got.len(), 5_001, "every distinct cell arrives");
        // Backlog cleared: exact sequences again.
        ob.push_tick(2, &[], &[], &[bc(-1, 1), bc(-1, 2)], &[]);
        let after: Vec<u16> = tick(&mut ob, true)
            .iter()
            .flat_map(|s| s.block_changes.iter().map(|b| b.new_block))
            .collect();
        assert_eq!(after, vec![1, 2]);
    }

    // A cell edited back and forth around a block that carries a block entity
    // (`apply_remote_block_change`'s power-device / container / plot-marker
    // side effects) must reach the joiner as the sequence the server produced.
    // Folded to its last value the joiner never sees the break, so it keeps
    // the stale entity.

    /// The block ids delivered for the cell at `x` (y = z = 0), in order.
    fn at_cell(states: &[StateUpdatePacket], x: i32) -> Vec<u16> {
        states
            .iter()
            .flat_map(|s| s.block_changes.iter().filter(move |b| b.x == x).map(|b| b.new_block))
            .collect()
    }

    fn drain_all(ob: &mut ClientOutbox) -> Vec<StateUpdatePacket> {
        let mut all = Vec::new();
        for _ in 0..10 {
            all.extend(tick(ob, true));
        }
        all
    }

    #[test]
    fn a_backlog_does_not_coalesce_a_block_entity_cell_when_pushed_after_it() {
        use crate::block::{AIR, CHEST};
        let mut ob = ClientOutbox::new(true);
        let filler: Vec<BlockChange> = (0..5_000).map(|i| bc(i, 1)).collect();
        ob.push_tick(0, &[], &[], &filler, &[]);
        assert!(ob.coalesce_index.is_some(), "the filler put this client in backlog mode");
        // Chest placed, broken, replaced — in one tick and across two.
        ob.push_tick(1, &[], &[], &[bc(-1, CHEST), bc(-1, AIR), bc(-1, CHEST)], &[]);
        ob.push_tick(2, &[], &[], &[bc(-1, AIR), bc(-1, CHEST)], &[]);
        // A plain cell beside it still coalesces (latest wins).
        ob.push_tick(3, &[], &[], &[bc(-2, 1), bc(-2, 2), bc(-2, 10)], &[]);
        let all = drain_all(&mut ob);
        assert_eq!(at_cell(&all, -1), vec![CHEST, AIR, CHEST, AIR, CHEST], "exact sequence");
        assert_eq!(at_cell(&all, -2), vec![10], "plain cells still fold");
    }

    #[test]
    fn entering_backlog_does_not_fold_a_block_entity_cell() {
        use crate::block::{AIR, CHEST};
        let mut ob = ClientOutbox::new(true);
        // The chest sequence is queued BEFORE the backlog starts, so it goes
        // through the fold in `enter_coalesce_mode`, not the in-place push path.
        let mut blocks = vec![bc(-1, CHEST), bc(-1, AIR), bc(-1, CHEST)];
        blocks.extend([bc(-2, 1), bc(-2, 2), bc(-2, 10)]);
        blocks.extend((0..5_000).map(|i| bc(i, 1)));
        ob.push_tick(0, &[], &[], &blocks, &[]);
        assert!(ob.coalesce_index.is_some());
        // A later repeat compares against the newest entry at the cell.
        ob.push_tick(1, &[], &[], &[bc(-1, AIR), bc(-2, 11)], &[]);
        let all = drain_all(&mut ob);
        assert_eq!(at_cell(&all, -1), vec![CHEST, AIR, CHEST, AIR]);
        assert_eq!(at_cell(&all, -2), vec![11], "plain repeats still fold to one");
    }

    #[test]
    fn a_coalesced_backlog_still_resets_the_joiners_stale_block_entities() {
        // End to end through the joiner's apply: the battery at `pos` holds
        // charge the server has since discarded (broken and rebuilt). Delivered
        // as the full sequence the joiner re-registers a fresh device; folded to
        // "battery" it would see no change and keep the stale one.
        use crate::block::{AIR, BATTERY, CHEST};
        let pos = (4, 64, 4);
        let cell = |block| BlockChange::with_meta(pos.0, pos.1, pos.2, block, 0);
        let mut ob = ClientOutbox::new(true);
        let filler: Vec<BlockChange> = (0..5_000).map(|i| bc(i, 1)).collect();
        ob.push_tick(0, &[], &[], &filler, &[]);
        ob.push_tick(1, &[], &[], &[cell(AIR), cell(BATTERY)], &[]);
        // And a chest at a second cell, broken and put back.
        let chest_pos = (6, 64, 6);
        let chest = |block| BlockChange::with_meta(chest_pos.0, chest_pos.1, chest_pos.2, block, 0);
        ob.push_tick(2, &[], &[], &[chest(AIR), chest(CHEST)], &[]);

        let mut w = crate::world::World::new();
        w.set_block(pos.0, pos.1, pos.2, BATTERY);
        let mut stale = crate::power::PowerDeviceData::new(
            crate::power::PowerDeviceKind::Battery,
            crate::meta::Facing::East,
        );
        stale.charge = 9;
        w.insert_power_device(pos, stale);
        w.set_block(chest_pos.0, chest_pos.1, chest_pos.2, CHEST);
        w.insert_chest(chest_pos, crate::chest::ChestData::new());

        for state in drain_all(&mut ob) {
            for b in &state.block_changes {
                w.apply_remote_block_change(b);
            }
        }
        assert_eq!(w.get_block(pos.0, pos.1, pos.2), BATTERY);
        assert_eq!(
            w.power_device_at(pos).expect("the placement registers a device").charge,
            0,
            "a fresh battery, not the stale one"
        );
        assert_eq!(w.get_block(chest_pos.0, chest_pos.1, chest_pos.2), CHEST);
        assert!(w.chest_at(chest_pos).is_none(), "the break dropped the stale chest entity");
    }

    #[test]
    fn updates_wait_for_their_spawn_and_stop_at_the_despawn() {
        let mut ob = ClientOutbox::new(true);
        let filler: Vec<BlockChange> = (0..8_000).map(|i| bc(i, 1)).collect();
        // Blocks first, then the spawn: the spawn is queued behind ~117 KB.
        ob.push_tick(0, &[], &[], &filler, &[]);
        ob.push_tick(1, &[spawn(9)], &[], &[], &[update(9, 1.0)]);
        let mut first_spawn_tick = None;
        for t in 0..10 {
            for s in tick(&mut ob, true) {
                if s.entity_spawns.iter().any(|e| e.id == 9) {
                    first_spawn_tick.get_or_insert(t);
                }
                if first_spawn_tick.is_none() {
                    assert!(s.entity_updates.is_empty(), "no update before its spawn");
                }
            }
            ob.push_tick(2 + t, &[], &[], &[], &[update(9, t as f32)]);
        }
        assert!(first_spawn_tick.unwrap() >= 2, "the spawn really was queued");
        // Despawn: the pending update is dropped, nothing for 9 afterwards.
        ob.push_tick(50, &[], &[9], &[], &[update(9, 99.0)]);
        let states = tick(&mut ob, true);
        assert!(states.iter().any(|s| s.entity_despawns == vec![9]));
        ob.push_tick(51, &[], &[], &[], &[update(9, 100.0)]);
        assert!(tick(&mut ob, true).iter().all(|s| s.entity_updates.is_empty()));
    }

    #[test]
    fn a_spawn_and_despawn_both_still_queued_arrive_in_order() {
        let mut ob = ClientOutbox::new(true);
        let filler: Vec<BlockChange> = (0..8_000).map(|i| bc(i, 1)).collect();
        ob.push_tick(0, &[], &[], &filler, &[]);
        ob.push_tick(1, &[spawn(4)], &[], &[], &[]);
        ob.push_tick(2, &[], &[4], &[], &[]);
        let mut order = Vec::new();
        for _ in 0..10 {
            for s in tick(&mut ob, true) {
                order.extend(s.entity_spawns.iter().map(|e| ('s', e.id)));
                order.extend(s.entity_despawns.iter().map(|&id| ('d', id)));
            }
        }
        assert_eq!(order, vec![('s', 4), ('d', 4)]);
    }

    /// What a joiner holds for entity `id` after `states`, applied the way
    /// its client does — per packet: spawns, then updates, then despawns.
    /// `None` = not held; `Some(None)` = held with no update since its
    /// (latest) spawn; `Some(Some(x))` = held, latest update at `x`.
    fn client_view(states: &[StateUpdatePacket], id: u32, view: &mut Option<Option<f32>>) {
        for s in states {
            if s.entity_spawns.iter().any(|e| e.id == id) {
                *view = Some(None);
            }
            if let Some(u) = s.entity_updates.iter().rev().find(|u| u.id == id)
                && view.is_some()
            {
                *view = Some(Some(u.x));
            }
            if s.entity_despawns.contains(&id) {
                *view = None;
            }
        }
    }

    /// Review D2a MEDIUM-2 (b), the delivered case. Entity 9 is shown, a
    /// block backlog builds, it leaves interest (despawn queued behind the
    /// backlog) and comes back (re-spawn + its update). The joiner must end
    /// up holding the re-spawned copy WITH that update — not the update sent
    /// ahead of the despawn onto the old copy and then lost to the spawn.
    #[test]
    fn a_reentry_while_the_withdrawal_is_queued_keeps_its_update() {
        let mut ob = ClientOutbox::new(true);
        let mut view = None;
        ob.push_tick(0, &[spawn(9)], &[], &[], &[update(9, 1.0)]);
        client_view(&tick(&mut ob, true), 9, &mut view);
        assert_eq!(view, Some(Some(1.0)));

        let filler: Vec<BlockChange> = (0..8_000).map(|i| bc(i, 1)).collect();
        ob.push_tick(1, &[], &[], &filler, &[]);
        ob.push_tick(2, &[], &[9], &[], &[]);
        ob.push_tick(3, &[spawn(9)], &[], &[], &[update(9, 7.0)]);
        let mut despawns = 0;
        for _ in 0..10 {
            let states = tick(&mut ob, true);
            despawns += states.iter().filter(|s| s.entity_despawns.contains(&9)).count();
            client_view(&states, 9, &mut view);
        }
        assert_eq!(view, Some(Some(7.0)), "the re-entered copy, with its update");
        assert_eq!(despawns, 0, "the moot withdrawal was cancelled, never sent");
    }

    /// Review D2a MEDIUM-2 (b), the queued case: the first spawn hasn't gone
    /// out either when the entity leaves and re-enters. Two spawns queued:
    /// draining the first must not let the update jump ahead of the second,
    /// and the despawn must not delete the re-entry's update.
    #[test]
    fn a_reentry_before_the_first_spawn_drained_keeps_its_update() {
        let mut ob = ClientOutbox::new(true);
        let mut view = None;
        let filler: Vec<BlockChange> = (0..8_000).map(|i| bc(i, 1)).collect();
        ob.push_tick(0, &[], &[], &filler, &[]);
        ob.push_tick(1, &[spawn(9)], &[], &[], &[update(9, 1.0)]);
        ob.push_tick(2, &[], &[9], &[], &[]);
        ob.push_tick(3, &[spawn(9)], &[], &[], &[update(9, 7.0)]);
        for _ in 0..10 {
            client_view(&tick(&mut ob, true), 9, &mut view);
        }
        assert_eq!(view, Some(Some(7.0)), "held, with the re-entry's update last");
    }

    #[test]
    fn updates_get_a_reserved_share_during_a_backlog() {
        let mut ob = ClientOutbox::new(true);
        ob.push_tick(0, &[spawn(1)], &[], &[], &[]);
        let _ = tick(&mut ob, true);
        let filler: Vec<BlockChange> = (0..20_000).map(|i| bc(i, 1)).collect();
        ob.push_tick(1, &[], &[], &filler, &[update(1, 5.0)]);
        let states = tick(&mut ob, true);
        assert!(
            states.iter().any(|s| s.entity_updates.iter().any(|u| u.id == 1)),
            "a mob keeps moving while blocks are backlogged"
        );
    }

    #[test]
    fn updates_take_turns_when_they_exceed_the_budget() {
        let mut ob = ClientOutbox::new(true);
        let n = 4_000u32; // 84 KB of updates a tick — more than the budget
        let spawns: Vec<EntitySpawn> = (0..n).map(spawn).collect();
        ob.push_tick(0, &spawns, &[], &[], &[]);
        for _ in 0..5 {
            let _ = tick(&mut ob, true);
        }
        let mut seen = HashSet::new();
        for t in 0..3 {
            let ups: Vec<EntityUpdate> = (0..n).map(|id| update(id, t as f32)).collect();
            ob.push_tick(10 + t, &[], &[], &[], &ups);
            for s in tick(&mut ob, true) {
                seen.extend(s.entity_updates.iter().map(|u| u.id));
            }
        }
        assert_eq!(seen.len(), n as usize, "every entity gets an update within a few ticks");
    }

    #[test]
    fn overflow_drops_queued_blocks_into_the_resync_set_and_keeps_entities() {
        let mut ob = ClientOutbox::new(true);
        ob.push_tick(0, &[spawn(1)], &[], &[], &[]);
        // Distinct cells across chunks x = 0..9 (16 wide), beyond the bound.
        let n = CLIENT_QUEUE_MAX_BYTES / 15 + 10;
        let blocks: Vec<BlockChange> = (0..n as i32)
            .map(|i| BlockChange::with_meta(i % 160, i / 160 % 16, i / 2560, 1, 0))
            .collect();
        ob.push_tick(0, &[], &[], &blocks, &[]);
        assert!(ob.queued_bytes() <= CLIENT_QUEUE_MAX_BYTES);
        let resync = ob.take_chunk_resync_requests();
        assert!(!resync.is_empty());
        assert!(resync.contains(&(0, 0, 0)) && resync.contains(&(9, 0, 0)));
        assert!(ob.take_chunk_resync_requests().is_empty(), "take clears the set");
        // The spawn survived; the dropped history never arrives.
        let mut spawns = 0;
        let mut blocks_got = 0;
        for _ in 0..3 {
            for s in tick(&mut ob, true) {
                spawns += s.entity_spawns.len();
                blocks_got += s.block_changes.len();
            }
        }
        assert_eq!(spawns, 1);
        assert!(blocks_got < n / 2, "the overflowed history was dropped, got {blocks_got}");
    }

    #[test]
    fn a_local_outbox_never_overflows_or_coalesces() {
        let mut ob = ClientOutbox::new(false);
        let n = CLIENT_QUEUE_MAX_BYTES / 15 + 10;
        let mut blocks: Vec<BlockChange> = (0..n as i32).map(|i| bc(i, 1)).collect();
        blocks.push(bc(0, 2));
        ob.push_tick(0, &[], &[], &blocks, &[]);
        assert!(ob.take_chunk_resync_requests().is_empty());
        let got: usize = tick(&mut ob, false).iter().map(|s| s.block_changes.len()).sum();
        assert_eq!(got, n + 1, "every change, repeats included, in one tick");
    }

    // ── B2a: chunk pushes in the queue ──────────────────────────────────

    /// What one drained packet was.
    #[derive(Debug, PartialEq)]
    enum Out {
        State(Vec<i32>),
        Chunk(ChunkCoord),
    }

    /// A serialized `ChunkData` packet for `coord`, padded to about `bytes`.
    fn chunk_packet(coord: ChunkCoord, bytes: usize) -> Vec<u8> {
        let pkt = protocol::ChunkDataPacket {
            cx: coord.0,
            cy: coord.1,
            cz: coord.2,
            compressed_blocks: vec![7; bytes],
            meta: Vec::new(),
            entities: Vec::new(),
            attachments: Vec::new(),
        };
        protocol::serialize_packet(protocol::PacketType::ChunkData, &pkt)
    }

    /// Drain one tick into what each packet carried (block-change xs, or the
    /// pushed chunk), checking the cap and the remote budget.
    fn drain_mixed(ob: &mut ClientOutbox) -> Vec<Out> {
        let raw = ob.drain_packets(&template());
        assert!(raw.iter().map(Vec::len).sum::<usize>() <= CLIENT_TICK_BUDGET_BYTES);
        raw.iter()
            .map(|p| {
                let (t, payload) = protocol::deserialize_header(p).unwrap();
                assert!(p.len() <= STATE_UPDATE_MAX_BYTES);
                match t {
                    protocol::PacketType::StateUpdate => {
                        let s: StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
                        Out::State(s.block_changes.iter().map(|b| b.x).collect())
                    }
                    protocol::PacketType::ChunkData => {
                        let c: protocol::ChunkDataPacket = protocol::safe_deserialize(payload).unwrap();
                        Out::Chunk((c.cx, c.cy, c.cz))
                    }
                    other => panic!("unexpected {other:?}"),
                }
            })
            .collect()
    }

    #[test]
    fn a_chunk_push_goes_as_its_own_packet_in_line_with_the_deltas() {
        let mut ob = ClientOutbox::new(true);
        ob.push_tick(0, &[], &[], &[bc(1, 1), bc(2, 1)], &[]);
        ob.push_chunk((0, 0, 0), vec![chunk_packet((0, 0, 0), 100)]);
        ob.push_tick(0, &[], &[], &[bc(3, 1)], &[]);
        let out = drain_mixed(&mut ob);
        assert_eq!(
            out,
            vec![Out::State(vec![1, 2]), Out::Chunk((0, 0, 0)), Out::State(vec![3])],
            "the snapshot sits between the changes before and after it"
        );
        assert_eq!(ob.queued_bytes(), 0);
    }

    #[test]
    fn a_tick_starts_with_the_snapshot_and_chunks_wait_for_the_budget() {
        let mut ob = ClientOutbox::new(true);
        for i in 0..8 {
            ob.push_chunk((i, 0, 0), vec![chunk_packet((i, 0, 0), 10_000)]);
        }
        let first = drain_mixed(&mut ob);
        assert!(matches!(first[0], Out::State(_)), "positions first, always");
        let chunks_first = first.iter().filter(|o| matches!(o, Out::Chunk(_))).count();
        assert!((1..8).contains(&chunks_first), "the budget holds the rest back: {chunks_first}");
        let mut all: Vec<ChunkCoord> = first
            .into_iter()
            .filter_map(|o| match o {
                Out::Chunk(c) => Some(c),
                Out::State(_) => None,
            })
            .collect();
        for _ in 0..4 {
            all.extend(drain_mixed(&mut ob).into_iter().filter_map(|o| match o {
                Out::Chunk(c) => Some(c),
                Out::State(_) => None,
            }));
        }
        assert_eq!(all, (0..8).map(|i| (i, 0, 0)).collect::<Vec<_>>(), "all of them, in order");
        assert_eq!(ob.queued_chunk_bytes(), 0);
    }

    #[test]
    fn a_change_after_a_queued_push_is_never_folded_ahead_of_it() {
        let mut ob = ClientOutbox::new(true);
        // Backlogged: cell x=5 (chunk (0,0,0)) has a queued change …
        let mut blocks = vec![bc(5, 10)];
        blocks.extend((100..5_000).map(|i| bc(i * 16, 1))); // other chunks
        ob.push_tick(0, &[], &[], &blocks, &[]);
        // … then a push of its chunk, then a newer change to the same cell.
        ob.push_chunk((0, 0, 0), vec![chunk_packet((0, 0, 0), 50)]);
        ob.push_tick(1, &[], &[], &[bc(5, 11)], &[]);
        let mut order = Vec::new();
        for _ in 0..10 {
            for o in drain_mixed(&mut ob) {
                match o {
                    Out::Chunk(c) => order.push(format!("chunk{c:?}")),
                    Out::State(xs) => order.extend(xs.into_iter().filter(|&x| x == 5).map(|_| "x5".to_string())),
                }
            }
        }
        assert_eq!(order, ["x5", "chunk(0, 0, 0)", "x5"], "the newer change comes after the snapshot");
    }

    #[test]
    fn entering_backlog_treats_a_queued_push_as_a_barrier() {
        let mut ob = ClientOutbox::new(true);
        // Not yet backlogged: change, push, change — all queued as written.
        ob.push_tick(0, &[], &[], &[bc(5, 10)], &[]);
        ob.push_chunk((0, 0, 0), vec![chunk_packet((0, 0, 0), 50)]);
        ob.push_tick(0, &[], &[], &[bc(5, 11)], &[]);
        // Now a backlog forms; the fold must not merge 11 into 10's slot.
        let filler: Vec<BlockChange> = (100..5_000).map(|i| bc(i * 16, 1)).collect();
        ob.push_tick(0, &[], &[], &filler, &[]);
        ob.push_tick(1, &[], &[], &[bc(5, 12)], &[]);
        let mut seen = Vec::new();
        for _ in 0..10 {
            let raw = ob.drain_packets(&template());
            for p in raw {
                let (t, payload) = protocol::deserialize_header(&p).unwrap();
                if t == protocol::PacketType::ChunkData {
                    seen.push(-1);
                } else {
                    let s: StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
                    seen.extend(s.block_changes.iter().filter(|b| b.x == 5).map(|b| i32::from(b.new_block)));
                }
            }
        }
        assert_eq!(seen, [10, -1, 12], "after the push the cell keeps its latest value, behind the push");
    }

    #[test]
    fn an_overflow_keeps_queued_pushes_and_does_not_count_them() {
        let mut ob = ClientOutbox::new(true);
        ob.push_chunk((9, 0, 9), vec![chunk_packet((9, 0, 9), 40_000)]);
        let blocks: Vec<BlockChange> =
            (0..(CLIENT_QUEUE_MAX_BYTES / 15 - 1000) as i32).map(|i| bc(i, 1)).collect();
        ob.push_tick(0, &[], &[], &blocks, &[]);
        assert!(ob.queued_bytes() > CLIENT_QUEUE_MAX_BYTES, "the push's bytes don't trip the bound");
        assert!(ob.take_chunk_resync_requests().is_empty(), "no overflow yet");
        ob.push_tick(1, &[], &[], &(0..2_000).map(|i| bc(-i - 1, 1)).collect::<Vec<_>>(), &[]);
        assert!(!ob.take_chunk_resync_requests().is_empty(), "now it overflowed");
        assert!(ob.queued_chunk_bytes() > 0, "the queued push survived");
        let out = drain_mixed(&mut ob);
        assert!(out.contains(&Out::Chunk((9, 0, 9))));
    }

    // ── B2a review HIGH-1: nothing at the head of the queue wedges it ───

    /// Drain up to `ticks` ticks (each within the budget); what arrived, in order.
    fn drain_ticks(ob: &mut ClientOutbox, ticks: usize) -> Vec<Out> {
        let mut all = Vec::new();
        for _ in 0..ticks {
            all.extend(drain_mixed(ob).into_iter().filter(|o| *o != Out::State(Vec::new())));
        }
        all
    }

    #[test]
    fn a_chunk_that_cannot_fit_behind_a_ticks_changes_goes_next_tick_and_never_wedges() {
        let mut ob = ClientOutbox::new(true);
        // About 42 KB of changes, then a full-size chunk packet that can't
        // fit behind them, then a change, another full-size push, a change.
        ob.push_tick(0, &[], &[], &(0..2_800).map(|i| bc(i, 1)).collect::<Vec<_>>(), &[]);
        ob.push_chunk((0, 0, 0), vec![chunk_packet((0, 0, 0), CHUNK_PACKET_MAX_BYTES - 64)]);
        ob.push_tick(1, &[], &[], &[bc(9_000, 1)], &[]);
        ob.push_chunk((1, 0, 0), vec![chunk_packet((1, 0, 0), CHUNK_PACKET_MAX_BYTES - 64)]);
        ob.push_tick(2, &[], &[], &[bc(9_001, 1)], &[]);
        let got = drain_ticks(&mut ob, 5);
        let xs: Vec<i32> = got
            .iter()
            .flat_map(|o| match o {
                Out::State(xs) => xs.clone(),
                Out::Chunk(_) => Vec::new(),
            })
            .collect();
        assert_eq!(xs.len(), 2_802, "every change arrived");
        let order: Vec<&Out> = got.iter().filter(|o| !matches!(o, Out::State(xs) if xs.len() > 1)).collect();
        assert_eq!(
            order,
            [&Out::Chunk((0, 0, 0)), &Out::State(vec![9_000]), &Out::Chunk((1, 0, 0)), &Out::State(vec![9_001])],
            "in line: each snapshot between the changes around it"
        );
        assert_eq!(ob.queued_bytes(), 0, "the queue emptied");
    }

    #[test]
    fn a_chunk_at_the_head_when_the_tick_starts_always_goes_whatever_its_size() {
        let mut ob = ClientOutbox::new(true);
        // Bigger than a whole tick's budget (the builder never makes one, but
        // nothing at the head may ever hold the queue).
        ob.push_chunk((0, 0, 0), vec![chunk_packet((0, 0, 0), CLIENT_TICK_BUDGET_BYTES)]);
        ob.push_tick(0, &[], &[], &[bc(1, 1)], &[]);
        let raw = ob.drain_packets(&template());
        let kinds: Vec<protocol::PacketType> =
            raw.iter().map(|p| protocol::deserialize_header(p).unwrap().0).collect();
        assert_eq!(
            kinds[..2],
            [protocol::PacketType::StateUpdate, protocol::PacketType::ChunkData],
            "positions first, then the oversized chunk"
        );
        assert_eq!(drain_mixed(&mut ob), vec![Out::State(vec![1])], "the change behind it next tick");
        assert_eq!(ob.queued_bytes(), 0);
    }

    #[test]
    fn entity_updates_never_crowd_out_a_chunk_next_in_line() {
        let mut ob = ClientOutbox::new(true);
        let n = 3_000u32; // about 102 KB of updates a tick: more than the budget
        ob.push_tick(0, &(0..n).map(spawn).collect::<Vec<_>>(), &[], &[], &[]);
        for _ in 0..5 {
            let _ = drain_mixed(&mut ob);
        }
        assert_eq!(ob.queued_bytes(), 0, "the spawns are delivered");
        ob.push_chunk((2, 0, 2), vec![chunk_packet((2, 0, 2), 20_000)]);
        let ups: Vec<EntityUpdate> = (0..n).map(|id| update(id, 1.0)).collect();
        ob.push_tick(10, &[], &[], &[bc(5, 1)], &ups);
        // Within the budget (drain_mixed checks), and the chunk goes now.
        let out = drain_mixed(&mut ob);
        assert!(out.contains(&Out::Chunk((2, 0, 2))), "the chunk was not crowded out: {out:?}");
        assert!(out.contains(&Out::State(vec![5])) || drain_mixed(&mut ob).contains(&Out::State(vec![5])));
    }
}

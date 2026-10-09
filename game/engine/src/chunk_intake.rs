//! A joiner taking in the chunks its server pushes (Phase B2a; Spec 04 §4.1).
//!
//! The server side is `chunk_push`. Here, on the joined client:
//!
//! - **Push wins.** A pushed chunk REPLACES whatever this client holds there —
//!   its own generation included — with its blocks, metadata, render-visible
//!   block entities and face attachments ([`ChunkIntake::apply`]). A column
//!   that was not loaded is marked loaded once ALL its chunks have been pushed
//!   (the server plans whole columns); until then this client's streamer
//!   neither generates over it nor counts it loaded, and server changes to
//!   its pushed chunks still apply ([`ChunkIntake::holds_chunk`]). A pushed
//!   column is never evicted: when this client lets it go — or a part-pushed
//!   one leaves its range — it is discarded and the server told
//!   ([`ChunkIntake::let_go`]), so it is pushed afresh on return.
//! - **In order.** A snapshot sits between the block changes that came
//!   before it and those after ([`interleave`]); applied out of order, a
//!   later change would be overwritten by the older snapshot.
//! - **Nothing dropped.** Every packet is applied as it is drained (an
//!   insert: cheap). Only the light pass and the meshing — the expensive
//!   part — wait in a queue drained a few columns a frame
//!   ([`ChunkIntake::take_relight`]).
//! - **Acknowledged.** Every `ChunkData` packet received counts towards the
//!   cumulative `InputPacket.chunk_ack` that opens the server's credit window
//!   — one that does not decode too ([`ChunkIntake::count_undecodable`]), or
//!   every later ack and drop report would be one short.
//! - **Drops are repeated until applied.** A let-go report rides every input
//!   until the server has applied one that carried it (its
//!   `last_acked_input` echo, [`ChunkIntake::confirm_drops`]); the report's
//!   `as_of` makes a repeat harmless on the server.
//!
//! - **Local notes** (Phase B2b, `chunk_verdict`). A server in `--chunk-sync
//!   touched` mode tells this joiner, for every column within
//!   `R = min(render distance, JoinAccept.chunk_note_radius)` of its SERVER
//!   body, either the column (a push, as above) or "this column is local"
//!   (`ColumnLocal`, numbered and acknowledged with the pushes). The client
//!   generates its own terrain everywhere a single-player client would,
//!   inside `R` too, before any verdict (B2b fix D1: waiting left a void
//!   moat round every join). A note CONFIRMS a column: one already
//!   generated is just marked local, one not generated yet is generated as
//!   usual. A push REPLACES it whole. So a touched column can show its
//!   pristine generation until its push lands. A local column is let go
//!   like a pushed one (discarded and reported), so the server decides it
//!   afresh on return; a column generated before any verdict (neither noted
//!   nor pushed) is none of the server's business: it unloads like a
//!   single-player column and is never reported.
//! - **The column check** ([`ChunkIntake::verify_local`]). A note carries the
//!   hash of the server's column as generation makes it
//!   (`chunk_verdict::column_hash`, cached with its `Untouched` verdict). The
//!   check is pending from the note; it is queued once this client holds the
//!   column ([`ChunkIntake::column_held`]: at the note for a column already
//!   held, else wherever the column is generated or restored) and run a few a
//!   frame ([`ChunkIntake::run_checks`]) — first, whatever the budget, when a
//!   server change or a pushed chunk is about to land on the column. It
//!   hashes the column as held; on a difference it hashes a SCRATCH
//!   generation of it (B2b fix HIGH-1), since this client may have written to
//!   its own copy since generating it (its snowfall, fluids, falling blocks,
//!   its player's edits, a drifted copy back from the evicted store). Scratch
//!   = note: generation agrees, the column is kept as it stands. Scratch ≠
//!   note: a real generation difference — the column is let go of (reported)
//!   and every input from then on asks for everything to be pushed
//!   ([`ChunkIntake::column_mismatch`], `InputPacket::column_mismatch`); no
//!   column is checked after that (the server pushes them all again).
//!
//! Renderer-free (the game loop meshes what [`ChunkIntake::take_relight`]
//! hands it), so it is unit-tested on a bare `World`.

use std::collections::VecDeque;

use crate::biome::BiomeGenerator;
use crate::chunk::Chunk;
use crate::chunk_push::{cell_pos, cells_in};
use crate::protocol::{ChunkDataPacket, ChunkDrop, ColumnMismatch, PushedAttachment, PushedEntity};
use crate::state_outbox::ChunkCoord;
use crate::world::{FaceAttachment, World, MAX_CHUNK_Y};

/// One packet of the server's numbered chunk stream, as `RemoteClient`
/// queues it.
#[derive(Debug)]
pub enum StreamItem {
    /// A pushed chunk (`ChunkData`).
    Chunk(ChunkDataPacket),
    /// "Column `(cx, cz)` is local" (`ColumnLocal`, Phase B2b), with the
    /// hash of the server's column.
    Local((i32, i32), u32),
    /// C3b-2 — a block entity's view (`StateUpdatePacket::block_views`),
    /// after the block changes of the same packet. Not part of the numbered
    /// chunk stream: it never counts towards the acknowledgement.
    View(Box<crate::protocol::BlockEntityView>),
    /// C3c-3b — a face attachment as it now stands
    /// (`StateUpdatePacket::attachment_changes`), after the block changes of
    /// the same packet. Not part of the numbered chunk stream.
    Attachment(crate::protocol::AttachmentChange),
}

/// One step of a frame's world intake, in arrival order (see [`interleave`]).
#[derive(Debug)]
pub enum IntakeStep {
    /// Apply one pushed chunk packet.
    Chunk(Box<ChunkDataPacket>),
    /// Take in a "column is local" note (Phase B2b), with its hash.
    Local((i32, i32), u32),
    /// C3b-2 — apply one block entity's view.
    View(Box<crate::protocol::BlockEntityView>),
    /// C3c-3b — apply one face attachment change.
    Attachment(crate::protocol::AttachmentChange),
    /// Apply these block changes (indices into the frame's
    /// `pending_block_changes`).
    Changes(std::ops::Range<usize>),
}

/// B2b fix-2 N3 — one owned step of a frame's world intake, in arrival order:
/// what [`ChunkIntake::plan_deltas`] hands the game loop, and what it carries
/// over to the next frame when a burst of forced column checks has used up
/// this one's allowance.
#[derive(Debug)]
pub enum Delta {
    /// Apply one pushed chunk packet.
    Chunk(Box<ChunkDataPacket>),
    /// Take in a "column is local" note, with its hash.
    Local((i32, i32), u32),
    /// Apply one block change from the server.
    Change(crate::protocol::BlockChange),
    /// C3b-2 — apply one block entity's view (`block_views::apply_view`).
    View(Box<crate::protocol::BlockEntityView>),
    /// C3c-3b — apply one face attachment change ([`apply_attachment_change`]).
    Attachment(crate::protocol::AttachmentChange),
}

/// Lay a frame's chunk-stream packets between its block changes in the order
/// they arrived: each carries the number of changes that had arrived before
/// it (`RemoteClient::chunk_queue`).
pub fn interleave(chunks: Vec<(usize, StreamItem)>, changes: usize) -> Vec<IntakeStep> {
    let mut steps = Vec::with_capacity(chunks.len() * 2 + 1);
    let mut done = 0;
    for (before, item) in chunks {
        let before = before.min(changes);
        if before > done {
            steps.push(IntakeStep::Changes(done..before));
            done = before;
        }
        steps.push(match item {
            StreamItem::Chunk(chunk) => IntakeStep::Chunk(Box::new(chunk)),
            StreamItem::Local(col, hash) => IntakeStep::Local(col, hash),
            StreamItem::View(v) => IntakeStep::View(v),
            StreamItem::Attachment(a) => IntakeStep::Attachment(a),
        });
    }
    if changes > done {
        steps.push(IntakeStep::Changes(done..changes));
    }
    steps
}

/// B2b fix LOW-3 — what a scratch generation costs against a frame's check
/// budget ([`ChunkIntake::run_checks`]), in column hashes. **Measured**
/// (B2b fix-2 N3, release build, i5-1235U with SHA extensions,
/// `chunk_verdict::tests::measure_check_costs`): one column hash 0.024 ms
/// (p95 0.029), one scratch generation and hash 1.52 ms (p95 1.74), so a
/// scratch is 63 hashes; the earlier estimate of 8 was far too low. A CPU
/// without SHA extensions hashes about ten times slower and the ratio falls
/// to about 6, so 63 over-charges there (the safe side). Charged on top of
/// the hash that found the difference.
pub const SCRATCH_CHECK_COST: usize = 63;

/// What a column check found ([`ChunkIntake::verify_local`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnCheck {
    /// No check was pending: the column was never noted, is checked already,
    /// or the session has switched to everything pushed.
    NotPending,
    /// The column as held hashes as its note said.
    Matched,
    /// It does not, but a scratch generation of it does: generation agrees,
    /// and the difference is this client's own writes since. Kept as it
    /// stands, and checked.
    Drifted,
    /// A scratch generation differs too: a real generation difference. The
    /// column was let go of (discarded and reported) and the push-everything
    /// switch set.
    Mismatched,
}

impl ColumnCheck {
    /// Was the column let go of? The caller then unloads what else it holds
    /// of it (loaded mark, meshes, fluids, wildlife).
    pub fn let_go(self) -> bool {
        self == ColumnCheck::Mismatched
    }

    /// What it cost against a frame's check budget, in column hashes.
    pub fn cost(self) -> usize {
        match self {
            ColumnCheck::NotPending => 0,
            ColumnCheck::Matched => 1,
            ColumnCheck::Drifted | ColumnCheck::Mismatched => 1 + SCRATCH_CHECK_COST,
        }
    }
}

/// A joined client's pushed-chunk state. See the module docs.
#[derive(Debug, Default)]
pub struct ChunkIntake {
    /// `ChunkData` packets taken in this session — the `chunk_ack` count.
    applied: u32,
    /// Chunks the server pushed that this client holds.
    pushed: ahash::AHashSet<ChunkCoord>,
    /// How many of each column's chunks are pushed ones.
    pushed_per_column: ahash::AHashMap<(i32, i32), u8>,
    /// Columns waiting for their light pass and meshing, oldest first.
    relight: VecDeque<(i32, i32)>,
    relight_queued: ahash::AHashSet<(i32, i32)>,
    /// Columns let go of whose report the server has not yet applied, oldest
    /// first, each with the sequence number of the first input that carried
    /// it (`None` = not sent yet).
    drops: Vec<(ChunkDrop, Option<u64>)>,
    /// Columns discarded by [`Self::apply`] (a chunk that did not decode),
    /// whose meshes the game loop drops ([`Self::take_discarded`]).
    discarded: Vec<(i32, i32)>,
    /// Phase B2b — `JoinAccept.chunk_note_radius`: how far round its server
    /// body this client is told about every column (`0` = no notes).
    note_radius: i32,
    /// Phase B2b — the column of this client's body as the server last
    /// reported it (the `JoinAccept` spawn until the first `StateUpdate`):
    /// where the server's push radius is centred.
    server_centre: Option<(i32, i32)>,
    /// Phase B2b — columns the server said are local, held (generated or to
    /// be generated) and not let go of.
    local: ahash::AHashSet<(i32, i32)>,
    /// Phase B2b — local columns whose generation is not checked yet, each
    /// with the hash its note carried ([`Self::verify_local`]).
    unverified: ahash::AHashMap<(i32, i32), u32>,
    /// B2b fix LOW-3 — columns of `unverified` this client holds, waiting
    /// for their check ([`Self::run_checks`]), oldest first.
    checks_ready: VecDeque<(i32, i32)>,
    checks_queued: ahash::AHashSet<(i32, i32)>,
    /// Phase B2b — the first local column whose generation did not hash as
    /// its note said: once set, every input asks for everything to be
    /// pushed, for the rest of the session.
    mismatch: Option<ColumnMismatch>,
    /// B2b fix-2 N3 — steps held back to a later frame, oldest first
    /// ([`Self::plan_deltas`]): they run before anything that arrives later,
    /// so the order of the stream is kept. Empty outside a burst.
    carried: VecDeque<Delta>,
}

impl ChunkIntake {
    /// `ChunkData` packets taken in so far (cumulative; `InputPacket.chunk_ack`).
    pub fn applied(&self) -> u32 {
        self.applied
    }

    /// Does this client hold any pushed chunk of column `col`?
    pub fn holds_pushed(&self, col: (i32, i32)) -> bool {
        self.pushed_per_column.contains_key(&col)
    }

    /// Does this client hold chunk `coord` as pushed? (The server sends
    /// changes only for such chunks, so they apply even while their column
    /// is still part-pushed and not yet loaded.)
    pub fn holds_chunk(&self, coord: ChunkCoord) -> bool {
        self.pushed.contains(&coord)
    }

    /// Columns holding pushed chunks that are not loaded: part-pushed ones,
    /// waiting for the rest of their column.
    pub fn part_pushed_columns(
        &self,
        loaded: &ahash::AHashSet<(i32, i32)>,
    ) -> ahash::AHashSet<(i32, i32)> {
        self.pushed_per_column.keys().filter(|c| !loaded.contains(*c)).copied().collect()
    }

    /// `ChunkData` packets received that did not decode: they still count
    /// (the server numbered them), or every later ack and `as_of` would be
    /// short.
    pub fn count_undecodable(&mut self, n: u32) {
        self.applied = self.applied.wrapping_add(n);
    }

    /// Has every chunk of column `col` been pushed?
    pub fn column_complete(&self, col: (i32, i32)) -> bool {
        self.pushed_per_column.get(&col).copied().unwrap_or(0) as i32 > MAX_CHUNK_Y
    }

    /// Phase B2b — this joined session hears a push or a "local" note for
    /// every column within `note_radius` (`JoinAccept.chunk_note_radius`; `0`
    /// = none) of its server body, which starts in column `spawn`.
    pub fn expect_notes(&mut self, note_radius: u8, spawn: (i32, i32)) {
        self.note_radius = i32::from(note_radius);
        self.server_centre = Some(spawn);
    }

    /// Phase B2b — does the server decide the columns round this client's
    /// body (a push or a "local" note for each)?
    pub fn server_decides(&self) -> bool {
        self.note_radius > 0
    }

    /// Phase B2b — the server now holds this client's body in column `col`.
    pub fn set_server_centre(&mut self, col: (i32, i32)) {
        self.server_centre = Some(col);
    }

    /// Phase B2b — take in a `ColumnLocal` note: the server said column
    /// `col` is local, and its column hashes as `hash`. It counts towards the
    /// ack like a push (it is a numbered packet of the same stream). Ignored
    /// otherwise in a session that expects no notes (B2b review LOW-2: a
    /// push-only joiner must never generate a column on one) and for a column
    /// this client holds pushed chunks of: those are the server's own data,
    /// never replaced by a generation. The column's check is pending from
    /// here; the caller queues it with [`Self::column_held`] if it holds the
    /// column, else whoever generates it does. After the push-everything
    /// switch no check is pending (B2b fix LOW-1): the column is just local.
    pub fn note_local(&mut self, col: (i32, i32), hash: u32) {
        self.applied = self.applied.wrapping_add(1);
        if !self.server_decides() {
            log::debug!("Local note for {col:?} in a session that expects none; ignored");
            return;
        }
        if self.holds_pushed(col) {
            log::debug!("Local note for pushed column {col:?}; keeping the pushed copy");
            return;
        }
        self.local.insert(col);
        if self.mismatch.is_none() {
            self.unverified.insert(col, hash);
        }
    }

    /// B2b fix LOW-2 — this client holds column `col` now: it was generated
    /// or restored (the streamer, the loading queue, both spawn-area
    /// pregenerations, the post-load void repair, a column generated for a
    /// change or a pushed chunk), or it was already held at its note. If its
    /// check is pending, queue it ([`Self::run_checks`]). Every path that
    /// makes a noted column present calls this.
    pub fn column_held(&mut self, col: (i32, i32)) {
        if self.unverified.contains_key(&col) && self.checks_queued.insert(col) {
            self.checks_ready.push_back(col);
        }
    }

    /// B2b fix-2 N3 — lay a frame's world intake out in arrival order
    /// ([`interleave`], with each change and the steps carried over from the
    /// last frame first) and bound the SYNCHRONOUS column checks it will force.
    /// A server change or a pushed chunk for a column whose check is pending
    /// checks that column first, outside [`Self::run_checks`]'s budget
    /// (`GameState::apply_world_deltas`); an unbounded burst of them (a
    /// lending host's snowfall pass in the second after a join, with up to
    /// about 289 checks pending) would be one long frame. So the first
    /// `forced_cap` distinct pending columns the frame would force are let
    /// through, and the step that would force the next, with every step after
    /// it, is held back until the next frame: the change waits for its check,
    /// as it always did, and the order of the stream is kept, because nothing
    /// overtakes a held step. The first forced check of a frame always
    /// runs, so a held burst drains a `forced_cap` a frame. The count is
    /// conservative: a note earlier in the same frame counts as pending, and
    /// a check that finds nothing to do still counts. Returns the steps to
    /// run now.
    pub fn plan_deltas(
        &mut self,
        chunks: Vec<(usize, StreamItem)>,
        changes: &[crate::protocol::BlockChange],
        forced_cap: usize,
    ) -> Vec<Delta> {
        let mut steps: VecDeque<Delta> = std::mem::take(&mut self.carried);
        for step in interleave(chunks, changes.len()) {
            match step {
                IntakeStep::Chunk(p) => steps.push_back(Delta::Chunk(p)),
                IntakeStep::Local(col, hash) => steps.push_back(Delta::Local(col, hash)),
                IntakeStep::View(v) => steps.push_back(Delta::View(v)),
                IntakeStep::Attachment(a) => steps.push_back(Delta::Attachment(a)),
                IntakeStep::Changes(range) => {
                    steps.extend(changes[range].iter().cloned().map(Delta::Change));
                }
            }
        }
        let cap = forced_cap.max(1);
        let mut forced: ahash::AHashSet<(i32, i32)> = ahash::AHashSet::new();
        let mut noted: ahash::AHashSet<(i32, i32)> = ahash::AHashSet::new();
        let mut run = Vec::with_capacity(steps.len());
        while let Some(step) = steps.pop_front() {
            let forces = match &step {
                Delta::Chunk(p) if !p.compressed_blocks.is_empty() => Some((p.cx, p.cz)),
                Delta::Change(bc) => Some(crate::chunk_stream::column_of_block(bc.x, bc.z)),
                _ => None,
            };
            if let Some(col) = forces
                && (self.unverified.contains_key(&col) || noted.contains(&col))
                && !forced.contains(&col)
            {
                if forced.len() >= cap {
                    self.carried.push_back(step);
                    self.carried.extend(steps);
                    return run;
                }
                forced.insert(col);
            }
            if let Delta::Local(col, _) = &step {
                noted.insert(*col);
            }
            run.push(step);
        }
        run
    }

    /// Steps held back to a later frame. Test-only.
    #[cfg(test)]
    pub fn carried_len(&self) -> usize {
        self.carried.len()
    }

    /// Is column `col`'s check still pending? Test-only.
    #[cfg(test)]
    pub fn has_pending_check(&self, col: (i32, i32)) -> bool {
        self.unverified.contains_key(&col)
    }

    /// B2b fix LOW-3 — run queued checks ([`Self::column_held`]), oldest
    /// first, until `budget` column hashes are spent (a scratch generation
    /// costs [`SCRATCH_CHECK_COST`] more; the check that crosses the budget
    /// finishes, so one always runs). A queued column this client no longer
    /// holds (`loaded`) is skipped: it is queued again when it is. Returns
    /// the columns let go of, for the caller to unload.
    pub fn run_checks(
        &mut self,
        world: &mut World,
        biome_gen: &BiomeGenerator,
        loaded: &ahash::AHashSet<(i32, i32)>,
        budget: usize,
    ) -> Vec<(i32, i32)> {
        let mut spent = 0;
        let mut gone = Vec::new();
        while spent < budget {
            let Some(col) = self.checks_ready.pop_front() else { break };
            self.checks_queued.remove(&col);
            if !loaded.contains(&col) {
                continue;
            }
            let check = self.verify_local(world, biome_gen, col);
            spent += check.cost();
            if check.let_go() {
                gone.push(col);
            }
        }
        gone
    }

    /// Phase B2b — check local column `col`, which this client holds, against
    /// its note's hash, if its check is pending (the caller decides when:
    /// [`Self::run_checks`], or first, whatever the budget, before a server
    /// change or a pushed chunk lands on it). It hashes the column as held;
    /// if that differs it hashes a scratch generation of it
    /// (`chunk_verdict::generated_column_hash`, `biome_gen` being the
    /// generator this client generates with), so this client's own writes
    /// since generating it never read as a determinism bug (B2b fix HIGH-1):
    /// the column is then kept as it stands. Only a scratch that differs too
    /// lets the column go (discarded and reported) and sets
    /// [`Self::column_mismatch`] for the rest of the session, which clears
    /// every pending check (B2b fix LOW-1: the server pushes every noted
    /// column again, so nothing is left worth checking).
    pub fn verify_local(&mut self, world: &mut World, biome_gen: &BiomeGenerator, col: (i32, i32)) -> ColumnCheck {
        if self.mismatch.is_some() {
            return ColumnCheck::NotPending;
        }
        let Some(server_hash) = self.unverified.remove(&col) else { return ColumnCheck::NotPending };
        let held_hash = crate::chunk_verdict::column_hash(world, col);
        if held_hash == server_hash {
            return ColumnCheck::Matched;
        }
        let client_hash = crate::chunk_verdict::generated_column_hash(world, biome_gen, col);
        if client_hash == server_hash {
            log::debug!(
                "Local column {col:?} differs from its note only by this client's own writes since it was \
                 generated (held {held_hash:#010x}, generation {client_hash:#010x}); kept"
            );
            return ColumnCheck::Drifted;
        }
        log::warn!(
            "Local column {col:?} does not generate as the server's does (generation {client_hash:#010x}, \
             held {held_hash:#010x}, server {server_hash:#010x}); letting it go and asking for every column to be pushed"
        );
        self.mismatch = Some(ColumnMismatch { cx: col.0, cz: col.1, server_hash, client_hash });
        self.unverified.clear();
        self.checks_ready.clear();
        self.checks_queued.clear();
        self.discard(world, col, true);
        ColumnCheck::Mismatched
    }

    /// B2b fix LOW-2 — call before [`Self::apply`] on pushed packet `pkt`
    /// (after generating a noted column not held yet,
    /// [`Self::generate_before_chunk`]): a chunk pushed into a column whose
    /// check is pending (an overflow resync) replaces part of what the column
    /// is checked by, so the column is checked first, as it stands, whatever
    /// the budget. A column in the evicted store comes back for it, as
    /// `apply` would bring it back. Before the packet counts, so the drop
    /// report of a column let go of predates the push, which the server then
    /// keeps as sent: the push still lands, on a column the server pushes
    /// whole again once it has the switch.
    pub fn check_before_chunk(
        &mut self,
        world: &mut World,
        loaded: &ahash::AHashSet<(i32, i32)>,
        biome_gen: &BiomeGenerator,
        pkt: &ChunkDataPacket,
    ) -> ColumnCheck {
        let col = (pkt.cx, pkt.cz);
        if pkt.compressed_blocks.is_empty() || !self.unverified.contains_key(&col) {
            return ColumnCheck::NotPending;
        }
        if !loaded.contains(&col) {
            world.restore_column(col.0, col.1);
        }
        if !(0..=MAX_CHUNK_Y).any(|cy| world.has_chunk(col.0, cy, col.1)) {
            // Nothing held to check; the push makes it the server's.
            return ColumnCheck::NotPending;
        }
        self.verify_local(world, biome_gen, col)
    }

    /// Phase B2b — the first local column whose generation did not match its
    /// note (`InputPacket::column_mismatch`, sent in every input once set).
    pub fn column_mismatch(&self) -> Option<ColumnMismatch> {
        self.mismatch
    }

    /// Phase B2b — has the server said column `col` is local (and this
    /// client not let go of it since)?
    pub fn is_local(&self, col: (i32, i32)) -> bool {
        self.local.contains(&col)
    }

    /// Phase B2b — the column a server block change `bc` lands in, when it
    /// is one the server said is local that this client has not generated
    /// (nor holds evicted): generate it first — the full load path — then
    /// apply the change, never a stray chunk. `None` otherwise.
    pub fn generate_before(
        &self,
        bc: &crate::protocol::BlockChange,
        loaded: &ahash::AHashSet<(i32, i32)>,
        world: &World,
    ) -> Option<(i32, i32)> {
        self.generate_before_at(bc.x, bc.z, loaded, world)
    }

    /// [`Self::generate_before`] for the block at `(x, z)` (C3b-2: a block
    /// view lands like a change).
    pub fn generate_before_at(
        &self,
        x: i32,
        z: i32,
        loaded: &ahash::AHashSet<(i32, i32)>,
        world: &World,
    ) -> Option<(i32, i32)> {
        let col = crate::chunk_stream::column_of_block(x, z);
        (self.local.contains(&col) && !crate::chunk_stream::remote_change_is_loaded(loaded, world, x, z))
            .then_some(col)
    }

    /// B2b fix D3 (review MEDIUM-1) — must column `pkt` lands in be
    /// generated before the pushed chunk `pkt` is applied? Yes for a chunk
    /// (not a continuation) of a column the server said is local that this
    /// client has neither generated nor holds evicted: an overflow resync can
    /// push one chunk of such a column with no change before it, and applied
    /// alone it would leave the column part-pushed and never generated (a
    /// shaft of void), every later change to its other chunks conjuring a
    /// stray one. Generated first — the full load path — the push overlays
    /// a whole column.
    pub fn generate_before_chunk(
        &self,
        pkt: &ChunkDataPacket,
        loaded: &ahash::AHashSet<(i32, i32)>,
        world: &World,
    ) -> Option<(i32, i32)> {
        let col = (pkt.cx, pkt.cz);
        let cs = crate::chunk::CHUNK_SIZE as i32;
        (!pkt.compressed_blocks.is_empty()
            && self.local.contains(&col)
            && !crate::chunk_stream::remote_change_is_loaded(loaded, world, col.0 * cs, col.1 * cs))
        .then_some(col)
    }

    /// Phase B2b — columns the server said are local that are not loaded:
    /// noted but not generated yet. One whose turn never came before its
    /// joiner moved away must still be let go of (and reported), or it would
    /// stay "local" here while the server forgot it, and on return be
    /// generated before the server decided it again.
    pub fn local_not_loaded(&self, loaded: &ahash::AHashSet<(i32, i32)>) -> ahash::AHashSet<(i32, i32)> {
        self.local.iter().filter(|c| !loaded.contains(*c)).copied().collect()
    }

    /// Phase B2b — the columns the server said are local. Test-only.
    #[cfg(test)]
    pub fn local_columns(&self) -> Vec<(i32, i32)> {
        let mut cols: Vec<(i32, i32)> = self.local.iter().copied().collect();
        cols.sort_unstable();
        cols
    }

    /// Must this client keep column `col` although its streamer would unload
    /// it (B2a verify NEW-1)? Yes for a column the server pushed or said is
    /// local while it lies within `render_distance + UNLOAD_HYSTERESIS` of
    /// the server body's column: that is where the server keeps sending it,
    /// and a rider's client body (whose inputs go out with no movement, so
    /// the server body stays where the ride began) moves away from there.
    /// Letting such columns go made the server hold them off and left them
    /// stale or void when the rider got off. Unloading only: nothing is
    /// loaded or generated round the server body.
    pub fn keeps_near_server_body(&self, col: (i32, i32), render_distance: i32) -> bool {
        let Some(centre) = self.server_centre else { return false };
        let keep = render_distance + crate::chunk_stream::UNLOAD_HYSTERESIS;
        (self.holds_pushed(col) || self.local.contains(&col))
            && (col.0 - centre.0).abs() <= keep
            && (col.1 - centre.1).abs() <= keep
    }

    /// Phase B2b — has the server decided column `col` for this client:
    /// pushed it whole, or said it is local?
    pub fn decided(&self, col: (i32, i32)) -> bool {
        self.column_complete(col) || self.local.contains(&col)
    }

    /// Apply one pushed packet to `world` (see the module docs). A chunk
    /// packet replaces the chunk and its side data; a continuation adds side
    /// data to a chunk still held. Returns whether `world` changed.
    pub fn apply(
        &mut self,
        world: &mut World,
        loaded: &mut ahash::AHashSet<(i32, i32)>,
        registry: &crate::block::BlockRegistry,
        pkt: &ChunkDataPacket,
    ) -> bool {
        // Every packet counts, used or not: the count is a position in the
        // server's stream, not a tally of chunks kept.
        self.applied = self.applied.wrapping_add(1);
        let coord = (pkt.cx, pkt.cy, pkt.cz);
        if !(0..=MAX_CHUNK_Y).contains(&pkt.cy) {
            log::warn!("Pushed chunk {coord:?} is outside the world; ignored");
            return false;
        }
        let col = (pkt.cx, pkt.cz);
        if pkt.compressed_blocks.is_empty() {
            // A continuation of a chunk this client has since let go of is
            // moot: the server pushes the chunk afresh when it is back.
            if !self.pushed.contains(&coord) {
                return false;
            }
            apply_side_data(world, registry, coord, pkt);
            self.queue_relight(col);
            return true;
        }
        let chunk = match crate::protocol::decompress_chunk(&pkt.compressed_blocks)
            .ok()
            .and_then(|raw| Chunk::from_bytes(&raw))
        {
            Some(c) => c,
            None => {
                // B2a verify NEW-2 — the server holds this chunk as sent, so
                // ignoring it left a hole it never fills. Let the whole
                // column go and report it: the server pushes it again (once
                // the hold-off on a column dropped inside its radius ends).
                log::warn!("Pushed chunk {coord:?} does not decode; letting its column go to be pushed again");
                self.discard(world, col, true);
                loaded.remove(&col);
                self.discarded.push(col);
                return true;
            }
        };
        // A column this client evicted (its own edits, kept while out of
        // range) comes back first, so its other chunks are not lost and no
        // block write lands in the evicted copy instead of this one.
        if !loaded.contains(&col) {
            world.restore_column(col.0, col.1);
        }
        // The column now holds server data: nothing left to check it by
        // (the caller checked it first, `check_before_chunk`).
        self.unverified.remove(&col);
        world.insert_chunk(pkt.cx, pkt.cy, pkt.cz, chunk);
        clear_side_data(world, coord);
        apply_side_data(world, registry, coord, pkt);
        rebuild_power_devices(world, coord);
        if self.pushed.insert(coord) {
            *self.pushed_per_column.entry(col).or_insert(0) += 1;
        }
        // Loaded once whole (a column this client generated itself already
        // is): a part-pushed column is never counted loaded, so nothing ever
        // treats a half column at the frontier as complete (review LOW-2/3).
        if self.column_complete(col) {
            loaded.insert(col);
        }
        self.queue_relight(col);
        true
    }

    fn queue_relight(&mut self, col: (i32, i32)) {
        if self.relight_queued.insert(col) {
            self.relight.push_back(col);
        }
    }

    /// Columns [`Self::apply`] discarded since the last call (B2a verify
    /// NEW-2): their meshes must go too.
    pub fn take_discarded(&mut self) -> Vec<(i32, i32)> {
        std::mem::take(&mut self.discarded)
    }

    /// Up to `budget` columns for the light pass and meshing, oldest first.
    /// A column let go of since it was queued is skipped.
    pub fn take_relight(&mut self, budget: usize) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        while out.len() < budget {
            let Some(col) = self.relight.pop_front() else { break };
            self.relight_queued.remove(&col);
            if self.holds_pushed(col) {
                out.push(col);
            }
        }
        out
    }

    /// This client is letting go of pushed or local column `col` (out of
    /// range): discard it — chunks and side data, never into the evicted
    /// store, since the server pushes it afresh or decides it again — and
    /// queue the report to the server. No-op for a column the server told
    /// this client nothing about — one it generated before any verdict
    /// included: that is not in the server's sent-set, so it unloads like a
    /// single-player column (evicted or dropped by the caller) and is never
    /// reported.
    pub fn let_go(&mut self, world: &mut World, col: (i32, i32)) {
        self.discard(world, col, false);
    }

    /// Discard column `col` (chunks, side data, any evicted copy) and queue
    /// its drop report — if the server sent it anything, or `always`.
    fn discard(&mut self, world: &mut World, col: (i32, i32), always: bool) {
        self.unverified.remove(&col);
        let was_local = self.local.remove(&col);
        if self.pushed_per_column.remove(&col).is_none() && !was_local && !always {
            return;
        }
        for cy in 0..=MAX_CHUNK_Y {
            let coord = (col.0, cy, col.1);
            self.pushed.remove(&coord);
            clear_side_data(world, coord);
        }
        world.discard_column(col.0, col.1);
        self.drops.push((ChunkDrop { cx: col.0, cz: col.1, as_of: self.applied }, None));
    }

    /// The drop reports for input number `seq`: up to `max` of the ones the
    /// server has not yet applied, oldest first (the rest wait). Each is
    /// repeated in every input until [`Self::confirm_drops`] retires it.
    pub fn drops_for_input(&mut self, seq: u64, max: usize) -> Vec<ChunkDrop> {
        self.drops
            .iter_mut()
            .take(max)
            .map(|(d, first)| {
                first.get_or_insert(seq);
                *d
            })
            .collect()
    }

    /// The server has applied every input up to `last_acked_input` — and so
    /// read the drop reports of every input before it (the stream is
    /// ordered, and the server reads the reports of every input it receives):
    /// retire those reports.
    pub fn confirm_drops(&mut self, last_acked_input: u64) {
        self.drops.retain(|(_, first)| first.is_none_or(|s| s > last_acked_input));
    }

    /// Drop reports the server has not yet applied. Test-only.
    #[cfg(test)]
    pub fn pending_drops(&self) -> usize {
        self.drops.len()
    }
}

/// Remove the side data (`block_meta`, block entities, face attachments,
/// drying racks — C3b-2-fix) of every cell of chunk `coord`: a push replaces
/// it whole (a rack that stands comes back as its view, after the push).
fn clear_side_data(world: &mut World, coord: ChunkCoord) {
    let racks: Vec<_> = cells_in(&world.drying_racks, coord).into_iter().map(|(_, c, _)| c).collect();
    for c in racks {
        world.drying_racks.remove(&c);
    }
    let meta: Vec<_> = cells_in(&world.block_meta, coord).into_iter().map(|(_, c, _)| c).collect();
    for c in meta {
        world.block_meta.remove(&c);
    }
    let entities: Vec<_> =
        cells_in(&world.block_entities, coord).into_iter().map(|(_, c, _)| c).collect();
    for c in entities {
        world.block_entities.remove(&c);
    }
    let faces: Vec<_> =
        cells_in(&world.face_attachments, coord).into_iter().map(|(_, c, _)| c).collect();
    for c in faces {
        world.face_attachments.remove(&c);
    }
}

/// Write a packet's side data into `world`. Cells outside the chunk are
/// ignored (a malformed packet).
fn apply_side_data(
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    coord: ChunkCoord,
    pkt: &ChunkDataPacket,
) {
    let valid = |cell: u16| usize::from(cell) < crate::chunk::CHUNK_VOLUME;
    for &(cell, m) in &pkt.meta {
        if valid(cell) {
            world.set_meta(cell_pos(coord, cell), m);
        }
    }
    for e in &pkt.entities {
        if !valid(e.cell) {
            continue;
        }
        let pos = cell_pos(coord, e.cell);
        match &e.entity {
            PushedEntity::Sign { text } => {
                let mut sign = crate::sign::SignData::new();
                sign.set_text(text);
                world.insert_sign(pos, sign);
            }
            PushedEntity::ItemFrame { item_kind, item_id, full_item, rotation } => {
                let item = crate::inventory::item_from_wire_full(full_item)
                    .or_else(|| crate::inventory::item_from_ref(*item_kind, *item_id, registry));
                let mut frame = crate::item_frame::ItemFrameData::new();
                frame.item = item.map(|item| crate::item::ItemStack { item, count: 1 });
                frame.rotation = *rotation % crate::item_frame::FRAME_ROTATIONS;
                world.insert_item_frame(pos, frame);
            }
            PushedEntity::Campfire { fuel_ticks, smoke_ticks, smoulder_ticks, raid_warning } => {
                // Burn state only: nothing is cooking on a joiner's copy.
                let fire = crate::campfire::CampfireData {
                    fuel_ticks: *fuel_ticks,
                    smoke_ticks: *smoke_ticks,
                    smoulder_ticks: *smoulder_ticks,
                    raid_warning_active: *raid_warning,
                    ..Default::default()
                };
                world.insert_campfire(pos, fire);
            }
        }
    }
    for a in &pkt.attachments {
        if !valid(a.cell) {
            continue;
        }
        world.set_face_attachment(cell_pos(coord, a.cell), usize::from(a.face), stub_attachment(a.attachment));
    }
}

/// The render stub a joined client holds for a pushed attachment: a
/// blueprint is the server's, so its copy is a body-less Plan of the same
/// develop state (`plan::PlanData::render_stub`).
fn stub_attachment(a: PushedAttachment) -> FaceAttachment {
    match a {
        PushedAttachment::Wallpaper(b) => FaceAttachment::Wallpaper(b),
        PushedAttachment::BlueprintBlank => FaceAttachment::BlueprintBlank,
        PushedAttachment::Blueprint { developed } => {
            FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::render_stub(developed)))
        }
    }
}

/// C3c-3b — apply one streamed face-attachment change
/// (`StateUpdatePacket::attachment_changes`) to a joined client's world, as
/// a push's side data lands: the face now holds the render stub of `att`, or
/// nothing. Returns whether the face changed (the caller remeshes its cell).
/// A face index out of range is ignored (a malformed packet).
pub fn apply_attachment_change(world: &mut World, a: &crate::protocol::AttachmentChange) -> bool {
    let face = usize::from(a.face);
    if face >= 6 {
        return false;
    }
    let pos = (a.x, a.y, a.z);
    match a.att {
        Some(att) => {
            let stub = stub_attachment(att);
            if world.face_attachment_at(pos, face) == Some(&stub) {
                return false;
            }
            world.set_face_attachment(pos, face, stub);
            true
        }
        None => world.remove_face_attachment(pos, face).is_some(),
    }
}

/// Give every power block in chunk `coord` its device entity, from its block
/// id and metadata — exactly what `World::apply_remote_block_change` does per
/// changed cell — and wake it, since this client's (still duplicated) power
/// sim reads the devices. Reads the chunk's own cells (no world lookups).
fn rebuild_power_devices(world: &mut World, coord: ChunkCoord) {
    let Some(chunk) = world.get_chunk(coord.0, coord.1, coord.2) else { return };
    let devices: Vec<_> = chunk
        .non_air_cells()
        .filter_map(|(i, block)| {
            crate::power::device_kind_for_block(block).map(|kind| (cell_pos(coord, i), kind))
        })
        .collect();
    for (pos, kind) in devices {
        let meta = world.meta_at(pos.0, pos.1, pos.2);
        world.insert_power_device(
            pos,
            crate::power::PowerDeviceData::new(kind, crate::meta::facing(meta)),
        );
        crate::power::sync_device_from_meta(world, pos, meta);
        world.mark_dirty(pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::protocol::{self, BlockChange, PushedBlockEntity, PushedFaceAttachment};

    fn reg() -> crate::block::BlockRegistry {
        crate::block::BlockRegistry::new()
    }

    fn packet_of(world: &World, coord: ChunkCoord) -> ChunkDataPacket {
        let bytes = crate::chunk_push::build_chunk_packets(world, coord);
        assert_eq!(bytes.len(), 1);
        let (_, payload) = protocol::deserialize_header(&bytes[0]).unwrap();
        protocol::safe_deserialize(payload).unwrap()
    }

    /// C3b-2 — a block view sits in the world stream where its packet
    /// arrived: after the snapshot pushed before it (whose side data it
    /// updates, so the snapshot can't wipe it), between the changes around
    /// it, and before a later snapshot. It is not a numbered stream packet.
    #[test]
    fn a_view_lands_after_the_snapshot_it_updates() {
        use crate::protocol::{BlockEntityView, BlockViewKind};
        let mut host = World::new();
        host.set_block(1, 1, 1, block::ITEM_FRAME);
        host.insert_item_frame((1, 1, 1), crate::item_frame::ItemFrameData::new());
        let snapshot = packet_of(&host, (0, 0, 0));
        // The server then framed a stone: the view that says so.
        let mut framed = crate::item_frame::ItemFrameData::new();
        framed.try_insert(crate::item::ItemStack::new_block(block::STONE, 1));
        let view = crate::block_views::view_of(&crate::world::BlockEntityData::ItemFrame(framed)).unwrap();
        let view = BlockEntityView { cell: [1, 1, 1], kind: BlockViewKind::ItemFrame, view };
        let shape: Vec<String> = interleave(
            vec![(0, StreamItem::Chunk(snapshot.clone())), (1, StreamItem::View(Box::new(view.clone())))],
            2,
        )
        .iter()
        .map(|s| match s {
            IntakeStep::Chunk(_) => "C".to_string(),
            IntakeStep::View(_) => "V".to_string(),
            IntakeStep::Attachment(_) => "A".to_string(),
            IntakeStep::Local(..) => "L".to_string(),
            IntakeStep::Changes(r) => format!("{}..{}", r.start, r.end),
        })
        .collect();
        assert_eq!(shape, ["C", "0..1", "V", "1..2"]);
        let mut intake = ChunkIntake::default();
        let mut joiner = World::new();
        let mut loaded = ahash::AHashSet::new();
        let steps = intake.plan_deltas(vec![(0, StreamItem::Chunk(snapshot)), (0, StreamItem::View(Box::new(view)))], &[], 4);
        assert_eq!(steps.len(), 2);
        for step in steps {
            match step {
                Delta::Chunk(p) => {
                    intake.apply(&mut joiner, &mut loaded, &reg(), &p);
                }
                Delta::View(v) => {
                    crate::block_views::apply_view(&mut joiner, &reg(), &v);
                }
                _ => unreachable!(),
            }
        }
        let frame = joiner.item_frame_at((1, 1, 1)).expect("the frame");
        assert!(!frame.is_empty(), "the view's stone, not the older snapshot's empty frame");
        assert_eq!(intake.applied(), 1, "the view is not a numbered stream packet");
    }

    /// C3c-3b — a streamed attachment change lands after the snapshot it
    /// updates and replaces what the joiner's copy holds on that face: set,
    /// then cleared; a repeat changes nothing; a bad face is ignored.
    #[test]
    fn a_streamed_attachment_change_lands_after_its_snapshot() {
        use crate::protocol::{AttachmentChange, PushedAttachment};
        let mut host = World::new();
        host.set_block(1, 1, 1, block::STONE);
        host.set_face_attachment((1, 1, 1), 2, FaceAttachment::Wallpaper(block::GLASS));
        let snapshot = packet_of(&host, (0, 0, 0));
        let painted = AttachmentChange { x: 1, y: 1, z: 1, face: 0, att: Some(PushedAttachment::Blueprint { developed: true }) };
        let peeled = AttachmentChange { x: 1, y: 1, z: 1, face: 2, att: None };
        let mut intake = ChunkIntake::default();
        let mut joiner = World::new();
        let mut loaded = ahash::AHashSet::new();
        let steps = intake.plan_deltas(
            vec![(0, StreamItem::Chunk(snapshot)), (0, StreamItem::Attachment(painted)), (0, StreamItem::Attachment(peeled))],
            &[],
            4,
        );
        let mut changed = Vec::new();
        for step in steps {
            match step {
                Delta::Chunk(p) => {
                    intake.apply(&mut joiner, &mut loaded, &reg(), &p);
                }
                Delta::Attachment(a) => changed.push(apply_attachment_change(&mut joiner, &a)),
                _ => unreachable!(),
            }
        }
        assert_eq!(changed, [true, true]);
        assert!(joiner.face_attachment_at((1, 1, 1), 2).is_none(), "the pushed wallpaper, peeled");
        assert!(matches!(
            joiner.face_attachment_at((1, 1, 1), 0),
            Some(FaceAttachment::Blueprint(p)) if p.develop_state.is_developed() && p.cells.is_empty()
        ), "a developed blueprint's render stub");
        assert!(!apply_attachment_change(&mut joiner, &painted), "a repeat changes nothing");
        assert!(!apply_attachment_change(&mut joiner, &peeled));
        assert!(!apply_attachment_change(&mut joiner, &AttachmentChange { face: 6, ..painted }), "no seventh face");
        assert_eq!(intake.applied(), 1, "an attachment change is not a numbered stream packet");
    }

    #[test]
    fn interleave_keeps_each_snapshot_between_the_changes_around_it() {
        let world = World::new();
        let c = |x| StreamItem::Chunk(packet_of(&world, (x, 0, 0)));
        let steps = interleave(
            vec![(0, c(1)), (2, c(2)), (2, StreamItem::Local((7, 7), 0)), (2, c(3)), (5, c(4))],
            6,
        );
        let shape: Vec<String> = steps
            .iter()
            .map(|s| match s {
                IntakeStep::Chunk(p) => format!("C{}", p.cx),
                IntakeStep::Local(col, _) => format!("L{}", col.0),
                IntakeStep::View(v) => format!("V{}", v.cell[0]),
                IntakeStep::Attachment(a) => format!("A{}", a.x),
                IntakeStep::Changes(r) => format!("{}..{}", r.start, r.end),
            })
            .collect();
        assert_eq!(shape, ["C1", "0..2", "C2", "L7", "C3", "2..5", "C4", "5..6"]);
    }

    /// A host world with an edit + side data in chunk (0, 1, 0), and the
    /// joiner's own (different) generation of it.
    fn host_and_joiner() -> (World, World) {
        let biome = crate::biome::BiomeGenerator::new(42);
        let mut host = World::new();
        host.generate_column(0, 0, &biome);
        host.set_block(3, 20, 3, block::GLASS);
        host.set_meta((3, 20, 3), 9);
        let mut sign = crate::sign::SignData::new();
        sign.set_text("hello joiner");
        host.insert_sign((4, 20, 4), sign);
        let mut joiner = World::new();
        joiner.generate_column(0, 0, &crate::biome::BiomeGenerator::new(7));
        joiner.set_meta((5, 21, 5), 3); // stale: not in the host's chunk
        joiner.block_entities.insert(
            (6, 22, 6),
            crate::world::BlockEntityData::Chest(crate::chest::ChestData::default()),
        );
        (host, joiner)
    }

    #[test]
    fn a_push_replaces_the_joiners_own_chunk_and_its_side_data() {
        let (host, mut joiner) = host_and_joiner();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        assert!(intake.apply(&mut joiner, &mut loaded, &reg(), &packet_of(&host, (0, 1, 0))));
        for x in 0..16 {
            for y in 16..32 {
                for z in 0..16 {
                    assert_eq!(joiner.get_block(x, y, z), host.get_block(x, y, z), "({x},{y},{z})");
                }
            }
        }
        assert_eq!(joiner.meta_at(3, 20, 3), 9);
        assert_eq!(joiner.meta_at(5, 21, 5), 0, "stale meta cleared");
        assert!(!joiner.block_entities.contains_key(&(6, 22, 6)), "stale entity cleared");
        assert_eq!(joiner.sign_at((4, 20, 4)).map(|s| s.text.as_str()), Some("hello joiner"));
        assert!(!loaded.contains(&(0, 0)), "one chunk of six: not loaded yet");
        assert!(intake.holds_pushed((0, 0)) && !intake.column_complete((0, 0)));
        assert!(intake.holds_chunk((0, 1, 0)) && !intake.holds_chunk((0, 2, 0)));
        assert_eq!(intake.applied(), 1);
        assert_eq!(intake.take_relight(8), vec![(0, 0)]);
    }

    #[test]
    fn a_part_pushed_column_is_loaded_only_once_whole() {
        // Review LOW-2/3: a half column is never counted loaded.
        let host = World::new();
        let mut joiner = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        for cy in 0..MAX_CHUNK_Y {
            intake.apply(&mut joiner, &mut loaded, &reg(), &packet_of(&host, (4, cy, 4)));
            assert!(!loaded.contains(&(4, 4)), "cy {cy}: part-pushed");
        }
        assert_eq!(intake.part_pushed_columns(&loaded).into_iter().collect::<Vec<_>>(), [(4, 4)]);
        intake.apply(&mut joiner, &mut loaded, &reg(), &packet_of(&host, (4, MAX_CHUNK_Y, 4)));
        assert!(loaded.contains(&(4, 4)) && intake.column_complete((4, 4)), "whole: loaded");
        assert!(intake.part_pushed_columns(&loaded).is_empty());
        // A column this client generated itself is loaded already and stays so.
        let mut own = ahash::AHashSet::from_iter([(9, 9)]);
        intake.apply(&mut joiner, &mut own, &reg(), &packet_of(&host, (9, 0, 9)));
        assert!(own.contains(&(9, 9)));
    }

    #[test]
    fn a_pushed_all_air_chunk_clears_terrain_the_joiner_generated() {
        let host = World::new(); // nothing at (0, 0, 0): dug out
        let mut joiner = World::new();
        joiner.generate_column(0, 0, &crate::biome::BiomeGenerator::new(42));
        assert_ne!(joiner.get_block(8, 0, 8), block::AIR);
        let mut intake = ChunkIntake::default();
        intake.apply(&mut joiner, &mut ahash::AHashSet::new(), &reg(), &packet_of(&host, (0, 0, 0)));
        assert!(joiner.get_chunk(0, 0, 0).is_some_and(Chunk::is_empty));
    }

    #[test]
    fn side_data_survives_a_render_round_trip() {
        let mut host = World::new();
        host.set_block(1, 1, 1, block::STONE);
        let mut frame = crate::item_frame::ItemFrameData::new();
        frame.try_insert(crate::item::ItemStack::new_tool(crate::crafting::Tool::new(
            crate::crafting::ToolType::Pickaxe,
            crate::crafting::ToolMaterial::Iron,
        )));
        frame.rotation = 3;
        host.set_block(2, 2, 2, block::ITEM_FRAME);
        host.insert_item_frame((2, 2, 2), frame.clone());
        host.set_block(3, 3, 3, block::CAMPFIRE);
        let fire = crate::campfire::CampfireData {
            fuel_ticks: 40,
            raid_warning_active: true,
            ..Default::default()
        };
        host.insert_campfire((3, 3, 3), fire);
        let mut plan = crate::plan::PlanData::debug_3x3_stone();
        plan.develop_state = crate::plan::DevelopState::Latent { exposure_ticks: 5 };
        host.set_face_attachment((1, 1, 1), 0, FaceAttachment::Blueprint(Box::new(plan)));
        host.set_face_attachment((1, 1, 1), 2, FaceAttachment::Wallpaper(block::GLASS));

        let mut joiner = World::new();
        ChunkIntake::default().apply(&mut joiner, &mut ahash::AHashSet::new(), &reg(), &packet_of(&host, (0, 0, 0)));
        let got = joiner.item_frame_at((2, 2, 2)).expect("frame");
        assert_eq!(got.rotation, 3);
        assert_eq!(got.item.as_ref().map(|s| &s.item), frame.item.as_ref().map(|s| &s.item));
        let cf = joiner.campfire_at((3, 3, 3)).expect("campfire");
        assert!(cf.raid_warning_active && cf.fuel_ticks == 40);
        assert!(cf.slots.iter().all(|s| s.item.is_none()), "never what is cooking");
        match joiner.face_attachment_at((1, 1, 1), 0) {
            Some(FaceAttachment::Blueprint(p)) => {
                assert!(!p.develop_state.is_developed());
                assert!(p.cells.is_empty(), "a render stub, never the host's plan");
            }
            other => panic!("blueprint stub expected, got {other:?}"),
        }
        assert_eq!(
            joiner.face_attachment_at((1, 1, 1), 2),
            Some(&FaceAttachment::Wallpaper(block::GLASS))
        );
    }

    /// C3b-2-fix (M4) — a push replaces a chunk's side data whole, the
    /// drying racks' side table included: a rack the pushed chunk no longer
    /// has isn't left behind (the rack's view follows the push if it stands).
    #[test]
    fn a_push_clears_the_chunks_drying_racks() {
        let host = World::new();
        let mut joiner = World::new();
        joiner.drying_racks.insert((2, 2, 2), crate::drying_rack::DryingRackData::default());
        joiner.drying_racks.insert((40, 2, 2), crate::drying_rack::DryingRackData::default());
        ChunkIntake::default().apply(&mut joiner, &mut ahash::AHashSet::new(), &reg(), &packet_of(&host, (0, 0, 0)));
        assert!(!joiner.drying_racks.contains_key(&(2, 2, 2)), "the pushed chunk's rack is gone");
        assert!(joiner.drying_racks.contains_key(&(40, 2, 2)), "another chunk's stays");
    }

    #[test]
    fn power_blocks_in_a_push_get_their_devices() {
        let mut host = World::new();
        let lever = block::LEVER;
        let kind = crate::power::device_kind_for_block(lever).expect("a lever is a device");
        host.set_block(5, 5, 5, lever);
        let mut joiner = World::new();
        ChunkIntake::default().apply(&mut joiner, &mut ahash::AHashSet::new(), &reg(), &packet_of(&host, (0, 0, 0)));
        assert_eq!(joiner.power_device_at((5, 5, 5)).map(|d| d.kind), Some(kind));
    }

    #[test]
    fn a_change_after_its_snapshot_wins_and_one_before_it_loses() {
        let mut host = World::new();
        host.set_block(1, 1, 1, block::STONE);
        let snapshot = packet_of(&host, (0, 0, 0));
        let mut joiner = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        let changes = [
            BlockChange::with_meta(1, 1, 1, block::DIRT, 0), // before: older than the snapshot
            BlockChange::with_meta(2, 2, 2, block::GLASS, 0), // after
        ];
        for step in interleave(vec![(1, StreamItem::Chunk(snapshot))], changes.len()) {
            match step {
                IntakeStep::Chunk(p) => {
                    intake.apply(&mut joiner, &mut loaded, &reg(), &p);
                }
                IntakeStep::Local(..) | IntakeStep::View(..) | IntakeStep::Attachment(..) => unreachable!(),
                IntakeStep::Changes(r) => {
                    for bc in &changes[r] {
                        joiner.apply_remote_block_change(bc);
                    }
                }
            }
        }
        assert_eq!(joiner.get_block(1, 1, 1), block::STONE, "the snapshot is newer");
        assert_eq!(joiner.get_block(2, 2, 2), block::GLASS, "the later change applies on top");
    }

    #[test]
    fn letting_go_discards_the_column_and_reports_it() {
        let host = World::new();
        let mut joiner = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        for cy in 0..=MAX_CHUNK_Y {
            intake.apply(&mut joiner, &mut loaded, &reg(), &packet_of(&host, (2, cy, 3)));
        }
        assert!(intake.column_complete((2, 3)));
        joiner.set_meta((40, 3, 50), 4);
        intake.let_go(&mut joiner, (2, 3));
        assert!((0..=MAX_CHUNK_Y).all(|cy| !joiner.has_chunk(2, cy, 3)));
        assert!(!joiner.is_column_evicted(2, 3), "never kept in the evicted store");
        assert_eq!(joiner.meta_at(40, 3, 50), 0);
        assert!(!intake.holds_pushed((2, 3)));
        assert_eq!(intake.drops_for_input(1, 16), vec![ChunkDrop { cx: 2, cz: 3, as_of: 6 }]);
        intake.confirm_drops(1);
        assert!(intake.drops_for_input(2, 16).is_empty());
        // A continuation for it arriving after is counted, not applied.
        let cont = ChunkDataPacket {
            cx: 2,
            cy: 0,
            cz: 3,
            compressed_blocks: Vec::new(),
            meta: vec![(0, 7)],
            entities: Vec::<PushedBlockEntity>::new(),
            attachments: Vec::<PushedFaceAttachment>::new(),
        };
        assert!(!intake.apply(&mut joiner, &mut loaded, &reg(), &cont));
        assert_eq!(intake.applied(), 7);
        assert_eq!(joiner.meta_at(32, 0, 48), 0);
    }

    #[test]
    fn a_note_marks_its_column_local_only_in_a_session_that_expects_notes() {
        // B2b fix D1 replaced "a column inside the note radius waits for its
        // verdict" (`awaits_verdict`, gone): the joiner generates there
        // anyway, and a note confirms the column.
        let mut intake = ChunkIntake::default();
        // B2b review LOW-2: a session that expects no notes (a push-only
        // joiner) counts one, for the ack, and takes nothing from it.
        intake.note_local((1, 1), 7);
        assert_eq!(intake.applied(), 1, "counted");
        assert!(!intake.is_local((1, 1)) && !intake.decided((1, 1)), "but ignored");
        intake.expect_notes(3, (10, 10));
        assert!(intake.server_decides());
        intake.note_local((13, 7), 7);
        assert_eq!(intake.applied(), 2, "a note counts towards the ack");
        assert!(intake.is_local((13, 7)) && intake.decided((13, 7)), "told it is local");
        assert!(!intake.decided((12, 12)), "not told anything yet");
    }

    #[test]
    fn a_column_generated_before_any_verdict_is_let_go_of_without_a_report() {
        // B2b fix D1: the joiner's own speculative generation is not in the
        // server's sent-set, so its unload is none of the server's business.
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        let mut joiner = World::new();
        joiner.generate_column(2, 2, &crate::biome::BiomeGenerator::new(42));
        intake.let_go(&mut joiner, (2, 2));
        assert_eq!(intake.pending_drops(), 0, "never reported");
        assert!(joiner.has_chunk(2, 0, 2), "left to the streamer's own unload (evict or drop)");
    }

    #[test]
    fn a_local_columns_generation_is_checked_against_its_notes_hash() {
        let biome = crate::biome::BiomeGenerator::new(42);
        let mut world = World::new();
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        // Generated before the note (speculatively): checked at the note.
        world.generate_column(1, 0, &biome);
        let good = crate::chunk_verdict::column_hash(&world, (1, 0));
        intake.note_local((1, 0), good);
        assert!(!intake.verify_local(&mut world, &biome, (1, 0)).let_go(), "it matches");
        assert!(intake.is_local((1, 0)) && intake.column_mismatch().is_none());
        // Noted before it is generated: nothing to check until it is.
        intake.note_local((4, 0), 0x600D);
        intake.note_local((2, 0), 0xBAD);
        world.generate_column(2, 0, &biome);
        let got = crate::chunk_verdict::column_hash(&world, (2, 0));
        assert!(intake.verify_local(&mut world, &biome, (2, 0)).let_go(), "a mismatch lets it go");
        assert!(!world.has_chunk(2, 0, 0) && !intake.is_local((2, 0)), "discarded");
        assert_eq!(intake.drops_for_input(1, 8), vec![ChunkDrop { cx: 2, cz: 0, as_of: 3 }], "reported");
        let first = ColumnMismatch { cx: 2, cz: 0, server_hash: 0xBAD, client_hash: got };
        assert_eq!(intake.column_mismatch(), Some(first), "the switch is set");
        // Checked once: a later call (a change landing on it) never re-checks.
        assert!(!intake.verify_local(&mut world, &biome, (1, 0)).let_go());
        // B2b fix LOW-1 (this replaces "every later mismatch is let go of
        // too"): once the switch is set nothing is checked any more — the
        // server pushes every noted column again. A check pending from before
        // is cleared, and a column noted after is just local, kept as it is.
        assert!(!intake.has_pending_check((4, 0)), "cleared at the switch");
        intake.note_local((3, 0), 0xBAD);
        world.generate_column(3, 0, &biome);
        assert_eq!(intake.verify_local(&mut world, &biome, (3, 0)), ColumnCheck::NotPending, "a no-op");
        assert!(world.has_chunk(3, 0, 0) && intake.is_local((3, 0)), "kept");
        assert_eq!(intake.pending_drops(), 1, "nothing more reported");
        // Sticky: the first mismatch is the one kept.
        assert_eq!(intake.column_mismatch(), Some(first));
    }

    #[test]
    fn a_local_column_written_to_before_its_note_is_kept_and_checked() {
        // B2b fix HIGH-1: the joiner generated the column, then wrote to it
        // (its snowfall, its fluids, its player's edit) before the note came.
        // Generation agrees — a scratch generation hashes as the note — so it
        // is no determinism bug: no switch, no report, the column kept.
        let biome = crate::biome::BiomeGenerator::new(42);
        let mut server = World::new();
        server.generate_column(1, 0, &biome);
        let note = crate::chunk_verdict::column_hash(&server, (1, 0));
        let mut world = World::new();
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        world.generate_column(1, 0, &biome);
        world.set_block(20, 90, 4, block::GLASS); // this client's own write
        intake.note_local((1, 0), note);
        assert_eq!(intake.verify_local(&mut world, &biome, (1, 0)), ColumnCheck::Drifted);
        assert!(intake.is_local((1, 0)) && !intake.has_pending_check((1, 0)), "local and checked");
        assert_eq!(world.get_block(20, 90, 4), block::GLASS, "kept as it stands");
        assert!(intake.column_mismatch().is_none(), "no switch");
        assert_eq!(intake.pending_drops(), 0, "nothing reported");
        // A note whose hash is not generation's is still caught, drift or not.
        world.generate_column(2, 0, &biome);
        world.set_block(40, 90, 4, block::GLASS);
        let wrong = crate::chunk_verdict::generated_column_hash(&world, &biome, (2, 0)) ^ 1;
        intake.note_local((2, 0), wrong);
        assert_eq!(intake.verify_local(&mut world, &biome, (2, 0)), ColumnCheck::Mismatched);
        assert!(!world.has_chunk(2, 0, 0) && !intake.is_local((2, 0)), "let go of");
        let m = intake.column_mismatch().expect("the switch is set");
        assert_eq!((m.cx, m.cz, m.server_hash), (2, 0, wrong));
        assert_eq!(m.client_hash, wrong ^ 1, "the generation's hash is reported");
    }

    #[test]
    fn pending_checks_run_within_a_frame_budget_and_a_scratch_counts_against_it() {
        // B2b fix LOW-3.
        let biome = crate::biome::BiomeGenerator::new(42);
        let mut world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        for x in 0..5 {
            world.generate_column(x, 0, &biome);
            loaded.insert((x, 0));
            intake.note_local((x, 0), crate::chunk_verdict::column_hash(&world, (x, 0)));
        }
        // Four queued (once each); (4, 0) is held but never queued.
        for x in 0..4 {
            intake.column_held((x, 0));
        }
        intake.column_held((0, 0));
        assert!(intake.run_checks(&mut world, &biome, &loaded, 2).is_empty());
        assert!(!intake.has_pending_check((0, 0)) && !intake.has_pending_check((1, 0)), "two checked");
        assert!(intake.has_pending_check((2, 0)) && intake.has_pending_check((3, 0)), "the rest wait");
        // A drifted column needs a scratch generation: it spends the frame.
        world.set_block(2 * 16 + 3, 90, 3, block::GLASS);
        intake.run_checks(&mut world, &biome, &loaded, 2);
        assert!(!intake.has_pending_check((2, 0)), "checked (drift, kept)");
        assert!(intake.has_pending_check((3, 0)), "the scratch took the rest of the budget");
        assert_eq!(ColumnCheck::Drifted.cost(), 1 + SCRATCH_CHECK_COST);
        intake.run_checks(&mut world, &biome, &loaded, 2);
        assert!(!intake.has_pending_check((3, 0)));
        assert!(intake.has_pending_check((4, 0)), "never queued: waits for its caller");
        // A queued column no longer held is skipped and stays pending.
        intake.column_held((4, 0));
        loaded.remove(&(4, 0));
        intake.run_checks(&mut world, &biome, &loaded, 8);
        assert!(intake.has_pending_check((4, 0)));
        assert!(intake.column_mismatch().is_none());
    }

    #[test]
    fn a_push_into_a_column_with_a_pending_check_checks_it_first() {
        // B2b fix LOW-2: an overflow resync used to clear the pending check
        // unchecked.
        let biome = crate::biome::BiomeGenerator::new(42);
        let host = World::new();
        let mut world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        // A held column with a true note: checked, then the push lands.
        world.generate_column(1, 0, &biome);
        loaded.insert((1, 0));
        intake.note_local((1, 0), crate::chunk_verdict::column_hash(&world, (1, 0)));
        let pkt = packet_of(&host, (1, 2, 0));
        assert_eq!(intake.check_before_chunk(&mut world, &loaded, &biome, &pkt), ColumnCheck::Matched);
        intake.apply(&mut world, &mut loaded, &reg(), &pkt);
        // An evicted copy (drifted) comes back to be checked.
        world.generate_column(3, 0, &biome);
        let note = crate::chunk_verdict::column_hash(&world, (3, 0));
        world.set_block(3 * 16 + 2, 90, 2, block::GLASS);
        assert!(world.evict_column(3, 0));
        intake.note_local((3, 0), note);
        let pkt = packet_of(&host, (3, 1, 0));
        assert_eq!(intake.check_before_chunk(&mut world, &loaded, &biome, &pkt), ColumnCheck::Drifted);
        // A forged note: let go of before the push counts, so the drop
        // report predates it and the server keeps the push as sent.
        world.generate_column(5, 0, &biome);
        loaded.insert((5, 0));
        intake.note_local((5, 0), crate::chunk_verdict::column_hash(&world, (5, 0)) ^ 1);
        let before = intake.applied();
        let pkt = packet_of(&host, (5, 0, 0));
        assert_eq!(intake.check_before_chunk(&mut world, &loaded, &biome, &pkt), ColumnCheck::Mismatched);
        assert_eq!(intake.drops_for_input(1, 8), vec![ChunkDrop { cx: 5, cz: 0, as_of: before }]);
        // A continuation, or a column with no check pending, checks nothing.
        let cont = ChunkDataPacket { compressed_blocks: Vec::new(), ..packet_of(&host, (1, 2, 0)) };
        assert_eq!(intake.check_before_chunk(&mut world, &loaded, &biome, &cont), ColumnCheck::NotPending);
    }

    #[test]
    fn a_pushed_chunk_for_a_local_column_not_generated_yet_generates_it_first() {
        // B2b fix D3 (review MEDIUM-1).
        let host = World::new();
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        let world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let pkt = packet_of(&host, (1, 2, -1));
        assert_eq!(intake.generate_before_chunk(&pkt, &loaded, &world), None, "not local");
        intake.note_local((1, -1), 0);
        assert_eq!(intake.generate_before_chunk(&pkt, &loaded, &world), Some((1, -1)));
        let cont = ChunkDataPacket { compressed_blocks: Vec::new(), ..pkt.clone() };
        assert_eq!(intake.generate_before_chunk(&cont, &loaded, &world), None, "a continuation adds side data only");
        loaded.insert((1, -1));
        assert_eq!(intake.generate_before_chunk(&pkt, &loaded, &world), None, "already generated");
    }

    #[test]
    fn a_change_for_a_local_column_not_generated_yet_generates_it_first() {
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        let world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let bc = BlockChange::with_meta(20, 70, -5, block::GLASS, 0); // column (1, -1)
        assert_eq!(intake.generate_before(&bc, &loaded, &world), None, "not local: not ours to make");
        intake.note_local((1, -1), 0);
        assert_eq!(intake.generate_before(&bc, &loaded, &world), Some((1, -1)));
        loaded.insert((1, -1));
        assert_eq!(intake.generate_before(&bc, &loaded, &world), None, "already generated");
    }

    #[test]
    fn letting_go_of_a_local_column_discards_it_and_reports_it() {
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        let mut joiner = World::new();
        let biome = crate::biome::BiomeGenerator::new(42);
        intake.note_local((1, 2), 0);
        joiner.generate_column(1, 2, &biome);
        joiner.set_block(20, 90, 40, block::GLASS); // a server change applied on top
        assert!(joiner.evict_column(1, 2), "edited: the streamer's stream-out keeps it");
        intake.let_go(&mut joiner, (1, 2));
        assert!(!joiner.is_column_evicted(1, 2), "discarded, never restored stale");
        assert!(!intake.is_local((1, 2)));
        assert_eq!(intake.drops_for_input(1, 16), vec![ChunkDrop { cx: 1, cz: 2, as_of: 1 }]);
        // A column the server told it nothing about is no one's business.
        intake.let_go(&mut joiner, (5, 5));
        assert_eq!(intake.pending_drops(), 1);
    }

    #[test]
    fn a_push_into_an_evicted_column_brings_the_column_back_first() {
        let mut joiner = World::new();
        joiner.generate_column(0, 0, &crate::biome::BiomeGenerator::new(42));
        joiner.set_block(1, 70, 1, block::GLASS); // the joiner's own edit: kept on eviction
        assert!(joiner.evict_column(0, 0));
        let host = World::new();
        let mut loaded = ahash::AHashSet::new();
        ChunkIntake::default().apply(&mut joiner, &mut loaded, &reg(), &packet_of(&host, (0, 0, 0)));
        assert!(!joiner.is_column_evicted(0, 0));
        joiner.set_block(1, 1, 1, block::STONE);
        assert_eq!(joiner.get_chunk(0, 0, 0).map(|c| c.get(1, 1, 1)), Some(block::STONE), "writes land in the live chunk");
        assert_eq!(joiner.get_block(1, 70, 1), block::GLASS, "the column's other chunks are back");
    }

    #[test]
    fn drop_reports_go_out_in_bounded_batches() {
        let mut intake = ChunkIntake::default();
        let mut world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let host = World::new();
        for x in 0..5 {
            intake.apply(&mut world, &mut loaded, &reg(), &packet_of(&host, (x, 0, 0)));
            intake.let_go(&mut world, (x, 0));
        }
        assert_eq!(intake.drops_for_input(1, 3).len(), 3);
        assert_eq!(intake.drops_for_input(2, 3).len(), 3, "repeated until applied");
        intake.confirm_drops(1);
        assert_eq!(intake.drops_for_input(3, 3).len(), 2, "the rest");
    }

    #[test]
    fn a_drop_report_repeats_until_the_server_has_applied_an_input_carrying_it() {
        // Review HIGH-2: a report the server never read must not be lost.
        let host = World::new();
        let mut world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        intake.apply(&mut world, &mut loaded, &reg(), &packet_of(&host, (1, 0, 1)));
        intake.let_go(&mut world, (1, 1));
        let d = ChunkDrop { cx: 1, cz: 1, as_of: 1 };
        assert_eq!(intake.drops_for_input(10, 8), vec![d], "first carried by input 10");
        // The server acknowledges input 9 only: 10 may not have been read.
        intake.confirm_drops(9);
        assert_eq!(intake.drops_for_input(11, 8), vec![d], "so it goes again");
        intake.confirm_drops(10);
        assert!(intake.drops_for_input(12, 8).is_empty(), "applied: retired");
        assert_eq!(intake.pending_drops(), 0);
    }

    #[test]
    fn an_undecodable_chunk_packet_still_counts() {
        // Review LOW-1: the server numbered it; every later ack and `as_of`
        // must too.
        let host = World::new();
        let mut world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        intake.apply(&mut world, &mut loaded, &reg(), &packet_of(&host, (0, 0, 0)));
        intake.count_undecodable(1);
        intake.apply(&mut world, &mut loaded, &reg(), &packet_of(&host, (0, 1, 0)));
        assert_eq!(intake.applied(), 3);
        intake.let_go(&mut world, (0, 0));
        assert_eq!(intake.drops_for_input(1, 8), vec![ChunkDrop { cx: 0, cz: 0, as_of: 3 }]);
    }

    #[test]
    fn a_chunk_that_does_not_decode_gives_up_its_column_to_be_pushed_again() {
        // B2a verify NEW-2: counted but never inserted, it left the column
        // part-pushed — never generated, never complete — for as long as the
        // joiner stayed near it.
        let host = World::new();
        let mut world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        intake.apply(&mut world, &mut loaded, &reg(), &packet_of(&host, (4, 0, 4)));
        let mut bad = packet_of(&host, (4, 1, 4));
        bad.compressed_blocks = vec![0xFF; 12];
        assert!(intake.apply(&mut world, &mut loaded, &reg(), &bad));
        assert!(!intake.holds_pushed((4, 4)), "the column is given up whole");
        assert!(!world.has_chunk(4, 0, 4));
        assert_eq!(intake.take_discarded(), vec![(4, 4)], "its meshes go too");
        assert_eq!(
            intake.drops_for_input(1, 8),
            vec![ChunkDrop { cx: 4, cz: 4, as_of: 2 }],
            "reported, as of the bad packet"
        );
        // Even the first chunk of a column it holds nothing of.
        intake.confirm_drops(1);
        let mut first = packet_of(&host, (9, 0, 9));
        first.compressed_blocks = vec![1, 2, 3];
        intake.apply(&mut world, &mut loaded, &reg(), &first);
        assert_eq!(intake.drops_for_input(2, 8), vec![ChunkDrop { cx: 9, cz: 9, as_of: 3 }]);
    }

    /// A change at the origin cell of column `(cx, 0)`.
    fn change_in(cx: i32) -> protocol::BlockChange {
        protocol::BlockChange::with_meta(cx * 16, 70, 0, block::STONE, 0)
    }

    /// Which columns a planned frame's changes sit in, in order.
    fn change_cols(run: &[Delta]) -> Vec<i32> {
        run.iter()
            .filter_map(|d| match d {
                Delta::Change(bc) => Some(bc.x.div_euclid(16)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_burst_of_forced_checks_is_capped_a_frame_and_the_rest_waits_in_order() {
        // B2b fix-2 N3: five noted columns, a change for each, then a second
        // change for the first: with a cap of 2 a frame, the third column's
        // change and everything after it wait, and nothing overtakes it.
        let biome = crate::biome::BiomeGenerator::new(42);
        let mut world = World::new();
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        let empty = crate::chunk_verdict::column_hash(&world, (0, 0));
        for cx in 1..=5 {
            intake.note_local((cx, 0), empty);
        }
        let changes: Vec<_> = [1, 2, 3, 4, 5, 1].into_iter().map(change_in).collect();
        let frame1 = intake.plan_deltas(vec![], &changes, 2);
        assert_eq!(change_cols(&frame1), vec![1, 2], "two columns' forced checks, then it holds");
        assert_eq!(intake.carried_len(), 4);
        // The caller runs the checks of what it was given.
        for cx in [1, 2] {
            assert_eq!(intake.verify_local(&mut world, &biome, (cx, 0)), ColumnCheck::Matched);
        }
        // Next frame: the held steps come first, ahead of anything that arrived since.
        let later = vec![change_in(9)];
        let frame2 = intake.plan_deltas(vec![], &later, 2);
        assert_eq!(change_cols(&frame2), vec![3, 4], "the held steps run first, in order");
        assert_eq!(intake.carried_len(), 3, "col 5, col 1 again and the new change for col 9");
        for cx in [3, 4] {
            intake.verify_local(&mut world, &biome, (cx, 0));
        }
        let frame3 = intake.plan_deltas(vec![], &[], 2);
        assert_eq!(change_cols(&frame3), vec![5, 1, 9], "col 1 is checked already: it and col 9 are not forced");
        assert_eq!(intake.carried_len(), 0, "drained");
    }

    #[test]
    fn the_first_forced_check_of_a_frame_always_runs_whatever_the_cap() {
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        intake.note_local((1, 0), 1);
        intake.note_local((2, 0), 2);
        let changes = vec![change_in(1), change_in(2)];
        let run = intake.plan_deltas(vec![], &changes, 0);
        assert_eq!(change_cols(&run), vec![1], "progress with a cap of 0: one a frame");
        assert_eq!(intake.carried_len(), 1);
    }

    #[test]
    fn only_a_pending_check_counts_against_the_cap() {
        let world = World::new();
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        intake.note_local((1, 0), 1);
        // Many changes in columns that were never noted, and several in the
        // noted one: one forced check in all.
        let mut changes = vec![change_in(1), change_in(1), change_in(1)];
        changes.extend((10..30).map(change_in));
        let run = intake.plan_deltas(vec![], &changes, 1);
        assert_eq!(run.len(), changes.len(), "nothing held");
        assert_eq!(intake.carried_len(), 0);
        // A pushed chunk for a pending column counts like a change; a note
        // earlier in the same frame makes its column pending; one with
        // nothing to hash (a continuation packet) is never a check.
        intake.note_local((2, 0), 2);
        let mut cont = packet_of(&world, (2, 0, 0));
        cont.compressed_blocks.clear();
        let chunks = vec![
            (0, StreamItem::Chunk(packet_of(&world, (1, 0, 0)))),
            (0, StreamItem::Chunk(cont)),
            (0, StreamItem::Local((3, 0), 3)),
            (0, StreamItem::Chunk(packet_of(&world, (3, 0, 0)))),
        ];
        let run = intake.plan_deltas(chunks, &[], 1);
        assert_eq!(run.len(), 3, "the push for col 1, the empty packet and the note run");
        assert_eq!(intake.carried_len(), 1, "the push for col 3 would force a second check: held");
    }

    #[test]
    fn no_check_is_forced_once_the_switch_is_set() {
        let biome = crate::biome::BiomeGenerator::new(42);
        let mut world = World::new();
        world.generate_column(1, 0, &biome);
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        intake.note_local((1, 0), 0xBAD);
        intake.note_local((2, 0), 0xBAD);
        assert!(intake.verify_local(&mut world, &biome, (1, 0)).let_go());
        let changes: Vec<_> = (2..12).map(change_in).collect();
        let run = intake.plan_deltas(vec![], &changes, 1);
        assert_eq!(run.len(), 10, "no column is checked any more, so none is held");
    }

    /// Measurement (B2a review LOW-6): what `clear_side_data` costs per
    /// pushed chunk when every side table is past a chunk's 4,096 cells (a
    /// mature, heavily built world: the probing case, the worst there is).
    /// `cargo test --lib measure_clear_side_data -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement"]
    fn measure_clear_side_data() {
        let mut world = World::new();
        let mut n = 0u32;
        'fill: for x in -64..64 {
            for z in -64..64 {
                for y in [40, 41, 70] {
                    world.block_meta.insert((x, y, z), 3);
                    world.face_attachments.insert((x, y, z), {
                        let mut f: crate::world::FaceAttachments = Default::default();
                        f[0] = Some(FaceAttachment::Wallpaper(block::GLASS));
                        f
                    });
                    world.insert_sign((x, y + 1, z), crate::sign::SignData::new());
                    n += 1;
                    if n == 20_000 {
                        break 'fill;
                    }
                }
            }
        }
        let coords: Vec<ChunkCoord> =
            (-4..4).flat_map(|x| (0..=MAX_CHUNK_Y).map(move |y| (x, y, x))).collect();
        let t = web_time::Instant::now();
        let rounds = 20;
        for _ in 0..rounds {
            for &c in &coords {
                clear_side_data(&mut world, c);
            }
        }
        let per = t.elapsed() / (rounds * coords.len() as u32);
        println!(
            "clear_side_data: {:.3} ms per chunk with {} meta / {} entities / {} faces",
            per.as_secs_f64() * 1e3,
            world.block_meta.len(),
            world.block_entities.len(),
            world.face_attachments.len()
        );
    }

    #[test]
    fn a_local_column_never_generated_is_still_one_to_let_go_of() {
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        intake.note_local((3, 3), 0);
        intake.note_local((1, 1), 0);
        let mut loaded = ahash::AHashSet::new();
        loaded.insert((1, 1));
        assert_eq!(intake.local_not_loaded(&loaded).into_iter().collect::<Vec<_>>(), vec![(3, 3)]);
        intake.let_go(&mut World::new(), (3, 3));
        assert_eq!(intake.pending_drops(), 1, "reported, so the server decides it again");
        assert!(!intake.is_local((3, 3)) && !intake.decided((3, 3)), "and it is undecided again");
    }

    #[test]
    fn pushed_and_local_columns_near_the_server_body_are_kept() {
        // B2a verify NEW-1: a rider's server body stays where the ride began.
        let host = World::new();
        let mut world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        for cy in 0..=MAX_CHUNK_Y {
            intake.apply(&mut world, &mut loaded, &reg(), &packet_of(&host, (5, cy, 0)));
        }
        intake.note_local((-6, 6), 0);
        let rd = 4;
        assert!(intake.keeps_near_server_body((5, 0), rd), "pushed, within 4 + 2");
        assert!(intake.keeps_near_server_body((-6, 6), rd), "local, within 4 + 2");
        assert!(!intake.keeps_near_server_body((1, 1), rd), "not the server's to keep");
        intake.set_server_centre((20, 0));
        assert!(!intake.keeps_near_server_body((5, 0), rd), "the body moved on: free to go");
    }
}

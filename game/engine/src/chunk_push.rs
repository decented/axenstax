//! Server → joiner chunk push (Phase B2a; Spec 04 §4.1 "Chunk push").
//!
//! A joiner stands on the HOST's world: the server sends it every chunk round
//! its body as real data, so builds and edits it never saw arrive as they are.
//! Chunks travel directly host → joiner on the game connection — no relay, no
//! third party (red line 2).
//!
//! **Per client** ([`ClientChunkPush`], one per remote slot on
//! `HostedServer`, reset whenever the slot is attached or released):
//!
//! - **What to push.** Every chunk of every column within
//!   `R = min(joiner render distance, server limit)` (Chebyshev, columns) of
//!   the joiner's SERVER body (S1: the authoritative position), nearest
//!   first, all six `cy` of a column (an absent chunk goes as an explicit
//!   all-air chunk: that is how a dug-out chunk reaches the joiner). The
//!   render distance is the joiner's CURRENT one: its `JoinRequest` gives the
//!   first, every `InputPacket` the latest, and `R` follows it. Only columns
//!   the server has loaded: the rest wait for the server's streamer. A column
//!   is planned whole — all its unsent chunks in one go — so a joiner is never
//!   left holding half a column at the frontier. The spawn ring (the 3×3
//!   round the body) goes before anything else and outside the credit window
//!   — a joiner's loading screen waits for it, and sends no acknowledgement
//!   until it has it.
//! - **The sent-set.** A chunk is "sent" from the moment its push is queued.
//!   The server's block changes reach a client ONLY for chunks in its
//!   sent-set (`HostedServer::broadcast_state`): a change to a chunk not yet
//!   sent is dropped for that client — the push carries it. A tick's pushes
//!   are planned AFTER its deltas are queued, from the world those deltas
//!   are already in: a change this tick to a chunk first queued this tick is
//!   filtered and in the snapshot; one to a chunk sent earlier rides after
//!   that chunk's snapshot. Either way the client ends up with the latest
//!   state, and a tick's deltas never wait behind its own new chunks.
//! - **Pacing.** Pushes are queued on the client's `state_outbox` FIFO, in
//!   line with its deltas and inside its per-tick byte budget, never more
//!   than about one tick's budget ahead ([`QUEUE_AHEAD_BYTES`]) plus the rest
//!   of the column that passed it (columns go whole), and a credit
//!   window bounds what is in flight: at most [`CHUNK_WINDOW_PACKETS`] packets
//!   and [`CHUNK_WINDOW_BYTES`] bytes not yet acknowledged. The client
//!   acknowledges cumulatively, piggybacked on its `InputPacket`
//!   (`chunk_ack`), so a slow link is never flooded (the transport closes a
//!   client with 8 MiB queued). The window is checked between columns, so
//!   one column may take it past its bound. The spawn ring is pushed
//!   whatever the window, so a loading joiner — which sends no input yet —
//!   still gets the ground it needs, however heavy (B2a review LOW-4).
//!   Acknowledgement, drops and render distance are read from EVERY input,
//!   one the server otherwise discards over its per-tick packet budget
//!   included (review HIGH-2).
//! - **Letting go.** The client reports every column it discards
//!   (`InputPacket.chunk_drops`, with its `chunk_ack` count at the time),
//!   repeating the report in every input until the server has applied an
//!   input that carried it (`last_acked_input`); `as_of` makes a repeat
//!   harmless. Those chunks leave the sent-set: their changes stop and any
//!   resync of them is cancelled. A column dropped while still inside `R` is
//!   not pushed again until it has left `R` and come back (review MEDIUM-1:
//!   no push → unload → drop → push churn, whatever the client does). As a
//!   backstop the server also forgets chunks far beyond anything the client
//!   keeps ([`FORGET_SLACK`]); the client then holds a stale copy until it
//!   comes back in range and gets a fresh one.
//! - **Resync.** A client whose outbox overflowed (`state_outbox`) has its
//!   dropped chunks pushed again whole, ahead of new ones — those still inside
//!   `R`; one outside it is pushed again when its column is back in range.
//!
//! **Mode** (`--chunk-sync`, [`ChunkSync`]): `touched` (Phase B2b, the
//! default) pushes a column only when it differs from generation
//! (`chunk_verdict`) and sends every other column in range as one
//! `ColumnLocal` note, numbered and acknowledged like a push, which the
//! joiner answers by generating the column itself; a column with no verdict
//! yet waits. `all` pushes every chunk in range (B2a). A joiner whose
//! terrain generator differs from the host's (`ServerPlayer::
//! worldgen_mismatch`) always gets everything, and so does every joiner of
//! an owning (`--no-lend`) host (`HostedServer::sends_notes`).
//!
//! **What a push carries** ([`build_chunk_packets`]): the blocks and the
//! player-placed mask (`Chunk::as_bytes`, LZ4), the chunk's `block_meta`, the
//! render-visible block entities (sign text, item-frame item, campfire burn
//! state — never a container's contents, an escrow or a plan) and face
//! attachments as render stubs. Not carried: drying racks, plots, rigs,
//! waypoints and other world-level tables (exhibits ride `JoinAccept`).
//!
//! **Anti-X-ray is NOT applied** (`anti_xray.rs` stays unwired). The seam is
//! [`build_chunk_packets`]: obfuscating buried ore there is one pass over the
//! chunk's cells before `as_bytes`. It only hides anything once the seed
//! stops shipping to joiners (they regenerate natural ore from it), which is
//! an owner decision.
//!
//! **Where the world is read.** Only from `HostedServer::tick`, which a
//! lending host runs inside its lend window (`sim_lend::LentSim`): the world
//! read here is the host client's own.
// Reached only through a hosted server with remote players, which the web
// build never has.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::chunk::{Chunk, CHUNK_SIZE, CHUNK_VOLUME};
use crate::protocol::{
    self, ChunkDataPacket, ChunkDrop, PushedAttachment, PushedBlockEntity, PushedEntity,
    PushedFaceAttachment,
};
use crate::state_outbox::{chunk_of_cell, ChunkCoord, CHUNK_PACKET_MAX_BYTES};
use crate::world::{BlockEntityData, FaceAttachment, World, MAX_CHUNK_Y};

/// Most chunk packets in flight (queued or sent, not yet acknowledged) to one
/// client. At least the spawn ring (3×3 columns × 6 chunks = 54), which a
/// loading joiner must get before it sends any acknowledgement.
pub const CHUNK_WINDOW_PACKETS: u32 = 64;

/// Most chunk-packet bytes in flight to one client. A terrain chunk is about
/// 2.3 KB (max about 4.4 KB, measured 2026-10-07), so the packet bound is the
/// one that bites; this one catches chunks with heavy side data (signs,
/// wallpaper).
pub const CHUNK_WINDOW_BYTES: usize = 512 * 1024;

/// Most queued bytes (deltas and pushes) a client's outbox may hold before
/// this tick's pushes are planned: about one tick's budget, so the next
/// tick's deltas wait behind at most that much chunk data plus the rest of
/// the one column that passed it (a column is planned whole; a heavy one —
/// signs, wallpaper — can hold them a few ticks).
pub const QUEUE_AHEAD_BYTES: usize = crate::state_outbox::CLIENT_TICK_BUDGET_BYTES;

/// The ring round the joiner's column (Chebyshev) pushed before anything
/// farther: its loading screen waits for these columns.
pub const SPAWN_RING_RADIUS: i32 = 1;

/// Columns beyond anything a client keeps (its render distance plus the
/// client's own unload slack) at which the server forgets a chunk it sent
/// without being told — a backstop for a client that never reports drops.
pub const FORGET_SLACK: i32 = 4;

/// `--chunk-sync`: which chunks a joiner is pushed (Spec 04 §4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChunkSync {
    /// Only columns that differ from generation (`chunk_verdict`); every
    /// other column in range is a "local" note the joiner generates itself
    /// (Phase B2b; the default).
    #[default]
    Touched,
    /// Every chunk in range (B2a).
    All,
}

impl ChunkSync {
    /// Parse a `--chunk-sync` value.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "touched" => Ok(Self::Touched),
            "all" => Ok(Self::All),
            other => Err(format!("unknown --chunk-sync value `{other}` (expected `touched` or `all`)")),
        }
    }

    /// Does a joiner get every chunk in range? Always when its terrain
    /// generator differs from the host's, whatever the mode.
    pub fn pushes_everything(self, worldgen_mismatch: bool) -> bool {
        worldgen_mismatch || self == Self::All
    }
}

/// The mode a host client was started with (`--chunk-sync`, set once by
/// `lib.rs::run`); a hosted server starts with it. The dedicated server sets
/// its own (`server_main`).
static CHUNK_SYNC: std::sync::OnceLock<ChunkSync> = std::sync::OnceLock::new();

/// The `--chunk-sync` mode this process was started with (`touched` unless set).
pub fn chunk_sync() -> ChunkSync {
    CHUNK_SYNC.get().copied().unwrap_or_default()
}

/// Read `--chunk-sync <mode>` (or `AXENSTAX_CHUNK_SYNC`) from a host
/// client's arguments and record it. An unusable value is logged and the
/// default (`touched`) kept.
#[cfg(not(target_arch = "wasm32"))]
pub fn configure_from_args(args: &[String]) {
    let value = args
        .iter()
        .position(|a| a == "--chunk-sync")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| std::env::var("AXENSTAX_CHUNK_SYNC").ok().filter(|v| !v.is_empty()));
    let Some(value) = value else { return };
    match ChunkSync::parse(&value) {
        Ok(mode) => {
            let _ = CHUNK_SYNC.set(mode);
        }
        Err(why) => log::warn!("{why}; keeping --chunk-sync touched"),
    }
}

/// One remote client's chunk-push state. See the module docs.
#[derive(Debug, Default)]
pub struct ClientChunkPush {
    /// The client's current render distance (`0` = never said), clamped to
    /// [`crate::graphics_settings::RENDER_DISTANCE_MAX`].
    render_distance: i32,
    /// Chunks this client holds as pushed → the number of the first packet
    /// of their latest push.
    sent: HashMap<ChunkCoord, u32>,
    /// Chunk packets queued for this client so far (the last one's number).
    pushed: u32,
    /// Packets the client has acknowledged (cumulative).
    acked: u32,
    /// `(number, bytes)` of every packet not yet acknowledged, oldest first.
    in_flight: VecDeque<(u32, usize)>,
    in_flight_bytes: usize,
    /// Chunks to push again whole (an outbox overflow dropped their changes),
    /// before any new chunk.
    resync: BTreeSet<ChunkCoord>,
    /// Columns the client let go of that have not been outside the push
    /// radius since: not pushed again until they have (review MEDIUM-1).
    held_off: HashSet<(i32, i32)>,
}

/// A render distance from the wire: `0` stays "not said", anything else is
/// clamped to what a client can ask for (a huge one would also switch off
/// the forget backstop).
fn wire_render_distance(rd: u8) -> i32 {
    i32::from(rd).min(crate::graphics_settings::RENDER_DISTANCE_MAX)
}

impl ClientChunkPush {
    /// A fresh client that announced `render_distance` columns.
    pub fn new(render_distance: u8) -> Self {
        Self { render_distance: wire_render_distance(render_distance), ..Self::default() }
    }

    /// The client's current render distance, from its latest input (`0` =
    /// unchanged). The push radius follows it from the next plan.
    pub fn set_render_distance(&mut self, render_distance: u8) {
        if render_distance != 0 {
            self.render_distance = wire_render_distance(render_distance);
        }
    }

    /// Has this client been sent chunk `c` (and not let go of it)?
    pub fn has_sent(&self, c: ChunkCoord) -> bool {
        self.sent.contains_key(&c)
    }

    /// How many chunks this client holds as pushed. Test-only.
    #[cfg(test)]
    pub fn sent_len(&self) -> usize {
        self.sent.len()
    }

    /// Packets the client has acknowledged. Test-only.
    #[cfg(test)]
    pub fn acked(&self) -> u32 {
        self.acked
    }

    /// Does every column with a chunk in the sent-set have all of them
    /// there? Test-only (review LOW-3: never half a column).
    #[cfg(test)]
    pub fn sent_columns_are_whole(&self) -> bool {
        let mut per_column: HashMap<(i32, i32), i32> = HashMap::new();
        for &(cx, _, cz) in self.sent.keys() {
            *per_column.entry((cx, cz)).or_default() += 1;
        }
        per_column.values().all(|&n| n == MAX_CHUNK_Y + 1)
    }

    /// Chunk packets queued so far.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn pushed(&self) -> u32 {
        self.pushed
    }

    /// Test-only: the client already holds chunk `c` (as one that walked
    /// there would), pushed before anything this test can see.
    #[cfg(test)]
    pub(crate) fn hold_for_test(&mut self, c: ChunkCoord) {
        self.sent.insert(c, 0);
    }

    /// The push radius: the client's render distance, at most `limit`.
    pub fn radius(&self, limit: i32) -> i32 {
        if self.render_distance > 0 { self.render_distance.min(limit) } else { limit }
    }

    /// The client has taken in `chunk_ack` packets (cumulative). Clamped to
    /// what was queued: a client can't open its window past that.
    pub fn ack(&mut self, chunk_ack: u32) {
        let ack = chunk_ack.min(self.pushed);
        if ack <= self.acked {
            return;
        }
        self.acked = ack;
        while let Some(&(n, bytes)) = self.in_flight.front() {
            if n > ack {
                break;
            }
            self.in_flight.pop_front();
            self.in_flight_bytes -= bytes;
        }
    }

    /// The client let go of these columns: their chunks leave the sent-set
    /// unless pushed again after the client's `as_of` count (that copy it
    /// holds), and any resync of them is cancelled (the client holds nothing
    /// newer to bring up to date). A column whose chunks this took out is held
    /// off until it has been outside the push radius ([`Self::plan`]).
    /// Idempotent: a repeated report changes nothing more.
    pub fn drop_columns(&mut self, drops: &[ChunkDrop]) {
        for d in drops.iter().take(protocol::MAX_CHUNK_DROPS_PER_INPUT) {
            let mut took = false;
            for cy in 0..=MAX_CHUNK_Y {
                let c = (d.cx, cy, d.cz);
                self.resync.remove(&c);
                if self.sent.get(&c).is_some_and(|&first| first <= d.as_of) {
                    self.sent.remove(&c);
                    took = true;
                }
            }
            if took {
                self.held_off.insert((d.cx, d.cz));
            }
        }
    }

    /// The client's outbox dropped these chunks' changes: push them again,
    /// first. A chunk never sent needs nothing — its push will carry it.
    pub fn request_resync(&mut self, chunks: impl IntoIterator<Item = ChunkCoord>) {
        for c in chunks {
            if self.sent.remove(&c).is_some() {
                self.resync.insert(c);
            }
        }
    }

    /// May another packet be queued outside the spawn ring?
    fn window_open(&self) -> bool {
        self.pushed - self.acked < CHUNK_WINDOW_PACKETS && self.in_flight_bytes < CHUNK_WINDOW_BYTES
    }

    /// Backstop: forget chunks far beyond anything the client keeps round
    /// `centre` (it would have reported them; see the module docs).
    fn forget_far(&mut self, centre: (i32, i32), limit: i32) {
        let keeps = self.radius(limit).max(if self.render_distance > 0 {
            self.render_distance
        } else {
            crate::graphics_settings::RENDER_DISTANCE_MAX
        });
        let reach = keeps + crate::chunk_stream::UNLOAD_HYSTERESIS + FORGET_SLACK;
        let near = |&(cx, _, cz): &ChunkCoord| {
            (cx - centre.0).abs() <= reach && (cz - centre.1).abs() <= reach
        };
        self.sent.retain(|c, _| near(c));
        self.resync.retain(near);
    }

    /// Record one queued push of `coord` and number its packets.
    fn record(&mut self, coord: ChunkCoord, packets: &[Vec<u8>]) {
        self.sent.insert(coord, self.pushed + 1);
        for p in packets {
            self.pushed += 1;
            self.in_flight.push_back((self.pushed, p.len()));
            self.in_flight_bytes += p.len();
        }
    }

    /// Record one queued "column is local" note (Phase B2b): every chunk of
    /// the column counts as sent, from the note's number — the joiner holds
    /// its own generation of each, so their changes reach it, a drop report
    /// takes them out, and the note is numbered and acknowledged like a push.
    fn record_local(&mut self, col: (i32, i32), packet: &[u8]) {
        let number = self.pushed + 1;
        for cy in 0..=MAX_CHUNK_Y {
            self.sent.insert((col.0, cy, col.1), number);
        }
        self.pushed = number;
        self.in_flight.push_back((number, packet.len()));
        self.in_flight_bytes += packet.len();
    }

    /// Build this tick's pushes for a client whose server body stands in
    /// column `centre`: resyncs first (those inside the radius), then the
    /// unsent chunks of each column within [`Self::radius`], nearest column
    /// first, a column at a time and always whole. A column starts only while
    /// fewer than `room` bytes are planned and — outside the spawn ring — the
    /// window is open; once started it finishes, so the last column may pass
    /// both. `loaded` is the server's loaded-column set; a column outside it
    /// waits (and in the spawn ring, holds back everything farther). A column
    /// the client let go of while inside the radius waits until it has been
    /// outside it. Returns what to queue, in queue order, already recorded as
    /// sent.
    ///
    /// `verdicts` (Phase B2b, `chunk_verdict`): `None` pushes every column (the
    /// `all` mode, another terrain generator). With verdicts, a `Touched`
    /// column is pushed whole as above, an `Untouched` one goes as one
    /// "local" note ([`Planned::Local`]), and one with no verdict yet waits —
    /// neither pushed nor declared local (in the spawn ring, it holds back
    /// everything farther, like an unloaded one). A column of which some
    /// chunks are still sent (an overflow resync deferred out of range) gets
    /// its other chunks pushed, never a note: the joiner may hold a stale
    /// copy.
    pub fn plan_columns(
        &mut self,
        world: &World,
        loaded: &ahash::AHashSet<(i32, i32)>,
        centre: (i32, i32),
        limit: i32,
        room: usize,
        verdicts: Option<&crate::chunk_verdict::Verdicts>,
    ) -> Vec<Planned> {
        self.forget_far(centre, limit);
        let r = self.radius(limit);
        let within = |(cx, cz): (i32, i32), r: i32| {
            (cx - centre.0).abs() <= r && (cz - centre.1).abs() <= r
        };
        // A held-off column is released once it is outside the radius.
        self.held_off.retain(|&col| within(col, r));
        let mut out = Vec::new();
        let mut planned = 0usize;
        if room == 0 {
            return out;
        }
        let resync: Vec<ChunkCoord> = self.resync.iter().copied().collect();
        for c in resync {
            if !within((c.0, c.2), r) {
                // Pushed again with its column once that is back in range.
                self.resync.remove(&c);
                continue;
            }
            if !loaded.contains(&(c.0, c.2)) {
                continue;
            }
            if !self.window_open() || planned >= room {
                // Later; the spawn ring may still go below.
                break;
            }
            self.resync.remove(&c);
            let packets = build_chunk_packets(world, c);
            planned += packets.iter().map(Vec::len).sum::<usize>();
            self.record(c, &packets);
            out.push(Planned::Chunk(c, packets));
        }
        // With the window shut only the spawn ring can still go.
        let ring = SPAWN_RING_RADIUS.min(r);
        let reach = if self.window_open() { r } else { ring };
        for (_, cx, cz) in columns_nearest_first(centre, reach) {
            let in_ring = within((cx, cz), ring);
            if !loaded.contains(&(cx, cz)) {
                if in_ring {
                    // The ground under the body first: wait for it.
                    break;
                }
                continue;
            }
            if self.held_off.contains(&(cx, cz)) {
                continue;
            }
            let unsent: Vec<i32> =
                (0..=MAX_CHUNK_Y).filter(|&cy| !self.sent.contains_key(&(cx, cy, cz))).collect();
            if unsent.is_empty() {
                continue;
            }
            let local = match verdicts.map(|v| v.get((cx, cz))) {
                None | Some(Some(crate::chunk_verdict::Verdict::Touched)) => false,
                Some(Some(crate::chunk_verdict::Verdict::Untouched)) => {
                    unsent.len() == (MAX_CHUNK_Y + 1) as usize
                }
                Some(None) => {
                    if in_ring {
                        break;
                    }
                    continue;
                }
            };
            if planned >= room || !(in_ring || self.window_open()) {
                return out;
            }
            if local {
                let note = build_local_note((cx, cz));
                planned += note.len();
                self.record_local((cx, cz), &note);
                out.push(Planned::Local((cx, cz), note));
                continue;
            }
            for cy in unsent {
                let c = (cx, cy, cz);
                let packets = build_chunk_packets(world, c);
                planned += packets.iter().map(Vec::len).sum::<usize>();
                self.record(c, &packets);
                out.push(Planned::Chunk(c, packets));
            }
        }
        out
    }

    /// [`Self::plan_columns`] pushing everything, as `(chunk, packets)`.
    /// Test-only (the B2a mechanism tests).
    #[cfg(test)]
    pub fn plan(
        &mut self,
        world: &World,
        loaded: &ahash::AHashSet<(i32, i32)>,
        centre: (i32, i32),
        limit: i32,
        room: usize,
    ) -> Vec<(ChunkCoord, Vec<Vec<u8>>)> {
        self.plan_columns(world, loaded, centre, limit, room, None)
            .into_iter()
            .map(|p| match p {
                Planned::Chunk(c, packets) => (c, packets),
                Planned::Local(..) => unreachable!("no verdicts, no notes"),
            })
            .collect()
    }

    /// The columns within the push radius round `centre`, nearest first, that
    /// need a verdict before this client can be told anything about them:
    /// loaded, not held off, not yet sent whole, with none in `verdicts` —
    /// and, while its credit window is shut, only in the spawn ring (nothing
    /// else could be sent anyway). At most `max` (Phase B2b;
    /// `HostedServer::broadcast_state` decides them).
    pub fn verdict_candidates(
        &self,
        loaded: &ahash::AHashSet<(i32, i32)>,
        centre: (i32, i32),
        limit: i32,
        verdicts: &crate::chunk_verdict::Verdicts,
        max: usize,
    ) -> Vec<(i64, (i32, i32))> {
        let r = self.radius(limit);
        let reach = if self.window_open() { r } else { SPAWN_RING_RADIUS.min(r) };
        let mut out = Vec::new();
        for (d, cx, cz) in columns_nearest_first(centre, reach) {
            if out.len() >= max {
                break;
            }
            let col = (cx, cz);
            if !loaded.contains(&col)
                || self.held_off.contains(&col)
                || verdicts.get(col).is_some()
                || (0..=MAX_CHUNK_Y).all(|cy| self.sent.contains_key(&(cx, cy, cz)))
            {
                continue;
            }
            out.push((d, col));
        }
        out
    }
}

/// One planned send for a client ([`ClientChunkPush::plan_columns`]).
#[derive(Debug)]
pub enum Planned {
    /// A chunk's `ChunkData` packet(s).
    Chunk(ChunkCoord, Vec<Vec<u8>>),
    /// A column's `ColumnLocal` note (Phase B2b).
    Local((i32, i32), Vec<u8>),
}

/// Every column within `r` (Chebyshev) of `centre` as `(d², cx, cz)`, nearest
/// first (ties in coordinate order).
fn columns_nearest_first(centre: (i32, i32), r: i32) -> Vec<(i64, i32, i32)> {
    let mut columns: Vec<(i64, i32, i32)> = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);
    for dx in -r..=r {
        for dz in -r..=r {
            columns.push((i64::from(dx * dx + dz * dz), centre.0 + dx, centre.1 + dz));
        }
    }
    columns.sort_unstable();
    columns
}

/// The serialized `ColumnLocal` note for column `col` (Phase B2b).
pub fn build_local_note(col: (i32, i32)) -> Vec<u8> {
    protocol::serialize_packet(
        protocol::PacketType::ColumnLocal,
        &protocol::ColumnLocalPacket { cx: col.0, cz: col.1 },
    )
}

/// World position of chunk-local cell `i` (`x + z*16 + y*256`) in `coord`.
pub(crate) fn cell_pos(coord: ChunkCoord, i: u16) -> (i32, i32, i32) {
    let cs = CHUNK_SIZE as i32;
    let i = i32::from(i);
    (coord.0 * cs + i % cs, coord.1 * cs + i / (cs * cs), coord.2 * cs + (i / cs) % cs)
}

/// Queue this tick's pushes for one client (see [`ClientChunkPush::plan`]) on
/// its outbox, after whatever it already holds: its overflow resyncs first,
/// then new chunks, up to [`QUEUE_AHEAD_BYTES`] queued in all.
pub fn queue_pushes(
    push: &mut ClientChunkPush,
    outbox: &mut crate::state_outbox::ClientOutbox,
    world: &World,
    loaded: &ahash::AHashSet<(i32, i32)>,
    centre: (i32, i32),
    limit: i32,
    verdicts: Option<&crate::chunk_verdict::Verdicts>,
) {
    push.request_resync(outbox.take_chunk_resync_requests());
    let room = QUEUE_AHEAD_BYTES.saturating_sub(outbox.queued_bytes());
    for planned in push.plan_columns(world, loaded, centre, limit, room, verdicts) {
        match planned {
            Planned::Chunk(coord, packets) => outbox.push_chunk(coord, packets),
            Planned::Local(col, note) => outbox.push_column_note(col, note),
        }
    }
}

/// The world-level side-table entries (`block_meta`, `block_entities`,
/// `face_attachments`, …) whose cell lies in chunk `coord`, as chunk-local
/// cell index + world cell, in cell order. Scans the map when it is smaller
/// than a chunk, else probes the chunk's cells — so the cost is never more
/// than one chunk's worth of lookups.
pub(crate) fn cells_in<V>(
    map: &ahash::AHashMap<(i32, i32, i32), V>,
    coord: ChunkCoord,
) -> Vec<(u16, (i32, i32, i32), &V)> {
    let cs = CHUNK_SIZE as i32;
    let local = |(x, y, z): (i32, i32, i32)| -> u16 {
        let (lx, ly, lz) = (x - coord.0 * cs, y - coord.1 * cs, z - coord.2 * cs);
        (lx + lz * cs + ly * cs * cs) as u16
    };
    let mut out: Vec<(u16, (i32, i32, i32), &V)> = if map.len() <= CHUNK_VOLUME {
        map.iter()
            .filter(|(cell, _)| chunk_of_cell(**cell) == coord)
            .map(|(&cell, v)| (local(cell), cell, v))
            .collect()
    } else {
        (0..CHUNK_VOLUME as u16)
            .filter_map(|i| {
                let cell = cell_pos(coord, i);
                map.get(&cell).map(|v| (i, cell, v))
            })
            .collect()
    };
    out.sort_unstable_by_key(|(i, _, _)| *i);
    out
}

/// The side-table entries of `map` on the BLOCK cells of `chunk` (at
/// `coord`), as chunk-local cell index, in cell order. Side data belongs to a
/// block — metadata, a block entity, a face attachment — so an air cell's is
/// never sent (B2a review LOW-6: an all-air chunk costs no lookup at all).
/// Walks the map when it is smaller than the chunk's block count, else probes
/// only the block cells.
pub(crate) fn block_entries_in<'a, V>(
    map: &'a ahash::AHashMap<(i32, i32, i32), V>,
    coord: ChunkCoord,
    chunk: &Chunk,
) -> Vec<(u16, &'a V)> {
    if map.is_empty() || chunk.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(u16, &V)> = if map.len() <= chunk.non_air_count() {
        cells_in(map, coord)
            .into_iter()
            .filter(|&(i, _, _)| {
                let (i, cs) = (usize::from(i), CHUNK_SIZE);
                chunk.get(i % cs, i / (cs * cs), (i / cs) % cs) != crate::block::AIR
            })
            .map(|(i, _, v)| (i, v))
            .collect()
    } else {
        chunk
            .non_air_cells()
            .filter_map(|(i, _)| map.get(&cell_pos(coord, i)).map(|v| (i, v)))
            .collect()
    };
    out.sort_unstable_by_key(|(i, _)| *i);
    out
}

/// What a joiner sees of a block entity, if anything.
fn pushed_entity(e: &BlockEntityData) -> Option<PushedEntity> {
    match e {
        BlockEntityData::Sign(s) => Some(PushedEntity::Sign { text: s.text.clone() }),
        BlockEntityData::ItemFrame(f) => {
            let (item_kind, item_id) = f.item.as_ref().map_or((protocol::item_kind::EMPTY, 0), |st| {
                crate::inventory::item_to_ref(&st.item).to_wire()
            });
            let full_item = f
                .item
                .as_ref()
                .map(|st| crate::inventory::item_to_wire_full(&st.item))
                .unwrap_or_default();
            Some(PushedEntity::ItemFrame { item_kind, item_id, full_item, rotation: f.rotation })
        }
        BlockEntityData::Campfire(c) => Some(PushedEntity::Campfire {
            fuel_ticks: c.fuel_ticks,
            smoke_ticks: c.smoke_ticks,
            smoulder_ticks: c.smoulder_ticks,
            raid_warning: c.raid_warning_active,
        }),
        _ => None,
    }
}

/// The render stub of a face attachment.
fn pushed_attachment(a: &FaceAttachment) -> PushedAttachment {
    match a {
        FaceAttachment::Wallpaper(b) => PushedAttachment::Wallpaper(*b),
        FaceAttachment::BlueprintBlank => PushedAttachment::BlueprintBlank,
        FaceAttachment::Blueprint(plan) => {
            PushedAttachment::Blueprint { developed: plan.develop_state.is_developed() }
        }
    }
}

/// The compressed bytes of an all-air chunk (every absent chunk's push).
fn empty_chunk_bytes() -> &'static [u8] {
    static EMPTY: std::sync::LazyLock<Vec<u8>> =
        std::sync::LazyLock::new(|| protocol::compress_chunk(&Chunk::new().as_bytes()));
    &EMPTY
}

/// One side-data item on its way into a packet.
enum Side {
    Meta((u16, u8)),
    Entity(PushedBlockEntity),
    Attachment(PushedFaceAttachment),
}

/// The serialized `ChunkData` packet(s) for chunk `coord` of `world`, each at
/// most [`CHUNK_PACKET_MAX_BYTES`]: the blocks with as much side data as
/// fits, then continuations for the rest. An absent chunk is an all-air one,
/// and an all-air chunk carries no side data. **The anti-X-ray seam** (see
/// the module docs).
pub fn build_chunk_packets(world: &World, coord: ChunkCoord) -> Vec<Vec<u8>> {
    let (cx, cy, cz) = coord;
    let chunk = world.get_chunk(cx, cy, cz).filter(|c| !c.is_empty());
    let blocks = match chunk {
        Some(c) => protocol::compress_chunk(&c.as_bytes()),
        None => empty_chunk_bytes().to_vec(),
    };
    let mut side: Vec<Side> = Vec::new();
    if let Some(chunk) = chunk {
        side.extend(
            block_entries_in(&world.block_meta, coord, chunk)
                .into_iter()
                .filter(|(_, m)| **m != 0)
                .map(|(i, m)| Side::Meta((i, *m))),
        );
        for (cell, e) in block_entries_in(&world.block_entities, coord, chunk) {
            if let Some(entity) = pushed_entity(e) {
                side.push(Side::Entity(PushedBlockEntity { cell, entity }));
            }
        }
    }
    let faces = chunk.map_or_else(Vec::new, |c| block_entries_in(&world.face_attachments, coord, c));
    for (cell, faces) in faces {
        for (face, slot) in faces.iter().enumerate() {
            if let Some(a) = slot {
                side.push(Side::Attachment(PushedFaceAttachment {
                    cell,
                    face: face as u8,
                    attachment: pushed_attachment(a),
                }));
            }
        }
    }

    let fresh = |compressed_blocks: Vec<u8>| ChunkDataPacket {
        cx,
        cy,
        cz,
        compressed_blocks,
        meta: Vec::new(),
        entities: Vec::new(),
        attachments: Vec::new(),
    };
    let size_of = |p: &ChunkDataPacket| 1 + bincode::serialized_size(p).expect("sizes") as usize;
    let mut packets = Vec::new();
    let mut pkt = fresh(blocks);
    let mut size = size_of(&pkt);
    for item in side {
        let item_size = match &item {
            Side::Meta(m) => bincode::serialized_size(m),
            Side::Entity(e) => bincode::serialized_size(e),
            Side::Attachment(a) => bincode::serialized_size(a),
        }
        .expect("sizes") as usize;
        if size + item_size > CHUNK_PACKET_MAX_BYTES {
            packets.push(std::mem::replace(&mut pkt, fresh(Vec::new())));
            size = size_of(&pkt);
        }
        size += item_size;
        match item {
            Side::Meta(m) => pkt.meta.push(m),
            Side::Entity(e) => pkt.entities.push(e),
            Side::Attachment(a) => pkt.attachments.push(a),
        }
    }
    packets.push(pkt);
    packets
        .iter()
        .map(|p| {
            let bytes = protocol::serialize_packet(protocol::PacketType::ChunkData, p);
            debug_assert!(bytes.len() <= CHUNK_PACKET_MAX_BYTES, "a chunk packet over the cap");
            bytes
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    fn decode(bytes: &[u8]) -> ChunkDataPacket {
        let (t, payload) = protocol::deserialize_header(bytes).expect("tagged");
        assert_eq!(t, protocol::PacketType::ChunkData);
        protocol::safe_deserialize(payload).expect("decodes")
    }

    fn all_loaded(r: i32) -> ahash::AHashSet<(i32, i32)> {
        let mut s = ahash::AHashSet::new();
        for x in -r..=r {
            for z in -r..=r {
                s.insert((x, z));
            }
        }
        s
    }

    #[test]
    fn an_absent_chunk_goes_as_an_explicit_all_air_chunk() {
        let world = World::new();
        let packets = build_chunk_packets(&world, (3, 2, -1));
        assert_eq!(packets.len(), 1);
        let p = decode(&packets[0]);
        assert_eq!((p.cx, p.cy, p.cz), (3, 2, -1));
        let raw = protocol::decompress_chunk(&p.compressed_blocks).unwrap();
        assert!(Chunk::from_bytes(&raw).unwrap().is_empty());
        assert!(packets[0].len() < 100, "an all-air chunk is tiny ({} bytes)", packets[0].len());
    }

    #[test]
    fn a_push_carries_blocks_meta_and_render_visible_entities_but_no_contents() {
        let mut world = World::new();
        world.set_block(17, 20, 1, block::STONE);
        world.set_meta((17, 20, 1), 5);
        world.set_meta((40, 20, 1), 7); // another chunk
        let mut sign = crate::sign::SignData::new();
        sign.set_text("Welcome");
        world.set_block(18, 20, 2, block::OAK_SIGN);
        world.insert_sign((18, 20, 2), sign);
        let fire = crate::campfire::CampfireData {
            fuel_ticks: 99,
            raid_warning_active: true,
            ..Default::default()
        };
        world.set_block(19, 21, 3, block::CAMPFIRE);
        world.insert_campfire((19, 21, 3), fire);
        let mut frame = crate::item_frame::ItemFrameData::new();
        frame.try_insert(crate::item::ItemStack::new_block(block::OAK_PLANKS, 1));
        world.set_block(20, 22, 4, block::ITEM_FRAME);
        world.insert_item_frame((20, 22, 4), frame);
        world.set_block(21, 20, 5, block::CHEST);
        world.block_entities.insert(
            (21, 20, 5),
            BlockEntityData::Chest(crate::chest::ChestData::default()),
        );
        world.set_block(22, 20, 6, block::STONE);
        // Side data on an AIR cell is never sent: it belongs to no block.
        world.set_meta((23, 20, 7), 3);
        world.face_attachments.insert((22, 20, 6), {
            let mut f: crate::world::FaceAttachments = Default::default();
            f[2] = Some(FaceAttachment::Wallpaper(block::GLASS));
            f
        });

        let packets = build_chunk_packets(&world, (1, 1, 0));
        assert_eq!(packets.len(), 1);
        let p = decode(&packets[0]);
        let chunk = Chunk::from_bytes(&protocol::decompress_chunk(&p.compressed_blocks).unwrap()).unwrap();
        assert_eq!(chunk.get(1, 4, 1), block::STONE);
        let cell = |x: i32, y: i32, z: i32| ((x - 16) + z * 16 + (y - 16) * 256) as u16;
        assert_eq!(p.meta, vec![(cell(17, 20, 1), 5)], "only this chunk's meta");
        assert_eq!(p.entities.len(), 3, "sign, campfire, frame — not the chest: {:?}", p.entities);
        assert!(p.entities.contains(&PushedBlockEntity {
            cell: cell(18, 20, 2),
            entity: PushedEntity::Sign { text: "Welcome".into() },
        }));
        assert!(p.entities.contains(&PushedBlockEntity {
            cell: cell(19, 21, 3),
            entity: PushedEntity::Campfire {
                fuel_ticks: 99,
                smoke_ticks: 0,
                smoulder_ticks: 0,
                raid_warning: true,
            },
        }));
        assert!(p.entities.iter().any(|e| e.cell == cell(20, 22, 4)
            && matches!(e.entity, PushedEntity::ItemFrame { item_kind: protocol::item_kind::BLOCK, item_id, .. } if item_id == block::OAK_PLANKS)));
        assert_eq!(
            p.attachments,
            vec![PushedFaceAttachment {
                cell: cell(22, 20, 6),
                face: 2,
                attachment: PushedAttachment::Wallpaper(block::GLASS),
            }]
        );
    }

    #[test]
    fn heavy_side_data_splits_into_continuations_under_the_cap() {
        let mut world = World::new();
        // 4,096 signs of 120 chars: far more than one packet.
        for i in 0..CHUNK_VOLUME as i32 {
            let mut s = crate::sign::SignData::new();
            s.set_text(&"x".repeat(crate::sign::SIGN_MAX_CHARS));
            let at = (i % 16, i / 256, (i / 16) % 16);
            world.set_block(at.0, at.1, at.2, block::OAK_SIGN);
            world.insert_sign(at, s);
        }
        let packets = build_chunk_packets(&world, (0, 0, 0));
        assert!(packets.len() > 1);
        let mut signs = 0;
        for (i, raw) in packets.iter().enumerate() {
            assert!(raw.len() <= CHUNK_PACKET_MAX_BYTES);
            let p = decode(raw);
            assert_eq!(p.compressed_blocks.is_empty(), i > 0, "only the first carries blocks");
            signs += p.entities.len();
        }
        assert_eq!(signs, CHUNK_VOLUME, "nothing dropped");
    }

    #[test]
    fn the_plan_pushes_the_spawn_ring_first_then_nearest_out_within_the_window() {
        let world = World::new();
        let loaded = all_loaded(8);
        let mut push = ClientChunkPush::new(10);
        let planned = push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        let column = MAX_CHUNK_Y as usize + 1;
        let window = CHUNK_WINDOW_PACKETS as usize;
        assert!(
            (window..window + column).contains(&planned.len()),
            "the window bounds a tick, whole columns: {}",
            planned.len()
        );
        assert_eq!(planned.len() % column, 0, "only whole columns");
        let ring: Vec<_> = planned[..54].iter().map(|(c, _)| (c.0, c.2)).collect();
        assert!(ring.iter().all(|&(x, z)| x.abs() <= 1 && z.abs() <= 1), "the 3×3 first: {ring:?}");
        assert_eq!(planned[0].0, (0, 0, 0), "the body's own column, bottom up");
        // Nothing more until the client acknowledges.
        assert!(push.plan(&world, &loaded, (0, 0), 8, usize::MAX).is_empty());
        push.ack(push.pushed() - 30);
        let more = push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert!(((window - 30)..(window - 30 + column)).contains(&more.len()), "{} more", more.len());
        assert_eq!(more.len() % column, 0);
    }

    #[test]
    fn a_column_is_planned_whole_even_past_the_room_or_the_window() {
        let world = World::new();
        let mut push = ClientChunkPush::new(8);
        // Room for about one all-air chunk: its whole column still goes.
        let planned = push.plan(&world, &all_loaded(8), (0, 0), 8, 1);
        assert_eq!(planned.len(), MAX_CHUNK_Y as usize + 1, "one whole column, not one chunk");
        assert!(planned.iter().all(|(c, _)| (c.0, c.2) == (0, 0)));
        // The window is checked between columns only.
        let mut push = ClientChunkPush::new(8);
        let mut total = 0;
        loop {
            let got = push.plan(&world, &all_loaded(8), (0, 0), 8, usize::MAX);
            if got.is_empty() {
                break;
            }
            assert_eq!(got.len() % (MAX_CHUNK_Y as usize + 1), 0, "whole columns, every tick");
            total += got.len();
            push.ack(push.pushed() - 5); // a laggard: 5 always unacknowledged
        }
        assert_eq!(total, 17 * 17 * (MAX_CHUNK_Y as usize + 1));
    }

    #[test]
    fn the_spawn_ring_goes_whatever_the_window() {
        // A ring needing more packets than the window (heavy side data in
        // every chunk) still reaches a joiner that has acknowledged nothing.
        let mut world = World::new();
        for cx in -1..=1 {
            for cz in -1..=1 {
                for cy in 0..=MAX_CHUNK_Y {
                    for i in 0..300 {
                        let at = (cx * 16 + i % 16, cy * 16 + i / 256, cz * 16 + (i / 16) % 16);
                        let mut s = crate::sign::SignData::new();
                        s.set_text(&"y".repeat(crate::sign::SIGN_MAX_CHARS));
                        world.set_block(at.0, at.1, at.2, block::OAK_SIGN);
                        world.insert_sign(at, s);
                    }
                }
            }
        }
        let mut push = ClientChunkPush::new(8);
        let mut ring_packets = 0usize;
        for _ in 0..4 {
            for (c, packets) in push.plan(&world, &all_loaded(8), (0, 0), 8, usize::MAX) {
                assert!(c.0.abs() <= 1 && c.2.abs() <= 1, "nothing past the ring unacknowledged: {c:?}");
                ring_packets += packets.len();
            }
        }
        assert!(ring_packets > CHUNK_WINDOW_PACKETS as usize, "{ring_packets} packets: more than the window");
        for cx in -1..=1 {
            for cz in -1..=1 {
                assert!((0..=MAX_CHUNK_Y).all(|cy| push.has_sent((cx, cy, cz))), "ring column ({cx}, {cz})");
            }
        }
    }

    #[test]
    fn a_tick_plans_no_more_than_its_room() {
        let world = World::new();
        let mut push = ClientChunkPush::new(8);
        let room = 500;
        let planned = push.plan(&world, &all_loaded(8), (0, 0), 8, room);
        // All-air chunks are about 60 bytes, so a column about 360: room for
        // two columns, the second passing it.
        let column = MAX_CHUNK_Y as usize + 1;
        let bytes: usize = planned.iter().flat_map(|(_, p)| p).map(Vec::len).sum();
        assert_eq!(planned.len() % column, 0, "whole columns");
        let last: usize = planned[planned.len() - column..].iter().flat_map(|(_, p)| p).map(Vec::len).sum();
        assert!(bytes >= room && bytes - last < room, "it stops once past the room: {bytes} bytes");
        assert!(push.plan(&world, &all_loaded(8), (0, 0), 8, 0).is_empty(), "no room, no pushes");
    }

    #[test]
    fn the_radius_is_the_joiners_render_distance_capped_by_the_server() {
        assert_eq!(ClientChunkPush::new(4).radius(8), 4);
        assert_eq!(ClientChunkPush::new(12).radius(8), 8);
        assert_eq!(ClientChunkPush::new(0).radius(8), 8, "unsaid: the server's limit");
        // Everything in a radius-2 square, all six cy, nothing beyond it.
        let world = World::new();
        let mut push = ClientChunkPush::new(2);
        let mut all = Vec::new();
        loop {
            let got = push.plan(&world, &all_loaded(6), (0, 0), 8, usize::MAX);
            if got.is_empty() {
                break;
            }
            push.ack(push.pushed());
            all.extend(got.into_iter().map(|(c, _)| c));
        }
        assert_eq!(all.len(), 25 * 6);
        assert!(all.iter().all(|c| c.0.abs() <= 2 && c.2.abs() <= 2));
    }

    #[test]
    fn an_unloaded_ring_column_holds_back_everything_farther() {
        let world = World::new();
        let mut loaded = all_loaded(4);
        loaded.remove(&(1, 0));
        let mut push = ClientChunkPush::new(4);
        let planned = push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert!(planned.iter().all(|(c, _)| c.0.abs() + c.2.abs() <= 1), "only the nearer ring columns");
        loaded.remove(&(4, 4));
        loaded.insert((1, 0));
        push.ack(push.pushed());
        let planned = push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert!(planned.iter().any(|(c, _)| (c.0, c.2) == (1, 0)), "it goes once loaded");
    }

    #[test]
    fn a_dropped_column_leaves_the_sent_set_unless_pushed_again_since() {
        let world = World::new();
        let loaded = all_loaded(2);
        let mut push = ClientChunkPush::new(2);
        while !push.plan(&world, &loaded, (0, 0), 8, usize::MAX).is_empty() {
            push.ack(push.pushed());
        }
        assert!(push.has_sent((0, 0, 0)));
        let first_of_origin = 1;
        // Dropped as of a count that covers the origin column's push.
        let drop = ChunkDrop { cx: 0, cz: 0, as_of: first_of_origin + 5 };
        push.drop_columns(&[drop]);
        assert!(!push.has_sent((0, 0, 0)) && !push.has_sent((0, 5, 0)));
        // Dropped inside the radius: held off until it has been outside it.
        push.ack(push.pushed());
        assert!(push.plan(&world, &loaded, (0, 0), 8, usize::MAX).is_empty(), "no churn");
        push.plan(&world, &loaded, (9, 0), 8, usize::MAX); // the body walks off …
        push.ack(push.pushed());
        push.plan(&world, &loaded, (0, 0), 8, usize::MAX); // … and back
        assert!(push.has_sent((0, 0, 0)), "pushed afresh once back in range");
        // Then a STALE drop report (as of before the re-push) arrives — a
        // repeat, say: the client holds the new copy, and nothing is held off.
        push.drop_columns(&[drop]);
        push.drop_columns(&[drop]);
        assert!(push.has_sent((0, 0, 0)), "a drop older than the push does not undo it");
        assert!(push.held_off.is_empty());
    }

    #[test]
    fn a_column_dropped_inside_the_radius_waits_until_it_has_left_it() {
        // Review MEDIUM-1: a client that keeps less than the server pushes
        // (a stale render distance, a body that drifted) must not churn.
        let world = World::new();
        let loaded = all_loaded(12);
        let mut push = ClientChunkPush::new(8);
        while !push.plan(&world, &loaded, (0, 0), 8, usize::MAX).is_empty() {
            push.ack(push.pushed());
        }
        let as_of = push.pushed();
        let outer: Vec<ChunkDrop> =
            (-8..=8).map(|z| ChunkDrop { cx: 8, cz: z, as_of }).collect();
        push.drop_columns(&outer);
        for _ in 0..10 {
            assert!(push.plan(&world, &loaded, (0, 0), 8, usize::MAX).is_empty(), "never pushed again in place");
        }
        // One column east and back: (8, z) is at 7, still inside — held.
        push.plan(&world, &loaded, (1, 0), 8, usize::MAX);
        push.ack(push.pushed());
        assert!(push.plan(&world, &loaded, (0, 0), 8, usize::MAX).is_empty());
        // One column west: (8, z) is at 9, outside — released; back east, it goes.
        push.plan(&world, &loaded, (-1, 0), 8, usize::MAX);
        push.ack(push.pushed());
        let again = push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert!(again.iter().all(|(c, _)| c.0 == 8) && !again.is_empty(), "{again:?}");
    }

    #[test]
    fn the_radius_follows_the_clients_current_render_distance() {
        let mut push = ClientChunkPush::new(10);
        assert_eq!(push.radius(8), 8);
        push.set_render_distance(4);
        assert_eq!(push.radius(8), 4);
        push.set_render_distance(0);
        assert_eq!(push.radius(8), 4, "0 = unchanged");
        push.set_render_distance(255);
        assert_eq!(push.radius(64), crate::graphics_settings::RENDER_DISTANCE_MAX, "clamped");
    }

    #[test]
    fn a_drop_cancels_a_pending_resync_and_a_resync_respects_the_radius() {
        // Review LOW-2.
        let world = World::new();
        let loaded = all_loaded(4);
        let mut push = ClientChunkPush::new(3);
        while !push.plan(&world, &loaded, (0, 0), 8, usize::MAX).is_empty() {
            push.ack(push.pushed());
        }
        // An overflow lists (2, 1, 0) and (3, 1, 0); the client lets go of
        // column (2, 0) in the same window.
        push.request_resync([(2, 1, 0), (3, 1, 0)]);
        push.drop_columns(&[ChunkDrop { cx: 2, cz: 0, as_of: push.pushed() }]);
        // The client lowers its range: column 3 is now outside the radius.
        push.set_render_distance(2);
        let planned = push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert!(planned.is_empty(), "no stray chunk of a let-go or out-of-range column: {planned:?}");
        assert!(push.resync.is_empty());
        // Back to 3: column 3's chunk goes again with its column's turn.
        push.set_render_distance(3);
        let planned = push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert_eq!(planned.iter().map(|(c, _)| *c).collect::<Vec<_>>(), [(3, 1, 0)]);
    }

    #[test]
    fn a_resync_pushes_the_chunk_again_before_new_ones() {
        let world = World::new();
        let loaded = all_loaded(3);
        let mut push = ClientChunkPush::new(3);
        push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        push.ack(push.pushed());
        push.request_resync([(0, 2, 0), (99, 0, 99)]);
        assert!(!push.has_sent((0, 2, 0)));
        let planned = push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert_eq!(planned[0].0, (0, 2, 0), "the resync goes first");
        assert!(!planned.iter().any(|(c, _)| *c == (99, 0, 99)), "never sent: nothing to resync");
    }

    #[test]
    fn chunks_far_beyond_what_the_client_keeps_are_forgotten() {
        let world = World::new();
        let mut push = ClientChunkPush::new(4);
        push.plan(&world, &all_loaded(4), (0, 0), 8, usize::MAX);
        assert!(push.has_sent((0, 0, 0)));
        push.ack(push.pushed());
        // The body moved 4 + 2 + 4 + 1 columns away: past the backstop.
        let far = 4 + crate::chunk_stream::UNLOAD_HYSTERESIS + FORGET_SLACK + 1;
        push.plan(&world, &ahash::AHashSet::new(), (far, 0), 8, usize::MAX);
        assert!(!push.has_sent((0, 0, 0)));
    }

    #[test]
    fn acks_are_clamped_and_cumulative() {
        let world = World::new();
        let mut push = ClientChunkPush::new(2);
        push.plan(&world, &all_loaded(2), (0, 0), 8, usize::MAX);
        push.ack(u32::MAX);
        assert_eq!(push.acked, push.pushed, "never past what was queued");
        push.ack(3);
        assert_eq!(push.acked, push.pushed, "an older ack changes nothing");
        assert!(push.in_flight.is_empty() && push.in_flight_bytes == 0);
    }

    #[test]
    fn touched_is_the_default_and_a_mismatch_forces_all() {
        assert_eq!(ChunkSync::default(), ChunkSync::Touched);
        assert_eq!(ChunkSync::parse("all"), Ok(ChunkSync::All));
        assert_eq!(ChunkSync::parse(" ALL "), Ok(ChunkSync::All));
        assert_eq!(ChunkSync::parse("Touched"), Ok(ChunkSync::Touched));
        assert!(ChunkSync::parse("some").is_err());
        assert!(ChunkSync::All.pushes_everything(false));
        assert!(ChunkSync::All.pushes_everything(true));
        assert!(!ChunkSync::Touched.pushes_everything(false));
        assert!(ChunkSync::Touched.pushes_everything(true), "another generator gets everything");
    }

    #[test]
    fn verdicts_turn_untouched_columns_into_notes_and_hold_back_undecided_ones() {
        use crate::chunk_verdict::{Verdict, Verdicts};
        let world = World::new();
        let mut verdicts = Verdicts::default();
        for dx in -1..=1 {
            for dz in -1..=1 {
                verdicts.set_for_test((dx, dz), Verdict::Untouched);
            }
        }
        verdicts.touch((0, 0));
        // Past the ring nothing is decided yet.
        let mut push = ClientChunkPush::new(2);
        let out = push.plan_columns(&world, &all_loaded(2), (0, 0), 8, usize::MAX, Some(&verdicts));
        let chunks: Vec<ChunkCoord> =
            out.iter().filter_map(|p| if let Planned::Chunk(c, _) = p { Some(*c) } else { None }).collect();
        let notes: Vec<(i32, i32)> =
            out.iter().filter_map(|p| if let Planned::Local(c, _) = p { Some(*c) } else { None }).collect();
        assert_eq!(chunks.len(), MAX_CHUNK_Y as usize + 1, "the touched column, whole");
        assert!(chunks.iter().all(|c| (c.0, c.2) == (0, 0)));
        assert_eq!(notes.len(), 8, "each untouched ring column is one note");
        assert!(push.has_sent((1, 3, 1)), "a note puts the whole column in the sent-set");
        assert!(!push.has_sent((2, 0, 0)), "an undecided column waits");
        assert_eq!(push.pushed(), MAX_CHUNK_Y as u32 + 1 + 8, "a note is one numbered packet");
        if let Planned::Local(_, note) = &out[out.len() - 1] {
            assert!(note.len() < 16, "a note is tiny ({} bytes)", note.len());
        }
        // An undecided ring column holds back everything farther.
        let mut far_only = Verdicts::default();
        far_only.set_for_test((2, 0), Verdict::Untouched);
        let mut push = ClientChunkPush::new(2);
        let out = push.plan_columns(&world, &all_loaded(2), (0, 0), 8, usize::MAX, Some(&far_only));
        assert!(out.is_empty(), "the ring goes first: {} planned", out.len());
    }

    /// Size distribution of `compress_chunk(Chunk::as_bytes())`.
    fn stats(label: &str, mut sizes: Vec<usize>) {
        sizes.sort_unstable();
        let n = sizes.len();
        if n == 0 {
            println!("{label}: no chunks");
            return;
        }
        let total: usize = sizes.iter().sum();
        println!(
            "{label}: n={n} min={} median={} mean={} p90={} p95={} p99={} max={} total={}",
            sizes[0],
            sizes[n / 2],
            total / n,
            sizes[n * 90 / 100],
            sizes[n * 95 / 100],
            sizes[(n * 99 / 100).min(n - 1)],
            sizes[n - 1],
            total
        );
    }

    /// Measurement, not a check: `AXENSTAX_CHUNK_DIR=<world>/chunks[:<world2>/chunks]
    /// cargo test --lib measure_ -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement over saved worlds named by AXENSTAX_CHUNK_DIR"]
    fn measure_saved_world_chunk_sizes() {
        let dir = std::env::var("AXENSTAX_CHUNK_DIR").expect("AXENSTAX_CHUNK_DIR");
        for d in dir.split(':') {
            let mut sizes = Vec::new();
            for e in std::fs::read_dir(d).expect("chunk dir") {
                let p = e.unwrap().path();
                if p.extension().and_then(|x| x.to_str()) != Some("chunk") {
                    continue;
                }
                let bytes = std::fs::read(&p).unwrap();
                let chunk = Chunk::from_bytes(&bytes).expect("a chunk file");
                sizes.push(protocol::compress_chunk(&chunk.as_bytes()).len());
            }
            stats(d, sizes);
        }
    }

    /// Measurement over fresh terrain (what a mismatch joiner is mostly sent).
    #[test]
    #[ignore = "measurement"]
    fn measure_generated_chunk_sizes() {
        for seed in [1u32, 42, 600_000, 123_456_789] {
            let biome = crate::biome::BiomeGenerator::new(seed);
            let mut world = World::new();
            let mut sizes = Vec::new();
            for cx in -4..=4 {
                for cz in -4..=4 {
                    world.generate_column(cx, cz, &biome);
                    for cy in 0..=MAX_CHUNK_Y {
                        sizes.extend(build_chunk_packets(&world, (cx, cy, cz)).iter().map(Vec::len));
                    }
                }
            }
            stats(&format!("generated seed {seed} (whole packets)"), sizes);
        }
    }
}

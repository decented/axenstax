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
//!   all-air chunk: that is how a dug-out chunk reaches the joiner). Only
//!   columns the server has loaded: the rest wait for the server's streamer.
//!   The spawn ring (the 3×3 round the body) goes before anything else — a
//!   joiner's loading screen waits for it.
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
//!   than about one tick's budget ahead ([`QUEUE_AHEAD_BYTES`]), and a credit
//!   window bounds what is in flight: at most [`CHUNK_WINDOW_PACKETS`] packets
//!   and [`CHUNK_WINDOW_BYTES`] bytes not yet acknowledged. The client
//!   acknowledges cumulatively, piggybacked on its `InputPacket`
//!   (`chunk_ack`), so a slow link is never flooded (the transport closes a
//!   client with 8 MiB queued). The window covers the spawn ring without any
//!   acknowledgement, so a loading joiner — which sends no input yet — still
//!   gets the ground it needs.
//! - **Letting go.** The client reports every column it discards
//!   (`InputPacket.chunk_drops`, with its `chunk_ack` count at the time), and
//!   those chunks leave the sent-set: their changes stop and they are pushed
//!   again when back in range. As a backstop the server also forgets chunks
//!   far beyond anything the client keeps ([`FORGET_SLACK`]); the client then
//!   holds a stale copy until it comes back in range and gets a fresh one.
//! - **Resync.** A client whose outbox overflowed (`state_outbox`) has its
//!   dropped chunks pushed again whole, ahead of new ones.
//!
//! **Mode** (`--chunk-sync`, [`ChunkSync`]): `all` — push every chunk in
//! range — is the only mode B2a builds and the default. A joiner whose
//! terrain generator differs from the host's (`ServerPlayer::
//! worldgen_mismatch`) always gets everything; Phase B2b adds `touched`
//! (push only edited chunks; untouched ones generate locally).
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

use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::chunk::{Chunk, CHUNK_SIZE, CHUNK_VOLUME};
use crate::protocol::{
    self, ChunkDataPacket, ChunkDrop, PushedAttachment, PushedBlockEntity, PushedEntity,
    PushedFaceAttachment,
};
use crate::state_outbox::{chunk_of_cell, ChunkCoord, STATE_UPDATE_MAX_BYTES};
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
/// this tick's pushes are planned: about one tick's budget, so the queue
/// never holds more chunk data than one tick drains and the next tick's
/// deltas never wait behind a pile of chunks.
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
    /// Every chunk in range (B2a; the default).
    #[default]
    All,
}

impl ChunkSync {
    /// Parse a `--chunk-sync` value.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "all" => Ok(Self::All),
            "touched" => Err(
                "--chunk-sync touched is not built yet (Phase B2b): this build pushes every chunk (all)"
                    .to_string(),
            ),
            other => Err(format!("unknown --chunk-sync value `{other}` (expected `all`)")),
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

/// The `--chunk-sync` mode this process was started with (`all` unless set).
pub fn chunk_sync() -> ChunkSync {
    CHUNK_SYNC.get().copied().unwrap_or_default()
}

/// Read `--chunk-sync <mode>` (or `AXENSTAX_CHUNK_SYNC`) from a host
/// client's arguments and record it. An unusable value is logged and `all`
/// kept.
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
        Err(why) => log::warn!("{why}; pushing every chunk"),
    }
}

/// One remote client's chunk-push state. See the module docs.
#[derive(Debug, Default)]
pub struct ClientChunkPush {
    /// The client's render distance from its JoinRequest (`0` = not said).
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
}

impl ClientChunkPush {
    /// A fresh client that announced `render_distance` columns.
    pub fn new(render_distance: u8) -> Self {
        Self { render_distance: i32::from(render_distance), ..Self::default() }
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
    /// holds).
    pub fn drop_columns(&mut self, drops: &[ChunkDrop]) {
        for d in drops {
            for cy in 0..=MAX_CHUNK_Y {
                let c = (d.cx, cy, d.cz);
                if self.sent.get(&c).is_some_and(|&first| first <= d.as_of) {
                    self.sent.remove(&c);
                    self.resync.remove(&c);
                }
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

    /// May another packet be queued?
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

    /// Build this tick's pushes for a client whose server body stands in
    /// column `centre`: resyncs first, then unsent chunks nearest first
    /// within [`Self::radius`], while the window is open and until `room`
    /// bytes are planned (the last push may pass it). `loaded` is the
    /// server's loaded-column set; a column outside it waits (and in the
    /// spawn ring, holds back everything farther). Returns `(chunk,
    /// packets)` in queue order, already recorded as sent.
    pub fn plan(
        &mut self,
        world: &World,
        loaded: &ahash::AHashSet<(i32, i32)>,
        centre: (i32, i32),
        limit: i32,
        room: usize,
    ) -> Vec<(ChunkCoord, Vec<Vec<u8>>)> {
        self.forget_far(centre, limit);
        let mut out = Vec::new();
        let mut planned = 0usize;
        let open = |push: &Self, planned: usize| push.window_open() && planned < room;
        if !open(self, planned) {
            return out;
        }
        let resync: Vec<ChunkCoord> = self.resync.iter().copied().collect();
        for c in resync {
            if !open(self, planned) {
                return out;
            }
            if loaded.contains(&(c.0, c.2)) {
                self.resync.remove(&c);
                let packets = build_chunk_packets(world, c);
                planned += packets.iter().map(Vec::len).sum::<usize>();
                self.record(c, &packets);
                out.push((c, packets));
            }
        }
        let r = self.radius(limit);
        let mut columns: Vec<(i64, i32, i32)> = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);
        for dx in -r..=r {
            for dz in -r..=r {
                let d = i64::from(dx * dx + dz * dz);
                columns.push((d, centre.0 + dx, centre.1 + dz));
            }
        }
        columns.sort_unstable();
        for (_, cx, cz) in columns {
            if !loaded.contains(&(cx, cz)) {
                let ring = (cx - centre.0).abs() <= SPAWN_RING_RADIUS
                    && (cz - centre.1).abs() <= SPAWN_RING_RADIUS;
                if ring {
                    // The ground under the body first: wait for it.
                    break;
                }
                continue;
            }
            for cy in 0..=MAX_CHUNK_Y {
                let c = (cx, cy, cz);
                if self.sent.contains_key(&c) {
                    continue;
                }
                if !open(self, planned) {
                    return out;
                }
                let packets = build_chunk_packets(world, c);
                planned += packets.iter().map(Vec::len).sum::<usize>();
                self.record(c, &packets);
                out.push((c, packets));
            }
        }
        out
    }
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
) {
    push.request_resync(outbox.take_chunk_resync_requests());
    let room = QUEUE_AHEAD_BYTES.saturating_sub(outbox.queued_bytes());
    for (coord, packets) in push.plan(world, loaded, centre, limit, room) {
        outbox.push_chunk(coord, packets);
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

/// [`cells_in`] without the world cell.
fn entries_in<V>(
    map: &ahash::AHashMap<(i32, i32, i32), V>,
    coord: ChunkCoord,
) -> Vec<(u16, &V)> {
    cells_in(map, coord).into_iter().map(|(i, _, v)| (i, v)).collect()
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
/// most [`STATE_UPDATE_MAX_BYTES`]: the blocks with as much side data as
/// fits, then continuations for the rest. An absent chunk is an all-air one.
/// **The anti-X-ray seam** (see the module docs).
pub fn build_chunk_packets(world: &World, coord: ChunkCoord) -> Vec<Vec<u8>> {
    let (cx, cy, cz) = coord;
    let blocks = match world.get_chunk(cx, cy, cz) {
        Some(c) => protocol::compress_chunk(&c.as_bytes()),
        None => empty_chunk_bytes().to_vec(),
    };
    let mut side: Vec<Side> = Vec::new();
    side.extend(entries_in(&world.block_meta, coord).into_iter().filter(|(_, m)| **m != 0).map(|(i, m)| Side::Meta((i, *m))));
    for (cell, e) in entries_in(&world.block_entities, coord) {
        if let Some(entity) = pushed_entity(e) {
            side.push(Side::Entity(PushedBlockEntity { cell, entity }));
        }
    }
    for (cell, faces) in entries_in(&world.face_attachments, coord) {
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
        if size + item_size > STATE_UPDATE_MAX_BYTES {
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
            debug_assert!(bytes.len() <= STATE_UPDATE_MAX_BYTES, "a chunk packet over the cap");
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
        world.insert_sign((18, 20, 2), sign);
        let fire = crate::campfire::CampfireData {
            fuel_ticks: 99,
            raid_warning_active: true,
            ..Default::default()
        };
        world.insert_campfire((19, 21, 3), fire);
        let mut frame = crate::item_frame::ItemFrameData::new();
        frame.try_insert(crate::item::ItemStack::new_block(block::OAK_PLANKS, 1));
        world.insert_item_frame((20, 22, 4), frame);
        world.block_entities.insert(
            (21, 20, 5),
            BlockEntityData::Chest(crate::chest::ChestData::default()),
        );
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
            world.insert_sign((i % 16, i / 256, (i / 16) % 16), s);
        }
        let packets = build_chunk_packets(&world, (0, 0, 0));
        assert!(packets.len() > 1);
        let mut signs = 0;
        for (i, raw) in packets.iter().enumerate() {
            assert!(raw.len() <= STATE_UPDATE_MAX_BYTES);
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
        assert_eq!(planned.len(), CHUNK_WINDOW_PACKETS as usize, "the window bounds a tick");
        let ring: Vec<_> = planned[..54].iter().map(|(c, _)| (c.0, c.2)).collect();
        assert!(ring.iter().all(|&(x, z)| x.abs() <= 1 && z.abs() <= 1), "the 3×3 first: {ring:?}");
        assert_eq!(planned[0].0, (0, 0, 0), "the body's own column, bottom up");
        // Nothing more until the client acknowledges.
        assert!(push.plan(&world, &loaded, (0, 0), 8, usize::MAX).is_empty());
        push.ack(32);
        assert_eq!(push.plan(&world, &loaded, (0, 0), 8, usize::MAX).len(), 32);
    }

    #[test]
    fn a_tick_plans_no_more_than_its_room() {
        let world = World::new();
        let mut push = ClientChunkPush::new(8);
        let planned = push.plan(&world, &all_loaded(8), (0, 0), 8, 200);
        // All-air chunks are about 60 bytes: room for four, the fourth passing it.
        let bytes: usize = planned.iter().flat_map(|(_, p)| p).map(Vec::len).sum();
        assert!(bytes >= 200 && planned.len() >= 2, "{} pushes, {bytes} bytes", planned.len());
        assert!(bytes - planned.last().unwrap().1[0].len() < 200, "it stops once past the room");
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
        push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert!(push.has_sent((0, 0, 0)));
        let first_of_origin = 1;
        // Dropped as of a count that covers the origin column's push.
        push.drop_columns(&[ChunkDrop { cx: 0, cz: 0, as_of: first_of_origin + 5 }]);
        assert!(!push.has_sent((0, 0, 0)) && !push.has_sent((0, 5, 0)));
        // Pushed again (after the ack), then a STALE drop report (as of
        // before the re-push) arrives: the client holds the new copy.
        push.ack(push.pushed());
        push.plan(&world, &loaded, (0, 0), 8, usize::MAX);
        assert!(push.has_sent((0, 0, 0)));
        push.drop_columns(&[ChunkDrop { cx: 0, cz: 0, as_of: first_of_origin + 5 }]);
        assert!(push.has_sent((0, 0, 0)), "a drop older than the push does not undo it");
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
    fn only_all_is_built() {
        assert_eq!(ChunkSync::parse("all"), Ok(ChunkSync::All));
        assert_eq!(ChunkSync::parse(" ALL "), Ok(ChunkSync::All));
        assert!(ChunkSync::parse("touched").unwrap_err().contains("B2b"));
        assert!(ChunkSync::parse("some").is_err());
        assert!(ChunkSync::All.pushes_everything(false));
        assert!(ChunkSync::All.pushes_everything(true));
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

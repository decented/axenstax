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
//!   (`ColumnLocal`, numbered and acknowledged with the pushes). Inside `R`
//!   the client generates a column only once it has been told it is local
//!   ([`ChunkIntake::awaits_verdict`]), never before, so a touched column
//!   never flashes pristine terrain; outside `R` it generates as before. A
//!   local column is let go like a pushed one (discarded and reported), so
//!   the server decides it afresh on return.
//!
//! Renderer-free (the game loop meshes what [`ChunkIntake::take_relight`]
//! hands it), so it is unit-tested on a bare `World`.

use std::collections::VecDeque;

use crate::chunk::Chunk;
use crate::chunk_push::{cell_pos, cells_in};
use crate::protocol::{ChunkDataPacket, ChunkDrop, PushedAttachment, PushedEntity};
use crate::state_outbox::ChunkCoord;
use crate::world::{FaceAttachment, World, MAX_CHUNK_Y};

/// One packet of the server's numbered chunk stream, as `RemoteClient`
/// queues it.
#[derive(Debug)]
pub enum StreamItem {
    /// A pushed chunk (`ChunkData`).
    Chunk(ChunkDataPacket),
    /// "Column `(cx, cz)` is local" (`ColumnLocal`, Phase B2b).
    Local((i32, i32)),
}

/// One step of a frame's world intake, in arrival order (see [`interleave`]).
#[derive(Debug)]
pub enum IntakeStep {
    /// Apply one pushed chunk packet.
    Chunk(Box<ChunkDataPacket>),
    /// Take in a "column is local" note (Phase B2b).
    Local((i32, i32)),
    /// Apply these block changes (indices into the frame's
    /// `pending_block_changes`).
    Changes(std::ops::Range<usize>),
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
            StreamItem::Local(col) => IntakeStep::Local(col),
        });
    }
    if changes > done {
        steps.push(IntakeStep::Changes(done..changes));
    }
    steps
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
    /// `col` is local. It counts towards the ack like a push (it is a
    /// numbered packet of the same stream). Ignored otherwise for a column
    /// this client holds pushed chunks of: those are the server's own data,
    /// never replaced by a generation.
    pub fn note_local(&mut self, col: (i32, i32)) {
        self.applied = self.applied.wrapping_add(1);
        if self.holds_pushed(col) {
            log::debug!("Local note for pushed column {col:?}; keeping the pushed copy");
            return;
        }
        self.local.insert(col);
    }

    /// Phase B2b — has the server said column `col` is local (and this
    /// client not let go of it since)?
    pub fn is_local(&self, col: (i32, i32)) -> bool {
        self.local.contains(&col)
    }

    /// Phase B2b — must column `col` wait for the server before this client
    /// generates it? Yes when it lies within `min(render_distance,
    /// note_radius)` of the server body and the server has not yet said it is
    /// local or pushed any of it. Outside that radius a column is generated
    /// as before; so is every column of a session the server sends no notes.
    pub fn awaits_verdict(&self, col: (i32, i32), render_distance: i32) -> bool {
        let Some(centre) = self.server_centre else { return false };
        let r = render_distance.min(self.note_radius);
        self.note_radius > 0
            && (col.0 - centre.0).abs() <= r
            && (col.1 - centre.1).abs() <= r
            && !self.local.contains(&col)
            && !self.holds_pushed(col)
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
        let col = crate::chunk_stream::column_of_block(bc.x, bc.z);
        (self.local.contains(&col)
            && !crate::chunk_stream::remote_change_is_loaded(loaded, world, bc.x, bc.z))
        .then_some(col)
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
    /// this client nothing about.
    pub fn let_go(&mut self, world: &mut World, col: (i32, i32)) {
        self.discard(world, col, false);
    }

    /// Discard column `col` (chunks, side data, any evicted copy) and queue
    /// its drop report — if the server sent it anything, or `always`.
    fn discard(&mut self, world: &mut World, col: (i32, i32), always: bool) {
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

/// Remove the side data (`block_meta`, block entities, face attachments) of
/// every cell of chunk `coord`: a push replaces it whole.
fn clear_side_data(world: &mut World, coord: ChunkCoord) {
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
        let attachment = match a.attachment {
            PushedAttachment::Wallpaper(b) => FaceAttachment::Wallpaper(b),
            PushedAttachment::BlueprintBlank => FaceAttachment::BlueprintBlank,
            PushedAttachment::Blueprint { developed } => {
                FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::render_stub(developed)))
            }
        };
        world.set_face_attachment(cell_pos(coord, a.cell), usize::from(a.face), attachment);
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

    #[test]
    fn interleave_keeps_each_snapshot_between_the_changes_around_it() {
        let world = World::new();
        let c = |x| StreamItem::Chunk(packet_of(&world, (x, 0, 0)));
        let steps = interleave(
            vec![(0, c(1)), (2, c(2)), (2, StreamItem::Local((7, 7))), (2, c(3)), (5, c(4))],
            6,
        );
        let shape: Vec<String> = steps
            .iter()
            .map(|s| match s {
                IntakeStep::Chunk(p) => format!("C{}", p.cx),
                IntakeStep::Local(col) => format!("L{}", col.0),
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
                IntakeStep::Local(_) => unreachable!(),
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
    fn inside_the_note_radius_a_column_waits_for_its_verdict() {
        let mut intake = ChunkIntake::default();
        assert!(!intake.awaits_verdict((0, 0), 10), "no notes: generate as before");
        intake.expect_notes(3, (10, 10));
        assert!(intake.server_decides());
        assert!(intake.awaits_verdict((13, 7), 10), "inside min(rd 10, radius 3) of the server body");
        assert!(!intake.awaits_verdict((14, 10), 10), "outside it: generated as before");
        assert!(!intake.awaits_verdict((12, 10), 1), "the render distance caps it too");
        intake.note_local((13, 7));
        assert_eq!(intake.applied(), 1, "a note counts towards the ack");
        assert!(!intake.awaits_verdict((13, 7), 10), "told it is local: generate it");
        assert!(intake.decided((13, 7)) && !intake.decided((12, 12)));
        // The centre follows the server body.
        intake.set_server_centre((20, 10));
        assert!(!intake.awaits_verdict((12, 12), 10), "out of range now");
        assert!(intake.awaits_verdict((22, 12), 10));
    }

    #[test]
    fn a_change_for_a_local_column_not_generated_yet_generates_it_first() {
        let mut intake = ChunkIntake::default();
        intake.expect_notes(8, (0, 0));
        let world = World::new();
        let mut loaded = ahash::AHashSet::new();
        let bc = BlockChange::with_meta(20, 70, -5, block::GLASS, 0); // column (1, -1)
        assert_eq!(intake.generate_before(&bc, &loaded, &world), None, "not local: not ours to make");
        intake.note_local((1, -1));
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
        intake.note_local((1, 2));
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
        intake.note_local((-6, 6));
        let rd = 4;
        assert!(intake.keeps_near_server_body((5, 0), rd), "pushed, within 4 + 2");
        assert!(intake.keeps_near_server_body((-6, 6), rd), "local, within 4 + 2");
        assert!(!intake.keeps_near_server_body((1, 1), rd), "not the server's to keep");
        intake.set_server_centre((20, 0));
        assert!(!intake.keeps_near_server_body((5, 0), rd), "the body moved on: free to go");
    }
}

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
//! Renderer-free (the game loop meshes what [`ChunkIntake::take_relight`]
//! hands it), so it is unit-tested on a bare `World`.

use std::collections::VecDeque;

use crate::chunk::Chunk;
use crate::chunk_push::{cell_pos, cells_in};
use crate::protocol::{ChunkDataPacket, ChunkDrop, PushedAttachment, PushedEntity};
use crate::state_outbox::ChunkCoord;
use crate::world::{FaceAttachment, World, MAX_CHUNK_Y};

/// One step of a frame's world intake, in arrival order (see [`interleave`]).
#[derive(Debug)]
pub enum IntakeStep {
    /// Apply one pushed chunk packet.
    Chunk(Box<ChunkDataPacket>),
    /// Apply these block changes (indices into the frame's
    /// `pending_block_changes`).
    Changes(std::ops::Range<usize>),
}

/// Lay a frame's pushed chunks between its block changes in the order they
/// arrived: each chunk carries the number of changes that had arrived before
/// it (`RemoteClient::chunk_queue`).
pub fn interleave(chunks: Vec<(usize, ChunkDataPacket)>, changes: usize) -> Vec<IntakeStep> {
    let mut steps = Vec::with_capacity(chunks.len() * 2 + 1);
    let mut done = 0;
    for (before, chunk) in chunks {
        let before = before.min(changes);
        if before > done {
            steps.push(IntakeStep::Changes(done..before));
            done = before;
        }
        steps.push(IntakeStep::Chunk(Box::new(chunk)));
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
                log::warn!("Pushed chunk {coord:?} does not decode; ignored");
                return false;
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

    /// This client is letting go of pushed column `col` (out of range):
    /// discard it — chunks and side data, never into the evicted store, since
    /// the server pushes it afresh — and queue the report to the server.
    /// No-op for a column holding no pushed chunk.
    pub fn let_go(&mut self, world: &mut World, col: (i32, i32)) {
        if self.pushed_per_column.remove(&col).is_none() {
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
        let c = |x| packet_of(&world, (x, 0, 0));
        let steps = interleave(vec![(0, c(1)), (2, c(2)), (2, c(3)), (5, c(4))], 6);
        let shape: Vec<String> = steps
            .iter()
            .map(|s| match s {
                IntakeStep::Chunk(p) => format!("C{}", p.cx),
                IntakeStep::Changes(r) => format!("{}..{}", r.start, r.end),
            })
            .collect();
        assert_eq!(shape, ["C1", "0..2", "C2", "C3", "2..5", "C4", "5..6"]);
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
        for step in interleave(vec![(1, snapshot)], changes.len()) {
            match step {
                IntakeStep::Chunk(p) => {
                    intake.apply(&mut joiner, &mut loaded, &reg(), &p);
                }
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
}

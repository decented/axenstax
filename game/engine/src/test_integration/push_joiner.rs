//! A joiner as the game loop runs one, minus the renderer — the harness the
//! chunk-push suites share (`chunk_push`, Phase B2a; `touched_columns`, Phase
//! B2b).
//!
//! It drives REAL joins through `HostedServer::tick` (channel transport, no
//! sockets): a `RemoteClient`, a `World` and a `ChunkIntake` that applies the
//! server's pushes in order with its block changes and acknowledges them on
//! its inputs. Its world holds only what the server pushed and, in touched
//! mode, the columns the server said are local, which it generates from the
//! seed in its `JoinAccept` (`GameState::apply_world_deltas` and the
//! streamer, mirrored: a change or a pushed chunk for a local column not
//! generated yet generates the column first; a local column's check is
//! queued when noted if held, else when generated, and the queue is run at
//! the end of each [`Joiner::take_in`] — with no budget by default
//! ([`Joiner::check_budget`]), so a test sees every check of a frame done —
//! except that a change or a pushed chunk for a column whose check is
//! pending checks it first). A test may also generate columns before any
//! verdict, as the game's streamer does ([`Joiner::generate`]).

use crate::biome::BiomeGenerator;
use crate::block;
use crate::chunk::CHUNK_SIZE;
use crate::chunk_intake::{interleave, ChunkIntake, IntakeStep};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol::{self, BlockChange};
use crate::remote_client::{build_join_request_guest, RemoteClient};
use crate::state_outbox::ChunkCoord;
use crate::world::World;

/// WebSocket + 0 remote slots: no accept thread, no sockets, the open
/// (guest) join policy. One local player (the host) at slot 0 — an OWNING
/// host, which pushes everything (`--chunk-sync all` behaviour). The world
/// name must not exist on disk.
pub(super) fn start_host(tag: &str) -> HostedServer {
    HostedServer::start(
        1,
        format!("chunk-push-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("hosted server starts")
}

/// The dedicated server (no local player): it streams columns round every
/// player and, by default, pushes only touched columns (Phase B2b).
pub(super) fn start_dedicated(tag: &str) -> HostedServer {
    HostedServer::start(
        0,
        format!("touched-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts")
}

/// A joined client as the game loop runs one, minus the renderer.
pub(super) struct Joiner {
    pub rc: RemoteClient,
    pub world: World,
    pub loaded: ahash::AHashSet<(i32, i32)>,
    pub intake: ChunkIntake,
    pub registry: crate::block::BlockRegistry,
    /// Every server block change that reached this client.
    pub changes_seen: Vec<BlockChange>,
    /// C3b-2 — every block view that reached this client, in order.
    pub views_seen: Vec<protocol::BlockEntityView>,
    pub slot: usize,
    /// The render distance its inputs carry (`InputPacket.render_distance`).
    pub render_distance: u8,
    /// The host's generator, from `JoinAccept` (B2b: local columns).
    pub biome: Option<BiomeGenerator>,
    /// B2b — generate a local column as soon as it is noted (the streamer).
    /// Off: noted columns wait for [`Self::generate_noted`] — so a change can
    /// reach one not generated yet.
    pub generate_on_note: bool,
    /// Columns this client generated itself, in order.
    pub generated: Vec<(i32, i32)>,
    /// Chunk-stream bytes received (pushes and notes).
    pub stream_bytes: usize,
    /// `ColumnLocal` notes received.
    pub notes_received: usize,
    /// The column checks run at the end of each [`Self::take_in`]
    /// (`GameState::apply_world_deltas`' `check_budget`); unlimited unless a
    /// test sets one.
    pub check_budget: usize,
}

impl Joiner {
    /// Attach and join with `render_distance`; `mismatch` announces a terrain
    /// generator different from the host's.
    pub fn join(hs: &mut HostedServer, render_distance: u8, mismatch: bool) -> Self {
        Self::join_with(hs, render_distance, mismatch, HostedServer::tick)
    }

    /// [`Self::join`] with [`Self::generate_on_note`] off from the start: the
    /// notes that come with the join are not generated either.
    pub fn join_without_generating(hs: &mut HostedServer, render_distance: u8) -> Self {
        Self::join_inner(hs, render_distance, false, false, HostedServer::tick)
    }

    pub fn join_with(
        hs: &mut HostedServer,
        render_distance: u8,
        mismatch: bool,
        tick: impl FnMut(&mut HostedServer),
    ) -> Self {
        Self::join_inner(hs, render_distance, mismatch, true, tick)
    }

    fn join_inner(
        hs: &mut HostedServer,
        render_distance: u8,
        mismatch: bool,
        generate_on_note: bool,
        mut tick: impl FnMut(&mut HostedServer),
    ) -> Self {
        let client = hs.attach_test_remote();
        let mut req = build_join_request_guest("Joiner", 0);
        req.render_distance = render_distance;
        if mismatch {
            req.worldgen_version = crate::world::worldgen_fingerprint() ^ 1;
        }
        let rc = RemoteClient::from_transport(Box::new(client), req, None);
        tick(hs);
        let mut j = Joiner {
            rc,
            world: World::new(),
            loaded: ahash::AHashSet::new(),
            intake: ChunkIntake::default(),
            registry: crate::block::BlockRegistry::new(),
            changes_seen: Vec::new(),
            views_seen: Vec::new(),
            slot: usize::MAX,
            render_distance,
            biome: None,
            generate_on_note,
            generated: Vec::new(),
            stream_bytes: 0,
            notes_received: 0,
            check_budget: usize::MAX,
        };
        j.take_in();
        j.slot = j.rc.player_index().expect("joined") as usize;
        j
    }

    /// Generate column `col` from the host's seed, as the streamer would
    /// (light and fluids are the renderer's side; not needed here), and
    /// queue its check if it is noted local (`ChunkIntake::column_held`).
    pub fn generate(&mut self, col: (i32, i32)) {
        let biome = self.biome.as_ref().expect("the JoinAccept gave us the seed");
        self.world.generate_column(col.0, col.1, biome);
        self.loaded.insert(col);
        self.generated.push(col);
        self.intake.column_held(col);
    }

    /// Check held column `col` against its note now, if its check is pending
    /// (`GameState::check_column_now`): gone on a real mismatch.
    fn check_now(&mut self, col: (i32, i32)) {
        if !self.loaded.contains(&col) {
            return;
        }
        let biome = self.biome.as_ref().expect("the JoinAccept gave us the seed");
        if self.intake.verify_local(&mut self.world, biome, col).let_go() {
            self.loaded.remove(&col);
        }
    }

    /// Run the queued column checks within [`Self::check_budget`]
    /// (`ChunkIntake::run_checks`).
    pub fn run_checks(&mut self) {
        let Some(biome) = self.biome.as_ref() else { return };
        for col in self.intake.run_checks(&mut self.world, biome, &self.loaded, self.check_budget) {
            self.loaded.remove(&col);
        }
    }

    /// Generate every column within `r` of `centre` not held yet, as the
    /// game's streamer does before any verdict (B2b fix D1).
    pub fn generate_around(&mut self, centre: (i32, i32), r: i32) {
        for col in columns_within(centre, r) {
            if !self.loaded.contains(&col) && !self.intake.holds_pushed(col) {
                self.generate(col);
            }
        }
    }

    /// Generate every column noted local and not generated yet.
    pub fn generate_noted(&mut self) {
        let todo: Vec<(i32, i32)> =
            self.intake.local_columns().into_iter().filter(|c| !self.loaded.contains(c)).collect();
        for col in todo {
            self.generate(col);
        }
    }

    /// Take in everything received: pushed chunks and local notes between
    /// the block changes around them (`GameState::apply_world_deltas`).
    /// Returns how many chunk-stream packets came.
    pub fn take_in(&mut self) -> u32 {
        self.rc.poll();
        if let Some(joined) = self.rc.pending_joined_world.take() {
            self.world.apply_meta_rules(&joined.to_meta());
            self.biome = Some(BiomeGenerator::new(joined.seed));
            if joined.worldgen_mismatch_notice().is_none()
                && let Some(spawn) = joined.spawn
            {
                self.intake
                    .expect_notes(joined.chunk_note_radius, crate::chunk_stream::column_of(spawn));
            }
        }
        let chunks = std::mem::take(&mut self.rc.chunk_queue);
        let changes = std::mem::take(&mut self.rc.pending_block_changes);
        self.intake.count_undecodable(std::mem::take(&mut self.rc.undecodable_chunks));
        if let Some(state) = self.rc.latest_state.take() {
            self.intake.confirm_drops(state.last_acked_input);
            let me = self.rc.player_index();
            if let Some(p) = state.players.iter().find(|p| Some(p.player_index) == me) {
                let pos = glam::Vec3::new(p.x, p.y, p.z);
                if pos.is_finite() {
                    self.intake.set_server_centre(crate::chunk_stream::column_of(pos));
                }
            }
        }
        let before = self.intake.applied();
        for step in interleave(chunks, changes.len()) {
            match step {
                IntakeStep::Chunk(p) => {
                    self.stream_bytes += protocol::serialize_packet(protocol::PacketType::ChunkData, &*p).len();
                    if let Some(col) = self.intake.generate_before_chunk(&p, &self.loaded, &self.world) {
                        self.generate(col);
                    }
                    if let Some(biome) = self.biome.as_ref()
                        && self.intake.check_before_chunk(&mut self.world, &self.loaded, biome, &p).let_go()
                    {
                        self.loaded.remove(&(p.cx, p.cz));
                    }
                    self.intake.apply(&mut self.world, &mut self.loaded, &self.registry, &p);
                }
                IntakeStep::Local(col, hash) => {
                    self.stream_bytes += crate::chunk_push::build_local_note(col, hash).len();
                    self.notes_received += 1;
                    self.intake.note_local(col, hash);
                    if self.loaded.contains(&col) {
                        self.intake.column_held(col);
                    } else if self.generate_on_note && self.intake.is_local(col) {
                        self.generate(col);
                    }
                }
                // C3b-2 — a block view lands like a change
                // (`GameState::apply_world_deltas`).
                IntakeStep::View(v) => {
                    self.views_seen.push((*v).clone());
                    let [x, y, z] = v.cell;
                    if let Some(col) = self.intake.generate_before_at(x, z, &self.loaded, &self.world) {
                        self.generate(col);
                    }
                    if crate::chunk_stream::remote_change_is_loaded(&self.loaded, &self.world, x, z)
                        || self.intake.holds_chunk(crate::state_outbox::chunk_of_cell((x, y, z)))
                    {
                        crate::block_views::apply_view(&mut self.world, &self.registry, &v);
                    }
                }
                IntakeStep::Changes(r) => {
                    for bc in &changes[r] {
                        self.changes_seen.push(bc.clone());
                        if let Some(col) = self.intake.generate_before(bc, &self.loaded, &self.world) {
                            self.generate(col);
                        }
                        self.check_now(crate::chunk_stream::column_of_block(bc.x, bc.z));
                        if crate::chunk_stream::remote_change_is_loaded(
                            &self.loaded, &self.world, bc.x, bc.z,
                        ) || self.intake.holds_chunk(crate::state_outbox::chunk_of(bc))
                        {
                            self.world.apply_remote_block_change(bc);
                        }
                    }
                }
            }
        }
        self.run_checks();
        self.intake.applied() - before
    }

    /// The idle input `network_send_input` would send now: the chunk ack,
    /// the drop reports not yet applied and the current render distance.
    pub fn input(&mut self) -> protocol::InputPacket {
        let seq = self.rc.next_input_seq();
        protocol::InputPacket {
            health: 20.0,
            chunk_ack: self.intake.applied(),
            chunk_drops: self.intake.drops_for_input(seq, protocol::MAX_CHUNK_DROPS_PER_INPUT),
            render_distance: self.render_distance,
            column_mismatch: self.intake.column_mismatch(),
            ..Default::default()
        }
    }

    /// Send that input.
    pub fn ack(&mut self) {
        let input = self.input();
        self.rc.send_input(&input).expect("connected");
    }

    /// Let go of column `col` as the game loop's streamer does.
    pub fn let_go(&mut self, col: (i32, i32)) {
        self.intake.let_go(&mut self.world, col);
        self.loaded.remove(&col);
    }

    /// The streamer's unload pass (`stream_chunks`): let go of every pushed
    /// or local column — whole or part-pushed — more than `keep` columns
    /// from where this client stands. Returns how many.
    pub fn unload_beyond(&mut self, hs: &HostedServer, keep: i32) -> usize {
        let me = self.column(hs);
        let mut held: Vec<(i32, i32)> = self.loaded.iter().copied().collect();
        held.extend(self.intake.part_pushed_columns(&self.loaded));
        let far: Vec<(i32, i32)> = held
            .into_iter()
            .filter(|&(x, z)| (x - me.0).abs() > keep || (z - me.1).abs() > keep)
            .collect();
        for &col in &far {
            self.let_go(col);
        }
        far.len()
    }

    /// The streamer's unload pass for a CLIENT body in column `client_col`
    /// with render distance `rd` (`stream_chunks`): let go of every pushed or
    /// local column more than `rd + UNLOAD_HYSTERESIS` from it — except, with
    /// `keep_near_server` (the B2a verify NEW-1 rule), what
    /// `ChunkIntake::keeps_near_server_body` keeps. Returns how many.
    pub fn unload_round(&mut self, client_col: (i32, i32), rd: i32, keep_near_server: bool) -> usize {
        let keep = rd + crate::chunk_stream::UNLOAD_HYSTERESIS;
        let mut held: Vec<(i32, i32)> = self.loaded.iter().copied().collect();
        held.extend(self.intake.part_pushed_columns(&self.loaded));
        let far: Vec<(i32, i32)> = held
            .into_iter()
            .filter(|&(x, z)| (x - client_col.0).abs() > keep || (z - client_col.1).abs() > keep)
            .filter(|&col| !(keep_near_server && self.intake.keeps_near_server_body(col, rd)))
            .collect();
        for &col in &far {
            self.let_go(col);
        }
        far.len()
    }

    /// `ticks` frames of play standing still: input, tick, take in, unload
    /// beyond `keep`. Returns the chunk packets that came.
    pub fn play(&mut self, hs: &mut HostedServer, ticks: usize, keep: i32) -> u32 {
        let mut got = 0;
        for _ in 0..ticks {
            self.ack();
            hs.tick();
            got += self.take_in();
            self.unload_beyond(hs, keep);
        }
        got
    }

    /// Tick, take in, acknowledge — until three ticks in a row bring no
    /// chunk-stream packet. Returns the ticks it took.
    pub fn settle(&mut self, hs: &mut HostedServer) -> usize {
        self.settle_with(hs, HostedServer::tick)
    }

    pub fn settle_with(&mut self, hs: &mut HostedServer, mut tick: impl FnMut(&mut HostedServer)) -> usize {
        let mut quiet = 0;
        for ticks in 1..4_000 {
            self.ack();
            tick(hs);
            if self.take_in() == 0 {
                quiet += 1;
                if quiet == 3 {
                    return ticks;
                }
            } else {
                quiet = 0;
            }
        }
        panic!("the chunk push never settled");
    }

    /// The column the server holds this joiner's body in.
    pub fn column(&self, hs: &HostedServer) -> (i32, i32) {
        crate::chunk_stream::column_of(hs.server.players[self.slot].player.pos)
    }
}

/// Assert the joiner's chunk `c` is the server's, block for block.
pub(super) fn assert_chunk_matches(server: &World, joiner: &World, c: ChunkCoord) {
    let cs = CHUNK_SIZE as i32;
    for x in c.0 * cs..(c.0 + 1) * cs {
        for y in c.1 * cs..(c.1 + 1) * cs {
            for z in c.2 * cs..(c.2 + 1) * cs {
                assert_eq!(
                    joiner.get_block(x, y, z),
                    server.get_block(x, y, z),
                    "joiner's block at ({x},{y},{z}) differs from the server's"
                );
            }
        }
    }
}

pub(super) fn columns_within(centre: (i32, i32), r: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            out.push((centre.0 + dx, centre.1 + dz));
        }
    }
    out
}

/// Move slot `slot`'s server body `columns` columns east (west if negative).
pub(super) fn move_body(hs: &mut HostedServer, slot: usize, columns: i32) {
    let p = &mut hs.server.players[slot].player;
    p.pos.x += (columns * CHUNK_SIZE as i32) as f32;
    p.velocity = glam::Vec3::ZERO;
    p.reset_fall();
}

/// Fill `count` cells of chunk `c` (from its bottom layer up) with signs of
/// the longest text: about 134 bytes of side data each.
pub(super) fn fill_with_signs(world: &mut World, c: ChunkCoord, count: i32) {
    let cs = CHUNK_SIZE as i32;
    for i in 0..count {
        let at = (c.0 * cs + i % cs, c.1 * cs + i / (cs * cs), c.2 * cs + (i / cs) % cs);
        let mut sign = crate::sign::SignData::new();
        sign.set_text(&format!("{i:03}{}", "s".repeat(crate::sign::SIGN_MAX_CHARS - 3)));
        world.set_block(at.0, at.1, at.2, block::OAK_SIGN);
        world.insert_sign(at, sign);
    }
}

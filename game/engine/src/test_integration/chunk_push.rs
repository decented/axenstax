//! Phase B2a — the server pushes chunks to joiners (`chunk_push` on the
//! server, `chunk_intake` on the joiner; Spec 04 §4.1).
//!
//! These drive REAL joins through `HostedServer::tick` (channel transport, no
//! sockets) with a joiner built the way the game loop builds one: a
//! `RemoteClient`, a `World` it has generated nothing into, and a
//! `ChunkIntake` that applies the server's pushes in order with its block
//! changes and acknowledges them on its inputs.
//!
//! What they pin: an edit the joiner never saw reaches it inside a push; a
//! joiner whose generator differs from the host's gets every chunk in its
//! range; a tick never sends past the per-client budget and the credit
//! window; changes to chunks not yet sent never reach the joiner; the
//! sent-set dies with the connection; a lending host's push reads the host's
//! own world; a dropped column comes back afresh; and an outbox overflow is
//! healed by pushing the chunk again. From the B2a review: a chunk with heavy
//! side data never wedges the stream (HIGH-1); a hitch of more than ten
//! inputs in a tick still delivers their acks and drops (HIGH-2); a lowered
//! render distance never starts a push/drop churn (MEDIUM-1); columns go
//! whole, even as the body moves (LOW-3); and a spawn ring heavier than the
//! credit window still arrives before any acknowledgement (LOW-4).

use crate::block;
use crate::chunk::CHUNK_SIZE;
use crate::chunk_intake::{interleave, ChunkIntake, IntakeStep};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol::{self, BlockChange};
use crate::remote_client::{build_join_request_guest, RemoteClient};
use crate::state_outbox::{ChunkCoord, CLIENT_TICK_BUDGET_BYTES};
use crate::transport::ClientTransport;
use crate::world::{World, MAX_CHUNK_Y};

/// WebSocket + 0 remote slots: no accept thread, no sockets, the open
/// (guest) join policy. One local player (the host) at slot 0. The world name
/// must not exist on disk.
fn start_host(tag: &str) -> HostedServer {
    HostedServer::start(
        1,
        format!("chunk-push-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("hosted server starts")
}

/// A joined client as the game loop runs one, minus the renderer.
struct Joiner {
    rc: RemoteClient,
    world: World,
    loaded: ahash::AHashSet<(i32, i32)>,
    intake: ChunkIntake,
    registry: crate::block::BlockRegistry,
    /// Every server block change that reached this client.
    changes_seen: Vec<BlockChange>,
    slot: usize,
    /// The render distance its inputs carry (`InputPacket.render_distance`).
    render_distance: u8,
}

impl Joiner {
    /// Attach and join with `render_distance`; `mismatch` announces a terrain
    /// generator different from the host's.
    fn join(hs: &mut HostedServer, render_distance: u8, mismatch: bool) -> Self {
        Self::join_with(hs, render_distance, mismatch, HostedServer::tick)
    }

    fn join_with(
        hs: &mut HostedServer,
        render_distance: u8,
        mismatch: bool,
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
            slot: usize::MAX,
            render_distance,
        };
        j.take_in();
        j.slot = j.rc.player_index().expect("joined") as usize;
        j
    }

    /// Take in everything received: pushed chunks between the block changes
    /// around them (`GameState::apply_world_deltas`). Returns how many chunk
    /// packets came.
    fn take_in(&mut self) -> u32 {
        self.rc.poll();
        let chunks = std::mem::take(&mut self.rc.chunk_queue);
        let changes = std::mem::take(&mut self.rc.pending_block_changes);
        self.intake.count_undecodable(std::mem::take(&mut self.rc.undecodable_chunks));
        if let Some(state) = self.rc.latest_state.take() {
            self.intake.confirm_drops(state.last_acked_input);
        }
        let before = self.intake.applied();
        for step in interleave(chunks, changes.len()) {
            match step {
                IntakeStep::Chunk(p) => {
                    self.intake.apply(&mut self.world, &mut self.loaded, &self.registry, &p);
                }
                IntakeStep::Changes(r) => {
                    for bc in &changes[r] {
                        self.changes_seen.push(bc.clone());
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
        self.intake.applied() - before
    }

    /// The idle input `network_send_input` would send now: the chunk ack,
    /// the drop reports not yet applied and the current render distance.
    fn input(&mut self) -> protocol::InputPacket {
        let seq = self.rc.next_input_seq();
        protocol::InputPacket {
            health: 20.0,
            chunk_ack: self.intake.applied(),
            chunk_drops: self.intake.drops_for_input(seq, protocol::MAX_CHUNK_DROPS_PER_INPUT),
            render_distance: self.render_distance,
            ..Default::default()
        }
    }

    /// Send that input.
    fn ack(&mut self) {
        let input = self.input();
        self.rc.send_input(&input).expect("connected");
    }

    /// Let go of column `col` as the game loop's streamer does.
    fn let_go(&mut self, col: (i32, i32)) {
        self.intake.let_go(&mut self.world, col);
        self.loaded.remove(&col);
    }

    /// The streamer's unload pass (`stream_chunks`): let go of every pushed
    /// column — whole or part-pushed — more than `keep` columns from where
    /// this client stands. Returns how many.
    fn unload_beyond(&mut self, hs: &HostedServer, keep: i32) -> usize {
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

    /// `ticks` frames of play standing still: input, tick, take in, unload
    /// beyond `keep`. Returns the chunk packets that came.
    fn play(&mut self, hs: &mut HostedServer, ticks: usize, keep: i32) -> u32 {
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
    /// chunk. Returns the ticks it took.
    fn settle(&mut self, hs: &mut HostedServer) -> usize {
        self.settle_with(hs, HostedServer::tick)
    }

    fn settle_with(&mut self, hs: &mut HostedServer, mut tick: impl FnMut(&mut HostedServer)) -> usize {
        let mut quiet = 0;
        for ticks in 1..2_000 {
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
    fn column(&self, hs: &HostedServer) -> (i32, i32) {
        crate::chunk_stream::column_of(hs.server.players[self.slot].player.pos)
    }
}

/// Assert the joiner's chunk `c` is the server's, block for block.
fn assert_chunk_matches(server: &World, joiner: &World, c: ChunkCoord) {
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

fn columns_within(centre: (i32, i32), r: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            out.push((centre.0 + dx, centre.1 + dz));
        }
    }
    out
}

#[test]
fn a_host_edit_far_from_spawn_reaches_a_late_joiner() {
    let mut hs = start_host("far-edit");
    // Before anyone joins: the host builds five columns east of where a
    // joiner will stand — glass in the sky, a sign under it.
    let host = hs.server.players[0].player.pos;
    let (x, z) = (host.x.floor() as i32 + 3 + 5 * 16, host.z.floor() as i32);
    hs.server.world.set_block(x, 90, z, block::GLASS);
    hs.server.world.set_block(x, 89, z, block::OAK_PLANKS);
    let mut sign = crate::sign::SignData::new();
    sign.set_text("built before you came");
    hs.server.world.insert_sign((x, 89, z), sign);

    let mut j = Joiner::join(&mut hs, 6, false);
    j.settle(&mut hs);

    assert_eq!(j.world.get_block(x, 90, z), block::GLASS, "the far edit is in the joiner's world");
    assert_eq!(j.world.get_block(x, 89, z), block::OAK_PLANKS);
    assert_eq!(
        j.world.sign_at((x, 89, z)).map(|s| s.text.as_str()),
        Some("built before you came"),
        "with the sign's text"
    );
    let me = j.column(&hs);
    for col in columns_within(me, 6) {
        assert!(j.intake.column_complete(col), "column {col:?} pushed whole");
    }
    // …and a later edit there reaches it as a plain change.
    hs.server.world.set_block(x, 91, z, block::STONE);
    hs.server.pending_block_changes.push(BlockChange::with_meta(x, 91, z, block::STONE, 0));
    hs.tick();
    j.take_in();
    assert_eq!(j.world.get_block(x, 91, z), block::STONE);
}

#[test]
fn a_joiner_with_another_generator_gets_every_chunk_in_range_and_nothing_beyond() {
    let mut hs = start_host("mismatch");
    let mut j = Joiner::join(&mut hs, 3, true);
    assert!(hs.server.players[j.slot].worldgen_mismatch());
    j.settle(&mut hs);
    let me = j.column(&hs);
    let range = columns_within(me, 3);
    for &col in &range {
        assert!(j.intake.column_complete(col), "column {col:?} pushed");
        for cy in 0..=MAX_CHUNK_Y {
            assert_chunk_matches(&hs.server.world, &j.world, (col.0, cy, col.1));
        }
    }
    assert_eq!(
        hs.chunk_push_for_test(j.slot).sent_len(),
        range.len() * (MAX_CHUNK_Y as usize + 1),
        "exactly the joiner's range, every chunk of it"
    );
    // This joiner generates nothing: everything it holds was pushed, and it
    // holds exactly its range — on every side.
    assert_eq!(j.loaded.len(), range.len());
    assert!(j.loaded.iter().all(|&(x, z)| (x - me.0).abs() <= 3 && (z - me.1).abs() <= 3));
    assert!(j.intake.part_pushed_columns(&j.loaded).is_empty(), "no half column anywhere");
}

#[test]
fn a_tick_never_sends_past_the_budget_or_the_credit_window() {
    let mut hs = start_host("pacing");
    let client = hs.attach_test_remote();
    let mut req = build_join_request_guest("Paced", 0);
    req.render_distance = 8;
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
    let mut received = 0u32; // chunk packets
    let mut acked = 0u32;
    let mut first_packets = 0usize; // chunks (not continuations)
    let mut seq = 1;
    for tick in 0..400 {
        hs.tick();
        let mut bytes = 0usize;
        while let Some(pkt) = client.try_recv_from_server() {
            let (t, payload) = protocol::deserialize_header(&pkt).unwrap();
            if !matches!(t, protocol::PacketType::JoinAccept | protocol::PacketType::Challenge) {
                bytes += pkt.len();
            }
            if t == protocol::PacketType::ChunkData {
                received += 1;
                let c: protocol::ChunkDataPacket = protocol::safe_deserialize(payload).unwrap();
                first_packets += usize::from(!c.compressed_blocks.is_empty());
            }
        }
        assert!(bytes <= CLIENT_TICK_BUDGET_BYTES, "tick {tick}: {bytes} bytes");
        // The window is checked between columns: one may pass it.
        assert!(
            received - acked <= crate::chunk_push::CHUNK_WINDOW_PACKETS + MAX_CHUNK_Y as u32 + 1,
            "tick {tick}: {} chunk packets unacknowledged",
            received - acked
        );
        // A slow client: it acknowledges every fourth tick only.
        if tick % 4 == 3 {
            acked = received;
            let input = protocol::InputPacket {
                tick: seq,
                health: 20.0,
                chunk_ack: acked,
                ..Default::default()
            };
            seq += 1;
            client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        }
    }
    assert_eq!(first_packets, 17 * 17 * 6, "the whole radius-8 range arrived");
}

#[test]
fn changes_to_chunks_not_yet_sent_never_reach_the_joiner() {
    let mut hs = start_host("filter");
    let mut j = Joiner::join(&mut hs, 8, false);
    // No acknowledgement: the push stops at its window (the spawn ring + a bit).
    for _ in 0..10 {
        hs.tick();
        j.take_in();
    }
    let me = j.column(&hs);
    let cs = CHUNK_SIZE as i32;
    let near = (me.0 * cs + 8, 90, me.1 * cs + 8);
    let far = ((me.0 + 7) * cs + 8, 90, me.1 * cs + 8);
    assert!(j.intake.column_complete(me));
    assert!(!j.intake.holds_pushed((me.0 + 7, me.1)), "the far column is not sent yet");
    for (x, y, z) in [near, far] {
        hs.server.world.set_block(x, y, z, block::GLASS);
        hs.server.pending_block_changes.push(BlockChange::with_meta(x, y, z, block::GLASS, 0));
    }
    hs.tick();
    j.take_in();
    let seen = |c: (i32, i32, i32)| j.changes_seen.iter().any(|b| (b.x, b.y, b.z) == c);
    assert!(seen(near), "a change in a sent chunk is sent");
    assert!(!seen(far), "a change in an unsent chunk is not");
    assert_eq!(j.world.get_block(far.0, far.1, far.2), block::AIR, "and nothing landed there");
    // Once the joiner acknowledges, the push reaches the far column — with
    // the change in it.
    j.settle(&mut hs);
    assert_eq!(j.world.get_block(far.0, far.1, far.2), block::GLASS);
}

#[test]
fn the_sent_set_dies_with_the_connection_and_a_new_joiner_starts_afresh() {
    let mut hs = start_host("reuse");
    let mut j = Joiner::join(&mut hs, 2, false);
    j.settle(&mut hs);
    let slot = j.slot;
    assert_eq!(hs.chunk_push_for_test(slot).sent_len(), 25 * 6);
    j.rc.disconnect();
    hs.tick();
    assert!(hs.slot_is_free(slot));
    assert_eq!(hs.chunk_push_for_test(slot).sent_len(), 0, "released with the slot");

    let mut next = Joiner::join(&mut hs, 2, false);
    assert_eq!(next.slot, slot, "the freed slot is reused");
    next.settle(&mut hs);
    assert_eq!(next.intake.applied() as usize, 25 * 6, "everything again, from the start");
    assert!(next.intake.column_complete(next.column(&hs)));
}

#[test]
fn a_lending_hosts_push_reads_the_hosts_own_world_inside_the_lend_window() {
    let (mut hs, mut host) = super::lent_world::start_lent("chunk-push");
    // The host client's world (lent to the server each tick) carries a
    // marker the server's own between-window world can't have.
    let at = hs.server.players[0].player.pos;
    let (x, z) = (at.x.floor() as i32 + 3, at.z.floor() as i32);
    host.world.set_block(x, 92, z, block::DIAMOND_BLOCK);
    let mut j = Joiner::join_with(&mut hs, 2, false, |hs| host.lend_tick(hs));
    j.settle_with(&mut hs, |hs| host.lend_tick(hs));
    assert_eq!(j.world.get_block(x, 92, z), block::DIAMOND_BLOCK, "pushed from the host's world");
    let me = j.column(&hs);
    for col in columns_within(me, 2) {
        for cy in 0..=MAX_CHUNK_Y {
            assert_chunk_matches(&host.world, &j.world, (col.0, cy, col.1));
        }
    }
    assert!(
        (0..=MAX_CHUNK_Y).all(|cy| !hs.server.world.has_chunk(me.0, cy, me.1)),
        "outside the window the server holds no world to push from"
    );
}

#[test]
fn a_column_the_joiner_lets_go_of_is_pushed_afresh() {
    let mut hs = start_host("drop");
    let mut j = Joiner::join(&mut hs, 2, false);
    j.settle(&mut hs);
    let me = j.column(&hs);
    let col = (me.0 + 2, me.1);
    let cs = CHUNK_SIZE as i32;
    let cell = (col.0 * cs + 3, 93, col.1 * cs + 3);
    j.let_go(col);
    // While it is gone the server changes it; the joiner reports the drop.
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::GLASS);
    // (The change is made with no broadcast: only a fresh push can carry it,
    // and only a drop the server took makes it push the column again.)
    j.settle(&mut hs);
    assert!(!hs.chunk_push_for_test(j.slot).has_sent((col.0, 0, col.1)), "the drop was taken");
    assert!(
        !j.intake.holds_pushed(col),
        "let go inside the radius: not pushed straight back (no churn)"
    );
    // The body walks off until the column is out of range, and comes back.
    move_body(&mut hs, j.slot, -5);
    j.settle(&mut hs);
    move_body(&mut hs, j.slot, 5);
    j.settle(&mut hs);
    assert!(j.intake.column_complete(col), "pushed again once back in range");
    assert_eq!(j.world.get_block(cell.0, cell.1, cell.2), block::GLASS, "with what changed meanwhile");
}

/// Move slot `slot`'s server body `columns` columns east (west if negative).
fn move_body(hs: &mut HostedServer, slot: usize, columns: i32) {
    let p = &mut hs.server.players[slot].player;
    p.pos.x += (columns * CHUNK_SIZE as i32) as f32;
    p.velocity = glam::Vec3::ZERO;
    p.reset_fall();
}

/// Fill `count` cells of chunk `c` (from its bottom layer up) with signs of
/// the longest text: about 134 bytes of side data each.
fn fill_with_signs(world: &mut World, c: ChunkCoord, count: i32) {
    let cs = CHUNK_SIZE as i32;
    for i in 0..count {
        let at = (c.0 * cs + i % cs, c.1 * cs + i / (cs * cs), c.2 * cs + (i / cs) % cs);
        let mut sign = crate::sign::SignData::new();
        sign.set_text(&format!("{i:03}{}", "s".repeat(crate::sign::SIGN_MAX_CHARS - 3)));
        world.set_block(at.0, at.1, at.2, block::OAK_SIGN);
        world.insert_sign(at, sign);
    }
}

#[test]
fn a_chunk_with_heavy_side_data_never_wedges_the_joiners_stream() {
    // Review HIGH-1: about 50 KiB of signs in one chunk. It used to go as one
    // packet that never fitted what was left of a tick, holding back every
    // change and push behind it for good.
    let mut hs = start_host("heavy");
    let host = hs.server.players[0].player.pos;
    let home = crate::chunk_stream::column_of(host);
    let heavy = (home.0 + 1, 5, home.1);
    fill_with_signs(&mut hs.server.world, heavy, 400);
    let mut j = Joiner::join(&mut hs, 3, false);
    j.settle(&mut hs);
    let me = j.column(&hs);
    for col in columns_within(me, 3) {
        assert!(j.intake.column_complete(col), "column {col:?}: the push flowed past the heavy chunk");
    }
    let cs = CHUNK_SIZE as i32;
    for i in [0, 199, 399] {
        let at = (heavy.0 * cs + i % cs, heavy.1 * cs + i / (cs * cs), heavy.2 * cs + (i / cs) % cs);
        assert!(
            j.world.sign_at(at).is_some_and(|s| s.text.starts_with(&format!("{i:03}"))),
            "sign {i} arrived"
        );
    }
    // And later changes still reach it.
    let far = ((me.0 - 3) * cs + 2, 95, me.1 * cs + 2);
    hs.server.world.set_block(far.0, far.1, far.2, block::GLASS);
    hs.server.pending_block_changes.push(BlockChange::with_meta(far.0, far.1, far.2, block::GLASS, 0));
    hs.tick();
    j.take_in();
    assert_eq!(j.world.get_block(far.0, far.1, far.2), block::GLASS);
}

#[test]
fn a_hitch_of_more_than_ten_inputs_in_a_tick_still_delivers_acks_and_drops() {
    // Review HIGH-2: the server reads at most ten packets a tick from a
    // client; the ack and drops of the ones past that used to be lost.
    let mut hs = start_host("hitch");
    let mut j = Joiner::join(&mut hs, 8, false);
    for _ in 0..5 {
        hs.tick();
        j.take_in();
    }
    let push = hs.chunk_push_for_test(j.slot);
    assert!(push.pushed() > push.acked(), "the window is waiting on an ack");
    let me = j.column(&hs);
    let col = (me.0 + 1, me.1);
    assert!(j.intake.column_complete(col));
    j.let_go(col);
    // A frame hitch: eleven stale inputs, then the one with the ack and drop.
    for _ in 0..11 {
        let input = protocol::InputPacket { health: 20.0, ..Default::default() };
        j.rc.send_input(&input).expect("connected");
    }
    let last = j.input();
    assert_eq!(last.chunk_drops.len(), 1);
    j.rc.send_input(&last).expect("connected");
    hs.tick();
    let push = hs.chunk_push_for_test(j.slot);
    assert_eq!(push.acked(), last.chunk_ack, "the twelfth input's ack was taken");
    assert!(!push.has_sent((col.0, 0, col.1)), "and its drop");
    assert!(j.take_in() > 0, "the window reopened");
}

#[test]
fn a_lowered_render_distance_never_starts_a_push_and_drop_churn() {
    // Review MEDIUM-1: the push radius followed the JOIN render distance, so
    // a joiner that lowered its own kept unloading what the server pushed
    // straight back — push, unload, drop, push — the whole session long.
    let mut hs = start_host("churn");
    let mut j = Joiner::join(&mut hs, 6, false);
    j.settle(&mut hs);
    assert_eq!(j.unload_beyond(&hs, 6 + crate::chunk_stream::UNLOAD_HYSTERESIS), 0);
    // First a client that keeps less than it says (a stale render distance:
    // it still sends 6 but keeps 5): the server holds the columns it lets go
    // of inside the radius off; it does not push them back.
    assert!(j.unload_beyond(&hs, 5) > 0);
    j.play(&mut hs, 5, 5);
    assert_eq!(j.play(&mut hs, 60, 5), 0, "nothing pushed back: no churn");
    // Then the real thing: render distance 2, so it keeps 2 + 2. The radius
    // follows, so nothing comes back either.
    j.render_distance = 2;
    let keep = 2 + crate::chunk_stream::UNLOAD_HYSTERESIS;
    assert!(j.unload_beyond(&hs, keep) > 0);
    j.play(&mut hs, 5, keep);
    assert_eq!(j.play(&mut hs, 60, keep), 0, "no churn at the lower radius");
    assert_eq!(
        hs.chunk_push_for_test(j.slot).radius(crate::chunk_stream::LENT_JOINER_SIM_DISTANCE),
        2,
        "the radius follows the client's current render distance"
    );
    assert_eq!(j.intake.pending_drops(), 0, "every report applied and retired");
    // And raising it again pushes the wider range.
    j.render_distance = 6;
    j.play(&mut hs, 60, 6 + crate::chunk_stream::UNLOAD_HYSTERESIS);
    let me = j.column(&hs);
    for col in columns_within(me, 6) {
        assert!(j.intake.column_complete(col), "column {col:?} pushed again at render distance 6");
    }
}

#[test]
fn columns_go_whole_even_while_the_body_moves() {
    // Review LOW-3: the plan used to stop mid-column, and a column then left
    // at the frontier stayed half-pushed.
    let mut hs = start_host("frontier");
    let mut j = Joiner::join(&mut hs, 4, false);
    for step in 0..60 {
        if step % 3 == 0 && step < 30 {
            move_body(&mut hs, j.slot, 1);
        }
        // A slow client: acknowledges every other tick.
        if step % 2 == 0 {
            j.ack();
        }
        hs.tick();
        j.take_in();
        assert!(
            hs.chunk_push_for_test(j.slot).sent_columns_are_whole(),
            "step {step}: the server never leaves a half column sent"
        );
    }
    j.settle(&mut hs);
    assert!(j.intake.part_pushed_columns(&j.loaded).is_empty(), "the joiner holds no half column");
}

#[test]
fn a_spawn_ring_heavier_than_the_credit_window_arrives_with_no_ack() {
    // Review LOW-4: a loading joiner sends no input, so no ack; a ring
    // needing more than 64 packets used to wait out the 30 s loading limit.
    let mut hs = start_host("heavy-ring");
    let host = hs.server.players[0].player.pos;
    let home = crate::chunk_stream::column_of(host);
    for col in columns_within(home, 1) {
        for cy in 0..=MAX_CHUNK_Y {
            fill_with_signs(&mut hs.server.world, (col.0, cy, col.1), 300);
        }
    }
    let mut j = Joiner::join(&mut hs, 2, false);
    assert_eq!(j.column(&hs), home, "the joiner stands in the host's column");
    let mut packets = 0;
    for _ in 0..200 {
        hs.tick();
        packets += j.take_in();
        if columns_within(j.column(&hs), 1).iter().all(|&c| j.intake.column_complete(c)) {
            break;
        }
    }
    assert!(packets > crate::chunk_push::CHUNK_WINDOW_PACKETS, "{packets}: more than the window");
    for col in columns_within(j.column(&hs), 1) {
        assert!(j.intake.column_complete(col), "ring column {col:?} arrived without an ack");
    }
}

#[test]
fn an_outbox_overflow_is_healed_by_pushing_the_chunks_again() {
    let mut hs = start_host("resync");
    let mut j = Joiner::join(&mut hs, 1, false);
    j.settle(&mut hs);
    let me = j.column(&hs);
    let cs = CHUNK_SIZE as i32;
    // One tick changes every cell of the ring's upper four chunks: ~147,000
    // changes, past the queue's 2 MiB bound.
    let mut burst = Vec::new();
    for col in columns_within(me, 1) {
        for cy in 2..=MAX_CHUNK_Y {
            for x in col.0 * cs..(col.0 + 1) * cs {
                for y in cy * cs..(cy + 1) * cs {
                    for z in col.1 * cs..(col.1 + 1) * cs {
                        hs.server.world.set_block(x, y, z, block::GLASS);
                        burst.push(BlockChange::with_meta(x, y, z, block::GLASS, 0));
                    }
                }
            }
        }
    }
    hs.server.pending_block_changes.extend(burst);
    let before = j.intake.applied();
    j.settle(&mut hs);
    assert!(j.intake.applied() > before, "the overflowed chunks were pushed again");
    for col in columns_within(me, 1) {
        for cy in 0..=MAX_CHUNK_Y {
            assert_chunk_matches(&hs.server.world, &j.world, (col.0, cy, col.1));
        }
    }
}

//! Bounded server→client state (gap-audit T1-5, 2026-10-06).
//!
//! Before this, `HostedServer::broadcast_state` put EVERY block change and
//! entity event of a tick into ONE `StateUpdate`. A crop field, a piston
//! array, a `/we` edit or a late-join backfill could push it past
//! `protocol::MAX_PACKET_SIZE` (64 KiB — about 4,300 block changes), and every
//! client's `safe_deserialize` then dropped the whole packet: blocks, spawns,
//! despawns, player positions, all of it.
//!
//! These tests drive a REAL join through `HostedServer::tick` (channel
//! transport, no sockets) and pin the fix: every StateUpdate stays under the
//! cap, a remote client gets at most a fixed byte budget per tick, the excess
//! drains on later ticks in order, a backlog coalesces repeated edits
//! (latest wins), entity spawns/despawns survive splitting in order, and a
//! queue past its hard bound drops its block changes into a per-client
//! "needs resync" set — the seam the late-joiner chunk push consumes.
//!
//! Injected bursts live in a marker region (`x >= MARK_X`) so the server's
//! own per-tick changes (water, leaf decay, …) never confuse a count.

use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol::{self, BlockChange, StateUpdatePacket};
use crate::state_outbox::{CLIENT_QUEUE_MAX_BYTES, CLIENT_TICK_BUDGET_BYTES, STATE_UPDATE_MAX_BYTES};
use crate::transport::{ChannelClientTransport, ClientTransport};

/// Injected changes sit at `x >= MARK_X` — far outside anything the server's
/// own sim touches.
const MARK_X: i32 = 1_000_000;

/// WebSocket + 0 remote slots: no accept thread, no sockets, and the open
/// (guest) join policy. The world name must not exist on disk.
///
/// These pin the outbox on its own, so the B2a chunk push is off: no pushes
/// share the budget, every change reaches the joiner (the push's sent-set
/// filter is pinned in `chunk_push`), and an overflow's resync requests stay
/// readable here.
fn start_open_server(world: &str) -> HostedServer {
    let mut hs = HostedServer::start(1, world.to_string(), 42, 0, RemoteTransport::WebSocket { port: 0 })
        .expect("hosted server starts");
    hs.without_chunk_push_for_test();
    hs
}

/// Attach a remote and complete a guest join. Returns the client half and the
/// join tick's StateUpdates (the late-join backfill rides those). The remote's
/// slot is 1 (one local player).
fn join_remote_keeping(hs: &mut HostedServer) -> (ChannelClientTransport, Vec<Vec<u8>>) {
    let client = hs.attach_test_remote();
    let req = crate::remote_client::build_join_request_guest("Budget", 0);
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
    hs.tick();
    let join_tick = drain_raw_state_updates(&client);
    (client, join_tick)
}

/// [`join_remote_keeping`], discarding the join tick's packets.
fn join_remote(hs: &mut HostedServer) -> ChannelClientTransport {
    join_remote_keeping(hs).0
}

/// Every raw StateUpdate packet waiting on `client`, in arrival order.
fn drain_raw_state_updates(client: &ChannelClientTransport) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((protocol::PacketType::StateUpdate, _)) = protocol::deserialize_header(&pkt) {
            out.push(pkt);
        }
    }
    out
}

/// Decode a raw StateUpdate exactly as a client does (`safe_deserialize`,
/// capped at `MAX_PACKET_SIZE`) — a packet over the cap fails here just as it
/// failed on every real client.
fn decode(pkt: &[u8]) -> StateUpdatePacket {
    let (_, payload) = protocol::deserialize_header(pkt).expect("tagged");
    protocol::safe_deserialize::<StateUpdatePacket>(payload)
        .unwrap_or_else(|e| panic!("a {}-byte StateUpdate must decode: {e}", pkt.len()))
}

/// Assert the per-packet cap on every packet of one tick, and (for a remote)
/// the per-tick byte budget. Returns the decoded packets.
fn check_tick(raw: &[Vec<u8>], remote: bool) -> Vec<StateUpdatePacket> {
    assert!(!raw.is_empty(), "every joined client gets at least one StateUpdate a tick");
    for p in raw {
        assert!(
            p.len() <= STATE_UPDATE_MAX_BYTES,
            "a {}-byte StateUpdate is over the {STATE_UPDATE_MAX_BYTES}-byte cap",
            p.len()
        );
        assert!(p.len() <= protocol::MAX_WIRE_PACKET_LEN);
    }
    if remote {
        let total: usize = raw.iter().map(Vec::len).sum();
        assert!(
            total <= CLIENT_TICK_BUDGET_BYTES,
            "{total} bytes in one tick is over the {CLIENT_TICK_BUDGET_BYTES}-byte budget"
        );
    }
    raw.iter().map(|p| decode(p)).collect()
}

fn marked(states: &[StateUpdatePacket]) -> Vec<BlockChange> {
    states
        .iter()
        .flat_map(|s| s.block_changes.iter())
        .filter(|b| b.x >= MARK_X)
        .cloned()
        .collect()
}

/// `n` distinct marker cells, in a fixed order.
fn burst(n: usize, block: u16) -> Vec<BlockChange> {
    (0..n as i32)
        .map(|i| BlockChange::with_meta(MARK_X + i % 64, 10 + (i / 64) % 64, i / 4096, block, 0))
        .collect()
}

#[test]
fn a_10k_change_tick_reaches_a_joiner_whole_over_several_ticks_under_the_cap() {
    let mut hs = start_open_server("state-budget-10k-test");
    let client = join_remote(&mut hs);

    let sent = burst(10_000, crate::block::STONE);
    hs.server.pending_block_changes.extend(sent.iter().cloned());

    let mut got: Vec<BlockChange> = Vec::new();
    let mut ticks_with_marked = 0;
    for _ in 0..40 {
        hs.tick();
        let states = check_tick(&drain_raw_state_updates(&client), true);
        let m = marked(&states);
        if !m.is_empty() {
            ticks_with_marked += 1;
        }
        got.extend(m);
        if got.len() >= sent.len() {
            break;
        }
    }
    let key = |b: &BlockChange| (b.x, b.y, b.z, b.new_block);
    assert_eq!(
        got.iter().map(key).collect::<Vec<_>>(),
        sent.iter().map(key).collect::<Vec<_>>(),
        "every change arrives exactly once, in the order the server produced them"
    );
    assert!(
        ticks_with_marked >= 3,
        "150 KB at a 48 KiB/tick budget must spread over several ticks, got {ticks_with_marked}"
    );
}

#[test]
fn the_host_loopback_gets_a_burst_in_one_tick_split_under_the_cap() {
    let mut hs = start_open_server("state-budget-loopback-test");
    hs.tick();
    while hs.local_transports[0].try_recv_from_server().is_some() {}

    let sent = burst(10_000, crate::block::STONE);
    hs.server.pending_block_changes.extend(sent.iter().cloned());
    hs.tick();
    let raw = drain_raw_state_updates(&hs.local_transports[0]);
    assert!(raw.len() >= 3, "10,000 changes need several packets, got {}", raw.len());
    let states = check_tick(&raw, false);
    assert_eq!(
        marked(&states).len(),
        sent.len(),
        "the in-process host loopback has no wire to protect: it drains in one tick"
    );
}

#[test]
fn repeated_edits_to_one_cell_arrive_in_order_when_nothing_is_backlogged() {
    let mut hs = start_open_server("state-budget-order-test");
    let client = join_remote(&mut hs);

    let cell = (MARK_X, 20, 5);
    for v in [crate::block::STONE, crate::block::DIRT, crate::block::GLASS] {
        hs.server
            .pending_block_changes
            .push(BlockChange::with_meta(cell.0, cell.1, cell.2, v, 0));
    }
    hs.tick();
    let got: Vec<u16> =
        marked(&check_tick(&drain_raw_state_updates(&client), true)).iter().map(|b| b.new_block).collect();
    assert_eq!(
        got,
        vec![crate::block::STONE, crate::block::DIRT, crate::block::GLASS],
        "an unbacklogged queue delivers exactly the produced sequence (no coalescing)"
    );
}

#[test]
fn a_backlogged_queue_coalesces_repeated_edits_latest_wins() {
    let mut hs = start_open_server("state-budget-coalesce-test");
    let client = join_remote(&mut hs);

    // A big burst first, so the joiner is backlogged…
    let filler = burst(20_000, crate::block::STONE);
    hs.server.pending_block_changes.extend(filler.iter().cloned());
    hs.tick();
    // …then a cell flickers many times while the backlog drains.
    let cell = (MARK_X + 500, 30, 7);
    let mut last = 0;
    let mut got = Vec::new();
    for round in 0..6u16 {
        for v in 0..20u16 {
            last = 100 + round * 20 + v;
            hs.server
                .pending_block_changes
                .push(BlockChange::with_meta(cell.0, cell.1, cell.2, last, 0));
        }
        got.extend(marked(&check_tick(&drain_raw_state_updates(&client), true)));
        hs.tick();
    }
    for _ in 0..60 {
        got.extend(marked(&check_tick(&drain_raw_state_updates(&client), true)));
        hs.tick();
    }
    got.extend(marked(&check_tick(&drain_raw_state_updates(&client), true)));

    let at_cell: Vec<u16> = got
        .iter()
        .filter(|b| (b.x, b.y, b.z) == cell)
        .map(|b| b.new_block)
        .collect();
    assert_eq!(at_cell.last().copied(), Some(last), "the cell ends on its latest value");
    assert!(
        at_cell.len() < 120,
        "a backlog coalesces repeated edits to one cell (got all {} of 120)",
        at_cell.len()
    );
    // Every filler cell still arrives (coalescing never loses a distinct cell).
    let distinct: std::collections::HashSet<(i32, i32, i32)> =
        got.iter().map(|b| (b.x, b.y, b.z)).collect();
    for b in &filler {
        assert!(distinct.contains(&(b.x, b.y, b.z)), "filler cell {:?} lost", (b.x, b.y, b.z));
    }
}

#[test]
fn entity_spawns_and_despawns_survive_splitting_in_order() {
    let mut hs = start_open_server("state-budget-entities-test");
    // Keep the join tick: its entity spawns are part of the order.
    let (client, join_tick) = join_remote_keeping(&mut hs);
    // Round the joiner's body, inside its interest radius (MP-D2a): a 100 x
    // 80 grid centred on it reaches at most 64 blocks out.
    let centre = hs.server.players.last().expect("the joiner's slot").player.pos;

    // A block backlog in front, then 2,000 drops — far more than one budget.
    hs.server.pending_block_changes.extend(burst(8_000, crate::block::STONE));
    for i in 0..2_000u32 {
        let at = glam::Vec3::new(
            centre.x - 50.0 + (i % 50) as f32 * 2.0,
            120.0,
            centre.z - 40.0 + (i / 50) as f32 * 2.0,
        );
        crate::entity::spawn_item(
            &mut hs.server.ecs,
            at,
            crate::item::ItemStack::new_material(crate::item::MaterialId::Bone, 1),
            i,
        );
    }
    hs.tick(); // all 2,000 spawns enter the diff this tick
    // Kill every third one while its spawn is (mostly) still queued.
    let mut items: Vec<hecs::Entity> = hs
        .server
        .ecs
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    items.sort_by_key(|e| e.id());
    assert!(items.len() >= 2_000);
    for e in items.iter().step_by(3) {
        let _ = hs.server.ecs.despawn(*e);
    }

    // Delivery order as the client sees it: per packet, spawns → updates →
    // despawns (the client's apply order).
    #[derive(Debug, PartialEq)]
    enum Ev {
        Spawn(u32),
        Update(u32),
        Despawn(u32),
    }
    let mut evs = Vec::new();
    let mut ticks = vec![join_tick];
    for _ in 0..80 {
        ticks.push(drain_raw_state_updates(&client));
        hs.tick();
    }
    for raw in &ticks {
        for s in check_tick(raw, true) {
            evs.extend(s.entity_spawns.iter().map(|e| Ev::Spawn(e.id)));
            evs.extend(s.entity_updates.iter().map(|e| Ev::Update(e.id)));
            evs.extend(s.entity_despawns.iter().map(|&id| Ev::Despawn(id)));
        }
    }

    let mut spawned = std::collections::HashSet::new();
    let mut despawned = std::collections::HashSet::new();
    for ev in &evs {
        match *ev {
            Ev::Spawn(id) => assert!(spawned.insert(id), "spawn {id} delivered twice"),
            Ev::Update(id) => {
                assert!(spawned.contains(&id), "update for {id} before its spawn");
                assert!(!despawned.contains(&id), "update for {id} after its despawn");
            }
            Ev::Despawn(id) => {
                assert!(spawned.contains(&id), "despawn {id} before its spawn");
                assert!(despawned.insert(id), "despawn {id} delivered twice");
            }
        }
    }
    let bones = spawned.len();
    assert!(bones >= 2_000, "all 2,000 drops reach the joiner, got {bones} spawns");
    assert!(
        despawned.len() >= items.len().div_ceil(3),
        "every killed drop's despawn reaches the joiner, got {}",
        despawned.len()
    );
}

#[test]
fn an_overflowing_joiner_queue_drops_its_blocks_into_the_resync_set() {
    let mut hs = start_open_server("state-budget-overflow-test");
    let client = join_remote(&mut hs);
    let remote_slot = 1;

    // More distinct cells in one tick than the hard bound holds.
    let n = CLIENT_QUEUE_MAX_BYTES / 15 + 1_000;
    let sent: Vec<BlockChange> = (0..n as i32)
        .map(|i| {
            BlockChange::with_meta(
                MARK_X + i % 256,
                16 + (i / 256) % 32,
                i / 8192,
                crate::block::STONE,
                0,
            )
        })
        .collect();
    hs.server.pending_block_changes.extend(sent.iter().cloned());

    let mut got = std::collections::HashSet::new();
    for _ in 0..10 {
        hs.tick();
        for b in marked(&check_tick(&drain_raw_state_updates(&client), true)) {
            got.insert((b.x, b.y, b.z));
        }
    }
    assert_eq!(hs.queued_state_bytes(remote_slot), 0, "the joiner's queue is empty again");
    assert!(
        got.len() < n / 2,
        "the overflowed history is dropped, not trickled out ({} of {n} arrived)",
        got.len()
    );

    let resync: std::collections::BTreeSet<(i32, i32, i32)> =
        hs.take_chunk_resync_requests(remote_slot).into_iter().collect();
    let chunk = |b: &BlockChange| (b.x.div_euclid(16), b.y.div_euclid(16), b.z.div_euclid(16));
    let all_chunks: std::collections::BTreeSet<_> = sent.iter().map(chunk).collect();
    assert!(!resync.is_empty());
    assert!(resync.is_subset(&all_chunks), "only chunks the burst touched");
    for b in &sent {
        if !got.contains(&(b.x, b.y, b.z)) {
            assert!(
                resync.contains(&chunk(b)),
                "a change the joiner never got ({:?}) has no resync request for its chunk",
                (b.x, b.y, b.z)
            );
        }
    }
    assert!(hs.take_chunk_resync_requests(remote_slot).is_empty(), "take clears the set");
    // The in-process host loopback has nothing to resync: it drained in full.
    assert!(hs.take_chunk_resync_requests(0).is_empty());
}

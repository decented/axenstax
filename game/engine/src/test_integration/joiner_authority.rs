//! Host authority over joiners (bug-squash wave 1, audit 2026-09-27).
//!
//! Drives REAL joins through `HostedServer::tick` over the in-process channel
//! transport and pins what the host now enforces for a remote player:
//! play mode, economy-block ownership, plot protection, reach and the per-tick
//! edit budget; a container broken by a joiner spills; and slots are freed on
//! disconnect / reject / pre-auth timeout / kick and reused, so the server
//! never fills with ghosts.

use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol;
use crate::transport::{ChannelClientTransport, ClientTransport};

/// WebSocket + 0 remote slots: no accept thread, no sockets, open (guest)
/// join policy. The world name must not exist on disk.
pub(super) fn start_open_server(tag: &str) -> HostedServer {
    HostedServer::start(
        1,
        format!("joiner-authority-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("hosted server starts")
}

pub(super) fn send_guest_join(client: &ChannelClientTransport, name: &str) {
    let req = crate::remote_client::build_join_request_guest(name, 0);
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
}

/// Attach + join a guest; returns (client, slot). The join's chunk push is
/// settled ([`settle_chunk_push`]): the ground round the joiner is its own,
/// so changes there reach it, as they do a real joiner once it has loaded.
pub(super) fn join_guest(hs: &mut HostedServer, name: &str) -> (ChannelClientTransport, usize) {
    let client = hs.attach_test_remote();
    send_guest_join(&client, name);
    hs.tick();
    let slot = accepted_index(&client).expect("guest join accepted");
    settle_chunk_push(hs, &client, HostedServer::tick);
    (client, slot)
}

/// B2a — tick (with `tick`, e.g. `HostedServer::tick` or a lend window)
/// until a tick brings `client` no chunk push. The test client acknowledges
/// nothing, so the push stops at its credit window: the 3×3 columns round
/// the joiner (which go first) and a little more. Everything `client`
/// receives meanwhile is discarded.
pub(super) fn settle_chunk_push<T>(
    target: &mut T,
    client: &ChannelClientTransport,
    mut tick: impl FnMut(&mut T),
) {
    for _ in 0..32 {
        let mut pushed = false;
        while let Some(pkt) = client.try_recv_from_server() {
            pushed |= matches!(
                protocol::deserialize_header(&pkt),
                Some((protocol::PacketType::ChunkData | protocol::PacketType::ColumnLocal, _))
            );
        }
        if !pushed {
            return;
        }
        tick(target);
    }
    panic!("the chunk push never settled");
}

/// Attach + join a guest; returns (client, slot, the `JoinAccept` it got).
pub(super) fn join_guest_accept(
    hs: &mut HostedServer,
    name: &str,
) -> (ChannelClientTransport, usize, protocol::JoinAcceptPacket) {
    let client = hs.attach_test_remote();
    send_guest_join(&client, name);
    hs.tick();
    let accept = accepted(&client).expect("guest join accepted");
    let slot = accept.player_index as usize;
    settle_chunk_push(hs, &client, HostedServer::tick);
    (client, slot, accept)
}

pub(super) fn accepted(client: &ChannelClientTransport) -> Option<protocol::JoinAcceptPacket> {
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
            && ptype == protocol::PacketType::JoinAccept
            && let Ok(a) = protocol::safe_deserialize::<protocol::JoinAcceptPacket>(payload)
        {
            return Some(a);
        }
    }
    None
}

fn accepted_index(client: &ChannelClientTransport) -> Option<usize> {
    accepted(client).map(|a| a.player_index as usize)
}

fn got_reject(client: &ChannelClientTransport) -> Option<String> {
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
            && ptype == protocol::PacketType::JoinReject
            && let Ok(r) = protocol::safe_deserialize::<protocol::JoinRejectPacket>(payload)
        {
            return Some(r.reason);
        }
    }
    None
}

pub(super) fn block_changes_seen(client: &ChannelClientTransport) -> Vec<protocol::BlockChange> {
    let mut out = Vec::new();
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
            && ptype == protocol::PacketType::StateUpdate
            && let Ok(s) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
        {
            out.extend(s.block_changes);
        }
    }
    out
}

/// A cell right next to where the server holds the joiner.
pub(super) fn cell_beside(hs: &HostedServer, slot: usize, dx: i32, dy: i32, dz: i32) -> (i32, i32, i32) {
    let at = hs.server.players[slot].player.pos;
    (at.x.floor() as i32 + dx, at.y.floor() as i32 + dy, at.z.floor() as i32 + dz)
}

/// Send one ClientInput carrying `edits`.
pub(super) fn send_edits(
    hs: &HostedServer,
    client: &ChannelClientTransport,
    slot: usize,
    tick: u64,
    edits: &[((i32, i32, i32), block::BlockId)],
) {
    let at = hs.server.players[slot].player.pos;
    let mut input = protocol::InputPacket {
        tick,
        x: at.x,
        y: at.y,
        z: at.z,
        health: 20.0,
        ..Default::default()
    };
    for &((x, y, z), b) in edits {
        input.block_changes.push(protocol::BlockChange { x, y, z, new_block: b, meta: 0 });
    }
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
}

#[test]
fn an_adventure_world_refuses_a_joiner_edit_and_sends_the_block_back() {
    let mut hs = start_open_server("adventure");
    hs.server.set_play_mode(crate::play_mode::PlayMode::Adventure);
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::STONE);
    let _ = block_changes_seen(&client);

    send_edits(&hs, &client, slot, 1, &[(cell, block::AIR)]);
    hs.tick();

    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::STONE, "read-only world");
    let back = block_changes_seen(&client);
    assert!(
        back.iter().any(|bc| (bc.x, bc.y, bc.z) == cell && bc.new_block == block::STONE),
        "the refused edit is un-ghosted with the authoritative block"
    );
}

#[test]
fn a_joiner_cannot_touch_a_vendor_they_dont_own_but_the_owning_npub_can() {
    let mut hs = start_open_server("vendor");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::VENDOR_BLOCK);
    hs.server.world.insert_vendor(
        cell,
        crate::vendor::VendorData {
            owner: Some(crate::vendor::VendorOwner::LocalPlayer(0)),
            ..Default::default()
        },
    );

    send_edits(&hs, &client, slot, 1, &[(cell, block::AIR)]);
    hs.tick();
    assert_eq!(
        hs.server.world.get_block(cell.0, cell.1, cell.2),
        block::VENDOR_BLOCK,
        "the host's vendor survives a joiner's break"
    );

    // A vendor owned by the joiner's own verified npub is theirs to break.
    let pk = [0x5a; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    hs.server.world.vendor_at_mut(cell).unwrap().owner =
        Some(crate::vendor::VendorOwner::Npub(crate::hosted_server::pubkey_to_npub(&pk)));
    send_edits(&hs, &client, slot, 2, &[(cell, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR);
}

#[test]
fn plot_protection_holds_against_joiners_on_the_host() {
    let mut hs = start_open_server("plot");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::AIR);
    hs.server.world.plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        cell.0,
        cell.1 - 5,
        cell.2,
    ));

    send_edits(&hs, &client, slot, 1, &[(cell, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR, "foreign plot");

    // The plot's npub owner may build in it.
    let pk = [0x6b; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    hs.server.world.plots[0].owner =
        crate::plot::PlotOwner::Npub(crate::hosted_server::pubkey_to_npub(&pk));
    send_edits(&hs, &client, slot, 2, &[(cell, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::STONE, "own plot");
}

#[test]
fn an_edit_beyond_reach_is_refused() {
    let mut hs = start_open_server("reach");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let far = cell_beside(&hs, slot, 9, 0, 0);
    let before = hs.server.world.get_block(far.0, far.1, far.2);
    send_edits(&hs, &client, slot, 1, &[(far, block::GLASS)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(far.0, far.1, far.2), before);
}

#[test]
fn the_edit_budget_is_per_tick_across_every_packet() {
    let mut hs = start_open_server("budget");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let mut cells = Vec::new();
    for dy in 0..2 {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)] {
            cells.push(cell_beside(&hs, slot, dx, dy, dz));
        }
    }
    // Three packets of four edits each, all in one tick.
    for (k, chunk) in cells.chunks(4).enumerate() {
        let edits: Vec<_> = chunk.iter().map(|&c| (c, block::GLASS)).collect();
        send_edits(&hs, &client, slot, k as u64 + 1, &edits);
    }
    hs.tick();
    let placed = cells
        .iter()
        .filter(|c| hs.server.world.get_block(c.0, c.1, c.2) == block::GLASS)
        .count();
    assert_eq!(placed, 4, "at most 4 edits a tick, however many packets carry them");
}

#[test]
fn a_joiner_breaking_a_chest_spills_its_contents() {
    let mut hs = start_open_server("chest");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::CHEST);
    let mut chest = crate::chest::ChestData::new();
    chest.slots[0] = Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Bone, 7));
    hs.server.world.insert_chest(cell, chest);

    send_edits(&hs, &client, slot, 1, &[(cell, block::AIR)]);
    hs.tick();

    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR);
    assert!(hs.server.world.chest_at(cell).is_none(), "no orphan ChestData left behind");
    let bones: u32 = hs
        .server
        .ecs
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .filter(|(_, it)| it.stack.item == crate::item::Item::Material(crate::item::MaterialId::Bone))
        .map(|(_, it)| it.stack.count as u32)
        .sum();
    assert_eq!(bones, 7, "the chest's contents are on the floor");
}

#[test]
fn a_dropped_connection_frees_its_slot_and_the_next_join_reuses_it() {
    let mut hs = start_open_server("reuse");
    let (a, slot_a) = join_guest(&mut hs, "Alpha");
    let (_occupied, total_before) = hs.occupancy();
    let len_before = hs.server.players.len();
    drop(a); // laptop lid closed: no Disconnect packet, the link just goes
    // FU3 (FU1 verify N6) — the tick that finds the link closed reads what
    // it sent last; the slot is freed on the next one.
    hs.tick();
    assert!(!hs.slot_is_free(slot_a), "the tick that sees the close still reads its last packets");
    hs.tick();
    assert!(hs.slot_is_free(slot_a), "the dead connection's slot is freed");
    assert!(!hs.server.players[slot_a].connected, "no ghost player in the sim");

    for n in 0..5 {
        let (c, slot) = join_guest(&mut hs, &format!("Churn{n}"));
        assert_eq!(slot, slot_a, "a freed slot is reused");
        drop(c);
        hs.tick();
        hs.tick();
    }
    assert_eq!(hs.server.players.len(), len_before, "connections coming and going don't grow the server");
    let _ = total_before;
}

#[test]
fn a_rejected_join_frees_its_slot_immediately() {
    let mut hs = start_open_server("reject");
    let client = hs.attach_test_remote();
    let slot = hs.server.players.len() - 1;
    let mut req = crate::remote_client::build_join_request_guest("Old", 0);
    req.protocol_version = protocol::PROTOCOL_VERSION.wrapping_sub(1);
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
    hs.tick();
    assert!(got_reject(&client).is_some(), "the client is told why");
    assert!(hs.slot_is_free(slot), "and the seat is given back at once");
}

#[test]
fn a_connection_that_never_joins_is_freed_after_the_pre_auth_timeout() {
    let mut hs = start_open_server("preauth");
    let client = hs.attach_test_remote();
    let slot = hs.server.players.len() - 1;
    hs.tick();
    assert!(!hs.slot_is_free(slot), "a fresh connection gets time to join");
    hs.advance_clock_for_test(30 * 20);
    hs.tick();
    assert!(hs.slot_is_free(slot), "a silent connection doesn't hold a seat for ever");
    assert_eq!(got_reject(&client).as_deref(), Some("Join timed out"));
}

#[test]
fn a_kick_after_a_rejoin_hits_the_live_player() {
    let mut hs = start_open_server("kick");
    let pk: [u8; 32] = {
        let secp = secp256k1::Secp256k1::new();
        let kp = secp256k1::Keypair::from_seckey_slice(&secp, &[0x31; 32]).unwrap();
        kp.x_only_public_key().0.serialize()
    };
    // Alice joins, leaves; Bob takes her old slot; Alice rejoins elsewhere.
    let (alice1, slot_a1) = join_guest(&mut hs, "Alice");
    hs.server.players[slot_a1].verified_pubkey = Some(pk);
    drop(alice1);
    hs.tick();
    hs.tick(); // FU3 (N6): freed the tick after the one that saw the close
    let (bob, slot_b) = join_guest(&mut hs, "Bob");
    assert_eq!(slot_b, slot_a1);
    let (alice2, slot_a2) = join_guest(&mut hs, "Alice");
    hs.server.players[slot_a2].verified_pubkey = Some(pk);

    let dir = std::env::temp_dir().join(format!("axenstax-kick-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("kick"), hex::encode(pk)).unwrap();
    hs.process_pending_kicks(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(hs.slot_is_free(slot_a2), "the live Alice is kicked");
    assert!(!hs.slot_is_free(slot_b), "Bob, in Alice's old slot, is untouched");
    assert!(got_reject(&alice2).is_some(), "the kicked player is told");
    let _ = bob;
}

// ── Review round (REVIEW-W1): live host state, exactly-once spill ─────────

pub(super) fn bones_on_server(hs: &HostedServer) -> u32 {
    bones_in_ecs(&hs.server.ecs)
}

/// Bone items lying in `ecs` (a lent world's is the host's: `lent_world.rs`).
pub(super) fn bones_in_ecs(ecs: &hecs::World) -> u32 {
    ecs
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .filter(|(_, it)| it.stack.item == crate::item::Item::Material(crate::item::MaterialId::Bone))
        .map(|(_, it)| it.stack.count as u32)
        .sum()
}

pub(super) fn chest_of_bones(n: u32) -> crate::chest::ChestData {
    let mut chest = crate::chest::ChestData::new();
    chest.slots[0] =
        Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Bone, n as _));
    chest
}

/// Send one ClientInput on the host's own local seat (slot 0).
pub(super) fn send_host_edits(hs: &HostedServer, tick: u64, edits: &[((i32, i32, i32), block::BlockId)]) {
    let at = hs.server.players[0].player.pos;
    let mut input =
        protocol::InputPacket { tick, x: at.x, y: at.y, z: at.z, health: 20.0, ..Default::default() };
    for &((x, y, z), b) in edits {
        input.block_changes.push(protocol::BlockChange { x, y, z, new_block: b, meta: 0 });
    }
    hs.local_transports[0]
        .send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
}

/// B1 scenario A, on an OWNING server (`--no-lend`): the host's own break
/// already spilled on the host's client; the server must not spill its copy
/// too. (A lending server never applies the host's edits at all —
/// `lent_world.rs`.)
#[test]
fn the_hosts_own_chest_break_never_spills_on_the_server() {
    let mut hs = start_open_server("host-break");
    hs.tick();
    let cell = cell_beside(&hs, 0, 1, 0, 0);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::CHEST);
    hs.server.world.insert_chest(cell, chest_of_bones(7));

    send_host_edits(&hs, 1, &[(cell, block::AIR)]);
    hs.tick();

    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR);
    assert!(hs.server.world.chest_at(cell).is_none(), "server copy discarded");
    assert_eq!(bones_on_server(&hs), 0, "no second spill of the host's own break");
}

// The host-filled-chest, host-claimed-plot and host-placed-vendor cases (B1 /
// S1) live in `lent_world.rs`: a LAN host now lends the server its own world
// (D1), so there is no second copy to keep in step. An owning server
// (`--no-lend`, the dedicated server) holds the state it loaded.

/// B1 scenarios B/C, on an OWNING host (`--no-lend`, D1 review fix 2): the
/// host fills a chest AFTER load (client-only state); a joiner breaks it. The
/// server spills the LIVE contents (kept live by `mirror_host_world_state`)
/// exactly once, and the host's world is left with no orphan entity.
#[test]
fn no_lend_a_joiner_break_spills_the_live_contents_once_and_leaves_no_host_orphan() {
    let mut hs = start_open_server("live-chest");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    // The server knows the block (the host's place was broadcast) but, like a
    // chest placed this session, holds no contents for it.
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::CHEST);
    let mut host = crate::world::World::new();
    host.set_block(cell.0, cell.1, cell.2, block::CHEST);
    host.insert_chest(cell, chest_of_bones(7));

    hs.mirror_host_world_state(&host);
    send_edits(&hs, &client, slot, 1, &[(cell, block::AIR)]);
    hs.tick();
    assert_eq!(bones_on_server(&hs), 7, "the live contents spill");
    assert!(hs.server.world.chest_at(cell).is_none());

    // The host hasn't consumed the broadcast yet: mirroring must not
    // resurrect the chest on the server, and nothing spills twice.
    hs.mirror_host_world_state(&host);
    hs.tick();
    assert!(hs.server.world.chest_at(cell).is_none(), "not resurrected");
    assert_eq!(bones_on_server(&hs), 7);

    // The host applies the broadcast: entity cleared, no local spill.
    for bc in block_changes_seen(&hs.local_transports[0]) {
        host.apply_remote_block_change(&bc);
    }
    assert!(host.chest_at(cell).is_none(), "no orphan on the host");
    hs.mirror_host_world_state(&host);
    hs.tick();
    assert_eq!(bones_on_server(&hs), 7, "exactly one spill");
}

/// S1, on an owning host (`--no-lend`): a plot claimed on the host after load
/// protects against a joiner.
#[test]
fn no_lend_a_plot_claimed_after_load_protects_against_a_joiner() {
    let mut hs = start_open_server("live-plot");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::AIR);
    let mut host = crate::world::World::new();
    host.plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        cell.0,
        cell.1 - 5,
        cell.2,
    ));
    hs.mirror_host_world_state(&host);

    send_edits(&hs, &client, slot, 1, &[(cell, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR);
}

/// S1, on an owning host (`--no-lend`): a vendor the host placed this session
/// is protected too.
#[test]
fn no_lend_a_vendor_placed_after_load_is_protected_from_a_joiner() {
    let mut hs = start_open_server("live-vendor");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::VENDOR_BLOCK);
    let mut host = crate::world::World::new();
    host.set_block(cell.0, cell.1, cell.2, block::VENDOR_BLOCK);
    host.insert_vendor(
        cell,
        crate::vendor::VendorData {
            owner: Some(crate::vendor::VendorOwner::LocalPlayer(0)),
            ..Default::default()
        },
    );
    hs.mirror_host_world_state(&host);

    send_edits(&hs, &client, slot, 1, &[(cell, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::VENDOR_BLOCK);
}

/// S4: reach is measured from the eye. A block straight overhead whose bottom
/// face is at the client's 5-block ray limit is in reach.
#[test]
fn an_overhead_break_at_max_reach_is_accepted() {
    let mut hs = start_open_server("overhead");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let feet = hs.server.players[slot].player.pos;
    let eye_y = feet.y + 1.62;
    let cell = (feet.x.floor() as i32, (eye_y + 5.0).floor() as i32, feet.z.floor() as i32);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::STONE);
    send_edits(&hs, &client, slot, 1, &[(cell, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR);
}

/// S5 / FU3 (FU1 verify N3): edits past the per-tick budget (4) used to be
/// sent back like any refusal, so an honest client's fifth edit of a tick was
/// lost. Now they wait and apply on the next tick, in order; nothing is sent
/// back for the budget.
#[test]
fn over_budget_edits_wait_and_apply_on_the_next_tick() {
    let mut hs = start_open_server("budget-back");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cells: Vec<_> = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)]
        .iter()
        .map(|&(dx, dz)| cell_beside(&hs, slot, dx, 1, dz))
        .collect();
    let _ = block_changes_seen(&client);
    let edits: Vec<_> = cells.iter().map(|&c| (c, block::GLASS)).collect();
    send_edits(&hs, &client, slot, 1, &edits);
    hs.tick();
    let first = block_changes_seen(&client);
    for c in &cells[4..] {
        assert!(
            !first.iter().any(|bc| (bc.x, bc.y, bc.z) == *c && bc.new_block != block::GLASS),
            "the over-budget edit at {c:?} is not sent back"
        );
        assert_ne!(hs.server.world.get_block(c.0, c.1, c.2), block::GLASS, "nor applied yet");
    }
    hs.tick();
    let second = block_changes_seen(&client);
    for c in &cells {
        assert_eq!(hs.server.world.get_block(c.0, c.1, c.2), block::GLASS, "{c:?} applied");
    }
    for c in &cells[4..] {
        assert!(
            second.iter().any(|bc| (bc.x, bc.y, bc.z) == *c && bc.new_block == block::GLASS),
            "the waiting edit at {c:?} reaches the joiner the next tick"
        );
    }
}

/// FU4a (FU3 verify L1) — a request waits behind its own client's earlier
/// edits still waiting past the edit budget. A lever placed as the fifth edit
/// of a tick waits a tick; the flip sent right after it used to be read
/// first, find no lever and do nothing. Now it waits for the lever.
#[test]
fn a_flip_sent_after_a_lever_waiting_past_the_edit_budget_finds_the_lever() {
    let mut hs = start_open_server("lever-behind-edits");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let mut edits: Vec<_> = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .map(|&(dx, dz)| (cell_beside(&hs, slot, dx, 1, dz), block::GLASS))
        .collect();
    let lever = cell_beside(&hs, slot, 1, 1, 1);
    edits.push((lever, block::LEVER)); // the 5th: waits a tick
    send_edits(&hs, &client, slot, 1, &edits);
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::DeviceInteract,
        &protocol::DeviceInteractPacket { pos: lever },
    ));
    hs.tick();
    assert!(hs.server.world.power_device_at(lever).is_none(), "the lever waits past the budget");
    hs.tick();
    let device = hs.server.world.power_device_at(lever).expect("the lever is placed");
    assert!(device.on, "and the flip sent after it found it");
}

/// S3: a joiner can't flip a lever inside someone else's plot.
#[test]
fn a_joiner_cannot_flip_a_lever_in_a_foreign_plot() {
    use crate::power::{PowerDeviceData, PowerDeviceKind};
    let mut hs = start_open_server("device-plot");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::LEVER);
    hs.server.world.insert_power_device(
        cell,
        PowerDeviceData::new(PowerDeviceKind::Lever, crate::meta::Facing::East),
    );
    hs.server.world.plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        cell.0,
        cell.1 - 5,
        cell.2,
    ));
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::DeviceInteract,
        &protocol::DeviceInteractPacket { pos: cell },
    ));
    hs.tick();
    assert!(!hs.server.world.power_device_at(cell).expect("lever").on, "lever untouched");

    // Outside any plot the same joiner can.
    hs.server.world.plots.clear();
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::DeviceInteract,
        &protocol::DeviceInteractPacket { pos: cell },
    ));
    hs.tick();
    assert!(hs.server.world.power_device_at(cell).expect("lever").on, "lever flipped");
}

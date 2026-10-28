//! A real unsigned guest's accepted edit and later, still-raw ordinary input.
use super::*;
use crate::{block, item::ItemStack};
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

struct Acknowledgements<T> {
    inner: T,
    actual: Arc<AtomicU64>,
}

impl<T: ClientTransport> ClientTransport for Acknowledgements<T> {
    fn send_to_server(&self, bytes: &[u8]) {
        self.inner.send_to_server(bytes);
    }
    fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
        let packet = self.inner.try_recv_from_server()?;
        assert!(packet.len() <= protocol::MAX_WIRE_PACKET_LEN);
        if let Some((PacketType::StateUpdate, body)) = protocol::deserialize_header(&packet) {
            let state: protocol::StateUpdatePacket = protocol::safe_deserialize(body).unwrap();
            self.actual
                .fetch_max(state.last_acked_input, Ordering::Relaxed);
        }
        // Only the fixed acknowledgement is retained; the original packet is
        // forwarded unchanged to the actual client's ordinary application.
        Some(packet)
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.inner.channel_binding()
    }
    fn ws_host(&self) -> Option<String> {
        self.inner.ws_host()
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    fn close_text(&self) -> Option<String> {
        self.inner.close_text()
    }
    fn host_closed_link(&self) -> bool {
        self.inner.host_closed_link()
    }
    fn proven_host(&self) -> Option<crate::transport::ProvenHost> {
        self.inner.proven_host()
    }
}

fn round(j: &mut Joined, guest: &mut HeadlessGame) {
    guest.ticks(1);
    guest.state.network_send_input();
    j.host.lend_tick(&mut j.hs);
    j.hg.frames(1);
    guest.frames(1);
    // Real frames can emit additional inputs. Let the authority process that
    // finite backlog without feeding another manual input on every tick.
    for _ in 0..32 {
        let remote = guest.state.remote_client.as_ref().unwrap();
        let Some(slot) = remote.player_index().map(|i| i as usize) else {
            return;
        };
        let sp = &j.hs.server.players[slot];
        if sp.last_input_tick == sp.last_applied_input
            && sp.last_applied_input.checked_add(1) == Some(remote.next_input_seq())
        {
            return;
        }
        j.host.lend_tick(&mut j.hs);
    }
    panic!("the real source did not catch up with this finite guest input batch");
}

fn settled(j: &Joined, guest: &HeadlessGame, slot: usize) -> bool {
    let sp = &j.hs.server.players[slot];
    let sent = guest.state.remote_client.as_ref().unwrap().next_input_seq();
    sp.last_input_tick == sp.last_applied_input
        && sp.last_applied_input.checked_add(1) == Some(sent)
        && [&j.hg, guest].iter().all(|game| {
            game.state.remote_client.as_ref().is_some_and(|remote| {
                remote.is_connected()
                    && remote.personal_ready()
                    && remote.handover_authority_settled()
            }) && game.state.window_inbox.is_empty()
        })
        && [j.slot, slot].iter().all(|&index| {
            let sp = &j.hs.server.players[index];
            sp.window_events.is_quiescent()
                && sp.pack_flight.is_none()
                && sp.action_queue.as_ref().is_none_or(|q| q.is_empty())
                && sp.action_results.as_ref().is_none_or(|r| r.is_quiescent())
        })
}

fn stones(game: &HeadlessGame) -> Option<u8> {
    game.state.players[0]
        .inventory
        .slot(0)
        .map(|stack| stack.count)
}

fn send_placement(guest: &mut HeadlessGame, cell: (i32, i32, i32)) -> u64 {
    assert_eq!(
        guest.state.world.get_block(cell.0, cell.1, cell.2),
        block::AIR
    );
    let before = stones(guest).unwrap();
    // The normal place arm predicts its one backed cost before queueing input.
    guest.state.players[0]
        .inventory
        .take_placeable_from_hotbar(0);
    assert_eq!(stones(guest), Some(before - 1));
    guest
        .state
        .world
        .set_block(cell.0, cell.1, cell.2, block::STONE);
    guest
        .state
        .pending_block_changes
        .push(protocol::BlockChange {
            x: cell.0,
            y: cell.1,
            z: cell.2,
            new_block: block::STONE,
            meta: 0,
        });
    let actual_sequence = guest.state.remote_client.as_ref().unwrap().next_input_seq();
    guest.state.network_send_input();
    assert_eq!(
        guest.state.remote_client.as_ref().unwrap().next_input_seq(),
        actual_sequence + 1
    );
    actual_sequence
}

#[derive(Debug, PartialEq, Eq)]
struct Front {
    allocation: usize,
    length: usize,
    capacity: usize,
    hash: [u8; 32],
}

fn front(j: &Joined, slot: usize, sequence: u64, cell: (i32, i32, i32)) -> Front {
    let packet =
        j.hs.handover_test_inbound_front(slot)
            .expect("actual guest inbound front");
    let (kind, body) = protocol::deserialize_header(packet).unwrap();
    assert_eq!(kind, PacketType::ClientInput);
    assert!(packet.len() <= protocol::MAX_WIRE_PACKET_LEN);
    let input: protocol::InputPacket = protocol::safe_deserialize(body).unwrap();
    assert_eq!(input.tick, sequence);
    assert_eq!(input.block_changes.len(), 1);
    let change = &input.block_changes[0];
    assert_eq!(
        (change.x, change.y, change.z, change.new_block),
        (cell.0, cell.1, cell.2, block::STONE)
    );
    Front {
        allocation: packet.as_ptr() as usize,
        length: packet.len(),
        capacity: packet.capacity(),
        hash: Sha256::digest(packet).into(),
    }
}

#[test]
#[ignore = "needs two real GameState GPU adapters and genuine joined input"]
fn game_harness_handover_guest_raw_input_waits_without_losing_accepted_edit() {
    let mut j = joined("raw-guest-cut");
    let mut guest = HeadlessGame::boot_into_world("raw-guest-client");
    let transport = j.hs.attach_test_remote();
    let acknowledgements = Arc::new(AtomicU64::new(0));
    guest.state.remote_client = Some(RemoteClient::from_transport(
        Box::new(Acknowledgements {
            inner: transport,
            actual: acknowledgements.clone(),
        }),
        crate::remote_client::build_join_request_guest("Unsigned visitor", 0),
        None,
    ));
    let deadline = Instant::now() + Duration::from_secs(180);
    let slot = loop {
        round(&mut j, &mut guest);
        if let Some(slot) = guest
            .state
            .remote_client
            .as_ref()
            .unwrap()
            .player_index()
            .map(|i| i as usize)
            && settled(&j, &guest, slot)
        {
            break slot;
        }
        assert!(Instant::now() < deadline, "the real guest never settled");
    };
    assert_ne!(slot, j.slot);
    assert!(
        j.hs.server.players[slot].verified_pubkey.is_none(),
        "this must be an actual unsigned guest"
    );
    let generation = j.hs.server.players[slot].attach_gen;
    // Ordinary blocks are declared initial fixture stock, matching both actual
    // holders. All later costs must be produced by the normal placement path.
    guest.state.players[0]
        .inventory
        .set_slot(0, Some(ItemStack::new_block(block::STONE, 3)));
    guest.state.players[0].hotbar_slot = 0;
    j.hs.server.players[slot]
        .inventory
        .set_slot(0, Some(ItemStack::new_block(block::STONE, 3)));
    let first = super::super::joiner_authority::cell_beside(&j.hs, slot, 0, 2, 0);
    let later = (first.0 + 1, first.1, first.2);
    assert_eq!(
        j.host.world.get_block(first.0, first.1, first.2),
        block::AIR
    );
    assert_eq!(
        j.host.world.get_block(later.0, later.1, later.2),
        block::AIR
    );
    let accepted_sequence = send_placement(&mut guest, first);
    loop {
        round(&mut j, &mut guest);
        if j.host.world.get_block(first.0, first.1, first.2) == block::STONE
            && j.hs.server.players[slot].last_applied_input >= accepted_sequence
            && acknowledgements.load(Ordering::Relaxed) >= accepted_sequence
            && settled(&j, &guest, slot)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the earlier real guest edit was not accepted and settled"
        );
    }
    assert_eq!(
        j.hs.server.players[slot].inventory.slot(0).map(|s| s.count),
        Some(2)
    );
    assert_eq!(stones(&guest), Some(2));
    let prior_applied = j.hs.server.players[slot].last_applied_input;
    let raw_sequence = send_placement(&mut guest, later);
    assert!(raw_sequence > prior_applied);
    // No source tick after this send: it remains in the real transport when
    // freeze checks the existing inbound/typed queues. No ACK is invented.
    assert_eq!(j.hs.server.players[slot].last_applied_input, prior_applied);
    let (selected_generation, key) = j.hs.handover_seat(j.slot).unwrap();
    let _lease =
        j.hs.handover_bind(j.slot, selected_generation, key, [0x6d; 16])
            .unwrap();
    j.hs.handover_freeze(None).unwrap();
    let ticks = (j.hs.server.tick_counter, j.host.world.total_ticks);
    assert!(j.hs.handover_take_frames().unwrap().is_empty());
    let held = front(&j, slot, raw_sequence, later);
    assert!(j.hs.handover_test_inbound_bytes(slot) >= held.capacity);
    for _ in 0..3 {
        j.host.lend_tick(&mut j.hs);
        assert!(j.hs.handover_take_frames().unwrap().is_empty());
        assert_eq!(front(&j, slot, raw_sequence, later), held);
        assert_eq!((j.hs.server.tick_counter, j.host.world.total_ticks), ticks);
        assert_eq!(j.hs.server.players[slot].last_applied_input, prior_applied);
        assert_eq!(
            j.hs.server.players[slot].inventory.slot(0).map(|s| s.count),
            Some(2)
        );
        assert_eq!(
            j.host.world.get_block(first.0, first.1, first.2),
            block::STONE
        );
        assert_eq!(
            j.host.world.get_block(later.0, later.1, later.2),
            block::AIR
        );
    }
    eprintln!(
        "actual raw guest: slot={slot} generation={generation} accepted={accepted_sequence} prior_applied={prior_applied} queued={raw_sequence} held={held:?}"
    );
    j.hs.handover_release();
    j.host.lend_tick(&mut j.hs);
    assert!(!j.hs.handover_frozen());
    assert_eq!(j.hs.server.players[slot].attach_gen, generation);
    assert!(j.hs.server.players[slot].verified_pubkey.is_none());
    assert_eq!(j.hs.server.players[slot].last_applied_input, raw_sequence);
    assert!(j.hs.handover_test_inbound_front(slot).is_none());
    assert_eq!(
        j.host.world.get_block(first.0, first.1, first.2),
        block::STONE
    );
    assert_eq!(
        j.host.world.get_block(later.0, later.1, later.2),
        block::STONE
    );
    assert_eq!(
        j.hs.server.players[slot].inventory.slot(0).map(|s| s.count),
        Some(1)
    );
    for _ in 0..3 {
        j.host.lend_tick(&mut j.hs);
        assert_eq!(
            j.hs.server.players[slot].inventory.slot(0).map(|s| s.count),
            Some(1)
        );
    }
    guest.frames(1);
    assert!(acknowledgements.load(Ordering::Relaxed) >= raw_sequence);
    assert_eq!(stones(&guest), Some(1));
}

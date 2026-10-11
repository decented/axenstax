//! World handover, live source integration: the control link is bound to an
//! exact authenticated seat and the source freeze stops every tick-driven
//! path while control frames still flow; a pre-retirement release resumes the
//! same seat's waiting gameplay through the ordinary path.
use super::joiner_authority::{accepted, cell_beside, send_edits};
use super::lent_world::start_lent;
use crate::block;
use crate::hosted_server::HostedServer;
use crate::protocol;
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};

pub(super) const BINDING: [u8; 32] = [17; 32];
const T: crate::handover::Id = [0x2a; 16];

/// The handover frames waiting for a joined client, decoded.
fn frames_for(client: &ChannelClientTransport) -> Vec<crate::handover::live::wire::Frame> {
    let mut out = Vec::new();
    while let Some(p) = client.try_recv_from_server() {
        if let Some(b) = crate::handover::live::recipient::handover_payload(&p) {
            out.push(crate::handover::live::wire::Frame::decode(b).expect("well-formed frame"));
        }
    }
    out
}

fn now_secs() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

/// A signed join over a bound channel through one lent tick; returns the
/// client, its seat and the seat's (generation, verified key).
pub(super) fn signed_join_lent(
    hs: &mut HostedServer,
    host: &mut OwnedSimParts,
    seckey: [u8; 32],
) -> (ChannelClientTransport, usize, u32, [u8; 32]) {
    let client = hs.attach_test_remote_with_binding(Some(BINDING));
    let chal = loop {
        let pkt = client.try_recv_from_server().expect("challenge issued on attach");
        let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
        if ptype == protocol::PacketType::Challenge {
            break protocol::safe_deserialize::<protocol::ChallengePacket>(payload).unwrap();
        }
    };
    let origin = crate::signet::join_origin(Some(BINDING));
    let ev = crate::hosted_server::tests::signed_auth_event(seckey, &chal.nonce_hex, &origin, now_secs());
    let req = crate::hosted_server::tests::join_with_auth(ev, None);
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
    host.lend_tick(hs);
    let slot = accepted(&client).expect("signed join accepted").player_index as usize;
    super::joiner_authority::settle_chunk_push(hs, &client, |hs| host.lend_tick(hs));
    let (generation, key) = hs.handover_seat(slot).expect("authenticated live seat");
    (client, slot, generation, key)
}

fn handover_frame(bytes: &[u8]) -> Vec<u8> {
    let mut p = vec![protocol::PacketType::Handover as u8];
    p.extend_from_slice(bytes);
    p
}

#[test]
fn handover_link_binds_only_the_exact_authenticated_seat() {
    let (mut hs, mut host) = start_lent("handover-bind");
    let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x51; 32]);
    assert_eq!(
        hs.handover_bind(slot, generation.wrapping_add(1), key, T).err(),
        Some("that player is no longer signed in to this world")
    );
    assert!(hs.handover_bind(slot, generation, [9; 32], T).is_err(), "another key");
    assert!(hs.handover_bind(0, generation, key, T).is_err(), "the host's own seat");
    let _lease = hs.handover_bind(slot, generation, key, T).unwrap();
    assert!(hs.handover_bind(slot, generation, key, T).is_err(), "one link at a time");
    // A frame from the bound seat reaches the link through ordinary processing.
    client.send_to_server(&handover_frame(b"hello"));
    host.lend_tick(&mut hs);
    assert_eq!(hs.handover_take_frames().unwrap(), vec![b"hello".to_vec()]);
    // The seat leaves: the link breaks and cannot freeze the source.
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Disconnect, &()));
    host.lend_tick(&mut hs);
    assert!(hs.handover_check().is_err());
    assert!(hs.handover_freeze(None).is_err());
    assert!(!hs.handover_frozen());
}

#[test]
fn handover_frozen_source_runs_no_tick_admits_nobody_and_resumes_after_release() {
    let (mut hs, mut host) = start_lent("handover-freeze");
    let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x52; 32]);
    let (other, other_slot, _, _) = signed_join_lent(&mut hs, &mut host, [0x53; 32]);
    let accept = hs.test_accept_channel();
    let _lease = hs.handover_bind(slot, generation, key, T).unwrap();
    let _ = frames_for(&other);
    hs.handover_freeze(None).unwrap();
    // Every joined player is told play is paused, so its client stops sending.
    use crate::handover::live::wire::Frame;
    assert_eq!(frames_for(&other), vec![Frame::Notice { transfer: T, paused: true }]);
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    let other_cell = cell_beside(&hs, other_slot, 0, 0, 1);
    let before = host.world.get_block(cell.0, cell.1, cell.2);
    let other_before = host.world.get_block(other_cell.0, other_cell.1, other_cell.2);
    assert_ne!(before, block::GLASS);
    // Both seats act; a newcomer's connection arrives; control frames flow.
    send_edits(&hs, &client, slot, 1, &[(cell, block::GLASS)]);
    send_edits(&hs, &other, other_slot, 1, &[(other_cell, block::GLASS)]);
    client.send_to_server(&handover_frame(b"accept"));
    let (server_side, newcomer) = crate::transport::channel_pair();
    accept.send(Box::new(server_side)).unwrap();
    let tally = host.world.sim_tally;
    for _ in 0..5 {
        host.lend_tick(&mut hs); // what an ungated caller would do: still nothing
    }
    assert_eq!(hs.handover_take_frames().unwrap(), vec![b"accept".to_vec()]);
    assert!(host.world.sim_tally == tally, "a frozen source simulated");
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), before, "frozen edit applied");
    assert_eq!(host.world.get_block(other_cell.0, other_cell.1, other_cell.2), other_before);
    assert!(newcomer.try_recv_from_server().is_none(), "admission ran while frozen");
    assert!(frames_for(&other).is_empty(), "nothing else reaches a joiner while frozen");
    // Pre-retirement release: same generations, so the waiting edits are
    // judged by the ordinary path, once, and the newcomer is met.
    hs.handover_release();
    assert!(!hs.handover_frozen());
    assert!(
        frames_for(&other).contains(&Frame::Notice { transfer: T, paused: false }),
        "joiners are told play resumes"
    );
    host.lend_tick(&mut hs);
    assert!(host.world.sim_tally != tally, "a released source must simulate again");
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), block::GLASS);
    assert_eq!(host.world.get_block(other_cell.0, other_cell.1, other_cell.2), block::GLASS);
    let mut challenged = false;
    while let Some(p) = newcomer.try_recv_from_server() {
        challenged |= matches!(protocol::deserialize_header(&p), Some((protocol::PacketType::Challenge, _)));
    }
    assert!(challenged, "the newcomer was not admitted after release");
}

/// CP2 review L1: while frozen a guest's flood moves only into its own
/// bounded queue (unprocessed); past the bound it is read no further and is
/// disconnected at release by the ordinary overflow kick.
#[test]
fn handover_frozen_guest_flood_is_bounded_and_kicked_at_release() {
    let (mut hs, mut host) = start_lent("handover-flood");
    let (_client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x54; 32]);
    let (guest, guest_slot, _, _) = signed_join_lent(&mut hs, &mut host, [0x55; 32]);
    let _lease = hs.handover_bind(slot, generation, key, T).unwrap();
    hs.handover_freeze(None).unwrap();
    let junk = vec![0u8; 60 * 1024];
    let n = crate::transport::MAX_INBOUND_BYTES / junk.len() + 8;
    for _ in 0..n {
        guest.send_to_server(&junk);
    }
    assert!(!hs.handover_test_transport_closed(guest_slot));
    hs.handover_take_frames().unwrap();
    assert!(hs.handover_test_inbound_bytes(guest_slot) > 0, "the guest's packets were left in its unbounded transport");
    assert!(
        hs.handover_test_transport_closed(guest_slot),
        "an overflowed guest's connection stayed open (and buffering) while frozen"
    );
    let bound = crate::transport::MAX_INBOUND_BYTES + junk.len() + crate::transport::INBOUND_ENTRY_OVERHEAD;
    for _ in 0..n {
        guest.send_to_server(&junk);
    }
    for _ in 0..4 {
        hs.handover_take_frames().unwrap();
    }
    assert!(hs.handover_test_inbound_bytes(guest_slot) <= bound, "the guest's queue grew past its bound while frozen");
    // LOW-1: the connection is closed at once, but the slot's bookkeeping
    // (leave, save) waits for release.
    assert!(hs.server.players[guest_slot].verified_pubkey.is_some(), "the slot was released while frozen");
    assert!(hs.handover_seat(guest_slot).is_none(), "an overflowed guest is still offered as a seat");
    hs.handover_release();
    assert!(hs.handover_seat(guest_slot).is_none(), "an overflowed guest stayed at release");
    // Review LOW-2: this release ran outside the lend (a cancel from the
    // frame's pump), so the kick waits for the next lent tick, where the
    // guest's leave settles on the authoritative world.
    assert!(!hs.handover_test_slot_released(guest_slot), "the overflow kick ran outside the lend (no leave settlement)");
    assert_eq!(hs.handover_test_deferred_overflow(), 1);
    host.lend_tick(&mut hs);
    assert!(hs.handover_test_slot_released(guest_slot), "the deferred overflow kick never ran");
    assert_eq!(hs.handover_test_deferred_overflow(), 0);
}

#[test]
fn handover_eligibility_refuses_live_seat_state_it_cannot_carry() {
    use crate::handover::live::eligibility::{Refusal, Seat, seats};
    let (mut hs, mut host) = start_lent("handover-elig");
    let (_c, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x61; 32]);
    let seat = Seat { slot, generation, key };
    assert_eq!(seats(&hs.server, seat), Ok(()), "a fresh tracked seat with no credits");
    assert!(seats(&hs.server, Seat { generation: generation.wrapping_add(1), ..seat }).is_err());
    assert!(seats(&hs.server, Seat { key: [3; 32], ..seat }).is_err());
    // One server credit: in flight first, then delivered — both refuse.
    hs.server.queue_kill_event(slot, generation, crate::mob::MobType::Cow, glam::Vec3::ZERO, false, false);
    assert!(seats(&hs.server, seat).is_err(), "credit still in flight");
    host.lend_tick(&mut hs);
    assert_eq!(
        seats(&hs.server, seat),
        Err(Refusal("That player has village or hunting progress this session, which this version can't move yet."))
    );
    // An unreadable record of a connected seat refuses (kept untouched).
    let (mut hs, mut host) = start_lent("handover-elig-read");
    let (_c, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x62; 32]);
    hs.server.failed_joiner_reads.insert(key);
    assert!(seats(&hs.server, Seat { slot, generation, key }).is_err());
    // Another signed-in player present: their agreement can't be asked yet.
    let (mut hs, mut host) = start_lent("handover-elig-other");
    let (_a, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x63; 32]);
    let (_b, _, _, _) = signed_join_lent(&mut hs, &mut host, [0x64; 32]);
    assert!(seats(&hs.server, Seat { slot, generation, key }).is_err());
}

#[test]
fn handover_dropped_session_lease_releases_the_source_and_tells_the_recipient() {
    let (mut hs, mut host) = start_lent("handover-lease");
    let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x54; 32]);
    let lease = hs.handover_bind(slot, generation, key, T).unwrap();
    hs.handover_freeze(None).unwrap();
    let tally = host.world.sim_tally;
    host.lend_tick(&mut hs);
    assert!(hs.handover_frozen() && host.world.sim_tally == tally, "held lease: still frozen");
    let _ = frames_for(&client);
    drop(lease); // the session went away without an explicit abort
    host.lend_tick(&mut hs);
    assert!(!hs.handover_frozen() && hs.handover_link().is_none(), "a dropped session's link is released");
    let got = frames_for(&client);
    use crate::handover::live::wire::Frame;
    assert!(got.contains(&Frame::Abort { transfer: T, aborted: None }), "the recipient is told: {got:?}");
    assert!(got.contains(&Frame::Notice { transfer: T, paused: false }));
}

#[test]
fn handover_release_discards_a_departed_recipients_waiting_edits() {
    let (mut hs, mut host) = start_lent("handover-departed");
    let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x55; 32]);
    let _lease = hs.handover_bind(slot, generation, key, T).unwrap();
    hs.handover_freeze(None).unwrap();
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    let before = host.world.get_block(cell.0, cell.1, cell.2);
    send_edits(&hs, &client, slot, 1, &[(cell, block::GLASS)]);
    // Pull the waiting edit into the seat's queue the way the frozen pump
    // does, then the recipient's connection goes.
    assert!(hs.handover_take_frames().unwrap().is_empty());
    drop(client);
    // The source's pumps notice the closed connection (frames sent before
    // the close are read first), then it releases.
    assert!((0..3).any(|_| hs.handover_take_frames().is_err()), "the link breaks when the recipient goes");
    hs.handover_release();
    for _ in 0..3 {
        host.lend_tick(&mut hs);
    }
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), before, "a departed seat's stale edit was replayed");
}

#[test]
fn handover_refuses_while_the_recipients_inventory_is_still_settling() {
    use crate::handover::live::eligibility::{Refusal, Seat, seats};
    let (mut hs, mut host) = start_lent("handover-settling");
    let (_c, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x65; 32]);
    let seat = Seat { slot, generation, key };
    assert_eq!(seats(&hs.server, seat), Ok(()));
    // A grant sent to the joiner and not yet acknowledged by its client.
    let stack = crate::item::ItemStack::new_block(block::GLASS, 1);
    crate::window_events::queue(&mut hs.server.players[slot], crate::window_events::WindowEvent::Grant(stack), 1);
    assert_eq!(
        seats(&hs.server, seat),
        Err(Refusal("That player's inventory is still settling with yours. Try again in a moment."))
    );
}

/// A signed join over a REAL QUIC client transport, ticking the lent server
/// until the challenge and the acceptance arrive.
fn signed_join_quic(
    hs: &mut HostedServer,
    host: &mut OwnedSimParts,
    client: &dyn ClientTransport,
    seckey: [u8; 32],
) -> (usize, u32, [u8; 32]) {
    use protocol::PacketType;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut wait_for = |hs: &mut HostedServer, want: PacketType| loop {
        host.lend_tick(hs);
        while let Some(pkt) = client.try_recv_from_server() {
            if let Some((t, payload)) = protocol::deserialize_header(&pkt)
                && t == want
            {
                return payload.to_vec();
            }
        }
        assert!(std::time::Instant::now() < deadline, "no {want:?} over QUIC");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let chal: protocol::ChallengePacket = protocol::safe_deserialize(&wait_for(hs, PacketType::Challenge)).unwrap();
    let origin = crate::signet::join_origin(client.channel_binding());
    let ev = crate::hosted_server::tests::signed_auth_event(seckey, &chal.nonce_hex, &origin, now_secs());
    let req = crate::hosted_server::tests::join_with_auth(ev, None);
    client.send_to_server(&protocol::serialize_packet(PacketType::JoinRequest, &req));
    let acc: protocol::JoinAcceptPacket = protocol::safe_deserialize(&wait_for(hs, PacketType::JoinAccept)).unwrap();
    let slot = acc.player_index as usize;
    let (generation, key) = hs.handover_seat(slot).expect("authenticated live seat");
    (slot, generation, key)
}

/// H1 (CP2 review): over REAL QUIC with a test-shortened idle timeout, a
/// silent freeze longer than that timeout (the frozen server broadcasts
/// nothing, the guest and the recipient send nothing) keeps both connections
/// and the control link; the cancel then resumes both.
#[test]
fn handover_silent_freeze_past_the_quic_idle_timeout_keeps_guest_and_recipient() {
    use crate::network::{Liveness, bridge_server_connection, connect_to_server_with, create_server_endpoint_with};
    use std::time::{Duration, Instant};
    let live = Liveness { idle: Duration::from_millis(1200), keep_alive: Some(Duration::from_millis(300)) };
    let (mut hs, mut host) = start_lent("handover-quic-idle");
    let accept = hs.test_accept_channel();
    let (addr_tx, addr_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let ep = create_server_endpoint_with("127.0.0.1:0".parse().unwrap(), live).unwrap();
            addr_tx.send(ep.local_addr().unwrap()).unwrap();
            for _ in 0..2 {
                let Some(incoming) = ep.accept().await else { return };
                let Ok(conn) = incoming.await else { return };
                let _ = accept.send(Box::new(bridge_server_connection(conn)));
            }
            let _ = stop_rx.await; // keep the endpoint driven until the test ends
        });
    });
    let addr = addr_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let recipient = connect_to_server_with(addr, live).unwrap();
    let (slot, generation, key) = signed_join_quic(&mut hs, &mut host, &recipient, [0x71; 32]);
    let guest = connect_to_server_with(addr, live).unwrap();
    let (guest_slot, ..) = signed_join_quic(&mut hs, &mut host, &guest, [0x72; 32]);
    let _lease = hs.handover_bind(slot, generation, key, T).unwrap();
    hs.handover_freeze(None).unwrap();
    // Silence for twice the idle timeout: frozen ticks, nothing sent.
    let until = Instant::now() + live.idle * 2;
    while Instant::now() < until {
        host.lend_tick(&mut hs);
        while guest.try_recv_from_server().is_some() || recipient.try_recv_from_server().is_some() {}
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!guest.is_closed(), "the guest's connection idled out during the freeze");
    assert!(!recipient.is_closed(), "the recipient's link idled out during the freeze");
    assert!(hs.handover_take_frames().is_ok(), "the source lost its control link");
    // Cancel (pre-retirement): both seats resume and hear the world again.
    hs.handover_release();
    for (who, c, s) in [("guest", &guest, guest_slot), ("recipient", &recipient, slot)] {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut heard = false;
        while !heard && Instant::now() < deadline {
            host.lend_tick(&mut hs);
            while let Some(p) = c.try_recv_from_server() {
                heard |= matches!(protocol::deserialize_header(&p), Some((protocol::PacketType::StateUpdate, _)));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(heard, "the {who} didn't resume after the cancel");
        assert!(hs.handover_seat(s).is_some(), "the {who}'s seat was lost");
    }
    let _ = stop_tx.send(());
    let _ = server.join();
}

/// OC (D4): a connected recipient whose session history isn't `Complete`
/// (here: it began from a v1 record) refuses even with a settled window, and
/// a recipient record with owed units or an incomplete history refuses at
/// the frozen capture; no record at all is proven.
#[test]
fn handover_refuses_a_participant_whose_obligations_are_not_proven_zero() {
    use crate::handover::live::eligibility::{OWED, Seat, recipient_record, seats};
    use crate::joiner_save::{IncompleteCause, JoinerSave};
    let (mut hs, mut host) = start_lent("handover-obligations");
    let (_c, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x66; 32]);
    let seat = Seat { slot, generation, key };
    assert_eq!(seats(&hs.server, seat), Ok(()));
    let proven = JoinerSave::capture(&hs.server.players[slot], hs.server.run_id);
    assert_eq!(recipient_record(Some(&proven)), Ok(()));
    assert_eq!(recipient_record(None), Ok(()), "no record is proven");
    hs.server.players[slot].obligations.mark(IncompleteCause::LegacyV1);
    assert!(hs.server.players[slot].window_events.is_quiescent(), "the window itself is settled");
    assert_eq!(seats(&hs.server, seat), Err(OWED), "an owed/unproven joiner was included");
    let unproven = JoinerSave::capture(&hs.server.players[slot], hs.server.run_id);
    assert_eq!(recipient_record(Some(&unproven)), Err(OWED), "an unproven recipient record became seat 0");
}

mod session {
    use super::*;
    use crate::handover::{
        context::RootContext,
        durable::{MarkerKind, MarkerStore},
        live::{
            capture,
            eligibility::Seat,
            recipient::{JoinedHost, RecipientConsent, RecipientIdentity, RecipientSession, RecipientStage},
            source::{Capture, HostAction, Live, SourceConfig, SourceSession, SourceStage},
            wire::{Frame, root_lineage},
        },
        personal::{PersonalRecord, RecordContext},
        phase::Authority,
    };
    use crate::remote_client::RemoteClient;
    use nostr::{JsonUtil, Keys, SecretKey, Timestamp, ToBech32};
    use std::sync::{Arc, Mutex};

    struct Dir(std::path::PathBuf);
    impl Dir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("handover-live-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn keys(b: u8) -> Keys {
        Keys::new(SecretKey::from_slice(&[b; 32]).unwrap())
    }
    fn now() -> u64 {
        now_secs() as u64
    }
    fn npub(k: &Keys) -> String {
        k.public_key().to_bech32().unwrap()
    }

    /// A joined client's connection, with an optional replacement for the
    /// Ready it sends (a man in the middle, for the source's own check).
    struct Tap {
        inner: Box<dyn ClientTransport>,
        forged_ready: Arc<Mutex<Option<Vec<u8>>>>,
        forged_bearer: Arc<Mutex<Option<[u8; 16]>>>,
        /// Re-signs the recipient's PreparedAck with a stranger's key.
        forge_ack: Arc<std::sync::atomic::AtomicBool>,
        /// Replaces the ack's possession proof (signed by the recipient's
        /// own runtime, so only the proof is wrong).
        ack_forge: Arc<Mutex<Option<(AckForge, RootContext)>>>,
        /// CP13b: the recipient's PreparedAck is lost in transit.
        drop_ack: Arc<std::sync::atomic::AtomicBool>,
    }
    /// CP8 review LOW-2: what a correctly signed ack carries in place of the
    /// real possession proof.
    #[derive(Clone, Copy, Debug)]
    enum AckForge {
        /// The v1 "proof": the root capability, computable from public data.
        V1Capability,
        /// A proof derived from another bearer.
        OtherBearer,
        /// A proof over another Prepared hash.
        OtherPrepared,
    }
    impl ClientTransport for Tap {
        fn send_to_server(&self, data: &[u8]) {
            let frame = crate::handover::live::recipient::handover_payload(data).and_then(|b| Frame::decode(b).ok());
            if matches!(frame, Some(Frame::PreparedAck { .. })) && self.drop_ack.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            if let Some(Frame::PreparedAck { transfer, event }) = &frame
                && let Some((forge, root)) = self.ack_forge.lock().unwrap().clone()
            {
                use base64::Engine;
                use crate::handover::live::wire::{prepared_ack_content, prepared_ack_proof};
                let real = nostr::Event::from_json(event).unwrap();
                let raw = base64::engine::general_purpose::STANDARD.decode(&real.content).unwrap();
                let at = 4 + "AxeNStax/handover/prepared-ack/v2".len() + 16;
                let prepared: crate::handover::Hash = raw[at..at + 32].try_into().unwrap();
                let proof = match forge {
                    AckForge::V1Capability => crate::handover::crypto::root_capability(&root, &TEST_BEARER).unwrap(),
                    AckForge::OtherBearer => prepared_ack_proof(&root, &[0x11; 16], prepared),
                    AckForge::OtherPrepared => prepared_ack_proof(&root, &TEST_BEARER, [0x77; 32]),
                };
                assert_eq!(real.content, prepared_ack_content(*transfer, prepared, prepared_ack_proof(&root, &TEST_BEARER, prepared)), "the real ack");
                let forged = pollster::block_on(
                    nostr::EventBuilder::new(real.kind, prepared_ack_content(*transfer, prepared, proof))
                        .custom_created_at(real.created_at)
                        .sign(&keys(RECIPIENT_RUNTIME)),
                )
                .unwrap();
                let f = Frame::PreparedAck { transfer: *transfer, event: forged.as_json() };
                return self.inner.send_to_server(&handover_frame(&f.encode().unwrap()));
            }
            if let Some(Frame::PreparedAck { transfer, event }) = &frame
                && self.forge_ack.load(std::sync::atomic::Ordering::SeqCst)
            {
                let real = nostr::Event::from_json(event).unwrap();
                let forged = pollster::block_on(
                    nostr::EventBuilder::new(real.kind, real.content.clone()).custom_created_at(real.created_at).sign(&keys(0x33)),
                )
                .unwrap();
                let f = Frame::PreparedAck { transfer: *transfer, event: forged.as_json() };
                return self.inner.send_to_server(&handover_frame(&f.encode().unwrap()));
            }
            let ready = frame.is_some_and(|f| matches!(f, Frame::Ready { .. }));
            match (ready, self.forged_ready.lock().unwrap().clone()) {
                (true, Some(forged)) => self.inner.send_to_server(&forged),
                _ => self.inner.send_to_server(data),
            }
        }
        fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
            let pkt = self.inner.try_recv_from_server()?;
            let forged = *self.forged_bearer.lock().unwrap();
            match (crate::handover::live::recipient::handover_payload(&pkt).and_then(|b| Frame::decode(b).ok()), forged) {
                (Some(Frame::Bearer { transfer, .. }), Some(wrong)) => {
                    let f = Frame::Bearer { transfer, bearer: crate::handover::live::bearer::Bearer::new(wrong) };
                    Some(handover_frame(&f.encode().unwrap()))
                }
                _ => Some(pkt),
            }
        }
        fn is_closed(&self) -> bool {
            self.inner.is_closed()
        }
        fn channel_binding(&self) -> Option<[u8; 32]> {
            self.inner.channel_binding()
        }
    }

    const SOURCE_RUNTIME: u8 = 0x81;
    const SOURCE_PERSONA: u8 = 0x82;
    const RECIPIENT_PERSONA: u8 = 0x71;
    const RECIPIENT_RUNTIME: u8 = 0x72;

    struct Pair {
        hs: HostedServer,
        host: OwnedSimParts,
        src: SourceSession,
        rc: RemoteClient,
        dst: Option<RecipientSession>,
        seat: Seat,
        live: Live,
        root: RootContext,
        forged_ready: Arc<Mutex<Option<Vec<u8>>>>,
        /// Replaces the bearer in the source's `Frame::Bearer` (a wrong one).
        forged_bearer: Arc<Mutex<Option<[u8; 16]>>>,
        forge_ack: Arc<std::sync::atomic::AtomicBool>,
        ack_forge: Arc<Mutex<Option<(AckForge, RootContext)>>>,
        drop_ack: Arc<std::sync::atomic::AtomicBool>,
        relay: crate::handover::relay_fixture::Relay,
        /// More fixture relays the invitation names (`pair_n`).
        extra: Vec<crate::handover::relay_fixture::Relay>,
        dir: Dir,
    }

    /// H5: the invitation names the loopback fixture relay (never a real one).
    fn root_at(relay: &str) -> RootContext {
        RootContext::new(keys(SOURCE_RUNTIME).public_key().to_bytes(), &npub(&keys(SOURCE_PERSONA)), now() + 3600, &[relay.into()], now())
            .unwrap()
    }
    /// Tests that never publish share one fixture relay.
    fn root() -> RootContext {
        static RELAY: std::sync::OnceLock<crate::handover::relay_fixture::Relay> = std::sync::OnceLock::new();
        root_at(&RELAY.get_or_init(crate::handover::relay_fixture::Relay::start).url())
    }
    const TEST_BEARER: [u8; 16] = [0x5A; 16];
    /// CP9 review LOW-6: the start-up scans take only this install's records.
    /// CP13: the install ledger every session in this test process writes
    /// (`<per-process test data root>/handover/ledger`); source and
    /// recipient entries are told apart by role.
    fn src_install() -> &'static crate::handover::ledger::Ledger {
        static L: std::sync::OnceLock<crate::handover::ledger::Ledger> = std::sync::OnceLock::new();
        L.get_or_init(crate::handover::ledger::Ledger::install)
    }
    fn dst_install() -> &'static crate::handover::ledger::Ledger {
        src_install()
    }
    fn bearer() -> crate::handover::live::bearer::Bearer {
        crate::handover::live::bearer::Bearer::new(TEST_BEARER)
    }
    /// The source runtime's player attestation for the source persona.
    fn source_attestation() -> nostr::Event {
        pollster::block_on(crate::runtime_identity::mint_player_attestation(
            &keys(SOURCE_PERSONA),
            &keys(SOURCE_RUNTIME).public_key(),
            Timestamp::from(now()),
            1,
        ))
        .unwrap()
    }
    fn joined_host() -> JoinedHost {
        JoinedHost {
            persona: keys(SOURCE_PERSONA).public_key().to_bytes(),
            runtime: keys(SOURCE_RUNTIME).public_key().to_bytes(),
        }
    }
    fn identity() -> RecipientIdentity {
        let runtime = keys(RECIPIENT_RUNTIME);
        let attestation = pollster::block_on(crate::runtime_identity::mint_player_attestation(
            &keys(RECIPIENT_PERSONA),
            &runtime.public_key(),
            Timestamp::from(now()),
            1,
        ))
        .unwrap();
        RecipientIdentity { runtime, attestation }
    }
    /// CP10: the recipient's records live in its import's own working world
    /// `handover/`. Whether `t`'s `<label>` record exists there.
    fn has_record(dir: &Dir, label: &str, t: crate::handover::Id) -> bool {
        let suffix = format!("-{}", hex::encode(t));
        std::fs::read_dir(dir.0.join("imports")).into_iter().flatten().flatten().any(|e| {
            e.file_name().to_str().is_some_and(|n| n.starts_with("handover-") && n.ends_with(&suffix))
                && crate::handover::live::authority::dir(&e.path().join("working"))
                    .join(format!("{label}-{}.dat", hex::encode(t)))
                    .is_file()
        })
    }
    /// The import's own records store (the session still holds its checkpoint).
    fn import_store(p: &Pair) -> MarkerStore {
        crate::handover::live::authority::store(&p.dst.as_ref().unwrap().checkpoint().unwrap().working_world()).unwrap()
    }

    /// A lent source with one signed joiner that keeps PLAYING through its
    /// RemoteClient: the Propose reaches it the ordinary way.
    fn pair(tag: &str) -> Pair {
        pair_n(tag, 1)
    }
    /// A pair whose invitation names `n` loopback fixture relays.
    fn pair_n(tag: &str, n: usize) -> Pair {
        let dir = Dir::new(tag);
        for d in ["imports", "markers", "scratch", "world"] {
            std::fs::create_dir_all(dir.0.join(d)).unwrap();
        }
        let (mut hs, mut host) = start_lent(&format!("handover-{tag}"));
        let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [RECIPIENT_PERSONA; 32]);
        let forged_ready = Arc::new(Mutex::new(None));
        let forged_bearer = Arc::new(Mutex::new(None));
        let forge_ack = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ack_forge = Arc::new(Mutex::new(None));
        let drop_ack = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let tap = Tap {
            inner: Box::new(client),
            forged_ready: forged_ready.clone(),
            forged_bearer: forged_bearer.clone(),
            forge_ack: forge_ack.clone(),
            ack_forge: ack_forge.clone(),
            drop_ack: drop_ack.clone(),
        };
        let rc = RemoteClient::from_transport(Box::new(tap), crate::remote_client::build_join_request_guest("dup", 0), None);
        let relay = crate::handover::relay_fixture::Relay::start();
        let extra: Vec<_> = (1..n).map(|_| crate::handover::relay_fixture::Relay::start()).collect();
        let urls: Vec<String> = std::iter::once(relay.url()).chain(extra.iter().map(|r| r.url())).collect();
        let root =
            RootContext::new(keys(SOURCE_RUNTIME).public_key().to_bytes(), &npub(&keys(SOURCE_PERSONA)), now() + 3600, &urls, now()).unwrap();
        let lineage = root_lineage(&root);
        let cfg = SourceConfig {
            runtime: keys(SOURCE_RUNTIME),
            root: root.clone(),
            authority: Authority::root(&root, lineage).unwrap(),
            lineage,
            world: [6; 16],
            generation: 1,
            scratch: dir.0.join("scratch"),
            validity: 600,
            world_dir: Some(dir.0.join("world")),
            bearer: bearer(),
            attestation: source_attestation(),
        };
        let seat = Seat { slot, generation, key };
        let source_key = keys(SOURCE_PERSONA).public_key().to_bytes();
        let action = HostAction { seat, source_key, world_name: "Loop".into() };
        let live = Live { signed_in: Some(source_key), local_seats: 1 };
        let src = SourceSession::begin(&mut hs, action, cfg, live, now()).expect("eligible");
        Pair { hs, host, src, rc, dst: None, seat, live, root, forged_ready, forged_bearer, forge_ack, ack_forge, drop_ack, relay, extra, dir }
    }

    /// The joined client reads the proposal through its ordinary poll, the
    /// recipient session checks it against the verified host.
    fn propose(p: &mut Pair) {
        p.host.lend_tick(&mut p.hs);
        p.rc.poll();
        let frames = p.rc.take_handover_frames();
        let propose = frames.iter().find(|b| matches!(Frame::decode(b), Ok(Frame::Propose { .. })));
        let dst = RecipientSession::propose(
            propose.expect("the Propose reached the joined client"),
            joined_host(),
            identity(),
            p.dir.0.join("imports"),
            now(),
        )
        .expect("a proposal from the verified host");
        assert_eq!(dst.stage(), RecipientStage::AwaitingConsent);
        assert_eq!(dst.proposal().world_name, "Loop");
        assert_eq!(dst.proposal().source_npub, npub(&keys(SOURCE_PERSONA)));
        p.dst = Some(dst);
    }

    /// Explicit consent: the session takes the joined connection (parked).
    fn consent(p: &mut Pair) {
        propose(p);
        let dst = p.dst.as_mut().unwrap();
        let prop = dst.proposal().clone();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let c = RecipientConsent { transfer: prop.transfer, world_name: prop.world_name, source_npub: prop.source_npub };
        let link = p.rc.park_transport();
        dst.consent(c, socket, link, now()).unwrap();
        assert!(p.rc.is_parked());
        p.host.lend_tick(&mut p.hs); // ordinary processing routes the Accept
        p.src.pump(&mut p.hs, p.live, now()).unwrap();
        assert_eq!(p.src.stage(), SourceStage::Frozen);
        assert!(p.hs.handover_frozen());
    }

    /// The frozen capture: a raw save of the source's own seat 0 with a
    /// seat-0 pet and a cart; the session neutralises it itself.
    fn offer(p: &mut Pair, seat0: &crate::player_slot::PlayerSlot) -> Vec<u8> {
        let (r, sidecar) = try_offer(p, seat0);
        r.unwrap();
        sidecar
    }
    fn try_offer(
        p: &mut Pair,
        seat0: &crate::player_slot::PlayerSlot,
    ) -> (Result<(), crate::handover::live::eligibility::Refusal>, Vec<u8>) {
        let transfer = p.src.transfer();
        let source_key = keys(SOURCE_PERSONA).public_key().to_bytes();
        let context = RecordContext { participant: source_key, lineage: root_lineage(&p.root), generation: 1, transfer };
        let sidecar = capture::local0_sidecar(seat0, [9; 16]).unwrap();
        let progress = capture::local0_progress(seat0, context).unwrap();
        let meta = crate::handover::checkpoint::transfer_metadata(&crate::save::WorldMeta::new("Loop")).unwrap();
        let mut ecs = hecs::World::new();
        crate::cart::spawn_cart(&mut ecs, (1, 70, 1));
        let carts = crate::save::carts_to_saved(&ecs);
        let mut wolf = crate::wolf::WolfData::untamed();
        wolf.ownership.owner_pubkey = "local-player-0".into();
        let pet = crate::save::SavedTamedPet { x: 2.0, y: 70.0, z: 2.0, data: crate::save::SavedTamedPetData::Wolf(wolf) };
        let save = crate::save::build_world_save(&p.host.world, std::slice::from_ref(seat0), 42, &carts, &[pet]).unwrap();
        let cap = Capture {
            meta: &meta,
            save,
            world: &p.host.world,
            disk_chunks: None,
            exhibit_dir: None,
            source_sidecar: &sidecar,
            source_progress: &progress,
        };
        // M-B: the pack runs on a worker; wait for it as later pumps would.
        let r = p.src.offer(&mut p.hs, p.live, cap, now()).and_then(|()| p.src.settle_pack(&mut p.hs, p.live, now()));
        (r, sidecar)
    }

    /// The frozen capture's parts, without offering them.
    fn start_offer(p: &mut Pair, seat0: &crate::player_slot::PlayerSlot) {
        let transfer = p.src.transfer();
        let source_key = keys(SOURCE_PERSONA).public_key().to_bytes();
        let context = RecordContext { participant: source_key, lineage: root_lineage(&p.root), generation: 1, transfer };
        let sidecar = capture::local0_sidecar(seat0, [9; 16]).unwrap();
        let progress = capture::local0_progress(seat0, context).unwrap();
        let meta = crate::handover::checkpoint::transfer_metadata(&crate::save::WorldMeta::new("Loop")).unwrap();
        let save = crate::save::build_world_save(&p.host.world, std::slice::from_ref(seat0), 42, &[], &[]).unwrap();
        let cap = Capture {
            meta: &meta,
            save,
            world: &p.host.world,
            disk_chunks: None,
            exhibit_dir: None,
            source_sidecar: &sidecar,
            source_progress: &progress,
        };
        p.src.offer(&mut p.hs, p.live, cap, now()).unwrap();
    }

    fn scratch_entries(p: &Pair) -> usize {
        std::fs::read_dir(p.dir.0.join("scratch")).unwrap().count()
    }

    /// CP2 review M-B: the pack runs on a worker. While it runs the pump
    /// returns at once (the frame and Cancel keep running); a cancel during a
    /// slow pack fences its late result off: nothing is offered, its scratch
    /// is removed, the recipient hears only the abort, and play resumes.
    #[test]
    fn handover_cancel_during_a_slow_pack_sends_nothing_and_leaves_no_scratch() {
        let mut p = pair("slowpack");
        consent(&mut p);
        let (reached, release) = p.src.hold_pack();
        start_offer(&mut p, &seat0());
        assert_eq!(p.src.stage(), SourceStage::Packing);
        // The worker has written the whole archive and waits: its result
        // will be complete, so only the fence can keep it from being used.
        reached.recv().unwrap();
        assert!(scratch_entries(&p) > 0, "the held pack has scratch data");
        for _ in 0..3 {
            p.src.pump(&mut p.hs, p.live, now()).unwrap();
        }
        assert_eq!(p.src.stage(), SourceStage::Packing, "the pump waited for the pack");
        let tally = p.host.world.sim_tally;
        p.src.abort(&mut p.hs, "cancelled");
        release.send(()).unwrap();
        p.src.join_pack_worker();
        // The finished (late) pack is drained by the next pump and dropped.
        assert!(p.src.pump(&mut p.hs, p.live, now()).is_ok(), "a cancelled pack's result was used");
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)));
        assert!(p.src.core().is_none(), "a cancelled pack was certified");
        assert_eq!(scratch_entries(&p), 0, "a cancelled pack left scratch data");
        assert!(!p.hs.handover_frozen() && p.hs.handover_link().is_none());
        p.host.lend_tick(&mut p.hs);
        assert!(p.host.world.sim_tally != tally, "play resumed");
        // The recipient heard the abort and nothing of the copy.
        let dst = p.dst.as_mut().unwrap();
        assert!(dst.pump(now()).is_err(), "the recipient heard the cancel");
        let (link, _) = dst.return_link().expect("connection handed back");
        while let Some(pkt) = link.try_recv_from_server() {
            if let Some(b) = crate::handover::live::recipient::handover_payload(&pkt) {
                let f = Frame::decode(b).unwrap();
                assert!(
                    !matches!(f, Frame::ManifestPart { .. } | Frame::Offer { .. } | Frame::Chunk { .. }),
                    "a cancelled pack was sent: {f:?}"
                );
            }
        }
    }

    /// A pack worker that dies without a result refuses (the source would
    /// otherwise sit frozen in `Packing`, which has no stall deadline).
    #[test]
    fn handover_pack_worker_lost_refuses_and_resumes() {
        let mut p = pair("lostpack");
        consent(&mut p);
        let (_reached, release) = p.src.hold_pack();
        start_offer(&mut p, &seat0());
        p.src.test_lose_pack_worker();
        let r = p.src.pump(&mut p.hs, p.live, now()).unwrap_err();
        assert_eq!(r.0, "The world couldn't be packed for the handover. Play has resumed.");
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)));
        assert!(!p.hs.handover_frozen(), "the source stayed frozen after its pack worker died");
        drop(release);
        p.src.join_pack_worker();
    }

    /// While the pack runs the source keeps the recipient's stall deadline
    /// fed (heartbeat); a finished pack is then offered as before.
    #[test]
    fn handover_pack_heartbeat_keeps_the_recipient_waiting_then_offers() {
        let mut p = pair("beat");
        consent(&mut p);
        let (_reached, release) = p.src.hold_pack();
        start_offer(&mut p, &seat0());
        let later = now() + crate::handover::live::source::HEARTBEAT_SECS;
        p.src.pump(&mut p.hs, p.live, later).unwrap();
        let stall = later + crate::handover::live::source::STALL_SECS - 1;
        p.dst.as_mut().unwrap().pump(later).unwrap();
        p.dst.as_mut().unwrap().pump(stall).unwrap();
        assert_eq!(p.dst.as_ref().unwrap().stage(), RecipientStage::Accepted, "the heartbeat kept the recipient waiting");
        release.send(()).unwrap();
        p.src.settle_pack(&mut p.hs, p.live, now()).unwrap();
        assert_eq!(p.src.stage(), SourceStage::Streaming);
        stream(&mut p, 200);
        assert_eq!(p.src.stage(), SourceStage::ReadyVerified);
    }

    /// L2: after Ready a silent recipient times the source out, and a silent
    /// host times the recipient out; before Retired both mean abort and play.
    #[test]
    fn handover_stall_deadlines_after_ready_and_on_the_recipient_restore_play() {
        let stall = crate::handover::live::source::STALL_SECS + 1;
        let mut p = pair("stall-src");
        consent(&mut p);
        offer(&mut p, &seat0());
        stream(&mut p, 200);
        assert_eq!(p.src.stage(), SourceStage::ReadyVerified);
        assert!(p.src.pump(&mut p.hs, p.live, now() + stall).is_err(), "a silent recipient held the source after Ready");
        assert!(!p.hs.handover_frozen(), "the source resumed");
        let mut q = pair("stall-dst");
        consent(&mut q);
        let dst = q.dst.as_mut().unwrap();
        dst.pump(now()).unwrap(); // what was already waiting is read first
        assert!(dst.pump(now() + stall).is_err(), "a silent host held the recipient");
        assert!(matches!(dst.stage(), RecipientStage::Aborted(_)));
        assert!(dst.return_link().is_some(), "the joined connection comes back");
    }

    /// L3: a `--no-lend` host refuses (its joiners' items live in a copy).
    #[test]
    fn handover_no_lend_host_refuses_with_a_reason() {
        let mut p = pair("nolend");
        consent_only(&mut p);
        // The Accept is routed; the host now runs as `--no-lend` would.
        p.hs.handover_test_set_owned();
        let r = p.src.pump(&mut p.hs, p.live, now()).unwrap_err();
        assert_eq!(r.0, crate::handover::live::copy::NO_LEND);
        assert!(!p.hs.handover_frozen());
    }

    /// L4: the transfer metadata carries the live world's work and clock.
    #[test]
    fn handover_transfer_metadata_takes_the_live_work_and_clock() {
        let p = pair("meta");
        let mut world = crate::world::World::new();
        world.total_work = 777;
        world.total_ticks = 4242;
        let mut saved = crate::save::WorldMeta::new("Loop");
        saved.total_work = 1;
        saved.pop_secret = Some([5; 32]);
        let meta = crate::handover_game::live_transfer_metadata(saved, &world).unwrap();
        assert_eq!((meta.total_work, meta.total_ticks), (777, 4242));
        assert!(meta.pop_secret.is_none(), "still sanitised");
        drop(p);
    }

    /// Pump both sides, ticking the frozen source the way an ungated caller
    /// would every round: it must simulate nothing.
    fn stream(p: &mut Pair, rounds: usize) {
        let tally = p.host.world.sim_tally;
        // M1: the import runs on a worker; rounds spent waiting for it don't
        // count, but a wall-clock bound does.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut n = 0;
        while n < rounds && std::time::Instant::now() < deadline {
            if p.dst.as_ref().unwrap().stage() == RecipientStage::Importing {
                std::thread::sleep(std::time::Duration::from_millis(2));
            } else {
                n += 1;
            }
            p.host.lend_tick(&mut p.hs);
            p.dst.as_mut().unwrap().pump(now()).unwrap();
            if p.src.pump(&mut p.hs, p.live, now()).is_err() || p.src.stage() == SourceStage::ReadyVerified {
                break;
            }
        }
        assert_eq!(p.host.world.sim_tally, tally, "a frozen source simulated while streaming");
    }

    fn seat0() -> crate::player_slot::PlayerSlot {
        let mut seat0 = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(3.0, 70.0, 3.0), 1.0);
        seat0.reputation.per_village.insert((2, -2), -40);
        seat0.kill_counter.insert(crate::mob::MobType::Pig, 7);
        seat0.bounties_claimed.insert(11, 2);
        seat0.inventory.set_slot(4, Some(crate::item::ItemStack::new_block(block::GLASS, 9)));
        seat0
    }

    #[test]
    fn handover_loopback_reaches_durable_ready_with_distinct_records_and_latched_listener() {
        let mut p = pair("ready");
        consent(&mut p);
        // The joined world is torn down: its RemoteClient goes, the session's
        // connection stays.
        let mut rc = std::mem::replace(
            &mut p.rc,
            RemoteClient::from_transport(Box::new(crate::transport::ClosedClientTransport), crate::remote_client::build_join_request_guest("x", 0), None),
        );
        rc.disconnect();
        drop(rc);
        let seat0 = seat0();
        let sidecar = offer(&mut p, &seat0);
        stream(&mut p, 10_000);
        let dst = p.dst.as_ref().unwrap();
        assert_eq!(p.src.stage(), SourceStage::ReadyVerified);
        assert_eq!(dst.stage(), RecipientStage::Ready);
        assert!(p.hs.handover_frozen(), "still frozen at Ready");
        assert!(!dst.gameplay_admitted(), "Ready is not activation");
        assert!(import_store(&p).read(MarkerKind::Ready, p.src.transfer()).unwrap().is_some(), "Ready persisted");
        // Distinct own records, each bound to its own identity.
        let transfer = p.src.transfer();
        let lineage = root_lineage(&p.root);
        let context = |participant| RecordContext { participant, lineage, generation: 1, transfer };
        let cp = dst.checkpoint().unwrap();
        let got = |who: [u8; 32], kind: &str| {
            let n = nostr::PublicKey::from_slice(&who).unwrap().to_bech32().unwrap();
            std::fs::read(cp.received_world().join(format!("{kind}/{n}.dat"))).unwrap()
        };
        let source_key = keys(SOURCE_PERSONA).public_key().to_bytes();
        let src_rec = PersonalRecord::decode(&got(source_key, "participants"), &context(source_key)).unwrap();
        assert_eq!(src_rec.reputation, vec![((2, -2), -40)]);
        assert_eq!(src_rec.kills, vec![(crate::mob::MobType::Pig, 7)]);
        assert_eq!(src_rec.claims, vec![(11, 2)]);
        assert!(src_rec.local.is_some());
        let dst_rec = PersonalRecord::decode(&got(p.seat.key, "participants"), &context(p.seat.key)).unwrap();
        assert!(dst_rec.is_default() && dst_rec.local.is_none(), "the recipient inherits nothing");
        assert_eq!(got(source_key, "joiners"), sidecar);
        assert_ne!(got(p.seat.key, "joiners"), sidecar);
        // The session neutralised the raw copy: the seat-0 pet is the
        // source's by npub, the cart came across, no seat rows remain.
        let bytes = std::fs::read(cp.received_world().join("world.dat")).unwrap();
        let (body, _) = crate::save_format::split_save_footer(&bytes);
        let world: crate::save::WorldSave = bincode::deserialize(body).unwrap();
        assert!(world.players.is_empty() && world.inventory.is_empty());
        assert_eq!(world.carts.len(), 1);
        match &world.saved_mobs[..] {
            [crate::save::SavedTamedPet { data: crate::save::SavedTamedPetData::Wolf(w), .. }] => {
                assert_eq!(w.ownership.owner_pubkey, npub(&keys(SOURCE_PERSONA)))
            }
            other => panic!("the pet didn't come across: {other:?}"),
        }
    }

    /// What a connecting client actually gets from the latched listener: a
    /// refused connection, never an established one.
    #[test]
    fn handover_latched_listener_refuses_a_connecting_client() {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let l = crate::handover::live::listener::LatchedListener::bind(socket).unwrap();
        let addr = l.local_addr();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let outcome = rt.block_on(async move {
            let ep = crate::network::create_client_endpoint().unwrap();
            let connecting = ep.connect(addr, "axenstax-server").unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(10), connecting).await
        });
        match outcome {
            Ok(Err(quinn::ConnectionError::ConnectionClosed(c))) => {
                assert_eq!(c.error_code, quinn::TransportErrorCode::CONNECTION_REFUSED)
            }
            Ok(Err(e)) => panic!("unexpected refusal shape: {e}"),
            Ok(Ok(_)) => panic!("the latched listener let a client connect"),
            Err(_) => panic!("the latched listener never answered"),
        }
        assert!(l.refused() > 0);
    }

    #[test]
    fn handover_recipient_cancel_before_ready_restores_play_on_both_sides() {
        let mut p = pair("cancel");
        consent(&mut p);
        let tally = p.host.world.sim_tally;
        let dst = p.dst.as_mut().unwrap();
        dst.abort("changed my mind");
        assert!(p.src.pump(&mut p.hs, p.live, now()).is_err());
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)));
        assert!(!p.hs.handover_frozen() && p.hs.handover_link().is_none());
        p.host.lend_tick(&mut p.hs);
        assert!(p.host.world.sim_tally != tally, "the source simulates again");
        // The recipient's joined client gets its connection back and keeps
        // playing: it hears that play resumed.
        let (link, parked) = p.dst.as_mut().unwrap().return_link().expect("connection handed back");
        p.rc.restore_transport(link, parked);
        assert!(!p.rc.is_parked());
        p.host.lend_tick(&mut p.hs);
        p.rc.poll();
        let frames: Vec<_> = p.rc.take_handover_frames().iter().map(|b| Frame::decode(b).unwrap()).collect();
        assert!(frames.iter().any(|f| matches!(f, Frame::Notice { paused: false, .. })), "{frames:?}");
        assert!(crate::world_exit::remote_session_end_reason(&p.rc).is_none(), "still in the world");
    }

    #[test]
    fn handover_source_refuses_a_ready_signed_over_the_wrong_challenge() {
        let mut p = pair("forged");
        consent(&mut p);
        offer(&mut p, &seat0());
        // A man in the middle swaps the recipient's Ready for one signed by
        // the same runtime over a different challenge.
        let core = p.src.core().unwrap().clone();
        let forged = pollster::block_on(crate::handover::crypto::sign_ready(&keys(RECIPIENT_RUNTIME), &core, [0x33; 16])).unwrap();
        let frame = Frame::Ready { transfer: p.src.transfer(), event: forged.as_json() }.encode().unwrap();
        let mut pkt = vec![protocol::PacketType::Handover as u8];
        pkt.extend_from_slice(&frame);
        *p.forged_ready.lock().unwrap() = Some(pkt);
        let mut refusal = None;
        for _ in 0..10_000 {
            let _ = p.dst.as_mut().unwrap().pump(now());
            if p.dst.as_ref().unwrap().stage() == RecipientStage::Importing {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            if let Err(r) = p.src.pump(&mut p.hs, p.live, now()) {
                refusal = Some(r);
                break;
            }
            assert_ne!(p.src.stage(), SourceStage::ReadyVerified, "the source accepted a forged Ready");
        }
        assert_eq!(refusal.map(|r| r.0), Some("The recipient's Ready didn't verify."));
        assert!(!p.hs.handover_frozen(), "a refused Ready releases the source");
    }

    /// Consent over the parked connection, without pumping the source.
    fn consent_only(p: &mut Pair) {
        propose(p);
        let dst = p.dst.as_mut().unwrap();
        let prop = dst.proposal().clone();
        let c = RecipientConsent { transfer: prop.transfer, world_name: prop.world_name, source_npub: prop.source_npub };
        dst.consent(c, std::net::UdpSocket::bind("127.0.0.1:0").unwrap(), p.rc.park_transport(), now()).unwrap();
        p.host.lend_tick(&mut p.hs);
    }

    #[test]
    fn handover_identity_change_since_the_host_action_refuses_at_freeze() {
        let mut p = pair("ident");
        consent_only(&mut p);
        let other = Live { signed_in: Some([7; 32]), ..p.live };
        let r = p.src.pump(&mut p.hs, other, now()).unwrap_err();
        assert_eq!(r.0, "You signed in as someone else since you started. Start the handover again.");
        assert!(!p.hs.handover_frozen() && p.hs.handover_link().is_none(), "never frozen");
    }

    #[test]
    fn handover_saved_record_written_after_the_host_action_refuses_at_freeze() {
        // A player who joined and left during the consent window: their
        // record refuses rather than being silently left out of the copy.
        let mut p = pair("late-record");
        let late = keys(0x44).public_key();
        std::fs::create_dir_all(p.dir.0.join("world/joiners")).unwrap();
        std::fs::write(p.dir.0.join(format!("world/joiners/{}.dat", late.to_bech32().unwrap())), b"opaque").unwrap();
        consent_only(&mut p);
        let r = p.src.pump(&mut p.hs, p.live, now()).unwrap_err();
        assert!(r.0.starts_with("Other players have saved records"), "{}", r.0);
        assert!(!p.hs.handover_frozen());
    }

    #[test]
    fn handover_sign_out_before_the_frozen_capture_refuses_the_offer() {
        let mut p = pair("ident-offer");
        consent(&mut p);
        let seat0 = seat0();
        p.live = Live { signed_in: None, ..p.live };
        let (r, _) = try_offer(&mut p, &seat0);
        assert_eq!(r.err().map(|e| e.0), Some("Sign in before handing this world over."));
        assert!(!p.hs.handover_frozen() && matches!(p.src.stage(), SourceStage::Aborted(_)));
    }

    #[test]
    fn handover_host_action_persona_must_be_the_authority_persona() {
        let (mut hs, mut host) = start_lent("handover-persona");
        let (_c, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x66; 32]);
        let root = root();
        let lineage = root_lineage(&root);
        let cfg = SourceConfig {
            runtime: keys(SOURCE_RUNTIME),
            root: root.clone(),
            authority: Authority::root(&root, lineage).unwrap(),
            lineage,
            world: [6; 16],
            generation: 1,
            scratch: std::env::temp_dir(),
            validity: 600,
            world_dir: None,
            bearer: bearer(),
            attestation: source_attestation(),
        };
        let stranger = keys(0x99).public_key().to_bytes();
        let action = HostAction { seat: Seat { slot, generation, key }, source_key: stranger, world_name: "Loop".into() };
        let live = Live { signed_in: Some(stranger), local_seats: 1 };
        let r = SourceSession::begin(&mut hs, action, cfg, live, now()).err().unwrap();
        assert_eq!(r.0, "This world's invitation belongs to a different identity than the one signed in.");
        assert!(hs.handover_link().is_none());
    }

    #[test]
    fn handover_recipient_abort_after_ready_releases_the_source() {
        let mut p = pair("after-ready");
        consent(&mut p);
        offer(&mut p, &seat0());
        stream(&mut p, 10_000);
        assert_eq!(p.src.stage(), SourceStage::ReadyVerified);
        p.dst.as_mut().unwrap().abort("changed my mind");
        assert!(p.src.pump(&mut p.hs, p.live, now()).is_err(), "the source hears the Abort after Ready");
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)));
        assert!(!p.src.bearer_disclosed() && !p.src.needs_invite_rotation(), "an abort before disclosure rotated the invite");
        assert!(!p.hs.handover_frozen(), "and resumes play");
        // P3: Ready is still before Prepared: the inert import is discarded
        // (directory, Ready marker, inert marker), nothing else is touched.
        let t = p.src.transfer();
        let dst = p.dst.as_ref().unwrap();
        assert!(dst.checkpoint().is_none(), "an aborted inert import was kept");
        assert_eq!(import_dirs(&p), Vec::<String>::new(), "the import folder was left behind");
        assert!(!has_record(&p.dir, "ready", t), "the Ready marker was left behind");
        assert!(crate::handover::live::inert::marked(&p.dir.0.join("imports"), t).unwrap().is_none());
    }

    /// Everything under the recipient's imports folder but inert markers.
    fn import_dirs(p: &Pair) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(p.dir.0.join("imports"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with("inert-"))
            .collect();
        v.sort();
        v
    }

    /// P3: the import is marked inert before its data is written; a session
    /// that ends without cleaning up (a crash) leaves a marked import that a
    /// sweep with no live session discards, while a live session's import
    /// and an unmarked (Prepared/Activated) import are never touched.
    #[test]
    fn handover_inert_import_is_marked_and_the_sweep_discards_only_marked_ones() {
        use crate::handover::live::inert;
        let mut p = pair("inert");
        consent(&mut p);
        offer(&mut p, &seat0());
        stream(&mut p, 10_000);
        assert_eq!(p.dst.as_ref().unwrap().stage(), RecipientStage::Ready);
        let t = p.src.transfer();
        let imports = p.dir.0.join("imports");
        assert!(inert::marked(&imports, t).unwrap().is_some(), "the import wasn't marked inert");
        let kept = imports.join("handover-00-unmarked");
        std::fs::create_dir_all(kept.join("working")).unwrap();
        assert_eq!(inert::sweep(&imports, Some(t)).unwrap(), 0, "the live session's import was swept");
        assert_eq!(import_dirs(&p).len(), 2);
        // The process "crashes": the session is gone without an abort.
        drop(p.dst.take());
        assert_eq!(inert::sweep(&imports, None).unwrap(), 1);
        assert_eq!(import_dirs(&p), vec!["handover-00-unmarked".to_string()], "only the marked import goes");
        assert!(!has_record(&p.dir, "ready", t), "its Ready marker went with it");
        assert!(inert::marked(&imports, t).unwrap().is_none());
        // A Prepared marker makes a transfer no longer inert: refused.
        inert::mark(&imports, [0x33; 16], [0x44; 16]).unwrap();
        let prepared = imports.join(format!("handover-{}-{}", hex::encode([0x44u8; 16]), hex::encode([0x33u8; 16])));
        let store = crate::handover::live::authority::store(&prepared.join("working")).unwrap();
        store.persist(MarkerKind::Prepared, [0x33; 16], b"prepared").unwrap();
        assert!(inert::discard(&imports, [0x33; 16]).is_err(), "a Prepared import was discarded");
        assert!(prepared.is_dir(), "a Prepared import was removed");
    }

    /// Pump until the recipient reaches `want` (M1 tests stop at Importing).
    fn stream_until(p: &mut Pair, want: RecipientStage) {
        for _ in 0..10_000 {
            p.host.lend_tick(&mut p.hs);
            p.dst.as_mut().unwrap().pump(now()).unwrap();
            if p.dst.as_ref().unwrap().stage() == want {
                return;
            }
            p.src.pump(&mut p.hs, p.live, now()).unwrap();
        }
        panic!("the recipient never reached {want:?}");
    }

    /// Review M1: the import runs on a worker. While it is slow the
    /// recipient's heartbeat keeps the source's stall clock fed past
    /// `STALL_SECS`, and the finished import then reaches Ready as before.
    #[test]
    fn handover_slow_import_runs_on_a_worker_and_its_heartbeat_holds_the_source() {
        use crate::handover::live::source::STALL_SECS;
        let mut p = pair("slowimport");
        consent(&mut p);
        offer(&mut p, &seat0());
        let (reached, release) = p.dst.as_mut().unwrap().hold_import();
        stream_until(&mut p, RecipientStage::Importing);
        reached.recv().unwrap();
        let t0 = now();
        for k in 1..=4u64 {
            let t = t0 + k * 30;
            p.dst.as_mut().unwrap().pump(t).expect("the importing recipient gave up");
            p.host.lend_tick(&mut p.hs);
            p.src.pump(&mut p.hs, p.live, t).expect("the source timed out a recipient that was importing");
        }
        assert!(t0 + 4 * 30 > t0 + STALL_SECS, "the import outlasted the stall deadline");
        assert_eq!(p.dst.as_ref().unwrap().stage(), RecipientStage::Importing);
        assert_eq!(p.src.stage(), SourceStage::Streaming);
        release.send(()).unwrap();
        p.dst.as_mut().unwrap().join_import_worker();
        let t = t0 + 4 * 30;
        p.dst.as_mut().unwrap().pump(t).unwrap();
        assert_eq!(p.dst.as_ref().unwrap().stage(), RecipientStage::Ready);
        p.host.lend_tick(&mut p.hs);
        p.src.pump(&mut p.hs, p.live, t).unwrap();
        assert_eq!(p.src.stage(), SourceStage::ReadyVerified);
    }

    /// Review M1: a cancel while the (complete, renamed) import is held on
    /// its worker never reaches Ready, and the worker discards exactly that
    /// inert import: no folder, Ready marker or inert marker remains. A
    /// worker that dies without a result refuses the same way.
    #[test]
    fn handover_cancelled_or_lost_import_worker_never_reaches_ready_and_leaves_nothing() {
        let t_of = |p: &Pair| p.src.transfer();
        for lost in [false, true] {
            let mut p = pair(if lost { "lostimport" } else { "cancelimport" });
            consent(&mut p);
            offer(&mut p, &seat0());
            let (reached, release) = p.dst.as_mut().unwrap().hold_import();
            stream_until(&mut p, RecipientStage::Importing);
            reached.recv().unwrap();
            let dst = p.dst.as_mut().unwrap();
            if lost {
                dst.test_lose_import_worker();
                assert!(dst.pump(now()).is_err(), "a lost import worker was waited on");
            } else {
                dst.abort("cancelled");
            }
            assert!(matches!(dst.stage(), RecipientStage::Aborted(_)));
            release.send(()).unwrap();
            dst.join_import_worker();
            assert!(dst.pump(now()).is_ok());
            assert!(matches!(dst.stage(), RecipientStage::Aborted(_)), "a cancelled import reached Ready");
            assert!(dst.checkpoint().is_none());
            assert_eq!(import_dirs(&p), Vec::<String>::new(), "a cancelled import left its data");
            let t = t_of(&p);
            assert!(!has_record(&p.dir, "ready", t), "a cancelled import wrote Ready");
            assert!(crate::handover::live::inert::marked(&p.dir.0.join("imports"), t).unwrap().is_none());
            assert!(p.dst.as_mut().unwrap().return_link().is_some(), "the joined connection comes back");
            p.host.lend_tick(&mut p.hs);
            assert!(p.src.pump(&mut p.hs, p.live, now()).is_err(), "the source hears the cancel");
            assert!(!p.hs.handover_frozen(), "and resumes play");
        }
    }

    /// P4: the heartbeat keeps both stall deadlines fed while the pack is
    /// slow, so only the whole-operation deadline ends it: before Retired
    /// that is an abort, play resumes on both sides and the recipient's
    /// connection comes back.
    #[test]
    fn handover_whole_operation_deadline_ends_a_heartbeat_fed_handover() {
        use crate::handover::live::source::OPERATION_SECS;
        let mut p = pair("total");
        consent(&mut p);
        let (_reached, release) = p.src.hold_pack();
        start_offer(&mut p, &seat0());
        // The consent's own clock read may be a second earlier than t0.
        let t0 = now();
        let mut t = t0;
        while t + 30 <= t0 + OPERATION_SECS - 2 {
            t += 30;
            p.src.pump(&mut p.hs, p.live, t).expect("the source gave up before the total deadline");
            p.dst.as_mut().unwrap().pump(t).expect("the recipient gave up before the total deadline");
        }
        assert_eq!(p.src.stage(), SourceStage::Packing, "still held by the slow pack");
        let late = t0 + OPERATION_SECS + 2;
        let r = p.src.pump(&mut p.hs, p.live, late).unwrap_err();
        assert_eq!(r.0, crate::handover::live::copy::TOO_LONG_SOURCE);
        assert!(!p.hs.handover_frozen(), "the source stayed frozen past the total deadline");
        let dst = p.dst.as_mut().unwrap();
        let r = dst.pump(late).unwrap_err();
        assert!(
            r.0 == crate::handover::live::copy::TOO_LONG || r.0 == "The host cancelled the handover.",
            "unexpected: {}",
            r.0
        );
        assert!(dst.return_link().is_some(), "the joined connection comes back");
        drop(release);
        p.src.join_pack_worker();
    }

    /// Review LOW-3: while streaming, a repeated Ack is not progress: a
    /// recipient that re-sends its last Ack just inside the stall deadline
    /// no longer holds the source (and every guest) frozen.
    #[test]
    fn handover_repeated_acks_dont_hold_a_streaming_source() {
        use crate::handover::live::source::STALL_SECS;
        let mut p = pair("dupack");
        consent(&mut p);
        offer(&mut p, &seat0());
        assert_eq!(p.src.stage(), SourceStage::Streaming);
        let t0 = now();
        let dup = Frame::Ack { transfer: p.src.transfer(), through: 0 };
        p.dst.as_ref().unwrap().test_send(&dup);
        p.src.pump(&mut p.hs, p.live, t0 + STALL_SECS / 2).expect("inside the stall deadline");
        p.dst.as_ref().unwrap().test_send(&dup);
        assert!(p.src.pump(&mut p.hs, p.live, t0 + STALL_SECS + 5).is_err(), "repeated Acks held the source frozen");
        assert!(!p.hs.handover_frozen(), "the source resumed");
    }

    /// CP5 review LOW-5: before the final Ack the recipient has no reason
    /// to send a heartbeat; a Notice then doesn't feed the stall clock.
    #[test]
    fn handover_heartbeat_before_the_final_ack_doesnt_hold_a_streaming_source() {
        use crate::handover::live::source::STALL_SECS;
        let mut p = pair("earlynotice");
        consent(&mut p);
        offer(&mut p, &seat0());
        assert_eq!(p.src.stage(), SourceStage::Streaming);
        let t0 = now();
        let beat = Frame::Notice { transfer: p.src.transfer(), paused: true };
        p.dst.as_ref().unwrap().test_send(&beat);
        p.src.pump(&mut p.hs, p.live, t0 + STALL_SECS / 2).expect("inside the stall deadline");
        p.dst.as_ref().unwrap().test_send(&beat);
        assert!(p.src.pump(&mut p.hs, p.live, t0 + STALL_SECS + 5).is_err(), "early heartbeats held the source frozen");
        assert!(!p.hs.handover_frozen(), "the source resumed");
    }

    /// Review LOW-6: the source keeps its pointer (mouselook, fighting)
    /// while the recipient decides; only a frozen world frees it.
    #[test]
    fn handover_source_pointer_is_freed_only_once_frozen() {
        use crate::handover_game::source_wants_pointer;
        assert!(!source_wants_pointer(SourceStage::Proposed), "the host lost mouselook while its world still ran");
        assert!(!source_wants_pointer(SourceStage::Aborted("x")));
        for s in [SourceStage::Frozen, SourceStage::Packing, SourceStage::Streaming, SourceStage::ReadyVerified] {
            assert!(source_wants_pointer(s), "{s:?}");
        }
    }

    /// Review LOW-7: a source session dropped without `abort` cancels its
    /// pack; orphaned pack scratch is swept (only `.handover-pack-*`); a
    /// recipient's held router lease goes back within a bound at exit.
    #[test]
    fn handover_drop_cancels_the_pack_sweep_removes_orphans_and_exit_releases_the_lease_bounded() {
        let mut p = pair("drop");
        consent(&mut p);
        let (_reached, release) = p.src.hold_pack();
        start_offer(&mut p, &seat0());
        let cancel = p.src.pack_cancel_flag();
        let Pair { src, hs, dir, .. } = p;
        drop(src);
        assert!(cancel.load(std::sync::atomic::Ordering::SeqCst), "a dropped session left its pack running");
        drop(release);
        drop(hs);
        let scratch = dir.0.join("scratch-sweep");
        std::fs::create_dir_all(scratch.join(".handover-pack-0011")).unwrap();
        std::fs::create_dir_all(scratch.join("keep")).unwrap();
        std::fs::write(scratch.join(".handover-pack-file"), b"x").unwrap();
        assert_eq!(crate::handover::live::inert::sweep_pack_scratch(&scratch).unwrap(), 1);
        assert!(!scratch.join(".handover-pack-0011").exists(), "an orphaned pack folder was left");
        assert!(scratch.join("keep").exists() && scratch.join(".handover-pack-file").exists());
        struct Lease(Arc<Mutex<Vec<&'static str>>>);
        impl crate::handover::live::recipient::HeldLease for Lease {
            fn renew(&mut self) {}
            fn give_back(self: Box<Self>) {
                self.0.lock().unwrap().push("detached");
            }
            fn give_back_within(self: Box<Self>, _: std::time::Duration) {
                self.0.lock().unwrap().push("bounded");
            }
            fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
                self
            }
        }
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut q = pair("drop-lease");
        consent(&mut q);
        let mut dst = q.dst.take().unwrap();
        dst.hold_lease(Box::new(Lease(log.clone())));
        dst.release_lease_within(std::time::Duration::from_millis(1500));
        drop(dst);
        assert_eq!(*log.lock().unwrap(), vec!["bounded"], "the lease went back detached at exit");
    }

    #[test]
    fn handover_recipient_refuses_a_proposal_that_isnt_from_the_host_it_verified() {
        let dir = Dir::new("spoof");
        let r = root();
        let propose = |source_npub: String, root: &RootContext| {
            Frame::Propose {
                transfer: [1; 16],
                world_name: "Loop".into(),
                source_npub,
                challenge: [2; 16],
                root_runtime: root.runtime(),
                root_persona: nostr::PublicKey::from_slice(&root.persona()).unwrap().to_bech32().unwrap(),
                root_expiry: root.expiry(),
                root_relays: root.relays().to_vec(),
                lineage: root_lineage(root),
            }
            .encode()
            .unwrap()
        };
        let session = |payload: &[u8], host: JoinedHost| {
            RecipientSession::propose(payload, host, identity(), dir.0.join("imports"), now())
        };
        assert!(session(&propose(npub(&keys(SOURCE_PERSONA)), &r), joined_host()).is_ok(), "the honest proposal");
        // Displays one npub while the invitation names another.
        let shown = session(&propose(npub(&keys(0x55)), &r), joined_host());
        assert_eq!(shown.err().map(|e| e.0), Some("This handover doesn't come from the host you joined."));
        // A self-consistent proposal for a host this client never verified.
        let other_root = RootContext::new(
            keys(0x56).public_key().to_bytes(),
            &npub(&keys(0x57)),
            now() + 3600,
            &["wss://relay.example".into()],
            now(),
        )
        .unwrap();
        let foreign = session(&propose(npub(&keys(0x57)), &other_root), joined_host());
        assert_eq!(foreign.err().map(|e| e.0), Some("This handover doesn't come from the host you joined."));
    }

    /// After Ready: pump both sides (joining the publish worker as later
    /// frames would wait for it) until the source reaches `want` or stops.
    /// Invariant checked on every round: the source never retires before the
    /// recipient has durably persisted Prepared (M-2).
    fn drive(p: &mut Pair, want: SourceStage) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let t = p.src.transfer();
        while p.src.stage() != want && std::time::Instant::now() < deadline {
            p.host.lend_tick(&mut p.hs);
            let _ = p.dst.as_mut().unwrap().pump(now());
            if p.src.stage() == SourceStage::Publishing {
                p.src.join_publish_worker();
            }
            let r = p.src.pump(&mut p.hs, p.live, now());
            if p.src.past_retirement() {
                assert!(has_record(&p.dir, "prepared", t), "retired before the recipient persisted Prepared");
            }
            if r.is_err() || matches!(p.src.stage(), SourceStage::Aborted(_)) {
                break;
            }
        }
    }
    fn ready_pair(tag: &str) -> Pair {
        let mut p = pair(tag);
        consent(&mut p);
        offer(&mut p, &seat0());
        stream(&mut p, 10_000);
        assert_eq!(p.src.stage(), SourceStage::ReadyVerified);
        p
    }

    /// CP7 Slice A: after Ready the bearer is disclosed, Prepared is
    /// published and read back through the loopback relay, the recipient
    /// verifies the bearer against it, persists Prepared (inert marker
    /// cleared) and acknowledges; the source verifies that and retires with
    /// the exact signed Commit bytes (0600, open guard refuses). Nothing
    /// final escaped. After Retired no Abort, Cancel or deadline resumes the
    /// source, and the recipient's late failure quarantines, never deletes.
    #[test]
    fn handover_source_retires_only_after_a_verified_prepared_ack() {
        let mut p = ready_pair("retire");
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Retired);
        assert_eq!(p.src.stage(), SourceStage::Retired);
        assert!(p.src.bearer_disclosed());
        let dst = p.dst.as_ref().unwrap();
        assert_eq!(dst.stage(), RecipientStage::Prepared);
        assert!(has_record(&p.dir, "prepared", t), "Prepared not persisted");
        assert!(crate::handover::live::inert::marked(&p.dir.0.join("imports"), t).unwrap().is_none(), "the inert marker stayed");
        assert_eq!(p.relay.stored().len(), 1, "only Prepared may be stored before Committed is published");
        // The durable retirement holds the exact signed Commit bytes.
        let world = p.dir.0.join("world");
        let store = crate::handover::live::authority::store(&world).unwrap();
        let bytes = store.read(MarkerKind::Retired, t).unwrap().expect("no retirement record");
        let record = crate::handover::phase::Retirement::decode(&bytes).unwrap();
        let commit = p.src.commit_event().expect("no Commit").clone();
        assert_eq!(record.committed_hash(), commit.id.to_bytes());
        assert_eq!(record.retained_copy(), p.src.retained_copy());
        assert!(bytes.windows(commit.as_json().len()).any(|w| w == commit.as_json().as_bytes()), "not the exact Commit bytes");
        assert!(!p.relay.stored().iter().any(|e| e.id == commit.id), "the Commit escaped before retirement was published");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = crate::handover::live::authority::dir(&world).join(format!("retired-{}.dat", hex::encode(t)));
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(
            crate::handover::live::authority::open_guard(&world).unwrap_err().0,
            crate::handover::live::copy::RETIRED,
            "the retired world still opens"
        );
        // No abort after Retired: Cancel, a late Abort and every deadline.
        p.src.abort(&mut p.hs, "cancel");
        let dst = p.dst.as_mut().unwrap();
        dst.abort("late");
        assert!(matches!(dst.stage(), RecipientStage::Quarantined(_)), "the recipient's late failure isn't a quarantine");
        p.host.lend_tick(&mut p.hs);
        let late = now() + crate::handover::live::source::OPERATION_SECS + crate::handover::live::source::STALL_SECS + 1;
        let _ = p.src.pump(&mut p.hs, p.live, late);
        assert_eq!(p.src.stage(), SourceStage::Retired, "a cap or a late Abort aborted after Retired");
        assert!(p.hs.handover_frozen(), "a retired source resumed play");
        assert!(!p.src.needs_invite_rotation());
        // M-2: after Prepared the recipient deletes nothing.
        assert!(!import_dirs(&p).is_empty(), "the recipient deleted an import after Prepared");
        assert!(has_record(&p.dir, "prepared", t));
        assert!(has_record(&p.dir, "ready", t));
    }

    /// P2: a bearer that doesn't derive the key Prepared is sealed to stops
    /// the recipient with a typed reason while the import is still inert
    /// (discarded, no Prepared); the source, which disclosed its bearer,
    /// resumes and must rotate its invitation.
    #[test]
    fn handover_wrong_bearer_is_refused_and_a_disclosed_abort_rotates_the_invite() {
        let mut p = ready_pair("wrong-bearer");
        *p.forged_bearer.lock().unwrap() = Some([0x11; 16]);
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Retired);
        let dst = p.dst.as_ref().unwrap();
        assert_eq!(dst.stage(), RecipientStage::Aborted(crate::handover::live::copy::WRONG_BEARER), "a wrong bearer was accepted");
        assert!(!has_record(&p.dir, "prepared", t), "a wrong bearer reached Prepared");
        assert_eq!(import_dirs(&p), Vec::<String>::new(), "the inert import was kept");
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)), "the source didn't hear the abort");
        assert!(!p.hs.handover_frozen(), "the source didn't resume");
        assert!(p.src.bearer_disclosed());
        assert!(p.src.needs_invite_rotation(), "an abort after disclosure kept the old bearer");
        assert!(crate::handover::live::authority::open_guard(&p.dir.0.join("world")).is_ok(), "a pre-retirement abort retired the world");
    }

    /// CP7 review HIGH-1: once retired, nothing on the source's side
    /// unfreezes the world. The session is dropped (as leaving the world
    /// drops it); after several server ticks the world is still frozen, the
    /// recipient heard no Abort, and the open guard still says retired.
    #[test]
    fn handover_retired_world_stays_frozen_after_its_session_is_dropped() {
        let mut p = ready_pair("retired-drop");
        drive(&mut p, SourceStage::Retired);
        assert_eq!(p.src.stage(), SourceStage::Retired);
        let Pair { mut hs, mut host, src, mut dst, dir, .. } = p;
        drop(src);
        for _ in 0..5 {
            host.lend_tick(&mut hs);
        }
        let dst = dst.as_mut().unwrap();
        let _ = dst.pump(now());
        assert!(hs.handover_frozen(), "a dropped retired session unfroze the world");
        hs.handover_release();
        assert!(hs.handover_frozen(), "a release unfroze a retired world");
        assert_eq!(dst.stage(), RecipientStage::Prepared, "the recipient heard an Abort after Retired");
        assert_eq!(
            crate::handover::live::authority::open_guard(&dir.0.join("world")).unwrap_err().0,
            crate::handover::live::copy::RETIRED
        );
    }

    /// Every file under `dir` (relative path -> bytes); folders aren't files.
    fn files(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut out = std::collections::BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.insert(path.strip_prefix(dir).unwrap().display().to_string(), std::fs::read(&path).unwrap());
                }
            }
        }
        out
    }
    /// The fixture's world folder (`<dir>/world`) as a real saved world.
    fn seed_world(p: &Pair) {
        crate::save::with_worlds_root(&p.dir.0, || {
            crate::save::write_world_folder(
                "world",
                &crate::save::WorldMeta::new("world"),
                &crate::save::minimal_world_save_for_tests(77),
                &crate::world::World::new(),
            )
        })
        .unwrap();
    }
    /// The frame's five-minute autosave of the fixture's world.
    fn autosave(root: &std::path::Path) -> Result<(), String> {
        crate::save::with_worlds_root(root, || {
            crate::save::autosave_world("world", &crate::world::World::new(), std::slice::from_ref(&seat0()), 77, &[], &[])
        })
    }

    /// CP14 (CP9 step 4): a save queued before the freeze whose commit comes
    /// only after retirement is discarded at its commit: the retirement
    /// record is byte-identical, the world's files are unchanged since the
    /// capture, and the world still refuses to open.
    #[test]
    fn handover_late_save_queued_before_the_freeze_never_lands_after_retirement() {
        let mut p = pair("late-save");
        seed_world(&p);
        let world = p.dir.0.join("world");
        let before = files(&world);
        assert!(before.contains_key("world.dat"));
        // Queue the save: it runs up to its commit point and waits there.
        let hold = crate::save_fence::hold::hold(&world);
        let root = p.dir.0.clone();
        let late = std::thread::spawn(move || autosave(&root));
        assert!(hold.wait_arrived(), "the save never reached its commit");
        // Freeze (the capture), then retire.
        consent(&mut p);
        offer(&mut p, &seat0());
        stream(&mut p, 10_000);
        drive(&mut p, SourceStage::Retired);
        assert_eq!(p.src.stage(), SourceStage::Retired);
        let t = p.src.transfer();
        let record = crate::handover::live::authority::dir(&world).join(format!("retired-{}.dat", hex::encode(t)));
        let retired = std::fs::read(&record).unwrap();
        let records = files(&crate::handover::live::authority::dir(&world));
        // Now let the save complete.
        hold.release();
        assert_eq!(
            late.join().unwrap(),
            Err(crate::save_fence::DISCARDED.to_string()),
            "a save queued before the freeze committed after retirement"
        );
        assert_eq!(std::fs::read(&record).unwrap(), retired, "the retirement record changed");
        assert_eq!(files(&crate::handover::live::authority::dir(&world)), records);
        let mut now_files = files(&world);
        now_files.retain(|k, _| !k.starts_with("handover"));
        assert_eq!(now_files, before, "a late save changed the world after the capture");
        assert_eq!(
            crate::handover::live::authority::open_guard(&world).unwrap_err().0,
            crate::handover::live::copy::RETIRED,
            "the retired world opens"
        );
        assert!(crate::save::with_worlds_root(&p.dir.0, || crate::save::world_open_refusal("world")).is_some());
        // And it stays fenced: a later save is refused too.
        assert!(autosave(&p.dir.0).is_err());
    }

    /// CP14: the fence lifts only on an abort that unfreezes. While frozen a
    /// save is discarded and changes nothing; after the abort a save lands.
    #[test]
    fn handover_abort_that_unfreezes_lifts_the_late_save_fence() {
        let mut p = pair("fence-abort");
        seed_world(&p);
        let world = p.dir.0.join("world");
        consent(&mut p);
        assert!(crate::save_fence::is_up(&world), "the freeze raised no fence");
        let before = files(&world);
        assert_eq!(autosave(&p.dir.0).unwrap_err(), crate::save_fence::DISCARDED);
        assert_eq!(files(&world), before, "a frozen world was written");
        p.src.abort(&mut p.hs, "cancel");
        assert!(!p.hs.handover_frozen());
        assert!(!crate::save_fence::is_up(&world), "an abort that unfroze left the fence up");
        autosave(&p.dir.0).expect("a save after the unfreeze lands");
        assert!(world.join("autosave").join("world.dat").is_file());
    }

    /// M-2: an acknowledgement the recipient's runtime didn't sign is never
    /// enough to retire: the source aborts (and rotates), nothing retired.
    #[test]
    fn handover_forged_prepared_ack_never_retires() {
        let mut p = ready_pair("forged-ack");
        p.forge_ack.store(true, std::sync::atomic::Ordering::SeqCst);
        drive(&mut p, SourceStage::Retired);
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)), "a forged acknowledgement retired the source");
        assert!(crate::handover::live::authority::open_guard(&p.dir.0.join("world")).is_ok(), "retired without a verified ack");
        assert!(p.src.needs_invite_rotation());
        assert!(!p.hs.handover_frozen());
        // CP10: the source's authenticated pre-retirement Abort reaches the
        // recipient past its ack: quarantined, and the durable quarantine
        // record ends the wait for Committed (no resume at start-up).
        let t = p.src.transfer();
        p.host.lend_tick(&mut p.hs);
        let dst = p.dst.as_mut().unwrap();
        let _ = dst.pump(now());
        assert_eq!(dst.stage(), RecipientStage::Quarantined("The host cancelled the handover."));
        assert!(has_record(&p.dir, "quarantined", t), "no quarantine record after the source's Abort");
        // CP13b (D2): the live Abort carried the signed Aborted, checked by
        // the one shared verification, and ends like the published one.
        assert!(!has_record(&p.dir, "prepared", t), "the live Aborted left the receipt (and its bearer)");
        assert!(p.src.abort_recorded(), "the source's abort after disclosure wrote no abort record");
        assert_eq!(
            crate::handover::live::resumer::Resumer::new(FAST).resume_imports(&p.dir.0.join("imports"), dst_install()),
            0,
            "a quarantined import went back to waiting for Committed"
        );
    }

    /// CP8 review LOW-2: an ack the recipient's own runtime signed, whose
    /// possession proof is wrong, never retires: the source aborts, rotates
    /// the invitation and resumes play, and the world is not retired.
    fn wrong_proof_never_retires(tag: &str, forge: AckForge) {
        let mut p = ready_pair(tag);
        *p.ack_forge.lock().unwrap() = Some((forge, p.root.clone()));
        drive(&mut p, SourceStage::Retired);
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)), "{forge:?}: a wrong possession proof was accepted ({:?})", p.src.stage());
        assert!(p.src.bearer_disclosed() && p.src.needs_invite_rotation(), "{forge:?}: the invitation wasn't rotated");
        assert!(!p.hs.handover_frozen(), "{forge:?}: the source didn't resume");
        assert!(crate::handover::live::authority::open_guard(&p.dir.0.join("world")).is_ok(), "{forge:?}: the world was retired");
        assert!(p.src.commit_event().is_none(), "{forge:?}: a Commit was sealed");
    }
    #[test]
    fn handover_prepared_ack_with_the_v1_capability_never_retires() {
        wrong_proof_never_retires("ack-v1", AckForge::V1Capability);
    }
    #[test]
    fn handover_prepared_ack_with_another_bearers_proof_never_retires() {
        wrong_proof_never_retires("ack-bearer", AckForge::OtherBearer);
    }
    #[test]
    fn handover_prepared_ack_with_a_proof_over_another_prepared_never_retires() {
        wrong_proof_never_retires("ack-prepared", AckForge::OtherPrepared);
    }

    /// CP6 review LOW-4: a pack that took longer than the stall deadline
    /// doesn't count against the recipient: its clock starts at Streaming.
    #[test]
    fn handover_streaming_stall_clock_starts_when_streaming_starts() {
        let mut p = pair("stream-clock");
        consent(&mut p);
        let (reached, release) = p.src.hold_pack();
        start_offer(&mut p, &seat0());
        reached.recv().unwrap();
        release.send(()).unwrap();
        let later = now() + crate::handover::live::source::STALL_SECS + 5;
        p.src.settle_pack(&mut p.hs, p.live, later).unwrap();
        assert_eq!(p.src.stage(), SourceStage::Streaming);
        assert!(p.src.pump(&mut p.hs, p.live, later).is_ok(), "a long pack counted against the recipient");
        assert_eq!(p.src.stage(), SourceStage::Streaming, "the source stalled on its own packing time");
    }

    // ── CP9: the Committed publication outlives the session ─────────────

    /// A source past Retired whose first run is gone entirely (session,
    /// server, joined link and recipient dropped), so only the world folder
    /// and the relay remain.
    fn retired_and_dropped(tag: &str) -> (crate::handover::Id, String, crate::handover::relay_fixture::Relay, Dir) {
        let mut p = ready_pair(tag);
        drive(&mut p, SourceStage::Retired);
        assert_eq!(p.src.stage(), SourceStage::Retired);
        let (t, commit) = (p.src.transfer(), p.src.commit_event().expect("no Commit").as_json());
        let Pair { hs, host, src, rc, dst, relay, dir, .. } = p;
        drop((src, dst, rc, hs, host));
        assert!(!relay.stored().iter().any(|e| e.as_json() == commit), "the Commit escaped before the restart");
        (t, commit, relay, dir)
    }
    fn wait_published(store: &MarkerStore, retired: &crate::handover::phase::RetiredCommit) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !retired.is_published(store) {
            assert!(std::time::Instant::now() < deadline, "the Committed published record never landed");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    const FAST: crate::handover::live::resumer::Backoff = crate::handover::live::resumer::Backoff {
        first: std::time::Duration::from_millis(100),
        ceiling: std::time::Duration::from_millis(400),
    };

    /// A source restarted between Retired and the Committed proof: a fresh
    /// resumer's start-up scan over the same worlds folder finds the
    /// retirement record, publishes its exact Commit bytes (witnessed on
    /// the loopback relay), writes the durable "Committed published" record
    /// (0600), and the world stays refused. A later start-up has nothing to do.
    #[test]
    fn handover_restart_between_retired_and_the_proof_resumes_the_exact_commit() {
        use crate::handover::live::resumer::Resumer;
        let (t, commit, relay, dir) = retired_and_dropped("restart-commit");
        let world = dir.0.join("world");
        let store = crate::handover::live::authority::store(&world).unwrap();
        let retired = crate::handover::phase::RetiredCommit::read(&store, t).unwrap().expect("no retirement record");
        assert_eq!(retired.exact_bytes(), commit.as_bytes(), "the record doesn't hold the exact Commit bytes");
        assert!(!retired.is_published(&store));
        let resumer = Resumer::new(FAST);
        assert_eq!(resumer.resume_all(&dir.0, src_install()), 1, "the start-up scan didn't resume the Committed publication");
        wait_published(&store, &retired);
        let stored: Vec<_> = relay.stored().into_iter().filter(|e| e.id.to_bytes() == retired.event_hash()).collect();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].as_json().as_bytes(), retired.exact_bytes(), "not the exact record bytes");
        assert_eq!(
            crate::handover::live::authority::open_guard(&world).unwrap_err().0,
            crate::handover::live::copy::RETIRED,
            "a published retirement reopened the world"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = crate::handover::live::authority::dir(&world).join(format!("published-{}.dat", hex::encode(t)));
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while resumer.holds(t) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!resumer.holds(t), "the resumer kept a proven world");
        assert_eq!(Resumer::new(FAST).resume_all(&dir.0, src_install()), 0, "a published world was resumed again");
    }

    /// CP14b (CP10 review LOW-8, coordinator): the start-up scan publishes
    /// no retirement record until it has been re-persisted (file and
    /// directory fsynced). A re-persist that fails takes nothing over and
    /// nothing reaches the relay; the next start-up re-persists, then
    /// publishes the exact Commit.
    #[test]
    fn handover_start_up_scan_publishes_no_retirement_before_re_persisting_it() {
        use crate::handover::durable::{MarkerKind, WriteBoundary, fault};
        use crate::handover::live::resumer::Resumer;
        let (t, _, relay, dir) = retired_and_dropped("scan-durable");
        let world = dir.0.join("world");
        let store = crate::handover::live::authority::store(&world).unwrap();
        let retired = crate::handover::phase::RetiredCommit::read(&store, t).unwrap().expect("no retirement record");
        let on_relay = |r: &crate::handover::relay_fixture::Relay| {
            r.stored().into_iter().filter(|e| e.id.to_bytes() == retired.event_hash()).count()
        };
        // The directory fsync of the identical re-persist fails.
        fault::arm(MarkerKind::Retired, WriteBoundary::DirectorySynced);
        let resumer = Resumer::new(FAST);
        assert_eq!(
            resumer.resume_all(&dir.0, src_install()),
            0,
            "the start-up scan took a retirement record over before re-persisting it"
        );
        assert!(!resumer.holds(t));
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(on_relay(&relay), 0, "a Commit was published from a record not made durable");
        assert!(!retired.is_published(&store));
        // The next start-up re-persists it and publishes.
        assert_eq!(resumer.resume_all(&dir.0, src_install()), 1);
        wait_published(&store, &retired);
        assert_eq!(on_relay(&relay), 1);
    }

    /// Bounded backoff that never gives up: while the relay withholds what
    /// it stores, attempts fail (no witness); once it serves again a later
    /// attempt proves it. Every attempt had a fresh Budget (nothing received
    /// at its start, a deadline of its own) and waited out the backoff.
    #[test]
    fn handover_committed_retry_backs_off_with_a_fresh_budget_each_attempt() {
        use crate::handover::live::resumer::Resumer;
        let (t, _, relay, dir) = retired_and_dropped("commit-backoff");
        relay.withhold(true);
        let world = dir.0.join("world");
        let store = crate::handover::live::authority::store(&world).unwrap();
        let retired = crate::handover::phase::RetiredCommit::read(&store, t).unwrap().unwrap();
        let resumer = Resumer::new(FAST);
        assert_eq!(resumer.resume_all(&dir.0, src_install()), 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while resumer.attempts().len() < 2 {
            assert!(std::time::Instant::now() < deadline, "no retries");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!retired.is_published(&store), "published without a witness");
        assert!(resumer.holds(t), "the resumer gave up");
        relay.withhold(false);
        wait_published(&store, &retired);
        let a = resumer.attempts();
        assert!(a.len() >= 3, "{a:?}");
        assert!(!a[0].proven && !a[1].proven, "a withheld relay proved it");
        assert!(a.last().unwrap().proven && a[..a.len() - 1].iter().all(|x| !x.proven));
        for (i, x) in a.iter().enumerate() {
            assert_eq!(x.transfer, t);
            assert_eq!(x.budget_bytes_at_start, 0, "attempt {i} reused a Budget");
            assert!(x.budget_deadline.into_std() >= x.started + std::time::Duration::from_secs(24), "attempt {i} reused a Budget");
            if i > 0 {
                assert!(x.budget_deadline > a[i - 1].budget_deadline, "attempt {i} reused a Budget");
                let gap = x.started.duration_since(a[i - 1].started);
                assert!(gap >= FAST.delay(i as u32), "attempt {i} didn't back off: {gap:?}");
            }
        }
        assert!(relay.stored().iter().any(|e| e.as_json().as_bytes() == retired.exact_bytes()));
    }

    /// CP8 review LOW-3: a retirement write that fails in a way that may
    /// have reached the disk (`retire` returns Err, `RetirementUncertain`)
    /// still marks the link retired: the session dropped and an explicit
    /// release leave the world frozen, and no Abort reaches the recipient.
    #[test]
    fn handover_retirement_uncertain_marks_the_link_retired() {
        let mut p = ready_pair("retire-uncertain");
        let t = p.src.transfer();
        // A stale temporary record with other bytes: the durable write
        // refuses after the phase model entered RetirementUncertain.
        let world = p.dir.0.join("world");
        crate::handover::live::authority::store(&world).unwrap();
        // CP9 review LOW-3: a stale temp FILE no longer jams a write; a temp
        // path that isn't a regular file still refuses.
        std::fs::create_dir_all(crate::handover::live::authority::dir(&world).join(format!("retired.tmp-{}", hex::encode(t)))).unwrap();
        drive(&mut p, SourceStage::RetirementUncertain);
        assert_eq!(p.src.stage(), SourceStage::RetirementUncertain, "the failed retirement wasn't uncertain");
        assert!(p.src.past_retirement());
        let Pair { mut hs, mut host, src, mut dst, .. } = p;
        drop(src);
        for _ in 0..5 {
            host.lend_tick(&mut hs);
        }
        hs.handover_release();
        assert!(hs.handover_frozen(), "an uncertain retirement unfroze the world");
        let dst = dst.as_mut().unwrap();
        let _ = dst.pump(now());
        assert_eq!(dst.stage(), RecipientStage::Prepared, "the recipient heard an Abort after an uncertain retirement");
    }
    /// CP10 (HIGH-2): the recipient acknowledges only once the bearer is
    /// durable with its Prepared receipt. A Prepared write that fails (a
    /// stale temporary record with other bytes) sends no ack: the source
    /// never retires, no Commit is sealed, and the recipient aborts while
    /// still inert (nothing kept).
    #[test]
    fn handover_recipient_acks_only_after_its_bearer_is_durable() {
        let mut p = ready_pair("ack-durable");
        let t = p.src.transfer();
        let records = crate::handover::live::authority::dir(&p.dst.as_ref().unwrap().checkpoint().unwrap().working_world());
        std::fs::create_dir_all(records.join(format!("prepared.tmp-{}", hex::encode(t)))).unwrap();
        drive(&mut p, SourceStage::Retired);
        assert!(!p.src.past_retirement(), "the source retired without a durable bearer on the recipient");
        assert!(p.src.commit_event().is_none(), "a Commit was sealed without a durable bearer on the recipient");
        assert!(matches!(p.dst.as_ref().unwrap().stage(), RecipientStage::Aborted(_)), "{:?}", p.dst.as_ref().unwrap().stage());
        assert!(!has_record(&p.dir, "prepared", t));
        assert_eq!(import_dirs(&p), Vec::<String>::new(), "an inert import was kept");
    }

    /// Every file under `root` (recursively) except those under `skip`.
    fn files_under(root: &std::path::Path, skip: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let path = e.path();
                if path.starts_with(skip) {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out
    }
    fn holds(bytes: &[u8], needle: &[u8; 16]) -> bool {
        bytes.windows(16).any(|w| w == needle)
    }

    /// CP10 (HIGH-2): a recipient restarted between its ack and the
    /// Committed proof ends Activated. Past the ack the stall and the cap
    /// end nothing and the import is refused (open guard and every save
    /// path). Then everything is dropped (session, server, link): the
    /// source's start-up scan publishes Committed, the recipient's start-up
    /// scan finds the Prepared receipt (with the bearer) in the import's own
    /// working world `handover/`, opens and witnesses the Committed, and
    /// writes the Activated record (with the bearer, 0600). Only then does
    /// the import open; a later start-up has nothing to resume.
    #[test]
    fn handover_recipient_restart_between_the_ack_and_the_committed_proof_ends_activated() {
        use crate::handover::live::{authority, resumer::Resumer};
        let mut p = ready_pair("recipient-restart");
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Retired);
        let working = p.dst.as_ref().unwrap().checkpoint().unwrap().working_world();
        let dst = p.dst.as_mut().unwrap();
        let late = now() + crate::handover::live::source::OPERATION_SECS + crate::handover::live::source::STALL_SECS + 1;
        dst.pump(late).unwrap();
        assert_eq!(dst.stage(), RecipientStage::Prepared, "the stall or the cap ended the wait for Committed");
        let prepared = std::fs::read(authority::dir(&working).join(format!("prepared-{}.dat", hex::encode(t)))).unwrap();
        assert!(holds(&prepared, &TEST_BEARER), "the Prepared receipt doesn't hold the bearer");
        assert_eq!(authority::open_guard(&working).unwrap_err().0, crate::handover::live::copy::NOT_ACTIVATED, "the import opened before Activated");
        assert!(crate::save::save_refusal_in(&working).is_some(), "a save path accepts the import before Activated");
        let Pair { hs, host, src, rc, dst, relay, dir, .. } = p;
        drop((src, dst, rc, hs, host));
        // CP13 (CP10 review MEDIUM-2): the runtime key file is damaged before
        // the next launch and re-mints; the ledger, not the key, says which
        // records are this install's, so both still resume.
        let key_file = dir.0.join("runtime_key.json");
        std::fs::write(&key_file, b"{ damaged").unwrap();
        let reminted = crate::runtime_identity::load_or_mint_runtime(&key_file).unwrap().public_key();
        assert!(reminted != keys(SOURCE_RUNTIME).public_key() && reminted != keys(RECIPIENT_RUNTIME).public_key());
        assert!(Resumer::new(FAST).resume_all(&dir.0, src_install()) == 1, "the source's Committed wasn't resumed");
        // CP11 (CP10 review HIGH-1): the recipient restarts two days later,
        // past the certificate (<= 1 h), the root (1 h) and both
        // attestations (1 day): it still activates.
        let recipient = Resumer::with_clock(FAST, || now() + 2 * 86_400);
        assert_eq!(recipient.resume_imports(&dir.0.join("imports"), dst_install()), 1, "the start-up scan didn't resume the wait for Committed");
        let store = authority::store(&working).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while store.read(MarkerKind::Activated, t).unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "the recipient never activated");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(relay.stored().len() >= 2, "Committed wasn't on the relay");
        let r = crate::handover::phase::Receiver::restart(&store, t).unwrap().unwrap();
        assert!(r.gameplay_admitted(), "the restarted receiver isn't activated");
        let activated = std::fs::read(authority::dir(&working).join(format!("activated-{}.dat", hex::encode(t)))).unwrap();
        assert!(holds(&activated, &TEST_BEARER), "the Activated record doesn't carry the bearer");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for kind in ["prepared", "activated"] {
                let path = authority::dir(&working).join(format!("{kind}-{}.dat", hex::encode(t)));
                assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600, "{kind} isn't 0600");
            }
        }
        assert!(authority::open_guard(&working).is_ok(), "an Activated import is still refused");
        assert!(crate::handover::phase::AwaitingCommit::read(&store, t).unwrap().is_none());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while recipient.holds_import(t) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!recipient.holds_import(t), "the resumer kept an activated import");
        assert_eq!(Resumer::new(FAST).resume_imports(&dir.0.join("imports"), dst_install()), 0, "an activated import was resumed again");
    }

    /// CP10 (HIGH-2 + H3): the bearer is stored only in the imported world's
    /// `handover/` records. A source world that itself holds a received
    /// handover's records (another bearer) hands over: neither that bearer
    /// nor the recipient's own one reaches the archive the recipient got
    /// (decompressed), its received or working copy, or anything else in
    /// the import outside `working/handover/`; the recipient's bearer IS in
    /// its Prepared receipt there. (The handover pack and the export archive
    /// are allowlists built from the world state, never a folder walk; the
    /// retained copy is not built yet.)
    #[test]
    fn handover_bearer_is_absent_from_every_archive_and_copy() {
        use crate::handover::live::authority;
        use std::io::Read;
        const OLD: [u8; 16] = [0xB7; 16];
        let mut p = ready_pair("bearer-archives");
        let t = p.src.transfer();
        let old = authority::store(&p.dir.0.join("world")).unwrap();
        let mut v = Vec::new();
        crate::handover::codec::frame(&mut v, b"AxeNStax/handover/activated/v2");
        v.extend_from_slice(&OLD);
        old.persist(MarkerKind::Prepared, [0x99; 16], &OLD).unwrap();
        old.persist(MarkerKind::Activated, [0x99; 16], &v).unwrap();
        drive(&mut p, SourceStage::Retired);
        assert_eq!(p.src.stage(), SourceStage::Retired);
        let cp = p.dst.as_ref().unwrap().checkpoint().unwrap();
        let records = authority::dir(&cp.working_world());
        let receipt = std::fs::read(records.join(format!("prepared-{}.dat", hex::encode(t)))).unwrap();
        assert!(holds(&receipt, &TEST_BEARER), "the positive control: the receipt holds the bearer");
        let mut checked = 0;
        for f in files_under(cp.directory(), &records) {
            let mut bytes = std::fs::read(&f).unwrap();
            if f.file_name().is_some_and(|n| n == "archive.tar.gz") {
                let mut plain = Vec::new();
                flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut plain).unwrap();
                bytes = plain;
            }
            assert!(!holds(&bytes, &TEST_BEARER), "the bearer reached {}", f.display());
            assert!(!holds(&bytes, &OLD), "a received world's bearer reached {}", f.display());
            checked += 1;
        }
        assert!(checked >= 4, "only {checked} files checked");
    }
    /// CP9 review MEDIUM-1: a relay that never answers (no OK, no EOSE) in
    /// the root's first place can't hold the Committed: with its own share
    /// of the attempt and the start rotating, the healthy second relay
    /// proves it within two attempts.
    #[test]
    fn handover_black_holed_first_relay_cant_hold_the_committed() {
        use crate::handover::live::resumer::Resumer;
        let mut p = pair_n("black-hole", 2);
        consent(&mut p);
        offer(&mut p, &seat0());
        stream(&mut p, 10_000);
        drive(&mut p, SourceStage::Retired);
        assert_eq!(p.src.stage(), SourceStage::Retired);
        let t = p.src.transfer();
        let first = p.root.relays()[0].clone();
        let Pair { hs, host, src, rc, dst, relay, extra, dir, .. } = p;
        drop((src, dst, rc, hs, host));
        let all: Vec<_> = std::iter::once(&relay).chain(extra.iter()).collect();
        let hole = all.iter().find(|r| r.url() == first).expect("the first relay");
        hole.black_hole(true);
        let world = dir.0.join("world");
        let store = crate::handover::live::authority::store(&world).unwrap();
        let retired = crate::handover::phase::RetiredCommit::read(&store, t).unwrap().unwrap();
        let resumer = Resumer::new(FAST);
        assert_eq!(resumer.resume_all(&dir.0, src_install()), 1);
        wait_published(&store, &retired);
        let a: Vec<_> = resumer.attempts().into_iter().filter(|a| !a.import).collect();
        assert!(a.len() <= 2, "Committed took {} attempts: {a:?}", a.len());
        assert!(a.last().unwrap().proven);
        assert!(all.iter().any(|r| r.url() != first && r.stored().iter().any(|e| e.id.to_bytes() == retired.event_hash())));
    }

    /// CP9 review MEDIUM-2: the exit grace wakes a publication sleeping out
    /// its backoff so it attempts at once, and never blocks past the grace;
    /// once the relay serves, the grace returns as soon as it is done.
    #[test]
    fn handover_exit_grace_attempts_at_once_and_never_blocks_longer() {
        use crate::handover::live::resumer::{Backoff, Resumer};
        let (t, _, relay, dir) = retired_and_dropped("exit-grace");
        relay.withhold(true);
        let slow = Backoff { first: std::time::Duration::from_secs(600), ceiling: std::time::Duration::from_secs(600) };
        let resumer = Resumer::new(slow);
        assert_eq!(resumer.resume_all(&dir.0, src_install()), 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while resumer.attempts().is_empty() {
            assert!(std::time::Instant::now() < deadline, "no first attempt");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let start = std::time::Instant::now();
        assert!(!resumer.finish_within(std::time::Duration::from_secs(2)), "a withheld Committed finished");
        assert!(start.elapsed() < std::time::Duration::from_secs(3), "the grace blocked past its bound");
        assert!(resumer.attempts().len() >= 2, "the grace didn't wake the sleeping publication");
        relay.withhold(false);
        let start = std::time::Instant::now();
        assert!(resumer.finish_within(std::time::Duration::from_secs(10)), "the grace didn't let it finish");
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        assert!(!resumer.holds(t));
    }

    /// CP9 review LOW-6 + LOW-5 + LOW-7: the real boot step takes only this
    /// install's records. A record signed by another runtime, and a v1 or
    /// tampered-relay record, never contact a relay and stay refused; this
    /// install's record is resumed by the same call `native_boot_services`
    /// makes.
    #[test]
    fn handover_boot_resumes_only_records_this_installs_ledger_lists() {
        use crate::handover::live::{authority, copy, resumer::Resumer};
        let (t, _, relay, dir) = retired_and_dropped("boot-planted");
        let bytes = authority::store(&dir.0.join("world")).unwrap().read(MarkerKind::Retired, t).unwrap().unwrap();
        assert!(crate::handover::phase::Retirement::decode(&bytes).is_ok());
        let mut v1 = bytes.clone();
        let at = v1.windows(13).position(|w| w == b"retirement/v2").unwrap();
        v1[at + 12] = b'1';
        let url = relay.url();
        let port = url.rsplit(':').next().unwrap().to_owned();
        let tampered = {
            let mut b = bytes.clone();
            let at = b.windows(port.len()).position(|w| w == port.as_bytes()).unwrap();
            b[at] = if b[at] == b'1' { b'2' } else { b'1' };
            b
        };
        let planted = dir.0.join("planted");
        for (name, record) in [("v1", &v1), ("tampered", &tampered)] {
            assert!(crate::handover::phase::Retirement::decode(record).is_err(), "a {name} record decoded");
            let w = planted.join(name);
            authority::store(&w).unwrap().persist(MarkerKind::Retired, t, record).unwrap();
            assert_eq!(authority::open_guard(&w).unwrap_err().0, copy::AUTHORITY_UNREADABLE, "a {name} record isn't refused");
        }
        // CP13 (CP10 review LOW-1): the planted record that DOES decode: this
        // install's own real signed Commit, `core.context` recomputed for an
        // attacker's relays, in a world folder the player unzipped.
        let attacker = crate::handover::relay_fixture::Relay::start();
        let low1 = crate::handover::phase::Retirement::replanted(&bytes, &[attacker.url()]);
        assert!(crate::handover::phase::Retirement::decode(&low1).is_ok(), "the LOW-1 plant doesn't decode");
        let unzipped = dir.0.join("unzipped");
        authority::store(&unzipped.join("Gift")).unwrap().persist(MarkerKind::Retired, t, &low1).unwrap();
        let received = relay.received();
        // Another install (an empty ledger): nothing is published, nothing
        // contacts a relay. (This process's recipient receipt is listed in
        // the same test ledger, so the source-side scans here pass no imports.)
        let other = Resumer::new(FAST);
        let elsewhere = crate::handover::ledger::Ledger::at(dir.0.join("another-install").join("ledger"));
        assert_eq!(other.boot(&dir.0, &dir.0.join("no-imports"), &elsewhere), (0, 0), "another install's record was resumed");
        assert_eq!(other.boot(&planted, &dir.0.join("no-imports"), src_install()), (0, 0), "an undecodable record was resumed");
        assert_eq!(other.boot(&unzipped, &dir.0.join("no-imports"), src_install()), (0, 0), "the planted record was resumed");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(relay.received(), received, "a planted record contacted a relay");
        assert_eq!(attacker.received(), 0, "the planted record reached the attacker's relay");
        assert!(authority::open_guard(&unzipped.join("Gift")).is_err(), "the planted world opened");
        assert_eq!(
            crate::handover::live::resumer::awaiting_statuses(&unzipped, &dir.0.join("no-imports"), src_install()),
            vec![copy::unlisted_world("Gift")],
            "the planted record has no status line"
        );
        assert!(
            crate::handover::live::resumer::awaiting_statuses(&dir.0, &dir.0.join("no-imports"), src_install()).is_empty(),
            "this install's own retirement shows as unlisted"
        );
        // This install: the boot step publishes it.
        let mine = Resumer::new(FAST);
        assert_eq!(mine.boot(&dir.0, &dir.0.join("no-imports"), src_install()), (1, 0));
        let store = authority::store(&dir.0.join("world")).unwrap();
        wait_published(&store, &crate::handover::phase::RetiredCommit::read(&store, t).unwrap().unwrap());
        // The boot call is the one native_boot_services makes.
        let lib = include_str!("../lib.rs");
        let boot = lib.find("fn native_boot_services").unwrap();
        assert!(lib[boot..boot + 2500].contains("Resumer::global().boot("), "native_boot_services doesn't run the handover boot step");
    }

    /// CP13 (D2, CP10 review MEDIUM-3 (A)): the receipt persisted, then the
    /// inert clear failed, so the ack never left. The recipient quarantines
    /// locally (record first, then the receipt and its bearer go), the
    /// source (no ack) aborts and keeps its world, no resumer adopts the
    /// import, and the world list says the handover didn't finish.
    #[test]
    fn handover_a_receipt_with_no_ack_is_quarantined_locally_and_never_adopted() {
        use crate::handover::live::{copy, resumer::{Resumer, awaiting_statuses}};
        let mut p = ready_pair("no-ack-quarantine");
        p.dst.as_mut().unwrap().test_fail_inert_clear = true;
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Retired);
        let dst = p.dst.as_ref().unwrap();
        assert!(matches!(dst.stage(), RecipientStage::Quarantined(_)), "the recipient ended {:?}", dst.stage());
        assert!(has_record(&p.dir, "quarantined", t), "no local quarantine record");
        assert!(!has_record(&p.dir, "prepared", t), "the receipt (and its bearer) stayed after the local quarantine");
        assert!(!dst.activation_handed(), "a receipt with no ack attempt was handed off");
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)), "the source retired without the ack ({:?})", p.src.stage());
        assert!(crate::handover::live::authority::open_guard(&p.dir.0.join("world")).is_ok(), "the source lost its world");
        assert_eq!(Resumer::new(FAST).resume_imports(&p.dir.0.join("imports"), dst_install()), 0, "a resumer adopted the quarantined import");
        assert!(
            awaiting_statuses(&p.dir.0.join("no-worlds"), &p.dir.0.join("imports"), dst_install()).contains(&copy::HANDOVER_UNFINISHED.to_owned()),
            "the quarantined import has no status line"
        );
    }

    /// CP13 (D2): a recipient Cancel after the ack sends no Abort and
    /// quarantines nothing; the wait stays adoptable and the import still
    /// activates on Committed.
    #[test]
    fn handover_a_recipient_cancel_after_the_ack_sends_no_abort_and_still_activates() {
        use crate::handover::live::resumer::Resumer;
        use crate::handover::live::wire::Frame;
        let mut p = ready_pair("cancel-after-ack");
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Retired);
        assert_eq!(p.src.stage(), SourceStage::Retired);
        let working = p.dst.as_ref().unwrap().checkpoint().unwrap().working_world();
        p.dst.as_mut().unwrap().abort("cancel");
        let frames = p.hs.handover_take_frames().unwrap_or_default();
        assert!(
            !frames.iter().any(|b| matches!(Frame::decode(b), Ok(Frame::Abort { .. }))),
            "a recipient Cancel after the ack sent Abort"
        );
        assert!(!has_record(&p.dir, "quarantined", t), "a recipient Cancel after the ack quarantined");
        assert!(p.src.hand_publication(&Resumer::new(FAST)), "the publication wasn't taken");
        let recipient = Resumer::new(FAST);
        assert_eq!(recipient.resume_imports(&p.dir.0.join("imports"), dst_install()), 1, "the wait wasn't adoptable after the Cancel");
        let store = crate::handover::live::authority::store(&working).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while store.read(MarkerKind::Activated, t).unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "the import never activated after the Cancel");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// CP13b (D2) (C)/(D): the source aborts after disclosing the bearer
    /// with no ack read (the ack was lost), so its abort record is durable
    /// before it unfreezes; a restarted source's start-up scan publishes the
    /// Aborted, and the recipient's resumer reads it back and quarantines
    /// (receipt and bearer gone, never adopted again).
    fn aborted_after_disclosure(tag: &str, dropped_link: bool) {
        use crate::handover::live::{authority, copy, resumer::{Resumer, awaiting_statuses}};
        let mut p = ready_pair(tag);
        p.drop_ack.store(true, std::sync::atomic::Ordering::SeqCst);
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Prepared);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !has_record(&p.dir, "prepared", t) {
            assert!(std::time::Instant::now() < deadline, "the recipient never persisted its receipt");
            p.host.lend_tick(&mut p.hs);
            let _ = p.dst.as_mut().unwrap().pump(now());
        }
        assert_eq!(p.dst.as_ref().unwrap().stage(), RecipientStage::Prepared);
        if dropped_link {
            // (D): the link drops after the (lost) ack.
            p.hs.test_close_seat_transport(p.seat.slot);
            let _ = p.src.pump(&mut p.hs, p.live, now());
        } else {
            // (C): the recipient crashed between its receipt and its ack.
            p.dst = None;
            let late = now() + crate::handover::live::source::STALL_SECS + 1;
            let _ = p.src.pump(&mut p.hs, p.live, late);
        }
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)), "the source didn't abort ({:?})", p.src.stage());
        assert!(p.src.abort_recorded(), "the abort after disclosure wrote no abort record");
        assert!(!p.hs.handover_frozen(), "the source didn't resume");
        let world = p.dir.0.join("world");
        assert!(authority::open_guard(&world).is_ok(), "an abort record refused the source's own world");
        let Pair { hs, host, src, rc, dst, relay, dir, .. } = p;
        drop((src, rc, hs, host));
        let src_store = authority::store(&world).unwrap();
        let aborted = crate::handover::phase::RetiredCommit::read_aborted(&src_store, t).unwrap().expect("no abort record");
        assert!(aborted.is_abort());
        assert_eq!(Resumer::new(FAST).resume_all(&dir.0, src_install()), 1, "the restarted source didn't resume the Aborted");
        wait_published(&src_store, &aborted);
        assert!(relay.stored().len() >= 2, "the Aborted wasn't on the relay");
        let recipient = Resumer::new(FAST);
        assert_eq!(recipient.resume_imports(&dir.0.join("imports"), dst_install()), 1, "the recipient's wait wasn't resumed");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !has_record(&dir, "quarantined", t) {
            assert!(std::time::Instant::now() < deadline, "the recipient never quarantined on the published Aborted");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!has_record(&dir, "prepared", t), "the published Aborted left the receipt (and its bearer)");
        assert!(!has_record(&dir, "activated", t));
        assert_eq!(Resumer::new(FAST).resume_imports(&dir.0.join("imports"), dst_install()), 0, "a quarantined import was resumed");
        assert!(
            awaiting_statuses(&dir.0.join("no-worlds"), &dir.0.join("imports"), dst_install()).contains(&copy::HANDOVER_UNFINISHED.to_owned()),
            "the quarantined import has no status line"
        );
        drop(dst);
    }
    #[test]
    fn handover_c_a_crash_between_the_receipt_and_the_ack_ends_on_the_published_aborted() {
        aborted_after_disclosure("aborted-c", false);
    }
    #[test]
    fn handover_d_a_dropped_link_after_the_ack_ends_on_the_published_aborted() {
        aborted_after_disclosure("aborted-d", true);
    }

    /// CP13b (D2): past the ack, a live Abort ends the wait only through the
    /// one shared verification. Carrying the source's own signed Committed
    /// (same author, transfer and kind; never readable as an Aborted) it
    /// quarantines nothing and the receipt stays.
    #[test]
    fn handover_a_live_abort_without_a_valid_aborted_never_quarantines() {
        let mut p = ready_pair("abort-not-aborted");
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Retired);
        let committed = p.src.commit_event().unwrap().as_json();
        let f = Frame::Abort { transfer: t, aborted: Some(committed) };
        p.hs.handover_send(&f.encode().unwrap()).unwrap();
        p.host.lend_tick(&mut p.hs);
        let _ = p.dst.as_mut().unwrap().pump(now());
        assert!(!has_record(&p.dir, "quarantined", t), "a live Abort carrying a Committed quarantined");
        assert!(has_record(&p.dir, "prepared", t), "a live Abort carrying a Committed removed the receipt");
    }

    /// CP13b (D2, CP12 review LOW-1): the abort record's write fails: the
    /// source still unfreezes, and its status line says so.
    #[test]
    fn handover_a_failed_abort_record_still_unfreezes_with_a_status_line() {
        let mut p = ready_pair("abort-unrecorded");
        p.src.test_fail_abort_record = true;
        p.forge_ack.store(true, std::sync::atomic::Ordering::SeqCst);
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Retired);
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)), "{:?}", p.src.stage());
        assert!(!p.hs.handover_frozen(), "a failed abort record kept the source frozen");
        assert!(p.src.abort_unrecorded() && !p.src.abort_recorded(), "the failed abort record isn't reported");
        let world = crate::handover::live::authority::dir(&p.dir.0.join("world"));
        assert!(!world.join(format!("aborted-{}.dat", hex::encode(t))).exists());
        // CP14b (CP13b review LOW-5): the line the session ends with (the
        // one `end_source_handover` returns, the leave path's notice
        // included) says the cancellation wasn't saved.
        let line = p.src.end_line(crate::handover::live::copy::LEFT);
        assert!(
            line.contains(crate::handover::live::copy::ABORT_UNRECORDED),
            "the end line doesn't say the cancellation couldn't be saved: {line}"
        );
    }

    /// CP14b (CP13b review LOW-3): a certain failure writing the retirement
    /// record (before its rename) is no retirement: the source goes back to
    /// Prepared and aborts with its abort record, unfreezes, the world
    /// opens, and the Aborted is published.
    #[test]
    fn handover_a_certain_failure_at_retire_aborts_and_the_world_opens() {
        use crate::handover::durable::{MarkerKind, WriteBoundary, fault};
        use crate::handover::live::{authority, resumer::Resumer};
        let mut p = ready_pair("retire-io");
        let t = p.src.transfer();
        fault::arm(MarkerKind::Retired, WriteBoundary::TempSynced);
        drive(&mut p, SourceStage::Retired);
        assert!(
            matches!(p.src.stage(), SourceStage::Aborted(_)),
            "a certain failure at retire didn't abort ({:?})",
            p.src.stage()
        );
        assert!(p.src.abort_recorded(), "no abort record after the failed retire");
        assert!(!p.hs.handover_frozen(), "the source stayed frozen");
        let world = p.dir.0.join("world");
        assert!(!authority::dir(&world).join(format!("retired-{}.dat", hex::encode(t))).exists());
        assert!(authority::open_guard(&world).is_ok(), "the world doesn't open after a failed retire");
        assert!(!crate::save_fence::is_up(&world), "the late-save fence stayed up");
        let resumer = Resumer::new(FAST);
        assert!(p.src.hand_publication(&resumer), "the Aborted wasn't handed over");
        let store = authority::store(&world).unwrap();
        let aborted = crate::handover::phase::RetiredCommit::read_aborted(&store, t).unwrap().expect("no abort record");
        wait_published(&store, &aborted);
    }

    /// CP14b (CP13b review LOW-1): a disclosed session dropped without
    /// `abort` hands its abort record to the process's resumer: the Aborted
    /// is published in this process (no restart, no scan), and the
    /// recipient, past its (lost) ack, quarantines on it.
    #[test]
    fn handover_a_dropped_disclosed_session_publishes_its_aborted_in_process() {
        use crate::handover::live::{authority, resumer::Resumer};
        let mut p = ready_pair("dropped-disclosed");
        p.drop_ack.store(true, std::sync::atomic::Ordering::SeqCst);
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Prepared);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !has_record(&p.dir, "prepared", t) {
            assert!(std::time::Instant::now() < deadline, "the recipient never persisted its receipt");
            p.host.lend_tick(&mut p.hs);
            let _ = p.dst.as_mut().unwrap().pump(now());
        }
        assert_eq!(p.src.stage(), SourceStage::Prepared);
        let Pair { hs, host, src, rc, dst, relay, dir, .. } = p;
        drop(src);
        let world = dir.0.join("world");
        let store = authority::store(&world).unwrap();
        let aborted = crate::handover::phase::RetiredCommit::read_aborted(&store, t)
            .unwrap()
            .expect("the dropped session wrote no abort record");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !aborted.is_published(&store) {
            assert!(std::time::Instant::now() < deadline, "the dropped session's Aborted wasn't published in this process");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let recipient = Resumer::new(FAST);
        assert_eq!(recipient.resume_imports(&dir.0.join("imports"), dst_install()), 1, "the recipient's wait wasn't resumed");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !has_record(&dir, "quarantined", t) {
            assert!(std::time::Instant::now() < deadline, "the recipient never quarantined on the published Aborted");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drop((rc, hs, host, dst, relay));
    }

    /// CP15: the source's world folder path of `t`'s `<label>` record.
    fn source_record(dir: &Dir, label: &str, t: crate::handover::Id) -> std::path::PathBuf {
        crate::handover::live::authority::dir(&dir.0.join("world")).join(format!("{label}-{}.dat", hex::encode(t)))
    }
    /// Drive to Prepared with the recipient's ack lost in transit, until
    /// the recipient's receipt is durable (it is past its ack).
    fn past_the_ack(tag: &str) -> Pair {
        let mut p = ready_pair(tag);
        p.drop_ack.store(true, std::sync::atomic::Ordering::SeqCst);
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Prepared);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !has_record(&p.dir, "prepared", t) {
            assert!(std::time::Instant::now() < deadline, "the recipient never persisted its receipt");
            p.host.lend_tick(&mut p.hs);
            let _ = p.dst.as_mut().unwrap().pump(now());
        }
        assert_eq!(p.src.stage(), SourceStage::Prepared);
        p
    }

    /// CP15 (a) (CP13b review LOW-3, coordinator): a source killed after the
    /// bearer left and before any terminal record (no `abort`, no `Drop`)
    /// leaves its pending abort record. The next start-up promotes it to
    /// the abort record through the choke point, publishes the Aborted on
    /// the relay fixture, and the recipient past its ack quarantines
    /// instead of waiting forever. The world opens, the pending record is
    /// gone, and nothing on disk holds the bearer (invitations are never
    /// persisted: the restarted host's invitation is a fresh one).
    #[test]
    fn handover_a_source_killed_after_disclosure_is_promoted_at_start_up() {
        use crate::handover::live::{authority, resumer::Resumer};
        let p = past_the_ack("pending-crash");
        let t = p.src.transfer();
        assert!(source_record(&p.dir, "pending-abort", t).is_file(), "no pending abort record before the bearer left");
        let Pair { hs, host, src, rc, dst, relay, dir, .. } = p;
        // The kill: no `abort`, no `Drop` backstop.
        std::mem::forget(src);
        drop((rc, hs, host));
        let world = dir.0.join("world");
        assert!(!source_record(&dir, "aborted", t).exists(), "the kill left an abort record");
        assert_eq!(Resumer::new(FAST).resume_all(&dir.0, src_install()), 1, "the restarted source didn't promote and publish the pending abort");
        assert!(!source_record(&dir, "pending-abort", t).exists(), "the promoted pending abort record stayed");
        let store = authority::store(&world).unwrap();
        let aborted = crate::handover::phase::RetiredCommit::read_aborted(&store, t).unwrap().expect("no abort record after the promotion");
        wait_published(&store, &aborted);
        assert!(relay.stored().iter().any(|e| e.id.to_bytes() == aborted.event_hash()), "the Aborted wasn't on the relay");
        let recipient = Resumer::new(FAST);
        assert_eq!(recipient.resume_imports(&dir.0.join("imports"), dst_install()), 1, "the recipient's wait wasn't resumed");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !has_record(&dir, "quarantined", t) {
            assert!(std::time::Instant::now() < deadline, "the recipient past its ack never quarantined");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!has_record(&dir, "prepared", t), "the published Aborted left the receipt");
        assert!(authority::open_guard(&world).is_ok(), "the promoted abort refused the source's world");
        let bearer = TEST_BEARER;
        for f in files_under(&world, &dir.0.join("imports")) {
            assert!(!holds(&std::fs::read(&f).unwrap(), &bearer), "{} holds the bearer", f.display());
        }
        drop((dst, relay));
    }

    /// CP15 (b): a pending abort record beside the transfer's retirement
    /// record: the retirement wins. The start-up publishes the Committed
    /// only, writes no abort record, and discards the pending record.
    #[test]
    fn handover_a_pending_abort_beside_a_retirement_record_loses_to_it() {
        use crate::handover::live::{authority, resumer::Resumer};
        let mut p = ready_pair("pending-retired");
        let t = p.src.transfer();
        drive(&mut p, SourceStage::Prepared);
        let pending = std::fs::read(source_record(&p.dir, "pending-abort", t)).expect("no pending abort record");
        drive(&mut p, SourceStage::Retired);
        assert_eq!(p.src.stage(), SourceStage::Retired);
        assert!(!source_record(&p.dir, "pending-abort", t).exists(), "a clean retire kept the pending abort record");
        let Pair { hs, host, src, rc, dst, relay, dir, .. } = p;
        drop((src, dst, rc, hs, host));
        // As a crash between the retirement and the removal would leave it.
        std::fs::write(source_record(&dir, "pending-abort", t), &pending).unwrap();
        let world = dir.0.join("world");
        assert_eq!(Resumer::new(FAST).resume_all(&dir.0, src_install()), 1, "the Committed wasn't resumed");
        let store = authority::store(&world).unwrap();
        let retired = crate::handover::phase::RetiredCommit::read(&store, t).unwrap().expect("no retirement record");
        wait_published(&store, &retired);
        assert!(!source_record(&dir, "aborted", t).exists(), "a pending abort beside a retirement became an abort record");
        assert!(!source_record(&dir, "pending-abort", t).exists(), "the losing pending abort record wasn't discarded");
        let payload = crate::handover::durable::marker_payload(&pending, crate::handover::durable::MarkerKind::PendingAbort, t).unwrap();
        let aborted = crate::handover::phase::AbortRecord::decode(&payload).unwrap().aborted_hash();
        let ids: Vec<_> = relay.stored().iter().map(|e| e.id.to_bytes()).collect();
        assert!(ids.contains(&retired.event_hash()));
        assert!(!ids.contains(&aborted), "the losing pending abort's Aborted was published");
        assert_eq!(authority::open_guard(&world).unwrap_err().0, crate::handover::live::copy::RETIRED);
    }

    /// CP15 (c): the pending abort record can't be written: the handover
    /// stops before anything leaves (no bearer frame, no invitation to
    /// rotate), the source unfreezes, and no record is left behind.
    #[test]
    fn handover_a_pending_abort_record_that_cant_be_written_discloses_nothing() {
        use crate::handover::durable::{MarkerKind, WriteBoundary, fault};
        use crate::handover::live::{authority, copy};
        let mut p = ready_pair("pending-unwritten");
        let t = p.src.transfer();
        fault::arm(MarkerKind::PendingAbort, WriteBoundary::TempSynced);
        let r = p.src.pump(&mut p.hs, p.live, now());
        assert_eq!(r.map_err(|e| e.0), Err(copy::PENDING_ABORT_UNWRITTEN), "the handover didn't stop on the unwritten pending abort");
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)), "{:?}", p.src.stage());
        assert!(!p.src.bearer_disclosed() && !p.src.needs_invite_rotation(), "the bearer was disclosed without a pending abort record");
        assert!(!p.hs.handover_frozen(), "the source stayed frozen");
        p.host.lend_tick(&mut p.hs);
        let _ = p.dst.as_mut().unwrap().pump(now());
        assert!(!frames_for_seat_hold_bearer(&p), "a bearer frame reached the recipient");
        assert!(!source_record(&p.dir, "pending-abort", t).exists() && !source_record(&p.dir, "aborted", t).exists());
        assert!(authority::open_guard(&p.dir.0.join("world")).is_ok());
    }
    fn frames_for_seat_hold_bearer(p: &Pair) -> bool {
        p.dst.as_ref().is_some_and(|d| d.bearer_received())
    }
}

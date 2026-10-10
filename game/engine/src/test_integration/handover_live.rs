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
    assert!(hs.handover_freeze().is_err());
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
    hs.handover_freeze().unwrap();
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
    hs.handover_freeze().unwrap();
    let junk = vec![0u8; 60 * 1024];
    let n = crate::transport::MAX_INBOUND_BYTES / junk.len() + 8;
    for _ in 0..n {
        guest.send_to_server(&junk);
    }
    hs.handover_take_frames().unwrap();
    assert!(hs.handover_test_inbound_bytes(guest_slot) > 0, "the guest's packets were left in its unbounded transport");
    let bound = crate::transport::MAX_INBOUND_BYTES + junk.len() + crate::transport::INBOUND_ENTRY_OVERHEAD;
    for _ in 0..n {
        guest.send_to_server(&junk);
    }
    for _ in 0..4 {
        hs.handover_take_frames().unwrap();
    }
    assert!(hs.handover_test_inbound_bytes(guest_slot) <= bound, "the guest's queue grew past its bound while frozen");
    assert!(hs.handover_seat(guest_slot).is_some(), "kicked while frozen");
    hs.handover_release();
    assert!(hs.handover_seat(guest_slot).is_none(), "an overflowed guest stayed at release");
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
    hs.handover_freeze().unwrap();
    let tally = host.world.sim_tally;
    host.lend_tick(&mut hs);
    assert!(hs.handover_frozen() && host.world.sim_tally == tally, "held lease: still frozen");
    let _ = frames_for(&client);
    drop(lease); // the session went away without an explicit abort
    host.lend_tick(&mut hs);
    assert!(!hs.handover_frozen() && hs.handover_link().is_none(), "a dropped session's link is released");
    let got = frames_for(&client);
    use crate::handover::live::wire::Frame;
    assert!(got.contains(&Frame::Abort { transfer: T }), "the recipient is told: {got:?}");
    assert!(got.contains(&Frame::Notice { transfer: T, paused: false }));
}

#[test]
fn handover_release_discards_a_departed_recipients_waiting_edits() {
    let (mut hs, mut host) = start_lent("handover-departed");
    let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x55; 32]);
    let _lease = hs.handover_bind(slot, generation, key, T).unwrap();
    hs.handover_freeze().unwrap();
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
    hs.handover_freeze().unwrap();
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
    }
    impl ClientTransport for Tap {
        fn send_to_server(&self, data: &[u8]) {
            let ready = crate::handover::live::recipient::handover_payload(data)
                .and_then(|b| Frame::decode(b).ok())
                .is_some_and(|f| matches!(f, Frame::Ready { .. }));
            match (ready, self.forged_ready.lock().unwrap().clone()) {
                (true, Some(forged)) => self.inner.send_to_server(&forged),
                _ => self.inner.send_to_server(data),
            }
        }
        fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
            self.inner.try_recv_from_server()
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
        dir: Dir,
    }

    fn root() -> RootContext {
        RootContext::new(
            keys(SOURCE_RUNTIME).public_key().to_bytes(),
            &npub(&keys(SOURCE_PERSONA)),
            now() + 3600,
            &["wss://relay.example".into()],
            now(),
        )
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
    fn markers(dir: &Dir) -> MarkerStore {
        MarkerStore::new(dir.0.join("markers")).unwrap()
    }

    /// A lent source with one signed joiner that keeps PLAYING through its
    /// RemoteClient: the Propose reaches it the ordinary way.
    fn pair(tag: &str) -> Pair {
        let dir = Dir::new(tag);
        for d in ["imports", "markers", "scratch", "world"] {
            std::fs::create_dir_all(dir.0.join(d)).unwrap();
        }
        let (mut hs, mut host) = start_lent(&format!("handover-{tag}"));
        let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [RECIPIENT_PERSONA; 32]);
        let forged_ready = Arc::new(Mutex::new(None));
        let tap = Tap { inner: Box::new(client), forged_ready: forged_ready.clone() };
        let rc = RemoteClient::from_transport(Box::new(tap), crate::remote_client::build_join_request_guest("dup", 0), None);
        let root = root();
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
        };
        let seat = Seat { slot, generation, key };
        let source_key = keys(SOURCE_PERSONA).public_key().to_bytes();
        let action = HostAction { seat, source_key, world_name: "Loop".into() };
        let live = Live { signed_in: Some(source_key), local_seats: 1 };
        let src = SourceSession::begin(&mut hs, action, cfg, live, now()).expect("eligible");
        Pair { hs, host, src, rc, dst: None, seat, live, root, forged_ready, dir }
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
            markers(&p.dir),
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
        for _ in 0..rounds {
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
        let markers = markers(&p.dir);
        assert!(markers.read(MarkerKind::Ready, p.src.transfer()).unwrap().is_some(), "Ready persisted");
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
        assert!(!p.hs.handover_frozen(), "and resumes play");
        // The recipient keeps its quarantined checkpoint after Ready.
        assert!(p.dst.as_ref().unwrap().checkpoint().is_some());
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
            RecipientSession::propose(payload, host, identity(), dir.0.join("imports"), markers(&dir), now())
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
}

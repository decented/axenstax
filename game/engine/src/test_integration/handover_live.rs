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
    assert_eq!(hs.handover_bind(slot, generation.wrapping_add(1), key), Err("that player is no longer signed in to this world"));
    assert!(hs.handover_bind(slot, generation, [9; 32]).is_err(), "another key");
    assert!(hs.handover_bind(0, generation, key).is_err(), "the host's own seat");
    hs.handover_bind(slot, generation, key).unwrap();
    assert!(hs.handover_bind(slot, generation, key).is_err(), "one link at a time");
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
    hs.handover_bind(slot, generation, key).unwrap();
    hs.handover_freeze().unwrap();
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
    // Pre-retirement release: same generations, so the waiting edits are
    // judged by the ordinary path, once, and the newcomer is met.
    hs.handover_release();
    assert!(!hs.handover_frozen());
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

mod session {
    use super::*;
    use crate::handover::{
        context::RootContext,
        durable::{MarkerKind, MarkerStore},
        live::{
            capture,
            eligibility::Seat,
            recipient::{RecipientConsent, RecipientIdentity, RecipientSession, RecipientStage},
            source::{Capture, HostAction, SourceConfig, SourceSession, SourceStage},
        },
        personal::{PersonalRecord, RecordContext},
        phase::Authority,
    };
    use nostr::{Keys, SecretKey, Timestamp, ToBech32};

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

    struct Pair {
        hs: HostedServer,
        host: OwnedSimParts,
        src: SourceSession,
        dst: RecipientSession,
        seat: Seat,
        source_persona: Keys,
        dir: Dir,
    }

    /// A lent source with one signed joiner whose RemoteClient is torn down
    /// after its transport moves into the recipient session.
    fn pair(tag: &str) -> Pair {
        let dir = Dir::new(tag);
        let (mut hs, mut host) = start_lent(&format!("handover-{tag}"));
        let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x71; 32]);
        let mut rc = crate::remote_client::RemoteClient::from_transport(
            Box::new(client),
            crate::remote_client::build_join_request_guest("dup", 0),
            None,
        );
        let link = rc.take_transport();
        rc.disconnect(); // the joined world's teardown: must not reach the link
        drop(rc);
        let runtime = keys(0x72);
        let attestation = pollster::block_on(crate::runtime_identity::mint_player_attestation(
            &keys(0x71),
            &runtime.public_key(),
            Timestamp::from(now()),
            1,
        ))
        .unwrap();
        std::fs::create_dir_all(dir.0.join("imports")).unwrap();
        std::fs::create_dir_all(dir.0.join("markers")).unwrap();
        let dst = RecipientSession::new(
            link,
            RecipientIdentity { runtime, attestation },
            dir.0.join("imports"),
            MarkerStore::new(dir.0.join("markers")).unwrap(),
        );
        let source_runtime = keys(0x81);
        let source_persona = keys(0x82);
        let root = RootContext::new(
            source_runtime.public_key().to_bytes(),
            &source_persona.public_key().to_bech32().unwrap(),
            now() + 3600,
            &["wss://relay.example".into()],
            now(),
        )
        .unwrap();
        let authority = Authority::root(&root, [5; 16]).unwrap();
        std::fs::create_dir_all(dir.0.join("scratch")).unwrap();
        let cfg = SourceConfig {
            runtime: source_runtime,
            root,
            authority,
            lineage: [5; 16],
            world: [6; 16],
            generation: 1,
            scratch: dir.0.join("scratch"),
            validity: 600,
        };
        let seat = Seat { slot, generation, key };
        let action = HostAction {
            seat,
            source_key: source_persona.public_key().to_bytes(),
            world_name: "Loop".into(),
        };
        let src = SourceSession::begin(&mut hs, action, cfg, Some(source_persona.public_key().to_bytes()), 1, &[])
            .expect("eligible");
        Pair { hs, host, src, dst, seat, source_persona, dir }
    }

    fn consent(p: &mut Pair) {
        p.dst.pump(now()).unwrap();
        assert_eq!(p.dst.stage(), RecipientStage::AwaitingConsent);
        let prop = p.dst.proposal().unwrap().clone();
        assert_eq!(prop.world_name, "Loop");
        assert_eq!(prop.source_npub, p.source_persona.public_key().to_bech32().unwrap());
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let c = RecipientConsent { transfer: prop.transfer, world_name: prop.world_name, source_npub: prop.source_npub };
        p.dst.consent(c, socket, now()).unwrap();
        p.host.lend_tick(&mut p.hs); // ordinary processing routes the Accept
        p.src.pump(&mut p.hs, now()).unwrap();
        assert_eq!(p.src.stage(), SourceStage::Frozen);
        assert!(p.hs.handover_frozen());
    }

    #[test]
    fn handover_loopback_reaches_durable_ready_with_distinct_records_and_latched_listener() {
        let mut p = pair("ready");
        consent(&mut p);
        // The frozen capture: the source's own seat 0 with its own state.
        let mut seat0 = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(3.0, 70.0, 3.0), 1.0);
        seat0.reputation.per_village.insert((2, -2), -40);
        seat0.kill_counter.insert(crate::mob::MobType::Pig, 7);
        seat0.bounties_claimed.insert(11, 2);
        seat0.inventory.set_slot(4, Some(crate::item::ItemStack::new_block(block::GLASS, 9)));
        let transfer = p.src.transfer();
        let context = |participant| RecordContext { participant, lineage: [5; 16], generation: 1, transfer };
        let source_key = p.source_persona.public_key().to_bytes();
        let sidecar = capture::local0_sidecar(&seat0, [9; 16]).unwrap();
        let progress = capture::local0_progress(&seat0, context(source_key)).unwrap();
        let meta = crate::handover::checkpoint::transfer_metadata(&crate::save::WorldMeta::new("Loop")).unwrap();
        let mut save = crate::save::build_world_save(&p.host.world, std::slice::from_ref(&seat0), 42, &[], &[]);
        let source_npub = p.source_persona.public_key().to_bech32().unwrap();
        crate::handover::live::eligibility::neutralise(&mut save, &source_npub).unwrap();
        let cap = Capture {
            meta: &meta,
            save: &save,
            world: &p.host.world,
            disk_chunks: None,
            exhibit_dir: None,
            source_sidecar: &sidecar,
            source_progress: &progress,
        };
        p.src.offer(&mut p.hs, cap, now()).unwrap();
        let tally = p.host.world.sim_tally;
        for _ in 0..10_000 {
            p.dst.pump(now()).unwrap();
            p.src.pump(&mut p.hs, now()).unwrap();
            if p.src.stage() == SourceStage::ReadyVerified {
                break;
            }
        }
        assert_eq!(p.src.stage(), SourceStage::ReadyVerified);
        assert_eq!(p.dst.stage(), RecipientStage::Ready);
        assert!(p.hs.handover_frozen() && p.host.world.sim_tally == tally, "still frozen, nothing simulated");
        assert!(!p.dst.gameplay_admitted(), "Ready is not activation");
        // Ready was persisted before it was sent.
        let markers = MarkerStore::new(p.dir.0.join("markers")).unwrap();
        assert!(markers.read(MarkerKind::Ready, p.src.transfer()).unwrap().is_some());
        // Distinct own records, each bound to its own identity.
        let cp = p.dst.checkpoint().unwrap();
        let got = |who: [u8; 32], kind: &str| {
            let n = nostr::PublicKey::from_slice(&who).unwrap().to_bech32().unwrap();
            std::fs::read(cp.received_world().join(format!("{kind}/{n}.dat"))).unwrap()
        };
        let src_rec = PersonalRecord::decode(&got(source_key, "participants"), &context(source_key)).unwrap();
        assert_eq!(src_rec.reputation, vec![((2, -2), -40)]);
        assert_eq!(src_rec.kills, vec![(crate::mob::MobType::Pig, 7)]);
        assert_eq!(src_rec.claims, vec![(11, 2)]);
        assert!(src_rec.local.is_some());
        let dst_rec = PersonalRecord::decode(&got(p.seat.key, "participants"), &context(p.seat.key)).unwrap();
        assert!(dst_rec.is_default() && dst_rec.local.is_none(), "the recipient inherits nothing");
        assert_eq!(got(source_key, "joiners"), sidecar);
        assert_ne!(got(p.seat.key, "joiners"), sidecar);
        // The latched listener refuses and never admits.
        let l = p.dst.listener().unwrap();
        assert!(!l.admits());
        let _try = crate::network::connect_to_server(l.local_addr());
        let mut refused = false;
        for _ in 0..100 {
            if l.refused() > 0 {
                refused = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(refused, "the latched listener did not refuse a connection");
    }

    #[test]
    fn handover_recipient_cancel_before_ready_restores_working_source() {
        let mut p = pair("cancel");
        consent(&mut p);
        let tally = p.host.world.sim_tally;
        p.dst.abort("changed my mind");
        assert!(p.src.pump(&mut p.hs, now()).is_err());
        assert!(matches!(p.src.stage(), SourceStage::Aborted(_)));
        assert!(!p.hs.handover_frozen());
        p.host.lend_tick(&mut p.hs);
        assert!(p.host.world.sim_tally != tally, "the source simulates again");
        assert!(p.hs.handover_link().is_none());
    }
}

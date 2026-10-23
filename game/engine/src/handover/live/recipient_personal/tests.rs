//! Stage/transport component controls; not durable receipt or active-carry evidence.
use super::*;
use crate::{
    handover::{
        context::RootContext,
        live::{
            recipient::{JoinedHost, RecipientIdentity},
            wire::{Frame, root_lineage},
        },
    },
    transport::ServerTransport,
};
use nostr::{Keys, SecretKey, Timestamp, ToBech32};

fn keys(byte: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[byte; 32]).unwrap())
}
fn session() -> RecipientSession {
    let runtime = keys(0x31);
    let source = keys(0x32);
    let identity_runtime = keys(0x33);
    let persona = keys(0x34);
    let now = 1_792_000_000;
    let root = RootContext::new(
        runtime.public_key().to_bytes(),
        &source.public_key().to_bech32().unwrap(),
        now + 3600,
        &["wss://relay.example".into()],
        now,
    )
    .unwrap();
    let payload = Frame::Propose {
        transfer: [1; 16],
        world_name: "Bounded transport fixture".into(),
        source_npub: source.public_key().to_bech32().unwrap(),
        challenge: [2; 16],
        root_runtime: root.runtime(),
        root_persona: source.public_key().to_bech32().unwrap(),
        root_expiry: root.expiry(),
        root_relays: root.relays().to_vec(),
        lineage: root_lineage(&root),
    }
    .encode()
    .unwrap();
    let attestation = pollster::block_on(crate::runtime_identity::mint_player_attestation(
        &persona,
        &identity_runtime.public_key(),
        Timestamp::from(now),
        1,
    ))
    .unwrap();
    RecipientSession::propose(
        &payload,
        JoinedHost {
            persona: source.public_key().to_bytes(),
            runtime: runtime.public_key().to_bytes(),
        },
        RecipientIdentity {
            runtime: identity_runtime,
            attestation,
        },
        std::env::temp_dir().join("axenstax-recipient-control-no-import"),
        now,
    )
    .unwrap()
}
fn consent(session: &mut RecipientSession, client: crate::transport::ChannelClientTransport) {
    let proposal = session.proposal();
    let consent = RecipientConsent {
        transfer: proposal.transfer,
        world_name: proposal.world_name.clone(),
        source_npub: proposal.source_npub.clone(),
    };
    session
        .consent(
            consent,
            std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
            Box::new(client),
            1_792_000_000,
        )
        .unwrap();
}
#[test]
fn d2c3_recipient_bounded_reader_yields_with_original_raw_remainder() {
    let mut session = session();
    let (server, client) = crate::transport::channel_pair();
    consent(&mut session, client);
    assert!(
        server.try_recv_from_client().is_some(),
        "real signed consent frame"
    );
    for n in 0..100u8 {
        server.send_to_client(&[protocol::PacketType::ClientInput as u8, n]);
    }
    session.read_link_bounded(1_792_000_001, 32).unwrap();
    assert_eq!(session.parked.len(), 32);
    assert_eq!(session.parked.front().unwrap()[1], 0);
    assert_eq!(session.parked.back().unwrap()[1], 31);
    assert!(
        server.try_recv_from_client().is_none(),
        "raw receipt grants no Personal ACK"
    );
    session.read_link_bounded(1_792_000_002, 32).unwrap();
    assert_eq!(session.parked.len(), 64);
    assert_eq!(session.parked.back().unwrap()[1], 63);
}
#[test]
fn d2c3_recipient_abort_returns_original_order_once_without_personal_ack() {
    let mut session = session();
    let (server, client) = crate::transport::channel_pair();
    consent(&mut session, client);
    let _ = server.try_recv_from_client();
    let originals = vec![
        vec![protocol::PacketType::ClientInput as u8, 1],
        vec![protocol::PacketType::StateUpdate as u8, 2],
        vec![protocol::PacketType::InventoryGrant as u8, 3],
    ];
    for packet in &originals {
        server.send_to_client(packet);
    }
    session.read_link_bounded(1_792_000_001, 32).unwrap();
    assert!(server.try_recv_from_client().is_none());
    session.abort("component abort");
    let (_link, replay) = session.return_link().expect("still-open real link");
    assert_eq!(replay.into_iter().collect::<Vec<_>>(), originals);
    assert!(session.return_link().is_none());
}
#[test]
fn d2c3_recipient_parked_bound_counts_actual_capacity_and_metadata() {
    let mut session = session();
    let mut packet = Vec::with_capacity(crate::handover::live::recipient::PARKED_BYTES);
    packet.push(protocol::PacketType::StateUpdate as u8);
    session.park_gameplay_packet(packet);
    assert!(
        session.parked_overflow,
        "outer queue metadata also occupies retained memory"
    );
    assert!(session.parked.is_empty());
    assert!(session.return_link().is_none());
}
#[test]
fn d2c3_recipient_durable_stage_dispatch_precedes_personal_link_gates() {
    let mut session = session();
    let (_server, client) = crate::transport::channel_pair();
    let mut remote = RemoteClient::from_transport(
        Box::new(client),
        crate::remote_client::build_join_request_guest("No live Personal binding", 0),
        None,
    );
    let mut window = WindowInbox::default();
    let mut slot = PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    // Isolated enum-dispatch fixture only: this does not manufacture or claim
    // a persisted Prepared receipt, current ACK, readiness or an accepted cut.
    for stage in [
        RecipientStage::Prepared,
        RecipientStage::Activated,
        RecipientStage::Quarantined("fixture"),
    ] {
        session.stage = stage;
        assert_eq!(
            session
                .pump_personal_control(
                    &mut remote,
                    &mut window,
                    &mut slot,
                    (usize::MAX, usize::MAX),
                    1_792_000_001
                )
                .unwrap(),
            PersonalControlProgress::Idle
        );
        assert_eq!(session.stage(), stage);
    }
}

//! Decoder negatives and actual ordinary guest admission; no carry readiness is assigned.
use super::*;

#[test]
fn d2c3_shared_personal_decoder_rejects_applied_and_oversized_input() {
    let applied = protocol::PersonalSyncPacket::Applied(protocol::PersonalAck {
        session: [7; 16],
        life: 0,
        revision: u64::MAX,
        window_event: u32::MAX,
        snapshot: [9; 32],
    });
    assert!(decode(&bincode::serialize(&applied).unwrap()).is_err());
    assert!(decode(&vec![0; protocol::MAX_PACKET_SIZE as usize + 1]).is_err());
    assert!(decode(&[0, 1, 2]).is_err());
}
#[test]
fn d2c3_shared_personal_normal_rejects_without_initial_inbox() {
    let (_server, client) = crate::transport::channel_pair();
    let mut remote = RemoteClient::from_transport(
        Box::new(client),
        crate::remote_client::build_join_request_guest("Decoder fixture", 0),
        None,
    );
    let context = crate::progression::GameplayContext::Guest {
        session: [7; 16],
        generation: 1,
    };
    let record = crate::progression::GameplayRecord::empty(context).unwrap();
    let value = protocol::PersonalSnapshot {
        session: [7; 16],
        life: 0,
        revision: 1,
        window_event: 2,
        context,
        bytes: record.encode().unwrap(),
    };
    let payload = bincode::serialize(&protocol::PersonalSyncPacket::Snapshot(value)).unwrap();
    assert!(
        remote
            .receive_personal_normal(&payload, payload.capacity())
            .is_err()
    );
    assert!(remote.personal_inbox.is_none());
    assert!(remote.pending_outcomes.is_empty());
    assert_eq!(remote.applied_window_event(), 0);
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn d2c3_shared_personal_normal_uses_actual_server_snapshot_and_marker() {
    let _root = crate::save::WorldsRootGuard::new("prepared-normal-guest");
    let mut host = crate::hosted_server::HostedServer::start(
        0,
        "prepared-normal-guest".into(),
        42,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
    )
    .unwrap();
    let client = host.attach_test_remote();
    let mut remote = RemoteClient::from_transport(
        Box::new(client),
        crate::remote_client::build_join_request_guest("Actual guest", 0),
        None,
    );
    host.tick();
    remote.poll();
    assert!(
        remote.is_connected(),
        "actual guest must be admitted before inspecting its carriers"
    );
    let inbox = remote
        .personal_inbox
        .as_ref()
        .expect("actual common initial snapshot");
    let markers: Vec<_> = remote
        .pending_outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            RequestOutcome::PersonalSync {
                window_event,
                actor_respawns,
            } => Some((*window_event, *actor_respawns)),
            _ => None,
        })
        .collect();
    assert_eq!(markers, [(2, 0)]);
    assert!(inbox.has_pending_fence(2, 0));
    assert!(!inbox.initialised());
    assert!(
        !remote.personal_ready(),
        "receipt alone cannot install or acknowledge initial state"
    );
    assert_eq!(remote.applied_window_event(), 0);
    assert!(remote.park_for_personal_control([3; 16]).is_err());
    assert!(!remote.is_parked());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn d2c3_personal_replay_retains_actual_capacity_after_transport_restore() {
    let (_server, client) = crate::transport::channel_pair();
    let mut remote = RemoteClient::from_transport(
        Box::new(client),
        crate::remote_client::build_join_request_guest("Raw ownership fixture", 0),
        None,
    );
    // Raw queue fixture only: no body, readiness or ACK is fabricated.
    let link = remote.park_transport();
    let mut packet = Vec::with_capacity(crate::action_journal::MAX_RETAINED_BYTES);
    packet.push(protocol::PacketType::StateUpdate as u8);
    let packet_capacity = packet.capacity();
    let mut replay = std::collections::VecDeque::new();
    replay.push_back(packet);
    let outer = replay.capacity() * std::mem::size_of::<Vec<u8>>();
    remote.restore_transport(link, replay);
    let usage = remote.personal_control_retained_usage();
    assert_eq!(usage.0, 1);
    assert!(usage.1 >= packet_capacity + outer);
    assert!(
        budget(usage).is_err(),
        "transfer cannot reset aggregate retained usage"
    );
}

//! A real in-flight prefix and an otherwise identical ordinary-delivery control.
use super::*;
use crate::test_integration::handover_carry_fixture as activity;

fn current_carry(delayed: bool) {
    let _worlds = crate::save::WorldsRootGuard::new(if delayed {
        "carry-delayed"
    } else {
        "carry-no-delay"
    });
    // The normal recipient environment prepares this parent before consent.
    let imports = crate::data_dir::data_root().join("handover").join("imports");
    std::fs::create_dir_all(&imports).expect("prepare the positive carry import parent");
    assert!(std::fs::symlink_metadata(&imports).unwrap().is_dir());
    activity::prepare_scratch(
        &crate::data_dir::data_root().join("handover").join("cadence-scratch"),
    );
    let relay = crate::handover::relay_fixture::Relay::start();
    let a = Actor {
        persona: 0x82,
        runtime: 0x81,
    };
    let b = Actor {
        persona: 0x71,
        runtime: 0x72,
    };
    let mut source = activity::active_source("cadence-source", [a.persona; 32]);
    let mut peer = activity::boot_carry_world("cadence-peer");
    let hub = FakeRelayHub::new();
    host(&mut source, a, b, vec![relay.url()], &hub);
    let link = source
        .state
        .online_host
        .as_ref()
        .unwrap()
        .invite_link(now())
        .unwrap();
    let original = crate::invite::Invite::parse(&link, now()).unwrap();
    let root = source
        .state
        .online_host
        .as_ref()
        .unwrap()
        .root_context(now())
        .unwrap();
    let bearer = source
        .state
        .online_host
        .as_ref()
        .unwrap()
        .live_bearer(now())
        .unwrap();
    let resolved = resolve(&link);
    assert_eq!(resolved.authority().unwrap().epoch(), 0);
    let joined = admission(&mut source, b, &original, &hub, &resolved);
    let checked_host = joined.host_contact().unwrap().pubkey;
    let checked_runtime = joined.host_runtime();
    let checked_continuation = joined.continuation().cloned();
    crate::online_prep::retire(joined);
    let client = source
        .state
        .hosted_server
        .as_mut()
        .unwrap()
        .attach_test_remote_with_binding(Some(BINDING));
    // Installed before the actual signed join; parking retains this exact link.
    let (client, control) = activity::delay::wrap(client);
    let (_seat, observation) = activity::attach(
        &mut source,
        &mut peer,
        client,
        [b.persona; 32],
        checked_host,
        checked_runtime,
    );
    peer.state.joined_continuation = checked_continuation;
    recipient_fixture(
        &mut peer,
        b,
        crate::data_dir::data_root()
            .join("handover")
            .join("imports"),
    );
    let seat = activity::active_recipient(&mut source, &mut peer, &observation);
    assert!(seat.2 == b.pubkey(), "actual recipient npub differs");
    activity::assert_boundary_ready(&source, &peer, seat);
    let original_scope = source.state.world.quest_scope.unwrap();
    source
        .state
        .begin_handover_with(
            seat,
            SourceEnv {
                runtime: a.keys(),
                root,
                scratch: crate::data_dir::data_root()
                    .join("handover")
                    .join("cadence-scratch"),
                bearer: Bearer::new(bearer),
                attestation: a.attestation(),
                continuation: None,
            },
            now(),
        )
        .unwrap();
    let completed = activity::continuation::transfer_with_delay(
        &mut source,
        &mut peer,
        seat,
        &control,
        delayed,
    );
    assert_eq!(completed.scope, original_scope);
    assert_eq!(resolve(&link).authority().unwrap().epoch(), 1);
}

#[test]
#[ignore = "needs two real GameState GPU adapters and signed quest/menu setup"]
fn game_harness_d2c3_carry_current_cadence_without_network_delay() {
    current_carry(false);
}

#[test]
#[ignore = "needs two real GameState GPU adapters and signed quest/menu setup"]
fn game_harness_d2c3_carry_current_cadence_with_actual_in_flight_prefix() {
    current_carry(true);
}

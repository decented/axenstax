//! First actual original-capability carry boundary with genuine common activity.
use super::*;
use crate::test_integration::handover_carry_fixture as activity;

#[test]
#[ignore = "needs two real GameState GPU adapters and signed quest/menu setup"]
fn game_harness_d2c3_original_chain_preserves_current_active_participant_quests_and_timers() {
    let _worlds = crate::save::WorldsRootGuard::new("active-original-boundary");
    let relay = crate::handover::relay_fixture::Relay::start();
    let a = Actor { persona: 0x82, runtime: 0x81 };
    let b = Actor { persona: 0x71, runtime: 0x72 };
    let mut source = activity::active_source("active-original-source", [a.persona; 32]);
    let mut peer = HeadlessGame::boot_into_world("active-original-peer");
    let hub = FakeRelayHub::new();
    host(&mut source, a, b, vec![relay.url()], &hub);
    let link = source.state.online_host.as_ref().unwrap().invite_link(now()).unwrap();
    let original = crate::invite::Invite::parse(&link, now()).unwrap();
    let root = source.state.online_host.as_ref().unwrap().root_context(now()).unwrap();
    let bearer = source.state.online_host.as_ref().unwrap().live_bearer(now()).unwrap();
    let resolved = resolve(&link);
    assert_eq!(resolved.authority().unwrap().epoch(), 0);
    let joined = admission(&mut source, b, &original, &hub, &resolved);
    let checked_host = joined.host_contact().unwrap().pubkey;
    let checked_runtime = joined.host_runtime();
    let checked_continuation = joined.continuation().cloned();
    crate::online_prep::retire(joined);
    let client = source.state.hosted_server.as_mut().unwrap().attach_test_remote_with_binding(Some(BINDING));
    let (_seat, observation) =
        activity::attach(&mut source, &mut peer, client, [b.persona; 32], checked_host, checked_runtime);
    peer.state.joined_continuation = checked_continuation;
    recipient_fixture(&mut peer, b, crate::data_dir::data_root().join("handover").join("imports"));
    let seat = activity::active_recipient(&mut source, &mut peer, &observation);
    assert!(seat.2 == b.pubkey(), "actual recipient npub differs");
    activity::assert_boundary_ready(&source, &peer, seat);
    let before = activity::boundary(&source, &peer, seat);
    assert!(source.state.handover.is_none());
    let result = source.state.begin_handover_with(
        seat,
        SourceEnv {
            runtime: a.keys(),
            root: root.clone(),
            scratch: crate::data_dir::data_root().join("handover").join("active-original-scratch"),
            bearer: Bearer::new(bearer),
            attestation: a.attestation(),
            continuation: None,
        },
        now(),
    );
    eprintln!("active-original first begin result={result:?} root_epoch=0 slot={} generation={}", seat.0, seat.1);
    if result.is_err() {
        assert!(source.state.handover.is_none());
        assert!(
            activity::boundary(&source, &peer, seat) == before,
            "refused first begin changed authority, holders, clocks or files"
        );
        assert!(
            source.state.online_host.as_ref().unwrap().invite_link(now()).unwrap() == link,
            "the original invitation changed"
        );
        assert_eq!(source.state.online_host.as_ref().unwrap().root_context(now()).unwrap(), root);
        assert!(
            source.state.online_host.as_ref().unwrap().live_bearer(now()) == Some(bearer),
            "the original capability changed"
        );
        assert_eq!(resolve(&link).prefix, resolved.prefix, "refused begin changed the checked original prefix");
    }
    // FIRST OWN: current common-active refusal must be observed here before
    // any frozen projection, Offer, Imported context or continued epoch exists.
    assert!(result.is_ok(), "genuine active participants must reach original carry: {result:?}");
    // This checkpoint owns only the first boundary. Actual0→1→2→3 transfer,
    // native/dedicated completion and retained proof acceptance are future tests.
}

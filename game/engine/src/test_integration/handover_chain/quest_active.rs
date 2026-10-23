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
    let mut completed = activity::continuation::transfer(&mut source, &mut peer, seat, now);
    let original_scope = completed.scope;
    for (epoch, source_actor, peer_actor) in [(1, a, b), (2, b, a), (3, a, b)] {
        let proof = crate::handover::live::personal_adapter::proof(&completed.working).unwrap().unwrap();
        assert_eq!((proof.core.epoch, proof.core.generation), (epoch, u64::from(epoch)));
        assert_eq!(proof.core.context, root.digest());
        assert_eq!(completed.scope, original_scope);
        assert!(completed.cut_tick > 0);
        let retired = source.state.world_name.clone();
        assert!(crate::handover::live::authority::open_guard(&crate::save::world_dir(&retired)).is_err());
        let retained = crate::handover::live::personal_adapter::proof(&crate::save::world_dir(&completed.retained))
            .unwrap()
            .unwrap();
        assert!(retained.is_retained());
        assert_eq!(retained.local_owner(), source_actor.pubkey());
        assert_eq!(retained.core, proof.core);
        assert!(source.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
        let imported = activity::continuation::promoted(&mut peer, completed.transfer);
        recipient_fixture(&mut peer, peer_actor, crate::data_dir::data_root().join("handover").join("imports"));
        activity::continuation::open(&mut peer, &imported);
        activity::continuation::restored(&mut peer, &completed.recipient, original_scope, peer_actor.pubkey());
        let cold = resolve(&link);
        assert_eq!(cold.root, root);
        assert_eq!(cold.authority().unwrap().epoch(), epoch);
        assert_eq!(cold.authority().unwrap().runtime(), peer_actor.keys().public_key().to_bytes());
        if epoch == 3 {
            activity::continuation::complete_native_once(&mut peer);
            break;
        }
        let own_before_save = activity::native(&peer);
        peer.state.save_live_world().unwrap();
        assert!(peer.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
        activity::continuation::open(&mut peer, &imported);
        activity::continuation::restored(&mut peer, &own_before_save, original_scope, peer_actor.pubkey());
        std::mem::swap(&mut source, &mut peer);
        // The returning actor is a genuine fresh client process/view. Its own
        // state must come from the transferred canonical member, not old RAM.
        drop(peer);
        peer = HeadlessGame::boot_into_world(&format!("active-return-client-{epoch}"));
        let next_source = peer_actor;
        let next_peer = source_actor;
        let hub = FakeRelayHub::new();
        host(&mut source, next_source, next_peer, root.relays().to_vec(), &hub);
        let canonical = crate::handover::live::personal_adapter::path(
            &crate::save::world_dir(&source.state.world_name),
            next_peer.pubkey(),
        );
        let offline_bytes = std::fs::read(&canonical).unwrap();
        let offline =
            crate::joiner_save::JoinerSave::decode(&offline_bytes).unwrap().personal.unwrap().record().unwrap();
        source.hosted_ticks(20);
        assert_eq!(
            std::fs::read(&canonical).unwrap(),
            offline_bytes,
            "offline members do not consume the new host's active ticks"
        );
        let resolved = resolve(&link);
        assert_eq!(resolved.authority().unwrap().epoch(), epoch);
        let joined = admission(&mut source, next_peer, &original, &hub, &resolved);
        let checked_host = joined.host_contact().unwrap().pubkey;
        let checked_runtime = joined.host_runtime();
        let continuation = joined.continuation().cloned();
        crate::online_prep::retire(joined);
        let client = source.state.hosted_server.as_mut().unwrap().attach_test_remote_with_binding(Some(BINDING));
        recipient_fixture(&mut peer, next_peer, crate::data_dir::data_root().join("handover").join("imports"));
        let before_join = source.state.tick_counter;
        let (next_seat, _observation) =
            activity::attach(&mut source, &mut peer, client, [next_peer.persona; 32], checked_host, checked_runtime);
        peer.state.joined_continuation = continuation;
        let sp = &source.state.hosted_server.as_ref().unwrap().server.players[next_seat.0];
        let current =
            sp.gameplay_clock.project(&sp.personal.as_ref().unwrap().record().unwrap(), source.state.tick_counter).0;
        activity::continuation::reconnect_progress(&current, &offline, source.state.tick_counter - before_join);
        source
            .state
            .begin_handover_with(
                next_seat,
                SourceEnv {
                    runtime: next_source.keys(),
                    root: root.clone(),
                    scratch: crate::data_dir::data_root()
                        .join("handover")
                        .join(format!("active-chain-{epoch}-scratch")),
                    bearer: Bearer::new(bearer),
                    attestation: next_source.attestation(),
                    continuation: Some(resolved),
                },
                now(),
            )
            .unwrap();
        completed = activity::continuation::transfer(&mut source, &mut peer, next_seat, now);
    }
    assert_eq!(resolve(&link).authority().unwrap().epoch(), 3);
}

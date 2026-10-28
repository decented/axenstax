//! Actual GameState controls around normal-owned consent and current preparation.
use super::*;

fn consent_draining(j: &mut Joined) -> SourceSession {
    let src = source(j);
    for _ in 0..400 {
        step(j);
        if recipient_stage(&j.hg) == Some(RecipientStage::AwaitingConsent) {
            break;
        }
    }
    assert_eq!(
        recipient_stage(&j.hg),
        Some(RecipientStage::AwaitingConsent)
    );
    arm_consent(&mut j.hg);
    j.hg.state.upnp_lease = Some(crate::online_prep::LeaseKeeper::lost_for_test());
    j.hg.state.handover_click = Some(UiAction::Accept);
    j.hg.frames(1);
    assert_eq!(recipient_stage(&j.hg), Some(RecipientStage::Draining));
    assert!(!j.hg.state.remote_client.as_ref().unwrap().is_parked());
    assert!(j.hg.state.upnp_lease.is_some());
    src
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_ordered_park_cancel_before_freeze_keeps_actual_normal_link_lease_and_inbox() {
    let _worlds = crate::save::WorldsRootGuard::new("park-normal-cancel");
    let mut j = joined("park-normal-cancel");
    let mut src = consent_draining(&mut j);
    let remote = j.hg.state.remote_client.as_ref().unwrap();
    let before = remote
        .personal_inbox
        .as_ref()
        .unwrap()
        .installed_identity(remote.applied_window_event())
        .unwrap();
    j.hg.state.handover_click = Some(UiAction::Cancel);
    j.hg.frames(1);
    assert!(j.hg.state.handover.is_none());
    assert!(
        j.hg.state.upnp_lease.is_some(),
        "normal owner never surrendered its actual lease"
    );
    let remote = j.hg.state.remote_client.as_ref().unwrap();
    assert!(!remote.is_parked());
    assert!(!remote.handover_drain_active(src.transfer()));
    assert!(
        before.same_attachment(
            &remote
                .personal_inbox
                .as_ref()
                .unwrap()
                .installed_identity(remote.applied_window_event())
                .unwrap()
        )
    );
    j.host.lend_tick(&mut j.hs);
    assert!(
        src.pump(&mut j.hs, live(), now()).is_err(),
        "real normal-owner Abort reaches the source"
    );
    assert!(!j.hs.handover_frozen());
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_ordered_park_changed_sign_in_aborts_during_normal_owned_wait() {
    let _worlds = crate::save::WorldsRootGuard::new("park-normal-identity");
    let mut j = joined("park-normal-identity");
    let mut src = consent_draining(&mut j);
    // This fixture seam represents an actual sign-out; no attachment or
    // Personal/window/source frontier is changed to produce the refusal.
    j.hg.state.handover_fixture.as_mut().unwrap().signed_in = None;
    j.hg.frames(1);
    assert!(j.hg.state.handover.is_none());
    assert!(!j.hg.state.remote_client.as_ref().unwrap().is_parked());
    assert!(j.hg.state.upnp_lease.is_some());
    assert_eq!(
        j.hg.state.toast.as_ref().map(|(text, _)| text.as_str()),
        Some("Your sign-in changed during the handover.")
    );
    j.host.lend_tick(&mut j.hs);
    assert!(src.pump(&mut j.hs, live(), now()).is_err());
    assert!(!j.hs.handover_frozen());
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_ordered_park_leave_during_normal_draining_cancels_before_owner_teardown() {
    let _worlds = crate::save::WorldsRootGuard::new("park-normal-leave");
    let mut j = joined("park-normal-leave");
    let mut src = consent_draining(&mut j);
    assert!(j.hg.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    assert!(
        j.hg.state.handover.is_none(),
        "an unparked session cannot outlive its actual reader"
    );
    assert!(j.hg.state.remote_client.is_none());
    assert!(matches!(j.hg.state.mode, crate::GameMode::Menu(_)));
    j.host.lend_tick(&mut j.hs);
    assert!(src.pump(&mut j.hs, live(), now()).is_err());
    assert!(!j.hs.handover_frozen());
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_ordered_park_real_current_holders_refuse_raw_input_before_commit_and_resume() {
    let _worlds = crate::save::WorldsRootGuard::new("park-current-busy");
    let dir = Tmp::new("park-current-busy");
    let (mut source, mut far, slot) = proposed("park-current-busy", &dir);
    // Establish a genuinely accepted positive input before the adverse raw one.
    for _ in 0..2 {
        let p = &source.state.hosted_server.as_ref().unwrap().server.players[slot];
        far.rc
            .send_input(&protocol::InputPacket {
                x: p.player.pos.x,
                y: p.player.pos.y,
                z: p.player.pos.z,
                health: p.combat.health,
                ..Default::default()
            })
            .unwrap();
        source.hosted_ticks(1);
        far.application.receive(&mut far.rc, &source.state.registry);
    }
    let proposal = far
        .rc
        .take_handover_frames()
        .into_iter()
        .find(|bytes| matches!(Frame::decode(bytes), Ok(Frame::Propose { .. })))
        .unwrap();
    let mut recipient = RecipientSession::propose(
        &proposal,
        joined_host(),
        identity(),
        dir.0.join("imports"),
        now(),
    )
    .unwrap();
    far.application.consent(&mut far.rc, &mut recipient, now());
    far.dst = Some(recipient);
    round(&mut source);
    assert_eq!(source_stage(&source), Some(SourceStage::AwaitingParked));
    let p = &source.state.hosted_server.as_ref().unwrap().server.players[slot];
    let accepted_before = p.last_input_tick;
    let raw = far
        .rc
        .send_input(&protocol::InputPacket {
            x: p.player.pos.x,
            y: p.player.pos.y,
            z: p.player.pos.z,
            health: p.combat.health,
            ..Default::default()
        })
        .unwrap();
    assert!(raw > accepted_before);
    for _ in 0..2048 {
        far.application.progress(
            &mut far.rc,
            far.dst.as_mut().unwrap(),
            &source.state.registry,
            now(),
        );
        if far.rc.is_parked() {
            break;
        }
    }
    assert!(far.rc.is_parked());
    // Take the actual GameState session only to inspect this precise API
    // boundary; no authority/ACK/frontier/timer value is assigned by the test.
    let Some(Handover::Source { mut session, .. }) = source.state.handover.take() else {
        panic!("no source")
    };
    let tick = source.state.tick_counter;
    let scope = source.state.world.quest_scope;
    let before_native = crate::progression::GameplayRecord::from_slot(
        source.state.players[0]
            .personal_binding
            .as_ref()
            .unwrap()
            .context(),
        &source.state.players[0],
    )
    .unwrap();
    let hs = source.state.hosted_server.as_mut().unwrap();
    let before_remote = hs.server.players[slot]
        .personal
        .as_ref()
        .unwrap()
        .record()
        .unwrap();
    session.pump(hs, live(), now()).unwrap();
    assert_eq!(session.stage(), SourceStage::Frozen);
    assert!(!session.test_current_cut_committed());
    assert_eq!(hs.server.players[slot].last_input_tick, accepted_before);
    let error = session
        .settle_current_cut(&mut source.state.players[0], hs, live(), tick, scope, now())
        .unwrap_err();
    assert_eq!(
        error.0,
        "players' actions or personal state are still settling; try again in a moment"
    );
    assert!(
        !session.test_current_cut_committed(),
        "ordinary raw work refused BEFORE two-holder commit"
    );
    assert!(!hs.handover_frozen());
    assert_eq!(
        hs.server.players[slot]
            .personal
            .as_ref()
            .unwrap()
            .record()
            .unwrap(),
        before_remote
    );
    assert_eq!(
        crate::progression::GameplayRecord::from_slot(
            source.state.players[0]
                .personal_binding
                .as_ref()
                .unwrap()
                .context(),
            &source.state.players[0]
        )
        .unwrap(),
        before_native
    );
    assert_eq!(source.state.tick_counter, tick);
    source.hosted_ticks(1);
    assert_eq!(
        source.state.hosted_server.as_ref().unwrap().server.players[slot].last_input_tick,
        raw
    );
}

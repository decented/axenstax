//! Real transport/admission component controls; no GameState carry claim.
use super::*;
mod bounds;

fn awaiting(p: &mut Pair) {
    consent_only(p);
    p.src.pump(&mut p.hs, p.live, now()).unwrap();
    assert_eq!(p.src.stage(), SourceStage::AwaitingParked);
    assert!(p.hs.handover_frozen());
    assert!(!p.rc.is_parked());
}

fn queued_actual_park(p: &mut Pair) {
    for _ in 0..2048 {
        p.application.progress(
            &mut p.rc,
            p.dst.as_mut().unwrap(),
            &p.hs.server.registry,
            now(),
        );
        if p.rc.is_parked() {
            return;
        }
    }
    panic!("genuine component owner never parked");
}

#[test]
fn ordered_park_source_deadline_precedes_already_queued_valid_proof() {
    let mut p = pair("park-late-proof");
    awaiting(&mut p);
    queued_actual_park(&mut p);
    assert_eq!(
        p.src.stage(),
        SourceStage::AwaitingParked,
        "the actual proof has not been read"
    );
    let error = p
        .src
        .pump(
            &mut p.hs,
            p.live,
            now() + crate::handover::live::park::WAIT_SECS + 1,
        )
        .unwrap_err();
    assert_eq!(
        error.0,
        "The other game didn't settle in time. Play has resumed."
    );
    assert!(!p.hs.handover_frozen());
    assert!(!p.src.wants_capture());
}

#[test]
fn ordered_park_source_rejects_closed_link_with_valid_early_proof_queued() {
    let mut p = pair("park-closed-proof");
    awaiting(&mut p);
    queued_actual_park(&mut p);
    p.dst.as_mut().unwrap().abort("actual component close");
    let (transport, waiting) = p
        .dst
        .as_mut()
        .unwrap()
        .return_link()
        .expect("actual held owner");
    assert!(waiting.is_empty());
    drop(transport); // A real close, not an eligibility flag assignment.
    let error = p.src.pump(&mut p.hs, p.live, now()).unwrap_err();
    assert_eq!(error.0, "the recipient left or reconnected");
    assert!(!p.hs.handover_frozen());
    assert!(
        !p.src.wants_capture(),
        "early proof never uses the final PreparedAck close exception"
    );
}

#[test]
fn ordered_park_normal_poll_needs_actual_application_credit_and_fence_stop_is_sticky() {
    let mut p = pair("park-poll-credit");
    awaiting(&mut p);
    let transfer = p.src.transfer();
    // The real source sent its pause notice then its checked fence. One
    // total carrier includes that first control, not only ordinary packets.
    let mut saw_notice = false;
    for _ in 0..2048 {
        p.rc.poll();
        let frames = p.rc.take_handover_frames();
        if !frames.is_empty() {
            assert_eq!(frames.len(), 1);
            assert!(matches!(
                Frame::decode(&frames[0]),
                Ok(Frame::Notice { paused: true, .. })
            ));
            saw_notice = true;
            break;
        }
        // Any real pre-freeze ordinary prefix is applied, never discarded.
        p.application.receive(&mut p.rc, &p.hs.server.registry);
    }
    assert!(
        saw_notice,
        "the actual source pause notice must precede its fence"
    );
    assert!(p.rc.handover_drain_fence(transfer).is_none());
    p.rc.poll();
    p.rc.poll();
    assert!(
        p.rc.handover_drain_fence(transfer).is_none(),
        "another poll cannot mint application credit"
    );
    p.application.receive(&mut p.rc, &p.hs.server.registry); // Actual passive application pass.
    p.rc.poll();
    assert!(p.rc.handover_drain_fence(transfer).is_some());
    p.hs.handover_send(
        &Frame::Notice {
            transfer,
            paused: true,
        }
        .encode()
        .unwrap(),
    )
    .unwrap();
    for _ in 0..3 {
        p.application.receive(&mut p.rc, &p.hs.server.registry);
        p.rc.poll();
        assert!(
            p.rc.take_handover_frames().is_empty(),
            "post-fence bytes must remain raw across all polls"
        );
    }
    queued_actual_park(&mut p);
    p.dst.as_mut().unwrap().pump(now()).unwrap(); // Actual held owner can now read that later notice.
}

#[test]
fn ordered_park_modified_source_generation_cannot_verify_a_genuine_park_signature() {
    let mut p = pair("park-wrong-generation");
    p.park_fence_fault
        .store(1, std::sync::atomic::Ordering::Relaxed);
    awaiting(&mut p);
    queued_actual_park(&mut p);
    let error = p.src.pump(&mut p.hs, p.live, now()).unwrap_err();
    assert_eq!(
        error.0,
        "The handover no longer matches this joined connection."
    );
    assert!(!p.hs.handover_frozen());
    assert!(!p.src.wants_capture());
}

#[test]
fn ordered_park_bad_authority_and_oversized_fence_refuse_with_actual_normal_owner_and_raw_fifo() {
    for fault in [2, 3, 4] {
        let mut p = pair(&format!("park-invalid-fence-{fault}"));
        p.park_fence_fault
            .store(fault, std::sync::atomic::Ordering::Relaxed);
        awaiting(&mut p);
        let transfer = p.src.transfer();
        for _ in 0..2048 {
            p.application.receive(&mut p.rc, &p.hs.server.registry);
            p.rc.take_handover_frames(); // Pause notices only; no ordinary body is removed here.
            if p.rc.handover_drain_failure(transfer).is_some() {
                break;
            }
        }
        assert!(p.rc.handover_drain_failure(transfer).is_some());
        assert!(
            !p.rc.is_parked(),
            "invalid fixed control never consumes the actual owner"
        );
        let mut replay = p.rc.test_handover_replay();
        let front = replay
            .next()
            .expect("the rejected actual raw allocation remains owned");
        if fault == 4 {
            assert_eq!(front.len(), 137, "valid short fixed fence bytes");
            assert!(front.capacity() > crate::handover::live::park::WIRE_CAP,
                "this case must fail on actual capacity, not encoded length");
        }
        assert_eq!(
            protocol::deserialize_header(front).unwrap().0,
            PacketType::Handover
        );
        assert!(replay.next().is_none());
        let allocation = front.as_ptr();
        p.rc.poll();
        assert_eq!(
            p.rc.test_handover_replay().next().unwrap().as_ptr(),
            allocation,
            "further polls cannot consume a rejected control into apparently empty input"
        );
        p.rc.cancel_handover_drain(transfer);
        p.rc.send_raw(&handover_frame(
            &Frame::Abort {
                transfer,
                aborted: None,
            }
            .encode()
            .unwrap(),
        ));
        assert!(p.src.pump(&mut p.hs, p.live, now()).is_err());
        assert!(!p.hs.handover_frozen());
    }
}

#[test]
fn ordered_park_pre_send_refusal_keeps_the_genuine_returned_owner_recoverable() {
    let mut p = pair("park-pre-send-owner");
    awaiting(&mut p);
    let transfer = p.src.transfer();
    let mut observed = None;
    for _ in 0..2048 {
        p.application.receive(&mut p.rc, &p.hs.server.registry);
        p.rc.take_handover_frames();
        observed = p.rc.handover_drain_fence(transfer);
        if observed.is_some() {
            break;
        }
    }
    let fence = observed.expect("actual source fence reached the same normal client");
    assert!(p.rc.handover_drain_application_ready(transfer));
    let before =
        p.rc.personal_inbox
            .as_ref()
            .unwrap()
            .installed_identity(p.rc.applied_window_event())
            .unwrap();
    let mut link = Some(p.rc.park_for_personal_control(transfer).unwrap());
    // A fixed-control API negative: no fake holder or ACK. Proof preparation
    // refuses a wrong runtime while borrowing the genuine returned owner.
    let mut bytes = Frame::FreezeFence { fence }.encode().unwrap();
    bytes[4 + 16 + 16 + 32 + 32] ^= 1;
    let Frame::FreezeFence { fence: wrong } = Frame::decode(&bytes).unwrap() else {
        unreachable!()
    };
    assert!(
        p.dst
            .as_mut()
            .unwrap()
            .prepare_parked(&wrong, &mut link, &p.rc, now())
            .is_err()
    );
    let (transport, _) = link
        .take()
        .expect("original owner still retained after refusal")
        .into_parts();
    p.rc.restore_transport(transport, std::collections::VecDeque::new());
    p.rc.cancel_handover_drain(transfer);
    assert!(!p.rc.is_parked());
    let after =
        p.rc.personal_inbox
            .as_ref()
            .unwrap()
            .installed_identity(p.rc.applied_window_event())
            .unwrap();
    assert!(
        before.same_attachment(&after),
        "actual same-origin Inbox remains installed"
    );
    p.rc.send_raw(&handover_frame(
        &Frame::Abort {
            transfer,
            aborted: None,
        }
        .encode()
        .unwrap(),
    ));
    assert!(
        p.src.pump(&mut p.hs, p.live, now()).is_err(),
        "the same restored normal transport sends real Abort"
    );
    assert!(!p.hs.handover_frozen());
}

#[test]
fn ordered_park_ordinary_after_fence_is_raw_until_held_refusal_and_exact_replay() {
    let mut p = pair("park-late-ordinary");
    awaiting(&mut p);
    queued_actual_park(&mut p);
    // Explicit adverse ordinary control, not a claim of accepted gameplay.
    // A valid Ping must not cross the strict held Personal-only boundary.
    let wire = protocol::serialize_packet(PacketType::Ping, &());
    p.hs.handover_send_park(&wire).unwrap();
    let dst = p.dst.as_mut().unwrap();
    dst.pump(now()).unwrap();
    let error = dst
        .pump_personal_control(
            &mut p.rc,
            &mut p.application.window,
            &mut p.application.slot,
            (0, 0),
            now(),
        )
        .unwrap_err();
    assert_eq!(
        error.0,
        "Gameplay arrived while the handover was waiting for personal state."
    );
    let (transport, replay) = dst.return_link().unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay.front().unwrap(), &wire);
    let allocation = replay.front().unwrap().as_ptr();
    p.rc.restore_transport(transport, replay);
    assert_eq!(
        p.rc.test_handover_replay().next().unwrap().as_ptr(),
        allocation
    );
    assert!(!p.rc.is_parked());
}

#[test]
fn ordered_park_raw_selected_input_survives_proof_extraction_and_precommit_busy_refusal() {
    let mut p = pair("park-busy-raw");
    for _ in 0..2 {
        let position = p.hs.server.players[p.seat.slot].player.pos;
        p.rc.send_input(&protocol::InputPacket {
            x: position.x,
            y: position.y,
            z: position.z,
            health: p.hs.server.players[p.seat.slot].combat.health,
            ..Default::default()
        })
        .unwrap();
        p.host.lend_tick(&mut p.hs);
        p.application.receive(&mut p.rc, &p.hs.server.registry);
    }
    awaiting(&mut p);
    let before = p.hs.server.players[p.seat.slot].last_input_tick;
    let position = p.hs.server.players[p.seat.slot].player.pos;
    let sequence =
        p.rc.send_input(&protocol::InputPacket {
            x: position.x,
            y: position.y,
            z: position.z,
            ..Default::default()
        })
        .expect("real normal transport still owns this adverse raw send");
    assert!(
        sequence > before,
        "this cannot pass on the sentinel or an old input"
    );
    queued_actual_park(&mut p);
    p.src.pump(&mut p.hs, p.live, now()).unwrap();
    assert_eq!(
        p.src.stage(),
        SourceStage::Frozen,
        "actual valid park was extracted past raw input"
    );
    assert_eq!(
        p.hs.server.players[p.seat.slot].last_input_tick, before,
        "raw receipt is not accepted gameplay"
    );
    assert!(
        p.hs.handover_park_precommit().is_err(),
        "real raw work blocks the current prepare boundary"
    );
    assert_eq!(p.hs.server.players[p.seat.slot].last_input_tick, before);
    p.src.abort(&mut p.hs, "component busy refusal");
    assert!(!p.hs.handover_frozen());
    p.host.lend_tick(&mut p.hs);
    assert_eq!(p.hs.server.players[p.seat.slot].last_input_tick, sequence);
    p.host.lend_tick(&mut p.hs);
    assert_eq!(
        p.hs.server.players[p.seat.slot].last_input_tick, sequence,
        "resume accepts that real input once"
    );
}

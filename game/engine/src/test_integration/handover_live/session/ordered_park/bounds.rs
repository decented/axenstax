//! Actual carrier/selection ownership bounds; no fabricated application progress.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn ordered_park_oversized_ordinary_carrier_refuses_before_normal_dispatch() {
    let mut p = pair("park-wire-limit");
    awaiting(&mut p);
    p.park_fence_fault
        .store(5, std::sync::atomic::Ordering::Relaxed);
    let transfer = p.src.transfer();
    assert!(p.rc.latest_state.is_none());
    p.application.receive(&mut p.rc, &p.hs.server.registry);
    assert_eq!(
        p.rc.handover_drain_failure(transfer),
        Some("The host sent an oversized packet during handover.")
    );
    let front = p.rc.test_handover_replay().next().unwrap();
    assert_eq!(front.len(), protocol::MAX_WIRE_PACKET_LEN + 1);
    assert_eq!(
        protocol::deserialize_header(front).unwrap().0,
        PacketType::StateUpdate
    );
    assert!(
        p.rc.latest_state.is_none(),
        "no oversized state was admitted to normal decoding/application"
    );
    assert!(!p.rc.is_parked());
}

#[test]
fn ordered_park_source_refuses_short_valid_proof_with_excess_actual_carrier_capacity() {
    let mut p = pair("park-proof-capacity");
    awaiting(&mut p);
    p.hs.handover_test_overallocate_fixed_controls(p.seat.slot);
    queued_actual_park(&mut p);
    let error = p.src.pump(&mut p.hs, p.live, now()).unwrap_err();
    assert_eq!(error.0, "The incoming handover control exceeded its limit.");
    assert!(!p.src.test_current_cut_committed());
    assert!(!p.src.wants_capture());
    assert!(!p.hs.handover_frozen());
    let proof = p.hs.handover_test_inbound_front(p.seat.slot).unwrap();
    assert_eq!(proof.len(), 133);
    assert!(proof.capacity() > crate::handover::live::park::WIRE_CAP);
    assert!(
        matches!(Frame::decode(&proof[1..]), Ok(Frame::Parked { .. })),
        "the signature control remains structurally valid and raw after refusal"
    );
}

fn trace(p: &Pair) -> Vec<(usize, usize, usize, [u8; 32])> {
    p.hs.handover_test_inbound_packets(p.seat.slot)
        .map(|packet| {
            (
                packet.as_ptr() as usize,
                packet.len(),
                packet.capacity(),
                Sha256::digest(packet).into(),
            )
        })
        .collect()
}

#[test]
fn ordered_park_source_batch_limit_preserves_actual_ordinary_prefix_and_repeated_control_remainder()
{
    let mut p = pair("park-batch-fifo");
    awaiting(&mut p);
    for _ in 0..3 {
        let body = &p.hs.server.players[p.seat.slot];
        p.rc.send_input(&protocol::InputPacket {
            x: body.player.pos.x,
            y: body.player.pos.y,
            z: body.player.pos.z,
            health: body.combat.health,
            ..Default::default()
        })
        .unwrap();
    }
    queued_actual_park(&mut p);
    // Inspect the genuine proof once, then repeat that fixed signed metadata
    // through the actual held owner. No Personal body or ACK is synthesised.
    let first = p.hs.handover_take_source_frames(512).unwrap();
    assert_eq!(first.len(), 1);
    let proof = Frame::decode(&first[0]).unwrap();
    assert!(matches!(proof, Frame::Parked { .. }));
    for _ in 0..50 {
        p.dst.as_ref().unwrap().test_send(&proof);
    }
    assert!(p.hs.handover_park_precommit().is_err()); // Actual bounded fill, no mutation.
    let before = trace(&p);
    assert_eq!(before.len(), 53);
    let old_bytes = p.hs.handover_test_inbound_bytes(p.seat.slot);
    let batch = p.hs.handover_take_source_frames(512).unwrap();
    assert_eq!(batch.len(), 16);
    assert!(
        batch
            .iter()
            .all(|bytes| Frame::decode(bytes).unwrap() == proof)
    );
    assert!(
        batch
            .iter()
            .all(|bytes| bytes.capacity() <= crate::handover::live::park::WIRE_CAP)
    );
    let after = trace(&p);
    let expected: Vec<_> = before[..3].iter().chain(&before[19..]).copied().collect();
    assert_eq!(
        after, expected,
        "the full original span keeps ordinary and unselected control allocations/FIFO"
    );
    assert_eq!(
        old_bytes - p.hs.handover_test_inbound_bytes(p.seat.slot),
        16 * (133 + crate::transport::INBOUND_ENTRY_OVERHEAD)
    );
    assert!(!p.src.test_current_cut_committed());
    assert_eq!(
        p.src.stage(),
        SourceStage::AwaitingParked,
        "batch extraction itself grants no phase authority"
    );
}

#[test]
fn ordered_park_capacity_failure_in_routed_batch_precedes_accept_phase_transition() {
    let mut other = pair("park-other-proof");
    awaiting(&mut other);
    queued_actual_park(&mut other);
    let proof = other.hs.handover_take_source_frames(512).unwrap().remove(0);
    let mut p = pair("park-accept-batch-capacity");
    consent_only(&mut p);
    p.hs.handover_test_overallocate_fixed_controls(p.seat.slot);
    p.rc.send_raw(&handover_frame(&proof));
    p.host.lend_tick(&mut p.hs); // Actual routed Accept then adverse fixed control.
    let error = p.src.pump(&mut p.hs, p.live, now()).unwrap_err();
    assert_eq!(error.0, "the incoming handover control exceeded its limit");
    assert!(!p.src.test_current_cut_committed());
    assert!(!p.hs.handover_frozen());
}

#[test]
fn ordered_park_repeated_invalid_signatures_are_bounded_before_any_proof_transition() {
    let mut p = pair("park-malformed-proof-batch");
    awaiting(&mut p);
    queued_actual_park(&mut p);
    let mut bytes = p.hs.handover_take_source_frames(512).unwrap().remove(0);
    *bytes.last_mut().unwrap() ^= 1;
    let malformed = Frame::decode(&bytes).unwrap();
    assert!(
        matches!(malformed, Frame::Parked { .. }),
        "fixed codec is valid; the actual signature is adverse"
    );
    for _ in 0..50 {
        p.dst.as_ref().unwrap().test_send(&malformed);
    }
    let error = p.src.pump(&mut p.hs, p.live, now()).unwrap_err();
    assert_eq!(
        error.0,
        "The handover no longer matches this joined connection."
    );
    assert!(!p.src.test_current_cut_committed());
    assert!(!p.hs.handover_frozen());
    assert_eq!(
        p.hs.handover_test_inbound_packets(p.seat.slot).count(),
        34,
        "only the bounded actual batch left the raw FIFO"
    );
}

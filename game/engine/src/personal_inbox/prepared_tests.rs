//! Bounded codec/transaction fixtures, not quest activity or handover acceptance.
use super::*;
use crate::{
    player_slot::PlayerSlot,
    progression::{GameplayContext, GameplayRecord},
};

fn snapshot(revision: u64, event: u32, life: u32) -> PersonalSnapshot {
    let context = GameplayContext::Guest {
        session: [7; 16],
        generation: 1,
    };
    let mut record = GameplayRecord::empty(context).unwrap();
    record.revision = revision;
    PersonalSnapshot {
        session: [7; 16],
        life,
        revision,
        window_event: event,
        context,
        bytes: record.encode().unwrap(),
    }
}
fn slot() -> PlayerSlot {
    PlayerSlot::new(0, glam::Vec3::ZERO, 1.0)
}

#[test]
fn d2c3_prepared_personal_admission_abandon_keeps_pending_and_context() {
    let mut inbox = Inbox::new([7; 16], 0);
    let capacity;
    {
        let mut push = inbox.prepare_push(snapshot(1, 2, 0)).unwrap();
        let proposed = push.fence_identity(1).unwrap().unwrap();
        assert_eq!(
            (proposed.base(), proposed.event(), proposed.life()),
            (1, 2, 0)
        );
        assert!(push.fence_identity(2).is_err());
        capacity = push.retained_usage().1;
    }
    assert!(capacity >= inbox.owned_bytes());
    assert_eq!(inbox.pending_count(), 0);
    assert!(inbox.context.is_none());
    assert!(inbox.last.is_none());
}
#[test]
fn d2c3_prepared_personal_proof_keeps_actual_base_and_distinct_inbox_origin() {
    let mut a = Inbox::new([7; 16], 0);
    let mut b = Inbox::new([7; 16], 0);
    let mut push = a.prepare_push(snapshot(1, 3, 0)).unwrap();
    let reserved = push.fence_identity(2).unwrap().unwrap();
    let PersonalAdmission::Admitted(admitted) = push.commit() else {
        panic!("actual admission")
    };
    assert!(admitted.identity().same_snapshot(&reserved));
    assert!(reserved.matches_inbox(&a, 2));
    b.push(snapshot(1, 3, 0)).unwrap();
    assert!(!reserved.matches_inbox(&b, 2));
    assert!(!reserved.matches_inbox(&a, 1));
    assert!(!reserved.same_namespace(&b.pending_fence_identity(3, 0, 2).unwrap()));
}
#[test]
fn d2c3_prepared_personal_normal_marker_uses_snapshot_observed_life() {
    let mut inbox = Inbox::new([7; 16], 0);
    inbox.observe_respawn(1).unwrap();
    let mut push = inbox.prepare_push(snapshot(1, 2, 1)).unwrap();
    let identity = push.fence_identity(1).unwrap().unwrap();
    assert_eq!(
        identity.life(),
        1,
        "normal wire marker names the snapshot's actual actor life"
    );
    let PersonalAdmission::Admitted(proof) = push.commit() else {
        panic!("actual admission")
    };
    assert_eq!(proof.identity().life(), 1);
    assert_eq!(
        inbox.life, 0,
        "socket observation did not apply the physical respawn"
    );
    assert!(inbox.apply(2, &mut slot()).unwrap().is_empty());
    assert_eq!(inbox.pending_count(), 1);
}
#[test]
fn d2c3_prepared_personal_decode_failure_preserves_pending_and_holder() {
    let mut inbox = Inbox::new([7; 16], 0);
    let mut player = slot();
    inbox.push(snapshot(1, 2, 0)).unwrap();
    let initial = inbox.apply(2, &mut player).unwrap();
    assert_eq!(initial.len(), 1);
    inbox.push(snapshot(2, 3, 0)).unwrap();
    // Deliberate corrupt-body fault after valid admission; no gameplay is claimed.
    inbox.pending.front_mut().unwrap().bytes.truncate(3);
    let before = inbox.pending.front().unwrap().clone();
    let last = inbox.last;
    assert!(inbox.prepare_apply(3, &mut player, (0, 0)).is_err());
    assert_eq!(inbox.pending.front(), Some(&before));
    assert_eq!(inbox.last, last);
    assert_eq!(player.gameplay_revision, 1);
}
#[test]
fn d2c3_prepared_personal_apply_abandon_has_no_install_or_ack() {
    let mut inbox = Inbox::new([7; 16], 0);
    let mut player = slot();
    inbox.push(snapshot(1, 2, 0)).unwrap();
    let before = inbox.pending.front().unwrap().clone();
    let old_revision = player.gameplay_revision;
    let prepared = inbox.prepare_apply(2, &mut player, (0, 0)).unwrap();
    assert_eq!(prepared.count(), 1);
    drop(prepared);
    assert_eq!(inbox.pending.front(), Some(&before));
    assert_eq!(player.gameplay_revision, old_revision);
    assert!(inbox.last.is_none());
    let batch = inbox
        .prepare_apply(2, &mut player, (0, 0))
        .unwrap()
        .commit();
    assert_eq!(player.gameplay_revision, 1);
    assert_eq!(inbox.pending_count(), 0);
    assert!(batch.matches_inbox(&inbox));
    let ack = batch.last_ack().unwrap();
    let wire = batch.wires().next().unwrap();
    let (kind, payload) = crate::protocol::deserialize_header(wire).unwrap();
    assert_eq!(kind, crate::protocol::PacketType::PersonalSync);
    assert_eq!(
        crate::protocol::safe_deserialize::<crate::protocol::PersonalSyncPacket>(payload).unwrap(),
        crate::protocol::PersonalSyncPacket::Applied(ack)
    );
    let PersonalAdmission::DuplicateApplied(duplicate) =
        inbox.prepare_push(before).unwrap().commit()
    else {
        panic!("exact actual installed duplicate")
    };
    assert_eq!(duplicate.last_ack(), Some(ack));
    assert!(duplicate.matches_inbox(&inbox));
}
#[test]
fn d2c3_prepared_personal_capacity_refusal_preserves_body_frontier_and_ack() {
    let mut inbox = Inbox::new([7; 16], 0);
    let value = snapshot(1, 2, 0);
    let maximum = crate::action_journal::MAX_RETAINED_BYTES;
    assert!(
        inbox
            .prepare_push_bounded(value.clone(), (0, maximum))
            .is_err()
    );
    assert!(inbox.pending.is_empty() && inbox.context.is_none());
    inbox.push(value.clone()).unwrap();
    let mut player = slot();
    let before = player.gameplay_revision;
    assert!(inbox.prepare_apply(2, &mut player, (0, maximum)).is_err());
    assert_eq!(player.gameplay_revision, before);
    assert_eq!(inbox.pending.front(), Some(&value));
    assert!(inbox.last.is_none());
    inbox.apply(2, &mut player).unwrap();
    let last = inbox.last;
    assert!(inbox.prepare_push_bounded(value, (0, maximum)).is_err());
    assert_eq!(inbox.last, last);
    assert!(inbox.pending.is_empty());
}

#[test]
fn d2c3_personal_streaming_borrowed_hash_matches_nonempty_owned_wire() {
    use sha2::Digest;
    let context = GameplayContext::WorldLocal {
        scope: [4; 16],
        player: crate::progression::WorldPlayer::Npub([5; 32]),
    };
    let mut record = GameplayRecord::empty(context).unwrap();
    // Codec fixture: nonempty typed values, never a gameplay authority claim.
    record.revision = 19;
    record.reputation.push(((3, -2), -25));
    record.kills.push((crate::mob::MobType::Cow, 2));
    record.claims.push((11, 1));
    record.quests.declines.push(([6; 16], 113));
    record.quests.penalty_left = 79;
    let value = PersonalSnapshot {
        session: [7; 16],
        life: 3,
        revision: 19,
        window_event: 23,
        context,
        bytes: record.encode().unwrap(),
    };
    let expected: [u8; 32] = sha2::Sha256::digest(bincode::serialize(&value).unwrap()).into();
    assert_eq!(acknowledgement(&value).snapshot, expected);
    assert_eq!(
        acknowledgement_parts(
            value.session,
            value.life,
            value.revision,
            value.window_event,
            value.context,
            &value.bytes
        ),
        acknowledgement(&value)
    );
}

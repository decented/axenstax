use super::*;
use crate::{
    player_slot::PlayerSlot,
    progression::{GameplayContext, GameplayRecord},
    protocol::PersonalSnapshot,
};

fn body(event: u32, revision: u64) -> PersonalSnapshot {
    let context = GameplayContext::Guest { session: [8; 16], generation: 1 };
    let mut record = GameplayRecord::empty(context).unwrap();
    record.revision = revision;
    PersonalSnapshot {
        session: [8; 16],
        life: 0,
        revision,
        window_event: event,
        context,
        bytes: record.encode().unwrap(),
    }
}
fn established() -> Inbox {
    let mut inbox = Inbox::new([8; 16], 0);
    inbox.push(body(1, 1)).unwrap();
    let mut slot = PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    assert_eq!(inbox.apply(1, &mut slot).unwrap().len(), 1);
    assert!(inbox.initialised());
    inbox
}
fn admit(window: &mut WindowInbox, inbox: &mut Inbox, event: u32) {
    let mut push = inbox.prepare_push(body(event, u64::from(event))).unwrap();
    let identity = push.fence_identity(1).unwrap().unwrap();
    window.prepare_personal_fence(&identity, push.retained_usage()).unwrap().pair(push).unwrap().commit();
}
#[test]
fn d2c3_personal_marker_pair_rejects_another_actual_inbox_before_either_publication() {
    let mut first = established();
    let mut second = established();
    let mut window = WindowInbox::default();
    let mut push = first.prepare_push(body(2, 2)).unwrap();
    let identity = push.fence_identity(1).unwrap().unwrap();
    drop(push);
    let mut foreign = second.prepare_push(body(2, 2)).unwrap();
    foreign.fence_identity(1).unwrap();
    assert!(window.prepare_personal_fence(&identity, foreign.retained_usage()).unwrap().pair(foreign).is_err());
    assert_eq!(first.pending_count(), 0);
    assert_eq!(second.pending_count(), 0);
    assert!(window.outcomes.is_empty());
}
#[test]
fn d2c3_personal_prefix_reserves_exact_same_inbox_bodies_and_abandons_without_consuming() {
    let mut inbox = established();
    let mut window = WindowInbox::default();
    admit(&mut window, &mut inbox, 2);
    admit(&mut window, &mut inbox, 3);
    let prepared = window.prepare_personal_prefix(1, 0, &inbox, 8).unwrap();
    assert_eq!((prepared.base(), prepared.through(), prepared.life()), (1, 3, 0));
    assert_eq!(prepared.identities().len(), 2);
    assert!(prepared.owned_bytes() >= 2 * size_of::<PersonalFenceIdentity>());
    drop(prepared);
    assert_eq!(window.outcomes.len(), 2);
    assert_eq!(inbox.pending_count(), 2);
    let prepared = window.prepare_personal_prefix(1, 0, &inbox, 8).unwrap();
    assert_eq!(prepared.identities().len(), 2);
    prepared.commit();
    assert!(window.outcomes.is_empty());
    assert_eq!(inbox.pending_count(), 2, "only SAME-Inbox prepared application may consume a body");
}
#[test]
fn d2c3_personal_prefix_refuses_duplicate_gap_life_and_ordinary_work_without_drain() {
    let mut inbox = established();
    let mut window = WindowInbox::default();
    admit(&mut window, &mut inbox, 2);
    window.outcomes.push(RequestOutcome::PersonalSync { window_event: 2, actor_respawns: 0 });
    assert!(matches!(
        window.prepare_personal_prefix(1, 0, &inbox, 8),
        Err(PersonalPrefixBlocked::Gap | PersonalPrefixBlocked::Missing)
    ));
    assert_eq!(window.outcomes.len(), 2);
    window.outcomes.pop();
    assert!(matches!(
        window.prepare_personal_prefix(0, 0, &inbox, 8),
        Err(PersonalPrefixBlocked::Gap | PersonalPrefixBlocked::Missing)
    ));
    assert!(matches!(window.prepare_personal_prefix(1, 1, &inbox, 8), Err(PersonalPrefixBlocked::Life)));
    window.outcomes.push(RequestOutcome::PersonalSync { window_event: 4, actor_respawns: 0 });
    assert!(matches!(
        window.prepare_personal_prefix(1, 0, &inbox, 8),
        Err(PersonalPrefixBlocked::Gap | PersonalPrefixBlocked::Missing)
    ));
    window.outcomes.pop();
    window.ack = Some(1);
    assert!(matches!(window.prepare_personal_prefix(1, 0, &inbox, 8), Err(PersonalPrefixBlocked::Ordinary)));
    assert_eq!(window.outcomes.len(), 1);
    assert_eq!(inbox.pending_count(), 1);
}
#[test]
fn d2c3_personal_marker_budget_failure_keeps_body_and_number_unpublished() {
    let mut inbox = established();
    let mut window = WindowInbox::default();
    let mut push = inbox.prepare_push(body(2, 2)).unwrap();
    let identity = push.fence_identity(1).unwrap().unwrap();
    assert!(window.prepare_personal_fence(&identity, (crate::action_journal::MAX_EVENT_ENTRIES, 0)).is_err());
    drop(push);
    assert_eq!(inbox.pending_count(), 0);
    assert!(window.outcomes.is_empty());
}

#[test]
fn d2c3_personal_published_marker_retains_actual_inbox_origin_and_normal_drain_releases_it() {
    let mut actual = established();
    let mut foreign = established();
    let mut window = WindowInbox::default();
    admit(&mut window, &mut actual, 2);
    foreign.push(body(2, 2)).unwrap();
    assert!(matches!(window.prepare_personal_prefix(1, 0, &foreign, 8), Err(PersonalPrefixBlocked::Missing)));
    assert_eq!(window.outcomes.len(), 1);
    assert_eq!(window.personal_fences.len(), 1);
    assert!(window.prepare_personal_prefix(1, 0, &actual, 8).is_ok());
    let (_ordered, _) = window.take_ordered();
    assert!(window.personal_fences.is_empty());
    assert_eq!(actual.pending_count(), 1, "normal application still owns actual pending body");
}

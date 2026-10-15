//! Truthful pre-begin boundary for the later retained-source acceptance path.
use super::*;
use crate::test_integration::handover_carry_fixture as activity;

#[test]
#[ignore = "needs two real GameState GPU adapters and signed quest/menu setup"]
fn game_harness_d2c3_retained_source_preserves_active_own_quests_and_current_timers() {
    let _worlds = crate::save::WorldsRootGuard::new("active-retained-boundary");
    let dir = Tmp::new("active-retained-boundary");
    let mut source = activity::active_source("active-retained-source", [SOURCE_PERSONA; 32]);
    let mut peer = HeadlessGame::boot_into_world("active-retained-peer");
    activity::start_lent(&mut source);
    let client = source.state.hosted_server.as_mut().unwrap().attach_test_remote_with_binding(Some(BINDING));
    activity::attach(&mut source, &mut peer, client, [RECIPIENT_PERSONA; 32], key(SOURCE_PERSONA), key(SOURCE_RUNTIME));
    let seat = activity::active_recipient(&mut source, &mut peer);
    assert!(seat.2 == key(RECIPIENT_PERSONA), "actual recipient npub differs");
    activity::assert_boundary_ready(&source, &peer, seat);
    assert!(source.state.handover.is_none());
    let before = activity::boundary(&source, &peer, seat);
    let context = root();
    let prefix = fresh_prefix(&context);
    let result = source.state.begin_handover_with(
        seat,
        SourceEnv {
            runtime: keys(SOURCE_RUNTIME),
            root: context.clone(),
            scratch: dir.0.join("scratch"),
            bearer: bearer(),
            attestation: source_attestation(),
            continuation: None,
        },
        now(),
    );
    eprintln!("active-retained first begin result={result:?} slot={} generation={}", seat.0, seat.1);
    if result.is_err() {
        assert!(source.state.handover.is_none());
        assert!(
            activity::boundary(&source, &peer, seat) == before,
            "refused pre-retirement boundary changed source authority or live state"
        );
        assert_eq!(fresh_prefix(&context), prefix, "refused begin changed the fresh independent root prefix");
    }
    // FIRST OWN: activity was created BEFORE this actual begin, unlike the
    // original frozen_after callback. No retained/Imported proof is fabricated.
    assert!(result.is_ok(), "genuine active source must reach the retained-copy transfer boundary: {result:?}");
    // No retirement or retained copy exists at this first boundary. Save,
    // Discard, full-folder/fault/retry tails are unimplemented at checkpoint34.
}

fn fresh_prefix(context: &RootContext) -> crate::handover::certificate::Prefix {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(crate::handover::live::continuation::lookup(
            context.clone(),
            bearer(),
            None,
            now(),
            &std::sync::atomic::AtomicBool::new(false),
        ))
        .unwrap()
        .prefix
}

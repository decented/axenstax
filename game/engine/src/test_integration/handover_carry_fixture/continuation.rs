//! Actual two-controller continuation. No manual Ready, Applied or Imported
//! authority is used; only the explicit UI consent fixture is armed.
use super::*;
use crate::{
    handover::{
        durable::{MarkerKind, MarkerStore},
        live::{recipient::RecipientStage, source::SourceStage},
    },
    handover_game::{Handover, UiAction},
};

pub(in crate::test_integration) struct Completed {
    pub working: PathBuf,
    pub retained: String,
    pub transfer: [u8; 16],
    pub scope: [u8; 16],
    pub source: GameplayRecord,
    pub recipient: GameplayRecord,
    pub cut_tick: u64,
}
fn source_stage(game: &HeadlessGame) -> Option<SourceStage> {
    match &game.state.handover {
        Some(Handover::Source { session, .. }) => Some(session.stage()),
        _ => None,
    }
}
fn round(source: &mut HeadlessGame, peer: &mut HeadlessGame) {
    source.hosted_ticks(1);
    source.frames(1);
    peer.frames(1);
}
fn projected(record: &GameplayRecord, elapsed: u64) -> GameplayRecord {
    let (mut next, expired) = crate::quest_play::project_timers(record, elapsed);
    if expired {
        next.revision = next.revision.checked_add(1).unwrap();
    }
    next
}
pub(in crate::test_integration) fn assert_progress(actual: &GameplayRecord, expected: &GameplayRecord, elapsed: u64) {
    let next = projected(expected, elapsed);
    assert_eq!(actual.quests, next.quests, "exact frozen terms/baselines and honest remaining timers");
    assert_eq!(actual.reputation, next.reputation);
    assert_eq!(actual.kills, next.kills);
    assert_eq!(actual.claims, next.claims);
    assert_eq!(actual.revision, next.revision, "one revision for an actual expiry, never for delivery");
    assert_eq!(actual.tracking, next.tracking);
}
pub(in crate::test_integration) fn transfer(
    source: &mut HeadlessGame,
    peer: &mut HeadlessGame,
    seat: Seat,
    now: fn() -> u64,
) -> Completed {
    transfer_with_custody(source, peer, seat, now, None, false)
}
/// The physical fixture has its own witnesses; the ordinary no-Plan baseline
/// never supplies them. Closing at Prepared exercises the real durable wait.
pub(in crate::test_integration) fn transfer_with_custody(
    source: &mut HeadlessGame,
    peer: &mut HeadlessGame,
    seat: Seat,
    now: fn() -> u64,
    custody: Option<&custody::Custody>,
    close_prepared: bool,
) -> Completed {
    let mut closed_prepared = false;
    let deadline = Instant::now() + Duration::from_secs(240);
    loop {
        if matches!(&peer.state.handover, Some(Handover::Recipient(s)) if s.stage() == RecipientStage::AwaitingConsent)
        {
            break;
        }
        round(source, peer);
        assert!(Instant::now() < deadline, "real proposal never reached consent: {:?}", source_stage(source));
    }
    let sp = &source.state.hosted_server.as_ref().unwrap().server.players[seat.0];
    assert!(sp.personal_delivery.current(
        sp.personal.as_ref().unwrap(),
        sp.action_frontier.as_ref().unwrap().header(0, 0),
        &sp.window_events
    ));
    assert!(peer.state.remote_client.as_ref().unwrap().handover_authority_settled());
    assert!(peer.state.window_inbox.is_empty());
    // Drain the genuine final pre-consent movement/control packets normally.
    // The UI action then parks that SAME established transport.
    source.hosted_ticks(1);
    source.frames(1);
    peer.state.handover_consent_shown = Some(web_time::Instant::now() - Duration::from_secs(1));
    peer.state.handover_click = Some(UiAction::Accept);
    peer.frames(1);
    assert!(matches!(&peer.state.handover, Some(Handover::Recipient(s)) if s.stage() == RecipientStage::Accepted));
    assert!(peer.state.remote_client.as_ref().unwrap().is_parked());
    let actual_binding = peer.state.players[0].personal_binding.clone();
    // This is an actual paused joined client. The narrow handover pump must
    // apply the real current snapshot and send its returned Inbox ACK here.
    peer.state.mode = crate::GameMode::Paused { confirm_quit: false, confirm_creative: false };
    let before_native = native(source);
    let before_remote = remote_record(source, seat);
    let before_tick = source.state.tick_counter;
    let scope = source.state.world.quest_scope.unwrap();
    let mut cut = None;
    let mut observed_working = None;
    loop {
        source.hosted_ticks(1);
        source.frames(1);
        if source_stage(source) == Some(SourceStage::Frozen) {
            let hs = source.state.hosted_server.as_ref().unwrap();
            let sp = &hs.server.players[seat.0];
            let current = sp.personal.as_ref().unwrap().record().unwrap();
            assert_progress(&native(source), &before_native, source.state.tick_counter - before_tick);
            assert_progress(&current, &before_remote, source.state.tick_counter - before_tick);
            assert!(sp.personal_delivery.pending.is_some(), "real nonzero timers require a fresh current snapshot");
            assert!(!sp.window_events.is_quiescent());
            if let Some(custody) = custody {
                custody.assert_unchanged(source, peer);
            }
            let tick = source.state.tick_counter;
            let local = native(source);
            let old = peer.state.remote_client.as_ref().unwrap().applied_window_event();
            let expected = sp.personal_delivery.pending.as_ref().map(crate::personal_sync::acknowledgement).unwrap();
            assert!(expected.window_event > old);
            let context = sp.personal.as_ref().unwrap().binding.context();
            for _ in 0..3 {
                source.hosted_ticks(1);
                source.frames(1);
                assert_eq!(source.state.tick_counter, tick);
                assert_eq!(native(source), local, "frozen wall time never consumes common timers");
                assert_eq!(source_stage(source), Some(SourceStage::Frozen));
            }
            peer.frames(1);
            assert!(matches!(peer.state.mode, crate::GameMode::Paused { .. }));
            assert_eq!(
                peer.state.players[0].personal_binding, actual_binding,
                "joined binding is observed, never invented"
            );
            assert_eq!(peer.state.remote_client.as_ref().unwrap().applied_window_event(), expected.window_event);
            let installed = GameplayRecord::from_slot(context, &peer.state.players[0]).unwrap();
            assert_progress(&installed, &current, 0);
            if let Some(custody) = custody {
                custody.assert_unchanged(source, peer);
            }
            // Until the source reads the actual held-link ACK, capture stays
            // blocked despite the recipient's genuine local installation.
            assert_eq!(source_stage(source), Some(SourceStage::Frozen));
            cut = Some((tick, local, current));
        } else if !closed_prepared {
            peer.frames(1);
        }
        if let Some(Handover::Recipient(s)) = &peer.state.handover {
            if let Some(checkpoint) = s.checkpoint() {
                observed_working = Some(checkpoint.working_world());
            }
        }
        if close_prepared
            && !closed_prepared
            && matches!(&peer.state.handover, Some(Handover::Recipient(s)) if s.stage() == RecipientStage::Prepared)
        {
            if let Some(custody) = custody {
                custody.assert_unchanged(source, peer);
            }
            assert!(peer.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
            assert!(peer.state.handover.is_none());
            assert!(matches!(peer.state.mode, crate::GameMode::Menu(_)));
            closed_prepared = true;
        }
        if source_stage(source) == Some(SourceStage::Retired) {
            break;
        }
        if let Some(SourceStage::Aborted(why)) = source_stage(source) {
            panic!("real active carry aborted: {why}");
        }
        assert!(
            source.state.handover.is_some(),
            "actual source ended before retirement: {:?}",
            source.state.toast.as_ref().map(|(text, _)| text)
        );
        assert!(Instant::now() < deadline, "actual active transfer did not retire: {:?}", source_stage(source));
        std::thread::sleep(Duration::from_millis(5));
    }
    let (cut_tick, source_record, recipient) = cut.expect("the fresh current fence was observed before capture");
    let Some(Handover::Source { session, .. }) = &source.state.handover else {
        panic!("real source session disappeared");
    };
    let transfer = session.transfer();
    let retained = session.retained_folder().expect("actual source retirement released its retained copy").to_owned();
    let working = observed_working.expect("the actual durable checkpoint was observed before activation");
    let store = MarkerStore::open_existing(working.join("handover")).unwrap();
    while store.read(MarkerKind::Activated, transfer).unwrap().is_none() {
        source.hosted_ticks(1);
        source.frames(1);
        if !closed_prepared {
            peer.frames(1);
        }
        assert!(Instant::now() < deadline, "real durable Prepared/Committed never activated");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!close_prepared || closed_prepared, "actual Prepared close boundary was not reached");
    let proof = crate::handover::live::personal_adapter::proof(&working).unwrap().unwrap();
    assert_eq!(proof.core.transfer, transfer);
    assert_eq!(
        crate::save::read_world_save(&std::fs::read(working.join("world.dat")).unwrap())
            .unwrap()
            .quest_world
            .unwrap()
            .scope,
        scope
    );
    let retained_proof =
        crate::handover::live::personal_adapter::proof(&crate::save::world_dir(&retained)).unwrap().unwrap();
    assert!(retained_proof.is_retained());
    assert_eq!(retained_proof.core, proof.core);
    Completed { working, retained, transfer, scope, source: source_record, recipient, cut_tick }
}
/// Cancel after BOTH current holders were committed, while the real recipient
/// has not applied the fresh fence. Abort resumes from that cut, never old RAM.
pub(in crate::test_integration) fn abort_current_with_custody(
    source: &mut HeadlessGame,
    peer: &mut HeadlessGame,
    seat: Seat,
    custody: &custody::Custody,
) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while !matches!(&peer.state.handover, Some(Handover::Recipient(s)) if s.stage() == RecipientStage::AwaitingConsent)
    {
        round(source, peer);
        assert!(Instant::now() < deadline);
    }
    source.hosted_ticks(1);
    source.frames(1);
    peer.state.handover_consent_shown = Some(web_time::Instant::now() - Duration::from_secs(1));
    peer.state.handover_click = Some(UiAction::Accept);
    peer.frames(1);
    assert!(peer.state.remote_client.as_ref().unwrap().is_parked());
    while source_stage(source) != Some(SourceStage::Frozen) {
        source.hosted_ticks(1);
        source.frames(1);
        assert!(Instant::now() < deadline, "real current cut did not freeze");
    }
    let tick = source.state.tick_counter;
    let local = native(source);
    let remote = remote_record(source, seat);
    let sp = &source.state.hosted_server.as_ref().unwrap().server.players[seat.0];
    assert!(sp.personal_delivery.pending.is_some());
    assert!(!sp.window_events.is_quiescent());
    custody.assert_unchanged(source, peer);
    for _ in 0..3 {
        source.hosted_ticks(1);
        source.frames(1);
        assert_eq!(source.state.tick_counter, tick);
        assert_eq!(source_stage(source), Some(SourceStage::Frozen));
        assert_eq!(native(source), local);
        custody.assert_unchanged(source, peer);
    }
    source.state.handover_click = Some(UiAction::Cancel);
    source.frames(1);
    assert!(source.state.handover.is_none());
    assert!(!source.state.hosted_server.as_ref().unwrap().handover_frozen());
    while peer.state.handover.is_some() || peer.state.remote_client.as_ref().unwrap().is_parked() {
        round(source, peer);
        assert!(Instant::now() < deadline, "actual abort did not return the SAME held link");
    }
    let current = settle(source, peer);
    assert_eq!(current, seat);
    assert_progress(&native(source), &local, source.state.tick_counter - tick);
    assert_progress(&remote_record(source, seat), &remote, source.state.tick_counter - tick);
    custody.assert_unchanged(source, peer);
    assert!(source.state.players[0].gameplay_carry_complete());
    assert!(source.state.hosted_server.as_ref().unwrap().server.players[seat.0].progression_tracked);
}
pub(in crate::test_integration) fn open(game: &mut HeadlessGame, folder: &str) {
    assert!(matches!(game.state.mode, crate::GameMode::Menu(_)), "true lobby entry, never direct Loading");
    let meta = crate::save::try_load_world_meta(folder).unwrap();
    game.state.pending_menu_action = Some((
        crate::menu::MenuAction::LoadWorld(folder.into(), crate::spawn_pref::SpawnPref::Default),
        web_time::Instant::now(),
    ));
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        game.frames(1);
        if matches!(game.state.mode, crate::GameMode::Playing) {
            break;
        }
        if let crate::GameMode::Menu(menu) = &game.state.mode {
            assert!(menu.notice.is_none(), "actual menu reopen refused: {:?}", menu.notice);
        }
        assert!(Instant::now() < deadline, "actual queued menu reopen never completed");
    }
    assert_eq!(game.state.world.total_ticks, meta.total_ticks + game.state.tick_counter);
}
pub(in crate::test_integration) fn promoted(peer: &mut HeadlessGame, transfer: [u8; 16]) -> String {
    if !matches!(peer.state.mode, crate::GameMode::Menu(_)) {
        assert!(peer.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
    }
    let marker = format!("activated-{}.dat", hex::encode(transfer));
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        peer.frames(1);
        let crate::GameMode::Menu(menu) = &peer.state.mode else {
            panic!("actual recipient left its lobby");
        };
        if let Some(world) =
            menu.worlds.iter().find(|w| crate::save::world_dir(&w.folder_name).join("handover").join(&marker).is_file())
        {
            return world.folder_name.clone();
        }
        assert!(Instant::now() < deadline, "activated import was not promoted by the real lobby");
        std::thread::sleep(Duration::from_millis(5));
    }
}
pub(in crate::test_integration) fn restored(
    game: &mut HeadlessGame,
    expected: &GameplayRecord,
    scope: [u8; 16],
    owner: [u8; 32],
) {
    assert!(!game.state.joined());
    assert_eq!(game.state.world.quest_scope, Some(scope));
    let binding = game.state.players[0].personal_binding.as_ref().unwrap();
    assert!(binding.context().participant() == Some(owner), "the actual own participant differs");
    assert!(matches!(binding, GameplayBinding::Imported(_)), "actual checked continuation installed its proof");
    let actual = native(game);
    assert_progress(&actual, expected, game.state.tick_counter);
    assert!(game.state.players[0].gameplay_carry_complete());
    for accepted in &actual.quests.accepted {
        native_setup::original_dialogue(game, accepted.npc);
    }
}

pub(in crate::test_integration) fn reconnect_progress(
    actual: &GameplayRecord,
    saved: &GameplayRecord,
    maximum_online: u64,
) {
    assert_eq!(actual.quests.accepted, saved.quests.accepted);
    assert_eq!(actual.kills, saved.kills);
    assert_eq!(actual.reputation, saved.reputation);
    assert_eq!(actual.claims, saved.claims);
    for (id, left) in &saved.quests.declines {
        let now = actual.quests.declines.iter().find(|(key, _)| key == id).map_or(0, |(_, n)| *n);
        assert!(
            now <= *left && u64::from(left - now) <= maximum_online,
            "only genuinely connected handshake/play ticks consume an offline holder's decline"
        );
    }
    assert!(actual.quests.declines.iter().all(|(id, _)| saved.quests.declines.iter().any(|(key, _)| key == id)));
    assert!(actual.quests.penalty_left <= saved.quests.penalty_left);
    assert!(u64::from(saved.quests.penalty_left - actual.quests.penalty_left) <= maximum_online);
    assert!(actual.revision >= saved.revision && actual.revision <= saved.revision.saturating_add(1));
}
#[path = "continuation/completion.rs"]
mod completion;
#[path = "continuation/dedicated.rs"]
mod dedicated;
pub(in crate::test_integration) use completion::complete_native_once;
pub(in crate::test_integration) use dedicated::complete_dedicated_and_return;

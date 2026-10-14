//! Real native preparation failure and current-authority controls.
use super::quest_preparation::*;
use super::*;
use crate::progression::GameplayRecord;

#[test]
#[ignore = "needs native autosave load and early preparation refusal"]
fn game_harness_d2c3_native_preparation_early_fence_keeps_selected_bytes_and_clock() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-precommit");
    let name = "d2c3-native-precommit";
    let mut game = HeadlessGame::boot_into_world(name);
    clear_pad(&mut game, None);
    game.state.save_live_world().unwrap();
    game.state.autosave_counter = 5999;
    game.ticks(1);
    let directory = crate::save::world_dir(name);
    assert!(directory.join("autosave").exists());
    assert!(game.state.leave_world(
        crate::world_exit::SaveChoice::Abandon,
        crate::world_exit::ExitTo::Lobby
    ));
    super::quest_native::load_native(&mut game, name);
    assert!(
        game.state.session_saves.opened_from_autosave,
        "the actual selected autosave was opened"
    );
    clear_pad(&mut game, None);
    let first_tick = kill_villager(&mut game, (9, 10));
    game.ticks(7);
    let before = live(&game);
    let old_primary = primary(&directory);
    let autosave = std::fs::read(directory.join("autosave/world.dat")).unwrap();
    let prior_saves = game.state.session_saves;
    crate::save_fence::raise(&directory);
    let prepared = game.state.prepare_local_gameplay();
    crate::save_fence::lift(&directory);
    assert!(prepared.is_err());
    assert_eq!(live(&game), before);
    assert_eq!(primary(&directory), old_primary);
    assert_eq!(
        std::fs::read(directory.join("autosave/world.dat")).unwrap(),
        autosave
    );
    assert!(
        game.state.gameplay_preparation.is_none(),
        "exact early predecessor bytes leave no provisional commit"
    );
    let mut failed = prior_saves;
    failed.note_save(false);
    assert_eq!(game.state.session_saves, failed);
    game.ticks(4);
    let before_strict = live(&game);
    let prior_strict_saves = game.state.session_saves;
    let fault = crate::fsync::tests::fail_directory_on_match(&directory, 1);
    assert!(game.state.prepare_local_gameplay().is_err());
    assert!(fault.failures() > 0);
    assert_eq!(live(&game), before_strict);
    let mut strict_failed = prior_strict_saves;
    strict_failed.note_save(false);
    assert_eq!(
        game.state.session_saves, strict_failed,
        "ordinary success before strict failure preserves actual autosave-open provenance"
    );
    assert!(game.state.session_saves.opened_from_autosave);
    assert!(
        !directory.join("autosave").exists(),
        "the actual commit superseded it; no old autosave is fabricated"
    );
    assert_ne!(primary(&directory), old_primary);
    assert!(game.state.gameplay_preparation.is_some());
    drop(fault);
    game.state.prepare_local_gameplay().unwrap();
    let (_, record) = selected_record(&directory);
    assert_eq!(
        record.quests.penalty_left,
        (600 - (game.state.tick_counter - first_tick)) as u32
    );
    assert!(
        !directory.join("autosave").exists(),
        "successful actual save superseded the older autosave"
    );
}

#[test]
#[ignore = "needs native pending identity and actual changed-file refusal"]
fn game_harness_d2c3_native_preparation_rejects_changed_signer_and_selected_file() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-pending-identity");
    let mut game = HeadlessGame::boot_into_world("d2c3-native-pending-identity");
    game.state.players[0].inventory.clear();
    let directory = crate::save::world_dir(&game.state.world_name);
    let fault = crate::fsync::tests::fail_directory_on_match(&directory, 1);
    assert!(game.state.prepare_local_gameplay().is_err());
    assert!(fault.failures() > 0);
    let before = live(&game);
    let retained = primary(&directory);
    let key = nostr::Keys::new(nostr::SecretKey::from_slice(&[87; 32]).unwrap())
        .public_key()
        .to_bytes();
    game.state.handover_fixture = Some(crate::handover_game::Fixture {
        signed_in: Some(key),
        recipient: None,
    });
    assert!(
        game.state.prepare_local_gameplay().is_err(),
        "a changed current native persona cannot adopt another pending context"
    );
    assert_eq!(live(&game), before);
    assert_eq!(primary(&directory), retained);
    game.state.handover_fixture = None;
    drop(fault);
    let foreign = b"changed selected primary";
    std::fs::write(directory.join("world.dat"), foreign).unwrap();
    assert!(game.state.prepare_local_gameplay().is_err());
    assert!(game.state.save_live_world().is_err());
    assert_eq!(primary(&directory), foreign);
    assert_eq!(live(&game), before);
    std::fs::write(directory.join("world.dat"), &retained).unwrap();
    game.state.prepare_local_gameplay().unwrap();
    assert_eq!(
        game.state.world.quest_scope,
        crate::save::read_world_save(&retained)
            .unwrap()
            .quest_world
            .map(|w| w.scope)
    );
}

#[test]
#[ignore = "needs actual abandon and selected provisional native reopen"]
fn game_harness_d2c3_native_preparation_reopen_uses_the_retained_actual_candidate() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-pending-reopen");
    let name = "d2c3-native-pending-reopen";
    let mut game = HeadlessGame::boot_into_world(name);
    game.state.players[0].inventory.clear();
    let directory = crate::save::world_dir(name);
    let fault = crate::fsync::tests::fail_directory_on_match(&directory, 1);
    assert!(game.state.prepare_local_gameplay().is_err());
    assert!(fault.failures() > 0);
    let retained = primary(&directory);
    let scope = crate::save::read_world_save(&retained)
        .unwrap()
        .quest_world
        .unwrap()
        .scope;
    let generation = game.state.gameplay_preparation_generation;
    assert!(game.state.leave_world(
        crate::world_exit::SaveChoice::Abandon,
        crate::world_exit::ExitTo::Lobby
    ));
    assert!(game.state.gameplay_preparation.is_none());
    assert_ne!(game.state.gameplay_preparation_generation, generation);
    assert_eq!(primary(&directory), retained);
    drop(fault);
    super::quest_native::load_native(&mut game, name);
    assert!(game.state.gameplay_preparation.is_none());
    assert_eq!(game.state.world.quest_scope, Some(scope));
    assert!(game.state.players[0].personal_binding.is_some());
    assert_eq!(primary(&directory), retained);
    game.state.prepare_local_gameplay().unwrap();
}

#[test]
#[ignore = "needs first-binding future timestamp refusal without a write"]
fn game_harness_d2c3_native_first_binding_refuses_a_future_legacy_penalty_tick() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-future-penalty");
    let mut game = HeadlessGame::boot_into_world("d2c3-native-future-penalty");
    game.state.save_live_world().unwrap();
    let directory = crate::save::world_dir(&game.state.world_name);
    let old = primary(&directory);
    game.state.players[0].last_villager_kill_rep_tick = Some(game.state.tick_counter + 1);
    let before = live(&game);
    assert!(game.state.prepare_local_gameplay().is_err());
    assert_eq!(live(&game), before);
    assert_eq!(primary(&directory), old);
    assert!(game.state.gameplay_preparation.is_none());
}

#[test]
#[ignore = "needs existing native holder strict shortcut and no timer remigration"]
fn game_harness_d2c3_bound_native_preparation_confirms_directory_without_remigration() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-bound-barrier");
    let mut game = HeadlessGame::boot_into_world("d2c3-native-bound-barrier");
    clear_pad(&mut game, None);
    kill_villager(&mut game, (11, 12));
    game.state.prepare_local_gameplay().unwrap();
    game.ticks(7);
    let directory = crate::save::world_dir(&game.state.world_name);
    let before = live(&game);
    let retained = primary(&directory);
    let fault = crate::fsync::tests::fail_directory_on_match(&directory, 1);
    assert!(game.state.prepare_local_gameplay().is_err());
    assert!(fault.failures() > 0);
    assert_eq!(live(&game), before);
    assert_eq!(primary(&directory), retained);
    assert!(
        game.state.gameplay_preparation.is_none(),
        "no new candidate was written on the existing-holder shortcut"
    );
    drop(fault);
    game.state.prepare_local_gameplay().unwrap();
    assert_eq!(live(&game), before);
    let slot = &game.state.players[0];
    assert_eq!(
        GameplayRecord::from_slot(slot.personal_binding.as_ref().unwrap().context(), slot)
            .unwrap()
            .quests
            .penalty_left,
        593
    );
}

#[test]
#[ignore = "needs actual unbound penalty expiry before first common capture"]
fn game_harness_d2c3_first_native_binding_keeps_an_actually_expired_legacy_penalty_empty() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-expired-first-penalty");
    let mut game = HeadlessGame::boot_into_world("d2c3-native-expired-first-penalty");
    clear_pad(&mut game, None);
    let first = kill_villager(&mut game, (13, 14));
    game.ticks(600);
    assert!(game.state.players[0].personal_binding.is_none());
    assert_eq!(game.state.tick_counter - first, 600);
    game.state.prepare_local_gameplay().unwrap();
    let (_, record) = selected_record(&crate::save::world_dir(&game.state.world_name));
    assert_eq!(
        record.quests.penalty_left, 0,
        "an expired live legacy interval must not restart at the cut"
    );
    kill_villager(&mut game, (13, 14));
    assert_eq!(
        game.state.players[0].kill_counter[&crate::mob::MobType::Villager],
        2
    );
    assert_eq!(
        game.state.players[0].reputation.per_village[&(13, 14)],
        -50,
        "the next real death after expiry charges exactly one new penalty"
    );
    let slot = &game.state.players[0];
    assert_eq!(
        GameplayRecord::from_slot(slot.personal_binding.as_ref().unwrap().context(), slot)
            .unwrap()
            .quests
            .penalty_left,
        600
    );
}

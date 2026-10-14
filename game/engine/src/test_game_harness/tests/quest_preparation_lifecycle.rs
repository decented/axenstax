//! Real native preparation failure and current-authority controls.
use super::quest_preparation::*;
use super::*;
use crate::progression::GameplayBinding;

#[test]
#[ignore = "needs a real valid changed selected primary before the first attempt"]
fn game_harness_d2c3_first_native_preparation_refuses_a_replaced_bound_primary() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-before-first-identity");
    let name = "d2c3-native-before-first-identity";
    let mut game = HeadlessGame::boot_into_world(name);
    game.state.save_live_world().unwrap();
    let directory = crate::save::world_dir(name);
    let old = primary(&directory);
    game.state.prepare_local_gameplay().unwrap();
    let bound = primary(&directory);
    assert!(
        crate::save::read_world_save(&bound)
            .unwrap()
            .local_personal
            .is_some()
    );
    assert!(game.state.leave_world(
        crate::world_exit::SaveChoice::Abandon,
        crate::world_exit::ExitTo::Lobby
    ));
    std::fs::write(directory.join("world.dat"), &old).unwrap();
    super::quest_native::load_native(&mut game, name);
    assert!(game.state.players[0].personal_binding.is_none());
    assert!(game.state.gameplay_preparation.is_none());
    let before = live(&game);
    // Actual same-version bound bytes replace the selected file AFTER this
    // unbound session opened; its first preparation must not overwrite them.
    std::fs::write(directory.join("world.dat"), &bound).unwrap();
    assert!(game.state.prepare_local_gameplay().is_err());
    assert_eq!(live(&game), before);
    assert_eq!(primary(&directory), bound);
    assert!(
        game.state.gameplay_preparation.is_some(),
        "foreign selected authority stays guarded until checked reopen"
    );
    assert!(game.state.save_live_world().is_err());
    clear_pad(&mut game, None);
    game.state.autosave_counter = 5999;
    game.ticks(1);
    assert!(!directory.join("autosave").exists());
    assert_eq!(primary(&directory), bound);
    assert_eq!(
        game.state
            .begin_handover_with((1, 1, [86; 32]), handover_env(&directory), 1_800_000_000),
        Err(crate::gameplay_preparation::PENDING.into())
    );
    assert!(game.state.leave_world(
        crate::world_exit::SaveChoice::Abandon,
        crate::world_exit::ExitTo::Lobby
    ));
    assert_eq!(primary(&directory), bound);
    super::quest_native::load_native(&mut game, name);
    assert!(game.state.gameplay_preparation.is_none());
    assert!(game.state.players[0].personal_binding.is_some());
}

#[test]
#[ignore = "needs an actual ordinary metadata error after the first scope rename"]
fn game_harness_d2c3_native_preparation_retains_a_primary_after_ordinary_postcommit_error() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-ordinary-postcommit");
    let mut game = HeadlessGame::boot_into_world("d2c3-native-ordinary-postcommit");
    game.state.save_live_world().unwrap();
    let directory = crate::save::world_dir(&game.state.world_name);
    let old = primary(&directory);
    let before = live(&game);
    let blocked = directory.join("world_meta.json.tmp");
    std::fs::create_dir(&blocked).unwrap();
    let error = game.state.prepare_local_gameplay().unwrap_err();
    assert!(
        error.contains("world_meta.json"),
        "actual ordinary metadata failure: {error}"
    );
    assert_eq!(live(&game), before);
    let retained = primary(&directory);
    assert_ne!(
        retained, old,
        "the actual ordinary writer already renamed a candidate primary"
    );
    let (selected, _) = selected_record(&directory);
    let scope = selected.quest_world.unwrap().scope;
    assert_eq!(
        game.state.gameplay_preparation.as_ref().unwrap().scope,
        scope
    );
    assert!(game.state.save_live_world().is_err());
    assert_eq!(primary(&directory), retained);
    std::fs::remove_dir(&blocked).unwrap();
    game.state.prepare_local_gameplay().unwrap();
    assert_eq!(game.state.world.quest_scope, Some(scope));
}

#[test]
#[ignore = "needs actual native enrolment, LentSim and signed admission barriers"]
fn game_harness_d2c3_native_lent_signed_join_confirms_the_actual_prepared_holder() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-lent-scope");
    let name = "d2c3-native-lent-scope";
    let mut game = HeadlessGame::boot_into_world(name);
    clear_pad(&mut game, None);
    let host = crate::hosted_server::HostedServer::start_host(
        1,
        name.into(),
        game.state.biome_gen.seed,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        crate::hosted_server::HostWorld::Lent,
    )
    .unwrap();
    assert!(host.server.selected_local_personal.is_none());
    game.state.hosted_server = Some(host);
    game.hosted_ticks(1);
    let scope = game.state.world.quest_scope.unwrap();
    let directory = crate::save::world_dir(name);
    let (saved, _) = selected_record(&directory);
    assert_eq!(saved.quest_world.unwrap().scope, scope);
    let host = game.state.hosted_server.as_mut().unwrap();
    assert!(matches!(&host.local_gameplay_preparation, Some(Ok(()))));
    assert!(
        host.server.selected_local_personal.is_none(),
        "the native holder is never copied into the server cache"
    );
    let client = host.attach_test_remote_with_binding(Some([92; 32]));
    crate::hosted_server::tests::send_signed_join(&client, [93; 32], Some([92; 32]));
    let retained = primary(&directory);
    let before = live(&game);
    let fault = crate::fsync::tests::fail_directory_on_match(&directory, 1);
    game.hosted_ticks(1);
    let refused = join_accepts(&client);
    assert!(
        fault.failures() > 0,
        "the real signed lent admission confirmed its own directory barrier"
    );
    assert!(refused.is_empty());
    assert_eq!(game.state.world.quest_scope, before.scope);
    assert_eq!(game.state.players[0].personal_binding, before.binding);
    assert_eq!(primary(&directory), retained);
    assert!(game.state.gameplay_preparation.is_none());
    drop(fault);
    let host = game.state.hosted_server.as_mut().unwrap();
    let retry = host.attach_test_remote_with_binding(Some([94; 32]));
    crate::hosted_server::tests::send_signed_join(&retry, [93; 32], Some([94; 32]));
    game.hosted_ticks(1);
    let accepted = join_accepts(&retry);
    assert_eq!(
        accepted.len(),
        1,
        "real prepared native lending admits the signed joiner"
    );
    let host = game.state.hosted_server.as_ref().unwrap();
    assert!(host.server.selected_local_personal.is_none());
    let slot = accepted[0].player_index as usize;
    assert!(
        matches!(&host.server.players[slot].personal.as_ref().unwrap().binding,
        GameplayBinding::WorldLocal { scope: actual, .. } if *actual == scope)
    );
    let retained = primary(&directory);
    let mut missing_holder = crate::save::read_world_save(&retained).unwrap();
    missing_holder.local_personal = None;
    let omitted = crate::save_format::encode_world_save(&missing_holder).unwrap();
    std::fs::write(directory.join("world.dat"), &omitted).unwrap();
    let host = game.state.hosted_server.as_mut().unwrap();
    let omitted_join = host.attach_test_remote_with_binding(Some([95; 32]));
    crate::hosted_server::tests::send_signed_join(&omitted_join, [96; 32], Some([95; 32]));
    game.hosted_ticks(1);
    assert!(
        join_accepts(&omitted_join).is_empty(),
        "a prepared native host cannot admit a scope-only primary that omitted its real holder"
    );
    assert_eq!(primary(&directory), omitted);
    std::fs::write(directory.join("world.dat"), retained).unwrap();
}

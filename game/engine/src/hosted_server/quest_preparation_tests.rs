//! Genuine dedicated preparation retries and selected-primary preservation.
use super::*;
use std::path::Path;

fn primary(directory: &Path) -> Vec<u8> {
    std::fs::read(directory.join("world.dat")).unwrap()
}

fn scope(directory: &Path) -> [u8; 16] {
    crate::save::read_world_save(&primary(directory))
        .unwrap()
        .quest_world
        .unwrap()
        .scope
}

pub(super) fn signed_scope_failure_tail(
    host: &mut HostedServer,
    directory: &Path,
    fault: &crate::fsync::tests::DirectoryFault,
) -> [u8; 16] {
    let proposed = scope(directory);
    assert_eq!(
        host.server.gameplay_preparation.as_ref().unwrap().scope,
        proposed
    );
    let retained = primary(directory);
    assert!(host.server.try_save().is_err());
    assert!(
        host.server.try_save_selected(None).is_err(),
        "an explicit role-cut saver cannot substitute pending scope"
    );
    assert_eq!(primary(directory), retained);
    // The no-op selected-owner check must remain reachable. The scope is
    // the operator world's, independently of this different signed joiner.
    let secret = [91; 32];
    let key = nostr::Keys::new(nostr::SecretKey::from_slice(&secret).unwrap())
        .public_key()
        .to_bytes();
    let retry = host.attach_test_remote_with_binding(Some([81; 32]));
    tests::send_signed_join(&retry, secret, Some([81; 32]));
    let hits = fault.failures();
    host.tick();
    let refused = c3d_tests::hear(&retry);
    assert!(
        fault.failures() > hits,
        "the actual signed retry itself reached the persistent barrier"
    );
    assert!(refused.accepts.is_empty());
    let player = host
        .server
        .players
        .iter()
        .find(|player| player.verified_pubkey == Some(key))
        .unwrap();
    assert!(!player.is_in_world());
    assert!(player.personal.is_none());
    assert!(host.server.world.quest_scope.is_none());
    assert_eq!(scope(directory), proposed);
    assert!(host.server.try_save().is_err());
    proposed
}

pub(super) fn another_signed_player_uses_the_same_scope(
    host: &mut HostedServer,
    proposed: [u8; 16],
) {
    let secret = [91; 32];
    let retry = host.attach_test_remote_with_binding(Some([82; 32]));
    tests::send_signed_join(&retry, secret, Some([82; 32]));
    host.tick();
    let heard = c3d_tests::hear(&retry);
    assert_eq!(
        heard.accepts.len(),
        1,
        "scope preparation introduces no first-requester npub ownership policy"
    );
    let player = &host.server.players[heard.accepts[0].player_index as usize];
    assert!(matches!(&player.personal.as_ref().unwrap().binding,
        crate::progression::GameplayBinding::WorldLocal { scope, .. } if *scope == proposed));
}

#[test]
fn d2c3_dedicated_preparation_early_fence_preserves_the_selected_predecessor() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-dedicated-precommit");
    let mut host = tests::start_room_test_server("d2c3-dedicated-precommit", 0);
    host.server.try_save().unwrap();
    let directory = crate::save::world_dir(&host.server.world_name);
    let old = primary(&directory);
    assert!(host.server.world.quest_scope.is_none());
    crate::save_fence::raise(&directory);
    let prepared = host.server.prepare_signed_scope();
    crate::save_fence::lift(&directory);
    assert!(prepared.is_err());
    assert_eq!(primary(&directory), old);
    assert!(host.server.world.quest_scope.is_none());
    assert!(host.server.gameplay_preparation.is_none());
    host.server.prepare_signed_scope().unwrap();
    assert_eq!(host.server.world.quest_scope, Some(scope(&directory)));
}

#[test]
fn d2c3_dedicated_preparation_retains_a_primary_after_ordinary_postcommit_error() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-dedicated-ordinary-postcommit");
    let mut host = tests::start_room_test_server("d2c3-dedicated-ordinary-postcommit", 0);
    host.server.world.set_block(3, 64, 5, crate::block::BEDROCK);
    host.server
        .world
        .set_block(40, 64, 5, crate::block::BEDROCK);
    host.server.try_save().unwrap();
    let directory = crate::save::world_dir(&host.server.world_name);
    let old = primary(&directory);
    let blocked = directory.join("chunks/0_4_0.chunk.tmp");
    std::fs::create_dir(&blocked).unwrap();
    let error = host.server.prepare_signed_scope().unwrap_err();
    assert!(
        error.contains("world.dat written"),
        "actual ordinary post-rename failure: {error}"
    );
    assert!(host.server.world.quest_scope.is_none());
    let retained = primary(&directory);
    assert_ne!(retained, old);
    let proposed = scope(&directory);
    assert_eq!(
        host.server.gameplay_preparation.as_ref().unwrap().scope,
        proposed
    );
    assert!(host.server.try_save().is_err());
    assert_eq!(primary(&directory), retained);
    let again = host.server.prepare_signed_scope().unwrap_err();
    assert!(again.contains("world.dat written"));
    assert_eq!(scope(&directory), proposed);
    assert!(host.server.world.quest_scope.is_none());
    std::fs::remove_dir(&blocked).unwrap();
    host.server.prepare_signed_scope().unwrap();
    assert_eq!(host.server.world.quest_scope, Some(proposed));
    assert!(host.server.gameplay_preparation.is_none());
}

#[test]
fn d2c3_first_dedicated_preparation_refuses_a_replaced_scoped_primary() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-dedicated-before-first-identity");
    let mut host = tests::start_room_test_server("d2c3-dedicated-before-first-identity", 0);
    host.server.try_save().unwrap();
    let directory = crate::save::world_dir(&host.server.world_name);
    let old = primary(&directory);
    host.server.prepare_signed_scope().unwrap();
    let scoped = primary(&directory);
    let name = host.server.world_name.clone();
    drop(host);
    std::fs::write(directory.join("world.dat"), &old).unwrap();
    let mut server = crate::server::GameServer::new(0, name, 42);
    server.initial_load().unwrap();
    assert!(server.world.quest_scope.is_none());
    // A genuine current-format scoped file replaces the selected primary
    // after the live unscoped predecessor was loaded.
    std::fs::write(directory.join("world.dat"), &scoped).unwrap();
    assert!(server.prepare_signed_scope().is_err());
    assert!(server.gameplay_preparation.is_some());
    assert!(server.world.quest_scope.is_none());
    assert!(server.try_save().is_err());
    assert!(server.try_save_selected(None).is_err());
    assert_eq!(primary(&directory), scoped);
}

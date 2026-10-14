//! A local seat cannot erase the canonical correction ledger or substitute
//! another world/persona. Refusals preserve the actual file byte for byte.
use super::*;

#[test]
fn d2c3_world_local_native_return_refuses_corrupt_foreign_owed_and_incomplete_without_rewriting() {
    use crate::{
        item::Item,
        joiner_save::{BoundPersonal, IncompleteCause, JoinerSave},
    };
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-return-refusals");
    let name = "d2c3-native-return-refusals";
    let mut server = crate::server::GameServer::new(1, name.into(), 42);
    let key = [17; 32];
    let scope = [19; 16];
    let directory = crate::save::world_dir(name);
    std::fs::create_dir_all(directory.join("joiners")).unwrap();
    let target = crate::handover::live::personal_adapter::path(&directory, key);
    server.players[0].personal = Some(
        BoundPersonal::fresh(GameplayBinding::WorldLocal {
            scope,
            player: WorldPlayer::Npub(key),
        })
        .unwrap(),
    );
    let valid = JoinerSave::capture(&server.players[0], [3; 16]);
    let valid_bytes = valid.encode().unwrap();
    std::fs::write(&target, &valid_bytes).unwrap();
    let selected = returned_world_local(&directory, Some(scope), Some(key))
        .unwrap()
        .unwrap();
    assert_eq!(selected.encode().unwrap(), valid_bytes);
    for (label, expected_scope, signer) in [
        ("foreign scope", Some([20; 16]), Some(key)),
        ("absent scope", None, Some(key)),
        ("foreign persona", Some(scope), Some([18; 32])),
        ("signed out", Some(scope), None),
    ] {
        assert!(
            returned_world_local(&directory, expected_scope, signer).is_err(),
            "{label} installed canonical authority"
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            valid_bytes,
            "{label} rewrote the canonical body"
        );
    }
    let mut foreign = JoinerSave::decode(&valid_bytes).unwrap();
    foreign.personal = Some(
        BoundPersonal::fresh(GameplayBinding::WorldLocal {
            scope,
            player: WorldPlayer::Npub([18; 32]),
        })
        .unwrap(),
    );
    let foreign = foreign.encode().unwrap();
    server.players[0].window_events.debt = crate::container_window::CorrectionDebt::from_entries(
        vec![(Item::Block(crate::block::STONE), 3)],
    )
    .unwrap();
    let owed = JoinerSave::capture(&server.players[0], [3; 16]);
    assert!(!owed.obligations_proven_zero());
    server.players[0].window_events.debt = Default::default();
    server.players[0]
        .obligations
        .mark(IncompleteCause::LegacyV1);
    let incomplete = JoinerSave::capture(&server.players[0], [3; 16]);
    assert!(!incomplete.obligations_proven_zero());
    let mut truncated = valid_bytes.clone();
    truncated.pop();
    for (label, bytes) in [
        ("corrupt", truncated),
        ("foreign binding", foreign),
        ("owed", owed.encode().unwrap()),
        ("incomplete", incomplete.encode().unwrap()),
    ] {
        std::fs::write(&target, &bytes).unwrap();
        assert!(
            returned_world_local(&directory, Some(scope), Some(key)).is_err(),
            "{label} installed a local seat"
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            bytes,
            "{label} rewrote the refused file"
        );
    }
}

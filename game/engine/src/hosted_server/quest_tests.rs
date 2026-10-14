//! Real fresh attachments must obtain common authoritative progression.
use super::*;
use crate::action_identity::InventoryMode;
use crate::transport::ClientTransport as _;

fn fresh_authority(mode: InventoryMode, signed: bool) {
    let tag = format!("d2c3-authority-{mode:?}-{signed}");
    let _root = crate::save::WorldsRootGuard::new(&tag);
    let mut host = tests::start_room_test_server(&tag, 0);
    host.select_inventory_mode(mode).unwrap();
    assert!(crate::handover::live::personal_adapter::proof(
        &crate::save::world_dir(&host.server.world_name)
    ).unwrap().is_none(), "an ordinary world has no imported proof");
    let client = if signed {
        host.attach_test_remote_with_binding(Some([17; 32]))
    } else {
        host.attach_test_remote()
    };
    if signed {
        tests::send_signed_join(&client, c3d_tests::KEY, Some([17; 32]));
    } else {
        client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::JoinRequest,
            &crate::remote_client::build_join_request_guest("Guest", 0),
        ));
    }
    host.tick();
    let heard = c3d_tests::hear(&client);
    assert!(heard.rejects.is_empty(), "the genuine attachment is admitted");
    let accepted = heard.accepts.first().expect("a real JoinAccept");
    let i = usize::try_from(accepted.player_index).unwrap();
    let player = &host.server.players[i];
    assert_eq!(player.verified_pubkey.is_some(), signed);
    assert!(player.is_present_and_alive());
    assert_eq!(accepted.inventory_mode, mode);
    assert!(player.personal.is_some(),
        "fresh {mode:?} signed={signed} attachment has no common authoritative progression");
    assert!(accepted.has_personal_state,
        "common authority must require its actual initial window and personal installation");
}

#[test]
fn d2c3_real_signed_attachment_has_common_authority_mirror() {
    fresh_authority(InventoryMode::Mirror, true);
}
#[test]
fn d2c3_real_signed_attachment_has_common_authority_strict() {
    fresh_authority(InventoryMode::Strict, true);
}
#[test]
fn d2c3_real_guest_attachment_has_common_authority_mirror() {
    fresh_authority(InventoryMode::Mirror, false);
}
#[test]
fn d2c3_real_guest_attachment_has_common_authority_strict() {
    fresh_authority(InventoryMode::Strict, false);
}

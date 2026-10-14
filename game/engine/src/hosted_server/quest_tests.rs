//! Real fresh attachments must obtain common authoritative progression.
use super::*;
use crate::action_identity::InventoryMode;
use crate::transport::ClientTransport as _;

fn expected_signed_key() -> [u8; 32] {
    nostr::Keys::new(nostr::SecretKey::from_slice(&c3d_tests::KEY).unwrap())
        .public_key()
        .to_bytes()
}

fn fresh_authority(mode: InventoryMode, signed: bool) {
    let tag = format!("d2c3-authority-{mode:?}-{signed}");
    let _root = crate::save::WorldsRootGuard::new(&tag);
    let mut host = tests::start_room_test_server(&tag, 0);
    host.select_inventory_mode(mode).unwrap();
    assert!(
        crate::handover::live::personal_adapter::proof(&crate::save::world_dir(
            &host.server.world_name
        ))
        .unwrap()
        .is_none(),
        "an ordinary world has no imported proof"
    );
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
    assert!(
        heard.rejects.is_empty(),
        "the genuine attachment is admitted"
    );
    let accepted = heard.accepts.first().expect("a real JoinAccept");
    let i = usize::try_from(accepted.player_index).unwrap();
    let player = &host.server.players[i];
    assert_eq!(player.verified_pubkey.is_some(), signed);
    assert!(player.is_present_and_alive());
    assert_eq!(accepted.inventory_mode, mode);
    assert!(
        player.personal.is_some(),
        "fresh {mode:?} signed={signed} attachment has no common authoritative progression"
    );
    assert!(
        accepted.has_personal_state,
        "common authority must require its actual initial window and personal installation"
    );
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

#[test]
fn d2c3_empty_fresh_signed_and_guest_receive_real_sync1_before_personal_fence() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for signed in [false, true] {
            let tag = format!("d2c3-empty-sync-{mode:?}-{signed}");
            let _root = crate::save::WorldsRootGuard::new(&tag);
            let mut host = tests::start_room_test_server(&tag, 0);
            host.select_inventory_mode(mode).unwrap();
            host.server.world.world_type = "normal".into();
            host.server.world.is_workshop = false;
            let client = if signed {
                host.attach_test_remote_with_binding(Some([27; 32]))
            } else {
                host.attach_test_remote()
            };
            if signed {
                tests::send_signed_join(&client, c3d_tests::KEY, Some([27; 32]));
            } else {
                client.send_to_server(&protocol::serialize_packet(
                    protocol::PacketType::JoinRequest,
                    &crate::remote_client::build_join_request_guest("Guest", 0),
                ));
            }
            host.tick();
            let mut accepted = None;
            let mut sync = None;
            let mut personal = None;
            while let Some(packet) = client.try_recv_from_server() {
                let Some((kind, body)) = protocol::deserialize_header(&packet) else {
                    continue;
                };
                match kind {
                    protocol::PacketType::JoinAccept => {
                        accepted = Some(
                            protocol::safe_deserialize::<protocol::JoinAcceptPacket>(body).unwrap(),
                        )
                    }
                    protocol::PacketType::JoinReject => panic!(
                        "honest fresh attachment refused: {}",
                        protocol::safe_deserialize::<protocol::JoinRejectPacket>(body)
                            .unwrap()
                            .reason
                    ),
                    protocol::PacketType::InventorySync => {
                        assert!(
                            personal.is_none(),
                            "personal fence preceded the actual physical Sync1"
                        );
                        sync = Some(
                            protocol::safe_deserialize::<protocol::InventorySyncPacket>(body)
                                .unwrap(),
                        );
                    }
                    protocol::PacketType::PersonalSync => {
                        if let protocol::PersonalSyncPacket::Snapshot(snapshot) =
                            protocol::safe_deserialize(body).unwrap()
                        {
                            assert!(sync.is_some());
                            personal = Some(snapshot);
                        }
                    }
                    _ => {}
                }
            }
            let accepted = accepted.expect("real fresh JoinAccept");
            assert!(accepted.has_personal_state);
            let sync = sync.expect("empty fresh bag still receives actual Sync1");
            assert_eq!(sync.window_event, 1);
            assert_eq!(sync.slots.len(), 50);
            let snapshot = personal.expect("progression has its genuine subsequent fence");
            assert_eq!(snapshot.window_event, 2);
            let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
            slot.inventory.set_slot(
                0,
                Some(crate::item::ItemStack::new_block(crate::block::GLASS, 7)),
            );
            let mut inbox = crate::personal_sync::Inbox::new(accepted.action_session, 0);
            inbox.push(snapshot).unwrap();
            assert!(inbox.apply(0, &mut slot).unwrap().is_empty());
            crate::inventory_sync::apply(
                &mut crate::window::WindowMut {
                    inv: &mut slot.inventory,
                    armour: &mut slot.armour_slots,
                    cursor: &mut slot.crafting_ui.cursor_item,
                    grid: &mut slot.crafting_ui.grid,
                    container: None,
                },
                &sync,
                &crate::block::BlockRegistry::new(),
                &mut Default::default(),
                &mut Default::default(),
                mode,
            )
            .unwrap();
            assert!(
                slot.inventory.slot(0).is_none(),
                "Sync1 was not physically applied"
            );
            assert!(
                inbox.apply(1, &mut slot).unwrap().is_empty(),
                "Sync1 alone settled the later personal fence"
            );
            let ack = inbox.apply(2, &mut slot).unwrap().remove(0);
            let index = accepted.player_index as usize;
            assert!(!host.server.players[index].personal_delivery.initialised);
            client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::PersonalSync,
                &protocol::PersonalSyncPacket::Applied(ack),
            ));
            host.tick();
            assert!(host.server.players[index].personal_delivery.initialised);
            let context = host.server.players[index]
                .personal
                .as_ref()
                .unwrap()
                .binding
                .context();
            if signed {
                assert!(
                    matches!(context, crate::progression::GameplayContext::WorldLocal { scope, player: crate::progression::WorldPlayer::Npub(key) } if Some(scope) == host.server.world.quest_scope && key == expected_signed_key())
                );
            } else {
                assert!(
                    matches!(context, crate::progression::GameplayContext::Guest { session, generation } if session == accepted.action_session && generation == host.server.players[index].attach_gen)
                );
            }
        }
    }
}

#[test]
fn d2c3_failed_scope_durability_does_not_publish_signed_authority_or_reset_world() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-scope-refusal");
    let mut host = tests::start_room_test_server("d2c3-scope-refusal", 0);
    let directory = crate::save::world_dir(&host.server.world_name);
    std::fs::create_dir_all(&directory).unwrap();
    let mut newer = Vec::new();
    newer.extend_from_slice(&(crate::save_format::SAVE_FORMAT_VERSION + 1).to_le_bytes());
    newer.extend_from_slice(&crate::save_format::SAVE_FOOTER_MAGIC);
    std::fs::write(directory.join("world.dat"), &newer).unwrap();
    assert!(host.server.world.quest_scope.is_none());
    assert!(host.server.prepare_signed_scope().is_err());
    assert!(
        host.server.world.quest_scope.is_none(),
        "failed durability published a live random authority scope"
    );
    assert_eq!(
        std::fs::read(directory.join("world.dat")).unwrap(),
        newer,
        "failed admission rewrote the refused snapshot"
    );
    assert!(
        host.server
            .players
            .iter()
            .all(|player| player.personal.is_none())
    );
}

#[test]
fn d2c3_fresh_signed_join_refuses_a_failed_first_scope_directory_barrier() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-fresh-signed-scope-sync");
    let mut host = tests::start_room_test_server("d2c3-fresh-signed-scope-sync", 0);
    let directory = crate::save::world_dir(&host.server.world_name);
    let key = expected_signed_key();
    let canonical = directory
        .join("joiners")
        .join(format!("{}.dat", pubkey_to_npub(&key)));
    assert!(
        !directory.join("world.dat").exists(),
        "a genuine first save is needed to reach the directory fault"
    );
    assert!(!canonical.exists());
    assert!(host.server.selected_local_personal.is_none());
    assert!(host.server.world.quest_scope.is_none());
    assert!(
        crate::handover::live::personal_adapter::proof(&directory)
            .unwrap()
            .is_none()
    );
    let client = host.attach_test_remote_with_binding(Some([79; 32]));
    tests::send_signed_join(&client, c3d_tests::KEY, Some([79; 32]));
    let fault = crate::fsync::tests::fail_directory_on_match(&directory, 1);
    host.tick();
    let heard = c3d_tests::hear(&client);
    let i = host.server.players.len() - 1;
    assert_eq!(
        host.server.players[i].verified_pubkey,
        Some(key),
        "the real challenge/signature reached authenticated admission"
    );
    assert!(
        fault.failures() > 0,
        "actual first-scope publication reached the persistent world-directory fault"
    );
    eprintln!(
        "fresh-signed-first-scope directory_failures={} accepts={} present={} personal={}",
        fault.failures(),
        heard.accepts.len(),
        host.server.players[i].is_in_world(),
        host.server.players[i].personal.is_some()
    );
    assert!(
        heard.accepts.is_empty(),
        "a fresh signed JoinAccept cannot publish authority without a confirmed world directory"
    );
    assert!(!host.server.players[i].is_in_world());
    assert!(host.server.players[i].personal.is_none());
    assert!(!canonical.exists());
    drop(fault);
    // Fresh retry after the actual fault is gone must prove its own directory
    // barrier even if the earlier world.dat rename is already visible.
    let retry = host.attach_test_remote_with_binding(Some([80; 32]));
    tests::send_signed_join(&retry, c3d_tests::KEY, Some([80; 32]));
    host.tick();
    let accepted = c3d_tests::hear(&retry);
    assert_eq!(accepted.accepts.len(), 1);
    crate::handover::live::personal_adapter::confirm_durable(&directory.join("world.dat")).unwrap();
}

#[test]
fn d2c3_world_local_selected_snapshot_survives_routine_save_failed_cut_and_reopen() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-selected-local-role");
    let mut host = tests::start_room_test_server("d2c3-selected-local-role", 0);
    let directory = crate::save::world_dir(&host.server.world_name);
    let scope = [19; 16];
    let mut local = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(2.0, 80.0, -3.0), 1.0);
    local.personal_binding = Some(crate::progression::GameplayBinding::WorldLocal {
        scope,
        player: crate::progression::WorldPlayer::Npub(expected_signed_key()),
    });
    local.inventory.set_slot(
        7,
        Some(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Bread,
            5,
        )),
    );
    local.reputation.per_village.insert((2, -4), 17);
    local.kill_counter.insert(crate::mob::MobType::Pig, 37);
    local.quest_progress.declines = vec![([13; 16], 4999)];
    local.gameplay_revision = 23;
    let selected = crate::joiner_save::capture_local(&local).unwrap().unwrap();
    let expected = crate::joiner_save::JoinerSave::decode(&selected.0).unwrap();
    host.server.world.quest_scope = Some(scope);
    host.server.selected_local_personal = Some(selected.clone());
    host.server.try_save().unwrap();
    let before = std::fs::read(directory.join("world.dat")).unwrap();
    let saved = crate::save::read_world_save(&before).unwrap();
    assert_eq!(saved.local_personal.unwrap().0, selected.0);
    assert!(
        crate::gameplay_attachment::selected_world_local(
            &selected,
            Some([20; 16]),
            Some(expected_signed_key()),
            Some(0)
        )
        .is_err()
    );
    assert!(
        crate::gameplay_attachment::selected_world_local(
            &selected,
            Some(scope),
            Some([8; 32]),
            Some(0)
        )
        .is_err()
    );
    crate::save_fence::raise(&directory);
    let refused = host
        .server
        .claim_remote_world_local(expected_signed_key(), 0);
    crate::save_fence::lift(&directory);
    assert!(
        refused.is_err(),
        "a failed canonical write exposed a remote role cut"
    );
    assert_eq!(
        host.server.selected_local_personal.as_ref().unwrap().0,
        selected.0
    );
    assert_eq!(std::fs::read(directory.join("world.dat")).unwrap(), before);
    host.server
        .claim_remote_world_local(expected_signed_key(), 0)
        .unwrap();
    assert!(host.server.selected_local_personal.is_none());
    let cut =
        crate::save::read_world_save(&std::fs::read(directory.join("world.dat")).unwrap()).unwrap();
    assert!(cut.local_personal.is_none());
    assert_eq!(cut.quest_world.unwrap().scope, scope);
    drop(host);
    let mut reopened = tests::start_room_test_server("d2c3-selected-local-role", 0);
    let client = reopened.attach_test_remote_with_binding(Some([38; 32]));
    tests::send_signed_join(&client, c3d_tests::KEY, Some([38; 32]));
    reopened.tick();
    let heard = c3d_tests::hear(&client);
    assert!(heard.rejects.is_empty());
    let index = heard
        .accepts
        .first()
        .expect("same verified player reopens canonical state")
        .player_index as usize;
    let canonical = crate::joiner_save::JoinerSave::capture(
        &reopened.server.players[index],
        reopened.server.run_id,
    );
    assert_eq!(
        canonical.personal.unwrap().record().unwrap(),
        expected.personal.unwrap().record().unwrap()
    );
    assert_eq!(
        crate::window_events::effective_window(&reopened.server.players[index])
            .inv
            .slot(7)
            .unwrap()
            .count,
        5
    );
}

fn common_quest_state(
    accepted: bool,
    decline: bool,
    penalty: bool,
) -> crate::progression::GameplayRecord {
    let context = crate::progression::GameplayContext::WorldLocal {
        scope: [11; 16],
        player: crate::progression::WorldPlayer::Npub(expected_signed_key()),
    };
    let mut record = crate::progression::GameplayRecord::empty(context).unwrap();
    if accepted {
        record
            .quests
            .accepted
            .push(crate::progression::AcceptedQuest {
                id: [12; 16],
                npc: [13; 16],
                village: (1, -2),
                flavour: crate::quest::QuestFlavour::Fetch {
                    item: crate::item::MaterialId::Bread,
                    count: 5,
                },
                rewards: vec![
                    crate::saved_item_slot::SavedItemSlot::capture_exact(Some(
                        &crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 2),
                    ))
                    .unwrap(),
                ],
                reputation: 3,
                baseline: 0,
            });
    }
    if decline {
        record.quests.declines = vec![([13; 16], 4999)];
    }
    if penalty {
        record.quests.penalty_left = 129;
    }
    record.validate().unwrap();
    record
}
fn local_common_guard(accepted: bool, decline: bool, penalty: bool) {
    let record = common_quest_state(accepted, decline, penalty);
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    record.install_progression(&mut slot);
    assert!(
        slot.active_quests.is_empty()
            && slot.kill_quest_baseline.is_empty()
            && slot.villager_decline_until.is_empty()
    );
    assert!(slot.last_villager_kill_rep_tick.is_none());
    assert!(
        crate::handover::live::eligibility::local_progress(&slot, 100).is_err(),
        "common accepted={accepted} decline={decline} penalty={penalty} local state passed handover before continuation acceptance"
    );
}
fn remote_common_guard(accepted: bool, decline: bool, penalty: bool) {
    let record = common_quest_state(accepted, decline, penalty);
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    slot.personal_binding = Some(crate::progression::GameplayBinding::WorldLocal {
        scope: [11; 16],
        player: crate::progression::WorldPlayer::Npub(expected_signed_key()),
    });
    record.install_progression(&mut slot);
    let save = crate::joiner_save::capture_local(&slot).unwrap().unwrap();
    let save = crate::joiner_save::JoinerSave::decode(&save.0).unwrap();
    assert!(
        save.obligations_proven_zero(),
        "the refusal must not be caused by old correction debt"
    );
    assert_eq!(
        save.personal.as_ref().unwrap().record().unwrap().quests,
        record.quests
    );
    assert!(
        crate::handover::live::eligibility::recipient_record(Some(&save)).is_err(),
        "common accepted={accepted} decline={decline} penalty={penalty} recipient state passed handover before continuation acceptance"
    );
}
#[test]
fn d2c3_local_handover_refuses_common_accepted_quest() {
    local_common_guard(true, false, false);
}
#[test]
fn d2c3_local_handover_refuses_common_decline_timer() {
    local_common_guard(false, true, false);
}
#[test]
fn d2c3_local_handover_refuses_common_penalty_timer() {
    local_common_guard(false, false, true);
}
#[test]
fn d2c3_recipient_handover_refuses_common_accepted_quest() {
    remote_common_guard(true, false, false);
}
#[test]
fn d2c3_recipient_handover_refuses_common_decline_timer() {
    remote_common_guard(false, true, false);
}
#[test]
fn d2c3_recipient_handover_refuses_common_penalty_timer() {
    remote_common_guard(false, false, true);
}

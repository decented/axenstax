//! Predecessor-compilable native continuation controls. Signed admission,
//! physical Sync/Personal application, canonical role cut and native load are
//! real. The ordinary giver owns a validated frozen test reward; no payment,
//! initial marker, identity proof or runtime-ready flag is invented.
use super::*;
use crate::{
    action_wire::{ActionIntent, ActionPacket, AppliedActionAck},
    item::{ItemStack, MaterialId},
    progression::{AcceptedQuest, GameplayRecord},
    protocol,
    quest_wire::{QuestOperation, QuestRequest, QuestResult},
    transport::{ChannelClientTransport, ClientTransport as _},
};

struct NativeCut {
    _root: crate::save::WorldsRootGuard,
    game: HeadlessGame,
    quest: AcceptedQuest,
    name: String,
}

/// Apply an actual terminal batch in its declared physical order, then send
/// its exact personal acknowledgement before its exact receipt frontier.
/// This is setup evidence, distinct from the native GameState/UI subject.
struct RemoteSetup {
    client: ChannelClientTransport,
    i: usize,
    actor: crate::player_slot::PlayerSlot,
    inbox: crate::personal_sync::Inbox,
    frontier: (u64, u32),
    target: u32,
}
impl RemoteSetup {
    fn request(
        &mut self,
        host: &mut crate::hosted_server::HostedServer,
        incarnation: u64,
        operation: QuestOperation,
    ) -> QuestResult {
        let i = self.i;
        let target = self.target;
        let client = &self.client;
        let actor = &mut self.actor;
        let inbox = &mut self.inbox;
        let frontier = &mut self.frontier;
        frontier.0 += 1;
        let sp = &host.server.players[i];
        let raw = crate::staged_window::MarkerWindow::try_capture_parts(
            &actor.inventory,
            &actor.armour_slots,
            &actor.crafting_ui.cursor_item,
            &actor.crafting_ui.grid,
            &host.server.registry,
        )
        .unwrap();
        let packet = ActionPacket {
            action: sp
                .action_frontier
                .as_ref()
                .unwrap()
                .header(frontier.0, frontier.1),
            intent: ActionIntent::Quest(QuestRequest {
                actor_respawns: sp.respawns,
                target,
                before: raw.digest_with(sp.station, None),
                incarnation,
                operation,
            }),
        };
        client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::Action,
            &packet,
        ));
        let mut result = None;
        let mut grants = Vec::new();
        let mut personal = None;
        let mut receipt = false;
        for _ in 0..24 {
            host.tick();
            while let Some(bytes) = client.try_recv_from_server() {
                match protocol::deserialize_header(&bytes) {
                    Some((protocol::PacketType::QuestResult, body)) => {
                        let value = QuestResult::decode(body).unwrap();
                        if value.action == packet.action {
                            assert!(!receipt);
                            result = Some(value);
                        }
                    }
                    Some((protocol::PacketType::InventoryGrant, body)) => {
                        assert!(result.is_some());
                        assert!(personal.is_none());
                        grants.push(
                            protocol::safe_deserialize::<protocol::InventoryGrantPacket>(body)
                                .unwrap(),
                        );
                    }
                    Some((protocol::PacketType::PersonalSync, body)) => {
                        if let protocol::PersonalSyncPacket::Snapshot(value) =
                            protocol::safe_deserialize(body).unwrap()
                        {
                            assert!(result.is_some());
                            personal = Some(value);
                        }
                    }
                    Some((protocol::PacketType::ActionReceipt, body)) => {
                        let value: crate::action_wire::ActionReceipt =
                            protocol::safe_deserialize(body).unwrap();
                        if value.action == packet.action {
                            assert!(personal.is_some());
                            receipt = true;
                        }
                    }
                    Some((protocol::PacketType::JoinReject, body)) => panic!(
                        "native quest setup refused: {}",
                        protocol::safe_deserialize::<protocol::JoinRejectPacket>(body)
                            .unwrap()
                            .reason
                    ),
                    _ => {}
                }
            }
            if receipt {
                break;
            }
        }
        assert!(receipt, "the genuine quest terminal batch must arrive");
        let result = result.unwrap();
        assert_eq!(result.fate, crate::action_identity::ActionFate::Accepted);
        crate::pack_client::apply_canonical_window(
            &mut crate::window::WindowMut {
                inv: &mut actor.inventory,
                armour: &mut actor.armour_slots,
                cursor: &mut actor.crafting_ui.cursor_item,
                grid: &mut actor.crafting_ui.grid,
                container: None,
            },
            &result.window,
            &host.server.registry,
            &Default::default(),
            &mut Default::default(),
            host.server.players[i].inventory_mode,
        )
        .unwrap();
        for grant in grants {
            assert_eq!(
                crate::remote_entities::apply_inventory_grant(
                    &mut actor.inventory,
                    &grant,
                    &host.server.registry
                ),
                Some(0)
            );
        }
        let snapshot = personal.unwrap();
        assert_eq!(snapshot.window_event, result.included_events);
        inbox.push(snapshot).unwrap();
        let ack = inbox
            .apply(result.included_events, actor)
            .unwrap()
            .remove(0);
        client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::PersonalSync,
            &protocol::PersonalSyncPacket::Applied(ack),
        ));
        client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::AppliedActionAck,
            &AppliedActionAck {
                session: result.action.session,
                epoch: result.action.epoch,
                life: result.action.life,
                through: result.action.seq,
                included_events: result.included_events,
            },
        ));
        frontier.1 = result.included_events;
        host.tick();
        let sp = &host.server.players[i];
        assert!(sp.is_in_world());
        assert!(sp.pack_flight.is_none());
        assert!(
            sp.personal_delivery
                .quiescent(sp.personal.as_ref().unwrap(), sp.respawns)
        );
        result
    }
}

fn load_native(game: &mut HeadlessGame, name: &str) {
    game.state.world_name = name.into();
    game.state.mode =
        crate::GameMode::Loading(crate::loading_screen::LoadingState::new(name.into()));
    assert!(
        game.state.begin_load(),
        "actual native loader must accept this verified zero-obligation canonical authority"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while !matches!(game.state.mode, crate::GameMode::Playing) {
        game.frames(1);
        assert!(
            std::time::Instant::now() < deadline,
            "actual native loading did not reach Playing"
        );
    }
    assert!(!game.state.joined());
}

fn native_cut(tag: &str, decline: bool) -> NativeCut {
    let root = crate::save::WorldsRootGuard::new(tag);
    let name = tag.to_string();
    let secret = [66; 32];
    let key = nostr::Keys::new(nostr::SecretKey::from_slice(&secret).unwrap())
        .public_key()
        .to_bytes();
    let mut game = HeadlessGame::boot_into_world(&name);
    game.state.handover_fixture = Some(crate::handover_game::Fixture {
        signed_in: Some(key),
        recipient: None,
    });
    clear_pad(&mut game, None);
    game.state.players[0].inventory.clear();
    game.state.players[0]
        .inventory
        .set_slot(0, Some(ItemStack::new_material(MaterialId::Stick, 27)));
    game.state.prepare_local_gameplay().unwrap();
    let scope = game.state.world.quest_scope.unwrap();
    assert!(game.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    let mut host = crate::hosted_server::HostedServer::start(
        0,
        name.clone(),
        42,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
    )
    .unwrap();
    host.server.difficulty = crate::survival::Difficulty::Peaceful;
    let client = host.attach_test_remote_with_binding(Some([63; 32]));
    crate::hosted_server::tests::send_signed_join(&client, secret, Some([63; 32]));
    host.tick();
    let mut accepted = None;
    let mut sync = None;
    let mut personal = None;
    while let Some(bytes) = client.try_recv_from_server() {
        match protocol::deserialize_header(&bytes) {
            Some((protocol::PacketType::JoinAccept, body)) => {
                accepted =
                    Some(protocol::safe_deserialize::<protocol::JoinAcceptPacket>(body).unwrap())
            }
            Some((protocol::PacketType::InventorySync, body)) => {
                sync =
                    Some(protocol::safe_deserialize::<protocol::InventorySyncPacket>(body).unwrap())
            }
            Some((protocol::PacketType::PersonalSync, body)) => {
                if let protocol::PersonalSyncPacket::Snapshot(value) =
                    protocol::safe_deserialize(body).unwrap()
                {
                    personal = Some(value);
                }
            }
            Some((protocol::PacketType::JoinReject, body)) => panic!(
                "actual signed role cut refused: {}",
                protocol::safe_deserialize::<protocol::JoinRejectPacket>(body)
                    .unwrap()
                    .reason
            ),
            _ => {}
        }
    }
    let accepted = accepted.expect("genuine matching signed JoinAccept");
    let i = accepted.player_index as usize;
    assert_eq!(host.server.players[i].verified_pubkey, Some(key));
    assert!(host.server.selected_local_personal.is_none());
    assert_eq!(host.server.world.quest_scope, Some(scope));
    let sync = sync.expect("genuine complete Sync1");
    let snapshot = personal.expect("genuine common Personal2");
    assert_eq!((sync.window_event, snapshot.window_event), (1, 2));
    let mut actor = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    crate::inventory_sync::apply(
        &mut crate::window::WindowMut {
            inv: &mut actor.inventory,
            armour: &mut actor.armour_slots,
            cursor: &mut actor.crafting_ui.cursor_item,
            grid: &mut actor.crafting_ui.grid,
            container: None,
        },
        &sync,
        &host.server.registry,
        &mut Default::default(),
        &mut Default::default(),
        accepted.inventory_mode,
    )
    .unwrap();
    assert_eq!(actor.inventory.slot(0).unwrap().count, 27);
    let mut inbox = crate::personal_sync::Inbox::new(accepted.action_session, accepted.action_life);
    inbox.push(snapshot).unwrap();
    assert!(inbox.apply(1, &mut actor).unwrap().is_empty());
    let ack = inbox.apply(2, &mut actor).unwrap().remove(0);
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::PersonalSync,
        &protocol::PersonalSyncPacket::Applied(ack),
    ));
    host.tick();
    assert!(host.server.players[i].window_events.is_quiescent());
    let at = host.server.players[i].player.pos
        + crate::camera::forward_from(host.server.players[i].yaw, 0.0) * 1.5;
    host.server
        .world
        .village_anchors
        .insert((2, 3), at.to_array().map(|n| n.floor() as i32));
    let giver = crate::entity::spawn_mob(&mut host.server.ecs, crate::mob::MobType::Villager, at);
    host.server
        .ecs
        .get::<&mut crate::villager::VillagerComponent>(giver)
        .unwrap()
        .profession = crate::villager::Profession::Blacksmith;
    host.tick();
    let target = host
        .server
        .ecs
        .get::<&crate::entity::ProtocolId>(giver)
        .unwrap()
        .0;
    while client.try_recv_from_server().is_some() {}
    let mut remote = RemoteSetup {
        client,
        i,
        actor,
        inbox,
        frontier: (0, 2),
        target,
    };
    let opened = remote.request(&mut host, 0, QuestOperation::Talk);
    let incarnation = opened.view.unwrap().incarnation;
    // A validated, server-owned frozen fixture isolates native item custody.
    // Natural ordinary pool amounts remain unchanged and have separate tests.
    let offer = host.server.players[i]
        .quest_session
        .as_mut()
        .unwrap()
        .offer
        .as_mut()
        .unwrap();
    offer.quest.flavour = crate::quest::QuestFlavour::Fetch {
        item: MaterialId::Stick,
        count: 27,
    };
    offer.quest.rewards = vec![
        crate::saved_item_slot::SavedItemSlot::capture_exact(Some(&ItemStack::new_material(
            MaterialId::Bread,
            2,
        )))
        .unwrap(),
    ];
    offer.quest.reputation = 12;
    offer.quest.validate().unwrap();
    let quest = offer.quest.clone();
    let view = remote.request(&mut host, 0, QuestOperation::Talk);
    assert_eq!(view.view.unwrap().terms.unwrap().id, quest.id);
    remote.request(
        &mut host,
        incarnation,
        if decline {
            QuestOperation::Decline { id: quest.id }
        } else {
            QuestOperation::Accept { id: quest.id }
        },
    );
    let record = host.server.players[i]
        .personal
        .as_ref()
        .unwrap()
        .record()
        .unwrap();
    if decline {
        assert!(
            record
                .quests
                .declines
                .iter()
                .any(|&(id, left)| id == quest.npc && left > 0)
        );
    } else {
        assert_eq!(record.quests.accepted, vec![quest.clone()]);
    }
    assert_eq!(
        host.server
            .ecs
            .get::<&crate::quest_npcs::QuestNpcId>(giver)
            .unwrap()
            .0,
        quest.npc
    );
    host.server.save_joiners().unwrap();
    host.server.try_save().unwrap();
    let directory = crate::save::world_dir(&name);
    let selected =
        crate::save::read_world_save(&std::fs::read(directory.join("world.dat")).unwrap()).unwrap();
    assert!(selected.local_personal.is_none());
    let canonical = directory.join("joiners").join(format!(
        "{}.dat",
        crate::hosted_server::pubkey_to_npub(&key)
    ));
    let bytes = std::fs::read(&canonical).unwrap();
    let saved = crate::joiner_save::JoinerSave::decode(&bytes).unwrap();
    assert!(saved.obligations_proven_zero());
    assert_eq!(
        saved
            .personal
            .as_ref()
            .unwrap()
            .record()
            .unwrap()
            .quests
            .accepted
            .len(),
        usize::from(!decline)
    );
    drop(host);
    load_native(&mut game, &name);
    assert_eq!(
        std::fs::read(canonical).unwrap(),
        bytes,
        "native return preserves canonical bytes"
    );
    assert!(game.state.local_personal.is_some());
    assert_eq!(game.state.world.quest_scope, Some(scope));
    NativeCut {
        _root: root,
        game,
        quest,
        name,
    }
}

#[test]
#[ignore = "needs actual signed role cut, original-giver native ray/click/render"]
fn game_harness_d2c3_native_portable_quest_uses_restored_original_giver_dialogue() {
    let mut rig = native_cut("d2c3-native-portable-dialogue", false);
    let giver = {
        let mut query = rig.game.state.ecs.query::<&crate::quest_npcs::QuestNpcId>();
        let entities: Vec<_> = query
            .iter()
            .filter(|(_, id)| id.0 == rig.quest.npc)
            .map(|(entity, _)| entity)
            .collect();
        assert_eq!(
            entities.len(),
            1,
            "the original durable giver restores once"
        );
        entities[0]
    };
    assert_eq!(
        rig.game.state.players[0].quest_progress.accepted,
        vec![rig.quest.clone()]
    );
    assert!(rig.game.state.players[0].active_quests.is_empty());
    let at = rig
        .game
        .state
        .ecs
        .get::<&crate::entity::Position>(giver)
        .unwrap()
        .0;
    assert!(
        (at - rig.game.state.players[0].player.eye_pos()).length()
            < crate::villager_ui::INTERACT_RANGE,
        "the actual restored giver must be within ordinary reach"
    );
    aim_at(&mut rig.game, at + glam::Vec3::Y * 0.7);
    let before = crate::staged_window::MarkerWindow::capture_parts(
        &rig.game.state.players[0].inventory,
        &rig.game.state.players[0].armour_slots,
        &rig.game.state.players[0].crafting_ui.cursor_item,
        &rig.game.state.players[0].crafting_ui.grid,
        &rig.game.state.registry,
    )
    .slots();
    right_click(&mut rig.game);
    assert_eq!(
        rig.game.state.players[0].dialogue_villager,
        Some(giver),
        "real native RMB must hit this exact restored giver"
    );
    assert_eq!(
        crate::staged_window::MarkerWindow::capture_parts(
            &rig.game.state.players[0].inventory,
            &rig.game.state.players[0].armour_slots,
            &rig.game.state.players[0].crafting_ui.cursor_item,
            &rig.game.state.players[0].crafting_ui.grid,
            &rig.game.state.registry
        )
        .slots(),
        before,
        "opening native dialogue does not pay the quest"
    );
    assert!(rig.game.state.renderer.egui.last_shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("Progress: 27 / 27"))), "native dialogue offered a different legacy quest instead of the restored portable progress");
}

#[test]
#[ignore = "needs actual signed native timer restoration, ticks and selected save/reload"]
fn game_harness_d2c3_native_elapsed_ticks_project_selected_timer_without_offline_decay() {
    let mut rig = native_cut("d2c3-native-portable-timer", true);
    let binding = rig.game.state.players[0]
        .personal_binding
        .as_ref()
        .unwrap()
        .clone();
    let before = GameplayRecord::from_slot(binding.context(), &rig.game.state.players[0]).unwrap();
    let left = before
        .quests
        .declines
        .iter()
        .find(|(id, _)| *id == rig.quest.npc)
        .unwrap()
        .1;
    let tick = rig.game.state.tick_counter;
    rig.game.ticks(40);
    let elapsed = rig.game.state.tick_counter - tick;
    assert_eq!(elapsed, 40, "actual local gameplay ticks must advance");
    let after = GameplayRecord::from_slot(binding.context(), &rig.game.state.players[0]).unwrap();
    assert_eq!(
        after
            .quests
            .declines
            .iter()
            .find(|(id, _)| *id == rig.quest.npc)
            .unwrap()
            .1,
        left - elapsed as u32,
        "native common timer did not project actual local gameplay elapsed ticks"
    );
    rig.game.state.save_live_world().unwrap();
    let selected = crate::save::read_world_save(
        &std::fs::read(crate::save::world_dir(&rig.name).join("world.dat")).unwrap(),
    )
    .unwrap();
    let saved =
        crate::joiner_save::JoinerSave::decode(&selected.local_personal.unwrap().0).unwrap();
    assert_eq!(
        saved.personal.unwrap().record().unwrap().quests,
        after.quests
    );
    assert!(rig.game.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    load_native(&mut rig.game, &rig.name);
    let restored =
        GameplayRecord::from_slot(binding.context(), &rig.game.state.players[0]).unwrap();
    assert_eq!(
        restored.quests, after.quests,
        "offline/load time does not consume a saved remaining timer"
    );
}

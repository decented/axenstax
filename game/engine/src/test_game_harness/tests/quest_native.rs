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

fn visible_text(game: &HeadlessGame, wanted: &str) -> bool {
    game.state.renderer.egui.last_shapes.iter().any(|shape| {
        matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text().contains(wanted))
    })
}

fn settle_dialogue_panel(game: &mut HeadlessGame) {
    for _ in 0..20 {
        if visible_text(game, "Blacksmith") {
            break;
        }
        game.frames(1);
    }
    assert!(
        visible_text(game, "Blacksmith"),
        "the normal settled dialogue panel visibly paints its profession"
    );
}

fn click_dialogue_text(game: &mut HeadlessGame, wanted: &str) {
    let at = game
        .state
        .renderer
        .egui
        .last_shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                Some(text.galley.rect.translate(text.pos.to_vec2()).center())
            }
            _ => None,
        })
        .expect("the real dialogue paints its requested button");
    for pressed in [true, false] {
        game.state.renderer.egui.headless_events = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            },
        ];
        game.frames(1);
    }
}

fn original_giver(rig: &NativeCut) -> hecs::Entity {
    let mut query = rig.game.state.ecs.query::<&crate::quest_npcs::QuestNpcId>();
    let mut matches = query.iter().filter(|(_, id)| id.0 == rig.quest.npc);
    let giver = matches.next().expect("the exact original giver exists").0;
    assert!(matches.next().is_none(), "the original giver is unique");
    giver
}

fn native_custody(game: &HeadlessGame) -> Vec<u8> {
    let slot = &game.state.players[0];
    let record =
        GameplayRecord::from_slot(slot.personal_binding.as_ref().unwrap().context(), slot).unwrap();
    bincode::serialize(&(
        slot.inventory
            .slots_iter()
            .map(|stack| stack.cloned())
            .collect::<Vec<_>>(),
        slot.armour_slots,
        &slot.crafting_ui.cursor_item,
        &slot.crafting_ui.grid,
        record.encode().unwrap(),
    ))
    .unwrap()
}

fn native_plan_allocations(game: &HeadlessGame) -> [usize; 3] {
    let slot = &game.state.players[0];
    let ptr = |stack: &ItemStack| match &stack.item {
        crate::item::Item::Plan(plan) => plan.cells.as_ptr() as usize,
        _ => panic!("actual private Plan body required"),
    };
    [
        ptr(slot.inventory.slot(1).unwrap()),
        ptr(slot.crafting_ui.cursor_item.as_ref().unwrap()),
        ptr(slot.crafting_ui.grid[0][0].as_ref().unwrap()),
    ]
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

pub(super) fn load_native(game: &mut HeadlessGame, name: &str) {
    game.state.world_name = name.into();
    game.state.mode =
        crate::GameMode::Loading(crate::loading_screen::LoadingState::new(name.into()));
    assert!(
        game.state.begin_load(),
        "actual native loader must accept this verified zero-obligation canonical authority"
    );
    eprintln!(
        "native-cut load-start actor={:?} eye={:?}",
        game.state.players[0].player.pos,
        game.state.players[0].player.eye_pos()
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
    eprintln!(
        "native-cut Playing actor={:?} eye={:?}",
        game.state.players[0].player.pos,
        game.state.players[0].player.eye_pos()
    );
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
    eprintln!(
        "native-cut prepared source={:?} yaw={}",
        game.state.players[0].player.pos, game.state.players[0].camera.yaw
    );
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
    let source_local = host.server.players[i]
        .personal
        .as_ref()
        .unwrap()
        .record()
        .unwrap()
        .local
        .expect("the genuine native source supplies its saved local pose");
    eprintln!(
        "native-cut admitted actor={:?} spawn={:?} source-local={:?} yaw={}",
        host.server.players[i].player.pos,
        host.server.players[i].spawn_pos,
        source_local.pos,
        source_local.yaw
    );
    // Arrange only test-world geometry before the real dialogue requests. The
    // restored native actor uses this genuine saved pose, rather than the
    // dedicated admission spawn. Its common holder and window stay untouched.
    host.server.players[i].player.pos = glam::Vec3::from_array(source_local.pos);
    host.server.players[i].yaw = source_local.yaw;
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
    eprintln!(
        "native-cut capture actor={:?} giver={:?}",
        host.server.players[i].player.pos,
        host.server
            .ecs
            .get::<&crate::entity::Position>(giver)
            .unwrap()
            .0
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
    eprintln!(
        "native-cut restored giver={:?} actor={:?} eye={:?} reach={}",
        at,
        rig.game.state.players[0].player.pos,
        rig.game.state.players[0].player.eye_pos(),
        (at - rig.game.state.players[0].player.eye_pos()).length()
    );
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
    settle_dialogue_panel(&mut rig.game);
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
    let treasury = rig.game.state.world.village_treasuries.clone();
    let rep = rig.game.state.players[0]
        .reputation
        .per_village
        .get(&rig.quest.village)
        .copied()
        .unwrap_or(0);
    rig.game.state.sats_policy.bitcoin_enabled = true;
    rig.game.state.players[0].charter_allows_sats = true;
    click_dialogue_text(&mut rig.game, "Turn in");
    assert!(
        rig.game.state.players[0].dialogue_villager.is_none(),
        "actual button click commits the native turn-in"
    );
    assert!(rig.game.state.players[0].quest_progress.accepted.is_empty());
    assert_eq!(
        rig.game.state.players[0]
            .inventory
            .count_material(MaterialId::Stick),
        0
    );
    assert_eq!(
        rig.game.state.players[0]
            .inventory
            .count_material(MaterialId::Bread),
        2
    );
    assert_eq!(
        rig.game.state.players[0].reputation.per_village[&rig.quest.village],
        rep + 12
    );
    assert_eq!(
        rig.game.state.world.village_treasuries, treasury,
        "portable completion never credits monetary treasury"
    );
    assert!(
        rig.game
            .state
            .turn_in_native_quest(0, giver, rig.quest.npc, rig.quest.id)
            .is_err(),
        "exact repeated completion is inert"
    );
    assert_eq!(
        rig.game.state.players[0]
            .inventory
            .count_material(MaterialId::Bread),
        2
    );
    rig.game.state.save_live_world().unwrap();
    assert!(rig.game.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    load_native(&mut rig.game, &rig.name);
    assert!(
        rig.game.state.players[0].quest_progress.accepted.is_empty(),
        "saved completion stays consumed on actual native re-entry"
    );
    assert_eq!(
        rig.game.state.players[0]
            .inventory
            .count_material(MaterialId::Bread),
        2
    );
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

#[test]
#[ignore = "needs actual native role cut, payment refusal and private body custody"]
fn game_harness_d2c3_native_payment_refusals_and_full_bag_preserve_original_custody() {
    let mut rig = native_cut("d2c3-native-payment-custody", false);
    let giver = original_giver(&rig);
    let pos = rig
        .game
        .state
        .ecs
        .get::<&crate::entity::Position>(giver)
        .unwrap()
        .0;
    let plan = |name: &str| ItemStack {
        item: crate::item::Item::Plan(crate::plan::PlanData {
            name: name.into(),
            ..crate::plan::PlanData::debug_3x3_stone()
        }),
        count: 1,
    };
    {
        let slot = &mut rig.game.state.players[0];
        slot.inventory
            .set_slot(1, Some(plan("Native bag private body")));
        slot.crafting_ui.cursor_item = Some(plan("Native cursor private body"));
        slot.crafting_ui.grid[0][0] = Some(plan("Native grid private body"));
        let mut armour = crate::armour::ArmourItem::new(
            crate::armour::ArmourSlot::Helmet,
            crate::armour::ArmourMaterial::Iron,
        );
        armour.durability = 7;
        slot.armour_slots[0] = Some(armour);
        slot.inventory
            .set_slot(0, Some(ItemStack::new_material(MaterialId::Stick, 26)));
    }
    let pointers = native_plan_allocations(&rig.game);
    let before = native_custody(&rig.game);
    assert!(
        rig.game
            .state
            .turn_in_native_quest(0, giver, rig.quest.npc, rig.quest.id)
            .is_err(),
        "changed current bag cannot pay"
    );
    assert_eq!(native_custody(&rig.game), before);
    rig.game.state.players[0]
        .inventory
        .set_slot(0, Some(ItemStack::new_material(MaterialId::Stick, 27)));
    let wrong =
        crate::entity::spawn_mob(&mut rig.game.state.ecs, crate::mob::MobType::Villager, pos);
    let before = native_custody(&rig.game);
    assert!(
        rig.game
            .state
            .turn_in_native_quest(0, wrong, rig.quest.npc, rig.quest.id)
            .is_err(),
        "nearby same-kind replacement is not the original giver"
    );
    assert_eq!(native_custody(&rig.game), before);
    let old_health = rig
        .game
        .state
        .ecs
        .get::<&crate::combat::Health>(giver)
        .unwrap()
        .current;
    rig.game
        .state
        .ecs
        .get::<&mut crate::combat::Health>(giver)
        .unwrap()
        .current = 0.0;
    assert!(
        rig.game
            .state
            .turn_in_native_quest(0, giver, rig.quest.npc, rig.quest.id)
            .is_err(),
        "dead original cannot complete the retained quest"
    );
    assert_eq!(native_custody(&rig.game), before);
    rig.game
        .state
        .ecs
        .get::<&mut crate::combat::Health>(giver)
        .unwrap()
        .current = old_health;
    rig.game.state.players[0]
        .inventory
        .set_slot(0, Some(ItemStack::new_material(MaterialId::Stick, 28)));
    for at in 2..36 {
        rig.game.state.players[0]
            .inventory
            .set_slot(at, Some(ItemStack::new_block(crate::block::STONE, 64)));
    }
    let cursor = rig.game.state.players[0].crafting_ui.cursor_item.clone();
    let grid = rig.game.state.players[0].crafting_ui.grid.clone();
    let armour = rig.game.state.players[0].armour_slots;
    let bag_plan = rig.game.state.players[0].inventory.slot(1).cloned();
    let dropped_bread = |game: &HeadlessGame| {
        game.state
            .ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter(|(_, item)| item.stack.item == crate::item::Item::Material(MaterialId::Bread))
            .map(|(_, item)| u32::from(item.stack.count))
            .sum::<u32>()
    };
    assert_eq!(dropped_bread(&rig.game), 0);
    rig.game
        .state
        .turn_in_native_quest(0, giver, rig.quest.npc, rig.quest.id)
        .unwrap();
    assert_eq!(
        rig.game.state.players[0]
            .inventory
            .count_material(MaterialId::Stick),
        1
    );
    assert_eq!(
        rig.game.state.players[0]
            .inventory
            .count_material(MaterialId::Bread),
        0
    );
    assert_eq!(
        dropped_bread(&rig.game),
        2,
        "only the actual reward remainder spills at the original giver"
    );
    assert_eq!(rig.game.state.players[0].crafting_ui.cursor_item, cursor);
    assert_eq!(rig.game.state.players[0].crafting_ui.grid, grid);
    assert_eq!(rig.game.state.players[0].armour_slots, armour);
    assert_eq!(
        rig.game.state.players[0].inventory.slot(1).cloned(),
        bag_plan
    );
    assert_eq!(
        native_plan_allocations(&rig.game),
        pointers,
        "unaffected private allocations are retained"
    );
    assert!(
        rig.game
            .state
            .turn_in_native_quest(0, giver, rig.quest.npc, rig.quest.id)
            .is_err()
    );
    assert_eq!(dropped_bread(&rig.game), 2);
}

#[test]
#[ignore = "needs actual native death sweep, common counters and elapsed penalty expiry"]
fn game_harness_d2c3_native_kill_and_penalty_credit_once_in_the_same_holder() {
    let mut rig = native_cut("d2c3-native-common-kills", true);
    let pos = rig.game.state.players[0].player.pos + glam::Vec3::X;
    let before = rig.game.state.players[0]
        .kill_counter
        .get(&crate::mob::MobType::Cow)
        .copied()
        .unwrap_or(0);
    let cow = crate::entity::spawn_mob(&mut rig.game.state.ecs, crate::mob::MobType::Cow, pos);
    rig.game
        .state
        .ecs
        .get::<&mut crate::combat::Health>(cow)
        .unwrap()
        .current = 0.0;
    rig.game.ticks(1);
    assert!(
        !rig.game.state.ecs.contains(cow),
        "the real native death sweep consumed the corpse"
    );
    assert_eq!(
        rig.game.state.players[0].kill_counter[&crate::mob::MobType::Cow],
        before + 1
    );
    rig.game.ticks(1);
    assert_eq!(
        rig.game.state.players[0].kill_counter[&crate::mob::MobType::Cow],
        before + 1,
        "later ticks do not re-credit the corpse"
    );
    let village = rig.quest.village;
    let rep = rig.game.state.players[0]
        .reputation
        .per_village
        .get(&village)
        .copied()
        .unwrap_or(0);
    for n in 0..2 {
        let victim =
            crate::entity::spawn_mob(&mut rig.game.state.ecs, crate::mob::MobType::Villager, pos);
        rig.game
            .state
            .ecs
            .get::<&mut crate::combat::Health>(victim)
            .unwrap()
            .current = 0.0;
        rig.game.ticks(1);
        assert!(!rig.game.state.ecs.contains(victim));
        assert_eq!(
            rig.game.state.players[0].reputation.per_village[&village],
            rep - 25,
            "only one penalty is charged during the actual cooldown"
        );
        let binding = rig.game.state.players[0].personal_binding.as_ref().unwrap();
        let record =
            GameplayRecord::from_slot(binding.context(), &rig.game.state.players[0]).unwrap();
        assert_eq!(record.quests.penalty_left, 600 - n);
    }
    let revision = rig.game.state.players[0].gameplay_revision;
    rig.game.ticks(599);
    assert_eq!(rig.game.state.players[0].quest_progress.penalty_left, 0);
    assert_eq!(
        rig.game.state.players[0].gameplay_revision,
        revision + 1,
        "actual expiry commits one revision rather than a revision per timer tick"
    );
    rig.game.state.save_live_world().unwrap();
    assert!(rig.game.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    load_native(&mut rig.game, &rig.name);
    assert_eq!(
        rig.game.state.players[0].kill_counter[&crate::mob::MobType::Cow],
        before + 1,
        "same counters survive selected native save and reopen"
    );
    assert_eq!(rig.game.state.players[0].quest_progress.penalty_left, 0);
}

#[test]
#[ignore = "needs genuine unbound native deaths and durable first common enrolment"]
fn game_harness_d2c3_first_native_binding_preserves_the_live_legacy_penalty() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-first-native-penalty");
    let name = "d2c3-first-native-penalty";
    let mut game = HeadlessGame::boot_into_world(name);
    clear_pad(&mut game, None);
    assert!(
        game.state.players[0].personal_binding.is_none(),
        "the predecessor must genuinely start unbound"
    );
    assert!(game.state.world.quest_scope.is_none());
    let pos = game.state.players[0].player.pos + glam::Vec3::X;
    let village = (4, 5);
    game.state
        .world
        .village_anchors
        .insert(village, pos.to_array().map(|value| value.floor() as i32));
    let victim = crate::entity::spawn_mob(&mut game.state.ecs, crate::mob::MobType::Villager, pos);
    game.state
        .ecs
        .get::<&mut crate::combat::Health>(victim)
        .unwrap()
        .current = 0.0;
    game.ticks(1);
    assert!(
        !game.state.ecs.contains(victim),
        "the first real unbound death was swept"
    );
    assert!(game.state.players[0].personal_binding.is_none());
    assert_eq!(
        game.state.players[0].kill_counter[&crate::mob::MobType::Villager],
        1
    );
    let first_score = game.state.players[0].reputation.per_village[&village];
    assert_eq!(first_score, -25);
    let first_tick = game.state.players[0]
        .last_villager_kill_rep_tick
        .expect("the legacy owner recorded its real penalty tick");
    game.ticks(7);
    let before_enrolment = game.state.tick_counter;
    assert_eq!(before_enrolment - first_tick, 7);
    game.state.prepare_local_gameplay().unwrap();
    let scope = game
        .state
        .world
        .quest_scope
        .expect("actual first preparation durably establishes the world scope");
    let selected = crate::save::read_world_save(
        &std::fs::read(crate::save::world_dir(name).join("world.dat")).unwrap(),
    )
    .unwrap();
    let saved = crate::joiner_save::JoinerSave::decode(
        &selected
            .local_personal
            .expect("the real selected snapshot owns this initial holder")
            .0,
    )
    .unwrap();
    let binding = game.state.players[0].personal_binding.as_ref().unwrap();
    let initial = saved.personal.as_ref().unwrap().record().unwrap();
    assert_eq!(initial.context, binding.context());
    assert_eq!(selected.quest_world.as_ref().unwrap().scope, scope);
    assert_eq!(
        initial
            .reputation
            .iter()
            .find(|(id, _)| *id == village)
            .map(|(_, score)| *score),
        Some(first_score)
    );
    assert_eq!(
        initial
            .kills
            .iter()
            .find(|(kind, _)| *kind == crate::mob::MobType::Villager)
            .map(|(_, count)| *count),
        Some(1)
    );
    let second = crate::entity::spawn_mob(&mut game.state.ecs, crate::mob::MobType::Villager, pos);
    game.state
        .ecs
        .get::<&mut crate::combat::Health>(second)
        .unwrap()
        .current = 0.0;
    game.ticks(1);
    assert!(
        !game.state.ecs.contains(second),
        "the second actual bound death reached the common owner"
    );
    assert_eq!(
        game.state.players[0].kill_counter[&crate::mob::MobType::Villager],
        2
    );
    let record = GameplayRecord::from_slot(
        game.state.players[0]
            .personal_binding
            .as_ref()
            .unwrap()
            .context(),
        &game.state.players[0],
    )
    .unwrap();
    eprintln!(
        "first-native-penalty first_tick={first_tick} enrol_tick={before_enrolment} current_tick={} initial_remaining={} current_remaining={} first_score={first_score} current_score={}",
        game.state.tick_counter,
        initial.quests.penalty_left,
        record.quests.penalty_left,
        game.state.players[0].reputation.per_village[&village]
    );
    assert_eq!(
        game.state.players[0].reputation.per_village[&village], first_score,
        "first common enrolment must not charge a second penalty within the same live cooldown"
    );
    assert_eq!(
        initial.quests.penalty_left, 593,
        "first durable snapshot carries exact remaining legacy cooldown"
    );
    assert_eq!(
        record.quests.penalty_left, 592,
        "the next actual tick consumes only one remaining tick"
    );
}

#[test]
#[ignore = "needs genuine native first scope and persistent directory durability failure"]
fn game_harness_d2c3_native_first_scope_refuses_a_failed_directory_barrier() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-native-first-scope-sync");
    let mut game = HeadlessGame::boot_into_world("d2c3-native-first-scope-sync");
    // This first scope has no private Plan write or remote canonical writer
    // which could accidentally confirm the world directory afterwards.
    game.state.players[0].inventory.clear();
    assert!(game.state.players[0].crafting_ui.cursor_item.is_none());
    assert!(
        game.state.players[0]
            .crafting_ui
            .grid
            .iter()
            .flatten()
            .all(Option::is_none)
    );
    assert!(game.state.hosted_server.is_none());
    assert!(game.state.players[0].personal_binding.is_none());
    assert!(game.state.local_personal.is_none());
    assert!(game.state.world.quest_scope.is_none());
    let directory = crate::save::world_dir(&game.state.world_name);
    assert!(
        crate::handover::live::personal_adapter::proof(&directory)
            .unwrap()
            .is_none()
    );
    let before = super::quest_preparation::live(&game);
    let prior_saves = game.state.session_saves;
    let fault = crate::fsync::tests::fail_directory_on_match(&directory, 1);
    let prepared = game.state.prepare_local_gameplay();
    assert!(
        fault.failures() > 0,
        "the actual selected writer reached the world-directory fault"
    );
    eprintln!(
        "native-first-scope directory_failures={} returned_error={} live_scope={} live_binding={}",
        fault.failures(),
        prepared.is_err(),
        game.state.world.quest_scope.is_some(),
        game.state.players[0].personal_binding.is_some()
    );
    assert!(
        prepared.is_err(),
        "first native authority must refuse an unconfirmed directory barrier"
    );
    assert!(game.state.world.quest_scope.is_none());
    assert!(game.state.players[0].personal_binding.is_none());
    assert!(game.state.local_personal.is_none());
    assert_eq!(super::quest_preparation::live(&game), before);
    let mut failed_saves = prior_saves;
    failed_saves.note_save(false);
    assert_eq!(game.state.session_saves, failed_saves);
    let retry = super::quest_preparation::native_scope_failure_tail(&mut game, &directory, &fault);
    // A rename may already be visible: do not demand old primary bytes after
    // that commit, or mistake visibility for confirmed crash durability.
    drop(fault);
    game.state.prepare_local_gameplay().unwrap();
    assert!(game.state.world.quest_scope.is_some());
    crate::handover::live::personal_adapter::confirm_durable(&directory.join("world.dat")).unwrap();
    super::quest_preparation::assert_native_retry(&game, &directory, retry);
}

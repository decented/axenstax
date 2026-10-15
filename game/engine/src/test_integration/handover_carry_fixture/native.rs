//! Actual signed own-key quest activity and checked native menu return.
use super::{key, native};
use crate::{
    action_identity::ActionFate,
    action_wire::ActionIntent,
    hosted_server::{HostedServer, RemoteTransport, quest_lifecycle_actor::Actor},
    progression::GameplayBinding,
    protocol::{self, PacketType},
    quest_wire::{QuestOperation, QuestRequest, QuestView},
    test_game_harness::HeadlessGame,
    transport::ClientTransport,
};
use glam::Vec3;
use std::time::{Duration, Instant};
fn actor_input(host: &mut HostedServer, actor: &mut Actor, yaw: f32, pitch: f32) {
    actor.input_seq += 1;
    let sp = &host.server.players[actor.i()];
    let at = sp.player.pos;
    actor.client.send_to_server(&protocol::serialize_packet(
        PacketType::ClientInput,
        &protocol::InputPacket {
            tick: actor.input_seq,
            x: at.x,
            y: at.y,
            z: at.z,
            yaw,
            pitch,
            health: sp.combat.health,
            events_applied: actor.applied,
            ..Default::default()
        },
    ));
    host.tick();
    actor.drain(&host.server.registry);
}
pub(super) fn actor_tick(host: &mut HostedServer, actor: &mut Actor) {
    let sp = &host.server.players[actor.i()];
    let (yaw, pitch) = (sp.yaw, sp.pitch);
    actor_input(host, actor, yaw, pitch);
}
#[derive(Debug)]
struct QuestGeometry {
    eye: Vec3,
    position: Vec3,
    centre: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    facing: f32,
}
fn quest_geometry(host: &HostedServer, actor: &Actor, target: u32) -> QuestGeometry {
    let ecs = &host.server.ecs;
    let entity = ecs
        .query::<&crate::entity::ProtocolId>()
        .iter()
        .find(|(_, id)| id.0 == target)
        .map(|(entity, _)| entity)
        .expect("actual quest target still exists");
    let position = ecs.get::<&crate::entity::Position>(entity).unwrap().0;
    let height = ecs.get::<&crate::entity::Hitbox>(entity).unwrap().height;
    let centre = position + Vec3::Y * (height * 0.5);
    let sp = &host.server.players[actor.i()];
    let eye = sp.player.eye_pos();
    let to_target = centre - eye;
    QuestGeometry {
        eye,
        position,
        centre,
        yaw: sp.yaw,
        pitch: sp.pitch,
        distance: to_target.length(),
        facing: to_target.normalize_or_zero().dot(crate::camera::forward_from(sp.yaw, sp.pitch)),
    }
}
pub(super) fn settle_actor(host: &mut HostedServer, actor: &mut Actor) {
    for _ in 0..32 {
        actor_tick(host, actor);
        let sp = &host.server.players[actor.i()];
        if actor.idle()
            && sp.window_events.is_quiescent()
            && sp.personal_delivery.quiescent(sp.personal.as_ref().unwrap(), sp.respawns)
            && sp.action_results.as_ref().is_none_or(|r| r.is_quiescent())
        {
            return;
        }
    }
    panic!("the real source-own-key initial/window/Personal/receipt boundary did not settle");
}
pub(super) fn actor_quest(
    host: &mut HostedServer,
    actor: &mut Actor,
    target: u32,
    incarnation: u64,
    operation: QuestOperation,
) -> Option<QuestView> {
    settle_actor(host, actor);
    let target_geometry = quest_geometry(host, actor, target);
    let direction = (target_geometry.centre - target_geometry.eye).normalize();
    actor_input(host, actor, (-direction.x).atan2(-direction.z), direction.y.asin());
    settle_actor(host, actor);
    // Observe the authenticated look after ordinary settlement, rather than
    // assigning server aim or assuming that the input has already landed.
    let aimed = quest_geometry(host, actor, target);
    assert!(
        aimed.eye.is_finite()
            && aimed.position.is_finite()
            && aimed.centre.is_finite()
            && aimed.yaw.is_finite()
            && aimed.pitch.is_finite()
            && aimed.distance.is_finite()
            && aimed.distance > 0.0
            && aimed.distance < crate::villager_ui::INTERACT_RANGE
            && aimed.facing.is_finite()
            && aimed.facing > 0.9,
        "actual source quest aim prerequisite operation={operation:?} target={target}: {aimed:?}"
    );
    let sp = &host.server.players[actor.i()];
    let raw = crate::staged_window::MarkerWindow::try_capture_parts(
        &actor.slot.inventory,
        &actor.slot.armour_slots,
        &actor.slot.crafting_ui.cursor_item,
        &actor.slot.crafting_ui.grid,
        &host.server.registry,
    )
    .unwrap();
    let action = actor.send(ActionIntent::Quest(QuestRequest {
        actor_respawns: sp.respawns,
        target,
        before: raw.digest_with(sp.station, None),
        incarnation,
        operation: operation.clone(),
    }));
    settle_actor(host, actor);
    let after = quest_geometry(host, actor, target);
    let result = actor.results.iter().find(|r| r.action == action).unwrap();
    assert_eq!(
        result.fate,
        ActionFate::Accepted,
        "actual source quest operation={operation:?} note={:?} target={target} \
         aimed eye={:?} position={:?} yaw={} pitch={} distance={} facing={} \
         after eye={:?} position={:?} yaw={} pitch={} distance={} facing={}",
        result.note,
        aimed.eye,
        aimed.position,
        aimed.yaw,
        aimed.pitch,
        aimed.distance,
        aimed.facing,
        after.eye,
        after.position,
        after.yaw,
        after.pitch,
        after.distance,
        after.facing,
    );
    result.view.clone()
}
fn actor_giver(host: &mut HostedServer, actor: &mut Actor, distance: f32) -> u32 {
    let sp = &host.server.players[actor.i()];
    let at = sp.player.pos + crate::camera::forward_from(sp.yaw, 0.0) * distance;
    host.server.world.village_anchors.insert((2, 3), at.to_array().map(|v| v.floor() as i32));
    let entity = crate::entity::spawn_mob(&mut host.server.ecs, crate::mob::MobType::Villager, at);
    host.server.ecs.get::<&mut crate::villager::VillagerComponent>(entity).unwrap().profession =
        crate::villager::Profession::Blacksmith;
    actor_tick(host, actor);
    host.server.ecs.get::<&crate::entity::ProtocolId>(entity).unwrap().0
}

/// Enrol and restore through ordinary selected/canonical authority, never an
/// invented Imported binding or the direct Loading helper used by older tests.
pub(super) fn active_source(name: &str, secret: [u8; 32]) -> HeadlessGame {
    let mut game = HeadlessGame::boot_into_world(name);
    game.state.handover_fixture = Some(crate::handover_game::Fixture { signed_in: Some(key(secret)), recipient: None });
    game.state.difficulty = "peaceful".into();
    game.state.prepare_local_gameplay().unwrap();
    let scope = game.state.world.quest_scope.unwrap();
    let seed = game.state.biome_gen.seed;
    assert!(game.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
    let mut host = HostedServer::start(0, name.into(), seed, 0, RemoteTransport::WebSocket { port: 0 }).unwrap();
    host.server.difficulty = crate::survival::Difficulty::Peaceful;
    let client = host.attach_test_remote_with_binding(Some([60; 32]));
    crate::hosted_server::tests::send_signed_join(&client, secret, Some([60; 32]));
    host.tick();
    let bytes = client.try_recv_from_server().expect("actual JoinAccept before initial carriers");
    let (kind, body) = protocol::deserialize_header(&bytes).unwrap();
    assert_eq!(kind, PacketType::JoinAccept);
    let accepted: protocol::JoinAcceptPacket = protocol::safe_deserialize(body).unwrap();
    let mut actor = Actor::new(
        client,
        accepted.clone(),
        crate::player_slot::PlayerSlot::new(0, Vec3::ZERO, 1.0),
        crate::personal_sync::Inbox::new(accepted.action_session, accepted.action_life),
        0,
    );
    actor.drain(&host.server.registry);
    assert_eq!(actor.initial_carriers, [(1, "sync"), (2, "personal")]);
    assert!(!host.server.players[actor.i()].personal_delivery.initialised);
    settle_actor(&mut host, &mut actor);
    assert!(host.server.players[actor.i()].verified_pubkey == Some(key(secret)), "actual source-owner npub differs");
    assert!(host.server.selected_local_personal.is_none());
    let local = host.server.players[actor.i()].personal.as_ref().unwrap().record().unwrap().local.unwrap();
    host.server.players[actor.i()].player.pos = Vec3::from_array(local.pos);
    host.server.players[actor.i()].yaw = local.yaw;
    let pos = Vec3::from_array(local.pos);
    for x in pos.x.floor() as i32 - 4..=pos.x.floor() as i32 + 4 {
        for z in pos.z.floor() as i32 - 4..=pos.z.floor() as i32 + 4 {
            let floor = pos.y.floor() as i32 - 1;
            host.server.world.set_block(x, floor, z, crate::block::STONE);
            for y in floor + 1..floor + 6 {
                host.server.world.set_block(x, y, z, crate::block::AIR);
            }
        }
    }
    host.server.players[actor.i()].player.velocity = Vec3::ZERO;
    host.server.players[actor.i()].player.on_ground = true;
    host.server.players[actor.i()].player.reset_fall();
    let target = actor_giver(&mut host, &mut actor, 1.5);
    let offered = actor_quest(&mut host, &mut actor, target, 0, QuestOperation::Talk).unwrap();
    let terms = offered.terms.as_ref().unwrap();
    assert!(matches!(terms.flavour, crate::quest::QuestFlavour::Fetch { .. }));
    let accepted_id = terms.id;
    let npc = offered.npc;
    actor_quest(&mut host, &mut actor, target, offered.incarnation, QuestOperation::Accept { id: accepted_id });
    let second = actor_giver(&mut host, &mut actor, -1.5);
    let declined = actor_quest(&mut host, &mut actor, second, 0, QuestOperation::Talk).unwrap();
    assert_ne!(declined.npc, npc);
    actor_quest(
        &mut host,
        &mut actor,
        second,
        declined.incarnation,
        QuestOperation::Decline { id: declined.terms.unwrap().id },
    );
    let record = host.server.players[actor.i()].personal.as_ref().unwrap().record().unwrap();
    let accepted = record.quests.accepted.iter().find(|q| q.id == accepted_id).unwrap().clone();
    assert_eq!(accepted.npc, npc);
    assert!(record.quests.declines.iter().any(|(id, left)| *id == declined.npc && *left > 0));
    actor.client.send_to_server(&protocol::serialize_packet(PacketType::Disconnect, &()));
    host.tick();
    assert!(host.slot_is_free(actor.i()));
    host.server.save_joiners().unwrap();
    host.server.try_save().unwrap();
    let directory = crate::save::world_dir(name);
    let canonical = crate::handover::live::personal_adapter::path(&directory, key(secret));
    let saved_bytes = std::fs::read(&canonical).unwrap();
    let saved = crate::joiner_save::JoinerSave::decode(&saved_bytes).unwrap();
    assert!(saved.obligations_proven_zero());
    assert_eq!(saved.personal.as_ref().unwrap().record().unwrap().quests.accepted, [accepted.clone()]);
    drop(actor);
    drop(host);
    let meta = crate::save::try_load_world_meta(name).unwrap();
    game.state.pending_menu_action = Some((
        crate::menu::MenuAction::LoadWorld(name.into(), crate::spawn_pref::SpawnPref::Default),
        web_time::Instant::now(),
    ));
    let deadline = Instant::now() + Duration::from_secs(180);
    while !matches!(game.state.mode, crate::GameMode::Playing) {
        game.frames(1);
        if let crate::GameMode::Menu(menu) = &game.state.mode {
            assert!(menu.notice.is_none(), "actual menu reopen refused: {:?}", menu.notice);
        }
        assert!(Instant::now() < deadline, "actual queued menu reopen did not complete");
    }
    assert!(!game.state.joined());
    assert_eq!(game.state.world.total_ticks, meta.total_ticks + game.state.tick_counter);
    assert_eq!(game.state.world.quest_scope, Some(scope));
    assert!(
        matches!(&game.state.players[0].personal_binding, Some(GameplayBinding::WorldLocal { scope: own_scope, player: crate::progression::WorldPlayer::Npub(npub) }) if *own_scope == scope && *npub == key(secret))
    );
    assert_eq!(std::fs::read(canonical).unwrap(), saved_bytes);
    assert_eq!(native(&game).quests.accepted, [accepted]);
    let saved_record = saved.personal.as_ref().unwrap().record().unwrap();
    let (_, saved_left) = saved_record.quests.declines.iter().find(|(id, _)| *id == declined.npc).unwrap();
    assert_eq!(
        native(&game).quests.declines.iter().find(|(id, _)| *id == declined.npc).unwrap().1,
        saved_left.saturating_sub(game.state.tick_counter as u32)
    );
    assert!(game.state.ecs.query::<&crate::quest_npcs::QuestNpcId>().iter().any(|(_, id)| id.0 == npc));
    game.state.difficulty = "peaceful".into();
    original_dialogue(&mut game, npc);
    native_penalty(&mut game);
    assert!(game.state.players[0].active_quests.is_empty());
    assert!(game.state.players[0].kill_quest_baseline.is_empty());
    assert!(game.state.players[0].villager_decline_until.is_empty());
    assert!(game.state.players[0].last_villager_kill_rep_tick.is_none());
    game
}
fn native_penalty(game: &mut HeadlessGame) {
    let pos = game.state.players[0].player.pos + Vec3::X;
    game.state.world.village_anchors.insert((2, 3), pos.to_array().map(|v| v.floor() as i32));
    let entity = crate::entity::spawn_mob(&mut game.state.ecs, crate::mob::MobType::Villager, pos);
    game.state.ecs.get::<&mut crate::combat::Health>(entity).unwrap().current = 1.0;
    let slot = &game.state.players[0];
    let eye = slot.player.eye_pos();
    let damage = slot.inventory.hotbar_attack_damage(slot.hotbar_slot);
    let owner = crate::tameable::local_owner_key(0);
    assert!(crate::combat::strike(
        &mut game.state.ecs,
        entity,
        &crate::combat::Swing {
            attacker: crate::combat::Attacker::Local(0),
            owner_key: &owner,
            eye,
            look_dir: (pos - eye).normalize(),
            crit: false,
            sprinting: false,
            sneaking: false,
            base_damage: damage,
        }
    ));
    assert!(game.state.ecs.get::<&crate::combat::Health>(entity).unwrap().is_dead());
    assert!(matches!(
        game.state.ecs.get::<&crate::combat::LastAttacker>(entity).unwrap().0,
        crate::combat::Attacker::Local(0)
    ));
    game.ticks(1);
    assert!(!game.state.ecs.contains(entity));
    let record = native(game);
    assert_eq!(record.kills.iter().find(|(kind, _)| *kind == crate::mob::MobType::Villager).unwrap().1, 1);
    assert_eq!(record.quests.penalty_left, 600);
    assert_eq!(game.state.players[0].reputation.score((2, 3)), -25);
}

pub(super) fn original_dialogue(game: &mut HeadlessGame, npc: [u8; 16]) {
    let entities: Vec<_> = game
        .state
        .ecs
        .query::<&crate::quest_npcs::QuestNpcId>()
        .iter()
        .filter(|(_, id)| id.0 == npc)
        .map(|(e, _)| e)
        .collect();
    assert_eq!(entities.len(), 1);
    let giver = entities[0];
    let at = game.state.ecs.get::<&crate::entity::Position>(giver).unwrap().0;
    assert!((at - game.state.players[0].player.eye_pos()).length() < crate::villager_ui::INTERACT_RANGE);
    let dir = (at + Vec3::Y * 0.7 - game.state.players[0].player.eye_pos()).normalize();
    game.state.players[0].camera.yaw = (-dir.x).atan2(-dir.z);
    game.state.players[0].camera.pitch = dir.y.asin();
    while game.state.players[0].place_cooldown > 0 {
        game.ticks(1);
    }
    let accepted = native(game).quests.accepted;
    game.state.input.cursor_captured = true;
    game.state.input.right_held = true;
    game.frames(1);
    game.state.input.right_held = false;
    assert_eq!(
        game.state.players[0].dialogue_villager,
        Some(giver),
        "actual native RMB selects the original durable giver"
    );
    for _ in 0..20 {
        if game
            .state
            .renderer
            .egui
            .last_shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("Progress:")))
        {
            break;
        }
        game.frames(1);
    }
    assert!(
        game.state
            .renderer
            .egui
            .last_shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("Progress:"))),
        "actual portable dialogue must paint the restored accepted quest"
    );
    let close = game
        .state
        .renderer
        .egui
        .last_shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Close" => {
                Some(text.galley.rect.translate(text.pos.to_vec2()).center())
            }
            _ => None,
        })
        .expect("real portable dialogue Close button");
    for pressed in [true, false] {
        game.state.renderer.egui.headless_events = vec![
            egui::Event::PointerMoved(close),
            egui::Event::PointerButton {
                pos: close,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            },
        ];
        game.frames(1);
    }
    assert!(game.state.players[0].dialogue_villager.is_none());
    assert_eq!(native(game).quests.accepted, accepted, "inspection never completes or replaces the accepted quest");
}

/// Walk with genuine authenticated input; never assign server position/look.
pub(super) fn walk_to_giver(host: &mut HostedServer, actor: &mut Actor, target: u32) {
    for _ in 0..1000 {
        let geometry = quest_geometry(host, actor, target);
        if geometry.distance < 3.0 { settle_actor(host, actor); return; }
        assert!(geometry.distance.is_finite() && geometry.distance < 128.0, "saved own giver is beyond the bounded movement fixture");
        let direction = (geometry.centre - geometry.eye).normalize();
        actor.input_seq += 1;
        let sp = &host.server.players[actor.i()];
        let at = sp.player.pos;
        actor.client.send_to_server(&protocol::serialize_packet(PacketType::ClientInput, &protocol::InputPacket {
            tick: actor.input_seq, x: at.x, y: at.y, z: at.z, yaw: (-direction.x).atan2(-direction.z), pitch: direction.y.asin(),
            move_forward: 1.0, health: sp.combat.health, events_applied: actor.applied, ..Default::default()
        }));
        host.tick(); actor.drain(&host.server.registry);
    }
    panic!("actual signed movement did not reach the original durable giver");
}
pub(super) fn repeat_completed(host: &mut HostedServer, actor: &mut Actor, target: u32, incarnation: u64, id: [u8;16]) {
    settle_actor(host, actor);
    let raw = crate::staged_window::MarkerWindow::try_capture_parts(&actor.slot.inventory, &actor.slot.armour_slots,
        &actor.slot.crafting_ui.cursor_item, &actor.slot.crafting_ui.grid, &host.server.registry).unwrap();
    let sp = &host.server.players[actor.i()];
    let action = actor.send(ActionIntent::Quest(QuestRequest { actor_respawns: sp.respawns, target,
        before: raw.digest_with(sp.station, None), incarnation, operation: QuestOperation::TurnIn { id } }));
    settle_actor(host, actor);
    let result = actor.results.iter().find(|result| result.action == action).unwrap();
    assert_eq!(result.fate, ActionFate::Refused, "new-sequence completion of the consumed exact quest must refuse: {:?}", result.note);
}

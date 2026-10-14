//! Genuine joined ordinary-villager click, without a fabricated Talk wire.
use super::*;
use crate::action_identity::InventoryMode;
use crate::transport::{ChannelClientTransport, ClientTransport};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct CountActions {
    inner: ChannelClientTransport,
    actions: Arc<AtomicUsize>,
}
impl ClientTransport for CountActions {
    fn send_to_server(&self, data: &[u8]) {
        if matches!(
            crate::protocol::deserialize_header(data),
            Some((crate::protocol::PacketType::Action, _))
        ) {
            self.actions.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.send_to_server(data);
    }
    fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
        self.inner.try_recv_from_server()
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.inner.channel_binding()
    }
}

fn joined_counted(
    mode: InventoryMode,
) -> (
    HeadlessGame,
    crate::hosted_server::HostedServer,
    usize,
    Arc<AtomicUsize>,
) {
    let tag = format!("d2c3-dialogue-{mode:?}");
    let mut hg = HeadlessGame::boot_into_world(&format!("harness-{tag}"));
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    let mut host = crate::hosted_server::HostedServer::start(
        0,
        format!("harness-{tag}-server-{}", std::process::id()),
        42,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
    )
    .unwrap();
    host.select_inventory_mode(mode).unwrap();
    host.server.difficulty = crate::survival::Difficulty::Peaceful;
    let actions = Arc::new(AtomicUsize::new(0));
    let transport = CountActions {
        inner: host.attach_test_remote(),
        actions: actions.clone(),
    };
    hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
        Box::new(transport),
        crate::remote_client::build_join_request_guest("Quest guest", 0),
        None,
    ));
    pack_wait(&mut host, &mut hg, |_, game| {
        game.state
            .remote_client
            .as_ref()
            .is_some_and(|client| client.is_connected() && client.personal_ready())
    });
    let slot = host.server.players.len() - 1;
    hg.state.players[0].inventory = crate::inventory::Inventory::new();
    hg.state.players[0].armour_slots = [None; 4];
    host.server.players[slot].inventory = crate::inventory::Inventory::new();
    host.server.players[slot].possession = Default::default();
    (hg, host, slot, actions)
}

#[test]
#[ignore = "needs the real GPU client click/render/tick path"]
fn game_harness_d2c3_real_joined_villager_click_sends_one_server_action() {
    isolate_saves();
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let (mut hg, mut host, slot, actions) = joined_counted(mode);
        clear_pad(&mut hg, Some(&mut host));
        let at = host.server.players[slot].player.pos + glam::Vec3::new(0.0, 0.0, -2.0);
        let npc = crate::entity::spawn_mob(&mut host.server.ecs, crate::mob::MobType::Villager, at);
        host.server
            .ecs
            .get::<&mut crate::villager::VillagerComponent>(npc)
            .unwrap()
            .profession = crate::villager::Profession::Farmer;
        pack_wait(&mut host, &mut hg, |server, game| {
            server
                .server
                .ecs
                .get::<&crate::entity::ProtocolId>(npc)
                .is_ok_and(|id| game.state.remote_mobs.position_of(id.0).is_some())
        });
        let id = host
            .server
            .ecs
            .get::<&crate::entity::ProtocolId>(npc)
            .unwrap()
            .0;
        let current = host
            .server
            .ecs
            .get::<&crate::entity::Position>(npc)
            .unwrap()
            .0;
        aim_at(&mut hg, current + glam::Vec3::Y * 0.7);
        pack_wait(&mut host, &mut hg, |server, game| {
            (server.server.players[slot].yaw - game.state.players[0].camera.yaw).abs() < 0.01
        });
        let p = &hg.state.players[0];
        let picked = hg
            .state
            .remote_mobs
            .ray_target(
                p.player.eye_pos(),
                p.camera.forward(),
                crate::combat::ATTACK_REACH,
            )
            .expect("the real mirrored ordinary NPC lies on the click ray");
        assert_eq!(picked.id, id);
        assert_eq!(picked.kind, crate::mob::MobType::Villager);
        assert!(
            (current - host.server.players[slot].player.eye_pos()).length() < 4.5,
            "the server body must genuinely reach this NPC"
        );
        let before = crate::staged_window::MarkerWindow::capture_parts(
            &p.inventory,
            &p.armour_slots,
            &p.crafting_ui.cursor_item,
            &p.crafting_ui.grid,
            &hg.state.registry,
        )
        .slots();
        let reputation = p.reputation.per_village.clone();
        let kills = p.kill_counter.clone();
        let before_actions = actions.load(Ordering::Relaxed);
        right_click(&mut hg);
        let p = &hg.state.players[0];
        assert_eq!(
            crate::staged_window::MarkerWindow::capture_parts(
                &p.inventory,
                &p.armour_slots,
                &p.crafting_ui.cursor_item,
                &p.crafting_ui.grid,
                &hg.state.registry
            )
            .slots(),
            before,
            "Talk must not predict a physical cost or reward"
        );
        assert_eq!(p.reputation.per_village, reputation);
        assert_eq!(p.kill_counter, kills);
        for _ in 0..3 {
            harness_step(&mut host, &mut hg);
        }
        assert!(
            host.server.players[slot].is_in_world(),
            "the actual server attachment remains live"
        );
        assert!(
            hg.state
                .remote_client
                .as_ref()
                .is_some_and(|client| client.is_connected()),
            "the actual client remains connected after its honest click"
        );
        eprintln!(
            "ordinary NPC click {mode:?}: picked={id}, new actions={}, toast={:?}",
            actions.load(Ordering::Relaxed) - before_actions,
            hg.state.toast.as_ref().map(|t| &t.0)
        );
        assert_eq!(
            actions.load(Ordering::Relaxed) - before_actions,
            1,
            "actual joined ordinary-villager right-click sent no server dialogue action"
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_d2c3_legacy_local_quest_refusal_keeps_host_tick_and_ping_alive() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-legacy-host-liveness");
    let name = "d2c3-legacy-host-liveness";
    let mut game = HeadlessGame::boot_into_world(name);
    game.state
        .set_play_mode(crate::play_mode::PlayMode::Survival);
    assert!(game.state.world.quest_scope.is_none());
    assert!(game.state.players[0].personal_binding.is_none());
    let giver = crate::entity::spawn_mob(
        &mut game.state.ecs,
        crate::mob::MobType::Villager,
        game.state.players[0].player.pos + glam::Vec3::X,
    );
    game.state
        .ecs
        .insert_one(
            giver,
            crate::villager::VillagerComponent {
                profession: crate::villager::Profession::Farmer,
                ..Default::default()
            },
        )
        .unwrap();
    let mut quest = crate::quest::generate_quest(crate::villager::Profession::Farmer, 17).unwrap();
    quest.accepted_by = Some(0);
    game.state.players[0].active_quests.insert(giver, quest);
    assert!(
        game.state.prepare_local_gameplay().is_err(),
        "unproven original-giver conversion must still refuse"
    );
    assert!(game.state.world.quest_scope.is_none());
    let mut host = crate::hosted_server::HostedServer::start_host(
        1,
        name.into(),
        game.state.biome_gen.seed,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        crate::hosted_server::HostWorld::Lent,
    )
    .unwrap();
    let client = host.attach_test_remote();
    client.send_to_server(&crate::protocol::serialize_packet(
        crate::protocol::PacketType::Ping,
        &(),
    ));
    let before = host.server.tick_counter;
    game.state.hosted_server = Some(host);
    game.hosted_ticks(1);
    assert!(matches!(game.state.mode, crate::GameMode::Playing));
    assert!(game.state.players[0].active_quests.contains_key(&giver));
    assert!(
        game.state.world.quest_scope.is_none(),
        "refused preparation published an unproven scope"
    );
    assert!(
        game.state
            .hosted_server
            .as_ref()
            .unwrap()
            .server
            .tick_counter
            > before,
        "local legacy quest conversion refusal starved the actual hosted tick/control path"
    );
    let mut pong = false;
    while let Some(packet) = client.try_recv_from_server() {
        pong |= matches!(
            crate::protocol::deserialize_header(&packet),
            Some((crate::protocol::PacketType::Pong, _))
        );
    }
    assert!(
        pong,
        "free Ping control stopped behind the local authority preparation refusal"
    );
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_d2c3_world_local_remote_cut_returns_through_actual_native_load() {
    let _root = crate::save::WorldsRootGuard::new("d2c3-world-local-native-return");
    let secret = [66u8; 32];
    let key = nostr::Keys::new(nostr::SecretKey::from_slice(&secret).unwrap())
        .public_key()
        .to_bytes();
    let name = "d2c3-world-local-native-return";
    let mut game = HeadlessGame::boot_into_world(name);
    game.state.handover_fixture = Some(crate::handover_game::Fixture {
        signed_in: Some(key),
        recipient: None,
    });
    game.state.players[0].inventory.clear();
    game.state.players[0].inventory.set_slot(
        7,
        Some(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Bread,
            5,
        )),
    );
    game.state.players[0]
        .reputation
        .per_village
        .insert((2, -4), 17);
    game.state.players[0]
        .kill_counter
        .insert(crate::mob::MobType::Pig, 37);
    game.state.prepare_local_gameplay().unwrap();
    let scope = game
        .state
        .world
        .quest_scope
        .expect("actual local baseline has a durable random scope");
    assert!(game.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    let mut host = crate::hosted_server::HostedServer::start(
        0,
        name.into(),
        42,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
    )
    .unwrap();
    let client = host.attach_test_remote_with_binding(Some([63; 32]));
    crate::hosted_server::tests::send_signed_join(&client, secret, Some([63; 32]));
    host.tick();
    let mut accepted = None;
    let mut initial_sync = None;
    let mut initial_personal = None;
    while let Some(packet) = client.try_recv_from_server() {
        if let Some((crate::protocol::PacketType::JoinAccept, body)) =
            crate::protocol::deserialize_header(&packet)
        {
            accepted = Some(
                crate::protocol::safe_deserialize::<crate::protocol::JoinAcceptPacket>(body)
                    .unwrap(),
            );
        }
        if let Some((crate::protocol::PacketType::JoinReject, body)) =
            crate::protocol::deserialize_header(&packet)
        {
            panic!(
                "genuine signed role cut refused: {}",
                crate::protocol::safe_deserialize::<crate::protocol::JoinRejectPacket>(body)
                    .unwrap()
                    .reason
            );
        }
        if let Some((crate::protocol::PacketType::InventorySync, body)) =
            crate::protocol::deserialize_header(&packet)
        {
            initial_sync = Some(
                crate::protocol::safe_deserialize::<crate::protocol::InventorySyncPacket>(body)
                    .unwrap(),
            );
        }
        if let Some((crate::protocol::PacketType::PersonalSync, body)) =
            crate::protocol::deserialize_header(&packet)
            && let crate::protocol::PersonalSyncPacket::Snapshot(value) =
                crate::protocol::safe_deserialize(body).unwrap()
        {
            initial_personal = Some(value);
        }
    }
    let accepted = accepted.expect("actual same-person signed JoinAccept");
    let index = accepted.player_index as usize;
    // Settle this genuine attachment's actual physical Sync1 and personal2
    // before modifying its server-owned fixture body. No readiness flag is set.
    let sync = initial_sync.expect("actual initial replacement");
    assert_eq!(sync.window_event, 1);
    let snapshot = initial_personal.expect("actual initial common progression fence");
    assert_eq!(snapshot.window_event, 2);
    let mut joined_seat = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    crate::inventory_sync::apply(
        &mut crate::window::WindowMut {
            inv: &mut joined_seat.inventory,
            armour: &mut joined_seat.armour_slots,
            cursor: &mut joined_seat.crafting_ui.cursor_item,
            grid: &mut joined_seat.crafting_ui.grid,
            container: None,
        },
        &sync,
        &crate::block::BlockRegistry::new(),
        &mut Default::default(),
        &mut Default::default(),
        accepted.inventory_mode,
    )
    .unwrap();
    assert_eq!(joined_seat.inventory.slot(7).unwrap().count, 5);
    let mut inbox = crate::personal_sync::Inbox::new(accepted.action_session, accepted.action_life);
    inbox.push(snapshot).unwrap();
    assert!(inbox.apply(1, &mut joined_seat).unwrap().is_empty());
    let ack = inbox.apply(2, &mut joined_seat).unwrap().remove(0);
    client.send_to_server(&crate::protocol::serialize_packet(
        crate::protocol::PacketType::PersonalSync,
        &crate::protocol::PersonalSyncPacket::Applied(ack),
    ));
    host.tick();
    assert!(host.server.players[index].personal_delivery.initialised);
    assert!(
        host.server.players[index].window_events.is_quiescent(),
        "pending Replace would overwrite the later fixture body"
    );
    assert_eq!(host.server.players[index].verified_pubkey, Some(key));
    assert!(host.server.selected_local_personal.is_none());
    assert_eq!(host.server.world.quest_scope, Some(scope));
    assert_eq!(
        crate::window_events::effective_window(&host.server.players[index])
            .inv
            .slot(7)
            .unwrap()
            .count,
        5
    );
    // Capture a distinctive current authoritative body, not the old primary p0.
    host.server.players[index].inventory.set_slot(
        7,
        Some(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Bread,
            8,
        )),
    );
    host.server.credit_personal_kill(
        index,
        crate::mob::MobType::Pig,
        host.server.players[index].player.pos,
    );
    host.server.save_joiners().unwrap();
    host.server.try_save().unwrap();
    let directory = crate::save::world_dir(name);
    let selected =
        crate::save::read_world_save(&std::fs::read(directory.join("world.dat")).unwrap()).unwrap();
    assert!(
        selected.local_personal.is_none(),
        "the selected snapshot must explicitly select canonical remote authority"
    );
    let canonical_path = directory.join("joiners").join(format!(
        "{}.dat",
        crate::hosted_server::pubkey_to_npub(&key)
    ));
    let bytes = std::fs::read(&canonical_path).unwrap();
    let canonical = crate::joiner_save::JoinerSave::decode(&bytes).unwrap();
    assert!(canonical.obligations_proven_zero());
    assert_eq!(
        canonical.personal.as_ref().unwrap().record().unwrap().kills,
        vec![(crate::mob::MobType::Pig, 38)]
    );
    drop(host);
    game.state.handover_fixture = Some(crate::handover_game::Fixture {
        signed_in: Some(key),
        recipient: None,
    });
    game.state.world_name = name.into();
    game.state.mode =
        crate::GameMode::Loading(crate::loading_screen::LoadingState::new(name.into()));
    assert!(
        game.state.begin_load(),
        "the actual native loader refused a complete matching canonical body"
    );
    let actual = &game.state.players[0];
    assert_eq!(
        actual.inventory.slot(7).map(|stack| stack.count),
        Some(8),
        "native return donated positional p0/defaults instead of selecting this verified player's canonical window"
    );
    assert_eq!(
        actual.kill_counter.get(&crate::mob::MobType::Pig),
        Some(&38)
    );
    assert_eq!(actual.reputation.score((2, -4)), 17);
    assert_eq!(
        std::fs::read(&canonical_path).unwrap(),
        bytes,
        "native return must not reset the current canonical record"
    );
    assert!(game.state.local_personal.is_some());
    let baseline =
        crate::save::read_world_save(&std::fs::read(directory.join("world.dat")).unwrap()).unwrap();
    assert_eq!(
        baseline
            .local_personal
            .expect("native play needs a durable selected baseline")
            .0,
        bytes
    );
}

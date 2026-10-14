//! Genuine joined ordinary-villager click, without a fabricated Talk wire.
use super::*;
use crate::action_identity::InventoryMode;
use crate::transport::{ChannelClientTransport, ClientTransport};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

struct CountActions {
    inner: ChannelClientTransport,
    actions: Arc<AtomicUsize>,
}
impl ClientTransport for CountActions {
    fn send_to_server(&self, data: &[u8]) {
        if matches!(crate::protocol::deserialize_header(data),
            Some((crate::protocol::PacketType::Action, _)))
        {
            self.actions.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.send_to_server(data);
    }
    fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
        self.inner.try_recv_from_server()
    }
    fn is_closed(&self) -> bool { self.inner.is_closed() }
    fn channel_binding(&self) -> Option<[u8; 32]> { self.inner.channel_binding() }
}

fn joined_counted(mode: InventoryMode) -> (HeadlessGame, crate::hosted_server::HostedServer, usize, Arc<AtomicUsize>) {
    let tag = format!("d2c3-dialogue-{mode:?}");
    let mut hg = HeadlessGame::boot_into_world(&format!("harness-{tag}"));
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    let mut host = crate::hosted_server::HostedServer::start(0,
        format!("harness-{tag}-server-{}", std::process::id()), 42, 0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 }).unwrap();
    host.select_inventory_mode(mode).unwrap();
    host.server.difficulty = crate::survival::Difficulty::Peaceful;
    let actions = Arc::new(AtomicUsize::new(0));
    let transport = CountActions { inner: host.attach_test_remote(), actions: actions.clone() };
    hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
        Box::new(transport), crate::remote_client::build_join_request_guest("Quest guest", 0), None));
    pack_wait(&mut host, &mut hg, |_, game| game.state.remote_client.as_ref()
        .is_some_and(|client| client.is_connected() && client.personal_ready()));
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
        host.server.ecs.get::<&mut crate::villager::VillagerComponent>(npc).unwrap().profession = crate::villager::Profession::Farmer;
        pack_wait(&mut host, &mut hg, |server, game| server.server.ecs
            .get::<&crate::entity::ProtocolId>(npc).is_ok_and(|id|
                game.state.remote_mobs.position_of(id.0).is_some()));
        let id = host.server.ecs.get::<&crate::entity::ProtocolId>(npc).unwrap().0;
        let current = host.server.ecs.get::<&crate::entity::Position>(npc).unwrap().0;
        aim_at(&mut hg, current + glam::Vec3::Y * 0.7);
        pack_wait(&mut host, &mut hg, |server, game|
            (server.server.players[slot].yaw - game.state.players[0].camera.yaw).abs() < 0.01);
        let p = &hg.state.players[0];
        let picked = hg.state.remote_mobs.ray_target(p.player.eye_pos(), p.camera.forward(), crate::combat::ATTACK_REACH)
            .expect("the real mirrored ordinary NPC lies on the click ray");
        assert_eq!(picked.id, id);
        assert_eq!(picked.kind, crate::mob::MobType::Villager);
        assert!((current - host.server.players[slot].player.eye_pos()).length() < 4.5,
            "the server body must genuinely reach this NPC");
        let before = crate::staged_window::MarkerWindow::capture_parts(&p.inventory, &p.armour_slots,
            &p.crafting_ui.cursor_item, &p.crafting_ui.grid, &hg.state.registry).slots();
        let reputation = p.reputation.per_village.clone();
        let kills = p.kill_counter.clone();
        let before_actions = actions.load(Ordering::Relaxed);
        right_click(&mut hg);
        let p = &hg.state.players[0];
        assert_eq!(crate::staged_window::MarkerWindow::capture_parts(&p.inventory, &p.armour_slots,
            &p.crafting_ui.cursor_item, &p.crafting_ui.grid, &hg.state.registry).slots(), before,
            "Talk must not predict a physical cost or reward");
        assert_eq!(p.reputation.per_village, reputation);
        assert_eq!(p.kill_counter, kills);
        for _ in 0..3 { harness_step(&mut host, &mut hg); }
        assert!(!host.disconnected[slot]);
        eprintln!("ordinary NPC click {mode:?}: picked={id}, new actions={}, toast={:?}",
            actions.load(Ordering::Relaxed) - before_actions, hg.state.toast.as_ref().map(|t| &t.0));
        assert_eq!(actions.load(Ordering::Relaxed) - before_actions, 1,
            "actual joined ordinary-villager right-click sent no server dialogue action");
    }
}

//! Authenticated mount, real connection-slot reuse and durable keeper reads.
use super::*;

fn attach(hs: &mut HostedServer, signed: bool, binding: u8) -> (usize, ChannelClientTransport, protocol::JoinAcceptPacket) {
    let client = if signed { hs.attach_test_remote_with_binding(Some([binding; 32])) } else { hs.attach_test_remote() };
    if signed {
        super::super::tests::send_signed_join(&client, super::super::c3d_tests::KEY, Some([binding; 32]));
    } else {
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &crate::remote_client::build_join_request_guest("Guest", 0)));
    }
    hs.tick();
    let heard = super::super::c3d_tests::hear(&client);
    assert!(heard.rejects.is_empty(), "the genuine join is admitted");
    let accept = heard.accepts.into_iter().next().expect("a real JoinAccept");
    let i = accept.player_index as usize;
    let event = hs.server.players[i].window_events.last_number();
    crate::window_events::apply_through(&mut hs.server.players[i], event, hs.server.tick_counter);
    hs.server.players[i].inventory = crate::inventory::Inventory::new();
    (i, client, accept)
}

fn mount(hs: &mut HostedServer, i: usize, client: &ChannelClientTransport, accept: &protocol::JoinAcceptPacket, entity: hecs::Entity, mode: InventoryMode, stage: &str) {
    let target = hs.server.ecs.get::<&crate::entity::Position>(entity).unwrap().0;
    let id = hs.server.ecs.get::<&crate::entity::ProtocolId>(entity).unwrap().0;
    let sp = &mut hs.server.players[i];
    sp.player.pos = target + glam::Vec3::Z * 1.5;
    sp.player.velocity = glam::Vec3::ZERO;
    let towards = (target + glam::Vec3::Y * 0.7 - sp.player.eye_pos()).normalize();
    sp.yaw = (-towards.x).atan2(-towards.z);
    sp.pitch = towards.y.asin();
    let boundary = sp.window_events.last_number();
    let (held_kind, held_id) = protocol::ItemRef::Empty.to_wire();
    let req = protocol::EntityInteractPacket { seq: 1, entity: id, kind: protocol::InteractKind::Mount, held_kind, held_id, held_full: protocol::WireItem::None, hotbar_slot: 0, sneak: false, events_applied: boundary };
    let bytes = match mode {
        InventoryMode::Mirror => protocol::serialize_packet(protocol::PacketType::EntityInteract, &req),
        InventoryMode::Strict => protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action: ActionHeader { session: accept.action_session, epoch: 0, life: accept.action_life, seq: 1, events_applied: boundary }, intent: ActionIntent::Interact(req) }),
    };
    client.send_to_server(&bytes);
    let mut accepted = false;
    let mut outcomes = Vec::new();
    for _ in 0..16 {
        hs.tick();
        while let Some(bytes) = client.try_recv_from_server() {
            match protocol::deserialize_header(&bytes) {
                Some((protocol::PacketType::InteractOutcome, payload)) => {
                    let outcome: protocol::InteractOutcomePacket = protocol::safe_deserialize(payload).unwrap();
                    accepted |= outcome.accepted;
                    outcomes.push((outcome.accepted, crate::mob_interact::InteractNote::from_wire(outcome.note)));
                }
                Some((protocol::PacketType::JoinReject, payload)) => panic!("honest mount disconnected: {}", protocol::safe_deserialize::<protocol::JoinRejectPacket>(payload).unwrap().reason),
                _ => {}
            }
        }
        if accepted { break; }
    }
    assert!(accepted, "actual joined mount {mode:?}/{stage} refused: {outcomes:?}; body {:?}, target {:?}, alive {}, cooldown {}, riding {:?}, entity ridden {}", hs.server.players[i].player.pos, hs.server.ecs.get::<&crate::entity::Position>(entity).unwrap().0, hs.server.players[i].is_present_and_alive(), hs.server.players[i].interact_cooldown, hs.server.players[i].riding, hs.server.ecs.get::<&crate::entity::Ridden>(entity).is_ok());
    assert_eq!(hs.server.players[i].riding, Some(entity));
}

#[test]
fn d2c2_authenticated_keeper_survives_reconnect_slot_reuse_and_restart_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("authenticated-keeping", mode);
        let owner = rig.hs.server.players[rig.joiner.i].pet_owner_key().unwrap();
        assert!(owner.starts_with("npub1"));
        {
            let mut data = rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap();
            data.keeper = None;
            data.kept_by = None;
            let mut pack = crate::chest::ChestData::new();
            pack.slots[8] = Some(ItemStack::new_block(block::STONE, 9));
            data.pack = Some(pack);
        }
        rig.hs.server.ecs.insert_one(rig.entity, crate::entity::Scattered).unwrap();
        mount(&mut rig.hs, rig.joiner.i, &rig.joiner.client, &rig.joiner.accept, rig.entity, mode, "first keeping");
        assert!(rig.hs.server.ecs.get::<&crate::entity::Scattered>(rig.entity).is_err());
        assert_eq!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().keeper_identity(), Some(crate::horse_ai::SteedKeeper::Npub(owner.clone())));
        let original_slot = rig.joiner.i;
        rig.hs.release_slot(original_slot, None);
        let (guest, guest_client, guest_accept) = attach(&mut rig.hs, false, 0);
        assert_eq!(guest, original_slot, "a genuinely new connection reuses the old numeric seat");
        assert!(!crate::tameable::is_own_pet(&rig.hs.server.ecs, rig.entity, &rig.hs.server.players[guest].pet_owner_key().unwrap_or_default(), None));
        let (rejoined, signed_client, signed_accept) = attach(&mut rig.hs, true, 34);
        assert_ne!(rejoined, original_slot, "the authenticated owner is now at another connection index");
        assert_eq!(rig.hs.server.players[rejoined].pet_owner_key().as_deref(), Some(owner.as_str()));
        assert!(crate::tameable::is_own_pet(&rig.hs.server.ecs, rig.entity, &owner, None));
        mount(&mut rig.hs, rejoined, &signed_client, &signed_accept, rig.entity, mode, "authenticated reconnect");
        rig.hs.release_slot(rejoined, None);
        // release_slot queues the mount release; the actual simulation tick
        // clears Ridden after inbound processing. A request before that tick
        // honestly sees an occupied mount, rather than a transferred keeper.
        assert!(rig.hs.server.ecs.get::<&crate::entity::Ridden>(rig.entity).is_ok());
        eprintln!("keeper {mode:?}: released owner slot {rejoined}, Ridden still present before actual simulation cleanup");
        rig.hs.tick();
        assert!(rig.hs.server.ecs.get::<&crate::entity::Ridden>(rig.entity).is_err());
        mount(&mut rig.hs, guest, &guest_client, &guest_accept, rig.entity, mode, "later guest");
        assert_eq!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().keeper_identity(), Some(crate::horse_ai::SteedKeeper::Npub(owner.clone())), "a later guest rider cannot replace authenticated keeping");
        rig.hs.release_slot(guest, None);
        rig.hs.server.try_save().unwrap();
        let name = rig.hs.server.world_name.clone();
        drop(rig.hs);
        let mut restored = HostedServer::start(0, name, 42, 0, RemoteTransport::WebSocket { port: 0 }).unwrap();
        restored.select_inventory_mode(mode).unwrap();
        let (i, client, accept) = attach(&mut restored, true, 51);
        let entities: Vec<_> = restored.server.ecs.query::<&crate::horse_ai::HorseData>().iter()
            .filter(|(_, data)| data.keeper_identity() == Some(crate::horse_ai::SteedKeeper::Npub(owner.clone())))
            .map(|(entity, _)| entity).collect();
        assert_eq!(entities.len(), 1, "restart restores exactly one kept steed");
        let entity = entities[0];
        assert_eq!(restored.server.ecs.get::<&crate::horse_ai::HorseData>(entity).unwrap().pack.as_ref().unwrap().slots[8].as_ref().unwrap().count, 9);
        assert!(crate::tameable::is_own_pet(&restored.server.ecs, entity, &restored.server.players[i].pet_owner_key().unwrap(), None));
        mount(&mut restored, i, &client, &accept, entity, mode, "durable restart");
        assert_eq!(restored.server.ecs.get::<&crate::horse_ai::HorseData>(entity).unwrap().keeper_identity(), Some(crate::horse_ai::SteedKeeper::Npub(owner)));
    }
}

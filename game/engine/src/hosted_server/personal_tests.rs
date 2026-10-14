//! Real inbound order when a newer snapshot is delayed past the initial ack.
use super::*;
use crate::transport::ClientTransport as _;

fn snapshot_from(client: &crate::transport::ChannelClientTransport) -> protocol::PersonalSnapshot {
    while let Some(packet) = client.try_recv_from_server() {
        if let Some((protocol::PacketType::PersonalSync, bytes)) =
            protocol::deserialize_header(&packet)
            && let protocol::PersonalSyncPacket::Snapshot(value) =
                protocol::safe_deserialize(bytes).unwrap()
        {
            return value;
        }
    }
    panic!("server did not issue the retained personal snapshot")
}

#[test]
fn handover_personal_delayed_supersession_keeps_honest_action_and_current_freeze_fence() {
    let _root = crate::save::WorldsRootGuard::new("personal-delayed-initial");
    let mut hs = super::super::tests::start_room_test_server("personal-delayed-initial", 0);
    let (i, client, heard) = super::super::c3d_tests::join(&mut hs, 17);
    let accept = heard.accepts.first().unwrap();
    assert_eq!(
        accept.inventory_mode,
        crate::action_identity::InventoryMode::Mirror
    );
    let key = hs.server.players[i].verified_pubkey.unwrap();
    let context = crate::handover::personal::RecordContext {
        participant: key,
        lineage: [1; 16],
        generation: 9,
        transfer: [4; 16],
    };
    let record =
        crate::handover::personal::PersonalRecord::from_maps(context, [], [], [], None).unwrap();
    hs.server.players[i].personal = Some(
        crate::joiner_save::BoundPersonal::new(
            crate::joiner_save::PersonalBinding {
                context,
                manifest: [1; 32],
                core: [2; 32],
            },
            &record,
        )
        .unwrap(),
    );
    // The synthetic personal holder needs the same initial Replace boundary
    // a restored imported join has before its first personal snapshot.
    let window = crate::window_events::effective_window(&hs.server.players[i]);
    let first_replace = hs.send_inventory_sync(i, window, Default::default());
    hs.start_personal(i, first_replace, false).unwrap();
    let first = snapshot_from(&client);
    assert_eq!(first.window_event, first_replace + 1, "personal state owns its genuine preceding fence");
    let first_event = first.window_event;
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    let mut inbox = crate::personal_sync::Inbox::new(accept.action_session, 0);
    inbox.push(first.clone()).unwrap();
    let first_ack = inbox.apply(first_event, &mut slot).unwrap().remove(0);
    // Issue a new numbered Replace/snapshot before the first ack reaches the
    // server. Its bytes are deliberately withheld from the honest client.
    let window = crate::window_events::effective_window(&hs.server.players[i]);
    let second_replace = hs.send_inventory_sync(i, window, Default::default());
    hs.start_personal(i, second_replace, true).unwrap();
    let second = snapshot_from(&client);
    assert_eq!(second.window_event, second_replace + 1);
    let second_event = second.window_event;
    assert!(second.revision > first.revision);
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::PersonalSync,
        &protocol::PersonalSyncPacket::Applied(first_ack),
    ));
    let action = hs.server.players[i]
        .action_frontier
        .as_ref()
        .unwrap()
        .header(1, first_event);
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::Action,
        &crate::action_wire::ActionPacket {
            action,
            intent: crate::action_wire::ActionIntent::CancelledNoop,
        },
    ));
    hs.tick();
    assert!(
        !hs.disconnected[i],
        "an honest action behind its exact initial Applied was torn down by a delayed newer snapshot"
    );
    let receipts = super::super::c3d_tests::hear(&client).receipts;
    assert_eq!(
        receipts.len(),
        1,
        "the honest ordered action never received its terminal result"
    );
    assert_eq!(receipts[0].action, action);
    assert_eq!(
        receipts[0].fate,
        crate::action_identity::ActionFate::Superseded
    );
    let sp = &hs.server.players[i];
    assert!(sp.personal_delivery.initialised);
    assert!(!sp.window_events.accepts_boundary(first_event - 1), "the real initial Applied advanced its own server prefix");
    assert_eq!(
        sp.personal_delivery.pending.as_ref().unwrap().revision,
        second.revision,
        "initial Applied released the newer personal tuple"
    );
    assert!(
        !sp.personal_delivery
            .quiescent(sp.personal.as_ref().unwrap(), sp.respawns)
    );
    let (generation, _) = hs.handover_seat(i).unwrap();
    let _lease = hs.handover_bind(i, generation, key, [8; 16]).unwrap();
    assert!(
        hs.handover_freeze(None).is_err(),
        "capture overtook unretired typed/personal/window state"
    );
    // The newer state installs only at its own actually applied window.
    inbox.push(second.clone()).unwrap();
    assert!(inbox.apply(first_event, &mut slot).unwrap().is_empty());
    let second_ack = inbox.apply(second_event, &mut slot).unwrap().remove(0);
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::PersonalSync,
        &protocol::PersonalSyncPacket::Applied(second_ack),
    ));
    hs.tick();
    assert!(hs.server.players[i].window_events.is_quiescent(), "the actual current Applied retired its no-window-change fence");
    assert!(
        hs.server.players[i]
            .personal_delivery
            .quiescent(hs.server.players[i].personal.as_ref().unwrap(), 0)
    );
    assert!(
        hs.handover_freeze(None).is_err(),
        "personal Applied bypassed the retained typed result's Applied fence"
    );
    let applied = crate::action_wire::AppliedActionAck {
        session: action.session,
        epoch: action.epoch,
        life: action.life,
        through: action.seq,
        included_events: second_event,
    };
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::AppliedActionAck,
        &applied,
    ));
    hs.tick();
    assert!(!hs.disconnected[i]);
    hs.handover_freeze(None).unwrap();
    assert!(
        hs.handover_frozen(),
        "complete current personal/window/action fences never released freeze"
    );
}

#[test]
fn d2c2_personal_applied_control_keeps_older_budgeted_strict_actions_in_order() {
    use crate::{action_identity::ActionFate, action_wire::{ActionIntent, ActionPacket, AppliedActionAck}, item::{Item, ItemStack}};
    let mut rig = super::super::c3d_strict_tests::strict_rig("personal-paced-authority");
    let i = rig.i;
    let key = rig.hs.server.players[i].verified_pubkey.unwrap();
    let context = crate::handover::personal::RecordContext { participant: key, lineage: [1; 16], generation: 9, transfer: [4; 16] };
    let record = crate::handover::personal::PersonalRecord::from_maps(context, [], [], [], None).unwrap();
    rig.hs.server.players[i].personal = Some(crate::joiner_save::BoundPersonal::new(crate::joiner_save::PersonalBinding { context, manifest: [1; 32], core: [2; 32] }, &record).unwrap());
    rig.hs.server.players[i].inventory = crate::inventory::Inventory::new();
    rig.hs.start_personal(i, rig.applied, false).unwrap();
    let first = snapshot_from(&rig.client);
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    let mut inbox = crate::personal_sync::Inbox::new(rig.accept.action_session, 0);
    inbox.push(first.clone()).unwrap();
    let first_ack = inbox.apply(first.window_event, &mut slot).unwrap().remove(0);
    rig.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::PersonalSync, &protocol::PersonalSyncPacket::Applied(first_ack)));
    rig.hs.tick();
    assert!(rig.hs.server.players[i].personal_delivery.initialised);
    rig.applied = first.window_event;
    let base = rig.applied;
    // The supported client may send several old-boundary configuration ops
    // while a newer server snapshot is delayed in reliable transit.
    let raw = crate::window_events::pack_raw_marker(&rig.hs.server.players[i], &rig.hs.server.registry).unwrap();
    let count = super::super::MAX_WINDOW_OPS_PER_TICK + 1;
    let mut packets = Vec::new();
    for sequence in 1..=count {
        let action = rig.next();
        let on = sequence.is_multiple_of(2);
        let mut candidate = raw.clone();
        candidate.set_auto_refill(on);
        let op = protocol::WindowOpPacket { op_seq: sequence as u32, op: protocol::WireWindowOp::SetAutoRefill { on }, digest: candidate.digest_with(crate::window::Station::Player, None), events_applied: base, touched: Vec::new(), claims: Vec::new(), client_ok: true };
        packets.push(ActionPacket { action, intent: ActionIntent::Window(op) });
    }
    // Genuine progression update and grant paths issue the newer fence;
    // no Replace makes these honest old action boundaries stale.
    rig.hs.server.credit_personal_kill(i, crate::mob::MobType::Cow, glam::Vec3::ZERO);
    rig.hs.grant_to_joiner(i, [ItemStack::new_block(crate::block::STONE, 2)]);
    let grant_event = rig.hs.server.players[i].window_events.last_number();
    rig.hs.flush_personal_updates();
    let latest = snapshot_from(&rig.client);
    assert_eq!(latest.window_event, grant_event + 1);
    assert!(latest.revision > first.revision);
    for packet in &packets {
        rig.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, packet));
    }
    inbox.push(latest.clone()).unwrap();
    assert!(inbox.apply(base, &mut slot).unwrap().is_empty());
    let ack = inbox.apply(latest.window_event, &mut slot).unwrap().remove(0);
    rig.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::PersonalSync, &protocol::PersonalSyncPacket::Applied(ack)));
    rig.hs.tick();
    assert!(!rig.hs.disconnected[i]);
    assert_eq!(rig.through(), (count - 1) as u64, "one real typed configuration remains budget-delayed");
    assert_eq!(rig.hs.server.players[i].action_queue.as_ref().unwrap().len(), 1);
    assert!(rig.hs.server.players[i].window_events.accepts_boundary(base), "a free personal control overtook its older authorisation queue");
    assert!(rig.hs.server.players[i].inventory.slots_iter().flatten().all(|stack| stack.item != Item::Block(crate::block::STONE)), "the prior grant must not fold ahead of the delayed authority");
    let (generation, _) = rig.hs.handover_seat(i).unwrap();
    let _lease = rig.hs.handover_bind(i, generation, key, [8; 16]).unwrap();
    assert!(rig.hs.handover_freeze(None).is_err());
    let mut receipts = super::super::c3d_tests::hear(&rig.client).receipts;
    for _ in 0..16 {
        if rig.through() == count as u64 && rig.hs.server.players[i].window_events.is_quiescent() { break; }
        rig.hs.tick();
        assert!(!rig.hs.disconnected[i], "an honest paced action hit FellBehind after personal Applied");
        receipts.extend(super::super::c3d_tests::hear(&rig.client).receipts);
    }
    assert_eq!(rig.through(), count as u64);
    assert!(rig.hs.server.players[i].window_events.is_quiescent(), "the accepted personal fence must retire after the older queue settles");
    assert_eq!(receipts.len(), count);
    assert!(receipts.iter().all(|receipt| receipt.fate == ActionFate::Accepted));
    assert_eq!(rig.hs.server.players[i].inventory.slots_iter().flatten().filter(|stack| stack.item == Item::Block(crate::block::STONE)).map(|stack| u32::from(stack.count)).sum::<u32>(), 2);
    assert!(rig.hs.handover_freeze(None).is_err(), "personal Applied cannot retire the typed results");
    let last = receipts.iter().max_by_key(|receipt| receipt.action.seq).unwrap();
    rig.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::AppliedActionAck, &AppliedActionAck { session: last.action.session, epoch: last.action.epoch, life: last.action.life, through: last.action.seq, included_events: last.included_events }));
    rig.hs.tick();
    assert!(!rig.hs.disconnected[i]);
    rig.hs.handover_freeze(None).unwrap();
}

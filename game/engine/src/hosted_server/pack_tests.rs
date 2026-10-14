//! Real joined pack dispatch and metadata retirement in both advertised modes.
use super::*;
use crate::{block, action_identity::{ActionHeader, InventoryMode}, action_wire::{ActionIntent, ActionPacket, AppliedActionAck}, container_window::{ContainerClick, ContainerTarget}, item::{Item, ItemStack}, pack_wire::{PackNote, PackOperation, PackRequest, PackResult}, transport::ClientTransport as _};

struct Joiner { i: usize, client: ChannelClientTransport, accept: protocol::JoinAcceptPacket, sequence: u64, applied: u32 }
struct Rig { _root: crate::save::WorldsRootGuard, hs: HostedServer, joiner: Joiner, entity: hecs::Entity, target: ContainerTarget }
impl Rig {
    fn new(tag: &str, mode: InventoryMode) -> Self {
        let root = crate::save::WorldsRootGuard::new(&format!("d2c2-pack-{tag}-{mode:?}"));
        let mut hs = super::tests::start_room_test_server(&format!("d2c2-pack-{tag}-{mode:?}"), 0);
        hs.select_inventory_mode(mode).unwrap();
        let (i, client, mut heard) = super::c3d_tests::join(&mut hs, 17);
        let accept = heard.accepts.pop().expect("a real signed join");
        let applied = hs.server.players[i].window_events.last_number();
        crate::window_events::apply_through(&mut hs.server.players[i], applied, hs.server.tick_counter);
        hs.server.players[i].inventory = crate::inventory::Inventory::new();
        let pos = hs.server.players[i].player.pos + crate::camera::forward_from(hs.server.players[i].yaw, 0.0) * 1.5;
        let entity = crate::entity::spawn_mob(&mut hs.server.ecs, crate::mob::MobType::Donkey, pos);
        hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(entity).unwrap().keeper = Some(crate::horse_ai::SteedKeeper::LegacyLocal(0));
        hs.tick();
        while client.try_recv_from_server().is_some() {}
        let id = hs.server.ecs.get::<&crate::entity::ProtocolId>(entity).unwrap().0;
        Self { _root: root, hs, joiner: Joiner { i, client, accept, sequence: 0, applied }, entity, target: ContainerTarget::Entity(id) }
    }
    fn packet(&mut self, operation: PackOperation) -> ActionPacket {
        self.joiner.sequence += 1;
        let sp = &self.hs.server.players[self.joiner.i];
        let raw = crate::window_events::pack_raw_marker(sp, &self.hs.server.registry).unwrap();
        ActionPacket { action: ActionHeader { session: self.joiner.accept.action_session, epoch: 0, life: self.joiner.accept.action_life, seq: self.joiner.sequence, events_applied: self.joiner.applied }, intent: ActionIntent::Pack(PackRequest { actor_respawns: sp.respawns, target: self.target, hotbar_slot: 0, before: raw.digest_with(sp.station, None), operation }) }
    }
    fn execute(&mut self, operation: PackOperation) -> (ActionPacket, PackResult) {
        let packet = self.packet(operation);
        let result = self.send(&packet);
        (packet, result)
    }
    fn send(&mut self, packet: &ActionPacket) -> PackResult {
        self.joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, packet));
        let mut result = None;
        let mut result_before_receipt = false;
        for _ in 0..16 {
            self.hs.tick();
            while let Some(bytes) = self.joiner.client.try_recv_from_server() {
                match protocol::deserialize_header(&bytes) {
                    Some((protocol::PacketType::PackResult, payload)) => {
                        let parsed = PackResult::decode(payload).unwrap();
                        if parsed.action == packet.action { result_before_receipt = true; result = Some(parsed); }
                    }
                    Some((protocol::PacketType::ActionReceipt, payload)) => {
                        let receipt: crate::action_wire::ActionReceipt = protocol::safe_deserialize(payload).unwrap();
                        if receipt.action == packet.action { assert!(result_before_receipt, "the immutable result reaches world FIFO before its receipt"); }
                    }
                    Some((protocol::PacketType::JoinReject, payload)) => panic!("honest pack action disconnected: {:?}", protocol::safe_deserialize::<protocol::JoinRejectPacket>(payload).unwrap().reason),
                    _ => {}
                }
            }
            if result.is_some() { break; }
        }
        result.expect("bounded state wait reached a real PackResult")
    }
    fn apply(&mut self, result: &PackResult) {
        self.joiner.applied = self.joiner.applied.max(result.window_event);
        self.joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::AppliedActionAck, &AppliedActionAck { session: result.action.session, epoch: result.action.epoch, life: result.action.life, through: result.action.seq, included_events: result.window_event }));
        self.hs.tick();
        assert!(self.hs.server.players[self.joiner.i].pack_flight.is_none(), "the real validated applied metadata releases the flight");
    }
    fn contents(&self) -> crate::chest::ChestData { self.hs.server.ecs.get::<&crate::horse_ai::HorseData>(self.entity).unwrap().pack.clone().expect("live pack") }
    fn session(&self) -> crate::pack_state::PackSession { self.hs.server.players[self.joiner.i].pack_session.expect("real opened session") }
    fn bag(&self, block: u16) -> u32 { count(&crate::window_events::effective_window(&self.hs.server.players[self.joiner.i]).inv, block) }
}
fn count(inv: &crate::inventory::Inventory, block: u16) -> u32 { inv.slots_iter().flatten().filter(|stack| stack.item == Item::Block(block)).map(|stack| u32::from(stack.count)).sum() }

#[test]
fn d2c2_real_joined_equip_open_deposit_withdraw_remove_conserves_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("round-trip", mode);
        let sp = &mut rig.hs.server.players[rig.joiner.i];
        sp.inventory.set_slot(0, Some(ItemStack::new_block(block::CHEST, 1)));
        sp.inventory.set_slot(1, Some(ItemStack::new_block(block::STONE, 4)));
        let (equip_packet, equip) = rig.execute(PackOperation::Equip);
        assert_eq!(equip.note, PackNote::Done);
        assert!(equip.view.is_none(), "equipping preserves the ordinary closed-screen gesture");
        assert_eq!(rig.contents().slots.len(), 27);
        assert_eq!(rig.bag(block::CHEST), 0);
        let repeated = rig.send(&equip_packet);
        assert_eq!(equip, repeated, "a duplicate returns the original exact outcome");
        assert_eq!(rig.bag(block::CHEST), 0);
        rig.apply(&equip);
        let (_, opened) = rig.execute(PackOperation::Open);
        assert_eq!(opened.view.as_ref().unwrap().target, rig.target);
        rig.apply(&opened);
        let session = rig.session();
        let (_, deposited) = rig.execute(PackOperation::Click { incarnation: session.incarnation, revision: session.revision, click: ContainerClick::Deposit { slot: 1, all: true } });
        assert_eq!(deposited.note, PackNote::Done);
        assert_eq!(rig.contents().slots[0].as_ref().unwrap().count, 4);
        assert_eq!(rig.bag(block::STONE), 0);
        rig.apply(&deposited);
        let session = rig.session();
        let (_, refused) = rig.execute(PackOperation::Remove { incarnation: session.incarnation, revision: session.revision });
        assert_eq!(refused.note, PackNote::NotEmpty);
        assert_eq!(rig.bag(block::CHEST), 0);
        assert_eq!(rig.contents().slots[0].as_ref().unwrap().count, 4);
        rig.apply(&refused);
        let session = rig.session();
        let (_, taken) = rig.execute(PackOperation::Click { incarnation: session.incarnation, revision: session.revision, click: ContainerClick::TakeAll });
        assert_eq!(taken.note, PackNote::Done);
        assert!(rig.contents().slots.iter().all(Option::is_none));
        assert_eq!(rig.bag(block::STONE), 4);
        rig.apply(&taken);
        let session = rig.session();
        let (_, removed) = rig.execute(PackOperation::Remove { incarnation: session.incarnation, revision: session.revision });
        assert_eq!(removed.note, PackNote::Done);
        assert!(removed.view.is_none());
        assert!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().pack.is_none());
        assert_eq!(rig.bag(block::CHEST), 1);
        assert_eq!(rig.bag(block::STONE), 4);
        rig.apply(&removed);
        assert!(!rig.hs.disconnected[rig.joiner.i]);
    }
}

#[test]
fn d2c2_current_revision_refuses_a_raced_take_without_honest_disconnect() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("stale-view", mode);
        let mut cargo = crate::chest::ChestData::new();
        cargo.slots[0] = Some(ItemStack::new_block(block::STONE, 1));
        rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(cargo);
        let (_, opened) = rig.execute(PackOperation::Open);
        rig.apply(&opened);
        let old = rig.session();
        let request = rig.packet(PackOperation::Click { incarnation: old.incarnation, revision: old.revision, click: ContainerClick::TakeAll });
        rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack.as_mut().unwrap().slots[0] = None;
        rig.hs.server.ecs.insert_one(rig.entity, crate::pack_state::PackRevision(old.revision + 1)).unwrap();
        let refused = rig.send(&request);
        assert_eq!(refused.note, PackNote::Updated);
        assert_eq!(rig.bag(block::STONE), 0, "the old view cannot mint the taken cargo");
        assert!(rig.contents().slots[0].is_none());
        assert_eq!(refused.view.as_ref().unwrap().revision, old.revision + 1);
        rig.apply(&refused);
        assert!(!rig.hs.disconnected[rig.joiner.i]);
    }
}

#[test]
fn d2c2_empty_removal_with_full_bag_spills_exactly_one_chest() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("remove-spill", mode);
        rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(crate::chest::ChestData::new());
        for slot in 0..36 { rig.hs.server.players[rig.joiner.i].inventory.set_slot(slot, Some(ItemStack::new_block(block::STONE, 64))); }
        let (_, opened) = rig.execute(PackOperation::Open);
        rig.apply(&opened);
        let session = rig.session();
        let (request, removed) = rig.execute(PackOperation::Remove { incarnation: session.incarnation, revision: session.revision });
        assert_eq!(removed.note, PackNote::Done);
        assert_eq!(rig.bag(block::CHEST), 0);
        let ground = |rig: &Rig| rig.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().filter(|(_, item)| item.stack.item == Item::Block(block::CHEST)).map(|(_, item)| u32::from(item.stack.count)).sum::<u32>();
        assert_eq!(ground(&rig), 1);
        assert_eq!(rig.send(&request), removed);
        assert_eq!(ground(&rig), 1, "terminal re-send does not spawn a second Chest");
        rig.apply(&removed);
    }
}

#[test]
fn d2c2_real_two_joiners_contest_one_unit_without_duplication() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("two-joiners", mode);
        let mut cargo = crate::chest::ChestData::new();
        cargo.slots[0] = Some(ItemStack::new_block(block::STONE, 1));
        rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(cargo);
        let (_, opened) = rig.execute(PackOperation::Open);
        rig.apply(&opened);
        let first_session = rig.session();
        let first_request = rig.packet(PackOperation::Click { incarnation: first_session.incarnation, revision: first_session.revision, click: ContainerClick::TakeAll });

        let client = rig.hs.attach_test_remote_with_binding(Some([34; 32]));
        super::tests::send_signed_join(&client, [67; 32], Some([34; 32]));
        rig.hs.tick();
        let j = rig.hs.server.players.len() - 1;
        let accept = super::c3d_tests::hear(&client).accepts.pop().unwrap();
        let applied = rig.hs.server.players[j].window_events.last_number();
        crate::window_events::apply_through(&mut rig.hs.server.players[j], applied, rig.hs.server.tick_counter);
        rig.hs.server.players[j].inventory = crate::inventory::Inventory::new();
        rig.hs.server.players[j].player.pos = rig.hs.server.players[rig.joiner.i].player.pos;
        rig.hs.server.players[j].yaw = rig.hs.server.players[rig.joiner.i].yaw;
        let request_for = |hs: &HostedServer, seq: u64, applied: u32, operation| {
            let sp = &hs.server.players[j];
            let raw = crate::window_events::pack_raw_marker(sp, &hs.server.registry).unwrap();
            ActionPacket { action: ActionHeader { session: accept.action_session, epoch: 0, life: accept.action_life, seq, events_applied: applied }, intent: ActionIntent::Pack(PackRequest { actor_respawns: sp.respawns, target: rig.target, hotbar_slot: 0, before: raw.digest_with(sp.station, None), operation }) }
        };
        let open = request_for(&rig.hs, 1, applied, PackOperation::Open);
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &open));
        rig.hs.tick();
        let mut second_open = None;
        while let Some(bytes) = client.try_recv_from_server() {
            if let Some((protocol::PacketType::PackResult, payload)) = protocol::deserialize_header(&bytes) { second_open = Some(PackResult::decode(payload).unwrap()); }
        }
        let opened = second_open.expect("second joined open");
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::AppliedActionAck, &AppliedActionAck { session: accept.action_session, epoch: 0, life: accept.action_life, through: 1, included_events: opened.window_event }));
        rig.hs.tick();
        let session = rig.hs.server.players[j].pack_session.unwrap();
        let second_request = request_for(&rig.hs, 2, opened.window_event, PackOperation::Click { incarnation: session.incarnation, revision: session.revision, click: ContainerClick::TakeAll });
        rig.joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &first_request));
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &second_request));
        rig.hs.tick();
        let mut results = Vec::new();
        for transport in [&rig.joiner.client, &client] {
            while let Some(bytes) = transport.try_recv_from_server() {
                if let Some((protocol::PacketType::PackResult, payload)) = protocol::deserialize_header(&bytes) { results.push(PackResult::decode(payload).unwrap()); }
            }
        }
        assert_eq!(results.len(), 2, "both receive exact terminals");
        assert_eq!(results.iter().filter(|result| result.note == PackNote::Done).count(), 1);
        assert_eq!(results.iter().filter(|result| result.note == PackNote::Updated).count(), 1);
        let total = rig.bag(block::STONE) + count(&crate::window_events::effective_window(&rig.hs.server.players[j]).inv, block::STONE);
        assert_eq!(total, 1, "one authoritative unit reaches one personal window");
        assert!(rig.contents().slots[0].is_none());
        assert!(!rig.hs.disconnected[rig.joiner.i] && !rig.hs.disconnected[j], "ordinary stock races are terminal zero-effect refusals, without teardown");
    }
}

#[test]
fn d2c2_result_capacity_backpressure_keeps_stock_and_ack_control_live() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("result-capacity", mode);
        let sp = &mut rig.hs.server.players[rig.joiner.i];
        sp.inventory.set_slot(0, Some(ItemStack::new_block(block::CHEST, 1)));
        for sequence in 1..=crate::action_journal::MAX_ACTIONS as u64 {
            let header = sp.action_frontier.as_ref().unwrap().header(sequence, rig.joiner.applied);
            let receipt = crate::action_wire::ActionReceipt { action: header, fate: crate::action_identity::ActionFate::CancelledNoop, included_events: rig.joiner.applied };
            let row = sp.action_results.as_mut().unwrap().prepare(receipt, Box::new([])).unwrap();
            sp.action_frontier.as_mut().unwrap().terminal(header, receipt.fate).unwrap();
            sp.action_results.as_mut().unwrap().commit(row);
        }
        rig.joiner.sequence = crate::action_journal::MAX_ACTIONS as u64;
        let request = rig.packet(PackOperation::Equip);
        rig.joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &request));
        rig.hs.tick();
        assert!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().pack.is_none(), "no attachment before complete terminal reservation");
        assert_eq!(rig.bag(block::CHEST), 1, "no named payment before room exists");
        assert!(rig.hs.server.players[rig.joiner.i].pack_flight.is_none());
        rig.joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::AppliedActionAck, &AppliedActionAck { session: rig.joiner.accept.action_session, epoch: 0, life: rig.joiner.accept.action_life, through: crate::action_journal::MAX_ACTIONS as u64, included_events: rig.joiner.applied }));
        rig.hs.tick();
        for _ in 0..16 {
            if rig.hs.server.players[rig.joiner.i].pack_flight.is_some() { break; }
            rig.hs.tick();
        }
        assert!(rig.hs.server.players[rig.joiner.i].pack_flight.is_some(), "applied metadata was read behind the waiting pack head and freed result capacity");
        assert!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().pack.is_some());
        assert_eq!(rig.bag(block::CHEST), 0);
    }
}

#[test]
fn d2c2_event_capacity_reservation_rejects_before_any_pack_effect() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("event-capacity", mode);
        let sp = &mut rig.hs.server.players[rig.joiner.i];
        sp.inventory.set_slot(0, Some(ItemStack::new_block(block::CHEST, 1)));
        let events = (0..crate::action_journal::MAX_EVENT_ENTRIES).map(|_| crate::window_events::WindowEvent::PackView).collect::<Vec<_>>().into_boxed_slice();
        crate::window_events::prepare_pack_events(sp, events).unwrap().commit(sp, rig.hs.server.tick_counter);
        let request = rig.packet(PackOperation::Equip);
        rig.joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &request));
        rig.hs.tick();
        assert!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().pack.is_none());
        assert_eq!(rig.bag(block::CHEST), 1);
        assert!(rig.hs.server.players[rig.joiner.i].pack_flight.is_none());
        assert!(rig.hs.server.players[rig.joiner.i].action_results.as_ref().unwrap().peek(request.action).is_none(), "no partial terminal row when canonical event cannot fit");
    }
}

#[test]
fn d2c2_queued_request_crossing_real_death_and_replace_is_zero_effect() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("actor-death", mode);
        rig.hs.server.players[rig.joiner.i].inventory.set_slot(0, Some(ItemStack::new_block(block::CHEST, 1)));
        let request = rig.packet(PackOperation::Equip);
        let sp = &mut rig.hs.server.players[rig.joiner.i];
        sp.combat.dead = true;
        sp.combat.health = 0.0;
        sp.death_handled = false;
        rig.hs.handle_joiner_deaths();
        let replaced = rig.hs.server.players[rig.joiner.i].window_events.last_number();
        assert!(replaced > request.action.events_applied, "the real death hook queued its Replace");
        let terminal = rig.send(&request);
        assert_eq!(terminal.note, PackNote::Dead);
        assert_eq!(terminal.fate, crate::action_identity::ActionFate::Superseded);
        assert!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().pack.is_none(), "the old body cannot equip after its death cut");
        assert_eq!(rig.bag(block::CHEST), 0, "the old Chest belongs to death settlement, never a fresh pack refund");
        assert!(!rig.hs.disconnected[rig.joiner.i]);
    }
}

#[test]
fn d2c2_real_guest_pack_access_uses_any_keeper_and_never_claims_a_wild_steed() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("guest-access", mode);
        let client = rig.hs.attach_test_remote();
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &crate::remote_client::build_join_request_guest("Guest", 0)));
        rig.hs.tick();
        let i = rig.hs.server.players.len() - 1;
        let accept = super::c3d_tests::hear(&client).accepts.pop().expect("an operator-open guest join");
        let applied = rig.hs.server.players[i].window_events.last_number();
        crate::window_events::apply_through(&mut rig.hs.server.players[i], applied, rig.hs.server.tick_counter);
        rig.hs.server.players[i].inventory = crate::inventory::Inventory::new();
        rig.hs.server.players[i].inventory.set_slot(0, Some(ItemStack::new_block(block::CHEST, 1)));
        rig.hs.server.players[i].player.pos = rig.hs.server.players[rig.joiner.i].player.pos;
        rig.hs.server.players[i].yaw = rig.hs.server.players[rig.joiner.i].yaw;
        rig.joiner = Joiner { i, client, accept, sequence: 0, applied };
        let (_, equipped) = rig.execute(PackOperation::Equip);
        assert_eq!(equipped.note, PackNote::Done, "a guest can equip a genuinely kept steed under P2");
        rig.apply(&equipped);
        assert_eq!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().keeper_identity(), Some(crate::horse_ai::SteedKeeper::LegacyLocal(0)), "access never transfers keeping to a guest connection slot");
        let (_, opened) = rig.execute(PackOperation::Open);
        assert_eq!(opened.note, PackNote::Done);
        rig.apply(&opened);
        let session = rig.session();
        let (_, removed) = rig.execute(PackOperation::Remove { incarnation: session.incarnation, revision: session.revision });
        rig.apply(&removed);
        {
            let mut data = rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap();
            data.keeper = None;
            data.kept_by = None;
        }
        let (_, refused) = rig.execute(PackOperation::Equip);
        assert_eq!(refused.note, PackNote::NeedsKeeper);
        assert!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().keeper_identity().is_none());
        assert_eq!(rig.bag(block::CHEST), 1, "wild refusal cannot consume the returned Chest");
        assert!(!rig.hs.disconnected[i]);
    }
}

#[test]
fn d2c2_entity_cargo_at_a_real_block_chest_cell_never_changes_the_block_stock() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("explicit-target", mode);
        let pos = rig.hs.server.ecs.get::<&crate::entity::Position>(rig.entity).unwrap().0;
        let cell = (pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32);
        rig.hs.server.world.set_block(cell.0, cell.1, cell.2, block::CHEST);
        let mut block_stock = crate::chest::ChestData::new();
        block_stock.slots[0] = Some(ItemStack::new_block(block::STONE, 7));
        rig.hs.server.world.insert_chest(cell, block_stock);
        let mut cargo = crate::chest::ChestData::new();
        cargo.slots[0] = Some(ItemStack::new_block(block::STONE, 2));
        rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(cargo);
        let (_, opened) = rig.execute(PackOperation::Open);
        rig.apply(&opened);
        let session = rig.session();
        let (_, taken) = rig.execute(PackOperation::Click { incarnation: session.incarnation, revision: session.revision, click: ContainerClick::TakeAll });
        assert_eq!(taken.note, PackNote::Done);
        assert_eq!(rig.bag(block::STONE), 2);
        assert_eq!(rig.hs.server.world.chest_at(cell).unwrap().slots[0].as_ref().unwrap().count, 7, "an entity's position is never its container address");
        assert!(rig.contents().slots[0].is_none());
    }
}

#[test]
fn d2c2_removed_entity_and_reused_protocol_id_cannot_inherit_the_old_session() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("generation", mode);
        let mut cargo = crate::chest::ChestData::new();
        cargo.slots[0] = Some(ItemStack::new_block(block::STONE, 1));
        rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(cargo);
        let (_, opened) = rig.execute(PackOperation::Open);
        rig.apply(&opened);
        let old = rig.session();
        let request = rig.packet(PackOperation::Click { incarnation: old.incarnation, revision: old.revision, click: ContainerClick::TakeAll });
        let pos = rig.hs.server.ecs.get::<&crate::entity::Position>(rig.entity).unwrap().0;
        rig.hs.server.ecs.despawn(rig.entity).unwrap();
        let replacement = crate::entity::spawn_mob(&mut rig.hs.server.ecs, crate::mob::MobType::Donkey, pos);
        assert_ne!(replacement, old.entity, "hecs generation changes even if its slot is reused");
        let ContainerTarget::Entity(id) = rig.target else { unreachable!() };
        rig.hs.server.ecs.insert_one(replacement, crate::entity::ProtocolId(id)).unwrap();
        {
            let mut data = rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(replacement).unwrap();
            data.keeper = Some(crate::horse_ai::SteedKeeper::LegacyLocal(0));
            let mut pack = crate::chest::ChestData::new();
            pack.slots[0] = Some(ItemStack::new_block(block::STONE, 5));
            data.pack = Some(pack);
        }
        let terminal = rig.send(&request);
        assert_eq!(terminal.note, PackNote::NoPack);
        assert!(terminal.view.is_none());
        assert_eq!(rig.bag(block::STONE), 0);
        assert_eq!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(replacement).unwrap().pack.as_ref().unwrap().slots[0].as_ref().unwrap().count, 5);
        assert!(!rig.hs.disconnected[rig.joiner.i]);
    }
}

#[test]
fn d2c2_earlier_grant_feedback_waits_for_canonical_ack_and_returns_once_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for full in [false, true] {
            let mut rig = Rig::new(&format!("held-unfit-{full}"), mode);
            rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(crate::chest::ChestData::new());
            if full {
                for place in 0..36 {
                    rig.hs.server.players[rig.joiner.i].inventory.set_slot(place, Some(ItemStack::new_block(block::DIRT, 64)));
                }
            }
            rig.hs.grant_to_joiner(rig.joiner.i, [ItemStack::new_block(block::STONE, 1)]);
            let event = rig.hs.server.players[rig.joiner.i].window_events.last_number();
            let (_, opened) = rig.execute(PackOperation::Open);
            let feedback = protocol::ItemActionPacket { seq: 0, action: protocol::ItemAction::GrantUnfit { event, count: 1 }, events_applied: event };
            let bytes = protocol::serialize_packet(protocol::PacketType::ItemAction, &feedback);
            rig.joiner.client.send_to_server(&bytes);
            rig.hs.tick();
            assert!(!rig.hs.disconnected[rig.joiner.i], "overflow feedback is control in either mode");
            assert!(rig.hs.server.players[rig.joiner.i].pack_flight.is_some());
            assert_eq!(rig.hs.server.players[rig.joiner.i].pack_unfit_feedback, vec![(event, 1)]);
            let floor = |rig: &Rig| rig.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().filter(|(_, drop)| drop.stack.item == Item::Block(block::STONE)).map(|(_, drop)| u32::from(drop.stack.count)).sum::<u32>();
            assert_eq!(floor(&rig), 0, "return waits until canonical custody installs");
            rig.apply(&opened);
            assert_eq!(rig.bag(block::STONE), 0, "a pending canonical snapshot cannot resurrect a returned grant");
            assert_eq!(floor(&rig), 1, "the existing ledger returns exactly one backed unit");
            assert!(rig.hs.server.players[rig.joiner.i].pack_unfit_feedback.is_empty());
            let repeat = protocol::ItemActionPacket { events_applied: opened.window_event, ..feedback };
            rig.joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &repeat));
            rig.hs.tick();
            assert!(!rig.hs.disconnected[rig.joiner.i]);
            assert_eq!(floor(&rig), 1, "repeated feedback cannot return the grant twice");
            assert_eq!(rig.bag(block::STONE), 0);
        }
    }
}

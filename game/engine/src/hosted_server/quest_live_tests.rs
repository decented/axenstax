//! Actual signed/guest action dispatch, ordered physical installation and ACKs.
use super::*;
use crate::{
    action_identity::InventoryMode,
    action_wire::{ActionIntent, ActionPacket, AppliedActionAck},
    item::{Item, ItemStack, MaterialId},
    quest_wire::{QuestOperation, QuestRequest, QuestResult},
    transport::ClientTransport as _,
};

struct Rig {
    _root: crate::save::WorldsRootGuard,
    host: HostedServer,
    client: ChannelClientTransport,
    i: usize,
    actor: crate::player_slot::PlayerSlot,
    inbox: crate::personal_sync::Inbox,
    applied: u32,
    seq: u64,
    entity: hecs::Entity,
    target: u32,
}
struct Reply {
    result: QuestResult,
    grants: Vec<protocol::InventoryGrantPacket>,
    personal: protocol::PersonalSnapshot,
}
impl Rig {
    fn new(tag: &str, mode: InventoryMode, signed: bool) -> Self {
        let name = format!("d2c3-live-{tag}-{mode:?}-{signed}");
        let root = crate::save::WorldsRootGuard::new(&name);
        let mut host = tests::start_room_test_server(&name, 0);
        host.select_inventory_mode(mode).unwrap();
        host.server.world.world_type = "normal".into();
        host.server.world.is_workshop = false;
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
        assert!(heard.rejects.is_empty());
        let accept = heard.accepts.first().expect("actual accepted attachment");
        let i = accept.player_index as usize;
        let mut actor = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        let sync = heard.syncs.first().expect("actual Sync1");
        crate::inventory_sync::apply(
            &mut crate::window::WindowMut {
                inv: &mut actor.inventory,
                armour: &mut actor.armour_slots,
                cursor: &mut actor.crafting_ui.cursor_item,
                grid: &mut actor.crafting_ui.grid,
                container: None,
            },
            sync,
            &host.server.registry,
            &mut Default::default(),
            &mut Default::default(),
            mode,
        )
        .unwrap();
        let mut inbox = crate::personal_sync::Inbox::new(accept.action_session, 0);
        let personal = heard.personal.first().expect("actual Personal2");
        inbox.push(personal.clone()).unwrap();
        let ack = inbox
            .apply(personal.window_event, &mut actor)
            .unwrap()
            .remove(0);
        client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::PersonalSync,
            &protocol::PersonalSyncPacket::Applied(ack),
        ));
        host.tick();
        let sp = &host.server.players[i];
        assert!(
            sp.personal_delivery
                .quiescent(sp.personal.as_ref().unwrap(), sp.respawns)
        );
        let at = sp.player.pos + crate::camera::forward_from(sp.yaw, 0.0) * 1.5;
        host.server
            .world
            .village_anchors
            .insert((2, 3), at.to_array().map(|n| n.floor() as i32));
        let entity =
            crate::entity::spawn_mob(&mut host.server.ecs, crate::mob::MobType::Villager, at);
        host.server
            .ecs
            .insert_one(
                entity,
                crate::villager::VillagerComponent {
                    profession: crate::villager::Profession::Blacksmith,
                    claimed_workstation: None,
                    gossip_line: Some("The forge is ready.".into()),
                    gossip_last_refresh_tick: host.server.tick_counter,
                    commission: None,
                },
            )
            .unwrap();
        host.tick();
        let target = host
            .server
            .ecs
            .get::<&crate::entity::ProtocolId>(entity)
            .unwrap()
            .0;
        c3d_tests::hear(&client);
        Self {
            _root: root,
            host,
            client,
            i,
            actor,
            inbox,
            applied: personal.window_event,
            seq: 0,
            entity,
            target,
        }
    }
    fn packet(&mut self, incarnation: u64, operation: QuestOperation) -> ActionPacket {
        self.seq += 1;
        let sp = &self.host.server.players[self.i];
        let raw = crate::staged_window::MarkerWindow::try_capture_parts(
            &self.actor.inventory,
            &self.actor.armour_slots,
            &self.actor.crafting_ui.cursor_item,
            &self.actor.crafting_ui.grid,
            &self.host.server.registry,
        )
        .unwrap();
        ActionPacket {
            action: sp
                .action_frontier
                .as_ref()
                .unwrap()
                .header(self.seq, self.applied),
            intent: ActionIntent::Quest(QuestRequest {
                actor_respawns: sp.respawns,
                target: self.target,
                before: raw.digest_with(sp.station, None),
                incarnation,
                operation,
            }),
        }
    }
    fn send(&mut self, packet: &ActionPacket) -> Reply {
        self.client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::Action,
            packet,
        ));
        let mut result = None;
        let mut grants = Vec::new();
        let mut personal = None;
        let mut receipt = false;
        for _ in 0..24 {
            self.host.tick();
            while let Some(bytes) = self.client.try_recv_from_server() {
                match protocol::deserialize_header(&bytes) {
                    Some((protocol::PacketType::QuestResult, body)) => {
                        let value = QuestResult::decode(body).unwrap();
                        if value.action == packet.action {
                            assert!(!receipt);
                            result = Some(value);
                        }
                    }
                    Some((protocol::PacketType::InventoryGrant, body)) => {
                        assert!(
                            result.is_some(),
                            "cost-only canonical carrier precedes every genuine reward"
                        );
                        assert!(
                            personal.is_none(),
                            "rewards precede the LAST progression fence"
                        );
                        grants.push(protocol::safe_deserialize(body).unwrap());
                    }
                    Some((protocol::PacketType::PersonalSync, body)) => {
                        if let protocol::PersonalSyncPacket::Snapshot(value) =
                            protocol::safe_deserialize(body).unwrap()
                        {
                            assert!(result.is_some(), "progression follows cost");
                            personal = Some(value);
                        }
                    }
                    Some((protocol::PacketType::ActionReceipt, body)) => {
                        let value: crate::action_wire::ActionReceipt =
                            protocol::safe_deserialize(body).unwrap();
                        if value.action == packet.action {
                            assert!(personal.is_some(), "receipt follows the full batch");
                            receipt = true;
                        }
                    }
                    Some((protocol::PacketType::JoinReject, body)) => panic!(
                        "honest quest disconnected: {:?}",
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
        assert!(receipt, "real terminal receipt arrived");
        Reply {
            result: result.expect("real quest result"),
            grants,
            personal: personal.expect("LAST actual snapshot"),
        }
    }
    fn apply(&mut self, reply: &Reply) {
        let p = &mut self.actor;
        crate::pack_client::apply_canonical_window(
            &mut crate::window::WindowMut {
                inv: &mut p.inventory,
                armour: &mut p.armour_slots,
                cursor: &mut p.crafting_ui.cursor_item,
                grid: &mut p.crafting_ui.grid,
                container: None,
            },
            &reply.result.window,
            &self.host.server.registry,
            &Default::default(),
            &mut Default::default(),
            self.host.server.players[self.i].inventory_mode,
        )
        .unwrap();
        for grant in &reply.grants {
            let unfit = crate::remote_entities::apply_inventory_grant(
                &mut p.inventory,
                grant,
                &self.host.server.registry,
            )
            .unwrap();
            if unfit > 0 {
                self.client.send_to_server(&protocol::serialize_packet(
                    protocol::PacketType::ItemAction,
                    &protocol::ItemActionPacket {
                        seq: 1,
                        action: protocol::ItemAction::GrantUnfit {
                            event: grant.window_event,
                            count: unfit,
                        },
                        events_applied: grant.window_event,
                    },
                ));
            }
        }
        assert_eq!(reply.personal.window_event, reply.result.included_events);
        self.inbox.push(reply.personal.clone()).unwrap();
        let ack = self
            .inbox
            .apply(reply.result.included_events, p)
            .unwrap()
            .remove(0);
        self.client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::PersonalSync,
            &protocol::PersonalSyncPacket::Applied(ack),
        ));
        self.client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::AppliedActionAck,
            &AppliedActionAck {
                session: reply.result.action.session,
                epoch: reply.result.action.epoch,
                life: reply.result.action.life,
                through: reply.result.action.seq,
                included_events: reply.result.included_events,
            },
        ));
        self.applied = reply.result.included_events;
        self.host.tick();
        assert!(!self.host.disconnected[self.i]);
        assert!(
            self.host.server.players[self.i].pack_flight.is_none(),
            "exact progression then receipt release the ONE flight"
        );
    }
    fn request(&mut self, incarnation: u64, operation: QuestOperation) -> Reply {
        let packet = self.packet(incarnation, operation);
        let reply = self.send(&packet);
        self.apply(&reply);
        reply
    }
    fn record(&self) -> crate::progression::GameplayRecord {
        self.host.server.players[self.i]
            .personal
            .as_ref()
            .unwrap()
            .record()
            .unwrap()
    }
    fn grant(&mut self, stacks: Vec<ItemStack>) {
        self.host.grant_to_joiner(self.i, stacks);
        while let Some(bytes) = self.client.try_recv_from_server() {
            if let Some((protocol::PacketType::InventoryGrant, body)) =
                protocol::deserialize_header(&bytes)
            {
                let grant: protocol::InventoryGrantPacket =
                    protocol::safe_deserialize(body).unwrap();
                assert_eq!(
                    crate::remote_entities::apply_inventory_grant(
                        &mut self.actor.inventory,
                        &grant,
                        &self.host.server.registry
                    ),
                    Some(0)
                );
                self.applied = self.applied.max(grant.window_event);
            }
        }
    }
}

#[test]
fn d2c3_actual_quest_dispatch_freezes_offer_accepts_once_and_uses_last_progression_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for signed in [true, false] {
            let mut rig = Rig::new("offer", mode, signed);
            let talk = rig.request(0, QuestOperation::Talk);
            assert_eq!(
                talk.result.fate,
                crate::action_identity::ActionFate::Accepted
            );
            let view = talk.result.view.unwrap();
            let terms = view.terms.unwrap();
            let again = rig.request(0, QuestOperation::Talk);
            assert_eq!(
                again.result.view.unwrap().terms.unwrap(),
                terms,
                "render/reopen cannot re-roll an active encounter"
            );
            rig.request(view.incarnation, QuestOperation::Accept { id: terms.id });
            assert_eq!(rig.record().quests.accepted.len(), 1);
            assert_eq!(
                rig.actor.quest_progress.accepted.len(),
                1,
                "actual PersonalSync installed the same accepted quest"
            );
            let repeated = rig.request(view.incarnation, QuestOperation::Accept { id: terms.id });
            assert_eq!(
                repeated.result.fate,
                crate::action_identity::ActionFate::Refused
            );
            assert_eq!(rig.record().quests.accepted.len(), 1);
            assert_eq!(
                rig.host
                    .server
                    .ecs
                    .get::<&crate::quest_npcs::QuestNpcId>(rig.entity)
                    .unwrap()
                    .0,
                view.npc
            );
            assert!(
                rig.host
                    .server
                    .ecs
                    .get::<&crate::entity::Scattered>(rig.entity)
                    .is_err()
            );
        }
    }
}

#[test]
fn d2c3_actual_completion_reward_overflow_uses_original_dead_giver_once_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("giver-overflow", mode, true);
        rig.request(0, QuestOperation::Talk);
        // Current natural pools retain their existing zero item rewards.
        // This explicit server-owned frozen reward fixture exercises the
        // supported exact item carrier without adding production reward amounts.
        rig.host.server.players[rig.i]
            .quest_session
            .as_mut()
            .unwrap()
            .offer
            .as_mut()
            .unwrap()
            .quest
            .rewards = vec![
            crate::saved_item_slot::SavedItemSlot::capture_exact(Some(&ItemStack::new_material(
                MaterialId::Bread,
                2,
            )))
            .unwrap(),
        ];
        let opened = rig.request(0, QuestOperation::Talk).result.view.unwrap();
        let terms = opened.terms.as_ref().unwrap();
        rig.request(opened.incarnation, QuestOperation::Accept { id: terms.id });
        let crate::quest::QuestFlavour::Fetch { item, count } = terms.flavour else {
            panic!("Blacksmith's existing pool is Fetch");
        };
        // Genuine numbered grants, leaving one cost unit in its occupied slot
        // after payment so rewards genuinely cannot fit any of the 36 places.
        let mut kit = vec![ItemStack::new_material(item, count + 1)];
        kit.extend((1..36).map(|_| ItemStack::new_block(crate::block::STONE, 64)));
        rig.grant(kit);
        let packet = rig.packet(opened.incarnation, QuestOperation::TurnIn { id: terms.id });
        let reply = rig.send(&packet);
        assert_eq!(reply.result.note, crate::quest_wire::QuestNote::Completed);
        assert!(rig.record().quests.accepted.is_empty());
        assert_eq!(reply.grants.len(), terms.rewards.as_slice().len());
        let origin = rig
            .host
            .server
            .ecs
            .get::<&crate::entity::Position>(rig.entity)
            .unwrap()
            .0;
        rig.host.server.ecs.despawn(rig.entity).unwrap();
        rig.apply(&reply);
        let reward_count: u32 = terms
            .rewards
            .as_slice()
            .iter()
            .map(|stack| u32::from(stack.count))
            .sum();
        let spilled: u32 = rig
            .host
            .server
            .ecs
            .query::<(&crate::entity::ItemEntity, &crate::entity::Position)>()
            .iter()
            .filter(|(_, (item, position))| {
                matches!(item.stack.item, Item::Material(MaterialId::Bread))
                    && position.0.distance(origin) < 1.5
            })
            .map(|(_, (item, _))| u32::from(item.stack.count))
            .sum();
        assert_eq!(
            spilled, reward_count,
            "only genuine overflow returns beside its original giver after death"
        );
        let reputation = rig.record().reputation.clone();
        rig.client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::ItemAction,
            &protocol::ItemActionPacket {
                seq: 1,
                action: protocol::ItemAction::GrantUnfit {
                    event: reply.grants[0].window_event,
                    count: 255,
                },
                events_applied: rig.applied,
            },
        ));
        rig.host.tick();
        assert_eq!(
            rig.record().reputation,
            reputation,
            "a repeated overflow report cannot award reputation"
        );
        let repeat = rig.packet(opened.incarnation, QuestOperation::TurnIn { id: terms.id });
        let repeated = rig.send(&repeat);
        assert_eq!(
            repeated.result.fate,
            crate::action_identity::ActionFate::Refused
        );
        assert!(
            repeated.grants.is_empty(),
            "consumed identity and dead giver cannot grant again"
        );
    }
}

#[test]
fn d2c3_natural_pool_completion_keeps_reward_amounts_and_zero_money_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for signed in [false, true] {
            let mut rig = Rig::new("natural-completion", mode, signed);
            let opened = rig.request(0, QuestOperation::Talk).result.view.unwrap();
            let terms = opened.terms.as_ref().unwrap();
            assert!(
                terms.rewards.as_slice().is_empty(),
                "existing natural item amounts stay unchanged"
            );
            assert_eq!(
                terms.native(&rig.host.server.registry).unwrap().reward.sats,
                0
            );
            rig.request(opened.incarnation, QuestOperation::Accept { id: terms.id });
            let crate::quest::QuestFlavour::Fetch { item, count } = terms.flavour else {
                panic!("existing Blacksmith Fetch pool");
            };
            rig.grant(vec![ItemStack::new_material(item, count)]);
            let done = rig.request(opened.incarnation, QuestOperation::TurnIn { id: terms.id });
            assert_eq!(done.result.note, crate::quest_wire::QuestNote::Completed);
            assert!(done.grants.is_empty());
            assert!(rig.record().quests.accepted.is_empty());
            assert!(rig.actor.quest_progress.accepted.is_empty());
            assert!(
                !rig.actor
                    .inventory
                    .slots_iter()
                    .flatten()
                    .any(|stack| stack.item == Item::Material(item))
            );
            assert_eq!(
                rig.record()
                    .reputation
                    .iter()
                    .find(|(village, _)| *village == (2, 3))
                    .map(|(_, score)| *score),
                Some(terms.reputation)
            );
            let before = rig.record();
            let repeated = rig.request(opened.incarnation, QuestOperation::TurnIn { id: terms.id });
            assert_eq!(
                repeated.result.fate,
                crate::action_identity::ActionFate::Refused
            );
            assert_eq!(
                rig.record(),
                before,
                "consumed quest identity cannot pay or adjust reputation again"
            );
        }
    }
}

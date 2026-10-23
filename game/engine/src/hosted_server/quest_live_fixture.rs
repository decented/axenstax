//! Actual signed/guest action dispatch, ordered physical installation and ACKs.
use super::*;
use crate::{
    action_identity::InventoryMode,
    action_wire::{ActionIntent, ActionPacket, AppliedActionAck},
    item::ItemStack,
    quest_wire::{QuestOperation, QuestRequest, QuestResult},
    transport::ClientTransport as _,
};

pub(super) struct Rig {
    pub(super) _root: crate::save::WorldsRootGuard,
    pub(super) host: HostedServer,
    pub(super) client: ChannelClientTransport,
    pub(super) i: usize,
    pub(super) actor: crate::player_slot::PlayerSlot,
    pub(super) inbox: crate::personal_sync::Inbox,
    pub(super) applied: u32,
    pub(super) seq: u64,
    pub(super) entity: hecs::Entity,
    pub(super) target: u32,
    pub(super) accepted: protocol::JoinAcceptPacket,
}
pub(super) struct Reply {
    pub(super) result: QuestResult,
    pub(super) grants: Vec<protocol::InventoryGrantPacket>,
    pub(super) personal: protocol::PersonalSnapshot,
}
impl Rig {
    pub(super) fn new(tag: &str, mode: InventoryMode, signed: bool) -> Self {
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
            accepted: accept.clone(),
        }
    }
    pub(super) fn packet(&mut self, incarnation: u64, operation: QuestOperation) -> ActionPacket {
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
    pub(super) fn send(&mut self, packet: &ActionPacket) -> Reply {
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
    pub(super) fn apply(&mut self, reply: &Reply) {
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
    pub(super) fn request(&mut self, incarnation: u64, operation: QuestOperation) -> Reply {
        let packet = self.packet(incarnation, operation);
        let reply = self.send(&packet);
        self.apply(&reply);
        reply
    }
    pub(super) fn record(&self) -> crate::progression::GameplayRecord {
        self.host.server.players[self.i]
            .personal
            .as_ref()
            .unwrap()
            .record()
            .unwrap()
    }
    pub(super) fn grant(&mut self, stacks: Vec<ItemStack>) {
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

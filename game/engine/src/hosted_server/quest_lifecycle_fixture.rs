//! Actual independently authenticated actors in one operator-owned world.
use super::*;
use super::{quest_lifecycle_actor::Actor, quest_live_fixture::Rig};
use crate::{
    action_identity::InventoryMode,
    action_wire::ActionIntent,
    quest_wire::{QuestOperation, QuestRequest, QuestResult},
    transport::ClientTransport as _,
};

pub(super) struct Room {
    pub root: crate::save::WorldsRootGuard,
    pub host: HostedServer,
    pub actors: Vec<Actor>,
    pub entity: hecs::Entity,
    pub target: u32,
    pub mode: InventoryMode,
}
impl Room {
    pub fn new(tag: &str, mode: InventoryMode, signed: bool) -> Self {
        let Rig {
            _root: root,
            host,
            client,
            i: _,
            actor,
            inbox,
            applied,
            seq,
            entity,
            target,
            accepted,
        } = Rig::new(tag, mode, signed);
        assert_eq!(seq, 0);
        let actor = Actor::new(client, accepted, actor, inbox, applied);
        let mut room = Self {
            root,
            host,
            actors: vec![actor],
            entity,
            target,
            mode,
        };
        room.pad();
        room.quiesce();
        room
    }
    pub fn tick(&mut self) {
        for actor in &mut self.actors {
            if !actor.active {
                continue;
            }
            actor.input_seq += 1;
            let at = actor.slot.player.pos;
            actor.client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::ClientInput,
                &protocol::InputPacket {
                    tick: actor.input_seq,
                    x: at.x,
                    y: at.y,
                    z: at.z,
                    yaw: std::f32::consts::PI,
                    health: 20.0,
                    events_applied: actor.applied,
                    ..Default::default()
                },
            ));
        }
        self.host.tick();
        for actor in &mut self.actors {
            actor.drain(&self.host.server.registry);
            if self.host.disconnected[actor.i()] {
                actor.active = false;
            }
        }
    }
    pub fn ticks(&mut self, count: u64) {
        for _ in 0..count {
            self.tick();
        }
    }
    pub fn quiesce(&mut self) {
        for _ in 0..24 {
            self.tick();
            if self
                .actors
                .iter()
                .filter(|actor| actor.active)
                .all(|actor| {
                    let sp = &self.host.server.players[actor.i()];
                    actor.idle()
                        && actor.rejects.is_empty()
                        && !self.host.disconnected[actor.i()]
                        && sp.action_frontier.as_ref().unwrap().header(0, 0).session
                            == actor.accepted.action_session
                        && sp.pack_flight.is_none()
                        && sp.window_events.is_quiescent()
                        && sp
                            .action_results
                            .as_ref()
                            .is_none_or(|results| results.is_quiescent())
                        && sp
                            .personal_delivery
                            .quiescent(sp.personal.as_ref().unwrap(), sp.respawns)
                })
            {
                return;
            }
        }
        panic!("actual actor carriers and returned ACKs did not quiesce");
    }
    pub fn connect(&mut self, secret: Option<[u8; 32]>, binding: [u8; 32]) -> usize {
        let client = if secret.is_some() {
            self.host.attach_test_remote_with_binding(Some(binding))
        } else {
            self.host.attach_test_remote()
        };
        if let Some(secret) = secret {
            tests::send_signed_join(&client, secret, Some(binding));
        } else {
            client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::JoinRequest,
                &crate::remote_client::build_join_request_guest("Guest", 0),
            ));
        }
        self.tick();
        let bytes = client
            .try_recv_from_server()
            .expect("real join accept precedes its carriers");
        let (kind, body) = protocol::deserialize_header(&bytes).unwrap();
        assert_eq!(kind, protocol::PacketType::JoinAccept);
        let accepted: protocol::JoinAcceptPacket = protocol::safe_deserialize(body).unwrap();
        let slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        let personal =
            crate::personal_sync::Inbox::new(accepted.action_session, accepted.action_life);
        let mut actor = Actor::new(client, accepted, slot, personal, 0);
        actor.drain(&self.host.server.registry);
        assert_eq!(actor.applied, 2, "actual initial Sync1 then Personal2");
        let i = actor.i();
        assert!(!self.host.server.players[i].personal_delivery.initialised);
        if let Some(secret) = secret {
            let key = nostr::Keys::new(nostr::SecretKey::from_slice(&secret).unwrap())
                .public_key()
                .to_bytes();
            assert_eq!(self.host.server.players[i].verified_pubkey, Some(key));
        } else {
            assert!(self.host.server.players[i].verified_pubkey.is_none());
        }
        let who = self.actors.len();
        self.actors.push(actor);
        self.pad();
        self.quiesce();
        who
    }
    pub fn pad(&mut self) {
        let at = glam::Vec3::new(40.5, 80.0, 40.5);
        for x in 36..=44 {
            for z in 36..=46 {
                self.host
                    .server
                    .world
                    .set_block(x, 79, z, crate::block::STONE);
                for y in 80..85 {
                    self.host.server.world.set_block(x, y, z, crate::block::AIR);
                }
            }
        }
        self.host.server.column_streamer = None;
        self.host.server.column_refill_per_tick = 0;
        self.host.server.difficulty = crate::survival::Difficulty::Peaceful;
        for actor in self.actors.iter_mut().filter(|actor| actor.active) {
            let sp = &mut self.host.server.players[actor.i()];
            sp.player.pos = at;
            sp.player.velocity = glam::Vec3::ZERO;
            sp.player.on_ground = true;
            sp.player.reset_fall();
            sp.yaw = std::f32::consts::PI;
            sp.pitch = 0.0;
            actor.slot.player.pos = at;
        }
        self.host
            .server
            .ecs
            .get::<&mut crate::entity::Position>(self.entity)
            .unwrap()
            .0 = at + glam::Vec3::Z * 2.0;
        self.host
            .server
            .ecs
            .get::<&mut crate::entity::Velocity>(self.entity)
            .unwrap()
            .0 = glam::Vec3::ZERO;
        self.host
            .server
            .world
            .village_anchors
            .insert((2, 3), [40, 80, 42]);
    }
    pub fn record(&self, who: usize) -> crate::progression::GameplayRecord {
        assert!(
            self.actors[who].active,
            "offline state is read from its actual saved file"
        );
        let sp = &self.host.server.players[self.actors[who].i()];
        sp.gameplay_clock
            .project(
                &sp.personal.as_ref().unwrap().record().unwrap(),
                self.host.server.tick_counter,
            )
            .0
    }
    pub fn quest(
        &mut self,
        who: usize,
        incarnation: u64,
        operation: QuestOperation,
    ) -> QuestResult {
        self.quiesce();
        let actor = &mut self.actors[who];
        let raw = crate::staged_window::MarkerWindow::try_capture_parts(
            &actor.slot.inventory,
            &actor.slot.armour_slots,
            &actor.slot.crafting_ui.cursor_item,
            &actor.slot.crafting_ui.grid,
            &self.host.server.registry,
        )
        .unwrap();
        let action = actor.send(ActionIntent::Quest(QuestRequest {
            actor_respawns: actor.accepted.action_life,
            target: self.target,
            before: raw.digest_with(actor.slot.crafting_ui.station(), None),
            incarnation,
            operation,
        }));
        self.quiesce();
        self.actors[who]
            .results
            .iter()
            .find(|result| result.action == action)
            .unwrap()
            .clone()
    }

    pub fn leave(&mut self, who: usize) -> crate::joiner_save::JoinerSave {
        self.quiesce();
        let i = self.actors[who].i();
        let expected = crate::joiner_save::JoinerSave::capture_projected(
            &self.host.server.players[i],
            self.host.server.run_id,
        )
        .unwrap();
        self.actors[who]
            .client
            .send_to_server(&protocol::serialize_packet(
                protocol::PacketType::Disconnect,
                &(),
            ));
        self.tick();
        assert!(self.host.disconnected[i]);
        self.actors[who].active = false;
        expected
    }
    pub fn directory(&self) -> std::path::PathBuf {
        crate::save::world_dir(&self.host.server.world_name)
    }
    pub fn saved(&self, key: [u8; 32]) -> crate::joiner_save::JoinerSave {
        let path = crate::handover::live::personal_adapter::path(&self.directory(), key);
        crate::joiner_save::JoinerSave::decode(&std::fs::read(path).unwrap()).unwrap()
    }
    pub fn restart(self) -> Self {
        assert!(self.actors.iter().all(|actor| !actor.active));
        let npc = self
            .host
            .server
            .ecs
            .get::<&crate::quest_npcs::QuestNpcId>(self.entity)
            .unwrap()
            .0;
        let scope = self.host.server.world.quest_scope.unwrap();
        let name = self.host.server.world_name.clone();
        let seed = self.host.server.biome_gen.seed;
        let Self {
            root, host, mode, ..
        } = self;
        drop(host);
        let mut host =
            HostedServer::start(0, name, seed, 0, RemoteTransport::WebSocket { port: 0 }).unwrap();
        host.select_inventory_mode(mode).unwrap();
        assert_eq!(host.server.world.quest_scope, Some(scope));
        let entities: Vec<_> = host
            .server
            .ecs
            .query::<&crate::quest_npcs::QuestNpcId>()
            .iter()
            .filter(|(_, id)| id.0 == npc)
            .map(|(entity, _)| entity)
            .collect();
        assert_eq!(
            entities.len(),
            1,
            "exact actual original giver restored once"
        );
        let entity = entities[0];
        host.tick();
        let target = host
            .server
            .ecs
            .get::<&crate::entity::ProtocolId>(entity)
            .unwrap()
            .0;
        Self {
            root,
            host,
            actors: Vec::new(),
            entity,
            target,
            mode,
        }
    }
}

//! Honest quest and empty-hand action dispatch through actual client channels.
use super::quest_lifecycle_fixture::Room;
use super::*;
use crate::{
    action_identity::InventoryMode, action_wire::ActionIntent, quest_wire::QuestOperation,
    transport::ClientTransport as _,
};

impl Room {
    pub fn grant(&mut self, who: usize, stacks: Vec<crate::item::ItemStack>) {
        let i = self.actors[who].i();
        self.host.grant_to_joiner(i, stacks);
        self.actors[who].drain(&self.host.server.registry);
        self.quiesce();
    }
    pub fn kill(&mut self, who: usize, kind: crate::mob::MobType) {
        self.quiesce();
        while self.host.server.tick_counter
            < self.host.server.players[self.actors[who].i()].next_swing_tick
        {
            self.tick();
        }
        let at = self.host.server.players[self.actors[who].i()].player.pos + glam::Vec3::Z * 2.5;
        let entity = crate::entity::spawn_mob(&mut self.host.server.ecs, kind, at);
        self.tick();
        let id = self
            .host
            .server
            .ecs
            .get::<&crate::entity::ProtocolId>(entity)
            .unwrap()
            .0;
        self.host
            .server
            .ecs
            .get::<&mut crate::entity::Position>(entity)
            .unwrap()
            .0 = at;
        self.host
            .server
            .ecs
            .get::<&mut crate::combat::Health>(entity)
            .unwrap()
            .current = 1.0;
        let actor = &mut self.actors[who];
        assert!(
            actor.slot.inventory.slot(8).is_none(),
            "a real empty hand, never an invented weapon"
        );
        actor.legacy_seq += 1;
        let request = protocol::EntityAttackPacket {
            seq: actor.legacy_seq,
            entity: id,
            held_kind: protocol::item_kind::EMPTY,
            held_id: 0,
            held_full: protocol::WireItem::None,
            sprint: false,
            sneak: false,
            hotbar_slot: 8,
            events_applied: actor.applied,
        };
        let outcomes = actor.outcomes.len();
        if self.mode == InventoryMode::Strict {
            actor.send(ActionIntent::Attack(request));
        } else {
            actor.client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::EntityAttack,
                &request,
            ));
        }
        self.tick();
        assert!(
            self.actors[who].outcomes[outcomes..]
                .iter()
                .any(|outcome| outcome.entity == id && outcome.accepted),
            "the actual empty-hand request landed"
        );
        assert!(
            !self.host.server.ecs.contains(entity),
            "actual attributed death was swept"
        );
        self.quiesce();
    }
    pub fn wounded_mob(&mut self, who: usize) -> (hecs::Entity, crate::combat::Attacker) {
        self.quiesce();
        while self.host.server.tick_counter
            < self.host.server.players[self.actors[who].i()].next_swing_tick
        {
            self.tick();
        }
        let at = self.host.server.players[self.actors[who].i()].player.pos + glam::Vec3::Z * 2.5;
        let entity =
            crate::entity::spawn_mob(&mut self.host.server.ecs, crate::mob::MobType::Cow, at);
        self.tick();
        let id = self
            .host
            .server
            .ecs
            .get::<&crate::entity::ProtocolId>(entity)
            .unwrap()
            .0;
        self.host
            .server
            .ecs
            .get::<&mut crate::entity::Position>(entity)
            .unwrap()
            .0 = at;
        let before = self
            .host
            .server
            .ecs
            .get::<&crate::combat::Health>(entity)
            .unwrap()
            .current;
        let actor = &mut self.actors[who];
        assert!(actor.slot.inventory.slot(8).is_none());
        actor.legacy_seq += 1;
        let request = protocol::EntityAttackPacket {
            seq: actor.legacy_seq,
            entity: id,
            held_kind: protocol::item_kind::EMPTY,
            held_id: 0,
            held_full: protocol::WireItem::None,
            sprint: false,
            sneak: false,
            hotbar_slot: 8,
            events_applied: actor.applied,
        };
        if self.mode == InventoryMode::Strict {
            actor.send(ActionIntent::Attack(request));
        } else {
            actor.client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::EntityAttack,
                &request,
            ));
        }
        self.tick();
        let health = self
            .host
            .server
            .ecs
            .get::<&crate::combat::Health>(entity)
            .unwrap()
            .current;
        assert!(
            health > 0.0 && health < before,
            "real attributed hit left a live victim"
        );
        let stamp = self
            .host
            .server
            .ecs
            .get::<&crate::combat::LastAttacker>(entity)
            .unwrap()
            .0;
        assert!(
            matches!(stamp, crate::combat::Attacker::Remote { slot, generation }
            if slot == self.actors[who].i() && generation == self.host.server.players[slot].attach_gen)
        );
        self.quiesce();
        (entity, stamp)
    }
    pub fn decline_elsewhere(&mut self, who: usize) -> crate::progression::GameplayId {
        let at = self.host.server.players[self.actors[who].i()].player.pos
            + glam::Vec3::new(2.0, 0.0, 1.5);
        let entity =
            crate::entity::spawn_mob(&mut self.host.server.ecs, crate::mob::MobType::Villager, at);
        self.host
            .server
            .ecs
            .insert_one(
                entity,
                crate::villager::VillagerComponent {
                    profession: crate::villager::Profession::Blacksmith,
                    claimed_workstation: None,
                    gossip_line: Some("The forge is ready.".into()),
                    gossip_last_refresh_tick: self.host.server.tick_counter,
                    commission: None,
                },
            )
            .unwrap();
        self.tick();
        let target = self
            .host
            .server
            .ecs
            .get::<&crate::entity::ProtocolId>(entity)
            .unwrap()
            .0;
        let old = (self.entity, self.target);
        self.entity = entity;
        self.target = target;
        let opened = self.quest(who, 0, QuestOperation::Talk).view.unwrap();
        self.quest(
            who,
            opened.incarnation,
            QuestOperation::Decline {
                id: opened.terms.as_ref().unwrap().id,
            },
        );
        self.entity = old.0;
        self.target = old.1;
        assert!(
            self.record(who)
                .quests
                .declines
                .iter()
                .any(|(npc, _)| *npc == opened.npc)
        );
        opened.npc
    }
    pub fn natural_kill_offer(&mut self, who: usize) -> crate::quest_wire::QuestView {
        self.host
            .server
            .ecs
            .get::<&mut crate::villager::VillagerComponent>(self.entity)
            .unwrap()
            .profession = crate::villager::Profession::Carpenter;
        let opened = self.quest(who, 0, QuestOperation::Talk).view.unwrap();
        if matches!(
            opened.terms.as_ref().unwrap().flavour,
            crate::quest::QuestFlavour::Kill { .. }
        ) {
            return opened;
        }
        // Close the actual encounter; its enrolled random identity stays the same.
        self.quest(who, opened.incarnation, QuestOperation::Close);
        let seed = u32::from_le_bytes(opened.npc[..4].try_into().unwrap());
        // A new Talk is read on the previous logical tick. Quiescence adds one tick.
        while (seed ^ (self.host.server.tick_counter as u32 + 1)) % 3 != 2 {
            self.tick();
        }
        let opened = self.quest(who, 0, QuestOperation::Talk).view.unwrap();
        assert!(
            matches!(
                opened.terms.as_ref().unwrap().flavour,
                crate::quest::QuestFlavour::Kill { .. }
            ),
            "actual natural Carpenter pool"
        );
        opened
    }
}

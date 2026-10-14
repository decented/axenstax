//! Bounded, server-selected personal snapshots. Exact applied tuples fence
//! initial window installation and handover capture; repeats never reinstall.
use crate::{
    handover::personal::PersonalRecord,
    protocol::{PersonalAck, PersonalSnapshot},
};

pub(crate) fn snapshot(
    personal: &crate::joiner_save::BoundPersonal,
    session: [u8; 16],
    life: u32,
    revision: u64,
    window_event: u32,
) -> Result<PersonalSnapshot, String> {
    personal.record()?;
    if session == [0; 16] || revision == 0 || window_event == 0 {
        return Err("invalid personal snapshot boundary".into());
    }
    let value = PersonalSnapshot {
        session,
        life,
        revision,
        window_event,
        context: personal.binding.context,
        bytes: personal.bytes.clone(),
    };
    let wire = crate::protocol::serialize_packet(
        crate::protocol::PacketType::PersonalSync,
        &crate::protocol::PersonalSyncPacket::Snapshot(value.clone()),
    );
    if wire.len() > crate::protocol::MAX_WIRE_PACKET_LEN {
        return Err("personal snapshot exceeds transport limit".into());
    }
    Ok(value)
}
fn acknowledgement(snapshot: &PersonalSnapshot) -> PersonalAck {
    PersonalAck {
        session: snapshot.session,
        life: snapshot.life,
        revision: snapshot.revision,
        window_event: snapshot.window_event,
        snapshot: crate::handover::hash(
            &bincode::serialize(snapshot).expect("fixed personal snapshot codec"),
        ),
    }
}

#[derive(Default)]
pub(crate) struct Delivery {
    pub pending: Option<PersonalSnapshot>,
    pub revision: u64,
    pub last: Option<PersonalAck>,
    pub last_bytes: [u8; 32],
    pub initialised: bool,
    pub started_tick: u64,
    pub sent_tick: u64,
    pub last_penalty: Option<u64>,
}
impl Delivery {
    pub fn accept(&mut self, ack: PersonalAck) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|s| acknowledgement(s) == ack)
        {
            self.last_bytes = crate::handover::hash(&self.pending.as_ref().unwrap().bytes);
            self.last = Some(ack);
            self.pending = None;
            self.initialised = true;
            true
        } else {
            self.last == Some(ack)
        }
    }
    pub fn quiescent(&self, personal: &crate::joiner_save::BoundPersonal, life: u32) -> bool {
        self.initialised
            && self.pending.is_none()
            && self.last.is_some_and(|a| a.life == life)
            && self.last_bytes == crate::handover::hash(&personal.bytes)
    }
}

pub(crate) struct Inbox {
    session: [u8; 16],
    pub life: u32,
    observed_life: u32,
    pending: std::collections::VecDeque<PersonalSnapshot>,
    last: Option<PersonalAck>,
    context: Option<crate::personal_value::PersonalContext>,
    started: web_time::Instant,
}
impl Inbox {
    pub fn new(session: [u8; 16], life: u32) -> Self {
        Self {
            session,
            life,
            observed_life: life,
            pending: Default::default(),
            last: None,
            context: None,
            started: web_time::Instant::now(),
        }
    }
    pub fn observe_respawn(&mut self, respawns: u32) -> Result<(), String> {
        if respawns <= self.observed_life {
            return Ok(());
        }
        if self.observed_life.checked_add(1) != Some(respawns) {
            return Err("personal life skipped".into());
        }
        self.observed_life = respawns;
        Ok(())
    }
    pub fn timed_out(&self) -> bool {
        !self.initialised() && self.started.elapsed() >= std::time::Duration::from_secs(30)
    }
    pub fn initialised(&self) -> bool {
        self.last.is_some()
    }
    pub fn push(&mut self, value: PersonalSnapshot) -> Result<Option<PersonalAck>, String> {
        if value.session != self.session
            || value.life > self.observed_life
            || value.revision == 0
            || value.window_event == 0
        {
            return Err("foreign or future personal snapshot".into());
        }
        if value.life < self.life {
            return Ok(None);
        }
        PersonalRecord::decode(&value.bytes, &value.context).map_err(|e| e.to_string())?;
        if self.context.is_some_and(|c| c != value.context) {
            return Err("personal snapshot identity changed".into());
        }
        let ack = acknowledgement(&value);
        if let Some(last) = self.last
            && value.revision <= last.revision
        {
            return if ack == last {
                Ok(Some(last))
            } else {
                Ok(None)
            };
        }
        if self.pending.iter().any(|s| acknowledgement(s) == ack) {
            return Ok(None);
        }
        if self.pending.len() >= 8 {
            return Err("personal snapshot queue full".into());
        }
        if self
            .pending
            .back()
            .is_some_and(|s| value.revision <= s.revision)
        {
            return Err("personal snapshot order changed".into());
        }
        self.context = Some(value.context);
        self.pending.push_back(value);
        Ok(None)
    }
    pub fn apply(
        &mut self,
        events: u32,
        slot: &mut crate::player_slot::PlayerSlot,
    ) -> Result<Vec<PersonalAck>, String> {
        let mut applied = Vec::new();
        while let Some(snapshot) = self.pending.front() {
            if snapshot.life < self.life {
                self.pending.pop_front();
                continue;
            }
            if snapshot.life != self.life || snapshot.window_event > events {
                break;
            }
            let value = self.pending.pop_front().unwrap();
            let record =
                PersonalRecord::decode(&value.bytes, &value.context).map_err(|e| e.to_string())?;
            record.install_progression(slot);
            let ack = acknowledgement(&value);
            self.last = Some(ack);
            applied.push(ack);
        }
        Ok(applied)
    }
}

impl crate::GameState {
    pub(crate) fn apply_personal_inbox(&mut self) {
        let Some(client) = self.remote_client.as_mut() else {
            return;
        };
        let events = client.applied_window_event();
        let Some(inbox) = &mut client.personal_inbox else {
            return;
        };
        let Some(slot) = self.players.first_mut() else {
            return;
        };
        match inbox.apply(events, slot) {
            Ok(acks) => {
                for ack in acks {
                    client.send_raw(&crate::protocol::serialize_packet(
                        crate::protocol::PacketType::PersonalSync,
                        &crate::protocol::PersonalSyncPacket::Applied(ack),
                    ));
                }
            }
            Err(error) => client.state = crate::remote_client::ConnectionState::Failed(error),
        }
    }
}

impl crate::server::GameServer {
    pub(crate) fn credit_personal_kill(
        &mut self,
        i: usize,
        kind: crate::mob::MobType,
        pos: glam::Vec3,
    ) {
        let sp = &mut self.players[i];
        let Some(personal) = &mut sp.personal else {
            return;
        };
        let Ok(record) = personal.record() else {
            sp.personal_read_failed = true;
            return;
        };
        let mut kills: ahash::AHashMap<_, _> = record.kills.iter().copied().collect();
        let Some(count) = kills.get(&kind).copied().unwrap_or(0).checked_add(1) else {
            sp.personal_read_failed = true;
            return;
        };
        kills.insert(kind, count);
        let mut reputation = crate::reputation::Reputation::default();
        reputation
            .per_village
            .extend(record.reputation.iter().copied());
        let penalty = matches!(
            kind,
            crate::mob::MobType::Villager | crate::mob::MobType::Peddler
        ) && sp.personal_delivery.last_penalty.is_none_or(|last| {
            self.tick_counter.saturating_sub(last)
                >= crate::reputation::VILLAGER_KILL_REP_COOLDOWN_TICKS
        });
        if penalty && let Some(village) = crate::reputation::village_at_position(&self.world, pos) {
            reputation.adjust(village, crate::reputation::VILLAGER_KILL_PENALTY);
            sp.personal_delivery.last_penalty = Some(self.tick_counter);
        }
        let claims: ahash::AHashMap<_, _> = record.claims.iter().copied().collect();
        match PersonalRecord::from_maps(
            record.context,
            reputation.per_village.iter(),
            kills.iter(),
            claims.iter(),
            record.local,
        )
        .and_then(|r| r.encode())
        {
            Ok(bytes) => personal.bytes = bytes,
            Err(_) => sp.personal_read_failed = true,
        }
    }
    pub(crate) fn decay_personal(&mut self) {
        if self.tick_counter == 0
            || !self
                .tick_counter
                .is_multiple_of(crate::reputation::REPUTATION_DECAY_INTERVAL_TICKS)
        {
            return;
        };
        for sp in &mut self.players {
            let Some(personal) = &mut sp.personal else {
                continue;
            };
            let Ok(mut record) = personal.record() else {
                sp.personal_read_failed = true;
                continue;
            };
            for (_, score) in &mut record.reputation {
                *score -= score.signum();
            }
            match record.encode() {
                Ok(bytes) => personal.bytes = bytes,
                Err(_) => sp.personal_read_failed = true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ServerTransport;
    fn bound() -> crate::joiner_save::BoundPersonal {
        let context = crate::personal_value::PersonalContext {
            participant: [7; 32],
            lineage: [1; 16],
            generation: 1,
            transfer: [4; 16],
        };
        let kills = [(crate::mob::MobType::Pig, 17u32)];
        let record =
            PersonalRecord::from_maps(context, [], kills.iter().map(|(k, v)| (k, v)), [], None)
                .unwrap();
        crate::joiner_save::BoundPersonal::new(
            crate::joiner_save::PersonalBinding {
                context,
                manifest: [1; 32],
                core: [2; 32],
            },
            &record,
        )
        .unwrap()
    }
    #[test]
    fn handover_personal_sync_applies_only_after_real_window_and_repeats_are_inert() {
        let personal = bound();
        let value = snapshot(&personal, [8; 16], 0, 1, 4).unwrap();
        let mut inbox = Inbox::new([8; 16], 0);
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        slot.kill_counter.insert(crate::mob::MobType::Cow, 99);
        inbox.push(value.clone()).unwrap();
        assert!(inbox.apply(3, &mut slot).unwrap().is_empty());
        assert!(!inbox.initialised());
        assert_eq!(
            slot.kill_counter[&crate::mob::MobType::Cow],
            99,
            "decode installed before the window was applied"
        );
        let acks = inbox.apply(4, &mut slot).unwrap();
        assert_eq!(acks, vec![acknowledgement(&value)]);
        assert!(inbox.initialised());
        assert_eq!(slot.kill_counter[&crate::mob::MobType::Pig], 17);
        slot.kill_counter.insert(crate::mob::MobType::Pig, 23);
        assert_eq!(
            inbox.push(value.clone()).unwrap(),
            Some(acknowledgement(&value))
        );
        assert!(inbox.apply(4, &mut slot).unwrap().is_empty());
        assert_eq!(
            slot.kill_counter[&crate::mob::MobType::Pig],
            23,
            "repeat reset newer personal state"
        );
        let mut delivery = Delivery {
            pending: Some(value.clone()),
            revision: 1,
            ..Default::default()
        };
        let mut bad = acknowledgement(&value);
        bad.window_event = 3;
        assert!(!delivery.accept(bad));
        assert!(
            !delivery.quiescent(&personal, 0),
            "an inexact applied tuple released capture"
        );
        assert!(delivery.accept(acknowledgement(&value)));
        assert!(delivery.quiescent(&personal, 0));
    }
    #[test]
    fn handover_personal_sync_batched_lives_wait_for_application_and_old_ack_cannot_release() {
        let personal = bound();
        let first = snapshot(&personal, [8; 16], 0, 1, 1).unwrap();
        let mut inbox = Inbox::new([8; 16], 0);
        inbox.push(first.clone()).unwrap();
        let second = snapshot(&personal, [8; 16], 1, 2, 2).unwrap();
        assert!(
            inbox.push(second.clone()).is_err(),
            "unobserved future life was admitted"
        );
        inbox.observe_respawn(1).unwrap();
        inbox.push(second.clone()).unwrap();
        inbox.observe_respawn(2).unwrap();
        let third = snapshot(&personal, [8; 16], 2, 3, 3).unwrap();
        inbox.push(third.clone()).unwrap();
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        assert!(inbox.apply(0, &mut slot).unwrap().is_empty());
        assert!(!inbox.initialised());
        inbox.life = 2;
        let acks = inbox.apply(3, &mut slot).unwrap();
        assert_eq!(acks, vec![acknowledgement(&third)]);
        let mut delivery = Delivery {
            pending: Some(third.clone()),
            revision: 3,
            ..Default::default()
        };
        assert!(!delivery.accept(acknowledgement(&first)));
        assert!(delivery.pending.is_some());
        assert!(!delivery.accept(acknowledgement(&second)));
        assert!(delivery.pending.is_some());
        assert!(delivery.accept(acknowledgement(&third)));
    }
    #[test]
    fn handover_personal_sync_actual_poll_holds_authorising_packets_until_applied() {
        let (server, transport) = crate::transport::channel_pair();
        let mut client = crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Personal", 0),
            None,
        );
        server.try_recv_from_client().unwrap();
        let accept = crate::protocol::JoinAcceptPacket {
            player_index: 1,
            seed: 42,
            spawn_x: 0.5,
            spawn_y: 70.0,
            spawn_z: 0.5,
            world_time: 0,
            is_creative: false,
            play_mode: crate::play_mode::PlayMode::Survival,
            difficulty: "normal".into(),
            server_identity: None,
            exhibits: vec![],
            world_rules: crate::protocol::WorldRules::from_meta(&crate::save::WorldMeta::new(
                "personal",
            )),
            worldgen_version: crate::world::worldgen_fingerprint(),
            chunk_note_radius: 0,
            inventory_mode: crate::action_identity::InventoryMode::Mirror,
            action_session: [8; 16],
            action_life: 0,
            has_personal_state: true,
        };
        server.send_to_client(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::JoinAccept,
            &accept,
        ));
        client.poll();
        assert!(!client.personal_ready());
        let packet = crate::protocol::ItemActionPacket {
            seq: 1,
            action: crate::protocol::ItemAction::OpenGrave { x: 1, y: 2, z: 3 },
            events_applied: 0,
        };
        client.send_item_action(&packet);
        assert!(
            server.try_recv_from_client().is_none(),
            "authorising packet escaped before personal installation"
        );
        let value = snapshot(&bound(), [8; 16], 0, 1, 1).unwrap();
        server.send_to_client(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::PersonalSync,
            &crate::protocol::PersonalSyncPacket::Snapshot(value.clone()),
        ));
        client.poll();
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        assert!(
            client
                .personal_inbox
                .as_mut()
                .unwrap()
                .apply(0, &mut slot)
                .unwrap()
                .is_empty()
        );
        assert!(!client.personal_ready());
        let acks = client
            .personal_inbox
            .as_mut()
            .unwrap()
            .apply(1, &mut slot)
            .unwrap();
        for ack in acks {
            client.send_raw(&crate::protocol::serialize_packet(
                crate::protocol::PacketType::PersonalSync,
                &crate::protocol::PersonalSyncPacket::Applied(ack),
            ));
        }
        client.send_item_action(&packet);
        let ack = server.try_recv_from_client().unwrap();
        assert_eq!(
            crate::protocol::deserialize_header(&ack).unwrap().0,
            crate::protocol::PacketType::PersonalSync,
            "first authorising packet overtook Applied"
        );
        let action = server.try_recv_from_client().unwrap();
        assert_eq!(
            crate::protocol::deserialize_header(&action).unwrap().0,
            crate::protocol::PacketType::ItemAction
        );
    }
}

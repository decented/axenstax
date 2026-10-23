//! Bounded, server-selected personal snapshots. Exact applied tuples fence
//! initial window installation and handover capture; repeats never reinstall.
use crate::protocol::{PersonalAck, PersonalSnapshot};

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
        context: personal.binding.context(),
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
#[cfg(test)]
pub(crate) use crate::personal_inbox::Inbox;
pub(crate) use crate::personal_inbox::acknowledgement;

#[derive(Default)]
pub(crate) struct Delivery {
    pub pending: Option<PersonalSnapshot>,
    /// Exact accepted client frontier, awaiting older server authorisations.
    pub acknowledged_window: Option<u32>,
    initial_expected: Option<PersonalAck>,
    pub revision: u64,
    pub last: Option<PersonalAck>,
    pub last_bytes: [u8; 32],
    pub initialised: bool,
    pub started_tick: u64,
    pub sent_tick: u64,
}
impl Delivery {
    pub fn issue(&mut self, snapshot: PersonalSnapshot, tick: u64) {
        if !self.initialised && self.initial_expected.is_none() {
            self.initial_expected = Some(acknowledgement(&snapshot));
        }
        self.revision = snapshot.revision;
        self.started_tick = tick;
        self.sent_tick = tick;
        self.pending = Some(snapshot);
    }
    pub fn accept(&mut self, ack: PersonalAck) -> bool {
        if self.pending.as_ref().is_some_and(|s| acknowledgement(s) == ack) {
            self.last_bytes = crate::handover::hash(&self.pending.as_ref().unwrap().bytes);
            self.last = Some(ack);
            self.pending = None;
            self.initialised = true;
            self.initial_expected = None;
            true
        } else if self.initial_expected == Some(ack) {
            // A newer snapshot may have been issued but delayed in transit.
            // The exact retained baseline proves initial installation only;
            // it does not settle or release that newer snapshot/freeze fence.
            self.initialised = true;
            self.initial_expected = None;
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
    /// Full current authority, including the actual authorised window fold.
    /// A high event or an older initial ACK alone cannot prove this tuple.
    pub(crate) fn current(
        &self,
        personal: &crate::joiner_save::BoundPersonal,
        header: crate::action_identity::ActionHeader,
        window: &crate::window_events::WindowEvents,
    ) -> bool {
        let Some(last) = self.last else {
            return false;
        };
        if !self.quiescent(personal, header.life)
            || last.session != header.session
            || last.life != header.life
            || last.revision != self.revision
            || last.window_event == 0
            || window.applied_number() < last.window_event
            || !window.is_quiescent()
        {
            return false;
        }
        crate::personal_inbox::acknowledgement_parts(
            header.session,
            header.life,
            self.revision,
            last.window_event,
            personal.binding.context(),
            &personal.bytes,
        ) == last
    }
}

impl crate::server::GameServer {
    /// Observe gameplay without per-tick canonical rewrites. An actual expiry
    /// is one discrete revision; normal snapshot delivery fences that change.
    pub(crate) fn advance_personal_timers(&mut self) {
        for sp in &mut self.players {
            if !sp.server_simulated || !sp.is_in_world() {
                continue;
            }
            let old_clock = sp.gameplay_clock;
            sp.gameplay_clock.observe(self.tick_counter);
            let Some(personal) = &mut sp.personal else {
                continue;
            };
            let Ok(record) = personal.record() else {
                sp.gameplay_clock = old_clock;
                sp.personal_read_failed = true;
                sp.progression_tracked = false;
                continue;
            };
            let (mut projected, expired) = sp.gameplay_clock.project(&record, self.tick_counter);
            if !expired {
                continue;
            }
            let Some(revision) = projected.revision.checked_add(1) else {
                sp.gameplay_clock = old_clock;
                sp.personal_read_failed = true;
                sp.progression_tracked = false;
                continue;
            };
            projected.revision = revision;
            match projected.encode() {
                Ok(bytes) => {
                    personal.bytes = bytes;
                    sp.gameplay_clock.committed(self.tick_counter);
                }
                Err(_) => {
                    sp.gameplay_clock = old_clock;
                    sp.personal_read_failed = true;
                    sp.progression_tracked = false;
                }
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn credit_personal_kill(&mut self, i: usize, kind: crate::mob::MobType, pos: glam::Vec3) {
        let village = crate::reputation::village_at_position(&self.world, pos);
        self.credit_personal_kill_in_village(i, kind, village);
    }
    pub(crate) fn credit_personal_kill_in_village(
        &mut self,
        i: usize,
        kind: crate::mob::MobType,
        village: Option<crate::reputation::VillageId>,
    ) {
        let sp = &mut self.players[i];
        let Some(personal) = &mut sp.personal else {
            sp.progression_tracked = false;
            return;
        };
        let Ok(record) = personal.record() else {
            sp.personal_read_failed = true;
            sp.progression_tracked = false;
            return;
        };
        let (mut record, _) = sp.gameplay_clock.project(&record, self.tick_counter);
        let mut kills: ahash::AHashMap<_, _> = record.kills.iter().copied().collect();
        let Some(count) = kills.get(&kind).copied().unwrap_or(0).checked_add(1) else {
            sp.personal_read_failed = true;
            sp.progression_tracked = false;
            return;
        };
        kills.insert(kind, count);
        let mut reputation = crate::reputation::Reputation::default();
        reputation.per_village.extend(record.reputation.iter().copied());
        let penalty = matches!(kind, crate::mob::MobType::Villager | crate::mob::MobType::Peddler)
            && record.quests.penalty_left == 0
            && village.is_some();
        if penalty && let Some(village) = village {
            reputation.adjust(village, crate::reputation::VILLAGER_KILL_PENALTY);
        }
        record.kills = kills.into_iter().collect();
        record.kills.sort_by_key(|(kind, _)| {
            u32::from_le_bytes(bincode::serialize(kind).expect("scalar mob kind").try_into().expect("mob kind width"))
        });
        record.reputation = reputation.per_village.into_iter().collect();
        record.reputation.sort_by_key(|(key, _)| *key);
        if penalty {
            record.quests.penalty_left = crate::reputation::VILLAGER_KILL_REP_COOLDOWN_TICKS as u32;
        }
        let Some(revision) = record.revision.checked_add(1) else {
            sp.personal_read_failed = true;
            sp.progression_tracked = false;
            return;
        };
        record.revision = revision;
        match record.encode() {
            Ok(bytes) => {
                personal.bytes = bytes;
                sp.gameplay_clock.committed(self.tick_counter);
            }
            Err(_) => {
                sp.personal_read_failed = true;
                sp.progression_tracked = false;
            }
        }
    }
    pub(crate) fn decay_personal(&mut self, days: u32) {
        if days == 0 {
            return;
        }
        for sp in &mut self.players {
            if !sp.server_simulated || !sp.is_in_world() {
                continue;
            }
            let Some(personal) = &mut sp.personal else {
                sp.progression_tracked = false;
                continue;
            };
            let Ok(record) = personal.record() else {
                sp.personal_read_failed = true;
                sp.progression_tracked = false;
                continue;
            };
            let (mut record, _) = sp.gameplay_clock.project(&record, self.tick_counter);
            if record.reputation.iter().all(|(_, score)| *score == 0) {
                continue;
            }
            let Some(revision) = record.revision.checked_add(1) else {
                sp.personal_read_failed = true;
                sp.progression_tracked = false;
                continue;
            };
            record.revision = revision;
            for (_, score) in &mut record.reputation {
                *score = crate::reputation_day::decay_score(*score, days);
            }
            match record.encode() {
                Ok(bytes) => {
                    personal.bytes = bytes;
                    sp.gameplay_clock.committed(self.tick_counter);
                }
                Err(_) => {
                    sp.personal_read_failed = true;
                    sp.progression_tracked = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handover::personal::PersonalRecord;
    use crate::transport::ServerTransport;
    fn bound() -> crate::joiner_save::BoundPersonal {
        let context = crate::personal_value::PersonalContext {
            participant: [7; 32],
            lineage: [1; 16],
            generation: 1,
            transfer: [4; 16],
        };
        let kills = [(crate::mob::MobType::Pig, 17u32)];
        let record = PersonalRecord::from_maps(context, [], kills.iter().map(|(k, v)| (k, v)), [], None).unwrap();
        crate::joiner_save::BoundPersonal::new(
            crate::joiner_save::PersonalBinding { context, manifest: [1; 32], core: [2; 32] },
            &record,
        )
        .unwrap()
    }
    #[test]
    fn d2c2_held_personal_snapshot_counts_actual_body_capacity_and_refuses_whole() {
        let mut value = snapshot(&bound(), [8; 16], 0, 1, 4).unwrap();
        value.bytes.reserve_exact(crate::action_journal::MAX_RETAINED_BYTES);
        let mut inbox = Inbox::new([8; 16], 0);
        assert!(inbox.push(value).is_err(), "a valid small encoded record cannot hide oversized retained allocation");
        assert_eq!(inbox.pending_count(), 0, "the failed body is never published partially");
        assert!(!inbox.initialised());
        let value = snapshot(&bound(), [8; 16], 0, 1, 4).unwrap();
        let minimum = value.bytes.capacity() + std::mem::size_of::<PersonalSnapshot>();
        inbox.push(value).unwrap();
        assert_eq!(inbox.pending_count(), 1);
        assert!(inbox.owned_bytes() >= minimum, "actual deque allocation plus nested body ownership is charged");
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
        assert_eq!(slot.kill_counter[&crate::mob::MobType::Cow], 99, "decode installed before the window was applied");
        let acks = inbox.apply(4, &mut slot).unwrap();
        assert_eq!(acks, vec![acknowledgement(&value)]);
        assert!(inbox.initialised());
        assert_eq!(slot.kill_counter[&crate::mob::MobType::Pig], 17);
        slot.kill_counter.insert(crate::mob::MobType::Pig, 23);
        assert_eq!(inbox.push(value.clone()).unwrap(), Some(acknowledgement(&value)));
        assert!(inbox.apply(4, &mut slot).unwrap().is_empty());
        assert_eq!(slot.kill_counter[&crate::mob::MobType::Pig], 23, "repeat reset newer personal state");
        let mut delivery = Delivery { pending: Some(value.clone()), revision: 1, ..Default::default() };
        let mut bad = acknowledgement(&value);
        bad.window_event = 3;
        assert!(!delivery.accept(bad));
        assert!(!delivery.quiescent(&personal, 0), "an inexact applied tuple released capture");
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
        assert!(inbox.push(second.clone()).is_err(), "unobserved future life was admitted");
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
        let mut delivery = Delivery { pending: Some(third.clone()), revision: 3, ..Default::default() };
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
            world_rules: crate::protocol::WorldRules::from_meta(&crate::save::WorldMeta::new("personal")),
            worldgen_version: crate::world::worldgen_fingerprint(),
            chunk_note_radius: 0,
            inventory_mode: crate::action_identity::InventoryMode::Mirror,
            action_session: [8; 16],
            action_life: 0,
            has_personal_state: true,
        };
        server.send_to_client(&crate::protocol::serialize_packet(crate::protocol::PacketType::JoinAccept, &accept));
        client.poll();
        assert!(!client.personal_ready());
        let packet = crate::protocol::ItemActionPacket {
            seq: 1,
            action: crate::protocol::ItemAction::OpenGrave { x: 1, y: 2, z: 3 },
            events_applied: 0,
        };
        client.send_item_action(&packet);
        assert!(server.try_recv_from_client().is_none(), "authorising packet escaped before personal installation");
        let value = snapshot(&bound(), [8; 16], 0, 1, 2).unwrap();
        server.send_to_client(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::PersonalSync,
            &crate::protocol::PersonalSyncPacket::Snapshot(value.clone()),
        ));
        client.poll();
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        assert!(client.personal_inbox.as_mut().unwrap().apply(0, &mut slot).unwrap().is_empty());
        assert!(!client.personal_ready());
        // This pure poll fixture uses the real physical replacement fold;
        // readiness is recorded only after that successful Sync1 application.
        let registry = crate::block::BlockRegistry::new();
        let mut window = crate::window_events::EffectiveWindow {
            inv: crate::inventory::Inventory::new(),
            armour: [None; 4],
            cursor: None,
            grid: Default::default(),
        };
        window.inv.set_slot(4, Some(crate::item::ItemStack::new_block(crate::block::GLASS, 9)));
        let sync = crate::inventory_sync::packet(&window, &Default::default(), 1);
        server.send_to_client(&crate::protocol::serialize_packet(crate::protocol::PacketType::InventorySync, &sync));
        client.poll();
        let received = client
            .pending_outcomes
            .iter()
            .find_map(|outcome| match outcome {
                crate::remote_client::RequestOutcome::Sync(sync) => Some(sync.clone()),
                _ => None,
            })
            .expect("actual poll lost Sync1");
        crate::inventory_sync::apply(
            &mut crate::window::WindowMut {
                inv: &mut slot.inventory,
                armour: &mut slot.armour_slots,
                cursor: &mut slot.crafting_ui.cursor_item,
                grid: &mut slot.crafting_ui.grid,
                container: None,
            },
            &received,
            &registry,
            &mut Default::default(),
            &mut Default::default(),
            crate::action_identity::InventoryMode::Mirror,
        )
        .unwrap();
        client.note_initial_personal_window_applied(received.window_event, received.locks);
        client.note_window_event(received.window_event);
        assert_eq!(slot.inventory.slot(4).unwrap().count, 9);
        assert!(!client.initial_personal_window_waits());
        assert!(client.personal_inbox.as_mut().unwrap().apply(1, &mut slot).unwrap().is_empty());
        assert!(!client.personal_ready(), "Sync1 alone released personal authorisation");
        client.send_item_action(&packet);
        assert!(server.try_recv_from_client().is_none());
        assert!(
            client.pending_outcomes.iter().any(|outcome| matches!(
                outcome,
                crate::remote_client::RequestOutcome::PersonalSync { window_event: 2, .. }
            )),
            "actual snapshot poll lost its own fence"
        );
        client.note_window_event(2);
        let acks = client.personal_inbox.as_mut().unwrap().apply(2, &mut slot).unwrap();
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
        assert_eq!(crate::protocol::deserialize_header(&action).unwrap().0, crate::protocol::PacketType::ItemAction);
    }
}

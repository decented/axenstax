//! Exact personal delivery fences for authenticated imported participants.
use super::*;
impl HostedServer {
    pub(super) fn start_personal(&mut self, i: usize, _event: u32, replace: bool) -> Result<(), String> {
        let sp = &mut self.server.players[i];
        let Some(personal) = &sp.personal else {
            return Ok(());
        };
        if !replace && sp.personal_delivery.pending.is_some() {
            return Ok(());
        }
        if !replace && sp.personal_delivery.quiescent(personal, sp.respawns) {
            return Ok(());
        }
        let revision = sp.personal_delivery.revision.checked_add(1).ok_or("personal revision exhausted")?;
        let session =
            sp.action_frontier.as_ref().ok_or("personal snapshot has no action namespace")?.header(0, 0).session;
        let event = sp.window_events.last_number().checked_add(1).ok_or("personal event sequence exhausted")?;
        let snapshot = crate::personal_sync::snapshot(personal, session, sp.respawns, revision, event)?;
        let prepared = crate::window_events::prepare_pack_events(
            sp,
            Box::new([crate::window_events::WindowEvent::PersonalSync(Box::new(snapshot.clone()))]),
        )
        .map_err(|_| "personal fence cannot fit")?;
        debug_assert_eq!(prepared.last_number(), event);
        let packet = protocol::serialize_packet(
            protocol::PacketType::PersonalSync,
            &protocol::PersonalSyncPacket::Snapshot(snapshot.clone()),
        );
        prepared.commit(sp, self.server.tick_counter);
        sp.personal_delivery.issue(snapshot, self.server.tick_counter);
        self.send_to_joined_slot(i, &packet);
        Ok(())
    }
    pub(super) fn personal_applied(&mut self, i: usize, payload: &[u8]) {
        let Ok(protocol::PersonalSyncPacket::Applied(ack)) = protocol::safe_deserialize(payload) else {
            let _ = self.release_slot(i, Some("Invalid personal acknowledgement"));
            return;
        };
        let Some(sp) = self.server.players.get_mut(i) else {
            return;
        };
        if sp.personal.is_none() {
            let _ = self.release_slot(i, Some("Unexpected personal acknowledgement"));
            return;
        }
        // Superseded/dead/old attaches cannot release the current tuple.
        // Exact repeats of the last applied tuple are harmless.
        // A quest's exact older progression tuple can be superseded by a
        // later ordinary update. It proves that batch without releasing the
        // latest/current handover fence. A new applied lifecycle tuple may
        // also retire an old body's terminal metadata, never its old state.
        if let Some(flight) = &mut sp.pack_flight
            && flight.personal_ack == Some(ack)
            && ack.life == sp.respawns
        {
            flight.requires_personal = false;
        }
        if sp.personal_delivery.accept(ack) {
            if let Some(flight) = &mut sp.pack_flight
                && ack.life == sp.respawns
                && ack.window_event >= flight.event
            {
                flight.requires_personal = false;
            }
            // Installation proof is immediate; its server window fold must
            // not overtake older authorisations that still name the old base.
            sp.personal_delivery.acknowledged_window = Some(
                sp.personal_delivery.acknowledged_window.map_or(ack.window_event, |old| old.max(ack.window_event)),
            );
            self.flush_personal_window(i);
        } else {
            let current = sp.action_frontier.as_ref().map(|f| f.header(0, 0).session);
            if current != Some(ack.session) || ack.life > sp.respawns || ack.revision > sp.personal_delivery.revision {
                let _ = self.release_slot(i, Some("Foreign or future personal acknowledgement"));
            }
        }
    }
    fn flush_personal_window(&mut self, i: usize) {
        let Some(sp) = self.server.players.get_mut(i) else { return };
        if sp.action_queue.as_ref().is_some_and(|queue| !queue.is_empty())
            || self.edit_queues.get(i).is_some_and(|queue| !queue.is_empty())
        {
            return;
        }
        if let Some(event) = sp.personal_delivery.acknowledged_window.take() {
            self.apply_window_events(i, event);
        }
    }

    pub(super) fn flush_personal_updates(&mut self) {
        for i in 0..self.server.players.len() {
            // This also runs after paced queues drain, without needing a
            // later movement packet to release the accepted window fence.
            self.flush_personal_window(i);
            let sp = &self.server.players[i];
            if !sp.server_simulated || !sp.is_in_world() || sp.personal.is_none() {
                continue;
            }
            if sp.personal_read_failed {
                let _ =
                    self.release_slot(i, Some("Personal progress couldn't be retained; saved bytes are preserved."));
                continue;
            }
            if let Some(pending) = &sp.personal_delivery.pending {
                let elapsed = self.server.tick_counter.saturating_sub(sp.personal_delivery.started_tick);
                if elapsed >= 600 {
                    let _ = self.release_slot(
                        i,
                        Some("Personal state wasn't applied in time; the saved player record is preserved."),
                    );
                    continue;
                }
                if self.server.tick_counter.saturating_sub(sp.personal_delivery.sent_tick) >= 20 {
                    let packet = protocol::serialize_packet(
                        protocol::PacketType::PersonalSync,
                        &protocol::PersonalSyncPacket::Snapshot(pending.clone()),
                    );
                    self.server.players[i].personal_delivery.sent_tick = self.server.tick_counter;
                    self.send_to_joined_slot(i, &packet);
                }
                continue;
            }
            let event = sp.window_events.last_number();
            if let Err(error) = self.start_personal(i, event, false) {
                log::warn!("Personal delivery refused: {error}");
                let _ =
                    self.release_slot(i, Some("Personal progress couldn't be delivered; saved bytes are preserved."));
            }
        }
    }
}

#[cfg(test)]
#[path = "personal_tests.rs"]
mod tests;

/// All fallible event, snapshot, tuple and output work precedes the cut.
/// The owner keeps an exclusive HostedServer borrow until commit/abandon.
pub(crate) struct PreparedCarryPersonal {
    events: crate::window_events::PreparedEvents,
    snapshot: protocol::PersonalSnapshot,
    wire: Vec<u8>,
    ack: protocol::PersonalAck,
    previous_event: u32,
}
impl PreparedCarryPersonal {
    pub(crate) fn ack(&self) -> protocol::PersonalAck {
        self.ack
    }
    pub(crate) fn ready(&self, sp: &crate::server::ServerPlayer) -> bool {
        sp.pack_flight.is_none() && sp.window_events.last_number() == self.previous_event
            && sp.window_events.is_quiescent()
            && self.events.last_number() == self.ack.window_event
            && sp.personal_delivery.initialised
            && sp.personal_delivery.pending.is_none()
    }
    pub(crate) fn abandon(self, sp: &mut crate::server::ServerPlayer) {
        self.events.abandon(sp);
    }
    pub(crate) fn commit(self, sp: &mut crate::server::ServerPlayer, tick: u64, bytes: Vec<u8>) -> Vec<u8> {
        // ready was checked under the SAME exclusive borrow before either
        // participant changed. Initial delivery was genuinely established.
        self.events.commit(sp, tick);
        sp.personal.as_mut().expect("prepared current holder").bytes = bytes;
        sp.gameplay_clock.committed(tick);
        sp.personal_delivery.issue(self.snapshot, tick);
        self.wire
    }
}
impl HostedServer {
    pub(crate) fn prepare_carry_personal(
        &mut self,
        slot: usize,
        bytes: &[u8],
        retained: usize,
    ) -> Result<PreparedCarryPersonal, String> {
        let sp = &mut self.server.players[slot];
        let personal = sp.personal.as_ref().ok_or("missing current Personal holder")?;
        let header = sp.action_frontier.as_ref().ok_or("missing current action namespace")?.header(0, 0);
        if !sp.personal_delivery.current(personal, header, &sp.window_events) {
            return Err("current Personal delivery has not settled".into());
        }
        let revision = sp.personal_delivery.revision.checked_add(1).ok_or("Personal delivery revision exhausted")?;
        let event = sp.window_events.last_number().checked_add(1).ok_or("Personal event exhausted")?;
        let candidate = crate::joiner_save::BoundPersonal { binding: personal.binding.clone(), bytes: bytes.to_vec() };
        let snapshot = crate::personal_sync::snapshot(&candidate, header.session, header.life, revision, event)?;
        let ack = crate::personal_sync::acknowledgement(&snapshot);
        let wire = protocol::serialize_packet(
            protocol::PacketType::PersonalSync,
            &protocol::PersonalSyncPacket::Snapshot(snapshot.clone()),
        );
        let staging = retained
            .saturating_add(personal.bytes.capacity())
            .saturating_add(candidate.bytes.capacity())
            .saturating_add(snapshot.bytes.capacity().saturating_mul(2))
            .saturating_add(wire.capacity())
            .saturating_add(3 * std::mem::size_of::<protocol::PersonalSnapshot>());
        if staging > crate::action_journal::MAX_RETAINED_BYTES {
            return Err("Personal cut staging exceeds its limit".into());
        }
        let previous_event = sp.window_events.last_number();
        let events = crate::window_events::prepare_pack_events(
            sp,
            Box::new([crate::window_events::WindowEvent::PersonalSync(Box::new(snapshot.clone()))]),
        )
        .map_err(|_| "Personal cut event cannot fit")?;
        Ok(PreparedCarryPersonal { events, snapshot, wire, ack, previous_event })
    }
    pub(crate) fn resend_carry_personal(
        &self,
        ticket: &crate::handover::active_carry::CutTicket,
    ) -> Result<(), String> {
        let seat = ticket.seat();
        let sp = &self.server.players[seat.slot];
        let pending = sp.personal_delivery.pending.as_ref().ok_or("current snapshot no longer pending")?;
        if Some(crate::personal_sync::acknowledgement(pending)) != ticket.expected() {
            return Err("current retry tuple changed".into());
        }
        let packet = protocol::serialize_packet(
            protocol::PacketType::PersonalSync,
            &protocol::PersonalSyncPacket::Snapshot(pending.clone()),
        );
        if packet.len() > protocol::MAX_WIRE_PACKET_LEN {
            return Err("current retry exceeds wire limit".into());
        }
        self.send_to_joined_slot(seat.slot, &packet);
        Ok(())
    }
    pub(crate) fn send_carry_personal(&self, slot: usize, wire: &[u8]) {
        self.send_to_joined_slot(slot, wire);
    }
}

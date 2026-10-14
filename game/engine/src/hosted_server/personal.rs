//! Exact personal delivery fences for authenticated imported participants.
use super::*;
impl HostedServer {
    pub(super) fn start_personal(
        &mut self,
        i: usize,
        _event: u32,
        replace: bool,
    ) -> Result<(), String> {
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
        let revision = sp
            .personal_delivery
            .revision
            .checked_add(1)
            .ok_or("personal revision exhausted")?;
        let session = sp
            .action_frontier
            .as_ref()
            .ok_or("personal snapshot has no action namespace")?
            .header(0, 0)
            .session;
        let event = sp.window_events.last_number().checked_add(1).ok_or("personal event sequence exhausted")?;
        let snapshot = crate::personal_sync::snapshot(personal, session, sp.respawns, revision, event)?;
        let prepared = crate::window_events::prepare_pack_events(sp, Box::new([crate::window_events::WindowEvent::PersonalSync(Box::new(snapshot.clone()))])).map_err(|_| "personal fence cannot fit")?;
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
        let Ok(protocol::PersonalSyncPacket::Applied(ack)) = protocol::safe_deserialize(payload)
        else {
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
        if sp.personal_delivery.accept(ack) {
            // Installation proof is immediate; its server window fold must
            // not overtake older authorisations that still name the old base.
            sp.personal_delivery.acknowledged_window = Some(sp.personal_delivery.acknowledged_window.map_or(ack.window_event, |old| old.max(ack.window_event)));
            self.flush_personal_window(i);
        } else {
            let current = sp.action_frontier.as_ref().map(|f| f.header(0, 0).session);
            if current != Some(ack.session)
                || ack.life > sp.respawns
                || ack.revision > sp.personal_delivery.revision
            {
                let _ = self.release_slot(i, Some("Foreign or future personal acknowledgement"));
            }
        }
    }
    fn flush_personal_window(&mut self, i: usize) {
        let Some(sp) = self.server.players.get_mut(i) else { return };
        if sp.action_queue.as_ref().is_some_and(|queue| !queue.is_empty())
            || self.edit_queues.get(i).is_some_and(|queue| !queue.is_empty())
        { return; }
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
                let _ = self.release_slot(
                    i,
                    Some("Personal progress couldn't be retained; saved bytes are preserved."),
                );
                continue;
            }
            if let Some(pending) = &sp.personal_delivery.pending {
                let elapsed = self
                    .server
                    .tick_counter
                    .saturating_sub(sp.personal_delivery.started_tick);
                if elapsed >= 600 {
                    let _ = self.release_slot(i, Some("Personal state wasn't applied in time; the saved player record is preserved."));
                    continue;
                }
                if self
                    .server
                    .tick_counter
                    .saturating_sub(sp.personal_delivery.sent_tick)
                    >= 20
                {
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
                let _ = self.release_slot(
                    i,
                    Some("Personal progress couldn't be delivered; saved bytes are preserved."),
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "personal_tests.rs"]
mod tests;

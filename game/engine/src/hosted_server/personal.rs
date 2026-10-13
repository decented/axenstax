//! Exact personal delivery fences for authenticated imported participants.
use super::*;
impl HostedServer {
    pub(super) fn start_personal(
        &mut self,
        i: usize,
        event: u32,
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
        let snapshot =
            crate::personal_sync::snapshot(personal, session, sp.respawns, revision, event)?;
        let packet = protocol::serialize_packet(
            protocol::PacketType::PersonalSync,
            &protocol::PersonalSyncPacket::Snapshot(snapshot.clone()),
        );
        sp.personal_delivery.started_tick = self.server.tick_counter;
        sp.personal_delivery.sent_tick = self.server.tick_counter;
        sp.personal_delivery.revision = revision;
        sp.personal_delivery.pending = Some(snapshot);
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
        if !sp.personal_delivery.accept(ack) {
            let current = sp.action_frontier.as_ref().map(|f| f.header(0, 0).session);
            if current != Some(ack.session)
                || ack.life > sp.respawns
                || ack.revision > sp.personal_delivery.revision
            {
                let _ = self.release_slot(i, Some("Foreign or future personal acknowledgement"));
            }
        }
    }
    pub(super) fn flush_personal_updates(&mut self) {
        for i in 0..self.server.players.len() {
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

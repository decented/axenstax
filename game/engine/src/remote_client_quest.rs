//! Quest carriers use the ONE shared pack flight and its held-prefix budget.
use super::{RemoteClient, RequestOutcome};
use crate::{
    action_identity::ActionHeader,
    action_wire::ActionReceipt,
    quest_wire::{QuestRequest, QuestResult, QuestViewPush},
};
use sha2::{Digest, Sha256};

impl RemoteClient {
    pub(crate) fn reserve_quest_request(
        &mut self,
        request: QuestRequest,
    ) -> Result<ActionHeader, String> {
        if !self.is_connected()
            || !self.personal_ready()
            || self.pack_request_pending()
            || self.respawn_unanswered()
        {
            return Err("Quest action is waiting".into());
        }
        if request.actor_respawns != self.applied_actor_respawns() {
            return Err("Quest action names an old body".into());
        }
        let actor_respawns = request.actor_respawns;
        let action = self.reserve_action(
            crate::action_wire::ActionIntent::Quest(request),
            Box::new([]),
            Box::new([]),
        )?;
        self.pack_flight = Some(super::pack::PackFlight {
            action,
            actor_respawns,
            received: None,
            world_ready: false,
            requires_personal: true,
        });
        Ok(action)
    }
    fn retired_quest_result(&self, packet: &QuestResult) -> bool {
        self.action_journal.as_ref().is_some_and(|journal| {
            journal.retired_receipt_is_duplicate(ActionReceipt {
                action: packet.action,
                fate: packet.fate,
                included_events: packet.included_events,
            })
        })
    }
    pub(super) fn receive_quest_result(&mut self, payload: &[u8]) -> Result<bool, String> {
        let packet = QuestResult::decode(payload)?;
        if self
            .pack_flight
            .as_ref()
            .is_none_or(|flight| flight.action != packet.action || !flight.requires_personal)
        {
            return if self.retired_quest_result(&packet) {
                Ok(false)
            } else {
                Err("Quest result has no matching request".into())
            };
        }
        let hash: [u8; 32] = Sha256::digest(payload).into();
        let flight = self.pack_flight.as_mut().expect("matching flight checked");
        if packet.actor_respawns != flight.actor_respawns {
            return Err("Quest result changed its actor life".into());
        }
        let identity = (packet.included_events, packet.fate, hash);
        if let Some(received) = flight.received {
            return if received == identity {
                Ok(false)
            } else {
                Err("A repeated quest result changed its outcome".into())
            };
        }
        self.pack_buffer.reserve(packet.owned_bytes())?;
        self.chunk_queue
            .try_reserve_exact(1)
            .map_err(|_| "Quest FIFO allocation failed")?;
        flight.received = Some(identity);
        self.chunk_queue.push((
            self.pending_block_changes.len(),
            crate::chunk_intake::StreamItem::QuestResult(Box::new(packet)),
        ));
        Ok(true)
    }
    pub(crate) fn apply_quest_result_intake(
        &mut self,
        packet: &QuestResult,
    ) -> Result<bool, String> {
        if self
            .pack_flight
            .as_ref()
            .is_none_or(|flight| flight.action != packet.action || !flight.requires_personal)
        {
            return if self.retired_quest_result(packet) {
                Ok(false)
            } else {
                Err("Quest intake has no matching request".into())
            };
        }
        let hash: [u8; 32] = Sha256::digest(packet.encode()?.as_ref()).into();
        let flight = self.pack_flight.as_mut().expect("matching flight checked");
        if flight.received != Some((packet.included_events, packet.fate, hash)) {
            return Err("Quest intake differs from its decoded carrier".into());
        }
        if flight.world_ready {
            return Ok(false);
        }
        flight.world_ready = true;
        Ok(true)
    }
    pub(super) fn receive_quest_view(&mut self, packet: QuestViewPush) -> Result<(), String> {
        packet.validate()?;
        let namespace = self
            .action_journal
            .as_ref()
            .and_then(|journal| journal.next_header(self.events_applied).ok());
        if namespace.is_none_or(|header| {
            header.session != packet.session
                || header.epoch != packet.epoch
                || header.life != packet.life
        }) {
            return Err("Quest view names another namespace".into());
        }
        self.pack_buffer.reserve(packet.owned_bytes())?;
        self.pending_outcomes
            .try_reserve_exact(1)
            .map_err(|_| "Quest view allocation failed")?;
        self.pending_outcomes
            .push(RequestOutcome::QuestView(Box::new(packet)));
        self.check_pack_poll_buffer()
    }
}

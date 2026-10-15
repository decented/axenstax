//! Personal-only progress on the one held link; ordinary bytes retain replay ownership.
use super::{RecipientConsent, RecipientSession, RecipientStage, Refusal};
use crate::{
    personal_inbox::AppliedPersonalBatch,
    player_slot::PlayerSlot,
    protocol::{self, PacketType},
    remote_client::{ParkedPersonalLink, PersonalControlProgress, RemoteClient},
    window_events::WindowInbox,
};
use std::mem::size_of;

const CONTROL_PER_PUMP: usize = 8;
const CONTROL_FAILED: Refusal =
    Refusal("The joined personal boundary changed, so the handover was cancelled.");

impl RecipientSession {
    pub(crate) fn consent_personal(
        &mut self,
        consent: RecipientConsent,
        listen: std::net::UdpSocket,
        link: ParkedPersonalLink,
        now: u64,
    ) -> Result<(), Refusal> {
        if link.binding().transfer() != self.transfer()
            || link.binding().participant() != Some(self.persona)
            || link.is_closed()
        {
            return Err(Refusal(
                "This joined personal state does not belong to this handover.",
            ));
        }
        let (transport, binding) = link.into_parts();
        self.consent(consent, listen, transport, now)?;
        self.personal_binding = Some(binding);
        Ok(())
    }
    pub(super) fn park_gameplay_packet(&mut self, packet: Vec<u8>) {
        if self.parked_overflow {
            return;
        }
        if self.parked.try_reserve_exact(1).is_err() {
            self.parked_overflow = true;
            self.parked.clear();
            self.parked_bytes = 0;
            return;
        }
        let bytes = self.parked_bytes.saturating_add(packet.capacity());
        if bytes.saturating_add(self.parked.capacity().saturating_mul(size_of::<Vec<u8>>()))
            > super::PARKED_BYTES
        {
            self.parked_overflow = true;
            self.parked.clear();
            self.parked_bytes = 0;
        } else {
            self.parked_bytes = bytes;
            self.parked.push_back(packet);
        }
    }
    fn parked_usage(&self) -> (usize, usize) {
        (
            self.parked.len(),
            self.parked_bytes
                .saturating_add(self.parked.capacity().saturating_mul(size_of::<Vec<u8>>())),
        )
    }
    fn dormant_personal_stage(&self) -> bool {
        matches!(
            self.stage,
            RecipientStage::AwaitingConsent
                | RecipientStage::Prepared
                | RecipientStage::Activated
                | RecipientStage::Aborted(_)
                | RecipientStage::Quarantined(_)
        )
    }
    fn validate_held_personal(&self, remote: &RemoteClient) -> Result<(), Refusal> {
        let binding = self.personal_binding.as_ref().ok_or(CONTROL_FAILED)?;
        if binding.transfer() != self.transfer()
            || binding.participant() != Some(self.persona)
            || self.link.as_ref().is_none_or(|link| link.is_closed())
            || self.parked_overflow
        {
            return Err(CONTROL_FAILED);
        }
        remote
            .validate_personal_control(binding)
            .map_err(|_| CONTROL_FAILED)
    }
    fn send_personal_applied(
        &mut self,
        remote: &RemoteClient,
        batch: AppliedPersonalBatch,
    ) -> Result<(), Refusal> {
        self.validate_held_personal(remote)?;
        if !remote.personal_batch_matches(&batch) {
            return Err(CONTROL_FAILED);
        }
        // No caller-supplied packet or hash can reach this held-link sink.
        // This queues bytes; it cannot claim the host accepted an ACK.
        let link = self.link.as_ref().ok_or(CONTROL_FAILED)?;
        for wire in batch.wires() {
            link.send_to_server(wire);
        }
        Ok(())
    }
    /// Called AFTER the ordinary session pump, which remains the sole reader.
    /// `external` excludes this parked queue, RemoteClient and WindowInbox;
    /// their actual capacities are added here/by the corresponding preparation.
    pub(crate) fn pump_personal_control(
        &mut self,
        remote: &mut RemoteClient,
        window: &mut WindowInbox,
        slot: &mut PlayerSlot,
        external: (usize, usize),
        _now: u64,
    ) -> Result<PersonalControlProgress, Refusal> {
        // Prepared's closed link is a durable wait, never a new Personal error.
        if self.dormant_personal_stage() || self.personal_binding.is_none() {
            return Ok(PersonalControlProgress::Idle);
        }
        if let Err(error) = self.validate_held_personal(remote) {
            return Err(self.fail(error));
        }
        let mut consumed = 0;
        for _ in 0..CONTROL_PER_PUMP {
            let Some(packet) = self.parked.front() else {
                break;
            };
            let Some((PacketType::PersonalSync, payload)) = protocol::deserialize_header(packet)
            else {
                // Pre-consent quiescence makes late ordinary work a refusal.
                // Keep every byte in arrival order for the existing abort replay.
                return Err(self.fail(Refusal(
                    "Gameplay arrived while the handover was waiting for personal state.",
                )));
            };
            if packet.len() > protocol::MAX_WIRE_PACKET_LEN {
                return Err(self.fail(CONTROL_FAILED));
            }
            let parked = self.parked_usage();
            let usage = (
                external.0.saturating_add(parked.0),
                external.1.saturating_add(parked.1),
            );
            let result = remote.receive_personal_parked(
                payload,
                window,
                usage,
                self.stage == RecipientStage::Accepted,
            );
            let duplicate = match result {
                Ok(value) => value,
                Err(_) => return Err(self.fail(CONTROL_FAILED)),
            };
            // Ownership moves exactly once: an admitted body/marker survives
            // abort in those SAME inboxes and can no longer be raw-replayed.
            let removed = self.parked.pop_front().expect("examined parked front");
            self.parked_bytes = self.parked_bytes.saturating_sub(removed.capacity());
            drop(removed);
            consumed += 1;
            if let Some(batch) = duplicate
                && let Err(error) = self.send_personal_applied(remote, batch)
            {
                return Err(self.fail(error));
            }
        }
        let Some(inbox) = remote.personal_inbox.as_ref() else {
            return Err(self.fail(CONTROL_FAILED));
        };
        if inbox.pending_count() == 0 {
            return Ok(if consumed == 0 {
                PersonalControlProgress::Idle
            } else {
                PersonalControlProgress::Waiting
            });
        }
        if self.stage != RecipientStage::Accepted {
            return Err(self.fail(CONTROL_FAILED));
        }
        let prefix = match window.prepare_personal_prefix(
            remote.applied_window_event(),
            remote.applied_actor_respawns(),
            inbox,
            CONTROL_PER_PUMP,
        ) {
            Ok(prefix) => prefix,
            Err(_) => return Err(self.fail(CONTROL_FAILED)),
        };
        let through = prefix.through();
        let parked = self.parked_usage();
        let usage = (
            external.0.saturating_add(parked.0),
            external.1.saturating_add(parked.1),
        );
        let prepared = match remote.prepare_personal_apply(slot, prefix, usage) {
            Ok(prepared) => prepared,
            Err(_) => return Err(self.fail(CONTROL_FAILED)),
        };
        let batch = prepared.commit();
        let count = batch.count();
        if let Err(error) = self.send_personal_applied(remote, batch) {
            return Err(self.fail(error));
        }
        Ok(PersonalControlProgress::Applied { through, count })
    }
}

#[cfg(test)]
#[path = "recipient_personal/tests.rs"]
mod tests;

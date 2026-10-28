//! Drain on the normal owner; move the actual park owner only after proof preparation.
use super::*;
use std::collections::VecDeque;

impl crate::GameState {
    /// The one ordinary receive/application pass runs even before Paused returns.
    /// Its caller skips the later pass in this same frame.
    pub(crate) fn handover_prepare_normal_receive(&mut self) -> bool {
        if !matches!(&self.handover, Some(Handover::Recipient(s)) if s.stage() == RecipientStage::Draining)
        {
            return false;
        }
        let pending = self.chunk_intake.has_carried_world_work();
        if let Some(remote) = self.remote_client.as_mut() {
            remote.handover_world_pending(pending);
        }
        true
    }

    pub(super) fn begin_normal_handover(&mut self, now: u64) {
        let Some(Handover::Recipient(session)) = self.handover.as_ref() else {
            return;
        };
        let proposal = session.proposal().clone();
        if self.handover_signed_in() != Some(session.park_identity().0) {
            self.decline_over_joined(proposal.transfer, DeclineReason::NotReady);
            self.handover = None;
            self.handover_toast("Your sign-in changed. Start the handover again.");
            return;
        }
        if let Some(refusal) = self.players.first().and_then(|slot| {
            crate::handover::live::eligibility::joined_progress(slot, self.tick_counter).err()
        }) {
            self.handover = None;
            self.decline_over_joined(proposal.transfer, DeclineReason::NotReady);
            self.handover_toast(refusal.0);
            return;
        }
        if !self.pending_block_changes.is_empty()
            || !self.window_inbox.is_empty()
            || self
                .players
                .first()
                .is_some_and(|slot| slot.crafting_ui.ops.first_stamp().is_some())
            || self.remote_client.as_ref().is_some_and(|client| {
                !client.handover_authority_settled() || client.pack_buffers_live()
            })
        {
            self.handover_toast("Your inventory is still settling. Try again in a moment.");
            return;
        }
        let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else {
            self.decline_over_joined(proposal.transfer, DeclineReason::NotReady);
            self.handover = None;
            self.handover_toast(
                "This machine couldn't open a port for the handover, so it was declined.",
            );
            return;
        };
        let pending = self.chunk_intake.has_carried_world_work();
        let result = (|| {
            let Some(Handover::Recipient(session)) = self.handover.as_mut() else {
                return Ok(());
            };
            let remote = self
                .remote_client
                .as_mut()
                .ok_or(Refusal("The joined connection ended."))?;
            if session
                .drain_fixed_bytes()
                .saturating_add(remote.handover_drain_fixed_bytes())
                > crate::handover::live::park::WIRE_CAP
            {
                return Err(Refusal("The handover control exceeded its limit."));
            }
            let consent = RecipientConsent {
                transfer: proposal.transfer,
                world_name: proposal.world_name,
                source_npub: proposal.source_npub,
            };
            let wire = session.consent_normal(consent, socket, now)?;
            let (participant, runtime) = session.park_identity();
            remote
                .begin_handover_drain(
                    proposal.transfer,
                    session.park_authority().clone(),
                    participant,
                    runtime,
                    pending,
                )
                .map_err(Refusal)?;
            remote.send_raw(&wire);
            Ok::<(), Refusal>(())
        })();
        if let Err(Refusal(why)) = result {
            if let Some(Handover::Recipient(session)) = self.handover.as_mut() {
                session.abort(why);
            }
            // Consent was not sent if normal ownership couldn't enter Draining.
            self.decline_over_joined(proposal.transfer, DeclineReason::NotReady);
            self.end_recipient(why);
        }
    }

    pub(super) fn advance_normal_handover(&mut self, now: u64) -> Result<(), Refusal> {
        let persona = self.handover_signed_in();
        let ready = self.joined()
            && self.players.len() == 1
            && self.pending_block_changes.is_empty()
            && self.window_inbox.is_empty()
            && !self.chunk_intake.has_carried_world_work()
            && self.players.first().is_some_and(|slot| {
                !slot.is_dead()
                    && slot.crafting_ui.ops.first_stamp().is_none()
                    && crate::handover::live::eligibility::joined_progress(slot, self.tick_counter)
                        .is_ok()
            });
        let Some(Handover::Recipient(session)) = self.handover.as_mut() else {
            return Ok(());
        };
        if session.stage() != RecipientStage::Draining {
            return Ok(());
        }
        if persona != Some(session.park_identity().0) {
            return Err(Refusal("Your sign-in changed during the handover."));
        }
        session.check_normal_wait(now)?;
        let transfer = session.proposal().transfer;
        let remote = self
            .remote_client
            .as_mut()
            .ok_or(Refusal("The joined connection ended."))?;
        if !remote.handover_drain_live(transfer) {
            return Err(Refusal("The connection to the host closed."));
        }
        if let Some(why) = remote.handover_drain_failure(transfer) {
            return Err(Refusal(why));
        }
        let Some(fence) = remote.handover_drain_fence(transfer) else {
            return Ok(());
        };
        if !ready || !remote.handover_drain_application_ready(transfer) {
            return Ok(());
        }
        let mut link = Some(
            remote
                .park_for_personal_control(transfer)
                .map_err(|_| Refusal("The joined personal boundary changed before park."))?,
        );
        match session.prepare_parked(&fence, &mut link, remote, now) {
            Ok(prepared) => {
                // No fallible work remains between moving the genuine owner and
                // installing its lease. Only then may any proof bytes leave.
                let wire = prepared.commit();
                if let Some(lease) = self.upnp_lease.take() {
                    session.hold_lease(Box::new(lease));
                }
                remote.cancel_handover_drain(transfer);
                session.send_parked_wire(&wire);
                Ok(())
            }
            Err(error) => {
                // These are the exact returned transport and honest empty replay:
                // reading stopped at the fence, with no post-fence read or clone.
                let (transport, _) = link
                    .take()
                    .expect("retained actual park owner")
                    .into_parts();
                remote.restore_transport(transport, VecDeque::new());
                // Keep the matching drain marker until end_recipient sends Abort
                // through the restored normal owner and clears that marker.
                Err(error)
            }
        }
    }
}

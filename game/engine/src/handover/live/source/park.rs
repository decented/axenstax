//! Source freeze precedes real recipient park; current-holder commit follows it.
use super::*;
use crate::handover::live::park::{Attachment, EncodedControl, Fence, Proof, WAIT_SECS, WIRE_CAP};
use std::{mem::size_of, time::Instant};

pub(super) struct SourcePark {
    fence: Fence,
    started: Instant,
    deadline: u64,
}

impl SourceSession {
    #[cfg(test)]
    pub(crate) fn test_current_cut_committed(&self) -> bool {
        self.current_cut.is_some()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Existing verified Accept fields enter one phase transition"
    )]
    pub(super) fn accept_before_park(
        &mut self,
        hs: &mut HostedServer,
        live: Live,
        runtime: [u8; 32],
        attestation: String,
        acceptance: String,
        challenge: Id,
        now: u64,
    ) -> Result<(), Refusal> {
        let attestation = self.verify_accept(runtime, &attestation, &acceptance, challenge, now)?;
        self.check(hs, live)?;
        // A legacy peer with no genuine installed boundary cannot claim park.
        Attachment::source(hs, self.action.seat, self.transfer)?;
        let deadline = now
            .checked_add(WAIT_SECS)
            .ok_or(Refusal("The handover deadline is unavailable."))?;
        let started = Instant::now();
        hs.handover_freeze(self.cfg.world_dir.as_deref())
            .map_err(Refusal)?;
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce)
            .map_err(|_| Refusal("The handover challenge couldn't be created."))?;
        let fence = Fence::issue(
            self.transfer,
            nonce,
            &self.cfg.authority,
            self.action.seat,
            runtime,
        )?;
        let encoded = EncodedControl::new(
            &Frame::FreezeFence { fence },
            size_of::<Option<SourcePark>>(),
        )?;
        let wire = encoded.tagged(size_of::<Option<SourcePark>>())?;
        self.recipient_runtime = Some(runtime);
        self.recipient_challenge = challenge;
        self.acceptance = Some(acceptance);
        self.recipient_attestation = Some(attestation);
        self.consented_at = Some(now);
        self.park = Some(SourcePark {
            fence,
            started,
            deadline,
        });
        self.stage = SourceStage::AwaitingParked;
        self.check_park_wait(hs, live, now)?;
        hs.handover_send_park(&wire).map_err(Refusal)
    }

    pub(super) fn check_park_wait(
        &self,
        hs: &mut HostedServer,
        live: Live,
        now: u64,
    ) -> Result<(), Refusal> {
        if self.stage != SourceStage::AwaitingParked {
            return Ok(());
        }
        let pending = self
            .park
            .as_ref()
            .ok_or(Refusal("The handover fence is unavailable."))?;
        if now >= pending.deadline || pending.started.elapsed().as_secs() >= WAIT_SECS {
            return Err(Refusal(
                "The other game didn't settle in time. Play has resumed.",
            ));
        }
        // The frozen reader's closed-last-frame rule is for final PreparedAck,
        // not an earlier park claim, even if its signature is valid.
        hs.handover_check().map_err(Refusal)?;
        self.check(hs, live)
    }

    pub(super) fn accept_parked(
        &mut self,
        hs: &mut HostedServer,
        live: Live,
        proof: Proof,
        now: u64,
    ) -> Result<(), Refusal> {
        self.check_park_wait(hs, live, now)?;
        let pending = self
            .park
            .as_ref()
            .ok_or(Refusal("The handover fence is unavailable."))?;
        if pending.fence.generation != self.action.seat.generation {
            return Err(Refusal("The other player reconnected before handover."));
        }
        proof.verify(
            &pending.fence,
            &self.cfg.authority,
            Attachment::source(hs, self.action.seat, self.transfer)?,
        )?;
        // No current preparation here. The caller repeats real raw/typed checks
        // immediately before its actual two-holder prepare/commit operation.
        self.stage = SourceStage::Frozen;
        Ok(())
    }
}

const _: () = assert!(size_of::<Option<SourcePark>>() <= WIRE_CAP);

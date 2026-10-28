//! Consent keeps normal ownership; the actual returned park owner remains recoverable.
use super::*;
use crate::{
    handover::live::park::{EncodedControl, Fence, Proof, WAIT_SECS, WIRE_CAP},
    remote_client::{ParkedPersonalLink, RemoteClient},
};
use std::{mem::size_of, time::Instant};

pub(super) struct Wait {
    started: Instant,
    deadline: u64,
}

/// No transport is consumed while signing or encoding can still refuse.
pub(crate) struct PreparedPark<'s, 'l> {
    session: &'s mut RecipientSession,
    link: &'l mut Option<ParkedPersonalLink>,
    wire: Vec<u8>,
    now: u64,
}

impl PreparedPark<'_, '_> {
    pub(crate) fn commit(self) -> Vec<u8> {
        // The exclusive borrow prevents substituting another returned link
        // between the checked preparation and this infallible move.
        let (transport, binding) = self
            .link
            .take()
            .expect("checked actual park owner")
            .into_parts();
        self.session.link = Some(transport);
        self.session.personal_binding = Some(binding);
        self.session.stage = RecipientStage::Accepted;
        self.session.last_heard = self.now;
        self.session.draining = None;
        self.wire
    }
}

impl RecipientSession {
    pub(crate) fn drain_fixed_bytes(&self) -> usize {
        size_of::<Option<Wait>>()
    }

    pub(crate) fn consent_normal(
        &mut self,
        consent: RecipientConsent,
        listen: std::net::UdpSocket,
        now: u64,
    ) -> Result<Vec<u8>, Refusal> {
        let deadline = now
            .checked_add(WAIT_SECS)
            .ok_or(Refusal("The handover deadline is unavailable."))?;
        let accept = self.prepare_acceptance(consent, listen, now)?;
        let wire = packet(&accept).ok_or(Refusal("Your consent couldn't be sent."))?;
        self.draining = Some(Wait {
            started: Instant::now(),
            deadline,
        });
        self.stage = RecipientStage::Draining;
        Ok(wire)
    }

    pub(crate) fn park_authority(&self) -> &Authority {
        &self.proposal.authority
    }

    pub(crate) fn park_identity(&self) -> ([u8; 32], [u8; 32]) {
        (self.persona, self.identity.runtime.public_key().to_bytes())
    }

    pub(crate) fn check_normal_wait(&self, now: u64) -> Result<(), Refusal> {
        let wait = self
            .draining
            .as_ref()
            .ok_or(Refusal("The joined handover wait changed."))?;
        if self.stage != RecipientStage::Draining || self.link.is_some() || self.socket.is_none() {
            return Err(Refusal("The joined handover wait changed."));
        }
        if now >= wait.deadline || wait.started.elapsed().as_secs() >= WAIT_SECS {
            return Err(Refusal(
                "The host didn't pause in time, so the handover was cancelled.",
            ));
        }
        Ok(())
    }

    pub(crate) fn prepare_parked<'s, 'l>(
        &'s mut self,
        fence: &Fence,
        link: &'l mut Option<ParkedPersonalLink>,
        remote: &RemoteClient,
        now: u64,
    ) -> Result<PreparedPark<'s, 'l>, Refusal> {
        self.check_normal_wait(now)?;
        let retained =
            size_of::<Option<Wait>>().saturating_add(remote.handover_drain_fixed_bytes());
        if retained > WIRE_CAP {
            return Err(Refusal("The handover control exceeded its limit."));
        }
        let proof = Proof::from_parked(
            fence,
            &self.proposal.authority,
            link.as_ref()
                .ok_or(Refusal("The actual park owner is unavailable."))?,
            remote,
            &self.identity.runtime,
        )?;
        let preparing = retained.saturating_add(size_of::<PreparedPark<'_, '_>>());
        let encoded = EncodedControl::new(&Frame::Parked { proof }, preparing)?;
        let wire = encoded.tagged(preparing)?;
        // Signing or allocation may have taken time. Expiry still precedes
        // installing ownership and the first possible proof send.
        self.check_normal_wait(now)?;
        Ok(PreparedPark {
            session: self,
            link,
            wire,
            now,
        })
    }

    pub(crate) fn send_parked_wire(&self, wire: &[u8]) {
        // Infallible owner installation and the actual lease transfer precede
        // this call. From here delivery is uncertain: normal held abort applies.
        self.link
            .as_ref()
            .expect("installed actual park owner")
            .send_to_server(wire);
    }
}

const _: () = assert!(size_of::<Option<Wait>>() <= WIRE_CAP);

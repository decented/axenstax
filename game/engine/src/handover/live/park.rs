//! Fixed ephemeral park claims. Personal application and retirement stay separate.
use super::{eligibility::Refusal, wire::Frame};
use crate::{
    handover::{Hash, Id, phase::Authority},
    hosted_server::HostedServer,
    progression::{GameplayContext, WorldPlayer},
    protocol::PROTOCOL_VERSION,
    remote_client::{ParkedPersonalLink, RemoteClient},
};
use nostr::Keys;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, mem::size_of};

pub(crate) const WIRE_CAP: usize = 512;
pub(crate) const CONTROL_CAP: usize = 16 * 1024;
pub(crate) const WAIT_SECS: u64 = 60;
const BAD: Refusal = Refusal("The handover no longer matches this joined connection.");

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fence {
    pub(crate) transfer: Id,
    pub(crate) nonce: Id,
    authority: Hash,
    participant: Hash,
    runtime: Hash,
    pub(crate) generation: u32,
}

impl fmt::Debug for Fence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Identity is shown elsewhere as an npub, never as these wire bytes.
        f.debug_struct("Fence")
            .field("transfer", &self.transfer)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proof {
    pub(crate) transfer: Id,
    nonce: Id,
    fence: Hash,
    signature: [[u8; 32]; 2],
}

/// Source-comparable fixed metadata, never a replacement for the local Arc origin.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Attachment {
    transfer: Id,
    player_index: u32,
    session: Id,
    context: GameplayContext,
    life: u32,
    observed_life: u32,
}

impl Attachment {
    pub(crate) fn checked(
        transfer: Id,
        player_index: u32,
        session: Id,
        context: GameplayContext,
        life: u32,
        observed_life: u32,
    ) -> Result<Self, Refusal> {
        if transfer == [0; 16] || session == [0; 16] || life != observed_life {
            return Err(BAD);
        }
        context.validate().map_err(|_| BAD)?;
        if context.participant().is_none() {
            return Err(BAD);
        }
        Ok(Self {
            transfer,
            player_index,
            session,
            context,
            life,
            observed_life,
        })
    }

    pub(crate) fn source(
        hs: &mut HostedServer,
        seat: super::eligibility::Seat,
        transfer: Id,
    ) -> Result<Self, Refusal> {
        hs.handover_check().map_err(Refusal)?;
        let sp = hs.server.players.get(seat.slot).ok_or(BAD)?;
        if sp.attach_gen != seat.generation || sp.verified_pubkey != Some(seat.key) {
            return Err(BAD);
        }
        let personal = sp.personal.as_ref().ok_or(BAD)?;
        let namespace = sp.action_frontier.as_ref().ok_or(BAD)?.header(0, 0);
        let last = sp.personal_delivery.last.ok_or(BAD)?;
        if namespace.session != last.session
            || namespace.life != last.life
            || last.life != sp.respawns
            || !sp
                .personal_delivery
                .current(personal, namespace, &sp.window_events)
            || personal.binding.context().participant() != Some(seat.key)
        {
            return Err(BAD);
        }
        Self::checked(
            transfer,
            u32::try_from(seat.slot).map_err(|_| BAD)?,
            namespace.session,
            personal.binding.context(),
            namespace.life,
            sp.respawns,
        )
    }

    fn digest(&self) -> Hash {
        let mut h = Sha256::new();
        h.update(b"AxeNStax:handover:attachment:v1\0");
        h.update(self.transfer);
        h.update(self.player_index.to_le_bytes());
        h.update(self.session);
        match self.context {
            GameplayContext::WorldLocal {
                scope,
                player: WorldPlayer::Npub(key),
            } => {
                h.update([1]);
                h.update(scope);
                h.update(key);
            }
            GameplayContext::Imported(c) => {
                // These are the installed holder's values, not the next copy's.
                h.update([2]);
                h.update(c.participant);
                h.update(c.lineage);
                h.update(c.generation.to_le_bytes());
                h.update(c.transfer);
            }
            _ => unreachable!("checked signed attachment"),
        }
        h.update(self.life.to_le_bytes());
        h.update(self.observed_life.to_le_bytes());
        h.finalize().into()
    }
}

fn authority_digest(a: &Authority) -> Hash {
    let mut h = Sha256::new();
    h.update(b"AxeNStax:handover:authority:v1\0");
    h.update(a.lineage());
    h.update(a.epoch().to_le_bytes());
    h.update(a.committed());
    h.update(a.runtime());
    h.update(a.persona());
    h.update(a.expiry().to_le_bytes());
    h.finalize().into()
}

impl Fence {
    pub(crate) fn issue(
        transfer: Id,
        nonce: Id,
        authority: &Authority,
        seat: super::eligibility::Seat,
        runtime: Hash,
    ) -> Result<Self, Refusal> {
        if transfer == [0; 16] || nonce == [0; 16] {
            return Err(BAD);
        }
        Ok(Self {
            transfer,
            nonce,
            authority: authority_digest(authority),
            participant: seat.key,
            runtime,
            generation: seat.generation,
        })
    }

    pub(crate) fn check(
        &self,
        transfer: Id,
        authority: &Authority,
        participant: Hash,
        runtime: Hash,
    ) -> Result<(), Refusal> {
        if self.transfer != transfer
            || self.nonce == [0; 16]
            || self.authority != authority_digest(authority)
            || self.participant != participant
            || self.runtime != runtime
        {
            return Err(BAD);
        }
        Ok(())
    }

    fn digest(&self, authority: &Authority) -> Result<Hash, Refusal> {
        self.check(self.transfer, authority, self.participant, self.runtime)?;
        let mut h = Sha256::new();
        h.update(b"AxeNStax:handover:freeze:v1\0");
        h.update(PROTOCOL_VERSION.to_le_bytes());
        h.update(self.transfer);
        h.update(self.nonce);
        h.update(authority_digest(authority));
        h.update(self.participant);
        h.update(self.runtime);
        h.update(self.generation.to_le_bytes());
        Ok(h.finalize().into())
    }

    fn message(&self, authority: &Authority, attachment: Attachment) -> Result<Hash, Refusal> {
        if attachment.transfer != self.transfer
            || attachment.context.participant() != Some(self.participant)
        {
            return Err(BAD);
        }
        let mut h = Sha256::new();
        h.update(b"AxeNStax:handover:parked:v1\0");
        h.update(PROTOCOL_VERSION.to_le_bytes());
        h.update(self.transfer);
        h.update(self.nonce);
        h.update(self.digest(authority)?);
        h.update(attachment.digest());
        h.update(self.runtime);
        h.update(self.participant);
        Ok(h.finalize().into())
    }
}

impl Proof {
    pub(crate) fn from_parked(
        fence: &Fence,
        authority: &Authority,
        link: &ParkedPersonalLink,
        remote: &RemoteClient,
        keys: &Keys,
    ) -> Result<Self, Refusal> {
        if link.is_closed() {
            return Err(BAD);
        }
        remote
            .validate_personal_control(link.binding())
            .map_err(|_| BAD)?;
        let attachment = link.park_attachment()?;
        fence.check(
            attachment.transfer,
            authority,
            attachment.context.participant().ok_or(BAD)?,
            keys.public_key().to_bytes(),
        )?;
        let message = nostr::secp256k1::Message::from_digest(fence.message(authority, attachment)?);
        let sig = keys.sign_schnorr(&message).serialize();
        Ok(Self {
            transfer: fence.transfer,
            nonce: fence.nonce,
            fence: fence.digest(authority)?,
            signature: [
                sig[..32].try_into().expect("fixed half"),
                sig[32..].try_into().expect("fixed half"),
            ],
        })
    }

    pub(crate) fn verify(
        &self,
        fence: &Fence,
        authority: &Authority,
        attachment: Attachment,
    ) -> Result<(), Refusal> {
        if self.transfer != fence.transfer
            || self.nonce != fence.nonce
            || self.fence != fence.digest(authority)?
        {
            return Err(BAD);
        }
        let mut sig = [0; 64];
        sig[..32].copy_from_slice(&self.signature[0]);
        sig[32..].copy_from_slice(&self.signature[1]);
        if !crate::signet::verify::schnorr_verify_bip340(
            &fence.runtime,
            &fence.message(authority, attachment)?,
            &sig,
        ) {
            return Err(BAD);
        }
        Ok(())
    }
}

/// One actual encoded allocation, with the tagging overlap charged before send.
pub(crate) struct EncodedControl(Vec<u8>);

fn control_bytes(retained: usize, encoded: usize, tagged: usize) -> usize {
    // Conservatively charge the fixed preparation values and both Vec owners
    // even when their actual lifetimes do not all overlap. Body/cache/transport
    // storage has its own existing limits and is not part of this control cap.
    retained
        .saturating_add(size_of::<Frame>())
        .saturating_add(
            size_of::<Fence>() + size_of::<Proof>() + size_of::<Attachment>() + size_of::<Id>(),
        )
        .saturating_add(size_of::<EncodedControl>() + size_of::<Vec<u8>>())
        .saturating_add(encoded)
        .saturating_add(tagged)
}

/// Payload may already have lost its tag; the actual raw allocation did not.
pub(crate) fn check_incoming_control(payload: &[u8], capacity: usize) -> Result<(), Refusal> {
    if Frame::is_fixed_park_control(payload)
        && (payload.len().saturating_add(1) > WIRE_CAP || capacity > WIRE_CAP)
    {
        return Err(Refusal("The incoming handover control exceeded its limit."));
    }
    Ok(())
}

pub(crate) fn check_incoming_batch(
    retained: usize,
    fixed_capacities: usize,
    outer_capacity: usize,
) -> Result<(), Refusal> {
    // Accept can start the new phase while this exact incoming batch is still
    // owned. Reserve both maximum outgoing fixed buffers as well as the actual
    // incoming owners, before extracting any selected control from its FIFO.
    if control_bytes(
        retained
            .saturating_add(2 * WIRE_CAP)
            .saturating_add(size_of::<Vec<Vec<u8>>>()),
        fixed_capacities,
        outer_capacity,
    ) > CONTROL_CAP
    {
        return Err(Refusal(
            "The incoming handover control batch exceeded its limit.",
        ));
    }
    Ok(())
}

impl EncodedControl {
    pub(crate) fn new(frame: &Frame, retained: usize) -> Result<Self, Refusal> {
        if !matches!(frame, Frame::FreezeFence { .. } | Frame::Parked { .. }) {
            return Err(BAD);
        }
        let bytes = frame.encode().map_err(|_| BAD)?;
        if bytes.capacity() > WIRE_CAP || control_bytes(retained, bytes.capacity(), 0) > CONTROL_CAP
        {
            return Err(BAD);
        }
        Ok(Self(bytes))
    }

    pub(crate) fn tagged(&self, retained: usize) -> Result<Vec<u8>, Refusal> {
        let mut wire = Vec::new();
        wire.try_reserve_exact(self.0.len().checked_add(1).ok_or(BAD)?)
            .map_err(|_| BAD)?;
        wire.push(crate::protocol::PacketType::Handover as u8);
        wire.extend_from_slice(&self.0);
        if wire.capacity() > WIRE_CAP
            || control_bytes(retained, self.0.capacity(), wire.capacity()) > CONTROL_CAP
        {
            return Err(BAD);
        }
        Ok(wire)
    }
}

const _: () =
    assert!(size_of::<Fence>() + size_of::<Proof>() + size_of::<Attachment>() <= WIRE_CAP);

#[cfg(test)]
#[path = "park/tests.rs"]
mod tests;

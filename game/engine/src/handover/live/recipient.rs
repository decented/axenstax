//! Recipient session. It owns the joined client's transport (taken out of
//! the RemoteClient, so leaving the joined world cannot close it), shows the
//! proposal for explicit consent, verifies the signed offer against its own
//! fresh challenge, imports into a fresh non-overwriting inactive directory,
//! makes it durable, binds a latched listener, and only then persists and
//! sends Ready. No gameplay admission or advertisement happens here.
use super::eligibility::Refusal;
use super::listener::LatchedListener;
use super::wire::{Frame, acceptance_content};
use crate::handover::{
    Id,
    certificate::Core,
    checkpoint::{AdmittedManifest, DurableCheckpoint, Import, NoFault, OfferContext},
    context::RootContext,
    durable::MarkerStore,
    phase::{Authority, Receiver},
    stream::Chunk,
};
use crate::transport::ClientTransport;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Timestamp};
use std::path::PathBuf;

/// What the player is asked to accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub transfer: Id,
    pub world_name: String,
    pub source_npub: String,
    challenge: Id,
    root: RootContext,
    lineage: Id,
}

/// The player's explicit consent to exactly this proposal. Only the consent
/// caller constructs it; nothing accepts on a timer or by default.
pub struct RecipientConsent {
    pub transfer: Id,
    pub world_name: String,
    pub source_npub: String,
}

pub struct RecipientIdentity {
    pub runtime: Keys,
    pub attestation: Event,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipientStage {
    AwaitingProposal,
    AwaitingConsent,
    Accepted,
    Receiving,
    /// Durable checkpoint, latched listener and persisted Ready: inert.
    Ready,
    Aborted(&'static str),
}

pub struct RecipientSession {
    link: Box<dyn ClientTransport>,
    stage: RecipientStage,
    identity: RecipientIdentity,
    imports: PathBuf,
    markers: MarkerStore,
    proposal: Option<Proposal>,
    challenge: Id,
    socket: Option<std::net::UdpSocket>,
    manifest: Vec<u8>,
    import: Option<Import>,
    received: u64,
    expected: u64,
    core: Option<Core>,
    checkpoint: Option<DurableCheckpoint>,
    listener: Option<LatchedListener>,
    receiver: Option<Receiver>,
}

impl RecipientSession {
    pub fn new(link: Box<dyn ClientTransport>, identity: RecipientIdentity, imports: PathBuf, markers: MarkerStore) -> Self {
        Self {
            link,
            stage: RecipientStage::AwaitingProposal,
            identity,
            imports,
            markers,
            proposal: None,
            challenge: [0; 16],
            socket: None,
            manifest: Vec::new(),
            import: None,
            received: 0,
            expected: 0,
            core: None,
            checkpoint: None,
            listener: None,
            receiver: None,
        }
    }
    pub fn stage(&self) -> RecipientStage {
        self.stage
    }
    pub fn proposal(&self) -> Option<&Proposal> {
        self.proposal.as_ref()
    }
    pub fn checkpoint(&self) -> Option<&DurableCheckpoint> {
        self.checkpoint.as_ref()
    }
    pub fn listener(&self) -> Option<&LatchedListener> {
        self.listener.as_ref()
    }
    /// No gameplay before Activated, a later boundary.
    pub fn gameplay_admitted(&self) -> bool {
        self.receiver.as_ref().is_some_and(|r| r.gameplay_admitted())
    }

    fn send(&self, f: &Frame) {
        if let Ok(bytes) = f.encode() {
            let mut p = vec![crate::protocol::PacketType::Handover as u8];
            p.extend_from_slice(&bytes);
            self.link.send_to_server(&p);
        }
    }
    fn transfer(&self) -> Id {
        self.proposal.as_ref().map(|p| p.transfer).unwrap_or([0; 16])
    }

    /// Before Ready: drop the import (its Drop removes pre-rename scratch)
    /// and remove this session's own renamed but inactive checkpoint. After
    /// Ready the checkpoint is kept (quarantined) for an explicit decision.
    pub fn abort(&mut self, why: &'static str) {
        if matches!(self.stage, RecipientStage::Aborted(_)) {
            return;
        }
        self.send(&Frame::Abort { transfer: self.transfer() });
        self.import = None;
        self.listener = None;
        if self.stage != RecipientStage::Ready
            && let Some(c) = self.checkpoint.take()
        {
            let _ = std::fs::remove_dir_all(c.directory());
        }
        self.stage = RecipientStage::Aborted(why);
    }
    fn fail(&mut self, r: Refusal) -> Refusal {
        self.abort(r.0);
        r
    }

    /// Explicit consent; `listen` is the socket the latched listener binds.
    pub fn consent(&mut self, c: RecipientConsent, listen: std::net::UdpSocket, now: u64) -> Result<(), Refusal> {
        let p = self.proposal.as_ref().ok_or(Refusal("There is no handover to accept."))?;
        if self.stage != RecipientStage::AwaitingConsent
            || c.transfer != p.transfer
            || c.world_name != p.world_name
            || c.source_npub != p.source_npub
        {
            return Err(Refusal("That isn't the handover you were offered."));
        }
        let content = acceptance_content(p.transfer, p.challenge, &p.world_name, &p.source_npub);
        let acceptance = pollster::block_on(
            EventBuilder::new(Kind::Custom(crate::handover::STORED_KIND), content)
                .custom_created_at(Timestamp::from(now))
                .sign(&self.identity.runtime),
        )
        .map_err(|_| Refusal("Your consent couldn't be signed."))?;
        getrandom::fill(&mut self.challenge).map_err(|_| Refusal("No randomness."))?;
        let accept = Frame::Accept {
            transfer: p.transfer,
            runtime: self.identity.runtime.public_key().to_bytes(),
            attestation: self.identity.attestation.as_json(),
            acceptance: acceptance.as_json(),
            challenge: self.challenge,
        };
        self.socket = Some(listen);
        self.send(&accept);
        self.stage = RecipientStage::Accepted;
        Ok(())
    }

    pub fn decline(&mut self) {
        self.send(&Frame::Decline { transfer: self.transfer() });
        self.stage = RecipientStage::Aborted("declined");
    }

    pub fn pump(&mut self, now: u64) -> Result<(), Refusal> {
        if matches!(self.stage, RecipientStage::Aborted(_) | RecipientStage::Ready) {
            return Ok(());
        }
        if self.link.is_closed() {
            return Err(self.fail(Refusal("The connection to the host closed.")));
        }
        while let Some(pkt) = self.link.try_recv_from_server() {
            let Some((crate::protocol::PacketType::Handover, bytes)) = crate::protocol::deserialize_header(&pkt) else {
                continue; // gameplay for a world this session no longer plays
            };
            let r = Frame::decode(bytes)
                .map_err(|_| Refusal("The host sent a malformed handover message."))
                .and_then(|f| self.on_frame(f, now));
            if let Err(r) = r {
                return Err(self.fail(r));
            }
            if matches!(self.stage, RecipientStage::Aborted(_) | RecipientStage::Ready) {
                break;
            }
        }
        Ok(())
    }

    fn on_frame(&mut self, f: Frame, now: u64) -> Result<(), Refusal> {
        if self.proposal.is_some() && f.transfer() != self.transfer() {
            return Err(Refusal("The host mixed up two handovers."));
        }
        match (self.stage, f) {
            (_, Frame::Abort { .. }) => Err(Refusal("The host cancelled the handover.")),
            (RecipientStage::AwaitingProposal, Frame::Propose {
                transfer, world_name, source_npub, challenge, root_runtime, root_persona, root_expiry, root_relays, lineage,
            }) => {
                let root = RootContext::new(root_runtime, &root_persona, root_expiry, &root_relays, now)
                    .map_err(|_| Refusal("The host's invitation is not valid."))?;
                if challenge == [0; 16] || transfer == [0; 16] {
                    return Err(Refusal("The host's proposal was malformed."));
                }
                self.proposal = Some(Proposal { transfer, world_name, source_npub, challenge, root, lineage });
                self.stage = RecipientStage::AwaitingConsent;
                Ok(())
            }
            (RecipientStage::Accepted, Frame::ManifestPart { total, offset, bytes, .. }) => {
                if total as usize > crate::handover::manifest::MANIFEST_LIMIT
                    || offset as usize != self.manifest.len()
                    || self.manifest.len() + bytes.len() > total as usize
                {
                    return Err(Refusal("The host's manifest was malformed."));
                }
                self.manifest.extend_from_slice(&bytes);
                Ok(())
            }
            (RecipientStage::Accepted, Frame::Offer { core, offer, .. }) => self.on_offer(&core, offer.as_bytes(), now),
            (RecipientStage::Receiving, Frame::Chunk { transfer, offset, bytes }) => {
                let import = self.import.as_mut().ok_or(Refusal("No import is running."))?;
                let len = bytes.len() as u64;
                if import
                    .accept(&Chunk { transfer, offset, bytes }, now)
                    .map_err(|_| Refusal("The world data didn't check out."))?
                {
                    self.received += len;
                }
                self.send(&Frame::Ack { transfer, through: self.received });
                if self.received == self.expected {
                    self.finish(now)?;
                }
                Ok(())
            }
            _ => Err(Refusal("The host sent a handover message out of order.")),
        }
    }

    fn on_offer(&mut self, core: &[u8], offer: &[u8], now: u64) -> Result<(), Refusal> {
        let p = self.proposal.as_ref().ok_or(Refusal("No proposal."))?;
        let core = Core::decode(core).map_err(|_| Refusal("The host's certificate was malformed."))?;
        // First hop only at this base: the invitation's own runtime and
        // persona offer. Later hops need the verified chain (not wired yet).
        if core.epoch != 1 || core.source_runtime != p.root.runtime() || core.source_persona != p.root.persona() {
            return Err(Refusal("This handover needs a verified invitation chain, which this version can't follow yet."));
        }
        let authority = Authority::root(&p.root, p.lineage).map_err(|_| Refusal("The world's lineage is missing."))?;
        let me = self.identity.runtime.public_key().to_bytes();
        let persona = crate::runtime_identity::verify_player_attestation(
            &self.identity.attestation,
            &self.identity.runtime.public_key(),
            Timestamp::from(now),
        )
        .map_err(|_| Refusal("Your device's sign-in link has expired."))?
        .to_bytes();
        let admitted = AdmittedManifest::verify(
            &self.manifest,
            offer,
            OfferContext {
                core: core.clone(),
                root: &p.root,
                authority: &authority,
                challenge: self.challenge,
                recipient_runtime: me,
                recipient_persona: persona,
                now,
            },
        )
        .map_err(|_| Refusal("The host's checkpoint offer didn't verify."))?;
        self.expected = admitted.manifest().stream_length;
        self.import = Some(Import::new(admitted, &self.imports, now).map_err(|_| Refusal("The world couldn't be stored on this device."))?);
        self.core = Some(core);
        self.manifest = Vec::new();
        self.stage = RecipientStage::Receiving;
        Ok(())
    }

    /// Durable checkpoint, then the latched listener, then the persisted
    /// Ready receipt, and only then Ready is sent.
    fn finish(&mut self, now: u64) -> Result<(), Refusal> {
        let mut import = self.import.take().ok_or(Refusal("No import is running."))?;
        let checkpoint = import
            .complete(now, &mut NoFault)
            .map_err(|_| Refusal("The world couldn't be stored safely on this device."))?;
        self.checkpoint = Some(checkpoint);
        let socket = self.socket.take().ok_or(Refusal("No listening socket."))?;
        self.listener = Some(LatchedListener::bind(socket).map_err(|_| Refusal("This device couldn't open its listener."))?);
        let p = self.proposal.as_ref().ok_or(Refusal("No proposal."))?;
        let core = self.core.as_ref().ok_or(Refusal("No certificate."))?;
        let ready = pollster::block_on(crate::handover::crypto::sign_ready(&self.identity.runtime, core, p.challenge))
            .map_err(|_| Refusal("Ready couldn't be signed."))?;
        let receiver = Receiver::checkpoint_ready(&self.markers, core, &ready, p.challenge, &p.root, now)
            .map_err(|_| Refusal("Ready couldn't be recorded safely."))?;
        self.receiver = Some(receiver);
        self.stage = RecipientStage::Ready;
        self.send(&Frame::Ready { transfer: p.transfer, event: ready.as_json() });
        Ok(())
    }
}

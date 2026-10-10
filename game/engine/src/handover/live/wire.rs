//! Control frames between a source host and ONE selected joiner, carried as
//! `PacketType::Handover` payloads over their existing authenticated direct
//! connection (never a relay). Bounded: every frame is at most the ordinary
//! packet payload and decodes with a hard limit and no trailing bytes.
use crate::handover::{Error, Id, Result};
use bincode::Options;
use serde::{Deserialize, Serialize};

pub const FRAME_LIMIT: u64 = crate::protocol::MAX_PACKET_SIZE;
/// Manifest parts and archive chunks share the core's 16KiB chunk size.
pub const PART_LIMIT: usize = crate::handover::stream::CHUNK_LIMIT;
pub const TEXT_LIMIT: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frame {
    /// Source → recipient: what is offered, and the source's fresh challenge
    /// that the recipient's Ready must sign. The root fields let the
    /// recipient rebuild the invitation context it already holds.
    Propose {
        transfer: Id,
        world_name: String,
        source_npub: String,
        challenge: Id,
        root_runtime: [u8; 32],
        root_persona: String,
        root_expiry: u64,
        root_relays: Vec<String>,
        lineage: Id,
    },
    /// Recipient → source: explicit consent, proven by the recipient's
    /// runtime key (fresh signature over the proposal) and its persona's
    /// player attestation; plus the recipient's own fresh challenge that the
    /// source's checkpoint offer must sign.
    Accept {
        transfer: Id,
        runtime: [u8; 32],
        attestation: String,
        acceptance: String,
        challenge: Id,
    },
    /// Recipient → source: no, with the typed reason the source shows.
    Decline { transfer: Id, reason: DeclineReason },
    ManifestPart { transfer: Id, total: u32, offset: u32, bytes: Vec<u8> },
    /// The signed checkpoint offer plus the source's own runtime-signed
    /// agreement (`source_agreement_content`), whose hash the manifest binds.
    Offer { transfer: Id, core: Vec<u8>, offer: String, agreement: String },
    Chunk { transfer: Id, offset: u64, bytes: Vec<u8> },
    Ack { transfer: Id, through: u64 },
    Ready { transfer: Id, event: String },
    Abort { transfer: Id },
    /// Source → every joined player: play is paused for a handover (or
    /// resumes). Joiners stop sending input while paused.
    Notice { transfer: Id, paused: bool },
}

/// Why a recipient declined a proposal. Every decline is typed: the source
/// shows the reason (`copy::host_declined`), never a silent "no".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclineReason {
    /// The player chose Decline (or left the world before answering).
    Player,
    /// The recipient is already in another handover.
    Busy,
    /// The recipient's join verified no host (a LAN join): it can't check
    /// that the proposal comes from the invitation's host.
    UnverifiedHost,
    /// The recipient isn't signed in for online play on this machine.
    NotReady,
    /// The proposal didn't match the host the recipient verified.
    Invalid,
}

fn options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(FRAME_LIMIT)
        .reject_trailing_bytes()
}

impl Frame {
    pub fn transfer(&self) -> Id {
        match self {
            Frame::Propose { transfer, .. }
            | Frame::Accept { transfer, .. }
            | Frame::Decline { transfer, .. }
            | Frame::ManifestPart { transfer, .. }
            | Frame::Offer { transfer, .. }
            | Frame::Chunk { transfer, .. }
            | Frame::Ack { transfer, .. }
            | Frame::Ready { transfer, .. }
            | Frame::Abort { transfer }
            | Frame::Notice { transfer, .. } => *transfer,
        }
    }
    fn validate(&self) -> Result<()> {
        let long = |s: &String| s.len() > TEXT_LIMIT;
        let bad = match self {
            Frame::Propose { world_name, source_npub, root_persona, root_relays, .. } => {
                world_name.len() > 128 || long(source_npub) || long(root_persona) || root_relays.len() > 8 || root_relays.iter().any(long)
            }
            Frame::Accept { attestation, acceptance, .. } => long(attestation) || long(acceptance),
            Frame::ManifestPart { bytes, .. } | Frame::Chunk { bytes, .. } => bytes.len() > PART_LIMIT,
            Frame::Offer { core, offer, agreement, .. } => {
                core.len() > crate::handover::certificate::CORE_LIMIT || long(offer) || long(agreement)
            }
            Frame::Ready { event, .. } => long(event),
            _ => false,
        };
        if bad {
            return Err(Error::Limit("handover frame field"));
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        options().serialize(self).map_err(|_| Error::Limit("handover frame"))
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let f: Frame = options().deserialize(bytes).map_err(|_| Error::Invalid("handover frame"))?;
        f.validate()?;
        Ok(f)
    }
}

/// The recipient's acceptance, signed by its runtime key: domain, transfer,
/// the source's challenge, the world name and the source npub it saw.
pub fn acceptance_content(transfer: Id, challenge: Id, world_name: &str, source_npub: &str) -> String {
    use base64::Engine;
    let mut v = Vec::new();
    crate::handover::codec::frame(&mut v, b"AxeNStax/handover/accept/v1");
    v.extend_from_slice(&transfer);
    v.extend_from_slice(&challenge);
    crate::handover::codec::frame(&mut v, world_name.as_bytes());
    crate::handover::codec::frame(&mut v, source_npub.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(v)
}

/// The source's own agreement, signed by its runtime key (the key the
/// certificate names as source runtime): a distinct domain from the
/// recipient's acceptance, binding transfer, the source's challenge, the
/// world name and the recipient npub it is handing to.
pub fn source_agreement_content(transfer: Id, challenge: Id, world_name: &str, recipient_npub: &str) -> String {
    use base64::Engine;
    let mut v = Vec::new();
    crate::handover::codec::frame(&mut v, b"AxeNStax/handover/source-agree/v1");
    v.extend_from_slice(&transfer);
    v.extend_from_slice(&challenge);
    crate::handover::codec::frame(&mut v, world_name.as_bytes());
    crate::handover::codec::frame(&mut v, recipient_npub.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(v)
}

/// The lineage of a root invitation: derived from its context digest, so
/// the recipient recomputes it from the host it verified instead of
/// trusting a value the source sent. A new invitation is a new lineage.
pub fn root_lineage(root: &crate::handover::context::RootContext) -> Id {
    let mut v = b"AxeNStax/handover/lineage/v1".to_vec();
    v.extend_from_slice(&root.digest());
    let h = crate::handover::hash(&v);
    let mut id = [0; 16];
    id.copy_from_slice(&h[..16]);
    id
}

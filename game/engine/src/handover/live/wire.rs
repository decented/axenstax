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
    Decline { transfer: Id },
    ManifestPart { transfer: Id, total: u32, offset: u32, bytes: Vec<u8> },
    Offer { transfer: Id, core: Vec<u8>, offer: String },
    Chunk { transfer: Id, offset: u64, bytes: Vec<u8> },
    Ack { transfer: Id, through: u64 },
    Ready { transfer: Id, event: String },
    Abort { transfer: Id },
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
            | Frame::Decline { transfer }
            | Frame::ManifestPart { transfer, .. }
            | Frame::Offer { transfer, .. }
            | Frame::Chunk { transfer, .. }
            | Frame::Ack { transfer, .. }
            | Frame::Ready { transfer, .. }
            | Frame::Abort { transfer } => *transfer,
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
            Frame::Offer { core, offer, .. } => core.len() > crate::handover::certificate::CORE_LIMIT || long(offer),
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

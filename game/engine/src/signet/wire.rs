//! Wire-friendly DTOs for Signet auth events and credentials.
//!
//! In-memory `SignetAuthEvent` and `SignetCredential` (in `event.rs`) carry
//! the 64-byte BIP-340 signature as `[u8; 64]`, which serde 1.x's `derive`
//! macro doesn't support — array derives top out at 32. Rather than push the
//! crypto types behind a custom serde impl, this module mirrors them with
//! `Vec<u8>` for the over-32 fields and length-validates on conversion.
//!
//! The wire shape is what travels inside `JoinRequestPacket` (Phase 3 of the
//! foundations spec). `verify::*` operates on the in-memory shape, so the
//! call site is:
//!
//!   wire → SignetAuthEvent (TryFrom, validates lengths) → verify_auth_event
//!
//! Wire types are cross-platform: a WASM client constructs and sends them
//! over the QUIC/channel transport. Conversion into the crypto types
//! (`From<&...>` / `TryFrom`) is native-only because the crypto types
//! themselves use libsecp256k1, which the engine doesn't pull in on WASM.

use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use super::event::{SignetAuthEvent, SignetCredential};

/// Wire form of `SignetAuthEvent`. `id` and `pubkey` are 32-byte arrays
/// (within serde's derive limit); `sig` is a `Vec<u8>` length-validated to
/// 64 in `TryFrom<SignetAuthEventWire>`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignetAuthEventWire {
    pub pubkey: [u8; 32],
    pub created_at: u32,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub id: [u8; 32],
    /// 64-byte BIP-340 signature; rejected on conversion if `len() != 64`.
    pub sig: Vec<u8>,
    pub from_np: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignetCredentialWire {
    pub pubkey: [u8; 32],
    pub created_at: u32,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub id: [u8; 32],
    pub sig: Vec<u8>,
}

/// Reasons a wire DTO can fail to convert into the in-memory crypto type.
/// Length checks only — cryptographic verification is layered separately.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// Signature was not exactly 64 bytes. Carries the actual length.
    BadSignatureLen(usize),
}

#[cfg(not(target_arch = "wasm32"))]
impl core::fmt::Display for WireError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WireError::BadSignatureLen(n) => write!(f, "bad signature length: {n} (expected 64)"),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::error::Error for WireError {}

#[cfg(not(target_arch = "wasm32"))]
impl From<&SignetAuthEvent> for SignetAuthEventWire {
    fn from(e: &SignetAuthEvent) -> Self {
        Self {
            pubkey: e.pubkey,
            created_at: e.created_at,
            kind: e.kind,
            tags: e.tags.clone(),
            content: e.content.clone(),
            id: e.id,
            sig: e.sig.to_vec(),
            from_np: e.from_np,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl TryFrom<SignetAuthEventWire> for SignetAuthEvent {
    type Error = WireError;
    fn try_from(w: SignetAuthEventWire) -> Result<Self, Self::Error> {
        if w.sig.len() != 64 {
            return Err(WireError::BadSignatureLen(w.sig.len()));
        }
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&w.sig);
        Ok(SignetAuthEvent {
            pubkey: w.pubkey,
            created_at: w.created_at,
            kind: w.kind,
            tags: w.tags,
            content: w.content,
            id: w.id,
            sig,
            from_np: w.from_np,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<&SignetCredential> for SignetCredentialWire {
    fn from(c: &SignetCredential) -> Self {
        Self {
            pubkey: c.pubkey,
            created_at: c.created_at,
            kind: c.kind,
            tags: c.tags.clone(),
            content: c.content.clone(),
            id: c.id,
            sig: c.sig.to_vec(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl TryFrom<SignetCredentialWire> for SignetCredential {
    type Error = WireError;
    fn try_from(w: SignetCredentialWire) -> Result<Self, Self::Error> {
        if w.sig.len() != 64 {
            return Err(WireError::BadSignatureLen(w.sig.len()));
        }
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&w.sig);
        Ok(SignetCredential {
            pubkey: w.pubkey,
            created_at: w.created_at,
            kind: w.kind,
            tags: w.tags,
            content: w.content,
            id: w.id,
            sig,
        })
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use super::super::event::AUTH_EVENT_KIND;

    fn sample_event() -> SignetAuthEvent {
        SignetAuthEvent {
            pubkey: [0xaau8; 32],
            created_at: 1_700_000_000,
            kind: AUTH_EVENT_KIND,
            tags: vec![
                vec!["challenge".into(), "abc".into()],
                vec!["origin".into(), "https://example.com".into()],
            ],
            content: String::new(),
            id: [0xccu8; 32],
            sig: [0xdeu8; 64],
            from_np: false,
        }
    }

    #[test]
    fn auth_event_round_trips_via_wire() {
        let original = sample_event();
        let wire: SignetAuthEventWire = (&original).into();
        let back: SignetAuthEvent = wire.try_into().expect("valid wire converts back");
        assert_eq!(back.pubkey, original.pubkey);
        assert_eq!(back.created_at, original.created_at);
        assert_eq!(back.kind, original.kind);
        assert_eq!(back.tags, original.tags);
        assert_eq!(back.content, original.content);
        assert_eq!(back.id, original.id);
        assert_eq!(back.sig, original.sig);
        assert_eq!(back.from_np, original.from_np);
    }

    #[test]
    fn auth_event_wire_serializes_via_bincode() {
        // Phase 3's whole point: this struct must travel inside JoinRequestPacket.
        // Round-trip via bincode catches any field that derive(Serialize) can't handle.
        let original = sample_event();
        let wire: SignetAuthEventWire = (&original).into();
        let bytes = bincode::serialize(&wire).expect("wire must bincode-serialize");
        let decoded: SignetAuthEventWire = bincode::deserialize(&bytes).expect("must decode");
        let back: SignetAuthEvent = decoded.try_into().expect("decoded wire converts back");
        assert_eq!(back.sig, original.sig);
        assert_eq!(back.id, original.id);
    }

    #[test]
    fn short_sig_rejected_on_conversion() {
        let mut wire: SignetAuthEventWire = (&sample_event()).into();
        wire.sig.truncate(32);
        let err = SignetAuthEvent::try_from(wire).unwrap_err();
        assert_eq!(err, WireError::BadSignatureLen(32));
    }

    #[test]
    fn long_sig_rejected_on_conversion() {
        let mut wire: SignetAuthEventWire = (&sample_event()).into();
        wire.sig.resize(96, 0);
        let err = SignetAuthEvent::try_from(wire).unwrap_err();
        assert_eq!(err, WireError::BadSignatureLen(96));
    }

    #[test]
    fn credential_round_trips_via_wire_and_bincode() {
        use super::super::event::CREDENTIAL_KIND;

        let original = SignetCredential {
            pubkey: [0xaau8; 32],
            created_at: 1_700_000_000,
            kind: CREDENTIAL_KIND,
            tags: vec![vec!["display-name".into(), "AxoLittle".into()]],
            content: String::new(),
            id: [0xccu8; 32],
            sig: [0xdeu8; 64],
        };
        let wire: SignetCredentialWire = (&original).into();
        let bytes = bincode::serialize(&wire).unwrap();
        let decoded: SignetCredentialWire = bincode::deserialize(&bytes).unwrap();
        let back: SignetCredential = decoded.try_into().unwrap();
        assert_eq!(back.sig, original.sig);
        assert_eq!(back.tags, original.tags);
    }
}

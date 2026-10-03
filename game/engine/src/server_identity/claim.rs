#![cfg(not(target_arch = "wasm32"))]
// Forward-API: the sign/verify primitive is fully tested; specific emitters
// (leaderboard score, payout receipt, integrity event) are wired into the
// economy/score paths as a per-claim product decision.
#![allow(dead_code)]
//! Signed authoritative claims (Track 6).
//!
//! A provisioned server can sign claims — scores, payouts, world-integrity
//! events — with its runtime key. A verifier checks the claim's signature AND
//! that the runtime key is authorised by the operator's attestation (chaining to
//! the expected operator npub), exactly like the join proof. This makes a
//! server's authoritative outputs attributable and auditable.
//!
//! This module is the sign + verify primitive; wiring specific emitters (e.g. a
//! leaderboard score, a Lightning payout receipt) into the economy/score paths
//! is the integration step and a per-claim product decision.

use nostr::{Event, EventBuilder, Kind, PublicKey, Tag, Timestamp, ToBech32};

use crate::server_identity::attestation::{parse_server_pubkey, verify};
use crate::server_identity::store::ServerIdentity;

/// Claim event kind (regular event; the `claims` kind reserved in the
/// attestation's `allowed_kinds`). Provisional — register in `forgesworn/nips`.
pub const CLAIM_KIND: u16 = 27421;

/// A verified server claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimView {
    /// The server runtime key that signed the claim.
    pub server_pubkey: PublicKey,
    /// The operator the runtime key chains to (verified).
    pub operator: PublicKey,
    /// Application claim type (the `t` tag), e.g. "score", "payout".
    pub claim_type: String,
    /// Claim payload (event content) — application-defined (often JSON).
    pub payload: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimError {
    Malformed,
    AttestationInvalid,
    OperatorMismatch,
    BadSignature,
    /// The claim was signed by a key the attestation doesn't authorise.
    WrongSigner,
}

/// Sign an authoritative claim with the server runtime key. `None` if the server
/// is unprovisioned (an unattested claim isn't verifiable, so we don't make one).
pub fn sign_claim(id: &ServerIdentity, claim_type: &str, payload: &str) -> Option<Event> {
    id.attestation()?; // only a provisioned (attested) server makes verifiable claims
    let tag = Tag::parse(["t", claim_type]).ok()?;
    let unsigned = EventBuilder::new(Kind::Custom(CLAIM_KIND), payload)
        .tags([tag])
        .build(id.runtime_pubkey());
    id.sign_event(unsigned).ok()
}

/// Verify a claim against the operator npub the consumer expects. Needs the
/// server's attestation to chain `claim signer (runtime key) → operator`.
pub fn verify_claim(
    claim: &Event,
    attestation: &Event,
    expected_op_npub: &str,
    now: Timestamp,
) -> Result<ClaimView, ClaimError> {
    let server_pubkey = parse_server_pubkey(attestation).ok_or(ClaimError::Malformed)?;
    let att = verify(attestation, &server_pubkey, now).map_err(|_| ClaimError::AttestationInvalid)?;
    let op_npub = att.operator.to_bech32().map_err(|_| ClaimError::Malformed)?;
    if op_npub != expected_op_npub {
        return Err(ClaimError::OperatorMismatch);
    }
    if claim.verify().is_err() {
        return Err(ClaimError::BadSignature);
    }
    if claim.pubkey != server_pubkey {
        return Err(ClaimError::WrongSigner);
    }
    if claim.kind != Kind::Custom(CLAIM_KIND) {
        return Err(ClaimError::Malformed);
    }
    let claim_type = claim
        .tags
        .iter()
        .find_map(|t| {
            let s = t.as_slice();
            (s.len() >= 2 && s[0] == "t").then(|| s[1].clone())
        })
        .unwrap_or_default();
    Ok(ClaimView {
        server_pubkey,
        operator: att.operator,
        claim_type,
        payload: claim.content.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_identity::attestation::mint_attestation;
    use crate::server_identity::store::generate_runtime;
    use nostr::{Keys, ToBech32};

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("axe_claim_{}_{}", tag, std::process::id()))
    }

    async fn provisioned(dir: &std::path::Path, op: &Keys) -> (ServerIdentity, Event) {
        let mut id = generate_runtime(dir).unwrap();
        let att = mint_attestation(
            op,
            &id.runtime_pubkey(),
            Timestamp::from(0),
            Timestamp::from(u64::MAX >> 1),
            &[CLAIM_KIND],
            "W",
            None,
        )
        .await
        .unwrap();
        id.store_attestation(dir, att.clone()).unwrap();
        (id, att)
    }

    #[tokio::test]
    async fn signed_claim_verifies_to_operator() {
        let dir = tmp("ok");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (id, att) = provisioned(&dir, &op).await;
        let claim = sign_claim(&id, "score", "{\"v\":42}").unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        let view = verify_claim(&claim, &att, &npub, Timestamp::from(100)).unwrap();
        assert_eq!(view.operator, op.public_key());
        assert_eq!(view.server_pubkey, id.runtime_pubkey());
        assert_eq!(view.claim_type, "score");
        assert_eq!(view.payload, "{\"v\":42}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unprovisioned_signs_no_claim() {
        let dir = tmp("none");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        assert!(sign_claim(&id, "score", "1").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn claim_rejected_for_wrong_operator() {
        let dir = tmp("op");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (id, att) = provisioned(&dir, &op).await;
        let claim = sign_claim(&id, "score", "1").unwrap();
        let other = Keys::generate().public_key().to_bech32().unwrap();
        assert_eq!(
            verify_claim(&claim, &att, &other, Timestamp::from(100)),
            Err(ClaimError::OperatorMismatch)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn claim_from_unauthorised_signer_rejected() {
        let dir = tmp("signer");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (_id, att) = provisioned(&dir, &op).await;
        // A claim signed by some OTHER key, presented with the real attestation.
        let imposter = Keys::generate();
        let claim = EventBuilder::new(Kind::Custom(CLAIM_KIND), "1")
            .tags([Tag::parse(["t", "score"]).unwrap()])
            .sign_with_keys(&imposter)
            .unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        assert_eq!(
            verify_claim(&claim, &att, &npub, Timestamp::from(100)),
            Err(ClaimError::WrongSigner)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tampered_claim_rejected() {
        let dir = tmp("tamper");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (id, att) = provisioned(&dir, &op).await;
        let mut claim = sign_claim(&id, "score", "1").unwrap();
        claim.content = "999".to_string();
        let npub = op.public_key().to_bech32().unwrap();
        assert_eq!(
            verify_claim(&claim, &att, &npub, Timestamp::from(100)),
            Err(ClaimError::BadSignature)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

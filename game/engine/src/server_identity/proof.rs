#![cfg(not(target_arch = "wasm32"))]
//! Server-identity proof exchanged at join time (Track 3).
//!
//! The client sends a fresh nonce in its `JoinRequest`; the server answers in
//! `JoinAccept` with this proof: its attestation event plus a BIP-340 signature,
//! made by the runtime key, over `challenge_msg(client_nonce, origin)`. A client
//! that pinned an operator npub (from the connect-string `#op=`) verifies the
//! whole chain — `runtime key signed my nonce` **and** `attestation authorises
//! that runtime key` **and** `attestation is by the expected operator` — and
//! refuses the connection on any failure. A client with no pinned operator
//! ignores the proof (anonymous join, unchanged).

use nostr::{Event, JsonUtil, PublicKey, Timestamp, ToBech32};

use crate::protocol::ServerIdentityProof;
use crate::server_identity::attestation::{parse_server_pubkey, verify};
use crate::server_identity::store::{challenge_msg, ServerIdentity};
use crate::signet::verify::schnorr_verify_bip340;

/// Fixed domain-separation origin the server signs the join proof over (and the
/// client verifies against). Trust here is the **operator npub**, not the host,
/// and the client's fresh per-join nonce is the anti-replay guarantee — so a
/// constant origin (rather than a brittle host string both ends must reconstruct
/// identically) is sufficient and removes a whole class of live-test mismatch.
pub const SERVER_IDENTITY_ORIGIN: &str = "axenstax:server-identity:v1";

/// The origin the join proof is actually signed and verified over (protocol v63,
/// audit fix B): `SERVER_IDENTITY_ORIGIN + "|" + signet::join_origin(binding)`,
/// where `binding` is each side's OWN view of the QUIC TLS exporter. Without the
/// binding, a relaying host M could forward a pinned client's nonce to the real
/// host H and hand H's proof back, so the client would show "verified operator"
/// while talking to M. With it, H's proof carries the H↔M exporter and the
/// client (on V↔M) refuses it. Residual: WebSocket is unbound on both legs
/// (TLS ends at the proxy), so there a relay still passes.
pub fn server_identity_origin(binding: Option<[u8; 32]>) -> String {
    format!("{SERVER_IDENTITY_ORIGIN}|{}", crate::signet::join_origin(binding))
}

/// The client's join-time verdict on a server's identity (C1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerAuthOutcome {
    /// No operator was pinned (bare `ws://`/addr) — any proof is ignored, join
    /// proceeds anonymously exactly as before.
    Anonymous,
    /// An operator was pinned and the server's proof verified to it. Carries the
    /// verified operator npub (for display / the inspect view).
    Verified(String),
    /// An operator was pinned but verification failed (no proof, wrong operator,
    /// expired attestation, bad signature …). The connection must be refused.
    Refused(String),
}

// Consumed by the native client at the 2-machine live-test boundary (threading
// the pinned operator npub through the connect state machine); fully unit-tested.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerProofError {
    /// Attestation JSON unparseable, missing `d`, or signature length wrong.
    Malformed,
    /// Attestation itself doesn't verify (operator sig / window / kind).
    AttestationInvalid,
    /// Attestation is by a different operator than the client pinned.
    OperatorMismatch,
    /// The runtime key's signature over the client nonce doesn't verify.
    BadChallengeSig,
}

/// Build the proof for a joining client's nonce. `None` if the server is
/// unprovisioned (anonymous) — the join proceeds without a proof.
pub fn identity_proof(
    id: &ServerIdentity,
    client_nonce: &[u8],
    origin: &str,
) -> Option<ServerIdentityProof> {
    let att = id.attestation()?;
    Some(ServerIdentityProof {
        attestation_json: att.event.as_json(),
        challenge_sig: id.sign_challenge(client_nonce, origin).to_vec(),
    })
}

/// Verify a server's proof against the operator npub the client pinned.
/// Returns the verified operator pubkey on success.
#[allow(dead_code)] // wired into the client connect flow at the live-test boundary
pub fn verify_server_proof(
    proof: &ServerIdentityProof,
    client_nonce: &[u8],
    origin: &str,
    expected_op_npub: &str,
    now: Timestamp,
) -> Result<PublicKey, ServerProofError> {
    let event = Event::from_json(&proof.attestation_json).map_err(|_| ServerProofError::Malformed)?;
    let server_pubkey = parse_server_pubkey(&event).ok_or(ServerProofError::Malformed)?;
    // The attestation must fully verify (operator signature + validity window)
    // for the runtime key it names.
    let att = verify(&event, &server_pubkey, now).map_err(|_| ServerProofError::AttestationInvalid)?;
    // …and be a SERVER delegation. A `role=player` event is a persona's
    // delegation to a player's runtime key (`runtime_identity`); accepting one
    // here would let a player attestation stand in for a server's, exactly what
    // `store::store_attestation` refuses on the way in.
    if att.role.is_some() {
        return Err(ServerProofError::AttestationInvalid);
    }
    // …and be by the operator the client pinned.
    let op_npub = att.operator.to_bech32().map_err(|_| ServerProofError::Malformed)?;
    if op_npub != expected_op_npub {
        return Err(ServerProofError::OperatorMismatch);
    }
    // …and the runtime key must have signed THIS client's nonce.
    let sig: [u8; 64] = proof
        .challenge_sig
        .as_slice()
        .try_into()
        .map_err(|_| ServerProofError::Malformed)?;
    let msg = challenge_msg(client_nonce, origin);
    if !schnorr_verify_bip340(&server_pubkey.to_bytes(), &msg, &sig) {
        return Err(ServerProofError::BadChallengeSig);
    }
    Ok(att.operator)
}

/// The client's join-time decision: given the operator npub it pinned (from the
/// connect-string `#op=` fragment, `None` = anonymous) and the proof the server
/// returned in `JoinAccept`, decide whether to trust, refuse, or ignore. Pure +
/// total, so the connect flow just maps the outcome to a state transition. The
/// proof is verified over [`server_identity_origin`] of the client's OWN
/// transport channel binding.
pub fn evaluate_server_identity(
    pinned_op_npub: Option<&str>,
    proof: Option<&ServerIdentityProof>,
    client_nonce: &[u8],
    binding: Option<[u8; 32]>,
    now: Timestamp,
) -> ServerAuthOutcome {
    let Some(pin) = pinned_op_npub else {
        return ServerAuthOutcome::Anonymous;
    };
    let Some(proof) = proof else {
        return ServerAuthOutcome::Refused(
            "you pinned an operator, but this server offered no identity proof".to_string(),
        );
    };
    let origin = server_identity_origin(binding);
    match verify_server_proof(proof, client_nonce, &origin, pin, now) {
        Ok(op) => ServerAuthOutcome::Verified(op.to_bech32().unwrap_or_default()),
        Err(e) => ServerAuthOutcome::Refused(format!("server identity check failed: {e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_identity::attestation::mint_attestation;
    use crate::server_identity::store::generate_runtime;
    use nostr::Keys;

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("axe_proof_{}_{}", tag, std::process::id()))
    }

    // Provision an identity with a valid attestation by operator `op`, valid
    // across [0, big]. Returns the identity (paired).
    async fn provisioned(dir: &std::path::Path, op: &Keys) -> ServerIdentity {
        let mut id = generate_runtime(dir).unwrap();
        let ev = mint_attestation(
            op,
            &id.runtime_pubkey(),
            Timestamp::from(0),
            Timestamp::from(u64::MAX >> 1),
            &[27420],
            "W",
            None,
        )
        .await
        .unwrap();
        id.store_attestation(dir, ev).unwrap();
        id
    }

    const ORIGIN: &str = "wss://play.example.com/ws";
    const NONCE: &[u8] = b"client-chosen-nonce";

    #[tokio::test]
    async fn valid_proof_verifies_to_operator() {
        let dir = tmp("ok");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        let proof = identity_proof(&id, NONCE, ORIGIN).unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        let got = verify_server_proof(&proof, NONCE, ORIGIN, &npub, Timestamp::from(100)).unwrap();
        assert_eq!(got, op.public_key());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// REGRESSION (whole-branch review, MINOR 7). `store::store_attestation`
    /// refuses a `role=player` delegation on the way in, but `verify_server_proof`
    /// accepted one it was handed — so a player's own attestation, which any
    /// player can mint with one phone tap, could be presented as an operator's
    /// delegation and the client would pin the wrong thing.
    #[tokio::test]
    async fn a_player_delegation_is_not_a_server_proof() {
        let dir = tmp("role");
        let _ = std::fs::remove_dir_all(&dir);
        let persona = Keys::generate();
        let id = generate_runtime(&dir).unwrap();
        // Exactly what a player's install holds after its one phone tap …
        let player_att = crate::runtime_identity::mint_player_attestation(
            &persona,
            &id.runtime_pubkey(),
            Timestamp::from(0),
            365,
        )
        .await
        .unwrap();
        // … presented as a server proof, with a genuine signature over the
        // client's nonce by the very key the attestation names. Everything
        // except the role is in order.
        let proof = ServerIdentityProof {
            attestation_json: player_att.as_json(),
            challenge_sig: id.sign_challenge(NONCE, ORIGIN).to_vec(),
        };
        let npub = persona.public_key().to_bech32().unwrap();
        assert_eq!(
            verify_server_proof(&proof, NONCE, ORIGIN, &npub, Timestamp::from(100)),
            Err(ServerProofError::AttestationInvalid),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unprovisioned_has_no_proof() {
        let dir = tmp("none");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        assert!(identity_proof(&id, NONCE, ORIGIN).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn wrong_nonce_fails_challenge() {
        let dir = tmp("nonce");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        let proof = identity_proof(&id, NONCE, ORIGIN).unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        assert_eq!(
            verify_server_proof(&proof, b"different-nonce", ORIGIN, &npub, Timestamp::from(100)),
            Err(ServerProofError::BadChallengeSig)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn wrong_origin_fails_challenge() {
        let dir = tmp("origin");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        let proof = identity_proof(&id, NONCE, ORIGIN).unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        assert_eq!(
            verify_server_proof(&proof, NONCE, "wss://evil/ws", &npub, Timestamp::from(100)),
            Err(ServerProofError::BadChallengeSig)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn operator_mismatch_is_rejected() {
        let dir = tmp("op");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        let proof = identity_proof(&id, NONCE, ORIGIN).unwrap();
        let other_npub = Keys::generate().public_key().to_bech32().unwrap();
        assert_eq!(
            verify_server_proof(&proof, NONCE, ORIGIN, &other_npub, Timestamp::from(100)),
            Err(ServerProofError::OperatorMismatch)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn expired_attestation_is_rejected() {
        let dir = tmp("exp");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let mut id = generate_runtime(&dir).unwrap();
        let ev = mint_attestation(
            &op,
            &id.runtime_pubkey(),
            Timestamp::from(0),
            Timestamp::from(100),
            &[27420],
            "W",
            None,
        )
        .await
        .unwrap();
        id.store_attestation(&dir, ev).unwrap();
        let proof = identity_proof(&id, NONCE, ORIGIN).unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        // now=500 is past valid_until=100.
        assert_eq!(
            verify_server_proof(&proof, NONCE, ORIGIN, &npub, Timestamp::from(500)),
            Err(ServerProofError::AttestationInvalid)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_attestation_json_is_rejected() {
        let proof = ServerIdentityProof {
            attestation_json: "not json".to_string(),
            challenge_sig: vec![0u8; 64],
        };
        assert_eq!(
            verify_server_proof(&proof, NONCE, ORIGIN, "npub1xyz", Timestamp::from(1)),
            Err(ServerProofError::Malformed)
        );
    }

    // ── evaluate_server_identity: the client's join-time decision (C1) ──────────
    // The proof is built and verified over SERVER_IDENTITY_ORIGIN (a fixed
    // domain-separation string), so the client never has to match a host string.

    #[test]
    fn anonymous_when_no_operator_pinned() {
        // No `#op=` in the connect string → ignore any proof, join anonymously.
        let out = evaluate_server_identity(None, None, NONCE, None, Timestamp::from(1));
        assert_eq!(out, ServerAuthOutcome::Anonymous);
    }

    #[tokio::test]
    async fn verified_when_proof_matches_pinned_operator() {
        let dir = tmp("eval_ok");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        let proof = identity_proof(&id, NONCE, &server_identity_origin(None)).unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        let out = evaluate_server_identity(Some(&npub), Some(&proof), NONCE, None, Timestamp::from(100));
        assert_eq!(out, ServerAuthOutcome::Verified(npub));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refused_when_pinned_but_no_proof_offered() {
        // Pinned an operator but the server sent no proof (unprovisioned / older) → refuse.
        let out = evaluate_server_identity(Some("npub1whatever"), None, NONCE, None, Timestamp::from(1));
        assert!(matches!(out, ServerAuthOutcome::Refused(_)));
    }

    #[tokio::test]
    async fn refused_when_proof_is_for_a_different_operator() {
        let dir = tmp("eval_mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        let proof = identity_proof(&id, NONCE, &server_identity_origin(None)).unwrap();
        let other_npub = Keys::generate().public_key().to_bech32().unwrap();
        let out =
            evaluate_server_identity(Some(&other_npub), Some(&proof), NONCE, None, Timestamp::from(100));
        assert!(matches!(out, ServerAuthOutcome::Refused(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Channel binding (audit fix B, v63) ────────────────────────────────────
    // The proof is signed over SERVER_IDENTITY_ORIGIN + "|" + join_origin(binding),
    // so a relaying host that forwards the real host's proof (made on the H↔M
    // channel) fails on the victim's V↔M channel.

    #[test]
    fn server_identity_origin_appends_the_join_origin() {
        let b = [0x11; 32];
        assert_eq!(
            server_identity_origin(Some(b)),
            format!("{SERVER_IDENTITY_ORIGIN}|{}", crate::signet::join_origin(Some(b)))
        );
        assert_eq!(
            server_identity_origin(None),
            format!("{SERVER_IDENTITY_ORIGIN}|axenstax-join:unbound")
        );
    }

    #[tokio::test]
    async fn verified_when_proof_is_bound_to_the_same_channel() {
        let dir = tmp("eval_bound_ok");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        let a = Some([0xaa; 32]);
        let proof = identity_proof(&id, NONCE, &server_identity_origin(a)).unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        let out = evaluate_server_identity(Some(&npub), Some(&proof), NONCE, a, Timestamp::from(100));
        assert_eq!(out, ServerAuthOutcome::Verified(npub));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn relayed_proof_from_another_channel_is_refused() {
        let dir = tmp("eval_relay");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        // H signed on the H↔M channel (A); V checks on the V↔M channel (B).
        let proof = identity_proof(&id, NONCE, &server_identity_origin(Some([0xaa; 32]))).unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        let out = evaluate_server_identity(
            Some(&npub),
            Some(&proof),
            NONCE,
            Some([0xbb; 32]),
            Timestamp::from(100),
        );
        assert!(matches!(out, ServerAuthOutcome::Refused(ref r) if r.contains("BadChallengeSig")), "{out:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unbound_legacy_proof_is_refused_on_a_bound_channel() {
        let dir = tmp("eval_legacy");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let id = provisioned(&dir, &op).await;
        let proof = identity_proof(&id, NONCE, SERVER_IDENTITY_ORIGIN).unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        let out = evaluate_server_identity(
            Some(&npub),
            Some(&proof),
            NONCE,
            Some([0xaa; 32]),
            Timestamp::from(100),
        );
        assert!(matches!(out, ServerAuthOutcome::Refused(_)), "{out:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

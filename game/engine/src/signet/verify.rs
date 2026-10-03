//! BIP-340 Schnorr verification + auth-event binding checks.
//!
//! Reference: `tools/sites/game/auth.py:verify_signet_auth_event` (line 80).
//!
//! The Python side brute-forces `created_at` across a ±300 s window because
//! the redirect flow doesn't return it. The engine side is different: the
//! full event travels inside `JoinRequestPacket`, `created_at` included, so
//! we can check the window directly against the provided value.

use super::event::{
    canonical_id, SignetAuthEvent, SignetCredential, AUTH_EVENT_KIND, CREDENTIAL_KIND,
};

/// Tolerance (seconds either side of `now`) for `created_at`. Matches the
/// website's `AUTH_EVENT_CREATED_AT_SKEW` — phone clocks drift.
pub const AUTH_EVENT_SKEW_SECS: u32 = 300;

/// Outcome of a verify call. `Ok` is the only accept; every other variant is
/// a distinct reason-for-rejection, logged by the server for ops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyResult {
    Ok,
    BadSignature,
    BadEventId,
    WrongKind,
    WrongChallenge,
    WrongOrigin,
    CreatedAtOutsideSkew,
    ExpiredCredential,
    NpFallbackRejected,
    CredentialPubkeyMismatch,
}

/// BIP-340 Schnorr verify (x-only pubkey, 32-byte message, 64-byte sig).
/// Semantically matches `_schnorr_verify_raw` in `tools/sites/game/auth.py`.
pub fn schnorr_verify_bip340(pubkey: &[u8; 32], msg32: &[u8; 32], sig: &[u8; 64]) -> bool {
    use secp256k1::{schnorr::Signature, Message, Secp256k1, XOnlyPublicKey};

    let Ok(xonly) = XOnlyPublicKey::from_slice(pubkey) else { return false; };
    let Ok(signature) = Signature::from_slice(sig) else { return false; };
    let Ok(msg) = Message::from_digest_slice(msg32) else { return false; };
    // Verification context is cheap to construct; no signing key involved.
    let secp = Secp256k1::verification_only();
    secp.verify_schnorr(&signature, &msg, &xonly).is_ok()
}

/// Verify an auth event against a challenge + origin the server issued.
///
/// Checks (in order):
/// 1. `kind == AUTH_EVENT_KIND` and `content == ""`.
/// 2. `from_np == false` (reject NP persona per §1.8.6).
/// 3. Required tags `["challenge", expected]` and `["origin", expected]` present.
/// 4. `created_at` within `±AUTH_EVENT_SKEW_SECS` of `now_ts`.
/// 5. Recomputed canonical id matches the claimed `id` byte-for-byte.
/// 6. Schnorr signature over `id` validates against `pubkey`.
pub fn verify_auth_event(
    event: &SignetAuthEvent,
    expected_challenge: &str,
    expected_origin: &str,
    now_ts: u32,
) -> VerifyResult {
    if event.kind != AUTH_EVENT_KIND {
        return VerifyResult::WrongKind;
    }
    if !event.content.is_empty() {
        return VerifyResult::WrongKind;
    }
    if event.from_np {
        return VerifyResult::NpFallbackRejected;
    }

    // Required tags. Match is strict on both key and value.
    let mut has_challenge = false;
    let mut has_origin = false;
    for tag in &event.tags {
        if tag.len() == 2 {
            if tag[0] == "challenge" && tag[1] == expected_challenge {
                has_challenge = true;
            } else if tag[0] == "origin" && tag[1] == expected_origin {
                has_origin = true;
            }
        }
    }
    if !has_challenge {
        return VerifyResult::WrongChallenge;
    }
    if !has_origin {
        return VerifyResult::WrongOrigin;
    }

    // created_at window — absolute difference in either direction.
    let drift = now_ts.abs_diff(event.created_at);
    if drift > AUTH_EVENT_SKEW_SECS {
        return VerifyResult::CreatedAtOutsideSkew;
    }

    let recomputed = canonical_id(
        &event.pubkey,
        event.created_at,
        event.kind,
        &event.tags,
        &event.content,
    );
    if recomputed != event.id {
        return VerifyResult::BadEventId;
    }

    if !schnorr_verify_bip340(&event.pubkey, &event.id, &event.sig) {
        return VerifyResult::BadSignature;
    }

    VerifyResult::Ok
}

/// Verify a handle credential binds to an auth pubkey and hasn't expired.
///
/// The credential is a signed kind-31000 event with an `expires` tag
/// carrying a unix-ts. `auth_pubkey` is the pubkey that signed the outer
/// auth event — the credential must be signed by the same key, otherwise a
/// player could slap a credential they didn't own onto their join.
pub fn verify_credential(
    cred: &SignetCredential,
    auth_pubkey: &[u8; 32],
    now_ts: u32,
) -> VerifyResult {
    if cred.kind != CREDENTIAL_KIND {
        return VerifyResult::WrongKind;
    }
    if &cred.pubkey != auth_pubkey {
        return VerifyResult::CredentialPubkeyMismatch;
    }

    // Expiry — if tag is present and parses as a u32, compare to now.
    // Missing tag is allowed (credentials can be permanent).
    for tag in &cred.tags {
        if tag.len() == 2 && tag[0] == "expires" {
            match tag[1].parse::<u32>() {
                Ok(ts) if ts <= now_ts => return VerifyResult::ExpiredCredential,
                Err(_) => return VerifyResult::ExpiredCredential,
                _ => {}
            }
        }
    }

    let recomputed = canonical_id(
        &cred.pubkey,
        cred.created_at,
        cred.kind,
        &cred.tags,
        &cred.content,
    );
    if recomputed != cred.id {
        return VerifyResult::BadEventId;
    }

    if !schnorr_verify_bip340(&cred.pubkey, &cred.id, &cred.sig) {
        return VerifyResult::BadSignature;
    }

    VerifyResult::Ok
}

/// Extract the `display-name` tag value from a credential, if present.
/// Returns `None` when the tag is missing — callers fall back to
/// `"Player <short-pubkey>"`.
pub fn extract_display_name(cred: &SignetCredential) -> Option<&str> {
    for tag in &cred.tags {
        if tag.len() == 2 && tag[0] == "display-name" {
            return Some(&tag[1]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a well-formed, unsigned auth event for a given challenge/origin.
    /// Tests that care about signature verification need to sign externally
    /// (see `valid_event_accepted`). Callers of this helper who want a
    /// matching id must assign `event.id = canonical_id(...)` afterwards.
    fn make_unsigned(pubkey: [u8; 32], challenge: &str, origin: &str, created_at: u32) -> SignetAuthEvent {
        let tags = vec![
            vec!["challenge".to_string(), challenge.to_string()],
            vec!["origin".to_string(), origin.to_string()],
        ];
        let id = canonical_id(&pubkey, created_at, AUTH_EVENT_KIND, &tags, "");
        SignetAuthEvent {
            pubkey,
            created_at,
            kind: AUTH_EVENT_KIND,
            tags,
            content: String::new(),
            id,
            sig: [0u8; 64],
            from_np: false,
        }
    }

    // --- Cross-impl parity vector (canonical_id) ---

    #[test]
    fn canonical_id_matches_python() {
        // Test vector generated from `tools/sites/game/auth.py:_nostr_event_id`
        // with:
        //   pubkey = "b" * 64
        //   created_at = 1_700_000_000
        //   kind = 21236
        //   tags = [["challenge", "a"*64], ["origin", "https://localhost:8094"]]
        //   content = ""
        //
        // Regeneration command (paste into the website venv):
        //   cd tools/sites/game && .venv/bin/python -c '
        //   import auth
        //   tags = [["challenge", "a"*64], ["origin", "https://localhost:8094"]]
        //   print(auth._nostr_event_id("b"*64, 1_700_000_000, 21236, tags, ""))'
        //
        // If this constant ever disagrees with the Python side, the two impls
        // have diverged — neither may land unless both match.
        const PYTHON_HEX: &str =
            "f0affcce500eead3b7eca8181250a8fd439e4c7b978ab4fd89627c6c9845bd51";

        let pubkey = [0xbbu8; 32];
        let tags = vec![
            vec!["challenge".to_string(), "a".repeat(64)],
            vec!["origin".to_string(), "https://localhost:8094".to_string()],
        ];
        let id = canonical_id(&pubkey, 1_700_000_000, AUTH_EVENT_KIND, &tags, "");
        let id_hex = hex::encode(id);
        assert_eq!(id_hex, PYTHON_HEX,
            "Rust canonical_id diverged from Python _nostr_event_id. \
             Regenerate the constant (see module docs) and reconcile.");
    }

    // --- Structural checks ---

    #[test]
    fn wrong_kind_rejected() {
        let mut ev = make_unsigned([1u8; 32], "c", "o", 100);
        ev.kind = 21237;
        assert_eq!(
            verify_auth_event(&ev, "c", "o", 100),
            VerifyResult::WrongKind
        );
    }

    #[test]
    fn non_empty_content_rejected() {
        let mut ev = make_unsigned([1u8; 32], "c", "o", 100);
        ev.content = "hello".to_string();
        assert_eq!(
            verify_auth_event(&ev, "c", "o", 100),
            VerifyResult::WrongKind
        );
    }

    #[test]
    fn from_np_rejected() {
        let mut ev = make_unsigned([1u8; 32], "c", "o", 100);
        ev.from_np = true;
        assert_eq!(
            verify_auth_event(&ev, "c", "o", 100),
            VerifyResult::NpFallbackRejected
        );
    }

    #[test]
    fn wrong_challenge_rejected() {
        let ev = make_unsigned([1u8; 32], "correct-chal", "o", 100);
        assert_eq!(
            verify_auth_event(&ev, "other-chal", "o", 100),
            VerifyResult::WrongChallenge
        );
    }

    #[test]
    fn wrong_origin_rejected() {
        let ev = make_unsigned([1u8; 32], "c", "correct-origin", 100);
        assert_eq!(
            verify_auth_event(&ev, "c", "other-origin", 100),
            VerifyResult::WrongOrigin
        );
    }

    #[test]
    fn created_at_too_old_rejected() {
        let ev = make_unsigned([1u8; 32], "c", "o", 100);
        let now = 100 + AUTH_EVENT_SKEW_SECS + 1;
        assert_eq!(
            verify_auth_event(&ev, "c", "o", now),
            VerifyResult::CreatedAtOutsideSkew
        );
    }

    #[test]
    fn created_at_too_new_rejected() {
        let ev = make_unsigned([1u8; 32], "c", "o", 1000);
        let now = 1000 - (AUTH_EVENT_SKEW_SECS + 1);
        assert_eq!(
            verify_auth_event(&ev, "c", "o", now),
            VerifyResult::CreatedAtOutsideSkew
        );
    }

    #[test]
    fn tampered_id_rejected() {
        let mut ev = make_unsigned([1u8; 32], "c", "o", 100);
        ev.id[0] ^= 0xFF;
        assert_eq!(
            verify_auth_event(&ev, "c", "o", 100),
            VerifyResult::BadEventId
        );
    }

    #[test]
    fn bad_signature_rejected_after_id_check_passes() {
        // `make_unsigned` gives a correct id but sig=zeros — Schnorr must fail.
        let ev = make_unsigned([1u8; 32], "c", "o", 100);
        assert_eq!(
            verify_auth_event(&ev, "c", "o", 100),
            VerifyResult::BadSignature
        );
    }

    // --- Real BIP-340 signature round-trip ---

    #[test]
    fn valid_event_accepted_when_signed() {
        use secp256k1::{Keypair, Secp256k1};

        let secp = Secp256k1::new();
        // Fixed secret bytes — deterministic run.
        let seckey_bytes = [0x42u8; 32];
        let keypair = Keypair::from_seckey_slice(&secp, &seckey_bytes).unwrap();
        let (xonly, _parity) = keypair.x_only_public_key();
        let mut pubkey = [0u8; 32];
        pubkey.copy_from_slice(&xonly.serialize());

        let mut ev = make_unsigned(pubkey, "challenge-hex", "https://example.com", 500);
        // Sign the canonical id.
        let msg = secp256k1::Message::from_digest_slice(&ev.id).unwrap();
        let sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
        ev.sig.copy_from_slice(sig.as_ref());

        assert_eq!(
            verify_auth_event(&ev, "challenge-hex", "https://example.com", 500),
            VerifyResult::Ok
        );
    }

    #[test]
    fn signature_from_different_key_rejected() {
        use secp256k1::{Keypair, Secp256k1};

        let secp = Secp256k1::new();
        let signer = Keypair::from_seckey_slice(&secp, &[0x11u8; 32]).unwrap();
        let other = Keypair::from_seckey_slice(&secp, &[0x22u8; 32]).unwrap();

        // Claim the `other` pubkey on the event but sign with `signer`.
        let (other_xonly, _) = other.x_only_public_key();
        let mut claimed_pk = [0u8; 32];
        claimed_pk.copy_from_slice(&other_xonly.serialize());

        let mut ev = make_unsigned(claimed_pk, "c", "o", 100);
        let msg = secp256k1::Message::from_digest_slice(&ev.id).unwrap();
        let sig = secp.sign_schnorr_no_aux_rand(&msg, &signer);
        ev.sig.copy_from_slice(sig.as_ref());

        assert_eq!(
            verify_auth_event(&ev, "c", "o", 100),
            VerifyResult::BadSignature
        );
    }

    // --- Credentials ---

    fn make_credential(pubkey: [u8; 32], display_name: &str, expires: Option<u32>) -> SignetCredential {
        let mut tags = vec![vec!["display-name".to_string(), display_name.to_string()]];
        if let Some(e) = expires {
            tags.push(vec!["expires".to_string(), e.to_string()]);
        }
        let id = canonical_id(&pubkey, 100, CREDENTIAL_KIND, &tags, "");
        SignetCredential {
            pubkey,
            created_at: 100,
            kind: CREDENTIAL_KIND,
            tags,
            content: String::new(),
            id,
            sig: [0u8; 64],
        }
    }

    #[test]
    fn credential_wrong_kind_rejected() {
        let mut cred = make_credential([1u8; 32], "Axo", None);
        cred.kind = 1234;
        assert_eq!(
            verify_credential(&cred, &[1u8; 32], 100),
            VerifyResult::WrongKind
        );
    }

    #[test]
    fn credential_pubkey_mismatch_rejected() {
        let cred = make_credential([1u8; 32], "Axo", None);
        assert_eq!(
            verify_credential(&cred, &[2u8; 32], 100),
            VerifyResult::CredentialPubkeyMismatch
        );
    }

    #[test]
    fn credential_expired_rejected() {
        let cred = make_credential([1u8; 32], "Axo", Some(100));
        assert_eq!(
            verify_credential(&cred, &[1u8; 32], 200),
            VerifyResult::ExpiredCredential
        );
    }

    #[test]
    fn credential_unparseable_expires_rejected() {
        let mut cred = make_credential([1u8; 32], "Axo", Some(100));
        // Replace the expires timestamp with a non-integer.
        for tag in &mut cred.tags {
            if tag.len() == 2 && tag[0] == "expires" {
                tag[1] = "forever".to_string();
            }
        }
        // Re-hash after mutation.
        cred.id = canonical_id(&cred.pubkey, cred.created_at, cred.kind, &cred.tags, &cred.content);
        assert_eq!(
            verify_credential(&cred, &[1u8; 32], 200),
            VerifyResult::ExpiredCredential
        );
    }

    #[test]
    fn extract_display_name_present_and_absent() {
        let with_name = make_credential([1u8; 32], "Axolittle", None);
        assert_eq!(extract_display_name(&with_name), Some("Axolittle"));

        let mut no_name = with_name.clone();
        no_name.tags.clear();
        assert_eq!(extract_display_name(&no_name), None);
    }
}

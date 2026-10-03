#![cfg(not(target_arch = "wasm32"))]
//! On-disk server identity: the runtime keypair and the operator's attestation.
//!
//! Files live under the identity directory (`<worlds>/.identity` by default),
//! written owner-only (`0600` on unix):
//!   - `runtime_key.json` — `{"secret_key_hex": "…"}`, the disposable runtime key
//!   - `attestation.json` — the raw operator-signed attestation event (JSON)
//!
//! Loading is best-effort for the attestation: any parse/verify failure (corrupt,
//! wrong server key, bad signature) leaves the server **unpaired** rather than
//! erroring — additive enforcement (the server still boots anonymously).

use std::fs;
use std::path::Path;

use nostr::{Event, JsonUtil, Keys, PublicKey, SecretKey, Timestamp, ToBech32};

use crate::server_identity::attestation::{
    verify, verify_structural, Attestation, AttestationError,
};

/// A loaded server identity: the runtime keypair plus, if paired, the operator's
/// attestation (already structurally verified against the runtime key).
pub struct ServerIdentity {
    runtime: Keys,
    attestation: Option<Attestation>,
}

#[cfg(unix)]
pub(crate) fn write_0600(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
pub(crate) fn write_0600(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    fs::write(path, bytes).map_err(|e| e.to_string())
}

impl ServerIdentity {
    pub fn runtime_pubkey(&self) -> PublicKey {
        self.runtime.public_key()
    }
    pub fn attestation(&self) -> Option<&Attestation> {
        self.attestation.as_ref()
    }

    /// Whether the server currently presents a verified operator identity:
    /// an attestation that fully verifies (signature, server-key match, and the
    /// `[valid_from, valid_until]` window) at `now`.
    pub fn is_verified(&self, now: Timestamp) -> bool {
        self.attestation
            .as_ref()
            .map(|a| verify(&a.event, &self.runtime.public_key(), now).is_ok())
            .unwrap_or(false)
    }

    /// The operator npub (bech32) from the attestation, if paired.
    pub fn operator_npub(&self) -> Option<String> {
        self.attestation
            .as_ref()
            .and_then(|a| a.operator.to_bech32().ok())
    }

    /// Persist and adopt an operator-signed attestation. Rejected (not written)
    /// if it does not structurally verify against this server's runtime key.
    pub fn store_attestation(&mut self, dir: &Path, event: Event) -> Result<(), String> {
        let att = verify_structural(&event, &self.runtime.public_key())
            .map_err(|e| format!("attestation rejected: {e:?}"))?;
        // A `role=player` event is a persona's delegation to a *player's*
        // runtime key (see `runtime_identity`), never a server's. Accepting it
        // here would let a player attestation stand in for a server one.
        if att.role.is_some() {
            return Err(format!(
                "attestation rejected: {:?}",
                AttestationError::Malformed("role must be server, not player".to_string())
            ));
        }
        write_0600(&dir.join("attestation.json"), event.as_json().as_bytes())?;
        self.attestation = Some(att);
        Ok(())
    }

    /// BIP-340 Schnorr sign a join challenge with the runtime key (local, no
    /// round-trip). The client verifies the returned signature against the
    /// runtime pubkey, and the runtime pubkey against the attestation (Track 3).
    #[allow(dead_code)] // consumed by Track 3 (join-auth); covered by unit tests
    pub fn sign_challenge(&self, nonce: &[u8], origin: &str) -> [u8; 64] {
        use secp256k1::{Keypair, Message, Secp256k1};
        let secp = Secp256k1::new();
        // Bridge the nostr secret key into a secp256k1 keypair via its hex form
        // (the conversion verify.rs uses for the inverse direction).
        let sk_bytes = hex::decode(self.runtime.secret_key().to_secret_hex())
            .expect("runtime secret key is valid hex");
        let kp = Keypair::from_seckey_slice(&secp, &sk_bytes)
            .expect("runtime secret key is a valid secp256k1 key");
        let msg = Message::from_digest_slice(&challenge_msg(nonce, origin))
            .expect("challenge_msg returns a 32-byte digest");
        let sig = secp.sign_schnorr_no_aux_rand(&msg, &kp);
        let mut out = [0u8; 64];
        out.copy_from_slice(sig.as_ref());
        out
    }

    /// Sign a nostr event with the runtime key (Track 6 — authoritative claims).
    #[allow(dead_code)] // consumed by Track 6 (signed claims); covered by unit tests
    pub fn sign_event(&self, unsigned: nostr::UnsignedEvent) -> Result<Event, String> {
        unsigned.sign_with_keys(&self.runtime).map_err(|e| e.to_string())
    }
}

/// Domain-separated 32-byte message a server signs to answer a join challenge:
/// `SHA256("axenstax-server-challenge:v1" || origin || 0x00 || nonce)`.
pub(crate) fn challenge_msg(nonce: &[u8], origin: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"axenstax-server-challenge:v1");
    h.update(origin.as_bytes());
    h.update([0x00]);
    h.update(nonce);
    h.finalize().into()
}

/// Generate a fresh runtime keypair, persist it (`0600`), and return an unpaired
/// identity. Overwrites any existing runtime key in `dir`.
pub fn generate_runtime(dir: &Path) -> Result<ServerIdentity, String> {
    let keys = Keys::generate();
    let json = format!(
        "{{\"secret_key_hex\":\"{}\"}}",
        keys.secret_key().to_secret_hex()
    );
    write_0600(&dir.join("runtime_key.json"), json.as_bytes())?;
    Ok(ServerIdentity {
        runtime: keys,
        attestation: None,
    })
}

/// Load the identity from `dir`. `Ok(None)` when no runtime key exists (the
/// server has never been provisioned). A present-but-unverifiable attestation is
/// silently dropped (server boots anonymous).
pub fn load(dir: &Path) -> Result<Option<ServerIdentity>, String> {
    let raw = match fs::read_to_string(dir.join("runtime_key.json")) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let sk_hex = v
        .get("secret_key_hex")
        .and_then(|x| x.as_str())
        .ok_or("runtime_key.json missing secret_key_hex")?;
    let sk = SecretKey::from_hex(sk_hex).map_err(|e| e.to_string())?;
    let runtime = Keys::new(sk);

    // Best-effort: any attestation parse/verify failure ⇒ unpaired, never an error.
    // A `role=player` event is likewise dropped here (not just refused at write
    // time in `store_attestation`) — a hand-edited or foreign-written
    // attestation.json must not let a player delegation act as a server one.
    let attestation = fs::read_to_string(dir.join("attestation.json"))
        .ok()
        .and_then(|s| Event::from_json(&s).ok())
        .and_then(|ev| verify_structural(&ev, &runtime.public_key()).ok())
        .filter(|att| att.role.is_none());

    Ok(Some(ServerIdentity {
        runtime,
        attestation,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_identity::attestation::mint_attestation;
    use nostr::Timestamp;

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("axe_srvid_{}_{}", tag, std::process::id()))
    }

    #[test]
    fn generate_then_load_round_trips() {
        let dir = tmp("rt");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        let pk = id.runtime_pubkey();
        let loaded = load(&dir).unwrap().unwrap();
        assert_eq!(loaded.runtime_pubkey(), pk);
        assert!(loaded.attestation().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_runtime_key_is_none() {
        let dir = tmp("none");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load(&dir).unwrap().is_none());
    }

    #[tokio::test]
    async fn store_and_reload_attestation() {
        let dir = tmp("att");
        let _ = std::fs::remove_dir_all(&dir);
        let mut id = generate_runtime(&dir).unwrap();
        let op = Keys::generate();
        let ev = mint_attestation(
            &op,
            &id.runtime_pubkey(),
            Timestamp::from(0),
            Timestamp::from(u64::MAX >> 1),
            &[27420],
            "W",
            None,
        )
        .await
        .unwrap();
        id.store_attestation(&dir, ev).unwrap();
        let loaded = load(&dir).unwrap().unwrap();
        assert!(loaded.attestation().is_some());
        assert_eq!(loaded.attestation().unwrap().operator, op.public_key());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn store_attestation_refuses_a_player_role_event() {
        let dir = tmp("player_store");
        let _ = std::fs::remove_dir_all(&dir);
        let mut id = generate_runtime(&dir).unwrap();
        let op = Keys::generate();
        // A role=player event, minted exactly the way runtime_identity mints one
        // for a player's install — it must never be adoptable as a *server*
        // delegation.
        let ev = crate::runtime_identity::mint_player_attestation(
            &op,
            &id.runtime_pubkey(),
            Timestamp::from(0),
            90,
        )
        .await
        .unwrap();
        assert!(
            id.store_attestation(&dir, ev).is_err(),
            "a player-role attestation must be refused by the server store"
        );
        assert!(id.attestation().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn load_drops_a_player_role_attestation() {
        let dir = tmp("player_load");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        let op = Keys::generate();
        let ev = crate::runtime_identity::mint_player_attestation(
            &op,
            &id.runtime_pubkey(),
            Timestamp::from(0),
            90,
        )
        .await
        .unwrap();
        // Written directly (bypassing store_attestation, which now refuses it) to
        // prove `load` itself also never adopts a player-role event as a server
        // delegation — e.g. a hand-edited or foreign-written attestation.json.
        std::fs::write(dir.join("attestation.json"), ev.as_json()).unwrap();
        let loaded = load(&dir).unwrap().unwrap();
        assert!(
            loaded.attestation().is_none(),
            "a player-role attestation must not load as a server one"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn attestation_for_other_key_is_dropped_on_load() {
        let dir = tmp("mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        let _id = generate_runtime(&dir).unwrap();
        // Write an attestation that targets a DIFFERENT server key.
        let op = Keys::generate();
        let other = Keys::generate().public_key();
        let ev = mint_attestation(
            &op,
            &other,
            Timestamp::from(0),
            Timestamp::from(u64::MAX >> 1),
            &[27420],
            "W",
            None,
        )
        .await
        .unwrap();
        std::fs::write(dir.join("attestation.json"), ev.as_json()).unwrap();
        let loaded = load(&dir).unwrap().unwrap();
        assert!(
            loaded.attestation().is_none(),
            "mismatched attestation must be ignored"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn is_verified_respects_window_and_exposes_npub() {
        let dir = tmp("ver");
        let _ = std::fs::remove_dir_all(&dir);
        let mut id = generate_runtime(&dir).unwrap();
        let op = Keys::generate();
        let ev = mint_attestation(
            &op,
            &id.runtime_pubkey(),
            Timestamp::from(100),
            Timestamp::from(200),
            &[27420],
            "W",
            None,
        )
        .await
        .unwrap();
        id.store_attestation(&dir, ev).unwrap();
        assert!(!id.is_verified(Timestamp::from(50)));
        assert!(id.is_verified(Timestamp::from(150)));
        assert!(!id.is_verified(Timestamp::from(250)));
        assert!(id.operator_npub().unwrap().starts_with("npub1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpaired_is_not_verified_and_has_no_npub() {
        let dir = tmp("unpaired");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        assert!(!id.is_verified(Timestamp::from(1)));
        assert!(id.operator_npub().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn challenge_signature_verifies_under_runtime_key() {
        use crate::signet::verify::schnorr_verify_bip340;
        let dir = tmp("sig");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        let sig = id.sign_challenge(b"nonce-bytes", "wss://play.example.com/ws");
        let msg = challenge_msg(b"nonce-bytes", "wss://play.example.com/ws");
        let xonly = id.runtime_pubkey().to_bytes();
        assert!(
            schnorr_verify_bip340(&xonly, &msg, &sig),
            "challenge signature must verify under the runtime key"
        );
        // A different origin yields a different message → the same sig must fail.
        let other = challenge_msg(b"nonce-bytes", "wss://evil/ws");
        assert!(!schnorr_verify_bip340(&xonly, &other, &sig));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sign_event_is_authored_by_runtime_key() {
        use nostr::{EventBuilder, Kind};
        let dir = tmp("signev");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        let unsigned = EventBuilder::new(Kind::Custom(27421), "claim").build(id.runtime_pubkey());
        let ev = id.sign_event(unsigned).unwrap();
        assert_eq!(ev.pubkey, id.runtime_pubkey());
        assert!(ev.verify().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_attestation_degrades_to_unpaired() {
        let dir = tmp("corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        let _id = generate_runtime(&dir).unwrap();
        std::fs::write(dir.join("attestation.json"), b"not json").unwrap();
        let loaded = load(&dir).unwrap().unwrap();
        assert!(loaded.attestation().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_key_is_locked_down() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("perms");
        let _ = std::fs::remove_dir_all(&dir);
        generate_runtime(&dir).unwrap();
        let mode = std::fs::metadata(dir.join("runtime_key.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "runtime key must be owner-only");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

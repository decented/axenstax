//! The player's per-install **runtime key** and the persona attestation over it.
//!
//! The persona secret never touches this machine (it lives in a bunker). So the
//! game mints a throwaway secp256k1 key per install and asks the persona to
//! sign, exactly once, a kind-30420 attestation naming it — the same delegation
//! pattern `--pair-server` uses for a dedicated server's runtime key
//! (`server_identity::{attestation, store, pairing}`), with the tag
//! `role=player` added so the two can never be confused.
//!
//! Signalling events are signed by this runtime key; the QUIC join is still
//! signed by the persona through the bunker. The attestation travels **inside**
//! the NIP-44 ciphertext of an offer/answer and is never published, so no relay
//! can correlate a runtime key to a persona (CLAUDE.md red line 3).
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md` §2.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, NostrSigner, PublicKey, Tag, Timestamp};

use crate::server_identity::attestation::{
    verify, AttestationError, ATTESTATION_KIND, ATTESTATION_VERSION,
};

/// The `role` tag value that marks a player's runtime key.
pub const PLAYER_ROLE: &str = "player";
/// The signalling kinds a player's runtime key is authorised to sign.
pub const PLAYER_ATTESTATION_KINDS: [u16; 2] = [20900, 20901];
/// How long a player attestation is minted for. Matches the server pairing
/// default (`server_identity::pairing::DEFAULT_DELEGATION_DAYS`); re-minting is
/// one phone tap.
pub const PLAYER_DELEGATION_DAYS: u64 = 90;

/// The runtime secret. Same `profile/` tree as `signet_session.json` — kept out
/// of `worlds/` so it never shows as a world card.
pub fn runtime_key_path() -> PathBuf {
    crate::data_dir::profile_dir().join("runtime_key.json")
}

/// The stored kind-30420 attestation event (raw JSON).
pub fn attestation_path() -> PathBuf {
    crate::data_dir::profile_dir().join("runtime_attestation.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredKey {
    secret_hex: String,
}

/// Load the runtime key, or mint and persist one (0600) on first use.
///
/// A corrupt file re-mints, which costs exactly one bunker tap to re-attest —
/// an acceptable degradation, because the key addresses nothing durable on
/// its own.
pub fn load_or_mint_runtime(path: &Path) -> Result<Keys, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            // The file is there and readable. A parse/hex failure still re-mints
            // (the degradation the doc above describes) — but a successful read
            // is not "absent", so it falls through to mint only on malformed
            // content, never silently on some other read outcome.
            if let Ok(stored) = serde_json::from_slice::<StoredKey>(&bytes)
                && let Ok(sk) = nostr::SecretKey::from_hex(&stored.secret_hex)
            {
                return Ok(Keys::new(sk));
            }
        }
        // No file yet ⇒ genuinely first use. Mint below.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // Anything else (permission denied, I/O error, …) must propagate: only
        // `NotFound` means "absent", so an EACCES never gets silently papered
        // over by minting a fresh key — which would orphan the existing
        // attestation (signed over a runtime pubkey this new key can't match).
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    }
    let keys = Keys::generate();
    let stored = StoredKey {
        secret_hex: keys.secret_key().to_secret_hex(),
    };
    let json = serde_json::to_vec(&stored).map_err(|e| format!("serialise runtime key: {e}"))?;
    write_0600(path, &json)?;
    Ok(keys)
}

/// Write owner-only, creating the parent directory. Mirrors
/// `server_identity::store::write_0600` (which is `pub(crate)` to that module).
///
/// The file is **created** 0600 rather than written and then chmod'd: the
/// write-then-chmod shape leaves a window in which the contents are on disk at
/// the umask's permissions, and anything private written that way is readable
/// by every account on the machine for as long as the window lasts. Shared with
/// `contacts::save_mirror`, which is the same kind of secret (who a child
/// knows).
pub(crate) fn write_0600(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        f.write_all(bytes).map_err(|e| format!("write: {e}"))?;
        // `.mode(0o600)` only governs the permissions of a newly *created* file;
        // an existing file (left wide-open by an older build, say) keeps its old
        // mode unless we chmod it explicitly here too.
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("chmod {}: {e}", path.display()))
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes).map_err(|e| format!("write: {e}"))
}

/// Read the stored attestation. Any absence or corruption reads as `None` — the
/// player is simply not yet attested, which the UI explains and a bunker tap
/// fixes.
pub fn load_attestation(path: &Path) -> Option<Event> {
    let s = std::fs::read_to_string(path).ok()?;
    Event::from_json(&s).ok()
}

/// Persist an attestation event verbatim (0600 — it names the persona, which is
/// not secret, but it sits beside the key and there is no reason to widen it).
pub fn store_attestation(path: &Path, ev: &Event) -> Result<(), String> {
    write_0600(path, ev.as_json().as_bytes())
}

/// Ask the persona (a bunker in production, a local `Keys` in tests — both
/// implement `NostrSigner`) to attest `runtime`. Producing the signature is the
/// only online step: for a bunker it is one tap on the player's phone.
pub async fn mint_player_attestation<S: NostrSigner>(
    persona: &S,
    runtime: &PublicKey,
    now: Timestamp,
    validity_days: u64,
) -> Result<Event, String> {
    let until = Timestamp::from(now.as_secs().saturating_add(validity_days.saturating_mul(86_400)));
    let mut tags = vec![
        Tag::parse(["d", &runtime.to_hex()]).map_err(|e| e.to_string())?,
        Tag::parse(["p", &runtime.to_hex()]).map_err(|e| e.to_string())?,
        Tag::parse(["valid_from", &now.as_secs().to_string()]).map_err(|e| e.to_string())?,
        Tag::parse(["valid_until", &until.as_secs().to_string()]).map_err(|e| e.to_string())?,
        Tag::parse(["v", ATTESTATION_VERSION]).map_err(|e| e.to_string())?,
        // A player attestation names no server, so `name` is empty and there is
        // no `host` tag. `verify_structural` tolerates both.
        Tag::parse(["name", ""]).map_err(|e| e.to_string())?,
        Tag::parse(["role", PLAYER_ROLE]).map_err(|e| e.to_string())?,
    ];
    for k in PLAYER_ATTESTATION_KINDS {
        tags.push(Tag::parse(["k", &k.to_string()]).map_err(|e| e.to_string())?);
    }
    EventBuilder::new(Kind::Custom(ATTESTATION_KIND), "")
        .tags(tags)
        .sign(persona)
        .await
        .map_err(|e| e.to_string())
}

/// Verify a player attestation and return the attesting persona.
///
/// Everything `server_identity::attestation::verify` checks (signature, kind,
/// version, `d` == `expected_runtime`, validity window) plus the `role=player`
/// requirement — which is what stops a server's delegation being replayed as a
/// player's, and vice versa.
pub fn verify_player_attestation(
    ev: &Event,
    expected_runtime: &PublicKey,
    now: Timestamp,
) -> Result<PublicKey, AttestationError> {
    let att = verify(ev, expected_runtime, now)?;
    if att.role.as_deref() != Some(PLAYER_ROLE) {
        return Err(AttestationError::Malformed("role is not player".to_string()));
    }
    Ok(att.operator)
}

/// This install's online-play identity: the runtime key plus, once the player
/// has tapped their phone, the persona's attestation over it.
pub struct RuntimeIdentity {
    keys: Keys,
    attestation: Option<Event>,
}

impl RuntimeIdentity {
    /// Load (minting the runtime key on first use). `Ok(None)` is never
    /// returned today — it is the shape a future "no profile dir" case would
    /// take; an unattested identity is `Some` with `attestation() == None`.
    pub fn load() -> Result<Option<RuntimeIdentity>, String> {
        let keys = load_or_mint_runtime(&runtime_key_path())?;
        let attestation = load_attestation(&attestation_path());
        Ok(Some(RuntimeIdentity { keys, attestation }))
    }

    /// Build an identity from parts. Production goes through [`load`]; this is
    /// how `online_host`/`online_join` tests stand one up without touching the
    /// filesystem or a bunker.
    #[cfg(test)]
    pub fn from_parts(keys: Keys, attestation: Option<Event>) -> RuntimeIdentity {
        RuntimeIdentity { keys, attestation }
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    pub fn runtime_pubkey(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub fn attestation(&self) -> Option<&Event> {
        self.attestation.as_ref()
    }

    /// The persona this runtime key is currently attested by, if the
    /// attestation is present and valid at `now`.
    pub fn persona(&self, now: Timestamp) -> Option<PublicKey> {
        let ev = self.attestation.as_ref()?;
        verify_player_attestation(ev, &self.keys.public_key(), now).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("axe_rtid_{}_{}", tag, std::process::id()))
    }

    #[test]
    fn runtime_key_mints_once_and_is_stable() {
        let dir = tmp("mint");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runtime_key.json");
        let a = load_or_mint_runtime(&path).unwrap();
        let b = load_or_mint_runtime(&path).unwrap();
        assert_eq!(a.public_key(), b.public_key(), "same key on reload");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_runtime_key_file_remints_a_valid_key() {
        let dir = tmp("key_corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runtime_key.json");
        std::fs::write(&path, b"{not json").unwrap();
        let minted = load_or_mint_runtime(&path).unwrap();
        // Reload proves the mint actually persisted, not just returned in memory.
        let reloaded = load_or_mint_runtime(&path).unwrap();
        assert_eq!(minted.public_key(), reloaded.public_key());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_read_error_other_than_not_found_propagates_instead_of_minting() {
        let dir = tmp("key_read_err");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Make the path itself a directory: `fs::read` on it fails (IsADirectory
        // on unix), and critically that error is not `NotFound` — it must
        // propagate rather than being treated as "no key yet" and minted over.
        let path = dir.join("runtime_key.json");
        std::fs::create_dir_all(&path).unwrap();
        assert!(
            load_or_mint_runtime(&path).is_err(),
            "a non-NotFound read error must not silently mint a fresh key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("perm");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runtime_key.json");
        load_or_mint_runtime(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the runtime secret is sensitive: 0600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn mint_round_trips_through_verify_and_yields_the_persona() {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            PLAYER_DELEGATION_DAYS,
        )
        .await
        .unwrap();
        let got = verify_player_attestation(
            &ev,
            &runtime.public_key(),
            nostr::Timestamp::from(1_500),
        )
        .unwrap();
        assert_eq!(got, persona.public_key());
    }

    #[tokio::test]
    async fn a_server_attestation_is_refused_as_a_player_one() {
        // No `role` tag ⇒ a server attestation. It must not admit a player.
        use crate::server_identity::attestation::mint_attestation;
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let ev = mint_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(0),
            nostr::Timestamp::from(9_999),
            &[27420],
            "",
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            verify_player_attestation(&ev, &runtime.public_key(), nostr::Timestamp::from(10)),
            Err(AttestationError::Malformed("role is not player".to_string()))
        );
    }

    #[tokio::test]
    async fn an_attestation_for_another_runtime_key_is_refused() {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let other = Keys::generate();
        let ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            PLAYER_DELEGATION_DAYS,
        )
        .await
        .unwrap();
        assert_eq!(
            verify_player_attestation(&ev, &other.public_key(), nostr::Timestamp::from(1_500)),
            Err(AttestationError::WrongServerKey)
        );
    }

    #[tokio::test]
    async fn a_tampered_attestation_is_refused() {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let mut ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            PLAYER_DELEGATION_DAYS,
        )
        .await
        .unwrap();
        ev.content = "tampered".to_string();
        assert_eq!(
            verify_player_attestation(&ev, &runtime.public_key(), nostr::Timestamp::from(1_500)),
            Err(AttestationError::BadSignature)
        );
    }

    #[tokio::test]
    async fn an_expired_attestation_is_refused() {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            1, // one day
        )
        .await
        .unwrap();
        assert_eq!(
            verify_player_attestation(
                &ev,
                &runtime.public_key(),
                nostr::Timestamp::from(1_000 + 86_400 + 1),
            ),
            Err(AttestationError::Expired)
        );
    }

    #[tokio::test]
    async fn store_then_load_round_trips_the_event() {
        let dir = tmp("store");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runtime_attestation.json");
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            PLAYER_DELEGATION_DAYS,
        )
        .await
        .unwrap();
        store_attestation(&path, &ev).unwrap();
        assert_eq!(load_attestation(&path).unwrap().id, ev.id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_attestation_file_loads_as_absent() {
        let dir = tmp("corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runtime_attestation.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load_attestation(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

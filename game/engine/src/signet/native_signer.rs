//! Native Signet sign-in — the engine-side wiring of the vendored
//! `signet-nip46-client` (Bucket 3, **login half**).
//!
//! This is the native counterpart of the WASM `auth.js` signing path (Spec 13).
//! It owns a generic NIP-46 bunker signer (pair / persist / restore + the
//! `NostrSigner` contract) plus an **offline-first identity** so a native build
//! knows *who you are* with zero network, and only reaches out for *fresh
//! signatures*.
//!
//! ## Binding contract
//! `docs/foundations/2026-06-10-offline-first-login-and-stash-sequencing.md`:
//!   1. **Login is additive, never a gate.** Guest is the default identity.
//!      Nothing in this module is on the single-player critical path; a dead
//!      connection never blocks play.
//!   2. **Identity persists offline; signatures don't.** `load_identity` and
//!      `restore_signer` are synchronous and offline — they only read local
//!      files and re-parse stored material. Pairing and producing a signature
//!      (`pair`, `user_public_key`, `build_join_auth_event`) are the online
//!      operations, allowed to fail gracefully.
//!
//! ## Boundaries (NOT done here — see the goal doc)
//!   - The **live bunker pairing handshake** needs the owner's phone. The
//!     harness is `tools/native-build-spike/src/bin/bunker_pair.rs`. Everything
//!     up to the relay handshake is unit-tested against a local-key signer.
//!   - **Stash** (`stash-rs` upload) is deferred + decoupled — not wired here.
//!
//! NATIVE-ONLY: the whole module is `#[cfg(not(target_arch = "wasm32"))]` at its
//! `mod` declaration in `signet/mod.rs`; it pulls `nostr` / `nostr-connect`,
//! which never touch the wasm32 bundle.

// This module is a deliberate sign-in **API surface** (same posture as the
// forward-compat re-exports in `signet/mod.rs`). Its lifecycle entry points
// (`pair`, `complete_sign_in`, `sign_out`, `restore_signer`, …) are consumed by
// the native sign-in UX + the Phase-4 authenticated-join path — both the
// owner/device boundary this goal stops at — and its lower-level helpers are
// exercised by the unit tests below. Allow `dead_code` so the non-test build
// (where only `init_identity` + `current_owner_pubkey` are wired today) stays
// warning-clean without dropping the ready API.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use signet_nip46_client::{BunkerSession, PersistedSession, SessionOptions};

use super::wire::SignetAuthEventWire;
use super::{SignetCredentialWire, AUTH_EVENT_KIND};

// ─── Profile-dir paths ───────────────────────────────────────────────────────
// Same `profile/` tree `wardrobe_store` uses — kept OUT of `worlds/` so it never
// shows as a world card. Single-user on native (no per-player identity), so one
// file each suffices.

/// Persisted NIP-46 session (the throwaway app key + bunker URI — **no persona
/// key**, but the app key is still sensitive, so we lock the file down).
pub fn session_path() -> PathBuf {
    crate::data_dir::profile_dir().join("signet_session.json")
}

/// Locally cached persona identity (just the pubkey) so we know who you are with
/// zero network on every later launch.
pub fn identity_path() -> PathBuf {
    crate::data_dir::profile_dir().join("signet_identity.json")
}

// ─── Identity model ──────────────────────────────────────────────────────────

/// Who the native player is right now. **Guest is the default** and unlocks the
/// whole game with no network; signing in *upgrades* it (cloud save, multiplayer
/// auth, attribution). Never used as a gate.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum NativeIdentity {
    /// No sign-in. The default identity; the entire single-player game is here.
    #[default]
    Guest,
    /// Paired with a bunker at least once. The pubkey is cached locally, so this
    /// is resolvable with zero network on later launches.
    SignedIn { pubkey_hex: String },
}

impl NativeIdentity {
    pub fn is_signed_in(&self) -> bool {
        matches!(self, NativeIdentity::SignedIn { .. })
    }

    /// The persona pubkey (lowercase 64-hex) when signed in; `None` for a guest.
    pub fn pubkey_hex(&self) -> Option<&str> {
        match self {
            NativeIdentity::SignedIn { pubkey_hex } => Some(pubkey_hex),
            NativeIdentity::Guest => None,
        }
    }

    /// User-facing npub (NIP-19 bech32) — never hex ([[feedback_npub_only_display]]).
    /// Real bech32 on native via `nostr` (the engine `npub` module only has the
    /// JS path). Fails closed: `None` if the key doesn't parse or won't
    /// bech32-encode, rather than ever falling back to showing the raw hex.
    pub fn npub(&self) -> Option<String> {
        let hex = self.pubkey_hex()?;
        PublicKey::from_hex(hex).ok().and_then(|pk| pk.to_bech32().ok())
    }
}

/// On-disk shape of the cached identity. Versioned-free (single field) JSON —
/// trivially forward-compatible; an unknown/corrupt file degrades to Guest.
#[derive(Serialize, Deserialize)]
struct CachedIdentity {
    pubkey_hex: String,
}

/// A 64-char lowercase-hex pubkey is the only thing we treat as a real identity;
/// anything else (truncated write, hand-edit) degrades to Guest rather than
/// surfacing a broken persona.
fn is_valid_pubkey_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

// ─── Offline identity load / cache (NO network) ──────────────────────────────

/// Read the cached identity from `path`. **Pure + offline** — a missing or
/// corrupt file yields `Guest`, never an error. This is the headline offline
/// property: who-you-are is known with zero network.
pub fn load_identity_from(path: &Path) -> NativeIdentity {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<CachedIdentity>(&bytes) {
            Ok(c) if is_valid_pubkey_hex(&c.pubkey_hex) => {
                NativeIdentity::SignedIn { pubkey_hex: c.pubkey_hex }
            }
            // Present but unreadable/invalid: don't surface a broken persona.
            _ => NativeIdentity::Guest,
        },
        // No cache (never signed in) — the normal default.
        Err(_) => NativeIdentity::Guest,
    }
}

/// Cache the resolved persona pubkey so later launches are offline. Best-effort.
pub fn cache_identity_to(path: &Path, pubkey_hex: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir profile: {e}"))?;
    }
    let json = serde_json::to_vec(&CachedIdentity { pubkey_hex: pubkey_hex.to_string() })
        .map_err(|e| format!("serialise identity: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write identity: {e}"))
}

/// Drop the cached identity (sign-out). Missing file is success.
pub fn clear_identity_at(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove identity: {e}")),
    }
}

/// Load the cached identity from the default profile path (offline).
pub fn load_identity() -> NativeIdentity {
    load_identity_from(&identity_path())
}

// ─── Persisted session store (sensitive — restrictive perms) ─────────────────

/// Write the persisted session to `path` with owner-only perms (`0600` on unix).
/// The `app_secret_key` it holds is the NIP-46 *client* identity — not the
/// persona key, but still secret. Never logged.
pub fn save_session_to(path: &Path, persisted: &PersistedSession) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir profile: {e}"))?;
    }
    let json =
        serde_json::to_vec(persisted).map_err(|e| format!("serialise session: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write session: {e}"))?;
    if let Err(e) = restrict_perms(path) {
        // Non-fatal: the file is written; we just couldn't tighten perms.
        log::warn!("could not restrict perms on session file: {e}");
    }
    Ok(())
}

/// Read the persisted session from `path`. `Ok(None)` when there is none.
pub fn load_session_from(path: &Path) -> Result<Option<PersistedSession>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<PersistedSession>(&bytes)
            .map(Some)
            .map_err(|e| format!("parse session: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read session: {e}")),
    }
}

/// Remove the persisted session (sign-out). Missing file is success.
pub fn clear_session_at(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove session: {e}")),
    }
}

#[cfg(unix)]
fn restrict_perms(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
}
#[cfg(not(unix))]
fn restrict_perms(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

// ─── Signer reconstruction (offline, non-blocking) ───────────────────────────

/// Rebuild a signer from the persisted session **without any network**.
/// `BunkerSession::restore` only re-parses the stored app key + bunker URI; the
/// relay handshake is lazy (first signature). Returns `Ok(None)` when there is
/// no saved session (= guest). A later signature may still fail if the bunker is
/// unreachable — that is expected and is never on the local-play path.
pub fn restore_signer_from(
    path: &Path,
    opts: SessionOptions,
) -> Result<Option<BunkerSession>, String> {
    match load_session_from(path)? {
        Some(persisted) => BunkerSession::restore(persisted, opts)
            .map(Some)
            .map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Default-path convenience over [`restore_signer_from`].
pub fn restore_signer() -> Result<Option<BunkerSession>, String> {
    restore_signer_from(&session_path(), SessionOptions::default())
}

// ─── Current-owner pubkey (Seam B input, cross-thread) ───────────────────────
// Process-global so the save path (any thread) can stamp the owning persona.
// Set once at startup by `init_identity`; updated on sign-in / sign-out.

static NATIVE_OWNER_PUBKEY: RwLock<Option<String>> = RwLock::new(None);

/// The signed-in persona pubkey (hex), or `None` for a guest. Pure read.
pub fn current_owner_pubkey() -> Option<String> {
    NATIVE_OWNER_PUBKEY.read().ok().and_then(|g| g.clone())
}

/// Set/clear the current owner (called on identity changes).
pub fn set_owner_pubkey(pubkey: Option<String>) {
    if let Ok(mut g) = NATIVE_OWNER_PUBKEY.write() {
        *g = pubkey;
    }
}

/// One-time startup hook: load the cached identity (offline) and publish the
/// owner pubkey for Seam B. Returns the identity for the menu to display.
/// **Non-blocking, offline-tolerant** — pure file reads, no network.
pub fn init_identity() -> NativeIdentity {
    let identity = load_identity();
    set_owner_pubkey(identity.pubkey_hex().map(str::to_string));
    identity
}

// ─── Sign-in (online — pairing handshake is the owner/device boundary) ───────

/// Begin a sign-in: parse the `bunker://` URI and construct the session
/// (offline). The first signing/`user_public_key` call performs the live relay
/// handshake + the phone approval — the owner/device boundary.
pub fn pair(uri: &str) -> Result<BunkerSession, JoinAuthError> {
    BunkerSession::pair(uri, SessionOptions::default()).map_err(|e| JoinAuthError::Other(e.to_string()))
}

/// Finish a sign-in: persist the session for silent reconnection, cache the
/// persona identity for offline launches, and publish the owner for Seam B.
///
/// The two **online** steps the caller runs first are `session.persist().await`
/// (→ `PersistedSession`) and `session.user_public_key().await` (→ `pubkey_hex`);
/// everything here is synchronous + offline + unit-testable. Returns the new
/// signed-in identity.
pub fn complete_sign_in(
    persisted: &PersistedSession,
    pubkey_hex: &str,
) -> Result<NativeIdentity, String> {
    save_session_to(&session_path(), persisted)?;
    cache_identity_to(&identity_path(), pubkey_hex)?;
    set_owner_pubkey(Some(pubkey_hex.to_string()));
    Ok(NativeIdentity::SignedIn { pubkey_hex: pubkey_hex.to_string() })
}

/// Sign out: clear the persisted session + cached identity + owner. Best-effort
/// (a failure to delete one file still clears the rest and the in-memory owner).
pub fn sign_out() -> Result<(), String> {
    set_owner_pubkey(None);
    let a = clear_session_at(&session_path());
    let b = clear_identity_at(&identity_path());
    a.and(b)
}

// ─── Join auth-event builder (Spec 1 client side) ────────────────────────────

/// Typed sign-in/signing failure, surfaced to the join UI as a state rather than
/// a raw string. NIP-46 transports report failures as messages, so timeout vs
/// denied is classified heuristically (best-effort) from the error text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinAuthError {
    /// No paired session — the player is a guest. The join UI offers guest-join
    /// or a sign-in prompt; this is **not** an error shown as a failure.
    NotPaired,
    /// The bunker didn't answer in time (offline, locked, or slow approval).
    Timeout,
    /// The bunker (or the human at the phone) declined the signing request.
    Denied,
    /// Anything else (URI/key/transport). Carries a short message for logs.
    Other(String),
}

impl std::fmt::Display for JoinAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinAuthError::NotPaired => write!(f, "not signed in"),
            JoinAuthError::Timeout => write!(f, "signer timed out (bunker offline or locked)"),
            JoinAuthError::Denied => write!(f, "signing request was declined"),
            JoinAuthError::Other(m) => write!(f, "signer error: {m}"),
        }
    }
}

/// Map the vendored crate's `Error` into a typed [`JoinAuthError`] for the UI.
/// Transport/approval failures (`Connect`/`Signer`) are classified heuristically
/// into Timeout/Denied; structural failures (`Uri`/`Key`) are `Other`.
pub fn map_signer_error(e: &signet_nip46_client::Error) -> JoinAuthError {
    use signet_nip46_client::Error;
    match e {
        Error::Uri(m) | Error::Key(m) => JoinAuthError::Other(m.clone()),
        Error::Connect(m) | Error::Signer(m) => classify_signer_error(m),
    }
}

/// Classify a signer error string into a typed state. Heuristic — NIP-46 errors
/// arrive as messages, so this matches on common substrings.
fn classify_signer_error(msg: &str) -> JoinAuthError {
    let m = msg.to_ascii_lowercase();
    if m.contains("timeout") || m.contains("timed out") || m.contains("deadline") {
        JoinAuthError::Timeout
    } else if m.contains("denied")
        || m.contains("rejected")
        || m.contains("declined")
        || m.contains("refused")
    {
        JoinAuthError::Denied
    } else {
        JoinAuthError::Other(msg.to_string())
    }
}

/// Build a server-verifiable kind-21236 Signet auth event over the server's
/// challenge nonce, signed by `signer`, and convert it to the wire form that
/// travels inside `JoinRequestPacket`.
///
/// Generic over any `NostrSigner` so it is unit-testable with a local `Keys`
/// signer; in production the caller passes a `BunkerSession`. Producing the
/// signature is the **online** step — for a bunker it round-trips to the phone.
///
/// The output is built to pass the engine's own `verify::verify_auth_event`
/// (same NIP-01 canonical id + BIP-340 verify the server runs) — proven by the
/// `builds_a_server_verifiable_auth_event` test below, entirely offline with a
/// local key.
pub async fn build_join_auth_event<S: NostrSigner>(
    signer: &S,
    challenge_hex: &str,
    origin: &str,
) -> Result<SignetAuthEventWire, JoinAuthError> {
    // Exactly the two tags the verifier requires, content empty per spec.
    let tags = [
        Tag::parse(["challenge", challenge_hex])
            .map_err(|e| JoinAuthError::Other(format!("challenge tag: {e}")))?,
        Tag::parse(["origin", origin])
            .map_err(|e| JoinAuthError::Other(format!("origin tag: {e}")))?,
    ];
    let event = EventBuilder::new(Kind::Custom(AUTH_EVENT_KIND as u16), "")
        .tags(tags)
        .sign(signer)
        .await
        .map_err(|e| classify_signer_error(&e.to_string()))?;
    Ok(event_to_auth_wire(&event))
}

/// Map a signed `nostr::Event` into the engine's `SignetAuthEventWire`. The
/// `from_np` flag is always `false` — a native persona key is not a Natural
/// Person keypair (the server rejects `from_np == true`).
fn event_to_auth_wire(event: &Event) -> SignetAuthEventWire {
    SignetAuthEventWire {
        pubkey: event.pubkey.to_bytes(),
        created_at: event.created_at.as_secs() as u32,
        kind: event.kind.as_u16() as u32,
        tags: event.tags.iter().map(|t| t.as_slice().to_vec()).collect(),
        content: event.content.clone(),
        id: event.id.to_bytes(),
        sig: event.sig.serialize().to_vec(),
        from_np: false,
    }
}

/// The signed auth material a `RemoteClient` attaches to its join: the auth event
/// plus an optional handle credential, both already in wire form. Cross-platform
/// (carries only wire DTOs) so `remote_client` stays target-agnostic.
#[derive(Clone, Debug)]
pub struct JoinAuth {
    pub auth_event: SignetAuthEventWire,
    pub credential: Option<SignetCredentialWire>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signet::{verify_auth_event, SignetAuthEvent, VerifyResult};

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("axe_signer_{}_{}", tag, std::process::id()))
    }

    // ── Offline identity ────────────────────────────────────────────────────

    #[test]
    fn no_cache_is_guest() {
        let p = temp_path("noident").join("signet_identity.json");
        let _ = std::fs::remove_file(&p);
        assert_eq!(load_identity_from(&p), NativeIdentity::Guest);
    }

    #[test]
    fn cached_pubkey_is_signed_in_offline() {
        // Write a cache, then read it back with ZERO network — the offline
        // "who am I" path. (Decision 2 of the contract.)
        let dir = temp_path("ident");
        let p = dir.join("signet_identity.json");
        let pk = "1".repeat(64);
        cache_identity_to(&p, &pk).unwrap();
        let id = load_identity_from(&p);
        assert_eq!(id, NativeIdentity::SignedIn { pubkey_hex: pk.clone() });
        assert!(id.is_signed_in());
        assert_eq!(id.pubkey_hex(), Some(pk.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_cache_degrades_to_guest() {
        let dir = temp_path("corrupt");
        let p = dir.join("signet_identity.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, b"not json").unwrap();
        assert_eq!(load_identity_from(&p), NativeIdentity::Guest);
        // Valid JSON but a bad (short) pubkey also degrades, never surfaces.
        std::fs::write(&p, br#"{"pubkey_hex":"abc"}"#).unwrap();
        assert_eq!(load_identity_from(&p), NativeIdentity::Guest);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn npub_renders_bech32_on_native() {
        // Real persona pubkey → npub1… (proves native bech32 via nostr).
        let keys = Keys::generate();
        let hex = keys.public_key().to_hex();
        let id = NativeIdentity::SignedIn { pubkey_hex: hex };
        let npub = id.npub().unwrap();
        assert!(npub.starts_with("npub1"), "expected bech32 npub, got {npub}");
    }

    #[test]
    fn npub_fails_closed_never_falls_back_to_hex() {
        // `nostr::PublicKey` (pinned `=0.44.3`) does zero curve validation:
        // `from_hex` only fails on malformed hex text (wrong length / non-hex
        // chars), and `PublicKey::to_bech32` has `type Err = Infallible` — any
        // 32 raw bytes always encodes. So there is no "valid hex, invalid
        // curve point" case to construct; the one real failure mode is a
        // `pubkey_hex` that isn't well-formed hex at all. That's reachable in
        // production: `native_signin.rs` and `menu.rs` build
        // `NativeIdentity::SignedIn { pubkey_hex }` directly from a live
        // sign-in/pairing result, bypassing the `is_valid_pubkey_hex` gate
        // that protects only the on-disk cache load path.
        //
        // Before the fix, `npub()` fell back to returning this malformed
        // string verbatim on parse failure; the "npub…, never hex" rule
        // (feedback_npub_only_display) requires it instead degrade to `None`
        // (shown by friends_ui as the not-signed-in / guest state) — it must
        // never surface a raw, unvalidated string as if it were the npub.
        let bad_hex = "not-a-hex-string-at-all".to_string();
        assert!(
            !is_valid_pubkey_hex(&bad_hex),
            "fixture must be something the cache-load gate would already reject, \
             proving this failure mode is only reachable via the direct-construction paths"
        );
        assert!(
            PublicKey::from_hex(&bad_hex).is_err(),
            "fixture must actually fail hex parsing, or this test proves nothing"
        );
        let id = NativeIdentity::SignedIn { pubkey_hex: bad_hex };
        assert_eq!(
            id.npub(),
            None,
            "an unparsable pubkey must fail closed to None, never fall back to the raw string"
        );
    }

    // ── Session store ─────────────────────────────────────────────────────────

    #[test]
    fn session_round_trips_and_no_session_is_none() {
        let dir = temp_path("sess");
        let p = dir.join("signet_session.json");
        let _ = std::fs::remove_file(&p);
        // No file → Ok(None) (= guest), never an error.
        assert!(load_session_from(&p).unwrap().is_none());

        let persisted = PersistedSession {
            app_secret_key: "a".repeat(64),
            bunker_uri: "bunker://79dff8f82963424e0bb02708a22e44b4980893e3a4be0fa3cb60a43b946764e3?relay=wss://relay.trotters.cc".to_string(),
        };
        save_session_to(&p, &persisted).unwrap();
        let back = load_session_from(&p).unwrap().unwrap();
        assert_eq!(back.app_secret_key, persisted.app_secret_key);
        assert_eq!(back.bunker_uri, persisted.bunker_uri);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn session_file_is_locked_down() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_path("perms");
        let p = dir.join("signet_session.json");
        save_session_to(
            &p,
            &PersistedSession {
                app_secret_key: "b".repeat(64),
                bunker_uri: "bunker://79dff8f82963424e0bb02708a22e44b4980893e3a4be0fa3cb60a43b946764e3?relay=wss://relay.trotters.cc".to_string(),
            },
        )
        .unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "session file must be owner-only");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_signer_offline_with_no_session_is_none() {
        // The startup path: no session on disk → no signer, no network, no error.
        let dir = temp_path("restore_none");
        let p = dir.join("signet_session.json");
        let _ = std::fs::remove_file(&p);
        assert!(restore_signer_from(&p, SessionOptions::default()).unwrap().is_none());
    }

    #[test]
    fn restore_signer_offline_reconstructs_from_persisted() {
        // A persisted session reconstructs a signer with NO network (the relay
        // handshake is lazy). This is the offline-relaunch property.
        let dir = temp_path("restore_some");
        let p = dir.join("signet_session.json");
        let app = Keys::generate();
        save_session_to(
            &p,
            &PersistedSession {
                app_secret_key: app.secret_key().to_secret_hex(),
                bunker_uri: "bunker://79dff8f82963424e0bb02708a22e44b4980893e3a4be0fa3cb60a43b946764e3?relay=wss://relay.trotters.cc".to_string(),
            },
        )
        .unwrap();
        let signer = restore_signer_from(&p, SessionOptions::default()).unwrap();
        assert!(signer.is_some(), "a persisted session must restore offline");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Owner pubkey (Seam B input) ───────────────────────────────────────────

    #[test]
    fn owner_pubkey_set_and_clear() {
        set_owner_pubkey(Some("c".repeat(64)));
        assert_eq!(current_owner_pubkey(), Some("c".repeat(64)));
        set_owner_pubkey(None);
        assert_eq!(current_owner_pubkey(), None);
    }

    // ── The headline: client build → server verify, offline, local key ───────

    #[tokio::test]
    async fn builds_a_server_verifiable_auth_event() {
        // Build a kind-21236 event with a LOCAL key and prove the engine's OWN
        // server-side verifier accepts it — the whole client path, no network,
        // no bunker. If the canonical-id or signature shape ever drifts from the
        // server's expectation, this fails (BadEventId / BadSignature).
        let keys = Keys::generate();
        let challenge = "f".repeat(64);
        let origin = "https://axenstax.app";

        let wire = build_join_auth_event(&keys, &challenge, origin)
            .await
            .expect("auth event should build + sign");

        // pubkey + sig lengths are exactly what the wire→crypto conversion wants.
        assert_eq!(wire.kind, AUTH_EVENT_KIND);
        assert_eq!(wire.sig.len(), 64);
        assert!(!wire.from_np);

        let event: SignetAuthEvent = wire.try_into().expect("wire converts to crypto type");
        // Verify against the event's own created_at so the skew window is trivially
        // satisfied (the window itself is covered separately in verify.rs).
        let now = event.created_at;
        assert_eq!(
            verify_auth_event(&event, &challenge, origin, now),
            VerifyResult::Ok,
            "the engine's server verifier must accept the client-built event"
        );
    }

    #[tokio::test]
    async fn auth_event_is_rejected_for_the_wrong_challenge() {
        // Sanity: the binding actually binds — a different challenge fails verify.
        let keys = Keys::generate();
        let wire = build_join_auth_event(&keys, &"a".repeat(64), "https://axenstax.app")
            .await
            .unwrap();
        let event: SignetAuthEvent = wire.try_into().unwrap();
        let now = event.created_at;
        assert_eq!(
            verify_auth_event(&event, &"b".repeat(64), "https://axenstax.app", now),
            VerifyResult::WrongChallenge,
        );
    }

    #[test]
    fn classify_signer_error_buckets() {
        assert_eq!(classify_signer_error("request timed out"), JoinAuthError::Timeout);
        assert_eq!(classify_signer_error("user denied the request"), JoinAuthError::Denied);
        assert!(matches!(classify_signer_error("relay closed"), JoinAuthError::Other(_)));
    }
}

//! # signet-nip46-client
//!
//! A thin, **generic** NIP-46 bunker signer for Rust. It wraps
//! [`nostr_connect::prelude::NostrConnect`] and exposes a small **pair / sign / restore**
//! surface plus an implementation of [`nostr::signer::NostrSigner`], so a paired session
//! is a drop-in signer anywhere rust-nostr wants one — `EventBuilder::sign`,
//! `nostr-blossom`, or a save-sync layer like `stash-rs`.
//!
//! The "key stays on your phone" model: the user's secret key lives in a remote NIP-46
//! bunker (Signet / Heartwood / nsec.app / Amber / a self-hosted signer). This crate is
//! the **client** half — it never holds the user's key, only a throwaway per-session app
//! keypair used to talk to the bunker.
//!
//! ## Generic, with Signet-flow defaults
//!
//! Nothing here is specific to any one bunker product or to any consuming app. The only
//! "Signet" content is [`SessionOptions::default`] (a 60s approval timeout tuned for the
//! Signet/Heartwood pairing UX). Any NIP-46 bunker works.
//!
//! ## Pair → persist → restore
//!
//! ```no_run
//! use signet_nip46_client::{BunkerSession, SessionOptions};
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! // 1. Pair (the user approves on their phone on first use).
//! let session = BunkerSession::pair(
//!     "bunker://<remote-pubkey>?relay=wss://relay.trotters.cc&secret=...",
//!     SessionOptions::default(),
//! )?;
//! let _persona = session.user_public_key().await?;       // round-trips to the bunker
//!
//! // 2. Persist for later (store this somewhere the app owns — keystore / file).
//! let persisted = session.persist().await?;
//! let json = serde_json::to_string(&persisted)?;
//!
//! // 3. On a later launch — reconnect WITHOUT a fresh approval.
//! let persisted = serde_json::from_str(&json)?;
//! let session = BunkerSession::restore(persisted, SessionOptions::default())?;
//! # Ok(()) }
//! ```
//!
//! The **live pairing handshake needs the user's device** — it is the one part that can't
//! be exercised headlessly. Construction, persistence round-tripping, and the
//! `NostrSigner` delegation are unit-tested with local keys.

mod error;

pub use error::Error;

use std::time::Duration;

use nostr::prelude::*;
use nostr::signer::{NostrSigner, SignerBackend, SignerError};
use nostr_connect::prelude::NostrConnect;
use nostr_relay_pool::prelude::{RelayOptions, RelayPool, RelayPoolNotification, SubscribeOptions};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::OnceCell;

/// Tunables for a bunker session.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// How long to wait for the bunker to answer a request (connect, sign, …).
    pub timeout: Duration,
}

impl Default for SessionOptions {
    fn default() -> Self {
        // Tuned for the Signet/Heartwood approval UX: long enough for a human to tap
        // "approve" on their phone, short enough to fail fast if the bunker is offline.
        Self {
            timeout: Duration::from_secs(60),
        }
    }
}

/// Everything needed to silently reconnect a session on a later launch.
///
/// Holds **no user key**: `app_secret_key` is the throwaway NIP-46 *client* identity, and
/// `bunker_uri` carries the remote signer's pubkey + relays. Persist it somewhere the app
/// controls (an OS keystore, an app-private file). Treat `app_secret_key` as a secret —
/// it identifies this client session to the bunker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedSession {
    /// Hex secret key of the per-session NIP-46 **client** keypair (not the persona).
    pub app_secret_key: String,
    /// `bunker://<remote-signer-pubkey>?relay=...` — the reconnect URI (secret stripped).
    pub bunker_uri: String,
}

/// A live NIP-46 bunker session. Implements [`nostr::signer::NostrSigner`].
#[derive(Debug)]
pub struct BunkerSession {
    /// The per-session NIP-46 **client** keypair (never the user's key).
    app_keys: Keys,
    /// The rust-nostr bunker client that does the signing round-trips. Seeded at
    /// construction for `pair`/`restore`; for `pair_nostrconnect` it is filled only
    /// once our own client-initiated handshake has discovered the remote signer.
    inner: OnceCell<NostrConnect>,
    /// Present only for a client-initiated (`nostrconnect://`) pairing.
    pending: Option<PendingNostrConnect>,
    timeout: Duration,
    /// The URI this session was paired/restored from, kept for a secret-less persist
    /// fallback when the canonical post-handshake `bunker_uri()` isn't reachable yet.
    original_uri: String,
}

/// State for a client-initiated pairing awaiting the signer's `connect` response.
///
/// DIVERGENCE (must be upstreamed — see VENDOR.md): rust-nostr 0.44's
/// `NostrConnectURI::Client` carries no `secret`, and its client only accepts a
/// literal `"ack"` as the signer's reply. Current NIP-46 requires a `secret` in the
/// `nostrconnect://` URI and has the signer reply with that secret, so signers that
/// follow the spec (mySignet) silently ignore our QR. We therefore mint the secret,
/// run the handshake ourselves, and hand the discovered signer to rust-nostr as a
/// plain `bunker://` session.
#[derive(Debug)]
struct PendingNostrConnect {
    relays: Vec<RelayUrl>,
    secret: String,
    /// Remote signer pubkey, set once the handshake completes.
    remote_signer: OnceCell<PublicKey>,
}

impl BunkerSession {
    /// Pair with a bunker from a `bunker://` or `nostrconnect://` URI. A fresh app keypair
    /// is generated for this session. Construction is offline; the first
    /// [`user_public_key`](Self::user_public_key) / sign call performs the relay handshake
    /// and triggers the bunker's approval prompt.
    pub fn pair(uri: &str, opts: SessionOptions) -> Result<Self, Error> {
        let parsed = NostrConnectURI::parse(uri).map_err(|e| Error::Uri(e.to_string()))?;
        let app_keys = Keys::generate();
        Self::from_uri(parsed, app_keys, opts, uri.to_string())
    }

    /// A session whose rust-nostr client is ready at construction (`pair` / `restore`).
    fn from_uri(
        uri: NostrConnectURI,
        app_keys: Keys,
        opts: SessionOptions,
        original_uri: String,
    ) -> Result<Self, Error> {
        let inner = NostrConnect::new(uri, app_keys.clone(), opts.timeout, None)
            .map_err(|e| Error::Connect(e.to_string()))?;
        Ok(Self {
            app_keys,
            inner: OnceCell::new_with(Some(inner)),
            pending: None,
            timeout: opts.timeout,
            original_uri,
        })
    }

    /// Begin a **client-initiated** pairing (the `nostrconnect://` direction): the
    /// app generates a fresh keypair and advertises itself on `relays`, returning
    /// the `nostrconnect://` URI to **display as a QR** for the signer (phone) to
    /// scan. The returned session resolves once the signer connects — the first
    /// [`user_public_key`](Self::user_public_key) / sign call completes the
    /// handshake and triggers the approval prompt.
    ///
    /// This is the reverse of [`pair`](Self::pair) (where the *bunker* generates a
    /// `bunker://` URI the app consumes). Construction is offline. Generic NIP-46:
    /// `app_name` is shown to the signer; `relays` are where the app listens.
    pub fn pair_nostrconnect<I, S>(
        relays: I,
        app_name: S,
        opts: SessionOptions,
    ) -> Result<(String, Self), Error>
    where
        I: IntoIterator<Item = String>,
        S: Into<String>,
    {
        let relay_urls: Vec<RelayUrl> = relays
            .into_iter()
            .map(|r| RelayUrl::parse(&r).map_err(|e| Error::Uri(e.to_string())))
            .collect::<Result<_, _>>()?;
        if relay_urls.is_empty() {
            return Err(Error::Uri("at least one relay is required".to_string()));
        }
        let app_keys = Keys::generate();
        let secret = generate_connect_secret();
        let uri_string =
            build_nostrconnect_uri(&app_keys.public_key(), &relay_urls, &app_name.into(), &secret)?;
        Ok((
            uri_string.clone(),
            Self {
                app_keys,
                inner: OnceCell::new(),
                pending: Some(PendingNostrConnect {
                    relays: relay_urls,
                    secret,
                    remote_signer: OnceCell::new(),
                }),
                timeout: opts.timeout,
                original_uri: uri_string,
            },
        ))
    }

    /// Reconnect a previously-paired session by reusing its stored app keypair + bunker
    /// URI. Re-uses the existing NIP-46 grant, so a well-behaved bunker does not re-prompt.
    pub fn restore(persisted: PersistedSession, opts: SessionOptions) -> Result<Self, Error> {
        let parsed =
            NostrConnectURI::parse(&persisted.bunker_uri).map_err(|e| Error::Uri(e.to_string()))?;
        let app_keys =
            Keys::parse(&persisted.app_secret_key).map_err(|e| Error::Key(e.to_string()))?;
        Self::from_uri(parsed, app_keys, opts, persisted.bunker_uri)
    }

    /// The ready rust-nostr client. For a `nostrconnect://` pairing the first call runs
    /// the client-initiated handshake (waits for the signer to scan + approve), then
    /// hands the discovered signer to rust-nostr as a secret-less `bunker://` session.
    async fn ready(&self) -> Result<&NostrConnect, Error> {
        self.inner
            .get_or_try_init(|| async {
                let pending = self.pending.as_ref().ok_or_else(|| {
                    Error::Connect("session has no bunker client".to_string())
                })?;
                let remote = *pending
                    .remote_signer
                    .get_or_try_init(|| {
                        await_nostrconnect_handshake(
                            &self.app_keys,
                            &pending.relays,
                            &pending.secret,
                            self.timeout,
                        )
                    })
                    .await?;
                let bunker = NostrConnectURI::Bunker {
                    remote_signer_public_key: remote,
                    relays: pending.relays.clone(),
                    secret: None,
                };
                NostrConnect::new(bunker, self.app_keys.clone(), self.timeout, None)
                    .map_err(|e| Error::Connect(e.to_string()))
            })
            .await
    }

    /// Produce the [`PersistedSession`] to store for a later [`restore`](Self::restore).
    ///
    /// Prefers the canonical post-handshake bunker URI (carries the discovered remote
    /// signer pubkey for the `nostrconnect://` flow and drops the single-use connect
    /// secret). Falls back to a secret-stripped form of the original `bunker://` URI when
    /// the session hasn't completed its handshake yet.
    ///
    /// **Errors** if the session was paired from a `nostrconnect://` URI and hasn't
    /// completed its handshake — that flow has no stable reconnect address until the remote
    /// signer pubkey is discovered, so call [`user_public_key`](Self::user_public_key)
    /// first (which completes the handshake), then `persist()`.
    pub async fn persist(&self) -> Result<PersistedSession, Error> {
        let bunker_uri = match (&self.pending, self.inner.get()) {
            // Client-initiated: the reconnect address is the signer our own handshake
            // discovered (no network needed). Before that, there is nothing to persist.
            (Some(pending), _) => match pending.remote_signer.get() {
                Some(remote) => NostrConnectURI::Bunker {
                    remote_signer_public_key: *remote,
                    relays: pending.relays.clone(),
                    secret: None,
                }
                .to_string(),
                None => return Err(nostrconnect_not_ready()),
            },
            (None, Some(inner)) => match inner.bunker_uri().await {
                Ok(uri) => uri.to_string(),
                Err(_) => self.secretless_original()?,
            },
            (None, None) => self.secretless_original()?,
        };
        Ok(PersistedSession {
            app_secret_key: self.app_keys.secret_key().to_secret_hex(),
            bunker_uri,
        })
    }

    /// The persona pubkey this session signs as. Round-trips to the bunker on first call.
    pub async fn user_public_key(&self) -> Result<PublicKey, Error> {
        self.ready()
            .await?
            .get_public_key()
            .await
            .map_err(|e| Error::Signer(e.to_string()))
    }

    /// Tear down relay connections held by the session.
    pub async fn shutdown(self) {
        if let Some(inner) = self.inner.into_inner() {
            inner.shutdown().await;
        }
    }

    /// Re-emit the original `bunker://` URI without its single-use connect secret (restore
    /// must not replay it). A `nostrconnect://` original has no stable reconnect address
    /// until the handshake completes, so it errors rather than persist an unusable URI that
    /// would silently re-pair on restore.
    fn secretless_original(&self) -> Result<String, Error> {
        let parsed =
            NostrConnectURI::parse(&self.original_uri).map_err(|e| Error::Uri(e.to_string()))?;
        match parsed {
            NostrConnectURI::Bunker {
                remote_signer_public_key,
                relays,
                ..
            } => Ok(NostrConnectURI::Bunker {
                remote_signer_public_key,
                relays,
                secret: None,
            }
            .to_string()),
            NostrConnectURI::Client { .. } => Err(nostrconnect_not_ready()),
        }
    }
}

fn nostrconnect_not_ready() -> Error {
    Error::Connect(
        "cannot persist a nostrconnect:// session before its handshake completes \
         (no stable reconnect URI yet — call user_public_key() first)"
            .to_string(),
    )
}

/// A fresh single-use connect secret: 16 bytes from the OS CSPRNG, hex-encoded.
fn generate_connect_secret() -> String {
    use nostr::prelude::rand::rngs::OsRng;
    use nostr::prelude::rand::RngCore;
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Build the `nostrconnect://` QR URI per current NIP-46: app pubkey, every relay, the
/// one-shot `secret`, and the app name both as `name=` and as `metadata={"name":..}`
/// (rust-nostr 0.44's parser requires `metadata`; some signers read `name`). Every
/// query value is form-urlencoded.
fn build_nostrconnect_uri(
    app_pk: &PublicKey,
    relays: &[RelayUrl],
    app_name: &str,
    secret: &str,
) -> Result<String, Error> {
    let mut url = Url::parse(&format!("nostrconnect://{}", app_pk.to_hex()))
        .map_err(|e| Error::Uri(e.to_string()))?;
    {
        let mut q = url.query_pairs_mut();
        for relay in relays {
            q.append_pair("relay", relay.as_str_without_trailing_slash());
        }
        q.append_pair("secret", secret);
        q.append_pair("name", app_name);
        q.append_pair(
            "metadata",
            &nostr::serde_json::json!({ "name": app_name }).to_string(),
        );
    }
    Ok(url.to_string())
}

/// The `connect` response a signer sends back for a client-initiated pairing. Parsed
/// loosely on purpose: rust-nostr's `ResponseResult::parse` for `connect` coerces to
/// `Ack` and rejects the secret echo that current NIP-46 specifies.
#[derive(Deserialize)]
struct ConnectReply {
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// What a decrypted client-initiated `connect` reply means for the handshake.
#[derive(Debug, PartialEq, Eq)]
enum ConnectVerdict {
    /// The signer echoed our secret — pairing accepted.
    Accept,
    /// The signer replied with a non-empty `error` — fail the handshake now.
    Refused(String),
    /// Anything else (wrong secret, `"ack"`, no result) — keep waiting.
    Ignore,
}

/// NIP-46 acceptance rule for a client-initiated `connect` reply. Our URI always
/// carries a `secret`, so ONLY an exact (constant-time) echo of it is accepted — a
/// bare `"ack"` is not: the relay operator sees our app pubkey in the `#p`
/// subscription (but never the secret), so accepting `"ack"` would let it race to
/// become our "signer". A non-empty `error` refuses the pairing.
fn classify_connect_reply(result: Option<&str>, error: Option<&str>, secret: &str) -> ConnectVerdict {
    use subtle::ConstantTimeEq;
    if let Some(e) = error.filter(|e| !e.is_empty()) {
        return ConnectVerdict::Refused(e.to_string());
    }
    match result {
        Some(r) if bool::from(r.as_bytes().ct_eq(secret.as_bytes())) => ConnectVerdict::Accept,
        _ => ConnectVerdict::Ignore,
    }
}

/// Classify `event` as a `connect` reply addressed to `app_keys` for this pairing's
/// `secret`. `Some(Ok(signer))` = accepted (the event author is the remote signer);
/// `Some(Err(msg))` = the signer refused; `None` = not ours / junk / wrong secret —
/// relays can carry junk, and it must never abort the wait.
fn accept_connect_event(
    event: &Event,
    app_keys: &Keys,
    secret: &str,
) -> Option<Result<PublicKey, String>> {
    if event.kind != Kind::NostrConnect || event.verify().is_err() {
        return None;
    }
    let plaintext = nip44::decrypt(app_keys.secret_key(), &event.pubkey, &event.content).ok()?;
    let reply: ConnectReply = nostr::serde_json::from_str(&plaintext).ok()?;
    match classify_connect_reply(reply.result.as_deref(), reply.error.as_deref(), secret) {
        ConnectVerdict::Accept => Some(Ok(event.pubkey)),
        ConnectVerdict::Refused(e) => Some(Err(e)),
        ConnectVerdict::Ignore => None,
    }
}

/// Shuts the handshake's relay pool down even if the handshake future is dropped
/// mid-wait (caller cancelled / runtime tearing down the task).
struct PoolGuard(Option<RelayPool>);

impl Drop for PoolGuard {
    fn drop(&mut self) {
        if let Some(pool) = self.0.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move { pool.shutdown().await });
            }
        }
    }
}

/// Listen on `relays` for the signer's reply to our `nostrconnect://` QR and return
/// the remote signer pubkey. Bounded by `timeout`; the relay pool is torn down after.
async fn await_nostrconnect_handshake(
    app_keys: &Keys,
    relays: &[RelayUrl],
    secret: &str,
    timeout: Duration,
) -> Result<PublicKey, Error> {
    let mut guard = PoolGuard(Some(RelayPool::default()));
    let pool = guard.0.clone().expect("pool just created");
    let result = async {
        for url in relays {
            pool.add_relay(url, RelayOptions::default())
                .await
                .map_err(|e| Error::Connect(e.to_string()))?;
        }
        pool.connect().await;
        // Take the receiver BEFORE subscribing so no reply can slip past us.
        let mut notifications = pool.notifications();
        let filter = Filter::new()
            .pubkey(app_keys.public_key())
            .kind(Kind::NostrConnect)
            .limit(0);
        pool.subscribe(filter, SubscribeOptions::default())
            .await
            .map_err(|e| Error::Connect(e.to_string()))?;

        let wait = async {
            loop {
                match notifications.recv().await {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        match accept_connect_event(&event, app_keys, secret) {
                            Some(Ok(remote)) => return Ok(remote),
                            Some(Err(refusal)) => {
                                return Err(Error::Signer(format!(
                                    "signer refused the pairing: {refusal}"
                                )))
                            }
                            None => {}
                        }
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => {
                        return Err(Error::Connect(
                            "relay pool closed before the signer connected".to_string(),
                        ))
                    }
                }
            }
        };
        tokio::time::timeout(timeout, wait).await.map_err(|_| {
            Error::Signer("timed out waiting for the signer to scan and approve".to_string())
        })?
    }
    .await;
    // Normal completion: shut down inline (the guard only covers a dropped future).
    guard.0 = None;
    pool.shutdown().await;
    result
}

// A BunkerSession IS a signer — every method delegates to the wrapped NostrConnect, which
// drives the NIP-46 round-trip to the remote bunker. This is what makes a paired session
// drop-in for `stash-rs`, `nostr-blossom`, and `EventBuilder::sign`.
impl NostrSigner for BunkerSession {
    fn backend(&self) -> SignerBackend<'_> {
        SignerBackend::NostrConnect
    }

    fn get_public_key(&self) -> BoxedFuture<'_, Result<PublicKey, SignerError>> {
        Box::pin(async move {
            self.ready()
                .await
                .map_err(SignerError::backend)?
                .get_public_key()
                .await
        })
    }

    fn sign_event(&self, unsigned: UnsignedEvent) -> BoxedFuture<'_, Result<Event, SignerError>> {
        Box::pin(async move {
            self.ready()
                .await
                .map_err(SignerError::backend)?
                .sign_event(unsigned)
                .await
        })
    }

    fn nip04_encrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.ready()
                .await
                .map_err(SignerError::backend)?
                .nip04_encrypt(public_key, content)
                .await
        })
    }

    fn nip04_decrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        encrypted_content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.ready()
                .await
                .map_err(SignerError::backend)?
                .nip04_decrypt(public_key, encrypted_content)
                .await
        })
    }

    fn nip44_encrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.ready()
                .await
                .map_err(SignerError::backend)?
                .nip44_encrypt(public_key, content)
                .await
        })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        payload: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.ready()
                .await
                .map_err(SignerError::backend)?
                .nip44_decrypt(public_key, payload)
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A well-formed bunker URI for construction tests (random pubkey; never connected).
    const BUNKER_URI: &str = "bunker://79dff8f82963424e0bb02708a22e44b4980893e3a4be0fa3cb60a43b946764e3?relay=wss://relay.trotters.cc&secret=abc123";

    #[test]
    fn pair_constructs_from_a_valid_bunker_uri() {
        let s = BunkerSession::pair(BUNKER_URI, SessionOptions::default());
        assert!(s.is_ok(), "pairing should construct (no live connection yet)");
    }

    #[test]
    fn pair_rejects_a_malformed_uri() {
        let s = BunkerSession::pair("not-a-uri", SessionOptions::default());
        assert!(matches!(s, Err(Error::Uri(_))));
    }

    #[test]
    fn pair_nostrconnect_builds_a_scannable_uri() {
        // The client-initiated (QR) flow: produces a nostrconnect:// URI carrying
        // the freshly-generated app pubkey, which round-trips through the parser.
        let (uri, _session) = BunkerSession::pair_nostrconnect(
            ["wss://relay.trotters.cc".to_string()],
            "Axe'n'Stax",
            SessionOptions::default(),
        )
        .expect("client pairing should construct (no live connection yet)");
        assert!(uri.starts_with("nostrconnect://"), "expected a nostrconnect URI, got {uri}");
        assert!(NostrConnectURI::parse(&uri).is_ok(), "the QR URI must be parseable");
    }

    /// Our own decode of the QR URI's query (what a spec-following signer reads).
    fn query(uri: &str) -> Vec<(String, String)> {
        Url::parse(uri)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    fn get<'a>(q: &'a [(String, String)], key: &str) -> Option<&'a str> {
        q.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn nostrconnect_uri_carries_secret_relay_and_name() {
        // mySignet's parseNostrConnectURI returns null without `secret` — the bug.
        let (uri, session) = BunkerSession::pair_nostrconnect(
            ["wss://relay.trotters.cc".to_string()],
            "Axe'n'Stax",
            SessionOptions::default(),
        )
        .unwrap();
        let q = query(&uri);
        let secret = get(&q, "secret").expect("URI must carry a secret");
        assert_eq!(secret.len(), 32, "16 random bytes, hex");
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(secret, session.pending.as_ref().unwrap().secret);
        assert_eq!(get(&q, "relay"), Some("wss://relay.trotters.cc"));
        assert_eq!(get(&q, "name"), Some("Axe'n'Stax"));
        let meta: nostr::serde_json::Value =
            nostr::serde_json::from_str(get(&q, "metadata").unwrap()).unwrap();
        assert_eq!(meta["name"], "Axe'n'Stax");
        let host = Url::parse(&uri).unwrap().host_str().unwrap().to_string();
        assert_eq!(host, session.app_keys.public_key().to_hex());
    }

    #[test]
    fn each_pairing_mints_a_fresh_secret() {
        let mk = || {
            BunkerSession::pair_nostrconnect(
                ["wss://relay.trotters.cc".to_string()],
                "x",
                SessionOptions::default(),
            )
            .unwrap()
            .1
        };
        assert_ne!(
            mk().pending.unwrap().secret,
            mk().pending.unwrap().secret
        );
    }

    #[test]
    fn connect_reply_predicate_follows_nip46() {
        use ConnectVerdict::*;
        assert_eq!(classify_connect_reply(Some("s3cret"), None, "s3cret"), Accept);
        assert_eq!(classify_connect_reply(Some("s3cret"), Some(""), "s3cret"), Accept);
        // "ack" is NOT enough when we issued a secret (relay-race hijack).
        assert_eq!(classify_connect_reply(Some("ack"), None, "s3cret"), Ignore);
        assert_eq!(classify_connect_reply(Some("wrong"), None, "s3cret"), Ignore);
        assert_eq!(classify_connect_reply(Some("s3cre"), None, "s3cret"), Ignore);
        assert_eq!(classify_connect_reply(None, None, "s3cret"), Ignore);
        assert_eq!(
            classify_connect_reply(Some("s3cret"), Some("denied"), "s3cret"),
            Refused("denied".to_string())
        );
    }

    /// A kind-24133 reply from `signer` to `to`, NIP-44 encrypted, as a relay delivers it.
    fn reply_event(signer: &Keys, to: &PublicKey, json: &str) -> Event {
        let ct = nip44::encrypt(signer.secret_key(), to, json, nip44::Version::V2).unwrap();
        EventBuilder::new(Kind::NostrConnect, ct)
            .tag(Tag::public_key(*to))
            .sign_with_keys(signer)
            .unwrap()
    }

    #[test]
    fn accept_connect_event_returns_the_signer_pubkey() {
        let app = Keys::generate();
        let signer = Keys::generate();
        let secret = "0123456789abcdef0123456789abcdef";
        // mySignet's sendConnectResponse shape: {id, result: <the secret>}.
        let ev = reply_event(&signer, &app.public_key(), &format!(r#"{{"id":"r1","result":"{secret}"}}"#));
        assert_eq!(accept_connect_event(&ev, &app, secret), Some(Ok(signer.public_key())));
    }

    #[test]
    fn accept_connect_event_rejects_a_bare_ack() {
        // Anyone who sees our #p subscription (e.g. the relay) can send "ack".
        let app = Keys::generate();
        let attacker = Keys::generate();
        let ev = reply_event(&attacker, &app.public_key(), r#"{"id":"r2","result":"ack"}"#);
        assert_eq!(accept_connect_event(&ev, &app, "0123456789abcdef0123456789abcdef"), None);
    }

    #[test]
    fn accept_connect_event_surfaces_a_signer_error() {
        let app = Keys::generate();
        let signer = Keys::generate();
        let secret = "0123456789abcdef0123456789abcdef";
        let ev = reply_event(
            &signer,
            &app.public_key(),
            &format!(r#"{{"id":"r","result":"{secret}","error":"denied"}}"#),
        );
        assert_eq!(accept_connect_event(&ev, &app, secret), Some(Err("denied".to_string())));
    }

    #[test]
    fn accept_connect_event_ignores_everything_else() {
        let app = Keys::generate();
        let signer = Keys::generate();
        let secret = "0123456789abcdef0123456789abcdef";
        let wrong = reply_event(&signer, &app.public_key(), r#"{"id":"r","result":"nope"}"#);
        assert_eq!(accept_connect_event(&wrong, &app, secret), None);
        let junk = reply_event(&signer, &app.public_key(), "not json");
        assert_eq!(accept_connect_event(&junk, &app, secret), None);
        // Encrypted to someone else — undecryptable by us.
        let other = Keys::generate();
        let misaddressed = reply_event(&signer, &other.public_key(), r#"{"id":"r","result":"ack"}"#);
        assert_eq!(accept_connect_event(&misaddressed, &app, secret), None);
        // Wrong kind.
        let note = EventBuilder::text_note("ack").sign_with_keys(&signer).unwrap();
        assert_eq!(accept_connect_event(&note, &app, secret), None);
    }

    #[tokio::test]
    async fn nostrconnect_persist_refuses_before_and_allows_after_handshake() {
        let (_uri, session) = BunkerSession::pair_nostrconnect(
            ["wss://relay.trotters.cc".to_string()],
            "Axe'n'Stax",
            SessionOptions::default(),
        )
        .unwrap();
        assert!(matches!(session.persist().await, Err(Error::Connect(_))));
        // Simulate the handshake having discovered the signer (no network).
        let signer = Keys::generate();
        session
            .pending
            .as_ref()
            .unwrap()
            .remote_signer
            .set(signer.public_key())
            .unwrap();
        let p = session.persist().await.expect("persist after handshake");
        assert_eq!(p.app_secret_key, session.app_keys.secret_key().to_secret_hex());
        match NostrConnectURI::parse(&p.bunker_uri).unwrap() {
            NostrConnectURI::Bunker {
                remote_signer_public_key,
                relays,
                secret,
            } => {
                assert_eq!(remote_signer_public_key, signer.public_key());
                assert_eq!(relays.len(), 1);
                assert!(secret.is_none(), "the one-shot secret must not be persisted");
            }
            other => panic!("expected a bunker URI, got {other:?}"),
        }
        // And it restores.
        assert!(BunkerSession::restore(p, SessionOptions::default()).is_ok());
    }

    #[test]
    fn pair_nostrconnect_rejects_an_empty_relay_list() {
        let r: [String; 0] = [];
        assert!(matches!(
            BunkerSession::pair_nostrconnect(r, "Axe'n'Stax", SessionOptions::default()),
            Err(Error::Uri(_))
        ));
    }

    #[test]
    fn restore_round_trips_a_persisted_session() {
        // The app keypair survives a hex round-trip with a stable pubkey, and the restored
        // session keeps the same client identity — the core of silent reconnection.
        let app_keys = Keys::generate();
        let persisted = PersistedSession {
            app_secret_key: app_keys.secret_key().to_secret_hex(),
            bunker_uri: BUNKER_URI.to_string(),
        };
        let restored =
            BunkerSession::restore(persisted, SessionOptions::default()).expect("restore");
        assert_eq!(
            restored.inner.get().unwrap().local_keys().public_key(),
            app_keys.public_key(),
            "restored session must reuse the same app keypair"
        );
    }

    #[test]
    fn restore_rejects_a_bad_secret_key() {
        let persisted = PersistedSession {
            app_secret_key: "deadbeef".to_string(), // too short to be a valid secret key
            bunker_uri: BUNKER_URI.to_string(),
        };
        assert!(matches!(
            BunkerSession::restore(persisted, SessionOptions::default()),
            Err(Error::Key(_))
        ));
    }

    #[test]
    fn persisted_session_serde_round_trips() {
        let p = PersistedSession {
            app_secret_key: "a".repeat(64),
            bunker_uri: BUNKER_URI.to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PersistedSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.app_secret_key, p.app_secret_key);
        assert_eq!(back.bunker_uri, p.bunker_uri);
    }

    #[test]
    fn secretless_fallback_strips_the_connect_secret() {
        let s = BunkerSession::pair(BUNKER_URI, SessionOptions::default()).unwrap();
        let stripped = s.secretless_original().unwrap();
        assert!(
            !stripped.contains("secret="),
            "the single-use connect secret must not survive into a restore URI: {stripped}"
        );
        // Round-trips back to a parseable bunker URI.
        assert!(NostrConnectURI::parse(&stripped).is_ok());
    }

    // The `NostrSigner` delegation pattern, exercised with a local Keys signer (the same
    // trait a BunkerSession implements) — proves the contract `stash-rs`/`nostr-blossom`
    // rely on holds. The live bunker delegation is the boundary (needs a device).
    #[tokio::test]
    async fn nostr_signer_contract_holds_for_a_local_key_signer() {
        let keys = Keys::generate();
        let pk = keys.public_key();
        // encrypt-to-self round-trip (the exact path stash-rs uses for blobs).
        let ct = NostrSigner::nip44_encrypt(&keys, &pk, "payload").await.unwrap();
        let pt = NostrSigner::nip44_decrypt(&keys, &pk, &ct).await.unwrap();
        assert_eq!(pt, "payload");
        // sign + verify.
        let event = EventBuilder::text_note("hi").sign(&keys).await.unwrap();
        assert!(event.verify_signature());
    }
}

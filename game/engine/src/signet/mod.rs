//! Signet auth on the engine side — Phase 1 of the foundations spec
//! `docs/foundations/2026-04-20-engine-signet-auth.md`.
//!
//! This module is the server-side Rust counterpart to the Python reference
//! implementation in `tools/sites/game/auth.py`. It parses and verifies signed
//! Nostr events (kind-21236 auth event, kind-31000 handle credential) without
//! touching the on-wire `JoinRequestPacket` yet — that's Phase 3.
//!
//! Wire boundary note: per `feedback_signet_boundary.md`, this is engine-side
//! adoption of existing Signet patterns. Nothing here is AxeNStax-specific;
//! every check matches what `mysignet.app` emits today.

#[cfg(not(target_arch = "wasm32"))]
pub mod challenge;
// Signet contacts v2 consumer wire (pure port; native only — contacts sync is
// never part of the web build). See `contacts_wire/mod.rs`.
#[cfg(not(target_arch = "wasm32"))]
pub mod contacts_wire;
#[cfg(not(target_arch = "wasm32"))]
pub mod event;
#[cfg(not(target_arch = "wasm32"))]
pub mod verify;
// Native sign-in (Bucket 3, login half) — the engine-side wrapper over the
// vendored `signet-nip46-client` + offline-first identity + the client-side
// join auth-event builder. Native-only: it pulls `nostr`/`nostr-connect`, which
// never touch the wasm32 bundle. The WASM sign-in path lives in `auth.js`.
#[cfg(not(target_arch = "wasm32"))]
pub mod native_signer;
// Wire DTOs are cross-platform — a WASM client builds and sends them inside
// `JoinRequestPacket`. Only the in-memory crypto types + verify call sites
// stay native-only.
pub mod wire;

// Public re-exports are forward-compat hooks for Phase 4 + the future
// signing-bridge consumers (see `project_charter_phase4_shared_gap`
// memory). They aren't used inside this crate today, which Rust
// flags as "unused" — silenced explicitly because they're API.
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use challenge::{ChallengeTable, DEFAULT_CAP, DEFAULT_TTL_SECS};
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use event::{
    canonical_id, SignetAuthEvent, SignetCredential, AUTH_EVENT_KIND, CREDENTIAL_KIND,
};
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use verify::{
    extract_display_name, schnorr_verify_bip340, verify_auth_event, verify_credential,
    VerifyResult, AUTH_EVENT_SKEW_SECS,
};
pub use wire::{SignetAuthEventWire, SignetCredentialWire};
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use wire::WireError;

// Phase 4 (2026-06-16): the `USE_SIGNET_AUTH` compile-time switch is RETIRED.
// The server now verifies an `auth_event` whenever one is present (a tampered or
// invalid event is always rejected) and rejects an *absent* auth only on a
// sign-in-required server (`HostedServer.require_signin`, set true for the QUIC
// LAN host). See `hosted_server::resolve_join_identity`. The old binary gate is
// gone; identity is policy-driven, not flag-driven.

/// The `["origin", …]` value a joiner signs into its kind-21236 join auth event,
/// and the value the host requires. Protocol v63 (audit fix B).
///
/// BOTH sides build it from their OWN transport — the client never signs an
/// origin the server supplied. With a channel binding (the QUIC connection's TLS
/// keying-material exporter, `network::channel_binding_of`) it is
/// `axenstax-join:tls-exporter:<64 lowercase hex>`; without one (WebSocket,
/// in-process channel) it is `axenstax-join:unbound`.
///
/// - **Relay:** a malicious host M that forwards a real host H's challenge to a
///   victim V gets V's signature over the V↔M exporter; H computes the H↔M
///   exporter, so the relayed event fails the origin check. This holds even
///   though certificate verification is skipped — the two TLS sessions still
///   have different keys.
/// - **Web-login oracle:** the `axenstax-join:` scheme can never equal an
///   `https://` website origin, so a join signature can never double as a
///   website sign-in, on any transport.
///
/// Residual: WebSocket TLS terminates at the reverse proxy, so there is no
/// end-to-end binding there (`unbound`) and a relay is not detected on that
/// transport. The oracle is still closed.
pub fn join_origin(binding: Option<[u8; 32]>) -> String {
    match binding {
        Some(b) => format!("{JOIN_ORIGIN_BOUND_PREFIX}{}", hex::encode(b)),
        None => JOIN_ORIGIN_UNBOUND.to_string(),
    }
}

/// Prefix of a channel-bound join origin; followed by 64 lowercase hex chars.
pub const JOIN_ORIGIN_BOUND_PREFIX: &str = "axenstax-join:tls-exporter:";
/// Join origin for a transport with no end-to-end channel binding.
pub const JOIN_ORIGIN_UNBOUND: &str = "axenstax-join:unbound";

#[cfg(test)]
mod join_origin_tests {
    use super::join_origin;

    #[test]
    fn unbound_transport_gets_the_unbound_origin() {
        assert_eq!(join_origin(None), "axenstax-join:unbound");
    }

    #[test]
    fn bound_transport_gets_the_exporter_hex_origin() {
        let mut b = [0u8; 32];
        b[0] = 0xab;
        b[31] = 0x01;
        assert_eq!(
            join_origin(Some(b)),
            format!("axenstax-join:tls-exporter:ab{}01", "00".repeat(30))
        );
    }

    #[test]
    fn join_origin_can_never_be_a_web_origin() {
        // The web-login oracle: a join signature must never double as a
        // website login, whatever the binding.
        for o in [join_origin(None), join_origin(Some([0xff; 32]))] {
            assert!(o.starts_with("axenstax-join:"), "{o}");
            assert!(!o.starts_with("https://") && !o.starts_with("http://"), "{o}");
        }
    }
}

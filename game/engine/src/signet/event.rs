//! Signet auth + credential event types. Wire shape matches Nostr NIP-01.
//!
//! Reference impl: `tools/sites/game/auth.py:_nostr_event_id` (line 55).
//! Cross-impl parity is locked by `tests/canonical_id_matches_python`.
//!
//! Phase 1 keeps these as plain in-memory types — no `serde` derive. Phase 3
//! (wire integration with `JoinRequestPacket`) picks its own serialisation
//! scheme, likely NIP-01 JSON with hex-encoded `pubkey`/`id`/`sig`. Leaving
//! the derive off here avoids locking Phase 3 into bincode's fixed layout
//! (which doesn't support `[u8; 64]` out of the box anyway — serde arrays
//! top out at 32).

/// Signet auth event kind. Matches `AUTH_EVENT_KIND` in `tools/sites/game/auth.py`.
pub const AUTH_EVENT_KIND: u32 = 21236;

/// Handle credential event kind (display-name binding). Matches the value used
/// in `/request-access` on the website side.
pub const CREDENTIAL_KIND: u32 = 31000;

/// A signed Nostr event — the wire shape Signet produces for kind-21236
/// auth events. The server never trusts `id` or `sig` on read; `verify::*`
/// recomputes the id and runs BIP-340 verify before accepting any field.
///
/// `from_np`: Signet marks events "from a Natural Person keypair" so that
/// game servers can reject the NP-as-player path. Client-asserted on the
/// wire, authenticated by the signed parent flow on the website path
/// (§1.8.6 of the networking spec). Engine treats `true` as rejection.
#[derive(Clone, Debug)]
pub struct SignetAuthEvent {
    /// x-only Schnorr public key (32 bytes).
    pub pubkey: [u8; 32],
    /// Unix timestamp (seconds since epoch).
    pub created_at: u32,
    /// Must equal `AUTH_EVENT_KIND` (21236) for server-side verification.
    pub kind: u32,
    /// NIP-01 tags. Must include `["challenge", <hex>]` and `["origin", <str>]`.
    pub tags: Vec<Vec<String>>,
    /// Per spec must be the empty string for auth events.
    pub content: String,
    /// SHA-256 of the canonical NIP-01 serialisation. Never trusted — the
    /// verify path recomputes and compares.
    pub id: [u8; 32],
    /// BIP-340 Schnorr signature over `id`.
    pub sig: [u8; 64],
    /// Client-asserted "signed by a Natural Person keypair". Engine rejects
    /// `true` outright — NP persona leakage is the T-NP-LEAK threat.
    pub from_np: bool,
}

/// A signed kind-31000 event that binds a `display-name` tag to an auth
/// pubkey. Optional on the join path — a join with no credential falls back
/// to `"Player <short-pubkey>"` on the server side.
#[derive(Clone, Debug)]
pub struct SignetCredential {
    pub pubkey: [u8; 32],
    pub created_at: u32,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub id: [u8; 32],
    pub sig: [u8; 64],
}

/// Compute the NIP-01 canonical event id.
///
/// Canonical JSON: compact (`(",", ":")` separators, no spaces), UTF-8, no
/// `ensure_ascii`. Reference: `tools/sites/game/auth.py:_nostr_event_id`.
///
/// The cross-impl test vector in `verify::tests::canonical_id_matches_python`
/// locks the byte-for-byte output against a hash generated from the Python
/// implementation.
pub fn canonical_id(
    pubkey: &[u8; 32],
    created_at: u32,
    kind: u32,
    tags: &[Vec<String>],
    content: &str,
) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let hex_pk = hex::encode(pubkey);
    // serde_json's default output is compact with `,` and `:` separators,
    // matching Python's `separators=(",", ":")`. JSON numbers are integers
    // (no trailing `.0`), matching Python's int serialisation for
    // `created_at` and `kind`. UTF-8 is the default output encoding.
    let payload = serde_json::json!([0, hex_pk, created_at, kind, tags, content]);
    let bytes = serde_json::to_vec(&payload).expect("canonical json");

    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().into()
}

# Engine-side Signet auth — `JoinRequestPacket` carries `SignetAuthEvent`

**Status**: Phase 1 DELIVERED (2026-04-20). Phases 2 + 3 DELIVERED (2026-05-03). **Phase 4 IMPLEMENTED 2026-06-16 (protocol v49)** — handshake reorder + native + web signing bridges + verify-when-present/reject-when-absent + `player_name` demotion + `USE_SIGNET_AUTH` retired + the inspect view (full copyable npub). The "Phase 4 prerequisite — engine-side signing bridge" blocker below is now RESOLVED (native_signin + `native_join_sign_driver`; ~~web `__axenstax_sign_auth_event` + `wasm_auth::js_sign_driver`~~ [CORRECTION 2026-10-04: the web half was never finished; `window.__axenstax_sign_auth_event` is defined nowhere, and the web build is now an anonymous offline taster with no sign-in path. The signing bridge exists on native only]). Remaining = **owner live test only** (2-machine LAN bunker pair; the browser+phone web leg no longer exists). See `docs/goals/2026-06-16-phase4-signet-multiplayer-auth.md` + the "Phase 4 delivery notes" section below.
**Date**: 2026-04-20 (Phase 1) · 2026-05-03 (Phases 2+3)
**Branch**: `feat/engine-signet-auth-p2-p3` for the 2+3 work; Phase 4 will branch from main once 2+3 lands.
**Session**: Fresh — implementer should treat this doc as the only brief.
**Upstream trigger**: consumer-side wire-up landed 2026-04-20 (`407f282`) — see `docs/integrations/signet/2026-04-20-accept-hint-consumer-wire-up.md`. Phase 1 was pure crypto + types and didn't depend on it; Phases 2–4 wire into `hosted_server.rs`.

## Phase 1 delivery notes (2026-04-20)

- `game/engine/src/signet/{mod,event,verify}.rs` exists. 17 unit tests — including the cross-impl `canonical_id_matches_python` vector locked against `f0affcce500eead3b7eca8181250a8fd439e4c7b978ab4fd89627c6c9845bd51` (generated from `tools/website/auth.py:_nostr_event_id`).
- `secp256k1 = "0.29"` added as native-only dep; `sha2` and `hex` moved to workspace-wide.
- **Deviation from spec**: types drop the `#[derive(Serialize, Deserialize)]` the spec outlined. Bincode (serde in general) doesn't derive for `[u8; 64]` — arrays top out at 32. Phase 3 picks its own wire encoding (separate `wire.rs` DTOs with `Vec<u8>` for the 64-byte sig, length-validated on `TryFrom`).
- `secp256k1` crate's `sign_schnorr_no_aux_rand` is used in tests (deterministic, appropriate for reproducible test sigs). Verify path uses `verify_schnorr`.
- Phase 1 adds no on-wire behaviour change. `cargo test --bin axenstax-engine` count: 31 → 48. `./check.sh` green.

## Phase 2 + 3 delivery notes (2026-05-03)

- **Phase 2 — `ChallengeTable`** (`game/engine/src/signet/challenge.rs`, ~210 lines incl. tests). 30 s TTL, cap 128, single-use consume. `issue` returns `Option<[u8; 32]>` (None on cap exhaustion after opportunistic GC). Re-issue for an existing key always succeeds — covers reconnect inside the TTL window. 11 unit tests.
- **Phase 3 — wire fields + verify path** (`game/engine/src/signet/wire.rs`, ~210 lines; `protocol.rs` + `hosted_server.rs` extensions; helper `verify_join_signet_auth`).
  - `SignetAuthEventWire` / `SignetCredentialWire` carry the wire shape (`sig: Vec<u8>` length-validated to 64). Cross-platform — WASM clients can build them.
  - `JoinRequestPacket` gains `auth_event` + `handle_credential` (both `Option<...Wire>`); `player_name` keeps a `BRIDGE` comment for Phase 4.
  - New `ChallengePacket { nonce_hex: String }` at packet tag 50; sent once on remote-connection accept.
  - `PROTOCOL_VERSION` bumped 2 → 3 (bincode is positional — even Option-only adds need a bump).
  - `signet::USE_SIGNET_AUTH: bool = false` const guards the verify path. With the flag false, behaviour is byte-identical to Phase 1 + Phase 2 (the verify branch is unreachable). With the flag flipped true locally, both `cargo build` and the verify-path unit tests are green; restored to false on commit per the playtest boundary.
  - `verify_join_signet_auth` encapsulates the verify call: consumes the nonce, runs `verify_auth_event` against the issued challenge + `https://localhost:<port>` origin (v63: now the channel-bound `signet::join_origin`, see Phase 4 notes), optionally `verify_credential`, returns the credential handle (`display-name` tag, `None` when absent — the label ladder is `verified_display_label`, T2-8; no `Player <hex>` fallback any more). 11 unit tests cover accept + every reject reason (origin mismatch, bad sig, NP fallback, expired credential, pubkey mismatch, missing challenge, missing event, malformed sig length, replay).
  - `getrandom = "0.3"` added to cross-platform deps (was WASM-only) for the native nonce source. No new top-level crates.
- `cargo test --bin axenstax-engine` count: 117 → **149** (32 new — 11 challenge, 5 wire, 11 verify_join + replay, 5 protocol). `./check.sh` green (clippy + cargo build + trunk WASM build + bundle-size gate at 1.52 MiB brotli).
- **Phase 4 boundary**: flag stays at `false` on commit. Flipping to `true` and removing the `player_name` BRIDGE wants a multi-client regression session (locking in the on-wire change with at least two real clients). Stops here cleanly. *(Superseded — see Phase 4 delivery notes below.)*

---

## Phase 4 delivery notes (2026-06-16, protocol v48)

Implemented per `docs/goals/2026-06-16-phase4-signet-multiplayer-auth.md`. Scope: **verified identity only** (the social-graph / trust-badge layer is a separate parked design).

- **Handshake reorder** (`remote_client.rs`): only the *authenticated* path changed. A guest join still sends immediately; an authed join holds the request, waits for the server's `ChallengePacket`, runs a one-shot `SignDriverFn(nonce, origin)` off the main loop, and sends the `JoinRequest` with the signed `auth_event` once the channel delivers it. `poll()` tolerates the async round-trip and fails cleanly (never hangs) on signer error / drop. New `SignedJoin` + `JoinFlow` (AwaitingChallenge → Signing → Sent). 3 unit tests (guest-immediate, authed wait→sign→send, signer-error→Failed).
- ~~**`ChallengePacket` carries `origin`** so the client signs exactly the value the server checks (the single-use nonce already prevents cross-server replay).~~ **Superseded in protocol v63 (audit fix B, 2026-09-27).** That reasoning was wrong: a single-use nonce does not stop a malicious host from relaying a real host's challenge to a victim (join as the victim), nor from sending a website origin + CSRF challenge (web-login oracle). `ChallengePacket` now carries only `nonce_hex`. Both sides build the origin from their own transport with `signet::join_origin`: `axenstax-join:tls-exporter:<hex of the QUIC TLS exporter>` or `axenstax-join:unbound` (WebSocket / channel). The server requires an exact match. Residual: WebSocket joins are unbound (TLS ends at Caddy). See Spec 04 §1.8.1 and Spec 08 §9.0.1.
- **Server cutover** (`hosted_server.rs`): `USE_SIGNET_AUTH` **retired**. New `resolve_join_identity` policy layer over `verify_join_signet_auth` (now returns `(handle, pubkey)`): verify a present auth_event (tamper/invalid → reject), reject an absent one only when `require_signin` (true for QUIC LAN host, false for the dedicated WebSocket server until web is live-verified). Verified handle is disambiguated against present players (`disambiguate_handle` + `npub_suffix`, a readable label, NOT a security boundary) and stored with the pubkey on `ServerPlayer.{display_name, verified_pubkey}` (economy ownership). 8 new unit tests.
- **Native signer wiring** (`game_loop::native_join_sign_driver`): restored `BunkerSession` signs the auth event on a worker thread (current-thread tokio + channel, mirroring `native_signin`). Native joins sign the auth event only (no kind-31000 credential yet → the host labels them via the T2-8 naming ladder (contacts book → typed name → short npub; no hex); a relay-read credential path is a follow-up).
- **Web signing bridge** (4b — spike resolved: **NOT upstream-gated**). The page's retained signet-login signer already exposes `signEvent`, so `auth.js` adds `window.__axenstax_sign_auth_event(challenge, origin)` and `wasm_auth::js_sign_driver` adapts the JS Promise into the handshake channel. WASM JoinGame joins authed when a signer is present (`has_js_signer` gate) and stays guest otherwise (so the dedicated-server guest-boot page is unaffected).
- **Inspect view** (v49): `PlayerEventType::Joined` carries the joiner's full `npub`; the client keeps a `RemoteClient::roster` (slot → handle + npub) from `Joined`/`Left`, and `hud_ui::draw_player_inspect` renders a collapsible top-right panel (auto-shown in multiplayer, default-collapsed) listing present players with a **Copy** button on each full npub — so a specific person can be verified beyond the grindable collision suffix. Follow-ups: roster-sync so a *late* joiner sees *earlier* players (today the host sees each remote joiner — the primary verify path); native kind-31000 credential fetch (native joins are named by the T2-8 ladder — contacts book → typed name → short npub — until then).
- **Verification**: `./check.sh` green (native build + tests + clippy error-gate + wasm trunk build + bundle gate). **Owner boundary**: live 2-machine LAN with a real bunker pair (native↔native + native↔dedicated) showing a verified handle + a tampered/absent join rejected; browser+phone for the web authed path.

---

## Phase 4 prerequisite — engine-side signing bridge (gap surfaced 2026-05-08; RESOLVED 2026-06-16)

**Status**: RESOLVED — native (`game_loop::native_join_sign_driver` over `native_signer::build_join_auth_event` + a restored `BunkerSession`) and web (`auth.js` `__axenstax_sign_auth_event` + `wasm_auth::js_sign_driver`). Historical context below.

The Phase 4 cutover described below assumes the engine client (WASM in browser, native binary on desktop) can produce a kind-21236 `SignetAuthEvent` signed against the **engine server's** challenge nonce + origin. **No such path exists today.** The engine has:

- `wasm_auth.rs` — receives a verified pubkey from `auth.js` via `window.__axenstax_set_pubkey(hex)` and stores it in `WASM_PUBKEY`. Read-only handoff.
- No JS bridge for signing. `auth.js` does not expose a `__axenstax_sign_auth_event(challenge_hex, origin)` (or equivalent) that calls Signet's bunker.
- Native client has no Signet integration at all — it only knows how to call `RemoteClient::connect(addr, "Player")`.

The website's existing QR sign-in produces an auth event signed against the **website's** challenge + origin (`https://localhost:8094`). That event will fail engine verify because (a) the challenge nonce was issued by the website, not by `ChallengeTable`, and (b) the origin tag points at `:8094`, not `:7700`. Re-using the website event is **not viable** without weakening server verification (which would defeat the threat model in spec 08 §9.0.1).

If Phase 4 lands without this bridge, every LAN JoinRequest under `USE_SIGNET_AUTH=true` returns `"missing auth_event under USE_SIGNET_AUTH=true"` from `verify_join_signet_auth` — single-player still works (it doesn't go through `RemoteClient::connect`), so `./check.sh --smoke` is green, but multiplayer is broken end-to-end.

### Resolution options (size before deciding)

This gap is the **A1 gap** in `docs/foundations/2026-05-08-charter-integration-axenstax.md` §A — sign-arbitrary-kind on demand. Already documented in `docs/integrations/signet/2026-05-05-nip46-signing-bunker-upstream.md` (DRAFT) and tracked as Sentinel **AS-005**. Sentinel **D-003** (decided 2026-05-07): wait for Phase 5 (Bitcoin) lead-in. Higher-priority cross-game-shared infra takes precedence per `state.yaml`.

That decision contextualises the options:

1. **WASM JS bridge → website → Signet bunker.** Engine WASM calls a new `window.__axenstax_sign_auth_event(challenge_hex, origin)` published by `auth.js` that round-trips through the bunker pairing using `nostr-tools/nip46`'s `sign_event`. **Bounded scope**, mirrors the pattern Charter Phase 1 uses for `charter_check`. But requires the broader `sign_event` capability from the bunker upstream — that's the AS-005 / D-003 'wait' territory. Don't ship this option ahead of D-003 reversing.

2. **Wait for Charter Phase 2's `signet-nip46-client` Rust crate.** Charter spec rev. 3 §Q5 commits Forgesworn to ship a cross-platform Rust crate post-alpha exposing generic `sign_event`, `nip44_encrypt`, `charter_check`, and a `KeyCustody` trait. The generic `sign_event` is exactly what Spec 1 Phase 4 needs. Adoption window: ~6–8 weeks post-alpha. **This is the natural unblocker** — same artefact, same calendar, no AxeNStax-internal infra build.

3. **Defer Phase 4 entirely until either (1) reverses or (2) lands.** Documented retreat, not abandonment. Spec 1 Phase 4 stays gated; multiplayer Signet auth waits. Single-player and the existing alpha posture are unaffected (the BRIDGE only touches LAN multiplayer JoinRequest).

**Recommended**: option 2. AxeNStax doesn't need its own signing-bridge build; Charter Phase 2 delivers it as part of the cross-game shared crate. Track via the same calendar.

### What Phase 4 actually needs to ship

- Resolution #1 or #2 implemented to the point where the engine WASM client can produce a valid `SignetAuthEvent` per LAN JoinRequest.
- The Phase 4 test sheet pre-flight expanded to cover the bridge wiring, not just the engine-side flag flip.
- Both pre-flight and playtest verifiable on a single dev box (one host + one peer device); today's pre-flight checklist is incomplete because step "RemoteClient::connect signature updated" leaves callers with nothing to pass.

### Why this surfaced 2026-05-08

A pre-Phase-4 audit looked at `game/engine/src/wasm_auth.rs`, `tools/sites/game/static/auth.js`, and `game_loop.rs:520` (`RemoteClient::connect(addr, "Player")` — single string-name call site, no auth state available locally). No code path produces `SignetAuthEvent` on the client. The previous test sheet's "mechanical pre-flight" framing missed this.

---

## TL;DR

The engine's `JoinRequestPacket.player_name: String` is **client-asserted** (CLAUDE.md §Known technical debt, bullet 5) and is the single biggest named BRIDGE blocking multiplayer. Replace it with:

```rust
pub struct JoinRequestPacket {
    pub protocol_version: u32,
    pub auth_event: SignetAuthEvent,            // signed kind-21236
    pub handle_credential: Option<SignetCredential>,  // signed kind-31000
}
```

Server verifies the auth event (BIP-340 Schnorr + challenge + origin + optional `fromNP` reject) and derives the player's handle from the optional `kind-31000` credential. Today's `player_name` field goes away. `ServerPlayer.name` becomes presentational, derived from credential — ban identity is the persona pubkey.

This is spec'd in `docs/spec/04-networking.md §1.8.1` and threat-modelled in `docs/spec/08-security-anti-cheat.md §9.0.1` (T-NP-LEAK). The website already does this exact verification — `tools/website/auth.py:verify_signet_auth_event` is the reference implementation. The engine just needs the Rust equivalent.

Memory rule signet boundary applies: this is engine-side adoption of an existing general-purpose Nostr/Signet pattern. No Signet-side asks.

---

## Context pointers

- Spec: `docs/spec/04-networking.md §1.8` (identity design), §1.8.1 (join handshake), §1.8.6 (NP-fallback policy), §1.8.4 (BRIDGE to remove).
- Threat model: `docs/spec/08-security-anti-cheat.md §9.0.1`.
- Reference impl (Python, already shipping): `tools/website/auth.py`:
  - `verify_signet_auth_event` (line 80) — Schnorr + event-id-reconstruction.
  - `_nostr_event_id` (line 55) — canonical NIP-01 id.
  - `_schnorr_verify_raw` (line 65) — BIP-340 verify via libsecp256k1.
  - Kind = 21236, created_at ±300 s brute-force window.
- Current engine join path: `game/engine/src/hosted_server.rs:295–370` (JoinRequest handling), `src/protocol.rs:41–46` (`JoinRequestPacket`), `src/remote_client.rs:43–52` (client-side emit).
- Memory: signet boundary — this change is consumer-side adoption; no Signet-side asks.

---

## Scope

Four phases. The workspace is large (Rust engine) but each phase is independently testable. Phase 1 drops types + verification behind `cargo test` with no behaviour change; Phase 4 flips the on-wire packet.

| Phase | What | Files | Est. lines |
|------:|------|-------|-----------:|
| 1 | New module `signet/` with types + BIP-340 Schnorr verification, unit-tested against the same event-id test vectors as the Python impl | `game/engine/src/signet/*.rs`, `Cargo.toml` | +600 |
| 2 | Server-side per-client challenge nonce table (TTL <60 s, bounded) | `game/engine/src/signet/challenge.rs`, `hosted_server.rs` | +120 |
| 3 | `JoinRequestPacket` grows the new fields in parallel (`player_name` stays, both carried) — behind `const USE_SIGNET_AUTH: bool`. Verification wired into `hosted_server.rs` behind the flag. Only the local two-player path flips the flag to true initially. | `protocol.rs`, `hosted_server.rs`, `remote_client.rs` | +180 |
| 4 | Remove `player_name` + the BRIDGE + the `USE_SIGNET_AUTH` flag; bump `PROTOCOL_VERSION`; update CLAUDE.md debt list; update specs §1.8.4 (delete BRIDGE section) and §9.0.1 (mark T-NP-LEAK mitigated) | `protocol.rs`, `hosted_server.rs`, `remote_client.rs`, `CLAUDE.md`, `docs/spec/04-networking.md`, `docs/spec/08-security-anti-cheat.md` | −30 / +40 |

**Total**: ~950 net lines. One new crate dep (`secp256k1`) plus `sha2` (already in Cargo.toml via `chacha20poly1305 → sha2`). No protocol changes until Phase 4 — earlier phases are additive.

---

## Phase 1 — Types, crypto, module layout

### File: `game/engine/Cargo.toml`

Add to `[dependencies]`:

```toml
secp256k1 = { version = "0.29", default-features = false, features = ["std", "recovery", "alloc"] }
```

Verify: `secp256k1` supports BIP-340 Schnorr in the `schnorrsig` module. If the crate feature surface differs from expectations, fall back to `k256 = { version = "0.13", features = ["schnorr"] }` which ships a Rust-native Schnorr. Pick one. Document the choice in the module header.

Also move `sha2 = "0.10"` from WASM-only to workspace-wide — server verification runs on native, the crate is tiny (~80 KB when pulled in natively).

### File: `game/engine/src/signet/mod.rs` (new)

Module root. Exposes:

```rust
pub mod event;        // SignetAuthEvent, SignetCredential, canonical id
pub mod verify;       // schnorr_verify_bip340, verify_auth_event, verify_credential
pub mod challenge;    // server-side nonce table (Phase 2)

pub use event::{SignetAuthEvent, SignetCredential, KeypairKind, AUTH_EVENT_KIND, CREDENTIAL_KIND};
pub use verify::{VerifyResult, verify_auth_event, verify_credential};
```

### File: `game/engine/src/signet/event.rs` (new)

```rust
//! Signet auth + credential event types. Wire shape matches Nostr NIP-01.

use serde::{Deserialize, Serialize};

pub const AUTH_EVENT_KIND: u32 = 21236;
pub const CREDENTIAL_KIND: u32 = 31000;

/// Wire shape of a signed Nostr event. The server-verification flow never
/// trusts this struct's `id` field without recomputing it (`canonical_id`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignetAuthEvent {
    pub pubkey: [u8; 32],           // x-only schnorr pubkey
    pub created_at: u32,
    pub kind: u32,                  // must be AUTH_EVENT_KIND
    pub tags: Vec<Vec<String>>,     // must include ["challenge", <hex>] and ["origin", <str>]
    pub content: String,            // "" per spec
    pub id: [u8; 32],               // SHA-256 of canonical serialisation
    pub sig: [u8; 64],              // BIP-340 Schnorr over id
    pub from_np: bool,              // §1.8.6 — client-asserted but HMAC'd by signed parent flow on the website path
}

/// Handle credential (kind-31000).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignetCredential {
    pub pubkey: [u8; 32],
    pub created_at: u32,
    pub kind: u32,                  // must be CREDENTIAL_KIND
    pub tags: Vec<Vec<String>>,     // includes ["display-name", <str>] and ["expires", <unix-ts>]
    pub content: String,
    pub id: [u8; 32],
    pub sig: [u8; 64],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeypairKind { NaturalPerson, Persona, ExtraPersona }

/// Compute the NIP-01 canonical event id.
///
/// Serialisation: JSON array `[0, pubkey, created_at, kind, tags, content]`
/// with compact separators (`,` and `:`), UTF-8, no ensure_ascii. SHA-256.
/// Reference impl: `tools/website/auth.py:_nostr_event_id` (line 55).
pub fn canonical_id(
    pubkey: &[u8; 32],
    created_at: u32,
    kind: u32,
    tags: &[Vec<String>],
    content: &str,
) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let hex_pk = hex::encode(pubkey);
    // Build the JSON payload deterministically. serde_json's default is
    // already compact+UTF-8; confirm separators match Python's (",",":") form.
    let payload = serde_json::json!([0, hex_pk, created_at, kind, tags, content]);
    // json!() default is compact. If in doubt, serialize explicitly with
    // the same settings.
    let bytes = serde_json::to_vec(&payload).expect("canonical json");
    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().into()
}
```

Add `hex = "0.4"` to Cargo.toml if not present — tiny, ubiquitous.

### File: `game/engine/src/signet/verify.rs` (new)

```rust
//! BIP-340 Schnorr verification + auth-event binding checks.
//!
//! Reference: `tools/website/auth.py:verify_signet_auth_event` (line 80).

use super::event::{AUTH_EVENT_KIND, CREDENTIAL_KIND, SignetAuthEvent, SignetCredential, canonical_id};

/// Tolerance (seconds either side of `now`) for `created_at` — matches the
/// website's AUTH_EVENT_CREATED_AT_SKEW. Phones drift.
pub const AUTH_EVENT_SKEW_SECS: u32 = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyResult {
    Ok,
    BadPubkey,
    BadSignature,
    BadEventId,          // recomputed id doesn't match
    WrongKind,
    WrongChallenge,      // tag mismatch
    WrongOrigin,
    ExpiredCredential,
    NpFallbackRejected,  // fromNP=true — §1.8.6
    CredentialPubkeyMismatch,
}

/// BIP-340 Schnorr verify. Thin wrapper over `secp256k1`.
pub fn schnorr_verify_bip340(pubkey: &[u8; 32], msg32: &[u8; 32], sig: &[u8; 64]) -> bool {
    // (Implementation uses secp256k1::schnorrsig. Match exactly the Python
    // semantics in _schnorr_verify_raw (auth.py:65).)
    unimplemented!()
}

/// Verify an auth event against a challenge + origin the server issued.
pub fn verify_auth_event(
    event: &SignetAuthEvent,
    expected_challenge: &str,
    expected_origin: &str,
    now_ts: u32,
) -> VerifyResult {
    if event.kind != AUTH_EVENT_KIND { return VerifyResult::WrongKind; }
    if event.content != "" { return VerifyResult::WrongKind; }
    if event.from_np { return VerifyResult::NpFallbackRejected; }

    // Tag-match: must contain ["challenge", expected_challenge] and ["origin", expected_origin]
    let mut has_challenge = false;
    let mut has_origin = false;
    for tag in &event.tags {
        match tag.as_slice() {
            [k, v] if k == "challenge" && v == expected_challenge => has_challenge = true,
            [k, v] if k == "origin"    && v == expected_origin    => has_origin    = true,
            _ => {}
        }
    }
    if !has_challenge { return VerifyResult::WrongChallenge; }
    if !has_origin    { return VerifyResult::WrongOrigin; }

    // created_at window
    let drift = now_ts.saturating_sub(event.created_at).max(event.created_at.saturating_sub(now_ts));
    if drift > AUTH_EVENT_SKEW_SECS { return VerifyResult::BadEventId; }

    // id must match canonical
    let recomputed = canonical_id(&event.pubkey, event.created_at, event.kind, &event.tags, &event.content);
    if recomputed != event.id { return VerifyResult::BadEventId; }

    // Schnorr verify over the id
    if !schnorr_verify_bip340(&event.pubkey, &event.id, &event.sig) {
        return VerifyResult::BadSignature;
    }
    VerifyResult::Ok
}

/// Verify a credential binds to an auth pubkey and hasn't expired.
pub fn verify_credential(
    cred: &SignetCredential,
    auth_pubkey: &[u8; 32],
    now_ts: u32,
) -> VerifyResult {
    if cred.kind != CREDENTIAL_KIND { return VerifyResult::WrongKind; }
    if &cred.pubkey != auth_pubkey { return VerifyResult::CredentialPubkeyMismatch; }

    // Expires tag
    for tag in &cred.tags {
        if let [k, v] = tag.as_slice() {
            if k == "expires" {
                if let Ok(ts) = v.parse::<u32>() {
                    if ts <= now_ts { return VerifyResult::ExpiredCredential; }
                } else {
                    return VerifyResult::ExpiredCredential;
                }
            }
        }
    }

    let recomputed = canonical_id(&cred.pubkey, cred.created_at, cred.kind, &cred.tags, &cred.content);
    if recomputed != cred.id { return VerifyResult::BadEventId; }
    if !schnorr_verify_bip340(&cred.pubkey, &cred.id, &cred.sig) {
        return VerifyResult::BadSignature;
    }
    VerifyResult::Ok
}

/// Extract the display-name tag value. None if missing.
pub fn extract_display_name(cred: &SignetCredential) -> Option<&str> {
    cred.tags.iter().find_map(|tag| match tag.as_slice() {
        [k, v] if k == "display-name" => Some(v.as_str()),
        _ => None,
    })
}
```

### Phase-1 unit tests (`src/signet/tests.rs` — `#[cfg(test)] mod tests`)

Put tests in a `tests` module within `verify.rs`. **At minimum**:

1. `valid_event_accepted` — hand-construct an event from a fixed test vector (copy one from the Python test corpus or generate one against a known keypair), verify passes.
2. `wrong_challenge_rejected`
3. `wrong_origin_rejected`
4. `bad_signature_rejected`
5. `wrong_kind_rejected`
6. `too_old_rejected` (created_at > skew)
7. `too_new_rejected` (created_at > now + skew)
8. `np_fallback_rejected` (from_np=true)
9. `canonical_id_matches_python` — hard-code a known-good pubkey/tags/content and a known-good id produced by the Python impl; verify the Rust `canonical_id` agrees byte-for-byte. **This test vector is load-bearing** — cross-impl consistency is the whole point.
10. `credential_pubkey_mismatch_rejected`
11. `credential_expired_rejected`
12. `extract_display_name_found/missing`

Capture the test vector by running the Python side once:

```bash
cd tools/website && .venv/bin/python -c '
import auth, json
tags = [["challenge", "a"*64], ["origin", "https://localhost:8094"]]
id_hex = auth._nostr_event_id("b"*64, 1_700_000_000, 21236, tags, "")
print(id_hex)'
```

Lock that hex into the Rust test so the next time someone reimplements `canonical_id`, the cross-impl check fires.

### Acceptance — Phase 1

- `cargo test --bin axenstax-engine` — all new tests green.
- `./check.sh` still green (no on-wire behaviour change yet).
- `cargo clippy` adds no new warnings against the new module.

---

## Phase 2 — Challenge nonce table

### File: `game/engine/src/signet/challenge.rs` (new)

Server issues a per-client nonce at connect time; client signs it in the auth event. Short TTL defends against replay. Table is bounded to cap memory (DoS).

```rust
use std::collections::HashMap;
use std::time::Instant;

pub struct ChallengeTable {
    entries: HashMap<String, (Instant, [u8; 32])>,
    cap: usize,
    ttl: std::time::Duration,
}

impl ChallengeTable {
    pub fn new(cap: usize, ttl_secs: u64) -> Self {
        Self { entries: HashMap::with_capacity(cap), cap, ttl: std::time::Duration::from_secs(ttl_secs) }
    }

    /// Issue a fresh challenge for a client (keyed by transport addr or slot).
    /// Returns the 32-byte nonce (caller sends to client as hex).
    pub fn issue(&mut self, client_key: String) -> [u8; 32] { /* ... */ }

    /// Consume a challenge on successful auth. Returns None if not present or expired.
    pub fn consume(&mut self, client_key: &str) -> Option<[u8; 32]> { /* ... */ }

    pub fn gc(&mut self) { /* drop expired */ }
}
```

**Bound**: TTL = 30 s, cap = 128 entries. Above cap, reject new issue requests with a 429-equivalent log line. Matches the pattern in `tools/website/auth.py:_sessions` cap (MAX_SESSIONS = 100).

### Wiring

`HostedServer::start` creates one `ChallengeTable`. On new transport connection (before JoinRequest), issue a challenge and send via a new server-initiated packet:

```rust
pub enum PacketType {
    // ...
    Challenge = 50,  // Server → Client on connect, payload = ChallengePacket
}

pub struct ChallengePacket {
    pub nonce_hex: String,  // 64 hex chars
}
```

The client emits its signed auth event in `JoinRequestPacket` with that nonce as the `challenge` tag. Server `consume`s on JoinRequest.

### Acceptance — Phase 2

- Unit tests: issue + consume + expire + GC + cap-overflow.
- Feature flag still off — no integration with JoinRequest yet.

---

## Phase 3 — Wire the new fields behind a flag

### File: `game/engine/src/protocol.rs`

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinRequestPacket {
    pub protocol_version: u32,
    pub player_name: String,  // BRIDGE — removed in Phase 4
    pub auth_event: Option<crate::signet::SignetAuthEvent>,       // Phase 3: optional
    pub handle_credential: Option<crate::signet::SignetCredential>, // Phase 3: optional
}
```

**Protocol compat**: adding `Option<T>` to a bincode struct is NOT backwards-compatible — bincode is position-serialised, not self-describing. Bump `PROTOCOL_VERSION` now and add a version-match check in the handshake. Clients and servers at the same version handle the new fields; mismatched versions are rejected by the existing version check.

### File: `game/engine/src/hosted_server.rs`

Alongside the existing `player_name` length/ctrl-char checks, if `USE_SIGNET_AUTH` is true:

1. Look up the issued nonce for transport slot `i` in `ChallengeTable`.
2. Call `signet::verify_auth_event(&req.auth_event.unwrap(), &nonce_hex, &signet::join_origin(transport.channel_binding()), now_ts())` (v63; the `server_origin` field is gone).
3. Reject with `JoinRejectPacket` on any non-`Ok` result. Log the variant.
4. If a credential is attached, call `signet::verify_credential`. Reject on failure.
5. Extract handle via `signet::extract_display_name`. (2026-10-06, T2-8: the `"Player <short_pubkey>"` fallback is gone; the naming ladder contacts book → credential → typed name → short npub lives in `hosted_server::verified_display_label`, see Spec 04 §1.8.1 step 3.)
6. Store `pubkey` on `ServerPlayer` (new field). Handle derives from credential.

### Feature flag

Add a compile-time const in `signet/mod.rs`:

```rust
pub const USE_SIGNET_AUTH: bool = false;  // Phase 4 flips to true then deletes the flag
```

Phase 3 lands with the flag false; Phase 4 flips on and deletes both branches of the conditional.

### Acceptance — Phase 3

- With `USE_SIGNET_AUTH = false`, `./check.sh` green and existing behaviour identical (regression scope: none).
- With `USE_SIGNET_AUTH = true` (flip locally and re-run), a synthetic test harness (see Spec 3 — test-harness) can:
  - Issue a challenge.
  - Sign a kind-21236 event against it with a known keypair.
  - Submit a JoinRequest and see it accepted.
  - Submit a JoinRequest with a tampered event (bad sig, wrong challenge, wrong origin, expired credential, fromNP) and see it rejected with the correct `VerifyResult`.

---

## Phase 4 — Cut over and remove the BRIDGE

> **Blocked** on the engine-side signing bridge — see "Phase 4 prerequisite" earlier in this doc. Without that bridge, the LAN JoinRequest path under `USE_SIGNET_AUTH=true` fails for every client. The mechanical steps below are correct *given* the bridge exists; they are not a complete Phase 4.

- Set `USE_SIGNET_AUTH = true`, delete the flag, delete the `player_name` field, delete the ctrl-char/length checks on the old field.
- Bump `PROTOCOL_VERSION` (second bump since Phase 3 — this one is the real on-wire change).
- `remote_client.rs:43` (`connect`) changes signature: `connect(addr: SocketAddr, auth: SignetAuthEvent, credential: Option<SignetCredential>)`. Callers build the auth event from the Signet website handoff (the same kind-21236 event that `tools/website/auth.py` verifies today).
- Update `docs/spec/04-networking.md §1.8.4` — delete the BRIDGE section; the code matches spec.
- Update `docs/spec/08-security-anti-cheat.md §9.0.1` — mark T-NP-LEAK "mitigated" (still defence-in-depth; server-reject is the authority).
- Update `CLAUDE.md` "Known technical debt" → move the `player_name` bullet to "Resolved".

### Acceptance — Phase 4

- `./check.sh` + `./check.sh --smoke` green.
- `cargo test` green, including the new Phase-1 cross-impl test vector.
- Manual: spin up a local HostedServer, drive a JoinRequest end-to-end using a keypair from a local Signet-app. See join accepted. Flip fromNP in the event → see JoinReject with a sensible reason.

---

## Global acceptance

- `./check.sh` green at the end of every phase.
- `cargo test` coverage ≥ 12 new tests in `signet::verify`.
- Cross-impl test vector matches Python `tools/website/auth.py:_nostr_event_id`.
- Spec §1.8.4 BRIDGE is deleted when Phase 4 lands.
- No client-asserted handle field remains on the wire.

---

## Non-goals

- **Multiplayer session orchestration.** Auth is one piece; server discovery, Agones fleets, matchmaking are out of scope.
- **Handle staleness re-fetch from relay.** §1.8.2 describes post-alpha periodic relay re-query; not covered here.
- **LN wallet / payout flows.** Completely orthogonal.
- **Website work.** The website already does this; engine catches up. No Signet-side asks (feedback_signet_boundary.md).
- **WebRTC path.** The Rust engine's WebRTC transport (for web multiplayer) uses the same `JoinRequestPacket` — no separate wiring. But actual WebRTC transport wiring stays out of this doc.

---

## Memory rules that apply

- signet boundary — this is engine-side adoption of an already-general-purpose Signet pattern. Don't propose changes to Signet from this work.
- pretest check — verify the Python reference impl against the Rust impl (cross-impl test vector) before claiming Phase 1 done.
- signet boundary also means: if the implementer finds they need Signet to emit something new, stop and re-read.

---

## What "done" looks like

- Four phases merged (one branch, four commits, or four PRs — implementer's call).
- `./check.sh` + `--smoke` green.
- Cross-impl test vector locked.
- `JoinRequestPacket.player_name` deleted everywhere.
- `docs/spec/04-networking.md §1.8.4` BRIDGE section deleted.
- `CLAUDE.md` debt list updated.
- This doc's status flipped from `READY TO BUILD` to `DELIVERED` with commit links.

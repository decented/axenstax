# Heartwood-signed server identity (Track 2 — the keystone)

**Date:** 2026-06-17
**Status:** BUILT 2026-06-17 (Track 2 keystone landed on branch
`feat/heartwood-signed-server-identity`; module `game/engine/src/server_identity/`,
CLI `--pair-server`/`--refresh-delegation`/`--show-connect`, 28 unit/integration
tests green via `check.sh`). **Remaining = owner boundary**: pairing against a real
Heartwood Pi + a live cross-machine join (the latter needs Track 3). Kind `30420`
still to be registered in `forgesworn/nips`.
**Scope:** Track 2 of the "signed dedicated server" vision. This spec covers **only** the
server-identity keystone: a dedicated server obtaining a verifiable, operator-backed
signing identity and being able to sign with it. The capabilities that consume it
(join-time server authentication, access policy, operator admin control plane, signed
authoritative claims, NAS/VPS deployment) are separate tracks, summarised at the end.

---

## 1. Motivation

The dedicated server (`tools/dedicated-server/` + `axenstax-engine --server`) today has **no
identity**. Nothing proves *who runs a server* or that it is the server it claims to be.
Players join as anonymous guests with client-asserted names (`require_signin = false` for the
WebSocket transport, `hosted_server.rs:286`).

We want self-hosted servers — on a NAS or a small VPS — to carry a **verifiable operator
identity rooted in a Heartwood signing device** (`<workspace>/forgesworn/heartwood/`, a NIP-46
Nostr bunker on a ~£24 Raspberry Pi / ESP32 HSM, keys never leave the device). That identity
is the foundation everything else (trust, join-auth, admin auth, signed claims) builds on.

### Vision decomposition (context only — not built here)

| Track | What | Depends on |
|---|---|---|
| 1 — Deployment — **BUILT 2026-06-17 (config)** | VPS-with-domain ACME (`AXENSTAX_DOMAIN` → Caddyfile.domain, real Let's Encrypt) + arch-aware image (Caddy per `TARGETARCH`, arm64 NAS). Self-contained cross-arch buildx (compile engine in Docker) = documented next step (owner-verify on hardware). Config-only; verified by `bash -n` + review | nothing (orthogonal) |
| **2 — Server identity (THIS SPEC)** | Server pairs to a Heartwood, gets a delegated runtime key, can sign | `signet-nip46-client` (vendored) |
| 3 — Operator identity & join-auth — **BUILT 2026-06-17** | Server presents a verifiable join proof (protocol v50); native client verification + Join-dialog pinning = owner-boundary | Track 2 |
| 4 — Access policy — **BUILT 2026-06-17 (sign-in + allowlist)** | `AXENSTAX_REQUIRE_SIGNIN` + operator npub allowlist in `resolve_join_identity` (a whitelist implies sign-in). **Age gates deferred** — need a Signet age credential | player Signet auth (Phase 4, built) |
| 5 — Operator admin control plane — **BUILT 2026-06-17** | Operator-signed commands (kind 27422; whitelist-add/remove, require-signin) verified against the operator pubkey, applied to on-disk policy via `--admin`, live within ~5s. Relay delivery of commands = owner-boundary | Track 2 + Track 4 |
| 6 — Signed authoritative claims — **BUILT 2026-06-17 (primitive)** | `server_identity::claim` sign_claim/verify_claim (kind 27421; runtime-key-signed, chains to operator). Wiring specific emitters (score/payout/integrity) into the economy paths = per-claim product decision | Track 2 |

Track 4's "whitelist" is **operator-controlled per-server policy** — *not* a revival of the
platform-wide access whitelist deliberately stripped on 2026-05-09 (see memory
`project_alpha_open_access`).

---

## 2. Decisions (locked)

1. **Bunker-agnostic.** Any NIP-46 signer works (Heartwood, Signet, Amber, nsec.app, a
   self-hosted signer). Heartwood is the **recommended** hardware root. The vendored
   `signet-nip46-client` is already generic — nothing here couples to one bunker product.
   Consistent with the Signet-boundary and shared-cross-game-infra principles.
2. **Delegated server runtime key.** The operator's real (master/persona) key never reaches
   the box. The server holds a disposable, rotatable **runtime keypair**; the operator's
   Heartwood signs a **delegation attestation** over that runtime pubkey **once**. The server
   then signs locally with no per-signature round-trips — fast, offline-capable, revocable.
3. **CLI pairing.** A one-time `axenstax-engine --pair-server` subcommand performs the NIP-46
   pairing on the headless box and persists the result on the worlds volume. No new web
   service (that would be Track 5).
4. **Connect-string anchors trust.** A server is shared as a string/QR embedding the operator
   npub. The attestation is nonetheless a **publishable Nostr event**, so a directory (Track 3)
   is a later drop-in with no format change.
5. **Additive / optional enforcement.** Unprovisioned or expired ⇒ the server boots
   anonymously exactly as today (signing is a trust *upgrade*, not a gate). An opt-in
   `AXENSTAX_REQUIRE_VERIFIED=1` flag makes a missing/expired attestation a hard startup
   failure for operators who want the stronger guarantee.

### Approaches considered & rejected

- **Live bunker signing** (round-trip every signature to the Heartwood over Tor): maximal
  security but too slow for per-join / per-claim hot paths and dies when the Pi is offline.
  Rejected for the hot path; the *pairing* step is the only live round-trip we keep.
- **Server holds a derived persona nsec** (Heartwood provisions the real persona private key
  onto the box): simplest, but puts a real private key on a VPS/NAS, against Heartwood's
  "keys never leave the device" model and hard to revoke. Rejected.

---

## 3. Architecture

```
  Operator's phone/Heartwood            The dedicated server (NAS / VPS)
  ───────────────────────────          ──────────────────────────────────────
                                        runtime keypair (generated, 0600)
   1. scan nostrconnect:// QR  ◀──────  --pair-server prints URI + QR
   2. approve on Heartwood
   3. Heartwood signs the      ──────▶  delegation attestation (Nostr event)
      delegation attestation            stored under <worlds>/.identity/
                                        + PersistedSession (for silent renewal)

  Runtime (every boot, no Heartwood needed):
    load runtime key + attestation → ServerIdentity
    sign_challenge / sign_event LOCALLY with the runtime key
    verify chain:  runtime key  →[attestation signed by]→  operator npub
```

The only time the Heartwood is involved is **pairing** and **renewal** (`--refresh-delegation`,
which reuses the restored bunker session — no fresh approval needed if the session is valid).

### New module: `game/engine/src/server_identity.rs`

Self-contained, native-only (`#![cfg(not(target_arch = "wasm32"))]`), so it lifts cleanly to
other games. Public surface:

```rust
pub struct ServerIdentity {
    runtime: Keys,                 // server runtime keypair (secret held locally, 0600)
    attestation: Option<Attestation>,
}

pub struct Attestation {           // parsed view of the operator-signed Nostr event
    pub event: Event,              // the raw signed event (republishable as-is)
    pub operator: PublicKey,       // = event.pubkey
    pub server_pubkey: PublicKey,  // = runtime.public_key(); must match `d` tag
    pub valid_from: Timestamp,
    pub valid_until: Timestamp,
    pub allowed_kinds: Vec<Kind>,
    pub server_name: String,
    pub host_hint: Option<String>,
}

impl ServerIdentity {
    pub fn load(dir: &Path) -> io::Result<Option<Self>>;           // None if unpaired
    pub fn generate_runtime(dir: &Path) -> io::Result<Self>;       // fresh key, no attestation
    pub fn store_attestation(&mut self, dir: &Path, ev: Event) -> Result<(), IdentityError>;

    pub fn sign_challenge(&self, nonce: &[u8], origin: &str) -> Signature;  // Track 3
    pub fn sign_event(&self, unsigned: UnsignedEvent) -> Event;            // Track 6
    pub fn attestation(&self) -> Option<&Attestation>;
    pub fn operator_npub(&self) -> Option<String>;
    pub fn is_verified(&self, now: Timestamp) -> bool;             // attestation present + in window + covers server key
}
```

Verification of an attestation (`Attestation::verify`) checks, in order: the event signature is
valid under `operator`; `d`/`p` tag == the runtime pubkey; `now ∈ [valid_from, valid_until]`;
the event kind and tag schema match the protocol version. Pure function over `(Event,
expected_server_pubkey, now)` ⇒ trivially unit-testable.

### Pairing (uses the vendored `signet-nip46-client`)

`signet_nip46_client::BunkerSession` already provides `pair_nostrconnect(...)`, `restore(...)`,
`persist()`, `user_public_key()`, and implements `NostrSigner`. Pairing flow:

1. `ServerIdentity::generate_runtime(dir)` if no runtime key yet.
2. `BunkerSession::pair_nostrconnect(relays, opts)` → emit the `nostrconnect://` URI; render it
   as a terminal QR (reuse the lobby/`bunker_pair.rs` QR helper) and print the raw URI as a
   fallback. Operator approves on their Heartwood.
3. Build the **unsigned** delegation event (kind, tags, validity window) for the runtime pubkey.
4. `bunker.sign_event(unsigned)` → the operator's Heartwood signs it → the **attestation**.
5. `identity.store_attestation(dir, signed)`; `bunker.persist()` → write `PersistedSession`.
6. Print the connect-string (`--show-connect`).

`--refresh-delegation` repeats steps 3–5 using `BunkerSession::restore(persisted, opts)` — no
new approval if the session is still authorised; falls back to full `--pair-server` if restore
fails.

### CLI surface (native, dispatched in `main.rs` before the event loop, like `--server`)

| Command | Effect |
|---|---|
| `--pair-server` | Interactive pairing; writes runtime key + attestation + session to the identity dir; prints the connect-string. |
| `--refresh-delegation` | Silent renewal via the restored session; falls back to `--pair-server` on failure. |
| `--show-connect` | Print `axenstax://<host>:<port>#op=<npub>` (host from `AXENSTAX_PUBLIC_HOST` or detected; npub from the stored attestation). |
| `--server` (existing) | On boot: load `ServerIdentity`; log operator npub + "verified" or "anonymous"; honour `AXENSTAX_REQUIRE_VERIFIED`. |

### Config (env, Docker-friendly; CLI `--flag` overrides where sensible)

| Var | Default | Meaning |
|---|---|---|
| `AXENSTAX_IDENTITY_DIR` | `<AXENSTAX_WORLDS_DIR>/.identity` | where the runtime key, attestation, session live (volume-persisted) |
| `AXENSTAX_PAIR_RELAY` | `wss://relay.trotters.cc` | relay used for the NIP-46 pairing round-trip (our own infra) |
| `AXENSTAX_PUBLIC_HOST` | _(detected)_ | host:port advertised in the connect-string |
| `AXENSTAX_REQUIRE_VERIFIED` | `0` | `1` ⇒ refuse to start without a valid, non-expired attestation |
| `AXENSTAX_DELEGATION_DAYS` | `90` | validity window minted at pairing time |

---

## 4. The delegation attestation (wire format)

A **parameterized-replaceable** Nostr event signed by the operator (so renewing republishes and
revoking republishes-expired). Provisional **kind `30420`** — to be registered in
`forgesworn/nips` before this leaves alpha (the exact number is not load-bearing for the design;
it is fixed here so there is no placeholder).

```jsonc
{
  "kind": 30420,
  "pubkey": "<operator pubkey hex>",          // the Heartwood-held identity
  "created_at": 1718...,
  "tags": [
    ["d", "<server runtime pubkey hex>"],     // replaceable key = the server identity
    ["p", "<server runtime pubkey hex>"],     // discoverable by server key
    ["valid_from", "1718..."],
    ["valid_until", "1726..."],
    ["k", "27420"], ["k", "27421"],           // allowed signing kinds (challenge, claims …)
    ["name", "Staxolottle's World"],
    ["host", "play.example.com:8080"],        // optional hint; trust is still the op npub
    ["v", "1"]                                 // attestation protocol version
  ],
  "content": "",
  "sig": "<operator signature>"
}
```

The connect-string is the canonical shareable form:
`axenstax://<host>:<port>#op=<npub>` — the `#op=` fragment is ignored by URL parsers and parsed
by us, so it degrades gracefully to the existing `ws://`/`wss://` Join handling. A bare
`ws://host:port` (no `#op=`) joins anonymously, preserving today's zero-config behaviour.

---

## 5. Error handling & cold-start

| Situation | Behaviour |
|---|---|
| `--server`, no identity dir / no attestation | Boot **anonymous**; log `server identity: anonymous (run --pair-server to add a verified identity)`. |
| `--server`, attestation expired | Boot anonymous; log a clear expiry warning naming `--refresh-delegation`. |
| `--server` + `AXENSTAX_REQUIRE_VERIFIED=1` + no valid attestation | **Refuse to start**, non-zero exit, actionable message. |
| `--pair-server`, bunker offline / timeout / declined | Clean non-zero exit; **nothing half-written** (write attestation+session atomically only on full success). |
| `--pair-server`, relay unreachable | Fail fast naming `AXENSTAX_PAIR_RELAY`. |
| Runtime key present, attestation references a *different* server pubkey | Treat as unpaired (mismatch); log and ignore the stale attestation. |

Runtime key + session files are written `0600` (mirrors the `~/.config/axenstax/` posture).
At-rest encryption of the runtime key is **out of scope** here: it is a delegated, kind-scoped,
revocable, rotatable key, and a headless server has no good passphrase-entry UX. Noted as a
possible future hardening, not a gap.

---

## 6. Testing

**Unit (pure, no hardware, run by `cargo test --bin axenstax-engine` ⇒ `check.sh`):**
- Attestation `verify`: valid; expired (now > valid_until); not-yet-valid (now < valid_from);
  wrong server pubkey in `d`; wrong operator (signature mismatch); unknown protocol version.
- `is_verified` window boundaries.
- Connect-string parse/emit round-trip (`axenstax://host:port#op=npub` ⇄ struct), plus
  graceful handling of a bare `ws://` with no `#op=`.
- Runtime key persist → load round-trip; mismatch detection.

**Integration (`src/test_integration/`):**
- An **in-process NIP-46 signer** stands in for a Heartwood (a local `Keys` acting as the
  "operator bunker"). Drives the full pair → mint attestation → store → `ServerIdentity::load`
  → `is_verified` → `sign_challenge` → verify-chain path, entirely in-process. This proves the
  keystone end-to-end with **no Pi and no relay**.

**Owner-only boundary (cannot be verified solo):**
- Pairing against a **real Heartwood Pi** (`--pair-server`, approve on device).
- A real **cross-machine** check once Track 3 lands (client verifies a live server's chain).
I will build and green everything up to this line, then stop and hand off, per the usual
playtest boundary.

---

## 7. Out of scope (explicit)

- Client-side join verification & the reversed challenge packet (Track 3).
- Any server directory / lobby "verified" badge (Track 3).
- Player sign-in gating, age gates, whitelists (Track 4).
- Operator admin control plane / admin web UI (Track 5).
- Signing economy/score/world-integrity claims (Track 6) — this spec only exposes the
  `sign_event` API they will call.
- NAS multi-arch image + VPS ACME TLS (Track 1).
- At-rest encryption of the runtime key.

---

## 8. Spec maintenance

Per `CLAUDE.md`, when this is built: register kind `30420` in `forgesworn/nips`, and reflect the
new `--pair-server`/`--refresh-delegation`/`--show-connect` surface and identity env vars in
`tools/dedicated-server/README.md` and the engine-commands docs.

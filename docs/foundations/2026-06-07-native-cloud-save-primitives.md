# Native Cloud-Save Primitives — `signet-nip46-client` + `stash-rs` (API contract)

**Status:** **API CONTRACT (design-bound). Crates implemented on `signet-nip46-client`
+ `stash-rs` (Forgesworn) per the solo native-build goal — see those repos for build
status.** This is the one step no compiler checks, so it is pinned here *before* the
code: a wrong call propagates to every consumer. The S0 eval spike
(`tools/native-build-spike/`) already proved the underlying crates compile, gate
cleanly for wasm32, sign with a local key, and round-trip a blob to Blossom.

**Source of truth:** `docs/architecture/2026-06-06-native-build-distribution-strategy.md`
(bucket 1 — Forgesworn primitives) and `docs/goals/native-build-solo.md` (Phases
S-SPEC → S3 → S4). The shape to mirror is `@forgesworn/stash`
(`<workspace>/forgesworn/stash`, `spec/protocol.md`).

---

## TL;DR

Two thin, **reusable, cross-game** Forgesworn Rust crates that give *native* builds the
cloud-save story the web already has — without any AxeNStax-specific code and without
touching the engine (that wiring is bucket 3):

- **`signet-nip46-client`** (`<workspace>/forgesworn/signet-nip46-client/`) — a thin wrapper
  over `nostr-connect` (rust-nostr) that pairs with a **NIP-46 bunker** (the "key stays
  on your phone" model Signet/Heartwood implement) and exposes **pair / sign / restore**.
  It implements `nostr::signer::NostrSigner`, so it is drop-in anywhere a signer is
  wanted. **Generic NIP-46** — the "signet" in the name is only sensible-default config
  for the Signet bunker flow; nothing here is AxeNStax- or even Signet-protocol-specific
  (Signet boundary, [[feedback_signet_boundary]]).
- **`stash-rs`** (`<workspace>/forgesworn/stash-rs/`) — the **Rust sibling of
  `@forgesworn/stash`**: save-sync orchestration that encrypts an item to the persona's
  own key (NIP-44), stores the ciphertext as a content-addressed Blossom blob, and indexes
  it in a per-persona, per-app encrypted Nostr manifest. It mirrors `@forgesworn/stash`'s
  **blob + manifest wire shapes byte-for-byte**, so a world saved on native and one saved
  on web are mutually readable once both sit on the destination tier.

Native's advantage over the web cloud-save ([[2026-05-26-cloud-save-blossom]]): web is
stuck on the **bridge tier** (server-key signing) because the `signet-login` SDK doesn't
yet expose `sign_event`/NIP-44. Native has a *real* signer (`nostr-connect`), so it goes
straight to the **destination tier** — player-key-owned blobs + NIP-44 + a Nostr-event
manifest on `wss://relay.trotters.cc` ([[reference_trotters_relay]]). No server proxy.

---

## Why these two, why now

- **The keystone for Track B** (native sign-in → cloud save → multiplayer auth). The
  expensive part was always the signer; the S0 spike confirmed it is ~90% `nostr-connect`,
  not from-scratch. `stash-rs` is then a thin compose over the spike's proven blob layer.
- **Cross-game lift.** Both are Forgesworn primitives ([[project_shared_infra_strategy]]):
  any Decented native game inherits NIP-46 sign-in and identity-keyed cloud save. Only the
  blob *contents* (the world format) are AxeNStax's; the crates never see them.
- **AxeNStax Spec-1-Phase-4 is the named demand** for `signet-nip46-client` in the Signet
  plans (`signet-plans/MESSAGE-FROM-AXENSTAX.md` §"Note from rev. 6 holodeck": "If a
  different driver … creates pull for the crate before mechanism B/C/D, this is the
  existing demand"). Building it now serves native cloud-save *and* native multiplayer auth.

---

## Pinned dependency set (from the S0 spike — do not drift)

```
nostr         = 0.44.3     nostr-blossom = 0.44.0   (rust-nostr family, shares nostr 0.44)
nostr-sdk     = 0.44.1     base64        = 0.22
nostr-connect = 0.44.1     tokio         = 1.x  (rt + macros)
```

All native-only. In any consumer (the engine, bucket 3) they live **only** under
`[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` and every reference is
`#[cfg(not(target_arch = "wasm32"))]`-gated. The spike (`tools/native-build-spike/`)
demonstrates the idiom keeps a `wasm32-unknown-unknown` build clean; the engine already
uses the same pattern (`game/engine/Cargo.toml`).

---

## Crate 1 — `signet-nip46-client` (the signer)

A thin, generic NIP-46 bunker client. Wraps `nostr_connect::NostrConnect`; adds
persistence (pair → save → restore) and Signet-flow defaults. **No AxeNStax knowledge.**

### Public surface (contract)

```rust
/// A live NIP-46 bunker session. Implements `nostr::signer::NostrSigner`, so it is a
/// drop-in signer for `EventBuilder::sign`, `nostr_blossom`, and `stash-rs`.
pub struct BunkerSession { /* wraps NostrConnect + the app keypair */ }

/// Everything needed to silently reconnect a session on a later launch — persist this
/// (e.g. an OS keystore / a file the app owns). Holds NO user key: the app keypair is a
/// throwaway NIP-46 client identity; the bunker URI carries the connect secret.
#[derive(Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub app_secret_key: String,   // hex; the NIP-46 *client* key (not the persona)
    pub bunker_uri: String,       // bunker://<remote-signer-pubkey>?relay=...&secret=...
}

#[derive(Clone)]
pub struct SessionOptions {
    pub timeout: Duration,        // default 60s
    pub relays: Vec<String>,      // optional extra relays; the URI's own relays always apply
}

impl BunkerSession {
    /// Pair with a bunker from a `bunker://` or `nostrconnect://` URI. Generates a fresh
    /// app keypair for this NIP-46 session. The first `user_public_key()`/`sign` call
    /// triggers the bunker's approval prompt (the live-phone step).
    pub fn pair(uri: &str, opts: SessionOptions) -> Result<Self, Error>;

    /// Reconnect a previously-paired session WITHOUT re-approval, by reusing the stored
    /// app keypair + bunker URI (NIP-46 session continuity).
    pub fn restore(persisted: PersistedSession, opts: SessionOptions) -> Result<Self, Error>;

    /// The data to persist for a later `restore`.
    pub async fn persist(&self) -> Result<PersistedSession, Error>;

    /// The persona pubkey this session signs as (round-trips to the bunker on first call).
    pub async fn user_public_key(&self) -> Result<PublicKey, Error>;

    /// Tear down relay connections.
    pub async fn shutdown(self);
}

// Delegates to the wrapped NostrConnect — the whole point: a BunkerSession IS a signer.
impl nostr::signer::NostrSigner for BunkerSession { /* get_public_key / sign_event /
    nip44_encrypt / nip44_decrypt */ }
```

### Design notes

- **Why `NostrSigner`, not a bespoke trait.** Both `Keys` and `NostrConnect` implement
  `nostr::signer::NostrSigner` (verified in the spike). Implementing the same trait means
  `stash-rs` is generic over *any* signer: a local `Keys` for tests, a `BunkerSession` in
  production. No adapter layer.
- **Restore semantics.** A NIP-46 session is identified by the *client* (app) keypair plus
  the bunker URI's connect secret. Persisting `{app_secret_key, bunker_uri}` and
  reconstructing `NostrConnect::new(uri, app_keys, timeout, opts)` with the **same** app
  keys lets the bunker recognise the existing grant — no re-prompt. (`NostrConnect::new`
  takes `client_keys: Keys`, so this is a direct reconstruction.)
- **Signet defaults, generic core.** Ship a `SessionOptions::default()` tuned for the
  Signet/Heartwood bunker flow (60s timeout). The crate must work with *any* NIP-46 bunker
  (nsec.app, Amber, a self-hosted Heartwood) — if a choice would only make sense for
  Signet-the-product, it has crossed the boundary.
- **Boundary:** the live relay handshake needs the owner's phone. `restore` and the
  `NostrSigner` impl are unit-testable against a local-key mock; `pair` against a real
  bunker is bucket 3 + a device (the spike's `bunker_pair.rs` is the owner harness).

---

## Crate 2 — `stash-rs` (save-sync), the Rust sibling of `@forgesworn/stash`

`stash-rs` is a thin orchestrator over a **signer** (`S: NostrSigner`), a **blob store**
(Blossom), and a **manifest store** (a Nostr replaceable event). It owns the crypto +
content-addressing + manifest read-modify-write; it never holds a key.

### The wire shapes it MUST mirror (interop-critical)

These are copied from `@forgesworn/stash` (`spec/protocol.md`, verified against
`src/blob.ts` + `src/index.ts` + `src/nostr/manifest.ts`). A drift here silently breaks
web↔native interop with no compiler warning — **pin and test against fixtures.**

**Blob** (`§4`):
```
plaintext_b64 = base64_standard(item_bytes)             // '+/' alphabet, '=' padding
ciphertext    = NIP44_encrypt_to_self(plaintext_b64)    // peer pubkey == own pubkey, v2
blob          = ciphertext.as_bytes()                    // UTF-8 of the ciphertext STRING
blob_hash     = hex(sha256(blob))                        // Blossom content address
size          = blob.len()                               // ciphertext byte length
```
The integrity rule (`§9`): on upload, the server-returned sha256 MUST equal `blob_hash`
(reject mismatch). On download, the fetched bytes MUST hash to the requested address
*before* decrypt (reject tamper/error-page).

**Descriptor** (`§5`) — the JSON that gets encrypted into a manifest entry. **Field names
are camelCase to match TS `JSON.stringify`** (`blobHash`, not `blob_hash`):
```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDescriptor {
    pub kind: String,      // "world" | "save" | "skin" | ...
    pub name: String,      // human label (encrypted at rest)
    pub blob_hash: String, // -> "blobHash"
    pub size: u64,
    pub updated: u64,      // unix seconds, last-write-wins
}
enc = NIP44_encrypt_to_self(serde_json::to_string(&descriptor))
```

**Manifest entry** (stored, `{blobHash, enc, updated}`):
```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry { pub blob_hash: String, pub enc: String, pub updated: u64 }
```

**Destination-tier manifest event** (`§5`, `src/nostr/manifest.ts`):
```
kind:       30819            // STASH_MANIFEST_KIND (placeholder; fixed by the Stash NIP)
pubkey:     the persona
tags:       [["d", app]]     // one parameterized-replaceable event per (persona, app)
content:    NIP44_encrypt_to_self( serde_json::to_string(&Vec<ManifestEntry>) )
```
Note the **double wrap**: each descriptor is individually encrypted into `enc`, then the
whole `Vec<ManifestEntry>` is JSON'd and encrypted *again* as the event content (hides even
the entry-list shape from the relay). `created_at` MUST be strictly increasing per
(persona, app) for the client's own writes; on read, break ties by `(created_at desc,
id asc)` to agree with NIP-01 relay semantics.

### Public surface (contract)

```rust
pub struct Stash<S: NostrSigner, B: BlobStore, M: ManifestStore> { /* app, signer, ... */ }

pub struct StashConfig<S, B, M> { pub app: String, pub signer: S, pub blossom: B, pub manifest: M }

impl<S, B, M> Stash<S, B, M> {
    pub fn new(cfg: StashConfig<S, B, M>) -> Self;

    /// True iff the signer can NIP-44 (always true for Keys/BunkerSession; the hook exists
    /// to mirror TS's auth-only-signer gate). Gate cloud UI on this.
    pub fn available(&self) -> bool;

    /// encrypt-to-self -> upload -> verify hash -> record in manifest. Every call uploads
    /// (NIP-44's random nonce defeats content-dedup; a caller that wants to skip unchanged
    /// items tracks the plaintext hash itself).
    pub async fn save(&self, kind: &str, name: &str, bytes: &[u8]) -> Result<ItemRef, Error>;

    /// fetch + decrypt this app's manifest -> the persona's items.
    pub async fn list(&self) -> Result<Vec<ItemDescriptor>, Error>;

    /// download by hash -> verify hash -> decrypt -> item bytes.
    pub async fn restore(&self, r: &ItemRef) -> Result<Vec<u8>, Error>;

    /// drop the descriptor from the manifest (blob left to Blossom retention).
    pub async fn remove(&self, r: &ItemRef) -> Result<(), Error>;

    /// (destination tier) every app namespace this persona owns — the cross-game view.
    pub async fn list_all_apps(&self) -> Result<HashMap<String, Vec<ItemDescriptor>>, Error>;
}

pub struct ItemRef { pub blob_hash: String, pub size: u64 }
```

### Pluggable backends (mirror TS's transport seams)

```rust
/// Content-addressed blob transport. The Blossom impl wraps `nostr_blossom::BlossomClient`.
pub trait BlobStore {
    async fn put(&self, ciphertext: &[u8]) -> Result<String, Error>; // returns server sha256 hex
    async fn get(&self, blob_hash: &str) -> Result<Vec<u8>, Error>;
}

/// Manifest index. The destination impl publishes the kind-30819 replaceable event.
pub trait ManifestStore {
    async fn record(&self, app: &str, blob_hash: &str, enc: &str, updated: u64) -> Result<(), Error>;
    async fn list(&self, app: &str) -> Result<Vec<ManifestEntry>, Error>;
    async fn remove(&self, app: &str, blob_hash: &str) -> Result<(), Error>;
    async fn list_all_apps(&self) -> Result<HashMap<String, Vec<ManifestEntry>>, Error>; // dest only
}
```

- `BlossomBlobStore` — wraps `BlossomClient`; `put` calls `upload_blob(data, Some(ct),
  None, Some(&signer))` and **verifies `descriptor.sha256 == sha256(data)`**; `get` calls
  `get_blob(hash, None, None, None::<&Keys>)` (public read) and **verifies the returned
  bytes hash to the request**. (Both checks proven in the spike.)
- `NostrManifestStore` — destination tier; takes the signer + a relay client; implements the
  read-modify-write with strictly-increasing `created_at` + the read tie-break above.
- A **bridge** `ManifestStore` (HTTP, mirroring TS `httpManifestStore` against the web's
  `/worlds/manifest`) is *optional future work* — native goes destination-first.

### Error model

```rust
pub enum Error {
    Signer(SignerError),        // wraps nostr SignerError (sign/encrypt/decrypt failures)
    Blossom(String),            // upload/download transport or integrity-check failure
    HashMismatch { expected: String, got: String }, // server stored != uploaded, or tamper
    Relay(String),              // manifest publish/query failure
    Codec(String),              // base64 / JSON / UTF-8
}
```
Mirror TS's two manifest-decrypt failure modes (`src/index.ts::decryptEntry`): a
descriptor that **fails to decrypt** is "not ours" → **skip silently** (cross-persona
isolation); one that decrypts but **fails to parse** is "ours but corrupt" → **log a
warning, skip** (never lose it silently — the blob is still recoverable by hash).

### Why generic over `S: NostrSigner` (not a hard dep on crate 1)

`stash-rs` accepts any `NostrSigner`. In tests it uses a local `Keys` (the encrypt→
manifest→decrypt logic and a **live Blossom round-trip** are fully solo-verifiable with a
generated key — exactly what the spike already proved). In production the engine passes a
`signet_nip46_client::BunkerSession`. So `stash-rs` does **not** depend on crate 1 — they
compose at the call site. End-to-end with the **live bunker** is the boundary (bucket 3 +
a device).

---

## Phased scope

| # | Phase | Crate | Solo-verifiable | Boundary |
|---|-------|-------|-----------------|----------|
| S-SPEC | This contract | docs | spec exists; manifest shape matches `@forgesworn/stash` | — |
| S3 | `signet-nip46-client`: pair/sign/restore + `NostrSigner` impl | crate 1 | `cargo test` against a local-key mock signer; `wasm32` gating note | live bunker pair (phone) |
| S4 | `stash-rs`: blob + descriptor + manifest + Blossom/Nostr stores | crate 2 | `cargo test` (offline crypto/manifest unit tests + camelCase fixture parity vs TS) **and** a live Blossom round-trip (generated key) | end-to-end with the live signer |
| (bucket 3) | engine deps + `save.rs` wiring + native sign-in UX | engine | — | owner-coordinated, real device |

### Cross-impl interop fixtures (the thing that actually catches drift)

S4 MUST include a test that:
1. Builds an `ItemDescriptor`, serialises it, and asserts the JSON string contains
   `"blobHash"`, `"size"`, `"updated"` (camelCase) — i.e. byte-compatible with what TS
   `JSON.stringify` and `JSON.parse` produce/consume.
2. Round-trips the **blob** layout (`base64 → NIP-44 → utf8 → sha256`) and asserts the
   content address equals what a faithful Blossom upload echoes back (proven live in S0).
3. Asserts `STASH_MANIFEST_KIND == 30819` and the event has exactly one `["d", app]` tag.

(If feasible cheaply, decode a fixture produced by the TS `@forgesworn/stash` test vectors;
otherwise pin the field-name assertions, which are the only place silent drift hides.)

---

## Acceptance criteria

- `signet-nip46-client` and `stash-rs` each compile; `cargo test` green offline.
- `stash-rs` has a **live Blossom round-trip** integration test (generated key + local-key
  signer): `save` → `list` → `restore` returns byte-identical bytes.
- The descriptor/manifest JSON field names are camelCase and asserted in a test; the
  manifest kind is 30819 with a single `d` tag.
- `BunkerSession` implements `NostrSigner` and has unit tests for `restore` (round-tripping
  `PersistedSession`) and the signer delegation against a local-key mock.
- Neither crate references AxeNStax, the engine, or any game content (Signet boundary +
  shared-infra rule). Neither is compiled into the wasm32 graph of any consumer.
- A short STATUS in each crate's README records what's proven vs. what needs the live
  bunker / a real device.

---

## Memory-rule check

- **[[feedback_signet_boundary]]:** `signet-nip46-client` is a *generic* NIP-46 bunker
  client; the only Signet-specific thing is default config. No Signet-internal protocol
  design, no AxeNStax repackaging. ✅
- **[[project_shared_infra_strategy]]:** both crates are Forgesworn primitives, game-
  agnostic; only blob *contents* are AxeNStax's, and the crates never see them. ✅
- **[[reference_trotters_relay]]:** the destination manifest publishes to our own trotters
  relay, not a third party. ✅
- **[[feedback_npub_only_display]]:** any user-facing rendering of the persona is the
  consumer's job; the crates surface hex `PublicKey` internally (correct — hex stays
  internal). ✅
- **`@forgesworn/stash` parity:** blob + manifest wire shapes mirrored byte-for-byte;
  camelCase field names pinned. ✅
- **UK English** throughout. ✅

---

## Context pointers

- **Shape to mirror:** `<workspace>/forgesworn/stash/spec/protocol.md` + `src/{blob,index,
  base64}.ts`, `src/nostr/manifest.ts` (kind 30819, double-wrap, created_at ordering).
- **Proven foundation:** `tools/native-build-spike/` (S0) — pinned versions, wasm32 gating,
  local-key signing, live Blossom round-trip, the `NostrSigner` delegation pattern.
- **Web sibling / interop target:** [[2026-05-26-cloud-save-blossom]]
  (`docs/foundations/2026-05-26-cloud-save-blossom.md`) — bridge vs destination tiers; the
  web is on the bridge tier, native goes destination-first; blobs match either way.
- **Named upstream demand:** `<workspace>/forgesworn/signet-plans/MESSAGE-FROM-AXENSTAX.md`
  §"signet-nip46-client"; `docs/foundations/2026-05-14-engine-signing-bridge.md` (Spec 13,
  the WASM-side sibling of the same signing capability).
- **Strategy:** `docs/architecture/2026-06-06-native-build-distribution-strategy.md`
  (buckets 1–3); **goal:** `docs/goals/native-build-solo.md` (Phases S3/S4 + boundaries).

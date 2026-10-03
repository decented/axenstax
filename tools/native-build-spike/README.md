# native-build-spike — B0 / S0 eval spike (throwaway de-risking harness)

A deliberately standalone crate (NOT in the engine workspace, NOT touching
`game/engine/Cargo.toml`) that hardens every downstream estimate for the native
cloud-save story **before** the real Forgesworn crates are written. Executable
subset of `docs/goals/native-build-solo.md` §"Phase S0".

## What it proves (all solo-verifiable, all GREEN as of 2026-06-07)

| # | Claim | How to verify | Result |
|---|-------|---------------|--------|
| 1 | The pinned native crates compile together as one tree | `cargo build` | ✅ green |
| 2 | The `cfg(not(wasm32))` idiom keeps the WASM build clean | `cargo build --target wasm32-unknown-unknown` | ✅ green; **zero** native-crate artifacts in the wasm32 deps dir |
| 3 | nostr-sdk signs an event with a locally generated key | `cargo test` (`sign_with_local_key_verifies`) | ✅ pass |
| 4 | A blob round-trips to a public Blossom server | `cargo run --bin blossom_roundtrip` | ✅ stored + verified + decrypted byte-identical against `blossom.primal.net` |

`cargo test` is green **offline** — the live Blossom test is `#[ignore]`d so CI never
needs the network; run it explicitly with `cargo test -- --ignored` or the bin above.

### Pinned versions (the reference set for S3/S4)

```
nostr         = 0.44.3        nostr-blossom = 0.44.0   (shares the nostr 0.44 core)
nostr-sdk     = 0.44.1        tokio         = 1.x
nostr-connect = 0.44.1        base64        = 0.22
```

The rust-nostr family moves in lockstep at `0.44.x`. `nostr-blossom` (rust-nostr
family) was chosen over `blossom-rs` 0.5.x because it shares the `nostr` 0.44
types — its `BlossomClient` takes any `nostr::signer::NostrSigner` (which both
`Keys` and `NostrConnect` implement), so the signer flows straight through with no
version bridging. (`blossom-rs` has *no* `nostr` dependency and would need a
hand-built BUD-01 auth event.) The exact APIs S4 depends on:

- `BlossomClient::new(url)` · `upload_blob(data, content_type, opts, Some(&signer))
  -> BlobDescriptor{ url, sha256, size }` · `get_blob(sha256, range, opts,
  None::<&Keys>) -> Vec<u8>` (public read).
- `Sha256Hash = nostr::hashes::sha256::Hash`; `Sha256Hash::hash(&bytes)` is the
  Blossom content address — equals `descriptor.sha256` on a faithful upload.
- NIP-44: `signer.nip44_encrypt(&pubkey, &str)` / `nip44_decrypt(&pubkey, &str)`.

### The WASM-gating idiom (the whole point)

Native crates live **only** under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`
(see `Cargo.toml`) and every reference is `#[cfg(not(target_arch = "wasm32"))]`-gated.
On wasm32 the lib collapses to `shim_marker()` and both bins to empty `main()`s, so
`cargo build --target wasm32-unknown-unknown` finishes in a fraction of a second
without pulling a single native crate. This is the exact pattern the engine already
uses (`game/engine/Cargo.toml`) and the one bucket-3 integration reuses verbatim.

## The boundary (NOT solo)

**Live Signet-bunker pairing.** `nostr_connect::NostrConnect` construction +
`get_public_key()` + `sign_event()` are wired (`native::pair_bunker_and_sign`,
driven by `src/bin/bunker_pair.rs`), but the relay handshake needs the owner's
Signet app on a phone to approve the connect/sign prompts. Owner harness:

```
# On the phone: Signet → export a bunker:// URI, then:
cargo run --bin bunker_pair -- 'bunker://<pubkey>?relay=wss://relay.trotters.cc&secret=...'
```

Success prints the user npub + a bunker-signed, locally-verified event. Until then
the signer path is exercised only against generated local keys (S3's mock tier).

## Run it

```bash
cd tools/native-build-spike
CARGO_TARGET_DIR="$HOME/<workspace>/AxeNStax/build/spike" cargo test            # offline: 4 pass, 1 ignored
CARGO_TARGET_DIR="$HOME/<workspace>/AxeNStax/build/spike" cargo build --target wasm32-unknown-unknown
CARGO_TARGET_DIR="$HOME/<workspace>/AxeNStax/build/spike" cargo run --bin blossom_roundtrip
```

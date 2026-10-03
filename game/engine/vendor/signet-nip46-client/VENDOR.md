# Vendored: `signet-nip46-client`

This is a **pinned, build-time mirror** of the canonical Forgesworn crate:

- **Canonical source:** `git@github.com:forgesworn/signet-nip46-client.git`
- **Vendored at commit:** `5b85902f4655e541d4c631da5bc3257868dcc96a` (v0.1.0)
- **Vendored on:** 2026-06-10 (native-login Bucket 3 integration)

> **⚠ This vendored copy is AHEAD of the canonical crate.** The native sign-in UI
> added `BunkerSession::pair_nostrconnect(relays, app_name, opts)` — the
> client-initiated (`nostrconnect://`) QR flow (the app shows a QR, the signer
> scans it), the reverse of `pair()`. It is a generic NIP-46 capability and
> **must be upstreamed** to the canonical crate (then re-bump the commit above).
>
> **Second divergence (2026-10-01) — the `nostrconnect://` secret + handshake.**
> rust-nostr 0.44 builds a client URI with **no `secret`**, and its client accepts
> only a literal `"ack"` as the signer's `connect` reply. Current NIP-46 requires
> the `secret` in the URI and has the signer reply with that secret, so
> spec-following signers (mySignet) ignored our QR. This copy now mints a 16-byte
> CSPRNG hex secret, builds the URI itself (`relay`, `secret`, `name`, `metadata`),
> runs the client-initiated handshake itself (`await_nostrconnect_handshake` /
> `accept_connect_event`: accept ONLY a constant-time exact echo of the secret;
> a bare `"ack"` is rejected because the relay sees our app pubkey in the `#p`
> subscription and could race to impersonate the signer; a non-empty `error`
> fails the handshake at once), then hands
> the discovered signer to `NostrConnect` as a secret-less `bunker://` session.
> **Signers must echo the secret** (mySignet does; an ack-only signer will not pair
> via QR — use the `bunker://` paste path). Adds deps `nostr-relay-pool =0.44.1`,
> `tokio` (`sync`, `time`, `rt`) and `subtle`. Upstream to
> the canonical crate (and ideally to rust-nostr's `nostr-connect`), then re-sync.

## Why vendored (not a path / git dependency)

The engine's WASM bundle is rebuilt by `trunk build --release` on **every push to
`main`** (`.github/workflows/deploy.yml`, auto-deploy to the live site). That job is a
plain `actions/checkout` with **no access** to `<workspace>/forgesworn/` and **no auth** for
the private Forgesworn git remote. Cargo resolves the *entire* dependency graph — including
`[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` — before it can build *any*
target, so an unresolvable path/git dep would fail the WASM deploy even though this crate
never compiles for `wasm32`. Vendoring keeps the source in-repo and CI-safe on all targets.

This mirrors the existing `signet-login.iife.js` vendoring pattern. It does **not** change
the Signet boundary: the crate stays a generic NIP-46 bunker client; this is purely a build
mechanism.

## Re-syncing

When `signet-nip46-client` is published to crates.io (or the repo opens / CI gains auth),
replace the `path = "vendor/signet-nip46-client"` dependency in `game/engine/Cargo.toml`
with the published/`git` dependency and delete this directory. Until then, to pull upstream
fixes: re-copy `Cargo.toml` + `src/` from the canonical repo and update the commit hash
above.

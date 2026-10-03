# open-stash primitive — build notes + live-verification runbook

**Date:** 2026-06-03 · **Goal:** btc-prague-demo Goal 2 · **Status:** code built + unit-tested (mocked); **live round-trip = OWNER step** (this doc).

open-stash is the PUBLIC, unencrypted, npub-signed content store — the public sibling of the private Stash (`cloud.js`). You publish unencrypted blobs to Blossom + a public, signed manifest event to the relay; anyone who FOLLOWS your npub can list + download it. It is the delivery rail for **official** (the AxeNStax npub) AND **community** mods/scenarios. Goal 5 consumes it to deliver Hash Dash + Satori Rush.

**Key robustness property:** reading open-stash content needs only **signature verification** (Schnorr + event-id, done in JS) — NOT the NIP-44 `capable()` gate that the private cross-device Stash depends on (the demo-plan §4 P0 risk). So official/community mod delivery works even if that gate is unavailable.

## What was built (autonomous, `check.sh` green)

| Piece | Where | Tested |
|---|---|---|
| **Core (pure, testable)** — manifest shape, build, parse-another-pubkey, followed-npubs store, content typing, pubkey normalisation | `game/engine/src/open_stash.rs` | ✅ 16 unit tests (mocked) |
| **WASM transport bridge** — `publish_open_stash` / `fetch_open_stash_manifest` / `download_open_stash_blob` / `list_followed_content` | `game/engine/src/open_stash.rs` (`#[cfg(target_arch = "wasm32")]`) | compiles under `trunk` (I/O, not unit-testable) |
| **JS transport** — Blossom PUT/GET (plaintext, BUD-02 signed) + relay publish/query (raw WS Nostr, like `persona-handle.js`) + Schnorr-verified reads | `tools/sites/game/static/openstash.js` | owner live-verify |
| **Page wiring** | `tools/sites/game/templates/lobby.html` (`<script src="/static/openstash.js">` after `cloud.js`) | owner live-verify |

### Wire shape (locked)
- **Manifest event:** kind **`30820`** (parameterised-replaceable, public), `d`-tag **`axenstax-open-stash`**, plus one `t` tag per distinct content type. `content` = cleartext JSON `{ "items": [ { name, blob_hash, content_type, size } ] }`. Re-publishing supersedes (newest `created_at` wins).
- **Blob:** plaintext to Blossom; the Blossom **sha256 hex IS the item's `blob_hash`** (the download key). No encryption.
- **Type tag:** generic — `content_type: "scenario"` for Goal 5; the store bakes in nothing game-specific.
- **Followed-npubs store:** `FollowedNpubs` (hex keys, deduped, order-preserving), seeded with `official_axenstax_pubkey()`. Persist with `to_json`/`from_json` (localStorage on WASM / file on native — caller's choice).

### Engine ↔ JS contract (`window.axenstax_openstash_*`)
```
put_blob(bytes: Uint8Array)        -> sha256 hex            (plaintext PUT)
publish_manifest(eventJson: str)   -> "ok" | throws         (sign + relay publish)
fetch_manifest(pubkeyHex: str)     -> raw signed event JSON | null
download(blobHash: str)            -> Uint8Array            (public GET)
```

## ⚠️ Owner steps before/at live verification

1. **Official npub is a PLACEHOLDER.** `open_stash.rs::OFFICIAL_AXENSTAX_PUBKEY_PLACEHOLDER` is 64 hex zeros. Provide the real AxeNStax pubkey (hex) via the `official_axenstax_pubkey()` seam (Goal 5 also depends on this). Until then `with_official()` follows a dead key.
2. **Public Blossom.** `BLOSSOM_PUBLIC_URL` must be set in `tools/sites/game/.env` to a CORS-enabled, browser-reachable Blossom (e.g. `https://blossom.primal.net`). Same requirement as private Stash (demo-plan §4 P1). Confirm `/game` meta shows a non-empty `blossom-url`.
3. **noble-curves loaded.** `openstash.js` verifies fetched manifests with `window.AxeNoble.schnorr` (loaded by `lobby.html`). If absent, reads are rejected (fail-closed — correct).

## Live round-trip (the ~15-min verification)

1. Start the game site (`tools/sites/game/start.sh`, HTTPS `https://localhost:8094`); sign in with a **burner A** Signet (same-device redirect; QR is backup).
2. In DevTools console, **publish** a tiny item to A's open-stash:
   ```js
   await window.axenstax_openstash_put_blob(new TextEncoder().encode('{"hello":"world"}'))
   // → sha256 hex H. Then publish a manifest referencing it:
   await window.axenstax_openstash_publish_manifest(JSON.stringify({
     kind: 30820,
     tags: [['d','axenstax-open-stash'], ['t','scenario']],
     content: JSON.stringify({ items: [{ name:'Test', blob_hash:H, content_type:'scenario', size:17 }] }),
   }))   // → "ok"
   ```
3. From a **second identity B** (or even anonymously — reads need no signer), **follow + list** A:
   ```js
   const raw = await window.axenstax_openstash_fetch_manifest('<A pubkey hex>') // raw signed event JSON | null
   const bytes = await window.axenstax_openstash_download(H)                    // Uint8Array
   ```
   Expect: `raw` parses to the manifest with the item; `bytes` decode back to `{"hello":"world"}`.
4. **Pass = GREEN:** publish→follow→list→download works cross-identity with only signature verification. Record the outcome here. Goal 5 can then publish the official ScenarioDefs to the real AxeNStax npub.

## Notes / deferred
- The polished in-game "share my mod" UI is a later product feature (this is the publish CODE/API + a console dev/test path).
- `relayPublish` resolves on the relay's `OK` frame; some relays are lenient — confirm `relay.trotters.cc` returns `OK true`.
- Source of truth: `docs/superpowers/specs/2026-06-03-conference-demo-plan.md` §2b.

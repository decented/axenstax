# Signer Retention + NIP-44 bridge — design

**Status:** DESIGNED 2026-06-01. The first concrete, **unblocked** step toward the per-persona encrypted save vault (`docs/architecture/2026-06-01-persona-save-vault.md`, prerequisite #1). Stands alone and is useful regardless of how the cross-game vault lands.

**Branch:** fork off `main` → `feat/signer-retention`.

## Goal

Stop discarding the Signet signer at login. Retain it, and expose its NIP-44 `encrypt`/`decrypt` (when the signer is capable) as a small JS bridge the rest of the app — including the WASM engine, eventually — can call. This is what makes zero-knowledge encrypt-to-self possible; without it, no cloud-save encryption can happen.

This spec does **NOT** wire encryption into the save flow or build cloud upload. It only makes the capability *available*. Consuming it is later work (the vault, or the AxeNStax bridge cloud-save).

## Background: what login returns (verified 2026-06-01, signet-login@0.7.1)

`Signet.login()` / `handleRedirectCallback()` return a `SignetSession` containing `signer: SignetSigner`:

```ts
interface SignetSigner {
  readonly pubkey: string;
  readonly method: 'nip07' | 'redirect' | 'bunker';
  readonly capabilities: { canSignEvents: boolean; hasNip44: boolean };
  signEvent(template): Promise<NostrEvent>;
  nip44?: { encrypt, decrypt };
  close(): Promise<void>;
}
```

Signer-by-method (from `signet-login/src/signers.ts` + `redirect.ts`):

| Method | Signer | `hasNip44` |
|---|---|---|
| bunker / nostrconnect / nsec | `BunkerSignerImpl` | **true** (hardcoded) |
| nip07 extension | `Nip07Signer` | `!!provider.nip44` |
| QR / redirect | `EphemeralSigner` → **auto-upgrades to BunkerSigner when signet-app returns a `bunkerUri`** (`redirect.ts:139`) | false → **true after upgrade** |

**Key fact:** the common AxeNStax QR/redirect flow already auto-upgrades to a NIP-44-capable bunker signer *when the signet-app deployment ships the `bunkerUri`*. So capability is "present when the deployment supports the redirect-bunker auto-pair, absent on older deployments." Whether users get it is a **signet-app deployment** matter, not AxeNStax code.

**The bug we're fixing:** `auth.js` extracts only `pubkey` from the session and calls `bootWasm(pubkey)` — `session.signer` (and its `nip44`) is dropped on the floor in every path (`runFreshAuth`, `handleRedirectCallback`, `probeWhoami`). We never even *retain* the capability, let alone use it.

## Decision (owner, 2026-06-01)

**Retain whatever signer we get; gate encryption on `capabilities.hasNip44`.** No auth-UX change. If the signer is capable → encryption available. If not → encryption unavailable, cloud save stays off, no error, no nag. (If capable signers turn out rare in practice, that's a future deployment conversation with signet-app — not an AxeNStax flow change.)

## Components

### 1. `auth.js` — retain the signer

Today the verified-pubkey paths discard the session. Change them to **stash the live signer** in a module-scoped holder before booting:

```js
let _signer = null;            // the retained SignetSigner, or null

function retainSigner(session) {
    _signer = (session && session.signer) || null;
    // expose capability + a stable handle for the bridge (below)
}
```

Call `retainSigner(session)` in each path that currently has a `session`:
- `runFreshAuth` — after `Signet.login()` returns `session` (before/with `verifyOnServer`).
- `handleRedirectCallback` path — `result.session` (this is the one that may be the upgraded bunker signer).
- `restoreSession` path, if/when added (bunker sessions reconnect via `Signet.restoreSession()` — worth calling so a returning user regains capability without re-login; **in scope** as a best-effort: try `restoreSession()` on the cookie fast-path, retain if it yields a signer).

> `probeWhoami` (server-cookie fast path) returns only a pubkey, no signer — a user who is cookie-authenticated but hasn't re-run the SDK login this page-load has **no live signer**. For those, attempt `Signet.restoreSession()` to recover one; if that fails, capability is simply absent this session. Documented, not worked around.

### 2. `crypto-bridge.js` (new) — the NIP-44 surface

A small module exposing a stable global the rest of the app (and the WASM engine via wasm-bindgen extern, later) calls. Keeps all key/signer handling in the JS layer — the engine never touches keys (mirrors `world_store.js`/`blossom.js`).

```js
// window.axenstax_crypto
//   .available()                         -> bool   (signer retained AND hasNip44)
//   .encryptToSelf(plaintextStr)         -> Promise<string ciphertext>   // NIP-44 to own pubkey
//   .decryptFromSelf(ciphertextStr)      -> Promise<string plaintext>
window.axenstax_crypto = {
    available() {
        return !!_signer && !!_signer.capabilities?.hasNip44 && !!_signer.nip44;
    },
    async encryptToSelf(plaintext) {
        if (!this.available()) throw new Error('nip44 unavailable');
        // encrypt-to-self: peer pubkey === own pubkey
        return _signer.nip44.encrypt(_signer.pubkey, plaintext);
    },
    async decryptFromSelf(ciphertext) {
        if (!this.available()) throw new Error('nip44 unavailable');
        return _signer.nip44.decrypt(_signer.pubkey, ciphertext);
    },
};
```

> Signature note: NIP-44 `encrypt(peerPubkey, plaintext)` / `decrypt(peerPubkey, ciphertext)`. For encrypt-to-self, peer = own pubkey. Confirm the exact `nip44.encrypt` arg order against `signet-login/src/signers.ts` at implementation (the BunkerSigner wraps nostr-tools nip44 — match its shape).

`_signer` lives in `auth.js`; `crypto-bridge.js` either reads it via a tiny accessor `auth.js` exports on `window`, or the two are merged. Prefer a one-line accessor (`window.__axenstax_get_signer()`) so `auth.js` stays the single owner of session state.

### 3. Binary-safe encryption (important)

World blobs are **binary** (tar+gzip), but NIP-44 encrypts **strings**. The bridge must base64-encode bytes before `encryptToSelf` and base64-decode after `decryptFromSelf`, OR expose byte-oriented wrappers:

```js
//   .encryptBytesToSelf(Uint8Array)  -> Promise<string ciphertext>
//   .decryptBytesFromSelf(string)    -> Promise<Uint8Array>
```

Recommend the byte wrappers (base64 internally) since every real consumer (worlds, tracks, skins) is binary. Plaintext-string methods can be dropped if unused (YAGNI).

### 4. Load `crypto-bridge.js` on lobby + `/game`

Add `<script src="/static/crypto-bridge.js"></script>` after `auth.js` in both `templates/lobby.html` and `game/engine/index.html` (the latter also fixes the long-standing "JS save helpers loaded, but no crypto" gap).

## What this explicitly does NOT do

- No wiring into `save_world` / no cloud upload (that's the vault / bridge cloud-save).
- No WASM extern yet (`js_crypto_encrypt`) — added when a consumer needs it. This spec stops at the JS surface + retention. (Rationale: nothing in the engine consumes it yet; adding the extern now would be dead code.)
- No manifest, no Blossom changes.
- No auth-UX change (no forcing bunker sessions).

## Error handling

- `available()` is the single gate; every consumer checks it first. No signer / not capable / no nip44 → `available()` false → consumers skip encryption cleanly.
- `encryptToSelf`/`decryptToSelf` throw only if called when unavailable (programmer error) or if the signer's nip44 call rejects (relay/bunker failure) — callers treat a throw as "encryption failed, fall back to not-uploading", never as a gameplay error.
- Signer loss mid-session (bunker disconnect): `available()` flips false on the next check; in-flight calls reject. Acceptable — cloud save just goes quiet.

## Testing

- **JS unit (new `tools/sites/game/tests/` — JS, or a small node harness):** with a **fake signer** object (`{pubkey, capabilities:{hasNip44:true}, nip44:{encrypt,decrypt}}`), assert `available()` true; assert `encryptBytesToSelf`→`decryptBytesFromSelf` round-trips bytes identically (fake nip44 = identity/base64 so the test is deterministic, no real crypto). With `hasNip44:false`, assert `available()` false and the encrypt/decrypt methods throw. This is the meaningful coverage — the retention + gating logic — without needing a real bunker.
- **Served-content test (extend `test_worlds_routes.py`):** assert `/static/crypto-bridge.js` serves and exposes `window.axenstax_crypto`; assert both lobby `/` and `/game` load it.
- **End-to-end (deferred, needs a real bunker-capable Signet login + browser):** real `encryptToSelf` round-trip against signet-app. Cannot run on this host (no bunker, no browser) — documented, not executed.

## Sequencing / relationship to the vault

This is vault prerequisite #1 and the foundation for *any* AxeNStax encryption (bridge cloud-save or full vault). Build order from here:
1. **This spec** — capability available.
2. Then either: AxeNStax **bridge cloud-save** (server proxy + server manifest, now encrypting via this bridge), or wait and build straight onto the Forgesworn **vault SDK**. Owner to choose when this lands.

## Memory-rule check

- `feedback_signet_boundary`: consumes only generic SDK capability (`signer.nip44`); no Signet-internal work. ✅
- `feedback_npub_only_display`: no user-facing surface here (internal crypto bridge). ✅
- `project_pwa_priority` / shared-infra: engine stays key-agnostic; crypto lives in JS. ✅
- UK English. ✅

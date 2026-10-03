# Cloud Save — Full Loop (AxeNStax bridge) design

**Status:** DESIGNED 2026-06-01. The consolidated, buildable design for the **prize**: *sign into AxeNStax on any machine → your worlds are there → open them*. Steam-Cloud behaviour for AxeNStax, shippable now without waiting on the cross-game Forgesworn vault.

**Relationship to the other docs:**
- `docs/architecture/2026-06-01-persona-save-vault.md` — the **destination**: a cross-game Forgesworn primitive (one persona's saves across AxeNStax + other games). This doc is the **AxeNStax bridge** of that vision: same encryption + retrieve model, AxeNStax-scoped manifest, extract-to-vault later.
- `docs/superpowers/specs/2026-06-01-signer-retention-nip44-design.md` — brick 1 of this loop (retain signer + NIP-44). Folded in as Phase A below.
- Supersedes the upload-only `2026-06-01-cloud-save-phase-5-design.md`.

**Branch:** `feat/cloud-save-full-loop` (off `main`).

---

## The prize (acceptance, in plain terms)

A kid builds a world on the home PC (cloud save on for that world). They go to a friend's house / a school Chromebook, open AxeNStax, sign in **with the same Signet identity**, and **their world is in the list**. They open it and it's their world, intact. They never touched the friend's machine before. That is the whole point — a PWA that behaves like an online game, not one chained to a single browser.

## Keying decision (read this first)

Worlds are keyed and encrypted to the **pubkey the current Signet login yields** (the identity npub). This is what makes cross-machine retrieve work: same login → same pubkey → same worlds, on any device.

- **Per-game persona derivation** (keeping AxeNStax's identity unlinkable from your other apps) is a **later privacy enhancement**, blocked on upstream Heartwood "reserved scope." It does **not** affect whether retrieve works. When it ships, migrating is a re-key-on-next-save, not a rebuild. Documented as deferred; the prize does not depend on it.

## Zero-knowledge model

- Each world blob is **NIP-44 encrypted-to-self** (peer = own pubkey) by the Signet signer before it leaves the browser. Server, proxy, Blossom, and the manifest store only ever see **ciphertext + hashes**. No plaintext world ever leaves the device.
- Requires a **NIP-44-capable signer** (`capabilities.hasNip44`). The QR/redirect flow auto-upgrades to a capable bunker signer when signet-app ships a `bunkerUri`; bunker/nsec.app logins are capable directly. If the signer is **not** capable, cloud save is cleanly unavailable (toggle inert, local play unaffected) — never an error.

## The full loop (what must exist for the prize)

```
SAVE (machine A, deliberate save/exit, world's cloud-save toggle ON):
  pack_world → [local IndexedDB write, always first]
            → encrypt-to-self (NIP-44) → POST /worlds/upload (ciphertext)
            → record {name, blobHash, meta, updated} in the per-pubkey MANIFEST

RETRIEVE (machine B, fresh, same login):
  sign in → get NIP-44-capable signer + pubkey
  lobby world list = MERGE(local IndexedDB list, MANIFEST entries for this pubkey)
    → cloud-only worlds appear in the list (badged "cloud")
  open a cloud world → GET /worlds/download/{hash} (ciphertext)
    → decrypt-from-self (NIP-44) → unpack_world → hydrate → play
```

The two pieces that don't exist yet — and without which the prize fails — are the **manifest** (so machine B knows what to fetch) and the **list-merge** (so cloud worlds appear in the lobby).

---

## Phases (build order)

### Phase A — Signer retention + NIP-44 bridge
Per `signer-retention-nip44-design.md`. Retain `session.signer`; expose `window.axenstax_crypto.{available, encryptBytesToSelf, decryptBytesFromSelf}` gated on `hasNip44`. Binary-safe (base64 inside). **Prerequisite for everything below.**

### Phase B — Encrypt on upload, decrypt on download
- `blossom.js` `cloudUpload`: encrypt bytes via `axenstax_crypto` before `POST /worlds/upload`; skip (no-op) if `!available()`.
- A `cloudDownload(hash)`: `GET /worlds/download/{hash}` → `decryptBytesFromSelf` → bytes.
- The blobHash recorded is the **ciphertext** hash (what Blossom stores) — the existing hardened integrity checks still apply.

### Phase C — Manifest (server-side bridge)
- New `app.py` routes: `POST /worlds/manifest` (record/update an entry: `{name, blobHash, meta, updated}` under the session pubkey) and `GET /worlds/manifest` (return this pubkey's entries). Session-cookie gated, rate-limited (reuse `_worlds_rate_allow`).
- Store: `data/manifests/{pubkey_hex}.json` (BRIDGE — single authority; blobs survive its loss but become undiscoverable. Destination = persona-signed Nostr event per the vault doc).
- Manifest holds **no plaintext** — names/meta are arguably low-sensitivity, but to stay honestly zero-knowledge, the entry's `name`/`meta` are stored **encrypted** too (or the whole entry list is an encrypted blob). Decide in plan: simplest honest option is the manifest value is itself NIP-44 ciphertext the client decrypts. *(Open item resolved in plan: encrypt manifest entries.)*

### Phase D — Upload wires to manifest + save flow
- Engine `save_world` (deliberate save/exit only; autosave stays local — the existing `save_world`/`autosave_world` split) → JS `cloudUpload` → on success, `POST /worlds/manifest`.
- Per-world **"Save online" toggle, default OFF** (privacy-first), stored in the world's IndexedDB meta, surfaced in the lobby world list. `cloudUpload` only fires for toggled-on worlds.
- DOM status indicator over `/game` (saving / synced / failed); never blocks.

### Phase E — Lobby list-merge + restore (the retrieve half)
- Lobby world list = merge `axenstax_list_worlds(pubkey)` (local IndexedDB) with `GET /worlds/manifest` (cloud). Dedupe by name; last-write-wins by `updated`; cloud-only entries badged so the kid sees "this lives in the cloud."
- Opening a cloud-only world: `cloudDownload(hash)` → decrypt → hand bytes to the existing WASM unpack/hydrate path (the same entry `load_world` uses) → into the world.
- Guard: a newer **local** copy is never silently clobbered by an older cloud copy (and vice-versa) — surface a choice if they conflict, else newest wins.

### Phase F — Cross-machine playtest gate
Build on PC (toggle a world on, save/exit) → second browser profile / Chromebook, same login → world appears, opens, intact. **Needs docker (Blossom) + a bunker-capable Signet login + two machines/profiles.** Cannot run on this host — the playtest boundary.

---

## What's buildable now vs blocked

- **Phases A–E are buildable** with: the hardened Blossom proxy (built), a NIP-44-capable signer (available via bunker/redirect-upgrade), a server-side manifest (new, simple). **No upstream dependency for the prize.**
- **Blocked / deferred:** per-game persona derivation (privacy enhancement, upstream Heartwood); the cross-game Forgesworn vault (the destination — AxeNStax bridge extracts into it later); end-to-end verification (Phase F — needs docker + browsers + bunker).

## Cross-game note (eye on the bigger prize)

This bridge is deliberately shaped so the destination is a clean extraction, not a rewrite: the manifest entry schema (`{app, kind, name, blobHash, updated}`), the encrypt-to-self model, and the Blossom blob layer are exactly the vault's — AxeNStax just hard-codes `app:"axenstax"` and uses a server-side manifest instead of a persona-signed Nostr event. When the Forgesworn vault SDK exists, AxeNStax swaps its manifest store + persona selection and inherits cross-game saves (worlds + a racing game's tracks + skins, all under one persona) with no change to the encrypt/blob/save-flow layers.

## Testing strategy

- **JS unit:** `axenstax_crypto` round-trip with a fake signer (Phase A); `cloudUpload`/`cloudDownload` encrypt-then-decrypt round-trip with a fake crypto + respx-mocked proxy (Phase B); manifest record/list with mocked fetch (Phase C/E); list-merge dedupe/last-write-wins logic as a pure function (Phase E).
- **Python:** manifest routes — session gate (401), disabled (503), rate limit (429), record-then-list round-trip, per-pubkey isolation (pubkey X never sees pubkey Y's entries). Extend `test_worlds_routes.py`.
- **Engine:** the save_world/autosave split already exists; assert (build-level) autosave never reaches the cloud hook.
- **E2E:** Phase F, deferred (docker + browsers + bunker).

## Memory-rule check
- `project_shared_infra_strategy`: bridge shaped for clean extraction to the cross-game vault. ✅
- `feedback_signet_boundary`: uses only generic signer NIP-44; no Signet-internal work. ✅
- `feedback_npub_only_display`: lobby badges/labels show npub/handle, never hex. ✅
- `reference_trotters_relay`: destination manifest → our trotters relay (bridge uses server store). ✅
- UK English. ✅

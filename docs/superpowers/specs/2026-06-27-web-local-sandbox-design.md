# Web = purely-local sandbox (kill web login) — design

**Status:** APPROVED (owner, 2026-06-27) — implementation in progress on branch
`feature/web-local-sandbox`. Decomposed from a wider discussion; the **export-
everything / native-import-and-merge** migration is a *separate, later* spec.
**Date:** 2026-06-27
**Branch:** `feature/web-local-sandbox` (off `main`).

---

## Why

The web build is hosted by us, which means we operate a service (OSA provider /
GDPR controller territory). The agreed posture (`../../../AxeNStax-internal/docs/
foundations/2026-06-24-privacy-tiers-data-minimisation-and-gdpr-posture.md`,
"Web = pure anonymous local sandbox — a taster") is to minimise the web to the
legal floor: **no login, no Stash, no multiplayer — local play only.** Identity,
cloud saves and multiplayer live in the **native** download (software, not
service) and the managed server (full compliance).

The web build had drifted from that: boot was already anonymous-by-default
(`web_main.rs`), but the sign-in path, cloud Stash, Join, and Logout were still
wired, and **world save/load was keyed by pubkey** — so a guest (no pubkey) could
reach the lobby but **could not load or save any world** (`"Local world load
failed: no pubkey"`). This spec finishes the conversion.

## Scope

In: make the web a login-free local sandbox; **fix the no-pubkey bug**; remove
the dead lobby buttons; surface a desktop-app nudge ("show them what they're
missing"). Keep **per-world Export/Import** (the local backup/restore) working.

Out (separate work): export-everything bundle + native first-run import + merge
(its own spec); native login/Stash/multiplayer (untouched).

## The two layers

- **Engine (shared Rust → web + native):** behaviour is `cfg(target_arch =
  "wasm32")`-gated. Native is never touched — it keeps login, Stash, Join.
- **Site (web-only JS/Python):** `world_store.js`, `auth.js`, `entrance.html`,
  `app.py`.

## Components

### 1. Local storage namespace — the bug fix
- `save::storage_namespace(Option<&str>) -> String` (pure, cross-platform,
  unit-tested): `None`/empty → `"local"`; else the pubkey. Never empty.
- `save::wasm_storage_key()` (wasm) wraps it over `WASM_PUBKEY`.
- All WASM save/load/list/delete/export call sites use it; the `"no pubkey"`
  early-returns are **removed** (`save.rs`, `menu.rs`, `game_loop.rs`).
- `world_store.js` `validPubkey()` accepts `"local"` alongside 64-hex.
- **Start fresh:** old signed-in-era blobs (keyed by pubkey) are orphaned, not
  migrated, not deleted. (Owner decision; web is alpha-fresh.)
- Cosmetic skin/wardrobe restore (`kick_off_skin_load`, `kick_off_wardrobe_load`)
  stay pubkey-gated (no-op for guests) — out of scope.

### 2. Lobby UI (engine, wasm-gated)
- **Removed on web:** "Log out" (no login), "Sync Stash" (no cloud), "Join Game"
  (no multiplayer).
- **Kept on web:** "Exit" (back to home), "Restore from file" + per-world
  "Backup" — the local backup/restore.
- **Added on web:** a header nudge — *"☁ Cloud saves & 👥 multiplayer live in the
  free desktop app →"* — opening `/download` (new tab).
- Native lobby unchanged.

### 3. Desktop funnel
- Game site `/download` → 302 to `DOCS_URL/download` (env-adaptive), so the engine
  needn't know the docs host.

### 4. Entrance / auth sign-in removal — Commit 2 (browser-verified)
- Remove the optional Signet sign-in section from `entrance.html`; short-circuit
  `auth.js` to a guest boot. Dead `/auth/*` routes left for a later cosmetic
  cleanup (not invoked). Split out because it's the least machine-verifiable
  piece (inter-script coupling) and wants a browser test.

### 5. Burner-key feedback — ALREADY IMPLEMENTED (cloud.js)
- `/bug`+`/idea` already send login-free: `cloud.js` `makeBurnerSigner()` mints a
  **fresh ephemeral key per send**, `mailboxFlush()` always uses it ("never the
  player's identity"), and `__axenstax_mailbox_enqueue` flushes **on submit**
  (not at the removed Sync button). The engine submits via
  `wasm_feedback.rs` → `__axenstax_mailbox_enqueue`; `/game` loads the full stack
  (relay/nostr/nip59/mailbox/cloud). So removing the Sync Stash button (§2) did
  **not** break feedback. No change needed here — done in the 2026-06-24
  compliance work. **Maturity exit:** remove web feedback entirely once volume
  drops. (`nip59.js`/`mailbox.js` are signer-agnostic — the burner is just a
  synthetic signer passed to the existing `wrap()`.)

## Backup / restore (answering "can I still back up worlds?")

Yes — per-world Export (`world_store.js` `exportWorld` → downloads `<name>.axeworld`)
and Import (`pickWorldFile` → unpack → local store) are **pure local file I/O, no
login dependency** (`pubkey` was only ever the IndexedDB namespace). They survive
the kill untouched, and the `.axeworld` format is byte-identical to native, so a
downloaded world restores on web or native. **A world only in IndexedDB (never
exported) is NOT carried across the kill** (start-fresh orphans it under the old
pubkey key) — so a world to keep must be Exported to a file first. The bulk
"export everything incl. trial progress" is the separate migration spec.

## Verification

`check.sh` (clippy + native build + `cargo test` + `trunk build` WASM + bundle
gate) covers the Rust. Unit test: `storage_namespace_guest_maps_to_local`.
Browser-only (owner verifies): web → Play → enter a trial + a world → place
blocks → **reload → state persists** (bug fixed); no Log out / Sync / Join; nudge
opens the download page; Export downloads a `.axeworld`, Restore re-imports it.

## Commits (all on the branch, not pushed until owner says)
1. Engine namespace fix + lobby gating + nudge + `/download` redirect +
   `validPubkey` + this spec + unit test. ✅ `check.sh` green.
2. Entrance/auth sign-in removal. ✅ (JS/HTML only; browser-verified by owner.)
3. Burner-key feedback — **no code: already implemented** (cloud.js); spec
   corrected to match.

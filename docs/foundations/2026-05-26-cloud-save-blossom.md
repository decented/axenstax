# Cloud Save (Blossom) — local-first backup + cross-device sync

**Status:** **PHASES 2–3 BUILT (bridge tier) — on `main` 2026-06-01, NOT yet verified end-to-end.** Plan: `docs/superpowers/plans/2026-05-31-cloud-save-phase-2-3.md`. The server-key-signed transport (Blossom server + `POST /worlds/upload` + `GET /worlds/download/{hash}` + `blossom.js`) is built and unit-tested. Phases 4–10 are **not** built. The feature is **inert at runtime** (nothing calls it; not loaded in `/game`) — see "Implementation status" below for the precise gap list. Still **post-alpha** per ADR-003 and `docs/roadmap.md:39`.
**Branch:** built on `main` (phases 2–3); hardening on `fix/cloud-save-bridge-hardening` (2026-06-01).
**Trigger:** The offline/login discussion of 2026-05-26 settled a **local-first, sync-on-reconnect** model: build offline → when you go online you can back up, share, and trade. Blossom save is the foundation that (a) makes local worlds recoverable across devices and (b) is the **transport every later sharing/trade feature rides on** (Spec 25 Plan Trade, world sharing, future Bitcoin-backed marketplace). It is the natural first concrete piece of the "go-online publish layer."

This **supersedes-by-update** the preserved Blossom architecture in `docs/superpowers/specs/2026-04-15-pwa-alpha-design.md` §2.4–§3, which was written before local-first IndexedDB shipped, before the sticky-Signet-first decision, and before the `signet-login` SDK. That doc remains the reference for the upload/download/list/manifest mechanics; this doc records the deltas and the phased build.

---

## TL;DR

A player builds worlds offline; they live in IndexedDB keyed by the player's Signet pubkey (the current alpha baseline — `world_store.js`, `wasm_save.rs`). When the player is online, a background **sync** pushes any locally-changed worlds to a Blossom blob store and pulls any cloud worlds missing locally. Local is always the source of truth; cloud is backup + cross-device carry. The engine already produces the exact artefact we upload — `wasm_save.rs::pack_world` returns a tar+gzip blob today — so this is overwhelmingly **website + JS layer work**, with a thin engine seam and a Blossom server to stand up. No new in-game UX beyond a "Save to cloud" / sync-state affordance in the lobby and world list.

### New shape in one paragraph

The save path becomes **local-first then async-push**: `pack_world` → write IndexedDB (as today) → *if online*, encrypt + upload the same blob to Blossom and record its hash in a per-pubkey **manifest**. Restore is **manifest → fetch blob by hash → decrypt → `unpack_world` → hydrate** (the unpack path already exists). The lobby's world list merges local + cloud entries with last-write-wins by `last_saved`. Because local saves and cloud blobs share **one identity** (the sticky Signet pubkey), there is **no identity merge** — the `<pubkey>:<world_name>` key is the same on both sides. Offline, every cloud affordance is simply inert; nothing forks the gameplay code.

---

## Why this lives here / why now

- **It's the keystone of the go-online layer.** Per the 2026-05-26 model, world/plan **sharing and trade are online-only and ride on blob storage.** Blossom save is the transport; Spec 25 (Plan Trade) and any world-sharing feature publish through the same primitive. Building it first unblocks the rest cleanly.
- **It closes the offline data-loss gap for online kids.** Offline solo worlds currently live only in one browser's IndexedDB (`world_store.js`: "no encryption at rest for alpha; per-pubkey keying is the only isolation"). A wiped cache or a new device loses everything. Once a kid reconnects and syncs, their worlds are recoverable. Per the owner's reality check (most kids are online), this shrinks the vulnerable window to "built offline and never been online since."
- **The sticky-Signet-first decision makes it migration-free.** Because identity is established before the first world exists, local and cloud share one pubkey. The Option-A/B local-identity reconciliation we explored on 2026-05-26 is moot here — there is nothing to re-key.
- **Cross-game lift** (per shared infra strategy). Encrypted blob backup keyed to a Nostr identity, with a relay-published manifest, is **engine-generic and game-agnostic.** Nothing here is AxeNStax-specific except the blob *contents* (the world format). Worth building so other games on the same primitives inherit it.

---

## What changed since the 2026-04-15 design (the deltas that matter)

The 2026-04-15 doc designed cloud save as the *primary* persistence with IndexedDB as a fallback, chose **Option A** (website proxy signs Blossom uploads with a **server** key), and used a **server-side Python dict** as the manifest authority (both labelled BRIDGE). Four things have moved:

1. **Local-first is now the baseline, not the fallback.** IndexedDB local save shipped and is the alpha contract. So Blossom is layered **on top** as async backup/sync — not a replacement. This removes Blossom from the critical path: a failed upload never blocks play; it just retries.
2. **Sticky Signet-first identity (2026-05-26 decision).** Local saves are already keyed by the verified pubkey, so cloud blobs key by the same pubkey. No identity merge, no orphaned-local-world problem.
3. **The `signet-login` SDK is now vendored** (`tools/sites/game/static/vendor/signet-login.iife.js`). This is where Blossom auth *would* be signed client-side — **and it's why this feature does NOT need the engine-side NIP-46 signing bridge** (Spec 1 Phase 4 / Spec 13). All signing + encryption happens in the JS layer, not the WASM engine. The engine just produces and consumes blobs.
4. **But the SDK does not yet expose generic signing/encryption.** `window.Signet` currently surfaces only `{login, handleRedirectCallback, restoreSession, logout}` (CLAUDE.md). It does **not** expose `sign_event` (kind-24242 Blossom auth) or NIP-44 encrypt-to-self. So the **player-key-owned, fully-portable** destination is blocked on the same upstream Signet capability family as Spec 13 — see the two-tier plan below.

---

## Architecture: bridge tier vs destination tier

The signing/manifest design has a **buildable bridge** and an **upstream-blocked destination**. Build the bridge; upgrade in place when upstream lands consumer-side signing.

### Bridge tier (buildable now)

- **Signing:** website proxy signs the Blossom kind-24242 auth event with a **server/app key** and uploads on the player's behalf (Option A from the 2026-04-15 doc, lines 198–215). Blobs are owned by the server identity, namespaced internally to the player's pubkey. Not portable to a third-party Blossom client — acceptable because, per the old doc, "the website IS the game distribution point."
- **Manifest:** server-side `{pubkey → [{world_name, blob_hash, meta}]}` map, persisted to `data/manifests/{pubkey_hex}.json` (BRIDGE — single authority; blobs survive its loss but become undiscoverable).
- **Encryption — open decision (see Open Questions).** Client-side encryption with a player-derived key needs SDK key access we don't have yet. Bridge options: (i) ship unencrypted-at-rest behind TLS + server access control — voxel worlds carry no PII, so the risk is low; or (ii) server-side encryption at rest. The roadmap calls the destination "encrypted cloud save," so client-side encryption is the target, not the bridge.

### Destination tier (when upstream Signet exposes consumer signing/encryption)

- **Signing:** the player's Signet key signs kind-24242 via the SDK → **player-owned blobs**, accessible from any Blossom client. The portability the 2026-04-15 doc wanted but couldn't reach.
- **Encryption:** NIP-44 encrypt-to-self via the SDK before upload; re-derivable on any device from the player's identity → true cross-device private worlds.
- **Manifest:** a **player-signed parameterised-replaceable Nostr event** published to `wss://relay.trotters.cc` (our own infra — trotters relay), replacing the server-side dict. Decentralised, no single authority. Needs `nostr-tools/relay` + `nip44.v2` re-vendored (that bundle was removed with the 2026-05-27 Charter strip; regenerate via `tools/sites/game/static/vendor/REGENERATE.md`).

The bridge → destination upgrade is **transparent to the local-first UX**: only the signer, encryptor, and manifest store change; `pack_world`/`unpack_world` and the sync orchestration are identical.

---

## The seam (where engine ends and JS begins)

The engine already does its half. `wasm_save.rs::pack_world(meta, save, world) -> Vec<u8>` produces the tar+gzip blob; today `js_save_world(pubkey, name, blob, meta_json)` writes it to IndexedDB via `world_store.js`. Cloud save adds a **parallel JS sink**: the same blob is (encrypted and) PUT to Blossom, and the hash recorded in the manifest. Restore feeds downloaded bytes into the existing `unpack_world` path. The engine gains at most a thin "load from these bytes" entry point if one isn't already reachable from JS — most of this never touches Rust.

---

## Phased scope

| # | Phase | Surface | Est. LOC | Notes |
|---|-------|---------|:---:|-------|
| 1 | **This spec** | `docs/foundations/2026-05-26-cloud-save-blossom.md` | ~600 | — |
| 2 | **Blossom server infra** | infra/docker + `tools/sites/game` config | ~80 | Stand up (or select) a Blossom server alongside trotters; surface `blossom_url` to the client via a meta tag (mirror the relay-url pattern). Document deploy. |
| 3 | **`blossom.js` — auth + transport** | `tools/sites/game/static/blossom.js`, `app.py` proxy routes | ~250 | BUD-01 auth + BUD-02 PUT/GET. **Bridge:** server-proxy signs with server key (`/worlds/upload`, `/worlds/download/<hash>`). Reuse vendored relay/crypto bundle where possible. |
| 4 | **Encryption layer** | `blossom.js` | ~120 | Per Open-Q decision. Destination: NIP-44 encrypt-to-self via SDK. Bridge: TLS-only or server-side — flagged, not assumed. |
| 5 | **Upload (local-first push)** | `blossom.js`, `world_store.js`, thin `wasm_save.rs` hook | ~150 | After the IndexedDB write succeeds, if online, encrypt + upload the same blob; record hash. Fire-and-forget with retry; never blocks save. |
| 6 | **Manifest** | `app.py` (bridge) → later `manifest` Nostr event (destination) | ~150 | Bridge: `data/manifests/{pubkey_hex}.json`. Destination: player-signed replaceable event on trotters. |
| 7 | **Download / restore** | `blossom.js`, `lobby.js`, `unpack_world` reuse | ~150 | Manifest → fetch by hash → decrypt → unpack → hydrate. Enables cross-device restore. |
| 8 | **World-list merge + sync state** | `lobby.js`, `world_store.js` | ~200 | Merge local + cloud entries; show per-world state (local-only / synced / cloud-only); last-write-wins by `last_saved` with a guard against clobbering a newer local copy. **npub, never hex, in any user-facing label** (npub only display). |
| 9 | **Sync orchestration + "Save to cloud" affordance** | `lobby.js`, game-site UI | ~150 | On reconnect: push dirty local worlds, pull cloud-only worlds. User-visible control reads "Save to cloud" — **never "Blossom"** (`roadmap.md:39`: that's implementation detail). Offline → controls inert/greyed, no nags. |
| 10 | **Playtest gate (Axolittle)** | n/a | 0 | Build offline → go online → confirm backup; clear cache / second browser → confirm restore; sync-state labels legible. |

**Total:** ~1,400 LOC, ~90% JS/Python/infra, thin engine touch. Phases 2–9 autonomous; Phase 10 is the playtest gate.

Recommended order: 2 → 3 → (4 ‖ 6-bridge) → 5 → 7 → 8 → 9 → 10. Each phase reaches a `check.sh`-green + smoke-green state before the next.

---

## Open questions / decisions for the owner

1. **Bridge encryption posture.** Ship the bridge unencrypted-at-rest (TLS + access control; voxel worlds carry no PII) or invest in server-side encryption now? The destination is client-side-encrypted regardless. *Recommendation: unencrypted-at-rest bridge — lowest cost, worlds are non-sensitive, and the destination supersedes it.*
2. **Own Blossom server vs hosted.** Run our own (consistent with owning trotters) or point at an existing Blossom host for alpha-of-this-feature? *Recommendation: our own, sibling to trotters — keeps it our infra.*
3. **Quota / fair-use.** Per-pubkey storage cap? World blobs are small (chunks only stored when non-empty), but unbounded worlds × unbounded saves needs a ceiling. Defer to a config knob with a sane default.
4. **When to upgrade bridge → destination.** Gate on upstream Signet exposing consumer-side `sign_event` (kind-24242) + NIP-44 — the same capability family tracked by Spec 13. Revisit alongside that.

---

## Acceptance criteria

- A world built offline appears in IndexedDB; on going online it uploads and shows "synced" in the list.
- Clearing browser data (or a second Chromium profile signed in as the same pubkey) restores the world from cloud, byte-identical after `unpack_world`.
- A newer local save is never overwritten by an older cloud copy (last-write-wins by `last_saved`, with the guard).
- Offline: no errors, no nags; cloud controls are inert; local play is unaffected.
- No user-facing string says "Blossom"; the feature reads as "Save to cloud." No user-facing string shows a hex pubkey.
- `check.sh` green (engine untouched or thin-seam-only); `--smoke` green.

---

## Memory-rule check

- **`project_pwa_priority` / `project_shared_infra_strategy`:** built as engine-generic blob backup; only the blob *contents* are AxeNStax-specific. ✅
- **`feedback_signet_boundary`:** consumes only generic SDK capabilities (kind-24242 signing, NIP-44 encrypt) at the destination tier — no Signet-internal protocol design, no AxeNStax-specific Signet work. The upstream dependency is the *generic* signing capability, shared with Spec 13. ✅
- **`reference_trotters_relay`:** destination manifest publishes to our own trotters relay, not a third-party. ✅
- **`feedback_npub_only_display`:** user-facing world list renders npub; hex stays internal (manifest filename, keys). ✅
- **`project_alpha_launch_posture`:** axenstax.app isn't live; this is post-alpha and explicitly **not** to be built until prioritised. ✅
- **UK English** throughout. ✅

---

## Context pointers

- **Reference design (mechanics):** `docs/superpowers/specs/2026-04-15-pwa-alpha-design.md` §2.4 (upload/download/list/manifest flows), §3 (the signing problem + Option A).
- **Deferral of record:** `docs/architecture/ADR-003-pwa-first-for-alpha.md`; `docs/roadmap.md:39` ("Cloud save deferred … framed as 'Save to cloud', not 'Blossom'").
- **Engine seam:** `game/engine/src/wasm_save.rs` (`pack_world`, `js_save_world`, `unpack_world`); `game/engine/src/save.rs` (`WorldMeta`, `WorldSave`).
- **Local store being extended:** `tools/sites/game/static/world_store.js` (IndexedDB, `<pubkey>:<world_name>` keying).
- **Auth machinery to reuse:** `tools/sites/game/static/auth.js`, `tools/sites/game/static/vendor/signet-login.iife.js`. (The `nostr-tools/relay` + `nip44.v2` bundle that `charter-evaluator.js` used was removed with the 2026-05-27 Charter strip — re-vendor for the destination-tier manifest via `tools/sites/game/static/vendor/REGENERATE.md`.)
- **Upstream signing dependency (shared):** `docs/foundations/2026-05-14-engine-signing-bridge.md` (Spec 13) — the destination tier waits on the same consumer-side Signet signing capability, surfaced to JS rather than the engine.
- **Identity model context:** the 2026-05-26 sticky-Signet-first + local-first decision (this conversation); `docs/spec/02-world-format.md` (persistence), `docs/spec/06-bitcoin-integration.md` §12 (offline/non-Bitcoin sandbox).

---

## Implementation status (2026-06-01) — honest gap list

An audit after the phases-2–3 merge found the feature is a tested *skeleton*, not a working feature. Recording the truth so "it's on main" is never mistaken for "it works."

### Built + hardened (phases 2–3, bridge tier)
- Blossom server infra (`infra/blossom/`), `POST /worlds/upload`, `GET /worlds/download/{hash}`, server-key kind-24242 signing (`nostr_auth.ServerKeys`), `blossom.js` client, lobby capability flag. Unit-tested incl. real wire-shape via `respx`.
- **2026-06-01 hardening** (`fix/cloud-save-bridge-hardening`): upload now validates the BUD-02 blob descriptor and rejects a server/local sha256 mismatch (was: trusted local hash blindly); download verifies returned bytes hash to the requested hash (was: no integrity check); per-pubkey rate limit + reduced 16 MiB default cap on `/worlds/*` (was: 50 MiB, unthrottled → OOM risk); test env/singleton pollution fixed; image pinned off `:master`; `start.sh` refuses to launch with the placeholder owner key.

### NOT built / NOT working — the real gaps
1. **Nothing calls it.** No code invokes `window.Blossom.upload/download`. (Phase 5.)
2. **Not loaded in `/game`.** `blossom.js` + the `cloud-save` meta tag live only in the lobby (`/`); the engine page (`game/engine/index.html` → `dist/`) loads neither, so `window.Blossom` is `undefined` where the engine runs. The engine save path (`save.rs::save_world`/`autosave_world`) writes IndexedDB only — no cloud call. **This is why it cannot have run end-to-end.** (Phase 5.)
3. **Blossom `config.yml` schema is UNVERIFIED** against the real image (no Docker on the build host). The auth model (single-pubkey allow-list via `owners:` + `requirePubkeyInList`) is best-guess; uploads may be open to any valid signature if those keys are wrong. Loopback-only binding is the compensating control. **Must verify against the pinned image before trusting.** (C2/C3 from the audit.)
4. **No encryption** (Phase 4), **no manifest** (Phase 6 — upload returns the hash but nothing persists per-pubkey ownership), **no download ownership check** (any signed-in user can fetch any hash they know — documented BRIDGE), **no world-list merge / sync UI / "Save to cloud" affordance** (Phases 7–9), **no Axolittle playtest** (Phase 10).

### To actually ship cloud save, next session
Verify `config.yml` against the pinned image with Docker → run the end-to-end round-trip (Task 8 of the plan) → then build Phase 5 (load `blossom.js` in `/game` + wire the WASM save flow to call upload after the IndexedDB write) → Phase 6 manifest → Phases 7–9 UI. Destination tier (player-key signing + NIP-44) stays blocked on upstream Signet (`docs/foundations/2026-05-14-engine-signing-bridge.md`).

# Native login: offline-first identity + Stash deferral — decision record

**Status:** DECISIONS RECORDED — architecture contract for the eventual native-login build. **No build authorised by this doc.** It constrains Spec 1 (engine Signet auth) Phase 4, Spec 13 (engine signing bridge), and the native-build **Bucket 3** integration. Whoever builds native login treats the rules below as binding, not advisory.
**Date:** 2026-06-10
**Branch:** n/a — docs only.
**Session:** Captured in conversation with Staxolottle, 2026-06-10.
**Trigger:** Owner asked how close native login is, then set two hard requirements — **(1) login must work**, **(2) there must be an offline mode** (internet on-site is sketchy) — and asked whether **Stash** (cloud save) can be deferred until the web version is solid. Answer to the last: yes, and these are the rules that keep "later" cheap.

---

## TL;DR

Three decisions and two seams.

1. **Login is additive, never a gate.** The native app must launch and play single-player with **no internet and no sign-in**. That is already its behaviour today (native has zero Signet integration; it saves to local disk). Adding login must *preserve* that property. Sign-in is an opportunistic upgrade, not a precondition for play.

2. **Identity persists across offline launches; only fresh signatures need the network.** Pair with the Signet bunker **once** while online (`signet-nip46-client` does this), `persist()` the session, and `restore()` it on later offline launches with a **locally cached pubkey** so the app knows who you are with zero network. Producing a *new* signature (joining a server, publishing) needs the bunker reachable; **local play never asks for a signature**, so a dead connection never costs a play session. The bunker auto-lock wall ([[project_cloud_save_bridge_on_main]]) is the same constraint in its extreme form — designing for offline forces the auto-lock-tolerant shape anyway.

3. **Stash is deferred and decoupled — build login + offline first, Stash later.** Stash sits *on top of* login (it uses the signer); login never uses Stash. Local-first saves are canonical; Stash is opportunistic cloud sync on top. **Build them web-first**: the web `@forgesworn/stash` package becomes the reference that pins the blob format, and native's `stash-rs` mirrors it.

**Two seams to honour now so adding Stash later is an *add*, not a *migration*:**

- **(A) Encryption-at-rest decision.** Decide up front whether native local saves are written in the Stash AEAD format from day one, or plaintext-now-convert-later. Cheapest end-state is **"the on-disk file *is* the Stash blob"** so sync = upload.
- **(B) Identity-tag + monotonic version on every save.** Stamp saves with the owning pubkey (when signed in) and a monotonically increasing version/timestamp. Without (B) you get orphan saves Stash can't attribute and no way to resolve local-vs-cloud divergence.

**The one genuinely new problem Stash introduces** (work, not rework): sync-conflict resolution once you have offline local edits *and* cloud blobs. Cheap with seam (B) in place; nasty to retrofit without it. The append-only `WorldSave` invariant ([[project_goal3_save_hardening_delivered]]) already helps.

**One open decision the owner still owns:** is **offline LAN multiplayer** in scope for alpha? It's the only piece that doesn't fall out of the rules above for free — see §6.

---

## Where native login stands today (factual baseline)

Verified against the engine source on 2026-06-10 (read pass, not memory):

- **The server-side verification stack is real, tested, and on `main`** — BIP-340 Schnorr verify, NIP-01 canonical event IDs, the challenge-nonce table, wire encoding, and `verify_join_signet_auth()` in `hosted_server.rs`. All of it gated behind a master switch: `signet/mod.rs` → `pub const USE_SIGNET_AUTH: bool = false;`. Flip-ready; not flipped.
- **The native signer primitive is built and solo-verified.** `signet-nip46-client` (Forgesworn) is a generic NIP-46 wrapper: `pair()` from a `bunker://` URI, `persist()`/`restore()`, and it implements `NostrSigner` (so it can both sign events *and* do NIP-44). Solo-verified up to the network boundary; the live bunker handshake needs a real device.
- **The native client is not yet wired.** `remote_client.rs` sends `auth_event: None`; `protocol.rs` `JoinRequestPacket` carries the `Option<auth_event>` / `Option<handle_credential>` fields but they're ignored while the flag is false. `wasm_auth.rs` is receive-only (stores a verified pubkey; no signing path).
- **Cross-platform save interchange already works.** `.axeworld` export/import rides the shared `world_archive` module ([[project_blank_canvas_and_interchange]]) — proof that "same logical save, different transport" is an established pattern in this codebase. Stash is the same trick: shared format, cloud transport.

Conclusion carried into this doc: the expensive uncertainty is gone. What remains is **Bucket 3 integration** (owner-coordinated, explicitly out of solo scope — [[project_native_build_distribution_strategy]]) plus a **live bunker pairing** only the owner can run. This doc fixes the *shape* that integration must take.

---

## Decision 1 — Login is additive, never a gate

**Rule:** the native binary must reach playable single-player with no network and no identity. Sign-in is a button on the menu, never a wall in front of it.

Two tiers coexist:

| Tier | Needs internet? | Unlocks |
|------|:---:|---------|
| **Local play** (single-player, local saves) | **No** | The whole game. Always available. |
| **Signed-in identity** (Signet bunker paired) | Only to *pair* and to *sign* | Cloud save (Stash), multiplayer join, sats, identity attribution |

This mirrors the web side's move to a **background, non-blocking** auth restore ([[project_cloud_save_bridge_on_main]]) — auth completing is never on the critical path to playing.

**Implication for the build:** there is no "must be logged in to continue" branch anywhere in the native startup or single-player path. A guest/local profile is the default identity; signing in *upgrades* it.

---

## Decision 2 — Identity persists offline; signatures don't

**Rule:** knowing *who you are* is offline-capable (cached); producing a *fresh signature* always needs the bunker reachable. Local play never needs a signature, so it's never blocked.

Mechanics (all already provided by `signet-nip46-client`):

- **First sign-in (needs internet, once):** pair with the phone bunker over a relay. Persist the session via `persist()` **and cache the resolved persona pubkey locally** so subsequent launches don't have to round-trip to the bunker just to learn who you are.
- **Every later launch (offline OK):** `restore()` the persisted session; show the cached identity; play. No network required.
- **Anything that needs a live signature** — joining a hosted server, publishing to Stash, on-chain actions — is allowed to fail gracefully when offline or when the bunker is locked. These are *inherently* online operations.

**Why this is robust by construction:** the bunker can be locked even *with* internet (the auto-lock wall, [[project_cloud_save_bridge_on_main]]). So the architecture must already treat "can I sign right now?" as a sometimes-available capability. Offline is just the extreme case of the same rule — building for offline gets auto-lock tolerance for free.

**Caveat to state plainly:** "login must work" really means **"login works when you have a connection, and degrades to cached identity when you don't."** The very first pairing cannot be done offline. Don't let anyone design first-run as a hard online gate.

---

## Decision 3 — Stash deferred, decoupled, and built web-first

**Rule:** ship login + offline first; add Stash later as a pure sync layer. The dependency only runs one way.

Three structural reasons deferral is clean, not a debt trap:

1. **Same engine → same save types.** Native and web are the same Rust engine compiled to two targets, serialising the same save structs. Native Stash isn't "re-model the save for the cloud" — it's "take the bytes already on disk and ship them to Blossom." (`.axeworld` already proves the pattern.)
2. **`stash-rs` already exists and mirrors the web package.** The native crate is built and pushed; the integration is already scoped additively as "wire `stash-rs` into `save.rs`" (Bucket 3). Not greenfield.
3. **The login signer already covers Stash's signing need.** Stash needs **NIP-44 on the stable persona key** — exactly why a delegated/session key was a dead end for it (#183 retracted). The login signer (`signet-nip46-client`, a `NostrSigner`) provides NIP-44 on the persona key. **Stash reuses the login signer and the login pairing — no second auth path.**

**Sequencing: web-first is correct, not a detour.** Web (`@forgesworn/stash`) becomes the reference implementation that *pins the blob format*; native's `stash-rs` is a second implementation of that one format, so you want the one you're actively debugging to lead. The thing to keep aligned across the two implementations is the **blob format spec** (AEAD scheme, vault-key NIP-44 wrap, envelope) — pin it on web, mirror it on native.

**What deferral costs (acceptable, per owner):** on native you lose cloud backup + cross-device sync until Stash lands — *not* persistence (native has a filesystem). Note the web asymmetry: on web/PWA "no Stash" means *no persistence at all* unless there's a local fallback (Locket / IndexedDB), because the browser has no durable disk. So if login work also touches the PWA, the web target still needs *some* local store while cloud Stash is parked — it just needn't be Stash.

---

## The two seams to build to now (so "later" is an add, not a migration)

### Seam A — Encryption-at-rest

Decide before writing the native save path: are local saves written **in the Stash AEAD format from the start**, or **plaintext now, converted at Stash-integration time**?

- **Recommended:** disk file == Stash blob (encrypted at rest with the vault key, as web Stash v2 already does — local AEAD + NIP-44-wrapped vault key, [[project_cloud_save_bridge_on_main]]). Then "sync later" is literally *upload the file you already have*; no format change, no migration.
- **Acceptable fallback:** plaintext-on-disk now, with an explicit converter at Stash-integration time. Cheaper to build now, pays a migration later. Only choose this if encrypted-at-rest blocks shipping login.

### Seam B — Identity tag + monotonic version on every save

Every save record carries:
- the **owning persona pubkey** when signed in (and a clean guest→identity claim path for saves made before sign-in), so Stash can attribute blobs to an owner instead of orphaning them; and
- a **monotonically increasing version (or timestamp/seq)** so that when a local copy and a cloud copy diverge, there's an ordering to resolve against.

Both are tiny to add now and expensive to retrofit once saves exist in the wild.

---

## The new problem Stash introduces (flagged, not solved here)

**Sync conflict.** Once there are offline local edits *and* cloud blobs, two devices (or one device offline then a web session) can both advance the same save. You need a "which wins" rule — last-writer-wins on the version from Seam B is the floor; a smarter merge is a v2 question. Deferring Stash doesn't *create* this; it *postpones surfacing* it. Seam B is what makes it tractable when it surfaces. The append-only `WorldSave` invariant ([[project_goal3_save_hardening_delivered]]) is a head start.

This is genuinely **new work** at Stash-integration time, not rework of login — call it out in the Stash foundation spec when that gets written.

---

## Open decision (owner's call) — offline LAN multiplayer in alpha?

Everything above falls out for free *except* one case: **LAN multiplayer with no internet.** Join-auth proves identity by having the bunker sign a challenge — which means reaching the bunker over a (public) relay, which needs internet **even on a purely local network**. So "kids on a LAN with dead internet" breaks if join strictly requires fresh bunker signing.

- **If offline LAN is out of scope for alpha:** nothing to do — multiplayer is an online feature, full stop.
- **If offline LAN is in scope:** we need a **short-lived local session key** minted while online and used to sign LAN joins offline. This is a *different, narrower* use than the NIP-44 case that killed the delegated-key idea for Stash, so it's not foreclosed — but it's its own small design + spec.

**This is the one thing the owner still needs to decide** before native multiplayer login is fully specced. Single-player login + offline (Decisions 1–3) don't depend on it.

---

## What this means for the existing queue

This doc is a **constraint layer above** three existing items; it doesn't replace any of them:

- **Spec 1 — engine Signet auth ([[2026-04-20-engine-signet-auth.md]]) Phase 4.** The `USE_SIGNET_AUTH` flip. Decision 2 says the flip must not break offline/local play (it won't — the flag only governs *remote* join auth). Decision 6/§6 (offline LAN) interacts with how strictly Phase 4 demands a live signature on join.
- **Spec 13 — engine signing bridge (`2026-05-14-engine-signing-bridge.md`).** The WASM `sign_event` consumer wire-up. Native's equivalent is `signet-nip46-client`. This doc adds the offline-first framing both must respect.
- **Native build / Bucket 3 ([[project_native_build_distribution_strategy]]).** Wiring `signet-nip46-client` + `stash-rs` into the engine + native sign-in UX. Owner-coordinated, explicitly out of solo scope. Seams A + B land *in this integration*.

**Build order implied:** native sign-in (Decisions 1–2) → defer Stash → web Stash hardened (pins the blob format) → native `stash-rs` integration (Decision 3, seams A+B) → (if owner says yes) offline-LAN local session key.

---

## Memory-rule check

- **[[feedback_signet_boundary]]** — honoured. `signet-nip46-client` is a generic NIP-46 signer; nothing here asks Signet for AxeNStax-specific behaviour. NIP-44 on the persona key is a standard capability.
- **[[project_settlement_model_decision_parked]] (non-custodial)** — consistent. Players hold their own keys/wallets; the engine holds saves and (later) a score, never custody. Identity = the player's own bunker.
- **[[project_stash_locket_save_architecture]]** — consistent: Stash = cloud blobs, Locket reserved for local. This doc's "local-first saves" are the native filesystem store; Locket/IndexedDB is the web local fallback.
- **[[feedback_merge_to_main_preauthorised]]** — not triggered; docs-only, no build.
- **[[feedback_autonomy_to_playtest_boundary]]** — respected: this captures decisions and stops at the build boundary, which is owner-coordinated (Bucket 3).

## Related docs

- `docs/foundations/2026-04-20-engine-signet-auth.md` — Spec 1, Phase 4 + "Phase 4 prerequisite".
- `docs/foundations/2026-05-14-engine-signing-bridge.md` — Spec 13, WASM signing-bridge consumer wire-up.
- `docs/integrations/signet/2026-05-20-signet-login-adoption.md` — current web sign-in path (signet-login SDK).
- Native build + distribution strategy doc (Bucket 3 scope; `signet-nip46-client` + `stash-rs` integration).
- `@forgesworn/stash` (web) + `stash-rs` (native) — the mirrored Stash implementations whose blob format this doc says web pins first.

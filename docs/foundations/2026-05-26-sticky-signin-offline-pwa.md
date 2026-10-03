# Sticky Sign-In + Offline-Launch PWA — "sign in once, then play offline"

**Status:** **Phase A + Phase B DELIVERED 2026-05-27** on `main` — Phase A (sticky 90-day sliding session + `axenstax_session_until` marker) via merge `6ad5885`; Phase B (service worker + manifest + offline boot) via merge `208b2c5`. **Phase 10 (Axolittle in-browser airplane-mode playtest) is the open gate** — the offline launch itself is built but not solo-verifiable. Deferred within Phase B: branded PWA icons (manifest ships `icons: []` until `branding/logo-favicons` merges → then installable).
**Branch:** delivered (`feat/sticky-session`, `feat/offline-pwa`, both merged + deleted).
**Trigger:** The 2026-05-26 decision: **Signet sign-in is required up front, but once you've signed in you can effectively play offline.** Two capabilities make that true, and neither exists today: (1) the session must *persist* (today it's 4 hours), and (2) the app must *launch and run with no network* after the first sign-in (today it isn't a real PWA — no service worker, bundle served `no-cache`, gate validated server-side).

This is the **enabler** for offline play. Its sibling, [Cloud Save (Blossom)](2026-05-26-cloud-save-blossom.md) (Spec 31), is the **safety net** — it makes the worlds you build offline recoverable and multi-device. Together they complete "sign in once → play offline → synced & recoverable." This spec makes offline play *possible*; Spec 31 makes it *durable*. Build this one first.

---

## TL;DR

After a player signs in once (online — the same trip they made to install the app), they stay signed in for a long window, and the PWA installs a service worker that caches the app shell + the content-hashed WASM bundle. On every later launch — including with no network — the service worker serves the cached app, the boot path reads the cached identity (`localStorage` `axenstax_pubkey`, already established by ADR-003 §3) plus a new JS-readable session-expiry marker, and starts the engine without a server round-trip. Single-player worlds (IndexedDB, keyed by that pubkey) play fully offline. Anything online — joining a server, Bitcoin, sync — re-validates server-side the moment there's a connection.

### New shape in one paragraph

Three changes turn the current "must be online and freshly signed in" gate into "online once, then offline-capable": **(1) sticky session** — extend `COOKIE_TTL` from 4h to a long sliding window and write a JS-readable `axenstax_session_until` alongside the existing `localStorage` pubkey; **(2) installable PWA** — add a web app manifest + a service worker that precaches the shell and the (already content-hashed) WASM assets; **(3) offline boot fallback** — when `/auth/whoami` is unreachable, the client trusts the durable local session marker and boots the engine from the cached pubkey instead of bouncing to the lobby. The hard server-side gate (`HttpOnly` cookie + `/auth/whoami`) stays exactly as-is for everything online; the offline path is a deliberately *soft* gate over a sandbox that has nothing to protect.

---

## Why this lives here / why now

- **It's the missing half of "sign in once, then play offline."** The owner confirmed (2026-05-26) that requiring Signet first is fine — you're online to download the app anyway, and it's open-source so a no-identity offline build is always a `cargo build` away. What's *not* fine is that even a returning, signed-in kid can't open the game without a network and a sub-4-hour-old cookie. This spec fixes that.
- **PWA is the top alpha priority** (`project_pwa_priority`). Sticky session + installability + offline launch are core to "it behaves like an app," not nice-to-haves.
- **Cross-game lift** (`project_shared_infra_strategy`). A sticky-session + offline-capable PWA shell with a soft offline identity gate is entirely game-agnostic — nothing here is AxeNStax-specific. Worth building so other Decented games inherit it.
- **It de-risks nothing in the engine.** All of this is website/JS layer; the engine already runs offline (the native binary proves it, and the WASM sim makes zero network calls in single-player). The only thing standing between the WASM client and offline play is the *delivery shell*.

---

## What exists today (and the exact gaps)

| Piece | Today | Gap |
|---|---|---|
| Session lifetime | `COOKIE_TTL = 4 * 3600` (4h), `HttpOnly`, `SameSite=Strict`, `Secure` on HTTPS (`auth.py:237, 564`) | Too short; re-auth most sessions. `HttpOnly` ⇒ JS can't read it for an offline gate. |
| Identity cache | `localStorage` `axenstax_pubkey` (ADR-003 §3) — JS-readable, skips auth UI before first paint | Exists ✅, but the boot still calls `/auth/whoami` (server-authoritative) and `/game` is gated server-side. |
| App launch | `/game` (`app.py:404`) validates the cookie server-side → 302 to lobby if absent | Requires a network round-trip; can't launch offline. |
| Caching | `/game` served `Cache-Control: no-cache, must-revalidate` (`app.py:373`); **no service worker, no manifest** | Bundle refetches from network every launch; nothing cached for offline. |
| Installability | CSP already allows `manifest-src 'self'` (`app.py:90`) but **no manifest is served** | Not installable as a PWA. |
| WASM boot | `auth.js` → `__axenstax_set_pubkey(pubkey)` + `__axenstax_start()` after `/auth/whoami` (`auth.js:200, 328`) | Boot is gated on the server check; no offline fallback to cached identity. |

---

## The trust model (why a *soft* offline gate is correct)

The session cookie is HMAC-signed and verified with a secret only the **server** holds, so an **offline client cannot cryptographically re-verify it.** The offline boot therefore trusts a **JS-readable durable marker** ("signed in as `<pubkey>`, valid until `<T>`") to decide whether to serve the cached app. That is intentionally a *soft* gate, and it's the right call:

- **Offline single-player has nothing to protect** — no server, no other players, no sats, no payouts. The only thing the gate "unlocks" is the local sandbox.
- **Worlds are namespaced by pubkey.** Tampering the local marker to a different pubkey just opens an empty world namespace — it grants nothing.
- **The hard gate is unchanged for everything that matters.** The `HttpOnly` cookie + `/auth/whoami` remain authoritative for any online action (server join, Bitcoin, sync). When the device reconnects, real authority re-validates server-side.

So: hard gate online, soft gate offline, over a surface with no protected assets. The soft gate is a UX affordance, not a security boundary.

> **Note (2026-05-27):** an earlier concern that the in-engine **Charter** session-start gate would fail-closed offline and block this path is now moot — Charter was stripped from AxeNStax (merge `df26b7d`). The offline-launch path has **no remaining session-start gate** to reconcile. If parental controls return, they belong at the online/server edge keyed to identity, never as a client gate (see `project_charter_rev7_pivot` memory).

---

## Phasing & scope

Two groupings. **Phase A (sticky session)** is small, low-risk, and arguably alpha-worthy on its own — it removes the every-few-hours re-auth without any PWA work. **Phase B (offline-launch PWA)** is the larger lift that delivers true offline play. The owner decides whether B is alpha or post-alpha; A can land regardless.

### Phase A — Sticky session (cheap, standalone)

| # | Phase | Surface | Est. LOC | Notes |
|---|-------|---------|:---:|-------|
| 1 | **This spec** | `docs/foundations/2026-05-26-sticky-signin-offline-pwa.md` | ~550 | — |
| 2 | **Extend session lifetime** | `auth.py` | ~30 | `COOKIE_TTL` 4h → long sliding window (see Open-Q). Refresh the cookie on each authenticated request so an active player never lapses. |
| 3 | **JS-readable session marker** | `auth.js`, `/auth/verify` + `/auth/whoami` payloads | ~40 | Write `axenstax_session_until` (epoch) to `localStorage` at sign-in, alongside the existing `axenstax_pubkey`. This is the offline-gate input (the cookie is `HttpOnly` and unreadable to JS). |

### Phase B — Offline-launch PWA (the real offline enabler)

| # | Phase | Surface | Est. LOC | Notes |
|---|-------|---------|:---:|-------|
| 4 | **Web app manifest + icons** | `tools/sites/game/static/manifest.webmanifest`, `index.html` `<link rel=manifest>`, `app.py` serve route + icons | ~80 | Installable PWA. CSP already permits `manifest-src 'self'`; add `worker-src 'self'` for the SW (Phase 5). |
| 5 | **Service worker — precache shell + bundle** | `tools/sites/game/static/sw.js` (root scope), registration in `auth.js`/`index.html`, `app.py` (serve `sw.js` + reconcile the `/game` `no-cache` header) | ~250 | Precache app shell + JS + the content-hashed WASM assets on install (after first successful sign-in). Cache-first for hashed assets (safe — names change per build), network-first for HTML. Serve `sw.js` at root with `Service-Worker-Allowed: /` so its scope covers the root-mounted WASM (`app.py:448`). |
| 6 | **Offline boot fallback** | `auth.js` | ~120 | When `/auth/whoami` is unreachable (offline) **and** `axenstax_session_until` is in the future: boot the engine from the cached pubkey (`__axenstax_set_pubkey` + `__axenstax_start`) instead of redirecting to the lobby. Detect offline cleanly; never hang on a failed fetch. |
| 7 | **Offline `/game` gate via the SW** | `sw.js`, `auth.js` | ~80 | Offline, the SW serves the cached `/game` (the server-side 302 gate is simply never reached). The client-side boot path (Phase 6) is what enforces "signed in?" offline, per the soft-gate model above. |
| 8 | **Sign-out + expiry UX** | `auth.js`, `lobby.js` | ~80 | Sign-out clears `axenstax_pubkey` + `axenstax_session_until` (forces re-auth) but keeps the cached bundle (it's just code). Expired session while online → silent re-auth via the SDK (`restoreSession`) or a prompt; expired while offline → a friendly "reconnect to sign in again" screen, not a broken redirect loop. |
| 9 | **Update/refresh strategy** | `sw.js` | ~60 | SW versioning so a new WASM build invalidates the cache when the device is next online — don't strand kids on a stale bundle. Standard "new version available, reload" pattern. |
| 10 | **Playtest gate (Axolittle)** | n/a | 0 | Install the PWA; sign in; airplane-mode; relaunch and play a solo world; sign out clears; ship a new build and confirm it updates on reconnect. |

**Total:** ~870 LOC, entirely website/JS/Python — **zero engine code.** Phases 2–9 autonomous; Phase 10 is the playtest gate. Phase A (2–3) is independently shippable before any of Phase B.

Recommended order: 2 → 3 (Phase A lands) → 4 → 5 → 6 → 7 → 8 → 9 → 10. Each phase reaches `check.sh`-green + `--smoke`-green before the next.

---

## Open questions / decisions for the owner

1. **Sticky TTL length.** 30 / 60 / 90 days? *Recommendation: 90-day sliding window, refreshed on each online visit — an active kid effectively never re-auths; an abandoned device eventually lapses.*
2. **Is Phase B alpha or post-alpha?** Phase A (sticky) is a clear alpha win regardless. Full offline launch (B) is bigger; it could ride alpha (PWA is top priority) or follow it. *Recommendation: ship A in alpha; decide B against alpha timeline.*
3. **Soft offline gate — accepted?** The §"trust model" argument says yes (nothing to protect offline). Confirm you're comfortable that a tampered `localStorage` marker grants only an empty local sandbox.
4. **Shared-device posture.** Sticky + offline-cached identity is ideal for a kid's own tablet; on a shared machine it means a long-lived signed-in identity. Fine for the target audience, but worth a conscious "yes."

---

## Acceptance criteria

- After one online sign-in, closing and reopening the app **hours or days later** does not re-prompt for sign-in (within the sticky window).
- With the device in airplane mode, the installed PWA **launches and plays a single-player world** with no network.
- Going offline mid-session never throws a redirect loop or a blank page; an expired-offline session shows the friendly reconnect screen.
- Online actions (would-be server join, sync) still hit the authoritative server gate; the offline soft gate grants nothing online.
- Sign-out forces a fresh Signet sign-in on next launch (online); the cached bundle may remain.
- A new WASM build is picked up on the next online launch (no permanent stale-bundle trap).
- No user-facing string shows a hex pubkey (npub only). `check.sh` + `--smoke` green; **engine untouched.**

---

## Memory-rule check

- **`project_pwa_priority`:** directly serves the top alpha priority — installable, sticky, offline-capable PWA. ✅
- **`project_shared_infra_strategy`:** built game-agnostic; sticky-session + offline-shell + soft offline gate lift to any Decented PWA. ✅
- **`feedback_signet_boundary`:** no Signet-internal work — uses only the SDK's existing `restoreSession`/`login`; sticky session is our own cookie/marker policy. ✅
- **`feedback_npub_only_display`:** any user-facing identity rendering uses npub; hex stays internal (`localStorage` cache, cookie). ✅
- **`project_alpha_launch_posture`:** axenstax.app isn't live; Phase B's alpha-or-not is explicitly left as an owner decision, not assumed. ✅
- **`project_qr_signin_regression_runbook`:** this changes session *lifetime* + offline boot, not the QR sign-in handshake — keep the relay/redirect auth path untouched so the regression runbook stays valid. ✅
- **UK English** throughout. ✅

---

## Context pointers

- **Session cookie:** `tools/sites/game/auth.py` (`COOKIE_TTL` :237, `set_cookie` attrs :564, clear :701).
- **Boot + identity cache + whoami fast path:** `tools/sites/game/static/auth.js` (`/auth/whoami` :289/:328, `__axenstax_set_pubkey`/`__axenstax_start` :200, redirect :161); `tools/sites/game/static/lobby.js`.
- **The gate + headers + asset mount:** `tools/sites/game/app.py` (`/game` gate :404, `no-cache` :373, WASM mount :448, CSP `manifest-src` :90).
- **Persistent-session intent of record:** `docs/architecture/ADR-003-pwa-first-for-alpha.md` §3 (the `localStorage` `axenstax_pubkey` cache this spec builds on).
- **Local world store (plays offline once booted):** `tools/sites/game/static/world_store.js`.
- **Sibling spec (the safety net):** [`2026-05-26-cloud-save-blossom.md`](2026-05-26-cloud-save-blossom.md) — backup + cross-device sync; complements this enabler.
- **Engine offline-capability evidence:** single-player makes zero network calls (`game/engine/src/game_loop.rs` network send/receive early-returns with no server/client); native binary runs auth-free.

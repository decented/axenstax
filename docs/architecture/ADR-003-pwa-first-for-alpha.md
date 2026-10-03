# ADR-003: PWA-First for Alpha

**Status**: Accepted
**Date**: 2026-04-17
**Supersedes (in part)**: ADR-002 §"Client Target: Native-First, WASM-Aware"

## Context

ADR-002 sequenced the client targets as *"Native desktop first, WASM-aware but native-only until gameplay loop proven."* The gameplay loop is now proven — mining, crafting, combat, mobs, world gen, lighting, water, inventory, and split-screen multiplayer all work natively (see `docs/roadmap.md` Phase 0 and Phase 1).

The critical missing piece for the project is **alpha tester feedback**. Without testers exercising the game, the team cannot validate gameplay decisions, prioritise bug fixes, or iterate toward the Phase 2–5 goals. Getting a build in front of Axolittle's friends and a small whitelisted group is the next hard gate.

Native distribution for alpha has real friction:

- Compiled binaries per OS, prerequisites on Linux (`libudev-dev`), no install path for Chromebooks
- Every tester update requires a re-download
- Alpha testers are primarily children (age 8–12) on Chromebooks, tablets, and shared PCs — Chromebooks in particular have no practical native path
- QR-based Signet login is already phone-mediated, which naturally pairs with a browser-based client

A substantial PWA alpha design and implementation plan already exists (`docs/superpowers/specs/2026-04-15-pwa-alpha-design.md`, `docs/superpowers/plans/2026-04-15-pwa-alpha.md`) — totalling ~2,500 lines. The decision here is not *how* to build the PWA (that's the plan) but whether to pull it forward as the current priority.

## Decision

**The PWA is the primary client target for alpha. Native polish is deferred until post-alpha.**

- The PWA alpha plan becomes the active workstream. Phase 1 multiplayer work that blocks PWA delivery pauses; Phase 1 work that unblocks PWA (e.g. save-format stabilisation) continues.
- Alpha browser scope is **Chromium-only** (Chrome / Edge / Chromium-based Android). Firefox and Safari are out of scope until the alpha exit criteria are met.
- **Signet-app (`mysignet.app`) is used as the sign-in mechanism only.** Deeper Signet protocol features (credentials, badges, age verification, NIP-46 signing) are NOT integrated for alpha. The integration surface is the "Sign in with Signet" redirect flow defined in `forgesworn/signet-app/README.md`: QR challenge → phone signs → callback returns pubkey + Schnorr signature.
- Alpha testers are whitelisted by Nostr pubkey on the live server. No public signup.
- The roadmap is updated to reflect this priority shift (see `docs/roadmap.md`).

## Alpha Delivery Pipeline

1. **PWA working** — WASM engine playable in Chromium at `/play/`
2. **JS-owned Signet auth** — the auth state machine runs in JS (`tools/website/static/auth.js`), not Rust. The WASM module receives an already-validated pubkey via a `wasm-bindgen` setter *before* the engine starts. Two paths:
   - **QR / relay mode** (cross-device) — `auth.js` generates an ephemeral secp256k1 session keypair, renders a QR of the auth URL (`mysignet.app` with `relay=wss://relay.trotters.cc&sessionPubkey=<hex>`), and calls `signet-verify.waitForAuthResponse` to receive the NIP-17-gift-wrapped signed auth event over the relay. The SDK performs seal-signature verification, rumor binding, origin checking. No backend poll.
   - **Same-device redirect** — a "Sign in on this device" button navigates the same browser to `mysignet.app` in redirect mode (no `relay`/`sessionPubkey` params, `callback=/auth/callback?session=<id>&same_device=1`). On return, the FastAPI callback verifies the Schnorr signature, mints an HMAC-signed token (`base64url(HMAC_SHA256(server_secret, "{pubkey}|{expires_ts}"))`, 60-s window), and redirects to `/play/#pubkey=…&expires=…&token=…`. `auth.js` POSTs to `/auth/verify-fragment`, which constant-time-checks the HMAC + single-use nonce + `X-Requested-With` header, then sets the session cookie.
3. **Persistent local session** — after first successful auth, the pubkey is cached in `localStorage` (`axenstax_pubkey`, lowercase hex, public key — not encrypted). Subsequent visits skip the auth UI entirely before first paint. A "Switch user" menu item clears the cache and reloads. Server-side trust is anchored in the `axenstax_session` HttpOnly cookie (not the `localStorage` value).
4. **Local-first world storage** — worlds are saved to IndexedDB only (DB `axenstax_worlds` v1, object store `worlds`, key `"<pubkey>:<world_name>"`). IndexedDB is accessed via a small JS helper (`world_store.js`) invoked from Rust via `wasm-bindgen`; going through `web_sys::IdbFactory` from Rust is avoided. No cloud sync for alpha. Clearing browser data loses worlds — acceptable single-device trade.
5. **Cloud save deferred** — Blossom-backed encrypted cloud save is explicitly **out of scope for alpha** and moves to a post-alpha "Save to cloud" feature (user-facing label: "Save to cloud" or "Save to axenstax.com" — not "Save to Blossom"; Blossom is an implementation detail, not a product).
6. **Live server deployment** — website + WASM bundle served over HTTPS from production host. Out of scope for the Phase-2 build spec; tracked separately.
7. **Whitelist gate** — `/play/` is served by a FastAPI route (not a static mount). Gate reads the `axenstax_session` cookie, constant-time-verifies its HMAC, checks the pubkey against `data/whitelist.txt`. Non-whitelisted pubkeys see a waitlist page; their pubkey is logged to `data/whitelist-requests.log`. `/admin/reload-whitelist` is shared-secret protected (`ADMIN_TOKEN` env var, constant-time compare, 1/sec rate limit, 503 if token is unset/short).
8. **Tester feedback loop** — daily build-test workflow (`docs/workflow/daily-build-test.md`) continues, driven by PWA-hosted builds.

### Security posture for alpha

- `HAS_SCHNORR=False` hard-fails startup (the previous format-only fallback was an auth bypass).
- `/auth/callback` locks a session permanently after 3 failed signature verifies, preventing third-party DoS of a real user's session.
- `/play/` index response carries strict CSP (`default-src 'self'; connect-src 'self' wss://relay.trotters.cc https://mysignet.app; …`), `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`.
- `axenstax_session` cookie: `HttpOnly; Secure; SameSite=Strict; Path=/`, 4-hour expiry.
- `auth_success.html` uses `<meta name="referrer" content="no-referrer">` to prevent `signature=` / `token=` leaking to next-request Referer.
- The HMAC secret for fragment tokens is read-or-created at startup into `data/fragment_hmac.key` (mode 0600), never regenerated on subsequent boots.

## Rationale

### Why now, not after Phase 2/3

- **Feedback dependency**: Phase 2 (Hosted Worlds) and Phase 3 (Gameplay Depth) both need tester input to prioritise. Building them without feedback risks wasted effort.
- **Distribution friction is the blocker**: The native build works. What's missing is a frictionless way for a tester to *try* it. PWA removes that friction at the exact moment the game is worth trying.
- **Chromebook reach**: A significant portion of Axolittle's friend group is on Chromebooks. Native builds can't reach them at all.
- **WASM readiness**: The existing PWA design notes the WASM build is 97% ready. The incremental cost to push it over the line is small relative to the tester-feedback value unlocked.

### Why Chromium-only for alpha

- **WebGPU stable in Chromium**; Firefox still variable, Safari 18+ only on recent Apple hardware
- `wgpu` has no clean WebGL fallback — supporting non-WebGPU browsers means a separate rendering path we don't have time to build
- Alpha is invite-only, so dictating the browser is acceptable. Broader browser support becomes a Phase 5-ish concern.

### Why local-first world storage, not Blossom, for alpha

- The primary alpha use case is **build at home, play at home** on a single device. Cross-device sync is not a day-one requirement.
- Blossom adds substantial complexity: upload proxy, kind-24242 auth signing, hash addressing, manifest storage, encrypted archive format. Roughly half the remaining PWA plan work.
- IndexedDB is dramatically simpler: synchronous, instant, no server round-trips, no encryption key threading. Better for the gameplay feel during alpha.
- Post-alpha, cloud save becomes a user-visible *feature* ("Save to cloud") — useful for marketing/differentiation rather than a silent infrastructure requirement. Label must be user-friendly ("Save to cloud" or "Save to axenstax.com"), never "Blossom" (implementation detail).
- Trade-off accepted: alpha testers who clear browser data lose their worlds. This is acceptable feedback signal ("would you pay for cloud sync?") and matches how most single-player PWA games behave.

### Why Signet-app as auth only (not deeper integration)

- The redirect flow is minimal and well-documented — scope is bounded
- Deeper Signet features (credentials, age proofs) are valuable but not gating for alpha gameplay feedback
- Keeping the integration surface thin means a Signet-app breaking change can't block alpha
- Age verification for Bitcoin features is a Phase 5 concern, not alpha

### What this does NOT change

- **Native remains a first-class target long-term.** This ADR reprioritises alpha delivery, not the overall platform vision. Native desktop, console hardware (Pi 5, N100), and companion-screen handhelds all stay on the roadmap.
- **Multiplayer is still the critical path for Phase 2+.** Alpha is single-player PWA; multiplayer-over-web is a follow-up.
- **Engine architecture stays WASM-aware as designed** (ADR-002). That design decision is what makes this pivot cheap.

## Consequences

- **CLAUDE.md** updated to flip the client-target line and add alpha browser scope.
- **`docs/roadmap.md`** updated to pull PWA Alpha forward as the active phase and renumber downstream phases.
- Specs 01 (engine arch), 03 (rendering), 04 (networking) reviewed for Chromium-PWA-first framing; any lingering "native-only" language updated.
- The PWA alpha plan (`docs/superpowers/plans/2026-04-15-pwa-alpha.md`) is the canonical execution doc. This ADR does not duplicate it.
- A live server deployment target (host, domain, TLS cert, whitelist store) becomes a near-term infrastructure need. Out of scope for this ADR; to be captured separately when deployment work starts.
- Any new engine feature must compile under `wasm32-unknown-unknown` and run in Chromium with WebGPU. PRs that break the WASM build are blockers, not background cleanup.

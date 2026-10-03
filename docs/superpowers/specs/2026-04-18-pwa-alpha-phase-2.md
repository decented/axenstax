# PWA Alpha — Phase 2 Build Spec

**Date**: 2026-04-18
**Status**: Ready to build (Phase A review amendments folded in)
**Supersedes**: the Blossom/save portions of `2026-04-15-pwa-alpha-design.md` (see ADR-003)
**Authority**: `docs/architecture/ADR-003-pwa-first-for-alpha.md`

This spec is self-contained. A fresh session starting here should be able to execute it without re-reading chat history or other planning docs. If something referenced here conflicts with older docs, this spec wins.

**Phase A amendments (2026-04-18)**: Task 1 cache moved from Rust to JS (JS must skip WASM boot on return visits). Task 2 names the real crypto lib (`@noble/curves`), vendors `signet-verify.iife.js` + `qrcode-generator` rather than trusting a CDN, and documents the one-time SDK build. Task 3 adds an HMAC-signed fragment so deep-link pubkey spoofing is blocked. Task 5 routes IndexedDB through a small JS helper rather than wrangling `web_sys::IdbFactory` from Rust, and explicitly deletes the now-unused Blossom code + HKDF encryption key. Task 6 moves whitelist enforcement server-side (cookie-gated `/play/` route) and adds admin-token auth to the reload endpoint. Open Questions 1–3 pre-answered; OQ 4 (splash UI) stays open for Axolittle. PWA installability explicitly punted to post-alpha.

**Phase B amendments (2026-04-18, holodeck consensus)**: Old Tasks 1+2 merged into a single Task 1 (JS owns both cache and relay). Old Tasks 3+4 swapped (callback template must land before the fragment handoff uses it). Security must-fixes folded in: `HAS_SCHNORR=False` hard-fails startup, `/auth/verify-fragment` uses `hmac.compare_digest` + single-use nonce + `X-Requested-With` header, `/admin/reload-whitelist` returns 503 if token is unset/short, `/auth/callback` locks sessions after 3 failed sig verifies, `axenstax_session` cookie is `HttpOnly; Secure; SameSite=Strict`, strict CSP + `X-Frame-Options: DENY` on `/play/`, `X-Player-Pubkey` removed from CORS allow-list. Performance gates added: Chromebook bundle size + parse time measurements required during Task 1. `noble-curves` vendored directly (no Signet boundary push). Dead code (encrypt_blob, derive_encryption_key, worlds.py) deleted outright rather than marked dormant. Done Definition split into alpha-ready (this spec) vs alpha-live (separate deploy).

---

## 0. Where We Are

**Committed and working** (as of commit `1cbbd4d`):
- WASM engine compiles, `trunk build` produces `dist/` with correctly-prefixed assets (`Trunk.toml` with `public_url = "/play/"`)
- `/play/` on the website (`https://localhost:8094/play/`) serves the WASM game, which boots in Chromium with WebGPU
- Canvas backing-store sizing is correct: explicit `set_width`/`set_height` from viewport dims, a window-resize listener, and a post-init `renderer.resize` syncthesis the wgpu surface to canvas dims
- The existing QR-auth flow round-trips end-to-end: website generates a challenge, WASM draws a QR, phone scans + signs via `mysignet.app`, callback reaches the website, WASM's polling loop sees the completed session
- ADR-003 captures the pivot to PWA-first alpha with Chromium-only scope, local-first storage, Signet-app as auth mechanism only, and Blossom deferred
- CLAUDE.md, `docs/roadmap.md` (Phase 1α NOW), and `docs/spec/04-networking.md` are aligned
- `docs/superpowers/plans/2026-04-15-pwa-alpha.md` has Phase C marked DEFERRED

**Signet-app delivered cross-device relay delivery** (their commits `b50c847`, `e7c3f55`, `67162d4` on `main`):
- Opt-in: consumer supplies `relay` (wss:// URL) + `sessionPubkey` (64-hex) URL params
- Publishes a NIP-17 gift-wrapped kind-29999 event containing a JSON `AuthResponse` with the full signed kind-21236 inner auth event
- In-app acknowledgement card instead of redirect when in relay mode
- Redirect mode unchanged; fully backward compatible
- Consumer SDK helper `waitForAuthResponse()` available in `forgesworn/signet-verify` (not the monorepo copy) — handles relay subscribe + NIP-17 unwrap + signature verification + origin/challenge validation

**Not yet done** (this spec covers it):
1. Doc touch-ups to reflect relay (not POST) as the direct-mode mechanism
2. Persistent local session — cache pubkey in IndexedDB, skip auth on return visits
3. Integrate Signet relay mode via JS (using `signet-verify.waitForAuthResponse`)
4. Same-device redirect button on the splash
5. Friendly `/auth/callback` landing page (for redirect-mode dead-ends on phone)
6. Local-first IndexedDB worlds (Phase C-lite replacing deferred Blossom)
7. Whitelist gate at `/play`

**Explicitly out of scope for alpha:**
- Blossom or any cloud save
- WebRTC browser multiplayer
- Firefox / Safari support
- Deeper Signet integration (credentials, age gates, NIP-46)
- "Authorized sites as app launcher" flow (future Signet UX work, not ours)
- **PWA installability** (manifest.webmanifest, service worker, install icons, "Add to home screen"). We're "PWA" in the sense of *browser-delivered app*, not in the sense of *installable offline app*. Punted because it's additive and doesn't gate tester feedback. Post-alpha task — see "Post-alpha note: offline / car-trip play" below for the motivating use case and the recommended implementation strategy.

**Post-alpha note: offline / car-trip play (added 2026-04-19)**

Motivating use case: kids play in the car with no WiFi. Worlds are already local (IndexedDB per pubkey) and the cached pubkey in `localStorage` already makes return visits skip auth — so the *only* missing piece is reliably serving the WASM/JS bundle when the network is gone. Today the browser cache *might* keep it; a service worker guarantees it.

**Trigger to do the work**: the week before a planned car trip, OR when build cadence slows to a few deploys a day. During active iteration the cache-first layer fights you (stale-bundle questions during tester feedback).

**Recommended implementation (best-practice caching strategy):**
- `manifest.webmanifest` + install icons — install-to-home-screen, fullscreen launch, no browser chrome. Also gives the bundle slightly stickier cache treatment on Chromebook even before the service worker lands, so manifest-only is a valid cheaper intermediate step (~30 min).
- Content-hashed assets (Trunk's `axenstax-<hash>.wasm`, `axenstax-<hash>.js`) → **cache-first, indefinite TTL**. Safe because the filename changes when the content changes.
- `/play/` entry HTML → **network-first with cache fallback**. Must point at the current hashes; a stale HTML pinned from cache would serve old WASM references.
- API calls (`/auth/*`, `/api/wasm-error`, `/auth/whoami`) → **network-only, never cached**. Auth and telemetry must not be served from cache.
- Update flow — `self.skipWaiting()` + `clients.claim()`, cache names tagged with build hash, visible "update available — reload" prompt. Done right, a new build takes effect within one navigation. Done wrong, users get stuck on old versions and you debug at a distance.
- First auth still needs internet (one trip to the Signet relay to mint the pubkey). After that, return sessions are zero-network.

Estimated effort: ~half to one focused day for the full service-worker strategy; ~30 min for manifest-only if that's all we need. The vendored-JS decision in §4 (no CDN for `signet-verify.js` / `noble-curves.js` / `qrcode-generator.js`) was made specifically to keep the offline story clean — that groundwork is already done.

---

## 1. Guiding Principles (read before every task)

1. **Spec-aligned, not demo-aligned** — per CLAUDE.md "Concrete, Not Cards." Every piece is production-grade. If a shortcut looks tempting, mark it `BRIDGE:` with the replacement plan.
2. **Chromium-only during alpha** — don't spend effort on Firefox/Safari fallbacks. Don't add WebGL fallback paths for wgpu.
3. **Signet is an auth mechanism only** — no credential, badge, age-proof, or NIP-46 integration for alpha. If a change seems to push features into Signet, stop and re-read `feedback_signet_boundary` in memory.
4. **Local-first** — worlds in IndexedDB. Saves are instant and synchronous from the user's perspective. No Blossom, no cloud proxy, no encryption key threading for alpha.
5. **Persistent session is a home-device feature** — the kid on their own PC should not re-auth on every visit.
6. **Don't build anything not explicitly scoped** — per CLAUDE.md "Do NOT build anything unless explicitly asked." Each task in §3 IS the explicit ask. Don't add surrounding features beyond them.
7. **Specs are the source of truth** — when a task changes behaviour, update the relevant spec alongside the code, not after.

---

## 2. Build Environment Reference

Working directory: `<repo>`

**Spin-up (three services, all run from this laptop since `<workspace>/` is not mounted here):**

```bash
# Website (VM-only on the other laptop, but runs natively here):
cd <repo>/tools/website && .venv/bin/python app.py
# Voice server:
cd <workspace>/Decented/voice-server && node server.mjs
# Engine WASM build (host):
cd <repo>/game/engine && PATH=$HOME/.cargo/bin:$PATH trunk build
```

Service URLs:
- Website + game: `https://localhost:8094` (HTTPS with self-signed cert)
- `/play/` — WASM game
- `/auth/challenge` (POST), `/auth/callback` (GET), `/auth/poll/{session_id}` (GET)
- `/api/wasm-error` (POST) — in-browser error reporter **(REMOVED 2026-10-03: no server-side crash log; errors show on the page + console only)**
- Voice server: `http://localhost:4100`

Tools:
- Rust: `~/.cargo/bin/cargo` (not on PATH by default in non-interactive shells)
- Trunk: `~/.cargo/bin/trunk` (same)
- Node 18+: `/usr/bin/node` (installed today)

---

## 3. Gotchas Learned Today (do not re-learn)

1. **Canvas sizing race.** On WASM, `winit 0.30`'s ResizeObserver fires a `Resized` event during async `GameState::new()` before our state exists; the early-return in `window_event()` drops it, and the wgpu surface stays at 1x1. Fix (already in): explicit `canvas.set_width/set_height` after insertion + post-init renderer resync in the "state became ready" branch. Don't remove any of that code without replacing it.

2. **Browser cache stubbornness with `/play/`.** Brave (and Chrome) aggressively cache the WASM bundle. When testing, use **devtools → Application → Clear Storage → Clear site data → reload**. Regular Ctrl+Shift+R is not always enough. The user needs to know this — surface it in the test sheet.

3. **Brave Shields.** The default Shields block WebGPU and the error-reporting fetch. Tester instructions must say "Shields Down for `localhost:8094`" (and later for the prod domain). This is a one-click thing in Brave; not a code change.

4. **WASM error reporting is silent unless routed right.** `index.html:47` posts to `/api/wasm-error` (relative). If someone changes it to an absolute URL pointing at a host the browser can't reach, errors vanish. Keep it relative.

5. **`trunk build` must run in `game/engine/`.** It finds `Trunk.toml` via cwd. A build from a different directory will fail with `Unable to find any Trunk configuration`. Always `cd game/engine && trunk build`.

6. **The website is running without restart on commits.** Python auto-reload isn't on. Restart it after changes to `tools/website/*.py`.

---

## 4. Tasks

Execute in this order. Each task has a goal, files, scope boundaries, and acceptance. Don't jump ahead.

### Task 0 — Doc touch-ups

**Goal**: Reflect the *actually-delivered* Signet mechanism (relay + NIP-17) rather than the originally-proposed POST in all planning and engine-spec docs.

**Files**:
- `docs/architecture/ADR-003-pwa-first-for-alpha.md` — rewrite the "Alpha Delivery Pipeline" section to describe relay + NIP-17 + JS-owned auth + HMAC fragment handoff.
- `docs/superpowers/specs/2026-04-15-pwa-alpha-design.md` — add a 2026-04-18 note at §1 (architecture overview): "Auth delivery: JS-side `waitForAuthResponse` via `signet-verify`. Save: local IndexedDB (no Blossom for alpha)." Leave the detailed Blossom sections as-is (preserved for post-alpha reference).
- `docs/superpowers/plans/2026-04-15-pwa-alpha.md` — add a `> SUPERSEDED BY 2026-04-18-pwa-alpha-phase-2.md` banner at the top of the file. Phase B tasks (backend-poll flow) are inactive for alpha.
- `docs/roadmap.md` — one-line refinement: Phase 1α Signet bullet → "QR (cross-device relay mode) + same-device button (redirect mode), both live."
- `docs/spec/04-networking.md` — add a "WASM alpha auth" subsection: JS owns auth, WASM receives already-validated pubkey via `wasm-bindgen` setter. `wasm_auth.rs` is a thin Rust shim, not a state machine.
- `docs/spec/02-world-format.md` — add a "WASM local storage" subsection: DB `axenstax_worlds` v1, object store `worlds`, key `"<pubkey>:<world_name>"`, index `by_pubkey`, record shape `{ blob: Uint8Array, meta: { name, size, last_saved, game_mode } }`. Pin the `WorldEntry` shape here so Task 4 has canonical reference.
- `docs/spec/08-security-anti-cheat.md` — add "Alpha auth threat model" subsection: HMAC fragment handoff (prevents deep-link pubkey spoof), cookie-gated `/play/` (prevents whitelist bypass), admin-token endpoint hardening, session-lock after failed sig verify.

**Scope**: Doc edits only. No code. Do not delete the Blossom sections from the 2026-04-15 design doc (preserved for post-alpha).

**Acceptance**:
- A fresh reader of ADR-003, spec/04, spec/02, spec/08 gets consistent relay-mode-first picture.
- The old direct-mode/POST proposal survives only in `docs/integrations/signet/2026-04-18-direct-auth-mode.md` (historical).
- `WorldEntry` shape is pinned in spec/02 so Task 4 can reference it.

---

### Task 1 — JS-side auth module (cache + relay + UI)

**Goal**: All auth logic — pubkey cache read, relay-mode subscribe, NIP-17 unwrap, WASM handoff — lives in a small JS module (`auth.js`). The WASM module receives an already-validated pubkey via a `wasm-bindgen` setter *before* `run()` is called. `wasm_auth.rs` collapses to a thin shim.

**Why this replaces old Tasks 1 + 2**: the Phase A amendment moved the pubkey cache to JS. At that point the "persistent local session" task and the "relay-mode integration" task share an owner (`auth.js`) and the same state machine. Keeping them separate forces a throwaway intermediate state. Combined here.

**Pre-work — build the SDK IIFE + vendor noble-curves** (one-off, before any Rust changes):
```bash
# 1. Build signet-verify IIFE.
cd <workspace>/forgesworn/signet-verify
npm install
npm run build
cp dist/signet-verify.iife.js <repo>/tools/website/static/signet-verify.js

# 2. Vendor noble-curves IIFE (we need schnorr.getPublicKey + randomPrivateKey for
#    the ephemeral session keypair — the signet-verify IIFE bundles noble internally
#    but doesn't expose it on `window.Signet`). Building a tiny wrapper:
mkdir -p /tmp/noble-wrap && cd /tmp/noble-wrap
npm init -y
npm install @noble/curves@^1.8 esbuild
cat > src.js <<'EOF'
import { schnorr } from '@noble/curves/secp256k1';
import { bytesToHex } from '@noble/hashes/utils';
window.AxeNoble = { schnorr, bytesToHex };
EOF
npx esbuild src.js --bundle --format=iife --outfile=dist.js --minify --target=es2020
cp dist.js <repo>/tools/website/static/noble-curves.js
```
Commit both vendored files. **Do not** ask `signet-verify` to add a consumer-specific `generateSessionKeypair()` helper — per the Signet-boundary memory, AxeNStax-shaped requests stay on our side.

**Pre-work — vendor the QR lib**:
```bash
# qrcode-generator is a zero-dep IIFE, ~4 KB. Fetch once and commit.
curl -fsSL https://raw.githubusercontent.com/kazuhikoarase/qrcode-generator/master/js/qrcode.js \
    -o <repo>/tools/website/static/qrcode-generator.js
```
Pin the commit SHA in a comment at the top of the vendored file. No runtime CDN.

**Files**:

- `tools/website/static/auth.js` — new. **Fully replaces** the old Rust-side state machine. Handles:
  1. **Cached-pubkey fast path**: read `localStorage.getItem('axenstax_pubkey')`. If present and 64-char lowercase hex, hide the auth UI before it paints, call `wasm.set_pubkey(hex)` after module instantiates, boot WASM. Source of truth for "am I signed in?" for UX only — the server trusts only the session cookie (Task 5).
  2. **Fragment-handoff fast path** (from Task 3): if `location.hash` matches `#pubkey=<64hex>&expires=<digits>&token=<b64url>`, POST `{pubkey, expires, token}` to `/auth/verify-fragment`. On 200: `localStorage.setItem`, `history.replaceState(null, '', '/play/')`, set pubkey, boot WASM. On fail: strip fragment, fall through to fresh auth.
  3. **Fresh auth UI**: render a centred modal over the still-hidden canvas with (a) a QR, (b) a "Sign in on this device" button, (c) an error area.
  4. **Ephemeral session keypair**: `const priv = crypto.getRandomValues(new Uint8Array(32)); const pub = AxeNoble.bytesToHex(AxeNoble.schnorr.getPublicKey(priv));`. Don't persist either.
  5. **Auth URL**: `https://mysignet.app/?auth=1&challenge=<64hex>&origin=<origin>&name=Axe%27n%27Stax&callback=<origin>/auth/callback&t=<unix>&relay=<relayUrl>&sessionPubkey=<pubHex>`. `challenge` is a freshly-generated 64-hex nonce (used as `requestId` in `waitForAuthResponse`).
  6. **QR render**: `qrcode-generator` into a `<div id="qr-slot">` child.
  7. **Same-device button**: `redirectUrl` = same auth URL with `relay` and `sessionPubkey` **both** omitted (confirmed: `signet-app` treats both-present as relay, otherwise redirect). First POSTs `/auth/challenge` to get a server-backed `session_id`, replaces `callback` with `…/auth/callback?session=<id>`, then `window.location.href = redirectUrl`.
  8. **Relay wait**: `await Signet.waitForAuthResponse({ requestId: challenge, relayUrl, sessionPrivKey: priv, expectedOrigin })`.
  9. **On resolve**: cache pubkey, call `wasm.set_pubkey(result.pubkey)`, hide auth UI, call `wasm.start()` (which invokes the existing `run()` entrypoint).
  10. **On reject**: visible error overlay + retry button. Log error code to `/api/wasm-error`.
- `tools/website/static/auth-ui.css` — new. QR modal styling. Full-viewport overlay, centred card, title, QR, button, error area. No animations during auth (the canvas is hidden; no conflict).
- `game/engine/index.html` — rewrite loader.
  - Load order in `<head>`, **no `defer`/`async`**, in this exact sequence: `noble-curves.js` → `signet-verify.js` → `qrcode-generator.js` → `auth.js`. Then the Trunk-injected `<link data-trunk rel="rust">` at the end of `<body>` (not `<head>`).
  - Do not let Trunk fetch the WASM until `navigator.gpu.requestAdapter()` resolves AND `auth.js` has a pubkey. Concretely: replace `<link data-trunk>` with a manual loader in `auth.js` that does `const mod = await import('/play/axenstax-XXXXXXXX.js'); await mod.default(); mod.set_pubkey(pubkey); mod.start();` — Trunk still emits the hashed filename; we just append the `<script>` tag manually after auth completes. Asset name can be read from a `<link rel="preload" data-trunk as="fetch" data-trunk-no-inject>` emitted at build time.
  - `<meta name="axenstax-relay" content="wss://relay.trotters.cc">` for rebuild-free relay swap.
- `game/engine/src/wasm_auth.rs` — collapse to ~40 lines. Keep: `#[wasm_bindgen] pub fn set_pubkey(hex: String)` (writes to `save::WASM_PUBKEY`). Delete: `AuthStatus` enum, `WasmAuth` struct, polling, QR texture generation, HKDF derivation, `encryption_key`/`blossom_url` fields, `fetch_challenge` / `fetch_poll` helpers.
- `game/engine/src/splash_ui.rs` — remove in-canvas QR draw. Splash becomes title-only (post-auth only; not rendered during auth because the DOM modal covers the canvas).
- `game/engine/src/main.rs` — WASM `resumed()`: read `WASM_PUBKEY`; if `None`, `log::error!` and freeze (shouldn't happen — JS enforces). Remove `auth: WasmAuth` field from `GameState`, remove any `auth.tick()` calls from `game_loop.rs`.
- `game/engine/src/menu.rs` — "Switch user" menu item. Calls `#[wasm_bindgen(js_namespace = window)] fn axenstax_switch_user()`, which `localStorage.removeItem('axenstax_pubkey')` + `location.reload()`.
- `tools/website/auth.py` — delete `/auth/poll` route and the polling code path. Keep `/auth/challenge` alive (still used by same-device redirect path). `/auth/callback` gets a session-lock after 3 failed sig verifies (permanent fail, forces re-challenge — see Task 2 hardening).

**Storage key**: `axenstax_pubkey`, lowercase hex 64 chars. Not encrypted (public key).

**Scope**:
- No Rust-side IndexedDB for the cache. `localStorage` is the right tool.
- Don't re-implement NIP-17 unwrap in Rust — SDK owns it.
- Don't cache the session privkey.
- Don't stack the DOM auth UI on top of the egui canvas QR; egui QR code is deleted, not hidden.
- Don't request a `generateSessionKeypair()` addition to `signet-verify`. Vendor noble-curves ourselves (boundary-compliant).

**Acceptance**:
- Fresh browser / clean profile: WebGPU check passes, DOM modal appears with QR + button, user scans with `mysignet.app`, SDK resolves, modal hides, canvas appears, menu loads. No backend polling anywhere in the network tab.
- Close tab, reopen `/play/`: straight to menu. No modal flash (auth UI must be hidden before first paint if `localStorage` has a valid pubkey).
- Click "Switch user" → page reloads, modal reappears, new pubkey cached.
- Incognito: modal shows each time.
- Console log sequence: `"session keypair ok"` → `"relay subscribed"` → `"QR ready"` → (scan) → `"auth response verified"` → `"wasm booting"`.
- Relay unreachable → visible error overlay (not silent).
- `window.Signet`, `window.AxeNoble`, `qrcode` global all exist before `auth.js` runs (verified by `console.assert` at top of the file).

---

### Task 2 — Callback handler + landing page

**Goal**: The phone (cross-device relay users) needs a friendly page to confirm "you're signed in, close this tab." The desktop same-device path (Task 3) uses the same callback, but redirects back to `/play/` with an HMAC-signed fragment. Build the template + the hardened callback handler first so Task 3 has a place to land.

**Files**:
- `tools/website/templates/auth_success.html` — new. Mobile-friendly, ~40 lines. Context vars passed from FastAPI: `short_pubkey` (first 16 hex chars + "…"), `redirect_target` (empty string for phone, `/play/#pubkey=…&expires=…&token=…` for desktop — populated only when the server decides to redirect). Includes `<meta name="referrer" content="no-referrer">` to prevent `signature=` / `token=` leaking into the next request's Referer.
  - Body: "✓ Signed in as `{short_pubkey}`. You can close this tab and return to your game." + inline `<script>` that does `if (redirect_target) { window.location.replace(redirect_target); }`.
  - No UA sniff on the client — the server decides whether to populate `redirect_target` based on a query param `?same_device=1` that Task 3's redirect URL sets on the `callback` param, or falls back to `navigator.userAgent` in the template if the param is absent (belt + braces).
- `tools/website/templates/auth_error.html` — new. Renders on invalid signature / expired session. "Sign-in failed — please try again from the game." No details leaked.
- `tools/website/auth.py`:
  - **Hard-fail startup if `HAS_SCHNORR=False`**. The existing format-only fallback (`auth.py:42-44`) is an auth bypass — `verify_schnorr` returns `True` without any crypto. Replace with `raise RuntimeError("secp256k1 Schnorr lib not available — refusing to start")` at module load if the import failed.
  - **Session lock after 3 failed sig verifies**: in `auth_callback`, track `sess["sig_failures"]`; on the 4th bad attempt, set `sess["status"] = "failed"` permanently (not pending). Prevents an attacker who knows a session_id from DoSing the real user's attempt.
  - Use `Jinja2Templates` to render `auth_success.html` / `auth_error.html` instead of the inline HTML currently at `auth.py:180-190`. Pass `short_pubkey` + `redirect_target`.
  - Set `axenstax_session` cookie on successful verify (4-hour expiry, `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`). Value: `base64url(HMAC_SHA256(secret, f"{pubkey}|{expires_ts}")) + "|" + pubkey + "|" + expires_ts`. **Keep `HttpOnly`** — `auth.js` doesn't read it; the server checks it on `/play/`.
  - `/auth/poll` route is deleted (see Task 1 notes).

**Scope**:
- Don't BroadcastChannel or postMessage back to the WASM app — phone can't talk to desktop's browser.
- One success template, one error template. Not a design project.
- `HttpOnly` final answer; delete any earlier text that waffles.
- No UA sniff for client-side routing if avoidable. Server populates `redirect_target` based on `?same_device=1`.

**Acceptance**:
- Phone user (relay mode): approves on Signet, lands on success page, reads short pubkey, closes tab. `redirect_target` is empty.
- Desktop user (same-device redirect): lands on callback with `?same_device=1`, server mints the HMAC token (see Task 3), populates `redirect_target`, template inline script redirects to `/play/#…`.
- Invalid signature → error page, session marked `failed` if it was the 4th failure.
- `HAS_SCHNORR=False` at startup → server refuses to start.
- `/auth/poll` returns 404.

---

### Task 3 — Same-device redirect + HMAC fragment handoff

**Goal**: Wire the "Sign in on this device" button to a redirect-mode flow that returns to `/play/` via a server-HMAC-signed URL fragment, blocking deep-link pubkey spoofing.

**Why HMAC (not a bare `#pubkey=<hex>`)**: a bare fragment lets an attacker craft `https://axenstax.com/play/#pubkey=<any>` and any victim who opens it caches that pubkey as "authenticated." Server-signed HMAC fragment + replay defence closes the hole.

**Flow**:
1. `auth.js` "Sign in on this device" click handler: POST `/auth/challenge` → `{session_id, challenge}`. Redirect URL = auth URL *without* `relay`/`sessionPubkey`, with `callback=<origin>/auth/callback?session=<session_id>&same_device=1`. `window.location.href = redirectUrl`.
2. User approves on `mysignet.app`. Phone redirects: `GET /auth/callback?session=<id>&same_device=1&pubkey=<hex>&npub=<bech32>&signature=<hex>&eventId=<hex>`.
3. `auth.py:/auth/callback`:
   - Verify Schnorr sig against stored challenge (existing code, plus the Task 2 hardening: HAS_SCHNORR hard-fail; 3-strike session lock).
   - If `same_device=1`, mint `token = base64url(HMAC_SHA256(server_secret, f"{pubkey}|{expires_ts}"))`, `expires_ts = int(time.time()) + 60`. Render `auth_success.html` with `redirect_target = f"/play/#pubkey={pubkey}&expires={expires_ts}&token={token}"`. Also set the `axenstax_session` cookie (Task 2) so `/play/` serves the WASM app once the fragment hands off.
   - If not same-device, render with empty `redirect_target` (phone just sees the success message).
4. `auth.js` on `/play/` sees the fragment → POSTs `{pubkey, expires, token}` to `/auth/verify-fragment`.
5. `auth.py:/auth/verify-fragment`:
   - **Constant-time compare** HMAC (`hmac.compare_digest`).
   - Check `expires_ts > time.time()` (not expired).
   - Check `pubkey` matches hex regex.
   - **Single-use nonce table**: `_used_fragment_tokens: set[str]` in memory, entry = `token`, cleared when `expires_ts` passes (called during `_cleanup_expired`). If the token is already in the set → 401. Otherwise add it + return 200.
   - On 200, set the `axenstax_session` cookie (same as Task 2's callback branch).
   - Require a custom header `X-Requested-With: fetch` on this request — CSRF double-submit. `auth.js` sets it explicitly.

**Server secret**: 32 random bytes. **Read-or-create** pattern at startup:
```python
HMAC_KEY_PATH = BASE_DIR / "data" / "fragment_hmac.key"
if HMAC_KEY_PATH.exists():
    server_secret = HMAC_KEY_PATH.read_bytes()
    if len(server_secret) != 32:
        raise RuntimeError("fragment_hmac.key is corrupt — delete and restart")
else:
    server_secret = secrets.token_bytes(32)
    HMAC_KEY_PATH.write_bytes(server_secret)
    HMAC_KEY_PATH.chmod(0o600)
```
Never regenerate on each boot.

**Files**:
- `tools/website/auth.py` — add `/auth/verify-fragment` (POST). Add HMAC helpers. Add the nonce table + cleanup. Hook the HMAC secret load-or-create into `configure()`.
- `tools/website/static/auth.js` — fragment-handoff fast path; include `X-Requested-With: fetch` header on the POST.

**Scope**:
- Don't store the token server-side; HMAC is stateless. The single-use nonce set is small (60s window, maybe a few entries) and can be in-memory.
- Don't extend the 60s window; same-device redirect should complete in <5s.
- Don't cookie-scope the token — it lives only in the fragment.

**Acceptance**:
- Click button → navigate to `mysignet.app` → approve → back to `/auth/callback?…&same_device=1` → success page → redirects to `/play/#pubkey=…&expires=…&token=…` → auth.js POSTs `/auth/verify-fragment` → 200 → cookie set → fragment stripped → WASM boots.
- Spoof: `/play/#pubkey=<arbitrary>&expires=9999999999&token=abc` → 401 → cache untouched, fresh auth UI shown.
- Expired: past `expires_ts` → 401.
- Replay: second POST of a valid token → 401 (nonce consumed).
- Missing `X-Requested-With` header → 401.

---

### Task 4 — Local-first IndexedDB worlds

**Goal**: Save/load worlds to IndexedDB in the browser. No network calls. Scoped per-pubkey.

**Implementation approach — JS-side storage helpers, not Rust-side `web_sys::IdbFactory`**: IndexedDB from Rust via `web_sys` is event-listener spaghetti. Write the IndexedDB layer as ~80 lines of JS in `tools/website/static/world_store.js`, expose as `window.AxeStore = { save, load, list, delete }`, and bind from Rust via `#[wasm_bindgen]`. Rust serialises the world to `Vec<u8>` via the existing `pack_world` tar+gzip helper, hands it to JS as a `Uint8Array`, JS writes the blob. Symmetric on load.

**`WorldEntry` shape** (canonical — also pinned in `docs/spec/02-world-format.md` per Task 0):
```rust
pub struct WorldEntry {
    pub name: String,       // user-visible world name
    pub size: u64,          // compressed bytes in IndexedDB
    pub last_saved: i64,    // unix seconds
    pub game_mode: String,  // "creative" | "survival"
}
```
JS mirror (JSON shape returned by `AxeStore.list`): `{ name: string, size: number, last_saved: number, game_mode: string }`.

**Files**:
- `tools/website/static/world_store.js` — new. IndexedDB wrapper. DB `axenstax_worlds` v1, object store `worlds`, key `"<pubkey>:<world_name>"`, index `by_pubkey`. Record value: `{ blob: Uint8Array, meta: WorldEntry-JSON }`. Methods: `save(pubkey, name, u8array, meta)`, `load(pubkey, name) → Uint8Array | null`, `list(pubkey) → WorldEntry[]`, `delete(pubkey, name)`. All return Promises. On `QuotaExceededError`, the promise rejects with `"quota"`; callers log via `/api/wasm-error`.
- `game/engine/src/wasm_save.rs` — rewrite. **Delete** the Blossom paths outright (`upload_world_async`, `fetch_world_list`, `download_world_async`, `CloudWorldEntry`, `encrypt_blob`, `decrypt_blob`, `MAGIC`, `NONCE_SIZE`, all related imports). **Keep** `pack_world` / `unpack_world` (tar+gzip helpers — reused). **Add** thin wrappers over `AxeStore`:
  - `pub async fn save_world_wasm(name: &str, pubkey: &str, blob: &[u8]) -> Result<(), String>`
  - `pub async fn load_world_wasm(name: &str, pubkey: &str) -> Result<Option<Vec<u8>>, String>`
  - `pub async fn list_worlds_wasm(pubkey: &str) -> Result<Vec<WorldEntry>, String>`
  - `pub async fn delete_world_wasm(name: &str, pubkey: &str) -> Result<(), String>`
- `game/engine/src/save.rs` — add `#[cfg(target_arch = "wasm32")]` branches. Async-from-sync pattern: mirror the existing `CloudWorldEntry` polling style in `menu.rs`. `save_world` on WASM kicks off an async IndexedDB write via `wasm_bindgen_futures::spawn_local`, drops the result into an `Rc<RefCell<Option<Result<_>>>>`, caller polls from `game_loop.rs`. Load is similar. Put `unpack_world` (gunzip + tar + chunk deserialise — can be 500–1500 ms for a 30 MB world on low-end hardware) behind the same async slot so a "Loading world…" frame can render while it runs.
- `game/engine/src/menu.rs` — replace `cloud_fetch` / `cloud_hashes` polling with IndexedDB equivalents. Field names: `local_fetch`, `local_loaded`. Same polling pattern.
- `tools/website/worlds.py` — **delete the file** (not just mark unused). It's Blossom-era dead code; keeping it tempts resurrection. The routes (`/worlds/upload`, `/worlds/download`, `/worlds/list`) go with it. Remove the router include from `app.py`. If we revive cloud save post-alpha we'll design fresh.

**Serialisation**: reuse `pack_world` (tar + gzip). No encryption at rest (see §scope).

**Scope**:
- No encryption at rest. Per-pubkey keyspace is the only isolation for alpha. Post-alpha encrypted-at-rest is a separate design (see ADR-003 §"Why local-first").
- No compression beyond what `pack_world` already does (gzip).
- No import/export for alpha.
- No migration from native saves to WASM saves.
- Quota: log a warning to `/api/wasm-error` on `QuotaExceededError`. No automatic pruning; "Switch user" clears.
- **Rule articulated**: delete dead code, don't keep it dormant. `derive_encryption_key`, `encrypt_blob`, `/worlds/*`, `CloudWorldEntry` all go. The alpha-only rule is: bridge code needs an explicit replacement trigger OR it's deleted. Nothing ambivalent.

**Acceptance**:
- Create world, place blocks, exit to menu, re-enter → blocks present.
- Close tab, reopen, pick world from list → blocks present.
- "Switch user" → new pubkey → empty list. Re-switch to original → worlds return.
- Autosave fires on the same interval native uses (read `game_loop.rs` during the task to confirm the actual number — don't trust a number in this spec).
- Quota-exceeded: test sheet note only; no automated test.
- `grep` for `encrypt_blob`, `derive_encryption_key`, `CloudWorldEntry`, `worlds.py` returns empty.

---

### Task 5 — Whitelist gate at `/play/` + hardening

**Goal**: Only approved pubkeys can reach `/play/`. Others see a waitlist page. Enforcement is server-side via the `axenstax_session` cookie set by Tasks 2 / 3.

**Files**:
- `tools/website/whitelist.py` — new. Loads `data/whitelist.txt` (newline-separated lowercase hex pubkeys, `#` comments) on startup into a `frozenset[str]`. Reloads on POST to `/admin/reload-whitelist`. On reload, build the new set fresh and atomic-swap the module-level reference (no torn reads under concurrent requests).
- `tools/website/app.py` —
  - **Replace** the `/play/` static mount with an explicit route: `GET /play/` → parse `axenstax_session` cookie, verify HMAC (constant-time), check `expires_ts > now`, check pubkey ∈ whitelist. If all pass: serve `game/engine/dist/index.html` with strict security headers (see below). Otherwise serve `waitlist.html`.
  - Keep a static mount on `/play/assets/` (or `/play/pkg/` — whatever Trunk emits) for `.wasm`, hashed `.js`, fonts. Ungated on purpose — the entry HTML is the gate.
  - Add `/auth/verify-whitelist` (POST `{pubkey}` → `{allowed: bool}`). UX-only; not the trust boundary.
  - Add `/admin/reload-whitelist` (POST). **Hardening**: (a) require `X-Admin-Token` header; (b) compare against env var `ADMIN_TOKEN` with `hmac.compare_digest`; (c) if `ADMIN_TOKEN` is unset OR shorter than 32 chars at startup, the route returns **503 unconditionally** (not 200); (d) rate-limit to 1/sec (in-memory counter).
  - **Strict CSP on `/play/` index response**: `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' wss://relay.trotters.cc https://mysignet.app; img-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'`. Also `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`.
  - **CORS tighten**: remove `X-Player-Pubkey` from `allow_headers` (it's client-asserted, trusted nowhere server-side — rot risk). Keep `Content-Type`.
- `tools/website/templates/waitlist.html` — new. "Thanks for your interest — Axe'n'Stax is in closed alpha. Your pubkey has been logged. We'll let you know when you're in." Short pubkey shown for confirmation. Friendly.

**Scope**:
- Flat text file + shared-secret endpoint. No admin UI, no email flow.
- Log unknown pubkeys to `data/whitelist-requests.log` (ISO timestamp + short pubkey + user-agent, one per line, cap at 10 MB — reuse the existing pattern from `app.py:400`).
- No per-IP rate limit on `/auth/verify-whitelist` (single hash lookup); revisit if abused.
- Rate-limit `/admin/reload-whitelist` to 1/sec.

**Acceptance**:
- Whitelisted pubkey completes auth → cookie set → `/play/` serves WASM → menu loads.
- Non-whitelisted pubkey completes auth → waitlist page → pubkey logged.
- Direct `GET /play/` with no cookie → waitlist page.
- Direct `GET /play/pkg/axenstax-<hash>.wasm` with no cookie → 200 (acceptable — HTML gate is the control).
- `curl -X POST /admin/reload-whitelist` without token → 401. With token → 200 + `{count}`. With empty `ADMIN_TOKEN` env → 503 regardless of header.
- Clearing `axenstax_session` cookie + reload → waitlist. Re-auth re-sets cookie.
- Browser dev-tools: `/play/` response has full CSP + `X-Frame-Options: DENY`.
- `grep X-Player-Pubkey app.py` returns empty.

---

## 5. Testing Rhythm

Per `docs/workflow/daily-build-test.md`: each task ends with a build → test sheet → user runs it → feedback → next task. For this spec:

- After **Task 1** (JS-side auth module): major test session — the auth flow rewrites. Axolittle exercises the QR path on his phone + the same-device button on desktop.
- After **Task 4** (IndexedDB worlds): confirm "my worlds persist between visits" is real.
- After **Task 5** (whitelist gate): prod deployment becomes unblocked (prod deployment itself is separate work — see §8).

The test-sheet file for each task lives at `docs/test-sheets/YYYY-MM-DD-<task-name>.md` — generate one at the end of each task.

**Measure-first gates during Task 1** (data that must land in the task's test sheet, not just assumed):
1. WASM + JS total transfer size — gzip'd and brotli'd. Gate: **<5 MB brotli** or red-flag.
2. Chromebook parse + instantiate time via DevTools Performance. Gate: **<2 s; red-flag >4 s**.
3. WebGPU `requestAdapter()` latency on the Chromebook. Record; no gate — informational.
4. `navigator.storage.estimate()` reported quota + usage on first load. Log to `/api/wasm-error` for telemetry.
5. Relay WSS handshake latency from `wss://relay.trotters.cc`. Gate: **<2 s** or the QR-scan UX feels broken.

---

## 6. Open Questions — Status (resolved in Phase A)

1. **`signet-verify.js` provenance** — **RESOLVED**: vendor the IIFE. Build `signet-verify` with `npm install && npm run build` in `forgesworn/signet-verify/`, copy `dist/signet-verify.iife.js` to `tools/website/static/signet-verify.js`, commit. No CDN (offline-hostile, supply chain). See Task 2 pre-work.

2. **QR rendering helper in the SDK?** — **RESOLVED**: no. `waitForAuthResponse` only subscribes to the relay; no QR UI. Use `qrcode-generator` (browser-ready IIFE, ~4 KB, no build step) vendored at `tools/website/static/qrcode-generator.js`.

3. **Drop `/auth/poll` entirely?** — **RESOLVED**: yes, once Task 3 lands (the fragment handoff removes the last caller). Delete the route + the polling branch in `wasm_auth.rs`. `/auth/challenge` stays (still needed as a session-table anchor for redirect-mode signature verification).

4. **Splash UI on WASM post-relay-integration** — **DEFERRED** to Axolittle. Current splash showed a logo + egui QR; post-Task-2 it just shows a title. Decide with Axolittle during the Task 2 test session whether to add a title animation, a boot progress bar, or leave it plain.

---

## 7. What NOT to touch

- The existing native (non-WASM) game path. All current behaviour on native must keep working. Every WASM-specific change goes behind `#[cfg(target_arch = "wasm32")]`.
- The server-authoritative GameServer, chunk streaming, movement, block interaction, combat, crafting, mobs, biomes — any of the Phase 0 gameplay systems. Alpha is about *distribution*, not gameplay changes.
- `tools/website/nostr_auth.py` and `auth.py` schema/signature verification — they work, the tests in `signet-app` verified the event format. Small additions OK; structural rewrites not OK.
- The Phase 1 multiplayer code (`network.rs`, `hosted_server.rs`, etc.). Paused per ADR-003.

---

## 8. Done Definition

Split into two gates — this spec delivers the first; the second unblocks testers but is out-of-scope here.

### 8a. Alpha-ready (this spec's deliverable)

All Tasks 0–5 complete, measured against local dev environment (`https://localhost:8094/play/`). Ready when:

- A clean-profile Chromium user lands on `/play/`, sees the QR + "Sign in on this device" button, completes either auth path in <20 s, plays the game, builds a world, closes the tab, reopens, and finds the world still there.
- On return visits, the auth step is skipped — straight to menu. No modal flash.
- "Switch user" works; per-pubkey world list is clean.
- Non-whitelisted pubkeys see the waitlist page; their pubkey is logged.
- No console errors during a normal play session.
- Security tests pass: HAS_SCHNORR hard-fail verified; HMAC replay defence verified; spoofed fragments rejected; admin endpoint with missing/short token returns 503; `/play/` without session cookie returns waitlist.
- Each task has its own test sheet in `docs/test-sheets/` with Axolittle's sign-off.

### 8b. Alpha-live (separate, out-of-scope for this spec)

Production deployment — domain, TLS, hosting provider, CI for trunk builds, DNS — is tracked separately. The spec explicitly does not commit to a prod-deploy date; alpha-ready is the handoff point.

Nothing beyond 8a is required for this spec. Anything further is post-alpha.

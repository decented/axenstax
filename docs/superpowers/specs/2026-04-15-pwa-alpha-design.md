# PWA Alpha — Design Spec

**Date**: 2026-04-15 (scope revised 2026-04-18)
**Status**: Design
**Builds on**: `2026-04-02-wasm-web-build-design.md` (WASM/WebGPU build)
**Scope**: Get the game playable in a browser with Signet login and **local-first world storage via IndexedDB**. Single-player only. Alpha testers only.

> **Scope revision (2026-04-18, per ADR-003):** Blossom-backed encrypted cloud save (original Section 1 "Architecture Overview" and subsequent sections) is **deferred to post-alpha**. Alpha uses local IndexedDB only. Signet login gains a **same-device redirect path** alongside the existing QR path. A **persistent local session** skips auth on return visits. See ADR-003 §"Alpha Delivery Pipeline" for the revised pipeline. The detailed Blossom architecture below is preserved for the post-alpha rebuild but is not active alpha scope.
>
> **Further revision (2026-04-18, build spec `2026-04-18-pwa-alpha-phase-2.md`):** Auth delivery is JS-side — `signet-verify.waitForAuthResponse` over a Nostr relay, not a backend `/auth/poll` loop. The WASM module receives an already-validated pubkey via a `wasm-bindgen` setter. The backend `/auth/poll` endpoint is removed. `/auth/challenge` is retained only as a session-table anchor for the same-device redirect path. World save uses a small `world_store.js` JS helper (not `web_sys::IdbFactory` from Rust). Detailed Blossom / encryption sections below are preserved for reference only — they do not describe alpha behaviour.

---

## 0. Problem Statement

The engine is mature enough for alpha testing (mining, crafting, combat, mobs, world gen, lighting, water, inventory). The WASM build is 97% ready. But there's no way for anyone to play it outside of compiling on a local machine.

**Goal**: Any alpha tester opens a browser, scans a QR code with their phone, and plays — with their worlds persisting across sessions and devices. No install, no account creation, no passwords.

**Primary users**: Children (age 8-12) on Chromebooks, tablets, and shared PCs. Axolittle and friends.

---

## 1. Architecture Overview

```
┌────────────────────────────────────────────────────────────┐
│  Browser (any device)                                      │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Website (FastAPI, port 8094)                        │  │
│  │  /play → auth gate → serves WASM game                │  │
│  │  /auth/challenge → generates challenge               │  │
│  │  /auth/poll → returns auth result                    │  │
│  │  /auth/callback → receives Signet response           │  │
│  └──────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  WASM Engine (single-player game loop)               │  │
│  │  Receives: pubkey, blossom_url (derives key locally)  │  │
│  │  save.rs → encrypt → Blossom upload (via fetch)      │  │
│  │  load.rs → Blossom download → decrypt → hydrate      │  │
│  └──────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────┘
                          │
          QR scan ────────┼────────── Blossom upload/download
                          │
┌─────────────────────┐   │   ┌──────────────────────────────┐
│  Phone (My Signet)  │   │   │  Blossom Server               │
│  Scans QR           │   │   │  PUT /upload (kind 24242 auth)│
│  Signs challenge    │───┘   │  GET /<hash>                  │
│  Hits callback URL  │       │  GET /list/<pubkey>           │
│  Keys never leave   │       └──────────────────────────────┘
└─────────────────────┘
```

### Key constraint: The WASM client has no private key

The player's Nostr private key stays on their phone (in the Signet app). The WASM client only receives the public key after authentication and derives the encryption key locally. This means:

- **Blossom uploads cannot be signed by the WASM client** — the website backend must proxy authenticated uploads, or the phone must pre-sign upload auth events.
- **Encryption/decryption happens in the WASM client** — the symmetric encryption key is derived during auth and passed to the client.

This is the critical architectural constraint. See Section 3 for the solution.

---

## 2. Component Breakdown

### 2.1 WASM Build (builds on existing WASM spec)

The `2026-04-02-wasm-web-build-design.md` spec covers the `#[cfg]` gating, Trunk.toml, and basic WASM compilation. This spec adds:

**New WASM dependencies (Cargo.toml):**
- `chacha20poly1305` — symmetric encryption (WASM-compatible, pure Rust)
- `sha2` — SHA-256 for Blossom content addressing (WASM-compatible, pure Rust)
- `hkdf` — key derivation (WASM-compatible, pure Rust)
- `flate2` — gzip compression (zstd has WASM compile issues; gzip via flate2 is proven)
- `tar` — archive creation/extraction (pure Rust, WASM-compatible)

**New web-sys features:**
- `Headers`, `Request`, `RequestInit`, `Response` — for fetch API (Blossom HTTP calls)
- `RequestMode` — for CORS handling

**Not needed for alpha:**
- `WebSocket` — auth goes through website backend, not relay
- `SubtleCrypto` — using pure Rust crypto instead

### 2.2 Website Auth Endpoints (FastAPI)

Three new endpoints on the existing website (port 8094):

**`POST /auth/challenge`**
- Generates a 32-byte random challenge (64 hex chars)
- Generates an ephemeral session ID
- Stores `{session_id: challenge, created_at, status: "pending"}` in memory (dict, not DB — alpha scale)
- Returns `{session_id, challenge, qr_data}` where `qr_data` is the compact JSON for Signet QR:

```json
{
  "type": "signet-login-request",
  "requestId": "<session_id>",
  "challenge": "<challenge_64hex>",
  "origin": "https://axenstax.io",
  "callbackUrl": "https://axenstax.io/auth/callback?session=<session_id>",
  "timestamp": <unix_seconds>
}
```

Note: Uses the explicit JSON format (not compact `"t":"sl"`) because the compact format lacks a `callbackUrl` field. The QR encodes this JSON directly. Signet app's `routeQR()` parses it as a `signet-login-request`.

**Signet validation constraints** (enforced by `routeQR`):
- `requestId`: 1-64 hex chars (use hex session ID)
- `challenge`: 16-512 chars (our 64-char hex challenge fits)
- `origin`: must be `https://` (or `http://localhost` for dev)
- `callbackUrl`: must be `https://`, origin must match `origin` field
- `timestamp`: within ±300 seconds of current time

Challenge expires after 5 minutes.

**`GET /auth/callback`**
- Receives `?session=<session_id>&pubkey=<hex>&signature=<hex>&npub=<bech32>&eventId=<hex>` from Signet redirect (on the phone's browser — this hits the server, not the game tab)
- Verifies Schnorr signature of challenge using pubkey (secp256k1)
- If valid: updates session status to `"authenticated"`, stores pubkey
- Derives deterministic world encryption key from pubkey (see Section 4)
- Returns a simple "Authentication successful, you can close this tab" HTML page (shown on phone)

**`GET /auth/poll/<session_id>`**
- WASM client polls this every 2 seconds
- Returns `{status: "pending"}` or `{status: "authenticated", pubkey, blossom_url}`
- `pubkey` is the player's hex public key (the WASM client derives the encryption key locally — see Section 4)
- `blossom_url` is the configured Blossom server URL

**Security notes:**
- Sessions expire after 5 minutes
- Session ID is 128-bit random (unguessable)
- Poll endpoint only returns data for the session that created it
- No private keys transit through the website — only pubkey and signature
- HTTPS required in production (self-signed acceptable for LAN alpha)

### 2.3 Signet Integration

**What the phone does (existing signet-app flows):**
1. Parent/player opens Signet app
2. Scans QR code displayed on game splash screen
3. Signet app parses QR via `routeQR()` → recognises `signet-login-request`
4. Approval screen shows: "Axe'n'Stax wants to sign in"
5. Player taps approve
6. Signet app signs the challenge with player's natural-person key
7. Redirects to callback URL with `pubkey`, `signature`, `npub`, `eventId`

**What the game does (new code):**
1. On splash screen, calls `POST /auth/challenge` via fetch
2. Renders QR code from `qr_data` response
3. Polls `GET /auth/poll/<session_id>` every 2 seconds
4. On `status: "authenticated"`: stores pubkey in memory, derives encryption key via HKDF (Section 4)
5. Fetches world list from Blossom
6. Transitions to world selection menu

**No changes needed to signet-app.** The existing URL redirect auth flow (`parseUrlAuthParams` → `buildAuthCallbackUrl`) handles this. The QR just contains the login request URL.

### 2.4 Blossom World Persistence

**Upload flow (save):**
1. Engine calls `save_world()` (existing function signature, WASM path)
2. Collect: world_meta.json (JSON), world.dat (bincode), all non-empty chunks (raw bytes)
3. Create tar archive in memory: `world_meta.json` + `world.dat` + `chunks/{cx}_{cy}_{cz}.chunk`
4. Compress with gzip (via `flate2` crate)
5. Encrypt with ChaCha20-Poly1305 using the deterministic world encryption key
6. POST encrypted blob to website proxy: `POST /worlds/upload?world_name=<name>`
7. Website proxy computes SHA-256, signs kind 24242 with server key, uploads to Blossom
8. Website proxy updates the player's world manifest (see below)

**Download flow (load):**
1. Fetch world list from website proxy: `GET /worlds/list`
2. User selects a world → get blob hash from manifest
3. Download via website proxy: `GET /worlds/download/<blob_hash>`
4. Decrypt with ChaCha20-Poly1305
5. Decompress gzip
6. Extract tar: world_meta.json → WorldMeta, world.dat → WorldSave, chunks → World
7. Hydrate game state

**World list flow:**
1. WASM client calls `GET /worlds/list` (website proxy)
2. Website returns `[{name, hash, created_at, last_saved, size, game_mode}]` from manifest
3. Display in menu UI (replaces the native filesystem `list_world_entries()`)

**World manifest (server-side):**
- The website backend maintains a `{pubkey → [{world_name, blob_hash, meta}]}` mapping in memory (Python dict)
- Persisted to a JSON file on disk: `data/manifests/{pubkey_hex}.json`
- Updated by the website on every upload (receives world name + new blob hash)
- This avoids the problem of content-addressed Blossom hashes being unpredictable — the website is the manifest authority for alpha

BRIDGE: For alpha, the website backend is the single manifest authority. If the website loses its manifest data, world references are lost (blobs still exist on Blossom but aren't discoverable). For production, the manifest should be a Nostr event (publishable, relay-backed, decentralised) or stored as a well-known Blossom blob with a convention-based tag. The server-side manifest is a pragmatic alpha choice, not the destination.

---

## 3. The Signing Problem

The WASM client has no private key. Blossom requires a signed kind 24242 event for uploads. Three approaches were considered:

### Option A: Website proxies uploads (chosen for alpha)

The WASM client sends the encrypted blob to the website backend. The website signs the Blossom upload auth event with a server-side Nostr key and uploads to Blossom on behalf of the player.

```
WASM client → POST /worlds/save (encrypted blob) → Website backend
Website backend → signs kind 24242 → PUT /upload → Blossom server
```

**Trade-offs:**
- Simple — no crypto in the WASM client beyond encryption
- Website has its own Nostr key (server identity), not the player's
- Blobs on Blossom are owned by the server key, not the player's key
- Player can't directly access their blobs from another Blossom client
- Acceptable for alpha — the website IS the game distribution point

**Migration path:** When NIP-46 remote signing is added to the WASM client (post-alpha), the player's phone signs the kind 24242 event via relay, and the WASM client uploads directly to Blossom. The website proxy is then retired.

### Option B: Phone pre-signs upload events (deferred)

During auth, the phone signs N blank kind 24242 events with incrementing expiry times. Website passes these to the WASM client. Client uses one per upload.

- More Nostr-pure
- Complex — phone must predict how many uploads the session needs
- Pre-signed events leak if intercepted (though they're time-limited)

### Option C: NIP-46 remote signing per upload (deferred)

WASM client sends sign_event request to phone via relay for each Blossom upload.

- Most correct
- Requires WebSocket relay client in WASM
- Requires phone to be online during play (poor UX for children — phone might be in parent's pocket)

**Decision:** Option A for alpha. The website proxies uploads. Blobs are tagged with the player's pubkey in the manifest so they're logically owned by the player even if signed by the server.

---

## 4. Encryption Scheme

### Deterministic world encryption key

Derived from the player's public key (which the WASM client receives after auth):

```
world_key = HKDF-SHA256(
    ikm    = pubkey_bytes,          // 32 bytes — x-coordinate of secp256k1 public key (hex-decoded)
    salt   = "axenstax-world-v1",   // ASCII domain separator (prevents cross-protocol key reuse)
    info   = "",                    // no additional context for alpha (per-world info added post-alpha)
    length = 32                     // 256-bit key output
)
```

This key is deterministic — same pubkey always produces the same key. Both the website backend and the WASM client can derive it independently (both know the pubkey after auth). Any device that authenticates as this player can derive the key.

BRIDGE: This is symmetric encryption keyed to a public value. It provides confidentiality against casual observers (someone who finds the Blossom hash can't read the world) but not against someone who knows the player's pubkey. For alpha testing among trusted testers, this is sufficient. For production, switch to a per-world random key wrapped with NIP-44 per-recipient (the pubkey-derived key becomes a key-encryption-key, not a data-encryption-key).

### Encryption format

```
[4 bytes: magic "AXE1"]
[12 bytes: random nonce]
[N bytes: ChaCha20-Poly1305 ciphertext]
[16 bytes: Poly1305 authentication tag (appended by AEAD)]
```

The plaintext is the gzip-compressed tar archive of the world.

### Why ChaCha20-Poly1305

- Same cipher as NIP-44 — one crypto dependency for the whole Nostr stack
- Pure Rust implementation (`chacha20poly1305` crate) compiles cleanly to WASM
- No Web Crypto API dependency (SubtleCrypto doesn't support ChaCha20)
- Fast in software (no AES-NI on WASM)

---

## 5. Save/Load Timing

### Auto-save (background, every 5 minutes)

Matches the native auto-save interval (6000 ticks at 20 TPS = 300 seconds).

On WASM, the auto-save flow is:
1. Collect world state (same as native `save_world`)
2. Create tar + compress + encrypt in memory
3. Upload to website backend via fetch (non-blocking, using `wasm_bindgen_futures`)
4. On success: update manifest
5. On failure: log warning, retry next interval

### Tab close (`beforeunload`)

The browser's `beforeunload` event fires when the tab closes. Time budget: ~2-5 seconds.

1. Save world state to IndexedDB (local cache, fast — no network)
2. Set a `pending_upload` flag in IndexedDB

On next session start:
1. Check IndexedDB for `pending_upload`
2. If found: upload cached world to Blossom before showing world list
3. Clear the flag

This provides crash recovery. The worst case is losing up to 5 minutes of progress (one auto-save interval).

### Manual save

Not included in alpha. The auto-save + tab-close cache covers the use case. The pause menu can gain a save button in a future iteration.

---

## 6. Touch Controls Integration

`touch_input.rs` already implements:
- Virtual joystick (left side of screen) for movement
- Look zone (right side of screen) for camera
- Action buttons: jump, break block, place block
- Hotbar slot selection

Integration needed in `game_loop.rs` (~50 lines):
1. Instantiate `TouchInput` in `GameState` (WASM only)
2. Forward touch events from winit to `TouchInput`
3. Convert `TouchInput` state to `PlayerIntent` each tick
4. Handle screen resize events to recalculate touch zones

This is mechanical wiring — no design decisions needed.

---

## 7. QR Code on Splash Screen

### Layout

The existing splash screen (`splash_ui.rs`) shows:
1. Title: "AXE'N'STAX" (56pt, gold)
2. Tagline: "PROOF OF PLAY" (14pt, grey)
3. Loading bar (240px wide)

For WASM auth, the flow becomes:
1. Title + tagline appear (0-0.6s, existing animation)
2. Loading bar fills (0.4-1.0s, existing)
3. QR code fades in below tagline (1.0s onwards)
4. Text below QR: "Scan with My Signet to play"
5. On successful auth: QR fades out, "Welcome, {display_name}" appears, world list loads

### QR rendering

Use the `qrcode` crate (pure Rust, WASM-compatible) to generate a QR code as a grid of modules. Render as an egui texture:
1. Generate QR matrix from auth challenge JSON
2. Convert to RGBA pixels (black modules on white background)
3. Upload as egui `TextureHandle`
4. Display as `egui::Image` widget, 150x150 pixels, centred

### Native behaviour

On native builds, the splash screen is unchanged — no QR code, no auth. The game goes straight to the world menu as it does today. All QR/auth code is `#[cfg(target_arch = "wasm32")]` gated.

---

## 8. Website Changes

### New route: `/play`

Serves the WASM game. The page:
1. Loads `index.html` from the Trunk build output (`dist/`)
2. Includes the `.wasm` and `.js` bundles
3. Passes configuration to the WASM client via `data-*` attributes or a global JS object:
   - `blossom_url`: configured Blossom server URL
   - `auth_endpoint`: `/auth` (relative to website origin)

### New API endpoints

As described in Section 2.2:
- `POST /auth/challenge`
- `GET /auth/callback`
- `GET /auth/poll/<session_id>`

### New upload proxy endpoint

- `POST /worlds/upload` — receives encrypted blob from WASM client, uploads to Blossom
- `GET /worlds/download/<hash>` — proxies Blossom download (avoids CORS issues)
- `GET /worlds/list/<pubkey>` — proxies Blossom list (avoids CORS issues)

BRIDGE: The proxy endpoints exist because Blossom servers may not set CORS headers for the game's origin. The proxy also handles Blossom auth signing (server key). When the WASM client gains direct Blossom access (post-alpha), the proxy can be retired or kept as a fallback.

### Blossom server configuration

The website reads `BLOSSOM_URL` from environment (`.env` file). For alpha, this points to the local Blossom server (`http://192.168.1.10:3000` — the Fathom dev instance) or a public server (`https://blossom.primal.net`).

---

## 9. Visibility Model (Design, Not Alpha Implementation)

Documented here for completeness. Only **Private** is implemented in alpha.

| Visibility | Blob | Nostr event | Who can access | Alpha? |
|-----------|------|-------------|----------------|--------|
| **Private** | Encrypted | None | Only the player | Yes |
| **Friends** | Encrypted | Gift-wrapped DM with key | Approved contacts | No |
| **Unlisted** | Unencrypted | None (hash shared manually) | Anyone with hash | No |
| **Public** | Unencrypted | Published to relays | Anyone | No |

### Child safety gating (future)

| Visibility | Adult (Tier 3+) | Child Stage 1-2 | Child Stage 3 | Child Stage 4-5 |
|-----------|----------------|-----------------|---------------|-----------------|
| Private | Yes | Yes | Yes | Yes |
| Friends | Yes | Guardian shares | Yes, guardian notified | Yes |
| Unlisted | Yes | No | No | Guardian approval |
| Public | Yes | No | No | Guardian approval |

---

## 10. What's NOT in Alpha

Explicitly deferred:

| Feature | Why deferred | When |
|---------|-------------|------|
| Multiplayer (WebRTC) | Large build, needs server-authoritative physics | Post-alpha |
| PWA manifest + service worker | Polish, not required for browser play | Post-alpha |
| Audio on web | Web Audio API integration, separate concern | Post-alpha |
| Gamepad on web | Gamepad API, lower priority than touch | Post-alpha |
| NIP-46 remote signing | Requires WebSocket relay client in WASM | Post-alpha |
| Direct Blossom uploads | Requires NIP-46 or pre-signed events | Post-alpha |
| World sharing (Friends/Unlisted/Public) | Requires NIP-44 key wrapping, guardian approval | Post-alpha |
| Custom Nostr event kind for worlds | Don't commit to a format before knowing what metadata matters | Post-alpha |
| Signet tier/age verification | No Bitcoin features to gate yet | Post-alpha |
| Per-world random encryption keys | Current pubkey-derived key is sufficient for private-only | Post-alpha |
| IndexedDB world cache | Only the `beforeunload` emergency cache uses IndexedDB | Post-alpha |

---

## 11. Dependency Map

```
WASM cfg gating (existing spec)
    │
    ├── Trunk.toml + build pipeline
    │       │
    │       └── /play route serves dist/
    │
    ├── Touch controls integration
    │       │
    │       └── Playable in browser (single-player, no persistence)
    │
    ├── Website auth endpoints ──── Signet QR on splash screen
    │       │                              │
    │       └──────────── Auth flow working (scan → play)
    │
    ├── Crypto crates (chacha20poly1305, sha2, hkdf, tar, flate2)
    │       │
    │       └── Encryption/decryption working in WASM
    │
    └── Blossom proxy endpoints ──── save.rs WASM stubs → Blossom
            │
            └── World persistence working (save → load across sessions)
```

Build order follows dependency arrows top-to-bottom.

---

## 12. Implementation Phases

### Phase A: WASM builds and runs in browser
- Apply `#[cfg]` gating per existing WASM spec
- Create `Trunk.toml`
- Wire touch controls into game loop
- Add `/play` route to website serving `dist/`
- **Exit criteria**: Game renders in Chrome, touch controls work, single-player playable

### Phase B: Signet auth flow
- Add auth endpoints to website (`/auth/challenge`, `/auth/callback`, `/auth/poll`)
- Add QR rendering to splash screen (WASM only)
- Add auth polling to splash screen
- **Exit criteria**: Scan QR with Signet app → game knows player's pubkey

### Phase C: Blossom world persistence
- Add crypto crates to Cargo.toml (WASM target)
- Implement tar + compress + encrypt + upload in save.rs (WASM path)
- Implement download + decrypt + decompress + extract in save.rs (WASM path)
- Add proxy endpoints to website (`/worlds/upload`, `/worlds/download`, `/worlds/list`)
- Implement manifest blob for world list
- Implement auto-save timer (WASM path)
- Implement `beforeunload` → IndexedDB cache
- **Exit criteria**: Create world → play → close tab → reopen → world loads from Blossom

### Phase D: Polish and test
- Test on Chromebook (Axolittle's device)
- Test on tablet (touch controls)
- Test on desktop Chrome (keyboard/mouse)
- Performance profiling (WASM bundle size, load time, save/upload time)
- Error handling (network failures, Blossom downtime, auth timeout)
- **Exit criteria**: Axolittle can play on his Chromebook, world persists

---

## 13. Estimated World Save Sizes

| World | Chunks | Raw | Compressed (deflate) | Encrypted overhead | Upload size |
|-------|--------|-----|---------------------|-------------------|-------------|
| New (just created) | ~50 | 400 KB | ~40 KB | +32 bytes | ~40 KB |
| Small (30 min play) | ~200 | 1.6 MB | ~160 KB | +32 bytes | ~160 KB |
| Medium (2 hours) | ~1,000 | 8 MB | ~800 KB | +32 bytes | ~800 KB |
| Large (extensive exploration) | ~5,000 | 40 MB | ~4 MB | +32 bytes | ~4 MB |

All well within Blossom's 100 MB limit. Upload at ~1 Mbps (conservative): small = instant, medium = 6 seconds, large = 30 seconds.

Auto-save should be non-blocking (background fetch). The 5-minute interval means the upload completes well before the next save.

---

## 14. Security Considerations

| Threat | Mitigation |
|--------|-----------|
| Session hijacking (someone guesses session ID) | 128-bit random session ID, 5-minute expiry |
| Replay attack (reuse old QR) | Timestamp in QR, ±300s validation window |
| Man-in-the-middle (intercept auth callback) | HTTPS required in production |
| Blob tampering on Blossom | ChaCha20-Poly1305 AEAD — any modification fails authentication |
| Encryption key compromise | Key derived from pubkey (public value) — provides confidentiality against casual access, not targeted attack. Acceptable for alpha. Production: per-world random keys. |
| World data leaks child behaviour | Encrypted by default, no public metadata events in alpha |
| Blossom server compromise | Blobs are encrypted — server sees only ciphertext. No PII in blobs. |
| Phone lost/stolen | Signet app has PIN/biometric lock, auto-lock after 15 min |
| Tab close data loss | IndexedDB emergency cache, uploaded on next session start |

---

## 15. Open Items for Post-Alpha

- Define custom Nostr event kind for world metadata/discovery
- Implement NIP-46 remote signing in WASM (retire website proxy)
- Implement per-world random encryption keys with NIP-44 key wrapping
- Implement Friends/Unlisted/Public visibility with guardian approval
- Implement world forking (download someone's public world, save as own)
- Add PWA manifest + service worker for installability
- Add Web Audio API support
- Add Gamepad API support
- Add WebRTC multiplayer transport
- Implement Signet tier checking for feature gating

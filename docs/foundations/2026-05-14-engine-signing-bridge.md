# Engine Signing Bridge — consumer-side wire-up for upstream Signet NIP-46 `sign_event`

**Status**: GATED ON UPSTREAM — ready to execute the day Signet bunker ships `sign_event` (currently tracked as Sentinel **AS-005** / **D-003** deferred-to-Phase-5-Bitcoin-lead-in). Each phase below is self-contained; once unblocked, total wire-up is ~1-2 days.
**Date**: 2026-05-14
**Branch**: `feat/engine-signing-bridge` once started; off `main`.
**Session**: Fresh — implementer should treat this doc as the only brief.
**Trigger**: Spec 1 Phase 4 BLOCKED on this signing bridge (CLAUDE.md tech-debt bullet 5; foundation spec `2026-04-20-engine-signet-auth.md` §"Phase 4 prerequisite"). This spec is the AxeNStax-side half of the resolution; the other half is upstream Signet (`docs/integrations/signet/2026-05-05-nip46-signing-bunker-upstream.md`, issue body ready to file).
**What this spec is NOT**: This does **not** spec the cross-platform Rust `signet-nip46-client` crate. Per signet boundary, that crate is Forgesworn-owned infra and ships post-alpha as Charter Phase 2's deliverable (per Charter spec rev. 3 §Q5). When that crate lands, it supersedes the native portion of this bridge; the WASM JS-interop portion of this spec stays viable in parallel.

---

## TL;DR

When upstream Signet bunker exposes a generic `sign_event` NIP-46 handler (per the upstream issue body), AxeNStax needs **three small wire-ups** to flip `USE_SIGNET_AUTH=true` and ship Spec 1 Phase 4:

1. **`auth.js` window hook** — publishes `window.__axenstax_sign_auth_event(challenge_hex, origin) → Promise<{event_json}>` that routes through the user's already-paired bunker session via `nostr-tools/nip46`'s `BunkerSigner.signEvent`.
2. **`wasm_auth.rs` engine-side caller** — an async `sign_auth_event(challenge, origin) -> Result<SignetAuthEvent, BridgeError>` that calls the window hook + parses the returned JSON into the existing `SignetAuthEvent` shape.
3. **`RemoteClient::connect` integration** — before sending `JoinRequestPacket` under `USE_SIGNET_AUTH=true`, populate `auth_event` from the signing bridge instead of leaving it `None`.

Then the Spec 1 Phase 4 cutover (flip the flag, drop the `player_name` BRIDGE) is unblocked.

**WASM-only.** Native client gap remains; resolves when Forgesworn's `signet-nip46-client` Rust crate ships (Charter Phase 2, post-alpha, 6-8 week window). Spec 7 below documents what swapping over looks like.

Total scope (excluding upstream + native): ~250 lines code + tests + spec edits. 6 phases + 1 deferred phase.

---

## Why this lives here

- **Spec 1 Phase 4 is the single biggest named BRIDGE in CLAUDE.md** (tech-debt bullet 5). T-NP-LEAK threat from Spec 8 §9.0.1 mitigates the moment Phase 4 ships. This signing-bridge wire-up is the one thing standing between Phase 4 staying gated and Phase 4 shipping.
- **The upstream side is already drafted** — issue body is paste-ready at `docs/integrations/signet/2026-05-05-nip46-signing-bunker-upstream-issue-body.md`. The deferral (D-003) is the calendar gate, not the protocol design.
- **Bounded scope, AxeNStax-owned.** Nothing in this spec touches Signet-internal protocol design. Per signet boundary, all Signet changes are general-purpose upgrades (the `sign_event` capability is the canonical NIP-46 method — not AxeNStax-specific). This spec covers only AxeNStax's consumption of that generic capability.
- **Charter rev. 7 precedent.** `auth.js` + the vendored `nostr-tools-nip46.iife.js` already implement the BunkerSigner pattern (see Charter rev. 5 groundwork preserved on `preserved/charter-rev5-groundwork`, dormant on main). This spec reuses the same pattern for a different kind (kind-21236 auth events vs Charter's signing-not-used path).
- **Cross-game lift** per shared infra strategy: the JS-side window hook pattern is generic — any AxeNStax-internal game (other games on the same primitives) running in the same browser context can call the same `__axenstax_sign_auth_event` hook by namespace-renaming, or the lobby can expose a generic `__signet_sign_event(kind, content, tags) → Promise<EventJson>` that any consumer uses.

---

## Context pointers

### Existing code surfaces this touches

- **`tools/sites/game/static/auth.js`** — lobby auth flow. Already manages a Signet session (cookie-bound) and has access to the user's bunker URI. Currently does **not** expose any window hook for engine WASM signing requests. This spec adds one.
- **`tools/sites/game/static/vendor/nostr-tools-nip46.iife.js`** — vendored 71 kB IIFE bundle (delivered 2026-05-08 by Spec 7, see `docs/foundations/2026-05-08-site-build-pipeline-for-npm.md`). Exposes `window.NostrTools.nip46.BunkerSigner`. Currently dormant on main (Charter rev. 7 routes around it). This spec re-activates it.
- **`game/engine/src/wasm_auth.rs`** — engine-side WASM auth glue, ~76 lines. Currently RECEIVE-ONLY: publishes `__axenstax_set_pubkey(hex)` and `__axenstax_start()` for `auth.js` to call. This spec adds an outbound call path: `sign_auth_event(challenge, origin)`.
- **`game/engine/src/signet/event.rs` + `signet/verify.rs`** — `SignetAuthEvent` type + Schnorr-verify. Already used by the server-side `verify_join_signet_auth` path. This spec returns events of this shape from the bridge.
- **`game/engine/src/network.rs` (RemoteClient::connect)** — the call site that builds + sends `JoinRequestPacket`. Currently passes `("Player", PROTOCOL_VERSION, ...)` per `game_loop.rs:520`. This spec inserts an `await sign_auth_event(...)` before constructing the packet.
- **`game/engine/src/protocol.rs`** — `JoinRequestPacket.auth_event: Option<SignetAuthEventWire>` already exists (delivered by Spec 1 Phase 2/3). Currently always `None` from the WASM client. This spec flips that to `Some(...)` when `USE_SIGNET_AUTH=true`.
- **`game/engine/src/lib.rs`** (or wherever `USE_SIGNET_AUTH` lives) — feature flag still at `false`. This spec doesn't flip it; the Spec 1 Phase 4 cutover does, *after* this spec's wire-up ships.

### Related specs

- **`docs/foundations/2026-04-20-engine-signet-auth.md`** (Spec 1) — Phase 4 prerequisite §. This bridge is the prerequisite. When this spec ships, Phase 4 is unblocked.
- **`docs/integrations/signet/2026-05-05-nip46-signing-bunker-upstream.md`** + **`-issue-body.md`** — upstream Signet proposal. Defines the wire shape this spec consumes.
- **`docs/foundations/2026-05-09-charter-rev7-integration.md`** — Charter rev. 7 routes around `sign_event` for Phase 1 (relay-read mechanism A). Future Charter mechanisms B/C/D will share this bridge.
- **`docs/foundations/2026-05-08-charter-pairing-storage.md`** — IDB schema + module preserved on main as dormant groundwork. The bunker URI persistence pattern from there is reusable here (auth.js already manages the URI via the Signet session, but the IDB module is available if a more durable store is needed).
- **`docs/foundations/2026-05-08-site-build-pipeline-for-npm.md`** — Spec 7, delivered. The vendored `nostr-tools-nip46.iife.js` it shipped is what this spec consumes.

### Memory pointers

- charter phase4 shared gap — Spec 1 Phase 4 + (formerly) Charter Phase 1 shared this gap. Charter rev. 7 decoupled Charter; Spec 1 Phase 4 is now the sole downstream consumer until future Charter mechanisms.
- signet boundary — load-bearing: Signet upstream changes must be general-purpose. `sign_event` IS the generic NIP-46 method; no boundary violation. AxeNStax-side wire-up is the consumer pattern, also generic.
- shared infra strategy — cross-game lift via namespace-genericised window hook (see §6 below).
- pretest check — implementer must re-verify `wasm_auth.rs` surface area + auth.js session state before each phase. Both files have evolved.
- alpha launch posture — axenstax.app is not live; the bridge is still LAN-multiplayer-only at first ship.

### What does NOT exist yet (and this spec assumes will exist)

- **Upstream Signet bunker `sign_event` handler.** Per the upstream issue body, `mysignet.app` needs:
  - `connect` handler that establishes a NIP-46 session keyed to the consumer pubkey
  - `sign_event` handler that prompts the user (per-request approval card) for each event sign, returns Schnorr-signed Nostr event JSON, supports a per-consumer kind allow-list
  - Kind-21236 (engine auth events) must be on the allow-list for the AxeNStax consumer entry
  - **D-003 deferred this 2026-05-07**; revisit at Phase 5 Bitcoin lead-in or earlier per Sentinel cadence
- **Bunker URI persistence in the engine WASM context.** Today, `auth.js` knows the bunker URI from the Signet session; the engine WASM doesn't query it directly. This spec keeps that boundary — `wasm_auth.rs` never sees the URI, only calls the JS hook which round-trips through `auth.js`.

---

## Scope

| # | Phase | Files | Est. lines | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-14-engine-signing-bridge.md` | ~700 | ✓ |
| 2 | **`auth.js` window hook** — `__axenstax_sign_auth_event(challenge_hex, origin) → Promise<{event_json}>` that uses `nostr-tools-nip46`'s `BunkerSigner` to call `sign_event` against the session's bunker URI for a kind-21236 event with the standard tags | `tools/sites/game/static/auth.js`, smoke test in `tools/smoke/` | ~120 | ✓ (once upstream ships) |
| 3 | **`wasm_auth.rs` outbound caller** — `pub async fn sign_auth_event(challenge: &[u8; 32], origin: &str) -> Result<SignetAuthEvent, BridgeError>` that calls the window hook + parses the returned JSON. Adds `BridgeError` enum (Timeout, BunkerDenied, InvalidResponse, NotPaired). | `game/engine/src/wasm_auth.rs`, new `game/engine/src/signet/bridge.rs` for the caller, tests | ~150 | ✓ |
| 4 | **`RemoteClient::connect` integration** — pre-fetch the auth event before constructing JoinRequestPacket when `USE_SIGNET_AUTH=true`. Surface errors to the join-UI ("Bunker timed out", "Bunker denied request", "Not paired") | `game/engine/src/network.rs` / `game_loop.rs:520` (current call site), tests | ~100 | ✓ |
| 5 | **Pre-flight test sheet** — updates Spec 1 Phase 4's test sheet (or this spec's own) to cover bridge wiring; manual verification against a real bunker on host + peer device | `docs/test-sheets/YYYY-MM-DD-engine-signing-bridge.md` | ~80 | partial — needs real bunker |
| 6 | **Spec 1 Phase 4 cutover trigger** — once 2-5 are green, Phase 4 flips `USE_SIGNET_AUTH=true`, drops the `player_name` BRIDGE comment, ships the multi-client regression session | `game/engine/src/lib.rs` (flag flip), `protocol.rs` (drop BRIDGE comment) — **executed under Spec 1 Phase 4, not this spec** | n/a | ✗ Spec 1's responsibility |
| 7 | **Native path (DEFERRED to Forgesworn Rust crate)** — when `signet-nip46-client` lands as Charter Phase 2's cross-platform deliverable, native client gains a parallel `sign_auth_event` implementation. WASM continues using the JS bridge from phases 2-3 (no rewrite — both paths coexist). | (future) `game/engine/src/signet/bridge_native.rs` | ~150 (future) | ✗ post-alpha |

**Total active scope**: ~450 lines code + tests + spec edits across phases 2-5. Phase 6 is Spec 1 Phase 4's responsibility. Phase 7 is post-alpha.

**Recommended order**: 2 → 3 → 4 → 5. Phases 2 and 3 can land in parallel (different files, no shared API surface until phase 4 joins them).

---

## Phase 1 — This spec

You're reading it. ✓ Move on.

---

## Phase 2 — `auth.js` window hook

### Goal

Publish a window-scoped function `__axenstax_sign_auth_event(challenge_hex, origin)` that:
1. Validates inputs (32-byte hex challenge, plausible origin string).
2. Confirms there's an active Signet session with a paired bunker.
3. Uses `nostr-tools-nip46`'s `BunkerSigner` (already vendored, see Spec 7) to send a `sign_event` request to the bunker for an unsigned kind-21236 template event.
4. Returns the signed event JSON to the caller, or rejects with a typed error.

### Event template

The unsigned event template the hook constructs:

```javascript
const unsigned = {
  kind: 21236,
  pubkey: window.__axenstax_session.pubkey,  // hex pubkey from current session
  created_at: Math.floor(Date.now() / 1000),
  tags: [
    ["challenge", challenge_hex],
    ["origin", origin],
    ["consumer", "axenstax-engine"],
    ["version", "1"],
  ],
  content: "",  // kind-21236 carries data in tags, not content
};
```

Bunker signs and returns `{id, sig, ...unsigned}`. Hook returns the full JSON object as a string (engine parses).

### Changes

- **`tools/sites/game/static/auth.js`**:
  - Import (or reference, given IIFE) `window.NostrTools.nip46.BunkerSigner`.
  - Add an internal `getBunkerSignerForSession()` helper that either retrieves an existing cached signer (per-session) or constructs a new one from the session's stored bunker URI + the app's transient secret. The session state already carries the bunker URI (per Charter rev. 5 groundwork); confirm it's still there on the current main.
  - Publish the window hook:

    ```javascript
    window.__axenstax_sign_auth_event = async (challenge_hex, origin) => {
      // input validation
      if (typeof challenge_hex !== "string" || !/^[0-9a-f]{64}$/i.test(challenge_hex)) {
        throw new TypeError("challenge_hex must be 32-byte hex");
      }
      if (typeof origin !== "string" || origin.length === 0 || origin.length > 256) {
        throw new TypeError("origin must be a non-empty short string");
      }

      const signer = await getBunkerSignerForSession();
      if (!signer) {
        throw new Error("not_paired");
      }

      const unsigned = buildAuthEventTemplate(challenge_hex, origin);
      try {
        const signed = await signer.signEvent(unsigned);
        return JSON.stringify(signed);
      } catch (e) {
        // map bunker errors to typed strings
        if (e?.message?.includes("denied")) throw new Error("bunker_denied");
        if (e?.message?.includes("timeout")) throw new Error("bunker_timeout");
        throw new Error("bunker_error: " + (e?.message ?? "unknown"));
      }
    };
    ```

  - Publish a companion `window.__axenstax_signing_bridge_ready()` that returns `true` once `auth.js` has bunker-pair-state. The engine WASM checks this before attempting to sign.

- **Smoke test**:
  - In `tools/smoke/`, add a Playwright test that loads the lobby with a mocked-bunker JS shim, calls `window.__axenstax_sign_auth_event` directly via `page.evaluate`, verifies the returned JSON has `kind: 21236`, has the right tags, and has a valid sig (or a sentinel sig if using the mock).

### Tests

- **Input validation** — invalid hex / wrong length / non-string → rejects with `TypeError`.
- **Not paired** — no session bunker → rejects with `Error("not_paired")`.
- **Bunker denied** — mocked bunker returns deny → rejects with `Error("bunker_denied")`.
- **Bunker timeout** — mocked bunker delays past timeout → rejects with `Error("bunker_timeout")`.
- **Happy path** — returns a valid signed event JSON with expected tags.

### Acceptance

- All five test cases pass via Playwright `tools/smoke/`.
- `./check.sh --smoke` ALL GREEN.
- Manual: with a real (D-003-unblocked) bunker, calling the hook from devtools console returns a kind-21236 signed event whose Schnorr sig verifies against the session pubkey.

### Save compat

N/A — this is browser-side JS, no save format.

---

## Phase 3 — `wasm_auth.rs` outbound caller

### Goal

Engine WASM code can call `sign_auth_event(challenge, origin)` and get back a typed `SignetAuthEvent` ready to embed in `JoinRequestPacket.auth_event`. The function bridges WASM↔JS via the window hook from Phase 2.

### Changes

- **New module** `game/engine/src/signet/bridge.rs`:

  ```rust
  use crate::signet::event::SignetAuthEvent;
  use thiserror::Error;
  use wasm_bindgen::prelude::*;
  use wasm_bindgen_futures::JsFuture;

  #[derive(Debug, Error)]
  pub enum BridgeError {
      #[error("signing bridge not available — auth.js hook not published")]
      HookMissing,
      #[error("not paired — no bunker session available")]
      NotPaired,
      #[error("bunker timed out")]
      BunkerTimeout,
      #[error("bunker denied request")]
      BunkerDenied,
      #[error("invalid response from bunker: {0}")]
      InvalidResponse(String),
      #[error("bunker error: {0}")]
      BunkerError(String),
  }

  /// Sign a kind-21236 auth event for the engine handshake.
  ///
  /// Calls into `auth.js`'s `__axenstax_sign_auth_event` window hook.
  /// Available only in WASM builds; native builds return BridgeError::HookMissing.
  #[cfg(target_arch = "wasm32")]
  pub async fn sign_auth_event(
      challenge: &[u8; 32],
      origin: &str,
  ) -> Result<SignetAuthEvent, BridgeError> {
      let window = web_sys::window().ok_or(BridgeError::HookMissing)?;
      let hook_name = "__axenstax_sign_auth_event";

      let fn_val = js_sys::Reflect::get(&window, &JsValue::from_str(hook_name))
          .map_err(|_| BridgeError::HookMissing)?;
      let f = fn_val
          .dyn_into::<js_sys::Function>()
          .map_err(|_| BridgeError::HookMissing)?;

      let challenge_hex = hex::encode(challenge);
      let promise = f
          .call2(
              &JsValue::NULL,
              &JsValue::from_str(&challenge_hex),
              &JsValue::from_str(origin),
          )
          .map_err(|e| map_js_error(e))?;

      let signed_json_value = JsFuture::from(js_sys::Promise::from(promise))
          .await
          .map_err(|e| map_js_error(e))?;
      let signed_json: String = signed_json_value
          .as_string()
          .ok_or_else(|| BridgeError::InvalidResponse("not a string".into()))?;

      // Parse the JSON into the existing SignetAuthEvent shape.
      let event: SignetAuthEvent = serde_json::from_str(&signed_json)
          .map_err(|e| BridgeError::InvalidResponse(format!("parse: {e}")))?;
      Ok(event)
  }

  #[cfg(not(target_arch = "wasm32"))]
  pub async fn sign_auth_event(
      _challenge: &[u8; 32],
      _origin: &str,
  ) -> Result<SignetAuthEvent, BridgeError> {
      Err(BridgeError::HookMissing)
  }

  fn map_js_error(e: JsValue) -> BridgeError {
      let msg = e.as_string().unwrap_or_else(|| format!("{:?}", e));
      if msg.contains("not_paired") { BridgeError::NotPaired }
      else if msg.contains("bunker_denied") { BridgeError::BunkerDenied }
      else if msg.contains("bunker_timeout") { BridgeError::BunkerTimeout }
      else { BridgeError::BunkerError(msg) }
  }
  ```

- **`game/engine/src/signet/mod.rs`** — add `pub mod bridge;`.
- **`game/engine/src/wasm_auth.rs`** — no direct change, but the public `sign_auth_event` is now reachable for `network.rs` (Phase 4).

### Tests

WASM-side async testing is tricky in `cargo test`. Practical test surface:
- **Native build** — `sign_auth_event` returns `BridgeError::HookMissing`. Tests this trivially.
- **Type-level** — `SignetAuthEvent` round-trips through the JSON shape Phase 2 emits (use a captured-output sample as a test fixture).
- **`BridgeError` variants** — each variant maps from the right JS error string.

Full WASM-side integration is covered by Phase 5's manual playtest (real bunker required).

### Acceptance

- `cargo test --bin axenstax-engine` green (new tests pass; existing unaffected).
- `cargo build --target wasm32-unknown-unknown` green.
- `./check.sh` ALL GREEN.

### Save compat

N/A — no on-disk state added.

---

## Phase 4 — `RemoteClient::connect` integration

### Goal

When `USE_SIGNET_AUTH=true`, the WASM `RemoteClient::connect` path awaits the signing bridge to produce an `auth_event` and includes it in `JoinRequestPacket`. Errors surface to the UI with kid-friendly messaging.

### Changes

- **`game/engine/src/game_loop.rs`** (around line 520, the `RemoteClient::connect("addr", "Player")` call site):
  - Refactor the connect call to pre-build the JoinRequestPacket including (when feature-flagged) the auth event.
  - Pseudocode shape:

    ```rust
    let auth_event_opt = if USE_SIGNET_AUTH {
        let challenge = generate_or_fetch_challenge();  // see protocol.rs ChallengePacket flow
        let origin = "axenstax-engine";  // matches the engine's challenge issuance
        match signet::bridge::sign_auth_event(&challenge, origin).await {
            Ok(ev) => Some(ev),
            Err(BridgeError::NotPaired) => {
                show_join_error("You need to sign in first. Open the lobby and pair your Signet identity.");
                return;
            }
            Err(BridgeError::BunkerDenied) => {
                show_join_error("Sign-in cancelled.");
                return;
            }
            Err(BridgeError::BunkerTimeout) => {
                show_join_error("Your phone didn't respond in time. Try again.");
                return;
            }
            Err(e) => {
                show_join_error(&format!("Sign-in error: {e}"));
                return;
            }
        }
    } else {
        None
    };

    let packet = JoinRequestPacket {
        protocol_version: PROTOCOL_VERSION,
        player_name: "Player".to_string(),  // BRIDGE comment retained per Spec 1 Phase 4
        auth_event: auth_event_opt,
        handle_credential: None,  // future: pull from session if present
    };
    RemoteClient::connect_with_packet(addr, packet).await;
    ```

  - Adapt `RemoteClient::connect` to accept the pre-built packet, or add a `connect_with_packet` variant. Existing single-player path doesn't go through this code path, so single-player remains unaffected.

- **`game/engine/src/chat_ui.rs`** (or wherever the join-error UI lives) — add `show_join_error(msg: &str)` if it doesn't exist; otherwise just use the existing chat-warning channel.

### Tests

- **Unit**: With `USE_SIGNET_AUTH=false`, the connect path produces a packet with `auth_event: None` (regression of current behaviour).
- **Unit**: With `USE_SIGNET_AUTH=true` and a stubbed signing bridge that returns `Ok(test_event)`, the packet has `auth_event: Some(test_event)`.
- **Unit**: With `USE_SIGNET_AUTH=true` and stubbed `Err(NotPaired)`, the connect aborts and `show_join_error` is invoked with the expected message.
- **Type-only**: All four `BridgeError` variants map to a user-facing string.

### Acceptance

- All new tests pass. `./check.sh` ALL GREEN.
- `USE_SIGNET_AUTH=false` (current setting) behaves identically to today — no regression for single-player or for the existing experimental multiplayer.

### Save compat

No format change. The flag stays `false` until Phase 6.

---

## Phase 5 — Pre-flight test sheet (manual, real bunker)

### Goal

Verify the WASM-end-to-end path against a real Signet bunker on a single dev box: host running engine, peer device running the lobby + bunker app.

### Test sheet contents

Create `docs/test-sheets/<DATE>-engine-signing-bridge.md`:

1. **Pre-conditions**:
   - Upstream Signet bunker has shipped `sign_event` per the issue body's acceptance criteria (D-003 unblocked).
   - Local stack running (game site + voice server + engine binary).
   - Tester has a paired Signet identity in the lobby.
2. **Pair check** — open lobby, sign in via QR, confirm bunker is paired (`window.__axenstax_signing_bridge_ready() === true`).
3. **Hook smoke** — devtools console: `await __axenstax_sign_auth_event("0".repeat(64), "axenstax-engine")` returns a kind-21236 signed event whose `sig` verifies.
4. **End-to-end join** — start engine; attempt to join own host LAN (flag temporarily set `true` for test): bunker prompts, approve, join succeeds, no `"missing auth_event"` error in server logs.
5. **Denial path** — same flow, deny bunker prompt: engine surfaces "Sign-in cancelled." error in chat-ui.
6. **Timeout path** — close bunker app mid-flow: engine surfaces "Your phone didn't respond in time." after configured timeout.
7. **Multi-client regression** (Spec 1 Phase 4's responsibility) — two clients, both joining successfully, both signing distinct challenges.
8. **No-pairing** — tester signs out, attempts join: surfaces "You need to sign in first."

### Acceptance

- Test sheet items 1-8 all PASS on a single dev box.
- Manual screenshots / logs captured per the daily-build-test workflow (`docs/workflow/daily-build-test.md`).

---

## Phase 6 — Spec 1 Phase 4 cutover trigger

### Goal

This phase is executed *under Spec 1 Phase 4's branch*, not this spec's branch. When phases 2-5 are green:

- Spec 1 Phase 4 flips `USE_SIGNET_AUTH=true` in `game/engine/src/lib.rs` (or wherever the constant lives).
- Drops the `player_name` BRIDGE comment in `protocol.rs` and removes the field's documentation as client-asserted.
- Runs the multi-client regression session per Spec 1's existing Phase 4 plan.

### Acceptance

Spec 1 Phase 4 status moves from BLOCKED → DELIVERED. CLAUDE.md tech-debt bullet 5 struck through.

---

## Phase 7 — Native path (DEFERRED, post-alpha)

### Goal

When Forgesworn ships the cross-platform `signet-nip46-client` Rust crate (Charter Phase 2 commitment, post-alpha 6-8 week window), native clients gain a parallel signing path.

### Sketch

- Add `signet-nip46-client = { workspace = true }` (or path = "...") to `game/engine/Cargo.toml`.
- New module `game/engine/src/signet/bridge_native.rs`:

  ```rust
  #[cfg(not(target_arch = "wasm32"))]
  pub async fn sign_auth_event(
      challenge: &[u8; 32],
      origin: &str,
  ) -> Result<SignetAuthEvent, BridgeError> {
      let client = signet_nip46_client::Client::from_paired_config_dir()?;
      let unsigned = build_auth_event_template(challenge, origin);
      let signed_json = client.sign_event(unsigned).await?;
      Ok(serde_json::from_str(&signed_json)?)
  }
  ```

- WASM bridge from phases 2-3 unchanged — both transports coexist. The single public `signet::bridge::sign_auth_event` function dispatches by `cfg`.
- Native pairing UX (no QR camera on a Rust desktop binary) is a separate concern — the crate's `from_paired_config_dir()` assumes the user paired once via the lobby's web flow, or pasted a `bunker://` URI into a config file. UX polish lives outside this spec.

### Acceptance

Native engine successfully joins a multiplayer game with `USE_SIGNET_AUTH=true` once the user has paired their bunker via whichever onboarding path Forgesworn ships.

---

## Open design questions

For Axolittle / Staxolottle / Forgesworn coordination:

### Affecting Phase 2 (`auth.js`)
1. **Bunker timeout** — what's the right timeout? 30 seconds matches Charter rev. 5 groundwork; 60 seconds gives more grace for slow networks. **Recommendation**: 30 s for engine handshake; engine UI shows progress spinner during wait.
2. **Bunker URI persistence** — `auth.js` reads from the Signet session today. If session expires mid-game, the bridge will fail. Should the engine cache a longer-lived bunker URI in IDB (reusing the Charter rev. 5 IDB schema) to survive session expiry? **Recommendation**: defer; let users re-sign-in on session expiry, same as today's lobby experience.

### Affecting Phase 3 (`wasm_auth.rs`)
3. **Origin string** — the upstream issue body says origin tag is "the consumer's identifier." Should AxeNStax use `"axenstax-engine"`, `"axenstax-engine-v1"`, or a full URL `"https://axenstax.app/engine"`? **Recommendation**: `"axenstax-engine"` — short, stable, identifies the consumer without versioning churn.
4. **Challenge issuance flow** — Spec 1 Phase 2/3 already added `ChallengePacket` for the server-issued challenge nonce. The client receives the challenge, then signs it via this bridge. Confirm `network.rs` already exposes the received challenge to the JoinRequest builder before pulling this into Phase 4 work.

### Affecting Phase 6 (Spec 1 Phase 4 cutover)
5. **Existing `player_name` field on JoinRequestPacket** — keep as legacy field set to empty string for back-compat? Drop entirely with a PROTOCOL_VERSION bump? **Recommendation**: keep `player_name: String` as legacy display-only field; server canonical name comes from the verified `handle_credential` per Spec 1's design. PROTOCOL_VERSION doesn't bump since the wire shape is unchanged.

### Affecting Phase 7 (native)
6. **Rust crate name** — Forgesworn's working name is `signet-nip46-client`. Final crate name may differ. **Action**: track via Sentinel; update Phase 7 when the crate ships.

### Cross-cutting
7. **Anti-abuse** — if a malicious site embeds the engine and asks for sign requests, the bunker's per-consumer kind-allow-list (kind 21236 only for AxeNStax) prevents impersonation of other consumers. But a malicious AxeNStax origin (someone running a fork) could still trigger sign-event prompts. **Recommendation**: defer — UI on the bunker side (per-request approval card) is the primary defence.

---

## Acceptance — overall

- Phases 2-5 complete or explicitly blocked (Phase 5 = real-bunker manual test).
- `./check.sh` ALL GREEN throughout.
- Spec 1 Phase 4 status updated from BLOCKED → READY TO EXECUTE (or DELIVERED if cutover happens same session).
- CLAUDE.md tech-debt bullet 5 updated to reflect either delivery or active-execution-state.
- Memory charter phase4 shared gap updated: Spec 1 Phase 4 no longer shares the gap (Charter Phase 1 already routed around it).
- Sentinel AS-005 status updated (deferred → in-progress → resolved as appropriate).

---

## Spec maintenance

Per CLAUDE.md "Spec Maintenance" rule:
- Drift between this spec and the upstream Signet `sign_event` wire shape is the highest-risk drift. Cross-check the upstream issue body's "Acceptance criteria" block against Phase 2's event template before starting.
- If the upstream bunker shapes the response differently from what Phase 2 expects, Phase 3's `serde_json::from_str` will fail loudly. The fixture-based test in Phase 3 should be regenerated from a real bunker output before merging.
- When Phase 7 (native) lands, this spec gets a status note + Phase 7 fleshed out from sketch to detail.

---

## Memory-rule check

- **signet boundary**: ✓ — all Signet changes are upstream-owned (the `sign_event` capability is generic NIP-46). This spec only describes AxeNStax's consumption of that generic capability.
- **shared infra strategy**: ✓ — window hook pattern lifts to other AxeNStax-internal games. Recommended namespace-renaming pattern: `__signet_sign_event(kind, content, tags)` as a generic hook in `auth.js`, with AxeNStax-engine and future games each calling it with their kind.
- **pretest check**: implementer must re-verify `wasm_auth.rs`, `auth.js`, and `game_loop.rs:520` surface state before starting each phase — these files have evolved.
- **charter phase4 shared gap**: this spec resolves the AxeNStax half. Charter rev. 7 already routed around the gap for Phase 1; future Charter mechanisms B/C/D will share the same bridge.
- **alpha launch posture**: bridge is LAN-multiplayer-only at first ship; axenstax.app is not live; no deploy infrastructure dependencies.
- **bitcoin parent controlled**: N/A — auth bridge doesn't touch Bitcoin.
- **uk english naming**: ✓ — no naming choices that span the UK/US distinction in this spec.
- **trotters relay**: `wss://relay.trotters.cc` is the default relay for the bunker's NIP-46 traffic; AxeNStax doesn't override.

---

## Cross-references

- **Spec 1 Phase 4 prerequisite section** — `docs/foundations/2026-04-20-engine-signet-auth.md` §"Phase 4 prerequisite — engine-side signing bridge"
- **Upstream proposal** — `docs/integrations/signet/2026-05-05-nip46-signing-bunker-upstream.md` (DRAFT) + `-issue-body.md` (paste-ready)
- **Sentinel AS-005** — `docs/sentinel/state.yaml`, status currently `deferred` blocked_by D-003
- **Sentinel D-003** — decided 2026-05-07 "wait for Phase 5 (Bitcoin) lead-in"; revisit cadence per Sentinel
- **Spec 7 (vendored NIP-46 IIFE)** — `docs/foundations/2026-05-08-site-build-pipeline-for-npm.md`, DELIVERED 2026-05-08
- **Charter rev. 7 spec** — `docs/foundations/2026-05-09-charter-rev7-integration.md` (decoupled Charter Phase 1 from this gap)

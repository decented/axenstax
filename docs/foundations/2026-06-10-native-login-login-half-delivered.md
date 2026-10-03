# Native login — Bucket 3 "login half" DELIVERED

**Status:** BUILT + merged to `main` (2026-06-10). The engine integration the
native-build-solo goal left as an owner-coordinated HARD STOP, executed under
`docs/goals/2026-06-10-native-login-integration.md`, bound by the decision
contract `docs/foundations/2026-06-10-offline-first-login-and-stash-sequencing.md`.
**Login only — Stash stays deferred + decoupled.**

This records what survives a rebuild; the contract doc holds the *why*.

---

## What shipped (Rust, native-target only)

- **Vendored signer.** `signet-nip46-client` (generic NIP-46 bunker client) is
  vendored in-repo at `game/engine/vendor/signet-nip46-client/` and added under
  `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` (plus a direct
  `nostr = "=0.44.3"`). **Why vendored, not a path/git dep:** the WASM bundle is
  rebuilt by `trunk build` on every push to `main` (auto-deploy), and cargo
  resolves the *whole* dep graph — including native-only deps — before building
  any target. A path to `<workspace>/forgesworn/` or the private git remote would
  fail that resolve in CI. Vendoring keeps it CI-safe; the canonical crate stays
  in Forgesworn (`VENDOR.md` records the pinned commit + re-sync steps). The
  native deps never enter the wasm32 graph (proven: `trunk build` green).

- **Native signer + offline-first identity** — `game/engine/src/signet/native_signer.rs`:
  - `BunkerSession` wrapper: `pair` (offline construct), `restore_signer_from`
    (offline reconnect), `complete_sign_in` (persist session + cache identity +
    publish owner), `sign_out`.
  - **Offline identity:** `NativeIdentity::{Guest, SignedIn}` loaded from a cached
    pubkey with **zero network**; `Guest` is the default and unlocks the whole
    game. Real `npub` on native via `nostr`'s bech32 (the `npub` module only had
    the JS path). The persisted session file is locked to `0600` (it holds the
    sensitive NIP-46 *client* key — never the persona key, never logged).
  - **Startup:** `init_identity()` runs in native `main()` — a non-blocking file
    read that publishes the owner pubkey for Seam B and logs who you are. **No
    login gate anywhere; sign-in is never on the single-player critical path.**

- **Client join auth (Spec 1 client side)** — `build_join_auth_event(signer,
  challenge, origin)` builds + signs a kind-21236 event and converts it to
  `SignetAuthEventWire`. Generic over `NostrSigner` (so it tests against a local
  `Keys`). `RemoteClient::connect` now takes `auth: Option<JoinAuth>` (pure
  `build_join_request` helper). Typed failure states for the join UI:
  `JoinAuthError::{NotPaired, Timeout, Denied, Other}`. **The flag stays
  `USE_SIGNET_AUTH = false`** — server still ignores it.

- **Seam B (identity tag + monotonic version)** — `WorldMeta.owner_pubkey:
  Option<String>` (`#[serde(default)]`, tolerant of pre-seam saves) + the existing
  monotonic `version`. `WorldMeta::claim_owner` is the guest→identity claim path:
  a guest save stays `None`; the first signed-in save claims the orphan; a
  *different* owner is never clobbered (logged as a divergence for future Stash
  conflict-resolution). Stamped in both native + WASM `save_world` via the
  cross-platform `current_owner_pubkey()` (native = cached Signet identity, WASM =
  `auth.js` pubkey).

- **Seam A (encrypt-at-rest)** — **NOT finalised** (pinned by the owner's web-Stash
  blob format). `write_world_folder` keeps the clean serialize→write boundary
  **wrap-ready** with a flagged note: an AEAD layer wraps `encoded` between
  `bincode::serialize` and `write_atomic` with no caller-visible format change.
  No AEAD implemented.

## Proven solo (unit tests, `./check.sh` green)

- `builds_a_server_verifiable_auth_event` — the headline: a client-built event
  (local key, no network) passes the engine's own `verify_auth_event` (`Ok`).
  `auth_event_is_rejected_for_the_wrong_challenge` proves the binding binds.
- Offline identity: `no_cache_is_guest`, `cached_pubkey_is_signed_in_offline`,
  `corrupt_cache_degrades_to_guest`, `npub_renders_bech32_on_native`.
- Session store: round-trip, `session_file_is_locked_down` (0600),
  `restore_signer_offline_{with_no_session_is_none,reconstructs_from_persisted}`.
- Seam B: `seam_b_claim_owner_semantics`, JSON round-trip + pre-seam tolerant
  decode (`owner_pubkey == None`).
- Join wiring: `guest_join_carries_no_auth`, `authenticated_join_attaches_the_signed_event`.

## Owner / real-device boundary (NOT attempted — left with harnesses)

1. **Live bunker pairing** — needs the owner's phone. Harness:
   `tools/native-build-spike/src/bin/bunker_pair.rs`. Everything signer-side is
   unit-tested against a local-key signer up to this line.
2. **`USE_SIGNET_AUTH = true` flip + a two-machine LAN join** — Spec 1 Phase 4.
   The authenticated-join orchestration (wait for the server `ChallengePacket` →
   `restore_signer` → `build_join_auth_event` → pass `Some(JoinAuth)`) is wired as
   a BRIDGE at the `game_loop` connect site; flipping the flag blind would break
   LAN multiplayer at runtime.
3. **Offline LAN multiplayer** — the one open owner decision (contract §6); needs
   a short-lived local session key. Not built.
4. ~~**Native sign-in menu UX**~~ — **BUILT 2026-06-10 (follow-up commit).** See
   "Sign-in UI + fullscreen" below. Only the *live phone handshake* remains the
   device boundary.
5. **Stash** (`stash-rs` → `save.rs`) — deferred + decoupled by the contract.

## Sign-in UI + fullscreen (follow-up, 2026-06-10)

Owner-requested after the signer landed: a real native sign-in screen + launch in
fullscreen.

- **Fullscreen.** Native launches **borderless-fullscreen** on the current
  monitor (`main.rs` window attrs; 1280×720 kept as the windowed fallback). **F11**
  toggles fullscreen. WASM unchanged (browser owns fullscreen).
- **Sign-in dialog** (`menu.rs` `MenuDialog::SignIn` + `draw_signin_dialog` +
  `draw_qr`, native-only). Lobby header gains **Sign in** (guest) / **Sign out**
  (signed-in) next to Quit, plus an identity line (`👤 Playing as guest …` or the
  npub). The dialog (the owner chose **both** input methods):
  - **QR (primary):** desktop shows a `nostrconnect://` QR (the new
    `BunkerSession::pair_nostrconnect` client-initiated flow — see VENDOR.md, must
    be upstreamed) for the phone to scan. Drawn as egui cells, no texture.
  - **Paste fallback:** a collapsible `bunker://` text field.
  - Live status: await-scan → await-approval → success(npub) / typed failure.
- **Async driver** (`native_signin.rs`): the handshake runs on a worker thread
  with its own current-thread tokio runtime; the menu `poll()`s a status each
  frame. `complete_sign_in` persists the session + caches identity + publishes the
  Seam-B owner on success. Unit test: `status_lifecycle_without_network`.
- **Verified solo:** `./check.sh` green; headless `--shot-lobby` (Sign in button +
  guest line) and a new `--shot-signin` (the QR dialog) render correctly — the QR
  is crisp + scannable. **Still the device boundary:** the live phone handshake
  (scan → approve) needs a real Signet bunker.

### Bug found 2026-10-01: QR never paired with mySignet (our bug, not Signet's)

Previously filed upstream as "mySignet doesn't accept `nostrconnect://`" (Signet
ticket 187). The real cause was on **our** side, in two places:

1. **No `secret` in the QR.** rust-nostr 0.44's `NostrConnectURI::client` has no
   secret field, so the URI was `nostrconnect://<pk>?metadata=…&relay=…`. Current
   NIP-46 requires `secret`; mySignet's parser returns null without it, so the scan
   silently did nothing.
2. **Ack-only acceptance.** Even with a secret, rust-nostr's client accepts only a
   literal `"ack"` as the signer's `connect` reply. Current NIP-46 has the signer
   echo the `secret` back (mySignet does), so the reply would have been ignored.

**Correct approach (now in the vendored crate):** `pair_nostrconnect` mints a fresh
16-byte CSPRNG hex `secret` and builds the URI itself (`relay`, `secret`, `name`,
`metadata`, all form-urlencoded). The first `user_public_key`/sign call runs our own
handshake: subscribe for kind 24133 `#p`=app pubkey, NIP-44 decrypt, accept the first
reply whose `result` is the exact secret (constant-time compare). **Secret echo is
required** — a bare `"ack"` is rejected, because the relay sees our app pubkey in the
subscription and could otherwise race to become our "signer". A reply with a
non-empty `error` fails the handshake immediately; everything else is ignored. The author of that event is the remote signer, which is handed to
rust-nostr as a secret-less `bunker://` session for signing/persist/restore. That
bootstrap sends one secret-less `connect`; mySignet ACKs it on the owner route while
its 2-minute NostrConnect serve window is open. Restoring later also sends `connect`
and needs mySignet to be serving (background serving on, or a fresh serve window).
See `vendor/signet-nip46-client/VENDOR.md` (must be upstreamed).

## Files

Login half: `game/engine/Cargo.toml` (+`Cargo.lock`),
`game/engine/vendor/signet-nip46-client/**`, `src/signet/{mod.rs,native_signer.rs}`,
`src/remote_client.rs`, `src/save.rs`, `src/main.rs`, `src/game_loop.rs`.
Sign-in UI + fullscreen: `src/native_signin.rs`, `src/menu.rs`, `src/main.rs`,
`vendor/signet-nip46-client/src/lib.rs` (+`VENDOR.md`), `Cargo.toml` (qrcode).

## Memory-rule check

- **[[feedback_signet_boundary]]** — honoured: a generic NIP-46 signer, vendored
  unchanged; nothing AxeNStax-specific asked of Signet.
- **[[feedback_npub_only_display]]** — native now renders real npub; hex stays
  internal (cache file, owner stamp).
- **[[project_settlement_model_decision_parked]]** — consistent: the engine holds
  saves + (later) a score, never custody; identity = the player's own bunker.
- **[[feedback_autonomy_to_playtest_boundary]]** — stopped cleanly at the
  live-bunker / device boundary, harnesses pointed to.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

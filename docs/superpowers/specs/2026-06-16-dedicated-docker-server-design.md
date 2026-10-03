# Dedicated Docker Server — Design Spec

**Date:** 2026-06-16
**Status:** Approved (owner deferred remaining calls to recommendations, 2026-06-16)
**Author:** Claude (Staxolottle directing)

## Goal

Today multiplayer is **client-hosted and native-only**: you run the full game in
"Host Game" mode and other **native** clients join over QUIC. Browsers physically
cannot speak QUIC (no raw UDP in a browser), so the **web app cannot join any
multiplayer game at all** (`max_remote_players` is clamped to 0 on `wasm32`).

This spec defines a **self-hostable dedicated server** shipped as a Docker image:

> *"Anyone can install the Docker, get a local URL, and both the web app and the
> native app can join that server."*

It must run headless (no GPU/display), persist its world, and accept connections
from **both** the native client and the browser PWA.

## Decisions (owner-approved)

1. **Transport: WebSocket.** The browser-compatible transport is WebSocket — universal,
   simplest, and one code path serves **both** web and native. It slots into the
   existing `Transport` trait. QUIC stays untouched for the existing native LAN
   peer-hosting path. (WebTransport is a possible future perf upgrade; not now.)
2. **All-in-one image.** The container serves the **web client and the game socket
   from the same origin** so the self-signed TLS cert is trusted once and reused —
   no mixed-content wall, no second cert prompt. "Install Docker, share one URL."

## Why WebSocket is a clean fit (and a free reliability win)

- The `Transport` trait (`transport.rs`) is a tiny **poll-based** interface:
  `send_to_*` / `try_recv_from_*`. `ChannelTransport` and `QuicServerTransport`
  already implement it; a WebSocket transport implements the same four methods.
- `HostedServer` accepts remote players as opaque `Box<dyn ServerTransport>` fed
  through an mpsc channel by an **accept thread** (`spawn_quic_accept_thread`).
  A **WebSocket accept thread** feeds the exact same channel — `HostedServer`
  never learns which transport a client used.
- The current QUIC path uses **unreliable, unordered datagrams** capped at ~1200
  bytes (`conn.send_datagram`). WebSocket is **reliable, ordered, and unbounded
  per message** — strictly better for `ChunkData` delivery (large LZ4 chunks that
  could silently exceed the datagram MTU today).

## Architecture

```
                 ┌──────────────────────────────────────────────────┐
                 │  axenstax-server container (docker compose up)     │
                 │                                                    │
  share one URL: │  Caddy  :8443  HTTPS + WSS  (self-signed)          │
 https://BOX:8443│    /        → static web client (WASM bundle)      │
                 │    /ws      → reverse-proxy → 127.0.0.1:8080        │
                 │                          │                         │
                 │  axenstax-engine --server (headless, no winit/wgpu)│
                 │    plain WebSocket on :8080   ◄── native may also   │
                 │    HostedServer (0 local + N remote)   connect here │
                 │      mobs · blocks · carts · economy · combat       │
                 │      proof-of-play · commands · persistence         │
                 │                          │                         │
                 │  /worlds   (Docker volume — world.dat + chunks)     │
                 └──────────────────────────┼─────────────────────────┘
                                            │
        ┌───────────────────────────────────┼───────────────────────────────┐
   web (Chromium) on machine A          web on machine B            native on machine C
   open https://BOX:8443                open https://BOX:8443       Join → wss://BOX:8443/ws
   → wss://BOX:8443/ws (same origin)    → wss://BOX:8443/ws         (or ws://BOX:8080 plain)
```

- **Engine WS server = plain `ws` on :8080.** TLS is handled entirely by Caddy, so
  the Rust side stays simple. The browser **requires** TLS (a remote LAN IP over
  `http`/`ws` is an *insecure context* → `navigator.gpu` / WebGPU is unavailable),
  which Caddy provides. Native can use `wss://BOX:8443/ws` (TLS, skip-verify for
  self-signed) **or** `ws://BOX:8080` (plain) — both reach the same engine.
- **Two processes, one container,** started by an entrypoint script (`server &` then
  `exec caddy run`). `docker compose` owns the worlds volume + env config.

## Use-case review — every implemented system on a shared server

The simulation is already **cleanly headless** (zero renderer/wgpu/winit coupling;
the `TestHost` harness drives `GameServer` with no GPU) and **server-authoritative**.
Per-system status on a dedicated server:

| System | Server-authoritative today? | On the dedicated server |
|---|---|---|
| World time / day-night | Yes (`server.rs`) | Works unchanged |
| Mob spawning + AI | Yes (pure fns, server ECS) | Works; spawns relative to joined players |
| Falling blocks / water / leaf decay | Yes | Works; broadcast as `BlockChange` |
| Combat + entity health/physics | Yes | Works |
| Carts / rail freight | Yes (`tick_carts`, persisted) | Works; entity-synced |
| Block edits (break/place) | Yes — validated (reach, registry, bedrock, reload) | Works; reliable over WS |
| Crafting / recipes | Pure (`match_recipe`) | Client-side today; deterministic |
| Proof-of-Play hashing | Yes — `server_secret` never leaves server | Works; payouts out of scope (no phoenixd in image) |
| Economy blocks (Vendor/TipJar/Plot/Auction/Market) | Yes — persisted | Persist & broadcast; **ownership caveat below** |
| Scenarios / challenges | Yes (`scenario.rs`) | Works |
| Chat / slash commands | Yes (`commands/`) | Works in-game; **server console = opportunity** |
| Cosmetics / skins | `skin_key` broadcast on `PlayerState` | Key broadcasts; **skin bytes delivery still gated** |
| Persistence | Yes (`worlds/<name>/`, append-only) | Mapped to a Docker volume + autosave |

## Gaps & opportunities (found while speccing)

**Fixed in this work:**

- **G1 — Web cannot join (headline).** Browsers can't do QUIC. → WebSocket transport
  + cross-platform `RemoteClient` + WASM `web_sys::WebSocket` client.
- **G2 — No dedicated server binary.** Hosting requires the full GUI client. →
  `--server` headless mode in the existing binary (no winit/wgpu init).
- **G3 — `JoinAccept` hardcodes `seed: 42`** (`hosted_server.rs:444`) regardless of the
  world's real seed → a joiner would generate **mismatched terrain**. The server is
  built with the real seed but never sends it. → Send the server's actual seed.
- **G4 — Spawn point assumes a host player** (`accept_new_remote_connections` spawns
  "near player 0"). With 0 local players there is no player 0. → Fall back to a
  world spawn (meta-driven, default `(0.5, 80, 0.5)`).
- **G5 — `max_remote_players` is effectively hardcoded** at call sites. → Configurable
  via env/CLI for the dedicated server (default 8). Existing per-client packet/block
  budgets + speed-cap anti-cheat already bound abuse.

**Documented limitations / deferred opportunities:**

- **L1 — Guest identity only.** `USE_SIGNET_AUTH = false`: joiners are anonymous with a
  client-asserted name (bounded/sanitised). On a shared server this means weak
  identity and name collisions. Real identity is the **Phase 4** boundary (blocked
  upstream on the engine NIP-46 signing bridge). For alpha: guest join, server
  de-dupes display names with a numeric suffix. Economy-block ownership uses
  `LocalPlayer(pidx)` (per-session); converge on Npub when Phase 4 lands (matches the
  economy-block-owner-convergence plan).
- **L2 — Server console (opportunity, included minimal).** Pipe stdin lines through the
  existing command dispatcher so an operator can `/time`, `/gamemode`, `/save`, etc.
  Minimal version included; full RCON deferred.
- **L3 — Skin bytes delivery still gated.** `skin_key` broadcasts but the bytes are
  out-of-band and not yet delivered to other clients. Players appear with default
  skins on the server. Unchanged by this work.
- **L4 — Bitcoin payouts out of scope for the image.** Proof-of-Play is structurally
  ready (server holds the secret) but no phoenixd/LNbits ships in this image. A future
  "Bitcoin-enabled server" image layers that on.
- **L5 — LAN discovery not wired.** Manual URL entry only (fine for Docker/remote).

## Components to build

1. **`transport.rs`** — `MaybeSend` marker so the `*Transport` traits require `Send`
   on native (accept thread sends `Box<dyn ServerTransport>` across threads) but **not**
   on `wasm32` (single-threaded; `web_sys::WebSocket`/`Rc` aren't `Send`).
2. **`ws_transport.rs` (native)** — `WebSocketServerTransport` (per-connection
   tokio-tungstenite bridge → mpsc, mirrors `network::bridge_server_connection`),
   `spawn_ws_accept_thread` (mirrors `spawn_quic_accept_thread`), and native
   `WebSocketClientTransport` (`ws://` plain + `wss://` skip-verify via the existing
   `SkipServerVerification`).
3. **`ws_transport_web.rs` (wasm32)** — `WebSocketClientTransport` over
   `web_sys::WebSocket` (`arraybuffer`; `onmessage` → inbound `VecDeque`; outbound
   buffered until `onopen`).
4. **`HostedServer`** — choose accept transport via a new `RemoteTransport { Quic, WebSocket }`
   param to `start` (existing callers pass `Quic`); G3/G4 fixes; configurable
   `max_remote_players`.
5. **`remote_client.rs`** — make cross-platform; `from_transport` constructor +
   `connect_websocket(url)` (native + wasm).
6. **`server_main.rs` (native)** — `--server` headless entry: parse env/CLI, load/create
   world, start `HostedServer(0 local, WebSocket, N remote)`, 20 TPS loop, periodic
   autosave, SIGTERM graceful save, minimal stdin console.
7. **Menu / game_loop** — WASM `JoinGame` path; dedicated-server page auto-detect
   (injected `window.AXENSTAX_DEDICATED_WS`) → default `wss://<origin>/ws` + a Join
   button; native Join dialog accepts `ws://`/`wss://` URLs (URL → WebSocket, bare
   `ip:port` → QUIC as today).
8. **Docker** — multi-stage build (trunk WASM bundle + `cargo --release` server binary),
   runtime image with Caddy + the binary + static web assets, `Caddyfile`,
   `docker-compose.yml` (worlds volume + env), entrypoint.

## Config surface (env, with CLI override)

`AXENSTAX_WORLD` (name, default `server-world`), `AXENSTAX_SEED`, `AXENSTAX_GAMEMODE`
(survival/creative/adventure), `AXENSTAX_MAX_PLAYERS` (default 8),
`AXENSTAX_WS_PORT` (default 8080), `AXENSTAX_SERVER_NAME` (MOTD/display),
`AXENSTAX_AUTOSAVE_SECS` (default 60), `AXENSTAX_WORLDS_DIR` (default `/worlds`).

## Wire compatibility

Same `protocol` (v46), same packets, same bincode framing. WebSocket carries the
identical `[1-byte type tag | bincode body]` frames the datagram path used, so
native↔native over WebSocket is wire-identical to QUIC. **Bump `PROTOCOL_VERSION`**
only because the `JoinAccept` seed fix (G3) changes observable join behaviour; native
clients must update in lockstep, which the version check already enforces.

## Verification plan (solo, up to the playtest boundary)

- `check.sh` green (clippy, build, `cargo test`, trunk WASM build, bundle size).
- New unit/integration tests: WS frame round-trip; `HostedServer` accepts a WS client
  and returns `JoinAccept` with the **real seed**; 0-local-player server ticks and
  spawns a joiner at the fallback spawn.
- Build the image; `docker compose up`; confirm the web client serves over HTTPS and a
  scripted WebSocket client completes the join handshake (native + a wasm-equivalent
  smoke).
- **Playtest boundary (owner):** actual cross-machine join from a second machine on
  both the native app and the browser — can't be self-verified here.

## Out of scope

Spec 07 Agones fleet / multi-world cluster; Bitcoin payout backend; Phase 4 Signet
auth; WebTransport; skin-bytes delivery; networked split-screen.

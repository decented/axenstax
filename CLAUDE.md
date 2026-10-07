# Axe'n'Stax

## Build Prerequisites (Host)

```bash
sudo apt install libudev-dev   # Required by gilrs (gamepad support)
# rfd xdg-portal backend: requires xdg-desktop-portal at runtime (standard on
# any desktop Linux — nothing extra to install; GTK fallback NOT used).
```

Rust toolchain (stable) and `cargo` must be installed on the host. Builds run on the host, not the VM.

## Sibling repos

- `../AxeNStax-internal/` (private) — **legacy** feedback data (`feedback/log.jsonl`,
  `items/`, `board.json`, audio). Historically written by the now-removed voice
  server; may carry PII (Whisper transcripts + game context), so the repo is
  private and never mirrored to the public `AxeNStax` tree. The current feedback
  path is the serverless lobby mailbox (`/bug`, `/idea`) → `tools/feedback-reader/`,
  not this repo.

## Spin Up

"Spin up" means build AND start all services. Two things to run:

### 1. Build the engine (HOST only)

```bash
cd <repo>/game/engine && CARGO_TARGET_DIR="$HOME/<workspace>/AxeNStax/build" cargo build --release
```

This puts the binary at `<repo>/build/release/axenstax-engine`,
which is what the docs site `/download` route serves
(`PROJECT_ROOT/build/release/axenstax-engine`, see `tools/sites/docs/app.py:28`).
Use `cargo build` (not `run`) — `run` launches the engine, which is
interactive; we just want the binary written.

### 2. Start the sites (host)

The website was split into independent FastAPI apps on 2026-04-30 and has since
grown to **six locally-served sites**, mirroring the production split
(**`.com` = product, `.org` = project** — see `docs/` on the IA redesign).
`start-all.sh` is the source of truth for the port map:

| Site | Dir | Port | Production host | Role |
|------|-----|------|-----------------|------|
| **Game** | `tools/sites/game/` | `8094` | `play.axenstax.com` | PWA runtime — Signet auth, `/game`, WASM bundle (no feedback channel on web) |
| **Docs** | `tools/sites/docs/` | `8095` | `docs.axenstax.org` | Spec / project-management site — ADRs, roadmap, test sheets, native binary download |
| **Marketing** | `tools/sites/marketing/` | `8096` | `axenstax.com` | Public landing — CTAs to game and docs |
| **Wiki** | `tools/sites/wiki/` | `8097` | `wiki.axenstax.com` | Player guide |
| **Learn** | `tools/sites/learn/` | `8098` | `learn.axenstax.com` | Learn-journey lessons |
| **Project** | `tools/sites/project/` | `8099` | `axenstax.org` | Project home |

> **The game is `play.axenstax.com`, NOT `axenstax.app`.** That domain has no DNS
> record at all — it was the pre-cutover plan and never existed. `tools/sites/claim/`
> also deploys (`claim.axenstax.com`) but is **deliberately isolated** — never link
> to it. `deploy.yml`'s smoke-test block is the authoritative list of live hosts.

Bring them all up at once (six public sites, plus the isolated `claim` site):

```bash
<repo>/tools/sites/start-all.sh
```

Stop them all:

```bash
<repo>/tools/sites/stop-all.sh
```

Each site has its own `start.sh` if you want to run one in isolation. Each
script kills a stale `python app.py` on its port and refuses loudly if
something unrelated holds it. Logs land at
`/tmp/axenstax-{game,docs,marketing}.log`.

Inter-site URLs are picked up via `GAME_URL` / `DOCS_URL` / `MARKETING_URL`
env vars (defaults: `https://localhost:80{94,95,96}`) so the marketing site's
"Open the game" button and the docs home's "About" link point at the right
host in production by setting those once.

First-time setup (venv + deps) is handled by each `start.sh` automatically.
If `python3 -m venv` fails on this host (Debian/Ubuntu without `python3.12-venv`),
copy one working site venv into the others (e.g.
`for d in docs marketing wiki learn project; do cp -a tools/sites/game/.venv tools/sites/$d/.venv; done`).

> **The voice server / "Games Master" chat widget is REMOVED** (decommissioned
> 2026-06-23 — implementation `tools/voice-server/` deleted, widget already
> stripped from the sites). Do **not** spin it up, re-add it, or suggest it.
> Feedback flows through the serverless lobby mailbox (`/bug`, `/idea` → NIP-17
> DMs; **native only**, off by default — hidden tester unlock (Settings → tap version ×7), 2026-10-03 — the web build has NO feedback channel, removed 2026-10-01
> (owner decision; also: our site apps do not log IPs — uvicorn `access_log=False`);
> native: `game/engine/src/native_mailbox/` per-report burner-key send + in-game
> `/mailbox` status board (no replies, 2026-10-01, spec
> docs/foundations/2026-10-01-feedback-status-board.md); reader
> `tools/feedback-reader/`; spec
> `docs/foundations/2026-06-07-lobby-mailbox-feedback.md`). (Old web reports may still sit on the relay; the gift-wrap
> helper the readers share is `tools/feedback-reader/lib/nip59.cjs`.)

### Quick check

| Service | Local URL | Live URL |
|---------|-----------|----------|
| Game site | `https://localhost:8094` | `https://play.axenstax.com/game` |
| Docs site | `https://localhost:8095` | `https://docs.axenstax.org` |
| Marketing site | `https://localhost:8096` | `https://axenstax.com` |
| Wiki (player guide) | `https://localhost:8097` | `https://wiki.axenstax.com` |
| Native download | `https://localhost:8095/download` | `https://docs.axenstax.org/download` |

## Verification

`./check.sh` at repo root is the single regression gate. It runs (in order):
0. Version parity — `game/engine/Cargo.toml` and `tools/packaging/packager.toml` must carry the same `version` (cargo-packager names artefacts from the latter, and `/download/latest.json` reads the version off those names). Then the docs-site unit tests (`tools/sites/docs/test_*.py`, bare `python3`, no venv) — the `/download/latest.json` contract. Then **every other site's tests**, each in that site's own `tools/sites/<site>/.venv`: `game` and `marketing` under pytest, `console` as plain scripts (its `check()` helpers set the exit status; pytest would pass them blindly). A missing venv or test dependency is a FAILURE that prints the exact fix (`(cd tools/sites/<site> && python3 -m venv .venv && .venv/bin/pip install -r requirements.txt [-r requirements-dev.txt])`), and a site that gains a `test_*.py` without being registered in `check.sh`'s `site_tests` list fails the gate. In a linked worktree (venvs are gitignored) it falls back to the main checkout's venv.
1. `cargo clippy` on the engine, gated `-D warnings` (Phase 4b, 2026-07-06) — any warning or error fails the run.
2. `cargo build` on the engine.
3. `cargo test --lib` — runs every `#[cfg(test)]` module in the engine library (currently about 4,700 tests, counted by `#[test]` attributes, across pure-function units + `TestHost`-driven integration tests under `src/test_integration/`). Never `--bin`: the bin is a thin shim, so it runs zero tests and still passes.
4. `trunk build` for the WASM bundle.
5. Bundle-size gate — brotli-compressed total must stay under 5 MiB (PWA alpha spec).
6. With `--smoke`: Playwright smoke test against a running website. Confirms `/`, `/game`, WASM asset, JS loader all serve correctly. Saves a screenshot to `tools/smoke/out/play-landing.png`.

Flags: `--release` uses the release profile (slower, matches CI). `--smoke` requires the website running on `:8094`.

**CI (manual only).** `.github/workflows/check.yml` runs `./check.sh` on an `ubuntu-latest` runner (system `python3` venvs for the sites, trunk and cargo-deny as sha256-verified prebuilt binaries, same action pins as the other workflows). It is `workflow_dispatch` only (Actions -> Check -> Run workflow): the owner decides when to spend Actions minutes on pushes/PRs, and a commented-out `push`/`pull_request` block in the file is the one-line switch. Until then, green `./check.sh` on your machine is still the bar. Bumping the trunk or cargo-deny pin means updating the workflow AND (for cargo-deny) the hint in `check.sh`.

First-time setup: `(cd tools/smoke && npm install && npx playwright install chromium)`.

### Testing

- **Pure-function unit tests**: `#[cfg(test)] mod tests` at the bottom of each source module (see `chunk.rs`, `spawning.rs`, `falling_blocks.rs`, `combat.rs`, `mob_ai.rs`, `save.rs`, `protocol.rs`, …). These assert real invariants on extracted free functions — not placeholders.
- **Integration tests**: `src/test_integration/{handshake,mobs,blocks,physics,smoke}.rs`, all gated `#[cfg(test)]`. They drive `TestHost` (in `src/test_harness.rs`), a synchronous wrapper around `GameServer` with no transport thread — fast, deterministic.
- **Adding a new integration test**: either extend `TestHost` in `src/test_harness.rs` with a new helper, or drop a file in `src/test_integration/` and register it in `src/test_integration/mod.rs`. Both are reached by `cargo test --lib` (which `check.sh` runs).
- The engine is a library (`src/lib.rs`, `rlib` + `cdylib` for Android) plus a thin bin shim (`src/main.rs`). The tests still live inside the crate rather than under `tests/*.rs`; they could move out mechanically now that a lib target exists.

### Authoring Trials (Explorer Challenges)

To add a new Trial, follow **`docs/authoring/trials.md`** — the annotated JSON
schema, the ordered "6 places to touch" checklist (a trial is a JSON file plus
scattered Rust registrations), the event palette, valid mob/block/kit names, and
a worked example + copy-paste skeleton (`game/engine/assets/scenarios/_skeleton.example.json`).
The `trials_lint` module (`src/test_integration/trials_lint.rs`, run by `check.sh`)
turns every silent-failure mode into a loud test failure — an objective listening
for an unfired event, a `Sequence`/`Checklist` leaf that isn't an `Action`, an
arena that doesn't provide the objective's species, an unrecognised (typo'd) JSON
field, or a money/earning word on any text surface. After authoring, `cargo test
--lib trials_lint` must be green; its failure messages name the
exact fix. Designed so a weaker model can author a correct trial.

## Project Posture

**Moonshot. Go big or go home. Full custom build.**

## Regulatory Red Lines — NEVER cross (load-bearing, every session)

**The whole compliance strategy rests on one principle: these laws (US KIDS Act,
US state laws, UK OSA, AU under-16 ban, COPPA) regulate *the operated service and
whoever controls access to it* — NOT the software. AxeNStax ships neutral,
self-hostable software; the operator/parent runs the service and is the regulated
party. It is FINE to build software that lets others self-host and run their own
groups; it is NOT fine to build anything that makes AxeNStax the operator/platform.
"Free" is never the dividing line — *operating the channel* is.** Full analysis +
current-build audit: internal repo, `docs/research/2026-07-01-kids-act-regulated-party-and-private-groups.md`.

**Before building or changing ANYTHING touching networking, discovery, identity,
hosting, data, or public copy, check it against these four lines. Crossing any one
turns AxeNStax from a software vendor into the regulated platform.**

1. **No public directory that AxeNStax operates which discovers/lists player-run
   servers/worlds/groups.** Discovery must stay LAN-local, decentralized **opt-in**
   Nostr announce (self-published by the operator, `--announce` never automatic),
   or direct-address. Any roadmap browser/matchmaking stays **opt-in +
   default-unlisted + child-safe**. A central browsable index = crossing the line.
2. **AxeNStax operates no game servers or the relays that carry a group's traffic.**
   Game worlds are self-hosted (direct QUIC/WebSocket); multiplayer does NOT run
   over Nostr. `relay.trotters.cc` is ours and is **no longer an AxeNStax-chosen default
   in the native app** (2026-10-02; caveats: mySignet itself may name trotters
   as the contacts grant relay, and the owner-local feedback reader still
   listens there for pre-v0.2.28 builds): sign-in, contacts pairing, online play, Server Card discovery and
   the updater use the player's own "Your relays" list (default
   `server_resolve::PUBLIC_DEFAULT_RELAYS`), and feedback goes to fixed public
   inbox relays (`native_mailbox::FEEDBACK_INBOX_RELAYS`). Players may add trotters
   themselves; it must NEVER carry a group's game traffic, presence, or in-game
   chat. No AxeNStax-hosted game/cloud fleet for others' groups.
3. **No central collection of kids' data.** Web taster stays anonymous/local (no
   login, cookies, analytics, or age data). Native Stash stays BYO-key /
   non-custodial / ciphertext-only. Feedback stays burner-key + E2E + owner-local.
   **Never affirmatively collect age** as raw data (age = a boolean from third-party
   Signet, never a DOB to our server). No kid PII to an AxeNStax-operated server.
4. **Never position/market the product as a "social network / kids' social media /
   chat platform."** Lead sovereignty / ownership / building. "Play with friends" =
   co-building, not social networking. Marketing claims must never run ahead of
   built capability (a safety claim ahead of the code is the FTC "unfair/deceptive"
   hook — e.g. don't advertise parental controls that aren't shipped).

**Corollary (Grokster): facilitate, never induce.** Shipping neutral, documented,
self-hostable software is protected. Do NOT build an induce-the-bypass path — a
"dev mode" or a disclaimer that is really a how-to for turning safety off. Sensitive
features (comms, etc.) are gated at a real **capability/age boundary** (Signet), not
disclaimed. Enabling others to self-host = fine; nudging them to strip safety = not.

## Rules

- **Do NOT build anything unless explicitly asked.** No code, no scaffolding, no implementations unless the user says to build it. Research, planning, and docs are fine.

## Team

- **Staxolottle** — Dev, tech stack, architecture, business decisions. The one who says "build it" when it's time.
- **Axolittle** — Minecraft gameplay expert, vibe coder, builder of blocks. Knows the game inside out — mechanics, feel, what makes it fun. Will often be chatting directly.
- You'll likely be able to tell who's talking from context and tone. When in doubt, ask.

## Project Overview

Open-source, massively scalable voxel sandbox platform — custom engine, custom client, custom server. Sovereignty-first: players own their identity, worlds and creations, and the whole stack is self-hostable. **Proof of Play** is an educational proof-of-work mechanic (every strike hashes); on operator-run Bitcoin-enabled servers it can optionally drive sats via a deterministic work-meter. Never describe or market it as "earn Bitcoin" (UK FCA financial-promotion risk; see red lines).

No existing engine (Luanti, Veloren, etc.) can deliver this vision without becoming the ceiling. See `docs/architecture/ADR-001-full-custom-engine.md`.

## Key Architecture Decisions

- **Engine**: Full custom build — own the entire stack, no inherited ceilings
- **Payments**: LNbits as integration hub, Lightning Network for micropayments
- **Scaling**: Shard-based (many independent world instances), not single mega-world
- **Orchestration**: ~~Kubernetes + Agones fleet~~ **RETIRED** — an AxeNStax-operated fleet for others' groups crosses red line 2. Worlds are self-hosted (single binary or the operator's own Docker).
- **Textures**: 16x16 default, resolution-agnostic renderer
- **Reward Mechanic**: **Proof of Play** — every pickaxe strike runs HMAC-SHA256 (even on grass) and the hash is surfaced to the player as an educational proof-of-work primitive. The same hash deterministically drives optional rare drops on plain stone and, on Bitcoin-enabled servers, sats payouts via a **deterministic work-meter** (effort accumulation — never a chance-based "probabilistic" threshold; a probabilistic real-Bitcoin payout sits inside the UK Gambling Act gaming perimeter, s.6, where free-to-play is not a defence, so it is **retired for real sats** — see `docs/research/2026-06-21-uk-online-safety-gambling-crypto-landscape.md`). Visible ore blocks (coal/iron/diamond — Wave 13) are Minecraft-style guaranteed drops with the correct tool. The player is **not** a Bitcoin miner; on Bitcoin-enabled servers the server operator translates proof-of-play effort into payouts. Anti-X-ray = (a) architectural defence at the reward layer (`server_secret` never leaves the server — this part is live) + (b) chunk-stream obfuscation that replaces buried ore with stone in the data sent to the client; exposed ore in cave walls stays visible. **(b) is built + unit-tested but not yet wired into the live chunk-stream send path** (`game/engine/src/anti_xray.rs`, `#![allow(dead_code)]`) — it lands with real remote-multiplayer chunk streaming. Don't describe (b) as active protection until that lands. Full design: `docs/foundations/2026-05-12-proof-of-play-clarification.md`.
- **Wallet**: Noncustodial Lightning wallet built into PWA, derived from Nostr 12-word seed
- **Identity**: Nostr (Signet persona) address per player. Parent/guardian relationships belong to Signet, not to engine code: the engine has no parent or child account model, only the local `PlayerSlot.charter_allows_sats` flag and the tightening-only comms policy file (see known debt)
- **Not a money transmitter**: Platform never touches funds — critical policy constraint
- **Development**: AI-driven development from specification documents

## Key Patterns

### Sign-in (as built)

The **web build is an anonymous, login-free offline taster**: `tools/sites/game/` has
no `/auth/*` routes, sets no cookie, and `static/vendor/signet-login.iife.js` is
gone (`tools/sites/game/test_no_login.py` pins this; web has no multiplayer, cloud
save or feedback channel). The old `signet-login` SDK cutover docs under
`docs/integrations/signet/` are historical.

**Native** sign-in is a NIP-46 / `nostrconnect` pairing driven by
`game/engine/src/native_signin.rs` (QR shown in the lobby; relays come from the
player's own "Your relays" list). Join-time identity (kind-21236 auth event,
kind-31000 handle credential) is covered under the Phase 4 entry in "Resolved
technical debt" below and in Spec 04 §1.8 / Spec 08 §9.0.1.

### Online play by contact (native)

A host binds one UDP socket, gathers reachability candidates on it (LAN, IPv6,
UPnP via `igd-next`, STUN), and hands that socket to quinn. A joiner picks a
**contact** — a Signet persona npub — and the two exchange an encrypted
offer/answer over public Nostr relays (kinds 20900/20901, NIP-44, signed by a
per-install **runtime key** the persona attested once with `role=player`), punch
through their routers, and connect **directly**. The QUIC join handshake and
`PROTOCOL_VERSION` are unchanged. Admission is "in my contacts at Kin or Kith,
or holding my live invite bearer"; everyone else gets silence. Relays carry
setup only and never game traffic. Modules: `invite.rs`, `runtime_identity.rs`,
`online_admission.rs`, `rendezvous/`, `nat/`, `online_host.rs`,
`online_join.rs`, `friends_ui.rs`. Spec:
`docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`, Spec 04
§1.9. Native only — the web build carries none of it, and
`tools/smoke/forbidden-symbol.mjs` proves that. Regulatory framing: this is
software the two players run themselves, never an AxeNStax-operated service —
see the red lines above and Spec 04 §1.9's "supersedes §1.7" note.

## Project Structure

```
docs/
  vision/          — Core vision, platform overview, design philosophy
  architecture/    — Architecture Decision Records (ADRs)
game/
  engine/          — Custom voxel engine (client + server)
  textures/        — Block textures (16x16 default pack)
platform/          — RETIRED placeholders (matchmaking/session directory/Agones cross red lines 1+2; see platform/README.md)
tools/
  research-viewer/ — Local web app for YouTube research (port 8888)
infra/
  k8s/             — RETIRED (see infra/k8s/README.md)
  docker/          — Dockerfiles for server images
```

## Key Docs

- `docs/vision/platform-overview.md` — Core vision, world types, cost model, visual strategy
- `docs/architecture/ADR-001-full-custom-engine.md` — Why full custom build
- Research reports that informed the ADRs (engine selection, Bitcoin/Lightning integration, market/policy) live in the internal repo under `docs/research/`.
- `docs/architecture/ADR-002-tech-stack.md` — Rust, wgpu, hybrid Bitcoin, two-tier hosting

## Spec Documents (docs/spec/)

Specs 01-04 carry "AS-BUILT" banners (audit 2026-10-04) where the as-designed text diverges from the shipped engine; read the banner first.

| # | Spec | Lines | Covers |
|---|------|-------|--------|
| 01 | Engine Architecture | 1,687 | **As-designed** 12-crate workspace; **as-built is ONE crate, `game/engine`** (wgpu 29, glam 0.33; no wasmtime plugin host). ECS (hecs), tick loop (20 TPS), threading, asset pipeline. Has an as-built banner |
| 02 | World Format | 2,007 | Block registry (u16), 16x16x16 chunks, palette compression, world gen pipeline, lighting, persistence |
| 03 | Rendering | 2,344 | wgpu pipeline, greedy meshing, texture arrays, 3 GPU tiers, procedural sky, egui UI, resource packs |
| 04 | Networking | 1,674 | UDP + WebRTC dual transport (design; as-built is QUIC + WebSocket, `u32` protocol version), custom protocol, client prediction, 3-tier spectator system, anti-DDoS |
| 05 | Gameplay Systems | 3,872 | Movement, block interaction, inventory, crafting, combat, mobs, game modes, particles/audio |
| 06 | Bitcoin Integration | 2,423 | Hash-on-mine, reward economics, LNbits API, revenue splits, creator kit, Signet age verification |
| 07 | Platform Services | 1,965 | **RETIRED banner** — operated matchmaking/Agones/session directory cross red lines; remaining: world lifecycle, region simulation, portals, moderation, cost controls |
| 08 | Security & Anti-Cheat | 1,736 | Threat model, server authority, anti-bot, payment security, plugin sandboxing, privacy, incident response |

## Tech Stack

- **Engine Language**: Rust (compiles to WASM for web, native for desktop)
- **Renderer**: wgpu (WebGPU/Vulkan/Metal/DX12 — one API, all platforms)
- **Client Targets**: **PWA-first for alpha** (ADR-003) — WASM + WebGPU in Chromium (Chrome/Edge). Native remains a first-class long-term target but is not gating alpha. Firefox/Safari are out of scope until post-alpha.
- **Alpha Auth**: Sign-in via `mysignet.app` (signet-app) — QR redirect flow only, no deeper Signet protocol integration. See ADR-003.
- **Networking**: Existing crates first (quinn/QUIC), custom protocol when voxel-specific needs demand it
- **Payments**: LNbits (MIT) + Lightning Network + keysend splits
- **Liquidity**: Amboss (~0.5%/tx), mandatory ~1% Lightning fee baked into splits
- **Infrastructure Billing**: BitLaunch (hourly, Bitcoin) — costs paid from transaction flow, not out of pocket
- **Bitcoin Model**: Hybrid — game works without Bitcoin, Bitcoin-enabled servers are flagship
- **Self-Hosting**: Single binary (personal) or the operator's own Docker — same engine
- **Infrastructure / Matchmaking**: ~~Kubernetes, Agones, Open Match~~ RETIRED (red lines 1+2) — discovery is LAN, opt-in self-published Nostr announce, or direct address
- **World Storage**: Pluggable backends, cloud-native snapshots

## Spec Maintenance

**The spec documents are the source of truth. They must always be up to date.**

- When a bug is found and fixed, update the relevant spec with what was wrong and the correct approach
- When a design decision is made or changed, update the spec immediately
- When Axolittle's feedback changes a gameplay mechanic, update the gameplay spec
- The engine will likely be rebuilt from scratch multiple times — the specs are what survive those rebuilds
- Bug fixes, winding orders, coordinate conventions, physics constants — all of it goes in the spec, not just in git history
- If it's not in the spec, it will be lost on the next rebuild

## Build Priorities

1. Scalability architecture
2. Gameplay systems (movement, block placement, chunk streaming)
3. Multiplayer stability + world persistence
4. Payment integration
5. Visual polish (iterates independently)

## Code Quality: Concrete, Not Cards

**Build for the moon, not for the demo.** Every system should be production-grade architecture that scales to multiplayer, modding, and thousands of concurrent players. The specs describe the destination — the code should be heading there, not just passing today's test.

### The Rule

Every piece of code must be either **concrete** (production-grade, spec-aligned, will survive a rebuild) or **explicitly marked as a bridge** (temporary, with a TODO pointing to what replaces it and when).

### Concrete means:
- **Proper abstractions** — items are items (not "blocks pretending to be items"), tools live in inventory (not a separate field), systems communicate through clean interfaces
- **Spec-aligned** — if the spec says "unified item registry," don't build three separate systems for blocks, tools, and food
- **Multiplayer-ready** — every system should work when there are two players. "Works in single-player" is not enough if it can't be made authoritative
- **Data-driven** — mob definitions, recipes, loot tables in data structures, not hardcoded match statements where possible
- **Decomposed** — no 2000-line god files. Each module has one clear responsibility. If a file is growing past ~500 lines, it's doing too much

### Bridge code (temporary) must:
- Be marked with `// BRIDGE: <what this should become> — replace when <trigger>`
- Be confined to one file/function (not spread across the codebase)
- Be replaced in the next session that touches that system, not left to rot
- Never be built on top of — if the next feature needs the bridge, replace the bridge first

### Before building a new feature, check:
1. Does the foundation support it? (e.g., "Do we have a proper item system before building crafting?")
2. If not, build the foundation first — even if it takes longer
3. If a bridge exists underneath, replace it before stacking

### Known technical debt (fix before building on top):
- **Single-player bypasses GameServer.** NewWorld and LoadWorld run no server, so client-side `GameState::tick()` runs the whole simulation. A LAN / online **host** no longer does: since D1 (2026-10-06) it lends its one `World` + ECS to its embedded `GameServer` every tick (`sim_lend.rs`, RAII `LentSim`; one owner per shared system via `SimSystem::lent_owner`, tripwire `World::sim_tally`), so the host's dual sim is gone and joiners are diffed from the host's real world. Still on the host client by design until D4: clock + weather, the mob-locomotion block (brigand pre-pass → `mob_ai` → entity physics, because species AI overrides sit between them) and the death sweep (single kill-attribution site). Remaining: D3 — single-player lends too (a `HostedServer` with no transport), default flipped after an Axo session; then D4 moves the client-owned rows into `GameServer::tick` behind a `SimEvents` outbox. `--no-lend` (one release) restores an owning host server. A **joiner** runs no mob world of its own since D2a (2026-10-07, protocol v68): it draws the server's mobs and carts from a render-only mirror (`remote_mobs`), its spawners and raids are off, and the server lands mob melee and lava/fire contact on its body (its health is the server's). Spec 01 §4.1.3, Spec 04 "Hosted mode — the host lends its world", Spec 04 §4.2c.
- **`--no-lend` escape hatch** (BRIDGE, D1 2026-10-06, one release). A host started with `--no-lend` keeps an OWNING server with a second copy of the world, and everything only that path needs: the `HostWorld::Owned` branch of `GameState::tick_hosted_server`, the host→server block-entity mirror (`HostedServer::mirror_host_world_state`), the server's 3×3 refill round joiners (`column_refill_per_tick`), `join_spawn`'s column generation, and the host loopback's generate-apply-evict for a server change in a column the host client has not loaded. Trigger: delete all of it together one release after D1 ships (after the owner's hosted-ON playtest). Spec 01 §4.1.3.
- **`ServerPlayer` vs `PlayerSlot` duplication** (BRIDGE in server.rs). Save path uses raw ServerPlayer fields rather than PlayerSlot; the two structs will drift. Trigger: when save becomes server-authoritative. D2a consequences, same trigger: a joiner's armour points (`InputPacket.armour_points`) and its own health changes (`InputPacket.health_delta`: eating, regen, poison, starvation) are client-asserted, and armour durability does not wear from the hits the server lands on a joiner.
- **Joiners cannot interact with the server's mobs yet** (D2a, 2026-10-07). Attack, tame, feed, breed, ride, lead, shear, milk and trade on a mirrored mob show a "not available when you've joined someone else's world yet" toast (`remote_mobs::JOINED_INTERACTION_TOAST`). Species-AI attacks (bee, goat, shark, bear) still run only on a host's client and reach only its local players; a server-side death shows a generic cause. Trigger: D2b (`EntityAttack` → server damage + kill events, then per-feature interaction packets).
- **`PlayerSlot.charter_comms` reads a local tightening-only policy file** (BRIDGE). Charter has no comms capability — verified 2026-09-05: `comms` is a reserved word in Charter's prose contract, not a shipped clause kind; the published SDK type is a single `kind: 'schedule'` literal. Since 2026-09-28 the file can only LOWER a player's ceiling below the safe default (`Approved`), never raise it — the old self-named `guardian_npub` signature proved nothing, so it is ignored. Raising waits for a real capability boundary. Trigger: the Charter comms clause shipping upstream, or a Signet guardian attestation. See `docs/foundations/2026-09-05-world-chat.md §3.3`.

- **`GameServer::tick` is not the whole simulation.** It runs mobs, water, leaf decay and player physics, but not pistons, hoppers, kegs, dispensers or crops, and it discards the `Vec` that `leaf_decay.tick` returns (`server.rs:862`). Those systems run only in the client-side `GameState::tick`, so a dedicated server has no working farms or machines. Trigger: tick parity, or routing single-player through `HostedServer`.
- **No possession check on joiner block placement** (BRIDGE, `hosted_server.rs:1892`). The server keeps no authoritative inventory for a remote player, so placing an item the joiner does not hold is not refused. Trigger: remote inventories become server-authoritative (with the `ServerPlayer`/`PlayerSlot` debt).
- **`World::set_block` auto-creates chunks** (`world.rs:1601`, via `chunk_for_block_write` `.or_insert_with(Chunk::new)`): a write to an unloaded cell conjures an empty chunk instead of failing. Callers cannot tell a missing chunk from a real write; audit callers before relying on it.
- **Five economy owner enums carry `LocalPlayer(pidx)`** and must converge on `Npub` together: `VendorOwner` (`vendor.rs:122`), `TipJarOwner` (`tip_jar.rs:34`), `AuctionOwner` (`auction.rs:36`), `PlotOwner` (`plot.rs:29`) and `HubOwner` (`market_hub.rs:30`). (The audit counted four; the code has these five. `ScreenContent::LocalPlayer` in `screen.rs` is a different, legitimate use.)

### Resolved technical debt:
- ~~Joiners run a private mob world and take hostile damage from it client-side~~ (D2a, 2026-10-07, protocol v68). The server's mobs and carts reach joiners through a per-client, changed-only entity diff (`entity_broadcast`: 80/96-block interest radius, velocity + render flags on `EntityUpdate`; a late joiner's empty interest set replaced the backfill) and are drawn from a render-only mirror in its own ECS (`remote_mobs`). The server lands hostile melee, lava/fire contact and explosion blasts on every joiner's body (a joined client no longer applies blast damage to itself) (`GameServer::tick_player_hazards`, the client's rules shared as `combat::hostile_melee_tick` / `survival::contact_hazard`); a joiner's health is the server's, with its client-owned changes reported as `health_delta` and reconciled (`health_sync`). The server's non-lethal starvation-floor BRIDGE is gone: the server runs no metabolism for any body (only `tick_timers()`, local slots included); a reported heal is capped per input (`MAX_REPORTED_HEAL_PER_INPUT`), so joiners can't sleep-to-full yet. Spec 04 §4.2c, §5.3.2; Spec 05 §6.4.1.
- ~~`HostedServer::mirror_host_world_state` is O(block entities) per call~~ (D1, 2026-10-06) — gone from the default path: a host lends its server its one world, so there is no second copy to mirror. It survives only on a `--no-lend` host (BRIDGE, deleted with that flag), where the server keeps its own copy and would otherwise spill stale chest contents.
- ~~`HostedServer` local-player position-trust path (BRIDGE)~~ — **design, not debt** (MP step 1 + D1, 2026-10-06, Q1 trap 9): a host's local slots stay position/health-trusted because the host is the authority's own machine, and on a lent world the client that moves them also owns the world — simulating its own players' input there buys no authority. `ServerPlayer`/`PlayerSlot` stay dual for local slots; on a lent world a local slot's edits are broadcast without the joiner budget/validation. Every joiner has ONE position, the server-simulated one: the joiner predicts it and reconciles against `last_acked_input` (snap beyond 1 block, camera glide below). See `docs/spec/04-networking.md §5.3.1`.
- ~~Dispenser arrows on a dedicated server are invisible to joiners~~ (MP-A3, 2026-10-06, protocol v67). `GameServer::tick` now runs the same `entity::tick_projectiles` the client runs wherever `simulates_block_machines` is set, and the entity diff (`entity_broadcast` since D2a) broadcasts projectiles as `EntityKind::Projectile`; joiners render them from `remote_entities::RemoteProjectiles`. Same change: a dead joiner stays dead on the server until it sends `PacketType::Respawn` (the 40-tick server-copy revive BRIDGE and `PlayerCombat.respawn_timer` are gone). Spec 04 §4.2b.
- ~~`JoinRequestPacket.player_name` is client-asserted~~ (Phase 4 cutover, 2026-06-16, protocol v49). The multiplayer-identity design (`docs/spec/04-networking.md §1.8`) replaces this with `auth_event: SignetAuthEvent` + `handle_credential: Option<SignetCredential>` — handle sourced from the signed kind-31000 `display-name` tag, never from the client string. Batch A bounded + sanitised the field as short-term hardening. **Phase 4 cutover IMPLEMENTED 2026-06-16** (protocol v49): the handshake reorders (authed client waits for `ChallengePacket` → signs `{nonce, origin}` off the main loop → sends `JoinRequest` with the signed `auth_event`); the server verifies any present auth_event (tamper/invalid → reject) and rejects an *absent* one on a sign-in-required host (`HostedServer.require_signin`, `true` for the QUIC LAN host; **the dedicated server requires sign-in by default too since 2026-10-06** (owner decision O-7 #3) — `--allow-guests` / `AXENSTAX_ALLOW_GUESTS=1` opens it to guests, the old `--require-signin` / `AXENSTAX_REQUIRE_SIGNIN` are accepted no-ops, and the `<identity-dir>/require_signin` file (console toggle / admin command) overrides both, `server_main::load_access_policy`). Native `ws://` joins to a dedicated server sign in when a signer is restored (`connect_websocket_authed`); the web build has no sign-in, so web players need a guest-open server. `USE_SIGNET_AUTH` is **retired** — identity is policy-driven (`hosted_server::resolve_join_identity`), not flag-gated. `player_name` is now a **display fallback only**, never trusted; the verified npub is stored on `ServerPlayer.verified_pubkey` for economy-block ownership; collisions get a `-<npub-suffix>` readable label, and a client-side **inspect view** (`hud_ui::draw_player_inspect`, fed by `RemoteClient::roster` + the npub now on the `Joined` event) shows each present player's full, copyable npub. The signing bridge exists on **native only**: `game_loop::native_join_sign_driver` (restored bunker on a worker thread). The web half was never finished — `wasm_auth::js_sign_driver` consumes `window.__axenstax_sign_auth_event`, but nothing defines it (audit 2026-10-04), and the web build is now an anonymous offline taster anyway, so there is no web sign-in path. **Remaining = owner live test only** (2-machine LAN with a real bunker pair, native↔native + native↔dedicated). See `docs/foundations/2026-04-20-engine-signet-auth.md` + `docs/goals/2026-06-16-phase4-signet-multiplayer-auth.md`. **Charter Phase 1 decoupled 2026-05-09** when Charter pivoted to rev. 7 mechanism A (relay-read only, no bunker calls in the hot path) — Spec 10 delivered on main. **The in-engine schedule-Charter gate was STRIPPED 2026-05-26** (the `charter::check` session-start gate, the deny overlay, `charter-*.js`, and the `/auth` Charter store are gone — superseded by the standalone `@forgesworn/charter` SDK shipped upstream 2026-05-25; preserved in git history). Note: the unrelated `charter_allows_sats` Bitcoin parental-gate on `PlayerSlot` is untouched and remains live. The signing-bridge gap still applies to Phase 4 only, plus future Charter mechanism B/C/D specs when they ship. See `docs/spec/08-security-anti-cheat.md §9.0.1` for threat model.
- ~~`main.rs` is too large~~ — decomposed into game_loop.rs, block_interact.rs, chunk_stream.rs, spawning.rs. **Caveat (audit 2026-10-04): the weight just moved — `game_loop.rs` is now about 22,300 lines**, far past the ~500-line guideline; splitting it is open debt
- ~~`held_tool` is a separate GameState field~~ — now in PlayerSlot.hotbar_slot
- ~~Tool cycling via T key~~ — replaced by proper inventory/crafting system
- ~~`static Mutex` for world name~~ — now a field in GameState and GameServer
- ~~No unified Item type~~ — Item enum covers Block, Tool, Material
- ~~Tool persistence broken~~ — all item types (blocks, tools, materials) now serialized with full data. Legacy saves auto-upgrade on load
- ~~Entity models rebuilt every frame~~ — model parts cached per MobType via LazyLock
- ~~`GameServer.tick()` doesn't exist~~ — server now runs world time, water/leaf decay, mob AI, entity physics, combat. HostedServer calls it
- ~~World time 4× speed for alpha~~ — rolled back post-playtest. `GameState.world_time_step` defaults to 1 in `main.rs::GameState::new`; server.rs world-time increment matched. `/time speed <n>` remains for runtime override testing. See `docs/foundations/2026-05-07-engine-commands.md`.
- ~~Mob AI targets player 0 only~~ — targets nearest player from all positions
- ~~Footstep sounds player 0 only~~ — triggers for all local players
- ~~`HostedServer` hardcoded is_creative/difficulty~~ — reads from world metadata
- ~~Mob spawning lived only in client-side `impl GameState`~~ (Task 1b) — now a pure free function `spawning::tick_mob_spawning` called from both GameServer and the single-player client path. GameServer runs the 400-tick cycle for hosted games.
- ~~Falling blocks coupled to renderer via `renderer.upload_chunk_mesh` inside `tick_falling_blocks`~~ (Task 1c) — extracted to pure free function `falling_blocks::tick_falling_blocks` returning `Vec<BlockChange>`. Mesh rebuild is now a separate client-side pass on dirty chunks.
- ~~`HostedServer` input processing trusted client-provided position for ALL players~~ (Task 1d, partial) — remote players are now server-simulated via `PlayerIntent::from_input_packet` + `Player::tick`, with horizontal speed cap as anti-cheat seed. Local players stay position-trusted by design (see the entry above).
- ~~`HostedServer` ran on a dedicated thread + tokio runtime, incompatible with `wasm32-unknown-unknown`~~ (Phase 2) — HostedServer is now main-loop-driven: `tick()` is called from the client's existing 20 TPS accumulator; channel transports replace mpsc between threads. The QUIC accept thread + LAN broadcaster stay native-only and are feature-gated; on WASM `max_remote_players` clamps to 0. The `transport`/`server`/`hosted_server` modules compile cross-platform.
- ~~`StateUpdatePacket.entity_spawns`/`updates`/`despawns` declared but never populated~~ (Phase 3a) — HostedServer now assigns a stable `ProtocolId` component to each mob on first broadcast and emits spawn/update/despawn deltas each tick via `diff_entities`. Unit tests cover first-broadcast spawn+update, subsequent-update-only, exactly-once despawn, and id stability.
- ~~Multi-player save silently dropped Player 2 on load~~ (Phase 0) — `chunk_stream::initial_load` pre-allocates PlayerSlots + GPU resources + screen layout to match the saved player count before running the restore loop. Split-screen stays native-only, so WASM continues to restore only slot 0. Covered by three new integration tests in `test_integration/save_load.rs`.
- ~~No engine command surface (chat / `/time` / `/gamemode` / etc.)~~ (2026-05-08) — engine commands feature delivered. Module: `game/engine/src/commands/` (parser, registry, dispatcher, 7 built-ins) + `chat_ui.rs` (egui overlay). T or / opens chat in-game, gameplay input gates while open, World Integrity Ledger flags persist on cheats. Plugin-shaped for cross-game lift. Spec: `docs/foundations/2026-05-07-engine-commands.md`. Phase 5 (multiplayer command sync) and Phase 6 (Axolittle UX playtest) deferred per the spec. Non-`/` lines are now world chat on the transport (`ChatSay`/`ChatDeliver`, protocol v60), not a local echo — native only. See `docs/foundations/2026-09-05-world-chat.md`.
- ~~Survival mode starts with a full inventory~~ (2026-05-07) — `Inventory::new()` returns empty; creative still gets its starter blocks via `chunk_stream::initial_load`. Spec 05 §4 updated.
- ~~In-game "Switch user" button labelling + world list always single-column~~ (2026-05-07 + 2026-05-21) — renamed to "Log out". Initially switched to `horizontal_wrapped` for 2-column desktop layout, but 2026-05-21 playtest showed kids didn't notice the vertical-scroll affordance and treated the 2×2 grid as "only 4 worlds available". Reverted to single-column vertical scroll with `auto_shrink([false, false])` so many-worlds saves are clearly scrollable.

## Daily Build-Test Workflow

Every build session follows: **Build → Test Sheet → Axolittle Tests → Feedback → Next Build**.

- Workflow doc: `docs/workflow/daily-build-test.md`
- Test sheets: `docs/test-sheets/YYYY-MM-DD-build-name.md`
- Claude builds unattended, generates test sheet with checklist + questions
- Axolittle runs through the test sheet, answers questions, adds notes
- Axolittle's feedback drives the next build session

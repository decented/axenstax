# Single-player → HostedServer routing

> **SUPERSEDED (2026-10-06, D1).** Do not build from this doc. Its target —
> the client ECS as a render-only mirror fed over an in-process transport —
> is the rejected alternative. The chosen design is **lending**: a host
> client lends its one `World` + ECS to its embedded `GameServer` for each
> tick (`game/engine/src/sim_lend.rs`, RAII `LentSim`), with one owner per
> shared system (`SimSystem::lent_owner`) and a per-world tally tripwire.
> Built for LAN / online hosts in D1 (Spec 01 §4.1.2, Spec 04 "Hosted mode —
> the host lends its world"); single-player joins the same path in D3 (a
> lending `HostedServer` with no transport). Phase 0a below is retired:
> `parity_check` and `ENABLE_SINGLEPLAYER_HOSTED_SERVER` were deleted in D1.
> Kept for history.

**Status**: Phase 0a DELIVERED 2026-05-03 on `main` (commit `47adf4b` — `parity_check` module + `WorldParityHash` + `ENABLE_SINGLEPLAYER_HOSTED_SERVER=false` flag-gated wiring). Phase 0b onwards remains READY TO BUILD — needs Axolittle for each phase's regression session before the next one starts.
**Date**: 2026-04-20 (original); Phase 0a delivery 2026-05-03.
**Branch**: original work on `feat/hostedserver-phase0-transport`, merged to main. Phase 0b: branch off main again.
**Session**: Fresh — implementer should treat this doc as the only brief.
**Risk**: High. This refactor touches live gameplay paths. Land it when Axolittle can test end-to-end, and phase every cut-over behind a feature flag so regressions can be reverted independently.

---

## TL;DR

Single-player today bypasses `HostedServer`. `NewWorld` and `LoadWorld` construct a `GameState` directly and its `tick()` runs all simulation (mob spawning, falling blocks, entity physics, mob AI) on the client. CLAUDE.md §Known technical debt, bullet 1 describes the consequence: the server ECS and client ECS diverge during single-player, and every subsystem has to be maintained twice (once in `server.rs`, once in `GameState::tick()`).

Target shape: single-player = `HostedServer` + 1 local transport + 1 client. The client ECS becomes a render-only mirror. All simulation runs on the server; the client receives authoritative updates over an in-process channel and interpolates.

This is the largest single refactor in the debt list and unblocks real multiplayer, anti-cheat, and replay. It's phased across six increments so each cut-over is isolated and reversible.

---

## Context pointers

- Debt description: `CLAUDE.md` §Known technical debt, bullet 1 (single-player bypasses GameServer), bullet 4 (`HostedServer` local-player position-trust path).
- Current client tick path: `game/engine/src/game_loop.rs:~75–170` (`BRIDGE: single-player still runs spawning on the client…`).
- Current server tick path: `game/engine/src/server.rs` (`GameServer::tick`) and `src/hosted_server.rs`.
- Free functions already extracted (Tasks 1b/1c/1d): `spawning::tick_mob_spawning`, `falling_blocks::tick_falling_blocks`, `PlayerIntent::from_input_packet`. These are the foundation — each subsystem already has one pure-function entry point that both sides call.
- BRIDGE note in `hosted_server.rs:~382` — "Local players are position-authoritative; remote players are server-simulated." This debt is bullet 4 in the list, resolved by this refactor.
- `ServerPlayer` vs `PlayerSlot` drift (CLAUDE.md bullet 2) — cleaned up as part of Phase 3.

---

## Target architecture

```
┌───────────────────┐         in-process channel         ┌───────────────────┐
│  Client (GameState) │ ───── ClientToServer[Intent] ──→ │  HostedServer      │
│                   │                                  │                   │
│  Renderer         │ ←── ServerToClient[Snapshot] ─── │  GameServer.tick() │
│  Input            │                                  │  (owns all ECS)    │
└───────────────────┘                                  └───────────────────┘
```

Same architecture used in multiplayer today (UDP transport). Single-player just swaps the transport to one that shares memory. Zero serialization overhead (or optional serialize+deserialize behind a debug flag to catch marshalling bugs early).

---

## Scope

Six phases. Each is independently shippable. Phase 0 is prep (safety net); phases 1–5 each cut over one subsystem.

| Phase | What | Files | Risk |
|------:|------|-------|:----:|
| 0 | In-process transport channel (two MPSC queues) + connect both ends in `NewWorld`/`LoadWorld` paths. No logic changes yet — client still simulates, server runs dark alongside. **Validation only**: assert the two diverge by zero per tick. | `src/transport/*.rs` (new), `game_loop.rs`, `main.rs` | Low |
| 1 | Cut over **mob spawning**. Already a free fn (`spawning::tick_mob_spawning`). Delete the client-side caller; server owns the 400-tick cycle. | `game_loop.rs`, `server.rs` | Medium |
| 2 | Cut over **falling blocks**. Already a free fn (`falling_blocks::tick_falling_blocks`). Same shape as Phase 1. | `game_loop.rs`, `server.rs`, `hosted_server.rs` | Medium |
| 3 | Cut over **entity physics** (remote players + mobs). Also kills the ServerPlayer vs PlayerSlot drift — one struct, sourced from the server. | `server.rs`, `hosted_server.rs`, `save.rs`, `game_loop.rs` | **High** |
| 4 | Cut over **mob AI**. Target selection already moved to "nearest player from all positions" — server owns the decision. Client reads target via state update. | `mob_ai.rs`, `server.rs`, `game_loop.rs` | High |
| 5 | Delete the `HostedServer` local-player position-trust BRIDGE. Single-player and multiplayer are now identical. Bump `PROTOCOL_VERSION`. | `hosted_server.rs`, `CLAUDE.md`, `docs/spec/01-engine-architecture.md`, `docs/spec/04-networking.md` | Medium |

**Phase gating**: ship each phase as its own PR. Each PR has an Axolittle test session before the next one starts. If something feels off to him, we roll that phase back and think harder — faster than waiting until Phase 5 to notice.

---

## Phase 0 — In-process transport

### New file: `game/engine/src/transport/mod.rs`

Define a `Transport` trait that both the network and in-process paths implement:

```rust
pub trait Transport: Send {
    fn try_recv_from_client(&mut self) -> Option<Vec<u8>>;
    fn send_to_client(&self, data: &[u8]);
    // ... mirror the surface hosted_server.rs already uses
}
```

Today's `HostedServer` holds a `Vec<Box<dyn Transport>>` already (or the equivalent — verify the exact trait name in `hosted_server.rs`). This phase generalises that so both UDP and in-process can be plugged in interchangeably.

### New file: `game/engine/src/transport/inprocess.rs`

```rust
use std::sync::mpsc::{Receiver, Sender, channel};

pub struct InProcessServerTransport {
    from_client: Receiver<Vec<u8>>,
    to_client: Sender<Vec<u8>>,
}

pub struct InProcessClientTransport {
    from_server: Receiver<Vec<u8>>,
    to_server: Sender<Vec<u8>>,
}

pub fn pair() -> (InProcessServerTransport, InProcessClientTransport) {
    let (c2s_tx, c2s_rx) = channel();
    let (s2c_tx, s2c_rx) = channel();
    (
        InProcessServerTransport { from_client: c2s_rx, to_client: s2c_tx },
        InProcessClientTransport { from_server: s2c_rx, to_server: c2s_tx },
    )
}
```

Implement the `Transport` trait on the server side. Client side owns its own receive loop driven from the main tick.

**Marshalling**: first cut ships real bincode serialize/deserialize on the in-process path too. Slightly wasteful but catches wire-format bugs during development when both ends share a process. A `#[cfg(debug_assertions)]` flag can skip serialisation in release builds.

### Wiring

`game_loop.rs` in the `NewWorld` / `LoadWorld` branches (~lines 443–545):

```rust
// Before (current):
match crate::hosted_server::HostedServer::start(...) {
    Ok(server) => self.hosted_server = Some(server),
    ...
}
// + GameState::tick runs full simulation

// After Phase 0:
let (server_tp, client_tp) = transport::inprocess::pair();
let server = HostedServer::start_with_transport(server_tp, world_name.clone(), ...);
self.hosted_server = Some(server);
self.client_transport = Some(client_tp);
// GameState::tick STILL runs full simulation — this phase adds the channel
// without moving any logic. Divergence detection asserts that both sides
// produced the same outputs, so we know the wiring is correct before
// flipping ownership.
```

### Divergence detector

Add a `#[cfg(debug_assertions)] divergence_check()` that hashes the mob count, falling-block count, and entity count on both sides each tick, panics on mismatch. This is the safety net for phases 1–4 — any cutover that breaks parity trips this immediately.

### Acceptance — Phase 0

- `./check.sh` + `--smoke` green.
- Single-player launches, plays normally (Axolittle confirms).
- Divergence detector stays silent across a 5-minute single-player session.
- Multiplayer (host game) still works — the `Transport` refactor didn't regress the UDP path.

---

## Phase 1 — Mob spawning cutover

Delete the client-side caller at `game_loop.rs:~81–90`:

```rust
// DELETE THIS BLOCK:
if self.world_time % 400 == 0 {
    let player_positions: Vec<glam::Vec3> =
        self.players.iter().map(|s| s.player.pos).collect();
    crate::spawning::tick_mob_spawning(
        &mut self.ecs,
        &self.world,
        self.world_time,
        &player_positions,
    );
}
```

Server already owns this via `GameServer::tick` (CLAUDE.md Resolved debt: "GameServer runs the 400-tick cycle for hosted games").

On the client side, mob entities arrive via the existing StateUpdate packets. No new transport message needed.

### Acceptance — Phase 1

- Divergence detector: no spawn count mismatch across 20 minutes.
- Axolittle session: mobs still spawn, same feel (not "they spawn on top of me now" or "they never spawn in caves anymore" — exact spec per `spawning::tick_mob_spawning`).
- `./check.sh` green.

### Rollback

One commit per phase. If Axolittle reports regressions, `git revert` and re-plan.

---

## Phase 2 — Falling blocks cutover

Same shape as Phase 1. `falling_blocks::tick_falling_blocks` is already a pure function. Delete the client-side invocation and the client-side mesh-rebuild trigger path that was wired into Task 1c.

Server emits `BlockChange` packets for every cell the function modified. Client receives, rebuilds dirty chunks.

### Acceptance — Phase 2

- Divergence detector silent on block counts.
- Axolittle: drop sand, it falls. Break a supporting block, the pillar collapses. Same feel.
- Sand-column regression test (add one, if not already present): 10 sand blocks stacked on a floating block, break the support, all 10 fall exactly one tick later per block.
- `./check.sh` green.

---

## Phase 3 — Entity physics + ServerPlayer/PlayerSlot unification

This is the high-risk phase. Remote-player physics already runs server-side (Task 1d); this phase extends server-ownership to local players too.

### Changes

1. Single-player's local input packet goes over the in-process transport (client-side emit; server-side `process_input_packet` applies physics). This makes server-authoritative physics the one true physics.
2. Delete `HostedServer` local-player position-trust path (CLAUDE.md bullet 4 — the "BRIDGE in hosted_server.rs:382" lives here).
3. `ServerPlayer` becomes the source of truth for every player state (position, yaw, pitch, inventory, hotbar). `PlayerSlot` on the client is a render-side snapshot only — the save path no longer writes raw `ServerPlayer` fields (CLAUDE.md bullet 2).
4. The save format may need a migration — existing single-player saves have been written via `PlayerSlot`. Either write both for one release cycle, or add a version-bump migration on load. **Prefer migration**: saves are short-lived in alpha, but breaking them silently is cruel. A version tag in `WorldMeta` selects old vs new layout.

### Acceptance — Phase 3

- Existing saves load cleanly (migration works).
- Single-player movement feels identical (Axolittle session: run, jump, fall, swim, sprint).
- Divergence detector silent.
- Split-screen two-player still works (multi-player intents still feed through the in-process transport per player).
- `./check.sh` green. A new integration test: authoritative-physics round-trip.

### Rollback plan

Revert the phase, restore the position-trust path. The client still has the old physics code from Phase 0 (we deleted it in Phase 3); keep the revert commit handy for emergency rollback. Consider gating Phase 3 behind a runtime env var for one release (`AXE_SERVER_AUTHORITATIVE=1`) before hard-flipping.

---

## Phase 4 — Mob AI cutover

Mob AI targeting ("nearest player from all positions") is already on the server path (CLAUDE.md Resolved debt). This phase moves the decision-loop tick to the server too.

Client receives the AI state (target entity id, next-action enum) via state update. No UI-visible change if the server tick rate and render tick rate are aligned.

### Acceptance — Phase 4

- Axolittle: mob AI feels responsive. Mobs target correctly. No visible lag between "mob sees me" and "mob lunges."
- Unit test (introduced in Spec 3 — test-harness): two-player session, mob targets the closer player, switches when positions swap.
- `./check.sh` green.

---

## Phase 5 — Delete the BRIDGE, bump protocol, update specs

- Delete `HostedServer::process_local_input_trusting_position`-style code (whatever name the BRIDGE hangs off in `hosted_server.rs:~382` — grep for "BRIDGE: local players").
- Bump `PROTOCOL_VERSION`. Single-player and multiplayer packets are now identical; old clients would trust fields the new server doesn't send.
- CLAUDE.md: move bullets 1, 2, 4 from "Known technical debt" to "Resolved technical debt".
- Update `docs/spec/01-engine-architecture.md` (client = render-only mirror).
- Update `docs/spec/04-networking.md` — if the BRIDGE notes on `JoinRequestPacket.player_name` reference the dual-sim, update those cross-references too.

### Acceptance — Phase 5

- `./check.sh` + `--smoke` green.
- Full Axolittle regression session: every gameplay subsystem feels the same or better.
- Spec docs describe what the code does.

---

## Global acceptance

- Six phases merged (one branch, six commits — prefer six PRs for Axolittle's phase-by-phase review).
- Divergence detector stays silent across every gameplay session.
- `./check.sh` + `--smoke` green at every phase boundary.
- CLAUDE.md debt bullets 1, 2, 4 resolved.
- No client-side simulation paths remain in `game_loop.rs` (the big refactor's "done" smell).

---

## Non-goals

- **Network latency prediction.** Server-authoritative physics with zero latency (in-process) is the foundation; multiplayer-aware prediction/reconciliation is a separate spec.
- **Rollback/replay.** Now possible (server owns state) but not in scope here.
- **Cross-process single-player.** Staying in-process; OS-level process isolation is a future platform concern.
- **WASM/web target.** In-process channels work the same under WASM. But WebRTC multiplayer transport is a separate spec.
- **Protocol redesign.** The packets Today Are Already Multiplayer-Shaped — we're just routing single-player through them. Don't redesign packets here.

---

## Memory rules that apply

- signet boundary — no Signet-side asks needed; this is engine-internal.
- pretest check — before each Axolittle handoff, verify the phase's acceptance list against real code. The divergence detector helps but doesn't replace eyes.
- CLAUDE.md "Concrete, Not Cards" — Every phase should leave the codebase closer to the target shape, never move sideways. If a phase needs a bridge, mark it with the `BRIDGE` pattern and the trigger for its removal.

---

## What "done" looks like

- Six phases merged, each with an Axolittle-tested PR.
- CLAUDE.md's debt list shrinks by 3 bullets.
- Single-player launches, runs, saves, loads — indistinguishably from today (that's the bar: zero feel regression).
- The engine is now **one codebase with one simulation path**, which unblocks:
  - Real multiplayer (just swap transport).
  - Rollback/replay (server owns state).
  - Deterministic testing (headless HostedServer + scripted inputs — see Spec 3).
  - Cloud dedicated servers (server ECS is already the only source of truth).

This is the single most leveraged piece of engine debt. Killing it pays for itself ten times.

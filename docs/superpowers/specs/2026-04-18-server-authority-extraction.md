# Server-Authority Extraction — Build Spec

**Date**: 2026-04-18
**Status**: Ready to build (holodeck-consensused — spec-purist × minimal × multiplayer-first)
**Authority**: `docs/spec/01-engine-architecture.md` (tick order, 12-crate target), `docs/spec/05-gameplay-systems.md` (PlayerIntent shape, 400-tick mob spawn), CLAUDE.md "Concrete, Not Cards" (BRIDGE resolution)

This spec is self-contained. A fresh session starting here should be able to execute it without re-reading chat history. If something referenced here conflicts with older docs, this spec wins for the three listed BRIDGE items only.

---

## 0. Where We Are

**Committed and working** (as of `127ee67`):
- `GameServer::tick()` at `server.rs:257-299` runs world time, water/leaf decay, mob AI, entity physics, combat, health timers, despawn.
- `HostedServer::run_server_loop` drives `GameServer::tick()` once per 50 ms on a dedicated thread. Single-player uses this path (via `channel_transport`).
- `PlayerIntent` struct at `player_intent.rs` with `forward / backward / left / right / jump / sprint / sneak / toggle_flight`.
- `protocol.rs` has `InputPacket { tick, x, y, z, yaw, pitch, health, held_item, block_changes }` and `StateUpdatePacket { players, block_changes, world_time }`.
- `./check.sh` is the regression gate (cargo + trunk + bundle-size + optional Playwright smoke).

**The three BRIDGE items to fix** (all noted in CLAUDE.md "Known technical debt"):

1. `spawning.rs:5` — `GameState::tick_mob_spawning()` runs client-side only. Called from `game_loop.rs:76-78` every tick, gated on `world_time % 400`. Spawns zombies at night, burns them in sun. Soft cap 80 mobs. Scans within 24-64 blocks of player 0 only.

2. `block_interact.rs:58` — `GameState::tick_falling_blocks()` runs client-side only and calls `self.renderer.upload_chunk_mesh()` directly for every modified chunk. Called from `game_loop.rs:148` every 4 ticks. Scans within 16 blocks of player 0. Displaces water when sand lands on it.

3. `hosted_server.rs:345-389` — `ClientInput` packet processing unpacks `x/y/z/yaw/pitch/health/held_item` and writes them straight to `ServerPlayer.pos` without re-running physics. A client that lies about its position is broadcast verbatim to other clients (subject to distance + rate checks only).

**Not in scope for this spec** (deferred with a documented trigger):
- `ServerPlayer` vs `PlayerSlot` unification (BRIDGE at `server.rs:302-303`). Trigger: when the save path becomes server-authoritative.
- `main.rs`/`game_loop.rs` decomposition below 500 lines. Separate task (#17).
- Chunk streaming to remote clients. Phase 1 multiplayer roadmap item, unblocked by this spec but not implemented here.
- Full anti-cheat (spec 08). Only bare-minimum validation lands here; full spec 08 is separate work.
- Interest management / bitpacked deltas. Phase 2 network optimisation.

---

## 1. Guiding Principles (read before every task)

1. **Spec-aligned, not demo-aligned** — per CLAUDE.md "Concrete, Not Cards". If a shortcut tempts you, mark it `BRIDGE:` with the replacement trigger.
2. **Server is the authority; client is a predictor.** After this spec, no mutation path on the client is legitimate except (a) local-player physics prediction and (b) mesh rebuilds driven by server-originated `BlockChange` records.
3. **Deterministic physics on both sides.** `Player::tick()` must be byte-identical given the same `(world, intent)`. No RNG except seeded deterministic, no wall-clock, no hashmap-iteration-order dependency. This is the non-negotiable foundation for reconciliation.
4. **Protocol is additive.** New wire fields use `#[serde(default)]`. `PROTOCOL_VERSION` bumps per task for discipline, but the decoder accepts one-version-down and degrades cleanly. When all clients are V2-native, V1 fields get deleted (tracked at §7).
5. **Single-player goes through the same code path as multiplayer host.** Spec 01 §1. `HostedServer` is already that path; don't introduce a second.
6. **Each task must be a green commit.** `./check.sh --release --smoke` passes on every commit this spec produces. A task that leaves the build red gets broken smaller.
7. **Crate boundaries visible even in single-crate state.** Each new module is tagged with its eventual spec-01 crate so the split is mechanical when it comes.

---

## 2. Target Tick Order (per `GameServer::tick()`)

Align verbatim with spec 01 §4.1. Each step is a private method on `GameServer`. Landing the extractions in the order below fills in the gaps.

| # | Phase | Landed in | Reads | Writes |
|---|---|---|---|---|
| 1 | `network_recv()` — drain client packets into `pending_inputs[i]: VecDeque<StampedIntent>` | Task 1d | transport queues | `pending_inputs` |
| 2 | `apply_inputs()` — pop intents, validate, stage | Task 1d | `pending_inputs`, `world` (validation only) | `ServerPlayer.intent`, `yaw/pitch`, `last_ack_seq`, `block_action_queue` |
| 3a | `tick_player_physics()` — `Player::tick(intent, ...)` per player | Task 1d | `ServerPlayer.intent`, `world`, `registry` | `ServerPlayer.pos/velocity/on_ground` |
| 3b | `tick_block_updates()` — water spread/retract, leaf decay, `tick_falling_blocks()`, queued `BlockAction`s | Task 1c (falling), Task 1d (actions) | `world`, `registry` | `world`, `dirty_blocks: Vec<BlockChange>` |
| 3c | `tick_entity_ai()` — `mob_ai::tick_mob_ai` | already present | `ecs`, `world`, player positions | mob intents/paths |
| 3d | `tick_entity_physics()` — gravity, collision, step-up for mobs | already present | `ecs`, `world` | entity positions |
| 3e | `tick_combat()` — mob-player pushout, attacks, health timers, despawn dead | already present | `ecs`, `player.pos` | damage events, `dead_entities` |
| 3f | `tick_mob_spawning()` — gated by `world_time % 400 == 0` | Task 1b | `world`, player positions | new mob entities, `spawn_events: Vec<EntitySpawn>` |
| 4 | `tick_worldgen()` — advance chunk queue from player positions | already present | player positions | `world` chunks, `loaded_columns` |
| 5 | `build_state_snapshot()` — compose per-client `StateUpdatePacket` | Task 1b, 1c, 1d (iterative) | all write-outputs | `outbound_snapshots[i]` |
| 6 | `network_send()` — push snapshots | already present | `outbound_snapshots` | transport buffers |
| 7 | `persist_tick()` — autosave every 6000 ticks | already present | world, players | disk/IndexedDB |
| 8 | `metrics_tick()` — tick-time histogram | Task 3e (benchmark) | timing | metrics |

`world_time` increments at the head of 3b, so inputs for tick N act on world-at-N-1 (Quake-style lockstep).

---

## 3. Protocol Changes — Additive, Version-Tagged

No existing field is removed this phase. New fields use `#[serde(default)]`. `PROTOCOL_VERSION` bumps to 2.

### InputPacket (client → server)

```rust
pub struct InputPacket {
    // V1 (keep — read as client hint, replaced by intent in server-authority mode):
    pub tick: u64,
    pub x: f32, pub y: f32, pub z: f32,
    pub yaw: f32, pub pitch: f32,
    pub health: f32,
    pub held_item: u8,
    pub block_changes: Vec<BlockChange>,

    // V2 additions (Task 1d):
    #[serde(default)] pub intent: Option<IntentFields>,
    #[serde(default)] pub client_seq: u64,   // monotonic per-client sequence
}

pub struct IntentFields {
    pub move_forward: bool, pub move_backward: bool,
    pub move_left: bool, pub move_right: bool,
    pub jump: bool, pub sneak: bool, pub sprint: bool,
    pub toggle_flight: bool,
    pub look_yaw: f32, pub look_pitch: f32,   // absolute — no accumulation drift
    pub primary_action: Option<BlockAction>,
    pub secondary_action: Option<BlockAction>,
    pub hotbar_select: Option<u8>,
}
```

When `intent` is `Some`, the server ignores `x/y/z` on that packet and simulates from the intent. When `intent` is `None` (V1 client), the server falls back to trusting position — but V1 is only accepted in single-player (`HostedServer::local_transports`); remote clients MUST send intent. Remote-without-intent = reject + kick.

### StateUpdatePacket (server → client)

```rust
pub struct StateUpdatePacket {
    // V1:
    pub players: Vec<PlayerState>,
    pub block_changes: Vec<BlockChange>,
    pub world_time: u32,

    // V2 additions:
    #[serde(default)] pub tick: u64,
    #[serde(default)] pub last_acked_input: u64,     // per-recipient — highest client_seq the server consumed
    #[serde(default)] pub entity_spawns: Vec<EntitySpawn>,
    #[serde(default)] pub entity_updates: Vec<EntityUpdate>,
    #[serde(default)] pub entity_despawns: Vec<u32>,
}

pub struct EntitySpawn { pub id: u32, pub kind: u8, pub x: f32, pub y: f32, pub z: f32, pub yaw: f32, pub health: u16 }
pub struct EntityUpdate { pub id: u32, pub x: f32, pub y: f32, pub z: f32, pub yaw: f32, pub state: u8 }  // state = AI state for anim
```

### Sizing

With 20 mobs per loaded area, `EntityUpdate` at ~24 bytes each, per-tick delta ≈ 500 B. Well under the 8 KB/s per-client LAN target (spec 04 §6).

### Deferred V2 fields (not in this spec)

- `falling_events: Vec<FallingEvent>` — the multiplayer-first framing proposed this for mid-air transient rendering. Deferred: `BlockChange` for `(src→AIR, dst→block)` pairs already renders correctly via existing dirty-chunk mesh rebuild. Add only if Axolittle reports the visual as jank.
- Per-player HUD delta — current snapshot already carries enough. Add when inventory is multi-slot networked.
- Bitpacked presence masks — Phase 2 optimisation.

---

## 4. Client Reconciliation

Model: **Quake 3 / Overwatch predict-and-replay.** Single strategy, no mode flag.

Client state (new, in `game_loop.rs` or new module `client_prediction.rs`):
```rust
pending_intents: VecDeque<(client_seq: u64, IntentFields)>   // bounded 60 entries = 3 s @ 20 TPS
predicted_history: VecDeque<(client_seq: u64, pos: Vec3, vel: Vec3, on_ground: bool)>
```

Per client tick:
1. Build `IntentFields` from this frame's `PlayerIntent`.
2. Increment `client_seq`. Push `(seq, intent)` into `pending_intents`.
3. Call `Player::tick(world, intent, ...)` locally (prediction). Push predicted state into `predicted_history`.
4. Send `InputPacket { intent: Some(...), client_seq: seq, ... }` to server.

On `StateUpdatePacket` arrival with `last_acked_input = S` and server's authoritative player pos `P`:
1. Drop `pending_intents` entries with `seq <= S`. Drop `predicted_history` entries with `seq <= S`.
2. If `|P - predicted_history_at(S)| < 0.05 blocks`: prediction was correct, no snap. (Log delta for telemetry.)
3. Else: snap local player to `P`. Replay every remaining `pending_intents` entry via `Player::tick()` against local world copy. End state is the new local position.

**Other players and mobs**: interpolation buffer. Keep last two `EntityUpdate`s per entity. Render position at `t - 100 ms` lerp. **Never predict** remote entities.

**Blocks**: server's `BlockChange` is always authoritative. Client applies + marks dirty chunk for re-mesh on the next frame (existing path). Client never optimistically places without a server round-trip confirmation — Task 1d deletes the `pending_block_changes.push` path in `game_loop.rs:~925`. Visual latency mitigation (optional, not in this spec): client can ghost the target block at alpha 0.5, cleared by the next snapshot.

**Determinism invariant**: `Player::tick()` is pinned as deterministic. Add a unit test (Phase 3d) that asserts identical input → identical output across 1000 ticks.

---

## 5. Task Breakdown

### Task 1a — Protocol extension only (no behaviour change)

**Goal**: Land the additive fields on `InputPacket` and `StateUpdatePacket` so subsequent tasks can fill them. No code reads or writes the new fields yet.

**Files**:
- `game/engine/src/protocol.rs` — add `IntentFields`, `EntitySpawn`, `EntityUpdate`, `intent`, `client_seq`, `tick`, `last_acked_input`, `entity_spawns`, `entity_updates`, `entity_despawns`. All `#[serde(default)]`. Bump `PROTOCOL_VERSION` const from 1 to 2.
- `game/engine/src/hosted_server.rs` — no handler changes; just confirm `safe_deserialize` still accepts V1 packets cleanly.

**Scope**: Additive only. `cargo build && trunk build && ./check.sh --smoke` green.

**Acceptance**:
- V1 InputPacket bincode-encoded → V2 decoder produces `intent: None, client_seq: 0`.
- V2 InputPacket → V1 decoder ignores trailing bytes (or V1 decoder stays in deploy anyway — we own both ends).
- `PROTOCOL_VERSION` reported correctly in handshake.

---

### Task 1b — Mob spawning → `GameServer`

**Goal**: Delete `GameState::tick_mob_spawning`. Replace with `GameServer::tick_mob_spawning` called at step 3f of the tick order.

**Files**:
- `game/engine/src/spawning.rs` — move `tick_mob_spawning` body to `server.rs`. Swap `self.players[0].pos` for a loop over `self.players` (spec 05:1234 says "around each player"). Swap `PlayerSlot` access for `ServerPlayer`. Delete the `impl GameState` wrapper.
- `game/engine/src/server.rs` — add `fn tick_mob_spawning(&mut self)`, call it from `tick()` when `world_time % 400 == 0`. Accumulate new mob spawns into `spawn_events: Vec<EntitySpawn>` on `GameServer`.
- `game/engine/src/game_loop.rs:76-78` — delete the `self.tick_mob_spawning()` call.
- `game/engine/src/server.rs::build_state_snapshot` — drain `spawn_events` into `StateUpdatePacket.entity_spawns`. Also emit `EntityUpdate` for existing mobs whose position/health changed this tick (compare against `last_broadcast_state: HashMap<EntityId, (Vec3, f32)>` on `GameServer`). Emit `entity_despawns` when a mob is removed.
- `game/engine/src/game_loop.rs` — on `StateUpdatePacket` receive, apply `entity_spawns / updates / despawns` to `self.ecs` (render-only mirror). Remote entities use interpolation buffer (§4); single-player snaps immediately since lag is zero.

**Scope**:
- Sun-burn damage stays coupled to spawning for this task (same function, same file as current `spawning.rs:14-42`). Move both.
- Don't unify `ServerPlayer` and `PlayerSlot`. Deferred (BRIDGE at server.rs:302-303).

**Acceptance**:
- Native single-player: zombies still spawn at night within 24-64 blocks of player. Zombies still burn at dawn.
- Mob count soft cap (80) still enforced.
- `game_loop.rs` no longer mutates `self.ecs` for spawns — only reacts to packets.
- `./check.sh --release --smoke` green.
- Manual: with website running + native engine open, spawn a world, wait for night, count ≥1 zombie. Commit the screenshot in `docs/test-sheets/2026-04-18-task1b-mob-spawn.md` as evidence.

---

### Task 1c — Falling blocks → `GameServer`, decouple renderer

**Goal**: Delete `GameState::tick_falling_blocks`. Replace with `GameServer::tick_falling_blocks` at step 3b. Never call `renderer.upload_chunk_mesh` from simulation.

**Files**:
- `game/engine/src/block_interact.rs:58` — move `tick_falling_blocks` body to `server.rs`. The body mutates `world` via `set_block`, emits water removal via `water.remove_source`, and currently calls `renderer.upload_chunk_mesh`. Server version: mutate `world`, emit `BlockChange` records for each `(src→AIR, dst→block)` pair into `dirty_blocks: Vec<BlockChange>`. **Zero renderer access on the server side.**
- `game/engine/src/server.rs` — new `fn tick_falling_blocks(&mut self)`. Runs every 4 ticks (5 Hz) matching current cadence. Scans within 16 blocks of every player (loop, not just player 0 — spec 05 pattern).
- `game/engine/src/game_loop.rs:148-171` — delete the `self.tick_falling_blocks()` call.
- Client-side mesh rebuild: already triggered by `BlockChange` application in the existing packet handler. Verify this path is wired; if not, wire it via the existing `dirty_chunks: HashSet<(i32,i32,i32)>` accumulator at `game_loop.rs:154-170`.
- `game/engine/src/server.rs::build_state_snapshot` — drain `dirty_blocks` into `StateUpdatePacket.block_changes` (already exists).

**Scope**:
- **Do not** introduce `FallingEvent` packet (deferred). A fallen block appears on the client as two `BlockChange`s, same frame. Looks "snap" rather than "drop" — acceptable for now; Axolittle feedback will drive whether we need the mid-air transient later.
- **Do not** introduce `ChunkMeshManager` as a separate module. The existing dirty-chunk loop in `game_loop.rs` is the decoupling point; it stays.
- Batching cap: if `dirty_blocks` exceeds 2048 in one tick, spill remaining into `overflow_dirty: VecDeque<BlockChange>` on `GameServer` for the next tick. Realistic max with 16-block radius: < 500 per tick; cap is safety net.

**Acceptance**:
- Native single-player: place sand above air, it falls. Sand landing on water removes the water source.
- `grep renderer block_interact.rs` returns only the client-local `rebuild_chunk_at` path (which is allowed — that's the mesh rebuild).
- `./check.sh --release --smoke` green.
- Manual: test sheet `docs/test-sheets/2026-04-18-task1c-falling-blocks.md`.

---

### Task 1d — PlayerIntent over network + server-driven physics

**Highest-risk task.** Replaces client-authoritative position with intent + reconciliation.

**Files**:
- `game/engine/src/game_loop.rs` — at the end of per-frame local player tick, build `IntentFields` from the current frame's `PlayerIntent`. Increment `client_seq`. Push to `pending_intents` + `predicted_history`. Include `intent: Some(...)` in the outgoing `InputPacket`.
- `game/engine/src/hosted_server.rs:345-389` — on `InputPacket` receive: if `intent.is_some()`, do NOT overwrite `sp.player.pos`. Instead push `(client_seq, intent)` into `sp.pending_inputs`. If `intent.is_none()` AND transport is local (single-player), fall back to current position-trust path (BRIDGE: document trigger = "when single-player physics goes full server-driven"). If `intent.is_none()` AND transport is remote, kick the client with `Kick { reason: "protocol v2 required" }`.
- `game/engine/src/server.rs` — new `fn apply_inputs(&mut self)` at step 2 of tick. Pops one intent per player, validates (§6), assigns to `ServerPlayer.intent`. New `fn tick_player_physics(&mut self)` at step 3a calls `Player::tick(world, registry, sp.intent, ...)` per player. Sets `sp.last_ack_seq` to the consumed `client_seq`.
- `game/engine/src/server.rs::build_state_snapshot` — per-client `last_acked_input = sp.last_ack_seq`.
- `game/engine/src/game_loop.rs` — on `StateUpdatePacket` receive: reconcile local player position per §4. Drop acked entries from `pending_intents` + `predicted_history`.

**Anti-cheat (bare-minimum validation in `apply_inputs`)**:
- Speed cap: intent's implied displacement ≤ `SPRINT_SPEED × 1.5` per tick. If over, clamp.
- Look sanity: yaw/pitch finite. If NaN, reject intent, reuse previous.
- Primary/secondary action reach: target block distance ≤ `REACH = 5.0` from server's current `sp.pos`. If over, drop the action.
- Hotbar select: `0..=8`. Clamp.

**Scope**:
- Keep V1 position path alive for single-player (`intent: None` fallback). Remote clients are intent-only. Rationale: native single-player works today; breaking it in this task multiplies risk. BRIDGE to be resolved in a follow-up that deletes V1 entirely.
- Don't delete `pending_block_changes.push` path in `game_loop.rs` yet — that's a client-local queue for the break-animation. Block actions now travel as intent fields; the queue becomes render-only (just drives the break progress bar). Retag as BRIDGE with trigger = "when break-animation drives off server ack".
- Don't delete V1 `InputPacket` fields (x/y/z). Mark `#[deprecated]` via comment only (Rust `#[deprecated]` causes warnings we can't gate yet).

**Acceptance**:
- Native single-player: player moves normally, camera looks normally, block break + place still work. No perceptible change.
- Two native instances with one hosting: second player's movement + block interactions visible to host correctly. (Manual test, no automation.)
- `Player::tick` determinism test: `cargo test player_tick_determinism` passes — same `(world, intent)` → same output across 1000 iterations.
- `./check.sh --release --smoke` green.
- Test sheet `docs/test-sheets/2026-04-18-task1d-player-intent.md` includes the determinism test output + a 30-second recorded movement session.

---

## 6. Crate-Boundary Hints (eventual 12-crate split)

Tag each item's future home. We stay single-crate today; tags exist so the later split is mechanical.

| Current file/module | Future crate |
|---|---|
| `player_intent.rs` | `genesis_core` |
| `physics.rs`, `world.rs`, `chunk.rs`, `block.rs`, `registry.rs`, `ecs` | `genesis_core` |
| `protocol.rs` (split into `input.rs`, `snapshot.rs`, `block.rs`, `entity.rs`, `handshake.rs` when we split crates) | `genesis_protocol` |
| `server.rs` (`GameServer` + `tick` + per-phase methods) | `genesis_server` |
| `spawning.rs` post-move | `genesis_server` |
| `block_interact.rs::tick_falling_blocks` post-move | `genesis_server` |
| `block_interact.rs::rebuild_chunk_at` (client-only, stays) | `genesis_client` |
| `game_loop.rs` prediction/reconciliation | `genesis_client` |
| `hosted_server.rs`, `network.rs`, `transport.rs` | `genesis_net` |

Concretely today: leave the file layout alone; when a method is extracted, place it in the `impl GameServer` block in `server.rs`, not in a new file.

---

## 7. Follow-up BRIDGE markers (created by this spec, resolved later)

1. `hosted_server.rs` fallback to V1 position for single-player when `intent: None`. **Trigger**: when single-player physics is fully server-driven (1d + followup); at that point delete V1 fields from `InputPacket`.
2. `InputPacket.{x, y, z, health}` fields remain post-1d. **Trigger**: one-release deprecation window after all alpha testers confirmed on V2; then delete.
3. `game_loop.rs::pending_block_changes` becomes render-only. **Trigger**: when break-animation drives off server ack instead of client queue.
4. `ServerPlayer` vs `PlayerSlot` duplication at `server.rs:302-303`. **Trigger**: when save path becomes server-authoritative.

---

## 8. Testing Rhythm

Each task follows `docs/workflow/daily-build-test.md`:

1. **Build**: implement behind the task's scope. `./check.sh --release --smoke` must pass before commit.
2. **Test sheet**: generate `docs/test-sheets/2026-04-18-task{1a,1b,1c,1d}-{name}.md` with checklist + questions for Axolittle. Include smoke-test screenshot evidence.
3. **Commit**: `feat(server-auth): task {N} — {title}` with motivating context in the body.
4. **Spec update**: if spec 01/05/02 drifted (schema, field shape, order), update in same commit.

Regression tests added by this spec:
- `cargo test player_tick_determinism` — Phase 3d carries, Task 1d requires.
- `cargo test protocol_v1_to_v2_roundtrip` — Task 1a requires.
- `cargo test mob_spawn_timing` — Task 1b adds (verify 400-tick cadence + night gating).

---

## 9. What NOT to do

- **Don't delete V1 `InputPacket` fields this phase.** Additive protocol is what keeps alpha testers on PWA working through the rollout.
- **Don't introduce `FallingEvent` / `ChunkMeshManager` / typed channel transport this phase.** Each is defensible but expands scope. Added only when a concrete regression or feedback demands them.
- **Don't fold `ServerPlayer` into `PlayerSlot` (or vice versa).** BRIDGE at `server.rs:302`; its own refactor.
- **Don't make `tick_mob_spawning` or `tick_falling_blocks` return a list that's piped through the client — the snapshot path is the only client-bound data path.**
- **Don't let the client's `GameState::tick()` call `Player::tick()` independently when intent is in flight.** The prediction step IS a `Player::tick` call, but it goes through the new `client_prediction` flow, not the old `game_loop.rs:136-141` direct call. The direct call deletes in Task 1d.
- **Don't expose server's `ecs` to the client.** Client's `ecs` is a render-only mirror keyed by `EntityId`, not a shared data structure. Spec-purist's rule: crossing that boundary is how single-crate forever happens.
- **Don't skip client `client_seq` stamping.** Tempting to "replay everything since last snapshot" — variable jitter means you'd replay already-acked intents and double-apply.
- **Don't put `tick_worldgen` before `tick_physics`.** Spec 01:549-555. Physics-before-worldgen prevents per-tick races where a freshly-gen'd chunk spawns inside a mid-fall player.

---

## 10. Done Definition

All four tasks (1a, 1b, 1c, 1d) complete. Measured against local dev environment:

- Three BRIDGE items from CLAUDE.md "Known technical debt" deleted (not merely moved behind another BRIDGE).
- Native single-player plays as before: movement, block interaction, mob spawning, falling sand all work. No perceptible regression in feel.
- WASM build green, PWA loads, auth flow unchanged, local-first IndexedDB saves unchanged.
- `./check.sh --release --smoke` green on every commit.
- `cargo test` green: `player_tick_determinism`, `protocol_v1_to_v2_roundtrip`, `mob_spawn_timing`.
- Spec 01, 05, 02 updated where behaviour shifted.
- CLAUDE.md "Known technical debt" section updated: the three BRIDGE items moved to "Resolved technical debt".
- Test sheets with Axolittle-ready checklists for each task live under `docs/test-sheets/`.

After that: Phase 2 (data-driven refactors) becomes unblocked — mob definitions, recipes, loot tables move to data files without wrestling with the simulation-leak problems this spec fixes.

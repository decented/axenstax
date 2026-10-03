# Engine test-harness expansion

**Status**: DELIVERED (2026-04-20, originally branch `feat/engine-test-harness`, merged to main; commits `313530b` spec pretest update, `467c8fe` Layer A +41 tests, `1cebf05` Layers B+C harness + integration suites +17). Test count went 31 → 89 across the three commits.
**Date**: 2026-04-20
**Branch**: create a new branch off `main` (clean). Suggested: `feat/engine-test-harness`.
**Session**: Fresh — implementer should treat this doc as the only brief.

---

## TL;DR

The engine has **31 unit tests** today (`#[test]` in `protocol.rs`, `block.rs`, `server.rs`, `mob.rs`, `screen.rs`). `./check.sh` already runs `cargo test --bin axenstax-engine`, so the gate exists — it's just thin. For the refactors ahead (Spec 1 engine-Signet-auth, Spec 2 single-player-HostedServer), the existing coverage is not enough to catch regressions early.

This spec builds out two orthogonal layers of test infrastructure:

1. **Unit tests on pure functions** — `spawning`, `falling_blocks`, `combat`, `mob_ai`, `save`, `protocol` edge cases. These are the extract-and-test payoffs from Tasks 1b/1c/1d. Low risk, high dividend.
2. **Headless integration harness** — a `HostedServer` + `Transport` fixture that a test can spin up, feed scripted `ClientInput` packets into, and assert on the resulting state. This is the keystone: Spec 2 (HostedServer routing) needs this to validate every phase's cut-over. Spec 1 (Signet auth) needs it for end-to-end join-handshake verification.

Total size: ~800 lines of test code, 2–3 new test helpers, zero production-code changes. Completely orthogonal to gameplay; can land any time; pays forever.

---

## Context pointers

- Existing tests: `grep -rn "#\[test\]" game/engine/src/` — 31 today. Patterns live in `protocol.rs:334`, `block.rs:263`, `server.rs:460`, `mob.rs:117`, `screen.rs:68`.
- `./check.sh` runs `cargo test --bin axenstax-engine --quiet`. The gate is already green — this spec adds test *content*, not infrastructure.
- Pure functions already extracted (CLAUDE.md Resolved debt):
  - `spawning::tick_mob_spawning` (Task 1b)
  - `falling_blocks::tick_falling_blocks` (Task 1c)
  - `PlayerIntent::from_input_packet`, `Player::tick` (Task 1d)
- Pure serialization: `protocol::safe_deserialize`, `protocol::compress_chunk` / `decompress_chunk`, `save::serialize_inventory_raw`, `chunk.rs` palette compression.

---

## Scope

Three layers, each independently shippable.

| Layer | What | Files | Est. tests added |
|------:|------|-------|:----------------:|
| A | Pure-function unit tests (spawning, falling_blocks, combat, save, mob_ai, protocol edge cases) | `src/spawning.rs`, `src/falling_blocks.rs`, `src/combat.rs`, `src/save.rs`, `src/mob_ai.rs`, `src/protocol.rs`, `src/chunk.rs` — append `#[cfg(test)] mod tests` blocks | ~40 |
| B | Headless integration harness: `TestHost` that wraps `HostedServer` + in-process transports + determinism controls (fixed seed, fixed tick clock) | new `src/test_harness.rs` (gated by `#[cfg(test)]`, declared at crate root in `main.rs`) | harness only |
| C | Integration test suites using the harness: join-handshake, mob spawn, falling blocks, player physics, multi-player interactions | new `src/test_integration/{handshake,mobs,blocks,physics}.rs`, all `#[cfg(test)]` | ~15 |

**Total**: ~55 new tests, ~800 lines. Zero production-code changes (layer A, C).

### Structural reality (spec update, 2026-04-20 pre-build check)

The engine crate is **bin-only** (`src/main.rs` declares `mod foo;` for every module; no `src/lib.rs`). Rust workspace-style integration tests under `tests/*.rs` require a library target, so the original spec layout (`tests/common/mod.rs`, `tests/handshake.rs`, …) would not compile without a lib refactor touching every `mod` declaration and `crate::` path in the engine.

**Resolution without the lib refactor**: Layers B and C live as `#[cfg(test)]` modules inside the existing bin crate. `cargo test --bin axenstax-engine` (which `check.sh` already runs) compiles the whole bin with `cfg(test)` enabled and finds every `#[test]` function — including those in Layer C's integration modules. Functionally identical coverage; no structural refactor.

Concrete layout:
- `src/test_harness.rs` — `TestHost`, `TestClient`, `TestConfig`. Declared via `#[cfg(test)] mod test_harness;` in `main.rs`.
- `src/test_integration/mod.rs` — `pub mod handshake; pub mod mobs; pub mod blocks; pub mod physics;`. Declared via `#[cfg(test)] mod test_integration;` in `main.rs`.
- Each sub-module carries its own `#[test]` functions.

If/when the engine grows a `lib.rs` (likely needed anyway for the PWA/wasm entry split, or for an eventual `genesis_server` crate split), Layers B and C move out to `tests/*.rs` with no test-code changes beyond swapping `use crate::...` for `use axenstax_engine::...`.

---

## Layer A — Pure-function unit tests

Each existing pure function gets a `#[cfg(test)] mod tests` block with:
- **One determinism test** — run the function twice with identical inputs, assert identical outputs.
- **One correctness test per rule** — e.g., spawning respects biome rules; falling_blocks only moves sand/gravel.
- **Edge cases** — empty inputs, bounded caps, underflow/overflow.

### Target: `src/spawning.rs` (`tick_mob_spawning`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinism() {
        // Same world + player positions + tick → same entities added.
        let mut ecs_a = hecs::World::new();
        let mut ecs_b = hecs::World::new();
        let world = fixture_world();
        let players = vec![glam::Vec3::new(0.0, 65.0, 0.0)];
        tick_mob_spawning(&mut ecs_a, &world, 400, &players);
        tick_mob_spawning(&mut ecs_b, &world, 400, &players);
        assert_eq!(ecs_a.len(), ecs_b.len());
    }

    #[test]
    fn respects_mob_cap() {
        // Running the fn 1000× shouldn't exceed the per-tick cap.
    }

    #[test]
    fn no_spawn_too_close_to_player() {
        // Known gameplay rule — add a test that locks it.
    }

    #[test]
    fn peaceful_mode_spawns_no_hostiles() {
        // Ensure difficulty=peaceful short-circuits.
    }
}

fn fixture_world() -> World { /* small deterministic world */ }
```

Target 6–8 tests.

### Target: `src/falling_blocks.rs` (`tick_falling_blocks`)

```rust
#[test] fn sand_falls_one_tick() { /* place sand at y=10 on y=5 floor, tick, expect it at y=9 */ }
#[test] fn sand_stops_on_floor() { /* place on floor, tick N times, expect no move */ }
#[test] fn gravel_behaves_like_sand() { /* same rule, different block */ }
#[test] fn stone_does_not_fall() { /* control */ }
#[test] fn column_collapses_when_support_broken() { /* 10-block stack, remove support, expect all move */ }
#[test] fn dependency_ordering_is_bottom_up() { /* lower blocks move first so upper ones don't clobber */ }
#[test] fn returns_correct_block_changes() { /* Vec<BlockChange> matches actual mutations */ }
```

Target 7 tests.

### Target: `src/combat.rs`

- Damage calculation for known weapon/mob pairs.
- Critical hit chance (deterministic seed).
- Invulnerability frames enforcement.
- Knockback direction math.

Target 5 tests.

### Target: `src/save.rs`

- Round-trip: `WorldMeta` → bytes → `WorldMeta` matches.
- Legacy-load: known-good old-format bytes (serialised from the pre-Item-enum era) deserialise into current structs with expected defaults.
- Empty world serialises without error.
- Corrupt trailing bytes are rejected cleanly.

Target 4 tests.

### Target: `src/mob_ai.rs`

- Nearest-player selection across a 3-player position set.
- Target swap when the previous target leaves range.
- Flee behaviour on low health.
- No-target behaviour in peaceful mode.

Target 4 tests.

### Target: `src/protocol.rs` — extend existing suite

- Malformed length prefix → `safe_deserialize` returns Err without panic.
- Truncated payload → Err, not panic.
- Compression round-trip for a chunk larger than 4 KiB.
- Decompression of crafted bomb input → bounded output (already gated by `MAX_CHUNK_DECOMPRESSED`).

Target 4 new tests.

### Target: `src/chunk.rs`

- Palette compression: 4096 identical blocks → near-minimal encoding.
- Palette compression: 4096 random blocks → round-trip correct.
- Edge case: empty palette behaves as all-air.

Target 3 tests.

### Acceptance — Layer A

- `cargo test` count goes from 31 → ~65.
- All new tests pass on first push.
- `./check.sh` green.
- Running tests take under 5 seconds on the host.

---

## Layer B — Headless integration harness

A reusable fixture other tests build on. Lives under `game/engine/src/test_harness.rs`, gated by `#[cfg(test)]` in `main.rs` (see "Structural reality" above — the crate is bin-only).

### File: `game/engine/src/test_harness.rs`

```rust
//! Headless test harness: HostedServer + in-process transports + deterministic clock.

use axenstax_engine::{HostedServer, /* ... */};

/// Fixture: starts a HostedServer with a fixed seed and tick-controlled clock.
/// Exposes helpers to drive client packets and assert on server state.
pub struct TestHost {
    server: HostedServer,
    clients: Vec<TestClient>,
}

pub struct TestClient {
    pub id: usize,
    // Write side: push ClientInput / JoinRequest / etc. into the server.
    // Read side: drain ServerToClient packets the server emits.
}

impl TestHost {
    /// Construct with a small deterministic world. Default: 1 client, difficulty normal, survival mode.
    pub fn start_with(config: TestConfig) -> Self { /* ... */ }

    /// Advance the server by N ticks. Client transports drain outputs.
    pub fn tick(&mut self, n: u32) { /* ... */ }

    /// Deliver a JoinRequest for a client slot.
    pub fn send_join(&mut self, client: usize, req: protocol::JoinRequestPacket) { /* ... */ }

    /// Deliver a raw ClientInput packet.
    pub fn send_input(&mut self, client: usize, input: protocol::InputPacket) { /* ... */ }

    /// Snapshot of the server's ECS for assertions.
    pub fn ecs(&self) -> &hecs::World { /* ... */ }

    /// Drain all ServerToClient packets queued for a client.
    pub fn drain_outbound(&mut self, client: usize) -> Vec<Vec<u8>> { /* ... */ }
}

pub struct TestConfig {
    pub seed: u32,
    pub clients: usize,
    pub difficulty: Difficulty,
    pub is_creative: bool,
}

impl Default for TestConfig { /* seed=42, clients=1, normal, survival */ }
```

### Determinism controls

- Fixed `seed` for world gen, mob spawn RNG, etc.
- Fixed tick clock (no `Instant::now()` in test paths — wire a clock trait or use `tick` argument consistently).
- Fixed spawn coordinates.

If the production code currently pulls `Instant::now()` deep inside a simulation function, that's the refactor tax this spec pays: wire the clock in as a parameter. Track every such site as a small commit during this layer.

### In-process transport

This spec **depends on** Spec 2 Phase 0 (the in-process `Transport` pair) landing first — or it builds a test-only shim independently. Recommendation: build the test shim independently (~80 lines) so this spec doesn't block on Spec 2. When Spec 2 Phase 0 lands, the test shim can be replaced with the production one.

### Acceptance — Layer B

- A smoke test (`test_integration::smoke::spins_up_and_ticks` or similar, under the bin's `cfg(test)` tree) spins up a `TestHost`, runs 200 ticks, asserts the server is still alive.
- `cargo test --bin axenstax-engine` green and includes the new test.
- Harness runs in under 1 second for a 200-tick session.

---

## Layer C — Integration test suites

Using the Layer B harness, add one test file per subsystem under `src/test_integration/`.

### `src/test_integration/handshake.rs`

- Valid JoinRequest accepted → JoinAccept delivered with correct `player_index`.
- JoinRequest with wrong `protocol_version` → JoinReject.
- Post-handshake `ClientInput` accepted; pre-handshake `ClientInput` silently dropped.
- Double JoinRequest on the same slot is no-op after the first (Task 1d debt fix).
- After Spec 1 lands: auth-event rejection cases (bad sig, wrong challenge, fromNP=true).

Target 6 tests (more after Spec 1).

### `src/test_integration/mobs.rs`

- Mob spawns within the 400-tick cycle when player is in a valid biome.
- Peaceful difficulty → no hostile spawns across 1000 ticks.
- Mob cap respected: server won't exceed the configured cap even across many cycles.
- Two-player config: mob AI targets the nearer player.

Target 4 tests.

### `src/test_integration/blocks.rs`

- Falling blocks integration: place sand on air above ground, tick, observe terminal state.
- Block-break → drop entity appears.
- Block-place from creative hotbar with unlimited stack.

Target 4 tests.

### `src/test_integration/physics.rs`

- Remote player physics: send InputPacket with movement, assert position updated.
- Anti-cheat seed: horizontal speed cap (>32 m/s) is clipped.
- Falling off the world: y < -64, expect death event.

Target 3 tests.

### Acceptance — Layer C

- 17 integration tests added.
- All pass.
- `cargo test` total: ~82.
- `./check.sh` green.

---

## Global acceptance

- `cargo test --bin axenstax-engine` — ~80 tests pass (unit + integration combined, since integration tests also live inside the bin per the "Structural reality" note).
- `./check.sh` runs it; must be green.
- Total test-suite runtime: under 15 seconds on the host.
- `CLAUDE.md` gets a new "Testing" section (or "Verification" expansion) documenting:
  - Where pure-function unit tests live (`#[cfg(test)] mod tests` inside each source module).
  - Where integration tests live (`src/test_integration/*.rs`, `#[cfg(test)]`).
  - How to add a new integration test (extend `TestHost` in `src/test_harness.rs` or add a new file under `src/test_integration/`).

---

## Non-goals

- **Coverage tooling.** `cargo-tarpaulin` or `grcov` would be great but adds CI complexity. Defer.
- **Fuzz testing.** `cargo-fuzz` for `safe_deserialize` is appealing but separate spec.
- **Benchmark suite.** `criterion` for perf gates is future work.
- **Property testing.** `proptest` or `quickcheck` would light up pure functions but adds a dep. If a test can be written by hand in 10 lines, write it by hand. Revisit when test count > 100.
- **WASM-side tests.** The engine compiles to WASM; a headless-browser test harness is a whole separate project.
- **Mutation testing.** `cargo-mutants` is interesting but way post-alpha.

---

## Memory rules that apply

- pretest check — tests verify code; verifying tests verify the right thing is on the implementer.
- signet boundary — Layer C integration tests for auth (post-Spec-1) must not propose Signet-side changes.
- CLAUDE.md "Concrete, Not Cards" — don't write placeholder tests that look impressive but assert nothing (e.g., `assert!(result.is_some())` when the function always returns `Some`). Tests should encode real invariants.

---

## What "done" looks like

- Three layers landed (can ship as one PR or three — implementer's call).
- `cargo test` count went from 31 → ~80.
- The headless harness is reusable by Spec 1 and Spec 2 implementers — they don't build their own.
- `./check.sh` still under 30 seconds.
- CLAUDE.md has a "Testing" note pointing at `src/test_integration/` (and the per-module `#[cfg(test)] mod tests` blocks) as the onboarding for new tests. The `tests/common/` layout from the original spec sketch was replaced by in-bin `#[cfg(test)]` modules per the "Structural reality" note above.
- Future refactors (starting with Spec 2) have a safety net: break a subsystem, a test fires, you know before Axolittle does.

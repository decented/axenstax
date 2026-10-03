# Quad split-screen — 3 and 4 player local viewports

**Status**: READY TO BUILD. Phases 2–5 are autonomous solo work. Phase 6 (Axolittle 4-player playtest) blocks on his time.
**Date**: 2026-05-16
**Branch**: `feat/quad-split-screen` once started; off `main`.
**Session**: Fresh — implementer should treat this doc as the only brief.
**Trigger**: Audit on 2026-05-16 surfaced that `compute_screen_layout` panics any path that would seat Player 3 or Player 4, despite the surrounding architecture (Vec<PlayerSlot>, Vec<GamepadState>, Vec<PlayerGpuResources>, save format, pause-menu leave buttons) being N-shaped for 1-4 already. Phase 1 split-screen design (2026-04-01) deferred 3-4 player layouts to a Phase 2 that never got specced. This spec is that Phase 2.

---

## TL;DR

The engine is already 1-4 player-shaped end-to-end **except** for two enforcement points: the layout solver caps at 2 viewports, and the "press A to join" rule caps at < 2 players. Lift those two ceilings, choose a 3-player and a 4-player layout, smoke-test the existing per-viewport HUD/crafting code at the smaller viewport sizes that result, and the engine seats up to four local players on one screen.

Scope is small (~600 lines incl. tests). The work is gated `#[cfg(not(target_arch = "wasm32"))]` like its parent — web stays 1-player local for now, per pwa priority and the existing native-only gating in `chunk_stream.rs:153` and `game_loop.rs:1028`. Lifting the WASM gate is a separate, much larger piece of work (gamepad-on-web, multi-input story) and is out of scope here.

---

## Why this lives here

- **Architecture is already there.** `GameState.players` is `Vec<PlayerSlot>` (`main.rs:230` doc says "split-screen ready: Vec holds 1-4 PlayerSlots"). Gamepad system tracks N controllers (`gamepad.rs:5`). HostedServer allocates N channel transports per `num_local_players` (`hosted_server.rs:145`). Save format is `Vec<PlayerSaveData>`. Pause menu's leave-buttons loop is `for pi in 0..num_players` (`menu.rs:1339`). The two missing pieces are the layout solver and the join cap.
- **Axolittle has asked for it implicitly.** AxeNStax targets couch co-op; 4 friends round one TV is the canonical use case. Confirmed scope on 2026-05-16 chat ("split screen needs to handle up to 4 players, with a quad screen").
- **Cross-game lift, per shared infra strategy.** `compute_screen_layout()` is engine-generic — once it knows 1/2/3/4, every Decented game that uses the same screen abstraction (other games on the same primitives) inherits 1-4 local players for free. The 3-player asymmetric layout in particular is a reusable primitive — any couch-co-op game that bolts onto the engine gets it.
- **Lights up the gamepad-only Player 3/4 path.** Today the join code in `game_loop.rs:1029` is `if self.players.len() < 2` — Player 3 and Player 4 never even get a chance to try. The fix is one-line but unblocks an entire input axis.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/screen.rs` — `compute_screen_layout(num_local_players, w, h)`. Currently matches `1`, `2`, and `_ => warn + fall back to 1`. This spec adds the `3` and `4` arms.
- `game/engine/src/game_loop.rs:1022-1069` — "press A on any unassigned controller to join" rule. Hardcoded `self.players.len() < 2`. Lift to `< 4`. Join toast string `"Player 2 joined!"` becomes `format!("Player {} joined!", self.players.len())`.
- `game/engine/src/chunk_stream.rs:153-180` — save-restore pre-allocation. Already loops `while self.players.len() < player_saves.len()`, so it handles up to 4 saved players already. No change needed (verify with a 4-player save round-trip test).
- `game/engine/src/menu.rs:1330-1355` — pause menu leave buttons. Already loops `for pi in 0..num_players`. No change needed (verify visually with 3 + 4 players).
- `game/engine/src/hud_ui.rs` — HUD drawing already takes a `viewport: ViewportRect` parameter (per the 2026-04-01 design). No structural change; verify the hotbar / hearts / block-name overlays still fit and render legibly at 960×540 (1080p quad) and 480×270 (sub-1080p quad).
- `game/engine/src/craft_ui.rs` — crafting UI takes a viewport parameter too. Verify the 3×3 grid + result slot still fits inside a quad-quadrant viewport at small resolutions; if not, scale down or define a minimum quad-viewport size that we require.
- `game/engine/src/renderer.rs` — per-viewport render loop in `render()` already loops `for screen in screens`. Per-viewport crosshair, per-player GPU resources, depth-buffer scissoring — all already viewport-rect driven. No change expected.
- `game/engine/src/gamepad.rs` — `Vec<GamepadState>` already. Per-controller routing in the intent collector needs to extend from 2-player table (in `game_loop.rs`) to 4-player table.
- `game/engine/src/hosted_server.rs:145, 161-181` — `num_local_players` already drives Vec sizing for local transports + handshake_done + disconnected vectors. No change.

### Native-only gating (preserve as-is)

The whole feature stays under `#[cfg(not(target_arch = "wasm32"))]`. The existing gates we leave in place:

- `chunk_stream.rs:153` (save pre-allocate loop) — already native-only.
- `game_loop.rs:1028` (controller-driven join) — already native-only.

WASM keeps its single-player local clamp. Lifting that is **out of scope for this spec** — it depends on a gamepad-on-web story, a multi-cursor story (or fully gamepad-only WASM mode), and the input-routing rework that follows.

### Related specs

- `docs/superpowers/specs/2026-04-01-split-screen-design.md` — the parent design. §1 names Phase 2 = "3-4 player layouts". §3 ("Input System") describes the per-player gamepad routing pattern this spec extends. §9 ("Phase 1 Scope") lists the explicitly-not-built items, of which "3-4 player layouts (Phase 2)" is one.
- `docs/research/2026-04-01-multiscreen-layout.md:37-38` — quad-split pixel budgets (1080p = 960×540 per quadrant; 4K = 960×540 per quadrant; sub-1080p = 480×270). Phase 5's HUD legibility check uses these as test resolutions.
- `docs/spec/05-gameplay-systems.md §1` — Movement / camera / FOV. No changes here, just a sanity check that FOV-clamped first-person view still works in a 480×270 viewport.
- `docs/spec/04-networking.md §1.7` — local-player join sequence. The 4-player local case is a sequence of N JoinRequests through channel transports; the spec already permits this, no protocol change.

### Memory pointers

- uk english naming — use UK English for any user-facing strings (toasts, menu labels).
- autonomy to playtest boundary — Phases 2-5 are autonomous; Phase 6 is the playtest gate.
- pwa priority / alpha launch posture — PWA target is solo-only; native is where this spec lands.
- shared infra strategy — keep `compute_screen_layout` engine-generic, no AxeNStax-specific assumptions.

### What does NOT exist yet (and this spec does NOT need)

- **Web gamepad support.** Out of scope; native-only feature.
- **Multi-window / multi-monitor split.** That is the *other* Phase 2 of the 2026-04-01 design (dual-screen). This spec is the one-window-many-viewports half. Multi-window is a separate spec.
- **N-mouse routing.** Mouse is always Player 0's, exactly as today. Players 2-4 are gamepad-only. No change.
- **Per-screen audio routing.** Phase 3 of the parent design. Out of scope.
- **Spectator content.** `ScreenContent::Spectator(...)` and `ScreenContent::Companion(...)` variants stay un-added; the enum stays single-variant (`LocalPlayer`). Future PiP / spectator work is a separate spec.

---

## Scope

| # | Phase | Files | Est. lines | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-16-quad-split-screen.md` | ~600 | ✓ |
| 2 | 3 + 4 player layouts in `compute_screen_layout` + unit tests | `screen.rs` | ~140 | ✓ |
| 3 | Raise join cap from 2 to 4; generalise join toast + log | `game_loop.rs` | ~30 | ✓ |
| 4 | Per-player input routing extended from 2 → 4 (keyboard always P0; gamepads 0-2 → P2-P4) | `game_loop.rs` (intent-collect block), `gamepad.rs` (if any per-player intent helper changes) | ~120 | ✓ |
| 5 | HUD + crafting legibility verification at 960×540 + 480×270; integration test for 4-player save round-trip | `hud_ui.rs` and/or `craft_ui.rs` (small fitting tweaks if needed), `src/test_integration/save_load.rs` | ~150 | ✓ |
| 6 | Axolittle 4-player playtest (couch co-op feel; layout-choice validation; layout-switch when a player leaves mid-game) | n/a | 0 | ✗ blocked |

**Total**: ~600 lines spec + code + tests. Phase 6 is the playtest gate.

Phases 2-5 are independent enough to land in any order, but the natural order is 2 → 3 → 4 → 5. Phase 2 can land on its own as a no-op (no caller produces `num_local_players > 2` until Phase 3 lifts the cap), so it's the safe-first step.

---

## Phase 1 — This spec

You're reading it. ✓ Move on.

---

## Phase 2 — 3 and 4 player layouts

### Goal

Replace the `_ => log::warn!("Only 1-2 players supported, ...")` arm in `compute_screen_layout` with real layouts for 3 and 4 local players. Cover the rounding cases (odd widths/heights).

### Layout choice

**4 players → 2×2 quad.** Top-left = P1, top-right = P2, bottom-left = P3, bottom-right = P4. This is the canonical couch co-op layout (Halo, Mario Kart, GoldenEye lineage); easy to reason about, equal screen real estate, no player gets a vertical-strip disadvantage.

**3 players → 2×2 quad with bottom-right empty.** Same grid math as 4-player; the missing quadrant shows a flat dark fill (no viewport rendered there) with the text "Waiting for Player 4 — press A on any controller to join" centred. This:

- Keeps a single layout-math primitive (`grid_2x2`) shared between 3- and 4-player.
- Makes the seat-up-for-P4 affordance obvious — the empty quadrant is the prompt.
- Avoids the asymmetric "big top, two below" 3-player layout, which is more code, makes one player visually privileged, and is less reusable.
- Matches the parent design's hint at quad as the target shape, not a 2+1 stack.

(Phase 6 playtest may surface a preference for 2+1 with P1 on top, in which case a follow-up tweak swaps the 3-player branch. Cheap to revisit.)

### Implementation

```rust
// game/engine/src/screen.rs

pub fn compute_screen_layout(
    num_local_players: usize,
    window_width: u32,
    window_height: u32,
) -> Vec<Screen> {
    match num_local_players {
        1 => vec![Screen {
            viewport: ViewportRect::full(window_width, window_height),
            content: ScreenContent::LocalPlayer(0),
        }],
        2 => {
            // existing 2-player code, unchanged
        }
        3 | 4 => grid_2x2(num_local_players, window_width, window_height),
        _ => {
            log::warn!("Only 1-4 players supported, got {num_local_players}");
            compute_screen_layout(1, window_width, window_height)
        }
    }
}

fn grid_2x2(n: usize, w: u32, h: u32) -> Vec<Screen> {
    let half_w = w / 2;
    let half_h = h / 2;
    let right_w = w - half_w;   // soaks up odd-pixel remainder
    let bottom_h = h - half_h;  // soaks up odd-pixel remainder

    let quadrants = [
        // (player_idx, x, y, width, height)
        (0, 0,      0,      half_w, half_h),
        (1, half_w, 0,      right_w, half_h),
        (2, 0,      half_h, half_w, bottom_h),
        (3, half_w, half_h, right_w, bottom_h),
    ];

    quadrants.iter()
        .take(n)
        .map(|&(pidx, x, y, width, height)| Screen {
            viewport: ViewportRect { x, y, width, height },
            content: ScreenContent::LocalPlayer(pidx),
        })
        .collect()
}
```

### Empty-quadrant rendering (3-player case)

The `take(n)` call yields 3 `Screen`s in the 3-player case — the bottom-right quadrant simply doesn't appear in the iter. The renderer's main loop is already `for screen in screens` (one render pass per Screen), so an absent quadrant is just unrendered pixels.

Those pixels carry whatever was last cleared into them by the first viewport's `LoadOp::Clear`. To make the empty slot look intentional and serve as a join prompt:

- After the per-viewport render loop completes, if `screens.len() == 3`, draw a single egui-rendered overlay into the bottom-right quadrant — flat dark fill (`Color32::from_rgba(20, 20, 25, 255)`) with centred text "Waiting for Player 4 — press A on any controller to join" (style consistent with existing toast).
- Implement as a new function `draw_empty_quadrant_prompt(ctx, viewport)` in `hud_ui.rs`, called from `game_loop.rs` after the existing HUD pass. Single egui `Area` at the quadrant's centred position.

### Tests

In `screen.rs::tests`:

- `three_player_2x2_with_empty_bottom_right` — assert 3 screens, viewports match top-left / top-right / bottom-left of a 1920×1080 grid (960/960/540/540), and that player indices are 0/1/2.
- `four_player_2x2` — assert 4 screens, viewports cover the whole window without overlap, player indices are 0/1/2/3.
- `four_player_odd_dimensions_no_gap` — width=1921 height=1081. Assert `screens[0].width + screens[1].width == 1921` and `screens[0].height + screens[2].height == 1081`. No row of pixels is missed.
- `four_player_aspect_ratio` — assert each viewport has aspect ≈ 16/9 / 1 ≈ matches a normal aspect (e.g. ~0.888 for 960×1080 in 2-player, ~1.78 for 960×540 in 4-player; just verify the value is finite + matches half-width-over-half-height).
- `five_player_falls_back_to_single` — assert it logs warn and returns 1-player layout (matching the existing fallback contract).

### Acceptance

- All five new unit tests pass.
- `check.sh` clean (clippy + build + test).
- Calling `compute_screen_layout(3, 1920, 1080)` returns 3 Screen objects with correct rects; calling with 4 returns 4.
- No callers in the bin yet pass >2 (that lands in Phase 3). Phase 2 is a no-op at runtime, safe to merge alone.

---

## Phase 3 — Raise the join cap

### Goal

Allow Player 3 and Player 4 to seat themselves via the "press A on any unconnected controller" rule.

### Changes

In `game/engine/src/game_loop.rs`, around lines 1022-1069 (the "Rule 2: Press A to join" block):

- Change `if self.players.len() < 2` to `if self.players.len() < 4`.
- The `let p2_spawn` / `let mut p2` variable names are inaccurate for P3/P4. Rename:
  - `p2_spawn` → `new_spawn`
  - `p2` → `new_slot`
  - Spawn offset: instead of always `+3.0` X, use `+3.0 * (new_player_index as f32)` so successive joiners don't pile on top of each other. Cleaner alternative: spawn each new player at `P0.pos + Vec3::new(3.0 * (idx as f32 + 1.0), 0.0, 0.0)` — fans them out along +X.
- `viewport_aspect = 0.5` is a 2-player half-width aspect estimate. Replace the per-player aspect set with one that reads off the new screens computed below (the existing post-recompute loop already does this — just rely on it and start with a placeholder 1.0).
- The join toast becomes `format!("Player {} joined!", new_player_index + 1)`.
- The log line: `log::info!("Player {} joined via controller {}!", new_player_index + 1, idx);`.

The HostedServer-aware path (channel transport allocation, handshake_done vec, etc.) is **not** invoked here — single-player adds a PlayerSlot to GameState directly, exactly as the 2-player case does today. The HostedServer cap-on-construction (Phase 2 spec's `max_remote_players + num_local_players`) covers the multiplayer-host case separately and is already correctly N-shaped.

### Tests

Add to `src/test_integration/save_load.rs` (or a new `test_integration/local_join.rs`):

- `join_p3_then_p4_increases_slot_count` — start at 1 player, manually push two more PlayerSlots, assert `players.len() == 3` then 4, assert `compute_screen_layout(...).len()` matches.
- `joining_a_fifth_player_is_a_no_op` — start at 4, attempt to add a fifth, assert nothing changes. (Phase 2's `compute_screen_layout(5, ...)` fallback path is already tested in Phase 2; this test is at the join-rule layer.)

### Acceptance

- Native build, run a world: hold A on controllers 2, 3, 4 in turn — each one joins, layout switches at each step (1 → 2-stripe → 2×2-with-empty → full 2×2).
- The leave button for each player in the pause menu already works (`menu.rs:1339` loop is N-aware) — clicking it removes the player and the layout reshrinks. Verify visually.

---

## Phase 4 — Input routing for 3 and 4 players

### Goal

Extend the per-player intent collector so P3 and P4 each have their own gamepad.

### Current state

The 2026-04-01 design's Section 3 sketched a 1-2 routing table. The current code lives in `game_loop.rs` (intent collection per tick) — keyboard+mouse always feeds P0, the first non-P0 gamepad feeds P1 (now also P2 in this spec's numbering — sorry, see *Note on indices* below).

### Note on indices

Throughout this spec, "Player N" (1-indexed, user-facing) maps to `self.players[N-1]` (0-indexed). Player 1 = `players[0]` = keyboard+mouse owner. Players 2-4 = `players[1..4]` = gamepad-only.

### Routing table

| Players | P1 input | P2 input | P3 input | P4 input |
|---------|----------|----------|----------|----------|
| 1 (no gp) | KB+M | — | — | — |
| 1 (gp) | KB+M + Gamepad[0] (merged) | — | — | — |
| 2 (1 gp) | KB+M | Gamepad[0] | — | — |
| 2 (2 gp) | KB+M + Gamepad[0] (merged) | Gamepad[1] | — | — |
| 3 | KB+M | Gamepad[0] | Gamepad[1] | — |
| 4 | KB+M | Gamepad[0] | Gamepad[1] | Gamepad[2] |

Mouse never routes to P2/P3/P4. Keyboard never routes to P2/P3/P4. P1 always has KB+M; if there are at least 2 gamepads connected, P1 *also* gets Gamepad[0]'s inputs merged in (matches the 2-player case today — Gamepad[0] is the controller P1 was using before P2 joined, and they keep it).

The `p1_gamepad` field on GameState (`main.rs:236` — "If Player 1 is using a gamepad (controller-only / console mode)") already encodes the P1-on-controller case; reuse it as the "P1 owns Gamepad[0]" flag and shift gamepad ownership for P2/P3/P4 accordingly.

### Implementation

In `game_loop.rs`, the existing tick code that builds the per-player `PlayerIntent` should generalise:

```rust
let mut intents: Vec<PlayerIntent> = Vec::with_capacity(self.players.len());
for player_idx in 0..self.players.len() {
    let intent = match player_idx {
        0 => {
            // P1: KB+M, plus Gamepad[0] if P1 owns it (p1_gamepad) OR
            // if there's only one gamepad total and no one else owns it.
            let mut intent = self.input.to_intent();
            if let Some(gp_idx) = self.p1_gamepad {
                if let Some(gp_intent) = self.gamepad.to_intent(gp_idx, dt) {
                    intent.merge(gp_intent);
                }
            }
            intent
        }
        n => {
            // P2/P3/P4: gamepad only. Map player index → controller index.
            // If P1 owns Gamepad[0], P2 owns Gamepad[1], P3 owns Gamepad[2], etc.
            // If P1 is on KB+M only, P2 owns Gamepad[0], P3 owns Gamepad[1], P4 owns Gamepad[2].
            let base = if self.p1_gamepad.is_some() { 1 } else { 0 };
            let gp_idx = base + (n - 1);
            self.gamepad.to_intent(gp_idx, dt).unwrap_or_default()
        }
    };
    intents.push(intent);
}
```

`PlayerIntent::default()` (no movement, no look, no actions) covers the "controller disconnected this tick" case — the player's character stands still rather than crashing. The existing disconnection toast (`gamepad.rs`'s `disconnected_this_frame`) keeps surfacing to the HUD.

### Tests

- `intent_routing_4_players_3_gamepads` — set up GameState with 4 PlayerSlots and 3 connected gamepads (no `p1_gamepad`), simulate one tick, assert `intents[0]` reflects KB inputs, `intents[1..4]` reflect Gamepad[0..3] inputs.
- `intent_routing_4_players_p1_on_controller` — same but with `p1_gamepad = Some(0)`. Assert P1's intent merges KB+Gamepad[0], P2-P4 use Gamepad[1..4]. Needs 4 gamepads connected.
- `intent_routing_disconnected_controller_yields_default` — P3's gamepad disconnects; assert `intents[2] == PlayerIntent::default()` and the other three players' intents are unchanged.

### Acceptance

- All three tests pass.
- Manual: plug 3 controllers, host a 4-player game (P1 = KB+M), each player can move + look + interact independently.

---

## Phase 5 — HUD/crafting legibility at quad resolutions

### Goal

Make sure the existing per-viewport HUD and crafting UI remain usable at quad-quadrant sizes.

### What to check

| Window | Quad quadrant | HUD elements |
|--------|---------------|--------------|
| 1920×1080 (1080p TV) | 960×540 | Hotbar fits; hearts fit; block-name centred; crosshair centred. |
| 3840×2160 (4K TV) | 1920×1080 | Same as today's single-player full-screen — trivially fine. |
| 1280×720 (720p) | 640×360 | Worst case. Hotbar likely needs to scale; crafting 3×3 may need a min-size guard. |
| 1024×600 (Pi-class display) | 512×300 | Below playable spec — define the floor here and warn. |

### Implementation strategy

1. **Test at all four resolutions.** Boot the game in windowed mode at each size, seat 4 local players, take screenshots, eyeball the HUDs. Use the existing per-viewport HUD draw — no structural change.
2. **If hotbar or hearts overflow the viewport at 640×360**, add a per-element scale factor that drops to ~0.75× when `viewport.width < 720`. Single-place change in `hud_ui.rs` (compute scale at draw entry, multiply all coords). Keep the >720px path pixel-identical to today.
3. **If crafting UI doesn't fit at 640×360**, the cleanest fix is a `min_viewport_for_crafting: u32 = 600` constant — opening the crafting UI in a viewport smaller than that pops a toast "Window too small for crafting in 4-player — make the window bigger" and refuses. Cheaper than a bespoke responsive crafting layout. Phase 6 playtest validates the floor.
4. **Document the recommended minimum.** Add a single line to `docs/spec/05-gameplay-systems.md §[player visuals or HUD section]` noting the 4-player recommended floor is 1280×720.

### Save round-trip test

In `src/test_integration/save_load.rs`, add:

```rust
#[test]
fn four_player_save_round_trip() {
    // Build a world with 4 PlayerSlots at distinct positions/inventories.
    // Save it. Load it. Assert all 4 players restored with correct state.
    // Assert screens layout matches 4-player quad.
}
```

`chunk_stream.rs:153-180`'s pre-allocate loop is already N-aware (uses `player_saves.len()`), so this should pass without code changes — it's a verification test, not a fix.

### Acceptance

- Manual screenshots at 1920×1080, 3840×2160, 1280×720 (and 1024×600 if warning lands) — HUD readable, crosshair correct, hotbar selection visible.
- `four_player_save_round_trip` integration test green.
- `check.sh` clean.

---

## Phase 6 — Axolittle 4-player playtest

### Goal

Validate the layout choice (2×2 quad), the join flow (press A), the in-game feel of a 4-player session, and the layout switch when a player leaves mid-game.

### Pre-playtest setup

- 4 USB or Bluetooth controllers paired with the playtest machine.
- World seeded with enough stuff to do (chests, a couple of crafting tables, monsters around to fight together).
- Recording set up (screencap) so a layout regression caught visually can be reproduced.

### What to look for

- Does the 3-player "empty quadrant" prompt read clearly, or is it confusing? (Alt: revisit 2+1 layout for 3.)
- When a controller disconnects mid-game, what's the experience? Toast appears, layout reflows on next join/leave. Smooth or jarring?
- Are 480×270 viewports (sub-1080p quad) playable? If not, the recommended minimum from Phase 5 needs a stronger enforcement (e.g. refuse the 4-player join below a window-size threshold).
- HUD legibility — hotbar, hearts, block name, debug overlay.
- Crafting in a quadrant — usable or annoying?
- Subjective: does it feel good?

### Out of playtest scope

- LAN multiplayer with split-screen on one side (4 local + N remote). That's Spec 2's territory once HostedServer routing is unified.
- Saving + reloading a 4-player world (covered by Phase 5's automated round-trip test).

### Outcome

Tuning notes from the session go into a follow-up commit. Likely tweaks: layout choice, viewport-size floor, HUD scale factor.

---

## Open questions / future work

- **3-player layout: 2×2-with-empty vs 2+1 stack.** This spec proposes 2×2-with-empty. Phase 6 validates or vetoes.
- **Per-screen audio for couch co-op.** Out of scope. Lifted to Phase 3 of the parent design.
- **WASM quad split.** Not in scope here. Needs gamepad-on-web first (separate spec).
- **Multi-window dual screen.** The *other* Phase 2 of the parent design. Separate spec.
- **Multiplayer + split-screen mix** (e.g. 2 local + 2 remote in a HostedServer). The architecture supports it (HostedServer is already N-shaped), but it's gated behind Spec 2's HostedServer-routing-for-singleplayer landing. Track as a follow-on after Spec 2.

---

## Memory rule check

- uk english naming — user-facing strings ("Waiting for Player 4 — press A on any controller to join", "Player N joined!", crafting toast) all UK English. ✓
- autonomy to playtest boundary — Phases 2-5 are solo-verifiable, Phase 6 is the explicit playtest gate. ✓
- pretest check — before claiming Phase 5 done, run `check.sh` + spot-check the HUDs at all four target resolutions. ✓
- pwa priority / alpha launch posture — feature stays native-only; the PWA's single-player-local clamp is preserved. ✓
- shared infra strategy — `compute_screen_layout`, `grid_2x2`, and the empty-quadrant prompt stay engine-generic. No AxeNStax-specific strings or assumptions in `screen.rs` or the HUD layout primitives. ✓

---

## Out of scope (explicitly)

- WASM split-screen of any kind.
- Multi-window / multi-monitor.
- Spectator / companion `ScreenContent` variants.
- Per-screen audio routing.
- 5+ player layouts (not a thing on one TV; remote players go through the network).
- Asymmetric layouts (2+1, 1+3, picture-in-picture).
- LAN multiplayer + local split-screen mix (track as follow-on to Spec 2).

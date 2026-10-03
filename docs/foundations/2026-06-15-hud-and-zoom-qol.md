# HUD & zoom QoL pass — debug readout, vital-stats overlays, hold-to-zoom

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-alpha-qol-building-blocks`, goal `2026-06-16-alpha-qol-and-building-blocks`). All three phases shipped: P1 debug HUD (facing/biome/light/FPS + native frame timing + Settings checkbox), P2 vital counts (free-slot + arrow badge; saturation deferred — no food-model value; durability bar already shipped), P3 hold-to-zoom (C key + touch Z button, transient FOV bypassing the clamp, settings slider). `check.sh` green (2590 tests). Feel/visual tuning = Axolittle playtest. Bundles backlog **#1 (status/debug HUD)** + **#4 (vital-stats HUD)** + **#5 (zoom)** from `docs/research/2026-06-15-native-bake-in-feature-backlog.md`.
**Date**: 2026-06-15
**Branch (when built)**: TBD (`qol/hud-zoom`).
**Owner decisions captured (2026-06-15)**: 4-spec packaging (this is spec A); **buttons-first / discoverable** controls for the kid audience — zoom is the one inherently held action, default key **C** (OptiFine convention) + a touch button; the debug readout keeps **F3** but gains a settings checkbox so it's discoverable. Build-now-able: single-player / PWA-Chromium alpha, no multiplayer gate.

---

## TL;DR

Three small, high-everyday-value QoL items that all **surface data the engine already has** (or already partly draws), bundled because they ship in one session and two of them share `hud_ui.rs::draw_hud`:

1. **Debug HUD (#1)** — extend the existing F3 overlay (`hud_ui.rs:653 draw_debug_overlay`, which today shows Mode/XYZ/Time/Target/Perf) with **facing direction**, **biome**, **light level under the cursor**, and a real **FPS** number. Replaces what players reach for F3 for.
2. **Vital-stats overlays (#4)** — AppleSkin + Inventory HUD+ behaviours: a **saturation outline** on the hunger bar, **per-item durability** on damaged held/hotbar tools, and **free-slot + arrow counts**. The armour-points badge already shipped (`hud_ui.rs:158`, Spec 28e); the potion/effects strip is **deferred** (no status-effect system exists yet — verified).
3. **Zoom (#5)** — hold-to-zoom: a transient narrow render FOV (~20°) applied directly to `camera.fov_y`, bypassing the 60–100° settings clamp, restored on release.

Reference behaviour (researched 2026-06-15): F3 debug screen; [AppleSkin](https://github.com/squeek502/AppleSkin) (saturation = yellow outline on the hunger drumsticks, shrinking right-to-left); Inventory HUD+ (durability/free-slot/arrow surfacing); OptiFine zoom (default key C, hold to narrow FOV).

---

## Why this lives here

- Per pwa priority + alpha launch posture: alpha is live and these are the cheap "make it feel finished" wins — the backlog ranks #1 as the highest value-per-hour on the whole board.
- Per the 2026-05-21/05-30 playtest lessons baked into `hud_ui.rs` (creative-mode HUD clutter; kids miss non-obvious affordances): keep the HUD clean and the new affordances discoverable. Debug stays F3 but gains a settings checkbox; vital overlays are passive (no interaction to discover); zoom gets a touch button.
- Per shared infra strategy: nothing here hard-codes AxeNStax — generic engine HUD/camera polish that lifts to any game on the engine.
- Per uk english naming: UK English throughout ("armour", "colour").

---

## The real seam (grounded)

`game/engine/src/hud_ui.rs`:
```text
:119  pub fn draw_hud(ctx, viewport, player_index, inventory, selected_slot,
                      registry, egui_integration, current_hp, max_hp,
                      hunger, max_hunger, armour_points, is_creative,
                      show_debug, player_pos, world_time, flying,
                      target_block, perf: PerfSamples)
:146    draw_hearts / draw_hunger          (survival only)
:149    draw_armour_readout                (Spec 28e — armour points badge, SHIPPED)
:150    draw_block_name
:153    if show_debug { draw_debug_overlay(...) }
:653  fn draw_debug_overlay(...) → Mode / XYZ / Time HH:MM / Target / Perf line
```
- `draw_hud` is called once per player per frame (`game_loop.rs:~10803`).
- `PerfSamples` (frame/tick ms, draw_calls, culled) is **populated WASM-only today** — native FPS needs a frame-time sample wired in (small).
- Camera FOV: `camera.rs:314 fov_y` (default 70, `:357`), `effective_fov_y()` (`:427`) adds the third-person pitch curve, `projection_matrix()` (`:513`). Settings FOV is clamped `FOV_MIN 60 / FOV_MAX 100` (`graphics_settings.rs:120-121`, clamp `:330`) — **zoom must not route through that clamp**; it sets the render FOV transiently.
- Input: `input.rs` exposes held/edge key state; new binds are registered in `key_pressed()` (`:114`). Settings persist via `graphics_settings.rs::{load,save,clamp}` (4-step add: struct field → presets → clamp → `sync_graphics_to_engine`).
- Biome lookup: `biome.rs`; light: `lighting.rs` (sample at `target_block`); facing from player `yaw`.

---

## Scope (phased)

### Phase 1 — Debug HUD (#1)
- Extend `draw_hud` + `draw_debug_overlay` to also show:
  - **Facing**: yaw → compass (`N/NE/E/…`) + degrees, e.g. `Facing: SW (218°)`.
  - **Biome**: name at player position (from `biome.rs`).
  - **Light**: light level at the `target_block` (from `lighting.rs`), e.g. `Light: 7`.
  - **FPS**: a number (derive from `perf.frame_ms_mean` when present; otherwise wire a lightweight native frame-time EMA so native isn't blank).
- Add a `show_debug_hud: bool` to `GraphicsSettings` (default **off**) so the overlay is **discoverable in Settings** as well as togglable with **F3** (F3 unchanged).
- **Out of scope:** the F7 spawn-proof/light-level *world overlay* — that is backlog **#8**, a separate spec; this is only the cursor read-out.

### Phase 2 — Vital-stats overlays (#4)
- **Saturation outline** on the hunger bar (`draw_hunger`), AppleSkin-style (yellow outline, shrinks right-to-left). **Dependency:** requires the food model to track a saturation value; if `PlayerCombat` doesn't yet, add an internal `saturation: f32` first (cheap) — confirm at build time, and if it can't be done cleanly, ship the rest of Phase 2 and defer saturation.
- **Durability**: a thin durability bar / numeric on damaged tools in the hotbar + a held-item read-out (tools already carry durability — `item.rs`).
- **Counts**: free inventory-slot count and total arrow count as a compact badge near the hotbar.
- **Deferred (in-spec note):** the potion/status-effect countdown strip — **no status-effect system exists** (verified: grep of `combat.rs`/`item.rs`/`player_slot.rs` for `status_effect|StatusEffect|potion` is empty). Add when effects ship.

### Phase 3 — Zoom (#5)
- Hold **C** → set the render FOV to a zoom value (`ZOOM_FOV ≈ 20°`, or `base_fov / 3.5`), bypassing the settings clamp; restore the player's configured FOV on release. Render-only transient (same shape as the pitch-curve offset).
- **Touch**: a zoom button on the touch HUD (`touch_input.rs`).
- **Settings**: `zoom_fov: f32` (clamped to a sane min, e.g. 15–45°) and an optional `smooth_zoom: bool`. Gamepad bind: TBD (note only — R3 is sneak per gamepad ui navigation shipped; pick a free combo at build time).
- Mouse sensitivity should scale with the zoom factor so aiming stays controllable (optional polish).

---

## Acceptance criteria

- **P1:** With F3 on (or the Settings checkbox enabled), the overlay shows Mode/XYZ/Time/Target/Perf **plus** Facing (compass+°), Biome, Light-at-target, and a non-zero FPS on **both** native and WASM. Unit test: yaw→compass mapping (8 sectors) and FPS-from-frame-ms derivation are pure functions with tests.
- **P2:** Hunger bar shows a saturation outline that depletes (or saturation explicitly deferred with a note); a damaged tool shows a durability indicator; free-slot and arrow counts render and update. No new clutter in Creative (mirrors the existing `is_creative` suppression at `hud_ui.rs:144`).
- **P3:** Holding C narrows the FOV smoothly and releasing restores the exact configured FOV; zoom FOV is unaffected by the 60–100° settings clamp; a touch zoom button works on the PWA. Unit test: zoom apply/restore leaves `fov_y` exactly at the configured value.
- `./check.sh` green (clippy + build + `cargo test --bin axenstax-engine` + trunk build + bundle-size gate).

## Memory-rule check
- **Concrete, not cards** (CLAUDE.md): all three extend existing systems through their real seams — no bridge code, no parallel HUD. Settings additions follow the established 4-step `GraphicsSettings` pattern.
- **Spec maintenance**: on build, update Spec 03 (Rendering/egui UI) for the HUD additions and Spec 05 (Gameplay/HUD) for vital-stats; record the zoom FOV constant + the saturation-model decision.
- **No build authorised** beyond this queue entry — graduates when the owner says "build the HUD/zoom pass" (or "add #1/#4/#5").

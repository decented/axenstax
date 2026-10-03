# GPU Performance & Graphics Settings — make the web app fast, and tunable on weak hardware

**Status: ✅ DELIVERED** (Spec 39) — merged to main 2026-06-03 (`graphics_settings.rs`; PR #102); playtest-gated. Reconciled 2026-06-19.
**Created: 2026-06-02**
**Target: PWA/WASM first (priority), native second — both ship from the same code**

---

## The Goal (one sentence)

Make the AxeNStax web client render efficiently enough that it feels as smooth as
Java-edition Minecraft on the same machine, and give the player a **Graphics
settings panel** with quality dials (and Low/Medium/High/Ultra presets) so the
game stays playable on weak GPUs.

## Why (the trigger)

Axolittle reports that AxeNStax in the browser is **noticeably laggier and less
enjoyable** than his Java Minecraft world on the same laptop, and that Minecraft
"feels richer." Two separable problems hide inside that one observation:

1. **Raw efficiency.** We are doing a large amount of unnecessary per-frame GPU and
   CPU work (see findings below). Fixing this makes the game faster for *everyone*,
   on every machine, with no visual loss.
2. **No headroom valve.** There is currently **no way to reduce graphics load**.
   Render distance, resolution, FOV, vsync — all hardcoded constants. A weak
   machine has no escape hatch; it either runs the one fixed quality level or it
   doesn't. Minecraft's perceived smoothness is partly *because* a kid can drop
   render distance to 6 and turn off fancy graphics.

This spec addresses both. The "richness/detail" gap (Minecraft has more blocks,
mobs, biomes) is **out of scope** — that's content, tracked under Spec 28. This
spec is purely about *frame rate, smoothness, and tunability*.

## Desired Outcome (acceptance — measurable)

When this is done:

- **A Graphics settings panel** is reachable from the in-game pause menu, with both
  one-click **presets** (Potato / Low / Medium / High / Ultra) and **individual
  dials** the player can fine-tune.
- Settings **persist** per device (localStorage on WASM, a `settings.json` on
  native) and apply immediately — no restart, no world reload.
- **Render distance, render scale (internal resolution), FOV, mouse sensitivity,
  vsync/frame-cap, and fog** are all live dials.
- On a low-end integrated GPU (the target: an everyday school/home laptop), the
  **Low preset holds a smooth frame rate** where the current build stutters, and
  the **F3 perf overlay** (already exists) shows the improvement in numbers.
- **No frame hitch** when placing/breaking a single block or walking into new
  terrain at the Low/Medium presets (the meshing-on-main-thread stalls are gone or
  bounded).
- The efficiency work (frustum culling, buffer reuse) is **invisible** — same
  picture, fewer wasted draw calls and allocations — verified by the F3 draw-call
  count dropping substantially when looking at a wall.
- `check.sh` green; new pure helpers (frustum test, settings serde, preset
  mapping) are unit-tested.

---

## Deep-Dive Findings (current state, 2026-06-02)

A full read of the render path produced this map. **File:line references are exact**
so the build session doesn't have to re-discover them.

### How a frame is drawn today

- **Render is fully decoupled from the 20 TPS sim** and runs as fast as the OS/browser
  allows. `game_loop.rs:8645` calls `self.window.request_redraw()` **unconditionally**
  every frame; `main.rs:1156` uses `ControlFlow::Poll`. On **native this spins the CPU
  at 100%** redrawing even when nothing moved. On WASM the browser's `requestAnimationFrame`
  caps it to vsync, but we still rebuild and resubmit the whole frame every tick.
- **Present mode is `PresentMode::AutoVsync`** hardcoded (`renderer.rs:285`, also the
  headless path at `renderer.rs:751`). No runtime toggle, no frame cap.
- The per-frame render pass sequence is in `Renderer::render` (`renderer.rs:1145`) →
  `render_world_viewport` (`renderer.rs:2065`): clear → opaque chunks → instanced plants
  → water → entities → avatars → wireframes → viewmodel → crosshair → egui → submit/present.

### The big cost drivers (ranked)

1. **No frustum culling.** `render_world_viewport` iterates **every loaded chunk mesh
   unconditionally** (`renderer.rs:2141-2145`), and again for water (`:2217`) and plants
   (`:2182`). At `RENDER_DISTANCE = 10` that's up to ~2,646 sub-chunk slots; with ~50%
   non-empty that's ~1,300 draw calls per frame, of which **~70% are outside the camera
   frustum** (behind the player, underground, off-screen) and get fully clipped by the GPU
   after we paid to submit them. A frustum test is a cheap CPU-side AABB-vs-planes check.
   `camera.rs:64` computes the view-projection matrix but never extracts frustum planes;
   **no frustum struct exists anywhere**. Spec 03 §5.2 already describes the Gribb-Hartmann
   approach — it was specced and never built.

2. **Main-thread synchronous meshing → frame hitches.** All `build_chunk_meshes`
   (`mesh.rs:188`) runs inline on the render thread. A single block edit rebuilds the
   chunk + up to 6 neighbours (`block_interact.rs:17`); a **lighting change rebuilds a
   3×3×3 = up to 27 chunks** (`block_interact.rs:59`); loading one new column meshes up to
   30 sub-chunks (`chunk_stream.rs:99-117`). On WASM this **blocks the browser thread** —
   this is the stutter Axolittle feels when placing a torch or exploring. Spec 03 §2.7
   describes a worker pool / Web Workers; **neither is implemented.**

3. **Per-frame GPU buffer reallocation.** Entity, avatar, viewmodel, wireframe, ghost,
   and **crosshair** vertex buffers are all created fresh via `device.create_buffer_init`
   **every frame** instead of reused with `queue.write_buffer`. The crosshair (a 12-vertex
   constant quad) is reallocated inside the hot path at `renderer.rs:2474`; entities at
   `renderer.rs:1018`; avatars `:1041`; viewmodel `:1085`/`:1110`; highlight `:957`; ghost
   `:939`. These buffers lack `COPY_DST`, so they can't be written in place — they're
   allocated and abandoned each frame, churning the GPU allocator (worse on WASM/WebGPU).

4. **Per-frame CPU entity vertex rebuild.** `build_entity_model_vertices` and friends
   (`game_loop.rs:5866-5876`) walk the **entire ECS every frame** and allocate fresh
   `Vec<Vertex>`, even when nothing moved.

5. **Water mesh has no greedy merging.** `build_water_mesh` (`mesh.rs:482`) emits one quad
   per water face — a flat 16×16 lake surface is 256 quads where greedy merging gives 1.
   Water is common; this is real overdraw.

6. **No mipmaps, no anisotropic filtering.** Block texture array uses `mip_level_count: 1`
   + nearest filtering (`renderer.rs:332-386`). Causes shimmer/aliasing at distance and
   denies the GPU a cheaper LOD sample path.

7. **Memory premium.** Vertices are **44 bytes** (`mesh.rs:19`) vs the 24-byte packed
   format Spec 03 §2.4 targets; indices are **u32** everywhere (`renderer.rs:2143`) where
   u16 suffices post-merge. ~80% vertex + 2× index memory premium — matters most on weak
   GPUs with little VRAM.

8. **No LOD** for distant chunks (Spec 03 §5.4 unimplemented) — every chunk meshes at full
   detail regardless of distance.

### Bugs / gaps spotted along the way

- **`RENDER_DISTANCE` is defined twice** — `main.rs:212` and `server.rs:23` — as separate
  `const i32`. They will drift. Making it a runtime setting must unify the source of truth.
- **Fog distances are shader literals disconnected from render distance.** `shader.wgsl:114-115`
  hardcodes `fog_start = 128`, `fog_end = 160`. It only *coincidentally* matches
  `RENDER_DISTANCE × 16 = 160`. **The moment render distance becomes a dial, fog will no
  longer track it** and players will see hard chunk load/unload edges (pop-in). Fog must
  become a uniform derived from the live render distance. (Water shader uses yet another
  pair, `:252`.)
- `MAX_CHUNK_Y = 5` (96-block world height) is also duplicated (`main.rs:215`, `world.rs:13`)
  — not in scope to change, but note it if touching constants.

### What infrastructure exists to build on

- **No settings system exists at all** — no settings struct, no preferences file, no
  options screen. This is greenfield.
- The UI is **all egui** via `EguiIntegration` (`egui_integration.rs`). The natural home
  for a Graphics screen is the **pause menu**, `draw_pause_menu` (`menu.rs:1550`), which
  already follows an on/off-panel pattern (`draw_skin_panel`). Dispatch is in
  `game_loop.rs:2295-2465` via integer return codes (`PAUSE_RESUME`, etc.) — add a
  `PAUSE_OPEN_SETTINGS` code + a `draw_settings_panel` following the skin-panel template.
- **Persistence patterns to copy:** WASM cosmetics already use `localStorage`
  (`cloud.js:161-216`, key `axenstax_cosmetic_<pubkey>`); the `wasm_bindgen` extern bridge
  pattern is in `wasm_save.rs:37-98`. Native uses serde-JSON for `WorldMeta`
  (`save.rs:1184`). Graphics settings are **per-device, not per-world** → a
  `localStorage` key (WASM) and a `settings.json` (native).
- The **F3 perf overlay already exists** on both native and WASM (added for Spec 15) and
  surfaces rolling frame/tick samples — extend it to show **draw-call count** and **culled
  count** so the wins are visible and Axolittle can read them off during playtest.
- The **command system** (`commands/`) can optionally expose dials as `/gfx renderdist 6`
  etc. for quick testing — `commands/builtins/time.rs` is the read-write template.

---

## The Plan — two tracks

### Track A — Efficiency (invisible wins, benefit every machine)

Ordered by impact-to-effort. Each is independently shippable and testable.

| # | Change | Impact | Effort | Files |
|---|--------|:------:|:------:|-------|
| A1 | **Frustum culling** — extract 6 planes from view-proj in `camera.rs`; per-chunk AABB test before the draw loop in `render_world_viewport`. Pure helper `frustum_contains_aabb` is unit-tested. | **Huge** (drops ~70% of draw calls) | Med | `camera.rs`, `renderer.rs:2065+` |
| A2 | **Reuse GPU vertex buffers** — give entity/avatar/viewmodel/wireframe/ghost/crosshair buffers `COPY_DST`, grow-only capacity, write with `queue.write_buffer`. Crosshair becomes a one-time static buffer. | High (kills per-frame allocator churn) | Med | `renderer.rs` (the `upload_*`/`set_*` fns + `:2474`) |
| A3 | **Dirty-frame / frame pacing** — stop unconditional `request_redraw`; redraw on input/sim-change/animation, and add an optional **max-FPS cap** (helps heat/battery + weak GPUs). Native: replace `ControlFlow::Poll` spin. | High on native CPU + battery | Med | `game_loop.rs:8645`, `main.rs:1156`, `web_main.rs:33` |
| A4 | **Off-main-thread meshing (or bounded budget)** — at minimum, **rate-limit** the synchronous rebuild storms (lighting 27-chunk, stream 30-chunk) to a per-frame budget so a single edit can't stall a frame. Full worker-pool meshing (Spec 03 §2.7) is the bigger, later win. | High (kills hitches) | Med→High | `block_interact.rs`, `chunk_stream.rs`, `game_loop.rs:1109-1430`, `mesh.rs` |
| A5 | **Greedy-merge water** (`build_water_mesh`). | Med | Low | `mesh.rs:482` |
| A6 | **Mipmaps** on the block texture array. **DELIVERED 2026-09-06, opt-in** — CPU alpha-weighted linear-space chain per layer (`mipmap.rs`), live rebuild via `Renderer::set_mipmaps`, sampler keeps `mag: Nearest` so pixel art stays crisp; **anisotropy dropped** (wgpu needs all three filters Linear for it). Off in every preset. See Spec 03 §9.4b. | Med (quality + less shimmer) | Low–Med | `renderer.rs`, `mipmap.rs` |
| A7 | **u16 indices + packed vertex** (Spec 03 §2.4). Memory + bandwidth. | Med (VRAM) | Med | `mesh.rs:19`, `renderer.rs:2143` |
| A8 | **Distance LOD** for far chunks (Spec 03 §5.4). | Med (far-view cost) | High | `mesh.rs`, `chunk_stream.rs` |

**A1, A2, A3 are the headline web wins** and should land first. A4 removes the
felt stutter. A5–A8 are polish/longer-tail.

### Track B — Graphics settings (the headroom valve)

A new `GraphicsSettings` struct is the single source of truth that **replaces the
hardcoded constants**, with a UI panel and per-device persistence.

**Dials (each maps to existing engine state):**

| Dial | Range / values | Wires to |
|------|----------------|----------|
| **Render distance** | 4–16 chunks (Low≈5, High=10, Ultra≈14) | unify `RENDER_DISTANCE` (`main.rs:212`+`server.rs:23`) into the struct; thread into `chunk_stream.rs` |
| **Render scale** | 0.5×–1.0× internal resolution | render to an offscreen target at scaled size, blit to surface — **single biggest lever for weak GPUs** |
| **FOV** | 60°–100° | `Camera.fov_y` (`camera.rs:24`), per-player |
| **Mouse sensitivity** | slider | the `0.003` literal at `game_loop.rs:2795` (+ touch `:152`, gamepad `gamepad.rs:18`) |
| **VSync / frame cap** | On / Off / 30 / 60 / 120 / uncapped | `PresentMode` (`renderer.rs:285`) + A3 frame pacing |
| **Fog** | derived-from-render-distance / off | make fog a uniform (fixes the disconnect bug); off = clip at far plane |
| **Smooth lighting / AO** | on/off | future hook (AO not yet in shader) |
| **Particles** | full / reduced / off | particle spawn rate |
| **Mipmaps/anisotropic** | on/off | A6 toggle |

**Presets** bundle the dials so a kid never has to understand them:

- **Potato** — render dist 4, render scale 0.5×, fog off, no mipmaps, particles off, frame cap 30.
- **Low** — dist 5, scale 0.75×, fog on, particles reduced, cap 60.
- **Medium** — dist 7, scale 1.0×, fog on, cap 60.
- **High** — dist 10 (current default), scale 1.0×, all on, cap vsync. ← *matches today's look.*
- **Ultra** — dist 14, AO on, uncapped.

> **As shipped (2026-09-06):** `mipmaps` is **off in every preset**, Medium/High/Ultra
> included — it changes the *distant* look rather than the cost, so it stays a
> deliberate opt-in and "High == today's look" keeps holding.

Selecting a preset fills the dials; touching any dial flips to "Custom".

**Persistence:** per-device.
- WASM: `localStorage` key `axenstax_gfx` (JSON), bridged via the `wasm_save.rs:37-98`
  extern pattern, JS side mirroring `cloud.js`.
- Native: `settings.json` next to the executable / in a config dir, serde-JSON.

**UI:** `draw_settings_panel` in `menu.rs`, opened from `draw_pause_menu` via a new
`PAUSE_OPEN_SETTINGS` code, dispatched in `game_loop.rs:2295`. Follows the
`draw_skin_panel` open/close pattern. Apply-on-change, no reload.

**Optional:** `/gfx <dial> <value>` command for fast testing (`commands/builtins/`).

---

## Phasing (suggested build order)

A no-Axolittle session can do almost all of this; only the final feel-tuning needs him.

1. **Phase 1 — `GraphicsSettings` struct + persistence + plumb render distance.**
   Create the struct, wire load/save (WASM localStorage + native JSON), unify the two
   `RENDER_DISTANCE` consts into it, thread through `chunk_stream`. No UI yet — default
   = today's values (High preset), so behaviour is unchanged. Unit-test serde + preset
   mapping. *Solo.*
2. **Phase 2 — Frustum culling (A1).** Biggest single win, no settings dependency.
   Unit-test the plane/AABB math. F3 overlay gains draw-call + culled counts. *Solo.*
3. **Phase 3 — Buffer reuse (A2) + crosshair static buffer.** *Solo.*
4. **Phase 4 — Settings UI panel + presets.** `draw_settings_panel`, pause-menu entry,
   apply-on-change for render distance / FOV / sensitivity / vsync. *Solo.*
5. **Phase 5 — Render scale (offscreen target + blit).** The weak-GPU lever. *Solo.*
6. **Phase 6 — Fog-as-uniform** (fixes the disconnect bug; ties fog to render distance) +
   frame pacing / FPS cap (A3). *Solo.*
7. **Phase 7 — Bounded/off-thread meshing (A4).** Kills the edit/stream hitches. *Solo to
   build; needs Axolittle to confirm the stutter is gone.*
8. **Phase 8 — Polish:** water greedy merge (A5), mipmaps+aniso (A6), then u16/packed
   verts (A7) and LOD (A8) as separate follow-ons. *Solo.*
9. **Phase 9 — Axolittle playtest gate.** Run on his actual laptop. Compare Low/Medium/High
   against his Java Minecraft. Tune preset values from his feedback. Read F3 numbers.

## Spec maintenance (mandatory per CLAUDE.md)

Spec 03 (`docs/spec/03-rendering.md`) is the source of truth and **already describes**
frustum culling (§5.2), LOD (§5.4), background meshing (§2.7), the fog formula (§5.5),
and the 24-byte vertex (§2.4) — all currently divergent from code. As each phase lands,
**update Spec 03** to mark these as implemented and record the new `GraphicsSettings`
surface, the fog-uniform fix, and the preset definitions. Add a §"Graphics Settings"
section. Note the `RENDER_DISTANCE` deduplication.

## Risks / open questions

- **Render scale (Phase 5)** adds an offscreen render target + blit pass — the most
  invasive renderer change. Could be deferred if Phases 1–4 already deliver enough
  headroom; decide after the Phase 2/3 numbers come in.
- **Frame pacing on WASM:** the browser's rAF already vsyncs; the win there is *not
  rebuilding the frame when idle* + an explicit cap, not changing present mode. Native is
  where the `ControlFlow::Poll` CPU spin actually hurts.
- **Off-thread meshing on WASM** needs Web Workers + transferable buffers (Spec 03 §2.7) —
  genuinely hard. Phase 7 should start with the **bounded-budget** version (cap rebuilds per
  frame) which is cheap and removes most of the felt stutter, and treat true worker meshing
  as a later, separate spec if needed.
- **Settings vs split-screen:** render distance and render scale are global; FOV is
  per-player. Keep the struct's per-player vs global fields clear.

## Memory-rule check

- **PWA/web is the priority** ([[project_pwa_priority]]) — this spec leads with the WASM
  path; native rides along.
- **Cross-game lift** ([[project_shared_infra_strategy]]): a `GraphicsSettings` struct +
  per-device persistence + a settings panel + frustum culling are **engine-generic** — they
  lift to every Decented game, not AxeNStax-specific. Keep them in the engine's general
  surface, not behind AxeNStax assumptions.
- **No build yet:** this is the goal/spec only. Owner ([[feedback_autonomy_to_playtest_boundary]])
  will say when to action it; build runs solo up to the Phase 9 playtest boundary.
- **Merge-to-main pre-authorised** ([[feedback_merge_to_main_preauthorised]]) for healthy-gate
  merges once built.

# Block-Break Crack Overlay — progressive mining visual feedback

**Status: DELIVERED — 2026-05-24** (engine + specs on main; pending owner visual
check). 1732 tests (+9: 7 `crack_stage`, 2 texture). `check.sh` ALL GREEN.
**Owner ask:** Axolittle/Staxolottle noticed survival mining gives *no* visual
feedback — blocks take multiple hits but look untouched until they vanish.
Minecraft shows a crack pattern that deepens until the block breaks. Spec 05
§2.2 already requires a 10-stage crack overlay; the mechanic exists, only the
visual was never built. This foundation builds the visual.

---

## TL;DR

The mining *logic* is complete and correct: `PlayerSlot.break_progress` (ticks)
accumulates while a block is held under the crosshair, breaking when it reaches
`crafting::break_time_ticks(block, tool)`. **What's missing is rendering** — the
progress is never shown. This spec adds a 10-stage crack overlay drawn on the
targeted block, client-side, derived purely from the existing break state. No
new gameplay, no networking, no protocol change. Single-viewport and
split-screen both covered (per-player, like the existing block highlight).

---

## Context pointers (verified against the tree 2026-05-24)

- **Break state** lives on `PlayerSlot`: `breaking_pos: Option<[i32;3]>`,
  `break_progress: u32` (`game/engine/src/player_slot.rs:41-43`). Reset to
  `None`/`0` whenever the player switches target or stops mining
  (`game_loop.rs:2831-2980, 3354-3374`).
- **Break time** is `crafting::break_time_ticks(block_id, Option<&Tool>) -> u32`
  (`crafting.rs:387`). Creative is instant (the survival branch at
  `game_loop.rs:3122` is the only place `break_progress` accumulates).
- **Per-frame target + highlight** is computed at `game_loop.rs:2520-2531`:
  raycast → `target_block` → `renderer.set_block_highlight(pidx, target)`. This
  is exactly where the crack overlay update hooks in.
- **Texture array**: a single `texture_2d_array<f32>` with
  `texture_gen::texture_count()` layers (currently **207**, indices 0-206),
  uploaded in `renderer.rs:202-247`. `mesh::Vertex` carries a `tex_layer: u32`
  that selects the layer (`mesh.rs:20-25`, `shader.wgsl` `VertexInput`).
- **Render passes** per viewport are an ordered list in
  `render_world_viewport` (`renderer.rs:1593-1849`): chunks → water → entities →
  **Pass 2 block-highlight wireframe** → Pass 2.5 ghost wireframe → Pass 2.8
  viewmodel (which *clears* depth, so it must stay last). The wireframe pipeline
  (`renderer.rs:486-528`) is the model to copy: alpha-blended, `depth_write=false`,
  `depth_compare=LessEqual`, depth bias `constant:-2, slope:-1` so the overlay
  hugs the block surface and is correctly occluded by nearer terrain.
- **Shaders**: `shader.wgsl` (chunk/entity, samples the texture array),
  `overlay.wgsl` (wireframe/crosshair, flat colour). The crack overlay needs the
  texture array, so it reuses `shader.wgsl`'s `vs_main` + a **new** fragment
  entry `fs_crack`.

Spec cross-refs: gameplay contract `docs/spec/05-gameplay-systems.md §2.2`
(updated by this work); rendering spec `docs/spec/03-rendering.md` had **no**
crack-overlay section — this work fills that gap.

---

## Design

### Stage selection (pure, testable)

Add to `crafting.rs` next to `break_time_ticks`:

```rust
/// Crack overlay stage (0-9) for the current mining progress, or None when
/// nothing should be drawn. Returns None for instant breaks (break_time == 0)
/// and before any progress accrues (progress == 0), so the overlay appears
/// only once the player has actually started mining and clears the instant
/// they stop (progress resets to 0 on release/target-switch).
pub fn crack_stage(break_progress: u32, break_time: u32) -> Option<u8> {
    if break_time == 0 || break_progress == 0 {
        return None;
    }
    // floor(progress/break_time * 10), clamped to the last stage so the final
    // tick (progress >= break_time) shows stage 9, not an out-of-range 10.
    let stage = (break_progress.saturating_mul(10) / break_time).min(9);
    Some(stage as u8)
}
```

Tests (write first, RED → GREEN):
- `crack_stage(0, 200) == None` — not started.
- `crack_stage(5, 0) == None` — instant/creative.
- `crack_stage(1, 200) == Some(0)` — just started → hairline.
- `crack_stage(199, 200) == Some(9)` — almost done → heavy.
- `crack_stage(200, 200) == Some(9)` — at break threshold, clamped (not 10).
- `crack_stage(500, 200) == Some(9)` — overshoot clamped.
- monotonic non-decreasing across rising progress for a fixed break_time.

### Textures (10 stages)

Append 10 procedural crack textures to `texture_gen::generate_textures()` at
layers **207-216**, and bump `texture_count()` 207 → **217**. Add to `block.rs`:

```rust
/// Crack-overlay layers (207-216 = stages 0-9). Stage N is TEX_CRACK_BASE + N.
pub const TEX_CRACK_BASE: u32 = 207;
```

`gen_crack_stage(stage: u8) -> Vec<u8>`: 16×16 RGBA on a **fully transparent**
background (`a = 0`); crack pixels are near-black (`rgb ≈ 18,18,18`) with alpha
ramping by stage so early stages are faint hairlines and late stages are dense,
opaque fracturing. Deterministic, seeded with `px_hash` so the pattern is stable
frame-to-frame. Density (number of crack segments) scales with stage. Because
the overlay is alpha-blended over the block, dark+partial-alpha pixels read as
cracks darkening the face — matching Minecraft's `destroy_stage_*` look.

### Renderer

**Pipeline** — add `crack_pipeline: wgpu::RenderPipeline`. Same layout as the
chunk pipeline (camera bind group 0 + texture bind group 1, `Vertex::layout()`),
but: `fs_crack` fragment, `ALPHA_BLENDING`, `cull_mode: None`, depth
`{ write:false, compare:LessEqual, bias: constant -2 / slope -1 }` (identical to
the wire pipeline's depth bias). Reuses the existing `texture_bind_group`.

**Shader** — add to `shader.wgsl`:

```wgsl
@fragment
fn fs_crack(in: VertexOutput) -> @location(0) vec4<f32> {
    // Crack texture carries its own alpha; no lighting/fog so cracks read
    // consistently in caves and daylight alike.
    return textureSample(block_textures, block_sampler, in.uv, i32(in.tex_layer));
}
```

**Per-player buffers** — add `crack_buffer: Option<wgpu::Buffer>` +
`crack_vertex_count: u32` to `PlayerGpuResources` (init `None`/`0`).

**Upload** — `set_crack_overlay(&mut self, player_index, overlay: Option<([i32;3], u8)>)`:
`None` clears the buffer; `Some((pos, stage))` builds a 6-face cube (36 verts,
`mesh::Vertex`) at `pos`, **inflated by ~0.003** on every axis so it sits just
outside the block surface, each vertex `tex_layer = TEX_CRACK_BASE + stage`,
full `uv` 0..1 per face, correct per-face `normal`, `light = 1.0`. Mirror the
`set_block_highlight` buffer-management pattern (`renderer.rs:740`).

**Pass** — new "crack_pass" inserted as **Pass 2.6**, immediately after the
ghost-wireframe pass and before the viewmodel pass (the viewmodel clears depth,
so anything that depth-tests against world depth must precede it). Load colour +
depth, set viewport/scissor, bind camera (0) + texture (1), draw the crack
buffer. Skip when `crack_buffer` is `None`.

### Game-loop wiring

Right after `set_block_highlight` at `game_loop.rs:2531`, compute the overlay
from the *existing* break state (one-frame latency vs the mining update later in
the same frame is imperceptible) and push it:

```rust
let crack_overlay = self.players[pidx].breaking_pos.and_then(|bp| {
    let blk = self.world.get_block(bp[0], bp[1], bp[2]); // World::get_block (world.rs:623)
    let tool = self.players[pidx].inventory
        .hotbar_slot(self.players[pidx].hotbar_slot)
        .and_then(|s| match &s.item { crate::item::Item::Tool(t) => Some(t), _ => None });
    let bt = crate::crafting::break_time_ticks(blk, tool);
    crate::crafting::crack_stage(self.players[pidx].break_progress, bt).map(|s| (bp, s))
});
self.renderer.set_crack_overlay(pidx, crack_overlay);
```

Compute into a local first (immutable borrows of `players`/`world`), then call
the renderer (mutable borrow) — avoids a borrow conflict. `break_progress == 0`
(reset on release / target-switch) makes `crack_stage` return `None`, so the
overlay clears the moment the player stops mining even if `breaking_pos` lingers
a frame.

---

## Phased scope

- **Phase 1 — stage logic (TDD).** `crack_stage` + 7 unit tests in `crafting.rs`.
- **Phase 2 — textures.** `gen_crack_stage` + 10 layers + `texture_count` 217 +
  `TEX_CRACK_BASE` + the `texture_count` doc-comment tally line.
- **Phase 3 — render path.** `fs_crack`, `crack_pipeline`, per-player buffers,
  `set_crack_overlay`, Pass 2.6.
- **Phase 4 — wiring.** Call `set_crack_overlay` in `game_loop.rs`.

All four are one session; no playtest gate between them.

---

## Acceptance criteria

- `crack_stage` unit tests pass (RED-first).
- `./check.sh` green: clippy (no new warnings), engine build, `cargo test`
  (all existing + the new crack_stage tests), `trunk build`, bundle < 5 MiB.
- Texture array length matches `texture_count()` (217) — there's an implicit
  contract that `generate_textures().len() == texture_count()`; verify the count
  is bumped in lockstep (add a unit test asserting equality if one doesn't exist).
- No protocol/save/format change (overlay is pure client render state).
- WASM + native both compile (the crack path is platform-agnostic — no threads,
  no native-only deps).

## Out of scope (deferred)

- **Multiplayer crack sync.** When a server simulates a *remote* player mining,
  their crack stage isn't broadcast — only the local player sees their own
  cracks. Matches the current single-viewport/split-screen alpha. Trigger:
  Spec 1 Phase 4 multiplayer or a dedicated "remote mining feedback" spec. Spec
  05 §2.2's "client-predicted" note already anticipates this.
- **Break particles** on completion (Spec 03 §particles) — separate feature;
  `audio.play_break()` already fires.
- **Per-block crack tinting** — single grey crack set covers all blocks
  (Minecraft does the same).

## Memory-rule check

- Visual-only, single-player-verifiable build BUT the final confirmation is a
  *visual* one the owner runs ([[feedback_autonomy_to_playtest_boundary]]): I
  build + run `check.sh` + ship, then stop for the owner's eyes-on check.
- Merge to main is pre-authorised on a healthy gate
  ([[feedback_merge_to_main_preauthorised]]) — `check.sh` green is the gate.
- No Bitcoin/Signet/economy surface touched; no owner-convergence concern.

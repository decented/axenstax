# Lighting system + torch/tall-grass visibility + campfire UX — Spec 30

**Status**: **DELIVERED 2026-05-21** on `feat/spec30-lighting-and-campfire-ux`. All 11 active phases (A–K) shipped; Phase L (Axolittle playtest) is the remaining gate. Bundle 1.72 MiB under 5 MiB gate; 1,206 tests pass (+16 from baseline).
**Date**: 2026-05-21
**Branch**: `feat/spec30-lighting-and-campfire-ux`
**Parent specs**: Spec 02 §"Lighting" (full BFS design — this is the build), Spec 05 §3.10 (current "Light gate dropped for T1" callout — removed by this spec), Spec 17 (Campfire), Spec 20 (Furnace).

---

## TL;DR

Three threads bundled because they share rendering surface:

1. **Lighting system — full BFS** per Spec 02. 4-bit block-light + 4-bit sky-light per voxel; propagation on world-gen, place, break; per-vertex brightness consumed by the chunk shader. Re-enables mob-spawn darkness gating + crop growth light gate (both currently disabled).
2. **Torch + tall-grass visibility fix.** Both blocks are flagged `transparent=true, solid=false`, which causes the greedy mesher to skip them entirely. Adds a small-cube non-greedy mesh path so they actually render.
3. **Campfire UX.** Floating hover-label when crosshair-on-campfire within strike distance, showing fuel ticks, lit state, smoke output, heat radius, light level. New **Smouldering** state — fuel exhaustion transitions to ~30 in-game-seconds of smoulder (visible but no light/heat); during smoulder, dropping fuel relights without friction/flint-and-steel.

After this lands the alpha has *real* light-driven gameplay: caves are dark, torches matter, campfires anchor a survival night, crops need sky.

---

## Scope (what's in)

### A. Torch + tall-grass visibility (small-cube mesh path)

The greedy mesher (`mesh.rs::greedy_face`) skips `transparent=true` blocks for both the block-being-drawn check (line 212) AND the adjacency-block check (line 222 — correct, so neighbours render against transparent blocks). The problem is the first check: torches + tall grass + flowers + future glow-blocks all get filtered out.

Add a second pass alongside greedy: `non_solid_mesh` (or fold into `build_chunk_meshes`). For each block where `solid=false AND transparent=true AND block != AIR AND block != WATER`, emit a small cube or cross-shape mesh. Alpha simplification: **small textured cube** centred on the block, ~50% of full-block size. Matches the current `block.rs:715` comment ("rendered as a small textured cube — proper X-shape lands when block-shape variants are a thing"). X-shape mesh is deferred to a future polish.

**Eligible blocks** (today): `TORCH` (23), `TALL_GRASS` (24). Future flowers / lanterns / glow-blocks inherit automatically.

### B. Lighting system — full BFS

Per Spec 02 §"Lighting" with two pragmatic simplifications for alpha:

#### B.1 Per-voxel data

- `block_light: u4` (0–15) per voxel — light emitted by torches, campfires, lava, glowstone-equivalent.
- `sky_light: u4` (0–15) per voxel — light from the sky, attenuated downward by opaque blocks.
- Packed as `light: u8` per voxel = `(sky << 4) | block`.
- Stored in a new `light` array on `Chunk`, parallel to the existing `blocks` array. Total: 1 byte × 16³ = 4 KB per chunk section, matches Spec 02's budget.

#### B.2 Light emission table

`BlockDef` gains `light_emission: u8` (0–15). Initial values:

| Block | Emission |
|---|:--:|
| `TORCH` | 14 |
| `CAMPFIRE` (lit) | 14 |
| `FURNACE_LIT` | 13 |
| `OAK_LOG` / planks / leaves / dirt / stone / cobblestone / ore | 0 |
| Future glowstone / lantern | 15 / 12 |
| Lava (future) | 15 |
| Everything else | 0 |

Smouldering campfire (state inside `CampfireData`, block-id still `CAMPFIRE_UNLIT`) emits 0 — matches the design ("no light during smolder").

#### B.3 BFS propagation

Two pure functions live in a new `lighting.rs`:

```rust
pub fn propagate_from(world: &mut World, sources: impl IntoIterator<Item = (i32, i32, i32)>);
pub fn remove_at(world: &mut World, pos: (i32, i32, i32), prev_light: u8);
```

Standard Minecraft-style BFS: queue (pos, light_level); pop; for each cardinal neighbour, if `current_light - 1 - absorption > neighbour_light`, set neighbour_light and enqueue. Absorption is 0 for transparent blocks, 15 for opaque (opaque blocks don't propagate; they sit at 0).

Removal: a darken-then-refill pass — set the source to 0, BFS-darken all neighbours that were lit *by* the source, then re-propagate from any remaining light sources at the boundary. Same primitive Minecraft uses.

Block-light and sky-light use **separate BFS queues** with the same algorithm. Sky-light additionally seeds from y = top of chunk downward.

#### B.4 World-gen initial pass

After `world::generate_column` finishes terrain + trees + structures for a column, run an initial light pass over all chunks in the column:

1. Top-down sky pass: for each (x, z), find the highest opaque block; everything above it gets sky_light = 15; everything from there down through transparent blocks also gets 15 until an opaque block stops the propagation.
2. Block-light pass: enumerate all light-emitter positions in the column; BFS-propagate.

Cross-column: light leaks across column boundaries. When a neighbouring column generates, re-propagate at the shared boundary.

#### B.5 Place / break re-lighting

- **Block place** (any block, not just emitters):
  - If new block has `light_emission > 0`: BFS-propagate.
  - If new block is opaque (absorption > 0): the previous sky/block light at that cell may now be wrong — run `remove_at` then re-propagate from any neighbouring sources.
- **Block break**:
  - If old block had `light_emission > 0`: `remove_at` with the old emission.
  - If old block was opaque: cell becomes transparent; re-let sky-light through (BFS).

The caller (`game_loop::place`/`break`) signals the lighting module after the world-state change.

#### B.6 Mesh integration

`Vertex` struct gains `light: f32` (0.0–1.0, where 1.0 = full bright). At mesh-emit time:

- Each face's quad reads the light value of the **air-side adjacent block** (the block whose normal points toward `AIR`). That's where the sky/block light is.
- For simplicity (no AO yet), all 4 corners of a quad share the same light value (the centre value). Spec 02's smooth-lighting via corner-averaging is a future polish.
- `light_combined = max(block_light, sky_light * time_of_day_factor) / 15.0`.
- `time_of_day_factor` lives in the **fragment shader** (lighting uniform buffer already has `sky_light_level` per Spec 03 §86) so day-night transitions don't require re-meshing. Vertex stores raw 0..15; shader multiplies.

Vertex format change → all chunk meshes rebuild on load.

#### B.7 Save format

- New chunk save field: `light: Vec<u8>` (or `[u8; 4096]`) per chunk section.
- Old saves: `#[serde(default)]` returns an empty Vec; load path detects this and runs the world-gen initial light pass on the loaded chunks.
- No `PROTOCOL_VERSION` bump (light is server-side derivable; not in the wire packet).

#### B.8 Re-enabled gameplay gates

- **Mob spawning** — Spec 5 §7 mob-spawn check: hostile mobs spawn iff `effective_light = max(block_light, sky_light - 4) <= 0`. The 4-offset is Minecraft's "night" threshold. Currently disabled per `spawning.rs`; this spec re-enables.
- **Crop growth** — Spec 5 §3.10 says crops require `light_level >= 9`. Currently dropped for T1; re-enabled.
- **Spec 5 §3.10 callout removed** — the "Light gate: dropped for Tier 1" text becomes a "Light gate active as of Spec 30 (2026-05-21)" line.

### C. Campfire UX — floating label + smoulder

#### C.1 Smoulder state

`CampfireData` gains a new tagged-union state:

```rust
pub enum CampfireBurnState {
    Unlit,            // Cold. Needs friction or flint-and-steel.
    Lit,              // Active. Cooks + emits light + heat + smoke.
    Smouldering { ticks_remaining: u32 },  // ~30 in-game seconds (600 ticks). No light, no heat, no cook progress. Visible flame-trace + faint smoke.
}
```

Or, to minimise serde migration: keep the current `lit: bool` + add `smoulder_ticks: u32`. If `lit == false && smoulder_ticks > 0` → smouldering. Both representations are fine; the tagged enum is cleaner long-term.

**Transitions**:

- `Lit` + `fuel_ticks` decremented to 0 → `Smouldering { ticks_remaining: 600 }`. Block-id flips to `CAMPFIRE_UNLIT` (the renderer uses the unlit texture for smouldering). Light emission drops to 0.
- `Smouldering { ticks: 0 }` → `Unlit`. No state change visible.
- `Smouldering` + player adds fuel (right-click with fuel item OR — Spec 29 parity — slot-click if a future UI lands) → `Lit` instantly with `fuel_ticks` from the fuel item. **No friction or flint-and-steel needed.** Block-id flips back to `CAMPFIRE`.
- `Unlit` + add fuel → still requires friction or flint-and-steel (existing behaviour preserved).

Visual: smouldering campfires use the same block-id as `CAMPFIRE_UNLIT` for now (texture-genned darker fire). A faint smoke particle continues at 25% the lit-state rate. Future polish: dedicated smouldering texture.

#### C.2 Floating hover label

Trigger: crosshair targets a `CAMPFIRE` or `CAMPFIRE_UNLIT` block AND the player is within **strike distance** (~4 blocks — same as `game_loop`'s break-block reach).

Label content (egui floating panel anchored above the block):

```
Campfire — Lit
  Fuel:    142 ticks (≈7 s)
  Light:   14
  Heat:    radius 5 blocks
  Smoke:   on
```

For Smouldering:

```
Campfire — Smouldering
  Time left:  18 s
  Drop fuel to relight
```

For Unlit:

```
Campfire — Cold
  Friction or flint and steel to light
```

Rendered each frame the conditions are met. Uses the existing egui overlay infrastructure (mirrors plaque dialog / villager dialog idiom). No state mutation — pure read.

#### C.3 Smoke + heat continue (small)

- Smouldering still emits the heat-fear AI signal at half the lit-state radius (so a smouldering campfire still mildly deters mobs).
- Smoke pillar (Spec 18) shrinks to one block during smoulder.

---

## Scope (what's out — deferred)

- **Smooth lighting / ambient occlusion.** Vertex stores per-block light, not per-corner. Smooth-shading via corner averaging is a polish pass.
- **Coloured light.** Single brightness channel for alpha.
- **Light bleed across chunk-edge fix-ups beyond cardinal neighbours.** Diagonal chunk neighbours don't re-light (Minecraft accepts this).
- **Per-vertex AO from neighbouring opaque blocks** (the dark corners in Minecraft caves).
- **Cross-shape mesh for torches** (proper X-quad like Minecraft). Small cube for alpha.
- **Sun-angle visual ramp** (sunrise/sunset reddening). Lighting uniform handles brightness modulation but colour is left as today.
- **Per-block-entity dynamic emission** (e.g. furnace light pulses with smelt progress). Static `light_emission` table is fine.

---

## Phasing

| # | Phase | Files | Solo? |
|:-:|---|---|:--:|
| A | Torch + tall-grass small-cube mesh path. Tests: chunk containing a TORCH emits non-zero mesh geometry. | `mesh.rs`, new tests | ✓ |
| B | `Vertex.light: f32` + WGSL shader plumbing. Default 1.0 so no visual change. | `mesh.rs`, `shaders/*.wgsl`, `renderer.rs` | ✓ |
| C | `BlockDef.light_emission: u8` + `lighting.rs` pure BFS (propagate + remove). Unit tests cover open propagation, opaque-block blocking, source removal, multi-source overlap. | `block.rs`, new `lighting.rs` | ✓ |
| D | `Chunk.light: [u8; 4096]` data field + world-gen initial pass. Tests: top-down sky pass + block-light from emitters in a generated column. | `chunk.rs`, `world.rs`, `lighting.rs` | ✓ |
| E | Mesh consumes per-face light from air-side adjacent block. Shader applies time-of-day modulation. Tests: a vertex adjacent to a torch has brightness > 0; a buried vertex has brightness 0. | `mesh.rs`, shaders | ✓ |
| F | Place/break re-lighting via lighting module + targeted chunk re-mesh. Tests: placing a torch in a dark cave lights surrounding blocks; removing it darkens them. | `game_loop.rs`, `lighting.rs` | ✓ |
| G | Save persistence for chunk light arrays. Old saves trigger initial-light-pass via `#[serde(default)]`. Tests: save → mutate light → load → identical light values. | `chunk.rs`, `save.rs`, `test_integration/save_load.rs` | ✓ |
| H | Re-enable mob-spawn light gate + crop growth light gate. Spec 5 §3.10 callout removed. | `spawning.rs`, `growth.rs`, `docs/spec/05-gameplay-systems.md` | ✓ |
| I | Campfire smoulder state (`CampfireBurnState` or `smoulder_ticks`). Block-id flip back to LIT on relight. Light-emission table reads campfire state. Tests: fuel-exhaust → smoulder → unlit transitions; fuel-during-smoulder = instant relight. | `campfire.rs`, `game_loop.rs` | ✓ |
| J | Campfire hover label UI — egui floating panel triggered by crosshair-on-campfire within strike distance. Reads CampfireData; pure render. | `campfire_ui.rs` (new), `game_loop.rs` | ✓ |
| K | Spec doc updates + `./check.sh` ALL GREEN + commit + PR + merge + `cargo build --release`. | docs/spec/02, docs/spec/05, docs/foundations/README.md | ✓ |
| L | Axolittle playtest — verify torches visible + light caves + campfire smoulders + hover label readable + mobs respect darkness. | – | playtest |

**Estimated total**: ~1,500–2,000 LOC including tests, across 11 active phases + 1 playtest gate.

---

## Acceptance — overall

- All 11 active phases complete or explicitly deferred.
- `./check.sh` ALL GREEN throughout — no regression in the existing 1,190 tests; new tests added per phase.
- Spec 02 §"Lighting" reflects what shipped.
- Spec 05 §3.10's "Light gate dropped" callout reads "Light gate active (Spec 30, 2026-05-21)".
- Spec 30 foundation doc flips READY TO BUILD → DELIVERED with commit links.
- Foundations README updates Spec 30 entry to delivered + crosses out the "lighting deferred" implicit gap.

---

## Open design questions

1. **Smoulder duration**: 30 in-game-seconds (600 ticks) — Axolittle's call on whether longer (1 minute = 1,200 ticks) feels right. Locked at 600 for alpha; tunable post-playtest.
2. **Initial-light-pass cost on load**: legacy saves with no light data run the full pass. May add 50–500 ms on world load depending on chunk count. Watchable in F3 perf overlay. If slow, defer to first-tick batched pass.
3. **Hover label range**: ~4 blocks (strike distance). If Axolittle wants a "look from across the room" gesture, easy to widen post-playtest.
4. **Block-light vs sky-light visual mix**: today's "use max" works. Minecraft uses a per-channel blend with sky tinted by time-of-day. Defer the per-channel tint.
5. **Cross-game lift**: lighting + campfire-state-aware emission lift to other games on the same primitives (cooking stations, incubator warmth, etc). Tagged in §"Cross-game lift" below.

---

## Cross-game lift

- **`lighting.rs` BFS module** — generic. Any voxel game inherits.
- **`BlockDef.light_emission`** — generic. Other games re-use.
- **`CampfireBurnState` enum** — pattern-generic (Unlit/Active/Cooling-down). another game's oven could mirror.
- **Hover-label-when-targeted UI** — generic widget. Vendor Block + Furnace future UIs may want it.

---

## Memory-rule check

- ✓ axenstax has farming — Phase H re-enables the crop light gate that farming needs for night-vs-day growth balance.
- ✓ shared infra strategy — `lighting.rs` + `CampfireBurnState` are explicitly engine-generic; called out in §"Cross-game lift".
- ✓ uk english naming — "smouldering" not "smoldering" throughout.
- ✓ pretest check — torch-invisibility root cause confirmed in `mesh.rs:212` before drafting.
- N/A charter phase4 shared gap — no Signet surface.
- N/A signet boundary — no Signet changes.

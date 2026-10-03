# Rail auto-connection (flat bends) — design

**Date:** 2026-07-02
**Status:** design, awaiting approval
**Branch:** `feature/rail-auto-connect`
**Scope tier:** medium, low architectural risk

## Problem

Player report (Axolittle, build 0.2.0): *"when you join tracks blocks, they all
appear as straights in one direction. they should align and also create bends
where appropriate. The minecraft way will be a good foundation."*

`TRACK` (id 260, `rail.rs:10`) currently renders as a single flat single-texture
slab with no orientation — the flat AABB `([0,0,0],[1,0.1,1])` emitted through the
non-solid small-cube path (`mesh.rs:403`, `mesh.rs:661`). A rail never reflects the
shape of the line it is part of: a straight east–west run and a straight
north–south run look identical, and an L-junction shows two straights meeting at a
right angle instead of a curve. This design gives rails a **visual shape** that
aligns to their neighbours and forms **corner bends**, mirroring Minecraft.

## How Minecraft does it (foundation)

Every rail stores a `shape`: `north_south`, `east_west`, four corners
(`north_east`, `north_west`, `south_east`, `south_west`), and four ascending
ramps. On placement — and when a rail is placed beside existing rails — each rail
re-picks its shape from its neighbours: one neighbour → straight toward it; two
opposite → straight; two perpendicular → **corner**; a T/cross → a corner chosen
by a fixed preference order (**west → east → south → north**). Neighbours
re-orient automatically. (Sources: Minecraft Wiki — Rail.)

## Scope

**In (v1):**
- Straights (`north_south`, `east_west`) + the four flat corner bends.
- Auto-alignment: placing or removing a rail restyles it and its rail neighbours.

**Out (clean follow-ups, explicitly deferred):**
- **Ascending / sloped rails — the "elevation layer" (next build).** A cart can't
  step up a block face, so rails cross elevation via a ramp. **Decision (owner,
  2026-07-02): lock to 45° ascending rails** (one block across = one block up —
  the natural voxel fit and Minecraft-parity). Needs cart *vertical* pathing
  (carts path on a flat plane and park at height changes today) plus a tilted
  ramp mesh + the ascending-detection rule (a rail with a rail one block up in an
  adjacent direction becomes an ascending ramp toward it). **Future exploration
  (not v-next, possible differentiator):** shallower, multi-block ramps (e.g. 2
  across for 1 up ≈ 26°) — this is a meaningfully bigger cart-physics job
  (non-45° slope traversal, variable ramp meshes) and is recorded in the roadmap
  so it isn't lost. Paired with the cable **surface-mount** feature below (same
  "changes height" story, different mechanism — cables turn 90° on surfaces, no
  ramp).
- **Junction switching.** Carts already park at >1-way splits; a T/cross renders as
  a bend by preference and stays visual-only.
- **Lone-rail player-facing default.** A rail with no neighbours defaults to
  north–south (see approach trade-off); Minecraft's "face the player" default would
  require stored state and is not worth it for a momentary, cosmetic case.

## Approach: live-computed shape (the Wall / Fence-pane pattern)

A rail's shape is **derived every time it is meshed**, purely from which of its four
cardinal neighbours are also rails — the exact mechanism `Wall` and `Pane` already
use (`block_shape.rs:79-93`, `world.rs:connection_mask` at `world.rs:1463`). It is
**not** stored per-block.

Chosen over Minecraft's "stamp a stored shape at placement" model because in this
engine live-computation means:
- **No save-format change** — nothing new persisted.
- **No network-protocol change** — nothing new on the wire; each client recomputes
  shape from block topology, which is already replicated.
- **Neighbours restyle for free** — placing/removing a block already re-meshes the
  owning chunk and marks seam-neighbour chunks dirty (`block_interact.rs:113-154`),
  so a connecting block auto-restyles with no explicit neighbour-notify. This is the
  documented invariant `Wall`/`Pane` rely on.
- **Matches how carts already path** (topological, see below).

**Trade-off accepted:** a *lone* rail cannot remember a player-chosen direction
(defaults to north–south until a neighbour gives it an axis), and a T/cross
auto-picks a bend rather than letting the player force a straight-through. Both are
**cosmetic only** — carts park at junctions regardless — and acceptable for v1.
(The alternative, stored meta stamped at placement, is strictly more code: a place-
time arm plus a re-stamp consumer for neighbours, plus save/protocol carriage. Not
justified here.)

## Shape model (the load-bearing logic)

A dedicated reader `rail_neighbour_mask(world, x, y, z)` builds a 4-bit mask by
checking the four cardinal cells for `TRACK`, using the existing bit convention
(`block_shape.rs:122-125`):

```
CONN_N = 0b0001  (−Z)   CONN_S = 0b0010  (+Z)
CONN_W = 0b0100  (−X)   CONN_E = 0b1000  (+X)
```

A rail connects only to another rail (`TRACK`) — same-level only (ascending rails
are out of scope). This is a small direct reader in `rail.rs`; it does **not** go
through `BlockShape`/`connects()` (see Rendering — rails need per-shape texture
selection the generic shaped path can't provide).

`RailShape` enum: `StraightNS`, `StraightEW`, `CornerNE`, `CornerNW`, `CornerSE`,
`CornerSW`. A `CornerNE` visually joins the **north** and **east** edges (an L to the
north neighbour and the east neighbour), and so on.

Pure, unit-testable mapping `rail_shape_from_mask(mask: u8) -> RailShape`, applying
Minecraft's preference order **west → east → south → north**:

```
horiz = if mask & CONN_W  { West }  else if mask & CONN_E  { East }  else None
vert  = if mask & CONN_S  { South } else if mask & CONN_N  { North } else None

match (horiz, vert):
    (Some(h), Some(v)) => Corner(v, h)   // S+W→SW, S+E→SE, N+W→NW, N+E→NE
    (Some(_), None)    => StraightEW     // only horizontal neighbour(s)
    (None,   Some(_))  => StraightNS     // only vertical neighbour(s)
    (None,   None)     => StraightNS     // lone rail default
```

Worked cases (all 16 masks resolve deterministically):

| Rail neighbours | Shape |
|---|---|
| none | `StraightNS` (default) |
| N only / S only / N+S | `StraightNS` |
| E only / W only / E+W | `StraightEW` |
| N+E | `CornerNE` |
| N+W | `CornerNW` |
| S+E | `CornerSE` |
| S+W | `CornerSW` |
| N+S+E (T) | `CornerSE` (horiz=E, vert=S) |
| N+S+E+W (+) | `CornerSW` (horiz=W, vert=S) |

This single rule reflects Minecraft's preference order (W beats E, S beats N) and
needs no special-casing per neighbour count.

## Rendering

**Mechanism (refined during planning):** the generic shaped-mesh path
(`render_cuboids`) textures every cuboid with the block's *fixed*
`tex_top/side/bottom`, so it cannot orient a straight NS vs EW run or give corners
a distinct texture. Rails therefore render through a small **dedicated branch** in
the chunk mesher rather than the generic path. (No `BlockShape::Rail`,
`is_connecting`, `connects`, or `render_cuboids` change.)

- **Remove** `TRACK` from the two non-solid small-cube special-cases
  (`mesh.rs:403` and `mesh.rs:661`) — it no longer draws as a single flat cube.
- **Add a rail branch** in the mesher's per-cell loop (~`mesh.rs:620`): compute
  `rail_shape_from_mask(rail_neighbour_mask(world, wx, wy, wz))`, then emit the
  shape's flat draw boxes via the existing `emit_small_cube`.
- A pure helper `rail_render_boxes(shape) -> Vec<(min, max, tex_layer)>` (in
  `rail.rs`) returns the geometry: `StraightNS` → one full slab (NS texture);
  `StraightEW` → one full slab (EW texture); each corner → two perpendicular
  half-legs, the vertical leg NS-textured and the horizontal leg EW-textured.
- **Collision**: unchanged — rails never went through `collision_aabbs`; the thin
  ground slab is implicit in the flat draw boxes (`y 0.0..0.1`).

### Textures

- `StraightNS` reuses the existing `gen_track` texture (`TEX_TRACK`, id 373).
- Add **one appended texture** — `gen_track_ew`, the rail image rotated 90° so the
  bars run along X — as a new layer `TEX_TRACK_EW` (next free id), appended after
  the last `textures.push(...)` and matched by a bumped `texture_count()`.
- **No bespoke corner texture in v1**: a corner is drawn as a NS leg + an EW leg
  meeting at the elbow, so the two straight images form the bend. A dedicated
  curved-corner texture is a post-playtest polish option, not required to fix the
  reported bug.
- Corner-leg overlap/elbow and the exact look are the **visual-judgment** part —
  expect an Axolittle playtest to confirm the bend reads correctly (see Risks).

## Carts — unchanged

Cart pathing is **topological**, not shape-based: `next_track_step`
(`rail.rs:36`) scans the four horizontal track neighbours and returns the onward
cell, so carts already turn corners (passing test `follows_a_corner`,
`rail.rs:92`). Adding a visual corner shape requires **zero** changes to cart
movement — the cart already turns the L; the corner mesh just makes it *look* like a
bend. Keep the existing test as a guard.

## Persistence & protocol — unchanged

Live-computed shape stores nothing, so there is **no save-format change and no
protocol bump**. The append-only `WorldSave` / `CartData` bincode layout is
untouched.

## Testing

- **Unit** (pure): `rail_shape_from_mask` for representative masks across all 16
  combinations, including the junction preference order (T and cross).
- **Unit**: `connects(Rail, TRACK)` is true; `connects(Rail, <solid/other>)` is
  false.
- **Regression**: keep `follows_a_corner` (cart still turns).
- **Mesh smoke**: `TRACK` now resolves through the shaped path and produces distinct
  geometry for `StraightNS` vs `StraightEW` vs a corner mask (not the old single
  flat cube). Run `check.sh` (clippy + build + `cargo test` + `trunk build` + bundle
  gate) as the correctness gate.

## Files to touch

| File | Change |
|---|---|
| `game/engine/src/rail.rs` | `RailShape` enum; `rail_shape_from_mask`; `rail_neighbour_mask`; `rail_render_boxes`; unit tests |
| `game/engine/src/mesh.rs` | Remove `TRACK` from small-cube cases (`:403`, `:661`); add the dedicated rail branch |
| `game/engine/src/texture_gen.rs` | `rotate90` + `gen_track_ew`; append the layer; bump `texture_count()` |
| `game/engine/src/block.rs` | `TEX_TRACK_EW` const (appended layer id) |
| `game/engine/src/test_integration/blocks.rs` (+ `test_harness.rs` if a setter is missing) | Integration test: an L of tracks resolves to a corner |

No changes to `block_shape.rs`, `save.rs`, `protocol.rs`, `cart.rs` movement, or
`world.rs`.

## Risks / open questions

- **Visual correctness of the corner mesh + texture** is the only real risk and is
  a look-at-it judgment: the auto-connect logic is verified by unit tests, but
  "does the bend look right / align cleanly with the straights" needs an Axolittle
  playtest. Ship behind `check.sh` green, then playtest the visuals.
- **Bit-order sanity**: `connection_mask` uses N=−Z, S=+Z, W=−X, E=+X — the shape
  table above is written against that exact convention; verify no axis flip when
  authoring the cuboids.
- Lone-rail default is north–south by design (see Approach); revisit only if the
  playtest says a placed single rail should face the player.

## Execution plan (agreed)

Opus (this session) writes the spec + implementation plan and does the final diff
review; a **Sonnet 5** subagent at `high` effort implements against this spec with
`check.sh` as the gate. Fable not used (task is well-specified with a test gate).

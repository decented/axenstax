# Building-detail blocks — quick spec + the block-shape foundation finding

**Status**: PARTIAL BUILD 2026-06-16 (worktree `worktree-alpha-qol-building-blocks`, goal
`2026-06-16-alpha-qol-and-building-blocks`). Backlog **#30** (audit-derived) + absorbs **#27** (vertical
slabs) + would unblock **#29** (double doors).

**Decision-light** per the goal ("make sensible defaults, no owner ping"). This spec was written
*after* a code-reality check that contradicts the audit's "Effort: M (S each)" estimate — so it scopes
honestly: ship the foundation-free blocks now, and gate the rest behind a real foundation.

---

## The finding (why most of #30 is NOT "small each")

A thorough exploration of the engine (2026-06-16) found that **doors, trapdoors, stairs, slabs,
fence gates, walls, and glass panes all require three foundations the engine does not have**:

1. **Per-block orientation / state storage.** A chunk is `chunk.rs:20` `blocks: [BlockId; 4096]` —
   a bare u16 per block, **no parallel state array, no metadata nibble**. Doors need facing +
   open/closed + hinge; stairs need facing + half; slabs need top/bottom. None of that can be stored.
   (The `World.block_entities` position-keyed map — `world.rs:211` — holds *inventory/cooking* state for
   chests/furnaces/etc., not orientation, and is heavyweight for a per-block nibble.)
2. **Per-block collision shapes.** Collision (`physics.rs:238 resolve_collisions`) reads only the
   binary `BlockDef.solid` via `registry.is_solid` — there is no per-block AABB. A half-height slab
   would collide as a full cube or (if marked non-solid) be walked through.
3. **Custom sub-cube meshing.** The mesher (`mesh.rs`) is greedy full-cube + a single hardcoded
   small-cube AABB fallback (`non_solid_shape_for`, `mesh.rs:373`) for non-solid décor (torches,
   plants, rails). There is **no per-block multi-primitive geometry** (stairs = wedges, panes = thin
   quads, doors = panels) and **no rotation**.

`BlockDef` (`block.rs:1129`) has `name/solid/transparent/gravity/color/tex_{top,bottom,side}` — and
**no shape/collision/orientation fields**. The existing `FENCE_POST` blocks are explicit full-cube
*placeholders* (the spec there defers "richer fences, gates" to v2).

**Conclusion:** the orientation+collision+mesh substrate is an **own foundation, comparable in size to
#31's neighbour-update/scheduled-tick architecture** — which this goal explicitly out-scopes as XL.
Shipping doors/slabs/stairs as full-cube placeholders would be "cards, not concrete" (CLAUDE.md), so
they are **deferred to that foundation**, not faked.

---

## Built now (foundation-free, this goal)

These need none of the three missing pillars — they ride the existing `non_solid_shape_for` décor path
(+ for ladders, a `climbable` flag mirroring the existing `is_water`/swim branch):

- **Ladder** (`LADDER`, id 261) — climbable. New `BlockDef.climbable` flag; `physics.rs` gains a climb
  branch mirroring the swim branch (`physics.rs:112`): while the player's feet overlap a climbable
  block, gravity is cancelled and jump/sneak climb up/down, slow descent otherwise. Realises the
  Spec 05 §1.6 "ladder/vine climbing" promise. Climb-velocity decision is a **pure unit-tested
  helper**; the feel is a playtest tune. Rendered as a thin back-panel via `non_solid_shape_for`
  (centred — per-wall orientation waits on the orientation pillar; a sensible default).
- **Carpet** (`CARPET`, id 262) — a thin (≈1/16) decorative top layer, non-solid (you walk on the
  block beneath, so no collision change). Pure `non_solid_shape_for` entry.

Both reuse existing texture layers for now (functional; a dedicated procedural texture is a visual
polish follow-up — texture appearance is a playtest item regardless).

## Deferred to the block-shape foundation (own spec/goal)

> **✅ RESOLVED 2026-06-19.** The foundation + this whole deferred list shipped to
> `main` under `docs/goals/2026-06-19-solo-buildout-wave-2.md`. Self-contained
> spec: **[Block-shape foundation (F1)](2026-06-19-block-shape-foundation.md)**.
> One design change from the recommendation below: pillar 1 reused the **sparse
> Spec 48 meta byte** (`World.block_meta` map) rather than a dense parallel
> `[u8; 4096]` array + save bump — lighter, since most cells carry zero meta. The
> rest landed as phased: slabs/stairs (#27 vertical) → doors/trapdoors/gates (#29
> double doors) → walls/panes/signs/frames + flush signal-input shapes.

doors · trapdoors · **stairs** · **slabs (incl. #27 vertical slabs)** · fence gates · walls · glass
panes · signs · item frames. Each needs ≥1 of the three pillars. **#29 (double doors) is therefore
also deferred** — it presupposes doors. Recommended phasing for the foundation goal:
1. per-block state (parallel `[u8; 4096]` orientation/variant nibble in `Chunk` + save-format bump),
2. per-block collision AABB (`registry.collision_aabb(id)` + `physics` swept-shape resolve),
3. custom-geometry mesh seam (`RenderShape` per block: FullCube / Slab(half) / Stairs / Pane / Cross),
then the blocks fall out cheaply on top.

---

## Acceptance (built-now subset)

- `LADDER` + `CARPET` registered, placeable, save/load round-trip (block ids already persist in the
  chunk — no save change). Ladder climb-velocity helper unit-tested; carpet is non-solid décor.
- `./check.sh` green. Spec 02 (block registry) + Spec 05 §1.6 (ladder climbing) updated.
- **No build** of the deferred blocks (would be non-functional placeholders) — captured here for the
  foundation goal instead.

## Memory-rule check
- **Concrete, not cards**: ships only blocks that actually behave; refuses placeholder doors/slabs.
- **Spec maintenance**: records the foundation finding so the next rebuild doesn't re-discover it.

# Block-shape foundation (F1) + building-detail family

**Status**: ✅ **DELIVERED 2026-06-19** on `main` (Wave 1 + Wave 2a/2b/2c of the
goal `docs/goals/2026-06-19-solo-buildout-wave-2.md`). `check.sh` ALL GREEN
(3162 engine tests at the time of writing). Commits: `d949a90d` (F1 pure
substrate) · `537804c2` (F1 collision resolver) · `5326c8f6` (fence gate) ·
`83622c13` (trapdoor + toggle dispatch) · `4335a902` (panes + bars) · `378cd4d0`
(2-tall door + #29 double doors) · `6896b376` (wall + connecting geometry) ·
`75582e49` (flush lever/button/plate) · `2d39bdb1` (editable signs) · `d0e44606`
(item frames).

> **Authored 2026-06-21 (retroactive).** This is the self-contained foundation
> spec the queue kept pointing at ("the block-shape foundation — own spec/goal",
> `2026-06-16-building-detail-blocks.md` §"Deferred"). The work shipped under a
> *goal* doc before the foundation *spec* was written; per the CLAUDE.md
> spec-maintenance rule ("the spec is what survives a rebuild") this records the
> delivered design so the next rebuild reproduces it rather than re-discovering
> the finding.

---

## TL;DR

The engine stored **one `u16` block id per cell** — no orientation, no per-block
collision shape, no sub-cube geometry. That blocked the entire building-detail
family (doors, stairs, slabs, fence gates, trapdoors, walls, glass panes, iron
bars, signs, item frames) and two Minecraft-gap items (#27 vertical slabs, #29
double doors). This foundation adds the three missing pillars **as pure,
match-on-registry functions** — no new `BlockDef` fields, the ~280 existing block
literals stay full-cube by default — and the whole family falls out cheaply on
top.

## Context pointers

- The finding that scoped this: `docs/foundations/2026-06-16-building-detail-blocks.md`
  (§"The finding" + §"Deferred to the block-shape foundation").
- The delivery goal: `docs/goals/2026-06-19-solo-buildout-wave-2.md` (Waves 1–2c).
- Source of truth in code: `game/engine/src/block_shape.rs` (pure geometry +
  placement/toggle helpers, 30+ unit tests) and `game/engine/src/meta.rs` (the
  per-block meta byte, originally from Spec 48 Electricity).
- Specs updated alongside: `docs/spec/02-world-format.md` (storage + persistence),
  `docs/spec/03-rendering.md` (sub-cuboid meshing), `docs/spec/05-gameplay-systems.md`
  (placement, orientation, open/close interaction, connecting shapes, signs/frames).

## The three pillars

1. **Per-block orientation / state — the meta byte (reused, not re-invented).**
   The building-detail finding proposed a parallel `[u8; 4096]` array in `Chunk`
   plus a save-format bump. The delivered design instead **reuses the Spec 48
   (Electricity) meta byte** (`meta.rs`): a sparse `World.block_meta:
   AHashMap<(i32,i32,i32), u8>` (absent ⇒ `0`, the plain-block default —
   `world.rs:510`). Layout: `facing` = bits 0–2 (Down/Up/N/S/W/E), `state` =
   bits 3–4 (2-bit, device-specific: open/closed, upside-down, top/bottom mount),
   `aux` = bits 5–7 (3-bit: door hinge side, …). Sparse beats a dense array —
   the overwhelming majority of cells carry zero meta, so nothing is stored for
   them, and there is no per-chunk allocation. `set_meta`/`block_meta_at`
   (`world.rs:1147`, `:1152`) are the accessors; `edit_block` (`world.rs:1188`)
   writes id + meta together.

2. **Per-block collision AABB(s).** `block_shape::collision_aabbs(shape, meta) ->
   Vec<Aabb>` returns the real boxes a shaped block occupies, in block-local
   `[0,1]³`. Physics resolves the player against those boxes (`physics.rs`
   overlap helpers take `&block_shape::Aabb`); `FullCube` returns a single unit
   box and is short-circuited so ordinary blocks keep the fast boolean
   `is_solid` path (regression-locked by
   `physics::tests::full_cube_collision_boxes_match_legacy_is_solid`). A slab is
   half-height, a stair is two boxes, an open gate is *empty* (passable), a
   closed gate is a thin blocking panel, signs/frames/buttons/levers/plates are
   pass-through (empty).

3. **Custom sub-cuboid render geometry.** `block_shape::render_cuboids(shape,
   meta) -> Vec<Aabb>`. The mesher (`mesh.rs:539`) checks `shape_of(id)`; if it
   isn't `FullCube` it emits one textured box per cuboid (faces textured by
   direction: +Y→`tex_top`, −Y→`tex_bottom`, sides→`tex_side`) instead of a
   merged greedy quad. Render boxes are kept separate from collision boxes
   because some shapes draw differently from how they collide (an open fence
   gate renders two end posts but collides as nothing; a pane renders thin but
   you can't walk through its post).

## The shape registry

`block_shape::shape_of(id) -> BlockShape` is the single id→shape binding, a match
in the engine idiom (`light_emission`, `camera_occlusion`) — **not** a `BlockDef`
field. `BlockShape` variants and their blocks:

| Shape | Blocks (ids) | Notes |
|---|---|---|
| `FullCube` | everything else (~280) | default; never reaches this module |
| `Slab` | `STONE_SLAB` (283) | 6 placements via one `facing` field — Up/Down horizontal + **N/S/W/E vertical (#27)** |
| `Stairs` | `STONE_STAIRS` (284) | two boxes (slab + step); `state` bit = upside-down |
| `FenceGate` | `OAK_FENCE_GATE` (285) | open/closed; open ⇒ passable |
| `Trapdoor` | `OAK_TRAPDOOR` (286) | open/closed × top/bottom mount |
| `Pane` | `GLASS_PANE` (287), `IRON_BARS` (288) | **connecting** (see below) |
| `Door` | `OAK_DOOR` (289) | thin panel, **2 cells tall**; `state` bit 1 = top half, `aux` bit 0 = hinge; **#29 double doors** open together |
| `Wall` | `COBBLESTONE_WALL` (290) | **connecting**; 0.5 post + 0.25 arms |
| `Sign` | `OAK_SIGN` (291) | post + board; text in `BlockEntityData::Sign`; pass-through |
| `ItemFrame` | `ITEM_FRAME` (292) | flush plate; framed item in `BlockEntityData::ItemFrame`; pass-through |
| `Button` / `Lever` / `PressurePlate` | `BUTTON` (273) / `LEVER` (272) / `PRESSURE_PLATE` (274) | flush **shapes only** — power behaviour is owned by Spec 48 Electricity |

### Connecting shapes (Wall, Pane)

Walls and panes restyle themselves from their **live neighbours** with **zero
persisted state**. The mesher and physics call `world.connection_mask(x, y, z,
shape, registry)` (`world.rs:1430`) to compute a 4-bit `CONN_{N,S,W,E}` nibble
(via `block_shape::connects` — walls join walls + full-cube solids; panes/bars
join panes/bars + full-cube solids), then pass that nibble where ordinary shapes
pass their stored meta. Placing or breaking a neighbour re-derives the mask on
the next mesh — no side-state to migrate, no save bump for connections.

## Placement & interaction

- **Place-time orientation** (`game_loop.rs:~9363`, gated on
  `block_shape::is_shaped`): slabs use `slab_placement_facing(face_normal,
  hit_y_frac)` (click top→bottom slab, bottom→top slab, side-middle→vertical
  slab, side high/low→horizontal); stairs use `stairs_placement(fwd_x, fwd_z,
  face_normal)` (face the camera, bottom-face click ⇒ upside-down); gates /
  trapdoors / doors / signs use `facing_from_forward`; buttons / levers / frames
  use `facing_from_face_normal` (mount on the clicked face); connecting shapes
  place with meta `0`.
- **Open/close** (`game_loop.rs:~7696`, gated on `block_shape::is_toggleable` =
  FenceGate | Trapdoor | Door): right-click flips `state` bit 0 via
  `toggled_open`. Doors toggle **both halves** (the partner cell is found via
  `door_is_top`); adjacent mirror-hinged doors toggle together (#29).
- **Signs**: place + right-click open an egui text editor; the text renders
  read-on-look in the HUD. **Item frames**: right-click to mount/rotate; the
  framed block renders as a small cube off the plate (`item_frame_item_cube`).
- Levers are deliberately **excluded** from `is_toggleable` — the Spec 48
  Electricity right-click handler owns the lever toggle so the power lever isn't
  hijacked by the generic openable path.

## Persistence & networking

- **Save**: `WorldSave.block_meta: Vec<(i32,i32,i32,u8)>` (`save.rs:232`),
  bincode-stable like `village_treasuries`; sparse, so a world with no shaped
  blocks writes nothing. (Introduced with Spec 48; the shaped blocks ride it.)
- **Protocol** (v51): `BlockChange` carries `meta: u8`
  (`protocol.rs:331`; `BlockChange::with_meta`); the chunk stream carries the
  sparse per-block metadata so a joining client sees correct orientations and
  open/closed states.

## What's deferred (intentionally, not missing)

- **Power behaviour** of button / pressure-plate (and the rest of the
  signal-input semantics) belongs to the **Spec 48 Electricity** arc — F1 ships
  only their *shapes* (flush geometry, pass-through collision). Lever toggle is
  already wired through Electricity.
- **More wood species** for doors / fence gates / trapdoors / signs (oak only at
  v1), and **more materials** for slabs / stairs (stone only at v1). The shape
  machinery is species-agnostic; adding a variant is a block id + recipe + a
  `shape_of` arm. Captured for a follow-up content wave.
- **Feel tuning** (collision step-up onto slabs/stairs, door swing arc, sign text
  ergonomics) — the Axolittle playtest item; the geometry is exact, the *feel* is
  a number-tune.

## Acceptance (all met)

- All shaped blocks registered (ids 272–274, 283–292), placeable with correct
  orientation, craftable, save/load round-trip incl. meta, protocol-synced.
- Collision: stand *on* slabs/stairs at the right height; walk *through* an open
  door/gate; can't pass a closed door/gate/wall; vertical slabs in all six
  orientations. Full-cube parity regression-locked.
- Meshing: each shape draws its sub-cuboids; connecting walls/panes join live
  neighbours.
- `block_shape.rs` carries 30+ pure unit tests; `./check.sh` green.

## Memory-rule check

- **Concrete, not cards**: real per-block state + collision + mesh, no
  placeholder full-cube doors. Reuses the existing meta byte rather than adding a
  parallel array — one storage mechanism, not two.
- **Spec maintenance**: this doc + the Spec 02/03/05 updates record the delivered
  design so the foundation survives a rebuild. The building-detail finding is now
  resolved (its deferred list is fully shipped).
- **UK English** throughout.

# Blueprint Column-Capture — build *on* the paper, stamp it, capture what's connected

**Status: BUILT — engine tests + WASM build green. Awaiting Axolittle playtest.** Reworks the
*capture semantics* of the shipped paper-thin blueprint system
(`docs/foundations/2026-06-04-blueprint-face-attachment.md`). The paper-thin lay / render /
peel / develop / persist machinery is **done and stays**; only the **capture algorithm** and
the **lock-in trigger** change, plus a new **Drafting Stamp** tool.

**Why this rework:** the shipped capture inherited the old Spec-24 "flood the air, grab the
surrounding shell, refuse if anything sits on the paper" model. The owner's mental model — and
the one we're committing to — is a **1:1 scale drawing**: lay paper as your footprint, **build
directly on it**, then **stamp it** to capture what you built. The old model actively *forbids*
building on the paper (`check_flat_ground` requires air above every tile); this spec removes
that and replaces the volume sweep with a **connectivity flood** so capture grabs exactly the
structure you built and nothing else.

---

## The model (owner + Axo, 2026-06-04)

1. **Lay paper** on a cleared floor → paper-thin `BlueprintBlank` attachments (already built).
2. **Build directly on it**, 1:1, connected to the ground. Floor, walls, furniture, multi-storey
   — anything, up to the height cap.
3. **Stamp it** with the **Drafting Stamp** tool → captures the connected structure sitting over
   the paper into a portable `Item::Plan`. Your real build stays standing; the paper is consumed.
4. Creative = finished plan instantly; survival = Latent plan that develops in sunlight (unchanged).

## The capture rule (the heart of this spec)

> **Seed from every block sitting directly on a paper tile. Flood through solid blocks
> (face-to-face), staying in the columns above the paper footprint and under the height cap.
> Everything the flood reaches is the plan. Air breaks the flood.**

This single rule produces every behaviour the owner specified:

| Case | Result | Why |
|------|--------|-----|
| Your connected build over the paper | ✅ captured | Solid path from a paper-seated block up through walls/roof |
| Overhanging tree above the paper | ❌ excluded | Air gap between it and your build → flood can't cross |
| Building inside a cave (ceiling above) | ❌ ceiling excluded | You leave an air gap up to the ceiling → flood stops at air |
| Hard against a neighbour's wall, no paper under it | ❌ their wall excluded | Outside the paper footprint → never seeded/flooded into |
| Overhang/eave past the paper edge | ❌ excluded | Outside the footprint — lay paper under it to include it |
| Open floor (no floor blocks laid) — gazebo/pergola | ✅ structure captured, floor stays open | Floor cells are air → not captured; roof comes via the pillars' solid path. Rebuilt on grass → grass shows through |

"There has to be an air gap if it's above it" — the **gap is the protection**: build up to the
cave ceiling with no gap and it would join; leave the gap and it won't.

## The Drafting Stamp (lock-in trigger)

Because the build now **covers** the paper, you can't click the paper through your own walls.
The trigger is a craftable **Drafting Stamp** tool:

- **Use:** right-click **any block of your build** (or the paper directly) with the Stamp.
- **Resolution:** from the clicked block `(x, y, z)`, scan **straight down the column** until a
  floor block carrying a `BlueprintBlank` top-attachment is found at `(x, py, z)` (cap the scan
  at `MAX_HEIGHT`). That tile anchors the capture; `plan::capture` runs the footprint flood-fill
  + the connectivity volume from there.
- **No paper under the clicked column →** no-op + a "no plan here" feedback toast.
- This needs no exposed "tab" and doesn't overload empty-hand right-click.

The Stamp is a normal tool item (registry entry + texture + recipe). It is **not consumed** by
use (it's a reusable tool). Proposed recipe (owner to confirm at build time): `Blueprint Paper +
Iron Nugget` or similar cheap craft — a one-line recipe-table entry, not a blocker.

---

## What changes vs. the shipped system

| Layer | Disposition |
|-------|-------------|
| `FaceAttachment` store, paper-thin render, persist, peel, develop tick, mode-split | **KEEP — unchanged** |
| `plan::flood_fill_tiles` (footprint from `BlueprintBlank` attachments, 4-connected) | **KEEP — unchanged** |
| `plan::check_flat_ground` | **REMOVE** — you build on the paper now; no "air above" requirement. (Drop the `CaptureRefusal::NotFlat` variant or leave it unused.) |
| `plan::flood_fill_volume` (air-flood + 1-ring shell) | **REPLACE** with `capture_connected_volume` (connectivity flood through solid, footprint-XZ-constrained) |
| `plan::capture` | **REWIRE** to the new volume fn; keep the footprint/height envelope guards (`MAX_FOOTPRINT`/`MAX_HEIGHT` = 32) |
| `plan::commit_capture` | **KEEP** — already consumes the paper attachments and leaves the floor (R3b fix); the real build is never touched |
| Empty-hand capture trigger in `game_loop.rs` | **REMOVE** — replaced by the Drafting Stamp trigger |
| Drafting Stamp item (registry, texture, recipe, use-hook) | **NEW** |

## Algorithm detail (`plan.rs`)

```
fn capture_connected_volume(world, tiles: &[(i32,i32,i32)]) -> Vec<(i32,i32,i32)>:
    footprint_xz = { (x, z) for (x, _, z) in tiles }        // columns above paper
    base_y       = tiles[0].1                               // all tiles share Y
    seeds        = [ (x, base_y+1, z)
                     for (x, _, z) in tiles
                     if world.get_block(x, base_y+1, z) != AIR ]   // blocks sitting on paper
    BFS from seeds, 6-connected (face neighbours), visiting cell (nx, ny, nz) iff:
        (nx, nz) in footprint_xz                            // stay over the paper
        and base_y+1 <= ny <= base_y + MAX_HEIGHT           // height cap
        and world.get_block(nx, ny, nz) != AIR              // flood through SOLID only
    captured = every visited non-air cell
    return captured
```

- **Relative coords** (unchanged from today): `min_x/min_z` from the captured set, `min_y =
  base_y + 1`; each `CapturedCell { rx, ry, rz, block_id }`.
- **Empty result →** `CaptureRefusal::EmptyVolume` (you stamped a footprint with nothing built on it).
- **Envelope guards →** footprint bounding box ≤ `MAX_FOOTPRINT`, height ≤ `MAX_HEIGHT`
  (`CaptureRefusal::TooLarge`), as today.
- **6-connected** (face neighbours). Diagonal-only connections are not captured — an acceptable
  edge case (real builds are face-connected); note it, don't solve it.

## Edge cases / decisions

- **Floor or no floor:** the player's choice falls straight out of the rule (build a floor → it's
  captured; leave it open → floorless plan → ground shows on rebuild). Drives gazebos/pergolas.
- **Stray block over the paper but disconnected** (floating lantern with an air gap): not captured
  (flood can't cross the gap). To capture it, connect it solidly to the build.
- **Too tall (> MAX_HEIGHT):** refuse with a clear toast (or raise the cap — owner call).
- **Two builds side-by-side:** still separate them with a 1-tile paper gap, or stamp one before
  laying the next (stamping consumes the paper). Auto-boundaries remain a later feature.
- **Stamp on a column with no paper:** no-op + feedback. **Stamp resolves the first paper tile
  found scanning down** the clicked column.

## Out of scope / deferred (unchanged from the parent spec)

- Old-save migration of legacy `LATENT_PRINT`/`CYANOTYPE_PRINT`/`BLUEPRINT_PAPER` cubes (Phase G).
- Provenance integrity-ledger snapshot (Phase F); `authored_in` (creative/survival) IS stamped.
- Merging two captured plans; auto-separating adjacent footprints; murals/chandeliers on walls/ceilings.
- Wall/ceiling capture (floor-only).

## Testing posture

- **Pure-function (plan.rs):** the connectivity flood is the crown jewel — unit-test every row of
  the capture-rule table (connected build captured; cave ceiling excluded by air gap; overhanging
  block excluded; adjacent non-papered wall excluded; gazebo open-floor captured-without-floor;
  multi-storey via internal connection; too-tall/too-wide refusals; empty-volume refusal).
- **Stamp resolution:** pure helper `paper_tile_under(world, clicked) -> Option<(i32,i32,i32)>`
  (scan-down) unit-tested; the right-click wiring is playtest-boundary.
- **Gate:** `./check.sh` all-green incl. WASM + bundle size.
- **Playtest (Axo):** does building-on-the-paper + stamp *feel* like a 1:1 drawing; do the
  exclusions behave intuitively in real builds.

## Memory-rule check

- `feedback_merge_to_main_preauthorised` — merge when `check.sh` green.
- `feedback_uk_english_naming` — UK English ("colour", "draughting", "neighbour").
- `feedback_autonomy_to_playtest_boundary` — build to the playtest boundary, stop cleanly.
- Spec maintenance (`CLAUDE.md`) — fold the column-capture rule + Drafting Stamp into Spec 05
  (block interaction) + the blueprint foundation docs once shipped.

---

## Implementation outcome (2026-06-04)

**Status: BUILT — engine tests + WASM build green. Awaiting Axolittle playtest.**

### Capture algorithm

`plan::flood_fill_volume` (the old air-flood + 1-ring shell sweep) and
`plan::check_flat_ground` (the "air above every tile" guard) were **removed** and
replaced by `plan::capture_connected_volume`. `plan::capture` was rewired to call
this new function.

`plan::flood_fill_tiles` (the 4-connected footprint finder that reads `BlueprintBlank`
attachments) is **unchanged**.

`capture_connected_volume` seeds from every solid block sitting directly on a paper tile
(`base_y + 1`), then floods 6-connected (face neighbours) through **solid blocks only**,
constrained to the XZ columns above the paper footprint and the height cap (`MAX_HEIGHT`
= 32). Air breaks the flood. The footprint/height envelope guards (`MAX_FOOTPRINT` /
`MAX_HEIGHT` = 32, `CaptureRefusal::TooLarge`) and the `EmptyVolume` / `Disconnected`
refusals are unchanged. `CaptureRefusal::NotFlat` was kept (still used by `capture_art`,
the wall/cyanotype path) but `capture` no longer returns it.

### Shipped capture-rule table

| Case | Result | Why |
|------|--------|-----|
| Connected build over the paper | ✅ captured | Solid path from a paper-seated block up through walls/roof |
| Cave ceiling above an air gap | ❌ excluded | Air gap between your build and the ceiling — flood stops at air |
| Overhanging tree / stray disconnected block | ❌ excluded | Air gap to the rest of your build |
| Neighbour's wall over non-papered ground | ❌ excluded | Outside the paper footprint — never seeded or flooded into |
| Open-floor gazebo/pergola (no floor blocks) | ✅ structure captured; floor stays open | Floor cells are air → not captured; roof reaches via pillars' solid path |

6-connected only; diagonal-only joins are not captured (acceptable edge case).

### Drafting Stamp

`ToolType::DraftingStamp` was added. Right-clicking any block of the build (or the
paper directly) with the Stamp calls `blueprint_attach::paper_tile_under`, which scans
straight down the clicked column to find the floor tile carrying a `BlueprintBlank`
top-attachment. If found, `plan::capture` runs from that tile and opens the existing
inspect/naming/licence dialog; `commit_capture` is unchanged (consumes the
`BlueprintBlank` attachments, preserves the floor blocks, broadcasts no AIR). If no
paper is found under the column, a "no blueprint here" feedback toast is shown and
nothing happens.

The Stamp is **not consumed** on use (no durability decrement — it is a reusable tool).
Recipe: Iron Ingot over Blueprint Paper (**v1 placeholder, pending owner
confirmation**). Hotbar abbreviation: "Ds". `/give drafting_stamp` works.
`PROTOCOL_VERSION` was bumped to 43 for the appended `ToolType` variant.

### Lock-in trigger change

The **empty-hand right-click** on the papered floor trigger was **removed** from
`game_loop.rs`. The Drafting Stamp is now the sole lock-in trigger.

### Unchanged from the parent spec

The `FaceAttachment` store, paper-thin render, save/load persistence, two-stage peel,
`tick_develop_attachments` sun-develop, and the **creative-instant / survival-Latent**
mode split are all unchanged. `authored_in` (creative/survival) continues to be stamped
on every captured plan.

### Still deferred

- **Phase G** — migration of old saved full-cube `LATENT_PRINT` (158) /
  `CYANOTYPE_PRINT` (172) / `BLUEPRINT_PAPER` (56) blocks to face attachments.
- **Phase F (full)** — provenance integrity-ledger snapshot (`integrity: IntegrityTag`,
  `trust: TrustLevel`); only `authored_in` is stamped (tier-1 self-asserted).
- Plan-merging; auto-separating adjacent footprints.
- Wall/ceiling capture (floor-only for now).
- Selling/economy for blueprints — blocked on the parked settlement model.

### Playtest boundary

The capture logic and Stamp wiring are engine-verified. The **feel** — does building
directly on the paper + stamping read as a 1:1 drawing; do the exclusions behave
intuitively in real builds — is the **Axolittle playtest boundary** and has not been
solo-verified.

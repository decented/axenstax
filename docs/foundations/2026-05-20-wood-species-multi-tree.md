# Wood Species — Spec 28b

**Status:** DELIVERED as of 2026-05-21 on main. All 6 wood species (Oak — existing — plus Birch / Spruce / Jungle / Acacia / DarkOak) shipped: 15 new BlockIds (66-80), `WoodSpecies` enum + helpers in `block.rs`, per-species tree shapes in `tree_shapes.rs`, species-neutral planks recipe (any plank species works in crafting), per-species sapling MaterialIds, biome-driven tree distribution via `assign_biome_whittaker`. Phase 11 (Axolittle playtest of feel) is the remaining gate.
**Branch (when building):** `feat/spec-28b-woods` off `main`.
**Trigger:** Sub-foundation of [Spec 28 Minecraft-parity content surface](2026-05-20-minecraft-parity-content-surface.md) §2. Depends on 28a Biomes for tree distribution; can independently land the block + recipe layer.

---

## TL;DR

Six wood species: **Oak** (existing), **Birch**, **Spruce**, **Jungle**, **Acacia**, **Dark Oak**. Each adds three new blocks (log, leaves, planks) and reuses the existing Tree-Mine workstation + Drying-Rack lifecycle (GreenLog → SeasonedLog → KilnDriedLog) tier-neutrally. Planks of any species behave identically for crafting recipes — what differs is colour + biome distribution + leaves visual.

Crafting parity rule: any recipe that needs "planks" accepts planks of any species. A pickaxe handle made from Birch planks is mechanically identical to Oak. Cosmetic only.

**Scope:** ~1,400 LOC + ~18 textures (3 per species × 6 species) across 11 phases.

---

## Why this lives here

- Single-wood worlds feel artificial. Even before biomes, the cabinet of "wood you can build with" should look like Minecraft's: warm pale-yellow Birch, dark Spruce, golden Acacia, deep-brown Dark Oak.
- The existing wood economy (Tree-Mine → Drying-Rack → planks → tools) is species-agnostic by design (GreenLog/SeasonedLog/KilnDriedLog don't carry species). Adding species is additive — no rewrites.
- 28b depends on 28a for biome-driven tree distribution. The block + recipe layer (Phases 2-5) can ship before 28a.
- Cross-game lift: per-species tree placement + species-neutral plank recipe is engine-generic.

---

## Context pointers

### Existing surfaces

- `block.rs` — OAK_LOG=7, OAK_LEAVES=8, OAK_PLANKS=9 already in place. Append birch/spruce/jungle/acacia/dark_oak at the next free IDs (post-PURE_DEEPSLATE_FAT=63, so starting at 66).
- `block.rs::mine_drop` — log drops `Item::Material(MaterialId::GreenLog)` (tier-neutral). Confirm this stays true for the new species.
- `crafting.rs` — planks recipe is `MaterialId::SeasonedLog → OAK_PLANKS`. Generalise to `(MaterialId::SeasonedLog, species: WoodSpecies) → species_planks(species)`.
- `chunk.rs` (tree gen) — current `place_tree(x, y, z)` is single-species. Refactor to `place_tree(species, x, y, z)`.
- `drying_rack.rs` — currently species-agnostic; ensure no regression after the BlockId additions.

### New modules / types

- `WoodSpecies` enum (in `block.rs` or `wood.rs`) — Oak, Birch, Spruce, Jungle, Acacia, DarkOak.
- `tree_shapes.rs` (new) — per-species tree shape: Spruce conical, Acacia umbrella, Jungle tall+vine-ready, Dark Oak 2x2 trunk. Pure functions returning `Vec<(dx,dy,dz, BlockId)>` offsets.

### Related specs

- Spec 02 §3.5 — tree placement; the per-species shapes plug in here.
- 28a Biomes §3 — biome → tree species pool.

### Memory pointers

- uk english naming — UK English; "Coloured" planks not "Colored".

---

## Phasing

| # | Phase | Files | LOC | Solo? |
|---|---|---|---|---|
| 1 | **This spec** | this doc | — | — |
| 2 | Add 15 new BlockIds (5 species × {log, leaves, planks}) — Birch, Spruce, Jungle, Acacia, Dark Oak | `block.rs` | ~250 | ✓ |
| 3 | `WoodSpecies` enum + `planks_block_for(species) -> BlockId` + `log_block_for(species)` + `leaves_block_for(species)` | `block.rs` | ~80 | ✓ |
| 4 | Generalise planks recipe — any seasoned log + (cosmetic species selector via UI) → 4 of that species' planks | `crafting.rs` | ~80 | ✓ |
| 5 | Test: drying-rack pipeline + planks crafting works for every species | `*.rs::tests` | ~120 | ✓ |
| 6 | Per-species tree shapes — Spruce (conical), Acacia (umbrella), Jungle (tall), Dark Oak (2x2 trunk), Birch (oak-like taller) | `tree_shapes.rs` | ~300 | ✓ build, ⚠ playtest |
| 7 | World-gen wires species selection — call `tree_shapes::place_tree(species, x, y, z)` reading from 28a biome blend | `chunk.rs` | ~150 | ⚠ playtest (needs 28a) |
| 8 | Mine_drop coverage — every species' log drops GreenLog; every species' leaves drops sapling-of-species at 1-2% (sapling materials added) | `block.rs::mine_drop` | ~100 | ✓ |
| 9 | Saplings as a `MaterialId` per species — OakSapling, BirchSapling, etc. Right-click on dirt plants. | `item.rs`, `block.rs` | ~150 | ⚠ playtest |
| 10 | Tests — block reg sane, recipe generalisation, sapling planting, tree-shape determinism | `*.rs::tests` | ~120 | ✓ |
| 11 | Axolittle playtest — chop one of each species, dry, plank, build a 2x3 hut with each colour | — | — | playtest gate |

**Phases 2-5 + 8 + 10 are solo-shippable today.** Phases 6-7-9-11 need playtest.

---

## §2 — BlockIds

```rust
// Wood species expansion (Spec 28b). IDs 66-80 (15 new blocks).
pub const BIRCH_LOG: BlockId = 66;
pub const BIRCH_LEAVES: BlockId = 67;
pub const BIRCH_PLANKS: BlockId = 68;
pub const SPRUCE_LOG: BlockId = 69;
pub const SPRUCE_LEAVES: BlockId = 70;
pub const SPRUCE_PLANKS: BlockId = 71;
pub const JUNGLE_LOG: BlockId = 72;
pub const JUNGLE_LEAVES: BlockId = 73;
pub const JUNGLE_PLANKS: BlockId = 74;
pub const ACACIA_LOG: BlockId = 75;
pub const ACACIA_LEAVES: BlockId = 76;
pub const ACACIA_PLANKS: BlockId = 77;
pub const DARK_OAK_LOG: BlockId = 78;
pub const DARK_OAK_LEAVES: BlockId = 79;
pub const DARK_OAK_PLANKS: BlockId = 80;
```

Each gets a `BlockDef` with appropriate colour (per species palette) and the same `solid`/`transparent`/`gravity` semantics as Oak's.

Palette (approximate — final values from Axolittle's preview):
- Birch logs: warm white-cream (0.95, 0.92, 0.82) with dark grey horizontal flecks (texture-only post-alpha).
- Spruce logs: deep red-brown (0.42, 0.28, 0.18).
- Jungle logs: green-tinted ochre (0.55, 0.45, 0.25).
- Acacia logs: golden orange (0.85, 0.55, 0.20).
- Dark Oak logs: near-black brown (0.20, 0.12, 0.08).

---

## §3 — WoodSpecies enum

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WoodSpecies {
    Oak,
    Birch,
    Spruce,
    Jungle,
    Acacia,
    DarkOak,
}

pub fn planks_block_for(s: WoodSpecies) -> BlockId { match s { ... } }
pub fn log_block_for(s: WoodSpecies) -> BlockId { match s { ... } }
pub fn leaves_block_for(s: WoodSpecies) -> BlockId { match s { ... } }
pub fn sapling_material_for(s: WoodSpecies) -> MaterialId { match s { ... } }
pub const ALL_WOOD_SPECIES: &[WoodSpecies] = &[Oak, Birch, Spruce, Jungle, Acacia, DarkOak];
```

---

## §4 — Planks recipe

The existing planks recipe takes `MaterialId::SeasonedLog → 4 OAK_PLANKS`. Add a UI step in the crafting overlay where the player picks a species (radio or thumbnail row) before confirming the craft. Default = Oak. The recipe writes the species' planks block, not Oak's.

Materially, every recipe that consumes "planks" now matches on `crate::wood::is_any_planks(block_id)` rather than `block_id == OAK_PLANKS`. The list of recipes to audit: pickaxe handle (via crafting.rs), crafting table, drying rack, vendor block, plan tile.

---

## §6 — Tree shapes

Per-species shape generators. Each returns a Vec of `(dx, dy, dz, BlockId)` offsets from the trunk base. Shapes:

**Oak** (existing): 4-6 tall trunk, spherical leaves canopy.
**Birch**: 6-8 tall trunk, narrower canopy.
**Spruce**: 6-12 tall trunk, conical layered canopy (radius shrinks with height).
**Jungle**: 8-16 tall, thick trunk, leaves at top + occasional mid-trunk.
**Acacia**: 4-6 trunk + 1-2 horizontal branches → umbrella canopy.
**Dark Oak**: 6-8 tall, 2x2 trunk, canopy spreads ~3 blocks beyond trunk.

All shape generators are pure and seeded by a hash of (x, z, world_seed) so the same coordinates always produce the same tree.

---

## §11 — Playtest

- Find one of each species in the world (or `/tp` to known biome coordinates).
- Chop one, dry it, craft planks. Confirm planks colour matches the species.
- Build a 2x3 hut using each species' planks. Confirm walls are visually distinct.
- Replant — saplings on dirt grow back into the right species.

Questions:
- Are the species visually distinct enough? Or do Oak/Birch read as "the same brown"?
- Spruce-tree height feels right? Too short / too tall?
- Sapling drop rate (1-2%) — too sparse?

---

## Acceptance — sub-foundation 28b overall

- `./check.sh` ALL GREEN.
- Tests pass.
- Manual playtest succeeds for all six species.
- Player Guide page added (`woods.md`).
- Foundations README updated.

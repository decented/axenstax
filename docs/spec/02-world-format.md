# Spec 02 -- World Format and Storage

**Status**: Draft
**Date**: 2026-03-03
**Addendum**: `_audit-2026-04-18.md` — falling blocks is now a pure free
function (`falling_blocks.rs`) gated at 5 Hz per player, radius 16 blocks.
**Depends on**: ADR-001, ADR-002, Platform Overview

> **AS-BUILT (audit 2026-10-04).** The shipped world format differs from the production design below: chunks are a **flat `u16` array** (`chunk.rs`, no palette compression), saved one file per non-empty chunk under `worlds/<folder>/chunks/` plus a bincode `world.dat` (`save.rs`). There are **no region files and no CRC-32C**. The built Y range is **0..95 (`MAX_CHUNK_Y = 5`, six 16-block sections; `World::set_block` ignores `y < 0`)**, not -64..383. `WorldSave` has **52 fields** (`save.rs`; append-only invariant). Treat §2, §3.1, §4 and the integrity paragraphs as design targets, not description.

---

## 0. Scope

This specification defines how Axe'n'Stax represents, stores, generates, and manages
voxel world data. It covers the block registry, chunk binary format, world dimensions,
region-based disk I/O, world generation pipeline, lighting system, chunk lifecycle,
persistence strategy, and world metadata.

The target audience is engine implementors. Every byte layout, algorithm, and data
structure is specified precisely enough to write code against without guessing.

---

## 1. Block Registry

### 1.1 Namespaced Identifiers

Every block type has a unique **namespaced string ID** of the form:

```
namespace:path
```

Examples:

```
genesis:air
genesis:stone
genesis:oak_log
genesis:redstone_lamp
myplugin:rainbow_brick
```

Rules:
- `namespace` and `path` are lowercase ASCII `[a-z0-9_]`.
- Maximum combined length: 255 bytes (including the colon).
- The `genesis` namespace is reserved for built-in blocks.
- Plugin namespaces are derived from the plugin manifest name.

Namespaced IDs are the **canonical** identity of a block type. They are used in
configuration files, world generation templates, commands, and plugin APIs.

### 1.2 Numeric Block IDs

At runtime, each namespaced ID is assigned a **numeric block ID** for compact storage.

- **Type**: `u16` (0..65535).
- **Rationale**: 65536 block types is sufficient for any practical game. Minecraft
  survives with roughly ~900 block states mapped into a palette; even with aggressive
  modding, exceeding 65k distinct *types* is unrealistic. `u16` halves per-block memory
  compared to `u32` and is critical for chunk compactness.
- **ID 0** is permanently reserved for `genesis:air`.
- IDs 1..1023 are reserved for built-in `genesis:*` blocks.
- IDs 1024..65535 are available for plugins, assigned at world load time.

The mapping from namespaced ID to numeric ID is stored in the **Block ID Table** within
the world metadata (Section 9). This table is written on every save and read on every
load, so the numeric IDs can be reassigned if plugins change between sessions.

### 1.3 Block Properties (Static)

Each registered block type carries a fixed property record:

```rust
#[repr(C)]
struct BlockProperties {
    /// Namespaced string ID (index into string table).
    name_id: u32,

    /// Bit flags for boolean properties.
    flags: BlockFlags,

    /// Light emitted by this block (0..15).
    light_emission: u8,

    /// Light absorbed when light passes through (0..15, 15 = fully opaque).
    light_absorption: u8,

    /// Hardness for mining calculations (0 = instant break, u16::MAX = unbreakable).
    hardness: u16,

    /// Tool type required for efficient harvesting (enum index).
    preferred_tool: ToolType,

    /// Blast resistance for explosions.
    blast_resistance: u16,

    /// Walk speed multiplier when on top of this block (fixed-point 8.8, 256 = 1.0x).
    walk_speed_modifier: u16,

    /// Sound group for step/place/break sounds (index into sound table).
    sound_group: u16,

    /// Render model (enum: full_cube, cross, slab, stairs, fluid, custom, etc).
    render_model: RenderModel,
}

bitflags! {
    struct BlockFlags: u32 {
        const SOLID           = 1 << 0;   // Has collision geometry
        const TRANSPARENT     = 1 << 1;   // Allows light and visibility through
        const FLUID           = 1 << 2;   // Water, lava, etc. -- uses fluid simulation
        const HARVESTABLE     = 1 << 3;   // Can be broken and drops items
        const GRAVITY         = 1 << 4;   // Falls like sand/gravel
        const FLAMMABLE       = 1 << 5;   // Can catch fire
        const REPLACEABLE     = 1 << 6;   // Tall grass, flowers -- overwritten on place
        const TICKS_RANDOMLY  = 1 << 7;   // Receives random tick (crop growth, decay)
        const BLOCKS_MOVEMENT = 1 << 8;   // Blocks entity movement even if not solid
        const WATERLOGGABLE   = 1 << 9;   // Can coexist with water
        const CONNECTS_REDSTONE = 1 << 10; // Participates in signal network
        const HAS_BLOCK_ENTITY  = 1 << 11; // Associated data beyond state (chests, signs)
    }
}
```

Block properties are **immutable** after registration. They are loaded once at startup
from block definition files (TOML or plugin API) and indexed by numeric ID for O(1) lookup.

### 1.4 Block States

Beyond the static type, individual block instances can have **state**. Block state is
encoded as a small integer that packs multiple orthogonal fields.

Each block type declares its state schema at registration:

```rust
struct BlockStateSchema {
    /// Total number of distinct states for this block type.
    state_count: u16,

    /// Ordered list of state fields.
    fields: Vec<StateField>,
}

struct StateField {
    name: &'static str,        // e.g. "facing", "open", "waterlogged"
    variant_count: u8,         // Number of possible values (2..=16)
}
```

The state is packed into a **`u16` state index**. The combined `(block_type_id: u16,
state_index: u16)` forms a 32-bit **block state ID**:

```
Block State ID (u32):
  [15..0]  block_type_id  (u16)
  [31..16] state_index    (u16)
```

This gives us up to 65536 distinct states *per block type*, which is more than enough.
Most blocks have 0 additional state (state_index = 0).

Example: `genesis:oak_log` has states for axis (`x`, `y`, `z`) and stripped (`true`,
`false`):
- axis: 3 variants
- stripped: 2 variants
- Total: 6 states, stored as state_index 0..5.

Encoding order is defined by the field list, packed densely:

```
state_index = stripped * 3 + axis
```

The **global block state ID** is what actually gets stored in chunk data. The palette
(Section 2) maps palette indices to these 32-bit block state IDs.

> **⚠ Implementation status (2026-06-16): block state is the destination, NOT yet built.**
> The engine currently stores a **bare `BlockId` (u16) per cell** (`chunk.rs`
> `blocks: [BlockId; CHUNK_VOLUME]`) — there is no `state_index`, no `BlockStateSchema`,
> no parallel state array. Logs render axis-agnostically as a workaround (no stored axis).
> This `(type, state)` system is the **gate** for orientation-bearing building blocks —
> doors, trapdoors, stairs, **slabs (incl. vertical, #27)**, fence gates, walls, panes —
> which is why the 2026-06-16 building-detail-blocks pass (#30) shipped only the
> **foundation-free** blocks (`LADDER` 261, climbable; `CARPET` 262, decorative — both
> non-solid, rendered via the `mesh.rs` small-cube path, needing no per-block state) and
> deferred the rest to the block-shape foundation goal. Full finding +
> phasing: `docs/foundations/2026-06-16-building-detail-blocks.md`. Pairs with the
> matching gaps in collision (binary `solid` only) and meshing (greedy cube + single
> small-cube fallback, no sub-cube geometry).

### 1.5 Block Entities

Some blocks need more data than a state index can hold (e.g., chest inventories, sign
text, command block scripts). These blocks set the `HAS_BLOCK_ENTITY` flag.

Block entity data is stored **outside** the chunk's block array, in a
`HashMap<LocalBlockPos, BlockEntityData>` per chunk. `LocalBlockPos` is a packed
`(u8, u8, u8)` for the position within the chunk.

```rust
struct BlockEntityData {
    /// Type discriminant (matches block type).
    kind: u16,

    /// Opaque serialized payload. Each block entity type defines its own schema.
    data: Vec<u8>,
}
```

Block entities are serialized with the chunk but are NOT part of the palette-compressed
block array. This keeps the hot path (block lookups) fast and cache-friendly.

> **Implemented block-entity set (engine):** the engine stores block entities in a
> world-level `block_entities: AHashMap<(i32,i32,i32), BlockEntityData>` (an enum, not the
> opaque `kind`/`data` blob above — same effect, type-safe), each variant persisted as its
> own `Vec<Saved*>` in `WorldSave`. Variants: Campfire, Furnace, Vendor, Hive, Chest, TipJar,
> Auction, LatentPrint, and **Grave** (#47, 2026-06-16 — block id 263; holds a 36-slot
> inventory snapshot index-aligned to the dead player's inventory; persisted as
> `WorldSave.graves: Vec<SavedGrave>`, append-only + serde-default). The **`keep_inventory`
> world option** (`WorldMeta.keep_inventory`, serde-default false; `true` for blank-canvas
> worlds) suppresses graves and keeps the inventory on death — runtime-mirrored on `World`
> like `mobs_enabled`. Spec: `docs/foundations/2026-06-15-graves-and-keep-inventory.md`.

> **Spec 48 (Electricity) — meta byte + power persistence (Phase 1 delivered 2026-06-17):**
> two world-format additions, both append-only / serde-default so old saves load clean.
> (1) A **sparse per-block meta byte** — `World.block_meta: AHashMap<(i32,i32,i32), u8>`,
> packed facing (low 3 bits) / state (2 bits) / aux (3 bits) via `meta.rs`; persisted as
> — **AUX carries the WATER depth level 0–7 since 2026-07-04** (0 = source/full; worldgen
> water stays meta-free ⇒ level 0; see Spec 05 §7.2) — and otherwise
> `WorldSave.block_meta: Vec<(i32,i32,i32,u8)>`. Not derivable from the block id (lever
> latch, gate op, powered-rail bit), so it must be saved. (2) A new block-entity variant
> **`BlockEntityData::PowerDevice`** (`PowerDeviceData`: kind, facing, on-latch, charge,
> Steam-Generator fuel `FurnaceData`, gate op), persisted as `WorldSave.power_devices:
> Vec<SavedPowerDevice>`. The transient power flood (`World.power.energised`) is **NOT**
> saved — it's rederived on load by `power::reseed_on_load` (enqueue every device → the next
> `power_tick` recomputes lit cables/lamps). Full design: `docs/foundations/2026-06-17-electricity-power-logic.md`.

> **F1 block-shape foundation (2026-06-19):** the meta byte above is now also the
> orientation/variant store for **shaped blocks**. `block_shape::shape_of(id)` maps a block
> to its geometry (`FullCube` is the default — the ~280 existing blocks are untouched;
> the full building-detail family — `Slab`, `Stairs`, `FenceGate`, `Trapdoor`, `Door`,
> `Pane`, `Wall`, `Sign`, `ItemFrame`, plus flush `Button`/`Lever`/`PressurePlate` — all
> shipped 2026-06-19). Shaped blocks reuse the existing meta **facing** (3 bits) +
> **state** (2 bits) fields — slab half / vertical-slab side, stair facing + upside-down —
> so there is **no new world-format field**: orientation rides the meta byte already saved
> as `WorldSave.block_meta`. New block ids: `STONE_SLAB` (283), `STONE_STAIRS` (284).
> Collision is per-box (Spec 05 §1.4); rendering emits sub-cuboids (Spec 03). The
> building-detail family (doors/trapdoors/fence-gates/walls/panes/signs/frames) is built
> on top. Spec: `docs/foundations/2026-06-19-block-shape-foundation.md` (finding:
> `docs/foundations/2026-06-16-building-detail-blocks.md`); build:
> `docs/goals/2026-06-19-solo-buildout-wave-2.md`.
>
> **Wave 2c additions (2026-06-19).** The building-detail family landed:
> - **Connecting shapes (`Wall`, `Pane`).** Geometry is derived from a block's **live
>   cardinal neighbours**, NOT stored meta: `World::connection_mask(x,y,z,shape,registry)`
>   returns a 4-bit `block_shape::CONN_*` mask each query, so a wall/pane grows an arm the
>   moment a neighbour is placed and drops it on break — **zero persisted side-state** (no
>   migration, no scheduler). Walls join walls + full-cube solids; panes/bars join panes +
>   full-cube solids. New id `COBBLESTONE_WALL` (290); `GLASS_PANE`/`IRON_BARS` upgraded
>   from the flat v1 sheet to post + arms.
> - **Signal-input shapes.** The existing Electricity power blocks `LEVER` (272) / `BUTTON`
>   (273) / `PRESSURE_PLATE` (274) gained flush F1 shapes (was: chunky near-cubes);
>   pass-through, facing from the clicked face. Power behaviour unchanged; a lever mirrors
>   its on-state into meta state bit 0 so its handle visibly flips.
> - **Block-entity shapes (new world-format fields, append-only).** `OAK_SIGN` (291) and
>   `ITEM_FRAME` (292) carry per-instance state in `BlockEntityData::{Sign,ItemFrame}`,
>   persisted as **`WorldSave.signs` then `WorldSave.item_frames`, appended LAST** in the
>   bincode wire order (positional → append-only invariant) and read last by the tolerant
>   decoder; pre-Wave-2c saves default them empty. Signs hold capped free-form text; frames
>   hold one item + rotation. Both shapes are pass-through.
> - **Creator Gallery exhibits (new world-format field, append-only — Phase 1 delivered 2026-06-19).**
>   An **Exhibit** is a placed, sized 2D art surface — either flush on a wall or a standing
>   Y-axis billboard on a pedestal — carrying `presentation` (`Wall`|`Standing`), an anchor
>   `(x,y,z)`, artist-set `width`/`height`/`yaw`, an `image_ref` (filename), a `label`, and
>   reserved payload fields (`link`/`sku`/`price`, consumed in later phases). Defined in the
>   pure `exhibit` module. Placements persist as **`WorldSave.exhibits: Vec<Exhibit>`, appended
>   LAST** in the bincode wire order (after `rigs`; positional → append-only invariant) and
>   read last via `read_tail(...).unwrap_or_default()`; pre-gallery saves default it empty.
>   They ride `world.dat`, so they travel in `.axeworld` automatically. Exhibit **image bytes**
>   are NOT in the save — they live in `worlds/<name>/exhibits/<image_ref>` on disk (native) and
>   are served same-origin (`/exhibits/<ref>`) for web; `image_ref` is sanitised to a flat
>   filename (keeps the extension; rejects traversal). Packing `exhibits/` images into
>   `.axeworld` is a deferred follow-up (only needed for Phase 3 portability). The live list is
>   `World.exhibits` (mirrors `waypoints`): snapshotted on save, restored on load. Authored
>   in-world via `/exhibit` (Spec 05). Spec: `docs/superpowers/specs/2026-06-19-creator-gallery-showcase-design.md`.

### 1.6 Plugin Extension

Plugins register new block types through the Block Registry API:

```rust
impl BlockRegistry {
    /// Register a new block type. Returns the assigned numeric ID.
    /// Called during the plugin initialization phase, before world load.
    fn register(
        &mut self,
        namespaced_id: &str,
        properties: BlockProperties,
        state_schema: Option<BlockStateSchema>,
    ) -> Result<u16, RegistryError>;
}
```

Constraints:
- Registration is only permitted during the initialization phase.
- Duplicate namespaced IDs are rejected.
- The registry is frozen after initialization. Runtime block creation is not supported
  (it would invalidate every chunk's palette).

---

## 2. Chunk Format

### 2.1 Chunk Dimensions: 16x16x16

Each chunk is a cube of **16 x 16 x 16** blocks = **4096 blocks**.

**Justification**:
- 16-wide matches the platform's texture grid (16x16 default textures, world coordinates
  align cleanly with texture-space).
- 4096 blocks fit entirely in L1 cache when packed (details below).
- 16x16x16 is the sweet spot for meshing: small enough that re-meshing a single chunk
  after a block edit is fast (~0.5ms target), large enough that the per-chunk overhead
  (metadata, palette, pointers) is amortized across meaningful volume.
- 32x32x32 = 32768 blocks would make re-meshing 8x slower per chunk, and dirty-flagging
  granularity would be too coarse.
- Aligns with GPU instancing / indirect draw call batching: one draw call per chunk
  section is standard practice.

A **column** of chunks sharing the same (X, Z) is called a **chunk column**. The number
of chunks in a column depends on world height (Section 3).

### 2.2 Palette-Based Compression

Storing a raw `u32` block state ID per block would cost 4096 * 4 = 16 KiB per chunk.
Most chunks contain only a handful of distinct block states (air + stone + dirt + grass =
4 states in a typical surface chunk). Palette-based compression exploits this:

```
Chunk Block Storage:
  palette:   Vec<u32>          -- maps palette_index -> block_state_id
  bit_width: u8                -- bits per entry (ceil(log2(palette.len())), min 1)
  data:      [u64; N]          -- packed bit array, N = ceil(4096 * bit_width / 64)
```

**Bit width scaling**:

| Palette size | Bits per block | Data array size | Total (approx)  |
|-------------|---------------|-----------------|------------------|
| 1           | 0 (special)   | 0 bytes         | 4 bytes          |
| 2           | 1             | 512 bytes       | 520 bytes        |
| 3..4        | 2             | 1024 bytes      | 1040 bytes       |
| 5..8        | 3             | 1536 bytes      | 1568 bytes       |
| 9..16       | 4             | 2048 bytes      | 2112 bytes       |
| 17..256     | 8             | 4096 bytes      | 4608+ bytes      |
| 257..65536  | 16 (direct)   | 8192 bytes      | 8192 bytes       |

**Special case**: When `palette.len() == 1`, the entire chunk is a single block state.
Store only the palette entry (4 bytes), no data array. This is extremely common (air-only
chunks in the sky, stone-only chunks deep underground).

**Bit packing layout**:
- Entries are packed into `u64` words in little-endian order.
- Entries do NOT span word boundaries. Each `u64` holds `floor(64 / bit_width)` entries,
  with unused high bits set to zero.
- Block at local position `(x, y, z)` has array index `y * 256 + z * 16 + x` (Y-major
  for vertical locality during meshing).

**Rationale for Y-major order**: Meshing iterates layer-by-layer (bottom to top) for
greedy meshing. Y-major order means each horizontal slice is contiguous in memory,
maximizing cache hits during the most performance-critical operation.

### 2.3 Accessing a Block

```rust
fn get_block_state(chunk: &ChunkSection, x: u8, y: u8, z: u8) -> u32 {
    if chunk.palette.len() == 1 {
        return chunk.palette[0];
    }
    let index = (y as usize) * 256 + (z as usize) * 16 + (x as usize);
    let entries_per_word = 64 / chunk.bit_width as usize;
    let word_index = index / entries_per_word;
    let bit_offset = (index % entries_per_word) * chunk.bit_width as usize;
    let mask = (1u64 << chunk.bit_width) - 1;
    let palette_index = ((chunk.data[word_index] >> bit_offset) & mask) as usize;
    chunk.palette[palette_index]
}
```

### 2.4 Light Data

Light is stored in a separate parallel array per chunk section, NOT interleaved with
block data (different access patterns: block data is read during meshing, light data is
read during meshing AND during light propagation updates).

```
Light Storage (per chunk section):
  sky_light:   [u8; 2048]    -- 4 bits per block, packed 2 per byte
  block_light: [u8; 2048]    -- 4 bits per block, packed 2 per byte
```

Total: 4096 bytes per chunk section for lighting.

Packing: block at index `i = y*256 + z*16 + x`.
- Even index: low nibble of `light[i / 2]`.
- Odd index: high nibble of `light[i / 2]`.

Light values range 0..15, matching the 4-bit storage.

### 2.5 Biome Data

Biomes are stored at **4x4x4 resolution** (one biome sample per 4x4x4 block volume),
matching the granularity needed for 3D biome blending.

```
Biome Storage (per chunk section):
  biomes: [u16; 64]    -- 4*4*4 = 64 entries, each a biome registry ID (u16)
```

Total: 128 bytes per chunk section. Negligible compared to block and light data.

If palette compression is desired (most chunks have 1-2 biomes), the same palette scheme
from Section 2.2 can be applied, but the savings are small (128 bytes to ~10 bytes).
Recommended: use palette compression for biomes only in the serialized (on-disk) format,
not in-memory.

### 2.6 Chunk Section In-Memory Layout

```rust
struct ChunkSection {
    /// Y index of this section within the chunk column (-4..=23 for default height).
    y_index: i8,

    /// Palette-compressed block states.
    blocks: PalettedContainer<u32>,

    /// Sky light, 4 bits per block, 2048 bytes.
    sky_light: Box<[u8; 2048]>,

    /// Block light, 4 bits per block, 2048 bytes.
    block_light: Box<[u8; 2048]>,

    /// Biome data at 4x4x4 resolution.
    biomes: [u16; 64],

    /// Block entities in this section.
    block_entities: HashMap<u16, BlockEntityData>,  // key = y<<8 | z<<4 | x

    /// Dirty flag: set when any block is modified, cleared after mesh rebuild + save.
    dirty: bool,

    /// Tick timestamp of last modification (for dirty ordering).
    last_modified_tick: u64,
}

struct ChunkColumn {
    /// Chunk coordinate (world-space, in chunk units).
    cx: i32,
    cz: i32,

    /// Sections indexed by (y_index - Y_MIN_SECTION).
    sections: Vec<Option<Box<ChunkSection>>>,

    /// Heightmap: highest non-air block Y for each (x, z). Used for sky light
    /// and fast surface queries.
    heightmap_motion_blocking: [i16; 256],
    heightmap_world_surface: [i16; 256],

    /// Entity list (non-block entities: mobs, items, etc).
    entities: Vec<EntityId>,

    /// Pending block ticks (scheduled updates).
    pending_ticks: Vec<ScheduledTick>,

    /// Column-level dirty flag.
    dirty: bool,

    /// Current lifecycle state.
    state: ChunkLifecycleState,
}
```

**Memory budget per loaded chunk section** (typical surface chunk, ~8 palette entries):
- PalettedContainer: ~1600 bytes (data) + ~32 bytes (palette) = ~1632 bytes
- Sky light: 2048 bytes
- Block light: 2048 bytes
- Biomes: 128 bytes
- Overhead (HashMap, flags, etc.): ~100 bytes
- **Total: ~6.0 KiB per section**

For a full chunk column (28 sections for 448-block height), most sections above the
surface are `None` (all-air sections are not allocated). A typical column with 10 active
sections costs ~60 KiB.

---

## 3. World Dimensions

### 3.1 Y-Axis Range

- **Minimum Y**: -64 (block coordinate)
- **Maximum Y**: 383 (block coordinate)
- **Total height**: 448 blocks = **28 chunk sections** (each 16 blocks tall)
- **Section Y indices**: -4 to +23 (28 sections)

**Rationale**: 448 blocks provides ample underground depth (64 blocks, 4 sections) for
caves, ores, and bedrock layers, plus 384 blocks of above-ground height for mountains,
tall builds, and sky islands. This is slightly larger than Minecraft's 384-block range
and evenly divisible by 16.

### 3.2 Horizontal Extent

- **Coordinate type**: `i32` for both X and Z, measured in blocks.
- **Theoretical range**: -2,147,483,648 to +2,147,483,647 on each axis.
- **Practical world border**: configurable per world, default +/- 30,000,000 blocks
  (60 million blocks diameter = 3,750,000 chunks per axis).

Chunk coordinates are derived by arithmetic right shift: `chunk_x = block_x >> 4`.

Region coordinates (Section 4): `region_x = chunk_x >> 5` (each region = 32x32 chunks).

### 3.3 Coordinate Systems

```
Block coordinates:   (x: i32, y: i32, z: i32)       -- global, Y up
Chunk coordinates:   (cx: i32, cy: i8, cz: i32)     -- cy is the section Y index
Column coordinates:  (cx: i32, cz: i32)              -- identifies a full column
Region coordinates:  (rx: i32, rz: i32)              -- groups of 32x32 columns
Local coordinates:   (lx: u8, ly: u8, lz: u8)       -- 0..15 within a section
```

Conversion:

```
cx = x >> 4          (arithmetic shift, works correctly for negative values)
lx = x & 0xF        (bitwise AND)
cy = (y + 64) >> 4 - 4   (offset to account for Y_MIN = -64)
```

### 3.4 Coordinate Diagram

```
Y = 383 ┌─────────────────────┐  Section index +23
        │   Sky / build limit  │
        │                     │
Y = 256 ├─────────────────────┤  Section index +16
        │   Upper mountains    │
        │                     │
Y = 128 ├─────────────────────┤  Section index +8
        │   Surface / terrain  │
        │                     │
Y =   0 ├─────────────────────┤  Section index  0  (sea level ~Y=62)
        │   Underground        │
        │                     │
Y = -64 └─────────────────────┘  Section index -4
```

---

## 4. Region Storage

### 4.1 Region Dimensions

A region groups **32 x 32 chunk columns** = **1024 columns** for batch I/O.

Region file name format: `r.{rx}.{rz}.gbr` (Axe'n'Stax Region).

Example: `r.0.0.gbr`, `r.-1.3.gbr`.

### 4.2 Region File Format

The region file uses a fixed-size header followed by variable-length compressed chunk
payloads. This design supports O(1) random access to any chunk within the region.

```
Region File Layout:
  ┌──────────────────────────────────────┐
  │  File Header (16 bytes)              │
  ├──────────────────────────────────────┤
  │  Chunk Offset Table (1024 * 8 bytes) │   = 8192 bytes
  ├──────────────────────────────────────┤
  │  Chunk Timestamp Table (1024*4 bytes)│   = 4096 bytes
  ├──────────────────────────────────────┤
  │  Chunk Data Payloads (variable)      │
  │  ...                                 │
  └──────────────────────────────────────┘
```

**File Header** (16 bytes):

```
Offset  Size  Field
0       4     Magic number: 0x47425246 ("GBRF" in ASCII)
4       2     Format version (u16, starting at 1)
6       2     Compression method (u16): 0=none, 1=zstd, 2=lz4
8       4     Region X coordinate (i32, little-endian)
12      4     Region Z coordinate (i32, little-endian)
```

**Chunk Offset Table** (8192 bytes):
- 1024 entries, 8 bytes each.
- Entry for chunk at local (lx, lz) where lx, lz in 0..31:
  `table_index = lz * 32 + lx`.

```
Offset  Size  Field
0       4     Payload offset in file (u32, in units of 4096-byte sectors)
4       3     Payload length in bytes (u24, little-endian)
7       1     Compression override (u8): 0 = use file default, 1..N = per-chunk override
```

A payload offset of 0 means the chunk has never been saved (not generated or empty).

**Chunk Timestamp Table** (4096 bytes):
- 1024 entries, 4 bytes each.
- Each entry is a Unix timestamp (u32, seconds since epoch) of last save.

**Payload sector alignment**: Payloads are aligned to 4096-byte sectors. This aligns
with filesystem and SSD page boundaries, enabling efficient direct I/O. Wasted bytes
within the last sector of each payload are zero-padded.

### 4.3 Chunk Payload Format

Each chunk column is serialized as a single payload:

```
Chunk Payload:
  ┌────────────────────────────────────┐
  │ Payload Header (8 bytes)           │
  ├────────────────────────────────────┤
  │ Compressed Data (variable)         │
  └────────────────────────────────────┘

Payload Header:
  Offset  Size  Field
  0       4     Uncompressed length (u32)
  4       4     CRC-32C of uncompressed data
```

The compressed data, once decompressed, contains the full serialized ChunkColumn.

> **Implementation note (current engine — 2026-06-08).** The alpha engine uses a
> simpler per-sub-chunk byte format (`Chunk::as_bytes` / `from_bytes`) ahead of the
> rkyv ChunkColumn above: the `CHUNK_VOLUME` block IDs as little-endian `u16`
> (8192 bytes) **followed by the per-voxel "player-placed" mask** — 4096 bits =
> `PLACED_MASK_BYTES` (512 bytes) of little-endian `u64` words (Spec 6 §2.2
> anti-farming). Light is not persisted (recomputed on load). `from_bytes` is
> **length-tolerant**: a legacy 8192-byte blob (no mask) decodes as all-natural,
> preserving the append-only save invariant; any other length is rejected rather
> than truncated. When the rkyv ChunkColumn lands, the placed mask becomes a
> field on it.

### 4.4 Serialization Format

**Primary format**: `rkyv` (zero-copy deserialization).

**Rationale**:
- **Zero-copy read path**: When loading a chunk from disk, `rkyv` allows accessing the
  deserialized structures directly from the memory-mapped / read buffer without a full
  parse-and-copy step. This is critical for fast chunk streaming.
- **Speed**: rkyv is 10-50x faster than bincode/serde for deserialization in benchmarks.
- **Deterministic layout**: rkyv produces identical bytes for identical input, important
  for checksums and deduplication.

**Fallback format**: `bincode` via serde, used for:
- World metadata (Section 9) where zero-copy is not critical.
- Network serialization (chunks sent to clients).
- Debug / human-inspection tooling (with a `--json` conversion flag).

Format version is stored in the region file header. Deserializers check the version and
dispatch to the appropriate reader.

### 4.5 Compression

**Primary algorithm**: **zstd** at level 3.

**Rationale**:
- zstd level 3 offers an excellent compression-ratio-to-speed balance for real-time game
  data. Typical voxel chunk compression ratios: 5:1 to 20:1.
- Decompression is faster than lz4 at equivalent ratios for our data patterns (palette
  data is highly compressible).
- zstd dictionaries can be pre-trained on typical world data for even better ratios on
  small payloads.

**Alternative**: **lz4** for hot-path scenarios where decompression speed is more
important than ratio (e.g., chunk data sent over the network to clients with limited CPU).
The compression method byte in the offset table allows per-chunk override.

Benchmark targets:
- Compression: < 1ms per chunk column on a single core.
- Decompression: < 0.3ms per chunk column.
- Compressed size of typical surface column: < 8 KiB.

### 4.6 Storage Backends

The region I/O layer abstracts behind a `StorageBackend` trait:

```rust
#[async_trait]
trait StorageBackend: Send + Sync {
    /// Read a chunk column from storage.
    async fn load_chunk(&self, cx: i32, cz: i32) -> Result<Option<Vec<u8>>>;

    /// Write a chunk column to storage.
    async fn save_chunk(&self, cx: i32, cz: i32, data: &[u8]) -> Result<()>;

    /// Atomically snapshot the entire world to a new location.
    async fn snapshot(&self, dest: &str) -> Result<SnapshotHandle>;

    /// List all chunk coordinates that exist in storage.
    async fn list_chunks(&self) -> Result<Vec<(i32, i32)>>;

    /// Flush all pending writes.
    async fn flush(&self) -> Result<()>;
}
```

**Implementations**:

| Backend | Use case | Details |
|---------|----------|---------|
| `FilesystemBackend` | Personal tier, self-hosted | Region files on local disk. Standard `open`/`pread`/`pwrite` with `O_DIRECT` where supported. |
| `S3Backend` | Production tier (cloud) | Region files stored as S3 objects. Chunk reads use range requests. Writes are batched and uploaded on flush. |
| `MemoryBackend` | Testing, ephemeral worlds | In-memory `HashMap`. |

For the `S3Backend`, the region file is the natural unit of S3 object storage. Reading a
single chunk within a region uses an HTTP Range request against the appropriate sector
offset. Write-behind caching ensures that multiple chunk saves within the same region are
coalesced into a single S3 PUT.

---

## 5. World Generation

### 5.1 Pipeline Architecture

World generation is a deterministic, multi-stage pipeline. Each stage operates on a
**generation context** containing the world seed and chunk coordinates. Stages are
executed in strict order, but independent chunks can be generated in parallel.

```
Pipeline stages (executed per chunk column):

  ┌─────────────┐
  │  1. Shape    │  Density field / height map generation
  └──────┬──────┘
         v
  ┌─────────────┐
  │  2. Biome   │  Biome assignment (climate + continental + erosion noise)
  └──────┬──────┘
         v
  ┌─────────────┐
  │  3. Surface  │  Surface decoration (grass, sand, snow based on biome)
  └──────┬──────┘
         v
  ┌─────────────┐
  │  4. Carve   │  Caves, ravines, overhangs
  └──────┬──────┘
         v
  ┌─────────────┐
  │  5. Feature  │  Ores, trees, flowers, boulders, small structures
  └──────┬──────┘
         v
  ┌─────────────┐
  │  6. Struct   │  Large structures (villages, temples, strongholds)
  └──────┬──────┘
         v
  ┌─────────────┐
  │  7. Light    │  Initial sky light propagation
  └──────┬──────┘
         v
  ┌─────────────┐
  │  8. Finalize │  Heightmap calculation, validation
  └──────┘
```

### 5.2 Seed and Determinism

- **World seed**: `u64`, set at world creation. Derived from user input string via
  SipHash, or randomly generated.
- All noise functions and RNG instances are seeded deterministically from the world seed
  combined with the chunk coordinates. This means generating chunk (10, 20) always
  produces the exact same result regardless of what other chunks exist.
- **Per-stage salt**: Each pipeline stage adds a unique salt to prevent noise correlation
  between stages.

```rust
fn stage_seed(world_seed: u64, stage_salt: u64, cx: i32, cz: i32) -> u64 {
    let mut hasher = SipHasher::new();
    hasher.write_u64(world_seed);
    hasher.write_u64(stage_salt);
    hasher.write_i32(cx);
    hasher.write_i32(cz);
    hasher.finish()
}
```

**As built (2026-10-06, gap-audit T2-9 + Phase B0) — `WORLDGEN_VERSION` and the worldgen
fingerprint.** The shipped seed is a `u32` (`WorldMeta.seed`; text seeds hash via FNV-1a,
`save::seed_from_text`). A multiplayer joiner regenerates the host's untouched terrain locally
from the seed plus the world flags in `JoinAcceptPacket.world_rules` (Spec 04 v65), so generator
output is a wire contract. `world::WORLDGEN_VERSION` (currently **2**) names the generator code.
**Bump it whenever generation output for a given seed + flags changes** — anything reached from
`World::generate_column`: terrain shape, biomes, caves, ore, trees, vegetation, villages,
hideouts, ravines, mineshafts, or the flat / water / Workshop-void presets.

What host and joiner actually exchange (`JoinAccept.worldgen_version` /
`JoinRequest.worldgen_version`, still a `u32`) is `world::worldgen_fingerprint()`: the first four
bytes of SHA-256 over `WORLDGEN_VERSION` and the content hash of the bundled plan registry villages
are built from (`PlanRegistry::bundled().content_hash()` — every entry's parsed `PlanData` via
bincode plus its category, in registry order; parsed content, so a CRLF checkout hashes the same).
So a bundled-plan edit changes the fingerprint with no version bump. 0 maps to 1, because a peer
that sent no value decodes as 0 and must read as a mismatch. On a mismatch the joiner gets a toast
("Some terrain may look different until you update") and the host flags the player
(`ServerPlayer::worldgen_mismatch`) so a later pass can push real chunks instead.

**Generation is a pure function of (seed, world flags, fingerprint)** (Phase B0, 2026-10-06):
independent of the order columns are generated in and of platform float maths. The rules that
keep it so (break one and joiners desync silently):

- **Deepslate variants bake a constant.** `base_rock_at` picks the deepslate visual variant with
  `biome::WORLDGEN_RESERVE_RICHNESS = 0.75` (what single-player always baked), never the live
  Reserve richness. *Bug fixed:* the client used to copy `reserve.richness` into
  `BiomeGenerator` every tick while `GameServer.biome_gen` kept 0.5, so the same seed baked
  different deepslate block ids on the host client, the server and a joiner. Richness-driven
  visuals move to render time when the reward layer lands (`game_loop.rs` BRIDGE;
  `docs/foundations/2026-05-17-deepslate-reserve.md`).
- **Decoration passes write only inside the column being generated, and read neighbour columns
  only through pure functions of `biome_gen`** — `tree_at_column_cell`, `terrain_block_at`,
  `village_gen::village_site` — never `World::get_block` on a column that may not exist yet (it
  reads AIR). *Bug fixed:* papyrus checked `is_water` on the neighbour cell, which at a column edge
  is another column, so reeds depended on generation order; it now asks `terrain_block_at`.
- **Structure gates use sites, not generated state.** The Brigand Hideout village-distance gate
  (no hideout within 128 blocks of a village) asks `village_gen::village_site_within`, built on the
  same `village_site` (cell roll, biome gate, sea-level gate) villages are placed from. *Bug fixed:*
  it read `World::village_anchors`, which only fills as village columns generate, so a hideout
  appeared or not depending on whether the nearby village's columns had generated first.
- **Villages sample `PlanRegistry::bundled()`** (immutable, parsed once per process), never
  `World::plan_registry`, which also holds runtime additions (`/importschem`; `add` bypasses the
  licence filter and replaces a same-named bundled plan). A test world (`World::new()`) therefore
  generates the same plan-built villages as production.
- **No platform transcendental functions.** Layout trig (village and hideout rings, ravine
  projection and wobble) is the pure-Rust `libm` crate (`sinf` / `cosf` / `sincosf`): platform
  libms (glibc, Android bionic, macOS, the WASM build) may differ by an ULP, enough to flip a
  `.round()`ed cell. Squares are `t * t`, not `powi`. Everything else is IEEE-exact
  (`+ - * / sqrt floor round`, int↔float casts). The `noise` crate's OpenSimplex uses only those
  plus `t.powi(4)`, which both LLVM's lowering and compiler-rt's `__powidf2` compute as `(t²)²` —
  not patched, noted as the one unpinned dependency-internal.
- **No hash-map iteration order reaches a block write.** The per-column village anchor set is a
  `BTreeMap`; the hideout's `AHashMap` village snapshot is gone.

The golden test `test_integration/worldgen_golden.rs` covers two column sets: terrain (seed
`20261006`: caves, ore, trees, vegetation across two biomes, plus flat grass, flat water and the
Workshop void) and structures (seed `2042`: village V, a hideout H that places, and a hideout
candidate R 108 blocks from V that the gate must reject). Each set is generated in row-major,
reversed and scattered column order into fresh `World`s and compared cell by cell (block id +
meta) plus every side table generation writes (`block_meta`, `block_entities` incl. chest loot,
`architect_plaques`, `procgen_plaque_sources`, `brigand_hideouts`, `village_anchors`). The golden
hash (blocks + meta of every set, plus structure side-table positions) is pinned together with
the bundled-plans hash: if only the plans changed the test says "update GOLDEN_PLANS and GOLDEN,
no version bump"; if the output changed with the same plans it says "bump WORLDGEN_VERSION". The
B0 order test failed on version-1 code with 183 cells differing (hideout R placed when its columns
generated before V's).

| Worldgen version | Date | Change |
|---|---|---|
| 1 | 2026-10-06 | Baseline: output as of protocol v65. |
| 2 | 2026-10-06 | Phase B0 purity: deepslate baked at fixed richness 0.75 (server-generated worlds used 0.5); hideout village gate from pure village sites (some hideouts near villages no longer place); papyrus water check pure; villages always from the bundled plans; `libm` trig in village / hideout / ravine layouts (a few cells may shift). Wire value becomes `worldgen_fingerprint()`. |

### 5.3 Stage Details

#### 5.3.1 Shape (Density / Height)

Generates the 3D terrain shape using layered noise:

- **Continental noise** (very low frequency): Determines landmass vs ocean.
- **Erosion noise** (low frequency): Determines terrain roughness.
- **Terrain height noise** (medium frequency): Base height variation.
- **3D density noise** (high frequency): Overhangs, floating islands, cave systems.

The density function outputs a value per block position. Blocks with density > threshold
become solid (initially `genesis:stone`). The threshold varies with Y to produce a
natural surface.

Noise implementation: **OpenSimplex2** via the `noise` crate. Octave-based fractal noise
(typically 4-6 octaves for terrain shape).

Performance target: < 2ms per chunk column for shape generation on a single core.

#### 5.3.1a Deepslate Replacement

After the shape stage produces stone, a deepslate-replacement pass converts stone to **pure deepslate** below a configurable threshold `Y_dp` (the deepslate horizon). Deepslate is mechanically a stronger stone variant with its own hardness profile and one critical property: it is the only block type eligible to host **Satori veins** (Spec 5 §3.8, Spec 6 §2.2c).

**Replacement rule:**

```
for each stone block at (x, y, z):
    if y < (Y_dp - transition_depth):
        # Below the transition zone: pure deepslate
        block <- PURE_DEEPSLATE
    else if y < Y_dp:
        # Transition zone: probabilistic mix, weighted toward deepslate with depth
        p = (Y_dp - y) / transition_depth   # 0.0 at Y_dp, 1.0 at floor of zone
        h = position_hash(seed, x, y, z)
        if h_to_unit_float(h) < p:
            block <- PURE_DEEPSLATE
        # else stay STONE
```

**Default parameters:**

| Parameter | Canonical default (Y=-64..383 spec) | Alpha-engine default (Y=0..96, see Spec 1 §X) | Notes |
|---|---|---|---|
| `Y_dp` | 0 | 30 | Where pure deepslate begins replacing stone. Defaults aligned with sea-level-relative-depth in each world configuration. |
| `transition_depth` | 8 | 8 | Vertical thickness of the stone↔deepslate transition zone. Below `Y_dp - transition_depth` the world is fully deepslate. |

**Ore variants in deepslate:**

When the feature-placement stage (§5.3.5) places an ore block, it checks the surrounding terrain context. If the placement position is in pure deepslate, the **deepslate-tier ore variant** is used:

| Stone-tier ore | Deepslate-tier ore | Notes |
|---|---|---|
| `genesis:coal_ore` | `genesis:deepslate_coal_ore` | Same drop (coal); slightly higher hardness; same tool requirement. |
| `genesis:iron_ore` | `genesis:deepslate_iron_ore` | Same drop (raw iron); same tool requirement. |
| `genesis:diamond_ore` | `genesis:deepslate_diamond_ore` | Same drop (diamond); same tool requirement (iron+ pickaxe). |

Deepslate ore variants are **distinct block types** in the registry (§1.2). They participate in the same anti-X-ray chunk-stream obfuscation as their stone counterparts (Spec 8 §5.2.2).

**Diamond is deepslate-only at the alpha defaults.** The alpha-engine diamond
roll band is `y < 15`, which lies entirely below the fully-deepslate floor
(`Y_dp - transition_depth = 30 - 8 = 22`). So every naturally-generated diamond
is `genesis:deepslate_diamond_ore`; the plain `genesis:diamond_ore` block **does
not generate naturally** (it remains registered for the diamond storage-block
recipe + `mine_drop`). The engine's `ore_at` conditional still handles both
arms defensively, so widening the diamond band above y=22 (or lowering `Y_dp`)
would start producing the plain variant — a deliberate change locked by
`biome::diamond_band_is_all_deepslate_never_plain_diamond_ore` (engine audit
2026-06-04, E: the original "spec mismatch" was the spec under-documenting this,
not a code bug).

**Ore depth bands (alpha engine, `biome::ore_at`):** Diamond/Iron/Coal share
one position hash (different bit-slices); Magnesium takes a different slice
of the same hash. Brimstone, Nitre and Copper each roll their own hash with a
distinct `seed.wrapping_add(N)` offset so they never bias the shared-hash
bands at the same position. Checked in priority order — the first band a
position falls in and rolls under wins:

| Ore | Depth band | Rarity | Deepslate variant? |
|---|---|---|---|
| Diamond | `y < 15` | ~0.4% | Yes (`deepslate_diamond_ore`) — see above, effectively always this variant at alpha defaults |
| Iron | `y < 50` | ~3% | Yes (`deepslate_iron_ore`) |
| Magnesium | `Y_dp..48` (30..48) | ~2% | No — stone-only |
| Brimstone | `Y_dp..48` (30..48) | ~3% | No — stone-only |
| Nitre | `Y_dp..SEA_LEVEL` (30..62) | ~2.8% | No — stone-only |
| **Copper** | `Y_dp..(SEA_LEVEL+8)` (30..70) | ~3% | No — stone-only |
| Coal | anywhere in stone | ~6% | Yes (`deepslate_coal_ore`) |

**Copper Ore** (Wind, Copper & Electricity wave, 2026-09-07, `docs/superpowers/specs/2026-09-07-wind-copper-electricity-wave-design.md` §1) closes the
"Electricity is a Survival dead end" gap: Copper Ore previously had no natural
generation path at all (creative-only placement). Its band sits entirely
above the deepslate threshold (like Magnesium/Brimstone/Nitre), so it has no
deepslate-tier variant and the ore↔substrate invariant holds. Buried Copper
Ore disguises as `genesis:stone` under anti-X-ray (Spec 8 §5.2.2), same as
Coal/Iron/Diamond.

**Gem-vein eligibility:**

Pure deepslate (NOT deepslate-ore variants) is the substrate for Satori veins:

- A block is eligible for vein membership iff its block type is exactly `genesis:pure_deepslate`. Deepslate-coal-ore, deepslate-iron-ore, deepslate-diamond-ore, polished deepslate, etc., are **not eligible**. The block has one identity; ore-bearing deepslate cannot also bear a gem.
- The vein-eligible depth gate is `y <= Y_dp - 21` (gem veins begin 21 blocks below where pure deepslate starts replacing stone). With the canonical-default `Y_dp = 0`, gate is `y <= -21`. With the alpha-default `Y_dp = 30`, gate is `y <= 9`.
- Vein generation algorithm and parameters: Spec 6 §2.2c.

**Why a separate stage:**

The deepslate replacement runs *between* shape (5.3.1) and feature placement (5.3.5) so that ore placement can correctly select the deepslate-tier variant where applicable. Surface decoration (5.3.3) and carving (5.3.4) run after deepslate replacement; caves carved into deepslate expose pure deepslate walls, which is the expected mining surface for the gem-vein mechanic + exposure-decay model (Spec 6 §2.2c.3).

#### 5.3.2 Biome Assignment

Biomes are determined by a multi-parameter lookup:

```
Biome = f(temperature, humidity, continentalness, erosion, depth, weirdness)
```

Each parameter is a separate noise field sampled at 4-block resolution (matching the
biome storage granularity from Section 2.5).

The biome lookup table maps parameter ranges to biome IDs:

```rust
struct BiomeParameters {
    temperature: f32,      // -1.0 (frozen) to 1.0 (hot)
    humidity: f32,         // -1.0 (dry) to 1.0 (wet)
    continentalness: f32,  // -1.0 (ocean) to 1.0 (inland)
    erosion: f32,          // -1.0 (flat) to 1.0 (mountainous)
    depth: f32,            // 0.0 (surface) to 1.0 (deep underground)
    weirdness: f32,        // -1.0 to 1.0 (unusual terrain features)
}
```

Biome selection uses a **nearest-neighbor search** in this 6D parameter space against a
table of registered biome definitions. This naturally produces smooth biome transitions
and allows plugins to inject new biomes by adding entries to the table.

**Alpha implementation (2026-05-27, `BiomeGenerator::biome_at`).** The shipped
classifier is a 2-step reduction of the above (Spec 28a). First a broad
**continentalness** noise (`height_noise` at scale 0.0015) picks the terrain-shape
biomes: `< -0.45` → Ocean, `> 0.45` → Mountains. The mid band is classified by
**climate** — temperature + humidity (OpenSimplex at scale 0.003) fed into the tested
`classify_whittaker` 4×3 grid → the 8 climate biomes. Decoupling Mountains from
temperature (it was previously `temp < -0.3`) is what lets the cold band resolve to
Taiga / Snowy Tundra. Thresholds are playtest-tunable. The full 6D nearest-neighbour
model above remains the destination.

#### 5.3.3 Surface Decoration

Replaces the top N blocks of solid terrain based on biome rules:

- Plains: 1 block grass, 3 blocks dirt, then stone.
- Desert: 4 blocks sand, 2 blocks sandstone, then stone.
- Ocean floor: 2 blocks gravel or sand, then stone.
- Mountains: Bare stone above Y=180, snow above Y=220.

Each biome registers a `SurfaceRule` that describes the replacement pattern.

#### 5.3.4 Carving

Caves and ravines are carved using noise-based worm algorithms:

- **Spaghetti caves**: 3D noise-based tunnels.
- **Cheese caves**: Large open caverns from inverted density noise.
- **Ravines**: 2D path with vertical extent.
- **Aquifers**: Water-filled cave systems below a configurable water table.

Carvers run AFTER surface decoration so they correctly cut through surface blocks.

#### 5.3.5 Feature Placement

Small decorations placed via scatter:

| Feature | Placement strategy |
|---------|-------------------|
| Trees | Biome-specific, surface scatter with spacing jitter |
| Ores | Y-range-limited random scatter, per-ore frequency table |
| Flowers | Surface scatter, biome-specific species |
| Boulders | Rare surface scatter in mountain biomes |
| Vegetation | Seagrass, kelp, lily pads in water biomes |

Features are placed using a deterministic position hash to ensure consistent output.
Features that cross chunk boundaries use a **feature placement region** that extends
1 chunk beyond the current chunk, with deduplication ensuring each feature is placed
exactly once.

**Alpha implementation (2026-05-27, `World::place_trees` + `World::place_vegetation`).**
Trees scatter per biome from `biome_properties(biome).tree_species` with a per-biome
threshold gate. Ground vegetation (`place_vegetation`, runs after trees, keyed by
`veg_hash(wx, wz, seed)`): **tall grass** (~12%) and **berry bushes** (~1%, placed
mature) on GRASS surfaces; **papyrus reeds** (mature, ~35%) on the shoreline cell one
block above sea level when an orthogonal neighbour is water. Single-block plants, so no
cross-chunk clipping. Flower species for dyes (Cornflower/Field Poppy/Buttercup) hook
into this same pass — see `docs/foundations/2026-05-27-flowers-dyes-colour-mixing.md`.

#### 5.3.6 Structure Generation

Large structures (spanning multiple chunks) use a two-phase approach:

1. **Structure start placement**: Deterministic hash check per structure region
   (configurable grid, e.g., one village attempt per 32x32 chunk area). If the hash
   passes the probability check, a structure start is recorded.
2. **Structure piece generation**: When a chunk is generated, it queries for any structure
   starts whose bounding boxes overlap. The structure's piece generator produces the
   blocks for the intersection of the structure and the current chunk.

This allows structures to span arbitrary numbers of chunks without requiring all chunks
to be generated simultaneously.

**Shipped (Spec 19 phase 4):** the first concrete structure generator is `village_gen`
(`game/engine/src/village_gen.rs`). One village per `VILLAGE_GRID = 32`-chunk cell, deterministic
on `hash(world_seed, grid_x, grid_z)`. ~80 % of cells host a village (rejected
20 % keeps the world feeling like it has wilderness between settlements).
Anchor world position is offset within the cell, biome-gated to Plains or
Forest. Each village places 3–6 cobblestone-walled houses with planks
floor/roof + a bed + a ceiling torch + an inward-facing door, plus a cobblestone
well around a water source, plus a Spec 17 lit campfire on a cross-shaped
cobblestone hearth. Generation is column-aware — the partial-build pattern
(`apply_layout_to_column`) writes only the blocks that fall inside the
calling column, so the structure straddles chunk boundaries without a global
pre-pass. The `StructureGenerator` trait below is the destination shape;
`village_gen` doesn't yet implement it (it's hooked directly into
`world::generate_column`) — folding the trait around it is mechanical and
lands when a second structure type joins.

**Bugfix 2026-06-23 — "void columns" / floor full of grid holes.** Symptom:
fresh worlds showed a grid of bottomless gaps ("a block city where the roads
are the void") — drop in and you fall forever and respawn. Root cause: a
`World::set_block` (including AIR carves from structure generation) into a
column that has *not* yet been through `generate_column` does
`chunks.entry(..).or_insert_with(Chunk::new)`, creating an **empty phantom
chunk** at that `cy`. `generate_column` then skipped any `cy` whose chunk
already existed (`contains_key`), so the phantom's `cy` — and, for `cy=0`, the
**unconditional bedrock floor** (`biome_block_at(y==0) == BEDROCK`) — was never
laid. The column rendered as void and had no collision floor. **Correct
approach:** `generate_column` must skip a `cy` only when its existing chunk is
**non-empty** (`chunks.get(..).is_some_and(|c| !c.is_empty())`); an empty
phantom is (re)filled. Real-block structure pre-writes (non-empty) are still
preserved — the surrounding `cy`s fill around them. Defense-in-depth: the
client streamer (`stream_chunks`) self-heals any column marked loaded that
lacks a `y=0` bedrock floor (normal worlds only), and a one-shot
`repair_void_columns_after_load` at the Loading→Playing handoff logs + repairs
any void column within render distance. Invariant test:
`generate_column_refills_empty_phantom_chunk_lays_bedrock`, plus the wide-grid
`generate_column_floors_and_surfaces_every_column`.

```rust
trait StructureGenerator: Send + Sync {
    /// Determine if a structure starts in this region.
    fn try_start(
        &self,
        seed: u64,
        region_x: i32,
        region_z: i32,
    ) -> Option<StructureStart>;

    /// Generate the portion of this structure that falls within the given chunk.
    fn generate_chunk(
        &self,
        start: &StructureStart,
        chunk: &mut ChunkBuildContext,
    );
}
```

#### 5.3.7 Initial Lighting

After all blocks are placed, the initial lighting pass runs:

1. Set sky light to 15 for all air blocks above the heightmap.
2. Propagate sky light downward through transparent blocks.
3. Compute block light from all light-emitting blocks (lava, glowstone, etc.).
4. Run the BFS flood fill (Section 6) for both light types.

Initial lighting is the most expensive generation stage (~40% of total gen time) because
it requires reading neighbor chunk data. It is deferred until all four cardinal neighbor
chunks have completed stages 1-6.

### 5.4 Parallelization

```
                    Independent chunks                Neighbor-dependent
                ┌─────────┐  ┌─────────┐
 Stages 1-6:    │ Chunk A │  │ Chunk B │  ...        (fully parallel)
                └────┬────┘  └────┬────┘
                     v            v
 Stage 7 (light):   ─────── barrier ──────            (requires neighbors)
                     v            v
 Stage 8 (final):   │ Chunk A │  │ Chunk B │          (parallel again)
```

Stages 1 through 6 are embarrassingly parallel per chunk column. The world generation
thread pool processes a queue of requested chunk columns, each running through stages 1-6
independently.

Stage 7 (lighting) requires neighbor data and is scheduled after a chunk and its
neighbors have all completed stages 1-6. A dependency tracker counts neighbor completion
and triggers the lighting pass when all four cardinal neighbors are ready.

### 5.5 Plugin Extension for World Generation

Plugins can register:

- **Custom biomes**: via `BiomeRegistry::register(BiomeDefinition)`.
- **Custom surface rules**: via `SurfaceRuleRegistry::register(biome_id, SurfaceRule)`.
- **Custom features**: via `FeatureRegistry::register(FeatureDefinition)`.
- **Custom structures**: via `StructureRegistry::register(Box<dyn StructureGenerator>)`.
- **Custom carvers**: via `CarverRegistry::register(Box<dyn Carver>)`.

All registrations happen during initialization. The pipeline queries registries at
generation time, so plugin content is seamlessly integrated.

---

## 6. Lighting System

### 6.1 Overview

Axe'n'Stax uses a dual-channel light model:

- **Sky light**: Sunlight propagating from above. Maximum value 15 at full exposure.
  Reduced by 1 per block of non-transparent material traversed. Time-of-day dimming is
  applied at render time, NOT stored in the light map.
- **Block light**: Light emitted by blocks (torches, lava, glowstone, etc.). Maximum
  value 15. Reduced by 1 per block of distance plus any absorption from transparent
  blocks.

Both channels are stored as 4-bit values (0..15) per block, packed as described in
Section 2.4. Total lighting storage: 1 byte per block (4 bits sky + 4 bits block).

### 6.2 Light Propagation Algorithm

Light propagation uses **BFS flood fill** from light sources. Two queues are maintained:
one for sky light, one for block light. The algorithm is identical for both channels.

#### 6.2.1 Light Increase (Placement / Generation)

When a light source is added (block placed, chunk generated):

```
PROCEDURE propagate_light_increase(queue: &mut VecDeque<(BlockPos, u8)>):
    WHILE queue is not empty:
        (pos, light_level) = queue.pop_front()
        FOR EACH neighbor in pos.six_neighbors():
            absorption = block_properties[get_block(neighbor)].light_absorption
            new_level = light_level - 1 - absorption
            IF new_level > 0 AND new_level > get_light(neighbor):
                set_light(neighbor, new_level)
                queue.push_back((neighbor, new_level))
```

#### 6.2.2 Light Decrease (Removal)

When a light source is removed (block broken, torch destroyed):

```
PROCEDURE propagate_light_decrease(
    decrease_queue: &mut VecDeque<(BlockPos, u8)>,
    increase_queue: &mut VecDeque<(BlockPos, u8)>,
):
    WHILE decrease_queue is not empty:
        (pos, old_level) = decrease_queue.pop_front()
        FOR EACH neighbor in pos.six_neighbors():
            neighbor_level = get_light(neighbor)
            IF neighbor_level != 0 AND neighbor_level < old_level:
                set_light(neighbor, 0)
                decrease_queue.push_back((neighbor, neighbor_level))
            ELSE IF neighbor_level >= old_level:
                increase_queue.push_back((neighbor, neighbor_level))
    propagate_light_increase(increase_queue)
```

The decrease pass zeroes out all light values that were dependent on the removed source,
then the increase pass re-fills from surviving adjacent sources.

#### 6.2.3 Sky Light Special Case

Sky light propagates **downward without attenuation** through air blocks. When sky light
at level 15 enters an air block from directly above, it remains 15 (not 14). This models
sunlight streaming straight down. Horizontal and upward propagation of sky light follows
the standard -1-per-block rule.

```rust
fn sky_light_attenuation(from: BlockPos, to: BlockPos, current_level: u8) -> u8 {
    if from.y > to.y && current_level == 15 && is_air(to) {
        15  // Sunlight streams straight down without loss
    } else {
        current_level.saturating_sub(1)
    }
}
```

### 6.3 Cross-Chunk Boundary Propagation

Light propagation naturally crosses chunk boundaries. The BFS queue contains absolute
world positions, and `get_light` / `set_light` resolve to the correct chunk section
through a chunk lookup cache.

To avoid locking multiple chunks simultaneously:

1. The light engine processes one chunk's worth of updates at a time.
2. When propagation reaches a chunk boundary, the update is placed in a **boundary queue**
   for the neighboring chunk.
3. Neighboring chunks process their boundary queues on their next light update tick.
4. Convergence is guaranteed within `ceil(15 / 1) = 15` cross-chunk propagation steps
   in the worst case, but typically converges in 1-2 steps because most light sources
   are localized.

### 6.4 Batch Updates

When multiple blocks change in rapid succession (e.g., explosions, world generation,
pasting a schematic), the light engine batches updates:

1. Collect all changed positions into a pending set.
2. After the batch is complete, run the decrease pass for all removals, then the
   increase pass for all additions.
3. Dirty-flag affected chunks for re-meshing.

This avoids redundant propagation when intermediate states are immediately overwritten.

### 6.5 Performance Targets

| Operation | Target | Notes |
|-----------|--------|-------|
| Single block light update | < 50us | Typical torch placement |
| Chunk initial lighting | < 1ms | During world generation |
| Explosion (100 blocks) | < 2ms | Batched update |
| Full sky light recalculation (column) | < 5ms | Rare, e.g., large roof removal |

---

## 7. Chunk Lifecycle

### 7.1 States

```
            load()
  ┌──────┐ ────────> ┌──────────┐
  │ UNLOADED │       │  LOADING  │
  └──────┘ <──────── └────┬─────┘
            unload()      │ loaded
                          v
                    ┌──────────┐    modify()    ┌──────────┐
                    │  ACTIVE  │ ──────────>    │  DIRTY   │
                    └──────────┘ <──────────    └────┬─────┘
                          ^       save_complete()    │ save()
                          │                          v
                          │                    ┌──────────┐
                          └────────────────────│  SAVING  │
                                               └──────────┘
```

- **UNLOADED**: Chunk exists only on disk (or not at all).
- **LOADING**: Async I/O in progress (disk read or world generation).
- **ACTIVE**: In memory, up to date, no unsaved changes.
- **DIRTY**: In memory, has unsaved modifications.
- **SAVING**: Async write in progress. The chunk remains readable but new modifications
  create a copy-on-write shadow. When the save completes, the shadow (if any) becomes the
  new DIRTY state; otherwise the chunk transitions to ACTIVE.

### 7.2 Loading Priority

Chunk loading is prioritized by distance from requesting players, with tie-breaking by
request type:

```
Priority (lower = loaded first):
  1. Chunk containing the player (must be loaded before spawn)
  2. Chunks within 2-chunk radius of player (immediate visibility)
  3. Chunks within tick distance (simulation needed)
  4. Chunks within view distance (rendering only)
  5. Pre-generation requests (worldgen background work)
```

The chunk loader maintains a priority queue sorted by these criteria. When a player moves,
priorities are recalculated. The loader processes up to N chunks per tick (configurable,
default 4 loads per tick + 2 generations per tick).

### 7.3 Tick Radius vs View Distance

Two separate distances control chunk behavior:

- **Tick distance** (default: 8 chunks = 128 blocks): Chunks within this radius of any
  player receive full simulation (mob AI, scheduled ticks, random ticks, redstone/signal
  updates, fluid flow, fire spread, crop growth).
- **View distance** (default: 12 chunks = 192 blocks): Chunks within this radius are
  loaded and sent to the client for rendering but do NOT simulate. Entities in these
  chunks are frozen.

```
  View distance ring:
  ┌─────────────────────────────────────┐
  │  Loaded, rendered, NOT ticked       │
  │  ┌─────────────────────────────┐    │
  │  │  Tick distance ring:        │    │
  │  │  Loaded, rendered, TICKED   │    │
  │  │  ┌─────────────────────┐    │    │
  │  │  │  Player's chunk     │    │    │
  │  │  └─────────────────────┘    │    │
  │  └─────────────────────────────┘    │
  └─────────────────────────────────────┘
```

The tick distance is server-side only. The view distance is configurable per client
(within server-imposed limits).

### 7.4 Lazy Loading

Chunks are loaded on demand, never speculatively loaded beyond the view distance. When
a player connects or teleports:

1. The server immediately queues the player's current chunk (priority 1).
2. A spiral-out pattern queues surrounding chunks at decreasing priority.
3. The client receives chunks as they become available, building the world progressively.
4. The client renders a loading screen until the player's chunk + a 2-chunk radius are
   ready.

### 7.5 Unloading

A chunk is eligible for unloading when:
- No player has it within their view distance.
- It is not within any player's tick distance.
- It has no pending scheduled ticks within the next 200 game ticks.
- It is in ACTIVE state (not DIRTY or SAVING).

DIRTY chunks are saved before unloading. The unload process:

1. Mark chunk as unloading (reject new modifications).
2. If DIRTY, trigger async save.
3. On save completion, remove from memory, release all heap allocations.
4. Entities in the chunk are serialized and stored with the chunk data.

#### 7.5.1 Current engine: the evicted-chunk store (2026-09-27)

The engine has no async per-chunk save yet, so "save before unload" is met by
**keeping** a chunk that must survive instead of dropping it.

**Bug this fixes (audit CRITICAL).** `chunk_stream::stream_chunks` unloaded any
column more than `render_distance + 2` chunks from every player by deleting its
chunks outright, with no save. Re-entry ran `World::generate_column` (pure
world-gen), so the next save or autosave overwrote the edited `.chunk` file with
regenerated terrain. `begin_load` loads every saved chunk at entry, and the far
ones were dropped on the first Playing frame, so a base far from spawn was wiped
by the next save.

**`Chunk::persist`** (runtime-only, not serialised; the `.chunk` byte layout is
unchanged) means "this chunk differs from pure world-gen, or came from a save".
It is set:
- by `World::set_block` when the block ID actually changes, and by
  `World::set_placed` when the placed bit actually flips, **outside world-gen**.
  `generate_column` holds a `worldgen_depth` guard, so terrain, trees, villages
  and structures (including spill into a neighbour's chunks) never set it, and a
  neighbour's existing flag is left alone.
- by `Chunk::from_bytes`, i.e. every chunk from a save, an archive import or the
  network.
- never by light writes (`set_block_light_at`, `set_sky_light_at`, the lighting
  BFS).

**Edited columns** (Phase B2b, 2026-10-07; runtime-only). `persist` cannot
say "edited": a chunk from a save carries it too. A world a hosted server
pushes from also records which **columns** were written outside world-gen
(`World::track_edited_columns` / `take_edited_columns`): the same two
`persist` sites, plus `set_meta` on a change, every block-entity insert and
removal, every `*_at_mut` block-entity accessor, and face-attachment set and
removal — never `generate_column`, `insert_chunk` or a save restore
(`World::without_edit_tracking`). The server turns each into a permanent
"touched" verdict for the chunk push (Spec 04 §4.1 "Touched columns").

**Whole-column eviction.** `World::evict_column(cx, cz)` replaces the old
per-chunk remove. If any chunk in the column has `persist`, all of the column's
chunks move to `World::evicted`. Otherwise they are dropped, because pristine
world-gen regenerates the same. Whole columns stop a restore from mixing
regenerated and restored slices. Side tables (`block_entities` and so on) stay
in `World` as before.

**Restore before generate.** Every re-entry path calls
`World::restore_column(cx, cz)` first and runs `generate_column` only when it
returns false: the `stream_chunks` load branch (which also covers its void-column
self-heal), `step_load`, the spawn-pref pre-generation in `begin_load`, and
`repair_void_columns_after_load`. After a restore, the column goes through the
same steps as a column read from disk: light pass, water/lava/fire rescan, mob
scatter, then mesh. `generate_column` must never run on a restored column,
because it refills chunks that were dug down to all air. An evicted chunk
overwrites any world-gen spill a neighbour left at its position.

**Saves.** Every chunk writer iterates `World::persistable_chunks()`, which yields
the loaded chunks plus the evicted ones, with the evicted copy winning on a
duplicate position. The writers are `partition_chunks_for_save` (so the native
save and the autosave), `write_world_folder`, `world_archive::pack_world` (WASM
IndexedDB and `.axeworld` export) and `GameServer`'s save. The all-air
delete rule (§8.4: read or written this session, and `persist`) applies to
evicted chunks too, on `GameServer::try_save` as well since its review
follow-up: an evicted, edited column is written, never deleted, and a pristine
column dropped on stream-out is in neither set, so its file stays. `World::clear` empties the store, so
nothing leaks between worlds.

**The dedicated server uses the same store (Phase B1, 2026-10-06).**
`GameServer::stream_columns` (Spec 01 §4.1.2) streams columns in and out around
every connected player through the same `chunk_stream::ColumnSims::stream_in` /
`stream_out` the client streamer uses, so a server-side unload is an
`evict_column` too, and `GameServer::try_save` writes the evicted chunks.
The streamer has no disk path: boot loads the whole save through `world_open`
(§8.4 — all or nothing, a torn chunk file kept aside), and streaming restores
from the in-memory store else generates, so it never generates over a column
file it failed to read (it reads none) and never writes or deletes one. Test:
`streaming_a_column_in_and_out_never_touches_a_chunk_file_on_disk`. Paging
evicted columns to disk, when it lands, must treat an unreadable column file as
unloadable (skip, log, leave the file) rather than generate over it.

**An evicted column is a light barrier (bug fixed 2026-10-06).** Light writes
into an evicted column are dropped and its light reads as 0 (block light) from
`World::chunks`. `lighting::bfs_propagate` used to enter it anyway: every cell
it reached still read dark, so it re-queued its neighbours without end, an
exponential blow-up (an 8 GiB allocation, found when the dedicated server
restored a column next to a still-evicted one holding lava; the client could
hit it the same way walking back into an edited area). The BFS now skips
neighbours where `World::is_evicted_at` is true, as the water, lava and fire
sims already did. Restore runs the column's own light pass. Regression test:
`lighting::tests::block_light_stops_at_an_evicted_column`.

**Memory bound.** Only edited or saved columns are kept: the columns a player
changed, plus every column loaded from the save. Pristine columns are still
dropped. The store is in memory and not paged to disk, so native and WASM behave
the same (WASM has no per-chunk file reads). A future option is to page evicted
columns to disk on native and to IndexedDB on WASM, restoring them from there
through the same `restore_column` hook.

**Read-through and write-through.** `World` tracks `evicted_columns`. For an
evicted column, `get_block` and `is_placed` read the evicted chunk, and
`set_block` and `set_placed` write into it (creating an absent cy there) and set
`persist`, so no stray chunk appears in `chunks`. This covers a LAN host
applying a joiner's edit more than rd+2 chunks from every host player. Light
writes to an evicted column are dropped. A column with no loaded or evicted
chunks keeps the old behaviour: it reads as air, and a write creates a chunk.

**Sims stop at a column that is not present (widened 2026-10-06, Phase B1
review).** Water, lava, fire, sapling growth, entity physics and mob AI check
`World::is_column_present_at(x, z)`: true only if one of the column's chunks
in `World::chunks` holds a real block. That excludes an evicted column (its
chunks sit in the store; reads go through to its real blocks), a dropped one,
and one never loaded. An empty chunk does not count, because block-light BFS
leaves light-only chunks in a never-loaded neighbour and `generate_column`
refills those. They treat such a column as a barrier: they don't spread,
ignite, grow a canopy or retract into it. Queue entries and fire cells inside
it are dropped when next processed, and the FIRE and fluid blocks stay in the
chunk. Entities there are frozen (Spec 05 §9.4).

*Cost.* The check runs per entity per tick on client and server, and per fluid,
fire and sapling spread step, so it must be O(1) per chunk. `Chunk` keeps a
non-air cell count, updated by `set` and counted once in `from_bytes` (the only
two places its block array can change), and `Chunk::is_empty` reads it. The
first version scanned cells for a block: about 3,800 reads for a flat world's
floor, which sits in the top layer of its chunk. Measured on a flat world:
about 675 ns per call before, about 52 ns after (a half present, half absent
mix, optimised test profile). The `non_air_count_*` and `chunk_counts_*` tests
pin the count against a full recount through random edits, worldgen, byte
loads, decompression, eviction and restore.

*Bug this fixes.* The barrier used to be `is_evicted_at` only. Cave lava
(y 2-10) at the edge of the loaded area flowed into a never-loaded neighbour,
whose reads are air; `set_block` created a cy 0 chunk there; when that column
streamed in, `generate_column` skipped the non-empty cy 0 (no bedrock or stone
in y 0-15), the void self-heal could not repair it (the same skip), and saves
kept it. A sapling growing at the edge did the same at canopy height. Tests:
`chunk_stream::tests::fluids_never_flow_into_a_never_loaded_column`,
`growth::tests::a_tree_never_grows_into_a_column_that_is_not_loaded`.

*Sources follow the column.* `ColumnSims::stream_out` forgets the column's
water and lava sources (`WaterSystem` / `LavaSystem::forget_column`; sources
are indexed by column in `fluids::SourceSet`). Water registers every water block
of a streamed-in column as a source, so the sets used to grow with every column
ever loaded. `stream_in`'s `register_column_sources` re-adds them. Retraction's
`can_reach_source` treats a fluid cell in a column that is not present as fed,
so forgetting a column's sources never drains the flow they feed across the
border. On restore, `register_column_sources` and `register_column_fires`
re-adopt the column's fluids and fires.

**Testing.** The world side of streaming is split into renderer-free helpers in
`chunk_stream.rs`: `columns_outside_anchors` (via the planner,
`plan_stream_step_for`), `unload_column_blocks` and `load_column_blocks`. `stream_chunks`, `step_load`, the spawn-pref path and the
void repair all call them, and unit tests cover them.

**Remote changes for columns a client does not hold (2026-10-06).** A server
block change lands in a client's world only if the column is loaded or evicted
(`chunk_stream::remote_change_is_loaded`; an evicted one takes it by
write-through). A joiner drops the rest (Spec 04 §4.1). A LAN host keeps them,
because its world is the save of record: `apply_remote_change_to_unloaded_column`
generates the column, applies the change and evicts it, so it streams back in
whole. Before, both wrote the change into a stray chunk that their own
generation later skipped, leaving a 16³ hole.

**Pushed columns are never evicted (Phase B2a, 2026-10-07).** A joined client
generates columns locally, but the server pushes the real chunks round it
(Spec 04 §4.1 "As built"), and a pushed chunk replaces the local one. Pushed
chunks come through `Chunk::from_bytes`, so they carry `persist`; left to the
streamer they would sit in the evicted store and a later restore would bring
back a stale copy. So a joiner never evicts a pushed column: when it unloads
one it drops it outright (`World::discard_column`, its side data too) and tells
the server, which pushes it afresh when it is back in range. A push into a
column the joiner had evicted (its own edits, kept while away) restores the
column first, so no block write lands in the stored copy. A pushed column is
never void-healed or regenerated (its all-air chunks stay air), and is exempt
from `repair_void_columns_after_load`. A column the joiner did not already hold
counts as loaded only once all six of its chunks are in; until then neither the
streamer nor `step_load` generates over it, and one that leaves the joiner's
range half-pushed is dropped the same way (B2a review LOW-2/3).

**Known gaps.** A joined client still generates the columns beyond the server's
push radius itself (and briefly the ones the push has not reached) — unless its
terrain generator differs from the host's: such a joiner generates nothing and
shows only pushed columns (Spec 04 §4.1). The writers that still do not
check `is_column_present_at` can create a stray chunk in a dropped or
never-loaded column at the loaded edge: pistons, dispensers (placed fluid or
fire), `FireSystem::ignite` (flint and steel) and keg blasts. A blast writes
air, which leaves an empty chunk that `generate_column` refills. A save that
already holds a stray chunk keeps its hole: restore never regenerates an evicted
column. Block light from an emitter in a loaded column is not re-propagated into
a neighbour that streams in later, so a torch at a column border leaves a dark
seam on the newcomer's side (a generated column and a restored one alike).

---

## 8. Persistence and Snapshots

### WASM Local Storage (Phase 1α PWA Alpha)

**Status**: Active for Phase 1α. Single-player only, no cloud sync.

On the WASM target, world persistence uses **IndexedDB** accessed via a small JS helper (`tools/website/static/world_store.js`) invoked from Rust via `wasm-bindgen`. Rust does not call `web_sys::IdbFactory` directly — the event-listener boilerplate is prohibitive.

**Database schema** (DB `axenstax_worlds`, version 1):

- Object store: `worlds`
- Key path: compound string `"<pubkey>:<world_name>"` (per-pubkey scoping is the only isolation — no cross-pubkey reads).
- Index: `by_pubkey` on the `pubkey` field for listing.
- Record value:
  ```ts
  {
    blob: Uint8Array,   // packed via pack_world (tar + gzip), same format as native
    meta: {
      name: string,
      size: number,        // bytes in blob
      last_saved: number,  // unix seconds
      game_mode: string,   // "creative" | "survival"
    }
  }
  ```

**Canonical `WorldEntry` shape** (Rust, as returned by `list_worlds_wasm`):

```rust
pub struct WorldEntry {
    pub name: String,
    pub size: u64,
    pub last_saved: i64,   // unix seconds
    pub game_mode: String, // "creative" | "survival"
}
```

**JS API** (`window.AxeStore`):

- `save(pubkey, name, u8array, meta) → Promise<void>`
- `load(pubkey, name) → Promise<Uint8Array | null>`
- `list(pubkey) → Promise<WorldEntry-JSON[]>`
- `delete(pubkey, name) → Promise<void>`

On `QuotaExceededError`, the promise rejects with `"quota"` and the caller surfaces it in the page. (The server-side `/api/wasm-error` log was removed 2026-10-03 — nothing about a player's browser is stored on our server.) No automatic pruning — testers use "Switch user" to clear.

**Serialisation** reuses `pack_world` / `unpack_world` (tar + gzip) from the native save path. No encryption at rest for alpha; post-alpha encrypted-at-rest is a separate design.

**What's explicitly out of scope**: cloud save (Blossom), cross-device sync, native ↔ WASM save migration, import/export.

**Build spec of record**: `docs/superpowers/specs/2026-04-18-pwa-alpha-phase-2.md` §Task 4.

---

#### Current Implementation (Step 11 — Prototype)

The prototype uses a simple format, not the production region-file system described below:
- `worlds/<name>/world.dat` — bincode-serialized metadata (seed, player pos/health/inventory)
- `worlds/<name>/chunks/<cx>_<cy>_<cz>.chunk` — raw LE u16 array (8192 bytes per chunk)
- Save on quit (Escape or window close), load on startup if save exists
- Dependencies: `serde` 1 + `bincode` 1 (not rkyv)
- No compression, no region files, no autosave timer, no CRC, no entity persistence

> **Update (2026-06-03):** the prototype has since grown a periodic autosave and full
> block-entity persistence (the `WorldSave` struct then carried 33 fields; as of 2026-10-04 it has 52 — chests,
> furnaces, vendors, tip-jar escrow, plots, villages, raids, bounties, wallpaper
> overlays, …). Old-save loading is now **backward-compatible for appended fields**
> via the tolerant positional decode described in §8.4 — older saves no longer fall
> through to the lossy `LegacyWorldSave` path. The single shared restore step
> (`save::apply_world_save_state`) is used by every load path (manual load, autosave
> crash-recovery, WASM/cloud) so they cannot drift.

> **Update (2026-06-10 — Rail Freight Phase 1):**
> - **`genesis:track` block — id 260.** Non-solid, transparent, flat-slab mesh
>   (`non_solid_shape_for` + `emit_small_cube`). `TEX_TRACK = 373`;
>   `texture_count()` bumped 373 → 374. Auto-connects to track neighbours via pathing
>   (no rotation/facing field). Full spec: `docs/foundations/2026-06-09-rail-freight-logistics.md`.
> - **`WorldSave.carts: Vec<SavedCart>`** — new trailing `#[serde(default)]` field,
>   appended in strict adherence to the append-only invariant. `SavedCart` wraps
>   `CartData { cell, came_from, progress, speed, facing, cargo: ChestData }`.
>   Old saves (written before this field) load with `carts == []`; the tolerant
>   positional decode (`read_tail`, §8.4) handles the absent tail gracefully.
>   Carts (entities) were not persisted before this release; track blocks persist via
>   normal chunk data as they always have.
> - **Cart items + hull armour (same-day follow-up, 2026-06-10):** three new `MaterialId`s —
>   `WoodCart=153`, `IronCart=154`, `DiamondCart=155` (`MATERIAL_ID_COUNT=156`; fixes a
>   latent `Bellows=152` gap in the `TryFrom<u16>` table). `CartData` gained a trailing
>   `hull: Hull { Wood, Iron, Diamond }` field (`#[serde(default)]`) — **persisted in the
>   save**, hence `PROTOCOL_VERSION` bumped **45 → 46**. Wire is unchanged (hull not
>   broadcast yet). Breach progress is `#[serde(skip)]` — transient only, no save change.
>   Full detail and recipes: `docs/foundations/2026-06-09-rail-freight-logistics.md`.

This will be replaced by the production format below when multiplayer lands.

### 8.1 Save Strategy: Periodic + On-Demand

Axe'n'Stax uses **periodic autosave** rather than a write-ahead log.

**Rationale**: WAL adds complexity and disk write amplification. For a game with ~20 TPS
tick rate, the volume of block changes per second is manageable with periodic flushes.
Most blocks don't change at all between saves.

**Autosave schedule**:
- DIRTY chunks are saved every **60 seconds** (configurable).
- A save pass iterates all DIRTY chunks in oldest-first order, saving up to 64 chunks
  per tick (rate-limited to avoid I/O spikes).
- On graceful shutdown, all DIRTY chunks are saved synchronously.

**On-demand saves**:
- Explicit `/save` command triggers an immediate full save.
- Snapshot requests (Section 8.3) trigger a full save first.

**Emptied chunks must delete their stale file (no resurrection).** In the
per-file native store (`worlds/<name>/chunks/<cx>_<cy>_<cz>.chunk`), a chunk
that becomes all-air (e.g. fully mined out) is *not written* — but its
previously-saved file must be **deleted**, or the mined-out area reloads from
the stale file and resurrects (engine audit 2026-06-04, A#4). `save_world` /
`autosave_world` partition the loaded chunks (`partition_chunks_for_save`) into
write-set + delete-set and `fs::remove_file` the delete-set first. Regression:
`save::save_world_deletes_stale_file_for_emptied_chunk`.

**Save robustness invariants** (engine audit 2026-06-04, A):
- `save_world` / `autosave_world` reject an **empty `players` slice** with an
  `Err` rather than panicking on `player_saves[0]`.
- `restore_inventory` **clamps** each stack `count` to `max_stack` on load, so a
  legacy / hand-edited over-max count can't enter the inventory.
- `sanitize_folder_name` truncates the folder name by **characters**
  (`chars().take(32)`), not bytes — a byte slice panics mid-char on a
  multibyte (accented/CJK) world name.
- **Untrusted cloud/import archives are decompressed under a hard cap.**
  `unpack_world` (the cloud/Stash restore path) gunzips via
  `save::read_bounded(GzDecoder, MAX_IMPORT_DECOMPRESSED_BYTES)` (512 MiB),
  refusing a decompression bomb instead of an unbounded `read_to_end` → OOM.
  The bounded inflated tar transitively bounds every downstream read (tar
  entries + the bincode `world.dat` decode), so a crafted bincode length prefix
  EOFs against the bounded slice rather than over-allocating.

### 8.2 Crash Recovery

Without a WAL, a crash loses unsaved modifications (up to 60 seconds of changes). This
is acceptable for a game -- players are accustomed to periodic save loss.

Mitigation:
- Region files are written atomically: new data is written to a temporary sector, then
  the offset table is updated in a single atomic write. This prevents corruption of the
  region file itself.
- A crash during a save leaves the old chunk data intact (the offset table still points
  to the previous version).
- On startup, a consistency check verifies CRC-32C checksums for all chunk payloads in
  recently-modified region files.
- **Prototype `world.dat` (current alpha):** written **atomically** (temp file → `fsync`
  → rename), so a crash / power-loss / disk-full never leaves a torn `world.dat`. This is
  load-bearing: the positional bincode blob has no length/CRC, and the tolerant decode
  (`read_world_save`, §8.4) cannot distinguish a cleanly-shorter OLD save from a truncated
  NEW one — so a torn blob would silently "load" with its tail (chests, **tip-jar escrow**,
  …) defaulted away. Eliminating torn writes removes that hazard at the source. (Goal 3
  review follow-up; `save::write_atomic`.)
- **Every other prototype file is atomic too, and damage is never papered over**
  (audit 2026-09-27). `world_meta.json`, each `chunks/<cx>_<cy>_<cz>.chunk`, and the
  profile blobs (`profile/skins.blob`, `profile/wardrobe.blob`) are written tmp + rename.
  `world.dat`, `world_meta.json` and the blobs are fsynced per file (`save::write_atomic`);
  chunk files are NOT (thousands per save made autosave a multi-second stall) — they use
  `write_atomic_nosync` plus ONE fsync of the `chunks/` directory, and are written BEFORE
  `world.dat`, which is the save's commit point. A file that fails to decode is renamed to
  `<file>.corrupt-<unix secs>` (`save::quarantine_corrupt`; never overwritten, a `-<n>`
  suffix on collision) and logged:
  - **`world_meta.json`**: a parse failure is NOT replaced with defaults (that used to
    mean seed 42 + survival + genesis reset, then persisted by the next autosave). The
    meta is rebuilt with the seed from `world.dat` (`WorldSave.seed`) and written back,
    conservatively: `cheats_used=true`, `pure_survival=false`, `genesis_block_found=true`
    (the Genesis Block is one-per-world, never findable twice); `WorldSave` holds no game
    mode / world type, so those default. A tear inside a multi-byte character is recovered
    the same way (bytes are parsed, not `read_to_string`). The Proof-of-Play `pop_secret`
    lives ONLY in this file (never in `world.dat`, which travels inside every export), so
    it is salvaged from the damaged bytes (`save::salvage_pop_secret`: a closed
    `"pop_secret": [32 values 0..=255]`, never all-zero; also read back from a quarantined
    `.corrupt-*` copy when the file is missing) and the world keeps its drop placement.
    Only when the array itself is torn does the rebuilt meta get a fresh random secret,
    written with it so later loads don't roll again; it is never None or zero;
    if `world.dat` can't supply it, the world shows as "world info damaged" in the list,
    `load_world` refuses it, and `save_world_meta` refuses every write while the damage
    stands (`save::try_load_world_meta`).
  - **Chunk files**: a torn chunk still regenerates from worldgen, but the damaged
    original is kept aside so no save can overwrite it.
  - **Profile blobs**: a failed load also blocks every save of that blob for the rest
    of the session, so an empty in-memory wardrobe never replaces the player's skins.
- **Tolerant tail stops at the first failure.** In `deserialize_world_save_tolerant`, the
  swallow-on-error tail fields (from `carts` on) stop at the first field that fails:
  bincode is positional, so the cursor is then misaligned and every later field takes
  its default instead of decoding garbage (`save::TailReader`, logs the failing field).
  Because the next save would then drop those fields for good, the loader first COPIES
  `world.dat` to `world.dat.corrupt-<ts>` (once per session, `save::keep_damaged_copy_once`).
- **Native state lives in a per-user data dir, not the CWD** (audit 2026-09-27;
  `data_dir.rs`): `$AXENSTAX_DATA_DIR`, else `$XDG_DATA_HOME/axenstax` /
  `~/.local/share/axenstax` (Linux), `~/Library/Application Support/axenstax` (macOS),
  `%APPDATA%\axenstax` (Windows). `worlds/`, `profile/`, `texturepacks/`,
  `settings.json` and `my_servers.json` all root there; `AXENSTAX_WORLDS_DIR` still
  overrides the worlds folder alone. Migration: unless the root holds a `.migrated`
  marker, legacy CWD state is COPIED (never moved or deleted; modes kept, 0600 keys stay
  0600) into a staging dir beside the root, the marker written last, then renamed into
  place — so a half-copied root never exists. On failure the staging dir is removed, the
  session runs on the legacy CWD state in place, and the next launch retries. An explicit
  `AXENSTAX_DATA_DIR` is used in place and never migrated into: the dedicated-server
  image sets it to its `/worlds` volume (`tools/dedicated-server/Dockerfile`).

### 8.3 Atomic Snapshots

For the production tier, the platform supports **atomic world snapshots** for backup and
world cloning.

**Filesystem backend**:
1. Trigger a full save (all DIRTY chunks flushed).
2. Create a snapshot directory.
3. Hard-link all region files and metadata into the snapshot directory (instantaneous
   on most filesystems, no data copy).
4. Future writes to the original region files use copy-on-write at the sector level
   (write new sectors, update offset table) so the snapshot's hard-linked files remain
   untouched.

**S3 backend**:
1. Trigger a full save.
2. Copy all S3 objects to a snapshot prefix using S3 server-side copy (fast, no data
   transfer).
3. Record the snapshot manifest (list of objects + their ETags) for consistency
   verification.

**Snapshot metadata**:

```rust
struct SnapshotManifest {
    /// Unique snapshot ID.
    id: Uuid,
    /// Timestamp of snapshot creation.
    created_at: u64,
    /// World format version at time of snapshot.
    format_version: u16,
    /// World seed.
    seed: u64,
    /// Number of region files.
    region_count: u32,
    /// SHA-256 of the concatenated CRC-32C values of all chunks (integrity check).
    checksum: [u8; 32],
}
```

### 8.4 Format Versioning

The world format version is stored in three places:
1. The world metadata file (Section 9).
2. Each region file header (Section 4.2).
3. Each snapshot manifest.

**Version numbering**: Simple incrementing `u16`. Current version: 1. (Design, for the
production region format. The as-built prototype `world.dat` carries its own `u32`
format version in a trailing footer — see the bincode note under Migration strategy.)

**Compatibility rules**:
- The engine MUST be able to read any format version <= its built-in version.
- The engine ALWAYS writes the latest version.
- When reading an older version, chunks are lazily upgraded: on load, the old format is
  read; on next save, the chunk is written in the new format.

**Migration strategy**:
- Minor changes (new fields appended): Same version number.
  - **rkyv (production region format):** rkyv's backward-compatible patterns.
  - **bincode (prototype `WorldSave`, current alpha):** `#[serde(default)]` is
    **inert** on this positional, non-self-describing format — an older save that
    lacks a newly-appended field hits EOF mid-struct and the derived `Deserialize`
    fails the *entire* load (it cannot tell "field absent" from "stream ended").
    The alpha mechanism is therefore a **tolerant positional decode**
    (`save::read_world_save` → `save::deserialize_world_save_tolerant`): the required
    field prefix (`seed`..`inventory`) is read strictly, then each appended
    `#[serde(default)]` tail field defaults to empty if the older stream ends before
    it. This delivers the "MUST read older versions" rule above for **append-only**
    changes — the only kind `WorldSave` has ever had. **Invariant: `WorldSave` fields
    may only be APPENDED, never reordered / removed / retyped.** The decoder's struct
    literal lists every field, so a new field that isn't added to it is a compile
    error (it cannot silently drift). Full rationale + the cloud-save blast radius:
    `docs/foundations/2026-06-03-old-save-data-integrity.md`.
  - **Forward compatibility — the format-version footer (gap-audit T1-7, 2026-10-06).**
    The tolerant decode alone let a build open a save from a *newer* build (unknown
    trailing fields ignored) and then silently drop the newer fields on re-save — an
    AppImage rollback lost data. Every `world.dat` is now written as
    `bincode(WorldSave) || format_version: u32 LE || b"AXSAVEv1"`
    (`save_format::encode_world_save`, the one encoder: native save, autosave, the
    dedicated server's `GameServer::try_save`, and `world_archive::pack_world` for
    web / cloud / `.axeworld` / `.axeprofile` / replay).
    - **A footer, not a header**: builds from before the footer already ignore
      unknown trailing bytes, so they keep opening new saves exactly as before (no
      worse), while every build from now on can read the version. A header would
      have made new saves unreadable to shipped builds, and a 4-byte header magic
      collides with a legacy save's leading `seed: u32`; an 8-byte trailing magic
      at the very end of a footer-less save is not a realistic collision.
    - **`SAVE_FORMAT_VERSION`** = `WORLD_SAVE_FIELD_COUNT` (52 today) +
      `SAVE_LAYOUT_REVISION` (0). Two tripwire tests stop drift: serde's field list
      for `WorldSave` must equal `WORLD_SAVE_FIELD_COUNT` (so appending a field
      without bumping the version fails), and the tolerant reader must consume every
      byte of a current save (so a field listed in its struct literal without a read
      fails). A wire change that is **not** an appended field (a field or enum variant
      inside a nested saved type, a retype) is invisible to both tripwires and must
      bump `SAVE_LAYOUT_REVISION` by hand.
    - **Read** (`save::read_world_save`): no footer → the legacy path, unchanged.
      Footer version ≤ ours → the footer is stripped *before* the tolerant decode
      (left on, an older save's footer would be decoded as the field it lacks), then
      decoded as before. Footer version > ours → `WorldSaveError::NewerVersion`,
      nothing decoded.
    - **Refusal is total**: `save::world_open_refusal` (reads only the 12-byte
      footers of `world.dat` and `autosave/world.dat`) is checked before a world is
      entered from every path — world card Play/Host, "Host online" (before any port
      or router mapping), the Workshop / Trial / online host catch-all in the lobby
      loop, `start_online_host`, and the dedicated
      server's boot (exit 1). The lobby card is labelled "(needs a newer version)"
      and the banner reads *"This world was saved by a newer version of Axe'n'Stax.
      Update the game to open it."* The web build shows the same banner when the
      IndexedDB / cloud blob fails to unpack for this reason. And every native writer
      (`save_world`, `write_world_folder`, `autosave_world`, `GameServer::try_save`,
      `save_world_meta`) refuses the folder first, before any chunk is written —
      belt and braces behind the load-failure rule below.
      The damaged-meta recovery does not quarantine or rebuild a newer world's meta.
    - **An unreadable `world.dat` is refused the same way.**
      `save_format::file_footer_version` returns `Err` for any I/O error other than
      "not found" (permissions after a restore, a directory in its place) — the
      version is unknown, so it is not "a save without a footer". The refusal is
      `WorldSaveError::Unreadable`: `world_open_refusal` refuses the world ("This
      world couldn't be opened: world.dat can't be read (…). Nothing was changed."),
      the lobby card is labelled "(can't be opened)", and every writer refuses it.
- **The load-failure rule (2026-10-06): a world that fails to load is never
  replaced by a freshly generated one** — native client and dedicated server.
  Before, any load error only logged a warning and generated a fresh world, which
  was marked live and saved (5-min autosave, Save & Quit, window close, the
  server's tick-0 save): the save deleted every chunk file that was all-air in the
  fresh world, overwrote the spawn-area chunks and `world.dat`, and wrote the meta
  last (so a damaged-meta refusal protected nothing).
  - **"Nothing saved here" vs "saved but failed to load"** (`world_open::open_world`,
    shared by `chunk_stream::begin_load` and `GameServer::initial_load`). A world is
    *on disk* when `world.dat`, `autosave/world.dat` or any `chunks/*.chunk` is
    there. `world_meta.json` alone is a **new** world: the Create dialog and the
    dedicated server's bootstrap write it before the first save. (`autosave/chunks/`
    without `autosave/world.dat` is a torn autosave nothing loads; it doesn't count.)
    A world on disk that fails to load for ANY reason — an I/O error reading
    `world.dat`, meta or a chunk; an undecodable `world.dat`; a chunk file whose name
    isn't three integers; a damaged chunk or meta that can't be kept aside; saved
    chunks with no `world.dat` (the refusal says how to recover: move the chunks
    folder aside and the world starts again from its seed, or restore `world.dat`
    from a backup); damaged world info — is refused: nothing generated,
    the world never marked live, nothing written. The client returns to the lobby
    with *"This world couldn't be opened: <short reason naming the file>. Nothing was
    changed."* (`save_format::unopenable_message`, via `leave_world_with_notice`
    with `SaveChoice::Abandon`); the dedicated server exits before its tick-0 save
    with the folder path, the file and why (`HostedServer::start` propagates the
    `Err`), and only bootstraps a new world's meta when `world_open::is_new_world`.
  - **Loads are all or nothing**, so a refused folder is byte-for-byte as it was:
    `load_chunk_dir` reads and decodes every chunk file before touching anything,
    then keeps each damaged one aside (`<file>.corrupt-<ts>`; if one rename fails,
    the earlier ones are moved back and the load fails), then inserts;
    `keep_damaged_copy_once` for a partly decoded `world.dat` runs only after the
    chunks loaded; the torn-meta recovery reads `world.dat` for the seed BEFORE it
    quarantines the meta. These keep-a-copy-aside recoveries still open the world
    when the rest loads — no data is lost, so they are not refusals.
  - **Reading a world never writes (review 2026-10-06).** The torn-meta recovery
    (quarantine + rebuilt meta) used to run whenever the meta was READ — the lobby
    list, every `load_world_meta`, the dedicated server's boot, and the top of
    `open_world` — so a world then refused for another reason had already been
    changed. Now every reader uses `save::peek_world_meta`, which rebuilds the meta
    in memory only (seed from `world.dat`, conservative flags, the PoP secret
    salvaged or `None`, never a freshly minted one), and `open_world` runs the
    repairing `try_load_world_meta` LAST, once the whole load has succeeded. If that
    repair fails the world is refused; a damaged chunk or autosave the load had
    already kept aside then stays aside (moved, never lost). The session adopts
    the PoP secret the repair persisted (`persist_pop_secret_if_missing`).
  - **Autosave fallback.** The client opens the crash-recovery autosave FIRST only
    when it was written after `world.dat` (third review, 2026-10-06 —
    `world_open::autosave_is_newer`): neither file records when it was saved, and
    each is written whole by tmp + rename, so their modification times are
    compared — strictly later wins, a tie goes to `world.dat`, and when either time
    can't be read the autosave goes first (the old rule). An autosave OLDER than
    `world.dat` is stale (a save that committed but failed later, on a build that
    didn't yet drop it at the commit; or a dedicated-server save, which never
    clears one): it used to be preferred anyway, rolling the newer save back. It
    is now cleared once `world.dat` has opened (left, its stale chunk files would
    mix into the next autosave). If the copy opened first fails to load (and is
    not a newer build's), the other is opened instead: a damaged autosave folder
    is renamed to `autosave.corrupt-<ts>`, so neither the next autosave nor the
    leave-time `clear_autosave` can destroy it; a damaged `world.dat` newer than
    the autosave is COPIED to `world.dat.corrupt-<ts>` and left in place (the lobby
    lists only folders with a `world.dat`), because the session's next save
    overwrites it (`OpenedFrom::AutosaveAfterLastSaveFailed`). A toast says which
    copy the player got (an autosave recovery is announced too). If that rename or
    copy fails, or the other copy fails too, the world is refused. An autosave the world opened FROM is kept until a
    save lands (review 2026-10-06): it used to be deleted as soon as the world
    opened, and with a damaged `world.dat` it is the only good copy. "Quit without
    saving" keeps it too, for the whole session until a save lands
    (`world_exit::SessionSaves`, below). The server never reads the autosave (it never
    clears one, so preferring it would shadow every later server save); a folder
    holding only an autosave is refused there ("open the world in the game once to
    recover it").
  - **A save never deletes a chunk file it didn't read or write.** `World` tracks the
    coordinates whose `.chunk` file this session read in (`chunks/` or
    `autosave/chunks/`) or wrote (`note_disk_chunk`); `partition_chunks_for_save`
    queues an all-air chunk's file for deletion (the mined-out case, engine audit
    2026-06-04 A) only for those. A file the session never read is unknown data and
    is left alone. The set survives a Workshop reset (same folder) and is forgotten
    on a world change. The delete is further gated on the chunk being `persist`
    (read from disk, or really edited): `World::set_block` creates an empty chunk
    when a block is set to air where the streamer had dropped the column, and air
    over air changes nothing, so that conjured chunk is never `persist` and never
    deletes the real file under it (review 2026-10-06; before, the next save
    left a hole after the restart). The dedicated server applies the same rule
    (`GameServer::try_save`, review 2026-10-06): it never deleted any, so a
    mined-out chunk came back after a restart.
  - **Every save checks before it touches anything**: `save_world` and
    `write_world_folder` run the newer/unreadable check AND the damaged-meta check
    (`meta_write_blocked`) first — before the stale-chunk deletes, chunk writes and
    `world.dat`.
  - **A world's first save stages its chunks, commits with `world.dat`, then
    publishes** (`save::write_first_save`; `save::is_first_save`: no `world.dat` and
    no `chunks/*.chunk` yet; used by `write_world_folder`, so `save_world` and the
    imports, and by `GameServer::try_save`). Every loader marks a column with ANY
    saved chunk as loaded and never generates the rest of it, so a first save that
    wrote its chunks one by one into `chunks/` and was cut short left permanent
    holes — and writing `world.dat` first (the previous rule) still left holes. The
    order now:
    1. every non-empty chunk is written into `chunks.new/` (one left by an earlier
       first save cut short is cleared first — it was never committed), then one
       directory fsync;
    2. `world.dat` is written atomically — the commit point — then
       `world_dat_committed`: one directory fsync of the world folder (so the
       rename in step 3 can never be durable without `world.dat`), the live
       session's save drops the autosave (below), and the staged chunks are noted
       as on disk (`note_disk_chunk`) — all before anything that can still fail.
       Noted only after step 3 (the previous rule), a publish that failed (a
       Windows antivirus lock on the rename) left the next save to publish them
       without knowing they were on disk, so a chunk mined out since came back;
    3. `chunks.new/` is renamed to `chunks/` (`publish_staged_chunks`): an empty
       `chunks/` is removed first, one holding anything else (a stray `.tmp`) is
       kept aside whole as `chunks.corrupt-<ts>`, and it never publishes over a
       saved chunk.

    A chunk that can't be written fails the save before `world.dat`. Cut short
    before step 2 there is no `world.dat` and no `chunks/*.chunk` — the world is
    still new (`chunks.new/` does not count as saved), and the next save is a first
    save again. Cut short after step 2, the next native load of `world.dat`
    (`load_world`, after the decode) or the next save (`save_world`,
    `write_world_folder`, `GameServer::try_save`) finishes step 3
    (`finish_staged_first_save`), before anything is read or deleted; a
    `chunks.new/` without a `world.dat` is left alone. A `chunks.new/` beside a
    `world.dat` AND saved `chunks/*.chunk` is stale — after a downgrade, an older
    build without staging saved over a committed first save it couldn't see, so
    the `chunks/` beside `world.dat` is newer — and is set aside whole as
    `chunks.new.stale-<ts>` (third review, 2026-10-06: it was refused at every load
    and save, forever). Every later save still writes its chunks into `chunks/`
    first and `world.dat` last (its commit point). Tests cut a first save short at
    every chunk boundary and just before the publish (`save::FIRST_SAVE_CUT`) and
    check that the folder is new or whole — and one lets the next SAVE, with no
    open in between, finish the publish and delete a chunk mined out since.
  - **A failed save is never silent (review 2026-10-06).** Pause → Save clears the
    crash-recovery autosave only once the save landed (it was cleared after a
    failed save too); any failed save shows *"Couldn't save: <reason>. Your last
    save is safe."* (`world_exit::save_failed_toast`, drawn over the pause menu as
    well). `leave_world` returns whether the player left: when its save fails —
    Save & Quit, Trial Leave, the end cards, the J-board arena hop, the skin-paint
    hop, the window close — the player stays in the world, still live and nothing
    torn down, with that toast, to retry or deliberately "Quit without saving".
  - **A failed save never funnels the player into deleting the autosave** (second
    review, 2026-10-06). `world_exit::SessionSaves` on `GameState` records what the
    session's saves have left the crash-recovery autosave guarding: `save_failed`
    (the last save failed, so the autosave may be the newest copy there is),
    `opened_from_autosave` (set in `chunk_stream::begin_load`) and
    `close_save_failed`. A save that lands resets it; every world entry and exit
    does too. While `keeps_autosave()` (`save_failed || opened_from_autosave`) holds,
    "Quit without saving" (`SaveChoice::Discard`) keeps the autosave, like a
    window close or a dropped connection does (`should_clear_autosave(.., keep_autosave)`),
    and the pause menu says so rather than promising a discard — the button reads
    *"Quit — your autosave from <age> is kept"* with *"Anything since your autosave
    from <age> will be lost."* under it (`save::autosave_age`). A failed
    window-close save (`GameState::close_window`) sets `close_save_failed` and
    appends *"Close the window again to try once more and quit — if the save fails
    again, your autosave from <age> is kept."* (or *"… the game quits without
    saving."* when none is kept) to the toast. The NEXT close tries the save once
    more and quits either way (`world_exit::after_close_save`): when it fails again,
    by a `SaveChoice::Abandon` that keeps the autosave — so a save that keeps
    failing never traps the player in the world, and a close never quits without
    first trying to save (third review, 2026-10-06: the flag used to make every
    later close quit unsaved, however much was played since). Any save that lands
    clears the flag.
  - **Every save of the live session supersedes the autosave the moment its
    `world.dat` commits** (`save::save_world_superseding_autosave` →
    `AtCommit::DropAutosave`, via `GameState::save_live_world`): the pause-menu
    Save, Save & Quit and every other exit that saves, a resumable scenario's start
    and the replay snapshot — the last two used to leave a stale autosave behind,
    which the loader preferred and so rolled the fresh save back after a crash. It
    is dropped at the commit, before the publish and the meta write (third review,
    2026-10-06): a save that failed AFTER the commit kept the older autosave and
    the next open preferred it over the newer `world.dat`. A save that fails
    BEFORE its commit keeps the autosave. `clear_autosave` removes
    `autosave/world.dat` (its commit point) first, so a clear that fails part-way
    never leaves an autosave that loads with some of its chunks gone. Imports
    (`write_world_folder`) and the dedicated server (`write_first_save` with
    `AtCommit::KeepAutosave`) leave it alone.
  - **An import never writes into a taken name** (review 2026-10-06). `.axeworld`
    and `.axeprofile` imports name against EVERY entry under the worlds root, not
    the lobby list (which shows only folders with a `world.dat` and hides the
    Workshop), so they can't write into the native Workshop, a chunks-only or
    autosave-only folder, or over a stray file; `write_unpacked_world` also refuses
    an existing folder outright. An import that fails part-way removes the folder
    it created (it is the import's own, checked above), so a half-written world is
    never left for the lobby to list once `world.dat` is in it.
  - **Proof-of-Play secret**: a legacy meta without `pop_secret` gets one in the
    lobby (`apply_world_seed`), but on native it is persisted only once the world
    has opened (`persist_pop_secret_if_missing` in `begin_load`), so a refused world
    is never written.
  - **Trial arenas** are planned from the disk BEFORE anything is wiped or written
    (`world_open::plan_arena_folder`, native): `world_open_refusal` first, then
    `is_new_world` — anything saved counts, not just a folder the lobby lists
    (one with a `world.dat`). A refused arena is neither wiped, re-created nor
    entered. A Resume arena holding chunks but no `world.dat` is resumed (and so
    refused by the open, untouched) — it used to get a fresh meta written over its
    real one first; a Reuse arena in that state is wiped and regenerated instead of
    refused on every launch. The web still plans from its lobby list.
  - **Web**: the IndexedDB / cloud load already stayed in the lobby on failure (the
    fresh-world branch runs only when there is no record at all), but it now shows
    the notice for every failure, not just a newer build, and the play path unpacks
    with `world_archive::unpack_world_to_play`, which refuses a damaged chunk rather
    than skipping it — skipped, the chunk would regenerate and the next save would
    repack the record without the original (the web has no side file to keep it
    in). Import, backup download and replay stay lenient (the archive itself is
    untouched there).
  - **Known, not fixed (open design call).** Only a world's FIRST save is
    all-or-nothing. A LATER save cut short can still leave a partial NEW column —
    world-generation chunks of a column first saved in that save, written one by one
    into `chunks/` — the same hole class (the loaders never generate the rest of a
    column that has any saved chunk). Closing it needs a manifest or a loader that
    can tell a complete column from a partial one. And `World::set_block` still
    auto-creates a chunk for an unloaded cell (known debt), so a non-air write into
    a column the streamer had dropped can overwrite the real file.
  - The first **non-append** change to `WorldSave` itself (reorder / removal /
    retype), and the bincode 1 → 3 move, need a real migration keyed on this
    version.
- Major changes (layout reorganization): New version number, migration code in
  `migration::v{N}_to_v{N+1}` module. Lazy per-chunk migration on load.
- Full-world migration tool: `axenstax-migrate` CLI that reads an entire world and
  rewrites it in the latest format. Optional but recommended before major engine updates.

### 8.5 Sleeping Worlds (Personal Tier)

When a personal world has no connected players:

1. All DIRTY chunks are saved.
2. All chunks are unloaded from memory.
3. The world runtime (tick loop, entity systems, etc.) is shut down.
4. The world's process/task is terminated.
5. Only the region files and metadata file remain on disk.

**Cost**: Storage only. A typical personal world with 500 explored chunks (common for
casual play) consumes approximately:

```
500 chunks * ~4 KiB compressed average = ~2 MiB on disk
+ metadata file: ~4 KiB
+ block ID table: ~2 KiB
Total: ~2 MiB per sleeping world
```

At $0.023/GB/month (S3 Standard): **$0.00005/month** per sleeping world. Effectively
free.

**Wake-up sequence**:
1. Player connects to the platform, requests their world.
2. Orchestrator assigns a worker node.
3. Worker loads world metadata + block ID table.
4. Worker begins loading chunks around the spawn point / player's last position.
5. Player receives chunks and can begin playing within 2-5 seconds (cold start target).

---

## 9. World Metadata

### 9.1 Metadata File

Each world has a metadata file: `world.dat` (bincode-serialized, zstd-compressed).

```rust
struct WorldMetadata {
    // ─── Identity ───
    /// Unique world ID (UUID v4).
    world_id: Uuid,

    /// Human-readable world name.
    name: String,

    /// World format version.
    format_version: u16,

    // ─── Generation ───
    /// World seed.
    seed: u64,

    /// World generator ID (e.g., "genesis:overworld", "genesis:flat").
    generator: String,

    /// Generator-specific settings (JSON blob for flexibility).
    generator_settings: String,

    // ─── Dimensions ───
    /// Minimum Y coordinate (default: -64).
    y_min: i32,

    /// Maximum Y coordinate (default: 383).
    y_max: i32,

    /// World border radius in blocks (default: 30_000_000).
    world_border: i32,

    // ─── Spawn ───
    /// Default spawn point.
    spawn_x: i32,
    spawn_y: i32,
    spawn_z: i32,

    /// Spawn radius for randomization (blocks).
    spawn_radius: u16,

    // ─── Time & Weather ───
    /// Current world time in ticks (0 = dawn, 24000 = full day cycle).
    world_time: u64,

    /// Current day count.
    day_count: u64,

    /// Whether the day-night cycle is enabled.
    day_cycle_enabled: bool,

    /// Current weather state.
    weather: WeatherState,

    /// Ticks remaining for current weather before next transition.
    weather_duration: u32,

    // ─── Game Rules ───
    /// Game rules as a key-value map.
    game_rules: HashMap<String, GameRuleValue>,

    // ─── Registry ───
    /// Block ID table: maps numeric ID -> namespaced string ID.
    block_id_table: Vec<String>,

    /// Biome ID table: maps numeric ID -> namespaced string ID.
    biome_id_table: Vec<String>,

    // ─── Statistics ───
    /// Total ticks the world has been active (the world-clock; advances only
    /// while the world is being played, NOT wall-clock). The lifetime
    /// "how long this world has been played".
    total_ticks: u64,

    /// Creation timestamp (Unix seconds).
    created_at: u64,

    /// Last played timestamp (Unix seconds).
    last_played_at: u64,

    /// Total play time in seconds.
    total_play_time_seconds: u64,

    // ─── Proof-of-Play stats (universal — present in EVERY world) ───
    // These are core per-world lifetime stats, not a game-mode/scenario
    // feature. Every world anyone creates carries them. Added 2026-06-03.

    /// Cumulative Proof-of-Play **work** done in this world: the sum of each
    /// broken block's work value, incremented on every successful
    /// `can_harvest` break. "How much work has this world done." See Spec 6 §2
    /// and `docs/foundations/2026-06-03-work-based-hashing.md` (work = f(hardness)).
    total_work: u64,

    /// Whether this world's **Genesis Block** — the first Satori mined in it —
    /// has been claimed. One per world (Spec 6 §2.2c.5a).
    genesis_block_found: bool,

    /// The `total_ticks` value at which the Genesis Block was mined (active
    /// world-clock time-to-genesis). `None` until claimed. E.g. a Satori
    /// speedrun reads this as its result.
    genesis_found_at_tick: Option<u64>,
}

enum WeatherState {
    Clear,
    Rain,
    Thunder,
}

enum GameRuleValue {
    Bool(bool),
    Int(i32),
    Float(f32),
}
```

### 9.2 Default Game Rules

```
do_daylight_cycle:    true     -- Advances time
do_weather_cycle:     true     -- Weather changes naturally
do_mob_spawning:      true     -- Hostile/passive mob spawning
do_mob_griefing:      true     -- Mobs can modify blocks
do_fire_spread:       true     -- Fire spreads to flammable blocks
do_tile_drops:        true     -- Blocks drop items when broken
keep_inventory:       false    -- Players keep items on death
mob_spawn_radius:     128      -- Distance from player for mob spawning
random_tick_speed:    3        -- Random ticks per chunk section per tick
max_entity_count:     200      -- Per chunk column entity cap
pvp_enabled:          true     -- Player vs player damage
tnt_explodes:         true     -- TNT is functional
fall_damage:          true     -- Players take fall damage
natural_regeneration: true     -- Health regenerates when food is full
```

### 9.3 Persistence

`world.dat` is saved:
- On every autosave cycle (alongside chunk saves).
- On graceful shutdown.
- On explicit save command.

The save is atomic: write to `world.dat.tmp`, then rename to `world.dat`. This prevents
corruption if the process is killed during the write.

### 9.4 World Directory Layout

```
world/
  world.dat                  -- World metadata (Section 9.1)
  world.dat.bak              -- Previous save (rotated backup)
  region/
    r.0.0.gbr                -- Region files (Section 4)
    r.0.-1.gbr
    r.-1.0.gbr
    ...
  entities/
    e.0.0.gbe                -- Entity data per region (same 32x32 grid)
    ...
  players/
    {uuid}.dat               -- Per-player data (position, inventory, health)
    ...
  plugins/
    {namespace}/
      data/                  -- Plugin-specific persistent data
        ...
```

**Entity storage note**: Entities (mobs, items, vehicles) are stored in separate entity
region files (`*.gbe`) rather than alongside chunk block data. This separation avoids
rewriting chunk block data when only entities have changed (entities change far more
frequently than blocks). The entity region file format mirrors the chunk region format
(Section 4.2) but payloads contain serialized entity lists instead of chunk columns.

### 9.5 Profile Bundle — `.axeprofile` (delivered 2026-09-06)

**Purpose.** Web saves live in the browser (IndexedDB) and are local-only by design. A
player moving to the desktop app needs to carry the **whole profile** across in one go —
every world *and* their Trials records — not one world at a time.

`.axeprofile` is a **container**, not a second world format. A world entry carries the
**exact `.axeworld` bytes** produced by the existing archive packer (`world_archive::pack_world`
— Section 8), so the two can never drift. Implementation: `game/engine/src/profile_bundle.rs`
(cross-platform, no dependencies).

**Byte layout** (little-endian throughout):

```
offset  size  field
0       8     magic          b"AXEPROFL"
8       1     format version u8 (currently 1)
9       4     entry count    u32
then, `count` times:
        1     kind           u8   (1 = World, 2 = Trials; other values reserved)
        4     name length    u32  (bytes of UTF-8)
        n     name           UTF-8
        4     payload length u32
        m     payload        raw bytes
```

| Entry kind | `name` | Payload |
|---|---|---|
| `World` (1) | the world's name as saved in the browser | a `.axeworld` archive, verbatim |
| `Trials` (2) | a label (readers key off `kind`) | `TrialBests::to_json` |

**Reader rules.**

- **Unknown `kind` bytes are skipped, not fatal.** The length fields describe every entry
  exactly, so the reader steps over an entry it doesn't understand and keeps its framing.
  A *new entry kind therefore needs no version bump*; bump `format version` only for a
  layout change.
- **A short read anywhere is a clean `Truncated` error**, never a panic. A payload length
  above 512 MiB is rejected before allocation (corrupt-input guard).
- **A version the build doesn't know is refused by name** ("made by a newer version …"),
  never guessed at.

**Import semantics (native).** Each `World` entry is written as a new world folder; the
`Trials` entry is **merged** into the local store keeping the better of each record (fewer
ticks wins on a race best, completed challenges are a union, the faster rival ghost wins).

**Conflict policy — INTERIM.** A world whose slug already exists in `worlds/` is imported
as `<name> (web)`, then `(web 2)`, … It never overwrites and never silently skips; the
summary states imported / renamed / failed counts. This is a **placeholder for an owner
decision** (last-writer-wins by save version vs merge vs prompt — see `docs/roadmap.md`,
"Take your worlds to native"), implemented at exactly one site
(`native_world_io::keep_both_name`) so it is cheap to replace.

**Boundary.** Purely local: a browser download on one side, an OS file picker on the
other. No upload, no server call, no identity — the same posture as `.axeworld`.

---

## Appendix A: Size Estimates

| Scenario | Explored area | Compressed disk size | Memory (loaded) |
|----------|--------------|---------------------|-----------------|
| New personal world (1 hour play) | ~200 columns | ~1 MiB | ~12 MiB |
| Active personal world (50 hours) | ~2,000 columns | ~10 MiB | ~40 MiB* |
| Social world (200 players) | ~50,000 columns | ~250 MiB | ~200 MiB* |
| Event world (5,000 players) | ~200,000 columns | ~1 GiB | ~800 MiB* |

*Memory figures assume only chunks within max view distance of active players are loaded.

## Appendix B: Format Magic Numbers

| File type | Extension | Magic bytes | ASCII |
|-----------|-----------|-------------|-------|
| Region (blocks) | `.gbr` | `0x47425246` | `GBRF` |
| Region (entities) | `.gbe` | `0x47424546` | `GBEF` |
| World metadata | `.dat` | `0x47425744` | `GBWD` |
| Snapshot manifest | `.gbsnap` | `0x47425350` | `GBSP` |
| Profile bundle | `.axeprofile` | `AXEPROFL` (8 bytes, ASCII) | see §9.5 |

## Appendix C: Glossary

- **Block state ID**: The 32-bit value combining a 16-bit block type ID and a 16-bit
  state index. Uniquely identifies a specific block configuration.
- **Chunk section**: A 16x16x16 cube of blocks, the fundamental unit of block storage.
- **Chunk column**: A vertical stack of chunk sections sharing the same (X, Z) chunk
  coordinates.
- **Region**: A 32x32 grid of chunk columns, the unit of disk I/O.
- **Palette**: A lookup table mapping small indices to block state IDs, enabling
  compressed storage when a chunk contains few distinct states.
- **Tick distance**: The radius around a player within which chunks are actively
  simulated.
- **View distance**: The radius around a player within which chunks are loaded and
  visible but not necessarily simulated.
- **Sleeping world**: A world with no connected players whose runtime has been shut down,
  persisting only as files on disk.

# Step 6 — Flowing Water & Leaf Decay

**Date:** 2026-03-29
**Source:** Axolittle playtest transcript 2026-03-29 via Games Master voice session
**Priority:** Water flow 8/10, Leaf decay 6/10

## Context

Axolittle tested the infinite world build (step 3-5). Chunk loading is smooth, no stutter, no frame drops. Two missing features stood out:

1. Water doesn't flow — it stays where generated. Should spread like Minecraft Java.
2. Leaves don't decay when logs are removed — should break up like Minecraft.

## Feature 1: Flowing Water (Simple Spread)

### Behavior

- **Source blocks** are water blocks placed by world generation or by the player.
- Source blocks spread to adjacent air blocks: 4 cardinal directions + downward.
- Water always tries to flow **down first** (gravity preference).
- Horizontal spread limited to **7 blocks** from the nearest source.
- All water renders at **full block height** (no partial-height flow levels this round).
- Removing or replacing a source block causes all water that depended on it to retract.
- Placing a solid block in water displaces that water block.
- Breaking a block adjacent to water allows water to spread into the gap.

### Speed

- Water spread processed at **5 Hz** (every 4 ticks at 20 TPS), matching the existing falling-blocks cadence.
- Each tick processes a bounded number of spread operations to avoid frame spikes.
- Spread is BFS-based: one layer of spread per tick, so water visibly expands outward over ~1.4 seconds to reach full 7-block range.

### Data Model

- `water_sources: AHashSet<(i32, i32, i32)>` — tracks all source block positions (world-gen + player-placed).
- `water_spread_queue: VecDeque<(i32, i32, i32, u8)>` — pending spread operations with distance from source.
- No new block types needed. Existing `WATER` (ID 6) is reused for all water blocks.
- Source vs spread water distinguished by membership in `water_sources` set.

### Spread Algorithm

Each water tick:
1. Process entries from `water_spread_queue` (budget: 64 blocks per tick).
2. For each entry `(x, y, z, dist)`:
   a. If block below is air → set to water, enqueue `(x, y-1, z, 0)` (distance resets on downward flow).
   b. If `dist < 7`, check 4 cardinal neighbors. For each air neighbor → set to water, enqueue with `dist + 1`.
3. When a source is removed: BFS from that position, remove water blocks that can't trace a path back to any other source within 7 horizontal blocks.

### Retraction Algorithm

When a water source is removed (block broken or replaced):
1. Collect all water blocks in a 7-block radius that aren't sources.
2. For each, BFS through water to find a remaining source within 7 horizontal steps.
3. Water blocks that find no source → set to air.
4. Process retraction over multiple ticks (budget: 64 blocks/tick) for visual effect.

### Interaction with Existing Systems

- **Swimming/underwater physics**: Unchanged. `world.is_water()` already checks block type.
- **Mesh generation**: Unchanged. `build_water_mesh()` already handles water blocks.
- **Falling blocks**: Sand/gravel falling into water displaces the water (becomes air, sand takes position).
- **World generation**: Existing ocean/cave water blocks registered as sources on load.
- **Chunk boundaries**: Spread operations use `world.set_block()` which handles cross-chunk updates.

## Feature 2: Leaf Decay

### Behavior

- Leaves must be connected to a log block within **4 blocks** (via other leaf blocks) to survive.
- When a log is broken, nearby leaves are queued for a support check.
- Unsupported leaves decay (become air) with a random delay for natural appearance.
- Decay is cosmetic only this round — no item drops.

### Speed

- Support check runs at **5 Hz** alongside water spread.
- Decay delay: random **5.4–21.6 seconds** per leaf after determined unsupported (tuned from Axolittle playtesting — original 0.6-2.4s felt too fast).
- Leaves decay one at a time (not all at once) for the Minecraft visual feel.

### Algorithm

On log break:
1. Scan all leaf blocks within a 5-block radius of the broken log.
2. For each leaf, queue a support check.

Support check (BFS):
1. From the leaf, BFS through adjacent leaf blocks (6 directions).
2. If BFS reaches a log within 4 steps → leaf is supported, remove from queue.
3. If BFS exhausts 4 steps without finding a log → leaf is unsupported.
4. Unsupported leaves added to `decay_queue` with a random delay timestamp.

Decay tick:
1. Check `decay_queue` entries whose delay has elapsed.
2. Set block to air, collect dirty chunks, rebuild meshes.
3. Budget: 8 leaves per tick to spread visual effect over time.

### Data Model

- `leaf_check_queue: Vec<(i32, i32, i32)>` — leaves pending support check.
- `leaf_decay_queue: Vec<(i32, i32, i32, f32)>` — unsupported leaves with decay timestamp.
- No new block types or metadata needed.

### Interaction with Existing Systems

- **Tree generation**: Unchanged. Trees still place logs + leaves normally.
- **Block breaking**: Hook into existing break handler — if broken block is `OAK_LOG`, trigger leaf scan.
- **Mesh generation**: Standard rebuild via dirty chunk system.
- **Inventory**: Decayed leaves don't drop items this round (future: saplings, sticks, apples).

## Not In Scope

- Partial-height water (flow levels 1-7 with decreasing visual height)
- Water current / player push mechanics
- Water + lava interaction (cobblestone/obsidian generation)
- Leaf item drops (saplings, sticks, apples)
- Waterlogged blocks (slabs, stairs holding water)
- Water sound effects

## Files Modified

| File | Changes |
|------|---------|
| `main.rs` | Water spread tick, leaf decay tick, source tracking, queue processing |
| `world.rs` | `register_water_sources()` for world-gen sources, helper methods |
| `block.rs` | No changes needed |
| `mesh.rs` | No changes needed |
| `physics.rs` | Sand/gravel displaces water on fall |

## Test Plan

See test sheet: `docs/test-sheets/2026-03-29-step6-water-leaf-decay.md`

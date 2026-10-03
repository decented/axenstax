# Step 7 — Entity System & Mob Rendering

**Date:** 2026-03-29
**Source:** Staxolottle directive + Axolittle feedback (rounds 1-3)

## Context

The engine has terrain, blocks, water, trees, inventory. No entities exist yet — no mobs, no NPCs, nothing alive in the world. This is sub-project 1 of the Mobs + Combat feature set: get the entity infrastructure in place and render mobs as coloured boxes.

## Scope

- **hecs ECS** for entity storage and component queries
- **3 mob types**: Cow (passive), Zombie (hostile), Chicken (passive)
- **Coloured box rendering** — reuse existing overlay shader pipeline (WireVertex format, TriangleList topology, depth-tested)
- **Basic entity physics** — gravity, ground detection, no AI movement
- **Test spawning** — scatter mobs on terrain surface during world generation

## Not In Scope (sub-project 2+)

- AI behaviour (wander, chase, flee, attack)
- Pathfinding
- Combat (health, damage, melee)
- Mob spawning rules (light level, mob cap, day/night)
- Day/night cycle
- Entity model rendering (skeletal, textured)
- Sound effects for mobs
- Loot drops
- Villagers, reputation, raids

## Mob Definitions

| Mob | Category | Size (W x H x D) | Colour RGB | Health |
|-----|----------|-------------------|------------|--------|
| Cow | Passive | 0.9 x 1.4 x 0.9 | `[0.55, 0.27, 0.07]` brown | 10 |
| Zombie | Hostile | 0.6 x 1.95 x 0.6 | `[0.3, 0.5, 0.2]` dark green | 20 |
| Chicken | Passive | 0.4 x 0.7 x 0.4 | `[0.9, 0.9, 0.9]` white | 4 |

## Architecture

### ECS (hecs)

Add `hecs = "0.10"` to Cargo.toml. Use `hecs::World` alongside the existing block `World`.

**Components:**

```rust
struct Position(glam::Vec3);      // foot position (bottom-centre)
struct Velocity(glam::Vec3);      // blocks/tick
struct MobKind(MobType);          // enum: Cow, Zombie, Chicken
struct Hitbox { w: f32, h: f32 }; // width and height (depth = width)
struct OnGround(bool);
```

### Entity Physics

Run at 20 TPS in the game tick, after player physics. Per entity:
1. Apply gravity: `velocity.y -= 0.08`
2. Apply drag: `velocity *= 0.91`
3. Move: `position += velocity`
4. Ground collision: snap to block top if overlapping solid block below feet
5. Set `OnGround` flag

No horizontal movement (no AI yet). Entities just stand where spawned, affected by gravity.

### Entity Rendering

Reuse the existing wire pipeline (overlay.wgsl `vs_main`/`fs_main`, WireVertex format, TriangleList topology, depth-tested with camera bind group). This pipeline already exists and renders coloured triangles in world space.

Each frame:
1. Query all entities with `Position`, `MobKind`, `Hitbox`
2. For each entity, generate 36 vertices (12 triangles) for a coloured cube at the entity's position with the entity's size and colour
3. Upload to a single vertex buffer
4. Draw in a render pass after water, before block highlight

### Test Spawning

During `initial_load()`, after terrain generation, scatter mobs on the surface:
- 1 mob per 4 chunks (sparse, ~100 mobs in render distance)
- Random position within chunk, placed on surface block
- Random type weighted: 50% cow, 30% chicken, 20% zombie
- Deterministic placement (hash-based, like trees)

## Files

| File | Action | Responsibility |
|------|--------|----------------|
| `Cargo.toml` | Modify | Add `hecs = "0.10"` |
| `game/engine/src/mob.rs` | Create | MobType enum, MobDef struct, mob registry |
| `game/engine/src/entity.rs` | Create | hecs components, spawn helpers, entity physics tick, cube mesh generation |
| `game/engine/src/renderer.rs` | Modify | Add entity buffer + render pass |
| `game/engine/src/main.rs` | Modify | Add hecs::World to GameState, spawn mobs, tick entities, render entities |

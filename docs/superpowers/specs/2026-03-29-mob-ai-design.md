# Step 8 — Mob AI (Wander + Chase)

**Date:** 2026-03-29
**Source:** Sub-project 2 of Mobs + Combat

## Context

Step 7 added entities as static coloured boxes. Now they need to move — passive mobs wander, hostile mobs chase the player.

## Scope

- **Idle/Wander** for all mobs — stand still 3-8 seconds, then walk to a random nearby point
- **Chase** for hostile mobs (zombie) — detect player within 16 blocks, walk toward them
- **Direct-line movement** — no A* pathfinding yet, just walk toward target with ground following
- **1-block step-up** — mobs can walk up 1-block ledges
- **Edge avoidance** — passive mobs won't walk off edges, hostile mobs will drop up to 3 blocks

## Not In Scope

- A* pathfinding (future: when mobs need to navigate around obstacles)
- Flee behaviour (needs combat/damage system first)
- Attack behaviour (sub-project 3)
- Panic state (needs fire)
- Line-of-sight checks
- Mob sounds

## Mob Speeds

| Mob | Speed (blocks/sec) | Speed (blocks/tick at 20 TPS) |
|-----|-------------------|------------------------------|
| Cow | 1.5 | 0.075 |
| Zombie | 2.28 | 0.114 |
| Chicken | 1.0 | 0.05 |

## AI State Machine

```
Idle(timer) → timer expires → pick random target within 8 blocks → Wander(target)
Wander(target) → reached target or stuck → Idle(new timer)
Chase(player_pos) → player > 32 blocks away or lost → Idle(new timer)

Hostile mobs: every 10 ticks, scan for player within 16 blocks → if found, Chase
```

Timer values: Idle duration 60-160 ticks (3-8 seconds at 20 TPS).

## Movement Logic

Each tick, if mob has a movement target:
1. Compute horizontal direction toward target
2. Set horizontal velocity = direction * speed (no acceleration, just direct speed)
3. Check if next position is walkable:
   - Ground below destination must be solid
   - Air at mob's feet and head height
   - If 1 block higher: step up (set y to block_top + 1)
   - If ground drops: passive mobs stop, hostile mobs continue if drop ≤ 3 blocks
4. If not walkable and not steppable: mob is stuck → return to Idle

## Architecture

Single new file `mob_ai.rs` containing:
- `MobAi` component struct (state enum, timer, target position)
- `tick_mob_ai()` system function that queries all entities with MobAi + Position + Velocity + MobKind
- Speed lookup per MobType
- Wander target selection (random offset, check walkability)
- Chase target acquisition (distance check to player)

## Files

| File | Action | Responsibility |
|------|--------|----------------|
| `src/mob_ai.rs` | Create | AI state machine, movement, wander/chase |
| `src/mob.rs` | Modify | Add speed field to MobDef |
| `src/entity.rs` | Modify | Add MobAi to spawn_mob, import MobAi |
| `src/main.rs` | Modify | Add mod mob_ai, call tick_mob_ai in tick(), pass player pos |

# Tools + Armour Roster — Spec 28e

**Status:** DELIVERED as of 2026-05-22. Armour data layer + crafting recipes shipped on main (commit `2a9a04b`, PR #41) — `Item::Armour` variant + `armour.rs` module + Leather/Iron/Diamond/Satori/Chainmail tiers with Helmet/Chestplate/Leggings/Boots slots. Shears + Fishing Rod shipped on main (commit `bb4f270`, PR #42). Pickaxe/Axe/Sword/Shovel/Hoe ladder is complete Wood→Satori. Per-tier Bow extension (Stone/Iron/Diamond/Satori) shipped under chunk 9 (PR #59). Armour-slot UI (vertical column in inventory overlay) + per-piece durability + `take_damage_with_armour` wiring through mob attacks + creeper blasts + HUD total-points badge shipped 2026-05-22 (`feat/spec-28e-armour-ui`). Phase 10 Axolittle playtest is the only remaining gate.
**Branch (when building):** `feat/spec-28e-armour` off `main` for the armour layer.
**Trigger:** Sub-foundation of [Spec 28 Minecraft-parity content surface](2026-05-20-minecraft-parity-content-surface.md) §5. Depends on 28c Materials for Bronze/Copper interactions (cosmetic only); independent at the type-system level.

---

## TL;DR

The tool ladder is **already complete** — every species × tier exists in the engine; the explorer (28f) demonstrates this. Spec 28e's actual scope is **Armour** plus a handful of missing tool variants (Shears, Fishing Rod).

**Armour** is a new system with four slots (Helmet / Chestplate / Leggings / Boots) and five material tiers (Leather / Iron / Diamond / Satori / Chainmail — the last being uncraftable, mob-drop only). Damage reduction applies before HP subtraction; per-tier value is a clean ladder.

**Scope:** ~1,600 LOC + ~20 textures across 9 phases.

---

## Why this lives here

- Combat without armour is binary: full HP or dead. Armour is the difference between "I survived two skeleton arrows" and "the world hated me". Axolittle's playtest confirms this is the missing slice from the combat loop.
- Tools are mostly done — 28e mostly leans on what's there. The new tool additions (Shears, Fishing Rod) are point fixes.
- Cross-game lift: armour slots + tier-based damage reduction is engine-generic; sister games consume by registering different tier values.

---

## Context pointers

### Existing surfaces

- `crafting.rs::ToolType` — already covers Pickaxe/Axe/Sword/Shovel/Hoe/Bow/FlintAndSteel.
- `crafting.rs::ToolMaterial` — Wood/Stone/Iron/Diamond/Satori.
- `combat.rs` — current damage path applies tool attack damage to entity; no armour reduction step yet.
- `inventory.rs` — slot index 36 used for hotbar; need 4 dedicated armour slots (index 36-39 or a parallel `armour_slots: [Option<ArmourItem>; 4]`).
- `hud_ui.rs` — armour bar rendering hooks needed (mirror health bar).

### New modules

- `game/engine/src/armour.rs` — new module. `ArmourSlot` enum (Helmet/Chestplate/Leggings/Boots), `ArmourMaterial` enum (Leather/Iron/Diamond/Satori/Chainmail), `ArmourItem` struct (slot + material + durability), `damage_reduction(slot, material) -> f32`, `apply_armour_damage(armour, raw) -> f32`.

### Related specs

- Spec 05 §3 — combat damage path; armour reduction inserts before HP delta.
- Spec 08 §4 — Anti-cheat; armour durability must be server-authoritative (BRIDGE while single-player).
- 28c Materials — Leather already exists; Chainmail Ingot deferred.

### Memory pointers

- uk english naming — Armour, not Armor. Defence, not Defense. Colour, not Color.
- axenstax has farming — farming-tier interactions; doesn't affect armour.

---

## Phasing

| # | Phase | Files | LOC | Solo? | Status |
|---|---|---|---|---|---|
| 1 | **This spec** | this doc | — | — | ✓ |
| 2 | Shears — new ToolType; recipe = 2 IronIngot in shears shape. Uses: shear sheep for Wool (already a sheep drop), shear Bee Hive for Honeycomb (28d-tied). | `crafting.rs`, `block.rs` | ~100 | ✓ | DELIVERED PR #42 |
| 3 | Fishing Rod — new ToolType; recipe = 2 Stick + 2 String in L-shape. Right-click on water to cast; deferred bobber + catch sub-system to v2 (just spawn the rod for now). | `crafting.rs` | ~80 | ✓ partial | DELIVERED PR #42 |
| 4 | `ArmourSlot` + `ArmourMaterial` enums + `ArmourItem` struct + tier durability/reduction table | `armour.rs` (new) | ~250 | ✓ | DELIVERED PR #41 |
| 5 | Inventory armour slots — extend `PlayerSlot` with 4 armour slots; serialise/deserialise; equip/unequip via inventory-overlay slot column; broken pieces auto-unequip | `player_slot.rs`, `save.rs`, `craft_ui.rs` | ~250 | ✓ | DELIVERED 2026-05-22 |
| 6 | Crafting recipes — Helmet (5 mat top arc), Chestplate (7 mat T-shape), Leggings (7 mat ∏-shape), Boots (4 mat L-shape). Per tier: Leather (Leather), Iron (IronIngot), Diamond (Diamond), Satori (Satori). Chainmail uncraftable. | `crafting.rs` | ~300 | ✓ | DELIVERED PR #41 |
| 7 | Combat hookup — `take_damage_with_armour` on PlayerSlot runs `damage_after_armour` before `combat.health -=`; mob-attack + creeper-blast paths route through it. Wears 1 durability per piece per landed hit; broken pieces unequip silently. | `combat.rs`, `player_slot.rs` | ~120 | ✓ | DELIVERED 2026-05-22 |
| 8 | HUD armour readout — small egui badge near the hearts showing total armour points (suppressed when zero). | `hud_ui.rs` | ~40 | ✓ | DELIVERED 2026-05-22 |
| 9 | Tests — armour reduction maths, equipment slot picks correct, durability decrements on hit, mismatched-slot no-op, save round-trip | `*.rs::tests`, `test_integration::save_load` | ~300 | ✓ | DELIVERED 2026-05-22 |
| 10 | Axolittle playtest — craft full Iron set, fight a zombie/skeleton, confirm survives where bare would die; durability ticks down on hits | — | — | playtest gate | OPEN |

**Total Phases 2-9:** ~1,450 LOC delivered. Phase 10 is the gate.

---

## §4 — Tier table

```rust
// Armour points per slot per material. Sum across equipped → total reduction %.
// Total reduction % capped at 80%.
fn armour_points(slot: ArmourSlot, mat: ArmourMaterial) -> u8 {
    use ArmourSlot::*;
    use ArmourMaterial::*;
    match (slot, mat) {
        (Helmet, Leather) => 1,
        (Chestplate, Leather) => 3,
        (Leggings, Leather) => 2,
        (Boots, Leather) => 1,
        (Helmet, Iron) => 2,
        (Chestplate, Iron) => 6,
        (Leggings, Iron) => 5,
        (Boots, Iron) => 2,
        (Helmet, Diamond) => 3,
        (Chestplate, Diamond) => 8,
        (Leggings, Diamond) => 6,
        (Boots, Diamond) => 3,
        (Helmet, Satori) => 4,
        (Chestplate, Satori) => 9,
        (Leggings, Satori) => 7,
        (Boots, Satori) => 4,
        (Helmet, Chainmail) => 2,
        (Chestplate, Chainmail) => 5,
        (Leggings, Chainmail) => 4,
        (Boots, Chainmail) => 1,
    }
}

fn damage_after_armour(raw: f32, armour_points_sum: u8) -> f32 {
    let pct = (armour_points_sum as f32 * 4.0).min(80.0);  // 4% per point, cap 80%
    raw * (1.0 - pct / 100.0)
}
```

---

## §10 — Playtest

- Craft full Leather set; fight a zombie. Confirm 4% per-armour-point reduction in damage taken (e.g., zombie hits for 4, reduce to ~3 with full Leather).
- Craft full Iron, then Diamond, then Satori. Confirm survival increasing through tiers.
- Armour durability — take 20 hits; confirm durability ticks down + breaks at 0 + auto-unequips.
- HUD armour bar fills proportionally to armour points.

Questions:
- Is the 4%/point feel right? Too generous (full Diamond is ~75% reduction)?
- Should Helmet protect specifically against falling damage? (Vanilla MC: no for Leather, yes for high tiers via enchant — defer enchant.)
- Chainmail uncraftable — does this feel like a fair mob-drop reward, or should it craft from chain Bronze?

---

## Acceptance — sub-foundation 28e overall

- `./check.sh` ALL GREEN.
- Tests pass.
- Playtest succeeds.
- Player Guide updated (`armour.md`).
- Foundations README updated.
- Shears + Fishing Rod (Phases 2-3) ship together with armour as one PR or split — owner's call at PR time.

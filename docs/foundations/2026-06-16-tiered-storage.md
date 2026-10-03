# Tiered storage + auto-collect — bigger chests, then they hoover

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-community-features`, goal `2026-06-16-community-features-solo-buildout`). Phase 1 feature **#15** (replaces Sophisticated Storage 77.2M / Iron Chests).

**What shipped** (2 increments, `check.sh`-green): (1) **tier ladder** — `ChestTier` enum + capacity ladder (27/36/45/54/72) + `ChestData.tier` (serde-default Wood, append-only) + 4 blocks (264-267) + 8 tinted textures (parameterised chest gen) + recipes (8× tier-material ring + wood-chest centre, `match_recipe` + catalogue cards + consistency/regression tests) + generalised right-click-open + mine-drop + tier-aware chest UI; (2) **auto-collect** — `tick_chest_autocollect` pulls nearby dropped items into Diamond/Satori chests (radius `CHEST_ABSORB_RADIUS`, reuses `try_insert`), throttled ~2 Hz in the single-player tick. **Remaining = Axo playtest** (capacity/recipe-cost feel, which tiers auto-collect + radius) + the deferred upgrade-module economy below.
**Date**: 2026-06-16
**Backlog**: `docs/research/2026-06-15-native-bake-in-feature-backlog.md` §15.

---

## TL;DR

Storage **tiers** with a capacity ladder — the existing wood `CHEST` becomes tier 0, plus Copper / Iron / Diamond / Satori chests with progressively more slots — craftable by ringing the tier's material around a wood chest. The top tiers **auto-collect**: a placed chest hoovers nearby dropped item entities into itself. Builds entirely on the existing chest system (`chest.rs`, `chest_ui.rs`, `BlockEntityData::Chest`).

## Material substitution

The backlog ladder is *wood→copper→iron→gold→diamond→Satori*. **Gold does not exist** in the engine (no gold ore/ingot/block — confirmed in `block.rs`/`item.rs`), and adding an ore+smelting chain is out of scope here. So the shipped ladder is **Wood → Copper → Iron → Diamond → Satori** (all materials already in the game). Gold can slot in later if a gold material is ever added.

## Design (concrete, not cards)

- **`ChestTier { Wood, Copper, Iron, Diamond, Satori }`** (`chest.rs`): pure methods `rows()`/`slots()` (capacity ladder), `display_name()`, `block_id()`, `from_block(BlockId) -> Option<ChestTier>`, `auto_collects()`. Capacity (rows×9): Wood 3/27, Copper 4/36, Iron 5/45, Diamond 6/54, Satori 8/72. (Numbers = playtest-tunable.)
- **`ChestData` gains `tier: ChestTier`** (`#[serde(default)]` ⇒ old saves load as Wood; append-only). `ChestData::for_tier(tier)` sizes `slots` to the tier. Capacity is the slot-vec length — `try_insert` already respects it.
- **Blocks**: 4 new ids (264 COPPER_CHEST … 267 SATORI_CHEST) with `BlockDef`s + textures, registered after `GRAVE` (263). The wood `CHEST` (112) is tier 0.
- **Open generalised**: the right-click handler keys off `ChestTier::from_block(blk)` instead of `== CHEST`; first open creates `ChestData::for_tier(tier)`.
- **UI tier-aware** (`chest_ui.rs`): draws `chest.slots.len()` in the tier's row layout + the tier name; everything else (move/sort/dump/restock) unchanged.
- **Recipes**: each tier chest = 8× its-material ring + a wood `CHEST` at centre (the "upgrade" motif), in `match_recipe` (authority) + a `crafting_catalogue` card each (the consistency test enforces card⇒matcher agreement).
- **Auto-collect** (top tiers): a tick system absorbs nearby `ItemEntity` (within a radius) into an auto-collecting chest, respecting capacity. Pure core reuses `ChestData::try_insert`; the tick scans ecs items near auto-collect chest positions. Server + single-player tick.

## Solo boundary → playtest gate

Solo: tier ladder, capacities, blocks, recipes (matcher + cards + consistency), persistence, generalised open, tier UI, auto-collect tick — all headless-testable. **Playtest**: capacity numbers, recipe costs, which tiers auto-collect + the radius (feel), UI row layout for big chests.

## Deferred (named — the "Sophisticated Storage upgrade slots")

A clean follow-up layer on this tier foundation, **not** placeholders:
- **Upgrade-module items + per-chest install slots** (the inventory-slot UI for modules).
- **Item filters** (whitelist/blacklist what a chest accepts).
- **Stack-size multipliers** (2×–16× per-slot, scaling `max_stack` for the chest).
- **Backpacks** (portable storage).

## Spec maintenance

Spec 05 (Gameplay) inventory/storage section gains the tier ladder + auto-collect note as increments land.

# Materials Expansion — Spec 28c

**Status:** PARTIALLY DELIVERED as of 2026-05-21. Substantial chunks shipped on main: tree-drop saplings (Birch/Spruce/Jungle/Acacia/DarkOak), mob drops Honeycomb/Honey/InkSac/GlowBerry, papyrus + papyrus sheet, sugar / sugar-beet / beetroot / pumpkin / berries / egg / milk-bucket / bowl, all T1.5 baking intermediates (Flour/Dough/Cream/Butter/Cheese/Cake/PumpkinPie/Cookie etc.), copper/tin/sulphur/amethyst/bronze ingots, baked-potato + baked-carrot + baked-corn. Remaining gaps tracked per chunk plan: RawRabbit/RabbitHide (chunk 3 with Horse+Rabbit), Wither Skull (chunk 7), HoneyBottle (chunk 5 with Bee). Surface continues to fill as mob roster lands.

**Branch:** `feat/spec-28c-materials` off `main`.
**Trigger:** Sub-foundation of [Spec 28 Minecraft-parity content surface](2026-05-20-minecraft-parity-content-surface.md) §3. Independent of 28a/28b/28d/28e at the data layer; some materials reference mobs/blocks added by those, so end-to-end usability follows the others.

---

## TL;DR

Add ~30 new MaterialIds to fill the Minecraft-parity gap. Group by source:

- **Stone variants** (block-form, materials drop on mining): Limestone, Marble, Granite, Slate (each as block + drops itself).
- **Ore products**: Copper, Tin (alloy precursors), Sulphur (later: gunpowder), Amethyst.
- **Mob drops** (require 28d mobs in place for the drops to fire): Honeycomb, Honey, Egg (chicken-laid), Ink Sac (squid), Glow Berry (decorative), Spider Silk (rename of String? — keep both).
- **Crafted intermediates**: Copper Ingot, Tin Ingot, Bronze Ingot (Copper + Tin smelted), Paper (existing PapyrusSheet — alias for naming consistency), Sugar (Sugar Cane drop).
- **Plants/farming-adjacent**: Sugar Cane (block + material), Watermelon (block + material), Melon Slice, Pumpkin (block + material), Pumpkin Seeds, Cocoa Beans (jungle wood face — needs 28b Jungle), Beetroot, Beetroot Seeds (UK English — beetroot is distinct from sugar beet, see memory).
- **Decorative**: Bone Block (placed bone), Hay Bale (placed wheat).

**Scope:** ~1,200 LOC (mostly data) + ~30 textures across 8 phases. No playtest gate for the data layer; usability gates are owned by the sub-foundations that consume the materials.

---

## Why this lives here

- Axolittle's intuition: a "Minecrafter" expects ~70-90 distinct items / materials. AxeNStax has 41 + ~65 blocks. The gap is real but most additions are inert data with one or two integration points (drop tables, recipe entries).
- Decoupled from 28a/28b/28d/28e so it can land independently; the missing-integration error is friendly (you can mine Copper Ore but can't make Bronze yet — the recipe lands with 28e).
- Cross-game lift: the MaterialId pattern is engine-generic. Sister games consume per-game subsets via a registration list.

---

## Context pointers

### Existing surfaces

- `item.rs::MaterialId` — append-only; positional bincode means every new variant goes at the end. **Do not** insert in the middle.
- `item.rs::Item::name` — match arm per new variant.
- `item.rs::Item::color` — match arm per new variant.
- `inventory_explorer.rs::ALL_MATERIAL_IDS` — append the new variants AND bump the `material_id_table_matches_enum_size` test count.
- `mob.rs::drops_for` — wire mob drops for honeycomb (bee), egg (chicken), ink sac (squid). These mobs are 28d's job.
- `block.rs` — add Sugar Cane, Watermelon, Pumpkin, Hay Bale, Bone Block, Limestone/Marble/Granite/Slate blocks.

### New modules

None — purely additive into existing modules.

### Related specs

- 28d Mobs — bees / chickens / squid produce the relevant drops.
- 28b Woods — cocoa beans depend on jungle wood.
- Spec 20 (Furnace) — copper-ingot smelting recipe lives in the existing furnace recipe table.

### Memory pointers

- uk english naming — UK English. Beetroot ≠ sugar beet; cocoa beans not "cacao".
- axenstax has farming — Spec 28c's farming-adjacent materials (Pumpkin, Watermelon, Beetroot, Sugar Cane) tie into the farming Tier 1.5+ pipeline owner-reserved at spec foundation level. **Do NOT** wire the planting/growth logic from this spec — the materials are added; the workstation+lifecycle integration is owner-track.

---

## Phasing

| # | Phase | Files | LOC | Solo? |
|---|---|---|---|---|
| 1 | **This spec** | this doc | — | — |
| 2 | Stone variants — Limestone, Marble, Granite, Slate (4 new BlockIds + 4 mine_drop entries). | `block.rs` | ~150 | ✓ |
| 3 | Ore products — Copper Ore (block) + Copper / Tin / Sulphur / Amethyst (MaterialIds + Item::name + Item::color) | `block.rs`, `item.rs` | ~180 | ✓ |
| 4 | Crafted intermediates — Copper Ingot, Tin Ingot, Bronze Ingot, Sugar | `item.rs`, `furnace.rs` (smelt recipes) | ~150 | ✓ |
| 5 | Mob-drop materials — Honeycomb, Honey, Egg, Ink Sac, Glow Berry. Variants added but drops_for wiring stays None until 28d lands. | `item.rs` | ~80 | ✓ |
| 6 | Farming-adjacent — Sugar Cane (block + material), Watermelon (block + material + Melon Slice material), Pumpkin (block + material + Pumpkin Seeds), Beetroot (block + material + Beetroot Seeds), Cocoa Beans | `block.rs`, `item.rs` | ~300 | ✓ data only — owner reserves farming planting/growth |
| 7 | Decorative — Bone Block, Hay Bale, Amethyst Block | `block.rs` | ~80 | ✓ |
| 8 | Tests — explorer enumerates new materials, mine_drop returns expected output, smelt recipes resolve, MaterialId variant count matches `ALL_MATERIAL_IDS` | `*.rs::tests` | ~200 | ✓ |

**Total Phases 2-8:** ~1,140 LOC. All solo-shippable.

**Carry-over after merge:** update `inventory_explorer::ALL_MATERIAL_IDS` + the count test in the same PR (the explorer relies on this table being exhaustive).

---

## §2 — Stone variants

```rust
pub const LIMESTONE: BlockId = 81;
pub const MARBLE: BlockId = 82;
pub const GRANITE: BlockId = 83;
pub const SLATE: BlockId = 84;
```

Each has the same mining properties as STONE (drops itself via mine_drop, breakable with Stone+ pickaxe). Distribution = stone-replacement noise in the eventual biome-aware world-gen (Phase 5 of 28a). Until then, generators don't place them; players who `/give` themselves the block can build with it.

---

## §3 — Ore products

```rust
pub const COPPER_ORE: BlockId = 85;
```

`MaterialId::Copper` (raw drop), `Tin` (smelter-input), `Sulphur` (future gunpowder upgrade — recipe TBD), `Amethyst` (deepslate-only ore drop).

Copper Ore drops `Copper` (raw); 1 Copper + furnace → 1 `CopperIngot`. Bronze recipe = 1 CopperIngot + 1 TinIngot in furnace → 2 BronzeIngot.

---

## §4 — Smelt recipes

Add to the existing furnace recipe table:
- Copper (raw) → CopperIngot (1 → 1, 200 ticks)
- Tin (raw) → TinIngot (1 → 1, 200 ticks)
- CopperIngot + TinIngot → BronzeIngot (Bronze recipe; alloy)

Bronze tier is *not* a new tool tier — Bronze Ingot is a decorative material on alpha. 28e Tools/Armour decides whether Bronze becomes a real tier (currently the master spec says Wood/Stone/Iron/Diamond/Satori is the durable ladder; Bronze is cosmetic).

---

## §6 — Farming-adjacent materials

These add the *blocks* (e.g., Pumpkin block) and the *output materials* (e.g., Pumpkin material; Pumpkin Seeds material). They do **not** wire the planting + growth + harvest mechanics — those belong to the farming roadmap which the owner reserves.

Pumpkin block is mineable into a Pumpkin material (similar to Bread — edible after baking; raw is craft-input). Pumpkin Seeds drop alongside the Pumpkin (separate-seed pattern, mirrors Wheat).

---

## §8 — Tests

```rust
#[test]
fn all_material_ids_has_one_entry_per_variant() {
    // mem::variant_count is unstable; assert manually.
    assert_eq!(ALL_MATERIAL_IDS.len(), 41 + NEW_VARIANTS_COUNT);
}

#[test]
fn copper_ore_mines_to_raw_copper() {
    let registry = BlockRegistry::new();
    let drop = registry.mine_drop(COPPER_ORE);
    assert!(matches!(drop.item, Item::Material(MaterialId::Copper)));
}

#[test]
fn copper_ingot_recipe_resolves() { /* furnace recipe lookup hits */ }

#[test]
fn bronze_is_decorative_not_tool_material() {
    // BronzeIngot must NOT appear in any (ToolType, ToolMaterial) combo.
    for (_tt, mat) in all_tool_combos() {
        assert_ne!(format!("{:?}", mat), "Bronze");
    }
}
```

---

## Acceptance — sub-foundation 28c overall

- `./check.sh` ALL GREEN.
- All tests pass.
- Inventory Explorer (28f) automatically picks up new materials (verify: open explorer, search "copper" → see Copper Ore block + Copper material + Copper Ingot material).
- Player Guide updated.
- Foundations README updated.
- **Carry-over:** when 28d Mobs lands, that PR wires Honeycomb/Egg/Ink Sac drops_for entries. This spec just plants the MaterialId variants so the integration is one-line per drop.

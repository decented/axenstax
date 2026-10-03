# Historical Pivot — Sub-Foundation 1: Drop-Economy Migration

**Status:** **DELIVERED 2026-05-22** on `feat/historical-pivot-sub1-drop-migration`. **Prerequisite:** Sub 0 (Naming Pass) merged first so the naming convention was stable before this PR added a new MaterialId.
**Branch:** `feat/historical-pivot-sub1-drop-migration` off `main` (after Sub 0 merges).
**Trigger:** Sub-foundation **1** of the [Historical Pivot Long-Run](../vision/historical-pivot-long-run.md). Ensures recipe-critical drops (Bone, String) survive the eventual Sub 6 cutover that removes fantasy mob sources.

---

## TL;DR

Replace recipe-critical drops that the cutover (Sub 6) will retire. Add `MaterialId::BrigandChieftainTrophy` so Sub 3's Brigand-Hideout Chieftain has a drop slot reserved. Add a `Wool → 4 String` recipe so Bow + Fishing Rod survive the loss of Spider drops. Deprecate (but don't remove) the now-unobtainable MaterialIds whose sources retire. Lands second of the seven (after Sub 0 Naming Pass) because the rest of the historical-pivot sequence assumes recipe-stable inputs.

**No mob changes ship in this PR.** Bears/Hyenas/Knights/Brigands/Marauders/Berserkers and the spawn-pool flip are all later sub-foundations.

---

## Why this lives here

- **Atomic safety net.** Sub 6 is a single-PR cutover. Without this sub landing first, Sub 6 would silently break Bow + Fishing Rod crafting the moment it ships.
- **Wool → String is the only blocking substitution.** Bone keeps its Skeleton source through Subs 2-5 (Skeletons still spawn during the transition window); Sub 2 wires Bears + Wolves to drop Bone too. The intent is recorded here so Sub 2 knows what's expected.
- **`BrigandChieftainTrophy` MaterialId is reserved now** so Sub 3 (Brigand Hideouts) can land it as a drop without a separate "add the type first" follow-up.
- **Cross-game lift.** The wool→string recipe is engine-generic — any Decented game with sheep + bow gets it for free. The bandit-trophy primitive is engine-generic too.

---

## Material status matrix

| Material | Current source | Recipe uses | Plan |
|---|---|---|---|
| **Bone** | Skeleton drop | → Bonemeal (farming growth boost) | Source-add deferred to Sub 2 (Bear + Wolf + Hyena drop Bone). No code change in this PR — Skeleton still drops Bone until Sub 6. |
| **Bonemeal** | crafted from Bone | farming | Unchanged. |
| **String** | Spider drop | Bow (all 5 tiers), Fishing Rod | **New recipe**: 1 Wool in any single craft slot → 4 String (shapeless single-ingredient; implementer picks the canonical shape that matches `crafting::match_recipe`'s existing single-ingredient patterns). Sheep already drop wool. Spider source goes silent at Sub 6 cutover; wool path replaces it. |
| **Gunpowder** | Creeper drop | none | **DEPRECATE.** Variant stays declared (positional bincode). Item::name + color stay. Comment: `// DEPRECATED 2026-05-22: historical pivot retired fantasy source.` |
| **SpiderEye** | Spider drop | eat (poison 60 ticks) | **DEPRECATE.** Variant stays. Removed from `is_food_classification_is_canonical` allowlist (Item::food_value match arm stays but becomes unreachable). |
| **RottenFlesh** | Zombie drop | eat (poison) | **DEPRECATE.** Same posture as SpiderEye. |
| **Slimeball** | Slime drop | none | **DEPRECATE.** Same posture as Gunpowder. |
| **WitherSkull** | WitherSkeleton drop | trophy | **REPLACE.** Variant stays declared but unobtainable post-Sub 6. New `MaterialId::BrigandChieftainTrophy` takes the trophy role; *generation* lives in Sub 3 (one drop per cleared Brigand Hideout). |

Why the deprecated-but-not-removed treatment: positional bincode pins every `MaterialId` variant. Removing a variant would corrupt every existing save. The cost of carrying 5 dead match arms is trivial compared to a save-format break.

---

## Phasing

| # | Phase | Files | LOC |
|---|---|---|---|
| 1 | **This spec** | `docs/foundations/2026-05-22-historical-pivot-drop-migration.md` | – |
| 2 | New `MaterialId::BrigandChieftainTrophy` variant + `Item::name` + `Item::color` arms + `inventory_explorer::ALL_MATERIAL_IDS` append + count test bump (97 → 98) | `item.rs`, `inventory_explorer.rs` | ~30 |
| 3 | New crafting recipe: 1 Wool single-ingredient → 4 String. Add to `crafting::match_recipe` (match the shape convention used by existing single-ingredient recipes like Papyrus Reed → Sheet). Test that Bow (all 5 tiers) + Fishing Rod still resolve with wool-derived string. | `crafting.rs` | ~80 |
| 4 | Deprecation comments on `MaterialId::{Gunpowder, SpiderEye, RottenFlesh, Slimeball, WitherSkull}`. One-line each. | `item.rs` | ~10 |
| 5 | Remove `RottenFlesh` + `SpiderEye` from `canonical_foods` in `item.rs::is_food_classification_is_canonical` test. Their `food_value` match arms stay but become unreachable. | `item.rs` | ~5 |
| 6 | Update `docs/foundations/README.md` — add historical-pivot section + this sub as DELIVERED with PR link. Add `docs/vision/historical-pivot-long-run.md` to the vision-docs list at the top. | docs | ~20 |

**Total**: ~145 LOC including tests. Solo through Phase 6; no playtest gate (no gameplay-visible change — wool→string is additive, deprecations are silent).

---

## What this PR ships

1. New `BrigandChieftainTrophy` MaterialId reserved (no drop wiring yet).
2. Wool → 4 String recipe live + tested.
3. Five deprecation comments on retired-source MaterialIds.
4. `canonical_foods` test allowlist tightened (RottenFlesh, SpiderEye no longer counted).

## What this PR does NOT do

- Doesn't touch Spider/Zombie/Creeper/Slime/Skeleton/WitherSkeleton spawning. Fantasy mobs still spawn until Sub 6.
- Doesn't add Bears, Hyenas, Knights, Brigand Hideouts. Subs 2-4.
- Doesn't modify Spec 22 wave composition. Sub 5.
- Doesn't migrate existing saves. Sub 6.
- Doesn't remove `MaterialId` variants. Bincode is positional — variants stay forever.
- Doesn't add a Brigand Hideout procgen structure. Sub 3.

---

## Tests

```rust
#[test]
fn wool_resolves_to_four_string() {
    // Shape per the implementer's call — mirror the existing
    // single-ingredient pattern used by Papyrus Reed → Sheet.
    let wool = CraftSlot::Material(MaterialId::Wool);
    let grid = vec![vec![wool]];
    let out = match_recipe(&grid).unwrap();
    assert!(matches!(out.item, Item::Material(MaterialId::String)));
    assert_eq!(out.count, 4);
}

#[test]
fn iron_bow_recipe_resolves_with_wool_derived_string() {
    // Craft 4 String from 1 Wool, then craft Iron Bow as normal.
    // This is an integration test — exercises the migration path
    // end-to-end without any Spider drops.
}

#[test]
fn fishing_rod_recipe_resolves_with_wool_derived_string() {
    // Same pattern.
}

#[test]
fn bandit_leader_trophy_appears_in_inventory_explorer() {
    let registry = BlockRegistry::new();
    let entries = enumerate_all_items(&registry);
    assert!(entries.iter().any(|e| matches!(&e.item, Item::Material(MaterialId::BrigandChieftainTrophy))));
}

#[test]
fn rotten_flesh_no_longer_in_canonical_foods() {
    // Existing is_food_classification_is_canonical test must continue
    // to pass after RottenFlesh is removed from canonical_foods.
    // (Implicitly tested — the test itself enforces the canonical
    // list matches Item::is_food output; removing from canonical AND
    // not removing from food_value would fail this test.)
}
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- Bow + Fishing Rod recipes resolve from wool-derived string for every existing tier.
- `MaterialId::BrigandChieftainTrophy` is enumerated by the inventory explorer.
- `RottenFlesh` + `SpiderEye` removed from `canonical_foods`; the existing audit test continues to pass.
- Existing saves with RottenFlesh / SpiderEye / Slimeball / Gunpowder / WitherSkull in inventory load cleanly (no panic; items sit unused).

---

## Memory-rule check

- ✓ uk english naming — "Brigand Chieftain Trophy" (UK English title casing).
- ✓ bitcoin parent controlled — orthogonal.
- ✓ shared infra strategy — wool→string is engine-generic; lifts to every Decented game with sheep + ranged weapons.
- ✓ merge to main preauthorised — healthy-gate merge fine when `check.sh` is green.

---

## Out of scope (deferred to later subs)

- Brigand Hideout procgen, brigand `MobType` variants, hideout stockpile mechanic — Sub 3.
- Bone source from Bears + Wolves + Hyenas — Sub 2.
- Iron Golem → Knight reroute — Sub 4.
- Raid wave composition — Sub 5.
- Spawn pool flip + save scrub — Sub 6.

---

## Open questions

None for this sub. The wool→string recipe shape is unambiguous (mirrors Minecraft's wool-to-string break); the deprecation pattern is consistent with how the project already handles bincode-pinned legacy variants.

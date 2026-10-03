# Historical Pivot — Sub-Foundation 0: Naming Pass

**Status:** DELIVERED 2026-05-22 on `feat/historical-pivot-sub0-naming-pass`. **Prerequisite:** none (lands first of the seven historical-pivot sub-foundations).
**Branch:** `feat/historical-pivot-sub0-naming-pass` off `main`.
**Trigger:** Sub-foundation **0** of the [Historical Pivot Long-Run](../vision/historical-pivot-long-run.md). Applies the rename table in the vision doc to the codebase before any other sub builds on the (now-stable) display names.

---

## TL;DR

Apply the naming principle: **pick the name a medieval person would have used**. If Mojang happened to land on the same English word for the same thing, that's coincidence (we keep it). If Mojang chose something coined or quirky and the proper historical term differs, we rename.

Sub 0 is the pure mechanical pass that applies this principle to the current codebase. Mostly display-name updates (Item::name / BlockDef::name match arms); two deeper identifier renames (Peddler, Scribe); GlowBerry stripped (no historical anchor). No gameplay change.

Lands first so Subs 1-6 build on already-correct names.

---

## Why this lives here

- **Establishes the naming convention** before Sub 1 (drop migration) starts adding new MaterialIds. Without Sub 0 first, Sub 1's new `BrigandChieftainTrophy` would land alongside still-named `Bonemeal` / `HoneyBottle` / etc. — partial migration is confusing.
- **Mechanical work, low risk.** Pure search-and-replace on display strings plus two identifier renames. No save-format change (bincode is positional; the Rust identifier `Wool` vs renamed `Wool` doesn't affect on-disk format).
- **Cross-game lift:** the naming principle ("historical accuracy wins over Mojang-familiarity") is engine-generic. Sister games (other games on the same primitives) can adopt the same rule.

---

## What this PR ships

### Display-name updates (Item::name / BlockDef::name match arms only)

| Symbol | Old display | New display |
|---|---|---|
| `MaterialId::Bonemeal` | "Bonemeal" | **"Bone Meal"** |
| `MaterialId::HoneyBottle` | "Honey Bottle" | **"Honey Jar"** |
| `BlockId::HAY_BALE` | "Hay Bale" | **"Hay Rick"** |
| `BlockId::BONE_BLOCK` | "Bone Block" | **"Bone Cairn"** |
| `BlockId::AMETHYST_BLOCK` | "Amethyst Block" | **"Amethyst Cluster"** |
| `BlockId::SUGARCANE` | "Sugarcane" | **"Sugar Cane"** |
| `BlockId::VENDOR_BLOCK` | "Vendor Block" | **"Market Stall"** |
| `BlockId::PLAN_TILE` | "Plan Tile" | **"Plan Scroll"** |
| `BlockId::CONSTRUCTION_ANCHOR` | "Construction Anchor" | **"Foundation Stone"** |
| `BlockId::ARCHITECT_PLAQUE` | "Architect Plaque" | **"Mason's Mark"** |
| `BlockId::DRAFTING_TABLE` | "Drafting Table" | **"Drafting Bench"** |

### Identifier + display renames (Rust-level rename)

| Old | New | File-level changes |
|---|---|---|
| `MobType::WanderingVillager` + `data/mobs/wandering_villager.toml` | **`MobType::Peddler`** + `data/mobs/peddler.toml` | Rename TOML file; update all `MobType::WanderingVillager` references; update tests that reference the mob; the on-the-wire `EntityKind` discriminant **stays the same numeric value** (positional protocol). |
| `Profession::Librarian` | **`Profession::Scribe`** | Update all `Profession::Librarian` references; update villager gossip text that mentions "librarian" to use "scribe". |

### Deprecation

| Symbol | Action |
|---|---|
| `MaterialId::GlowBerry` | Mark with `// DEPRECATED 2026-05-22: historical pivot — no medieval analog (bioluminescent fruit isn't real). Variant stays declared per positional bincode rule.` No code change beyond the comment + removing the entry from `inventory_explorer::ALL_MATERIAL_IDS` (last in the table — single-line removal + count bump 97 → 96, OR keep it in the explorer table for save-compat visibility — see open question below). |

### Mojang-coincidence keeps (no change)

These names are correct historical English even if Mojang also uses them. Listed here for traceability so reviewers don't ask "why didn't you rename Furnace":

- `BlockId::FURNACE` / `FURNACE_LIT` — historical "furnace" is the right word for a heated smelting chamber.
- `BlockId::CRAFTING_TABLE` — slightly Mojang-coined; "Workbench" is marginally more medieval, but "Crafting Table" reads more obviously in-game. *Deferred to a possible future cosmetic pass.*
- `BlockId::BED` — generic.
- `BlockId::BEE_HIVE` — "hive" is millennia-old usage for a bee dwelling. (The medieval-specific "Skep" is deferred per open question 7 in the vision doc.)
- `BlockId::TORCH`, `VILLAGE_BELL`, `TILLED_SOIL`, `CAMPFIRE`, `MILL`, `OVEN`, `AGING_RACK`, `DRYING_RACK` — all historically correct.
- `MaterialId::{Stick, Leather, Feather, Wool, Bone, String, Coal, RawIron, Diamond, IronIngot, Arrow, Flint, Honeycomb, InkSac, Bread, Carrot, Potato, Wheat, ...}` — all generic or historically correct.
- `Profession::{Farmer, Blacksmith, Cook, Carpenter, Builder}` — all real medieval trades.
- `ToolType::{Pickaxe, Axe, Sword, Shovel, Hoe, Bow, FlintAndSteel, Shears, FishingRod}` — all historically correct medieval tools.

---

## Phasing

| # | Phase | Files | LOC |
|---|---|---|---|
| 1 | **This spec** | `docs/foundations/2026-05-22-historical-pivot-naming-pass.md` | – |
| 2 | Display-name match-arm updates (11 entries; pure string replacement in `Item::name` + `BlockRegistry::name`) | `item.rs`, `block.rs` | ~25 |
| 3 | `MobType::WanderingVillager → MobType::Peddler` Rust-level rename. Update all references across `mob.rs`, `wandering_villager.rs` (rename file → `peddler.rs`?), entity_model.rs, biome.rs, save.rs, tests. Rename `data/mobs/wandering_villager.toml → peddler.toml`. EntityKind on-the-wire stays unchanged (positional). | `mob.rs` and ~10 reference sites | ~60 |
| 4 | `Profession::Librarian → Profession::Scribe` Rust-level rename. Update villager dialogue strings to say "scribe" not "librarian". Update profession-claim arm comments. | `villager.rs` and ~5 reference sites | ~30 |
| 5 | `MaterialId::GlowBerry` deprecation comment + remove from `inventory_explorer::ALL_MATERIAL_IDS` if approved (see open question below). Count test bump if removed. | `item.rs`, `inventory_explorer.rs` | ~10 |
| 6 | Test updates — any test that asserts on the old display name string ("Bone Block" → "Bone Cairn" etc.) gets its expected-string updated. | various `tests` modules | ~30 |
| 7 | Docs — `docs/foundations/README.md` updated to add HP-0 row (DELIVERED) + the vision doc reference. Player guide pages that mention the renamed items get refreshed (Mason's Mark page, Market Stall page, etc.). | docs | ~50 |

**Total**: ~205 LOC including test updates. Solo through Phase 7; no playtest gate (display-name changes are visible but non-breaking; user-perception change is a Sub 6 cutover concern, not a Sub 0 concern).

---

## What this PR does NOT do

- Doesn't touch any mob behaviour, AI, or spawn rule.
- Doesn't add new MaterialIds (HP-1's job).
- Doesn't add new mobs (HP-2, HP-3, HP-4's jobs).
- Doesn't change recipe shapes or yields.
- Doesn't remove `MaterialId` or `MobType` variants (bincode positional — variants stay forever).
- Doesn't change `EntityKind` discriminants on the wire (positional protocol).
- Doesn't rename `raid.rs` module to `incursion.rs` (the module identifier stays for code-history continuity; only user-facing display strings around raid mechanics get the "Incursion" treatment when Sub 5 ships).
- Doesn't apply the optional Skep / Workbench renames (deferred per open questions 7-8 in the vision doc).
- Doesn't rename `BlockId::CRAFTING_TABLE` (Workbench rename deferred).
- Doesn't touch Nostrich, Satori, or any other Original-AxeNStax IP names.

---

## Tests

```rust
#[test]
fn renamed_display_strings() {
    let registry = BlockRegistry::new();
    assert_eq!(Item::Material(MaterialId::Bonemeal).name(&registry), "Bone Meal");
    assert_eq!(Item::Material(MaterialId::HoneyBottle).name(&registry), "Honey Jar");
    assert_eq!(Item::Block(block::HAY_BALE).name(&registry), "Hay Rick");
    assert_eq!(Item::Block(block::BONE_BLOCK).name(&registry), "Bone Cairn");
    assert_eq!(Item::Block(block::AMETHYST_BLOCK).name(&registry), "Amethyst Cluster");
    assert_eq!(Item::Block(block::SUGARCANE).name(&registry), "Sugar Cane");
    assert_eq!(Item::Block(block::VENDOR_BLOCK).name(&registry), "Market Stall");
    assert_eq!(Item::Block(block::PLAN_TILE).name(&registry), "Plan Scroll");
    assert_eq!(Item::Block(block::CONSTRUCTION_ANCHOR).name(&registry), "Foundation Stone");
    assert_eq!(Item::Block(block::ARCHITECT_PLAQUE).name(&registry), "Mason's Mark");
    assert_eq!(Item::Block(block::DRAFTING_TABLE).name(&registry), "Drafting Bench");
}

#[test]
fn peddler_mob_loads_from_renamed_toml() {
    // data/mobs/peddler.toml exists; mob_def(MobType::Peddler) resolves.
    let def = mob_def(MobType::Peddler);
    assert!(!def.name.is_empty());
}

#[test]
fn scribe_profession_claims_lectern() {
    // Profession::Scribe replaces ::Librarian wherever it was the
    // workstation-claim profession in villager dispatch.
    // (Concrete assertion depends on what block Librarian claimed —
    // if anything; check existing villager.rs to wire the test.)
}

#[test]
fn glowberry_is_marked_deprecated() {
    // Compile-time deprecation? Or just absent from ALL_MATERIAL_IDS?
    // Whichever shape Phase 5 implements.
}
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- All 11 renamed display strings load correctly (covered by `renamed_display_strings` test).
- `MobType::Peddler` (was WanderingVillager) resolves from its TOML and spawns in the same biomes as before.
- `Profession::Scribe` (was Librarian) claims the same workstation blocks Librarian did.
- GlowBerry is deprecated (comment present; explorer table reflects the decision per Open Question 1).
- Existing saves load cleanly — on-wire EntityKind discriminants unchanged; on-disk MaterialId/MobType numeric variants unchanged (bincode positional).

---

## Open questions

1. **GlowBerry — strip from `inventory_explorer::ALL_MATERIAL_IDS` or leave in for save-compat visibility?** Stripping is consistent with how the other source-retired materials get handled in Sub 1 (they stay in the table because they're not stripped yet — Sub 1 deprecates without removing from explorer). For consistency with Sub 1's pattern: **leave it in the explorer table; just add the deprecation comment.** Implementer's call.
2. **Should `wandering_villager.rs` be renamed to `peddler.rs`?** If the file exists. If it's just a couple of references in `mob.rs` and no dedicated module, no rename needed. Implementer audits and decides.

---

## Memory-rule check

- ✓ uk english naming — UK English throughout. "Honey Jar" / "Hay Rick" / "Bone Cairn" / "Sugar Cane" (two-words) are all UK-English-compatible.
- ✓ bitcoin parent controlled — orthogonal; no sats flow changes.
- ✓ shared infra strategy — the naming principle ("historical accuracy wins") is engine-generic and lifts to sister games.
- ✓ merge to main preauthorised — healthy-gate merge fine when `check.sh` is green.

---

## Out of scope (deferred to later subs or future passes)

- New mobs / animals / structures / professions — Subs 2-7.
- Spawn pool / save scrub — Sub 6.
- Optional Skep / Workbench renames — vision doc open questions 7-8; future cosmetic pass.
- Cooper / Tanner / Fletcher / Mason professions — future Sub 8+ if needed.
- Rename of `raid.rs` module identifier — stays for code-history continuity.
- Texture changes — display strings only; no art revision in this PR.

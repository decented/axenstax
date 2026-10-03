# Minecraft-Parity Content Surface — Spec 28

**Status:** PARTIALLY DELIVERED as of 2026-05-21. 28a (Biomes) + 28b (Woods) + 28c (Materials, partial) + 28e (Tools surface — armour + Shears + Fishing Rod) + 28f (Inventory explorer) + 28d.wolves shipped on main. Remaining 28d species (Horse / Goat / Rabbit / Bee / Squid / Skeleton archer / Wither Skeleton) pending — owner is working through them per chunk plan.
**Branch:** `feat/spec-28-minecraft-parity` off `main`. Will fork into sub-branches per phase.
**Trigger:** "If a typical Minecrafter jumps on, they should get pretty much everything they need for a reasonable experience in Axe'n'Stax." The engine foundation is well-tested; this spec is the **content fill-out** that brings the surface from "alpha curiosity" to "feels like a real game". Drafted 2026-05-20.

---

## TL;DR

Six tightly-related content lanes ship together so the world feels alive on first contact:

1. **Materials roster** — `MaterialId` from 44 → ~115 entries. Fills the raw-drop + crafted-intermediate gap without touching Brewing / Redstone / Nether / End (per `whats-coming.md` exclusions).
2. **Biome expansion** — 5 → 12 biomes. Adds Taiga, Birch Forest, Jungle, Savanna, Swamp, Beach, Snowy Tundra; softens biome boundaries so adjacent biomes blend rather than slam-cutting.
3. **Wood species** — single `OAK_LOG` family → eight-species `WoodSpecies` enum. Per-species log + planks + leaves + sapling + tree-gen shape, biome-driven distribution. Every wood-consuming recipe accepts any species.
4. **Mob roster** — current 10 mob types → 21. Adds Cat, Horse + Donkey + Mule (with breeding flag for T2 farming), Fox, Rabbit, Bee, Squid + Glow Squid, Witch, Phantom. Wolves are explicitly revisited in §4 design discussion.
5. **Tool surface** — fills armour (Helmet / Chestplate / Leggings / Boots × Leather / Iron / Diamond / Satori), Shield, Fishing Rod, Shears, Trident, Saddle.
6. **Inventory explorer with search** — new full-screen pane (separate from hotbar + crafting) showing every item + block + tool in the engine, filterable by text + category. Drag-to-hotbar in creative; read-only "you have N" in survival.

**Scope:** ~12,000 LOC + ~200 KB of textures across 28 phases, organised into 6 sub-foundations (one per lane). Build order: 2 → 3 → 1 → 4 → 5 → 6. Each sub-foundation can ship and playtest independently.

**Cuts (per `whats-coming.md` §"Things that AREN'T in the game"):** Brewing/potions, Redstone, Nether/End dimensions and their mobs/blocks/items, VR, marriage, villager-to-villager trade. Wolves get a one-paragraph design pass in §4 but are NOT in v1 scope.

---

## Why this lives here

- **Content gravity.** The engine has shipped Furnace, Vendor Block, Plaque tipping, four-tier deepslate, Build Schematics, Villages, Charter. The economy primitives are wired. What's missing is **variety**: too few mobs, one wood species, sparse biomes, no armour. A Minecrafter today would feel the gameplay loops but bounce off the texture pool.
- **Lift-not-stretch architecture.** The big architectural change is **wood species** (a real `WoodSpecies` enum that parameterises four block-defs + tree-gen). Materials + mobs + tools are surface adds with no new systems. Biomes need a softened-boundary pass but no new biome architecture.
- **Cross-game lift per shared infra strategy.** Wood species enum, biome blend solver, mob component shape, armour-slot UI are all engine-generic. The specific roster (mobs, woods) is AxeNStax-flavoured; the patterns lift.

---

## Self-review checklist (run before execution)

- ✅ **Dependency order makes sense.** Biomes (Phase 2) precede Woods (Phase 3) because tree distribution needs biome assignments. Woods precede Materials (Phase 1 expansion) because the new MaterialIds include species-specific items (e.g., per-species saplings). Materials precede Tools (Phase 5) because armour needs Leather/Iron/etc. — all already in MaterialId. Mobs (Phase 4) can land in parallel with anything from 1.
- ✅ **LOC estimates are honest.** Drawn from existing analogous work — Spec 19 Villages was ~2,250 LOC for 3 mob types; 11 new mob types here ≈ ~3,000 LOC. Wood species refactor ≈ same shape as Spec 17 Campfire's two-variant pattern but with 8 variants and 4 block-defs per variant.
- ✅ **No conflict with existing specs.** Doesn't touch Spec 1 multiplayer auth, Spec 2 HostedServer, Spec 12 T1.5 (owner-reserved), Spec 22 Raid Defence. Light coupling with Spec 19 (Bee profession line drafted), Spec 20 (Furnace), Spec 23 (PapyrusReed already in MaterialId).
- ✅ **Cuts are explicit.** Brewing, Redstone, Nether/End all named in §"What's NOT in scope" below. Wolves discussed and deferred with rationale.
- ✅ **Playtest gates per sub-foundation, not one mega-gate.** Each sub-foundation (28a-f) has its own playtest phase. The user can run them at their own pace without one giant playtest session.

---

## Sub-foundations

Spec 28 will fork into six sub-foundation docs in `docs/foundations/`, each executable independently:

- **28a — Biome expansion** (`2026-05-21-biome-expansion.md`). Ships first because Woods + Mobs depend on biome assignments.
- **28b — Wood species** (`2026-05-21-wood-species-expansion.md`). Ships after biomes. Refactor-heavy.
- **28c — Materials roster fill-out** (`2026-05-21-materials-roster-fill.md`). Surface adds; parallelisable with 28d and 28e after 28a + 28b.
- **28d — Mob roster expansion** (`2026-05-21-mob-roster-expansion.md`). Surface adds; parallelisable with 28c, 28e.
- **28e — Tool surface (armour + utility)** (`2026-05-21-tool-surface-armour.md`). Parallelisable with 28c, 28d.
- **28f — Inventory explorer with search** (`2026-05-21-inventory-explorer-search.md`). Ships last so it reflects the full content surface.

This spec doc (28) is the **integration brief** + design discussion. Sub-foundations are the executable plans.

---

## §1 — Materials roster fill-out

Current `MaterialId` has 44 variants. Target ~115. The gap is **raw drops + crafted intermediates that a Minecrafter expects but Axe'n'Stax doesn't have yet**.

### New raw drops (~30)

| Group | New entries | Source | Notes |
|---|---|---|---|
| **Tree drops** | OakSapling, BirchSapling, SpruceSapling, AcaciaSapling, JungleSapling, DarkOakSapling, CherrySapling, MangroveSapling | Mining any leaves block has 5% chance | Replant via right-click on dirt/grass. Shape per species in §3. |
| **Mob drops** | RawRabbit, RabbitFoot, RabbitHide, RawCod, RawSalmon, Tropical-Fish, Pufferfish, HoneyBottle, Honeycomb, GlowInkSac, InkSac, BeeStinger | New mobs in §4 | Cooking targets in furnace are noted in §1 below. |
| **Plant drops** | Apple, MelonSlice, SweetBerries, GlowBerries, BambooStick, CocoaBeans, Mushroom (Red/Brown), Kelp | New biome plants (Jungle, Forest, Swamp) | Apple at 1% from oak leaves on break. |
| **Mineral drops** | EmeraldCrystal, Quartz, Amethyst, Lapis | New ore variants in Jungle/Mountains | Plus the existing Diamond / Coal / RawIron. |

### New crafted intermediates (~25)

| Group | New entries | Recipe shape | Notes |
|---|---|---|---|
| **Cooking** | CookedRabbit, CookedCod, CookedSalmon | Furnace 10s | Standard cooking; food values per Spec 5 §6.2. |
| **Sugar / dairy / baked** | Sugar, Egg, Milk-via-Bucket, Cake, Cookie, PumpkinPie | Spec 12 T1.5 owns most of these | Document the dependency. Sugar from Sugarcane (already raw). |
| **Wood-derived** | Charcoal, Stick (already in), Bamboo-Stick variant | Charcoal = log + furnace (smelt) | Burns same as Coal in fuel ladder. |
| **String + cloth** | Wool variants (currently single WhiteWool) | Sheep dyeing post-dye spec | Out of v1 — skipped per Skip list. |
| **Misc** | Bread (in), Bone, Leather (in), Saddle-leather, BambooPlank-equivalent | — | |

### Materials NOT in v1 (skipped per `whats-coming.md`)

- All brewing ingredients (Blaze Powder, Glistering Melon, Spider Eye gold variants, Nether Wart, etc.)
- Redstone components (Dust, Repeater, Comparator, Piston parts)
- Dyes (16-colour explosion — defer to a dedicated dye spec)
- Nether/End items (Blaze Rod, Ender Pearl, Eye of Ender, Shulker Shell, etc.)
- Music discs, Maps, Banner Patterns, Books-and-Quill

### Implementation per phase (sub-foundation 28c, ~8 phases, ~1,500 LOC)

1. `MaterialId` extension — append ~70 new variants preserving bincode indices (additive only). Item display names + colours.
2. Mob-drop wiring — link new mob types (§4) to their drops via `mob::loot_for`.
3. Plant-drop wiring — leaves drop saplings + apples; bamboo + sugarcane + cocoa drop their materials.
4. Cooking arms — `furnace::smelt_recipe_for_input` adds the new raw → cooked pairs.
5. `/give` aliases for every new material so Axolittle can spawn-test.
6. Texture pack — ~70 new procedural item-icon textures (procedural so we don't ship binary assets).
7. Tests — `MaterialId` count assertion, recipe lookup tests per new arm, leather-drop probability test.
8. Axolittle playtest — does each new material spawn / drop / get used as expected.

---

## §2 — Biome expansion

Current biome enum: Plains, Forest, Desert, Ocean, Mountains. Target 12.

### New biomes (~7)

| Biome | Temperature × humidity bias | Surface | Trees | Mobs |
|---|---|---|---|---|
| **Taiga** | Cold + moderate | Snow / dirt | Spruce | Wolf (deferred), Rabbit |
| **Birch Forest** | Temperate + moderate | Grass | Birch (primary) + Oak (10%) | Standard forest mobs |
| **Jungle** | Hot + wet | Grass | Jungle (primary) + Cocoa-bearing | Fox, Ocelot/Cat |
| **Savanna** | Hot + dry-ish | Grass (yellow tint) | Acacia | Horse |
| **Swamp** | Temperate + very wet | Grass (dark) | Oak + Mangrove | Witch, Slime |
| **Beach** | Coast — any temperature | Sand | None | Crab (deferred) |
| **Snowy Tundra** | Very cold | Snow / ice | Spruce (sparse) | Polar Bear (deferred) |

### Softened biome boundaries

Today `biome_at()` returns a single discrete biome per (x, z). For the Forest variants (Oak Forest, Birch Forest, Jungle, Dark Oak Forest), a per-position **secondary** sampling pass picks tree species inside a biome zone so adjacent biomes blend rather than slam-cutting:

- Inside Forest, 60% Oak / 30% Birch / 10% Cherry (a "mixed deciduous" feel).
- Inside Birch Forest, 80% Birch / 20% Oak.
- Inside Jungle, 90% Jungle / 5% Mangrove / 5% Cocoa-bearing Jungle.

The per-position species pick uses a new helper `biome::tree_species_at(x, z, biome, seed) -> WoodSpecies` keyed on a 24-block-scale noise so each cluster has a coherent species but adjacent clusters differ.

### Biome-to-mob mapping

`spawning::tick_mob_spawning` already runs per-chunk. Add a `biome_mob_pool(biome) -> &[MobType]` lookup so spawns respect biome (Fox in Jungle, Horse in Savanna, etc.). Hostile mobs (Zombie, Skeleton, Spider, Creeper, Witch, Phantom) spawn in any biome at night per existing rules.

### Implementation per phase (sub-foundation 28a, ~6 phases, ~1,800 LOC)

1. `Biome` enum gets 7 new variants. `biome_at()` temperature × humidity thresholds widened to map onto the new biomes.
2. Surface block selection per biome (`biome_block_at` in `world.rs`).
3. `tree_species_at(x, z, biome, seed) -> WoodSpecies` blender function (depends on §3 enum existing — sub-foundation 28a ships AFTER 28b's enum is in, even though the biome phase is earlier conceptually).
4. `biome_mob_pool(biome) -> &[MobType]` + spawning integration.
5. Tests — biome distribution at known seeds, blend function determinism, surface-block per-biome.
6. Axolittle playtest — explore all 12 biomes, confirm visual + mob distinction.

---

## §3 — Wood species expansion

Current `block.rs` has one wood: `OAK_LOG`, `OAK_PLANKS`, `OAK_LEAVES`. The drying-rack + papyrus + all crafting recipes assume oak. This phase **parameterises** wood species across 8 variants.

### `WoodSpecies` enum

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WoodSpecies {
    Oak,         // existing — back-compat through bincode default
    Birch,
    Spruce,
    Acacia,
    Jungle,
    DarkOak,
    Cherry,
    Mangrove,
}
```

### Block layout per species

Each species gets **4 new block ids**: log, planks, leaves, sapling. Total new block ids = 8 species × 4 blocks = 32 (block ids 66-97).

Existing `OAK_LOG` (id 7), `OAK_PLANKS` (id 9), `OAK_LEAVES` (id 8) stay as the canonical IDs for Oak. The new IDs are for the 7 non-Oak species.

### Crafting recipes generalised

Today the recipes hard-code `OAK_LOG` / `OAK_PLANKS`. Refactor to accept any `WoodSpecies` via a `is_log(BlockId)` / `is_planks(BlockId)` predicate (mirror of `is_paperish_slot` from Spec 23):

- `is_logish(BlockId)` already exists in `crafting.rs` (Wave 29 generalised log → 4 planks). Extend to all 8 species.
- New `is_planks(BlockId)` predicate. Every recipe currently using `Block(OAK_PLANKS)` migrates to a `is_planks` match-guard.
- Tools made with any species' planks yield Wood-tier with the species's colour (visual variety, gameplay-identical).

### Tree-gen shapes per species

`world.rs::place_trees` currently has one shape. Expand to 8 shapes:

| Species | Shape | Height | Canopy |
|---|---|---|---|
| Oak | (existing) | 5-7 | spherical |
| Birch | tall + narrow | 7-9 | small spherical |
| Spruce | conical | 7-12 | layered cone |
| Acacia | branching Y | 5-7 | flat top, twin canopies |
| Jungle | tall + canopy | 10-16 | broad flat top |
| DarkOak | 2×2 trunk | 6-8 | wide dark canopy |
| Cherry | medium + pink leaves | 6-8 | spherical |
| Mangrove | propped + roots | 7-10 | dense canopy |

### Implementation per phase (sub-foundation 28b, ~10 phases, ~2,500 LOC)

1. `WoodSpecies` enum + 32 new block ids + BlockDefs + 32 new procedural textures (4 per species — log side / log top / planks / leaves).
2. `is_logish` + new `is_planks` + new `is_leaves` predicates; existing recipe arms migrated.
3. Drying Rack accepts any species; `GreenLog`/`SeasonedLog`/`KilnDriedLog` materials parameterised by species (added to MaterialId in §1).
4. Tree-gen shapes per species (`world.rs::place_trees`).
5. Per-biome species distribution wired via `biome::tree_species_at` (depends on §2).
6. Sapling drops from leaves on break (5% chance, per species).
7. Sapling planting — right-click sapling on dirt/grass plants it; grows into the matching tree over ~3 in-game days.
8. Mine_drop normalisation — mining any species log drops the appropriate species log; same for planks.
9. Tests — recipe-acceptance across species, tree-gen shape per species, sapling-grows-to-correct-species.
10. Axolittle playtest — chop one of each species, plant saplings, confirm recipe works with mixed species.

---

## §4 — Mob roster expansion

Current `MobType`: Cow, Pig, Chicken, Sheep, Zombie, Skeleton, Spider, Creeper, Slime, Villager, IronGolem, WanderingVillager. Target: add 11 more (21 total).

### New mobs

| Mob | Type | Biome | Drops | Notes |
|---|---|---|---|---|
| **Rabbit** | Passive | Plains / Snowy Tundra / Taiga | RawRabbit, RabbitFoot (5%), RabbitHide | Small, fast, hops |
| **Fox** | Neutral | Taiga / Jungle | (none — predator) | Hunts Rabbit + Chicken |
| **Cat** | Tameable | Jungle / Village | (none) | Scares Phantom (alpha-deferred); tame with raw fish |
| **Horse** | Tameable | Plains / Savanna | Leather (rare) | Saddle to ride; breeding for T2 farming |
| **Donkey** | Tameable | Plains / Savanna | Leather (rare) | Slower than horse; carries 1 chest's worth |
| **Mule** | Bred-only | (from Horse + Donkey breeding) | — | Per axenstax has farming — bred-only mule line |
| **Bee** | Neutral | Forest / Plains / Sunflower (future) | Honeycomb (on hive break) | Pollinates crops (T1.5 hook); stings on attack |
| **Squid** | Passive | Ocean / River | InkSac | Underwater swimmer |
| **GlowSquid** | Passive | Deep Ocean / Caves | GlowInkSac | Bioluminescent |
| **Witch** | Hostile | Swamp | Stick, Glowstone Dust (alpha-deferred to brewing) | Throws splash potions (alpha-deferred — alpha does melee only) |
| **Phantom** | Hostile (night) | Open Sky | Phantom Membrane (alpha-deferred — drops nothing on alpha) | Spawns over players who haven't slept in 3 days |

### Wolves — design discussion + decision

`whats-coming.md` lists Wolf / pet companions as "not specced". This spec is the design-pass to revisit.

**For** adding wolves:
- Tameable companions are a core Minecraft beat.
- Combat AI partner is a meaningful gameplay primitive.
- Spec 19 Iron Golem proves the friendly-mob-fights-hostile pattern is already supported.

**Against**:
- Adds tame/follow/sit AI states — ~400 LOC of new mob_ai.rs surface.
- Charter/Bitcoin consideration: would a tame wolf earn its master's PoP trickle? Probably yes (it's "near the player"), but the consensus rule is fiddly.
- The simpler primitive — Cat/Horse — already covers the tame-and-bond emotional beat without combat AI.

**Decision (REVISED 2026-05-20)**: wolves lifted from v2-deferral. Spec'd at [28d.wolves](2026-05-20-wolves-tameable-companion.md) as the canonical first pet AND the consumer that justifies extracting the `tameable.rs` framework that Cat / Parrot / future companions then build on. Charter/PoP-trickle consideration deferred — alpha tamed wolves do not earn PoP for their owner; revisit if Axolittle playtest surfaces it as missing.

### Implementation per phase (sub-foundation 28d, ~10 phases, ~3,000 LOC)

1. `MobType` extension — 11 new variants.
2. Per-mob model + textures (procedural; 11 mobs × ~4 textures each = ~44 new texture layers).
3. Mob AI states — Hop (Rabbit), Hunt (Fox), Tame (Cat/Horse/Donkey), Pollinate (Bee), Swim (Squid/GlowSquid), Cast (Witch — alpha melee), Dive (Phantom).
4. Biome-based spawn pool (`biome_mob_pool` from §2 — depends on §2).
5. Drops table per new mob; furnace recipes for RawRabbit / RawCod / RawSalmon (depends on §1).
6. Tameable interaction — right-click with the right food item: Cat with raw fish, Horse with apple, Donkey with apple. Tamed mobs follow + sit.
7. Saddle + ride — Horse/Donkey/Mule. Camera moves to mount-eye position; movement controls the mount.
8. Breeding — two tame mobs of the same species near each other with feed item produce a baby. Horse + Donkey → Mule (sterile).
9. Bee → hive → Honeycomb economy.
10. Axolittle playtest — find each new mob in the right biome, tame one of each tameable, breed the Horse/Donkey to get a Mule.

---

## §5 — Tool surface (armour + utility)

Current tools: Pickaxe / Axe / Shovel / Sword / Hoe × {Wood, Stone, Iron, Diamond, Satori} + Bow (Wood-only) + FlintAndSteel (Iron-only).

Missing: **Armour**, **Shield**, **Fishing Rod**, **Shears**, **Trident**, **Saddle**.

### Armour

| Slot | Recipe shape | Tiers |
|---|---|---|
| **Helmet** | M M M / M . M / . . . | Leather, Iron, Diamond, Satori |
| **Chestplate** | M . M / M M M / M M M | Leather, Iron, Diamond, Satori |
| **Leggings** | M M M / M . M / M . M | Leather, Iron, Diamond, Satori |
| **Boots** | . . . / M . M / M . M | Leather, Iron, Diamond, Satori |

Where M = material head (Leather for Leather tier, IronIngot, Diamond, Satori). **No Gold tier** per existing engine choice.

Damage reduction per tier (Minecraft baseline scaled):
- Leather: 4% reduction per piece (16% max full set)
- Iron: 6% / 24%
- Diamond: 8% / 32%
- Satori: 10% / 40% (top tier)

`PlayerCombat` already has `health: f32`; add `armour: [Option<Tool>; 4]` (helmet/chest/legs/boots slots) and `compute_armour_reduction()`.

### Utility tools

| Tool | Recipe | Tier | Use |
|---|---|---|---|
| **Shield** | P P / P I (2×2) | Wood-only | Right-click held to block 50% incoming damage. Durability 336. |
| **Fishing Rod** | . . S / . S T / S . T | Wood-only | Right-click into water → Raw Cod/Salmon/Pufferfish (probabilistic). |
| **Shears** | . I / I . (mirror) | Iron-only | Right-click sheep → Wool drops + sheep stays. Right-click leaves → drop leaves block. |
| **Trident** | M . M / . M . / . M . | Diamond-only | Melee + throwable. Damage 9. |
| **Saddle** | L L L / L . L (drop-only on Mule-bred or quest reward) | Leather-only craft (also drop) | Mounts a tamed Horse/Donkey/Mule. |

(P = Plank, I = Iron Ingot, S = String, T = Stick, M = matching tier head, L = Leather.)

### Implementation per phase (sub-foundation 28e, ~7 phases, ~1,800 LOC)

1. `ToolType` extension — 8 new variants (Helmet, Chestplate, Leggings, Boots, Shield, FishingRod, Shears, Trident, Saddle).
2. Recipe arms per shape; tier-aware via existing `material_from_slot`.
3. Armour-slot inventory UI — 4 new slots above the standard inventory grid.
4. Damage reduction wired into `PlayerCombat::take_damage`.
5. Shield block mechanic — held-down right-click reduces damage.
6. Fishing rod cast + Cod/Salmon/Pufferfish drop table.
7. Axolittle playtest — craft full Leather → Iron → Diamond → Satori armour set; fish; shear sheep; trident a Phantom.

---

## §6 — Inventory explorer with search

A full-screen UI pane separate from the hotbar + crafting grid. Shows every item / block / tool in the engine, filterable by text + category.

### Design

Open via `B` key (currently unbound — confirm no collision) OR via a new HUD button. Layout:

```
+--------------------------------------------------+
| INVENTORY EXPLORER                          [X]  |
+--------------------------------------------------+
| Search: [_______________________]                |
| [ All ] [ Blocks ] [ Tools ] [ Materials ]       |
| [ Mobs ] [ Plans ]                               |
+--------------------------------------------------+
| (grid of item icons, name on hover)              |
|                                                  |
| Each item:                                       |
|   - Icon                                         |
|   - Name on hover                                |
|   - Tooltip: type, source, food value (if any),  |
|     damage (if tool), recipe (if craftable)      |
|                                                  |
| Creative mode: drag icon to hotbar to spawn.     |
| Survival mode: shows "You have: N" if you own.   |
+--------------------------------------------------+
```

Per Spec 5 §3.6 rendering patterns (Background-order overlay, pinned-width Middle-order panel, layered painter).

### Implementation per phase (sub-foundation 28f, ~6 phases, ~1,400 LOC)

1. New `inventory_explorer.rs` module with `ExplorerState` + `draw_inventory_explorer`.
2. Item registry — pure function `enumerate_all_items() -> Vec<ExplorerEntry>` that walks every BlockId + every MaterialId + every (ToolType, ToolMaterial) combo. Tagged by category.
3. Search filter — substring match on item name, case-insensitive.
4. Category filter — radio-button row.
5. Drag-to-hotbar in creative — drag icon from explorer to a hotbar slot triggers `/give`.
6. "You have N" overlay in survival — query inventory for each entry's count.
7. Axolittle playtest — open explorer, search "iron", confirm all iron-related items appear (iron ore, iron ingot, iron pickaxe, iron block, …).

---

## What's NOT in scope (explicit cuts)

These exclusions match `docs/player-guide/whats-coming.md` §"Things that AREN'T in the game (and may never be)":

- **Brewing / potions / enchantments.** Not specced. Half of Minecraft's material count.
- **Redstone primitives.** Not specced.
- **Nether / End dimensions.** Not specced. Their mobs, blocks, items, ingredients all out.
- **Wolves / pet combat companions.** Discussed in §4; deferred to a v2 sub-foundation post-Cat/Horse playtest.
- **Marriage / family trees.** Out of scope.
- **VR mode.** Not committed.
- **Villager-to-villager trade.** Deliberately skipped per Spec 19 — trade is player-to-player via Vendor Block.
- **Dyes / 16-colour-variant explosion.** Out of v1 — defer to a dedicated dye spec post-content surface.
- **Maps / banner patterns / music discs / books-and-quill.** Out of v1.

---

## Memory pointers

- uk english naming — UK English in user-facing copy. "Cobblestone", "Pickaxe", "Armour" (not "Armor"), "Colour" (not "Color"), "Defence" (not "Defense").
- alpha launch posture — alpha posture; this content fill-out is the right priority once the engine foundation is stable.
- shared infra strategy — WoodSpecies + biome blend + armour-slot UI patterns lift cross-game.
- axenstax has farming — Mule-via-breeding fits the T2 farming roadmap.
- autonomy to playtest boundary — each sub-foundation has its own playtest gate.

---

## Acceptance — Spec 28 overall

- All six sub-foundation docs written + status flipped to READY TO BUILD in `docs/foundations/README.md`.
- Build order documented: 28a → 28b → (28c parallel with 28d parallel with 28e) → 28f.
- Each sub-foundation includes its own LOC estimate, phase list, file references, acceptance criteria, playtest gate.

When each sub-foundation phase ships, flip the queue row and update the Player Guide. The Player Guide section that absorbs each lane is documented in the sub-foundation.

---

## Build sequencing summary

```
28a Biomes      ─┐
                 ├─► 28b Woods ─┐
                 │              ├─► 28c Materials ─┐
                 │              ├─► 28d Mobs       ├─► 28f Inventory Explorer
                 │              └─► 28e Tools      │
                 │                                  │
                 └──────────────────────────────────┘
```

The Inventory Explorer (28f) ships last because it inventories everything — the more content is in the engine when 28f writes its `enumerate_all_items` registry, the more complete the explorer is on day one.

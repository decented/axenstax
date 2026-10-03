# Biomes + Mixed Forests — Spec 28a

**Status:** DELIVERED. `Biome` enum + `BiomeProperties` + Whittaker classifier + biome blending live in `game/engine/src/biome.rs` (2026-05-21). **World-gen wire-up landed 2026-05-27** (commit on `feat/world-spawning-biomes-mobs`): `BiomeGenerator::biome_at` now routes the mid-elevation band through `classify_whittaker` and decides Ocean/Mountains from a new `continentalness` field, so all 10 biomes generate. SnowyTundra gained a snow surface in `biome_block_at`. Verified by `all_ten_biomes_generate_from_biome_at`. Remaining = playtest-gated tuning (continentalness thresholds, biome balance) + per-biome sky/fog tint (post-alpha).

**Branch (when building):** `feat/spec-28a-biomes` off `main`.
**Trigger:** Sub-foundation of [Spec 28 Minecraft-parity content surface](2026-05-20-minecraft-parity-content-surface.md) §1. 28a precedes 28b (Woods) because tree placement reads the biome assignment.

---

## TL;DR

Add a `Biome` enum + a biome-assignment function that runs alongside the existing world-gen pipeline. Each biome controls (a) surface block, (b) sub-surface block, (c) tree species + density, (d) mob spawn weights, (e) sky/fog tint hooks (post-alpha). Biome transitions blend across ~16-block bands so the world doesn't look pixellated. Mixed forests fall out of biome blending: when two forest biomes touch, the boundary zone draws trees from both species pools.

**Eight launch biomes:** Plains, Forest, Birch Forest, Taiga, Jungle, Savanna, Desert, Snowy Tundra. (No Nether / End / Mushroom Island / Mesa — those are out of scope per the master spec.) Each is single-purpose: one dominant feel, not 30 micro-variants.

**Scope:** ~1,800 LOC + ~12 textures across 10 phases. Phase 10 is the Axolittle playtest gate.

---

## Why this lives here

- Axolittle's feedback (2026-05-20 conversation): a typical Minecrafter expects more than one ecosystem. The current world is "Plains, every block". This is the single biggest tell that AxeNStax feels less complete than Minecraft, even with everything else in place.
- Biomes precede woods (28b) because tree species selection is a *consequence* of biome, not an independent system. Building woods first then back-fitting biomes would mean rewriting tree placement twice.
- Mixed forests are an emergent property of biome blending — no special-case code needed. The boundary band sees neighbouring biome weights and samples from a union pool.
- Cross-game lift: the biome-assignment-then-feature-overlay pattern is engine-generic. Any voxel game with terrain wants this shape.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/world_gen.rs` (if it exists; otherwise the `chunk.rs` per-column gen path) — biome assignment runs here, before surface block decisions.
- `game/engine/src/block.rs` — new surface blocks: `BIRCH_LOG`/`BIRCH_LEAVES`/`SPRUCE_LOG`/`SPRUCE_LEAVES`/`JUNGLE_LOG`/`JUNGLE_LEAVES`/`ACACIA_LOG`/`ACACIA_LEAVES` (cross-coordinated with 28b Woods), plus `RED_SAND` (savanna/badlands tint), `COARSE_DIRT`, `PODZOL` (taiga).
- `game/engine/src/spawning.rs` — mob spawn weights become biome-aware.
- `game/engine/src/spawn_pref.rs` — already framed around biome-prefer semantics; this is where the actual biome enum plugs in.

### New modules

- `game/engine/src/biome.rs` (already exists in light form for the pure-deepslate richness threshold; expand to hold the public `Biome` enum + pure functions).

### Related specs

- Spec 02 §3.4 — world-gen pipeline; biome assignment slots in just after the heightmap noise pass.
- Spec 5 §4.2 — mob spawn rules; biome weights override the existing per-time table.

### Memory pointers

- uk english naming — "Tundra" not "Tundra Biome" in UI; "Plains" / "Savanna" with double-n.
- shared infra strategy — Biome enum + assignment function is engine-generic. Per-game flavour comes via the registration table.

---

## Phasing

| # | Phase | Files | LOC | Solo? |
|---|---|---|---|---|
| 1 | **This spec** | this doc | — | — |
| 2 | `Biome` enum + `BiomeProperties` struct (surface_block, subsurface_block, tree_species_pool, mob_weights, top-block color tint) + `BIOME_TABLE` const | `biome.rs` | ~150 | ✓ |
| 3 | `assign_biome(world_x, world_z, seed)` pure function — two-octave Perlin temperature/humidity sample, classify into the 8 biomes via Whittaker-style grid | `biome.rs` | ~120 | ✓ |
| 4 | `blend_biome_weights(x, z, seed) -> [(Biome, f32); N]` — read neighbouring biomes at ±16 blocks and weight by distance for transition smoothing | `biome.rs` | ~100 | ✓ |
| 5 | World-gen wire-up — per-column biome assignment, surface block selection drives from `BiomeProperties.surface_block`. Existing per-block surface logic in chunk gen routes through this. | `chunk.rs` or `world_gen.rs` | ~200 | ✓ build, ⚠ playtest |
| 6 | Tree placement reads weighted biome pool — co-developed with 28b Woods, but the biome side is the weight table. | `chunk.rs` | ~150 | ⚠ playtest |
| 7 | Mob spawning reads biome weights — extend `spawning::tick_mob_spawning` to multiply the existing per-mob rate by the local biome's weight for that mob | `spawning.rs` | ~120 | ⚠ playtest |
| 8 | Tests — biome assignment determinism for fixed seed, blend at biome boundary returns multi-weight, tree pool union across mixed boundary, default mob weights sum to 1.0 | `biome.rs::tests` | ~150 | ✓ |
| 9 | Player Guide page — `tools/sites/docs/content/player-guide/biomes.md` listing the 8 biomes + what to expect | docs | ~80 | ✓ |
| 10 | Axolittle playtest — fly around with `/time speed 64` + `/tp` to confirm: (a) Desert looks like desert, (b) Snowy Tundra has snow + spruce, (c) Jungle is dense, (d) mixed Forest/Birch boundary shows both tree species | — | — | playtest gate |

**Total Phases 2-9:** ~1,070 LOC + ~80 lines of docs. Phase 10 is the gate.

---

## §2 — Biome enum + properties table

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Biome {
    Plains,
    Forest,
    BirchForest,
    Taiga,
    Jungle,
    Savanna,
    Desert,
    SnowyTundra,
}

pub struct BiomeProperties {
    pub surface_block: BlockId,
    pub subsurface_block: BlockId,
    /// Species pool for tree placement; weighted random within. Empty
    /// = treeless biome (Desert, Plains-low-density).
    pub tree_species: &'static [TreeSpecies],
    /// Mob spawn weight multipliers. 1.0 = baseline; 0.0 = never spawn.
    /// Mobs absent from the table use 1.0 by default.
    pub mob_weights: &'static [(MobType, f32)],
    /// Density: trees-per-chunk Poisson mean.
    pub tree_density: f32,
    /// Surface temperature for `snow_layer_at_surface` decision.
    pub temperature: f32,
    /// Light cosmetic tint; alpha implementation can stub to identity.
    pub grass_tint: [f32; 3],
}

pub const BIOMES: [(Biome, BiomeProperties); 8] = [
    (Biome::Plains, BiomeProperties { surface_block: GRASS, subsurface_block: DIRT, tree_species: &[TreeSpecies::Oak], tree_density: 0.5, mob_weights: &[(MobType::Cow, 2.0), (MobType::Pig, 2.0)], temperature: 0.8, grass_tint: [0.55, 0.83, 0.30] }),
    (Biome::Forest, BiomeProperties { surface_block: GRASS, subsurface_block: DIRT, tree_species: &[TreeSpecies::Oak, TreeSpecies::Birch], tree_density: 6.0, mob_weights: &[(MobType::Wolf, 0.0)], temperature: 0.7, grass_tint: [0.50, 0.78, 0.25] }),
    (Biome::BirchForest, BiomeProperties { surface_block: GRASS, subsurface_block: DIRT, tree_species: &[TreeSpecies::Birch], tree_density: 6.0, mob_weights: &[], temperature: 0.6, grass_tint: [0.52, 0.80, 0.30] }),
    (Biome::Taiga, BiomeProperties { surface_block: GRASS, subsurface_block: PODZOL, tree_species: &[TreeSpecies::Spruce], tree_density: 8.0, mob_weights: &[(MobType::Wolf, 0.0)], temperature: 0.25, grass_tint: [0.42, 0.62, 0.30] }),
    (Biome::Jungle, BiomeProperties { surface_block: GRASS, subsurface_block: DIRT, tree_species: &[TreeSpecies::Jungle], tree_density: 12.0, mob_weights: &[(MobType::Ocelot, 0.0)], temperature: 0.95, grass_tint: [0.30, 0.92, 0.20] }),
    (Biome::Savanna, BiomeProperties { surface_block: GRASS, subsurface_block: DIRT, tree_species: &[TreeSpecies::Acacia], tree_density: 1.0, mob_weights: &[(MobType::Cow, 1.0), (MobType::Sheep, 1.0)], temperature: 1.2, grass_tint: [0.75, 0.80, 0.35] }),
    (Biome::Desert, BiomeProperties { surface_block: SAND, subsurface_block: SANDSTONE, tree_species: &[], tree_density: 0.0, mob_weights: &[(MobType::Cow, 0.0), (MobType::Pig, 0.0), (MobType::Sheep, 0.0)], temperature: 2.0, grass_tint: [0.85, 0.85, 0.50] }),
    (Biome::SnowyTundra, BiomeProperties { surface_block: SNOW, subsurface_block: DIRT, tree_species: &[TreeSpecies::Spruce], tree_density: 1.0, mob_weights: &[(MobType::Cow, 0.0), (MobType::Pig, 0.0)], temperature: -0.5, grass_tint: [0.50, 0.72, 0.55] }),
];
```

`TreeSpecies` is co-defined with 28b Woods. `MobType` is the existing enum.

---

## §3 — Assignment via Whittaker climate sample

Two octaves of Perlin noise on (worldx/512, worldz/512) → temperature in [-1, 2] and humidity in [0, 1]. Classify into a 4×3 Whittaker grid (cold/cool/warm/hot × dry/mid/wet); each cell maps to one of the 8 biomes via a lookup table.

The seed in the noise generator is the world seed XOR a domain salt — biomes are stable for a given world, but two worlds with different seeds get different biome layouts.

---

## §4 — Blending

For mob spawn + tree species, sample 5 points (centre + 4 cardinals at ±16 blocks). Each point votes for one biome; the centre weighs 0.5, the four cardinals 0.125 each. The resulting weighted bag is the "biome blend" at this position. When two biomes share the bag, tree placement samples uniformly from their union pool → mixed forest.

The visual surface block uses pure centre (no blending) — blending the surface block would produce checkerboard noise at borders, which looks worse than a clean transition. Trees + mobs blend because they're already stochastic.

---

## §5-7 — Wire-up

Surface gen: per-column, call `biome_at(x, z, seed)` once, read `props.surface_block`, write to the highest non-air y. Sub-surface fills 3-4 blocks below.

Trees: when the existing tree-placement logic decides "tree here", pull the species from `blend_biome_weights(x, z, seed)` and call `place_tree(species, x, surface_y, z)`. Co-developed with 28b.

Mobs: `tick_mob_spawning` per-attempt — look up the biome at the candidate spawn position, multiply the mob's baseline weight by the biome's modifier, accept/reject.

---

## §8 — Tests

```rust
#[test]
fn biome_assignment_is_deterministic_for_seed() { /* same (x,z,seed) → same biome */ }

#[test]
fn biome_table_is_complete() { /* every Biome variant has an entry */ }

#[test]
fn desert_has_no_trees() {
    let props = lookup(Biome::Desert);
    assert!(props.tree_species.is_empty());
    assert_eq!(props.tree_density, 0.0);
}

#[test]
fn blend_at_clean_boundary_returns_two_weighted_entries() { /* synthetic biome map */ }

#[test]
fn snowy_tundra_uses_snow_surface() {
    assert_eq!(lookup(Biome::SnowyTundra).surface_block, SNOW);
}

#[test]
fn mob_weight_lookup_defaults_to_one() { /* mobs not in table return 1.0 */ }
```

---

## §10 — Playtest

- Fly with creative + `/time speed 64` from origin to (5000, 5000) — count distinct biomes (target ≥ 5).
- Land in a Forest/Birch Forest border. Confirm trees of both species within ~32 blocks of the boundary.
- Stand in Desert at night. Confirm no Cow/Pig/Sheep spawns over 5 minutes (creative time).
- Stand in Taiga. Confirm spruce trees + podzol under grass.

Questions:
- Are the biomes large enough to feel like "places" or do they feel slivery?
- Is the climate noise scale right (512-block period feels Minecraft-ish; could be 256 or 1024)?
- Does Plains feel too empty? Tweak density up?

---

## Acceptance — sub-foundation 28a overall

- `./check.sh` ALL GREEN.
- Tests pass.
- Manual playtest checks above.
- Player Guide page added.
- Foundations README updated.

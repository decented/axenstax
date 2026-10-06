//! Biome system — temperature/humidity noise selects biome per column.
//!
//! Spec 02 Section 5.3: 6D parameter space reduced to 2D for prototype
//! (temperature + humidity). Full spec uses continentalness, erosion, depth, weirdness.

use noise::{NoiseFn, OpenSimplex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Biome {
    Plains,
    Forest,
    Desert,
    Mountains,
    Ocean,
    // Spec 28a — additional launch biomes (2026-05-20). Each gets a
    // BiomeProperties entry in BIOME_TABLE; assign_biome_whittaker
    // classifies into the 8-launch set (Plains/Forest/BirchForest/
    // Taiga/Jungle/Savanna/Desert/SnowyTundra). Mountains and Ocean
    // remain as terrain-shape biomes from the existing pipeline; new
    // 28a additions are climate-shape biomes.
    BirchForest,
    Taiga,
    Jungle,
    Savanna,
    SnowyTundra,
}

impl Biome {
    /// Human-readable label for the HUD/debug overlay (#44). UK English;
    /// multi-word biomes are spaced (e.g. "Birch Forest", "Snowy Tundra").
    pub fn name(&self) -> &'static str {
        match self {
            Biome::Plains => "Plains",
            Biome::Forest => "Forest",
            Biome::Desert => "Desert",
            Biome::Mountains => "Mountains",
            Biome::Ocean => "Ocean",
            Biome::BirchForest => "Birch Forest",
            Biome::Taiga => "Taiga",
            Biome::Jungle => "Jungle",
            Biome::Savanna => "Savanna",
            Biome::SnowyTundra => "Snowy Tundra",
        }
    }
}

/// Sea level — water fills below this in ocean biomes and low areas.
pub const SEA_LEVEL: i32 = 62;

/// P10 — caves at or below this depth pool with lava (deep-cave lava lakes).
/// Sits in the deep-mining band (bedrock at y0, diamonds y<15), so reaching
/// diamonds means braving lava — the Minecraft risk/reward at depth.
pub const LAVA_CAVE_LEVEL: i32 = 10;

/// Y at which pure deepslate begins replacing stone in world generation
/// (Spec 2 §5.3.1a). In the alpha 96-block-tall world this is `30` — well
/// below sea level and below the existing diamond-ore band, but with enough
/// vertical room beneath for the gem-vein gate at `Y_dp - 21 = 9`.
///
/// Server operators can override this in world config. Tests assume the
/// default; the gem-vein gate (Spec 6 §2.2c) derives from `Y_DP` so they
/// stay in sync if this value changes.
pub const Y_DP: i32 = 30;

/// Vertical thickness of the stone↔deepslate transition zone (Spec 2
/// §5.3.1a). At `Y_DP` the chance of deepslate is 0; at `Y_DP - TRANSITION_DEPTH`
/// it is 1.0. Default 8 blocks of gradual mixing.
pub const DEEPSLATE_TRANSITION_DEPTH: i32 = 8;

/// The Reserve richness world generation bakes deepslate variants with
/// (Spec 16 Phase 3b; Phase B0 worldgen purity).
///
/// Generation must be a pure function of seed + world flags + the worldgen
/// fingerprint, so a joiner regenerates the host's untouched chunks
/// bit-identically. Live Reserve state therefore never feeds it: the host
/// client, a dedicated server and a joiner all bake this constant. 0.75 is
/// what single-player always baked (`ReserveState::synthetic_default()`), so
/// existing worlds show no seam. Richness-driven deepslate visuals move to
/// render time when the reward layer lands
/// (`docs/foundations/2026-05-17-deepslate-reserve.md`). Changing this value
/// changes generation output: bump `world::WORLDGEN_VERSION`.
pub const WORLDGEN_RESERVE_RICHNESS: f32 = 0.75;

pub struct BiomeGenerator {
    /// The world seed this generator was initialised with. Surfaced for
    /// commands like `/seed` and for save migrations.
    pub seed: u32,
    temp_noise: OpenSimplex,
    humidity_noise: OpenSimplex,
    height_noise: OpenSimplex,
    detail_noise: OpenSimplex,
    cave_noise: OpenSimplex,
}

impl BiomeGenerator {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            temp_noise: OpenSimplex::new(seed),
            humidity_noise: OpenSimplex::new(seed.wrapping_add(1000)),
            height_noise: OpenSimplex::new(seed.wrapping_add(2000)),
            detail_noise: OpenSimplex::new(seed.wrapping_add(3000)),
            cave_noise: OpenSimplex::new(seed.wrapping_add(4000)),
        }
    }

    /// Continentalness / elevation field — a broad, low-frequency noise
    /// independent of climate. Drives the terrain-shape biomes: low
    /// values are ocean basins, high values are mountain massifs, the
    /// mid band is normal land where climate decides the biome. Returns
    /// roughly `[-1, 1]`. Sampled coarser (0.0015) than climate (0.003)
    /// so continents are larger than the climate regions inside them.
    pub fn continentalness(&self, x: i32, z: i32) -> f64 {
        self.height_noise.get([x as f64 * 0.0015, z as f64 * 0.0015])
    }

    /// Sample biome at world column (x, z).
    ///
    /// Spec 28a wire-up (2026-05-27): the terrain-shape biomes
    /// (Ocean/Mountains) are now elevation-driven via `continentalness`,
    /// and everything in the mid-elevation band is classified by the
    /// tested Whittaker climate grid (`classify_whittaker`) fed from the
    /// coherent temperature/humidity noise. Decoupling Mountains from
    /// temperature is what frees the cold-climate band to become Taiga /
    /// Snowy Tundra — previously `temp < -0.3` swallowed all of it.
    pub fn biome_at(&self, x: i32, z: i32) -> Biome {
        let cont = self.continentalness(x, z);
        // Terrain-shape biomes first — decided by elevation, not climate.
        if cont < -0.45 {
            return Biome::Ocean;
        }
        if cont > 0.45 {
            return Biome::Mountains;
        }
        // Mid-elevation land: climate decides. Reuse the coherent
        // OpenSimplex temp/humidity (scale 0.003 ≈ 333-block regions)
        // and run it through the Whittaker classifier.
        let scale = 0.003;
        let temp = self.temp_noise.get([x as f64 * scale, z as f64 * scale]);
        let humidity = self.humidity_noise.get([x as f64 * scale, z as f64 * scale]);
        classify_whittaker(temp as f32, humidity as f32)
    }

    /// Terrain height at world position. Multi-octave noise, shaped by biome.
    pub fn terrain_height(&self, x: i32, z: i32) -> i32 {
        let biome = self.biome_at(x, z);

        // Base continent shape (low frequency)
        let base = self.height_noise.get([x as f64 * 0.002, z as f64 * 0.002]);
        // Medium detail
        let detail = self.detail_noise.get([x as f64 * 0.01, z as f64 * 0.01]);
        // Fine detail
        let fine = self.detail_noise.get([x as f64 * 0.04, z as f64 * 0.04]);

        let combined = base * 0.6 + detail * 0.3 + fine * 0.1;

        // Bugfix 2026-05-21 — pre-fix, lake-prone biomes produced
        // 1-block-deep puddles in Forest/Plains because `combined`
        // rarely went much below 0. Apply a 4× depth boost to the
        // negative branch in lake-prone biomes so dips below sea
        // level carve proper lake basins (3-5 blocks deep typically).
        // Land elevation (positive branch) is unchanged.
        let scale_lake = |amp: f32| -> i32 {
            let raw = combined as f32 * amp;
            (if raw < 0.0 { raw * 4.0 } else { raw }) as i32
        };

        match biome {
            Biome::Plains => SEA_LEVEL + scale_lake(8.0),
            Biome::Forest => SEA_LEVEL + scale_lake(10.0),
            Biome::Desert => SEA_LEVEL + (combined * 6.0) as i32, // no lake boost
            Biome::Mountains => SEA_LEVEL + 10 + (combined * 30.0) as i32,
            Biome::Ocean => SEA_LEVEL - 8 + (combined * 5.0) as i32, // already deep
            // Spec 28a — climate-shape biomes share the Plains/Forest
            // height curve until 28a world-gen wire-up lands. The
            // assign_biome_whittaker classifier is the new entry point;
            // biome_at (legacy) doesn't return these yet.
            Biome::BirchForest => SEA_LEVEL + scale_lake(10.0),
            Biome::Taiga => SEA_LEVEL + scale_lake(12.0),
            // +5 baseline lift (2026-05-30): jungle was generating ~56% of
            // its cells below the waterline (deep lake basins from the 4×
            // negative lake boost), leaving whole jungles drowned and
            // treeless. The lift floats the near-zero-noise majority onto
            // dry land while the deeper dips stay genuine lakes.
            Biome::Jungle => SEA_LEVEL + 5 + scale_lake(14.0),
            Biome::Savanna => SEA_LEVEL + scale_lake(8.0),
            Biome::SnowyTundra => SEA_LEVEL + scale_lake(6.0),
        }
    }

    /// Pick an ore variant for a stone position, or None to leave it as
    /// stone. Depth-banded so coal is common, iron is mid-depth, diamond is
    /// rare and only deep. Magnesium/Brimstone/Nitre/Copper are additional
    /// stone-only bands (no deepslate variant) layered on top — each on its
    /// own hash seed so they don't bias coal/iron/diamond at the same
    /// position. Copper (Wind, Copper & Electricity wave, 2026-09-07) is the
    /// mid-depth band Y_DP..(SEA_LEVEL+8). Deterministic per (x, y, z, seed).
    ///
    /// Stone-tier ores are returned for y ≥ Y_DP; below Y_DP the
    /// deepslate-tier variants are returned. The transition zone (Y_DP -
    /// TRANSITION_DEPTH .. Y_DP) picks variant based on whether the host
    /// rock at that position is stone or pure deepslate (Spec 2 §5.3.1a).
    pub fn ore_at(&self, x: i32, y: i32, z: i32) -> Option<crate::block::BlockId> {
        let h = ore_hash(x, y, z, self.seed);
        let in_deepslate = self.is_deepslate_substrate(x, y, z);
        // Diamond first (rarest, narrowest depth band). NOTE: the diamond band
        // (y < 15) lies entirely below the deepslate-substrate threshold
        // (Y_DP - DEEPSLATE_TRANSITION_DEPTH = 22), so `in_deepslate` is always
        // true here and the plain `DIAMOND_ORE` arm is currently UNREACHABLE —
        // diamonds generate exclusively as DEEPSLATE_DIAMOND_ORE (engine audit
        // 2026-06-04, E: the old "spec mismatch" was the spec, not the code).
        // The conditional is kept defensively: widening the band above y=22 (or
        // lowering Y_DP) must be a deliberate change — `diamond_band_is_all_deepslate`
        // locks the current reality. See Spec 2 §5.3.
        if y < 15 && (h % 1000) < 4 {
            return Some(if in_deepslate {
                crate::block::DEEPSLATE_DIAMOND_ORE
            } else {
                crate::block::DIAMOND_ORE
            });
        }
        // Iron in the lower 50 layers, ~3% per stone block.
        if y < 50 && ((h / 1000) % 100) < 3 {
            return Some(if in_deepslate {
                crate::block::DEEPSLATE_IRON_ORE
            } else {
                crate::block::IRON_ORE
            });
        }
        // Magnesium (Spec 37) — mid-depth mineral, ~2% in stone in the band
        // Y_DP..48 (above the deepslate threshold, so it's always in stone —
        // it has no deepslate variant, and the ore↔substrate invariant only
        // holds for stone). Distinct hash slice so it doesn't bias the
        // coal/iron/diamond bands at the same position.
        if (Y_DP..48).contains(&y) && ((h / 10_000) % 100) < 2 {
            return Some(crate::block::MAGNESIUM_ORE);
        }
        // Spec 49 — Brimstone (sulphur ore): a deep stone band (Y_DP..48), ~iron
        // rarity. Stone-only (no deepslate variant, like Magnesium), so it sits
        // above the deepslate threshold and `ore_variant_matches_substrate` holds.
        // Own hash seed so it doesn't bias the coal/iron/magnesium bands.
        if (Y_DP..48).contains(&y) {
            let hb = ore_hash(x, y, z, self.seed.wrapping_add(4900));
            if (hb % 1000) < 30 {
                return Some(crate::block::BRIMSTONE);
            }
        }
        // Spec 49 — Nitre ore (saltpetre): a shallower arid / cave-wall band
        // (Y_DP..SEA_LEVEL), ~iron rarity, stone-only. The miner's route to
        // saltpetre — the Composter is the farmer's route to the same material.
        if (Y_DP..SEA_LEVEL).contains(&y) {
            let hn = ore_hash(x, y, z, self.seed.wrapping_add(4901));
            if (hn % 1000) < 28 {
                return Some(crate::block::NITRE_ORE);
            }
        }
        // Wind, Copper & Electricity wave (2026-09-07) — Copper Ore: a
        // mid-depth stone band (Y_DP..SEA_LEVEL+8), ~iron rarity. Stone-only
        // (no deepslate variant, like Magnesium/Brimstone/Nitre) — the band
        // sits entirely at/above Y_DP, so `ore_variant_matches_substrate` and
        // `copper_is_stone_only_never_in_deepslate` keep holding. Own hash
        // seed so it doesn't bias the other bands at the same position. Spec
        // 02 §5.3.1a; closes the "Electricity is a Survival dead end" gap
        // (Copper Ore previously never generated naturally).
        if (Y_DP..(SEA_LEVEL + 8)).contains(&y) {
            let hc = ore_hash(x, y, z, self.seed.wrapping_add(4902));
            if (hc % 1000) < 30 {
                return Some(crate::block::COPPER_ORE);
            }
        }
        // Coal anywhere in stone, ~6%.
        if (h / 100_000) % 100 < 6 {
            return Some(if in_deepslate {
                crate::block::DEEPSLATE_COAL_ORE
            } else {
                crate::block::COAL_ORE
            });
        }
        None
    }

    /// Does the substrate at this position resolve to pure deepslate
    /// (vs. plain stone)? Spec 2 §5.3.1a:
    ///   y >= Y_DP                                  → stone
    ///   y <  Y_DP - TRANSITION_DEPTH               → deepslate
    ///   in between → probabilistic mix weighted toward deepslate by depth
    ///
    /// Deterministic per (x, y, z, seed). Used by chunk generation to pick
    /// between STONE and PURE_DEEPSLATE, and by `ore_at` to pick the right
    /// ore variant.
    pub fn is_deepslate_substrate(&self, x: i32, y: i32, z: i32) -> bool {
        if y >= Y_DP {
            return false;
        }
        if y < Y_DP - DEEPSLATE_TRANSITION_DEPTH {
            return true;
        }
        // Transition zone: p = (Y_DP - y) / TRANSITION_DEPTH in [0, 1].
        let depth = (Y_DP - y) as u32;
        let p = depth.saturating_mul(1000) / DEEPSLATE_TRANSITION_DEPTH as u32; // 0..=1000
        let h = ore_hash(x, y, z, self.seed.wrapping_add(7777));
        (h % 1000) < p
    }

    /// Resolve the base block at this position (no ore overlay applied).
    /// Returns a PURE_DEEPSLATE-family variant in the deepslate region
    /// (per `is_deepslate_substrate`) — variant chosen from the fixed
    /// [`WORLDGEN_RESERVE_RICHNESS`] via [`pick_deepslate_variant`] — and
    /// STONE otherwise. Callers (chunk-gen) should call `ore_at` first
    /// and fall back to this if no ore is placed.
    ///
    /// Salt feature (2026-05-23) — in Mountains + Desert biomes between
    /// y = 60 and surface, the STONE fallback can be substituted by
    /// ROCK_SALT via `try_rock_salt_vein`. Deepslate still wins below
    /// the deepslate threshold.
    pub fn base_rock_at(&self, x: i32, y: i32, z: i32) -> crate::block::BlockId {
        if self.is_deepslate_substrate(x, y, z) {
            return pick_deepslate_variant(x, y, z, self.seed, WORLDGEN_RESERVE_RICHNESS);
        }
        if let Some(b) = self.try_rock_salt_vein(x, y, z) {
            return b;
        }
        crate::block::STONE
    }

    /// Salt feature — vein test for ROCK_SALT. Returns `Some(ROCK_SALT)`
    /// when the position passes the biome gate (Mountains | Desert),
    /// the depth gate (y ∈ [60, terrain_height]), AND a two-axis noise
    /// gate that clusters veins in groups of 3-6 blocks. Returns `None`
    /// otherwise so callers fall through to STONE.
    fn try_rock_salt_vein(&self, x: i32, y: i32, z: i32) -> Option<crate::block::BlockId> {
        let biome = self.biome_at(x, z);
        if !matches!(biome, Biome::Mountains | Biome::Desert) {
            return None;
        }
        if y < 60 || y > self.terrain_height(x, z) {
            return None;
        }
        // Two-axis hash gate. Horizontal noise (x, z) clusters veins
        // into deposits; vertical noise (y, x) shapes the vein depth
        // band so a deposit isn't a single solid column.
        let h_xz = ore_hash(x, 0, z, self.seed.wrapping_add(4242));
        let h_yx = ore_hash(y, x, 0, self.seed.wrapping_add(4243));
        // Roughly 5 % of qualifying cells become ROCK_SALT — enough to
        // form visible deposits in the band without flooding mountains.
        if (h_xz % 100) < 5 && (h_yx % 100) < 40 {
            Some(crate::block::ROCK_SALT)
        } else {
            None
        }
    }

    /// Returns true if a cave should exist at this 3D position.
    pub fn is_cave(&self, x: i32, y: i32, z: i32) -> bool {
        if y <= 1 || y >= 55 {
            return false;
        }
        let scale = 0.05;
        let noise_val = self.cave_noise.get([
            x as f64 * scale,
            y as f64 * scale,
            z as f64 * scale,
        ]);
        noise_val.abs() < 0.08
    }
}

fn ore_hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(374761393)
        ^ (y as u32).wrapping_mul(668265263)
        ^ (z as u32).wrapping_mul(1274126177)
        ^ seed.wrapping_mul(2246822519);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

// ── Spec 28a — Biome pure-function layer (2026-05-20) ──────────────
//
// Whittaker climate classifier + per-biome properties table + biome
// blending for mixed-forest support. Pure functions; world-gen
// wire-up (replacing the legacy `BiomeGenerator::biome_at` with this
// classifier) is DEFERRED to playtest.

/// The eight launch biomes the Whittaker classifier returns. Subset
/// of the full `Biome` enum (which also includes Mountains + Ocean
/// from the legacy terrain-shape pipeline). Future iterations will
/// fold Mountains + Ocean into the new layer. Test-only for now (see the
/// "DEFERRED to playtest" note above).
#[cfg_attr(not(test), allow(dead_code))]
pub const LAUNCH_BIOMES: &[Biome] = &[
    Biome::Plains,
    Biome::Forest,
    Biome::BirchForest,
    Biome::Taiga,
    Biome::Jungle,
    Biome::Savanna,
    Biome::Desert,
    Biome::SnowyTundra,
];

/// Tree species pool per biome — used by 28b's per-species tree
/// placement (deferred to playtest). Empty pool = treeless biome.
#[derive(Clone, Copy, Debug)]
pub struct BiomeProperties {
    pub surface_block: crate::block::BlockId,
    /// Set on every table row but no consumer reads it back yet (the
    /// Whittaker-layer wire-up is deferred — see the module note above).
    #[allow(dead_code)]
    pub subsurface_block: crate::block::BlockId,
    /// Tree species pool — caller weights uniformly.
    pub tree_species: &'static [crate::block::WoodSpecies],
    /// Poisson mean trees per chunk.
    pub tree_density: f32,
    /// Surface temperature (Whittaker x-axis sample). Meant to drive snow
    /// overlay + crop-growth biome bonuses, but neither consumer reads it yet.
    #[allow(dead_code)]
    pub temperature: f32,
    /// Cosmetic grass tint — alpha may stub to identity. No renderer consumer yet.
    #[allow(dead_code)]
    pub grass_tint: [f32; 3],
}

/// Look up the properties for one of the eight launch biomes. For
/// legacy biomes (Mountains/Ocean) returns Plains' properties as a
/// safe fallback — those biomes will get their own entries when the
/// 28a wire-up replaces the legacy terrain-shape pipeline.
pub fn biome_properties(biome: Biome) -> BiomeProperties {
    use crate::block as B;
    use crate::block::WoodSpecies as W;
    match biome {
        Biome::Plains => BiomeProperties {
            surface_block: B::GRASS, subsurface_block: B::DIRT,
            tree_species: &[W::Oak], tree_density: 0.5,
            temperature: 0.8, grass_tint: [0.55, 0.83, 0.30],
        },
        Biome::Forest => BiomeProperties {
            surface_block: B::GRASS, subsurface_block: B::DIRT,
            tree_species: &[W::Oak, W::Birch], tree_density: 6.0,
            temperature: 0.7, grass_tint: [0.50, 0.78, 0.25],
        },
        Biome::BirchForest => BiomeProperties {
            surface_block: B::GRASS, subsurface_block: B::DIRT,
            tree_species: &[W::Birch], tree_density: 6.0,
            temperature: 0.6, grass_tint: [0.52, 0.80, 0.30],
        },
        Biome::Taiga => BiomeProperties {
            surface_block: B::GRASS, subsurface_block: B::DIRT,
            tree_species: &[W::Spruce], tree_density: 8.0,
            temperature: 0.25, grass_tint: [0.42, 0.62, 0.30],
        },
        Biome::Jungle => BiomeProperties {
            surface_block: B::GRASS, subsurface_block: B::DIRT,
            // Spec 32 follow-on: Rubber joins the Jungle pool so tapped
            // trees are discoverable in fresh worlds without /give.
            // 50/50 with Jungle; world-gen picks uniformly.
            tree_species: &[W::Jungle, W::Rubber], tree_density: 12.0,
            temperature: 0.95, grass_tint: [0.30, 0.92, 0.20],
        },
        Biome::Savanna => BiomeProperties {
            surface_block: B::GRASS, subsurface_block: B::DIRT,
            tree_species: &[W::Acacia], tree_density: 1.0,
            temperature: 1.2, grass_tint: [0.75, 0.80, 0.35],
        },
        Biome::Desert => BiomeProperties {
            surface_block: B::SAND, subsurface_block: B::SANDSTONE,
            tree_species: &[], tree_density: 0.0,
            temperature: 2.0, grass_tint: [0.85, 0.85, 0.50],
        },
        Biome::SnowyTundra => BiomeProperties {
            surface_block: B::SNOW, subsurface_block: B::DIRT,
            tree_species: &[W::Spruce], tree_density: 1.0,
            temperature: -0.5, grass_tint: [0.50, 0.72, 0.55],
        },
        // Legacy biomes — fall back to Plains until 28a fully replaces
        // the terrain-shape pipeline.
        Biome::Mountains | Biome::Ocean => BiomeProperties {
            surface_block: B::GRASS, subsurface_block: B::DIRT,
            tree_species: &[W::Oak], tree_density: 0.5,
            temperature: 0.5, grass_tint: [0.55, 0.83, 0.30],
        },
    }
}

/// Deterministic value-noise sample on a 2-octave grid keyed by seed +
/// salt. Returns `[-1, 1)`. Pure; no engine state. Used so
/// `assign_biome_whittaker` is fully testable without dragging in the
/// OpenSimplex crate's runtime state.
fn value_noise_2d(x: f64, z: f64, seed: u32, salt: u32) -> f32 {
    // 2 octaves of value noise. Fold the seed into the constant term so
    // outputs vary with seed even at (0, 0) — otherwise the multiplier
    // collapses to zero and the test for "varies with seed" fails.
    fn mix(v: u64) -> u64 {
        let mut h = v.wrapping_mul(0x9E3779B97F4A7C15);
        h ^= h >> 27;
        h = h.wrapping_mul(0xBF58476D1CE4E5B9);
        h ^= h >> 31;
        h.wrapping_mul(0x94D049BB133111EB) ^ (h >> 32)
    }
    let key1 = ((x * 100.0) as i64 as u64)
        .wrapping_add((z as i64 as u64).wrapping_mul(73856093))
        .wrapping_add(seed as u64)
        .wrapping_add((salt as u64).wrapping_mul(0xA2A2A2));
    let h1 = mix(key1);
    let n1 = ((h1 >> 32) as u32 as f32 / u32::MAX as f32) * 2.0 - 1.0;
    let key2 = ((x * 400.0) as i64 as u64)
        .wrapping_add((z as i64 as u64).wrapping_mul(19349663))
        .wrapping_add((seed as u64).wrapping_mul(2654435761))
        .wrapping_add((salt as u64).wrapping_mul(0x5A5A5A));
    let h2 = mix(key2);
    let n2 = ((h2 >> 32) as u32 as f32 / u32::MAX as f32) * 2.0 - 1.0;
    n1 * 0.7 + n2 * 0.3
}

/// Spec 28a Whittaker classifier. Pure: given (x, z, seed) returns
/// one of the 8 launch biomes. Samples temperature + humidity via
/// `value_noise_2d` (deterministic) and runs a 4×3 climate grid:
///
///   cold + dry    →  Snowy Tundra
///   cold + mid    →  Taiga
///   cold + wet    →  Taiga
///   cool + dry    →  Plains
///   cool + mid    →  Forest
///   cool + wet    →  Birch Forest
///   warm + dry    →  Savanna
///   warm + mid    →  Forest
///   warm + wet    →  Jungle
///   hot  + dry    →  Desert
///   hot  + mid    →  Savanna
///   hot  + wet    →  Jungle
pub fn assign_biome_whittaker(x: i32, z: i32, seed: u32) -> Biome {
    let temp = value_noise_2d(x as f64, z as f64, seed, 0x71_C5);
    let humidity = value_noise_2d(x as f64, z as f64, seed, 0xA3_47);
    classify_whittaker(temp, humidity)
}

/// Pure classification from temperature + humidity to biome.
/// Extracted so tests can exercise the grid corners directly without
/// reverse-engineering noise inputs.
pub fn classify_whittaker(temp: f32, humidity: f32) -> Biome {
    // Temperature bands: cold (< -0.4), cool (< 0.0), warm (< 0.5), hot.
    // Humidity bands: dry (< -0.2), mid (< 0.3), wet.
    let temp_band = if temp < -0.4 {
        0
    } else if temp < 0.0 {
        1
    } else if temp < 0.5 {
        2
    } else {
        3
    };
    let humidity_band = if humidity < -0.2 {
        0
    } else if humidity < 0.3 {
        1
    } else {
        2
    };
    match (temp_band, humidity_band) {
        (0, 0) => Biome::SnowyTundra,
        (0, 1) | (0, 2) => Biome::Taiga,
        (1, 0) => Biome::Plains,
        (1, 1) => Biome::Forest,
        (1, 2) => Biome::BirchForest,
        (2, 0) => Biome::Savanna,
        (2, 1) => Biome::Forest,
        (2, 2) => Biome::Jungle,
        (3, 0) => Biome::Desert,
        (3, 1) => Biome::Savanna,
        (3, 2) => Biome::Jungle,
        _ => Biome::Plains,
    }
}

/// Blend the biomes around a sample point — weight `centre` at 0.5,
/// each of the 4 cardinal neighbours (±16 blocks) at 0.125. Returns
/// a small vec of `(Biome, weight)` pairs where same-biome neighbours
/// stack. Drives mixed-forest tree placement: when two forest biomes
/// touch, the boundary samples from both species pools.
///
/// Pure: classifier is deterministic, so this is too.
#[cfg_attr(not(test), allow(dead_code))]
pub fn blend_biome_weights(x: i32, z: i32, seed: u32) -> Vec<(Biome, f32)> {
    let samples = [
        (assign_biome_whittaker(x, z, seed), 0.5),
        (assign_biome_whittaker(x + 16, z, seed), 0.125),
        (assign_biome_whittaker(x - 16, z, seed), 0.125),
        (assign_biome_whittaker(x, z + 16, seed), 0.125),
        (assign_biome_whittaker(x, z - 16, seed), 0.125),
    ];
    let mut out: Vec<(Biome, f32)> = Vec::new();
    for (b, w) in samples {
        if let Some(existing) = out.iter_mut().find(|e| e.0 == b) {
            existing.1 += w;
        } else {
            out.push((b, w));
        }
    }
    out
}

#[cfg(test)]
mod spec_28a_tests {
    use super::*;

    #[test]
    fn whittaker_classifier_covers_all_eight_launch_biomes() {
        // Sweep the climate grid and confirm every launch biome is
        // reachable via some (temp, humidity) input.
        use std::collections::HashSet;
        let mut seen: HashSet<Biome> = HashSet::new();
        for t_i in -10..=10 {
            for h_i in -10..=10 {
                let t = t_i as f32 * 0.15;
                let h = h_i as f32 * 0.15;
                seen.insert(classify_whittaker(t, h));
            }
        }
        for b in LAUNCH_BIOMES {
            assert!(seen.contains(b),
                "launch biome {:?} unreachable via classify_whittaker", b);
        }
    }

    #[test]
    fn classifier_specific_grid_cells() {
        // Spot-check a few cells against the rule table.
        assert_eq!(classify_whittaker(-0.6, -0.5), Biome::SnowyTundra);
        assert_eq!(classify_whittaker(-0.6, 0.4), Biome::Taiga);
        assert_eq!(classify_whittaker(-0.2, 0.5), Biome::BirchForest);
        assert_eq!(classify_whittaker(0.2, -0.5), Biome::Savanna);
        assert_eq!(classify_whittaker(0.2, 0.1), Biome::Forest);
        assert_eq!(classify_whittaker(0.2, 0.5), Biome::Jungle);
        assert_eq!(classify_whittaker(0.7, -0.5), Biome::Desert);
        assert_eq!(classify_whittaker(0.7, 0.5), Biome::Jungle);
    }

    #[test]
    fn assign_biome_is_deterministic_for_seed() {
        let a = assign_biome_whittaker(1000, 2000, 42);
        let b = assign_biome_whittaker(1000, 2000, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn assign_biome_varies_with_seed() {
        // Across many seeds at the same position, expect at least 2
        // distinct biomes — confirms the seed actually influences output.
        use std::collections::HashSet;
        let mut seen: HashSet<Biome> = HashSet::new();
        for seed in 0..100u32 {
            seen.insert(assign_biome_whittaker(0, 0, seed));
        }
        assert!(seen.len() >= 2,
            "expected ≥2 distinct biomes across 100 seeds, got {}", seen.len());
    }

    #[test]
    fn biome_properties_complete_for_all_launch_biomes() {
        for b in LAUNCH_BIOMES {
            let props = biome_properties(*b);
            assert!(props.temperature >= -1.0 && props.temperature <= 2.5,
                "{:?} temp out of range: {}", b, props.temperature);
            // Desert has no trees — others should have at least one.
            if *b == Biome::Desert {
                assert!(props.tree_species.is_empty());
                assert_eq!(props.tree_density, 0.0);
            } else {
                assert!(!props.tree_species.is_empty(),
                    "{:?} should have at least one tree species", b);
                assert!(props.tree_density > 0.0);
            }
        }
    }

    #[test]
    fn desert_has_sand_surface() {
        assert_eq!(biome_properties(Biome::Desert).surface_block, crate::block::SAND);
    }

    #[test]
    fn snowy_tundra_has_snow_surface() {
        assert_eq!(biome_properties(Biome::SnowyTundra).surface_block, crate::block::SNOW);
    }

    #[test]
    fn jungle_has_highest_tree_density() {
        // Sanity: Jungle should be densest among non-Desert biomes.
        let jungle = biome_properties(Biome::Jungle).tree_density;
        for b in [Biome::Forest, Biome::BirchForest, Biome::Taiga,
                  Biome::Savanna, Biome::SnowyTundra, Biome::Plains] {
            assert!(jungle >= biome_properties(b).tree_density,
                "Jungle density {} should be ≥ {:?} density", jungle, b);
        }
    }

    #[test]
    fn blend_weights_sum_to_one() {
        let blend = blend_biome_weights(100, 100, 42);
        let total: f32 = blend.iter().map(|(_, w)| w).sum();
        assert!((total - 1.0).abs() < 0.001,
            "weights should sum to 1.0, got {total}");
    }

    #[test]
    fn blend_returns_at_most_five_distinct_biomes() {
        // Trivial structure check — 5 samples can return at most 5
        // distinct biomes, often fewer when neighbours collide.
        for seed in 0..20u32 {
            let blend = blend_biome_weights(seed as i32 * 100, 0, seed);
            assert!(!blend.is_empty(), "blend should always return ≥1 entry");
            assert!(blend.len() <= 5, "blend at most 5 distinct entries");
        }
    }

    #[test]
    fn blend_centre_carries_at_least_half_the_weight() {
        // The centre sample is weighted 0.5 — at minimum some biome
        // must carry weight ≥ 0.5 (the centre biome).
        let blend = blend_biome_weights(100, 100, 42);
        let max_weight = blend.iter().map(|(_, w)| *w).fold(0.0_f32, f32::max);
        assert!(max_weight >= 0.5 - 0.001,
            "centre biome should carry ≥ 0.5 weight, got max {max_weight}");
    }

    #[test]
    fn forest_pool_contains_oak_and_birch() {
        let props = biome_properties(Biome::Forest);
        assert!(props.tree_species.contains(&crate::block::WoodSpecies::Oak));
        assert!(props.tree_species.contains(&crate::block::WoodSpecies::Birch));
    }
}

/// Spec 16 Phase 3b — pick a deepslate visual variant from a
/// (position, seed, richness) triple. Deterministic per position so
/// the variant doesn't flicker on chunk reload. Richness ∈ [0, 1]
/// drives the tier distribution — at 0.5 the four tiers are roughly
/// evenly spread; at 1.0 the FAT/HEALTHY tiers dominate; at 0.0
/// almost every block is the canonical PURE_DEEPSLATE.
///
/// Returns a member of the `is_pure_deepslate_family` set. Callers
/// (chunk-gen) use this from `base_rock_at` when the substrate check
/// already said "this position is deepslate".
///
/// Mechanic-parity invariant: all four variants behave identically
/// for mining, drops, tool gate, and Proof-of-Play hash (the hash
/// inputs are position-only — block-id is not part of the digest;
/// locked in by `proof_hash_independent_of_deepslate_variant`).
pub fn pick_deepslate_variant(x: i32, y: i32, z: i32, seed: u32, richness: f32) -> crate::block::BlockId {
    use crate::block;
    let r = richness.clamp(0.0, 1.0);
    // Per-position hash, decoupled from `ore_hash` so the variant
    // pick can't bias ore distribution at the same position. Seeded
    // additively so different worlds with the same seed get
    // different variant patterns.
    let mut h = (x as u32).wrapping_mul(2654435761)
        ^ (y as u32).wrapping_mul(40503)
        ^ (z as u32).wrapping_mul(987654321)
        ^ seed.wrapping_mul(514229);
    h = (h ^ (h >> 13)).wrapping_mul(2246822519);
    h ^= h >> 16;
    let roll = (h % 10_000) as f32 / 10_000.0; // [0, 1)

    // Triangular weighting per tier — each tier peaks at its
    // characteristic richness, decays linearly to zero ±1/3 away.
    // Peaks: Empty @ r=0, Thin @ r=1/3, Healthy @ r=2/3, Fat @ r=1.
    //
    // r=0   → weights [1, 0, 0, 0]    → 100% canonical PURE_DEEPSLATE
    // r=1/3 → weights [0, 1, 0, 0]    → 100% THIN
    // r=2/3 → weights [0, 0, 1, 0]    → 100% HEALTHY
    // r=1   → weights [0, 0, 0, 1]    → 100% FAT
    // Intermediate values produce smooth mixes between the two
    // bracketing tiers (e.g. r=0.5 gives ~50% THIN + ~50% HEALTHY).
    let variants = [
        block::PURE_DEEPSLATE,
        block::PURE_DEEPSLATE_THIN,
        block::PURE_DEEPSLATE_HEALTHY,
        block::PURE_DEEPSLATE_FAT,
    ];
    let mut weights = [0.0f32; 4];
    let mut total = 0.0;
    for (i, w) in weights.iter_mut().enumerate() {
        let peak_r = i as f32 / 3.0;
        *w = (1.0 - 3.0 * (r - peak_r).abs()).max(0.0);
        // Snap floating-point noise to 0 — f32 fp around `r * 3` and
        // `peak_r * 3` accumulates ~6e-8 error, which would leak a
        // few "wrong-tier" picks at exact richness extremes (r=0,
        // r=1/3, r=2/3, r=1). Snap below 1e-5 to keep the extremes
        // strictly monomodal.
        if *w < 1e-5 {
            *w = 0.0;
        }
        total += *w;
    }
    if total <= 0.0 {
        return block::PURE_DEEPSLATE; // defensive: never happens given r∈[0,1]
    }
    let pick = roll * total;
    let mut cumulative = 0.0;
    for i in 0..4 {
        cumulative += weights[i];
        if pick < cumulative {
            return variants[i];
        }
    }
    // Floating-point edge — pick exactly at total. Return Fat (the last bucket).
    block::PURE_DEEPSLATE_FAT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    #[test]
    fn ore_at_is_deterministic() {
        let g = BiomeGenerator::new(42);
        let a = g.ore_at(10, 20, -5);
        let b = g.ore_at(10, 20, -5);
        assert_eq!(a, b);
    }

    #[test]
    fn diamond_band_is_all_deepslate_never_plain_diamond_ore() {
        // Engine audit 2026-06-04, E: the diamond band (y<15) is entirely below
        // the deepslate threshold (y<22), so plain DIAMOND_ORE never generates.
        // Lock it: scan the band and assert deepslate-only.
        let g = BiomeGenerator::new(20260604);
        let mut found_deepslate_diamond = false;
        for y in 0..15 {
            for x in 0..400 {
                match g.ore_at(x, y, 0) {
                    Some(o) if o == block::DIAMOND_ORE => {
                        panic!("plain DIAMOND_ORE generated at ({x},{y},0) — band must be all-deepslate");
                    }
                    Some(o) if o == block::DEEPSLATE_DIAMOND_ORE => found_deepslate_diamond = true,
                    _ => {}
                }
            }
        }
        assert!(found_deepslate_diamond, "the diamond band should yield deepslate diamond");
    }

    #[test]
    fn diamond_only_below_y15() {
        let g = BiomeGenerator::new(99);
        // Above 15: scan a chunk's worth of positions, no diamond should spawn
        // (stone-tier or deepslate-tier).
        for x in 0..32 {
            for z in 0..32 {
                for y in 15..50 {
                    let v = g.ore_at(x, y, z);
                    assert_ne!(v, Some(block::DIAMOND_ORE),
                        "diamond at ({x},{y},{z}) should be impossible above y=15");
                    assert_ne!(v, Some(block::DEEPSLATE_DIAMOND_ORE),
                        "deepslate-diamond at ({x},{y},{z}) should be impossible above y=15");
                }
            }
        }
    }

    #[test]
    fn iron_only_below_y50() {
        let g = BiomeGenerator::new(7);
        for x in 0..32 {
            for z in 0..32 {
                for y in 50..100 {
                    let v = g.ore_at(x, y, z);
                    assert_ne!(v, Some(block::IRON_ORE),
                        "iron at ({x},{y},{z}) should be impossible above y=50");
                    assert_ne!(v, Some(block::DEEPSLATE_IRON_ORE),
                        "deepslate-iron at ({x},{y},{z}) should be impossible above y=50");
                }
            }
        }
    }

    #[test]
    fn deepslate_substrate_only_below_y_dp() {
        let g = BiomeGenerator::new(42);
        for x in 0..32 {
            for z in 0..32 {
                for y in Y_DP..(Y_DP + 20) {
                    assert!(!g.is_deepslate_substrate(x, y, z),
                        "expected stone substrate at ({x},{y},{z}), y >= Y_DP={Y_DP}");
                }
                // Well below transition zone — always deepslate.
                for y in 0..(Y_DP - DEEPSLATE_TRANSITION_DEPTH) {
                    assert!(g.is_deepslate_substrate(x, y, z),
                        "expected deepslate at ({x},{y},{z}), y < Y_DP - transition");
                }
            }
        }
    }

    #[test]
    fn deepslate_transition_zone_is_mixed() {
        // In the transition zone, sample a row and verify we see BOTH
        // stone and deepslate substrate. (Deterministic from seed.)
        let g = BiomeGenerator::new(123);
        let mut saw_stone = false;
        let mut saw_deepslate = false;
        let y = Y_DP - DEEPSLATE_TRANSITION_DEPTH / 2; // mid-transition
        for x in 0..64 {
            for z in 0..64 {
                if g.is_deepslate_substrate(x, y, z) {
                    saw_deepslate = true;
                } else {
                    saw_stone = true;
                }
                if saw_stone && saw_deepslate {
                    return;
                }
            }
        }
        panic!("transition zone at y={y} should contain both substrates (saw stone={saw_stone}, deepslate={saw_deepslate})");
    }

    #[test]
    fn ore_variant_matches_substrate() {
        // Sample ore positions and verify deepslate-variant matches
        // is_deepslate_substrate for the same position.
        let g = BiomeGenerator::new(55);
        for x in 0..32 {
            for z in 0..32 {
                for y in 0..50 {
                    let Some(ore) = g.ore_at(x, y, z) else { continue };
                    let in_deepslate = g.is_deepslate_substrate(x, y, z);
                    let is_deepslate_ore = matches!(
                        ore,
                        block::DEEPSLATE_COAL_ORE
                            | block::DEEPSLATE_IRON_ORE
                            | block::DEEPSLATE_DIAMOND_ORE
                    );
                    assert_eq!(in_deepslate, is_deepslate_ore,
                        "ore variant at ({x},{y},{z}): substrate-says-deepslate={in_deepslate} but ore-is-deepslate={is_deepslate_ore} (ore={ore:?})");
                }
            }
        }
    }

    #[test]
    fn base_rock_picks_pure_deepslate_when_substrate_is_deepslate() {
        let g = BiomeGenerator::new(99);
        // Sample a column from y=0 up to Y_DP+5 and verify base_rock
        // returns a PURE_DEEPSLATE-family member in the deepslate band
        // and STONE elsewhere. Spec 16 Phase 3b — the canonical and
        // its three visual variants all count.
        for y in 0..(Y_DP + 5) {
            let want_deepslate = g.is_deepslate_substrate(10, y, -3);
            let rock = g.base_rock_at(10, y, -3);
            if want_deepslate {
                assert!(
                    block::is_pure_deepslate_family(rock),
                    "expected pure-deepslate-family at y={y}, got {rock}",
                );
            } else {
                assert_eq!(rock, block::STONE, "expected stone at y={y}");
            }
        }
    }

    #[test]
    fn pick_deepslate_variant_at_zero_richness_returns_canonical() {
        // r=0 → every roll falls below the Empty threshold (1.0) →
        // canonical PURE_DEEPSLATE for every position.
        for x in 0..32 {
            for z in 0..32 {
                let v = pick_deepslate_variant(x, -25, z, 123, 0.0);
                assert_eq!(v, block::PURE_DEEPSLATE);
            }
        }
    }

    #[test]
    fn pick_deepslate_variant_at_one_richness_is_all_fat() {
        // r=1 → triangular weights are [0, 0, 0, 1] → every position
        // is FAT regardless of the hash.
        for x in 0..64 {
            for z in 0..64 {
                let v = pick_deepslate_variant(x, -25, z, 7, 1.0);
                assert_eq!(v, block::PURE_DEEPSLATE_FAT);
            }
        }
    }

    #[test]
    fn pick_deepslate_variant_at_half_richness_mixes_thin_and_healthy() {
        // r=0.5 sits between Thin (peak at 1/3) and Healthy (peak at
        // 2/3). Their weights at r=0.5 are 0.5 and 0.5; Empty + Fat
        // weights are 0. So the entire grid splits between THIN +
        // HEALTHY with no canonical / no FAT.
        let mut counts = [0; 4];
        for x in 0..64 {
            for z in 0..64 {
                match pick_deepslate_variant(x, -25, z, 7, 0.5) {
                    block::PURE_DEEPSLATE => counts[0] += 1,
                    block::PURE_DEEPSLATE_THIN => counts[1] += 1,
                    block::PURE_DEEPSLATE_HEALTHY => counts[2] += 1,
                    block::PURE_DEEPSLATE_FAT => counts[3] += 1,
                    _ => panic!("unexpected variant"),
                }
            }
        }
        assert_eq!(counts[0], 0, "Empty should not appear at r=0.5");
        assert_eq!(counts[3], 0, "Fat should not appear at r=0.5");
        assert!(counts[1] > 0 && counts[2] > 0, "expected mix at r=0.5, got {counts:?}");
    }

    #[test]
    fn pick_deepslate_variant_is_deterministic_per_position() {
        // Spec 16 Phase 3b — same (x, y, z, seed, richness) must
        // produce the same variant. Locks the "doesn't flicker on
        // chunk reload" invariant.
        let a = pick_deepslate_variant(5, -25, 12, 42, 0.6);
        let b = pick_deepslate_variant(5, -25, 12, 42, 0.6);
        assert_eq!(a, b);
    }

    #[test]
    fn coal_appears_at_some_density() {
        let g = BiomeGenerator::new(123);
        let mut coal = 0;
        let mut total = 0;
        for x in 0..50 {
            for y in 1..70 {
                for z in 0..50 {
                    total += 1;
                    if g.ore_at(x, y, z) == Some(block::COAL_ORE) {
                        coal += 1;
                    }
                }
            }
        }
        // Expect ~6% coal per the formula (less because higher-priority
        // diamond/iron eat some). Don't pin too tight; just sanity-check.
        let pct = coal * 100 / total;
        assert!(pct >= 3 && pct <= 9,
            "coal should be ~6% of stone (got {pct}%, {coal}/{total})");
    }

    // Spec 49 (Explosives) — Brimstone + Nitre ore placement.

    #[test]
    fn brimstone_appears_in_deep_stone_band() {
        let g = BiomeGenerator::new(4949);
        let mut found = false;
        'scan: for x in 0..80 {
            for y in Y_DP..48 {
                for z in 0..80 {
                    if g.ore_at(x, y, z) == Some(block::BRIMSTONE) {
                        found = true;
                        break 'scan;
                    }
                }
            }
        }
        assert!(found, "expected at least one BRIMSTONE in the Y_DP..48 stone band");
    }

    #[test]
    fn nitre_appears_underground() {
        let g = BiomeGenerator::new(4950);
        let mut found = false;
        'scan: for x in 0..80 {
            for y in Y_DP..SEA_LEVEL {
                for z in 0..80 {
                    if g.ore_at(x, y, z) == Some(block::NITRE_ORE) {
                        found = true;
                        break 'scan;
                    }
                }
            }
        }
        assert!(found, "expected at least one NITRE_ORE in the Y_DP..SEA_LEVEL band");
    }

    #[test]
    fn brimstone_and_nitre_are_stone_only_never_in_deepslate() {
        // The two new ores have no deepslate variant, so they must never
        // generate in the deepslate substrate (keeps `ore_variant_matches_substrate`
        // and the demolition-vs-mining invariants honest).
        let g = BiomeGenerator::new(4951);
        for x in 0..40 {
            for y in 0..50 {
                for z in 0..40 {
                    let ore = g.ore_at(x, y, z);
                    if ore == Some(block::BRIMSTONE) || ore == Some(block::NITRE_ORE) {
                        assert!(
                            !g.is_deepslate_substrate(x, y, z),
                            "Brimstone/Nitre placed in deepslate at ({x},{y},{z})"
                        );
                    }
                }
            }
        }
    }

    // Wind, Copper & Electricity wave (2026-09-07) — Copper Ore placement,
    // Spec 02 §5.3.1a. Mirrors the Brimstone/Nitre test shape above.

    #[test]
    fn copper_appears_in_mid_band() {
        let g = BiomeGenerator::new(4902);
        let mut found = false;
        'scan: for x in 0..80 {
            for y in Y_DP..(SEA_LEVEL + 8) {
                for z in 0..80 {
                    if g.ore_at(x, y, z) == Some(block::COPPER_ORE) {
                        found = true;
                        break 'scan;
                    }
                }
            }
        }
        assert!(found, "expected at least one COPPER_ORE in the Y_DP..(SEA_LEVEL+8) band");
    }

    #[test]
    fn copper_only_inside_band() {
        // No copper below Y_DP (deepslate territory — copper has no deepslate
        // variant) and none above SEA_LEVEL + 8 (above the mid-depth band).
        let g = BiomeGenerator::new(4903);
        for x in 0..40 {
            for z in 0..40 {
                for y in 0..Y_DP {
                    assert_ne!(
                        g.ore_at(x, y, z),
                        Some(block::COPPER_ORE),
                        "COPPER_ORE generated below Y_DP at ({x},{y},{z})"
                    );
                }
                for y in (SEA_LEVEL + 8)..120 {
                    assert_ne!(
                        g.ore_at(x, y, z),
                        Some(block::COPPER_ORE),
                        "COPPER_ORE generated above SEA_LEVEL+8 at ({x},{y},{z})"
                    );
                }
            }
        }
    }

    #[test]
    fn copper_is_stone_only_never_in_deepslate() {
        // Copper has no deepslate variant (like Magnesium/Brimstone/Nitre), so
        // it must never generate where the substrate resolves to deepslate.
        // The Copper band (Y_DP..SEA_LEVEL+8) sits entirely at/above Y_DP, where
        // `is_deepslate_substrate` is always false, but lock the invariant
        // directly so a future band-widening trips this test first.
        let g = BiomeGenerator::new(4904);
        for x in 0..40 {
            for y in 0..(SEA_LEVEL + 8) {
                for z in 0..40 {
                    if g.ore_at(x, y, z) == Some(block::COPPER_ORE) {
                        assert!(
                            !g.is_deepslate_substrate(x, y, z),
                            "Copper Ore placed in deepslate at ({x},{y},{z})"
                        );
                    }
                }
            }
        }
    }

    // Salt feature (2026-05-23) — ROCK_SALT vein placement.

    /// Walk a big xz range looking for a (wx, wz) classified as one of
    /// the given biomes. Returns the first hit or None across the
    /// search radius.
    fn find_biome_column(
        g: &BiomeGenerator,
        biomes: &[Biome],
        radius: i32,
    ) -> Option<(i32, i32)> {
        for cx in -radius..=radius {
            for cz in -radius..=radius {
                let wx = cx * 16;
                let wz = cz * 16;
                let b = g.biome_at(wx, wz);
                if biomes.contains(&b) {
                    return Some((wx, wz));
                }
            }
        }
        None
    }

    #[test]
    fn rock_salt_appears_in_mountains_or_desert_band() {
        let g = BiomeGenerator::new(123);
        // Search a wide radius: post-Spec-28a-wire-up, Mountains are
        // elevation-driven (coarse continentalness, ~666-block period),
        // so a small window near origin can be entirely mid-elevation
        // land. ±128 chunks spans several continental features.
        let (wx, wz) = find_biome_column(&g, &[Biome::Mountains, Biome::Desert], 128)
            .expect("expected at least one Mountains or Desert column in -128..128 chunks");
        let mut saw_rock_salt = false;
        for dx in 0..16 {
            for dz in 0..16 {
                let x = wx + dx;
                let z = wz + dz;
                if !matches!(g.biome_at(x, z), Biome::Mountains | Biome::Desert) {
                    continue;
                }
                let surface = g.terrain_height(x, z);
                for y in 60..=surface {
                    if g.base_rock_at(x, y, z) == block::ROCK_SALT {
                        saw_rock_salt = true;
                        break;
                    }
                }
                if saw_rock_salt { break; }
            }
            if saw_rock_salt { break; }
        }
        assert!(saw_rock_salt,
            "expected at least one ROCK_SALT in a 16x16 mountain/desert sweep");
    }

    #[test]
    fn rock_salt_does_not_appear_in_plains() {
        let g = BiomeGenerator::new(123);
        let (wx, wz) = find_biome_column(&g, &[Biome::Plains], 32)
            .expect("expected at least one Plains column in -32..32");
        for dx in 0..16 {
            for dz in 0..16 {
                let x = wx + dx;
                let z = wz + dz;
                if !matches!(g.biome_at(x, z), Biome::Plains) {
                    continue;
                }
                let surface = g.terrain_height(x, z);
                for y in 60..=surface {
                    assert_ne!(g.base_rock_at(x, y, z), block::ROCK_SALT,
                        "ROCK_SALT must not appear in Plains at ({x},{y},{z})");
                }
            }
        }
    }

    #[test]
    fn all_ten_biomes_generate_from_biome_at() {
        // Spec 28a wire-up regression (2026-05-27): before this, the
        // legacy biome_at only ever returned 6 of the 10 biomes —
        // Savanna/Taiga/BirchForest/SnowyTundra never generated, so the
        // mobs gated to them (Nostrich, etc.) had no home. Sample a wide
        // grid and confirm every biome — the 8 climate biomes plus the
        // two terrain-shape biomes — is now reachable.
        use std::collections::HashSet;
        let g = BiomeGenerator::new(42);
        let mut seen: HashSet<Biome> = HashSet::new();
        for x in (-4000..=4000).step_by(32) {
            for z in (-4000..=4000).step_by(32) {
                seen.insert(g.biome_at(x, z));
            }
        }
        for b in [
            Biome::Plains, Biome::Forest, Biome::BirchForest, Biome::Taiga,
            Biome::Jungle, Biome::Savanna, Biome::Desert, Biome::SnowyTundra,
            Biome::Mountains, Biome::Ocean,
        ] {
            assert!(seen.contains(&b), "biome {:?} never generated by biome_at", b);
        }
    }

    #[test]
    fn every_biome_has_a_display_name() {
        // The #44 debug HUD reads Biome::name() — every variant must map to a
        // non-empty, human-readable label (UK English, multi-word spaced).
        for b in [
            Biome::Plains, Biome::Forest, Biome::Desert, Biome::Mountains,
            Biome::Ocean, Biome::BirchForest, Biome::Taiga, Biome::Jungle,
            Biome::Savanna, Biome::SnowyTundra,
        ] {
            assert!(!b.name().is_empty(), "biome {:?} has empty name", b);
        }
        assert_eq!(Biome::BirchForest.name(), "Birch Forest");
        assert_eq!(Biome::SnowyTundra.name(), "Snowy Tundra");
    }
}

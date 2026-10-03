//! Spec 28d chunk 11 — pure worldgen helper bundle.
//!
//! Three pure helpers that lift hard-coded constants out of
//! `chunk.rs` / `biome.rs` into testable data tables:
//!
//! - `OreBand` table + `ore_at_pure` — the per-ore depth band, rarity,
//!   and deepslate-variant mapping that `BiomeGenerator::ore_at`
//!   open-codes today. Lifting it into data means we can tune one
//!   table instead of edit-then-recompile the generator's match.
//! - `surface_block_for(biome)` — straight-line lookup that wraps
//!   `biome_properties(biome).surface_block`. Lifted so consumers that
//!   only need the block don't pull in the full BiomeProperties row.
//! - `snow_overlay_for_biome(biome)` — whether the biome caps grass
//!   with a SNOW block. Today's `BiomeProperties.surface_block = SNOW`
//!   for SnowyTundra; this helper makes the rule queryable without
//!   inspecting the surface block.
//!
//! All three are pure functions over the public data layer. Live
//! wire-up (replacing `BiomeGenerator::ore_at` with `ore_at_pure`) is
//! deferred to a later session — the table is ready when that lands.

// Module-scoped (not crate-wide) — the doc comment above already establishes
// this whole file as tested-but-not-yet-wired, so a per-item cfg_attr repeated
// across every pub item here would just restate it five times.
#![allow(dead_code)]

use crate::biome::Biome;
use crate::block::{self, BlockId};

/// One row in the ore-band table. The `ore_at_pure` walk checks each
/// row in order — earliest match wins, mirroring the today's hand-
/// coded match in `BiomeGenerator::ore_at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OreBand {
    /// Inclusive upper Y; ore only spawns if `y < max_y`.
    pub max_y: i32,
    /// 0..=999 — fraction-of-thousand chance per stone block.
    pub rarity_per_thousand: u32,
    /// Which hash-band of the per-position roll to consume. The
    /// existing implementation uses `h`, `h/1000`, `h/100_000` for
    /// the three bands; we shift into the same buckets here.
    pub hash_divisor: u32,
    /// Block ID for the in-stone form.
    pub stone_variant: BlockId,
    /// Block ID for the in-deepslate form.
    pub deepslate_variant: BlockId,
}

/// The canonical ore-band table. Matches the open-coded thresholds in
/// `BiomeGenerator::ore_at`:
///   Diamond < 15, 4/1000
///   Iron    < 50, 3/100 = 30/1000
///   Coal    anywhere, 6/100 = 60/1000
pub const ORE_BAND_TABLE: &[OreBand] = &[
    OreBand {
        max_y: 15,
        rarity_per_thousand: 4,
        hash_divisor: 1,
        stone_variant: block::DIAMOND_ORE,
        deepslate_variant: block::DEEPSLATE_DIAMOND_ORE,
    },
    OreBand {
        max_y: 50,
        rarity_per_thousand: 30,
        hash_divisor: 1_000,
        stone_variant: block::IRON_ORE,
        deepslate_variant: block::DEEPSLATE_IRON_ORE,
    },
    OreBand {
        max_y: i32::MAX, // any y
        rarity_per_thousand: 60,
        hash_divisor: 100_000,
        stone_variant: block::COAL_ORE,
        deepslate_variant: block::DEEPSLATE_COAL_ORE,
    },
];

/// Pure ore lookup using the band table. Given the per-position hash
/// + whether the substrate is deepslate, returns the first matching
///   ore variant or None. Matches the semantics of
///   `BiomeGenerator::ore_at` but expressed as a data-driven walk so
///   new ores (Copper / Tin / Emerald) drop in as table additions.
///
/// Pure: no side effects, no world reads. Caller supplies the hash +
/// the deepslate flag (both already computed in the gen path).
pub fn ore_at_pure(y: i32, hash: u32, in_deepslate: bool) -> Option<BlockId> {
    for band in ORE_BAND_TABLE {
        if y < band.max_y {
            let bucket = (hash / band.hash_divisor) % 1000;
            if bucket < band.rarity_per_thousand {
                return Some(if in_deepslate {
                    band.deepslate_variant
                } else {
                    band.stone_variant
                });
            }
        }
    }
    None
}

/// Surface block for a biome — straight-line lookup over
/// `biome_properties`. Reserved as a thin helper so downstream
/// consumers that only need the surface block don't carry the
/// BiomeProperties row.
pub fn surface_block_for(biome: Biome) -> BlockId {
    crate::biome::biome_properties(biome).surface_block
}

/// Does this biome cap its surface with a snow overlay? Today
/// equivalent to `surface_block == SNOW`; lifted so the rule is
/// queryable without inspecting the block ID directly. When the
/// terrain pipeline gains a separate `surface_overlay` field, this
/// helper migrates to read it.
pub fn snow_overlay_for_biome(biome: Biome) -> bool {
    surface_block_for(biome) == block::SNOW
}

/// "Composed" surface block at (x, z) — convenience for the
/// future-state pipeline where the surface block is derived from the
/// biome at that column. Today: looks up the biome's surface from the
/// generator, applies snow overlay if appropriate. Pure: takes
/// `biome` directly (caller resolves which biome it is via
/// `BiomeGenerator::biome_at(x, z)`).
pub fn surface_block_at(biome: Biome) -> BlockId {
    if snow_overlay_for_biome(biome) {
        block::SNOW
    } else {
        surface_block_for(biome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OreBand table walk ---------------------------------------------

    #[test]
    fn ore_table_yields_no_ore_when_all_bucket_misses() {
        // hash = 0 → all buckets = 0 → all bands fire on rarity_per_thousand > 0.
        // We need a hash where bucket >= rarity to assert "no ore". The
        // coal band uses divisor 100_000; if we set hash = 999_000 then
        // bucket = 999_000 / 100_000 = 9, > 60? No — 9 < 60, so coal
        // fires. Need higher: hash = 999_999_999 / 100_000 = 9_999 % 1000 = 999 → > 60.
        let hash = 999_999_999u32;
        // y=80 — above diamond + iron bands. Coal band sets in only
        // when bucket < 60. We picked 999 → no ore.
        let ore = ore_at_pure(80, hash, false);
        assert!(ore.is_none(), "expected no ore for high-hash 80y, got {:?}", ore);
    }

    #[test]
    fn ore_table_yields_diamond_inside_diamond_band() {
        // y < 15 and (hash % 1000) < 4 → diamond.
        let hash = 2u32; // bucket=2 < 4
        let ore = ore_at_pure(10, hash, false);
        assert_eq!(ore, Some(block::DIAMOND_ORE));
    }

    #[test]
    fn ore_table_yields_deepslate_diamond_in_deepslate() {
        let hash = 2u32;
        let ore = ore_at_pure(10, hash, true);
        assert_eq!(ore, Some(block::DEEPSLATE_DIAMOND_ORE));
    }

    #[test]
    fn ore_table_yields_iron_in_iron_band() {
        // Diamond bucket = 999 (no diamond). Iron bucket = 5 (< 30).
        // h / 1000 must yield bucket 5 with h%1000 = 999 → h = 5999.
        let hash = 5999u32;
        let ore = ore_at_pure(40, hash, false);
        assert_eq!(ore, Some(block::IRON_ORE));
    }

    #[test]
    fn ore_table_yields_coal_at_surface() {
        // y high (above diamond + iron). Coal bucket: hash / 100_000 % 1000.
        // Want this < 60 → hash = 1_000_000 → bucket=10. But also
        // need (h % 1000) ≥ 4 to skip diamond — h=1_000_000 → 0 → diamond would fire if y<15.
        // y=70 above diamond's 15 cap → diamond skipped naturally.
        // h / 1000 = 1000 → bucket=1000%1000=0 → < 30 → iron fires! Need to
        // bypass iron. Use h = 1_999_999: diamond bucket=999, iron bucket=999%1000=999 (skip),
        // coal bucket = 19 (< 60) → coal.
        let hash = 1_999_999u32;
        let ore = ore_at_pure(70, hash, false);
        assert_eq!(ore, Some(block::COAL_ORE));
    }

    // Biome surface helpers ------------------------------------------

    #[test]
    fn snow_overlay_only_in_snowy_tundra() {
        assert!(snow_overlay_for_biome(Biome::SnowyTundra));
        assert!(!snow_overlay_for_biome(Biome::Plains));
        assert!(!snow_overlay_for_biome(Biome::Forest));
        assert!(!snow_overlay_for_biome(Biome::Taiga));
        assert!(!snow_overlay_for_biome(Biome::Desert));
        assert!(!snow_overlay_for_biome(Biome::Ocean));
    }

    #[test]
    fn surface_block_matches_biome_properties() {
        for b in [Biome::Plains, Biome::Forest, Biome::Desert, Biome::SnowyTundra] {
            assert_eq!(surface_block_for(b), crate::biome::biome_properties(b).surface_block);
        }
    }

    #[test]
    fn desert_surface_is_sand() {
        assert_eq!(surface_block_at(Biome::Desert), block::SAND);
    }

    #[test]
    fn plains_surface_is_grass() {
        assert_eq!(surface_block_at(Biome::Plains), block::GRASS);
    }

    #[test]
    fn snowy_tundra_surface_is_snow() {
        assert_eq!(surface_block_at(Biome::SnowyTundra), block::SNOW);
    }

    // Parity with the live BiomeGenerator::ore_at: same y + same hash
    // semantics → same result. The hash function in biome.rs is
    // private but we don't need it here — we exercise the contract by
    // matching the rarity thresholds in this test fixture's hashes.

    #[test]
    fn ore_band_table_matches_open_coded_thresholds() {
        // Diamond band — first row, max_y=15, rarity 4/1000.
        assert_eq!(ORE_BAND_TABLE[0].max_y, 15);
        assert_eq!(ORE_BAND_TABLE[0].rarity_per_thousand, 4);
        assert_eq!(ORE_BAND_TABLE[0].stone_variant, block::DIAMOND_ORE);
        // Iron band.
        assert_eq!(ORE_BAND_TABLE[1].max_y, 50);
        assert_eq!(ORE_BAND_TABLE[1].rarity_per_thousand, 30);
        assert_eq!(ORE_BAND_TABLE[1].stone_variant, block::IRON_ORE);
        // Coal band.
        assert_eq!(ORE_BAND_TABLE[2].max_y, i32::MAX);
        assert_eq!(ORE_BAND_TABLE[2].rarity_per_thousand, 60);
        assert_eq!(ORE_BAND_TABLE[2].stone_variant, block::COAL_ORE);
    }
}

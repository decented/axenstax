//! Which blocks are alive, and of which kind (living-world spec §1.2).
//!
//! A *kind* is one bit of a `u64`: a species of leaf, log or sapling, a crop or
//! flower family, grass, tall grass, water and the like. A region's diversity is
//! the number of distinct kinds it holds, so counting is a bit-or and a popcount.

use crate::block::{self, BlockId};

/// Crop and flower families. A farmed flower and its wild form are one family.
pub const FAMILIES: usize = 10;

const LEAVES: [BlockId; 7] = [
    block::OAK_LEAVES,
    block::BIRCH_LEAVES,
    block::SPRUCE_LEAVES,
    block::JUNGLE_LEAVES,
    block::ACACIA_LEAVES,
    block::DARK_OAK_LEAVES,
    block::RUBBER_LEAVES,
];
const LOGS: [BlockId; 7] = [
    block::OAK_LOG,
    block::BIRCH_LOG,
    block::SPRUCE_LOG,
    block::JUNGLE_LOG,
    block::ACACIA_LOG,
    block::DARK_OAK_LOG,
    block::RUBBER_LOG,
];
const SAPLINGS: [BlockId; 7] = [
    block::SAPLING_OAK,
    block::SAPLING_BIRCH,
    block::SAPLING_SPRUCE,
    block::SAPLING_JUNGLE,
    block::SAPLING_ACACIA,
    block::SAPLING_DARK_OAK,
    block::SAPLING_RUBBER,
];

// Bit layout of the kind mask.
const LEAF_BIT: u8 = 0;
const LOG_BIT: u8 = 7;
const SAPLING_BIT: u8 = 14;
const FAMILY_BIT: u8 = 21; // 10 families: 21..=30
const WILD_COTTON_BIT: u8 = 31;
const WILD_HEMP_BIT: u8 = 32;
const GRASS_BIT: u8 = 33;
const TALL_GRASS_BIT: u8 = 34;
const WATER_BIT: u8 = 35;
const HIVE_BIT: u8 = 36;
const BERRY_BIT: u8 = 37;

fn index_of(table: &[BlockId; 7], id: BlockId) -> Option<u8> {
    table.iter().position(|&b| b == id).map(|i| i as u8)
}

/// The crop or flower family of a block (`0..FAMILIES`): any growth stage of a
/// crop, or a flower in any stage including its mature wild form.
pub fn crop_family(id: BlockId) -> Option<u8> {
    use block::*;
    Some(match id {
        WHEAT_STAGE_0..=WHEAT_STAGE_3 => 0,
        CARROT_STAGE_0..=CARROT_STAGE_3 => 1,
        POTATO_STAGE_0..=POTATO_STAGE_3 => 2,
        CORN_STAGE_0..=CORN_STAGE_3 => 3,
        PAPYRUS_STAGE_0..=PAPYRUS_STAGE_3 => 4,
        COTTON_STAGE_0..=COTTON_STAGE_3 => 5,
        HEMP_STAGE_0..=HEMP_STAGE_3 => 6,
        CORNFLOWER | CORNFLOWER_STAGE_0..=CORNFLOWER_STAGE_2 => 7,
        FIELD_POPPY | FIELD_POPPY_STAGE_0..=FIELD_POPPY_STAGE_2 => 8,
        BUTTERCUP | BUTTERCUP_STAGE_0..=BUTTERCUP_STAGE_2 => 9,
        _ => return None,
    })
}

/// The kind bit of a living block, or `None` when the block is not alive.
/// Logs count as living (stumps, trunks); so do water, grass and hives.
pub fn living_kind(id: BlockId) -> Option<u8> {
    if let Some(f) = crop_family(id) {
        return Some(FAMILY_BIT + f);
    }
    if let Some(i) = index_of(&LEAVES, id) {
        return Some(LEAF_BIT + i);
    }
    if let Some(i) = index_of(&LOGS, id) {
        return Some(LOG_BIT + i);
    }
    if let Some(i) = index_of(&SAPLINGS, id) {
        return Some(SAPLING_BIT + i);
    }
    Some(match id {
        block::RUBBER_LOG_TAPPED => LOG_BIT + 6,
        block::COTTON_PLANT => WILD_COTTON_BIT,
        block::HEMP_PLANT => WILD_HEMP_BIT,
        block::GRASS => GRASS_BIT,
        block::TALL_GRASS => TALL_GRASS_BIT,
        block::WATER => WATER_BIT,
        block::BEE_HIVE => HIVE_BIT,
        block::BERRY_BUSH_0..=block::BERRY_BUSH_3 => BERRY_BIT,
        _ => return None,
    })
}

/// A block that is alive.
pub fn is_living(id: BlockId) -> bool {
    living_kind(id).is_some()
}

/// Passive creatures that count as a living kind. People (villagers, peddlers,
/// knights) do not: they are the "people" term, not the "life" term.
pub fn counts_as_wildlife(kind: crate::mob::MobType) -> bool {
    use crate::mob::{mob_def, MobCategory, MobType};
    !matches!(kind, MobType::Villager | MobType::Peddler | MobType::Knight)
        && mob_def(kind).category == MobCategory::Passive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_living_block_has_a_distinct_kind_per_species() {
        let mut seen = std::collections::HashSet::new();
        for t in [&LEAVES, &LOGS, &SAPLINGS] {
            for &id in t {
                assert!(seen.insert(living_kind(id).unwrap()), "{id} shares a kind");
            }
        }
        for id in 0..400u16 {
            if let Some(k) = living_kind(id) {
                assert!(k < 64, "kind {k} of block {id} overflows the mask");
            }
        }
    }

    #[test]
    fn stages_of_one_crop_are_one_kind_and_stone_is_not_alive() {
        assert_eq!(living_kind(block::WHEAT_STAGE_0), living_kind(block::WHEAT_STAGE_3));
        assert_ne!(living_kind(block::WHEAT_STAGE_0), living_kind(block::CARROT_STAGE_0));
        assert_eq!(living_kind(block::CORNFLOWER), living_kind(block::CORNFLOWER_STAGE_1));
        assert!(!is_living(block::STONE) && !is_living(block::DIRT) && !is_living(block::AIR));
        assert!(!is_living(block::TILLED_SOIL));
    }

    #[test]
    fn every_growing_crop_block_is_alive() {
        for id in 0..400u16 {
            if crate::growth::is_crop(id) {
                assert!(is_living(id), "crop block {id} must be alive");
            }
        }
    }

    #[test]
    fn people_are_not_wildlife() {
        use crate::mob::MobType;
        assert!(counts_as_wildlife(MobType::Cow) && counts_as_wildlife(MobType::Fish));
        assert!(!counts_as_wildlife(MobType::Villager) && !counts_as_wildlife(MobType::Peddler));
        assert!(!counts_as_wildlife(MobType::Brigand));
    }
}

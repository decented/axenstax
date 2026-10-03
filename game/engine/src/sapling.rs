//! Spec 28b — sapling planting predicate.
//!
//! Pure: given a position + world, returns whether a sapling material
//! can be planted there. Live planting (right-click handler that
//! consumes the sapling + places a SAPLING_STAGE_0 block) is deferred
//! to the live tree-gen wave.
//!
//! Rules (matches Minecraft baseline):
//! - The block at the target position must be AIR.
//! - The block directly below must be GRASS or DIRT.
//! - Sufficient light (deferred — light grid not yet readable here).
//! - Enough vertical clearance for the eventual tree to grow into
//!   (deferred — needs species-aware lookup, which the future live
//!   planter will check).
//!
//! Spec: `docs/foundations/2026-05-20-wood-species-multi-tree.md` §9.

use crate::block::{self, BlockId};
use crate::world::World;

/// Can a sapling material be planted at this position?
///
/// Pure: reads world block lookups, doesn't mutate.
pub fn can_plant_sapling_at(x: i32, y: i32, z: i32, world: &World) -> bool {
    // Target cell must be empty so the sprouted sapling block has
    // somewhere to go.
    if world.get_block(x, y, z) != block::AIR {
        return false;
    }
    // Block below must be plantable soil (grass or dirt). Tilled
    // soil is reserved for crops; saplings prefer the wild surface.
    let below = world.get_block(x, y - 1, z);
    matches!(below, b if b == block::GRASS || b == block::DIRT)
}

/// Convenience: is this BlockId a sapling-plantable surface? No production
/// caller — `can_plant_sapling_at` above inlines the same grass/dirt match
/// rather than delegating here; exercised by the tests below.
#[cfg_attr(not(test), allow(dead_code))]
pub fn is_plantable_surface(id: BlockId) -> bool {
    id == block::GRASS || id == block::DIRT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    fn world_with_block_at(x: i32, y: i32, z: i32, id: BlockId) -> World {
        let mut w = World::new();
        w.set_block(x, y, z, id);
        w
    }

    #[test]
    fn plant_on_grass_succeeds() {
        let w = world_with_block_at(0, 60, 0, block::GRASS);
        // AIR at y=61 above grass at y=60.
        assert!(can_plant_sapling_at(0, 61, 0, &w));
    }

    #[test]
    fn plant_on_dirt_succeeds() {
        let w = world_with_block_at(0, 60, 0, block::DIRT);
        assert!(can_plant_sapling_at(0, 61, 0, &w));
    }

    #[test]
    fn plant_on_stone_fails() {
        let w = world_with_block_at(0, 60, 0, block::STONE);
        assert!(!can_plant_sapling_at(0, 61, 0, &w));
    }

    #[test]
    fn plant_on_sand_fails_until_we_add_jungle_sand_rule() {
        let w = world_with_block_at(0, 60, 0, block::SAND);
        // Conservative — alpha rejects sand. Future jungle/bamboo
        // saplings might accept sand; not in scope today.
        assert!(!can_plant_sapling_at(0, 61, 0, &w));
    }

    #[test]
    fn plant_into_non_air_target_fails() {
        let mut w = World::new();
        w.set_block(0, 60, 0, block::GRASS);
        w.set_block(0, 61, 0, block::STONE); // target blocked
        assert!(!can_plant_sapling_at(0, 61, 0, &w));
    }

    #[test]
    fn plant_at_void_below_fails() {
        // No block at all below — world::get_block returns AIR.
        let w = World::new();
        assert!(!can_plant_sapling_at(0, 61, 0, &w));
    }

    #[test]
    fn is_plantable_surface_recognises_grass_and_dirt() {
        assert!(is_plantable_surface(block::GRASS));
        assert!(is_plantable_surface(block::DIRT));
        assert!(!is_plantable_surface(block::STONE));
        assert!(!is_plantable_surface(block::SAND));
        assert!(!is_plantable_surface(block::WATER));
    }

    #[test]
    fn plant_on_tilled_soil_fails() {
        // Tilled soil is reserved for crops; saplings can't plant on
        // it (the player would be wasting tilled soil they'd dug for
        // wheat).
        let w = world_with_block_at(0, 60, 0, block::TILLED_SOIL);
        assert!(!can_plant_sapling_at(0, 61, 0, &w));
    }
}

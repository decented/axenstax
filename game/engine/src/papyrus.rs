//! Papyrus Reed — water-adjacent placement predicates.
//!
//! Per Spec 23 / `docs/foundations/2026-05-19-papyrus-reed.md`. Three
//! pure free functions matching the `growth.rs` precedent so the
//! right-click placement handler and the tests share one source of
//! truth. The water-adjacency rule is the entire placement gate —
//! papyrus is "wild" (no tilled-soil prerequisite), so finding water
//! is the meaningful first-day discovery moment.

use crate::block;
use crate::world::World;

/// Is there `block::WATER` within 1 block of `(x, y, z)`? Checks the
/// four horizontal neighbours at the same Y plus the cell directly
/// below (riverbed water under sand). Tight on purpose — papyrus
/// shouldn't sprout from a tile two squares away from a river.
pub fn is_water_adjacent(world: &World, x: i32, y: i32, z: i32) -> bool {
    for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        if world.get_block(x + dx, y, z + dz) == block::WATER {
            return true;
        }
    }
    world.get_block(x, y - 1, z) == block::WATER
}

/// Is `(x, y, z)` a valid base for planting a papyrus stem? Three
/// conjunctive conditions:
///   1. Block at (x, y, z) is one of DIRT / GRASS / SAND.
///   2. Block directly above is AIR (somewhere to grow into).
///   3. Water touches the base tile (see [`is_water_adjacent`]).
pub fn is_valid_planting_base(world: &World, x: i32, y: i32, z: i32) -> bool {
    let base = world.get_block(x, y, z);
    let base_ok = matches!(base, block::DIRT | block::GRASS | block::SAND);
    base_ok
        && world.get_block(x, y + 1, z) == block::AIR
        && is_water_adjacent(world, x, y, z)
}

/// Attempt to plant a papyrus stem in the air block directly above
/// `(x, y, z)`. Returns `true` if planted; `false` if any rule failed.
/// Idempotent on failure — leaves the world untouched.
/// The live right-click-with-reed handler (game_loop.rs) inlines this same
/// `place_player_block(..., PAPYRUS_STAGE_0)` call directly rather than going
/// through this helper — tested directly here.
#[cfg_attr(not(test), allow(dead_code))]
pub fn try_plant_papyrus(world: &mut World, x: i32, y: i32, z: i32) -> bool {
    if !is_valid_planting_base(world, x, y, z) {
        return false;
    }
    world.set_block(x, y + 1, z, block::PAPYRUS_STAGE_0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_world_with_water_at(x: i32, y: i32, z: i32) -> World {
        let mut w = World::new();
        w.set_block(x, y, z, block::WATER);
        w
    }

    #[test]
    fn is_water_adjacent_detects_horizontal_neighbour_north() {
        let world = fresh_world_with_water_at(0, 64, 1);
        assert!(is_water_adjacent(&world, 0, 64, 0));
    }

    #[test]
    fn is_water_adjacent_detects_horizontal_neighbour_south() {
        let world = fresh_world_with_water_at(0, 64, -1);
        assert!(is_water_adjacent(&world, 0, 64, 0));
    }

    #[test]
    fn is_water_adjacent_detects_horizontal_neighbour_east() {
        let world = fresh_world_with_water_at(1, 64, 0);
        assert!(is_water_adjacent(&world, 0, 64, 0));
    }

    #[test]
    fn is_water_adjacent_detects_horizontal_neighbour_west() {
        let world = fresh_world_with_water_at(-1, 64, 0);
        assert!(is_water_adjacent(&world, 0, 64, 0));
    }

    #[test]
    fn is_water_adjacent_detects_below_tile() {
        // Water at (x, y-1, z) — riverbed seeping up.
        let world = fresh_world_with_water_at(0, 63, 0);
        assert!(is_water_adjacent(&world, 0, 64, 0));
    }

    #[test]
    fn is_water_adjacent_false_for_dry_tile() {
        let world = World::new();
        // No water anywhere — fresh world default.
        assert!(!is_water_adjacent(&world, 0, 64, 0));
    }

    #[test]
    fn is_water_adjacent_does_not_check_y_plus_1() {
        // Water above the tile doesn't count — papyrus grows up, not down.
        // (Also: a tile with water directly above isn't a sensible planting
        // base; the block above must be AIR per is_valid_planting_base.)
        let world = fresh_world_with_water_at(0, 65, 0);
        assert!(!is_water_adjacent(&world, 0, 64, 0));
    }

    #[test]
    fn is_water_adjacent_does_not_reach_diagonal_corners() {
        // Diagonal (dx=1, dz=1) is Chebyshev-1 but NOT within the
        // 4-orthogonal-neighbour set this function checks. Papyrus
        // demands a 4-connected water tile, not 8-connected — keeps
        // the placement rule tight.
        let world = fresh_world_with_water_at(1, 64, 1);
        assert!(!is_water_adjacent(&world, 0, 64, 0));
    }

    #[test]
    fn is_valid_planting_base_accepts_dirt_with_water_and_air_above() {
        let mut world = fresh_world_with_water_at(1, 64, 0);
        world.set_block(0, 64, 0, block::DIRT);
        // Block above (y=65) is AIR by default.
        assert!(is_valid_planting_base(&world, 0, 64, 0));
    }

    #[test]
    fn is_valid_planting_base_accepts_grass() {
        let mut world = fresh_world_with_water_at(1, 64, 0);
        world.set_block(0, 64, 0, block::GRASS);
        assert!(is_valid_planting_base(&world, 0, 64, 0));
    }

    #[test]
    fn is_valid_planting_base_accepts_sand() {
        let mut world = fresh_world_with_water_at(1, 64, 0);
        world.set_block(0, 64, 0, block::SAND);
        assert!(is_valid_planting_base(&world, 0, 64, 0));
    }

    #[test]
    fn is_valid_planting_base_rejects_stone() {
        let mut world = fresh_world_with_water_at(1, 64, 0);
        world.set_block(0, 64, 0, block::STONE);
        assert!(!is_valid_planting_base(&world, 0, 64, 0));
    }

    #[test]
    fn is_valid_planting_base_rejects_when_block_above_is_solid() {
        let mut world = fresh_world_with_water_at(1, 64, 0);
        world.set_block(0, 64, 0, block::DIRT);
        // Block above the base is occupied — no room to grow.
        world.set_block(0, 65, 0, block::STONE);
        assert!(!is_valid_planting_base(&world, 0, 64, 0));
    }

    #[test]
    fn is_valid_planting_base_rejects_dry_tile() {
        let mut world = World::new();
        world.set_block(0, 64, 0, block::DIRT);
        // No water anywhere.
        assert!(!is_valid_planting_base(&world, 0, 64, 0));
    }

    #[test]
    fn try_plant_papyrus_writes_stage_0_above_valid_base() {
        let mut world = fresh_world_with_water_at(1, 64, 0);
        world.set_block(0, 64, 0, block::DIRT);
        assert!(try_plant_papyrus(&mut world, 0, 64, 0));
        assert_eq!(world.get_block(0, 65, 0), block::PAPYRUS_STAGE_0);
    }

    #[test]
    fn try_plant_papyrus_idempotent_on_failure() {
        // Dry tile — planting must refuse + leave the world untouched.
        let mut world = World::new();
        world.set_block(0, 64, 0, block::DIRT);
        let above_before = world.get_block(0, 65, 0);
        assert!(!try_plant_papyrus(&mut world, 0, 64, 0));
        assert_eq!(world.get_block(0, 65, 0), above_before);
    }
}

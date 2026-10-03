//! Falling-block physics (sand / gravel).
//!
//! Pure free function over world state — no renderer, no ECS. The caller is
//! responsible for broadcasting `BlockChange` records to connected clients
//! and/or rebuilding dirty chunk meshes. Both `GameServer::tick_falling_blocks`
//! (authoritative path) and the single-player client path in `game_loop.rs`
//! call this the same way on their respective worlds.
//!
//! Eventual crate home: `genesis_server` (server-authoritative sim).

use glam::Vec3;

use crate::block;
use crate::block::BlockRegistry;
use crate::chunk::CHUNK_SIZE;
use crate::protocol::BlockChange;
use crate::water::WaterSystem;
use crate::world::World;

/// Scan radius around each player position (blocks).
const SCAN_RADIUS: i32 = 16;

/// Run one 5 Hz falling-block pass. Gate upstream on `tick % 4 == 0`.
///
/// Scans a `SCAN_RADIUS`-block cube around every player position and, for each
/// gravity block (sand / gravel) with air or water below it, moves it down one
/// and emits two `BlockChange` records (src → AIR, dst → block). Water sources
/// displaced by landing sand are removed from `water`.
///
/// Returns all block changes from this pass, caller order.
pub fn tick_falling_blocks(
    world: &mut World,
    registry: &BlockRegistry,
    water: &mut WaterSystem,
    player_positions: &[Vec3],
    max_chunk_y: i32,
) -> Vec<BlockChange> {
    if player_positions.is_empty() {
        return Vec::new();
    }

    let max_y = (max_chunk_y + 1) * CHUNK_SIZE as i32;

    // Collect candidates first so we don't mutate the world mid-scan.
    let mut to_fall: Vec<(i32, i32, i32)> = Vec::new();
    let mut seen: ahash::AHashSet<(i32, i32, i32)> = ahash::AHashSet::new();

    for p_pos in player_positions {
        let px = p_pos.x.floor() as i32;
        let pz = p_pos.z.floor() as i32;
        for x in (px - SCAN_RADIUS)..=(px + SCAN_RADIUS) {
            for z in (pz - SCAN_RADIUS)..=(pz + SCAN_RADIUS) {
                for y in 1..max_y {
                    if !seen.insert((x, y, z)) {
                        continue;
                    }
                    let b = world.get_block(x, y, z);
                    if registry.has_gravity(b) {
                        let below = world.get_block(x, y - 1, z);
                        if below == block::AIR || below == block::WATER {
                            to_fall.push((x, y, z));
                        }
                    }
                }
            }
        }
    }

    let mut changes: Vec<BlockChange> = Vec::with_capacity(to_fall.len() * 2);

    for (x, y, z) in to_fall {
        // Re-read in case a previous iteration consumed this cell.
        let b = world.get_block(x, y, z);
        if !registry.has_gravity(b) {
            continue;
        }
        let below = world.get_block(x, y - 1, z);
        if below != block::AIR && below != block::WATER {
            continue;
        }

        // Spec 06 §2.2 — the player-placed flag travels with the block. A
        // falling block keeps its origin: player-placed sand stays placed at
        // the landing cell (can't be laundered into "natural" by dropping it
        // and re-mining), and natural gravel stays natural.
        let was_placed = world.is_placed(x, y, z);
        world.set_block(x, y, z, block::AIR);
        world.set_block(x, y - 1, z, b);
        world.set_placed(x, y, z, false);
        world.set_placed(x, y - 1, z, was_placed);
        // Owner-inbox #1/2/3 — the vacated source cell drops any wallpaper
        // overlays (a fallen block doesn't carry its paper down). The dest was
        // AIR/WATER (guarded above), so any overlay there is a stale orphan —
        // clear it too so it can't transfer onto the fallen block.
        world.remove_face_attachments_at((x, y, z));
        world.remove_face_attachments_at((x, y - 1, z));
        changes.push(BlockChange { x, y, z, new_block: block::AIR, meta: 0 });
        changes.push(BlockChange { x, y: y - 1, z, new_block: b, meta: 0 });

        if below == block::WATER {
            water.remove_source(x, y - 1, z);
        }
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{BlockRegistry, AIR, GRAVEL, SAND, STONE};

    /// Build a minimal support floor around (0,0,0) so dropped blocks have somewhere
    /// to land. The world is empty everywhere else.
    fn fixture_floor(world: &mut World, floor_y: i32) {
        for x in -2..=2 {
            for z in -2..=2 {
                world.set_block(x, floor_y, z, STONE);
            }
        }
    }

    #[test]
    fn no_players_yields_no_changes() {
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        world.set_block(0, 10, 0, SAND);
        let changes = tick_falling_blocks(&mut world, &reg, &mut water, &[], 5);
        assert!(changes.is_empty());
        assert_eq!(world.get_block(0, 10, 0), SAND, "sand must not move without players");
    }

    #[test]
    fn sand_drops_one_cell_per_tick() {
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        fixture_floor(&mut world, 5);
        world.set_block(0, 10, 0, SAND);

        let changes = tick_falling_blocks(
            &mut world,
            &reg,
            &mut water,
            &[Vec3::new(0.0, 10.0, 0.0)],
            5,
        );

        // Sand moved from y=10 to y=9 — two block changes.
        assert_eq!(world.get_block(0, 10, 0), AIR);
        assert_eq!(world.get_block(0, 9, 0), SAND);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn gravel_falls_same_as_sand() {
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        fixture_floor(&mut world, 5);
        world.set_block(0, 10, 0, GRAVEL);

        tick_falling_blocks(&mut world, &reg, &mut water, &[Vec3::new(0.0, 10.0, 0.0)], 5);
        assert_eq!(world.get_block(0, 9, 0), GRAVEL);
    }

    #[test]
    fn stone_does_not_fall() {
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        world.set_block(0, 10, 0, STONE);
        let changes = tick_falling_blocks(&mut world, &reg, &mut water, &[Vec3::new(0.0, 10.0, 0.0)], 5);
        assert!(changes.is_empty());
        assert_eq!(world.get_block(0, 10, 0), STONE);
    }

    #[test]
    fn sand_rests_on_solid_floor() {
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        fixture_floor(&mut world, 5);
        world.set_block(0, 6, 0, SAND);
        // Multiple ticks must not push sand through the floor.
        for _ in 0..10 {
            tick_falling_blocks(&mut world, &reg, &mut water, &[Vec3::new(0.0, 6.0, 0.0)], 5);
        }
        assert_eq!(world.get_block(0, 6, 0), SAND, "sand must rest directly on floor");
        assert_eq!(world.get_block(0, 5, 0), STONE, "floor must be untouched");
    }

    #[test]
    fn sand_landing_on_water_removes_source() {
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        fixture_floor(&mut world, 5);
        world.set_block(0, 6, 0, block::WATER);
        water.add_source(0, 6, 0);
        world.set_block(0, 10, 0, SAND);

        // Sand falls and lands on water block — repeated ticks.
        for _ in 0..10 {
            tick_falling_blocks(&mut world, &reg, &mut water, &[Vec3::new(0.0, 10.0, 0.0)], 5);
        }
        assert!(!water.is_source(0, 6, 0), "water source must be consumed when sand lands on it");
    }

    #[test]
    fn falling_block_clears_its_face_attachments() {
        // Owner-inbox #1/2/3 — a painted sand block that falls must NOT leave an
        // orphan wallpaper overlay at the vacated cell. An orphan is invisible
        // (the decal mesher skips overlays over non-solid cells) until a block
        // re-occupies the cell, when it resurfaces as phantom wallpaper + a
        // free-item dupe on break.
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        fixture_floor(&mut world, 5);
        world.set_block(0, 10, 0, SAND);
        world.set_face_attachment(
            (0, 10, 0),
            0,
            crate::world::FaceAttachment::Wallpaper(crate::block::WALLPAPER_RED),
        );
        // Seed a STALE orphan overlay at the destination cell (still AIR) — the
        // fallen block must not inherit it (the dest is cleared on landing).
        world.set_face_attachment(
            (0, 9, 0),
            2,
            crate::world::FaceAttachment::Wallpaper(crate::block::WALLPAPER_BLUE),
        );

        tick_falling_blocks(&mut world, &reg, &mut water, &[Vec3::new(0.0, 10.0, 0.0)], 5);

        assert!(
            world.face_attachment_at((0, 10, 0), 0).is_none(),
            "vacated source cell must not keep its wallpaper"
        );
        assert!(
            world.face_attachment_at((0, 9, 0), 0).is_none(),
            "a fallen block does not carry its paper down to the destination"
        );
        assert!(
            world.face_attachment_at((0, 9, 0), 2).is_none(),
            "a stale orphan at the destination must be cleared, not transferred onto the fallen block"
        );
    }

    #[test]
    fn player_placed_block_keeps_placed_flag_through_fall() {
        // Spec 06 §2.2 — a player places sand on a ledge; when it falls the
        // placed flag must travel with it so re-mining the landed block still
        // earns no work (no "launder a placed block into a natural one").
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        fixture_floor(&mut world, 5);
        world.place_player_block(0, 10, 0, SAND);
        assert!(world.is_placed(0, 10, 0));

        tick_falling_blocks(&mut world, &reg, &mut water, &[Vec3::new(0.0, 10.0, 0.0)], 5);

        assert_eq!(world.get_block(0, 9, 0), SAND);
        assert!(world.is_placed(0, 9, 0), "placed flag must travel with the fallen block");
        assert!(!world.is_placed(0, 10, 0), "vacated source cell must be cleared");
    }

    #[test]
    fn natural_gravel_stays_natural_through_fall() {
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        fixture_floor(&mut world, 5);
        world.set_block(0, 10, 0, GRAVEL); // natural

        tick_falling_blocks(&mut world, &reg, &mut water, &[Vec3::new(0.0, 10.0, 0.0)], 5);

        assert_eq!(world.get_block(0, 9, 0), GRAVEL);
        assert!(!world.is_placed(0, 9, 0), "natural gravel must stay natural after falling");
    }

    #[test]
    fn returned_changes_describe_actual_mutations() {
        let mut world = World::new();
        let reg = BlockRegistry::new();
        let mut water = WaterSystem::new();
        fixture_floor(&mut world, 5);
        world.set_block(0, 10, 0, SAND);

        let changes = tick_falling_blocks(&mut world, &reg, &mut water, &[Vec3::new(0.0, 10.0, 0.0)], 5);

        // Every change entry must be consistent with the current world state.
        for c in &changes {
            assert_eq!(world.get_block(c.x, c.y, c.z), c.new_block,
                "change {:?} must reflect post-tick world state", c);
        }
    }
}

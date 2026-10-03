//! Spawn-proof overlay (#8) — a toggle that marks where hostile mobs can spawn,
//! so builders can light-proof an area. The marker geometry is rendered through
//! the shared world-space overlay buffer (`renderer::set_spawn_markers`); the
//! eligibility rule + the column scan live here (pure, headless-testable).
//!
//! **Engine rule is binary**, not Minecraft's graded 0-7: `spawning.rs` spawns a
//! mob on a solid, non-water surface only where the **block-light** one cell
//! above is exactly 0 (sky-light is handled by the day/night gate elsewhere). So
//! the overlay marks "will spawn at night" cells in one colour — there is no
//! "spawnable only in deeper dark" tier to distinguish.
//!
//! Spec: `docs/foundations/2026-06-16-spawn-proof-overlay.md`.

use crate::block;
use crate::block::BlockId;
use crate::world::World;

/// How many columns out from the player the overlay scans (a 41×41 area at 20).
/// The scan is throttled by the caller since it is O(radius²) columns.
pub const SPAWN_OVERLAY_RADIUS: i32 = 20;

/// Whether a mob can spawn standing on `surface`, given the block-light at the
/// cell directly above it. Mirrors `spawning::tick_mob_spawning`'s gate.
pub fn is_spawnable_surface(surface: BlockId, block_light_above: u8) -> bool {
    surface != block::AIR && surface != block::WATER && block_light_above == 0
}

/// Collect the surface cells within `radius` columns of `centre` (x,z) where a
/// mob can spawn at night. Each returned position is the **surface block** (the
/// marker renders on its top). Bounded by `radius`; the caller throttles.
pub fn scan_spawnable(world: &World, centre: [i32; 3], radius: i32) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            let x = centre[0] + dx;
            let z = centre[2] + dz;
            if let Some((sy, sb)) = world.highest_block(x, z) {
                let light = world.block_light_at(x, sy + 1, z);
                if is_spawnable_surface(sb, light) {
                    out.push((x, sy, z));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawnable_only_on_solid_dark_non_water_surfaces() {
        assert!(is_spawnable_surface(block::STONE, 0), "dark stone top spawns");
        assert!(!is_spawnable_surface(block::STONE, 1), "any block-light blocks it");
        assert!(!is_spawnable_surface(block::AIR, 0), "no surface, no spawn");
        assert!(!is_spawnable_surface(block::WATER, 0), "water surface doesn't spawn");
    }

    #[test]
    fn scan_finds_dark_surfaces_and_skips_lit_or_empty_columns() {
        let mut w = World::new();
        // A stone surface at (0,40,0) in the dark — spawnable.
        w.set_block(0, 40, 0, block::STONE);
        // A stone surface at (2,40,0) but lit (a torch-level light above) — safe.
        w.set_block(2, 40, 0, block::STONE);
        w.set_block_light_at(2, 41, 0, 7);
        let cells = scan_spawnable(&w, [0, 40, 0], 3);
        assert!(cells.contains(&(0, 40, 0)), "dark stone is a spawn cell");
        assert!(!cells.contains(&(2, 40, 0)), "lit stone is light-proofed");
        // An empty (air) column contributes nothing.
        assert!(!cells.iter().any(|&(x, _, z)| x == 3 && z == 3));
    }
}

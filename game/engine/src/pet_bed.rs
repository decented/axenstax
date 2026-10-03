//! Pet Bed rescue (2026-07-06 pets wave) — an anti-loss anchor for tamed
//! pets. When a player's owned pet (Wolf / Companion (Cat, Parrot, ...) /
//! Nostrich / Horse-family) would otherwise die, the death-handling sweep in
//! `game_loop.rs` calls [`find_pet_bed_near`] to look for a `PET_BED` block
//! in the loaded world close to the death position. If one is found, the pet
//! is teleported there and fully healed instead of despawning — see
//! `block.rs`'s doc comment on `block::PET_BED` for the full feature note.
//!
//! Beds are communal in v1 — there's no per-bed ownership; the *pet* must be
//! owned (any of the four ownership shapes probed by
//! `tameable::pet_owner_of`), but any bed in range will do.
//!
//! ## Scan bounds
//!
//! The search box is intentionally generous but bounded: ±[`BED_RESCUE_RADIUS_XZ`]
//! blocks horizontally, ±[`BED_RESCUE_RADIUS_Y`] blocks vertically, centred on
//! the death position. This is a death-time-only scan (rescues are rare
//! events), so the ~139k-cell box is cheap in practice — no per-tick cost.
//!
//! `World::get_block` returns `block::AIR` for any position in an unloaded
//! chunk (see `world.rs`'s `get_block`, which falls back to `AIR` when the
//! chunk lookup misses) — so this scan naturally treats unloaded regions as
//! "no bed here" without any separate loaded-chunk check.

use crate::block;
use crate::world::World;

/// Horizontal (x/z) half-extent of the bed search box, in blocks.
pub const BED_RESCUE_RADIUS_XZ: i32 = 32;
/// Vertical (y) half-extent of the bed search box, in blocks.
pub const BED_RESCUE_RADIUS_Y: i32 = 16;

/// Scan the loaded world around `center` for the nearest `PET_BED` block
/// within the bounded box (±[`BED_RESCUE_RADIUS_XZ`] xz, ±[`BED_RESCUE_RADIUS_Y`]
/// y). Returns the bed's block coordinates, or `None` if no bed is in range
/// (including when the whole region is unloaded, since unloaded cells read
/// as `AIR`).
pub fn find_pet_bed_near(world: &World, center: (f32, f32, f32)) -> Option<(i32, i32, i32)> {
    let (cx, cy, cz) = (center.0, center.1, center.2);
    let bx = cx.floor() as i32;
    let by = cy.floor() as i32;
    let bz = cz.floor() as i32;

    let mut nearest: Option<(i32, i32, i32)> = None;
    let mut nearest_dist_sq = f32::MAX;

    for x in (bx - BED_RESCUE_RADIUS_XZ)..=(bx + BED_RESCUE_RADIUS_XZ) {
        for z in (bz - BED_RESCUE_RADIUS_XZ)..=(bz + BED_RESCUE_RADIUS_XZ) {
            for y in (by - BED_RESCUE_RADIUS_Y)..=(by + BED_RESCUE_RADIUS_Y) {
                if world.get_block(x, y, z) != block::PET_BED {
                    continue;
                }
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let dz = z as f32 - cz;
                let dist_sq = dx * dx + dy * dy + dz * dz;
                if dist_sq < nearest_dist_sq {
                    nearest_dist_sq = dist_sq;
                    nearest = Some((x, y, z));
                }
            }
        }
    }
    nearest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn finds_nearest_bed_within_radius() {
        let mut world = World::new();
        world.set_block(5, 64, 0, crate::block::PET_BED);
        world.set_block(20, 64, 0, crate::block::PET_BED);
        assert_eq!(find_pet_bed_near(&world, (0.0, 64.0, 0.0)), Some((5, 64, 0)));
    }

    #[test]
    fn no_bed_out_of_radius() {
        let mut world = World::new();
        world.set_block(100, 64, 0, crate::block::PET_BED);
        assert_eq!(find_pet_bed_near(&world, (0.0, 64.0, 0.0)), None);
    }

    #[test]
    fn no_bed_anywhere_returns_none() {
        let world = World::new();
        assert_eq!(find_pet_bed_near(&world, (0.0, 64.0, 0.0)), None);
    }

    #[test]
    fn bed_out_of_vertical_radius_is_not_found() {
        let mut world = World::new();
        world.set_block(0, 64 + BED_RESCUE_RADIUS_Y + 1, 0, crate::block::PET_BED);
        assert_eq!(find_pet_bed_near(&world, (0.0, 64.0, 0.0)), None);
    }

    #[test]
    fn bed_at_exact_radius_edge_is_found() {
        let mut world = World::new();
        world.set_block(BED_RESCUE_RADIUS_XZ, 64, 0, crate::block::PET_BED);
        assert_eq!(
            find_pet_bed_near(&world, (0.0, 64.0, 0.0)),
            Some((BED_RESCUE_RADIUS_XZ, 64, 0))
        );
    }
}

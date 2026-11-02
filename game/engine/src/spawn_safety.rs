//! Spawn safety: a body put at a spawn point stands on ground, its feet and
//! head cells clear.
//!
//! The world spawn (`chunk_stream::world_spawn_point`) already searches for
//! clear ground, but two spots handed to a joiner were never checked against
//! the real loaded world: the search's estimate-based fallback, and a returning
//! joiner's restored saved spawn. [`fix_up`] is the one check for both, and for
//! the respawn spot. It reads blocks only: a spot in a column that is not
//! loaded comes back unchanged.
//!
//! Also here, the client-side half of the phone reports of "spawning inside
//! terrain": every joiner of a Keep starts on the same spawn cell, so the
//! second one's camera sat inside the first one's avatar (drawn without
//! back-face culling: a flat skin-coloured screen). [`camera_inside_avatar`]
//! is the test the avatar draw loop skips such a body by.

use crate::block::{self, BlockRegistry};
use crate::world::World;
use glam::Vec3;

/// How far (blocks, Chebyshev) [`fix_up`] looks for a clear column round a
/// blocked one: the world spawn's own clear-ground spiral reach.
pub const SEARCH_RADIUS: i32 = 6;

/// How far up or down its own column [`fix_up`] first nudges a spot that is
/// nearly right (a spawn half a block in the floor, or a step above it).
const NUDGE: i32 = 2;

/// Can a body stand at `(x, feet_y, z)`: feet and head cells neither solid
/// nor fluid, and a block under the feet that is not air or fluid (the ground
/// `standing_spot` stands a body on: a bed or a slab counts as well as stone).
pub fn stands_clear(world: &World, registry: &BlockRegistry, x: i32, feet_y: i32, z: i32) -> bool {
    let open = |y: i32| {
        let b = world.get_block(x, y, z);
        !registry.is_solid(b) && !block::is_fluid(b)
    };
    let below = world.get_block(x, feet_y.saturating_sub(1), z);
    open(feet_y) && open(feet_y.saturating_add(1)) && below != block::AIR && !block::is_fluid(below)
}

/// How far (blocks) below `pos.y` the ground's top may be for [`can_stand_exactly`]
/// to call a body supported: a float's worth, not a fall.
const GROUND_TOLERANCE: f32 = 0.05;

/// Can a body stand at exactly `pos`, as physics would leave it there? The
/// body's box (`PLAYER_WIDTH` wide, `PLAYER_HEIGHT` tall, from `pos.y` up) must
/// touch no collision box (a box whose top is at or below `pos.y` plus the
/// landing slack is the floor, not an obstruction), sit in no fluid or fire,
/// and have ground under its footprint: a collision box whose top is within
/// [`GROUND_TOLERANCE`] below `pos.y`. Collision boxes, not whole cells, so
/// a slab, a low stair step, a bottom trapdoor or a snow layer is a floor, and
/// a body within 0.3 of a ledge, its centre over air, still stands. Fails
/// closed: false for a position that is not finite, a column that is not
/// loaded (it reads as air, so nothing is ground), a spot inside a block or
/// over a drop. [`stands_clear`] stays the whole-cell check [`fix_up`] and the
/// world spawn use.
pub fn can_stand_exactly(world: &World, registry: &BlockRegistry, pos: Vec3) -> bool {
    use crate::physics::{FOOT_INSET, LAND_EPS, PLAYER_HEIGHT, PLAYER_WIDTH};
    if !pos.is_finite() {
        return false;
    }
    let half = PLAYER_WIDTH / 2.0 - FOOT_INSET;
    let (x0, x1) = ((pos.x - half).floor() as i32, (pos.x + half).floor() as i32);
    let (z0, z1) = ((pos.z - half).floor() as i32, (pos.z + half).floor() as i32);
    let top = pos.y + PLAYER_HEIGHT;
    let feet_cell = pos.y.floor() as i32;
    let head_cell = (top - LAND_EPS).floor() as i32;
    let mut supported = false;
    for x in x0..=x1 {
        for z in z0..=z1 {
            // Ground may be in the cell just under the feet when `pos.y` sits on a
            // cell boundary; the cells the body occupies are `feet_cell..=head_cell`.
            let lowest = (pos.y - GROUND_TOLERANCE).floor() as i32;
            for y in lowest.min(feet_cell)..=head_cell {
                let id = world.get_block(x, y, z);
                let body_cell = y >= feet_cell;
                if body_cell && (block::is_fluid(id) || id == block::FIRE) {
                    return false;
                }
                for b in world.collision_boxes_at(x, y, z, registry) {
                    let overlaps_xz = b.min[0] < pos.x + half
                        && b.max[0] > pos.x - half
                        && b.min[2] < pos.z + half
                        && b.max[2] > pos.z - half;
                    if !overlaps_xz {
                        continue;
                    }
                    if b.max[1] > pos.y + LAND_EPS && b.min[1] < top - LAND_EPS {
                        return false;
                    }
                    if b.max[1] <= pos.y + LAND_EPS && b.max[1] >= pos.y - GROUND_TOLERANCE {
                        supported = true;
                    }
                }
            }
        }
    }
    supported
}

/// The surface of the loaded column `(x, z)`: walk down from its real top
/// (`World::column_top_y`, not a generator estimate) to the first ground —
/// past air, and past leaves and logs so a tree's canopy is not the floor —
/// and return the feet Y above it when the body can stand there. `None` for an
/// unloaded or empty column, a water- or lava-topped one, or one whose ground
/// has no room above it (under a trunk).
pub fn surface_feet(world: &World, registry: &BlockRegistry, x: i32, z: i32) -> Option<i32> {
    let mut y = world.column_top_y(x, z);
    while y > 0 {
        let b = world.get_block(x, y, z);
        if block::is_fluid(b) {
            return None;
        }
        let passable = b == block::AIR
            || block::is_any_leaves(b)
            || block::is_any_log_block(b)
            || !registry.is_solid(b);
        if !passable {
            return stands_clear(world, registry, x, y + 1, z).then_some(y + 1);
        }
        y -= 1;
    }
    None
}

/// The feet Y just above the highest block of the loaded column `(x, z)`,
/// whatever it is: clear of terrain by construction (everything above a
/// column's top is air). `None` when the column has no block (not loaded).
pub fn above_column_top(world: &World, x: i32, z: i32) -> Option<i32> {
    let mut y = world.column_top_y(x, z);
    while y >= 0 {
        if world.get_block(x, y, z) != block::AIR {
            return Some(y + 1);
        }
        y -= 1;
    }
    None
}

/// Where a body put at `pos` should stand. `pos` itself when it already
/// [`stands_clear`]; else the nearest clear cell within [`NUDGE`] up or down
/// its own column; else its column's surface; else the surface of the nearest
/// column within [`SEARCH_RADIUS`] (rings outward, a fixed order, so every
/// joiner given the same spot gets the same answer); else just above its own
/// column's top. `pos` unchanged when it is not finite or its column is not
/// loaded (nothing to read). A saved spawn can be any finite float, so the
/// block maths saturates at the `i32` limits rather than overflowing.
pub fn fix_up(world: &World, registry: &BlockRegistry, pos: Vec3) -> Vec3 {
    if !pos.is_finite() {
        return pos;
    }
    let (x, fy, z) = (pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32);
    if stands_clear(world, registry, x, fy, z) {
        return pos;
    }
    for step in 1..=NUDGE {
        for y in [fy.saturating_add(step), fy.saturating_sub(step)] {
            if stands_clear(world, registry, x, y, z) {
                return Vec3::new(pos.x, y as f32, pos.z);
            }
        }
    }
    if let Some(y) = surface_feet(world, registry, x, z) {
        return Vec3::new(pos.x, y as f32, pos.z);
    }
    for ring in 1..=SEARCH_RADIUS {
        for dz in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dz.abs()) != ring {
                    continue;
                }
                let (nx, nz) = (x.saturating_add(dx), z.saturating_add(dz));
                if let Some(y) = surface_feet(world, registry, nx, nz) {
                    return Vec3::new(nx as f32 + 0.5, y as f32, nz as f32 + 0.5);
                }
            }
        }
    }
    match above_column_top(world, x, z) {
        Some(y) => Vec3::new(pos.x, y as f32, pos.z),
        None => pos,
    }
}

/// Is a camera at `eye` inside (or against) the body of a player standing at
/// `feet`? Horizontally within a body's width of its centre and vertically
/// between its feet and the top of its head — or, as the draw loop always
/// skipped, within ~0.7 blocks of its feet.
pub fn camera_inside_avatar(feet: Vec3, eye: Vec3) -> bool {
    /// A player body is 0.6 wide; the head box reaches a little past it.
    const REACH: f32 = 0.6;
    /// Feet to the top of the head, with a small margin.
    const HEIGHT: f32 = 2.0;
    if (feet - eye).length_squared() < 0.5 {
        return true;
    }
    let (dx, dz) = (eye.x - feet.x, eye.z - feet.z);
    dx * dx + dz * dz < REACH * REACH && eye.y >= feet.y && eye.y <= feet.y + HEIGHT
}

impl crate::server::GameServer {
    /// [`fix_up`] on this server's world, first loading the 3x3 columns round
    /// `pos` when `load` (an owning server; a lending host's columns are its
    /// streamer's to load). For a spot a joiner is handed that the world
    /// spawn search did not choose: a returning joiner's saved spawn.
    pub(crate) fn safe_spawn_loading(&mut self, pos: Vec3, load: bool) -> Vec3 {
        if load && pos.is_finite() {
            let (cx, cz) = crate::chunk_stream::column_of(pos);
            for dx in -1..=1 {
                for dz in -1..=1 {
                    self.ensure_column_loaded(cx + dx, cz + dz);
                }
            }
        }
        fix_up(&self.world, &self.registry, pos)
    }

    /// Keep slice 2a: can a body go back to exactly `pos`? [`can_stand_exactly`],
    /// not [`fix_up`] unchanged: `fix_up` leaves a spot in an unloaded column as
    /// it is, because it has nothing to read.
    pub(crate) fn can_stand_exactly_loading(&mut self, pos: Vec3, load: bool) -> bool {
        if !pos.is_finite() {
            return false;
        }
        self.safe_spawn_loading(pos, load);
        can_stand_exactly(&self.world, &self.registry, pos)
    }
}

#[cfg(test)]
mod tests;

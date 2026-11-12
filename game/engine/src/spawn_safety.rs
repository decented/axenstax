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
/// `safe_respawn_point` stands a body on: a bed or a slab counts as well as stone).
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

// ---- The one respawn rule ------------------------------------------------

/// What [`safe_respawn_point`] needs of whoever respawns a body: the world it
/// reads, and a way to make a column exist before it does. One impl per owner
/// of a world (the client's `GameState`, the server's `GameServer`), so the
/// single-player respawn and a joiner's go through the same function.
pub(crate) trait RespawnWorld {
    /// The world, its registry and its generator.
    fn respawn_parts(&self) -> (&World, &BlockRegistry, &crate::biome::BiomeGenerator);
    /// Make column `(cx, cz)` exist (restore it if evicted, else generate it,
    /// light it) without meshing it: a no-op when it is loaded, and for a world
    /// this owner may not generate into (a joiner's, a lent host's server).
    /// `true` when this call loaded it.
    fn ensure_respawn_column(&mut self, cx: i32, cz: i32) -> bool;
    /// The columns this respawn loaded, once, when it is done: the owner draws
    /// them (the client queues their meshes the way streaming does, so a
    /// respawn never meshes a dozen columns in one frame). Nothing to do on a
    /// server.
    fn respawn_loaded(&mut self, _columns: &[(i32, i32)]) {}
}

/// The columns [`safe_respawn_point`] reads round `spawn`: its own 3x3.
fn columns_round(spawn: Vec3) -> impl Iterator<Item = (i32, i32)> {
    let (cx, cz) = crate::chunk_stream::column_of(spawn);
    (-1..=1).flat_map(move |dx| (-1..=1).map(move |dz| (cx.saturating_add(dx), cz.saturating_add(dz))))
}

/// A body at `spawn` falls (or is lifted) in its own column: down to the first
/// non-air cell at or below it, or, from a spawn cell buried in the ground, up
/// out of the buried run. Reads only the column; `None` for a column with
/// nothing below the spawn to stand on (unloaded, or a void). The first step of
/// [`settle`]; a liquid is not told from ground here, [`fix_up`] and the dry
/// check after it do that.
fn standing_in_column(world: &World, spawn: Vec3) -> Option<Vec3> {
    let (x, z) = (spawn.x.floor() as i32, spawn.z.floor() as i32);
    // Above the column's highest block every cell is air.
    let top = world.column_top_y(x, z).max(0);
    let mut y = (spawn.y.floor() as i32).clamp(0, top + 1);
    let air = |y: i32| world.get_block(x, y, z) == block::AIR;
    if !air(y) {
        while y <= top && !air(y) {
            y += 1;
        }
        return Some(Vec3::new(spawn.x, y as f32, spawn.z));
    }
    while y > 0 && air(y) {
        y -= 1;
    }
    if air(y) {
        return None;
    }
    Some(Vec3::new(spawn.x, y as f32 + 1.0, spawn.z))
}

/// A spawn cell that is not itself clear (a bed in a room two cells high: the
/// ceiling is where the head goes) first tries the cells BESIDE it, as a bed
/// respawn does in Minecraft: ring 1, then ring 2 (a fixed order), at the
/// spawn's level less one (a body standing on the floor beside the bed) and at
/// the spawn's level. Without it the vertical nudge in [`fix_up`] climbs past
/// the ceiling and puts the body on the roof. `None` when the spawn is clear
/// already or no cell beside it is.
fn beside_spawn(world: &World, registry: &BlockRegistry, at: Vec3) -> Option<Vec3> {
    if !at.is_finite() {
        return None;
    }
    let (x, fy, z) = (at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
    if stands_clear(world, registry, x, fy, z) {
        return None;
    }
    // Nearest first: ring 1 before ring 2, and within a ring the cells straight
    // beside the spawn before the diagonal ones (a bed in a hut's corner steps
    // into the room, not out through the doorway), then a fixed order.
    let mut offsets: Vec<(i32, i32)> = (-BESIDE_RADIUS..=BESIDE_RADIUS)
        .flat_map(|dz| (-BESIDE_RADIUS..=BESIDE_RADIUS).map(move |dx| (dx, dz)))
        .filter(|&(dx, dz)| (dx, dz) != (0, 0))
        .collect();
    offsets.sort_by_key(|&(dx, dz)| (dx.abs().max(dz.abs()), dx * dx + dz * dz, dz, dx));
    for (dx, dz) in offsets {
        let (nx, nz) = (x.saturating_add(dx), z.saturating_add(dz));
        for y in [fy.saturating_sub(1), fy] {
            if stands_clear(world, registry, nx, y, nz) {
                return Some(Vec3::new(nx as f32 + 0.5, y as f32, nz as f32 + 0.5));
            }
        }
    }
    None
}

/// How far (blocks) [`beside_spawn`] looks round a spawn cell that is not clear.
const BESIDE_RADIUS: i32 = 2;

/// Where a body put at `spawn` stands, by what is loaded: its column's ground
/// ([`standing_in_column`]), then the cells beside it if it is not clear
/// ([`beside_spawn`]), then [`fix_up`].
fn settle(world: &World, registry: &BlockRegistry, spawn: Vec3) -> Vec3 {
    let ground = standing_in_column(world, spawn).unwrap_or(spawn);
    beside_spawn(world, registry, ground).unwrap_or_else(|| fix_up(world, registry, ground))
}

/// How far (blocks) [`dry_land_near`] looks round a world spawn that is not dry.
const DRY_SEARCH_RADIUS: i32 = 32;

/// The nearest dry standing spot within [`DRY_SEARCH_RADIUS`] of `near`, over
/// the columns that are loaded (rings outward in a fixed order; a column that
/// is not loaded or is water-topped is skipped). The floor of the respawn
/// guarantee for a world whose own spawn is sea: land close by, if the loaded
/// ground holds any.
fn dry_land_near(world: &World, registry: &BlockRegistry, near: Vec3) -> Option<Vec3> {
    if !near.is_finite() {
        return None;
    }
    let (x, z) = (near.x.floor() as i32, near.z.floor() as i32);
    for ring in 0..=DRY_SEARCH_RADIUS {
        for dz in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dz.abs()) != ring {
                    continue;
                }
                let (nx, nz) = (x.saturating_add(dx), z.saturating_add(dz));
                if let Some(y) = surface_feet(world, registry, nx, nz) {
                    return Some(Vec3::new(nx as f32 + 0.5, y as f32, nz as f32 + 0.5));
                }
            }
        }
    }
    None
}

/// Is `pos` a place to stand on dry land: feet and head cells open, and a
/// non-air, non-liquid block under the feet ([`stands_clear`] at `pos`).
pub(crate) fn is_dry_standing(world: &World, registry: &BlockRegistry, pos: Vec3) -> bool {
    pos.is_finite()
        && stands_clear(
            world,
            registry,
            pos.x.floor() as i32,
            pos.y.floor() as i32,
            pos.z.floor() as i32,
        )
}

/// THE respawn rule, for the single-player respawn and a joiner's alike: where
/// a body that dies with `spawn` as its respawn point stands up.
///
/// 1. Make the 3x3 columns round `spawn` exist (the owner's rule: see
///    [`RespawnWorld::ensure_respawn_column`]); blocks and light only, the owner
///    draws what it loaded afterwards ([`RespawnWorld::respawn_loaded`]).
/// 2. Stand it on `spawn`'s ground: in its own column from the spawn's height
///    (a bed under a roof stays under the roof), then, if that is not clear,
///    the cells beside it ([`beside_spawn`]: a bed in a small room respawns
///    beside the bed, not on the roof), else the column's top, else the nearest
///    clear column within [`SEARCH_RADIUS`] ([`settle`]).
/// 3. If that is not dry ground (the column is sea or lava, or has no ground at
///    all: [`is_dry_standing`]), use the world spawn instead: the dry land the
///    new-world search picks (`chunk_stream::world_spawn_point`), its columns
///    made to exist first, settled the same way.
/// 4. If the world spawn is not dry either (a sea origin nothing dry within
///    the search's reach could replace), the nearest dry land within
///    [`DRY_SEARCH_RADIUS`] of it over the LOADED columns ([`dry_land_near`]).
///    Only if that finds nothing is the body left where step 2 put it. That is
///    the floor of the guarantee: a world with no dry land loaded within 32
///    blocks of its spawn still respawns above water.
///
/// A spawn that cannot be read (not finite, columns the owner may not load) comes
/// back as [`fix_up`] leaves it. Nothing here reads whether the spawn came from
/// a bed: a bed that is gone is a spawn with no ground under it, so it falls
/// through step 2 to the nearest ground or to step 3.
pub(crate) fn safe_respawn_point(ctx: &mut impl RespawnWorld, spawn: Vec3) -> Vec3 {
    let mut loaded = Vec::new();
    let at = respawn_point(ctx, spawn, &mut loaded);
    if !loaded.is_empty() {
        ctx.respawn_loaded(&loaded);
    }
    at
}

fn respawn_point(ctx: &mut impl RespawnWorld, spawn: Vec3, loaded: &mut Vec<(i32, i32)>) -> Vec3 {
    // Not a number, or past the playable limit (`/tp` and a saved spawn clamp to
    // it): nothing there to read, and no column worth generating.
    let limit = crate::commands::builtins::tp::MAX_COORD;
    if !spawn.is_finite() || spawn.abs().max_element() > limit {
        return spawn;
    }
    for (cx, cz) in columns_round(spawn) {
        if ctx.ensure_respawn_column(cx, cz) {
            loaded.push((cx, cz));
        }
    }
    let here = {
        let (world, registry, _) = ctx.respawn_parts();
        settle(world, registry, spawn)
    };
    {
        let (world, registry, _) = ctx.respawn_parts();
        if is_dry_standing(world, registry, here) {
            return here;
        }
    }
    let wanted: Vec<(i32, i32)> = {
        let (world, _, biome_gen) = ctx.respawn_parts();
        crate::chunk_stream::spawn_pregen_columns(world, biome_gen).to_vec()
    };
    for (cx, cz) in wanted {
        if ctx.ensure_respawn_column(cx, cz) {
            loaded.push((cx, cz));
        }
    }
    let (world, registry, biome_gen) = ctx.respawn_parts();
    let world_spot = settle(world, registry, crate::chunk_stream::world_spawn_point(world, biome_gen));
    if is_dry_standing(world, registry, world_spot) {
        return world_spot;
    }
    dry_land_near(world, registry, world_spot).unwrap_or(here)
}

/// A server asked to respawn a body: the server, and whether it may load the
/// columns the rule reads. `may_load` comes from whoever knows how the server
/// holds its world (`HostedServer`: an owning server loads, a lending host's
/// columns are its client's streamer's, and the server's own world between
/// windows is an empty placeholder no column may be generated into).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct ServerRespawn<'a> {
    pub server: &'a mut crate::server::GameServer,
    pub may_load: bool,
}

impl RespawnWorld for ServerRespawn<'_> {
    fn respawn_parts(&self) -> (&World, &BlockRegistry, &crate::biome::BiomeGenerator) {
        (&self.server.world, &self.server.registry, &self.server.biome_gen)
    }

    fn ensure_respawn_column(&mut self, cx: i32, cz: i32) -> bool {
        if !self.may_load || self.server.loaded_columns.contains(&(cx, cz)) {
            return false;
        }
        self.server.ensure_column_loaded(cx, cz);
        true
    }
}

impl RespawnWorld for crate::GameState {
    fn respawn_parts(&self) -> (&World, &BlockRegistry, &crate::biome::BiomeGenerator) {
        (&self.world, &self.registry, &self.biome_gen)
    }

    /// A column this client holds is left alone; any other is restored or
    /// generated and lit, not meshed (`load_column_unmeshed`, which restores an
    /// evicted column before it generates, so a far bed's saved edits survive).
    /// A joined client generates nothing: its ground is what the server pushes.
    fn ensure_respawn_column(&mut self, cx: i32, cz: i32) -> bool {
        if self.joined() || self.loaded_columns.contains(&(cx, cz)) {
            return false;
        }
        self.load_column_unmeshed(cx, cz);
        true
    }

    /// The columns' meshes go through the streaming path: a bounded few
    /// sections now, the rest (and the seams) in the dirty-mesh queue the frame
    /// loop drains, not a dozen columns in one frame.
    fn respawn_loaded(&mut self, columns: &[(i32, i32)]) {
        self.mesh_stream_batch(columns, crate::stream_mesh::MeshPolicy::STREAMING);
    }
}

// ---- A saved spawn point: the legacy default is "unset" -------------------

/// The respawn point every world used to save when its player never slept in a
/// bed or typed `/spawnpoint`: `PlayerSlot::new` at the fixed fresh-slot
/// position (0.5, 80, 0.5), which the new-world search never overwrote.
pub(crate) const LEGACY_DEFAULT_SPAWN: [f32; 3] = [0.5, 80.0, 0.5];

/// The respawn point to restore for a saved slot: `saved` as it is, except the
/// exact legacy default, which was never anyone's choice and is the world spawn
/// (`world_spawn`, asked only then). No saved spawn at all (a legacy save)
/// keeps its own rule: the last position.
///
/// Why the exact value is a safe marker: a player's own spawn is a bed's cell
/// above it (`item_actions::bed_spawn`: block centre, one up) or what
/// `/spawnpoint` stored (a typed coordinate, or the player's own float position
/// with no arguments). Only a bed at (0, 79, 0) can make the default's exact
/// floats, and `bed_here` (the block under the default is a bed) leaves that
/// one alone. `/spawnpoint 0.5 80 0.5`, typed or taken standing at exactly that
/// float, is the one spot a player could have chosen that is also the marker:
/// the repair then moves it to the world spawn, which for a world whose ground
/// at the origin is at y 79 is that very point, and otherwise puts it on dry
/// ground where the default point hung in the air. No save-format change is
/// needed: the field already exists, and the repaired value is saved next time.
pub(crate) fn restored_spawn(
    saved: Option<[f32; 3]>,
    last_pos: Vec3,
    bed_here: bool,
    world_spawn: impl FnOnce() -> Vec3,
) -> Vec3 {
    match saved {
        Some(s) if s == LEGACY_DEFAULT_SPAWN && !bed_here => world_spawn(),
        Some(s) => Vec3::from_array(s),
        None => last_pos,
    }
}

#[cfg(test)]
mod tests;

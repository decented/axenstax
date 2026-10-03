//! DDA ray casting through voxel grid.
//!
//! Spec 05 Section 2.5: Ray cast from eye_pos in look direction,
//! step through voxels using DDA, return first non-air block hit.

use glam::Vec3;
use crate::block::{self, BlockId, BlockRegistry, AIR};
use crate::world::World;

/// Returns true iff the block at `(x,y,z)` is something the player's
/// crosshair should target. Solid blocks always qualify. Non-solid
/// blocks qualify too (torches, tall grass, crops, flowers, fibre
/// plants, construction anchors — anything the player breaks via
/// the normal mine path) *except* a small atmospheric/fluid set we
/// explicitly want the cursor to pass through:
///
/// - `AIR` — empty cell, nothing to hit.
/// - `WATER` — player must be able to see and aim through it.
/// - `CAMPFIRE_SMOKE` — atmospheric overlay; the campfire's tick
///   regenerates it instantly so breaking it is a wasted swing.
///
/// 2026-05-28 — added because the prior `is_solid`-only gate skipped
/// every plant block (flowers reported by playtest, but the same bug
/// hit every crop / fibre / tall grass since the very first commit).
fn is_pickable(block: BlockId, registry: &BlockRegistry) -> bool {
    if block == AIR {
        return false;
    }
    if block == block::WATER {
        return false;
    }
    if block == block::CAMPFIRE_SMOKE {
        return false;
    }
    // Solid blocks: always pickable. Non-solid + transparent blocks
    // (the "small-cube" render class — see `mesh::non_solid_shape_for`)
    // are pickable too so the player can break them.
    registry.is_solid(block) || registry.is_transparent(block)
}

/// Track + cable render as a shallow floor slab (`mesh::emit_connected_rail`),
/// not a full cube. The raycast tests only this slab height so the crosshair
/// passes *over* a rail to the block beyond — otherwise you place the next
/// piece on the rail's invisible full-cube top and it floats one cell up.
const RAIL_SLAB_H: f32 = 0.15;
fn is_thin_rail(block: BlockId) -> bool {
    matches!(block, crate::rail::TRACK | block::CABLE | block::CABLE_LIT)
}

/// Result of a ray cast.
pub struct RayHit {
    /// World position of the hit block.
    pub block_pos: [i32; 3],
    /// The face that was hit (normal direction).
    pub face_normal: [i32; 3],
    /// Distance from ray origin to hit.
    pub distance: f32,
    /// Block type at hit position.
    pub block: BlockId,
}

/// Which 16×16 texel of a block face the crosshair struck. `hit` is the
/// world-space hit point (`origin + dir * RayHit.distance`), `block` the hit
/// block position, `face_normal` the struck face (one axis ±1). Returns
/// `(u, v)`, each in `0..16` — column and row on that face.
///
/// Works at **any** rendered scale (it reads the fractional position inside the
/// 1×1 cell), which is why a block need not be blown up to 16 blocks for the
/// crosshair to select a single texel. NB: the exact axis orientation / flips
/// are reconciled against the texture UV convention in Phase 3 (painting); this
/// phase only needs a stable, clamped 16×16 mapping.
// No production caller yet — reserved for Phase 3 (painting); exercised
// directly by the tests below in the meantime.
#[cfg_attr(not(test), allow(dead_code))]
pub fn face_texel(hit: [f32; 3], block: [i32; 3], face_normal: [i32; 3]) -> (u32, u32) {
    let fx = (hit[0] - block[0] as f32).clamp(0.0, 1.0);
    let fy = (hit[1] - block[1] as f32).clamp(0.0, 1.0);
    let fz = (hit[2] - block[2] as f32).clamp(0.0, 1.0);
    let (u_frac, v_frac) = match face_normal {
        [0, 1, 0] | [0, -1, 0] => (fx, fz), // top / bottom → (x, z)
        [1, 0, 0] | [-1, 0, 0] => (fz, fy), // east / west  → (z, y)
        _ => (fx, fy),                      // north / south → (x, y)
    };
    let to_texel = |f: f32| ((f * 16.0) as i32).clamp(0, 15) as u32;
    (to_texel(u_frac), to_texel(v_frac))
}

/// What the crosshair selected on a blown-up ×4 working copy (the 16³ cage).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellPick {
    /// First occupied cell the ray hits (paint / carve / eyedropper target), 0..16.
    pub cell: [i32; 3],
    /// Face of `cell` the ray entered through (outward normal, one axis ±1).
    pub face: [i32; 3],
    /// Empty neighbour across that face (place target): `cell + face`. May lie
    /// outside 0..16 (then `place` is refused by the buffer — the cage is the wall).
    pub place: [i32; 3],
    /// World distance from `eye` to the hit (to choose the nearest of several cages).
    pub dist: f32,
}

/// Pick the cell of a blown-up ×4 working copy under the crosshair. The copy
/// fills the cage `[corner, corner+4]` (world block coords), 16 cells per side
/// (¼-block cells). `occupied(cell)` reports whether a 0..16 grid cell is a
/// solid microblock. DDA from the ray's entry into the cage AABB; returns the
/// first occupied cell hit, or `None` if the ray misses the cage or exits
/// through only empty cells.
pub fn pick_cell_in_cage(
    eye: [f32; 3],
    dir: [f32; 3],
    corner: [i32; 3],
    occupied: impl Fn([i32; 3]) -> bool,
) -> Option<CellPick> {
    const N: i32 = 16; // cells per side
    const SPAN: f32 = 4.0; // blocks per side (×4)
    const CELL: f32 = SPAN / N as f32; // 0.25 world units per cell

    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-8 {
        return None;
    }
    let dn = [dir[0] / len, dir[1] / len, dir[2] / len];
    let cf = [corner[0] as f32, corner[1] as f32, corner[2] as f32];
    let lo = cf;
    let hi = [cf[0] + SPAN, cf[1] + SPAN, cf[2] + SPAN];

    // Slab-clip the ray to the cage AABB; remember which axis we entered on.
    let mut t_enter = 0.0f32;
    let mut t_exit = f32::INFINITY;
    let mut enter_axis = 0usize;
    for a in 0..3 {
        if dn[a].abs() < 1e-8 {
            if eye[a] < lo[a] || eye[a] > hi[a] {
                return None; // parallel and outside this slab → never enters
            }
        } else {
            let inv = 1.0 / dn[a];
            let mut t0 = (lo[a] - eye[a]) * inv;
            let mut t1 = (hi[a] - eye[a]) * inv;
            if t0 > t1 {
                std::mem::swap(&mut t0, &mut t1);
            }
            if t0 > t_enter {
                t_enter = t0;
                enter_axis = a;
            }
            if t1 < t_exit {
                t_exit = t1;
            }
        }
    }
    if t_enter > t_exit || t_exit < 0.0 {
        return None;
    }

    // Origin inside the cage (or exactly on a wall): no slab raised `t_enter`
    // above 0, so `enter_axis` is meaningless. Derive the entry face from the
    // dominant viewing axis so it points back toward the viewer (the balloon
    // has no collision, so the player can edit from inside it).
    if t_enter == 0.0 {
        enter_axis = if dn[0].abs() >= dn[1].abs() && dn[0].abs() >= dn[2].abs() {
            0
        } else if dn[1].abs() >= dn[2].abs() {
            1
        } else {
            2
        };
    }

    // Entry point in world; convert to a cell index, clamped into 0..N-1.
    let start_t = t_enter.max(0.0);
    let p = [
        eye[0] + dn[0] * start_t,
        eye[1] + dn[1] * start_t,
        eye[2] + dn[2] * start_t,
    ];
    let to_cell = |w: f32, c0: f32| (((w - c0) / CELL).floor() as i32).clamp(0, N - 1);
    let mut cell = [
        to_cell(p[0], cf[0]),
        to_cell(p[1], cf[1]),
        to_cell(p[2], cf[2]),
    ];

    // Per-axis DDA setup (cell size CELL).
    let mut step = [0i32; 3];
    let mut t_max = [f32::INFINITY; 3];
    let mut t_delta = [f32::INFINITY; 3];
    for a in 0..3 {
        if dn[a] > 1e-8 {
            step[a] = 1;
            let next = cf[a] + (cell[a] + 1) as f32 * CELL;
            t_max[a] = start_t + (next - p[a]) / dn[a];
            t_delta[a] = CELL / dn[a];
        } else if dn[a] < -1e-8 {
            step[a] = -1;
            let next = cf[a] + cell[a] as f32 * CELL;
            t_max[a] = start_t + (next - p[a]) / dn[a];
            t_delta[a] = CELL / -dn[a];
        }
    }

    // The entry face: the slab we entered through faces back toward the viewer.
    let entry_face = |axis: usize, d: f32| -> [i32; 3] {
        let mut f = [0i32; 3];
        f[axis] = if d > 0.0 { -1 } else { 1 };
        f
    };
    let mut face = entry_face(enter_axis, dn[enter_axis]);
    let mut t_cur = start_t;

    for _ in 0..(N * 3 + 3) {
        if cell_in_grid_world(cell, N) && occupied(cell) {
            return Some(CellPick {
                cell,
                face,
                place: [cell[0] + face[0], cell[1] + face[1], cell[2] + face[2]],
                dist: t_cur,
            });
        }
        // Step to the next cell boundary along the smallest t_max axis.
        let a = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
            0
        } else if t_max[1] <= t_max[2] {
            1
        } else {
            2
        };
        t_cur = t_max[a];
        cell[a] += step[a];
        t_max[a] += t_delta[a];
        face = {
            let mut f = [0i32; 3];
            f[a] = -step[a];
            f
        };
        if !cell_in_grid_world(cell, N) {
            return None; // exited the cage through empty cells
        }
    }
    None
}

#[inline]
fn cell_in_grid_world(c: [i32; 3], n: i32) -> bool {
    (0..n).contains(&c[0]) && (0..n).contains(&c[1]) && (0..n).contains(&c[2])
}

/// Cast a ray through the voxel world using the DDA algorithm. Returns the first
/// block the player's crosshair should target (see [`is_pickable`]) within
/// `max_dist`. This is the gameplay/aim ray — mining, placing, combat.
pub fn cast_ray(
    origin: Vec3,
    direction: Vec3,
    max_dist: f32,
    world: &World,
    registry: &BlockRegistry,
) -> Option<RayHit> {
    cast_ray_with(origin, direction, max_dist, world, registry, is_pickable)
}

/// Like [`cast_ray`] but stops only at blocks whose per-block
/// [`CameraOcclusion`](crate::block::CameraOcclusion) collides — the third-person
/// camera-collision predicate (Phase 4). See-through blocks (glass, leaves) and
/// non-solids (plants, water) let the camera pass, so it never clips on something
/// you can see past; opaque solids squeeze it in. Keys off registry intent, not
/// the visual box (the Minecraft Glass-vs-Barrier fix).
pub fn cast_ray_camera(
    origin: Vec3,
    direction: Vec3,
    max_dist: f32,
    world: &World,
    registry: &BlockRegistry,
) -> Option<RayHit> {
    cast_ray_with(origin, direction, max_dist, world, registry, |b, r| r.camera_occludes(b))
}

/// Phase 2 — third-person camera collision. Given a `camera` (its eye is
/// `position`, plus mode + look basis) and the world, return the collision
/// fraction `[0,1]` the render eye should target this tick (`1.0` = fully
/// extended / clear). The render eye is `eye + (desired - eye) * fraction`.
/// Raycasts against SOLID blocks only. Shared by the live game loop and
/// `TestHost` so the two can't drift. **Render-only** — never affects aim.
pub fn camera_collision_fraction(
    camera: &crate::camera::Camera,
    world: &World,
    registry: &BlockRegistry,
) -> f32 {
    let eye = camera.position;
    let desired = camera.desired_render_eye();
    let off = desired - eye;
    let len = off.length();
    if len <= 1e-5 {
        return 1.0; // first-person / no pull-back → nothing to clamp
    }
    let hit = cast_ray_camera(eye, off / len, len, world, registry).map(|h| h.distance);
    crate::camera::collision_target_fraction(len, hit, crate::camera::COLLISION_MARGIN)
}

/// DDA core shared by [`cast_ray`] and [`cast_ray_solid`]. Steps voxels from
/// `origin` along `direction`, returning the first block for which `stop` is
/// true within `max_dist`.
fn cast_ray_with(
    origin: Vec3,
    direction: Vec3,
    max_dist: f32,
    world: &World,
    registry: &BlockRegistry,
    stop: impl Fn(BlockId, &BlockRegistry) -> bool,
) -> Option<RayHit> {
    if direction.length_squared() < 1e-10 {
        return None;
    }

    let dir = direction.normalize();

    // Current voxel position
    let mut voxel_x = origin.x.floor() as i32;
    let mut voxel_y = origin.y.floor() as i32;
    let mut voxel_z = origin.z.floor() as i32;

    // Step direction (+1 or -1)
    let step_x: i32 = if dir.x >= 0.0 { 1 } else { -1 };
    let step_y: i32 = if dir.y >= 0.0 { 1 } else { -1 };
    let step_z: i32 = if dir.z >= 0.0 { 1 } else { -1 };

    // Distance along ray to cross one voxel in each axis
    let t_delta_x = if dir.x.abs() > 1e-10 { (1.0 / dir.x).abs() } else { f32::MAX };
    let t_delta_y = if dir.y.abs() > 1e-10 { (1.0 / dir.y).abs() } else { f32::MAX };
    let t_delta_z = if dir.z.abs() > 1e-10 { (1.0 / dir.z).abs() } else { f32::MAX };

    // Distance to first voxel boundary in each axis
    let t_max_x = if dir.x >= 0.0 {
        ((voxel_x as f32 + 1.0) - origin.x) * t_delta_x
    } else {
        (origin.x - voxel_x as f32) * t_delta_x
    };
    let t_max_y = if dir.y >= 0.0 {
        ((voxel_y as f32 + 1.0) - origin.y) * t_delta_y
    } else {
        (origin.y - voxel_y as f32) * t_delta_y
    };
    let t_max_z = if dir.z >= 0.0 {
        ((voxel_z as f32 + 1.0) - origin.z) * t_delta_z
    } else {
        (origin.z - voxel_z as f32) * t_delta_z
    };

    let mut t_max = [t_max_x, t_max_y, t_max_z];
    let t_delta = [t_delta_x, t_delta_y, t_delta_z];
    let step = [step_x, step_y, step_z];

    let mut face_normal = [0i32; 3];
    let mut dist = 0.0f32;

    // Step through voxels
    let max_steps = (max_dist * 2.0) as usize + 1;
    for _ in 0..max_steps {
        // Check current voxel
        let block = world.get_block(voxel_x, voxel_y, voxel_z);
        if stop(block, registry) {
            // Thin rails count as a hit only if the ray actually passes through
            // their shallow floor slab within this cell; otherwise the crosshair
            // skims over and the DDA keeps stepping to the block beyond.
            let hit = if is_thin_rail(block) {
                let t_exit = t_max[0].min(t_max[1]).min(t_max[2]);
                let y_enter = origin.y + dir.y * dist;
                let y_exit = origin.y + dir.y * t_exit;
                let (ylo, yhi) = if y_enter <= y_exit {
                    (y_enter, y_exit)
                } else {
                    (y_exit, y_enter)
                };
                let slab_bot = voxel_y as f32;
                let slab_top = voxel_y as f32 + RAIL_SLAB_H;
                yhi >= slab_bot && ylo <= slab_top
            } else {
                true
            };
            if hit {
                return Some(RayHit {
                    block_pos: [voxel_x, voxel_y, voxel_z],
                    face_normal,
                    distance: dist,
                    block,
                });
            }
        }

        // Advance to next voxel boundary
        if t_max[0] < t_max[1] && t_max[0] < t_max[2] {
            dist = t_max[0];
            voxel_x += step[0];
            t_max[0] += t_delta[0];
            face_normal = [-step[0], 0, 0];
        } else if t_max[1] < t_max[2] {
            dist = t_max[1];
            voxel_y += step[1];
            t_max[1] += t_delta[1];
            face_normal = [0, -step[1], 0];
        } else {
            dist = t_max[2];
            voxel_z += step[2];
            t_max[2] += t_delta[2];
            face_normal = [0, 0, -step[2]];
        }

        if dist > max_dist {
            break;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{self, BlockRegistry};
    use crate::world::World;

    // Treats every in-grid cell as solid (a fully-inflated, un-edited balloon).
    fn cell_in_grid_i(c: [i32; 3]) -> bool {
        (0..16).contains(&c[0]) && (0..16).contains(&c[1]) && (0..16).contains(&c[2])
    }

    fn empty_world() -> (World, BlockRegistry) {
        let mut world = World::new();
        // Allocate chunk (0,0,0) so set_block has somewhere to write.
        world.set_block(0, 0, 0, block::AIR);
        (world, BlockRegistry::new())
    }

    #[test]
    fn thin_rail_lets_a_level_ray_pass_over_to_the_block_beyond() {
        // A track lies on the floor at x=2; a stone wall stands at x=5. A ray at
        // eye-ish height going level +X must skim OVER the thin rail and hit the
        // stone — so aiming at the ground ahead places flush, not on the rail.
        let (mut world, registry) = empty_world();
        world.set_block(2, 0, 0, crate::rail::TRACK);
        world.set_block(5, 0, 0, block::STONE);
        let hit = cast_ray(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.0, 0.0, 0.0),
            20.0,
            &world,
            &registry,
        )
        .expect("ray should hit the stone beyond the track");
        assert_eq!(hit.block_pos, [5, 0, 0], "skims over the thin track");
        assert_eq!(hit.block, block::STONE);
    }

    #[test]
    fn thin_rail_is_still_hit_when_aimed_low_at_it() {
        // Aiming low (within the slab) DOES hit the track, so it stays mineable.
        let (mut world, registry) = empty_world();
        world.set_block(2, 0, 0, crate::rail::TRACK);
        let hit = cast_ray(
            Vec3::new(0.5, 0.05, 0.5),
            Vec3::new(1.0, 0.0, 0.0),
            20.0,
            &world,
            &registry,
        )
        .expect("a low ray should hit the track");
        assert_eq!(hit.block_pos, [2, 0, 0]);
        assert_eq!(hit.block, crate::rail::TRACK);
    }

    #[test]
    fn ray_through_air_returns_none() {
        // Sanity baseline — an all-air column produces no hit.
        let (world, registry) = empty_world();
        let hit = cast_ray(
            Vec3::new(5.5, 5.5, 0.5),
            Vec3::new(0.0, 0.0, 1.0),
            10.0,
            &world,
            &registry,
        );
        assert!(hit.is_none(), "ray through air should not hit anything");
    }

    #[test]
    fn ray_hits_solid_block() {
        // Sanity baseline — a single STONE block in the path is hit.
        let (mut world, registry) = empty_world();
        world.set_block(5, 5, 5, block::STONE);
        let hit = cast_ray(
            Vec3::new(5.5, 5.5, 0.5),
            Vec3::new(0.0, 0.0, 1.0),
            10.0,
            &world,
            &registry,
        );
        let hit = hit.expect("ray should hit STONE");
        assert_eq!(hit.block_pos, [5, 5, 5]);
        assert_eq!(hit.block, block::STONE);
    }

    #[test]
    fn ray_hits_flower_block() {
        // 2026-05-28 — wild flowers (CORNFLOWER/FIELD_POPPY/BUTTERCUP) are
        // declared non-solid so they don't block movement. The pre-fix
        // raycast filtered `is_solid(block)`, which meant the player's cursor
        // passed straight through every flower and the break/drop pipeline
        // never triggered. Symptom: hitting flowers with axe/fist did nothing
        // and they couldn't be collected. This test asserts that the raycast
        // *does* return a hit on a flower so the existing break-progress +
        // `growth::crop_break` path can actually run.
        for plant in [block::CORNFLOWER, block::FIELD_POPPY, block::BUTTERCUP] {
            let (mut world, registry) = empty_world();
            world.set_block(5, 5, 5, plant);
            let hit = cast_ray(
                Vec3::new(5.5, 5.5, 0.5),
                Vec3::new(0.0, 0.0, 1.0),
                10.0,
                &world,
                &registry,
            )
            .unwrap_or_else(|| panic!("ray should hit plant block {plant}"));
            assert_eq!(hit.block_pos, [5, 5, 5], "plant {plant} hit at wrong cell");
            assert_eq!(hit.block, plant, "plant {plant} returned wrong BlockId");
        }
    }

    #[test]
    fn ray_passes_through_water() {
        // Water is non-solid; it must remain non-pickable so the player can
        // see/target through it (and so the new non-solid plant-pick logic
        // doesn't accidentally start hitting water surfaces).
        let (mut world, registry) = empty_world();
        world.set_block(5, 5, 5, block::WATER);
        let hit = cast_ray(
            Vec3::new(5.5, 5.5, 0.5),
            Vec3::new(0.0, 0.0, 1.0),
            10.0,
            &world,
            &registry,
        );
        assert!(hit.is_none(), "ray must not hit WATER");
    }

    #[test]
    fn face_texel_centre_and_corners_top_face() {
        let block = [0, 0, 0];
        let top = [0, 1, 0];
        assert_eq!(face_texel([0.5, 1.0, 0.5], block, top), (8, 8), "centre");
        assert_eq!(face_texel([0.0, 1.0, 0.0], block, top), (0, 0));
        assert_eq!(face_texel([0.999, 1.0, 0.0], block, top), (15, 0));
        assert_eq!(face_texel([0.0, 1.0, 0.999], block, top), (0, 15));
        assert_eq!(face_texel([0.999, 1.0, 0.999], block, top), (15, 15));
    }

    #[test]
    fn face_texel_clamps_into_0_15() {
        let block = [5, 5, 5];
        let north = [0, 0, 1]; // ±z face → in-face axes (x, y)
        assert_eq!(face_texel([5.5, 5.5, 6.0], block, north), (8, 8));
        // An out-of-range hit point clamps rather than overflowing past 15.
        assert_eq!(face_texel([99.0, 5.5, 6.0], block, north), (15, 8));
    }

    #[test]
    fn pick_cell_centre_hits_near_face_of_a_solid_cage() {
        // Cage corner at the origin; fully solid balloon. Stand on −Z, look +Z down
        // the middle. We must hit the −Z (north) shell cell the ray enters.
        let corner = [0, 0, 0];
        let pick = pick_cell_in_cage(
            [2.0, 2.0, -5.0], // eye: centred in X/Y (cage spans 0..4), well outside on −Z
            [0.0, 0.0, 1.0],  // looking +Z
            corner,
            |c| cell_in_grid_i(c), // fully solid, un-edited balloon
        )
        .expect("ray into a solid cage hits a cell");
        assert_eq!(pick.cell[2], 0, "first solid cell is on the −Z shell");
        assert_eq!(pick.face, [0, 0, -1], "entered through the −Z face");
        assert_eq!(pick.place, [pick.cell[0], pick.cell[1], -1], "place cell is just outside on −Z");
    }

    #[test]
    fn pick_misses_when_ray_points_away() {
        let corner = [0, 0, 0];
        let pick = pick_cell_in_cage([2.0, 2.0, -5.0], [0.0, 0.0, -1.0], corner, |c| cell_in_grid_i(c));
        assert!(pick.is_none(), "ray pointing away from the cage misses");
    }

    #[test]
    fn pick_passes_through_empty_cells_to_the_first_solid() {
        // Carve the −Z shell column the ray travels so the first solid is deeper in.
        let corner = [0, 0, 0];
        // Occupied everywhere EXCEPT z==0 in the centred column the ray follows.
        let occ = |c: [i32; 3]| cell_in_grid_i(c) && !(c[2] == 0);
        let pick = pick_cell_in_cage([2.0, 2.0, -5.0], [0.0, 0.0, 1.0], corner, occ).expect("hits deeper cell");
        assert_eq!(pick.cell[2], 1, "skips the empty z==0 layer, hits z==1");
        assert_eq!(pick.face, [0, 0, -1]);
    }

    #[test]
    fn pick_respects_the_corner_offset() {
        // Same geometry shifted by a non-zero corner: the ray must be offset too.
        let corner = [10, 20, 30];
        let pick = pick_cell_in_cage(
            [10.0 + 2.0, 20.0 + 2.0, 30.0 - 5.0],
            [0.0, 0.0, 1.0],
            corner,
            |c| cell_in_grid_i(c),
        )
        .expect("hit");
        assert_eq!(pick.cell[2], 0);
        assert_eq!(pick.face, [0, 0, -1]);
    }

    #[test]
    fn pick_entry_from_minus_x() {
        let corner = [0, 0, 0];
        let pick = pick_cell_in_cage([-5.0, 2.0, 2.0], [1.0, 0.0, 0.0], corner, |c| cell_in_grid_i(c))
            .expect("enters from −X");
        assert_eq!(pick.cell[0], 0, "first solid cell is on the −X shell");
        assert_eq!(pick.face, [-1, 0, 0], "entered through the −X face");
    }

    #[test]
    fn pick_entry_from_plus_y() {
        // Above the cage looking down (−Y): enters through the +Y (top) face.
        let corner = [0, 0, 0];
        let pick = pick_cell_in_cage([2.0, 9.0, 2.0], [0.0, -1.0, 0.0], corner, |c| cell_in_grid_i(c))
            .expect("enters from +Y");
        assert_eq!(pick.cell[1], 15, "first solid cell is on the +Y (top) shell");
        assert_eq!(pick.face, [0, 1, 0], "entered through the +Y face");
    }

    // ── Phase 2 — third-person camera collision (solid-only raycast) ─────────

    #[test]
    fn cast_ray_camera_passes_through_a_plant_and_stops_at_stone() {
        // The camera collides only with occluders — a flower (non-solid →
        // PassThrough) in the path is ignored; the opaque stone behind it stops it.
        let (mut world, registry) = empty_world();
        world.set_block(5, 5, 5, block::CORNFLOWER); // non-solid, pickable by the cursor
        world.set_block(5, 5, 7, block::STONE);
        let hit = cast_ray_camera(
            Vec3::new(5.5, 5.5, 0.5),
            Vec3::new(0.0, 0.0, 1.0),
            20.0,
            &world,
            &registry,
        )
        .expect("camera ray stops at the opaque stone");
        assert_eq!(hit.block_pos, [5, 5, 7], "skips the flower, collides with stone");
        assert_eq!(hit.block, block::STONE);
    }

    #[test]
    fn cast_ray_camera_ignores_water() {
        // Water is non-solid; it must never push the third-person camera.
        let (mut world, registry) = empty_world();
        world.set_block(5, 5, 5, block::WATER);
        let hit = cast_ray_camera(
            Vec3::new(5.5, 5.5, 0.5),
            Vec3::new(0.0, 0.0, 1.0),
            10.0,
            &world,
            &registry,
        );
        assert!(hit.is_none(), "camera ray must pass through water");
    }

    #[test]
    fn cast_ray_camera_passes_through_glass_but_stops_at_stone_behind_it() {
        // Phase 4 — GLASS is solid (blocks movement) but transparent → PassThrough
        // for the camera: it must see through it and stop at the opaque stone
        // behind. (`is_solid` would wrongly stop at the glass — the MC bug.)
        let (mut world, registry) = empty_world();
        world.set_block(5, 5, 5, block::GLASS);
        world.set_block(5, 5, 8, block::STONE);
        let hit = cast_ray_camera(
            Vec3::new(5.5, 5.5, 0.5),
            Vec3::new(0.0, 0.0, 1.0),
            20.0,
            &world,
            &registry,
        )
        .expect("camera ray passes the glass and stops at stone");
        assert_eq!(hit.block_pos, [5, 5, 8], "glass is see-through → camera passes it");
        assert_eq!(hit.block, block::STONE);
    }

    #[test]
    fn camera_collision_fraction_full_when_clear_clamps_for_a_wall_behind() {
        // Orbit-behind: looking -Z, the camera pulls toward +Z. With nothing
        // behind, the fraction is full (1.0); a wall within the orbit distance
        // clamps it below 1.0 (render-only — aim is untouched).
        let (mut world, registry) = empty_world();
        let mut cam = crate::camera::Camera::new(Vec3::new(5.5, 5.5, 5.5), 1.0);
        cam.mode = crate::camera::CameraMode::OrbitBehind;
        assert_eq!(camera_collision_fraction(&cam, &world, &registry), 1.0, "clear path → full");

        world.set_block(5, 5, 7, block::STONE); // ~1.5 behind, inside the 4-block orbit
        let f = camera_collision_fraction(&cam, &world, &registry);
        assert!(f < 1.0 && f >= 0.0, "wall behind clamps the camera, got {f}");
    }

    #[test]
    fn camera_collision_fraction_is_full_in_first_person() {
        // No pull-back offset → no clamp, even with a block right behind the eye.
        let (mut world, registry) = empty_world();
        world.set_block(5, 5, 7, block::STONE);
        let cam = crate::camera::Camera::new(Vec3::new(5.5, 5.5, 5.5), 1.0); // FirstPerson
        assert_eq!(camera_collision_fraction(&cam, &world, &registry), 1.0);
    }

    #[test]
    fn pick_origin_inside_cage_uses_dominant_axis_for_face() {
        // Eye inside the cage, looking +Z. Dominant axis is Z so the entry face is −Z.
        let corner = [0, 0, 0];
        let pick = pick_cell_in_cage([2.0, 2.0, 2.0], [0.0, 0.0, 1.0], corner, |c| cell_in_grid_i(c))
            .expect("inside a solid cage, always hits");
        assert_eq!(pick.face, [0, 0, -1], "dominant-axis face must be −Z when looking +Z");
        assert_eq!(pick.cell[2], 8, "first hit is the eye's own cell (z=8 for eye at z=2.0)");
    }
}

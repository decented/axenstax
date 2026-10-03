//! Rail freight (Phase 1) — the TRACK block and, in later tasks, the carts
//! that roll along it. Task 1.1 is the block only: a flat, non-solid
//! (walk-through) rail that renders as a thin slab on the ground.
//!
//! Spec: `docs/foundations/2026-06-09-rail-freight-logistics.md`.

/// Flat directional rail block — you walk *over* it, not into it, and a cart
/// (later tasks) rolls along it. Non-solid + transparent so the renderer
/// treats it as a thin ground slab via the small-cube path in `mesh.rs`.
pub const TRACK: crate::block::BlockId = 260; // confirmed next-free id

/// The flat visual shape a rail resolves to, derived live from its four cardinal
/// rail neighbours. No stored state — recomputed at mesh time (like Wall/Pane).
/// `CornerNE` joins the north and east edges, etc.
///
/// Task 1.1 (this file) is the block only, rendered as a uniform flat slab —
/// the mesher doesn't call `rail_shape_from_mask`/`rail_neighbour_mask` yet to
/// pick a directional shape; that's a later Rail Freight task. Tested below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum RailShape {
    StraightNS,
    StraightEW,
    CornerNE,
    CornerNW,
    CornerSE,
    CornerSW,
}

/// Map a 4-bit rail-neighbour mask (`CONN_*` bits) to a `RailShape`.
///
/// Mirrors Minecraft's placement rule: one neighbour → straight toward it; two
/// opposite → straight; two perpendicular → the matching corner; a T/cross →
/// a corner chosen by preference order **west → east → south → north**.
#[cfg_attr(not(test), allow(dead_code))]
pub fn rail_shape_from_mask(mask: u8) -> RailShape {
    use crate::block_shape::{CONN_E, CONN_N, CONN_S, CONN_W};
    // Preference: west beats east (horizontal), south beats north (vertical).
    let horiz = if mask & CONN_W != 0 {
        Some(false) // west
    } else if mask & CONN_E != 0 {
        Some(true) // east
    } else {
        None
    };
    let vert = if mask & CONN_S != 0 {
        Some(true) // south
    } else if mask & CONN_N != 0 {
        Some(false) // north
    } else {
        None
    };
    match (horiz, vert) {
        (Some(is_east), Some(is_south)) => match (is_south, is_east) {
            (false, false) => RailShape::CornerNW,
            (false, true) => RailShape::CornerNE,
            (true, false) => RailShape::CornerSW,
            (true, true) => RailShape::CornerSE,
        },
        (Some(_), None) => RailShape::StraightEW,
        (None, Some(_)) => RailShape::StraightNS,
        (None, None) => RailShape::StraightNS,
    }
}

/// Build the 4-bit connection mask for the cell at (x,y,z): one bit per
/// cardinal neighbour for which `member(neighbour_block)` is true. Same-level only.
pub fn neighbour_link_mask(
    world: &crate::world::World,
    x: i32, y: i32, z: i32,
    member: impl Fn(crate::block::BlockId) -> bool,
) -> u8 {
    use crate::block_shape::{CONN_E, CONN_N, CONN_S, CONN_W};
    let mut m = 0u8;
    if member(world.get_block(x, y, z - 1)) { m |= CONN_N; }
    if member(world.get_block(x, y, z + 1)) { m |= CONN_S; }
    if member(world.get_block(x - 1, y, z)) { m |= CONN_W; }
    if member(world.get_block(x + 1, y, z)) { m |= CONN_E; }
    m
}

/// Track connection mask: cardinal neighbours that are also `TRACK`. No
/// caller yet (see `RailShape` above) and no test.
#[allow(dead_code)]
pub fn rail_neighbour_mask(world: &crate::world::World, x: i32, y: i32, z: i32) -> u8 {
    neighbour_link_mask(world, x, y, z, |b| b == TRACK)
}

/// A grid cell address (x, y, z). Used for pure track-path logic; no world
/// access — `is_track` is injected as a closure so the functions are fully
/// unit-testable without touching the ECS or `World`.
pub type Cell = (i32, i32, i32);

/// The 4 horizontal (XZ-plane) neighbours of a cell. Y is unchanged — vertical
/// / sloped rail steps are not modelled in Phase 1.
pub fn h_neighbours(c: Cell) -> [Cell; 4] {
    [
        (c.0 + 1, c.1, c.2),
        (c.0 - 1, c.1, c.2),
        (c.0, c.1, c.2 + 1),
        (c.0, c.1, c.2 - 1),
    ]
}

/// Every rail cell a cart could roll to from `c`: the four horizontal neighbours
/// plus, for each, one step UP and one step DOWN — the 45° ascending/descending
/// ramps. A staircase of rails (a rail one block up-and-over on each step)
/// auto-links into a continuous slope; a flat run only ever finds its flat
/// neighbours (the up/down cells are air), so flat behaviour is unchanged.
pub fn linked_neighbours(c: Cell) -> [Cell; 12] {
    let mut out = [(0, 0, 0); 12];
    let mut i = 0;
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        for dy in [0, 1, -1] {
            out[i] = (c.0 + dx, c.1 + dy, c.2 + dz);
            i += 1;
        }
    }
    out
}

/// If this rail cell ramps UP toward a horizontal direction — i.e. there is a
/// rail exactly one block up in that direction — returns that direction
/// `(dx, dz)`; `None` means a flat rail. Used only to pick the tilted ramp mesh;
/// carts follow ramps purely via `linked_neighbours`/`next_track_step`.
pub fn rail_ascend_dir(is_track: impl Fn(Cell) -> bool, at: Cell) -> Option<(i32, i32)> {
    let mut found = None;
    let mut count = 0usize;
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        if is_track((at.0 + dx, at.1 + 1, at.2 + dz)) {
            count += 1;
            found = Some((dx, dz));
        }
    }
    // A single up-neighbour = a clean ramp toward it. Two (a peak/valley) is
    // ambiguous — fall back to flat rather than guess.
    if count == 1 {
        found
    } else {
        None
    }
}

/// Advance one step along a track.
///
/// Given the cell a cart is currently on (`at`) and the cell it came from
/// (`came_from`), returns the next track cell to roll to.
///
/// * Dead-end (terminus) → `None`.
/// * Junction (more than one onward track neighbour) → `None` for the Phase 1
///   MVP (carts stop at junctions; no switching logic yet).
pub fn next_track_step(
    at: Cell,
    came_from: Option<Cell>,
    is_track: impl Fn(Cell) -> bool,
) -> Option<Cell> {
    // Allocation-free "exactly one onward neighbour" check — runs per cart per
    // tick once carts move, so we avoid a per-step Vec on this hot path.
    let mut only: Option<Cell> = None;
    let mut count = 0usize;
    for n in linked_neighbours(at) {
        if is_track(n) && Some(n) != came_from {
            count += 1;
            only = Some(n);
        }
    }
    if count == 1 {
        only
    } else {
        None // 0 = terminus, >1 = junction (no switching in Phase 1)
    }
}

/// The depot chest for a track terminus, if any.
///
/// A **depot is just a chest placed in one of the terminus's four horizontal
/// neighbours** — no dedicated block. Returns the first adjacent cell for which
/// `is_chest` is true (deterministic order from [`h_neighbours`]: +X, -X, +Z,
/// -Z), or `None` when the terminus has no chest beside it.
///
/// Pure: `is_chest` is injected as a closure exactly like `is_track`, so the
/// load/unload wiring (Task 1.4) is fully unit-testable without `World`.
pub fn depot_chest_for(term: Cell, is_chest: impl Fn(Cell) -> bool) -> Option<Cell> {
    h_neighbours(term).into_iter().find(|n| is_chest(*n))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Treat the given cells as the only track blocks, everything else air.
    fn line(cells: &[Cell]) -> impl Fn(Cell) -> bool + '_ {
        move |c| cells.contains(&c)
    }

    #[test]
    fn runs_straight_until_terminus() {
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        assert_eq!(next_track_step((0, 0, 0), None, line(&l)), Some((1, 0, 0)));
        assert_eq!(
            next_track_step((1, 0, 0), Some((0, 0, 0)), line(&l)),
            Some((2, 0, 0))
        );
        assert_eq!(next_track_step((2, 0, 0), Some((1, 0, 0)), line(&l)), None); // terminus
    }

    #[test]
    fn follows_a_corner() {
        let l = [(0, 0, 0), (1, 0, 0), (1, 0, 1)]; // east then north
        assert_eq!(
            next_track_step((1, 0, 0), Some((0, 0, 0)), line(&l)),
            Some((1, 0, 1))
        );
    }

    #[test]
    fn rolls_up_a_ramp() {
        // A staircase east: flat (0,0,0), then a rail one up-and-over each step.
        let l = [(0, 0, 0), (1, 1, 0), (2, 2, 0)];
        // From the flat cell heading east, the onward step is the higher rail.
        assert_eq!(
            next_track_step((0, 0, 0), Some((-1, 0, 0)), line(&l)),
            Some((1, 1, 0))
        );
        // Continue up the slope.
        assert_eq!(
            next_track_step((1, 1, 0), Some((0, 0, 0)), line(&l)),
            Some((2, 2, 0))
        );
    }

    #[test]
    fn rolls_down_a_ramp() {
        // Same staircase, travelling the other way (came from above).
        let l = [(0, 0, 0), (1, 1, 0), (2, 2, 0)];
        assert_eq!(
            next_track_step((2, 2, 0), Some((3, 3, 0)), line(&l)),
            Some((1, 1, 0))
        );
        assert_eq!(
            next_track_step((1, 1, 0), Some((2, 2, 0)), line(&l)),
            Some((0, 0, 0))
        );
    }

    #[test]
    fn ascend_dir_detects_the_up_slope() {
        // A rail one block up to the east -> this cell ramps east.
        let l = [(0, 0, 0), (1, 1, 0)];
        assert_eq!(rail_ascend_dir(line(&l), (0, 0, 0)), Some((1, 0)));
        // A flat pair -> no ramp.
        let flat = [(0, 0, 0), (1, 0, 0)];
        assert_eq!(rail_ascend_dir(line(&flat), (0, 0, 0)), None);
    }

    #[test]
    fn junction_stops_for_mvp() {
        let l = [(1, 0, 0), (0, 0, 0), (2, 0, 0), (1, 0, 1)]; // 3 ways out of (1,0,0)
        assert_eq!(next_track_step((1, 0, 0), Some((1, 0, 1)), line(&l)), None);
    }

    #[test]
    fn came_from_none_mid_line_stops() {
        // A cart with no direction history (`came_from = None`) sitting on the
        // MIDDLE of a straight line has TWO onward neighbours, so it can't pick
        // one and stays put. Expected Phase 1 behaviour: a dispatched cart must
        // be seeded with `came_from` (or placed at a terminus) to get going —
        // Task 1.3 supplies the initial direction on dispatch.
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        assert_eq!(next_track_step((1, 0, 0), None, line(&l)), None);
    }

    #[test]
    fn finds_adjacent_depot() {
        let chest = (3, 0, 0);
        assert_eq!(depot_chest_for((2, 0, 0), |c| c == chest), Some(chest));
    }

    #[test]
    fn no_depot_when_no_adjacent_chest() {
        // A terminus with no chest neighbour resolves to None.
        assert_eq!(depot_chest_for((2, 0, 0), |_| false), None);
    }

    #[test]
    fn track_block_is_non_solid_walkthrough() {
        let reg = crate::block::BlockRegistry::new();
        let def = reg.get(TRACK);
        assert_eq!(def.name, "genesis:track");
        assert!(!def.solid, "you walk over track, not into it");
        assert!(def.transparent);
    }

    #[test]
    fn lone_rail_defaults_to_north_south() {
        assert_eq!(rail_shape_from_mask(0), RailShape::StraightNS);
    }

    #[test]
    fn single_and_opposite_neighbours_are_straight() {
        use crate::block_shape::{CONN_N, CONN_S, CONN_W, CONN_E};
        assert_eq!(rail_shape_from_mask(CONN_N), RailShape::StraightNS);
        assert_eq!(rail_shape_from_mask(CONN_S), RailShape::StraightNS);
        assert_eq!(rail_shape_from_mask(CONN_N | CONN_S), RailShape::StraightNS);
        assert_eq!(rail_shape_from_mask(CONN_E), RailShape::StraightEW);
        assert_eq!(rail_shape_from_mask(CONN_W), RailShape::StraightEW);
        assert_eq!(rail_shape_from_mask(CONN_E | CONN_W), RailShape::StraightEW);
    }

    #[test]
    fn perpendicular_neighbours_form_the_matching_corner() {
        use crate::block_shape::{CONN_N, CONN_S, CONN_W, CONN_E};
        assert_eq!(rail_shape_from_mask(CONN_N | CONN_E), RailShape::CornerNE);
        assert_eq!(rail_shape_from_mask(CONN_N | CONN_W), RailShape::CornerNW);
        assert_eq!(rail_shape_from_mask(CONN_S | CONN_E), RailShape::CornerSE);
        assert_eq!(rail_shape_from_mask(CONN_S | CONN_W), RailShape::CornerSW);
    }

    #[test]
    fn junctions_pick_a_corner_by_preference_west_east_south_north() {
        use crate::block_shape::{CONN_N, CONN_S, CONN_W, CONN_E};
        // T-junction N+S+E: horizontal pick E (no W), vertical pick S -> SE.
        assert_eq!(rail_shape_from_mask(CONN_N | CONN_S | CONN_E), RailShape::CornerSE);
        // T-junction N+S+W: horizontal pick W, vertical pick S -> SW.
        assert_eq!(rail_shape_from_mask(CONN_N | CONN_S | CONN_W), RailShape::CornerSW);
        // Cross N+S+E+W: W beats E, S beats N -> SW.
        assert_eq!(
            rail_shape_from_mask(CONN_N | CONN_S | CONN_E | CONN_W),
            RailShape::CornerSW
        );
    }
}

//! Spec 48 (Electricity) Phase 2 — beam routing for the Beam Sensor + Mirror.
//!
//! The raycast + mirror reflection are **pure**: they take an injected cell
//! classifier (`Fn(Pos) -> BeamCell`), so they unit-test with no `World`. The
//! power tick wires `classify` to the world (reading block ids + the Mirror's
//! meta orientation) and then runs entity-crossing detection over the returned
//! path — reusing the same entity-position scan the Pressure Plate already does.
//!
//! Arming modes (per the locked cosmology mechanics):
//!   * **through-beam** — the beam reaches a *second* sensor, and
//!   * **retroreflective** — the beam bounces off a 180° Mirror and returns to
//!     its *origin* sensor (the origin cell classifies as `Sensor`).
//!     A 90° Mirror turns the beam; a beam that hits anything opaque (or runs out of
//!     range) dead-ends and does NOT arm. While armed, an entity crossing any path
//!     cell breaks the beam → the sensor triggers (drives power like any input).

use crate::meta::Facing;

pub type Pos = (i32, i32, i32);

/// What a cell does to a travelling beam — resolved from the world by the caller
/// (block id + the Mirror's meta orientation), so the trace itself stays pure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeamCell {
    /// Empty / transparent — the beam passes through.
    Pass,
    /// A beam sensor — a valid terminal (a paired through-beam sensor, or the
    /// origin sensor a retroreflected beam returns to).
    Sensor,
    /// A Mirror in 180° mode — reflects the beam straight back the way it came.
    Retroreflector,
    /// A Mirror in 90° mode — the beam leaves in this new direction.
    Turn(Facing),
    /// Anything opaque — the beam dies here (no arm).
    Block,
}

/// Result of tracing a beam from a sensor.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BeamTrace {
    /// Every cell the beam passes through, in travel order, EXCLUDING the origin
    /// sensor cell. These are the cells an entity can break by crossing.
    pub cells: Vec<Pos>,
    /// True iff the beam reached a valid terminal (a `Sensor` cell) within range.
    pub armed: bool,
}

/// Max beam length in cells before it gives up unarmed. Bounds the raycast so a
/// mirror loop (e.g. two retroreflectors facing each other) can't spin forever.
pub const MAX_BEAM_LEN: usize = 64;

/// Trace a beam from `origin` heading `dir`, consulting `classify` for each cell
/// the beam enters (the origin cell itself is not classified). Pure. Returns the
/// path travelled + whether it armed (reached a `Sensor` terminal).
pub fn trace_beam(origin: Pos, dir: Facing, classify: impl Fn(Pos) -> BeamCell) -> BeamTrace {
    let mut pos = origin;
    let mut d = dir;
    let mut cells = Vec::new();
    for _ in 0..MAX_BEAM_LEN {
        let (dx, dy, dz) = d.offset();
        pos = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
        match classify(pos) {
            BeamCell::Pass => cells.push(pos),
            BeamCell::Sensor => {
                cells.push(pos);
                return BeamTrace { cells, armed: true };
            }
            BeamCell::Retroreflector => {
                cells.push(pos);
                d = d.opposite();
            }
            BeamCell::Turn(new_d) => {
                cells.push(pos);
                d = new_d;
            }
            BeamCell::Block => return BeamTrace { cells, armed: false },
        }
    }
    BeamTrace { cells, armed: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a classifier from an explicit cell map; everything else is `Pass`.
    fn classifier(map: Vec<(Pos, BeamCell)>) -> impl Fn(Pos) -> BeamCell {
        move |p| {
            map.iter()
                .find(|(q, _)| *q == p)
                .map(|(_, c)| *c)
                .unwrap_or(BeamCell::Pass)
        }
    }

    #[test]
    fn through_beam_arms_at_a_paired_sensor() {
        // Origin faces East; a second sensor sits 3 cells East.
        let c = classifier(vec![((3, 0, 0), BeamCell::Sensor)]);
        let t = trace_beam((0, 0, 0), Facing::East, c);
        assert!(t.armed, "through-beam reaches the paired sensor");
        assert_eq!(t.cells, vec![(1, 0, 0), (2, 0, 0), (3, 0, 0)]);
    }

    #[test]
    fn retroreflector_returns_to_origin_and_arms() {
        // A 180° mirror 3 East; the origin (0,0,0) is itself a sensor.
        let c = classifier(vec![
            ((3, 0, 0), BeamCell::Retroreflector),
            ((0, 0, 0), BeamCell::Sensor),
        ]);
        let t = trace_beam((0, 0, 0), Facing::East, c);
        assert!(t.armed, "retroreflected beam returns to the origin sensor");
    }

    #[test]
    fn ninety_degree_mirror_turns_the_beam_to_a_sensor() {
        // East to a 90° mirror at (3,0,0) that turns the beam North (-Z); a
        // sensor sits 3 cells North of the mirror.
        let c = classifier(vec![
            ((3, 0, 0), BeamCell::Turn(Facing::North)),
            ((3, 0, -3), BeamCell::Sensor),
        ]);
        let t = trace_beam((0, 0, 0), Facing::East, c);
        assert!(t.armed, "turned beam reaches the sensor");
        assert!(t.cells.contains(&(3, 0, 0)), "passes the mirror");
        assert!(t.cells.contains(&(3, 0, -3)), "reaches the turned-to sensor");
    }

    #[test]
    fn dead_end_into_a_block_does_not_arm() {
        let c = classifier(vec![((3, 0, 0), BeamCell::Block)]);
        let t = trace_beam((0, 0, 0), Facing::East, c);
        assert!(!t.armed);
        assert_eq!(t.cells, vec![(1, 0, 0), (2, 0, 0)], "stops before the block");
    }

    #[test]
    fn open_beam_gives_up_unarmed_at_max_range() {
        let c = classifier(vec![]);
        let t = trace_beam((0, 0, 0), Facing::East, c);
        assert!(!t.armed, "a beam that never finds a terminal does not arm");
        assert_eq!(t.cells.len(), MAX_BEAM_LEN);
    }
}

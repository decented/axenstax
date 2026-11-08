//! Probes (living-world spec §1.5): a named box whose bloom is worked out over
//! exactly its columns instead of whole regions. The trial `Place` objective
//! reads one.

use glam::IVec3;

use super::eco::EcoTally;
use super::level::{self, Stage};
use super::people::{self, CommonsArea, FeastMarker, PeopleState, PRESENCE_MARGIN};

/// A probe's name.
pub type ProbeId = u32;

/// Default goal for a probe nobody gave one: the Grey Quarry's.
const DEFAULT_AT_LEAST: f32 = 0.55;
const DEFAULT_HOLD_SECS: f32 = 20.0;
/// Ticks between two scans of a probe (once a second at 20 TPS).
pub const PROBE_SCAN_TICKS: u64 = 20;

/// What a probe reads right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeState {
    pub level: f32,
    pub target: f32,
    /// The life term.
    pub eco: f32,
    /// The people-together term `P`.
    pub people: f32,
    pub stage: Stage,
    /// Seconds the level has stayed at or above the goal.
    pub held_secs: f32,
    /// `false` until the probe's first scan: every number is a placeholder.
    pub scanned: bool,
}

pub(super) struct Probe {
    pub id: ProbeId,
    pub min: IVec3,
    pub max: IVec3,
    pub at_least: f32,
    pub hold_secs: f32,
    pub tally: EcoTally,
    pub wildlife: u32,
    pub eco: f32,
    pub people: PeopleState,
    pub floor: f32,
    pub commons: bool,
    pub level: f32,
    pub target: f32,
    pub held: f32,
    pub scanned: bool,
    pub next_scan: u64,
}

impl Probe {
    pub fn new(id: ProbeId, a: IVec3, b: IVec3) -> Self {
        Self {
            id,
            min: a.min(b),
            max: a.max(b),
            at_least: DEFAULT_AT_LEAST,
            hold_secs: DEFAULT_HOLD_SECS,
            tally: EcoTally::default(),
            wildlife: 0,
            eco: 0.0,
            people: PeopleState::default(),
            floor: 0.0,
            commons: false,
            level: level::UNKNOWN_LEVEL,
            target: level::UNKNOWN_LEVEL,
            held: 0.0,
            scanned: false,
            next_scan: 0,
        }
    }

    pub fn contains_column(&self, x: i32, z: i32) -> bool {
        x >= self.min.x && x <= self.max.x && z >= self.min.z && z <= self.max.z
    }

    /// Does the probe's box overlap the block rectangle (inclusive, x and z)?
    pub fn overlaps(&self, min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> bool {
        self.min.x <= max_x && self.max.x >= min_x && self.min.z <= max_z && self.max.z >= min_z
    }

    /// Is a player at `(x, z)` near enough to count as present here?
    pub fn player_present(&self, x: f32, z: f32) -> bool {
        x >= self.min.x as f32 - PRESENCE_MARGIN
            && x < (self.max.x + 1) as f32 + PRESENCE_MARGIN
            && z >= self.min.z as f32 - PRESENCE_MARGIN
            && z < (self.max.z + 1) as f32 + PRESENCE_MARGIN
    }

    /// Refresh the Feast floor and Commons flag from the current inputs.
    pub fn refresh_inputs(&mut self, feasts: &[FeastMarker], commons: &[CommonsArea]) {
        self.floor = people::marker_floor(
            feasts.iter().filter(|m| self.contains_column(m.x, m.z)).map(|m| m.guests),
        );
        self.commons = commons
            .iter()
            .any(|c| c.overlaps(self.min.x, self.max.x, self.min.z, self.max.z));
    }

    pub fn recompute_eco(&mut self) {
        self.eco = self.tally.eco(self.wildlife);
    }

    /// `P` right now.
    pub fn people_value(&self) -> f32 {
        self.people.value(self.floor, self.commons)
    }

    /// Advance one tick: people, target, easing, the held clock.
    pub fn step(&mut self, present: usize, creative: bool, dt: f32, scale: f32) {
        self.people.step(present, self.commons, dt);
        self.target = level::target(self.eco, self.people_value(), creative);
        if self.scanned {
            self.level = level::ease(self.level, self.target, dt, scale);
        }
        if self.scanned && self.level >= self.at_least {
            self.held += dt;
        } else {
            self.held = 0.0;
        }
    }

    pub fn state(&self) -> ProbeState {
        ProbeState {
            level: self.level,
            target: self.target,
            eco: self.eco,
            people: self.people_value(),
            stage: Stage::of(self.level, self.at_least, self.held, self.hold_secs),
            held_secs: self.held,
            scanned: self.scanned,
        }
    }
}

//! `BloomField`: the bloom of every region near the local players, worked out
//! on this client from what it can see (living-world spec §1).
//!
//! Presentation only. It reads the world, the ECS and player positions; it
//! writes nothing to any of them, sends nothing and saves nothing.

use std::collections::HashMap;

use glam::{IVec3, Vec3};

use super::eco::EcoTally;
use super::kinds::counts_as_wildlife;
use super::level::{self, GREY_BELOW, UNKNOWN_LEVEL};
use super::natural::NaturalCache;
use super::people::{self, CommonsArea, FeastMarker, PeopleState};
use super::probe::{Probe, ProbeId, ProbeState, PROBE_SCAN_TICKS};
use super::scan::{scan_columns, RegionChunks};
use super::RegionKey;
use crate::biome::BiomeGenerator;
use crate::block::BlockRegistry;
use crate::chunk::CHUNK_SIZE;
use crate::mob::MobType;
use crate::world::World;

/// A region is tracked while it is within this many chunks of a local player.
pub const TRACK_RADIUS: i32 = 8;
/// Regions re-scanned per tick, oldest first. A full sweep of the 289 tracked
/// regions then takes about four seconds at 20 TPS.
pub const SCANS_PER_TICK: usize = 4;
/// Ticks between passive-mob censuses.
pub const CENSUS_TICKS: u64 = 20;
/// Natural-terrain regenerations per tick (see `natural.rs`).
const NATURAL_GENS_PER_TICK: u32 = 1;
/// Seconds after the field starts before `PlaceWentGrey` may fire, so that
/// loading a world never counts as an event.
pub const GREY_GRACE_SECS: f32 = 60.0;
/// Cells per side of [`BloomField::grid_around`].
pub const GRID_SIDE: usize = 16;

const REGION: i32 = CHUNK_SIZE as i32;

/// Everything one tick reads.
pub struct TickInput<'a> {
    pub world: &'a World,
    pub ecs: &'a hecs::World,
    pub biome_gen: &'a BiomeGenerator,
    pub registry: &'a BlockRegistry,
    /// Local (split-screen) seats; regions are tracked round these.
    pub local_players: &'a [Vec3],
    /// Every other player this client knows the position of.
    pub remote_players: &'a [Vec3],
    pub creative: bool,
    /// Seconds this tick stands for.
    pub dt: f32,
    /// May `PlaceWentGrey` be reported? False in a trial arena world.
    pub allow_grey_event: bool,
}

/// What a tick reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickOutput {
    /// A place in this world went grey for the first time: fire the event.
    pub place_went_grey: bool,
}

/// Averages over the known regions (the Tally's "world bloom").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldSummary {
    pub mean_level: f32,
    pub mean_eco: f32,
    pub mean_people: f32,
    pub regions: usize,
}

struct Region {
    tally: EcoTally,
    wildlife: u32,
    eco: f32,
    people: PeopleState,
    floor: f32,
    commons: bool,
    /// Built together (Tier 2 S2): from the region's distinct recorded authors,
    /// refreshed with each region scan, not every tick.
    built: f32,
    /// Lived in (Tier 2 S3): from the region's use record, refreshed with each
    /// region scan like `built`.
    lived: f32,
    /// A tended shelf (Tier 2 S6): from the Commons Shelves in the region's
    /// blocks and meta, refreshed with each region scan. Unlike `built` and
    /// `lived` it needs nothing the authority alone keeps, so a joined client
    /// sees it too.
    shelf: crate::shelf::ShelfTerms,
    level: f32,
    target: f32,
    last_scan: Option<u64>,
    known: bool,
}

impl Region {
    fn new() -> Self {
        Self {
            tally: EcoTally::default(),
            wildlife: 0,
            eco: 0.0,
            people: PeopleState::default(),
            floor: 0.0,
            commons: false,
            built: 0.0,
            lived: 0.0,
            shelf: crate::shelf::ShelfTerms::default(),
            level: UNKNOWN_LEVEL,
            target: UNKNOWN_LEVEL,
            last_scan: None,
            known: false,
        }
    }

    fn p(&self) -> f32 {
        self.people.value_all(self.floor, self.commons, self.built, self.lived, self.shelf.s())
    }

    /// Where this region's level is heading: the people term lifts it, and a
    /// looted shelf dims it.
    fn target_now(&self, creative: bool) -> f32 {
        level::target_scaled(self.eco, self.p(), creative, self.shelf.target_scale())
    }
}

fn block_rect(key: RegionKey) -> (i32, i32, i32, i32) {
    (key.0 * REGION, key.0 * REGION + REGION - 1, key.1 * REGION, key.1 * REGION + REGION - 1)
}

pub struct BloomField {
    regions: HashMap<RegionKey, Region>,
    natural: NaturalCache,
    probes: Vec<Probe>,
    /// Passive wildlife `(x, z, kind)`, from the last census.
    census: Vec<(f32, f32, MobType)>,
    feasts: Vec<FeastMarker>,
    commons: Vec<CommonsArea>,
    initial: Vec<(IVec3, IVec3, f32)>,
    time_scale: f32,
    tick_no: u64,
    age_secs: f32,
    grey_fired: bool,
    creative: bool,
}

impl Default for BloomField {
    fn default() -> Self {
        Self::new()
    }
}

impl BloomField {
    pub fn new() -> Self {
        Self {
            regions: HashMap::new(),
            natural: NaturalCache::default(),
            probes: Vec::new(),
            census: Vec::new(),
            feasts: Vec::new(),
            commons: Vec::new(),
            initial: Vec::new(),
            time_scale: 1.0,
            tick_no: 0,
            age_secs: 0.0,
            grey_fired: false,
            creative: false,
        }
    }

    /// Forget everything: a new world was entered.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    // ── Read API ─────────────────────────────────────────────────────

    /// The level (0..=1) of the region holding `(x, z)`; 0.6 where unknown.
    pub fn level_at(&self, x: f32, z: f32) -> f32 {
        let key = ((x / REGION as f32).floor() as i32, (z / REGION as f32).floor() as i32);
        self.known_level(key).unwrap_or(UNKNOWN_LEVEL)
    }

    fn known_level(&self, key: RegionKey) -> Option<f32> {
        self.regions.get(&key).filter(|r| r.known).map(|r| r.level)
    }

    /// A 16 x 16 grid of region levels round `(cx, cz)`, row-major in z then x,
    /// and the world x and z of its min corner. Cell `(i, j)` is the region whose
    /// min corner is `(origin_x + 16 i, origin_z + 16 j)`; unknown cells are 0.6.
    pub fn grid_around(&self, cx: f32, cz: f32) -> ([f32; GRID_SIDE * GRID_SIDE], f32, f32) {
        let half = (GRID_SIDE / 2) as i32;
        let rx = (cx / REGION as f32).floor() as i32 - half;
        let rz = (cz / REGION as f32).floor() as i32 - half;
        let mut grid = [UNKNOWN_LEVEL; GRID_SIDE * GRID_SIDE];
        for j in 0..GRID_SIDE {
            for i in 0..GRID_SIDE {
                if let Some(l) = self.known_level((rx + i as i32, rz + j as i32)) {
                    grid[j * GRID_SIDE + i] = l;
                }
            }
        }
        (grid, (rx * REGION) as f32, (rz * REGION) as f32)
    }

    /// Means over the regions that have been scanned. All zero when none has.
    pub fn world_summary(&self) -> WorldSummary {
        let known: Vec<&Region> = self.regions.values().filter(|r| r.known).collect();
        if known.is_empty() {
            return WorldSummary::default();
        }
        let n = known.len() as f32;
        WorldSummary {
            mean_level: known.iter().map(|r| r.level).sum::<f32>() / n,
            mean_eco: known.iter().map(|r| r.eco).sum::<f32>() / n,
            mean_people: known.iter().map(|r| r.p()).sum::<f32>() / n,
            regions: known.len(),
        }
    }

    // ── Probes ───────────────────────────────────────────────────────

    /// Add (or replace) a probe over the box `min..=max`. Only x and z matter.
    pub fn add_probe(&mut self, id: ProbeId, min: IVec3, max: IVec3) {
        let mut p = Probe::new(id, min, max);
        p.refresh_inputs(&self.feasts, &self.commons);
        self.probes.retain(|q| q.id != id);
        self.probes.push(p);
    }

    /// Set a probe's goal: the level it must reach and how long (seconds) it
    /// must stay there to read "Alive".
    pub fn set_probe_goal(&mut self, id: ProbeId, at_least: f32, hold_secs: f32) {
        if let Some(p) = self.probes.iter_mut().find(|p| p.id == id) {
            p.at_least = at_least;
            p.hold_secs = hold_secs;
        }
    }

    pub fn remove_probe(&mut self, id: ProbeId) {
        self.probes.retain(|p| p.id != id);
    }

    pub fn probe(&self, id: ProbeId) -> Option<ProbeState> {
        self.probes.iter().find(|p| p.id == id).map(Probe::state)
    }

    // ── Inputs from elsewhere ────────────────────────────────────────

    /// The world's bloom time scale (1.0 normally; trials set it).
    pub fn set_time_scale(&mut self, scale: f32) {
        self.time_scale = scale.max(0.0);
    }

    /// Start every region and probe overlapping the box at `level` instead of
    /// at its target, when it is first scanned (authored grey starts). Applies
    /// at once to anything already scanned.
    pub fn set_initial_level(&mut self, min: IVec3, max: IVec3, level: f32) {
        let (lo, hi) = (min.min(max), min.max(max));
        let level = level.clamp(0.0, 1.0);
        // The same box again (a Place trial's retry) replaces its level.
        match self.initial.iter_mut().find(|(l, h, _)| *l == lo && *h == hi) {
            Some(entry) => entry.2 = level,
            None => self.initial.push((lo, hi, level)),
        }
        for (&key, r) in &mut self.regions {
            let (x0, x1, z0, z1) = block_rect(key);
            if r.known && lo.x <= x1 && hi.x >= x0 && lo.z <= z1 && hi.z >= z0 {
                r.level = level;
            }
        }
        for p in &mut self.probes {
            if p.scanned && p.overlaps(lo.x, hi.x, lo.z, hi.z) {
                p.level = level;
            }
        }
    }

    /// A Feast was held at `(x, z)` for `guests` guests: a surge in the region
    /// (and any probe) there.
    pub fn note_feast(&mut self, x: i32, z: i32, guests: u8) {
        let key = (x.div_euclid(REGION), z.div_euclid(REGION));
        if let Some(r) = self.regions.get_mut(&key) {
            r.people.note_feast(guests);
        }
        for p in self.probes.iter_mut().filter(|p| p.contains_column(x, z)) {
            p.people.note_feast(guests);
        }
    }

    /// Replace the list of Feast Markers standing in the loaded world.
    pub fn set_feast_markers(&mut self, markers: &[(i32, i32, u8)]) {
        self.feasts = markers.iter().copied().map(FeastMarker::from).collect();
        self.refresh_people_inputs();
    }

    /// Replace the list of Commons footprints in the loaded world.
    pub fn set_commons(&mut self, areas: &[CommonsArea]) {
        self.commons = areas.to_vec();
        self.refresh_people_inputs();
    }

    fn people_inputs(&self, key: RegionKey) -> (f32, bool) {
        let (x0, x1, z0, z1) = block_rect(key);
        let floor = people::marker_floor(
            self.feasts
                .iter()
                .filter(|m| m.x >= x0 && m.x <= x1 && m.z >= z0 && m.z <= z1)
                .map(|m| m.guests),
        );
        (floor, self.commons.iter().any(|c| c.overlaps(x0, x1, z0, z1)))
    }

    fn refresh_people_inputs(&mut self) {
        let keys: Vec<RegionKey> = self.regions.keys().copied().collect();
        for key in keys {
            let (floor, commons) = self.people_inputs(key);
            if let Some(r) = self.regions.get_mut(&key) {
                r.floor = floor;
                r.commons = commons;
            }
        }
        let (feasts, commons) = (self.feasts.clone(), self.commons.clone());
        for p in &mut self.probes {
            p.refresh_inputs(&feasts, &commons);
        }
    }

    fn initial_for(&self, min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> Option<f32> {
        self.initial
            .iter()
            .rev()
            .find(|(lo, hi, _)| lo.x <= max_x && hi.x >= min_x && lo.z <= max_z && hi.z >= min_z)
            .map(|&(_, _, l)| l)
    }

    // ── The tick ─────────────────────────────────────────────────────

    /// Advance one logical tick.
    pub fn tick(&mut self, input: &TickInput<'_>) -> TickOutput {
        self.tick_no += 1;
        self.age_secs += input.dt;
        self.creative = input.creative;

        let centres: Vec<RegionKey> = input
            .local_players
            .iter()
            .map(|p| ((p.x / REGION as f32).floor() as i32, (p.z / REGION as f32).floor() as i32))
            .collect();
        self.track(&centres);
        if self.tick_no.is_multiple_of(CENSUS_TICKS) {
            self.take_census(input.ecs);
        }
        let mut gens = NATURAL_GENS_PER_TICK;
        self.scan_regions(input, &centres, &mut gens);
        self.scan_probes(input, &mut gens);
        self.advance_people_and_levels(input);
        self.detect_grey(input.allow_grey_event)
    }

    /// Keep exactly the regions within [`TRACK_RADIUS`] of a local player.
    fn track(&mut self, centres: &[RegionKey]) {
        let near = |k: &RegionKey| {
            centres.iter().any(|c| (k.0 - c.0).abs() <= TRACK_RADIUS && (k.1 - c.1).abs() <= TRACK_RADIUS)
        };
        let before = self.regions.len();
        self.regions.retain(|k, _| near(k));
        if self.regions.len() != before {
            self.natural.retain(|k| self.regions.contains_key(&k));
        }
        for c in centres {
            for dz in -TRACK_RADIUS..=TRACK_RADIUS {
                for dx in -TRACK_RADIUS..=TRACK_RADIUS {
                    let key = (c.0 + dx, c.1 + dz);
                    if !self.regions.contains_key(&key) {
                        let mut r = Region::new();
                        (r.floor, r.commons) = self.people_inputs(key);
                        self.regions.insert(key, r);
                    }
                }
            }
        }
    }

    /// One ECS pass: the passive wildlife and where it stands.
    fn take_census(&mut self, ecs: &hecs::World) {
        self.census.clear();
        for (_, (pos, kind)) in ecs.query::<(&crate::entity::Position, &crate::entity::MobKind)>().iter() {
            if counts_as_wildlife(kind.0) {
                self.census.push((pos.0.x, pos.0.z, kind.0));
            }
        }
        let mut masks: HashMap<RegionKey, u64> = HashMap::new();
        for &(x, z, kind) in &self.census {
            let key = ((x / REGION as f32).floor() as i32, (z / REGION as f32).floor() as i32);
            *masks.entry(key).or_default() |= 1u64 << (kind as u32);
        }
        for (key, r) in &mut self.regions {
            r.wildlife = masks.get(key).map_or(0, |m| m.count_ones());
            if r.known {
                r.eco = r.tally.eco(r.wildlife);
            }
        }
    }

    fn scan_regions(&mut self, input: &TickInput<'_>, centres: &[RegionKey], gens: &mut u32) {
        let mut order: Vec<(u64, i32, RegionKey)> = self
            .regions
            .iter()
            .map(|(&k, r)| {
                let d = centres.iter().map(|c| (k.0 - c.0).abs().max((k.1 - c.1).abs())).min().unwrap_or(0);
                (r.last_scan.map_or(0, |t| t + 1), d, k)
            })
            .collect();
        order.sort_unstable();
        let mut scans = 0;
        for (_, _, key) in order {
            if scans >= SCANS_PER_TICK {
                break;
            }
            let chunks = RegionChunks::of(input.world, key);
            let (x0, x1, z0, z1) = block_rect(key);
            let init = self.initial_for(x0, x1, z0, z1);
            let Some(r) = self.regions.get_mut(&key) else { continue };
            if !chunks.is_loaded() {
                // Nothing here yet. Rotate it to the back; it costs no scan.
                r.last_scan = Some(self.tick_no);
                r.known = false;
                r.tally = EcoTally::default();
                continue;
            }
            let natural = if chunks.is_touched() {
                if !self.natural.contains(key) {
                    if *gens == 0 {
                        continue;
                    }
                    *gens -= 1;
                    self.natural.ensure(input.world, input.biome_gen, key);
                }
                self.natural.get(key)
            } else {
                None
            };
            let mut tally = EcoTally::default();
            scan_columns(&chunks, natural, input.registry, 0..=CHUNK_SIZE - 1, 0..=CHUNK_SIZE - 1, &mut tally);
            scans += 1;
            r.last_scan = Some(self.tick_no);
            r.tally = tally;
            r.eco = r.tally.eco(r.wildlife);
            // Tier 2 S2 — b, on the scan's own cadence. A joined client's table
            // is empty (the authority keeps it), so there it is 0.
            r.built = crate::authorship::built_together(input.world.authors_in_region(key.0, key.1));
            // Tier 2 S3 — u, on the same cadence. A joined client's table is
            // empty (the authority keeps it), so there it is 0.
            r.lived = input.world.lived_in_term(key.0, key.1);
            // Tier 2 S6 — the shelves, from blocks and meta every client has.
            r.shelf = crate::shelf::region_terms(input.world, key);
            if !r.known {
                r.known = true;
                r.target = r.target_now(input.creative);
                r.level = init.unwrap_or(r.target);
            }
        }
    }

    fn scan_probes(&mut self, input: &TickInput<'_>, gens: &mut u32) {
        for i in 0..self.probes.len() {
            if self.tick_no < self.probes[i].next_scan {
                continue;
            }
            let (min, max) = (self.probes[i].min, self.probes[i].max);
            let Some(tally) = self.scan_box(input, min, max, gens) else { continue };
            let init = self.initial_for(min.x, max.x, min.z, max.z);
            let mask = self
                .census
                .iter()
                .filter(|&&(x, z, _)| {
                    x >= min.x as f32 && x < (max.x + 1) as f32 && z >= min.z as f32 && z < (max.z + 1) as f32
                })
                .fold(0u64, |m, &(_, _, k)| m | (1u64 << (k as u32)));
            let p = &mut self.probes[i];
            p.next_scan = self.tick_no + PROBE_SCAN_TICKS;
            p.wildlife = mask.count_ones();
            p.tally = tally;
            p.recompute_eco();
            if !p.scanned {
                p.scanned = true;
                p.target = level::target(p.eco, p.people_value(), input.creative);
                p.level = init.unwrap_or(p.target);
            }
        }
    }

    /// Scan the columns of a box across the regions it spans. `None` when a
    /// touched region's natural top is still to be generated this tick.
    fn scan_box(&mut self, input: &TickInput<'_>, min: IVec3, max: IVec3, gens: &mut u32) -> Option<EcoTally> {
        let mut tally = EcoTally::default();
        let (rx0, rx1) = (min.x.div_euclid(REGION), max.x.div_euclid(REGION));
        let (rz0, rz1) = (min.z.div_euclid(REGION), max.z.div_euclid(REGION));
        for rz in rz0..=rz1 {
            for rx in rx0..=rx1 {
                let key = (rx, rz);
                let chunks = RegionChunks::of(input.world, key);
                if !chunks.is_loaded() {
                    continue;
                }
                let natural = if chunks.is_touched() {
                    if !self.natural.contains(key) {
                        if *gens == 0 {
                            return None;
                        }
                        *gens -= 1;
                        self.natural.ensure(input.world, input.biome_gen, key);
                    }
                    self.natural.get(key)
                } else {
                    None
                };
                let local = |lo: i32, hi: i32, r: i32| {
                    ((lo.max(r * REGION) - r * REGION) as usize)..=((hi.min(r * REGION + REGION - 1) - r * REGION) as usize)
                };
                scan_columns(&chunks, natural, input.registry, local(min.x, max.x, rx), local(min.z, max.z, rz), &mut tally);
            }
        }
        Some(tally)
    }

    fn advance_people_and_levels(&mut self, input: &TickInput<'_>) {
        let players: Vec<Vec3> =
            input.local_players.iter().chain(input.remote_players).copied().collect();
        let mut present: HashMap<RegionKey, usize> = HashMap::new();
        for p in &players {
            for key in people::regions_near(p.x, p.z) {
                *present.entry(key).or_default() += 1;
            }
        }
        let (dt, scale, creative) = (input.dt, self.time_scale, input.creative);
        for (key, r) in &mut self.regions {
            r.people.step(present.get(key).copied().unwrap_or(0), r.commons, dt);
            if r.known {
                r.target = r.target_now(creative);
                r.level = level::ease(r.level, r.target, dt, scale);
            }
        }
        for p in &mut self.probes {
            let n = players.iter().filter(|q| p.player_present(q.x, q.z)).count();
            p.step(n, creative, dt, scale);
        }
    }

    fn detect_grey(&mut self, allowed: bool) -> TickOutput {
        if self.grey_fired || !allowed || self.age_secs < GREY_GRACE_SECS {
            return TickOutput::default();
        }
        let grey = self.regions.values().any(|r| r.known && r.level < GREY_BELOW && r.eco < GREY_BELOW);
        self.grey_fired = grey;
        TickOutput { place_went_grey: grey }
    }

    /// The terms of a scanned region: `(eco, people, level, target)`.
    pub fn region_terms(&self, key: RegionKey) -> Option<(f32, f32, f32, f32)> {
        self.regions.get(&key).filter(|r| r.known).map(|r| (r.eco, r.p(), r.level, r.target))
    }

    /// Regions currently tracked (tests and the Tally).
    pub fn tracked_regions(&self) -> usize {
        self.regions.len()
    }
}

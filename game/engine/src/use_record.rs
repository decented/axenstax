//! Living world Tier 2 S3 — how a place is used (spec
//! `docs/superpowers/specs/2026-11-09-living-world-tier2.md` §3 and §1 `u`).
//!
//! Each region (a chunk column, the bloom region) keeps a small record: the
//! lived days a player was present in it, the nights a bed in it was slept in,
//! and when it was last used. It lives on the machine that owns the world
//! (single-player, a lending host, a dedicated server), is saved with the world
//! as one appended `WorldSave` field, and holds no identities: it is never sent
//! on the wire and never shown per person.
//!
//! * **Lived days (T6).** The world keeps a counter, `lived_day`, that goes up
//!   by one the first time a player is present on a new world day (the night
//!   calendar's number). An idle dedicated server, `/time` and beds therefore
//!   cannot age a place: no player present, no lived day; a jump of several
//!   calendar days is still one lived day. `RegionUse::last_day` and the fade
//!   below are in lived days.
//! * **Once per day, once per night.** [`note_presence`] counts a lived day at
//!   most once per region; [`note_night`] a night at most once per region.
//! * **The bloom term.** [`u_term`]: `u = 0.6 (1 - e^(-(days + 2 nights)/10))`,
//!   only where the region holds at least [`MIN_PLACED`] placed blocks (a place,
//!   not a field walked through), faded linearly to 0 between 14 and 28 lived
//!   days since the region was last used.
//!
//! Path wear (the other half of §3) is in `path_wear.rs`; [`phase`] below is the
//! one shared entry point both call from.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::path_wear::Foot;
use crate::protocol::BlockChange;
use crate::world::World;

/// A region, as bloom keys it: a chunk column `(cx, cz)`.
pub type RegionKey = (i32, i32);

/// The per-region table. Ordered, so saving and digesting are canonical.
pub type UseTable = BTreeMap<RegionKey, RegionUse>;

/// Most regions kept. Past it the least recently used one is forgotten when a
/// new one is first used.
pub const MAX_REGIONS: usize = 16_384;
/// Placed blocks a region needs before the lived-in term applies.
pub const MIN_PLACED: u32 = 16;
/// `u` never exceeds this.
pub const U_CAP: f32 = 0.6;
/// Lived days (plus two per night) at which `u` reaches 63 % of its cap.
const U_SCALE: f32 = 10.0;
/// Lived days since last use at which `u` starts to fade.
pub const FADE_START: u32 = 14;
/// Lived days since last use at which `u` is gone.
pub const FADE_END: u32 = 28;
/// Ticks between presence notes (once a second at 20 TPS).
pub const PRESENCE_PERIOD: u64 = 20;

/// What the world has seen of one region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionUse {
    /// Lived days a player was present here.
    pub days: u16,
    /// The lived day of the latest presence (or sleep) here. 0 = never, since
    /// the first lived day is 1.
    pub last_day: u32,
    /// Nights slept in a bed here.
    pub nights: u16,
    /// The night-calendar number of the latest night counted, so a night
    /// counts once per region. `Option`, because night 0 is a real night.
    pub last_night: Option<u32>,
}

/// The region a block position is in.
pub fn region_of(x: i32, z: i32) -> RegionKey {
    (x.div_euclid(16), z.div_euclid(16))
}

/// Make room for a new region if the table is full: forget the least recently
/// used (smallest `last_day`, then key, so it is deterministic).
fn make_room(table: &mut UseTable, region: RegionKey) {
    if table.len() < MAX_REGIONS || table.contains_key(&region) {
        return;
    }
    if let Some(oldest) = table.iter().min_by_key(|(k, r)| (r.last_day, **k)).map(|(k, _)| *k) {
        table.remove(&oldest);
    }
}

/// A player is present in `region` on lived day `lived_day`: the day counts
/// once. Returns whether it was newly counted. A `lived_day` of 0 (nobody has
/// been present yet) counts nothing.
pub fn note_presence(table: &mut UseTable, region: RegionKey, lived_day: u32) -> bool {
    if lived_day == 0 {
        return false;
    }
    make_room(table, region);
    let r = table.entry(region).or_default();
    if r.last_day == lived_day {
        return false;
    }
    r.last_day = lived_day;
    r.days = r.days.saturating_add(1);
    true
}

/// A bed in `region` was slept in on night `night` (the night calendar's
/// number), while the world is on lived day `lived_day`: the night counts once
/// per region. A sleeper is present in the bed's region, so sleeping also
/// counts the lived day there (once, whichever of the two notes comes first)
/// and refreshes `last_day`. Returns whether the night was newly counted.
pub fn note_night(table: &mut UseTable, region: RegionKey, night: u32, lived_day: u32) -> bool {
    note_presence(table, region, lived_day);
    make_room(table, region);
    let r = table.entry(region).or_default();
    if r.last_night == Some(night) {
        return false;
    }
    r.last_night = Some(night);
    r.nights = r.nights.saturating_add(1);
    true
}

/// Bloom's lived-in term for one region: 0 below [`MIN_PLACED`] placed blocks
/// or with no record, else `U_CAP (1 - e^(-(days + 2 nights)/10))`, faded
/// linearly to 0 between [`FADE_START`] and [`FADE_END`] lived days since the
/// region was last used (`lived_day_now`).
pub fn u_term(record: Option<&RegionUse>, lived_day_now: u32, placed_count: u32) -> f32 {
    let Some(r) = record else { return 0.0 };
    if placed_count < MIN_PLACED {
        return 0.0;
    }
    let x = f32::from(r.days) + 2.0 * f32::from(r.nights);
    let base = U_CAP * (1.0 - (-x / U_SCALE).exp());
    let since = lived_day_now.saturating_sub(r.last_day);
    let fade = if since <= FADE_START {
        1.0
    } else if since >= FADE_END {
        0.0
    } else {
        (FADE_END - since) as f32 / (FADE_END - FADE_START) as f32
    };
    base * fade
}

/// The world's use record: the lived-day counter and the per-region table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UseRecord {
    lived_day: u32,
    last_world_day: Option<u32>,
    regions: UseTable,
}

impl UseRecord {
    /// The lived day the world is on (0 until a player has been present).
    pub fn lived_day(&self) -> u32 {
        self.lived_day
    }

    /// One region's record.
    pub fn get(&self, region: RegionKey) -> Option<&RegionUse> {
        self.regions.get(&region)
    }

    /// Nothing recorded at all.
    pub fn is_empty(&self) -> bool {
        self.lived_day == 0 && self.last_world_day.is_none() && self.regions.is_empty()
    }

    /// Forget everything (a world reset).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// A player is present on world day `world_day` (the night calendar's
    /// number): the lived day goes up by one the first time, and not again for
    /// the same world day. A jump of the calendar by several days is one step.
    pub fn begin_day(&mut self, world_day: u32) {
        if self.last_world_day != Some(world_day) {
            self.last_world_day = Some(world_day);
            self.lived_day = self.lived_day.saturating_add(1);
        }
    }

    /// A player stands in the region of block `(x, z)` on world day
    /// `world_day`.
    pub fn note_present_at(&mut self, world_day: u32, x: i32, z: i32) {
        self.begin_day(world_day);
        note_presence(&mut self.regions, region_of(x, z), self.lived_day);
    }

    /// A bed at block `(x, z)` was slept in on night `night`.
    pub fn note_slept_at(&mut self, night: u32, x: i32, z: i32) -> bool {
        note_night(&mut self.regions, region_of(x, z), night, self.lived_day)
    }

    /// The canonical saved form; `None` when nothing is recorded.
    pub fn to_saved(&self) -> Option<SavedRegionUse> {
        if self.is_empty() {
            return None;
        }
        Some(SavedRegionUse {
            lived_day: self.lived_day,
            last_world_day: self.last_world_day,
            regions: self.regions.iter().map(|(k, r)| (*k, *r)).collect(),
        })
    }

    /// Back from a save. A present-but-invalid row (used in the future of the
    /// record, a repeated region, past the table's bound) is dropped with a
    /// warning; the record never refuses to load.
    pub fn from_saved(saved: &SavedRegionUse) -> Self {
        let mut out = Self {
            lived_day: saved.lived_day,
            last_world_day: saved.last_world_day,
            regions: UseTable::new(),
        };
        let mut dropped = 0usize;
        for (key, r) in &saved.regions {
            if r.last_day > saved.lived_day || out.regions.len() >= MAX_REGIONS || out.regions.contains_key(key) {
                dropped += 1;
                continue;
            }
            out.regions.insert(*key, *r);
        }
        if dropped > 0 {
            log::warn!("world.dat: {dropped} invalid use-record row(s) dropped");
        }
        out
    }
}

/// The saved use record (`WorldSave.region_use`). Positional bincode: a change
/// to this type or to [`RegionUse`] is a `SAVE_LAYOUT_REVISION` bump. Holds no
/// identities, so a handover's transfer copy keeps it as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRegionUse {
    pub lived_day: u32,
    pub last_world_day: Option<u32>,
    /// Sorted by region.
    pub regions: Vec<(RegionKey, RegionUse)>,
}

impl World {
    /// Placed (non-air) blocks in region `(cx, cz)`, over the column's sixteen
    /// sections. Bloom calls it on the region-scan cadence, only for a region
    /// that has a use record.
    pub fn placed_blocks_in_region(&self, cx: i32, cz: i32) -> u32 {
        (0..=crate::world::MAX_CHUNK_Y)
            .filter_map(|cy| self.get_chunk(cx, cy, cz))
            .map(|c| c.placed_count())
            .sum()
    }

    /// Does region `(cx, cz)` have a use record (a player has stood in it)?
    pub fn has_use_record(&self, cx: i32, cz: i32) -> bool {
        self.use_record.get((cx, cz)).is_some()
    }

    /// Bloom's lived-in term for region `(cx, cz)` now.
    pub fn lived_in_term(&self, cx: i32, cz: i32) -> f32 {
        match self.use_record.get((cx, cz)) {
            None => 0.0,
            Some(r) => u_term(Some(r), self.use_record.lived_day(), self.placed_blocks_in_region(cx, cz)),
        }
    }
}

/// The use-record system of one tick, on whichever machine owns the world
/// (`SimSystem::UseRecord`): path wear every tick, and once a second a
/// presence note for each body. `feet` is every PRESENT, ALIVE body, whatever
/// its kind (a lending host's own seats are among a server's); `world_day` is
/// the night calendar's number; `creative` turns path wear off. Returns the
/// path changes to broadcast (standalone applies them already, the world is
/// written here).
pub fn phase(world: &mut World, tick: u64, world_day: u32, creative: bool, feet: &[Foot]) -> Vec<BlockChange> {
    if feet.is_empty() {
        return Vec::new();
    }
    if tick.is_multiple_of(PRESENCE_PERIOD) {
        for f in feet {
            world.use_record.note_present_at(world_day, f.pos.x.floor() as i32, f.pos.z.floor() as i32);
        }
    }
    crate::path_wear::tick_wear(world, feet, creative)
}

/// The cells a client that wrote [`phase`]'s changes to its own world must
/// redraw (a standalone game: no server sends them back as `BlockChange`s).
/// One per change, in order; the call site hands each to `rebuild_chunk_at`.
pub fn redraw_cells(changes: &[BlockChange]) -> impl Iterator<Item = (i32, i32, i32)> + '_ {
    changes.iter().map(|c| (c.x, c.y, c.z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn a_day_counts_once_per_region() {
        let mut t = UseTable::new();
        assert!(note_presence(&mut t, (0, 0), 1));
        assert!(!note_presence(&mut t, (0, 0), 1), "the same lived day again counts nothing");
        assert!(note_presence(&mut t, (0, 0), 2));
        assert!(note_presence(&mut t, (1, 0), 2), "another region has its own day");
        assert_eq!(t[&(0, 0)].days, 2);
        assert_eq!(t[&(1, 0)].days, 1);
        assert_eq!(t[&(0, 0)].last_day, 2);
        assert!(!note_presence(&mut t, (5, 5), 0), "nobody present yet: no lived day to count");
        assert!(!t.contains_key(&(5, 5)));
    }

    #[test]
    fn a_night_counts_once_per_night_per_region_and_night_zero_is_a_night() {
        let mut t = UseTable::new();
        assert!(note_night(&mut t, (0, 0), 0, 1), "night 0 is a real night");
        assert!(!note_night(&mut t, (0, 0), 0, 1));
        assert!(note_night(&mut t, (0, 0), 1, 2));
        assert!(!note_night(&mut t, (0, 0), 1, 2));
        assert!(note_night(&mut t, (1, 0), 1, 2), "another region's bed");
        assert_eq!(t[&(0, 0)].nights, 2);
        assert_eq!(t[&(0, 0)].last_day, 2, "sleeping is a use");
        assert_eq!(t[&(0, 0)].days, 2, "a sleeper is present: it counts the lived day, once");
        // Presence first or sleep first, the same day is counted once.
        let mut a = UseTable::new();
        note_presence(&mut a, (3, 3), 4);
        note_night(&mut a, (3, 3), 9, 4);
        let mut b = UseTable::new();
        note_night(&mut b, (3, 3), 9, 4);
        note_presence(&mut b, (3, 3), 4);
        assert_eq!(a, b);
        assert_eq!(a[&(3, 3)].days, 1);
        // Before any lived day (nobody's second has passed yet) it counts the night only.
        let mut c = UseTable::new();
        assert!(note_night(&mut c, (0, 0), 0, 0));
        assert_eq!((c[&(0, 0)].days, c[&(0, 0)].last_day, c[&(0, 0)].nights), (0, 0, 1));
    }

    #[test]
    fn a_lived_day_needs_a_player_and_jumps_count_once() {
        let mut r = UseRecord::default();
        assert_eq!(r.lived_day(), 0);
        // Nobody present: nothing is ever called, nothing moves (an idle server).
        assert!(r.is_empty());
        r.note_present_at(10, 5, 5);
        assert_eq!(r.lived_day(), 1);
        r.note_present_at(10, 5, 5);
        assert_eq!(r.lived_day(), 1, "the same world day is the same lived day");
        // `/time` or a bed jumps the calendar by 40 days: one step.
        r.note_present_at(50, 5, 5);
        assert_eq!(r.lived_day(), 2);
        // And the calendar going backwards (a reset) is a new day too, never a panic.
        r.note_present_at(0, 5, 5);
        assert_eq!(r.lived_day(), 3);
        assert_eq!(r.get((0, 0)).unwrap().days, 3);
    }

    #[test]
    fn u_is_zero_below_sixteen_placed_blocks_and_follows_the_curve() {
        let used = RegionUse { days: 10, last_day: 10, nights: 0, last_night: None };
        assert_eq!(u_term(Some(&used), 10, MIN_PLACED - 1), 0.0);
        assert_eq!(u_term(None, 10, 100), 0.0);
        let want = 0.6 * (1.0 - (-1.0f32).exp());
        assert!((u_term(Some(&used), 10, MIN_PLACED) - want).abs() < 1e-6);
        // Nights count double.
        let slept = RegionUse { days: 6, last_day: 10, nights: 2, last_night: Some(3) };
        assert!((u_term(Some(&slept), 10, 50) - want).abs() < 1e-6, "6 + 2 x 2 = 10");
        // Saturates under the cap.
        let long = RegionUse { days: 60_000, last_day: 1, nights: 60_000, last_night: None };
        assert!(u_term(Some(&long), 1, 50) <= U_CAP + 1e-6);
    }

    #[test]
    fn u_fades_linearly_between_14_and_28_lived_days() {
        let r = RegionUse { days: 10, last_day: 100, nights: 0, last_night: None };
        let full = u_term(Some(&r), 100, 50);
        assert!(full > 0.3);
        // At 14 days since use: not faded yet. At 21: half. At 28: gone.
        assert!((u_term(Some(&r), 100 + 14, 50) - full).abs() < 1e-6);
        assert!((u_term(Some(&r), 100 + 21, 50) - full / 2.0).abs() < 1e-6);
        assert_eq!(u_term(Some(&r), 100 + 28, 50), 0.0);
        assert_eq!(u_term(Some(&r), 100 + 400, 50), 0.0);
    }

    #[test]
    fn the_table_is_bounded_and_forgets_the_least_recently_used() {
        let mut t = UseTable::new();
        for i in 0..MAX_REGIONS as i32 {
            note_presence(&mut t, (i, 0), 1 + (i as u32 % 5));
        }
        assert_eq!(t.len(), MAX_REGIONS);
        // (0, 0) was used on day 1, the oldest; a new region evicts it.
        assert!(note_presence(&mut t, (-1, -1), 7));
        assert_eq!(t.len(), MAX_REGIONS);
        assert!(!t.contains_key(&(0, 0)) && t.contains_key(&(-1, -1)));
        // A region already in the table never evicts anything.
        note_presence(&mut t, (-1, -1), 8);
        assert_eq!(t.len(), MAX_REGIONS);
    }

    #[test]
    fn the_saved_form_is_canonical_empty_is_none_and_it_round_trips() {
        assert_eq!(UseRecord::default().to_saved(), None);
        let mut a = UseRecord::default();
        a.note_present_at(3, 40, 40);
        a.note_present_at(3, -20, 5);
        a.note_slept_at(2, 40, 41);
        // The same events in another order give the same bytes.
        let mut b = UseRecord::default();
        b.begin_day(3);
        b.note_present_at(3, -20, 5);
        b.note_slept_at(2, 40, 41);
        b.note_present_at(3, 40, 40);
        assert_eq!(
            bincode::serialize(&a.to_saved()).unwrap(),
            bincode::serialize(&b.to_saved()).unwrap()
        );
        let saved = a.to_saved().unwrap();
        assert!(saved.regions.windows(2).all(|w| w[0].0 < w[1].0), "sorted by region");
        assert_eq!(UseRecord::from_saved(&saved), a);
    }

    #[test]
    fn an_invalid_saved_row_is_dropped_never_refused() {
        let ok = RegionUse { days: 1, last_day: 2, nights: 0, last_night: None };
        let future = RegionUse { days: 1, last_day: 9, nights: 0, last_night: None };
        let saved = SavedRegionUse {
            lived_day: 2,
            last_world_day: Some(4),
            regions: vec![((0, 0), ok), ((0, 0), ok), ((1, 0), future), ((2, 0), ok)],
        };
        let r = UseRecord::from_saved(&saved);
        assert_eq!(r.lived_day(), 2);
        assert!(r.get((0, 0)).is_some() && r.get((2, 0)).is_some());
        assert!(r.get((1, 0)).is_none(), "used in the future of its own record");
        assert_eq!(r.regions.len(), 2, "the repeat of (0, 0) is dropped");
    }

    #[test]
    fn the_phase_notes_presence_once_a_second_and_nothing_without_a_body() {
        let mut w = World::new();
        let feet = [Foot { key: 0, pos: Vec3::new(3.5, 40.0, 3.5), on_ground: true }];
        phase(&mut w, 7, 1, false, &feet);
        assert!(w.use_record.is_empty(), "not on a second boundary");
        phase(&mut w, 20, 1, false, &[]);
        assert!(w.use_record.is_empty(), "nobody present: no lived day");
        phase(&mut w, 20, 1, false, &feet);
        assert_eq!(w.use_record.lived_day(), 1);
        assert_eq!(w.use_record.get((0, 0)).unwrap().days, 1);
        phase(&mut w, 40, 1, false, &feet);
        assert_eq!(w.use_record.get((0, 0)).unwrap().days, 1, "the same day counts once");
    }

    #[test]
    fn a_standalone_seat_gets_every_worn_cell_back_to_redraw() {
        // The standalone seat loop writes the world itself and redraws what
        // `phase` returns: this is its seam. A grass lane paced 24 times each
        // way wears; the one tick that wears a cell hands it back, and the
        // world already reads the path there (what the redraw will mesh).
        let mut w = World::new();
        for x in 0..2 {
            w.set_block(x, 39, 0, crate::block::GRASS);
        }
        let mut redrawn = Vec::new();
        for i in 0..60u64 {
            let x = if i % 2 == 0 { 0.5 } else { 1.5 };
            let feet = [Foot { key: 0, pos: Vec3::new(x, 40.0, 0.5), on_ground: true }];
            let worn = phase(&mut w, i + 1, 1, false, &feet);
            for (cx, cy, cz) in redraw_cells(&worn) {
                assert_eq!(w.get_block(cx, cy, cz), crate::block::WORN_PATH);
                redrawn.push((cx, cy, cz));
            }
        }
        redrawn.sort_unstable();
        assert_eq!(redrawn, vec![(0, 39, 0), (1, 39, 0)], "each worn cell once, none missed");
        assert_eq!(redraw_cells(&[]).count(), 0);
    }
}

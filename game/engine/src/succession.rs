//! Living world Tier 2 S5 — last seen and succession (spec
//! `docs/superpowers/specs/2026-11-09-living-world-tier2.md` §5, calls T3, T6, T8).
//!
//! A verified joiner who places a Plot Marker now owns a real Plot
//! (`PlotOwner::Npub`). A Plot nobody visits would keep land closed forever, so
//! it passes on: when its owner has not been in the world for [`GONE_SECS`] of
//! **real** time (the wall clock, not the world's: a dedicated server runs about
//! 72 world days per real day, and anyone could sleep the calendar forward), the
//! Plot is released, its marker turns into a Commons Stone, and the owner's
//! authorship inside the footprint is cleared so the village can tend it.
//!
//! * **Authority-side only.** The table lives on the machine that owns the world
//!   (a dedicated server, a lending host, single-player) and is saved as one
//!   appended `WorldSave` field ([`SavedSuccession`]). Last seen is never shown
//!   and never sent; an export drops the whole field (it holds identities and
//!   times), a handover keeps it.
//! * **A local seat's Plot never passes on** (the household's); only `Npub`
//!   owners are tracked or judged.
//! * **No wall clock, nothing happens.** [`now_unix_secs`] is `None` on wasm
//!   (which has no joiners), and [`phase`] then updates and passes on nothing.
//! * **Pure rules take `now`.** [`due`], [`Succession::check`] and [`phase`] are
//!   handed the time, so tests never read the clock.
//! * **Upgrade-safe.** An owner with no row is stamped "now" the first time the
//!   table is checked, so a world from before this field never passes a Plot
//!   at once.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::authorship::AuthorKey;
use crate::plot::{PlotData, PlotOwner};
use crate::protocol::BlockChange;
use crate::world::World;

/// Real seconds an owner may be away before their Plot passes on: 60 days.
pub const GONE_SECS: u64 = 60 * 86_400;
/// Ticks between last-seen stamps of the owners present (a minute at 20 TPS).
pub const STAMP_PERIOD: u64 = 1_200;
/// Most owners tracked. A new owner past it is simply not tracked (their Plot
/// is then never judged gone; it is the safe side).
pub const MAX_OWNERS: usize = 4_096;
/// Most passed-on markers remembered; past it the lowest is forgotten.
pub const MAX_PASSED: usize = 4_096;
/// Longest npub string a saved row may carry.
const MAX_NPUB_LEN: usize = 128;

/// A Plot Marker's cell.
pub type Marker = (i32, i32, i32);

/// When each Plot owner was last in the world, and which Plots have passed on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Succession {
    /// npub to unix seconds. Only owners of a Plot have a row.
    last_seen: BTreeMap<String, u64>,
    /// Markers that became a Commons Stone through succession (the Satoshi line).
    passed_on: BTreeSet<Marker>,
    /// The night-calendar day succession was last checked on. Runtime only:
    /// `None` after a load, so the first call checks (once on load).
    checked_day: Option<u32>,
}

/// The distinct npubs that own at least one Plot.
fn plot_owners(plots: &[PlotData]) -> BTreeSet<&str> {
    plots
        .iter()
        .filter_map(|p| match &p.owner {
            PlotOwner::Npub(n) => Some(n.as_str()),
            PlotOwner::LocalPlayer(_) => None,
        })
        .collect()
}

/// The markers of the Plots whose `Npub` owner was last seen [`GONE_SECS`] or
/// more before `now`. An owner with no row is not due (it is stamped first, see
/// [`Succession::check`]); a `LocalPlayer` Plot is never due. Sorted.
pub fn due(plots: &[PlotData], last_seen: &BTreeMap<String, u64>, now: u64) -> Vec<Marker> {
    let mut out: Vec<Marker> = plots
        .iter()
        .filter_map(|p| {
            let PlotOwner::Npub(n) = &p.owner else { return None };
            let seen = *last_seen.get(n)?;
            (now.saturating_sub(seen) >= GONE_SECS).then_some(p.marker)
        })
        .collect();
    out.sort_unstable();
    out
}

impl Succession {
    /// Nothing recorded (a runtime-only `checked_day` does not count).
    pub fn is_empty(&self) -> bool {
        self.last_seen.is_empty() && self.passed_on.is_empty()
    }

    /// Forget everything (a world reset).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Stamp "now" for each present npub that owns a Plot. Others are not
    /// tracked.
    pub fn note_present(&mut self, plots: &[PlotData], present: &[String], now: u64) {
        let owners = plot_owners(plots);
        for npub in present {
            if !owners.contains(npub.as_str()) {
                continue;
            }
            if self.last_seen.len() >= MAX_OWNERS && !self.last_seen.contains_key(npub) {
                continue;
            }
            self.last_seen.insert(npub.clone(), now);
        }
    }

    /// Stamp "now" for each Plot owner with no row, and drop the rows of npubs
    /// that own no Plot.
    pub fn prepare(&mut self, plots: &[PlotData], now: u64) {
        let owners = plot_owners(plots);
        self.last_seen.retain(|n, _| owners.contains(n.as_str()));
        for n in owners {
            if self.last_seen.len() >= MAX_OWNERS {
                break;
            }
            self.last_seen.entry(n.to_string()).or_insert(now);
        }
    }

    /// The succession check at `now`: prepare the table (stamp the unrecorded,
    /// prune the rest), then the markers [`due`].
    pub fn check(&mut self, plots: &[PlotData], now: u64) -> Vec<Marker> {
        self.prepare(plots, now);
        due(plots, &self.last_seen, now)
    }

    /// When `npub` was last seen (tests).
    #[cfg(test)]
    pub fn last_seen_of(&self, npub: &str) -> Option<u64> {
        self.last_seen.get(npub).copied()
    }

    /// Has the Plot at `marker` passed on (tests; the Satoshi test reads the set itself)?
    #[cfg(test)]
    pub fn has_passed_on(&self, marker: &Marker) -> bool {
        self.passed_on.contains(marker)
    }

    fn remember_passed_on(&mut self, marker: Marker) {
        self.passed_on.insert(marker);
        while self.passed_on.len() > MAX_PASSED {
            self.passed_on.pop_first();
        }
    }

    /// The canonical saved form, `None` when nothing is recorded.
    pub fn to_saved(&self) -> Option<SavedSuccession> {
        if self.is_empty() {
            return None;
        }
        Some(SavedSuccession {
            last_seen: self.last_seen.iter().map(|(n, t)| (n.clone(), *t)).collect(),
            passed_on: self.passed_on.iter().copied().collect(),
        })
    }

    /// Back from a save. A row with an empty or over-long npub, a repeat, or
    /// one past a table's bound is dropped with a warning; the table never
    /// refuses to load.
    pub fn from_saved(saved: &SavedSuccession) -> Self {
        let mut out = Self::default();
        let mut dropped = 0usize;
        for (npub, seen) in &saved.last_seen {
            if npub.is_empty()
                || npub.len() > MAX_NPUB_LEN
                || out.last_seen.len() >= MAX_OWNERS
                || out.last_seen.contains_key(npub)
            {
                dropped += 1;
                continue;
            }
            out.last_seen.insert(npub.clone(), *seen);
        }
        for m in &saved.passed_on {
            if out.passed_on.len() >= MAX_PASSED || !out.passed_on.insert(*m) {
                dropped += 1;
            }
        }
        if dropped > 0 {
            log::warn!("world.dat: {dropped} invalid Plot succession row(s) dropped");
        }
        out
    }
}

/// The saved table (`WorldSave.plot_succession`). Positional bincode: a change
/// to this type is a `SAVE_LAYOUT_REVISION` bump. Holds identities and times, so
/// an export drops it and it is never sent on the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSuccession {
    /// `(npub, unix seconds last seen)`, sorted by npub.
    pub last_seen: Vec<(String, u64)>,
    /// Markers that passed on, sorted.
    pub passed_on: Vec<Marker>,
}

/// The wall clock in unix seconds; `None` where there is none (wasm).
pub fn now_unix_secs() -> Option<u64> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

/// Pass the Plots in `markers` on, in order for each: release the Plot, clear
/// its owner's authorship inside the footprint, then turn the marker into a
/// Commons Stone (unless that stone would be refused, or the cell no longer
/// holds a marker, in which case the cell is left as it is). A Plot whose marker
/// chunk is not loaded waits for a later check. Returns the block changes the
/// authority broadcasts (the world is already written).
fn pass_on(world: &mut World, markers: &[Marker]) -> Vec<BlockChange> {
    // Take the plots first, so two of one owner's overlapping Plots do not
    // refuse each other's stone.
    let mut passing: Vec<PlotData> = Vec::new();
    for &m in markers {
        let (cx, cy, cz) = crate::authorship::chunk_of(m.0, m.1, m.2);
        if !world.has_chunk(cx, cy, cz) {
            continue;
        }
        if let Some(p) = world.plots.iter().find(|p| p.marker == m && matches!(p.owner, PlotOwner::Npub(_))) {
            passing.push(p.clone());
        }
    }
    for p in &passing {
        world.release_plot(p.marker);
    }
    let mut changes = Vec::new();
    for p in passing {
        let PlotOwner::Npub(npub) = &p.owner else { continue };
        world.authors.clear_key_in_box(&AuthorKey::Npub(npub.clone()), p.min_x, p.max_x, p.min_z, p.max_z);
        let (x, y, z) = p.marker;
        if world.get_block(x, y, z) != crate::block::PLOT_MARKER {
            continue;
        }
        if crate::commons::stone_overlaps_a_plot(&world.plots, x, z) {
            continue;
        }
        world.set_block(x, y, z, crate::block::COMMONS_STONE);
        world.plot_succession.remember_passed_on(p.marker);
        changes.push(BlockChange::with_meta(x, y, z, crate::block::COMMONS_STONE, 0));
    }
    changes
}

/// Succession for one tick, on whichever machine owns the world. `present` is
/// asked only when a stamp or a check is due, and gives the npub of every
/// verified body in the world right now (none standalone);
/// `world_day` is the night calendar's number; `now` the wall clock
/// ([`now_unix_secs`]; `None` = no clock, nothing happens).
///
/// Every [`STAMP_PERIOD`] ticks the present owners are stamped. On each new
/// world day, and on the first call after a load, the table is checked (the
/// present owners stamped first, so no one who is here is ever passed over) and
/// the due Plots pass on. Returns the block changes to broadcast.
pub fn phase(world: &mut World, tick: u64, world_day: u32, now: Option<u64>, present: impl FnOnce() -> Vec<String>) -> Vec<BlockChange> {
    let Some(now) = now else { return Vec::new() };
    let stamp_due = tick.is_multiple_of(STAMP_PERIOD);
    let day_due = world.plot_succession.checked_day != Some(world_day);
    if !stamp_due && !day_due {
        return Vec::new();
    }
    let plots = std::mem::take(&mut world.plots);
    world.plot_succession.note_present(&plots, &present(), now);
    let markers = if day_due {
        world.plot_succession.checked_day = Some(world_day);
        world.plot_succession.check(&plots, now)
    } else {
        Vec::new()
    };
    world.plots = plots;
    if markers.is_empty() {
        return Vec::new();
    }
    pass_on(world, &markers)
}

/// Does column `(x, z)` lie in a Commons that came from a passed-on Plot (its
/// stone still standing)? The Satoshi line's test.
pub fn in_passed_on_commons(world: &World, x: f32, z: f32) -> bool {
    if world.plot_succession.passed_on.is_empty() {
        return false;
    }
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    world.plot_succession.passed_on.iter().any(|&(mx, my, mz)| {
        crate::bloom::CommonsArea::from_marker(mx, mz).overlaps(bx, bx, bz, bz)
            && world.get_block(mx, my, mz) == crate::block::COMMONS_STONE
    })
}

/// What Satoshi says the first time a seat walks into a passed-on Commons.
pub const SATOSHI_PASSED_ON: &str = "Someone built this and moved on. It's the village's now.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    const DAY: u64 = 86_400;
    const T0: u64 = 1_800_000_000;

    fn npub_plot(n: &str, x: i32) -> PlotData {
        PlotData::from_marker(PlotOwner::Npub(n.into()), x, 64, 0)
    }

    fn seen(rows: &[(&str, u64)]) -> BTreeMap<String, u64> {
        rows.iter().map(|(n, t)| (n.to_string(), *t)).collect()
    }

    #[test]
    fn due_at_exactly_sixty_days_and_not_a_second_before() {
        let plots = [npub_plot("npub1a", 0)];
        let rows = seen(&[("npub1a", T0)]);
        assert_eq!(due(&plots, &rows, T0 + 60 * DAY), vec![(0, 64, 0)], "60 days to the second");
        assert!(due(&plots, &rows, T0 + 60 * DAY - 1).is_empty(), "59 days 23:59:59");
        assert!(due(&plots, &rows, T0 + 59 * DAY + DAY / 100 * 99).is_empty(), "59.99 days");
        assert_eq!(due(&plots, &rows, T0 + 400 * DAY).len(), 1);
        assert!(due(&plots, &rows, T0 - 5).is_empty(), "a clock that went backwards is no absence");
    }

    #[test]
    fn an_unrecorded_owner_is_stamped_not_due() {
        let plots = [npub_plot("npub1a", 0)];
        let mut s = Succession::default();
        assert!(due(&plots, &s.last_seen, T0 + 999 * DAY).is_empty(), "no row, not due, whatever the clock");
        assert!(s.check(&plots, T0).is_empty());
        assert_eq!(s.last_seen_of("npub1a"), Some(T0), "stamped now");
        assert!(s.check(&plots, T0 + 59 * DAY).is_empty());
        assert_eq!(s.check(&plots, T0 + 60 * DAY), vec![(0, 64, 0)], "the sixty days run from the stamp");
    }

    #[test]
    fn a_local_seats_plot_is_never_due_or_tracked() {
        let plots = [PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0)];
        let mut s = Succession::default();
        assert!(s.check(&plots, T0).is_empty());
        assert!(s.check(&plots, T0 + 9_999 * DAY).is_empty());
        assert!(s.is_empty(), "a seat is not tracked");
    }

    #[test]
    fn rows_of_npubs_that_own_no_plot_are_pruned() {
        let mut s = Succession::default();
        s.last_seen.insert("npub1gone".into(), T0);
        s.last_seen.insert("npub1a".into(), T0);
        s.prepare(&[npub_plot("npub1a", 0)], T0 + 5);
        assert_eq!(s.last_seen_of("npub1gone"), None);
        assert_eq!(s.last_seen_of("npub1a"), Some(T0), "an existing row is not refreshed by a check");
        // Only Plot owners are stamped as present.
        s.note_present(&[npub_plot("npub1a", 0)], &["npub1a".into(), "npub1visitor".into()], T0 + 10);
        assert_eq!(s.last_seen_of("npub1a"), Some(T0 + 10));
        assert_eq!(s.last_seen_of("npub1visitor"), None, "a visitor owns nothing: not tracked");
    }

    #[test]
    fn the_saved_form_is_canonical_empty_is_none_and_bad_rows_are_dropped() {
        assert_eq!(Succession::default().to_saved(), None);
        let mut a = Succession::default();
        a.last_seen.insert("npub1z".into(), 5);
        a.last_seen.insert("npub1a".into(), 9);
        a.passed_on.insert((3, 64, 3));
        a.passed_on.insert((-3, 64, 3));
        let mut b = Succession::default();
        b.passed_on.insert((-3, 64, 3));
        b.last_seen.insert("npub1a".into(), 9);
        b.passed_on.insert((3, 64, 3));
        b.last_seen.insert("npub1z".into(), 5);
        assert_eq!(a.to_saved(), b.to_saved(), "insertion order does not matter");
        let saved = a.to_saved().unwrap();
        assert_eq!(saved.last_seen, vec![("npub1a".to_string(), 9), ("npub1z".to_string(), 5)]);
        assert_eq!(saved.passed_on, vec![(-3, 64, 3), (3, 64, 3)]);
        assert_eq!(Succession::from_saved(&saved).to_saved(), Some(saved.clone()));
        let bad = SavedSuccession {
            last_seen: vec![("".into(), 1), ("npub1a".into(), 2), ("npub1a".into(), 3), ("x".repeat(200), 4)],
            passed_on: vec![(1, 1, 1), (1, 1, 1)],
        };
        let back = Succession::from_saved(&bad);
        assert_eq!(back.last_seen_of("npub1a"), Some(2), "the first row of a repeat wins");
        assert_eq!(back.last_seen.len(), 1);
        assert_eq!(back.passed_on.len(), 1);
    }

    fn world_with_plot(npub: &str, at: i32) -> World {
        let mut w = World::new();
        w.place_player_block(at, 64, 0, block::PLOT_MARKER);
        w.plots.push(PlotData::from_marker(PlotOwner::Npub(npub.into()), at, 64, 0));
        w
    }

    #[test]
    fn the_phase_passes_a_plot_on_after_sixty_real_days_and_not_before() {
        let mut w = world_with_plot("npub1a", 0);
        // First call: stamps the owner, passes nothing.
        assert!(phase(&mut w, 1, 7, Some(T0), Vec::new).is_empty());
        assert_eq!(w.plot_succession.last_seen_of("npub1a"), Some(T0));
        // Same day, nothing to do even with a huge clock.
        assert!(phase(&mut w, 2, 7, Some(T0 + 99 * DAY), Vec::new).is_empty());
        // A new day, 59 days on: not yet.
        assert!(phase(&mut w, 3, 8, Some(T0 + 59 * DAY), Vec::new).is_empty());
        assert_eq!(w.plots.len(), 1);
        // Another new day at 60: it passes.
        let changes = phase(&mut w, 4, 9, Some(T0 + 60 * DAY), Vec::new);
        assert_eq!(changes, vec![BlockChange::with_meta(0, 64, 0, block::COMMONS_STONE, 0)]);
        assert!(w.plots.is_empty(), "released");
        assert_eq!(w.get_block(0, 64, 0), block::COMMONS_STONE);
        assert_eq!(w.commons_areas().len(), 1, "a real Commons");
        assert!(w.plot_succession.has_passed_on(&(0, 64, 0)));
        // The owner owns nothing now: the next check prunes their row.
        phase(&mut w, 5, 10, Some(T0 + 61 * DAY), Vec::new);
        assert!(w.plot_succession.last_seen.is_empty());
    }

    #[test]
    fn a_present_owner_is_never_passed_over() {
        let mut w = world_with_plot("npub1a", 0);
        phase(&mut w, 1, 1, Some(T0), Vec::new);
        // 90 days later the owner is online when the new day is checked.
        let changes = phase(&mut w, 7, 2, Some(T0 + 90 * DAY), || vec!["npub1a".to_string()]);
        assert!(changes.is_empty());
        assert_eq!(w.plots.len(), 1);
        assert_eq!(w.plot_succession.last_seen_of("npub1a"), Some(T0 + 90 * DAY));
    }

    #[test]
    fn a_stamp_comes_every_period_and_no_clock_means_nothing_happens() {
        let mut w = world_with_plot("npub1a", 0);
        phase(&mut w, 1, 1, Some(T0), Vec::new);
        let me = ["npub1a".to_string()];
        phase(&mut w, STAMP_PERIOD, 1, Some(T0 + 100), || me.to_vec());
        assert_eq!(w.plot_succession.last_seen_of("npub1a"), Some(T0 + 100));
        phase(&mut w, STAMP_PERIOD + 1, 1, Some(T0 + 200), || me.to_vec());
        assert_eq!(w.plot_succession.last_seen_of("npub1a"), Some(T0 + 100), "off the period: no stamp");
        // No wall clock: no stamp, no check.
        let before = w.plot_succession.clone();
        assert!(phase(&mut w, 2 * STAMP_PERIOD, 50, None, || me.to_vec()).is_empty());
        assert_eq!(w.plot_succession, before);
    }

    #[test]
    fn the_first_call_after_a_load_checks_once() {
        let mut w = world_with_plot("npub1a", 0);
        w.plot_succession.last_seen.insert("npub1a".into(), T0);
        // A loaded world: checked_day is None, so the very first call judges.
        let changes = phase(&mut w, 1, 40, Some(T0 + 61 * DAY), Vec::new);
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn the_stone_is_left_out_when_it_would_be_refused_but_the_plot_still_goes() {
        let mut w = world_with_plot("npub1a", 0);
        // A foreign plot whose footprint touches this one's (a legacy overlap).
        w.plots.push(PlotData::from_marker(PlotOwner::LocalPlayer(0), 20, 64, 0));
        w.plot_succession.last_seen.insert("npub1a".into(), T0);
        let changes = phase(&mut w, 1, 1, Some(T0 + 61 * DAY), Vec::new);
        assert!(changes.is_empty(), "no stone");
        assert_eq!(w.get_block(0, 64, 0), block::PLOT_MARKER, "the marker stays a plain block");
        assert_eq!(w.plots.len(), 1, "the other plot is untouched; this one is released");
        assert!(matches!(w.plots[0].owner, PlotOwner::LocalPlayer(0)));
        assert!(!w.plot_succession.has_passed_on(&(0, 64, 0)));
    }

    #[test]
    fn a_marker_chunk_that_is_not_loaded_waits() {
        let mut w = World::new();
        w.plots.push(npub_plot("npub1a", 5_000));
        w.plot_succession.last_seen.insert("npub1a".into(), T0);
        assert!(phase(&mut w, 1, 1, Some(T0 + 61 * DAY), Vec::new).is_empty());
        assert_eq!(w.plots.len(), 1, "not released: the stone could not be written");
    }

    #[test]
    fn the_owners_authorship_in_the_footprint_goes_and_others_stay() {
        let mut w = world_with_plot("npub1a", 0);
        let a = AuthorKey::Npub("npub1a".into());
        let b = AuthorKey::Npub("npub1b".into());
        w.place_player_block_as(3, 65, 3, block::STONE, Some(&a));
        w.place_player_block_as(-16, 65, 16, block::STONE, Some(&a)); // the footprint's far corner
        w.place_player_block_as(4, 65, 3, block::STONE, Some(&b));
        w.place_player_block_as(17, 65, 0, block::STONE, Some(&a)); // outside
        w.plot_succession.last_seen.insert("npub1a".into(), T0);
        phase(&mut w, 1, 1, Some(T0 + 61 * DAY), Vec::new);
        assert_eq!(w.author_at(3, 65, 3), None, "the owner's blocks are anyone's now");
        assert_eq!(w.author_at(-16, 65, 16), None);
        assert!(w.is_placed(3, 65, 3), "but they are still built");
        assert_eq!(w.author_at(4, 65, 3), Some(&b), "another builder's stay theirs");
        assert_eq!(w.author_at(17, 65, 0), Some(&a), "outside the footprint is untouched");
        // And a Commons protects what is still authored: b's block holds, a's is free.
        assert!(w.commons_protects(4, 65, 3, Some(&a)));
        assert!(!w.commons_protects(3, 65, 3, Some(&b)));
    }

    #[test]
    fn two_overlapping_plots_of_one_owner_both_pass_on() {
        let mut w = World::new();
        for x in [0, 10] {
            w.place_player_block(x, 64, 0, block::PLOT_MARKER);
            w.plots.push(PlotData::from_marker(PlotOwner::Npub("npub1a".into()), x, 64, 0));
        }
        w.plot_succession.last_seen.insert("npub1a".into(), T0);
        let changes = phase(&mut w, 1, 1, Some(T0 + 61 * DAY), Vec::new);
        assert_eq!(changes.len(), 2, "neither stone is refused by the other");
        assert!(w.plots.is_empty());
    }

    #[test]
    fn the_satoshi_test_needs_the_stone_still_standing() {
        let mut w = world_with_plot("npub1a", 0);
        w.plot_succession.last_seen.insert("npub1a".into(), T0);
        phase(&mut w, 1, 1, Some(T0 + 61 * DAY), Vec::new);
        assert!(in_passed_on_commons(&w, 5.5, -7.2));
        assert!(in_passed_on_commons(&w, 16.9, 16.0), "inside the footprint's edge");
        assert!(!in_passed_on_commons(&w, 17.2, 0.0), "outside");
        // A Commons the player declared by hand is no passed-on one.
        w.place_player_block(100, 64, 0, block::COMMONS_STONE);
        assert!(!in_passed_on_commons(&w, 100.0, 0.0));
        w.set_block(0, 64, 0, block::AIR);
        assert!(!in_passed_on_commons(&w, 5.5, 0.0), "the stone was taken away");
    }

    #[test]
    fn the_satoshi_line_is_plain_and_money_free() {
        use crate::copy_lint::{banned_in, BANNED_MONEY_WORDS};
        assert!(banned_in(SATOSHI_PASSED_ON, BANNED_MONEY_WORDS).is_empty());
        assert!(banned_in(&format!("Satoshi: {SATOSHI_PASSED_ON}"), BANNED_MONEY_WORDS).is_empty());
    }
}

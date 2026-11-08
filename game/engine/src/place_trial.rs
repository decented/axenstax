//! Place trials (living world spec `docs/superpowers/specs/2026-11-08-living-world-tier1.md` §6):
//! the `Objective::Place` half of the trial engine.
//!
//! A trial whose objective is a `Place` asks the player to bring a stretch of
//! ground back to life. While the scenario runs, a bloom probe sits over the
//! area (`BloomField::add_probe`); the leaf completes when the probe has read
//! "Alive" (level at or above the goal for the held time). This module holds
//! - the pure rules: the tempo a scenario may set, the recovery shares behind
//!   the end card's flower, its sentence and the ways it records;
//! - the per-run record ([`PlaceTrack`]) kept on the `ScenarioState`;
//! - the `GameState` glue: registering and removing probes, the per-tick read,
//!   and the grey start of a fresh arena.
//!
//! Presentation of the result (the flower) is in `hud_ui`; nothing here touches
//! the wire, a save or the world.

use crate::bloom::level::{self, Stage};
use crate::bloom::ProbeState;
use crate::scenario::{ArenaSetup, PlaceArea};

/// First probe id used by place trials; a leaf's probe is `BASE + flat leaf
/// index`. Far from any id another system might pick.
pub const PLACE_PROBE_BASE: u32 = 0x5000_0000;

/// The probe id of the `Place` leaf with flat index `leaf`.
pub fn probe_id(leaf: usize) -> u32 {
    PLACE_PROBE_BASE + leaf as u32
}

/// The bloom tempo to apply this tick. A scenario's own `bloom_time_scale`
/// counts ONLY while it runs in a fresh trial-arena world; the in-world
/// `/scenario` overlay, and every world with no scenario, runs at `1.0`, so a
/// trial can never change the tempo of the player's own world.
pub fn effective_bloom_time_scale(scenario_scale: Option<f32>, fresh_arena: bool) -> f32 {
    match scenario_scale {
        Some(s) if fresh_arena && s.is_finite() && s > 0.0 => s,
        _ => 1.0,
    }
}

// ── The recovery, in terms ────────────────────────────────────────────

/// The two terms a probe reads: life (`eco`) and people-together (`P`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Terms {
    pub life: f32,
    pub people: f32,
}

impl Terms {
    pub fn of(state: &ProbeState) -> Self {
        Self { life: state.eco, people: state.people }
    }

    /// Where these terms head: the bloom target, without the creative floor.
    #[cfg(test)]
    pub fn target(self) -> f32 {
        level::target(self.life, self.people, false)
    }
}

/// How much of a recovery each term did, as shares that sum to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recovery {
    pub start: Terms,
    pub end: Terms,
    pub life: f32,
    pub people: f32,
}

/// Split the rise in target from `start` to `end` between the two terms. Each
/// term's part is the average of its effect applied first and applied last
/// (a Shapley split), so the parts add up to the whole rise and neither term is
/// favoured by the order. A place that did not rise reads as all life.
pub fn recovery(start: Terms, end: Terms) -> Recovery {
    let (life, people) = parts(start, end);
    let (life, people) = (life.max(0.0), people.max(0.0));
    let total = life + people;
    let (life, people) = if total <= 1e-6 { (1.0, 0.0) } else { (life / total, people / total) };
    Recovery { start, end, life, people }
}

/// The raw Shapley parts `(life, people)`; they sum to the rise in target.
fn parts(start: Terms, end: Terms) -> (f32, f32) {
    let t = |life: f32, people: f32| level::target(life, people, false);
    let life = 0.5
        * ((t(end.life, start.people) - t(start.life, start.people))
            + (t(end.life, end.people) - t(start.life, end.people)));
    let people = 0.5
        * ((t(start.life, end.people) - t(start.life, start.people))
            + (t(end.life, end.people) - t(end.life, start.people)));
    (life, people)
}

/// The two shares are "both" when they are within this many points of each other.
const BOTH_WITHIN: f32 = 0.25;

impl Recovery {
    /// "life" and/or "people": the ways this recovery counts as having been done.
    pub fn ways(&self) -> Vec<&'static str> {
        if (self.life - self.people).abs() <= BOTH_WITHIN {
            vec!["life", "people"]
        } else if self.life > self.people {
            vec!["life"]
        } else {
            vec!["people"]
        }
    }

    /// The one sentence under the flower.
    pub fn sentence(&self) -> &'static str {
        match self.ways().as_slice() {
            ["life", "people"] => "You brought it back through life and people together.",
            ["people"] => "You brought it back mostly through people together.",
            _ => "You brought it back mostly through life.",
        }
    }

    /// The flower's petals as sizes in 0..=1, in the order life, ideas, people,
    /// time. Ideas and time are not built yet: they are always 0, and are drawn
    /// closed.
    pub fn petals(&self) -> [f32; 4] {
        [self.life, 0.0, self.people, 0.0]
    }
}

// ── The record of one run ─────────────────────────────────────────────

/// What a running scenario remembers about its `Place` leaves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaceTrack {
    /// Probes are registered with the field (set by `register_place_probes`).
    pub(crate) probes_live: bool,
    /// Per place leaf, `(flat index, terms at the start, terms at the end)`.
    leaves: Vec<(usize, Terms, Option<Terms>)>,
    /// Latest stage of each place leaf.
    stages: Vec<(usize, Stage)>,
    /// Satoshi's halfway line was offered.
    halfway: bool,
}

impl PlaceTrack {
    /// Note a fresh read of leaf `leaf`'s probe. The first read is the start.
    pub fn note(&mut self, leaf: usize, state: &ProbeState) {
        if !self.leaves.iter().any(|l| l.0 == leaf) {
            self.leaves.push((leaf, Terms::of(state), None));
        }
        match self.stages.iter_mut().find(|s| s.0 == leaf) {
            Some(s) => s.1 = state.stage,
            None => self.stages.push((leaf, state.stage)),
        }
    }

    /// The leaf finished with these end terms.
    pub fn finish(&mut self, leaf: usize, state: &ProbeState) {
        self.note(leaf, state);
        if let Some(l) = self.leaves.iter_mut().find(|l| l.0 == leaf) {
            l.2 = Some(Terms::of(state));
        }
    }

    /// The stage phrase of `leaf`, if it has been read.
    pub fn stage_of(&self, leaf: usize) -> Option<Stage> {
        self.stages.iter().find(|s| s.0 == leaf).map(|s| s.1)
    }

    /// The first read leaf's stage (the badge shows one phrase).
    pub fn first_stage(&self) -> Option<Stage> {
        self.stages.iter().min_by_key(|s| s.0).map(|s| s.1)
    }

    /// The recovery of the first finished place leaf.
    pub fn recovery(&self) -> Option<Recovery> {
        self.leaves
            .iter()
            .filter_map(|(i, start, end)| end.map(|e| (*i, *start, e)))
            .min_by_key(|(i, _, _)| *i)
            .map(|(_, start, end)| recovery(start, end))
    }

    /// True once, the first time any leaf has reached "Waking up" or beyond:
    /// the moment for Satoshi's halfway line.
    pub fn take_halfway(&mut self) -> bool {
        let reached = self.stages.iter().any(|s| matches!(s.1, Stage::WakingUp | Stage::Alive));
        if reached && !self.halfway {
            self.halfway = true;
            return true;
        }
        false
    }
}

// ── The scenario and the field ───────────────────────────────────────

use crate::bloom::BloomField;
use crate::scenario::ScenarioState;

/// Put a bloom probe over every `Place` leaf of `scn`, areas taken as offsets
/// from `origin` (the spawn cell).
pub fn register_probes(scn: &mut ScenarioState, bloom: &mut BloomField, origin: (i32, i32, i32)) {
    let leaves: Vec<(usize, PlaceArea, f32, u32)> = crate::objective_progress::place_leaves(&scn.def().objective)
        .into_iter()
        .map(|l| (l.index, *l.area, l.at_least, l.hold_seconds))
        .collect();
    if leaves.is_empty() {
        return;
    }
    for (index, area, at_least, hold) in leaves {
        let (min, max) = area.world_box(origin);
        bloom.add_probe(probe_id(index), min, max);
        bloom.set_probe_goal(probe_id(index), at_least, hold as f32);
    }
    scn.place_track_mut().probes_live = true;
}

/// Take `scn`'s probes off the field (the scenario ended or was replaced).
pub fn remove_probes(scn: &mut ScenarioState, bloom: &mut BloomField) {
    if !scn.place_track_mut().probes_live {
        return;
    }
    scn.place_track_mut().probes_live = false;
    for l in crate::objective_progress::place_leaves(&scn.def().objective) {
        bloom.remove_probe(probe_id(l.index));
    }
}

/// The grey starts of a fresh arena (`ArenaSetup::drained`): the initial level
/// of the probe and of the regions under each area.
pub fn apply_drained(bloom: &mut BloomField, arena: &ArenaSetup, origin: (i32, i32, i32)) {
    for d in &arena.drained {
        let (min, max) = d.area.world_box(origin);
        bloom.set_initial_level(min, max, d.level);
    }
}

/// "Try again" on a Place trial in its fresh arena: re-lay the arena's box
/// fills round `origin` (the terrain the player has dug, flooded or built
/// over) and re-apply its grey start, so the second run begins as the first
/// did. The arena's single blocks and mobs are NOT re-run (the chest, the
/// spring and the five animals are already there). Returns one cell per chunk
/// touched, for the caller to re-mesh. What the player planted above the fills
/// stays (a known limit).
pub fn rewind_arena(
    world: &mut crate::world::World,
    water: &mut crate::water::WaterSystem,
    lava: &mut crate::lava::LavaSystem,
    bloom: &mut BloomField,
    arena: &ArenaSetup,
    origin: (i32, i32, i32),
) -> Vec<(i32, i32, i32)> {
    let touched = crate::arena_build::build_fills(world, water, lava, origin, arena);
    apply_drained(bloom, arena, origin);
    touched
}

/// One tick of a running scenario's `Place` leaves: read each open leaf's
/// probe, remember its stage and terms, and finish the leaf when the probe
/// reads "Alive". Returns `true` once, when Satoshi's halfway line is due.
pub fn step(scn: &mut ScenarioState, bloom: &BloomField) -> bool {
    if !scn.place_track_mut().probes_live || scn.is_ended() {
        return false;
    }
    for index in scn.open_place_indices() {
        let Some(state) = bloom.probe(probe_id(index)) else { continue };
        if !state.scanned {
            continue;
        }
        scn.place_track_mut().note(index, &state);
        if state.stage == Stage::Alive {
            scn.complete_place(index, &state);
        }
    }
    scn.place_track_mut().take_halfway() && !scn.is_ended()
}

// ── GameState glue ────────────────────────────────────────────────────

impl crate::GameState {
    /// The block the area offsets are relative to: the first seat's feet cell
    /// (the same origin `apply_arena_setup` uses).
    fn place_origin(&self) -> (i32, i32, i32) {
        // In a fresh arena the origin is where the arena was laid, wherever
        // the player has walked since.
        if self.scenario_fresh_arena
            && let Some(spawn) = self.arena_spawn
        {
            return (spawn.x.floor() as i32, spawn.y.floor() as i32, spawn.z.floor() as i32);
        }
        let p = self.players[0].player.pos;
        (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
    }

    /// Put a bloom probe over every `Place` leaf of the running scenario. Called
    /// by `start_scenario` (the overlay and the arena launch alike).
    pub(crate) fn register_place_probes(&mut self) {
        let origin = self.place_origin();
        if let Some(scn) = self.scenario.as_mut() {
            register_probes(scn, &mut self.bloom, origin);
        }
    }

    /// Take the probes off the field (the scenario ended or was replaced).
    pub(crate) fn remove_place_probes(&mut self) {
        if let Some(scn) = self.scenario.as_mut() {
            remove_probes(scn, &mut self.bloom);
        }
    }

    /// The grey starts of a fresh arena. Fresh-world launch only; the
    /// `/scenario` overlay builds no arena and so never calls it.
    pub(crate) fn apply_arena_drained(&mut self, arena: &ArenaSetup) {
        let origin = self.place_origin();
        apply_drained(&mut self.bloom, arena, origin);
    }

    /// "Try again" on a Place trial in its fresh arena: back to the start,
    /// terrain re-laid and grey again (see [`rewind_arena`]). Run after
    /// `start_scenario` has made the fresh scenario state, with the fresh-arena
    /// flag and `arena_spawn` restored. No-op outside a fresh arena or for a
    /// scenario with no `Place` leaf.
    pub(crate) fn rewind_place_arena(&mut self) {
        let Some(spawn) = self.arena_spawn.filter(|_| self.scenario_fresh_arena) else { return };
        let Some(def) = self.scenario.as_ref().map(|s| s.def().clone()) else { return };
        let Some(arena) = def.arena.as_ref() else { return };
        if crate::objective_progress::place_leaves(&def.objective).is_empty() {
            return;
        }
        // The probes were laid at wherever the player stood: lay them at the arena.
        self.remove_place_probes();
        self.register_place_probes();
        let origin = self.place_origin();
        let touched = rewind_arena(&mut self.world, &mut self.water, &mut self.lava, &mut self.bloom, arena, origin);
        for (x, y, z) in touched {
            self.rebuild_chunk_at(x, y, z);
        }
        // Stand at the start again, not inside the re-laid stone.
        self.players[0].player.pos = spawn;
        self.players[0].player.velocity = glam::Vec3::ZERO;
        self.players[0].player.reset_fall();
    }

    /// Per tick, while a scenario with `Place` leaves runs (see [`step`]).
    /// Satoshi's halfway line is offered once.
    pub(crate) fn tick_place_trial(&mut self) {
        let Some(scn) = self.scenario.as_mut() else { return };
        if scn.is_ended() {
            self.remove_place_probes();
            return;
        }
        if !step(scn, &self.bloom) {
            return;
        }
        let display = scn.display_name().to_string();
        let (_intro, hint) = crate::scenario::trial_satoshi_for_display(&display);
        if !hint.is_empty() {
            let title = display.split_once(" \u{2014} ").map(|(t, _)| t).unwrap_or(&display).to_string();
            self.satoshi_brief =
                Some((title, hint.to_string(), web_time::Instant::now() + std::time::Duration::from_secs(8)));
        }
    }

    /// The tempo the bloom field runs at this tick (see
    /// [`effective_bloom_time_scale`]).
    pub(crate) fn bloom_time_scale_now(&self) -> f32 {
        effective_bloom_time_scale(
            self.scenario.as_ref().map(|s| s.def().tuning.bloom_time_scale),
            self.scenario_fresh_arena,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(life: f32, people: f32) -> Terms {
        Terms { life, people }
    }

    #[test]
    fn a_scenario_sets_the_tempo_only_in_a_fresh_arena() {
        assert_eq!(effective_bloom_time_scale(Some(12.0), true), 12.0);
        // The in-world overlay: a scenario, but not a fresh trial arena.
        assert_eq!(effective_bloom_time_scale(Some(12.0), false), 1.0);
        // No scenario at all.
        assert_eq!(effective_bloom_time_scale(None, true), 1.0);
        assert_eq!(effective_bloom_time_scale(None, false), 1.0);
        // A broken number never stalls or races the world.
        for bad in [0.0, -3.0, f32::NAN, f32::INFINITY] {
            assert_eq!(effective_bloom_time_scale(Some(bad), true), 1.0, "{bad}");
        }
    }

    #[test]
    fn the_shares_add_up_to_one_and_follow_the_term_that_moved() {
        let life = recovery(terms(0.05, 0.0), terms(0.7, 0.0));
        assert!((life.life + life.people - 1.0).abs() < 1e-5);
        assert_eq!(life.ways(), vec!["life"]);
        assert!(life.sentence().ends_with("mostly through life."));

        let people = recovery(terms(0.05, 0.0), terms(0.05, 0.9));
        assert_eq!(people.ways(), vec!["people"]);
        assert!(people.sentence().ends_with("mostly through people together."), "{}", people.sentence());

        // Both rose by about the same.
        let both = recovery(terms(0.0, 0.0), terms(0.4, 0.55));
        assert!((both.life - both.people).abs() <= BOTH_WITHIN, "{both:?}");
        assert_eq!(both.ways(), vec!["life", "people"]);
        assert!(both.sentence().ends_with("through life and people together."));
    }

    #[test]
    fn a_split_is_the_same_whichever_term_is_counted_first() {
        // The Shapley split sums to the whole rise in target.
        let (a, b) = (terms(0.1, 0.05), terms(0.6, 0.7));
        let r = recovery(a, b);
        let rise = b.target() - a.target();
        assert!(rise > 0.3);
        let (l, p) = parts(a, b);
        assert!((l + p - rise).abs() < 1e-5, "{l} + {p} vs {rise}");
        assert!((r.life + r.people - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_place_that_did_not_rise_reads_as_life_and_ideas_and_time_stay_closed() {
        let r = recovery(terms(0.5, 0.0), terms(0.5, 0.0));
        assert_eq!((r.life, r.people), (1.0, 0.0));
        let p = r.petals();
        assert_eq!(p[1], 0.0, "ideas is closed");
        assert_eq!(p[3], 0.0, "time is closed");
        assert_eq!(p[0] + p[2], 1.0);
    }

    fn state(eco: f32, people: f32, stage: Stage) -> ProbeState {
        ProbeState { level: 0.0, target: 0.0, eco, people, stage, held_secs: 0.0, scanned: true }
    }

    #[test]
    fn the_track_keeps_the_first_read_as_the_start_and_the_finish_as_the_end() {
        let mut t = PlaceTrack::default();
        assert!(t.recovery().is_none());
        t.note(0, &state(0.05, 0.0, Stage::GreyAndStill));
        t.note(0, &state(0.3, 0.0, Stage::Stirring));
        assert_eq!(t.stage_of(0), Some(Stage::Stirring));
        assert!(!t.take_halfway(), "still only stirring");
        t.note(0, &state(0.5, 0.0, Stage::WakingUp));
        assert!(t.take_halfway());
        assert!(!t.take_halfway(), "offered once");
        t.finish(0, &state(0.7, 0.0, Stage::Alive));
        let r = t.recovery().expect("finished");
        assert_eq!(r.start, terms(0.05, 0.0));
        assert_eq!(r.end, terms(0.7, 0.0));
        assert_eq!(r.ways(), vec!["life"]);
    }
}

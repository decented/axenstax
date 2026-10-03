//! Spec 36 Plot Ownership integration tests.
//!
//! End-to-end coverage over World.plots claim/release + the
//! foreign-plot predicate + save/load round-trip. Pure region maths
//! are covered in `src/plot.rs`.

use crate::plot::{self, PlotData, PlotOwner};
use crate::world::World;

#[test]
fn claim_pushes_plot_and_protects_region() {
    let mut world = World::new();
    world.plots.push(PlotData::from_marker(
        PlotOwner::LocalPlayer(0), 0, 64, 0,
    ));
    // Player 0 owns it → not foreign to them.
    assert!(!plot::is_in_foreign_plot(&world.plots, 5, 5, 0));
    // Player 1 is locked out.
    assert!(plot::is_in_foreign_plot(&world.plots, 5, 5, 1));
    // plot_at_column finds it.
    assert!(world.plot_at_column(5, 5).is_some());
    assert!(world.plot_at_column(10_000, 10_000).is_none());
}

#[test]
fn release_on_marker_break_removes_plot() {
    let mut world = World::new();
    world.plots.push(PlotData::from_marker(
        PlotOwner::LocalPlayer(0), 0, 64, 0,
    ));
    assert_eq!(world.plots.len(), 1);
    world.release_plot((0, 64, 0));
    assert!(world.plots.is_empty(), "releasing the marker clears the plot");
    // After release the region is no longer protected.
    assert!(!plot::is_in_foreign_plot(&world.plots, 5, 5, 1));
}

#[test]
fn release_only_removes_the_matching_marker() {
    let mut world = World::new();
    world.plots.push(PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0));
    world.plots.push(PlotData::from_marker(PlotOwner::LocalPlayer(0), 100, 64, 100));
    world.release_plot((0, 64, 0));
    assert_eq!(world.plots.len(), 1, "only the matched marker's plot is removed");
    assert_eq!(world.plots[0].marker, (100, 64, 100));
}

#[test]
fn conflict_rejects_overlapping_foreign_claim() {
    let mut world = World::new();
    world.plots.push(PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0));
    // Player 1 can't claim overlapping land.
    assert!(plot::claim_would_conflict(
        &world.plots, &PlotOwner::LocalPlayer(1), 10, 10,
    ));
    // Player 0 (same owner) can claim adjacent land.
    assert!(!plot::claim_would_conflict(
        &world.plots, &PlotOwner::LocalPlayer(0), 10, 10,
    ));
    // Anyone can claim far away.
    assert!(!plot::claim_would_conflict(
        &world.plots, &PlotOwner::LocalPlayer(1), 5000, 5000,
    ));
}

#[test]
fn plots_round_trip_save_load() {
    // Populate plots, serialise via the WorldSave shape, restore,
    // assert preserved. PlotData is plain serde so it round-trips
    // directly (no Saved* mirror).
    let plots = vec![
        PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0),
        PlotData::from_marker(PlotOwner::Npub("npub1abc".to_string()), 200, 70, -200),
    ];
    let bytes = bincode::serialize(&plots).expect("serialize");
    let back: Vec<PlotData> = bincode::deserialize(&bytes).expect("deserialize");
    assert_eq!(back.len(), 2);
    assert_eq!(back[0].owner, PlotOwner::LocalPlayer(0));
    assert_eq!(back[0].marker, (0, 64, 0));
    assert_eq!(back[1].owner, PlotOwner::Npub("npub1abc".to_string()));
    assert_eq!(back[1].marker, (200, 70, -200));
}

#[test]
fn creative_bypass_is_a_caller_concern_not_in_predicate() {
    // The predicate itself doesn't know about creative — the game
    // loop applies `!self.is_creative &&` before calling. This test
    // documents that the predicate purely reflects ownership, so a
    // creative-bypass regression would show up at the call site, not
    // here.
    let plots = vec![PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0)];
    // Even "as a different player" the predicate says foreign — the
    // creative bypass is the `!is_creative` guard in game_loop.
    assert!(plot::is_in_foreign_plot(&plots, 5, 5, 1));
}

#[test]
fn world_clear_resets_plots() {
    let mut world = World::new();
    world.plots.push(PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0));
    world.clear();
    assert!(world.plots.is_empty(), "clear() must reset plots");
}

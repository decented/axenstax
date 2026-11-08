//! Living world S3 — Commons (spec `docs/superpowers/specs/2026-11-08-living-world-tier1.md` §4).
//!
//! A [`block::COMMONS_STONE`] declares a Commons: the same 33 x 33 footprint as
//! a Plot (marker +-`plot::PLOT_HALF_EXTENT`), open to everyone to build in.
//! At Tier 1 that was all it was: a declaration and a bloom lift (the `k` term
//! and the doubled co-presence rate). Since Tier 2 S2 a block built there can be
//! broken only by its author (`authorship`, `World::commons_protects`); the
//! stone itself, and any block with no recorded author, can be broken by anyone.
//!
//! The one rule is that a Commons and a Plot may not overlap, either way round:
//! a Commons Stone whose footprint touches any Plot is refused, and so is a Plot
//! Marker whose footprint touches a Commons. These pure predicates are the rule;
//! the local placement arm (`game_loop.rs`) and the authority
//! (`HostedServer::validate_block_edit`) both ask them, so single-player, a
//! host's seats and a joiner are refused the same way.
//!
//! Commons areas are found from the loaded chunks (`World::commons_areas`,
//! which walks only chunks whose maintained counter says they hold a stone), and
//! are stored nowhere else.

use crate::bloom::CommonsArea;
use crate::plot::{PlotData, PLOT_HALF_EXTENT};

/// The toast for a placement refused for overlapping a claim.
pub const CLAIMED_TOAST: &str = "This land is claimed.";

/// The footprint of a Commons Stone at column `(x, z)`.
pub fn footprint(x: i32, z: i32) -> CommonsArea {
    CommonsArea::from_marker(x, z)
}

/// Would a Commons Stone at `(x, z)` overlap any Plot?
pub fn stone_overlaps_a_plot(plots: &[PlotData], x: i32, z: i32) -> bool {
    let c = footprint(x, z);
    plots.iter().any(|p| c.overlaps(p.min_x, p.max_x, p.min_z, p.max_z))
}

/// Would a Plot Marker at `(x, z)` claim ground a Commons already covers?
pub fn plot_overlaps_a_commons(areas: &[CommonsArea], x: i32, z: i32) -> bool {
    areas.iter().any(|c| c.overlaps(x - PLOT_HALF_EXTENT, x + PLOT_HALF_EXTENT, z - PLOT_HALF_EXTENT, z + PLOT_HALF_EXTENT))
}

/// Would placing `block` at column `(x, z)` break a claim rule? The Commons
/// Stone over a Plot, the Plot Marker over a Commons, and a Feast Marker (a
/// record only a held feast makes) are refused. The one predicate for a Plan's
/// builder, which has no player to ask (`plan::tick_build`, review L1).
pub fn placement_refused(world: &crate::world::World, block: crate::block::BlockId, x: i32, z: i32) -> bool {
    match block {
        crate::block::COMMONS_STONE => stone_overlaps_a_plot(&world.plots, x, z),
        crate::block::PLOT_MARKER => plot_overlaps_a_commons(&world.commons_areas(), x, z),
        crate::block::FEAST_MARKER => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plot::PlotOwner;
    use crate::world::World;

    fn plot_at(x: i32, z: i32) -> PlotData {
        PlotData::from_marker(PlotOwner::LocalPlayer(0), x, 64, z)
    }

    #[test]
    fn the_footprint_is_a_plots_33_by_33() {
        let c = footprint(100, -50);
        assert_eq!((c.min_x, c.max_x, c.min_z, c.max_z), (100 - PLOT_HALF_EXTENT, 100 + PLOT_HALF_EXTENT, -50 - PLOT_HALF_EXTENT, -50 + PLOT_HALF_EXTENT));
        assert_eq!(c.max_x - c.min_x + 1, 33);
        let p = plot_at(100, -50);
        assert_eq!((c.min_x, c.max_x, c.min_z, c.max_z), (p.min_x, p.max_x, p.min_z, p.max_z), "the same square as a plot");
    }

    #[test]
    fn a_stone_over_a_plot_is_refused_and_one_beside_it_is_not() {
        let plots = [plot_at(0, 0)];
        assert!(stone_overlaps_a_plot(&plots, 0, 0));
        assert!(stone_overlaps_a_plot(&plots, 32, 32), "corners touching overlap (inclusive bounds)");
        assert!(!stone_overlaps_a_plot(&plots, 34, 0), "34 apart clears two 33-wide squares");
        assert!(!stone_overlaps_a_plot(&[], 0, 0));
        // Even the plot's own owner may not put a Commons over their plot.
        assert!(stone_overlaps_a_plot(&plots, 5, -5));
    }

    #[test]
    fn a_plot_over_a_commons_is_refused_and_one_beside_it_is_not() {
        let areas = [footprint(0, 0)];
        assert!(plot_overlaps_a_commons(&areas, 10, 10));
        assert!(plot_overlaps_a_commons(&areas, -32, 0));
        assert!(!plot_overlaps_a_commons(&areas, 0, 34));
        assert!(!plot_overlaps_a_commons(&[], 0, 0));
    }

    #[test]
    fn two_commons_may_overlap_each_other() {
        // Only a Plot conflicts (a Commons is open ground; overlapping ones are
        // simply more of it).
        let areas = [footprint(0, 0)];
        assert!(footprint(10, 10).overlaps(areas[0].min_x, areas[0].max_x, areas[0].min_z, areas[0].max_z));
        assert!(!stone_overlaps_a_plot(&[], 10, 10));
    }

    #[test]
    fn a_world_finds_its_commons_only_where_stones_stand() {
        let mut w = World::new();
        assert!(w.commons_areas().is_empty());
        w.set_block(40, 70, -20, crate::block::COMMONS_STONE);
        w.set_block(-100, 64, 7, crate::block::COMMONS_STONE);
        w.set_block(5, 64, 5, crate::block::STONE);
        let areas = w.commons_areas();
        assert_eq!(areas.len(), 2);
        assert!(areas.contains(&footprint(40, -20)) && areas.contains(&footprint(-100, 7)));
        assert!(w.commons_overlaps(30, 30, -20, -20));
        assert!(!w.commons_overlaps(0, 0, 0, 0));
        // Breaking a stone takes its area with it.
        w.set_block(40, 70, -20, crate::block::AIR);
        assert_eq!(w.commons_areas(), vec![footprint(-100, 7)]);
    }

    #[test]
    fn a_plan_may_not_stamp_a_claim_the_placement_arm_would_refuse() {
        use crate::block::{COMMONS_STONE, FEAST_MARKER, PLOT_MARKER, STONE};
        let mut w = World::new();
        w.plots.push(plot_at(0, 0));
        w.set_block(200, 64, 200, COMMONS_STONE);
        assert!(placement_refused(&w, COMMONS_STONE, 5, 5), "a stone over a plot");
        assert!(!placement_refused(&w, COMMONS_STONE, 100, 100), "a stone on clear ground");
        assert!(placement_refused(&w, PLOT_MARKER, 210, 210), "a plot over a commons");
        assert!(!placement_refused(&w, PLOT_MARKER, 100, 100));
        assert!(placement_refused(&w, FEAST_MARKER, 100, 100), "a marker is a record, never stamped");
        assert!(!placement_refused(&w, STONE, 5, 5));
    }

    #[test]
    fn no_copy_here_carries_a_money_word() {
        let banned: std::collections::HashSet<&str> = crate::copy_lint::BANNED_MONEY_WORDS.iter().copied().collect();
        for w in CLAIMED_TOAST.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
            assert!(!banned.contains(w.to_lowercase().as_str()), "{w:?}");
        }
    }
}

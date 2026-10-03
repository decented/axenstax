//! Plot Ownership v1 — first land economy primitive (Spec 36).
//!
//! A Plot Marker block claims a fixed-size square region owned by the
//! placer. Non-owners can't place/break inside (anti-grief build-
//! protection); owner + creative bypass. Pure region + ownership
//! helpers; the game-loop applies the place/break gates + creative
//! bypass.
//!
//! `World.plots: Vec<PlotData>` is the side-table — linear scan, fine
//! at alpha plot counts (mirrors the salt_licks / village_anchors
//! posture).
//!
//! Spec: `docs/foundations/2026-05-23-plot-ownership.md`.

use serde::{Deserialize, Serialize};

/// Half-extent of a claimed plot in columns. 16 → a 32×32 footprint
/// (the marker column + 15 each way on the low side, 16 on the high
/// side via the inclusive bound below… see `from_marker`). Server-
/// tunable.
pub const PLOT_HALF_EXTENT: i32 = 16;

/// Owner of a plot. `LocalPlayer(pidx)` on alpha; `Npub(String)`
/// declared for the Spec-1-Phase-4 cutover (converge with
/// VendorOwner + TipJarOwner — all three share the split-screen →
/// solo-reload edge case).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlotOwner {
    LocalPlayer(usize),
    Npub(String),
}

/// One claimed plot — an axis-aligned square in the XZ plane, full
/// height. `marker` is the claim anchor (the PLOT_MARKER block); the
/// plot is released when that block is broken by its owner.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlotData {
    pub owner: PlotOwner,
    pub marker: (i32, i32, i32),
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
}

impl PlotData {
    /// Build a plot centred on a marker at `(mx, my, mz)`. The region
    /// spans `[mx - HALF, mx + HALF]` × `[mz - HALF, mz + HALF]`
    /// inclusive → a `(2*HALF + 1)`-column square.
    pub fn from_marker(owner: PlotOwner, mx: i32, my: i32, mz: i32) -> Self {
        PlotData {
            owner,
            marker: (mx, my, mz),
            min_x: mx - PLOT_HALF_EXTENT,
            max_x: mx + PLOT_HALF_EXTENT,
            min_z: mz - PLOT_HALF_EXTENT,
            max_z: mz + PLOT_HALF_EXTENT,
        }
    }

    /// Does this plot's footprint contain column `(x, z)`?
    pub fn contains_column(&self, x: i32, z: i32) -> bool {
        x >= self.min_x && x <= self.max_x && z >= self.min_z && z <= self.max_z
    }

    /// Do two plots' footprints overlap (AABB intersection)?
    pub fn overlaps(&self, other: &PlotData) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_z <= other.max_z
            && self.max_z >= other.min_z
    }
}

/// True iff `owner` is the given local player.
pub fn is_local_owner(owner: &PlotOwner, pidx: usize) -> bool {
    matches!(owner, PlotOwner::LocalPlayer(p) if *p == pidx)
}

/// Is column `(x, z)` inside a plot owned by someone OTHER than
/// `pidx`? The build-protection predicate. The caller applies the
/// creative bypass (creative players ignore the gate entirely).
pub fn is_in_foreign_plot(plots: &[PlotData], x: i32, z: i32, pidx: usize) -> bool {
    plots.iter().any(|p| {
        p.contains_column(x, z) && !is_local_owner(&p.owner, pidx)
    })
}

/// True iff `owner` is the given verified npub. `None` (a guest) owns nothing.
/// A `LocalPlayer` owner is a seat on the host's own machine, never a remote
/// joiner.
pub fn owner_is_npub(owner: &PlotOwner, npub: Option<&str>) -> bool {
    matches!((owner, npub), (PlotOwner::Npub(n), Some(m)) if n == m)
}

/// The host's build-protection predicate for a REMOTE joiner identified by
/// `npub` (`None` = guest): is column `(x, z)` inside a plot they don't own?
/// The sibling of [`is_in_foreign_plot`], which keys on local seat indexes.
/// The caller applies the creative bypass.
pub fn is_in_foreign_plot_for_npub(plots: &[PlotData], x: i32, z: i32, npub: Option<&str>) -> bool {
    plots
        .iter()
        .any(|p| p.contains_column(x, z) && !owner_is_npub(&p.owner, npub))
}

/// Would a new plot anchored at `(mx, mz)` overlap any FOREIGN plot?
/// Used at place-time to reject conflicting claims. Overlap with the
/// claimant's own plots is allowed (a player can claim adjacent land).
pub fn claim_would_conflict(plots: &[PlotData], owner: &PlotOwner, mx: i32, mz: i32) -> bool {
    let candidate = PlotData::from_marker(owner.clone(), mx, 0, mz);
    plots.iter().any(|p| {
        candidate.overlaps(p) && p.owner != *owner
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_marker_centres_region() {
        let p = PlotData::from_marker(PlotOwner::LocalPlayer(0), 100, 64, -50);
        assert_eq!(p.min_x, 100 - PLOT_HALF_EXTENT);
        assert_eq!(p.max_x, 100 + PLOT_HALF_EXTENT);
        assert_eq!(p.min_z, -50 - PLOT_HALF_EXTENT);
        assert_eq!(p.max_z, -50 + PLOT_HALF_EXTENT);
        assert_eq!(p.marker, (100, 64, -50));
    }

    #[test]
    fn contains_column_inside_and_edges() {
        let p = PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0);
        assert!(p.contains_column(0, 0), "centre");
        assert!(p.contains_column(PLOT_HALF_EXTENT, PLOT_HALF_EXTENT), "max corner (inclusive)");
        assert!(p.contains_column(-PLOT_HALF_EXTENT, -PLOT_HALF_EXTENT), "min corner");
        assert!(!p.contains_column(PLOT_HALF_EXTENT + 1, 0), "just outside +x");
        assert!(!p.contains_column(0, PLOT_HALF_EXTENT + 1), "just outside +z");
    }

    #[test]
    fn is_in_foreign_plot_true_for_other_owner() {
        let plots = vec![PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0)];
        // Player 1 is inside player 0's plot.
        assert!(is_in_foreign_plot(&plots, 5, 5, 1));
    }

    #[test]
    fn is_in_foreign_plot_false_for_own_plot() {
        let plots = vec![PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0)];
        // Player 0 is inside their own plot — not foreign.
        assert!(!is_in_foreign_plot(&plots, 5, 5, 0));
    }

    #[test]
    fn is_in_foreign_plot_false_outside() {
        let plots = vec![PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0)];
        assert!(!is_in_foreign_plot(&plots, 1000, 1000, 1));
    }

    #[test]
    fn claim_would_conflict_rejects_overlap_with_foreign() {
        let plots = vec![PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0)];
        // Player 1 tries to claim right next to player 0's marker —
        // overlapping footprint → conflict.
        assert!(claim_would_conflict(&plots, &PlotOwner::LocalPlayer(1), 5, 5));
        // Far away → no conflict.
        assert!(!claim_would_conflict(&plots, &PlotOwner::LocalPlayer(1), 1000, 1000));
    }

    #[test]
    fn claim_would_conflict_allows_overlap_with_own() {
        let plots = vec![PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0)];
        // Player 0 claiming adjacent land that overlaps their OWN plot
        // is allowed (no conflict).
        assert!(!claim_would_conflict(&plots, &PlotOwner::LocalPlayer(0), 5, 5));
    }

    #[test]
    fn remote_joiner_owns_only_their_npub_plots() {
        let plots = vec![
            PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0),
            PlotData::from_marker(PlotOwner::Npub("npub1me".into()), 1000, 64, 0),
        ];
        // A host-seat plot is foreign to every joiner, verified or guest.
        assert!(is_in_foreign_plot_for_npub(&plots, 5, 5, Some("npub1me")));
        assert!(is_in_foreign_plot_for_npub(&plots, 5, 5, None));
        // Their own npub plot isn't; someone else's (or a guest) is.
        assert!(!is_in_foreign_plot_for_npub(&plots, 1000, 5, Some("npub1me")));
        assert!(is_in_foreign_plot_for_npub(&plots, 1000, 5, Some("npub1other")));
        assert!(is_in_foreign_plot_for_npub(&plots, 1000, 5, None));
        // Open land is nobody's.
        assert!(!is_in_foreign_plot_for_npub(&plots, 500, 500, None));
    }

    #[test]
    fn is_local_owner_matches_pidx() {
        assert!(is_local_owner(&PlotOwner::LocalPlayer(3), 3));
        assert!(!is_local_owner(&PlotOwner::LocalPlayer(3), 0));
        assert!(!is_local_owner(&PlotOwner::Npub("npub1x".to_string()), 0));
    }

    #[test]
    fn overlaps_is_symmetric_and_correct() {
        let a = PlotData::from_marker(PlotOwner::LocalPlayer(0), 0, 64, 0);
        let b = PlotData::from_marker(PlotOwner::LocalPlayer(1), 10, 64, 10);
        let far = PlotData::from_marker(PlotOwner::LocalPlayer(2), 1000, 64, 1000);
        assert!(a.overlaps(&b) && b.overlaps(&a), "adjacent plots overlap");
        assert!(!a.overlaps(&far) && !far.overlaps(&a), "distant plots don't");
    }
}

//! Living world Tier 2 S4 — how often each Plan has been built in this world
//! (spec `docs/superpowers/specs/2026-11-09-living-world-tier2.md` §4).
//!
//! A table keyed by a Plan's content hash ([`crate::plan::content_hash`], the
//! hash its derivation chain already carries): `built` is how many times that
//! exact design was built here, `built_from` how many builds descended from it
//! (it appears earlier in the built Plan's derivation chain). It lives on the
//! machine that owns the world, is saved as one appended `WorldSave` field,
//! and holds no identities: nothing in it says who built anything. It is never
//! sent on the wire, so a joined client's plaque shows the lineage but no
//! counts.
//!
//! One pure function feeds it, [`PlanBuilds::note_completed`]. `plan::complete_build`
//! calls it, and every completion (a player's build through
//! `sim_machines::tick_construction`, an NPC commission through `builder.rs`)
//! goes through `complete_build`, so a cancelled or refunded build, which never
//! reaches it, counts nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::plan::PlanData;

/// A Plan's content hash.
pub type PlanHash = [u8; 32];

/// Most designs kept. Past it, a new design evicts the one with the fewest
/// counts (the lowest hash on a tie), so a table of throwaway Plans cannot grow
/// without bound.
pub const MAX_ROWS: usize = 8_192;
/// Most distinct ancestors one completion credits (a chain is short in
/// practice; the cap bounds a crafted one).
pub const MAX_ANCESTORS: usize = 64;

/// What the world has seen of one design.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanCounts {
    /// Times this design was built here.
    pub built: u32,
    /// Builds of designs derived from it.
    pub built_from: u32,
}

impl PlanCounts {
    fn weight(&self) -> u64 {
        u64::from(self.built) + u64::from(self.built_from)
    }
}

/// The world's Plan build counts. Ordered, so saving and digesting are canonical.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanBuilds {
    rows: BTreeMap<PlanHash, PlanCounts>,
}

/// The hash a Plan's builds are counted under: the last link of its derivation
/// chain, the hash frozen when it was captured, which is the key its plaque
/// reads. `content_hash` is not that key for a licence copy (`vendor.rs` clears
/// `is_master`, which the hash covers), so counting under the recomputed hash
/// would put a copy's builds on a row nothing shows. A Plan with no chain falls
/// back to the recomputed hash.
pub fn build_key(plan: &PlanData) -> PlanHash {
    plan.derivation_chain
        .last()
        .map_or_else(|| crate::plan::content_hash(plan), |link| link.plan_hash)
}

/// The distinct ancestor hashes of `plan`: every `plan_hash` in its derivation
/// chain before its own (last) link, except its own key, each once, in chain
/// order, at most [`MAX_ANCESTORS`]. A chain that repeats a hash, or a
/// derivative whose cells equal its parent's (so the parent's hash is its own),
/// credits nothing twice.
pub fn ancestors_of(plan: &PlanData) -> Vec<PlanHash> {
    let own = build_key(plan);
    let before_own = plan.derivation_chain.len().saturating_sub(1);
    let mut out: Vec<PlanHash> = Vec::new();
    for link in &plan.derivation_chain[..before_own] {
        if link.plan_hash != own && !out.contains(&link.plan_hash) {
            out.push(link.plan_hash);
            if out.len() >= MAX_ANCESTORS {
                break;
            }
        }
    }
    out
}

impl PlanBuilds {
    /// Nothing recorded.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Forget everything (a world reset).
    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// The counts for one design (zero for a design never built here).
    pub fn counts(&self, hash: &PlanHash) -> PlanCounts {
        self.rows.get(hash).copied().unwrap_or_default()
    }

    /// A build of the design `plan_hash` completed. `built` goes up for it and
    /// `built_from` for each distinct hash in `ancestors` (a repeat counts once,
    /// and the design's own hash is not its own ancestor). When the table is full,
    /// room is made for the whole batch first, never at the expense of a row this
    /// completion is writing.
    pub fn note_completed(&mut self, plan_hash: PlanHash, ancestors: &[PlanHash]) {
        let mut keys: Vec<PlanHash> = Vec::with_capacity(1 + ancestors.len().min(MAX_ANCESTORS));
        keys.push(plan_hash);
        for a in ancestors {
            if keys.contains(a) {
                continue;
            }
            if keys.len() > MAX_ANCESTORS {
                break;
            }
            keys.push(*a);
        }
        self.make_room(&keys);
        let own = self.rows.entry(plan_hash).or_default();
        own.built = own.built.saturating_add(1);
        for a in &keys[1..] {
            let row = self.rows.entry(*a).or_default();
            row.built_from = row.built_from.saturating_add(1);
        }
    }

    /// World totals for bloom's "ideas" term and the grown-up panel: how many
    /// distinct designs have been built here, and how many builds in all.
    #[allow(dead_code)] // BRIDGE: read by the S9 panel — remove this when it lands.
    pub fn ideas_total(&self) -> (u32, u32) {
        let designs = self.rows.values().filter(|c| c.built > 0).count() as u32;
        let builds = self.rows.values().fold(0u32, |n, c| n.saturating_add(c.built));
        (designs, builds)
    }

    /// Make room for the rows in `keys` that are not yet in the table: evict the
    /// least-counted others (the lowest hash on a tie) until they all fit. A row
    /// in `keys` is never the victim.
    fn make_room(&mut self, keys: &[PlanHash]) {
        let incoming = keys.iter().filter(|k| !self.rows.contains_key(*k)).count();
        while self.rows.len() + incoming > MAX_ROWS {
            let victim = self
                .rows
                .iter()
                .filter(|(h, _)| !keys.contains(h))
                .min_by_key(|(h, c)| (c.weight(), **h))
                .map(|(h, _)| *h);
            let Some(v) = victim else { break };
            self.rows.remove(&v);
        }
    }

    /// The canonical saved form; `None` when nothing is recorded.
    pub fn to_saved(&self) -> Option<SavedPlanBuilds> {
        if self.rows.is_empty() {
            return None;
        }
        Some(SavedPlanBuilds {
            rows: self.rows.iter().map(|(h, c)| (*h, c.built, c.built_from)).collect(),
        })
    }

    /// Back from a save. A row that counts nothing, repeats a hash or lies past
    /// the table's bound is dropped with a warning; the table never refuses to
    /// load.
    pub fn from_saved(saved: &SavedPlanBuilds) -> Self {
        let mut out = Self::default();
        let mut dropped = 0usize;
        for (hash, built, built_from) in &saved.rows {
            if (*built == 0 && *built_from == 0) || out.rows.len() >= MAX_ROWS || out.rows.contains_key(hash) {
                dropped += 1;
                continue;
            }
            out.rows.insert(*hash, PlanCounts { built: *built, built_from: *built_from });
        }
        if dropped > 0 {
            log::warn!("world.dat: {dropped} invalid Plan build-count row(s) dropped");
        }
        out
    }
}

/// The saved table (`WorldSave.plan_builds`). Positional bincode: a change to
/// this type is a `SAVE_LAYOUT_REVISION` bump. Holds no identities, so a
/// handover's transfer copy and an export keep it as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPlanBuilds {
    /// `(content hash, built, built_from)`, sorted by hash.
    pub rows: Vec<(PlanHash, u32, u32)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{DerivationLink, PlanLicense};

    fn h(n: u8) -> PlanHash {
        [n; 32]
    }

    fn link(name: &str, author: &str, hash: PlanHash) -> DerivationLink {
        DerivationLink {
            author_npub: author.to_string(),
            plan_name: name.to_string(),
            license: PlanLicense::default(),
            captured_at: 0,
            plan_hash: hash,
        }
    }

    #[test]
    fn a_build_counts_its_hash_once_and_each_distinct_ancestor_once() {
        let mut t = PlanBuilds::default();
        t.note_completed(h(3), &[h(1), h(2)]);
        assert_eq!(t.counts(&h(3)), PlanCounts { built: 1, built_from: 0 });
        assert_eq!(t.counts(&h(1)), PlanCounts { built: 0, built_from: 1 });
        assert_eq!(t.counts(&h(2)), PlanCounts { built: 0, built_from: 1 });
        t.note_completed(h(3), &[h(1), h(2)]);
        assert_eq!(t.counts(&h(3)).built, 2);
        assert_eq!(t.counts(&h(1)).built_from, 2);
    }

    #[test]
    fn a_repeated_ancestor_in_the_chain_counts_once_and_a_plan_is_not_its_own_ancestor() {
        let mut t = PlanBuilds::default();
        t.note_completed(h(3), &[h(1), h(1), h(2), h(1), h(3)]);
        assert_eq!(t.counts(&h(1)).built_from, 1);
        assert_eq!(t.counts(&h(2)).built_from, 1);
        assert_eq!(t.counts(&h(3)), PlanCounts { built: 1, built_from: 0 });
    }

    #[test]
    fn nothing_noted_counts_nothing() {
        // A cancelled or refunded build never reaches `note_completed`.
        let t = PlanBuilds::default();
        assert!(t.is_empty());
        assert_eq!(t.counts(&h(1)), PlanCounts::default());
        assert_eq!(t.ideas_total(), (0, 0));
        assert_eq!(t.to_saved(), None);
    }

    #[test]
    fn ancestors_of_a_plan_are_its_chain_less_itself_and_repeats() {
        let mut plan = PlanData::debug_3x3_stone();
        let own = crate::plan::content_hash(&plan);
        plan.derivation_chain = vec![link("a", "", h(1)), link("b", "", h(2)), link("a again", "", h(1)), link("me", "", own)];
        assert_eq!(ancestors_of(&plan), vec![h(1), h(2)]);
        plan.derivation_chain.clear();
        assert!(ancestors_of(&plan).is_empty());
    }

    #[test]
    fn ideas_total_counts_designs_built_and_builds() {
        let mut t = PlanBuilds::default();
        t.note_completed(h(3), &[h(1)]);
        t.note_completed(h(3), &[h(1)]);
        t.note_completed(h(4), &[]);
        // h(1) was only ever an ancestor: not a design built here.
        assert_eq!(t.ideas_total(), (2, 3));
    }

    #[test]
    fn the_saved_form_is_canonical_whatever_the_insert_order() {
        let mut a = PlanBuilds::default();
        let mut b = PlanBuilds::default();
        a.note_completed(h(9), &[h(2)]);
        a.note_completed(h(5), &[]);
        b.note_completed(h(5), &[]);
        b.note_completed(h(9), &[h(2)]);
        let (sa, sb) = (a.to_saved().unwrap(), b.to_saved().unwrap());
        assert_eq!(bincode::serialize(&sa).unwrap(), bincode::serialize(&sb).unwrap());
        assert!(sa.rows.windows(2).all(|w| w[0].0 < w[1].0), "sorted by hash");
        assert_eq!(PlanBuilds::from_saved(&sa), a);
    }

    #[test]
    fn invalid_saved_rows_are_dropped_not_refused() {
        let saved = SavedPlanBuilds { rows: vec![(h(1), 2, 0), (h(1), 5, 5), (h(2), 0, 0), (h(3), 0, 4)] };
        let t = PlanBuilds::from_saved(&saved);
        assert_eq!(t.counts(&h(1)), PlanCounts { built: 2, built_from: 0 }, "the first of a repeat wins");
        assert_eq!(t.counts(&h(2)), PlanCounts::default(), "an all-zero row is not a row");
        assert_eq!(t.counts(&h(3)), PlanCounts { built: 0, built_from: 4 });
    }

    #[test]
    fn the_table_is_bounded_and_evicts_the_least_counted() {
        let mut t = PlanBuilds::default();
        for i in 0..MAX_ROWS as u32 {
            let mut hash = [0u8; 32];
            hash[..4].copy_from_slice(&i.to_be_bytes());
            t.note_completed(hash, &[]);
        }
        // Make the row for `i = 0` the most counted so it is not the victim.
        t.note_completed([0u8; 32], &[]);
        let fresh = [0xFFu8; 32];
        t.note_completed(fresh, &[]);
        assert_eq!(t.rows.len(), MAX_ROWS);
        assert_eq!(t.counts(&fresh).built, 1);
        assert_eq!(t.counts(&[0u8; 32]).built, 2, "a well-counted design survives");
    }

    #[test]
    fn a_full_table_never_evicts_a_row_the_completion_is_writing() {
        let mut t = PlanBuilds::default();
        for i in 1..=MAX_ROWS as u32 {
            let mut hash = [0u8; 32];
            hash[..4].copy_from_slice(&i.to_be_bytes());
            t.note_completed(hash, &[]);
        }
        assert_eq!(t.rows.len(), MAX_ROWS);
        // The new design has the lowest hash of the weight-1 rows, and its new
        // ancestor must not push it straight out.
        let (design, ancestor) = ([0u8; 32], [0xFEu8; 32]);
        t.note_completed(design, &[ancestor]);
        assert_eq!(t.counts(&design), PlanCounts { built: 1, built_from: 0 });
        assert_eq!(t.counts(&ancestor), PlanCounts { built: 0, built_from: 1 });
        assert_eq!(t.rows.len(), MAX_ROWS, "two rows in, two least-counted others out");
    }
}

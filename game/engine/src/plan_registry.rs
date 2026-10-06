//! Spec 27 — Plan Registry for village procgen.
//!
//! Replaces `village_gen.rs::build_house`'s hardcoded shapes with
//! samples from a curated, licence-tagged registry. v1 ships the
//! framework with an empty engine-bundled set; content authoring
//! (Phase 5 — the ~200 KB of curated plan JSON) lands in a separate
//! pass. Today's village procgen falls back to the hardcoded shape
//! when the registry is empty, so this commit is gameplay-neutral
//! and just unblocks future content.
//!
//! Phase boundaries (see `docs/foundations/2026-05-20-village-procgen-integration.md`):
//! - Phase 2 (this commit) — `PlanRegistry` data structure.
//! - Phase 3 — `PlanCategory` enum + auto-inference.
//! - Phase 4 — `sample_plan_for_slot`.
//! - Phase 5 — DEFERRED: engine-bundled curated plan JSON files.
//! - Phase 6 — `village_gen::build_house` falls through to registry
//!   when present; hardcoded fallback retained.
//! - Phase 7 — Workshop placement (DEFERRED post-content).
//! - Phase 8 — Plaque placement during procgen (DEFERRED post-content).
//! - Phase 9 — Village Bell "Houses" tab (DEFERRED post-content).

use serde::{Deserialize, Serialize};

use crate::plan::{PlanData, PlanLicense};
use crate::villager::Profession;

/// Spec 27 Phase 5 — wrapper for an engine-bundled `.plan.json` file.
/// The optional `category` field lets authors override the inferred
/// classification (needed for cases like a Scribe workshop with no
/// distinguishing block, or a small house whose 80+ cells would
/// otherwise infer as LargeHouse).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundledPlan {
    pub plan: PlanData,
    #[serde(default)]
    pub category: Option<PlanCategory>,
}

/// Spec 27 Phase 3 — what kind of building this plan represents. The
/// village procgen sampler filters by category. v1 covers the six
/// shapes that the current village layout uses (or will use post-
/// Phase 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanCategory {
    /// Up to ~30 cells. Bed + lit interior. Fits an 8×8 slot.
    SmallHouse,
    /// 30+ cells. Multiple beds OR a second storey. Fits a 12×12 slot.
    LargeHouse,
    /// Profession-bound workstation building. Fits a 10×10 slot.
    Workshop(Profession),
    /// Central village structure. 16×16 slot.
    TownHall,
    /// Decorative — fountain, well, sign. No size constraint.
    Decoration,
    /// Storage-focused — chests inside, no bed. 8×8 slot.
    Storage,
}

/// Slot dimensions the sampler must fit into.
#[derive(Clone, Copy, Debug)]
pub struct SlotDims {
    pub width: u8,
    pub depth: u8,
}

/// Spec 27 Phase 3 — infer a category from a plan's contents. Authors
/// can override via an explicit category tag on the registry entry;
/// this is the fallback when none is set.
///
/// Order matters: a Forge (Furnace + Bed) classifies as Blacksmith
/// workshop, not SmallHouse — the workstation block wins.
pub fn infer_category(plan: &PlanData) -> PlanCategory {
    use crate::block;
    let has = |id: u16| plan.cells.iter().any(|c| c.block_id == id);

    // Workstation precedence — most specific wins.
    // Historical Pivot Sub 7 (2026-05-23) — T1.5 workstations checked
    // first so a plan with both a Mill + a Furnace classifies as the
    // more specific Miller workshop rather than the generic
    // Blacksmith.
    if has(block::MILL) {
        return PlanCategory::Workshop(Profession::Miller);
    }
    if has(block::OVEN) {
        return PlanCategory::Workshop(Profession::Baker);
    }
    if has(block::AGING_RACK) {
        return PlanCategory::Workshop(Profession::Brewer);
    }
    if has(block::FURNACE) || has(block::FURNACE_LIT) {
        return PlanCategory::Workshop(Profession::Blacksmith);
    }
    if has(block::CAMPFIRE) || has(block::CAMPFIRE_UNLIT) {
        return PlanCategory::Workshop(Profession::Cook);
    }
    if has(block::TILLED_SOIL) {
        return PlanCategory::Workshop(Profession::Farmer);
    }
    // Spec 26 — Drafting Table claims Builder. Checked before
    // CraftingTable because a building with both would more naturally
    // be a Builder workshop (drafting is the more specific activity).
    if has(block::DRAFTING_TABLE) {
        return PlanCategory::Workshop(Profession::Builder);
    }
    if has(block::CRAFTING_TABLE) {
        return PlanCategory::Workshop(Profession::Carpenter);
    }
    // Residential vs storage vs decoration.
    if has(block::BED) {
        if plan.cells.len() > 60 {
            PlanCategory::LargeHouse
        } else {
            PlanCategory::SmallHouse
        }
    } else {
        PlanCategory::Decoration
    }
}

/// True iff the plan's licence permits procgen use. CC-0 and CC-BY-SA
/// only — Spec 27 §8.6. The other current `PlanLicense` variants —
/// `AllRightsReserved` (no shared use) and `Ccbynd` (no derivatives,
/// and procgen sampling counts as derivation) — are filtered out at
/// registry-load time so `sample_plan_for_slot` never returns them.
pub fn license_eligible_for_procgen(lic: PlanLicense) -> bool {
    matches!(lic, PlanLicense::CC0 | PlanLicense::Ccbysa)
}

/// A registered entry in the plan registry. Pairs a plan with its
/// category (explicit OR inferred) so the sampler doesn't have to
/// re-classify on every call.
#[derive(Clone, Debug)]
pub struct RegistryEntry {
    pub plan: PlanData,
    pub category: PlanCategory,
}

/// Spec 27 Phase 2 — the registry itself. Loaded once per world
/// session from engine-bundled JSON + server-local additions.
#[derive(Clone, Debug, Default)]
pub struct PlanRegistry {
    entries: Vec<RegistryEntry>,
}

impl PlanRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from a pre-filtered list of (plan, explicit-category)
    /// pairs. Filters out licence-ineligible entries silently with a
    /// warning log line so callers can debug missing content. Inferred
    /// category is used when `category` is None.
    pub fn from_entries(entries: Vec<(PlanData, Option<PlanCategory>)>) -> Self {
        let mut out = Vec::new();
        for (plan, category) in entries {
            if !license_eligible_for_procgen(plan.license) {
                log::warn!(
                    "PlanRegistry: dropping '{}' — licence {:?} not eligible for procgen",
                    plan.name, plan.license,
                );
                continue;
            }
            let category = category.unwrap_or_else(|| infer_category(&plan));
            out.push(RegistryEntry { plan, category });
        }
        Self { entries: out }
    }

    /// Number of entries currently registered. Doc claims the village-gen
    /// fallback reads this to decide whether to call `sample_plan_for_slot`,
    /// but no such caller exists today — exercised only by the tests below
    /// and `test_integration/villages.rs`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Read-only access to every registered entry. Used by the Village
    /// Bell "Houses" tab (Phase 9) to map house plans to architects.
    pub fn entries(&self) -> &[RegistryEntry] {
        &self.entries
    }

    /// Look up an entry by plan name. Used by the Village Bell tab to
    /// resolve a house's `plan_name` back to its derivation chain.
    pub fn find_by_plan_name(&self, name: &str) -> Option<&RegistryEntry> {
        self.entries.iter().find(|e| e.plan.name == name)
    }

    /// #10 — add a plan at runtime (the schematic-import path). Category is
    /// inferred. A same-name plan is replaced so re-importing updates in place.
    pub fn add(&mut self, plan: PlanData) {
        let category = infer_category(&plan);
        self.entries.retain(|e| e.plan.name != plan.name);
        self.entries.push(RegistryEntry { plan, category });
    }

    /// The engine-bundled registry, parsed once per process. This is the ONLY
    /// plan source world generation reads (`village_gen`), so a world's
    /// generated villages are a pure function of seed + flags + engine build,
    /// never of runtime additions to `World::plan_registry` such as
    /// `/importschem` (Phase B0 worldgen purity). Its
    /// [`content_hash`](Self::content_hash) is folded into
    /// `world::worldgen_fingerprint`, so two builds whose bundled plans differ
    /// announce different fingerprints to each other.
    pub fn bundled() -> &'static PlanRegistry {
        static BUNDLED: std::sync::LazyLock<PlanRegistry> =
            std::sync::LazyLock::new(PlanRegistry::load_bundled);
        &BUNDLED
    }

    /// Stable SHA-256 of this registry as world generation sees it: every
    /// entry in registry order, as its full parsed `PlanData` (bincode — fixed
    /// little-endian, no maps) plus its category. Hashing parsed content, not
    /// the `include_str!` bytes, keeps it independent of line endings (a
    /// Windows checkout with autocrlf has different JSON bytes, same plans).
    pub fn content_hash(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"axenstax-plan-registry\0");
        h.update((self.entries.len() as u64).to_le_bytes());
        for e in &self.entries {
            let plan = bincode::serialize(&e.plan).expect("PlanData bincode");
            h.update((plan.len() as u64).to_le_bytes());
            h.update(&plan);
            // Debug name, not the serde variant index, so reordering the
            // `PlanCategory`/`Profession` enums doesn't change the hash.
            let category = format!("{:?}", e.category);
            h.update((category.len() as u64).to_le_bytes());
            h.update(category.as_bytes());
        }
        h.finalize().into()
    }

    /// Spec 27 Phase 5 — load every engine-bundled `.plan.json` file
    /// shipped under `game/engine/assets/registered_plans/`. JSON is
    /// `include_str!`'d at compile time so the binary stays self-
    /// contained (no runtime asset IO; works on WASM identically to
    /// native). Licence-ineligible entries are filtered the same as
    /// `from_entries`.
    pub fn load_bundled() -> Self {
        let raw = bundled_plan_sources();
        let mut entries: Vec<(PlanData, Option<PlanCategory>)> = Vec::with_capacity(raw.len());
        for (name, source) in raw {
            match serde_json::from_str::<BundledPlan>(source) {
                Ok(mut bp) => {
                    // Engine-bundled plans ship with an empty
                    // derivation_chain — fill in a synthetic self-
                    // link at load time so procgen Plaques carry
                    // attribution (otherwise the dialog renders as
                    // "(unsigned)" with no plan name link). The
                    // self-link mirrors what `commit_capture` builds
                    // for player captures.
                    if bp.plan.derivation_chain.is_empty() {
                        let hash = crate::plan::content_hash(&bp.plan);
                        bp.plan.derivation_chain.push(crate::plan::DerivationLink {
                            author_npub: bp.plan.author_npub.clone(),
                            plan_name: bp.plan.name.clone(),
                            license: bp.plan.license,
                            captured_at: 0,
                            plan_hash: hash,
                        });
                    }
                    entries.push((bp.plan, bp.category));
                }
                Err(e) => {
                    log::warn!("PlanRegistry: failed to parse bundled '{name}': {e}");
                }
            }
        }
        Self::from_entries(entries)
    }

    /// Spec 27 Phase 4 — pick a plan for a slot from the candidates
    /// matching `category` AND fitting `slot`. Returns None when no
    /// candidate qualifies (caller falls back to the hardcoded shape).
    /// `seed` drives the random pick so deterministic-seed worlds
    /// produce the same village layouts.
    pub fn sample_plan_for_slot(
        &self,
        category: PlanCategory,
        slot: SlotDims,
        seed: u64,
    ) -> Option<&PlanData> {
        let candidates: Vec<&RegistryEntry> = self
            .entries
            .iter()
            .filter(|e| categories_match(e.category, category))
            .filter(|e| e.plan.width <= slot.width && e.plan.depth <= slot.depth)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        // Deterministic pick — wyhash-style mixing on the seed.
        let idx = (seed.wrapping_mul(0x9E3779B97F4A7C15) as usize) % candidates.len();
        Some(&candidates[idx].plan)
    }
}

/// Spec 27 Phase 5 — engine-bundled `.plan.json` content, embedded at
/// compile time via `include_str!`. Adding a new bundled plan is a
/// two-step append: drop the file in `assets/registered_plans/` and
/// add a `(filename, include_str!("..."))` row here. Keeping the list
/// explicit (rather than a build-script-driven scan) keeps the bundle
/// surface auditable and the binary self-contained on WASM.
fn bundled_plan_sources() -> &'static [(&'static str, &'static str)] {
    &[
        ("small_house_oak_cottage", include_str!("../assets/registered_plans/small_house_oak_cottage.plan.json")),
        ("small_house_cobble_hut", include_str!("../assets/registered_plans/small_house_cobble_hut.plan.json")),
        ("small_house_cosy_cabin", include_str!("../assets/registered_plans/small_house_cosy_cabin.plan.json")),
        ("small_house_stone_hut", include_str!("../assets/registered_plans/small_house_stone_hut.plan.json")),
        ("small_house_compact_lodge", include_str!("../assets/registered_plans/small_house_compact_lodge.plan.json")),
        ("small_house_tent_hut", include_str!("../assets/registered_plans/small_house_tent_hut.plan.json")),
        ("large_house_oak_manor", include_str!("../assets/registered_plans/large_house_oak_manor.plan.json")),
        ("large_house_stone_hall", include_str!("../assets/registered_plans/large_house_stone_hall.plan.json")),
        ("large_house_multi_room_cabin", include_str!("../assets/registered_plans/large_house_multi_room_cabin.plan.json")),
        ("large_house_townhouse", include_str!("../assets/registered_plans/large_house_townhouse.plan.json")),
        ("workshop_farmer_shed", include_str!("../assets/registered_plans/workshop_farmer_shed.plan.json")),
        ("workshop_cook_house", include_str!("../assets/registered_plans/workshop_cook_house.plan.json")),
        ("workshop_carpenter", include_str!("../assets/registered_plans/workshop_carpenter.plan.json")),
        ("workshop_blacksmith_forge", include_str!("../assets/registered_plans/workshop_blacksmith_forge.plan.json")),
        ("workshop_librarian_reading_room", include_str!("../assets/registered_plans/workshop_librarian_reading_room.plan.json")),
        ("town_hall", include_str!("../assets/registered_plans/town_hall.plan.json")),
        ("decoration_fountain", include_str!("../assets/registered_plans/decoration_fountain.plan.json")),
        ("decoration_well", include_str!("../assets/registered_plans/decoration_well.plan.json")),
    ]
}

/// Pure helper: does a registry entry's category satisfy a slot's
/// requested category? Exact match for most variants; `Workshop`
/// matches only if the profession matches too.
fn categories_match(entry: PlanCategory, requested: PlanCategory) -> bool {
    match (entry, requested) {
        (PlanCategory::Workshop(a), PlanCategory::Workshop(b)) => a == b,
        (a, b) => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::plan::{CapturedCell, PlanData};

    fn plan(name: &str, license: PlanLicense, cells: Vec<CapturedCell>, w: u8, d: u8) -> PlanData {
        PlanData {
            version: 1,
            name: name.to_string(),
            author_npub: String::new(),
            license,
            derivation_chain: Vec::new(),
            is_master: true,
            width: w, depth: d, height: 1,
            cells,
            authored_in: "survival".to_string(),
            develop_state: crate::plan::DevelopState::Developed,
            kind: crate::plan::PlanKind::Building,
        }
    }

    #[test]
    fn bundled_is_the_parsed_bundle_and_hashes_stably() {
        let bundled = PlanRegistry::bundled();
        assert!(!bundled.is_empty(), "the engine bundle must parse");
        assert_eq!(bundled.content_hash(), PlanRegistry::load_bundled().content_hash());
    }

    #[test]
    fn content_hash_changes_with_any_plan_content_or_category() {
        let cell = CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::STONE };
        let base = || plan("p", PlanLicense::CC0, vec![cell], 3, 3);
        let reg = |p: PlanData, c| PlanRegistry::from_entries(vec![(p, Some(c))]);
        let a = reg(base(), PlanCategory::SmallHouse).content_hash();
        assert_eq!(a, reg(base(), PlanCategory::SmallHouse).content_hash());

        let mut moved = base();
        moved.cells[0].rx = 1;
        assert_ne!(a, reg(moved, PlanCategory::SmallHouse).content_hash(), "cell change");
        assert_ne!(a, reg(base(), PlanCategory::LargeHouse).content_hash(), "category change");
        assert_ne!(a, PlanRegistry::new().content_hash(), "entry removed");
    }

    #[test]
    fn empty_registry_returns_none_for_any_sample() {
        let r = PlanRegistry::new();
        let slot = SlotDims { width: 8, depth: 8 };
        assert!(r.sample_plan_for_slot(PlanCategory::SmallHouse, slot, 1).is_none());
    }

    #[test]
    fn license_filter_rejects_cc_by_nd_at_construction() {
        let bad = plan("nd-plan", PlanLicense::Ccbynd, vec![], 3, 3);
        let r = PlanRegistry::from_entries(vec![(bad, Some(PlanCategory::SmallHouse))]);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn license_filter_rejects_all_rights_reserved() {
        // Post-merge review: AllRightsReserved is a PlanLicense
        // variant; the filter must drop it like CC-BY-ND.
        let bad = plan("locked", PlanLicense::AllRightsReserved, vec![], 3, 3);
        let r = PlanRegistry::from_entries(vec![(bad, Some(PlanCategory::SmallHouse))]);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn license_eligible_helper_matches_only_cc0_and_cc_by_sa() {
        // Direct check on the predicate — locks the four-variant
        // behaviour even when the registry isn't involved.
        assert!(license_eligible_for_procgen(PlanLicense::CC0));
        assert!(license_eligible_for_procgen(PlanLicense::Ccbysa));
        assert!(!license_eligible_for_procgen(PlanLicense::Ccbynd));
        assert!(!license_eligible_for_procgen(PlanLicense::AllRightsReserved));
    }

    #[test]
    fn infer_category_recognises_builder_via_drafting_table() {
        // Post-merge review #5 — DRAFTING_TABLE (Spec 26) was missing
        // from `infer_category`. A captured building containing a
        // Drafting Table now classifies as Workshop(Builder).
        let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::DRAFTING_TABLE }];
        let p = plan("drafting-room", PlanLicense::Ccbysa, cells, 3, 3);
        assert!(matches!(
            infer_category(&p),
            PlanCategory::Workshop(Profession::Builder),
        ));
    }

    #[test]
    fn license_filter_accepts_cc0_and_cc_by_sa() {
        let cc0 = plan("a", PlanLicense::CC0, vec![], 3, 3);
        let sa = plan("b", PlanLicense::Ccbysa, vec![], 3, 3);
        let r = PlanRegistry::from_entries(vec![
            (cc0, Some(PlanCategory::SmallHouse)),
            (sa, Some(PlanCategory::SmallHouse)),
        ]);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn infer_category_recognises_blacksmith_via_furnace() {
        let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::FURNACE }];
        let p = plan("forge", PlanLicense::Ccbysa, cells, 3, 3);
        assert!(matches!(infer_category(&p), PlanCategory::Workshop(Profession::Blacksmith)));
    }

    #[test]
    fn infer_category_recognises_miller_via_mill() {
        // Historical Pivot Sub 7 — Mill block claims a Miller workshop.
        let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::MILL }];
        let p = plan("mill-room", PlanLicense::Ccbysa, cells, 3, 3);
        assert!(matches!(
            infer_category(&p),
            PlanCategory::Workshop(Profession::Miller),
        ));
    }

    #[test]
    fn infer_category_recognises_baker_via_oven() {
        let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::OVEN }];
        let p = plan("bakery", PlanLicense::Ccbysa, cells, 3, 3);
        assert!(matches!(
            infer_category(&p),
            PlanCategory::Workshop(Profession::Baker),
        ));
    }

    #[test]
    fn infer_category_recognises_brewer_via_aging_rack() {
        let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::AGING_RACK }];
        let p = plan("brewery", PlanLicense::Ccbysa, cells, 3, 3);
        assert!(matches!(
            infer_category(&p),
            PlanCategory::Workshop(Profession::Brewer),
        ));
    }

    #[test]
    fn infer_category_t1_5_workstations_win_over_furnace() {
        // A plan containing BOTH a Mill and a Furnace classifies as
        // Miller — most specific workstation wins. Guards the
        // ordering of the arms in `infer_category` against accidental
        // reshuffle.
        let cells = vec![
            CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::MILL },
            CapturedCell { rx: 1, ry: 0, rz: 0, block_id: block::FURNACE },
        ];
        let p = plan("mill-and-forge", PlanLicense::Ccbysa, cells, 3, 3);
        assert!(matches!(
            infer_category(&p),
            PlanCategory::Workshop(Profession::Miller),
        ));
    }

    #[test]
    fn infer_category_recognises_cook_via_campfire() {
        let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::CAMPFIRE }];
        let p = plan("kitchen", PlanLicense::CC0, cells, 3, 3);
        assert!(matches!(infer_category(&p), PlanCategory::Workshop(Profession::Cook)));
    }

    #[test]
    fn infer_category_recognises_small_house_via_bed() {
        let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::BED }];
        let p = plan("cottage", PlanLicense::Ccbysa, cells, 3, 3);
        assert!(matches!(infer_category(&p), PlanCategory::SmallHouse));
    }

    #[test]
    fn infer_category_large_house_when_bed_plus_many_cells() {
        let mut cells = Vec::new();
        for i in 0..65 {
            cells.push(CapturedCell { rx: (i % 8) as u8, ry: 0, rz: (i / 8) as u8, block_id: block::OAK_PLANKS });
        }
        cells.push(CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::BED });
        let p = plan("mansion", PlanLicense::Ccbysa, cells, 8, 9);
        assert!(matches!(infer_category(&p), PlanCategory::LargeHouse));
    }

    #[test]
    fn infer_category_decoration_when_no_bed_no_workstation() {
        let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::OAK_PLANKS }];
        let p = plan("fountain", PlanLicense::Ccbysa, cells, 1, 1);
        assert!(matches!(infer_category(&p), PlanCategory::Decoration));
    }

    #[test]
    fn sample_filters_by_size() {
        let small = plan("small", PlanLicense::Ccbysa, vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::BED }], 3, 3);
        let too_big = plan("too-big", PlanLicense::Ccbysa, vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::BED }], 12, 12);
        let r = PlanRegistry::from_entries(vec![
            (small.clone(), Some(PlanCategory::SmallHouse)),
            (too_big.clone(), Some(PlanCategory::SmallHouse)),
        ]);
        let slot = SlotDims { width: 8, depth: 8 };
        let picked = r.sample_plan_for_slot(PlanCategory::SmallHouse, slot, 1);
        assert_eq!(picked.map(|p| p.name.as_str()), Some("small"));
    }

    #[test]
    fn sample_filters_by_category_workshop_profession() {
        let cook_kitchen = plan("kitchen", PlanLicense::CC0, vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::CAMPFIRE }], 3, 3);
        let forge = plan("forge", PlanLicense::CC0, vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::FURNACE }], 3, 3);
        let r = PlanRegistry::from_entries(vec![
            (cook_kitchen, None),
            (forge, None),
        ]);
        let slot = SlotDims { width: 10, depth: 10 };
        let picked = r.sample_plan_for_slot(PlanCategory::Workshop(Profession::Cook), slot, 1);
        assert_eq!(picked.map(|p| p.name.as_str()), Some("kitchen"));
        let picked2 = r.sample_plan_for_slot(PlanCategory::Workshop(Profession::Blacksmith), slot, 1);
        assert_eq!(picked2.map(|p| p.name.as_str()), Some("forge"));
        // Carpenter has no entries — None.
        assert!(r.sample_plan_for_slot(PlanCategory::Workshop(Profession::Carpenter), slot, 1).is_none());
    }

    // ─── Spec 27 Phase 5 — bundled-plan loader tests ───────────────

    #[test]
    fn load_bundled_parses_every_shipped_plan() {
        let r = PlanRegistry::load_bundled();
        let raw = bundled_plan_sources();
        assert!(!raw.is_empty(), "no bundled plans declared");
        // Every declared source must parse + survive licence filter.
        // The shipped set is authored CC-BY-SA / CC-0, so none are
        // expected to drop.
        assert_eq!(
            r.len(),
            raw.len(),
            "expected {} entries; got {} — a bundled plan failed to parse OR was filtered",
            raw.len(),
            r.len(),
        );
    }

    #[test]
    fn bundled_registry_has_a_small_house_that_fits_an_8x8_slot() {
        let r = PlanRegistry::load_bundled();
        let slot = SlotDims { width: 8, depth: 8 };
        assert!(
            r.sample_plan_for_slot(PlanCategory::SmallHouse, slot, 1).is_some(),
            "bundled registry must include at least one SmallHouse for the village-gen path",
        );
    }

    #[test]
    fn bundled_registry_has_one_workshop_per_alpha_profession() {
        let r = PlanRegistry::load_bundled();
        let slot = SlotDims { width: 10, depth: 10 };
        for prof in [
            Profession::Farmer,
            Profession::Cook,
            Profession::Carpenter,
            Profession::Blacksmith,
            Profession::Scribe,
        ] {
            assert!(
                r.sample_plan_for_slot(PlanCategory::Workshop(prof), slot, 1).is_some(),
                "expected at least one Workshop({prof:?}) in the bundled registry",
            );
        }
    }

    #[test]
    fn bundled_registry_has_a_town_hall_that_fits_16x16() {
        let r = PlanRegistry::load_bundled();
        let slot = SlotDims { width: 16, depth: 16 };
        assert!(
            r.sample_plan_for_slot(PlanCategory::TownHall, slot, 1).is_some(),
        );
    }

    #[test]
    fn bundled_registry_has_at_least_one_large_house_under_12x12() {
        let r = PlanRegistry::load_bundled();
        let slot = SlotDims { width: 12, depth: 12 };
        assert!(
            r.sample_plan_for_slot(PlanCategory::LargeHouse, slot, 1).is_some(),
        );
    }

    #[test]
    fn bundled_plan_with_only_bed_under_60_cells_loads_as_small_house() {
        // `compact_lodge` is the lone SmallHouse with no explicit
        // category tag — relies on `infer_category` BED < 60 path. If
        // anyone bumps its cell count past 60 without adding an
        // explicit `category: SmallHouse` tag, this test catches it.
        let r = PlanRegistry::load_bundled();
        let found = r.entries().iter().find(|e| e.plan.name == "Compact Lodge")
            .expect("Compact Lodge missing from bundle");
        assert!(matches!(found.category, PlanCategory::SmallHouse));
    }

    #[test]
    fn sample_is_deterministic_per_seed() {
        let entries = (0..5)
            .map(|i| (plan(&format!("h{i}"), PlanLicense::Ccbysa, vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::BED }], 3, 3), Some(PlanCategory::SmallHouse)))
            .collect();
        let r = PlanRegistry::from_entries(entries);
        let slot = SlotDims { width: 8, depth: 8 };
        let pick_a = r.sample_plan_for_slot(PlanCategory::SmallHouse, slot, 42);
        let pick_b = r.sample_plan_for_slot(PlanCategory::SmallHouse, slot, 42);
        assert_eq!(
            pick_a.map(|p| p.name.as_str()),
            pick_b.map(|p| p.name.as_str()),
        );
    }
}

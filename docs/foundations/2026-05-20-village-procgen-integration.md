# Village Procgen Integration — Foundation E of Build Schematics

**Status:** **DELIVERED 2026-05-22** on `feat/spec-27-plan-content` (PR #78). **Phases 2-4 shipped 2026-05-20** (registry framework). **Phases 5-9 shipped 2026-05-22**: 18 engine-bundled curated `.plan.json` files (~272 KB) covering 6 SmallHouses + 4 LargeHouses + 5 profession workshops + 1 TownHall + 2 Decorations, all CC-BY-SA or CC-0. `PlanRegistry::load_bundled()` parses every file at startup via `include_str!` (binary stays self-contained on WASM). `village_gen::build_house` consults the registry first and falls back to the legacy hardcoded shape when empty (gameplay-neutral integration). New `build_workshop` allocates one workshop per claimed alpha profession on the village's outer ring (radius 18-22). Procgen Plaques carry the plan's full derivation chain (auto-synthesised at load time for bundled plans) + are tagged in `World::procgen_plaque_sources` so the plaque dialog surfaces "Sampled by village procgen". Village Bell right-click opens the "Houses" tab listing every nearby plaque with a per-architect Tip button + a "Tip all architects" pool that fans each leg through `economy::apply_sats_payout(_, PayoutKind::PlaqueTip, _, _)` so per-server tax + Charter + Reserve gates apply per architect. +15 tests across `plan_registry`, `village_bell_ui`, `test_integration::villages`. Phase 11 = playtest gate.
**Branch:** `feat/village-procgen-integration` off `main` (after Specs 19 + 24 merge).
**Trigger:** Foundation **E** of the Build Schematics economy (`docs/vision/build-schematics-long-run.md`). Implements Phase 6 (village procgen from the Plan Registry). Replaces `village_gen.rs::build_house`'s hardcoded shapes with samples from a curated, license-tagged Plan Registry — closing the community-quality flywheel for village content.

---

## TL;DR

`village_gen.rs::build_house` currently hardcodes a single boxy 5×5 house shape. This spec replaces that with a **Plan Registry**: an engine-bundled curated set (~12-20 plans for alpha) + a server-local additions slot (admins drop Plans into a `plans/` directory on the server). When village procgen places a house slot, it samples from the registry filtered by (a) category match (small house for a 8×8 slot, workshop for a Carpenter slot) and (b) license eligibility (CC-0 + CC-BY-SA only — Licence-encumbered plans don't qualify for procgen use). The Village Bell info dialog gets a "Houses in this village" tab that shows credited architects per building + a multi-architect "Tip them all" pool button.

### New shape in one paragraph

`PlanRegistry` is a new lazy-static struct that loads engine-bundled `.plan.json` files at startup + scans a server-local `plans/` directory for additions. `PlanCategory` enum tags each registered plan: `SmallHouse` / `LargeHouse` / `Workshop(Profession)` / `TownHall` / `Decoration` / `Storage`. Categorisation can be explicit (author tags it) OR auto-inferred from contents (≥1 bed → SmallHouse; ≥1 workstation block → Workshop). License-gate at registration time: only CC-0 + CC-BY-SA pass. Village procgen samples per slot — fixed slot sizes for v1 (8×8 for houses, 10×10 for workshops, 16×16 for TownHall). When a plan is sampled and built, its Plaque drops with the architect's full derivation chain — village houses become attributable for the first time.

---

## Phases summary

| # | Phase | Files | Approx LOC |
|---|-------|-------|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-20-village-procgen-integration.md` | – |
| 2 | `PlanRegistry` data structure + lazy-static initialisation. Loads engine-bundled plans from `game/engine/assets/registered_plans/*.plan.json` at first call; scans `${world_dir}/plans/` for server-local additions on world-load. License-gates entries — any plan with license `CCBYND` or `CCBYNCSA` is rejected with a warning log line. | new `plan_registry.rs`, `world.rs` (load hook) | ~150 |
| 3 | `PlanCategory` enum + categorisation helpers. Manual tag via a `category: Option<PlanCategory>` field on PlanData (optional, defaults None). Auto-inference function `infer_category(&PlanData) -> PlanCategory` based on block contents (e.g., contains BED → SmallHouse, contains FURNACE → Workshop(Blacksmith), etc). | `plan.rs`, `plan_registry.rs` | ~120 |
| 4 | `sample_plan_for_slot(registry, category, rng) -> Option<&PlanData>`. Filters the registry by category match + license eligibility + slot-size compatibility (footprint fits within slot dimensions). Returns a random sample biased toward "most-frequent" or "weighted-by-author-rep" — v1 just uniform-random over the filtered set. | `plan_registry.rs` | ~80 |
| 5 | Engine-bundled curated plan files. ~12-20 plans authored as JSON files in `game/engine/assets/registered_plans/`. Includes: 5-6 SmallHouse variants, 3-4 LargeHouse, 1 per profession Workshop (Farmer / Cook / Carpenter / Blacksmith / Librarian = 5 plans), 1 TownHall, 1-2 Decoration. All authored CC-BY-SA by "Axe'n'Stax" pubkey. | `game/engine/assets/registered_plans/*.plan.json` | ~0 LOC (data files; ~200 KB total) |
| 6 | `village_gen.rs::build_house` replacement. The hardcoded 5×5 shape is gone. Instead: sample a SmallHouse plan from the registry for each house slot; if no plan fits (registry empty for that category), fall back to a single hardcoded 8×8 house as the emergency-only shape. | `village_gen.rs` | ~120 |
| 7 | Workshop placement. Villages with a Farmer claim need a Farmer-Workshop tile; same for the other professions. The village-gen sampling pass picks a Workshop plan per claimed-profession slot. | `village_gen.rs` | ~80 |
| 8 | Plaque placement during procgen. When a plan is built via procgen (not player commission), the Plaque carries the architect's derivation chain + an additional "Sampled by village procgen" line. This is the first real attribution-on-procgen-content the engine has. | `village_gen.rs`, `plan.rs` | ~50 |
| 9 | Village Bell info dialog. Spec 19's Village Bell already exists; this spec adds a "Houses in this village" tab to the bell's right-click dialog. Each house in the village is listed with its plan name + architect(s) credit + a multi-architect "Tip all" pool button (sums per-architect tip into one transaction that fans out via the helper). | `villager_ui.rs` (or new `village_bell_ui.rs`), `economy.rs` | ~180 |
| 10 | Tests — registry load (engine-bundled + server-local), category inference, sampling filter (license + size), procgen integration (a village spawn produces houses whose Plaque-chains match registry sources). | `plan_registry.rs::tests`, `village_gen.rs::tests`, `test_integration/village_procgen.rs` | ~150 |
| 11 | Axolittle playtest — explore a village; right-click houses + workshops to read Plaques; verify architect credit; tip-all-pool from Village Bell; confirm hardcoded fallback still works (drop registry by renaming the assets dir + spawn a village in a debug seed). | – | playtest gate |

**Total**: ~930 LOC across 10 build phases (+ ~200 KB JSON data). The README's ~600 estimate was for code only — the JSON data isn't counted in that bucket. Phase 11 is the playtest gate.

---

## Why this lives here

- **Closes the village-quality flywheel.** Hardcoded village shapes have been a placeholder since Spec 19 shipped. This replaces them with community content that grows over time.
- **First procgen-attribution surface.** Player-built Plaques have been creditable since Spec 24 Phase 12. Procgen-built content has been anonymous. This makes village content honest: every house has an author.
- **Cross-server portability for shared culture.** A server's `plans/` additions distribute server-by-server; eventually a Nostr-backed Plan Registry (v2) lets a plan published globally appear in every server's procgen pass automatically.
- **Cross-game lift.** Categorisation + sampling-by-slot + license-gate-on-procgen all lift cross-game. Any game with procgen settlements can adopt the registry shape.

---

## Creative vs Survival

Per-server policy can require survival-authored plans only (per `docs/foundations/2026-05-19-build-schematics-core.md` "Where policy layers can hook in later"). This spec ships the **registry-level filter hook** for that policy but doesn't enforce it by default — server admins toggle it via a per-server config flag.

---

## Context pointers

### Existing code surfaces this touches

- `village_gen.rs` — `build_house` is gutted + replaced with the sampling path; new Workshop placement.
- `plan.rs` — `PlanData` gains `category: Option<PlanCategory>` field (defaults None — back-compat with Spec 24 PlanData).
- `villager_ui.rs` (or new file) — Village Bell info dialog gains "Houses" tab.
- `economy.rs` — multi-architect tip-pool helper (fans out one logical tip across N architects via `apply_server_tax_and_payout` per leg).
- `world.rs` — load-hook for server-local plans dir.

### New modules

- `plan_registry.rs` (~300 LOC) — registry data structure, JSON load + parse, category inference, sampling.
- `game/engine/assets/registered_plans/` — ~12-20 JSON files. Each contains the `PlanData` + a `category` field.

### Related specs

- `docs/foundations/2026-05-18-villages-and-villagers.md` (Spec 19) — provides villages + bells + dialogue framework.
- `docs/foundations/2026-05-19-build-schematics-core.md` (Spec 24) — provides Plan item + Plaque + animated build.
- `docs/foundations/2026-05-20-plan-trade-plaque-tipping.md` (Spec 25) — provides tipping flow + sats helper. Multi-architect tip-pool extends it.
- `docs/vision/build-schematics-long-run.md` §8 (Phase 6 — Village procgen) — design contract.

### Memory pointers

- uk english naming — "registry" (lowercase noun); use UK English in architect-facing tooltips.
- shared infra strategy — registry + categorisation + sampling lifts cross-game.
- bitcoin parent controlled — tip-all-pool is gated like any other sats flow.

### What does NOT exist yet (deferred to v2)

- **Nostr-decentralised registry.** v1 = engine-bundled + server-local file scan. v2 = subscribe to a Nostr relay for plan publications.
- **Dynamic-layout village procgen.** v1 uses fixed slot sizes (8×8 for SmallHouse, 10×10 for Workshop, 16×16 for TownHall). v2 may pack variable-sized plans more cleverly.
- **Plan quality scoring.** No "good plans get sampled more" weighting yet — uniform random over filtered set. v2 can add weighting by Plaque tip count or other social signal.
- **Per-server plan upload UI.** Server admins drop JSON files into the dir manually. v2 may add an admin-side upload dialog.

---

## Phase 2 — Registry shape

```rust
pub struct PlanRegistry {
    entries: Vec<(PlanData, PlanCategory)>,
}

impl PlanRegistry {
    pub fn load() -> Self {
        let mut entries = Vec::new();
        // 1. Engine-bundled
        for path in BUNDLED_PLAN_PATHS {
            if let Some((plan, cat)) = load_plan_file(path) {
                if license_eligible(&plan.license) {
                    entries.push((plan, cat));
                }
            }
        }
        // 2. Server-local (loaded by World::load() with a world-dir path)
        // ...
        Self { entries }
    }

    pub fn sample(&self, category: PlanCategory, slot: SlotDims, rng: &mut impl Rng) -> Option<&PlanData> {
        let candidates: Vec<&PlanData> = self.entries.iter()
            .filter(|(_, c)| *c == category)
            .filter(|(p, _)| p.width <= slot.w && p.depth <= slot.d)
            .map(|(p, _)| p)
            .collect();
        candidates.choose(rng).copied()
    }
}

fn license_eligible(lic: &PlanLicense) -> bool {
    matches!(lic, PlanLicense::CC0 | PlanLicense::CCBYSA)
}
```

---

## Phase 3 — Category inference

```rust
pub enum PlanCategory {
    SmallHouse,
    LargeHouse,
    Workshop(Profession),
    TownHall,
    Decoration,
    Storage,
}

pub fn infer_category(plan: &PlanData) -> PlanCategory {
    let has = |b: BlockId| plan.cells.iter().any(|c| c.block_id == b);
    if has(block::FURNACE) || has(block::FURNACE_LIT) {
        return PlanCategory::Workshop(Profession::Blacksmith);
    }
    if has(block::CAMPFIRE) || has(block::CAMPFIRE_UNLIT) {
        return PlanCategory::Workshop(Profession::Cook);
    }
    if has(block::TILLED_SOIL) {
        return PlanCategory::Workshop(Profession::Farmer);
    }
    if has(block::CRAFTING_TABLE) {
        return PlanCategory::Workshop(Profession::Carpenter);
    }
    if has(block::BED) {
        let cell_count = plan.cells.len();
        if cell_count > 60 { PlanCategory::LargeHouse } else { PlanCategory::SmallHouse }
    } else {
        PlanCategory::Decoration
    }
}
```

Authors can override via `plan.category` field; absent that, this inference applies.

---

## Phase 9 — Village Bell "Houses" tab

The Village Bell already opens an info dialog on right-click (Spec 19 §10). This spec adds a "Houses" tab next to the existing village-info content. Each house in the village is listed with:

- Plan name (e.g., "Cosy Farmer's Cottage")
- Footprint preview (ASCII top-down reuse)
- Architects credited (from the Plaque's derivation chain)
- Per-architect tip buttons (reuse Spec 25 Phase 7 surface)
- **"Tip all" pool button** — sums small tips across every architect in the village. Subject to Charter + per-server policy gates. Each leg of the fan-out goes through `apply_server_tax_and_payout` so the tax flow stays canonical.

---

## Phase 10 — Test plan

```rust
#[test]
fn registry_rejects_cc_by_nd_plans() {
    let plan = PlanData { license: PlanLicense::CCBYND, ..test_plan() };
    let registry = PlanRegistry::from_entries(vec![(plan, PlanCategory::SmallHouse)]);
    assert_eq!(registry.entries.len(), 0);
}

#[test]
fn sample_filters_by_size_and_category() {
    let small_house = PlanData { width: 5, depth: 5, ..test_plan() };
    let workshop = PlanData { width: 10, depth: 10, ..test_plan() };
    let registry = PlanRegistry::from_entries(vec![
        (small_house, PlanCategory::SmallHouse),
        (workshop, PlanCategory::Workshop(Profession::Cook)),
    ]);
    let slot = SlotDims { w: 8, d: 8 };
    assert!(registry.sample(PlanCategory::SmallHouse, slot, &mut rng()).is_some());
    assert!(registry.sample(PlanCategory::Workshop(Profession::Cook), slot, &mut rng()).is_none());
}

#[test]
fn category_inference_distinguishes_house_vs_workshop() {
    let house = PlanData { cells: vec![cell(BED), cell(OAK_PLANKS)], ..test_plan() };
    assert!(matches!(infer_category(&house), PlanCategory::SmallHouse));
    let cookshop = PlanData { cells: vec![cell(CAMPFIRE), ..test_plan().cells], ..test_plan() };
    assert!(matches!(infer_category(&cookshop), PlanCategory::Workshop(Profession::Cook)));
}
```

Plus integration test: spawn a village in a deterministic seed; verify the houses' Plaques carry registry-source architects + the Sampled-by-procgen line.

---

## Phase 11 — Axolittle playtest

- Explore a village. Right-click each house's Plaque + read the credit.
- Right-click the Village Bell → "Houses" tab. Confirm credits match the Plaques.
- Tip-all-pool → verify the fan-out routes through Charter + server policy + tax helper.
- Rename `assets/registered_plans/` to a different name → spawn a fresh village → confirm the hardcoded fallback shape still works (no crashes; just less variety).

---

## Acceptance

- `./check.sh` ALL GREEN.
- All Phase 2-9 tests pass.
- Integration test: a procgen village's houses match registry sources by category + license.
- Manual: tip-all-pool works on a multi-architect village.

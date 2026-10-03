# Historical Pivot — Sub-Foundation 7: Miller / Baker / Brewer Professions

**Status:** DELIVERED 2026-05-23 on `feat/historical-pivot-sub7-miller-baker-brewer`. Phase 9 (Axolittle playtest gate) is the remaining open task. **Prerequisite:** none beyond HP-0 (Naming Pass — provides `Profession::Scribe` rename which this sub matches in style). **Independent** of HP-2/3/4/5/6 — touches `villager.rs` + `quest.rs` + `plan_registry.rs` + `village_gen.rs` only.
**Branch:** `feat/historical-pivot-sub7-miller-baker-brewer` off `main`.
**Trigger:** Sub-foundation **7** of the [Historical Pivot Long-Run](../vision/historical-pivot-long-run.md). Adds vocational depth to villages by giving the existing Mill / Oven / Aging Rack workstations dedicated professions.

---

## TL;DR

Three new `Profession` variants (`Miller`, `Baker`, `Brewer`) join the existing six. Each claims a workstation block — Mill / Oven / Aging Rack respectively — using the established profession-claim pattern. Each gets a display label, a 3-4-line gossip pool, and a quest pool with Fetch + Make entries matching their trade. Smallest of the historical-pivot subs (~260 LOC).

Parallelisable with HP-2/3/4/5 since it only touches `villager.rs` + `quest.rs` + a few one-line arms in `plan_registry.rs`.

---

## Why this lives here

- **Closes a long-standing gap.** Mill / Oven / Aging Rack ship as workstation blocks (Spec T1.5), but no villager profession claims them. Villages with these blocks have no vocational identity for them; the building reads as decoration rather than trade.
- **Matches the existing per-profession pattern.** Six professions are already wired; this sub follows the same shape three more times — display label, workstation-claim arm, gossip pool, quest pool, procgen-pool entry.
- **Engine-generic.** The profession + workstation-claim pattern lifts cross-game. Sister games can add their own profession variants by following the same template.
- **Independent of every other historical-pivot sub.** Touches a small, well-bounded surface (villager + quest); doesn't interact with the brigand/animal/wave-composition work in HP-2/3/4/5.

---

## What this PR ships

### 1. `Profession::{Miller, Baker, Brewer}` enum variants

In `game/engine/src/villager.rs`, append three variants to the `Profession` enum:

```rust
pub enum Profession {
    None,
    Farmer,
    Blacksmith,
    Cook,
    Scribe,        // renamed from Librarian in HP-0
    Carpenter,
    Builder,
    // HP-7 (2026-05-23) — workstation-bound trades for the T1.5 economy.
    Miller,
    Baker,
    Brewer,
}
```

Bincode positional — variants append at the end, never in the middle. Saves stay compatible.

### 2. Display labels

In the `Profession::display_name` (or equivalent) match arm:

```rust
Profession::Miller => "Miller",
Profession::Baker => "Baker",
Profession::Brewer => "Brewer",
```

### 3. Workstation-claim arms

In the `from_workstation_block` (or equivalent — the function that maps a block id to a profession) match arm:

```rust
crate::block::MILL => Some(Profession::Miller),
crate::block::OVEN => Some(Profession::Baker),
crate::block::AGING_RACK => Some(Profession::Brewer),
```

Pattern matches the existing arms — `FURNACE → Blacksmith`, `CAMPFIRE → Cook`, `TILLED_SOIL → Farmer`, `CRAFTING_TABLE → Carpenter`, `DRAFTING_TABLE → Builder`. Scribe currently has no workstation per HP-0 (it inherited Librarian's lack of one).

### 4. Gossip pools (3-4 flavour lines each)

In the gossip-line match arm (around `villager.rs:274` per the HP-0 audit), add:

```rust
Profession::Miller => &[
    "Two stones grind better than one.",
    "Wheat in, flour out — same as it's always been.",
    "The wind never delivers on time.",
    "A heavy stone is a slow stone, but a fair one.",
],
Profession::Baker => &[
    "Dough's done when it springs back.",
    "Cold oven, cold customers.",
    "Yeast asks for patience.",
    "A burnt loaf is a lesson, not a loss.",
],
Profession::Brewer => &[
    "Time makes the drink, not the recipe.",
    "Honey first, water second, patience third.",
    "An empty cask is a sad cask.",
    "Don't drink the first batch — that's for the cooper.",
],
```

### 5. Quest pools

In `quest.rs`, add three new `*_POOL` constants matching the existing `FARMER_POOL` / `BLACKSMITH_POOL` shape:

```rust
const MILLER_POOL: &[QuestFlavour] = &[
    QuestFlavour::Fetch { item: MaterialId::Wheat, count: 8 },
    QuestFlavour::Fetch { item: MaterialId::Wheat, count: 16 },
    QuestFlavour::Make  { item: MaterialId::Flour, count: 4 },
    QuestFlavour::Make  { item: MaterialId::Flour, count: 8 },
];

const BAKER_POOL: &[QuestFlavour] = &[
    QuestFlavour::Fetch { item: MaterialId::Flour, count: 4 },
    QuestFlavour::Make  { item: MaterialId::Bread, count: 3 },
    QuestFlavour::Make  { item: MaterialId::Bread, count: 6 },
    QuestFlavour::Make  { item: MaterialId::Cake, count: 1 },
];

const BREWER_POOL: &[QuestFlavour] = &[
    QuestFlavour::Fetch { item: MaterialId::Berries, count: 8 },
    QuestFlavour::Fetch { item: MaterialId::HoneyBottle, count: 2 },
    QuestFlavour::Make  { item: MaterialId::BeetrootSoup, count: 3 },
    QuestFlavour::Make  { item: MaterialId::Pancakes, count: 4 },
];
```

Add the dispatch arms in the `quest_pool_for(profession)` (or equivalent) function:

```rust
Profession::Miller => Some(MILLER_POOL.as_slice()),
Profession::Baker => Some(BAKER_POOL.as_slice()),
Profession::Brewer => Some(BREWER_POOL.as_slice()),
```

**Implementer note**: verify each `MaterialId` and each Make recipe actually exists + resolves before committing the pool. Swap any unverified entries for verified ones from the existing T1.5 catalogue. The display name `HoneyBottle` was renamed to "Honey Jar" in HP-0 but the Rust identifier is still `MaterialId::HoneyBottle`; use the identifier.

### 6. Procgen pool integration

In `villager.rs` (or wherever the procgen profession sampler lives — search for `Profession::Farmer | Profession::Blacksmith | …`), add `Miller`, `Baker`, `Brewer` to whichever lists drive village procgen profession sampling. Two places observed in HP-1-era code per `villager.rs:284`-style listings:

```rust
[Profession::Farmer, Profession::Cook, Profession::Carpenter,
 Profession::Blacksmith, Profession::Scribe,
 Profession::Builder,
 Profession::Miller, Profession::Baker, Profession::Brewer]
```

This makes new villages spawn with workstations for the three new trades, and the existing claim logic (villager nearest to a workstation block claims it) wires the profession at run time.

### 7. Spec 27 Plan Registry `infer_category` arms

In `plan_registry.rs`, extend `infer_category` so plans containing the three new workstations classify correctly:

```rust
if has(block::MILL) {
    return PlanCategory::Workshop(Profession::Miller);
}
if has(block::OVEN) {
    return PlanCategory::Workshop(Profession::Baker);
}
if has(block::AGING_RACK) {
    return PlanCategory::Workshop(Profession::Brewer);
}
```

Place these BEFORE the existing `FURNACE` / `CAMPFIRE` / `TILLED_SOIL` arms in priority order — most specific wins. Add three matching unit tests next to the existing `infer_category_recognises_blacksmith_via_furnace` etc.

---

## Phasing

| # | Phase | Files | LOC est |
|---|---|---|---|
| 1 | **This spec** | — | — |
| 2 | `Profession::{Miller, Baker, Brewer}` enum variants + display arms + workstation-claim arms | `villager.rs` | ~30 |
| 3 | Gossip pools (3 × ~4 lines + dispatch) | `villager.rs` | ~30 |
| 4 | Quest pools (3 × ~4 entries + dispatch arms), implementer verifies MaterialId + Make recipes exist | `quest.rs` | ~60 |
| 5 | Procgen integration — add to the profession list(s) that drive village sampling | `villager.rs` / `village_gen.rs` | ~10 |
| 6 | Spec 27 `infer_category` 3 new arms + 3 new unit tests | `plan_registry.rs` | ~40 |
| 7 | Tests (per-profession claim/display/gossip; quest_pool_for returns Some for each; procgen sampling produces all 3) | various | ~80 |
| 8 | Docs (HP-7 spec → DELIVERED, README HP-7 row flip) | docs | ~25 |
| 9 | Axolittle playtest gate (visit a village with a Mill/Oven/Aging Rack; right-click villager; verify profession + quest + gossip work) | — | — |

**Total**: ~275 LOC across 7 build phases + 1 playtest gate. Single PR.

---

## What this PR does NOT do

- Doesn't add new workstation blocks (Mill / Oven / Aging Rack already exist from Spec T1.5).
- Doesn't add new mobs, animals, or threat types.
- Doesn't touch HP-2/3/4/5 surfaces (Bears, Hyenas, Brigands, Knights, Wave Refresh).
- Doesn't fix `scatter_mobs_in_column` broader passive-mob spawn issue.
- Doesn't add workstation-proximity gating for Make quests (same deferral as Spec 19 Phase 9 / Spec 20 Phase 9 — needs Axolittle UX air for radius + duration tuning).
- Doesn't add Cooper / Tanner / Fletcher / Mason — those are possible Sub 8+ additions if a future foundation needs them.
- Doesn't add per-profession unique recipes — Miller doesn't unlock new milling recipes, just claims the Mill.
- Doesn't add profession-specific clothing/skins — visual polish; defer.

---

## Tests

```rust
#[test]
fn miller_claims_mill_workstation() {
    assert_eq!(
        Profession::from_workstation_block(block::MILL),
        Some(Profession::Miller),
    );
}

#[test]
fn baker_claims_oven_workstation() {
    assert_eq!(
        Profession::from_workstation_block(block::OVEN),
        Some(Profession::Baker),
    );
}

#[test]
fn brewer_claims_aging_rack_workstation() {
    assert_eq!(
        Profession::from_workstation_block(block::AGING_RACK),
        Some(Profession::Brewer),
    );
}

#[test]
fn new_professions_have_display_labels() {
    assert_eq!(Profession::Miller.display_name(), "Miller");
    assert_eq!(Profession::Baker.display_name(), "Baker");
    assert_eq!(Profession::Brewer.display_name(), "Brewer");
}

#[test]
fn new_professions_have_gossip_pools() {
    // each pool returns a non-empty slice
    assert!(!gossip_for(Profession::Miller).is_empty());
    assert!(!gossip_for(Profession::Baker).is_empty());
    assert!(!gossip_for(Profession::Brewer).is_empty());
}

#[test]
fn new_professions_have_quest_pools() {
    assert!(quest_pool_for(Profession::Miller).is_some());
    assert!(quest_pool_for(Profession::Baker).is_some());
    assert!(quest_pool_for(Profession::Brewer).is_some());
}

#[test]
fn plan_registry_infers_miller_workshop_via_mill() {
    let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::MILL }];
    let p = plan("mill-room", PlanLicense::CCBYSA, cells, 3, 3);
    assert!(matches!(infer_category(&p), PlanCategory::Workshop(Profession::Miller)));
}

#[test]
fn plan_registry_infers_baker_workshop_via_oven() {
    let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::OVEN }];
    let p = plan("bakery", PlanLicense::CCBYSA, cells, 3, 3);
    assert!(matches!(infer_category(&p), PlanCategory::Workshop(Profession::Baker)));
}

#[test]
fn plan_registry_infers_brewer_workshop_via_aging_rack() {
    let cells = vec![CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::AGING_RACK }];
    let p = plan("brewery", PlanLicense::CCBYSA, cells, 3, 3);
    assert!(matches!(infer_category(&p), PlanCategory::Workshop(Profession::Brewer)));
}
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- All Phase 2-7 tests pass.
- A new villager placed near a Mill claims `Profession::Miller`; near an Oven claims `Baker`; near an Aging Rack claims `Brewer`.
- Right-clicking such a villager shows the new display label + a non-empty gossip line + offers Fetch/Make quests from the new pool.
- Plan Registry correctly categorises bundled plans whose primary workstation is Mill / Oven / Aging Rack.
- Save format unchanged (positional bincode; new variants append at end).
- No PROTOCOL_VERSION bump (no on-wire change).

---

## Memory-rule check

- ✓ uk english naming — "Miller", "Baker", "Brewer" are UK English (and US English; no spelling divergence here).
- ✓ bitcoin parent controlled — orthogonal; quest payouts use the existing `apply_sats_payout` Charter gates.
- ✓ shared infra strategy — profession + workstation-claim pattern lifts cross-game.
- ✓ merge to main preauthorised — healthy-gate merge fine when check.sh is green.
- ✓ HP-0 naming principle — "Miller", "Baker", "Brewer" are the proper medieval English for these trades; Mojang doesn't have these as named professions (Mojang's Cartographer / Fletcher / Leatherworker are different roles), so no Mojang-coincidence concern.

---

## Out of scope (deferred to later subs or future passes)

- Cooper / Tanner / Fletcher / Mason — possible Sub 8+ additions.
- Workstation-proximity gating for Make quests — Spec 19 Phase 9 deferral applies here too.
- Per-profession unique recipes (e.g., Brewer-only mead) — could add later; HP-7 ships the profession + quest pool only.
- Profession-specific clothing/skins — visual polish.
- New workstation blocks (Cooperage, Tannery, Fletchery) — would derive from the Sub 8+ professions if needed.

---

## Open questions

- **Quest pool entries**: I sketched plausible ones using known T1.5 materials, but the implementer should verify each `MaterialId` and Make recipe actually exists and resolves before committing. Any quest entry that references a non-existent material gets swapped for one that exists.

---

## Delivery notes (2026-05-23)

Implemented in a single PR off `feat/historical-pivot-sub7-miller-baker-brewer`.

**Quest-pool swaps**. The spec sketched pools using `Flour`, `Cake`, `Pancakes`, `BeetrootSoup` and a `Bread`-make for the Baker. All four materials exist as `MaterialId` variants, but none of them are obtainable in survival on alpha — Mill / Oven / Aging Rack workstation right-click recipes are deferred (`item.rs` §"T1.5 Processed Economy Base"), and crafting-table recipes for them don't ship either. Quest pools were rewritten to use only verified-obtainable materials:

- **MILLER_POOL**: Fetch Wheat 8/16, Make Bread 4/8 (wheat is the miller's input; bread is the closest craftable downstream product via the existing 3-wheat horizontal recipe).
- **BAKER_POOL**: Fetch Wheat 6, Fetch Bread 3, Make Bread 3/6.
- **BREWER_POOL**: Fetch Berries 8/16, Fetch HoneyBottle 2/4 (HoneyBottle is reachable via Bee Hive + Bucket per `bee_hive.rs::resolve_right_click`).

A new `hp7_quest_pools_use_obtainable_materials_only` test guards against future regressions reintroducing unreachable items into these three pools.

**Procgen integration**. The `alpha_professions` list in `village_gen.rs` was extended to include Miller / Baker / Brewer, and `build_hardcoded_workshop`'s workstation table was extended with `Profession::Miller → MILL`, `Baker → OVEN`, `Brewer → AGING_RACK` so the corresponding villager-claim path lights up at run time.

**Plan-registry ordering**. The three new `infer_category` arms are placed **before** the FURNACE arm so a plan containing both a Mill and a Furnace classifies as Miller (most-specific workstation wins). A new `infer_category_t1_5_workstations_win_over_furnace` test guards the ordering.

**Save / wire compat**. No `PROTOCOL_VERSION` bump. The new `Profession` variants are appended to the enum (bincode-positional), so existing saves with old `Profession` indices round-trip unchanged.

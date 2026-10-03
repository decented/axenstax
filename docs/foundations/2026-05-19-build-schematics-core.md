# Build Schematics Core — Plan items + capture + auto-build (Foundation B)

> **AMENDED 2026-05-28 by Spec 38 (Blueprint / Cyanotype)** — the capture
> front-end has been rewritten. The block formerly known as **Plan Tile**
> is now **Blueprint Paper** (same block id `56`, renamed const
> `block::BLUEPRINT_PAPER`, display name "Blueprint Paper"). The
> `Stick + PapyrusSheet → 9 Plan Tiles` recipe is retired; the new
> sensitisation recipe is `Papyrus Sheet + Iron + Salt → 3 Blueprint
> Paper` (vertical column). Fresh captures now land as **Latent** plans
> (`PlanData.develop_state = DevelopState::Latent { exposure_ticks: 0 }`)
> rather than ready-to-build; the player lays the Latent Plan back into
> the world as a `LATENT_PRINT` block (id `158`), waits ~90 s of
> cumulative open-sky daylight, and the embedded `develop_state` flips
> to `Developed`. Right-clicking the LATENT_PRINT retrieves the Plan
> (Latent or Developed — whichever has been reached) back into
> inventory. Plaque, animated build, content-hash + Save-As derivation,
> placement, licensing — all unchanged and downstream of develop. See
> [`2026-05-27-blueprint-cyanotype.md`](2026-05-27-blueprint-cyanotype.md)
> for the full design rationale (cyanotype process — Herschel 1842 +
> Anna Atkins 1843).

**Status:** **DELIVERED 2026-05-20** across three branches (`feat/build-schematics-core` → `feat/build-schematics-phase-5` → `feat/build-schematics-phase-8`). All solo phases (1-16) green; Phase 17 = Axolittle playtest gate. Final close-out branch flips Phase 6 (Save-As derivation — content-hash + 50%-block match + Master-only guard, dialog checkbox, derivation-chain prepend) and Phase 8 (Wireframe ghost preview + Q/E rotation + tri-state colour tint + raycast anchor + left-click commit + right-click cancel) from deferred to done.
**Branch:** `feat/build-schematics-core` off `main` (post-Spec-23).
**Trigger:** Foundation **B** of the Build Schematics economy (`docs/vision/build-schematics-long-run.md`). Implements Phases 1, 2, and 4 of the vision doc's six-phase lifecycle (Author / Inspect / Player-Self-Build) — no trade economy yet (Spec 25), no NPC commission (Spec 26), no procgen (Spec 27).

---

## TL;DR

A player lays Plan Tiles around a building, right-clicks any tile with empty hand, names + licences the plan, and gets a 📜 **Plan** item. Right-clicking the Plan opens an Inspect dialog with the material list. With materials in inventory, clicking "Place in world" enters wireframe ghost mode; Q/E rotates, left-click confirms, a 2-blocks-per-tick animated builder lays the structure, and an **Architect's Plaque** crediting the player drops in last. Saving + reloading preserves both completed builds (just blocks) and in-progress builds (via `ConstructionAnchor` markers).

### New shape in one paragraph

`Item::Plan(PlanData)` joins `Block/Tool/Material` as a fourth `Item` variant — non-stacking, per-instance content. `PLAN_TILE` is a new block (id 56). Capture is right-click-empty-hand on a Plan Tile, which flood-fills connected tiles (4-connected XZ), verifies a flat-ground build envelope, runs a 6-connected upward flood-fill to find every block in the captured volume, then opens an egui dialog for naming + licence (4-way picker, CC-BY-SA default with sticky per-player preference). Save-As is detected via SHA-256 content-hash + 50%-block-match on Master plans in the player's inventory; only Master holders can derive. **`ConstructionAnchor`** (id 57) marks an in-progress build site; **`ARCHITECT_PLAQUE`** (id 58) auto-places last with the full derivation chain.

### Phases summary

| # | Phase | Files | Est. LOC |
|---|-------|-------|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-19-build-schematics-core.md` | ~750 |
| 2 | `Item::Plan(PlanData)` + bincode tests | `item.rs`, `crafting.rs` (CraftSlot), `inventory.rs` (stack rules) | ~250 |
| 3 | `PLAN_TILE` block (id 56) + texture + per-grade recipes (`1 stick + 1 PapyrusSheet → 9 tiles`; future `+ 1 PulpPaper → 25 tiles` lands with Spec 12) | `block.rs`, `texture_gen.rs`, `crafting.rs` | ~150 |
| 4 | Capture mechanic — flood-fill + flat-ground check + volume sweep (tiles consumed, NOT refunded) | new `plan.rs` | ~250 |
| 5 | Capture dialog (egui) + name + licence picker + first-time onboarding flag | new `plan_ui.rs`, `save.rs::WorldMeta`, `player_slot.rs` | ~200 |
| 6 | Save-As derivation — SHA-256 content-hash + 50% match + Master-only guard | `plan.rs`, `plan_ui.rs` | ~150 |
| 7 | Inspect dialog — ASCII top-down preview + material list + place button | `plan_ui.rs`, `inventory_ui` integration | ~150 |
| 8 | Wireframe ghost preview + Q/E rotation + validity colours | `renderer.rs` or new `ghost.rs` | ~200 |
| 9 | Placement validation (flat / empty volume / no-player-in-volume / loaded chunks) | `plan.rs` | ~80 |
| 10 | `ConstructionAnchor` block (id 57) + resume dialog + admin-only break | `block.rs`, `plan_ui.rs`, `plan.rs` | ~150 |
| 11 | Animated build — 2 blocks/tick Y→X→Z + HUD overlay + Esc cancel | `plan.rs`, `game_loop.rs`, `hud_ui.rs` | ~200 |
| 12 | `ARCHITECT_PLAQUE` block (id 58) + attribution dialog (chain display + tip placeholder) | `block.rs`, `plan_ui.rs` | ~150 |
| 13 | Save/load — `construction_anchors` + `architect_plaques` Vec fields, `#[serde(default)]` | `save.rs` | ~80 |
| 14 | `/give` debug aliases — plan_tile / construction_anchor / architect_plaque / debug plan | `commands/builtins/give.rs` | ~40 |
| 15 | PROTOCOL_VERSION 16 → 17 with history note | `protocol.rs`, `test_integration/handshake.rs` | ~10 |
| 16 | Spec 5 §3.13 Build Schematics subsection + foundations README + vision doc flip | `docs/spec/05-gameplay-systems.md`, `docs/foundations/README.md`, `docs/vision/build-schematics-long-run.md` | ~80 |
| 17 | Axolittle playtest — capture → place → animated build → plaque round-trip; Save-As derivation; multi-licence scenarios | n/a | 0 |

**Total**: ~2,000 LOC including tests. Phases 2-16 autonomous. Phase 17 is the playtest gate.

Recommended order: 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12 → 13 → 14 → 15 → 16. Each phase reaches a check.sh-green state before the next starts.

---

## Why this lives here

- **Largest single piece of the Build Schematics economy.** Specs 25/26/27 all consume what this spec ships (Plan item, Plaque, capture). This is the meaningful first ship — players can capture + self-build + derive + plaque-credit with no economy yet.
- **Closes the vision-doc lifecycle Phases 1, 2, and 4.** Author / Inspect / Player-Self-Build complete. Trade (Phase 3) is Spec 25, NPC (Phase 5) is Spec 26, procgen (Phase 6) is Spec 27 — each shipping independently after this.
- **Cross-game lift, per shared infra strategy.** Plan-item + connectivity-flood-fill capture + Master/Licence/Derivative tier model + CC-licence enum + Plaque derivation chain + ghost preview + animated builder — all engine-generic. The specific recipe (1 stick + 1 paper → 4 tiles), the licence default (CC-BY-SA), and the build cadence (2/tick) are AxeNStax-specific data.
- **Foundation for the kid-creator economy.** Once shipped, Axolittle can capture his own builds and replay them anywhere. The Plaque carries his attribution forever. Even pre-sats, that's a real creative tool.

---

## Creative vs Survival — no restriction, full transparency

Plans behave **identically across game modes** with two narrow exceptions:

1. **Build-time material consumption is mode-aware.** Survival decrements the locked-materials snapshot from the player's inventory (Phase 11 step 2). Creative skips the lock entirely — the animated builder lays blocks from thin air. The Inspect dialog's material list still renders in creative (informational only — useful for *designing* survival-targeted plans), but the "Place in world" button is never greyed for materials.
2. **PlanData carries an `authored_in: GameMode` field.** Populated at capture time. Surfaced in the Plaque attribution dialog ("Authored in Creative" / "Authored in Survival") and in any future Vendor Block listing (Spec 25). Tag, don't gate.

**The tile-consumption rule is mode-agnostic.** Tiles are NOT refunded on capture in either mode. In creative the player can `/give plan_tile` freely, so there's no economic argument for distinguishing — keeping the rule uniform avoids a "did I capture this in survival or creative?" inventory-audit surprise. Paper is the real cost; in creative, paper is itself free, so the rule is academic.

**No one-way transfer restriction.** Plans authored in creative mode are first-class. They can be traded, derived, sold, and used by survival players without any caste-marker beyond the visible authorship tag. The mental model is **architect-at-the-CAD-station** — the design IS the deliverable; materials are someone else's problem to source. A creative-authored plan still requires every block to be gathered to *build* in survival, so creative authorship doesn't bypass the survival economy — it just removes the layout-design labour for the buyer.

### Where policy layers can hook in later (NOT this spec)

- **Per-server policy** (Spec 6 §13 economy modes) can ban creative-authored plans from the Vendor Block trade flow on purist survival servers.
- **Village procgen registry** (Spec 27) can prefer or require survival authorship to keep hardcoded-village texture intact — TBD by that spec.
- **Buyer-side filter** (Spec 25 Vendor Block) can let survival players filter listings by `authored_in`.

None of this lives in Spec 24. Spec 24 ships the **field + transparency**; downstream specs add policy.

### Why this is the right cut (one-paragraph rationale)

A plan is a template, not a teleport. The survival player must still gather every block to build, regardless of who authored it. Restricting creative→survival would penalise design as a form of labour (sketching a tavern is real work; arguably more skilled than chopping the logs), create a content-moderation nightmare (mode-switching mid-capture, derivation from mixed-mode ancestors), and break the cross-mode supply asymmetry that's the design's secret weapon (creative architects → survival builders is the same dynamic as real-world architecture firms → real-world construction crews). Plaque attribution + content-hash already give us full audit. Show authorship; let policy filter; never gate.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/item.rs`
  - Add `Item::Plan(PlanData)` variant alongside `Block/Tool/Material`.
  - `Item::can_stack_with` — `Plan` never stacks (per-instance content).
  - `Item::name` — display the plan's stored name. `Item::color` — dark-gold parchment hex.
  - `Item::max_stack` — 1 for plans.
  - Trace every `match ... Item::Block | Tool | Material` — most are exhaustive matches that the compiler will flag once the new variant lands. Add explicit `Plan` arms returning sensible defaults: 0 attack damage, no food, no placeable-as-block.
- `game/engine/src/block.rs`
  - 3 new block IDs: `PLAN_TILE = 56`, `CONSTRUCTION_ANCHOR = 57`, `ARCHITECT_PLAQUE = 58` (after PAPYRUS_STAGE_3 = 55).
  - 6 new texture layers: PLAN_TILE_TOP / PLAN_TILE_SIDE / CONSTRUCTION_ANCHOR / ARCHITECT_PLAQUE_TOP / ARCHITECT_PLAQUE_SIDE / placeholder. Layers 166..=171.
  - `BlockRegistry::new()` — push 3 BlockDefs. PLAN_TILE is `solid: true, transparent: false`; ARCHITECT_PLAQUE same; CONSTRUCTION_ANCHOR `solid: false, transparent: true` (it's a marker — walk through it).
  - `mine_drop` — PLAN_TILE drops itself; CONSTRUCTION_ANCHOR + ARCHITECT_PLAQUE are admin-protected (mine_drop unreachable in normal play but returns the block-item for `/give` round-trip).
- `game/engine/src/crafting.rs`
  - New 1×2 vertical recipe: `[stick]` over `[any paper via is_paperish_slot]` → 4 PLAN_TILEs. Mirrors the flint-and-steel 1×2 pattern.
- `game/engine/src/save.rs::WorldSave`
  - `#[serde(default)] pub construction_anchors: Vec<SavedConstructionAnchor>` — anchor pos + plan content (so resume works after reload) + orientation + locked-materials snapshot + progress index.
  - `#[serde(default)] pub architect_plaques: Vec<SavedArchitectPlaque>` — pos + derivation chain. (The Plaque's data is server-of-truth; right-click dialog reads from this Vec via a position lookup.)
- `game/engine/src/save.rs::WorldMeta`
  - `#[serde(default)] pub has_seen_license_onboarding: bool` — first-time-modal trigger. **(2026-06-22 fix)** The source of truth moved to `GraphicsSettings::has_seen_license_onboarding` (per-device, cross-platform persisted — localStorage on web, `settings.json` native) because the per-world meta flag was **never persisted on WASM**, so the CC-BY-SA modal re-blocked the capture dialog every web session ("i can't save any blueprints"). Load now reads `meta.has_seen_license_onboarding || graphics.has_seen_license_onboarding` (native legacy worlds still honoured); dismiss writes the graphics flag (both platforms) and keeps the meta flag in sync on native.
- `game/engine/src/player_slot.rs::PlayerSlot`
  - `pub last_chosen_license: Option<PlanLicense>` (transient, not persisted — alpha-tunable to per-save persistence if Axolittle wants it sticky across sessions).
- `game/engine/src/protocol.rs`
  - `PROTOCOL_VERSION 16 → 17`. Version-history entry.
- `game/engine/src/game_loop.rs`
  - Right-click handler — new branches:
    - `target_blk == PLAN_TILE` + empty hand → start capture sequence (flood-fill + dialog).
    - Held item is `Item::Plan(_)` → open Inspect dialog.
    - `target_blk == CONSTRUCTION_ANCHOR` + holding the matching plan → open resume dialog; without plan → abandon-only dialog.
    - `target_blk == ARCHITECT_PLAQUE` → open attribution dialog (read-only in v1).
  - Per-tick: drive any in-progress animated builds (2 blocks/tick).
  - Esc handler: if a build is in progress for this player, open cancel-confirm dialog.

### New modules

- `game/engine/src/plan.rs` (~600 LOC) — `PlanData`, `PlanLicense`, `DerivationLink`, capture flood-fill, hash, Save-As detection, placement validation, animated-build state machine.
- `game/engine/src/plan_ui.rs` (~400 LOC) — egui dialogs: Capture, Inspect, Resume, Plaque-attribution, License-onboarding-modal.
- (Optional) `game/engine/src/ghost.rs` — wireframe block-edge rendering. May fold into `renderer.rs` if it's just a few hundred lines.

### Related specs

- `docs/vision/build-schematics-long-run.md` — the design contract. Phases 1, 2, 4 live in this spec.
- `docs/foundations/2026-05-19-papyrus-reed.md` (Spec 23) — supplies `MaterialId::PapyrusSheet` + `is_paperish_slot`. Recipe input.
- `docs/foundations/2026-05-18-villages-and-villagers.md` (Spec 19) — Plaque visual + admin-protection patterns mirror the Village Bell. Plaque carries derivation chain; visible to anyone on right-click.
- `docs/foundations/2026-05-18-vendor-block.md` (Spec 21) — Spec 25 (Plan Trade) plugs into Vendor Block sub-modes. Not touched by this spec.
- `docs/foundations/2026-05-18-furnace.md` (Spec 20) — introduces `apply_server_tax_and_payout` (used by Spec 25's tipping flow). Not touched here — Spec C handles all sats flows.

### Memory pointers

- uk english naming — "Plan", "Plan Tile", "Architect's Plaque" (possessive apostrophe), "licence" (noun) / "license" (verb — but field names use `license` per Rust convention + cross-game-lift readability). Vision doc uses American "license" for the enum; preserved for code consistency.
- pretest check — block IDs 56+ available after Papyrus took 52-55. PROTOCOL_VERSION = 16 post-Spec-23. Texture layers 166+ available. Item enum has 4 variants once `Plan` is added — every match-on-Item compile site must add a new arm.
- autonomy to playtest boundary — Phases 2-16 autonomous, Phase 17 = playtest.
- shared infra strategy — Plan + Plaque + derivation chain primitives lift cross-game. AxeNStax-specific: 2-blocks/tick build rate, the 4-licence enum's specific entries, the recipe inputs.
- bitcoin parent controlled — outgoing sats (tipping) is gated. **No sats flow in this spec** — Plaque attribution dialog renders the chain + tip-target placeholder, but tipping itself ships in Spec 25.
- proof of play is proof of work — orthogonal. No hashing on plans except the content-hash for Save-As detection.

### What does NOT exist yet (and Spec 24 does NOT need)

- **Vendor Block trade integration.** Spec 25 territory. Plans are just inventory items in this spec; selling them happens in Spec 25.
- **Plaque tipping flow.** Spec 25 routes outgoing sats through `apply_server_tax_and_payout`. This spec renders the tip-target placeholder + chain display only.
- **NPC commission.** Spec 26 territory.
- **Procgen integration.** Spec 27 territory.
- **Slope handling.** Vision doc §9 — v2. This spec is flat-ground-only.
- **Multi-block-tall plans on hills.** Same — v2.
- **Multi-player ownership rules.** Vision doc §3.6 — deferred. Single-player owns this spec end-to-end.
- **Cryptographic plan signing via Nostr.** Vision doc §5.8 — v2.

---

## Phase 1 — This spec

You're reading it. ✓

---

## Phase 2 — `Item::Plan(PlanData)` variant

### Goal

The `Item` enum grows a `Plan(PlanData)` variant. `PlanData` is a serde-stable struct with name, author npub, licence, derivation chain, content cells, and footprint. Bincode round-trips. Every match-on-Item in the codebase gains an explicit `Plan` arm.

### `PlanData` design

```rust
/// Spec 24 — captured-building blueprint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanData {
    /// Forward-compat byte. Always 1 for v1 captures; bump on any
    /// breaking content change.
    pub version: u8,
    pub name: String,
    /// Author's Nostr pubkey. Empty string in solo single-player until
    /// Signet auth is mandatory (Spec 1 Phase 4 cutover).
    pub author_npub: String,
    pub license: PlanLicense,
    /// Ordered ancestor chain. v1 captures are length 1 with just
    /// self; derivatives prepend the parent's chain + add the new
    /// entry on top.
    pub derivation_chain: Vec<DerivationLink>,
    /// Footprint bounding-box dimensions (computed at capture time).
    pub width: u8,
    pub depth: u8,
    pub height: u8,
    /// Captured cells. Only non-air blocks are stored. (rel_x, rel_y,
    /// rel_z, block_id). Relative coords are 0..width, 0..height,
    /// 0..depth — relative to the lowest-XZ tile corner at y=base.
    pub cells: Vec<CapturedCell>,
    /// The game mode the plan was authored in. Populated at capture
    /// time from the world's current mode. Surfaced in the Plaque
    /// attribution dialog + Vendor Block listings (Spec 25). NOT used
    /// to gate trade/derivation — see "Creative vs Survival" section
    /// above. `#[serde(default)]` lets pre-amendment captures load as
    /// `GameMode::Survival` (the conservative assumption).
    #[serde(default)]
    pub authored_in: GameMode,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapturedCell {
    pub rx: u8, pub ry: u8, pub rz: u8,
    pub block_id: BlockId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivationLink {
    pub author_npub: String,
    pub plan_name: String,
    pub license: PlanLicense,
    pub captured_at: u64,
    pub plan_hash: [u8; 32],
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanLicense {
    AllRightsReserved,
    CC0,
    CCBYSA,    // Platform default per vision §5.5
    CCBYND,
}
```

### Changes

- `item.rs`:
  - Add `Plan(PlanData)` to `Item` enum.
  - Match arms: `Item::Plan(p) => p.name.clone()` for name; dark-gold colour `[0.65, 0.50, 0.20]`; `max_stack = 1`; `attack_damage = 1.0`; `food_value = None`; `as_block = None`; `can_stack_with(Plan(_), _) = false`.
  - All other existing helpers add a `Plan` arm returning the no-op default.
- `crafting.rs`:
  - `CraftSlot` — add `Plan` variant if the slot system needs to represent plans (probably not — plans aren't crafting inputs in v1; the variant is added only if the compiler requires it for exhaustive matches).
- `save.rs`:
  - `SavedSlot` gains a `Plan { data: PlanData }` variant. `serialize_inventory` + `restore_inventory` extend with the new arm.
  - Bincode-positional safe: append at end of `SavedSlot` enum.

### Tests

- `plan::tests::plan_data_bincode_round_trip` — full PlanData with chain + cells survives serialise/deserialise.
- `plan::tests::plan_with_long_chain_round_trips` — 10-deep derivation chain.
- `item::tests::plan_never_stacks` — `can_stack_with` returns false for `Plan/Plan` and `Plan/Block` pairs.
- `item::tests::plan_max_stack_is_one`.
- `save::tests::saved_slot_plan_variant_round_trips_bincode`.

### Acceptance

`cargo test` green. `check.sh` ALL GREEN.

---

## Phase 3 — `PLAN_TILE` block + recipe

### Goal

`PLAN_TILE` exists as a placeable, mineable block (id 56). Texture: parchment-cream top with a dark border on the sides — readable as a drafting square. **Two recipes, one per paper grade** — yield scales with paper quality, mirroring how real drafting paper grades scale:

- **`1 stick + 1 PapyrusSheet → 9 PLAN_TILEs`** (T1 entry-grade; 3×3 floor-section yield)
- **`1 stick + 1 PulpPaper → 25 PLAN_TILEs`** (T1.5 mill-grade; 5×5 floor-section yield) — *recipe lands when Spec 12 ships PulpPaper; this spec stubs only the papyrus recipe*

Both use the campfire's flint-and-steel 1×2 vertical layout. Paper is irreversibly converted into tiles; tiles cannot be melted back into paper. **Tiles are NOT refunded on capture** (see Phase 4 step 8) — every plan locks in its paper cost permanently, mirroring real architectural drawings.

### Why the per-grade split

Plan-making at scale needs cheaper paper, and T1.5 Mill infrastructure is the natural unlock. Papyrus is hand-farmed reed → 9 tiles per sheet keeps the entry-tier honest (a 10×10 plan needs ~12 papyrus). Pulp Paper requires the Mill workstation (Spec 12 Phase 5) — investing in the mill rewards the player with 2.78× better paper yield. A 10×10 plan drops from 12 papyrus to 4 pulp. A 20×20 mansion drops from 45 papyrus to 16 pulp. Real upgrade arc.

The shared `is_paperish_slot` predicate (Spec 23) stays in use for OTHER paper-consumers (books, maps, quest scrolls) where grade is irrelevant. Plan Tile crafting specifically forks per grade because the yield differs.

### Changes

- `block.rs`:
  - `PLAN_TILE: BlockId = 56`.
  - `TEX_PLAN_TILE_TOP = 166`, `TEX_PLAN_TILE_SIDE = 167`.
  - `BlockDef` — solid, opaque, non-gravity. `tex_top = TEX_PLAN_TILE_TOP`, `tex_side = TEX_PLAN_TILE_SIDE`, `tex_bottom = TEX_PLAN_TILE_SIDE`.
  - `mine_drop(PLAN_TILE)` → 1 PLAN_TILE block-item (unused tiles can be mined + relaid; only *committed-to-plan* tiles are consumed).
- `texture_gen.rs`:
  - `gen_plan_tile_top()` — parchment-cream base with a faint grid pattern.
  - `gen_plan_tile_side()` — dark walnut border, suggesting a drafting board.
  - `texture_count()` increment.
- `crafting.rs`:
  - 1×2 vertical: stick on top, **PapyrusSheet** below → **9 PLAN_TILEs**.
  - (Future) 1×2 vertical: stick on top, **PulpPaper** below → **25 PLAN_TILEs** — added by Spec 12 Phase 5 (Mill) cross-spec follow-on.

### Tests

- `block::tests::plan_tile_registered`
- `crafting::tests::plan_tile_recipe_yields_nine_from_papyrus_sheet`
- `crafting::tests::plan_tile_recipe_rejects_pulp_paper_in_papyrus_arm` — separate recipes; Pulp arm doesn't ship until Spec 12.
- `crafting::tests::plan_tile_recipe_rejects_wrong_order` — paper on top, stick below → no match.

### Acceptance

Tests green. Manual: 9 tiles from 1 stick + 1 papyrus sheet.

---

## Phase 4 — Capture mechanic

### Goal

Right-click any PLAN_TILE with empty hand → engine collects every connected tile (4-connected XZ at the tile's Y), verifies flat-ground (every cell above must be AIR; every cell below must be solid), runs a 6-connected 3D flood-fill upward from each tile-adjacent column, and produces a `PlanData` candidate. **Tiles are consumed by the capture** (set to AIR, not refunded) — paper irreversibly transcribes into a plan, mirroring how real drafting paper commits to an archive. Unused tiles (never confirmed-into-a-plan) can be mined and relaid normally; only committed tiles vanish.

Failure modes (each toast-specific):
- **"Tiles must all be connected"** — disconnected groups detected.
- **"Plot must be flat — clear obstructions or fill divots first"** — flat-ground rule fails.
- **"Plan too large — max 32×32×32"** — exceeds envelope.
- **"Captured volume is empty"** — only the tiles themselves, no build on top.

### Algorithm

1. **Collect tile cells**: flood-fill 4-connected from the right-clicked tile's XZ at Y; gather every connected PLAN_TILE position.
2. **Flat-ground check**: for each tile cell, verify `world.get_block(x, y+1, z) == AIR` initially (will hold structure later) and `world.get_block(x, y-1, z)` is solid.
3. **Bounding-box compute**: `min_x..=max_x`, `min_z..=max_z` over tile cells; `width = max_x - min_x + 1`, `depth = max_z - min_z + 1`.
4. **Envelope guard**: `width <= 32 && depth <= 32`.
5. **Build-volume flood-fill (6-connected, upward)**: BFS starting from each `(tile_x, y+1, tile_z)`. Frontier = AIR cells; capture cell = non-AIR. Stop when no new non-AIR neighbours found OR height > 32. Collect every reached non-AIR cell as a `CapturedCell` with relative coords.
6. **Height envelope guard**: `height <= 32`. If exceeded, refuse with envelope toast.
7. **Empty-volume guard**: `cells.is_empty()` → refuse with "Captured volume is empty".
8. **Consume tiles**: each PLAN_TILE position is replaced with AIR. **No refund** — tiles vanish into the plan. (This is the "paper-locked-in" rule. Mining unused tiles before capture still refunds them as items; only the capture step consumes them.)
9. **Produce `PlanData`**: with cells, footprint, default name `"Plan: {width}×{depth}"`, default licence (player's last_chosen_license or CC-BY-SA), empty author_npub for now (Spec 1 Phase 4 cutover later), `authored_in` set from `world.game_mode()` at capture time, derivation_chain = single self-entry with hash computed in §6.
10. **Open Capture dialog** (Phase 5) with the candidate.

If the player cancels the dialog, the candidate is dropped and the tiles remain placed in the world untouched (no rollback needed — the tile consumption + AIR replacement happens only on Confirm). The player can mine unused tiles back into inventory if they want.

### Implementation note

The capture and the dialog are decoupled. Capture produces a `Result<PlanCaptureCandidate, CaptureRefusal>`; the dialog code holds the candidate and only commits on Confirm. This keeps the flood-fill logic pure-function-testable without dragging egui into `plan.rs`.

### Changes

- New `game/engine/src/plan.rs`:
  - `pub struct PlanCaptureCandidate { pub data: PlanData, pub tile_positions: Vec<(i32,i32,i32)> }`.
  - `pub enum CaptureRefusal { Disconnected, NotFlat, TooLarge, EmptyVolume }`.
  - `pub fn capture(world: &World, click_pos: (i32, i32, i32)) -> Result<PlanCaptureCandidate, CaptureRefusal>`.
  - Helper free functions: `flood_fill_tiles`, `check_flat_ground`, `flood_fill_volume`.
- `game_loop.rs`:
  - New right-click branch: `target_blk == PLAN_TILE` + empty hand → call `plan::capture`. On Ok, store the candidate in the player's transient UI state (`PlayerSlot.pending_capture: Option<PlanCaptureCandidate>`). The egui dialog (Phase 5) renders this state.

### Tests

- `plan::tests::flood_fill_tiles_returns_single_for_isolated_tile`
- `plan::tests::flood_fill_tiles_returns_all_connected`
- `plan::tests::flood_fill_tiles_does_not_cross_y_levels` — tiles at different Y don't merge.
- `plan::tests::check_flat_ground_passes_on_uniform_dirt_under_air`
- `plan::tests::check_flat_ground_fails_on_obstructed_cell` — stone above a tile fails.
- `plan::tests::check_flat_ground_fails_on_unsupported_cell` — AIR below a tile fails.
- `plan::tests::flood_fill_volume_captures_simple_box` — 3×3 base + 4-high walls → 28 cells.
- `plan::tests::flood_fill_volume_includes_chain_connected_lamp` — chain-block links a chandelier-style lamp; both included.
- `plan::tests::flood_fill_volume_excludes_floating_torch` — disconnected torch not included.
- `plan::tests::capture_refuses_disconnected_tiles`
- `plan::tests::capture_refuses_oversized_footprint`
- `plan::tests::capture_refuses_empty_volume`
- `plan::tests::capture_returns_candidate_on_success_without_world_mutation` (NOTE: capture is read-only — the commit happens in Phase 5's dialog Confirm. Test verifies the candidate's tile_positions match the input and the world is untouched until Confirm.)
- `plan::tests::commit_capture_consumes_tiles_without_refund` — after Confirm, tile positions are AIR and the player's inventory has gained NO tiles back (the paper-locked-in rule).

### Acceptance

Tests green. Manual: place tiles, build on top, right-click empty hand → toast or candidate produced.

---

## Phase 5 — Capture dialog (egui)

### Goal

After a successful capture, an egui modal renders with name input, licence picker, auto-detected material summary, Confirm/Cancel. Confirming commits the capture (tiles → AIR with no refund — the paper-locked-in rule, Plan item inserted into inventory). Cancelling leaves the tiles + built structure untouched in the world (the player can adjust + re-try, or mine the tiles back to recover them). On the player's first-ever capture in this world, an onboarding popup explains CC-BY-SA and how to change the default.

### Changes

- `save.rs::WorldMeta`: `#[serde(default)] pub has_seen_license_onboarding: bool` (defaults false).
- `player_slot.rs::PlayerSlot`:
  - `pub last_chosen_license: Option<PlanLicense>` (transient, in-memory).
  - `pub pending_capture: Option<PlanCaptureCandidate>`.
  - `pub pending_inspect_plan: Option<usize>` (hotbar slot index of an inspect target).
- New `game/engine/src/plan_ui.rs`:
  - `pub fn show_capture_dialog(ctx: &egui::Context, candidate: &mut PlanCaptureCandidate, last_license: &mut Option<PlanLicense>, has_seen_onboarding: &mut bool) -> CaptureDialogOutcome`.
  - `CaptureDialogOutcome::{InProgress, Confirmed, Cancelled}`.
  - First-frame branch: if `!*has_seen_onboarding`, render the onboarding modal; on dismiss, set the flag and proceed.
  - Body: name `egui::TextEdit::singleline`, licence radio buttons (4), material list (collected from candidate.cells), Confirm/Cancel buttons.
- `game_loop.rs`:
  - Per-frame: if any player has `pending_capture.is_some()`, render the dialog and react to outcomes:
    - `Confirmed`: set every tile_position to AIR (**no refund — tiles consumed by the plan**), insert a fresh `Item::Plan(candidate.data)` into the player's inventory at the first empty slot, clear `pending_capture`. Compute and store the content-hash in `derivation_chain[0].plan_hash` at this point.
    - `Cancelled`: clear `pending_capture` (tiles + build untouched).

### Tests

- `plan_ui::tests::onboarding_flag_flips_after_first_capture` — direct state test, no egui rendering.
- `plan::tests::confirm_commits_capture` — calls into a non-egui helper `commit_capture(world, inv, candidate)` and asserts the state changes.

### Acceptance

Tests green. Manual: capture flow happy-path produces a plan in inventory; onboarding modal shows the first time.

---

## Phase 6 — Save-As derivation

### Goal

When the player attempts to capture a structure that strongly resembles a Master plan in their inventory (content-hash matches OR ≥50% of blocks match by position), the capture dialog adds a checkbox: "Mark as derivative of «{name}»" (default checked). On confirm, the new PlanData's derivation_chain prepends the parent's chain + adds the new entry.

If the player holds only a **Licence** (not Master), the engine refuses with toast *"You hold a Licence for a similar plan, not the Master. Derivatives need the Master."*

For v1 every captured Plan is a Master — Licence-tier-distinction infrastructure isn't shipped until Spec 25. Add a `is_master: bool` flag on `PlanData` (default true) so the guard has something to gate on; in v1 it's always true.

### Content-hash

`fn content_hash(data: &PlanData) -> [u8; 32]`: `sha2::Sha256` over the bincode-serialised PlanData with `derivation_chain` zeroed out. This means a derivative's content-hash matches its parent IF the cells + footprint are identical (which is what we want for derivation detection). Different cell contents → different hash → not detected as derivative.

### 50% match

If the content-hash check returns False, fall back to position-by-position matching: for each candidate cell, check if a Master in inventory has a cell at the same `(rx, ry, rz)` with the same `block_id`. Compute `matched_cells * 2 >= total_cells_in_candidate`. If true, suggest derivation.

### Changes

- `plan.rs`:
  - `pub fn content_hash(data: &PlanData) -> [u8; 32]` — uses `sha2` crate (add to Cargo if not already a dep).
  - `pub fn detect_parent(candidate: &PlanData, inventory_plans: &[(usize, &PlanData)]) -> Option<(usize, ParentMatchKind)>` — returns hotbar slot index + match kind.
  - `pub enum ParentMatchKind { ContentHash, BlockMatch50 }`.
  - `is_master: bool` field on PlanData.
- `plan_ui.rs`:
  - Capture dialog gains the "Mark as derivative" checkbox conditional on a parent match.
  - License-tier refusal toast for Licence-tier holders (no-op in v1 since `is_master` is always true).

### Tests

- `plan::tests::content_hash_independent_of_derivation_chain`
- `plan::tests::content_hash_changes_when_cells_change`
- `plan::tests::detect_parent_via_content_hash`
- `plan::tests::detect_parent_via_50_percent_block_match`
- `plan::tests::detect_parent_returns_none_at_49_percent`

### Acceptance

Tests green. Manual: build a house, capture it, modify it, capture again — dialog offers "Mark as derivative".

---

## Phase 7 — Inspect dialog

### Goal

Right-click while holding a Plan item → modal opens with name, author, licence, **authored-in mode tag**, footprint preview (ASCII top-down grid), material list with green/red inventory delta, "Place in world" and "Close" buttons. The Place button is greyed if any material is short **in survival**; in creative the button is always live and the material list is shown informational-only.

### ASCII top-down

For a W×D footprint, render W rows × D columns of `#` / `.` (block-present / air at any height in that XZ column). Helps the player see L-shapes and complex footprints.

### Material list

Aggregate by `block_id`: for each `CapturedCell`, increment a HashMap<BlockId, u32> counter. Display as `{count} × {block_name}` with the player's current inventory count side-by-side. In survival: colour-coded green if owned >= needed, red otherwise. In creative: render the count + name as informational (no inventory comparison, no colour-coding) — the architect doesn't care what's in their hotbar; they're showing what the *plan* needs.

### Changes

- `plan_ui.rs`:
  - `pub fn show_inspect_dialog(...) -> InspectOutcome` — `InProgress / PlaceClicked / Closed`.
  - `pub fn ascii_footprint(data: &PlanData) -> String` — pure helper, testable.
  - `pub fn material_summary(data: &PlanData) -> Vec<(BlockId, u32)>` — pure helper.
- `game_loop.rs`:
  - Right-click handler when holding `Item::Plan(_)` → open Inspect dialog.
  - On `PlaceClicked`: transition the player into Ghost-preview mode (Phase 8).

### Tests

- `plan_ui::tests::ascii_footprint_renders_3x3` — block-present grid yields expected string.
- `plan_ui::tests::material_summary_aggregates_correctly`
- `plan_ui::tests::material_summary_empty_for_empty_plan`

### Acceptance

Tests green. Manual: right-click a plan in inventory → dialog renders correctly.

---

## Phase 8 — Wireframe ghost preview + Q/E rotation

### Goal

When the player clicks "Place in world", the engine enters Ghost mode for that player. A wireframe rendering of the plan's blocks tracks the player's cursor. Q/E (or scroll-wheel-with-modifier) rotates 90° about the centre. Colour-codes:
- **Green** = valid + materials sufficient
- **Yellow** = valid placement but inventory short of materials
- **Red** = invalid (terrain not flat / volume not empty / chunk not loaded)

Left-click on green → confirm + start build (Phase 11). Right-click or Esc → cancel.

### Cursor anchoring

Raycast from camera; anchor the plan's `min_y = cursor_block_y + 1`. The plan's footprint min-corner aligns with the cursor's XZ.

### Wireframe rendering

For each `CapturedCell`, draw the 12 edges of its 1×1×1 cube as line primitives at the cell's world position. Reuse the existing wireframe-render path (used for the targeting overlay around a hovered block). If the engine doesn't already have a dynamic line-batch renderer, add one minimally.

### Rotation

`pub fn rotate_cells(cells: &[CapturedCell], rotations: u8, width: u8, depth: u8) -> Vec<CapturedCell>`. `rotations ∈ {0, 1, 2, 3}` for 0°/90°/180°/270° clockwise.

### Changes

- `plan.rs`:
  - `pub struct GhostState { pub plan_hotbar_slot: usize, pub rotations: u8, pub anchor: (i32, i32, i32) }`.
  - `pub fn rotate_cells(...)`.
- `player_slot.rs`:
  - `pub ghost_state: Option<GhostState>`.
- `renderer.rs` (or new `ghost.rs`):
  - `pub fn draw_wireframe_for_ghost(..., color: [f32; 3])` — emits line vertices for each cell's 12 edges.
- `game_loop.rs`:
  - When `ghost_state.is_some()`: handle Q/E input → rotations + 1 mod 4; update anchor each frame from raycast; left-click → trigger Phase 11 commit; right-click/Esc → clear.

### Tests

- `plan::tests::rotate_cells_zero_rotations_is_identity`
- `plan::tests::rotate_cells_90_swaps_axes` — relative coords flip predictably.
- `plan::tests::rotate_cells_360_returns_to_origin` — four 90° rotations = identity.

### Acceptance

Tests green. Manual: ghost wireframe appears + rotates with Q/E.

---

## Phase 9 — Placement validation

### Goal

`fn can_place_plan(world, data, anchor, rotations) -> PlacementCheck` returns a ranked refusal:

```rust
pub enum PlacementCheck {
    Valid,
    NotFlat,
    VolumeNotEmpty,
    PlayerInVolume,
    ChunksNotLoaded,
    MaterialsShort,  // separate concern, checked alongside
}
```

Wireframe colour driven by the check. Specific tooltip per refusal.

### Changes

- `plan.rs`:
  - `pub fn validate_placement(world: &World, players: &[PlayerSlot], data: &PlanData, anchor: (i32, i32, i32), rotations: u8) -> PlacementCheck`.
  - Checks (in order): chunks-loaded (all required chunks are populated) → flat-ground at anchor → volume empty (every captured-cell position in world space is AIR) → no player inside.

### Tests

- `plan::tests::validate_placement_succeeds_on_empty_flat_terrain`
- `plan::tests::validate_placement_fails_on_uneven_ground`
- `plan::tests::validate_placement_fails_when_volume_occupied`
- `plan::tests::validate_placement_fails_when_player_in_volume`

### Acceptance

Tests green. Manual: red wireframe over obstructions.

---

## Phase 10 — ConstructionAnchor + resume

### Goal

When a build starts, the engine places a `CONSTRUCTION_ANCHOR` block at the build's lowest-XZ corner. The anchor's state (plan content + rotations + orientation + locked-materials snapshot + placed-cell index) lives in a parallel `World::construction_anchors: AHashMap<(i32,i32,i32), ConstructionAnchorData>` mirroring the drying-rack pattern.

Right-click the anchor with the matching plan in hand → Resume dialog (continue / abandon). Without the plan → Abandon only. The anchor is admin-protected — only the world-owner can mine it.

### Changes

- `block.rs`:
  - `CONSTRUCTION_ANCHOR: BlockId = 57`. Non-solid, transparent (it's a marker). Texture: small surveyor's flag.
- `world.rs`:
  - `pub construction_anchors: AHashMap<(i32, i32, i32), plan::ConstructionAnchorData>`.
- `plan.rs`:
  - `pub struct ConstructionAnchorData { pub plan: PlanData, pub rotations: u8, pub anchor: (i32, i32, i32), pub placed_index: usize, pub locked_materials: Vec<ItemStack> }`.
  - `pub fn order_cells(data: &PlanData, rotations: u8) -> Vec<CapturedCell>` — produces the placement order (Y-ascending, then X, then Z within each Y-layer).
- `plan_ui.rs`:
  - `pub fn show_resume_dialog(...) -> ResumeOutcome { InProgress, Continue, Abandon }`.

### Tests

- `plan::tests::order_cells_y_ascending`
- `plan::tests::order_cells_deterministic`
- `world::tests::construction_anchor_state_round_trip` — placed, retrieved, removed.

### Acceptance

Tests green.

---

## Phase 11 — Animated build

### Goal

Once the player confirms a placement, the engine:
1. Re-validates materials **in survival** (defends against inventory churn). **Skip entirely in creative.**
2. **Survival path:** locks the required materials in the anchor's `locked_materials` snapshot, decrementing them from the player's inventory. **A per-block-id total above 255 is split across multiple `ItemStack` entries** (each `count` is `u8`) — `needed as u8` truncated it (300 → 44), under-refunding large builds on cancel (engine audit 2026-06-04, A). The consume + cancel-refund loops iterate every stack, so multiple entries per block-id are transparent. Regression: `plan::lock_materials_does_not_truncate_count_above_255`.
   **Creative path:** `locked_materials` is left empty; the animated builder pulls blocks from thin air. The CAD-architect analogy from "Creative vs Survival" — design IS the deliverable; the materials don't exist.
3. Places the CONSTRUCTION_ANCHOR.
4. Each tick, places **2 blocks** from `order_cells` in order. In survival: consumes from `locked_materials`. In creative: places without any material consumption (the block to place is read directly from `order_cells[i].block_id`). Esc opens a cancel-confirm dialog; cancel keeps placed blocks + clears the anchor.
5. On completion: places the ARCHITECT_PLAQUE at the most-central tile cell (Phase 12), clears the anchor.
6. HUD overlay (bottom-centre): "🏗 Building «{name}» — N/M blocks". Reuses chat_ui/toast styling.

### Branch helper

Add `fn lock_materials_for_mode(mode: GameMode, plan: &PlanData, inv: &mut Inventory) -> Vec<ItemStack>` — returns the locked-materials snapshot. In creative this returns `vec![]` without touching the inventory; in survival it decrements and returns the snapshot. Single chokepoint; one easy place to test both branches.

### Changes

- `plan.rs`:
  - `pub fn tick_construction(world: &mut World, ...) -> Vec<BlockChange>` — advances 2 cells per call.
- `game_loop.rs`:
  - Per-tick: call `plan::tick_construction` for every active anchor.
- `hud_ui.rs`:
  - Build-progress overlay reading from `world.construction_anchors`.

### Tests

- `plan::tests::tick_construction_places_two_cells_per_tick`
- `plan::tests::tick_construction_completes_after_n_over_2_ticks`
- `plan::tests::tick_construction_consumes_locked_materials_in_survival`
- `plan::tests::tick_construction_skips_material_consumption_in_creative` — builds the same plan with an empty inventory and confirms the structure is placed in full.
- `plan::tests::lock_materials_for_mode_returns_empty_in_creative_without_touching_inventory`
- `plan::tests::lock_materials_for_mode_decrements_inventory_in_survival`
- `plan::tests::tick_construction_terminates_when_placed_index_equals_cell_count`

### Acceptance

Tests green. Manual: a 60-block build completes in ~30 ticks (1.5 s @ 20 TPS).

---

## Phase 12 — Architect's Plaque

### Goal

`ARCHITECT_PLAQUE` block (id 58). Placed last in every animated build at the most-central XZ tile cell, on the floor (Y = anchor.y + 1). Stores the plan's derivation chain in `World::architect_plaques: AHashMap<(i32,i32,i32), Vec<DerivationLink>>` **plus the root plan's `authored_in: GameMode`** so the dialog can render the mode tag without re-looking up the original Plan item.

Right-click → attribution modal showing:
- Plan name + current licence
- Full derivation chain (each link: author, plan name, licence, captured timestamp)
- **Authored in: Creative / Survival** badge near the root architect's name
- Per-architect tip-target placeholder (tipping itself ships in Spec 25)

Admin-only break.

### Changes

- `block.rs`:
  - `ARCHITECT_PLAQUE: BlockId = 58`. Solid, opaque. Parchment-on-wood texture.
- `world.rs`:
  - `pub architect_plaques: AHashMap<(i32, i32, i32), ArchitectPlaqueData>` (was `Vec<DerivationLink>`).
  - `pub struct ArchitectPlaqueData { pub chain: Vec<DerivationLink>, pub authored_in: GameMode }` — single struct so save/load + dialog have one shape.
- `plan_ui.rs`:
  - `pub fn show_plaque_dialog(...) -> PlaqueOutcome { Closed }` — read-only in v1. Renders chain + the `authored_in` badge near the root link.

### Tests

- `plan::tests::plaque_placement_position_is_central`
- `plan::tests::plaque_carries_derivation_chain`
- `plan::tests::plaque_carries_authored_in_mode` — creative-authored + survival-authored plans build plaques with the correct mode.
- `plan_ui::tests::plaque_dialog_renders_creative_badge_for_creative_plan` — pure helper that returns the badge label given an `ArchitectPlaqueData`.

### Acceptance

Tests green. Manual: every finished build has a plaque at the floor centre.

---

## Phase 13 — Save/load

### Goal

`WorldSave` gains `#[serde(default)]` `construction_anchors: Vec<SavedConstructionAnchor>` + `architect_plaques: Vec<SavedArchitectPlaque>`. Save path collects from world; load path rehydrates. Plans-in-inventory are already covered by `Item::Plan(PlanData)` + the new `SavedSlot::Plan` variant (Phase 2).

### Changes

- `save.rs`:
  - `SavedConstructionAnchor` + `SavedArchitectPlaque` structs. `SavedArchitectPlaque` carries `chain: Vec<DerivationLink>` + `#[serde(default)] authored_in: GameMode` so pre-amendment plaques (if any exist in dev/playtest saves) load with the conservative `Survival` default.
  - `WorldSave` fields + serde default.
  - Save path: flatten `world.construction_anchors` + `world.architect_plaques` into Vecs.
  - Load path: rehydrate into world.
  - `LegacyWorldSave::upgrade` — populate empty vecs.
  - `SavedSlot::Plan` carries the updated `PlanData` with `#[serde(default)] authored_in`. Pre-amendment plans in saved inventories load as Survival-authored.

### Tests

- `save::tests::construction_anchor_round_trips_through_save`
- `save::tests::architect_plaque_round_trips_through_save`
- `save::tests::plan_in_inventory_round_trips_through_save` — full Plan with derivation chain survives a save/load.
- `save::tests::pre_spec_24_save_loads_with_empty_anchors_and_plaques`

### Acceptance

Tests green. Manual: in-progress build survives quit + reload.

---

## Phase 14 — `/give` debug aliases

### Goal

`/give plan_tile`, `/give construction_anchor`, `/give architect_plaque` debug spawns. `/give debug_plan` produces a tiny test plan (3×3 footprint, 1 block of stone in each cell — useful for ghost + build tests).

### Changes

- `commands/builtins/give.rs`:
  - `plan_tile | tile` → PLAN_TILE block.
  - `construction_anchor | anchor` → CONSTRUCTION_ANCHOR block (admin debug).
  - `architect_plaque | plaque` → ARCHITECT_PLAQUE block (admin debug).
  - `debug_plan` → an Item::Plan with hardcoded test data.

### Tests

- `commands::builtins::give::tests::give_plan_tile_works`
- `commands::builtins::give::tests::give_construction_anchor_works`
- `commands::builtins::give::tests::give_architect_plaque_works`
- `commands::builtins::give::tests::give_debug_plan_works`

### Acceptance

Tests green.

---

## Phase 15 — PROTOCOL_VERSION bump

### Goal

`PROTOCOL_VERSION 16 → 17`. Stale clients reject cleanly.

### Changes

- `protocol.rs` — bump + history entry.
- `test_integration/handshake.rs` — assertion update.

### Tests

- `protocol_version_pinned`
- `protocol_version_is_the_pinned_value`

### Acceptance

check.sh ALL GREEN.

---

## Phase 16 — Spec 5 §3.13 + foundations README + vision flip

### Goal

`docs/spec/05-gameplay-systems.md` gains a §3.13 Build Schematics subsection describing the loop. `docs/foundations/README.md` flips Spec 24 to DELIVERED. `docs/vision/build-schematics-long-run.md` §10 Foundation B row flips.

### Acceptance

Drift-audit re-read finds no contradictions.

---

## Phase 17 — Axolittle playtest (BLOCKED on his time)

### What he's evaluating

- **Capture round-trip** — lay tiles, build on top, right-click empty hand → plan in inventory. Feels intuitive?
- **License picker** — does the CC-BY-SA default feel right? Does the onboarding modal land?
- **Inspect dialog** — material list + ASCII footprint readable at a glance?
- **Ghost preview** — wireframe colour reads as valid/invalid/short-materials? Q/E rotation natural?
- **Animated build** — 2 blocks/tick pace feels like "construction" rather than "magic"? Or too slow?
- **Plaque** — finishing beat lands? Right-click attribution modal readable?
- **Save-As** — modify a built plan, capture again → derivation suggestion appears?
- **Save/reload** — in-progress build survives a quit?

### Outputs

Tuning patch with adjusted constants if needed (build cadence, ghost colours, dialog layouts). Spec 5 amended for any design changes.

---

## Future spec hooks (designed-for, deferred)

### Plan Trade + Plaque tipping (Spec 25)

- Vendor Block Sell-Master / Sell-Licence sub-modes.
- Plaque tip buttons route via `apply_server_tax_and_payout`.
- License-tier infrastructure (Master vs Licence vs Derivative-Master) becomes load-bearing.

### NPC Builder (Spec 26)

- `Profession::Builder` consumes `Item::Plan(_)` + materials + sats.
- Same `tick_construction` engine, NPC-paced (1 block per 2 ticks).

### Procgen integration (Spec 27)

- Plan Registry feeds `village_gen::build_house`.
- License gating: only CC-0 + CC-BY-SA qualify.
- Plaque attribution becomes the village-quality flywheel signal.

### Slope support (vision §9, v2)

- Auto-flatten + stilt-pillar modes selectable at capture.
- Multi-level cell offsets stored on `PlanData`.

---

## Memory rule check

- ✓ signet boundary — N/A. Plans don't sign anything yet (Spec 25 v2 adds Nostr signing).
- ✓ uk english naming — "Plan", "Plan Tile", "Architect's Plaque" (possessive). "Licence" in prose; `license` in field names per cross-game-lift conformance.
- ✓ pretest check — block IDs 56+ verified available post-Papyrus (52-55). PROTOCOL_VERSION = 16 post-Spec-23. Texture layers 166+ verified available.
- ✓ autonomy to playtest boundary — Phases 2-16 autonomous, Phase 17 = playtest.
- ✓ shared infra strategy — Plan + Plaque + derivation primitives are engine-generic. Capture flood-fill algorithm is engine-generic. Game-specific: 4-licence enum, recipe shape, build cadence.
- ✓ bitcoin parent controlled — no outgoing sats in this spec (tipping ships in Spec 25).
- ✓ axenstax has farming — orthogonal, coherent with the kid-creator-economy direction.
- ✓ alpha launch posture — post-alpha-priority but useful enough to ship before alpha if cycles allow.

---

## Acceptance — overall

- 17 phases complete or explicitly blocked (Phase 17 = playtest).
- `./check.sh` ALL GREEN throughout.
- New `Item::Plan(PlanData)` variant; bincode-stable.
- `PLAN_TILE` + `CONSTRUCTION_ANCHOR` + `ARCHITECT_PLAQUE` blocks (ids 56-58).
- New `plan.rs` + `plan_ui.rs` modules.
- Full single-player capture → inspect → ghost → build → plaque loop.
- Save-As derivation with content-hash + 50%-block-match.
- Save/load preserves both Plans-in-inventory and in-progress builds.
- `PROTOCOL_VERSION` 16 → 17.
- Spec 5 §3.13 + foundations README + vision doc reference updated.
- This foundation doc's status line: READY TO BUILD → DELIVERED at the top.

---

## Out of scope (explicitly)

- Plaque tipping (Spec 25).
- Vendor Block trade modes (Spec 25).
- NPC Builder commission (Spec 26).
- Village procgen integration (Spec 27).
- Slope handling / multi-level capture (v2 per vision doc §9).
- Multi-player concurrent builds + ownership rules (deferred).
- Cryptographic Nostr-signed plans (v2).
- Player-stockable Builder NPCs (Spec 26 v2).
- 3D / isometric Inspect preview (vision §4.2 — ASCII top-down covers v1).
- Master / Licence / Derivative-Master infrastructure beyond the `is_master` flag (Spec 25).

# NPC Builder Profession — Foundation D of Build Schematics

**Status:** **PHASES 2-14 DELIVERED 2026-05-22** on `feat/spec-26-builder-commission`. Phases 2-4 shipped 2026-05-20 (`Profession::Builder` + DRAFTING_TABLE block id 65 + crafting recipe + gossip + label). **Phases 5-14 shipped 2026-05-22**: commission dialog UI (`commission_ui.rs`), Pick-build-site flow (reuses Spec 24 `GhostState`), `BuilderCommission` data struct + status machine (`builder.rs`), Builder NPC pathfind override via `process_builder_commissions` driver, animated half-rate build via `pace_divider`+`pace_counter` on `ConstructionAnchorData`, Plaque crediting via new `BuilderCredit` field on `ArchitectPlaqueData`, fee formula (base 100 + 2 sats/cell + 20 sats/premium-block, tier discount 0/10/25 %), `PayoutKind::BuilderCommission` settlement to `world.village_treasuries`, edge-case refund flow (path-timeout / cancel / player-offline pause), and a `test_integration/builder.rs` end-to-end suite. Right-click entry path: BOTH the Builder villager AND the DRAFTING_TABLE block open the dialog (Builder villager dispatches before the regular quest dialogue when a workstation is claimed). 30 unit tests + 5 commission-UI tests + 6 integration tests; full suite 1355 passing. **Phase 15 (Axolittle playtest) stays open**.
**Branch:** `feat/npc-builder-profession` off `main` (after Specs 19 + 21 + 24 + 25 all merge).
**Trigger:** Foundation **D** of the Build Schematics economy (`docs/vision/build-schematics-long-run.md`). Implements Phase 5 (NPC Builder commission) of the vision doc's lifecycle. First service-economy lane: pay a villager to build a plan for you while you're off doing something else.

---

## TL;DR

A new 6th villager profession (`Profession::Builder`) claims a new `DRAFTING_TABLE` workstation block. Right-click → commission dialog: slot a Plan, slot the required materials, pick a build site (raycast-anchored, same as Spec 24 Phase 8), pay a sats fee. The NPC pathfinds to the site, runs the existing animated-build state machine at NPC pace (1 block per 2 ticks — half the player rate so it feels like real work), and the Plaque drops at the end crediting the architect + a "Built by {villager} of {village}" Builder line. Reputation-discount on fee for higher-tier villagers (Friendly = 10% off, Beloved = 25% off).

### New shape in one paragraph

`Profession::Builder` joins the 5 existing professions (Farmer / Cook / Carpenter / Librarian / Blacksmith). `DRAFTING_TABLE` is a new block (id 61, after Spec 20's FURNACE_LIT = 60). The commission dialog mirrors the layered-painter pattern from Spec 19 villager dialogues: plan-slot picker, materials-list display (read from `plan::cell_block_counts`), site-pick (raycast cursor like the ghost preview), sats-fee input. Confirm spawns a `Commission` entity (or attaches it to the villager's component) that drives the NPC's pathfinding from workstation to site and back. The existing `tick_build` (Spec 24 Phase 11) runs at half-rate when driven by an NPC commission instead of a player.

---

## Phases summary

| # | Phase | Files | Approx LOC |
|---|-------|-------|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-20-npc-builder-profession.md` | – |
| 2 | `Profession::Builder` enum variant + `from_workstation_block(DRAFTING_TABLE) => Some(Builder)` arm. `BUILDER_POOL` quest list (3-5 Build-specific quest flavours: "Source 32 oak planks", "Source 4 iron ingots" — Builders ask players for materials, then the player can commission them later). | `villager.rs`, `quest.rs` | ~100 |
| 3 | `DRAFTING_TABLE` block (id 61). Solid + opaque + non-gravity. BlockDef + 2 procedural textures (drafting-board top + plain-wood side). Crafting recipe: 1 paper + 4 oak planks in a 2×2 shape (paper from Spec 23's PapyrusSheet via `is_paperish_slot`). | `block.rs`, `texture_gen.rs`, `crafting.rs` | ~120 |
| 4 | `villager.rs` — Builder villagers claim DRAFTING_TABLE the same way Blacksmiths claim FURNACE. Procgen village-gen places ~1 Drafting Table per 4 Carpenter houses (so most villages get one Builder slot). | `villager.rs`, `village_gen.rs` | ~80 |
| 5 | Commission dialog UI. Slot for the Plan, materials-required readout, "Pick build site" button, fee preview, Confirm button. Layered-painter pattern from Spec 19. | new `commission_ui.rs`, `game_loop.rs` (open hook on right-click DRAFTING_TABLE OR right-click a Builder villager) | ~250 |
| 6 | Pick-build-site flow. After clicking "Pick build site" the dialog closes, the player enters ghost mode (Spec 24 Phase 8) for placement preview, left-click confirms the anchor, dialog re-opens with the confirmed location. | `commission_ui.rs`, `plan.rs` (reuse `GhostState`) | ~80 |
| 7 | `BuilderCommission` data struct + per-villager component. Stores: plan, anchor, rotations, locked materials (taken from player at confirm), fee (locked at confirm), commissioning player npub, status (PathingToSite / Building / Returning / Done). | new `builder.rs` | ~120 |
| 8 | Builder NPC pathfinding. While `status == PathingToSite`, the NPC's mob AI overrides to walk toward the anchor. Reuses the existing villager-wander pathfinder. On arrival, status → Building. While Building, drives `tick_build` at half rate. On completion, status → Returning. | `mob_ai.rs`, `builder.rs` | ~150 |
| 9 | Animated build at NPC pace. `plan::tick_build` already places 2 blocks per tick (Spec 24 Phase 11). NPC commission halves this — 1 block per 2 ticks via a `pace_divider: u8` field on the construction anchor. | `plan.rs::tick_build` | ~30 |
| 10 | Plaque crediting. When the NPC's build completes, the Plaque carries both the architect (from the Plan's existing derivation chain) AND a new Builder credit line ("Built by {villager_name} of {village_name}"). | `plan.rs`, `plan_ui.rs::show_plaque_dialog` | ~60 |
| 11 | Fee calculation v1. Formula: `base_fee + per_block * cell_count + premium_block_surcharge × reputation_discount`. Defaults: base=100 sats, per_block=2 sats, premium-block list = {Diamond, Iron, IronIngot, Coal}, premium_surcharge=20 sats each. Reputation discount: Neutral=0%, Friendly=10%, Beloved=25%. | `builder.rs::fee_for_commission` | ~80 |
| 12 | Sats settlement via `apply_server_tax_and_payout` (Spec 20/21 helper). Fee debited from player; tax split + remainder routes to the village treasury (or NPC's pubkey-derived address if v2 wants per-NPC wallets — v1 = village treasury). | `builder.rs`, `economy.rs` | ~50 |
| 13 | Edge cases — refund flow if the NPC can't reach the site (lost pathfind), if the materials are stolen mid-commission (player can re-slot), if the player logs off mid-commission (build pauses + resumes when player returns). | `builder.rs::tests` | ~60 |
| 14 | Tests — full commission cycle (workstation claim, dialog round-trip, NPC pathfind, build at half-rate, Plaque with Builder credit, fee paid to village treasury). Integration test driven by `TestHost`. | `builder.rs::tests`, `test_integration/builder.rs` | ~150 |
| 15 | Axolittle playtest — full commission of a 16×16 house plan; verify pace feels right; rep-discount visible in fee preview; charter-disabled child sees read-only Builder; village treasury credit logged. | – | playtest gate |

**Total**: ~1,330 LOC across 14 build phases. Bigger than the README's ~800 estimate because the commission UI + NPC pathfinding + fee calculation each need real wiring. Phase 15 is the playtest gate.

---

## Why this lives here

- **Closes the C7 Knowledge-economy row.** Architects already trade plans (Spec 25). Now they can hire NPCs to build them. Service economy lane open.
- **Lifts Spec 19 villagers into the build-schematics loop.** Without this, Builders are a profession with no purpose. With it, villages become hubs where you can drop off your Master plan + come back to a finished tavern.
- **First per-NPC service flow.** Sets the pattern for future NPC services (paid healer, paid courier, paid scout). The commission-dialog + pathfind + return + payout shape generalises.
- **Cross-game lift.** Profession-bound workstation + commission flow + reputation-discount fee formula all lift cross-game.

---

## Creative vs Survival

- **Creative**: fees can be skipped (per-server policy toggle), but pace is the same (1 block per 2 ticks). Creative architects who want to *test* their build at NPC pace skip the materials-lock too.
- **Survival**: materials are locked from the player's inventory at commission-confirm time. If the player doesn't have the materials, the commission can't be confirmed.

---

## Context pointers

### Existing code surfaces this touches

- `villager.rs` — new `Profession::Builder` variant; `from_workstation_block` arm; quest pool addition.
- `village_gen.rs` — drafting-table placement during village procgen.
- `block.rs` — DRAFTING_TABLE block id + BlockDef.
- `texture_gen.rs` — 2 new textures.
- `crafting.rs` — DRAFTING_TABLE recipe.
- `plan.rs` — `pace_divider: u8` field on `ConstructionAnchorData`; `tick_build` reads it.
- `plan_ui.rs::show_plaque_dialog` — Builder credit line.
- `mob_ai.rs` — Builder pathfind override.
- `economy.rs` — `apply_server_tax_and_payout` already exists by the time this builds.

### New modules

- `commission_ui.rs` (~250 LOC) — Commission dialog egui surface. Pattern mirrors `villager_ui.rs`.
- `builder.rs` (~250 LOC) — `BuilderCommission` data + fee formula + pathfind hooks + status machine.

### Related specs

- `docs/foundations/2026-05-18-villages-and-villagers.md` (Spec 19) — provides villagers + professions + dialogue framework.
- `docs/foundations/2026-05-18-vendor-block.md` (Spec 21) — provides Vendor Block (and the sats helper).
- `docs/foundations/2026-05-19-build-schematics-core.md` (Spec 24) — provides Plan, ConstructionAnchor, animated build, Plaque.
- `docs/foundations/2026-05-20-plan-trade-plaque-tipping.md` (Spec 25) — provides Plan trade. The NPC Builder needs a Plan in inventory to commission; that Plan often came from a Vendor Block trade.
- `docs/vision/build-schematics-long-run.md` §7 (Phase 5 — NPC Builder) — design contract.

### Memory pointers

- bitcoin parent controlled — Charter gate on the commission fee.
- shared infra strategy — commission + pathfind primitive lifts cross-game.
- uk english naming — Builder (capital B) for the profession; "drafting table" lowercase except in block-name strings.

### What does NOT exist yet (deferred to v2)

- **Per-NPC wallets.** Villagers don't have their own LN addresses; fees go to village treasury. v2 may add per-NPC pubkey-derived wallets.
- **Multi-NPC commissions.** A single villager handles one commission at a time. No parallel builds per Builder. v2.
- **Player-spectator-tip on a Builder's work.** Out of scope; Builder credit is the architect-attribution surface; tipping during the build is post-alpha.
- **Refurbishment / repairs.** A Builder can only build new structures, not repair existing. v2.

---

## Phase 11 — Fee calculation v1 (the formula)

```
fee = base_fee
    + per_block * cell_count
    + premium_surcharge * (count of premium blocks in plan)
fee *= reputation_discount   // 1.0 / 0.9 / 0.75 by tier
```

Premium blocks: `Diamond`, `IronIngot`, `IronBlock`, `DiamondBlock`, `CoalBlock`, `Satori`, `SatoriBlock`. Crafted-material premiums (e.g. baked goods) are NOT premium for v1 — they're easy to source.

Tunable in playtest. The defaults (100 / 2 / 20 / 0%/10%/25%) are starting points; Axolittle playtest tells us whether the kid's willing to pay them.

---

## Phase 14 — Integration test outline

```rust
let mut host = TestHost::new();
// Spawn a village with one Builder villager.
host.world.spawn_village_for_test();
let builder = host.spawn_builder_villager_at_drafting_table();

// Player has a plan + materials.
host.give_player_plan(plan);
host.give_player_materials(materials);

// Right-click the Builder, open commission dialog (synthetic).
let commission = host.confirm_commission(builder, plan, anchor, fee);

// Tick forward until Builder reaches the site.
host.tick_until(|h| h.builder_status(builder) == Building);

// Tick forward until the build completes.
let total_ticks = plan.cells.len() * 2 * 2;  // 2 blocks/tick × pace_divider 2
host.tick(total_ticks + 10);
assert_eq!(host.builder_status(builder), Returning);

// Verify the Plaque has the Builder credit.
let plaque = host.find_plaque_at(anchor);
assert!(plaque.builder_credit.is_some());

// Verify fee debit.
assert_eq!(host.player_sats_balance(), starting_balance - fee);
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- Integration test above passes.
- Manual playtest (Phase 15): commission a real house plan; verify pace + fee + Plaque credit + village treasury logging.


# Plan Trade + Plaque Tipping — Foundation C of Build Schematics

**Status:** **DELIVERED 2026-05-20 + 2026-05-22**. **Phase 7 (Plaque tipping) shipped 2026-05-20** on `feat/plan-trade-tipping` — `PlaqueDialogOutcome::TipLink(usize)` + per-architect Tip buttons + game-loop handler routing through `economy::apply_sats_payout(_, PayoutKind::PlaqueTip, _, _)` with Charter + per-server gating. New `PayoutKind::PlaqueTip` variant. **Phases 2-4 + 6 (Vendor Block plan-listing sub-modes) DELIVERED 2026-05-22** on `feat/spec-25-vendor-plan-modes`: two new `VendorMode` variants (`SellPlanMaster` / `SellPlanLicence`), `UNLIMITED_STOCK = u32::MAX` marker, pure helpers (`slot_accepts_for_mode`, `plan_listing_allowed`, `try_buy_plan`), owner-side mode-picker + Plan-only slot filter + Licence stock counter (+/- + Unlimited toggle), Master-tier listing refusal for Licence-tier plans, buyer-side render with gold/silver tier tint + ASCII footprint + material summary + author credit + tier-aware Buy button + Charter-disabled tooltip. Master sale = stock 1→0 + listing locks; Licence = decrement per sale (or never with Unlimited). Settlement routes through `economy::apply_sats_payout` with `PayoutKind::VendorSale` (spec-mandated reuse — no new kind). +14 new vendor tests. **Phase 5 (Licence-tier derive guard)** stays dormant — Spec 24's `detect_parent` already skips non-Master plans (the guard is dormant). Phase 9 = Axolittle playtest gate.
**Branch:** `feat/plan-trade-plaque-tipping` off `main` (after Specs 21 + 24 merge).
**Trigger:** Foundation **C** of the Build Schematics economy (`docs/vision/build-schematics-long-run.md`). Implements Phase 3 (Trade) of the vision doc's lifecycle. Adds the first sats-flow path for plans: selling Master vs Licence copies through Vendor Blocks, plus per-architect tipping via the Plaque attribution dialog.

---

## TL;DR

Two new sub-modes on the Vendor Block (`Sell Master` + `Sell Licence`) let plan-owners price and list their work; buyers slot sats and walk away with the right tier of Plan item. The Architect's Plaque (Spec 24 §12) gains a tip button per credited architect — derivation-chain tipping that flows through `apply_server_tax_and_payout` (Spec 21's unified sats helper). Per-licence enforcement gates Save-As (derivative capture) at the slot — Licence holders can't derive; only Masters can. Charter flag gates outgoing sats (per bitcoin parent controlled); on a guardian-disabled child or a per-server policy block, the trade UI is read-only.

### New shape in one paragraph

Spec 21's Vendor Block grows two extra mode-enum variants: `SellPlanMaster(PlanData, price)` and `SellPlanLicence(PlanData, price)`. The owner-side dialog adds a "List a plan…" branch that takes a Plan from inventory + lets the owner set a price and tier. The buyer-side dialog renders the listing with the plan's name + licence + price + an inline Inspect-style ASCII top-down. On purchase the listing decrements (or stays infinite for Master, configurable), the buyer receives the right Plan tier (Master keeps full chain access; Licence is read-only + Save-As-blocked), and sats flow through `apply_server_tax_and_payout` (Spec 20 Phase 10 / Spec 21 sats helper). The Plaque dialog (Spec 24 §12) becomes interactive: each architect in the derivation chain gets a "Tip" button that resolves their `lud16` (LNURL) at click time + spawns a sats payment subject to the same Charter + tax flow.

---

## Phases summary

| # | Phase | Files | Approx LOC |
|---|-------|-------|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-20-plan-trade-plaque-tipping.md` | – |
| 2 | `VendorMode::{SellPlanMaster, SellPlanLicence}` variants on Spec 21's vendor-block-data enum. Discriminator field for "Master" vs "Licence" at storage time so the UI can render distinct columns. | `vendor.rs` (Spec 21), `vendor_ui.rs` (Spec 21) | ~150 |
| 3 | Plan listing UI on Vendor Block owner-side. "List a plan" branch: takes a Plan from inventory, prompts for price + tier (Master tier is one-time + price-locked at listing; Licence is per-copy + can have a Buy-N counter). Confirmation writes the listing to `vendor.listing`. | `vendor_ui.rs`, `inventory.rs` (slot pickers) | ~150 |
| 4 | Plan listing UI on Vendor Block buyer-side. Renders the listing with Plan name + licence label + price + inline ASCII top-down (reuses `plan_ui::ascii_footprint`). "Buy" button confirms purchase + decrements (Licence) OR locks (Master, single sale). | `vendor_ui.rs` | ~120 |
| 5 | Licence-tier enforcement — Save-As guard. `plan::detect_parent` already skips non-Master plans (Spec 24 Phase 6). Add `Item::Plan` discrimination at the capture-dialog level so the "Mark as derivative" toast surfaces an explicit "Licence-tier plans can't be derivative parents" message instead of just silently not offering the checkbox. | `plan.rs`, `plan_ui.rs` | ~50 |
| 6 | Sats settlement via `apply_server_tax_and_payout`. Each purchase routes through the same hook Spec 21 introduces (or the Spec-20-deferred-to-Spec-21 hook); Charter `charter_allows_sats` gate; per-server-policy gate; tax-and-payout split. | `vendor.rs`, `economy.rs` (new helper if not from Spec 20) | ~80 |
| 7 | Plaque tipping UI — derivation-chain display becomes interactive. Each `DerivationLink` row in the Plaque dialog (Spec 24 §12) gains a "Tip" button. Click → resolves `author_lud16` → triggers `apply_server_tax_and_payout` with the architect's pubkey as destination. | `plan_ui.rs::show_plaque_dialog` | ~120 |
| 8 | Tests + Acceptance — listing round-trip (owner places, buyer purchases, item received + sats accounted), Licence-derive refusal, Plaque-tip CTA flow. Mock Lightning settlement (BRIDGE until real LN) so the unit tests can assert tax-and-payout splits without touching a wallet. | `vendor.rs::tests`, `plan.rs::tests`, `plan_ui.rs::tests` | ~120 |
| 9 | Axolittle playtest — guardian-allowed-sats child buys a plan; guardian-disabled child sees read-only listings + the Charter-deny overlay on the Buy button; tip from the Plaque flows to the architect's lud16 (mocked). | – | playtest gate |

**Total**: ~790 LOC across 8 build phases (Phase 9 = playtest). Targeted estimate from `docs/foundations/README.md` was ~500; the spec grew a bit on Phase 2/3/4 because the Vendor Block sub-mode shape needs more wiring than a single new arm.

---

## Why this lives here

- **First player-to-player sats path in the engine.** Until this lands, the only sats flows are server-tax-and-payout from village-quest payouts (Spec 19) — admin-driven, not player-economy. This spec opens the markets-economy lane (`docs/vision/economies-long-run.md`).
- **Tests the unified sats helper.** Either Spec 20 Phase 10 ships it or Spec 21 does; whichever lands first, Spec 25 is the first spec needing it for **two distinct flows** (purchase + tip). The helper's shape solidifies under load here.
- **Cross-game lift.** Architect-tipping via lud16 + per-server-policy gating + Charter guarding lifts cross-game cleanly. The Plaque-specific dressing stays AxeNStax-side; the helper API and the policy chain go cross-game.

---

## Creative vs Survival

Plans listed for sale work identically in both modes. The `authored_in: GameMode` tag (Spec 24) is rendered in the listing card so survival-purist servers can implement buyer-side filters via a future per-server-policy rule — out of scope for this spec.

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/vendor.rs` (Spec 21 — to be merged first) — `VendorMode` enum gains two new variants. Discriminator field stored on disk.
- `game/engine/src/vendor_ui.rs` (Spec 21) — owner + buyer dialogs gain plan-listing branches. Reuse Spec 24's `ascii_footprint` + `material_summary` helpers for the listing preview.
- `game/engine/src/plan.rs` — Licence-tier guard at capture-dialog level. The `is_master: bool` field exists since Spec 24 Phase 6; this spec actually *gates* it (currently dormant).
- `game/engine/src/plan_ui.rs::show_plaque_dialog` — Plaque dialog becomes interactive. Each row in the derivation-chain display gains a "Tip" button.
- `game/engine/src/economy.rs` (or wherever Spec 20/21 puts it) — `apply_server_tax_and_payout` becomes the universal sats settlement helper.

### New modules

None. All work lives in existing files.

### Related specs

- `docs/foundations/2026-05-18-vendor-block.md` (Spec 21) — provides Vendor Block + base settlement hook. **Must merge first.**
- `docs/foundations/2026-05-19-build-schematics-core.md` (Spec 24) — provides Plan item + Plaque + derivation chain. **Already on `main`.**
- `docs/foundations/2026-05-18-furnace.md` (Spec 20) — first spec planning the unified sats helper; defers to Spec 21 for the actual landing. By the time Spec 25 builds, the helper exists.
- `docs/vision/build-schematics-long-run.md` §5 (Phase 3 — Trade) — design contract.

### Memory pointers

- uk english naming — "Licence" (noun) / "license" (verb); field names use `license` per Rust convention.
- bitcoin parent controlled — outgoing sats is gated. Tips + purchases both honour `PlayerSlot.charter_allows_sats` + per-server policy.
- shared infra strategy — tipping primitive + lud16 resolution lifts cross-game.
- proof of play is proof of work — orthogonal. No mining hash on plan trade.
- pretest check — confirm Spec 21 + Spec 24 are both on `main` before starting this spec.

### What does NOT exist yet (deferred to v2)

- **Architect royalty on resales.** If a Licence-tier buyer re-sells the Licence on the secondary market, the original architect doesn't see a cut. v2 may add a per-resale split.
- **Cross-server portability.** A Plan bought on server A is freely usable on server B — there's no DRM. Vision doc §5.8 says this is intentional for v1.
- **Auction-style listings.** Vendor Block listings are fixed-price. Bidding is post-alpha.
- **Bulk Licence discount.** Buy-5-get-1-free style. Not in v1.

---

## Phase 2 — VendorMode variants

The Vendor Block (Spec 21) carries a `VendorMode` enum: `Sell(ItemStack, price)`, `Buy(ItemStack, price)`, `Barter(...)`. This spec adds:

```rust
pub enum VendorMode {
    Sell(ItemStack, u32),
    Buy(ItemStack, u32),
    Barter(...),
    // Spec 25 — plan listings.
    SellPlanMaster(PlanData, u32),
    SellPlanLicence(PlanData, u32),
}
```

The Master variant is one-time (single sale, then the listing disappears). The Licence variant is per-copy (each purchase decrements a counter; when 0, the listing disappears OR refreshes to a new listing if the owner has Buy-N enabled).

**Save format**: the `VendorMode` enum already serialises via bincode. Two new variants → wire-version bump within Spec 21's save shape.

### Tests

- `vendor::tests::sell_plan_master_listing_round_trips_via_bincode`
- `vendor::tests::sell_plan_licence_decrements_on_purchase`

---

## Phase 3 — Plan listing (owner side)

Vendor Block owner dialog (Spec 21) gets a new branch: "List a plan…". Clicking it opens a sub-dialog that:

1. Lets the owner pick a Plan from their inventory (filtered to `Item::Plan(_)` only).
2. Prompts for tier (radio: Master / Licence).
3. Prompts for price (numeric input — sats).
4. If Licence tier, prompts for copy count (1 / 5 / 10 / Unlimited).
5. Confirm → writes the listing to the Vendor's storage.

### Tests

- `vendor_ui::tests::plan_listing_dialog_round_trip_master`
- `vendor_ui::tests::plan_listing_dialog_blocks_unlicensed_licence_listing` — if the owner only holds a Licence (not Master), refuse the listing.

---

## Phase 4 — Plan listing (buyer side)

Vendor Block buyer dialog renders the active listing with:

- Plan name + licence label
- Tier (Master / Licence) — distinct visual treatment (e.g., gold border for Master)
- ASCII footprint preview (`plan_ui::ascii_footprint`)
- Material count summary (`plan_ui::material_summary`)
- Price + Buy button
- Charter-disabled state: Buy button greyed + tooltip "Bitcoin disabled for this account"

### Tests

- `vendor_ui::tests::plan_listing_buyer_view_charter_disabled_greys_buy`
- `vendor_ui::tests::plan_listing_buyer_view_master_distinct_from_licence`

---

## Phase 5 — Licence-tier derive guard

Spec 24 Phase 6 added `is_master: bool` to PlanData. `detect_parent` already skips non-Master plans. This spec:

1. Capture dialog surfaces an explicit toast when no parent was detected because the candidate matches a Licence-tier plan in inventory.
2. The capture flow itself is unchanged (any captured plan is still a Master in v1; the future-spec hook is the toast wording).

### Tests

- `plan::tests::detect_parent_with_only_licence_returns_none_with_reason`

---

## Phase 6 — Sats settlement

`apply_server_tax_and_payout(amount, kind, dest, server_policy, charter_flag)` from Spec 20 Phase 10 / Spec 21 handles:

- Charter flag check (deny if disabled).
- Server-policy check (deny if per-server policy forbids).
- Tax + payout split (server takes its cut; remainder goes to architect).

For purchases: the buyer's wallet is debited via the helper; the seller's lud16 is the destination. For tips: same shape — tipper's wallet debited; tipped architect's lud16 is destination.

Mocked Lightning settlement (BRIDGE) for alpha; replace when LN integration lands.

### Tests

- `vendor::tests::purchase_routes_through_sats_helper`
- `vendor::tests::purchase_with_charter_off_does_not_settle`
- `vendor::tests::purchase_with_server_policy_off_does_not_settle`

---

## Phase 7 — Plaque tipping

Spec 24's `show_plaque_dialog` renders the derivation chain as a vertical list of `DerivationLink` entries. This spec adds a per-row "Tip" button. Click:

1. Resolve `author_lud16` (stored on the DerivationLink — Spec 24 reserves the field).
2. Prompt for sats amount (default: 100, configurable).
3. Confirm → route through `apply_server_tax_and_payout`.
4. Toast: "Tip sent to {architect_name}." (or Charter-deny / server-deny variant).

### Tests

- `plan_ui::tests::plaque_dialog_tip_button_renders_per_link`
- `plan_ui::tests::plaque_dialog_tip_charter_off_greyed`

---

## Phase 8 — Test plan

- All Phase 2-7 tests above run in `cargo test --bin axenstax-engine`.
- Integration test: list a Master, buy it from another player slot, confirm sats balance change + Plan item delivered.
- Integration test: tip an architect from a Plaque attached to a placed build.

---

## Phase 9 — Axolittle playtest (BLOCKED on his time)

Solo verification can confirm the wire works; the playtest gate is what tells us whether **the UX of plan trade + tipping feels right to a kid**. Specific things to watch:

- Is the Master/Licence distinction visible enough on the listing card?
- Does the tip-from-Plaque button feel like a natural CTA or buried?
- Does the Charter-disabled state confuse the kid or feel respectful?
- Is the ASCII top-down preview enough, or should the buyer-side render a wireframe?

---

## Acceptance

- `./check.sh` ALL GREEN.
- All Phase 2-7 tests pass.
- Manual integration: list → buy → use → tip flow works end-to-end on a single dev box (with two player slots).
- Charter-disabled child sees read-only buy + tip surface.

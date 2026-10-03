# Bulk / Wholesale Vendor mode (v1)

**Status:** READY TO BUILD 2026-05-23 (spec only — build has a refactor prerequisite, see below). Phase N = Axolittle playtest gate.
**Branch (when built):** `feat/bulk-vendor` off `main`.
**Trigger:** Goal-driven solo dev cycle, Round 10. From `docs/vision/economies-long-run.md` §4.4 — "Bulk / Wholesale (T4)". A high-volume Vendor variant: lower per-unit price, larger minimum buy. Written spec-only because the build has a clean prerequisite (extract the inline vendor-buy logic into a pure `vendor::try_buy` helper) that's best done as its own first phase rather than tangled into a feature at the tail of a long session.

---

## TL;DR

Add a **`VendorMode::Bulk`** to the existing Vendor Block (Spec 21). A bulk vendor sells its stocked item in fixed-size lots (e.g. 64) at a reduced per-unit price, with a configurable **minimum buy quantity**. The buyer purchases a whole lot at once; you can't cherry-pick a single unit from a bulk vendor.

- Owner config: pick Bulk mode → set `lot_size` (e.g. 16 / 32 / 64) + per-unit price (typically below the item's `trade_value` ×1, since bulk trades volume for margin).
- Buyer: sees "Buy 64 × Cobblestone for N sats" — one click moves the whole lot + sats.
- **Anti-monopoly hook** (vision): market-hub operators can tax-break bulk vendors; deferred to when hub taxes exist (v2).

This is **surgery on the Vendor Block**, not a new block. The new surface: a `VendorMode::Bulk` enum variant (positional append), a `lot_size: u32` field on `VendorData`, and the buy path honouring lot-size + the per-unit discount.

PROTOCOL_VERSION bump (new VendorMode variant + VendorData field = wire change). **Estimated ~500 LOC including the refactor prerequisite + tests.**

---

## Build prerequisite (Phase 2 — do this first)

The Vendor **buy logic currently lives inline** in `game_loop.rs` (the `open_vendor` dialog branch, Sell-mode `BuyerBuy` outcome — roughly the lines the Round-8 audit walked at `game_loop.rs:5889-5972`). There is **no `vendor::try_buy` pure helper** (only `try_buy_plan` for the Spec-25 plan modes).

Bulk mode adds a second "buy a quantity at a price" path. Rather than copy-paste the inline logic, **Phase 2 extracts the existing Sell-mode buy into a pure `vendor::try_buy(data, buyer_inventory, …) -> BuyOutcome` helper** (mirroring `try_buy_plan`'s shape), with the game_loop branch calling it. This:
- de-risks Bulk (it becomes a thin variant of a tested helper, not new inline logic),
- makes the buy path unit-testable (it currently isn't),
- is a clean, behaviour-preserving refactor that should land + be verified green *before* any Bulk behaviour is added.

**This is why the build is queued rather than done same-session**: the refactor wants a clean, fresh, behaviour-preserving pass with its own review, not to be entangled with new-feature work at the end of a 9-feature session.

---

## What v1 ships (after the prerequisite)

1. **`vendor::try_buy` pure helper** (Phase 2 refactor) — extracts the inline Sell-mode buy. Behaviour-preserving; game_loop calls it. Unit-tested.
2. **`VendorMode::Bulk`** — positional enum append after `SellPlanLicence`.
3. **`VendorData.lot_size: u32`** — `#[serde(default)]`; 1 for non-bulk modes, the lot size for Bulk.
4. **Buy path honours lot-size** — a bulk buy transfers `lot_size` units (or refuses if stock < lot_size) for `lot_size × per_unit_price`. Per-unit price is the owner's setting (the "discount" is just the owner pricing below singles — no separate discount math needed; the UI can *suggest* a sub-trade-value price).
5. **`vendor_ui.rs`** — Bulk added to the owner mode picker; the buyer view shows "Buy {lot_size} × {item} for {total} sats".
6. Tests: `try_buy` refactor unit tests (the behaviour it preserves) + Bulk-specific (lot-size transfer, stock < lot_size refusal, total-price math). Integration: configure-bulk → buy-lot → inventory + escrow correct.

---

## Out of scope (v2)
- **Market-hub tax breaks for bulk vendors** — needs hub taxes (don't exist).
- **Tiered bulk pricing** (bigger lot = bigger discount) — v1 = flat per-unit.
- **Bulk Buy mode** (vendor *buys* bulk from players) — v1 = bulk Sell only.

---

## Known limitations (BRIDGEs)
1. Notional sats on alpha (every economy block).
2. Reuses `VendorOwner::LocalPlayer` — the owner-model convergence ([[project_economy_block_owner_convergence]]) applies; no new owner debt (it's the existing Vendor owner).

---

## Implementation phases
| # | Phase | Note |
|---|---|---|
| 1 | Spec + plan | this doc |
| 2 | **Refactor: extract `vendor::try_buy`** | behaviour-preserving; land + verify green BEFORE Bulk |
| 3 | `VendorMode::Bulk` + `VendorData.lot_size` + PROTOCOL bump | enum/field append |
| 4 | Buy path honours lot-size in `try_buy` | the Bulk behaviour |
| 5 | `vendor_ui.rs` Bulk mode (owner picker + buyer lot view) | |
| 6 | Tests (try_buy unit + Bulk + integration) | |
| 7 | check.sh + doc-flip + README | |
| 8 | Merge | |
| N | Axolittle playtest — lot sizes, bulk pricing feel | |

---

## Acceptance criteria (solo)
- The `vendor::try_buy` refactor lands behaviour-preserving (existing Sell-mode buy unchanged), green.
- A vendor set to Bulk sells its item in `lot_size` lots; a buy moves the whole lot for the total price.
- Stock < lot_size → buy refused.
- Save/reload round-trips `lot_size` + Bulk mode.
- `check.sh` ALL GREEN.

---

## Memory-rule check
- ✓ economies vision — markets §4.4.
- ✓ bitcoin parent controlled — buy via the existing Charter-gated payout path.
- ✓ proof of play is proof of work — deterministic bulk pricing, not chance.
- ✓ uk english naming — "Bulk" / "Wholesale" / "lot".
- ✓ economy block owner convergence — reuses the existing Vendor owner; no new debt.
- ✓ autonomy to playtest boundary — solo through Phase 7.

---

## Cross-spec interactions
- **Vendor Block (Spec 21)** — Bulk is a new mode on it; the `try_buy` extraction also benefits the existing Sell/Buy/Barter/Plan modes (unit-testable).
- **Server Bazaar (Spec 39)** — bulk vendors price *above* the Bazaar floor but *below* singles; the two coexist as the volume vs convenience trade-off.
- **Construction commissions** — the vision notes bulk vendors feed construction material demand; no code dependency, just the use-case.

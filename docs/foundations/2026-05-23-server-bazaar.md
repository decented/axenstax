# Server Bazaar (v1) — the trade-value sell-floor

**Status:** Phases 2-9 DELIVERED 2026-05-23 on `feat/server-bazaar`. Phase N = Axolittle playtest gate (open).
**Branch:** `feat/server-bazaar` off `main`.
**Trigger:** Goal-driven solo dev cycle, Round 9. From `docs/vision/economies-long-run.md` §4.6 — "Server Bazaar / NPC Merchants". The market-of-last-resort: gives **every** item guaranteed liquidity at its `trade_value` floor, so nothing is ever truly worthless. The cleanest remaining new self-contained block — server-run (no owner), stateless (no block-entity), reuses the shipped `trade_value` ladder + `apply_sats_payout` pipeline.

---

## TL;DR

A new **Bazaar Block** (id 130). Right-click → a sell dialog: the player's held stack shows its floor value (`item.trade_value() × count`) and a **Sell** button consumes the stack and pays out that many sats via `economy::apply_sats_payout(_, PayoutKind::BazaarSale, _, _)`. Server-run — **no owner, no escrow, no block-entity, no save state** (it computes the floor on the fly from the item). Anyone can place/use/break it (it protects nothing).

This is the economy's **price floor**: a player can always offload surplus to the Bazaar at trade-value, even when no human vendor is buying. It deliberately pays *only the floor* (not the ×1.5 human-market premium), so player-to-player markets (Vendor / Auction) stay the better deal for anything in demand — the Bazaar catches the long tail.

**v1 is sell-only.** The buy-side (Bazaar sells to players at trade-value × 1.5, "catches items no human's selling") needs a catalogue browser UI; deferred to v2.

1 new BlockId (130), 1 new MaterialId (`BazaarBlockItem`), 1 new `PayoutKind::BazaarSale`, 1 new module (`bazaar.rs` — a pure `sell_quote` helper), 1 new UI (`bazaar_ui.rs` — the sell dialog), 1 recipe, 1 texture. No `World`/`WorldSave` changes (stateless). PROTOCOL_VERSION 36 → 37. **Estimated ~450 LOC including tests** — the smallest economy block since it has no per-block state.

---

## Why this lives here

- **Liquidity floor makes the whole economy feel alive.** Without an always-buyer, surplus items (a chest of cobblestone, spare seeds) are worthless until a human happens to want them. The Bazaar means *every* item has a baseline worth — which is what makes the `trade_value` ladder (shipped in T1.5) actually matter to a kid.
- **Anchors price discovery.** Vendor + Auction prices now have a floor to sit above: a player won't sell to a human below trade-value when the Bazaar guarantees it. Makes the markets legible.
- **No owner-model BRIDGE** — server-run, so it sidesteps the `LocalPlayer(pidx)` reload edge case that every player-owned economy block carries ([[project_economy_block_owner_convergence]]). The simplest economy block to get right.
- **Cross-game lifts** — a trade-value sell-floor block is engine-generic.

---

## What v1 ships

### 1. `BAZAAR_BLOCK` (id 130)
Stateless, no owner. Anyone places/breaks/uses it. Recipe: a distinct shape — e.g. emerald-less "trading post" of planks + iron + (a gold-ish accent); verify no collision (recipe space is dense — likely a plank-ring around a distinct centre, or a 3-tall column). Mining drops itself.

### 2. `bazaar.rs` — pure helpers
```rust
/// What the Bazaar pays for a stack: the item's trade_value × count.
/// None if the item isn't tradeable (AIR/WATER) or the stack is empty.
pub fn sell_quote(stack: &ItemStack) -> Option<u64>;
```
That's the whole module — the Bazaar has no state, so there's no tick driver, no settle, no escrow. (A `buy_price` helper — `trade_value × 3/2` — is the v2 laydown but not wired in v1.)

### 3. `PayoutKind::BazaarSale`
Append after `RepairTax`. Label `"bazaar sale"`. The player *receives* the floor sats (a payout TO the player, unlike RepairTax which debits).

### 4. `bazaar_ui.rs` — sell dialog
Right-click the Bazaar → a panel:
- Header: *"Bazaar — sell at the trade-value floor"*.
- The held stack: name + count + *"Sells for N sats"* (the quote).
- A **Sell** button (enabled when the held stack is tradeable + Charter/Bitcoin allow + no Vow). On click → consume the stack, fire the payout, toast.
- Charter-off / Bitcoin-disabled: read-only message (no rep fallback — the Bazaar is a sats venue; barter has no meaning against a server).
- Esc / 'E' closes.

### 5. Wiring (`game_loop.rs`)
- Right-click `BAZAAR_BLOCK` → `player.open_bazaar = Some(pos)`.
- Per-frame dialog branch (alongside the others). On Sell: read the held stack's `sell_quote`, gate on Charter + Vow, `apply_sats_payout(quote, PayoutKind::BazaarSale, …)`, consume the held stack (`take_slot`), toast `"Sold for N sats"`.
- No place-time stamp (stateless), no break special-casing (nothing to clean up).

### 6. `PlayerSlot.open_bazaar` field. `/give` aliases (`bazaar`, `merchant`). PROTOCOL 36 → 37 + handshake.

### 7. Tests
Pure (`bazaar.rs`): `sell_quote` = trade_value × count; None for AIR; scales with count; None for a 0-count stack. Plus a sanity test that a sample of materials all return Some (every real item has a floor). Integration (`test_integration/bazaar.rs`): selling consumes the stack + the quote matches trade_value × count; AIR/empty refused. (No save round-trip — stateless.)

---

## Out of scope (v2)
- **Buy-side** (Bazaar sells to players at trade_value × 1.5) — needs a catalogue browser UI. The `buy_price` helper is a v2 laydown.
- **Per-server price overrides** — `trade_value` already notes server operators override the map (Spec 6 §10.3); the Bazaar reads whatever `trade_value` returns, so this lands free when the override map ships.
- **Bulk discounts / wholesale** — that's the separate Bulk Vendor spec (§4.4).
- **Sell confirmation for high-value stacks** — v1 sells immediately; a "are you sure" guard for Satori-tier items is a v2 nicety.

---

## Known limitations (BRIDGEs)
1. **Notional sats on alpha** — `apply_sats_payout` is accounting-only (no real wallet). The player doesn't accrue a real balance; the payout is logged. Same BRIDGE as every economy block.
2. **No anti-dupe on the sell** — a player selling a stack gets floor sats; there's no per-item cooldown or anti-farming. On alpha (notional sats) this is moot; when a real wallet lands, the server-economy reserve drain in `apply_sats_payout` is the throttle (the Bazaar pays from the server's pool, which is finite). Documented.

---

## Implementation phases (per-phase green)
| # | Phase | Est LOC |
|---|---|---|
| 1 | Spec + plan | 0 |
| 2 | BlockId + texture + recipe + MaterialId + /give + placeable mapping | ~120 |
| 3 | `bazaar.rs` (sell_quote) + tests | ~80 |
| 4 | `PayoutKind::BazaarSale` + label | ~10 |
| 5 | `PlayerSlot.open_bazaar` + right-click open | ~20 |
| 6 | `bazaar_ui.rs` + dialog branch + sell handler | ~180 |
| 7 | PROTOCOL 36 → 37 + handshake | ~10 |
| 8 | Integration tests | ~80 |
| 9 | check.sh + doc-flip + README | 0 |
| 10 | Merge | — |
| N | Axolittle playtest — does the floor feel right, is sell-only enough, is the held-stack sell intuitive |

**Phases 2–9 solo-buildable.** Phase N = playtest.

---

## Acceptance criteria (solo)
- BAZAAR_BLOCK craftable/placeable/mineable (no owner gating).
- Right-click → dialog showing the held stack's floor value.
- Sell consumes the stack + pays `trade_value × count` via `PayoutKind::BazaarSale`.
- Untradeable held item (or empty hand) → Sell disabled.
- Bitcoin-disabled / Charter-off / Vow → read-only.
- No save state (stateless) — save/reload unaffected beyond the block id.
- `check.sh` ALL GREEN. PROTOCOL 37.

---

## Memory-rule check
- ✓ economies vision — markets §4.6.
- ✓ bitcoin parent controlled — payout via `apply_sats_payout` with Charter + Vow gating.
- ✓ proof of play is proof of work — selling at a deterministic trade-value floor is not chance-based. No gambling.
- ✓ uk english naming — "Bazaar" (UK-natural marketplace term).
- ✓ alpha open access — no whitelist.
- ✓ shared infra strategy — trade-value sell-floor block is engine-generic.
- ✓ autonomy to playtest boundary — solo through Phase 9.
- ✓ economy block owner convergence — N/A (server-run, no owner) — notably the first economy block that DOESN'T join the convergence list.

---

## Cross-spec interactions
- **Vendor Block / Auction** — the Bazaar floor sits *below* human-market prices (it pays trade-value; humans pay more for demand). No code interaction; it's a pricing anchor.
- **trade_value (T1.5)** — the Bazaar is the first consumer that makes `trade_value` directly matter to the player (sell price). Validates the ladder.
- **Proof-of-Play / Reserve** — when real sats land, the Bazaar pays from the server pool; `apply_sats_payout`'s reserve-drain is the sustainability throttle.
- **Market Hubs** — a v2 thought: list the Bazaar in the hub directory as an always-available "merchant". Out of scope for v1.

# Vendor Block — placeable player-run shop primitive

**Status:** **DELIVERED 2026-05-20** on `feat/vendor-block`. Phases 2-9 + Phase 10 (docs) shipped solo. Phases 11-14 (multi-slot conversion, reputation gating + trade-value floors, stale-shop expiry, raid-supplies highlight) **DEFERRED** post-playtest — the meta-economy review added them after the original 9-phase scope; the v1 single-slot shape with Charter gating + Buy/Sell/Barter modes is enough to validate the markets-economy lane. Phase 15 = Axolittle playtest gate.

## TL;DR

A craftable, placeable block that lets one player run a fixed-price shop for other players. Three modes: **Sell** (block stocks items, charges sats), **Buy** (block holds sats, takes items in exchange), and **Barter** (item-for-item, sat-free — works on no-Bitcoin servers). Owner controls stock and price; non-owners can interact through a dialogue UI that mirrors the Spec 19 villager pattern.

This is the first concrete piece of the **Markets** economy in `docs/vision/economies-long-run.md` §4.1. Spec 19 deliberately gave villagers QUESTS rather than goods-trade so this slot would stay open; Vendor Block fills it. It's also the engine's first **Bitcoin-touching block-entity** — every other economy primitive in this doc shows up as Spec 6 §X.Y; vendor blocks turn that "trade-value" abstraction into actual player-to-player sats flow.

## Narrative

Today: if I farm 64 wheat and a friend wants 32 of them, the only way to trade is to stand next to each other and Q-drop the items, then hope the friend Q-drops something back. There's no price discovery, no asynchronous trade, no shop you can leave running while you're offline.

Vendor Block fixes that with the smallest possible block-entity:

- I craft + place a Vendor Block.
- I right-click it (as owner) → mode picker + stock screen.
- I set **Sell mode, 32 wheat @ 5 sats each** and load 64 wheat into the stock slot.
- A buyer right-clicks it → **buy screen** showing my offer. They click Buy → 5 sats leave their wallet → 1 wheat enters their inventory → my owner-balance ticks up by 5 sats. Repeat until my stock runs out.
- I right-click again → withdraw screen → I can pull stock + accumulated sats.

The owner-side UI is the same shape as the Spec 19 dialogue (Background-order overlay, pinned-width Middle-order panel, layer_painter cursor); the buyer-side UI is a thinner variant of the same.

The Bitcoin path is gated by per-server policy + per-guardian flag, per bitcoin parent controlled. On a non-Bitcoin server (or for a kid whose guardian hasn't enabled sats), Sell and Buy modes are unavailable, and only **Barter** shows in the mode picker. Barter is the always-available trade primitive — kid-friendly, no compliance surface.

## Scope (what's in)

- New `block::VENDOR_BLOCK` (id picked up after Furnace's range).
- Crafting recipe — TBD (Design choice 1).
- Block-entity `VendorData` carrying:
  - Owner identity (pubkey on Bitcoin-enabled servers; player-slot index on alpha single-player).
  - Mode: Sell / Buy / Barter.
  - Item slot (single item for alpha — Design choice 2).
  - Price (sats per unit for Sell / Buy; bartered-item for Barter).
  - Stock count + accumulated balance.
- Owner-side dialogue: mode picker, item-slot edit, price edit, stock view, withdraw button.
- Buyer-side dialogue: read-only offer, Buy button.
- Right-click handler: opens owner UI for the owner, buyer UI for everyone else.
- Anti-grief: non-owners cannot mine the block. Mining as owner returns the block + all stock + accumulated balance.
- Sats settlement: BRIDGE on alpha (same pattern as Spec 19 quest payout — log + toast, real Lightning settlement gated on Spec 16 Reserve drain landing). The hook is wired through the same code path so when the BRIDGE flips, vendor blocks light up alongside quest payouts.
- Save/load via the Spec 17 / Furnace block-entity side-table.

## Cross-economy hooks (added 2026-05-18 after meta-economy review)

Six explicit connections this spec should ship, per `docs/vision/sat-flow-and-economy-loops.md`:

### A. Discovery via villager gossip

Without a way to find a Vendor Block, the Markets economy is dead in the water. Villagers (Spec 19) become the discovery layer:

- Extend `VillagerComponent` with a `gossip_line: Option<String>` (lives on the Spec 19 side; this spec depends on it).
- Daily refresh per villager picks one of: a nearby Vendor Block ("Old Margery is selling wheat for 5 sats at the next village"), an upcoming raid ("bandits coming for the south village"), a reputation tip ("you smell like iron — the Blacksmith would love that"), or a trade asymmetry ("cooked beef is fetching twice as much in the next market").
- Implementation: ~150 LOC sitting on `villager.rs` + `villager_ui.rs`. Ships **before** Vendor Block Phase 5 (the buyer-side UI) so the discovery is there when the UI lands.

This is the **biggest single value-add** for the Vendor Block primitive. A vendor nobody finds is a vendor that doesn't matter.

### B. Reputation gates buying (not just multipliers)

Spec 19 §9's reputation tiers gain teeth:

| Tier | Vendor buying behaviour |
|---|---|
| Hostile | **Blocked.** Right-click vendor → "The village won't trade with you." |
| Wary | Allowed; **prices ×1.25** (the village is suspicious). |
| Neutral | Baseline. |
| Friendly | **Prices ×0.95**; some Friendly-only vendors show a 🌿 tag. |
| Beloved | **Prices ×0.9**; Beloved-only items become available in a special slot category. |

Implementation: the buyer-side UI reads `players[pidx].reputation.tier(vid)` where `vid` is the nearest village to the vendor's position. Tier mismatches block the Buy button; multiplier adjusts the displayed price.

Makes reputation **load-bearing** across both Spec 19 quest and Vendor Block trade. Spec 19 phase 9 already wired the `reward_multiplier` method without using it; Vendor Block consumes it.

### C. Trade-value floor hints in both modes

T1.5 introduces `Item.trade_value()`. Display it in the price field as a *suggestion* not a constraint:

- Sell mode owner UI: "Trade-value: 4 sats. Your price: ___" with a one-click "set to trade-value" shortcut.
- Buyer UI: small "Trade-value: 4 sats" caption under the displayed price. The kid learns "5 sats per wheat is fair; 50 sats per wheat is a rip-off."
- **Barter mode**: trade-value comparator works the same. "5 wheat (TV 20) ↔ 1 bread (TV 20) — fair." Barter-only kids get the same price-discovery aid as Bitcoin-mode kids. Per the Bitcoin/Barter parity posture in `sat-flow-and-economy-loops.md` §3.3.

### D. Multi-slot vendor (3-slot variant)

Single-slot vendors are too restrictive — a kid running a "general store" plants a row of 12 blocks. Sweet spot: **3 item slots per vendor**, each with its own price + stock + mode. The owner UI shows the three side-by-side; the buyer UI shows them as three rows of "item → price → Buy".

Implementation: `VendorData.slots: [Option<VendorSlot>; 3]`. Backward-compatible save format: a Vec of slots written; old saves with 1 slot upgrade trivially. Crafting recipe unchanged.

### E. Stale-shop expiry

A vendor with no transactions for 7 in-game days auto-pauses. The block stays, the inventory stays, but the buyer-side UI shows "Closed — owner hasn't restocked." Right-click as owner unfreezes. Keeps abandoned shops from cluttering the world without destroying anyone's hard-built stalls.

### F. Raid-supplies highlight (Raid Defence integration)

When a Vendor Block sits within 32 blocks of a village with an active or warned raid, AND the item slot's item is on the raid-supplies whitelist (food, arrows, swords, healing potions when those exist), the buyer-side UI shows a 🛡️ "Raid Supplies — wanted for tonight's defence" tag. Creates an in-world *market spike* for raid-prep items.

Implementation: ~30 LOC reading `world.active_raids` from the Raid Defence side-table. Whitelist is hard-coded on alpha; data-driven post-T1.5.

### G. Routes sats through the unified helper

Every Vendor Block transaction goes through `apply_server_tax_and_payout` (specified in Furnace foundation Phase 10). Server-tax + Reserve-drain + Charter-flag gating apply identically to Quest payouts, Raid bounties, and Vendor sales. Per `docs/vision/sat-flow-and-economy-loops.md` §5.

## Scope (what's out — deferred)

- **Multi-item slot vendors.** Alpha = one item per block. Players who want a "general store" plant a row of them. T2 Market Hubs spec gives multi-item.
- **Compass / discovery.** "Where's the nearest market" is the T2 Market Hubs spec.
- **Auction mode.** T3 spec.
- **Trade-by-need bidirectional.** T5 spec.
- **NPC vendor merchants.** T6 — same UI shape but server-stocked, not player-stocked.
- **Vendor density caps + market-hub designation.** Server-policy work; T2 Market Hubs.
- **Threshold auto-payout to owner's Lightning wallet.** Hook is in but the actual LN-keysend lands with Spec 6 §4 LNbits work, post the BRIDGE flipping.

## Design choices

### 1. Crafting recipe

Has to be makeable in the alpha-shipped crafting grid. Candidates:

- **Chest + Sign** — but neither block exists in AxeNStax yet.
- **3×3 plank ring with iron-ingot centre** — gives "wooden shop with iron coin slot" flavour. Iron is a Furnace recipe (Spec 19's blacksmith pool will offer iron-fetch quests, so the iron is socially-available).

**Recommendation: 3×3 plank ring + iron-ingot centre.** Costs 8 planks + 1 iron — moderate barrier, kid-affordable, doesn't gate on blocks that don't exist yet.

### 2. One slot per vendor vs multi-slot

Minecraft Trading is multi-input per villager. Real-world shop blocks (e.g. ComputerCraft inventories) are often single-item.

**Recommendation: one slot.** Forces simple per-block mental model — "this block sells wheat for 5 sats" — and avoids the UI complexity of inventory layouts. The kid plants a row of blocks for a multi-item shop. T2 Market Hubs can ship a multi-slot variant when the demand surfaces.

### 3. Owner identity on alpha

Pre-Spec-1-Phase-4: player identity on the wire is `JoinRequestPacket.player_name` (BRIDGE: client-asserted; CLAUDE.md tech-debt). Post Phase 4: identity is `SignetAuthEvent` + handle credential.

**Recommendation: store owner as `Option<String>` on alpha — the player_name string is fine here because vendor block ownership doesn't need cryptographic enforcement until multiplayer goes hostile.** Migrate to `SignetIdentity` when Phase 4 lands; a one-pass save migration handles existing alpha worlds (treat the legacy string as a synthetic identity).

In single-player + split-screen, owner = player slot index (`Option<u32>`), distinct from the network-identity path. The struct holds an enum: `Owner::Local(u32) | Owner::Remote(String)`.

### 4. Mode picker UI

Three modes — radio buttons, segmented control, or one button per mode?

**Recommendation: segmented control across the top of the owner UI.** Common pattern; visually compact; the kid sees all three options at once and understands the trade-off (Barter doesn't need sats).

### 5. Price input

Three options: numeric input field, +/- buttons, free-text.

**Recommendation: +/- buttons with a fixed sat-tick (1, 5, 25, 100).** Avoids the keyboard-input-into-egui-during-gameplay path that the chat overlay already had to special-case. Numeric input lands when a future general-purpose number-entry widget exists.

### 6. Anti-grief — can the OWNER break the block?

Yes, but the contents drop with the block (item stock + sats accumulator). If the player breaks their own vendor mid-sale (very rare race condition), the buyer's click that's mid-flight no-ops (already a paid-but-the-block-is-gone edge case → buyer's sats refund, vendor block was mined, items return to owner).

### 7. Charter-parent control hook

Per bitcoin parent controlled: sats-touching gameplay is gated by per-guardian flag. Vendor Block Sell + Buy modes are sats-touching; Barter isn't.

**Recommendation: the mode picker shows only the modes the player's Charter flag allows.** A guardian-disabled kid sees only Barter — clean, no error dialogue.

### 8. Receive-side notification

When a buyer purchases from your shop while you're online, do you get a notification?

**Recommendation: a soft toast on the next tick** the owner is online: "Someone bought 3 Wheat from your shop. +15 sats earned." Doesn't disrupt gameplay; the kid sees their revenue accumulate.

## Phasing

| Phase | What | Files | Approx LOC | Solo? |
|-------|------|-------|------------|:----:|
| 1 | Foundation spec (this doc) | – | – | – |
| 2 | `VendorData` struct + ownership model + mode enum + slot/price/stock fields. Block-entity enum variant (`BlockEntityData::Vendor(VendorData)`) added — gated on the Furnace foundation's enum refactor landing first. | depends on Furnace Phase 2; new `vendor.rs` | ~150 | depends |
| 3 | `VENDOR_BLOCK` block-id + 3×3 plank ring + iron centre recipe + texture (chest-like wood top, iron coin-slot front). Placement registers the block-entity with `Owner::Local(pidx)`. | `block.rs`, `crafting.rs`, `texture_gen.rs`, `block_interact.rs` | ~200 | ✓ |
| 4 | Owner-side dialogue UI — segmented mode picker + item slot + price +/- + stock view + Withdraw button. New `vendor_ui.rs`. Renders via the Spec 5 §3.6 layered-painter pattern. | new `vendor_ui.rs`, `game_loop.rs` render hook | ~400 | ✓ |
| 5 | Buyer-side dialogue UI — read-only offer + Buy button + sat balance display. Same module, different render function. Right-click handler chooses which to draw based on `vendor.owner == this_player`. | `vendor_ui.rs`, `game_loop.rs` | ~250 | ✓ |
| 6 | Transaction logic — Buy click decrements vendor stock + increments owner balance + decrements buyer's local sats counter (BRIDGE — Reserve drain integration lands the real settlement). Same toast pattern as Spec 19 quest payout. Barter path swaps items between vendor stock and buyer inventory with no sats touched. | `vendor.rs`, `game_loop.rs` | ~200 | ✓ |
| 7 | Anti-grief — non-owner block-break attempt is silently a no-op + toast "Only the owner can mine this." Mining as owner returns block + remaining stock + balance to owner inventory. | `block_interact.rs`, `vendor.rs` | ~100 | ✓ |
| 8 | Charter-flag gating — mode picker shows only modes allowed by `players[pidx].charter_flags.allow_sats` (or whatever the Charter integration calls it; existing `charter.rs` infra). Barter always available. | `vendor_ui.rs` | ~50 | ✓ |
| 9 | Save/load — `SavedVendor` analog to `SavedFurnace`. `WorldSave` gains `#[serde(default)] vendors: Vec<SavedVendor>` and the load path rehydrates `world.block_entities`. | `save.rs` | ~80 | ✓ |
| 10 | Docs — Spec 5 new section §3.14 "Vendor Block" or similar; Spec 6 §13 (T1.5's introduces; this spec extends) gains "vendor block trade-value flow"; Foundation README marks Vendor Block DELIVERED. | docs only | ~100 | ✓ |
| 11 | Multi-slot conversion — `VendorData.slots: [Option<VendorSlot>; 3]` replaces the single slot. Owner UI shows three rows; save format upgrades existing single-slot saves cleanly. | `vendor.rs`, `vendor_ui.rs`, `save.rs` | ~200 | ✓ |
| 12 | Reputation gating + trade-value floor hints. Buyer-side UI reads `players[pidx].reputation.tier(vid)`; Hostile blocks; Wary/Friendly/Beloved multiply the displayed price; Beloved unlocks a special slot category. Trade-value hint displays in both owner and buyer UI. | `vendor_ui.rs`, `reputation.rs` (uses the existing `reward_multiplier`) | ~250 | ✓ |
| 13 | Stale-shop expiry — `VendorData.last_txn_tick` field; UI shows "Closed" if `now - last_txn_tick > 7 in-game days`. Owner right-click unfreezes. | `vendor.rs`, `vendor_ui.rs` | ~100 | ✓ |
| 14 | Raid-supplies highlight — 🛡️ tag on Vendor Blocks within 32 blocks of a warned-or-active raid where the item is on the raid-supplies whitelist. | `vendor_ui.rs`, reads `world.active_raids` (from Raid Defence side-table) | ~80 | ✓ |
| 15 | Axolittle playtest — kid sets up a 3-slot shop with wheat/bread/iron; friend (P2 split-screen) buys; rep-Hostile prevents purchase from an enemy village's vendor; stale shop turns "Closed" after a saved-then-resumed long break; the village's pending raid lights up the 🛡️ tag on bread + arrow vendors. Confirm Barter mode works on a guardian-disabled session with the trade-value hint visible. | – | – | playtest gate |

**Total Phases 2-14:** ~2,160 LOC across 13 build phases (was 9 + ~1,530; meta-economy review added multi-slot conversion + reputation gating + trade-value hints + stale-shop expiry + raid-supplies highlight).

**Dependencies:**
- Phase 2 (block-entity enum refactor) is shared with the Furnace foundation spec. Whichever ships first lays it; the other inherits.
- The villager gossip extension (`VillagerComponent.gossip_line`, ~150 LOC sitting on Spec 19) ships **before** Phase 5 (buyer-side UI) so vendor discovery works on day one.
- Phase 14 (raid-supplies highlight) depends on Raid Defence having shipped at least its scheduler so `world.active_raids` exists.

## Cross-game lift

- **Owner-vs-buyer dialogue split**. Engine-generic. Any Decented game with player-owned interactive blocks (diner tables for reservations, matchmaking boards) reuses the same dispatch pattern.
- **Sats settlement BRIDGE.** Same hook every Bitcoin-touching gameplay element uses — quest payouts, future raid bounties, plot deeds.
- **Charter-flag gating.** Engine-generic; lifts the moment another sats-touching primitive exists.
- **Mode-picker segmented control + +/- price input.** UI shape reusable for any "configurable owned block" — future plot deeds, future arena registration blocks.

Per shared infra strategy: vendor_ui rendering is generic; the recipe table + item names are AxeNStax-specific data. Keep the engine-generic / game-specific split visible at the file level (`vendor.rs` is the engine primitive; recipe + item bindings live in callers).

## Bitcoin economy hook

Vendor Block is the **first player-to-player Bitcoin gameplay path** in AxeNStax. Quest payouts (Spec 19) flow server → player; Vendor blocks flow player → player.

Compliance posture (per bitcoin parent controlled):

- **Per-server policy**: a Bitcoin-disabled server hides Sell + Buy modes globally; Barter still works. The Spec 16 Reserve drain integration honours the same flag.
- **Per-guardian flag**: a guardian-disabled kid sees only Barter; their Vendor Blocks reject Sell + Buy setup with a toast. A guardian-enabled player can use all three modes — Sell + Buy gated by **also** server policy.
- **UK OSA July 2025**: same posture as Proof-of-Play. Trade is deterministic — `5 sats per wheat × 3 wheat = 15 sats`. No chance-based mechanic. Same proof of play is proof of work argument; nothing new for the compliance-oracle proposal upstream (compliance oracle upstream).

The BRIDGE: until Spec 16 Reserve drain integration matures, Buy transactions log + toast but don't move actual sats. Owner-balance and buyer-sats counters are tracked client-side; a future server-authoritative pass attaches them to Lightning channels. The hook is wired through the same `pop_server_secret`-flavoured code path Spec 19 already uses, so when D-003 reverses (per compliance oracle upstream) both quest payouts AND vendor sales light up simultaneously.

## Open design questions

1. **Sat-tick increments.** 1, 5, 25, 100 feels right for kid-shop scale. Should "Friendly" reputation tier (Spec 19 phase 9) unlock a finer 0.5-sat tick for low-volume items? **Skip for alpha.** Tier-gated price granularity is a clever bonus but adds reputation-lookup paths to the vendor UI; punt to a tier-system polish spec.
2. **Vendor blocks as a service economy?** A "Sell mode" vendor that offers a service (e.g. "1 sword sharpening for 10 sats" — vendor takes a damaged sword, returns sharpened) is structurally identical to a Sell vendor with a recipe step. **Skip for alpha** — it's a workstation-flavoured vendor, distinct primitive. Future spec.
3. **Trade-history log per vendor?** Useful for the owner to see "Wednesday: sold 14 wheat for 70 sats". **Skip for alpha** — egui scrollable log is the right shape, but it's polish on top of the core primitive. Add when the kid asks for it.
4. **Vendor self-replication exploit?** If a Sell vendor sells a Vendor Block, can a player auto-replicate shops? **Yes — and that's fine** in the absence of land economy. T2 Plot Ownership gates "where you can place a vendor" via deeds; until then, free placement is alpha-cool. Add to the playtest open-questions list.
5. **What happens to the buyer's sats counter on a non-Bitcoin server?** They don't have one. The server policy hides Sell + Buy globally → the buyer never sees a Buy button → no edge case.

## Memory-rule check

- ✓ economies vision — slot 4.1 of the vision doc. This spec exists *because* that section called it out as the next spec to write.
- ✓ bitcoin parent controlled — full compliance posture documented above.
- ✓ shared infra strategy — engine-generic primitive, AxeNStax-specific recipe + item bindings.
- ✓ alpha open access — no whitelist gating; any signed-in player can place a Vendor Block.
- ✓ uk english naming — "Vendor" not "Vendor", "centre" not "center". All consistent.
- ✓ proof of play is proof of work — same legal posture; trade is deterministic not chance-based.
- ✓ axenstax has farming — vendor blocks are the trade vehicle for farming output. Tier 1 wheat → vendor block → buyer's bread Furnace.

## Acceptance criteria (Phases 2-10 combined)

- [ ] Crafting a 3×3 plank ring with an iron-ingot centre yields a VENDOR_BLOCK item.
- [ ] Placing a Vendor Block + right-clicking as the owner opens the owner-side UI.
- [ ] Setting a Sell offer (item slot + price), loading stock, and stepping away leaves the block ready to sell.
- [ ] A second player (split-screen P2) right-clicking the same block sees the buyer UI, not the owner UI.
- [ ] Buyer Buy click: stock decrements, owner balance increments, buyer's sats counter decrements, toast confirms.
- [ ] Barter Sell + Buyer Buy path swaps items without touching sats.
- [ ] Non-owner block-break attempt is a no-op + toast.
- [ ] Owner block-break returns the block + remaining stock + accumulated balance to the owner's inventory.
- [ ] Charter-flag-disabled session sees only Barter in the mode picker.
- [ ] Save/load preserves vendor block + item slot + price + stock + balance + owner identity.
- [ ] Sats settlement still logs as BRIDGE — `Vendor sale (BRIDGE): would have credited N sats` — until Spec 16 Reserve drain reverses.
- [ ] All tests pass; check.sh ALL GREEN; bundle stays under 5 MiB brotli.
- [ ] A villager's gossip line surfaces a nearby Vendor Block ("Old Margery is selling wheat for 5 sats at the next village"); the kid can follow the lead and find the shop.
- [ ] A 3-slot vendor can sell wheat + bread + iron simultaneously from one block.
- [ ] Hostile reputation with a village blocks Buy on every Vendor Block in that village's 64-block radius; tier-Wary shows ×1.25 prices; Friendly ×0.95; Beloved ×0.9 with a 🌿 tag on Beloved-only items.
- [ ] Trade-value hint visible in both Sell and Barter mode owner UIs; buyer UI shows trade-value caption.
- [ ] A vendor with no transactions in 7 in-game days shows "Closed" to buyers; owner right-click re-opens it.
- [ ] A Vendor Block within 32 blocks of a warned-or-active raid, selling a whitelisted raid-supply item, displays the 🛡️ tag.

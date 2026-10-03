# Market Hubs (v1) — vendor clustering + compass navigation

**Status:** Phases 2-10 DELIVERED 2026-05-23 on `feat/market-hubs`. Phase N = Axolittle playtest gate (open). (Spec was initially written build-deferred in Round 6 ASSESS, then built same-round.)
**Branch (when built):** `feat/market-hubs` off `main`.
**Trigger:** Goal-driven solo dev cycle, Round 6 ASSESS. From `docs/vision/economies-long-run.md` §4.2 — "Market Hubs (T2)". Written as a queued deliverable; the build was deferred at the tail of a long session because Market Hubs is larger than the self-contained economy blocks (Specs 33-36) and has a soft dependency on plot rentals.

---

## TL;DR

A **Market Hub** is a player-designated zone that aggregates nearby Vendor Blocks (Spec 21) and makes them discoverable. v1 minimal scope:

1. A **Market Bell** block (id 128) — placing it designates a market hub centred on it (radius ~32 blocks). Owned by the placer (reuses the `LocalPlayer(pidx)` / `Npub` owner model that Vendor / Tip Jar / Plot already share — see [[project_economy_block_owner_convergence]]).
2. **Compass navigation** — a `/market` chat command (or a held "Market Compass" item) returns a directional hint to the nearest known Market Hub ("Nearest market: 140 blocks east").
3. **Vendor roll-up** — right-click the Market Bell to see a directory of every Vendor Block within the hub radius: item, price, owner, in-stock count. A read-only "what's for sale here" panel (no remote buying in v1 — you still walk to the vendor).

**Deferred to v2** (the parts that need other systems first):
- **Stall slots / rentals** — needs plot rentals (economies-vision §8.2, T3), which need Plot Ownership v2 (rule toggles + deed transfer). v1 hubs are just discovery + aggregation overlays; they don't own/rent the land.
- **Anti-grief zone rules** (no-PvP, no-break inside the hub) — needs the PvP system + plot rule-toggles. v1 relies on individual Vendor Block + Plot Ownership anti-grief.
- **Vendor density caps** — server-policy anti-monopoly; v2.

1 new BlockId (128, MARKET_BELL), 1 new MaterialId (`MarketBellItem`), optionally 1 `MARKET_COMPASS` tool-or-material, 1 new module (`market_hub.rs` — zone + nearest-hub + vendor-rollup helpers), 1 new UI module (`market_hub_ui.rs` — the directory panel), 1 recipe, 1-2 textures. `World.market_hubs: Vec<MarketHubData>` + `WorldSave.market_hubs`. PROTOCOL_VERSION 34 → 35. **Estimated ~900 LOC including tests.**

---

## Why this lives here

- **Makes the markets economy legible.** Vendor Blocks (Spec 21) work, but a buyer has no way to *find* them — they're scattered. Market Hubs are the discovery layer that turns isolated vendors into a marketplace. Without discovery, the markets economy is theoretical (you can only trade with vendors you happen to walk past).
- **The directory panel surfaces price discovery** — the economies-vision's recurring theme. Seeing "iron sword: 30 sats here, 25 sats at the next stall" is how a kid learns market pricing.
- **Cross-game lifts** — the zone + compass + aggregation pattern is engine-generic.
- **Composes existing primitives** rather than adding new economic mechanics — it reads Vendor Block state (Spec 21) + reuses the owner model (Specs 21/34/36) + the compass hint pattern could lift to village/raid discovery too.

---

## What v1 ships

### 1. `MARKET_BELL` block (id 128)
Placing it creates a `MarketHubData { owner, centre, radius: 32 }`. Owner-only break (anti-grief, like Vendor/Tip Jar). Recipe: a bell-ish shape — e.g. `IronIngot` cap over a `VILLAGE_BELL`-adjacent motif, or 4 iron + a gold nugget; pick a non-colliding shape at build time (the recipe space is getting dense — verify against all existing 3×3 recipes).

### 2. `market_hub.rs` — pure helpers
```rust
pub const MARKET_HUB_RADIUS: i32 = 32;
pub struct MarketHubData { owner: HubOwner, centre: (i32,i32,i32), radius: i32 }
/// Nearest hub to a position + its direction + distance, for the compass.
pub fn nearest_hub(hubs: &[MarketHubData], from: Vec3) -> Option<(&MarketHubData, f32, CompassDir)>;
/// All Vendor Blocks within a hub's radius (reads world.iter_vendors()).
pub fn vendors_in_hub(hub: &MarketHubData, world: &World) -> Vec<VendorListing>;
/// 8-point compass direction from a delta.
pub fn compass_dir(dx: f32, dz: f32) -> CompassDir;
```

### 3. Compass navigation
A `/market` engine command (the command system exists — `commands/builtins/`) that toasts "Nearest market: N blocks <direction>" using `nearest_hub`. Optionally a held Market Compass item that shows the same as a HUD hint (defer to v2 if the command suffices).

### 4. Directory panel (`market_hub_ui.rs`)
Right-click the Market Bell → a scrollable list of `vendors_in_hub`: each row = item name + price (or barter request) + owner + stock. Read-only (walk to the vendor to buy). Esc/'E' closes.

### 5. Save/load
`World.market_hubs: Vec<MarketHubData>` + `WorldSave.market_hubs` (`#[serde(default)]`, plain serde). Owner-break releases the hub (like Plot Marker).

### 6. Tests
Pure: `nearest_hub` picks the closest + correct direction; `compass_dir` 8-point correctness; `vendors_in_hub` radius filter. Integration: place→hub created, vendors-in-radius roll-up, save round-trip, owner-break release.

---

## Dependency note (why build was deferred)

Market Hubs §4.2's full vision (stall **rentals**) depends on:
- **Plot Ownership v2** — rule toggles + deed transfer (Spec 36 shipped v1: claim + build-protection only).
- **Plot rentals** (economies-vision §8.2, T3) — doesn't exist.

v1 as specced above sidesteps this: a Market Hub is a **discovery + aggregation overlay**, not a landlord. It reads existing Vendor Blocks; it doesn't own or rent the land they sit on. That makes v1 fully buildable today against shipped primitives. The rental/stall-slot layer is a clean v2 once plot rentals land.

---

## Implementation phases (per-phase green)

| # | Phase | Est LOC |
|---|---|---|
| 1 | Spec + plan | 0 |
| 2 | MARKET_BELL block + texture + recipe + MaterialId + /give | ~120 |
| 3 | `market_hub.rs` — MarketHubData + nearest_hub + compass_dir + vendors_in_hub + tests | ~250 |
| 4 | `World.market_hubs` + WorldSave round-trip | ~70 |
| 5 | Place-claim + owner-break release | ~60 |
| 6 | `/market` command (compass hint) | ~50 |
| 7 | `market_hub_ui.rs` directory panel + right-click open + dialog branch | ~220 |
| 8 | PROTOCOL 34 → 35 + handshake | ~10 |
| 9 | Integration tests | ~150 |
| 10 | check.sh + doc-flip + queue README | 0 |
| 11 | Merge | — |
| N | Axolittle playtest — hub radius feel, directory usefulness, compass clarity, "do I want remote buying" | n/a |

---

## Acceptance criteria (solo)
- MARKET_BELL craftable/placeable; placing creates a hub owned by the placer.
- `/market` returns a direction + distance to the nearest hub (or "no markets found").
- Right-click shows the in-radius vendor directory.
- Owner-break releases the hub; save/load round-trips.
- `check.sh` ALL GREEN. PROTOCOL 35.

---

## Memory-rule check
- ✓ economies vision — markets economy §4.2.
- ✓ economy block owner convergence — MARKET_BELL reuses the LocalPlayer/Npub owner model; add it to the convergence list when built.
- ✓ bitcoin parent controlled — v1 has no new sats flow (reads Vendor prices; buying still happens at the vendor with its existing Charter gate).
- ✓ proof of play is proof of work — orthogonal.
- ✓ uk english naming — "Market Hub" / "Market Bell" (UK-natural).
- ✓ alpha open access — no whitelist.
- ✓ shared infra strategy — zone + compass + aggregation engine-generic.
- ✓ autonomy to playtest boundary — solo through Phase 10; Phase N = playtest.

---

## Cross-spec interactions
- **Vendor Block (Spec 21)** — the directory reads `world.iter_vendors()` + filters by radius. Read-only; no Vendor changes needed.
- **Plot Ownership (Spec 36)** — v2 stall rentals will tie hubs to rentable plots. v1 hubs are independent overlays.
- **Spec 19 Village Bell** — distinct block + purpose (villager migration vs market discovery); pick a distinct texture + recipe so they don't read as the same thing.
- **Engine commands** — `/market` joins the existing `commands/builtins/` set.

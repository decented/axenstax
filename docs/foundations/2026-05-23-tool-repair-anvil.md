# Tool Repair (Repair Bench) — the economy's first sat sink

**Status:** Phases 2-11 DELIVERED 2026-05-23 on `feat/tool-repair`. Phase N = Axolittle playtest gate (open).
**Branch:** `feat/tool-repair` off `main`.
**Trigger:** Goal-driven solo dev cycle, Round 3. Flagged in `docs/vision/sat-flow-and-economy-loops.md` §6 ("Item-decay & repair — deferred") as the natural **sat sink** counterpart to the sat *sources* shipped in Rounds 1-2 (Mob Bounty Board + Tip Jar both *add* sats to the economy; repair *removes* them). The vision: "damaged tools repair at a Furnace by paying sats + a material cost. Repair tax is the sat sink; material cost is the item sink."

---

## TL;DR

A new **Repair Bench** block (id 126) — a dedicated station so the repair UX doesn't clutter the Furnace's smelting slots (Spec 29). Right-click opens a 2-slot dialog: a **Tool slot** (the damaged tool) + a **Material slot** (the tool's tier material — e.g. IronIngot for an iron pickaxe). A **Repair** button restores durability proportional to the material spent, and deducts a sats **repair tax** routed through `economy::apply_sats_payout(_, PayoutKind::RepairTax, _, _)`.

Design rationale for a dedicated block over "repair at the Furnace": the Furnace already has a 3-slot smelting UI (Spec 29) with a tick-driven state machine. Bolting repair onto it would require disambiguating "is this iron ingot fuel for smelting or material for repair?" — a UX + state-machine hazard. A dedicated Repair Bench keeps both stations single-purpose. The vision says "at a Furnace" loosely; a Repair Bench *next to* a Furnace satisfies the spirit (a smithing corner) without the slot collision.

1 new BlockId (126), 1 new MaterialId (`RepairBenchItem`), 1 new `PayoutKind::RepairTax`, 1 new module (`repair.rs` — pure cost + restore helpers), 1 new UI module (`repair_ui.rs`), 1 new recipe (IronIngot + 2 stone-tier — an anvil-ish shape), 1 procedural texture. No new block-entity (the bench is stateless — the tool + material live in the player's interaction, not the block). **Estimated ~700 LOC including tests.** PROTOCOL_VERSION 32 → 33.

Independent of every other open spec. Touches the now well-trodden block/item/crafting/economy/texture surfaces + a new dialog branch in game_loop.

---

## Why this lives here

- **First sat sink.** Every economy primitive so far *mints* or *transfers* sats (mining, quests, vendors, bounties, tips). Nothing *destroys* them. Without a sink, a Bitcoin-enabled server's in-world sats balance only grows — which the sat-flow vision (§6) flags as an economy-health problem. Repair tax is the first deflationary pressure.
- **Item sink too.** The material cost (ingots consumed per repair) drains crafted materials, giving the production economy a downstream demand that isn't just "make more tools".
- **Closes the durability loop.** Today a tool breaks and is gone — the player re-crafts from scratch. Repair gives an investment-preservation path: a hard-won Diamond pickaxe is worth maintaining, which makes the Proof-of-Play grind that produced it feel durable.
- **Cross-game lifts.** Any Decented game with degradable equipment + a sats layer ships the same Repair Bench + `PayoutKind::RepairTax`.

---

## What this PR ships

### 1. `REPAIR_BENCH` block (id 126)

| ID | Block | Mining behaviour | Notes |
|---|---|---|---|
| 126 | `REPAIR_BENCH` | Drops itself (`RepairBenchItem`). | Stateless — no block-entity. Anvil-ish texture (dark metal block with a worn top face). |

**Recipe (3×3):**
```
I I I
. S .
S S S
```
3 IronIngot top row + 1 Stone centre + 3 Stone (or Cobblestone) bottom row → 1 REPAIR_BENCH. (Minecraft-anvil-adjacent shape; the exact stone-vs-cobblestone choice is a Phase-2 detail to confirm against existing recipe collisions.)

### 2. `repair.rs` — pure cost + restore helpers

```rust
/// Sats charged per point of durability restored. Server-tunable;
/// this is the alpha default. The repair tax is the sat SINK.
pub const REPAIR_TAX_SATS_PER_POINT: u64 = 1;

/// How many durability points one unit of the tier material restores.
/// e.g. 1 IronIngot → RESTORE_PER_MATERIAL points on an iron tool.
pub const RESTORE_PER_MATERIAL: u16 = 50;

/// The material that repairs a given tool. Returns None for tools
/// that can't be repaired (single-tier utility tools like Eraser /
/// FlintAndSteel — v1 keeps it to the material-laddered tools).
pub fn repair_material_for(tool: &Tool) -> Option<MaterialId>;

/// Compute a repair quote: given the tool's current + max durability
/// and the number of material units the player is willing to spend,
/// returns (durability_restored, material_consumed, sats_tax).
/// Caps restoration at the tool's max durability — never over-repairs.
pub fn repair_quote(
    current: u16,
    max: u16,
    material_available: u16,
) -> RepairQuote;

pub struct RepairQuote {
    pub durability_restored: u16,
    pub material_consumed: u16,
    pub sats_tax: u64,
}

/// Apply the quote to a tool in-place. Pure; caller handles the sats
/// payout + material decrement + the Charter/Vow gate.
pub fn apply_repair(tool: &mut Tool, quote: &RepairQuote);
```

Plus a new `Tool::max_durability(&self) -> u16` method on the existing `Tool` struct (generalises the per-material + single-tier-constant logic already in `Tool::new`).

### 3. `PayoutKind::RepairTax`

Append to the enum after `Tip`. **Note the semantic inversion:** every other PayoutKind credits a player; RepairTax *debits* one. The `apply_sats_payout` pipeline returns a `credited` amount that, for RepairTax, represents "sats successfully removed from the player". On a Bitcoin-disabled / Charter-off server the repair is **free** (no sats to charge) — the material cost still applies (the item sink works regardless of the Bitcoin layer). Label: `"repair tax"`.

### 4. `repair_ui.rs` — 2-slot repair dialog

- Right-click `REPAIR_BENCH` opens the dialog.
- **Tool slot**: click to deposit the held tool (or the first damaged tool in the hotbar).
- **Material slot**: auto-shows the required material (`repair_material_for`) + how many units the player has.
- A live **quote line**: "Restore 100 durability · 2 IronIngot · 100 sats".
- **Repair** button — applies the quote. Disabled when the tool is at full durability, the material is missing, or (on Bitcoin servers) the player can't afford the tax.
- Esc / 'E' closes (matches the established convention).

### 5. Wiring in game_loop.rs

- Right-click `REPAIR_BENCH` → `player.open_repair_bench = Some(pos)`.
- Per-frame dialog branch alongside the existing vendor/furnace/chest/bounty/tip ladder.
- Repair handler:
  1. The dialog renders a quote computed from a render-time snapshot (preview only).
  2. **On apply, re-derive against the LIVE inventory** via `repair::execute_repair(&mut inventory, hotbar_slot)`: it re-quotes from the tool currently in the slot + the player's *current* material count, consumes exactly `quote.material_consumed` (always `<= available`), restores the matching durability, and returns the applied quote. The old handler trusted the snapshot quote and applied the **full** `durability_restored` even after `consume_material` came up short — a free/partial repair (engine audit 2026-06-04, A). Re-deriving makes the restore always fully paid for and immune to a stale held-slot index.
  3. Charter/Vow check — on Bitcoin-enabled + Charter-on + no-Vow, charge the sats tax via `apply_sats_payout(applied.sats_tax, PayoutKind::RepairTax, ...)`. Otherwise repair is sats-free.
  4. Toast: "Repaired — +N durability for M sats" (or "…for free" on Bitcoin-disabled). Regression: `repair::execute_repair_only_restores_what_live_material_pays_for`.

### 6. `PlayerSlot.open_repair_bench` field + /give aliases (`repair_bench`, `anvil`).

### 7. PROTOCOL_VERSION 32 → 33 + handshake history line.

### 8. Test surface

Pure-function tests in `repair.rs`:
- `repair_quote_caps_at_max_durability`
- `repair_quote_scales_with_material`
- `repair_quote_zero_when_already_full`
- `repair_material_for_iron_tool_is_iron`
- `repair_material_for_utility_tool_is_none`
- `apply_repair_restores_durability`
- `tool_max_durability_matches_new`
- `repair_tax_is_zero_for_zero_restore`

Integration tests in `test_integration/repair.rs`:
- `repair_charges_sats_on_bitcoin_server`
- `repair_is_free_on_bitcoin_disabled`
- `repair_consumes_material`
- `repair_never_exceeds_max_durability`

---

## Out of scope (v2 polish round, post-playtest)

- **Repairing utility tools** (Eraser / FlintAndSteel / Shears / FishingRod / Slingshot). v1 = material-laddered tools only (pickaxe / axe / shovel / hoe / sword). The single-tier tools have no obvious "tier material".
- **Repair degrades max durability** (Minecraft anvil "Too Expensive" / prior-work penalty). v1 = repair restores to full cap with no penalty. v2 may add a soft cap.
- **Combining two damaged tools** (Minecraft anvil tool-merge). v1 = material-only repair.
- **Enchantment/naming** (Minecraft anvil's other functions). Out of scope entirely — no enchantment system exists.
- **Armour repair.** v1 = tools only. Armour repair is a clean v2 extension (armour already has per-piece durability from Spec 28e).

---

## Known limitations (documented BRIDGEs)

1. **No real sats wallet on alpha.** Same BRIDGE as Vendor Block + Tip Jar — `apply_sats_payout` is accounting-only; the repair tax is logged + audited but no actual balance moves. The material cost (item sink) DOES apply for real, so the repair is meaningfully gated even pre-wallet. **BRIDGE:** `// BRIDGE: repair tax is accounting-only until LN settlement + a real per-player wallet land.`
2. **Repair Bench "near a Furnace" is not enforced.** The vision frames repair as a smithing-corner activity but v1 lets the bench stand alone. No adjacency requirement — keeps it simple. If playtest wants the smithing-corner fiction, a Phase-N follow-up can add a "must be within N blocks of a lit Furnace" gate.

---

## Implementation phases

| # | Phase | Files | Est LOC |
|---|---|---|---|
| 1 | Spec + plan docs | docs/ | 0 |
| 2 | BlockId + texture + BlockDef + MaterialId + recipe + /give | `block.rs`, `texture_gen.rs`, `item.rs`, `crafting.rs`, `commands/builtins/give.rs` | ~120 |
| 3 | `Tool::max_durability` method + `repair.rs` module + 8 pure tests | `crafting.rs`, new `repair.rs` | ~220 |
| 4 | `PayoutKind::RepairTax` + label | `economy.rs` | ~10 |
| 5 | `PlayerSlot.open_repair_bench` field | `player_slot.rs` | ~5 |
| 6 | `repair_ui.rs` dialog | new `repair_ui.rs` | ~200 |
| 7 | game_loop right-click + dialog branch + repair handler | `game_loop.rs` | ~150 |
| 8 | PROTOCOL_VERSION 32 → 33 + handshake history | `protocol.rs`, `test_integration/handshake.rs` | ~10 |
| 9 | Integration tests | new `test_integration/repair.rs`, `test_integration/mod.rs` | ~150 |
| 10 | `./check.sh` ALL GREEN | — | — |
| 11 | Doc-flip + queue README | docs/ | 0 |
| 12 | Merge to main --no-ff | — | — |
| N | Axolittle playtest gate — repair-cost balance + material-cost feel + "is the bench discoverable / does it want furnace-adjacency" | n/a | n/a |

**Phases 2–11 are solo-buildable.** Phase N (playtest) is Axo-gated — the repair-cost + material-cost tuning genuinely needs a kid's economy feel.

---

## Acceptance criteria (solo)

- REPAIR_BENCH craftable, placeable, mineable; right-click opens the dialog.
- Depositing a damaged material-laddered tool + its tier material shows a live quote.
- Repair restores durability (capped at max), consumes material, charges sats on Bitcoin servers / free on Bitcoin-disabled.
- Full-durability tool → Repair button disabled.
- No block-entity state (bench is stateless); save/load unaffected beyond the block id itself.
- `check.sh` ALL GREEN. PROTOCOL_VERSION 33.

---

## Memory-rule check

- ✓ economies vision + `sat-flow-and-economy-loops.md` §6 — first sat sink + item sink.
- ✓ bitcoin parent controlled — repair tax goes through `apply_sats_payout` with Charter gating; free on Charter-off (material cost still applies, so the mechanic works in Barter mode).
- ✓ proof of play is proof of work — repair is a deterministic cost, not chance-based. No gambling.
- ✓ uk english naming — "Repair Bench" (not "Repair Station"); UK-natural.
- ✓ alpha open access — no whitelist.
- ✓ shared infra strategy — block + payout-kind + repair helpers are engine-generic.
- ✓ autonomy to playtest boundary — solo through Phase 11; Phase N = playtest.
- ✓ merge to main preauthorised — healthy-gate merge.

---

## Cross-spec interactions

- **Spec 29 Furnace** — deliberately NOT merged into the Furnace UI (slot-collision hazard); the Repair Bench is a sibling station. If a future playtest wants the smithing-corner fiction, add a furnace-adjacency gate.
- **Spec 28e Armour** — armour has per-piece durability already; armour repair is a clean v2 extension of this same bench.
- **Proof-of-Play floor** — repair preserves the tools that mine the floor, making the grind's output durable. The sats charged by repair flow back out of the player's balance, providing the first deflationary counter-pressure to the PoP sat faucet.
- **Vendor Block / Tip Jar / Bounty Board** — all three are sat *sources*; Repair Bench is the first *sink*. Together they start to form a closed loop rather than a one-way faucet.

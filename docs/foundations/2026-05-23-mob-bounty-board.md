# Mob Bounty Board — first combat economy primitive

**Status:** Phases 2-12 DELIVERED 2026-05-23 on `feat/mob-bounty-board`. Phase N = Axolittle playtest gate (open). Includes reviewer-pass fix persisting `kill_counter` + `bounties_claimed` so bounty progress survives save+quit.
**Branch:** `feat/mob-bounty-board` off `main`.
**Trigger:** Goal-driven solo dev cycle. Picked from `docs/vision/economies-long-run.md` §6.1 — "Mob Bounty Board (T2 — first combat economy spec)" — as the smallest, most self-contained, highest-leverage opener for the combat economy lane. Sits parallel to (and uses) the just-shipped last-attacker kill attribution (merge `7f6de9d`).

---

## TL;DR

A new placeable block — `BOUNTY_BOARD` (id 124) — surfaces server-issued bounties to the player: *"Kill 10 Zombies for 100 sats"*. Right-click the board to open a dialog listing the day's bounties; a Claim button is enabled per bounty when the player's `kill_counter` has the required count. Claiming consumes the count, fires `economy::apply_sats_payout(_, PayoutKind::BountyClaim, _, _)`, and emits a HMAC-SHA256 audit hash of `(player_pubkey, mob_kind, kill_count, server_secret)`.

Bounties are server-generated. v1 ships **3 fixed templates** sampled into a per-day rotation:
- *Kill 10 Zombies* — 100 sats (or 50 rep on Bitcoin-disabled / Charter-disabled).
- *Kill 5 Skeletons* — 75 sats (or 35 rep).
- *Kill 3 Brigands* — 150 sats (or 75 rep).

The day's rotation refreshes every `BOUNTY_REFRESH_TICKS = 24 000` ticks (~1 in-game day, same cadence as the brigand hideout replenisher + rubber tap cooldown).

1 new BlockId (124), 1 new MaterialId (`BountyBoardItem` — the placed block's item form), 1 new `PayoutKind::BountyClaim`, 1 new module (`bounty.rs`), 1 new UI module (`bounty_ui.rs`), 1 new recipe (8 planks ring around an iron ingot — same shape as Vendor Block), 1 new procedural texture. **Estimated ~900 LOC including tests.** PROTOCOL_VERSION 30 → 31.

Independent of every other open spec — no shared files with Specs 1 (engine Signet auth) or 2 (HostedServer routing). Parallelisable with anything currently in flight.

---

## Why this lives here

- **First combat-economy primitive.** Until now, the only sats-payout paths are mining (Proof-of-Play), Spec 19 quests, Spec 21 vendor sales, Spec 22 raid bounties, Spec 25 plaque tips, and Spec 26 builder commissions. None of them route value through *combat skill on its own*. The bounty board makes "I am good at fighting" a directly-paying loop, which the kid playtest will want.
- **Closes the loop on the just-shipped last-attacker fix** (merge `7f6de9d`). Without it, bounty credit would silently leak to whichever teammate happened to be closer at moment-of-death. With it, kills credit to the actual attacker — exactly the contract a bounty board needs.
- **Lifts cross-game.** Every Decented game with mobs (other games on the same primitives, future games) can ship the same Bounty Board block + the same `economy::apply_sats_payout(_, PayoutKind::BountyClaim, _, _)` wiring with no AxeNStax-specific assumptions. The mob-type-to-payout mapping is data-driven through the bounty template table.
- **Unblocks the C-tier combat specs.** Per `docs/vision/economies-long-run.md` §13, Boss Fights (C-tier) and Dungeon Clears (C-tier) extend the same primitive (server-funded payout pool + verified kill attribution).

---

## What this PR ships

### 1. `BOUNTY_BOARD` block (id 124)

| ID | Block | Mining behaviour | Notes |
|---|---|---|---|
| 124 | `BOUNTY_BOARD` | Drops itself (`BountyBoardItem` material). | Solid 1×1 placeable. Texture: cork-board base with parchment notes pinned. Right-click opens the bounty dialog. |

Recipe (3×3):
```
P P P
P S P
P P P
```
8 planks (species-neutral via `is_any_planks`) + 1 PapyrusSheet at centre → 1 BOUNTY_BOARD. (PapyrusSheet rather than IronIngot avoids the recipe collision with Vendor Block, and is thematically apt — bounty notices pinned to parchment.)

### 2. New `bounty.rs` module

Pure helpers + tick driver. Mirrors the `salt_lick.rs` / `rubber.rs` shape:

```rust
pub const BOUNTY_REFRESH_TICKS: u64 = 24_000;

/// One bounty template — server-issued, claimed by any player.
pub struct BountyTemplate {
    pub mob_kind: MobType,
    pub required_count: u32,
    pub payout_sats: u64,
    pub fallback_rep: i32,  // when sats are suppressed (Charter-off)
    pub label: &'static str,
}

/// Built-in template pool. v1 = 3 fixed templates.
pub const BOUNTY_TEMPLATES: &[BountyTemplate] = &[
    BountyTemplate { mob_kind: MobType::Zombie,   required_count: 10, payout_sats: 100, fallback_rep: 50, label: "Kill 10 Zombies" },
    BountyTemplate { mob_kind: MobType::Skeleton, required_count: 5,  payout_sats: 75,  fallback_rep: 35, label: "Kill 5 Skeletons" },
    BountyTemplate { mob_kind: MobType::Brigand,  required_count: 3,  payout_sats: 150, fallback_rep: 75, label: "Kill 3 Brigands" },
];

/// One active bounty in the world. Many players may claim it independently
/// (each player's claim is tracked separately by `bounty_claims`).
pub struct ActiveBounty {
    pub id: u32,                  // monotonic, save-stable
    pub template_idx: usize,       // index into BOUNTY_TEMPLATES
    pub issued_tick: u64,          // tick_counter at issue time
}

/// Returns true if `player_kills` ≥ template required count.
pub fn can_claim(template: &BountyTemplate, player_kills: u32) -> bool { /* ... */ }

/// Roll the day's bounty rotation: pick 2 or 3 templates pseudo-
/// randomly from BOUNTY_TEMPLATES, return a Vec<ActiveBounty>.
pub fn roll_daily_bounties(world_seed: u32, day_index: u64, next_id: &mut u32) -> Vec<ActiveBounty>;

/// Tick driver: refreshes the bounty rotation every BOUNTY_REFRESH_TICKS.
pub fn tick_bounty_refresh(world: &mut World, monotonic_tick: u64, world_seed: u32);

/// HMAC audit hash for a claim, per `docs/vision/economies-long-run.md` §6.1.
/// Player-side hash only — server replay verification lands when the
/// Spec 1 Phase 4 signing bridge resolves.
pub fn claim_audit_hash(
    player_pubkey: &[u8; 32],
    mob_kind: MobType,
    kill_count: u32,
    server_secret: &[u8],
) -> [u8; 32];
```

### 3. `World.bounties: Vec<ActiveBounty>` index

- Held on `World` as a runtime side-table; populated by `tick_bounty_refresh`.
- Persisted via a new `WorldSave.bounties: Vec<SavedBounty>` + `WorldSave.bounty_next_id: u32` + `WorldSave.bounty_last_refresh_tick: u64` — all marked `#[serde(default)]` so legacy saves load cleanly.
- Per-player claim history: `PlayerSlot.bounties_claimed: AHashMap<u32, u32>` (bounty_id → kill_count_at_claim), so reclaiming the same bounty is blocked until the rotation refreshes the bounty id.

### 4. `PayoutKind::BountyClaim`

Appended to the `PayoutKind` enum in `economy.rs` after `BuilderCommission`. Label string: `"bounty claim"`. Positional bincode enum-append discipline — append-only.

### 5. `bounty_ui.rs` — claim dialog

- Right-click `BOUNTY_BOARD` opens an egui floating dialog.
- Header: *"Bounty Board — refreshes at dawn"*.
- One row per active bounty:
  - Label, required kill count, current player kill count, payout (sats OR rep depending on policy + Charter flag).
  - Claim button — enabled when `kill_counter ≥ required`, disabled with a tooltip otherwise.
- Esc / 'E' closes (matches the Spec 29 Furnace UX convention).
- Buyer/owner split unnecessary — the board is server-issued; no per-player ownership.

### 6. Claim path wiring

When the Claim button is pressed for an ActiveBounty:

1. `bounty::can_claim(template, player_kills)` — must be true. Defensive (UI already gates).
2. Drain `required_count` from the player's `kill_counter` for that mob kind.
3. Record the claim in `PlayerSlot.bounties_claimed[bounty_id] = required_count`.
4. Sats branch:
   - `economy::apply_sats_payout(payout_sats, PayoutKind::BountyClaim, &policy, charter_allows_sats)`.
   - If `result.credited > 0`: credit to player's sats balance, toast *"Claimed: +N sats"*.
   - If `result.suppressed`: skip sats, fall through to rep branch.
5. Rep branch (fallback when sats suppressed):
   - Apply `fallback_rep` to all village reputations within 64 blocks (mirrors Spec 22 raid rep distribution).
   - Toast *"Claimed: +N reputation in nearby villages"*.
6. Emit `claim_audit_hash` to the engine log (`info!`) for the future server-side replay check.

### 7. Tick driver wiring

`bounty::tick_bounty_refresh(&mut self.world, self.tick_counter, self.biome_gen.seed)` called from both:
- `game_loop.rs::tick()` next to the existing salt_lick + rubber cooldown calls.
- `server.rs::tick()` next to the same calls.

Self-throttled internally on `BOUNTY_REFRESH_TICKS`; safe to call every tick. MUST use `tick_counter` (monotonic), not `world_time` (cyclic) — same lesson as the merge `1f4c3c0` clock-fix round.

### 8. /give aliases

`bounty_board` + `board` for the BOUNTY_BOARD material.

### 9. Test surface

Pure-function tests in `bounty.rs`:
- `can_claim_returns_true_at_exact_count`
- `can_claim_returns_false_below_count`
- `roll_daily_bounties_is_deterministic_per_seed_and_day`
- `roll_daily_bounties_returns_2_or_3_bounties`
- `tick_bounty_refresh_no_op_within_period`
- `tick_bounty_refresh_replaces_on_period_boundary`
- `claim_audit_hash_is_deterministic`

Integration tests in `test_integration/bounty.rs`:
- `claim_drains_kill_counter_and_records_claim`
- `claim_fires_payout_with_correct_kind`
- `cannot_double_claim_same_bounty_id`
- `bounty_index_round_trips_save_load`
- `daily_refresh_changes_bounty_ids`

### 10. PROTOCOL_VERSION bump

30 → 31. History line in `test_integration/handshake.rs`:
```
// v31 (2026-05-23): Mob Bounty Board — BOUNTY_BOARD (id 124) +
// BountyBoardItem MaterialId + PayoutKind::BountyClaim + 3 new
// WorldSave fields (bounties Vec, bounty_next_id, bounty_last_refresh_tick).
// Positional bincode enum append.
```

---

## Out of scope (v2 polish round, post-playtest)

- **Player-posted bounties.** v1 = server-issued only. Player → player bounty escrow lives in Spec 25 follow-on (Plan-trade was first player-economy plumb-in; bounty pool comes later).
- **Bounty stacking.** A player can only have one outstanding claim per bounty id. Multi-claim batching is a v2 UX polish.
- **Variable bounty payouts.** v1 = fixed sats per template. Dynamic supply-demand pricing (more kills → lower payout) is v2.
- **NPC bounty-issuer Villager.** v1 = passive board only. A "Bounty Hunter" villager profession that walks players through bounties is v2.
- **Boss bounties / dungeon-clear bounties** (C-tier in the economies vision). Separate foundation specs each.

---

## Known limitations (documented BRIDGEs)

1. **Server-side kill verification gap.** Per CLAUDE.md "Known technical debt" §1 (single-player bypasses GameServer): `kill_counter` is currently client-authoritative. A cheating client could inflate its counter and claim bounties. The HMAC audit hash captures the claim for post-hoc replay-check, but enforcement waits for the dual-sim cleanup. **BRIDGE:** `// BRIDGE: bounty claim is client-asserted until single-player routes through HostedServer (Spec 2 Phase 5).` In `bounty_ui.rs` claim handler.
2. **Projectile kill attribution still proximity-only.** The just-shipped last-attacker fix (`7f6de9d`) only covers melee. Bow / Slingshot kills fall back to nearest-player. Acceptable for v1; tightened when projectile-hit path also stamps `LastAttacker`.

---

## Implementation phases

| # | Phase | Files | Est LOC |
|---|---|---|---|
| 1 | Spec doc (this file) + plan doc | `docs/foundations/`, `docs/superpowers/plans/` | 0 (docs) |
| 2 | BlockId + texture + BlockDef + MaterialId + recipe | `block.rs`, `texture_gen.rs`, `item.rs`, `crafting.rs` | ~120 |
| 3 | `bounty.rs` module — templates, ActiveBounty, can_claim, roll_daily_bounties, claim_audit_hash | new `bounty.rs` | ~250 |
| 4 | `World.bounties` runtime index + `WorldSave` serde + `rebuild_bounty_state_on_load` if needed | `world.rs`, `save.rs` | ~80 |
| 5 | `PayoutKind::BountyClaim` + label | `economy.rs` | ~10 |
| 6 | Tick driver `tick_bounty_refresh` + wire into game_loop + server | `bounty.rs`, `game_loop.rs`, `server.rs` | ~80 |
| 7 | `PlayerSlot.bounties_claimed` field + save round-trip | `player_slot.rs`, `save.rs` | ~30 |
| 8 | `bounty_ui.rs` dialog + right-click handler | new `bounty_ui.rs`, `game_loop.rs` | ~200 |
| 9 | Claim path — drain kill_counter, fire payout, emit audit hash, toast | `game_loop.rs` (or `bounty_ui.rs`) | ~80 |
| 10 | `/give` aliases | `commands/builtins/give.rs` | ~10 |
| 11 | PROTOCOL_VERSION 30 → 31 + handshake history line | `protocol.rs`, `test_integration/handshake.rs` | ~10 |
| 12 | Integration tests (5 in `test_integration/bounty.rs`) | new `test_integration/bounty.rs`, `test_integration/mod.rs` | ~180 |
| 13 | Doc-flip + queue README row | `docs/foundations/2026-05-23-mob-bounty-board.md`, `docs/foundations/README.md` | 0 (docs) |
| N | Axolittle playtest gate — bounty discoverability + claim feel + reward balance | n/a | n/a |

**Phases 2–12 are solo-buildable.** Phase N (playtest) is Axo-gated and stops the build.

---

## Acceptance criteria (solo)

- `BOUNTY_BOARD` craftable, placeable, mineable; drops itself.
- Right-clicking opens a dialog showing the day's 2–3 bounties.
- Claim button is disabled with a tooltip when kill_counter is too low; enabled when reached.
- Claiming drains the kill_counter, fires `PayoutKind::BountyClaim`, and (on Bitcoin-disabled / Charter-off) falls back to rep.
- Re-clicking Claim on the same bounty id does nothing (already claimed).
- After `BOUNTY_REFRESH_TICKS` ticks, the dialog shows new bounty ids and `bounties_claimed` no longer blocks them.
- Save → quit → reload → bounties survive, `bounties_claimed` survives.
- `check.sh` ALL GREEN. PROTOCOL_VERSION 31.

---

## Memory-rule check

- ✓ economies vision — first combat economy spec. Vision §6.1.
- ✓ bitcoin parent controlled — sats path goes through `apply_sats_payout` with Charter-flag gating; rep fallback on suppression.
- ✓ proof of play is proof of work — bounty payouts are work-based (kill effort), not chance-based. No gambling primitive introduced.
- ✓ uk english naming — UK English throughout (no "vendue", no "tester-payment").
- ✓ alpha open access — no whitelist; every player with a Signet identity can claim.
- ✓ shared infra strategy — board + payout-kind + audit hash are engine-generic; lifts cross-game.
- ✓ autonomy to playtest boundary — solo-buildable through Phase 13; Phase N is the playtest gate.
- ✓ merge to main preauthorised — healthy-gate merges per `check.sh` green.

---

## Cross-spec interactions

- **Spec 22 Raid Defence** — raid kills also drain `kill_counter`. A player who clears a raid wave gets credit toward open Zombie/Skeleton bounties simultaneously. **Intentional** — rewards layered combat involvement.
- **Spec 19 Quests** — Villager Kill quests also use `kill_counter`. Claiming a bounty drains it; if a player accepted a Villager Kill quest for the same mob type, the bounty claim may pull their quest progress backward. Acceptable for v1 (player chooses their reward path); documented in the bounty dialog as *"Claiming uses kills shared with active quests"*.
- **HP-3 / HP-6 Brigand spawning** — bounty template targets `MobType::Brigand`. The Brigand mob actually spawns from hideouts + the post-cutover night-spawn pool; both paths credit through `LastAttacker`. No new wiring needed.
- **Nostrich Vow** — players under the Nostrich Vow have sats payouts suppressed (per `nostrich_vow.rs`). Bounty payouts MUST honour the vow — `apply_sats_payout` already gates on the charter flag but the Vow is a separate suppression. Confirm during build that the existing `nostrich_vow::sats_suppression_active` check is consulted at the claim site.

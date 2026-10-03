# Deepslate Reserve & In-World Visibility

**Status**: READY TO BUILD. Phases 2–5 are autonomous solo work. Phase 6 (Axolittle playtest) blocks on his time.
**Date**: 2026-05-17
**Branch**: `feat/deepslate-reserve` once started; off `main`.
**Session**: Fresh — implementer should treat this doc + Spec 6 (especially §1.4, §6.2, §7) as the only briefs.
**Trigger**: 2026-05-17 conversation hardening the Bitcoin layer's closed-world economy + pay-to-win-proof principle. The reward pool mechanics already exist in Spec 6 §7 as `reward_pool`. This spec adds the **fiction layer** that makes that pool visible, intelligible, and emotionally resonant for the player — particularly the kid player who needs to understand "where the sats come from" without reading economic policy documents.

---

## TL;DR

Spec 6 §7 specifies a `reward_pool` LNbits wallet that mining rewards draw from. Today that pool is **invisible** to the player except as a number in the Treasury Transparency Panel ("Pool Balance: 45,230 sats"). It works mechanically but it doesn't *feel* like anything in the game world.

This spec adds the fiction layer: **the pool's depth is rendered as the richness of the deepslate the player can see and mine**. A well-funded server has visibly thick, varied deepslate at depth — bright veins, dense ore-like seams (cosmetic only — the mining mechanic is unchanged). A depleted server has thin, pale deepslate. Players read the server's economic health from the world itself, not from a stats panel.

The same fiction extends to the "Fund the Reserve" flow (Spec 6 §6.2 amended): paying sats into the pool spawns visible "rich deepslate" pockets that everyone can see and mine. The act of sponsorship has a visual footprint in the world.

**Scope: ~400 lines of code + tests across 6 phases.** Phase 6 is the playtest gate. No new server-side economic mechanics — every change is either rendering (mesh / shader) or UX (HUD / menu copy).

---

## Why this lives here

- **Spec 6 mechanics are correct but invisible.** The whole §1.4 closed-world principle hinges on the player *understanding* that funding goes to a shared pool. Numeric transparency (current §7.5 panel) is necessary but not sufficient — a 9-year-old doesn't read tables. Visual transparency lands intuitively.
- **The "buried sats" fiction is already in the canon.** Spec 6 §2.2c.2 describes Satori vein generation in deepslate. Spec 6 §10.5 #5 calls the player's role "Proof of Play" — they're observing proof-of-work, not generating it. The "sats are buried in the rock" metaphor matches the established lore; this spec just makes it visible.
- **Cross-game lift.** Any Decented game with a "mining-equivalent" loop (ingredient-gathering, egg-hunting) can adopt the same "pool-as-visible-world-feature" pattern. The primitive is engine-generic; the *what gets rendered as rich* is per-game.
- **Kid-friendliness.** Axolittle's design feedback consistently favours **visible feedback loops** over abstracted stats. The deepslate-as-pool framing is exactly the kind of mechanic that reads instantly to an 11-year-old: "the rock is full of sats; I dig; I get sats; my friend's dad just sponsored, so now the rock is fuller."

---

## Context pointers

### Existing code surfaces this touches

- `game/engine/src/block.rs` — block registry. Add deepslate "richness" variants (cosmetic block IDs) or a single deepslate variant with a runtime `richness` parameter (favoured — keeps the registry lean).
- `game/engine/src/world/biome.rs` (or wherever deepslate gen lives) — chunk generation at depth. Currently produces uniform deepslate; needs to weight visual variants by server-side richness signal.
- `game/engine/src/renderer.rs` + shader files — the deepslate texture variants render with subtle differences (vein density, colour saturation). No new shader stage; just texture array variants.
- `game/engine/src/hud_ui.rs` — HUD richness gauge (replaces or augments the current Treasury Transparency Panel entry).
- `game/engine/src/proof_of_play.rs` (if it exists in code; spec form is Spec 6 §2) — the per-strike HMAC roll. The visual richness doesn't change the roll outcome, only the visual; mechanic untouched.
- `tools/sites/game/static/css/menu.css` (PWA-side) — "Fund the Reserve" dialog styling.

### Native vs WASM scope

Everything in this spec is rendering + UX. **Cross-platform on both targets.** The reserve-state-as-RPC is a small JSON payload from server to client; nothing that depends on QUIC vs WebRTC.

### Related specs

- **Spec 6 §1.4 (closed-world principle)** — the architectural backbone this spec serves.
- **Spec 6 §6.2 (Fund the Reserve, amended 2026-05-17)** — the UX flow this spec deepens.
- **Spec 6 §7 (Treasury and Pool Management)** — the mechanical pool this spec visualises. No mechanical change; only visualisation.
- **Spec 6 §2.2c (Satori vein generation)** — the existing buried-sats fiction. Continues unchanged; the Deepslate Reserve is a sibling visualisation, not a replacement for Satori veins.
- `docs/vision/economies-long-run.md` §11.1 (Proof-of-Play floor) — the cross-economy framing this fits into.

### Memory pointers

- proof of play is proof of work — the structural argument that pool-driven mining is not gambling.
- bitcoin parent controlled — Bitcoin features are opt-in per server, parent-controlled. The reserve visualisation works in both Bitcoin-enabled and Bitcoin-disabled modes (the latter shows "untranslated proof-of-work effort" without the sats overlay).
- economies vision — the multi-economy vision this serves.
- uk english naming — UK English throughout. "Sponsor", "Fund", "Reserve" — not "Patron" / "Donate" / "Pool" (the latter still works as a technical term but in the user-facing copy prefer "Reserve").
- autonomy to playtest boundary — Phases 2–5 solo, Phase 6 = Axolittle.

### What does NOT exist yet (and this spec does NOT need)

- Real-time chunk regeneration when reserve depth changes. Reserve richness affects **newly-loaded chunks**; already-streamed chunks update on next reload or via a per-chunk "refresh" tick. No live-mutate of every loaded chunk on every funding event.
- Live deepslate "respawn" when mined out. Mining still removes the block; the visual richness affects new visible blocks streamed in, not previously-mined-out areas. The reserve depth determines *new* visible richness; player choices of where to mine are unchanged.
- Per-player visual variance. Every player sees the same deepslate richness on the same server — server-state derived, not per-player.

---

## Scope

| # | Phase | Files | Est. lines | Autonomous? |
|---|-------|-------|:---:|:---:|
| 1 | **This spec** | `docs/foundations/2026-05-17-deepslate-reserve.md` | ~400 | ✓ |
| 2 | Reserve state RPC — server publishes richness signal (0.0–1.0) every N seconds; client renders accordingly | `game/engine/src/protocol.rs`, `game/engine/src/server.rs` | ~80 | ✓ |
| 3 | Visual variants — deepslate texture array with 4 richness tiers; chunk gen weights variant by server signal | `game/engine/src/block.rs`, `game/engine/src/chunk.rs`, shader textures | ~150 | ✓ |
| 4 | HUD richness gauge — replaces "Pool Balance" raw number with a visual gauge ("The Reserve is fat / healthy / thin"); add to Bitcoin menu | `game/engine/src/hud_ui.rs` | ~80 | ✓ |
| 5 | Fund-the-Reserve dialog — visual flow showing where sats go, split breakdown, "sponsor of [server]" confirmation | `tools/sites/game/static/js/auth.js` (PWA-side), `game/engine/src/menu.rs` (native-side) | ~90 | ✓ |
| 6 | Axolittle playtest — does the "rich deepslate" reading feel right? Does the funding flow feel like sponsorship not pay-to-win? | n/a | 0 | ✗ blocked |

**Total**: ~400 lines spec + code. Phase 6 is the playtest gate.

Phases 2 → 3 → 4 → 5 are roughly serial. Phase 2 establishes the data flow; Phase 3 consumes it in render; Phase 4 surfaces it in HUD; Phase 5 ties it to the funding action.

---

## Phase 1 — This spec

You're reading it. ✓ Move on.

---

## Phase 2 — Reserve state RPC

### Goal

Server publishes a single floating-point "richness" signal (0.0 = empty, 1.0 = at-or-above target) to clients on every reserve-state change (funding event, payout milestone, periodic 30-second sync).

### Design

The server already computes `reward_multiplier` (Spec 6 §7.2) which is exactly the 0.0–1.0 ratio we want. Two changes:

1. Promote `reward_multiplier` to a wire-visible field in the existing `ServerState` or `StateUpdate` packet (the engine already broadcasts world state periodically).
2. Add a richness-changed event flag so clients can re-render deepslate variants when crossing the visual thresholds (every 0.1 increment, say).

```rust
// In protocol.rs
pub struct ServerStatePacket {
    // ... existing fields ...
    pub reserve_richness: f32,        // 0.0 - 1.0
    pub reserve_target_sats: u64,     // for transparency
    pub reserve_current_sats: u64,    // for transparency
}
```

### Tests

- Server sets richness = 0.7; serialised packet roundtrips correctly.
- Below-threshold richness (< 0.25) gets clamped to 0.0 — matches Spec 6 §7.2 low-water-mark behaviour.
- Visual quantisation: client converts continuous richness to discrete tier (4 tiers: empty / thin / healthy / fat).

### Acceptance

- Server emits `reserve_richness` on state updates.
- Client reads it. Test packet roundtrip.
- `check.sh` clean.

---

## Phase 3 — Visual variants

> **Status (2026-05-20): FULLY DELIVERED.** Phase 3a on `feat/deepslate-visual-variants` (helper + parity test + audit); **Phase 3b on `feat/deepslate-phase-3b`** — BlockDefs registered for ids 61-63, 3 procedural textures (layers 174-176: subtle-blue-speckle THIN → blue-orange-veined HEALTHY → glowing-dense FAT), `texture_count` 174→177, `pick_deepslate_variant` in `biome.rs` with triangular weighting per tier (r=0 → 100% canonical; r=1/3 → 100% THIN; r=2/3 → 100% HEALTHY; r=1 → 100% FAT; smooth mix between), wired into `base_rock_at`. `BiomeGenerator.reserve_richness` synced from `GameState.reserve.richness` at top of `tick()`. Mine_drop normalises all variants to canonical PURE_DEEPSLATE. 6 new tests (helper-matches-all-four, mine-drop normalisation, picker zero/one/half richness, determinism). PROTOCOL unchanged. Phase 6 = Axolittle playtest.
>
> **Original Status (2026-05-17): DEFERRED — not blocked.** Phases 2, 4, 5 deliver the protocol + gauge + dialog. Phase 3's value (visual richness in the world itself) is real but the implementation risk is meaningful: a `grep -n PURE_DEEPSLATE` across the engine returns ~20 call sites (mining-break-time, drop tables, tool-tier gates, biome gen, proof-of-play deepslate detection, save format). Adding 4 variants without compromising the mining-mechanic-parity invariant ("strike on Fat tile produces same HMAC roll as strike on Empty tile at same position") requires either: (a) a helper `is_pure_deepslate_any_tier(block)` that every existing site adopts, or (b) a runtime-variant overlay on a single block ID via per-position metadata at mesh time. Both are 1-2 sessions of careful work. Defer until the gameplay-parity test surface is strengthened first.
>
> **What the alpha gets without Phase 3.** The gauge (Phase 4) + dialog (Phase 5) still surface the Reserve concept to the player. The visual treatment of deepslate is unchanged from today — players see "Healthy 75%" on the F3 gauge but the rock looks identical. Phase 6 playtest will tell us whether the gauge-only delivery is sufficient or whether the visual layer is the load-bearing one.
>
> **Resumption checklist for a future session:**
>
> - Decide variant strategy (4 block IDs vs runtime-variant overlay). 4-IDs is simpler but bloats the registry; overlay is cleaner but needs mesh-time per-position lookup.
> - Audit all 20 PURE_DEEPSLATE call sites; introduce `block::is_pure_deepslate_family(id) -> bool` helper.
> - Add gameplay-parity regression test: identical HMAC roll for the same position across all tiers (Spec 6 §2.2 — server_secret + position fixed, only block variant changes).
> - Save-format migration test: existing saves with `PURE_DEEPSLATE` (id 25) load correctly and don't show as "unknown block".
> - Then implement variants per spec body below.

### Goal

Render deepslate at depth (Y ≤ -21 per Spec 6 §2.2c) with one of four texture variants chosen by the server's richness signal:

| Tier | Richness range | Visual treatment |
|------|---------------|------------------|
| **Empty** | 0.00–0.25 | Plain dark grey deepslate. No veins. |
| **Thin** | 0.25–0.50 | Subtle blue-tinted speckling. Faint glints. |
| **Healthy** | 0.50–0.75 | Visible blue-orange veins (Satori-adjacent palette). Hints of warmth. |
| **Fat** | 0.75–1.00 | Dense, glowing veins. The rock looks *alive* with potential. |

The mining mechanic is unchanged — the Proof-of-Play HMAC roll on each strike is unaffected by visual tier. A "fat" tile and an "empty" tile have the same hash distribution; the visual signal is purely communicative: *more is available right now*.

### Implementation outline

1. Add a `BlockVariant` enum or per-block `richness_tier: u8` field on deepslate blocks. Variants share the same gameplay identity (still `Block::Deepslate`, still mineable identically); only texture coordinates differ.
2. In chunk-gen, when emitting deepslate at depth, query the server's current `reserve_richness` and pick variant. Cache richness-at-chunk-gen-time so subsequently-loaded chunks reflect the value at the time they were generated.
3. Texture array gets 4 new deepslate textures (16×16 default per CLAUDE.md asset spec).
4. On richness-tier change (every 0.25 boundary cross), broadcast a "redecorate" hint — newly-streamed chunks pick up the new tier. Already-loaded chunks update on next stream cycle (chunk eviction / re-load).

### Tests

- Block-variant rendering: assert each tier maps to the correct texture array layer.
- Chunk-gen at richness=0.7 produces "healthy" variants in the deepslate band.
- Tier boundary edge cases: richness=0.25 maps to "thin" (not "empty"); richness=0.50 maps to "healthy" (not "thin"); etc.
- Mining mechanic unaffected: strike on "fat" tile vs "empty" tile produces same HMAC roll given same world position (regression test).

### Acceptance

- Visual variants render at depth.
- Server richness change propagates to newly-loaded chunks within one stream cycle.
- Mining mechanic identical regardless of tier.
- `check.sh` clean.

---

## Phase 4 — HUD richness gauge

### Goal

Replace (or augment) the raw "Pool Balance: 45,230 sats" line in the Bitcoin menu's Treasury Transparency Panel with a visual gauge + plain-language label.

### Design

Native + WASM, in the Bitcoin menu (keybind `B` per Spec 6 §6.5):

```
+============================================+
|       THE RESERVE                          |
+============================================+
|                                            |
|  [===============>            ]  Healthy   |
|  45,230 / 60,000 sats  (75% — Fat)         |
|                                            |
|  Mining rate this hour:  +14 sats / 1000   |
|                          digs              |
|                                            |
|  Last hour's sponsors:                     |
|    npub1abc...xyz (5,000 sats)             |
|    npub1def...uvw (1,000 sats)             |
|                                            |
|  [Fund the Reserve]  [View Audit]          |
+============================================+
```

- **Gauge bar** — coloured per tier (dim grey → cool blue → warm orange → bright gold).
- **Plain label** — "Empty" / "Thin" / "Healthy" / "Fat". Kid-readable.
- **Mining rate** — the operational consequence of the current richness (Spec 6 §7.2 multiplier × payout-per-1000-digs baseline).
- **Recent sponsors** — opt-in (sponsors can hide their pubkey at funding time). Encourages public sponsorship as a social signal.
- **Action buttons** — link to Fund the Reserve (§5 below) and the audit log (Spec 6 §8.6).

### Tests

- HUD render at each tier with snapshot tests (egui doesn't have easy snapshot testing; verify the tier-label-and-colour pairings via unit tests on the gauge-state function).
- Recent-sponsors list respects opt-out flag.
- Mining-rate text matches the underlying §7.2 multiplier math.

### Acceptance

- The Bitcoin menu's pool section renders the gauge + label.
- Sponsor list correctly attributes recent funding events from audit log.
- `check.sh` clean.

---

## Phase 5 — Fund-the-Reserve dialog

### Goal

The actual UX moment when a parent / player chooses to fund. This dialog has to do the most communicative work in the whole feature — it's the moment of "I'm spending real money, what am I getting?".

### Flow

When the player clicks "Fund the Reserve" in the Bitcoin menu:

1. **Amount selection** — preset chips (100, 500, 1000, 5000 sats) + custom amount field. Default: 1000.
2. **Split breakdown** — visualised as a horizontal stacked bar:
   ```
   [== Pool 50% ==][= Creator 30% =][Platform 15%][Reserve 5%]
   ```
   With sats values below each segment. Hover/tap shows the recipient's Lightning Address + Nostr identity (per §5.4 amended).
3. **Confirmation copy** — text block, kid-readable:
   > "You're sponsoring this world. Your contribution makes mining more rewarding for **every** player here. You don't get sats back to your own wallet — you earn yours by playing, the same as everyone else. Your sponsorship is public (unless you opt out). Thank you."
4. **Opt-out checkbox** — "Show my contribution anonymously" (defaults off; sponsorship is public by default).
5. **Confirm + pay** — generates a BOLT11 invoice, displays QR + LNURL, player pays from their Lightning wallet.
6. **On settlement** — show a celebration moment ("You sponsored [server] with 1000 sats! The Reserve just got fatter — every player will mine a bit more this hour."), update the gauge in real-time, log the Nostr-signed "Sponsor of [server]" credential to the player's identity.

### Native vs WASM parity

The flow is identical on native and WASM. The only difference is the underlying Lightning wallet bridge: native uses LNbits BOLT11 + QR; WASM uses the same BOLT11 + QR + (optionally) WebLN if available.

### Per-server policy

Server operator can customise:
- Confirmation-dialog copy (locale, age-appropriateness).
- Preset chips (100, 500, 1000, 5000 by default; server can override).
- Whether sponsor-public is the default (default yes; some operators may prefer privacy by default).

### Tests

- Split visualisation: assert each segment width matches its percentage given a configured split.
- Amount validation: minimum 100 sats (Spec 6 §1.4 fractional-sat avoidance); maximum per-tx cap from operator config.
- Identity tooltip: each recipient segment shows the §5.4 identity binding.
- Sponsor-public flag: when set, the audit log entry includes the funder's pubkey; when unset, it's stored as "anonymous" (server-side cryptographic commitment retained for fairness, but not displayed).

### Acceptance

- Native + WASM both render the dialog correctly.
- Sponsorship completes end-to-end against a test LNbits instance.
- Audit log records the event with correct attribution flag.
- HUD gauge updates within one tick of settlement.
- `check.sh` clean.

---

## Phase 6 — Axolittle playtest

### Goal

Validate the fiction layer. Does an 11-year-old actually read "the rock is full of sats" from the visual treatment? Does the Fund-the-Reserve dialog feel like sponsorship rather than pay-to-win?

### Pre-playtest setup

- Test server running at varying richness levels (operator manually adjusts pool balance to hit each tier).
- Test Lightning wallet with ~10K test sats funded for sponsorship attempts.
- Axolittle in a flow where he's mining for ~15 minutes and sees the deepslate variants in situ.

### What to look for

- **Reading the rock.** When the richness changes from "thin" to "fat", does Axolittle notice without being told? Does he change his behaviour (mine more aggressively, or stay in deepslate areas longer)?
- **Mining-rate intuition.** Does the gauge label ("Healthy", "Fat") match how he describes the mining feel? If the gauge says "Fat" but mining feels slow, the calibration is off.
- **Funding flow read.** Walk him through the dialog. Does the message land that he's not buying his own sats? Does he understand the per-player vs per-pool distinction?
- **Aesthetic.** Are the visual tiers distinct enough? Or do "thin" and "healthy" blur together?
- **Sponsor list.** Does seeing other players named as sponsors make him want to sponsor? Or does it feel like a leaderboard?
- **Edge cases.** What if richness drops mid-session (server pays out faster than it gets funded)? Does the visual "fade" feel right?

### Outcome

Tuning notes. Likely follow-ups:
- Adjust tier boundary thresholds (maybe 0.20/0.40/0.60/0.80 instead of 0.25/0.50/0.75).
- Iterate the gauge label words (maybe "Quiet/Steady/Rich/Brimming" instead of "Empty/Thin/Healthy/Fat"; ask Axolittle).
- Adjust sponsor-public default.
- Refine dialog copy.

---

## Open questions

- **Should mining rate scale linearly with richness, or step at the tier boundaries?** Spec 6 §7.2 currently uses a linear interpolation from `low_water_mark_pct` to `warning_water_mark_pct`. The visual tiers are discrete, which might create a perception mismatch ("the rock just turned 'Fat' but I'm not earning any more"). Resolution: keep mechanical linear scaling, communicate the discrete tier purely as a *visual label for a continuous gauge*. Test in playtest.
- **What about Bitcoin-disabled servers?** They have no reserve, no funding flow. Show the gauge as "(no Bitcoin layer on this server)" or hide it entirely? Probably hide; mention only in server-info panel.
- **What about no-internet single-player worlds?** Same as Bitcoin-disabled — no reserve, no visuals. The "Educational HMAC" still shows (Spec 6 design — players see proof-of-work hash on each strike without sats translation).
- **What about Satori veins?** Spec 6 §2.2c.2 has Satori (special block) generation deep in deepslate. Those visuals already exist (orange-gem palette). Do they layer on top of Reserve richness? Decision: yes — Satori veins are extra-special spawns that appear *regardless* of richness (they're separate generation, per §2.2c). They appear *more often* in fat reserves because the richness multiplier affects the per-strike payout, but the visual placement of Satori is independent.
- **What about cosmetic-only servers (no payouts but full visuals)?** Imagine an experimental "sandbox with full Bitcoin UI but no actual Lightning". For developer testing. Operator config flag `bitcoin_visuals_only = true`. Renders everything visually but skips actual settlement. Useful for development; not part of alpha.

---

## Memory rule check

- proof of play is proof of work — visual richness affects *display* of pool, not mining mechanic. PoP determinism unchanged. ✓
- bitcoin parent controlled — feature opt-in per server, hidden on Bitcoin-disabled servers. ✓
- shared infra strategy — Reserve-as-visible-pool primitive lifts cross-game. Generic. ✓
- uk english naming — "Reserve", "Fund", "Sponsor" — UK English. ✓
- autonomy to playtest boundary — Phases 2–5 solo, Phase 6 = Axolittle. ✓
- economies vision — the Reserve fiction is the §11.1 Proof-of-Play floor made visible. ✓

---

## Out of scope (explicitly)

- New mining mechanics — the per-strike HMAC roll is unchanged.
- New economic policy — split percentages, work-meter rates, pool sustainability math all live in Spec 6 §5, §3, §7.
- Cross-server Reserve visualisation — each server has its own Reserve; federation between server Reserves is post-alpha + needs its own spec.
- Per-player visual variance — every player sees the same Reserve; this is a server-state-derived feature.
- Real-time chunk regeneration on funding — newly-loaded chunks reflect the new state; already-loaded chunks update on stream cycle, not instantly.
- "Bitcoin only" servers that omit the visual layer entirely — operators who don't want the in-world fiction can disable Reserve visuals via config; Spec 6 §6.2's "Fund the Reserve" UX still works without the visual layer.

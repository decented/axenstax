# Plot Ownership (v1) — first land economy primitive

**Status:** Phases 2-10 DELIVERED 2026-05-23 on `feat/plot-ownership`. Phase N = Axolittle playtest gate (open). Build also fixed a latent cross-spec bug: the Bounty Board / Tip Jar / Repair Bench item-materials were missing from `material_as_placeable_block`, so they crafted but couldn't be placed — now corrected (PlotMarker included).
**Branch:** `feat/plot-ownership` off `main`.
**Trigger:** Goal-driven solo dev cycle, Round 4. From `docs/vision/economies-long-run.md` §8.1 — "Plot Ownership (T2 — first land economy spec)". The economies vision flags it as parallelisable with Mob Bounty Board (different files). v1 is deliberately **minimal**: claim a region + build-protection enforcement. The fuller rule set (PvP toggle, mob-spawn toggle, vendor-only zones) + deed resale are explicitly v2.

---

## TL;DR

A new **Plot Marker** block (id 127) claims a fixed-size square region (16×16 columns, full height) centred on the column it's placed in, owned by the placing player. Once claimed, **non-owners cannot place or break blocks inside the plot** (anti-grief — the single highest-value land rule). The owner + any creative-mode player bypass the gate. Breaking the marker releases the plot. Overlapping claims are rejected at place-time (a marker can't be placed if its region would overlap a *foreign* plot).

Plot state lives in `World.plots: Vec<PlotData>` (linear scan — fine at alpha plot counts, mirrors the `salt_licks` / `village_anchors` posture). Persisted via `WorldSave.plots`. Enforcement hooks into the two `game_loop` decision points the Round-4 audit mapped: the break gate at the combined anti-grief guard, and a hoisted place gate before the place-dispatch.

1 new BlockId (127), 1 new MaterialId (`PlotMarkerItem`), 1 new module (`plot.rs` — pure region/ownership helpers), 1 new recipe (4 IronIngot corners + 1 OAK_PLANKS centre — a "boundary post" shape, distinct from existing recipes), 1 procedural texture. **No new UI** (v1 is enforcement-only — no dialog; the marker just works). PROTOCOL_VERSION 33 → 34. **Estimated ~700 LOC including tests.**

Independent of every other open spec. Touches block/item/crafting/save/world/game_loop + a new `plot.rs`.

---

## Why this lives here

- **First land-economy primitive.** Players can now own space, not just objects. Anti-grief build-protection is the foundational land right — every richer land rule (tolls, rentals, speculation) builds on "this region is mine".
- **The most-requested kid feature, implicitly.** A child who builds a castle wants it safe from a sibling/another player flattening it. Build-protection is that, concretely.
- **Cross-game lifts.** The region-ownership + place/break gate is engine-generic; any Decented multiplayer game with shared builds wants it.
- **Foundation for Market-Stall Rentals (8.2), Real-Estate (8.3), Land Tolls (8.4).** All extend the `PlotData` region + ownership model.
- **Completes the economy quartet this session.** Rounds 1-3 shipped combat/spectator sources + the first sink; land ownership is the substrate the markets economy eventually rents against.

---

## What this PR ships

### 1. `PLOT_MARKER` block (id 127)

| ID | Block | Mining behaviour | Notes |
|---|---|---|---|
| 127 | `PLOT_MARKER` | Owner (or creative) breaking it **releases the plot** + drops `PlotMarkerItem`. Non-owner break is refused (the marker is inside its own plot, so the plot gate already protects it). | Boundary-post texture (striped survey stake). |

**Recipe (3×3):**
```
I . I
. P .
I . I
```
4 IronIngot corners + 1 OAK_PLANKS centre → 1 PLOT_MARKER. Corner-iron shape is distinct from every existing recipe (furnace ring, vendor/bounty/tip centre-fills, repair-bench top-row, tool patterns).

### 2. `plot.rs` — pure region + ownership helpers

```rust
/// Half-extent of a claimed plot in columns (16 → a 33×33 region
/// centred on the marker… or 16×16 — pick at build time; spec
/// assumes a 16-column half-extent → 32×32 footprint). Server-tunable.
pub const PLOT_HALF_EXTENT: i32 = 16;

/// Owner of a plot. LocalPlayer(pidx) on alpha; Npub(String) variant
/// declared for the Spec-1-Phase-4 cutover (same migration as
/// VendorOwner / TipJarOwner — the three should converge together).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlotOwner { LocalPlayer(usize), Npub(String) }

/// One claimed plot. Axis-aligned square in the XZ plane, full height.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlotData {
    pub owner: PlotOwner,
    pub marker: (i32, i32, i32),  // the marker block position (claim anchor)
    pub min_x: i32, pub max_x: i32,
    pub min_z: i32, pub max_z: i32,
}

impl PlotData {
    pub fn from_marker(owner: PlotOwner, mx: i32, my: i32, mz: i32) -> Self;
    pub fn contains_column(&self, x: i32, z: i32) -> bool;
}

/// Is (x, z) inside a plot owned by someone OTHER than `pidx`?
/// The build-protection predicate. Creative bypass is applied by the
/// caller (creative players ignore the gate entirely).
pub fn is_in_foreign_plot(plots: &[PlotData], x: i32, z: i32, pidx: usize) -> bool;

/// Would a new plot anchored at (mx, mz) overlap any FOREIGN plot?
/// Used at place-time to reject conflicting claims. Overlap with the
/// placer's own plots is allowed (a player can claim adjacent land).
pub fn claim_would_conflict(plots: &[PlotData], owner: &PlotOwner, mx: i32, mz: i32) -> bool;

/// True iff this PlotOwner is the given local player.
pub fn is_local_owner(owner: &PlotOwner, pidx: usize) -> bool;
```

### 3. `World.plots: Vec<PlotData>` + accessors + WorldSave round-trip

- `world.plots` runtime side-table; linear scan (alpha-scale).
- `world.plot_at_column(x, z) -> Option<&PlotData>`, `world.release_plot(marker_pos)`.
- `WorldSave.plots: Vec<PlotData>` (`#[serde(default)]`). `PlotData` is plain serde — no separate `Saved*` mirror needed (it's already a flat struct; mirrors the `SavedBounty` simplicity).

### 4. Place-time claim + enforcement wiring in `game_loop.rs`

**Claim (place a PLOT_MARKER):**
- After the marker block is placed, `claim_would_conflict` check: if it overlaps a foreign plot, refuse (don't place the marker, toast "Too close to another player's plot"). Otherwise push a new `PlotData` to `world.plots` + toast "Plot claimed — 32×32 protected."

**Build-protection (per the Round-4 audit's hook points):**
- **Break gate** at `game_loop.rs:~2868` — compute `let plot_blocked = !self.is_creative && plot::is_in_foreign_plot(&self.world.plots, pos[0], pos[2], pidx)`; add `&& !plot_blocked` to the existing combined anti-grief guard (covers creative + survival break automatically). Non-owner break → toast "This land belongs to another player."
- **Place gate** — hoisted guard just after the place-target resolves (`game_loop.rs:~3540`): if `!self.is_creative && plot::is_in_foreign_plot(...)` for the place column, toast + skip the entire place-dispatch (covers normal place + eraser + hoe + seed + papyrus/salt writes that the audit flagged).

**Release (break own PLOT_MARKER):**
- When the broken block is `PLOT_MARKER` and the breaker owns it (or is creative), `world.release_plot(pos)` removes the `PlotData` + drops `PlotMarkerItem`.

### 5. `/give` aliases (`plot_marker`, `plot`, `claim`).

### 6. PROTOCOL_VERSION 33 → 34 + handshake history line.

### 7. Test surface

Pure tests in `plot.rs`:
- `from_marker_centres_region`
- `contains_column_inside_and_edges`
- `is_in_foreign_plot_true_for_other_owner`
- `is_in_foreign_plot_false_for_own_plot`
- `is_in_foreign_plot_false_outside`
- `claim_would_conflict_rejects_overlap_with_foreign`
- `claim_would_conflict_allows_overlap_with_own`
- `is_local_owner_matches_pidx`

Integration tests in `test_integration/plot.rs`:
- `claim_pushes_plot_and_protects_region`
- `non_owner_blocked_owner_allowed`
- `release_on_marker_break_removes_plot`
- `plots_round_trip_save_load`
- `creative_bypasses_plot_gate`

---

## Out of scope (v2 — post-playtest)

- **Per-plot rule toggles** (PvP on/off, mob-spawn on/off, vendor-only, public-build). v1 = a single hardcoded rule: non-owners can't build/break. The rule-config UI is v2.
- **Deed item + resale.** v1 = the marker IS the claim; breaking it (as owner) releases. A tradable deed that transfers ownership without re-placing is v2 (routes through Vendor Block per the markets economy).
- **Two-corner / resizable plots.** v1 = fixed 32×32 centred on the marker. Variable bounds are v2.
- **Plot caps / anti-monopoly** (per-player plot limits). v1 = unlimited; alpha plot counts are tiny.
- **Per-plot entity rules** (mob-spawn suppression inside plots). v2.
- **Plot Marker UI / info panel.** v1 = enforcement-only, no dialog. A "whose plot is this?" panel is a nice v2.

---

## Known limitations (documented BRIDGEs)

1. **`PlotOwner::LocalPlayer(pidx)` has the split-screen → solo-reload edge case** — identical to Vendor Block + Tip Jar. A plot claimed by Player 2, reloaded solo, would have an owner nobody matches → the plot stays protected but nobody can edit/release it (worse than the Tip Jar case, since the land is permanently locked). **BRIDGE:** `// BRIDGE: PlotOwner::LocalPlayer(pidx) locks the plot under split-screen → solo reload. Replace with Npub when Spec 1 Phase 4 lands — converge with VendorOwner + TipJarOwner.` Mitigation for alpha: creative-mode players bypass the gate, so an admin/parent in creative can always clear a stuck plot.
2. **No vertical bounds.** v1 plots are full-height columns. A plot owner controls bedrock-to-sky in their footprint. Layered ownership (someone owns the surface, another the caves) is out of scope.
3. **Enforcement is client-side** (single-player + the placing client). Per CLAUDE.md dual-sim debt — server-authoritative plot enforcement waits on Spec 2. Acceptable for alpha (solo + trusted-LAN).

---

## Implementation phases (per-phase green commits)

| # | Phase | Files | Est LOC |
|---|---|---|---|
| 1 | Spec + plan docs | docs/ | 0 |
| 2 | BlockId + texture + BlockDef + MaterialId + recipe + /give | `block.rs`, `texture_gen.rs`, `item.rs`, `crafting.rs`, `commands/builtins/give.rs` | ~120 |
| 3 | `plot.rs` module — PlotOwner, PlotData, predicates + 8 pure tests | new `plot.rs` | ~220 |
| 4 | `World.plots` + accessors + WorldSave round-trip | `world.rs`, `save.rs` | ~70 |
| 5 | Claim-on-place + conflict rejection + release-on-marker-break | `game_loop.rs` | ~80 |
| 6 | Build-protection: break gate (line ~2868) + hoisted place gate (line ~3540) | `game_loop.rs` | ~50 |
| 7 | PROTOCOL 33 → 34 + handshake history | `protocol.rs`, `test_integration/handshake.rs` | ~10 |
| 8 | Integration tests | new `test_integration/plot.rs`, `mod.rs` | ~180 |
| 9 | `./check.sh` ALL GREEN | — | — |
| 10 | Doc-flip + queue README | docs/ | 0 |
| 11 | Merge to main --no-ff | — | — |
| N | Axolittle playtest — plot size feel, claim/release UX, "do I want rule toggles", does build-protection feel fair | n/a | n/a |

**Phases 2–10 solo-buildable.** Phase N = playtest gate.

---

## Acceptance criteria (solo)

- PLOT_MARKER craftable, placeable; placing it claims a 32×32 region owned by the placer.
- Placing a marker that would overlap a foreign plot is refused.
- A non-owner (survival) can't place or break inside the plot; gets a toast.
- The owner can build/break freely inside their own plot.
- A creative-mode player bypasses the gate entirely.
- Breaking your own marker releases the plot (subsequent edits by anyone allowed).
- Save → reload → plots + ownership survive.
- `check.sh` ALL GREEN. PROTOCOL 34.

---

## Memory-rule check

- ✓ economies vision — first land economy spec, §8.1.
- ✓ bitcoin parent controlled — v1 has no sats flow (pure ownership/anti-grief); the rental/toll sats layers are v2 and will route through `apply_sats_payout`.
- ✓ proof of play is proof of work — no chance, no payout; orthogonal.
- ✓ uk english naming — "Plot Marker" / "claim" (UK-natural; not "lot").
- ✓ alpha open access — no whitelist; anyone can claim.
- ✓ shared infra strategy — region-ownership + place/break gate are engine-generic.
- ✓ autonomy to playtest boundary — solo through Phase 10; Phase N = playtest.
- ✓ merge to main preauthorised — healthy-gate merge.

---

## Cross-spec interactions

- **Vendor Block + Tip Jar** — share the `LocalPlayer(pidx)` owner model + its reload edge case. The Npub migration should converge all three (+ now Plot Ownership).
- **Spec 19 Villages / procgen** — villages are server-generated, NOT plot-claimed; they don't collide. (A future rule could let players claim *around* a village; out of scope.)
- **Build Schematics (Spec 24-27)** — players building from plans inside their own plot is fine; building in a foreign plot is blocked by the same place gate. No special-casing needed.
- **Salt Path / Snowfall / crop growth** — these are world-system writes (not player place/break), so they're NOT gated by plot ownership (snow can still fall on a claimed plot). Intentional — environment isn't "building".

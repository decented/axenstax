# Build Schematics — Long-Run Master Design

**Status**: VISION DOCUMENT — establishes the end-state. Per-phase foundation specs derive from this.
**Date**: 2026-05-19
**Author commit**: Captures Staxolottle's 2026-05-19 design call ("an architect's plan economy that lets the community contribute village content") and frames it as the **C7 — Build Schematics** lane of the Knowledge economy in `economies-long-run.md §9` + the replacement for `village_gen.rs::build_house`'s hardcoded shapes.
**Companions**: `docs/vision/economies-long-run.md` §9 (Knowledge & Creator Economy); `docs/foundations/2026-05-18-villages-and-villagers.md` (Spec 19 — provides the village shell + villager + Plaque-protection ownership model); `docs/foundations/2026-05-18-vendor-block.md` (Spec 21 — the trade primitive that plans plug into); `docs/foundations/2026-05-18-furnace.md` (Spec 20 — `apply_server_tax_and_payout` helper that every plan-related sats flow routes through).
**Cross-refs**: Memory axenstax has farming (Papyrus reed is sequenced alongside T1 farming work), bitcoin parent controlled (outgoing-sats Charter gating on tips + commissions), shared infra strategy (registry + plaque + commission pattern lifts cross-game), uk english naming, proof of play is proof of work.

---

## 0. TL;DR

Players capture buildings into **Plan** items — reusable, tradeable, license-tagged blueprints — and other players (or NPC Builders) construct from them. The engine also samples Plans from a curated registry when spawning villages, replacing today's hardcoded `build_house` with community-authored content. Architect attribution is **structural** (an in-world Plaque carries the full derivation chain) and **monetisable** (each link in the chain can be tipped sats directly).

Six phases describe the end-to-end lifecycle:

1. **Author** — lay plan tiles → build on top → right-click any tile to capture → get a Plan item.
2. **Inspect** — preview the captured plan + see its quantity-surveyor material list.
3. **Trade** — sell Master plans (full IP transfer) or mint Licences (per-copy use rights) through Vendor Blocks. Save-As derivatives are first-class. Four license tiers (All Rights Reserved, CC-0, CC-BY-SA, CC-BY-ND) with **CC-BY-SA as the platform default**.
4. **Player-Self-Build** — ghost preview + animated auto-build (1–2 blocks/tick).
5. **NPC Builder** — commission a Builder villager: bring a plan + materials + sats, and they walk to your chosen site and construct it.
6. **Village Procgen** — engine samples plans from the registry (engine-bundled curated set + server-local additions; Nostr-decentralised v2) when generating villages. Replaces `build_house`.

**Locked design positions** for v1:
- Plans are **strict templates** — exact blocks at exact relative positions.
- Plans are **reusable forever** — the use-right doesn't expire.
- Three-tier IP model: **Master** (full rights), **Licence** (use-only), **Derivative Master** (Save-As result with `derived_from` citation).
- **Plaques carry the full derivation chain** in-world; each architect in the chain is tippable.
- **Flat terrain only** for v1; slopes deferred to v2 (auto-flatten + stilt-pillar).

**Cross-game lift** is substantial. The registry framework, license-tagged content metadata, derivation chain, plaque-attribution pattern, commission-NPC primitive, and `apply_server_tax_and_payout` integration all generalise to any voxel game with player-authored content (nest patterns, constellations, recipe stations in other games on the same primitives).

---

## 1. Where this fits

### 1.1 Replaces today's hardcoded village content

Currently `game/engine/src/village_gen.rs::build_house` paints the same boxy shape every time, and `village_gen.rs::HOUSES_MIN..HOUSES_MAX` rolls 3–6 of those identical houses per village. Tonally unfinished; "really poor quality" per the original design call. After Phase 6 ships:

- Each house slot samples from the **Plan Registry** instead of calling `build_house`.
- Every building gets a **Plaque** crediting its architect — visible to players walking the village.
- Village quality scales with community contributions, not engine-team output.
- The engine team escapes the trap of having to author every village shape by hand.

### 1.2 Names the existing Knowledge-economy lane

`economies-long-run.md §9` already calls out a knowledge-economy IP model:

> DRM is impossible in voxel — once a player makes a recipe, the visuals/inputs are visible. Knowledge economies sell **first-discovery credit** + **shared-tooling** more than IP rights.

`economies-long-run.md` row **C7** is titled "Build Schematics (T3): Save + sell builds." This vision doc is exactly that, fleshed out.

### 1.3 Composes with five existing primitives

This is not a green-field design — it composes with infrastructure that's already specced or shipped:

- **Vendor Block** (Spec 21) is the trade primitive. Plans become tradeable items via Sell-Master and Sell-Licence sub-modes.
- **Villages + Villager profession system** (Spec 19) provides the social shell. Builder is a new profession; Drafting Table is a new workstation.
- **Furnace's `apply_server_tax_and_payout` helper** (Spec 20) is the sats-flow hub. Every plan-related sats transaction — sale, tip, commission — routes through it (server tax + Reserve drain + recipient payout).
- **Charter** (Spec 10) parent-controls outgoing sats. Tips + commissions from kid accounts hit the Charter gate, same as any other sats flow.
- **Reserve drain** (Spec 16) takes its slice of every plan-related sats flow, same as every other economy. No special treatment.

### 1.4 Companion to the farming + economies long-run docs

This sits alongside `farming-economy-long-run.md` and `economies-long-run.md` as the third pillar of the long-run design layer:

- `farming-economy-long-run.md` → what players grow + cook + age over time.
- `economies-long-run.md` → how production translates to sats across seven economies + the Proof-of-Play floor.
- `build-schematics-long-run.md` → how players' creative output (buildings) translates to sats + becomes shared community content.

### 1.5 Creative and survival are equal first-class authors

Plans work identically across game modes. **Architect-at-the-CAD-station** is the mental model — the design IS the deliverable. A creative-mode author skips material consumption when *building from* a plan in their own world (they have infinite blocks anyway), and the Inspect dialog renders the material list informationally rather than as an inventory check. Every other surface — capture, naming, licensing, derivation, plaque attribution, trade, NPC commission, procgen — is mode-agnostic.

`PlanData` carries an `authored_in: GameMode` field. The Plaque dialog renders this as a badge near the root architect's name; Vendor Block listings (§5.2) surface it so survival buyers can choose to filter. **Tag, don't gate.** There is no one-way transfer restriction between modes.

**Why no restriction:** a plan is a template, not a teleport. A survival player who buys a creative-authored plan still has to gather every block to build it; creative authorship doesn't shortcut the survival economy. Restricting creative→survival would penalise design as a form of labour (sketching the tavern is real work, arguably more skilled than chopping the logs), create a content-moderation nightmare (mode-switching mid-capture, derivation chains from mixed-mode ancestors), and break the cross-mode supply asymmetry that's the design's secret weapon — creative architects supplying survival builders is the same dynamic as real-world architecture firms supplying real-world construction crews. Plaque attribution + content-hash already give full audit. Show authorship; let policy filter; never gate.

**Where policy layers can hook in:**

- Per-server policy (Spec 6 §13 economy modes) can ban creative-authored plans from the Vendor Block trade flow on purist-survival servers.
- Village procgen registry (§8) can prefer or require survival authorship to keep village texture rough-hewn — TBD by that section.
- Buyer-side filter in Vendor Block (§5.2) lets survival players filter listings by `authored_in`.

None of these policies are mandatory. Default behaviour is full cross-mode interop.

---

## 2. Lifecycle overview

```
        ┌──────────────────────────────────────────────────────────┐
        │  PLAYER (Architect)                                      │
        │                                                          │
        │   1. lay plan tiles on flat ground                       │
        │   2. build on top                                        │
        │   3. right-click any tile → capture                      │
        │      ┌─→ dialog: name + license picker                   │
        │      │                                                   │
        │      ▼                                                   │
        │   📜 Plan item (Master)  ──────────┐                     │
        │                                    │                     │
        └────────────────────────────────────┼─────────────────────┘
                                             │
                ┌────────────────────────────┼────────────────────┐
                │                            │                    │
                ▼                            ▼                    ▼
        ┌───────────────┐         ┌────────────────────┐  ┌─────────────┐
        │  INSPECT      │         │  TRADE             │  │  VILLAGE    │
        │  (Phase 2)    │         │  (Phase 3)         │  │  PROCGEN    │
        │               │         │                    │  │  (Phase 6)  │
        │  preview +    │         │  Vendor Block:     │  │             │
        │  material     │         │  - Sell Master     │  │  Plan       │
        │  list         │         │  - Sell Licence    │  │  Registry   │
        └───────┬───────┘         │  - Save-As (B)     │  │  samples    │
                │                 │  + Plaque tipping  │  │  from CC-0/ │
                ▼                 └─────────┬──────────┘  │  CC-BY-SA   │
        ┌───────────────┐                   │             └─────┬───────┘
        │  PLAYER-SELF  │                   │                   │
        │  -BUILD       │                   │                   │
        │  (Phase 4)    │◄─────┐            │                   │
        │  ghost prev + │      │            │                   │
        │  animated     │      │            │                   │
        │  auto-build   │      │            ▼                   │
        └───────────────┘      │   ┌─────────────────┐           │
                               │   │  NPC BUILDER    │           │
                               │   │  COMMISSION     │           │
                               └───│  (Phase 5)      │           │
                                   │  Drafting Table │           │
                                   │  + Builder NPC  │           │
                                   │  + sats payment │           │
                                   └─────────────────┘           │
                                                                 │
        Every built structure ←──────────────────────────────────┘
        gets a Plaque with full derivation chain (see §5.6 below —
        Plaques are shared infrastructure used by Phases 4, 5, and 6)
```

Six phases. Phase 1 produces Plans. Phases 2–4 use them solo. Phase 5 uses them through an NPC. Phase 6 uses them at world-gen scale. All phases route sats flows through `apply_server_tax_and_payout`.

---

## 3. Phase 1 — Author

### 3.1 Plan Tile block

`BlockId::PLAN_TILE` — a new flat block laid on the ground. Pale parchment-coloured top, dark border on sides so it reads as a drafting board square from above.

**Recipes** (v1): per-paper-grade yield, mirroring real drafting-paper grades. Both 1×2 vertical: stick on top, paper below.

- **`1 stick + 1 PapyrusSheet → 9 Plan Tiles`** (T1 entry-grade; 3×3 floor-section yield).
- **`1 stick + 1 PulpPaper → 25 Plan Tiles`** (T1.5 mill-grade; 5×5 floor-section yield). Pulp Paper is a Mill output (Spec 12 T1.5 — sugarcane bagasse, see §3.4).

**Tiles are consumed by capture, never refunded** (see §3.6). Paper irreversibly transcribes into a plan — mirroring how a real architectural drawing commits its paper to the archive. A Save-As-style derivative needs fresh tiles (= fresh paper) just like the original capture did. This is the *can't-repurpose-paper* rule, and it makes the paper economy meaningful long-term.

**Scale check at 9 tiles/papyrus + 25 tiles/pulp:**

| Plan footprint | Tiles | @ Papyrus (9/sheet) | @ Pulp (25/paper) |
|---|---:|---:|---:|
| Hut (4×4) | 16 | 2 sheets | 1 paper |
| Cottage (7×7) | 49 | 6 sheets | 2 papers |
| House (10×10) | 100 | 12 sheets | 4 papers |
| Big house (15×15) | 225 | 25 sheets | 9 papers |
| Mansion (20×20) | 400 | 45 sheets | 16 papers |
| Castle (25×25) | 625 | 70 sheets | 25 papers |
| Max (32×32) | 1024 | 114 sheets | 41 papers |

Papyrus is the day-1 entry-tier — viable for small builds without Mill infrastructure but expensive at scale. Pulp Paper is the **T1.5 unlock** — once the player invests in a Mill, paper becomes ~2.78× more efficient and serious building plans become economical. The upgrade arc is the design's spine.

**Placement**: solid block on grass/dirt/sand, no special placement rules. Player lays a connected footprint by placing tiles adjacent to each other.

### 3.2 Connectivity check at capture time

The capture trigger validates that all PLAN_TILE blocks at the trigger position's Y-level form **one contiguous shape** by 4-connected flood-fill. If a player accidentally placed tiles in two disconnected groups, capture refuses with the toast *"Tiles must all be connected."* This keeps plans single-footprint (no surprise multi-building captures).

### 3.3 Flat-ground rule (v1)

Every cell directly above a tile must be `AIR` (no obstructions in the build volume). Every cell directly below a tile must be `SOLID` (the foundation is real). If either fails, capture refuses with a specific toast (*"Plot must be flat — clear obstructions or fill divots first."*).

Slope support deferred to v2 (§11).

### 3.4 Paper — historical tech tree

Paper is load-bearing for Plan Tiles AND for several downstream systems (books, maps, quest scrolls, charter clauses). Two historical tiers, each with its own source AND each with its own Plan Tile yield:

| Tier | Source | In-game tier | Workstation? | Plan Tile yield | Tonal feel |
|---|---|---|---|---:|---|
| **Papyrus Sheet** | Papyrus reed (water-adjacent plant) | T1 / Day-1 (earliest) | None — crafting table only | **9 tiles/sheet** | Egyptian, rough, the original paper |
| **Pulp Paper** | Sugarcane bagasse (Mill workstation) | T1.5 (`farming-tier-1.5`) | Mill / Pulper | **25 tiles/paper** | Industrial, abundant, paper as a sugar by-product |

**Why bagasse (not parallel sugarcane→paper like Minecraft):** Real-world sugarcane gives sugar from juice + bagasse from the fibrous residue. Bagasse → paper. This is how paper is industrially made in countries with sugarcane economies. Modelling it correctly:

- Justifies T1.5's processing workstations (Mill produces a real two-stream output).
- Teaches "nothing is wasted" by-product thinking.
- Distinguishes AxeNStax from Minecraft substantively.

```
Sugarcane ──┐
            ▼
     [Mill / Crusher]
       ╱           ╲
  Sugar-Juice    Bagasse (fibrous residue)
      │              │
      ▼              ▼
   [Boiler]    [Pulper / Paper Press]
      │              │
      ▼              ▼
    Sugar         Pulp Paper
```

**Plan Tile crafting forks per paper grade** — two separate recipes, different yields. The shared `is_paperish_slot` predicate (Spec 23) stays in use for OTHER paper-consumers (books, maps, quest scrolls, charter clauses) where the grade doesn't matter. Plan Tile specifically discriminates because the yield is the upgrade arc's main hook.

**Leather is explicitly NOT a paper source.** Leather is reserved for clothing / armour / saddles / belts. Diluting it would be wrong tonally.

### 3.5 Build envelope: connectivity flood-fill

Capture doesn't just snap a fixed bounding box. It runs a **3D 6-connected flood-fill upward from each tile-adjacent cell**, capturing every block reachable through the build. A torch the player left floating in the air as working light is NOT included (it's disconnected from the structure). Floating chandeliers / lanterns the architect *meant* to include need at least one chain-block connection — explicit design tradeoff that rewards correct construction.

Hard limits: max footprint 32×32 tiles; max captured height 32 blocks. The v1 "house-scale" envelope; megabuilds are out of scope.

### 3.6 Capture trigger

**Right-click any plan tile with empty hand.** All connected tiles vanish (set to AIR) — **no refund**. The paper-cost is locked into the resulting Plan, mirroring how real architectural drawings commit their paper to an archive. (Unused tiles, never confirmed-into-a-plan, can still be mined and relaid normally; only the capture step consumes them.) A 📜 **Plan item** drops at the trigger position.

Captured state stored as bincode-serialised `PlanData`:
- Name (architect-set at capture).
- `author_npub` (immutable; the architect's Nostr pubkey).
- `license: PlanLicense` (immutable per-Master; see §5.4).
- Footprint shape (W×D grid of "has tile" bits).
- Per-cell column of `(y_offset, BlockId)` entries — only non-air cells captured.
- `derivation_chain: Vec<DerivationLink>` (see §5.6).

**Multi-player ownership (deferred)**: in single-player, the player who right-clicks owns the resulting Plan. In multi-player, this becomes a real question — what if Player A right-clicks tiles Player B placed inside Player B's village? V1 ships single-player only, so this is a deferred multi-player concern. The multi-player phase will pin down a rule (e.g. "the tile-layer owns the capture" or "the world owner can override"). Until then, the engine just lets whoever right-clicks first take the Plan — fine for solo play.

### 3.7 Capture dialog

After a successful capture, a brief modal:

```
┌─ Plan captured ──────────────────────────────────┐
│  Name: [Riverside House                    ]    │
│                                                  │
│  License: [CC-BY-SA (default)              ▼]    │
│    ▸ All Rights Reserved — your IP, paid Licences   │
│    ▸ CC-0 — Public Domain, anyone can do anything   │
│    ▸ CC-BY-SA — anyone can use/modify, must credit │
│       + share-alike (DEFAULT)                       │
│    ▸ CC-BY-ND — anyone can use, credit, no modifs   │
│                                                  │
│  Materials: 12 Oak Planks, 4 Cobblestone, ...   │
│                                                  │
│  [ Cancel ]  [ Confirm Capture ]                │
└──────────────────────────────────────────────────┘
```

- **Name** pre-filled with `"Plan: <W>×<D>"`; architect can rename. Enter confirms.
- **License**: see §5.4 for the full picker semantics. CC-BY-SA is the default (per platform philosophy — see §5.5).
- **Material list** shown both at capture (so the architect immediately knows what they made) and in the Inspect dialog (for buyers — §4).
- **Cancel** restores the tiles and the build remains as-is. Player loses nothing.

Once confirmed, license + author + content are immutable. To change anything, the architect captures again (creating a new plan).

---

## 4. Phase 2 — Inspect

### 4.1 Plan item rendering

**Type model**: Plans are *not* a `MaterialId` variant — `MaterialId` is a fungible-token enum where every entry of a variant is interchangeable, which can't carry per-instance data. Plans need their own `Item` variant alongside `Item::Block`, `Item::Tool`, `Item::Material`:

```rust
pub enum Item {
    Block(BlockId),
    Tool(Tool),
    Material(MaterialId),
    Plan(PlanData),     // ← new
}
```

`PlanData` holds the embedded content + name + `author_npub` + license + derivation chain + content-hash. Inventory `ItemStack::can_stack_with` returns `false` for any `Item::Plan` pair (each plan is unique; never merges).

Hotbar / inventory rendering: scroll icon, dark-gold colour, plan name as the display label. Hotbar tooltip:

```
📜 «Riverside House» — 8×6×5, 47 blocks
   by alice.npub1q4vrz... · CC-BY-SA
```

### 4.2 Inspect dialog

Right-click while holding a Plan opens a modal dialog — **does not** immediately place anything in the world. Three regions:

#### Header
- Plan name + version (e.g. `«Riverside House» v3` for derivatives).
- `author_npub` truncated, full chain visible in derivation list below.
- License with mascot icon (cc / cc-by-sa-circle / cc-by-nd-circle).
- Footprint W×D×H.
- Total block count.
- "Derived from <original_architect_npub>'s <plan_name>" if the plan is a derivative.

#### Top-down 2D footprint preview
- ASCII-grid showing which cells are part of the plan footprint.
- Lets the player see L-shapes, +-shapes, and other non-rectangular footprints at a glance.
- (3D / isometric preview deferred — 2D top-down covers v1's "is this the building I want?" decision.)

#### Material list table
- Every distinct block type with required count.
- Player's current inventory count side-by-side.
- Colour-coded: `12 / 12 ✓ Oak Planks` (green), `4 / 8 ✗ Cobblestone (need 4)` (red).
- Sub-craftables aren't recursively expanded — the list shows the raw block (e.g. `12 Oak Planks`), not the chain (`3 Oak Logs → 12 Oak Planks`). Players figure out the chain via their existing crafting knowledge.

#### Action buttons
- **Place in world** — closes dialog, enters Phase-4 placement mode. Greyed if any row is `✗`.
- **Close** — dismisses.

### 4.3 Capture-time material list

The architect sees the same material list briefly at capture — so they immediately know what their plan needs without re-opening Inspect. Helps with pricing the plan for sale + with knowing whether to keep iterating.

---

## 5. Phase 3 — Trade (plan ↔ sats + licensing + derivatives + plaques)

This is the largest phase by surface area. Five subsections.

### 5.1 Two ownership tiers (three when derivatives count)

| Tier | What you can do | How you got it |
|---|---|---|
| **Master Plan** | Build forever. Mint Licences. Sell Master (full transfer). Save-As (derive). | Captured it yourself, OR bought as Master. |
| **Licence** | Build forever. **Cannot mint Licences. Cannot Save-As. Cannot sell as Master.** Can be physically given/dropped/picked up. | Bought from a Master-holder. |
| **Derivative Master** | Same rights as any Master — including further derivation. Carries an optional `derived_from` citation pointing at the original. | Captured by modifying a Master-built structure. Treated as a Master economically. |

"Master = IP itself; Licence = use-rights; Derivative Master = your own IP, citing the source." The Licence tier is what prevents knockoff distribution chains.

### 5.2 Vendor Block as the trade channel

Plans use Spec 21's Vendor Block. **No new economic primitive** — plans are just another tradeable inventory item with three sub-modes specific to plans:

#### Sub-mode 1: Sell Master (one-time IP transfer)
- **Slot**: Master plan.
- Sats price.
- Buyer pays → receives **the Master** in their inventory. Slot empties.
- Buyer becomes the new Master-holder + can mint Licences themselves + can resell as Master.
- Original `author_npub` stays with the Master forever (architect attribution survives any chain of resales).

#### Sub-mode 2: Sell Licence (per-copy, recurring)
- **Slot**: Master plan (stays — never leaves).
- **Slot**: Paper stock (papyrus sheets, or pulp paper post-T1.5). One per sale.
- Sats price per Licence.
- Buyer pays → receives a **Licence** (separate item; same `author_npub` as the Master; `tier = Licence` flag). Paper stock -1.

#### Sub-mode 3: Slot rejects a Licence
- Trying to slot a Licence into Sell-Master or Sell-Licence raises an error toast: *"Licences can be used, not relicensed. Capture a new plan to mint your own."*
- A player who bought a Licence can still **re-sell the physical Licence item** as a standard Vendor Block "Sell" (no new copies minted, ownership transferred). Engine sees this as moving the existing Licence, not minting.

### 5.3 Save-As (derivatives via the existing capture mechanic)

**There is no separate "Save As" action.** All edits + derivatives flow through the existing capture mechanic from §3:

1. Player holds Master, builds it via Phase 4, then modifies the result freely.
2. Player lays a fresh footprint of plan tiles around the modified building.
3. Right-click any tile with empty hand → capture dialog.
4. **Engine checks**: does the player hold a **Master** whose footprint + ≥50% block-set matches the in-world build? If yes, the dialog adds a checkbox:
   - ☑ *"Mark as derivative of <existing Master name>"* (default checked, citation set).
5. Player confirms → new Master created. Their `author_npub` is the author; `derived_from` field set if checkbox stayed ticked.

The original Master is untouched — captures never mutate existing plans. Each "version" is a separate Master with its own life. Existing Licences in the wild stay valid (they're immutable snapshots of the version they were minted from).

**Licence-tier holders cannot Save-As.** Engine refuses capture with content-hash match if the player holds only a Licence: *"You hold a Licence for a similar plan, not the Master. Derivatives need the Master."* This isn't bulletproof (player could build identical from memory + claim independent capture) — that's content policy, not engine policy. The structural guard catches the obvious abuse case.

### 5.4 Four licenses

```rust
pub enum PlanLicense {
    AllRightsReserved,
    CC0,
    CCBYSA,              // ← DEFAULT
    CCBYND,
}
```

| License | Build rights | Derive (Save-As) | Mint Licences | Attribution required | Derivatives must use… |
|---|:---:|:---:|:---:|:---:|---|
| **All Rights Reserved** | Licence/Master only | Master only | Master only | Structural | (n/a) |
| **CC-0** | Anyone | Anyone | Anyone | Optional | Any license |
| **CC-BY-SA** | Anyone | Anyone | Anyone | **Required** | **CC-BY-SA** (forced — share-alike; can't tighten to ARR or ND, can't drop to CC-0) |
| **CC-BY-ND** | Anyone (identical copies) | **Blocked** | Anyone (copies only) | **Required** | (n/a) |

**Why CC-BY-SA + not CC-BY:** CC-BY-SA forces derivatives to share the same license, preserving the architect's openness intent forever. CC-BY (without -SA) lets a derivative tighten to ARR or ND, which lets community work get fenced off downstream. CC-BY-SA protects against that.

**Why no CC-NC / CC-BY-NC variants:** Impossible to enforce in a game with Bitcoin built into the trade layer — every sat sale is "commercial." Skipping NC variants avoids licenses we can't honestly enforce.

### 5.5 CC-BY-SA as platform default

The capture-dialog license picker defaults to CC-BY-SA (with a per-player sticky preference once the architect picks any value). Three reasons this is the right default:

1. **Matches the platform's open-content philosophy.** AxeNStax is community-first, Nostr-cross-server, open-ecosystem. CC-BY-SA aligns. ARR creates artificial scarcity around community-build content; CC-BY-SA preserves credit while removing scarcity.

2. **Drives village-procgen content supply.** Plan-registry qualifying licenses are CC-0 or CC-BY-SA. If default is ARR, most plans don't qualify and the procgen well runs dry. If default is CC-BY-SA, the procgen registry fills up organically as the community plays — the village-quality flywheel spins.

3. **Kid-first ethos.** A kid building a fun house has no real business gating IP. Default permissive lets them participate in the community; their `author_npub` is preserved for credit.

Bitcoin economics aren't damaged — CC-BY-SA architects can still **sell Licences** through Vendor Blocks. The license only prevents license-tightening + requires attribution. The architect's monopoly pricing power is removed (someone could publish an alternative based on yours), but that's a feature in a community economy.

#### Surprise-factor mitigations
- **First-time onboarding modal** explains the default + how to change it.
- **Sticky per-player preference** — once a player picks any license, future captures default to that.
- **License always visible at capture** — never silently applied.

### 5.6 Plaque + derivation chain

`BlockId::ARCHITECT_PLAQUE` — a small parchment-on-wood block, hung inside every built structure. Distinct visual from regular signs.

**Auto-placed at build-time**. When any Plan is built (Phase 4 / 5 / 6), the engine places one Plaque at a sensible default position (most-central tile cell, on the floor, oriented toward the highest-numbered direction). Architect-placed plaques inside the captured volume override the auto-placement.

**Protected**: only the world-owner / world-admin (single-player: the player; multi-player: the host) can break it. Other players can right-click read only.

#### Derivation chain stored on the plan

```rust
pub struct DerivationLink {
    pub author_npub: String,
    pub plan_name: String,
    pub license: PlanLicense,
    pub captured_at: u64,    // unix-ish
    pub plan_hash: [u8; 32], // content hash for audit
}

// On PlanData:
pub derivation_chain: Vec<DerivationLink>,  // [original, v2, v3, ..., this]
```

Every Save-As prepends the parent's full chain + adds the new entry. Chain is immutable; new entries can only append. ~100 bytes per link; a 10-generation chain is ~1 KB.

#### Right-click plaque → attribution + tip dialog

```
┌─ Architect's Plaque ─────────────────────────────┐
│  «Riverside House v3»                            │
│  License: CC-BY-SA  ⓘ                            │
│                                                  │
│  Built by:  carol.npub1xx88... (this version)    │
│  Tip:       [1] [5] [50] [custom] sats   [Send] │
│                                                  │
│  Derivation chain:                               │
│  ├─ v3 by carol.npub1xx88...  (2026-06-15)      │
│  │   [tip carol →]                               │
│  ├─ v2 by bob.npub1qq22...    (2026-06-01)      │
│  │   [tip bob →]                                 │
│  └─ v1 by alice.npub1q4vrz... (2026-05-19, orig)│
│      [tip alice →]                               │
│                                                  │
│  License: CC-BY-SA (uniform across chain)        │
│                                                  │
│  [ Close ]                                       │
└──────────────────────────────────────────────────┘
```

When the licenses in a derivation chain are all the same (the common case for CC-BY-SA chains, since ShareAlike forces the license to inherit), the plaque dialog collapses the display to *"License: <X> (uniform across chain)"* rather than showing the same license repeated N times. Only when the chain is mixed (e.g. a CC-0 root with a CC-BY-SA derivative on top — allowed because CC-0 imposes no restrictions on derivative-license choice) does the full chain render: *"License chain: CC-0 → CC-BY-SA → CC-BY-SA"*. Keeps the common case clean.

- Every link in the chain is independently tippable.
- Tip target = each architect's Nostr profile's `lud16` Lightning address (cached via the platform's existing Nostr infrastructure).
- Tips route through `apply_server_tax_and_payout` (server tax + Reserve drain + recipient payout).
- Outgoing sats are Charter-gated per bitcoin parent controlled.
- License chain at the bottom shows the licensing trajectory — visible sanity check that no link tightened.

### 5.7 Sats split (v1)

Each sale + tip + commission routes through `apply_server_tax_and_payout`:

- **1% server tax** (per Spec 6 §13).
- **~1% Reserve drain** (per Spec 16).
- **Remainder** → recipient (`lud16` invoice).

For Master sales: 98% → seller (current Master-holder, may not be original architect).
For Licence sales: 98% → Vendor Block owner (current Master-holder).
For tips: 98% → the tipped architect's `lud16`.
For commissioned builds: 98% → Builder villager's village treasury (§7).

**Original-architect royalty on Licence/commission sales** deferred to v2 — needs cross-server identity work + playtest tuning.

### 5.8 Cross-server portability

**V1** — Plans are inventory items in the player's save. Joining a different server brings them with you. Plans work on any server (block IDs universal across AxeNStax instances).

**V2** — Nostr-signed plans. Architect signs a kind-X event with plan content hash + npub. Buyer can verify cryptographically. Plans become tradeable as Nostr events too. Marketplace listings cross-server-indexed. License-chain integrity provable.

### 5.9 Gifting

Free distribution is just `price = 0 sats` on a Vendor Block. Architect uploads + stocks paper + signposts "Free Plans." Or drops a plan on the ground for a friend. No special mechanic.

---

## 6. Phase 4 — Player-Self-Build

### 6.1 Entry

The Inspect dialog's "Place in world" button (enabled only when material list is all ✓) hands the player into placement mode. Inspect closes; world enters ghost-preview cursor mode.

### 6.2 Ghost preview + rotation

Semi-transparent wireframe tracks the player's cursor on the ground.

- **Cursor projection**: raycast from player's eye to first solid block; anchor the plan's lowest-Y at `cursor_block_y + 1`. Players naturally aim at "the ground I want to build on."
- **Q / E or scroll-wheel** rotates the wireframe 90° about its centre. Four cardinal orientations; no mirror, no tilt for v1.
- **Colour-coded validity**:
  - **Green** = valid position + materials still match.
  - **Yellow** = valid but materials short (defends against inventory churn since Inspect dialog).
  - **Red** = invalid. Tooltip shows reason.
- **Left-click on green** → confirm + start animated build.
- **Right-click / Esc** → cancel placement, plan returns to inventory unused.

### 6.3 Placement validation rules

Wireframe goes red if:

1. Terrain not flat (cells over slope, water, or hole; v1 flat-only constraint).
2. Build volume not empty (any cell in captured volume is non-air).
3. A player or mob standing in the volume.
4. Volume extends out of loaded chunks.

Each refusal shows a specific tooltip.

### 6.4 Materials confirmation + lock

**Survival mode** — when player left-clicks on green:

1. Re-validate materials at click time (defends against inventory churn).
2. **Lock** the required stacks in inventory — hot-swapping or hotbar-shifting can't disrupt the build mid-flight.
3. Animated build begins; each placed block consumes from the locked pool.

**Creative mode** — material lock is skipped entirely (per §1.5). The animated builder pulls blocks from thin air; the player's inventory is never touched. The ghost-preview yellow-state (materials short) is unreachable in creative; valid placements always show green.

### 6.5 Animated auto-build

- **Rate**: 1–2 blocks per engine tick (20 TPS) — a 60-block house = 1.5–3 seconds; a 150-block plan = ~5–8 seconds. Brisk time-lapse, not a "snap" + not a chore.
- **Order**: layered bottom-up (Y ascending), then by X, then by Z within each Y-layer. Floors first, then walls, then roof — reads visually as construction.
- **Per-block effects**: light "place" sound, dust-puff particle. Aggregated, feels alive.
- **Materials consumed** from locked pool as each block places. HUD inventory ticks down live.
- **Plaque placed last** — final block, "and... it's done!" beat.
- **Block changes** propagate through standard `pending_block_changes` path → mesh rebuild + multi-player sync if applicable.

### 6.6 In-progress HUD overlay

```
🏗  Building «Riverside House»
    [████████░░░░░░░░░░░░] 24/60 blocks
    Material: oak_plank (12 remaining)
```

Visible only to the player driving the build. Dismissable.

### 6.7 Cancellation + resume

**Mid-build Esc**:
- Dialog: *"Cancel build? (4 of 60 blocks placed. Materials used so far stay used.)"*
- Cancel → engine stops; partial structure remains in world; plan + reduced inventory remain.
- Keep building → animation resumes.

**Interrupted by world-quit / crash**:
- Engine places a **`ConstructionAnchor` marker block** at the build's lowest-XZ corner the moment a build starts. The marker stores the plan content-hash + the build's orientation + the lock state. It's persisted with the world. (Visual: a small surveyor's-flag block — also acts as a "build-in-progress" signal for the player.)
- On interruption, the partial structure + the anchor marker both stay in the world.
- **Resume**: player walks back to the build site. If they're still holding the same Plan, right-clicking the anchor marker triggers the resume dialog: *"Continue Riverside House? 4 of 60 blocks placed. Need 56 more of <materials>. [Continue] [Abandon]."*. Continue → animation resumes from where it stopped, consuming materials from current inventory. Abandon → the anchor marker is removed, the partial blocks stay (they're just blocks now), and the player can rebuild from scratch elsewhere if they want.
- **If the player lost the Plan** (sold it, dropped it): right-clicking the anchor offers only [Abandon] (you can't continue without the source plan).
- **Anchor block protection**: the marker can be broken by the world-owner / admin (clean up an unwanted build site); a normal player can't break someone else's anchor.
- The anchor block is placed by the engine — it's not in the Plan's captured contents — so it never appears in a captured plan, only in in-progress builds.

**Engine internal failure** (chunk streaming, world corruption): refund locked materials in full. Different from player-cancelled.

### 6.8 What v1 doesn't do

- Build on slopes (v2 — §11).
- Manual block-by-block guided placement (alternative to auto-build) — defer.
- Multi-player concurrent builds + ownership — multi-player phase later.
- Save replays / time-lapses for spectator-economy content — spec territory.

---

## 7. Phase 5 — NPC Builder commission

### 7.1 New profession: Builder, claims a Drafting Table

- **`Profession::Builder`** — 6th villager profession (Farmer / Blacksmith / Cook / Librarian / Carpenter / **Builder**).
- **`BlockId::DRAFTING_TABLE`** — slate-grey surface, rolled plans on top. Villagers within 3 blocks of an unclaimed Drafting Table become Builders.
- **Distinct from Carpenter** — Carpenter does wood-craft (sticks, planks, axe quests). Builder does construction. Different role, workstation, quests.
- Spawns naturally in procgen villages — villages with a Drafting Table get a Builder.

### 7.2 Two services from one dialog

```
┌─ Builder Wanda  ─ rep: Friendly ─────────────────┐
│                                                  │
│  ── Plans for sale ──                            │
│   📜 «Riverside House» (CC-BY-SA)     50 sats    │
│   📜 «Stone Tower» (ARR)             100 sats    │
│   [Browse all → ]                                │
│                                                  │
│  ── Commission a build ──                        │
│   Bring me a plan + materials + sats and I'll    │
│   build it on your chosen site.                  │
│   [Open Commission Form → ]                      │
│                                                  │
│  [ Quests ]  [ Close ]                           │
└──────────────────────────────────────────────────┘
```

### 7.3 Commission form

```
┌─ Commission a Build ─────────────────────────────┐
│  Step 1 — Plan:                                  │
│    [Slot the Plan here]                          │
│                                                  │
│  Step 2 — Materials (exactly what the plan needs)│
│    ☐ 12 Oak Planks  [slot]                       │
│    ☑ 4 Cobblestone  [filled]                     │
│    ☑ 2 Oak Log      [filled]                     │
│                                                  │
│  Step 3 — Build site:                            │
│    Click [Mark site on map] then walk + click    │
│    on flat ground to confirm.                    │
│    Selected: 124, 80, -47 ✓ (flat, valid)        │
│                                                  │
│  Step 4 — Fee:                                   │
│    Wanda charges 60 sats for this build.         │
│    Friendly-rep discount: -10%. Final: 54 sats.  │
│    [Pay 54 sats + Commission Build]              │
│                                                  │
│  [ Cancel ]                                      │
└──────────────────────────────────────────────────┘
```

### 7.4 Execution

1. Lock plan + materials + sats into the Builder's commission slot.
2. Builder pathfinds to the build site. Distance > 64 blocks or pathfind fails after ~30 s → engine teleports them to a safe spot near the site.
3. Brief "unfurling plans" animation (~2 s).
4. Animated auto-build identical to Phase 4 but **slower** (1 block per 2 ticks — reads as careful work, not magic).
5. Plaque placed last with architect's `author_npub` AND a new `built_by_villager` field naming the Builder + village ("Built by Wanda of Hillcrest Village").
6. Sats flow into the village treasury (per Spec 19's village-sats arrangements).
7. Builder returns to their Drafting Table.

### 7.5 Fee calculation v1

```
fee = 10 base
    + 1 sat per block
    + 5 sat surcharge per "premium" block
    × (1.0 - reputation_discount[tier])
```

**Premium-block list** is derived from Spec 5's tool-tier system rather than hardcoded here. Specifically, a block is "premium" if `crafting::min_tool_tier(block_id) >= ToolMaterial::Iron` — i.e., it takes at least an iron pickaxe to legitimately harvest. This naturally captures iron blocks, diamond blocks, Satori blocks, and any future premium-tier blocks without the fee formula needing updates. The surcharge reflects real material scarcity in the world.

Reputation discounts (per Spec 19's reputation tiers):
- Neutral: 0%
- Friendly: 10%
- Trusted: 20%
- Legendary: 30%
- Hostile: +20% markup

Tunable in playtest. Sats flow to village treasury per Spec 19; Builder's individual `lud16` doesn't enter v1.

### 7.6 Materials sourcing — player brings (v1)

Player provides the exact material list from the plan. Builder is labour-for-hire, not a supplier. **V2**: paid sourcing premium where Builder sources from village/world.

### 7.7 Charter + sats integration

Outgoing sats from player Charter-gated per bitcoin parent controlled. A kid commissioning a 500-sat build → guardian rule check → allowed/denied. Routes through `apply_server_tax_and_payout`. Architect royalty on commissions deferred to v2.

### 7.8 NPC plan library — where stock comes from

V1 — Builder's stock comes from two sources:
1. **Curated bundle** — 2-3 plans from the engine-bundled curated registry. CC-0 / CC-BY-SA only.
2. **Random sample** — 0-2 plans rolled from community-uploaded plans (post-Phase-6).

Stock is small + curated. Not a giant marketplace — just plans the Builder happens to know.

V2 — player-stockable Builders (drop a Plan into stock slot → it becomes a sale item; village treasury gets the cut).

### 7.9 Edge cases (failure refunds)

- Builder killed mid-commission: refund materials + sats. Toast notification.
- Site invalidated mid-walk (terrain changed, conflicting build appeared): refund + toast.
- Engine internal failure: refund + log.
- Player cancels mid-walk: 100% refund (build hasn't started).
- Player cancels mid-build animation: no further refund (placed blocks + consumed materials + paid sats — matches Phase 4 "no undo on placed blocks").

### 7.10 What v1 doesn't do

- Paid materials sourcing (v2).
- Architect royalty on commissioned builds (v2 royalty pass).
- Player-stockable Builder stock (v2).
- Build quality / fumble mechanic (v2 polish).
- Multi-Builder concurrent commissions queue (v2 — one at a time per Builder).
- Across-village Builder networks (post-alpha multi-server).

---

## 8. Phase 6 — Village procgen from the Plan Registry

### 8.1 What this replaces

Today's `village_gen.rs::build_house` is one hardcoded shape painted 3-6 times per village. After Phase 6:

- Each house slot samples from the Plan Registry.
- Every building has a Plaque crediting its architect.
- Village quality scales with community contributions.
- Engine team escapes the "hand-author every village" trap.

### 8.2 Plan Registry — three sources, v1 takes two

| Source | Curation | Available in | Plans included |
|---|---|---|---|
| **Engine-bundled** | Decented team review | v1 | ~30 curated plans shipped with engine binary. Cycle with releases. All categories. All CC-0. |
| **Server-local** | Server admin / world host | v1 | Host adds plans via admin command or config dir. Host-vetted. CC-0 or CC-BY-SA only. |
| **Nostr-decentralised** | Community moderation | v2 (post-alpha) | Architects publish plans as Nostr events; servers subscribe to curated relays/tag-feeds. |

V1 registry is **bundled + server-local merged at startup** into a single lookup table.

### 8.3 Plan categorisation

```rust
pub enum PlanCategory {
    SmallHouse,            // 1-2 villager capacity, ≤ 6×6 footprint
    LargeHouse,            // 3-4 villager capacity, ≤ 8×8
    Workshop(Profession),  // Contains the profession's workstation
    TownHall,              // Centerpiece, ≤ 16×16, unique per village
    Decoration,            // Wells, statues, lamp-posts
    Storage,               // Library, treasury
    Other,                 // Catch-all, not used by procgen
}
```

**Auto-inference at capture-time**:
- Contains a bed → SmallHouse or LargeHouse (by footprint).
- Contains a workstation block (Crafting Table / Furnace / Campfire / Bookshelf / Drafting Table) → Workshop(profession-of-block).
- Contains both → defer to size: LargeHouse for big, Workshop for small.
- Neither → Decoration.

Architect can override at capture-time. UI shows the auto-inferred default with "Edit" link.

### 8.4 Sampling logic — what village procgen does

When a village spawns, for each layout slot (defined per `village_gen.rs::HouseSpec`):

1. **Determine slot type**:
   - Slot 0 (centre) → TownHall if registry has any, else LargeHouse.
   - Workshop slots → Workshop(<profession needed>).
   - Remaining → mix of SmallHouse + LargeHouse.

2. **Filter registry**:
   - Plans matching slot category.
   - **License = CC-0 or CC-BY-SA** (gating rule from §5.4).
   - Footprint fits within slot's allotted area.
   - Not already used in THIS village (variety constraint; allow repeats if registry too small).

3. **Sample uniformly** from filtered set.

4. **Place** the plan at slot anchor + auto-place Plaque attributing architect chain. Engine handles placement same as Phase 4 but **instant** (no animation — world-gen one-shot).

5. **Register village → architect attribution** in village metadata (visible via right-click bell — §8.7).

### 8.5 Fixed slot sizes for v1

- **House slots**: max 8×8 footprint, max 8 blocks tall.
- **Workshop slots**: max 10×10 footprint, max 8 blocks tall.
- **TownHall slot**: max 16×16, max 12 blocks tall.

Plans larger than their slot are filtered out at sampling time. Most architects designing village-house plans naturally fit; megabuilds aren't procgen candidates.

V2 polish: dynamic village layout adapting road geometry to plan footprints. Defer.

### 8.6 License gating (already established in §5.4)

- **CC-0** and **CC-BY-SA** = qualifying.
- **CC-BY-ND** = excluded (procgen's repeated sampling + adjacency is arguably "compositional derivation").
- **All Rights Reserved** = excluded (can't auto-distribute IP without consent + payment).

Architect's `author_npub` baked on every building's Plaque — first-discovery credit at world-scale.

### 8.7 Village Info dialog (surfacing architect credit)

Right-clicking the Village Bell (existing Spec 19 block) opens a new tab in its dialog:

```
┌─ Hillcrest Village ──────────────────────────────┐
│  Established: 2026-05-19                         │
│                                                  │
│  Architects credited in this village:            │
│   • alice.npub1q4vrz... (3 buildings)            │
│   • bob.npub1xx88...   (1 building, town hall)   │
│   • carol.npub1qq...   (2 buildings)             │
│                                                  │
│   [Tip all architects → ] (splits sats pool       │
│    proportionally across credited architects)    │
│                                                  │
│  Reputation:  Friendly                           │
└──────────────────────────────────────────────────┘
```

A new spectator-economy primitive falls out: visiting a beautiful village + tipping the community that built it. Adds to `economies-long-run.md §7` spectator economy.

### 8.8 Sats split for procgen plaque tips

Plaque tips inside procgen villages flow through standard `apply_server_tax_and_payout`. Multi-architect tip-all splits proportionally to building-count (or equal shares — playtest decides).

### 8.9 Community contribution UX (v1)

```bash
# World host adds a plan they liked:
axenstax-engine admin registry add ~/path/to/my-plan.dat

# Or in-game via chat (ADMIN/HOST ONLY — engine checks the
# caller's npub against the configured host-admin list; non-admin
# players see "You don't have permission to add to the registry."):
/registry add <hotbar-slot-N>
```

Both paths are **admin/host-only** in v1 — they're treated as world-administration commands, not gameplay ones. Without this gate, the chat command would be a spam vector. V2 adds a community-submission flow (Nostr-event-based) for non-admin players.

Validation at add-time:
- License is CC-0 or CC-BY-SA (else rejected).
- Footprint within slot-size limits (else rejected or stored as `Other`).
- Plan content-hash unique vs. existing registry entries (avoid spam).

V2: community gallery + Nostr-event-based decentralised registry.

### 8.10 Quality control

- **Bundled plans**: Decented team curates.
- **Server-local plans**: host responsibility — same way a Minecraft server admin curates resource packs.
- **Nostr-decentralised (v2)**: relies on Nostr community moderation + curator reputation.

### 8.11 What v1 doesn't do

- Nostr-event-based decentralised registry (v2).
- Architect-reputation curated feeds (v2).
- Dynamic village layout adapting to varied footprints (v2 polish).
- Per-server content rating / age gating (v2).
- Architect leaderboards by tip-volume / building-count (spectator-economy spec territory).

---

## 9. Slope handling (v2 deferral)

V1 hard constraint: **all plan placement requires flat ground**. Engine refuses placement if any cell in the footprint isn't flat with solid base.

V2 options (sketch, not committed):

| Approach | Mechanic | Tradeoff |
|---|---|---|
| **Auto-flatten** | Engine pre-flattens slot, consuming extra dirt/stone (player inventory or NPC fee). | Cheap; tonally aggressive. |
| **Stilt pillars** | Build floats above the highest cell; stilts fill underneath. | Hilltop + waterfront builds; needs stilt block; shallow slopes only. |
| **Multi-level plans** | Capture records per-tile vertical offsets relative to baseline. Built plans curve with terrain. | Most natural; biggest changes; complex. |

V2 recommendation: auto-flatten + stilt-pillar combo (architect picks at capture-time which strategy their plan supports). Multi-level deferred further still.

---

## 10. Foundation-spec sequencing

The vision doc spans **5 buildable foundation specs**, sequenced by prerequisite:

| # | Foundation Spec | Depends On | What ships | Approx LOC |
|---|---|---|---|---|
| **A** | ~~**Papyrus Reed** (T1 plant)~~ — **DELIVERED 2026-05-19** on `feat/papyrus-reed`. Water-adjacent 4-stage plant + sugarcane-style auto-regrow harvest + 3-reeds → 3-sheets recipe + `is_paperish_slot` predicate. Foundation `2026-05-19-papyrus-reed.md`. | Nothing | Water-adjacent reed plant + growth + harvest + papyrus recipe. Reusable cross-systems (books, maps, scrolls, quest scrolls, charter clauses). | ~400 |
| **B (partial)** | **Build Schematics Core** — **PARTIALLY DELIVERED 2026-05-19** on `feat/build-schematics-core`. Foundation types (`PlanData`, `PlanLicense`, capture flood-fill, content-hash Save-As detection, rotation, animated-build state machine), 3 new blocks (Plan Tile + Construction Anchor + Architect's Plaque), recipe, save/load round-trip, capture-on-right-click with default values (no dialog yet). Phases 5-12 (egui dialogs + wireframe ghost + animated build driver + Plaque UI) deferred. | A | Per-spec breakdown in `2026-05-19-build-schematics-core.md`. | ~2000 |
| **B** | **Build Schematics Core** | A | Plan Tile block + capture + Inspect dialog + Plan item + license picker (default CC-BY-SA) + Plaque (no tipping yet) + ghost-preview + animated auto-build + Save-As / derivation. The full single-player loop. | ~2000 |
| **C** | **Plaque Tipping + Plan Trade via Vendor Block** | B + Spec 21 (Vendor Block) | Sell Master + Sell Licence modes + license enforcement at slot + Plaque tipping (lud16 resolution + Charter-gating + sats split). | ~500 |
| **D** | **NPC Builder profession** | B + Spec 19 (Villages) + Spec 21 + C | Drafting Table workstation + Builder profession + commission UI + NPC build execution. | ~800 |
| **E** | **Procgen integration** | B + Spec 19 | Plan Registry + sampling logic + replace `build_house` + Village Info dialog + multi-architect tip-all. | ~600 |

**Recommended order**: A → B → (C and E parallelisable) → D.

C unblocks trade economy; E unblocks village-quality flywheel. They're independent. D needs both Spec 19 + Spec 21 live, so it's last.

This gives a meaningful first ship at B (players can capture + self-build + derive + plaque-credit, no economy yet) and incremental layers thereafter.

**Parallelism rules**:
- A is gating — must ship before B.
- B is gating — must ship before C, D, E.
- C and E are independent of each other; can ship in either order.
- D needs C (for Charter-gated tipping infra) + Spec 21 (Vendor Block).

---

## 11. Cross-game lift

This vision is rich in engine-generic primitives. Each lifts to other Decented games:

| Primitive | Cross-game application |
|---|---|
| **Plan Tile + connectivity flood-fill capture** | Any voxel game where players author content: nest patterns, constellations, future builders in other games on the same primitives. |
| **Plan item + Inspect dialog** | Any inventory-tradeable content. Recipe cards, decoration patterns, gear schematics. |
| **Master / Licence / Derivative tier model** | Any IP-tracking system. Recipe IP in a sibling game. Sound-design IP in future audio games. |
| **CC license enum + enforcement** | Any platform with creator-content trading. Wholly engine-generic. |
| **Derivation chain + Plaque attribution** | Any game where attribution + tipping flows are valuable. Spectator + creator economies. |
| **Ghost-preview + animated auto-build** | Any voxel game with templated structures. Nest building, recipe stations. |
| **NPC commission profession + Drafting Table workstation** | Any service-economy NPC pattern. Nest Architect, Chef, Tailor. |
| **Plan Registry + license gating + categorised sampling** | Any procgen world-gen with community content. Generic primitive worth lifting first. |
| **Multi-architect tip-all (Village Bell dialog)** | Any spectator-economy hub. Restaurant tip pools, festival shared-tip jars. |

Per shared infra strategy: the framework lives in shared engine code; per-game configurations define profession names, workstation blocks, fee formulas, and accepted content types.

---

## 12. Memory + spec references

### Cross-game shared-infra strategy
- shared infra strategy — all primitives in this vision are engine-generic; lifts cross-game.

### Bitcoin parent control
- bitcoin parent controlled — outgoing sats (tips, commission fees) Charter-gated per-player guardian policy.

### Reserved-language conventions
- uk english naming — UK English throughout.

### Cross-spec composition
- `docs/foundations/2026-05-18-villages-and-villagers.md` — Spec 19; villager profession infrastructure, Village Bell, reputation tiers.
- `docs/foundations/2026-05-18-vendor-block.md` — Spec 21; trade primitive that all plan sales route through.
- `docs/foundations/2026-05-18-furnace.md` — Spec 20; introduces `apply_server_tax_and_payout` helper that every plan-related sats flow uses.
- `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md` — T1.5; introduces Mill workstation that produces sugarcane → sugar + bagasse.
- `docs/foundations/2026-05-12-proof-of-play-clarification.md` — Spec 6 §2.2; the underlying sats economy + Reserve drain pattern.

### Long-run companion docs
- `docs/vision/economies-long-run.md` — the seven economies + Proof-of-Play floor. This vision is the Knowledge-economy row **C7** fleshed out.
- `docs/vision/farming-economy-long-run.md` — the agricultural economy this composes with (sugarcane → bagasse → paper).
- `docs/vision/sat-flow-and-economy-loops.md` — the canonical sats flow patterns; all plan-related transactions follow these.

---

## 13. Open questions (v2 punch list)

1. **Royalty splits** on Licence resales, derivative sales, commissioned builds — v2 royalty pass.
2. **Cryptographic plan signing** via Nostr events — v2.
3. **In-place Master editing** (mutate existing Master, not just Save-As) — v2.
4. **Slope support** — v2 (auto-flatten + stilt-pillar).
5. **Visual similarity detection** beyond 50% block-hash — content policy.
6. **Plan rentals** (time-limited Licences) — defer.
7. **Architect-reputation curated registry feeds** — v2.
8. **Multi-architect tip-all proportion** (equal vs by-building-count) — playtest decides.
9. **Plaque protection in multi-player** — admin-only break; multi-player admin TBD.
10. **3D / isometric preview** in Inspect dialog — defer; 2D top-down covers v1.
11. **Player-stockable Builder NPCs** — v2.
12. **Per-server content rating / age-gating** — v2.

---

## Acceptance — long-run vision

This document succeeds if:

- A player who's never seen the spec can read this and understand the full lifecycle (Phase 1 → 6).
- Foundation specs (A → E) derive cleanly from this without contradicting each other.
- The licensing model is robust enough that an architect publishing under CC-BY-SA has a meaningful guarantee their work stays open.
- The Plaque attribution chain holds across an arbitrary number of derivations.
- Village procgen quality is bounded only by community contribution, not engine-team output.
- Sats flows are auditable + Charter-gated end-to-end.
- Cross-game lift is identifiable per primitive (§11), not just claimed.

Build out Foundation A (Papyrus Reed) first to unblock the rest.

# Historical Pivot — Sub-Foundation 3: Brigand Hideouts + 3 Human Tiers

**Status:** READY TO BUILD. **Prerequisite:** HP-2 (Wild Animals + Chest) merged — HP-3 reuses `BlockId::CHEST` + `BlockEntityData::Chest(ChestData)` for the stolen-goods stockpile.
**Branch:** `feat/historical-pivot-sub3-brigand-hideouts` off `main`.
**Trigger:** Sub-foundation **3** of the [Historical Pivot Long-Run](../vision/historical-pivot-long-run.md). The largest sub. Adds the human-threat track + the worldgen antagonist primitive (Brigand Hideout) that the rest of the pivot consumes.

---

## TL;DR

Three new human-mob `MobType`s (`Brigand`, `Marauder`, `Berserker`) on a single shared AI base, tuned per tier. A new worldgen structure (Brigand Hideout) sparser than villages (1 per 64×64 chunks), gated to temperate biomes, kept ≥128 blocks from any village. Each hideout is a wooden palisade ring around a central unlit campfire + a stolen-goods Chest (HP-2's) seeded with deterministic loot. Brigands spawn out of the hideout (not from surface darkness) and patrol/chase/flee per tier. The fantasy darkness-spawner keeps running alongside until HP-6.

Berserker drops the HP-1-reserved `BrigandChieftainTrophy` — the kill-the-boss reward; v1 has no further use beyond Vendor Block sale value.

All-enemies-die-on-HP-0 per the historical pivot's design principle. Puff-of-smoke on defeat, no blood, no gore.

---

## Why this lives here

- **First human threats.** Replaces the role Zombies/Skeletons play during the transition. Hideouts are the spawn origin; darkness is not.
- **Hideout-economy primitive.** A worldgen-placed, side-tabled, lootable structure that ties mob spawning to a physical location — engine-generic shape that lifts cross-game (any survival-flavoured game wants "raidable bandit camp" content).
- **Tier-tunable single AI base.** One `BrigandTier` ECS component switches DETECT_RANGE / flee-on-low-HP / day-aggressive on the shared `tick_mob_ai` dispatcher. Avoids three near-identical AI modules.
- **Closes the kept-fantasy / new-roster gap.** With HP-3 in, the pivot has a real night-time threat source on the historical side; HP-5 then re-aims Spec 22 raids at brigands and HP-6 retires the fantasy darkness spawner.

---

## What this PR ships

### 1. Three new `MobType` variants

Appended to `MobType` after `Hyena`. Definitions in `data/mobs/{brigand,marauder,berserker}.toml`. Categorised `hostile`.

| Tier | HP | Damage/hit | Speed b/s | Detect range | Flees < 25 % HP | Drops |
|---|---|---|---|---|---|---|
| Brigand | 16 | 4 | 4.5 | 16 | **Yes** | 0-1 Wool, 0-1 Bread, 0-1 IronIngot |
| Marauder | 28 | 6 | 5.0 | 24 | No | 1-2 IronIngot, 0-1 Bread, 0-1 Leather |
| Berserker | 45 | 9 | 6.0 | 32 | No | 2-3 IronIngot, 1 BrigandChieftainTrophy, 0-1 Leather |

**Day/night chase gating** — Brigand + Marauder passive during the day, Berserker always-on. **Delivered 2026-05-23 (PR #94, df3a8f1)** as a polish-round follow-on to the HP-6 cutover. `Tier::always_aggressive()` predicate guards the chase-promotion arm; `is_night_at(world_time)` reuses `camera::compute_sun` with the same 0.3 brightness threshold as `spawning::tick_mob_spawning` so spawn-time and AI-time agree on what counts as night. Wave-spawn brigands without a `HomeHideout` follow the same gate.

Drop seeds use the same `roll(seed, salt, max)` pattern as existing arms in `mob.rs::drops_for`. Salts pick fresh numbers (next after Hyena's `25`) — `26..=33`.

### 2. Per-tier tunables via `BrigandTier` ECS component

New ECS component `BrigandTier { tier: Tier }` where `Tier { Brigand, Marauder, Berserker }`. Inserted on spawn. Read by `tick_mob_ai` to short-circuit the default hostile-mob behaviour:

- **Detect range** — replaces the global `DETECT_RANGE` constant for these mobs; tier table above.
- **Flee gate** — Brigand-only; when `health.current / health.max < 0.25` they switch state to a new `AiState::FleeToHome { hideout_anchor: [i32;3] }`. Berserker + Marauder ignore the gate.

Day/night chase gating delivered 2026-05-23 (see the tier-table caption).

`FleeToHome` is a fresh `AiState` arm — pathfind toward the brigand's home hideout anchor at `speed * 1.33`. Once within 4 blocks of the anchor → `Idle`. Anchor coordinates come from the side-table entry the spawner tagged at spawn time (a new `HomeHideout([i32;3])` ECS component).

### 3. Brigand Hideout worldgen

New module `brigand_hideout_gen.rs`, modelled on `village_gen.rs`. Sparser grid, biome-gated, distance-gated from villages.

- `HIDEOUT_GRID = 64` (chunks) → 1 hideout per 1024×1024-block cell.
- `cell_has_hideout(world_seed, gx, gz)`: ~60 % of cells (`hash % 5 < 3`) so most cells have one.
- Biome gate: **Plains**, **Forest**, **Savanna**, **Taiga**. Reject anywhere else (Desert, Mountains, Ocean, SnowyTundra, BirchForest, Jungle).
- **Anti-clustering**: reject layout if any village anchor is within 128 blocks (Manhattan check against `world.village_anchors` snapshot — taken once per layout call).

#### Layout components (deterministic per `(world_seed, gx, gz)`)
- **Palisade** — oak-log ring of radius 6 around the anchor, 3 blocks tall. One 2-wide gap on the south side (door).
- **Stockpile chest** — `BlockId::CHEST` at anchor, pre-populated via `seed_hideout_loot(world_seed, gx, gz)`.
- **Central unlit campfire** — `CAMPFIRE_UNLIT` (id 44) at `anchor + (0, 0, 0)` is occupied by the chest, so the campfire goes at `anchor + (2, 0, 0)` on a cobblestone hearth. Unlit on purpose ("abandoned-looking" feel).
- **Corner torches** — 4 `TORCH` blocks one block above the palisade NE/NW/SE/SW corners (gives the hideout a low-light visible-at-night silhouette).
- **3-4 brigand huts** — small 3×3 oak-plank floors with one OAK_LOG wall + hay-bale "beds", arranged inside the palisade.
- **Brigand Hideout Banner** — new BlockId `BRIGAND_HIDEOUT_BANNER` (id **113**) at the palisade gate. Visible-at-distance marker for the player; pure decoration, no block-entity. Texture: rough purple-and-red banner on a stick (procedural, `texture_gen.rs`).

#### Stockpile loot table (deterministic)
Driven by `seed_hideout_loot(world_seed, gx, gz)` — pure function returning a `Vec<ItemStack>` of length ≤ 6:
- 2-4 Wheat
- 1-2 Bread
- 1-2 IronIngot
- 1-2 Wool
- Always one slot left empty for player loot intuition

No BrigandChieftainTrophy in the chest — the Trophy is the **Berserker kill drop**, not stockpile loot. (Players can't simply break the palisade and skip the boss fight.)

### 4. Hideout side-table on `World`

New field `World.brigand_hideouts: AHashMap<(i32, i32), HideoutData>` keyed by grid cell.

```rust
pub struct HideoutData {
    pub anchor_world: [i32; 3],
    pub gx: i32,
    pub gz: i32,
    /// Current live brigand population. Incremented on spawn,
    /// decremented on kill. Refilled to target on the 24 000-tick
    /// replenish cycle when target unmet.
    pub population: u32,
    /// Target population. 4 on normal hideouts; 5 on "rare-hideout"
    /// roll (~20 %) which also seeds a Berserker.
    pub population_target: u32,
    /// `true` if this hideout's roster includes a Berserker.
    pub has_berserker: bool,
    /// Last tick the spawner topped up the population.
    pub last_replenish_tick: u64,
}
```

Save/load via new `WorldSave.brigand_hideouts: Vec<SavedHideout>` (`#[serde(default)]` for back-compat).

### 5. Hideout spawner + replenisher

New `tick_hideout_spawning(world: &mut World, ecs: &mut hecs::World, current_tick: u64) -> u32`. Called once per second (every 20 ticks) from `GameState::tick` and `GameServer::tick` (same dual-call shape as `tick_iron_golem_spawn`).

- For each loaded hideout (chunk containing the anchor is loaded):
  - If `population < population_target` and `current_tick - last_replenish_tick >= 24_000`, spawn the missing mobs.
  - First mob in a fresh hideout: Marauder (the captain). Then fill with Brigands. If `has_berserker`, last slot is a Berserker.
  - Each spawned mob gets the `HomeHideout(anchor_world)` ECS component + `BrigandTier(tier)` component.
- Kill-attribution path (existing `combat.rs::on_mob_death`) decrements `population` on any brigand mob's death, keyed by the `HomeHideout` component.

### 6. Wire-format additions

- `EntityKind::Brigand = 22`, `Marauder = 23`, `Berserker = 24` appended to `protocol.rs::EntityKind`.
- `BRIGAND_HIDEOUT_BANNER = 113` BlockId added to `block.rs`.
- `PROTOCOL_VERSION: 25 → 26` bump.

### 7. Entity model

`entity_model.rs` gains a tier-tinted Villager-shape (human bipedal). Tier tint via colour table:
- Brigand: dark brown overcoat (drab).
- Marauder: dark grey + a metallic-grey "helmet" top cube.
- Berserker: rust-red body + a black "helmet" cube.

All three reuse the existing Villager mesh — only the colour palette differs. Reduces model work to a small `tier_palette(tier) -> [Color; N]` helper.

---

## What this PR does NOT do

- Doesn't add Knights — Sub 4.
- Doesn't change Spec 22 raid wave composition — Sub 5.
- Doesn't remove fantasy darkness spawning — Sub 6. Brigands and Zombies coexist during HP-3 → HP-5.
- Doesn't add stockpile replenishment after the player loots. Cleared = cleared.
- Doesn't add a "respawn this hideout from scratch" mechanic. Player can break the palisade; if they break the banner the side-table entry stays alive (the spawner only checks chunk-loaded + population, not block presence). Cleaner-removal is v2.
- Doesn't add new sats payout paths. Brigand kills are item-loot only. Trophy can be sold via Vendor Block at the default `Item::trade_value(MaterialId::BrigandChieftainTrophy)` (defaults to 50, overridable via `ServerEconomyConfig`).
- Doesn't introduce Charter gating on brigand kills — they're not sats payouts.
- Doesn't add tier-stratified hideouts (e.g. "Berserker stronghold"). One Berserker per rare hideout; richer tiering is v2.
- Doesn't render the stolen-goods pile as visual cubes inside the palisade. The Chest holds the loot; the palisade is the visual cue.

---

## Phasing

| # | Phase | Files | LOC est |
|---|---|---|---|
| 1 | **This spec** | — | — |
| 2 | New mob TOMLs + `MobType` variants + `drops_for` arms + `DEFS` | `mob.rs`, `data/mobs/brigand.toml`, `data/mobs/marauder.toml`, `data/mobs/berserker.toml` | ~180 |
| 3 | Wire-format: `EntityKind::Brigand/Marauder/Berserker`, `BRIGAND_HIDEOUT_BANNER = 113`, `PROTOCOL_VERSION` bump | `protocol.rs`, `block.rs` | ~60 |
| 4 | `BrigandTier` + `HomeHideout` ECS components + new `AiState::FleeToHome` arm + `tick_mob_ai` tier-tunable hooks | `entity.rs`, `mob_ai.rs`, new `brigand.rs` | ~260 |
| 5 | `HideoutData` side-table + save/load (`SavedHideout`) | `world.rs`, `save.rs` | ~150 |
| 6 | New module `brigand_hideout_gen.rs` — layout, palisade, huts, chest+loot, banner — hooked into `world::generate_column` after villages | new `brigand_hideout_gen.rs`, `world.rs` | ~420 |
| 7 | `tick_hideout_spawning` + kill-attribution decrement via `HomeHideout` | `brigand_hideout_gen.rs` or new `brigand.rs`, `combat.rs`, `game_loop.rs`, `server.rs` | ~180 |
| 8 | Deterministic stockpile loot seeding + tests | `brigand_hideout_gen.rs` | ~100 |
| 9 | Entity model tier palettes + banner texture | `entity_model.rs`, `texture_gen.rs` | ~120 |
| 10 | Tests (per-tier drops + flee gate + day-aggressive gate + hideout layout deterministic + village-distance gate + stockpile loot range + repopulation cooldown) | various `*::tests` modules | ~280 |
| 11 | Docs (HP-3 spec → DELIVERED, README flip, Spec 5 §3 mobs section append for the three new tiers) | docs | ~30 |
| 12 | Axolittle playtest | — | — |

**Total**: ~1,780 LOC across 10 build phases + 1 playtest gate. Single PR.

---

## Tests

```rust
// drops_for
#[test] fn brigand_drops_match_table() { /* 200 seeds, wool 0-1, bread 0-1, iron 0-1 */ }
#[test] fn marauder_drops_match_table() { /* iron 1-2, bread 0-1, leather 0-1 */ }
#[test] fn berserker_always_drops_trophy_and_iron_2_to_3() {
    for seed in 0..200 {
        let d = drops_for(MobType::Berserker, seed);
        assert!(d.iter().any(|s| matches!(s.item,
            Item::Material(MaterialId::BrigandChieftainTrophy))));
    }
}

// AI tier behaviour
#[test] fn brigand_flees_below_quarter_hp() { /* set HP=3/16, tick, assert FleeToHome */ }
#[test] fn marauder_does_not_flee_at_low_hp() { /* HP=3/28, tick, assert no FleeToHome */ }
#[test] fn berserker_chases_in_daytime() { /* world_time=noon, player 20 blocks, tick, assert Chase */ }
#[test] fn brigand_does_not_chase_in_daytime() { /* world_time=noon, player 10 blocks, tick, assert Idle/Wander */ }

// Hideout worldgen
#[test] fn hideout_layout_deterministic() {
    let a = layout_for_hideout_cell(123, 5, 5, &bg);
    let b = layout_for_hideout_cell(123, 5, 5, &bg);
    assert_eq!(a.map(|x| x.anchor_world), b.map(|x| x.anchor_world));
}
#[test] fn hideout_rejected_within_128_blocks_of_village() { /* place village, hideout should reject */ }
#[test] fn hideout_only_in_temperate_biomes() { /* sweep, assert biome ∈ {Plains, Forest, Savanna, Taiga} */ }
#[test] fn hideout_stockpile_seeded_with_expected_ranges() {
    for seed in 0..50 {
        let loot = seed_hideout_loot(seed, 0, 0);
        let wheat: u8 = loot.iter().filter(|s| matches!(s.item,
            Item::Material(MaterialId::Wheat))).map(|s| s.count).sum();
        assert!(wheat >= 2 && wheat <= 4);
    }
}
#[test] fn hideout_save_load_round_trip_preserves_population() { /* ... */ }

// Spawner
#[test] fn fresh_hideout_spawns_marauder_then_brigands() { /* population 0 → 4 over time, first is Marauder */ }
#[test] fn rare_hideout_seeds_berserker() { /* hash-driven branch */ }
#[test] fn killed_brigand_decrements_population() { /* spawn, kill, assert -1 */ }
#[test] fn replenish_waits_for_cooldown() { /* kill all, tick 10 000 — no respawn; tick 24 000+ — respawn */ }
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- All Phase 2-10 tests pass.
- A test world with a fixed seed places at least one Brigand Hideout inside the explorable area; the structure builds visibly (palisade + chest + banner + huts + torches).
- The hideout's chest contains the deterministic loot when first reached.
- Brigands spawn out of the hideout (not from surface darkness) and patrol within the palisade radius; on aggro they chase players within their tier range.
- Brigands flee at < 25 % HP back toward the home anchor.
- Marauders + Berserkers do not flee.
- Berserkers chase in daylight.
- Killing the Berserker drops `BrigandChieftainTrophy` 100 % of the time; the `Item::name` reads "Brigand Chieftain Trophy" and renders in the inventory.
- Population decrements as brigands die; after the 24 000-tick cooldown the hideout respawns missing mobs.
- Phase 12: Axolittle playtest confirms the encounter loop — hideouts are findable, satisfying to clear, and the Berserker fight feels distinct from a Marauder fight.

---

## Memory-rule check

- ✓ uk english naming — "Brigand", "Marauder", "Berserker", "palisade", "hideout", "behaviour" all UK English.
- ✓ bitcoin parent controlled — no new sats payout paths; brigand kills are item-loot only. Trophy sale value lives in `ServerEconomyConfig` so guardian / server policy already gates the Vendor Block path.
- ✓ shared infra strategy — per-tier ECS-tunable single AI base + side-tabled procgen-structure with population spawner are engine-generic. Lifts cross-game to any survival-flavoured game with "raidable enemy camp" content.
- ✓ merge to main preauthorised — healthy-gate merge when `check.sh` is green.
- ✓ proof of play is proof of work — orthogonal; no compliance impact.

---

## v2 polish round (DELIVERED 2026-05-23 on main)

Five PRs landed as a polish round after the HP-6 cutover, lifting items from the original Out-of-scope list. They share a single test gate (1530 tests total post-round) — Phase 12 (Axolittle playtest) covers the lot.

- **PR #94 (df3a8f1)** — Day/night chase gating (see tier table). Brigand + Marauder skip the chase-promotion arm during daytime; Berserker bypasses. `is_night_at(world_time)` reuses the 0.3 brightness threshold from `spawning`.
- **PR #95 (b76ccbe)** — Not strictly HP-3, but lands here for context: `entity::scatter_mobs_in_column` now reads `mob::biome_passive_spawn_weights` instead of a hardcoded Cow/Sheep/Pig/Chicken table, so Spec 28d's passive roster + HP-2's Bear/Hyena + 28d.nostrich's Nostrich actually spawn in fresh worlds. Squid water gate + Hyena pack-spawn + Nostrich family group all wire through the same pure picker.
- **PR #96 (c8a463e)** — **Trophy Wall** block (`TROPHY_WALL = 114`, PROTOCOL_VERSION 27 → 28). 3×1 vertical recipe (Trophy / Plank / Plank → 1 Trophy Wall). Trophy is consumed; the wall becomes the kept memento. Procedural texture (oak-plank plaque + purple/red trophy motif + iron pegs).
- **PR #97 (9deb3c9)** — **Hideout stockpile replenishment**. `HideoutData.dormant_since: Option<u64>` + `world_seed: u32` (both `#[serde(default)]` so HP-3 v1 saves load cleanly). When `population == 0` AND the stockpile chest is empty AND `STOCKPILE_REPLENISH_AFTER_CLEAR_TICKS` (48 000) ticks have passed since dormancy began, the chest re-seeds via `seed_hideout_loot` and population resets to 0 to trigger the regular replenisher next tick. Any live brigand cancels dormant tracking.
- **PR #98 (1e3e903)** — **Brigand→village treasury bleed**. The hideout-to-village raid feedback loop, minimal-shape. Every stockpile reset deducts up to `BRIGAND_STEAL_AMOUNT_SATS = 50` from the nearest village within `BRIGAND_STEAL_RADIUS_BLOCKS = 256` via the pure `bleed_nearest_village_treasury` helper. Capped at the village's current treasury so we never overdraw. Clearing a hideout protects the nearest village's treasury for 2 in-game days.

## Out of scope (still deferred)

- Knights as village defenders — Sub 4 (DELIVERED 2026-05-23, PR #91).
- Spec 22 wave composition refresh to brigand mixes — Sub 5 (DELIVERED 2026-05-23, PR #92).
- Removing fantasy darkness spawning — Sub 6 (DELIVERED 2026-05-23, PR #93).
- Tier-stratified hideouts (e.g. dedicated Berserker stronghold) — v2.
- Per-hideout sats reward on full clear — v2 (sats reserved for raids + sales on alpha).
- Brigand-armed-with-iron visual variation — v2 (deferred to dedicated mesh post-playtest).
- Hideout-to-village raid feedback loop **gameplay path** (brigands actively walking out of the hideout to attack the nearest village) — v2; the v1.5 bleed (PR #98) is the economic-shape version. The walking-raid would be the visible-shape version.

---

## Open questions

None. All design decisions resolved during HP-3 brainstorming (autonomous-mode call).

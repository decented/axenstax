# Historical Pivot — Sub-Foundation 2: Wild Animals + Chest

**Status:** DELIVERED 2026-05-22 on `feat/historical-pivot-sub2-wild-animals`. Phase 11 (Axolittle playtest) still outstanding. **Prerequisite:** Sub 1 (Drop-Economy Migration) merged first — Sub 1 reserved the `BrigandChieftainTrophy` MaterialId and added Wool→String, which this sub assumes is live.
**Branch:** `feat/historical-pivot-sub2-wild-animals` off `main`.
**Trigger:** Sub-foundation **2** of the [Historical Pivot Long-Run](../vision/historical-pivot-long-run.md). Adds the wild-animal threat track + the Chest storage primitive that Sub 3 (Brigand Hideouts) will inherit.

---

## TL;DR

Adds 2 new mob species (`Bear`, `Hyena`), 1 audit pass on existing Wolves (add Bone drop), and 1 new block (`Chest`) so the Bear's food-raid mechanic has somewhere to raid. The Chest is a HP-3 prerequisite anyway (Brigand Hideouts need a visible stockpile chest), so HP-2 builds it now and HP-3 inherits it for free.

Bears are Forest/Taiga territorial omnivores that smell food (mature crops + Chests containing food items) and walk over to take it. Hyenas are Savanna pack-hunters that ignore the player by day and hunt them by night. Wolves get a Bone drop so the post-Sub-6 farming chain stays alive (Sub 1 reserved this intent).

All-enemies-die-on-HP-0 per the historical pivot's design principle. Puff-of-smoke on defeat, no blood, no gore.

---

## Why this lives here

- **First "wild" threats** on the historical-pivot side. Replaces the ambient hostile-mob spawning that Zombies/Skeletons/Spiders currently do at night.
- **Closes the Bone supply.** Sub 1 deferred adding a Bone drop to Wolf (since Skeletons still spawn during the transition). This sub wires Bear + Hyena + Wolf Bone drops so by the time Sub 6 retires Skeletons, Bone is well-sourced.
- **Ships the Chest primitive** that Sub 3 needs anyway. Avoids HP-3 having to detour through "add Chest first" — instead HP-3 just uses `BlockId::CHEST` directly for the Brigand Hideout stockpile.
- **Cross-game lift.** Per-species AI files + the Chest block + the food-raid mechanic all lift cross-game. The "animal raids player food storage" pattern is engine-generic.

---

## What this PR ships

### 1. `MobType::Bear`

- Biomes: **Forest**, **Taiga** (spawn weight 3 in each).
- **HP**: 30. **Damage**: 6/hit. **Speed**: 4 b/s walking, 6 b/s charging.
- **AI states** (in `bear_ai.rs`):
  - `Wander` — default, slow ambient movement around territory.
  - `SmellFood` — triggered when a food source exists within 12 blocks: mature crop block (any harvest-ready stage of Wheat/Carrot/Potato/Corn/Beetroot/SugarBeet/Pumpkin/Berries — implementer verifies the actual ripe-stage block constants present in `block.rs`) OR a Chest whose contents include at least one `Item::is_food()`-true item. Walks toward the nearest source.
  - `EatCrop` — at a mature crop block: stands still 2 seconds (40 ticks), eats it (block reverts to TilledSoil), gains 60-second satiety.
  - `RaidChest` — at a Chest with food: stands still 3 seconds (60 ticks), removes ONE food item from the chest (any `Item::is_food()` true), gains 90-second satiety. Chest stays intact; only contents diminish.
  - `Aggro` — if hit by player, attacks attacker (charges at 6 b/s, 6 damage/hit). 30-second timer; returns to Wander if attacker escapes line-of-sight.
- **Satiety system** — `satiety_ticks_remaining: u32` on Bear ECS state. Decrements per tick; while > 0 Bear is in `Wander` only (won't smell food). When reaches 0, Bear can re-enter `SmellFood`.
- **Fence defeats crop-raid** — Bear pathfinding treats blocks marked `solid + non-walkable-over` (planks, cobble, etc.) as impassable. Player can wall the farm to keep bears out. No new Fence block required; any opaque ≥1 block wall works.
- **Drops on defeat**: `1-3 Bone + 0-1 Leather` (puff, no gore).

### 2. `MobType::Hyena`

- Biome: **Savanna** only (spawn weight 5).
- **HP**: 12. **Damage**: 3/hit. **Speed**: 6 b/s.
- **Pack spawn**: when a Hyena spawn rolls, spawn 2-4 hyenas in a tight cluster (within a 1-block radius of the spawn origin).
- **AI states** (in `hyena_ai.rs`):
  - `Lazy` (day) — sits or wanders slowly within ~10 blocks of pack-centre. Ignores player unless attacked.
  - `Hunt` (night, `world_time` in night range) — actively pathfinds toward nearest player or villager within 24 blocks; attacks in melee.
  - `PackBoost` — passive modifier (not a state). When ≥3 hyenas are within 6 blocks of each other, all hyenas in the cluster gain +1 damage (pack-hunter feel; rewards thinning the pack quickly).
  - `Aggro` (any time if attacked) — same as Bear's Aggro state.
- **Despawn**: standard 32-block despawn distance from any player (matches existing mob despawn).
- **Drops on defeat**: `1-2 Bone` (puff).

### 3. Wolf audit

- Open `mob.rs::drops_for` Wolf arm (or `wolf.rs` if drops are localised there); add `0-2 Bone` drops alongside any existing wolf drops.
- Verify existing `wolf.rs` AI already handles "danger if attacked" (it does per earlier session audit; tameable framework via PR #49). No behaviour change.

### 4. `BlockId::CHEST`

- Next BlockId slot (likely **id 112** — verify via `block.rs`).
- Opaque, solid, breakable with any tool (faster with axe).
- **Recipe**: 8 oak planks ring around an empty centre (mirrors Furnace recipe shape). Shapeless across plank variants — any planks work.
- **Inventory**: 27 slots (3 rows × 9 cols, Minecraft-standard).
- **`chest_ui.rs`** — new egui dialog on right-click. Standard slot grid; click + shift-click move items in/out (reuse Spec 29 furnace UX click patterns where applicable).
- **Save/load**: new `BlockEntityData::Chest(ChestData { slots: [Option<ItemStack>; 27] })` variant. `#[serde(default)]` for forward-compat.
- **Break behaviour**: drops all contents at the chest's world position (existing item-drop entity pattern). Block itself drops 1 Chest item.
- **Anti-grief** (HP-2 scope): no ownership system. Anyone can break, anyone can open. Matches Vendor Block's existing single-player pattern. Multiplayer ownership is a future concern (Charter-tier permissions or similar); explicitly out of scope.
- **Animation**: simple — no chest-open animation needed for v1. Right-click opens the dialog. Close on Escape.

### 5. Bear food-raid mechanic specifics

- Per-tick (or per-N-tick to throttle) Bear scans world within 12-block cube:
  - Mature crop blocks via existing block-id check (per-crop "ripe" stage — implementer enumerates the actual ripe-stage block constants from `block.rs`; expected set covers all currently-shipped crops).
  - Chest blocks via `world.chest_at(pos).is_some_and(|c| c.slots.iter().any(|s| s.as_ref().is_some_and(|st| st.item.is_food())))`.
- Picks nearest qualifying target → `SmellFood` state → walks there.
- On arrival: `EatCrop` or `RaidChest` per target type.
- On consume: satiety set; Bear returns to `Wander`.
- Bear's 12-block scan is bounded; no full-world walk. Throttled to once-per-second per Bear (20-tick cadence) to keep cost predictable.

### 6. New biome mob-pool wiring

Per the README's previously-flagged solo gap ("`scatter_mobs_in_column` is hardcoded to Cow|Sheep|Pig|Chicken"), HP-2's animal additions also push on this surface. Scope decision: keep HP-2 focused on the new species + chest. The biome-pool wire-up for ALL passive mobs (28d's Horse/Rabbit/Goat/Bee/Squid PLUS HP-2's Bear/Hyena) is a separate concern that should be tackled as its own pass. **HP-2 will register Bear + Hyena in the biome pools alongside the new entries (additive), but won't fix the broader scatter_mobs_in_column issue** — that's a follow-on cleanup item.

---

## Phasing

| # | Phase | Files | LOC est |
|---|---|---|---|
| 1 | **This spec** | — | — |
| 2 | `BlockId::CHEST` + recipe + textures + BlockDef | `block.rs`, `texture_gen.rs`, `crafting.rs` | ~150 |
| 3 | `BlockEntityData::Chest` + `ChestData` + `world.chest_at[_mut]` helpers + save/load | `world.rs`, `chest.rs` (new), `save.rs` | ~130 |
| 4 | `chest_ui.rs` egui dialog + right-click handler in `game_loop.rs` | `chest_ui.rs` (new), `game_loop.rs`, `player_slot.rs` (open_chest field) | ~180 |
| 5 | `MobType::Bear` + `bear.toml` + entity model + drops_for + biome spawn weights + Aggro/Wander base AI | `mob.rs`, `data/mobs/bear.toml`, `entity_model.rs`, `bear_ai.rs` (new), `biome.rs` | ~200 |
| 6 | Bear food-raid mechanic (`SmellFood` / `EatCrop` / `RaidChest` states + satiety + 12-block scan) | `bear_ai.rs` | ~120 |
| 7 | `MobType::Hyena` + `hyena.toml` + entity model + pack-spawn + Lazy/Hunt states + PackBoost + drops_for + biome wire | `mob.rs`, `data/mobs/hyena.toml`, `entity_model.rs`, `hyena_ai.rs` (new), `biome.rs` | ~180 |
| 8 | Wolf bone-drop addition (one-line `drops_for` change + test) | `mob.rs` or `wolf.rs` | ~10 |
| 9 | Tests (per-species AI + chest round-trip + bear-eats-crop + bear-raids-chest-leaves-intact + pack-boost-fires + wolf-now-drops-bone) | various `*::tests` modules | ~200 |
| 10 | Docs (HP-2 spec → DELIVERED, README flip, Spec 5 §3 mob section update) | docs | ~30 |
| 11 | Axolittle playtest | — | — |

**Total**: ~1,200 LOC across 10 build phases + 1 playtest gate. Single PR. Larger than original HP-2 estimate (vision doc said ~600 LOC for just animals); the +600 LOC is the Chest block + UI + save which we agreed to fold in.

---

## What this PR does NOT do

- Doesn't touch existing Zombie/Skeleton/Creeper/Spider/Slime/WitherSkeleton spawning. Fantasy mobs still spawn until Sub 6.
- Doesn't add Brigands/Marauders/Berserkers. Sub 3.
- Doesn't add Knights. Sub 4.
- Doesn't touch Spec 22 wave composition. Sub 5.
- Doesn't fix the broader `scatter_mobs_in_column` issue affecting Spec 28d passive mobs (Horse/Rabbit/etc. spawning). That's a separate cleanup item flagged in the audit report — HP-2 just adds Bear + Hyena entries to the biome pools.
- Doesn't add a Fence block (any opaque wall keeps bears out). Fence as a distinct low-walkable-over block can land later if needed.
- Doesn't add Chest ownership / Charter gating. Single-player anyone-can-touch matches existing Vendor Block pattern; multiplayer ownership is a future concern.
- Doesn't add chest-opening animation (visual polish; v2).

---

## Tests

```rust
// Chest round-trip
#[test]
fn chest_save_load_preserves_27_slots() { /* place chest, fill some slots, save, load, verify */ }

#[test]
fn chest_break_drops_all_contents() { /* fill chest, mine it, verify item entities spawn */ }

#[test]
fn chest_recipe_resolves_from_eight_planks_ring() { /* 8 oak planks centred-empty → 1 chest */ }

// Bear
#[test]
fn bear_spawns_in_forest_and_taiga() { /* sample spawn rolls, assert biome distribution */ }

#[test]
fn bear_smells_mature_crop_within_12_blocks() {
    // place mature wheat at (10, _, 0); place bear at (0, _, 0).
    // tick bear; assert state == SmellFood, target == wheat position.
}

#[test]
fn bear_eat_crop_reverts_to_tilled_soil() {
    // bear at crop, tick EatCrop until satiety; assert block became TilledSoil.
}

#[test]
fn bear_raid_chest_removes_one_food_item_leaves_chest_intact() {
    // chest with [Bread, IronIngot]; bear raids; assert Bread is gone,
    // IronIngot remains, chest BlockId still CHEST.
}

#[test]
fn bear_satiety_blocks_further_smell_until_expired() {
    // bear with satiety_ticks_remaining > 0 + nearby crop;
    // assert state stays Wander.
}

#[test]
fn bear_with_solid_wall_between_and_crop_does_not_path() {
    // Wall blocks pathfinding; bear can't reach crop.
}

#[test]
fn bear_drops_bone_and_leather_on_defeat() { /* range check 1-3 + 0-1 across seeds */ }

// Hyena
#[test]
fn hyena_pack_spawns_2_to_4() { /* sample spawn rolls in Savanna, assert pack-size dist */ }

#[test]
fn hyena_lazy_during_day_hunt_at_night() {
    // set world_time to noon; spawn hyena + player; tick; assert Lazy.
    // set to midnight; tick; assert Hunt + pathing toward player.
}

#[test]
fn hyena_packboost_fires_when_three_within_six_blocks() {
    // place 3 hyenas in cluster; assert damage modifier active on each.
}

#[test]
fn hyena_drops_bone_on_defeat() { /* range 1-2 across seeds */ }

// Wolf
#[test]
fn wolf_now_drops_bone() {
    let drops = drops_for(MobType::Wolf, seed);
    // expect bone in the drop list at least sometimes.
}
```

---

## Acceptance

- `./check.sh` ALL GREEN.
- All Phase 2-9 tests pass.
- Chest place + open + save + load works in-engine.
- Bears spawn in Forest + Taiga; Hyenas spawn in Savanna packs at appropriate density.
- Bear-eats-crop visibly works (place a crop near a Bear in the test, watch it disappear and Bear become sated).
- Bear-raids-chest works (place a Chest with bread near a Bear, watch the bread disappear and Bear become sated; chest stays intact).
- Wolves now drop Bone (≥1 in test runs across seeds).
- Phase 11: Axolittle playtest confirms the gameplay feel — bears feel threatening but defeatable; hyenas feel like a pack; fence works to keep bears out.

---

## Memory-rule check

- ✓ uk english naming — "Bear", "Hyena", "Wolf", "Chest", "behaviour" — all UK English.
- ✓ bitcoin parent controlled — orthogonal; no new sats flows.
- ✓ shared infra strategy — per-species AI + Chest block + food-raid mechanic all lift cross-game.
- ✓ merge to main preauthorised — healthy-gate merge when check.sh is green.
- ✓ axenstax has farming — Bear's crop-trampling adds a real reason to fence farms, which deepens the farming gameplay rather than fighting it.

---

## Out of scope (deferred to later subs or follow-on cleanup)

- Brigand mob types + Brigand Hideout structures — Sub 3.
- Knights as village defenders — Sub 4.
- Spec 22 wave composition refresh — Sub 5.
- Removing fantasy mob spawning — Sub 6.
- `scatter_mobs_in_column` broader fix for Spec 28d passive mobs — follow-on cleanup.
- Chest ownership / Charter gating — future concern.
- Fence block as distinct low-walkable-over block — future cosmetic add.
- Chest-opening animation — future polish.

---

## Open questions

None. All design decisions resolved during brainstorming.

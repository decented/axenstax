# Fantasy-Roster Excision — Open-Source IP Cleanup

**Date:** 2026-05-24
**Status:** SPEC — ready for plan (owner-approved scope; one design default flagged)
**Branch:** `worktree-fantasy-roster-removal` off `main`.
**Driver:** Open-sourcing AxeNStax. The owner does not want Minecraft-adjacent IP in the
public tree. This completes the historical pivot (HP-6 retired the fantasy roster from
*spawning*; it kept the enum variants for wire stability) by **removing the fantasy
roster from the code entirely**.

---

## 1. Goal & scope

Remove every reference to the hostile fantasy mob roster and its Minecraft-flavoured
drops from the engine, so the open-sourced codebase contains no Minecraft-IP creature
names (`Creeper`, `Wither*`) or their distinctive models/behaviours, and no orphaned
fantasy items.

This **reverses HP-6's "keep retired variants forever" decision** (that was for wire
stability; the owner accepts the breaking change for IP cleanliness, pre-launch).

### Remove completely

- **Mobs (7):** `Zombie`, `Skeleton`, `Spider`, `Creeper`, `Slime`, `WitherSkeleton`,
  `IronGolem` — from `MobType` and `EntityKind`, plus their model functions, procedural
  textures, AI/combat behaviours, dedicated modules (`skeleton_archer.rs`,
  `wither_skeleton.rs`), `mob_def`/`drops_for` arms, and all spawn/quest/bounty/combat
  references and tests.
- **Drop items (5), pure-fantasy with no other consumer:** `RottenFlesh`, `SpiderEye`,
  `Slimeball`, `WitherSkull`, `Gunpowder` — from `MaterialId`, their drop arms, textures,
  and the inventory explorer.

### Keep (generic / load-bearing — NOT Minecraft IP)

- **`Villager`** — owner decision; load-bearing across economy/quests/defenders.
- **`Knight`** — sole village/town defender (already replaced the Iron Golem in HP-4/HP-6).
- **`Arrow`** — already crafted from stick + feather (`pure_helpers.rs:99`) and drives
  ranged combat; historically appropriate. Only the (now-removed) skeleton drop went away.
- **`Bone` + `Bonemeal`** — farming depends on them (`Bone → 3 Bonemeal` fertiliser,
  Bone Block crafting). Bones are generic. **Re-sourced from animals** (see §3.3) since
  skeletons are gone.
- All animals (Cow/Pig/Chicken/Sheep/Bear/Hyena/Wolf/Horse/Rabbit/Goat/Bee/Squid/Nostrich),
  Brigand/Marauder/Berserker, and `GlowBerry` (a berry, not a hostile-mob drop).

### Design default (flagged for owner)

**Bones now drop from livestock** (Cow/Sheep/Pig drop 0–1 `Bone`) so players keep a path
to `Bonemeal` for crops. Alternative sources (mining, crafting) are easy to swap if
preferred.

### Out of scope

- Renaming `Villager`/`Bonemeal` (generic; no IP gain).
- A broader **texture/block trade-dress IP review** (block names, recipe parity, the
  blocky aesthetic). Flagged to the owner as the larger, separate exposure; **not** in
  this spec.
- Removing animal/Brigand/Knight content.

---

## 2. Wire & save impact (the breaking change)

- **`EntityKind`** is on the wire (`StateUpdatePacket.entity_spawns`). Removing variants
  shifts serde/bincode ordinals → **breaking protocol change**. Bump `PROTOCOL_VERSION`.
  Entities are **not** persisted (alpha mob state is per-session), so **no save loss from
  the mob removal**.
- **`MaterialId`** is persisted (player inventories) **and** on the wire. Removing the 5
  drop variants **invalidates old alpha inventory saves** that contain them, and is a wire
  change. **Accepted** (pre-launch, owner-approved). Update the `ALL_MATERIAL_IDS` table +
  its count assertion (already stale at 98 vs 125 — this pass corrects it).
- No new world-gen/block-format changes.

This is the right time: the game is not live (see `project_alpha_launch_posture`), so a
clean break beats carrying dead IP forever.

---

## 3. Component-by-component

### 3.1 Mob types, models, behaviours
- `mob.rs`: remove the 7 `MobType` variants, their `mob_def` table rows, `drops_for` arms,
  the `"zombie" => …` string parses, and the sun-burn/AI-tag references.
- `entity_model.rs`: remove `MODEL_CACHE` inserts + the model fns (`zombie_model`,
  `skeleton_model`, `spider_model`, `creeper_model`, `slime_model`, `iron_golem_model`;
  `WitherSkeleton` reuses `skeleton_model` — confirm and drop).
- `combat.rs` / `mob_ai.rs`: remove the creeper fuse/explosion path and the slime
  split/contact-damage arms.
- Delete modules `skeleton_archer.rs`, `wither_skeleton.rs`; remove their `mod` lines and
  any `tick_*` calls in `game_loop.rs`.
- `hosted_server.rs`: remove the `MobType::X => EntityKind::X` mapping arms.

### 3.2 EntityKind (protocol)
- Remove the 7 `EntityKind` variants; bump `PROTOCOL_VERSION`; update the version-history
  comment + the pinned-version tests.

### 3.3 Drop items + re-sourcing
- `item.rs` `MaterialId`: remove `RottenFlesh`, `SpiderEye`, `Slimeball`, `WitherSkull`,
  `Gunpowder`; update `TryFrom<u16>`, `name`, texture mapping, and `ALL_MATERIAL_IDS`
  (+ count assertion).
- `mob.rs` `drops_for`: remove the fantasy drop arms; **add a `Bone` drop (0–1) to
  Cow/Sheep/Pig** so `Bonemeal` stays reachable.
- `crafting.rs`: keep `Bone → 3 Bonemeal` and Bone Block; keep the Arrow recipe; remove
  any recipe consuming a removed item (audit — none expected beyond the removed drops).
- `texture_gen.rs` / `entity_model.rs` TEX consts: remove the fantasy texture generators
  + layer consts. **Texture-layer reindex is the highest-risk step** (the fantasy layers
  sit mid-array, so removing them shifts every subsequent `TEX_*` const + `texture_count()`).
  Plan handles this as its own carefully-tested phase; **fallback**: if reindexing proves
  too fragile, replace the fantasy generators with neutral 1×1 filler at the same indices
  and rename the consts to neutral `TEX_RETIRED_N` (keeps the array stable, removes the
  IP names) — decide in the plan after measuring blast radius.

### 3.4 Quests & bounties (the live player-facing leftovers)
- `quest.rs`: `kill(Zombie,3)`→`kill(Brigand,3)` (Farmer), `kill(Spider,2)`→`kill(Brigand,2)`
  (Carpenter), `kill(Skeleton,4)`→`kill(Marauder,4)` (Scribe); update labels.
- `bounty.rs`: `BOUNTY_TEMPLATES` "Kill 10 Zombies"→Brigands, "Kill 5 Skeletons"→Marauders;
  fix the stale comment.

### 3.5 Misc references
- `spawning.rs`: drop the dormant sun-burn legacy arm + the fantasy test spawns/asserts.
- `parity_check.rs`, `tip_jar.rs`, `bounty.rs` tests: swap fantasy mob fixtures to a kept
  mob (e.g. `Brigand`).
- `raid.rs`, `armour.rs`, `chunk_stream.rs`, `inventory.rs`, `wasm_feedback.rs`,
  `brigand.rs`, `save.rs`: audit + clean each reference (mostly comments/tests).

---

## 4. Testing

- Replace fantasy fixtures with `Brigand`/`Marauder` throughout `#[cfg(test)]`.
- New tests:
  - `MobType`/`EntityKind` no longer contain any fantasy variant (compile-time: the names
    don't exist — a doc/grep gate rather than a runtime test).
  - `drops_for(Cow/Sheep/Pig)` can yield `Bone` (re-source works → `Bonemeal` reachable).
  - Night spawn + hideout spawn still produce only kept mobs (extend the HP-6 tests).
  - `MaterialId::TryFrom<u16>` round-trips over the **new** variant set; `ALL_MATERIAL_IDS`
    count assertion matches the real enum length.
  - Quest/bounty pools reference only kept mobs.
  - Arrow craft + ranged combat still work (existing tests should pass unchanged).
- `./check.sh` ALL GREEN — clippy (no new errors), build, `cargo test`, trunk WASM,
  bundle-size gate (the texture removal should *shrink* the bundle).

## 5. Verification boundary
Logic/wire/recipes are test-covered. The **visual** check (no fantasy mob ever appears;
farming still works end-to-end; bone→bonemeal loop) is playtest-gated → test sheet
`docs/test-sheets/2026-05-24-fantasy-roster-excision.md`.

## 6. Memory-rule check
- ✓ `feedback_uk_english_naming` — UK English (e.g. "Fertiliser" if Bonemeal is ever renamed).
- ✓ `feedback_merge_to_main_preauthorised` — healthy-gate merge when `check.sh` green.
- ✓ Reverses `2026-05-23-historical-pivot-migration-cutover` §"keep variants forever" —
  documented here; update that spec's note on completion.
- ✓ `project_alpha_launch_posture` — pre-launch, so the wire/save break is acceptable now.

## 7. Open question
- Bone re-source (§3.3 default = livestock drop). Owner may redirect to mining/crafting.

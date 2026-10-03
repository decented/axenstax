# Graves & keep-inventory — recoverable death container, no more scatter-loss

**Status**: ✅ BUILT 2026-06-16 (worktree `worktree-alpha-qol-building-blocks`, goal `2026-06-16-alpha-qol-and-building-blocks`). P1: `GRAVE` block (id 263) + `BlockEntityData::Grave` (`grave.rs` `GraveData`, 36-slot snapshot **index-aligned** to the inventory) + death redirect (snapshot → safe-placed grave, void/lava/build-safe via `find_safe_grave_pos`, scatter fallback) + right-click recovery-to-original-slot + break-spill; persisted `WorldSave.graves` (append-only serde-default, round-trip tested). P2: `keep_inventory` (`WorldMeta` + runtime `World` mirror, default off, **on for blank-canvas**) + `/keepinventory [on|off]` (`/ki`). Death stays sats/score penalty-free (only ever MOVES items). `check.sh` green; pure cores unit-tested (restore-to-slot, count-conservation, safe-placement, save round-trip, command toggle). **Deferred**: a chest-style partial-loot UI (recovery is one-shot restore-to-slot for now), `/keepinventory` persistence to meta across reload (live toggle only; blank-canvas default persists), cosmetic decay/death-history (pairs with #6 minimap). Grave feel/visual = Axo playtest. Backlog **#19** from `docs/research/2026-06-15-native-bake-in-feature-backlog.md` (spec D of the 2026-06-15 build-now QoL sweep). #19 was unblocked when #18 shipped (foundations README, 2026-06-14 reconciliation).
**Date**: 2026-06-15
**Branch (when built)**: TBD (`qol/graves`).
**Owner decisions captured (2026-06-15)**: **Graves are the default Survival death behaviour** — death spawns a recoverable container, replacing today's scatter-drop. **`keep_inventory` is a per-world option** (default **off** in Survival; **on** for blank-canvas / parkour worlds). Build-now-able: single-player / PWA, no multiplayer gate (grave *ownership* is the only deferred multiplayer piece).

---

## TL;DR

Death item-loss is the single biggest source of survival rage-quitting, and the **#1-demand un-covered gameplay gap** (Corpse ~119M downloads). Today death **scatters every stack as item entities** (`game_loop.rs:2526-2549`) — the harshest possible outcome. This spec replaces that with a **grave**: a recoverable container placed at the death spot holding the full inventory, **items returned to their original slots** on recovery, void/lava-proof, persisted in the world save. **Keep-inventory** is the gentler per-world alternative.

Reference behaviour (researched 2026-06-15): [Corpse](https://modrinth.com/mod/corpse) — spawns a container at death holding all items; right-click to recover; **items return to their original slots**; empties then vanishes; can't fall into the void or burn in lava; a U death-history with coordinates + teleport; (optional) 1-hour cosmetic decay to a skeleton.

---

## Why this lives here

- Per the backlog: highest verified demand of any new round-5 item; small-to-medium build with **clean hooks already identified**; removes the biggest survival frustration.
- Per blank canvas and interchange: blank-canvas / parkour worlds already lock day/night + disable mobs at creation — `keep_inventory = true` is the natural matching default there (you shouldn't lose your build kit to a parkour fall).
- Per settlement model decision parked (non-custodial, **death must not cost sats**): graves keep items *safe*, and death currently has **no** proof-of-play / economy penalty (verified — `economy.rs`/`proof_of_play.rs` are not hooked into death). This spec preserves that: death costs *time to walk back*, never money or score.
- Per play modes shipped: Creative is already invulnerable/no-drop (`game_loop.rs:2518`); Adventure/Spectator interactions noted below. Per uk english naming: UK English.

---

## The real seam (grounded)

`game/engine/src/`:
```text
game_loop.rs:2515-2560   Death + respawn loop
   :2518   Creative → invulnerable, no drop (unchanged)
   :2526   if just_died { snapshot 36 slots → spawn_item() scatter → clear slots }   ← REDIRECT HERE
   :2552   if dead && respawn_timer == 0 → respawn() + teleport to spawn_pos
combat.rs:97   PlayerCombat { dead, respawn_timer, just_died };  take_damage (:256) sets just_died; respawn (:271)
chest.rs:31    ChestData { slots: Vec<Option<ItemStack>> } (27); try_insert (:95); cleanup_chest (:134, spills + removes)
save.rs:66     WorldSave { ... chests: Vec<SavedChest> (:108), carts: Vec<SavedCart> (:217) }
   :448  SavedChest { x, y, z, data: ChestData }      ← pattern to mirror
   :1862 WorldMeta { game_mode (:1865), difficulty (:1875), mobs_enabled (:1986, serde-default true) }   ← flag pattern
```
**What exists:** the death one-shot + a complete container model (`ChestData` + `chest_ui`) + a position-keyed persisted-container pattern (`SavedChest`) + the WorldMeta serde-default flag pattern. **What's missing:** a grave block + a `graves` save list + the redirect of the death snapshot + a `keep_inventory` flag.

---

## Scope (phased)

### Phase 1 — Grave container + death redirect (the core)
- **Grave block**: a dedicated `GRAVE` block id (new entry in `block.rs`/`BlockRegistry`, rendered as a headstone/mound) whose contents reuse `ChestData` + `chest_ui` for the open/loot UI. *(Alternative if a registry add is unwanted: an entity-based marker like carts — but a placed block is the cleanest fit with the existing position-keyed chest persistence; default to the block.)*
- **Save**: add `graves: Vec<SavedGrave>` to `WorldSave` **after `carts`** (append-only invariant; `#[serde(default)]` so old saves load with no graves). `SavedGrave { x, y, z, data, slot_map, created_tick }` where `slot_map` records each stack's **original inventory slot** so recovery restores exactly (Corpse parity).
- **Death redirect** (`game_loop.rs:2526`): in the `just_died` branch, *if not `keep_inventory`*, instead of scattering: snapshot the 36 slots, choose a **safe placement** (the death block, or the nearest solid/air if death was in void/lava/liquid — search up then outward), place a grave block there, store the snapshot as a grave container, clear the inventory. Print the grave coordinates to chat (commands/chat system exists) so the player can find it.
- **Recovery**: interacting opens the grave (`chest_ui`); **Take all** restores each stack to its **original slot** when free, else first free slot; when emptied, the grave block + save entry are removed (reuse the `cleanup_chest` path). Breaking the grave spills its contents (same path).
- **Void/lava-proof**: the safe-placement search guarantees a reachable, non-destroying location; contents never sit in lava/void.
- Scatter-drop is **removed as the Survival default**; keep it only as a last-resort fallback if no safe grave location can be found (rare).

### Phase 2 — keep-inventory option
- Add `keep_inventory: bool` to `WorldMeta` (`#[serde(default)]`, default **false**). When true, the `just_died` branch leaves the inventory intact and creates **no** grave.
- Surface it: a toggle in world settings, and a `/keepinventory [on|off]` chat command (mirrors `/gamemode`, `/time` — `commands/` registry). **Blank-canvas world creation sets `keep_inventory = true`** to match its day/night-lock + mobs-off profile.
- Creative is unaffected (already no-drop). Adventure respects the flag; Spectator can't die (noclip).

### Phase 3 (optional) — death marker / history
- A death-location marker + short history with coordinates (Corpse's U feature). **Defer and pair with backlog #6 (minimap + waypoints)** — a death waypoint is the natural home; for now the Phase-1 chat coordinate line is enough.

### Deferred (in-spec)
- **Grave ownership** (who may loot another player's grave) — multiplayer concern; single-player = it's yours. Owner-only/skeleton-stage rules land with multiplayer.
- **Cosmetic decay to skeleton** — purely visual; skip for alpha (single-player graves wait indefinitely).
- **XP retention** — no XP system exists; n/a.

---

## Acceptance criteria

- **P1:** Dying in Survival places a grave at a safe, reachable spot holding the full inventory; the inventory is emptied; recovering returns each stack to its original slot (when free); emptying removes the grave; breaking it spills contents; dying over the void/in lava never destroys items. The grave **persists across save/load** — `graves: Vec<SavedGrave>` round-trips and an **old save with no `graves` field loads cleanly** (serde-default test, preserving the append-only invariant).
- **P2:** With `keep_inventory = true`, death drops nothing and creates no grave; the inventory is intact after respawn. `/keepinventory` toggles it; a new blank-canvas world defaults it on; an old save without the field loads with it false. Creative behaviour unchanged.
- **Invariant:** no item duplication or loss across death → grave → recovery (count-conservation test), and **death applies no score/sats penalty** (guard test that `economy`/`proof_of_play` are untouched by the death path).
- `./check.sh` green (clippy + build + `cargo test --bin axenstax-engine` + trunk + bundle gate); new tests for grave round-trip, original-slot restore, safe-placement, and the keep-inventory branch.

## Memory-rule check
- **Concrete, not cards**: the grave reuses `ChestData` + `chest_ui` + the `SavedChest` persistence pattern — no parallel container system. Save stays **append-only** (`graves` after `carts`, serde-default), honouring the established invariant (goal3 save hardening delivered).
- **Multiplayer-ready**: the grave is a position-keyed server-persistable container like chests; only the *ownership policy* is deferred, and that's a rule layered on top, not a redesign.
- **Settlement-model fit**: death stays non-custodial and penalty-free for sats/score — explicitly preserved and tested.
- **Spec maintenance**: on build, update Spec 05 §Combat/Death + Spec 02 (World Format — new `GRAVE` block + `graves` save list + `keep_inventory` WorldMeta field).
- **No build authorised** beyond this queue entry — graduates on "build graves" / "add #19".

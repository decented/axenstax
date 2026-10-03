# MC-Parity Deepening Wave — Living World · Underworld · Lava · Contraptions

**Status:** DELIVERED 2026-06-21 (overnight, autonomous build) — `check.sh`
green, all engine tests passing.
**Branch:** `feature/mc-parity-deepening`.
**Trigger:** Owner asked for a big overnight build from the 2026-06-21
feature-audit (`tools/feature-audit/`), cross-checked so nothing already
shipped. The audit's recommended gaps were already closed by the earlier
gap-closure wave; this wave attacks four of the remaining **v2 / polish**
fronts the owner selected: **A** Living World (mobs), **C** Underworld
(worldgen), **B** Lava & the Deep (fluids), **D** Contraptions (redstone/
electricity).

The four are sequenced — not parallel — because they all touch shared,
order-sensitive surfaces (the block registry, `game_loop` tick wiring, and the
**append-only** save format). Each landed behind a green test gate as its own
commit.

> **Cross-check correction (armour):** the audit's parity table said armour had
> *"equip UI deferred."* That was **stale** — equipping (`craft_ui.rs`
> `click_armour_slot`), the 🛡 HUD bar (`hud_ui.rs:454`), damage reduction +
> durability + auto-unequip (`player_slot.rs::take_damage_with_armour`) are all
> live with 13+ tests. Only auto-equip-on-pickup and cosmetic-armour-on-avatar
> remain. The audit row was fixed.

---

## A — Living World: wake the dormant species AI

**Problem.** `rabbit_ai`, `goat_ai`, `bee_ai` (and `horse_ai`) each shipped a
fully unit-tested *pure* tick function plus an `XData` ECS component, but
**nothing called them** — these animals fell through to the generic
`mob_ai::tick_mob_ai` wander and read as indistinct.

**Solution.** New `species_ai.rs` dispatch layer (free functions over a bare
`hecs::World`, so unit-testable without a `GameState`), called from
`game_loop` right after `tick_wolf_companions`:

- `dispatch_rabbits` — rabbits **hop** (a forward leap + an upward bounce that
  physics arcs back down) and bolt from a too-close player.
- `dispatch_goats` — goats **charge** a nearby player and **gore** for 1 HP +
  knockback on impact. Players aren't ECS entities, so impacts are returned as
  `GoatImpact` data and applied against `self.players` by a thin wrapper.
- `dispatch_bees` — bees **fly**. A new `Flying` marker (in `entity.rs`) makes
  them gravity-exempt in `tick_entities` (block collision still applies); the
  dispatcher drives the full velocity vector incl. the vertical hover-bob, and
  distant-hive bees head home.

**The composition rule (important).** Dispatch runs *after* `tick_mob_ai`
(which re-sets each mob's horizontal velocity every tick from its `MobAi`
state). The dispatcher **yields** — leaves the generic velocity untouched —
when the mob's state is reactive (`Flee` from being struck via
`combat::spook_if_prey`, `Chase`, `InvestigateCampfire`, `GolemGuard`). This
preserves the shipped struck-prey bolt and campfire attraction unchanged;
species flavour only owns locomotion during the idle/wander states.

Components attached at `entity::spawn_mob` (mirrors the `WolfData` pattern).

**Follow-ups:** bee **sting**-on-attack (needs recent-attacker plumbing) +
**pollination** (crop-growth buff near a hive); horse herd cohesion; bear
food-raiding; squid ink; hyena pack AI. The pure AIs for several of these are
already written and just need dispatch.

## C — Underworld: ravines + abandoned mineshafts

Two new **column-aware** structure generators on the established
village/brigand-hideout pattern (a deterministic virtual grid → per-cell
layout → clip to the chunk-column), wired into `World::generate_column` after
the hideout placer. Determinism means a structure straddles chunk boundaries
cleanly and is identical regardless of visit order, with no global pre-pass.

- **`ravine_gen.rs`** — rare (~1 candidate/40×40 chunks, 45 % gated, land
  only) canyons that split the surface and cut 30–50 deep, tapering toward the
  floor with a sine wobble, exposing ore in the walls, with a lava thread along
  the deepest floors. Pure geometry helpers (`project` / `half_width_at` /
  `is_inside`) are unit-tested independently of the carve.
- **`mineshaft_gen.rs`** — buried (y ≈ 16–40) timber-framed corridor networks:
  2–4 axis-aligned arms, a 3-wide plank walkway, fence-post + plank-beam
  support frames every 5 blocks, and a **loot chest** ~60 % along each arm.
  Loot is seeded by chest position (planks / coal / iron / bone / bread /
  arrows, rare diamond) and a re-gen never overwrites a looted chest.

**Follow-ups:** dungeons / spawner-equivalent rooms, temples/ruins, rails in
mineshafts (TRACK already exists), cobweb décor.

## B — Lava & the Deep: flow + obsidian

Lava was static v1 (pools + light 15 + contact damage). Now it **flows**:

- **`lava.rs`** — a `LavaSystem` mirroring `water::WaterSystem` but
  **shorter-range** (3 blocks vs 7) and **slower** (it advances every 3rd
  tick). Owned by `GameState` (`self.lava`); ticked after water; natural lava
  (cave pools, the new ravine floors) is registered as sources via
  `register_column_sources` so it pours into any air it borders; digging a
  block next to lava wakes it (`notify_block_removed`, mirroring water).
- **Obsidian** — new `OBSIDIAN` block (**id 302**; reuses the coal-block
  texture for v1 like LAVA/PISTON before it; drops itself). Flowing lava that
  reaches a cell bordering water **freezes to obsidian** (in `lava.rs`); water
  poured *onto* lava freezes it too (`water::freeze_adjacent_lava`) — so the
  classic obsidian farm works whichever fluid moves last.

Originally single-player (`GameState`) only — the authoritative `GameServer`
had a `water` system but **no `lava` system and never ticked lava**, so on any
hosted/dedicated world lava (world-gen pools + emptied buckets) sat static
forever. **Fixed 2026-07-04** (assessment finding #1): `GameServer` now owns a
`lava: LavaSystem`, registers lava column sources on load beside water (the 3
`register_column_sources` sites), and ticks `lava.tick_spread`/`tick_retract`
in `tick()` on the same 4-tick cadence as water. Player block edits on the
server (`hosted_server.rs`) now notify both fluids of source add/remove/open-gap
via the new shared `fluids::notify_block_edit` (previously the server's
`set_block` on a player edit told neither fluid, so a placed bucket didn't flow
there either).

Also fixed (finding #2): `water::freeze_adjacent_lava` sets a lava cell to
obsidian but holds no `LavaSystem` reference, so a frozen **source** was left as
a phantom in `LavaSystem.sources` — `is_source` lied and lava re-placed there
was silently inert (`add_source`'s `insert` no-ops on the stale entry). Both
tick paths (client `GameState` + server `GameServer`) now call
`fluids::reconcile_frozen_lava_sources` on the water tick's dirty list to drop
any freshly-frozen obsidian cell from the lava source set.

The two independent fluid systems still hold no reference to each other; the
small logic that must see both lives in the new **`fluids.rs`** module so the
client and server paths can't drift.

**Follow-ups:** ~~lava + water **buckets** (scoop/place — finishes the inert
`Bucket` item)~~ — **DONE 2026-06-22** (`bucket.rs` pure fill/empty rules +
block-interaction handler; `WaterBucket`/`LavaBucket` materials; see Spec 05
§"Animal products"). ~~lava flow on the server path~~ — **DONE 2026-07-04**
(above). ~~broadcasting server-side fluid deltas to clients~~ — **DONE
2026-07-04**: `GameServer::tick` now reads the settled block at every cell the
water/lava spread+retract passes touched and queues a `BlockChange` into
`pending_block_changes`, which `hosted_server.rs` drains into the
`StateUpdatePacket` (same path as falling blocks + power). Remote clients apply
and remesh these in `network_receive` (guarded by `current != new_block`), so
fluid motion is now visible on hosted/dedicated worlds instead of only after a
full chunk resync. Remaining: fire block + fire spread, distinct
fire/lava/drowning damage typing; **leaf decay on the server** still ticks
without broadcasting its dirty cells — the identical `BlockChange` pattern, not
yet wired.

## D — Contraptions: sticky pistons

Extends the just-shipped piston (`piston.rs`). New **`STICKY_PISTON`** (**id
303**): pushes exactly like a plain piston, but on **retract** it pulls the
single block stuck to its head back into place (Minecraft parity — sticky
pulls one pushable block, plain pulls none).

- `retract()` gained a `sticky` flag; `tick_pistons` derives it from the block
  id; `collect_piston_positions` + placement-facing generalise to both variants
  via a new `is_piston()` helper.
- **Recipe:** Piston + **Rubber** (our sticky binder, tapped from rubber trees)
  — rubber-over-piston, mirroring Minecraft's slimeball+piston.

**Follow-ups:** dispensers + droppers, note blocks, observers, analog power
levels + a comparator-equivalent (the `power.rs` `u8` is already reserved),
piston arm push/pull **animation** (currently an instant move).

---

## Block registry

| id | const | notes |
|----|-------|-------|
| 302 | `OBSIDIAN` | water-quenched lava; coal-block texture v1; solid, no light |
| 303 | `STICKY_PISTON` | piston that pulls on retract; piston textures + amber tint |

`is_known` boundary test bumped to `STICKY_PISTON` (the new last id). No
existing ids shifted; the save format (positional bincode) is untouched — no
new `WorldSave` field was added by this wave.

## What was deliberately NOT done overnight

- **Single-player → HostedServer dual-sim collapse** — the audit itself flags
  it as wanting Axolittle present for phase-by-phase regression; can't be
  solo-verified.
- **Live multiplayer auth test** — owner-boundary (2-machine LAN + bunker).

## Verification

`check.sh` green (clippy no errors · native build · full engine test suite ·
trunk WASM build · brotli bundle-size gate). New tests this wave: 8
(`species_ai`) + 3 (bee flight + `Flying` gravity) + 11 (ravine/mineshaft) + 4
(lava) + 3 (sticky piston) = **29**, all passing alongside the prior 3,200.

**Feel-tuning is the playtest boundary** — the tunables (hop height, charge
speed, gore knockback, ravine width/depth, mineshaft loot, lava cadence) carry
documented defaults for Axolittle to refine. See the test sheet:
`docs/test-sheets/2026-06-21-mc-parity-deepening.md`.

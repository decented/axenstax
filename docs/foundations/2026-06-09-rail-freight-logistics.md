# Rail Freight & Logistics — the grounded transport economy (replaces "waystones")

**Status**: PHASE 1 BUILT + MERGED TO MAIN — 2026-06-10. **Phase 3 robbery + ledger BUILT 2026-06-19** (goal `2026-06-19-solo-buildout-wave-2`, Wave 5): cart tiers (`Hull`) already shipped in P1, and the solo-buildable rest landed — a new `world.hostile_acts` ledger (`hostile_acts.rs::HostileActLedger`, serialised append-only as `WorldSave.hostile_acts`) and the **transit-only robbery gate** (breaching a cart while `speed > 0` records a `CartRobbery` + toast; breaking a parked cart at a depot is the owner reclaiming it, no record). **Reputation hit + bounty fire + the per-victim/perpetrator identity** remain — they need a victim identity (multiplayer-identity, Spec 1 Phase 4). **Phase 2 commercial-freight 'pay' BUILT 2026-06-19** (same goal, decision-light): a **Bulk** vendor draws its wholesale stock from an adjacent **freight depot chest** (the cart's unload destination) on open — `vendor::restock_bulk_from_depot` moves matching freight into the vendor's `stock` — then sells lots through the existing `try_buy` → `escrow_sats` pipeline, completing load→haul→unload→**pay**. Single-item `Sell` vendors stay pocket-fed (the "personal vs commercial" rule). **No real Lightning settlement touched** — earnings are the in-world escrow score; the cross-boundary/real-LN settlement decision stays owner economics. Phase 4 (Aether security) still gated on the Aether tier.
**Date**: 2026-06-09 (drafted); Phase 1 delivered 2026-06-10
**Branch**: `worktree-rail-freight-p1` (merged to `main` 2026-06-10)
**Cosmology**: [The Six Elements & the Grounded-Frontier Principle](../vision/elements-cosmology-long-run.md).
**Supersedes**: the earlier "Waystones" idea — there is no teleport block; transport is a
**timed, physical rail network** instead.
**Detailed build plan**: [`../goals/2026-06-09-rail-freight-logistics.md`](../goals/2026-06-09-rail-freight-logistics.md)
— bite-sized, TDD, grounded in verified seams.

---

## Phase 1 — DELIVERED (2026-06-10)

Phase 1 shipped as a complete, `check.sh`-green build from worktree `worktree-rail-freight-p1`.
All seven tasks were TDD'd and two-stage-reviewed. The durable facts a rebuild needs:

### Confirmed wire / registry values

| Thing | Confirmed value | Notes |
|---|---|---|
| `TRACK` block id | **260** (`genesis:track`) | Non-solid, transparent, flat-slab mesh (`non_solid_shape_for` + `emit_small_cube`). Auto-connects to neighbours via pathing — no rotation field. |
| `TEX_TRACK` | **373**; `texture_count()` 373 → 374 | Two rail lines on a sleeper base. |
| `EntityKind::Cart` | **26** (wire repr); bincode ordinal **19** (retired-variant gaps keep ordinals fixed) | Appended after `Knight = 25`. Existing variant ordinals unchanged. |
| `PROTOCOL_VERSION` | bumped **44 → 45** | Concurrent work had already bumped 43 → 44 before this phase built. |
| `WorldSave.carts` | `Vec<SavedCart>` — trailing `#[serde(default)]` field | Append-only save invariant. Old saves (carts-less) load with `carts == []`. |

### Mechanics delivered

**Cart entity** — engine's first vehicle. ECS components `(Position, CartEntity, CartData)`.
Deliberately omits `OnGround`, so `tick_entities` gravity/collision skips it entirely.
Driven by `tick_carts`, called after `tick_entities` in both `GameState::tick` and
`GameServer::tick`.

`CartData` fields: `cell`, `came_from`, `progress`, `speed`, `facing`, `cargo: ChestData`.
`CART_SPEED = 0.08` cells/tick (≈ 1.6 blocks/s at 20 TPS — tunable in playtest).

**Track pathing** — pure function `next_track_step(at, came_from, is_track)` returns the
single onward track neighbour. Returns `None` at a terminus (dead-end) AND at a junction
(> 1 onward neighbour) — carts **park at junctions** in Phase 1; no switching.
A cart dispatched mid-line must be seeded an initial direction (`seed_came_from`, from the
dispatcher's look direction) or it cannot pick a side.

**Depot** — no new block. A depot is simply **a chest adjacent to a track terminus**.
- **Load on dispatch:** a parked cart adjacent to a depot chest pulls freight aboard, then departs.
- **Unload on arrival:** a cart parking at a terminus that has an adjacent depot chest dumps
  its cargo into it. Overflow stays on the cart — freight is never silently lost.

**Ride-along** — right-click a parked cart with an empty hand to mount; the rider's camera
follows the cart over real in-world time; rider cannot act at distance while in transit ("you're
on the train"). Dismount on arrival or on jump. `PlayerSlot.riding` is **transient** — not
persisted (a live entity id cannot survive reload).

**Persistence** — `CartData` (cell / came\_from / progress / speed / facing / cargo) survives
save/load and carts keep rolling after reload.

**Multiplayer broadcast** — carts emit as `EntityKind::Cart` via the hosted-server
`diff_entities` path (spawn-once, then updates; no spurious despawn). Client-side rendering
from the receive path is **deferred** — the same systemic gap that exists for mob remote
rendering. Two-client visual verification = playtest boundary.

### New modules
- `game/engine/src/rail.rs` — `TRACK` constant, `h_neighbours`, `next_track_step`,
  `depot_chest_for`. All pure + unit-tested.
- `game/engine/src/cart.rs` — `CartEntity`, `CartData`, `CartSpeed`, `spawn_cart`,
  `advance` (pure, tested), `tick_carts`.

### Post-merge review fixes (2026-06-10)
A second adversarial pass after merge surfaced that the feature shipped **unreachable**
(no way to obtain track) plus several edge cases. Fixed same day:
- **Obtainability** — `/give track` (alias `rail`) + TRACK in the creative starter (plus a
  `/give chest` arm + chest in the creative starter for depots). No survival recipe yet —
  a deliberate design deferral. Carts still come from the `/spawncart` cheat.
- **Build gesture** — right-clicking a track cell only dispatches when a parked cart is
  actually there; otherwise it falls through to block placement (so you can lay/extend rail
  by clicking the line). The earlier "fall through to placement" was a no-op.
- **Game modes** — Spectator is blocked from cart dispatch/mount/drain (it overrode noclip);
  Adventure/Survival/Creative may ride + dispatch.
- **Robustness** — a cart whose own cell is no longer track parks (mined-out rail);
  dismount settles the rider onto solid footing (no fall-through the non-solid slab over a
  gap); the 3P camera retracts/auto-recenters while riding; corrupt-save speed/progress are
  sanitised and `advance` caps steps/tick (closed-ring guard).

### Craftable carts + hull armour (2026-06-10)

A follow-up build delivered on the same day as Phase 1, extending Rail Freight with
crafting recipes, a three-tier hull armour system, and a breach-to-break / cart-pickup
mechanic. All landed in worktree `cart-armour`, merged to `main` 2026-06-10.

#### Confirmed registry values

| Thing | Confirmed value | Notes |
|---|---|---|
| `MaterialId::WoodCart` | **153** | `/give wood_cart` |
| `MaterialId::IronCart` | **154** | `/give iron_cart` |
| `MaterialId::DiamondCart` | **155** | `/give diamond_cart` |
| `MATERIAL_ID_COUNT` | **156** | Fixed a latent bug in passing: `Bellows=152` was missing from the `TryFrom<u16>` inverse table. |
| `PROTOCOL_VERSION` | bumped **45 → 46** | `CartData.hull` appended to the save shape; wire is unchanged (hull not broadcast yet). |

#### Recipes

**Track (survival):** `I.I / ISI / I.I` (6 Iron Ingot in two columns + 1 Stick centre) → **16 Track** blocks.

**Wood Cart:** `P.P / PPP` (5 Planks in a U shape; mixed species OK) → **1 Wood Cart**.

**Iron Cart:** `III / IWI / III` (8 Iron Ingot surrounding 1 Wood Cart centre) → **1 Iron Cart**.

**Diamond Cart:** `DDD / DRD / DDD` (8 Diamond surrounding 1 Iron Cart centre) → **1 Diamond Cart**.

All three cart items are `/give`-able (`/give wood_cart|iron_cart|diamond_cart`). Creative
starter includes track + chest; carts come from crafting or `/give` (the old debug
`/spawncart` still spawns a Wood cart).

#### Hull armour tier — `CartData.hull: Hull { Wood, Iron, Diamond }`

`CartData` gained a `hull: Hull` field appended last with `#[serde(default)]`. The field
**persists in the save** — save shape changed, hence the `PROTOCOL_VERSION 45→46` bump.
Wire is unchanged: broadcast carts carry no hull field yet (deferred to Phase 3).

Pre-hull saves that contain carts load with those carts **reset** (tolerant decode trade-off;
pre-launch is acceptable). Pre-hull saves with no carts load cleanly as before.

Breach progress lives in a companion `#[serde(skip)]` transient field — **no save change**
for breach state; it resets on reload (expected and correct).

#### Place a cart from an item

Right-click a track cell holding a Cart item, with no cart already present and
`can_edit_world` — spawns a parked cart of the matching hull tier, consuming one item from
the held stack. (`/spawncart` cheat continues to spawn a Wood cart unconsumed.)

#### Breach-to-break (armour effect + cart pickup)

Mine / left-click a track cell that has a cart on it — this **breaches the cart** instead
of the rail. Breach time scales by hull:

| Hull | Breach ticks | ≈ seconds at 20 TPS |
|---|---:|---:|
| Wood | 20 | 1 s |
| Iron | 60 | 3 s |
| Diamond | 120 | 6 s |

On a successful breach: the cart is despawned; its **Cart item is dropped** (player recovers
the cart); **cargo spills as item drops** (no freight is silently lost); the **track cell
stays**. This realises the hull armour concept (heavier hull buys exposure time, never
invincibility) and fills the gap that carts previously could not be picked up.

Forward-compatible with Phase 3 transit-robbery — the same breach mechanic, re-gated to
carts in transit and annotated with a `// BRIDGE:` note.

#### Combined track-cell interaction model

| Gesture | Cart present? | Mode | Result |
|---|---|---|---|
| **Right-click** (empty hand) | Yes | Not Spectator | Mount / ride or dispatch (parked) |
| **Right-click** (holding Cart item) | No | `can_edit_world` | Place cart from item |
| **Right-click** (holding block/item) | — | Any | Lay rail / place block (fall-through) |
| **Left-click / break** | Yes | `can_edit_world` | Breach the cart (track stays) |
| **Left-click / break** | No | `can_edit_world` | Break the rail normally |

Mode gates: place + breach = Survival/Creative (`can_edit_world`); dispatch/mount = anyone
except Spectator. Adventure may ride + dispatch but not place or break ("use, don't wreck").

#### Deferrals from this build

- **Cart-item textures** — placeholder block-texture tints today (BRIDGE-marked); dedicated
  art awaits the texture wave that covers cart visuals.
- **Breach / recipe-cost feel** — breach ticks and recipe costs are tunable; Axolittle
  playtest determines final values.
- **Phase 3 robbery** — transit-only breach gate (`// BRIDGE:` in the breach handler) remains
  unbuilt; gated on the hostile-act ledger and Phase 3 generally.
- **Hull broadcast** — `CartData.hull` is not sent in multiplayer entity updates yet (wire
  unchanged from Phase 1); deferred to Phase 3 / two-client visual verification.

### Known deferrals (not yet built)
- **Chest/plot ownership on depot drain** — `cart::dispatch_cart` auto-drains the adjacent
  depot chest with no ownership check (parity with today's unprotected chests; a `// BRIDGE:`
  note pins it). Route through the access predicate when chest/plot protection lands.
- **Live multiplayer cart broadcast is inert** — `/spawncart`/dispatch write the *client*
  ECS; `diff_entities` reads the *server* ECS, so only save-loaded carts broadcast. Resolves
  with the standing dual-sim debt (route single-player through the server). Two-client visual
  = playtest boundary regardless (no client renders broadcast entities yet).

---

## TL;DR

Fast travel and bulk-goods movement are the **same problem**, and in a shared persistent
world the only *coherent* solution is timed, physical transport — a **rail network**. A
cart is a real object, at a real position, over real ticks; that's the only honest way to
make travel "cost time" without teleporting (which breaks shared-world time coherence).

The same rail solves goods movement. Carrying unlimited ore in your pockets and walking
is fine for **personal use** (weightless, free — kids and builders never think about it),
but the moment you produce **for trade at volume**, goods become **physical freight** that
must be hauled. That friction isn't a tax on fun — it's the thing that *creates* a trade
economy at all (distance → margin → merchants → hauliers → regional specialisation).

**We have no railway system today** (confirmed: zero rail/cart/track code). So this spec's
job is two things: (1) a deliberately **minimal MVP concept** that gets a rail+freight loop
on screen and *starts* the economy locally, and (2) an honest list of **what we need to
build** to deliver it. Robbery, cart tiers, and Aether security are captured here but
**phased after** the basic loop works. Keep it simple; start the economy before worrying
about it becoming global.

---

## Why this lives here

- Per [[project_economies_vision]]: transport friction is the missing engine under the
  markets/services/land economies — it's what makes trade *exist* rather than just
  production. Rail is the artery between mine/farm and the already-built market end.
- Per [[project_axenstax_has_farming]] + the mining loop: bulk materials are the natural
  first freight; the **mine-to-surface haul** is rail's first killer app.
- Per the cosmology canon: rail **showcases the element ladder** — muscle/gravity now →
  electric powered rail (electricity tier) → Aether freight security (aether tier).
- Per [[project_shared_infra_strategy]]: the **cart is the engine's first *vehicle*
  entity** — a cross-game-liftable primitive (any Decented game wanting vehicles).
- Per [[feedback_autonomy_to_playtest_boundary]]: the MVP is solo-buildable; *feel*
  (speed, distances, whether robbery is fun) is the playtest gate.
- Per [[feedback_uk_english_naming]]: UK English throughout.

---

## What we already have (the seams this leans on)

- **Chests** are a complete block-entity — `chest::ChestData` (27 slots),
  `BlockEntityData::Chest`, `world.chest_at()`, bincode save/load, `cleanup_chest`.
  → **A depot is just a chest.** No new "depot" block for v1.
- **The ECS already moves item-carrying entities** — `entity.rs` has `Position` +
  `Velocity` components, an `ItemEntity` that holds an `ItemStack`, projectile velocity
  integration, and `tick_entities`. → A cart is "an `ItemEntity` with a bigger inventory,
  constrained to track." The shape is known.
- **The market end is fully built** — `vendor.rs` (+ Bulk Vendor, Spec 40),
  `market_hub.rs`, `auction.rs`, `bazaar.rs`, `economy.rs` (`PayoutKind`). Rail feeds
  these; it doesn't replace them.
- **Most risk systems exist** — `reputation.rs`, `bounty.rs`, `raid.rs`. Robbery plugs into
  these. **Correction:** there is **no** hostile-act "Integrity Ledger" (only a cheat-command
  ledger) — the robbery phase must build a small `world.hostile_acts` ledger (build plan P3.3).

---

## The MVP — the simplest concept that proves the idea

**Goal: load a cart from a chest, send it down a track, it arrives later, unload into the
chest at the far end. That's it.** One mechanic that delivers *both* timed player travel
(the waystone replacement) *and* freight.

1. **Lay track** (new block) from A to B.
2. **Place a chest at each end** — these *are* the depots. No new block.
3. **Place a cart** (new entity) on the track at A.
4. **Load** the cart from the adjacent chest (item transfer).
5. **Dispatch** — the cart runs A→B along the track **over real in-world time** (timed →
   multiplayer-coherent). You **may ride it** (so the same cart is also timed player
   fast-travel along your own line).
6. **Unload** into the chest at B (auto on arrival, or manual).

**Propulsion (v1):** muscle/gravity — you push/dispatch it and it runs the line at a fixed
speed. **No electricity required** (the electricity tier doesn't exist yet); powered rail
is a later phase gated on that tier. Honest and period-correct.

**Scope of the MVP:** a single line, single world, point-to-point. **No global economy, no
networked freight, no robbery, no tiers.** Just: rail makes moving volume efficient and
moving yourself possible — the carrot, with no stick yet.

### Depot decision (answering "what is a depot?")

For v1: **a depot is a chest sitting at a rail terminus.** Reuse `ChestData` as-is. **No
capacity tiers** — one chest = one depot, existing 27-slot limit. "Bigger depots for more
capacity" is deferred; if it ever lands it's a multi-chest or a dedicated block, but not
now. Start with the chest-at-the-end-of-the-line and see how it feels.

---

## Phased scope (dependency-ordered)

### Phase 1 — Rail MVP — **DELIVERED 2026-06-10**
Track block(s), cart entity (first vehicle), muscle/gravity propulsion, load/unload from an
adjacent chest, **timed travel**, ride-along, save/load (carts + cargo + track persist).
See the "Phase 1 — DELIVERED" section above for confirmed ids, mechanics, and module map.
`check.sh` green; new pure-helper unit tests across `rail.rs`, `cart.rs`, and `test_integration/`.

### Phase 2 — Personal vs commercial freight *(the "meaning" rule)*
Personal inventory stays weightless/free. **Selling at volume** routes through freight:
a Bulk Vendor / Market Hub only accepts **wholesale stock delivered as freight to its
depot chest** (small face-to-face single-item sales stay pocket-fed). This is where rail
gains *economic necessity*, not just convenience. Informal gifting/dropping stays legal —
it's self-limiting (pocket-shuttling doesn't scale), so no anti-gift rule is needed.

### Phase 3 — Cart tiers + robbery (transit-only)
- **Cart material = breach time** (reuse block-hardness): **Wood** (instant) → **Iron**
  (slow) → **Diamond-reinforced** (very slow). Armour buys *exposure time*, never
  invincibility. Heavier hull = dearer + slower (decision: protect ∝ cargo value × route
  risk; nobody armours a gravel run).
- **Only freight *in transit* is robbable — never the depot.** Stealing a load means
  hijacking the whole cart and hauling it (exposed, slow, committed); the thief faces the
  same logistics problem. Breach tanks **reputation** + fires a **bounty** + records to a
  new `world.hostile_acts` ledger (none exists yet — see build plan P3.3). *Robbery must be
  rare, hard, risky, and costly — spice, not tax;*
  most freight should arrive safely (critical for the kid alpha).

### Phase 4 — Aether freight security *(gated on the Aether tier existing)*
Active layer, **information + flagging only** (owner-confirmed — no remote lockdown,
robbery stays possible). Tiers: **breach alarm** (wireless notify anywhere) → **live
tracking** (see where it stalled, intercept) → **beacon/auto-flag** (broadcast the thief,
tag the stolen load so it stays trackable while hauled). Grounding: the module is
**electric-powered, Aether-transmitted** (the two new elements on one gadget). *Robber
counterplay (an Aether jammer arms race) is acknowledged but explicitly deferred* — keep
it one-directional for now.

### Future (parallel, gated on the electricity tier)
**Electric powered rail** — ✅ **BASIC SPEED-UP SHIPPED 2026-06-17** with Spec 48
Electricity Phase 1: a TRACK cell whose meta powered-bit is set (driven by an
adjacent energised cable) speeds a passing cart by `cart::POWERED_RAIL_SPEED_MULT`
(2×). Power it by running Cable beside the line + any source (lever/generator).
Still future: uphill haulage / longer-route automation tuning. And later,
**Aether signalling/dispatch** (routing, block signals) — railway signalling was
an early telecom network, a natural aether job.

### Final — Axolittle playtest gate
Speed, distances, ride-tedium on long hauls, and whether robbery is *fun*, are feel
numbers only a real player (and the kid who'll tell us if a 15-minute ride is boring) can
tune.

---

## What we actually need to build (engineering — since rail = zero today)

The MVP's genuinely-new work, smallest first:

1. **Track block(s)** — new `BlockDef`(s): straight, corner, slope. Placement + render.
   v1 can be straight + simple corners only (no junctions/switches).
2. **Cart entity** — the engine's **first vehicle**. An ECS entity with a cargo inventory
   (reuse `ChestData`-style slots), constrained to track, with a travel speed. Models on
   the existing `ItemEntity` + `Velocity` pattern.
3. **Movement-along-track** — cart follows the rail (v1: straight runs + corners; no
   pathfinding, just "advance along the laid line"). Arrival detection at a terminus.
4. **Propulsion (v1)** — dispatch-and-auto-run at fixed speed (gravity/muscle abstracted);
   ride-along so the player rides the same cart.
5. **Load/unload transfer** — move stacks between an adjacent chest (depot) and the cart's
   cargo; auto-unload on arrival.
6. **Timed travel** — the cart occupies the transit duration in real ticks (coherent in
   multiplayer; in single-player the world advances normally too). Riders are "in transit"
   for the duration.
7. **Save/load** — carts (position along line + cargo) and track persist;
   `#[serde(default)]`-safe additions. Likely a `PROTOCOL_VERSION` bump (new block + entity).

Out of the MVP entirely: junctions/switches, powered rail, capacity tiers, robbery, Aether,
inter-depot routing, anything networked-global.

---

## Decisions captured

- **Depot = a chest at a rail terminus.** No new block, no capacity tiers in v1.
- **Transport is timed + physical** (a rail journey), never instant teleport.
- **The cart does double duty**: freight haul *and* timed player travel.
- **Only freight in transit is robbable; depots are safe.**
- **Aether security = information + flagging only** (no remote lockdown); jammer counterplay
  deferred.
- **v1 power = muscle/gravity** (electricity tier not built); powered rail + Aether security
  are gated on their element tiers landing.
- **Start local, not global** — one line, one world; the economy starts small.

---

## Acceptance criteria (MVP)

- Lay a track A→B, place chests at both ends, place a cart, load it from chest A.
- Dispatch → the cart travels the line over **real in-world time** → unloads into chest B.
- You can **ride** the cart for timed travel along your own line.
- Carts, cargo, and track survive save/load.
- No regression to existing block/entity/chest behaviour. `check.sh` green; new pure-helper
  unit tests (track-follow step, load/unload transfer, arrival detection, save round-trip).
- Spec 02 (World Format — new blocks/entity) and Spec 05 (Gameplay — transport) updated;
  cosmology doc linked.

---

## Memory-rule check

- [[feedback_concurrent_agent_use_worktree]] — build in a dedicated worktree.
- [[feedback_autonomy_to_playtest_boundary]] — build the MVP solo; stop at the feel
  playtest gate.
- [[project_shared_infra_strategy]] — keep the **cart/vehicle** + **timed-travel**
  primitives engine-generic (cross-game lift).
- [[feedback_merge_to_main_preauthorised]] — healthy-gate merges once built.
- [[feedback_uk_english_naming]] — UK English.
- **No build yet** — per CLAUDE.md "Do NOT build anything unless explicitly asked," this is
  a spec only. Building Phase 1 needs an explicit "build it."

---

## Open questions

1. **Long-haul UX** — riders are "in transit" for the duration; is real-time riding fun, or
   should early rail be **short mine/village spines only** (long regional hauls deferred)?
   My lean: start short.
2. **Propulsion feel** — dispatch-and-auto-run vs a rideable push-cart vs both. (Both is
   tidy: dispatch for freight, ride for travel — same entity.)
3. **The commercial trigger (Phase 2)** — is "a Bulk Vendor / Market Hub only accepts
   freight-delivered stock" the right line between personal and commercial? Confirm before
   Phase 2.
4. **Track materials** — does track itself tier later (wood rail → iron rail → powered
   rail), and does rail cost gate how far the economy can sprawl early? (Probably yes —
   keeps it local before global.)
5. **Where the cosmology canon lives** — kept in AxeNStax `docs/vision/` for now; promote
   to a Decented-platform doc when a second product consumes it. (Owner to confirm.)

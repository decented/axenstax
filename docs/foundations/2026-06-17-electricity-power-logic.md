# Electricity — Power & Logic (the redstone replacement) — Foundation Spec

**Status:** READY TO BUILD — Phase 1 is fully solo-buildable; Phases 2–4 gated as noted.
**Date:** 2026-06-17
**Spec #:** 48 (foundations queue)
**Cosmology (CANON, sits above this spec):** [The Six Elements & the Grounded-Frontier Principle](../vision/elements-cosmology-long-run.md) — Electricity = the **wired** 5th element; the redstone replacement; **power is *generated and stored*, never mined.**
**Names + first mechanics:** **LOCKED 2026-06-16** (owner: Staxolottle) in the cosmology doc's "Electricity tier — component glossary + first mechanics" section. This spec does **not** re-open naming; it builds what was locked.
**Substrate audit (design groundwork):** Engine content + Electricity audit (internal repo) — "No power/logic layer at all; the real foundational missing piece is a neighbour-update / scheduled-tick mechanism."
**Showcase consumer:** [Rail Freight & Logistics](2026-06-09-rail-freight-logistics.md) — Phase 2 of that spec ("electric powered rail") becomes reachable when **this** spec's Phase 1 lands.

---

## TL;DR

The engine is content-rich but has **zero power/logic layer**. There's a single seed — the `CopperCable` material (no gameplay effect) — and nothing else. This spec lays the **Electricity tier**: a grounded, "real Victorian engineering" power-and-logic system that replaces Minecraft's redstone.

Two **missing engine primitives** are built first, because nothing else works without them — and both pay off far beyond electricity:

1. **Neighbour-update / scheduled-tick scheduler.** Today every block-entity ticks in isolation; nothing "pushes" a change to a neighbour. This is *the* gap the audit flags. We add a reusable scheduler ("block A changes → notify block B", plus delayed ticks).
2. **Per-block metadata byte.** Today a placed block carries only its `u16` id — no orientation, no state. Directional/stateful blocks (levers, gates, mirrors — **and** the whole building-blocks backlog: doors, stairs, slabs) need a per-block `meta` byte. We add a sparse one.

On top of those, Phase 1 delivers a **complete, playable redstone-replacement**: insulated **Cable**, the **Hand Crank** + **Steam Generator** + **Battery** (generate-and-store), the **Lever / Button / Pressure Plate** inputs, a **Logic Gate** (AND/OR/NOT/XOR), and the first consumers — **Electric Lamp** and **Powered Rail**. Phase 2 adds the **Beam Sensor + Mirror** (parkour-friendly photoelectric triplines) and **Motion Sensor**. Phase 3 turns binary power into a quantitative **energy economy** (rated generation, battery capacity, brownout) and the machines that need it — **Electric Furnace**, **Electric Motor**. Phase 4 is the prerequisite-gated tail — the **Water Wheel** shipped 2026-09-06 once directional water flow existed, and the **Windmill** shipped 2026-09-07 once a wind mechanic (`wind.rs`) existed — after which the **Aether** wireless tier gets its own spec.

**No `unsafe`, no new dependencies. Block ids append (no reorder). One `PROTOCOL_VERSION` bump (BlockChange gains a `meta` byte; chunk stream + `WorldSave` carry sparse `block_meta`).**

---

## 1. What exists today (verified substrate)

All references checked in code 2026-06-17. The build plan grafts onto these — it does **not** invent new patterns where one exists.

| Substrate | Where | Role in the power layer |
|---|---|---|
| Block storage `Chunk { blocks: [BlockId; CHUNK_VOLUME], light, placed, mesh_dirty }`; `BlockId = u16`, `0 = AIR` | `chunk.rs:20`, `block.rs:5` | The block grid. **No per-block metadata exists** → we add a sparse `block_meta` (§3). |
| `BlockDef { name, solid, transparent, gravity, color, tex_top/bottom/side }` registry = `Vec<BlockDef>` indexed by id | `block.rs:1165`, `block.rs:2733` | New blocks append. We add a `directional: bool` BlockDef flag so the mesher knows to read `meta`. |
| Block-entities = tagged enum `BlockEntityData::{Campfire, Furnace, Vendor, Hive, Chest, TipJar, Auction, LatentPrint, Grave}` in `World::block_entities: AHashMap<(i32,i32,i32), BlockEntityData>` | `world.rs:31` | We add **one variant**: `PowerDevice(PowerDeviceData)`. Rich per-device state (switch on/off, gate op, sensor config, battery charge, generator fuel) lives here — exactly the furnace pattern. |
| `FurnaceData` + `tick_one(&mut FurnaceData) -> FurnaceTickOutcome` (fuel-burn + lit-flip pattern) | `furnace.rs:68`, tick at `game_loop.rs:2323` | The **Steam Generator** and **Electric Furnace** reuse this fuel→heat→output state machine almost verbatim. |
| Lighting BFS: `NEIGHBOURS: [(i32,i32,i32);6]`, queue-based propagate-with-decay, **two-phase darken→refill removal** | `lighting.rs` (~:74) | The propagation template for power flood + power-removal. Crosses chunk boundaries cleanly. |
| Water flood: `WaterSystem { sources, spread_queue, retract_queue }`, `SPREAD_BUDGET = 64`/tick, `notify_block_removed(...)` | `water.rs` | The **per-tick budget** + **explicit neighbour-notify** template. Power generalises water's hand-rolled notify into the shared scheduler (§2). |
| `BlockChange { x, y, z, new_block: u16 }` → accumulated in `pending_block_changes` (`server.rs:275`) → `StateUpdatePacket.block_changes` → client re-mesh | `protocol.rs:277`, `protocol.rs:356` | Visual power state (lamp lit, cable energised, rail powered) rides this path. We extend it with a `meta: u8`. |
| `GameServer::tick()` ordered phases (world clock → time → spawn(400t) → falling/water/leaf(every 4th) → player physics → mob AI → … → **entity physics (`server.rs:652`)** → **carts (`server.rs:659`)** → combat …) | `server.rs:524–698` | The **power tick** slots in **after entity physics, before carts** (carts read powered-rail state the power tick just computed). Mirrored in `GameState::tick` for single-player, beside furnace/campfire. |
| `WorldSave` append-only with `#[serde(default)]`; `SavedFurnace { x,y,z, data }`; rail precedent `WorldSave.carts` + `PROTOCOL_VERSION` bump on save-shape append | `save.rs:66`, `save.rs:269` | Persistence template. We append `block_meta` + `power_devices`; `energised` is **not** saved (rederived on load). |
| `rail::next_track_step(at, came_from, is_track)`; `cart::tick_carts` after `tick_entities`; `CART_SPEED = 0.08` | `rail.rs:36`, `cart.rs` | **Powered Rail** = a TRACK cell whose `meta` powered-bit is set; the cart's per-step speed reads it. No new block type. |
| `PROTOCOL_VERSION = 50`; highest block id `SATORI_CHEST = 267` (next free **268**) | `protocol.rs:687`, `block.rs` | Append ids from 268; one protocol bump for the `meta` wire change. (Both numbers may drift under concurrent work — read them at build time.) |

**The one hard truth:** there is **no neighbour-notification API today.** Water spreads only because it explicitly calls `notify_block_removed` when an adjacent block breaks — a per-system hack. The power layer needs this generalised, which is exactly Phase 1's first deliverable.

---

## 2. Primitive #1 — the neighbour-update / scheduled-tick scheduler

This is the spine. It is **engine-generic** (not power-specific) so Aether, future redstone-likes, and even existing systems (water could later migrate onto it) reuse it.

### 2.1 Data (`world.rs`, new `block_update.rs`)

```rust
// New module: game/engine/src/block_update.rs
pub struct UpdateScheduler {
    /// Positions needing re-evaluation THIS tick (neighbour updates). Deduped.
    pending: VecDeque<(i32, i32, i32)>,
    in_pending: AHashSet<(i32, i32, i32)>,      // dedupe guard
    /// Delayed updates keyed by ABSOLUTE target tick (gate delay, button release,
    /// pressure-plate release). BTreeMap → due-tick draining is a range scan.
    scheduled: BTreeMap<u64, Vec<ScheduledUpdate>>,
}

pub struct ScheduledUpdate { pub pos: (i32, i32, i32), pub kind: ScheduleKind }
pub enum ScheduleKind { PowerReeval, GateSettle, ButtonRelease, PlateRelease }
```

`UpdateScheduler` lives on `World` (so single-player and server share it). The absolute-tick key uses the existing monotonic `tick_counter` (`server.rs:550`).

### 2.2 API

```rust
impl World {
    /// Enqueue the 6 cardinal neighbours of `pos` for re-evaluation this tick.
    pub fn notify_neighbours(&mut self, pos: (i32, i32, i32));
    /// Enqueue `pos` itself for re-evaluation this tick.
    pub fn mark_dirty(&mut self, pos: (i32, i32, i32));
    /// Enqueue `pos` for re-evaluation `delay` ticks from `now`.
    pub fn schedule(&mut self, pos: (i32,i32,i32), delay: u64, kind: ScheduleKind, now: u64);
}
```

Cardinal-6 matches lighting/water and is correct for power (diagonal cells don't conduct). Diagonal/26-neighbour is reserved for systems that need it.

### 2.3 The gameplay-edit hook (the cleanup that makes this stick)

Today, block edits scatter `world.set_block(...)` + `dirty_chunks.insert(...)` + `pending_block_changes.push(...)` across `game_loop.rs`/`server.rs` (e.g. furnace lit-flip at `game_loop.rs:2327`). We introduce **one helper** that does all of it *and* notifies neighbours:

```rust
impl World {
    /// THE gameplay block-edit path: set the block, mark the chunk dirty,
    /// queue a BlockChange for broadcast, and notify neighbours for power/logic.
    /// Worldgen and chunk-fill keep using raw `set_block` (no notify, no broadcast).
    pub fn edit_block(&mut self, pos: (i32,i32,i32), new_block: BlockId, meta: u8,
                      out: &mut Vec<BlockChange>) { /* set + dirty + push + notify_neighbours */ }
}
```

> **Why split `edit_block` from `set_block`:** worldgen calls `set_block` millions of times; we must **not** schedule a power re-eval per worldgen block. Gameplay place/break/falling-block/water/power-visual go through `edit_block`; worldgen stays on `set_block`. This also retires the scattered `pending_block_changes.push` boilerplate (a real "concrete, not cards" win).

### 2.4 The tick drain (per `GameServer::tick`, after entity physics, `server.rs:652`)

```
power_tick(world, now):
  1. Drain scheduled[..=now] into `pending` (range-pop the BTreeMap).
  2. Evaluate `pending` as a bounded flood (§4), budget POWER_BUDGET cells/tick;
     overflow stays queued for next tick (water's pattern).
  3. Emit BlockChanges for any visual flips (lamp, cable-energised, rail-powered, lit gen).
```

`POWER_BUDGET` (start ~1024, tunable) caps worst-case cost; logic-gate 1-tick settle (§4.3) bounds oscillator rate. Networks are touched **only when something changes** — no full-world scan.

---

## 3. Primitive #2 — per-block metadata byte

Directional and stateful blocks need more than a `u16` id. Rather than explode the registry into hundreds of `LEVER_NORTH_ON`-style ids, we add **one sparse meta byte per block** — and it immediately unblocks the *building-blocks* backlog too (doors/stairs/slabs/trapdoors all need orientation; see audit Part 2).

### 3.1 Storage (`world.rs`)

```rust
pub struct World {
    // ... existing ...
    /// Sparse per-block metadata. Absent ⇒ meta == 0 (the default for all plain blocks).
    /// Only directional/stateful blocks ever store a byte here.
    pub block_meta: AHashMap<(i32, i32, i32), u8>,
}
impl World {
    pub fn meta_at(&self, x:i32,y:i32,z:i32) -> u8 { *self.block_meta.get(&(x,y,z)).unwrap_or(&0) }
    pub fn set_meta(&mut self, pos:(i32,i32,i32), m:u8); // m==0 removes the entry (stays sparse)
}
```

Sparse `AHashMap<pos,u8>` matches the engine's existing side-table style (`block_entities`, `drying_racks`, …) and avoids 4 KiB/chunk of mostly-zero bytes.

### 3.2 Meta layout conventions (per block family; documented in `block.rs`)

| Bits | Meaning | Used by |
|---|---|---|
| `0b00000_111` | **facing** (0=Down,1=Up,2=N,3=S,4=W,5=E) | lever, button, gate, sensor, motor, … |
| `0b00011_000` | **state** (device-specific: on/off, pressed, op variant low bits, mirror angle) | all stateful |
| `0b11100_000` | **aux** (gate op high bit, battery visual-fill tier, beam-armed) | as needed |

Pack/unpack are pure helpers (`meta::facing(m)`, `meta::with_state(m, s)`, …) with unit tests.

### 3.3 Rendering, wire, save

- **BlockDef gains `directional: bool`.** When true, the mesher reads `world.meta_at(...)` to pick the model orientation/variant (lever tilt, gate texture-by-op, mirror angle). Non-directional blocks ignore meta — zero change to the hot meshing path for the 99% case.
- **Wire:** `BlockChange` gains `meta: u8` (default 0); the chunk-stream packet carries the chunk's non-zero `block_meta` entries. → **one `PROTOCOL_VERSION` bump** (50 → 51 at time of writing; read current value at build).
- **Save:** `WorldSave.block_meta: Vec<(i32,i32,i32,u8)>` appended with `#[serde(default)]` (old saves load with empty meta → all blocks meta 0, correct).

---

## 4. The power model

### 4.1 Representation

- **Energised state** is transient, sparse, **never saved**: `World::power.energised: AHashMap<(i32,i32,i32), u8>` — the signal level at each conductor/device terminal. **Phase 1 is binary** (`>0` = on); the `u8` is reserved for Phase-3 analog strength (dimmer lamp, motor speed, sensor threshold). On world load, `energised` is **rederived** by enqueuing every source/switch into the scheduler — so the save stays tiny and can never desync from device state.
- **Device state** (switch on/off, gate op, sensor mode, battery charge, generator fuel, orientation) lives in `block_entities` as `PowerDevice(PowerDeviceData)` and **is** saved (`SavedPowerDevice { x,y,z, data }`, like `SavedFurnace`).
- **Stateless power blocks** — Cable, Lamp, Powered Rail — have **no** block-entity; their on/off is read from `energised` and shown via a visual flip (Cable↔Cable-lit, Lamp↔Lamp-lit, rail meta powered-bit).

```rust
pub enum BlockEntityData { /* …existing… */, PowerDevice(PowerDeviceData) }

pub struct PowerDeviceData {
    pub kind: PowerDeviceKind,
    pub facing: Facing,          // also mirrored into block_meta for client rendering
    pub on: bool,                // switch/output latch
    pub charge: u32,             // battery (Phase 3); 0 otherwise
    pub fuel: Option<FurnaceData>, // steam gen / electric furnace reuse the furnace machine
    pub gate_op: GateOp,         // AND/OR/NOT/XOR (LogicGate only)
    pub sensor: SensorState,     // beam mode/armed, motion cooldown (Phase 2)
}
pub enum PowerDeviceKind {
    Lever, Button, PressurePlate, LogicGate, HandCrank, SteamGenerator, Battery,
    ElectricLamp,                    // (Lamp is mostly stateless, but stores nothing extra)
    BeamSensor, Mirror, MotionSensor, // Phase 2
    ElectricMotor, ElectricFurnace,   // Phase 3
}
```

### 4.2 Conductors, sources, sinks — the directed model

A pure undirected flood isn't enough once logic gates exist (a gate has **input** faces and an **output** face). The model is a small directed graph:

- **Cable** is an **undirected conductor**: a cable cell is energised iff any connected *driving output* (a source, a switch in the "on" position, a gate output, a charged battery) reaches it through connected cable. (Phase 1: no distance decay — real wire; see Open Question §11.1.)
- **Devices** read **input faces** and drive **output faces**, defined by `facing`:
  - **Inputs** (drive cable when active): Lever (latched on/off), Button (momentary pulse), Pressure Plate (while stood on), Hand Crank (while cranked), Steam Generator (while fuelled), Battery (while charged + discharging), Beam/Motion Sensor (on trip).
  - **Logic Gate**: reads its input face(s), applies `gate_op`, drives its output face.
  - **Consumers** (read cable, do work): Electric Lamp (lit), Powered Rail (speed-up flag), Electric Motor / Electric Furnace (Phase 3, consume energy).

**Battery no-self-charge / no-chain rule (bug found + fixed 2026-09-07).** A
Battery's own output was being read back as a driving input, so it recharged
itself off its own wire — free perpetual power, and the same hole let two
batteries keep each other alive indefinitely. Fixed by `power::battery_is_fed`,
which requires a driving source that is **not itself a battery** (a direct
neighbour, or anywhere on a cable component that is already on `mains`).
Corollary, and the rule to design around: **nothing that is itself a battery
ever charges a battery — batteries do not chain.** Hang each battery off the
same generator run rather than daisy-chaining them; legitimate wiring (a
generator, wheel, mill, crank or switch on the same component) is unaffected.

### 4.3 Evaluation (one settle pass per tick)

```
For each dirty cell (from §2.4):
  • If CABLE: recompute energised = OR over connected driving outputs (bounded BFS over
    connected cable + adjacent device output-faces). If it flipped, notify_neighbours.
  • If a DEVICE: recompute its output from its input faces NOW:
      - Lever/Button/Plate/Crank/Gen/Battery/Sensor → from their own state.
      - Logic Gate → truth function; but its OUTPUT is applied via a 1-tick scheduled
        GateSettle (schedule(out_pos, 1, GateSettle, now)). This (a) gives a deterministic
        evaluation order, (b) breaks combinational feedback loops, (c) bounds oscillators
        to ≤ 10 Hz (clock circuits run, but can't melt the tick).
  • Apply visuals via edit_block: Lamp→LAMP_LIT, Cable→CABLE_LIT, TRACK powered-bit,
    Generator→_LIT. Broadcasts ride pending_block_changes automatically.
Budget: stop after POWER_BUDGET cells; re-queue the rest for next tick.
```

Power **removal** (break a cable, flip a lever off, generator runs dry) reuses lighting's **two-phase darken→refill**: clear energised outward from the lost driver, then re-flood from any still-driving boundary. This is the proven pattern for "a source disappeared, recompute who's still lit."

---

## 5. Build phases

Phases map 1:1 to the cosmology doc's **locked build order** (its steps 1–6).

### Phase 1 — Signal & Logic spine ⭐ (locked steps 1, 2, 3-simple, 5)

Delivers a **complete, playable redstone-replacement.** Solo-buildable end to end; the playtest gate is feel-tuning, not capability.

1. **Scheduler** (§2) — `block_update.rs`, `World` integration, `edit_block` hook + retire scattered `pending_block_changes.push`. Power tick slotted into `GameServer::tick` (after entity physics) **and** `GameState::tick` (single-player, beside furnace).
2. **Per-block meta** (§3) — `World::block_meta`, `BlockDef.directional`, mesher read, `BlockChange.meta`, chunk-stream meta, `WorldSave.block_meta`, **PROTOCOL bump**.
3. **Power model + `PowerDevice` block-entity** (§4) — `power.rs` (flood, removal, evaluation, budget), `BlockEntityData::PowerDevice`, `SavedPowerDevice` + `WorldSave.power_devices`, rederive-on-load.
4. **Conductors:** `CABLE` / `CABLE_LIT` blocks. Recipe via the existing `CopperCable` seed (§6).
5. **Generate-and-store (simple/binary):**
   - **Hand Crank** — right-click to crank; drives output for N ticks per crank (momentary bootstrap).
   - **Steam Generator** — fuelled like a furnace (`FurnaceData` reuse); drives output at full strength while burning; `_LIT` visual.
   - **Battery** — buffers: holds its output high for a configurable run-down after its input drops (Phase-1 = a timed capacitor; Phase-3 makes it a quantitative accumulator).
6. **Inputs:** `LEVER` (latched), `BUTTON` (momentary, `ButtonRelease` scheduled), `PRESSURE_PLATE` (entity-on check each power tick).
7. **Logic Gate** — `LOGIC_GATE`, op in meta/`gate_op` (AND/OR/NOT/XOR), 1-tick settle.
8. **Consumers:** `ELECTRIC_LAMP` / `_LIT`; **Powered Rail** = TRACK powered-bit read by `cart::advance` for a speed multiplier (wires up Rail spec Phase 2).
9. Tests (§8), test sheet, Spec docs touched (§9).

### Phase 2 — Sensors (locked step 4) — **DELIVERED 2026-06-17**

**Status:** built + TDD-tested + full `check.sh` green. Ids 280–282 (`BEAM_SENSOR`,
`MIRROR`, `MOTION_SENSOR`) + 3 procedural textures (397–399). Pure beam raycast in
`beam.rs` (`trace_beam`: through-beam pairing, retroreflective 180°, 90°-turn
mirrors, dead-end, bounded range — 5 unit tests). `power::sensor_beam` wires it to
the world; `tick_devices` trips a Beam Sensor when its armed beam is crossed by an
entity and a Motion Sensor on proximity (`MOTION_RADIUS` 3 blocks). Place sets
facing from look; right-click cycles a Mirror's 5 reflective configs. Craftable +
`/give`-able. Visible red beam via the wireframe LineList pipeline
(`renderer::set_beam_lines` + `game_loop::refresh_beam_overlay`). Remaining =
**Axolittle feel playtest** (mega-test sheet covers it). Implementation diverges
from the sketch below only in keeping the mirror model simple (retroreflect +
single 90° turn toward facing) — fine for the locked mechanics.

- **Beam Sensor** (`BEAM_SENSOR`) + **Mirror** (`MIRROR`) + **Motion Sensor** (`MOTION_SENSOR`), exactly per the **locked mechanics** in the cosmology doc:
  - Beam Sensor emits **and** detects a faint, slightly-visible IR beam from the block centre.
  - **Retroreflective** (sensor + Mirror terminal, primary) and **through-beam** (sensor at each end) arming modes.
  - Mirrors rotate to reflect **180°** or turn **90°**; a beam that dead-ends doesn't arm.
  - Beam is **non-destructive / parkour-safe**; an entity crossing any segment **breaks** it → trigger → feeds Cable/Logic Gate like any input.
  - **Motion Sensor** = beamless PIR proximity/area trigger.
- Beam raycast + mirror routing are **pure functions** (heavily unit-testable). Entity-crossing detection runs server-side each power tick (reuses the entity-position scan the pressure plate already does).
- Strong synergy with blank-canvas **parkour** (timing gates, "cross the beam to open the door").

### Phase 3 — Energy economy + machines (locked step 3, quantitative)

Upgrades binary power to a **quantitative energy balance** (the "generated and stored" differentiator made real):
- Sources gain a **rated output** (units/tick); Battery gains **capacity** + charge/discharge; consumers gain a **draw**. Each tick: supply (generation + battery discharge) vs demand; shortfall → **brownout** (lowest-priority consumers drop). The Phase-1 `energised: u8` becomes the carrier.
- **Electric Furnace** (`ELECTRIC_FURNACE` / `_LIT`) — fuel-free, faster smelting; reuses `FurnaceData` minus fuel, gated on energy draw.
- **Electric Motor** (`ELECTRIC_MOTOR`) — rotational output primitive for future machines.
- Gated on a Phase-1 **playtest** (so the binary layer's feel is validated before adding numeric complexity).

### Phase 4 — Prerequisite-gated sources, then Aether

- **Water Wheel** — **DELIVERED 2026-09-06.** The blocker (directional water flow) landed with `water::flow_vector` on 2026-07-04, so the wheel shipped ahead of the rest of Phase 4. Two block ids (`WATER_WHEEL` 316 / `WATER_WHEEL_TURNING` 317, the Steam-Generator idle/active twin pattern) and a `PowerDeviceKind::WaterWheel` appended at the end of the enum. **Adjacency rule:** each tick the wheel sets `on = has_current_neighbour(world, pos)` — true when any of the **four horizontal neighbours, or the cell directly below**, is water with a current (`water::flow_vector(..) == Some(_)`). The cell **above** is deliberately excluded (water poured on the roof is a shower, not a race). Still water — a pond, a lone source block — has no flow vector and drives nothing, which is the teaching point: a wheel needs a *stream*, so the builder cuts a channel and gives the water somewhere to fall. On a change of state the block swaps to/from the turning twin, pushes a `BlockChange`, marks dirty and notifies neighbours. Crafted from **8 Planks (any species) ringing 1 Copper Ingot** — the plank ring keeps it clear of the Steam Generator (same copper core, iron ring) and the copper axle keeps it clear of the Vendor Block (same plank ring, iron centre). No light emission, no meta byte, no protocol bump.
- **Windmill** — **DELIVERED 2026-09-07** (Wind, Copper & Electricity wave). Needed a wind mechanic first: `wind.rs` derives a deterministic `WindSample` from `(tick, weather, seed, altitude)` — a pure function, never saved or synced, so both sides of the wire compute the identical breeze with no protocol change for wind itself. Base breeze is two-octave smooth noise mapped to `0.10..=0.60`; weather adds `+0.25` rain / `+0.50` storm (storm supersedes rain, does not stack); altitude adds `+0.010`/block above sea level, capped at `+0.30` at **+30 blocks** (inside the 96-block world, ceiling `SEA_LEVEL + 33` — the first tuning used `+0.004`/block, needing +75 blocks for the cap, outside any legal build, so it was retuned before ship). Player-facing words: calm `<0.20`, light `<0.35`, fresh `<0.60`, strong `<0.85`, gale `≥0.85`. **Turn rule** (`power::windmill_turns`): a mill turns only when *exposed* — clear sky for `WINDMILL_SKY_SCAN` (8) cells directly above **and** at least two of its four horizontal neighbours clear — gated by `WINDMILL_START = 0.35` to start and `WINDMILL_STOP = 0.30` to stop (hysteresis, so a lull doesn't flicker it; sea-level clear-day duty cycle ~0.38–0.51 across seeds, ~1.00 at +30). **The wind-blocking rule is SOLIDITY OR FLUID, not "not AIR."** `blocks_wind` reads the block registry's `solid` flag plus a new `block::is_fluid` (water/lava collide but were never `solid`) — glass, leaves, water, lava and snow layers block the wind; a Cable, torch or sapling on the mill's own output face does not (a real bug found: without this a mill could not be wired up through the obvious face). Two block ids (`WINDMILL` 318 / `WINDMILL_TURNING` 319, the Water Wheel idle/active twin pattern) and `PowerDeviceKind::Windmill` appended last (bincode-positional). Crafted from **Canvas / Plank / Stick ringing Copper + Iron** (placeholder recipe, Axolittle feel-check pending, as is the 8-cell sky-scan depth). No light emission, no meta byte, no protocol bump for the block itself — see §10 for the `DeviceInteract` protocol bump (v62) that shipped alongside it for switches elsewhere in the tier.
- **Aether** (wireless: telecom/signalling/telemetry, alarms, remote logic) — the **6th element, separate future foundation spec**; consumes this tier's signal layer.

---

## 6. Recipes (proposed — owner/Axolittle confirm at build, like Rail)

Grounded in existing materials (copper ore→ingot, rubber tree, iron, the existing `CopperCable` seed). Marked placeholder; the *catalogue→matcher consistency test* (Spec 43) will cover any that land.

| Component | Proposed recipe | Notes |
|---|---|---|
| **Copper Wire** (new material) | 3 Copper Ingot in a row → 6 Copper Wire | "drawing wire"; ore→ingot→wire chain |
| **Cable** (places `CABLE`) | Wire / Rubber / Wire (horizontal) → 2 Cable | Supersedes the current `Copper/Rubber/Copper → CopperCable` data-laydown recipe (keep that as a bridge until Wire lands) |
| **Hand Crank** | Stick / Copper Ingot / Plank stack | manual bootstrap |
| **Steam Generator** | Iron ring + Copper core + Furnace | "boiler + dynamo"; fuelled like a furnace |
| **Battery** | Copper / Acid-or-Salt / Copper layered | "voltaic pile / accumulator"; salt economy already exists |
| **Lever** | Stick on Cobble | classic |
| **Button** | 1 Plank or 1 Stone | classic |
| **Pressure Plate** | 2 Plank or 2 Stone (row) | classic |
| **Logic Gate** | Copper Wire + Iron + Stone | "relay" |
| **Electric Lamp** | Glass + Copper Wire + (filament: Iron/Copper) | filament/arc lamp |
| **Beam Sensor** *(P2)* | Glass + Copper Wire + Iron | photoelectric |
| **Mirror** *(P2)* | Glass + Iron (+ Silver if added) | reflector; also decorative |
| **Motion Sensor** *(P2)* | Copper Wire + Iron + Glass | PIR |
| **Electric Furnace** *(P3)* | Furnace + Copper Wire ring | fuel-free smelting |
| **Electric Motor** *(P3)* | Iron + Copper Wire coil | rotational output |

Obtainability guard (learned from Rail shipping unreachable): every new block/item must be `/give`-able **and** craftable (or in the creative starter) before Phase 1 closes.

---

## 7. Block ids, protocol, persistence (summary)

- **Block ids** append from the next free id (**268** at time of writing — verify; concurrent work may have moved it). Phase 1 ≈ `CABLE, CABLE_LIT, ELECTRIC_LAMP, ELECTRIC_LAMP_LIT, LEVER, BUTTON, PRESSURE_PLATE, LOGIC_GATE, HAND_CRANK, STEAM_GENERATOR, STEAM_GENERATOR_LIT, BATTERY` (~12). Phase 2 +3 (`BEAM_SENSOR, MIRROR, MOTION_SENSOR`). Phase 3 +3 (`ELECTRIC_MOTOR, ELECTRIC_FURNACE, ELECTRIC_FURNACE_LIT`). **Powered Rail adds no id** (TRACK + meta powered-bit). Appending ids alone does **not** bump protocol.
- **Protocol:** **one bump** (Phase 1) for `BlockChange.meta: u8` + chunk-stream `block_meta`. Per the Rail precedent, the `WorldSave` shape change also rides this bump. Read the current `PROTOCOL_VERSION` at build (50 now → 51) — concurrent work may have advanced it.
- **Persistence (append-only, `#[serde(default)]`):** `WorldSave.block_meta: Vec<(i32,i32,i32,u8)>` + `WorldSave.power_devices: Vec<SavedPowerDevice>`. `energised` is **rederived** on load (enqueue all sources/switches), never serialised. Old saves load with empty meta + no devices → identical behaviour.

---

## 8. Testing (engine conventions: pure-fn units + `TestHost` integration)

**Pure-function units** (`#[cfg(test)] mod tests`):
- `meta::` pack/unpack round-trips (facing × state × aux).
- Gate truth tables: AND/OR/NOT/XOR over all input combinations.
- Beam routing: straight run, mirror 180°, mirror 90°, dead-end → not armed, through-beam pairing.
- Flood/removal: energise across N cells; cut mid-run → downstream darkens; two-phase refill from an alternate driver.
- Scheduler: due-tick draining order, dedupe, button-release timing, gate 1-tick settle, oscillator bounded.

**`TestHost` integration** (`src/test_integration/power.rs`, registered in `mod.rs`):
- Crank → Cable → Lamp: crank lights it; stops → goes dark after the buffer.
- Lever latches a lamp; Button pulses then auto-releases.
- Pressure Plate lights on player-walk, clears on step-off.
- AND gate needs both inputs; NOT inverts.
- Battery keeps a lamp lit for the buffer window after the source stops.
- Beam Sensor + Mirror arms; an entity crossing breaks it → downstream lamp toggles (P2).
- Powered Rail speeds a cart vs unpowered (asserts `cart` step delta).
- **Save/load round-trip** of a live circuit: devices persist, `energised` rederives, lamp state matches pre-save.
- Break a generator's fuel → whole network darkens (removal path).

`check.sh` must stay green (clippy + build + `cargo test --bin axenstax-engine` + `trunk build` + bundle-size gate).

---

## 9. Spec docs to update (source-of-truth rule)

- **`docs/spec/05-gameplay-systems.md`** — new "Electricity / Power & Logic" section (model, components, recipes, mechanics).
- **`docs/spec/02-world-format.md`** — document the new per-block `meta` byte + sparse `block_meta` storage + save shape (this is a world-format change and MUST be captured here, per CLAUDE.md "winding orders, coordinate conventions … all of it goes in the spec").
- **`docs/spec/01-engine-architecture.md`** — the `UpdateScheduler` (neighbour-update / scheduled-tick) as an engine subsystem + its tick-loop slot.
- **`docs/spec/04-networking.md`** — `BlockChange.meta` + chunk-stream meta + the `PROTOCOL_VERSION` bump.
- **Cosmology doc** — flip the Electricity tier's "foundation spec still to be written" line to "Phase 1 spec'd → see `2026-06-17-electricity-power-logic.md`."
- **Rail spec** — note Phase 2 (electric powered rail) is now unblocked by this spec's Phase 1.
- **This foundations `README.md`** — Spec 48 row (added).
- **Building-blocks backlog** (`docs/research/2026-06-15-native-bake-in-feature-backlog.md`) — note that #27 (slabs) / #29 (doors) and the broader directional-block gap are unblocked by Primitive #2 (per-block meta).

---

## 10. Multiplayer, authority, performance, proof-of-play

- **Authority:** the power sim is **server-authoritative** in `GameServer::tick`. Single-player runs it in `GameState::tick` beside furnace/campfire — i.e. it lives **inside** the known dual-sim BRIDGE (CLAUDE.md debt #1). It does not deepen that debt (it's another pure subsystem both sides call), and it migrates for free when single-player routes through `HostedServer`.
- **Inputs:** lever/button toggles are player actions → server; pressure-plate / beam-break / motion are server-side entity-position checks each power tick. Visual flips broadcast via `BlockChange` (id + meta) on the existing path. Two-machine visual verification of remote power state = a **playtest boundary** (same systemic client-render gap as remote mobs/carts).
- **A joiner's placements are now real on the host (fixed 2026-09-07).** `hosted_server.rs` used to apply an incoming `BlockChange` with a bare `set_block`, dropping both the meta byte (every directional block a joiner placed faced north on the host: a Logic Gate drove the wrong cell, a Mirror bounced the wrong way) and the `PowerDevice` block-entity (a joiner's complete circuit was pure scenery — nothing sourced, nothing lit, and a broken source could live on as a ghost powering the run forever). The host now applies `bc.meta` and registers/drops the device, keyed on the block **kind** changing — a lit/unlit twin swap (generator, lamp, wheel, mill) leaves the device's fuel/charge alone.
- **Switches reach the host via `DeviceInteract` (protocol v62, 2026-09-07).** Right-clicking a lever/button/plunger detonator/hand crank/mirror used to mutate only the acting client's own world — nothing on the wire carried a device interaction at all. `PacketType::DeviceInteract = 56` (C→S, `{ pos: (i32,i32,i32) }`, 12 bytes) asserts a cell and nothing else; the host validates the sender is joined, the cell is within the same reach envelope block changes use, and a toggle-class `PowerDevice` is actually there — dropping silently otherwise, same posture as a block change — then applies the interaction with `power::interact_device` (shared by the client's own place-arm right-click, so the two ends can't drift) and the flip rides the existing block-change broadcast back to everyone, including the asker. **Toggle-class covered:** Lever, Button, Plunger Detonator, Hand Crank, Mirror. **NOT covered — stays single-player/host-local:** fuelling a Steam Generator (the item would have to come out of the *server's* copy of a remote player's inventory, which is still client-authoritative — CLAUDE.md known debt) and Logic Gate op-cycling (UI-driven, out of scope for this bump). Autonomous sources (Windmill, Water Wheel, Pressure Plate, sensors, a fuelled generator) already reach the host through the server's own sim, per the previous bullet — this closes the remaining gap, a switch a player actually touches.
- **Performance:** neighbour-update-driven (no full-world scans), sparse storage, `POWER_BUDGET` cap with carry-over (water's pattern), gate 1-tick settle bounding oscillators. Worst case (a player builds a megastructure clock) is bounded by the budget — it slows, it doesn't melt.
- **Proof-of-Play:** power is **not** reward-bearing and does **not** bypass Proof-of-Play. Electric tools/furnace still run the per-strike HMAC on mining; the Electric Furnace changes smelting speed, never drop economics. No anti-cheat surface beyond standard server authority.

---

## 11. Open questions / deferrals (recommendations stated — owner may veto)

1. **Wire decay?** **Recommendation: no distance decay** (real insulated cable doesn't drop a volt per metre at this scale; it's the grounded differentiator vs redstone). Network limits come from **source capacity** (Phase 3), not artificial wire falloff. Revisit at Phase-1 playtest — Axolittle knows redstone's 15-block rule and may want a familiar limit; easy to add as a meta-strength decay if so.
2. **Analog strength (0–15).** Field is reserved (`energised: u8`); Phase 1 treats `>0` as on. Dimmer lamp / motor-speed / sensor-threshold land in Phase 3.
3. **Bundled / coloured cables** (channel isolation) — later; not Phase 1.
4. ~~**Water Wheel** waits on directional water flow~~ — **RESOLVED / DELIVERED 2026-09-06.** `water::flow_vector` (2026-07-04) supplied the current query, and the wheel shipped on it: it turns while a *flowing* water cell touches one of its four horizontal neighbours or the cell directly below, and stays idle beside still water. ~~**Windmill** waits on a wind mechanic~~ — **RESOLVED / DELIVERED 2026-09-07.** `wind.rs` supplied the deterministic breeze; the mill turns per the hysteresis + exposure rule in §5 Phase 4.
5. **Migrate water onto the shared scheduler?** Tempting cleanup (water currently hand-rolls `notify_block_removed`), but out of scope — note it as a future consolidation once the scheduler is proven.
6. **Aether (wireless)** is a separate future spec; do not build wireless signalling here.

---

## 12. Acceptance criteria (Phase 1)

- [ ] `UpdateScheduler` + `edit_block` land; scattered `pending_block_changes.push` boilerplate retired at the touched call sites.
- [ ] Sparse `block_meta` stores + saves + streams + renders (a directional test block renders by facing); `PROTOCOL_VERSION` bumped once; old saves load unchanged.
- [ ] `BlockEntityData::PowerDevice` + `SavedPowerDevice` + `WorldSave.power_devices` round-trip; `energised` rederives on load.
- [ ] Cable, Hand Crank, Steam Generator, Battery, Lever, Button, Pressure Plate, Logic Gate, Electric Lamp all placeable, craftable **and** `/give`-able; Powered Rail speeds carts.
- [ ] A player can build: **Lever → Cable → Lamp** (toggles), **AND gate** of two levers, **Crank/Generator → Battery → Lamp** (buffered), **Pressure Plate → door-equivalent consumer**.
- [ ] All §8 tests pass; `check.sh` green; bundle stays under the 5 MiB brotli gate.
- [ ] Spec docs (§9) updated in the same session.
- [ ] Test sheet authored (`docs/test-sheets/2026-06-17-electricity-phase1.md`); **Axolittle feel/playtest = the closing gate** (does it feel like real, learnable electrical engineering, not reskinned redstone?).

---

## 13. Memory-rule check

- **Names/mechanics locked** in the cosmology doc (owner, 2026-06-16) — this spec builds them, doesn't re-open them. ✓
- **UK English** throughout (e.g. "energised"). ✓
- **Cross-game lift:** the `UpdateScheduler` + per-block `meta` are **engine-generic primitives** (Signet-boundary-equivalent for the engine) — built without AxeNStax-specific assumptions so other Decented voxel games (and the Aether tier) reuse them. ✓
- **Concrete, not cards:** the `meta` byte unblocks the whole directional-building-blocks backlog; the scheduler generalises water's hack — both are foundations, not bridges. ✓
- **Merge-to-main pre-authorised** for healthy-gate (check.sh green) AxeNStax merges (owner, 2026-05-20) — Phase 1 may merge when green; Axolittle playtest remains the feel gate. ✓

---

## Cross-refs

- Cosmology canon + **locked names/mechanics:** [`../vision/elements-cosmology-long-run.md`](../vision/elements-cosmology-long-run.md)
- Substrate audit (design groundwork): `../research/2026-06-16-engine-content-and-electricity-audit.md` (internal repo)
- Showcase consumer (Phase 2 unblocked): [`2026-06-09-rail-freight-logistics.md`](2026-06-09-rail-freight-logistics.md)
- Building-blocks backlog (unblocked by per-block meta): `../research/2026-06-15-native-bake-in-feature-backlog.md` (internal repo) (#27 slabs, #29 doors)
- Block-entity + workstation precedent this reuses: [`2026-05-18-furnace.md`](2026-05-18-furnace.md)

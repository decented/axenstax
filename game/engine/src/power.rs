//! Power & logic model (Spec 48 — Electricity).
//!
//! The redstone replacement. Binary in Phase 1 (`energised` is 0/1; the `u8`
//! is reserved for Phase-3 analog strength). Three roles:
//! * **Conductor** — `CABLE`: an undirected wire that floods power between
//!   connected cells.
//! * **Source** — a device that, while active, drives power into the cells it
//!   touches (Lever/Button/PressurePlate/HandCrank/SteamGenerator/Battery/
//!   WaterWheel, and a LogicGate whose output is high — gates drive only their
//!   output face).
//! * **Consumer** — a device that reads adjacent power and does work
//!   (ElectricLamp lights; a powered TRACK speeds carts).
//!
//! `energised` (transient, sparse, never saved) is rederived on load by
//! enqueuing every source into the scheduler. Device state lives in
//! `BlockEntityData::PowerDevice` and IS saved.

use crate::meta::Facing;
use crate::wind::WindSample;
use ahash::AHashMap;

/// Worst-case cells evaluated per tick; overflow re-queues for next tick
/// (water's bounded-budget pattern). Tunable.
pub const POWER_BUDGET: usize = 1024;

pub type Pos = (i32, i32, i32);

/// Logic-gate operations (the redstone-logic replacement).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GateOp {
    And,
    Or,
    Not,
    Xor,
}

/// The kind of power device occupying a block-entity slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PowerDeviceKind {
    Lever,
    Button,
    PressurePlate,
    LogicGate,
    HandCrank,
    SteamGenerator,
    Battery,
    ElectricLamp,
    // Phase 2 — sensors. BeamSensor + MotionSensor are inputs (drive power when
    // tripped); Mirror is passive (it only reflects beams in `beam::trace_beam`).
    BeamSensor,
    Mirror,
    MotionSensor,
    // Spec 49 (Explosives). The Blasting Keg is a SINK: a rising-edge power
    // signal lights its fuse (`charge` counts down in the game-loop keg sweep).
    // The Plunger Detonator is a momentary SOURCE — the Button pulse path in a
    // dramatic box (the *cha-CHUNK* plunger).
    BlastingKeg,
    PlungerDetonator,
    // Spec 48 Phase 4 — the Water Wheel is a SOURCE driven by *moving* water:
    // `on` is recomputed each tick from [`has_current_neighbour`]. Appended
    // last; the enum is bincode-positional, so new kinds only ever go here.
    WaterWheel,
    // Wind, Copper & Electricity wave §2.2 — the Windmill is a SOURCE driven by
    // the derived breeze (`crate::wind`): `on` is recomputed each tick from the
    // wind speed at its height plus [`windmill_is_exposed`]. Appended last, for
    // the same bincode-positional reason.
    Windmill,
}

/// Spec 49 (Explosives) — the Blasting Keg fuse length, in ticks (~4 s @ 20 TPS).
/// Stored in `PowerDeviceData.charge` once lit (by the Magnesium Firestarter or a
/// rising-edge power pulse) and counted down in the game-loop keg sweep. Tunable
/// const — Axolittle tunes the final feel at playtest.
pub const KEG_FUSE_TICKS: u32 = 80;

/// Per-device state stored in `BlockEntityData::PowerDevice` (saved).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PowerDeviceData {
    pub kind: PowerDeviceKind,
    #[serde(default)]
    pub facing: Facing,
    /// Source latch / gate output / "currently driving" flag.
    #[serde(default)]
    pub on: bool,
    /// Battery charge or HandCrank run-down counter, in ticks. 0 otherwise.
    #[serde(default)]
    pub charge: u32,
    /// SteamGenerator fuel state (reuses the furnace burn machine). None for
    /// every other device.
    #[serde(default)]
    pub fuel: Option<crate::furnace::FurnaceData>,
    /// LogicGate operation; ignored by every other kind.
    #[serde(default = "default_gate_op")]
    pub gate_op: GateOp,
    /// Mirror reflection mode (Spec 48 Phase 2): `false` = retroreflect 180°
    /// (bounce straight back), `true` = turn 90° toward `facing`. Right-click
    /// cycles it. Ignored by every non-Mirror kind.
    #[serde(default)]
    pub mirror_turn: bool,
}

fn default_gate_op() -> GateOp {
    GateOp::And
}

impl PowerDeviceData {
    /// A bare device of `kind` facing `facing`, off, no charge/fuel.
    pub fn new(kind: PowerDeviceKind, facing: Facing) -> Self {
        PowerDeviceData {
            kind,
            facing,
            on: false,
            charge: 0,
            fuel: None,
            gate_op: GateOp::And,
            mirror_turn: false,
        }
    }
}

/// Transient power state: which conductor cells are currently energised.
/// Never serialised — rederived on load.
#[derive(Default)]
pub struct PowerState {
    pub energised: AHashMap<Pos, u8>,
    /// The subset of `energised` driven by a source that is NOT a battery —
    /// "mains", as opposed to a cable a battery is holding up on its own
    /// reserve. Written by the same component flood that fills `energised`, and
    /// read only by [`battery_is_fed`], which is why a battery can tell its own
    /// output apart from a supply (see that function). Transient, like
    /// `energised`, and rederived by `reseed_on_load`.
    pub mains: AHashSet<Pos>,
}

impl PowerState {
    /// Signal level at `pos` (0 if absent). Phase 1 is binary (`>0` == on).
    pub fn level(&self, pos: Pos) -> u8 {
        self.energised.get(&pos).copied().unwrap_or(0)
    }
    pub fn is_on(&self, pos: Pos) -> bool {
        self.level(pos) > 0
    }
}

/// Evaluate a logic gate over its boolean inputs.
/// * `And` — every input high (and at least one input).
/// * `Or`  — any input high.
/// * `Xor` — an odd number of inputs high.
/// * `Not` — no input high (single-input inversion; NOR-like for many).
pub fn eval_gate(op: GateOp, inputs: &[bool]) -> bool {
    match op {
        GateOp::And => !inputs.is_empty() && inputs.iter().all(|&b| b),
        GateOp::Or => inputs.iter().any(|&b| b),
        GateOp::Xor => inputs.iter().filter(|&&b| b).count() % 2 == 1,
        GateOp::Not => !inputs.iter().any(|&b| b),
    }
}

use crate::block_update::ScheduleKind;
use crate::protocol::BlockChange;
use crate::world::{BlockEntityData, World};
use ahash::AHashSet;

const NEIGHBOURS: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

fn neighbours(p: Pos) -> [Pos; 6] {
    let mut out = [(0, 0, 0); 6];
    for (i, n) in NEIGHBOURS.iter().enumerate() {
        out[i] = (p.0 + n.0, p.1 + n.1, p.2 + n.2);
    }
    out
}

/// The device a freshly-placed power block gets, or `None` for a power block
/// that carries no block-entity at all (a CABLE is pure conductor).
///
/// One table, three callers: the client place arm (`game_loop`), the host
/// applying a joined client's placement (`hosted_server`), and the test
/// harness. Before it existed the client's copy ended in a `_ => ElectricLamp`
/// catch-all, so any power block someone forgot to list quietly became a lamp.
/// `BlockId` is a `u16`, so the compiler cannot make this exhaustive — the
/// `every_power_block_maps_to_a_device_kind` integration test does, by walking
/// the registry and failing on any power block that lands in the wildcard.
pub fn device_kind_for_block(block: crate::block::BlockId) -> Option<PowerDeviceKind> {
    use crate::block as b;
    use PowerDeviceKind as K;
    Some(match block {
        b::CABLE | b::CABLE_LIT => return None,
        b::ELECTRIC_LAMP | b::ELECTRIC_LAMP_LIT => K::ElectricLamp,
        b::LEVER => K::Lever,
        b::BUTTON => K::Button,
        b::PRESSURE_PLATE => K::PressurePlate,
        b::LOGIC_GATE => K::LogicGate,
        b::HAND_CRANK => K::HandCrank,
        b::STEAM_GENERATOR | b::STEAM_GENERATOR_LIT => K::SteamGenerator,
        b::BATTERY => K::Battery,
        b::BEAM_SENSOR => K::BeamSensor,
        b::MIRROR => K::Mirror,
        b::MOTION_SENSOR => K::MotionSensor,
        // Spec 49 (Explosives) — keg sink + plunger source.
        b::BLASTING_KEG => K::BlastingKeg,
        b::PLUNGER_DETONATOR => K::PlungerDetonator,
        // Spec 48 Phase 4 — Water Wheel; Wind wave §2.2 — Windmill.
        b::WATER_WHEEL | b::WATER_WHEEL_TURNING => K::WaterWheel,
        b::WINDMILL | b::WINDMILL_TURNING => K::Windmill,
        _ => return None,
    })
}

/// Is this device currently driving power into the cells it touches?
pub fn is_active_source(d: &PowerDeviceData) -> bool {
    match d.kind {
        PowerDeviceKind::Lever
        | PowerDeviceKind::Button
        | PowerDeviceKind::PressurePlate
        | PowerDeviceKind::LogicGate
        // Spec 49 — the Plunger is a Button-style momentary source.
        | PowerDeviceKind::PlungerDetonator => d.on,
        PowerDeviceKind::HandCrank | PowerDeviceKind::Battery => d.charge > 0,
        PowerDeviceKind::SteamGenerator => d.fuel.as_ref().is_some_and(|f| f.lit),
        // Phase 2 — a Beam/Motion Sensor drives power while tripped (`on` is set
        // each tick by the beam-break / proximity check). A Mirror is passive
        // (it only reflects beams in `beam::trace_beam`), never a source.
        PowerDeviceKind::BeamSensor | PowerDeviceKind::MotionSensor => d.on,
        // Spec 48 Phase 4 — a Water Wheel drives power while it is turning
        // (`on` is set from the adjacent current in the device sweep).
        // Wind wave §2.2 — a Windmill does the same, off the breeze.
        PowerDeviceKind::WaterWheel | PowerDeviceKind::Windmill => d.on,
        // Spec 49 — the Blasting Keg is a pure sink (never drives the network).
        PowerDeviceKind::ElectricLamp
        | PowerDeviceKind::Mirror
        | PowerDeviceKind::BlastingKeg => false,
    }
}

/// Does an active source at `src` drive power into `target`? Non-gate sources
/// drive all six neighbours; a logic gate drives only its output face.
fn drives_cell(src: Pos, d: &PowerDeviceData, target: Pos) -> bool {
    match d.kind {
        PowerDeviceKind::LogicGate => {
            let (dx, dy, dz) = d.facing.offset();
            (src.0 + dx, src.1 + dy, src.2 + dz) == target
        }
        _ => neighbours(src).contains(&target),
    }
}

/// Is `cell` directly driven by an active source in one of its six neighbours?
fn driven_by_neighbour_source(world: &World, cell: Pos) -> bool {
    neighbours(cell).iter().any(|&n| {
        world
            .power_device_at(n)
            .is_some_and(|d| is_active_source(d) && drives_cell(n, d, cell))
    })
}

/// Does `cell` have at least one energised cable among its six neighbours?
fn has_energised_cable_neighbour(world: &World, cell: Pos) -> bool {
    neighbours(cell).iter().any(|&n| {
        crate::block::is_cable(world.get_block(n.0, n.1, n.2)) && world.power.is_on(n)
    })
}

/// Is the conductor/input cell itself carrying power right now? An energised
/// cable, or a cell directly driven by an adjacent source.
fn is_powered(world: &World, cell: Pos) -> bool {
    (crate::block::is_cable(world.get_block(cell.0, cell.1, cell.2)) && world.power.is_on(cell))
        || driven_by_neighbour_source(world, cell)
}

/// Should a consumer (lamp, rail) at `pos` be on? It reads adjacent power: an
/// energised cable next to it, or a source driving it directly.
fn consumer_powered(world: &World, pos: Pos) -> bool {
    has_energised_cable_neighbour(world, pos) || driven_by_neighbour_source(world, pos)
}

/// Public power query for non-lamp consumers (P11 pistons): is the block at
/// `pos` currently being driven by the Electricity grid (an energised cable
/// neighbour or an adjacent active source)?
pub fn is_block_powered(world: &World, pos: Pos) -> bool {
    consumer_powered(world, pos)
}

/// Spec 48 Phase 4 — is a **flowing** water cell touching `pos`? True when any
/// of the four horizontal neighbours, or the cell directly below, is water with
/// a current ([`crate::water::flow_vector`] returns a direction). Still water —
/// a pond or a lone source block — returns `None` from `flow_vector`, so it
/// never turns a wheel: the teaching point is that a wheel needs a *stream*.
///
/// The cell above is deliberately excluded: water falling onto the wheel's top
/// face reads as a shower, not as a race, and including it would let a single
/// bucket poured on the roof power a circuit forever.
pub fn has_current_neighbour(world: &World, pos: Pos) -> bool {
    const CURRENT_CELLS: [(i32, i32, i32); 5] = [
        (1, 0, 0),
        (-1, 0, 0),
        (0, 0, 1),
        (0, 0, -1),
        (0, -1, 0),
    ];
    CURRENT_CELLS.iter().any(|&(dx, dy, dz)| {
        crate::water::flow_vector(world, pos.0 + dx, pos.1 + dy, pos.2 + dz).is_some()
    })
}

/// The input cells a gate reads, by orientation + op. 2-input gates read the
/// two horizontal cells perpendicular to the output face; NOT reads the single
/// cell behind the output.
fn gate_input_cells(pos: Pos, facing: Facing, op: GateOp) -> Vec<Pos> {
    let off = |f: Facing| {
        let (dx, dy, dz) = f.offset();
        (pos.0 + dx, pos.1 + dy, pos.2 + dz)
    };
    match op {
        GateOp::Not => vec![off(facing.opposite())],
        _ => match facing {
            // Output along X → inputs along Z; output along Z → inputs along X;
            // vertical gates fall back to the N/S pair.
            Facing::East | Facing::West => vec![off(Facing::North), off(Facing::South)],
            Facing::North | Facing::South => vec![off(Facing::East), off(Facing::West)],
            Facing::Up | Facing::Down => vec![off(Facing::North), off(Facing::South)],
        },
    }
}

/// A gate's intended output from its inputs' current power.
fn gate_output(world: &World, pos: Pos, facing: Facing, op: GateOp) -> bool {
    let inputs: Vec<bool> = gate_input_cells(pos, facing, op)
        .into_iter()
        .map(|ic| is_powered(world, ic))
        .collect();
    eval_gate(op, &inputs)
}

/// Is the Battery at `pos` being charged by a source that is not itself a
/// battery?
///
/// A battery cannot use plain [`consumer_powered`], because a battery DRIVES
/// its six neighbours: wire one to a cable and the cable it energises reads
/// straight back as a supply, so it tops itself up every tick and never runs
/// down — a perpetual-motion generator, found by the Spec 48 end-to-end audit.
///
/// **The rule: nothing that is itself a battery ever charges a battery.** That
/// closes the two-battery version of the same loop (A and B keeping each other
/// alive for ever) without any cycle-chasing, and it is a rule a builder can
/// hold in their head. The cost is that batteries do not chain — a generator
/// feeding A does not fill B behind it; hang both off the same cable run
/// instead. Everything else charges normally: a generator, wheel, mill, crank
/// or switch anywhere on a cable component that touches the battery.
///
/// Six lookups, no flood: the "is this component on mains?" question is
/// answered once per component by [`recompute`]'s existing walk, which records
/// the answer in [`PowerState::mains`].
///
/// Public so the hover label can say "charging" on exactly the ticks the device
/// sweep actually charges it, rather than on the plain
/// [`is_block_powered`] answer — which a battery satisfies with its OWN output.
pub fn battery_is_fed(world: &World, pos: Pos) -> bool {
    neighbours(pos).iter().any(|&n| {
        // Straight off an adjacent non-battery source…
        let direct = world.power_device_at(n).is_some_and(|d| {
            d.kind != PowerDeviceKind::Battery && is_active_source(d) && drives_cell(n, d, pos)
        });
        // …or off a cable that something other than a battery is holding up.
        direct
            || (crate::block::is_cable(world.get_block(n.0, n.1, n.2))
                && world.power.mains.contains(&n))
    })
}

/// Swap the block at `p` to `want` and queue the broadcast for it, carrying the
/// cell's CURRENT metadata byte.
///
/// The power sim's flips are lit↔unlit twin swaps (generator, lamp, wheel,
/// mill) and cable lit swaps. Every one of those blocks wears a facing byte
/// written at placement, and `BlockChange.meta` is the wire's only carrier for
/// it — so building the change with a literal `meta: 0` (which is what the old
/// `BlockChange::new` quietly did) meant a Windmill catching the wind turned to
/// face north on the host and on every other client. `set_block` never touches
/// metadata, so reading the byte back after the swap is exactly the byte the
/// block already had. Mirrors
/// `game_loop::broadcast_change` on the client side; this is the server/sim
/// side of the same rule.
fn swap_and_broadcast(
    world: &mut World,
    p: Pos,
    want: crate::block::BlockId,
    out: &mut Vec<BlockChange>,
) {
    world.set_block(p.0, p.1, p.2, want);
    let meta = world.meta_at(p.0, p.1, p.2);
    out.push(BlockChange::with_meta(p.0, p.1, p.2, want, meta));
}

/// Land a turning source's new state (Water Wheel, Windmill): both are
/// recomputed from the world every tick and both wear their state as a pair of
/// block ids, so the "did it change?" bookkeeping is identical — latch `on`,
/// swap to the matching twin, broadcast that one flip, and re-seed the network
/// around it. On no change this does nothing at all, which is what keeps a
/// steadily-turning mill off the wire (see
/// `windmill_swaps_its_block_exactly_once_per_transition`).
fn apply_turn(
    world: &mut World,
    p: Pos,
    turning: bool,
    was_on: bool,
    turning_block: crate::block::BlockId,
    idle_block: crate::block::BlockId,
    out: &mut Vec<BlockChange>,
) {
    if turning == was_on {
        return;
    }
    if let Some(dm) = world.power_device_at_mut(p) {
        dm.on = turning;
    }
    let want = if turning { turning_block } else { idle_block };
    if world.get_block(p.0, p.1, p.2) != want {
        swap_and_broadcast(world, p, want, out);
    }
    world.mark_dirty(p);
    world.notify_neighbours(p);
}

/// Per-tick device upkeep (the furnace-sweep pattern): generators burn fuel,
/// cranks + batteries count down, pressure plates poll entity presence. Any
/// state change marks the device dirty so the network recomputes, and a
/// generator's lit flip is broadcast.
fn tick_devices(
    world: &mut World,
    entity_positions: &[(f32, f32, f32)],
    wind: WindSample,
    registry: &crate::block::BlockRegistry,
    out: &mut Vec<BlockChange>,
) {
    let positions: Vec<Pos> = world
        .block_entities
        .iter()
        .filter_map(|(p, e)| matches!(e, BlockEntityData::PowerDevice(_)).then_some(*p))
        .collect();

    // Pre-compute which plate cells have an entity standing on them.
    let occupied: AHashSet<Pos> = entity_positions
        .iter()
        .map(|&(x, y, z)| (x.floor() as i32, y.floor() as i32, z.floor() as i32))
        .collect();

    for p in positions {
        let Some(d) = world.power_device_at(p) else {
            continue;
        };
        match d.kind {
            PowerDeviceKind::SteamGenerator => {
                // Burn fuel CONTINUOUSLY (no smelting input) — `tick_burner`, not
                // the furnace `tick_one` which needs a recipe to burn. A lit
                // generator is an active power source (`is_active_source`).
                let mut fuel = d.fuel.clone().unwrap_or_default();
                let lit_changed = crate::furnace::tick_burner(&mut fuel).is_some();
                let now_lit = fuel.lit;
                if let Some(dm) = world.power_device_at_mut(p) {
                    dm.fuel = Some(fuel);
                }
                if lit_changed {
                    let want = if now_lit {
                        crate::block::STEAM_GENERATOR_LIT
                    } else {
                        crate::block::STEAM_GENERATOR
                    };
                    if world.get_block(p.0, p.1, p.2) != want {
                        swap_and_broadcast(world, p, want, out);
                    }
                    world.mark_dirty(p);
                    world.notify_neighbours(p);
                }
            }
            PowerDeviceKind::HandCrank => {
                if d.charge > 0 {
                    let new = d.charge - 1;
                    if let Some(dm) = world.power_device_at_mut(p) {
                        dm.charge = new;
                    }
                    if new == 0 {
                        world.mark_dirty(p);
                    }
                }
            }
            PowerDeviceKind::Battery => {
                // Recharge to full while something ELSE feeds it; otherwise run
                // down one tick (a timed capacitor in Phase 1).
                let fed = battery_is_fed(world, p);
                let cur = d.charge;
                let new = if fed {
                    BATTERY_CAPACITY
                } else {
                    cur.saturating_sub(1)
                };
                if new != cur {
                    if let Some(dm) = world.power_device_at_mut(p) {
                        dm.charge = new;
                    }
                    // Crossing the on/off boundary changes what the battery drives.
                    if (cur > 0) != (new > 0) {
                        world.mark_dirty(p);
                    }
                }
            }
            PowerDeviceKind::PressurePlate => {
                let pressed = occupied.contains(&(p.0, p.1 + 1, p.2)) || occupied.contains(&p);
                if pressed != d.on {
                    if let Some(dm) = world.power_device_at_mut(p) {
                        dm.on = pressed;
                    }
                    world.mark_dirty(p);
                }
            }
            PowerDeviceKind::BeamSensor => {
                // Trace the beam; it triggers when ARMED and an entity is
                // crossing a path cell (tripwire). An entity's feet cell breaks
                // both that cell and the one above (its ~2-tall body).
                let facing = d.facing;
                let trace = sensor_beam(world, p, facing);
                let broken = trace.cells.iter().any(|c| {
                    occupied.contains(c) || occupied.contains(&(c.0, c.1 - 1, c.2))
                });
                let tripped = trace.armed && broken;
                if tripped != world.power_device_at(p).is_some_and(|x| x.on) {
                    if let Some(dm) = world.power_device_at_mut(p) {
                        dm.on = tripped;
                    }
                    world.mark_dirty(p);
                }
            }
            PowerDeviceKind::WaterWheel => {
                // Spec 48 Phase 4 — the wheel turns only in a CURRENT. Still
                // water (a pond, a lone source block) has no flow vector, so a
                // builder has to cut a channel and let the water run.
                let turning = has_current_neighbour(world, p);
                apply_turn(
                    world,
                    p,
                    turning,
                    d.on,
                    crate::block::WATER_WHEEL_TURNING,
                    crate::block::WATER_WHEEL,
                    out,
                );
            }
            PowerDeviceKind::Windmill => {
                // Wind wave §2.2 — the mill turns on the derived breeze at its
                // own height, but only out in the open, and with a dead band so
                // a mill sitting on the threshold doesn't chatter.
                let turning = windmill_turns(world, p, wind, d.on, registry);
                apply_turn(
                    world,
                    p,
                    turning,
                    d.on,
                    crate::block::WINDMILL_TURNING,
                    crate::block::WINDMILL,
                    out,
                );
            }
            PowerDeviceKind::MotionSensor => {
                // Beamless PIR — trips while any entity is within MOTION_RADIUS.
                let here = (p.0 as f32 + 0.5, p.1 as f32 + 0.5, p.2 as f32 + 0.5);
                let near = entity_positions.iter().any(|&(x, y, z)| {
                    let (dx, dy, dz) = (x - here.0, y - here.1, z - here.2);
                    dx * dx + dy * dy + dz * dz <= MOTION_RADIUS * MOTION_RADIUS
                });
                if near != d.on {
                    if let Some(dm) = world.power_device_at_mut(p) {
                        dm.on = near;
                    }
                    world.mark_dirty(p);
                }
            }
            _ => {}
        }
    }
}

/// One battery's run-down window, in ticks (~3 s @ 20 TPS). Phase-1 capacitor;
/// Phase 3 makes this a quantitative accumulator.
pub const BATTERY_CAPACITY: u32 = 60;
/// How long a hand crank drives after one turn, in ticks.
pub const CRANK_RUN_TICKS: u32 = 40;
/// Motion Sensor trip radius, in blocks (Spec 48 Phase 2). Tunable in playtest.
pub const MOTION_RADIUS: f32 = 3.0;

/// Wind, Copper & Electricity wave §2.2 — the wind speed at which an exposed
/// Windmill starts turning, and the (lower) speed at which a turning one gives
/// up. The gap is deliberate hysteresis: the derived breeze drifts across a
/// threshold slowly, and without a dead band a mill parked on 0.35 would swap
/// its block id — and re-broadcast it — several times a minute. Referenced by
/// name from `wind`'s duty-cycle tests.
pub const WINDMILL_START: f32 = 0.35;
pub const WINDMILL_STOP: f32 = 0.30;

/// How far above a Windmill the sky check looks for something in the way.
pub const WINDMILL_SKY_SCAN: i32 = 8;

/// Does the block at `p` stand between a Windmill and the breeze?
///
/// **Solid** blocks do, per the registry's `solid` flag — the same flag physics
/// and the camera read, so "if you'd bump into it, the wind does too" is a rule
/// a builder can check by walking into it. Non-solid cells (a CABLE running off
/// the mill's top face, a torch, a sapling, snow) blow straight through, which
/// matters because the wire out of a mill has to leave from *somewhere*. Glass
/// and leaves are solid and so do block — a mill under a canopy is becalmed.
///
/// **Fluids** block too, even though the registry calls them non-solid (you
/// swim through them): without this, a mill on the seabed under eight blocks of
/// ocean reads as standing in open air.
fn blocks_wind(world: &World, p: Pos, registry: &crate::block::BlockRegistry) -> bool {
    let b = world.get_block(p.0, p.1, p.2);
    registry.is_solid(b) || crate::block::is_fluid(b)
}

/// Wind, Copper & Electricity wave §2.2 — is the Windmill at `pos` out in the
/// open? Two conditions, both of which a player can read off their own build:
/// * nothing in the way in the [`WINDMILL_SKY_SCAN`] cells directly above (the
///   cell immediately above it is the first of those — a cheap sky check);
/// * at least two of its four horizontal neighbours clear, so the sails have
///   somewhere to sweep.
///
/// A mill walled into a room, buried, or roofed over turns nothing, however
/// hard it is blowing outside.
pub fn windmill_is_exposed(
    world: &World,
    pos: Pos,
    registry: &crate::block::BlockRegistry,
) -> bool {
    const SIDES: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let sky_clear = (1..=WINDMILL_SKY_SCAN)
        .all(|dy| !blocks_wind(world, (pos.0, pos.1 + dy, pos.2), registry));
    let open_sides = SIDES
        .iter()
        .filter(|&&(dx, dz)| !blocks_wind(world, (pos.0 + dx, pos.1, pos.2 + dz), registry))
        .count();
    sky_clear && open_sides >= 2
}

/// Should the Windmill at `pos` be turning next tick? `on` is its current
/// state, which is what makes this hysteretic: a still mill needs
/// [`WINDMILL_START`] to get going, a turning one keeps going down to
/// [`WINDMILL_STOP`]. `wind` is the SEA-LEVEL sample for this tick — the
/// altitude term is re-applied here for the mill's own height, so one sample
/// per tick serves every mill in the world.
pub fn windmill_turns(
    world: &World,
    pos: Pos,
    wind: WindSample,
    on: bool,
    registry: &crate::block::BlockRegistry,
) -> bool {
    if !windmill_is_exposed(world, pos, registry) {
        return false;
    }
    let here = crate::wind::with_altitude(wind, pos.1);
    let threshold = if on { WINDMILL_STOP } else { WINDMILL_START };
    here.speed >= threshold
}

/// Trace a Beam Sensor's beam through the world (Spec 48 Phase 2). Builds the
/// `beam::classify` from the world — a Beam Sensor cell is a terminal, a Mirror
/// retroreflects (or turns 90° toward its `facing` when `mirror_turn`), clear AIR
/// passes, and anything else is an opaque dead-end — then runs the pure
/// `beam::trace_beam`. Used by the tick (trip detection) and the client (render).
pub fn sensor_beam(world: &World, origin: Pos, facing: Facing) -> crate::beam::BeamTrace {
    use crate::beam::BeamCell;
    crate::beam::trace_beam(origin, facing, |pos| {
        let b = world.get_block(pos.0, pos.1, pos.2);
        if b == crate::block::BEAM_SENSOR {
            BeamCell::Sensor
        } else if b == crate::block::MIRROR {
            match world.power_device_at(pos) {
                Some(d) if d.mirror_turn => BeamCell::Turn(d.facing),
                _ => BeamCell::Retroreflector,
            }
        } else if b == crate::block::AIR {
            BeamCell::Pass
        } else {
            BeamCell::Block
        }
    })
}

/// Recompute the energised state of every cable component touched by the dirty
/// cells, then re-evaluate the consumers + gates around them. A connected cable
/// component is energised iff any cable in it is driven by an adjacent source —
/// one driver lights the whole component (binary flood). Breaking a driver
/// recomputes the component from scratch, which subsumes the two-phase removal.
fn recompute(world: &mut World, dirty: &[Pos], now: u64, out: &mut Vec<BlockChange>) {
    // Gather the cable cells reachable from the dirty seeds + the device cells
    // to re-evaluate around them.
    let mut comp_cables: AHashSet<Pos> = AHashSet::new();
    let mut device_cells: AHashSet<Pos> = AHashSet::new();
    let mut stack: Vec<Pos> = Vec::new();

    let is_cable = |w: &World, p: Pos| crate::block::is_cable(w.get_block(p.0, p.1, p.2));

    for &d in dirty {
        device_cells.insert(d);
        if is_cable(world, d) {
            stack.push(d);
        }
        for n in neighbours(d) {
            device_cells.insert(n);
            if is_cable(world, n) {
                stack.push(n);
            }
        }
    }
    while let Some(c) = stack.pop() {
        if comp_cables.len() >= POWER_BUDGET {
            break;
        }
        if !is_cable(world, c) || !comp_cables.insert(c) {
            continue;
        }
        for n in neighbours(c) {
            device_cells.insert(n);
            if is_cable(world, n) && !comp_cables.contains(&n) {
                stack.push(n);
            }
        }
    }

    // Label connected components and compute each one's energised flag, plus
    // whether anything OTHER than a battery is driving it (see `battery_is_fed`
    // — this is the one walk that question gets answered in).
    let mut energised_now: AHashSet<Pos> = AHashSet::new();
    let mut mains_now: AHashSet<Pos> = AHashSet::new();
    let mut seen: AHashSet<Pos> = AHashSet::new();
    for &start in &comp_cables {
        if seen.contains(&start) {
            continue;
        }
        let mut comp: Vec<Pos> = Vec::new();
        let mut driven = false;
        let mut mains = false;
        let mut st = vec![start];
        while let Some(c) = st.pop() {
            if !seen.insert(c) {
                continue;
            }
            comp.push(c);
            for n in neighbours(c) {
                if let Some(d) = world.power_device_at(n)
                    && is_active_source(d)
                    && drives_cell(n, d, c)
                {
                    driven = true;
                    mains |= d.kind != PowerDeviceKind::Battery;
                }
                if comp_cables.contains(&n) && !seen.contains(&n) {
                    st.push(n);
                }
            }
        }
        if driven {
            energised_now.extend(comp.iter().copied());
        }
        if mains {
            mains_now.extend(comp);
        }
    }

    // Apply cable energised changes + emit CABLE ↔ CABLE_LIT visuals. The
    // mains flag is refreshed for every cell in the region (not just the ones
    // that changed): it is a property of what is driving the component, which
    // can change without the lit-ness changing at all.
    for &c in &comp_cables {
        if mains_now.contains(&c) {
            world.power.mains.insert(c);
        } else {
            world.power.mains.remove(&c);
        }
        let now_on = energised_now.contains(&c);
        if now_on != world.power.is_on(c) {
            if now_on {
                world.power.energised.insert(c, 1);
            } else {
                world.power.energised.remove(&c);
            }
            let want = if now_on {
                crate::block::CABLE_LIT
            } else {
                crate::block::CABLE
            };
            swap_and_broadcast(world, c, want, out);
            for n in neighbours(c) {
                device_cells.insert(n);
            }
        }
    }

    // Re-evaluate consumers (lamp, powered rail) + gates around the region.
    for p in device_cells {
        update_consumer_or_gate(world, p, now, out);
    }
}

/// Update a single consumer (lamp / powered rail) or gate at `p`.
fn update_consumer_or_gate(world: &mut World, p: Pos, now: u64, out: &mut Vec<BlockChange>) {
    // Powered rail: TRACK with no block-entity; the powered flag rides meta bit
    // (state field). The cart stepper reads it for a speed multiplier.
    if world.get_block(p.0, p.1, p.2) == crate::rail::TRACK {
        let powered = consumer_powered(world, p);
        let m = world.meta_at(p.0, p.1, p.2);
        let want = crate::meta::with_state(m, if powered { 1 } else { 0 });
        if want != m {
            world.set_meta(p, want);
            out.push(BlockChange::with_meta(p.0, p.1, p.2, crate::rail::TRACK, want));
        }
        return;
    }

    let Some((kind, facing, op, on)) = world
        .power_device_at(p)
        .map(|d| (d.kind, d.facing, d.gate_op, d.on))
    else {
        return;
    };

    match kind {
        PowerDeviceKind::ElectricLamp => {
            let lit = consumer_powered(world, p);
            let want = if lit {
                crate::block::ELECTRIC_LAMP_LIT
            } else {
                crate::block::ELECTRIC_LAMP
            };
            if world.get_block(p.0, p.1, p.2) != want {
                if let Some(dm) = world.power_device_at_mut(p) {
                    dm.on = lit;
                }
                swap_and_broadcast(world, p, want, out);
            }
        }
        PowerDeviceKind::LogicGate => {
            let intended = gate_output(world, p, facing, op);
            if intended != on {
                // Apply the output one tick later — deterministic order, breaks
                // combinational feedback, bounds oscillators.
                world.schedule_update(p, 1, ScheduleKind::GateSettle, now);
            }
        }
        PowerDeviceKind::BlastingKeg => {
            // Spec 49 (Explosives) — rising-edge detection: a fresh power signal
            // (unpowered → powered) lights the fuse exactly once. `on` is the
            // previous powered state; `charge` is the fuse timer (counted down in
            // the game-loop keg sweep). Edge-triggered, not level: a held-on wire
            // fires it once, and a re-pulse on an already-burning keg does NOT
            // reset the timer. Detonation (radius/damage) is the game loop's job.
            let now_powered = consumer_powered(world, p);
            if !on && now_powered {
                if let Some(dm) = world.power_device_at_mut(p) {
                    if dm.charge == 0 {
                        dm.charge = KEG_FUSE_TICKS;
                    }
                    dm.on = true;
                }
            } else if on != now_powered
                && let Some(dm) = world.power_device_at_mut(p) {
                    dm.on = now_powered;
                }
        }
        _ => {}
    }
}

/// Latch a gate's output (called when its scheduled GateSettle comes due), then
/// re-flood from it.
fn settle_gate(world: &mut World, pos: Pos, now: u64) {
    let Some((kind, facing, op)) = world
        .power_device_at(pos)
        .map(|d| (d.kind, d.facing, d.gate_op))
    else {
        return;
    };
    if kind != PowerDeviceKind::LogicGate {
        return;
    }
    let intended = gate_output(world, pos, facing, op);
    if let Some(dm) = world.power_device_at_mut(pos) {
        dm.on = intended;
    }
    let (dx, dy, dz) = facing.offset();
    world.mark_dirty(pos);
    world.notify_neighbours(pos);
    world.mark_dirty((pos.0 + dx, pos.1 + dy, pos.2 + dz));
    let _ = now;
}

/// The whole-power step: device upkeep → delayed events → bounded network
/// recompute. Slots into the server tick after entity physics, before carts.
pub fn power_tick(
    world: &mut World,
    now: u64,
    entity_positions: &[(f32, f32, f32)],
    wind: WindSample,
    registry: &crate::block::BlockRegistry,
) -> Vec<BlockChange> {
    let mut out = Vec::new();

    // (A) Per-tick device upkeep (may mark devices dirty).
    tick_devices(world, entity_positions, wind, registry, &mut out);

    // (B) Delayed scheduled events.
    for u in world.scheduler.drain_due(now) {
        match u.kind {
            ScheduleKind::ButtonRelease => {
                if let Some(dm) = world.power_device_at_mut(u.pos) {
                    // Spec 49 — the Plunger Detonator releases on the same path.
                    if dm.kind == PowerDeviceKind::Button
                        || dm.kind == PowerDeviceKind::PlungerDetonator
                    {
                        dm.on = false;
                    }
                }
                world.notify_neighbours(u.pos);
                world.mark_dirty(u.pos);
            }
            ScheduleKind::GateSettle => settle_gate(world, u.pos, now),
            ScheduleKind::PlateRelease | ScheduleKind::PowerReeval => {}
        }
    }

    // (C) Bounded network recompute over the dirty set.
    let dirty = world.scheduler.take_pending(POWER_BUDGET);
    if !dirty.is_empty() {
        recompute(world, &dirty, now, &mut out);
    }
    out
}

/// Spec 49 (Explosives) — advance every lit Blasting Keg fuse by one tick. A keg
/// is "lit" when its `charge` (the fuse timer) is > 0; the fuse is started by the
/// Magnesium Firestarter (hand) or a rising-edge power pulse (electricity). Returns
/// the positions of kegs whose fuse reached 0 this tick — the caller detonates
/// them (the blast needs the wider game state: audio, lighting, combat). Pure on
/// the `World`, so it's unit-testable without a full client. Runs every game-loop
/// tick, independent of network activity, so a primed fuse always burns down.
pub fn tick_keg_fuses(world: &mut World) -> Vec<Pos> {
    let lit: Vec<Pos> = world
        .block_entities
        .iter()
        .filter_map(|(&p, be)| match be {
            BlockEntityData::PowerDevice(d)
                if d.kind == PowerDeviceKind::BlastingKeg && d.charge > 0 =>
            {
                Some(p)
            }
            _ => None,
        })
        .collect();
    let mut detonations = Vec::new();
    for pos in lit {
        if let Some(d) = world.power_device_at_mut(pos)
            && d.kind == PowerDeviceKind::BlastingKeg && d.charge > 0 {
                d.charge -= 1;
                if d.charge == 0 {
                    detonations.push(pos);
                }
            }
    }
    detonations
}

/// Drive the lighting BFS for a single power-tick block flip. `power_tick`
/// flips emitter blocks (Electric Lamp ↔ lit, Steam Generator ↔ lit) with a
/// bare `set_block`, which does NOT recompute light — so the caller runs this
/// for each returned [`BlockChange`] (it already holds the world + registry).
///
/// The previous block is reconstructed from the lit/unlit pairing (these are the
/// only emitters the power tick toggles), then [`crate::lighting::update_for_block_change`]
/// removes the old emission and propagates the new one. A no-op for every other
/// power block (cable / rail / gate / inputs emit no light), so it is safe to
/// call on every change in the returned vector.
pub fn relight_after_power_change(
    world: &mut World,
    bc: &BlockChange,
    registry: &crate::block::BlockRegistry,
) {
    use crate::block::{
        ELECTRIC_LAMP, ELECTRIC_LAMP_LIT, STEAM_GENERATOR, STEAM_GENERATOR_LIT,
    };
    let prev = match bc.new_block {
        ELECTRIC_LAMP_LIT => ELECTRIC_LAMP,
        ELECTRIC_LAMP => ELECTRIC_LAMP_LIT,
        STEAM_GENERATOR_LIT => STEAM_GENERATOR,
        STEAM_GENERATOR => STEAM_GENERATOR_LIT,
        _ => return,
    };
    crate::lighting::update_for_block_change(
        world,
        (bc.x, bc.y, bc.z),
        prev,
        bc.new_block,
        registry,
    );
}

/// What a right-click on a power device did, for the caller to finish off.
///
/// Deliberately not "and the caller renders it": the two callers need different
/// halves. The client needs `remesh` (its own chunk mesh has to be rebuilt this
/// frame so a lever handle visibly flips); the host needs `changes` (its
/// authoritative flips ride the block-change broadcast). Both get both, and
/// neither reimplements the interaction.
#[derive(Clone, Debug, Default)]
pub struct DeviceInteraction {
    /// Cells whose block or metadata changed and must go on the wire. Today
    /// this is at most the device's own cell (a Lever's latch bit, a Mirror's
    /// facing); the knock-on lit cables and lamps come from the next
    /// `power_tick` over the region this marked dirty.
    pub changes: Vec<BlockChange>,
    /// The device's own cell needs re-meshing (its metadata changed, and the
    /// block's shape reads it). Meaningless server-side.
    pub remesh: bool,
}

/// True iff a right-click on `kind` is a *toggle-class* interaction — one whose
/// whole input is "a player touched this cell".
///
/// These are the kinds [`interact_device`] handles, and the only kinds the
/// `DeviceInteract` packet is allowed to name. Everything else a player can
/// right-click in the power tier needs something a client must not be trusted
/// to assert: fuelling a Steam Generator has to take the unit from the
/// *server's* copy of that player's inventory, and remote inventories are still
/// client-authoritative (CLAUDE.md known debt). Those stay single-player /
/// host-local until that debt is paid.
pub fn is_toggle_class(kind: PowerDeviceKind) -> bool {
    matches!(
        kind,
        PowerDeviceKind::Lever
            | PowerDeviceKind::Button
            | PowerDeviceKind::PlungerDetonator
            | PowerDeviceKind::HandCrank
            | PowerDeviceKind::Mirror
    )
}

/// Apply one right-click to the toggle-class power device at `p`, at tick `now`.
///
/// THE single implementation of "what does a right-click mean here", called by
/// the single-player/host client place arm and by the host's `DeviceInteract`
/// dispatch. It used to be an inline match in `game_loop.rs` that only ever ran
/// on the acting machine, which is why a joiner's lever moved nothing in the
/// world the host actually simulates.
///
/// * **Lever** — latches, and mirrors its new state into metadata bit 0 so the
///   handle shape flips (that metadata byte is also what tells every other
///   client which way the handle is now pointing).
/// * **Button / Plunger Detonator** — a momentary pulse; a `ButtonRelease` is
///   scheduled 10 ticks out to drop it again.
/// * **Hand Crank** — one turn winds `charge` up to [`CRANK_RUN_TICKS`].
/// * **Mirror** — cycles the five reflective configs (retroreflect → turn-N →
///   E → S → W → back) and writes the new facing into metadata.
///
/// Returns `None` when there is no toggle-class device at `p` — the caller
/// treats that as "not mine", which for the host means dropping the packet.
pub fn interact_device(world: &mut World, p: Pos, now: u64) -> Option<DeviceInteraction> {
    let kind = world.power_device_at(p).map(|d| d.kind)?;
    if !is_toggle_class(kind) {
        return None;
    }
    let mut out = DeviceInteraction::default();

    match kind {
        PowerDeviceKind::Lever => {
            let on = {
                let d = world.power_device_at_mut(p)?;
                d.on = !d.on;
                d.on
            };
            // Wave 2c — mirror the latch into meta state bit 0 so the F1 handle
            // shape visibly flips. (Button is a momentary pulse; its nub stays
            // put for v1.)
            let m = crate::meta::with_state(world.meta_at(p.0, p.1, p.2), on as u8);
            world.set_meta(p, m);
            out.changes.push(BlockChange::with_meta(
                p.0,
                p.1,
                p.2,
                world.get_block(p.0, p.1, p.2),
                m,
            ));
            out.remesh = true;
        }
        PowerDeviceKind::Button | PowerDeviceKind::PlungerDetonator => {
            if let Some(d) = world.power_device_at_mut(p) {
                d.on = true;
            }
            world.schedule_update(p, 10, ScheduleKind::ButtonRelease, now);
        }
        PowerDeviceKind::HandCrank => {
            if let Some(d) = world.power_device_at_mut(p) {
                d.charge = CRANK_RUN_TICKS;
            }
        }
        PowerDeviceKind::Mirror => {
            // Spec 48 Phase 2 — cycle the five reflective configs. Beam sensors
            // re-trace every tick, so the new routing is picked up next tick.
            let facing = {
                let d = world.power_device_at_mut(p)?;
                if !d.mirror_turn {
                    d.mirror_turn = true;
                    d.facing = Facing::North;
                } else {
                    d.facing = match d.facing {
                        Facing::North => Facing::East,
                        Facing::East => Facing::South,
                        Facing::South => Facing::West,
                        _ => Facing::North,
                    };
                    if d.facing == Facing::North {
                        d.mirror_turn = false; // completed the loop → retroreflect
                    }
                }
                d.facing
            };
            let m = crate::meta::with_facing(0, facing);
            world.set_meta(p, m);
            out.changes.push(BlockChange::with_meta(
                p.0,
                p.1,
                p.2,
                world.get_block(p.0, p.1, p.2),
                m,
            ));
            out.remesh = true;
        }
        // `is_toggle_class` gates the match above, so this is unreachable
        // today — and it stays a `return None`, never a panic, because the
        // caller is a network packet dispatch. If someone adds a kind to
        // `is_toggle_class` and forgets the arm, the host must drop the packet,
        // not die on it.
        _ => return None,
    }

    world.mark_dirty(p);
    world.notify_neighbours(p);
    Some(out)
}

/// Fold a remote block change's metadata byte back into the device standing at
/// that cell (the joined client's apply path).
///
/// A joiner no longer runs a device interaction itself — it asks the host and
/// waits for the broadcast. But `World::apply_remote_block_change` writes only
/// the block id and its metadata, so the joiner's own `PowerDeviceData.on`
/// would stay stale and its *local* power sim (still a dual-sim — CLAUDE.md
/// known debt) would darken the host's lit wire the next time anything nearby
/// dirtied the network. The Lever is the one device whose state travels in
/// metadata, so it is the one that can be re-derived; a Mirror's retroreflect /
/// turn distinction is not in the byte, so it is deliberately left alone.
pub fn sync_device_from_meta(world: &mut World, p: Pos, meta: u8) {
    if let Some(d) = world.power_device_at_mut(p)
        && d.kind == PowerDeviceKind::Lever
    {
        d.on = crate::meta::state(meta) != 0;
    }
}

/// Try to load ONE unit of `held` into a Steam Generator's fuel slot (Spec 48
/// #4 — the right-click fuel path, mirroring the furnace v1 bridge). Accepts
/// only valid furnace fuels; stacks onto a matching fuel (up to 64), fills an
/// empty slot, or refuses if the slot already holds a *different* fuel. Returns
/// `true` iff a unit was accepted — the caller then removes one from the
/// player's hand. Pure: operates on the device + the held stack, no world.
pub fn try_load_generator_fuel(
    device: &mut PowerDeviceData,
    held: &crate::item::ItemStack,
) -> bool {
    use crate::item::{Item, ItemStack};
    if device.kind != PowerDeviceKind::SteamGenerator {
        return false;
    }
    let (mat, blk) = match held.item {
        Item::Material(m) => (Some(m), None),
        Item::Block(b) => (None, Some(b)),
        _ => return false,
    };
    if crate::furnace::fuel_value(mat, blk).is_none() {
        return false; // not a fuel
    }
    let fd = device.fuel.get_or_insert_with(Default::default);
    match &mut fd.fuel {
        None => {
            fd.fuel = Some(match (mat, blk) {
                (Some(m), _) => ItemStack::new_material(m, 1),
                (_, Some(b)) => ItemStack::new_block(b, 1),
                _ => return false,
            });
            true
        }
        Some(existing) => {
            let same = match (&existing.item, mat, blk) {
                (Item::Material(e), Some(m), _) => *e == m,
                (Item::Block(e), _, Some(b)) => *e == b,
                _ => false,
            };
            if same && existing.count < 64 {
                existing.count += 1;
                true
            } else {
                false
            }
        }
    }
}

/// Rederive transient power state after a world load: clear energised + enqueue
/// every power block so the next `power_tick` recomputes lit cables/lamps from
/// the saved device state. Cheap one-shot scan of the loaded block-entities.
pub fn reseed_on_load(world: &mut World) {
    world.power.energised.clear();
    world.power.mains.clear();
    let positions: Vec<Pos> = world
        .block_entities
        .iter()
        .filter_map(|(p, e)| matches!(e, BlockEntityData::PowerDevice(_)).then_some(*p))
        .collect();
    for p in positions {
        world.mark_dirty(p);
        world.notify_neighbours(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn and_gate_truth_table() {
        assert!(eval_gate(GateOp::And, &[true, true]));
        assert!(!eval_gate(GateOp::And, &[true, false]));
        assert!(!eval_gate(GateOp::And, &[false, false]));
        assert!(!eval_gate(GateOp::And, &[]));
    }

    #[test]
    fn or_gate_truth_table() {
        assert!(!eval_gate(GateOp::Or, &[false, false]));
        assert!(eval_gate(GateOp::Or, &[true, false]));
        assert!(eval_gate(GateOp::Or, &[true, true]));
    }

    #[test]
    fn xor_gate_truth_table() {
        assert!(!eval_gate(GateOp::Xor, &[false, false]));
        assert!(eval_gate(GateOp::Xor, &[true, false]));
        assert!(!eval_gate(GateOp::Xor, &[true, true]));
        assert!(eval_gate(GateOp::Xor, &[true, true, true]));
    }

    #[test]
    fn not_gate_inverts() {
        assert!(eval_gate(GateOp::Not, &[false]));
        assert!(!eval_gate(GateOp::Not, &[true]));
        // No inputs reads as "nothing high" → high out.
        assert!(eval_gate(GateOp::Not, &[]));
    }

    // ── network behaviour (World-driven) ──────────────────────────────────────

    use crate::block;
    use crate::world::World;

    /// One power tick against the stock block registry — the only registry the
    /// game ever builds, and (since the Windmill's wind check reads the `solid`
    /// flag) the one every call below needs. Wraps [`power_tick`] so the tests
    /// stay about the circuit rather than about passing a registry around.
    fn tick(
        w: &mut World,
        now: u64,
        entities: &[(f32, f32, f32)],
        wind: WindSample,
    ) -> Vec<BlockChange> {
        static REG: std::sync::LazyLock<crate::block::BlockRegistry> =
            std::sync::LazyLock::new(crate::block::BlockRegistry::new);
        power_tick(w, now, entities, wind, &REG)
    }

    fn dev(w: &mut World, p: Pos, blk: u16, kind: PowerDeviceKind, facing: Facing) {
        w.set_block(p.0, p.1, p.2, blk);
        w.block_entities
            .insert(p, BlockEntityData::PowerDevice(PowerDeviceData::new(kind, facing)));
        w.mark_dirty(p);
        w.notify_neighbours(p);
    }

    fn set_on(w: &mut World, p: Pos, on: bool) {
        if let Some(d) = w.power_device_at_mut(p) {
            d.on = on;
        }
        w.mark_dirty(p);
        w.notify_neighbours(p);
    }

    #[test]
    fn lever_cable_lamp_lights_and_clears() {
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::LEVER, PowerDeviceKind::Lever, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        w.set_block(2, 0, 0, block::CABLE);
        dev(&mut w, (3, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);
        set_on(&mut w, (0, 0, 0), true);

        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(1, 0, 0), block::CABLE_LIT, "near cable lit");
        assert_eq!(w.get_block(2, 0, 0), block::CABLE_LIT, "far cable lit");
        assert_eq!(w.get_block(3, 0, 0), block::ELECTRIC_LAMP_LIT, "lamp lit");

        set_on(&mut w, (0, 0, 0), false);
        tick(&mut w, 2, &[], WindSample::CALM);
        assert_eq!(w.get_block(1, 0, 0), block::CABLE, "cable dark");
        assert_eq!(w.get_block(3, 0, 0), block::ELECTRIC_LAMP, "lamp dark");
    }

    #[test]
    fn manual_break_sequence_kills_the_network_and_the_ghost_device() {
        // Spec 48 §2.3 regression — the game_loop manual-break path must BOTH
        // remove a broken device's PowerDevice entity AND notify neighbours.
        // Power sources are entity-driven (`driven_by_neighbour_source` never
        // checks the block id), so `set_block(AIR)` alone leaves a GHOST
        // lever that keeps powering the network forever. This pins why the
        // break arms in game_loop.rs do the remove + notify pair.
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::LEVER, PowerDeviceKind::Lever, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        dev(&mut w, (2, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);
        set_on(&mut w, (0, 0, 0), true);
        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(2, 0, 0), block::ELECTRIC_LAMP_LIT, "lamp lit");

        // The pre-fix break behaviour: block cleared, entity + notify forgotten.
        w.set_block(0, 0, 0, block::AIR);
        tick(&mut w, 2, &[], WindSample::CALM);
        assert_eq!(
            w.get_block(2, 0, 0),
            block::ELECTRIC_LAMP_LIT,
            "ghost device keeps the lamp lit — why the break path MUST remove the entity"
        );

        // The fixed break sequence (game_loop.rs break arms).
        w.block_entities.remove(&(0, 0, 0));
        w.notify_neighbours((0, 0, 0));
        tick(&mut w, 3, &[], WindSample::CALM);
        tick(&mut w, 4, &[], WindSample::CALM);
        assert_eq!(w.get_block(1, 0, 0), block::CABLE, "cable settles dark");
        assert_eq!(w.get_block(2, 0, 0), block::ELECTRIC_LAMP, "lamp settles dark");
    }

    #[test]
    fn cutting_a_cable_with_neighbour_notify_darkens_downstream() {
        // Spec 48 §2.3 regression — the other half of the manual-break fix:
        // cutting a CABLE mid-run (no device entity involved) + neighbour
        // notify must darken everything downstream. Before the game_loop
        // break arms notified, a cut cable's network never re-evaluated and
        // the downstream stayed lit forever.
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::LEVER, PowerDeviceKind::Lever, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        w.set_block(2, 0, 0, block::CABLE);
        dev(&mut w, (3, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);
        set_on(&mut w, (0, 0, 0), true);
        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(3, 0, 0), block::ELECTRIC_LAMP_LIT, "lamp lit");

        // Cut the near cable the way the game_loop break arm does.
        w.set_block(1, 0, 0, block::AIR);
        w.notify_neighbours((1, 0, 0));
        tick(&mut w, 2, &[], WindSample::CALM);
        tick(&mut w, 3, &[], WindSample::CALM);
        assert_eq!(w.get_block(2, 0, 0), block::CABLE, "downstream cable dark after the cut");
        assert_eq!(w.get_block(3, 0, 0), block::ELECTRIC_LAMP, "lamp dark after the cut");
    }

    #[test]
    fn try_load_generator_fuel_accepts_stacks_and_rejects() {
        use crate::item::{ItemStack, MaterialId};
        let mut device = PowerDeviceData::new(PowerDeviceKind::SteamGenerator, Facing::Up);
        // Coal is fuel → loads into the empty slot.
        assert!(try_load_generator_fuel(&mut device, &ItemStack::new_material(MaterialId::Coal, 9)));
        assert_eq!(device.fuel.as_ref().unwrap().fuel.as_ref().unwrap().count, 1);
        // Same fuel stacks.
        assert!(try_load_generator_fuel(&mut device, &ItemStack::new_material(MaterialId::Coal, 9)));
        assert_eq!(device.fuel.as_ref().unwrap().fuel.as_ref().unwrap().count, 2);
        // A non-fuel (raw beef) is refused.
        assert!(!try_load_generator_fuel(&mut device, &ItemStack::new_material(MaterialId::RawBeef, 1)));
        // A different fuel is refused while the slot holds coal.
        let before = device.fuel.as_ref().unwrap().fuel.as_ref().unwrap().count;
        assert!(!try_load_generator_fuel(&mut device, &ItemStack::new_block(crate::block::OAK_PLANKS, 1)));
        assert_eq!(device.fuel.as_ref().unwrap().fuel.as_ref().unwrap().count, before);
        // Not a generator → never loads.
        let mut lever = PowerDeviceData::new(PowerDeviceKind::Lever, Facing::Up);
        assert!(!try_load_generator_fuel(&mut lever, &ItemStack::new_material(MaterialId::Coal, 1)));
    }

    #[test]
    fn fuelled_steam_generator_drives_a_lamp() {
        // Spec 48 #4 — a Steam Generator with fuel in its slot burns it (no
        // smelting input needed) and, while lit, powers its neighbours.
        let mut w = World::new();
        let gen_pos = (0, 0, 0);
        dev(&mut w, gen_pos, block::STEAM_GENERATOR, PowerDeviceKind::SteamGenerator, Facing::Up);
        if let Some(dm) = w.power_device_at_mut(gen_pos) {
            dm.fuel = Some(crate::furnace::FurnaceData {
                fuel: Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Coal, 1)),
                ..Default::default()
            });
        }
        // Lamp directly adjacent — a non-gate source drives all six neighbours.
        dev(&mut w, (1, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(0, 0, 0), block::STEAM_GENERATOR_LIT, "generator lights once fuelled");
        assert_eq!(w.get_block(1, 0, 0), block::ELECTRIC_LAMP_LIT, "fuelled generator powers the lamp");
    }

    // ── Spec 48 Phase 4 — Water Wheel (stream-driven source) ─────────────────

    /// Lay a short race of WATER with an increasing depth level over solid
    /// ground. A depth gradient is exactly what `water::flow_vector` reads as a
    /// current, so this is the minimal "moving water" fixture.
    fn flowing_race(w: &mut World, cells: &[Pos]) {
        for (i, &(x, y, z)) in cells.iter().enumerate() {
            w.set_block(x, y - 1, z, block::STONE);
            w.set_block(x, y, z, block::WATER);
            w.set_meta((x, y, z), crate::meta::with_aux(0, i as u8));
        }
    }

    /// A still pool: uniform depth over solid ground, nowhere to fall.
    fn still_pool(w: &mut World, cells: &[Pos]) {
        for &(x, y, z) in cells {
            w.set_block(x, y - 1, z, block::STONE);
            w.set_block(x, y, z, block::WATER);
            w.set_meta((x, y, z), crate::meta::with_aux(0, 0));
        }
    }

    /// Build height for the water fixtures — `World::set_block` only takes
    /// effect at y >= 0, so the races sit at a realistic ground level.
    const WY: i32 = 64;

    #[test]
    fn water_wheel_turns_in_a_current_and_drives_a_lamp() {
        // Spec 48 Phase 4 — the wheel beside a running race turns, swaps to its
        // turning face, and drives the cable/lamp run like any other source.
        let mut w = World::new();
        flowing_race(&mut w, &[(0, WY, 0), (1, WY, 0), (2, WY, 0)]);
        dev(&mut w, (1, WY, 1), block::WATER_WHEEL, PowerDeviceKind::WaterWheel, Facing::Up);
        w.set_block(1, WY, 2, block::CABLE);
        dev(&mut w, (1, WY, 3), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        tick(&mut w, 1, &[], WindSample::CALM);
        assert!(w.power_device_at((1, WY, 1)).unwrap().on, "a current turns the wheel");
        assert_eq!(
            w.get_block(1, WY, 1),
            block::WATER_WHEEL_TURNING,
            "turning wheel swaps to its spinning face"
        );
        assert_eq!(w.get_block(1, WY, 2), block::CABLE_LIT, "wheel energises the cable");
        assert_eq!(w.get_block(1, WY, 3), block::ELECTRIC_LAMP_LIT, "wheel lights the lamp");
    }

    #[test]
    fn still_water_never_turns_the_wheel() {
        // The whole teaching point: a pond is not a stream. A uniform pool has
        // no flow vector, so the wheel stays idle and the lamp stays dark.
        let mut w = World::new();
        still_pool(
            &mut w,
            &[
                (0, WY, 0), (1, WY, 0), (2, WY, 0),
                (0, WY, -1), (1, WY, -1), (2, WY, -1),
            ],
        );
        dev(&mut w, (1, WY, 1), block::WATER_WHEEL, PowerDeviceKind::WaterWheel, Facing::Up);
        dev(&mut w, (1, WY, 2), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        tick(&mut w, 1, &[], WindSample::CALM);
        assert!(!w.power_device_at((1, WY, 1)).unwrap().on, "a still pool drives nothing");
        assert_eq!(w.get_block(1, WY, 1), block::WATER_WHEEL, "idle wheel keeps its idle face");
        assert_eq!(w.get_block(1, WY, 2), block::ELECTRIC_LAMP, "lamp stays dark beside a pond");
    }

    #[test]
    fn a_current_directly_below_turns_the_wheel() {
        // An undershot wheel: the race runs under the wheel rather than beside
        // it. The cell directly below counts; the cell above deliberately does
        // not (see `has_current_neighbour`).
        let mut w = World::new();
        flowing_race(&mut w, &[(0, WY, 0), (1, WY, 0), (2, WY, 0)]);
        dev(&mut w, (1, WY + 1, 0), block::WATER_WHEEL, PowerDeviceKind::WaterWheel, Facing::Up);
        tick(&mut w, 1, &[], WindSample::CALM);
        assert!(
            w.power_device_at((1, WY + 1, 0)).unwrap().on,
            "a race running under the wheel turns it"
        );

        // The same race running ABOVE the wheel is a shower, not a race.
        let mut w2 = World::new();
        flowing_race(&mut w2, &[(0, WY, 0), (1, WY, 0), (2, WY, 0)]);
        dev(&mut w2, (1, WY - 2, 0), block::WATER_WHEEL, PowerDeviceKind::WaterWheel, Facing::Up);
        tick(&mut w2, 1, &[], WindSample::CALM);
        assert!(
            !w2.power_device_at((1, WY - 2, 0)).unwrap().on,
            "water on the roof is not a race"
        );
    }

    #[test]
    fn a_current_that_dries_up_stops_the_wheel_and_darkens_downstream() {
        let mut w = World::new();
        let race = [(0, WY, 0), (1, WY, 0), (2, WY, 0)];
        flowing_race(&mut w, &race);
        dev(&mut w, (1, WY, 1), block::WATER_WHEEL, PowerDeviceKind::WaterWheel, Facing::Up);
        w.set_block(1, WY, 2, block::CABLE);
        dev(&mut w, (1, WY, 3), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);
        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(1, WY, 3), block::ELECTRIC_LAMP_LIT, "lamp lit while the race runs");

        // The race dries up.
        for &(x, y, z) in &race {
            w.set_block(x, y, z, block::AIR);
        }
        tick(&mut w, 2, &[], WindSample::CALM);
        assert!(!w.power_device_at((1, WY, 1)).unwrap().on, "no current, no turn");
        assert_eq!(w.get_block(1, WY, 1), block::WATER_WHEEL, "wheel swaps back to idle");
        assert_eq!(w.get_block(1, WY, 2), block::CABLE, "cable settles dark");
        assert_eq!(w.get_block(1, WY, 3), block::ELECTRIC_LAMP, "lamp settles dark");
    }

    // ── Windmill (Wind, Copper & Electricity wave §2.2) ──────────────────────

    /// Mills sit at sea level so `wind::with_altitude` contributes exactly
    /// nothing and the speeds in these tests are the speeds the rule sees.
    const MY: i32 = crate::biome::SEA_LEVEL;

    /// A breeze of `speed`, blowing north. Direction is presentation only —
    /// the Windmill is omnidirectional in this wave.
    fn gust(speed: f32) -> WindSample {
        WindSample { speed, direction: 0 }
    }

    fn windmill(w: &mut World, p: Pos) {
        dev(w, p, block::WINDMILL, PowerDeviceKind::Windmill, Facing::Up);
    }

    #[test]
    fn windmill_turns_in_a_storm_when_exposed_and_drives_a_lamp() {
        // The dry twin of `water_wheel_turns_in_a_current_and_drives_a_lamp`:
        // open sky, a storm blowing, so the mill turns, swaps to its spinning
        // face, and drives the cable/lamp run like any other source.
        let mut w = World::new();
        windmill(&mut w, (1, MY, 1));
        w.set_block(1, MY, 2, block::CABLE);
        dev(&mut w, (1, MY, 3), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        tick(&mut w, 1, &[], gust(0.70));
        assert!(w.power_device_at((1, MY, 1)).unwrap().on, "a storm turns an exposed mill");
        assert_eq!(
            w.get_block(1, MY, 1),
            block::WINDMILL_TURNING,
            "a turning mill swaps to its spinning face"
        );
        assert_eq!(w.get_block(1, MY, 2), block::CABLE_LIT, "the mill energises the cable");
        assert_eq!(w.get_block(1, MY, 3), block::ELECTRIC_LAMP_LIT, "the mill lights the lamp");
    }

    #[test]
    fn a_calm_day_leaves_the_windmill_still() {
        let mut w = World::new();
        windmill(&mut w, (1, MY, 1));
        dev(&mut w, (1, MY, 2), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        tick(&mut w, 1, &[], gust(0.05));
        assert!(!w.power_device_at((1, MY, 1)).unwrap().on, "dead air drives nothing");
        assert_eq!(w.get_block(1, MY, 1), block::WINDMILL, "a still mill keeps its idle face");
        assert_eq!(w.get_block(1, MY, 2), block::ELECTRIC_LAMP, "the lamp stays dark");
    }

    #[test]
    fn an_enclosed_windmill_never_turns_however_hard_it_blows() {
        // (a) A roof one cell up: no sky, no turn.
        let mut roofed = World::new();
        windmill(&mut roofed, (1, MY, 1));
        roofed.set_block(1, MY + 1, 1, block::STONE);
        tick(&mut roofed, 1, &[], gust(0.95));
        assert!(
            !roofed.power_device_at((1, MY, 1)).unwrap().on,
            "a mill under a roof turns in no gale"
        );
        assert_eq!(roofed.get_block(1, MY, 1), block::WINDMILL, "…and keeps its idle face");

        // (b) Sky is clear but three of the four sides are walled in — only one
        // side is open, and the sails need two.
        let mut walled = World::new();
        windmill(&mut walled, (1, MY, 1));
        walled.set_block(0, MY, 1, block::STONE);
        walled.set_block(2, MY, 1, block::STONE);
        walled.set_block(1, MY, 0, block::STONE);
        tick(&mut walled, 1, &[], gust(0.95));
        assert!(
            !walled.power_device_at((1, MY, 1)).unwrap().on,
            "walled in on three sides, the sails have nowhere to sweep"
        );

        // (c) A roof EIGHT cells up is still in the way — the sky check looks
        // the whole `WINDMILL_SKY_SCAN` column, not just the cell above.
        let mut high_roof = World::new();
        windmill(&mut high_roof, (1, MY, 1));
        high_roof.set_block(1, MY + WINDMILL_SKY_SCAN, 1, block::STONE);
        tick(&mut high_roof, 1, &[], gust(0.95));
        assert!(
            !high_roof.power_device_at((1, MY, 1)).unwrap().on,
            "a roof anywhere in the scan column blocks the wind"
        );
    }

    #[test]
    fn solids_and_fluids_stop_the_wind_but_a_cable_does_not() {
        // Carry-over from the Task 1 review: `blocks_wind` used to treat
        // anything but AIR as in the way, so the CABLE carrying the mill's own
        // output off its top face stopped it dead — you could not wire a
        // windmill up at all. The rule is the registry's `solid` flag.
        for (blk, label) in [
            (block::CABLE, "the mill's own output cable"),
            (block::TORCH, "a torch"),
        ] {
            let mut w = World::new();
            windmill(&mut w, (1, MY, 1));
            w.set_block(1, MY + 1, 1, blk);
            tick(&mut w, 1, &[], gust(0.70));
            assert!(
                w.power_device_at((1, MY, 1)).unwrap().on,
                "{label} on the top face must not becalm the mill"
            );
            assert_eq!(w.get_block(1, MY, 1), block::WINDMILL_TURNING);
        }

        // …and the solid ones still do — including GLASS and LEAVES, which are
        // see-through but solid: a mill under a canopy is becalmed. WATER and
        // LAVA are not `solid` in the registry (you swim through them) but must
        // block too, or a mill on the seabed under eight blocks of ocean would
        // read as standing in open air.
        for (blk, label) in [
            (block::STONE, "stone"),
            (block::GLASS, "glass"),
            (block::OAK_LEAVES, "a leaf canopy"),
            (block::WATER, "the sea above a drowned mill"),
            (block::LAVA, "a lava lake"),
        ] {
            let mut w = World::new();
            windmill(&mut w, (1, MY, 1));
            w.set_block(1, MY + 1, 1, blk);
            tick(&mut w, 1, &[], gust(0.95));
            assert!(
                !w.power_device_at((1, MY, 1)).unwrap().on,
                "{label} overhead must becalm the mill"
            );
        }
    }

    #[test]
    fn a_cable_on_a_turning_mill_keeps_it_turning_and_stays_lit() {
        // The whole point of the rule above: a mill wired up through its top
        // face must keep turning AND keep the run it feeds alight.
        let mut w = World::new();
        windmill(&mut w, (1, MY, 1));
        // Riser off the top face, then out sideways — the LAMP is solid, so it
        // has to leave the mill's sky column or it becalms the thing it lights.
        w.set_block(1, MY + 1, 1, block::CABLE);
        w.set_block(2, MY + 1, 1, block::CABLE);
        dev(&mut w, (3, MY + 1, 1), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        tick(&mut w, 1, &[], gust(0.70));
        assert_eq!(w.get_block(1, MY, 1), block::WINDMILL_TURNING, "mill turns under its own wire");
        assert_eq!(w.get_block(1, MY + 1, 1), block::CABLE_LIT, "the riser lights");
        assert_eq!(w.get_block(3, MY + 1, 1), block::ELECTRIC_LAMP_LIT, "and the lamp with it");
    }

    #[test]
    fn a_roof_beyond_the_sky_scan_does_not_block() {
        // The far boundary of the scan column: `WINDMILL_SKY_SCAN` cells are
        // checked, so the first cell ABOVE that range is out of reach. Pairs
        // with case (c) above, which pins the last cell that DOES block.
        let mut w = World::new();
        windmill(&mut w, (1, MY, 1));
        w.set_block(1, MY + WINDMILL_SKY_SCAN + 1, 1, block::STONE);
        tick(&mut w, 1, &[], gust(0.70));
        assert!(
            w.power_device_at((1, MY, 1)).unwrap().on,
            "a roof one cell beyond the scan column leaves the mill in open air"
        );
    }

    #[test]
    fn altitude_starts_a_mill_that_would_stall_at_sea_level() {
        // `power_tick` takes ONE sea-level sample and each mill re-applies its
        // own altitude term, so a speed that is short of WINDMILL_START down in
        // the valley clears it on a hilltop. 0.25 + 30 blocks × 0.010 = 0.55,
        // and 30 blocks up is exactly where `wind::ALTITUDE_CAP` lands — near
        // the roof of a 96-block world, so the cap is something you can build to.
        let breeze = gust(0.25);
        assert!(breeze.speed < WINDMILL_START, "the fixture speed must stall at sea level");

        let mut valley = World::new();
        windmill(&mut valley, (1, MY, 1));
        tick(&mut valley, 1, &[], breeze);
        assert!(
            !valley.power_device_at((1, MY, 1)).unwrap().on,
            "at sea level this breeze is not enough"
        );

        let mut hilltop = World::new();
        let high = (1, MY + 30, 1);
        windmill(&mut hilltop, high);
        tick(&mut hilltop, 1, &[], breeze);
        assert!(
            hilltop.power_device_at(high).unwrap().on,
            "the same breeze 30 blocks up clears the start threshold — build it high"
        );
        assert_eq!(hilltop.get_block(high.0, high.1, high.2), block::WINDMILL_TURNING);
    }

    #[test]
    fn windmill_hysteresis_holds_a_turning_mill_through_a_lull() {
        // The derived breeze drifts slowly across a threshold. Without the dead
        // band a mill parked on 0.35 would swap (and re-broadcast) its block id
        // several times a minute.
        let mut w = World::new();
        windmill(&mut w, (1, MY, 1));

        tick(&mut w, 1, &[], gust(0.50));
        assert!(w.power_device_at((1, MY, 1)).unwrap().on, "0.50 starts a still mill");

        tick(&mut w, 2, &[], gust(0.33));
        assert!(
            w.power_device_at((1, MY, 1)).unwrap().on,
            "0.33 is below the start threshold but above the stop one — keep turning"
        );

        tick(&mut w, 3, &[], gust(0.29));
        assert!(!w.power_device_at((1, MY, 1)).unwrap().on, "0.29 drops below the stop threshold");

        // …and the same 0.33 can't get a stopped mill going again.
        tick(&mut w, 4, &[], gust(0.33));
        assert!(
            !w.power_device_at((1, MY, 1)).unwrap().on,
            "0.33 is not enough to start a still mill"
        );
    }

    #[test]
    fn windmill_swaps_its_block_exactly_once_per_transition() {
        let mut w = World::new();
        let p = (1, MY, 1);
        windmill(&mut w, p);

        let started = tick(&mut w, 1, &[], gust(0.70));
        assert_eq!(
            started.iter().filter(|bc| (bc.x, bc.y, bc.z) == p).count(),
            1,
            "starting swaps the block once"
        );

        let steady = tick(&mut w, 2, &[], gust(0.70));
        assert!(
            !steady.iter().any(|bc| (bc.x, bc.y, bc.z) == p),
            "a mill that keeps turning must not re-broadcast its block every tick"
        );

        let stopped = tick(&mut w, 3, &[], gust(0.05));
        assert_eq!(
            stopped.iter().filter(|bc| (bc.x, bc.y, bc.z) == p).count(),
            1,
            "stopping swaps the block once"
        );
        assert_eq!(w.get_block(p.0, p.1, p.2), block::WINDMILL, "…back to the idle face");
    }

    /// The contract `game_loop`'s `ChallengeEvent::SourceTurned` fire site reads:
    /// an idle→turning transition puts exactly ONE `*_TURNING` block change on
    /// `power_tick`'s returned list, and a turning→idle transition puts the IDLE
    /// block there instead. If a stop ever reported the turning face, the Trial
    /// would count "the mill started" every time it stopped.
    #[test]
    fn only_a_start_reports_the_turning_face_on_the_returned_changes() {
        // (a) The windmill.
        let mut w = World::new();
        let p = (1, MY, 1);
        windmill(&mut w, p);

        let started = tick(&mut w, 1, &[], gust(0.70));
        assert_eq!(
            started.iter().filter(|bc| bc.new_block == block::WINDMILL_TURNING).count(),
            1,
            "a start reports the turning face exactly once"
        );

        let steady = tick(&mut w, 2, &[], gust(0.70));
        assert!(
            !steady.iter().any(|bc| bc.new_block == block::WINDMILL_TURNING),
            "a mill that keeps turning reports nothing — the event must not repeat"
        );

        let stopped = tick(&mut w, 3, &[], gust(0.05));
        assert!(
            !stopped.iter().any(|bc| bc.new_block == block::WINDMILL_TURNING),
            "a stop must never report the turning face"
        );
        assert_eq!(
            stopped.iter().filter(|bc| bc.new_block == block::WINDMILL).count(),
            1,
            "…it reports the idle face instead"
        );

        // (b) The water wheel, through the same `apply_turn`.
        let mut w = World::new();
        let wheel = (1, WY, 1);
        flowing_race(&mut w, &[(0, WY, 0), (1, WY, 0), (2, WY, 0)]);
        dev(&mut w, wheel, block::WATER_WHEEL, PowerDeviceKind::WaterWheel, Facing::Up);

        let started = tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(
            started.iter().filter(|bc| bc.new_block == block::WATER_WHEEL_TURNING).count(),
            1,
            "the wheel catching the current reports its turning face once"
        );
        let steady = tick(&mut w, 2, &[], WindSample::CALM);
        assert!(
            !steady.iter().any(|bc| bc.new_block == block::WATER_WHEEL_TURNING),
            "a wheel that keeps turning reports nothing"
        );
    }

    #[test]
    fn reseed_on_load_re_drives_a_saved_turning_windmill() {
        // A world saved mid-gale reloads with `on: true` and the turning block,
        // but `energised` is transient. `reseed_on_load` must enqueue the mill
        // so the first tick after the load relights everything downstream.
        let mut w = World::new();
        let p = (1, MY, 1);
        w.set_block(p.0, p.1, p.2, block::WINDMILL_TURNING);
        let mut d = PowerDeviceData::new(PowerDeviceKind::Windmill, Facing::Up);
        d.on = true;
        w.block_entities.insert(p, BlockEntityData::PowerDevice(d));
        w.set_block(1, MY, 2, block::CABLE);
        w.block_entities.insert(
            (1, MY, 3),
            BlockEntityData::PowerDevice(PowerDeviceData::new(
                PowerDeviceKind::ElectricLamp,
                Facing::Up,
            )),
        );
        w.set_block(1, MY, 3, block::ELECTRIC_LAMP);
        assert!(!w.power.is_on((1, MY, 2)), "nothing is energised before the reseed");

        reseed_on_load(&mut w);
        tick(&mut w, 1, &[], gust(0.70));
        assert!(w.power_device_at(p).unwrap().on, "the mill is still turning after the load");
        assert_eq!(w.get_block(p.0, p.1, p.2), block::WINDMILL_TURNING);
        assert_eq!(w.get_block(1, MY, 2), block::CABLE_LIT, "the reseeded mill relights its run");
        assert_eq!(w.get_block(1, MY, 3), block::ELECTRIC_LAMP_LIT);
    }

    #[test]
    fn beam_sensor_trips_when_an_entity_crosses_an_armed_beam() {
        // Spec 48 Phase 2 — sensor at origin facing East, a retroreflective
        // Mirror 4 cells East. The beam reaches the mirror, bounces back to the
        // origin sensor → ARMED. A lamp sits above the sensor.
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::BEAM_SENSOR, PowerDeviceKind::BeamSensor, Facing::East);
        dev(&mut w, (4, 0, 0), block::MIRROR, PowerDeviceKind::Mirror, Facing::West);
        dev(&mut w, (0, 1, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        // Armed but unbroken → sensor off → lamp dark.
        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(0, 1, 0), block::ELECTRIC_LAMP, "lamp dark while the beam is unbroken");

        // An entity stands in a beam cell (2,0,0) → breaks the beam → sensor
        // trips → drives the adjacent lamp.
        tick(&mut w, 2, &[(2.5, 0.0, 0.5)], WindSample::CALM);
        assert!(w.power_device_at((0, 0, 0)).unwrap().on, "crossing the beam trips the sensor");
        assert_eq!(w.get_block(0, 1, 0), block::ELECTRIC_LAMP_LIT, "tripped sensor lights the lamp");

        // Step off → beam restored → sensor releases → lamp dark again.
        tick(&mut w, 3, &[], WindSample::CALM);
        assert_eq!(w.get_block(0, 1, 0), block::ELECTRIC_LAMP, "lamp dark once the beam is clear");
    }

    #[test]
    fn beam_that_dead_ends_never_trips() {
        // No mirror / second sensor → the beam never arms, so an entity crossing
        // it does nothing.
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::BEAM_SENSOR, PowerDeviceKind::BeamSensor, Facing::East);
        dev(&mut w, (0, 1, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);
        tick(&mut w, 1, &[(2.5, 0.0, 0.5)], WindSample::CALM); // standing "in" the unarmed beam
        assert!(!w.power_device_at((0, 0, 0)).unwrap().on, "an unarmed beam can't trip");
        assert_eq!(w.get_block(0, 1, 0), block::ELECTRIC_LAMP, "lamp stays dark");
    }

    #[test]
    fn motion_sensor_trips_on_a_nearby_entity() {
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::MOTION_SENSOR, PowerDeviceKind::MotionSensor, Facing::Up);
        dev(&mut w, (1, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(1, 0, 0), block::ELECTRIC_LAMP, "dark with nobody near");

        tick(&mut w, 2, &[(1.0, 0.0, 1.0)], WindSample::CALM); // ~1.2 blocks away, inside MOTION_RADIUS
        assert_eq!(w.get_block(1, 0, 0), block::ELECTRIC_LAMP_LIT, "motion within range lights the lamp");

        tick(&mut w, 3, &[(40.0, 0.0, 40.0)], WindSample::CALM); // far away
        assert_eq!(w.get_block(1, 0, 0), block::ELECTRIC_LAMP, "dark once they leave");
    }

    #[test]
    fn lit_lamp_emits_block_light_via_relight_helper() {
        // Spec 48 — a lit Electric Lamp must actually ILLUMINATE, not just swap
        // texture. `power_tick` flips the block via set_block (no relight); the
        // caller runs `relight_after_power_change` per change to drive the BFS.
        let mut w = World::new();
        let reg = crate::block::BlockRegistry::new();
        dev(&mut w, (0, 0, 0), block::LEVER, PowerDeviceKind::Lever, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        dev(&mut w, (2, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);
        set_on(&mut w, (0, 0, 0), true);
        assert_eq!(w.block_light_at(2, 0, 0), 0, "lamp dark before the tick");

        let changes = tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(2, 0, 0), block::ELECTRIC_LAMP_LIT, "lamp lit");
        for bc in &changes {
            relight_after_power_change(&mut w, bc, &reg);
        }
        assert_eq!(w.block_light_at(2, 0, 0), 14, "lit lamp emits at its own cell");
        assert!(w.block_light_at(3, 0, 0) >= 13, "light spills to the neighbour");

        // Switch off → the helper removes the emitted light again.
        set_on(&mut w, (0, 0, 0), false);
        let changes = tick(&mut w, 2, &[], WindSample::CALM);
        for bc in &changes {
            relight_after_power_change(&mut w, bc, &reg);
        }
        assert_eq!(w.get_block(2, 0, 0), block::ELECTRIC_LAMP, "lamp dark");
        assert_eq!(w.block_light_at(2, 0, 0), 0, "unlit lamp emits nothing");
    }

    /// Drive a gate input cell high by sitting an active lever directly beneath
    /// it (a non-gate source drives all six neighbours).
    fn feed_input(w: &mut World, input: Pos, on: bool) {
        let below = (input.0, input.1 - 1, input.2);
        if w.power_device_at(below).is_none() {
            dev(w, below, block::LEVER, PowerDeviceKind::Lever, Facing::Up);
        }
        set_on(w, below, on);
    }

    #[test]
    fn and_gate_lights_only_with_both_inputs() {
        let mut w = World::new();
        // Gate at origin, output East → (1,0,0); inputs N/S → (0,0,-1)/(0,0,1).
        dev(&mut w, (0, 0, 0), block::LOGIC_GATE, PowerDeviceKind::LogicGate, Facing::East);
        if let Some(d) = w.power_device_at_mut((0, 0, 0)) {
            d.gate_op = GateOp::And;
        }
        dev(&mut w, (1, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        // Only one input high → gate stays off → lamp dark.
        feed_input(&mut w, (0, 0, -1), true);
        for t in 1..=4 {
            tick(&mut w, t, &[], WindSample::CALM);
        }
        assert_eq!(
            w.get_block(1, 0, 0),
            block::ELECTRIC_LAMP,
            "one input: lamp dark"
        );

        // Both inputs high → after the 1-tick gate settle, lamp lights.
        feed_input(&mut w, (0, 0, 1), true);
        for t in 5..=9 {
            tick(&mut w, t, &[], WindSample::CALM);
        }
        assert!(
            w.power_device_at((0, 0, 0)).unwrap().on,
            "gate latched on with both inputs"
        );
        assert_eq!(
            w.get_block(1, 0, 0),
            block::ELECTRIC_LAMP_LIT,
            "both inputs: lamp lit"
        );
    }

    #[test]
    fn pressure_plate_presses_under_an_entity() {
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::PRESSURE_PLATE, PowerDeviceKind::PressurePlate, Facing::Up);
        w.set_block(1, 0, 0, block::CABLE);
        dev(&mut w, (2, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);

        // Entity standing on the cell above the plate.
        let on_plate = [(0.5f32, 1.0f32, 0.5f32)];
        tick(&mut w, 1, &on_plate, WindSample::CALM);
        tick(&mut w, 2, &on_plate, WindSample::CALM);
        assert!(
            w.power_device_at((0, 0, 0)).unwrap().on,
            "plate pressed under entity"
        );
        assert_eq!(w.get_block(2, 0, 0), block::ELECTRIC_LAMP_LIT, "plate lights lamp");

        // Entity steps away → plate releases → lamp dark.
        tick(&mut w, 3, &[], WindSample::CALM);
        tick(&mut w, 4, &[], WindSample::CALM);
        assert!(!w.power_device_at((0, 0, 0)).unwrap().on, "plate released");
        assert_eq!(w.get_block(2, 0, 0), block::ELECTRIC_LAMP, "lamp dark after step-off");
    }

    #[test]
    fn hand_crank_runs_down_and_drops_power() {
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::HAND_CRANK, PowerDeviceKind::HandCrank, Facing::Up);
        dev(&mut w, (1, 0, 0), block::ELECTRIC_LAMP, PowerDeviceKind::ElectricLamp, Facing::Up);
        // Crank it: a few ticks of charge.
        if let Some(d) = w.power_device_at_mut((0, 0, 0)) {
            d.charge = 3;
        }
        w.mark_dirty((0, 0, 0));
        w.notify_neighbours((0, 0, 0));

        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.get_block(1, 0, 0), block::ELECTRIC_LAMP_LIT, "crank lights lamp");
        // Run the charge down (3 → 0).
        for t in 2..=6 {
            tick(&mut w, t, &[], WindSample::CALM);
        }
        assert_eq!(
            w.power_device_at((0, 0, 0)).unwrap().charge,
            0,
            "crank ran down"
        );
        assert_eq!(w.get_block(1, 0, 0), block::ELECTRIC_LAMP, "lamp dark when crank stops");
    }

    // ── Spec 49 (Explosives) — Blasting Keg sink + Plunger source ──────────────

    #[test]
    fn a_battery_never_recharges_itself_through_its_own_wire() {
        // Found by the Spec 48 end-to-end audit: a battery drives its own six
        // neighbours, so `consumer_powered` read the cable the battery was
        // energising as a supply and topped it back up to full every tick —
        // free power for ever from one flick of a lever.
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::LEVER, PowerDeviceKind::Lever, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        dev(&mut w, (2, 0, 0), block::BATTERY, PowerDeviceKind::Battery, Facing::Up);
        set_on(&mut w, (0, 0, 0), true);

        // Tick 1 floods the wire; the device sweep reads that flood on tick 2
        // (the same one-tick lag every device upkeep has — it runs before the
        // network recompute).
        tick(&mut w, 1, &[], WindSample::CALM);
        tick(&mut w, 2, &[], WindSample::CALM);
        assert_eq!(
            w.power_device_at((2, 0, 0)).unwrap().charge,
            BATTERY_CAPACITY,
            "the lever charges it through the wire"
        );

        // Lever off: it must run down, one tick per tick, and empty.
        set_on(&mut w, (0, 0, 0), false);
        for t in 3..=(BATTERY_CAPACITY as u64 + 4) {
            tick(&mut w, t, &[], WindSample::CALM);
        }
        assert_eq!(
            w.power_device_at((2, 0, 0)).unwrap().charge,
            0,
            "an unfed battery runs flat — it is a capacitor, not a generator"
        );
    }

    #[test]
    fn two_batteries_do_not_keep_each_other_alive() {
        // The other half of the same exploit: A drives B, B drives A, and the
        // pair burns for ever. No battery charges a battery.
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::BATTERY, PowerDeviceKind::Battery, Facing::Up);
        dev(&mut w, (1, 0, 0), block::BATTERY, PowerDeviceKind::Battery, Facing::Up);
        for p in [(0, 0, 0), (1, 0, 0)] {
            w.power_device_at_mut(p).unwrap().charge = BATTERY_CAPACITY;
            w.mark_dirty(p);
        }
        for t in 1..=(BATTERY_CAPACITY as u64 + 2) {
            tick(&mut w, t, &[], WindSample::CALM);
        }
        assert_eq!(w.power_device_at((0, 0, 0)).unwrap().charge, 0, "first battery flat");
        assert_eq!(w.power_device_at((1, 0, 0)).unwrap().charge, 0, "second battery flat");
    }

    #[test]
    fn a_battery_does_not_charge_the_battery_behind_it() {
        // The documented cost of the no-battery-charges-a-battery rule (see
        // `battery_is_fed`): batteries do NOT chain. A generator fills A; B,
        // hanging off A's own output, never fills. The fix if you want a longer
        // hold is to wire both onto the same run, not to queue them up.
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::STEAM_GENERATOR, PowerDeviceKind::SteamGenerator, Facing::Up);
        w.power_device_at_mut((0, 0, 0)).unwrap().fuel = Some(crate::furnace::FurnaceData {
            fuel: Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Coal, 1)),
            ..Default::default()
        });
        w.set_block(1, 0, 0, block::CABLE);
        dev(&mut w, (2, 0, 0), block::BATTERY, PowerDeviceKind::Battery, Facing::Up);
        dev(&mut w, (3, 0, 0), block::BATTERY, PowerDeviceKind::Battery, Facing::Up);

        for t in 1..=5 {
            tick(&mut w, t, &[], WindSample::CALM);
        }
        assert_eq!(
            w.power_device_at((2, 0, 0)).unwrap().charge,
            BATTERY_CAPACITY,
            "the generator fills the first battery through the wire"
        );
        assert_eq!(
            w.power_device_at((3, 0, 0)).unwrap().charge,
            0,
            "…and the battery behind it stays empty — no battery charges a battery"
        );
    }

    #[test]
    fn keg_fuse_lights_on_a_rising_edge() {
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::LEVER, PowerDeviceKind::Lever, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        dev(&mut w, (2, 0, 0), block::BLASTING_KEG, PowerDeviceKind::BlastingKeg, Facing::Up);
        set_on(&mut w, (0, 0, 0), true);
        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(
            w.power_device_at((2, 0, 0)).unwrap().charge,
            KEG_FUSE_TICKS,
            "a rising power edge lights the keg's fuse"
        );
    }

    #[test]
    fn a_held_wire_lights_the_keg_once_not_every_tick() {
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::LEVER, PowerDeviceKind::Lever, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        dev(&mut w, (2, 0, 0), block::BLASTING_KEG, PowerDeviceKind::BlastingKeg, Facing::Up);
        set_on(&mut w, (0, 0, 0), true);
        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.power_device_at((2, 0, 0)).unwrap().charge, KEG_FUSE_TICKS);
        // Simulate the fuse partly burning down, then re-run the network with the
        // lever STILL held. Edge-triggered, not level: the fuse must NOT re-arm.
        w.power_device_at_mut((2, 0, 0)).unwrap().charge = 40;
        w.mark_dirty((2, 0, 0));
        tick(&mut w, 2, &[], WindSample::CALM);
        assert_eq!(
            w.power_device_at((2, 0, 0)).unwrap().charge,
            40,
            "a held-on wire does not re-light an already-burning fuse"
        );
    }

    #[test]
    fn one_pulse_fans_out_to_multiple_kegs() {
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::LEVER, PowerDeviceKind::Lever, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        w.set_block(2, 0, 0, block::CABLE);
        w.set_block(3, 0, 0, block::CABLE);
        // Two kegs sitting on the cable run — synchronised demolition.
        dev(&mut w, (1, 1, 0), block::BLASTING_KEG, PowerDeviceKind::BlastingKeg, Facing::Up);
        dev(&mut w, (3, 1, 0), block::BLASTING_KEG, PowerDeviceKind::BlastingKeg, Facing::Up);
        set_on(&mut w, (0, 0, 0), true);
        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(w.power_device_at((1, 1, 0)).unwrap().charge, KEG_FUSE_TICKS, "first keg armed");
        assert_eq!(
            w.power_device_at((3, 1, 0)).unwrap().charge,
            KEG_FUSE_TICKS,
            "second keg armed by the same pulse"
        );
    }

    #[test]
    fn plunger_pulse_lights_a_wired_keg() {
        let mut w = World::new();
        dev(&mut w, (0, 0, 0), block::PLUNGER_DETONATOR, PowerDeviceKind::PlungerDetonator, Facing::East);
        w.set_block(1, 0, 0, block::CABLE);
        dev(&mut w, (2, 0, 0), block::BLASTING_KEG, PowerDeviceKind::BlastingKeg, Facing::Up);
        // Push the plunger — a Button-style momentary source.
        set_on(&mut w, (0, 0, 0), true);
        tick(&mut w, 1, &[], WindSample::CALM);
        assert_eq!(
            w.power_device_at((2, 0, 0)).unwrap().charge,
            KEG_FUSE_TICKS,
            "a plunger pulse lights the keg's fuse down the cable"
        );
    }

    #[test]
    fn tick_keg_fuses_counts_down_and_detonates_at_zero() {
        let mut w = World::new();
        dev(&mut w, (5, 5, 5), block::BLASTING_KEG, PowerDeviceKind::BlastingKeg, Facing::Up);
        w.power_device_at_mut((5, 5, 5)).unwrap().charge = 3;
        assert!(tick_keg_fuses(&mut w).is_empty(), "fuse 3→2, no detonation");
        assert!(tick_keg_fuses(&mut w).is_empty(), "fuse 2→1, no detonation");
        assert_eq!(tick_keg_fuses(&mut w), vec![(5, 5, 5)], "fuse 1→0 detonates exactly once");
        assert!(tick_keg_fuses(&mut w).is_empty(), "a spent keg doesn't re-detonate");
    }
}


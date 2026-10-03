//! Spec 48 (Electricity) — the end-to-end audit.
//!
//! Every test here drives [`TestHost`], i.e. the authoritative `GameServer`
//! tick, rather than poking `power_tick` directly: the unit tests in `power.rs`
//! pin the RULES, these pin that a player who mines, smelts, crafts, builds and
//! flips a switch gets a lit lamp out of it.
//!
//! Sources with their own weather/water inputs (Water Wheel, Windmill), the
//! save round-trip and the multiplayer broadcast live in `electricity_sources`.

use crate::block;
use crate::crafting::{CraftSlot, ToolMaterial, ToolType};
use crate::item::{ItemStack, MaterialId};
use crate::meta::Facing;
use crate::power::{GateOp, PowerDeviceKind};
use crate::test_harness::{TestConfig, TestHost};

/// Build height for every rig below — comfortably inside the world's 0..95.
const Y: i32 = 64;

fn host() -> TestHost {
    TestHost::start_with(TestConfig::default())
}

fn mat(m: MaterialId) -> CraftSlot {
    CraftSlot::Material(m)
}
fn blk(b: block::BlockId) -> CraftSlot {
    CraftSlot::Block(b)
}

/// A crafting grid from `(row, col, slot)` triples; every other cell empty.
fn grid(cells: &[(usize, usize, CraftSlot)]) -> [[CraftSlot; 3]; 3] {
    let mut g = [[CraftSlot::Empty; 3]; 3];
    for &(r, c, s) in cells {
        g[r][c] = s;
    }
    g
}

// ── Case 1 — the Survival chain, no `/give` ─────────────────────────────────

#[test]
fn survival_chain_from_copper_ore_to_a_lit_lamp() {
    // The whole Survival route to electricity, mined and crafted with nothing
    // handed to the player: logs → planks → sticks → wooden pickaxe → stone →
    // cobblestone → stone pickaxe → COPPER ORE → smelt → Copper Ingot, plus
    // rubber off a tapped tree and glass out of sand. Every step spends real
    // inventory, so a break anywhere in the chain fails here rather than in a
    // playtest. This is the test the whole wave exists for: before Copper Ore
    // generated, this route had no first step at all.
    let mut h = host();

    // ── Wood ────────────────────────────────────────────────────────────────
    for i in 0..4 {
        h.set_block(i, Y, 0, block::OAK_LOG);
        assert!(h.mine_into_inventory(i, Y, 0).is_some(), "a log comes off by hand");
    }
    assert_eq!(h.carrying(mat(MaterialId::GreenLog)), 4, "four green logs felled");

    for _ in 0..2 {
        assert!(h.craft(grid(&[(0, 0, mat(MaterialId::GreenLog))])).is_some(), "log → planks");
    }
    assert_eq!(h.carrying(blk(block::OAK_PLANKS)), 8);

    let planks = blk(block::OAK_PLANKS);
    for _ in 0..2 {
        assert!(
            h.craft(grid(&[(0, 0, planks), (1, 0, planks)])).is_some(),
            "two planks → four sticks"
        );
    }
    assert_eq!(h.carrying(mat(MaterialId::Stick)), 8);

    // ── Wooden pickaxe → stone → stone pickaxe ──────────────────────────────
    let stick = mat(MaterialId::Stick);
    let pickaxe = |m: CraftSlot| {
        grid(&[
            (0, 0, m), (0, 1, m), (0, 2, m),
            (1, 1, stick),
            (2, 1, stick),
        ])
    };
    assert!(h.craft(pickaxe(planks)).is_some(), "3 planks + 2 sticks → wooden pickaxe");
    assert!(h.equip_tool(ToolType::Pickaxe, ToolMaterial::Wood), "the wooden pickaxe is in hand");

    for i in 0..4 {
        h.set_block(i, Y, 2, block::STONE);
        assert!(h.mine_into_inventory(i, Y, 2).is_some(), "wood tier mines stone");
    }
    assert_eq!(h.carrying(blk(block::COBBLESTONE)), 4);

    // The tier gate is the reason the chain needs the stone pickaxe at all:
    // copper ore in a wooden pickaxe's hands breaks and drops nothing.
    h.set_block(9, Y, 2, block::COPPER_ORE);
    assert!(
        h.mine_into_inventory(9, Y, 2).is_none(),
        "copper ore is stone-tier — a wooden pickaxe wastes the vein"
    );

    let cobble = blk(block::COBBLESTONE);
    assert!(h.craft(pickaxe(cobble)).is_some(), "3 cobble + 2 sticks → stone pickaxe");
    assert!(
        h.equip_tool(ToolType::Pickaxe, ToolMaterial::Stone),
        "…and the stone one is now in hand"
    );

    // ── Copper ──────────────────────────────────────────────────────────────
    for i in 0..2 {
        h.set_block(i, Y, 4, block::COPPER_ORE);
        let drop = h.mine_into_inventory(i, Y, 4).expect("stone tier harvests copper ore");
        assert_eq!(
            drop.item,
            crate::item::Item::Material(MaterialId::Copper),
            "Copper Ore drops raw Copper, not the block"
        );
    }
    assert_eq!(h.carrying(mat(MaterialId::Copper)), 2);

    // ── Smelt ───────────────────────────────────────────────────────────────
    let furnace = (6, Y, 6);
    h.load_furnace(
        furnace.0,
        furnace.1,
        furnace.2,
        ItemStack::new_material(MaterialId::Copper, 2),
        ItemStack::new_material(MaterialId::GreenLog, 2),
    );
    h.tick_furnaces(2 * crate::furnace::SMELT_TICKS_PER_ITEM + 8);
    let ingots = h
        .take_furnace_output(furnace.0, furnace.1, furnace.2)
        .expect("the furnace produced something");
    assert_eq!(ingots.item, crate::item::Item::Material(MaterialId::CopperIngot));
    assert_eq!(ingots.count, 2, "both copper smelted");

    // ── Rubber + glass ──────────────────────────────────────────────────────
    for i in 0..2 {
        h.set_block(i, Y, 8, block::RUBBER_LOG);
        assert!(h.tap_rubber_log(i, Y, 8), "a live rubber log taps");
    }
    assert_eq!(h.carrying(mat(MaterialId::Rubber)), 2);

    for i in 0..4 {
        h.set_block(i, Y, 10, block::SAND);
        assert!(h.mine_into_inventory(i, Y, 10).is_some());
    }
    let sand = blk(block::SAND);
    assert!(
        h.craft(grid(&[(0, 0, sand), (0, 1, sand), (1, 0, sand), (1, 1, sand)])).is_some(),
        "4 sand → 4 glass"
    );

    // ── The Spec 48 parts ───────────────────────────────────────────────────
    let rubber = mat(MaterialId::Rubber);
    let copper_i = mat(MaterialId::CopperIngot);
    let glass = blk(block::GLASS);
    let cable_out = h
        .craft(grid(&[(0, 0, rubber), (0, 1, copper_i), (0, 2, rubber)]))
        .expect("Rubber / Copper Ingot / Rubber → Cable");
    assert_eq!(cable_out.item, crate::item::Item::Block(block::CABLE));
    assert_eq!(cable_out.count, 3, "one copper ingot insulates three cables");

    assert!(
        h.craft(grid(&[(0, 0, glass), (0, 1, copper_i), (0, 2, glass)])).is_some(),
        "Glass / Copper Ingot / Glass → Electric Lamp"
    );
    assert!(
        h.craft(grid(&[(0, 0, stick), (1, 0, cobble)])).is_some(),
        "Stick over Cobblestone → Lever"
    );

    // ── Build it and switch it on — only out of what was crafted ────────────
    assert!(h.place_from_inventory(0, Y, 12, block::LEVER, Facing::East));
    for x in 1..=3 {
        assert!(
            h.place_from_inventory(x, Y, 12, block::CABLE, Facing::Up),
            "three cables came out of one craft"
        );
    }
    assert!(h.place_from_inventory(4, Y, 12, block::ELECTRIC_LAMP, Facing::Up));
    assert_eq!(h.carrying(blk(block::CABLE)), 0, "every crafted cable is in the wall");

    h.toggle_lever(0, Y, 12);
    h.tick(1);
    assert_eq!(h.get_block(2, Y, 12), block::CABLE_LIT, "the run lights");
    assert_eq!(
        h.get_block(4, Y, 12),
        block::ELECTRIC_LAMP_LIT,
        "…and the lamp the player mined every part of comes on"
    );
}

// ── Case 2 — lever → cable run → lamp ───────────────────────────────────────

/// Lever at x=0, `len` cables, lamp on the far end. Returns the lamp's x.
fn lever_run(h: &mut TestHost, len: i32) -> i32 {
    h.place_power_block(0, Y, 0, block::LEVER, Facing::East);
    for x in 1..=len {
        h.place_power_block(x, Y, 0, block::CABLE, Facing::Up);
    }
    h.place_power_block(len + 1, Y, 0, block::ELECTRIC_LAMP, Facing::Up);
    len + 1
}

#[test]
fn a_lever_lights_a_twenty_block_run_in_one_tick_and_the_toggle_kills_it() {
    let mut h = host();
    let lamp = lever_run(&mut h, 20);

    h.tick(1);
    assert_eq!(h.get_block(lamp, Y, 0), block::ELECTRIC_LAMP, "dark before the flip");

    h.toggle_lever(0, Y, 0);
    h.tick(1);
    // The flood is one BFS over the whole connected component, so distance
    // costs nothing: 20 blocks light in the same tick the first one does.
    for x in 1..=20 {
        assert_eq!(h.get_block(x, Y, 0), block::CABLE_LIT, "cable {x} lit");
    }
    assert_eq!(h.get_block(lamp, Y, 0), block::ELECTRIC_LAMP_LIT, "lamp lit");
    assert!(h.device_on(lamp, Y, 0), "…and the lamp device agrees");

    h.toggle_lever(0, Y, 0);
    h.tick(1);
    assert_eq!(h.get_block(1, Y, 0), block::CABLE, "near cable dark");
    assert_eq!(h.get_block(20, Y, 0), block::CABLE, "far cable dark");
    assert_eq!(h.get_block(lamp, Y, 0), block::ELECTRIC_LAMP, "lamp out");
}

#[test]
fn cutting_the_cable_mid_run_kills_everything_downstream() {
    let mut h = host();
    let lamp = lever_run(&mut h, 20);
    h.toggle_lever(0, Y, 0);
    h.tick(1);
    assert_eq!(h.get_block(lamp, Y, 0), block::ELECTRIC_LAMP_LIT, "lit to start with");

    h.break_power_block(10, Y, 0);
    h.tick(2);
    assert_eq!(h.get_block(5, Y, 0), block::CABLE_LIT, "upstream of the cut stays live");
    assert_eq!(h.get_block(11, Y, 0), block::CABLE, "downstream of the cut goes dark");
    assert_eq!(h.get_block(20, Y, 0), block::CABLE, "…all the way to the end");
    assert_eq!(h.get_block(lamp, Y, 0), block::ELECTRIC_LAMP, "and the lamp with it");
}

#[test]
fn breaking_the_lever_leaves_no_ghost_source() {
    // Sources are entity-driven — nothing in the flood looks at the block id —
    // so a break that clears the block but forgets the `PowerDevice` leaves an
    // invisible lever powering the run forever. This is that regression, driven
    // through the harness's break path (the one `game_loop`'s break arms run).
    let mut h = host();
    let lamp = lever_run(&mut h, 3);
    h.toggle_lever(0, Y, 0);
    h.tick(1);
    assert_eq!(h.get_block(lamp, Y, 0), block::ELECTRIC_LAMP_LIT);

    h.break_power_block(0, Y, 0);
    h.tick(2);
    assert!(h.power_device(0, Y, 0).is_none(), "the device went with the block");
    assert_eq!(h.get_block(1, Y, 0), block::CABLE, "the run settles dark");
    assert_eq!(h.get_block(lamp, Y, 0), block::ELECTRIC_LAMP, "no ghost keeps the lamp lit");
}

// ── Case 3 — hand crank, battery, steam generator ───────────────────────────

#[test]
fn a_hand_crank_drives_for_its_run_then_stops() {
    let mut h = host();
    h.place_power_block(0, Y, 0, block::HAND_CRANK, Facing::Up);
    h.place_power_block(1, Y, 0, block::CABLE, Facing::Up);
    h.place_power_block(2, Y, 0, block::ELECTRIC_LAMP, Facing::Up);

    h.turn_crank(0, Y, 0);
    h.tick(1);
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP_LIT, "one turn lights the lamp");

    // It free-wheels for CRANK_RUN_TICKS and no longer: still going most of the
    // way through, stopped by the end. (The run-down and the network recompute
    // are one tick apart, hence the -2 / +2 either side of the constant.)
    h.tick(crate::power::CRANK_RUN_TICKS - 2);
    assert_eq!(
        h.get_block(2, Y, 0),
        block::ELECTRIC_LAMP_LIT,
        "still turning as the run winds down"
    );
    h.tick(4);
    assert_eq!(h.power_device(0, Y, 0).unwrap().charge, 0, "the crank has run out");
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP, "and the lamp is out");
}

#[test]
fn a_battery_holds_the_lamp_for_its_buffer_after_the_source_stops() {
    // Lever ─ cable ─ battery ─ lamp: the lamp's ONLY neighbour that can drive
    // it is the battery, so what it does after the lever goes off is the buffer.
    let mut h = host();
    h.place_power_block(0, Y, 0, block::LEVER, Facing::East);
    h.place_power_block(1, Y, 0, block::CABLE, Facing::Up);
    h.place_power_block(2, Y, 0, block::BATTERY, Facing::Up);
    h.place_power_block(3, Y, 0, block::ELECTRIC_LAMP, Facing::Up);

    h.toggle_lever(0, Y, 0);
    h.tick(2);
    assert_eq!(
        h.power_device(2, Y, 0).unwrap().charge,
        crate::power::BATTERY_CAPACITY,
        "a fed battery sits at full charge"
    );
    assert_eq!(h.get_block(3, Y, 0), block::ELECTRIC_LAMP_LIT, "lamp lit off the battery");

    h.toggle_lever(0, Y, 0);
    h.tick(1);
    assert!(!h.device_on(0, Y, 0), "the lever is off");
    assert_eq!(
        h.get_block(3, Y, 0),
        block::ELECTRIC_LAMP_LIT,
        "…but the battery carries the lamp on its own"
    );

    let mut held_for = 1;
    while h.get_block(3, Y, 0) == block::ELECTRIC_LAMP_LIT && held_for < 300 {
        h.tick(1);
        held_for += 1;
    }
    assert_eq!(h.get_block(3, Y, 0), block::ELECTRIC_LAMP, "the buffer does run out");
    let cap = crate::power::BATTERY_CAPACITY;
    assert!(
        (cap..=cap + 3).contains(&held_for),
        "the battery should hold for its {cap}-tick capacity, held {held_for}"
    );
}

#[test]
fn a_steam_generator_is_lit_only_while_it_is_fuelled() {
    let mut h = host();
    h.place_power_block(0, Y, 0, block::STEAM_GENERATOR, Facing::Up);
    h.place_power_block(1, Y, 0, block::ELECTRIC_LAMP, Facing::Up);

    h.tick(2);
    assert_eq!(h.get_block(0, Y, 0), block::STEAM_GENERATOR, "cold and dark unfuelled");
    assert_eq!(h.get_block(1, Y, 0), block::ELECTRIC_LAMP);

    // Cobblestone is not fuel; a stick is (40 ticks of burn).
    assert!(
        !h.fuel_generator(0, Y, 0, &ItemStack::new_block(block::COBBLESTONE, 1)),
        "a generator refuses a rock"
    );
    assert!(h.fuel_generator(0, Y, 0, &ItemStack::new_material(MaterialId::Stick, 1)));

    h.tick(1);
    assert_eq!(h.get_block(0, Y, 0), block::STEAM_GENERATOR_LIT, "fuelled → lit face");
    assert_eq!(h.get_block(1, Y, 0), block::ELECTRIC_LAMP_LIT, "…and it powers the lamp");

    // A stick burns 2 s. Run past that and the whole thing goes cold on its own.
    h.tick(60);
    assert_eq!(h.get_block(0, Y, 0), block::STEAM_GENERATOR, "out of fuel, out of light");
    assert_eq!(h.get_block(1, Y, 0), block::ELECTRIC_LAMP, "lamp dark with the generator");
}

// ── Case 4 — logic gates, button pulse, pressure plate ──────────────────────

/// A gate rig: gate at the origin facing East, its two input cells reachable
/// through cable from a lever, output cable → lamp. `op` is set on the gate.
/// Returns the lamp cell.
fn gate_rig(h: &mut TestHost, op: GateOp) -> (i32, i32, i32) {
    h.place_power_block(0, Y, 0, block::LOGIC_GATE, Facing::East);
    h.set_gate_op(0, Y, 0, op);
    // Output side.
    h.place_power_block(1, Y, 0, block::CABLE, Facing::Up);
    h.place_power_block(2, Y, 0, block::ELECTRIC_LAMP, Facing::Up);
    // Input A (north) and B (south): a lever two cells out, cabled in.
    for (dz, lever_z) in [(-1, -3), (1, 3)] {
        h.place_power_block(0, Y, dz, block::CABLE, Facing::Up);
        h.place_power_block(0, Y, dz * 2, block::CABLE, Facing::Up);
        h.place_power_block(0, Y, lever_z, block::LEVER, Facing::Up);
    }
    (2, Y, 0)
}

/// Flip input A (north lever) / B (south lever).
fn set_inputs(h: &mut TestHost, a: bool, b: bool) {
    for (lever_z, want) in [(-3, a), (3, b)] {
        if h.device_on(0, Y, lever_z) != want {
            h.toggle_lever(0, Y, lever_z);
        }
    }
}

#[test]
fn two_input_gates_answer_their_truth_tables_through_cable() {
    for (op, table) in [
        (GateOp::And, [(false, false, false), (true, false, false), (false, true, false), (true, true, true)]),
        (GateOp::Or, [(false, false, false), (true, false, true), (false, true, true), (true, true, true)]),
        (GateOp::Xor, [(false, false, false), (true, false, true), (false, true, true), (true, true, false)]),
    ] {
        for (a, b, want) in table {
            let mut h = host();
            let lamp = gate_rig(&mut h, op);
            set_inputs(&mut h, a, b);
            // Two ticks: one to flood the input cables, one for the gate to
            // settle. A third proves it stays put rather than oscillating.
            h.tick(3);
            let lit = h.get_block(lamp.0, lamp.1, lamp.2) == block::ELECTRIC_LAMP_LIT;
            assert_eq!(lit, want, "{op:?}({a}, {b}) should be {want}");
        }
    }
}

#[test]
fn a_not_gate_inverts_its_single_input() {
    // NOT reads the cell BEHIND its output face, so this rig wires the west
    // side rather than the north/south pair.
    let mut h = host();
    h.place_power_block(0, Y, 0, block::LOGIC_GATE, Facing::East);
    h.set_gate_op(0, Y, 0, GateOp::Not);
    h.place_power_block(1, Y, 0, block::CABLE, Facing::Up);
    h.place_power_block(2, Y, 0, block::ELECTRIC_LAMP, Facing::Up);
    h.place_power_block(-1, Y, 0, block::CABLE, Facing::Up);
    h.place_power_block(-2, Y, 0, block::LEVER, Facing::Up);

    h.tick(3);
    assert_eq!(
        h.get_block(2, Y, 0),
        block::ELECTRIC_LAMP_LIT,
        "an un-driven NOT gate drives its output"
    );

    h.toggle_lever(-2, Y, 0);
    h.tick(3);
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP, "power in, dark out");
}

#[test]
fn a_gate_costs_exactly_one_extra_tick() {
    // The deliberate one-tick latch is what breaks combinational feedback and
    // bounds oscillators — it is a designed cost, so it is pinned here.
    let mut h = host();
    let lamp = gate_rig(&mut h, GateOp::Or);
    set_inputs(&mut h, true, false);

    h.tick(1);
    assert_eq!(
        h.get_block(lamp.0, lamp.1, lamp.2),
        block::ELECTRIC_LAMP,
        "tick 1 floods the input cable; the gate has not latched yet"
    );
    h.tick(1);
    assert!(h.device_on(0, Y, 0), "tick 2 settles the gate");
    assert_eq!(
        h.get_block(lamp.0, lamp.1, lamp.2),
        block::ELECTRIC_LAMP_LIT,
        "…and the output lights in the same tick it settles"
    );
}

#[test]
fn a_button_pulse_releases_after_ten_ticks() {
    let mut h = host();
    h.place_power_block(0, Y, 0, block::BUTTON, Facing::East);
    h.place_power_block(1, Y, 0, block::CABLE, Facing::Up);
    h.place_power_block(2, Y, 0, block::ELECTRIC_LAMP, Facing::Up);

    h.press_button(0, Y, 0);
    h.tick(1);
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP_LIT, "the pulse lights the lamp");

    h.tick(8);
    assert!(h.device_on(0, Y, 0), "still held nine ticks in");
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP_LIT);

    h.tick(1);
    assert!(!h.device_on(0, Y, 0), "the scheduled release fires on the tenth tick");
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP, "and the lamp goes out with it");
}

#[test]
fn a_pressure_plate_is_on_only_while_someone_stands_on_it() {
    let mut h = host();
    h.place_power_block(0, Y, 0, block::PRESSURE_PLATE, Facing::Up);
    h.place_power_block(1, Y, 0, block::CABLE, Facing::Up);
    h.place_power_block(2, Y, 0, block::ELECTRIC_LAMP, Facing::Up);
    // Park the player well clear to start with.
    h.teleport_player(0, glam::Vec3::new(40.5, Y as f32, 40.5));

    h.tick(2);
    assert!(!h.device_on(0, Y, 0), "nobody on it, nothing doing");
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP);

    h.teleport_player(0, glam::Vec3::new(0.5, (Y + 1) as f32, 0.5));
    h.tick(2);
    assert!(h.device_on(0, Y, 0), "a player standing on the plate presses it");
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP_LIT, "…and lights the lamp");

    h.teleport_player(0, glam::Vec3::new(40.5, Y as f32, 40.5));
    h.tick(2);
    assert!(!h.device_on(0, Y, 0), "step off and it releases");
    assert_eq!(h.get_block(2, Y, 0), block::ELECTRIC_LAMP);
}

// ── Case 5 — pistons, beam sensor + mirror, motion sensor ───────────────────

/// A piston (or sticky piston) at the origin facing East, powered by a lever on
/// its west face, with a stone block in front of it to shove.
fn piston_rig(h: &mut TestHost, piston: block::BlockId) {
    h.set_block(0, Y, 0, piston);
    h.set_meta_facing(0, Y, 0, Facing::East);
    h.set_block(1, Y, 0, block::STONE);
    h.place_power_block(-1, Y, 0, block::LEVER, Facing::East);
}

#[test]
fn a_powered_piston_shoves_the_block_in_front_of_it() {
    let mut h = host();
    piston_rig(&mut h, block::PISTON);

    h.toggle_lever(-1, Y, 0);
    h.tick(1);
    assert!(h.tick_pistons().len() > 1, "the powered piston fires");
    assert_eq!(h.get_block(1, Y, 0), block::PISTON_HEAD, "the arm comes out");
    assert_eq!(h.get_block(2, Y, 0), block::STONE, "the stone is shoved one cell along");

    // A plain piston leaves the block where it pushed it.
    h.toggle_lever(-1, Y, 0);
    h.tick(1);
    h.tick_pistons();
    assert_eq!(h.get_block(1, Y, 0), block::AIR, "the arm retracts");
    assert_eq!(h.get_block(2, Y, 0), block::STONE, "a plain piston does not pull back");
}

#[test]
fn a_sticky_piston_pulls_its_block_back_on_retract() {
    let mut h = host();
    piston_rig(&mut h, block::STICKY_PISTON);

    h.toggle_lever(-1, Y, 0);
    h.tick(1);
    h.tick_pistons();
    assert_eq!(h.get_block(2, Y, 0), block::STONE, "pushed like any piston");

    h.toggle_lever(-1, Y, 0);
    h.tick(1);
    h.tick_pistons();
    assert_eq!(h.get_block(2, Y, 0), block::AIR, "the stone leaves the far cell");
    assert_eq!(h.get_block(1, Y, 0), block::STONE, "…because the sticky head pulled it back");
}

#[test]
fn a_crossed_beam_between_sensor_and_mirror_trips_the_alarm() {
    let mut h = host();
    // Sensor facing East with a retroreflecting Mirror four cells along: the
    // beam goes out, bounces, comes home — ARMED.
    h.place_power_block(0, Y, 0, block::BEAM_SENSOR, Facing::East);
    h.place_power_block(4, Y, 0, block::MIRROR, Facing::West);
    h.place_power_block(0, Y + 1, 0, block::ELECTRIC_LAMP, Facing::Up);
    h.teleport_player(0, glam::Vec3::new(40.5, Y as f32, 40.5));

    h.tick(2);
    assert!(!h.device_on(0, Y, 0), "an unbroken beam is quiet");
    assert_eq!(h.get_block(0, Y + 1, 0), block::ELECTRIC_LAMP);

    h.teleport_player(0, glam::Vec3::new(2.5, Y as f32, 0.5));
    h.tick(2);
    assert!(h.device_on(0, Y, 0), "walking through the beam trips the sensor");
    assert_eq!(h.get_block(0, Y + 1, 0), block::ELECTRIC_LAMP_LIT, "the alarm lamp lights");

    // Take the mirror away and the beam never arms, so the same trespass does
    // nothing — the mirror is load-bearing, not decoration.
    h.teleport_player(0, glam::Vec3::new(40.5, Y as f32, 40.5));
    h.tick(2);
    h.break_power_block(4, Y, 0);
    h.teleport_player(0, glam::Vec3::new(2.5, Y as f32, 0.5));
    h.tick(2);
    assert!(!h.device_on(0, Y, 0), "without the mirror there is no armed beam to break");
    assert_eq!(h.get_block(0, Y + 1, 0), block::ELECTRIC_LAMP);
}

#[test]
fn a_motion_sensor_trips_inside_its_radius_and_not_outside() {
    let mut h = host();
    h.place_power_block(0, Y, 0, block::MOTION_SENSOR, Facing::Up);
    h.place_power_block(1, Y, 0, block::ELECTRIC_LAMP, Facing::Up);
    let r = crate::power::MOTION_RADIUS;

    // Just outside the radius, measured from the sensor's cell centre.
    h.teleport_player(0, glam::Vec3::new(0.5 + r + 1.0, Y as f32 + 0.5, 0.5));
    h.tick(2);
    assert!(!h.device_on(0, Y, 0), "out of range, out of mind");
    assert_eq!(h.get_block(1, Y, 0), block::ELECTRIC_LAMP);

    h.teleport_player(0, glam::Vec3::new(0.5 + r - 1.0, Y as f32 + 0.5, 0.5));
    h.tick(2);
    assert!(h.device_on(0, Y, 0), "inside MOTION_RADIUS it trips");
    assert_eq!(h.get_block(1, Y, 0), block::ELECTRIC_LAMP_LIT);

    h.teleport_player(0, glam::Vec3::new(40.5, Y as f32, 40.5));
    h.tick(2);
    assert!(!h.device_on(0, Y, 0), "and releases when they leave");
    assert_eq!(h.get_block(1, Y, 0), block::ELECTRIC_LAMP);
}

// ── The device table the client, the host and this harness all share ────────

#[test]
fn every_power_block_maps_to_a_device_kind() {
    // `device_kind_for_block` is what turns a placed block into something the
    // power sim can see. A power block missing from it is placeable but inert —
    // exactly the failure the old `_ => ElectricLamp` catch-all hid by making
    // it a lamp instead.
    for id in 0..crate::block::BlockRegistry::new().len() as block::BlockId {
        if !block::is_power_block(id) {
            assert!(
                crate::power::device_kind_for_block(id).is_none(),
                "block {id} is not a power block but claims a device kind"
            );
            continue;
        }
        if block::is_cable(id) {
            assert!(
                crate::power::device_kind_for_block(id).is_none(),
                "a cable is a pure conductor — it carries no device"
            );
            continue;
        }
        assert!(
            crate::power::device_kind_for_block(id).is_some(),
            "power block {id} has no device kind — it would be placeable but inert"
        );
    }
    // The two lit/unlit twins that carry state must map to the SAME kind, or a
    // twin swap would silently rebuild the device and lose its fuel/charge.
    assert_eq!(
        crate::power::device_kind_for_block(block::STEAM_GENERATOR),
        Some(PowerDeviceKind::SteamGenerator),
    );
    assert_eq!(
        crate::power::device_kind_for_block(block::STEAM_GENERATOR_LIT),
        Some(PowerDeviceKind::SteamGenerator),
    );
}

// ── The wire's metadata byte ────────────────────────────────────────────────

/// Every `pending_block_changes.push(...)` in `src` whose argument is not the
/// shared `broadcast_change` helper (or a `BlockChange` another module already
/// built, which owns its own metadata). Returns the offending snippets.
/// Everything between the `(` that `rest` starts just after and the `)` that
/// closes it — so an `.extend(...)` argument is judged whole, however long it
/// runs, rather than by an arbitrary prefix.
fn call_argument(rest: &str) -> &str {
    let mut depth = 1usize;
    for (i, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..i];
                }
            }
            _ => {}
        }
    }
    rest
}

fn pushes_that_guess_their_metadata(src: &str) -> Vec<String> {
    const QUEUE: &str = "pending_block_changes";
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find(QUEUE) {
        rest = &rest[i + QUEUE.len()..];
        // rustfmt splits a long call by putting the RECEIVER on its own line,
        // so the `.push(` can be a newline and an indent away from the queue's
        // name. Skip whatever whitespace is between them before deciding —
        // matching the contiguous form alone left four live sites unlinted.
        let after = rest.trim_start();
        let (verb, arg) = match (after.strip_prefix(".push("), after.strip_prefix(".extend(")) {
            (Some(a), _) => (".push", a.trim_start()),
            (_, Some(a)) => (".extend", a.trim_start()),
            // `.clear()`, `.drain(..)`, a `let` binding — not a broadcast.
            _ => continue,
        };
        let whole = call_argument(arg);
        let snippet: String = arg.chars().take(60).collect();
        // A `.push(` hands over ONE change, so it must be built by a helper
        // that reads the metadata byte off the world (`broadcast_change`) or
        // relayed verbatim from the wire (`bc.clone()`). An `.extend(` hands
        // over a collection built somewhere else — power_tick's flips, a
        // shared handler's outcome — so it is judged on whether a
        // `BlockChange` is being CONSTRUCTED here, where the world is in reach
        // and guessing is inexcusable.
        let ok = if verb == ".extend" {
            !whole.contains("BlockChange")
        } else {
            snippet.starts_with("broadcast_change(")
                || snippet.starts_with("crate::game_loop::broadcast_change(")
                || snippet.starts_with("bc.clone()")
        };
        if !ok {
            out.push(snippet);
        }
    }
    out
}

#[test]
fn the_metadata_lint_can_tell_a_good_push_from_a_bad_one() {
    // Guard the guard: a lint that cannot fail is not a lint.
    let bad = "self.pending_block_changes.push(crate::protocol::BlockChange {\n                x, y, z, new_block: blk, meta: 0,\n });";
    assert_eq!(pushes_that_guess_their_metadata(bad).len(), 1, "the literal form is caught");
    // The split rustfmt ACTUALLY produces: the receiver goes on its own line,
    // not the argument. The old lint matched `pending_block_changes.push(` as
    // one contiguous string, so this shape — four live sites of it — sailed
    // straight through, and the self-test exercised a split rustfmt never
    // writes.
    let also_bad = "self.pending_block_changes\n                    .push(crate::protocol::BlockChange::new(x, y, z, blk));";
    assert_eq!(
        pushes_that_guess_their_metadata(also_bad).len(),
        1,
        "…including when rustfmt puts the receiver on its own line"
    );
    let arg_split = "self.pending_block_changes.push(\n                     crate::protocol::BlockChange::new(x, y, z, blk),\n);";
    assert_eq!(
        pushes_that_guess_their_metadata(arg_split).len(),
        1,
        "…and when the line break hides the argument instead"
    );
    let bad_extend = "self.pending_block_changes.extend(\n    placed.iter().map(|&(x, y, z, b)| crate::protocol::BlockChange::new(x, y, z, b)),\n);";
    assert_eq!(
        pushes_that_guess_their_metadata(bad_extend).len(),
        1,
        "an extend that builds its own changes is caught too"
    );
    let good = "self.pending_block_changes.push(\n                 broadcast_change(&self.world, x, y, z, blk),\n);";
    assert!(pushes_that_guess_their_metadata(good).is_empty());
    let good_split = "self.pending_block_changes\n                    .push(broadcast_change(&self.world, x, y, z, blk));";
    assert!(pushes_that_guess_their_metadata(good_split).is_empty());
    let good_extend = "self.pending_block_changes.extend(power_changes);";
    assert!(
        pushes_that_guess_their_metadata(good_extend).is_empty(),
        "extending with a collection built elsewhere is fine — that source has its own guard"
    );
    let not_a_broadcast = "self.pending_block_changes.clear();\nlet mut pending_block_changes = Vec::new();";
    assert!(pushes_that_guess_their_metadata(not_a_broadcast).is_empty());
}

#[test]
fn no_client_block_change_push_guesses_its_metadata() {
    // Source lint, in the `packaging_copy_lint` tradition: the ONLY way a
    // facing / rail shape / slab half / lever latch reaches the host and the
    // other players is `BlockChange.meta`, and for the whole life of the engine
    // every push in the client tick wrote a literal `meta: 0`. That is what
    // landed a joiner's Logic Gate facing north on the host whichever way they
    // pointed it. `game_loop::broadcast_change` reads the byte back off the
    // world instead; this fails the build if a new push goes back to guessing.
    for file in ["game_loop.rs", "block_interact.rs"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(file);
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "electricity lint: cannot read {} ({e}). If the client tick moved, \
                 update this lint's path — do not delete the lint.",
                path.display()
            )
        });
        let offenders = pushes_that_guess_their_metadata(&raw);
        assert!(
            offenders.is_empty(),
            "{file} queues block changes without `game_loop::broadcast_change` (or, for \
             an `.extend`, builds them inline), so the metadata byte (facing, rail shape, \
             slab half, lever latch) is dropped on the wire: {offenders:?}"
        );
    }
}

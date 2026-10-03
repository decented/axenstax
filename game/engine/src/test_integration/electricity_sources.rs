//! Spec 48 (Electricity) — the sources that run themselves, plus persistence
//! and the multiplayer broadcast.
//!
//! The Water Wheel and the Windmill are the two sources with an input outside
//! the power grid — a current and the weather — so they are the two that can
//! only really be trusted end to end: real water poured down a real spillway,
//! and the derived breeze the server actually samples.
//!
//! Native-only, mirroring `test_integration/power.rs`: `save_world` has no sync
//! API on WASM.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec3;

use crate::block;
use crate::meta::Facing;
use crate::power::{PowerDeviceData, PowerDeviceKind};
use crate::test_harness::{TestConfig, TestHost};
use crate::wind::WindSample;
use crate::world::World;

const Y: i32 = 64;

fn host() -> TestHost {
    TestHost::start_with(TestConfig::default())
}

/// Tick until `pred` holds, up to `limit` ticks; returns how many it took, or
/// `None` if it never did.
fn tick_until(h: &mut TestHost, limit: u32, pred: impl Fn(&TestHost) -> bool) -> Option<u32> {
    for n in 1..=limit {
        h.tick(1);
        if pred(h) {
            return Some(n);
        }
    }
    None
}

// ── Case 6 — the Water Wheel, driven by the real water sim ──────────────────

/// A three-step spillway cut into a stone hillside at z=0: the channel floor
/// drops a block at x=1, x=2 and x=3, then runs level to a stone end wall. Cut
/// out of solid rock so the water can only go one way — nothing here authors a
/// depth level, the water sim works them out on its way down.
///
/// Kept short on purpose: `water`'s retraction BFS only reaches
/// `MAX_SPREAD_DIST` (7) cells from a pulled source, so a longer cascade would
/// never drain (see the report — that is a water-sim limit, not a wheel bug).
fn spillway(h: &mut TestHost) {
    for x in -1..=6 {
        for z in -1..=1 {
            for y in (Y - 1)..=(Y + 3) {
                h.set_block(x, y, z, block::STONE);
            }
        }
    }
    // The carved channel: (x, lowest air y).
    for (x, floor_top) in [(0, Y + 3), (1, Y + 2), (2, Y + 1), (3, Y), (4, Y), (5, Y)] {
        for y in floor_top..=(Y + 3) {
            h.set_block(x, y, 0, block::AIR);
        }
    }
}

#[test]
fn a_wheel_beside_a_real_stream_turns_and_stops_when_it_dries_up() {
    let mut h = host();
    spillway(&mut h);
    // The wheel sits in the bank beside the bottom of the run, wired to a lamp.
    h.place_power_block(4, Y, 1, block::WATER_WHEEL, Facing::Up);
    h.place_power_block(4, Y, 2, block::CABLE, Facing::Up);
    h.place_power_block(4, Y, 3, block::ELECTRIC_LAMP, Facing::Up);

    h.tick(4);
    assert!(!h.device_on(4, Y, 1), "a dry wheel is idle");
    assert_eq!(h.get_block(4, Y, 1), block::WATER_WHEEL);

    // Pour it in at the top and let the water find its own way down.
    h.place_water_source(0, Y + 3, 0);
    tick_until(&mut h, 400, |h| {
        crate::water::flow_vector(h.world(), 4, Y, 0).is_some()
    })
    .expect("the stream reaches the wheel's cell within 400 ticks");

    let turning = tick_until(&mut h, 20, |h| h.device_on(4, Y, 1))
        .expect("a current beside the wheel turns it");
    assert!(turning <= 4, "the wheel picks the current up on the next device sweep");
    assert_eq!(h.get_block(4, Y, 1), block::WATER_WHEEL_TURNING, "it swaps to its turning face");
    assert_eq!(h.get_block(4, Y, 2), block::CABLE_LIT, "and drives the run");
    assert_eq!(h.get_block(4, Y, 3), block::ELECTRIC_LAMP_LIT);

    // Pull the source and the channel drains — the wheel must idle again.
    h.remove_water_source(0, Y + 3, 0);
    tick_until(&mut h, 800, |h| !h.device_on(4, Y, 1))
        .expect("the wheel stops once the stream dries up");
    assert_eq!(h.get_block(4, Y, 1), block::WATER_WHEEL, "back to its idle face");
    h.tick(2);
    assert_eq!(h.get_block(4, Y, 2), block::CABLE, "the cable settles dark");
    assert_eq!(h.get_block(4, Y, 3), block::ELECTRIC_LAMP, "and so does the lamp");
}

#[test]
fn a_still_pond_never_turns_a_wheel() {
    // The teaching point of the whole block: a wheel needs a STREAM. A walled
    // pool of sources has no gradient and nowhere to fall, so it turns nothing
    // however much water is in it.
    let mut h = host();
    for x in 0..=1 {
        for z in 0..=1 {
            h.set_block(x, Y - 1, z, block::STONE);
            h.place_water_source(x, Y, z);
        }
    }
    // Wall the pool in so no cell can spread or spill.
    for (x, z) in [(-1, 0), (-1, 1), (2, 0), (2, 1), (0, -1), (1, -1), (0, 2), (1, 2)] {
        h.set_block(x, Y, z, block::STONE);
    }
    h.place_power_block(0, Y + 1, 0, block::WATER_WHEEL, Facing::Up);

    h.tick(40);
    assert!(
        crate::water::flow_vector(h.world(), 0, Y, 0).is_none(),
        "a walled pool of sources has no current"
    );
    assert!(!h.device_on(0, Y + 1, 0), "…so the wheel beside it never turns");
    assert_eq!(h.get_block(0, Y + 1, 0), block::WATER_WHEEL);
}

// ── Case 7 — the Windmill, driven by the derived breeze ─────────────────────

/// The highest a mill can stand in a 96-block world, and the altitude the wind
/// rule is really about — and, since the Task 2b retune, exactly where
/// `wind::ALTITUDE_CAP` (+0.30) is reached. The world ceiling is `SEA_LEVEL +
/// 33`, so the full altitude bonus is something a player can build to.
const HILLTOP: i32 = crate::biome::SEA_LEVEL + 30;

#[test]
fn a_storm_turns_an_exposed_mill_and_a_roof_stops_it() {
    let mut h = host();
    h.place_power_block(0, Y, 0, block::WINDMILL, Facing::Up);
    h.place_power_block(0, Y, 1, block::CABLE, Facing::Up);
    h.place_power_block(0, Y, 2, block::ELECTRIC_LAMP, Facing::Up);

    h.force_storm(2000);
    assert!(
        h.wind_next_tick(Y).speed >= crate::power::WINDMILL_START,
        "fixture: a thunderstorm always clears the start threshold"
    );
    h.tick(2);
    assert!(h.device_on(0, Y, 0), "an exposed mill turns in a storm");
    assert_eq!(h.get_block(0, Y, 0), block::WINDMILL_TURNING);
    assert_eq!(h.get_block(0, Y, 2), block::ELECTRIC_LAMP_LIT, "and lights the lamp");

    // Roof it over — same gale, no wind reaches the sails.
    h.set_block(0, Y + 3, 0, block::STONE);
    h.tick(2);
    assert!(!h.device_on(0, Y, 0), "a mill under a roof is becalmed");
    assert_eq!(h.get_block(0, Y, 0), block::WINDMILL, "idle face");
    assert_eq!(h.get_block(0, Y, 2), block::ELECTRIC_LAMP, "lamp out");

    // Take the roof off and it picks straight back up.
    h.set_block(0, Y + 3, 0, block::AIR);
    h.tick(2);
    assert!(h.device_on(0, Y, 0), "open the sky and it turns again");
}

#[test]
fn altitude_is_what_makes_a_clear_day_mill_worth_building() {
    // Two mills, same world, same clear sky, 30 blocks apart vertically. The
    // hilltop one turns; the one in the valley never does. Both preconditions
    // are asserted off the same deterministic `wind::sample` the sim uses, so a
    // future re-tune of the breeze fails loudly as a fixture problem rather than
    // silently making this test vacuous.
    let mut h = host();
    h.place_power_block(0, Y, 0, block::WINDMILL, Facing::Up);
    h.place_power_block(0, HILLTOP, 0, block::WINDMILL, Facing::Up);

    let start = crate::power::WINDMILL_START;
    let valley_wind = h.wind_next_tick(Y).speed;
    let hill_wind = h.wind_next_tick(HILLTOP).speed;
    assert!(valley_wind < start, "fixture: seed 42's clear-sky breeze stalls at sea level");
    assert!(hill_wind >= start, "fixture: …and clears the bar 30 blocks up");

    let turned = tick_until(&mut h, 200, |h| h.device_on(0, HILLTOP, 0));
    assert!(turned.is_some(), "the hilltop mill turns on a clear day");
    assert_eq!(h.get_block(0, HILLTOP, 0), block::WINDMILL_TURNING);
    assert!(!h.device_on(0, Y, 0), "the valley mill never gets going");
    assert_eq!(h.get_block(0, Y, 0), block::WINDMILL);
}

// ── Case 8 — save → load → reseed ───────────────────────────────────────────

fn scrub(name: &str) {
    let _ = crate::save::delete_world(name);
}

#[test]
fn a_lit_network_and_a_turning_mill_come_back_after_a_save_and_load() {
    // `PowerState.energised` is transient by design — it is NOT saved. What
    // must survive is the device state, and `reseed_on_load` re-drives the
    // network from it on the first tick after the load. Without that a world
    // reloads with every lamp dark until someone touches a switch.
    let name = "__test_electricity_roundtrip__";
    scrub(name);

    let mut h = host();
    h.place_power_block(0, Y, 0, block::LEVER, Facing::East);
    for x in 1..=3 {
        h.place_power_block(x, Y, 0, block::CABLE, Facing::Up);
    }
    h.place_power_block(4, Y, 0, block::ELECTRIC_LAMP, Facing::Up);
    h.toggle_lever(0, Y, 0);
    // A turning windmill on the same world, with its own lamp.
    h.place_power_block(0, Y, 8, block::WINDMILL, Facing::South);
    h.place_power_block(0, Y, 9, block::CABLE, Facing::Up);
    h.place_power_block(0, Y, 10, block::ELECTRIC_LAMP, Facing::Up);
    h.force_storm(2000);
    h.tick(2);
    assert_eq!(h.get_block(4, Y, 0), block::ELECTRIC_LAMP_LIT, "lit before the save");
    assert_eq!(h.get_block(0, Y, 8), block::WINDMILL_TURNING, "turning before the save");

    let player = crate::player_slot::PlayerSlot::new(0, Vec3::new(0.0, 80.0, 0.0), 1.0);
    crate::save::save_world(name, h.world(), &[player], 42, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    crate::save::load_world(name, &mut loaded).expect("load_world");
    scrub(name);

    // The load path clears `energised` and re-enqueues every device, so the
    // lit-ness below is rederived, not restored.
    assert!(!loaded.power.is_on((2, Y, 0)), "the flood itself is not saved");
    assert!(loaded.power_device_at((0, Y, 0)).unwrap().on, "the lever is still latched");
    let mill = loaded.power_device_at((0, Y, 8)).expect("the mill's device survived");
    assert_eq!(mill.kind, PowerDeviceKind::Windmill);
    assert_eq!(mill.facing, Facing::South, "its facing came back too");
    assert!(mill.on, "and it was saved mid-turn");

    let registry = block::BlockRegistry::new();
    let gale = WindSample { speed: 0.9, direction: 0 };
    crate::power::power_tick(&mut loaded, 1, &[], gale, &registry);
    assert_eq!(loaded.get_block(2, Y, 0), block::CABLE_LIT, "the lever's run relights");
    assert_eq!(loaded.get_block(4, Y, 0), block::ELECTRIC_LAMP_LIT, "…and its lamp");
    assert_eq!(loaded.get_block(0, Y, 8), block::WINDMILL_TURNING, "the mill keeps turning");
    assert_eq!(loaded.get_block(0, Y, 10), block::ELECTRIC_LAMP_LIT, "…and lights its own lamp");
}

// ── Case 9 — the multiplayer block-change broadcast ─────────────────────────

use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol;
use crate::transport::{ChannelClientTransport, ClientTransport};

/// A hosted server with no accept thread, no sockets and an open join policy —
/// the shape `late_join.rs` uses. The world name must not exist on disk.
fn start_open_server(world: &str) -> HostedServer {
    HostedServer::start(1, world.to_string(), 42, 0, RemoteTransport::WebSocket { port: 0 })
        .expect("hosted server starts")
}

fn send_guest_join(client: &ChannelClientTransport) {
    let req = crate::remote_client::build_join_request_guest("Sparky", 0);
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
}

/// Every block change the joiner has been sent so far.
fn drain_block_changes(client: &ChannelClientTransport) -> Vec<protocol::BlockChange> {
    let mut out = Vec::new();
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
            && ptype == protocol::PacketType::StateUpdate
            && let Ok(state) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
        {
            out.extend(state.block_changes);
        }
    }
    out
}

/// Wire a lever → cable → lamp straight into a server world (the shape a loaded
/// save has: blocks plus their devices).
fn wire_a_lamp(world: &mut World, origin: (i32, i32, i32)) {
    let (x, y, z) = origin;
    for (dx, blk, kind) in [
        (0, block::LEVER, Some(PowerDeviceKind::Lever)),
        (1, block::CABLE, None),
        (2, block::CABLE, None),
        (3, block::ELECTRIC_LAMP, Some(PowerDeviceKind::ElectricLamp)),
    ] {
        world.set_block(x + dx, y, z, blk);
        if let Some(kind) = kind {
            world.insert_power_device(
                (x + dx, y, z),
                PowerDeviceData::new(kind, Facing::East),
            );
        }
        world.mark_dirty((x + dx, y, z));
        world.notify_neighbours((x + dx, y, z));
    }
}

#[test]
fn a_joined_client_is_sent_the_cable_and_lamp_flips() {
    // The host's power sim is authoritative and its flips ride
    // `pending_block_changes` onto every StateUpdate. This is the whole reason
    // a second player sees the lights come on.
    let mut hs = start_open_server("electricity-broadcast-test");
    let origin = (2, 70, 2);
    wire_a_lamp(&mut hs.server.world, origin);

    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();
    let _ = drain_block_changes(&client); // clear the join backlog

    // The host flips the switch.
    hs.server
        .world
        .power_device_at_mut(origin)
        .expect("lever")
        .on = true;
    hs.server.world.mark_dirty(origin);
    hs.server.world.notify_neighbours(origin);
    hs.tick();

    let changes = drain_block_changes(&client);
    let at = |dx: i32| {
        changes
            .iter()
            .find(|bc| (bc.x, bc.y, bc.z) == (origin.0 + dx, origin.1, origin.2))
            .map(|bc| bc.new_block)
    };
    assert_eq!(at(1), Some(block::CABLE_LIT), "the joiner is told the near cable lit");
    assert_eq!(at(2), Some(block::CABLE_LIT), "…and the far one");
    assert_eq!(at(3), Some(block::ELECTRIC_LAMP_LIT), "…and that the lamp came on");

    // Switching off broadcasts the other way too.
    hs.server
        .world
        .power_device_at_mut(origin)
        .expect("lever")
        .on = false;
    hs.server.world.mark_dirty(origin);
    hs.server.world.notify_neighbours(origin);
    hs.tick();
    let changes = drain_block_changes(&client);
    assert!(
        changes.iter().any(|bc| (bc.x, bc.y, bc.z)
            == (origin.0 + 3, origin.1, origin.2)
            && bc.new_block == block::ELECTRIC_LAMP),
        "the lamp going out is broadcast as well"
    );
}

#[test]
fn a_block_a_joiner_places_gets_its_device_and_its_facing_on_the_host() {
    // Found by the end-to-end audit: the host applied a joiner's block change
    // as a bare `set_block`, dropping BOTH the metadata byte and the
    // `PowerDevice`. A joiner could build a whole circuit that existed on the
    // host as scenery — nothing sourced, nothing lit — and a Logic Gate placed
    // by a joiner faced north on the host whatever way they pointed it.
    let mut hs = start_open_server("electricity-joiner-place-test");
    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();

    // A joined remote player is server-simulated, so the reach check runs
    // against the position the SERVER holds for them — place next door to it.
    let idx = hs
        .server
        .players
        .iter()
        .position(|p| p.server_simulated)
        .expect("the guest took a slot");
    let at = hs.server.players[idx].player.pos;
    let cell = (at.x.floor() as i32 + 1, at.y.floor() as i32, at.z.floor() as i32);
    let facing = Facing::South;
    // The packet is built the way the CLIENT builds it: a place arm writes the
    // block and its facing into the placing player's own world, then
    // `game_loop::broadcast_change` reads that cell back. Nothing here
    // hand-writes a metadata byte — if the client-side push ever goes back to
    // guessing `meta: 0`, this test goes red with it (and see
    // `electricity::no_client_block_change_push_guesses_its_metadata`).
    let mut placer = host();
    placer.place_power_block(cell.0, cell.1, cell.2, block::LOGIC_GATE, facing);
    let mut input = protocol::InputPacket {
        tick: 1,
        x: at.x,
        y: at.y,
        z: at.z,
        health: 20.0,
        ..Default::default()
    };
    input.block_changes.push(crate::game_loop::broadcast_change(
        placer.world(),
        cell.0,
        cell.1,
        cell.2,
        block::LOGIC_GATE,
    ));
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ClientInput,
        &input,
    ));
    hs.tick();

    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::LOGIC_GATE);
    let device = hs
        .server
        .world
        .power_device_at(cell)
        .expect("the host registered a device for the joiner's power block");
    assert_eq!(device.kind, PowerDeviceKind::LogicGate);
    assert_eq!(device.facing, facing, "the facing the joiner sent is the facing the host stores");
    assert_eq!(
        crate::meta::facing(hs.server.world.meta_at(cell.0, cell.1, cell.2)),
        facing,
        "…and the metadata byte itself survived the wire"
    );

    // Breaking it takes the device with it — no ghost source on the host.
    let mut breaker = protocol::InputPacket {
        tick: 2,
        x: at.x,
        y: at.y,
        z: at.z,
        health: 20.0,
        ..Default::default()
    };
    placer.break_power_block(cell.0, cell.1, cell.2);
    breaker.block_changes.push(crate::game_loop::broadcast_change(
        placer.world(),
        cell.0,
        cell.1,
        cell.2,
        block::AIR,
    ));
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ClientInput,
        &breaker,
    ));
    hs.tick();
    assert!(
        hs.server.world.power_device_at(cell).is_none(),
        "breaking the block drops the device — a forgotten one powers the run for ever"
    );
}

// ── Case 10 — the joiner's switch (DeviceInteract, protocol v62) ────────────
//
// The end-to-end audit found the hole this closes: autonomous sources reach the
// host through the server's own sim, but a SWITCH had no carrier on the wire at
// all, so a joiner's lever flipped nothing but their own copy of the world. The
// host is the authority — it re-derives the interaction from the cell alone.

/// The cell a joined guest can legitimately reach, and the cell the server has
/// them standing in. A joined remote player is server-simulated, so the reach
/// gate runs against the position the SERVER holds — not one they can assert.
fn guest_slot(hs: &HostedServer) -> (usize, glam::Vec3) {
    let idx = hs
        .server
        .players
        .iter()
        .position(|p| p.server_simulated)
        .expect("the guest took a slot");
    (idx, hs.server.players[idx].player.pos)
}

fn send_device_interact(client: &ChannelClientTransport, pos: (i32, i32, i32)) {
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::DeviceInteract,
        &protocol::DeviceInteractPacket { pos },
    ));
}

#[test]
fn a_joiners_lever_lights_the_hosts_lamp_and_comes_back_on_the_wire() {
    let mut hs = start_open_server("electricity-device-interact-test");
    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();
    let (_idx, at) = guest_slot(&hs);

    // Wire the run starting one block from where the server has them standing,
    // so the lever itself is inside the reach envelope.
    let origin = (at.x.floor() as i32 + 1, at.y.floor() as i32, at.z.floor() as i32);
    wire_a_lamp(&mut hs.server.world, origin);
    hs.tick();
    let _ = drain_block_changes(&client); // clear the join + wiring backlog
    assert!(!hs.server.world.power_device_at(origin).expect("lever").on);

    // The joiner right-clicks the lever. Nothing was applied on their side —
    // this packet is the whole interaction.
    send_device_interact(&client, origin);
    hs.tick();
    assert!(
        hs.server.world.power_device_at(origin).expect("lever").on,
        "the host latched the joiner's lever"
    );
    // The power sim settles the run over the next tick or two.
    hs.tick();
    hs.tick();
    assert_eq!(
        hs.server.world.get_block(origin.0 + 3, origin.1, origin.2),
        block::ELECTRIC_LAMP_LIT,
        "…and the host's own lamp came on"
    );

    let changes = drain_block_changes(&client);
    let at_cell = |dx: i32| {
        changes
            .iter()
            .find(|bc| (bc.x, bc.y, bc.z) == (origin.0 + dx, origin.1, origin.2))
    };
    assert_eq!(
        at_cell(0).map(|bc| crate::meta::state(bc.meta)),
        Some(1),
        "the joiner is told the lever handle is now up"
    );
    assert_eq!(at_cell(1).map(|bc| bc.new_block), Some(block::CABLE_LIT));
    assert_eq!(
        at_cell(3).map(|bc| bc.new_block),
        Some(block::ELECTRIC_LAMP_LIT),
        "…and that the lamp is lit"
    );

    // An unrelated edit next to the run re-seeds the network from the lever the
    // host now holds latched. Nothing may darken: this is the server half of
    // the "the flip must live in the DEVICE, not just in the meta byte" rule
    // (`world::apply_remote_block_change_latches_the_lever_the_broadcast_
    // describes` is the client half).
    hs.server.world.set_block(origin.0 + 1, origin.1 + 1, origin.2, block::STONE);
    hs.server.world.notify_neighbours((origin.0 + 1, origin.1 + 1, origin.2));
    hs.tick();
    hs.tick();
    assert_eq!(
        hs.server.world.get_block(origin.0 + 3, origin.1, origin.2),
        block::ELECTRIC_LAMP_LIT,
        "a nearby edit re-seeds the run and it stays lit"
    );
    let _ = drain_block_changes(&client);

    // A second interact toggles it back off, all the way down the run.
    send_device_interact(&client, origin);
    hs.tick();
    hs.tick();
    hs.tick();
    assert!(!hs.server.world.power_device_at(origin).expect("lever").on);
    let changes = drain_block_changes(&client);
    assert_eq!(
        changes
            .iter()
            .find(|bc| (bc.x, bc.y, bc.z) == (origin.0 + 3, origin.1, origin.2))
            .map(|bc| bc.new_block),
        Some(block::ELECTRIC_LAMP),
        "the lamp going out is broadcast too"
    );
}

#[test]
fn a_device_interact_out_of_reach_or_on_a_plain_block_is_ignored() {
    let mut hs = start_open_server("electricity-device-interact-reject-test");
    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();
    let (_idx, at) = guest_slot(&hs);
    let base = (at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);

    // (a) A lever the joiner could never touch from where the server has them.
    let far = (base.0 + 60, base.1, base.2);
    wire_a_lamp(&mut hs.server.world, far);
    // (b) A lever they CAN reach, so the test proves the reach gate is what
    //     refused (a) rather than the fixture being wrong.
    let near = (base.0 + 1, base.1, base.2);
    wire_a_lamp(&mut hs.server.world, near);
    hs.tick();

    send_device_interact(&client, far);
    hs.tick();
    assert!(
        !hs.server.world.power_device_at(far).expect("lever").on,
        "a lever 60 blocks away is out of reach — silently dropped"
    );

    // (c) A cell with no device at all, and (d) a cell with a device that is
    //     NOT toggle-class (a cable is not a switch). Neither may panic or
    //     mutate anything.
    hs.server.world.set_block(base.0, base.1 + 2, base.2, block::STONE);
    send_device_interact(&client, (base.0, base.1 + 2, base.2));
    send_device_interact(&client, (near.0 + 1, near.1, near.2)); // the cable
    hs.tick();
    assert_eq!(
        hs.server.world.get_block(base.0, base.1 + 2, base.2),
        block::STONE,
        "interacting with a plain block changes nothing"
    );
    assert_eq!(
        hs.server.world.get_block(near.0 + 1, near.1, near.2),
        block::CABLE,
        "a cable is not a switch"
    );

    // …and the reachable lever still works, so nothing above disabled the path.
    send_device_interact(&client, near);
    hs.tick();
    assert!(
        hs.server.world.power_device_at(near).expect("lever").on,
        "the reachable lever still flips"
    );
}

#[test]
fn a_generators_lit_twin_reaches_the_joiner_with_its_facing_intact() {
    // Review carry-over: the power path built its twin swaps with
    // `BlockChange::new`, i.e. `meta: 0`. Every power block wears a facing byte
    // from placement, so a Steam Generator catching light — or a Windmill
    // catching the wind — turned to face north on every other client.
    let mut hs = start_open_server("electricity-lit-twin-facing-test");
    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();
    let _ = drain_block_changes(&client);

    let cell = (3, 70, 3);
    let facing = Facing::West;
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::STEAM_GENERATOR);
    hs.server.world.set_meta(cell, crate::meta::with_facing(0, facing));
    hs.server
        .world
        .insert_power_device(cell, PowerDeviceData::new(PowerDeviceKind::SteamGenerator, facing));
    assert!(
        crate::power::try_load_generator_fuel(
            hs.server.world.power_device_at_mut(cell).expect("generator"),
            &crate::item::ItemStack::new_material(crate::item::MaterialId::Stick, 1),
        ),
        "a stick is a valid furnace fuel"
    );
    hs.server.world.mark_dirty(cell);
    hs.server.world.notify_neighbours(cell);

    // The burner lights on the next device sweep and swaps to the lit twin.
    let mut lit = None;
    for _ in 0..5 {
        hs.tick();
        if let Some(bc) = drain_block_changes(&client)
            .into_iter()
            .find(|bc| (bc.x, bc.y, bc.z) == cell)
        {
            lit = Some(bc);
            break;
        }
    }
    let lit = lit.expect("the generator's lit twin is broadcast");
    assert_eq!(lit.new_block, block::STEAM_GENERATOR_LIT);
    assert_eq!(
        crate::meta::facing(lit.meta),
        facing,
        "the twin swap carries the facing the generator was placed with"
    );
    assert_eq!(
        crate::meta::facing(hs.server.world.meta_at(cell.0, cell.1, cell.2)),
        facing,
        "…and the host's own world kept it too"
    );
}

/// Task 2b review fix (b). A lever flip reaches the server as a *same-kind
/// meta-only* block change — LEVER to LEVER, state bit moved — because that is
/// what `power::interact_device` produces and what the host's own client puts
/// in its `InputPacket`. The server rebuilds a `PowerDevice` only when the block
/// KIND changes, so before the fix that change wrote the metadata byte and left
/// the authoritative device latched the old way: the server's sim disagreed with
/// the host about every switch the host threw, and the lamp never came on in the
/// world everyone else is being sent.
#[test]
fn a_meta_only_lever_flip_latches_the_servers_own_device() {
    let mut hs = start_open_server("electricity-meta-only-flip-test");
    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();
    let (_idx, at) = guest_slot(&hs);

    let origin = (at.x.floor() as i32 + 1, at.y.floor() as i32, at.z.floor() as i32);
    wire_a_lamp(&mut hs.server.world, origin);
    hs.tick();
    let _ = drain_block_changes(&client);
    assert!(!hs.server.world.power_device_at(origin).expect("lever").on);

    // The flip, exactly as the interaction builds it: same block, state bit up.
    let facing_meta = hs.server.world.meta_at(origin.0, origin.1, origin.2);
    let mut input = protocol::InputPacket {
        tick: 1,
        x: at.x,
        y: at.y,
        z: at.z,
        health: 20.0,
        ..Default::default()
    };
    input.block_changes.push(protocol::BlockChange::with_meta(
        origin.0,
        origin.1,
        origin.2,
        block::LEVER,
        crate::meta::with_state(facing_meta, 1),
    ));
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ClientInput,
        &input,
    ));
    hs.tick();

    assert!(
        hs.server.world.power_device_at(origin).expect("lever").on,
        "the state bit on the wire latches the SERVER's device, not just its meta byte"
    );
    hs.tick();
    hs.tick();
    assert_eq!(
        hs.server.world.get_block(origin.0 + 3, origin.1, origin.2),
        block::ELECTRIC_LAMP_LIT,
        "…so the authoritative sim lights the lamp"
    );
    let changes = drain_block_changes(&client);
    assert!(
        changes.iter().any(|bc| (bc.x, bc.y, bc.z)
            == (origin.0 + 3, origin.1, origin.2)
            && bc.new_block == block::ELECTRIC_LAMP_LIT),
        "…and everyone else is told about it"
    );
}

/// Task 2b review fix (5). `DeviceInteract` has its own per-tick budget: it does
/// NOT share the block-change one, and `MAX_PACKETS_PER_TICK` alone would let a
/// client send ten a tick. Two per tick is well above the 8-tick client-side
/// place cooldown that gates a real right-click.
#[test]
fn a_client_cannot_spend_more_than_its_device_interact_budget_in_one_tick() {
    let mut hs = start_open_server("electricity-interact-budget-test");
    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();
    let (_idx, at) = guest_slot(&hs);

    let origin = (at.x.floor() as i32 + 1, at.y.floor() as i32, at.z.floor() as i32);
    wire_a_lamp(&mut hs.server.world, origin);
    hs.tick();

    // Five flips in one tick. Only the budget's worth are applied, so the lever
    // lands on the parity the BUDGET dictates (2 → off), not the parity five
    // flips would give (odd → on).
    for _ in 0..5 {
        send_device_interact(&client, origin);
    }
    hs.tick();
    assert!(
        !hs.server.world.power_device_at(origin).expect("lever").on,
        "only 2 of the 5 interacts were spent, so the lever is back down"
    );
    // The budget resets each tick — the next one still works.
    send_device_interact(&client, origin);
    hs.tick();
    assert!(hs.server.world.power_device_at(origin).expect("lever").on);
}

/// Task 2b review fix (2). `interact_device` is reached from a network packet,
/// so a kind that `is_toggle_class` accepts but the match does not handle must
/// drop the interaction, never panic the host. Every accepted kind is handled
/// today — this pins that, so the mismatch can never be introduced silently.
#[test]
fn every_toggle_class_kind_has_an_interaction() {
    use crate::world::World;
    for (blk, kind) in [
        (block::LEVER, PowerDeviceKind::Lever),
        (block::BUTTON, PowerDeviceKind::Button),
        (block::PLUNGER_DETONATOR, PowerDeviceKind::PlungerDetonator),
        (block::HAND_CRANK, PowerDeviceKind::HandCrank),
        (block::MIRROR, PowerDeviceKind::Mirror),
    ] {
        assert!(crate::power::is_toggle_class(kind), "{kind:?} is toggle-class");
        assert_eq!(
            crate::power::device_kind_for_block(blk),
            Some(kind),
            "the client's block gate and the host's kind gate name the same device"
        );
        let mut w = World::new();
        let p = (0, 64, 0);
        w.set_block(p.0, p.1, p.2, blk);
        w.insert_power_device(p, PowerDeviceData::new(kind, Facing::East));
        assert!(
            crate::power::interact_device(&mut w, p, 1).is_some(),
            "{kind:?} is accepted by is_toggle_class, so it must have an arm"
        );
    }
    // …and a kind that is NOT toggle-class is refused rather than mishandled.
    let mut w = World::new();
    let p = (0, 64, 0);
    w.set_block(p.0, p.1, p.2, block::ELECTRIC_LAMP);
    w.insert_power_device(p, PowerDeviceData::new(PowerDeviceKind::ElectricLamp, Facing::Up));
    assert!(crate::power::interact_device(&mut w, p, 1).is_none());
}

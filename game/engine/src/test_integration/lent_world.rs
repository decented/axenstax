//! D1 — a host lends its world to its embedded server (`crate::sim_lend`).
//!
//! These drive a REAL `HostedServer` through the lend window the game loop
//! opens every tick (`sim_lend::LentSim`), with an `OwnedSimParts` standing in
//! for the host client's fields: the world a normal server loaded is taken out
//! of it as the host's own, and from then on the server only ever sees it
//! inside the window. Joins and edits ride the in-process channel transport.
//!
//! What they pin: each shared sim system runs on exactly one side (the
//! server's rows here, the host client's never); the server reads the host's
//! clock and never advances it; joiners are diffed from the host's real ECS;
//! the host's own edits are broadcast, never re-validated or sent back; a
//! joiner's edit or a server-made change comes back for the host to remesh;
//! state the host made this session (a filled chest, a plot, a vendor) is
//! what a joiner meets, with no mirror in between.

use super::joiner_authority::{
    accepted, block_changes_seen, bones_in_ecs, cell_beside, chest_of_bones, send_edits,
    send_guest_join, send_host_edits, start_open_server,
};
use crate::block;
use crate::hosted_server::HostedServer;
use crate::protocol;
use crate::sim_lend::{OwnedSimParts, SimSide, SimSystem};
use crate::transport::ChannelClientTransport;

/// A lending host: a normal server loads (generates) a world, which is then
/// taken out of it as the host client's own. Nothing has been broadcast yet.
pub(super) fn start_lent(tag: &str) -> (HostedServer, OwnedSimParts) {
    let mut hs = start_open_server(&format!("lent-{tag}"));
    let host = OwnedSimParts::take_from(&mut hs);
    (hs, host)
}

/// Attach + join a guest through one lent tick; returns (client, slot).
pub(super) fn join_guest_lent(
    hs: &mut HostedServer,
    host: &mut OwnedSimParts,
    name: &str,
) -> (ChannelClientTransport, usize) {
    let client = hs.attach_test_remote();
    send_guest_join(&client, name);
    host.lend_tick(hs);
    let slot = accepted(&client).expect("guest join accepted").player_index as usize;
    (client, slot)
}

fn entity_spawns_seen(client: &ChannelClientTransport) -> Vec<protocol::EntitySpawn> {
    use crate::transport::ClientTransport;
    let mut out = Vec::new();
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
            && ptype == protocol::PacketType::StateUpdate
            && let Ok(s) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
        {
            out.extend(s.entity_spawns);
        }
    }
    out
}

#[test]
fn a_lent_tick_runs_only_the_servers_systems_and_never_moves_the_hosts_clock() {
    let (mut hs, mut host) = start_lent("tally");
    for _ in 0..3 {
        let before = host.world.sim_tally;
        host.lend_tick(&mut hs);
        let after = host.world.sim_tally;
        for s in SimSystem::ALL {
            let runs = after.get(s) - before.get(s);
            match s.lent_owner() {
                SimSide::HostClient => assert_eq!(runs, 0, "{s:?} is the host client's to run"),
                SimSide::Server if s.every_tick() => assert_eq!(runs, 1, "{s:?} runs once a tick"),
                SimSide::Server => assert!(runs <= 1, "{s:?} ran {runs} times"),
            }
        }
        // The clock the host handed in, never +1: the host owns `/time`.
        assert_eq!(hs.server.world_time, host.clock.world_time);
        assert_eq!(hs.server.tick_counter, host.clock.tick_counter);
        assert_eq!(hs.server.weather, host.clock.weather);
    }
    // Outside the window the server holds nothing of the host's.
    assert!(!hs.server.lent);
    assert!(hs.server.loaded_columns.is_empty());
    assert_eq!(hs.server.ecs.len(), 0);
}

#[test]
fn a_joiners_diff_carries_the_hosts_own_mobs() {
    let (mut hs, mut host) = start_lent("mobs");
    let at = hs.server.players[0].player.pos;
    let cow = crate::entity::spawn_mob(
        &mut host.ecs,
        crate::mob::MobType::Cow,
        at + glam::Vec3::new(2.0, 0.0, 0.0),
    );
    let client = hs.attach_test_remote();
    send_guest_join(&client, "Visitor");
    host.lend_tick(&mut hs);
    host.lend_tick(&mut hs);

    let spawns = entity_spawns_seen(&client);
    let cow_id = host
        .ecs
        .get::<&crate::entity::ProtocolId>(cow)
        .expect("the host's own cow is broadcast")
        .0;
    let cows: Vec<_> = spawns.iter().filter(|s| s.id == cow_id).collect();
    assert_eq!(cows.len(), 1, "the host's cow reaches the joiner exactly once");
    assert_eq!(cows[0].kind, protocol::EntityKind::Cow);
    // Every mob in the host's world reached the joiner — the diff is the
    // host's ECS, not a second population.
    let mut host_mobs = 0;
    for (_e, (pid, _kind)) in host
        .ecs
        .query::<(&crate::entity::ProtocolId, &crate::entity::MobKind)>()
        .iter()
    {
        host_mobs += 1;
        assert!(spawns.iter().any(|s| s.id == pid.0), "host mob {} not sent", pid.0);
    }
    assert!(host_mobs >= 1);
    assert_eq!(hs.server.ecs.len(), 0, "the server keeps no population of its own");
}

#[test]
fn every_host_edit_reaches_joiners_and_none_is_validated_or_sent_back() {
    let (mut hs, mut host) = start_lent("host-edits");
    let (client, _slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    let _ = block_changes_seen(&client);
    // The host's client made these in its own world (as its break/place arms
    // do), more than an owning server's budget of 4, one far out of reach.
    let mut edits = Vec::new();
    for dx in 1..=5 {
        edits.push((cell_beside(&hs, 0, dx, 3, 0), block::GLASS));
    }
    edits.push((cell_beside(&hs, 0, 30, 3, 0), block::GLASS));
    for &((x, y, z), b) in &edits {
        host.world.set_block(x, y, z, b);
    }
    send_host_edits(&hs, 1, &edits);
    host.lend_tick(&mut hs);

    let seen = block_changes_seen(&client);
    for &(c, b) in &edits {
        assert!(
            seen.iter().any(|bc| (bc.x, bc.y, bc.z) == c && bc.new_block == b),
            "host edit at {c:?} broadcast"
        );
        assert!(
            !seen.iter().any(|bc| (bc.x, bc.y, bc.z) == c && bc.new_block != b),
            "host edit at {c:?} was sent back"
        );
    }
    let (_, edit_cells) = hs.take_lent_changes();
    assert!(edit_cells.is_empty(), "the host's own edits are meshed by its own client");
}

#[test]
fn a_joiner_edit_lands_in_the_hosts_world_and_comes_back_to_remesh() {
    let (mut hs, mut host) = start_lent("joiner-edit");
    let (client, slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    let _ = hs.take_lent_changes();
    let cell = cell_beside(&hs, slot, 1, 1, 0);
    host.world.set_block(cell.0, cell.1, cell.2, block::AIR);
    send_edits(&hs, &client, slot, 1, &[(cell, block::GLASS)]);
    host.lend_tick(&mut hs);
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), block::GLASS);
    let (_, edit_cells) = hs.take_lent_changes();
    assert!(edit_cells.contains(&cell), "the host remeshes a joiner's edit");
}

#[test]
fn a_server_made_change_comes_back_for_the_host_to_remesh() {
    let (mut hs, mut host) = start_lent("sand");
    // Sand over air, well inside the falling-block scan around the host.
    let sand = cell_beside(&hs, 0, 0, 8, 0);
    let below = (sand.0, sand.1 - 1, sand.2);
    host.world.set_block(sand.0, sand.1, sand.2, block::SAND);
    host.world.set_block(below.0, below.1, below.2, block::AIR);
    let mut sim = Vec::new();
    for _ in 0..4 {
        host.lend_tick(&mut hs);
        sim.extend(hs.take_lent_changes().0);
    }
    assert_eq!(host.world.get_block(below.0, below.1, below.2), block::SAND, "fell in the host's world");
    assert!(sim.iter().any(|bc| (bc.x, bc.y, bc.z) == sand && bc.new_block == block::AIR));
    assert!(sim.iter().any(|bc| (bc.x, bc.y, bc.z) == below && bc.new_block == block::SAND));
}

#[test]
fn a_joiner_break_spills_the_hosts_live_chest_once() {
    let (mut hs, mut host) = start_lent("live-chest");
    let (client, slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    // Filled this session in the host's chest UI — there is no other copy.
    host.world.set_block(cell.0, cell.1, cell.2, block::CHEST);
    host.world.insert_chest(cell, chest_of_bones(7));
    send_edits(&hs, &client, slot, 1, &[(cell, block::AIR)]);
    host.lend_tick(&mut hs);
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), block::AIR);
    assert!(host.world.chest_at(cell).is_none(), "no orphan in the host's world");
    assert_eq!(bones_in_ecs(&host.ecs), 7, "the live contents spill into the host's world");
    for _ in 0..3 {
        host.lend_tick(&mut hs);
    }
    assert_eq!(bones_in_ecs(&host.ecs), 7, "exactly one spill");
}

#[test]
fn a_plot_the_host_claimed_this_session_protects_against_a_joiner() {
    let (mut hs, mut host) = start_lent("live-plot");
    let (client, slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    host.world.set_block(cell.0, cell.1, cell.2, block::AIR);
    host.world.plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        cell.0,
        cell.1 - 5,
        cell.2,
    ));
    send_edits(&hs, &client, slot, 1, &[(cell, block::STONE)]);
    host.lend_tick(&mut hs);
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), block::AIR);
}

#[test]
fn a_vendor_the_host_placed_this_session_is_protected_from_a_joiner() {
    let (mut hs, mut host) = start_lent("live-vendor");
    let (client, slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    host.world.set_block(cell.0, cell.1, cell.2, block::VENDOR_BLOCK);
    host.world.insert_vendor(
        cell,
        crate::vendor::VendorData {
            owner: Some(crate::vendor::VendorOwner::LocalPlayer(0)),
            ..Default::default()
        },
    );
    send_edits(&hs, &client, slot, 1, &[(cell, block::AIR)]);
    host.lend_tick(&mut hs);
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), block::VENDOR_BLOCK);
}

#[test]
fn the_first_lend_clears_protocol_ids_an_earlier_server_left() {
    let (mut hs, mut host) = start_lent("stale-ids");
    let at = hs.server.players[0].player.pos;
    let cow = crate::entity::spawn_mob(
        &mut host.ecs,
        crate::mob::MobType::Cow,
        at + glam::Vec3::new(2.0, 0.0, 0.0),
    );
    host.ecs.insert_one(cow, crate::entity::ProtocolId(1_000_000)).unwrap();
    host.lend_tick(&mut hs);
    let id = host.ecs.get::<&crate::entity::ProtocolId>(cow).expect("re-numbered").0;
    assert_ne!(id, 1_000_000, "a stale id from an earlier server never reaches joiners");
}

/// One idle input from a joiner (no movement, no edits): the server steps a
/// joiner's body only on its inputs, so gravity acts only while they arrive.
fn send_idle(hs: &HostedServer, client: &ChannelClientTransport, slot: usize, seq: u64) {
    send_edits(hs, client, slot, seq, &[]);
}

/// D1 review fix 1 (HIGH) — the host walks far from a joiner. The host
/// client's streamer anchors on every joiner's server body
/// (`chunk_stream::client_stream_anchors`, fed `HostedServer::lent_joiner_columns`
/// exactly as `GameState::stream_chunks` feeds it), so the joiner's columns stay
/// loaded: their body stays on its ground and their edit there is accepted.
/// Before, the anchors were the host's own players alone — the column unloaded,
/// the body fell through server air and every edit there was refused Unloaded.
#[test]
fn a_joiners_columns_stay_loaded_when_the_host_walks_away() {
    use crate::chunk_stream::{
        client_stream_anchors, column_of, plan_stream_step_for, UNLOAD_HYSTERESIS,
    };
    let (mut hs, mut host) = start_lent("anchors");
    let (client, slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    assert!(
        hs.server.loaded_columns.is_empty(),
        "a lending server generates nothing of its own (one column-loading story)"
    );
    let mut seq = 1;
    for _ in 0..30 {
        send_idle(&hs, &client, slot, seq);
        seq += 1;
        host.lend_tick(&mut hs);
    }
    let settled = hs.server.players[slot].player.pos;
    let joiner_col = column_of(settled);
    assert_eq!(hs.lent_joiner_columns(), vec![joiner_col]);

    // The host walks off, beyond its render distance + hysteresis.
    let rd = 4;
    let host_col = (joiner_col.0 + rd + UNLOAD_HYSTERESIS + 1, joiner_col.1);
    let host_only = client_stream_anchors(&[host_col], rd, &[]);
    let pre_fix = plan_stream_step_for(&host_only, &[host_col], 0, &host.loaded_columns, |_, _| false);
    assert!(pre_fix.unload.contains(&joiner_col), "control: host anchors alone drop it");
    for _ in 0..3 {
        // Frames of the host's streamer, as `stream_chunks` runs it.
        let joiners = hs.lent_joiner_columns();
        let anchors = client_stream_anchors(&[host_col], rd, &joiners);
        let mut nearest = vec![host_col];
        nearest.extend_from_slice(&joiners);
        let step = plan_stream_step_for(&anchors, &nearest, 2, &host.loaded_columns, |_, _| false);
        let mut sims = host.column_sims(&hs.server);
        for &(cx, cz) in &step.unload {
            sims.stream_out(cx, cz);
        }
        for &(cx, cz) in &step.load {
            sims.stream_in(cx, cz);
        }
    }
    for dx in -1..=1 {
        for dz in -1..=1 {
            let col = (joiner_col.0 + dx, joiner_col.1 + dz);
            assert!(host.loaded_columns.contains(&col), "joiner's column {col:?} kept");
        }
    }

    // Their body stays on its ground…
    for _ in 0..40 {
        send_idle(&hs, &client, slot, seq);
        seq += 1;
        host.lend_tick(&mut hs);
    }
    let now = hs.server.players[slot].player.pos;
    assert!((now.y - settled.y).abs() < 0.5, "the joiner's body fell: {settled} -> {now}");

    // …and their edit there is accepted, in the host's world.
    let cell = cell_beside(&hs, slot, 1, 1, 0);
    host.world.set_block(cell.0, cell.1, cell.2, block::AIR);
    send_edits(&hs, &client, slot, seq, &[(cell, block::GLASS)]);
    host.lend_tick(&mut hs);
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), block::GLASS);
}

/// D1 review fix 3 (LOW) — a split-screen host: only player 1 sends input
/// over the loopback, so player 2's server slot stood where the server put it.
/// On a lent world the server runs power (plates) and mob spawning (anchors)
/// on the host's world from its slots, so every local slot's position must
/// reach it: `GameState::tick_hosted_server` hands them all to
/// `HostedServer::sync_local_slots` before each server tick.
#[test]
fn a_split_screen_second_player_presses_a_plate_on_a_lent_world() {
    let mut hs = HostedServer::start(
        2,
        format!("lent-split-screen-{}", std::process::id()),
        42,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
    )
    .expect("hosted server starts");
    let mut host = OwnedSimParts::take_from(&mut hs);
    // A plate in the air near the host, as the host client places one.
    let base = hs.server.players[0].player.pos;
    let plate = (base.x.floor() as i32 + 6, 120, base.z.floor() as i32);
    host.world.set_block(plate.0, plate.1, plate.2, block::PRESSURE_PLATE);
    host.world.insert_power_device(
        plate,
        crate::power::PowerDeviceData::new(
            crate::power::device_kind_for_block(block::PRESSURE_PLATE).expect("a device"),
            crate::meta::Facing::Up,
        ),
    );
    host.world.set_meta(plate, crate::meta::with_facing(0, crate::meta::Facing::Up));
    host.world.mark_dirty(plate);
    host.world.notify_neighbours(plate);
    let on = |host: &OwnedSimParts| host.world.power_device_at(plate).expect("plate").on;

    host.lend_tick(&mut hs);
    host.lend_tick(&mut hs);
    assert!(!on(&host), "nobody on the plate yet");

    // Player 2 (local slot 1) walks onto it in the host client.
    let on_plate = glam::Vec3::new(plate.0 as f32 + 0.5, (plate.1 + 1) as f32, plate.2 as f32 + 0.5);
    let p1 = hs.server.players[0].player.pos;
    let slots = [(p1, 0.0, 0.0, 20.0), (on_plate, 0.0, 0.0, 20.0)];
    hs.sync_local_slots(&slots);
    host.lend_tick(&mut hs);
    host.lend_tick(&mut hs);
    assert_eq!(hs.server.players[1].player.pos, on_plate);
    assert!(on(&host), "player 2 presses the plate on the lent world");
    assert_eq!(hs.server.players[0].player.pos, p1, "slot 0 is fed by its own input");
}

/// `sync_local_slots` touches local slots 1.. only: never slot 0 (its input
/// carries it), never a joiner's server-simulated slot, never a non-finite
/// position.
#[test]
fn sync_local_slots_never_moves_slot_zero_or_a_joiner() {
    let (mut hs, mut host) = start_lent("sync-slots");
    let (_client, slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    let p0 = hs.server.players[0].player.pos;
    let joiner = hs.server.players[slot].player.pos;
    let far = glam::Vec3::new(500.0, 90.0, 500.0);
    hs.sync_local_slots(&vec![(far, 0.0, 0.0, 20.0); slot + 1]);
    assert_eq!(hs.server.players[0].player.pos, p0);
    assert_eq!(hs.server.players[slot].player.pos, joiner);
}

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
    // Every mob of the host's world near the joiner reached it — the diff is
    // the host's ECS, not a second population — and none far from it did
    // (MP-D2a: a joiner hears about the entities in its interest radius).
    let joiner = hs.server.players.last().expect("the joiner's slot").player.pos;
    let mut near_mobs = 0;
    for (_e, (pid, pos, _kind)) in host
        .ecs
        .query::<(&crate::entity::ProtocolId, &crate::entity::Position, &crate::entity::MobKind)>()
        .iter()
    {
        let d = glam::Vec2::new(pos.0.x - joiner.x, pos.0.z - joiner.z).length();
        if d <= crate::entity_broadcast::INTEREST_ENTER_RADIUS {
            near_mobs += 1;
            assert!(spawns.iter().any(|s| s.id == pid.0), "host mob {} not sent", pid.0);
        } else if d > crate::entity_broadcast::INTEREST_LEAVE_RADIUS {
            assert!(!spawns.iter().any(|s| s.id == pid.0), "far host mob {} sent", pid.0);
        }
    }
    assert!(near_mobs >= 1);
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
    let host_only = client_stream_anchors(&[host_col], rd, &[], &[]);
    let pre_fix = plan_stream_step_for(&host_only, &[host_col], 0, &host.loaded_columns, |_, _| false);
    assert!(pre_fix.unload.contains(&joiner_col), "control: host anchors alone drop it");
    for _ in 0..3 {
        // Frames of the host's streamer, as `stream_chunks` runs it.
        let joiners = hs.lent_joiner_columns();
        let anchors = client_stream_anchors(&[host_col], rd, &joiners, &[]);
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

/// One frame of the host client's streamer over `host`'s lent world, as
/// `GameState::stream_chunks` runs it: the host at `host_col` (render distance
/// `rd`), anchors on every joiner's body and every dead joiner's spawn column,
/// `budget` loads a frame (`None` loads nothing — an unload-only frame).
fn stream_frame(
    hs: &HostedServer,
    host: &mut OwnedSimParts,
    host_col: (i32, i32),
    rd: i32,
    budget: usize,
    load: bool,
) {
    use crate::chunk_stream::{client_stream_anchors, plan_stream_step_for};
    let joiners = hs.lent_joiner_columns();
    let respawns = hs.lent_respawn_columns();
    let anchors = client_stream_anchors(&[host_col], rd, &joiners, &respawns);
    let mut nearest = vec![host_col];
    nearest.extend_from_slice(&joiners);
    nearest.extend_from_slice(&respawns);
    let step = plan_stream_step_for(&anchors, &nearest, budget, &host.loaded_columns, |_, _| false);
    let mut sims = host.column_sims(&hs.server);
    for &(cx, cz) in &step.unload {
        sims.stream_out(cx, cz);
    }
    if load {
        for &(cx, cz) in &step.load {
            sims.stream_in(cx, cz);
        }
    }
}

/// Where the joiner has been told it respawned (`Respawned` events seen).
fn respawns_seen(client: &ChannelClientTransport) -> Vec<glam::Vec3> {
    use crate::transport::ClientTransport;
    let mut out = Vec::new();
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
            && ptype == protocol::PacketType::PlayerEvent
            && let Ok(e) = protocol::safe_deserialize::<protocol::PlayerEventPacket>(payload)
            && let protocol::PlayerEventType::Respawned { x, y, z } = e.event
        {
            out.push(glam::Vec3::new(x, y, z));
        }
    }
    out
}

/// Final review fix 1 (MEDIUM) — a joiner on a lent host dies a long way from
/// where it joined, the host having walked there with it. Its spawn column was
/// kept by nobody (the host's anchors and the joiner's body are both far off),
/// so it is unloaded — and a respawn would stand the body in air above the raw
/// spawn point (`GameServer::standing_spot` reads only what is loaded), the
/// streamer loading the ground a frame too late: buried, stuck, or falling
/// with damage. A dead joiner's spawn column now anchors the host's streamer,
/// and the server ignores the Respawn (the client re-sends it until
/// `Respawned` arrives) until that column is loaded.
#[test]
fn a_joiner_who_dies_far_from_home_respawns_only_once_its_spawn_column_loads() {
    use crate::chunk_stream::column_of;
    use crate::transport::ClientTransport;
    let (mut hs, mut host) = start_lent("far-death");
    let (client, slot) = join_guest_lent(&mut hs, &mut host, "Traveller");
    let mut seq = 1;
    for _ in 0..30 {
        send_idle(&hs, &client, slot, seq);
        seq += 1;
        host.lend_tick(&mut hs);
    }
    let spawn = hs.server.players[slot].spawn_pos;
    let spawn_col = column_of(spawn);
    let ring = |c: (i32, i32)| (-1..=1).flat_map(move |dx| (-1..=1).map(move |dz| (c.0 + dx, c.1 + dz)));
    assert!(ring(spawn_col).all(|c| host.loaded_columns.contains(&c)), "control: home is loaded");

    // The host and the joiner travel 30 columns off (the joiner's body is
    // placed in the far terrain, as a long walk would leave it) and the host's
    // streamer drops the ground they left behind.
    let rd = 4;
    let host_col = (spawn_col.0 - 30, spawn_col.1);
    hs.server.players[slot].player.pos.x += 30.0 * 16.0;
    stream_frame(&hs, &mut host, host_col, rd, 0, false);
    assert!(
        ring(spawn_col).all(|c| !host.loaded_columns.contains(&c)),
        "control: the far trip left the spawn columns unloaded"
    );

    // The joiner dies there.
    assert!(hs.server.players[slot].combat.take_damage(1000.0));
    for _ in 0..25 {
        host.lend_tick(&mut hs);
    }
    assert!(hs.server.players[slot].combat.dead);
    assert_eq!(hs.lent_respawn_columns(), vec![spawn_col], "a dead joiner's spawn is an anchor");
    while client.try_recv_from_server().is_some() {}

    // Asked at once — before the streamer has loaded it — the Respawn is
    // ignored: no body in air above ground that is not there yet.
    let respawn = || {
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Respawn, &()));
    };
    respawn();
    host.lend_tick(&mut hs);
    assert!(respawns_seen(&client).is_empty(), "ignored while its column is unloaded");
    assert!(hs.server.players[slot].combat.dead, "still dead");

    // The host's streamer frames load it (nothing else wanted nearer first).
    for _ in 0..30 {
        stream_frame(&hs, &mut host, host_col, rd, 2, true);
        if ring(spawn_col).all(|c| host.loaded_columns.contains(&c)) {
            break;
        }
    }
    assert!(host.loaded_columns.contains(&spawn_col), "the streamer loaded the spawn column");

    // The client re-sends; now it lands on the ground, whole.
    respawn();
    host.lend_tick(&mut hs);
    let at = respawns_seen(&client);
    assert_eq!(at.len(), 1, "respawned once the column loaded: {at:?}");
    let at = at[0];
    let (bx, by, bz) = (at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
    assert_eq!(host.world.get_block(bx, by, bz), block::AIR, "feet in the open");
    assert_ne!(host.world.get_block(bx, by - 1, bz), block::AIR, "on solid ground, not in air ({at:?})");
    assert!(!hs.server.players[slot].combat.dead);
    let health = hs.server.players[slot].combat.health;
    for _ in 0..20 {
        send_idle(&hs, &client, slot, seq);
        seq += 1;
        host.lend_tick(&mut hs);
    }
    let now = hs.server.players[slot].player.pos;
    assert!((now.y - at.y).abs() < 0.5, "the body stays where it was put: {at} -> {now}");
    assert_eq!(hs.server.players[slot].combat.health, health, "no fall damage");
    assert_eq!(hs.server.players[slot].combat.health, hs.server.players[slot].combat.max_health);
}

/// D1 review fix 3 (LOW) — a split-screen world hosted: hosting starts the
/// server with ONE local slot (`game_loop`'s Host Game), while the save gives
/// the host client two seats, and only seat 0 sends input. On a lent world the
/// server runs power (plates) and mob spawning (anchors) on the host's world
/// from its slots, so `GameState::tick_hosted_server` hands every seat to
/// `HostedServer::sync_local_slots` each tick, which grows the local slots on
/// the first (no joiner yet) and follows every seat.
#[test]
fn a_split_screen_second_player_presses_a_plate_on_a_lent_world() {
    let (mut hs, mut host) = start_lent("split-screen");
    assert_eq!(hs.server.players.len(), 1, "hosting starts with one local slot");
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

    // Player 2 (seat 1) stands on it in the host client.
    let on_plate =
        glam::Vec3::new(plate.0 as f32 + 0.5, (plate.1 + 1) as f32, plate.2 as f32 + 0.5);
    host.lend_tick(&mut hs);
    host.lend_tick(&mut hs);
    assert!(!on(&host), "seat 1 is not on the server yet: nothing presses the plate");

    let p1 = hs.server.players[0].player.pos;
    let seats = [(p1, 0.0, 0.0, 20.0), (on_plate, 0.0, 0.0, 20.0)];
    hs.sync_local_slots(&seats);
    host.lend_tick(&mut hs);
    host.lend_tick(&mut hs);
    assert_eq!(hs.server.players.len(), 2, "a local slot grew for seat 1");
    assert_eq!(hs.server.players[1].player.pos, on_plate);
    assert!(!hs.server.players[1].server_simulated, "position-trusted, as every local slot");
    assert!(on(&host), "player 2 presses the plate on the lent world");
    assert_eq!(hs.server.players[0].player.pos, p1, "slot 0 is fed by its own input");

    // A joiner who comes after takes the slot after both local ones.
    let (_client, slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    assert_eq!(slot, 2, "remote slots follow the local ones");

    // Seat 1 leaves (the pause menu): its slot is out of the world.
    hs.sync_local_slots(&seats[..1]);
    assert!(!hs.server.players[1].is_present_and_alive());
    host.lend_tick(&mut hs);
    host.lend_tick(&mut hs);
    assert!(!on(&host), "a seat that left presses nothing");
}

/// Slots grow only while no joiner holds one (slot indices are the wire's
/// `player_index`), and `sync_local_slots` never writes slot 0 (its input
/// carries it) or a joiner's slot.
#[test]
fn sync_local_slots_never_moves_slot_zero_or_a_joiner() {
    let (mut hs, mut host) = start_lent("sync-slots");
    let (_client, slot) = join_guest_lent(&mut hs, &mut host, "Visitor");
    let p0 = hs.server.players[0].player.pos;
    let joiner = hs.server.players[slot].player.pos;
    let far = glam::Vec3::new(500.0, 90.0, 500.0);
    hs.sync_local_slots(&vec![(far, 0.0, 0.0, 20.0); slot + 1]);
    assert_eq!(hs.server.players.len(), slot + 1, "no slot grew under a seated joiner");
    assert_eq!(hs.server.players[0].player.pos, p0);
    assert_eq!(hs.server.players[slot].player.pos, joiner);
    assert!(hs.server.players[slot].is_present_and_alive(), "the joiner is untouched");
}

/// Review D2a MEDIUM-1 — a host's local slot is health-trusted: its client
/// writes the health every frame, and nothing on the server may kill its
/// copy. Its hunger is never on the wire, so a server-side metabolism starves
/// the copy within ten minutes of hosting; on Hard (starvation floor 0) the
/// next pulse against a host at 1 HP then kills it for good — nothing revives
/// a local slot's copy — and spawning and plate power stop seeing that player
/// (both anchor on `is_present_and_alive`).
#[test]
fn a_hosts_local_slot_never_dies_on_its_server_copy() {
    let (mut hs, mut host) = start_lent("local-starve");
    hs.server.difficulty = crate::survival::Difficulty::Hard;
    let p0 = hs.server.players[0].player.pos;
    // Seat 1 (split screen) at 1 HP, written through the local path.
    let seats = [(p0, 0.0, 0.0, 20.0), (p0, 0.0, 0.0, 1.0)];
    hs.sync_local_slots(&seats);
    for sp in &mut hs.server.players {
        // Ten minutes of hosting: the copies' hunger is gone.
        sp.combat.hunger = 0;
    }
    for _ in 0..2 * crate::combat::STARVATION_INTERVAL_TICKS {
        hs.sync_local_slots(&seats);
        host.lend_tick(&mut hs);
    }
    assert!(hs.server.players[1].is_present_and_alive(), "the seat's copy is alive");
    assert_eq!(hs.server.players[1].combat.health, 1.0, "its client's health, untouched");
    assert!(hs.server.players[0].is_present_and_alive());
}

//! A joiner builds the host's world from `JoinAccept` (gap-audit T2-9,
//! protocol v65).
//!
//! The joiner regenerates the host's terrain locally, so it must generate
//! with the host's seed AND world flags. Before v65 `JoinAccept` carried only
//! the seed, and the joiner never applied even that: it generated from a
//! random-seed blank meta before the accept arrived. These tests drive a REAL
//! join through `HostedServer::tick` (channel transport, no sockets) and build
//! the joiner's world the way the loading screen does — `JoinedWorld::to_meta`
//! then `World::apply_meta_rules` + the meta's seed — and compare generated
//! columns block for block with the host's.

use crate::biome::BiomeGenerator;
use crate::chunk::CHUNK_SIZE;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol;
use crate::remote_client::{build_join_request_guest, JoinedWorld, RemoteClient};
use crate::transport::ClientTransport;
use crate::world::{World, MAX_CHUNK_Y, WORLDGEN_VERSION};

/// WebSocket + 0 remote slots: no accept thread, no sockets, and the open
/// join policy a guest JoinRequest needs. The world name must not exist on
/// disk, so `initial_load` generates fresh terrain and writes nothing.
fn start_open_server(world: &str, seed: u32) -> HostedServer {
    HostedServer::start(1, world.to_string(), seed, 0, RemoteTransport::WebSocket { port: 0 })
        .expect("hosted server starts")
}

/// Join as a guest and return what the loading screen would build from.
fn join(hs: &mut HostedServer) -> JoinedWorld {
    let client = hs.attach_test_remote();
    let mut rc = RemoteClient::from_transport(
        Box::new(client),
        build_join_request_guest("Joiner", 0),
        None,
    );
    hs.tick();
    rc.poll();
    rc.pending_joined_world.take().expect("the host's JoinAccept arrived")
}

/// The joiner's world, built as `GameState::enter_joined_world` builds it.
fn joiner_world(joined: &JoinedWorld) -> (World, BiomeGenerator) {
    let meta = joined.to_meta();
    let mut w = World::new();
    w.load_bundled_plans(); // as `GameState::new` does for every client world
    w.apply_meta_rules(&meta);
    (w, BiomeGenerator::new(meta.seed))
}

fn host_generate(hs: &mut HostedServer, cx: i32, cz: i32) {
    let s = &mut hs.server;
    s.world.generate_column(cx, cz, &s.biome_gen);
}

fn assert_column_matches(host: &World, joiner: &World, cx: i32, cz: i32) {
    let cs = CHUNK_SIZE as i32;
    let mut solid = 0u32;
    for x in cx * cs..(cx + 1) * cs {
        for z in cz * cs..(cz + 1) * cs {
            for y in 0..(MAX_CHUNK_Y + 1) * cs {
                let (h, j) = (host.get_block(x, y, z), joiner.get_block(x, y, z));
                assert_eq!(h, j, "joiner's block at ({x},{y},{z}) differs from the host's");
                if h != crate::block::AIR {
                    solid += 1;
                }
            }
        }
    }
    assert!(solid > 0, "column ({cx},{cz}) generated nothing to compare");
}

#[test]
fn joiner_generates_the_hosts_normal_terrain() {
    let mut hs = start_open_server("t29-join-normal", 31_337);
    let joined = join(&mut hs);
    assert_eq!(joined.seed, 31_337, "the host's real seed");
    assert_eq!(joined.rules.world_type, "normal");
    assert_eq!(joined.worldgen_version, WORLDGEN_VERSION);

    let (mut jw, jbg) = joiner_world(&joined);
    // Columns far from both spawns, so neither side generated them yet.
    for (cx, cz) in [(37, -12), (38, -12)] {
        host_generate(&mut hs, cx, cz);
        jw.generate_column(cx, cz, &jbg);
        assert_column_matches(&hs.server.world, &jw, cx, cz);
    }
}

#[test]
fn joiner_of_a_flat_world_generates_flat_not_biome_terrain() {
    let mut hs = start_open_server("t29-join-flat", 7);
    // A flat sand world with no mobs and keep-inventory, as `initial_load`
    // would have mirrored it from the host's meta.
    let mut meta = crate::save::WorldMeta::new("t29-join-flat");
    meta.world_type = "flat".into();
    meta.ground = "sand".into();
    meta.mobs_enabled = false;
    meta.keep_inventory = true;
    meta.fire_spread_enabled = false;
    hs.server.world.apply_meta_rules(&meta);
    hs.server.fire_spread_enabled = false;

    let joined = join(&mut hs);
    assert_eq!(joined.rules, protocol::WorldRules::from_meta(&meta), "every flag reaches the joiner");

    let (mut jw, jbg) = joiner_world(&joined);
    assert!(jw.has_flat_floor() && !jw.mobs_enabled && jw.keep_inventory);
    let (cx, cz) = (25, 25);
    host_generate(&mut hs, cx, cz);
    jw.generate_column(cx, cz, &jbg);
    assert_column_matches(&hs.server.world, &jw, cx, cz);
    let floor = crate::workshop::WORKSHOP_FLOOR_Y;
    let (x, z) = (cx * CHUNK_SIZE as i32 + 3, cz * CHUNK_SIZE as i32 + 3);
    assert_eq!(jw.get_block(x, floor, z), crate::block::SAND, "a flat sand floor");
    assert_eq!(jw.get_block(x, 0, z), crate::block::AIR, "no biome terrain under it");
}

#[test]
fn joiner_of_a_workshop_world_generates_the_void_preset() {
    let mut hs = start_open_server("t29-join-void", 7);
    hs.server.world.is_workshop = true;
    let joined = join(&mut hs);
    assert!(joined.rules.is_workshop);
    let (mut jw, jbg) = joiner_world(&joined);
    let (cx, cz) = (-30, 18);
    host_generate(&mut hs, cx, cz);
    jw.generate_column(cx, cz, &jbg);
    assert_column_matches(&hs.server.world, &jw, cx, cz);
}

#[test]
fn join_accept_spawn_is_where_the_host_placed_the_joiner() {
    let mut hs = start_open_server("t29-join-spawn", 7);
    let joined = join(&mut hs);
    let slot = hs.server.players.len() - 1;
    assert_eq!(joined.spawn, Some(hs.server.players[slot].player.pos));
}

#[test]
fn host_records_a_joiners_worldgen_version() {
    let mut hs = start_open_server("t29-join-wgv", 7);
    let client = hs.attach_test_remote();
    let mut req = build_join_request_guest("Older", 0);
    req.worldgen_version = WORLDGEN_VERSION + 1;
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
    hs.tick();
    let slot = hs.server.players.len() - 1;
    assert_eq!(hs.server.players[slot].client_worldgen_version, WORLDGEN_VERSION + 1);
    assert!(hs.server.players[slot].worldgen_mismatch());
}

#[test]
fn an_older_clients_join_request_gets_the_mismatch_reason_not_silence() {
    // A v64 JoinRequest lacks the trailing `worldgen_version`, so it no
    // longer decodes; before v65 that meant the client waited forever.
    #[derive(serde::Serialize)]
    struct V64JoinRequest {
        protocol_version: u32,
        player_name: String,
        auth_event: Option<crate::signet::SignetAuthEventWire>,
        handle_credential: Option<crate::signet::SignetCredentialWire>,
        skin_key: u64,
        client_nonce_hex: String,
    }
    let mut hs = start_open_server("t29-join-v64", 7);
    let client = hs.attach_test_remote();
    let old = V64JoinRequest {
        protocol_version: 64,
        player_name: "Old".into(),
        auth_event: None,
        handle_credential: None,
        skin_key: 0,
        client_nonce_hex: String::new(),
    };
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &old));
    hs.tick();
    let mut reason = None;
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((protocol::PacketType::JoinReject, payload)) = protocol::deserialize_header(&pkt)
            && let Ok(rej) = protocol::safe_deserialize::<protocol::JoinRejectPacket>(payload)
        {
            reason = Some(rej.reason);
        }
    }
    assert_eq!(reason, Some(protocol::protocol_mismatch_reason(64)));
}

//! Late joiners see the entities already in the world (wave-hardening
//! backlog, 2026-07-12; reworked for MP-D2a).
//!
//! An entity's `EntitySpawn` used to be broadcast exactly once, globally, so
//! a client whose handshake completed later got only `EntityUpdate`s for ids
//! it had never seen — every pre-existing drop and mob invisible. A one-shot
//! backfill fixed that; since MP-D2a each client has its own interest set
//! (`entity_broadcast::ClientInterest`), which starts empty, so every entity
//! near the joiner simply enters on its first broadcast. These tests drive a
//! REAL join through `HostedServer::tick` (channel transport, no sockets or
//! accept threads) and pin the observable: the joiner gets a spawn for every
//! pre-existing entity near it, exactly once, with its full payload.

use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol;
use crate::transport::{ChannelClientTransport, ClientTransport};

/// WebSocket + 0 remote slots: no accept thread, no sockets, and the open
/// (non-sign-in) join policy a guest JoinRequest needs. The world name must
/// not exist on disk — `initial_load` then generates fresh seed-42 terrain
/// and never writes anything.
fn start_open_server(world: &str) -> HostedServer {
    HostedServer::start(
        1,
        world.to_string(),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("hosted server starts")
}

fn send_guest_join(client: &ChannelClientTransport) {
    let req = crate::remote_client::build_join_request_guest("Latecomer", 0);
    let pkt = protocol::serialize_packet(protocol::PacketType::JoinRequest, &req);
    client.send_to_server(&pkt);
}

/// Drain every packet the joiner has received and return the StateUpdates.
/// Other packet types (Challenge, JoinAccept, …) are skipped, not asserted on.
fn drain_state_updates(client: &ChannelClientTransport) -> Vec<protocol::StateUpdatePacket> {
    let mut out = Vec::new();
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
            && ptype == protocol::PacketType::StateUpdate
            && let Ok(state) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
        {
            out.push(state);
        }
    }
    out
}

fn bone_wire() -> (u8, u16) {
    crate::inventory::item_to_ref(&crate::item::Item::Material(crate::item::MaterialId::Bone))
        .to_wire()
}

#[test]
fn late_joiner_receives_spawns_for_preexisting_entities() {
    let mut hs = start_open_server("late-join-backfill-test");
    // Server-side loot + a mob, both born and broadcast BEFORE the join.
    crate::entity::spawn_item(
        &mut hs.server.ecs,
        glam::Vec3::new(8.0, 64.0, 8.0),
        crate::item::ItemStack::new_material(crate::item::MaterialId::Bone, 3),
        0,
    );
    let mob = crate::entity::spawn_mob(
        &mut hs.server.ecs,
        crate::mob::MobType::Cow,
        glam::Vec3::new(6.0, 64.0, 6.0),
    );
    hs.tick(); // first broadcast — the host has shown both already

    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();

    let mob_id = hs
        .server
        .ecs
        .get::<&crate::entity::ProtocolId>(mob)
        .expect("broadcast mob carries a ProtocolId")
        .0;
    let states = drain_state_updates(&client);
    let all_spawns: Vec<&protocol::EntitySpawn> =
        states.iter().flat_map(|s| s.entity_spawns.iter()).collect();

    // The pre-existing drop reaches the late joiner, stack payload intact,
    // exactly once (natural mob spawning can add cows, never Bone items —
    // the item assertion is unambiguous; the mob one is keyed by wire id).
    let bones: Vec<_> = all_spawns
        .iter()
        .filter(|s| {
            s.kind == protocol::EntityKind::Item
                && (s.item_kind, s.item_id) == bone_wire()
        })
        .collect();
    assert_eq!(
        bones.len(),
        1,
        "a pre-existing drop must reach the late joiner exactly once"
    );
    assert_eq!(bones[0].item_count, 3, "stack count rides the spawn");

    let mob_spawns: Vec<_> = all_spawns.iter().filter(|s| s.id == mob_id).collect();
    assert_eq!(
        mob_spawns.len(),
        1,
        "a pre-existing mob must reach the late joiner exactly once"
    );
    assert_eq!(mob_spawns[0].kind, protocol::EntityKind::Cow);
}

#[test]
fn same_tick_entities_arrive_once_and_dead_entities_are_not_backfilled() {
    let mut hs = start_open_server("late-join-no-dup-test");
    // An item that lives and dies entirely BEFORE the join (stand-in for
    // pickup/decay): it must never reach the joiner.
    crate::entity::spawn_item(
        &mut hs.server.ecs,
        glam::Vec3::new(8.0, 64.0, 8.0),
        crate::item::ItemStack::new_material(crate::item::MaterialId::RawBeef, 2),
        0,
    );
    hs.tick(); // broadcast once
    let dead: Vec<hecs::Entity> = hs
        .server
        .ecs
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for e in dead {
        hs.server.ecs.despawn(e).expect("despawn pre-join item");
    }
    hs.tick(); // despawn broadcast — the id is gone for good

    let client = hs.attach_test_remote();
    send_guest_join(&client);
    // An item born the SAME tick the join is processed: new to everyone and
    // new to the joiner's interest set alike — it must still arrive once.
    crate::entity::spawn_item(
        &mut hs.server.ecs,
        glam::Vec3::new(9.0, 64.0, 9.0),
        crate::item::ItemStack::new_material(crate::item::MaterialId::Bone, 5),
        0,
    );
    hs.tick();
    hs.tick(); // an extra tick must not re-spawn anything either

    let states = drain_state_updates(&client);
    let all_spawns: Vec<&protocol::EntitySpawn> =
        states.iter().flat_map(|s| s.entity_spawns.iter()).collect();

    let bones: Vec<_> = all_spawns
        .iter()
        .filter(|s| {
            s.kind == protocol::EntityKind::Item
                && (s.item_kind, s.item_id) == bone_wire()
        })
        .collect();
    assert_eq!(
        bones.len(),
        1,
        "a same-tick spawn must arrive exactly once"
    );
    assert_eq!(bones[0].item_count, 5);

    let beef_wire = crate::inventory::item_to_ref(&crate::item::Item::Material(
        crate::item::MaterialId::RawBeef,
    ))
    .to_wire();
    assert!(
        !all_spawns.iter().any(|s| {
            s.kind == protocol::EntityKind::Item
                && (s.item_kind, s.item_id) == beef_wire
        }),
        "an entity that died before the join must never reach the joiner"
    );
}

/// Death-drops phase 3 (2026-09-06, v61) — a late joiner must get the FULL
/// fidelity of a pre-existing tool drop, not just its bare material tier.
/// (Before MP-D2a the late-join spawn was a second encode site; it is now the
/// same spawn every client gets when the entity enters its interest.)
#[test]
fn late_joiner_backfill_carries_full_item_fidelity() {
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    let mut hs = start_open_server("late-join-full-item-test");
    let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
    pick.durability = 37;
    crate::entity::spawn_item(
        &mut hs.server.ecs,
        glam::Vec3::new(8.0, 64.0, 8.0),
        crate::item::ItemStack::new_tool(pick),
        0,
    );
    hs.tick(); // first broadcast — the host has shown the drop already

    let client = hs.attach_test_remote();
    send_guest_join(&client);
    hs.tick();

    let states = drain_state_updates(&client);
    let tools: Vec<crate::item::Item> = states
        .iter()
        .flat_map(|s| s.entity_spawns.iter())
        .filter(|s| s.kind == protocol::EntityKind::Item)
        .filter_map(|s| crate::inventory::item_from_wire_full(&s.full_item))
        .collect();
    assert_eq!(
        tools,
        vec![crate::item::Item::Tool(pick)],
        "the late joiner's tool spawn carries type + material + durability, once"
    );
}

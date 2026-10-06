//! MP-A3 (2026-10-06) — projectiles on a dedicated server are real.
//!
//! The dedicated server has ticked dispensers since T1-3, but it never ran the
//! projectile sim and `diff_entities` never broadcast projectiles: a
//! server-shot arrow was consumed from the dispenser, never seen by a joiner,
//! never hit anything, and quietly expired. These tests drive a REAL
//! 0-local-player `HostedServer` (what `server_main` runs) with a guest joined
//! over the in-process transport, and read the joiner's actual `StateUpdate`s.

use glam::Vec3;

use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item::{ItemStack, MaterialId};
use crate::meta::Facing;
use crate::protocol::{self, EntityKind};
use crate::test_harness::{TestConfig, TestHost};
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;

/// A 0-local-player server — what `server_main` runs.
fn start_dedicated_server(tag: &str) -> HostedServer {
    HostedServer::start(
        0,
        format!("server-projectiles-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts")
}

/// Every entity event the joiner has been sent, in order.
#[derive(Default)]
struct Seen {
    spawns: Vec<protocol::EntitySpawn>,
    updates: Vec<protocol::EntityUpdate>,
    despawns: Vec<u32>,
}

fn drain_entity_events(client: &ChannelClientTransport, seen: &mut Seen) {
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
            && ptype == protocol::PacketType::StateUpdate
            && let Ok(s) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
        {
            seen.spawns.extend(s.entity_spawns);
            seen.updates.extend(s.entity_updates);
            seen.despawns.extend(s.entity_despawns);
        }
    }
}

const Y: i32 = 85;
/// The dispenser, high in clear air so nothing but the target is in the way.
const DISPENSER: (i32, i32, i32) = (10, Y, 10);

/// A Dispenser facing East loaded with `arrows` arrows, powered by an active
/// lever on its south face, with the air ahead of it cleared. The rising edge
/// fires one arrow on the server's next 4-tick machine pass.
fn arrow_rig(hs: &mut HostedServer, arrows: u8) {
    let world = &mut hs.server.world;
    for x in DISPENSER.0 - 1..=DISPENSER.0 + 40 {
        for y in Y - 6..=Y + 2 {
            for z in DISPENSER.2 - 2..=DISPENSER.2 + 2 {
                world.set_block(x, y, z, block::AIR);
            }
        }
    }
    world.set_block(DISPENSER.0, DISPENSER.1, DISPENSER.2, block::DISPENSER);
    world.set_meta(DISPENSER, crate::meta::with_facing(0, Facing::East));
    let mut d = crate::dispenser::DispenserData::new();
    d.chest.slots[0] = Some(ItemStack::new_material(MaterialId::Arrow, arrows));
    world.insert_dispenser(DISPENSER, d);
    let lever = (DISPENSER.0, DISPENSER.1, DISPENSER.2 + 1);
    world.set_block(lever.0, lever.1, lever.2, block::LEVER);
    let mut dev = crate::power::PowerDeviceData::new(crate::power::PowerDeviceKind::Lever, Facing::Up);
    dev.on = true;
    world.insert_power_device(lever, dev);
}

fn arrows_left(hs: &HostedServer) -> u8 {
    hs.server
        .world
        .dispenser_at(DISPENSER)
        .and_then(|d| d.chest.slots[0].as_ref().map(|s| s.count))
        .unwrap_or(0)
}

fn projectiles_on_server(hs: &HostedServer) -> usize {
    hs.server.ecs.query::<&crate::entity::ProjectileEntity>().iter().count()
}

/// The one projectile spawn the joiner saw; panics on zero or several.
fn the_projectile(seen: &Seen) -> protocol::EntitySpawn {
    let p: Vec<_> = seen.spawns.iter().filter(|s| s.kind == EntityKind::Projectile).collect();
    assert_eq!(p.len(), 1, "exactly one projectile spawn reached the joiner");
    p[0].clone()
}

#[test]
fn a_dedicated_servers_dispenser_arrow_reaches_the_joiner_flies_and_hits_a_mob() {
    let mut hs = start_dedicated_server("hit");
    assert!(hs.server.simulates_block_machines);
    let (client, _slot) = join_guest(&mut hs, "Archer");
    arrow_rig(&mut hs, 3);
    // A target eight blocks down-range, in the arrow's flight band. Bare
    // components: no AI or physics to move it out of the way.
    let target = hs.server.ecs.spawn((
        crate::entity::MobKind(crate::mob::MobType::Cow),
        crate::entity::Position(Vec3::new(18.5, Y as f32 - 1.0, 10.5)),
        crate::entity::Hitbox { width: 0.9, height: 1.4 },
        crate::combat::Health::new(100.0),
    ));
    let mut seen = Seen::default();
    drain_entity_events(&client, &mut seen);
    seen = Seen::default();

    for _ in 0..40 {
        hs.tick();
        drain_entity_events(&client, &mut seen);
    }

    assert_eq!(arrows_left(&hs), 2, "the rising edge fired exactly one arrow");
    let spawn = the_projectile(&seen);
    assert!(
        (spawn.x - (DISPENSER.0 as f32 + 0.5)).abs() < 2.0 && (spawn.z - 10.5).abs() < 1e-3,
        "the arrow appears at the dispenser's mouth, not somewhere else: {spawn:?}"
    );
    let xs: Vec<f32> = seen.updates.iter().filter(|u| u.id == spawn.id).map(|u| u.x).collect();
    assert!(xs.len() >= 2, "the joiner sees the arrow in flight, not just its birth: {xs:?}");
    assert!(xs.windows(2).all(|w| w[1] > w[0]), "it flies east, tick by tick: {xs:?}");
    assert_eq!(
        seen.despawns.iter().filter(|&&id| id == spawn.id).count(),
        1,
        "the hit despawns it on the joiner exactly once"
    );
    let hp = hs.server.ecs.get::<&crate::combat::Health>(target).unwrap().current;
    assert!(
        (hp - (100.0 - crate::entity::ARROW_DAMAGE)).abs() < 1e-3,
        "the arrow lands on the mob through the server's damage path (hp {hp})"
    );
    assert_eq!(projectiles_on_server(&hs), 0, "the arrow is gone from the server sim");
}

#[test]
fn a_dedicated_servers_arrow_with_no_target_comes_down_and_despawns_once() {
    let mut hs = start_dedicated_server("miss");
    let (client, _slot) = join_guest(&mut hs, "Watcher");
    arrow_rig(&mut hs, 1);
    let mut seen = Seen::default();
    drain_entity_events(&client, &mut seen);
    seen = Seen::default();

    // Long enough for any end: the terrain below, or the arrow's own lifetime.
    for _ in 0..(crate::entity::ARROW_LIFETIME_TICKS + 20) {
        hs.tick();
        drain_entity_events(&client, &mut seen);
    }

    assert_eq!(arrows_left(&hs), 0);
    let spawn = the_projectile(&seen);
    let ys: Vec<f32> = seen.updates.iter().filter(|u| u.id == spawn.id).map(|u| u.y).collect();
    assert!(ys.len() >= 2, "the joiner sees it fly");
    assert!(ys.first() > ys.last(), "gravity brings it down: {ys:?}");
    assert_eq!(
        seen.despawns.iter().filter(|&&id| id == spawn.id).count(),
        1,
        "despawned exactly once"
    );
    assert_eq!(projectiles_on_server(&hs), 0);
}

#[test]
fn only_a_machine_ticking_server_runs_the_projectile_sim() {
    // A LAN host's CLIENT owns its projectiles (and its dispensers); the
    // server side moving the same arrow would be a second sim.
    for (machines, should_move) in [(true, true), (false, false)] {
        let mut h = TestHost::start_with(TestConfig::default());
        h.server.simulates_block_machines = machines;
        let start = Vec3::new(0.5, 90.0, 0.5);
        crate::entity::spawn_arrow(&mut h.server.ecs, start, Vec3::new(0.5, 0.0, 0.0), 4.0, None);
        h.tick(1);
        let pos = h
            .server
            .ecs
            .query::<(&crate::entity::Position, &crate::entity::ProjectileEntity)>()
            .iter()
            .map(|(_, (p, _))| p.0)
            .next()
            .expect("still in flight after one tick");
        assert_eq!(pos != start, should_move, "machines={machines}: arrow at {pos:?}");
    }
}

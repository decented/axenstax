//! MP-A3 (2026-10-06) — a dead joiner stays dead on the server until they
//! choose Respawn.
//!
//! The server copy of a remote player used to revive itself 40 ticks after it
//! died (a BRIDGE: the server couldn't see the client's death-screen choice),
//! so while the joiner was still reading "You died" their server copy was
//! back on its feet — walking on queued input and vacuuming up the very items
//! the death had scattered. Now death is a state the server holds: no physics,
//! no pickups, no edits, invisible to mobs, until the joiner's client sends an
//! explicit `Respawn`. Every test drives a REAL 0-local-player `HostedServer`
//! over the in-process transport and reads what the joiner is actually sent.

use glam::Vec3;

use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item::{Item, ItemStack, MaterialId};
use crate::protocol::{self, PlayerEventType};
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;

fn start_dedicated_server(tag: &str) -> HostedServer {
    HostedServer::start(
        0,
        format!("joiner-death-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts")
}

/// What the joiner has been sent that these tests care about.
#[derive(Default)]
struct Inbox {
    /// `(player_index, event)` for every PlayerEvent.
    events: Vec<(u32, PlayerEventType)>,
    grants: usize,
    block_changes: Vec<protocol::BlockChange>,
}

impl Inbox {
    fn drain(&mut self, client: &ChannelClientTransport) {
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::PlayerEvent => {
                    if let Ok(e) = protocol::safe_deserialize::<protocol::PlayerEventPacket>(payload) {
                        self.events.push((e.player_index, e.event));
                    }
                }
                protocol::PacketType::InventoryGrant => self.grants += 1,
                protocol::PacketType::StateUpdate => {
                    if let Ok(s) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload) {
                        self.block_changes.extend(s.block_changes);
                    }
                }
                _ => {}
            }
        }
    }

    fn died(&self, slot: usize) -> usize {
        self.events
            .iter()
            .filter(|(i, e)| *i as usize == slot && matches!(e, PlayerEventType::Died))
            .count()
    }

    fn respawned(&self, slot: usize) -> Vec<Vec3> {
        self.events
            .iter()
            .filter(|(i, _)| *i as usize == slot)
            .filter_map(|(_, e)| match e {
                PlayerEventType::Respawned { x, y, z } => Some(Vec3::new(*x, *y, *z)),
                _ => None,
            })
            .collect()
    }
}

fn tick_n(hs: &mut HostedServer, client: &ChannelClientTransport, inbox: &mut Inbox, n: u32) {
    for _ in 0..n {
        hs.tick();
        inbox.drain(client);
    }
}

/// Send one ClientInput from the joiner.
fn send_input(
    client: &ChannelClientTransport,
    tick: u64,
    at: Vec3,
    health: f32,
    move_forward: f32,
    edits: &[((i32, i32, i32), block::BlockId)],
) {
    let mut input = protocol::InputPacket {
        tick,
        x: at.x,
        y: at.y,
        z: at.z,
        health,
        move_forward,
        ..Default::default()
    };
    for &((x, y, z), b) in edits {
        input.block_changes.push(protocol::BlockChange { x, y, z, new_block: b, meta: 0 });
    }
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
}

fn send_respawn(client: &ChannelClientTransport) {
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Respawn, &()));
}

/// Stand the joiner's server copy on a fresh stone floor far from anything,
/// so items dropped at its feet settle within pickup reach. Returns where it
/// stands.
fn stand_on_floor(hs: &mut HostedServer, slot: usize) -> Vec3 {
    let (fx, fy, fz) = (40, 80, 40);
    let world = &mut hs.server.world;
    for x in fx - 4..=fx + 4 {
        for z in fz - 4..=fz + 4 {
            world.set_block(x, fy - 1, z, block::STONE);
            for y in fy..=fy + 4 {
                world.set_block(x, y, z, block::AIR);
            }
        }
    }
    let at = Vec3::new(fx as f32 + 0.5, fy as f32, fz as f32 + 0.5);
    let p = &mut hs.server.players[slot].player;
    p.pos = at;
    p.velocity = Vec3::ZERO;
    p.reset_fall();
    at
}

fn scatter_bones_at(hs: &mut HostedServer, at: Vec3, stacks: u32) {
    for k in 0..stacks {
        crate::entity::spawn_item(
            &mut hs.server.ecs,
            at,
            ItemStack::new_material(MaterialId::Bone, 2),
            k.wrapping_mul(0x9E37_79B9),
        );
    }
}

fn bones_on_ground(hs: &HostedServer) -> u32 {
    hs.server
        .ecs
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .filter(|(_, it)| it.stack.item == Item::Material(MaterialId::Bone))
        .map(|(_, it)| u32::from(it.stack.count))
        .sum()
}

#[test]
fn an_alive_joiner_picks_up_items_at_its_feet() {
    // The control for the next test: the rig really is within pickup reach.
    let mut hs = start_dedicated_server("alive-pickup");
    let (client, slot) = join_guest(&mut hs, "Alive");
    let mut inbox = Inbox::default();
    let at = stand_on_floor(&mut hs, slot);
    scatter_bones_at(&mut hs, at, 3);
    tick_n(&mut hs, &client, &mut inbox, 200);
    assert_eq!(bones_on_ground(&hs), 0, "a living joiner collects what lies at its feet");
    assert!(inbox.grants > 0, "and is granted it");
}

#[test]
fn a_dead_joiner_never_vacuums_up_its_scattered_items() {
    let mut hs = start_dedicated_server("dead-pickup");
    let (client, slot) = join_guest(&mut hs, "Fallen");
    let mut inbox = Inbox::default();
    let at = stand_on_floor(&mut hs, slot);
    // Killed on the server (a fall, drowning…): the server copy's own death.
    assert!(hs.server.players[slot].combat.take_damage(1000.0));
    scatter_bones_at(&mut hs, at, 3);

    tick_n(&mut hs, &client, &mut inbox, 200);

    assert!(hs.server.players[slot].combat.dead, "no revive on a timer");
    assert_eq!(bones_on_ground(&hs), 6, "the death drops stay on the ground for others");
    assert_eq!(inbox.grants, 0, "the dead joiner is granted nothing");
    assert_eq!(inbox.died(slot), 1, "the joiner is told it died — once");
    assert!(inbox.respawned(slot).is_empty(), "nobody asked to respawn");
}

#[test]
fn a_joiner_reporting_zero_health_is_dead_on_the_server_and_its_input_is_ignored() {
    let mut hs = start_dedicated_server("reported");
    let (client, slot) = join_guest(&mut hs, "Reporter");
    let mut inbox = Inbox::default();
    let at = stand_on_floor(&mut hs, slot);
    let cell = (at.x.floor() as i32 + 1, at.y as i32, at.z.floor() as i32);
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR);

    // Killed in the joiner's own sim (a mob it runs): its input says so.
    send_input(&client, 1, at, 0.0, 0.0, &[]);
    tick_n(&mut hs, &client, &mut inbox, 1);
    assert!(hs.server.players[slot].combat.dead, "a reported death is taken");
    assert_eq!(hs.server.players[slot].combat.health, 0.0);

    // While dead: walking and building go nowhere.
    for t in 2..40 {
        let edits: &[((i32, i32, i32), block::BlockId)] =
            if t == 2 { &[(cell, block::STONE)] } else { &[] };
        send_input(&client, t, at, 0.0, 1.0, edits);
        tick_n(&mut hs, &client, &mut inbox, 1);
    }
    assert_eq!(hs.server.players[slot].player.pos, at, "a dead body doesn't walk");
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR, "nor build");
    assert!(
        inbox
            .block_changes
            .iter()
            .any(|bc| (bc.x, bc.y, bc.z) == cell && bc.new_block == block::AIR),
        "the refused edit is sent back so the joiner's ghost block un-places"
    );

    // A reported recovery is NOT a respawn: health never comes back on the
    // client's say-so.
    send_input(&client, 50, at, 20.0, 0.0, &[]);
    tick_n(&mut hs, &client, &mut inbox, 5);
    assert!(hs.server.players[slot].combat.dead);
}

#[test]
fn a_dead_joiner_respawns_only_on_request_at_its_spawn_point() {
    let mut hs = start_dedicated_server("respawn");
    let (client, slot) = join_guest(&mut hs, "Phoenix");
    let mut inbox = Inbox::default();
    let spawn = hs.server.players[slot].spawn_pos;
    stand_on_floor(&mut hs, slot);
    send_input(&client, 1, Vec3::ZERO, 0.0, 0.0, &[]);
    tick_n(&mut hs, &client, &mut inbox, 200);
    assert!(hs.server.players[slot].combat.dead, "still dead after 200 ticks");
    assert!(inbox.respawned(slot).is_empty());

    send_respawn(&client);
    tick_n(&mut hs, &client, &mut inbox, 1);

    let sp = &hs.server.players[slot];
    assert!(!sp.combat.dead, "the request respawns them");
    assert_eq!(sp.combat.health, sp.combat.max_health);
    let at = sp.player.pos;
    assert_eq!((at.x, at.z), (spawn.x, spawn.z), "at their spawn point");
    let (bx, by, bz) = (at.x.floor() as i32, at.y as i32, at.z.floor() as i32);
    assert_eq!(hs.server.world.get_block(bx, by, bz), block::AIR, "feet in the open");
    assert_ne!(
        hs.server.world.get_block(bx, by - 1, bz),
        block::AIR,
        "standing on the ground, not dropped from the sky ({at:?})"
    );
    assert_eq!(inbox.respawned(slot), vec![at], "and the joiner is told where");
}

#[test]
fn a_respawn_request_from_a_living_joiner_is_ignored() {
    // Otherwise Respawn would be a free teleport home.
    let mut hs = start_dedicated_server("alive-respawn");
    let (client, slot) = join_guest(&mut hs, "Tourist");
    let mut inbox = Inbox::default();
    let at = stand_on_floor(&mut hs, slot);
    send_respawn(&client);
    tick_n(&mut hs, &client, &mut inbox, 1);
    assert_eq!(hs.server.players[slot].player.pos, at);
    assert!(inbox.respawned(slot).is_empty());
}

#[test]
fn mobs_do_not_target_a_dead_joiner() {
    let mut hs = start_dedicated_server("mobs");
    let (client, slot) = join_guest(&mut hs, "Bait");
    let mut inbox = Inbox::default();
    let at = stand_on_floor(&mut hs, slot);
    let mob = crate::entity::spawn_mob(
        &mut hs.server.ecs,
        crate::mob::MobType::Brigand,
        at + Vec3::new(3.0, 0.0, 0.0),
    );
    let chasing = |hs: &mut HostedServer, client: &ChannelClientTransport, inbox: &mut Inbox| {
        hs.server.ecs.get::<&mut crate::mob_ai::MobAi>(mob).unwrap().state =
            crate::mob_ai::AiState::Idle { timer: 0 };
        tick_n(hs, client, inbox, 1);
        matches!(
            hs.server.ecs.get::<&crate::mob_ai::MobAi>(mob).unwrap().state,
            crate::mob_ai::AiState::Chase
        )
    };
    assert!(chasing(&mut hs, &client, &mut inbox), "control: a brigand chases a living joiner");

    send_input(&client, 1, at, 0.0, 0.0, &[]);
    assert!(!chasing(&mut hs, &client, &mut inbox), "a dead joiner is not a target");
    assert!(hs.server.players[slot].combat.dead);
}

#[test]
fn a_joiner_who_disconnects_while_dead_is_dropped_not_revived() {
    let mut hs = start_dedicated_server("dead-leave");
    let (client, slot) = join_guest(&mut hs, "Ghost");
    let mut inbox = Inbox::default();
    stand_on_floor(&mut hs, slot);
    send_input(&client, 1, Vec3::ZERO, 0.0, 0.0, &[]);
    tick_n(&mut hs, &client, &mut inbox, 1);
    assert!(hs.server.players[slot].combat.dead);

    drop(client);
    for _ in 0..200 {
        hs.tick();
    }
    assert!(hs.slot_is_free(slot), "the slot is dropped as usual");
    let sp = &hs.server.players[slot];
    assert!(!sp.connected && sp.combat.dead, "and nothing revives the body");
}

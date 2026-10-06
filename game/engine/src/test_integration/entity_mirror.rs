//! MP-D2a (2026-10-07) — joiners see the server's mobs and are hurt by them.
//!
//! Every test drives a REAL 0-local-player `HostedServer` (a dedicated
//! server: it owns its world and runs its own mob AI) over the in-process
//! transport, and reads what the joiner is actually sent: the per-client,
//! changed-only entity diff that feeds a joiner's `RemoteMobs` mirror, and
//! the joiner's own `PlayerState.health`, which is now the server's (mob
//! melee and lava/fire contact land server-side on the body it simulates).

use glam::Vec3;

use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::mob::MobType;
use crate::protocol::{self, entity_flags, EntityKind};
use crate::remote_mobs::RemoteMobs;
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;

fn start_dedicated_server(tag: &str) -> HostedServer {
    HostedServer::start(
        0,
        format!("entity-mirror-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts")
}

/// Everything entity-shaped and own-body-shaped the joiner was sent.
#[derive(Default)]
struct Inbox {
    spawns: Vec<protocol::EntitySpawn>,
    updates: Vec<protocol::EntityUpdate>,
    despawns: Vec<u32>,
    /// `(last_acked_input, own health)` per StateUpdate, in order.
    own: Vec<(u64, f32)>,
    died: usize,
}

impl Inbox {
    fn drain(&mut self, client: &ChannelClientTransport, slot: usize, mirror: &mut RemoteMobs) {
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::StateUpdate => {
                    let Ok(s) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
                    else {
                        continue;
                    };
                    // The joiner's own fold, exactly as `network_receive` does it.
                    mirror.apply(&s.entity_spawns, &s.entity_updates, &s.entity_despawns);
                    if let Some(me) = s.players.iter().find(|p| p.player_index as usize == slot) {
                        self.own.push((s.last_acked_input, me.health));
                    }
                    self.spawns.extend(s.entity_spawns);
                    self.updates.extend(s.entity_updates);
                    self.despawns.extend(s.entity_despawns);
                }
                protocol::PacketType::PlayerEvent => {
                    if let Ok(e) = protocol::safe_deserialize::<protocol::PlayerEventPacket>(payload)
                        && e.player_index as usize == slot
                        && matches!(e.event, protocol::PlayerEventType::Died)
                    {
                        self.died += 1;
                    }
                }
                _ => {}
            }
        }
    }

    fn spawns_of(&self, id: u32) -> usize {
        self.spawns.iter().filter(|s| s.id == id).count()
    }

    fn updates_of(&self, id: u32) -> usize {
        self.updates.iter().filter(|u| u.id == id).count()
    }

    fn last_health(&self) -> f32 {
        self.own.last().expect("the joiner got a StateUpdate").1
    }
}

struct Rig {
    hs: HostedServer,
    client: ChannelClientTransport,
    slot: usize,
    inbox: Inbox,
    mirror: RemoteMobs,
    at: Vec3,
}

impl Rig {
    /// A dedicated server with one guest joiner standing on a fresh stone
    /// floor at (40, 80, 40), its mobs cleared.
    fn new(tag: &str) -> Self {
        let mut hs = start_dedicated_server(tag);
        let (client, slot) = join_guest(&mut hs, "Mirror");
        let (fx, fy, fz) = (40, 80, 40);
        for x in fx - 6..=fx + 6 {
            for z in fz - 6..=fz + 6 {
                hs.server.world.set_block(x, fy - 1, z, block::STONE);
                for y in fy..=fy + 4 {
                    hs.server.world.set_block(x, y, z, block::AIR);
                }
            }
        }
        let at = Vec3::new(fx as f32 + 0.5, fy as f32, fz as f32 + 0.5);
        let p = &mut hs.server.players[slot].player;
        p.pos = at;
        p.velocity = Vec3::ZERO;
        p.reset_fall();
        // No streaming (and so no wildlife scatter) round the joiner: the
        // only mobs are the ones a test spawns.
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        crate::remote_mobs::purge_private_mobs(&mut hs.server.ecs);
        let mut rig = Rig { hs, client, slot, inbox: Inbox::default(), mirror: RemoteMobs::default(), at };
        rig.tick(1);
        rig
    }

    fn tick(&mut self, n: u32) {
        for _ in 0..n {
            self.hs.tick();
            self.inbox.drain(&self.client, self.slot, &mut self.mirror);
        }
    }

    fn spawn(&mut self, kind: MobType, at: Vec3) -> hecs::Entity {
        crate::entity::spawn_mob(&mut self.hs.server.ecs, kind, at)
    }

    /// The mob's wire id (assigned on its first broadcast).
    fn wire_id(&self, e: hecs::Entity) -> u32 {
        self.hs.server.ecs.get::<&crate::entity::ProtocolId>(e).expect("broadcast").0
    }

    fn server_health(&self) -> f32 {
        self.hs.server.players[self.slot].combat.health
    }

    fn send_input(&self, seq: u64, health_delta: f32, armour_points: u8) {
        let input = protocol::InputPacket {
            tick: seq,
            x: self.at.x,
            y: self.at.y,
            z: self.at.z,
            health: self.server_health(),
            health_delta,
            armour_points,
            ..Default::default()
        };
        self.client
            .send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }
}

#[test]
fn a_joiner_sees_a_server_mob_spawn_move_and_despawn() {
    let mut rig = Rig::new("lifecycle");
    let calf = rig.spawn(MobType::Cow, rig.at + Vec3::new(4.0, 0.0, 0.0));
    rig.hs
        .server
        .ecs
        .insert_one(calf, crate::breeding::Baby { adult_at_tick: u64::MAX })
        .unwrap();
    rig.tick(1);
    let id = rig.wire_id(calf);

    assert_eq!(rig.inbox.spawns_of(id), 1, "the joiner is told about the mob once");
    let s = rig.inbox.spawns.iter().find(|s| s.id == id).unwrap();
    assert_eq!(s.kind, EntityKind::Cow, "with its species");
    let u = rig.inbox.updates.iter().find(|u| u.id == id).unwrap();
    assert_eq!(u.flags & entity_flags::BABY, entity_flags::BABY, "and its baby flag");
    assert_eq!(rig.mirror.len(), 1, "the joiner's mirror holds it");

    // The server moves it: the joiner's mirror follows.
    rig.hs.server.ecs.get::<&mut crate::entity::Position>(calf).unwrap().0.x += 2.0;
    rig.tick(1);
    rig.mirror.advance(1.0);
    let (drawn, kind) = rig.mirror.drawn(id).expect("still mirrored");
    assert_eq!(kind, Some(MobType::Cow));
    let server_x = rig.hs.server.ecs.get::<&crate::entity::Position>(calf).unwrap().0.x;
    assert!((drawn.x - server_x).abs() < 0.5, "drawn where the server has it: {drawn:?}");

    rig.hs.server.ecs.despawn(calf).unwrap();
    rig.tick(1);
    assert_eq!(rig.inbox.despawns.iter().filter(|&&d| d == id).count(), 1);
    assert!(rig.mirror.is_empty(), "gone from the mirror");
}

#[test]
fn entities_out_of_range_are_never_sent() {
    let mut rig = Rig::new("range");
    let near = rig.spawn(MobType::Pig, rig.at + Vec3::new(10.0, 0.0, 0.0));
    let far = rig.spawn(
        MobType::Pig,
        rig.at + Vec3::new(crate::entity_broadcast::INTEREST_LEAVE_RADIUS + 50.0, 0.0, 0.0),
    );
    rig.tick(3);
    assert_eq!(rig.inbox.spawns_of(rig.wire_id(near)), 1);
    let far_id = rig.wire_id(far);
    assert_eq!(rig.inbox.spawns_of(far_id), 0, "an out-of-range mob's spawn is never sent");
    assert_eq!(rig.inbox.updates_of(far_id), 0, "nor any update");
}

#[test]
fn a_resting_mob_sends_no_updates_until_it_changes() {
    let mut rig = Rig::new("changed-only");
    let cow = rig.spawn(MobType::Cow, rig.at + Vec3::new(3.0, 0.0, 3.0));
    // Let it land and settle (spawned exactly on the floor: little to do).
    rig.tick(3);
    let id = rig.wire_id(cow);
    // Hold it still: idle, no velocity.
    {
        let ecs = &mut rig.hs.server.ecs;
        ecs.get::<&mut crate::mob_ai::MobAi>(cow).unwrap().state =
            crate::mob_ai::AiState::Idle { timer: 1000 };
        ecs.get::<&mut crate::entity::Velocity>(cow).unwrap().0 = Vec3::ZERO;
    }
    rig.tick(2);
    let before = rig.inbox.updates_of(id);
    rig.tick(10);
    assert_eq!(rig.inbox.updates_of(id), before, "an unchanged mob costs no bandwidth");

    rig.hs.server.ecs.get::<&mut crate::combat::Health>(cow).unwrap().take_damage(1.0);
    rig.tick(1);
    let hurt = rig.inbox.updates.iter().rev().find(|u| u.id == id).unwrap();
    assert_eq!(rig.inbox.updates_of(id), before + 1, "a change sends one update");
    assert_ne!(hurt.flags & entity_flags::HURT, 0, "carrying the hurt flash");
}

#[test]
fn a_hostile_next_to_a_joiner_hurts_it_server_side_and_its_health_follows() {
    let mut rig = Rig::new("melee");
    assert_eq!(rig.server_health(), 20.0);
    let brigand = rig.spawn(MobType::Brigand, rig.at + Vec3::new(0.6, 0.0, 0.0));
    rig.tick(1);

    let hp = rig.server_health();
    assert!(hp < 20.0, "the brigand's melee landed on the server's body (health {hp})");
    assert_eq!(
        rig.hs.server.players[rig.slot].combat.last_damage,
        crate::survival::DamageCause::Mob(MobType::Brigand)
    );
    assert_eq!(rig.inbox.last_health(), hp, "the joiner is sent the server's health");

    // The joiner shows it: the server's value wins (`health_sync`).
    let mut own = crate::health_sync::OwnHealth::new();
    own.sent(0, 0.0, 20.0, false);
    let (acked, server_hp) = *rig.inbox.own.last().unwrap();
    let shown = own.apply_server(server_hp, acked, 20.0, 20.0, false);
    assert_eq!(shown.health, hp);
    assert!(shown.hurt, "and flashes the hit");

    // The armour the joiner reports soaks the next hit.
    rig.send_input(1, 0.0, 20);
    rig.tick(1);
    assert_eq!(rig.hs.server.players[rig.slot].armour_points, 20);
    {
        let sp = &mut rig.hs.server.players[rig.slot];
        sp.combat.health = 20.0;
        sp.combat.invincible_timer = 0;
        sp.player.pos = rig.at;
        sp.player.velocity = Vec3::ZERO;
    }
    rig.hs.server.ecs.get::<&mut crate::entity::Position>(brigand).unwrap().0 =
        rig.at + Vec3::new(0.6, 0.0, 0.0);
    rig.tick(1);
    let raw = crate::survival::scale_mob_damage(
        crate::combat::HOSTILE_MELEE_DAMAGE,
        rig.hs.server.difficulty,
    );
    let soaked = crate::armour::damage_after_armour(raw, 20);
    assert!(soaked < raw);
    assert!(
        (rig.server_health() - (20.0 - soaked)).abs() < 1e-4,
        "20 armour points soak the hit: {} != {}",
        rig.server_health(),
        20.0 - soaked
    );
}

#[test]
fn lava_contact_hurts_a_joiner_server_side() {
    let mut rig = Rig::new("lava");
    let (x, y, z) = (rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32);
    rig.hs.server.world.set_block(x, y, z, block::LAVA);
    rig.tick(crate::survival::CONTACT_HAZARD_PERIOD_TICKS as u32 + 1);
    let hp = rig.server_health();
    assert!(hp < 20.0, "standing in lava burns (health {hp})");
    assert_eq!(rig.hs.server.players[rig.slot].combat.last_damage, crate::survival::DamageCause::Lava);
    assert_eq!(rig.inbox.last_health(), hp);
}

#[test]
fn a_lethal_hit_on_the_server_kills_the_joiner_through_the_died_event() {
    let mut rig = Rig::new("lethal");
    rig.hs.server.players[rig.slot].combat.health = 1.0;
    rig.spawn(MobType::Berserker, rig.at + Vec3::new(0.5, 0.0, 0.0));
    rig.tick(2);
    assert!(rig.hs.server.players[rig.slot].combat.dead);
    assert_eq!(rig.inbox.died, 1, "the joiner is told it died, once");
}

#[test]
fn the_joiners_own_heal_is_applied_with_the_input_it_rode() {
    let mut rig = Rig::new("heal");
    // Nothing but the report may move the health here.
    rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    rig.hs.server.players[rig.slot].combat.health = 12.0;
    rig.send_input(7, 3.0, 0);
    rig.tick(1);
    assert_eq!(rig.server_health(), 15.0, "the reported heal lands on the server's copy");
    let (acked, hp) = *rig.inbox.own.last().unwrap();
    assert_eq!((acked, hp), (7, 15.0), "acknowledged together with its input");

    // The server runs no metabolism of its own for a joiner: no regen.
    rig.tick(200);
    assert_eq!(rig.server_health(), 15.0, "a joiner's regen is its client's to report");
}

#[test]
fn peaceful_hostiles_do_not_bite_and_creative_takes_no_contact_damage() {
    let mut rig = Rig::new("peaceful");
    rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    rig.spawn(MobType::Brigand, rig.at + Vec3::new(0.6, 0.0, 0.0));
    rig.tick(3);
    assert_eq!(rig.server_health(), 20.0, "no hostile attacks on Peaceful");

    let mut rig = Rig::new("creative");
    rig.hs.server.play_mode = crate::play_mode::PlayMode::Creative;
    let (x, y, z) = (rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32);
    rig.hs.server.world.set_block(x, y, z, block::LAVA);
    rig.tick(25);
    assert_eq!(rig.server_health(), 20.0, "creative is immune");
}

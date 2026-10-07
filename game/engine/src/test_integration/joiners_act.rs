//! MP-D2b (2026-10-07) — joiners act on the server's mobs: a swing, the
//! one-shot right-click interactions, kill credit; and the D2a gaps that
//! close with them (server-landed hits wear a joiner's armour, a server-side
//! death names its cause, species attacks reach joiners).
//!
//! Every test drives a REAL `HostedServer` over the in-process transport —
//! a dedicated server (0 local slots: it owns its world and runs its own
//! death sweep), or a lending host (`sim_lend::OwnedSimParts` standing in
//! for the host client, whose death sweep is the host client's) — and reads
//! what each joiner is actually sent.

use glam::Vec3;

use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item::{Item, MaterialId};
use crate::mob::MobType;
use crate::mob_interact::InteractNote;
use crate::protocol::{self, InteractKind, PlayerEventType, WireDamageCause};
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;

/// A valid x-only public key (secp256k1's generator point), so the joiner's
/// pet-owner key is a real bech32 npub.
const NPUB_KEY: [u8; 32] = [
    0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95, 0xCE, 0x87, 0x0B, 0x07,
    0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59, 0xF2, 0x81, 0x5B, 0x16, 0xF8, 0x17, 0x98,
];

/// What one joiner was sent that these tests read.
#[derive(Default)]
struct Inbox {
    outcomes: Vec<protocol::InteractOutcomePacket>,
    kills: Vec<protocol::KillEventPacket>,
    grants: Vec<protocol::InventoryGrantPacket>,
    /// Its own `PlayerEvent`s.
    events: Vec<PlayerEventType>,
}

impl Inbox {
    fn drain(&mut self, client: &ChannelClientTransport, slot: usize) {
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::InteractOutcome => {
                    self.outcomes.push(protocol::safe_deserialize(payload).unwrap());
                }
                protocol::PacketType::KillEvent => {
                    self.kills.push(protocol::safe_deserialize(payload).unwrap());
                }
                protocol::PacketType::InventoryGrant => {
                    self.grants.push(protocol::safe_deserialize(payload).unwrap());
                }
                protocol::PacketType::PlayerEvent => {
                    let e: protocol::PlayerEventPacket = protocol::safe_deserialize(payload).unwrap();
                    if e.player_index as usize == slot {
                        self.events.push(e.event);
                    }
                }
                _ => {}
            }
        }
    }

    fn outcome(&self, seq: u32) -> &protocol::InteractOutcomePacket {
        self.outcomes.iter().find(|o| o.seq == seq).expect("the request was answered")
    }

    fn granted(&self, m: MaterialId) -> u32 {
        let (kind, id) = crate::inventory::item_to_ref(&Item::Material(m)).to_wire();
        self.grants
            .iter()
            .filter(|g| g.item_kind == kind && g.item_id == id)
            .map(|g| g.count as u32)
            .sum()
    }
}

fn sword() -> Item {
    Item::Tool(Tool::new(ToolType::Sword, ToolMaterial::Iron))
}

fn mat(m: MaterialId) -> Item {
    Item::Material(m)
}

fn held_wire(held: Option<&Item>) -> (u8, u16, protocol::WireItem) {
    match held {
        Some(item) => {
            let (k, i) = crate::inventory::item_to_ref(item).to_wire();
            (k, i, crate::inventory::item_to_wire_full(item))
        }
        None => {
            let (k, i) = protocol::ItemRef::Empty.to_wire();
            (k, i, protocol::WireItem::None)
        }
    }
}

/// One joiner of a server: its transport, slot and inbox.
struct Joiner {
    client: ChannelClientTransport,
    slot: usize,
    inbox: Inbox,
    seq: u32,
}

impl Joiner {
    fn attack(&mut self, entity: u32, held: Option<&Item>, sneak: bool) -> u32 {
        self.seq += 1;
        let (held_kind, held_id, held_full) = held_wire(held);
        let pkt = protocol::EntityAttackPacket {
            seq: self.seq,
            entity,
            held_kind,
            held_id,
            held_full,
            sprint: false,
            sneak,
        };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::EntityAttack, &pkt));
        self.seq
    }

    fn interact(&mut self, entity: u32, kind: InteractKind, held: Option<&Item>) -> u32 {
        self.seq += 1;
        let (held_kind, held_id, held_full) = held_wire(held);
        let pkt = protocol::EntityInteractPacket {
            seq: self.seq,
            entity,
            kind,
            held_kind,
            held_id,
            held_full,
            hotbar_slot: 0,
            sneak: false,
        };
        self.client
            .send_to_server(&protocol::serialize_packet(protocol::PacketType::EntityInteract, &pkt));
        self.seq
    }
}

/// Stand `slot`'s server body on a stone floor at (40, 80, 40), on the
/// ground (no critical hits) and at rest.
fn floor_and_stand(world: &mut crate::world::World, hs: &mut HostedServer, slot: usize) -> Vec3 {
    let (fx, fy, fz) = (40, 80, 40);
    for x in fx - 8..=fx + 8 {
        for z in fz - 8..=fz + 8 {
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
    p.on_ground = true;
    p.reset_fall();
    at
}

/// A dedicated server with joiners standing on a floor, mobs cleared.
struct Rig {
    hs: HostedServer,
    joiners: Vec<Joiner>,
    at: Vec3,
}

impl Rig {
    fn new(tag: &str, joiners: usize) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("joiners-act-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let mut list = Vec::new();
        for n in 0..joiners {
            let (client, slot) = join_guest(&mut hs, &format!("J{n}"));
            list.push(Joiner { client, slot, inbox: Inbox::default(), seq: 0 });
        }
        let mut world = std::mem::replace(&mut hs.server.world, crate::world::World::new());
        let mut at = Vec3::ZERO;
        for j in &list {
            at = floor_and_stand(&mut world, &mut hs, j.slot);
        }
        hs.server.world = world;
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        hs.server.difficulty = crate::survival::Difficulty::Peaceful;
        crate::remote_mobs::purge_private_mobs(&mut hs.server.ecs);
        let mut rig = Rig { hs, joiners: list, at };
        rig.tick(1);
        rig
    }

    fn tick(&mut self, n: u32) {
        for _ in 0..n {
            self.hs.tick();
            for j in &mut self.joiners {
                j.inbox.drain(&j.client, j.slot);
            }
        }
    }

    /// A `kind` at `offset` from the joiners, broadcast once (so it has a
    /// wire id). Returns (entity, wire id).
    fn spawn(&mut self, kind: MobType, offset: Vec3) -> (hecs::Entity, u32) {
        let e = crate::entity::spawn_mob(&mut self.hs.server.ecs, kind, self.at + offset);
        self.tick(1);
        let id = self.hs.server.ecs.get::<&crate::entity::ProtocolId>(e).expect("broadcast").0;
        (e, id)
    }

    /// Put `e` back at `offset` from the joiners, at rest — the request sent
    /// next is read before this tick's mob AI moves it.
    fn place(&mut self, e: hecs::Entity, offset: Vec3) {
        let at = self.at + offset;
        self.hs.server.ecs.get::<&mut crate::entity::Position>(e).unwrap().0 = at;
        self.hs.server.ecs.get::<&mut crate::entity::Velocity>(e).unwrap().0 = Vec3::ZERO;
    }

    fn health(&self, e: hecs::Entity) -> f32 {
        self.hs.server.ecs.get::<&crate::combat::Health>(e).unwrap().current
    }

    fn sign_in(&mut self, j: usize) -> String {
        let slot = self.joiners[j].slot;
        self.hs.server.players[slot].verified_pubkey = Some(NPUB_KEY);
        let key = self.hs.server.players[slot].pet_owner_key().unwrap();
        assert!(key.starts_with("npub1"), "a pet's owner is the verified npub, never hex: {key}");
        key
    }
}

#[test]
fn a_joiner_swing_in_reach_damages_knocks_back_and_names_the_joiner() {
    let mut rig = Rig::new("hit", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    let max = rig.health(cow);
    rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
    let y0 = rig.at.y;
    let seq = rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(seq);
    assert!(out.accepted && out.kind.is_none(), "a valid swing is confirmed (the weapon wears)");
    assert_eq!(rig.health(cow), max - sword().attack_damage(), "the sword's damage, no crit on the ground");
    let la = rig.hs.server.ecs.get::<&crate::combat::LastAttacker>(cow).expect("stamped").0;
    assert_eq!(la, crate::combat::Attacker::Remote(rig.joiners[0].slot));
    let y = rig.hs.server.ecs.get::<&crate::entity::Position>(cow).unwrap().0.y;
    assert!(y > y0, "the hit's knockback pops the cow off the floor ({y} > {y0})");
}

#[test]
fn an_out_of_reach_or_over_rate_swing_is_refused_and_changes_nothing() {
    let mut rig = Rig::new("refuse", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 7.0));
    let max = rig.health(cow);
    rig.place(cow, Vec3::new(0.0, 0.0, 7.0));
    let far = rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    assert!(!rig.joiners[0].inbox.outcome(far).accepted, "7 blocks is out of reach");
    assert_eq!(rig.health(cow), max);
    assert!(rig.hs.server.ecs.get::<&crate::combat::LastAttacker>(cow).is_err());

    // Two swings in one tick: the second is inside the server's cooldown.
    rig.tick(crate::combat::ATTACK_COOLDOWN);
    rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
    let first = rig.joiners[0].attack(id, None, false);
    let second = rig.joiners[0].attack(id, None, false);
    rig.tick(1);
    assert!(rig.joiners[0].inbox.outcome(first).accepted);
    assert!(!rig.joiners[0].inbox.outcome(second).accepted, "over the rate: refused, no wear");
    assert_eq!(rig.health(cow), max - 1.0, "one fist's worth, once");
}

#[test]
fn a_dedicated_servers_kill_goes_to_the_killer_alone_and_drops_loot() {
    let mut rig = Rig::new("kill-dedicated", 2);
    let (chicken, id) = rig.spawn(MobType::Chicken, Vec3::new(0.0, 0.0, 2.0));
    rig.hs.server.ecs.get::<&mut crate::combat::Health>(chicken).unwrap().current = 1.0;
    rig.place(chicken, Vec3::new(0.0, 0.0, 2.0));
    rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    assert!(rig.hs.server.ecs.get::<&crate::combat::Health>(chicken).is_err(), "dead and swept");
    let kills = &rig.joiners[0].inbox.kills;
    assert_eq!(kills.len(), 1, "the killer is told, once");
    assert_eq!(kills[0].victim, protocol::EntityKind::Chicken);
    assert_eq!(kills[0].cause, protocol::kill_cause::MELEE);
    assert!(rig.joiners[1].inbox.kills.is_empty(), "nobody else is credited");
    let items = rig.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().count();
    assert!(items > 0, "the kill's loot drops as world items");
}

#[test]
fn a_lent_hosts_kill_by_a_joiner_goes_to_that_joiner_and_never_to_the_host() {
    use super::lent_world::{join_guest_lent, start_lent};
    let (mut hs, mut host) = start_lent("d2b-kill");
    let (client, slot) = join_guest_lent(&mut hs, &mut host, "Joiner");
    let mut world = std::mem::replace(&mut host.world, crate::world::World::new());
    let at = floor_and_stand(&mut world, &mut hs, slot);
    host.world = world;
    crate::remote_mobs::purge_private_mobs(&mut host.ecs);
    let chicken = crate::entity::spawn_mob(&mut host.ecs, MobType::Chicken, at + Vec3::new(0.0, 0.0, 2.0));
    host.ecs.get::<&mut crate::combat::Health>(chicken).unwrap().current = 1.0;
    host.lend_tick(&mut hs);
    let id = host.ecs.get::<&crate::entity::ProtocolId>(chicken).expect("broadcast").0;
    let mut joiner = Joiner { client, slot, inbox: Inbox::default(), seq: 0 };
    joiner.attack(id, Some(&sword()), false);
    host.lend_tick(&mut hs);
    joiner.inbox.drain(&joiner.client, slot);

    // The host client's death sweep (a lent world's is the client's): the
    // host's own player stands right beside the body, nearer than anyone.
    let host_player = (at + Vec3::new(0.3, 0.0, 2.0), false);
    let deaths = crate::combat::despawn_dead(&mut host.ecs);
    assert_eq!(deaths.len(), 1);
    let (kind, pos, attacker) = deaths[0];
    assert_eq!(attacker, Some(crate::combat::Attacker::Remote(slot)));
    let credited =
        crate::server::route_client_kill(Some(&mut hs.server), attacker, kind, pos, false, &[host_player]);
    assert_eq!(credited, None, "the host's player is not credited with the joiner's kill");

    host.lend_tick(&mut hs);
    joiner.inbox.drain(&joiner.client, slot);
    assert_eq!(joiner.inbox.kills.len(), 1, "the KillEvent rides the next tick to the joiner");
    assert_eq!(joiner.inbox.kills[0].victim, protocol::EntityKind::Chicken);

    // Nobody's hit: the nearest local player, as ever.
    assert_eq!(
        crate::server::route_client_kill(Some(&mut hs.server), None, kind, pos, false, &[host_player]),
        Some(0)
    );
}

#[test]
fn feeding_two_cows_puts_them_in_love_and_they_breed() {
    let mut rig = Rig::new("feed", 1);
    let (a, ida) = rig.spawn(MobType::Cow, Vec3::new(1.0, 0.0, 1.0));
    let (b, idb) = rig.spawn(MobType::Cow, Vec3::new(-1.0, 0.0, 1.0));
    let wheat = mat(MaterialId::Wheat);
    rig.place(a, Vec3::new(1.0, 0.0, 1.0));
    rig.place(b, Vec3::new(-1.0, 0.0, 1.0));
    let s1 = rig.joiners[0].interact(ida, InteractKind::Feed, Some(&wheat));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    rig.place(b, Vec3::new(-1.0, 0.0, 1.0));
    let s2 = rig.joiners[0].interact(idb, InteractKind::Feed, Some(&wheat));
    rig.tick(1);
    for s in [s1, s2] {
        let out = rig.joiners[0].inbox.outcome(s);
        assert!(out.accepted);
        assert_eq!(out.consume_held, 1, "one wheat each");
        assert_eq!(InteractNote::from_wire(out.note), InteractNote::Fed);
    }
    for cow in [a, b] {
        assert!(rig.hs.server.ecs.get::<&crate::breeding::InLove>(cow).is_ok());
    }
    // Pull them together: the breeding rule pairs them.
    rig.place(a, Vec3::new(0.5, 0.0, 1.0));
    rig.place(b, Vec3::new(-0.5, 0.0, 1.0));
    let tick = rig.hs.server.tick_counter;
    let babies = crate::breeding::tick_breeding(&mut rig.hs.server.ecs, tick);
    assert_eq!(babies.len(), 1, "two fed adults breed");
}

#[test]
fn shearing_drops_wool_the_joiner_picks_up_and_takes_nothing_from_the_hand() {
    let mut rig = Rig::new("shear", 1);
    let (sheep, id) = rig.spawn(MobType::Sheep, Vec3::new(0.0, 0.0, 1.0));
    rig.place(sheep, Vec3::new(0.0, 0.0, 1.0));
    let shears = Item::Tool(Tool::new(ToolType::Shears, ToolMaterial::Iron));
    let seq = rig.joiners[0].interact(id, InteractKind::Shear, Some(&shears));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(seq);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 0, "shears are not used up");
    assert_eq!(InteractNote::from_wire(out.note), InteractNote::Sheared);
    rig.tick(60);
    assert!(rig.joiners[0].inbox.granted(MaterialId::Wool) >= 1, "the wool arrives as an InventoryGrant");
    // Shorn: a second go is refused until it regrows, and takes nothing.
    rig.place(sheep, Vec3::new(0.0, 0.0, 1.0));
    let again = rig.joiners[0].interact(id, InteractKind::Shear, Some(&shears));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(again);
    assert!(!out.accepted);
    assert_eq!(InteractNote::from_wire(out.note), InteractNote::WoolGrowing);
}

#[test]
fn milking_swaps_the_bucket_for_a_milk_bucket() {
    let mut rig = Rig::new("milk", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 1.5));
    rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
    let seq = rig.joiners[0].interact(id, InteractKind::Milk, Some(&mat(MaterialId::Bucket)));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(seq);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 1, "the bucket is used");
    assert_eq!(rig.joiners[0].inbox.granted(MaterialId::MilkBucket), 1, "a milk bucket is granted");
}

#[test]
fn a_signed_in_joiner_tames_to_its_npub_and_a_guest_cannot() {
    let mut rig = Rig::new("tame", 2);
    let key = rig.sign_in(0);
    let (cat, id) = rig.spawn(MobType::Cat, Vec3::new(0.0, 0.0, 1.5));
    let treat = mat(MaterialId::CatTreat);

    // The guest first: refused, nothing taken, still wild.
    rig.place(cat, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[1].interact(id, InteractKind::Tame, Some(&treat));
    rig.tick(1);
    let out = rig.joiners[1].inbox.outcome(s);
    assert!(!out.accepted);
    assert_eq!(out.consume_held, 0, "a refused interaction consumes nothing");
    assert_eq!(InteractNote::from_wire(out.note), InteractNote::SignInToTame);
    assert!(crate::tameable::pet_owner_of(&rig.hs.server.ecs, cat).is_none());

    rig.place(cat, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(id, InteractKind::Tame, Some(&treat));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 1);
    assert_eq!(InteractNote::from_wire(out.note), InteractNote::Tamed);
    assert_eq!(crate::tameable::pet_owner_of(&rig.hs.server.ecs, cat), Some(key));
}

#[test]
fn sit_toggle_works_on_the_joiners_own_pet_only() {
    let mut rig = Rig::new("sit", 1);
    let key = rig.sign_in(0);
    let (mine, my_id) = rig.spawn(MobType::Wolf, Vec3::new(1.0, 0.0, 1.5));
    let (theirs, their_id) = rig.spawn(MobType::Wolf, Vec3::new(-1.0, 0.0, 1.5));
    rig.hs.server.ecs.get::<&mut crate::wolf::WolfData>(mine).unwrap().ownership.owner_pubkey = key;
    rig.hs.server.ecs.get::<&mut crate::wolf::WolfData>(theirs).unwrap().ownership.owner_pubkey =
        "npub1someoneelse".into();
    rig.place(mine, Vec3::new(1.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(my_id, InteractKind::SitToggle, None);
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted);
    assert_eq!(InteractNote::from_wire(out.note), InteractNote::Sat);
    assert_eq!(
        rig.hs.server.ecs.get::<&crate::wolf::WolfData>(mine).unwrap().state,
        crate::wolf::WolfAiState::Sit
    );

    rig.place(theirs, Vec3::new(-1.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(their_id, InteractKind::SitToggle, None);
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(!out.accepted);
    assert_eq!(InteractNote::from_wire(out.note), InteractNote::NotYourPet);
    assert_ne!(
        rig.hs.server.ecs.get::<&crate::wolf::WolfData>(theirs).unwrap().state,
        crate::wolf::WolfAiState::Sit
    );
}

#[test]
fn a_lead_goes_on_to_the_joiners_body_and_comes_back_off() {
    let mut rig = Rig::new("lead", 1);
    let slot = rig.joiners[0].slot;
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 1.5));
    rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(id, InteractKind::LeadAttach, Some(&mat(MaterialId::Lead)));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 1, "the Lead is used");
    let tether = rig.hs.server.ecs.get::<&crate::tether::Tethered>(cow).unwrap().target;
    assert_eq!(tether, crate::tether::TetherTarget::Player(slot), "fastened to the joiner");

    rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(id, InteractKind::LeadDetach, None);
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 0);
    assert!(rig.hs.server.ecs.get::<&crate::tether::Tethered>(cow).is_err());
    assert_eq!(rig.joiners[0].inbox.granted(MaterialId::Lead), 1, "the Lead comes back");
}

#[test]
fn an_interaction_out_of_reach_or_too_soon_is_refused_and_takes_nothing() {
    let mut rig = Rig::new("interact-refuse", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 8.0));
    rig.place(cow, Vec3::new(0.0, 0.0, 8.0));
    let wheat = mat(MaterialId::Wheat);
    let far = rig.joiners[0].interact(id, InteractKind::Feed, Some(&wheat));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(far);
    assert!(!out.accepted && out.consume_held == 0);
    assert!(rig.hs.server.ecs.get::<&crate::breeding::InLove>(cow).is_err());

    rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
    let first = rig.joiners[0].interact(id, InteractKind::Milk, Some(&mat(MaterialId::Bucket)));
    let soon = rig.joiners[0].interact(id, InteractKind::Feed, Some(&wheat));
    rig.tick(1);
    assert!(rig.joiners[0].inbox.outcome(first).accepted);
    let out = rig.joiners[0].inbox.outcome(soon);
    assert!(!out.accepted && out.consume_held == 0, "inside the interaction cooldown");
}

/// D2a gap — a hit the server lands on a joiner wears its armour (one
/// `ArmourWorn` hit per landed hit), and a server-side death names its cause.
#[test]
fn server_landed_hits_wear_armour_and_a_death_names_its_cause() {
    let mut rig = Rig::new("armour-cause", 1);
    rig.hs.server.difficulty = crate::survival::Difficulty::Normal;
    let slot = rig.joiners[0].slot;
    let (brigand, _) = rig.spawn(MobType::Brigand, Vec3::new(0.6, 0.0, 0.0));
    let worn: u32 = rig.joiners[0]
        .inbox
        .events
        .iter()
        .map(|e| match e {
            PlayerEventType::ArmourWorn { hits } => *hits as u32,
            _ => 0,
        })
        .sum();
    assert!(worn >= 1, "the brigand's landed hit wears the joiner's armour");

    rig.hs.server.players[slot].combat.health = 1.0;
    rig.hs.server.players[slot].combat.invincible_timer = 0;
    rig.place(brigand, Vec3::new(0.6, 0.0, 0.0));
    rig.tick(2);
    assert!(rig.hs.server.players[slot].combat.dead);
    let cause = rig.joiners[0].inbox.events.iter().find_map(|e| match e {
        PlayerEventType::DiedOf { cause } => Some(*cause),
        _ => None,
    });
    assert_eq!(cause, Some(WireDamageCause::Mob(protocol::EntityKind::Brigand)));
    assert_eq!(
        crate::remote_client::damage_cause_from_wire(cause.unwrap()),
        crate::survival::DamageCause::Mob(MobType::Brigand),
        "the death screen reads 'Killed by a Brigand'"
    );
}

/// D2a gap — a species attack the host client's AI lands on a joiner (a bee
/// sting, a goat charge, a shark bite) reaches its server body: armour-soaked,
/// worn, and named as the cause.
#[test]
fn a_species_hit_lands_on_a_joiners_body_wears_its_armour_and_names_the_species() {
    let mut rig = Rig::new("species", 1);
    let slot = rig.joiners[0].slot;
    assert!(rig.hs.server.land_hit_on_joiner(
        slot,
        2.0,
        crate::survival::DamageCause::Mob(MobType::Bee),
        Vec3::new(0.4, 0.0, 0.0),
    ));
    assert_eq!(rig.hs.server.players[slot].combat.health, 18.0);
    assert!(rig.hs.server.players[slot].player.velocity.x > 0.0, "knocked back");
    rig.tick(1);
    assert!(rig.joiners[0].inbox.events.contains(&PlayerEventType::ArmourWorn { hits: 1 }));
    // Lethal: the joiner is told it died, and of what.
    rig.hs.server.players[slot].combat.invincible_timer = 0;
    assert!(rig.hs.server.land_hit_on_joiner(
        slot,
        50.0,
        crate::survival::DamageCause::Mob(MobType::Shark),
        Vec3::ZERO,
    ));
    rig.tick(1);
    assert!(rig.joiners[0].inbox.events.contains(&PlayerEventType::DiedOf {
        cause: WireDamageCause::Mob(protocol::EntityKind::Shark),
    }));
    // The dead take no more hits.
    assert!(!rig.hs.server.land_hit_on_joiner(
        slot,
        2.0,
        crate::survival::DamageCause::Mob(MobType::Bee),
        Vec3::ZERO,
    ));
}

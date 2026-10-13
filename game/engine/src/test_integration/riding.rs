//! MP-D2c-1 (2026-10-09) — riding runs on the server. A joiner rides a
//! horse, donkey, mule, Nostrich or rail cart by request
//! (`InteractKind::Mount`); the server steers the mount from the joiner's
//! input each tick (`riding::steer`, right before entity physics), pins the
//! joiner's body to the seat after it (`riding::follow`), ends the ride by
//! the same rules single-player runs, and says so (`own_riding`,
//! `entity_flags::RIDDEN`, `player_flags::SEATED`).
//!
//! Every test drives a REAL `HostedServer` over the in-process transport — a
//! dedicated server (0 local slots: no species AI, it owns its world and its
//! entity physics), or a lending host (`sim_lend::OwnedSimParts` standing in
//! for the host client, whose species AI and entity physics step —
//! `sim_lend::host_entity_physics` — run before each lend window, as
//! `GameState::tick` runs them) — and reads what each joiner is sent.

use std::collections::HashMap;

use glam::Vec3;

use crate::block::{self, BlockRegistry};
use crate::entity::{Position, ProtocolId, Ridden};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item::Item;
use crate::mob::MobType;
use crate::mob_interact::InteractNote;
use crate::protocol::{self, entity_flags, player_flags, InteractKind};
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;
use super::lent_world::{join_guest_lent, start_lent};

/// Facing +z (`Camera::horizontal_forward` at yaw π).
const SOUTH: f32 = std::f32::consts::PI;

/// The arena's floor: stone at y = 79 (feet at 80) round (40, 40).
const CX: i32 = 40;
const FLOOR: i32 = 79;
const CZ: i32 = 40;
const R: i32 = 14;

/// What one joiner was sent that these tests read.
#[derive(Default)]
struct Inbox {
    outcomes: Vec<protocol::InteractOutcomePacket>,
    /// `own_riding` of the latest StateUpdate.
    own_riding: Option<u32>,
    /// The latest `PlayerState` of every player, by slot.
    players: HashMap<u32, protocol::PlayerState>,
    /// The latest `EntityUpdate` flags of every entity, by wire id.
    flags: HashMap<u32, u8>,
}

impl Inbox {
    fn drain(&mut self, client: &ChannelClientTransport) {
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::InteractOutcome => {
                    self.outcomes.push(protocol::safe_deserialize(payload).unwrap());
                }
                protocol::PacketType::StateUpdate => {
                    let s: protocol::StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
                    self.own_riding = s.own_riding;
                    for p in s.players {
                        self.players.insert(p.player_index, p);
                    }
                    for u in s.entity_updates {
                        self.flags.insert(u.id, u.flags);
                    }
                }
                _ => {}
            }
        }
    }

    fn outcome(&self, seq: u32) -> &protocol::InteractOutcomePacket {
        self.outcomes.iter().find(|o| o.seq == seq).expect("the request was answered")
    }
}

/// The keys one input carries.
#[derive(Clone, Copy, Default)]
struct Keys {
    forward: f32,
    sneak: bool,
    jump: bool,
}

fn ahead() -> Keys {
    Keys { forward: 1.0, ..Default::default() }
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

struct Joiner {
    client: ChannelClientTransport,
    slot: usize,
    inbox: Inbox,
    seq: u32,
    input_seq: u64,
}

impl Joiner {
    /// One input, looking along +z.
    fn input(&mut self, k: Keys) {
        self.input_seq += 1;
        let input = protocol::InputPacket {
            tick: self.input_seq,
            yaw: SOUTH,
            health: 20.0,
            move_forward: k.forward,
            sneak: k.sneak,
            jump: k.jump,
            ..Default::default()
        };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    /// Ask to ride wire id `entity` with `held` in hand.
    fn mount(&mut self, entity: u32, held: Option<&Item>) -> u32 {
        self.seq += 1;
        let (held_kind, held_id, held_full) = held_wire(held);
        let pkt = protocol::EntityInteractPacket {
            seq: self.seq,
            entity,
            kind: InteractKind::Mount,
            held_kind,
            held_id,
            held_full,
            hotbar_slot: 0,
            sneak: false,
            events_applied: u32::MAX,
        };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::EntityInteract, &pkt));
        self.seq
    }

    fn leave(&self) {
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Disconnect, &()));
    }
}

/// A walled-off stone floor round (40, 80, 40), clear air above it, its
/// columns loaded.
fn arena(world: &mut crate::world::World, loaded: &mut ahash::AHashSet<(i32, i32)>) {
    for x in CX - R..=CX + R {
        for z in CZ - R..=CZ + R {
            world.set_block(x, FLOOR, z, block::STONE);
            for y in FLOOR + 1..=FLOOR + 8 {
                world.set_block(x, y, z, block::AIR);
            }
        }
    }
    for cx in (CX - R).div_euclid(16)..=(CX + R).div_euclid(16) {
        for cz in (CZ - R).div_euclid(16)..=(CZ + R).div_euclid(16) {
            loaded.insert((cx, cz));
        }
    }
}

/// Joiners on a floor: a dedicated server (`host: None`) or a lending host.
struct Rig {
    hs: HostedServer,
    host: Option<OwnedSimParts>,
    joiners: Vec<Joiner>,
    registry: BlockRegistry,
}

impl Rig {
    fn new(tag: &str, joiners: usize) -> Self {
        let hs = HostedServer::start(
            0,
            format!("riding-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        Self::seat_joiners(hs, None, joiners)
    }

    fn new_lent(tag: &str, joiners: usize) -> Self {
        let (hs, host) = start_lent(&format!("riding-{tag}"));
        Self::seat_joiners(hs, Some(host), joiners)
    }

    fn seat_joiners(hs: HostedServer, host: Option<OwnedSimParts>, n: usize) -> Self {
        let mut rig = Rig { hs, host, joiners: Vec::new(), registry: BlockRegistry::new() };
        for k in 0..n {
            let (client, slot) = match rig.host.as_mut() {
                Some(host) => join_guest_lent(&mut rig.hs, host, &format!("R{k}")),
                None => join_guest(&mut rig.hs, &format!("R{k}")),
            };
            rig.joiners.push(Joiner { client, slot, inbox: Inbox::default(), seq: 0, input_seq: 0 });
        }
        match rig.host.as_mut() {
            Some(host) => arena(&mut host.world, &mut host.loaded_columns),
            None => arena(&mut rig.hs.server.world, &mut rig.hs.server.loaded_columns),
        }
        rig.hs.server.column_streamer = None;
        rig.hs.server.column_refill_per_tick = 0;
        rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
        crate::remote_mobs::purge_private_mobs(rig.ecs_mut());
        // Side by side, a block apart, looking along +z.
        for k in 0..rig.joiners.len() {
            rig.stand(k, Vec3::new(CX as f32 + 0.5 + k as f32, (FLOOR + 1) as f32, CZ as f32 + 0.5));
        }
        rig.tick(1);
        rig
    }

    /// Put joiner `j`'s server body at `at`, at rest, looking along +z.
    fn stand(&mut self, j: usize, at: Vec3) {
        let sp = &mut self.hs.server.players[self.joiners[j].slot];
        sp.yaw = SOUTH;
        sp.pitch = 0.0;
        sp.player.pos = at;
        sp.player.velocity = Vec3::ZERO;
        sp.player.on_ground = true;
        sp.player.reset_fall();
    }

    /// One logical tick. Lent: the host client's tick first — its species
    /// AI, then its entity-physics step (which steers the joiners' mounts
    /// right before it moves them) — then the lend window.
    fn tick(&mut self, n: u32) {
        for _ in 0..n {
            match self.host.as_mut() {
                Some(host) => {
                    let bodies: Vec<Vec3> = self.hs.server.players.iter().map(|sp| sp.player.pos).collect();
                    let tick = host.clock.tick_counter;
                    crate::mob_ai::tick_mob_ai(&mut host.ecs, &host.world, &self.registry, &bodies);
                    crate::species_ai::dispatch_horses(&mut host.ecs, tick);
                    crate::species_ai::dispatch_nostriches(&mut host.ecs, &bodies, tick, &[]);
                    crate::sim_lend::host_entity_physics(Some(&mut self.hs), &mut host.ecs, &host.world, &self.registry);
                    host.lend_tick(&mut self.hs);
                }
                None => self.hs.tick(),
            }
            for j in &mut self.joiners {
                j.inbox.drain(&j.client);
            }
        }
    }

    fn ecs(&self) -> &hecs::World {
        self.host.as_ref().map_or(&self.hs.server.ecs, |h| &h.ecs)
    }

    fn ecs_mut(&mut self) -> &mut hecs::World {
        match self.host.as_mut() {
            Some(host) => &mut host.ecs,
            None => &mut self.hs.server.ecs,
        }
    }

    fn world_mut(&mut self) -> &mut crate::world::World {
        match self.host.as_mut() {
            Some(host) => &mut host.world,
            None => &mut self.hs.server.world,
        }
    }

    /// Joiner 0's feet, the spot mounts are put relative to.
    fn at(&self) -> Vec3 {
        Vec3::new(CX as f32 + 0.5, (FLOOR + 1) as f32, CZ as f32 + 0.5)
    }

    /// A `kind` at `offset` from joiner 0, settled on the floor and
    /// broadcast once (so it has a wire id). Returns (entity, wire id).
    fn spawn(&mut self, kind: MobType, offset: Vec3) -> (hecs::Entity, u32) {
        let at = self.at() + offset;
        let e = crate::entity::spawn_mob(self.ecs_mut(), kind, at);
        self.tick(1);
        self.place(e, offset);
        let id = self.ecs().get::<&ProtocolId>(e).expect("broadcast").0;
        (e, id)
    }

    /// Put `e` back at `offset` from joiner 0, at rest.
    fn place(&mut self, e: hecs::Entity, offset: Vec3) {
        let at = self.at() + offset;
        self.ecs_mut().get::<&mut Position>(e).unwrap().0 = at;
        self.ecs_mut().get::<&mut crate::entity::Velocity>(e).unwrap().0 = Vec3::ZERO;
    }

    fn pos(&self, e: hecs::Entity) -> Vec3 {
        self.ecs().get::<&Position>(e).unwrap().0
    }

    fn body(&self, j: usize) -> &crate::physics::Player {
        &self.hs.server.players[self.joiners[j].slot].player
    }

    fn riding(&self, j: usize) -> Option<hecs::Entity> {
        self.hs.server.players[self.joiners[j].slot].riding
    }

    /// Joiner `j` asks to ride wire id `id` and the server answers.
    fn mount(&mut self, j: usize, id: u32, held: Option<&Item>) -> protocol::InteractOutcomePacket {
        let seq = self.joiners[j].mount(id, held);
        self.tick(1);
        self.joiners[j].inbox.outcome(seq).clone()
    }

    /// `n` ticks of joiner `j` sending `k`.
    fn ride(&mut self, j: usize, k: Keys, n: u32) {
        for _ in 0..n {
            self.joiners[j].input(k);
            self.tick(1);
        }
    }

    /// Joiner `j`'s body is on the seat over `mount`.
    fn assert_seated(&self, j: usize, mount: hecs::Entity) {
        let seat = self.pos(mount) + Vec3::new(0.0, crate::cart::SEAT_LIFT, 0.0);
        let body = self.body(j);
        assert!((body.pos - seat).length() < 1e-4, "the rider sits on the seat: {} vs {seat}", body.pos);
        assert_eq!(body.velocity, Vec3::ZERO);
    }

    /// Joiner `j` stands on the floor, its ride over.
    fn assert_off_on_the_floor(&self, j: usize) {
        let body = self.body(j);
        assert!(self.riding(j).is_none(), "off");
        assert_eq!(body.pos.y, (FLOOR + 1) as f32, "set down on the floor: {}", body.pos);
        assert!(body.on_ground);
        assert_eq!(self.joiners[j].inbox.own_riding, None, "its client hears it is off");
    }
}

#[test]
fn a_joiner_mounts_a_horse_steers_it_and_sneaks_off_and_everyone_sees_it() {
    // A dedicated server: no species AI, and a guest rides all the same.
    let mut rig = Rig::new("horse", 2);
    assert!(!rig.hs.server.animal_life_simulated, "a dedicated server runs no species AI");
    let (horse, id) = rig.spawn(MobType::Horse, Vec3::new(0.0, 0.0, 2.0));
    let out = rig.mount(0, id, None);
    assert!(out.accepted && out.kind == Some(InteractKind::Mount), "{out:?}");
    assert_eq!(out.consume_held, 0, "riding takes nothing");
    assert_eq!(rig.riding(0), Some(horse));
    assert!(rig.ecs().get::<&Ridden>(horse).is_ok(), "the mount carries Ridden");
    assert!(
        !rig.ecs().get::<&crate::horse_ai::HorseData>(horse).unwrap().is_kept(),
        "a guest's ride creates no persistent keeper"
    );
    assert_eq!(rig.joiners[0].inbox.own_riding, Some(id), "own_riding: on");
    rig.assert_seated(0, horse);

    // Forward: the server gallops the horse along the look, 0.3 b/tick before
    // physics drags it, and the rider rides along.
    let start = rig.pos(horse);
    for _ in 0..8 {
        let before = rig.pos(horse);
        rig.ride(0, ahead(), 1);
        let step = rig.pos(horse) - before;
        assert!(
            (step.z - crate::riding::HORSE_SPEED_PER_TICK * crate::entity::DRAG).abs() < 1e-3,
            "one gallop step a tick: {step}"
        );
        assert!(step.x.abs() < 1e-4);
        rig.assert_seated(0, horse);
    }
    assert!(rig.pos(horse).z - start.z > 2.0);

    // The other joiner sees the horse taken and the rider seated, there.
    let seat = rig.body(0).pos;
    let watcher = &rig.joiners[1].inbox;
    assert_ne!(watcher.flags[&id] & entity_flags::RIDDEN, 0, "RIDDEN");
    let rider = &watcher.players[&(rig.joiners[0].slot as u32)];
    assert_ne!(rider.flags & player_flags::SEATED, 0, "SEATED");
    assert_eq!(rider.anim_state, 0, "sitting, not walking");
    assert!((Vec3::new(rider.x, rider.y, rider.z) - seat).length() < 1e-4, "at the seat");

    // Sneak: off, set down on the floor, the horse free.
    rig.ride(0, Keys { sneak: true, ..ahead() }, 1);
    rig.assert_off_on_the_floor(0);
    assert!(rig.ecs().get::<&Ridden>(horse).is_err(), "the horse is free");
    let watcher = &rig.joiners[1].inbox;
    assert_eq!(watcher.flags[&id] & entity_flags::RIDDEN, 0);
    assert_eq!(watcher.players[&(rig.joiners[0].slot as u32)].flags & player_flags::SEATED, 0);
}

#[test]
fn a_nostrich_winds_up_per_tick_and_a_slow_one_over_water_sinks_its_rider_off() {
    use crate::nostrich_ride::{ACCEL, BASE_SPEED};
    let mut rig = Rig::new("nostrich", 1);
    let (bird, id) = rig.spawn(MobType::Nostrich, Vec3::new(0.0, 0.0, 2.0));
    assert!(rig.mount(0, id, None).accepted);
    // One ride step a tick: BASE_SPEED off the line, then ACCEL b/s².
    for k in 1..=8u32 {
        let before = rig.pos(bird);
        rig.ride(0, ahead(), 1);
        let speed = rig.hs.server.players[rig.joiners[0].slot].ride.speed;
        let want = BASE_SPEED + ACCEL * k as f32 / 20.0;
        assert!((speed - want).abs() < 1e-3, "tick {k}: {speed} vs {want}");
        let step = rig.pos(bird) - before;
        assert!((step.z - want / 20.0 * crate::entity::DRAG).abs() < 1e-3, "tick {k}: {step}");
        rig.assert_seated(0, bird);
    }
    // Ease off to a stop on the floor (no sneak: that would get us off).
    rig.ride(0, Keys::default(), 20);
    assert_eq!(rig.hs.server.players[rig.joiners[0].slot].ride.speed, 0.0);
    assert_eq!(rig.joiners[0].inbox.own_riding, Some(id), "still on");
    // Water under its feet, and it is far too slow to run across: it sinks
    // and the rider is off — which its client hears through own_riding.
    let p = rig.pos(bird);
    let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
    rig.world_mut().set_block(x, FLOOR, z, block::WATER);
    rig.ride(0, Keys::default(), 1);
    assert!(rig.riding(0).is_none(), "sank off");
    assert!(rig.ecs().get::<&Ridden>(bird).is_err());
    assert_eq!(rig.joiners[0].inbox.own_riding, None, "own_riding: off");
}

#[test]
fn a_joiner_rides_a_parked_server_cart_until_it_parks_and_a_full_hand_only_dispatches_it() {
    let mut rig = Rig::new("cart", 1);
    // A straight line along +z, a parked cart at its near end.
    for z in CZ + 2..=CZ + 8 {
        rig.world_mut().set_block(CX, FLOOR + 1, z, crate::rail::TRACK);
    }
    let cart = crate::cart::spawn_cart(rig.ecs_mut(), (CX, FLOOR + 1, CZ + 2));
    rig.tick(1);
    let id = rig.ecs().get::<&ProtocolId>(cart).expect("broadcast").0;

    let out = rig.mount(0, id, None);
    assert!(out.accepted, "{out:?}");
    assert_eq!(rig.riding(0), Some(cart));
    assert!(rig.ecs().get::<&crate::cart::CartData>(cart).unwrap().speed > 0.0, "dispatched");
    assert_eq!(rig.joiners[0].inbox.own_riding, Some(id));
    // The cart rolls itself; the rider sits on it all the way.
    let mut ticks = 0;
    while rig.riding(0).is_some() {
        rig.assert_seated(0, cart);
        rig.ride(0, ahead(), 1);
        ticks += 1;
        assert!(ticks < 400, "the cart never parked");
    }
    let end = rig.ecs().get::<&crate::cart::CartData>(cart).unwrap().cell;
    assert_eq!(end, (CX, FLOOR + 1, CZ + 8), "parked at the far end");
    assert!(rig.ecs().get::<&Ridden>(cart).is_err(), "the cart is free");
    rig.assert_off_on_the_floor(0);

    // Something in hand: the click only dispatches the cart (its freight);
    // nobody is seated and nothing is taken.
    rig.stand(0, Vec3::new(CX as f32 + 0.5, (FLOOR + 1) as f32, (CZ + 6) as f32 + 0.5));
    let stone = Item::Block(block::STONE);
    let out = rig.mount(0, id, Some(&stone));
    assert!(out.accepted && out.consume_held == 0, "{out:?}");
    assert!(rig.riding(0).is_none(), "not seated");
    assert!(rig.ecs().get::<&crate::cart::CartData>(cart).unwrap().speed > 0.0, "dispatched back");
}

#[test]
fn a_mounted_mob_dying_ends_the_ride() {
    let mut rig = Rig::new("mount-dies", 1);
    let (mule, id) = rig.spawn(MobType::Mule, Vec3::new(0.0, 0.0, 2.0));
    assert!(rig.mount(0, id, None).accepted);
    rig.ride(0, ahead(), 2);
    rig.ecs_mut().get::<&mut crate::combat::Health>(mule).unwrap().current = 0.0;
    rig.ride(0, ahead(), 1);
    assert!(rig.ecs().get::<&Position>(mule).is_err(), "the server's death sweep took it");
    rig.assert_off_on_the_floor(0);
}

#[test]
fn a_second_rider_on_an_occupied_mount_is_refused_and_a_rider_mounts_nothing_else() {
    let mut rig = Rig::new("taken", 2);
    let (horse, id) = rig.spawn(MobType::Horse, Vec3::new(0.0, 0.0, 2.0));
    let (donkey, other) = rig.spawn(MobType::Donkey, Vec3::new(1.0, 0.0, 2.5));
    rig.place(horse, Vec3::new(0.0, 0.0, 2.0));
    assert!(rig.mount(0, id, None).accepted);
    let out = rig.mount(1, id, None);
    assert!(!out.accepted, "one rider a mount");
    assert_eq!(InteractNote::from_wire(out.note), InteractNote::MountTaken);
    assert!(rig.riding(1).is_none());
    assert_eq!(rig.riding(0), Some(horse), "the first rider keeps it");
    // A rider asks for another mount: nothing.
    rig.place(donkey, Vec3::new(1.0, 0.0, 2.5));
    let out = rig.mount(0, other, None);
    assert!(!out.accepted);
    assert_eq!(rig.riding(0), Some(horse));
    assert!(rig.ecs().get::<&Ridden>(donkey).is_err());
}

/// On a lent world the joiners' mounts move by the HOST client's entity
/// physics. Their steering must land after every velocity writer the host
/// runs first and right before that physics step
/// (`sim_lend::host_entity_physics`): a tamed Nostrich following its owner —
/// here its rider — is held still by its species AI every tick, and still
/// runs where its rider steers it.
#[test]
fn a_lent_world_steers_a_joiners_mount_after_the_hosts_species_ai_and_before_its_physics() {
    let mut rig = Rig::new_lent("lent", 1);
    let (bird, id) = rig.spawn(MobType::Nostrich, Vec3::new(0.0, 0.0, 2.0));
    let owner = format!("local-player-{}", rig.joiners[0].slot);
    {
        let mut d = rig.ecs_mut().get::<&mut crate::nostrich::NostrichData>(bird).unwrap();
        d.ownership.owner_pubkey = owner;
        d.state = crate::nostrich::NostrichAiState::Follow;
    }
    assert!(rig.mount(0, id, None).accepted);
    let start = rig.pos(bird);
    rig.ride(0, ahead(), 10);
    assert_eq!(
        rig.ecs().get::<&crate::nostrich::NostrichData>(bird).unwrap().state,
        crate::nostrich::NostrichAiState::Follow,
        "its species AI ran (and held it beside its owner) every tick"
    );
    let gone = rig.pos(bird).z - start.z;
    assert!(gone > 2.0, "the rider's steering had the last word: {gone}");
    rig.assert_seated(0, bird);
    assert_eq!(rig.joiners[0].inbox.own_riding, Some(id));
    rig.ride(0, Keys::default(), 25);
    // The sneak steers on the host's next physics step: a tick later.
    rig.ride(0, Keys { sneak: true, ..Keys::default() }, 2);
    rig.assert_off_on_the_floor(0);
}

/// The coordinator's D2c-1 addition: a rider's death ends its ride by any
/// path, and a respawn never finds a seat to be pinned back to.
#[test]
fn a_riders_death_ends_the_ride_and_its_respawn_finds_no_seat() {
    let mut rig = Rig::new("rider-dies", 1);
    let (horse, id) = rig.spawn(MobType::Horse, Vec3::new(0.0, 0.0, 2.0));
    assert!(rig.mount(0, id, None).accepted);
    rig.ride(0, ahead(), 3);
    let slot = rig.joiners[0].slot;
    rig.hs.server.players[slot].combat.take_damage_from(100.0, crate::survival::DamageCause::Generic);
    rig.tick(1);
    assert!(rig.hs.server.players[slot].combat.dead);
    assert!(rig.riding(0).is_none(), "a dead rider is off");
    assert!(rig.ecs().get::<&Ridden>(horse).is_err(), "the horse is free");
    assert_eq!(rig.joiners[0].inbox.own_riding, None);
    let fell = rig.body(0).pos;
    assert_eq!(fell.y, (FLOOR + 1) as f32, "the body came down where it fell: {fell}");

    // A stale seat (no path may leave one, but the respawn is the backstop):
    // the respawn ends it, and the body stays at the spawn point.
    rig.tick(crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN + 1);
    rig.hs.server.players[slot].riding = Some(horse);
    rig.ecs_mut().insert_one(horse, Ridden).unwrap();
    let spawn = rig.hs.server.respawn_player(slot).expect("respawned");
    assert!(rig.riding(0).is_none());
    assert!(rig.ecs().get::<&Ridden>(horse).is_err());
    rig.tick(2);
    let body = rig.body(0).pos;
    assert!(
        (body.x - spawn.x).abs() < 1e-3 && (body.z - spawn.z).abs() < 1e-3,
        "not pinned back to the horse: {body} (spawn {spawn})"
    );
}

#[test]
fn a_rider_who_leaves_frees_its_mount() {
    let mut rig = Rig::new("leaves", 1);
    let (horse, id) = rig.spawn(MobType::Horse, Vec3::new(0.0, 0.0, 2.0));
    assert!(rig.mount(0, id, None).accepted);
    rig.ride(0, ahead(), 2);
    rig.joiners[0].leave();
    rig.tick(2);
    assert!(rig.ecs().get::<&Ridden>(horse).is_err(), "free for the next rider");
}

#[test]
fn d2c2_signed_mount_keeps_the_verified_persona_and_guest_riding_preserves_it() {
    let mut rig = Rig::new("d2c2-keeping", 2);
    let signed_slot = rig.joiners[0].slot;
    // Join-time authentication has already established this server fact;
    // the riding request itself supplies no ownership claim.
    rig.hs.server.players[signed_slot].verified_pubkey = Some([7; 32]);
    let npub = rig.hs.server.players[signed_slot].pet_owner_key().unwrap();
    let (steed, id) = rig.spawn(MobType::Donkey, Vec3::new(0.0, 0.0, 2.0));
    rig.ecs_mut().insert_one(steed, crate::entity::Scattered).unwrap();
    assert!(rig.mount(0, id, None).accepted);
    assert_eq!(rig.ecs().get::<&crate::horse_ai::HorseData>(steed).unwrap().keeper_identity(), Some(crate::horse_ai::SteedKeeper::Npub(npub.clone())));
    assert!(rig.ecs().get::<&crate::entity::Scattered>(steed).is_err());
    rig.joiners[0].leave();
    rig.tick(2);
    rig.place(steed, Vec3::new(0.0, 0.0, 2.0));
    assert!(rig.mount(1, id, None).accepted, "a guest can ride an already-kept steed");
    assert_eq!(crate::tameable::pet_owner_of(rig.ecs(), steed), Some(npub.clone()), "guest mounting never replaces the persistent keeper");
    assert!(crate::tameable::is_own_pet(rig.ecs(), steed, &npub, None), "the same identity remains protected after leaving its slot");
}

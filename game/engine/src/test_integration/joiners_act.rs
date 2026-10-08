//! MP-D2b (2026-10-07) — joiners act on the server's mobs: a swing, the
//! one-shot right-click interactions, kill credit; and the D2a gaps that
//! close with them (server-landed hits wear a joiner's armour, a server-side
//! death names its cause, species attacks reach joiners).
//!
//! Every test drives a REAL `HostedServer` over the in-process transport —
//! a dedicated server (0 local slots: it owns its world and runs its own
//! death sweep), or a lending host (`sim_lend::OwnedSimParts` standing in
//! for the host client, whose death sweep and breeding step are the host
//! client's) — and reads what each joiner is actually sent.

use glam::Vec3;

use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::sim_lend::OwnedSimParts;
use crate::item::{Item, MaterialId};
use crate::mob::MobType;
use crate::mob_interact::InteractNote;
use crate::protocol::{self, InteractKind, PlayerEventType, WireDamageCause};
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;
use super::lent_world::{join_guest_lent, start_lent};

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
    /// The sequence number of its last `ClientInput` (counts from 1).
    input_seq: u64,
}

impl Joiner {
    /// FU1 — `n` inputs standing still, looking along +z (where these tests
    /// put the mobs), as a client catching up after a frame hitch sends them:
    /// one per tick it ran, all at once.
    fn catch_up_burst(&mut self, at: Vec3, n: u32) {
        for _ in 0..n {
            self.input_seq += 1;
            let input = protocol::InputPacket {
                tick: self.input_seq,
                x: at.x,
                y: at.y,
                z: at.z,
                yaw: std::f32::consts::PI,
                health: 20.0,
                ..Default::default()
            };
            self.client
                .send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        }
    }

    fn attack(&mut self, entity: u32, held: Option<&Item>, sneak: bool) -> u32 {
        self.attack_from(0, entity, held, sneak)
    }

    /// [`Self::attack`] with the weapon in hotbar slot `hotbar_slot`.
    fn attack_from(&mut self, hotbar_slot: u8, entity: u32, held: Option<&Item>, sneak: bool) -> u32 {
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
            hotbar_slot,
            // C3a-fix-1 — a test client that applies every window event the
            // moment the server sends it.
            events_applied: u32::MAX,
        };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::EntityAttack, &pkt));
        self.seq
    }

    fn leave(&self) {
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Disconnect, &()));
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
            events_applied: u32::MAX,
        };
        self.client
            .send_to_server(&protocol::serialize_packet(protocol::PacketType::EntityInteract, &pkt));
        self.seq
    }
}

/// Stand `slot`'s server body on a stone floor at (40, 80, 40), on the
/// ground (no critical hits), at rest, and looking along +z (yaw π), where
/// these tests put the mobs it acts on (a joiner's target must be ahead of
/// its server body, review D2b LOW-2).
pub(super) fn floor_and_stand(world: &mut crate::world::World, hs: &mut HostedServer, slot: usize) -> Vec3 {
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
    let sp = &mut hs.server.players[slot];
    sp.yaw = std::f32::consts::PI;
    sp.pitch = 0.0;
    let p = &mut sp.player;
    p.pos = at;
    p.velocity = Vec3::ZERO;
    p.on_ground = true;
    p.reset_fall();
    at
}

/// A server with joiners standing on a floor, mobs cleared: a dedicated
/// server (`host: None` — it owns its world, ECS and death sweep) or a
/// lending host (`host`: the host client's world and ECS, lent to the server
/// for each tick; its death sweep and breeding step are run by hand, as the
/// host client runs them).
struct Rig {
    hs: HostedServer,
    host: Option<OwnedSimParts>,
    joiners: Vec<Joiner>,
    at: Vec3,
}

impl Rig {
    /// A dedicated server.
    fn new(tag: &str, joiners: usize) -> Self {
        let hs = HostedServer::start(
            0,
            format!("joiners-act-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        Self::seat(hs, None, joiners)
    }

    /// A `--no-lend` host: one local seat, and a server that owns its own
    /// copy of the world (`HostWorld::Owned`) and runs its own death sweep.
    fn new_no_lend(tag: &str, joiners: usize) -> Self {
        let hs = HostedServer::start(
            1,
            format!("joiners-act-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("--no-lend host starts");
        Self::seat(hs, None, joiners)
    }

    /// A lending host (one local seat, slot 0, whose world the server ticks).
    fn new_lent(tag: &str, joiners: usize) -> Self {
        let (hs, host) = start_lent(&format!("joiners-act-{tag}"));
        Self::seat(hs, Some(host), joiners)
    }

    fn seat(hs: HostedServer, host: Option<OwnedSimParts>, joiners: usize) -> Self {
        let mut rig = Rig { hs, host, joiners: Vec::new(), at: Vec3::ZERO };
        for n in 0..joiners {
            rig.join(&format!("J{n}"));
        }
        rig.hs.server.column_streamer = None;
        rig.hs.server.column_refill_per_tick = 0;
        rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
        crate::remote_mobs::purge_private_mobs(rig.ecs_mut());
        rig.tick(1);
        rig
    }

    /// A guest joins and stands on the floor; returns its index in `joiners`.
    fn join(&mut self, name: &str) -> usize {
        let (client, slot) = match self.host.as_mut() {
            Some(host) => join_guest_lent(&mut self.hs, host, name),
            None => join_guest(&mut self.hs, name),
        };
        let mut world = match self.host.as_mut() {
            Some(host) => std::mem::replace(&mut host.world, crate::world::World::new()),
            None => std::mem::replace(&mut self.hs.server.world, crate::world::World::new()),
        };
        self.at = floor_and_stand(&mut world, &mut self.hs, slot);
        match self.host.as_mut() {
            Some(host) => host.world = world,
            None => self.hs.server.world = world,
        }
        self.joiners.push(Joiner { client, slot, inbox: Inbox::default(), seq: 0, input_seq: 0 });
        self.joiners.len() - 1
    }

    fn tick(&mut self, n: u32) {
        for _ in 0..n {
            match self.host.as_mut() {
                Some(host) => host.lend_tick(&mut self.hs),
                None => self.hs.tick(),
            }
            for j in &mut self.joiners {
                j.inbox.drain(&j.client, j.slot);
            }
        }
    }

    /// C3a-fix-1 — every joiner's client says it applied the window events it
    /// was sent (a take, a wear, a grant), and the server applies them to its
    /// shadow.
    fn report(&mut self) {
        for j in &self.joiners {
            super::joiner_authority::report_window_events(&j.client);
        }
        self.tick(1);
    }

    /// The ECS the server's mobs live in (outside a lend window: the host's).
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

    fn tick_counter(&self) -> u64 {
        self.host.as_ref().map_or(self.hs.server.tick_counter, |h| h.clock.tick_counter)
    }

    /// A `kind` at `offset` from the joiners, broadcast once (so it has a
    /// wire id). Returns (entity, wire id).
    fn spawn(&mut self, kind: MobType, offset: Vec3) -> (hecs::Entity, u32) {
        let at = self.at + offset;
        let e = crate::entity::spawn_mob(self.ecs_mut(), kind, at);
        self.tick(1);
        let id = self.ecs().get::<&crate::entity::ProtocolId>(e).expect("broadcast").0;
        (e, id)
    }

    /// Put `e` back at `offset` from the joiners, at rest — the request sent
    /// next is read before this tick's mob AI moves it.
    fn place(&mut self, e: hecs::Entity, offset: Vec3) {
        let at = self.at + offset;
        self.ecs_mut().get::<&mut crate::entity::Position>(e).unwrap().0 = at;
        self.ecs_mut().get::<&mut crate::entity::Velocity>(e).unwrap().0 = Vec3::ZERO;
    }

    fn health(&self, e: hecs::Entity) -> f32 {
        self.ecs().get::<&crate::combat::Health>(e).unwrap().current
    }

    fn sign_in(&mut self, j: usize) -> String {
        let slot = self.joiners[j].slot;
        self.hs.server.players[slot].verified_pubkey = Some(NPUB_KEY);
        let key = self.hs.server.players[slot].pet_owner_key().unwrap();
        assert!(key.starts_with("npub1"), "a pet's owner is the verified npub, never hex: {key}");
        key
    }

    /// The `Attacker` joiner `j` stamps: its slot and connection generation.
    fn attacker(&self, j: usize) -> crate::combat::Attacker {
        let slot = self.joiners[j].slot;
        crate::combat::Attacker::Remote { slot, generation: self.hs.server.players[slot].attach_gen }
    }
}

fn note(out: &protocol::InteractOutcomePacket) -> InteractNote {
    InteractNote::from_wire(out.note)
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
    let la = rig.ecs().get::<&crate::combat::LastAttacker>(cow).expect("stamped").0;
    assert_eq!(la, rig.attacker(0));
    let y = rig.ecs().get::<&crate::entity::Position>(cow).unwrap().0.y;
    assert!(y > y0, "the hit's knockback pops the cow off the floor ({y} > {y0})");
}

/// Out of reach, behind the body (review D2b LOW-2) or a perched parrot
/// (review D2b LOW-3): refused, and nothing changes.
#[test]
fn an_out_of_reach_behind_or_perched_target_is_refused_and_changes_nothing() {
    let mut rig = Rig::new("refuse", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 7.0));
    let max = rig.health(cow);
    rig.place(cow, Vec3::new(0.0, 0.0, 7.0));
    let far = rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    assert!(!rig.joiners[0].inbox.outcome(far).accepted, "7 blocks is out of reach");
    assert_eq!(rig.health(cow), max);
    assert!(rig.ecs().get::<&crate::combat::LastAttacker>(cow).is_err());

    // Two blocks BEHIND the body (it looks along +z): in reach, not ahead.
    rig.place(cow, Vec3::new(0.0, 0.0, -2.0));
    let behind = rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    assert!(!rig.joiners[0].inbox.outcome(behind).accepted, "a mob behind the body is refused");
    assert_eq!(rig.health(cow), max);

    // A parrot perched on someone's shoulder is no target for any gesture.
    let (parrot, pid) = rig.spawn(MobType::Parrot, Vec3::new(0.0, 0.0, 1.5));
    let parrot_max = rig.health(parrot);
    rig.ecs_mut().get::<&mut crate::companion::CompanionData>(parrot).unwrap().state =
        crate::companion::CompanionState::Perch;
    rig.place(parrot, Vec3::new(0.0, 0.0, 1.5));
    let swing = rig.joiners[0].attack(pid, Some(&sword()), true);
    rig.tick(1);
    assert!(!rig.joiners[0].inbox.outcome(swing).accepted, "a perched parrot is refused");
    assert_eq!(rig.health(parrot), parrot_max);
}

/// Review D2b LOW-2 — the server's swing schedule: a second swing in one
/// tick is refused, one 6 ticks after the first is too early even with the
/// 3-tick jitter allowance and one 7 ticks after is taken; and the schedule
/// moves a full cooldown per swing, so a client swinging every 7 ticks is
/// held to the client's average (the next at +7 again is refused, at +10
/// taken).
#[test]
fn the_server_holds_a_joiners_swings_to_the_clients_rate() {
    let mut rig = Rig::new("rate", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    let max = rig.health(cow);
    // Invulnerability frames are the cow's business, not the schedule's:
    // cleared before each swing so every accepted one lands.
    let swing_at = |rig: &mut Rig| -> bool {
        rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
        rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap().invincible_timer = 0;
        let seq = rig.joiners[0].attack(id, None, false);
        rig.tick(1);
        rig.joiners[0].inbox.outcome(seq).accepted
    };
    // Each `swing_at` is read in the tick it ticks; `tick(n)` moves the
    // clock between them.
    let t0 = rig.tick_counter();
    let first = rig.joiners[0].attack(id, None, false);
    let same_tick = rig.joiners[0].attack(id, None, false);
    rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
    rig.tick(1);
    assert!(rig.joiners[0].inbox.outcome(first).accepted);
    assert!(!rig.joiners[0].inbox.outcome(same_tick).accepted, "a second swing in the same tick");
    rig.tick(5); // read at t0 + 6
    assert_eq!(rig.tick_counter(), t0 + 6);
    assert!(!swing_at(&mut rig), "+6: too early, even with the jitter allowance");
    assert!(swing_at(&mut rig), "+7: the cooldown less the jitter");
    rig.tick(6); // read at t0 + 14: 7 after the last
    assert!(!swing_at(&mut rig), "+7 again: the schedule moved a full cooldown, not 7");
    rig.tick(2); // read at t0 + 17
    assert!(swing_at(&mut rig), "+10 after the scheduled time it was due");
    assert_eq!(rig.health(cow), max - 3.0, "three fists, no more");
}

// ── FU1: an action behind a catch-up burst waits; it is never dropped ───────

/// FU1 — a client catching up after a frame hitch runs up to ten ticks a
/// frame and sends one input for each, all at once; the swing it made
/// arrives behind them. The server reads at most
/// `hosted_server::MAX_PACKETS_PER_TICK` of a client's packets a tick and the
/// rest wait for the next, in arrival order: the swing lands, exactly once.
/// (It used to be the 13th packet of the tick and was dropped unanswered.)
#[test]
fn a_swing_behind_a_catch_up_burst_is_applied_exactly_once() {
    let mut rig = Rig::new("burst-swing", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    let max = rig.health(cow);
    let at = rig.at;
    let burst = crate::hosted_server::MAX_PACKETS_PER_TICK as u32 + 2;
    rig.joiners[0].catch_up_burst(at, burst);
    let seq = rig.joiners[0].attack(id, Some(&sword()), false);
    for _ in 0..3 {
        // Held where the swing was aimed until it is read.
        if rig.joiners[0].inbox.outcomes.is_empty() {
            rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
        }
        rig.tick(1);
    }
    let answers: Vec<_> = rig.joiners[0].inbox.outcomes.iter().filter(|o| o.seq == seq).collect();
    assert_eq!(answers.len(), 1, "the swing behind the burst is answered, once");
    assert!(answers[0].accepted, "and confirmed");
    assert_eq!(rig.health(cow), max - sword().attack_damage(), "one sword hit landed");
    assert_eq!(
        rig.hs.server.players[rig.joiners[0].slot].last_input_tick,
        u64::from(burst),
        "every input of the burst was read"
    );
}

/// FU1 — the same for a right-click: milking behind a catch-up burst is
/// applied once (one bucket used, one milk bucket granted).
#[test]
fn an_interaction_behind_a_catch_up_burst_is_applied_exactly_once() {
    let mut rig = Rig::new("burst-milk", 1);
    let slot = rig.joiners[0].slot;
    rig.hs.server.players[slot]
        .inventory
        .set_slot(0, Some(crate::item::ItemStack::new_material(MaterialId::Bucket, 1)));
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 1.5));
    let at = rig.at;
    rig.joiners[0].catch_up_burst(at, crate::hosted_server::MAX_PACKETS_PER_TICK as u32 + 2);
    let seq = rig.joiners[0].interact(id, InteractKind::Milk, Some(&mat(MaterialId::Bucket)));
    for _ in 0..3 {
        if rig.joiners[0].inbox.outcomes.is_empty() {
            rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
        }
        rig.tick(1);
    }
    let answers: Vec<_> = rig.joiners[0].inbox.outcomes.iter().filter(|o| o.seq == seq).collect();
    assert_eq!(answers.len(), 1, "the right-click behind the burst is answered, once");
    assert!(answers[0].accepted && answers[0].consume_held == 1, "and the bucket is used");
    assert_eq!(rig.joiners[0].inbox.granted(MaterialId::MilkBucket), 1, "one milk bucket");
}

/// FU3 (FU1 verify N1) — a host whose game thread stops for 55 s (a long
/// save, a loading screen) finds about 1,100 of each joiner's inputs waiting:
/// the joiner's bridge thread kept reading and queueing them. That used to
/// cross FU1's 1,024-packet bound and disconnect every joiner as a flooder.
/// Now the joiner stays, its queue drains in about a second (a client with
/// more than `CATCH_UP_QUEUE_LEN` waiting is read
/// `CATCH_UP_PACKETS_PER_TICK` a tick), the swing it made mid-stall is
/// answered exactly once, and S1's acknowledgement stays sound: monotonic,
/// never ahead of what arrived, and caught up with the newest input once the
/// host has replayed its missed ticks (it runs up to ten a frame, so a
/// joiner's next input arrives only every few server ticks meanwhile).
#[test]
fn a_55_second_host_stall_keeps_the_joiner_and_answers_its_swing_once() {
    let mut rig = Rig::new("host-stall", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    let max = rig.health(cow);
    let at = rig.at;
    let slot = rig.joiners[0].slot;
    // The stall: the server doesn't tick while the joiner sends 20 inputs a
    // second, with a swing in the middle.
    let stall_inputs = 55 * 20;
    rig.joiners[0].catch_up_burst(at, stall_inputs / 2);
    let seq = rig.joiners[0].attack(id, Some(&sword()), false);
    rig.joiners[0].catch_up_burst(at, stall_inputs - stall_inputs / 2);
    let charged: usize = {
        let input = protocol::InputPacket { tick: 1_000, health: 20.0, ..Default::default() };
        protocol::serialize_packet(protocol::PacketType::ClientInput, &input).len()
            + crate::transport::INBOUND_ENTRY_OVERHEAD
    };
    // The figure transport.rs and Spec 04 §11.2a derive "half an hour or
    // more" from: a bare input is charged about 150 bytes (90 on the wire).
    assert!((120..=200).contains(&charged), "a bare input is charged about 150 bytes, not {charged}");
    assert!(
        charged * stall_inputs as usize * 8 < crate::transport::MAX_INBOUND_BYTES,
        "the stall is far under the byte bound ({charged} B an input)"
    );

    let mut drained_at = None;
    let mut last_ack = 0;
    // Four seconds of the host replaying its missed ticks at twice real time
    // (it runs up to ten a frame): one joiner input every other server tick.
    for t in 0..80u32 {
        if rig.joiners[0].inbox.outcomes.is_empty() {
            rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
        }
        if t % 2 == 0 {
            rig.joiners[0].catch_up_burst(at, 1);
        }
        rig.tick(1);
        let sp = &rig.hs.server.players[slot];
        assert!(sp.last_applied_input >= last_ack, "the acknowledgement never goes back");
        assert!(sp.last_applied_input <= sp.last_input_tick, "nor runs ahead of what arrived");
        assert!(sp.step_credit <= crate::server::MAX_STEP_CREDIT);
        last_ack = sp.last_applied_input;
        if drained_at.is_none() && rig.hs.inbound_len_for_test(slot) == 0 {
            drained_at = Some(t + 1);
        }
    }
    assert!(!rig.hs.slot_is_free(slot), "an honest stall is not a flood: the joiner stays");
    let drained_at = drained_at.expect("the backlog drained");
    assert!(drained_at <= 40, "within about two seconds of ticks: {drained_at}");
    let answers: Vec<_> = rig.joiners[0].inbox.outcomes.iter().filter(|o| o.seq == seq).collect();
    assert_eq!(answers.len(), 1, "the swing made mid-stall is answered, once");
    assert!(answers[0].accepted, "and lands");
    assert_eq!(rig.health(cow), max - sword().attack_damage(), "one sword hit");
    let sp = &rig.hs.server.players[slot];
    assert_eq!(sp.last_input_tick, rig.joiners[0].input_seq, "every input was read");
    assert_eq!(sp.last_applied_input, rig.joiners[0].input_seq, "and the acknowledgement caught up with the newest");
}

#[test]
fn a_dedicated_servers_kill_goes_to_the_killer_alone_and_drops_loot() {
    let mut rig = Rig::new("kill-dedicated", 2);
    let (chicken, id) = rig.spawn(MobType::Chicken, Vec3::new(0.0, 0.0, 2.0));
    rig.ecs_mut().get::<&mut crate::combat::Health>(chicken).unwrap().current = 1.0;
    rig.place(chicken, Vec3::new(0.0, 0.0, 2.0));
    rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    assert!(rig.ecs().get::<&crate::combat::Health>(chicken).is_err(), "dead and swept");
    let kills = &rig.joiners[0].inbox.kills;
    assert_eq!(kills.len(), 1, "the killer is told, once");
    assert_eq!(kills[0].victim, protocol::EntityKind::Chicken);
    assert_eq!(kills[0].reason, protocol::kill_reason::LAST_HIT);
    assert!(rig.joiners[1].inbox.kills.is_empty(), "nobody else is credited");
    let items = rig.ecs().query::<&crate::entity::ItemEntity>().iter().count();
    assert!(items > 0, "the kill's loot drops as world items");
}

/// Review D2b LOW-5 — a death no player's hit caused (lava, a fall, another
/// mob) goes to the nearest living player, as in single-player — and on a
/// server with joiners that may be a joiner: here the only one.
#[test]
fn an_unstamped_death_beside_a_joiner_is_that_joiners_as_the_nearest_player() {
    let mut rig = Rig::new("kill-nearest", 1);
    let (cow, _) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
    rig.tick(1);
    let kills = &rig.joiners[0].inbox.kills;
    assert_eq!(kills.len(), 1);
    assert_eq!(kills[0].reason, protocol::kill_reason::NEAREST, "credited as the nearest, not the hitter");
}

#[test]
fn a_lent_hosts_kill_by_a_joiner_goes_to_that_joiner_and_never_to_the_host() {
    let mut rig = Rig::new_lent("d2b-kill", 1);
    let slot = rig.joiners[0].slot;
    let (chicken, id) = rig.spawn(MobType::Chicken, Vec3::new(0.0, 0.0, 2.0));
    rig.ecs_mut().get::<&mut crate::combat::Health>(chicken).unwrap().current = 1.0;
    rig.place(chicken, Vec3::new(0.0, 0.0, 2.0));
    rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);

    // The host client's death sweep (a lent world's is the client's): the
    // host's own player stands right beside the body, nearer than anyone.
    let host_player = (rig.at + Vec3::new(0.3, 0.0, 2.0), false);
    let deaths = crate::combat::despawn_dead(rig.ecs_mut());
    assert_eq!(deaths.len(), 1);
    let (kind, pos, attacker) = deaths[0];
    assert_eq!(attacker, Some(rig.attacker(0)));
    let credited =
        crate::server::route_client_kill(Some(&mut rig.hs.server), attacker, kind, pos, false, &[host_player]);
    assert_eq!(credited, None, "the host's player is not credited with the joiner's kill");

    rig.tick(1);
    assert_eq!(rig.joiners[0].inbox.kills.len(), 1, "the KillEvent rides the next tick to the joiner");
    assert_eq!(rig.joiners[0].inbox.kills[0].victim, protocol::EntityKind::Chicken);
    assert_eq!(slot, rig.joiners[0].slot);

    // Nobody's hit: the nearest living player — the host's, beside it.
    assert_eq!(
        crate::server::route_client_kill(Some(&mut rig.hs.server), None, kind, pos, false, &[host_player]),
        Some(0)
    );
    // …and the joiner when the host's player is far off (review D2b LOW-5).
    let far_host = (rig.at + Vec3::new(400.0, 0.0, 0.0), false);
    assert_eq!(
        crate::server::route_client_kill(Some(&mut rig.hs.server), None, kind, pos, false, &[far_host]),
        None
    );
    rig.tick(1);
    assert_eq!(rig.joiners[0].inbox.kills.len(), 2);
    assert_eq!(rig.joiners[0].inbox.kills[1].reason, protocol::kill_reason::NEAREST);
}

/// Review D2b MEDIUM-1 — a joiner hits a cow and provokes a Bear, then
/// leaves; the next joiner is given the same slot. The cow later dies with
/// no other player's hit: NOBODY is credited (not the new joiner, not by the
/// nearest-player fallback either — the departed joiner's hit was the last).
/// The Bear's and an angry bee's grudges against the slot are dropped. And
/// the generation tag catches a stamp the release never reached (a kick
/// outside the lend window): it credits nobody either.
#[test]
fn a_reused_slot_inherits_no_kill_credit_and_no_grudge() {
    let mut rig = Rig::new("reuse", 1);
    let slot = rig.joiners[0].slot;
    let first = rig.attacker(0);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
    rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    assert!(rig.health(cow) > 0.0, "hit, not killed");
    assert_eq!(rig.ecs().get::<&crate::combat::LastAttacker>(cow).unwrap().0, first);
    let (bear, _) = rig.spawn(MobType::Bear, Vec3::new(6.0, 0.0, 6.0));
    rig.ecs_mut().get::<&mut crate::bear_ai::BearData>(bear).unwrap().state =
        crate::bear_ai::BearAiState::Aggro { ticks_remaining: 500, attacker_pidx: slot };
    let (bee, _) = rig.spawn(MobType::Bee, Vec3::new(-6.0, 1.0, 6.0));
    let sting_until = rig.tick_counter() + 1_000;
    rig.ecs_mut().get::<&mut crate::bee_ai::BeeData>(bee).unwrap().state =
        crate::bee_ai::BeeAiState::Sting { target_id: slot as u64, until_tick: sting_until };

    rig.joiners[0].leave();
    rig.tick(1);
    assert!(rig.hs.slot_is_free(slot));
    let b = rig.join("B");
    assert_eq!(rig.joiners[b].slot, slot, "the freed slot is reused");
    assert_ne!(rig.attacker(b), first, "a new connection, a new generation");

    assert_eq!(
        rig.ecs().get::<&crate::combat::LastAttacker>(cow).unwrap().0,
        crate::combat::Attacker::Departed,
        "the departed joiner's stamp names no slot"
    );
    assert_eq!(
        rig.ecs().get::<&crate::bear_ai::BearData>(bear).unwrap().state,
        crate::bear_ai::BearAiState::Wander,
        "the Bear forgets the slot's old occupant"
    );
    assert_eq!(
        rig.ecs().get::<&crate::bee_ai::BeeData>(bee).unwrap().state,
        crate::bee_ai::BeeAiState::Idle,
        "so does an angry bee"
    );

    rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
    rig.tick(1);
    assert!(rig.ecs().get::<&crate::combat::Health>(cow).is_err(), "the cow died and was swept");
    assert!(rig.joiners[b].inbox.kills.is_empty(), "the slot's new joiner is not credited");

    // A stamp still naming the old generation (released where the forget
    // pass couldn't reach it): the generation no longer matches.
    let (chicken, _) = rig.spawn(MobType::Chicken, Vec3::new(1.0, 0.0, 2.0));
    rig.ecs_mut().insert_one(chicken, crate::combat::LastAttacker(first)).unwrap();
    rig.ecs_mut().get::<&mut crate::combat::Health>(chicken).unwrap().current = 0.0;
    rig.tick(2);
    assert!(rig.ecs().get::<&crate::combat::Health>(chicken).is_err());
    assert!(rig.joiners[b].inbox.kills.is_empty(), "an old generation's kill credits nobody");
}

/// Review D2b MEDIUM-1 on a lending host: the departed joiner's cow dies in
/// the host client's sweep with the host's own player beside it — the host
/// is not credited either.
#[test]
fn a_departed_joiners_kill_credits_neither_the_host_nor_the_slots_next_joiner() {
    let mut rig = Rig::new_lent("reuse-lent", 1);
    let slot = rig.joiners[0].slot;
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
    rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    rig.joiners[0].leave();
    rig.tick(1);
    let b = rig.join("B");
    assert_eq!(rig.joiners[b].slot, slot);

    rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
    let deaths = crate::combat::despawn_dead(rig.ecs_mut());
    let (kind, pos, attacker) = deaths[0];
    assert_eq!(attacker, Some(crate::combat::Attacker::Departed));
    let host_beside = (pos + Vec3::new(0.3, 0.0, 0.0), false);
    assert_eq!(
        crate::server::route_client_kill(Some(&mut rig.hs.server), attacker, kind, pos, false, &[host_beside]),
        None,
        "the host's player is not credited"
    );
    rig.tick(1);
    assert!(rig.joiners[b].inbox.kills.is_empty(), "nor the slot's new joiner");
}

/// Review D2b MEDIUM-2 — a dedicated server runs no breeding, Leads or pet
/// AI (D4), so feeding, taming, a Lead on a mob or a post and a pet command
/// are refused with "not on this server", and nothing is used or changed.
/// No baby ever comes of it.
#[test]
fn a_dedicated_server_refuses_what_it_does_not_simulate_and_takes_nothing() {
    refuses_what_it_does_not_simulate(Rig::new("not-here", 1));
}

/// Review D2b MEDIUM-2 — likewise a `--no-lend` host: its server owns a
/// second copy of the world, whose animals nothing breeds, leashes or walks
/// (the host client runs those systems on its own copy).
#[test]
fn a_no_lend_host_refuses_what_its_server_does_not_simulate_and_takes_nothing() {
    let rig = Rig::new_no_lend("not-here-owned", 1);
    assert!(!rig.hs.server.animal_life_simulated);
    refuses_what_it_does_not_simulate(rig);
    assert!(Rig::new_lent("here-lent", 0).hs.server.animal_life_simulated, "a lending host's world has it");
}

fn refuses_what_it_does_not_simulate(mut rig: Rig) {
    let key = rig.sign_in(0);
    let (a, ida) = rig.spawn(MobType::Cow, Vec3::new(1.0, 0.0, 1.0));
    let (b, idb) = rig.spawn(MobType::Cow, Vec3::new(-1.0, 0.0, 1.0));
    let (wolf, wolf_id) = rig.spawn(MobType::Wolf, Vec3::new(0.0, 0.0, 1.5));
    let (pet, pet_id) = rig.spawn(MobType::Wolf, Vec3::new(1.5, 0.0, 1.5));
    rig.ecs_mut().get::<&mut crate::wolf::WolfData>(pet).unwrap().ownership.owner_pubkey = key;
    let post = [rig.at.x as i32 + 2, rig.at.y as i32, rig.at.z as i32];
    rig.world_mut().set_block(post[0], post[1], post[2], block::OAK_FENCE_POST);
    let wheat = mat(MaterialId::Wheat);
    let cows_before = rig.ecs().query::<&crate::entity::MobKind>().iter().filter(|(_, k)| k.0 == MobType::Cow).count();
    let asks: [(hecs::Entity, u32, InteractKind, Option<Item>, Vec3); 6] = [
        (a, ida, InteractKind::Feed, Some(wheat.clone()), Vec3::new(1.0, 0.0, 1.0)),
        (b, idb, InteractKind::Feed, Some(wheat), Vec3::new(-1.0, 0.0, 1.0)),
        (wolf, wolf_id, InteractKind::Tame, Some(mat(MaterialId::Bone)), Vec3::new(0.0, 0.0, 1.5)),
        (a, ida, InteractKind::LeadAttach, Some(mat(MaterialId::Lead)), Vec3::new(1.0, 0.0, 1.0)),
        (pet, pet_id, InteractKind::SitToggle, None, Vec3::new(1.5, 0.0, 1.5)),
        (a, 0, InteractKind::LeadToPost { post }, Some(mat(MaterialId::Lead)), Vec3::new(1.0, 0.0, 1.0)),
    ];
    for (e, id, kind, held, at) in asks {
        rig.place(e, at);
        let s = rig.joiners[0].interact(id, kind, held.as_ref());
        rig.tick(1);
        let out = rig.joiners[0].inbox.outcome(s);
        assert!(!out.accepted, "{kind:?} is refused");
        assert_eq!(out.consume_held, 0, "{kind:?} takes nothing");
        assert_eq!(note(out), InteractNote::NotOnThisServer, "{kind:?} says why");
    }
    for cow in [a, b] {
        assert!(rig.ecs().get::<&crate::breeding::InLove>(cow).is_err());
        assert!(rig.ecs().get::<&crate::tether::Tethered>(cow).is_err());
    }
    assert!(crate::tameable::pet_owner_of(rig.ecs(), wolf).is_none(), "still wild");
    assert_ne!(rig.ecs().get::<&crate::wolf::WolfData>(pet).unwrap().state, crate::wolf::WolfAiState::Sit);
    rig.tick(40);
    let cows_after = rig.ecs().query::<&crate::entity::MobKind>().iter().filter(|(_, k)| k.0 == MobType::Cow).count();
    assert_eq!(cows_after, cows_before, "no baby");
    assert_eq!(
        InteractNote::NotOnThisServer.toast(None, false).map(|t| t.0).as_deref(),
        Some("This server doesn't support that yet.")
    );
}

/// On a lending host (whose client breeds the lent world's animals) a
/// joiner's feeding takes: two fed cows breed in the host client's breeding
/// step, and — review D2b B2 — the breed is the FEEDER's: a `Bred` event to
/// the joiner, and no `BreedAnimals` for the host's players.
#[test]
fn feeding_two_cows_on_a_lending_host_breeds_and_credits_the_feeder() {
    let mut rig = Rig::new_lent("feed", 1);
    let (a, ida) = rig.spawn(MobType::Cow, Vec3::new(1.0, 0.0, 1.0));
    let (b, idb) = rig.spawn(MobType::Cow, Vec3::new(-1.0, 0.0, 1.0));
    let wheat = mat(MaterialId::Wheat);
    rig.place(a, Vec3::new(1.0, 0.0, 1.0));
    let s1 = rig.joiners[0].interact(ida, InteractKind::Feed, Some(&wheat));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    rig.place(b, Vec3::new(-1.0, 0.0, 1.0));
    let s2 = rig.joiners[0].interact(idb, InteractKind::Feed, Some(&wheat));
    rig.tick(1);
    for s in [s1, s2] {
        let out = rig.joiners[0].inbox.outcome(s);
        assert!(out.accepted);
        assert_eq!(out.consume_held, 1, "one wheat each");
        assert_eq!(note(out), InteractNote::Fed);
    }
    let feeder = rig.attacker(0);
    for cow in [a, b] {
        let love = *rig.ecs().get::<&crate::breeding::InLove>(cow).expect("in love");
        assert_eq!(love.fed_by, Some(feeder), "the feeder is recorded");
    }
    // The host client's breeding step — `breeding::client_step`, the call
    // `GameState::tick` makes on the world it lends: the two fed adults pair
    // and the baby is born there.
    rig.place(a, Vec3::new(0.5, 0.0, 1.0));
    rig.place(b, Vec3::new(-0.5, 0.0, 1.0));
    let tick = rig.tick_counter();
    let host = rig.host.as_mut().unwrap();
    let born = crate::breeding::client_step(&mut host.ecs, tick, Some(&mut rig.hs.server));
    assert_eq!(born.len(), 1, "two fed adults breed");
    assert_eq!(born[0].kind, MobType::Cow);
    assert!(rig.ecs().get::<&crate::breeding::Baby>(born[0].entity).is_ok(), "the calf is in the lent world");
    assert!(!born[0].credit_here, "a joiner's breed is never the host's");
    rig.tick(1);
    assert!(
        rig.joiners[0].inbox.events.contains(&PlayerEventType::Bred { offspring: protocol::EntityKind::Cow }),
        "the feeder is told"
    );
    // A host player's own feed is the host's.
    assert!(crate::server::route_client_breed(
        Some(&mut rig.hs.server),
        [Some(crate::combat::Attacker::Local(0)), Some(feeder)],
        MobType::Cow
    ));
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
    assert_eq!(note(out), InteractNote::Sheared);
    rig.tick(60);
    assert!(rig.joiners[0].inbox.granted(MaterialId::Wool) >= 1, "the wool arrives as an InventoryGrant");
    // Shorn: a second go is refused until it regrows, and takes nothing.
    rig.place(sheep, Vec3::new(0.0, 0.0, 1.0));
    let again = rig.joiners[0].interact(id, InteractKind::Shear, Some(&shears));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(again);
    assert!(!out.accepted);
    assert_eq!(note(out), InteractNote::WoolGrowing);
}

#[test]
fn milking_swaps_the_bucket_for_a_milk_bucket() {
    let mut rig = Rig::new("milk", 1);
    // C1 — the server's shadow of the joiner's inventory holds the bucket in
    // another slot than the one the request names: owed, it goes from there.
    let slot = rig.joiners[0].slot;
    rig.hs.server.players[slot]
        .inventory
        .set_slot(20, Some(crate::item::ItemStack::new_material(MaterialId::Bucket, 1)));
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 1.5));
    rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
    let seq = rig.joiners[0].interact(id, InteractKind::Milk, Some(&mat(MaterialId::Bucket)));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(seq);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 1, "the bucket is used");
    assert_eq!(rig.joiners[0].inbox.granted(MaterialId::MilkBucket), 1, "a milk bucket is granted");
    // C1 — and the shadow follows: the bucket out, the milk bucket in
    // (C3a-fix-1: once the client says it applied both).
    rig.report();
    let shadow = &rig.hs.server.players[slot].inventory;
    let count = |m: MaterialId| -> u32 {
        shadow.slots_iter().flatten().filter(|s| s.item == mat(m)).map(|s| u32::from(s.count)).sum()
    };
    assert_eq!(count(MaterialId::Bucket), 0, "the shadow paid the bucket, from wherever it was");
    assert_eq!(count(MaterialId::MilkBucket), 1, "and holds the product");
    assert_eq!(rig.hs.server.players[slot].possession.mismatched, 0);

    // C1 — an outcome the shadow can't pay (no bucket left in it) is a
    // possession mismatch: counted, never refused. (Another cow: this one
    // needs time before it can be milked again.)
    let (cow2, id2) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 1.5));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    rig.place(cow2, Vec3::new(0.0, 0.0, 1.5));
    let again = rig.joiners[0].interact(id2, InteractKind::Milk, Some(&mat(MaterialId::Bucket)));
    rig.tick(1);
    assert!(rig.joiners[0].inbox.outcome(again).accepted);
    rig.report();
    assert_eq!(rig.hs.server.players[slot].possession.mismatched, 1);
}

#[test]
fn a_signed_in_joiner_tames_to_its_npub_and_a_guest_cannot() {
    let mut rig = Rig::new_lent("tame", 2);
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
    assert_eq!(note(out), InteractNote::SignInToTame);
    assert!(crate::tameable::pet_owner_of(rig.ecs(), cat).is_none());

    rig.place(cat, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(id, InteractKind::Tame, Some(&treat));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 1);
    assert_eq!(note(out), InteractNote::Tamed);
    assert_eq!(crate::tameable::pet_owner_of(rig.ecs(), cat), Some(key));
}

#[test]
fn sit_toggle_works_on_the_joiners_own_pet_only() {
    let mut rig = Rig::new_lent("sit", 1);
    let key = rig.sign_in(0);
    let (mine, my_id) = rig.spawn(MobType::Wolf, Vec3::new(1.0, 0.0, 1.5));
    let (theirs, their_id) = rig.spawn(MobType::Wolf, Vec3::new(-1.0, 0.0, 1.5));
    rig.ecs_mut().get::<&mut crate::wolf::WolfData>(mine).unwrap().ownership.owner_pubkey = key;
    rig.ecs_mut().get::<&mut crate::wolf::WolfData>(theirs).unwrap().ownership.owner_pubkey =
        "npub1someoneelse".into();
    rig.place(mine, Vec3::new(1.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(my_id, InteractKind::SitToggle, None);
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted);
    assert_eq!(note(out), InteractNote::Sat);
    assert_eq!(rig.ecs().get::<&crate::wolf::WolfData>(mine).unwrap().state, crate::wolf::WolfAiState::Sit);

    rig.place(theirs, Vec3::new(-1.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(their_id, InteractKind::SitToggle, None);
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(!out.accepted);
    assert_eq!(note(out), InteractNote::NotYourPet);
    assert_ne!(rig.ecs().get::<&crate::wolf::WolfData>(theirs).unwrap().state, crate::wolf::WolfAiState::Sit);
}

#[test]
fn a_lead_goes_on_to_the_joiners_body_and_comes_back_off() {
    let mut rig = Rig::new_lent("lead", 1);
    let slot = rig.joiners[0].slot;
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 1.5));
    rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(id, InteractKind::LeadAttach, Some(&mat(MaterialId::Lead)));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 1, "the Lead is used");
    let tether = rig.ecs().get::<&crate::tether::Tethered>(cow).unwrap().target;
    assert_eq!(tether, crate::tether::TetherTarget::Player(slot), "fastened to the joiner");

    rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(id, InteractKind::LeadDetach, None);
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 0);
    assert!(rig.ecs().get::<&crate::tether::Tethered>(cow).is_err());
    assert_eq!(rig.joiners[0].inbox.granted(MaterialId::Lead), 1, "the Lead comes back");
}

/// Review D2b B3 — a joiner's Lead on a fence post ties its own leashed mob
/// to the post (a Lead used), as in single-player; with no leashed mob near,
/// or the post out of reach, nothing happens and nothing is used.
#[test]
fn a_joiners_lead_on_a_fence_post_ties_its_leashed_mob_there() {
    let mut rig = Rig::new_lent("post", 1);
    let lead = mat(MaterialId::Lead);
    let post = [rig.at.x as i32 + 2, rig.at.y as i32, rig.at.z as i32 + 1];
    rig.world_mut().set_block(post[0], post[1], post[2], block::OAK_FENCE_POST);

    // Nothing leashed yet: refused, nothing used.
    let s = rig.joiners[0].interact(0, InteractKind::LeadToPost { post }, Some(&lead));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(!out.accepted && out.consume_held == 0, "no mob of ours to tie");

    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 1.5));
    rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(id, InteractKind::LeadAttach, Some(&lead));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    assert!(rig.joiners[0].inbox.outcome(s).accepted);

    rig.place(cow, Vec3::new(1.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(0, InteractKind::LeadToPost { post }, Some(&lead));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted, "tied to the post");
    assert_eq!(out.consume_held, 1, "a Lead is used, as in single-player");
    assert_eq!(out.kind, Some(InteractKind::LeadToPost { post }));
    assert_eq!(
        rig.ecs().get::<&crate::tether::Tethered>(cow).unwrap().target,
        crate::tether::TetherTarget::Post(post)
    );

    // A post far out of reach of the joiner's server body: refused.
    let far = [post[0] + 12, post[1], post[2]];
    rig.world_mut().set_block(far[0], far[1], far[2], block::OAK_FENCE_POST);
    let s = rig.joiners[0].interact(0, InteractKind::LeadToPost { post: far }, Some(&lead));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(!out.accepted && out.consume_held == 0, "out of reach");
}

/// On a lending host, where feeding is simulated, so the refusals here are
/// reach and the cooldown alone.
#[test]
fn an_interaction_out_of_reach_or_too_soon_is_refused_and_takes_nothing() {
    let mut rig = Rig::new_lent("interact-refuse", 1);
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 8.0));
    rig.place(cow, Vec3::new(0.0, 0.0, 8.0));
    let wheat = mat(MaterialId::Wheat);
    let far = rig.joiners[0].interact(id, InteractKind::Feed, Some(&wheat));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(far);
    assert!(!out.accepted && out.consume_held == 0);
    assert!(rig.ecs().get::<&crate::breeding::InLove>(cow).is_err());

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

/// C3a-2b — an accepted swing wears the weapon in the server's shadow as it
/// wears on the client (`Inventory::use_tool_at` via
/// `joiner_actions::apply_outcome`), by the latest input's hotbar slot. A
/// refused swing wears nothing, and a swing with a sword the shadow's slot
/// doesn't hold is a tallied `wear_mismatch`, never refused.
#[test]
fn an_accepted_swing_wears_the_shadows_sword_as_the_clients_wears() {
    let mut rig = Rig::new("swing-wear", 1);
    let slot = rig.joiners[0].slot;
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    {
        let mut h = rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap();
        h.max = 10_000.0;
        h.current = 10_000.0;
    }
    rig.hs.server.players[slot].inventory.set_slot(0, Some(crate::item::ItemStack { item: sword(), count: 1 }));
    let mut client = crate::inventory::Inventory::new();
    client.set_slot(0, Some(crate::item::ItemStack { item: sword(), count: 1 }));
    let swing = |rig: &mut Rig, held: Option<&Item>| -> bool {
        rig.tick(10); // past the swing schedule
        rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
        rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap().invincible_timer = 0;
        let seq = rig.joiners[0].attack(id, held, false);
        rig.tick(1);
        // C3a-fix-1 — the wear lands on the shadow once the client says it
        // wore its own.
        rig.report();
        rig.joiners[0].inbox.outcome(seq).accepted
    };
    for _ in 0..5 {
        assert!(swing(&mut rig, Some(&sword())));
        client.use_hotbar_tool(0);
    }
    let shadow = |rig: &Rig| rig.hs.server.players[slot].inventory.slot(0).map(|s| s.item.clone());
    assert_eq!(shadow(&rig), client.slot(0).map(|s| s.item.clone()), "five swings wear alike");
    assert_eq!(rig.hs.server.players[slot].possession.wear_mismatch, 0);
    // A bare-hand swing wears nothing and is no mismatch.
    assert!(swing(&mut rig, None));
    assert_eq!(shadow(&rig), client.slot(0).map(|s| s.item.clone()));
    assert_eq!(rig.hs.server.players[slot].possession.wear_mismatch, 0);
    // A different weapon than the slot holds: accepted, tallied, nothing worn.
    let axe = Item::Tool(Tool::new(ToolType::Axe, ToolMaterial::Iron));
    assert!(swing(&mut rig, Some(&axe)));
    assert_eq!(shadow(&rig), client.slot(0).map(|s| s.item.clone()));
    assert_eq!(rig.hs.server.players[slot].possession.wear_mismatch, 1);
}

/// C3a-fix-1 (C-L2) — a select and a swing in one tick, with two of the same
/// sword (slots 0 and 3): the swing goes out before the input that carries
/// the new selection, so the server's latest slot is still 0. The swing
/// carries its own slot (3), and both sides wear the sword there.
#[test]
fn a_select_and_swing_in_one_tick_wears_the_same_sword_on_both_sides() {
    use crate::joiner_actions::{apply_outcome, Asked, Pending};
    let mut rig = Rig::new("swing-slot", 1);
    let slot = rig.joiners[0].slot;
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    {
        let mut h = rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap();
        h.max = 10_000.0;
        h.current = 10_000.0;
    }
    let stack = crate::item::ItemStack { item: sword(), count: 1 };
    let mut inv = crate::inventory::Inventory::new();
    for s in [0, 3] {
        rig.hs.server.players[slot].inventory.set_slot(s, Some(stack.clone()));
        inv.set_slot(s, Some(stack.clone()));
    }
    let mut ui = crate::craft_ui::CraftingUi::new();
    rig.tick(10);
    assert_eq!(rig.hs.server.players[slot].hotbar_slot, 0, "the server's latest slot is 0");
    rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
    rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap().invincible_timer = 0;
    let seq = rig.joiners[0].attack_from(3, id, Some(&sword()), false);
    rig.tick(1);
    let outcome = rig.joiners[0].inbox.outcome(seq).clone();
    assert!(outcome.accepted);
    let request = Pending { kind: Asked::Swing, mob: Some(MobType::Cow), hotbar_slot: 3, held: Some(sword()) };
    apply_outcome(&mut inv, &mut ui, &request, &outcome);
    rig.report();
    let fresh = match sword() {
        Item::Tool(t) => t.durability,
        _ => unreachable!(),
    };
    let worn = |item: Option<&crate::item::ItemStack>| match item.map(|s| &s.item) {
        Some(Item::Tool(t)) => Some(t.durability),
        _ => None,
    };
    let server = &rig.hs.server.players[slot].inventory;
    assert_eq!((worn(inv.slot(0)), worn(inv.slot(3))), (Some(fresh), Some(fresh - 1)), "the client wore slot 3");
    assert_eq!((worn(server.slot(0)), worn(server.slot(3))), (Some(fresh), Some(fresh - 1)), "and so did the server");
}

/// C3a-2a — the sword moved off its hotbar slot by a window op (mirrored on
/// the server) before the swing is accepted: both sides wear it where it now
/// is, the slot first, then anywhere in the 36 (`joiner_actions::where_now`,
/// shared), so the same piece wears and the windows stay equal.
#[test]
fn a_swing_wears_the_weapon_where_it_now_is_on_both_sides() {
    use crate::joiner_actions::{apply_outcome, Asked, Pending};
    let mut rig = Rig::new("swing-moved", 1);
    let slot = rig.joiners[0].slot;
    let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 2.0));
    {
        let mut h = rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap();
        h.max = 10_000.0;
        h.current = 10_000.0;
    }
    let stack = crate::item::ItemStack { item: sword(), count: 1 };
    rig.hs.server.players[slot].inventory.set_slot(0, Some(stack.clone()));
    let mut inv = crate::inventory::Inventory::new();
    inv.set_slot(0, Some(stack));
    let mut ui = crate::craft_ui::CraftingUi::new();
    let mut armour = [None; 4];
    // The player opens its inventory and moves the sword from hotbar slot 0
    // into the bag (slot 9); the ops reach the server.
    ui.open_player_crafting(&inv, &armour);
    for click in [crate::window::WindowClick::Slot { slot: 0, right: false }, crate::window::WindowClick::Slot { slot: 9, right: false }] {
        ui.apply_click(&mut inv, &mut armour, &click, false, Vec3::ZERO, |_| block::AIR);
    }
    for (n, logged) in ui.take_ops(&inv, &armour).into_iter().enumerate() {
        let pkt = protocol::WindowOpPacket {
            op_seq: n as u32 + 1,
            op: logged.op,
            digest: logged.digest,
            events_applied: u32::MAX,
            touched: logged.touched,
            claims: logged.claims,
        };
        rig.joiners[0].client.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
    }
    rig.tick(10); // the ops, and past the swing schedule
    assert!(rig.hs.server.players[slot].inventory.slot(0).is_none(), "the server's copy moved it too");
    assert_eq!(rig.hs.server.players[slot].hotbar_slot, 0, "the swing still comes from slot 0");
    rig.place(cow, Vec3::new(0.0, 0.0, 2.0));
    rig.ecs_mut().get::<&mut crate::combat::Health>(cow).unwrap().invincible_timer = 0;
    let seq = rig.joiners[0].attack(id, Some(&sword()), false);
    rig.tick(1);
    let outcome = rig.joiners[0].inbox.outcome(seq).clone();
    assert!(outcome.accepted);
    let request = Pending { kind: Asked::Swing, mob: Some(MobType::Cow), hotbar_slot: 0, held: Some(sword()) };
    assert!(apply_outcome(&mut inv, &mut ui, &request, &outcome).wear.is_some(), "the client wore it in slot 9");
    rig.report();
    let fresh = match sword() {
        Item::Tool(t) => t.durability,
        _ => unreachable!(),
    };
    let worn = |item: Option<&crate::item::ItemStack>| match item.map(|s| &s.item) {
        Some(Item::Tool(t)) => Some(t.durability),
        _ => None,
    };
    assert_eq!(worn(inv.slot(9)), Some(fresh - 1));
    assert_eq!(worn(rig.hs.server.players[slot].inventory.slot(9)), Some(fresh - 1), "the server wore the same piece");
    assert_eq!(rig.hs.server.players[slot].possession.wear_mismatch, 0);
    let sp = &rig.hs.server.players[slot];
    assert_eq!(
        crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station),
        crate::window::digest_parts(&inv, &armour, &ui.cursor_item, &ui.grid, ui.station()),
        "the windows agree"
    );
}

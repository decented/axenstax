//! Joiner position truth (Spec 04 §5.3, MP step 1).
//!
//! A joiner has ONE position — the one the server simulates from its inputs —
//! and its client agrees with it by prediction + reconciliation. These tests
//! drive a REAL `HostedServer` over the in-process channel transport with a
//! joiner whose client side is the real thing in miniature: its own `World`,
//! its own `Player` stepped by the same `Player::tick`, and the client's
//! `OwnPrediction` folding in every `StateUpdate` it is sent.
//!
//! The riskiest claim comes first: over walking, jumping, walking into a
//! wall, falling off a ledge and swimming, the server's body and the client's
//! prediction from the same inputs agree — so reconciliation never corrects
//! anything when nothing is wrong.

use std::collections::{HashMap, VecDeque};

use glam::Vec3;

use crate::block::{self, BlockRegistry};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::physics::Player;
use crate::play_mode::PlayMode;
use crate::player_intent::PlayerIntent;
use crate::prediction::{OwnPrediction, Reconciled};
use crate::protocol::{self, InputPacket};
use crate::remote_client::{build_join_request_guest, ConnectionState, RemoteClient};
use crate::transport::{ChannelClientTransport, ClientTransport};
use crate::world::World;

use super::joiner_authority::join_guest;

/// 0 local players (a dedicated server), guest-open, no sockets.
fn start_dedicated(tag: &str) -> HostedServer {
    HostedServer::start(
        0,
        format!("position-truth-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts")
}

// ── The arena ────────────────────────────────────────────────────────────────
//
// Built identically into the server's world and the joiner's, so the geometry
// the two simulate against is the same whatever the terrain generator did:
// a walled stone floor (feet at y = 61), a wall across it, a raised platform
// to walk off, and a pool to swim in.

const FLOOR_Y: i32 = 60;
const HALF: i32 = 12;

fn build_arena(world: &mut World) {
    for x in -HALF..=HALF {
        for z in -HALF..=HALF {
            for y in FLOOR_Y + 1..=95 {
                world.set_block(x, y, z, block::AIR);
            }
            world.set_block(x, FLOOR_Y, z, block::STONE);
            // Perimeter wall, so nothing leaves the arena.
            if x.abs() == HALF || z.abs() == HALF {
                for y in FLOOR_Y + 1..=FLOOR_Y + 6 {
                    world.set_block(x, y, z, block::STONE);
                }
            }
        }
    }
    // A wall across x = 6 (z -4..=4), three high.
    for z in -4..=4 {
        for y in FLOOR_Y + 1..=FLOOR_Y + 3 {
            world.set_block(6, y, z, block::STONE);
        }
    }
    // A platform five above the floor (x -10..=-7, z -2..=2).
    for x in -10..=-7 {
        for z in -2..=2 {
            world.set_block(x, FLOOR_Y + 5, z, block::STONE);
        }
    }
    // A pool two deep (x -3..=1, z 5..=9): water at y 61..=62, rim above.
    for x in -3..=1 {
        for z in 5..=9 {
            world.set_block(x, FLOOR_Y + 1, z, block::WATER);
            world.set_block(x, FLOOR_Y + 2, z, block::WATER);
        }
    }
}

/// Where a body put down at `(x, z)` on the arena floor stands.
fn on_floor(x: f32, z: f32) -> Vec3 {
    Vec3::new(x, (FLOOR_Y + 1) as f32, z)
}

// ── The joiner ───────────────────────────────────────────────────────────────

/// One movement input, as the test scripts it.
#[derive(Clone, Copy, Default)]
struct Move {
    forward: f32,
    right: f32,
    jump: bool,
    sprint: bool,
    sneak: bool,
    /// Facing (radians; 0 = −Z, π/2 = −X — `Camera::horizontal_forward`).
    yaw: f32,
}

/// How a test joiner reaches the server.
enum Link {
    /// Raw packets on the channel, numbered by the test itself.
    Raw(ChannelClientTransport),
    /// The real connection: `RemoteClient` stamps each input's sequence
    /// number, and the prediction records under the number it returns —
    /// exactly `network_send_input`'s wiring (`OwnPrediction::send`).
    Client(Box<RemoteClient>),
}

/// A joined client in miniature, wired the way `game_loop.rs` wires it:
/// step the body → send the input → record it; fold in each StateUpdate.
struct Joiner {
    link: Link,
    slot: usize,
    world: World,
    registry: BlockRegistry,
    body: Player,
    prediction: OwnPrediction,
    seq: u64,
    /// StateUpdates in flight: delivered once older than `latency` ticks.
    in_flight: VecDeque<(u64, protocol::StateUpdatePacket)>,
    latency: u64,
    ticks: u64,
    /// What this client predicted after each input, untouched by
    /// reconciliation — the measure of agreement.
    predicted: HashMap<u64, Vec3>,
    /// Largest |server − prediction| at an acknowledged input.
    max_error: f32,
    /// Acknowledged inputs compared.
    compared: usize,
    /// The last input went out riding (see [`Joiner::ride_step`]).
    riding: bool,
    outcomes: Vec<Reconciled>,
}

impl Joiner {
    /// Join `hs` as a guest and put both bodies — the server's and ours — at
    /// rest at `start`, in arenas built into both worlds.
    fn join(hs: &mut HostedServer, start: Vec3, latency: u64) -> Self {
        let (client, slot) = join_guest(hs, "Walker");
        let joiner = Self::joined(hs, Link::Raw(client), slot, start, latency);
        // Drop the join's own traffic.
        while joiner.raw().try_recv_from_server().is_some() {}
        joiner
    }

    /// Join `hs` through a real `RemoteClient` (see [`Link::Client`]).
    fn join_via_remote_client(hs: &mut HostedServer, start: Vec3, latency: u64) -> Self {
        let transport = hs.attach_test_remote();
        let mut rc = RemoteClient::from_transport(
            Box::new(transport),
            build_join_request_guest("Wire", 0),
            None,
        );
        hs.tick();
        rc.poll();
        let ConnectionState::Connected { player_index, .. } = rc.state else {
            panic!("the join did not complete");
        };
        rc.latest_state = None;
        Self::joined(hs, Link::Client(Box::new(rc)), player_index as usize, start, latency)
    }

    fn joined(hs: &mut HostedServer, link: Link, slot: usize, start: Vec3, latency: u64) -> Self {
        build_arena(&mut hs.server.world);
        let mut world = World::new();
        build_arena(&mut world);
        hs.server.players[slot].player = Player::new(start);
        let body = Player::new(start);
        Self {
            link,
            slot,
            world,
            registry: BlockRegistry::new(),
            body,
            prediction: OwnPrediction::new(),
            seq: 0,
            in_flight: VecDeque::new(),
            latency,
            ticks: 0,
            predicted: HashMap::new(),
            max_error: 0.0,
            compared: 0,
            riding: false,
            outcomes: Vec::new(),
        }
    }

    fn raw(&self) -> &ChannelClientTransport {
        match &self.link {
            Link::Raw(client) => client,
            Link::Client(_) => panic!("this joiner talks through a RemoteClient"),
        }
    }

    /// Updates folded in that confirmed the prediction outright.
    fn agreed(&self) -> usize {
        self.outcomes.iter().filter(|r| matches!(r, Reconciled::Agreed)).count()
    }

    fn server_pos(&self, hs: &HostedServer) -> Vec3 {
        hs.server.players[self.slot].player.pos
    }

    /// One client tick: predict, send, record — then one server tick.
    fn step(&mut self, hs: &mut HostedServer, m: Move) {
        self.predict_and_send(m);
        self.server_tick(hs);
    }

    /// The client half of a tick (`tick()` + `network_send_input`).
    fn predict_and_send(&mut self, m: Move) {
        self.seq += 1;
        let mut input = InputPacket {
            // Over a `RemoteClient` this is junk on purpose — far from the
            // connection's own count, as `network_send_input`'s process-wide
            // counter is after an earlier session: the client overwrites it.
            tick: match self.link {
                Link::Raw(_) => self.seq,
                Link::Client(_) => self.seq + 500,
            },
            yaw: m.yaw,
            health: 20.0,
            move_forward: m.forward,
            move_right: m.right,
            jump: m.jump,
            sprint: m.sprint,
            sneak: m.sneak,
            ..Default::default()
        };
        let intent = PlayerIntent::from_input_packet(&input);
        let mut cam = crate::camera::Camera::new(self.body.pos, 1.0);
        cam.yaw = m.yaw;
        self.body.tick(&intent, &cam, &self.world, &self.registry, PlayMode::Survival);
        input.x = self.body.pos.x;
        input.y = self.body.pos.y;
        input.z = self.body.pos.z;
        let seq = match &mut self.link {
            Link::Raw(client) => {
                client.send_to_server(&protocol::serialize_packet(
                    protocol::PacketType::ClientInput,
                    &input,
                ));
                self.prediction.record(self.seq, &input, &self.body);
                self.seq
            }
            Link::Client(rc) => self
                .prediction
                .send(
                    rc,
                    &input,
                    &mut self.body,
                    false,
                    &self.world,
                    &self.registry,
                    PlayMode::Survival,
                )
                .expect("connected: the input went out"),
        };
        self.riding = false;
        self.predicted.insert(seq, self.body.pos);
    }

    /// One tick riding a cart or mount this client simulates on its own
    /// (`game_loop.rs`: no physics step; `apply_riding_follow` pins the body
    /// to the seat at `seat`), steering with `m` — then one server tick.
    fn ride_step(&mut self, hs: &mut HostedServer, m: Move, seat: Vec3) {
        self.body.pos = seat;
        self.body.velocity = Vec3::ZERO;
        self.body.on_ground = false;
        self.seq += 1;
        let input = InputPacket {
            tick: self.seq,
            yaw: m.yaw,
            health: 20.0,
            move_forward: m.forward,
            move_right: m.right,
            jump: m.jump,
            sprint: m.sprint,
            sneak: m.sneak,
            x: seat.x,
            y: seat.y,
            z: seat.z,
            ..Default::default()
        };
        let Link::Client(rc) = &mut self.link else {
            panic!("riding needs a RemoteClient link");
        };
        self.prediction
            .send(rc, &input, &mut self.body, true, &self.world, &self.registry, PlayMode::Survival)
            .expect("connected: the input went out");
        self.riding = true;
        self.server_tick(hs);
    }

    /// One server tick, then deliver whatever StateUpdates have "arrived".
    fn server_tick(&mut self, hs: &mut HostedServer) {
        hs.tick();
        self.ticks += 1;
        match &mut self.link {
            Link::Raw(client) => {
                while let Some(pkt) = client.try_recv_from_server() {
                    if let Some((protocol::PacketType::StateUpdate, payload)) =
                        protocol::deserialize_header(&pkt)
                        && let Ok(state) =
                            protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
                    {
                        self.in_flight.push_back((self.ticks, state));
                    }
                }
            }
            Link::Client(rc) => {
                rc.poll();
                if let Some(state) = rc.latest_state.take() {
                    self.in_flight.push_back((self.ticks, state));
                }
            }
        }
        while self.in_flight.front().is_some_and(|(at, _)| at + self.latency <= self.ticks) {
            let (_, state) = self.in_flight.pop_front().unwrap();
            self.deliver(&state);
        }
        // `OwnPrediction::send` decays the glide over a real client.
        if matches!(self.link, Link::Raw(_)) {
            self.prediction.decay();
        }
    }

    /// `game_loop.rs::network_receive`'s own-body apply.
    fn deliver(&mut self, state: &protocol::StateUpdatePacket) {
        let Some(own) = state.players.iter().find(|p| p.player_index as usize == self.slot) else {
            return;
        };
        let server = Vec3::new(own.x, own.y, own.z);
        if self.riding {
            self.prediction.note_server_pos(server);
            return;
        }
        if let Some(predicted) = self.predicted.get(&state.last_acked_input) {
            self.max_error = self.max_error.max((server - *predicted).length());
            self.compared += 1;
        }
        let r = self.prediction.reconcile(
            state.last_acked_input,
            server,
            &mut self.body,
            &self.world,
            &self.registry,
            PlayMode::Survival,
        );
        self.outcomes.push(r);
    }

    fn corrections(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|r| matches!(r, Reconciled::Smoothed { .. } | Reconciled::Snapped { .. }))
            .count()
    }

    fn run(&mut self, hs: &mut HostedServer, m: Move, ticks: usize) {
        for _ in 0..ticks {
            self.step(hs, m);
        }
    }
}

const EAST: f32 = -std::f32::consts::FRAC_PI_2; // forward = +X
const WEST: f32 = std::f32::consts::FRAC_PI_2; // forward = −X
const SOUTH: f32 = std::f32::consts::PI; // forward = +Z

/// Walk, sprint, jump, hit the wall, walk off the platform, swim — the course
/// both agreement tests run.
fn run_the_course(hs: &mut HostedServer, j: &mut Joiner) {
    let walk_east = Move { forward: 1.0, yaw: EAST, ..Default::default() };
    // Settle onto the floor, then walk and sprint east into the x = 6 wall.
    j.run(hs, Move::default(), 5);
    j.run(hs, walk_east, 10);
    j.run(hs, Move { sprint: true, ..walk_east }, 10);
    j.run(hs, Move { jump: true, ..walk_east }, 12);
    // Pressed against the wall, still walking into it.
    j.run(hs, walk_east, 15);
    assert!(j.body.pos.x < 6.0, "the wall stopped us: {}", j.body.pos);
    // Diagonal along the wall, crouched.
    j.run(hs, Move { forward: 1.0, right: 1.0, sneak: true, yaw: EAST, ..Default::default() }, 8);
    j.run(hs, Move::default(), 5);
}

#[test]
fn prediction_and_the_server_agree_over_walking_jumping_and_a_wall() {
    let mut hs = start_dedicated("agree");
    let mut j = Joiner::join(&mut hs, on_floor(0.5, 0.5), 0);
    run_the_course(&mut hs, &mut j);

    assert!(j.compared > 60, "most inputs were compared: {}", j.compared);
    assert!(j.max_error < 1e-4, "prediction drifted from the server by {}", j.max_error);
    assert_eq!(j.corrections(), 0, "agreement must never correct: {:?}", j.outcomes);
    assert!((j.body.pos - j.server_pos(&hs)).length() < 1e-4);
}

#[test]
fn prediction_and_the_server_agree_falling_off_a_ledge_and_swimming() {
    let mut hs = start_dedicated("ledge");
    // On the platform, facing east, its edge under two blocks ahead.
    let mut j = Joiner::join(&mut hs, Vec3::new(-7.5, (FLOOR_Y + 6) as f32, 0.5), 0);
    let east = Move { forward: 1.0, yaw: EAST, ..Default::default() };
    let south = Move { forward: 1.0, yaw: SOUTH, ..Default::default() };
    j.run(&mut hs, Move::default(), 3);
    j.run(&mut hs, east, 30);
    assert!(
        (j.body.pos.y - (FLOOR_Y + 1) as f32).abs() < 1e-3,
        "walked off the platform and landed on the floor: {}",
        j.body.pos
    );
    // Over the pool's x, then south into it.
    for _ in 0..80 {
        let m = if j.body.pos.x < -1.0 { east } else { south };
        j.step(&mut hs, m);
        if j.body.in_water {
            break;
        }
    }
    assert!(j.body.in_water, "reached the pool: {}", j.body.pos);
    // Swim about: forward, up, down.
    j.run(&mut hs, south, 6);
    j.run(&mut hs, Move { jump: true, ..south }, 8);
    j.run(&mut hs, Move { sneak: true, sprint: true, yaw: WEST, forward: 1.0, ..Default::default() }, 6);
    j.run(&mut hs, Move::default(), 6);

    assert!(j.compared > 50, "{}", j.compared);
    assert!(j.max_error < 1e-4, "prediction drifted from the server by {}", j.max_error);
    assert_eq!(j.corrections(), 0, "agreement must never correct: {:?}", j.outcomes);
}

#[test]
fn agreement_holds_with_inputs_in_flight() {
    // StateUpdates arrive three ticks late, so every one acknowledges an
    // input with three more already predicted past it.
    let mut hs = start_dedicated("latency");
    let mut j = Joiner::join(&mut hs, on_floor(0.5, 0.5), 3);
    run_the_course(&mut hs, &mut j);

    assert!(j.compared > 60, "{}", j.compared);
    assert!(j.max_error < 1e-4, "prediction drifted from the server by {}", j.max_error);
    assert_eq!(j.corrections(), 0, "{:?}", j.outcomes);
}

#[test]
fn the_real_client_records_each_input_under_the_number_it_went_out_with() {
    // The whole client path: `RemoteClient::send_input` numbers each input on
    // the wire (from 1, per connection) and `OwnPrediction::send` records it
    // under THAT number — so the server's acknowledgements line up with the
    // records. The test's own `tick` on each packet is junk the client
    // overwrites. Zero corrections alone proves nothing (an ack matching no
    // record is skipped, not corrected): the acks must also confirm records.
    let mut hs = start_dedicated("wire");
    let mut first = Joiner::join_via_remote_client(&mut hs, on_floor(0.5, 0.5), 2);
    run_the_course(&mut hs, &mut first);
    assert!(first.compared > 60, "acks matched recorded inputs: {}", first.compared);
    assert!(first.agreed() > 60, "acks confirmed the prediction: {:?}", first.outcomes);
    assert!(first.max_error < 1e-4, "prediction drifted from the server by {}", first.max_error);
    assert_eq!(first.corrections(), 0, "{:?}", first.outcomes);

    // Leave, and join again in the same process: a new connection counting
    // from 1 again, the same prediction (`GameState` keeps one), and the
    // caller's own counter still running on from the first session.
    let (prediction, seq) = (std::mem::take(&mut first.prediction), first.seq);
    if let Link::Client(rc) = &mut first.link {
        rc.disconnect();
    }
    drop(first);
    hs.tick();
    hs.tick();
    let mut second = Joiner::join_via_remote_client(&mut hs, on_floor(0.5, 0.5), 2);
    second.prediction = prediction;
    second.seq = seq;
    run_the_course(&mut hs, &mut second);
    assert!(second.compared > 60, "{}", second.compared);
    assert!(second.agreed() > 60, "the second session's acks match too: {:?}", second.outcomes);
    assert!(second.max_error < 1e-4, "drifted by {}", second.max_error);
    assert_eq!(second.corrections(), 0, "{:?}", second.outcomes);
}

#[test]
fn a_ride_leaves_the_server_body_where_it_began_and_getting_off_rejoins_it() {
    // A ride is this client's own sim (BRIDGE until the server simulates
    // rides). Its steering keys — forward, jump, sprint, sneak — must not walk
    // the server's body away (or into a pit) meanwhile; on getting off the
    // joiner is put straight back on the server's body, and from there the
    // prediction agrees with the server again.
    let mut hs = start_dedicated("ride");
    let mut j = Joiner::join_via_remote_client(&mut hs, on_floor(-3.5, -9.5), 2);
    j.run(&mut hs, Move::default(), 5);
    j.run(&mut hs, Move { forward: 1.0, yaw: SOUTH, ..Default::default() }, 5);
    j.run(&mut hs, Move::default(), 5);
    let got_on = j.server_pos(&hs);
    assert!((j.body.pos - got_on).length() < 1e-4);

    let steer = Move { forward: 1.0, right: 0.5, jump: true, sprint: true, sneak: true, yaw: EAST };
    for i in 1..=40 {
        j.ride_step(&mut hs, steer, got_on + Vec3::new(0.25 * i as f32, 0.6, 0.0));
    }
    assert!(
        (j.server_pos(&hs) - got_on).length() < 1e-4,
        "the server's body stood still: {got_on} -> {}",
        j.server_pos(&hs)
    );
    assert!((j.body.pos - got_on).length() > 5.0, "the ride carried the joiner: {}", j.body.pos);

    // Off, and walk on.
    let (before, agreed_before) = (j.outcomes.len(), j.agreed());
    j.step(&mut hs, Move { forward: 1.0, yaw: EAST, ..Default::default() });
    assert!((j.body.pos - got_on).length() < 1e-4, "back on the server's body: {}", j.body.pos);
    j.run(&mut hs, Move { forward: 1.0, yaw: SOUTH, ..Default::default() }, 15);
    j.run(&mut hs, Move::default(), 5);
    let after = &j.outcomes[before..];
    assert!(
        !after.iter().any(|r| matches!(r, Reconciled::Smoothed { .. } | Reconciled::Snapped { .. })),
        "getting off corrected nothing: {after:?}"
    );
    assert!(j.agreed() > agreed_before + 15, "{after:?}");
    assert!((j.body.pos - j.server_pos(&hs)).length() < 1e-4);
}

#[test]
fn a_server_teleport_snaps_the_joiner_to_the_server_body() {
    let mut hs = start_dedicated("teleport");
    let mut j = Joiner::join(&mut hs, on_floor(0.5, 0.5), 2);
    let walk = Move { forward: 1.0, yaw: SOUTH, ..Default::default() };
    j.run(&mut hs, Move::default(), 5);
    j.run(&mut hs, walk, 5);
    assert_eq!(j.corrections(), 0);

    // The server moves the body five blocks west (as a respawn or an
    // operator teleport would).
    hs.server.players[j.slot].player.pos += Vec3::new(-5.0, 0.0, 0.0);
    j.run(&mut hs, walk, 4);
    assert!(
        j.outcomes.iter().any(|r| matches!(r, Reconciled::Snapped { .. })),
        "a 5-block disagreement snaps: {:?}",
        j.outcomes
    );
    assert_eq!(j.prediction.visual_offset(), Vec3::ZERO, "a snap doesn't glide");

    // Once the late updates catch up the two agree again.
    let corrections_after_snap = j.corrections();
    j.run(&mut hs, walk, 10);
    assert_eq!(j.corrections(), corrections_after_snap, "{:?}", j.outcomes);
    let server = j.server_pos(&hs);
    j.run(&mut hs, Move::default(), 4);
    assert!(j.body.pos.x < -4.0, "the joiner is where the server put it: {}", j.body.pos);
    assert!((j.body.pos - j.server_pos(&hs)).length() < 1e-3, "{} vs {server}", j.body.pos);
}

#[test]
fn a_small_server_correction_glides_without_a_snap() {
    let mut hs = start_dedicated("nudge");
    let mut j = Joiner::join(&mut hs, on_floor(0.5, 0.5), 2);
    let walk = Move { forward: 1.0, yaw: SOUTH, ..Default::default() };
    j.run(&mut hs, Move::default(), 5);
    j.run(&mut hs, walk, 5);

    hs.server.players[j.slot].player.pos += Vec3::new(0.3, 0.0, 0.0);
    let before = j.outcomes.len();
    j.step(&mut hs, walk);
    j.step(&mut hs, walk);
    j.step(&mut hs, walk);
    let new: Vec<_> = j.outcomes[before..].to_vec();
    assert!(
        new.iter().any(|r| matches!(r, Reconciled::Smoothed { .. })),
        "a 0.3-block disagreement glides: {new:?}"
    );
    assert!(!j.outcomes.iter().any(|r| matches!(r, Reconciled::Snapped { .. })));
    assert!(j.prediction.visual_offset().length() > 0.0, "the camera is still easing over");

    j.run(&mut hs, Move::default(), 15);
    assert_eq!(j.prediction.visual_offset(), Vec3::ZERO, "the glide finished");
    assert!((j.body.pos - j.server_pos(&hs)).length() < 1e-3);
}

#[test]
fn bunched_inputs_are_each_simulated_with_their_own_heading() {
    // Two inputs reach the server in one tick, facing different ways. The
    // server must step each with the heading it was predicted with — not
    // both with the later packet's — or the joiner is pulled sideways.
    let mut hs = start_dedicated("bunched");
    let mut j = Joiner::join(&mut hs, on_floor(0.5, 0.5), 0);
    j.run(&mut hs, Move::default(), 5);
    for pair in [[EAST, SOUTH], [SOUTH, WEST], [EAST, SOUTH]] {
        for yaw in pair {
            j.predict_and_send(Move { forward: 1.0, yaw, ..Default::default() });
        }
        // Both arrived before this tick; the next one simulates the second.
        j.server_tick(&mut hs);
        j.server_tick(&mut hs);
    }
    j.run(&mut hs, Move::default(), 6);
    assert!(j.compared > 10, "{}", j.compared);
    assert!(j.max_error < 1e-4, "bunched inputs drifted by {}", j.max_error);
    assert_eq!(j.corrections(), 0, "{:?}", j.outcomes);
}

#[test]
fn a_client_frame_hitch_catches_up_without_a_correction() {
    // The joiner's frame stalls for half a second: the server ticks ten times
    // with nothing from it, then the client runs its ten missed ticks in one
    // frame and sends ten inputs at once. All ten are simulated, in order,
    // so its prediction is never corrected.
    let mut hs = start_dedicated("hitch");
    let mut j = Joiner::join_via_remote_client(&mut hs, on_floor(3.5, -10.5), 1);
    let south = Move { forward: 1.0, yaw: SOUTH, ..Default::default() };
    j.run(&mut hs, Move::default(), 5);
    j.run(&mut hs, south, 5);
    for _ in 0..2 {
        for _ in 0..10 {
            j.server_tick(&mut hs);
        }
        // All ten reach the server before its next tick.
        for _ in 0..10 {
            j.predict_and_send(Move { sprint: true, ..south });
        }
        j.server_tick(&mut hs);
        j.run(&mut hs, south, 6);
        j.run(&mut hs, Move::default(), 6);
    }
    assert!(j.agreed() > 20, "{:?}", j.outcomes);
    assert!(j.max_error < 1e-4, "the hitch cost the server steps: off by {}", j.max_error);
    assert_eq!(j.corrections(), 0, "{:?}", j.outcomes);
    assert!((j.body.pos - j.server_pos(&hs)).length() < 1e-4);
}

#[test]
fn a_burst_past_the_packet_budget_catches_up_without_a_correction() {
    // FU1 — more inputs than the server reads a tick reach it at once (a
    // network stall, or two catch-up frames landing together). The ones past
    // `MAX_PACKETS_PER_TICK` wait for the next tick instead of being dropped,
    // so every one is still simulated, in order, on the steps the stall
    // banked: the prediction is never corrected, and the acknowledgement
    // reaches the newest input.
    let mut hs = start_dedicated("burst-past-budget");
    let mut j = Joiner::join_via_remote_client(&mut hs, on_floor(3.5, -10.5), 1);
    let south = Move { forward: 1.0, yaw: SOUTH, ..Default::default() };
    j.run(&mut hs, Move::default(), 5);
    j.run(&mut hs, south, 5);
    let burst = crate::hosted_server::MAX_PACKETS_PER_TICK + 2;
    for _ in 0..burst {
        j.server_tick(&mut hs);
    }
    for _ in 0..burst {
        j.predict_and_send(Move { sprint: true, ..south });
    }
    j.server_tick(&mut hs);
    j.run(&mut hs, south, 6);
    j.run(&mut hs, Move::default(), 6);
    assert!(j.agreed() > 10, "{:?}", j.outcomes);
    assert!(j.max_error < 1e-4, "the burst cost the server steps: off by {}", j.max_error);
    assert_eq!(j.corrections(), 0, "{:?}", j.outcomes);
    assert!((j.body.pos - j.server_pos(&hs)).length() < 1e-4);
}

#[test]
fn state_updates_acknowledge_the_last_input_the_server_applied() {
    let mut hs = start_dedicated("ack");
    let mut j = Joiner::join(&mut hs, on_floor(0.5, 0.5), 0);
    j.run(&mut hs, Move::default(), 3);
    assert_eq!(hs.server.players[j.slot].last_applied_input, 3);
    // Three inputs arrive at once: the next tick applies one of them, and
    // says so — not the newest one received.
    for _ in 0..3 {
        j.seq += 1;
        let input = InputPacket { tick: j.seq, health: 20.0, ..Default::default() };
        j.raw()
            .send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }
    // Nothing banked (`ServerPlayer::step_credit`): one step this tick.
    hs.server.players[j.slot].step_credit = 0;
    hs.tick();
    let mut acks = Vec::new();
    while let Some(pkt) = j.raw().try_recv_from_server() {
        if let Some((protocol::PacketType::StateUpdate, payload)) = protocol::deserialize_header(&pkt)
            && let Ok(s) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
        {
            acks.push(s.last_acked_input);
        }
    }
    assert_eq!(acks, vec![4], "one input applied this tick, acknowledged by its number");
}

// ── Spawn ────────────────────────────────────────────────────────────────────

fn join_accept(client: &ChannelClientTransport) -> protocol::JoinAcceptPacket {
    while let Some(pkt) = client.try_recv_from_server() {
        if let Some((protocol::PacketType::JoinAccept, payload)) = protocol::deserialize_header(&pkt)
            && let Ok(a) = protocol::safe_deserialize::<protocol::JoinAcceptPacket>(payload)
        {
            return a;
        }
    }
    panic!("no JoinAccept");
}

fn join_and_accept(hs: &mut HostedServer) -> (usize, Vec3) {
    let client = hs.attach_test_remote();
    let req = crate::remote_client::build_join_request_guest("Spawner", 0);
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
    hs.tick();
    let accept = join_accept(&client);
    let joined = crate::remote_client::JoinedWorld::from_accept(&accept);
    (accept.player_index as usize, joined.spawn.expect("finite spawn"))
}

fn is_standing_spot(world: &World, at: Vec3) -> bool {
    let (x, y, z) = (at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
    world.get_block(x, y - 1, z) != block::AIR
        && world.get_block(x, y, z) == block::AIR
        && world.get_block(x, y + 1, z) == block::AIR
}

#[test]
fn the_joiner_starts_exactly_where_the_server_body_starts() {
    // A host with its own player: the joiner's spawn (what its client places
    // itself at, from JoinAccept) IS the server body's position and respawn
    // point — one value, decided once.
    let mut hs = HostedServer::start(
        1,
        format!("position-truth-host-spawn-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("host starts");
    let (slot, spawn) = join_and_accept(&mut hs);
    let sp = &hs.server.players[slot];
    assert_eq!(sp.player.pos, spawn);
    assert_eq!(sp.spawn_pos, spawn);
}

#[test]
fn a_joiner_beside_a_far_travelled_host_can_move() {
    // The host walked far from where hosting began, out of the columns the
    // server loaded then. The joiner's spawn columns load with it, so its body
    // moves instead of standing frozen at the terrain edge.
    let mut hs = HostedServer::start(
        1,
        format!("position-truth-far-host-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("host starts");
    hs.server.players[0].player.pos = Vec3::new(400.5, 95.0, 8.5);
    assert!(!hs.server.loaded_columns.contains(&(25, 0)), "the host is beyond the loaded area");
    let (client, slot) = join_guest(&mut hs, "Far");
    let spawn = hs.server.players[slot].player.pos;
    let col = ((spawn.x.floor() as i32).div_euclid(16), (spawn.z.floor() as i32).div_euclid(16));
    assert!(hs.server.loaded_columns.contains(&col), "spawn column {col:?} loaded");
    for seq in 1..=20 {
        let input = InputPacket {
            tick: seq,
            yaw: EAST,
            move_forward: 1.0,
            health: 20.0,
            ..Default::default()
        };
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        hs.tick();
    }
    let at = hs.server.players[slot].player.pos;
    assert!(at.x > spawn.x + 1.0 || at.y < spawn.y - 1.0, "the body moved: {spawn} -> {at}");
}

#[test]
fn a_dedicated_server_spawns_joiners_on_the_surface_not_in_the_sky() {
    let mut hs = start_dedicated("surface");
    let (slot, spawn) = join_and_accept(&mut hs);
    assert_ne!(spawn, Vec3::new(0.5, 80.0, 0.5), "no longer the fixed sky spawn");
    assert_eq!(hs.server.players[slot].player.pos, spawn);
    assert_eq!(hs.server.players[slot].spawn_pos, spawn);
    assert!(is_standing_spot(&hs.server.world, spawn), "feet on ground, head clear: {spawn}");
    // The rule a fresh single-player world places its player by.
    assert_eq!(
        spawn,
        crate::chunk_stream::world_spawn_point(&hs.server.world, &hs.server.biome_gen)
    );
    // A second joiner starts at the world spawn too, not beside the first.
    let (_, second) = join_and_accept(&mut hs);
    assert_eq!(second, spawn);
}

#[test]
fn the_world_spawn_loads_the_columns_it_searches() {
    // A server whose loaded area doesn't cover the origin (a saved world
    // whose players were far away) generates the spawn columns first.
    let mut server = crate::server::GameServer::new(0, "position-truth-unloaded".into(), 7);
    assert!(server.loaded_columns.is_empty());
    let spawn = server.world_spawn();
    for cx in -1..=1 {
        for cz in -1..=1 {
            assert!(server.loaded_columns.contains(&(cx, cz)), "column ({cx}, {cz}) loaded");
        }
    }
    assert!(is_standing_spot(&server.world, spawn), "{spawn}");
}

// ── The edge of the server's terrain ─────────────────────────────────────────

#[test]
fn on_a_lan_host_a_joiner_goes_on_past_the_terrain_the_server_started_with() {
    // A LAN host's server loaded the area round where hosting began. Its
    // joiners' bodies must not meet an invisible wall at that area's edge:
    // the columns ahead of them load as they go.
    let mut hs = HostedServer::start(
        1,
        format!("position-truth-lan-roam-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("host starts");
    let (client, slot) = join_guest(&mut hs, "Rover");
    let first_area = hs.server.loaded_columns.clone();
    // Flying above the world's tallest terrain, so nothing but the edge of
    // the server's terrain could stop it.
    hs.server.play_mode = PlayMode::Creative;
    let sp = &mut hs.server.players[slot];
    sp.player.pos.y = 100.0;
    sp.player.flying = true;
    let start = sp.player.pos;
    for seq in 1..=200 {
        let input = InputPacket {
            tick: seq,
            yaw: EAST,
            move_forward: 1.0,
            sprint: true,
            health: 20.0,
            ..Default::default()
        };
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        hs.tick();
    }
    let at = hs.server.players[slot].player.pos;
    let col = ((at.x.floor() as i32).div_euclid(16), (at.z.floor() as i32).div_euclid(16));
    assert!(at.x > start.x + 180.0, "flew on east: {start} -> {at}");
    assert!(!first_area.contains(&col), "beyond the area hosting began with: {col:?}");
    assert!(hs.server.loaded_columns.contains(&col), "its column loaded: {col:?}");
}

#[test]
fn a_body_pushing_at_the_edge_in_mid_air_still_comes_down() {
    // The edge stops a body sideways only. Reverting its height too left a
    // joiner who jumped or fell against the edge hanging in mid-air for as
    // long as it pushed. (A dedicated server: no refill. Its streamer is off
    // here so the edge stays put — the guard is the backstop for a body that
    // outruns `SERVER_STREAM_BUDGET`, Phase B1.)
    let mut hs = start_dedicated("edge-fall");
    hs.server.column_streamer = None;
    let (client, slot) = join_guest(&mut hs, "Faller");
    let edge_cx = (0..64).find(|cx| !hs.server.loaded_columns.contains(&(*cx, 0))).unwrap();
    let edge_x = edge_cx * 16;
    let y = 84;
    for x in edge_x - 4..edge_x {
        for z in 7..=9 {
            hs.server.world.set_block(x, y, z, block::STONE);
            for above in y + 1..=y + 7 {
                hs.server.world.set_block(x, above, z, block::AIR);
            }
        }
    }
    // Three blocks above the runway, right at the edge.
    let start = Vec3::new(edge_x as f32 - 0.05, (y + 4) as f32, 8.5);
    hs.server.players[slot].player = Player::new(start);
    for seq in 1..=30 {
        let input = InputPacket {
            tick: seq,
            yaw: EAST,
            move_forward: 1.0,
            health: 20.0,
            ..Default::default()
        };
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        hs.tick();
    }
    let at = hs.server.players[slot].player.pos;
    assert!((at.y - (y + 1) as f32).abs() < 1e-3, "came down onto the runway: {at}");
    assert!(at.x < edge_x as f32, "still held at the edge: {at}");
}

#[test]
fn a_joiner_body_stops_at_the_edge_of_the_servers_terrain() {
    // Beyond the columns the server generated its world is empty air. The
    // body must stop at the edge, not walk out and fall through the world —
    // the joiner's client now follows it. The dedicated streamer is off so the
    // edge stays put: the guard is the backstop for a body that outruns the
    // per-tick streaming budget (Phase B1).
    let mut hs = start_dedicated("edge");
    hs.server.column_streamer = None;
    let (client, slot) = join_guest(&mut hs, "Edge");
    let edge_cx = (0..64).find(|cx| !hs.server.loaded_columns.contains(&(*cx, 0))).unwrap();
    let edge_x = edge_cx * 16;
    // A stone runway up to the last loaded cell, clear above.
    let y = 88;
    for x in edge_x - 10..edge_x {
        for z in 7..=9 {
            hs.server.world.set_block(x, y, z, block::STONE);
            for above in y + 1..=y + 3 {
                hs.server.world.set_block(x, above, z, block::AIR);
            }
        }
    }
    let start = Vec3::new(edge_x as f32 - 6.5, (y + 1) as f32, 8.5);
    hs.server.players[slot].player = Player::new(start);
    for seq in 1..=60 {
        let input = InputPacket {
            tick: seq,
            yaw: EAST,
            move_forward: 1.0,
            sprint: true,
            health: 20.0,
            ..Default::default()
        };
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        hs.tick();
    }
    let at = hs.server.players[slot].player.pos;
    assert!(at.x < edge_x as f32 && at.x > edge_x as f32 - 1.0, "stopped at the edge: {at}");
    assert!((at.y - start.y).abs() < 1e-3, "still on the runway, not falling: {at}");
    assert!(!hs.server.loaded_columns.contains(&(edge_cx, 0)), "the test walked at a real edge");
}

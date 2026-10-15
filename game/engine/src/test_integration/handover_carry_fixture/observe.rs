//! Fixed diagnostic metadata around the unchanged real client transport.
use super::{Seat, settle, step};
use crate::{
    action_identity::{ActionFate, ActionHeader},
    action_wire::{ActionIntent, ActionPacket},
    protocol::{self, PacketType},
    quest_wire::{QuestNote, QuestOperation, QuestResult, QuestStatus, QuestView, QuestViewPush},
    test_game_harness::HeadlessGame,
    transport::{ChannelClientTransport, ClientTransport, Packet},
};
use glam::Vec3;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

const LIMIT: usize = 256;
const BYTE_LIMIT: usize = 512 * 1024;
const DECODE_FAILED: u8 = 1;
const CAPACITY_FAILED: u8 = 2;
const POISONED: u8 = 4;

#[derive(Clone, Copy)]
struct ViewMeta {
    target: u32,
    npc: [u8; 16],
    incarnation: u64,
    revision: u64,
    status: QuestStatus,
    profession: crate::villager::Profession,
    terms: Option<([u8; 16], crate::quest::QuestFlavour)>,
}
impl From<&QuestView> for ViewMeta {
    fn from(v: &QuestView) -> Self {
        Self {
            target: v.target,
            npc: v.npc,
            incarnation: v.incarnation,
            revision: v.revision,
            status: v.status,
            profession: v.profession,
            terms: v.terms.as_ref().map(|t| (t.id, t.flavour)),
        }
    }
}
impl fmt::Debug for ViewMeta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View")
            .field("target", &self.target)
            .field("npc", &self.npc)
            .field("incarnation", &self.incarnation)
            .field("revision", &self.revision)
            .field("status", &self.status)
            .field("profession", &self.profession)
            .field("terms", &self.terms)
            .finish()
    }
}
#[derive(Clone, Copy, Debug)]
struct InputMeta {
    seq: u64,
    position: Vec3,
    yaw: f32,
    pitch: f32,
    applied: u32,
}
impl From<&protocol::InputPacket> for InputMeta {
    fn from(p: &protocol::InputPacket) -> Self {
        Self { seq: p.tick, position: Vec3::new(p.x, p.y, p.z), yaw: p.yaw, pitch: p.pitch, applied: p.events_applied }
    }
}
#[derive(Clone)]
enum Meta {
    Input(InputMeta),
    Quest(ActionHeader, u32, u32, u64, u32, QuestOperation),
    Result(ActionHeader, u32, ActionFate, QuestNote, u32, u32, Option<ViewMeta>),
    Push([u8; 16], u32, u32, u32, u32, u64, u32, Option<ViewMeta>),
}
impl fmt::Debug for Meta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(p) => f.debug_tuple("Input").field(&(p.seq, p.position, p.yaw, p.pitch, p.applied)).finish(),
            Self::Quest(h, respawns, target, incarnation, before, op) => {
                f.debug_tuple("Quest").field(&(h, respawns, target, incarnation, before, op)).finish()
            }
            Self::Result(h, respawns, fate, note, event, fence, view) => {
                f.debug_tuple("Result").field(&(h, respawns, fate, note, event, fence, view)).finish()
            }
            Self::Push(session, epoch, life, respawns, target, incarnation, event, view) => f
                .debug_tuple("Push")
                .field(&(session, epoch, life, respawns, target, incarnation, event, view))
                .finish(),
        }
    }
}
#[derive(Clone)]
struct Event {
    ordinal: u64,
    bytes: usize,
    hash: [u8; 32],
    meta: Meta,
}
struct State {
    active: bool,
    errors: u8,
    ordinal: u64,
    sent: u64,
    received: u64,
    last_input: Option<(u64, InputMeta)>,
    events: Vec<Event>,
}
#[derive(Clone)]
pub(in crate::test_integration) struct Observation(Arc<Mutex<State>>);
pub(super) struct ObservedTransport {
    inner: ChannelClientTransport,
    trace: Observation,
}
pub(super) fn wrap(inner: ChannelClientTransport) -> (ObservedTransport, Observation) {
    let mut events = Vec::new();
    let errors = if events.try_reserve_exact(LIMIT).is_err()
        || events.capacity().checked_mul(std::mem::size_of::<Event>()).is_none_or(|bytes| bytes > BYTE_LIMIT)
    {
        CAPACITY_FAILED
    } else {
        0
    };
    let trace = Observation(Arc::new(Mutex::new(State {
        active: false,
        errors,
        ordinal: 0,
        sent: 0,
        received: 0,
        last_input: None,
        events,
    })));
    (ObservedTransport { inner, trace: trace.clone() }, trace)
}
impl Observation {
    fn state(&self) -> MutexGuard<'_, State> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.errors |= POISONED;
                state
            }
        }
    }
    fn begin(&self) -> Option<(u64, InputMeta)> {
        let mut s = self.state();
        s.events.clear();
        s.active = true;
        s.last_input
    }
    fn observe(&self, bytes: &[u8], outgoing: bool) {
        let mut s = self.state();
        s.ordinal = s.ordinal.saturating_add(1);
        if outgoing {
            s.sent = s.sent.saturating_add(1);
        } else {
            s.received = s.received.saturating_add(1);
        }
        let Some((tag, body)) = protocol::deserialize_header(bytes) else {
            s.errors |= DECODE_FAILED;
            return;
        };
        let decoded = match (outgoing, tag) {
            (true, PacketType::ClientInput) => protocol::safe_deserialize::<protocol::InputPacket>(body)
                .map(|p| Meta::Input(InputMeta::from(&p)))
                .map_err(|_| ()),
            (true, PacketType::Action) if s.active => match ActionPacket::decode(body) {
                Ok(p) => match p.intent {
                    ActionIntent::Quest(q) => {
                        Ok(Meta::Quest(p.action, q.actor_respawns, q.target, q.incarnation, q.before, q.operation))
                    }
                    _ => return,
                },
                Err(_) => Err(()),
            },
            (false, PacketType::QuestResult) if s.active => QuestResult::decode(body)
                .map(|p| {
                    Meta::Result(
                        p.action,
                        p.actor_respawns,
                        p.fate,
                        p.note,
                        p.window_event,
                        p.included_events,
                        p.view.as_ref().map(ViewMeta::from),
                    )
                })
                .map_err(|_| ()),
            (false, PacketType::QuestView) if s.active => {
                protocol::safe_deserialize::<QuestViewPush>(body).map_err(|_| ()).and_then(|p| {
                    p.validate().map_err(|_| ()).map(|()| {
                        Meta::Push(
                            p.session,
                            p.epoch,
                            p.life,
                            p.actor_respawns,
                            p.target,
                            p.incarnation,
                            p.window_event,
                            p.view.as_ref().map(ViewMeta::from),
                        )
                    })
                })
            }
            _ => return,
        };
        let meta = match decoded {
            Ok(meta) => meta,
            Err(()) => {
                s.errors |= DECODE_FAILED;
                return;
            }
        };
        if let Meta::Input(input) = &meta {
            s.last_input = Some((s.ordinal, *input));
        }
        if !s.active {
            return;
        }
        if s.events.len() >= LIMIT || s.events.len() >= s.events.capacity() {
            s.errors |= CAPACITY_FAILED;
            return;
        }
        let ordinal = s.ordinal;
        s.events.push(Event { ordinal, bytes: bytes.len(), hash: Sha256::digest(bytes).into(), meta });
    }
}
impl ClientTransport for ObservedTransport {
    fn send_to_server(&self, bytes: &[u8]) {
        self.inner.send_to_server(bytes);
        self.trace.observe(bytes, true);
    }
    fn try_recv_from_server(&self) -> Option<Packet> {
        let packet = self.inner.try_recv_from_server()?;
        self.trace.observe(&packet, false);
        Some(packet)
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.inner.channel_binding()
    }
    fn ws_host(&self) -> Option<String> {
        self.inner.ws_host()
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

struct TargetMeta {
    present: bool,
    position: Option<Vec3>,
    centre: Option<Vec3>,
    health: Option<(f32, bool)>,
    kind: Option<crate::mob::MobType>,
    profession: Option<crate::villager::Profession>,
}
impl fmt::Debug for TargetMeta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("present", &self.present)
            .field("position", &self.position)
            .field("centre", &self.centre)
            .field("health", &self.health)
            .field("kind", &self.kind)
            .field("profession", &self.profession)
            .finish()
    }
}
struct Pose {
    peer_eye: Vec3,
    camera: (f32, f32),
    next_input: u64,
    server_eye: Vec3,
    look: (f32, f32),
    grounded_velocity: (bool, Vec3, bool, Vec3),
    target: TargetMeta,
    distance: Option<f32>,
    foot_distance: Option<f32>,
    facing: Option<f32>,
    limits: (f32, f32, f32),
    generation: u32,
    respawns: u32,
    frontier: ActionHeader,
    ticks: (u64, u64),
    inputs: (u64, u64, bool, usize),
    window: (u32, u32, bool, bool, Option<u32>),
    peer_digest: Option<u32>,
    authority: (bool, bool, bool, bool),
}
impl fmt::Debug for Pose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pose")
            .field("peer_eye", &self.peer_eye)
            .field("camera", &self.camera)
            .field("next_input", &self.next_input)
            .field("server_eye", &self.server_eye)
            .field("look", &self.look)
            .field("peer_server_grounded_velocity", &self.grounded_velocity)
            .field("target", &self.target)
            .field("distance", &self.distance)
            .field("foot_distance", &self.foot_distance)
            .field("facing", &self.facing)
            .field("reach_minFacing_footRange", &self.limits)
            .field("generation", &self.generation)
            .field("respawns", &self.respawns)
            .field("frontier", &self.frontier)
            .field("ticks", &self.ticks)
            .field("received_applied_pending_queued", &self.inputs)
            .field("lastEvent_base_predatesReplace_acceptsBoundary_rawDigest", &self.window)
            .field("peer_digest", &self.peer_digest)
            .field("scope_personal_readFailed_tracked", &self.authority)
            .finish()
    }
}
fn pose(source: &HeadlessGame, peer: &HeadlessGame, seat: Seat, target: u32) -> Pose {
    let ecs = &source.state.ecs;
    let entity = ecs.query::<&crate::entity::ProtocolId>().iter().find(|(_, id)| id.0 == target).map(|(e, _)| e);
    let position = entity.and_then(|e| ecs.get::<&crate::entity::Position>(e).ok().map(|p| p.0));
    let height = entity.and_then(|e| ecs.get::<&crate::entity::Hitbox>(e).ok().map(|h| h.height));
    let centre = position.zip(height).map(|(p, h)| p + Vec3::Y * (h * 0.5));
    let host = &source.state.hosted_server.as_ref().unwrap().server;
    let sp = &host.players[seat.0];
    let remote = peer.state.remote_client.as_ref().unwrap();
    let slot = &peer.state.players[0];
    let server_eye = sp.player.eye_pos();
    let delta = centre.map(|c| c - server_eye);
    let base = remote.applied_window_event();
    Pose {
        peer_eye: slot.player.eye_pos(),
        camera: (slot.camera.yaw, slot.camera.pitch),
        next_input: remote.next_input_seq(),
        server_eye,
        look: (sp.yaw, sp.pitch),
        grounded_velocity: (slot.player.on_ground, slot.player.velocity, sp.player.on_ground, sp.player.velocity),
        target: TargetMeta {
            present: entity.is_some(),
            position,
            centre,
            health: entity.and_then(|e| ecs.get::<&crate::combat::Health>(e).ok().map(|h| (h.current, h.is_dead()))),
            kind: entity.and_then(|e| ecs.get::<&crate::entity::MobKind>(e).ok().map(|k| k.0)),
            profession: entity
                .and_then(|e| ecs.get::<&crate::villager::VillagerComponent>(e).ok().map(|v| v.profession)),
        },
        distance: delta.map(|d| d.length()),
        foot_distance: position.map(|p| p.distance(sp.player.pos)),
        facing: delta.map(|d| d.normalize_or_zero().dot(crate::camera::forward_from(sp.yaw, sp.pitch))),
        limits: (
            crate::combat::ATTACK_REACH + crate::hosted_server::ATTACK_REACH_TOLERANCE,
            crate::hosted_server::JOINER_MIN_FACING_DOT,
            crate::villager_ui::INTERACT_RANGE,
        ),
        generation: sp.attach_gen,
        respawns: sp.respawns,
        frontier: sp.action_frontier.as_ref().unwrap().header(sp.action_frontier.as_ref().unwrap().through(), base),
        ticks: (source.state.tick_counter, host.tick_counter),
        inputs: (sp.last_input_tick, sp.last_applied_input, sp.pending_intent.is_some(), sp.intent_queue.len()),
        window: (
            sp.window_events.last_number(),
            base,
            sp.window_events.predates_replace(base),
            sp.window_events.accepts_boundary(base),
            crate::window_events::pack_raw_marker(sp, &host.registry).ok().map(|raw| raw.digest_with(sp.station, None)),
        ),
        peer_digest: crate::staged_window::MarkerWindow::try_capture_parts(
            &slot.inventory,
            &slot.armour_slots,
            &slot.crafting_ui.cursor_item,
            &slot.crafting_ui.grid,
            &peer.state.registry,
        )
        .map(|raw| raw.digest_with(slot.crafting_ui.station(), None)),
        authority: (
            source.state.world.quest_scope.is_some(),
            sp.personal.is_some(),
            sp.personal_read_failed,
            sp.progression_tracked,
        ),
    }
}
fn physical_ready(source: &HeadlessGame, peer: &HeadlessGame, seat: Seat) -> bool {
    let sp = &source.state.hosted_server.as_ref().unwrap().server.players[seat.0];
    let player = &peer.state.players[0].player;
    let remote = peer.state.remote_client.as_ref().unwrap();
    let still = |p: &crate::physics::Player| {
        p.pos.is_finite() && p.velocity.is_finite() && p.on_ground && p.velocity.length_squared() <= 0.0001
    };
    sp.is_present_and_alive() && !peer.state.players[0].is_dead() && remote.is_connected()
        && still(player) && still(&sp.player) && player.pos.distance(sp.player.pos) <= 0.25
        && remote.player_index() == Some(seat.0 as u32) && remote.applied_actor_respawns() == sp.respawns
        && sp.last_input_tick > 0 && sp.last_applied_input == sp.last_input_tick
        && sp.pending_intent.is_none() && sp.intent_queue.is_empty()
}
/// Network quiescence alone does not mean the two actual bodies have landed.
pub(super) fn positive_giver_seat(source: &mut HeadlessGame, peer: &mut HeadlessGame) -> Seat {
    let seat = settle(source, peer);
    let life = source.state.hosted_server.as_ref().unwrap().server.players[seat.0].respawns;
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut stable = 0;
    loop {
        step(source, peer);
        let current = settle(source, peer);
        assert!(current == seat, "the positive fixture's signed attachment changed while landing");
        let sp = &source.state.hosted_server.as_ref().unwrap().server.players[seat.0];
        assert!(sp.respawns == life && peer.state.remote_client.as_ref().unwrap().applied_actor_respawns() == life,
            "the positive fixture's actual actor life changed while landing");
        stable = if physical_ready(source, peer, seat) { stable + 1 } else { 0 };
        assert!(Instant::now() < deadline,
            "positive bodies did not settle: peer={:?}/{:?}/{} server={:?}/{:?}/{} received={} applied={}",
            peer.state.players[0].player.pos, peer.state.players[0].player.velocity,
            peer.state.players[0].player.on_ground, sp.player.pos, sp.player.velocity, sp.player.on_ground,
            sp.last_input_tick, sp.last_applied_input);
        if stable == 2 { return seat; }
    }
}
pub(super) fn quest(
    source: &mut HeadlessGame,
    peer: &mut HeadlessGame,
    trace: &Observation,
    target: u32,
    incarnation: u64,
    operation: QuestOperation,
) -> Option<QuestView> {
    let seat = settle(source, peer);
    let before = pose(source, peer, seat, target);
    if operation == QuestOperation::Talk && before.frontier.seq == 0 {
        assert!(physical_ready(source, peer, seat), "first positive Talk bodies are not settled: {before:?}");
        assert!(before.target.present && before.target.health.is_some_and(|(hp, dead)| hp.is_finite() && !dead)
            && before.target.kind == Some(crate::mob::MobType::Villager)
            && before.target.profession == Some(crate::villager::Profession::Carpenter)
            && before.distance.is_some_and(|n| n.is_finite() && n < before.limits.0 - 0.5)
            && before.foot_distance.is_some_and(|n| n.is_finite() && n < before.limits.2 - 0.5)
            && before.facing.is_some_and(|n| n.is_finite() && n > before.limits.1 + 0.1),
            "first positive Talk target is not genuinely reachable: {before:?}");
    }
    let remote = peer.state.remote_client.as_ref().unwrap();
    let expected = remote.action_journal.as_ref().unwrap().next_header(remote.applied_window_event()).unwrap();
    let preceding_input = trace.begin();
    peer.state.request_quest(target, incarnation, operation.clone());
    settle(source, peer);
    let after = pose(source, peer, seat, target);
    let remote = peer.state.remote_client.as_ref().unwrap();
    let mut s = trace.state();
    eprintln!(
        "joined quest operation={operation:?} target={target} incarnation={incarnation} expected={expected:?} \
        before={before:?} preceding_input={preceding_input:?} after={after:?} final_note={:?} final_view={:?} sent={} received={} last_input={:?} errors={}",
        remote.quest_note,
        remote.quest_view.as_ref().map(ViewMeta::from),
        s.sent,
        s.received,
        s.last_input,
        s.errors
    );
    for event in &s.events {
        eprintln!(
            "joined observation ordinal={} bytes={} sha256={:02x?} {:?}",
            event.ordinal, event.bytes, event.hash, event.meta
        );
    }
    assert_eq!(s.errors, 0, "diagnostic failure; original packets were forwarded, primary timeline printed above");
    assert!(
        s.events.iter().any(|e| matches!(&e.meta, Meta::Quest(h, life, t, inc, digest, op)
        if *h==expected && *life==before.respawns && *t==target && *inc==incarnation
            && Some(*digest)==before.peer_digest && *op==operation)),
        "the real outgoing quest must match its actual expected namespace, sequence and request"
    );
    assert!(
        s.events.iter().any(|e| matches!(&e.meta, Meta::Result(h, life, ..)
        if *h==expected && *life==before.respawns)),
        "matching real result must be present; no fate assumed"
    );
    assert!(after.frontier.seq > before.frontier.seq, "a genuine typed quest request was processed");
    s.active = false;
    remote.quest_view.clone()
}

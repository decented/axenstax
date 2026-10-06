//! Client-side prediction and server reconciliation for a joiner's own body
//! (Spec 04 §5.3).
//!
//! A joiner has ONE position: the one the server simulates from its inputs.
//! Its client still moves at once — it runs the same `Player::tick` over the
//! same input locally (prediction) — and every `StateUpdate` tells it where
//! the server's body is after the last input the server applied
//! (`last_acked_input`). [`OwnPrediction`] keeps the inputs sent since, with
//! the body each produced, and on each update:
//!
//! 1. drops the inputs the server has applied;
//! 2. if the server's body is where this client's was after that input, the
//!    two agree — nothing changes;
//! 3. otherwise it rebases onto the server's position and replays the inputs
//!    still in flight, giving where the body really is now. More than
//!    [`SNAP_DISTANCE`] from the prediction: snap. Less: the body takes the
//!    corrected state at once and the difference becomes a visual offset that
//!    decays over a few ticks ([`OwnPrediction::visual_offset`]), so a small
//!    correction glides instead of jumping.
//!
//! The physics body always holds a state `Player::tick` produced (never a
//! position part-way between two), so a correction can't leave it embedded in
//! a wall; only the camera is smoothed.
//!
//! The server sends position only. The replay takes velocity, ground contact,
//! flight and swimming from this client's own record of that input's step —
//! they agree whenever the positions do.

use std::collections::VecDeque;

use glam::Vec3;

use crate::block::BlockRegistry;
use crate::physics::Player;
use crate::play_mode::PlayMode;
use crate::player_intent::PlayerIntent;
use crate::protocol::InputPacket;
use crate::remote_client::RemoteClient;
use crate::world::World;

/// Most inputs kept awaiting acknowledgement: 6.4 s at 20 TPS. Older ones are
/// dropped; an acknowledgement for one is then ignored (nothing to rebase on)
/// and the next one in range corrects as usual.
pub const HISTORY_CAP: usize = 128;

/// A corrected position further than this (blocks) from the predicted one is
/// snapped to; a nearer one is smoothed.
pub const SNAP_DISTANCE: f32 = 1.0;

/// Server and prediction within this (blocks) agree: no correction at all.
pub const AGREE_EPSILON: f32 = 1e-3;

/// Share of the visual offset kept each tick (0.6 → under 1% after 10 ticks).
pub const SMOOTH_DECAY: f32 = 0.6;

/// Offsets shorter than this (blocks) are dropped to zero.
const OFFSET_EPSILON: f32 = 1e-3;

/// The parts of the body one physics step produces and the next one reads.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BodyState {
    pos: Vec3,
    velocity: Vec3,
    on_ground: bool,
    flying: bool,
    in_water: bool,
}

impl BodyState {
    fn of(p: &Player) -> Self {
        Self {
            pos: p.pos,
            velocity: p.velocity,
            on_ground: p.on_ground,
            flying: p.flying,
            in_water: p.in_water,
        }
    }

    fn apply_to(&self, p: &mut Player) {
        p.pos = self.pos;
        p.velocity = self.velocity;
        p.on_ground = self.on_ground;
        p.flying = self.flying;
        p.in_water = self.in_water;
    }
}

/// One input sent to the server, and the body this client predicted from it.
struct Sent {
    seq: u64,
    /// Exactly what the server will simulate (`PlayerIntent::from_input_packet`
    /// — clamped moves, no edge-triggered jump), not the live intent.
    intent: PlayerIntent,
    yaw: f32,
    pitch: f32,
    /// The body right after this input's step.
    after: BodyState,
}

/// What [`OwnPrediction::reconcile`] did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reconciled {
    /// Nothing to compare: no input acknowledged yet, or the acknowledged one
    /// is no longer held.
    Skipped,
    /// The server's body is where this client predicted it.
    Agreed,
    /// Corrected by `error` blocks (corrected − predicted); the camera glides.
    Smoothed { error: Vec3 },
    /// Corrected by `error` blocks, beyond [`SNAP_DISTANCE`]; no glide.
    Snapped { error: Vec3 },
}

/// Clear the movement keys from an input — what a riding joiner sends (see
/// [`OwnPrediction::send`]). Look, hand and edits are kept.
pub fn hold_still(input: &mut InputPacket) {
    input.move_forward = 0.0;
    input.move_right = 0.0;
    input.sprint = false;
    input.sneak = false;
    input.jump = false;
    input.toggle_flight = false;
}

/// A joiner's prediction of its own body. Empty (and inert) when not joined.
#[derive(Default)]
pub struct OwnPrediction {
    sent: VecDeque<Sent>,
    /// Added to the camera: where the player was last shown minus where the
    /// body now is. Decays to zero.
    visual_offset: Vec3,
    /// The last input went out while riding (see [`Self::send`]).
    riding: bool,
    /// Where the server last said our body is.
    server_pos: Option<Vec3>,
}

impl OwnPrediction {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything — a new session.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Forget every held input and any glide.
    fn clear_history(&mut self) {
        self.sent.clear();
        self.visual_offset = Vec3::ZERO;
    }

    /// Note where the server holds our body, without reconciling: while
    /// riding, there is nothing to reconcile, but getting off goes back here.
    pub fn note_server_pos(&mut self, server_pos: Vec3) {
        if server_pos.is_finite() {
            self.server_pos = Some(server_pos);
        }
    }

    /// Inputs sent and not yet acknowledged (plus the last acknowledged one,
    /// kept as the base the next replay starts from).
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.sent.len()
    }

    /// Send this tick's input over `client` and hold what this client
    /// predicted from it under the sequence number it went out with — the
    /// number the server acknowledges ([`RemoteClient::send_input`] stamps
    /// its own; `input.tick` is ignored). Returns that number; `None` when
    /// nothing was sent.
    ///
    /// `riding`: the body is on a cart or mount, which this client
    /// simulates on its own (BRIDGE: rides stay client-side until the server
    /// simulates them — Spec 04 §5.3.1). Its steering keys must not walk the
    /// server's body, so the input goes out with no movement ([`hold_still`])
    /// and the server's body stands where the ride began; nothing is held.
    /// On the first input after getting off (also sent still), the body is
    /// put back on the server's at once ([`Self::take_server_body`]) — one
    /// clean jump, after which prediction and server agree again.
    #[allow(clippy::too_many_arguments)]
    pub fn send(
        &mut self,
        client: &mut RemoteClient,
        input: &InputPacket,
        body: &mut Player,
        riding: bool,
        world: &World,
        registry: &BlockRegistry,
        mode: PlayMode,
    ) -> Option<u64> {
        let got_off = !riding && self.riding;
        let still;
        let input = if riding || got_off {
            let mut i = input.clone();
            hold_still(&mut i);
            still = i;
            &still
        } else {
            input
        };
        let seq = client.send_input(input);
        if let Some(seq) = seq {
            if riding {
                self.riding = true;
                self.clear_history();
            } else {
                if got_off {
                    self.riding = false;
                    self.take_server_body(body, world, registry, mode);
                }
                self.record(seq, input, body);
            }
        }
        self.decay();
        seq
    }

    /// The end of a ride: put `body` where the server holds it, at rest, and
    /// forget everything in flight. The server's body stood still meanwhile
    /// (the ride sent no movement), so one still step settles ours as its is —
    /// grounded, nothing left of the ride's speed. No server position yet:
    /// the body stays, and the first acknowledgement corrects it.
    fn take_server_body(
        &mut self,
        body: &mut Player,
        world: &World,
        registry: &BlockRegistry,
        mode: PlayMode,
    ) {
        self.clear_history();
        let Some(pos) = self.server_pos else {
            return;
        };
        body.pos = pos;
        body.velocity = Vec3::ZERO;
        body.on_ground = false;
        let cam = crate::camera::Camera::new(pos, 1.0);
        body.tick(&PlayerIntent::default(), &cam, world, registry, mode);
        body.reset_fall();
    }

    /// Record input `seq` just sent, with the body its step produced. A
    /// sequence number not after the newest held one starts a new history:
    /// it belongs to a new connection (each counts from 1), and the old
    /// records can't be acknowledged by it.
    pub fn record(&mut self, seq: u64, input: &InputPacket, body: &Player) {
        if self.sent.back().is_some_and(|last| last.seq >= seq) {
            self.clear_history();
        }
        if self.sent.len() >= HISTORY_CAP {
            self.sent.pop_front();
        }
        self.sent.push_back(Sent {
            seq,
            intent: PlayerIntent::from_input_packet(input),
            yaw: input.yaw,
            pitch: input.pitch,
            after: BodyState::of(body),
        });
    }

    /// Fold in the server's word: its body for us is at `server_pos` after
    /// our input `acked`. Corrects `body` when it disagrees (see the module
    /// doc). Fall bookkeeping (`fall_distance`, `pending_landing`) is left
    /// alone: health is still this client's, and a replayed landing must not
    /// hurt twice.
    pub fn reconcile(
        &mut self,
        acked: u64,
        server_pos: Vec3,
        body: &mut Player,
        world: &World,
        registry: &BlockRegistry,
        mode: PlayMode,
    ) -> Reconciled {
        if !server_pos.is_finite() {
            return Reconciled::Skipped;
        }
        self.server_pos = Some(server_pos);
        if acked == 0 {
            return Reconciled::Skipped;
        }
        while self.sent.front().is_some_and(|s| s.seq < acked) {
            self.sent.pop_front();
        }
        let Some(base) = self.sent.front_mut().filter(|s| s.seq == acked) else {
            return Reconciled::Skipped;
        };
        if (server_pos - base.after.pos).length() <= AGREE_EPSILON {
            return Reconciled::Agreed;
        }

        // Rebase on the server's position and replay what it hasn't applied.
        // The base is rewritten too, so the same update arriving again agrees.
        base.after.pos = server_pos;
        let mut sim = Player::new(server_pos);
        sim.sprint_boots_mult = body.sprint_boots_mult;
        base.after.apply_to(&mut sim);
        for sent in self.sent.iter_mut().skip(1) {
            let mut cam = crate::camera::Camera::new(sim.pos, 1.0);
            cam.yaw = sent.yaw;
            cam.pitch = sent.pitch;
            sim.tick(&sent.intent, &cam, world, registry, mode);
            sent.after = BodyState::of(&sim);
        }

        let error = sim.pos - body.pos;
        let shown = body.pos + self.visual_offset;
        BodyState::of(&sim).apply_to(body);
        if error.length() > SNAP_DISTANCE {
            self.visual_offset = Vec3::ZERO;
            Reconciled::Snapped { error }
        } else {
            self.visual_offset = (shown - body.pos).clamp_length_max(SNAP_DISTANCE);
            Reconciled::Smoothed { error }
        }
    }

    /// Where the camera is drawn relative to the body: zero unless a small
    /// correction is gliding out.
    pub fn visual_offset(&self) -> Vec3 {
        self.visual_offset
    }

    /// Advance the glide one tick.
    pub fn decay(&mut self) {
        self.visual_offset *= SMOOTH_DECAY;
        if self.visual_offset.length() < OFFSET_EPSILON {
            self.visual_offset = Vec3::ZERO;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    /// A stone floor at y = 63 over `-r..=r`, feet at y = 64.
    fn floor_world(r: i32) -> World {
        let mut world = World::new();
        for x in -r..=r {
            for z in -r..=r {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        world
    }

    fn input(seq: u64, forward: f32) -> InputPacket {
        InputPacket { tick: seq, move_forward: forward, health: 20.0, ..Default::default() }
    }

    /// Predict `n` forward steps from `body`, recording each.
    fn walk(
        pred: &mut OwnPrediction,
        body: &mut Player,
        world: &World,
        reg: &BlockRegistry,
        from_seq: u64,
        n: u64,
    ) {
        for seq in from_seq..from_seq + n {
            let pkt = input(seq, 1.0);
            let cam = crate::camera::Camera::new(body.pos, 1.0);
            body.tick(&PlayerIntent::from_input_packet(&pkt), &cam, world, reg, PlayMode::Survival);
            pred.record(seq, &pkt, body);
        }
    }

    fn grounded(at: Vec3) -> Player {
        let mut p = Player::new(at);
        p.on_ground = true;
        p
    }

    #[test]
    fn nothing_acknowledged_is_skipped() {
        let world = floor_world(8);
        let reg = BlockRegistry::new();
        let mut pred = OwnPrediction::new();
        let mut body = grounded(Vec3::new(0.5, 64.0, 0.5));
        walk(&mut pred, &mut body, &world, &reg, 1, 3);
        let r = pred.reconcile(0, Vec3::ZERO, &mut body, &world, &reg, PlayMode::Survival);
        assert_eq!(r, Reconciled::Skipped);
    }

    #[test]
    fn agreement_changes_nothing_and_drops_applied_inputs() {
        let world = floor_world(8);
        let reg = BlockRegistry::new();
        let mut pred = OwnPrediction::new();
        let mut body = grounded(Vec3::new(0.5, 64.0, 0.5));
        walk(&mut pred, &mut body, &world, &reg, 1, 2);
        let at_two = body.pos;
        walk(&mut pred, &mut body, &world, &reg, 3, 3);
        let before = body.pos;
        let r = pred.reconcile(2, at_two, &mut body, &world, &reg, PlayMode::Survival);
        assert_eq!(r, Reconciled::Agreed);
        assert_eq!(body.pos, before);
        assert_eq!(pred.visual_offset(), Vec3::ZERO);
        assert_eq!(pred.held(), 4, "inputs 2..=5 held: 2 as the base, 3..=5 in flight");
    }

    #[test]
    fn a_small_disagreement_replays_and_glides() {
        let world = floor_world(8);
        let reg = BlockRegistry::new();
        let mut pred = OwnPrediction::new();
        let mut body = grounded(Vec3::new(0.5, 64.0, 0.5));
        walk(&mut pred, &mut body, &world, &reg, 1, 2);
        let at_two = body.pos;
        walk(&mut pred, &mut body, &world, &reg, 3, 3);
        let predicted = body.pos;
        let nudge = Vec3::new(0.3, 0.0, 0.0);
        let r = pred.reconcile(2, at_two + nudge, &mut body, &world, &reg, PlayMode::Survival);
        let Reconciled::Smoothed { error } = r else { panic!("expected a glide, got {r:?}") };
        assert!((error - nudge).length() < 1e-4, "replay carries the offset forward: {error}");
        assert!((body.pos - (predicted + nudge)).length() < 1e-4);
        // The camera stays where it was and glides over.
        assert!((pred.visual_offset() + nudge).length() < 1e-4);
        for _ in 0..12 {
            pred.decay();
        }
        assert_eq!(pred.visual_offset(), Vec3::ZERO);
        // The same update again now agrees.
        let r = pred.reconcile(2, at_two + nudge, &mut body, &world, &reg, PlayMode::Survival);
        assert_eq!(r, Reconciled::Agreed);
    }

    #[test]
    fn a_large_disagreement_snaps() {
        let world = floor_world(16);
        let reg = BlockRegistry::new();
        let mut pred = OwnPrediction::new();
        let mut body = grounded(Vec3::new(0.5, 64.0, 0.5));
        walk(&mut pred, &mut body, &world, &reg, 1, 4);
        let server = Vec3::new(5.5, 64.0, -3.5);
        let r = pred.reconcile(4, server, &mut body, &world, &reg, PlayMode::Survival);
        assert!(matches!(r, Reconciled::Snapped { .. }), "{r:?}");
        assert!((body.pos - server).length() < 1e-4, "nothing in flight: body = server");
        assert_eq!(pred.visual_offset(), Vec3::ZERO);
    }

    #[test]
    fn a_new_connection_counting_from_one_again_starts_a_new_history() {
        // A second join in the same process: its inputs count from 1 again.
        // The first session's records must not shadow them.
        let world = floor_world(8);
        let reg = BlockRegistry::new();
        let mut pred = OwnPrediction::new();
        let mut body = grounded(Vec3::new(0.5, 64.0, 0.5));
        walk(&mut pred, &mut body, &world, &reg, 1, 40);
        let mut body = grounded(Vec3::new(-3.5, 64.0, 2.5));
        walk(&mut pred, &mut body, &world, &reg, 1, 2);
        let at_two = body.pos;
        walk(&mut pred, &mut body, &world, &reg, 3, 2);
        assert_eq!(pred.held(), 4, "only the new connection's inputs are held");
        let r = pred.reconcile(2, at_two, &mut body, &world, &reg, PlayMode::Survival);
        assert_eq!(r, Reconciled::Agreed);
    }

    #[test]
    fn the_history_is_bounded() {
        let world = floor_world(4);
        let reg = BlockRegistry::new();
        let mut pred = OwnPrediction::new();
        let body = grounded(Vec3::new(0.5, 64.0, 0.5));
        for seq in 1..=(HISTORY_CAP as u64 + 50) {
            pred.record(seq, &input(seq, 0.0), &body);
        }
        assert_eq!(pred.held(), HISTORY_CAP);
        let mut b = grounded(Vec3::new(0.5, 64.0, 0.5));
        let r = pred.reconcile(10, Vec3::new(3.0, 64.0, 0.5), &mut b, &world, &reg, PlayMode::Survival);
        assert_eq!(r, Reconciled::Skipped, "an input no longer held can't be rebased on");
    }
}

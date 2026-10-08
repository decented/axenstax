//! Remote client — connects to a hosted game server for LAN co-op.
//!
//! Handles the join handshake, input sending, and state receiving.
//! The actual game rendering and input collection happen in the game loop;
//! this module manages the network session.

#[cfg(not(target_arch = "wasm32"))]
use std::net::SocketAddr;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::protocol::{self, PacketType};
use crate::transport::ClientTransport;

/// A present remote player's verified identity, for the inspect view. Built from
/// the server's `Joined` events. `npub` is the full NIP-19 npub (copyable, lets a
/// specific person be verified beyond the grindable collision suffix) or `""` for
/// a guest / unverified join.
#[derive(Clone, Debug)]
pub struct RemoteIdentity {
    pub handle: String,
    pub npub: String,
}

/// Connection state for a remote client.
pub enum ConnectionState {
    /// Not connected.
    Disconnected,
    /// Connection established, waiting for join accept.
    Connecting,
    /// Joined and playing.
    Connected {
        player_index: u32,
        /// Carried for completeness; game_loop.rs applies the world seed
        /// straight from the `JoinAccept` packet (`apply_world_seed`) rather
        /// than reading it back out of this state, so nothing reads it here.
        #[allow(dead_code)]
        seed: u32,
    },
    /// Connection failed or was rejected.
    Failed(String),
}

/// Shown when the link to the host closes after joining (host quit, crashed,
/// dropped us, timed out).
pub const HOST_CONNECTION_LOST: &str = "Disconnected from host";
/// Shown when the link closes before the join completed.
pub const HOST_UNREACHABLE: &str = "Couldn't reach the host";

/// Shown when the host never lets us in (no `JoinAccept`, no refusal) within
/// [`JOIN_ACCEPT_TIMEOUT_SECS`] — e.g. a host on another protocol version
/// that can't read our request.
pub const HOST_NO_ANSWER: &str = "The host didn't let us in. Try joining again.";

/// How long the loading screen waits for the host's `JoinAccept` before it
/// gives up. This is the *client's* patience, and it is longer than the host's:
/// a host frees a slot that has not completed its join after 30 s
/// (`hosted_server::PRE_AUTH_TIMEOUT_TICKS`, the join challenge's lifetime) and
/// refuses it with "Join timed out". So a signed-in join that waits on a slow
/// signer (possibly a phone) is cut off by the host at about 30 s, well before
/// this clock; 90 s is only the backstop for a host that neither accepts nor
/// refuses — e.g. one on another protocol version that can't read our request.
pub const JOIN_ACCEPT_TIMEOUT_SECS: f32 = 90.0;

/// Furthest a host may place a joiner horizontally (blocks, either side of the
/// origin, on both x and z). Horizontal terrain is unbounded by design
/// (`World::set_block`), so this is a sanity cap, not a world border: past it
/// the chunk coordinates derived from the spawn stop being safe to compute.
pub const MAX_JOIN_SPAWN_HORIZONTAL: f32 = 30_000_000.0;

/// Lowest y a host may place a joiner at: the void-rescue line
/// (`physics::Player::tick` lifts anyone below it back to the surface).
pub const MIN_JOIN_SPAWN_Y: f32 = -64.0;

/// Highest y a host may place a joiner at: the world's top
/// (`(world::MAX_CHUNK_Y + 1) * CHUNK_SIZE`, Y 0..=95) plus headroom.
pub const MAX_JOIN_SPAWN_Y: f32 = ((crate::world::MAX_CHUNK_Y + 1) * crate::chunk::CHUNK_SIZE as i32) as f32 + 64.0;

/// Shown when the host sends world data this game cannot read where it
/// cannot be skipped (a `ColumnLocal` note that does not decode, B2b review
/// LOW-1). An honest host of the same version never does.
pub const HOST_BAD_WORLD_DATA: &str =
    "The host sent world data this game can't read, so we left. Try joining again.";

/// Shown when the host's `JoinAccept` places us outside any sane world.
pub const HOST_BAD_SPAWN: &str =
    "The host sent a starting position outside the world, so we didn't join. Try another game.";

/// Is a *finite* host-chosen spawn inside the range the joiner will accept?
/// (Non-finite spawns are not refused — they are dropped, see
/// [`JoinedWorld::from_accept`].)
fn join_spawn_in_range(spawn: glam::Vec3) -> bool {
    spawn.x.abs() <= MAX_JOIN_SPAWN_HORIZONTAL
        && spawn.z.abs() <= MAX_JOIN_SPAWN_HORIZONTAL
        && (MIN_JOIN_SPAWN_Y..=MAX_JOIN_SPAWN_Y).contains(&spawn.y)
}

/// Why a `JoinAccept` must be refused over its spawn, if it must: a finite
/// position outside the accepted range. A hostile host could otherwise send a
/// huge-but-finite spawn that overflows the chunk coordinates derived from it
/// (debug panic, wrong terrain in release).
pub fn join_spawn_refusal(accept: &protocol::JoinAcceptPacket) -> Option<&'static str> {
    let spawn = glam::Vec3::new(accept.spawn_x, accept.spawn_y, accept.spawn_z);
    (spawn.is_finite() && !join_spawn_in_range(spawn)).then_some(HOST_BAD_SPAWN)
}

/// Toast for a joiner whose terrain generator differs from the host's.
pub const WORLDGEN_MISMATCH_NOTICE: &str = "This world was made with a different version of the game. Some terrain may look different until you update.";

/// Folder name of a joined session's world. Never read from or written to
/// disk: a joined world is the host's (`GameState::persists_locally`).
pub const JOINED_WORLD_FOLDER: &str = "remote_game";

/// What the host's `JoinAccept` says the joined world IS (gap-audit T2-9): the
/// joiner builds its world meta from this and generates terrain only after it
/// has arrived, so its world matches the host's seed and rules.
#[derive(Clone, Debug, PartialEq)]
pub struct JoinedWorld {
    pub seed: u32,
    pub rules: protocol::WorldRules,
    /// The host's `worldgen_fingerprint()`.
    pub worldgen_version: u32,
    /// Where the host placed us. `None` when the host sent a non-finite
    /// position (NaN/inf would poison every f32→i32 cast downstream).
    pub spawn: Option<glam::Vec3>,
    /// Phase B2b — `JoinAccept.chunk_note_radius`: how far round our server
    /// body the host tells us about every column (`0` = it pushes everything).
    pub chunk_note_radius: u8,
}

impl JoinedWorld {
    pub fn from_accept(accept: &protocol::JoinAcceptPacket) -> Self {
        let spawn = glam::Vec3::new(accept.spawn_x, accept.spawn_y, accept.spawn_z);
        Self {
            seed: accept.seed,
            rules: accept.world_rules.clone(),
            worldgen_version: accept.worldgen_version,
            spawn: spawn.is_finite().then_some(spawn),
            chunk_note_radius: accept.chunk_note_radius,
        }
    }

    /// The joiner's world meta: a blank meta (fresh Proof-of-Play secret, no
    /// owner) carrying the host's seed and rules.
    pub fn to_meta(&self) -> crate::save::WorldMeta {
        let mut meta = crate::save::WorldMeta::new(JOINED_WORLD_FOLDER);
        meta.seed = self.seed;
        self.rules.apply_to_meta(&mut meta);
        meta
    }

    /// The toast to show when the host's generator differs from ours.
    pub fn worldgen_mismatch_notice(&self) -> Option<&'static str> {
        (self.worldgen_version != crate::world::worldgen_fingerprint())
            .then_some(WORLDGEN_MISMATCH_NOTICE)
    }
}

/// Where a joined session's world load stands.
#[derive(Debug, PartialEq)]
pub enum JoinGate {
    /// The host's world is known — build it, then load.
    Ready(JoinedWorld),
    /// Still waiting for the host's `JoinAccept`.
    Waiting,
    /// The join is over (refused, link lost, or no answer): leave with this.
    Ended(String),
}

/// Pure decision for the loading screen of a joined session: a session that
/// has ended wins, then an arrived `JoinAccept`, then the timeout.
pub fn join_gate(
    state: &ConnectionState,
    accepted: Option<JoinedWorld>,
    waited_secs: f32,
) -> JoinGate {
    match state {
        ConnectionState::Failed(reason) => return JoinGate::Ended(reason.clone()),
        ConnectionState::Disconnected => return JoinGate::Ended(HOST_CONNECTION_LOST.to_string()),
        ConnectionState::Connecting | ConnectionState::Connected { .. } => {}
    }
    match accepted {
        Some(world) => JoinGate::Ready(world),
        None if waited_secs >= JOIN_ACCEPT_TIMEOUT_SECS => JoinGate::Ended(HOST_NO_ANSWER.to_string()),
        None => JoinGate::Waiting,
    }
}

/// What a closed transport means for a session in `state`: `None` when the
/// session already ended (its own reason stands).
fn transport_closed_reason(state: &ConnectionState) -> Option<String> {
    match state {
        ConnectionState::Connecting => Some(HOST_UNREACHABLE.to_string()),
        ConnectionState::Connected { .. } => Some(HOST_CONNECTION_LOST.to_string()),
        ConnectionState::Failed(_) | ConnectionState::Disconnected => None,
    }
}

/// Signed join material produced off the main loop (native bunker on a worker
/// thread, web JS signer via a Promise) in response to the server's challenge.
/// Cross-platform — carries only wire DTOs so `RemoteClient` stays
/// target-agnostic.
pub struct SignedJoin {
    pub auth_event: crate::signet::SignetAuthEventWire,
    pub credential: Option<crate::signet::SignetCredentialWire>,
}

/// One-shot signing driver: given the server's `(challenge_hex, origin)`, kick
/// off signing off the main loop and return a receiver that yields exactly one
/// result. `Send` on native (the closure runs on a worker thread); no bound on
/// wasm (single-threaded, the result arrives from a JS Promise). The driver
/// owns its own timeout, so a hung/offline signer resolves to `Err` rather than
/// hanging the handshake.
#[cfg(not(target_arch = "wasm32"))]
pub type SignDriverFn =
    Box<dyn FnOnce(String, String) -> Receiver<Result<SignedJoin, String>> + Send>;
#[cfg(target_arch = "wasm32")]
pub type SignDriverFn = Box<dyn FnOnce(String, String) -> Receiver<Result<SignedJoin, String>>>;

/// A join challenge nonce as the host issues it: exactly 64 lowercase hex chars.
fn is_challenge_nonce_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Client-side join handshake state. Guest joins send the request immediately
/// (`Sent`); authenticated joins wait for the server's `Challenge`, sign it, and
/// only then send (`AwaitingChallenge` → `Signing` → `Sent`).
enum JoinFlow {
    /// JoinRequest already sent — awaiting JoinAccept/JoinReject.
    Sent,
    /// Authenticated: challenge not yet received. Hold the base request + the
    /// one-shot driver to invoke when the `ChallengePacket` lands.
    AwaitingChallenge {
        base: protocol::JoinRequestPacket,
        driver: Option<SignDriverFn>,
    },
    /// Authenticated: signing in progress off the main loop. Poll `rx`.
    Signing {
        base: protocol::JoinRequestPacket,
        rx: Receiver<Result<SignedJoin, String>>,
    },
}

/// MP-A3 — ticks between re-sends of an unanswered `Respawn` request. The
/// server only honours a `Respawn` once its copy has been dead 20 ticks
/// (`server::MIN_DEAD_TICKS_BEFORE_RESPAWN`), so a request sent the instant
/// after a death is refused and this resend is what lands — keep the two equal.
pub const RESPAWN_RESEND_TICKS: u64 = 20;

/// MP-A3 — a server decision about the local player's own body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OwnLifeEvent {
    /// The server holds us dead: enter the death screen if not already on it.
    /// MP-D2b — with the cause the server recorded (the death-screen line).
    Died(crate::survival::DamageCause),
    /// The server respawned us here (after our `Respawn` request).
    Respawned(glam::Vec3),
    /// MP-D2b — this many hits the server landed on our body wear our armour
    /// (`PlayerSlot::wear_armour` once per hit). C3a-fix-1 — with its window
    /// event (`PlayerEventPacket.window_event`).
    ArmourWorn(u8, u32),
    /// Review D2b B2 — a baby of this species was born to an animal we fed
    /// (the `BreedAnimals` challenge).
    Bred(crate::mob::MobType),
}

/// The server's answer to one of our requests, in arrival order (MP-D2b,
/// C2a): an `InteractOutcome` (a swing or a mob interaction) or an
/// `ItemActionOutcome` (eating, sleeping). One queue for both: the requests
/// share one sequence (`joiner_actions`), and an answer applied out of order
/// would forget the earlier request still waiting (`JoinerActions::take`).
///
/// C3b-1 — the server's container answers ride the same queue, in arrival
/// order with the outcomes, into the window inbox
/// (`window_events::WindowInbox`): a `WindowSlotSet` that changes player
/// slots is a numbered window event and is applied at its number's turn
/// with the grants and the armour wear; `ContainerOpened` and a set of
/// container slots only (`window_event` 0) keep their place among the
/// outcomes.
#[derive(Clone, Debug, PartialEq)]
pub enum RequestOutcome {
    Interact(protocol::InteractOutcomePacket),
    Item(protocol::ItemActionOutcomePacket),
    /// C3b-1 — the answer to our `OpenContainer`.
    ContainerOpened(protocol::ContainerOpenedPacket),
    /// C3b-1 — the server's values for some slots of our window.
    SlotSet(protocol::WindowSlotSetPacket),
}

/// C3b-fix-b (B-M1) — a request this client sends mid-frame, in the one
/// order everything it sends keeps: it waits behind unsent edits
/// ([`RemoteClient::queue_request`]) and goes right after the input that
/// carries them.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Attack(protocol::EntityAttackPacket),
    Interact(protocol::EntityInteractPacket),
    Item(protocol::ItemActionPacket),
    Device((i32, i32, i32)),
}

impl Request {
    /// C3b-fix-d (A-L1) — the request number it goes under
    /// (`JoinerActions`' shared sequence), which names its claim; `None`
    /// for a device right-click, which has none.
    pub fn seq(&self) -> Option<u32> {
        match self {
            Request::Attack(pkt) => Some(pkt.seq),
            Request::Interact(pkt) => Some(pkt.seq),
            Request::Item(pkt) => Some(pkt.seq),
            Request::Device(_) => None,
        }
    }
}

/// Most unnumbered carriers of one kind a poll keeps; a numbered window
/// event (`window_event != 0`) is never dropped (C3b-fix-b, B-L1): the
/// server applies its side when the client reports the number, so a carrier
/// skipped here would leave the client's window behind the server's.
const MAX_UNNUMBERED_PER_POLL: usize = 256;
/// Most own-life events (deaths, births, respawns) a poll keeps.
const MAX_LIFE_EVENTS_PER_POLL: usize = 16;

/// Room for one more carrier in a queue holding `held`: numbered ones always
/// fit, the rest up to `cap`.
fn has_room(held: usize, cap: usize, window_event: u32) -> bool {
    window_event != 0 || held < cap
}

/// MP-D2b — the death cause a `DiedOf` names, as the death screen reads it.
/// A species this build doesn't know reads as a generic death.
pub fn damage_cause_from_wire(cause: protocol::WireDamageCause) -> crate::survival::DamageCause {
    use crate::survival::DamageCause;
    use protocol::WireDamageCause as W;
    match cause {
        W::Generic => DamageCause::Generic,
        W::Fall => DamageCause::Fall,
        W::Drowning => DamageCause::Drowning,
        W::Starvation => DamageCause::Starvation,
        W::Lava => DamageCause::Lava,
        W::Fire => DamageCause::Fire,
        W::Explosion => DamageCause::Explosion,
        W::Mob(kind) => {
            crate::remote_mobs::mob_type_for(kind).map_or(DamageCause::Generic, DamageCause::Mob)
        }
    }
}

/// A remote game client connected to a server.
pub struct RemoteClient {
    transport: Box<dyn ClientTransport>,
    pub state: ConnectionState,
    /// The sequence number (`InputPacket.tick`) the NEXT input goes out with:
    /// 1, 2, 3… per connection. The ONE input counter of a joined session —
    /// the server acknowledges these numbers (`last_acked_input`), so the
    /// client's prediction records under the number [`Self::send_input`]
    /// returns, never a counter of its own (Spec 04 §5.3.1). Never 0: the
    /// server reads 0 as "nothing received yet" and drops it.
    tick: u64,
    /// Latest state update from the server (consumed by game loop each frame).
    pub latest_state: Option<protocol::StateUpdatePacket>,
    /// Chunk pushes received since the game loop last drained them (Phase
    /// B2a), each with the length `pending_block_changes` had when it
    /// arrived: the snapshot sits between those changes and the later ones,
    /// and is applied there (`chunk_intake::apply_in_order`) — a later
    /// change applied first would be overwritten by the older snapshot.
    /// Never trimmed: the server's credit window bounds it, and a server past
    /// [`MAX_QUEUED_CHUNK_PACKETS`] ends the session loudly instead.
    /// Phase B2b: the stream also carries `ColumnLocal` notes, queued here
    /// in arrival order with the pushes ([`crate::chunk_intake::StreamItem`]).
    pub chunk_queue: Vec<(usize, crate::chunk_intake::StreamItem)>,
    /// `ChunkData` packets received since the game loop last drained
    /// [`Self::chunk_queue`] that did not decode. Drained with it into
    /// `ChunkIntake::count_undecodable`: the server numbered them, so the
    /// cumulative ack and every drop's `as_of` must count them too (B2a
    /// review LOW-1).
    pub undecodable_chunks: u32,
    /// Spawn position from JoinAccept.
    pub spawn_pos: Option<(f32, f32, f32)>,
    /// Play mode received in the JoinAccept packet. Consumed once by the game
    /// loop via `set_play_mode` so the `is_creative` cache stays in lock-step.
    pub pending_play_mode: Option<crate::play_mode::PlayMode>,
    /// W2 — the host's difficulty string from `JoinAccept` (the field has
    /// always been on the wire). Consumed once by the game loop into
    /// `GameState.difficulty`, so a joiner's client-side mob/starvation sim
    /// runs at the host's difficulty, not its own default.
    pub pending_difficulty: Option<String>,
    /// The host's clock from `JoinAccept`, so the sky is right before the
    /// first StateUpdate (review W3 N5).
    pub pending_world_time: Option<u32>,
    /// Creator-gallery exhibits from JoinAccept (Spec 2026-06-19 §9). Consumed
    /// once by the game loop into `world.exhibits` so the joiner renders the
    /// world's authored 2D art. `None` until a JoinAccept arrives; empty for a
    /// normal world.
    pub pending_exhibits: Option<Vec<crate::exhibit::Exhibit>>,
    /// The host's world (seed, rules, spawn) from `JoinAccept`. Taken once by
    /// the loading screen, which builds the joined world from it BEFORE any
    /// terrain is generated (gap-audit T2-9).
    pub pending_joined_world: Option<JoinedWorld>,
    /// Join handshake state — drives whether/when the JoinRequest is sent.
    join: JoinFlow,
    /// Present remote players keyed by server slot, for the inspect view. Built
    /// from `Joined`/`Left` events.
    roster: HashMap<u32, RemoteIdentity>,
    /// Operator npub pinned from the connect-string `#op=` fragment (C1). `None`
    /// = anonymous join; the server's identity proof (if any) is ignored.
    pinned_op_npub: Option<String>,
    /// The nonce bytes this client sent in its JoinRequest, kept to verify the
    /// server's identity proof when `JoinAccept` arrives.
    client_nonce: Vec<u8>,
    /// Verified operator npub, set once the server's proof checks out against the
    /// pinned operator. `None` = anonymous / unverified. Surfaced in the UI.
    pub verified_operator: Option<String>,
    /// A server-suggested resource pack (Spec 03 §11.6), set when a
    /// `ResourcePackSuggest` arrives. Consumed once by the game loop, which
    /// prompts the player + (on accept) fetches/loads it. `None` = none pending.
    pub pending_resource_pack: Option<protocol::ResourcePackSuggestPacket>,
    /// JSON of the latest Operator Console snapshot (Spec B task 7), set when an
    /// `OperatorSnapshot` arrives — the server only sends these to the verified
    /// operator. Stored as raw JSON so `RemoteClient` stays cross-platform (the
    /// native game loop parses it into a `ConsoleSnapshot` for the panel).
    pub pending_operator_snapshot_json: Option<String>,
    /// Stacks the server picked up on our behalf (death-drops phase 2b,
    /// `InventoryGrantPacket`). Drained each frame by `network_receive`,
    /// which decodes and adds them to the local player's inventory.
    pub pending_grants: Vec<protocol::InventoryGrantPacket>,
    /// MP-A3 — what the server decided about OUR body: it died (a fall or
    /// drowning the server saw, or our own reported death echoed back), or it
    /// respawned us after our `Respawn`, at the spawn point it holds. Other
    /// players' events are not queued. Drained each frame by `network_receive`.
    pub pending_life_events: Vec<OwnLifeEvent>,
    /// MP-D2b — the server's answers to our `EntityAttack` / `EntityInteract`
    /// requests (and, C2a, our `ItemAction`s, in the same arrival order), and
    /// the kills it credited to us. Drained each frame by `network_receive`;
    /// bounded like `pending_grants`.
    pub pending_outcomes: Vec<RequestOutcome>,
    pub pending_kills: Vec<protocol::KillEventPacket>,
    /// MP-A3 — the tick (`self.tick`) our last `Respawn` request went out, while
    /// the server has not yet answered with `Respawned`. `send_input` re-sends
    /// every [`RESPAWN_RESEND_TICKS`] until it does: the server drops a
    /// client's packets past its per-tick budget, and one lost `Respawn` would
    /// otherwise leave us walking about a respawned body the server holds dead.
    respawn_resend_from: Option<u64>,
    /// C3a-2a — the `op_seq` the last window op went out with: 1, 2, 3… per
    /// connection ([`Self::send_window_op`]).
    window_op_seq: u32,
    /// C3a-fix-1 — the highest server window event this client has applied
    /// (`window_events`), per connection. Stamped on every packet the server
    /// judges against the window as it goes out (`events_applied`): the game
    /// loop applies no event while it holds unsent edits or ops logged before
    /// it, so the count at sending is the count they were made at.
    events_applied: u32,
    /// Entity-event and block-change DELTAS accumulated across every
    /// StateUpdate since the game loop last drained them. `latest_state` is
    /// last-write-wins, which is right for snapshot fields (players,
    /// world_time, reserve) but silently dropped these deltas whenever a
    /// frame hitch batched two server ticks into one poll — lost spawns
    /// meant permanently invisible loot, lost despawns ghost items, lost
    /// block changes world desync. Drained via `std::mem::take` each frame.
    ///
    /// Entity deltas stay grouped per StateUpdate, in arrival order
    /// (`remote_entities::EntityDeltas`): an entity withdrawn in one packet
    /// and re-spawned in the next must not have the despawn applied last.
    pub pending_entity_batches: Vec<crate::remote_entities::EntityDeltas>,
    pub pending_block_changes: Vec<protocol::BlockChange>,
    /// Edits [`serialize_input_within_cap`] trimmed off an earlier input
    /// packet (or [`tag_cut`] / [`order_cut_at`] held back), oldest first,
    /// each with the `mined` tag of the break that made it, if it was one
    /// (C1; paired at the source, FU1), or (C3c-1) its use tag, and its own
    /// order stamp (C3b-fix-d, A-L3). [`Self::send_input`] puts them ahead of the next
    /// packet's own edits, so a burst too big for one packet is spread over
    /// several instead of the tail being lost (the host never saw it, so it
    /// could never refuse and un-ghost it on this client); a tag rides only
    /// with its own edit. At most [`INPUT_CARRY_OVER_MAX_CHANGES`].
    input_carry_over: Vec<PairedEdit>,
    /// C3b-fix-d (A-L3) — the order stamps (`window_ops::order_stamp`) of
    /// the NEXT input's edits, one each, noted by the game loop just before
    /// it ([`Self::note_edit_stamps`]). Never sent: each rides with its edit
    /// into `input_carry_over` if the edit has to wait, and the first one
    /// there is the cut the ops and requests made after it wait behind
    /// ([`Self::first_carried_stamp`]).
    next_edit_stamps: Vec<u64>,
    /// C3c-1 — the use tags of the NEXT input's edits, one each (`None` for
    /// an edit that is no use), noted by the game loop just before it
    /// ([`Self::note_edit_uses`]). Each rides with its edit from then on.
    next_edit_uses: Vec<Option<protocol::UseTag>>,
    /// C3c-1 — the order stamp of the first window op the game loop still
    /// holds unsent, noted just before the next input
    /// ([`Self::note_order_cut`]): an edit made after it waits for a later
    /// input, so the op (sent right after this input) reaches the server
    /// between the edits it was made between. A use's gain lands on the
    /// server's copy of the window when its edit is processed, so the order
    /// of an op and a use matters there.
    order_cut: Option<u64>,
    /// C3b-fix-b (B-M1) — requests made while edits were unsent, each with
    /// its order stamp, oldest first: they go right after the input that
    /// carries those edits ([`Self::queue_request`],
    /// [`Self::take_queued_requests`]).
    queued_requests: Vec<(u64, Request)>,
    /// World chat (Phase 2) — lines the server delivered to us this poll,
    /// drained by the game loop each frame into `ChatState`. Bounded like
    /// `pending_grants`: a hostile server can't grow this without limit
    /// between frames. Native-only — the web build carries no chat surface
    /// at all (`docs/foundations/2026-09-05-world-chat.md` §6).
    #[cfg(not(target_arch = "wasm32"))]
    pub pending_chat: Vec<protocol::ChatDeliverPacket>,
}

/// Most chunk packets [`RemoteClient::chunk_queue`] holds before the game
/// loop drains it. The server keeps at most `chunk_push::CHUNK_WINDOW_PACKETS`
/// unacknowledged, so an honest server never comes near this.
pub const MAX_QUEUED_CHUNK_PACKETS: usize = 4096;

/// This machine's render distance (columns), announced in every JoinRequest
/// so the server pushes chunks that far (Phase B2a). Kept in step with the
/// graphics settings by the game loop (`sync_graphics_to_engine`); `0` until
/// then, which the server reads as "use your own limit". A joined session
/// then sends its current render distance in every `InputPacket`.
static JOIN_RENDER_DISTANCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Record the render distance a JoinRequest announces (clamped to `u8`).
pub fn set_join_render_distance(columns: i32) {
    JOIN_RENDER_DISTANCE.store(
        columns.clamp(0, i32::from(u8::MAX)) as u8,
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// The render distance a JoinRequest announces.
fn join_render_distance() -> u8 {
    JOIN_RENDER_DISTANCE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Build a guest JoinRequest (no Signet auth) — cross-platform (native + wasm),
/// so the WebSocket join path (browser + native) shares it. Accepted only by an
/// open (non-sign-in) server; a sign-in-required host rejects a guest join.
pub fn build_join_request_guest(player_name: &str, skin_key: u64) -> protocol::JoinRequestPacket {
    protocol::JoinRequestPacket {
        protocol_version: protocol::PROTOCOL_VERSION,
        player_name: player_name.to_string(),
        auth_event: None,
        handle_credential: None,
        skin_key,
        client_nonce_hex: random_nonce_hex(),
        worldgen_version: crate::world::worldgen_fingerprint(),
        ws_host: String::new(),
        render_distance: join_render_distance(),
    }
}

/// 32 random bytes as lowercase hex — the client's server-auth challenge nonce
/// (Track 3). The server signs `challenge_msg(nonce, origin)` over it. Same OS
/// RNG (`getrandom`) the server uses to mint its own challenge nonces.
fn random_nonce_hex() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS RNG unavailable");
    hex::encode(buf)
}

/// Build the JoinRequest packet from PRE-BUILT auth material — pure, so it is
/// unit-testable without a live transport. The live authenticated path no longer
/// uses this (it signs lazily after the challenge via the `SignDriverFn`); this
/// remains the wire packer + the unit-test entry point. When `auth` is present
/// its signed wire event + optional handle credential are attached.
///
/// `skin_key: 0` (default) — the local player's `CosmeticDescriptor` lives on
/// `GameState` (`local_cosmetic`), not reachable here; native joins legitimately
/// announce the default skin until a skin-bytes delivery path exists.
///
/// Native-only because `JoinAuth` is sourced from the native Signet signer.
/// The live native auth path (`native_join_sign_driver` /
/// `connect_websocket_authed`) defers signing until the server's challenge
/// arrives instead of building the request with auth already in hand, so this
/// direct one-shot builder is exercised only by tests now.
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(test), allow(dead_code))]
pub fn build_join_request(
    player_name: &str,
    skin_key: u64,
    auth: Option<crate::signet::native_signer::JoinAuth>,
) -> protocol::JoinRequestPacket {
    let (auth_event, handle_credential) = match auth {
        Some(a) => (Some(a.auth_event), a.credential),
        None => (None, None),
    };
    protocol::JoinRequestPacket {
        protocol_version: protocol::PROTOCOL_VERSION,
        player_name: player_name.to_string(),
        auth_event,
        handle_credential,
        skin_key,
        client_nonce_hex: random_nonce_hex(),
        worldgen_version: crate::world::worldgen_fingerprint(),
        ws_host: String::new(),
        render_distance: join_render_distance(),
    }
}

impl RemoteClient {
    /// The join origin THIS client signs and verifies the server's identity
    /// proof over, built from its own transport (`signet::client_join_origin`):
    /// the QUIC exporter, else the WebSocket host it dialled, else `unbound`.
    fn own_join_origin(&self) -> String {
        crate::signet::client_join_origin(
            self.transport.channel_binding(),
            self.transport.ws_host().as_deref(),
        )
    }

    /// Connect to a remote server as a **guest** (QUIC) — sends the JoinRequest
    /// immediately, no Signet auth. A sign-in-required host (`require_signin`)
    /// rejects this with "sign-in required"; an open server accepts it. For an
    /// authenticated join use [`RemoteClient::connect_authed`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect(
        server_addr: SocketAddr,
        player_name: &str,
        pinned_op_npub: Option<String>,
    ) -> Result<Self, String> {
        let transport = crate::network::connect_to_server(server_addr)
            .map_err(|e| format!("Connection failed: {e}"))?;
        log::info!("Connecting to server at {server_addr} (QUIC, guest)...");
        Ok(Self::from_transport(
            Box::new(transport),
            build_join_request_guest(player_name, 0),
            pinned_op_npub,
        ))
    }

    /// Connect to a remote server with **verified identity** (QUIC). Does NOT
    /// send the JoinRequest on connect: it waits for the server's
    /// `ChallengePacket`, runs `driver(nonce, origin)` (origin built client-side by
    /// `signet::client_join_origin` from the transport's channel binding) to sign a kind-21236 auth
    /// event off the main loop, then sends the JoinRequest with the signed event.
    /// The live signature (a bunker round-trip) is the owner boundary; the
    /// reordered state machine itself is unit-tested with a fake driver.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect_authed(
        server_addr: SocketAddr,
        player_name: &str,
        driver: SignDriverFn,
        pinned_op_npub: Option<String>,
    ) -> Result<Self, String> {
        let transport = crate::network::connect_to_server(server_addr)
            .map_err(|e| format!("Connection failed: {e}"))?;
        log::info!("Connecting to server at {server_addr} (QUIC, authed)...");
        Ok(Self::from_transport_authed(
            Box::new(transport),
            build_join_request_guest(player_name, 0),
            driver,
            pinned_op_npub,
        ))
    }

    /// Authenticated join over a transport the caller already built.
    ///
    /// The online path needs this because the connection is not made by dialling
    /// one address: it is the winner of a race across several candidates on a
    /// pre-bound socket (`network::connect_to_server_on_socket`). Everything
    /// after that — challenge, sign, JoinRequest — is the SAME handshake
    /// `connect_authed` runs, unchanged.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(dead_code)] // Driven by the online join loop (later task).
    pub fn connect_authed_on_transport(
        transport: Box<dyn ClientTransport>,
        player_name: &str,
        driver: SignDriverFn,
        pinned_op_npub: Option<String>,
    ) -> Self {
        log::info!("Joining over an online transport (QUIC, authed)...");
        Self::from_transport_authed(
            transport,
            build_join_request_guest(player_name, 0),
            driver,
            pinned_op_npub,
        )
    }

    /// Connect to a dedicated server over WebSocket as a **guest** — the path
    /// the browser PWA (which has no signer) and a signed-out native client use
    /// (browsers can't speak QUIC). A sign-in-required server — every dedicated
    /// server by default — refuses it. `url` is `ws://host:port` (native) or
    /// `wss://host/ws` (browser, via the Caddy front).
    pub fn connect_websocket(
        url: &str,
        player_name: &str,
        pinned_op_npub: Option<String>,
    ) -> Result<Self, String> {
        #[cfg(not(target_arch = "wasm32"))]
        let transport: Box<dyn ClientTransport> =
            Box::new(crate::ws_transport::connect_ws(url)?);
        #[cfg(target_arch = "wasm32")]
        let transport: Box<dyn ClientTransport> =
            Box::new(crate::ws_transport_web::connect_ws(url)?);
        log::info!("Connecting to server at {url} (WebSocket, guest)...");
        Ok(Self::from_transport(
            transport,
            build_join_request_guest(player_name, 0),
            pinned_op_npub,
        ))
    }

    /// Connect to a dedicated server over WebSocket with **verified identity**
    /// (browser + native). Like `connect_websocket` but defers the JoinRequest
    /// until the challenge arrives, then signs via `driver(nonce, origin)`. The
    /// web driver bridges to the page's Signet signer (4b); on any signer error
    /// the caller may fall back to a guest `connect_websocket`.
    /// Live callers: the native `ws://` / `axenstax://` joins in game_loop.rs
    /// (signed by the restored bunker via `native_join_sign_driver`) and the
    /// wasm join path. WebSocket has no channel binding, so the joiner signs
    /// `axenstax-join:ws-host:<the host it dialled>` (v66) and declares that
    /// host in the JoinRequest; a server with `--public-host` refuses any other
    /// host (Spec 04 §1.8.1). `url` must be the address actually dialled.
    pub fn connect_websocket_authed(
        url: &str,
        player_name: &str,
        driver: SignDriverFn,
        pinned_op_npub: Option<String>,
    ) -> Result<Self, String> {
        #[cfg(not(target_arch = "wasm32"))]
        let transport: Box<dyn ClientTransport> =
            Box::new(crate::ws_transport::connect_ws(url)?);
        #[cfg(target_arch = "wasm32")]
        let transport: Box<dyn ClientTransport> =
            Box::new(crate::ws_transport_web::connect_ws(url)?);
        log::info!("Connecting to server at {url} (WebSocket, authed)...");
        Ok(Self::from_transport_authed(
            transport,
            build_join_request_guest(player_name, 0),
            driver,
            pinned_op_npub,
        ))
    }

    /// Build a client around a transport, sending the JoinRequest immediately
    /// (guest path). Transport-agnostic (QUIC / WebSocket / channel).
    pub(crate) fn from_transport(
        transport: Box<dyn ClientTransport>,
        mut join_req: protocol::JoinRequestPacket,
        pinned_op_npub: Option<String>,
    ) -> Self {
        // v66: a WebSocket joiner declares the host it dialled — the server
        // checks it and signs its identity proof over it, guests included.
        join_req.ws_host = transport.ws_host().unwrap_or_default();
        let client_nonce = hex::decode(&join_req.client_nonce_hex).unwrap_or_default();
        let packet = protocol::serialize_packet(PacketType::JoinRequest, &join_req);
        transport.send_to_server(&packet);
        Self {
            transport,
            state: ConnectionState::Connecting,
            tick: 1,
            latest_state: None,
            chunk_queue: Vec::new(),
            undecodable_chunks: 0,
            spawn_pos: None,
            pending_play_mode: None,
            pending_difficulty: None,
            pending_world_time: None,
            pending_exhibits: None,
            pending_joined_world: None,
            join: JoinFlow::Sent,
            roster: HashMap::new(),
            pinned_op_npub,
            client_nonce,
            verified_operator: None,
            pending_resource_pack: None,
            pending_grants: Vec::new(),
            pending_life_events: Vec::new(),
            pending_outcomes: Vec::new(),
            pending_kills: Vec::new(),
            respawn_resend_from: None,
            window_op_seq: 0,
            events_applied: 0,
            pending_operator_snapshot_json: None,
            pending_entity_batches: Vec::new(),
            pending_block_changes: Vec::new(),
            input_carry_over: Vec::new(),
            next_edit_stamps: Vec::new(),
            next_edit_uses: Vec::new(),
            order_cut: None,
            queued_requests: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            pending_chat: Vec::new(),
        }
    }

    /// Build a client around a transport WITHOUT sending the JoinRequest. The
    /// authenticated handshake waits for the server's `ChallengePacket`, signs
    /// it via `driver`, and only then sends (see `poll`).
    fn from_transport_authed(
        transport: Box<dyn ClientTransport>,
        mut base: protocol::JoinRequestPacket,
        driver: SignDriverFn,
        pinned_op_npub: Option<String>,
    ) -> Self {
        // v66: declare the dialled WS host; the signed origin names the same one.
        base.ws_host = transport.ws_host().unwrap_or_default();
        let client_nonce = hex::decode(&base.client_nonce_hex).unwrap_or_default();
        Self {
            transport,
            state: ConnectionState::Connecting,
            tick: 1,
            latest_state: None,
            chunk_queue: Vec::new(),
            undecodable_chunks: 0,
            spawn_pos: None,
            pending_play_mode: None,
            pending_difficulty: None,
            pending_world_time: None,
            pending_exhibits: None,
            pending_joined_world: None,
            join: JoinFlow::AwaitingChallenge { base, driver: Some(driver) },
            roster: HashMap::new(),
            pinned_op_npub,
            client_nonce,
            verified_operator: None,
            pending_resource_pack: None,
            pending_grants: Vec::new(),
            pending_life_events: Vec::new(),
            pending_outcomes: Vec::new(),
            pending_kills: Vec::new(),
            respawn_resend_from: None,
            window_op_seq: 0,
            events_applied: 0,
            pending_operator_snapshot_json: None,
            pending_entity_batches: Vec::new(),
            pending_block_changes: Vec::new(),
            input_carry_over: Vec::new(),
            next_edit_stamps: Vec::new(),
            next_edit_uses: Vec::new(),
            order_cut: None,
            queued_requests: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            pending_chat: Vec::new(),
        }
    }

    /// Poll for server messages. Call each frame.
    /// Returns true if state changed (e.g., join accepted, state update received).
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        // Read BEFORE draining: every packet the peer sent before the link
        // went (a kick's JoinReject included) is then already queued, and is
        // handled first — so its reason wins over the generic one below.
        let transport_closed = self.transport.is_closed();

        while let Some(packet) = self.transport.try_recv_from_server() {
            if let Some((ptype, payload)) = protocol::deserialize_header(&packet) {
                match ptype {
                    PacketType::JoinAccept => {
                        if let Ok(accept) = protocol::safe_deserialize::<protocol::JoinAcceptPacket>(payload) {
                            // C1 — enforce the server's identity proof against the
                            // pinned operator before trusting the connection. Native
                            // only (verification needs the `nostr` crate); the browser
                            // client ignores the proof and joins as today.
                            #[cfg(not(target_arch = "wasm32"))]
                            let refusal: Option<String> = {
                                use crate::server_identity::proof::{
                                    evaluate_server_identity, ServerAuthOutcome,
                                };
                                match evaluate_server_identity(
                                    self.pinned_op_npub.as_deref(),
                                    accept.server_identity.as_ref(),
                                    &self.client_nonce,
                                    // v63/v66: verify over OUR join origin
                                    // (channel binding, or the WS host we
                                    // dialled), so a proof relayed from
                                    // another leg or address fails.
                                    &self.own_join_origin(),
                                    nostr::Timestamp::now(),
                                ) {
                                    ServerAuthOutcome::Verified(op) => {
                                        log::info!("Server identity verified: operator {op}");
                                        self.verified_operator = Some(op);
                                        None
                                    }
                                    ServerAuthOutcome::Anonymous => None,
                                    ServerAuthOutcome::Refused(reason) => Some(reason),
                                }
                            };
                            #[cfg(target_arch = "wasm32")]
                            let refusal: Option<String> = None;
                            // A spawn no sane world could hold is refused before
                            // anything is derived from it (chunk coordinates).
                            let refusal =
                                refusal.or_else(|| join_spawn_refusal(&accept).map(str::to_string));

                            if let Some(reason) = refusal {
                                log::warn!("Refusing server (identity): {reason}");
                                self.state = ConnectionState::Failed(reason);
                                changed = true;
                            } else {
                                log::info!(
                                    "Joined server as player {} (seed: {}, mode: {:?})",
                                    accept.player_index, accept.seed, accept.play_mode
                                );
                                self.spawn_pos =
                                    Some((accept.spawn_x, accept.spawn_y, accept.spawn_z));
                                // Queue play_mode for game loop to apply via set_play_mode.
                                self.pending_play_mode = Some(accept.play_mode);
                                self.pending_difficulty = Some(accept.difficulty.clone());
                                self.pending_world_time = Some(accept.world_time);
                                // The host's world (seed + rules + spawn) for the
                                // loading screen to build ours from (T2-9).
                                self.pending_joined_world = Some(JoinedWorld::from_accept(&accept));
                                // Queue authored exhibits for the game loop to apply
                                // into world.exhibits (Creator Gallery render path).
                                self.pending_exhibits = Some(accept.exhibits);
                                self.state = ConnectionState::Connected {
                                    player_index: accept.player_index,
                                    seed: accept.seed,
                                };
                                changed = true;
                            }
                        }
                    }
                    PacketType::JoinReject => {
                        if let Ok(reject) = protocol::safe_deserialize::<protocol::JoinRejectPacket>(payload) {
                            log::warn!("Join rejected: {}", reject.reason);
                            self.state = ConnectionState::Failed(reject.reason);
                            changed = true;
                        }
                    }
                    PacketType::StateUpdate => {
                        if let Ok(mut state) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload) {
                            // Deltas accumulate — see the pending_* field docs.
                            // Snapshot fields stay latest-wins via latest_state.
                            let deltas = crate::remote_entities::EntityDeltas::take_from(&mut state);
                            if !deltas.is_empty() {
                                self.pending_entity_batches.push(deltas);
                            }
                            self.pending_block_changes.append(&mut state.block_changes);
                            // C3b-2 — this packet's block views go in the world
                            // stream after its block changes (and after every
                            // chunk push that arrived before it). Not numbered
                            // stream packets: no bound, no acknowledgement.
                            for view in state.block_views.drain(..) {
                                self.chunk_queue.push((
                                    self.pending_block_changes.len(),
                                    crate::chunk_intake::StreamItem::View(Box::new(view)),
                                ));
                            }
                            self.latest_state = Some(state);
                            changed = true;
                        }
                    }
                    PacketType::ChunkData => {
                        match protocol::safe_deserialize::<protocol::ChunkDataPacket>(payload) {
                            Ok(chunk) if self.chunk_queue.len() < MAX_QUEUED_CHUNK_PACKETS => {
                                self.chunk_queue.push((
                                    self.pending_block_changes.len(),
                                    crate::chunk_intake::StreamItem::Chunk(chunk),
                                ));
                                changed = true;
                            }
                            Ok(_) => {
                                // A server past its own credit window: never
                                // drop part of the world silently — end it.
                                log::error!(
                                    "Server sent over {MAX_QUEUED_CHUNK_PACKETS} undrained chunk packets; leaving"
                                );
                                self.state = ConnectionState::Failed(
                                    "The server sent more world data than this game can take in.".to_string(),
                                );
                                changed = true;
                            }
                            Err(e) => {
                                log::warn!("Undecodable chunk packet: {e}");
                                self.undecodable_chunks = self.undecodable_chunks.wrapping_add(1);
                            }
                        }
                    }
                    // Phase B2b — "this column is local": in line with the
                    // pushes, numbered with them (it counts towards the ack).
                    PacketType::ColumnLocal => {
                        match protocol::safe_deserialize::<protocol::ColumnLocalPacket>(payload) {
                            Ok(note) if self.chunk_queue.len() < MAX_QUEUED_CHUNK_PACKETS => {
                                self.chunk_queue.push((
                                    self.pending_block_changes.len(),
                                    crate::chunk_intake::StreamItem::Local((note.cx, note.cz), note.hash),
                                ));
                                changed = true;
                            }
                            Ok(_) => {
                                log::error!(
                                    "Server sent over {MAX_QUEUED_CHUNK_PACKETS} undrained chunk packets; leaving"
                                );
                                self.state = ConnectionState::Failed(
                                    "The server sent more world data than this game can take in.".to_string(),
                                );
                                changed = true;
                            }
                            Err(e) => {
                                // B2b review LOW-1 — its column is unknown, so
                                // it can be neither taken in nor let go of; an
                                // honest server of this version never sends
                                // one. End the session with a reason rather
                                // than wait on a column for good.
                                log::error!("Undecodable column note: {e}; leaving");
                                self.state = ConnectionState::Failed(HOST_BAD_WORLD_DATA.to_string());
                                changed = true;
                            }
                        }
                    }
                    PacketType::PlayerEvent => {
                        if let Ok(event) = protocol::safe_deserialize::<protocol::PlayerEventPacket>(payload) {
                            match &event.event {
                                protocol::PlayerEventType::Joined { name, npub } => {
                                    log::info!("Player {} joined: {}", event.player_index, name);
                                    self.roster.insert(
                                        event.player_index,
                                        RemoteIdentity { handle: name.clone(), npub: npub.clone() },
                                    );
                                    changed = true;
                                }
                                protocol::PlayerEventType::Left => {
                                    log::info!("Player {} left", event.player_index);
                                    self.roster.remove(&event.player_index);
                                    changed = true;
                                }
                                // MP-A3 — death and respawn are server-held.
                                // Only our own body's are ours to act on;
                                // bounded like `pending_grants`.
                                // A `Died` while our own `Respawn` is
                                // unanswered is about the life we already
                                // left (review D2a-verify N1): the stream is
                                // ordered, and the server answers `Respawned`
                                // before it can kill the new body, so any
                                // death before that answer is the old one —
                                // an echo of a death this client took itself.
                                // Taken, it would kill the respawned client
                                // a second time.
                                protocol::PlayerEventType::Died
                                | protocol::PlayerEventType::DiedOf { .. } => {
                                    if self.player_index() == Some(event.player_index)
                                        && self.respawn_resend_from.is_none()
                                        && self.pending_life_events.len() < MAX_LIFE_EVENTS_PER_POLL
                                    {
                                        let cause = match event.event {
                                            protocol::PlayerEventType::DiedOf { cause } => {
                                                damage_cause_from_wire(cause)
                                            }
                                            _ => crate::survival::DamageCause::Generic,
                                        };
                                        self.pending_life_events.push(OwnLifeEvent::Died(cause));
                                        changed = true;
                                    }
                                }
                                protocol::PlayerEventType::ArmourWorn { hits } => {
                                    if self.player_index() == Some(event.player_index)
                                        && has_room(self.pending_life_events.len(), MAX_LIFE_EVENTS_PER_POLL, event.window_event)
                                    {
                                        self.pending_life_events
                                            .push(OwnLifeEvent::ArmourWorn(*hits, event.window_event));
                                        changed = true;
                                    }
                                }
                                // Review D2b B2 — an unknown species (a newer
                                // server) is ignored.
                                protocol::PlayerEventType::Bred { offspring } => {
                                    if self.player_index() == Some(event.player_index)
                                        && self.pending_life_events.len() < MAX_LIFE_EVENTS_PER_POLL
                                        && let Some(kind) = crate::remote_mobs::mob_type_for(*offspring)
                                    {
                                        self.pending_life_events.push(OwnLifeEvent::Bred(kind));
                                        changed = true;
                                    }
                                }
                                protocol::PlayerEventType::Respawned { x, y, z } => {
                                    if self.player_index() == Some(event.player_index) {
                                        // The server has answered: stop asking,
                                        // whether or not the position is usable.
                                        self.respawn_resend_from = None;
                                        // Held to the same range as a JoinAccept
                                        // spawn: a hostile host could otherwise
                                        // put us anywhere finite.
                                        let at = glam::Vec3::new(*x, *y, *z);
                                        if at.is_finite()
                                            && join_spawn_in_range(at)
                                            && self.pending_life_events.len() < MAX_LIFE_EVENTS_PER_POLL
                                        {
                                            self.pending_life_events
                                                .push(OwnLifeEvent::Respawned(at));
                                            changed = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    PacketType::Challenge => {
                        // Phase 4 — an authenticated join is waiting for this.
                        // Kick off signing of `{nonce, origin}` off the main loop;
                        // the JoinRequest is sent once the signature lands (below).
                        // Guest joins (already `Sent`) ignore the challenge.
                        if let Ok(chal) =
                            protocol::safe_deserialize::<protocol::ChallengePacket>(payload)
                            && matches!(self.join, JoinFlow::AwaitingChallenge { .. })
                                && let JoinFlow::AwaitingChallenge { base, driver } =
                                    std::mem::replace(&mut self.join, JoinFlow::Sent)
                                {
                                    match driver {
                                        // Defence in depth (review N3): only
                                        // ever hand the bunker the one shape a
                                        // real host sends — 32 bytes, lowercase
                                        // hex — never an arbitrary string.
                                        Some(_) if !is_challenge_nonce_hex(&chal.nonce_hex) => {
                                            self.state = ConnectionState::Failed(
                                                "server sent a malformed join challenge".to_string(),
                                            );
                                        }
                                        Some(d) => {
                                            // Audit fix B (v63): the origin is
                                            // built from OUR transport (channel
                                            // binding, or the WS host we dialled,
                                            // v66) — never taken from the server,
                                            // which could be a relay or be
                                            // fishing for a web login.
                                            let origin = self.own_join_origin();
                                            let rx = d(chal.nonce_hex, origin);
                                            self.join = JoinFlow::Signing { base, rx };
                                        }
                                        None => { /* no driver — leave as Sent */ }
                                    }
                                }
                    }
                    PacketType::Ping => {
                        // Respond with pong
                        let pong = protocol::serialize_packet(PacketType::Pong, &());
                        self.transport.send_to_server(&pong);
                    }
                    PacketType::OperatorSnapshot => {
                        // The server streams these only to the verified operator
                        // (Spec B task 7). Store the raw JSON for the game loop.
                        if let Ok(snap) = protocol::safe_deserialize::<
                            protocol::OperatorSnapshotPacket,
                        >(payload)
                        {
                            self.pending_operator_snapshot_json = Some(snap.snapshot_json);
                            changed = true;
                        }
                    }
                    PacketType::InteractOutcome => {
                        if let Ok(out) = protocol::safe_deserialize::<
                            protocol::InteractOutcomePacket,
                        >(payload)
                            && has_room(self.pending_outcomes.len(), MAX_UNNUMBERED_PER_POLL, out.window_event)
                        {
                            self.pending_outcomes.push(RequestOutcome::Interact(out));
                            changed = true;
                        }
                    }
                    PacketType::ItemActionOutcome => {
                        if let Ok(out) = protocol::safe_deserialize::<
                            protocol::ItemActionOutcomePacket,
                        >(payload)
                            && has_room(self.pending_outcomes.len(), MAX_UNNUMBERED_PER_POLL, out.window_event)
                        {
                            self.pending_outcomes.push(RequestOutcome::Item(out));
                            changed = true;
                        }
                    }
                    // C3b-1 — container answers, in order with the outcomes.
                    PacketType::ContainerOpened => {
                        if let Ok(opened) = protocol::safe_deserialize::<
                            protocol::ContainerOpenedPacket,
                        >(payload)
                            // C3b-fix-a (v78) — an opened container is a
                            // numbered view: never dropped by the cap.
                            && has_room(self.pending_outcomes.len(), MAX_UNNUMBERED_PER_POLL, opened.window_event)
                        {
                            self.pending_outcomes.push(RequestOutcome::ContainerOpened(opened));
                            changed = true;
                        }
                    }
                    PacketType::WindowSlotSet => {
                        if let Ok(set) = protocol::safe_deserialize::<
                            protocol::WindowSlotSetPacket,
                        >(payload)
                            && has_room(self.pending_outcomes.len(), MAX_UNNUMBERED_PER_POLL, set.window_event)
                        {
                            self.pending_outcomes.push(RequestOutcome::SlotSet(set));
                            changed = true;
                        }
                    }
                    PacketType::KillEvent => {
                        if let Ok(kill) =
                            protocol::safe_deserialize::<protocol::KillEventPacket>(payload)
                            && self.pending_kills.len() < 256
                        {
                            self.pending_kills.push(kill);
                            changed = true;
                        }
                    }
                    PacketType::InventoryGrant => {
                        // Death-drops phase 2b — the server picked up a stack
                        // for this player. Queue for the game loop to decode
                        // into the local inventory.
                        if let Ok(grant) = protocol::safe_deserialize::<
                            protocol::InventoryGrantPacket,
                        >(payload)
                        {
                            // Bounded: a hostile server can't grow this
                            // without limit between frames (a numbered
                            // grant is never dropped, B-L1: the transport's
                            // byte bound still holds for those).
                            if has_room(self.pending_grants.len(), MAX_UNNUMBERED_PER_POLL, grant.window_event) {
                                self.pending_grants.push(grant);
                                changed = true;
                            }
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    PacketType::ChatDeliver => {
                        // World chat (Phase 2) — one line, already permitted
                        // for us by the server's tier-rule evaluation.
                        // Bounded like `pending_grants`. Native-only — the web
                        // build carries no chat surface at all (see
                        // `pending_chat`'s doc comment); on wasm this falls
                        // through to the wildcard arm below.
                        if let Ok(deliver) = protocol::safe_deserialize::<
                            protocol::ChatDeliverPacket,
                        >(payload)
                            && self.pending_chat.len() < 256
                        {
                            self.pending_chat.push(deliver);
                            changed = true;
                        }
                    }
                    PacketType::ResourcePackSuggest => {
                        // Server suggests a resource pack (Spec 03 §11.6). Store it
                        // for the game loop to prompt the player + (on accept) load.
                        if let Ok(suggest) = protocol::safe_deserialize::<
                            protocol::ResourcePackSuggestPacket,
                        >(payload)
                        {
                            log::info!(
                                "server suggests resource pack '{}' ({}, required={})",
                                suggest.name,
                                suggest.url,
                                suggest.required
                            );
                            self.pending_resource_pack = Some(suggest);
                            changed = true;
                        }
                    }
                    _ => {
                        // Unknown or unexpected packet type
                    }
                }
            }
        }

        // Drive an in-flight signature (authenticated join). When it resolves,
        // attach the signed event to the base request and send it; on error or a
        // dropped signer, fail cleanly rather than hang.
        if matches!(self.join, JoinFlow::Signing { .. }) {
            match std::mem::replace(&mut self.join, JoinFlow::Sent) {
                JoinFlow::Signing { base, rx } => match rx.try_recv() {
                    Ok(Ok(signed)) => {
                        let mut req = base;
                        req.auth_event = Some(signed.auth_event);
                        req.handle_credential = signed.credential;
                        let pkt = protocol::serialize_packet(PacketType::JoinRequest, &req);
                        self.transport.send_to_server(&pkt);
                        changed = true;
                    }
                    Ok(Err(e)) => {
                        log::warn!("Join signing failed: {e}");
                        self.state = ConnectionState::Failed(e);
                        changed = true;
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.state = ConnectionState::Failed(
                            "signer stopped before producing a signature".to_string(),
                        );
                        changed = true;
                    }
                    Err(TryRecvError::Empty) => {
                        // Still signing — put the state back and keep waiting.
                        self.join = JoinFlow::Signing { base, rx };
                    }
                },
                other => self.join = other,
            }
        }

        if transport_closed && let Some(reason) = transport_closed_reason(&self.state) {
            log::warn!("Server link closed: {reason}");
            self.state = ConnectionState::Failed(reason);
            changed = true;
        }

        changed
    }

    /// The sequence number the next [`Self::send_input`] stamps.
    pub fn next_input_seq(&self) -> u64 {
        self.tick
    }

    /// Send player input to the server. Call each tick (20 TPS).
    ///
    /// Returns the sequence number the input went out with (its `tick` is
    /// overwritten with this connection's own counter, whatever the caller
    /// put there) — the number the server will acknowledge it by. `None`
    /// when nothing was sent (not connected).
    pub fn send_input(&mut self, input: &protocol::InputPacket) -> Option<u64> {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return None;
        }

        let mut input = input.clone();
        let seq = self.tick;
        input.tick = seq;
        self.tick += 1;

        // MP-A3 — an unanswered Respawn is asked again (see the field).
        if let Some(sent) = self.respawn_resend_from
            && self.tick.saturating_sub(sent) >= RESPAWN_RESEND_TICKS
        {
            self.send_respawn();
        }

        // C3a-fix-1 — the window events applied when these edits were made.
        input.events_applied = self.events_applied;
        // C1/FU1 — this tick's edits, each paired with the `mined` tag of the
        // break that made it, behind the edits trimmed from earlier packets
        // (the host validates them in the order they were made). C3a-fix-1
        // (C-M1) — and with the hand it was made with, which travels with it
        // into a later packet if it is trimmed or held back.
        let fresh_edits = std::mem::take(&mut input.block_changes);
        let mut hands = std::mem::take(&mut input.edit_hands);
        hands.resize(fresh_edits.len(), (u8::MAX, input.held_kind, input.held_id));
        // C3b-fix-d (A-L3) — and with its own order stamp (one taken now for
        // an edit the game loop noted none for).
        let mut stamps = std::mem::take(&mut self.next_edit_stamps);
        stamps.truncate(fresh_edits.len());
        stamps.resize_with(fresh_edits.len(), crate::window_ops::order_stamp);
        // C3c-1 — and with its use tag, paired at the source (the game loop
        // keeps it beside its edit from the moment the use is made).
        let mut uses = std::mem::take(&mut self.next_edit_uses);
        uses.truncate(fresh_edits.len());
        uses.resize(fresh_edits.len(), None);
        input.use_tags.clear();
        let fresh = pair_tags_with_edits(
            fresh_edits
                .into_iter()
                .zip(hands)
                .zip(stamps)
                .zip(uses)
                .map(|(((bc, hand), stamp), use_tag)| (bc, hand, stamp, use_tag))
                .collect(),
            std::mem::take(&mut input.mined),
        );
        let mut edits = std::mem::take(&mut self.input_carry_over);
        edits.extend(fresh);
        // C1 (review LOW-3) / FU1 (C1 verify N4) — the edits from the first
        // that can't go with its tag in this packet wait for the next one.
        // C3c-1 — and so do those made after the first op or request still
        // waiting: they go after it.
        let order_cut = [self.order_cut.take(), self.queued_requests.first().map(|(stamp, _)| *stamp)]
            .into_iter()
            .flatten()
            .min();
        let cut = tag_cut(&edits).min(order_cut_at(&edits, order_cut));
        let mut held_back = edits.split_off(cut);
        let (packet, mut trimmed) = serialize_input_within_cap(&mut input, edits);
        trimmed.append(&mut held_back);
        if trimmed.len() > INPUT_CARRY_OVER_MAX_CHANGES {
            let drop = trimmed.len() - INPUT_CARRY_OVER_MAX_CHANGES;
            log::warn!(
                "input edits are backing up: dropping the {drop} oldest of {} waiting to be sent",
                trimmed.len()
            );
            trimmed.drain(..drop);
        }
        // C3b-fix-b (B-L2) / C3b-fix-d (A-L3) — what waits keeps each edit's
        // own stamp: the first of them is the cut.
        self.input_carry_over = trimmed;
        self.transport.send_to_server(&packet);
        Some(seq)
    }

    /// C3b-fix-d (A-L3) — the order stamps of the edits of the input about
    /// to be sent, one each, in order (`PendingEdits::take`), noted just
    /// before [`Self::send_input`]: an edit the packet can't carry keeps its
    /// own stamp while it waits.
    pub fn note_edit_stamps(&mut self, stamps: Vec<u64>) {
        self.next_edit_stamps = stamps;
    }

    /// C3c-1 — the use tags of the edits of the input about to be sent, one
    /// each, in order (`PendingEdits::take`), noted just before
    /// [`Self::send_input`].
    pub fn note_edit_uses(&mut self, uses: Vec<Option<protocol::UseTag>>) {
        self.next_edit_uses = uses;
    }

    /// C3c-1 — the order stamp of the first window op still waiting to be
    /// sent (`window_ops::OpLog::first_stamp`), noted just before
    /// [`Self::send_input`]: the edits made after it wait for a later input.
    pub fn note_order_cut(&mut self, stamp: Option<u64>) {
        self.order_cut = stamp;
    }

    /// C3b-fix-b (B-L2) — the order stamp of the first edit waiting in the
    /// carry-over, if any: an op or request logged after it goes after the
    /// input that carries it. C3b-fix-d (A-L3) — that edit's own stamp, so
    /// what was made before it (after an edit that already went) goes now.
    pub fn first_carried_stamp(&self) -> Option<u64> {
        self.input_carry_over.first().map(|(_, _, _, stamp)| *stamp)
    }

    /// C3b-fix-b (B-M1) — hold `request`, made at order stamp `stamp` while
    /// edits are unsent: it goes right after the input that carries them
    /// ([`Self::take_queued_requests`]), so the server reads it after them.
    /// Dropped before the join completes, as a send would be.
    pub fn queue_request(&mut self, stamp: u64, request: Request) {
        if matches!(self.state, ConnectionState::Connected { .. }) {
            self.queued_requests.push((stamp, request));
        }
    }

    /// C3b-fix-b (B-M1) — are requests waiting for an input?
    pub fn has_queued_requests(&self) -> bool {
        !self.queued_requests.is_empty()
    }

    /// C3b-fix-b (B-M1) — the queued requests with their stamps, oldest
    /// first, once the edits they waited behind are all sent. C3b-fix-d
    /// (A-L3) — those made before the first edit still waiting in the
    /// carry-over ([`Self::first_carried_stamp`]) are due now; the rest wait
    /// for the input that carries it.
    pub fn take_queued_requests(&mut self) -> Vec<(u64, Request)> {
        let due = match self.first_carried_stamp() {
            Some(cut) => self.queued_requests.iter().take_while(|(stamp, _)| *stamp < cut).count(),
            None => self.queued_requests.len(),
        };
        self.queued_requests.drain(..due).collect()
    }

    /// C3b-fix-d (A-L1) — every queued request, oldest first, whatever still
    /// waits in the carry-over: the link is gone and nothing will send them
    /// (the caller releases their claims).
    pub fn discard_queued_requests(&mut self) -> Vec<(u64, Request)> {
        std::mem::take(&mut self.queued_requests)
    }

    /// C3b-fix-b (B-M1) — send `request` now, stamped with the window events
    /// applied (as each `send_*` does).
    pub fn send_request(&mut self, request: Request) {
        match request {
            Request::Attack(pkt) => self.send_entity_attack(&pkt),
            Request::Interact(pkt) => self.send_entity_interact(&pkt),
            Request::Item(pkt) => self.send_item_action(&pkt),
            Request::Device(pos) => self.send_device_interact(pos),
        }
    }

    /// Ask the server to apply a right-click to the power device in `pos`
    /// (Wind/Copper/Electricity Task 2b). No-op before the join completes —
    /// mirrors `send_input`'s guard.
    ///
    /// A joined client asserts the CELL and nothing else: the host looks up the
    /// device there, decides what a right-click means for it, and broadcasts the
    /// result on the ordinary block-change path. Unlike a block change, nothing
    /// is applied locally first — the host's world is the one the power sim runs
    /// in, so its broadcast is the only truth about what the switch did.
    pub fn send_device_interact(&mut self, pos: (i32, i32, i32)) {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return;
        }
        let packet = protocol::serialize_packet(
            PacketType::DeviceInteract,
            &protocol::DeviceInteractPacket { pos },
        );
        self.transport.send_to_server(&packet);
    }

    /// MP-D2b — ask the server to land a swing on one of its entities. No-op
    /// before the join completes — mirrors `send_input`'s guard.
    pub fn send_entity_attack(&mut self, pkt: &protocol::EntityAttackPacket) {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return;
        }
        let pkt = protocol::EntityAttackPacket { events_applied: self.events_applied, ..pkt.clone() };
        self.transport
            .send_to_server(&protocol::serialize_packet(PacketType::EntityAttack, &pkt));
    }

    /// MP-D2b — ask the server for a one-shot interaction with one of its
    /// mobs. No-op before the join completes.
    pub fn send_entity_interact(&mut self, pkt: &protocol::EntityInteractPacket) {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return;
        }
        let pkt = protocol::EntityInteractPacket { events_applied: self.events_applied, ..pkt.clone() };
        self.transport
            .send_to_server(&protocol::serialize_packet(PacketType::EntityInteract, &pkt));
    }

    /// C2a — ask the server for an item action (eat, sleep). No-op before
    /// the join completes. Not native-only: a web joiner eats and sleeps on
    /// the server too.
    pub fn send_item_action(&mut self, pkt: &protocol::ItemActionPacket) {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return;
        }
        let pkt = protocol::ItemActionPacket { events_applied: self.events_applied, ..pkt.clone() };
        self.transport
            .send_to_server(&protocol::serialize_packet(PacketType::ItemAction, &pkt));
    }

    /// C3a-2a — send one window op (`WindowOp`): `op` as the client applied
    /// it, `digest` its window's digest after it, numbered 1, 2, 3… per
    /// connection. No-op before the join completes (and the number doesn't
    /// move). Never answered. Not native-only: a web joiner's window is
    /// mirrored too.
    ///
    /// C3b-1 — with a container op, the slots it changed on our side and
    /// our values before it of the player slots it acts on (both empty for
    /// any other op); C3b-fix-a — and our own verdict on it (`client_ok`).
    pub fn send_window_op(&mut self, logged: crate::window_ops::LoggedOp) {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return;
        }
        self.window_op_seq = self.window_op_seq.wrapping_add(1);
        let pkt = logged.packet(self.window_op_seq, self.events_applied);
        self.transport.send_to_server(&protocol::serialize_packet(PacketType::WindowOp, &pkt));
    }

    /// C3a-fix-1 — this client applied the server's window event `event`
    /// (0 = none): the count every later packet reports goes up to it.
    pub fn note_window_event(&mut self, event: u32) {
        self.events_applied = self.events_applied.max(event);
    }

    /// C3a-fix-1 — are edits waiting for a later input (trimmed off a full
    /// packet, or held back behind a tag)? The game loop applies no window
    /// event while they do: they were made at the count they will go out
    /// with.
    pub fn has_carry_over(&self) -> bool {
        !self.input_carry_over.is_empty()
    }

    /// Send a chat line to the server (world chat, Phase 2). No-op before
    /// the join completes — mirrors `send_input`'s guard. Native-only — the
    /// web build carries no chat surface at all (see `pending_chat`'s doc
    /// comment); `game_loop::send_chat_line`'s wasm arm never calls this.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_chat(&mut self, text: &str) {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return;
        }
        let packet = protocol::serialize_packet(
            PacketType::ChatSay,
            &protocol::ChatSayPacket { text: text.to_string() },
        );
        self.transport.send_to_server(&packet);
    }

    /// MP-A3 — tell the server we chose Respawn on the death screen. It
    /// respawns us only if it holds us dead, and answers with
    /// `PlayerEventType::Respawned`. No-op before the join completes —
    /// mirrors `send_input`'s guard.
    pub fn send_respawn(&mut self) {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return;
        }
        let packet = protocol::serialize_packet(PacketType::Respawn, &());
        self.transport.send_to_server(&packet);
        // Asked again every `RESPAWN_RESEND_TICKS` until the server answers
        // `Respawned` (or this client disconnects / is dropped on leaving the
        // world — the state lives and dies with the client).
        self.respawn_resend_from = Some(self.tick);
    }

    /// MP-A3 — stop re-sending `Respawn`: we died again before the server
    /// answered, so a late request would respawn the body out from under the
    /// new death screen. The next death screen's own Respawn starts it afresh.
    pub fn cancel_respawn_resend(&mut self) {
        self.respawn_resend_from = None;
    }

    /// Send a graceful disconnect to the server.
    pub fn disconnect(&mut self) {
        let packet = protocol::serialize_packet(PacketType::Disconnect, &());
        self.transport.send_to_server(&packet);
        self.respawn_resend_from = None;
        self.state = ConnectionState::Disconnected;
        log::info!("Disconnected from server");
    }

    /// Whether the client is connected and playing.
    pub fn is_connected(&self) -> bool {
        matches!(self.state, ConnectionState::Connected { .. })
    }

    /// Whether the client is still trying to connect. No caller yet — a
    /// "connecting…" spinner UI would use this.
    #[allow(dead_code)]
    pub fn is_connecting(&self) -> bool {
        matches!(self.state, ConnectionState::Connecting)
    }

    /// Is a signed join waiting on the player's signer (a phone approving the
    /// join) right now? The loading screen says so instead of sitting silent.
    pub fn awaiting_signer(&self) -> bool {
        matches!(self.join, JoinFlow::Signing { .. })
    }

    /// Get the assigned player index (if connected).
    pub fn player_index(&self) -> Option<u32> {
        match &self.state {
            ConnectionState::Connected { player_index, .. } => Some(*player_index),
            _ => None,
        }
    }

    /// Present remote players' verified identities (slot → handle + npub), for
    /// the inspect view. Empty in single-player / before anyone joins.
    pub fn roster(&self) -> &HashMap<u32, RemoteIdentity> {
        &self.roster
    }

    /// Get the error message (if failed). No caller yet — a "connection
    /// failed: {msg}" UI would use this.
    #[allow(dead_code)]
    pub fn error(&self) -> Option<&str> {
        match &self.state {
            ConnectionState::Failed(msg) => Some(msg),
            _ => None,
        }
    }
}

impl RemoteClient {
    /// Test-only: the `mined` tags still waiting with their edits.
    #[cfg(test)]
    fn carried_tags(&self) -> usize {
        self.input_carry_over.iter().filter(|(_, tag, _, _)| tag.is_some()).count()
    }

    #[cfg(test)]
    fn carried_use_tags(&self) -> usize {
        self.input_carry_over.iter().filter(|(_, tag, _, _)| tag.as_ref().is_some_and(|t| t.use_tag().is_some())).count()
    }
}

impl Drop for RemoteClient {
    fn drop(&mut self) {
        if self.is_connected() {
            self.disconnect();
        }
    }
}

/// Most trimmed input edits [`RemoteClient`] holds back for later packets.
/// Past it the oldest are dropped (and logged): a client this far behind has
/// more edits queued than the host's per-tick budget will clear in seconds, and
/// holding them without bound would grow memory for nothing. About 240 KB.
const INPUT_CARRY_OVER_MAX_CHANGES: usize = 16_384;

/// C1/FU1 — an edit waiting to be sent, with its tag: the `mined` tag of the
/// break that made it or (C3c-1) the use tag of the use that made it
/// (`None` for every other edit), (C3a-fix-1, C-M1) the hotbar slot and hand
/// it was made with, and (C3b-fix-d, A-L3) its order stamp, which never goes
/// on the wire.
type PairedEdit = (protocol::BlockChange, Option<protocol::EditTag>, protocol::EditHand, u64);

/// Write `edits` into `input`: its block changes in order, and beside them the
/// tags of the tagged ones, in the same order (the mined ones in `mined`,
/// C3c-1 the use ones in `use_tags`), and every edit's hand (`edit_hands`, in
/// step with the block changes).
fn set_input_edits(input: &mut protocol::InputPacket, edits: &[PairedEdit]) {
    input.block_changes = edits.iter().map(|(bc, _, _, _)| bc.clone()).collect();
    input.mined = edits.iter().filter_map(|(_, tag, _, _)| tag.as_ref().and_then(|t| t.mined()).copied()).collect();
    input.use_tags =
        edits.iter().filter_map(|(_, tag, _, _)| tag.as_ref().and_then(|t| t.use_tag()).cloned()).collect();
    input.edit_hands = edits.iter().map(|(_, _, hand, _)| *hand).collect();
}

/// Serialize a `ClientInput` carrying `edits`, trimming them (newest first)
/// until the packet fits [`protocol::MAX_WIRE_PACKET_LEN`] — the frame cap,
/// which a bigger packet would trip, closing the connection (gap-audit
/// T2-12). Before the cap was aligned such a packet still went out, and the
/// host dropped the whole of it (position included) at `safe_deserialize`.
/// A trimmed edit's tag goes with it.
///
/// Returns the packet and the trimmed tail, oldest first. The caller carries it
/// into the next packet (`RemoteClient::send_input`): the host applies at most
/// `MAX_BLOCK_CHANGES_PER_TICK` edits a tick (the rest wait, FU3) and sends back
/// the real block for each it refuses, but it can only do that for an edit it
/// has seen.
fn serialize_input_within_cap(
    input: &mut protocol::InputPacket,
    mut edits: Vec<PairedEdit>,
) -> (Vec<u8>, Vec<PairedEdit>) {
    set_input_edits(input, &edits);
    let packet = protocol::serialize_packet(PacketType::ClientInput, &*input);
    let Some((first, _, hand, _)) = edits.first() else {
        return (packet, Vec::new());
    };
    if packet.len() <= protocol::MAX_WIRE_PACKET_LEN {
        return (packet, Vec::new());
    }
    // Each edit dropped frees at least its block change and (C3a-fix-1) its
    // hand in `edit_hands` (a tagged one, its tag in `mined` or `use_tags`
    // too), so dropping this many always fits.
    let per_change = (bincode::serialized_size(first).expect("a block change sizes")
        + bincode::serialized_size(hand).expect("a hand sizes")) as usize;
    let excess = packet.len() - protocol::MAX_WIRE_PACKET_LEN;
    let keep = edits.len().saturating_sub(excess.div_ceil(per_change));
    log::warn!(
        "input packet over the {}-byte cap: sending {keep} of {} block changes, the rest in the next packet(s)",
        protocol::MAX_WIRE_PACKET_LEN,
        edits.len()
    );
    let trimmed = edits.split_off(keep);
    set_input_edits(input, &edits);
    (protocol::serialize_packet(PacketType::ClientInput, &*input), trimmed)
}

/// FU1 (C1 verify N4) — pair this tick's `tags` with this tick's `edits`
/// (each with its hand, order stamp and, C3c-1, the use tag it was made
/// with), at the source, so a tag only ever travels with the edit it was
/// made for. The
/// survival break arm pushes a mined cell's edit and then its tag, so each tag
/// goes to the last edit of its cell not yet paired that emptied it (the
/// break leaves AIR), or failing that the last of its cell not yet paired (a
/// crop harvest leaves its replacement). A tag with no edit of its cell this
/// tick has nothing to yield and is not sent. A use's edit already has its
/// tag and takes no mined one.
fn pair_tags_with_edits(
    edits: Vec<(protocol::BlockChange, protocol::EditHand, u64, Option<protocol::UseTag>)>,
    tags: Vec<protocol::MinedBlock>,
) -> Vec<PairedEdit> {
    let mut paired: Vec<PairedEdit> = edits
        .into_iter()
        .map(|(bc, hand, stamp, use_tag)| (bc, use_tag.map(protocol::EditTag::Use), hand, stamp))
        .collect();
    for tag in tags {
        let cell = (tag.x, tag.y, tag.z);
        let free_here = |(bc, t, _, _): &PairedEdit| t.is_none() && (bc.x, bc.y, bc.z) == cell;
        let at = paired
            .iter()
            .rposition(|p| free_here(p) && p.0.new_block == crate::block::AIR)
            .or_else(|| paired.iter().rposition(free_here));
        match at {
            Some(k) => paired[k].1 = Some(protocol::EditTag::Mined(tag)),
            None => log::debug!("a mined tag for {cell:?} with no edit of its cell: not sent"),
        }
    }
    paired
}

/// C1 (review LOW-3) / FU1 (C1 verify N4) / C3c-1 — where to cut `edits` so
/// every tagged edit goes in this packet with its tag and the server pairs
/// each tag with its own edit: before the first edit that can't (the whole
/// list when every one can). What is cut off goes in the next packet, in
/// order, tags and all. Three reasons:
///
/// - the server reads at most `MAX_MINED_PER_INPUT` tags from one input,
///   mined and use tags together, so the 17th tagged edit waits;
/// - the server gives a cell's mined tags, in order, to the edits of that
///   cell that break it (`HostedServer::classify_joiner_edit`), so a mined
///   edit behind an untagged edit of its own cell (an Eraser, a bucket, the
///   client's own piston clearing it — or a placement there) waits: the
///   untagged one could otherwise take its tag;
/// - C3c-1 — the server gives a use tag to the LAST edit of its cell in the
///   input, so any edit of a cell behind a use-tagged edit of that cell
///   waits: a second use of the cell (a bucket emptied then filled again
///   within one send), the break of what the use just grew, a placement
///   over the water it just poured.
fn tag_cut(edits: &[PairedEdit]) -> usize {
    let mut tagged = 0;
    let mut untagged_cells = std::collections::HashSet::new();
    let mut used_cells = std::collections::HashSet::new();
    for (i, (bc, tag, _, _)) in edits.iter().enumerate() {
        let cell = (bc.x, bc.y, bc.z);
        if used_cells.contains(&cell) {
            return i;
        }
        let Some(tag) = tag else {
            untagged_cells.insert(cell);
            continue;
        };
        tagged += 1;
        if tagged > protocol::MAX_MINED_PER_INPUT {
            return i;
        }
        match tag {
            protocol::EditTag::Mined(_) if untagged_cells.contains(&cell) => return i,
            protocol::EditTag::Mined(_) => {}
            protocol::EditTag::Use(_) => {
                used_cells.insert(cell);
            }
        }
    }
    edits.len()
}

/// C3c-1 — where to cut `edits` for the order: before the first edit made
/// after `cut` (the order stamp of the first window op or request still
/// waiting), so that op or request goes ahead of it.
fn order_cut_at(edits: &[PairedEdit], cut: Option<u64>) -> usize {
    match cut {
        Some(cut) => edits.iter().position(|(_, _, _, stamp)| *stamp > cut).unwrap_or(edits.len()),
        None => edits.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signet::native_signer::JoinAuth;
    use crate::signet::SignetAuthEventWire;

    fn sample_wire() -> SignetAuthEventWire {
        SignetAuthEventWire {
            pubkey: [0xaa; 32],
            created_at: 1_700_000_000,
            kind: crate::signet::AUTH_EVENT_KIND,
            tags: vec![
                vec!["challenge".into(), "f".repeat(64)],
                vec!["origin".into(), "https://axenstax.app".into()],
            ],
            content: String::new(),
            id: [0xcc; 32],
            sig: vec![0xde; 64],
            from_np: false,
        }
    }

    #[test]
    fn guest_join_carries_no_auth() {
        let req = build_join_request("Player", 0, None);
        assert_eq!(req.protocol_version, protocol::PROTOCOL_VERSION);
        assert_eq!(req.player_name, "Player");
        assert!(req.auth_event.is_none());
        assert!(req.handle_credential.is_none());
    }

    #[test]
    fn authenticated_join_attaches_the_signed_event() {
        let auth = JoinAuth { auth_event: sample_wire(), credential: None };
        let req = build_join_request("Axo", 7, Some(auth));
        assert_eq!(req.skin_key, 7);
        let attached = req.auth_event.expect("auth event attached");
        assert_eq!(attached.kind, crate::signet::AUTH_EVENT_KIND);
        assert_eq!(attached.tags[0][0], "challenge");
    }

    // ── Phase 4: reordered handshake (challenge → sign → send) ────────────────

    use crate::transport::{channel_pair, ServerTransport};

    fn wire_signed_over(nonce_hex: &str, origin: &str) -> SignetAuthEventWire {
        SignetAuthEventWire {
            pubkey: [0xaa; 32],
            created_at: 1_700_000_000,
            kind: crate::signet::AUTH_EVENT_KIND,
            tags: vec![
                vec!["challenge".into(), nonce_hex.to_string()],
                vec!["origin".into(), origin.to_string()],
            ],
            content: String::new(),
            id: [0xcc; 32],
            sig: vec![0xde; 64],
            from_np: false,
        }
    }

    fn read_join_request(srv: &dyn ServerTransport) -> Option<protocol::JoinRequestPacket> {
        let pkt = srv.try_recv_from_client()?;
        let (ptype, payload) = protocol::deserialize_header(&pkt)?;
        assert_eq!(ptype, PacketType::JoinRequest);
        protocol::safe_deserialize::<protocol::JoinRequestPacket>(payload).ok()
    }

    #[test]
    fn guest_join_sends_request_immediately() {
        let (srv, client) = channel_pair();
        let _rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Guest", 0),
            None,
        );
        let req = read_join_request(&srv).expect("guest JoinRequest sent on connect");
        assert!(req.auth_event.is_none(), "guest carries no auth");
    }

    /// The loading screen's "Waiting for your signer" line reads this: true
    /// exactly while the signer holds the join, not before the challenge and
    /// not once the signed request is on its way.
    #[test]
    fn awaiting_signer_while_the_signer_holds_the_join() {
        let (srv, client) = channel_pair();
        let (tx, rx) = std::sync::mpsc::channel();
        let driver: SignDriverFn = Box::new(move |_nonce, _origin| rx);
        let mut rc = RemoteClient::from_transport_authed(
            Box::new(client),
            build_join_request_guest("Axo", 0),
            driver,
            None,
        );
        assert!(!rc.awaiting_signer(), "no challenge yet: not the signer's turn");

        let chal = protocol::ChallengePacket { nonce_hex: "b".repeat(64) };
        srv.send_to_client(&protocol::serialize_packet(PacketType::Challenge, &chal));
        rc.poll();
        assert!(rc.awaiting_signer(), "challenge handed to the signer");
        rc.poll();
        assert!(rc.awaiting_signer(), "still waiting while the phone is unanswered");

        tx.send(Ok(SignedJoin {
            auth_event: wire_signed_over(&"b".repeat(64), &crate::signet::join_origin(None)),
            credential: None,
        }))
        .unwrap();
        rc.poll();
        assert!(!rc.awaiting_signer(), "signed: the request is sent");
        assert!(read_join_request(&srv).is_some());
    }

    #[test]
    fn authed_join_waits_for_challenge_then_sends_signed_request() {
        let (srv, client) = channel_pair();
        let driver: SignDriverFn = Box::new(|nonce, origin| {
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(Ok(SignedJoin {
                auth_event: wire_signed_over(&nonce, &origin),
                credential: None,
            }))
            .unwrap();
            rx
        });
        let mut rc = RemoteClient::from_transport_authed(
            Box::new(client),
            build_join_request_guest("Axo", 0),
            driver,
            None,
        );

        // Nothing sent before the challenge arrives.
        assert!(read_join_request(&srv).is_none(), "no JoinRequest before challenge");

        // Server issues a challenge. It carries no origin (v63): the client
        // builds the origin from its own transport.
        let chal = protocol::ChallengePacket {
            nonce_hex: "a".repeat(64),
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::Challenge, &chal));

        // Client polls: reads challenge → signs (driver ready) → sends.
        rc.poll();
        rc.poll();

        let req = read_join_request(&srv).expect("signed JoinRequest sent after challenge");
        let ev = req.auth_event.expect("authed join carries the signed event");
        assert_eq!(ev.tags[0], vec!["challenge".to_string(), "a".repeat(64)]);
        // A channel transport has no channel binding → the unbound origin.
        assert_eq!(
            ev.tags[1],
            vec!["origin".to_string(), crate::signet::join_origin(None)]
        );
    }

    /// A client transport with an injected channel binding (stands in for the
    /// QUIC exporter).
    struct BoundClient {
        inner: crate::transport::ChannelClientTransport,
        binding: [u8; 32],
    }
    impl ClientTransport for BoundClient {
        fn send_to_server(&self, data: &[u8]) {
            self.inner.send_to_server(data)
        }
        fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
            self.inner.try_recv_from_server()
        }
        fn channel_binding(&self) -> Option<[u8; 32]> {
            Some(self.binding)
        }
    }

    #[test]
    fn authed_join_signs_the_origin_built_from_its_own_channel_binding() {
        let (srv, client) = channel_pair();
        let binding = [0x5a; 32];
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let seen_d = seen.clone();
        let driver: SignDriverFn = Box::new(move |nonce, origin| {
            *seen_d.lock().unwrap() = Some(origin.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(Ok(SignedJoin { auth_event: wire_signed_over(&nonce, &origin), credential: None }))
                .unwrap();
            rx
        });
        let mut rc = RemoteClient::from_transport_authed(
            Box::new(BoundClient { inner: client, binding }),
            build_join_request_guest("Axo", 0),
            driver,
            None,
        );
        let chal = protocol::ChallengePacket { nonce_hex: "c".repeat(64) };
        srv.send_to_client(&protocol::serialize_packet(PacketType::Challenge, &chal));
        rc.poll();
        rc.poll();

        let expected = crate::signet::join_origin(Some(binding));
        assert_eq!(seen.lock().unwrap().as_deref(), Some(expected.as_str()));
        let req = read_join_request(&srv).expect("signed JoinRequest sent");
        assert_eq!(req.auth_event.unwrap().tags[1], vec!["origin".to_string(), expected]);
    }

    /// A client transport that dialled a WebSocket host (stands in for
    /// `ws_transport::connect_ws`).
    struct WsClient {
        inner: crate::transport::ChannelClientTransport,
        host: &'static str,
    }
    impl ClientTransport for WsClient {
        fn send_to_server(&self, data: &[u8]) {
            self.inner.send_to_server(data)
        }
        fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
            self.inner.try_recv_from_server()
        }
        fn ws_host(&self) -> Option<String> {
            Some(self.host.to_string())
        }
    }

    /// v66: a WS joiner signs the host it dialled and declares the same host.
    #[test]
    fn authed_websocket_join_signs_and_declares_the_dialled_host() {
        let (srv, client) = channel_pair();
        let driver: SignDriverFn = Box::new(move |nonce, origin| {
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(Ok(SignedJoin { auth_event: wire_signed_over(&nonce, &origin), credential: None }))
                .unwrap();
            rx
        });
        let mut rc = RemoteClient::from_transport_authed(
            Box::new(WsClient { inner: client, host: "play.example.org:6767" }),
            build_join_request_guest("Axo", 0),
            driver,
            None,
        );
        let chal = protocol::ChallengePacket { nonce_hex: "d".repeat(64) };
        srv.send_to_client(&protocol::serialize_packet(PacketType::Challenge, &chal));
        rc.poll();
        rc.poll();
        let req = read_join_request(&srv).expect("signed JoinRequest sent");
        assert_eq!(req.ws_host, "play.example.org:6767");
        assert_eq!(
            req.auth_event.unwrap().tags[1],
            vec!["origin".to_string(), "axenstax-join:ws-host:play.example.org:6767".to_string()]
        );
    }

    /// A guest WS join declares its host too: the server's identity proof (a
    /// `#op=` pin) is signed over it.
    #[test]
    fn guest_websocket_join_declares_the_dialled_host() {
        let (srv, client) = channel_pair();
        let _rc = RemoteClient::from_transport(
            Box::new(WsClient { inner: client, host: "[2001:db8::1]:6767" }),
            build_join_request_guest("Axo", 0),
            None,
        );
        let req = read_join_request(&srv).expect("guest JoinRequest sent at once");
        assert_eq!(req.ws_host, "[2001:db8::1]:6767");
        assert!(req.auth_event.is_none());
    }

    #[test]
    fn inventory_grant_is_queued_for_the_game_loop() {
        // Death-drops phase 2b — the server picked up a stack for us; the
        // packet must land on pending_grants for network_receive to apply.
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        let grant = protocol::InventoryGrantPacket {
            item_kind: protocol::item_kind::MATERIAL,
            item_id: 4,
            count: 2,
            full_item: protocol::WireItem::None,
            window_event: 1,
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::InventoryGrant, &grant));
        rc.poll();
        assert_eq!(rc.pending_grants.len(), 1);
        assert_eq!(rc.pending_grants[0].item_kind, protocol::item_kind::MATERIAL);
        assert_eq!(rc.pending_grants[0].item_id, 4);
        assert_eq!(rc.pending_grants[0].count, 2);
    }

    #[test]
    fn entity_events_and_block_changes_survive_batched_state_updates() {
        // Two StateUpdates can land between polls (frame hitch, join-time
        // chunk meshing). `latest_state` is last-write-wins — correct for
        // snapshot data (players/world_time/reserve) but it silently DROPPED
        // the earlier packet's entity diff + block changes, leaving ghost or
        // invisible items and desynced blocks. Deltas must accumulate.
        fn state_update(
            tick: u64,
            spawns: Vec<protocol::EntitySpawn>,
            despawns: Vec<u32>,
            block_changes: Vec<protocol::BlockChange>,
        ) -> Vec<u8> {
            let state = protocol::StateUpdatePacket {
                tick,
                players: Vec::new(),
                block_changes,
                world_time: 0,
                last_acked_input: 0,
                entity_spawns: spawns,
                entity_updates: Vec::new(),
                entity_despawns: despawns,
                reserve_richness: 0.0,
                reserve_target_sats: 0,
                reserve_current_sats: 0,
                rain_ticks_left: 0,
                storm_ticks_left: 0,
                own_hunger: 0,
                block_views: Vec::new(),
            };
            protocol::serialize_packet(PacketType::StateUpdate, &state)
        }
        let spawn7 = protocol::EntitySpawn {
            id: 7,
            kind: protocol::EntityKind::Item,
            x: 1.0,
            y: 65.0,
            z: 2.0,
            yaw: 0.0,
            health: 0,
            item_kind: protocol::item_kind::MATERIAL,
            item_id: 4,
            item_count: 1,
            full_item: protocol::WireItem::None,
        };

        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        srv.send_to_client(&state_update(
            1,
            vec![spawn7],
            vec![],
            vec![protocol::BlockChange::with_meta(1, 64, 0, 9, 0)],
        ));
        srv.send_to_client(&state_update(
            2,
            vec![],
            vec![7],
            vec![protocol::BlockChange::with_meta(2, 64, 0, 0, 0)],
        ));
        rc.poll();

        let batches = &rc.pending_entity_batches;
        assert_eq!(batches.len(), 2, "one batch per StateUpdate, in arrival order");
        let spawn_ids: Vec<u32> = batches[0].spawns.iter().map(|s| s.id).collect();
        assert_eq!(spawn_ids, vec![7], "tick 1's spawn survives tick 2's packet");
        assert_eq!(batches[1].despawns, vec![7], "tick 2's despawn kept too");
        let change_xs: Vec<i32> = rc.pending_block_changes.iter().map(|b| b.x).collect();
        assert_eq!(change_xs, vec![1, 2], "BOTH ticks' block changes, in order");
        // Snapshot data stays last-write-wins.
        assert_eq!(rc.latest_state.as_ref().map(|s| s.tick), Some(2));
    }

    /// Review D2a MEDIUM-2 (a). A mob leaves our interest (despawn, tick 1)
    /// and comes back (full spawn + its update, tick 2), and both packets
    /// land in one frame. Applied packet by packet the mob is there, moving;
    /// folded into one flat list (spawns, then updates, then despawns) the
    /// earlier despawn would delete the new copy — and the server, counting
    /// it as shown, would send only updates the mirror drops.
    #[test]
    fn a_withdrawal_and_reentry_in_one_frame_leave_the_mob_mirrored() {
        use crate::remote_entities::{apply_entity_batches, RemoteItems, RemoteProjectiles};
        use crate::remote_mobs::RemoteMobs;
        let cow = protocol::EntitySpawn {
            id: 5,
            kind: protocol::EntityKind::Cow,
            x: 3.0,
            y: 64.0,
            z: 3.0,
            yaw: 0.0,
            health: 10,
            item_kind: 0,
            item_id: 0,
            item_count: 0,
            full_item: protocol::WireItem::None,
        };
        let mut shown = RemoteMobs::default();
        shown.apply(std::slice::from_ref(&cow), &[], &[]);

        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        let packet = |tick, spawns: Vec<protocol::EntitySpawn>, updates, despawns| {
            protocol::serialize_packet(
                PacketType::StateUpdate,
                &protocol::StateUpdatePacket {
                    tick,
                    players: Vec::new(),
                    block_changes: Vec::new(),
                    world_time: 0,
                    last_acked_input: 0,
                    entity_spawns: spawns,
                    entity_updates: updates,
                    entity_despawns: despawns,
                    reserve_richness: 0.0,
                    reserve_target_sats: 0,
                    reserve_current_sats: 0,
                    rain_ticks_left: 0,
                    storm_ticks_left: 0,
                    own_hunger: 0,
                    block_views: Vec::new(),
                },
            )
        };
        let moving = protocol::EntityUpdate {
            id: 5,
            x: 3.5,
            y: 64.0,
            z: 3.0,
            vx: 0.1,
            ..Default::default()
        };
        srv.send_to_client(&packet(1, vec![], vec![], vec![5]));
        srv.send_to_client(&packet(2, vec![cow.clone()], vec![moving], vec![]));
        rc.poll();

        let (mut items, mut projectiles) = (RemoteItems::default(), RemoteProjectiles::default());
        apply_entity_batches(&rc.pending_entity_batches, &mut items, &mut projectiles, Some(&mut shown));
        assert_eq!(shown.len(), 1, "the re-entered cow is mirrored");
        assert!(shown.drawn(5).is_some());

        // The flat fold this replaces loses it.
        let mut flat = RemoteMobs::default();
        flat.apply(std::slice::from_ref(&cow), &[], &[]);
        let all = &rc.pending_entity_batches;
        let spawns: Vec<_> = all.iter().flat_map(|b| b.spawns.clone()).collect();
        let updates: Vec<_> = all.iter().flat_map(|b| b.updates.clone()).collect();
        let despawns: Vec<_> = all.iter().flat_map(|b| b.despawns.clone()).collect();
        flat.apply(&spawns, &updates, &despawns);
        assert_eq!(flat.len(), 0, "(the hazard: a flat fold applies the despawn last)");
    }

    /// Gap-audit T2-12: with the frame cap aligned to the decode cap, an
    /// input packet carrying a client sim's burst would close the connection.
    /// It is trimmed to fit instead — oldest changes kept — and still decodes.
    #[test]
    fn an_oversized_input_packet_is_trimmed_to_the_frame_cap() {
        let mut small = protocol::InputPacket {
            block_changes: vec![protocol::BlockChange::with_meta(1, 2, 3, 4, 0)],
            ..Default::default()
        };
        let edits = small.block_changes.iter().map(|bc| (bc.clone(), None, (0, 0, 0), 0)).collect();
        let (pkt, trimmed) = serialize_input_within_cap(&mut small, edits);
        assert!(trimmed.is_empty(), "nothing trimmed under the cap");
        assert_eq!(small.block_changes.len(), 1, "a packet under the cap is untouched");
        assert_eq!(pkt, protocol::serialize_packet(PacketType::ClientInput, &small));

        let mut big = protocol::InputPacket {
            block_changes: (0..10_000)
                .map(|i| protocol::BlockChange::with_meta(i, 64, 0, 1, 0))
                .collect(),
            ..Default::default()
        };
        let edits = big.block_changes.iter().map(|bc| (bc.clone(), None, (0, 0, 0), 0)).collect();
        let (pkt, trimmed) = serialize_input_within_cap(&mut big, edits);
        assert!(pkt.len() <= protocol::MAX_WIRE_PACKET_LEN, "{} bytes", pkt.len());
        let (_, payload) = protocol::deserialize_header(&pkt).unwrap();
        let back: protocol::InputPacket = protocol::safe_deserialize(payload).unwrap();
        // C3a-fix-1 — each edit is 19 bytes on the wire now: its 15-byte
        // block change and its 4-byte hand (`edit_hands`).
        let per_edit = 15 + 4;
        assert!(
            back.block_changes.len() >= protocol::MAX_WIRE_PACKET_LEN / per_edit - 16,
            "only the overflow is trimmed: {} kept",
            back.block_changes.len()
        );
        assert_eq!(back.edit_hands.len(), back.block_changes.len(), "every edit keeps its hand");
        assert_eq!(back.block_changes[0].x, 0, "the oldest changes are the ones kept");
        // The trimmed tail is handed back, not lost: kept + trimmed is the lot, in order.
        assert_eq!(back.block_changes.len() + trimmed.len(), 10_000);
        let xs: Vec<i32> = back.block_changes.iter().chain(trimmed.iter().map(|(b, _, _, _)| b)).map(|b| b.x).collect();
        assert_eq!(xs, (0..10_000).collect::<Vec<_>>());
    }

    /// A connected client whose server end the test can read.
    fn connected_client() -> (Box<dyn ServerTransport>, RemoteClient) {
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        rc.state = ConnectionState::Connected { player_index: 1, seed: 7 };
        // Discard the JoinRequest so only input packets are left to read.
        let _ = srv.try_recv_from_client();
        (Box::new(srv), rc)
    }

    /// The block-change x's of the next `ClientInput` the server end holds.
    fn next_input_xs(srv: &dyn ServerTransport) -> Vec<i32> {
        next_input(srv).block_changes.iter().map(|b| b.x).collect()
    }

    /// The next `ClientInput` the server end holds, decoded.
    fn next_input(srv: &dyn ServerTransport) -> protocol::InputPacket {
        let pkt = srv.try_recv_from_client().expect("an input packet was sent");
        assert!(pkt.len() <= protocol::MAX_WIRE_PACKET_LEN, "{} bytes", pkt.len());
        let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
        assert_eq!(ptype, PacketType::ClientInput);
        protocol::safe_deserialize(payload).unwrap()
    }

    fn input_with(xs: impl IntoIterator<Item = i32>) -> protocol::InputPacket {
        protocol::InputPacket {
            block_changes: xs
                .into_iter()
                .map(|i| protocol::BlockChange::with_meta(i, 64, 0, 1, 0))
                .collect(),
            ..Default::default()
        }
    }

    /// Review fix: the edits `serialize_input_within_cap` trims off one packet
    /// used to vanish — the host never saw them, so it never un-ghosted them on
    /// the sender. They ride the next packets instead, oldest first, ahead of
    /// anything newer.
    #[test]
    fn trimmed_input_edits_ride_the_next_packets_in_order() {
        let (srv, mut rc) = connected_client();
        rc.send_input(&input_with(0..10_000));
        let mut delivered = next_input_xs(&*srv);
        assert!(delivered.len() < 10_000, "the first packet had to be trimmed");
        // A newer edit lands behind the carried-over ones.
        rc.send_input(&input_with([50_000]));
        for _ in 0..10 {
            delivered.extend(next_input_xs(&*srv));
            rc.send_input(&protocol::InputPacket::default());
        }
        let mut expect: Vec<i32> = (0..10_000).collect();
        expect.push(50_000);
        assert_eq!(delivered, expect, "every edit arrives once, in the order it was made");
        // Nothing left over: the carry-over queue drained.
        assert!(rc.input_carry_over.is_empty());
        assert!(next_input_xs(&*srv).is_empty(), "an idle tick carries no edits");
    }

    /// C1 — the server yields a mined cell only when its `mined` tag rides
    /// with its edit: an edit trimmed onto a later packet takes its tag along.
    #[test]
    fn a_trimmed_edits_mined_tag_rides_with_it() {
        let (srv, mut rc) = connected_client();
        let tag = protocol::MinedBlock { x: 9_999, y: 64, z: 0, tool: protocol::WireItem::None };
        let mut input = input_with(0..10_000);
        input.mined = vec![tag];
        rc.send_input(&input);
        let mut tagged_with_edit = false;
        for _ in 0..12 {
            let Some(pkt) = srv.try_recv_from_client() else {
                rc.send_input(&protocol::InputPacket::default());
                continue;
            };
            let (_, payload) = protocol::deserialize_header(&pkt).unwrap();
            let sent: protocol::InputPacket = protocol::safe_deserialize(payload).unwrap();
            if sent.block_changes.iter().any(|b| b.x == 9_999) {
                tagged_with_edit = sent.mined.contains(&tag);
            }
            rc.send_input(&protocol::InputPacket::default());
        }
        assert!(tagged_with_edit, "the tag went out in the packet carrying its edit");
        assert_eq!(rc.carried_tags(), 0, "and stopped riding once the edit went");
    }

    /// Review LOW-3 — the server reads at most `MAX_MINED_PER_INPUT` tags
    /// from one input, and a tag counts only in the packet that carries its
    /// edit. 16 carried-over tags plus a new one used to go out together, so
    /// the new one was never read and its drop was lost. Now no packet carries
    /// more tagged cells than the server reads: the edits past the limit wait
    /// for the next packet with their tags, and every tag goes out exactly
    /// once, beside its edit.
    #[test]
    fn carried_over_and_new_mined_tags_each_go_out_once_beside_their_edit() {
        let (srv, mut rc) = connected_client();
        let tag = |x: i32| protocol::MinedBlock { x, y: 64, z: 0, tool: protocol::WireItem::None };
        let max = protocol::MAX_MINED_PER_INPUT as i32;
        // A burst too big for one packet, its last 16 edits mined.
        let n = 10_000;
        let mut input = input_with(0..n);
        input.mined = (n - max..n).map(tag).collect();
        let mut packets = vec![{
            rc.send_input(&input);
            next_input(&*srv)
        }];
        assert_eq!(rc.carried_tags(), max as usize, "16 tags carried over with their edits");
        // The next input mines one more cell.
        let mut next = input_with([50_000]);
        next.mined = vec![tag(50_000)];
        rc.send_input(&next);
        packets.push(next_input(&*srv));
        for _ in 0..6 {
            rc.send_input(&protocol::InputPacket::default());
            packets.push(next_input(&*srv));
        }
        let mut sent = Vec::new();
        for p in &packets {
            assert!(p.mined.len() <= protocol::MAX_MINED_PER_INPUT, "{} tags in one packet", p.mined.len());
            for m in &p.mined {
                assert!(p.block_changes.iter().any(|b| b.x == m.x), "tag {} rides beside its edit", m.x);
                sent.push(m.x);
            }
        }
        let mut expect: Vec<i32> = (n - max..n).collect();
        expect.push(50_000);
        assert_eq!(sent, expect, "every tag goes out exactly once, oldest first");
        let edits: Vec<i32> = packets.iter().flat_map(|p| p.block_changes.iter().map(|b| b.x)).collect();
        let mut all: Vec<i32> = (0..n).collect();
        all.push(50_000);
        assert_eq!(edits, all, "and every edit once, in order");
        assert!(rc.carried_tags() == 0 && rc.input_carry_over.is_empty());
    }

    /// FU1 (C1 verify N4) — a tag rides only with the edit it was made for.
    /// The mined edit goes out in the first packet; a later placement in the
    /// same cell, trimmed onto a later packet, used to take the tag along
    /// again (tags waited while ANY edit of their cell did), and the server
    /// would have read it beside that edit.
    #[test]
    fn a_tag_never_rides_again_beside_a_later_edit_of_its_cell() {
        let (srv, mut rc) = connected_client();
        let tag = protocol::MinedBlock { x: 5, y: 64, z: 0, tool: protocol::WireItem::None };
        let mut input = input_with(0..10_000);
        input.block_changes[5].new_block = crate::block::AIR; // the mine
        input.block_changes.push(protocol::BlockChange::with_meta(5, 64, 0, 1, 0)); // refilled after
        input.mined = vec![tag];
        rc.send_input(&input);
        let mut packets = vec![next_input(&*srv)];
        for _ in 0..6 {
            rc.send_input(&protocol::InputPacket::default());
            packets.push(next_input(&*srv));
        }
        let with_tag: Vec<usize> = (0..packets.len()).filter(|&k| !packets[k].mined.is_empty()).collect();
        assert_eq!(with_tag, vec![0], "the tag goes out once, in the first packet");
        assert_eq!(packets[0].mined, vec![tag]);
        assert!(packets[0].block_changes.iter().any(|b| b.x == 5 && b.new_block == crate::block::AIR));
        let refill = packets.iter().position(|p| p.block_changes.iter().any(|b| b.x == 5 && b.new_block == 1));
        assert!(refill.is_some_and(|k| k > 0), "the refill went later, untagged");
        let edits: usize = packets.iter().map(|p| p.block_changes.len()).sum();
        assert_eq!(edits, 10_001, "every edit once");
        assert_eq!(rc.carried_tags(), 0);
    }

    /// FU1 (C1 verify N4) — the server gives a cell's tags in order to the
    /// edits that break it, so a mined edit behind an untagged edit of its
    /// own cell (an Eraser, then a placement, then the mine) waits for the
    /// next packet: the untagged one can't take its tag.
    #[test]
    fn a_tagged_edit_behind_an_untagged_edit_of_its_cell_waits_for_the_next_packet() {
        let (srv, mut rc) = connected_client();
        let tag = protocol::MinedBlock { x: 7, y: 64, z: 0, tool: protocol::WireItem::None };
        let edit = |b| protocol::BlockChange::with_meta(7, 64, 0, b, 0);
        let input = protocol::InputPacket {
            block_changes: vec![edit(crate::block::AIR), edit(1), edit(crate::block::AIR)],
            mined: vec![tag],
            ..Default::default()
        };
        rc.send_input(&input);
        let first = next_input(&*srv);
        assert_eq!(first.block_changes, vec![edit(crate::block::AIR), edit(1)]);
        assert!(first.mined.is_empty(), "the Eraser and the placement go untagged");
        rc.send_input(&protocol::InputPacket::default());
        let second = next_input(&*srv);
        assert_eq!(second.block_changes, vec![edit(crate::block::AIR)]);
        assert_eq!(second.mined, vec![tag], "the mine goes next, with its tag");
    }

    /// FU1 — a tag pairs with the edit its break made: the last of its cell
    /// that emptied it this tick (a mine, then a placement there in the same
    /// tick, pairs with the mine), else the last of its cell (a harvest);
    /// one with no edit of its cell is not sent.
    #[test]
    fn tags_pair_with_the_edit_their_break_made() {
        let tag = |x| protocol::MinedBlock { x, y: 64, z: 0, tool: protocol::WireItem::None };
        let edit = |x, b| protocol::BlockChange::with_meta(x, 64, 0, b, 0);
        let paired = pair_tags_with_edits(
            vec![edit(1, crate::block::AIR), edit(1, 3), edit(2, crate::block::TILLED_SOIL)]
                .into_iter()
                .map(|e| (e, (0, 0, 0), 0, None))
                .collect(),
            vec![tag(1), tag(2), tag(9)],
        );
        let tags: Vec<Option<i32>> =
            paired.iter().map(|(_, t, _, _)| t.as_ref().and_then(|t| t.mined()).map(|m| m.x)).collect();
        assert_eq!(tags, vec![Some(1), None, Some(2)]);
    }

    fn use_tag(x: i32) -> protocol::UseTag {
        protocol::UseTag { x, y: 64, z: 0, kind: 1, slot: 0, used: None, tool: protocol::WireItem::None }
    }

    /// C3c-1 — send `edits` (each with its use tag, if any) in one input, as
    /// the game loop does: the tags noted beside their edits.
    fn send_uses(rc: &mut RemoteClient, edits: Vec<(protocol::BlockChange, Option<protocol::UseTag>)>) {
        let (changes, uses): (Vec<_>, Vec<_>) = edits.into_iter().unzip();
        rc.note_edit_uses(uses);
        rc.send_input(&protocol::InputPacket { block_changes: changes, ..Default::default() });
    }

    /// C3c-1 — a use tag rides beside the edit it was made with, and a
    /// mined tag never lands on a use's edit.
    #[test]
    fn a_use_tag_rides_beside_its_own_edit() {
        let (srv, mut rc) = connected_client();
        let edit = |x, b| protocol::BlockChange::with_meta(x, 64, 0, b, 0);
        send_uses(&mut rc, vec![(edit(1, 3), None), (edit(2, crate::block::AIR), Some(use_tag(2))), (edit(3, 4), None)]);
        let sent = next_input(&*srv);
        assert_eq!(sent.block_changes.len(), 3);
        assert_eq!(sent.use_tags, vec![use_tag(2)]);
        assert!(sent.mined.is_empty());
        let paired = pair_tags_with_edits(
            vec![(edit(2, crate::block::AIR), (0, 0, 0), 0, Some(use_tag(2)))],
            vec![protocol::MinedBlock { x: 2, y: 64, z: 0, tool: protocol::WireItem::None }],
        );
        assert_eq!(paired[0].1, Some(protocol::EditTag::Use(use_tag(2))), "the use keeps its own tag");
    }

    /// C3c-1 — the server pairs a use tag with the LAST edit of its cell in
    /// the input, so two uses of one cell in one input (a bucket emptied,
    /// then filled again within one send) can't share an input: the second
    /// waits for the next, with its tag — and so does any edit of the cell
    /// behind a use of it.
    #[test]
    fn a_second_edit_of_a_used_cell_waits_for_the_next_input_with_its_tag() {
        let (srv, mut rc) = connected_client();
        let edit = |x, b| protocol::BlockChange::with_meta(x, 64, 0, b, 0);
        let empty = protocol::UseTag { kind: 1, ..use_tag(7) };
        let fill = protocol::UseTag { kind: 0, ..use_tag(7) };
        send_uses(
            &mut rc,
            vec![
                (edit(7, crate::block::WATER), Some(empty.clone())),
                (edit(8, 3), None),
                (edit(7, crate::block::AIR), Some(fill.clone())),
                (edit(7, 5), None),
            ],
        );
        let first = next_input(&*srv);
        assert_eq!(first.block_changes, vec![edit(7, crate::block::WATER), edit(8, 3)]);
        assert_eq!(first.use_tags, vec![empty]);
        assert_eq!(rc.carried_use_tags(), 1);
        rc.send_input(&protocol::InputPacket::default());
        let second = next_input(&*srv);
        assert_eq!(second.block_changes, vec![edit(7, crate::block::AIR)]);
        assert_eq!(second.use_tags, vec![fill], "the second use goes next, with its tag");
        rc.send_input(&protocol::InputPacket::default());
        let third = next_input(&*srv);
        assert_eq!(third.block_changes, vec![edit(7, 5)], "and the placement behind it after");
        assert!(third.use_tags.is_empty() && !rc.has_carry_over());
    }

    /// C3c-1 — use tags count with mined tags against the one per-input
    /// limit: the 17th tagged edit waits for the next input, its tag with
    /// it, never apart.
    #[test]
    fn a_use_tag_past_the_limit_travels_with_its_edit_in_the_next_input() {
        let (srv, mut rc) = connected_client();
        let edit = |x| protocol::BlockChange::with_meta(x, 64, 0, 3, 0);
        let max = protocol::MAX_MINED_PER_INPUT as i32;
        // 10 mined edits, then 10 uses: 20 tags.
        let mut input = protocol::InputPacket {
            block_changes: (0..2 * 10).map(|x| protocol::BlockChange::with_meta(x, 64, 0, crate::block::AIR, 0)).collect(),
            mined: (0..10).map(|x| protocol::MinedBlock { x, y: 64, z: 0, tool: protocol::WireItem::None }).collect(),
            ..Default::default()
        };
        for x in 10..20 {
            input.block_changes[x as usize] = edit(x);
        }
        rc.note_edit_uses((0..20).map(|x| (x >= 10).then(|| use_tag(x))).collect());
        rc.send_input(&input);
        let first = next_input(&*srv);
        assert_eq!(first.mined.len() + first.use_tags.len(), max as usize, "16 tags in all");
        assert_eq!(first.block_changes.len(), max as usize);
        for u in &first.use_tags {
            assert!(first.block_changes.iter().any(|b| b.x == u.x), "use tag {} beside its edit", u.x);
        }
        rc.send_input(&protocol::InputPacket::default());
        let second = next_input(&*srv);
        assert_eq!(second.use_tags.iter().map(|u| u.x).collect::<Vec<_>>(), (16..20).collect::<Vec<_>>());
        assert_eq!(second.block_changes.iter().map(|b| b.x).collect::<Vec<_>>(), (16..20).collect::<Vec<_>>());
    }

    /// C3c-1 — an edit made after a window op still waiting waits for the
    /// next input, so the op (sent after this one) reaches the server
    /// between the two uses it was made between.
    #[test]
    fn edits_made_after_a_waiting_op_wait_for_the_next_input() {
        let (srv, mut rc) = connected_client();
        let edit = |x| protocol::BlockChange::with_meta(x, 64, 0, 3, 0);
        rc.note_edit_stamps(vec![10, 30]);
        rc.note_order_cut(Some(20));
        send_uses(&mut rc, vec![(edit(1), Some(use_tag(1))), (edit(2), Some(use_tag(2)))]);
        let first = next_input(&*srv);
        assert_eq!(first.block_changes, vec![edit(1)]);
        assert_eq!(rc.first_carried_stamp(), Some(30), "the op (stamp 20) is due before the held edit");
        rc.send_input(&protocol::InputPacket::default());
        let second = next_input(&*srv);
        assert_eq!(second.block_changes, vec![edit(2)]);
        assert_eq!(second.use_tags, vec![use_tag(2)]);
    }

    #[test]
    fn the_input_carry_over_is_bounded_and_drops_the_oldest() {
        let (srv, mut rc) = connected_client();
        let n = 50_000;
        rc.send_input(&input_with(0..n));
        let first = next_input_xs(&*srv);
        assert_eq!(first[0], 0, "the oldest edits go out first");
        assert!(rc.input_carry_over.len() <= INPUT_CARRY_OVER_MAX_CHANGES);
        assert_eq!(rc.input_carry_over.len(), INPUT_CARRY_OVER_MAX_CHANGES, "full, not emptied");
        // What is left is the NEWEST edits, contiguous up to the last one made.
        let mut rest = Vec::new();
        for _ in 0..20 {
            rc.send_input(&protocol::InputPacket::default());
            rest.extend(next_input_xs(&*srv));
        }
        let kept = INPUT_CARRY_OVER_MAX_CHANGES as i32;
        assert_eq!(rest, (n - kept..n).collect::<Vec<_>>());
        assert!(rc.input_carry_over.is_empty());
    }

    #[test]
    fn roster_tracks_joined_and_left() {
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        // Server announces another player joined, carrying their npub.
        let ev = protocol::PlayerEventPacket {
            player_index: 5,
            event: protocol::PlayerEventType::Joined {
                name: "Axolittle".into(),
                npub: "npub1axoexample".into(),
            },
            window_event: 0,
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::PlayerEvent, &ev));
        rc.poll();
        let entry = rc.roster().get(&5).expect("joined player in roster");
        assert_eq!(entry.handle, "Axolittle");
        assert_eq!(entry.npub, "npub1axoexample");

        // …and is removed when they leave.
        let left = protocol::PlayerEventPacket {
            player_index: 5,
            event: protocol::PlayerEventType::Left,
            window_event: 0,
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::PlayerEvent, &left));
        rc.poll();
        assert!(rc.roster().get(&5).is_none(), "left player removed from roster");
    }

    /// A client transport whose link reports closed on demand.
    struct ClosingClient {
        inner: crate::transport::ChannelClientTransport,
        closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl ClientTransport for ClosingClient {
        fn send_to_server(&self, data: &[u8]) {
            self.inner.send_to_server(data)
        }
        fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
            self.inner.try_recv_from_server()
        }
        fn is_closed(&self) -> bool {
            self.closed.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    fn closing_client() -> (
        crate::transport::ChannelServerTransport,
        RemoteClient,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let (srv, client) = channel_pair();
        let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rc = RemoteClient::from_transport(
            Box::new(ClosingClient { inner: client, closed: closed.clone() }),
            build_join_request_guest("Me", 0),
            None,
        );
        (srv, rc, closed)
    }

    #[test]
    fn a_closed_link_after_joining_ends_the_session_as_host_lost() {
        // Review W3 S3: a host that quit or crashed left the joiner standing
        // in a frozen world forever.
        let (_srv, mut rc, closed) = closing_client();
        rc.state = ConnectionState::Connected { player_index: 1, seed: 7 };
        assert!(!rc.poll());
        closed.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(rc.poll());
        assert!(matches!(&rc.state, ConnectionState::Failed(r) if r == HOST_CONNECTION_LOST));
    }

    #[test]
    fn a_closed_link_while_connecting_says_the_host_was_unreachable() {
        let (_srv, mut rc, closed) = closing_client();
        closed.store(true, std::sync::atomic::Ordering::Relaxed);
        rc.poll();
        assert!(matches!(&rc.state, ConnectionState::Failed(r) if r == HOST_UNREACHABLE));
    }

    #[test]
    fn a_kick_reason_sent_before_the_close_wins() {
        let (srv, mut rc, closed) = closing_client();
        rc.state = ConnectionState::Connected { player_index: 1, seed: 7 };
        let reject = protocol::JoinRejectPacket {
            reason: "You were removed from this world by its operator.".to_string(),
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::JoinReject, &reject));
        closed.store(true, std::sync::atomic::Ordering::Relaxed);
        rc.poll();
        assert!(
            matches!(&rc.state, ConnectionState::Failed(r) if r.starts_with("You were removed")),
            "the host's own reason is shown, not the generic one"
        );
    }

    #[test]
    fn receives_and_stores_a_server_pack_suggestion() {
        let (srv, client) = channel_pair();
        let mut rc =
            RemoteClient::from_transport(Box::new(client), build_join_request_guest("Me", 0), None);
        assert!(rc.pending_resource_pack.is_none(), "nothing pending before the packet");

        let suggest = protocol::ResourcePackSuggestPacket {
            name: "Server pack".into(),
            url: "https://server/pack.json".into(),
            sha256: "ab".into(),
            size_bytes: 12,
            required: true,
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::ResourcePackSuggest, &suggest));
        rc.poll();

        let pending = rc.pending_resource_pack.as_ref().expect("suggestion stored");
        assert_eq!(pending.name, "Server pack");
        assert!(pending.required);
        assert_eq!(pending.url, "https://server/pack.json");
    }

    #[test]
    fn receives_and_stores_an_operator_snapshot() {
        let (srv, client) = channel_pair();
        let mut rc =
            RemoteClient::from_transport(Box::new(client), build_join_request_guest("Op", 0), None);
        let json = r#"{"server_name":"X","players_cur":1}"#;
        let pkt = protocol::OperatorSnapshotPacket { snapshot_json: json.to_string() };
        srv.send_to_client(&protocol::serialize_packet(PacketType::OperatorSnapshot, &pkt));
        rc.poll();
        assert_eq!(
            rc.pending_operator_snapshot_json.as_deref(),
            Some(json),
            "operator snapshot JSON stored for the game loop"
        );
    }

    fn proofless_accept() -> protocol::JoinAcceptPacket {
        protocol::JoinAcceptPacket {
            player_index: 0,
            seed: 1,
            spawn_x: 0.0,
            spawn_y: 0.0,
            spawn_z: 0.0,
            world_time: 0,
            is_creative: false,
            play_mode: crate::play_mode::PlayMode::Survival,
            difficulty: "normal".into(),
            server_identity: None,
            exhibits: Vec::new(),
            world_rules: protocol::WorldRules::default(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            chunk_note_radius: 0,
        }
    }

    fn flat_sand_rules() -> protocol::WorldRules {
        protocol::WorldRules {
            world_type: "flat".into(),
            ground: "sand".into(),
            mobs_enabled: false,
            keep_inventory: true,
            ..protocol::WorldRules::default()
        }
    }

    #[test]
    fn join_accept_queues_the_hosts_world_for_the_loader() {
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        let mut acc = proofless_accept();
        acc.seed = 4242;
        acc.spawn_x = 100.5;
        acc.spawn_y = 41.0;
        acc.spawn_z = -7.5;
        acc.world_rules = flat_sand_rules();
        srv.send_to_client(&protocol::serialize_packet(PacketType::JoinAccept, &acc));
        rc.poll();
        assert_eq!(
            rc.pending_joined_world,
            Some(JoinedWorld {
                seed: 4242,
                rules: flat_sand_rules(),
                worldgen_version: crate::world::worldgen_fingerprint(),
                spawn: Some(glam::Vec3::new(100.5, 41.0, -7.5)),
                chunk_note_radius: 0,
            })
        );
    }

    #[test]
    fn joined_world_meta_is_built_from_the_accept() {
        let mut acc = proofless_accept();
        acc.seed = 99;
        acc.world_rules = flat_sand_rules();
        let meta = JoinedWorld::from_accept(&acc).to_meta();
        assert_eq!(meta.seed, 99, "the host's seed, not a random one");
        assert_eq!(meta.world_type, "flat");
        assert_eq!(meta.ground, "sand");
        assert!(!meta.mobs_enabled);
        assert!(meta.keep_inventory);
        assert!(meta.pop_secret.is_some(), "a blank meta never needs saving back");
    }

    #[test]
    fn a_non_finite_join_spawn_is_dropped() {
        let mut acc = proofless_accept();
        acc.spawn_y = f32::NAN;
        assert_eq!(JoinedWorld::from_accept(&acc).spawn, None);
        acc.spawn_y = f32::INFINITY;
        assert_eq!(JoinedWorld::from_accept(&acc).spawn, None);
    }

    fn accept_at(x: f32, y: f32, z: f32) -> protocol::JoinAcceptPacket {
        let mut acc = proofless_accept();
        acc.spawn_x = x;
        acc.spawn_y = y;
        acc.spawn_z = z;
        acc
    }

    #[test]
    fn a_spawn_inside_the_range_is_not_refused() {
        for (x, y, z) in [
            (0.0, 64.0, 0.0),
            (MAX_JOIN_SPAWN_HORIZONTAL, MIN_JOIN_SPAWN_Y, -MAX_JOIN_SPAWN_HORIZONTAL),
            (-MAX_JOIN_SPAWN_HORIZONTAL, MAX_JOIN_SPAWN_Y, MAX_JOIN_SPAWN_HORIZONTAL),
        ] {
            assert_eq!(join_spawn_refusal(&accept_at(x, y, z)), None, "({x}, {y}, {z})");
        }
    }

    #[test]
    fn a_huge_but_finite_spawn_is_refused() {
        for (x, y, z) in [
            (1.0e10, 64.0, 0.0),
            (0.0, 64.0, -1.0e10),
            (MAX_JOIN_SPAWN_HORIZONTAL + 100.0, 64.0, 0.0),
            (0.0, MAX_JOIN_SPAWN_Y + 1.0, 0.0),
            (0.0, MIN_JOIN_SPAWN_Y - 1.0, 0.0),
            (0.0, 1.0e9, 0.0),
            (f32::MAX, f32::MAX, f32::MAX),
        ] {
            assert_eq!(
                join_spawn_refusal(&accept_at(x, y, z)),
                Some(HOST_BAD_SPAWN),
                "({x}, {y}, {z})"
            );
        }
    }

    /// Non-finite spawns keep their old treatment (dropped, not refused): see
    /// `a_non_finite_join_spawn_is_dropped`.
    #[test]
    fn a_non_finite_spawn_is_not_a_range_refusal() {
        assert_eq!(join_spawn_refusal(&accept_at(f32::NAN, 64.0, 0.0)), None);
        assert_eq!(join_spawn_refusal(&accept_at(0.0, f32::INFINITY, 0.0)), None);
    }

    /// End to end through `poll`: a hostile host's out-of-range spawn refuses
    /// the join with a readable reason and queues nothing for the loader.
    #[test]
    fn join_accept_with_an_out_of_range_spawn_refuses_the_join() {
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        srv.send_to_client(&protocol::serialize_packet(
            PacketType::JoinAccept,
            &accept_at(2.0e9, 64.0, 0.0),
        ));
        assert!(rc.poll());
        assert!(
            matches!(&rc.state, ConnectionState::Failed(reason) if reason == HOST_BAD_SPAWN),
            "the join is refused with the readable reason"
        );
        assert_eq!(rc.pending_joined_world, None, "nothing reaches the world loader");
        assert_eq!(rc.spawn_pos, None);
        assert_eq!(
            join_gate(&rc.state, rc.pending_joined_world.take(), 0.0),
            JoinGate::Ended(HOST_BAD_SPAWN.to_string()),
            "the loading screen leaves with the readable reason"
        );
    }

    #[test]
    fn worldgen_mismatch_notice_only_for_another_version() {
        let mut acc = proofless_accept();
        assert_eq!(JoinedWorld::from_accept(&acc).worldgen_mismatch_notice(), None);
        acc.worldgen_version = crate::world::worldgen_fingerprint() ^ 1;
        assert_eq!(
            JoinedWorld::from_accept(&acc).worldgen_mismatch_notice(),
            Some(WORLDGEN_MISMATCH_NOTICE)
        );
    }

    #[test]
    fn join_gate_waits_then_builds_then_times_out() {
        let world = JoinedWorld::from_accept(&proofless_accept());
        assert_eq!(join_gate(&ConnectionState::Connecting, None, 1.0), JoinGate::Waiting);
        assert_eq!(
            join_gate(&ConnectionState::Connected { player_index: 1, seed: 1 }, Some(world.clone()), 1.0),
            JoinGate::Ready(world.clone())
        );
        assert_eq!(
            join_gate(&ConnectionState::Connecting, None, JOIN_ACCEPT_TIMEOUT_SECS),
            JoinGate::Ended(HOST_NO_ANSWER.to_string())
        );
        // A late JoinAccept still wins over the clock.
        assert_eq!(
            join_gate(&ConnectionState::Connecting, Some(world.clone()), 500.0),
            JoinGate::Ready(world)
        );
    }

    #[test]
    fn join_gate_ends_on_a_refusal_or_a_lost_link() {
        assert_eq!(
            join_gate(&ConnectionState::Failed("Sign-in required".into()), None, 0.0),
            JoinGate::Ended("Sign-in required".into())
        );
        assert_eq!(
            join_gate(&ConnectionState::Disconnected, None, 0.0),
            JoinGate::Ended(HOST_CONNECTION_LOST.to_string())
        );
    }

    /// C1 — a client that pinned an operator (`#op=`) must refuse a server that
    /// offers no identity proof. Native only (verification needs the nostr crate).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pinned_client_refuses_join_without_proof() {
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Pinned", 0),
            Some("npub1pinnedoperatorexample".to_string()),
        );
        srv.send_to_client(&protocol::serialize_packet(
            PacketType::JoinAccept,
            &proofless_accept(),
        ));
        rc.poll();
        assert!(
            matches!(rc.state, ConnectionState::Failed(_)),
            "pinned client must refuse an unproven server"
        );
    }

    /// An anonymous (no `#op=`) client still joins a proofless server exactly as
    /// before — the default zero-config path is unchanged.
    #[test]
    fn unpinned_client_accepts_proofless_join() {
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Anon", 0),
            None,
        );
        srv.send_to_client(&protocol::serialize_packet(
            PacketType::JoinAccept,
            &proofless_accept(),
        ));
        rc.poll();
        assert!(
            matches!(rc.state, ConnectionState::Connected { .. }),
            "anonymous join must be unaffected"
        );
    }

    #[test]
    fn authed_join_refuses_to_sign_a_non_nonce_challenge() {
        let (srv, client) = channel_pair();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_d = called.clone();
        let driver: SignDriverFn = Box::new(move |_n, _o| {
            called_d.store(true, std::sync::atomic::Ordering::SeqCst);
            std::sync::mpsc::channel().1
        });
        let mut rc = RemoteClient::from_transport_authed(
            Box::new(client),
            build_join_request_guest("Axo", 0),
            driver,
            None,
        );
        // e.g. a website CSRF token a malicious host wants signed.
        let chal = protocol::ChallengePacket { nonce_hex: "site-csrf-token".into() };
        srv.send_to_client(&protocol::serialize_packet(PacketType::Challenge, &chal));
        rc.poll();
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst), "bunker must not be asked");
        assert!(matches!(rc.state, ConnectionState::Failed(_)));
        assert!(read_join_request(&srv).is_none());
    }

    #[test]
    fn authed_join_fails_cleanly_when_signer_errors() {
        let (srv, client) = channel_pair();
        let driver: SignDriverFn = Box::new(|_nonce, _origin| {
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(Err("bunker offline".to_string())).unwrap();
            rx
        });
        let mut rc = RemoteClient::from_transport_authed(
            Box::new(client),
            build_join_request_guest("Axo", 0),
            driver,
            None,
        );
        let chal = protocol::ChallengePacket {
            nonce_hex: "b".repeat(64),
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::Challenge, &chal));
        rc.poll();
        rc.poll();

        assert!(matches!(rc.state, ConnectionState::Failed(_)), "signer error → Failed");
        assert!(read_join_request(&srv).is_none(), "no JoinRequest on signer failure");
    }

    // ── MP-A3: death + respawn are server-held ───────────────────────────────

    fn joined_as(index: u32) -> (crate::transport::ChannelServerTransport, RemoteClient) {
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        let accept = protocol::JoinAcceptPacket { player_index: index, ..proofless_accept() };
        srv.send_to_client(&protocol::serialize_packet(PacketType::JoinAccept, &accept));
        rc.poll();
        assert!(rc.is_connected());
        while srv.try_recv_from_client().is_some() {}
        (srv, rc)
    }

    fn life_event(srv: &dyn ServerTransport, index: u32, event: protocol::PlayerEventType) {
        // C3a-fix-1 — an armour wear is window event 6.
        let window_event = if matches!(event, protocol::PlayerEventType::ArmourWorn { .. }) { 6 } else { 0 };
        let ev = protocol::PlayerEventPacket { player_index: index, event, window_event };
        srv.send_to_client(&protocol::serialize_packet(PacketType::PlayerEvent, &ev));
    }

    #[test]
    fn own_death_and_respawn_events_are_queued_for_the_game_loop() {
        let (srv, mut rc) = joined_as(3);
        life_event(&srv, 3, protocol::PlayerEventType::Died);
        // Someone else's death is theirs to show, not ours to act on.
        life_event(&srv, 4, protocol::PlayerEventType::Died);
        life_event(&srv, 3, protocol::PlayerEventType::Respawned { x: 1.5, y: 70.0, z: -2.5 });
        rc.poll();
        assert_eq!(
            std::mem::take(&mut rc.pending_life_events),
            vec![
                OwnLifeEvent::Died(crate::survival::DamageCause::Generic),
                OwnLifeEvent::Respawned(glam::Vec3::new(1.5, 70.0, -2.5)),
            ]
        );
    }

    /// MP-D2b — the server names the cause, and the hits it landed wear our
    /// armour; another player's events are not ours.
    #[test]
    fn a_died_of_names_the_cause_and_armour_wear_is_ours_alone() {
        let (srv, mut rc) = joined_as(3);
        life_event(&srv, 3, protocol::PlayerEventType::ArmourWorn { hits: 2 });
        life_event(&srv, 4, protocol::PlayerEventType::ArmourWorn { hits: 5 });
        life_event(
            &srv,
            3,
            protocol::PlayerEventType::DiedOf {
                cause: protocol::WireDamageCause::Mob(protocol::EntityKind::Brigand),
            },
        );
        rc.poll();
        assert_eq!(
            std::mem::take(&mut rc.pending_life_events),
            vec![
                OwnLifeEvent::ArmourWorn(2, 6),
                OwnLifeEvent::Died(crate::survival::DamageCause::Mob(crate::mob::MobType::Brigand)),
            ]
        );
    }

    /// Review D2a-verify N1 — a `Died` that arrives while our `Respawn` is
    /// unanswered is about the life we already left (the server sent it
    /// before it handled the Respawn, and the stream is ordered): ignored, or
    /// it would kill the respawned client a second time. A death after the
    /// server's `Respawned` is a new one, and is taken.
    #[test]
    fn a_stale_died_before_respawned_is_ignored_and_a_new_death_after_it_is_not() {
        let (srv, mut rc) = joined_as(1);
        life_event(&srv, 1, protocol::PlayerEventType::Died);
        rc.poll();
        assert_eq!(std::mem::take(&mut rc.pending_life_events).len(), 1, "the real death");
        // The death screen's Respawn goes out; the echo of the death we
        // already took arrives before the server's answer.
        rc.send_respawn();
        life_event(&srv, 1, protocol::PlayerEventType::DiedOf { cause: protocol::WireDamageCause::Fall });
        rc.poll();
        assert!(rc.pending_life_events.is_empty(), "a stale Died is dropped");
        // The answer, then a genuine new death.
        life_event(&srv, 1, protocol::PlayerEventType::Respawned { x: 0.5, y: 70.0, z: 0.5 });
        life_event(&srv, 1, protocol::PlayerEventType::DiedOf { cause: protocol::WireDamageCause::Lava });
        rc.poll();
        assert_eq!(
            std::mem::take(&mut rc.pending_life_events),
            vec![
                OwnLifeEvent::Respawned(glam::Vec3::new(0.5, 70.0, 0.5)),
                OwnLifeEvent::Died(crate::survival::DamageCause::Lava),
            ]
        );
    }

    /// MP-D2b — outcomes and kill events are queued for the game loop.
    #[test]
    fn outcomes_and_kills_are_queued_for_the_game_loop() {
        let (srv, mut rc) = joined_as(0);
        let out = protocol::InteractOutcomePacket {
            seq: 4,
            entity: 9,
            kind: Some(protocol::InteractKind::Milk),
            accepted: true,
            consume_held: 1,
            note: 2,
            window_event: 3,
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::InteractOutcome, &out));
        let kill = protocol::KillEventPacket {
            victim: protocol::EntityKind::Cow,
            reason: protocol::kill_reason::LAST_HIT,
            x: 1.0,
            y: 64.0,
            z: 2.0,
            victim_flags: 0,
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::KillEvent, &kill));
        // C2a — an item action's answer joins the same queue, in order.
        let item = protocol::ItemActionOutcomePacket { seq: 5, accepted: false, consume_held: 0, note: 3, window_event: 0, wear_held: false, bite_after: 0 };
        srv.send_to_client(&protocol::serialize_packet(PacketType::ItemActionOutcome, &item));
        rc.poll();
        assert_eq!(
            std::mem::take(&mut rc.pending_outcomes),
            vec![RequestOutcome::Interact(out), RequestOutcome::Item(item)]
        );
        assert_eq!(std::mem::take(&mut rc.pending_kills), vec![kill]);
    }

    #[test]
    fn a_respawned_event_with_a_non_finite_position_is_dropped() {
        let (srv, mut rc) = joined_as(0);
        life_event(&srv, 0, protocol::PlayerEventType::Respawned { x: f32::NAN, y: 0.0, z: 0.0 });
        rc.poll();
        assert!(rc.pending_life_events.is_empty());
    }

    #[test]
    fn send_respawn_asks_the_server_once_joined() {
        let (srv, client) = channel_pair();
        let mut rc = RemoteClient::from_transport(
            Box::new(client),
            build_join_request_guest("Me", 0),
            None,
        );
        while srv.try_recv_from_client().is_some() {}
        rc.send_respawn();
        assert!(srv.try_recv_from_client().is_none(), "nothing before the join completes");

        let (srv, mut rc) = joined_as(2);
        rc.send_respawn();
        let pkt = srv.try_recv_from_client().expect("a Respawn request");
        let (ptype, _) = protocol::deserialize_header(&pkt).unwrap();
        assert_eq!(ptype, PacketType::Respawn);
    }

    /// Respawn requests the server has been sent since the last call.
    fn respawn_requests_seen(srv: &crate::transport::ChannelServerTransport) -> usize {
        let mut n = 0;
        while let Some(pkt) = srv.try_recv_from_client() {
            if protocol::deserialize_header(&pkt).is_some_and(|(t, _)| t == PacketType::Respawn) {
                n += 1;
            }
        }
        n
    }

    fn input_ticks(rc: &mut RemoteClient, n: u32) {
        for _ in 0..n {
            rc.send_input(&protocol::InputPacket::default());
        }
    }

    #[test]
    fn a_respawn_request_is_resent_every_20_ticks_until_the_server_answers() {
        // The server drops everything past a client's 10th packet in a tick,
        // and a lost Respawn would leave the joiner walking about its own
        // respawned body while the server holds it dead for good.
        let (srv, mut rc) = joined_as(2);
        rc.send_respawn();
        assert_eq!(respawn_requests_seen(&srv), 1, "sent at once");

        input_ticks(&mut rc, 19);
        assert_eq!(respawn_requests_seen(&srv), 0, "not before the 20th tick");
        input_ticks(&mut rc, 1);
        assert_eq!(respawn_requests_seen(&srv), 1, "re-sent on the 20th");
        input_ticks(&mut rc, 40);
        assert_eq!(respawn_requests_seen(&srv), 2, "and every 20 after");

        // Someone else's respawn is not the answer.
        life_event(&srv, 5, protocol::PlayerEventType::Respawned { x: 0.5, y: 70.0, z: 0.5 });
        rc.poll();
        input_ticks(&mut rc, 20);
        assert_eq!(respawn_requests_seen(&srv), 1, "still asking");

        // Ours is.
        life_event(&srv, 2, protocol::PlayerEventType::Respawned { x: 0.5, y: 70.0, z: 0.5 });
        rc.poll();
        input_ticks(&mut rc, 60);
        assert_eq!(respawn_requests_seen(&srv), 0, "answered — no more requests");
    }

    #[test]
    fn the_respawn_resend_stops_on_disconnect() {
        let (srv, mut rc) = joined_as(1);
        rc.send_respawn();
        assert_eq!(respawn_requests_seen(&srv), 1);
        rc.disconnect();
        input_ticks(&mut rc, 60);
        assert_eq!(respawn_requests_seen(&srv), 0, "a disconnected client asks nothing");
    }

    #[test]
    fn a_new_death_cancels_a_pending_respawn_resend() {
        // Respawned, then died again before the answer came: a late resend
        // would respawn the body out from under the new death screen.
        let (srv, mut rc) = joined_as(1);
        rc.send_respawn();
        assert_eq!(respawn_requests_seen(&srv), 1);
        rc.cancel_respawn_resend();
        input_ticks(&mut rc, 60);
        assert_eq!(respawn_requests_seen(&srv), 0);
    }

    #[test]
    fn a_respawned_event_outside_the_world_is_dropped_like_a_bad_join_spawn() {
        // The same range the JoinAccept spawn is held to: a hostile host could
        // otherwise teleport us anywhere finite and overflow the chunk maths.
        for (x, y, z) in [
            (1.0e9, 70.0, 0.5),
            (0.5, 70.0, -1.0e9),
            (0.5, -10_000.0, 0.5),
            (0.5, 1.0e6, 0.5),
        ] {
            let (srv, mut rc) = joined_as(0);
            life_event(&srv, 0, protocol::PlayerEventType::Respawned { x, y, z });
            rc.poll();
            assert!(
                rc.pending_life_events.is_empty(),
                "({x}, {y}, {z}) is outside the world and must not be queued"
            );
        }
        let (srv, mut rc) = joined_as(0);
        life_event(&srv, 0, protocol::PlayerEventType::Respawned { x: 25.5, y: 64.0, z: -40.5 });
        rc.poll();
        assert_eq!(
            rc.pending_life_events,
            vec![OwnLifeEvent::Respawned(glam::Vec3::new(25.5, 64.0, -40.5))],
            "an ordinary respawn still goes through"
        );
    }

    // ── B2a: pushed chunks ──────────────────────────────────────────────

    fn chunk_pkt(cx: i32) -> Vec<u8> {
        let p = protocol::ChunkDataPacket {
            cx,
            cy: 0,
            cz: 0,
            compressed_blocks: vec![1],
            meta: Vec::new(),
            entities: Vec::new(),
            attachments: Vec::new(),
        };
        protocol::serialize_packet(PacketType::ChunkData, &p)
    }

    #[test]
    fn every_pushed_chunk_is_queued_in_line_with_the_block_changes() {
        let (srv, client) = channel_pair();
        let mut rc =
            RemoteClient::from_transport(Box::new(client), build_join_request_guest("Guest", 0), None);
        let state = |xs: &[i32]| protocol::StateUpdatePacket {
            tick: 1,
            players: Vec::new(),
            block_changes: xs.iter().map(|&x| protocol::BlockChange::with_meta(x, 0, 0, 1, 0)).collect(),
            world_time: 0,
            last_acked_input: 0,
            entity_spawns: Vec::new(),
            entity_updates: Vec::new(),
            entity_despawns: Vec::new(),
            reserve_richness: 1.0,
            reserve_target_sats: 0,
            reserve_current_sats: 0,
            rain_ticks_left: 0,
            storm_ticks_left: 0,
            own_hunger: 0,
            block_views: Vec::new(),
        };
        srv.send_to_client(&protocol::serialize_packet(PacketType::StateUpdate, &state(&[1, 2])));
        // More than the old 256-packet cap, which dropped the rest silently.
        for cx in 0..300 {
            srv.send_to_client(&chunk_pkt(cx));
        }
        srv.send_to_client(&protocol::serialize_packet(PacketType::StateUpdate, &state(&[3])));
        rc.poll();
        assert_eq!(rc.chunk_queue.len(), 300, "nothing dropped");
        assert!(rc.chunk_queue.iter().all(|(before, _)| *before == 2), "each after the first two changes");
        assert_eq!(rc.pending_block_changes.len(), 3);
        assert!(!matches!(rc.state, ConnectionState::Failed(_)));
    }

    #[test]
    fn a_column_local_note_is_queued_in_line_with_the_pushes() {
        let (srv, client) = channel_pair();
        let mut rc =
            RemoteClient::from_transport(Box::new(client), build_join_request_guest("Guest", 0), None);
        srv.send_to_client(&chunk_pkt(1));
        srv.send_to_client(&crate::chunk_push::build_local_note((5, -6), 0xABCD));
        srv.send_to_client(&chunk_pkt(2));
        rc.poll();
        let shape: Vec<String> = rc
            .chunk_queue
            .iter()
            .map(|(_, item)| match item {
                crate::chunk_intake::StreamItem::Chunk(p) => format!("C{}", p.cx),
                crate::chunk_intake::StreamItem::Local((x, z), hash) => format!("L{x},{z}#{hash:x}"),
                crate::chunk_intake::StreamItem::View(v) => format!("V{}", v.cell[0]),
            })
            .collect();
        assert_eq!(shape, ["C1", "L5,-6#abcd", "C2"]);
    }

    #[test]
    fn a_column_local_note_that_does_not_decode_ends_the_session() {
        // B2b review LOW-1: its column is unknown, so it could only leave the
        // joiner waiting on a column for good.
        let (srv, client) = channel_pair();
        let mut rc =
            RemoteClient::from_transport(Box::new(client), build_join_request_guest("Guest", 0), None);
        srv.send_to_client(&chunk_pkt(1));
        srv.send_to_client(&[PacketType::ColumnLocal as u8, 1, 2, 3]);
        assert!(rc.poll());
        assert!(
            matches!(&rc.state, ConnectionState::Failed(why) if why == HOST_BAD_WORLD_DATA),
            "the session ends with a reason"
        );
    }

    #[test]
    fn a_server_past_any_credit_window_ends_the_session_loudly() {
        let (srv, client) = channel_pair();
        let mut rc =
            RemoteClient::from_transport(Box::new(client), build_join_request_guest("Guest", 0), None);
        for cx in 0..=MAX_QUEUED_CHUNK_PACKETS as i32 {
            srv.send_to_client(&chunk_pkt(cx));
        }
        rc.poll();
        assert_eq!(rc.chunk_queue.len(), MAX_QUEUED_CHUNK_PACKETS, "what was taken in is kept");
        assert!(
            matches!(&rc.state, ConnectionState::Failed(why) if why.contains("more world data")),
            "the session ends with a reason instead of losing part of the world"
        );
    }

    // ── C3b-fix-b: one send order, numbered carriers never dropped ─────────

    fn drop_request(seq: u32) -> Request {
        Request::Item(protocol::ItemActionPacket {
            seq,
            action: protocol::ItemAction::Drop {
                hotbar_slot: 0,
                held_kind: 0,
                held_id: 0,
                held_full: protocol::WireItem::None,
            },
            events_applied: 0,
        })
    }

    /// The packet types the server end holds, in the order they were sent.
    fn packet_types(srv: &dyn ServerTransport) -> Vec<PacketType> {
        std::iter::from_fn(|| srv.try_recv_from_client())
            .map(|pkt| protocol::deserialize_header(&pkt).expect("a packet").0)
            .collect()
    }

    /// B-M1 — a request made while edits were unsent is held, and goes right
    /// after the input that carries them: the server reads the placement,
    /// then the Q-drop.
    #[test]
    fn a_request_behind_unsent_edits_goes_out_after_the_input_carrying_them() {
        let (srv, mut rc) = connected_client();
        rc.queue_request(10, drop_request(0));
        assert!(rc.has_queued_requests());
        assert!(packet_types(&*srv).is_empty(), "queued, not sent");
        rc.send_input(&input_with([1, 2]));
        // The edits all went, so the request follows at once.
        let due = rc.take_queued_requests();
        assert_eq!(due.iter().map(|(stamp, _)| *stamp).collect::<Vec<_>>(), vec![10]);
        for (_, request) in due {
            rc.send_request(request);
        }
        assert_eq!(packet_types(&*srv), vec![PacketType::ClientInput, PacketType::ItemAction]);
        assert!(!rc.has_queued_requests());
    }

    /// B-L2 / C3b-fix-d (A-L3) — edits waiting in the carry-over keep their
    /// own stamps, and the first of them is the cut: a request made after it
    /// waits for the input that carries it, and is released once every edit
    /// made before it has gone.
    #[test]
    fn the_carry_over_keeps_each_edits_stamp_and_holds_requests_made_after_the_first_waiting() {
        let (srv, mut rc) = connected_client();
        assert_eq!(rc.first_carried_stamp(), None);
        // A burst stamped 1..=10_000, too big for one packet.
        rc.note_edit_stamps((1..=10_000).collect());
        rc.send_input(&input_with(0..10_000));
        let sent = 10_000 - rc.input_carry_over.len() as u64;
        let cut = rc.first_carried_stamp().expect("the burst was trimmed");
        assert_eq!(cut, sent + 1, "the first WAITING edit's own stamp, not the burst's first");
        rc.queue_request(10_001, drop_request(0));
        // A later input's own edit goes behind the burst; the cut moves on
        // with the burst's edits as they go.
        rc.note_edit_stamps(vec![10_002]);
        rc.send_input(&input_with([50_000]));
        let next = rc.first_carried_stamp().expect("still trimmed");
        assert!(next > cut && next < 10_001, "the burst's next waiting edit: {next}");
        let mut guard = 0;
        while rc.first_carried_stamp().is_some_and(|stamp| stamp < 10_001) {
            assert!(rc.take_queued_requests().is_empty(), "made after edits that still wait");
            assert!(rc.has_queued_requests(), "kept for later");
            rc.send_input(&protocol::InputPacket::default());
            guard += 1;
            assert!(guard < 100, "the carry-over drains");
        }
        assert_eq!(rc.take_queued_requests().len(), 1, "released once the edits made before it went");
        let _ = srv;
    }

    /// The packets the server end holds, in order: each one's type, and for
    /// a `ClientInput` its block changes (x, new block) and tagged cells.
    fn sent_in_order(srv: &dyn ServerTransport) -> Vec<(PacketType, Vec<(i32, u16)>, usize)> {
        std::iter::from_fn(|| srv.try_recv_from_client())
            .map(|pkt| {
                let (ptype, payload) = protocol::deserialize_header(&pkt).expect("a packet");
                if ptype != PacketType::ClientInput {
                    return (ptype, Vec::new(), 0);
                }
                let input: protocol::InputPacket = protocol::safe_deserialize(payload).unwrap();
                let edits = input.block_changes.iter().map(|b| (b.x, b.new_block)).collect();
                (ptype, edits, input.mined.len())
            })
            .collect()
    }

    /// C3b-fix-d (A-L3) — each edit keeps its own order stamp, so the
    /// carry-over's cut is the first WAITING edit's: in one tick a placement
    /// at X (e1, stamp 1), a Q-drop (queued, stamp 2), then a tagged break of
    /// X (e2, stamp 3, held back behind the untagged placement of its cell)
    /// reach the server as e1, the drop, e2: the client's order. (The cut
    /// used to be e1's stamp, so the drop waited for e2.)
    #[test]
    fn a_request_made_between_a_sent_edit_and_a_held_one_goes_between_them() {
        let (srv, mut rc) = connected_client();
        let place = protocol::BlockChange { x: 3, y: 70, z: 0, new_block: crate::block::STONE, meta: 0 };
        let brk = protocol::BlockChange { x: 3, y: 70, z: 0, new_block: crate::block::AIR, meta: 0 };
        let input = protocol::InputPacket {
            block_changes: vec![place, brk],
            mined: vec![protocol::MinedBlock { x: 3, y: 70, z: 0, tool: protocol::WireItem::None }],
            ..Default::default()
        };
        rc.queue_request(2, drop_request(0));
        rc.note_edit_stamps(vec![1, 3]);
        rc.send_input(&input);
        assert!(rc.has_carry_over(), "the tagged break waits behind the placement of its cell");
        assert_eq!(rc.first_carried_stamp(), Some(3), "with its own stamp, not the placement's");
        for (_, request) in rc.take_queued_requests() {
            rc.send_request(request);
        }
        rc.send_input(&protocol::InputPacket::default());
        assert_eq!(
            sent_in_order(&*srv),
            vec![
                (PacketType::ClientInput, vec![(3, crate::block::STONE)], 0),
                (PacketType::ItemAction, Vec::new(), 0),
                (PacketType::ClientInput, vec![(3, crate::block::AIR)], 1),
            ],
            "e1, the drop, e2"
        );
    }

    /// C3b-fix-d (A-L1) — a closed link's queued requests are all handed
    /// back for discarding, even while edits still wait in the carry-over
    /// (`take_queued_requests` holds them then), so their claims can be
    /// released.
    #[test]
    fn a_discard_takes_every_queued_request_even_behind_a_carry_over() {
        let (srv, mut rc) = connected_client();
        rc.send_input(&input_with(0..10_000));
        assert!(rc.has_carry_over());
        rc.queue_request(u64::MAX - 1, drop_request(7));
        assert!(rc.take_queued_requests().is_empty(), "held behind the carry-over");
        let discarded = rc.discard_queued_requests();
        assert_eq!(discarded.iter().map(|(_, r)| r.seq()).collect::<Vec<_>>(), vec![Some(7)]);
        assert!(!rc.has_queued_requests());
        assert_eq!(Request::Device((1, 2, 3)).seq(), None, "a device right-click claims nothing");
        let _ = srv;
    }

    fn numbered_grant(window_event: u32) -> protocol::InventoryGrantPacket {
        protocol::InventoryGrantPacket {
            item_kind: protocol::item_kind::MATERIAL,
            item_id: 4,
            count: 1,
            full_item: protocol::WireItem::None,
            window_event,
        }
    }

    /// B-L1 — a numbered carrier is never dropped by the per-poll caps: a
    /// hitch that batches 300 grants into one poll applies all 300 (the
    /// server applies its side of each number the client reports). The caps
    /// still bound the unnumbered ones.
    #[test]
    fn three_hundred_numbered_grants_in_one_poll_are_all_kept() {
        let (srv, mut rc) = joined_as(3);
        for n in 1..=300 {
            srv.send_to_client(&protocol::serialize_packet(PacketType::InventoryGrant, &numbered_grant(n)));
        }
        rc.poll();
        assert_eq!(rc.pending_grants.len(), 300);
        assert_eq!(rc.pending_grants.last().map(|g| g.window_event), Some(300), "in order, none skipped");
        rc.pending_grants.clear();
        for _ in 0..300 {
            srv.send_to_client(&protocol::serialize_packet(PacketType::InventoryGrant, &numbered_grant(0)));
        }
        rc.poll();
        assert_eq!(rc.pending_grants.len(), MAX_UNNUMBERED_PER_POLL, "unnumbered ones keep the cap");
    }

    /// B-L1 — the same for the outcomes (a furnace screen pushes every tick)
    /// and for the own-life events: armour wear is numbered, a death is not.
    #[test]
    fn numbered_outcomes_and_armour_wear_survive_a_long_hitch() {
        let (srv, mut rc) = joined_as(3);
        for n in 1..=300 {
            let set = protocol::WindowSlotSetPacket {
                op_seq_applied: 0,
                reason: protocol::slot_set_reason::CORRECTION,
                sets: Vec::new(),
                furnace: None,
                window_event: n,
                take: Vec::new(),
                give: Vec::new(),
            };
            srv.send_to_client(&protocol::serialize_packet(PacketType::WindowSlotSet, &set));
        }
        for _ in 0..40 {
            life_event(&srv, 3, protocol::PlayerEventType::Died);
        }
        for n in 1..=40u32 {
            let ev = protocol::PlayerEventPacket {
                player_index: 3,
                event: protocol::PlayerEventType::ArmourWorn { hits: 1 },
                window_event: 1000 + n,
            };
            srv.send_to_client(&protocol::serialize_packet(PacketType::PlayerEvent, &ev));
        }
        rc.poll();
        assert_eq!(rc.pending_outcomes.len(), 300);
        let worn = rc.pending_life_events.iter().filter(|e| matches!(e, OwnLifeEvent::ArmourWorn(..))).count();
        let died = rc.pending_life_events.iter().filter(|e| matches!(e, OwnLifeEvent::Died(_))).count();
        assert_eq!(worn, 40, "every numbered wear");
        assert_eq!(died, MAX_LIFE_EVENTS_PER_POLL, "the unnumbered ones keep their cap");
    }
}

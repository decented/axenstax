//! Hosted server — wraps `GameServer` with per-client transports and drives
//! the authoritative simulation from the main loop.
//!
//! Historically this spawned a dedicated thread that ran `GameServer` at
//! 20 TPS and talked to clients via mpsc channels. That model doesn't port
//! to wasm32-unknown-unknown (no `std::thread::spawn`), and is incompatible
//! with the Chromium-only PWA alpha target without nightly Rust +
//! SharedArrayBuffer + COOP/COEP headers.
//!
//! The current model is main-loop-driven: `HostedServer::tick()` runs one
//! authoritative tick of the simulation, poll all transports, broadcast
//! state, and return. The game loop calls it at the same 20 TPS cadence
//! it already uses for the client sim. QUIC/LAN discovery for remote
//! players remain native-only and run on their own thread; WASM builds
//! always host with `max_remote_players = 0`.
//!
//! Call shape:
//!   let mut hs = HostedServer::start(num_local, name, seed, max_remote)?;
//!   // 20 TPS accumulator in the main loop:
//!   hs.tick();
//!
//! Dropping the HostedServer shuts down the QUIC accept thread if it was
//! spawned (native-only).

use std::collections::VecDeque;

use crate::protocol;
#[cfg(not(target_arch = "wasm32"))]
use crate::signet;
use crate::transport::{self, ChannelClientTransport, ServerTransport};

/// MP-D2b — how far past `combat::ATTACK_REACH` a joiner's swing or
/// right-click may land, measured from the eye of the body the SERVER holds
/// to the target's centre. Covers the round trip between where the client
/// drew the mob and where the server has it now (a mob walks ~0.2 blocks a
/// tick), plus the body's own prediction error (a snap only past 1 block).
pub const ATTACK_REACH_TOLERANCE: f32 = 1.5;

/// MP-D2b — how early a joiner's swing may arrive on the server's schedule
/// (`ServerPlayer::next_swing_tick`, review D2b LOW-2): two swings the client
/// spaced a full `combat::ATTACK_COOLDOWN` apart are never refused for
/// arriving a tick or two closer together, but each accepted swing moves the
/// schedule a full cooldown on, so the long-run rate is the client's. A
/// second swing in the same tick is always refused.
pub const ATTACK_COOLDOWN_JITTER_TICKS: u32 = 3;

/// MP-D2b — the server's cooldown between a joiner's one-shot interactions
/// (`EntityInteract`): the client's 8-tick right-click cooldown less jitter.
/// Bounds a modified client's tame rolls (the food it claims is its word).
pub const INTERACT_COOLDOWN_TICKS: u32 = 6;

/// MP-D2b — `EntityAttack` + `EntityInteract` requests read per client per
/// tick (their own budget, not the block-change one's). FU3 — the rest WAIT,
/// with everything the client sent after them (arrival order is kept): a
/// catch-up reads up to [`CATCH_UP_PACKETS_PER_TICK`] packets a tick, which
/// span about 64 client ticks — six swings (10-tick `combat::ATTACK_COOLDOWN`)
/// and eight right-clicks (8-tick place cooldown) — so an honest stall dump
/// does reach it, and skipping the excess (FU1's rule, safe only at ten
/// packets a tick) would leave honest requests unanswered.
const MAX_ENTITY_REQUESTS_PER_TICK: usize = 4;

/// C2a — `ItemAction` requests (eat, sleep; C2b craft, drop) read per client
/// per tick: their own budget, not [`MAX_ENTITY_REQUESTS_PER_TICK`]'s. One
/// past it WAITS for the next tick in the client's inbound queue (FU1), with
/// everything sent after it: never dropped, never refused (FU3: as an entity
/// request or a device interaction past its own budget now does). No honest
/// client reaches it (an eat every 16 ticks, `item_actions::EAT_COOLDOWN_TICKS`;
/// a sleep once a night; a drop every `item_actions::DROP_INTERVAL_TICKS`; a
/// craft per click). C2b — a drop also waits while the joiner's drop bucket
/// is empty (`item_actions::DropBucket`).
const MAX_ITEM_ACTIONS_PER_TICK: usize = 4;

/// C2a — is `packet` an `ItemAction` (budgeted by deferral, not dropping)?
fn is_item_action(packet: &[u8]) -> bool {
    matches!(protocol::deserialize_header(packet), Some((protocol::PacketType::ItemAction, _)))
}

/// Max device interactions per tick per client. A right-click is gated
/// client-side by an 8-tick place cooldown, so legitimate play is well under
/// one a tick; this is the `DeviceInteract` sibling of the block-change budget,
/// which it does NOT share. FU3 — past it the interaction waits for the next
/// tick, in order, like an entity request past [`MAX_ENTITY_REQUESTS_PER_TICK`]
/// (a catch-up's 64 packets can hold eight right-clicks).
const MAX_DEVICE_INTERACTS_PER_TICK: usize = 2;

/// Review D2b LOW-2 — how far ahead of a joiner's server body a target must
/// be for its swing or right-click: `dot(look, to-target) >= 0`, the half
/// space ahead. Lenient on purpose (single-player's pick wants 0.5, 60°): the
/// server holds the look of the joiner's last input, which may lag a quick
/// turn by a round trip.
pub const JOINER_MIN_FACING_DOT: f32 = 0.0;

/// D1 — where a hosted server's world lives (`crate::sim_lend`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostWorld {
    /// The server loads and simulates its own `World` + ECS. The dedicated
    /// server (no host client), and a host started with `--no-lend`.
    Owned,
    /// A host client (LAN Host Game, online host) lends its own world to the
    /// server for each tick (`sim_lend::LentSim`): one world, one simulation,
    /// and joiners are diffed from the host's real world and entities.
    Lent,
}

/// `--no-lend`: the one-release escape hatch back to a host whose embedded
/// server owns a second copy of the world (the pre-D1 behaviour, its
/// host→server block-entity mirror included —
/// [`HostedServer::mirror_host_world_state`]). Set once at startup.
#[cfg(not(target_arch = "wasm32"))]
static NO_LEND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record the `--no-lend` command-line flag (`lib.rs::run`).
#[cfg(not(target_arch = "wasm32"))]
pub fn set_no_lend(no_lend: bool) {
    NO_LEND.store(no_lend, std::sync::atomic::Ordering::Relaxed);
}

/// How a host client's embedded server holds its world: lent, unless the
/// player started with `--no-lend`. Single-player runs no server at all, and
/// the dedicated server always owns its world (`HostedServer::start`).
#[cfg(not(target_arch = "wasm32"))]
pub fn host_world_mode() -> HostWorld {
    if NO_LEND.load(std::sync::atomic::Ordering::Relaxed) {
        HostWorld::Owned
    } else {
        HostWorld::Lent
    }
}

/// One column-loading story per mode, never two generators on one world
/// (Spec 01 §4.1.2-4.1.3):
/// - the dedicated server (0 local players, no host client streaming for it)
///   loads / unloads columns round every connected player + the spawn itself
///   (Phase B1, `server_stream.rs`);
/// - a LAN / online host that keeps its own copy (`--no-lend`) generates
///   terrain ahead of its joiners' bodies (Spec 04 §5.3.1); its host client
///   streams its own;
/// - a host that LENDS its world (D1) does neither: its host client's streamer
///   anchors on every joiner's server body too
///   ([`HostedServer::lent_joiner_columns`] → `chunk_stream::client_stream_anchors`),
///   so it loads and keeps their columns, and a second generator here would
///   write the host's own world behind that streamer's back.
///
/// The streamer is its own flag, not an alias of `simulates_block_machines`: a
/// host lending its world (D4) will tick machines but not stream.
// Reached only from native hosting / the dedicated server.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn assign_column_loading(
    server: &mut crate::server::GameServer,
    num_local_players: usize,
    host_world: HostWorld,
) {
    server.column_streamer =
        (num_local_players == 0).then(crate::server_stream::ColumnStreamer::default);
    server.column_refill_per_tick = if num_local_players > 0 && host_world == HostWorld::Owned {
        crate::server::HOST_COLUMN_REFILL_PER_TICK
    } else {
        0
    };
}

/// Which network transport accepts remote players.
///
/// Existing native LAN peer-hosting ("Host Game") uses QUIC. The dedicated
/// Docker server uses WebSocket — the only transport a browser can speak — so
/// both the web app and native can join one server. Local / WASM hosts spawn
/// no accept thread at all (`max_remote_players == 0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteTransport {
    /// QUIC datagrams on `protocol::SERVER_PORT` (today's native LAN host).
    Quic,
    /// Plain WebSocket on `port` (dedicated server; web + native join).
    WebSocket { port: u16 },
}

#[cfg(not(target_arch = "wasm32"))]
use std::net::SocketAddr;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// How long (20 TPS ticks) a remote slot may sit without completing its join
/// before it is freed. Tied to the join challenge's TTL
/// (`signet::challenge::DEFAULT_TTL_SECS`, 30 s): past it the nonce is dead,
/// so the slot could never authenticate anyway. Deliberately NOT 10 s — a
/// Signet join waits on a human approving on their phone (the bunker's own
/// round-trip timeout is 60 s), so a shorter window would refuse real joins.
const PRE_AUTH_TIMEOUT_TICKS: u64 = 30 * 20;

/// What a kicked player is told before their connection closes.
const KICK_REASON: &str = "You were removed from this world by its operator.";

/// Told to the operator when they join over a transport with no channel
/// binding (WebSocket — TLS ends at the proxy). Such a join could be a relayed
/// replay of the operator's signature, so it never carries operator privileges
/// (Spec 08 §9.0.1, T-JOIN-RELAY). Normal play is unaffected.
const OPERATOR_NEEDS_DIRECT_NOTICE: &str = "Operator tools need a direct connection.";

/// FU1 — the packets the server processes per client per tick (its CPU
/// budget for one client's traffic). Packets past it are NOT dropped: they
/// wait in the slot's `transport::InboundQueue` for the next tick, in arrival
/// order, so an honest client never loses one — a client catching up after a
/// frame hitch sends up to ten inputs a frame (`game_loop` runs at most 10
/// ticks a frame) plus the actions it made, and the swing behind them used to
/// be dropped. Ten covers one such frame. Control packets (`Respawn`,
/// `Disconnect`) cost nothing against it, up to
/// [`FREE_CONTROL_PACKETS_PER_TICK`] a tick. A client with more than
/// [`CATCH_UP_QUEUE_LEN`] waiting gets [`CATCH_UP_PACKETS_PER_TICK`] instead
/// (unless its edit queue is full, FU4a).
/// Memory is bounded by the queue's hard bound (`transport::MAX_INBOUND_BYTES`):
/// only a client past it is disconnected ([`INBOUND_OVERFLOW_REASON`]).
/// Spec 04 §11.2a.
pub const MAX_PACKETS_PER_TICK: usize = 10;

/// FU3 (FU1 verify N1) — a client with more packets than this waiting (about
/// two seconds of an honest client's traffic: it sends about 20 a second) is
/// catching up on a stall — usually the host's own (the host's game thread
/// stopped while each joiner's bridge thread kept queueing), or its own
/// replayed freeze — and gets [`CATCH_UP_PACKETS_PER_TICK`] read that tick.
pub const CATCH_UP_QUEUE_LEN: usize = 40;

/// FU3 — the read budget of a client catching up ([`CATCH_UP_QUEUE_LEN`]): a
/// 55-second host stall (about 1,100 inputs) drains in under a second instead
/// of about 6 s at ten a tick. Its stale movement is not simulated in bulk:
/// S1 keeps the newest `server::MAX_QUEUED_INTENTS` inputs and steps at most
/// `server::MAX_INTENTS_PER_TICK` a tick on banked credit. The inputs S1
/// drops were acknowledged long after the client sent them, past its
/// 128-input prediction history, so they reconcile as skipped: the joiner is
/// corrected on the last tick or two of the catch-up, not input by input. The
/// per-kind budgets (edits, entity requests, device interacts) do not grow:
/// what is past them waits, in order. FU4a (FU3 verify M2) — a client whose
/// edit queue is full (`edit_queue::EditQueue::is_full`) is read
/// [`MAX_PACKETS_PER_TICK`] however much waits: no honest client is there, and
/// the catch-up multiplied what its flood cost the host.
pub const CATCH_UP_PACKETS_PER_TICK: usize = 64;

/// FU4a (FU3 verify L6) — control packets (`Respawn`, `Disconnect`) a client
/// may have read free of the read budget each tick. Past it each costs one
/// like any packet, so a burst of tiny control packets in one fill (about
/// 129,000 fit the inbound bound) is read over many ticks, not in one. An
/// honest client sends one `Respawn` every ~20 ticks while dead and one
/// `Disconnect`.
pub const FREE_CONTROL_PACKETS_PER_TICK: usize = 8;

/// FU4a (FU3 verify M2) — the refused edits sent back per client per tick:
/// by reach or plot, a dead joiner's (arriving or waiting), an earlier life's.
/// Each costs a world lookup and a broadcast to every joiner; past it the
/// refused edit is dropped with nothing sent back. An honest client has at
/// most the four edits processed a tick refused, plus those of the input it
/// sent as it died; only a joiner that dies with more than 64 edits waiting
/// (a long stall's backlog) passes it, and the edits past it stay on its
/// screen until their cells next change.
pub const MAX_SEND_BACKS_PER_CLIENT_PER_TICK: usize = 64;

/// FU1 — told to a client disconnected for crossing its inbound queue's hard
/// bound (`transport::MAX_INBOUND_BYTES`).
pub const INBOUND_OVERFLOW_REASON: &str =
    "Disconnected: your game sent more than the server could keep up with.";

/// A control packet (MP-A3): costs nothing against [`MAX_PACKETS_PER_TICK`]
/// (up to [`FREE_CONTROL_PACKETS_PER_TICK`] a tick, FU4a).
fn is_control_packet(packet: &[u8]) -> bool {
    matches!(
        protocol::deserialize_header(packet),
        Some((protocol::PacketType::Respawn | protocol::PacketType::Disconnect, _))
    )
}

/// FU3 — does `packet` (at the front of a client's queue) wait for the next
/// tick because its kind's budget is spent this tick: an entity request past
/// [`MAX_ENTITY_REQUESTS_PER_TICK`], a device interaction past
/// [`MAX_DEVICE_INTERACTS_PER_TICK`], or (C2b) an `ItemAction::Drop` while
/// the joiner's drop bucket is empty (`drop_ready` false:
/// `item_actions::DropBucket`). Waiting, not skipping: an honest catch-up
/// can hold more of either than one tick reads.
///
/// FU4a (FU3 verify L1) — and any request ([`is_request`]) while the client
/// has edits waiting (`edits_waiting`): it was sent after them, so it must
/// find them made (a lever placed then flipped, a post placed then a Lead
/// tied to it, a bed placed then slept in).
fn waits_for_kind_budget(
    packet: &[u8],
    entity_requests: usize,
    device_interacts: usize,
    drop_ready: bool,
    edits_waiting: bool,
) -> bool {
    use protocol::PacketType as P;
    if edits_waiting && is_request(packet) {
        return true;
    }
    match protocol::deserialize_header(packet) {
        Some((P::EntityAttack | P::EntityInteract, _)) => entity_requests >= MAX_ENTITY_REQUESTS_PER_TICK,
        Some((P::DeviceInteract, _)) => device_interacts >= MAX_DEVICE_INTERACTS_PER_TICK,
        Some((P::ItemAction, payload))
            if protocol::peek_item_action_variant(payload) == Some(protocol::item_action_variant::DROP) =>
        {
            !drop_ready
        }
        _ => false,
    }
}

/// C2b verify M4 — is `packet` an `ItemAction::Drop`?
fn is_drop_action(packet: &[u8]) -> bool {
    matches!(
        protocol::deserialize_header(packet),
        Some((protocol::PacketType::ItemAction, payload))
            if protocol::peek_item_action_variant(payload) == Some(protocol::item_action_variant::DROP)
    )
}

/// FU4a (FU3 verify L1) — a request about the world as the client's own
/// edits left it, so it waits behind those still waiting: `EntityAttack`,
/// `EntityInteract`, `DeviceInteract`, `ItemAction`. (`Respawn` and
/// `Disconnect` don't: a dead joiner's waiting edits are sent back.)
fn is_request(packet: &[u8]) -> bool {
    use protocol::PacketType as P;
    matches!(
        protocol::deserialize_header(packet),
        Some((P::EntityAttack | P::EntityInteract | P::DeviceInteract | P::ItemAction, _))
    )
}

/// FU4a — one client's edit counters for one tick
/// (`HostedServer::process_inbound_packets`).
#[derive(Default)]
struct EditTickBudget {
    /// Edits processed, refused or not ([`MAX_BLOCK_CHANGES_PER_TICK`]).
    edits: usize,
    /// Refused edits sent back ([`MAX_SEND_BACKS_PER_CLIENT_PER_TICK`]).
    send_backs: usize,
}

/// Most block edits the server processes per client per tick, across ALL of
/// its packets that tick — legitimate play is 1–2. Every processed edit
/// counts, refused or not. FU3 — edits past it wait in the slot's
/// `edit_queue::EditQueue` and go first next tick, in arrival order; only
/// past that queue's hard cap (`edit_queue::MAX_DEFERRED_EDITS`) is an edit
/// dropped for volume (FU4a: with nothing sent back).
const MAX_BLOCK_CHANGES_PER_TICK: usize = 4;

/// Handle to a running hosted server. Owns the `GameServer`, all client
/// transports, and (on native with remote players enabled) the QUIC
/// accept thread. Drop to shut down the accept thread cleanly.
pub struct HostedServer {
    /// Channel transports for local players (host + split-screen partner).
    /// Held separately from the boxed `transports` so callers can read
    /// state-update packets back without going through a `dyn` call. A seat
    /// added after start (`sync_local_slots`) has no loopback, so this may be
    /// shorter than `num_local_players`.
    pub local_transports: Vec<ChannelClientTransport>,
    /// Server simulation. Owned here so `tick()` can advance it inline
    /// from the main loop.
    pub server: crate::server::GameServer,
    /// All client transports, indexed the same as `server.players`. Local
    /// players come first (indices `0..num_local_players`); remote QUIC
    /// clients are appended as they connect.
    transports: Vec<Box<dyn ServerTransport>>,
    /// Whether each transport has completed the JoinRequest/JoinAccept
    /// handshake. Local slots skip this — they're trusted from the start.
    handshake_done: Vec<bool>,
    /// Whether each slot is free. Slot indexes stay stable while a player is
    /// connected (they are the wire `player_index`); a freed remote slot is
    /// handed to the next connection (`attach_remote_transport`), so the
    /// parallel vectors are bounded by peak concurrency, not by every
    /// connection ever made (audit 2026-09-27, "Unbounded growth").
    disconnected: Vec<bool>,
    /// Server tick at which each slot was attached — the pre-auth timeout
    /// clock (`PRE_AUTH_TIMEOUT_TICKS`). Local slots: 0 (no handshake).
    attached_tick: Vec<u64>,
    /// FU1 — per slot, indexed like `transports`: the packets its client sent
    /// that the server has not processed yet (past the per-tick budget,
    /// [`MAX_PACKETS_PER_TICK`]), in arrival order. Emptied whenever a slot is
    /// attached or released.
    inbound: Vec<transport::InboundQueue>,
    /// FU3 — per slot, indexed like `transports`: the block edits its client
    /// sent past the per-tick edit budget ([`MAX_BLOCK_CHANGES_PER_TICK`]),
    /// waiting, in arrival order, to go first next tick. Emptied whenever a
    /// slot is attached or released.
    edit_queues: Vec<crate::edit_queue::EditQueue>,
    /// Block changes the server produced (falling blocks, remote edits)
    /// that haven't been broadcast yet. Drained into the per-client
    /// `outboxes` on the next broadcast.
    pending_block_changes: Vec<protocol::BlockChange>,
    /// D1 — whether this server owns its world or borrows the host client's
    /// (`sim_lend`). Fixed at start.
    host_world: HostWorld,
    /// D1, lent only — the changes the server's own systems made to the
    /// host's world this tick (fluids, falling blocks, leaf decay, power, …),
    /// captured from `server.pending_block_changes` before the broadcast
    /// drains them. The host client remeshes them and replays their
    /// presentation (`take_lent_changes`).
    lent_sim_changes: Vec<protocol::BlockChange>,
    /// D1, lent only — cells a joiner's accepted edit or device flip changed
    /// in the host's world this tick (the host's own edits are already
    /// meshed by its client).
    lent_edit_cells: Vec<(i32, i32, i32)>,
    /// D1, lent only — set once the host's ECS has been cleared of
    /// `ProtocolId`s from any earlier server (ids are per `HostedServer`, so
    /// a stale one would collide with this server's fresh numbering).
    lent_ids_reset: bool,
    /// Per-slot outbound StateUpdate queue (gap-audit T1-5), indexed like
    /// `transports`: splits a tick under the packet cap, holds a remote
    /// client to its per-tick byte budget, coalesces a backlog, and turns an
    /// overflow into chunk-resync requests (`take_chunk_resync_requests`).
    /// Reset whenever a slot is attached or released.
    outboxes: Vec<crate::state_outbox::ClientOutbox>,
    /// Per-slot chunk push (Phase B2a, `chunk_push`), indexed like
    /// `outboxes` and reset with them: the chunks each remote client has been
    /// sent, its credit window and its pending resyncs. Local slots carry an
    /// idle one (they share the host's world; nothing is pushed or filtered).
    chunk_pushes: Vec<crate::chunk_push::ClientChunkPush>,
    /// `--chunk-sync`: which chunks a joiner is pushed (`touched` by default,
    /// Phase B2b; `all` pushes everything, B2a).
    chunk_sync: crate::chunk_push::ChunkSync,
    /// Phase B2b — the shared verdict cache (`chunk_verdict`): which columns
    /// still match generation. Read by every joiner's push plan; an edit makes
    /// a column touched for good. Lives as long as this server (never saved).
    verdicts: crate::chunk_verdict::Verdicts,
    /// Phase B2b — verdicts computed per tick (`chunk_verdict::VerdictBudget::
    /// for_server`: small on a lending host's frame, large on a server that
    /// does not lend; a test may change it).
    verdict_budget: crate::chunk_verdict::VerdictBudget,
    /// Phase B2b — verdicts computed in the last tick. Test-only.
    #[cfg(test)]
    verdicts_last_tick: usize,
    /// Phase B2b — column-mismatch warnings logged (`column_mismatch`).
    /// Test-only.
    #[cfg(test)]
    column_mismatch_warnings: usize,
    /// Test-only: the pre-B2a delivery (no pushes, every block change to
    /// every client), for the tests that pin the outbox on its own.
    #[cfg(test)]
    chunk_push_off: bool,
    /// Monotonic server tick counter sent with every StateUpdate.
    server_tick: u64,
    /// MP-D2a — the server-wide half of the entity broadcast: `ProtocolId`
    /// assignment and the changed-only baseline (`entity_broadcast`).
    entity_broadcast: crate::entity_broadcast::EntityBroadcast,
    /// MP-D2a — per slot, indexed like `outboxes`: the entities that client
    /// has been told about. A joiner hears only about entities near its body
    /// (interest radius); a fresh set is empty, so a late joiner gets every
    /// entity in range on its first broadcast — no separate backfill. Reset
    /// whenever a slot is attached or released.
    entity_interest: Vec<crate::entity_broadcast::ClientInterest>,
    /// How many of the front slots are local players. Remote slot count =
    /// `server.players.len() - num_local_players`.
    num_local_players: usize,
    max_remote_players: usize,
    /// The port the accept thread is listening on. For an online host this is
    /// the pre-bound socket's port, which is also what the peer was told to
    /// dial — so it is the value the Online panel must show.
    // Read today only by the pre-bound-socket integration test; the Online
    // panel becomes its live reader in Phase 4.
    #[cfg_attr(not(test), allow(dead_code))]
    pub port: u16,
    pub server_name: String,
    difficulty: String,

    /// Per-client challenge nonces. The server issues one (with its origin) to
    /// each new transport on connect, and `resolve_join_identity` consumes it
    /// (single-use) when a JoinRequest carrying an `auth_event` arrives. Indexed
    /// by transport slot stringified — a stable key for the lifetime of the
    /// connection. Phase 4 (v48) made this live. WASM builds skip the Signet path
    /// entirely (no remote QUIC clients exist there).
    #[cfg(not(target_arch = "wasm32"))]
    challenges: signet::ChallengeTable,

    /// Phase 4: whether a join must carry a verified `auth_event`. `true` for the
    /// QUIC LAN host (the verified-identity deliverable — absent auth is
    /// rejected). A WebSocket host starts `false` here only until
    /// [`HostedServer::set_access_policy`] runs; the dedicated server always
    /// calls it at boot (`server_main::load_access_policy`), and since
    /// 2026-10-06 that requires sign-in unless the operator opens the server
    /// (`--allow-guests` / `AXENSTAX_ALLOW_GUESTS=1`, or a `require_signin`
    /// file saying `false`). A *present* auth_event is always verified
    /// regardless of this flag.
    #[cfg(not(target_arch = "wasm32"))]
    require_signin: bool,

    /// Track 3: the server's Heartwood-backed identity, if provisioned. When
    /// present, every `JoinAccept` carries a proof (attestation + a signature
    /// over the client's nonce) so a client that pinned an operator npub can
    /// verify the server. `None` ⇒ anonymous server (no proof sent). Set by the
    /// dedicated server (`server_main`) after `start`; `None` for LAN hosts.
    #[cfg(not(target_arch = "wasm32"))]
    identity: Option<crate::server_identity::ServerIdentity>,

    /// v66 — the public addresses this server answers to (`--public-host`,
    /// `AXENSTAX_PUBLIC_HOST`, `AXENSTAX_DOMAIN`). A WebSocket join whose
    /// declared dialled host isn't one of them is refused, so a relayed
    /// signature (made for the relay's address) can't get in (Spec 08 §9.0.1
    /// T-JOIN-RELAY). Empty ⇒ any host is accepted (the residual). Set by the
    /// dedicated server via [`HostedServer::set_ws_public_hosts`].
    #[cfg(not(target_arch = "wasm32"))]
    ws_public_hosts: crate::signet::ws_host::PublicHosts,

    /// Track 4 — operator allowlist of verified pubkeys (x-only, 32 bytes). When
    /// non-empty, only these npubs may join (a whitelist implies sign-in). Empty
    /// ⇒ no allowlist. Set by the dedicated server from config; runtime-mutable
    /// by Track 5 admin commands.
    #[cfg(not(target_arch = "wasm32"))]
    whitelist: Vec<[u8; 32]>,
    /// Operator blocklist (`blocklist.txt`); a blocked npub is refused regardless
    /// of the allowlist (block wins, Spec B §5). Runtime-mutable by admin commands.
    #[cfg(not(target_arch = "wasm32"))]
    blocklist: Vec<[u8; 32]>,

    /// World chat (Phase 3) — the operator's chat-tightening policy for this
    /// world. Charter sets the ceiling; the operator may only tighten
    /// (`crate::comms::effective_level`, §2.5). Defaults to `Anyone`: an unset
    /// policy means the operator declined to tighten, not that they granted
    /// anything (§2.6). Set by the dedicated server via `set_chat_level`;
    /// native-only — there is no chat on web at all (spec §0/§6).
    #[cfg(not(target_arch = "wasm32"))]
    operator_comms: crate::comms::CommsLevel,

    /// Operator-private session telemetry (Spec B §6 / Spec C). Records verified
    /// player connect/disconnect **only when the operator opts into a tracking
    /// privacy level** — the default `None` records nothing. Never leaves the box.
    #[cfg(not(target_arch = "wasm32"))]
    session_log: crate::console_telemetry::SessionLog,
    /// Where the session log is persisted (the identity dir); `None` until the
    /// operator wires it via [`HostedServer::set_telemetry`].
    #[cfg(not(target_arch = "wasm32"))]
    telemetry_dir: Option<std::path::PathBuf>,
    /// Active privacy level governing whether/how long sessions are kept (Spec C).
    #[cfg(not(target_arch = "wasm32"))]
    privacy_level: crate::privacy::PrivacyLevel,
    /// Cached operator console settings (server name / capacity / announce /
    /// privacy), refreshed alongside telemetry. Feeds the operator snapshot (B-7a).
    #[cfg(not(target_arch = "wasm32"))]
    console_settings: crate::console_settings::ConsoleSettings,

    /// LAN-discovery broadcaster. Native-only because it binds a UDP socket.
    #[cfg(not(target_arch = "wasm32"))]
    broadcaster: Option<crate::discovery::ServerBroadcaster>,
    /// QUIC accept thread feeds new transports through this channel.
    #[cfg(not(target_arch = "wasm32"))]
    remote_rx: Option<mpsc::Receiver<Box<dyn ServerTransport>>>,
    /// Current remote-player count (shared with the accept thread so it
    /// stops accepting once we're full and starts again on disconnect).
    #[cfg(not(target_arch = "wasm32"))]
    current_remote: Arc<AtomicUsize>,
    /// Shutdown signal for the QUIC accept thread.
    #[cfg(not(target_arch = "wasm32"))]
    shutdown: Arc<AtomicBool>,
    /// Handle to the QUIC accept thread (if spawned).
    #[cfg(not(target_arch = "wasm32"))]
    quic_thread: Option<thread::JoinHandle<()>>,

    /// World chat §4 — the attached room, if any (`/room join`). The room is
    /// a member of the world's conversation, not a bypass of it — mirroring
    /// runs the same tier rule as in-world delivery (`crate::world_room`).
    /// Native only: there is no subprocess and no chat on web at all
    /// (spec §0/§6).
    #[cfg(not(target_arch = "wasm32"))]
    world_room: Option<Box<dyn crate::world_room::WorldRoom + Send>>,
    /// The room's own roster, refreshed every tick from `world_room.members()`
    /// (§4.2) so outbound mirroring has the speaker's classification of each
    /// member without needing a second live call into the trait object mid-tick.
    #[cfg(not(target_arch = "wasm32"))]
    room_members: Vec<crate::world_room::RosterMember>,
    /// The link we attached with, for `/room` status + `/room invite`. Kept
    /// here rather than asked of the trait — `WorldRoom` has no `invite()`;
    /// printing the link we already hold is enough, and `/room rotate` is
    /// deliberately out of scope (rotation is a keeper operation; we join as
    /// a member, not the keeper).
    #[cfg(not(target_arch = "wasm32"))]
    room_link: Option<String>,
    /// The relays we attached with (post-lint), for `/room` status.
    #[cfg(not(target_arch = "wasm32"))]
    room_relays: Vec<String>,
}

/// `/room` status snapshot (spec §4.5). A plain data struct so
/// `hosted_server.rs` stays free of chat-line formatting — the caller (the
/// game loop, which owns `self.chat`) turns this into a line.
#[cfg(not(target_arch = "wasm32"))]
pub struct RoomStatus {
    pub attached: bool,
    pub link: Option<String>,
    pub member_count: usize,
    pub relays: Vec<String>,
}

/// Review fix (Task 13) — the authoritative server-side reach gate for a
/// block change, distance-squared-based to match the call site. Before this
/// fix the gate was a flat `(5.0 + 0.5) * (5.0 + 0.5)` constant with no
/// knowledge of held items at all, so a Reach Claw's `+REACH_CLAW_BONUS`
/// (client-side `GameState::effective_reach`) never applied in any real
/// multiplayer/hosted session — every block change beyond ~5.5 blocks was
/// silently dropped, Reach Claw or not (single-player is unaffected since it
/// bypasses `HostedServer` entirely).
///
/// `held_kind`/`held_id` are the same wire-form fields already live on
/// `ServerPlayer` (populated from the client's `InputPacket` every tick, just
/// above the call site) — no new persisted state. Decodes them through
/// `protocol::ItemRef::from_wire` and reuses
/// `game_loop::reach_bonus_for_item_ref` so the server applies exactly the
/// bonus rule the client already computes, rather than duplicating the match
/// arm on `MaterialId::ReachClaw`.
///
/// BRIDGE: Anti-cheat regression fix. `held_kind`/`held_id` for a
/// server-simulated (remote) player are relayed straight from that client's
/// `InputPacket` with no possession check — the server keeps no
/// authoritative inventory for remote players (see
/// `server_player_item_ref`'s doc comment: their server-side `inventory` is
/// only the server's drifting shadow of theirs, C1). A modified remote client could claim
/// `MaterialId::ReachClaw` every tick and get the full `REACH_CLAW_BONUS` on
/// every block edit with nothing backing the claim. `server_simulated` gates
/// the bonus: `false` (local/position-trusted, same process as the host) is
/// the only case where the claimed held item is honoured, matching the
/// existing position-trust model for that path (`sp.player.pos` is likewise
/// taken verbatim from the local InputPacket). `true` (remote) forces the
/// flat base cap regardless of the claimed item. Replace this gate with the
/// full tool-aware bonus for remote players once remote inventories become
/// server-authoritative (tracked in the CLAUDE.md known-debt list alongside
/// the rest of the server-side inventory/crafting bridge).
pub(crate) fn block_change_within_reach(
    dist_sq: f32,
    held_kind: u8,
    held_id: u16,
    server_simulated: bool,
) -> bool {
    // Measured eye → block centre while the client ray-casts eye → block
    // FACE: up to sqrt(3)/2 further, plus ~0.5 of movement / queue lag
    // (review S4).
    const REACH_MARGIN: f32 = 0.87 + 0.5;
    let bonus = if server_simulated {
        0.0
    } else {
        crate::game_loop::reach_bonus_for_item_ref(protocol::ItemRef::from_wire(
            held_kind, held_id,
        ))
    };
    let limit = super::REACH_DISTANCE + REACH_MARGIN + bonus;
    dist_sq <= limit * limit
}

impl HostedServer {
    /// Start a hosted server. The shared body behind [`HostedServer::start`]
    /// (LAN / dedicated — the accept thread binds its own socket) and
    /// [`HostedServer::start_online`] (online play by contact — the accept
    /// thread adopts a socket the caller already bound and punched from).
    ///
    /// `num_local_players`: how many local players (1 for solo, 2 for split-screen host)
    /// `server_name`: world folder / LAN discovery name
    /// `seed`: world seed — threaded into `GameServer` so hosted terrain matches
    ///         the client (#8). Callers pass `load_world_meta(folder).seed`.
    /// `max_remote_players`: upper bound on inbound remote clients (0 = local only).
    ///                       On WASM this value is clamped to 0 because no network runtime exists.
    /// `remote_transport`: QUIC (native LAN "Host Game") or WebSocket (dedicated server).
    /// `prebound`: `Some(socket)` on the online path — quinn adopts it and the
    ///             listening port is that socket's port, because that is the
    ///             port already inside the candidates the peer was sent.
    /// `host_world`: [`HostWorld::Lent`] when a host client will lend its
    ///             world every tick (the server then loads no world of its
    ///             own), [`HostWorld::Owned`] otherwise.
    fn start_inner(
        num_local_players: usize,
        server_name: String,
        seed: u32,
        max_remote_players: usize,
        remote_transport: RemoteTransport,
        #[cfg(not(target_arch = "wasm32"))] prebound: Option<std::net::UdpSocket>,
        host_world: HostWorld,
    ) -> Result<Self, String> {
        // A lent world is a host CLIENT's: there must be one to lend it.
        if host_world == HostWorld::Lent && num_local_players == 0 {
            return Err("a lent-world server needs a host client (at least one local player)".to_string());
        }
        // Effective listening port. An online host bound its own socket before
        // gathering candidates, so the port is whatever the OS gave it — and it
        // MUST be that one, because that is the port already inside the
        // candidates the peer was sent. Otherwise: QUIC uses the fixed
        // SERVER_PORT; the dedicated WebSocket server uses its configured port.
        #[cfg(not(target_arch = "wasm32"))]
        let port = match (&prebound, remote_transport) {
            (Some(s), _) => s
                .local_addr()
                .map_err(|e| format!("pre-bound socket has no address: {e}"))?
                .port(),
            (None, RemoteTransport::Quic) => protocol::SERVER_PORT,
            (None, RemoteTransport::WebSocket { port }) => port,
        };
        #[cfg(target_arch = "wasm32")]
        let port = match remote_transport {
            RemoteTransport::Quic => protocol::SERVER_PORT,
            RemoteTransport::WebSocket { port } => port,
        };
        // LAN "Host Game" (T2-10): bind the UDP port NOW, on this thread, and hand
        // the socket to the accept thread. Binding inside that thread meant a
        // taken port was only a log line while the caller carried on as if hosting
        // had worked; this way it is an `Err` with a reason the player can read.
        // `create_server_endpoint` binds the same way, so behaviour is otherwise
        // unchanged. Skipped when the caller pre-bound (online) or no accept
        // thread will run (no remote slots).
        #[cfg(not(target_arch = "wasm32"))]
        let prebound = match (prebound, remote_transport) {
            (None, RemoteTransport::Quic) if max_remote_players > 0 => {
                Some(crate::lan_host::bind_lan_socket(port)?)
            }
            (other, _) => other,
        };
        let name_clone = server_name.clone();

        // Clamp max_remote_players to 0 on WASM — no network stack.
        #[cfg(target_arch = "wasm32")]
        let max_remote_players = {
            let _ = max_remote_players;
            0usize
        };

        // Local transports: one channel pair per local player.
        let mut local_transports = Vec::with_capacity(num_local_players);
        let mut transports: Vec<Box<dyn ServerTransport>> = Vec::with_capacity(num_local_players);
        for _ in 0..num_local_players {
            let (server_side, client_side) = transport::channel_pair();
            transports.push(Box::new(server_side));
            local_transports.push(client_side);
        }

        // Seed GameServer with local players only. Remote players are
        // appended on connect (see `tick`). #8 — thread the world's real seed
        // through so hosted terrain matches the client's biome_gen (was pinned
        // to 42, so every LAN-hosted world regenerated identically regardless
        // of the chosen/persisted seed). The caller passes the same value the
        // client uses (load_world_meta(folder).seed).
        let mut server = crate::server::GameServer::new(num_local_players, name_clone.clone(), seed);
        // T1-3 — the server ticks the block machines (pistons, furnaces,
        // crops, hoppers, …; `block_machines.rs`) and server projectiles only
        // when no local host client does. Invariant: 0 local players ⇔ no
        // host client — every client host path (LAN Host Game, online host)
        // starts with ≥ 1, and the dedicated server (`server_main`, the
        // WebSocket dedicated path) starts with 0. A lent world's machines
        // are the host client's, so this is never set on a lending server.
        server.simulates_block_machines = num_local_players == 0;
        // Review D2b MEDIUM-2 — breeding, Leads and pets following run only
        // in a host client's sim, so only on the world a host lends.
        server.animal_life_simulated = host_world == HostWorld::Lent;
        assign_column_loading(&mut server, num_local_players, host_world);
        match host_world {
            // A world on disk that fails to load is refused here — before the
            // accept thread starts or anything is saved — never replaced by a
            // fresh world (Spec 02 §8.4). The error names the file and why.
            HostWorld::Owned => server.initial_load()?,
            // D1 — the host client loads the one world (and refuses a damaged
            // one itself, leaving the world, which drops this server). Only
            // the meta rules and the saved players are read here.
            HostWorld::Lent => server.initial_load_lent(),
        }
        // The dedicated streamer's spawn anchor follows the computed world
        // spawn (`world_spawn` records its column) from boot, not only from
        // the first join.
        if server.column_streamer.is_some() {
            server.world_spawn();
        }

        // Pull difficulty from the world meta for JoinAccept and UI wiring.
        // play_mode is sourced from GameServer.play_mode (set by initial_load).
        let meta = crate::save::load_world_meta(&name_clone);
        let difficulty = meta.difficulty.clone();

        // The local player's identity comes from their own sign-in on this
        // machine, not from a JoinRequest — local slots skip the handshake
        // entirely (`handshake_done` is pre-seeded `true` just below). Without
        // this, the person hosting the world has no `verified_pubkey` and so,
        // by the no-chat-without-a-verified-key rule (world-chat spec §3.5),
        // cannot use the chat they are hosting. That is not the rule doing its
        // job — it is the rule being asked about a path it was not written for.
        //
        // This is not a client assertion: it is read from this machine's own
        // cached sign-in, and the Charter ceiling still binds to that identity,
        // so a child hosting a world is no less governed than one joining it.
        //
        // Slot 0 only. Split-screen players 2..n share a single sign-in and
        // have no identity of their own, so they get no chat — they are on the
        // same sofa and can speak out loud.
        // World chat (Phase 3) — the operator's tightening policy, resolved once
        // here and threaded through both to this local-join computation and to
        // the struct field below. Defaults to `Anyone`; the dedicated server
        // overrides it post-construction via `set_chat_level`. Native-only: no
        // Charter bridge and no chat on web at all (§0/§6).
        #[cfg(not(target_arch = "wasm32"))]
        let operator_comms = crate::comms::CommsLevel::Anyone;

        if let (Some(pk), Some(sp)) = (
            crate::server::local_identity_pubkey(crate::save::current_owner_pubkey().as_deref()),
            server.players.get_mut(0),
        ) {
            sp.verified_pubkey = Some(pk);
            // World chat (Phase 3) — resolved at join, never cached (§2.5,
            // §3.3): the Charter ceiling for this verified pubkey, tightened
            // by the operator's policy. Native-only — the Charter bridge reads
            // a local file via native-only crypto, and there is no sign-in (so
            // no verified key, so no chat regardless of `comms`) on the web
            // taster in the first place.
            #[cfg(not(target_arch = "wasm32"))]
            {
                sp.comms = resolve_join_comms(crate::charter::comms_level(&pk), operator_comms);
                // World chat (Phase 4) — this machine's local contacts book,
                // if a guardian has dropped a Kenspeckle export in the config
                // dir (`contacts::load_local_book`, §3.2). Best-effort: an
                // absent or unreadable book leaves `contacts` empty, which is
                // safe by construction (`ServerPlayer::tier_of` falls back to
                // `Stranger`), so the tier rule has real data to read when it
                // exists and simply talks to nobody when it doesn't.
                sp.contacts = crate::contacts::load_local_book()
                    .into_iter()
                    .map(|c| (c.pubkey, c.tier))
                    .collect();
            }
        }

        let handshake_done = vec![true; num_local_players];
        let disconnected = vec![false; num_local_players];

        #[cfg(not(target_arch = "wasm32"))]
        let (broadcaster, remote_rx, current_remote, shutdown, quic_thread) = {
            // LAN UDP discovery only makes sense for the QUIC peer-host path; the
            // dedicated WebSocket server is reached by URL, not broadcast.
            let broadcaster = match remote_transport {
                RemoteTransport::Quic => Some(crate::discovery::ServerBroadcaster::new(port)),
                RemoteTransport::WebSocket { .. } => None,
            };
            let current_remote = Arc::new(AtomicUsize::new(0));
            let shutdown = Arc::new(AtomicBool::new(false));

            if max_remote_players > 0 {
                let (tx, rx) = mpsc::channel::<Box<dyn ServerTransport>>();
                // `quic_thread` holds whichever accept thread we spawned (the
                // field name is historical — it may be the QUIC or the WS one).
                let handle = match remote_transport {
                    RemoteTransport::Quic => spawn_quic_accept_thread(
                        port,
                        max_remote_players,
                        current_remote.clone(),
                        shutdown.clone(),
                        tx,
                        prebound,
                    )?,
                    RemoteTransport::WebSocket { port: ws_port } => {
                        crate::ws_transport::spawn_ws_accept_thread(
                            ws_port,
                            max_remote_players,
                            current_remote.clone(),
                            shutdown.clone(),
                            tx,
                        )?
                    }
                };
                (broadcaster, Some(rx), current_remote, shutdown, Some(handle))
            } else {
                (broadcaster, None, current_remote, shutdown, None)
            }
        };

        log::info!(
            "Hosted server started: '{}' with {} local player(s), max remote = {}",
            server_name,
            num_local_players,
            max_remote_players
        );

        Ok(HostedServer {
            local_transports,
            server,
            transports,
            handshake_done,
            disconnected,
            attached_tick: vec![0; num_local_players],
            inbound: (0..num_local_players).map(|_| transport::InboundQueue::default()).collect(),
            edit_queues: (0..num_local_players).map(|_| Default::default()).collect(),
            pending_block_changes: Vec::new(),
            host_world,
            lent_sim_changes: Vec::new(),
            lent_edit_cells: Vec::new(),
            lent_ids_reset: false,
            // Local slots ride an in-process channel: unbudgeted outboxes.
            outboxes: (0..num_local_players)
                .map(|_| crate::state_outbox::ClientOutbox::new(false))
                .collect(),
            chunk_pushes: (0..num_local_players)
                .map(|_| crate::chunk_push::ClientChunkPush::default())
                .collect(),
            chunk_sync: crate::chunk_push::chunk_sync(),
            verdicts: crate::chunk_verdict::Verdicts::default(),
            // Tests count verdicts, never time them: the time cap would make
            // how many a tick decides depend on the machine's load.
            verdict_budget: {
                let budget = crate::chunk_verdict::VerdictBudget::for_server(host_world == HostWorld::Lent);
                if cfg!(test) {
                    crate::chunk_verdict::VerdictBudget { time: std::time::Duration::MAX, ..budget }
                } else {
                    budget
                }
            },
            #[cfg(test)]
            verdicts_last_tick: 0,
            #[cfg(test)]
            column_mismatch_warnings: 0,
            #[cfg(test)]
            chunk_push_off: false,
            server_tick: 0,
            entity_broadcast: crate::entity_broadcast::EntityBroadcast::new(),
            entity_interest: (0..num_local_players).map(|_| Default::default()).collect(),
            num_local_players,
            max_remote_players,
            port,
            server_name,
            difficulty,
            #[cfg(not(target_arch = "wasm32"))]
            challenges: signet::ChallengeTable::default(),
            // QUIC LAN host requires sign-in (verified identity deliverable).
            // A WebSocket host starts open only until `set_access_policy` runs;
            // the dedicated server applies its (sign-in-by-default) policy at
            // boot, before the main loop processes its first join.
            #[cfg(not(target_arch = "wasm32"))]
            require_signin: matches!(remote_transport, RemoteTransport::Quic),
            // Set by the dedicated server after `start` via `set_identity`.
            #[cfg(not(target_arch = "wasm32"))]
            identity: None,
            // Set by the dedicated server after `start` via `set_ws_public_hosts`.
            #[cfg(not(target_arch = "wasm32"))]
            ws_public_hosts: Default::default(),
            // Set by the dedicated server after `start` via `set_access_policy`.
            #[cfg(not(target_arch = "wasm32"))]
            whitelist: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            blocklist: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            operator_comms,
            #[cfg(not(target_arch = "wasm32"))]
            session_log: crate::console_telemetry::SessionLog::default(),
            #[cfg(not(target_arch = "wasm32"))]
            telemetry_dir: None,
            #[cfg(not(target_arch = "wasm32"))]
            privacy_level: crate::privacy::PrivacyLevel::None,
            #[cfg(not(target_arch = "wasm32"))]
            console_settings: crate::console_settings::ConsoleSettings::default(),
            #[cfg(not(target_arch = "wasm32"))]
            broadcaster,
            #[cfg(not(target_arch = "wasm32"))]
            remote_rx,
            #[cfg(not(target_arch = "wasm32"))]
            current_remote,
            #[cfg(not(target_arch = "wasm32"))]
            shutdown,
            #[cfg(not(target_arch = "wasm32"))]
            quic_thread,
            // No room attached at start. `/room join` attaches one.
            #[cfg(not(target_arch = "wasm32"))]
            world_room: None,
            #[cfg(not(target_arch = "wasm32"))]
            room_members: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            room_link: None,
            #[cfg(not(target_arch = "wasm32"))]
            room_relays: Vec::new(),
        })
    }

    /// Start a hosted server (LAN / dedicated). On the QUIC LAN path (with at
    /// least one remote slot) the UDP game port is bound **here, synchronously**
    /// and the socket handed to the accept thread, so a taken port is an `Err`
    /// with a readable reason (T2-10); the WebSocket dedicated server binds in
    /// its own accept thread as before.
    ///
    /// The server owns (loads and simulates) its world: the dedicated server,
    /// and the test rigs. A host client starts through [`Self::start_host`].
    pub fn start(
        num_local_players: usize,
        server_name: String,
        seed: u32,
        max_remote_players: usize,
        remote_transport: RemoteTransport,
    ) -> Result<Self, String> {
        Self::start_inner(
            num_local_players,
            server_name,
            seed,
            max_remote_players,
            remote_transport,
            #[cfg(not(target_arch = "wasm32"))]
            None,
            HostWorld::Owned,
        )
    }

    /// Start the server a host client (LAN Host Game) embeds. `host_world` is
    /// [`host_world_mode`] in the game: [`HostWorld::Lent`] unless the player
    /// passed `--no-lend`.
    pub fn start_host(
        num_local_players: usize,
        server_name: String,
        seed: u32,
        max_remote_players: usize,
        remote_transport: RemoteTransport,
        host_world: HostWorld,
    ) -> Result<Self, String> {
        Self::start_inner(
            num_local_players,
            server_name,
            seed,
            max_remote_players,
            remote_transport,
            #[cfg(not(target_arch = "wasm32"))]
            None,
            host_world,
        )
    }

    /// D1 — does this server simulate the host client's lent world (rather
    /// than a copy of its own)?
    pub fn lends_host_world(&self) -> bool {
        self.host_world == HostWorld::Lent
    }

    /// D1 review fix 1 — the column of every connected joiner's server body,
    /// on a world this host lends (empty otherwise; sorted, deduplicated). The
    /// host client's streamer keeps them loaded at the server's sim distance
    /// (`chunk_stream::client_stream_anchors`): the lent world is the only one
    /// the server simulates a joiner on, so a column only the host's players
    /// kept would unload under the joiner when the host walked away. Read
    /// outside the lend window — `players` is never lent.
    pub fn lent_joiner_columns(&self) -> Vec<(i32, i32)> {
        if !self.lends_host_world() {
            return Vec::new();
        }
        let mut cols: Vec<(i32, i32)> = self
            .server
            .players
            .iter()
            .filter(|sp| sp.server_simulated && sp.connected && sp.player.pos.is_finite())
            .map(|sp| crate::chunk_stream::column_of(sp.player.pos))
            .collect();
        cols.sort_unstable();
        cols.dedup();
        cols
    }

    /// Final review fix 1 — the spawn column of every DEAD joiner, on a world
    /// this host lends (empty otherwise; sorted, deduplicated). A respawn
    /// stands the body on the first solid block of its spawn point's column
    /// (`GameServer::standing_spot`), which reads only air while that column is
    /// unloaded — and after a long trip with the host that column is: the
    /// streamer kept it for the host's own players and the joiner's body, both
    /// far away by now. The host client's streamer anchors each at a ring of
    /// one column (`chunk_stream::client_stream_anchors`), and
    /// [`Self::handle_respawn`] waits until it is loaded.
    pub fn lent_respawn_columns(&self) -> Vec<(i32, i32)> {
        if !self.lends_host_world() {
            return Vec::new();
        }
        let mut cols: Vec<(i32, i32)> = self
            .server
            .players
            .iter()
            .filter(|sp| {
                sp.server_simulated
                    && sp.connected
                    && sp.combat.dead
                    && sp.spawn_pos.is_finite()
            })
            .map(|sp| crate::chunk_stream::column_of(sp.spawn_pos))
            .collect();
        cols.sort_unstable();
        cols.dedup();
        cols
    }

    /// D1 review fix 3 — the host client's local players as it simulated them
    /// this tick, `(position, yaw, pitch, health)` per seat in order. Only
    /// seat 0 sends input over the loopback (its edits and intent ride it),
    /// and hosting starts with ONE local slot, while a split-screen save
    /// loaded for hosting gives the host client more seats. On a lent world
    /// the server runs power (pressure plates), falling blocks and mob
    /// spawning (its anchors) on the host's world from its slots, and joiners
    /// see the local players through them — so every seat needs a slot that
    /// follows it:
    /// - missing slots are added (position-trusted, no input, no sign-in of
    ///   their own — split-screen seats share seat 0's), but only while no
    ///   joiner holds a slot: slot indices are the wire's `player_index` and
    ///   remote slots come after the local ones. The first hosted tick runs
    ///   before any accept, so that is when a multi-seat world grows them;
    /// - a slot whose seat has left (the pause menu's leave) is out of the
    ///   world (`connected` off, not broadcast) until a seat fills it again;
    /// - slot 0 (its input carries it), a joiner's slot and a non-finite
    ///   position are never written.
    pub fn sync_local_slots(&mut self, slots: &[(glam::Vec3, f32, f32, f32)]) {
        if slots.len() > self.num_local_players {
            if self.transports.len() == self.num_local_players {
                while self.num_local_players < slots.len() {
                    let pos = slots[self.num_local_players].0;
                    self.add_local_slot(if pos.is_finite() { pos } else { glam::Vec3::ZERO });
                }
            } else {
                static WARNED: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(false);
                if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    log::warn!(
                        "hosting: {} local seats but {} local slots and joiners already \
                         seated; the extra seats are not on the server",
                        slots.len(),
                        self.num_local_players
                    );
                }
            }
        }
        for i in 1..self.num_local_players {
            let seat = slots.get(i);
            self.disconnected[i] = seat.is_none();
            let sp = &mut self.server.players[i];
            sp.connected = seat.is_some();
            let Some(&(pos, yaw, pitch, health)) = seat else {
                continue;
            };
            if sp.server_simulated || !pos.is_finite() {
                continue;
            }
            sp.player.pos = pos;
            sp.yaw = yaw;
            sp.pitch = pitch;
            sp.combat.health = health.clamp(0.0, 20.0);
        }
    }

    /// One more local slot, set up as `start_inner` sets up its local slots
    /// (handshake done, unbudgeted outbox), behind a [`transport::NullServerTransport`]:
    /// the host client feeds it through [`Self::sync_local_slots`] and never
    /// reads it, so `local_transports` (seats with a loopback) may be shorter
    /// than `num_local_players`. Only while no remote slot exists.
    fn add_local_slot(&mut self, pos: glam::Vec3) {
        debug_assert_eq!(self.transports.len(), self.num_local_players, "a joiner holds a slot");
        debug_assert_eq!(self.server.players.len(), self.transports.len());
        self.server.players.push(crate::server::ServerPlayer::new(pos));
        self.transports.push(Box::new(transport::NullServerTransport));
        self.handshake_done.push(true);
        self.disconnected.push(false);
        self.attached_tick.push(self.server_tick);
        self.inbound.push(transport::InboundQueue::default());
        self.edit_queues.push(Default::default());
        self.outboxes.push(crate::state_outbox::ClientOutbox::new(false));
        self.entity_interest.push(Default::default());
        self.chunk_pushes.push(crate::chunk_push::ClientChunkPush::default());
        self.num_local_players += 1;
    }

    /// Test-only: switch how this server holds its world after it started
    /// (`sim_lend::OwnedSimParts::take_from` takes an owning server's loaded
    /// world out of it and makes it lend from then on).
    #[cfg(test)]
    pub(crate) fn set_host_world_for_test(&mut self, host_world: HostWorld) {
        self.host_world = host_world;
        self.server.animal_life_simulated = host_world == HostWorld::Lent;
        assign_column_loading(&mut self.server, self.num_local_players, host_world);
    }

    /// D1 — called by `sim_lend::LentSim::lend` once the host's world is in.
    /// The first time, clears every `ProtocolId` from the host's ECS: ids are
    /// numbered per `HostedServer`, so an id left by an earlier server (the
    /// host hosted, stopped, and hosted again on the same ECS) would collide
    /// with this one's numbering and reach joiners as a duplicate spawn. A
    /// safety net — every host path today loads the world after the server
    /// starts, so the ECS is fresh.
    pub(crate) fn on_lend(&mut self) {
        if self.lent_ids_reset {
            return;
        }
        self.lent_ids_reset = true;
        let tagged: Vec<hecs::Entity> = self
            .server
            .ecs
            .query::<&crate::entity::ProtocolId>()
            .iter()
            .map(|(e, _)| e)
            .collect();
        for e in tagged {
            let _ = self.server.ecs.remove_one::<crate::entity::ProtocolId>(e);
        }
    }

    /// D1, lent only — what this tick changed in the host's world that the
    /// host client did not do itself: `(the server's own sim changes, cells a
    /// joiner's edit or device flip changed)`. The host remeshes both and
    /// replays the sim changes' presentation (power challenges). Cleared by
    /// the call; always empty on an owning server.
    pub fn take_lent_changes(&mut self) -> (Vec<protocol::BlockChange>, Vec<(i32, i32, i32)>) {
        (
            std::mem::take(&mut self.lent_sim_changes),
            std::mem::take(&mut self.lent_edit_cells),
        )
    }

    /// Queue block changes the host client made itself (a `/we` region batch)
    /// for the next broadcast, without the host remeshing them again.
    pub fn queue_host_broadcast(&mut self, changes: impl IntoIterator<Item = protocol::BlockChange>) {
        self.pending_block_changes.extend(changes);
    }

    /// How many block changes are already queued for the next broadcast
    /// (bounds a `/we` region batch).
    pub fn queued_broadcasts(&self) -> usize {
        self.pending_block_changes.len() + self.server.pending_block_changes.len()
    }

    /// Start a hosted server on an **already-bound** UDP socket (online play by
    /// contact, spec §4.2).
    ///
    /// The caller bound the socket, gathered candidates on it (so its STUN
    /// mapping is the one QUIC will use), punched with a clone of it, and now
    /// hands the original over. `require_signin` stays `true` — this is the
    /// QUIC path — and the caller feeds the contacts-plus-bearer allowlist in
    /// through `set_access_policy`.
    ///
    /// `max_remote_players` must be at least 1. A hosted server with no remote
    /// slots never spawns an accept thread, so the socket would be dropped —
    /// closed — the moment this returned, while `self.port` went on reporting
    /// the port the peer was told to dial. That is a silent failure, so it is
    /// an error instead.
    ///
    /// **The retained `try_clone()` is non-blocking.** quinn puts the socket
    /// into non-blocking mode, and a `try_clone()` shares one file description
    /// with the original — so the clone the caller kept for punching is
    /// non-blocking too, whatever mode it was in when it was cloned. Phase 4
    /// must never `recv`/`recv_from` on it expecting to block; it will return
    /// `WouldBlock`.
    #[cfg(not(target_arch = "wasm32"))]
    #[cfg_attr(not(test), allow(dead_code))] // Driven by the online host loop (Phase 4).
    pub fn start_online(
        num_local_players: usize,
        server_name: String,
        seed: u32,
        max_remote_players: usize,
        socket: std::net::UdpSocket,
        host_world: HostWorld,
    ) -> Result<Self, String> {
        if max_remote_players == 0 {
            return Err(
                "an online host needs at least one remote slot — with none, no accept thread is \
                 started and the pre-bound socket would be closed rather than listened on"
                    .to_string(),
            );
        }
        Self::start_inner(
            num_local_players,
            server_name,
            seed,
            max_remote_players,
            RemoteTransport::Quic,
            Some(socket),
            host_world,
        )
    }

    /// Attach a Heartwood-backed identity so every `JoinAccept` carries a
    /// server-identity proof (Track 3). Set by the dedicated server after
    /// `start`. Native-only — WASM hosts have no remote joins.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_identity(&mut self, identity: Option<crate::server_identity::ServerIdentity>) {
        self.identity = identity;
    }

    /// Set the access policy (Track 4): whether sign-in is required and the
    /// operator allowlist of verified pubkeys. A non-empty allowlist forces
    /// sign-in regardless of `require_signin`. Native-only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_access_policy(
        &mut self,
        require_signin: bool,
        whitelist: Vec<[u8; 32]>,
        blocklist: Vec<[u8; 32]>,
    ) {
        self.require_signin = require_signin;
        self.whitelist = whitelist;
        self.blocklist = blocklist;
    }

    /// Set the public addresses WebSocket joins must be addressed to (v66).
    /// Empty leaves WS joins unprotected against relaying; `server_main` logs
    /// `signet::ws_host::relay_protection_warning` once at boot in that case.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_ws_public_hosts(&mut self, hosts: crate::signet::ws_host::PublicHosts) {
        self.ws_public_hosts = hosts;
    }

    /// `(players present, total slots)` — what the online host needs in order to
    /// answer an offer with `full` instead of silence. Same arithmetic as the
    /// LAN heartbeat, kept in one place so the two can't disagree.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn occupancy(&self) -> (usize, usize) {
        let active = self.disconnected.iter().filter(|d| !**d).count();
        (active, self.num_local_players + self.max_remote_players)
    }

    /// The remote players in the world right now, as
    /// `(display handle, verified persona)`, for the Online panel's connected
    /// list. Local players are excluded (they are the person reading it), and
    /// the persona is the **verified** key, never the client-asserted name.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connected_players(&self) -> Vec<(String, Option<[u8; 32]>)> {
        self.server
            .players
            .iter()
            .filter(|p| p.connected && p.server_simulated)
            .map(|p| (p.display_name.clone(), p.verified_pubkey))
            .collect()
    }

    /// Set the operator's chat-tightening policy (world chat Phase 3, §2.5).
    /// Native-only. Takes effect on the NEXT join for every player — it does
    /// not retroactively recompute `comms` for players already connected,
    /// matching `effective_level`'s "recomputed server-side on every join,
    /// never cached across sessions" rule (a join is exactly the recompute
    /// point; there isn't a second one mid-session).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_chat_level(&mut self, level: crate::comms::CommsLevel) {
        self.operator_comms = level;
    }

    /// `/room join <link>` (spec §4.5). Attaches a [`crate::kithmoot_keeper::KithMootKeeper`]
    /// as the world's room. Relays come from `AXENSTAX_ROOM_RELAYS`
    /// (comma-separated) and nowhere else — there is deliberately no default,
    /// because kithmoot's own default list begins with `relay.trotters.cc`
    /// (§4.4). An unset/empty env var means an empty relay list, which
    /// `lint_relays` refuses on its own with a message naming the problem —
    /// that refusal IS the "no relays configured" error, not a special case
    /// here.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn room_join(
        &mut self,
        link: &str,
        name: &str,
    ) -> Result<(), crate::world_room::RoomError> {
        self.room_join_with(Box::new(crate::kithmoot_keeper::KithMootKeeper::new()), link, name)
    }

    /// The testable half of `room_join`: takes the [`crate::world_room::WorldRoom`]
    /// implementation as a parameter so a test can pass a `FakeRoom` and drive
    /// the refusal paths (empty relay env, a link without `owned-by-members`)
    /// with no Node and no network. Both gates run here, explicitly and in
    /// order, BEFORE `room.start()` — not left to whatever the implementation
    /// behind the trait happens to check, because the double-check is what
    /// makes the refusal messages testable against any `WorldRoom` impl,
    /// including one (like the test double) that does neither on its own.
    #[cfg(not(target_arch = "wasm32"))]
    fn room_join_with(
        &mut self,
        mut room: Box<dyn crate::world_room::WorldRoom + Send>,
        link: &str,
        name: &str,
    ) -> Result<(), crate::world_room::RoomError> {
        use crate::world_room::{check_room_link, lint_relays, RoomConfig, RoomError};

        let raw_relays: Vec<String> = std::env::var("AXENSTAX_ROOM_RELAYS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let linted = lint_relays(&raw_relays).map_err(RoomError::Relays)?;
        check_room_link(link).map_err(RoomError::Link)?;

        let cfg = RoomConfig {
            link: link.to_string(),
            name: name.to_string(),
            relays: linted.kept.clone(),
        };
        room.start(&cfg)?;

        self.room_link = Some(link.to_string());
        self.room_relays = linted.kept;
        self.room_members = room.members();
        self.world_room = Some(room);
        Ok(())
    }

    /// `/room leave` (spec §4.5). A no-op, not an error, if nothing is
    /// attached — leaving a room you're not in is a successful no-op, not a
    /// failure to report.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn room_leave(&mut self) -> Result<(), crate::world_room::RoomError> {
        if let Some(mut room) = self.world_room.take() {
            room.stop()?;
        }
        self.room_link = None;
        self.room_relays.clear();
        self.room_members.clear();
        Ok(())
    }

    /// `/room` status (spec §4.5): attached or not, the link, member count,
    /// relays.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn room_status(&self) -> RoomStatus {
        RoomStatus {
            attached: self.world_room.is_some(),
            link: self.room_link.clone(),
            member_count: self.room_members.len(),
            relays: self.room_relays.clone(),
        }
    }

    /// This player's classification of each current room member, from the
    /// SPEAKER's own address book (§4.2: "how the speaker classifies each
    /// room member"). A member whose `participant` hex does not decode to a
    /// pubkey is a `Stranger` — the same strictness `server::local_identity_pubkey`
    /// applies to a local sign-in, and an undecodable identity must never read
    /// as anything more trusted than a stranger.
    #[cfg(not(target_arch = "wasm32"))]
    fn room_member_tiers_for(
        speaker: &crate::server::ServerPlayer,
        members: &[crate::world_room::RosterMember],
    ) -> Vec<crate::comms::Tier> {
        members
            .iter()
            .map(|m| {
                crate::server::local_identity_pubkey(Some(&m.participant))
                    .map(|pk| speaker.tier_of(&pk))
                    .unwrap_or(crate::comms::Tier::Stranger)
            })
            .collect()
    }

    /// World chat §4.2 — mirror an already-delivered in-world line out to the
    /// attached room, if the speaker would be permitted to speak to at least
    /// one of its members. No-op if no room is attached. Errors are logged,
    /// not propagated — a room hiccup must never break in-world chat.
    #[cfg(not(target_arch = "wasm32"))]
    fn mirror_outbound_to_room(&mut self, sender: usize, text: &str) {
        if self.world_room.is_none() {
            return;
        }
        let Some(sp) = self.server.players.get(sender) else {
            return;
        };
        let tiers = Self::room_member_tiers_for(sp, &self.room_members);
        let speaker_comms = sp.comms;
        if !crate::world_room::should_mirror_out(speaker_comms, &tiers) {
            return;
        }
        if let Some(room) = self.world_room.as_mut()
            && let Err(e) = room.post(&crate::world_room::OutboundLine { text: text.to_string() })
        {
            log::warn!("room: failed to mirror outbound line: {e:?}");
        }
    }

    /// A truncated pubkey to show when the room gave no display name for a
    /// speaker. Never trust the room's own `name` for identity — this is a
    /// display fallback only, exactly like `ChatDeliverPacket.from_name`
    /// elsewhere.
    #[cfg(not(target_arch = "wasm32"))]
    fn truncate_room_participant(hex: &str) -> String {
        // chars, not a byte slice: a non-ASCII `from` must not panic the host.
        if hex.chars().count() > 8 {
            format!("{}…", hex.chars().take(8).collect::<String>())
        } else {
            hex.to_string()
        }
    }

    /// World chat §4.2 — poll the attached room for lines that arrived since
    /// the last tick, refresh the membership roster, and deliver each line to
    /// every connected player for whom the hearing rule passes. No-op if no
    /// room is attached.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_room(&mut self) {
        let Some(room) = self.world_room.as_mut() else {
            return;
        };
        let lines = room.poll();
        self.room_members = room.members();
        for line in lines {
            // Room lines come from outside the world: same bounds as ChatSay.
            let Ok(text) = crate::comms::sanitize_chat_text(&line.text) else {
                continue;
            };
            let from_pubkey = crate::server::local_identity_pubkey(Some(&line.from));
            let from_name = line
                .name
                .as_deref()
                .map(sanitise_handle)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| Self::truncate_room_participant(&line.from));
            let pkt = protocol::serialize_packet(
                protocol::PacketType::ChatDeliver,
                &protocol::ChatDeliverPacket {
                    from_pubkey,
                    from_name,
                    text,
                    kind: protocol::ChatWireKind::Room,
                },
            );
            for j in 0..self.server.players.len() {
                let Some(player) = self.server.players.get(j) else {
                    continue;
                };
                // §3.5 — no verified key, no chat, whatever `comms` says: an
                // unsigned seat can't escape a guardian's ceiling by not
                // signing in. Mirrors `handle_chat_say`.
                if player.verified_pubkey.is_none() {
                    continue;
                }
                let tier = from_pubkey
                    .map(|pk| player.tier_of(&pk))
                    .unwrap_or(crate::comms::Tier::Stranger);
                if crate::world_room::should_mirror_in(player.comms, tier) {
                    self.send_raw_to_slot(j, &pkt);
                }
            }
        }
    }

    /// Wire operator-private telemetry (Spec B §6 / Spec C). On the first call the
    /// on-disk session log is loaded; later calls only refresh the privacy `level`
    /// (so a `privacy` admin command takes effect without dropping in-memory
    /// sessions). The default `None` level records nothing. Native-only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_telemetry(&mut self, dir: std::path::PathBuf, level: crate::privacy::PrivacyLevel) {
        if self.telemetry_dir.is_none() {
            self.session_log = crate::console_telemetry::SessionLog::load(&dir);
        }
        // Settings are config — always refresh (picks up `privacy`/`name`/announce
        // admin-command edits) so the operator snapshot reflects the live posture.
        self.console_settings = crate::console_settings::ConsoleSettings::load(&dir);
        self.telemetry_dir = Some(dir);
        self.privacy_level = level;
    }

    /// Apply retention + persist the session log (Spec C). Called from the
    /// dedicated server's periodic reload. No-op until telemetry is wired.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn flush_telemetry(&mut self) {
        if let Some(dir) = self.telemetry_dir.clone() {
            let now = current_unix_ts() as u64;
            self.session_log.apply_retention(&self.privacy_level, now);
            let _ = self.session_log.save(&dir);
        }
    }

    /// Build + send the live Operator Console snapshot (Spec B task 7 / B-7a) to
    /// any connected player who IS the verified operator. Called periodically from
    /// `tick`. Early-returns when there's no verified operator identity or no
    /// operator is in-game, so the common path costs nothing. Defensive: assembles
    /// from pure helpers + a fallible JSON encode — never panics the server tick.
    #[cfg(not(target_arch = "wasm32"))]
    fn broadcast_operator_snapshot(&mut self) {
        // Operator pubkey from the attestation; no identity ⇒ nobody to send to.
        let Some(op) = self.operator_pubkey() else {
            return;
        };
        // Only build when an operator is actually connected.
        let any_operator = (0..self.transports.len()).any(|i| self.has_operator_privileges(i, op));
        if !any_operator {
            return;
        }

        let verified: Vec<Option<[u8; 32]>> =
            self.server.players.iter().map(|p| p.verified_pubkey).collect();
        let handles: Vec<String> =
            self.server.players.iter().map(|p| p.display_name.clone()).collect();
        let roster = crate::console_snapshot::roster_rows(
            &verified,
            &handles,
            &self.handshake_done,
            &self.disconnected,
        );

        let now = current_unix_ts() as u64;
        let agg = self.session_log.aggregates(now, now - (now % 86_400));
        let policy = (
            self.require_signin,
            self.whitelist.len() as u32,
            self.blocklist.len() as u32,
        );
        let ident = self.identity.as_ref().map_or_else(
            || (String::new(), String::new(), 0u64),
            |id| {
                use nostr::ToBech32;
                (
                    id.operator_npub().unwrap_or_default(),
                    id.runtime_pubkey().to_bech32().unwrap_or_default(),
                    id.attestation().map(|a| a.valid_until.as_secs()).unwrap_or(0),
                )
            },
        );
        let snap =
            crate::console_snapshot::build_snapshot(&self.console_settings, &roster, &agg, policy, ident);
        let json = serde_json::to_string(&snap).unwrap_or_default();
        let pkt = protocol::serialize_packet(
            protocol::PacketType::OperatorSnapshot,
            &protocol::OperatorSnapshotPacket { snapshot_json: json },
        );
        for i in 0..self.transports.len() {
            if self.has_operator_privileges(i, op) {
                self.transports[i].send_to_client(&pkt);
            }
        }
    }

    /// Whether slot `i` holds operator privileges: a live, handshaken seat
    /// whose verified npub is the operator's AND whose join was channel-bound.
    /// An unbound (WebSocket) join can be a relayed replay of the operator's
    /// signature (Spec 08 §9.0.1, T-JOIN-RELAY), so it plays as an ordinary
    /// verified player and never gets the OperatorSnapshot.
    #[cfg(not(target_arch = "wasm32"))]
    fn has_operator_privileges(&self, i: usize, op: [u8; 32]) -> bool {
        self.handshake_done[i]
            && !self.disconnected[i]
            && self.transports[i].channel_binding().is_some()
            && crate::console_snapshot::is_operator(
                self.server.players.get(i).and_then(|p| p.verified_pubkey),
                Some(op),
            )
    }

    /// The operator's pubkey from this server's attestation, if paired.
    #[cfg(not(target_arch = "wasm32"))]
    fn operator_pubkey(&self) -> Option<[u8; 32]> {
        self.identity
            .as_ref()
            .and_then(|id| id.attestation())
            .map(|att| att.operator.to_bytes())
    }

    /// Consume the kick queue (`<dir>/kick`, written by an admin command) and
    /// disconnect any matching connected players (Spec B §5). Called from the
    /// dedicated server's periodic reload. The slot match is the pure
    /// `kick::slot_for_pubkey`; the live disconnect (a player on another machine
    /// actually dropping) is the owner boundary. Native-only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn process_pending_kicks(&mut self, dir: &std::path::Path) {
        let targets = take_kick_queue(dir);
        self.kick_pubkeys(&targets);
    }

    /// Disconnect every live remote player whose verified persona is in
    /// `targets` (the same teardown as an operator kick). Also used when a
    /// Signet block reaches a hosted world, so a blocked player already in
    /// the world is removed, not just refused next time (D8). A pubkey with
    /// no live slot is a no-op. Native-only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn kick_pubkeys(&mut self, targets: &[[u8; 32]]) {
        for t in targets {
            // Resolved by the stable identity (the verified npub) at the
            // moment of the kick, against LIVE remote slots only — a stale
            // slot from an earlier session never soaks up the kick while the
            // player carries on in their new one.
            let live: Vec<(Option<[u8; 32]>, bool)> = self
                .server
                .players
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let live = i >= self.num_local_players
                        && !self.disconnected.get(i).copied().unwrap_or(true);
                    (p.verified_pubkey, live)
                })
                .collect();
            for slot in crate::kick::live_slots_for_pubkey(&live, t) {
                if let Some(left) = self.release_slot(slot, Some(KICK_REASON)) {
                    self.send_to_joined_except(slot, &left);
                }
                log::info!("kick: released slot {slot}");
            }
        }
    }

    /// Free slot `i`. The ONE teardown shared by a client's `Disconnect`, a
    /// dropped connection, a rejected join, the pre-auth timeout, a kick and a
    /// same-npub replacement (audit 2026-09-27: each used to leak differently —
    /// a kick or a reject never released the seat, a dropped QUIC link was
    /// never noticed at all).
    ///
    /// For a remote slot: `tell_client` (if any) goes out as a `JoinReject`
    /// first, the seat returns to the accept thread's counter, the challenge is
    /// dropped, and the real transport is replaced by a `ClosedTransport` —
    /// dropping it is what closes the socket, after flushing that last packet.
    /// The `ServerPlayer` stays (marked `connected = false`) until the slot is
    /// reused. The server keeps no authoritative inventory for a remote player
    /// (see `block_change_within_reach`'s BRIDGE), so there is nothing to drop.
    ///
    /// Returns the `Left` event to fan out if the slot had completed its join.
    fn release_slot(&mut self, i: usize, tell_client: Option<&str>) -> Option<Vec<u8>> {
        if self.disconnected.get(i).copied().unwrap_or(true) {
            return None;
        }
        let was_joined = self.handshake_done[i];
        self.disconnected[i] = true;
        // FU1 — whatever it sent and the server had not yet read goes with the
        // connection (FU3: and the edits still waiting past the budget).
        self.inbound[i].clear();
        self.edit_queues[i].clear();
        if let Some(sp) = self.server.players.get_mut(i) {
            sp.connected = false;
            sp.pending_intent = None;
            sp.intent_queue.clear();
            // C1 — one line on what the log-only possession check saw.
            if sp.server_simulated
                && let Some(line) = sp.possession.summary(&sp.display_name)
            {
                log::info!("{line}");
            }
            // Review D2b MEDIUM-1 — what this connection stamped on the
            // world's mobs (its hits, its feeds, a Bear's grudge) is forgotten
            // at the top of the next server tick, before the slot's next
            // occupant can act.
            if sp.server_simulated {
                self.server.released_joiners.push((i, sp.attach_gen));
            }
        }
        if i >= self.num_local_players {
            if let Some(reason) = tell_client {
                let reject = protocol::JoinRejectPacket {
                    reason: reason.to_string(),
                };
                let pkt = protocol::serialize_packet(protocol::PacketType::JoinReject, &reject);
                self.transports[i].send_to_client(&pkt);
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                crate::admission::release_seat(&self.current_remote);
                let _ = self.challenges.consume(&challenge_key_for_slot(i));
                if was_joined {
                    // Close the telemetry session (no-op for a guest / when
                    // not tracking).
                    crate::console_telemetry::capture_disconnect(
                        &mut self.session_log,
                        self.server.players[i].verified_pubkey,
                        current_unix_ts() as u64,
                    );
                }
            }
            self.transports[i] = Box::new(transport::ClosedTransport);
            // Nothing queued for the old connection reaches the next one, and
            // nothing it was sent counts as sent to the next (B2a).
            self.outboxes[i] = crate::state_outbox::ClientOutbox::new(true);
            self.entity_interest[i] = Default::default();
            self.chunk_pushes[i] = crate::chunk_push::ClientChunkPush::default();
        }
        log::info!("Slot {i} released");
        was_joined.then(|| {
            protocol::serialize_packet(
                protocol::PacketType::PlayerEvent,
                &protocol::PlayerEventPacket {
                    player_index: i as u32,
                    event: protocol::PlayerEventType::Left,
                },
            )
        })
    }

    /// Send `pkt` to every joined, live slot except `source`.
    fn send_to_joined_except(&self, source: usize, pkt: &[u8]) {
        for j in 0..self.transports.len() {
            if j != source && self.handshake_done[j] && !self.disconnected[j] {
                self.transports[j].send_to_client(pkt);
            }
        }
    }

    /// Send `pkt` to slot `i` alone — if it is a joined, live slot.
    fn send_to_joined_slot(&self, i: usize, pkt: &[u8]) {
        if self.handshake_done.get(i).copied().unwrap_or(false)
            && !self.disconnected.get(i).copied().unwrap_or(true)
        {
            self.transports[i].send_to_client(pkt);
        }
    }

    /// Is slot `i` a server-simulated joiner the server holds dead (MP-A3)?
    fn joiner_is_dead(&self, i: usize) -> bool {
        self.server.players.get(i).is_some_and(|sp| sp.server_simulated && sp.combat.dead)
    }

    /// MP-A3 — turn this tick's server-originated joiner deaths (the
    /// `just_died` one-shot: a fall, drowning, a mob, lava or fire, a blast,
    /// or — MP-D2a — a reported `health_delta` that took a body the server
    /// held lower than its client knew to zero) into
    /// `PlayerEventType::Died`, sent to the dead player alone (nobody else has
    /// a use for it, and it names where someone is not safe). That is how a
    /// joiner whose server copy died — when its own sim didn't see it — reaches
    /// its death screen, and with it the Respawn button the server is waiting
    /// on. A death the joiner's input REPORTED (`health <= 0`) sets no
    /// one-shot (`GameServer::report_player_death`): its client already knows,
    /// and an echo arriving after a quick Respawn would kill it a second time.
    fn announce_joiner_deaths(&mut self) {
        for i in 0..self.server.players.len() {
            let sp = &mut self.server.players[i];
            if !sp.server_simulated || !sp.combat.just_died {
                continue;
            }
            sp.combat.just_died = false;
            // MP-D2b — naming the cause the body recorded, for the joiner's
            // death screen.
            let cause = damage_cause_to_wire(sp.combat.last_damage);
            let pkt = protocol::serialize_packet(
                protocol::PacketType::PlayerEvent,
                &protocol::PlayerEventPacket {
                    player_index: i as u32,
                    event: protocol::PlayerEventType::DiedOf { cause },
                },
            );
            self.send_to_joined_slot(i, &pkt);
        }
    }

    /// MP-D2b — this tick's server-landed hits wear each joiner's armour
    /// (`PlayerEventType::ArmourWorn`, to that joiner alone), and the kills
    /// credited to joiners go out as `KillEvent`s, each to its killer alone.
    fn announce_joiner_hits_and_kills(&mut self) {
        for i in 0..self.server.players.len() {
            let sp = &mut self.server.players[i];
            let hits = std::mem::take(&mut sp.armour_wear_hits);
            if hits == 0 || !sp.server_simulated {
                continue;
            }
            let pkt = protocol::serialize_packet(
                protocol::PacketType::PlayerEvent,
                &protocol::PlayerEventPacket {
                    player_index: i as u32,
                    event: protocol::PlayerEventType::ArmourWorn { hits },
                },
            );
            self.send_to_joined_slot(i, &pkt);
        }
        for (slot, kill) in std::mem::take(&mut self.server.pending_kill_events) {
            let pkt = protocol::serialize_packet(protocol::PacketType::KillEvent, &kill);
            self.send_to_joined_slot(slot, &pkt);
        }
        // Review D2b B2 — babies born to animals a joiner fed.
        for (slot, offspring) in std::mem::take(&mut self.server.pending_bred_events) {
            let pkt = protocol::serialize_packet(
                protocol::PacketType::PlayerEvent,
                &protocol::PlayerEventPacket {
                    player_index: slot as u32,
                    event: protocol::PlayerEventType::Bred { offspring },
                },
            );
            self.send_to_joined_slot(slot, &pkt);
        }
    }

    /// MP-D2b — the server's own entity `id` (its `ProtocolId`) as joiner
    /// `i` may act on it: a living mob whose centre is within
    /// `combat::ATTACK_REACH` + [`ATTACK_REACH_TOLERANCE`] of the eye of the
    /// body the server holds for `i`, which must itself be a present, living
    /// joiner. The entity is looked up now (it may have died since the
    /// client saw it). Review D2b LOW-2 — and in front of the body (the
    /// server's last look direction for it, leniently: anywhere in the half
    /// space ahead, for the look that changed in flight); single-player's
    /// pick wants it within 60°. Review D2b LOW-3 — never a perched parrot,
    /// which single-player's pick skips for every gesture.
    fn joiner_target(&self, i: usize, id: u32) -> Option<(hecs::Entity, crate::mob::MobType)> {
        let sp = self.server.players.get(i)?;
        if !sp.server_simulated || !sp.is_present_and_alive() {
            return None;
        }
        let ecs = &self.server.ecs;
        let e = ecs
            .query::<&crate::entity::ProtocolId>()
            .iter()
            .find(|(_, p)| p.0 == id)
            .map(|(e, _)| e)?;
        let kind = ecs.get::<&crate::entity::MobKind>(e).ok()?.0;
        if ecs.get::<&crate::combat::Health>(e).ok()?.is_dead() {
            return None;
        }
        if ecs
            .get::<&crate::companion::CompanionData>(e)
            .is_ok_and(|d| d.state == crate::companion::CompanionState::Perch)
        {
            return None;
        }
        let pos = ecs.get::<&crate::entity::Position>(e).ok()?.0;
        let height = ecs.get::<&crate::entity::Hitbox>(e).map_or(0.0, |h| h.height);
        let centre = pos + glam::Vec3::new(0.0, height * 0.5, 0.0);
        let reach = crate::combat::ATTACK_REACH + ATTACK_REACH_TOLERANCE;
        let to_target = centre - sp.player.eye_pos();
        let ahead = to_target.normalize_or_zero().dot(crate::camera::forward_from(sp.yaw, sp.pitch))
            >= JOINER_MIN_FACING_DOT;
        (to_target.length() <= reach && ahead).then_some((e, kind))
    }

    /// MP-D2b — joiner `i` swings at `req.entity`. Validated (a living mob in
    /// reach of the server's body, off the server's cooldown), then landed
    /// by `combat::strike` + `combat::after_swing`, the single-player rule:
    /// damage from the claimed held item, a critical hit if the server's
    /// body is airborne, knockback, the sweep, `LastAttacker` naming this
    /// joiner (its kill is credited to it). Answered with an
    /// `InteractOutcome`: accepted = the swing was valid and spent, which is
    /// when the joiner's weapon wears.
    fn handle_entity_attack(&mut self, i: usize, req: &protocol::EntityAttackPacket) {
        let accepted = self.land_joiner_swing(i, req);
        self.send_outcome(i, req.seq, req.entity, None, accepted, 0, 0);
    }

    fn land_joiner_swing(&mut self, i: usize, req: &protocol::EntityAttackPacket) -> bool {
        let Some((target, _kind)) = self.joiner_target(i, req.entity) else {
            return false;
        };
        let tick = self.server.tick_counter;
        // A guest's key is empty: no pet is ever its own.
        let key = self.server.players[i].pet_owner_key().unwrap_or_default();
        // 1C no-friendly-fire: the joiner's own pet takes a swing only as a
        // deliberate (sneaking) hit, as single-player's target pick has it.
        if !req.sneak && crate::tameable::is_own_pet(&self.server.ecs, target, &key, None) {
            return false;
        }
        let sp = &mut self.server.players[i];
        // Review D2b LOW-2 — on the server's schedule: up to the jitter
        // early, and each swing moves it a full cooldown on (never banking
        // swings an idle client didn't make).
        if tick + u64::from(ATTACK_COOLDOWN_JITTER_TICKS) < sp.next_swing_tick {
            return false;
        }
        sp.next_swing_tick = sp.next_swing_tick.max(tick) + u64::from(crate::combat::ATTACK_COOLDOWN);
        // BRIDGE: possession check — replace when phase C makes joiner
        // inventories server-authoritative. The held item is the client's
        // word (as a block placement's is, `validate_block_edit`): it sets
        // the damage, so a modified client can claim a better sword than it
        // holds. Nothing else of the swing is its word.
        let held = held_item_from_wire(req.held_kind, req.held_id, &req.held_full, &self.server.registry);
        let base_damage = held.as_ref().map_or(1.0, crate::item::Item::attack_damage);
        let eye = sp.player.eye_pos();
        let look_dir = crate::camera::forward_from(sp.yaw, sp.pitch);
        let crit = !sp.player.on_ground;
        crate::combat::strike(
            &mut self.server.ecs,
            target,
            &crate::combat::Swing {
                attacker: crate::combat::Attacker::Remote { slot: i, generation: sp.attach_gen },
                owner_key: &key,
                eye,
                look_dir,
                crit,
                sprinting: req.sprint,
                sneaking: req.sneak,
                base_damage,
            },
        );
        let kick =
            crate::combat::after_swing(&mut self.server.ecs, target, &key, None, req.sneak, eye, tick);
        if let Some(kick) = kick {
            // Spec 28d.nostrich v2 — the kick-back lands on the joiner's body
            // (no armour, as in single-player).
            let sp = &mut self.server.players[i];
            sp.combat.take_damage_from(
                kick,
                crate::survival::DamageCause::Mob(crate::mob::MobType::Nostrich),
            );
        }
        true
    }

    /// MP-D2b — joiner `i` right-clicks `req.entity` for `req.kind`.
    /// Validated like a swing (a living mob in reach of the server's body)
    /// plus the server's interaction cooldown, then run through
    /// `mob_interact::run` — the functions single-player's right-click runs —
    /// as this joiner: its pet-owner key is its verified npub (a guest can't
    /// tame) and a Lead it fastens anchors to its body. Answered with an
    /// `InteractOutcome` (accepted, what to take from the hand, a note);
    /// products ride `InventoryGrant`.
    fn handle_entity_interact(&mut self, i: usize, req: &protocol::EntityInteractPacket) {
        let result = self.run_joiner_interaction(i, req);
        let (accepted, consume, note) = match &result {
            Some(r) => (r.done, if r.done { r.consume } else { 0 }, r.note.to_wire()),
            None => (false, 0, 0),
        };
        self.send_outcome(i, req.seq, req.entity, Some(req.kind), accepted, consume, note);
        if let Some(r) = result
            && r.done
        {
            // C1 — the server's shadow of the joiner's inventory follows the
            // outcome as its client does: what it used, owed from wherever
            // the item is now (`joiner_actions::take_owed`, the client's own
            // rule), then the products (`InventoryGrant`), in the order the
            // client applies them.
            if consume > 0
                && let Some(held) =
                    held_item_from_wire(req.held_kind, req.held_id, &req.held_full, &self.server.registry)
            {
                self.shadow_take_owed(i, usize::from(req.hotbar_slot), &held, consume, "in an interaction");
            }
            self.grant_to_joiner(i, r.give);
        }
    }

    /// C2a — slot `i`'s `ItemAction`, judged and applied by `item_actions`:
    /// an eat feeds and heals the body and takes the food from the server's
    /// shadow of the joiner's inventory (a shortfall is log-only); a sleep
    /// sets the spawn point at the bed and heals the body. Always answered
    /// with an `ItemActionOutcome` (the client's claim on the food waits for
    /// it, `joiner_actions`).
    fn handle_item_action(&mut self, i: usize, req: &protocol::ItemActionPacket) {
        use crate::item_actions::{self, ItemNote};
        let served: Result<u8, ItemNote> = match &req.action {
            protocol::ItemAction::Eat { hotbar_slot, held_kind, held_id, held_full } => {
                let held = held_item_from_wire(*held_kind, *held_id, held_full, &self.server.registry);
                let eaten = self
                    .server
                    .players
                    .get_mut(i)
                    .map_or(Err(ItemNote::NotNow), |sp| item_actions::serve_eat(sp, held.as_ref()));
                if eaten.is_ok()
                    && let Some(held) = &held
                {
                    self.shadow_take_owed(i, usize::from(*hotbar_slot), held, 1, "by eating");
                }
                eaten.map(|()| 1)
            }
            protocol::ItemAction::Sleep { bed } => {
                let server = &mut self.server;
                // C2a verify L1 — the clock can be ahead of the calendar here
                // (a lending host sets it before the tick; a `/time` jump
                // lands before inbound processing), and `GameServer::tick`
                // only shows it the clock after this. Show it now: `observe`
                // is idempotent for the same reading, so the tick's own call
                // changes nothing, and `tonight()` is this tick's night.
                server.night_calendar.observe(server.world_time);
                let (world, world_time, tonight) =
                    (&server.world, server.world_time, server.night_calendar.tonight());
                server
                    .players
                    .get_mut(i)
                    .map_or(Err(ItemNote::NotNow), |sp| {
                        item_actions::serve_sleep(sp, world, world_time, tonight, *bed)
                    })
                    .map(|()| 0)
            }
            // C2b — fire-and-forget: mirrored on the server, never answered.
            protocol::ItemAction::Craft { grid, table } => return self.mirror_joiner_craft(i, grid, *table),
            protocol::ItemAction::Drop { hotbar_slot, held_kind, held_id, held_full } => {
                let held = held_item_from_wire(*held_kind, *held_id, held_full, &self.server.registry);
                return self.spawn_joiner_drop(i, usize::from(*hotbar_slot), held);
            }
        };
        let (accepted, consume_held, note) = match served {
            Ok(n) => (true, n, ItemNote::None.to_wire()),
            Err(note) => (false, 0, note.to_wire()),
        };
        let pkt = protocol::serialize_packet(
            protocol::PacketType::ItemActionOutcome,
            &protocol::ItemActionOutcomePacket { seq: req.seq, accepted, consume_held, note },
        );
        self.send_to_joined_slot(i, &pkt);
    }

    /// C1 — joiner `i`'s accepted request (an interaction, C2a an eat) used
    /// `n` of `held`, claimed from hotbar slot `slot`: take them from the
    /// server's shadow of its inventory (`joiner_actions::take_owed`). What
    /// the shadow can't pay is a possession mismatch — counted and logged
    /// (rate-limited), never refused. `how` ends the log line's "used … ".
    fn shadow_take_owed(&mut self, i: usize, slot: usize, held: &crate::item::Item, n: u8, how: &str) {
        let Some(sp) = self.server.players.get_mut(i) else { return };
        let taken = crate::joiner_actions::take_owed(&mut sp.inventory, slot, held, n);
        self.note_shortfall(i, held, n, taken, how);
    }

    /// C1 — joiner `i` used `n` of `held` `how`, and the shadow paid `taken`:
    /// a shortfall is a possession mismatch, counted and logged
    /// (rate-limited), never refused.
    fn note_shortfall(&mut self, i: usize, held: &crate::item::Item, n: u8, taken: u8, how: &str) {
        if taken >= n {
            return;
        }
        let tick = self.server.tick_counter;
        let Some(sp) = self.server.players.get_mut(i) else { return };
        let due = sp.possession.note_mismatch(tick);
        log::log!(
            crate::joiner_inventory::mismatch_log_level(due),
            "possession check (log-only): {} used {n} × {held:?} {how}; the server's copy \
             of their inventory held {taken}{} — accepted",
            sp.display_name,
            held_back_note(due),
        );
    }

    /// C2b — joiner `i` crafted once from `grid` (`table`: the crafting table
    /// its 3×3 grid was opened from): judged and mirrored on the server's
    /// shadow of its inventory by `item_actions::serve_craft` (one of each
    /// input taken, owed; the output added). Never answered — the client's
    /// own craft stands. An input the shadow can't pay is a log-only
    /// mismatch; an output that doesn't fit is counted, not spilled (the
    /// client holds it); a refusal leaves the shadow as it was and is counted
    /// by reason.
    // BRIDGE: replaced when C3a mirrors the craft grid as window state (the
    // result click becomes a window op); judge_craft's rule carries over.
    fn mirror_joiner_craft(&mut self, i: usize, grid: &[(u8, u16); 9], table: Option<[i32; 3]>) {
        let server = &mut self.server;
        let Some(sp) = server.players.get_mut(i) else { return };
        let short = match crate::item_actions::serve_craft(sp, &server.world, &server.registry, grid, table) {
            Ok(applied) => {
                sp.possession.crafts = sp.possession.crafts.saturating_add(1);
                sp.possession.craft_overflow =
                    sp.possession.craft_overflow.saturating_add(u32::from(applied.overflow));
                applied.short
            }
            Err(why) => {
                log::debug!("{}'s craft not mirrored: {}", sp.display_name, why.label());
                sp.possession.note_craft_refused(why);
                return;
            }
        };
        for item in &short {
            self.note_shortfall(i, item, 1, 0, "in a craft");
        }
    }

    /// C2b — joiner `i` Q-dropped one of `held` from hotbar slot `slot`: the
    /// CLAIMED item (full fidelity — the client's wear is nearer the truth
    /// than the shadow's) is thrown from the server body as a real ground
    /// item everyone sees and can pick up (`entity::q_drop_launch` +
    /// `spawn_thrown_item`, the client's own drop; the dropper waits out
    /// `ITEM_DROP_PICKUP_DELAY_TICKS`), and taken from the shadow by the
    /// owed rule. A shortfall is a log-only mismatch and the item still
    /// spawns. Never answered. The drop already spent a token of the
    /// joiner's drop bucket (it waited in the inbound queue for one).
    ///
    /// BRIDGE: possession check — a modified client can drop an item it
    /// doesn't hold, and that item is then real for everyone. Closes with
    /// enforcement (C3).
    fn spawn_joiner_drop(&mut self, i: usize, slot: usize, held: Option<crate::item::Item>) {
        let tick = self.server.tick_counter;
        let Some(sp) = self.server.players.get_mut(i) else { return };
        sp.drop_bucket.take(tick);
        let Some(drop) = crate::item_actions::serve_drop(sp, slot, held) else { return };
        sp.possession.drops = sp.possession.drops.saturating_add(1);
        let (pos, velocity) = crate::entity::q_drop_launch(drop.eye, drop.forward);
        crate::entity::spawn_thrown_item(&mut self.server.ecs, pos, velocity, drop.stack.clone(), i as u8);
        if !drop.paid {
            self.note_shortfall(i, &drop.stack.item, 1, 0, "in a Q-drop");
        }
    }

    fn run_joiner_interaction(
        &mut self,
        i: usize,
        req: &protocol::EntityInteractPacket,
    ) -> Option<crate::mob_interact::Interaction> {
        // Review D2b B3 — a Lead on a fence post names a block, not a mob.
        if let protocol::InteractKind::LeadToPost { post } = req.kind {
            return self.run_joiner_lead_to_post(i, req, post);
        }
        let (target, kind) = self.joiner_target(i, req.entity)?;
        // Review D2b MEDIUM-2 — what this world doesn't simulate (breeding,
        // Leads, pets following) is refused, not taken for nothing.
        if !self.server.animal_life_simulated && crate::mob_interact::needs_animal_life(req.kind) {
            return Some(crate::mob_interact::Interaction::refused(
                crate::mob_interact::InteractNote::NotOnThisServer,
            ));
        }
        let tick = self.server.tick_counter;
        let sp = &mut self.server.players[i];
        if sp.interact_cooldown > 0 {
            return None;
        }
        sp.interact_cooldown = INTERACT_COOLDOWN_TICKS;
        // BRIDGE: possession check — replace when phase C makes joiner
        // inventories server-authoritative. The food, bucket, shears or Lead
        // in hand is the client's word; the client gives up what an accepted
        // outcome says it used.
        let held = held_item_from_wire(req.held_kind, req.held_id, &req.held_full, &self.server.registry);
        let key = sp.pet_owner_key();
        let actor = crate::mob_interact::Actor {
            owner_key: key.as_deref(),
            tether: crate::tether::TetherTarget::Player(i),
            who: crate::combat::Attacker::Remote { slot: i, generation: sp.attach_gen },
        };
        crate::mob_interact::run(
            &mut self.server.ecs,
            target,
            kind,
            req.kind,
            held.as_ref(),
            req.sneak,
            &actor,
            tick,
        )
    }

    /// Review D2b B3 — joiner `i` puts a Lead on the fence post at `post`:
    /// `mob_interact::lead_to_post`, the single-player rule, with the
    /// joiner's own leashed mobs (`TetherTarget::Player(i)`). The post must
    /// be within block reach of the server body's eye (the reach a joiner's
    /// block edit gets) and the server must run the Leads (MEDIUM-2); the
    /// interaction cooldown applies.
    fn run_joiner_lead_to_post(
        &mut self,
        i: usize,
        req: &protocol::EntityInteractPacket,
        post: [i32; 3],
    ) -> Option<crate::mob_interact::Interaction> {
        let sp = self.server.players.get(i)?;
        if !sp.server_simulated || !sp.is_present_and_alive() {
            return None;
        }
        let centre = glam::Vec3::new(post[0] as f32 + 0.5, post[1] as f32 + 0.5, post[2] as f32 + 0.5);
        // A joiner's block-edit reach: the held item is its word, so no
        // reach bonus.
        if !block_change_within_reach((centre - sp.player.eye_pos()).length_squared(), 0, 0, true) {
            return None;
        }
        if !self.server.animal_life_simulated {
            return Some(crate::mob_interact::Interaction::refused(
                crate::mob_interact::InteractNote::NotOnThisServer,
            ));
        }
        let sp = &mut self.server.players[i];
        if sp.interact_cooldown > 0 {
            return None;
        }
        sp.interact_cooldown = INTERACT_COOLDOWN_TICKS;
        // BRIDGE: possession check — the Lead in hand is the client's word
        // (see `run_joiner_interaction`).
        let held = held_item_from_wire(req.held_kind, req.held_id, &req.held_full, &self.server.registry);
        let actor = crate::mob_interact::Actor {
            owner_key: None,
            tether: crate::tether::TetherTarget::Player(i),
            who: crate::combat::Attacker::Remote { slot: i, generation: sp.attach_gen },
        };
        crate::mob_interact::lead_to_post(&mut self.server.ecs, &self.server.world, post, held.as_ref(), &actor)
    }

    #[allow(clippy::too_many_arguments)]
    fn send_outcome(
        &self,
        i: usize,
        seq: u32,
        entity: u32,
        kind: Option<protocol::InteractKind>,
        accepted: bool,
        consume_held: u8,
        note: u8,
    ) {
        let pkt = protocol::serialize_packet(
            protocol::PacketType::InteractOutcome,
            &protocol::InteractOutcomePacket { seq, entity, kind, accepted, consume_held, note },
        );
        self.send_to_joined_slot(i, &pkt);
    }

    /// MP-A3 — slot `i`'s `Respawn` request. Honoured only for a joined,
    /// server-simulated player whose server copy has been dead long enough
    /// (`GameServer::respawn_player`); anything else is ignored (a living
    /// player's Respawn would be a free teleport). Answers `Respawned` to that
    /// player alone.
    ///
    /// Final review fix 1 — also ignored until the column of their spawn point
    /// is loaded: the respawn stands the body on that column's ground
    /// (`GameServer::standing_spot`), which reads only air from an unloaded
    /// one (a lent world after a long trip: the host's streamer has dropped
    /// it), and the body would be put in the air above ground that arrives a
    /// frame later. The host's streamer anchors the spawn column of every dead
    /// joiner (`lent_respawn_columns`), and the joiner's client re-sends
    /// `Respawn` every ~20 ticks until `Respawned` arrives, so the wait is a
    /// few frames. Where the server owns its world it loads the column itself
    /// once the respawn is due (C2a: a joiner's bed spawn can be far from the
    /// join spawn's 3x3 and the dedicated streamer's anchors).
    fn handle_respawn(&mut self, i: usize) {
        if !self.handshake_done[i] || self.disconnected[i] {
            return;
        }
        let Some(sp) = self.server.players.get(i).filter(|sp| sp.server_simulated) else {
            return;
        };
        let column = crate::chunk_stream::column_of(sp.spawn_pos);
        // C2a — a bed spawn can be anywhere, far from every column an owning
        // server keeps loaded (its players' and the world spawn's): it loads
        // that one column itself, once the respawn is due. A lent world's is
        // the host client's to stream (`lent_respawn_columns`).
        if !self.lends_host_world()
            && sp.combat.dead
            && sp.dead_ticks >= crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN
        {
            self.server.ensure_column_loaded(column.0, column.1);
        }
        if !self.server.loaded_columns.contains(&column) {
            return;
        }
        if let Some(at) = self.server.respawn_player(i) {
            // FU4a (FU3 verify L1) — a new life: edits still waiting from the
            // last one are sent back, not applied (`process_edit_group`).
            self.server.players[i].respawns = self.server.players[i].respawns.wrapping_add(1);
            let pkt = protocol::serialize_packet(
                protocol::PacketType::PlayerEvent,
                &protocol::PlayerEventPacket {
                    player_index: i as u32,
                    event: protocol::PlayerEventType::Respawned { x: at.x, y: at.y, z: at.z },
                },
            );
            self.send_to_joined_slot(i, &pkt);
        }
    }

    /// Free every remote slot whose connection has gone (peer closed, network
    /// error, idle timeout) or that has sat past `PRE_AUTH_TIMEOUT_TICKS`
    /// without completing its join. Runs every tick after inbound packets, so a
    /// transport's last packets (a `Disconnect`) are read first.
    ///
    /// FU3 (FU1 verify N6) — only a slot whose transport was ALREADY closed
    /// before this tick's fill (`closed_before_fill`, from
    /// [`Self::process_inbound_packets`]) and whose queue is empty now is
    /// freed: a connection's bridge hands over every frame before it marks
    /// the connection closed, so that fill took everything it sent. A slot
    /// that closed after the fill (its last frame may have landed after it
    /// too) waits a tick, and one with packets still waiting past the
    /// per-tick budget (FU1) waits until they have been read.
    fn reap_slots(&mut self, closed_before_fill: &[bool]) {
        for i in self.num_local_players..self.transports.len() {
            if self.disconnected[i] {
                continue;
            }
            if closed_before_fill.get(i).copied().unwrap_or(false) && self.inbound[i].is_empty() {
                log::info!("Player {i}: connection closed");
                if let Some(left) = self.release_slot(i, None) {
                    self.send_to_joined_except(i, &left);
                }
            } else if !self.handshake_done[i]
                && self.server_tick.saturating_sub(self.attached_tick[i]) >= PRE_AUTH_TIMEOUT_TICKS
            {
                log::info!("Slot {i}: no completed join in time — freeing it");
                let _ = self.release_slot(i, Some("Join timed out"));
            }
        }
    }

    /// Test-only: jump the server clock `ticks` forward, so the pre-auth
    /// timeout can be driven without simulating that many ticks.
    #[cfg(test)]
    pub(crate) fn advance_clock_for_test(&mut self, ticks: u64) {
        self.server_tick += ticks;
    }

    /// Test-only: how many of slot `slot`'s packets wait in its inbound queue.
    #[cfg(test)]
    pub(crate) fn inbound_len_for_test(&self, slot: usize) -> usize {
        self.inbound.get(slot).map_or(0, transport::InboundQueue::len)
    }

    /// Test-only: how many of slot `slot`'s block edits wait past the budget.
    #[cfg(test)]
    pub(crate) fn edit_queue_len_for_test(&self, slot: usize) -> usize {
        self.edit_queues.get(slot).map_or(0, crate::edit_queue::EditQueue::len)
    }

    /// Test-only: whether slot `slot` is currently free.
    #[cfg(test)]
    pub(crate) fn slot_is_free(&self, slot: usize) -> bool {
        self.disconnected.get(slot).copied().unwrap_or(true)
    }

    /// Run one authoritative server tick at 20 TPS. Callers drive this
    /// from the main loop's tick accumulator.
    ///
    /// Order of operations (preserves the previous threaded behaviour):
    /// 1. Drain newly-connected remote transports onto the server
    /// 2. Process inbound packets from every connected transport
    /// 3. Tick the simulation
    /// 4. Broadcast a StateUpdate to every completed-handshake client
    /// 5. Heartbeat LAN discovery (native + remote players enabled only)
    ///
    /// A lending server (D1) is ticked ONLY inside the lend window —
    /// `sim_lend::LentSim::lend(hs, parts, clock).tick()` — so every step
    /// above reads and writes the host client's own world.
    pub fn tick(&mut self) {
        debug_assert_eq!(
            self.lends_host_world(),
            self.server.lent,
            "a lending HostedServer ticks only inside a LentSim window (and an owning one never does)"
        );
        self.accept_new_remote_connections();
        let closed_before_fill = self.process_inbound_packets();
        self.reap_slots(&closed_before_fill);
        self.server.tick();
        self.announce_joiner_deaths();
        self.announce_joiner_hits_and_kills();
        // World chat §4.2 — inbound room lines, delivered under the same
        // hearing rule as an in-world speaker. No-op unless a room is attached.
        #[cfg(not(target_arch = "wasm32"))]
        self.poll_room();
        // Death-drops phase 2b — deliver this tick's server-side pickup
        // grants to the players who earned them. Per-connection (never
        // broadcast: the stack belongs to one inventory); the item's
        // disappearance for everyone rides the entity-despawn diff below.
        let grants = std::mem::take(&mut self.server.pending_item_grants);
        for (idx, pkt) in build_grant_packets(&grants) {
            if self.handshake_done.get(idx).copied().unwrap_or(false)
                && !self.disconnected.get(idx).copied().unwrap_or(true)
            {
                self.transports[idx].send_to_client(&pkt);
            }
        }
        self.server_tick += 1;
        self.broadcast_state();
        // Operator Console snapshot (Spec B task 7 / B-7a): stream to the
        // verified-operator player ~every 2s. No-op unless an operator is in-game.
        #[cfg(not(target_arch = "wasm32"))]
        if self.server_tick.is_multiple_of(40) {
            self.broadcast_operator_snapshot();
        }
        self.heartbeat_discovery();
    }

    /// Pull any new transports the QUIC accept thread has pushed and
    /// attach them as server-simulated players. No-op on WASM.
    fn accept_new_remote_connections(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(rx) = &self.remote_rx {
            // Drain the channel first — `attach_remote_transport` needs
            // `&mut self`, which the `rx` borrow would otherwise block.
            let mut incoming: Vec<Box<dyn ServerTransport>> = Vec::new();
            while let Ok(transport) = rx.try_recv() {
                incoming.push(transport);
            }
            for transport in incoming {
                self.attach_remote_transport(transport);
            }
        }
    }

    /// Where a joiner is placed, and so where it respawns
    /// (`ServerPlayer::spawn_pos`): the one place that decides, and the spawn
    /// `JoinAccept` names — so the server's body for the joiner and the
    /// joiner's own client start from one position (Spec 04 §5.3.1).
    ///
    /// A server with a host client (LAN / online host: `num_local_players > 0`)
    /// puts the joiner beside the host's own body, as it always has — with the
    /// columns round that spot loaded, since the host may have walked far from
    /// where hosting began and a body can't step in an unloaded column (an
    /// owning `--no-lend` server generates them; a lending host's streamer
    /// holds them already, as they are beside the host).
    /// Otherwise it is the world's spawn point (`GameServer::world_spawn`, the
    /// computed surface spawn) — never another joiner's position: slot 0 is a
    /// stranger on a dedicated server, and a respawn point taken from wherever
    /// they happened to stand (a trap) would be everyone's.
    fn join_spawn(&mut self) -> glam::Vec3 {
        let host = self.server.players.first().map(|host| host.player.pos);
        match host {
            Some(p) if self.num_local_players > 0 => {
                let spawn = glam::Vec3::new(p.x + 3.0, p.y, p.z);
                // A lent world's columns are the host client's streamer's to
                // load (it anchors on every joiner, D1): beside the host they
                // are loaded already, and generating here — outside the lend
                // window, into the server's empty between-window world, or
                // inside it, behind the streamer's back — would be a second
                // generator.
                if !self.lends_host_world() {
                    let (cx, cz) = crate::chunk_stream::column_of(spawn);
                    for dx in -1..=1 {
                        for dz in -1..=1 {
                            self.server.ensure_column_loaded(cx + dx, cz + dz);
                        }
                    }
                }
                spawn
            }
            _ => self.server.world_spawn(),
        }
    }

    /// Attach one remote transport as a new server-simulated player slot and
    /// issue its Phase 4 join challenge. Shared by the accept-thread drain
    /// above and the in-process test harness.
    #[cfg(not(target_arch = "wasm32"))]
    fn attach_remote_transport(&mut self, transport: Box<dyn ServerTransport>) {
        let mut remote = crate::server::ServerPlayer::new(self.join_spawn());
        // Remote players run server-simulated physics (Task 1d).
        remote.server_simulated = true;
        // Review D2b MEDIUM-1 — a fresh generation per connection, so nothing
        // credited to an earlier occupant of a reused slot reaches this one.
        self.server.next_attach_gen = self.server.next_attach_gen.wrapping_add(1).max(1);
        remote.attach_gen = self.server.next_attach_gen;
        // Not in the world until the join handshake completes: no survival
        // damage, no pickups, no mob targeting (`ServerPlayer::awaiting_join`).
        remote.awaiting_join = true;
        // Reuse the lowest freed remote slot before growing the vectors, so
        // they stay bounded by peak concurrency however many connections come
        // and go (audit 2026-09-27).
        let reuse = (self.num_local_players..self.transports.len()).find(|&j| self.disconnected[j]);
        let slot = match reuse {
            Some(j) => {
                self.server.players[j] = remote;
                self.transports[j] = transport;
                self.handshake_done[j] = false;
                self.disconnected[j] = false;
                self.attached_tick[j] = self.server_tick;
                self.inbound[j].clear();
                self.edit_queues[j].clear();
                self.outboxes[j] = crate::state_outbox::ClientOutbox::new(true);
                self.entity_interest[j] = Default::default();
                self.chunk_pushes[j] = crate::chunk_push::ClientChunkPush::default();
                j
            }
            None => {
                self.server.players.push(remote);
                self.transports.push(transport);
                self.handshake_done.push(false);
                self.disconnected.push(false);
                self.attached_tick.push(self.server_tick);
                self.inbound.push(transport::InboundQueue::default());
                self.edit_queues.push(Default::default());
                self.outboxes.push(crate::state_outbox::ClientOutbox::new(true));
                self.entity_interest.push(Default::default());
                self.chunk_pushes.push(crate::chunk_push::ClientChunkPush::default());
                self.transports.len() - 1
            }
        };

        // Phase 4: issue a fresh per-connection challenge and push it
        // immediately. No origin rides with it (v63): the client builds the
        // origin from its own transport's channel binding. An authenticated client waits for this,
        // signs the nonce into its kind-21236 auth event, and sends the
        // result in its JoinRequest. Bounded-table refusal (None) logs
        // and falls through — a client that then sends an auth_event
        // hits the "no live challenge" reject; a guest is accepted only
        // on an open (non-require_signin) server.
        let key = challenge_key_for_slot(slot);
        if let Some(nonce) = self.challenges.issue(key) {
            let pkt = protocol::ChallengePacket {
                nonce_hex: hex::encode(nonce),
            };
            let bytes = protocol::serialize_packet(protocol::PacketType::Challenge, &pkt);
            self.transports[slot].send_to_client(&bytes);
        } else {
            log::warn!(
                "Challenge table full ({} entries) — refusing nonce for new slot {slot}",
                self.challenges.len()
            );
        }

        log::info!(
            "Remote player added (total players: {})",
            self.server.players.len()
        );
    }

    /// Test-only: attach an in-process channel transport as a remote player
    /// and return the client half, so integration tests can drive a real
    /// join handshake through `tick()` without sockets or accept threads.
    #[cfg(test)]
    pub(crate) fn attach_test_remote(&mut self) -> transport::ChannelClientTransport {
        let (server_side, client_side) = transport::channel_pair();
        // Mirror the accept thread, which reserves the seat before handing
        // the transport over.
        self.current_remote.fetch_add(1, Ordering::Relaxed);
        self.attach_remote_transport(Box::new(server_side));
        client_side
    }

    /// Test-only: like [`Self::attach_test_remote`], but the server side of the
    /// transport reports `binding` as its channel binding (stands in for the
    /// QUIC TLS exporter).
    #[cfg(test)]
    pub(crate) fn attach_test_remote_with_binding(
        &mut self,
        binding: Option<[u8; 32]>,
    ) -> transport::ChannelClientTransport {
        let (server_side, client_side) = transport::channel_pair();
        self.current_remote.fetch_add(1, Ordering::Relaxed);
        self.attach_remote_transport(Box::new(transport::BoundServerTransport {
            inner: server_side,
            binding,
            websocket: false,
        }));
        client_side
    }

    /// Test-only (FU3, FU1 verify N6): attach a remote whose last frame lands
    /// just after a fill has emptied its channel, with the close right behind
    /// it (`transport::LateFrameServerTransport`). Put the frame in the
    /// returned slot to arm it.
    #[cfg(test)]
    pub(crate) fn attach_test_remote_late_frame(
        &mut self,
    ) -> (transport::ChannelClientTransport, std::sync::Arc<std::sync::Mutex<Option<transport::Packet>>>) {
        let (server_side, client_side) = transport::channel_pair();
        let late = std::sync::Arc::new(std::sync::Mutex::new(None));
        self.current_remote.fetch_add(1, Ordering::Relaxed);
        self.attach_remote_transport(Box::new(transport::LateFrameServerTransport::new(server_side, late.clone())));
        (client_side, late)
    }

    /// Test-only: attach a remote whose server side reports `is_websocket()`
    /// (no channel binding) — stands in for a WebSocket connection.
    #[cfg(test)]
    pub(crate) fn attach_test_remote_ws(&mut self) -> transport::ChannelClientTransport {
        let (server_side, client_side) = transport::channel_pair();
        self.current_remote.fetch_add(1, Ordering::Relaxed);
        self.attach_remote_transport(Box::new(transport::BoundServerTransport {
            inner: server_side,
            binding: None,
            websocket: true,
        }));
        client_side
    }

    /// Read and process every client's packets for this tick. Returns, per
    /// slot, whether its transport was already closed before this tick's
    /// fill (FU3, FU1 verify N6): `reap_slots` frees only such a slot, so a
    /// frame that lands between the fill and the close is still read next
    /// tick (the bridge sends every frame before it marks the connection
    /// closed, so a fill after the close has taken them all).
    fn process_inbound_packets(&mut self) -> Vec<bool> {
        // Queue of StateUpdate-shaped events to fan out after we've
        // finished reading. We can't send directly inside the read loop
        // because we mutate `self.transports`/`handshake_done`/etc. above.
        let mut broadcasts: VecDeque<(usize, Vec<u8>)> = VecDeque::new();
        // FU3 (N6) — read before any fill this tick.
        let closed_before_fill: Vec<bool> = self
            .transports
            .iter()
            .zip(&self.disconnected)
            .map(|(t, &gone)| !gone && t.is_closed())
            .collect();

        for i in 0..self.transports.len() {
            if self.disconnected[i] {
                continue;
            }
            // FU1 — everything the client sent goes to the back of its queue
            // first; the queue's hard bound is the one thing that ends a
            // connection for what it sends (no honest client gets there).
            let arrived = match self.inbound[i].fill_from(&*self.transports[i]) {
                Ok(arrived) => arrived,
                Err(over) => {
                    log::warn!(
                        "Player {i} has {} packets ({} bytes charged) waiting, past the inbound \
                         bound ({} bytes) — disconnecting it",
                        over.packets,
                        over.bytes,
                        transport::MAX_INBOUND_BYTES,
                    );
                    if let Some(left) = self.release_slot(i, Some(INBOUND_OVERFLOW_REASON)) {
                        broadcasts.push_back((i, left));
                    }
                    continue;
                }
            };
            // FU3 — a client catching up on a stall gets a bigger read budget
            // this tick (chosen once, from what waits after the fill). FU4a
            // (M2) — not one whose edit queue is full: no honest client is.
            let read_budget = if self.inbound[i].len() > CATCH_UP_QUEUE_LEN && !self.edit_queues[i].is_full() {
                CATCH_UP_PACKETS_PER_TICK
            } else {
                MAX_PACKETS_PER_TICK
            };
            let mut packets_this_tick = 0usize;
            let mut controls_this_tick = 0usize;
            let mut interacts_this_tick = 0usize;
            let mut entity_requests_this_tick = 0usize;
            let mut item_actions_this_tick = 0usize;
            // Per client per TICK, not per packet (audit 2026-09-27: the
            // budget reset for every packet, so 10 packets × 4 edits got in).
            let mut budget = EditTickBudget::default();
            // FU3 — the edits that waited past last tick's budget go first.
            self.process_waiting_edits(i, &mut budget);
            // C2b verify M4 — whether this client is replaying a backlog
            // (more than one ordinary tick's reading waiting after the fill):
            // each `ClientInput` it reads then credits the joiner's drop
            // bucket in client time. An honest client in step never has that
            // many waiting. (Not tied to the catch-up read budget: the tail of
            // a drain is below that budget's threshold and would otherwise
            // fall back to server-time pacing.)
            let catching_up = self.inbound[i].len() > MAX_PACKETS_PER_TICK;
            // C2b verify M4 — a transport that is closed now (read after the
            // fill, which has taken every frame it handed over before it closed).
            let closed_now = closed_before_fill.get(i).copied().unwrap_or(false) || self.transports[i].is_closed();
            loop {
                // C2b verify M4 — a connection that has closed gets none of
                // its queued Drops spawned: they are discarded (at no read
                // budget) so the queue empties and `reap_slots` can free the
                // slot, rather than the leaver's backlog dribbling out one
                // item per bucket token for hours.
                if closed_now && self.inbound[i].front().is_some_and(|p| is_drop_action(p))
                {
                    let _ = self.inbound[i].pop();
                    continue;
                }
                // FU1 — defer, don't drop: once the budget is spent, the rest
                // waits for the next tick, in arrival order. Control packets
                // (MP-A3: `Respawn`, `Disconnect`) cost no budget, so one at
                // the front is still read — but never ahead of what the client
                // sent before it (a Respawn read ahead of the health-0 inputs
                // queued before it would be undone by them). FU4a (L6) — only
                // the first FREE_CONTROL_PACKETS_PER_TICK of them a tick.
                if packets_this_tick >= read_budget
                    && !(controls_this_tick < FREE_CONTROL_PACKETS_PER_TICK
                        && self.inbound[i].front().is_some_and(|p| is_control_packet(p)))
                {
                    if !self.inbound[i].is_empty() {
                        log::debug!(
                            "Player {i}: {} packets wait for the next tick (budget {read_budget})",
                            self.inbound[i].len()
                        );
                    }
                    break;
                }
                // C2a — an item action past its budget waits too, and so does
                // everything behind it (one ordered stream). FU3 — so do an
                // entity request and a device interaction past theirs; C2b —
                // and a Q-drop the joiner's drop bucket can't pay for yet.
                // FU4a (L1) — and any request while this client's edits wait.
                let now = self.server.tick_counter;
                let drop_ready = self.server.players.get(i).is_none_or(|sp| sp.drop_bucket.ready(now));
                let edits_waiting = !self.edit_queues[i].is_empty();
                if (item_actions_this_tick >= MAX_ITEM_ACTIONS_PER_TICK
                    && self.inbound[i].front().is_some_and(|p| is_item_action(p)))
                    || self.inbound[i].front().is_some_and(|p| {
                        waits_for_kind_budget(
                            p,
                            entity_requests_this_tick,
                            interacts_this_tick,
                            drop_ready,
                            edits_waiting,
                        )
                    })
                {
                    break;
                }
                let Some(packet) = self.inbound[i].pop() else {
                    break;
                };
                if is_control_packet(&packet) && controls_this_tick < FREE_CONTROL_PACKETS_PER_TICK {
                    controls_this_tick += 1;
                } else {
                    packets_this_tick += 1;
                }
                let Some((ptype, payload)) = protocol::deserialize_header(&packet) else {
                    continue;
                };
                match ptype {
                    protocol::PacketType::JoinRequest => {
                        if self.handshake_done[i] {
                            // Re-sends after handshake are dropped silently.
                            continue;
                        }
                        let Ok(req) = protocol::safe_deserialize::<protocol::JoinRequestPacket>(payload)
                        else {
                            // Another version's JoinRequest shape doesn't
                            // decode; its leading `protocol_version` still
                            // reads, so the joiner learns why (not silence).
                            if let Some(v) = protocol::peek_protocol_version(payload)
                                && v != protocol::PROTOCOL_VERSION
                            {
                                log::warn!(
                                    "Rejecting undecodable JoinRequest on slot {i}: protocol v{v} (server v{})",
                                    protocol::PROTOCOL_VERSION
                                );
                                let _ = self.release_slot(i, Some(&protocol::protocol_mismatch_reason(v)));
                                break;
                            }
                            continue;
                        };
                        if !protocol::player_name_is_valid(&req.player_name) {
                            log::warn!(
                                "Rejecting JoinRequest on slot {i}: invalid name ({} bytes)",
                                req.player_name.len()
                            );
                            let _ = self.release_slot(i, Some(&protocol::player_name_reject_reason()));
                            break;
                        }
                        if req.protocol_version != protocol::PROTOCOL_VERSION {
                            log::warn!(
                                "Rejecting '{}': protocol v{} (server v{})",
                                req.player_name, req.protocol_version, protocol::PROTOCOL_VERSION
                            );
                            let reason = protocol::protocol_mismatch_reason(req.protocol_version);
                            let _ = self.release_slot(i, Some(&reason));
                            break;
                        }

                        // The origin this join must have signed — and the
                        // one our identity proof is signed over — built from
                        // OUR side, never from anything a relay supplied:
                        // audit fix B (v63) for QUIC (this connection's TLS
                        // exporter); v66 for WebSocket (the host the joiner
                        // declares it dialled, re-normalised and checked
                        // against our public hosts — guests included, since
                        // the proof is signed over it).
                        #[cfg(not(target_arch = "wasm32"))]
                        let expected_origin = if self.transports[i].is_websocket() {
                            match crate::signet::ws_host::expected_ws_join_origin(
                                &req.ws_host,
                                &self.ws_public_hosts,
                            ) {
                                Ok(origin) => origin,
                                Err(refusal) => {
                                    // The joiner gets the generic reason; the
                                    // configured hosts stay in our own log.
                                    log::warn!(
                                        "Rejecting JoinRequest on slot {i}: {}",
                                        refusal.detail
                                    );
                                    let _ = self.release_slot(i, Some(&refusal.reason));
                                    break;
                                }
                            }
                        } else {
                            signet::join_origin(self.transports[i].channel_binding())
                        };

                        // Phase 4 — verified identity. Verify any present
                        // auth_event (tamper/invalid → reject); reject an absent
                        // one only on a sign-in-required server. The verified
                        // handle is disambiguated against present players and
                        // stored, with the pubkey, on the ServerPlayer (the npub
                        // economy specs are waiting on). `join_name` is what we
                        // broadcast in the Joined event. On WASM there are no
                        // remote joins, so we keep the asserted name.
                        #[cfg(not(target_arch = "wasm32"))]
                        let (join_name, join_npub): (String, String) = {
                            let present: Vec<String> = self
                                .server
                                .players
                                .iter()
                                .enumerate()
                                .filter(|(idx, _)| *idx != i)
                                .map(|(_, p)| p.display_name.clone())
                                .filter(|s| !s.is_empty())
                                .collect();
                            let present_refs: Vec<&str> = present.iter().map(String::as_str).collect();
                            match resolve_join_identity(
                                i,
                                &req,
                                self.require_signin,
                                &mut self.challenges,
                                &expected_origin,
                                &present_refs,
                                &self.whitelist,
                                &self.blocklist,
                                // T2-8: the host's own contacts book names a known
                                // joiner. Read per verified join (rare), never
                                // cached — a Signet sync can change it mid-session.
                                crate::contacts::load_local_book,
                            ) {
                                Ok(id) => {
                                    // One seat per verified identity. Without
                                    // a channel binding (WS) a relayed
                                    // signature can't be told from a real
                                    // reconnect, so the newcomer is refused
                                    // while the old slot is live rather than
                                    // evicting it (review S6). QUIC replaces
                                    // after auth, below.
                                    let twin = id.pubkey.and_then(|pk| {
                                        (self.num_local_players..self.transports.len()).find(|&j| {
                                            j != i
                                                && !self.disconnected[j]
                                                && self.server.players[j].verified_pubkey
                                                    == Some(pk)
                                        })
                                    });
                                    if twin.is_some()
                                        && self.transports[i].channel_binding().is_none()
                                    {
                                        log::warn!(
                                            "Refusing slot {i}: that identity is already live on slot {twin:?} (unbound transport)"
                                        );
                                        let _ = self.release_slot(
                                            i,
                                            Some("You're already in this world from another connection."),
                                        );
                                        break;
                                    }
                                    if let Some(sp) = self.server.players.get_mut(i) {
                                        sp.verified_pubkey = id.pubkey;
                                        sp.display_name = id.display_name.clone();
                                        // World chat (Phase 3) — resolved at
                                        // join, never cached (§2.5, §3.3). A
                                        // guest (no verified key) gets no
                                        // chat regardless of `comms` — the
                                        // §3.5 gate in `handle_chat_say`
                                        // catches that — so there's nothing
                                        // to compute for them here.
                                        if let Some(pk) = id.pubkey {
                                            sp.comms = resolve_join_comms(
                                                crate::charter::comms_level(&pk),
                                                self.operator_comms,
                                            );
                                        }
                                    }
                                    // One seat per verified identity: a
                                    // reconnect (laptop woke up, client
                                    // restarted) replaces the old slot
                                    // rather than stacking a second player.
                                    if let Some(pk) = id.pubkey {
                                        let stale: Vec<usize> = (self.num_local_players
                                            ..self.transports.len())
                                            .filter(|&j| {
                                                j != i
                                                    && !self.disconnected[j]
                                                    && self.server.players[j].verified_pubkey
                                                        == Some(pk)
                                            })
                                            .collect();
                                        for j in stale {
                                            log::info!("Slot {j} replaced by a reconnect on slot {i}");
                                            if let Some(left) = self.release_slot(
                                                j,
                                                Some("You joined this world again from another connection."),
                                            ) {
                                                broadcasts.push_back((j, left));
                                            }
                                        }
                                    }
                                    log::info!(
                                        "Join accepted on slot {i}: {} (verified: {})",
                                        id.display_name,
                                        id.pubkey.is_some()
                                    );
                                    let npub = id.pubkey.map(|pk| pubkey_to_npub(&pk)).unwrap_or_default();
                                    (id.display_name, npub)
                                }
                                Err(reason) => {
                                    log::warn!("Rejecting JoinRequest on slot {i}: {reason}");
                                    let _ = self.release_slot(i, Some(&reason));
                                    break;
                                }
                            }
                        };
                        #[cfg(target_arch = "wasm32")]
                        let (join_name, join_npub): (String, String) =
                            (req.player_name.clone(), String::new());

                        // B2a — the chunk push starts afresh for this join,
                        // out to the render distance it announced.
                        self.chunk_pushes[i] =
                            crate::chunk_push::ClientChunkPush::new(req.render_distance);

                        // Record the joining client's announced skin reference
                        // on its (already-created, at transport-accept time)
                        // ServerPlayer. `collect_player_state` rebroadcasts it on
                        // every PlayerState. `0` = default skin. The skin bytes
                        // themselves are delivered out-of-band (gated).
                        if let Some(sp) = self.server.players.get_mut(i) {
                            sp.skin_key = req.skin_key;
                            // T2-9 — whose terrain generator differs from ours.
                            sp.client_worldgen_version = req.worldgen_version;
                            if sp.worldgen_mismatch() {
                                log::warn!(
                                    "Slot {i} generates terrain with worldgen {:#010x} (host {:#010x}): its terrain may differ",
                                    req.worldgen_version,
                                    crate::world::worldgen_fingerprint()
                                );
                            }
                        }

                        // The spawn point this slot was created at
                        // (`join_spawn`) — also where it respawns.
                        let spawn = if let Some(sp) = self.server.players.get(i) {
                            sp.spawn_pos
                        } else {
                            self.server.world_spawn()
                        };
                        // Track 3 — server-identity proof bound to the client's
                        // nonce, if this server is provisioned. Native-only
                        // (crypto); WASM hosts never reach here with remotes.
                        #[cfg(not(target_arch = "wasm32"))]
                        let server_identity = self.identity.as_ref().and_then(|id| {
                            hex::decode(&req.client_nonce_hex)
                                .ok()
                                .filter(|n| !n.is_empty())
                                .and_then(|nonce| {
                                    // Sign over the fixed domain-separation origin
                                    // (C1) joined to THIS join's origin (v63
                                    // channel binding; v66 checked WS host), so
                                    // a relaying host can't forward our proof to
                                    // a pinned client.
                                    crate::server_identity::identity_proof(
                                        id,
                                        &nonce,
                                        &crate::server_identity::proof::server_identity_origin(
                                            &expected_origin,
                                        ),
                                    )
                                })
                        });
                        #[cfg(target_arch = "wasm32")]
                        let server_identity: Option<protocol::ServerIdentityProof> = None;
                        let accept = protocol::JoinAcceptPacket {
                            player_index: i as u32,
                            // G3 — send the world's REAL seed so the joiner
                            // generates matching terrain (was hardcoded 42,
                            // desyncing every joiner's world from the host's).
                            seed: self.server.seed,
                            spawn_x: spawn.x,
                            spawn_y: spawn.y,
                            spawn_z: spawn.z,
                            world_time: self.server.world_time,
                            is_creative: self.server.play_mode.is_creative(),
                            play_mode: self.server.play_mode,
                            difficulty: self.difficulty.clone(),
                            server_identity,
                            // Creator Gallery (Spec 2026-06-19 §9): the world's
                            // authored exhibits, so the joiner renders them (they
                            // can't be regenerated client-side like the gallery).
                            exhibits: self.server.world.exhibits.clone(),
                            // T2-9 — the flags + generator version a joiner
                            // needs to build the same world before any terrain.
                            world_rules: self.server.world_rules(),
                            worldgen_version: crate::world::worldgen_fingerprint(),
                            // B2b — how far round its body this joiner hears
                            // a push or a "local" note for every column.
                            chunk_note_radius: if self.sends_notes(i) {
                                u8::try_from(self.chunk_push_limit()).unwrap_or(u8::MAX)
                            } else {
                                0
                            },
                        };
                        let pkt = protocol::serialize_packet(protocol::PacketType::JoinAccept, &accept);
                        self.transports[i].send_to_client(&pkt);
                        self.handshake_done[i] = true;
                        // The body enters the world now, at full health,
                        // hunger and breath, whatever happened to the copy
                        // while the handshake dragged on.
                        if let Some(sp) = self.server.players.get_mut(i) {
                            sp.enter_world();
                        }

                        // The operator joining without a channel binding plays
                        // normally but gets no operator tools — say so, so the
                        // missing console isn't a mystery (Spec 08 §9.0.1).
                        #[cfg(not(target_arch = "wasm32"))]
                        if self.transports[i].channel_binding().is_none()
                            && let Some(op) = self.operator_pubkey()
                            && crate::console_snapshot::is_operator(
                                self.server.players.get(i).and_then(|p| p.verified_pubkey),
                                Some(op),
                            )
                        {
                            self.send_chat_system(i, OPERATOR_NEEDS_DIRECT_NOTICE);
                        }

                        // Late joiner: no entity backfill needed — its
                        // interest set (`entity_interest[i]`) is empty, so
                        // every entity near it enters, with its spawn, on
                        // this tick's broadcast (MP-D2a).

                        // Operator telemetry (Spec B §6 / Spec C): record the
                        // connect for a verified REMOTE player — gated by the
                        // privacy level (default no-tracking = no-op; guest = no-op).
                        #[cfg(not(target_arch = "wasm32"))]
                        if i >= self.num_local_players {
                            crate::console_telemetry::capture_connect(
                                &mut self.session_log,
                                &self.privacy_level,
                                self.server.players[i].verified_pubkey,
                                current_unix_ts() as u64,
                            );
                        }

                        // Server-suggested resource pack (Spec 03 §11.6): if the
                        // operator configured one (AXENSTAX_PACK_URL), send it now
                        // the handshake is complete. Native only — a WASM host has
                        // no remote clients to suggest to.
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Some(suggest) = crate::resource_pack::configured_suggestion() {
                            let s = protocol::serialize_packet(
                                protocol::PacketType::ResourcePackSuggest,
                                &suggest,
                            );
                            self.transports[i].send_to_client(&s);
                        }

                        // Queue a join-event broadcast for every other
                        // handshake-complete client.
                        let event = protocol::PlayerEventPacket {
                            player_index: i as u32,
                            event: protocol::PlayerEventType::Joined {
                                name: join_name,
                                npub: join_npub,
                            },
                        };
                        let event_pkt = protocol::serialize_packet(
                            protocol::PacketType::PlayerEvent,
                            &event,
                        );
                        broadcasts.push_back((i, event_pkt));
                    }
                    protocol::PacketType::ClientInput => {
                        if !self.handshake_done[i] {
                            continue;
                        }
                        let Ok(mut input) = protocol::safe_deserialize::<protocol::InputPacket>(payload)
                        else {
                            continue;
                        };
                        // C2b verify M4 — in a catch-up, one input is one
                        // client tick of drop-bucket credit.
                        if catching_up && let Some(sp) = self.server.players.get_mut(i) {
                            sp.drop_bucket.credit_client_input(now);
                        }
                        // B2a — the chunk push's credit window, the columns the
                        // client let go of and its render distance. Cumulative /
                        // as-of / idempotent, so a stale or repeated packet can't
                        // undo a newer one; taken from EVERY input — one refused
                        // below (non-finite, stale, from a dead joiner) included.
                        self.take_chunk_feedback(i, &input);
                        if !input.x.is_finite()
                            || !input.y.is_finite()
                            || !input.z.is_finite()
                            || !input.yaw.is_finite()
                            || !input.pitch.is_finite()
                            || !input.health.is_finite()
                            || !input.move_forward.is_finite()
                            || !input.move_right.is_finite()
                        {
                            continue;
                        }
                        let Some(sp) = self.server.players.get_mut(i) else {
                            continue;
                        };
                        if !input_tick_is_fresh(input.tick, sp.last_input_tick) {
                            continue;
                        }
                        sp.last_input_tick = input.tick;
                        // MP-A3 — death is server-held for a joiner. While
                        // dead its input is ignored: no moves, no look, no
                        // edits — each edit is sent back so the ghost block
                        // un-places (FU4a: within the per-tick send-back
                        // cap). Zero health in its input is its own sim
                        // reporting a death; that is taken at the END of this
                        // packet, because the edits riding with it were made
                        // while the player was still alive.
                        let simulated = sp.server_simulated;
                        if simulated && sp.combat.dead {
                            for bc in &input.block_changes {
                                self.send_back_authoritative_block(bc, &mut budget);
                            }
                            continue;
                        }
                        let reports_death = simulated && input.health <= 0.0;
                        sp.yaw = input.yaw;
                        sp.pitch = input.pitch;
                        sp.held_item = input.held_item;
                        // Tool-capable held ref, client-authoritative. The
                        // only live source of the broadcast held item for
                        // server-simulated players (their server-side
                        // inventory is only the server's shadow, C1).
                        sp.held_kind = input.held_kind;
                        sp.held_id = input.held_id;
                        // MP-D2a — what its client says it wears; soaks the
                        // mob and lava/fire hits the server lands on a
                        // joiner (`GameServer::tick_player_hazards`).
                        sp.armour_points = input.armour_points;
                        // Keep hotbar_slot live so server_player_item_ref is
                        // correct for the local (position-trusted) path.
                        if let Some(slot) = input.hotbar_slot
                            && (slot as usize) < 9 {
                                sp.hotbar_slot = slot as usize;
                            }
                        // Phase 2 — latch sneak + "acting" (break/place this
                        // tick) for the avatar broadcast. pending_intent is
                        // consumed by tick_player_physics before broadcast,
                        // so the broadcast can't read it; stash it here.
                        sp.last_sneak = input.sneak;
                        sp.last_acting = input.break_block || input.place_block;
                        sp.last_move_mag =
                            (input.move_forward.powi(2) + input.move_right.powi(2)).sqrt();

                        if sp.server_simulated {
                            // Queued, not overwritten: two inputs bunched into
                            // one tick by jitter are both simulated (one per
                            // tick), instead of one step silently vanishing.
                            sp.queue_input(crate::server::QueuedInput::from_packet(&input));
                        } else {
                            // Local / position-trusted path — by design, not
                            // debt: a local slot is the host's own player on
                            // the host's own machine, and the host is the
                            // authority's own machine (Spec 04 §5.3; D1, Q1
                            // trap 9) — on a lent world the host client that
                            // moves it also owns the world. Its position is
                            // applied as sent, so the input is applied now.
                            sp.player.pos = glam::Vec3::new(input.x, input.y, input.z);
                            sp.combat.health = input.health.clamp(0.0, 20.0);
                            sp.last_applied_input = input.tick;
                        }
                        // D1 — on a lent world a local slot's edits are
                        // already in the world (the host client made them,
                        // with its own fluid, power, leaf and container
                        // bookkeeping) and already meshed. No budget, reach,
                        // validation or apply: only the broadcast to joiners,
                        // and never a send-back (re-sending a cell the host
                        // itself changed would only race its next edit).
                        if !simulated && self.lends_host_world() {
                            self.pending_block_changes.extend(input.block_changes.iter().cloned());
                            continue;
                        }
                        // C1/FU1 — this input's `mined` tags (the first
                        // MAX_MINED_PER_INPUT, its DoS guard), each paired
                        // with its own edit as the input is read (FU4a, L2:
                        // `edit_queue::EditGroup::new`): the break it was sent
                        // with. A tag goes where its edit goes (FU3: through
                        // the wait past the budget too): a refused edit's tag
                        // yields nothing and is gone with it.
                        // Every edit — from every packet this tick — goes
                        // through the one validator; the budget is per tick,
                        // and what is past it waits (FU3).
                        let edits = std::mem::take(&mut input.block_changes);
                        let held = (input.held_kind, input.held_id);
                        self.process_or_queue_edits(i, edits, &input.mined, held, &mut budget);
                        // MP-A3 — a reported death (only ever believed
                        // downward: health coming back is never taken — only
                        // a `Respawn` revives). Drops this packet's move too.
                        if reports_death {
                            self.server.report_player_death(i);
                        }
                    }
                    protocol::PacketType::DeviceInteract => {
                        if !self.handshake_done[i] || self.disconnected[i] {
                            continue;
                        }
                        // Never past the budget: one at it waits at the
                        // front (`waits_for_kind_budget`).
                        interacts_this_tick += 1;
                        let Ok(req) =
                            protocol::safe_deserialize::<protocol::DeviceInteractPacket>(payload)
                        else {
                            continue;
                        };
                        if self.joiner_is_dead(i) {
                            // MP-A3 — a dead joiner flips nothing.
                            continue;
                        }
                        self.handle_device_interact(i, req.pos);
                    }
                    protocol::PacketType::Respawn => {
                        // MP-A3 — the joiner chose Respawn on its death screen.
                        self.handle_respawn(i);
                    }
                    protocol::PacketType::EntityAttack | protocol::PacketType::EntityInteract => {
                        if !self.handshake_done[i] || self.disconnected[i] {
                            continue;
                        }
                        // Never past the budget (`waits_for_kind_budget`).
                        entity_requests_this_tick += 1;
                        if ptype == protocol::PacketType::EntityAttack {
                            if let Ok(req) =
                                protocol::safe_deserialize::<protocol::EntityAttackPacket>(payload)
                            {
                                self.handle_entity_attack(i, &req);
                            }
                        } else if let Ok(req) =
                            protocol::safe_deserialize::<protocol::EntityInteractPacket>(payload)
                        {
                            self.handle_entity_interact(i, &req);
                        }
                    }
                    protocol::PacketType::ItemAction => {
                        if !self.handshake_done[i] || self.disconnected[i] {
                            continue;
                        }
                        item_actions_this_tick += 1;
                        if let Ok(req) =
                            protocol::safe_deserialize::<protocol::ItemActionPacket>(payload)
                        {
                            self.handle_item_action(i, &req);
                        }
                    }
                    // Native-only — the web build carries no chat surface at
                    // all (docs/foundations/2026-09-05-world-chat.md §6); on
                    // wasm this falls through to the wildcard arm below.
                    #[cfg(not(target_arch = "wasm32"))]
                    protocol::PacketType::ChatSay => {
                        if !self.handshake_done[i] || self.disconnected[i] {
                            continue;
                        }
                        let Ok(say) =
                            protocol::safe_deserialize::<protocol::ChatSayPacket>(payload)
                        else {
                            continue;
                        };
                        self.handle_chat_say_packet(i, &say.text);
                    }
                    protocol::PacketType::Disconnect => {
                        // `release_slot` is idempotent, so a duplicate
                        // Disconnect can't double-release the seat (engine
                        // audit 2026-06-04, D).
                        log::info!("Player {i} disconnected");
                        if let Some(left) = self.release_slot(i, None) {
                            broadcasts.push_back((i, left));
                        }
                        if i >= self.num_local_players {
                            break;
                        }
                    }
                    protocol::PacketType::Ping => {
                        let pong = protocol::serialize_packet(protocol::PacketType::Pong, &());
                        self.transports[i].send_to_client(&pong);
                    }
                    _ => {}
                }
            }
            // B2a review HIGH-2 / FU1 — an input that arrived this tick but
            // waits past the budget still has its chunk acknowledgement and
            // drop reports taken now, so a backlog never holds the push's
            // window shut (the joiner needs chunks most right after a hitch).
            // Both are cumulative and `as_of`-stamped: taken again in order
            // when the input is processed, they change nothing. Its render
            // distance waits for its turn (a newer one read early would be
            // undone by the older inputs still queued), and so does all of an
            // input carrying a column-mismatch switch the server has not acted
            // on yet — and every input after it, which carries it too: in
            // order the switch comes before the drops (`take_chunk_feedback`),
            // and a drop taken first would hold the mismatched column off.
            let waiting_new = arrived.min(self.inbound[i].len());
            if waiting_new > 0 && self.handshake_done[i] && i >= self.num_local_players {
                let push = &mut self.chunk_pushes[i];
                for packet in self.inbound[i].newest(waiting_new) {
                    if let Some((protocol::PacketType::ClientInput, payload)) = protocol::deserialize_header(packet)
                        && let Ok(input) = protocol::safe_deserialize::<protocol::InputPacket>(payload)
                    {
                        if input.column_mismatch.is_some() && !push.pushes_everything() {
                            break;
                        }
                        push.ack(input.chunk_ack);
                        push.drop_columns(&input.chunk_drops);
                    }
                }
            }
        }

        for (source_idx, pkt) in broadcasts {
            for j in 0..self.transports.len() {
                if j == source_idx {
                    continue;
                }
                if self.handshake_done[j] && !self.disconnected[j] {
                    self.transports[j].send_to_client(&pkt);
                }
            }
        }
        closed_before_fill
    }

    /// FU3 — process the edits slot `i` has waiting past earlier ticks'
    /// budgets, oldest first, while this tick's edit budget lasts. They go
    /// before anything the client sent this tick.
    fn process_waiting_edits(&mut self, i: usize, budget: &mut EditTickBudget) {
        while budget.edits < MAX_BLOCK_CHANGES_PER_TICK {
            let Some(mut group) = self.edit_queues[i].pop_front() else {
                break;
            };
            self.process_edit_group(i, &mut group, budget);
            if !group.is_empty() {
                self.edit_queues[i].push_front(group);
                break;
            }
        }
    }

    /// FU3 — one input's edits (`edits`, its `tags` and the hand it
    /// reported): processed now, in order, while the tick's edit budget
    /// lasts, if nothing of this client's waits ahead of them; the rest (all
    /// of them, if something waits) queued behind, to go first next tick.
    /// Nothing is sent back for the budget. FU4a (FU3 verify M1, M2) — past
    /// the queue's hard cap ([`crate::edit_queue::MAX_DEFERRED_EDITS`], which
    /// no honest client reaches) an edit is dropped before it is even paired
    /// with its tag, with nothing sent back (a volume refusal is no honest
    /// edit to undo, and sending each back multiplied a flood's cost on the
    /// host), and one warning is logged until the queue next empties.
    fn process_or_queue_edits(
        &mut self,
        i: usize,
        edits: Vec<protocol::BlockChange>,
        tags: &[protocol::MinedBlock],
        held: (u8, u16),
        budget: &mut EditTickBudget,
    ) {
        let waiting = !self.edit_queues[i].is_empty();
        let keep = if waiting { self.edit_queues[i].room() } else { usize::MAX };
        let life = self.server.players.get(i).map_or(0, |sp| sp.respawns);
        let world = &self.server.world;
        let (mut group, mut dropped) =
            crate::edit_queue::EditGroup::new(edits, keep, tags, held, life, |x, y, z| world.get_block(x, y, z));
        if !waiting {
            self.process_edit_group(i, &mut group, budget);
        }
        dropped += self.edit_queues[i].push_back(group);
        if dropped > 0 && self.edit_queues[i].take_cap_warning() {
            log::warn!(
                "Player {i} has {} block edits waiting, at the cap ({}) — dropping what it sends \
                 past it (no honest client gets here)",
                self.edit_queues[i].len(),
                crate::edit_queue::MAX_DEFERRED_EDITS,
            );
        }
    }

    /// FU3 — process `group`'s edits in order while this tick's edit budget
    /// lasts; what the budget doesn't reach stays in `group`. A joiner the
    /// server holds dead edits nothing (MP-A3): its waiting edits are sent
    /// back, all at once — they were made while it was alive, but the body
    /// and inventory they would act on are gone. FU4a (FU3 verify L1) — so
    /// is a group made in an earlier life (`EditGroup::life`): a respawn was
    /// answered since. Within the per-tick send-back cap
    /// ([`MAX_SEND_BACKS_PER_CLIENT_PER_TICK`], M2); past it they are dropped.
    fn process_edit_group(
        &mut self,
        i: usize,
        group: &mut crate::edit_queue::EditGroup,
        budget: &mut EditTickBudget,
    ) {
        let earlier_life = self.server.players.get(i).is_some_and(|sp| sp.respawns != group.life);
        if self.joiner_is_dead(i) || earlier_life {
            for bc in group.take_edits() {
                self.send_back_authoritative_block(&bc, budget);
            }
            return;
        }
        while budget.edits < MAX_BLOCK_CHANGES_PER_TICK {
            let Some((bc, tag)) = group.pop_front() else {
                break;
            };
            budget.edits += 1;
            self.process_one_edit(i, &bc, tag, (group.held_kind, group.held_id), budget);
        }
    }

    /// Validate and apply one of slot `i`'s edits, with its own `mined` tag
    /// (FU4a, L2) and the hand its input reported. A refused edit is sent
    /// back (within the per-tick cap), so the sender's optimistic local edit
    /// is undone; its tag yields nothing.
    fn process_one_edit(
        &mut self,
        i: usize,
        bc: &protocol::BlockChange,
        tag: Option<protocol::MinedBlock>,
        hand: (u8, u16),
        budget: &mut EditTickBudget,
    ) {
        if let Err(why) = self.validate_block_edit(i, bc, hand) {
            log::debug!("Refused slot {i}'s edit at ({}, {}, {}): {why:?}", bc.x, bc.y, bc.z);
            // Un-ghost the refused edit on the sender: re-send what is
            // really there.
            self.send_back_authoritative_block(bc, budget);
            return;
        }
        let old_block =
            self.server.world.get_block(bc.x, bc.y, bc.z);
        // A container broken out from under the host
        // spills its contents instead of stranding them
        // as an orphan block entity (audit 2026-09-27).
        let remote = self.server.players[i].server_simulated;
        // C1 — what a server-simulated player's edit is to
        // its inventory (`joiner_inventory`): a break it
        // mined, a plain placement, or neither. A break's
        // yield is read now, before the block leaves the
        // world (`break_drops`).
        let joiner_edit = remote.then(|| self.classify_joiner_edit(bc, old_block, tag.as_ref(), hand));
        let break_yield = match joiner_edit {
            Some(crate::joiner_inventory::JoinerEdit::Break { tool }) => {
                self.joiner_break_yield(bc, old_block, tool)
            }
            _ => None,
        };
        self.spill_container_on_change(
            (bc.x, bc.y, bc.z),
            old_block,
            bc.new_block,
            remote,
        );
        // …and an economy block (vendor, tip jar, auction)
        // broken out from under its entity leaves no
        // orphan either — what a receiving client's
        // `World::apply_remote_block_change` does. On a
        // lent world this IS the host's entity.
        self.server.world.drop_orphaned_family_entity(
            (bc.x, bc.y, bc.z),
            old_block,
            bc.new_block,
        );
        // The validator only lets a plot marker's owner
        // break it — breaking it releases the claim.
        if old_block == crate::block::PLOT_MARKER
            && bc.new_block != crate::block::PLOT_MARKER
        {
            self.server.world.release_plot((bc.x, bc.y, bc.z));
        }
        self.server.world.set_block(bc.x, bc.y, bc.z, bc.new_block);
        // FU1 (C1 verify N1) — only when the block really
        // changed: an edit that leaves it as it was (a
        // meta-only toggle, or a modified client
        // "replacing" natural deepslate with itself) puts
        // nothing into the cell, so a natural cell stays
        // natural (its Satori roll stands).
        if remote && old_block != bc.new_block {
            // C1 (review MEDIUM-1) — every block a joiner
            // puts into a cell is player-placed, whatever
            // the edit was classified as (a plain
            // placement, a tool claimed in hand, a fill
            // carrying a `mined` tag, creative), as the
            // client's own placements always are: re-mining
            // it yields no Satori (Spec 06 §2.2). A break
            // the server yielded leaves the cell natural
            // again (AIR, or a harvested crop's tilled
            // soil), as single-player's break arm does.
            let placed = bc.new_block != crate::block::AIR && break_yield.is_none();
            self.server.world.set_placed(bc.x, bc.y, bc.z, placed);
        }
        if let Some(edit) = joiner_edit {
            self.settle_joiner_edit(i, bc, edit, break_yield);
        }
        // T1-3 — a log broken by a REMOTE player queues its
        // leaves for the server's leaf-decay pass (the
        // client break arms' `on_log_broken`, server side),
        // on EVERY host kind: a joiner's client runs no
        // decay of its own (that rolled a second set of
        // saplings), so the server is the only one who can.
        // Gated on `remote`, NOT `simulates_block_machines`:
        // a LAN host's own (local) breaks stay with the host
        // client's decay, so feeding them here too would
        // roll every sapling twice.
        if remote
            && crate::block::is_any_log_block(old_block)
            && !crate::block::is_any_log_block(bc.new_block)
        {
            self.server.leaf_decay.on_log_broken(
                bc.x,
                bc.y,
                bc.z,
                &self.server.world,
            );
        }
        // Keep both fluid systems' source bookkeeping in step
        // with the edit (placed water/lava becomes a source,
        // dug fluid drops its source, an opened gap wakes a
        // neighbour) — same as the client's inline handling.
        crate::fluids::notify_block_edit(
            &mut self.server.water,
            &mut self.server.lava,
            &self.server.world,
            bc.x,
            bc.y,
            bc.z,
            old_block,
            bc.new_block,
        );
        // The wire carries a metadata byte for exactly this
        // reason — facing, lever latch, gate op, rail state.
        // Dropping it left every directional block a joiner
        // placed facing north on the host, so a Logic Gate
        // drove the wrong cell and a Mirror bounced the wrong
        // way in the world the SERVER simulates.
        let cell = (bc.x, bc.y, bc.z);
        self.server.world.set_meta(cell, bc.meta);
        // Spec 48 (Electricity) — register (or drop) the
        // PowerDevice behind a joiner's placement, mirroring
        // the client's own place/break arms via the shared
        // kind table. Without it the host's authoritative
        // power sim saw a lever with nothing behind it —
        // nothing sourced, nothing lit, and a broken source
        // lived on as a ghost that powered the run forever.
        // Keyed on the KIND changing, so a lit↔unlit twin
        // swap (generator, lamp, wheel, mill) leaves the
        // device — and its fuel and charge — alone.
        let old_kind = crate::power::device_kind_for_block(old_block);
        let new_kind = crate::power::device_kind_for_block(bc.new_block);
        if old_kind != new_kind {
            self.server.world.block_entities.remove(&cell);
            if let Some(kind) = new_kind {
                self.server.world.insert_power_device(
                    cell,
                    crate::power::PowerDeviceData::new(
                        kind,
                        crate::meta::facing(bc.meta),
                    ),
                );
            }
        }
        // …and because that rebuild is keyed on the KIND,
        // a same-kind meta-only change — which is exactly
        // what a lever flip looks like on the wire, LEVER
        // to LEVER with the state bit moved — left the
        // SERVER's device latched the old way. The
        // authoritative sim then disagreed with the host
        // about every switch the host threw. Same fold as
        // the client apply path does.
        crate::power::sync_device_from_meta(
            &mut self.server.world,
            cell,
            bc.meta,
        );
        // Spec 48 §2.3 — EVERY edit nudges the six
        // neighbours: a plain block can still change a
        // network (block a Beam Sensor's beam, support a
        // Pressure Plate). Power cells seed themselves too.
        if crate::block::is_power_block(bc.new_block)
            || crate::block::is_power_block(old_block)
        {
            self.server.world.mark_dirty(cell);
        }
        self.server.world.notify_neighbours(cell);
        self.pending_block_changes.push(bc.clone());
        // D1 — a joiner's edit landed in the host's own
        // world: its client must remesh the cell.
        if self.lends_host_world() {
            self.lent_edit_cells.push(cell);
        }
        // FU3 (FU1 verify N3) — a joiner's campfire edit: the server runs the
        // campfire rule itself, so the joiner sends the one edit, not the
        // pillar behind it. Gated on `remote` like the container spill: a
        // host's own edits carry their smoke already.
        if remote {
            self.derive_campfire_edit(cell, old_block, bc.new_block);
        }
    }

    /// FU3 (FU1 verify N3) — what a joiner's accepted edit `old → new` at
    /// `cell` does to a campfire there (`campfire::on_block_edit`, the rule a
    /// client's own break and light arms run): a broken fire's smoke cleared
    /// and what was cooking spilled into the world, a smoky fire's pillar
    /// raised when it is lit, a pillar cleared when it goes out — from the
    /// SERVER's campfire state, and broadcast to everyone (the joiner itself
    /// included: it no longer sends or predicts the pillar).
    fn derive_campfire_edit(&mut self, cell: (i32, i32, i32), old: crate::block::BlockId, new: crate::block::BlockId) {
        let edit = crate::campfire::on_block_edit(&mut self.server.world, cell, old, new);
        let at = glam::Vec3::new(cell.0 as f32 + 0.5, cell.1 as f32 + 0.5, cell.2 as f32 + 0.5);
        for (k, stack) in edit.spill.into_iter().enumerate() {
            crate::entity::spawn_item(&mut self.server.ecs, at, stack, k as u32 * 6529);
        }
        for ((x, y, z), new_block) in edit.smoke {
            self.pending_block_changes.push(protocol::BlockChange { x, y, z, new_block, meta: 0 });
            if self.lends_host_world() {
                self.lent_edit_cells.push((x, y, z));
            }
        }
    }

    /// C1 — what slot `i`'s accepted edit `old → bc.new_block` is to its
    /// inventory (`joiner_inventory::classify`): a break if it is one with its
    /// own `mined` tag (`tag`), else a plain placement by what its input says
    /// is in hand, else unchecked.
    ///
    /// FU1 — a tag is used up by its own edit (C1 verify N4): the break it
    /// yields, or an emptying of the cell that yields nothing (dug lava or
    /// fire). One tag, one edit, so it can never yield for another edit of the
    /// same cell (a later Eraser or bucket there, a re-mine), and the client
    /// sends one per mined edit, beside that edit (`RemoteClient::send_input`).
    /// FU4a (FU3 verify L2) — the pairing is made once, as the input is read
    /// (`edit_queue::EditGroup::new`), so the edit a tag was paired with
    /// spends it whatever happens to it (classified here, refused, dropped at
    /// the cap); it is never "the cell's oldest tag" at processing time. A
    /// fill is never paired, so a place-then-break of one cell in one input
    /// consumes the placement and yields the break; an edit whose tag doesn't
    /// make it a break (the server's copy of the cell disagreed) is classified
    /// as untagged.
    fn classify_joiner_edit(
        &self,
        bc: &protocol::BlockChange,
        old: crate::block::BlockId,
        tag: Option<&protocol::MinedBlock>,
        (held_kind, held_id): (u8, u16),
    ) -> crate::joiner_inventory::JoinerEdit {
        use crate::joiner_inventory::{classify, JoinerEdit};
        // BRIDGE: possession check — the hand (and a break's tool) is the
        // client's word until the shadow can be enforced (see
        // `validate_block_edit`). FU3 — the hand of the edit's own input
        // (`edit_queue::EditGroup`), not of a later one read since.
        let hand = crate::joiner_inventory::Hand::from_wire(held_kind, held_id, &self.server.registry);
        let creative = self.server.play_mode.is_creative();
        if let Some(tag) = tag {
            let edit = classify(old, bc.new_block, Some(tag), hand, creative);
            if matches!(edit, JoinerEdit::Break { .. }) {
                return edit;
            }
        }
        classify(old, bc.new_block, None, hand, creative)
    }

    /// C1 — the yield of a joiner's break of `old` at `bc` with `tool`: the
    /// rules single-player's break arm runs (`break_drops::break_yield`), on
    /// this server's world, clock and Proof-of-Play secret — so a rare drop is
    /// the WORLD's roll, never the joiner's. `None` when the edit doesn't
    /// leave what that break would (the joiner's copy of the cell disagreed
    /// with the server's: no yield, counted unchecked).
    fn joiner_break_yield(
        &self,
        bc: &protocol::BlockChange,
        old: crate::block::BlockId,
        tool: Option<crate::crafting::Tool>,
    ) -> Option<crate::break_drops::BreakYield> {
        let y = crate::break_drops::break_yield(
            &self.server.world,
            &self.server.registry,
            old,
            (bc.x, bc.y, bc.z),
            tool.as_ref(),
            self.server.tick_counter,
            &self.server.pop_keys(),
        );
        (bc.new_block == crate::block::AIR || bc.new_block == y.replacement).then_some(y)
    }

    /// C1 — slot `i`'s accepted edit is in the world (its placed flag set:
    /// every fill player-placed, a yielded break natural again); settle it
    /// with the server's shadow of its inventory (`joiner_inventory`). A
    /// break exposes its deepslate neighbours (what the client's break arm
    /// does), and the yield goes to the shadow and to the client by
    /// `InventoryGrant`. A plain placement: the log-only possession
    /// check, consuming one from the shadow's held slot on a match. Anything
    /// else is counted unchecked.
    fn settle_joiner_edit(
        &mut self,
        i: usize,
        bc: &protocol::BlockChange,
        edit: crate::joiner_inventory::JoinerEdit,
        break_yield: Option<crate::break_drops::BreakYield>,
    ) {
        use crate::joiner_inventory::{JoinerEdit, PlaceCheck};
        let tick = self.server.tick_counter;
        match (edit, break_yield) {
            (JoinerEdit::Break { .. }, Some(y)) => {
                let world = &mut self.server.world;
                crate::break_drops::mark_exposed_neighbours(world, bc.x, bc.y, bc.z, tick);
                self.server.players[i].possession.breaks += 1;
                self.grant_to_joiner(i, y.drops.into_iter().chain(y.gem));
            }
            (JoinerEdit::Place, _) => {
                let sp = &mut self.server.players[i];
                let slot = sp.hotbar_slot;
                match crate::joiner_inventory::check_placement(&mut sp.inventory, slot, bc.new_block) {
                    PlaceCheck::Matched => sp.possession.matched += 1,
                    PlaceCheck::Mismatched { held } => {
                        let due = sp.possession.note_mismatch(tick);
                        let registry = &self.server.registry;
                        let held = held.map_or("nothing placeable", |b| registry.get(b).name);
                        log::log!(
                            crate::joiner_inventory::mismatch_log_level(due),
                            "possession check (log-only): {} placed {} from hotbar slot {slot}; \
                             the server's copy of that slot holds {held}{} — accepted",
                            sp.display_name,
                            registry.get(bc.new_block).name,
                            held_back_note(due),
                        );
                    }
                }
            }
            _ => self.server.players[i].possession.unchecked += 1,
        }
    }

    /// C1 — what the server gives joiner `i` (a break's yield, an
    /// interaction's products): into its shadow of the joiner's inventory, and
    /// to the joiner by `InventoryGrant`. C2b-fix (verify M1) — the client is
    /// always sent the WHOLE stack, whatever fits the shadow: the shadow fills
    /// by drift in ordinary play (container deposits, worn-out tools, armour
    /// put on, consumes), so a "full" shadow must not cost the joiner an item.
    /// What the shadow can't hold is tallied as `grant_overflow`, never
    /// spilled. (The client's own spill when ITS inventory is full —
    /// `remote_entities::apply_inventory_grant` — is the client's, not a
    /// duplicate.) Plans have no wire form and are never granted.
    // BRIDGE: spill the shadow's overflow as a real item once C3d makes the
    // server inventory the truth — replace when C3d lands.
    pub(crate) fn grant_to_joiner(&mut self, i: usize, stacks: impl IntoIterator<Item = crate::item::ItemStack>) {
        let Some(sp) = self.server.players.get_mut(i) else { return };
        let mut grants: Vec<(usize, crate::item::ItemStack)> = Vec::new();
        for stack in stacks {
            if stack.count == 0 || matches!(stack.item, crate::item::Item::Plan(_)) {
                continue;
            }
            if let Some(rest) = sp.inventory.add_item(stack.clone()) {
                sp.possession.grant_overflow =
                    sp.possession.grant_overflow.saturating_add(u32::from(rest.count));
            }
            grants.push((i, stack));
        }
        for (slot, pkt) in build_grant_packets(&grants) {
            self.send_to_joined_slot(slot, &pkt);
        }
    }

    /// The host's authority over a block edit from slot `i` (audit 2026-09-27,
    /// "Host applies joiner-supplied block edits … with no mode, inventory,
    /// ownership or play-mode check"). Every client edit passes here before it
    /// touches the world:
    ///
    /// - well-formed: a registry-known block, never bedrock, inside the world's
    ///   height, in a loaded column, not replacing bedrock;
    /// - within reach of the position the SERVER holds for the player
    ///   ([`block_change_within_reach`]);
    ///
    /// and, for a remote (server-simulated) player — local seats are gated by
    /// the host's own client, which runs these same rules —
    ///
    /// - the world's play mode allows editing (Adventure / Spectator don't; the
    ///   server keeps one mode per world, which is the joiner's mode);
    /// - an economy block (vendor, tip jar, auction, plot marker, market bell)
    ///   may only be touched by the npub that owns it ([`economy_owner_allows`]);
    /// - the cell is not inside a plot the joiner doesn't own (Spec 36's rule —
    ///   creative bypasses — with a remote joiner owning a plot only through a
    ///   matching `PlotOwner::Npub`).
    ///
    /// BRIDGE: possession check — a place of an item the joiner doesn't hold
    /// is NOT refused. Since C1 the server keeps a SHADOW of a remote
    /// player's inventory (`ServerPlayer.inventory`, `joiner_inventory`) and
    /// checks each plain block placement against its held slot, but LOG-ONLY
    /// (`PossessionTally`): the shadow doesn't yet see chests, slot moves or
    /// what the joiner arrived with (C2b mirrors crafting), so refusing would refuse
    /// legitimate placements. Make it refuse (and send the block back) when
    /// the remaining gains reach the server (inventory authority merges 2-3,
    /// the `ServerPlayer` vs `PlayerSlot` debt in CLAUDE.md).
    fn validate_block_edit(
        &self,
        i: usize,
        bc: &protocol::BlockChange,
        (held_kind, held_id): (u8, u16),
    ) -> Result<(), EditRefusal> {
        let Some(sp) = self.server.players.get(i) else {
            return Err(EditRefusal::Malformed);
        };
        if !self.server.registry.is_known(bc.new_block) || bc.new_block == crate::block::BEDROCK {
            return Err(EditRefusal::Malformed);
        }
        let cs = crate::chunk::CHUNK_SIZE as i32;
        if bc.y < 0 || bc.y >= 6 * cs {
            return Err(EditRefusal::OutOfWorld);
        }
        if !self
            .server
            .loaded_columns
            .contains(&(bc.x.div_euclid(cs), bc.z.div_euclid(cs)))
        {
            return Err(EditRefusal::Unloaded);
        }
        let current = self.server.world.get_block(bc.x, bc.y, bc.z);
        if current == crate::block::BEDROCK {
            return Err(EditRefusal::Protected);
        }
        let eye = sp.player.eye_pos();
        let dx = bc.x as f32 + 0.5 - eye.x;
        let dy = bc.y as f32 + 0.5 - eye.y;
        let dz = bc.z as f32 + 0.5 - eye.z;
        if !block_change_within_reach(dx * dx + dy * dy + dz * dz, held_kind, held_id, sp.server_simulated) {
            return Err(EditRefusal::Reach);
        }
        if sp.server_simulated {
            self.remote_may_touch(sp, bc.x, bc.z)?;
            let npub = verified_npub(sp.verified_pubkey);
            let cell = (bc.x, bc.y, bc.z);
            if !economy_owner_allows(&self.server.world, cell, current, npub.as_deref()) {
                return Err(EditRefusal::EconomyOwner);
            }
        }
        Ok(())
    }

    /// The play-mode and plot gates a REMOTE player's world touch passes —
    /// shared by block edits and `DeviceInteract` (review S3).
    fn remote_may_touch(
        &self,
        sp: &crate::server::ServerPlayer,
        x: i32,
        z: i32,
    ) -> Result<(), EditRefusal> {
        if !self.server.play_mode.can_edit_world() {
            return Err(EditRefusal::PlayMode);
        }
        let npub = verified_npub(sp.verified_pubkey);
        if !self.server.play_mode.is_creative()
            && crate::plot::is_in_foreign_plot_for_npub(&self.server.world.plots, x, z, npub.as_deref())
        {
            return Err(EditRefusal::ForeignPlot);
        }
        Ok(())
    }

    /// Mirror the host client's live block-entity state into the server world
    /// (review B1/S1) — on an OWNING host (`--no-lend`) only. The host's client
    /// is where chests are filled, furnaces fed, vendors stocked and plots
    /// claimed; an owning server's copy would otherwise be frozen at world
    /// load, so a joiner's break would spill stale contents (duplication) while
    /// the host's `World::apply_remote_block_change` dropped the live chest
    /// unspilled (loss), and the plot / economy gates would miss anything
    /// claimed or placed since. Called once a tick, just before
    /// [`Self::tick`], from the one place the host feeds its hosted server
    /// (`GameState::tick_hosted_server`). A lending host (D1) has one world
    /// and nothing to mirror.
    ///
    /// An entity is copied only where the server's block agrees on its family,
    /// so a container a joiner just broke (block already gone here, entity not
    /// yet cleared on the host) is never resurrected; entities the host no
    /// longer has are dropped. Plots and market hubs are copied whole.
    ///
    /// BRIDGE: host-client → server mirroring for the `--no-lend` escape hatch
    /// — delete with `--no-lend` (one release after D1), when every host lends.
    pub fn mirror_host_world_state(&mut self, host: &crate::world::World) {
        // A lent world IS the host's: mirroring it into itself is a no-op at
        // best, and outside the window it would write into the server's empty
        // placeholder world.
        debug_assert!(!self.lends_host_world(), "mirror_host_world_state on a lending host");
        // T1-3 — the mirror exists because a host CLIENT owns the machines;
        // a server that ticks them itself (the dedicated server) has no host
        // client to mirror, and mirroring would overwrite its own sim.
        debug_assert!(
            !self.server.simulates_block_machines,
            "mirror_host_world_state on a server that ticks its own block machines"
        );
        use crate::world::BlockEntityData as E;
        let server = &mut self.server.world;
        for (&pos, data) in &host.block_entities {
            let Some(family) = data.mirrored_family() else {
                continue;
            };
            if crate::world::mirrored_family(server.get_block(pos.0, pos.1, pos.2)) != Some(family) {
                continue;
            }
            match (server.block_entities.get_mut(&pos), data) {
                // Cheap equality where the types have it (a chest is the
                // common, big one); everything else is small, so copy.
                (Some(E::Chest(a)), E::Chest(b)) if a == b => {}
                (Some(E::TipJar(a)), E::TipJar(b)) if a == b => {}
                (Some(existing), _) => existing.clone_from(data),
                (None, _) => {
                    server.block_entities.insert(pos, data.clone());
                }
            }
        }
        server.block_entities.retain(|pos, d| {
            d.mirrored_family().is_none()
                || host.block_entities.get(pos).and_then(E::mirrored_family) == d.mirrored_family()
        });
        if server.plots != host.plots {
            server.plots.clone_from(&host.plots);
        }
        if server.market_hubs != host.market_hubs {
            server.market_hubs.clone_from(&host.market_hubs);
        }
    }

    /// Queue the block that is REALLY at a refused edit's cell onto the next
    /// StateUpdate, so the sender's optimistic local edit is overwritten. Rides
    /// the ordinary block-change broadcast (idempotent for everyone else).
    /// Skipped for cells outside the world or in an unloaded column, where the
    /// server has nothing authoritative to say. FU4a (FU3 verify M2) — and
    /// past the client's [`MAX_SEND_BACKS_PER_CLIENT_PER_TICK`] this tick
    /// (`budget`): dropped silently.
    fn send_back_authoritative_block(&mut self, bc: &protocol::BlockChange, budget: &mut EditTickBudget) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        if budget.send_backs >= MAX_SEND_BACKS_PER_CLIENT_PER_TICK
            || bc.y < 0
            || bc.y >= 6 * cs
            || !self
                .server
                .loaded_columns
                .contains(&(bc.x.div_euclid(cs), bc.z.div_euclid(cs)))
        {
            return;
        }
        budget.send_backs += 1;
        self.pending_block_changes.push(protocol::BlockChange {
            x: bc.x,
            y: bc.y,
            z: bc.z,
            new_block: self.server.world.get_block(bc.x, bc.y, bc.z),
            meta: self.server.world.meta_at(bc.x, bc.y, bc.z),
        });
    }

    /// When an accepted edit replaces a container (chest of any tier, furnace,
    /// dispenser/dropper, grave) with anything that isn't the same container,
    /// empty the server's copy — the same `cleanup_*` hooks the client's own
    /// break arm runs. A lit↔unlit furnace swap or a chest-tier change keeps
    /// its contents.
    ///
    /// Exactly one spill per container break (review B1): only a REMOTE
    /// joiner's break (`spill = true`) drops the contents here. On a lent
    /// world (D1) that is the host's own, live container, spilled into the
    /// host's own ECS; the host's own breaks never reach this (its client
    /// already spilled them). On an owning server (`--no-lend`, dedicated)
    /// the host's breaks discard the server's copy, and a host client clears
    /// its entity for a joiner's break without spilling
    /// (`World::apply_remote_block_change`); an owning LAN host's copy is
    /// kept live by [`Self::mirror_host_world_state`], so a joiner's break
    /// there spills what the host's chest holds now, not what it held at load.
    fn spill_container_on_change(
        &mut self,
        cell: (i32, i32, i32),
        old_block: crate::block::BlockId,
        new_block: crate::block::BlockId,
        spill: bool,
    ) {
        let Some(family) = crate::world::mirrored_family(old_block).filter(|f| f.is_container())
        else {
            return;
        };
        if crate::world::mirrored_family(new_block) == Some(family) {
            return;
        }
        let (x, y, z) = cell;
        let world = &mut self.server.world;
        use crate::world::MirroredFamily as F;
        let contents = match family {
            F::Chest => crate::chest::cleanup_chest(world, x, y, z),
            F::Furnace => crate::furnace::cleanup_furnace(world, x, y, z),
            F::Dispenser => crate::dispenser::cleanup_dispenser(world, x, y, z),
            F::Grave => crate::grave::cleanup_grave(world, x, y, z),
            _ => Vec::new(),
        };
        if !spill {
            return;
        }
        let at = glam::Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
        for (k, stack) in contents.into_iter().enumerate() {
            crate::entity::spawn_item(&mut self.server.ecs, at, stack, k as u32 * 7919);
        }
    }

    /// `DeviceInteract` dispatch — a joined player right-clicked a power device
    /// (Wind/Copper/Electricity Task 2b).
    ///
    /// The host is the authority. It re-derives everything: the packet names a
    /// cell, and nothing else, so a client can ask for a lever to be touched but
    /// never for an outcome. Three gates, all with the same silent-drop posture
    /// as a block change (a rejected packet gets no reply — a "no" is
    /// information a prober would rather have):
    ///
    /// 1. the sender is a joined player with a slot;
    /// 2. the cell is inside the SAME reach envelope block changes use
    ///    ([`block_change_within_reach`], measured from the position the SERVER
    ///    holds for them, which for a server-simulated joiner is not a position
    ///    they can lie about);
    /// 3. a toggle-class `PowerDevice` actually stands there
    ///    ([`crate::power::is_toggle_class`]).
    ///
    /// The interaction itself is `power::interact_device` — literally the same
    /// function the single-player client calls, so the two cannot drift. Its
    /// changes join `pending_block_changes`, which is what puts the flipped
    /// lever, and next tick's lit cable and lamp, on every client including the
    /// one that asked.
    fn handle_device_interact(&mut self, sender: usize, pos: (i32, i32, i32)) {
        let Some(sp) = self.server.players.get(sender) else {
            return;
        };
        let player_pos = sp.player.eye_pos();
        if sp.server_simulated && self.remote_may_touch(sp, pos.0, pos.2).is_err() {
            return;
        }
        let (held_kind, held_id, server_simulated) =
            (sp.held_kind, sp.held_id, sp.server_simulated);
        let dx = pos.0 as f32 + 0.5 - player_pos.x;
        let dy = pos.1 as f32 + 0.5 - player_pos.y;
        let dz = pos.2 as f32 + 0.5 - player_pos.z;
        if !block_change_within_reach(
            dx * dx + dy * dy + dz * dz,
            held_kind,
            held_id,
            server_simulated,
        ) {
            return;
        }
        let now = self.server.tick_counter;
        if let Some(outcome) = crate::power::interact_device(&mut self.server.world, pos, now) {
            // D1 — on a lent world the flip happened in the host's own world.
            if self.lends_host_world() {
                self.lent_edit_cells
                    .extend(outcome.changes.iter().map(|bc| (bc.x, bc.y, bc.z)));
            }
            self.pending_block_changes.extend(outcome.changes);
        }
    }

    fn broadcast_state(&mut self) {
        // D1 — on a lent world these are changes the server's systems made
        // to the HOST's world: keep a copy for the host client to remesh
        // (`take_lent_changes`) before the broadcast drains them. Under
        // lending there is no loopback re-apply that could notice them.
        if self.lends_host_world() {
            self.lent_sim_changes
                .extend_from_slice(&self.server.pending_block_changes);
        }
        // Drain server-produced block changes (falling blocks, etc.) into
        // the outbound queue; they ride the same StateUpdate as block
        // changes we accepted from clients above.
        let server_changes = self.server.pending_block_changes.drain(..);
        self.pending_block_changes.extend(server_changes);

        let player_states: Vec<protocol::PlayerState> = self
            .server
            .players
            .iter()
            .enumerate()
            .filter(|(idx, _)| !self.disconnected.get(*idx).copied().unwrap_or(true))
            .map(|(idx, sp)| crate::server::collect_player_state(sp, idx as u32))
            .collect();

        let entity_tick = self.entity_broadcast.diff(&mut self.server.ecs, self.server.tick_counter);

        // Deepslate Reserve snapshot (Spec 16). Synthetic value in alpha
        // pending Sentinel D-003 reversal — see `reserve::ReserveState::synthetic_default`.
        let reserve = crate::reserve::ReserveState::synthetic_default();

        // P9 weather sync (v59) — the server's OWN weather window (advanced
        // once per tick in `GameServer::tick`), as ticks-remaining so the
        // sync is correct regardless of any tick_counter offset between the
        // server and this client. A host client hands its own window in
        // (`sim_lend::HostClock` when lent; `game_loop::tick_hosted_server`
        // translates it on an owning host) before `HostedServer::tick()`
        // runs, so a hosting player's own rain window IS this broadcast.
        let (rain_ticks_left, storm_ticks_left) =
            self.server.weather.ticks_left(self.server.tick_counter);

        // The tick's snapshot fields, repeated on every StateUpdate a client
        // gets this tick. The deltas (block changes, entity events) go through
        // each client's outbox instead (gap-audit T1-5): one tick's worth no
        // longer has to fit in one packet.
        let mut template = protocol::StateUpdatePacket {
            tick: self.server_tick,
            players: player_states,
            block_changes: Vec::new(),
            world_time: self.server.world_time,
            // Per client — set in the loop below.
            last_acked_input: 0,
            entity_spawns: Vec::new(),
            entity_updates: Vec::new(),
            entity_despawns: Vec::new(),
            reserve_richness: reserve.richness,
            reserve_target_sats: reserve.target_sats,
            reserve_current_sats: reserve.current_sats,
            rain_ticks_left,
            storm_ticks_left,
            // Per client — stamped in the loop below (C2a).
            own_hunger: 0,
        };
        let block_changes = std::mem::take(&mut self.pending_block_changes);

        // B2a — the chunk push reads the world here, inside `tick` (on a
        // lending host: inside the lend window, the host client's own world).
        debug_assert_eq!(
            self.lends_host_world(),
            self.server.lent,
            "the chunk push reads a lent world only inside the lend window"
        );
        let push_limit = self.chunk_push_limit();
        // B2b — the world's edits since last tick touch their columns for
        // good (the changes broadcast now included: their columns were marked
        // as the writes landed); then this tick's verdicts, nearest first.
        self.server.world.track_edited_columns();
        for col in self.server.world.take_edited_columns() {
            self.verdicts.touch(col);
        }
        for b in &block_changes {
            let (cx, _, cz) = crate::state_outbox::chunk_of(b);
            self.verdicts.touch((cx, cz));
        }
        self.decide_verdicts(push_limit);
        for i in 0..self.transports.len() {
            if !self.handshake_done[i] || self.disconnected[i] {
                continue;
            }
            // MP-D2a — this client's share of the entity events: a joiner
            // hears about entities near its body, changed-only (a late
            // joiner's empty interest set doubles as its backfill).
            let anchor = self.entity_interest_anchor(i);
            let entities = self.entity_interest[i].events(&entity_tick, &self.server.ecs, anchor);
            // B2a — a remote client hears of changes only to chunks it has
            // been sent (queued counts): the rest arrive inside their push.
            let filtered: Vec<protocol::BlockChange>;
            let changes: &[protocol::BlockChange] = if self.filters_changes(i) {
                let push = &self.chunk_pushes[i];
                filtered = block_changes
                    .iter()
                    .filter(|b| push.has_sent(crate::state_outbox::chunk_of(b)))
                    .cloned()
                    .collect();
                &filtered
            } else {
                &block_changes
            };
            let push_centre = self.push_centre(i);
            let verdicts = self.sends_notes(i).then_some(&self.verdicts);
            let outbox = &mut self.outboxes[i];
            outbox.push_tick(
                self.server_tick,
                &entities.spawns,
                &entities.despawns,
                changes,
                &entities.updates,
            );
            // B2a — then this tick's chunk pushes, AFTER its deltas: a change
            // to a chunk queued now was filtered above and is already in the
            // snapshot, and the tick's deltas never wait behind new chunks.
            if let Some(centre) = push_centre {
                crate::chunk_push::queue_pushes(
                    &mut self.chunk_pushes[i],
                    outbox,
                    &self.server.world,
                    &self.server.loaded_columns,
                    centre,
                    push_limit,
                    verdicts,
                );
            }
            // The last input of THIS client's that its server state includes
            // — its prediction drops those and replays the rest (§5.3).
            template.last_acked_input = self
                .server
                .players
                .get(i)
                .map_or(0, |sp| sp.last_applied_input);
            // C2a — and its own hunger, which the server runs for a joiner
            // (a local slot's is its own client's: 0, never read).
            template.own_hunger = self
                .server
                .players
                .get(i)
                .filter(|sp| sp.server_simulated)
                .map_or(0, |sp| sp.combat.hunger);
            for pkt in outbox.drain_packets(&template) {
                self.transports[i].send_to_client(&pkt);
            }
        }
    }

    /// B2a — what a remote client's input tells its chunk push: its
    /// cumulative acknowledgement, the columns it let go of, and its current
    /// render distance (`0` = unchanged). A local slot has no push.
    fn take_chunk_feedback(&mut self, i: usize, input: &protocol::InputPacket) {
        if i < self.num_local_players {
            return;
        }
        // B2b — a column the joiner was told is local did not generate as
        // ours did. Before the drops: the column it let go of rides in this
        // same input, and taken out here first it is pushed again at once,
        // never held off.
        if let Some(m) = input.column_mismatch {
            self.column_mismatch(i, m);
        }
        let Some(push) = self.chunk_pushes.get_mut(i) else { return };
        push.set_render_distance(input.render_distance);
        push.ack(input.chunk_ack);
        push.drop_columns(&input.chunk_drops);
    }

    /// B2b — slot `i` reports that its generation of a column it was told is
    /// local does not hash as our note said (`InputPacket::column_mismatch`,
    /// repeated in every input; only the first does anything). A determinism
    /// bug with matching worldgen fingerprints — a platform floating-point
    /// difference in generating the column on its own. (Not an order
    /// dependence: the joiner confirms a mismatch on a scratch generation of
    /// the column alone, as the server's note was made, so a generation that
    /// depends on its neighbours reads there as the joiner's own drift and is
    /// kept, never reported — FU1, B2b fix2-verify N1.) Logged loudly, and that joiner is
    /// pushed everything for the rest of its session — every column it was
    /// noted is pushed again, since all of them are suspect, and it is noted
    /// no more (`sends_notes`). Ignored, silently, from a joiner never sent a
    /// note this session (B2b fix LOW-4: an honest client sets it only from a
    /// note, so it is a hostile one, and its report would be a misleading
    /// warning).
    fn column_mismatch(&mut self, i: usize, m: protocol::ColumnMismatch) {
        let Some(push) = self.chunk_pushes.get_mut(i) else { return };
        if push.pushes_everything() || !push.noted_ever() {
            return;
        }
        let repushed = push.push_everything_from_now();
        let who = self.server.players.get(i).map_or_else(
            || format!("slot {i}"),
            |sp| {
                // Remote players exist only on native builds.
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(pk) = &sp.verified_pubkey {
                    return format!("slot {i} ({}, {})", sp.display_name, pubkey_to_npub(pk));
                }
                format!("slot {i} ({})", sp.display_name)
            },
        );
        #[cfg(test)]
        {
            self.column_mismatch_warnings += 1;
        }
        log::warn!(
            "Terrain generation differs on joiner {who}: column ({}, {}) hashes {:#010x} there, {:#010x} here, \
             with the same worldgen fingerprint {:#010x} — a determinism bug, please report it. \
             Pushing that joiner every column from now on ({repushed} it generated itself will be pushed again).",
            m.cx,
            m.cz,
            m.client_hash,
            m.server_hash,
            crate::world::worldgen_fingerprint(),
        );
    }

    /// Does slot `i`'s block-change stream go through its sent-set? For every
    /// remote slot: it hears of changes only to chunks it has been pushed or
    /// told are local (B2b: a note puts the whole column in the sent-set).
    /// A local slot shares the host's world.
    fn filters_changes(&self, i: usize) -> bool {
        #[cfg(test)]
        if self.chunk_push_off {
            return false;
        }
        i >= self.num_local_players
    }

    /// B2b — does slot `i` get verdicts (a push for a touched column, a
    /// "local" note for the rest) rather than every chunk? A remote joiner
    /// whose terrain generator matches ours, under `--chunk-sync touched`, on
    /// a server that keeps every column round a joiner loaded out to its push
    /// limit: the dedicated server (it streams round each player) or a
    /// lending host (its client's streamer anchors on each joiner). An owning
    /// (`--no-lend`) host loads only round where hosting began and each
    /// joiner's 3×3, so a column in range might never get a verdict and its
    /// joiner, waiting for one, would show a hole: it pushes everything, as
    /// in B2a. Nor a joiner whose generation of a noted column did not match
    /// ours (`column_mismatch`): pushed everything for the rest of its session.
    fn sends_notes(&self, i: usize) -> bool {
        i >= self.num_local_players
            && (self.lends_host_world() || self.server.column_streamer.is_some())
            && !self.chunk_pushes.get(i).is_some_and(crate::chunk_push::ClientChunkPush::pushes_everything)
            && self
                .server
                .players
                .get(i)
                .is_some_and(|sp| !self.chunk_sync.pushes_everything(sp.worldgen_mismatch()))
    }

    /// B2b — this tick's verdicts (`chunk_verdict`): the undecided columns
    /// within every noted joiner's push radius, nearest first across all of
    /// them, up to the budget. Ties go to the joiners in turn (the start
    /// rotates with the tick), so none waits behind another's ring.
    fn decide_verdicts(&mut self, push_limit: i32) {
        let budget = self.verdict_budget;
        let joiners: Vec<usize> = (0..self.transports.len())
            .filter(|&i| self.handshake_done[i] && !self.disconnected[i] && self.sends_notes(i))
            .collect();
        let mut candidates: Vec<(i64, usize, (i32, i32))> = Vec::new();
        let n = joiners.len().max(1);
        for (k, &i) in joiners.iter().enumerate() {
            let Some(centre) = self.push_centre(i) else { continue };
            let turn = (k + self.server_tick as usize) % n;
            candidates.extend(
                self.chunk_pushes[i]
                    .verdict_candidates(
                        &self.server.loaded_columns,
                        centre,
                        push_limit,
                        &self.verdicts,
                        budget.count,
                    )
                    .into_iter()
                    .map(|(d, col)| (d, turn, col)),
            );
        }
        candidates.sort_unstable();
        let cols: Vec<(i32, i32)> = candidates.into_iter().map(|(_, _, col)| col).collect();
        let decided =
            self.verdicts.decide(&self.server.world, &self.server.biome_gen, &cols, budget);
        #[cfg(test)]
        {
            self.verdicts_last_tick = decided;
        }
        #[cfg(not(test))]
        let _ = decided;
    }

    /// How far round a joiner this server can push (`chunk_push`): as far as
    /// it keeps columns loaded round one — the dedicated server's sim
    /// distance, a host's joiner anchor (`LENT_JOINER_SIM_DISTANCE`). Only
    /// loaded columns are ever pushed, so an owning host (which loads less)
    /// simply pushes what it has.
    fn chunk_push_limit(&self) -> i32 {
        self.server.column_streamer.as_ref().map_or(
            crate::chunk_stream::LENT_JOINER_SIM_DISTANCE,
            crate::server_stream::ColumnStreamer::sim_distance,
        )
    }

    /// B2a — the column round which slot `i` is pushed chunks (its server
    /// body's), or `None` when it is pushed nothing: a local slot, a slot not
    /// (or no longer) joined, or a body with no finite position. Every mode
    /// pushes (B2b: `touched` pushes only touched columns and notes the rest).
    fn push_centre(&self, i: usize) -> Option<(i32, i32)> {
        #[cfg(test)]
        if self.chunk_push_off {
            return None;
        }
        if i < self.num_local_players || !self.handshake_done[i] || self.disconnected[i] {
            return None;
        }
        let pos = self.server.players.get(i)?.player.pos;
        pos.is_finite().then(|| crate::chunk_stream::column_of(pos))
    }

    /// `--chunk-sync` for this server (the dedicated server's flag).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_chunk_sync(&mut self, mode: crate::chunk_push::ChunkSync) {
        self.chunk_sync = mode;
    }

    /// Test-only: the pre-B2a delivery — no chunk pushes, every block change
    /// to every client, outbox overflows left for
    /// [`Self::take_chunk_resync_requests`] — for the tests that pin the
    /// outbox on its own.
    #[cfg(test)]
    pub(crate) fn without_chunk_push_for_test(&mut self) {
        self.chunk_push_off = true;
    }

    /// Test-only: slot `slot`'s joiner already holds every chunk of column
    /// `col` (see `ClientChunkPush::hold_for_test`) — for a test that moves
    /// a joiner's body somewhere it never walked.
    #[cfg(test)]
    pub(crate) fn hold_column_for_test(&mut self, slot: usize, col: (i32, i32)) {
        for cy in 0..=crate::world::MAX_CHUNK_Y {
            self.chunk_pushes[slot].hold_for_test((col.0, cy, col.1));
        }
    }

    /// Test-only: slot `slot`'s chunk-push state.
    #[cfg(test)]
    pub(crate) fn chunk_push_for_test(&self, slot: usize) -> &crate::chunk_push::ClientChunkPush {
        &self.chunk_pushes[slot]
    }

    /// Test-only (B2b): how many column-mismatch warnings were logged.
    #[cfg(test)]
    pub(crate) fn column_mismatch_warnings_for_test(&self) -> usize {
        self.column_mismatch_warnings
    }

    /// Test-only (B2b): slot `slot`'s notes carry a wrong column hash (a
    /// forged mismatch).
    #[cfg(test)]
    pub(crate) fn forge_note_hashes_for_test(&mut self, slot: usize) {
        self.chunk_pushes[slot].forge_note_hashes = true;
    }

    /// Test-only (B2b): push chunk `c` to slot `slot` again, as an outbox
    /// overflow would (`ClientChunkPush::request_resync`).
    #[cfg(test)]
    pub(crate) fn resync_for_test(&mut self, slot: usize, c: crate::state_outbox::ChunkCoord) {
        self.chunk_pushes[slot].request_resync([c]);
    }

    /// Test-only (B2b): the shared verdict cache.
    #[cfg(test)]
    pub(crate) fn verdicts_for_test(&self) -> &crate::chunk_verdict::Verdicts {
        &self.verdicts
    }

    /// Test-only (B2b): how many verdicts the last tick computed.
    #[cfg(test)]
    pub(crate) fn verdicts_last_tick_for_test(&self) -> usize {
        self.verdicts_last_tick
    }

    /// Test-only (B2b): the per-tick verdict budget in force.
    #[cfg(test)]
    pub(crate) fn verdict_budget_for_test(&self) -> crate::chunk_verdict::VerdictBudget {
        self.verdict_budget
    }

    /// Test-only (B2b): the per-tick verdict budget.
    #[cfg(test)]
    pub(crate) fn set_verdict_budget_for_test(&mut self, budget: crate::chunk_verdict::VerdictBudget) {
        self.verdict_budget = budget;
    }

    /// Test-only: chunks whose block changes slot `slot` will never receive
    /// as deltas — its outbound queue passed
    /// `state_outbox::CLIENT_QUEUE_MAX_BYTES` and the queued changes were
    /// dropped. Sorted `(cx, cy, cz)`; cleared by the call. Empty for an
    /// unknown or local slot. Live, the chunk push consumes these every tick
    /// and sends each chunk whole again; read here only with the push off
    /// ([`Self::without_chunk_push_for_test`]).
    #[cfg(test)]
    pub fn take_chunk_resync_requests(&mut self, slot: usize) -> Vec<(i32, i32, i32)> {
        self.outboxes
            .get_mut(slot)
            .map(crate::state_outbox::ClientOutbox::take_chunk_resync_requests)
            .unwrap_or_default()
    }

    /// Test-only: serialized bytes still queued for slot `slot`.
    #[cfg(test)]
    pub(crate) fn queued_state_bytes(&self, slot: usize) -> usize {
        self.outboxes.get(slot).map_or(0, crate::state_outbox::ClientOutbox::queued_bytes)
    }

    /// World chat (Phase 2) — `ChatSay` dispatch. Runs the whole pipeline via
    /// `server::handle_chat_say` (verified-key gate, rate limit, sanitiser,
    /// tier rule) and turns the outcome into wire sends: a `System` line back
    /// to the sender alone on any refusal, or a `Player` `ChatDeliver` to
    /// every permitted recipient plus an echo back to the sender (a speaker
    /// always hears their own line — the tier rule is never run against
    /// yourself). A dedicated per-recipient loop, deliberately NOT the
    /// `broadcasts` queue: that queue's whole shape is "same bytes to
    /// everyone", which is exactly what chat must not be. See
    /// `docs/foundations/2026-09-05-world-chat.md` §7.6.
    ///
    /// Native-only — the web build carries no chat surface at all (§6); its
    /// only caller (the `ChatSay` dispatch arm above) is native-only too.
    #[cfg(not(target_arch = "wasm32"))]
    fn handle_chat_say_packet(&mut self, sender: usize, raw_text: &str) {
        let from_name = self
            .server
            .players
            .get(sender)
            .map(|p| p.display_name.clone())
            .unwrap_or_default();
        match crate::server::handle_chat_say(
            &mut self.server.players,
            sender,
            raw_text,
            self.server_tick,
        ) {
            crate::server::ChatSayOutcome::NoVerifiedKey => {
                self.send_chat_system(sender, "Chat needs a verified sign-in.");
            }
            crate::server::ChatSayOutcome::RateLimited { warn } => {
                // Telling somebody they're rate-limited on every dropped line
                // would itself be a flood — only the first refusal warns.
                if warn {
                    self.send_chat_system(
                        sender,
                        "You're chatting too fast — slow down a moment.",
                    );
                }
            }
            crate::server::ChatSayOutcome::Rejected(reject) => {
                self.send_chat_system(sender, reject.message());
            }
            crate::server::ChatSayOutcome::Delivered { text, recipients } => {
                // World chat §4.2 — the room is a member of the world's
                // conversation, not a bypass of it: mirror out only if the
                // speaker would be permitted to speak to at least one member.
                self.mirror_outbound_to_room(sender, &text);
                let from_pubkey =
                    self.server.players.get(sender).and_then(|p| p.verified_pubkey);
                let pkt = protocol::serialize_packet(
                    protocol::PacketType::ChatDeliver,
                    &protocol::ChatDeliverPacket {
                        from_pubkey,
                        from_name,
                        text,
                        kind: protocol::ChatWireKind::Player,
                    },
                );
                self.send_raw_to_slot(sender, &pkt);
                for j in recipients {
                    self.send_raw_to_slot(j, &pkt);
                }
            }
        }
    }

    /// Send an already-serialized packet to slot `j` iff it's a live,
    /// handshake-complete client. Shared by the chat delivery loop and
    /// `send_chat_system` — the one place that knows a slot might have
    /// disconnected mid-tick. Every caller today is native-only chat code
    /// (`poll_room`, `handle_chat_say_packet`), so this is unused on wasm —
    /// `cfg_attr` rather than a bare `#[cfg]` so a future cross-platform
    /// caller doesn't have to un-gate it.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn send_raw_to_slot(&self, j: usize, pkt: &[u8]) {
        if self.handshake_done.get(j).copied().unwrap_or(false)
            && !self.disconnected.get(j).copied().unwrap_or(true)
        {
            self.transports[j].send_to_client(pkt);
        }
    }

    /// A `System`-kind `ChatDeliver` to one client only — refusals and
    /// rate-limit warnings never reach anyone but the player they're about.
    /// Native-only — `handle_chat_say_packet` and the unbound-operator join
    /// notice are its callers.
    #[cfg(not(target_arch = "wasm32"))]
    fn send_chat_system(&self, to: usize, text: &str) {
        let pkt = protocol::serialize_packet(
            protocol::PacketType::ChatDeliver,
            &protocol::ChatDeliverPacket {
                from_pubkey: None,
                from_name: "System".to_string(),
                text: text.to_string(),
                kind: protocol::ChatWireKind::System,
            },
        );
        self.send_raw_to_slot(to, &pkt);
    }

    /// MP-D2a — where slot `i`'s entity interest is centred: a joiner's
    /// (server-simulated) body. `None` for a local slot — the host's own
    /// loopback hears about every entity.
    fn entity_interest_anchor(&self, i: usize) -> Option<glam::Vec3> {
        self.server.players.get(i).filter(|sp| sp.server_simulated).map(|sp| sp.player.pos)
    }

    fn heartbeat_discovery(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(broadcaster) = self.broadcaster.as_mut() {
            let active_players = self.disconnected.iter().filter(|d| !**d).count();
            let capacity = self.num_local_players + self.max_remote_players;
            broadcaster.tick(
                &self.server_name,
                active_players as u8,
                capacity as u8,
                self.server.play_mode,
            );
        }
    }

    /// Returns true unless `shutdown()` has been called or the QUIC accept
    /// thread requested termination. The hosted server itself is always
    /// driven by the caller — it has no independent "is running" state.
    /// No caller yet — the main loop currently drives `tick()` unconditionally.
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            !self.shutdown.load(Ordering::Relaxed)
        }
        #[cfg(target_arch = "wasm32")]
        {
            true
        }
    }

    /// Flag the QUIC accept thread (if any) to stop. The caller should
    /// stop calling `tick` after this.
    pub fn shutdown(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl Drop for HostedServer {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.shutdown.store(true, Ordering::Relaxed);
            if let Some(handle) = self.quic_thread.take() {
                let _ = handle.join();
            }
        }
    }
}

/// Why the host refused a client's block edit (logged at debug; the client
/// only ever sees the authoritative block come back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditRefusal {
    /// Unknown block id, or an attempt to place bedrock.
    Malformed,
    /// Outside the world's height.
    OutOfWorld,
    /// In a column the server hasn't loaded.
    Unloaded,
    /// Replacing bedrock.
    Protected,
    /// Beyond reach of the server-held position.
    Reach,
    /// The world's play mode doesn't allow editing (Adventure / Spectator).
    PlayMode,
    /// An economy block owned by someone else.
    EconomyOwner,
    /// Inside a plot the joiner doesn't own.
    ForeignPlot,
}

/// MP-D2b — the item a joiner's request claims to hold: the full-fidelity
/// form when present (a tool's type, material and durability), else the
/// `ItemRef` pair; `None` for an empty hand or anything this build can't
/// decode.
fn held_item_from_wire(
    kind: u8,
    id: u16,
    full: &protocol::WireItem,
    registry: &crate::block::BlockRegistry,
) -> Option<crate::item::Item> {
    crate::inventory::item_from_wire_full(full)
        .or_else(|| crate::inventory::item_from_ref(kind, id, registry))
}

/// MP-D2b — a body's recorded death cause on the wire (`DiedOf`).
pub(crate) fn damage_cause_to_wire(cause: crate::survival::DamageCause) -> protocol::WireDamageCause {
    use crate::survival::DamageCause as D;
    use protocol::WireDamageCause as W;
    match cause {
        D::Generic => W::Generic,
        D::Fall => W::Fall,
        D::Drowning => W::Drowning,
        D::Starvation => W::Starvation,
        D::Lava => W::Lava,
        D::Fire => W::Fire,
        D::Explosion => W::Explosion,
        D::Mob(kind) => W::Mob(crate::entity_broadcast::wire_kind_for(kind)),
    }
}

/// The verified npub of a player, bech32 — the form economy owners are stored
/// in. `None` for a guest (and always on web, which has no remote joiners).
pub(crate) fn verified_npub(pk: Option<[u8; 32]>) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        pk.map(|pk| pubkey_to_npub(&pk))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = pk;
        None
    }
}

/// May the holder of `npub` (a remote joiner; `None` = guest) edit the economy
/// block at `cell`? Only if its recorded owner is that same npub. A
/// `LocalPlayer(_)` owner is a seat on the HOST's machine, never a remote
/// joiner. A cell with no owner data has nothing to protect and is allowed.
/// Non-economy blocks are always allowed here.
fn economy_owner_allows(
    world: &crate::world::World,
    cell: (i32, i32, i32),
    current: crate::block::BlockId,
    npub: Option<&str>,
) -> bool {
    let is_me = |n: &str| npub == Some(n);
    match current {
        crate::block::VENDOR_BLOCK => match world.vendor_at(cell).and_then(|v| v.owner.as_ref()) {
            Some(crate::vendor::VendorOwner::Npub(n)) => is_me(n),
            Some(crate::vendor::VendorOwner::LocalPlayer(_)) => false,
            None => true,
        },
        crate::block::TIP_JAR => match world.tip_jar_at(cell).and_then(|t| t.owner.as_ref()) {
            Some(crate::tip_jar::TipJarOwner::Npub(n)) => is_me(n),
            Some(crate::tip_jar::TipJarOwner::LocalPlayer(_)) => false,
            None => true,
        },
        crate::block::AUCTION_BLOCK => match world.auction_at(cell).map(|a| &a.owner) {
            Some(crate::auction::AuctionOwner::Npub(n)) => is_me(n),
            Some(crate::auction::AuctionOwner::LocalPlayer(_)) => false,
            None => true,
        },
        crate::block::PLOT_MARKER => world
            .plots
            .iter()
            .find(|p| p.marker == cell)
            .is_none_or(|p| crate::plot::owner_is_npub(&p.owner, npub)),
        crate::block::MARKET_BELL => match world.market_hubs.iter().find(|h| h.bell == cell) {
            Some(h) => matches!(&h.owner, crate::market_hub::HubOwner::Npub(n) if is_me(n)),
            None => true,
        },
        _ => true,
    }
}

/// Per-connection key for the challenge table. Slots are reused after a
/// release, which is safe: `release_slot` consumes the slot's challenge and
/// attaching a new connection issues a fresh one.
#[cfg(not(target_arch = "wasm32"))]
fn challenge_key_for_slot(slot: usize) -> String {
    format!("slot:{slot}")
}

/// Server-side cryptographic verification of a Signet-bearing `JoinRequest`.
/// Returns `(handle, pubkey)` on success — the handle from the signed kind-31000
/// credential's `display-name` tag (`None` when no credential was presented, or
/// it carried no usable name) and the verified x-only pubkey — or a reject
/// reason on any failure.
///
/// This is the crypto layer; it does NOT invent a label. `resolve_join_identity`
/// wraps it with the Phase 4 policy (verify-when-present, reject-when-absent) and
/// the naming ladder (`verified_display_label`: contacts book → this handle →
/// typed name → short npub, disambiguated). It runs entirely on owned data so it
/// can be tested in isolation against a handcrafted `JoinRequestPacket`.
#[cfg(not(target_arch = "wasm32"))]
fn verify_join_signet_auth(
    slot: usize,
    req: &protocol::JoinRequestPacket,
    challenges: &mut signet::ChallengeTable,
    expected_origin: &str,
) -> Result<(Option<String>, [u8; 32]), &'static str> {
    let auth_wire = req
        .auth_event
        .as_ref()
        .ok_or("missing auth_event")?;
    let auth_event: signet::SignetAuthEvent = auth_wire
        .clone()
        .try_into()
        .map_err(|_| "auth_event signature length invalid")?;

    // Consume the issued nonce — also handles expired/missing.
    let nonce = challenges
        .consume(&challenge_key_for_slot(slot))
        .ok_or("no live challenge for this connection")?;
    let nonce_hex = hex::encode(nonce);

    // verify_auth_event runs all the binding + crypto checks.
    let now_ts = current_unix_ts();
    match signet::verify_auth_event(&auth_event, &nonce_hex, expected_origin, now_ts) {
        signet::VerifyResult::Ok => {}
        signet::VerifyResult::WrongChallenge => return Err("auth event challenge mismatch"),
        signet::VerifyResult::WrongOrigin => return Err("auth event origin mismatch"),
        signet::VerifyResult::WrongKind => return Err("auth event kind/content invalid"),
        signet::VerifyResult::CreatedAtOutsideSkew => {
            return Err("auth event created_at outside ±300s skew")
        }
        signet::VerifyResult::BadEventId => return Err("auth event id mismatch"),
        signet::VerifyResult::BadSignature => return Err("auth event signature invalid"),
        signet::VerifyResult::NpFallbackRejected => {
            return Err("natural-person key not accepted on this server")
        }
        signet::VerifyResult::ExpiredCredential
        | signet::VerifyResult::CredentialPubkeyMismatch => {
            // Not produced by verify_auth_event — guarded for completeness.
            return Err("auth event verify produced an unexpected variant");
        }
    }

    // Optional credential. Verify same-pubkey + signature; extract handle.
    let mut handle: Option<String> = None;
    if let Some(cred_wire) = req.handle_credential.as_ref() {
        let cred: signet::SignetCredential = cred_wire
            .clone()
            .try_into()
            .map_err(|_| "handle_credential signature length invalid")?;
        match signet::verify_credential(&cred, &auth_event.pubkey, now_ts) {
            signet::VerifyResult::Ok => {
                // Audit 2026-09-28 A#18: the handle is credential-signed but
                // still attacker-chosen text — bound + strip it like a name.
                handle = signet::extract_display_name(&cred)
                    .map(sanitise_handle)
                    .filter(|s| !s.is_empty());
            }
            signet::VerifyResult::CredentialPubkeyMismatch => {
                return Err("handle_credential pubkey != auth_event pubkey")
            }
            signet::VerifyResult::ExpiredCredential => return Err("handle_credential expired"),
            signet::VerifyResult::WrongKind => return Err("handle_credential wrong kind"),
            signet::VerifyResult::BadEventId => return Err("handle_credential id mismatch"),
            signet::VerifyResult::BadSignature => return Err("handle_credential signature invalid"),
            // Auth-event-specific variants don't apply here.
            signet::VerifyResult::WrongChallenge
            | signet::VerifyResult::WrongOrigin
            | signet::VerifyResult::CreatedAtOutsideSkew
            | signet::VerifyResult::NpFallbackRejected => {
                return Err("handle_credential verify produced an unexpected variant")
            }
        }
    }

    Ok((handle, auth_event.pubkey))
}

/// 4 chars of the bech32 npub (the `npub1` prefix stripped) — a readable label,
/// **not** a security boundary (it is grindable for high-value handles; the real
/// anti-impersonation is the parked operator-verifier/trust-badge layer). Falls
/// back to hex if bech32 encoding fails.
#[cfg(not(target_arch = "wasm32"))]
fn npub_suffix(pubkey: &[u8; 32], n: usize) -> String {
    use nostr::ToBech32;
    let s = nostr::PublicKey::from_slice(pubkey)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .map(|npub| npub.strip_prefix("npub1").unwrap_or(&npub).to_string())
        .unwrap_or_else(|| hex::encode(pubkey));
    s.chars().take(n).collect()
}

/// Full NIP-19 npub (bech32) for a verified pubkey — sent to clients for the
/// inspect view so a specific person can be verified beyond the grindable
/// collision suffix. Falls back to hex if encoding fails.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pubkey_to_npub(pubkey: &[u8; 32]) -> String {
    use nostr::ToBech32;
    nostr::PublicKey::from_slice(pubkey)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex::encode(pubkey))
}

/// Longest display handle (in chars) the server will broadcast.
#[cfg(not(target_arch = "wasm32"))]
const MAX_HANDLE_CHARS: usize = 32;

/// Bound a display handle for broadcast: drop control characters, bidi
/// overrides/isolates/marks and zero-width format characters (which can
/// reorder or hide text to spoof another name), trim, and cap at
/// [`MAX_HANDLE_CHARS`]. Applied to verified handles and guest names alike.
#[cfg(not(target_arch = "wasm32"))]
fn sanitise_handle(raw: &str) -> String {
    fn hidden(c: char) -> bool {
        c.is_control()
            || matches!(c,
                '\u{200B}'..='\u{200F}'   // zero-width space/joiners, LRM/RLM
                | '\u{202A}'..='\u{202E}' // bidi embeddings/overrides
                | '\u{2060}'..='\u{2069}' // word joiner … bidi isolates
                | '\u{061C}'              // Arabic letter mark
                | '\u{FEFF}')             // BOM / zero-width no-break space
    }
    let cleaned: String = raw.chars().filter(|c| !hidden(*c)).collect();
    cleaned.trim().chars().take(MAX_HANDLE_CHARS).collect::<String>().trim().to_string()
}

/// A guest's (unverified) name. It is **always** marked ` (guest)` — not only on
/// a collision — so no guest can ever read as a verified player or a contact
/// (whose names are bare); a name that is empty, generic, or shaped like an npub
/// or a pubkey becomes plain `Player (guest)`. Two guests with the same name are
/// told apart with a number. Sanitised like every other handle (audit
/// 2026-09-28 A#18).
#[cfg(not(target_arch = "wasm32"))]
fn guest_display_name(asserted: &str, taken: &[&str]) -> String {
    let name = usable_name(asserted).unwrap_or_else(|| "Player".to_string());
    let base = format!("{name} (guest)");
    let in_use = |candidate: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(candidate));
    if !in_use(&base) {
        return base;
    }
    (2u32..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !in_use(candidate))
        .expect("an unused guest number exists")
}

/// Disambiguate a contacts-book name against handles already in use by other
/// present players. On collision (case-insensitive), append `-<npub_suffix>` so
/// two people the host calls the same thing stay distinguishable. (Self-asserted
/// names don't come through here: they always carry the suffix — see
/// `verified_display_label`.) The suffix is a readable label only — NOT a
/// security boundary. The full npub remains available via `verified_pubkey` for
/// an inspect view.
#[cfg(not(target_arch = "wasm32"))]
fn disambiguate_handle(handle: &str, pubkey: &[u8; 32], taken: &[&str]) -> String {
    // Case-insensitive, like `guest_display_name`: `sam` must not pass as `Sam`.
    if taken.iter().any(|t| t.eq_ignore_ascii_case(handle)) {
        format!("{handle}-{}", npub_suffix(pubkey, 4))
    } else {
        handle.to_string()
    }
}

/// A compact, human-readable form of a verified key for when nothing better is
/// known: `npub1` + the first four and last four characters of the bech32 body,
/// joined by an ellipsis (`npub1abcd…wxyz`). Never hex — the project rule is
/// npub-only display. A verified key always encodes; the non-key fallback only
/// exists so this function is total.
#[cfg(not(target_arch = "wasm32"))]
fn short_npub(pubkey: &[u8; 32]) -> String {
    use nostr::ToBech32;
    let Some(npub) = nostr::PublicKey::from_slice(pubkey).ok().and_then(|pk| pk.to_bech32().ok())
    else {
        return "Unknown player".to_string();
    };
    let body = npub.strip_prefix("npub1").unwrap_or(&npub);
    let n = body.chars().count();
    if n <= 8 {
        return format!("npub1{body}");
    }
    let head: String = body.chars().take(4).collect();
    let tail: String = body.chars().skip(n - 4).collect();
    format!("npub1{head}\u{2026}{tail}")
}

/// Whether `name` contains a run of 16 or more hex digits — the shape of a raw
/// pubkey (or a slice of one), which must never be shown to a person.
#[cfg(not(target_arch = "wasm32"))]
fn has_pubkey_shaped_run(name: &str) -> bool {
    let mut run = 0usize;
    for c in name.chars() {
        if c.is_ascii_hexdigit() {
            run += 1;
            if run >= 16 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Clean a candidate display name for the naming ladder (verified joiners'
/// contact, credential and typed names, and guests' typed names alike): sanitise
/// it like every other handle, then refuse an empty result, anything containing
/// a pubkey-shaped run (16+ hex digits) and anything that starts like an npub
/// (`npub1…`, which also covers our own `npub1abcd…wxyz` short form) — a client
/// can type whatever it likes, and a key must never be shown dressed as a name.
#[cfg(not(target_arch = "wasm32"))]
fn usable_name(raw: &str) -> Option<String> {
    let name = sanitise_handle(raw);
    let npub_shaped = name.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("npub1"));
    (!name.is_empty() && !npub_shaped && !has_pubkey_shaped_run(&name)).then_some(name)
}

/// The label the host sees for a **verified** joiner (gap-audit T2-8). Pure.
///
/// Resolution order, first usable wins:
///   (a) `contact_name` — what the host's own contacts book (Signet contacts
///       sync / Kenspeckle / mirror) calls this npub. The host chose it, so it
///       is the **only** kind of name that renders bare.
///   (b) `credential` — the `display-name` of the joiner's signed kind-31000
///       handle credential, if one was presented at join.
///   (c) `typed` — the `player_name` in the JoinRequest. A display fallback
///       only, never trusted; the generic `Player` the native client sends by
///       default counts as "no name" so it falls through to (d).
///   (d) a short npub (`npub1abcd…wxyz`) — never hex. It is the identity itself,
///       not a claimed name, so it carries no tag.
///
/// **Anti-impersonation rule.** A self-asserted name — (b) or (c) — ALWAYS
/// carries the `-<npub suffix>` tag, collision or not. A contact's name is
/// bare, so no claimed name can ever render identically to one, whatever case,
/// zero-width or lookalike trick it uses; no confusables table is needed. (The
/// tag is a readable label, not a security boundary — it is only four bech32
/// characters and grindable; the full npub is in the inspect view.) A guest
/// always carries ` (guest)` (`guest_display_name`).
///
/// Every name is sanitised (`usable_name`) BEFORE any comparison. The only
/// comparison left is for (a): a contact name equal (case-insensitively) to a
/// present player's handle gets the suffix too, so two people the host calls
/// "Sam" stay distinguishable.
#[cfg(not(target_arch = "wasm32"))]
fn verified_display_label(
    pubkey: &[u8; 32],
    contact_name: Option<&str>,
    credential: Option<&str>,
    typed: &str,
    present: &[&str],
) -> String {
    if let Some(name) = contact_name.and_then(usable_name) {
        return disambiguate_handle(&name, pubkey, present);
    }
    let typed = usable_name(typed).filter(|n| !n.eq_ignore_ascii_case("player"));
    match credential.and_then(usable_name).or(typed) {
        Some(name) => format!("{name}-{}", npub_suffix(pubkey, 4)),
        None => short_npub(pubkey),
    }
}

/// World chat (Phase 3) — resolve a verified player's effective comms `Party`
/// at join: the operator's policy tightens whatever the Charter ceiling says
/// (`crate::comms::effective_level`, §2.5). A free function (not inlined at
/// each call site) so both join paths — the local slot and a verified remote
/// join — compute this the same way, and so the composition has one place to
/// test without a real guardian-policy file on disk (`charter::comms_level`'s
/// own tests cover the file-reading half; this covers the `min` at join).
#[cfg(not(target_arch = "wasm32"))]
fn resolve_join_comms(
    charter: crate::comms::CommsLevel,
    operator: crate::comms::CommsLevel,
) -> crate::comms::Party {
    crate::comms::Party::at(crate::comms::effective_level(charter, operator))
}

/// The identity decision for a joining client.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct JoinIdentity {
    /// Display handle to broadcast (verified + disambiguated, or guest fallback).
    display_name: String,
    /// Verified Signet pubkey, or `None` for a guest join on an open server.
    pubkey: Option<[u8; 32]>,
}

/// Phase 4 join-identity policy (the layer above the crypto in
/// `verify_join_signet_auth`):
///   - `auth_event` present  → verify it (tamper/invalid → `Err(reason)`); on
///     success name the joiner via `verified_display_label` (contacts book →
///     credential handle → typed name → short npub; only a contacts-book name
///     renders bare, every self-asserted name carries the `-<npub suffix>` tag)
///     and surface the pubkey. A present-but-invalid event is ALWAYS rejected —
///     it never silently downgrades to a guest join.
///   - `auth_event` absent + `require_signin` → `Err` ("sign-in required").
///   - `auth_event` absent + open server      → guest: the sanitised asserted
///     name (or "Player") marked ` (guest)`, no verified pubkey.
///
/// Free function (not a method) so it borrows only the fields it needs, leaving
/// the rest of `HostedServer` free in the caller. Returns owned `String` errors
/// (the reject reason is sent verbatim to the client).
/// Read the queued kick npubs from `<dir>/kick` and delete the file. Returns the
/// x-only pubkey bytes; blank/unparseable lines are skipped.
#[cfg(not(target_arch = "wasm32"))]
fn take_kick_queue(dir: &std::path::Path) -> Vec<[u8; 32]> {
    let path = dir.join("kick");
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(&path);
    contents
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            nostr::PublicKey::parse(l).ok().map(|pk| pk.to_bytes())
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)] // each arg is a distinct join input/policy
fn resolve_join_identity(
    slot: usize,
    req: &protocol::JoinRequestPacket,
    require_signin: bool,
    challenges: &mut signet::ChallengeTable,
    expected_origin: &str,
    present_handles: &[&str],
    whitelist: &[[u8; 32]],
    blocklist: &[[u8; 32]],
    load_host_book: impl FnOnce() -> Vec<crate::contacts::Contact>,
) -> Result<JoinIdentity, String> {
    if req.auth_event.is_some() {
        let (credential_handle, pubkey) =
            verify_join_signet_auth(slot, req, challenges, expected_origin).map_err(str::to_string)?;
        // Access precedence (Spec B §5): block > allowlist > sign-in.
        crate::access_policy::decide_access(Some(pubkey), blocklist, whitelist, require_signin)
            .map_err(crate::access_policy::reject_reason)?;
        // T2-8: name them from the host's contacts book first, then their
        // signed credential, then the typed name, then a short npub. The book is
        // a disk read, so it happens only now — for a verified, admitted join —
        // and never for a rejected packet or a guest.
        let host_book = load_host_book();
        let contact_name = crate::contacts::find(&host_book, &pubkey)
            .and_then(|c| c.display_name.as_deref());
        let display_name = verified_display_label(
            &pubkey,
            contact_name,
            credential_handle.as_deref(),
            &req.player_name,
            present_handles,
        );
        Ok(JoinIdentity { display_name, pubkey: Some(pubkey) })
    } else {
        // No auth event: a guest. The same precedence applies (a guest is refused
        // by `require_signin` or a non-empty allowlist).
        crate::access_policy::decide_access(None, blocklist, whitelist, require_signin)
            .map_err(crate::access_policy::reject_reason)?;
        // Open server: a guest join keeps its asserted name as a display label
        // only — never a trusted identity. Sanitised and ALWAYS marked
        // ` (guest)` (audit 2026-09-28 A#18; T2-8 review).
        let name = guest_display_name(&req.player_name, present_handles);
        Ok(JoinIdentity { display_name: name, pubkey: None })
    }
}

/// Whether an input packet's `tick` is fresh — strictly newer than the last
/// accepted tick for that player. Tick 0 is reserved as the "never received"
/// sentinel (`last_input_tick` starts at 0), so it is always rejected: the old
/// `tick != 0 && tick <= last` filter accepted tick 0 unconditionally, letting a
/// client pin every packet to 0 to defeat ordering/replay (engine audit
/// 2026-06-04, D). Real input ticks are monotonically increasing and ≥ 1.
fn input_tick_is_fresh(tick: u64, last: u64) -> bool {
    tick > last
}

#[cfg(not(target_arch = "wasm32"))]
fn current_unix_ts() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

/// C1 — the tail of a possession-mismatch WARNING: how many mismatches went
/// to the debug log only since the previous one (`PossessionTally::note_mismatch`).
fn held_back_note(due: Option<u32>) -> String {
    match due {
        Some(n) if n > 0 => format!(" (+{n} more since the last warning, logged at debug)"),
        _ => String::new(),
    }
}

/// Serialize this tick's server-side pickup grants (death-drops phase 2b)
/// into per-connection `InventoryGrantPacket`s: `(player_index, bytes)` pairs
/// for the caller to route to `transports[player_index]`. Stacks that don't
/// survive the `ItemRef` wire encoding are dropped defensively — the pickup
/// pass in `GameServer::tick` already refuses them, so hitting that branch
/// means a logic regression upstream, not player-visible loss.
fn build_grant_packets(
    grants: &[(usize, crate::item::ItemStack)],
) -> Vec<(usize, Vec<u8>)> {
    grants
        .iter()
        .filter_map(|(idx, stack)| {
            // Death-drops phase 3 — everything except a Plan is grantable.
            // Blocks/materials ride the lossless `(kind, id)` pair; tools and
            // armour ride `full_item`, which the client prefers on decode.
            if matches!(stack.item, crate::item::Item::Plan(_)) {
                return None;
            }
            let (item_kind, item_id) = crate::inventory::item_to_ref(&stack.item).to_wire();
            let full_item = crate::inventory::item_to_wire_full(&stack.item);
            let pkt = protocol::InventoryGrantPacket {
                item_kind,
                item_id,
                count: stack.count,
                full_item,
            };
            Some((
                *idx,
                protocol::serialize_packet(protocol::PacketType::InventoryGrant, &pkt),
            ))
        })
        .collect()
}

/// How long one incoming QUIC connection gets to complete its handshake.
#[cfg(not(target_arch = "wasm32"))]
const QUIC_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Spawn the QUIC accept thread. Native-only — QUIC/tokio isn't available on WASM.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_quic_accept_thread(
    port: u16,
    max_remote_players: usize,
    current_remote: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    remote_tx: mpsc::Sender<Box<dyn ServerTransport>>,
    prebound: Option<std::net::UdpSocket>,
) -> Result<thread::JoinHandle<()>, String> {
    thread::Builder::new()
        .name("quic-accept".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime for accept loop");

            rt.block_on(async move {
                let endpoint = match prebound {
                    // Online play: quinn adopts the socket we already bound,
                    // gathered candidates on, and punched from.
                    Some(sock) => match crate::network::create_server_endpoint_on_socket(sock) {
                        Ok(ep) => ep,
                        Err(e) => {
                            log::error!("Failed to adopt the pre-bound QUIC socket: {e}");
                            return;
                        }
                    },
                    None => {
                        let bind_addr: SocketAddr = format!("0.0.0.0:{port}")
                            .parse()
                            .expect("valid bind address");
                        match crate::network::create_server_endpoint(bind_addr) {
                            Ok(ep) => {
                                log::info!("QUIC server listening on port {port}");
                                ep
                            }
                            Err(e) => {
                                log::error!("Failed to create QUIC endpoint: {e}");
                                return;
                            }
                        }
                    }
                };

                while !shutdown.load(Ordering::Relaxed) {
                    let accept_future = endpoint.accept();
                    match tokio::time::timeout(Duration::from_secs(1), accept_future).await {
                        Ok(Some(incoming)) => {
                            if current_remote.load(Ordering::Relaxed) >= max_remote_players {
                                log::info!("Rejecting connection — server full");
                                incoming.refuse();
                                continue;
                            }
                            // Reserve the seat now (so concurrent handshakes
                            // can't overshoot the cap) and finish the handshake
                            // in its own task, so one stalled peer never holds
                            // up the accept loop (audit 2026-09-27).
                            current_remote.fetch_add(1, Ordering::Relaxed);
                            let remote_tx = remote_tx.clone();
                            let current_remote = current_remote.clone();
                            tokio::spawn(async move {
                                let handed_over =
                                    match tokio::time::timeout(QUIC_HANDSHAKE_TIMEOUT, incoming).await {
                                        Ok(Ok(connection)) => {
                                            log::info!(
                                                "Remote player connected from {}",
                                                connection.remote_address()
                                            );
                                            let transport =
                                                crate::network::bridge_server_connection(connection);
                                            remote_tx.send(Box::new(transport)).is_ok()
                                        }
                                        Ok(Err(e)) => {
                                            log::warn!("Connection failed: {e}");
                                            false
                                        }
                                        Err(_) => {
                                            log::warn!("QUIC handshake timed out");
                                            false
                                        }
                                    };
                                if !handed_over {
                                    crate::admission::release_seat(&current_remote);
                                }
                            });
                        }
                        Ok(None) => break, // endpoint closed
                        Err(_) => {} // timeout → re-check shutdown flag
                    }
                }
                log::info!("QUIC accept loop stopped");
            });
        })
        .map_err(|e| format!("Failed to spawn QUIC accept thread: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Death-drops phase 2b: pickup grants ride the wire per connection ──

    fn decode_grant(bytes: &[u8]) -> protocol::InventoryGrantPacket {
        let (ptype, payload) = protocol::deserialize_header(bytes).unwrap();
        assert_eq!(ptype, protocol::PacketType::InventoryGrant);
        protocol::safe_deserialize(payload).unwrap()
    }

    #[test]
    fn grant_packets_encode_stack_per_player_and_skip_plans() {
        let grants = vec![
            (
                2usize,
                crate::item::ItemStack::new_material(crate::item::MaterialId::Bone, 2),
            ),
            // Plans stay floor-bound by design — `PlanData` has no wire form,
            // so a plan grant must be dropped, not garbled.
            (
                1usize,
                crate::item::ItemStack {
                    item: crate::item::Item::Plan(crate::plan::PlanData::debug_3x3_stone()),
                    count: 1,
                },
            ),
        ];
        let packets = build_grant_packets(&grants);
        assert_eq!(packets.len(), 1, "only the wire-encodable grant survives");
        let (idx, bytes) = &packets[0];
        assert_eq!(*idx, 2, "packet addressed to the picking-up player's slot");
        let pkt = decode_grant(bytes);
        assert_eq!(pkt.item_kind, protocol::item_kind::MATERIAL);
        assert_eq!(pkt.item_id, crate::item::MaterialId::Bone as u16);
        assert_eq!(pkt.count, 2);
        assert_eq!(
            pkt.full_item,
            protocol::WireItem::None,
            "a material needs no full-fidelity payload"
        );
    }

    // ── Death-drops phase 3 (v61): full-fidelity item wire ──

    #[test]
    fn grant_packets_carry_tool_and_armour_fidelity() {
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        pick.durability = 37;
        let mut boots = ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Diamond);
        boots.durability = 11;
        let grants = vec![
            (0usize, crate::item::ItemStack::new_tool(pick)),
            (
                1usize,
                crate::item::ItemStack { item: crate::item::Item::Armour(boots), count: 1 },
            ),
        ];
        let packets = build_grant_packets(&grants);
        assert_eq!(packets.len(), 2, "tools and armour are grantable from v61");

        let tool_pkt = decode_grant(&packets[0].1);
        assert_eq!(
            crate::inventory::item_from_wire_full(&tool_pkt.full_item),
            Some(crate::item::Item::Tool(pick)),
            "half-worn iron pickaxe survives the wire exactly"
        );

        let armour_pkt = decode_grant(&packets[1].1);
        assert_eq!(
            crate::inventory::item_from_wire_full(&armour_pkt.full_item),
            Some(crate::item::Item::Armour(boots)),
            "armour slot + material + durability survive the wire"
        );
    }

    #[test]
    fn input_tick_zero_is_never_fresh_so_replay_pinning_is_blocked() {
        // Tick 0 is the "never received" sentinel — always reject it (the old
        // `tick != 0 && tick <= last` accepted tick 0 unconditionally, defeating
        // replay/ordering protection; engine audit D).
        assert!(!input_tick_is_fresh(0, 0), "tick 0 rejected at session start");
        assert!(!input_tick_is_fresh(0, 5), "tick 0 rejected mid-session (replay pin)");
        assert!(input_tick_is_fresh(1, 0), "first real tick accepted");
        assert!(input_tick_is_fresh(6, 5), "a strictly newer tick is fresh");
        assert!(!input_tick_is_fresh(5, 5), "an equal tick (replay) is rejected");
        assert!(!input_tick_is_fresh(4, 5), "an older tick (out of order) is rejected");
    }

    // ── Review fix: server-side reach gate must honour a held Reach Claw ──
    // (Task 13 follow-up). Before the fix, `block_change_within_reach` didn't
    // exist — the gate was a flat constant blind to held items, so a Reach
    // Claw never worked in any hosted/multiplayer session. 6.5 blocks is
    // beyond the base (5.0 + 1.37 margin = 6.37) limit but within the
    // Reach-Claw-boosted (6.37 + REACH_CLAW_BONUS(2.0) = 8.37) limit.

    // `server_simulated = false` below means "local / position-trusted
    // player" — the same trust boundary `sp.player.pos` already relies on
    // (see the BRIDGE comment on `block_change_within_reach`).

    #[test]
    fn block_change_beyond_base_reach_is_rejected_without_reach_claw() {
        let dist_sq = 6.5f32 * 6.5;
        assert!(
            !block_change_within_reach(dist_sq, protocol::item_kind::EMPTY, 0, false),
            "empty-handed player must not reach 6.5 blocks"
        );
    }

    #[test]
    fn block_change_beyond_base_reach_is_accepted_with_reach_claw() {
        let dist_sq = 6.5f32 * 6.5;
        let reach_claw_id = crate::item::MaterialId::ReachClaw as u16;
        assert!(
            block_change_within_reach(dist_sq, protocol::item_kind::MATERIAL, reach_claw_id, false),
            "a local player holding a Reach Claw must reach 6.5 blocks"
        );
    }

    #[test]
    fn block_change_beyond_reach_claw_limit_is_still_rejected() {
        // 9 blocks is beyond even the Reach-Claw-boosted 8.37 limit.
        let dist_sq = 9.0f32 * 9.0;
        let reach_claw_id = crate::item::MaterialId::ReachClaw as u16;
        assert!(
            !block_change_within_reach(dist_sq, protocol::item_kind::MATERIAL, reach_claw_id, false),
            "the Reach Claw bonus is finite — 9 blocks is still out of reach"
        );
    }

    #[test]
    fn block_change_at_base_reach_is_accepted_regardless_of_held_item() {
        let dist_sq = 5.4f32 * 5.4; // within the base 5.5 limit either way
        assert!(block_change_within_reach(dist_sq, protocol::item_kind::EMPTY, 0, false));
        let stick_id = crate::item::MaterialId::Stick as u16;
        assert!(block_change_within_reach(
            dist_sq,
            protocol::item_kind::MATERIAL,
            stick_id,
            false
        ));
    }

    #[test]
    fn block_change_reach_unaffected_by_non_reach_claw_materials() {
        // A Crab Claw (the Reach Claw's own crafting ingredient) must not
        // itself grant the bonus — only the crafted ReachClaw tool does.
        let dist_sq = 6.5f32 * 6.5;
        let crab_claw_id = crate::item::MaterialId::CrabClaw as u16;
        assert!(
            !block_change_within_reach(dist_sq, protocol::item_kind::MATERIAL, crab_claw_id, false),
            "holding a raw Crab Claw must not grant extra reach"
        );
    }

    // ── Anti-cheat regression fix: a remote player's claimed held item must
    // not widen their reach — the server has no possession check on remote
    // inventories (see the BRIDGE comment on `block_change_within_reach`).

    #[test]
    fn remote_player_claiming_reach_claw_is_still_capped_at_base_reach() {
        // 6.5 blocks is beyond the flat 5.5 base cap but within the
        // Reach-Claw-boosted 7.5 cap a local player would get.
        let dist_sq = 6.5f32 * 6.5;
        let reach_claw_id = crate::item::MaterialId::ReachClaw as u16;
        assert!(
            !block_change_within_reach(dist_sq, protocol::item_kind::MATERIAL, reach_claw_id, true),
            "a remote (server_simulated) player's claimed Reach Claw must not widen reach"
        );
    }

    #[test]
    fn remote_player_within_base_reach_is_accepted_regardless_of_held_item() {
        let dist_sq = 5.4f32 * 5.4; // within the flat 5.5 base cap
        let reach_claw_id = crate::item::MaterialId::ReachClaw as u16;
        assert!(block_change_within_reach(
            dist_sq,
            protocol::item_kind::MATERIAL,
            reach_claw_id,
            true
        ));
        assert!(block_change_within_reach(dist_sq, protocol::item_kind::EMPTY, 0, true));
    }

    #[test]
    fn local_player_with_reach_claw_reaches_7_blocks() {
        // 7 blocks is beyond the flat 5.5 base cap but within the local
        // player's Reach-Claw-boosted 7.5 cap.
        let dist_sq = 7.0f32 * 7.0;
        let reach_claw_id = crate::item::MaterialId::ReachClaw as u16;
        assert!(block_change_within_reach(
            dist_sq,
            protocol::item_kind::MATERIAL,
            reach_claw_id,
            false
        ));
    }

    // The entity-broadcast diff tests (spawn/update/despawn, items, carts,
    // projectiles, interest) live with the code in `entity_broadcast.rs`.

    // ── Phase 3: verify_join_signet_auth ─────────────────────────────────────
    //
    // Drive the verify path against a real Schnorr-signed kind-21236 event
    // built from a fixed-seed keypair. Mirrors the unit tests in
    // `signet::verify` but exercises the JoinRequest-shaped envelope
    // (incl. wire-DTO conversion + challenge consume).

    use crate::signet::{
        canonical_id, ChallengeTable, SignetAuthEvent, SignetAuthEventWire,
        SignetCredential, SignetCredentialWire, AUTH_EVENT_KIND, CREDENTIAL_KIND,
    };
    use secp256k1::{Keypair, Secp256k1};

    fn signed_auth_event(
        seckey_bytes: [u8; 32],
        challenge_hex: &str,
        origin: &str,
        created_at: u32,
    ) -> SignetAuthEvent {
        let secp = Secp256k1::new();
        let keypair = Keypair::from_seckey_slice(&secp, &seckey_bytes).unwrap();
        let (xonly, _parity) = keypair.x_only_public_key();
        let mut pubkey = [0u8; 32];
        pubkey.copy_from_slice(&xonly.serialize());

        let tags = vec![
            vec!["challenge".to_string(), challenge_hex.to_string()],
            vec!["origin".to_string(), origin.to_string()],
        ];
        let id = canonical_id(&pubkey, created_at, AUTH_EVENT_KIND, &tags, "");
        let msg = secp256k1::Message::from_digest_slice(&id).unwrap();
        let sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(sig.as_ref());

        SignetAuthEvent {
            pubkey,
            created_at,
            kind: AUTH_EVENT_KIND,
            tags,
            content: String::new(),
            id,
            sig: sig_bytes,
            from_np: false,
        }
    }

    fn signed_credential(
        seckey_bytes: [u8; 32],
        display_name: &str,
        expires: Option<u32>,
        created_at: u32,
    ) -> SignetCredential {
        let secp = Secp256k1::new();
        let keypair = Keypair::from_seckey_slice(&secp, &seckey_bytes).unwrap();
        let (xonly, _parity) = keypair.x_only_public_key();
        let mut pubkey = [0u8; 32];
        pubkey.copy_from_slice(&xonly.serialize());

        let mut tags = vec![vec![
            "display-name".to_string(),
            display_name.to_string(),
        ]];
        if let Some(e) = expires {
            tags.push(vec!["expires".to_string(), e.to_string()]);
        }
        let id = canonical_id(&pubkey, created_at, CREDENTIAL_KIND, &tags, "");
        let msg = secp256k1::Message::from_digest_slice(&id).unwrap();
        let sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(sig.as_ref());

        SignetCredential {
            pubkey,
            created_at,
            kind: CREDENTIAL_KIND,
            tags,
            content: String::new(),
            id,
            sig: sig_bytes,
        }
    }

    /// Build a `JoinRequestPacket` carrying the given event + credential.
    fn join_with_auth(
        event: SignetAuthEvent,
        cred: Option<SignetCredential>,
    ) -> protocol::JoinRequestPacket {
        protocol::JoinRequestPacket {
            protocol_version: protocol::PROTOCOL_VERSION,
            player_name: "client-asserted-name".to_string(),
            auth_event: Some(SignetAuthEventWire::from(&event)),
            handle_credential: cred.as_ref().map(SignetCredentialWire::from),
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        }
    }

    /// Pre-issue a nonce for the slot, returning the hex form.
    fn issue_nonce(challenges: &mut ChallengeTable, slot: usize) -> String {
        let nonce = challenges.issue(challenge_key_for_slot(slot)).expect("issue");
        hex::encode(nonce)
    }

    const TEST_ORIGIN: &str = "https://localhost:7700";

    #[test]
    fn signet_join_accepted_with_valid_event_and_credential() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();

        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        let cred = signed_credential([0x42; 32], "Axolittle", None, now);
        let req = join_with_auth(ev, Some(cred));

        let (handle, pubkey) = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN)
            .expect("valid auth + credential must accept");
        assert_eq!(handle.as_deref(), Some("Axolittle"));
        // Phase 4 — verify now also surfaces the pubkey (economy ownership).
        assert_ne!(pubkey, [0u8; 32]);
    }

    #[test]
    fn signet_join_without_credential_yields_no_handle() {
        // T2-8: the crypto layer no longer invents a `Player <hex>` label. A
        // missing credential is simply "no handle"; the naming ladder
        // (`verified_display_label`) decides what the host sees.
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();

        let ev = signed_auth_event([0x55; 32], &nonce_hex, TEST_ORIGIN, now);
        let req = join_with_auth(ev, None);

        let (handle, _pubkey) = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN)
            .expect("missing credential is allowed");
        assert_eq!(handle, None, "no credential means no credential handle");
    }

    #[test]
    fn signet_join_rejected_when_auth_event_missing() {
        let mut chals = ChallengeTable::default();
        let _ = issue_nonce(&mut chals, 0);
        let req = protocol::JoinRequestPacket {
            protocol_version: protocol::PROTOCOL_VERSION,
            player_name: "x".into(),
            auth_event: None,
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        };
        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert!(err.contains("missing auth_event"));
    }

    #[test]
    fn signet_join_rejected_when_no_challenge_issued() {
        // ChallengeTable is empty — `consume` returns None.
        let mut chals = ChallengeTable::default();
        let now = current_unix_ts();
        let ev = signed_auth_event([0x42; 32], &"00".repeat(32), TEST_ORIGIN, now);
        let req = join_with_auth(ev, None);

        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert_eq!(err, "no live challenge for this connection");
    }

    #[test]
    fn signet_join_rejected_when_origin_does_not_match() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();

        // Sign for one origin, server expects a different one.
        let ev = signed_auth_event([0x42; 32], &nonce_hex, "https://other.example", now);
        let req = join_with_auth(ev, None);

        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert_eq!(err, "auth event origin mismatch");
    }

    #[test]
    fn signet_join_rejected_on_tampered_signature() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();

        let mut ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        ev.sig[0] ^= 0xff;
        let req = join_with_auth(ev, None);

        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert_eq!(err, "auth event signature invalid");
    }

    #[test]
    fn signet_join_rejected_when_from_np() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();

        let mut ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        ev.from_np = true;
        let req = join_with_auth(ev, None);

        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert_eq!(err, "natural-person key not accepted on this server");
    }

    #[test]
    fn signet_join_rejected_when_credential_has_wrong_pubkey() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();

        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        // Credential signed by a *different* keypair → pubkey mismatch.
        let bad_cred = signed_credential([0x99; 32], "Pretender", None, now);
        let req = join_with_auth(ev, Some(bad_cred));

        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert_eq!(err, "handle_credential pubkey != auth_event pubkey");
    }

    #[test]
    fn signet_join_rejected_when_credential_expired() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();

        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        let cred = signed_credential([0x42; 32], "Axo", Some(now - 1), now);
        let req = join_with_auth(ev, Some(cred));

        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert_eq!(err, "handle_credential expired");
    }

    #[test]
    fn signet_join_rejected_on_short_signature_blob() {
        // Wire DTO with a deliberately-short sig is rejected at the
        // length-validation step before any crypto runs.
        let mut chals = ChallengeTable::default();
        let _ = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();
        let ev = signed_auth_event([0x42; 32], &"00".repeat(32), TEST_ORIGIN, now);
        let mut wire: SignetAuthEventWire = (&ev).into();
        wire.sig.truncate(32);
        let req = protocol::JoinRequestPacket {
            protocol_version: protocol::PROTOCOL_VERSION,
            player_name: "x".into(),
            auth_event: Some(wire),
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        };
        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert!(err.contains("signature length invalid"));
    }

    #[test]
    fn challenge_consume_is_single_use_across_join_attempts() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();
        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        let req = join_with_auth(ev, None);

        // First call consumes the nonce.
        verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap();
        // Second call (replay attempt) finds nothing in the table.
        let err = verify_join_signet_auth(0, &req, &mut chals, TEST_ORIGIN).unwrap_err();
        assert_eq!(err, "no live challenge for this connection");
    }

    // ── Phase 4: handle disambiguation + join-identity policy ─────────────────

    #[test]
    fn sanitise_handle_strips_bidi_and_control_and_caps_length() {
        assert_eq!(sanitise_handle("Axo\u{202E}elttil"), "Axoelttil");
        assert_eq!(sanitise_handle("a\nb\u{200B}c\u{2066}d"), "abcd");
        assert_eq!(sanitise_handle("  Axolittle  "), "Axolittle");
        let long = "x".repeat(100);
        assert_eq!(sanitise_handle(&long).chars().count(), MAX_HANDLE_CHARS);
        assert_eq!(sanitise_handle("\u{202E}\u{200F}"), "");
    }

    #[test]
    fn guest_cannot_take_a_present_players_handle() {
        // A guest is always marked, so it can never equal a present player's
        // bare handle; the marker alone is the defence (no table needed).
        assert_eq!(guest_display_name("Operator", &["Operator"]), "Operator (guest)");
        assert_eq!(guest_display_name("operator", &["Operator"]), "operator (guest)");
        assert_eq!(guest_display_name("Oper\u{202E}ator", &["Operator"]), "Operator (guest)");
        assert_eq!(guest_display_name("Steve", &["Operator"]), "Steve (guest)");
        assert_eq!(guest_display_name("", &[]), "Player (guest)");
    }

    #[test]
    fn disambiguate_handle_unique_when_no_collision() {
        let pk = [0x11u8; 32];
        assert_eq!(disambiguate_handle("Axolittle", &pk, &["Staxolottle"]), "Axolittle");
    }

    #[test]
    fn disambiguate_handle_appends_suffix_on_collision() {
        let pk = [0x11u8; 32];
        let out = disambiguate_handle("Axolittle", &pk, &["Axolittle"]);
        assert!(out.starts_with("Axolittle-"), "got {out}");
        assert!(out.len() > "Axolittle-".len(), "suffix must be non-empty: {out}");
    }

    #[test]
    fn pubkey_to_npub_full_is_bech32() {
        let pk = [0x11u8; 32];
        let npub = pubkey_to_npub(&pk);
        assert!(npub.starts_with("npub1"), "got {npub}");
        assert!(npub.len() > "npub1".len() + 50, "full npub, got {npub}");
    }

    #[test]
    fn npub_suffix_is_deterministic_and_bounded() {
        let pk = [0x11u8; 32];
        let a = npub_suffix(&pk, 4);
        let b = npub_suffix(&pk, 4);
        assert_eq!(a, b, "suffix is a pure function of the key");
        assert_eq!(a.chars().count(), 4);
    }

    #[test]
    fn resolve_join_accepts_signed_and_returns_pubkey() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();
        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        let cred = signed_credential([0x42; 32], "Axolittle", None, now);
        let req = join_with_auth(ev, Some(cred));

        let id = resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &[], &[], Vec::new)
            .expect("valid signed join must be accepted");
        // A credential name is self-asserted: always tagged with the npub suffix.
        let pk = id.pubkey.expect("verified join must surface the pubkey");
        assert_eq!(id.display_name, format!("Axolittle-{}", npub_suffix(&pk, 4)));
    }

    #[test]
    fn resolve_join_disambiguates_against_present_players() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();
        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        let cred = signed_credential([0x42; 32], "Axolittle", None, now);
        let req = join_with_auth(ev, Some(cred));

        let id = resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &["Axolittle"], &[], &[], Vec::new)
            .expect("accepted");
        assert!(id.display_name.starts_with("Axolittle-"), "got {}", id.display_name);
    }

    #[test]
    fn resolve_join_rejects_absent_auth_when_signin_required() {
        let mut chals = ChallengeTable::default();
        let req = protocol::JoinRequestPacket {
            protocol_version: protocol::PROTOCOL_VERSION,
            player_name: "Guest".into(),
            auth_event: None,
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        };
        let err = resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &[], &[], Vec::new).unwrap_err();
        assert!(err.to_lowercase().contains("sign"), "got {err}");
    }

    #[test]
    fn resolve_join_allows_guest_when_signin_not_required() {
        let mut chals = ChallengeTable::default();
        let req = protocol::JoinRequestPacket {
            protocol_version: protocol::PROTOCOL_VERSION,
            player_name: "Wanderer".into(),
            auth_event: None,
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        };
        let id = resolve_join_identity(0, &req, false, &mut chals, TEST_ORIGIN, &[], &[], &[], Vec::new)
            .expect("open server allows guests");
        assert_eq!(id.display_name, "Wanderer (guest)");
        assert!(id.pubkey.is_none(), "guest has no verified pubkey");
    }

    // ── Track 4: operator allowlist ───────────────────────────────────────────

    fn xonly_of(secret: [u8; 32]) -> [u8; 32] {
        let secp = secp256k1::Secp256k1::new();
        let kp = secp256k1::Keypair::from_seckey_slice(&secp, &secret).unwrap();
        kp.x_only_public_key().0.serialize()
    }

    #[test]
    fn resolve_join_whitelisted_pubkey_accepted() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();
        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        let cred = signed_credential([0x42; 32], "Axolittle", None, now);
        let req = join_with_auth(ev, Some(cred));
        let allow = vec![xonly_of([0x42; 32])];
        let id = resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &allow, &[], Vec::new)
            .expect("whitelisted npub must be accepted");
        assert!(id.pubkey.is_some());
    }

    #[test]
    fn resolve_join_unlisted_pubkey_rejected() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();
        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        let cred = signed_credential([0x42; 32], "Axolittle", None, now);
        let req = join_with_auth(ev, Some(cred));
        let allow = vec![xonly_of([0x99; 32])]; // a different operator-approved key
        let err = resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &allow, &[], Vec::new)
            .unwrap_err();
        assert!(err.to_lowercase().contains("allowlist"), "got {err}");
    }

    #[test]
    fn resolve_join_whitelist_implies_signin() {
        let mut chals = ChallengeTable::default();
        let req = protocol::JoinRequestPacket {
            protocol_version: protocol::PROTOCOL_VERSION,
            player_name: "Guest".into(),
            auth_event: None,
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        };
        // require_signin=false, but a non-empty whitelist forces sign-in.
        let allow = vec![xonly_of([0x42; 32])];
        let err = resolve_join_identity(0, &req, false, &mut chals, TEST_ORIGIN, &[], &allow, &[], Vec::new)
            .unwrap_err();
        assert!(err.to_lowercase().contains("sign"), "got {err}");
    }

    #[test]
    fn resolve_join_rejects_tampered_auth_even_on_open_server() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();
        let mut ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        ev.sig[0] ^= 0xff;
        let req = join_with_auth(ev, None);
        // Even on an open server, a PRESENT-but-invalid auth event is rejected —
        // tamper never silently downgrades to a guest join.
        let err = resolve_join_identity(0, &req, false, &mut chals, TEST_ORIGIN, &[], &[], &[], Vec::new).unwrap_err();
        assert!(err.contains("signature invalid"), "got {err}");
    }

    // ── T2-8: real joiner names (gap-audit) ───────────────────────────────────
    //
    // Ladder for a verified npub: (a) the host's contacts book, (b) the signed
    // kind-31000 display-name, (c) the typed `player_name` (display fallback),
    // (d) a short npub. Never hex. ONLY (a) renders bare: every self-asserted
    // name, (b) and (c), always carries the `-<npub suffix>` tag, and a guest
    // always carries ` (guest)`, so no claimed name can pass as a contact's.

    fn book_entry(pubkey: [u8; 32], name: Option<&str>) -> crate::contacts::Contact {
        crate::contacts::Contact {
            pubkey,
            display_name: name.map(str::to_string),
            tier: crate::comms::Tier::Kin,
            is_child: false,
            runtime_pubkey: None,
            added_via: crate::contacts::AddedVia::Paste,
            added_at: 0,
            last_joined: None,
        }
    }

    /// `Player 1a2b3c` — the old hex fallback this work removes.
    fn looks_like_old_hex_label(s: &str) -> bool {
        s.strip_prefix("Player ").is_some_and(|rest| {
            rest.len() >= 6 && rest.chars().all(|c| c.is_ascii_hexdigit())
        })
    }

    /// The tag a self-asserted name carries.
    fn tag(pk: &[u8; 32]) -> String {
        format!("-{}", npub_suffix(pk, 4))
    }

    #[test]
    fn label_prefers_contact_over_credential_over_typed() {
        let pk = [0x11u8; 32];
        let l = verified_display_label(&pk, Some("Mum"), Some("Sam"), "Typed", &[]);
        assert_eq!(l, "Mum", "(a) the host's own contacts book wins, bare");
        let l = verified_display_label(&pk, None, Some("Sam"), "Typed", &[]);
        assert_eq!(l, format!("Sam{}", tag(&pk)), "(b) the signed credential beats the typed name");
        let l = verified_display_label(&pk, None, None, "Typed", &[]);
        assert_eq!(l, format!("Typed{}", tag(&pk)), "(c) the typed name is the fallback");
    }

    #[test]
    fn only_contact_names_render_bare() {
        // The impersonation defence: a self-asserted name ALWAYS carries the
        // npub tag, collision or not, so it can never equal a bare contact name
        // — whatever case, zero-width or lookalike trick it uses.
        let pk = [0x11u8; 32];
        for claimed in ["Mum", "mum", "MUM", "Mum\u{200B}", "M\u{200D}um", " Mum ", "Mum\u{202E}"] {
            for l in [
                verified_display_label(&pk, None, Some(claimed), "", &[]),
                verified_display_label(&pk, None, None, claimed, &[]),
            ] {
                assert_ne!(l, "Mum", "claimed {claimed:?} rendered as a bare contact name");
                assert!(l.ends_with(&tag(&pk)), "claimed {claimed:?} -> {l} must carry the tag");
            }
        }
        // …and no collision with another player is needed for the tag.
        let l = verified_display_label(&pk, None, Some("Zed"), "", &[]);
        assert_eq!(l, format!("Zed{}", tag(&pk)));
    }

    #[test]
    fn label_falls_back_to_a_short_npub_never_hex() {
        let pk = [0x11u8; 32];
        let l = verified_display_label(&pk, None, None, "", &[]);
        assert!(l.starts_with("npub1"), "got {l}");
        assert!(l.contains('\u{2026}'), "short form is elided: {l}");
        // npub1 + 4 + ellipsis + 4
        assert_eq!(l.chars().count(), 5 + 4 + 1 + 4, "got {l}");
        assert!(!looks_like_old_hex_label(&l));
        // The elided ends are the real npub's ends.
        let full = pubkey_to_npub(&pk);
        let head: String = full.chars().take(9).collect();
        let tail: String = full.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        assert_eq!(l, format!("{head}\u{2026}{tail}"));
    }

    #[test]
    fn label_treats_blank_and_generic_names_as_absent() {
        let pk = [0x22u8; 32];
        // The native client hard-codes "Player" as its typed name.
        for typed in ["", "   ", "Player", "player", " PLAYER "] {
            let l = verified_display_label(&pk, None, None, typed, &[]);
            assert!(l.starts_with("npub1"), "typed {typed:?} must fall through, got {l}");
        }
        // A blank contact name or credential falls to the next rung.
        let l = verified_display_label(&pk, Some("  "), Some("Sam"), "x", &[]);
        assert_eq!(l, format!("Sam{}", tag(&pk)));
        let l = verified_display_label(&pk, Some(""), Some("\u{200B}"), "Typed", &[]);
        assert_eq!(l, format!("Typed{}", tag(&pk)), "a name that sanitises to nothing is absent");
    }

    #[test]
    fn label_refuses_hex_and_npub_shaped_names_from_any_source() {
        let pk = [0x33u8; 32];
        let hexname = hex::encode(pk);
        let npubname = pubkey_to_npub(&[0x44u8; 32]);
        let elided = "npub1abcd\u{2026}wxyz".to_string();
        for bad in [
            hexname.as_str(),
            "deadbeefdeadbeefdeadbeef",
            "Bob 0123456789abcdef01",
            npubname.as_str(),
            "NPUB1qqqq",
            elided.as_str(),
        ] {
            for l in [
                verified_display_label(&pk, None, None, bad, &[]),
                verified_display_label(&pk, None, Some(bad), "", &[]),
                verified_display_label(&pk, Some(bad), None, "", &[]),
            ] {
                assert!(l.starts_with("npub1") && l.contains('\u{2026}'), "{bad:?} must not be shown, got {l}");
            }
            let g = guest_display_name(bad, &[]);
            assert_eq!(g, "Player (guest)", "a guest cannot wear {bad:?}");
        }
        // Ordinary names that merely contain a few hex-able letters are fine.
        assert_eq!(guest_display_name("Facade", &[]), "Facade (guest)");
    }

    #[test]
    fn label_is_sanitised_like_every_other_handle() {
        let pk = [0x44u8; 32];
        let l = verified_display_label(&pk, Some("A\u{202E}xel"), None, "", &[]);
        assert_eq!(l, "Axel", "bidi override stripped from a contact name");
        let long = "n".repeat(100);
        let l = verified_display_label(&pk, None, None, &long, &[]);
        assert_eq!(l, format!("{}{}", "n".repeat(MAX_HANDLE_CHARS), tag(&pk)));
    }

    #[test]
    fn a_contact_name_is_sanitised_before_it_is_compared_with_present_players() {
        // The contact is stored with a hidden character; the present player's
        // handle is already clean. They must still be seen as the same name.
        let pk = [0x11u8; 32];
        let l = verified_display_label(&pk, Some("S\u{200B}am"), None, "", &["Sam"]);
        assert!(l.starts_with("Sam-"), "got {l}");
        let l = verified_display_label(&pk, Some("Sam"), None, "", &["sam"]);
        assert!(l.starts_with("Sam-"), "case-insensitive: got {l}");
    }

    #[test]
    fn disambiguate_handle_is_case_insensitive() {
        let pk = [0x11u8; 32];
        let out = disambiguate_handle("axolittle", &pk, &["Axolittle"]);
        assert!(out.starts_with("axolittle-"), "got {out}");
        assert_eq!(disambiguate_handle("Sam", &pk, &["Other"]), "Sam");
    }

    #[test]
    fn a_guest_always_carries_the_guest_marker() {
        assert_eq!(guest_display_name("Wanderer", &[]), "Wanderer (guest)");
        assert_eq!(guest_display_name("", &[]), "Player (guest)");
        assert_eq!(guest_display_name("Player", &[]), "Player (guest)");
        assert_eq!(guest_display_name("Oper\u{202E}ator", &[]), "Operator (guest)");
        // A guest can never read as a contact called "Mum" (that is bare).
        assert_ne!(guest_display_name("Mum", &[]), "Mum");
    }

    #[test]
    fn two_guests_with_one_name_stay_distinguishable() {
        assert_eq!(guest_display_name("Sam", &["Sam (guest)"]), "Sam (guest) 2");
        assert_eq!(
            guest_display_name("sam", &["Sam (guest)", "Sam (guest) 2"]),
            "sam (guest) 3"
        );
        assert_eq!(guest_display_name("Steve", &["Operator"]), "Steve (guest)");
    }

    #[test]
    fn resolve_join_names_a_known_contact_from_the_hosts_book() {
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let now = current_unix_ts();
        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, now);
        let cred = signed_credential([0x42; 32], "Axolittle", None, now);
        let req = join_with_auth(ev, Some(cred));
        let book = vec![book_entry(xonly_of([0x42; 32]), Some("Little Axo"))];

        let id = resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &[], &[], || book)
            .expect("accepted");
        assert_eq!(id.display_name, "Little Axo");
        assert!(id.pubkey.is_some());
    }

    #[test]
    fn resolve_join_without_credential_uses_the_typed_name_then_npub_never_hex() {
        // Typed name present -> that, tagged.
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let ev = signed_auth_event([0x55; 32], &nonce_hex, TEST_ORIGIN, current_unix_ts());
        let req = join_with_auth(ev, None); // typed: "client-asserted-name"
        let id = resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &[], &[], Vec::new)
            .expect("accepted");
        let pk = id.pubkey.expect("verified");
        assert_eq!(id.display_name, format!("client-asserted-name{}", tag(&pk)));

        // The real client types "Player" -> short npub, not `Player 1a2b3c`.
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let ev = signed_auth_event([0x55; 32], &nonce_hex, TEST_ORIGIN, current_unix_ts());
        let mut req = join_with_auth(ev, None);
        req.player_name = "Player".into();
        let id = resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &[], &[], Vec::new)
            .expect("accepted");
        assert!(id.display_name.starts_with("npub1"), "got {}", id.display_name);
        assert!(!looks_like_old_hex_label(&id.display_name));
        assert!(id.pubkey.is_some());
    }

    #[test]
    fn resolve_join_guest_is_marked_and_has_no_hex() {
        let mut chals = ChallengeTable::default();
        let req = protocol::JoinRequestPacket {
            protocol_version: protocol::PROTOCOL_VERSION,
            player_name: "".into(),
            auth_event: None,
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        };
        let id = resolve_join_identity(0, &req, false, &mut chals, TEST_ORIGIN, &[], &[], &[], Vec::new)
            .expect("open server allows guests");
        assert_eq!(id.display_name, "Player (guest)");
        assert!(!looks_like_old_hex_label(&id.display_name));
        assert!(id.pubkey.is_none());
    }

    #[test]
    fn the_contacts_book_is_read_only_after_verification_succeeds() {
        use std::cell::Cell;
        let reads = Cell::new(0u32);
        let loader = || {
            reads.set(reads.get() + 1);
            Vec::new()
        };

        // A tampered join is rejected before the book is touched.
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let mut ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, current_unix_ts());
        ev.sig[0] ^= 0xff;
        let req = join_with_auth(ev, None);
        resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &[], &[], loader)
            .expect_err("tampered");
        assert_eq!(reads.get(), 0, "no disk read for a failed verification");

        // A guest has no npub to look up.
        let mut chals = ChallengeTable::default();
        let guest = protocol::JoinRequestPacket {
            protocol_version: protocol::PROTOCOL_VERSION,
            player_name: "G".into(),
            auth_event: None,
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        };
        resolve_join_identity(0, &guest, false, &mut chals, TEST_ORIGIN, &[], &[], &[], loader)
            .expect("guest");
        assert_eq!(reads.get(), 0, "no disk read for a guest");

        // A blocked key is refused before the book is touched.
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, current_unix_ts());
        let req = join_with_auth(ev, None);
        let blocked = vec![xonly_of([0x42; 32])];
        resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &[], &blocked, loader)
            .expect_err("blocked");
        assert_eq!(reads.get(), 0, "no disk read for a refused key");

        // A verified, admitted join reads it exactly once.
        let mut chals = ChallengeTable::default();
        let nonce_hex = issue_nonce(&mut chals, 0);
        let ev = signed_auth_event([0x42; 32], &nonce_hex, TEST_ORIGIN, current_unix_ts());
        let req = join_with_auth(ev, None);
        resolve_join_identity(0, &req, true, &mut chals, TEST_ORIGIN, &[], &[], &[], loader)
            .expect("accepted");
        assert_eq!(reads.get(), 1);
    }

    // ── T2-10: a taken LAN port is an error, not a log line ───────────────────

    #[test]
    fn lan_host_start_fails_with_a_readable_reason_when_the_port_is_taken() {
        // Hold the LAN port ourselves. If something else already owns it the
        // premise (a taken port) still holds, so the assertion stands either way.
        let _holder = std::net::UdpSocket::bind(("0.0.0.0", protocol::SERVER_PORT));
        match HostedServer::start(
            1,
            "lan-port-taken-test".to_string(),
            42,
            4,
            RemoteTransport::Quic,
        ) {
            Err(e) => {
                assert!(e.contains("already in use"), "got {e}");
                assert!(e.contains(&protocol::SERVER_PORT.to_string()), "got {e}");
            }
            Ok(_) => panic!("start must fail while the LAN port is held"),
        }
    }

    // ── World chat Phase 3: the join-time composition ──
    //
    // `comms.rs`'s `effective_level_full_table` already covers the pure 3×3
    // `min`. What these cover instead is the exact call both join sites make
    // (`resolve_join_comms`), so a future refactor of either call site can't
    // silently swap the argument order or drop the operator half.

    #[test]
    fn join_charter_approved_on_a_permissive_operator_stays_approved() {
        let party = resolve_join_comms(crate::comms::CommsLevel::Approved, crate::comms::CommsLevel::Anyone);
        assert_eq!(party, crate::comms::Party::at(crate::comms::CommsLevel::Approved));
    }

    #[test]
    fn join_operator_blocked_beats_a_charter_anyone() {
        let party = resolve_join_comms(crate::comms::CommsLevel::Anyone, crate::comms::CommsLevel::Blocked);
        assert_eq!(party, crate::comms::Party::at(crate::comms::CommsLevel::Blocked));
    }

    // ── World chat §4: the room plug, wired to `HostedServer` ──
    //
    // `world_room.rs` already covers `should_mirror_out`/`should_mirror_in` as
    // pure functions. What these cover instead is the WIRING: that
    // `handle_chat_say_packet`/`poll_room` actually call them with the right
    // arguments and turn the result into real sends — with a `FakeRoom` test
    // double so none of it touches Node, a relay or a network.

    use crate::transport::ClientTransport;
    use crate::world_room::{InboundLine, LinkError, RoomConfig, RoomError, RosterMember, WorldRoom};

    /// A test double for the seam, mirroring `world_room::tests::FakeRoom` but
    /// with `post()` reporting through a channel so a test can observe what was
    /// posted without needing to downcast the trait object back out of
    /// `HostedServer.world_room`.
    struct FakeRoom {
        posted: mpsc::Sender<String>,
        inbox: Vec<InboundLine>,
        members: Vec<RosterMember>,
    }

    impl WorldRoom for FakeRoom {
        fn start(&mut self, cfg: &RoomConfig) -> Result<(), RoomError> {
            // Deliberately does NOT check the link policy — same as
            // `world_room::tests::FakeRoom` — so a test can tell whether the
            // REFUSAL came from `HostedServer`'s own gate rather than from
            // whatever the implementation behind the trait happens to check.
            crate::world_room::lint_relays(&cfg.relays).map_err(RoomError::Relays)?;
            Ok(())
        }
        fn post(&mut self, line: &crate::world_room::OutboundLine) -> Result<(), RoomError> {
            let _ = self.posted.send(line.text.clone());
            Ok(())
        }
        fn poll(&mut self) -> Vec<InboundLine> {
            std::mem::take(&mut self.inbox)
        }
        fn members(&self) -> Vec<RosterMember> {
            self.members.clone()
        }
        fn stop(&mut self) -> Result<(), RoomError> {
            Ok(())
        }
    }

    fn fake_room(inbox: Vec<InboundLine>) -> (Box<dyn WorldRoom + Send>, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel();
        (
            Box::new(FakeRoom { posted: tx, inbox, members: Vec::new() }),
            rx,
        )
    }

    /// A world name unique enough that parallel test threads never collide on
    /// disk, matching `test_integration/late_join.rs`'s "must not exist on
    /// disk" requirement. `WebSocket { port: 0 }` + 0 remote players spawns no
    /// accept thread and no socket at all.
    fn start_room_test_server(tag: &str, num_local: usize) -> HostedServer {
        let world = format!("test-room-wiring-{}-{tag}", std::process::id());
        HostedServer::start(num_local, world, 42, 0, RemoteTransport::WebSocket { port: 0 })
            .expect("hosted server starts")
    }

    /// A room whose members are the speaker's own kin: the line IS posted.
    #[test]
    fn outbound_line_from_kin_room_member_is_posted() {
        let mut hs = start_room_test_server("kin", 1);
        let member_pk = [7u8; 32];
        hs.server.players[0].comms = crate::comms::Party::at(crate::comms::CommsLevel::Approved);
        hs.server.players[0].contacts.insert(member_pk, crate::comms::Tier::Kin);
        hs.room_members = vec![RosterMember {
            participant: hex::encode(member_pk),
            name: Some("Mum".to_string()),
            agent: false,
        }];
        let (room, rx) = fake_room(Vec::new());
        hs.world_room = Some(room);

        hs.mirror_outbound_to_room(0, "hello family");

        assert_eq!(rx.try_recv().as_deref(), Ok("hello family"));
    }

    /// An `Approved` speaker's room is full of strangers: the line is NOT
    /// posted — the room is a member of the conversation, not a bypass of it.
    #[test]
    fn outbound_line_from_approved_speaker_to_a_stranger_room_is_not_posted() {
        let mut hs = start_room_test_server("strangers", 1);
        let member_pk = [8u8; 32];
        hs.server.players[0].comms = crate::comms::Party::at(crate::comms::CommsLevel::Approved);
        // No contacts entry for member_pk: falls back to Stranger.
        hs.room_members = vec![RosterMember {
            participant: hex::encode(member_pk),
            name: None,
            agent: false,
        }];
        let (room, rx) = fake_room(Vec::new());
        hs.world_room = Some(room);

        hs.mirror_outbound_to_room(0, "crew chat");

        assert!(rx.try_recv().is_err(), "a stranger room must not receive the line");
    }

    /// An inbound room line reaches an `Anyone` listener and does not reach an
    /// `Approved` listener who has not ken'd the speaker — the room cannot
    /// bypass a ceiling by virtue of being a room.
    #[test]
    fn inbound_room_line_reaches_anyone_but_not_an_unkenned_approved_listener() {
        let mut hs = start_room_test_server("inbound", 2);
        hs.server.players[0].verified_pubkey = Some([1u8; 32]);
        hs.server.players[1].verified_pubkey = Some([2u8; 32]);
        hs.server.players[0].comms = crate::comms::Party::at(crate::comms::CommsLevel::Anyone);
        hs.server.players[1].comms = crate::comms::Party::at(crate::comms::CommsLevel::Approved);
        let speaker_hex = hex::encode([9u8; 32]);
        let (room, _rx) = fake_room(vec![InboundLine {
            from: speaker_hex,
            name: Some("Guest".to_string()),
            text: "hi from the room".to_string(),
        }]);
        hs.world_room = Some(room);

        hs.poll_room();

        let got_room_line = |t: &ChannelClientTransport| -> bool {
            while let Some(pkt) = t.try_recv_from_server() {
                if let Some((ptype, payload)) = protocol::deserialize_header(&pkt)
                    && ptype == protocol::PacketType::ChatDeliver
                    && let Ok(cd) = protocol::safe_deserialize::<protocol::ChatDeliverPacket>(payload)
                    && cd.kind == protocol::ChatWireKind::Room
                {
                    return true;
                }
            }
            false
        };
        assert!(got_room_line(&hs.local_transports[0]), "an Anyone listener must hear the room");
        assert!(
            !got_room_line(&hs.local_transports[1]),
            "an Approved listener who hasn't ken'd the speaker must not hear the room"
        );
    }

    /// Audit 2026-09-27: a guest, a split-screen seat and an unsigned slot 0
    /// all defaulted to `Anyone`, and `poll_room` never checked for a verified
    /// key — so room lines from strangers reached exactly the players §3.5
    /// excludes. By default nobody unverified hears the room; even an
    /// unverified seat forced to `Anyone` doesn't.
    #[test]
    fn inbound_room_lines_never_reach_an_unverified_seat() {
        let mut hs = start_room_test_server("guest-default", 2);
        // Slot 1: split-screen seat at its default level.
        assert_eq!(hs.server.players[1].comms.level, crate::comms::CommsLevel::Blocked);
        // Slot 0: unsigned, but somehow at Anyone — still no key, no chat.
        hs.server.players[0].verified_pubkey = None;
        hs.server.players[0].comms = crate::comms::Party::at(crate::comms::CommsLevel::Anyone);
        let (room, _rx) = fake_room(vec![InboundLine {
            from: hex::encode([9u8; 32]),
            name: Some("Stranger".to_string()),
            text: "hello".to_string(),
        }]);
        hs.world_room = Some(room);
        hs.poll_room();
        for t in &hs.local_transports {
            while let Some(pkt) = t.try_recv_from_server() {
                let (ptype, _) = protocol::deserialize_header(&pkt).unwrap();
                assert_ne!(ptype, protocol::PacketType::ChatDeliver, "no room line to an unverified seat");
            }
        }
    }

    /// Serialises the two `/room join` refusal tests below, which are the
    /// only tests in the crate touching `AXENSTAX_ROOM_RELAYS` — a shared
    /// process-global, so they must not run concurrently with each other.
    static ROOM_RELAYS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// `/room join` with no relays configured refuses — never falling back to
    /// kithmoot's own default list, which begins with our relay (§4.4).
    /// `lint_relays` produces the refusal message; this test pins that
    /// `HostedServer::room_join_with` actually reaches it.
    #[test]
    fn room_join_with_no_relay_env_refuses_naming_the_problem() {
        let _guard = ROOM_RELAYS_ENV_LOCK.lock().unwrap();
        // SAFETY: serialised by ROOM_RELAYS_ENV_LOCK; no other test touches
        // this var concurrently.
        unsafe { std::env::remove_var("AXENSTAX_ROOM_RELAYS") };
        let mut hs = start_room_test_server("join-no-relays", 1);
        let (room, _rx) = fake_room(Vec::new());

        let err = hs
            .room_join_with(room, "https://example.org/j/#owned-by-members", "Host")
            .unwrap_err();

        match &err {
            RoomError::Relays(relay_err) => {
                assert!(relay_err.message().contains("Supply your own"));
            }
            other => panic!("expected a relay refusal, got {other:?}"),
        }
    }

    /// `/room join` with a link that doesn't carry `agents: "owned-by-members"`
    /// refuses, even when relays are fine — an unowned agent in a room with a
    /// child is an anonymous stranger with a language model attached (§4.3).
    #[test]
    fn room_join_with_a_link_missing_owned_by_members_refuses() {
        let _guard = ROOM_RELAYS_ENV_LOCK.lock().unwrap();
        // SAFETY: serialised by ROOM_RELAYS_ENV_LOCK; restored before unlock.
        unsafe { std::env::set_var("AXENSTAX_ROOM_RELAYS", "wss://nos.lol") };
        let mut hs = start_room_test_server("join-bad-link", 1);
        let (room, _rx) = fake_room(Vec::new());

        let err = hs.room_join_with(room, "https://example.org/j/#nope", "Host").unwrap_err();

        unsafe { std::env::remove_var("AXENSTAX_ROOM_RELAYS") };
        assert_eq!(err, RoomError::Link(LinkError::AgentsNotOwned));
    }

    // ── Join channel binding (audit fix B, protocol v63) ─────────────────────
    //
    // The origin the joiner signs is built by the CLIENT from its own
    // transport's channel binding, and the server recomputes it from ITS
    // transport. A relaying host holds two TLS sessions with different
    // exporters, so the victim's signature never matches the real host's.

    /// Drive one signed join through `tick()` on a server whose remote
    /// transport reports `server_binding`, with the joiner signing `origin`.
    /// `Ok(())` on JoinAccept, `Err(reason)` on JoinReject.
    fn signed_join_over_binding(
        tag: &str,
        server_binding: Option<[u8; 32]>,
        origin: &str,
    ) -> Result<(), String> {
        use crate::transport::ClientTransport as _;
        let mut hs = start_room_test_server(&format!("bind-{tag}"), 1);
        let client = hs.attach_test_remote_with_binding(server_binding);
        let chal = loop {
            let pkt = client.try_recv_from_server().expect("challenge issued on attach");
            let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
            if ptype == protocol::PacketType::Challenge {
                break protocol::safe_deserialize::<protocol::ChallengePacket>(payload).unwrap();
            }
        };
        let ev = signed_auth_event([0x42; 32], &chal.nonce_hex, origin, current_unix_ts());
        let req = join_with_auth(ev, None);
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
        hs.tick();
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::JoinAccept => return Ok(()),
                protocol::PacketType::JoinReject => {
                    let r: protocol::JoinRejectPacket = protocol::safe_deserialize(payload).unwrap();
                    return Err(r.reason);
                }
                _ => {}
            }
        }
        panic!("neither JoinAccept nor JoinReject after tick");
    }

    /// Sign and send a join for `seckey` over a transport with `binding`.
    fn send_signed_join(client: &ChannelClientTransport, seckey: [u8; 32], binding: Option<[u8; 32]>) {
        use crate::transport::ClientTransport as _;
        let chal = loop {
            let pkt = client.try_recv_from_server().expect("challenge issued on attach");
            let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
            if ptype == protocol::PacketType::Challenge {
                break protocol::safe_deserialize::<protocol::ChallengePacket>(payload).unwrap();
            }
        };
        let origin = crate::signet::join_origin(binding);
        let ev = signed_auth_event(seckey, &chal.nonce_hex, &origin, current_unix_ts());
        let req = join_with_auth(ev, None);
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
    }

    /// Audit 2026-09-27: join never refused a duplicate verified pubkey, so a
    /// reconnect stacked a second live slot for the same person. On a
    /// channel-bound transport (QUIC) the new connection replaces the old one.
    #[test]
    fn a_reconnect_by_the_same_npub_replaces_the_old_slot() {
        use crate::transport::ClientTransport as _;
        let mut hs = start_room_test_server("same-npub", 1);
        let (b1, b2) = (Some([0x11; 32]), Some([0x22; 32]));
        let first = hs.attach_test_remote_with_binding(b1);
        send_signed_join(&first, [0x42; 32], b1);
        hs.tick();
        let first_slot = hs.server.players.len() - 1;
        assert!(!hs.slot_is_free(first_slot));

        let second = hs.attach_test_remote_with_binding(b2);
        send_signed_join(&second, [0x42; 32], b2);
        hs.tick();
        let second_slot = hs.server.players.len() - 1;
        assert_ne!(first_slot, second_slot);
        assert!(hs.slot_is_free(first_slot), "the stale connection's slot is released");
        assert!(!hs.slot_is_free(second_slot), "the reconnect holds the one seat");
        let mut told = false;
        while let Some(pkt) = first.try_recv_from_server() {
            if let Some((protocol::PacketType::JoinReject, _)) = protocol::deserialize_header(&pkt) {
                told = true;
            }
        }
        assert!(told, "the old connection is told why it was closed");
    }

    /// Review S6: without a channel binding (WS) a relayed signature can't be
    /// told from a reconnect, so the newcomer is refused and the live slot
    /// kept.
    #[test]
    fn an_unbound_same_npub_join_is_refused_while_the_old_slot_is_live() {
        use crate::transport::ClientTransport as _;
        let mut hs = start_room_test_server("same-npub-unbound", 1);
        let first = hs.attach_test_remote_with_binding(None);
        send_signed_join(&first, [0x43; 32], None);
        hs.tick();
        let first_slot = hs.server.players.len() - 1;
        assert!(!hs.slot_is_free(first_slot));

        let second = hs.attach_test_remote_with_binding(None);
        send_signed_join(&second, [0x43; 32], None);
        hs.tick();
        let second_slot = hs.server.players.len() - 1;
        assert!(!hs.slot_is_free(first_slot), "the live slot is kept");
        assert!(hs.slot_is_free(second_slot), "the newcomer is refused");
        let mut refused = false;
        while let Some(pkt) = second.try_recv_from_server() {
            if let Some((protocol::PacketType::JoinReject, _)) = protocol::deserialize_header(&pkt) {
                refused = true;
            }
        }
        assert!(refused, "the newcomer is told why");
    }

    #[test]
    fn pre_auth_timeout_matches_the_join_challenge_ttl() {
        assert_eq!(PRE_AUTH_TIMEOUT_TICKS, crate::signet::challenge::DEFAULT_TTL_SECS * 20);
    }

    #[test]
    fn join_accepted_when_origin_matches_the_server_transport_binding() {
        let b = [0xb0; 32];
        signed_join_over_binding("match", Some(b), &crate::signet::join_origin(Some(b)))
            .expect("origin built from the same channel binding must be accepted");
    }

    #[test]
    fn join_accepted_on_an_unbound_transport_with_the_unbound_origin() {
        signed_join_over_binding("unbound", None, &crate::signet::join_origin(None))
            .expect("unbound transport + unbound origin must be accepted");
    }

    #[test]
    fn relayed_join_signed_over_another_channel_is_rejected() {
        // V signed over its V↔M channel (binding A); M relays it to H, whose
        // H↔M channel has binding B.
        let a = [0xaa; 32];
        let b = [0xbb; 32];
        let err = signed_join_over_binding("relay", Some(b), &crate::signet::join_origin(Some(a)))
            .unwrap_err();
        assert!(err.contains("auth event origin mismatch"), "{err}");
    }

    #[test]
    fn bound_origin_is_rejected_on_an_unbound_transport() {
        let err = signed_join_over_binding(
            "bound-on-unbound",
            None,
            &crate::signet::join_origin(Some([0xaa; 32])),
        )
        .unwrap_err();
        assert!(err.contains("auth event origin mismatch"), "{err}");
    }

    #[test]
    fn legacy_https_localhost_origin_is_rejected() {
        for (tag, binding) in [("legacy-unbound", None), ("legacy-bound", Some([0xbb; 32]))] {
            let err = signed_join_over_binding(tag, binding, "https://localhost:0").unwrap_err();
            assert!(err.contains("auth event origin mismatch"), "{tag}: {err}");
        }
    }

    // ── Operator privileges need a channel-bound join (WS relay residual) ────
    //
    // A WS join has no end-to-end binding, so a relaying server can replay the
    // operator's signature. Such a join plays normally but never receives the
    // OperatorSnapshot; it is told why instead.

    const OPERATOR_SECKEY: [u8; 32] = [0x42; 32];

    /// A server identity paired to the operator whose secret is `OPERATOR_SECKEY`.
    fn operator_paired_identity(tag: &str) -> crate::server_identity::ServerIdentity {
        let dir = std::env::temp_dir()
            .join(format!("axe_opbind_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut id = crate::server_identity::generate_runtime(&dir).unwrap();
        let op = nostr::Keys::new(nostr::SecretKey::from_slice(&OPERATOR_SECKEY).unwrap());
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let ev = rt
            .block_on(crate::server_identity::attestation::mint_attestation(
                &op,
                &id.runtime_pubkey(),
                nostr::Timestamp::from(0),
                nostr::Timestamp::from(u64::MAX >> 1),
                &[27420],
                "W",
                None,
            ))
            .unwrap();
        id.store_attestation(&dir, ev).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        id
    }

    /// Join as the operator over `binding`, push one snapshot round, and
    /// report (got an OperatorSnapshot, got the direct-connection notice).
    fn operator_join_over(tag: &str, binding: Option<[u8; 32]>) -> (bool, bool) {
        use crate::transport::ClientTransport as _;
        let mut hs = start_room_test_server(tag, 1);
        hs.set_identity(Some(operator_paired_identity(tag)));
        let client = hs.attach_test_remote_with_binding(binding);
        send_signed_join(&client, OPERATOR_SECKEY, binding);
        hs.tick();
        let slot = hs.server.players.len() - 1;
        assert!(!hs.slot_is_free(slot), "the operator's join is accepted either way");
        hs.broadcast_operator_snapshot();
        let (mut snapshot, mut notice) = (false, false);
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::OperatorSnapshot => snapshot = true,
                protocol::PacketType::ChatDeliver => {
                    let cd: protocol::ChatDeliverPacket = protocol::safe_deserialize(payload).unwrap();
                    if cd.kind == protocol::ChatWireKind::System
                        && cd.text == OPERATOR_NEEDS_DIRECT_NOTICE
                    {
                        notice = true;
                    }
                }
                _ => {}
            }
        }
        (snapshot, notice)
    }

    #[test]
    fn an_unbound_operator_join_gets_no_snapshot_and_is_told_why() {
        let (snapshot, notice) = operator_join_over("op-unbound", None);
        assert!(!snapshot, "an unbound join must never receive the OperatorSnapshot");
        assert!(notice, "the operator is told operator tools need a direct connection");
        assert_eq!(OPERATOR_NEEDS_DIRECT_NOTICE, "Operator tools need a direct connection.");
    }

    #[test]
    fn a_bound_operator_join_still_gets_the_snapshot() {
        let (snapshot, notice) = operator_join_over("op-bound", Some([0x5a; 32]));
        assert!(snapshot, "a channel-bound operator join receives the OperatorSnapshot");
        assert!(!notice, "no notice on a direct connection");
    }

    // ── WebSocket join origin (v66, T-JOIN-RELAY WebSocket residual) ─────────
    //
    // A WS joiner signs `axenstax-join:ws-host:<the host it dialled>` and
    // declares that host in its JoinRequest. A server that knows its public
    // address refuses a signature made for any other — so a relaying server
    // M can't replay victim V's signature (made for M) to the real server H.

    use crate::signet::ws_host::{relay_protection_warning, ws_host_origin, PublicHosts};

    const WS_NONCE: [u8; 32] = [0x77; 32];

    /// One WebSocket join against `hs`: the joiner declares `declared` as the
    /// host it dialled and, when `signed` is `Some((seckey, origin))`, carries
    /// an auth event signed over `origin` (else it is a guest). Returns the
    /// client and `Ok(JoinAccept)` / `Err(reject reason)`.
    fn ws_join(
        hs: &mut HostedServer,
        declared: &str,
        signed: Option<([u8; 32], String)>,
    ) -> (ChannelClientTransport, Result<protocol::JoinAcceptPacket, String>) {
        use crate::transport::ClientTransport as _;
        let client = hs.attach_test_remote_ws();
        let chal = loop {
            let pkt = client.try_recv_from_server().expect("challenge issued on attach");
            let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
            if ptype == protocol::PacketType::Challenge {
                break protocol::safe_deserialize::<protocol::ChallengePacket>(payload).unwrap();
            }
        };
        let mut req = match signed {
            Some((seckey, origin)) => join_with_auth(
                signed_auth_event(seckey, &chal.nonce_hex, &origin, current_unix_ts()),
                None,
            ),
            None => crate::remote_client::build_join_request_guest("Guest", 0),
        };
        req.ws_host = declared.to_string();
        req.client_nonce_hex = hex::encode(WS_NONCE);
        client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
        hs.tick();
        let mut out = None;
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::JoinAccept if out.is_none() => {
                    out = Some(Ok(protocol::safe_deserialize(payload).unwrap()));
                }
                protocol::PacketType::JoinReject if out.is_none() => {
                    let r: protocol::JoinRejectPacket = protocol::safe_deserialize(payload).unwrap();
                    out = Some(Err(r.reason));
                }
                _ => {}
            }
        }
        (client, out.expect("neither JoinAccept nor JoinReject after tick"))
    }

    fn h_hosts() -> PublicHosts {
        PublicHosts::parse(&["h.example.org"], 6767).unwrap()
    }

    /// The relay scenario: V dialled M and signed for M's address; M forwards
    /// it to H, which knows its own address.
    #[test]
    fn a_relayed_ws_signature_is_refused_by_a_server_that_knows_its_address() {
        let mut h = start_room_test_server("ws-relay-h", 1);
        h.set_ws_public_hosts(h_hosts());
        let v = [0x44; 32];
        let for_m = ws_host_origin("m.example.net:6767");
        // M forwards V's JoinRequest verbatim (declaring M's host)…
        let (_c, res) = ws_join(&mut h, "m.example.net:6767", Some((v, for_m.clone())));
        let err = res.unwrap_err();
        assert!(err.starts_with("auth event origin mismatch"), "{err}");
        // …or rewrites the declared host to H's: V's signature still names M.
        let (_c, res) = ws_join(&mut h, "h.example.org:6767", Some((v, for_m)));
        assert!(res.unwrap_err().contains("auth event origin mismatch"));
        // A player who really dialled H gets in.
        let (_c, res) = ws_join(&mut h, "h.example.org:6767", Some((v, ws_host_origin("h.example.org:6767"))));
        res.expect("an honest WS join to H's own address is accepted");
    }

    /// Without `--public-host` the residual stands (any host is accepted) and
    /// the boot warning fires.
    #[test]
    fn an_unconfigured_server_still_accepts_a_relayed_ws_signature_and_warns() {
        let mut h = start_room_test_server("ws-relay-open", 1);
        assert!(relay_protection_warning(&h.ws_public_hosts).is_some(), "boot warning");
        let (_c, res) =
            ws_join(&mut h, "m.example.net:6767", Some(([0x45; 32], ws_host_origin("m.example.net:6767"))));
        res.expect("unconfigured: the WS relay residual is accepted");
        h.set_ws_public_hosts(h_hosts());
        assert_eq!(relay_protection_warning(&h.ws_public_hosts), None);
    }

    /// IP-vs-domain: a player who dialled the server's IP while it answers as
    /// its domain is refused with a generic reason. The rejection goes to an
    /// unauthenticated peer, so it must not list the configured hosts (LAN
    /// addresses included); they are logged server-side instead.
    #[test]
    fn a_ws_join_by_ip_to_a_domain_server_is_refused_without_naming_its_hosts() {
        let mut h = start_room_test_server("ws-ip-domain", 1);
        h.set_ws_public_hosts(
            PublicHosts::parse(&["h.example.org", "192.168.1.20"], 6767).unwrap(),
        );
        let (_c, res) =
            ws_join(&mut h, "203.0.113.7:6767", Some(([0x46; 32], ws_host_origin("203.0.113.7:6767"))));
        let err = res.unwrap_err();
        assert!(err.starts_with("auth event origin mismatch"), "{err}");
        assert!(err.contains("expects to be reached at its public address"), "{err}");
        for configured in ["h.example.org", "192.168.1.20"] {
            assert!(!err.contains(configured), "the JoinReject leaked '{configured}': {err}");
        }
    }

    /// A LAN address in the public hosts still admits LAN joins (the server is
    /// WS-only), but the boot log marks it as not relay-protected.
    #[test]
    fn a_lan_public_host_still_admits_lan_joins_but_is_flagged_at_boot() {
        let mut h = start_room_test_server("ws-lan", 1);
        let hosts = PublicHosts::parse(&["h.example.org", "192.168.1.20"], 6767).unwrap();
        assert_eq!(hosts.non_unique(), vec!["192.168.1.20".to_string()]);
        h.set_ws_public_hosts(hosts);
        let (_c, res) = ws_join(&mut h, "192.168.1.20:6767", None);
        res.expect("a LAN join to a listed LAN address is admitted");
    }

    /// A relay listening on another port of the server's own name is refused:
    /// a port-less entry no longer means "any port".
    #[test]
    fn a_ws_join_on_an_unlisted_port_of_the_servers_name_is_refused() {
        let mut h = start_room_test_server("ws-port", 1);
        h.set_ws_public_hosts(h_hosts());
        let (_c, res) =
            ws_join(&mut h, "h.example.org:9000", Some(([0x48; 32], ws_host_origin("h.example.org:9000"))));
        assert!(res.unwrap_err().starts_with("auth event origin mismatch"));
        let (_c, res) = ws_join(&mut h, "h.example.org:8443", None);
        res.expect("8443 (Caddy) is admitted by a port-less entry");
    }

    /// Guests are checked too: the identity proof is signed over their host.
    #[test]
    fn a_guest_ws_join_to_another_address_is_refused_when_configured() {
        let mut h = start_room_test_server("ws-guest", 1);
        h.set_ws_public_hosts(h_hosts());
        let (_c, res) = ws_join(&mut h, "m.example.net:6767", None);
        assert!(res.unwrap_err().starts_with("auth event origin mismatch"));
        let (_c, res) = ws_join(&mut h, "h.example.org", None);
        res.expect("a guest that dialled H is admitted on a guest-open server");
    }

    /// A v65-style WS join (signed `unbound`, no declared host) no longer passes.
    #[test]
    fn an_unbound_or_undeclared_ws_join_is_refused() {
        let mut h = start_room_test_server("ws-unbound", 1);
        let (_c, res) =
            ws_join(&mut h, "h.example.org", Some(([0x47; 32], crate::signet::join_origin(None))));
        assert!(res.unwrap_err().contains("auth event origin mismatch"));
        let (_c, res) = ws_join(&mut h, "", None);
        assert!(res.unwrap_err().contains("didn't say which address"));
    }

    /// The JoinAccept identity proof is bound to the WS host: it verifies for
    /// a client that dialled that host and fails for one that dialled a relay.
    #[test]
    fn the_ws_identity_proof_is_bound_to_the_dialled_host() {
        use nostr::ToBech32;
        let mut h = start_room_test_server("ws-proof", 1);
        h.set_identity(Some(operator_paired_identity("ws-proof")));
        h.set_ws_public_hosts(h_hosts());
        let (_c, res) = ws_join(&mut h, "h.example.org:6767", None);
        let accept = res.expect("guest join to H's address");
        let proof = accept.server_identity.expect("a provisioned server sends a proof");
        let npub = nostr::Keys::new(nostr::SecretKey::from_slice(&OPERATOR_SECKEY).unwrap())
            .public_key()
            .to_bech32()
            .unwrap();
        use crate::server_identity::proof::{evaluate_server_identity, ServerAuthOutcome};
        let now = nostr::Timestamp::now();
        let at_h = ws_host_origin("h.example.org:6767");
        assert_eq!(
            evaluate_server_identity(Some(&npub), Some(&proof), &WS_NONCE, &at_h, now),
            ServerAuthOutcome::Verified(npub.clone())
        );
        let at_m = ws_host_origin("m.example.net:6767");
        assert!(matches!(
            evaluate_server_identity(Some(&npub), Some(&proof), &WS_NONCE, &at_m, now),
            ServerAuthOutcome::Refused(_)
        ));
    }

    /// Operator privileges still need a channel-bound (QUIC) join, even when
    /// the WS join is relay-protected by a configured public host.
    #[test]
    fn a_relay_protected_ws_operator_join_still_gets_no_snapshot() {
        use crate::transport::ClientTransport as _;
        let mut h = start_room_test_server("ws-op", 1);
        h.set_identity(Some(operator_paired_identity("ws-op")));
        h.set_ws_public_hosts(h_hosts());
        let (client, res) =
            ws_join(&mut h, "h.example.org", Some((OPERATOR_SECKEY, ws_host_origin("h.example.org"))));
        res.expect("the operator plays normally over WS");
        h.broadcast_operator_snapshot();
        while let Some(pkt) = client.try_recv_from_server() {
            if let Some((ptype, _)) = protocol::deserialize_header(&pkt) {
                assert_ne!(ptype, protocol::PacketType::OperatorSnapshot, "no snapshot over WS");
            }
        }
    }
}

/// T1-3 — which side ticks the block machines is decided at construction, by
/// whether a host client exists. These pin the wiring of
/// `GameServer::simulates_block_machines` in `start_inner`.
#[cfg(test)]
mod block_machine_flag_tests {
    use super::*;

    /// Unique per process so parallel test threads never share a world folder.
    /// `WebSocket { port: 0 }` + 0 remote players spawns no accept thread and
    /// binds no socket (the `start_room_test_server` pattern).
    fn start(tag: &str, num_local: usize) -> HostedServer {
        let world = format!("test-block-machines-{}-{tag}", std::process::id());
        HostedServer::start(num_local, world, 42, 0, RemoteTransport::WebSocket { port: 0 })
            .expect("hosted server starts")
    }

    #[test]
    fn a_dedicated_server_ticks_its_own_block_machines() {
        // 0 local players = the dedicated server (`server_main`): nobody else
        // will smelt, push or grow anything, so the server must.
        let hs = start("dedicated", 0);
        assert!(hs.server.simulates_block_machines);
    }

    #[test]
    fn a_lan_host_server_leaves_block_machines_to_its_host_client() {
        // ≥ 1 local player = a host client exists and already ticks every
        // machine (and mirrors its block-entities in). A second tick here would
        // double every piston push.
        let hs = start("lan-host", 1);
        assert!(!hs.server.simulates_block_machines);
    }
}

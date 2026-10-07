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
}

impl JoinedWorld {
    pub fn from_accept(accept: &protocol::JoinAcceptPacket) -> Self {
        let spawn = glam::Vec3::new(accept.spawn_x, accept.spawn_y, accept.spawn_z);
        Self {
            seed: accept.seed,
            rules: accept.world_rules.clone(),
            worldgen_version: accept.worldgen_version,
            spawn: spawn.is_finite().then_some(spawn),
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
    Died,
    /// The server respawned us here (after our `Respawn` request).
    Respawned(glam::Vec3),
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
    pub chunk_queue: Vec<(usize, protocol::ChunkDataPacket)>,
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
    /// MP-A3 — the tick (`self.tick`) our last `Respawn` request went out, while
    /// the server has not yet answered with `Respawned`. `send_input` re-sends
    /// every [`RESPAWN_RESEND_TICKS`] until it does: the server drops a
    /// client's packets past its per-tick budget, and one lost `Respawn` would
    /// otherwise leave us walking about a respawned body the server holds dead.
    respawn_resend_from: Option<u64>,
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
    /// packet, oldest first. [`Self::send_input`] puts them ahead of the next
    /// packet's own edits, so a burst too big for one packet is spread over
    /// several instead of the tail being lost (the host never saw it, so it
    /// could never refuse and un-ghost it on this client). At most
    /// [`INPUT_CARRY_OVER_MAX_CHANGES`].
    input_carry_over: Vec<protocol::BlockChange>,
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
/// then, which the server reads as "use your own limit".
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
            respawn_resend_from: None,
            pending_operator_snapshot_json: None,
            pending_entity_batches: Vec::new(),
            pending_block_changes: Vec::new(),
            input_carry_over: Vec::new(),
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
            respawn_resend_from: None,
            pending_operator_snapshot_json: None,
            pending_entity_batches: Vec::new(),
            pending_block_changes: Vec::new(),
            input_carry_over: Vec::new(),
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
                            self.latest_state = Some(state);
                            changed = true;
                        }
                    }
                    PacketType::ChunkData => {
                        match protocol::safe_deserialize::<protocol::ChunkDataPacket>(payload) {
                            Ok(chunk) if self.chunk_queue.len() < MAX_QUEUED_CHUNK_PACKETS => {
                                self.chunk_queue.push((self.pending_block_changes.len(), chunk));
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
                            Err(e) => log::warn!("Undecodable chunk packet: {e}"),
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
                                protocol::PlayerEventType::Died => {
                                    if self.player_index() == Some(event.player_index)
                                        && self.pending_life_events.len() < 16
                                    {
                                        self.pending_life_events.push(OwnLifeEvent::Died);
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
                                            && self.pending_life_events.len() < 16
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
                    PacketType::InventoryGrant => {
                        // Death-drops phase 2b — the server picked up a stack
                        // for this player. Queue for the game loop to decode
                        // into the local inventory.
                        if let Ok(grant) = protocol::safe_deserialize::<
                            protocol::InventoryGrantPacket,
                        >(payload)
                        {
                            // Bounded: a hostile server can't
                            // grow this without limit between frames.
                            if self.pending_grants.len() < 256 {
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

        // Edits trimmed from an earlier packet go first: the host validates
        // them in the order they were made.
        if !self.input_carry_over.is_empty() {
            let mut edits = std::mem::take(&mut self.input_carry_over);
            edits.append(&mut input.block_changes);
            input.block_changes = edits;
        }
        let (packet, mut trimmed) = serialize_input_within_cap(&mut input);
        if trimmed.len() > INPUT_CARRY_OVER_MAX_CHANGES {
            let drop = trimmed.len() - INPUT_CARRY_OVER_MAX_CHANGES;
            log::warn!(
                "input edits are backing up: dropping the {drop} oldest of {} waiting to be sent",
                trimmed.len()
            );
            trimmed.drain(..drop);
        }
        self.input_carry_over = trimmed;
        self.transport.send_to_server(&packet);
        Some(seq)
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

/// Serialize a `ClientInput`, trimming its block changes (newest first) until
/// the packet fits [`protocol::MAX_WIRE_PACKET_LEN`] — the frame cap, which a
/// bigger packet would trip, closing the connection (gap-audit T2-12). Before
/// the cap was aligned such a packet still went out, and the host dropped the
/// whole of it (position included) at `safe_deserialize`.
///
/// Returns the packet and the trimmed tail, oldest first. The caller carries it
/// into the next packet (`RemoteClient::send_input`): the host applies at most
/// `MAX_BLOCK_CHANGES_PER_TICK` edits a tick and sends back the real block for
/// each it refuses, but it can only do that for an edit it has seen.
fn serialize_input_within_cap(
    input: &mut protocol::InputPacket,
) -> (Vec<u8>, Vec<protocol::BlockChange>) {
    let packet = protocol::serialize_packet(PacketType::ClientInput, &*input);
    let Some(first) = input.block_changes.first() else {
        return (packet, Vec::new());
    };
    if packet.len() <= protocol::MAX_WIRE_PACKET_LEN {
        return (packet, Vec::new());
    }
    let per_change = bincode::serialized_size(first).expect("a block change sizes") as usize;
    let excess = packet.len() - protocol::MAX_WIRE_PACKET_LEN;
    let keep = input.block_changes.len().saturating_sub(excess.div_ceil(per_change));
    log::warn!(
        "input packet over the {}-byte cap: sending {keep} of {} block changes, the rest in the next packet(s)",
        protocol::MAX_WIRE_PACKET_LEN,
        input.block_changes.len()
    );
    let trimmed = input.block_changes.split_off(keep);
    (protocol::serialize_packet(PacketType::ClientInput, &*input), trimmed)
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
        let (pkt, trimmed) = serialize_input_within_cap(&mut small);
        assert!(trimmed.is_empty(), "nothing trimmed under the cap");
        assert_eq!(small.block_changes.len(), 1, "a packet under the cap is untouched");
        assert_eq!(pkt, protocol::serialize_packet(PacketType::ClientInput, &small));

        let mut big = protocol::InputPacket {
            block_changes: (0..10_000)
                .map(|i| protocol::BlockChange::with_meta(i, 64, 0, 1, 0))
                .collect(),
            ..Default::default()
        };
        let (pkt, trimmed) = serialize_input_within_cap(&mut big);
        assert!(pkt.len() <= protocol::MAX_WIRE_PACKET_LEN, "{} bytes", pkt.len());
        let (_, payload) = protocol::deserialize_header(&pkt).unwrap();
        let back: protocol::InputPacket = protocol::safe_deserialize(payload).unwrap();
        assert!(back.block_changes.len() > 4_000, "only the overflow is trimmed");
        assert_eq!(back.block_changes[0].x, 0, "the oldest changes are the ones kept");
        // The trimmed tail is handed back, not lost: kept + trimmed is the lot, in order.
        assert_eq!(back.block_changes.len() + trimmed.len(), 10_000);
        let xs: Vec<i32> = back.block_changes.iter().chain(&trimmed).map(|b| b.x).collect();
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
        let pkt = srv.try_recv_from_client().expect("an input packet was sent");
        assert!(pkt.len() <= protocol::MAX_WIRE_PACKET_LEN, "{} bytes", pkt.len());
        let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
        assert_eq!(ptype, PacketType::ClientInput);
        let input: protocol::InputPacket = protocol::safe_deserialize(payload).unwrap();
        input.block_changes.iter().map(|b| b.x).collect()
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
        let ev = protocol::PlayerEventPacket { player_index: index, event };
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
                OwnLifeEvent::Died,
                OwnLifeEvent::Respawned(glam::Vec3::new(1.5, 70.0, -2.5)),
            ]
        );
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
}

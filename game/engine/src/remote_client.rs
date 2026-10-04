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

/// A remote game client connected to a server.
pub struct RemoteClient {
    transport: Box<dyn ClientTransport>,
    pub state: ConnectionState,
    /// Tick counter for input packets.
    tick: u64,
    /// Latest state update from the server (consumed by game loop each frame).
    pub latest_state: Option<protocol::StateUpdatePacket>,
    /// Queued chunk data packets (consumed by game loop each frame).
    pub chunk_queue: Vec<protocol::ChunkDataPacket>,
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
    /// Entity-event and block-change DELTAS accumulated across every
    /// StateUpdate since the game loop last drained them. `latest_state` is
    /// last-write-wins, which is right for snapshot fields (players,
    /// world_time, reserve) but silently dropped these deltas whenever a
    /// frame hitch batched two server ticks into one poll — lost spawns
    /// meant permanently invisible loot, lost despawns ghost items, lost
    /// block changes world desync. Drained via `std::mem::take` each frame.
    pub pending_entity_spawns: Vec<protocol::EntitySpawn>,
    pub pending_entity_updates: Vec<protocol::EntityUpdate>,
    pub pending_entity_despawns: Vec<u32>,
    pub pending_block_changes: Vec<protocol::BlockChange>,
    /// World chat (Phase 2) — lines the server delivered to us this poll,
    /// drained by the game loop each frame into `ChatState`. Bounded like
    /// `pending_grants`: a hostile server can't grow this without limit
    /// between frames. Native-only — the web build carries no chat surface
    /// at all (`docs/foundations/2026-09-05-world-chat.md` §6).
    #[cfg(not(target_arch = "wasm32"))]
    pub pending_chat: Vec<protocol::ChatDeliverPacket>,
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
    }
}

impl RemoteClient {
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
    /// `signet::join_origin` from the transport's channel binding) to sign a kind-21236 auth
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

    /// Connect to a dedicated server over WebSocket — the path BOTH the browser
    /// PWA and the native client use (browsers can't speak QUIC). Guest join
    /// (no Signet auth yet). `url` is `ws://host:port` (native) or
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
    /// Live caller: game_loop.rs, wasm32-only today (the native authed path
    /// goes through `native_join_sign_driver` + the QUIC transport instead).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
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
    fn from_transport(
        transport: Box<dyn ClientTransport>,
        join_req: protocol::JoinRequestPacket,
        pinned_op_npub: Option<String>,
    ) -> Self {
        let client_nonce = hex::decode(&join_req.client_nonce_hex).unwrap_or_default();
        let packet = protocol::serialize_packet(PacketType::JoinRequest, &join_req);
        transport.send_to_server(&packet);
        Self {
            transport,
            state: ConnectionState::Connecting,
            tick: 0,
            latest_state: None,
            chunk_queue: Vec::new(),
            spawn_pos: None,
            pending_play_mode: None,
            pending_difficulty: None,
            pending_world_time: None,
            pending_exhibits: None,
            join: JoinFlow::Sent,
            roster: HashMap::new(),
            pinned_op_npub,
            client_nonce,
            verified_operator: None,
            pending_resource_pack: None,
            pending_grants: Vec::new(),
            pending_operator_snapshot_json: None,
            pending_entity_spawns: Vec::new(),
            pending_entity_updates: Vec::new(),
            pending_entity_despawns: Vec::new(),
            pending_block_changes: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            pending_chat: Vec::new(),
        }
    }

    /// Build a client around a transport WITHOUT sending the JoinRequest. The
    /// authenticated handshake waits for the server's `ChallengePacket`, signs
    /// it via `driver`, and only then sends (see `poll`).
    fn from_transport_authed(
        transport: Box<dyn ClientTransport>,
        base: protocol::JoinRequestPacket,
        driver: SignDriverFn,
        pinned_op_npub: Option<String>,
    ) -> Self {
        let client_nonce = hex::decode(&base.client_nonce_hex).unwrap_or_default();
        Self {
            transport,
            state: ConnectionState::Connecting,
            tick: 0,
            latest_state: None,
            chunk_queue: Vec::new(),
            spawn_pos: None,
            pending_play_mode: None,
            pending_difficulty: None,
            pending_world_time: None,
            pending_exhibits: None,
            join: JoinFlow::AwaitingChallenge { base, driver: Some(driver) },
            roster: HashMap::new(),
            pinned_op_npub,
            client_nonce,
            verified_operator: None,
            pending_resource_pack: None,
            pending_grants: Vec::new(),
            pending_operator_snapshot_json: None,
            pending_entity_spawns: Vec::new(),
            pending_entity_updates: Vec::new(),
            pending_entity_despawns: Vec::new(),
            pending_block_changes: Vec::new(),
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
                                    // v63: verify over OUR channel binding, so
                                    // a proof relayed from another leg fails.
                                    self.transport.channel_binding(),
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
                            self.pending_entity_spawns.append(&mut state.entity_spawns);
                            self.pending_entity_updates.append(&mut state.entity_updates);
                            self.pending_entity_despawns.append(&mut state.entity_despawns);
                            self.pending_block_changes.append(&mut state.block_changes);
                            self.latest_state = Some(state);
                            changed = true;
                        }
                    }
                    PacketType::ChunkData => {
                        if let Ok(chunk) = protocol::safe_deserialize::<protocol::ChunkDataPacket>(payload) {
                            // Cap chunk queue to prevent memory exhaustion from
                            // server flooding or slow client consumption.
                            if self.chunk_queue.len() < 256 {
                                self.chunk_queue.push(chunk);
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
                                            // built from OUR transport's channel
                                            // binding — never taken from the
                                            // server, which could be a relay or
                                            // be fishing for a web login.
                                            let origin = crate::signet::join_origin(
                                                self.transport.channel_binding(),
                                            );
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
                            // Bounded like chunk_queue: a hostile server can't
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
    pub fn send_input(&mut self, input: &protocol::InputPacket) {
        if !matches!(self.state, ConnectionState::Connected { .. }) {
            return;
        }

        let mut input = input.clone();
        input.tick = self.tick;
        self.tick += 1;

        let packet = protocol::serialize_packet(PacketType::ClientInput, &input);
        self.transport.send_to_server(&packet);
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

    /// Send a graceful disconnect to the server.
    pub fn disconnect(&mut self) {
        let packet = protocol::serialize_packet(PacketType::Disconnect, &());
        self.transport.send_to_server(&packet);
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

        let spawn_ids: Vec<u32> = rc.pending_entity_spawns.iter().map(|s| s.id).collect();
        assert_eq!(spawn_ids, vec![7], "tick 1's spawn survives tick 2's packet");
        assert_eq!(rc.pending_entity_despawns, vec![7], "tick 2's despawn kept too");
        let change_xs: Vec<i32> = rc.pending_block_changes.iter().map(|b| b.x).collect();
        assert_eq!(change_xs, vec![1, 2], "BOTH ticks' block changes, in order");
        // Snapshot data stays last-write-wins.
        assert_eq!(rc.latest_state.as_ref().map(|s| s.tick), Some(2));
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
        }
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
}

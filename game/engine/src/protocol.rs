//! Network protocol — packet types for client-server communication.
//!
//! All packets are serialized with bincode and prefixed with a 1-byte packet type tag.
//! Chunks are additionally compressed with LZ4.

use bincode::Options;
use serde::{Deserialize, Serialize};

/// Packet type tag (first byte of every packet).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum PacketType {
    /// Client → Server: player input this tick
    ClientInput = 1,
    /// Server → Client: world state update
    StateUpdate = 2,
    /// Server → Client: chunk data (reliable, ordered)
    ChunkData = 3,
    /// Server → Client: "this column is local" (v71, Phase B2b): the column is
    /// exactly as generation made it, so the joiner generates it itself. Rides
    /// the same ordered stream as `ChunkData` and is numbered with it (it
    /// counts towards `InputPacket.chunk_ack`). See [`ColumnLocalPacket`].
    ColumnLocal = 4,
    /// Client → Server: request to join
    JoinRequest = 10,
    /// Server → Client: join accepted with player info
    JoinAccept = 11,
    /// Server → Client: join rejected with reason
    JoinReject = 12,
    /// Client → Server: graceful disconnect
    Disconnect = 20,
    /// Server → Client: player joined/left notification
    PlayerEvent = 21,
    /// Both: keepalive ping
    Ping = 30,
    /// Both: keepalive pong
    Pong = 31,
    /// Server → Client: LAN broadcast discovery
    ServerAnnounce = 40,
    /// Server → Client: per-connection auth challenge nonce. Issued once on
    /// connect; the client signs the nonce inside the kind-21236 auth event
    /// it puts in `JoinRequestPacket`. Phase 3 of `docs/foundations/2026-04-20-engine-signet-auth.md`.
    Challenge = 50,
    /// Server → Client: live Operator Console snapshot, sent only to a connected
    /// player whose verified npub is the server operator (Spec B task 7).
    OperatorSnapshot = 51,
    /// Server → Client: suggested resource pack (Spec 03 §11.6). Sent once after
    /// join when the operator configures one; the client fetches + caches it by
    /// hash and (if accepted) loads it as the active pack. `required` packs
    /// disconnect a declining client.
    ResourcePackSuggest = 52,
    /// Server → Client: a stack the server picked up on this player's behalf
    /// (death-drops phase 2b). Sent only to the picking-up client, whose
    /// inventory is client-authoritative until the dual-sim rework — the
    /// server-side pickup despawns the item for everyone via the entity
    /// despawn broadcast, and this delivers the stack.
    InventoryGrant = 53,
    /// Client → Server: a chat line the player typed. Carries only what the
    /// client is entitled to assert — the text — never attribution (see
    /// `ChatDeliver`). World chat, Phase 2:
    /// `docs/foundations/2026-09-05-world-chat.md` §7.1.
    ///
    /// The web build's forbidden-symbol gate (`tools/smoke/forbidden-symbol.mjs`)
    /// greps the wasm bundle for the chat *payload struct names*
    /// (`ChatSayPacket`, `ChatDeliverPacket`) so that the implementation stays
    /// native-only — chat would make the web taster a service, which the spec
    /// forbids. This variant (and `ChatDeliver` below) is deliberately NOT
    /// `#[cfg]`-gated: `PacketType` is a wire-stable discriminant table shared
    /// by both targets, and letting its numbering differ per target invites a
    /// future mismatch. Its `Debug` string ("ChatSay") staying in the wasm
    /// bundle is fine and expected — do not "finish the job" by gating these
    /// variants or their `deserialize_header` arms.
    ChatSay = 54,
    /// Server → Client: one chat line, already evaluated against the
    /// world-chat tier rule for THIS recipient. Carries what the server has
    /// decided (attribution + kind) — a single symmetric packet would invite
    /// a client to assert its own `from` field. See the note on `ChatSay`
    /// above: this variant stays ungated on both targets.
    ChatDeliver = 55,
    /// Client → Server: the player right-clicked an interactable power device
    /// (Wind/Copper/Electricity wave, Task 2b). Carries the CELL and nothing
    /// else — what the interaction does is the host's decision, taken from the
    /// device it finds standing there. Autonomous sources (Windmill, Water
    /// Wheel, pressure plates, sensors) already reached the host through the
    /// server's own sim; a switch had no carrier at all, so a joiner's lever
    /// flipped only their own copy of the world.
    DeviceInteract = 56,
    /// Client → Server: the player chose Respawn on their death screen
    /// (protocol v67, MP-A3). Empty payload — it asserts the wish and nothing
    /// else: the server respawns the player only if IT holds them dead, at the
    /// spawn point IT holds, and answers with `PlayerEventType::Respawned`.
    /// Sent by a joiner only; a host's own players respawn in its client sim.
    /// A client re-sends it every 20 ticks until answered (the server's
    /// per-tick packet budget never drops it, but a request made inside the
    /// first 20 ticks of a death is ignored — see `Respawned`).
    Respawn = 57,
    /// Client → Server: the player swung at one of the server's entities
    /// (MP-D2b). Names the entity by its `ProtocolId` and claims the item in
    /// hand; the server decides everything else (alive, reach from its own
    /// body, cooldown, damage, knockback, kill credit) and answers with an
    /// `InteractOutcome`. Sent by a joiner only.
    EntityAttack = 58,
    /// Client → Server: a one-shot right-click interaction with one of the
    /// server's mobs (MP-D2b): feed, tame, shear, milk, lead on/off, sit
    /// toggle. Same shape of trust as `EntityAttack`; answered with an
    /// `InteractOutcome`. Riding and villager trading are not on it (D2c).
    EntityInteract = 59,
    /// Server → Client: the server's decision on one `EntityAttack` or
    /// `EntityInteract` (MP-D2b), sent to that player alone. The client owns
    /// its inventory until phase C, so it takes the consumed items and wears
    /// its weapon ONLY on an accepted outcome; products ride `InventoryGrant`.
    InteractOutcome = 60,
    /// Server → Client: a mob this player killed has died (MP-D2b), sent to
    /// the killer alone, so its client's kill attribution (kill counters,
    /// challenges, the Nostrich's Vow, village reputation) runs for the kill.
    KillEvent = 61,
    /// Client → Server: an item action (C2a, [`ItemActionPacket`]) — eat the
    /// food in hand, sleep in a bed. The server decides (`item_actions`): it
    /// runs the joiner's hunger, so eating feeds and heals the body it holds,
    /// and a sleep sets the spawn point it respawns the joiner at. Answered
    /// with an `ItemActionOutcome`. Sent by a joiner only (web joiners
    /// included); a host's own players eat and sleep in its client sim.
    ItemAction = 62,
    /// Server → Client: the decision on one `ItemAction` (C2a), sent to that
    /// player alone. Like `InteractOutcome`, the client takes the food it
    /// claimed only on an accepted outcome.
    ItemActionOutcome = 63,
    /// Client → Server: one inventory-window op (C3a-2a, [`WindowOpPacket`]):
    /// a click the joiner's client applied to its window, or the screen it
    /// opened, or its auto-refill setting. The server applies the same rule
    /// (`window::apply`) to its copy of that joiner's window, in arrival
    /// order behind the client's edits, and compares digests (log-only).
    /// Never answered. Sent by a joiner only.
    WindowOp = 64,
    /// Server → Client: the answer to a joiner's `WireWindowOp::OpenContainer`
    /// (C3b-1, [`ContainerOpenedPacket`]): the server's real chest, dispenser,
    /// dropper or furnace at that cell, slot for slot, or why it can't open.
    ContainerOpened = 65,
    /// Server → Client: the server's values for some slots of the joiner's
    /// window (C3b-1, [`WindowSlotSetPacket`]): a correction after a container
    /// op whose result differed, or a push of what changed in the open
    /// container. Never a whole-window overwrite.
    WindowSlotSet = 66,
}

// ─── Handshake ───────────────────────────────────────────────

/// Longest `JoinRequestPacket::player_name` a host accepts, in BYTES (the host
/// compares `str::len`). The single source of truth: the host's join gate
/// ([`player_name_is_valid`]), its rejection text and the handshake test all
/// read this one constant.
pub const MAX_PLAYER_NAME_LEN: usize = 32;

/// Is `name` an acceptable `JoinRequestPacket::player_name`? At most
/// [`MAX_PLAYER_NAME_LEN`] bytes and no control characters (a terminal-escape
/// name must never reach a host's log or another player's screen). The name is
/// a display fallback only, so this is hygiene, not identity.
pub fn player_name_is_valid(name: &str) -> bool {
    name.len() <= MAX_PLAYER_NAME_LEN && !name.chars().any(|c| c.is_control())
}

/// What the host tells a joiner whose name failed [`player_name_is_valid`].
pub fn player_name_reject_reason() -> String {
    format!("Invalid player name (max {MAX_PLAYER_NAME_LEN} chars, no control chars)")
}

/// Client requests to join a server.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinRequestPacket {
    /// Protocol version (for compatibility checking)
    pub protocol_version: u32,
    /// Player display name — **display fallback only, NEVER a trusted identity
    /// source** (Phase 4, `docs/foundations/2026-04-20-engine-signet-auth.md`).
    /// Real identity is the verified `auth_event` pubkey + the handle from
    /// `handle_credential` (kind-31000 `display-name`). The server uses this field
    /// only for a guest join on an open (non-sign-in) server; a verified join
    /// derives the handle from the credential and ignores it.
    pub player_name: String,
    /// Signed kind-21236 auth event proving control of `auth_event.pubkey` and
    /// binding to the server-issued challenge nonce. `None` for a guest join;
    /// populated by an authenticated client (Phase 4). The server verifies it
    /// whenever present and rejects an absent one on a sign-in-required host.
    pub auth_event: Option<crate::signet::SignetAuthEventWire>,
    /// Optional kind-31000 handle credential (display-name binding). Same
    /// pubkey as `auth_event`; verified separately. `None` is allowed — a
    /// verified join with no credential is named from the host's contacts book,
    /// then the typed `player_name`, then a short npub (never hex).
    pub handle_credential: Option<crate::signet::SignetCredentialWire>,
    /// Skin reference the joining client announces: a `u64` content hash
    /// (`CosmeticDescriptor::skin_key`). `0` = default skin. The host copies this
    /// onto the new `ServerPlayer.skin_key`, which is then rebroadcast on every
    /// `PlayerState`. Carries only the REFERENCE — the skin bytes are delivered
    /// out-of-band (gated; no native upload source today, so native joins send
    /// `0`).
    pub skin_key: u64,
    /// Client-chosen challenge nonce (lowercase hex of 32 random bytes) for
    /// **server** authentication (Track 3). The server signs
    /// `challenge_msg(nonce, origin)` with its runtime key and returns the
    /// signature + attestation in `JoinAcceptPacket.server_identity`, letting a
    /// client that pinned an operator npub verify the server. Empty string = the
    /// client doesn't request server-identity proof (anonymous join, unchanged).
    pub client_nonce_hex: String,
    /// The joiner's own [`crate::world::worldgen_fingerprint`] (v65, gap-audit
    /// T2-9). A joiner regenerates the host's terrain locally, so the host
    /// records this on the player (`ServerPlayer::worldgen_mismatch`) to know
    /// whose terrain may differ from its own.
    #[serde(default)]
    pub worldgen_version: u32,
    /// WebSocket joins only (v66, Spec 04 §1.8.1): the normalised
    /// `host[:port]` the joiner actually dialled (`signet::ws_host::
    /// ws_url_host`). Its join auth event signs `axenstax-join:ws-host:<this>`,
    /// and the server re-normalises it, checks it against its `--public-host`
    /// list and signs its identity proof over the same origin. Empty on
    /// QUIC / in-process joins, where it is ignored. Untrusted: a lie only
    /// makes the signature (or the proof) fail.
    #[serde(default)]
    pub ws_host: String,
    /// The joiner's render distance in columns (v69, Phase B2a). The server
    /// pushes chunks out to `min(this, its own limit)` round the joiner's
    /// body (`chunk_push`). `0` = not said: the server's limit applies.
    /// Read once, at join. APPEND-ONLY: stays last.
    #[serde(default)]
    pub render_distance: u8,
}

/// Server accepts a join request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinAcceptPacket {
    /// Assigned player index on the server
    pub player_index: u32,
    /// World seed (for client-side terrain gen if needed)
    pub seed: u32,
    /// Spawn position
    pub spawn_x: f32,
    pub spawn_y: f32,
    pub spawn_z: f32,
    /// Current world time (for day/night sync)
    pub world_time: u32,
    /// Game mode (legacy projection — kept for consumers that only branch on
    /// creative; new consumers read `play_mode` below).
    pub is_creative: bool,
    /// Full play mode (Spec 05 §8). `is_creative` above is the derived
    /// projection kept for back-compat with consumers that only branch on
    /// creative; new consumers read `play_mode`.
    pub play_mode: crate::play_mode::PlayMode,
    /// Difficulty
    pub difficulty: String,
    /// Server-identity proof (Track 3): the operator-signed attestation + the
    /// runtime key's signature over the client's `client_nonce_hex`. `None` when
    /// the server is unprovisioned (anonymous) or the client sent no nonce. A
    /// client that pinned an operator npub verifies this and refuses on mismatch;
    /// a client with no pinned operator ignores it.
    pub server_identity: Option<ServerIdentityProof>,
    /// Creator-gallery exhibits (Spec 2026-06-19 §9) — the world's authored 2D art
    /// placements, sent on join so a remote/web client renders them (they are
    /// authored data, not procedural, so unlike the gallery they can't be
    /// regenerated client-side). APPEND-ONLY (bincode is positional): later
    /// fields go after it, never before. Empty for a normal world.
    #[serde(default)]
    pub exhibits: Vec<crate::exhibit::Exhibit>,
    /// The world's generation flags + rules (v65, gap-audit T2-9). A joiner
    /// applies them BEFORE it generates any terrain, so a flat or void world
    /// generates flat or void on the joiner too, and mobs / fire / explosives /
    /// keep-inventory / the day lock behave as on the host.
    #[serde(default)]
    pub world_rules: WorldRules,
    /// The host's [`crate::world::worldgen_fingerprint`] (v65). A joiner on a
    /// different version warns its player that terrain may look different.
    #[serde(default)]
    pub worldgen_version: u32,
    /// Phase B2b (v71): how far round its server body, in columns (Chebyshev),
    /// this joiner hears a verdict for every column — a push or a
    /// [`ColumnLocalPacket`] — capped further by its own render distance.
    /// Inside that it generates a column only on a "local" note; outside it,
    /// as before. `0` = this server sends no notes: it pushes everything in
    /// range (`--chunk-sync all`, another terrain generator, or a host that
    /// does not keep columns loaded round its joiners).
    #[serde(default)]
    pub chunk_note_radius: u8,
}

/// The rule + generation flags of a world that a joiner needs to generate and
/// behave like the host (v65, gap-audit T2-9). Exactly the `WorldMeta` fields
/// that change terrain output or gameplay rules; the seed, play mode,
/// difficulty and clock travel in their own `JoinAcceptPacket` fields.
/// `commands_enabled` is deliberately NOT carried: on a joiner it would also
/// gate the chat key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldRules {
    /// `WorldMeta.world_type`: `"normal"`, `"flat"`, `"testlab"`, ….
    pub world_type: String,
    /// `WorldMeta.ground`: the flat world's floor block.
    pub ground: String,
    /// `WorldMeta.water_depth`: water layers for `ground = "water"`.
    pub water_depth: u8,
    /// `WorldMeta.is_workshop`: the void Workshop preset.
    pub is_workshop: bool,
    /// `WorldMeta.time_lock`: `"cycle"`, `"day"` or `"night"`.
    pub time_lock: String,
    /// `WorldMeta.mobs_enabled`.
    pub mobs_enabled: bool,
    /// `WorldMeta.explosives_enabled` (Spec 49).
    pub explosives_enabled: bool,
    /// `WorldMeta.fire_spread_enabled`.
    pub fire_spread_enabled: bool,
    /// `WorldMeta.keep_inventory` (#47).
    pub keep_inventory: bool,
}

impl Default for WorldRules {
    /// A fresh world's rules (the `WorldMeta::new` defaults).
    fn default() -> Self {
        Self::from_meta(&crate::save::WorldMeta::new(""))
    }
}

impl WorldRules {
    /// The rules a saved world's meta records.
    pub fn from_meta(meta: &crate::save::WorldMeta) -> Self {
        Self {
            world_type: meta.world_type.clone(),
            ground: meta.ground.clone(),
            water_depth: meta.water_depth,
            is_workshop: meta.is_workshop,
            time_lock: meta.time_lock.clone(),
            mobs_enabled: meta.mobs_enabled,
            explosives_enabled: meta.explosives_enabled,
            fire_spread_enabled: meta.fire_spread_enabled,
            keep_inventory: meta.keep_inventory,
        }
    }

    /// Write these rules into `meta` (the joiner builds its world meta from
    /// `JoinAcceptPacket` this way). Touches only the fields listed above.
    pub fn apply_to_meta(&self, meta: &mut crate::save::WorldMeta) {
        meta.world_type = self.world_type.clone();
        meta.ground = self.ground.clone();
        meta.water_depth = self.water_depth;
        meta.is_workshop = self.is_workshop;
        meta.time_lock = self.time_lock.clone();
        meta.mobs_enabled = self.mobs_enabled;
        meta.explosives_enabled = self.explosives_enabled;
        meta.fire_spread_enabled = self.fire_spread_enabled;
        meta.keep_inventory = self.keep_inventory;
    }
}

/// Server → Client (inside `JoinAcceptPacket`): proof of the server's operator
/// identity. Cross-platform wire DTO (no crypto deps) — the native client
/// verifies it via `server_identity::verify_server_proof`; the WASM client can't
/// verify yet (nostr is native-only) and ignores it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerIdentityProof {
    /// The operator-signed attestation event, JSON-encoded.
    pub attestation_json: String,
    /// BIP-340 signature (64 bytes) by the runtime key over the client nonce.
    pub challenge_sig: Vec<u8>,
}

/// Server rejects a join request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinRejectPacket {
    pub reason: String,
}

/// Server → operator-player: a JSON-encoded `ConsoleSnapshot` (Spec B). Carrying
/// JSON (rather than a typed field) keeps the wire forward-compatible as the
/// snapshot grows, and avoids the engine depending on the console types here.
// Constructed by the operator-snapshot send + client route (Spec B owner-boundary
// follow-up B-7a); the wire shape + round-trip are tested now.
#[allow(dead_code)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperatorSnapshotPacket {
    pub snapshot_json: String,
}

/// Server → Client: a resource pack the server suggests (Spec 03 §11.6). The
/// client checks its hash-keyed cache, fetches `url` over HTTPS if absent,
/// verifies `sha256`, and prompts the player (Accept/Decline). A `required` pack
/// disconnects a declining player; otherwise declining keeps the current pack.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourcePackSuggestPacket {
    /// Human-readable pack name for the prompt.
    pub name: String,
    /// HTTPS URL of the pack artifact (a `pack.json` index the client walks).
    pub url: String,
    /// Lowercase hex SHA-256 of the pack artifact, for cache key + integrity.
    pub sha256: String,
    /// Artifact size in bytes (for the prompt + cache accounting); 0 if unknown.
    pub size_bytes: u64,
    /// If true, declining disconnects the player (Spec 03 §11.6 step 4).
    pub required: bool,
}

/// Server → Client: a stack the server picked up on this player's behalf
/// (death-drops phase 2b, v58). The stack rides the same `ItemRef` wire
/// encoding as `EntitySpawn.item_kind`/`item_id` — which only round-trips
/// blocks and materials — plus, since v61, a trailing `full_item` carrying
/// the per-instance fidelity that pair loses (tool type/material/durability,
/// armour slot/material/durability). Plans have no wire form and stay
/// floor-bound: the server never grants one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InventoryGrantPacket {
    /// `item_kind::*` discriminator (`BLOCK` or `MATERIAL` in practice).
    pub item_kind: u8,
    /// Block id / material discriminant per `ItemRef::to_wire`.
    pub item_id: u16,
    /// Units granted (partial pickups grant only what landed server-side).
    pub count: u8,
    /// Full-fidelity payload (v61). `WireItem::None` for blocks/materials,
    /// where the pair above is already lossless. When present it WINS over
    /// the pair on decode. Appended last; `serde(default)` documents the
    /// intent, but the join-time version gate is the real back-compat —
    /// bincode has no field-presence marker, so a v60 peer's shorter stream
    /// would mis-decode, not default.
    #[serde(default)]
    pub full_item: WireItem,
    /// v76 (C3a-fix-1) — the number of this grant as a window event: the
    /// server numbers every change it makes to a joiner's window (1, 2, 3…
    /// per connection) and applies it to its copy only once the client
    /// reports, in a later packet's `events_applied`, that it applied it
    /// too (`window_ops::WindowEvents`). Always non-zero on a grant.
    #[serde(default)]
    pub window_event: u32,
}

// ─── World chat (Phase 2) ───
//
// Native-only: the web build is the anonymous local taster (no sign-in, no
// server-authoritative attribution), so these payload structs must not exist
// in the wasm bundle at all — `tools/smoke/forbidden-symbol.mjs` greps for
// their names. The `PacketType::ChatSay`/`ChatDeliver` *discriminants* stay
// ungated on both targets (see the comment there); only the payload types and
// their users are native-only.

/// Client → Server: what a player typed. The text only — the server decides
/// who it reaches and what it's attributed to; a client-asserted `from`
/// field would be exactly the hole `ChatDeliver` exists to close.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatSayPacket {
    pub text: String,
}

/// Which conversation a delivered line belongs to. `Player` and `Room` render
/// with distinct HUD colours (a room line reads as "from outside the world");
/// `System` is a refusal/rate-limit/notice line, never delivered to anyone but
/// the player it's about.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatWireKind {
    Player,
    Room,
    System,
}

/// Server → Client: one chat line, already permitted for this specific
/// recipient (world-chat tier rule evaluated server-side, per recipient —
/// see `crate::comms`).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatDeliverPacket {
    /// The speaker's verified pubkey — `None` only for a `System` line, which
    /// has no speaker.
    pub from_pubkey: Option<[u8; 32]>,
    /// Display fallback, server-chosen (the sender's `ServerPlayer.display_name`)
    /// — never the client's own assertion.
    pub from_name: String,
    pub text: String,
    pub kind: ChatWireKind,
}

/// Server → Client: per-connection challenge nonce. Sent once immediately
/// after the transport is accepted, before the client builds its
/// `JoinRequestPacket`.
///
/// `nonce_hex` is the lowercase hex of 32 random bytes, matching the
/// `["challenge", <hex>]` tag the client puts on its kind-21236 auth event.
///
/// Phase 4: the client now consumes the nonce — an authenticated join waits for
/// this packet, signs `{nonce, origin}`, and sends the resulting `auth_event`
/// inside its `JoinRequestPacket`.
///
/// v63 (audit fix B): the packet carries NO origin. Until v62 the server sent
/// one and the client signed it verbatim, on the claim that a single-use nonce
/// made that safe. It did not: a malicious host could relay a real host's
/// challenge to a victim and join as them, or send a website origin plus that
/// site's CSRF challenge and get a valid web login signed. The client now builds
/// the origin from its own transport (`signet::join_origin` over the QUIC TLS
/// exporter) and the server recomputes it from its own; a relay sees two
/// different exporters and fails the check.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChallengePacket {
    pub nonce_hex: String,
}

// ─── Input (Client → Server, every tick, unreliable datagram) ───

/// Player input sent from client to server each tick.
/// This is the network-serialized version of PlayerIntent.
///
/// A joiner's position is the server's: the server simulates it from the
/// movement fields below, and the client predicts the same and reconciles
/// against `StateUpdatePacket.last_acked_input` (Spec 04 §5.3). Only a host's
/// own local (position-trusted) slots have `x/y/z` applied as sent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InputPacket {
    /// Client input sequence number: strictly increasing per session (the
    /// server drops replays), and what `last_acked_input` echoes back.
    pub tick: u64,
    /// The client's predicted position after this input. Applied only for
    /// position-trusted local slots; a joiner's is ignored (server-simulated).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Look direction (absolute, not delta — server doesn't accumulate)
    pub yaw: f32,
    pub pitch: f32,
    /// Health as this client sees it. Applied as sent for a host's own local
    /// (position-trusted) slot. For a joiner the server's copy is the truth
    /// (MP-D2a): this field only reports a death the client's own sim caused
    /// (`<= 0`, MP-A3); its owned changes ride `health_delta`.
    pub health: f32,
    /// Currently held item (block ID or 0)
    ///
    /// Legacy block-only field, kept for back-compat with code that still
    /// reads a bare block id. The tool-capable wire form is the
    /// `held_kind`/`held_id` pair below, resolved client-side from the
    /// active hotbar slot's `Item` via `inventory::item_to_ref().to_wire()`.
    pub held_item: u16,
    /// Tool-capable held-item wire encoding (client-authoritative).
    /// `held_kind` is an `item_kind::*` discriminator, `held_id` the id
    /// within that kind. The server relays these straight onto the
    /// per-player broadcast for server-simulated (remote) players.
    pub held_kind: u8,
    pub held_id: u16,
    /// Movement (analog, -1.0 to 1.0)
    pub move_forward: f32,
    pub move_right: f32,
    /// Actions (booleans, packed as individual fields for clarity)
    pub sprint: bool,
    pub sneak: bool,
    pub jump: bool,
    pub toggle_flight: bool,
    pub break_block: bool,
    pub place_block: bool,
    pub toggle_inventory: bool,
    pub drop_item: bool,
    pub hotbar_slot: Option<u8>,
    /// Block changes this tick (placed or broken blocks)
    pub block_changes: Vec<BlockChange>,
    /// MP-D2a (v68) — the armour points this player is wearing
    /// (`PlayerSlot::total_armour_points`). A joiner's armour lives in its
    /// client-held inventory (CLAUDE.md known debt), so the server, which now
    /// lands mob and lava/fire hits on a joiner's body, is told how much of
    /// each hit armour soaks up. Client-asserted, like `held_kind`: a lying
    /// client could only claim armour it isn't wearing, and until D2a its
    /// health was wholly its own anyway. Ignored for a local slot.
    /// APPEND-ONLY with `health_delta` (bincode is positional). Like every
    /// earlier append, `serde(default)` here documents intent only: a v67
    /// peer's shorter packet would fail to decode, not default, and the
    /// join-time `PROTOCOL_VERSION` gate is the real back-compat (see
    /// `InventoryGrantPacket.full_item` and the test
    /// `state_update_from_a_shorter_older_peer_is_rejected_not_defaulted`).
    #[serde(default)]
    pub armour_points: u8,
    /// MP-D2a (v68) — the change this client made to its own health since its
    /// previous input. A joiner's health is the server's; the server adds this
    /// to its copy when it simulates the input, so the next `StateUpdate`
    /// acknowledging the input (`last_acked_input`) carries it. Fall,
    /// drowning, mob and lava/fire damage are NOT in it: the server applies
    /// those itself. C2a (v73) — and so are eating, regen, starvation, poison
    /// and the sleep heal (the server runs a joiner's metabolism and its item
    /// actions): the server takes a LOSS only, and a reported heal counts as
    /// zero (`server::sanitise_reported_health_change`); a heal the client
    /// shows itself (an op's `/heal` on its own view) does not stick. Zero
    /// from a local slot (its `health` is applied as sent). `serde(default)`:
    /// see `armour_points`.
    #[serde(default)]
    pub health_delta: f32,
    /// v69 (Phase B2a) — how many `ChunkData` packets this client has taken
    /// in since it joined, cumulative. The server's chunk-push credit window
    /// (`chunk_push`) counts a push as in flight until this passes it.
    #[serde(default)]
    pub chunk_ack: u32,
    /// v69 — columns this client let go of (unloaded, discarded), each with
    /// its `chunk_ack` count at that moment. The server takes them out of its
    /// sent-set so it stops sending their changes and pushes them again when
    /// they are back in range. A report is repeated in every input until the
    /// server has applied one that carried it (`last_acked_input`); `as_of`
    /// makes a repeat harmless. Bounded per packet
    /// ([`MAX_CHUNK_DROPS_PER_INPUT`]); the rest go in the next.
    #[serde(default)]
    pub chunk_drops: Vec<ChunkDrop>,
    /// v69 — this client's CURRENT render distance in columns (`0` = not
    /// said / unchanged). The server's chunk-push radius follows it
    /// (`min(this, server limit)`), so lowering it mid-session never leaves
    /// the server pushing columns the client unloads (B2a review MEDIUM-1).
    #[serde(default)]
    pub render_distance: u8,
    /// v71 (Phase B2b) — set once this client found a column it was told is
    /// local whose own generation (a scratch generation, not the column as
    /// it holds it: B2b fix HIGH-1) does not hash as the server's note said
    /// (`ColumnLocalPacket::hash`), then sent in EVERY input for the rest of
    /// the session: a sticky "push me everything" switch (repeating it makes
    /// it survive a lost or budget-dropped input; the server acts on the
    /// first and ignores the rest, and ignores it from a client it never
    /// sent a note). The server logs it as a determinism bug, pushes this
    /// client every column it had noted local and notes no more.
    #[serde(default)]
    pub column_mismatch: Option<ColumnMismatch>,
    /// v72 (C1) — the blocks this client MINED (its survival break arm)
    /// since its previous input, each with the tool it mined with. The
    /// server yields a joiner's break itself (`break_drops`,
    /// granted by `InventoryGrant`), and needs to tell a mined cell from the
    /// other edits that empty a cell (a bucket scoop, an Eraser, a Latent
    /// Print lifted, a piston or keg the client's own machines ran) and to
    /// know the tool: the `held_kind`/`held_id` pair carries no tool type,
    /// and is sampled after the strike wore the tool (the strike that breaks
    /// a pickaxe still yields). The tool is the client's word, like every held
    /// item (BRIDGE: possession check). One tag per mined edit, in the
    /// packet carrying that edit and never another (FU1, C1 verify N4: paired
    /// at the source, `RemoteClient::send_input`); the server gives a cell's
    /// tags, in order, to the edits of that cell that break it, each tag to
    /// one (`HostedServer::classify_joiner_edit`). The server reads at most
    /// [`MAX_MINED_PER_INPUT`], and the client never sends more: edits past
    /// the limit wait for its next input with their tags.
    #[serde(default)]
    pub mined: Vec<MinedBlock>,
    /// v76 (C3a-fix-1) — the highest server window event this client had
    /// applied when it made this input's edits (`window_ops::WindowEvents`):
    /// the server applies its own queued window events up to this number
    /// before it processes the edits, so its copy of the window sees them in
    /// the order the client did. A joined client applies no event while it
    /// holds edits it has not sent (`window_ops::WindowInbox`), so every edit
    /// of one input was made at this one count.
    #[serde(default)]
    pub events_applied: u32,
    /// v76 (C3a-fix-1, C-M1) — parallel to `block_changes`: the hotbar slot
    /// and held item ([`EditHand`]) each edit was made with, recorded when
    /// the edit was made. The server charges a placement to that slot,
    /// wears a break's tool there and classifies the edit by that hand; an
    /// edit past the end of this list (or with a slot of 9 or more) falls
    /// back to the input-level `hotbar_slot` and `held_kind`/`held_id`.
    #[serde(default)]
    pub edit_hands: Vec<EditHand>,
}

/// v76 (C3a-fix-1) — one edit's hand ([`InputPacket::edit_hands`]): the
/// hotbar slot it was made at (below 9), then the held item's `ItemRef`
/// wire pair (`item_kind`, id), as `InputPacket.held_kind`/`held_id`
/// encode it.
pub type EditHand = (u8, u8, u16);

/// A local column whose generation differed from the server's (v71, Phase
/// B2b): which one, the hash the server's note carried and the hash of the
/// client's own generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnMismatch {
    pub cx: i32,
    pub cz: i32,
    /// `ColumnLocalPacket::hash`: the server's column, as generation makes it.
    pub server_hash: u32,
    /// The same hash over the client's own scratch generation of the column.
    pub client_hash: u32,
}

/// Most [`MinedBlock`]s the server reads from one `InputPacket` (a survival
/// break takes at least a tick, so one is the norm) — its DoS guard. An
/// honest client never sends more (it holds the edits past the limit back for
/// its next input, tags and all), so only a modified client's extra tags are
/// ever ignored.
pub const MAX_MINED_PER_INPUT: usize = 16;

/// A block a joiner mined (v72, C1): its cell and the tool in hand for the
/// strike (`WireItem::Tool`, or `WireItem::None` for a bare hand or a
/// non-tool).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinedBlock {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub tool: WireItem,
}

/// Most [`ChunkDrop`]s one `InputPacket` carries (12 bytes each).
pub const MAX_CHUNK_DROPS_PER_INPUT: usize = 512;

/// A column a joiner discarded (v69, Phase B2a). `as_of` is its `chunk_ack`
/// count when it did: every `ChunkData` packet up to that count that touched
/// the column is gone from the client; anything pushed after it is held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkDrop {
    pub cx: i32,
    pub cz: i32,
    pub as_of: u32,
}

// ─── State sync (Server → Client, every tick, unreliable datagram) ───

/// Bit flags carried in `PlayerState.flags` describing transient avatar state.
/// Wire-stable bit positions; append only.
pub mod player_flags {
    pub const SWINGING: u8 = 1;
    pub const CROUCHING: u8 = 2;
    pub const ON_GROUND: u8 = 4;
}

/// Per-player state as seen by other clients.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerState {
    pub player_index: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub held_kind: u8,  // item_kind::*
    pub held_id: u16,   // id within kind
    pub anim_state: u8, // locomotion: 0 idle, 1 walk, 2 jump (crouch is player_flags::CROUCHING, can combine)
    pub flags: u8,      // bit0 swinging, bit1 crouching, bit2 on_ground
    /// Skin reference: a `u64` content hash of this player's skin
    /// (`CosmeticDescriptor::skin_key`). `0` = the bundled default skin
    /// (sentinel); any other value identifies a custom skin for matching/
    /// diffing. This carries only the REFERENCE — the skin bytes are delivered
    /// out-of-band (gated; no native upload source today, so native broadcasts
    /// currently send `0`). Foundation for a networked/Signet-verified skin path.
    pub skin_key: u64,
}

/// Item-kind discriminator for the tool-capable held-item wire encoding.
/// Wire-stable: never renumber, append only.
pub mod item_kind {
    pub const EMPTY: u8 = 0;
    pub const BLOCK: u8 = 1;
    pub const TOOL: u8 = 2;
    pub const MATERIAL: u8 = 3;
    /// C3b-1 — reserved for [`super::WireStack`]: a Plan the receiver holds
    /// only as a placeholder it can't take. A Plan's body has no wire form
    /// (it can reach about 160 KB), so a host's Plan in a shared container
    /// reaches a joiner as this kind (id 0) and decodes to a body-less
    /// stand-in (`plan::PlanData::placeholder`).
    pub const PLAN: u8 = 4;
}

/// A reference to an item held by a player, encoded on the wire as a
/// `(kind, id)` pair. `Empty` is canonical `(0, 0)`. This makes the held
/// item tool-capable instead of block-only.
// NOTE: intentionally NOT Serialize/Deserialize — the wire form is always the (held_kind: u8, held_id: u16) pair via to_wire/from_wire, never a bincode enum discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemRef {
    Empty,
    Block(u16),
    Tool(u16),
    Material(u16),
}

impl ItemRef {
    /// Encode to the `(kind, id)` wire pair.
    pub fn to_wire(self) -> (u8, u16) {
        match self {
            ItemRef::Empty => (item_kind::EMPTY, 0),
            ItemRef::Block(id) => (item_kind::BLOCK, id),
            ItemRef::Tool(id) => (item_kind::TOOL, id),
            ItemRef::Material(id) => (item_kind::MATERIAL, id),
        }
    }

    /// Decode from a `(kind, id)` wire pair. Unknown kinds decode to `Empty`.
    pub fn from_wire(kind: u8, id: u16) -> Self {
        match kind {
            item_kind::BLOCK => ItemRef::Block(id),
            item_kind::TOOL => ItemRef::Tool(id),
            item_kind::MATERIAL => ItemRef::Material(id),
            _ => ItemRef::Empty,
        }
    }
}

/// Full-fidelity item payload (death-drops phase 3, v61). Rides alongside the
/// lossy `(item_kind, item_id)` pair on `EntitySpawn` and
/// `InventoryGrantPacket`, carrying the per-instance state the pair throws
/// away: a tool's type + material + remaining durability, and an armour
/// piece's slot + material + durability.
///
/// **Bincode-positional — append-only forever**, exactly like `EntityKind`.
/// `None` MUST stay variant 0 so a payload that carries no extra fidelity
/// (mobs, carts, block/material drops) encodes as the cheapest discriminant.
/// `Plan` is a deliberate FUTURE append — `plan::PlanData` is heavy and plans
/// stay floor-bound for now.
///
/// The inner fields are plain `u8`s, not the gameplay enums, so a newer peer's
/// unknown tool type or armour tier decodes to "unrecognised" rather than a
/// bincode error. The u8 to enum mapping lives in
/// `inventory::item_to_wire_full` / `inventory::item_from_wire_full` as an
/// explicit match in both directions (no `as`-casts), and an unrecognised byte
/// refuses the whole item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireItem {
    /// No extra fidelity — fall back to the `(item_kind, item_id)` pair.
    #[default]
    None,
    Tool {
        tool_type: u8,
        material: u8,
        durability: u16,
    },
    Armour {
        slot: u8,
        material: u8,
        durability: u16,
    },
}

/// A block change in the world.
///
/// `meta` carries the per-block metadata byte (Spec 48 — facing/state/aux) so
/// directional and stateful blocks (levers, gates, powered rail, lamps) render
/// correctly on the client. `0` for every plain block — the common case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockChange {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub new_block: u16,
    pub meta: u8,
}

impl BlockChange {
    /// A change carrying an explicit metadata byte.
    ///
    /// The ONLY constructor, on purpose. There used to be a `new` that filled
    /// in `meta: 0` for you, and every caller that reached for it was a caller
    /// that had not thought about the byte — which is how a joiner's Logic Gate
    /// ended up facing north on the host, and how the power sim's lit-twin
    /// swaps wiped a Windmill's facing for everyone. Callers that hold a
    /// `World` should not use this directly either: `game_loop::broadcast_change`
    /// (client) and `power::swap_and_broadcast` (sim) read the byte back off
    /// the world, which is the only way to be sure it is right. A literal `0` is
    /// correct for AIR and for a test fixture, and should be conspicuous
    /// anywhere else.
    pub fn with_meta(x: i32, y: i32, z: i32, new_block: u16, meta: u8) -> Self {
        BlockChange { x, y, z, new_block, meta }
    }
}

/// Client → Server: "I right-clicked the power device in this cell."
///
/// The sibling of a block change, and deliberately much thinner than one: a
/// block change asserts the resulting block AND its metadata byte, because the
/// client is the only one that knows what the player was holding. A device
/// interaction asserts nothing at all. The host looks up the device standing at
/// `pos`, decides what a right-click means for that kind (latch a Lever, pulse
/// a Button, wind a Hand Crank, rotate a Mirror) via the same
/// `power::interact_device` the single-player client calls, and broadcasts the
/// result on the normal block-change path. A client that lies can only ask for
/// a cell — never for an outcome.
///
/// 12 bytes on the wire; `safe_deserialize`'s `MAX_PACKET_SIZE` cap applies as
/// it does to every other packet. Fuelling a Steam Generator is NOT on this
/// packet: taking the fuel would have to come out of the server's copy of the
/// player's inventory, and remote inventories are still client-authoritative
/// (CLAUDE.md known debt). See `power::interact_device`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DeviceInteractPacket {
    /// The block cell the player right-clicked, in world coordinates.
    pub pos: (i32, i32, i32),
}

// ─── Joiners act on the server's entities (MP-D2b, v70) ───────

/// Client → Server: swing at the server entity `entity` (MP-D2b).
///
/// Everything here is the client's word, and the server believes only what it
/// cannot check yet: the item in hand (`held_*`, BRIDGE until phase C makes a
/// joiner's inventory server-authoritative). The target must exist and be
/// alive, its centre within `combat::ATTACK_REACH` +
/// `hosted_server::ATTACK_REACH_TOLERANCE` of the eye of the body the SERVER
/// holds, and the swing off the server's cooldown. A critical hit is the
/// server's call too (its body airborne), so the packet carries no crit flag.
/// Damage, knockback, the sweep and `LastAttacker` are `combat::strike`, the
/// code single-player runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityAttackPacket {
    /// Client request number, echoed in the `InteractOutcome`. Per
    /// connection, any value; the client matches its own pending list.
    pub seq: u32,
    /// The target's `ProtocolId` (from the entity diff).
    pub entity: u32,
    /// The held item, `ItemRef` wire pair + full fidelity (`WireItem`), as
    /// `InventoryGrantPacket` encodes it. Empty hand = `item_kind::EMPTY`.
    pub held_kind: u8,
    pub held_id: u16,
    pub held_full: WireItem,
    /// Sprinting: the extra knockback.
    pub sprint: bool,
    /// Sneaking: a deliberate hit on the player's own pet (no friendly-fire
    /// shield), as in single-player.
    pub sneak: bool,
    /// v76 (C3a-fix-1, C-L2) — the hotbar slot the weapon was in when the
    /// swing was made. The server wears its copy of the weapon there if that
    /// slot still holds it, else the first slot that does
    /// (`joiner_actions::where_now`) — the slot the client's own wear
    /// starts from.
    pub hotbar_slot: u8,
    /// v76 — the highest window event the client had applied when it sent
    /// this ([`InputPacket::events_applied`]).
    pub events_applied: u32,
}

/// What an [`EntityInteractPacket`] asks for (MP-D2b). Wire-stable, append
/// only. Riding and trading are not here (D2c).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InteractKind {
    /// The species' breeding food on an adult: love mode.
    Feed,
    /// Taming food (companion food, a Cat Treat, a Bone, Mixed Berries).
    Tame,
    /// Shears on a sheep.
    Shear,
    /// A bucket on a cow.
    Milk,
    /// A Lead on a passive mob.
    LeadAttach,
    /// Take a Lead off a tethered mob (it comes back via `InventoryGrant`).
    LeadDetach,
    /// Empty hand on the player's OWN pet: sit / follow (wolf, Nostrich) or
    /// the companion's command cycle.
    SitToggle,
    /// A Lead on the fence post at `post` (review D2b B3): the player's own
    /// leashed mob nearest the post (within 4 blocks) is tied to it instead,
    /// and a Lead is used, as in single-player. Names no entity — the
    /// request's `entity` is ignored; the server picks the mob.
    LeadToPost { post: [i32; 3] },
}

/// Client → Server: a one-shot right-click on the server mob `entity`
/// (MP-D2b). Validated like [`EntityAttackPacket`] (alive, reach from the
/// server's body, rate), then run through `mob_interact`, the functions
/// single-player's right-click runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityInteractPacket {
    pub seq: u32,
    pub entity: u32,
    pub kind: InteractKind,
    /// The held item (see [`EntityAttackPacket::held_kind`]).
    pub held_kind: u8,
    pub held_id: u16,
    pub held_full: WireItem,
    /// The hotbar slot the item is in — echoed so a client can tell which
    /// stack an accepted outcome consumes from (the server does not read it).
    pub hotbar_slot: u8,
    /// Sneaking (the horse family's breeding feed is a sneak gesture).
    pub sneak: bool,
    /// v76 — the highest window event the client had applied when it sent
    /// this ([`InputPacket::events_applied`]).
    pub events_applied: u32,
}

/// What an [`ItemActionPacket`] asks for (C2a; C2b `Craft` and `Drop`; v76
/// `GrantUnfit`). Wire-stable, APPEND ONLY: Eat = 0, Sleep = 1, Craft = 2,
/// Drop = 3, GrantUnfit = 4 (pinned on the wire bytes by
/// `item_action_packets_round_trip`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ItemAction {
    /// Eat one of the food in hotbar slot `hotbar_slot`. The held claim
    /// mirrors [`EntityInteractPacket`]'s (`held_kind` / `held_id` pair plus
    /// `held_full`): the food is the client's word until the shadow
    /// inventory is enforced (C3); the server takes it from its shadow.
    Eat { hotbar_slot: u8, held_kind: u8, held_id: u16, held_full: WireItem },
    /// Sleep in the bed at `bed`: the server checks the bed, its reach from
    /// the server body, the night and once a night, then sets the spawn point
    /// there and heals the body to full. It never skips the night.
    Sleep { bed: [i32; 3] },
    /// C2b (v74) — the client crafted once from this grid (row-major, as it
    /// stood BEFORE the craft consumed it), each cell an `(item_kind,
    /// item_id)` pair; `table` is the crafting table the 3×3 grid was opened
    /// from. **Unused since v75 (C3a-2a):** the craft is the window's result
    /// click, sent as a `WindowOp`. Kept because this enum is append-only; a
    /// v75 client never sends it, and a v75 server ignores it and tallies it
    /// (`PossessionTally::crafts_ignored`).
    Craft { grid: [(u8, u16); 9], table: Option<[i32; 3]> },
    /// C2b — the client Q-dropped one of the item in hotbar slot
    /// `hotbar_slot` (the held claim mirrors `Eat`'s). It spawned nothing
    /// itself: the server spawns the claimed item, full fidelity from
    /// `held_full`, as a real ground item everyone sees. Fire-and-forget,
    /// paced by the server's drop bucket (`item_actions::DropBucket`).
    Drop { hotbar_slot: u8, held_kind: u8, held_id: u16, held_full: WireItem },
    /// v76 (C3a-fix-1, D-M2) — `count` of the stack granted by window event
    /// `event` (an `InventoryGrant`) didn't fit the client's inventory. The
    /// client spills nothing itself: the server takes that part back out of
    /// its copy of the window and spawns it as a real ground item at the
    /// joiner's feet, which everyone can see and pick up. Never more than the
    /// grant gave (the server clamps it). Fire-and-forget (no outcome).
    GrantUnfit { event: u32, count: u8 },
}

/// Wire index of an [`ItemAction`] variant the server reads before decoding
/// (bincode writes it as a `u32` right after the packet's `seq`,
/// [`peek_item_action_variant`]). The order is Eat = 0, Sleep = 1, Craft = 2,
/// Drop = 3, GrantUnfit = 4 (v76), pinned by `item_action_packets_round_trip`.
pub mod item_action_variant {
    /// `ItemAction::Drop`, paced by the joiner's drop bucket.
    pub const DROP: u32 = 3;
}

/// C2b — the variant index of an `ItemActionPacket` payload without decoding
/// it ([`item_action_variant`]): the `u32` after the leading `seq`. `None`
/// when the payload is shorter than that.
pub fn peek_item_action_variant(payload: &[u8]) -> Option<u32> {
    payload.get(4..8).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Client → Server: one item action (C2a, `PacketType::ItemAction`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemActionPacket {
    /// The client's request number, echoed in the outcome. Shares its
    /// sequence with `EntityAttack` / `EntityInteract` (`joiner_actions`).
    /// A `Craft`, `Drop` or `GrantUnfit` takes a number too, and is never
    /// answered.
    pub seq: u32,
    pub action: ItemAction,
    /// v76 — the highest window event the client had applied when it sent
    /// this ([`InputPacket::events_applied`]).
    pub events_applied: u32,
}

/// One window op (C3a-2a, [`WindowOpPacket`]). Wire-stable, APPEND ONLY:
/// Click = 0, OpenPlayer = 1, OpenTable = 2, SetAutoRefill = 3, (C3b-1)
/// OpenContainer = 4, Container = 5 (pinned by `window_op_packets_round_trip`
/// and `container_packets_round_trip`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WireWindowOp {
    /// A click on the window (`window::WindowClick`, itself append-only):
    /// the server applies `window::apply` to its copy, at its station, from
    /// its body.
    Click(crate::window::WindowClick),
    /// The client opened its own 2×2 inventory screen (E): the server's
    /// station is the player's grid.
    OpenPlayer,
    /// The client opened the crafting table at `cell` (only while it is in
    /// reach, `window::table_in_reach`): the server's station is that table.
    OpenTable { cell: [i32; 3] },
    /// The client's auto-refill setting (`Inventory::auto_refill`), sent at
    /// join and whenever it changes: a placement refills the hotbar on both
    /// sides by the same rule.
    SetAutoRefill { on: bool },
    /// C3b-1 — the client right-clicked the container at `cell` (a chest of
    /// any tier, a dispenser or dropper, a furnace lit or not) and asks to
    /// open the server's real one. Nothing opens until the server answers
    /// with [`ContainerOpenedPacket`]; the window doesn't change.
    OpenContainer { cell: [i32; 3] },
    /// C3b-1 — a click on the open container's screen
    /// (`container_window::ContainerClick`, itself append-only): the client
    /// applied `container_window::apply_container` to its mirror and its
    /// window, and the server applies the same rule to the real container
    /// and its copy of the window. The digest covers the container.
    Container(crate::container_window::ContainerClick),
}

/// C3b-1 — the most slots one container shows: the largest chest tier
/// (`chest::ChestTier::Satori`, 8 rows of 9).
pub const MAX_CONTAINER_SLOTS: usize = 72;

/// C3b-1 — the most slots one [`WindowSlotSetPacket`] names (and one
/// container op reports touched): a whole container, the 36 slots, the 4
/// armour slots, the cursor and the 9 grid cells.
pub const MAX_WINDOW_SLOTS: usize = MAX_CONTAINER_SLOTS + 36 + 4 + 1 + 9;

/// C3b-1 — one slot of a joiner's window, as a correction or a push names
/// it, and as a container op reports it touched. Wire-stable, APPEND ONLY:
/// Inv = 0, Armour = 1, Cursor = 2, Grid = 3, Container = 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WireWindowSlot {
    /// Inventory slot 0..36.
    Inv(u8),
    /// Armour slot `ArmourSlot as usize`, 0..4.
    Armour(u8),
    /// The stack on the cursor.
    Cursor,
    /// Crafting-grid cell `(row, col)`.
    Grid(u8, u8),
    /// Slot of the open container: a chest's index, or a furnace's input (0),
    /// fuel (1) and output (2).
    Container(u8),
}

/// C3b-1 — a full-fidelity stack on the wire: the `(item_kind, item_id)`
/// pair (`inventory::item_to_ref`), its count, and the tool/armour state the
/// pair loses (`WireItem`, which wins on decode, as `InventoryGrantPacket`'s
/// does). `item_kind::PLAN` is reserved for a Plan placeholder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireStack {
    pub item_kind: u8,
    pub item_id: u16,
    pub count: u8,
    pub full_item: WireItem,
}

/// C3b-1 — one window or container slot on the wire: empty, or a stack.
pub type WireSlot = Option<WireStack>;

/// C3b-1 — a furnace's progress as its screen draws it (the bars), sent with
/// [`ContainerOpenedPacket`] and with each push while the furnace cooks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FurnaceView {
    pub smelt_progress: u32,
    pub smelt_total: u32,
    pub fuel_ticks_remaining: u32,
    pub lit: bool,
}

impl FurnaceView {
    /// The progress `furnace` shows.
    pub fn of(furnace: &crate::furnace::FurnaceData) -> Self {
        FurnaceView {
            smelt_progress: furnace.smelt_progress,
            smelt_total: furnace.smelt_total,
            fuel_ticks_remaining: furnace.fuel_ticks_remaining,
            lit: furnace.lit,
        }
    }

    /// Show this progress on a mirror `furnace` (its slots stay as they are).
    pub fn apply_to(self, furnace: &mut crate::furnace::FurnaceData) {
        furnace.smelt_progress = self.smelt_progress;
        furnace.smelt_total = self.smelt_total;
        furnace.fuel_ticks_remaining = self.fuel_ticks_remaining;
        furnace.lit = self.lit;
    }
}

/// C3b-1 — why a container didn't open for a joiner. Wire-stable, APPEND
/// ONLY: OutOfReach = 0, Protected = 1, NotAContainer = 2, NotInWorld = 3.
/// The client toasts "You can't open that here." for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenRefusal {
    /// Beyond the block reach of the server's body
    /// (`item_actions::cell_in_reach`).
    OutOfReach,
    /// The world's play mode forbids it, or it stands in a plot the joiner
    /// doesn't own (`HostedServer::remote_may_touch`).
    Protected,
    /// The cell holds no chest, dispenser, dropper or furnace.
    NotAContainer,
    /// The joiner isn't in the world, or is dead.
    NotInWorld,
}

/// Server → Client (C3b-1, `PacketType::ContainerOpened`): the answer to
/// `WireWindowOp::OpenContainer`. Opened: `kind` and `slots` are the server's
/// real container at `cell` (a furnace's slots are input, fuel and output,
/// with `furnace` its progress), and the client's screen opens on a mirror of
/// them. Refused (`refused` set): `kind`, `slots` and `furnace` mean nothing,
/// and nothing opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContainerOpenedPacket {
    pub cell: [i32; 3],
    pub kind: crate::container_window::ContainerKind,
    /// At most [`MAX_CONTAINER_SLOTS`] (more doesn't decode).
    #[serde(deserialize_with = "bounded_container_slots")]
    pub slots: Vec<WireSlot>,
    pub furnace: Option<FurnaceView>,
    pub refused: Option<OpenRefusal>,
}

/// C3b-1 — `WindowSlotSetPacket::reason` values.
pub mod slot_set_reason {
    /// A container op's result differed from the client's (someone else got
    /// there first, or the server refused it): the REAL values of the
    /// container slots it involved, and the server's re-run of the op over
    /// the client's claimed pre-op player slots
    /// (`WindowOpPacket::claims`) for each player slot whose result differs
    /// from the client's own prediction — never the server's drifted copy.
    pub const CORRECTION: u8 = 0;
    /// What changed in the open container since the last push, made by
    /// anyone but this joiner's own ops: another player, a hopper, the
    /// furnace's cooking, a host's click on its lent world.
    pub const CHANGED: u8 = 1;
}

/// Server → Client (C3b-1, `PacketType::WindowSlotSet`): values for exactly
/// the named slots of the joiner's window (§3 rule 7: never a whole-window
/// overwrite, which would revert local uses not mirrored yet). The client
/// overwrites those slots and nothing else; nothing is replayed, and a later
/// mismatch is corrected again. It is applied in arrival order with the
/// other window-event carriers (`window_events::WindowInbox`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSlotSetPacket {
    /// The `op_seq` of the last window op from this client the server had
    /// applied when it sent this.
    pub op_seq_applied: u32,
    /// [`slot_set_reason`]: `CORRECTION` or `CHANGED`.
    pub reason: u8,
    /// At most [`MAX_WINDOW_SLOTS`] (more doesn't decode).
    #[serde(deserialize_with = "bounded_slot_sets")]
    pub sets: Vec<(WireWindowSlot, WireSlot)>,
    /// The open furnace's progress, when the open container is a furnace.
    pub furnace: Option<FurnaceView>,
    /// v77 — a set that changes this joiner's PLAYER slots (inventory,
    /// armour, cursor, grid) is a numbered window event (`window_events`,
    /// as `InventoryGrantPacket::window_event`): the client applies it in
    /// arrival order with the other carriers, and the server applies its
    /// side when the client reports it (`events_applied`). 0 for a set of
    /// container slots only (a push, or a container-only correction): shared
    /// state, applied at once.
    pub window_event: u32,
}

fn bounded_container_slots<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<WireSlot>, D::Error> {
    let slots = Vec::<WireSlot>::deserialize(d)?;
    if slots.len() > MAX_CONTAINER_SLOTS {
        return Err(serde::de::Error::invalid_length(slots.len(), &"at most 72 container slots"));
    }
    Ok(slots)
}

fn bounded_slot_sets<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<(WireWindowSlot, WireSlot)>, D::Error> {
    let sets = Vec::<(WireWindowSlot, WireSlot)>::deserialize(d)?;
    if sets.len() > MAX_WINDOW_SLOTS {
        return Err(serde::de::Error::invalid_length(sets.len(), &"at most 122 window slots"));
    }
    Ok(sets)
}

fn bounded_window_slots<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<WireWindowSlot>, D::Error> {
    let touched = Vec::<WireWindowSlot>::deserialize(d)?;
    if touched.len() > MAX_WINDOW_SLOTS {
        return Err(serde::de::Error::invalid_length(touched.len(), &"at most 122 touched slots"));
    }
    Ok(touched)
}

/// Client → Server: one window op (C3a-2a, `PacketType::WindowOp`). A joined
/// client sends one for every window transition it applies, in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowOpPacket {
    /// 1, 2, 3… per connection (`RemoteClient`): which op this is.
    pub op_seq: u32,
    pub op: WireWindowOp,
    /// The client's window digest after applying the op
    /// (`window::digest`). The server compares its copy's (log-only).
    pub digest: u32,
    /// v76 (C3a-fix-1) — the highest server window event the client had
    /// applied when it applied this op: the server applies its queued
    /// events up to it first ([`InputPacket::events_applied`]).
    pub events_applied: u32,
    /// C3b-1 (v77) — for a `Container` op, the slots the client's own apply
    /// changed (`container_window::ContainerApplied::touched`); empty for
    /// every other op. A mismatched container op's correction covers the
    /// container slots among these as well as the ones the server's apply
    /// changed. At most [`MAX_WINDOW_SLOTS`].
    #[serde(deserialize_with = "bounded_window_slots")]
    pub touched: Vec<WireWindowSlot>,
    /// C3b-1 (v77) — for a `Container` op, the client's values BEFORE the op
    /// of the player slots it acts on (`container_window::claim_slots`: the
    /// ones it touched, the ones the click names, and all 36 for Restock,
    /// whose rule reads them all), at full fidelity; empty for every other
    /// op. The server re-runs the op over these slots and the REAL container
    /// (`window_ops::serve_op`), so a correction of a player slot is
    /// relative to the client's own state, never to the server's drifted
    /// copy. Believed while the mirror is log-only (C3d refuses). At most
    /// [`MAX_WINDOW_SLOTS`].
    #[serde(deserialize_with = "bounded_slot_sets")]
    pub claims: Vec<(WireWindowSlot, WireSlot)>,
}

/// Server → Client: the decision on one [`ItemActionPacket`] (C2a).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemActionOutcomePacket {
    /// The request's `seq`.
    pub seq: u32,
    /// It happened. A refused request changes nothing.
    pub accepted: bool,
    /// Items the client takes from what it claimed (accepted only): 1 for an
    /// eaten food, 0 for a sleep.
    pub consume_held: u8,
    /// Why it was refused (an `item_actions::ItemNote` code), 0 = nothing.
    /// Unknown codes are shown as nothing.
    pub note: u8,
    /// v76 (C3a-fix-1) — the window event this outcome's take is
    /// ([`InventoryGrantPacket::window_event`]); 0 when it changes nothing
    /// in the window (a refusal, a sleep).
    pub window_event: u32,
}

/// Server → Client: the decision on one attack or interaction (MP-D2b).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteractOutcomePacket {
    /// The request's `seq`.
    pub seq: u32,
    /// The request's entity.
    pub entity: u32,
    /// `None` = an `EntityAttack`'s outcome; otherwise the interaction's kind.
    pub kind: Option<InteractKind>,
    /// Attack: the swing was valid and spent (target alive and in reach, off
    /// cooldown) — the client wears its weapon, as single-player wears it on
    /// any swing that found a target, invulnerability frames or not.
    /// Interaction: it happened. A refused request changes nothing.
    pub accepted: bool,
    /// Items the client takes from the held stack (accepted only). A
    /// product (milk, the Lead back) rides `InventoryGrant`: a bucket→milk
    /// swap is `consume_held = 1` plus one grant.
    pub consume_held: u8,
    /// What to tell the player: a `mob_interact::InteractNote` code, 0 =
    /// nothing. Unknown codes are shown as nothing.
    pub note: u8,
    /// v76 (C3a-fix-1) — the window event this outcome is
    /// ([`InventoryGrantPacket::window_event`]): an accepted interaction's
    /// take, or an accepted swing's weapon wear; 0 when it changes nothing in
    /// the window (a refusal, nothing used, no tool in hand).
    pub window_event: u32,
}

/// Server → Client: a kill this player made (MP-D2b), to the killer alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KillEventPacket {
    /// The victim's species.
    pub victim: EntityKind,
    /// Why the kill was credited to this player: [`kill_reason`]. (A mob
    /// keeps no record of what dealt its last point of damage — a cow a
    /// joiner hit may die later in lava — so this names the credit rule,
    /// not the killing blow; review D2b LOW-6.)
    pub reason: u8,
    /// Where it died.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// [`entity_flags`] of the victim as it died (`TAMED`: it was somebody's
    /// pet — a tamed Nostrich's death does not bring the Vow).
    pub victim_flags: u8,
}

/// `KillEventPacket.reason` codes. Wire-stable, append only.
pub mod kill_reason {
    /// This player's hit was the last any player landed on it (a swing or
    /// its sweep), whatever finished it off.
    pub const LAST_HIT: u8 = 0;
    /// No player hit it: this player was the nearest living one (the
    /// single-player rule for environment, mob-on-mob and pet kills).
    pub const NEAREST: u8 = 1;
}

/// A death cause on the wire (`PlayerEventType::DiedOf`, MP-D2b): the
/// `survival::DamageCause` the death screen names. Wire-stable, append only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireDamageCause {
    #[default]
    Generic,
    Fall,
    Drowning,
    Starvation,
    Lava,
    Fire,
    Explosion,
    Mob(EntityKind),
}

/// Entity kind discriminator on the wire. Matches `MobKind` subset the engine
/// currently spawns. New kinds append; never renumber (wire-stable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EntityKind {
    // NOTE: discriminants 0/5/6/7/8/10/18 held the 7 retired fantasy-roster
    // mobs, excised in v39 (2026-05-24, BREAKING) for open-source cleanup.
    // The gaps are intentional — kept variants retain their original
    // discriminant so the wire value for surviving mobs is unchanged.
    Cow = 1,
    Chicken = 2,
    Pig = 3,
    Sheep = 4,
    Villager = 9,
    WanderingVillager = 11,
    Wolf = 12,
    Horse = 13,
    Rabbit = 14,
    Goat = 15,
    Bee = 16,
    Squid = 17,
    // Spec 28d.nostrich — purple ostrich, the Nostr mascot. Savanna
    // passive with retaliate-kick AI. Appended at id 19 (wire-stable).
    Nostrich = 19,
    // HP-2 (2026-05-22) — Forest/Taiga territorial omnivore. Appended at
    // id 20.
    Bear = 20,
    // HP-2 — Savanna pack hunter. Appended at id 21.
    Hyena = 21,
    // HP-3 (2026-05-23) — three new human-tier mob discriminants.
    // Brigand / Marauder / Berserker spawn out of Brigand Hideouts.
    Brigand = 22,
    Marauder = 23,
    Berserker = 24,
    // HP-4 (2026-05-23) — Knight, human village defender that replaces
    // the Iron Golem post-HP-6 cutover. Ships alongside the Golem in
    // HP-4 during the transition window.
    Knight = 25,
    // Rail freight Phase 1 (2026-06-10) — minecart. A track-driven entity,
    // NOT a mob (no MobKind, no AI). Broadcast so a cart renders on other
    // clients; carries the cart's lerped Position + `CartData.facing` yaw.
    // health is unused (carts aren't combat entities → wire health 0).
    Cart = 26,
    // Aquatic wave (2026-06-22) — three new ocean species, appended wire-stable.
    Fish = 27,
    Shark = 28,
    GlowSquid = 29,
    // Wild fauna wave (2026-06-22) — appended wire-stable.
    Fox = 30,
    PolarBear = 31,
    Reindeer = 32,
    // Companions wave (2026-06-22) — appended wire-stable.
    Cat = 33,
    Parrot = 34,
    // Logistics wave (2026-06-22) — appended wire-stable.
    Donkey = 35,
    Mule = 36,
    // Pets wave Task 13 (2026-07-06) — coastal Crab, appended wire-stable.
    Crab = 37,
    // Death-drops phase 2 (2026-07-11, v57) — a dropped-item entity
    // (server-side loot from death_drops.rs). Not a mob: no AI, no health;
    // the stack it carries rides EntitySpawn's item_* fields.
    Item = 38,
    // MP-A3 (2026-10-06, v67) — a projectile in flight (`ProjectileEntity`:
    // a dispenser's arrow on the dedicated server today). Not a mob: no AI,
    // no health. `yaw` is its flight heading; `EntityUpdate.state` is 0 for
    // an arrow, 1 for a blunt slingshot ball. Render-only on the joiner.
    Projectile = 39,
}

/// Server → client: a new entity appeared. Sent in the tick the entity spawns.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntitySpawn {
    pub id: u32,
    pub kind: EntityKind,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub health: u16,
    /// Death-drops phase 2 (2026-07-11, v57) — the stack a `kind == Item`
    /// spawn carries, in the `ItemRef` wire encoding (`item_kind::*`
    /// discriminator + id) plus a count. All zero for mobs/carts.
    pub item_kind: u8,
    pub item_id: u16,
    pub item_count: u8,
    /// Death-drops phase 3 (2026-09-06, v61) — full-fidelity payload for a
    /// `kind == Item` spawn whose stack the `(item_kind, item_id)` pair can't
    /// express (tools, armour). `WireItem::None` for mobs, carts, and
    /// block/material drops. Appended last; see `InventoryGrantPacket.full_item`
    /// on why `serde(default)` is documentation, not back-compat.
    #[serde(default)]
    pub full_item: WireItem,
}

/// Bit flags carried in `EntityUpdate.flags` (MP-D2a, v68): the per-entity
/// render state a joiner's mirror draws. Wire-stable bit positions; append
/// only.
pub mod entity_flags {
    /// The mob's damage flash is showing (`combat::Health::is_flashing`).
    pub const HURT: u8 = 1;
    /// A juvenile (`breeding::Baby`) — drawn at the baby scale.
    pub const BABY: u8 = 2;
    /// Somebody's tamed pet or kept steed (`tameable::pet_owner_of`). A
    /// joiner reads it to offer the sit / follow command on an empty-hand
    /// right-click (D2b; the server checks the pet is the joiner's own).
    pub const TAMED: u8 = 4;
    /// Satoshi the guide (`satoshi::SatoshiMarker`) — a Villager drawn with
    /// his own hooded model.
    pub const SATOSHI: u8 = 8;
    /// MP-D2b (v70) — on a Lead (`tether::Tethered`): a right-click that
    /// isn't feeding, milking, shearing or companion food takes the Lead off,
    /// as in single-player.
    pub const TETHERED: u8 = 16;
    /// FU3 (FU1 verify N8, NO version bump) — a cow that can't be milked yet
    /// or a sheep whose wool is growing back (`mob_interact::product_ready`,
    /// on the server's clock). A joiner's bucket or shears on it is then no
    /// mob action (`remote_mobs::MirrorTarget::right_click_action`): the
    /// click goes on to the block — a bucket fills at water beside a cow just
    /// milked. 0 means ready OR unknown, so an older server (never sets it)
    /// and an older joiner (ignores it) behave as they did.
    pub const PRODUCT_NOT_READY: u8 = 32;
}

/// Server → client: an existing entity's position/state changed.
///
/// **Changed-only (MP-D2a, v68):** sent when the entity's state differs from
/// the last update broadcast for it (position, velocity or yaw beyond a small
/// epsilon, or a `state`/`flags` change), and once when the entity enters a
/// client's interest radius (alongside its `EntitySpawn`). An entity that
/// sends nothing has not changed; the client keeps the last update it got.
/// `state` is an AI-state tag for animation (idle=0, wander=1, chase=2,
/// attack=3); for a projectile 0 = arrow, 1 = blunt ball.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityUpdate {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub state: u8,
    /// MP-D2a (v68) — velocity in blocks per tick (`entity::Velocity`), for
    /// the walk cycle and to smooth the motion between updates. Zero for an
    /// entity with no velocity (a cart). APPEND-ONLY with `flags`. The
    /// `serde(default)`s on these four document intent only — the join-time
    /// version gate is the real back-compat (see `InputPacket.armour_points`).
    #[serde(default)]
    pub vx: f32,
    #[serde(default)]
    pub vy: f32,
    #[serde(default)]
    pub vz: f32,
    /// MP-D2a (v68) — [`entity_flags`] bits.
    #[serde(default)]
    pub flags: u8,
}

/// State update sent from server to client each tick.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateUpdatePacket {
    /// Server tick number
    pub tick: u64,
    /// All player positions/states
    pub players: Vec<PlayerState>,
    /// Block changes since last ack
    pub block_changes: Vec<BlockChange>,
    /// World time
    pub world_time: u32,
    /// Highest client_seq (= InputPacket.tick) the server has consumed for
    /// the addressed client. Used by the client's prediction+replay loop to
    /// discard already-acked intents. Zero when no intent has been acked.
    pub last_acked_input: u64,
    /// Entities spawned this tick. Client inserts into its render-only ECS.
    pub entity_spawns: Vec<EntitySpawn>,
    /// Entities whose state changed this tick.
    pub entity_updates: Vec<EntityUpdate>,
    /// Entities removed this tick (by id).
    pub entity_despawns: Vec<u32>,
    /// Reserve richness in [0.0, 1.0] — the depth of the server's
    /// Deepslate Reserve relative to its target balance. Drives both
    /// the mining-rate multiplier (Spec 6 §7.2) and the in-world
    /// visual treatment of deepslate (Spec 16). `1.0` = at-or-above
    /// target ("fat"); `0.0` = empty (mining paused).
    pub reserve_richness: f32,
    /// Reserve target (sats). Informational — displayed in the
    /// Transparency Panel alongside richness.
    pub reserve_target_sats: u64,
    /// Reserve current balance (sats). Informational — displayed
    /// alongside richness so players can see how the gauge maps to
    /// the underlying pool. May trail the LNbits balance slightly
    /// (Spec 6 §7.2 syncs every 30 s).
    pub reserve_current_sats: u64,
    /// P9 weather sync (v59) — ticks remaining in the server's rain window,
    /// from `Weather::ticks_left`. A *duration*, not the server's absolute
    /// `rain_until`, so applying it is correct regardless of any offset
    /// between the server's and this client's `tick_counter`. APPEND-ONLY:
    /// must stay LAST alongside `storm_ticks_left` (bincode is positional).
    /// `#[serde(default)]` so a v58 peer's shorter packet still decodes —
    /// zero, which the client reads as clear weather (acceptable degrade).
    #[serde(default)]
    pub rain_ticks_left: u32,
    /// P9 weather sync (v59) — ticks remaining in the server's thunderstorm
    /// window (always `<= rain_ticks_left`'s implied end). See
    /// `rain_ticks_left` for the wire-compat rationale.
    #[serde(default)]
    pub storm_ticks_left: u32,
    /// C2a (v73) — the addressed client's OWN hunger as the server holds it:
    /// the server runs every joiner's metabolism, and the joined client
    /// writes this into its slot each update. Per client (the template is
    /// re-stamped per recipient, like `last_acked_input`); `0` for a host's
    /// local slot, whose hunger is its own client's. APPEND-ONLY: last.
    #[serde(default)]
    pub own_hunger: u8,
}

// ─── Chunk data (Server → Client, reliable stream) ───

/// One chunk the server pushes to a joiner (Phase B2a, v69; Spec 04 §4.1).
/// A snapshot at its place in the client's ordered stream: the client
/// REPLACES whatever it holds there (its own generation included) and
/// applies every later block change on top.
///
/// A chunk whose side data does not fit one packet goes as several: the
/// first carries the blocks (and replaces the chunk and its side data); each
/// CONTINUATION (empty `compressed_blocks`) adds more side data to it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkDataPacket {
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
    /// LZ4-compressed `Chunk::as_bytes()` ([`compress_chunk`]) — 8192 bytes
    /// of u16 block IDs plus the 512-byte player-placed mask = 8704 bytes
    /// uncompressed (Spec 6 §2.2). An all-air chunk is sent too (about 50
    /// bytes): it is how a dug-out chunk reaches the client. EMPTY = a
    /// continuation packet (see the type docs).
    pub compressed_blocks: Vec<u8>,
    /// Per-block metadata in this chunk (`World.block_meta`: shape, facing,
    /// device latch, water depth), `(cell, meta)`, `cell` the chunk-local
    /// index `x + z*16 + y*256`. Zero entries are not sent.
    pub meta: Vec<(u16, u8)>,
    /// The render-visible block entities in this chunk — never a
    /// container's contents, an escrow or a plan.
    pub entities: Vec<PushedBlockEntity>,
    /// Face attachments (wallpaper, blueprints) in this chunk, render stubs.
    pub attachments: Vec<PushedFaceAttachment>,
}

/// "Column `(cx, cz)` is local" (v71, Phase B2b; Spec 04 §4.1 "Touched
/// columns"): every chunk of it is exactly what generation makes from the
/// world's seed and flags, so the joiner generates it itself instead of being
/// pushed it. A snapshot claim at its place in the ordered chunk stream, like
/// a push: block changes after it apply to the joiner's own generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnLocalPacket {
    pub cx: i32,
    pub cz: i32,
    /// `chunk_verdict::column_hash` of the column as generation makes it (its
    /// blocks and placed bits), taken from the scratch its `Untouched` verdict
    /// compared against and cached with that verdict — so it is the server's
    /// live column too, and the note hashes nothing. The joiner checks its
    /// own column against it, and on a difference its own scratch generation;
    /// only a generation that differs lets the column go and asks for
    /// everything to be pushed ([`InputPacket::column_mismatch`]).
    pub hash: u32,
}

/// A render-visible block entity in a pushed chunk (v69). `cell` as in
/// [`ChunkDataPacket::meta`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PushedBlockEntity {
    pub cell: u16,
    pub entity: PushedEntity,
}

/// What a joiner sees of a block entity (v69). APPEND-ONLY enum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PushedEntity {
    /// A sign's text (read on look; at most `sign::SIGN_MAX_CHARS`).
    Sign { text: String },
    /// An item frame's shown item, as the held-item `(kind, id)` pair plus
    /// [`WireItem`] fidelity (a framed plan shows as an empty frame), and
    /// its rotation.
    ItemFrame { item_kind: u8, item_id: u16, full_item: WireItem, rotation: u8 },
    /// A campfire's burn state (the lit/smoke pillar and the raid-warning
    /// smoke tint) — not what is cooking on it.
    Campfire { fuel_ticks: u32, smoke_ticks: u32, smoulder_ticks: u32, raid_warning: bool },
}

/// One face attachment in a pushed chunk (v69). `face` is
/// `mesh::Face::index` (0..6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PushedFaceAttachment {
    pub cell: u16,
    pub face: u8,
    pub attachment: PushedAttachment,
}

/// The render stub of a face attachment (v69). A blueprint travels as its
/// develop state only: the mesher reads nothing else, and the plan is the
/// host's. APPEND-ONLY enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushedAttachment {
    Wallpaper(u16),
    BlueprintBlank,
    Blueprint { developed: bool },
}

// ─── Player events ───

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PlayerEventType {
    /// `name` is the display handle (verified credential name, disambiguated, or
    /// guest fallback). `npub` is the joiner's full verified npub (NIP-19 bech32)
    /// for the client's inspect view, or `""` for a guest / unverified join —
    /// the inspect view lets a specific person be verified beyond the grindable
    /// collision suffix (Phase 4).
    Joined { name: String, npub: String },
    Left,
    /// MP-A3 (v67) — the server's own sim killed this player's body (a fall,
    /// drowning): it now holds them dead — no physics, no pickups, no edits, no
    /// mob targeting — until they send `PacketType::Respawn`. Sent to that
    /// player ALONE (never broadcast); their own client enters its death
    /// screen. NOT sent for a death the joiner's input reported (its client
    /// already knows; an echo after a quick Respawn would kill it again).
    Died,
    /// MP-A3 (v67) — the server respawned this player at the spawn point it
    /// holds for them, after a `Respawn` it held them dead for at least
    /// `server::MIN_DEAD_TICKS_BEFORE_RESPAWN` ticks to honour. Sent to that
    /// player ALONE; their own client moves there (if the position is inside
    /// the join-spawn range).
    Respawned { x: f32, y: f32, z: f32 },
    /// MP-D2b (v70) — `Died`, naming what killed the body (the mob's species,
    /// lava, a fall …) for the death screen and the client's own records.
    /// The server sends this instead of `Died`, under the same rules.
    DiedOf { cause: WireDamageCause },
    /// MP-D2b (v70) — `hits` hits the server landed on this player's body
    /// since the last report wear its armour, one durability per worn piece
    /// per hit (`PlayerSlot::wear_armour`, the single-player rule). Sent to
    /// that player alone, on the tick the hits land.
    ArmourWorn { hits: u8 },
    /// MP-D2b (v70, review B2) — a baby was born to an animal this player
    /// fed (`offspring` = its species), so its client fires the
    /// `BreedAnimals` challenge single-player fires for a breed. Sent to
    /// that player alone; a host's own players are credited by its client.
    Bred { offspring: EntityKind },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerEventPacket {
    pub player_index: u32,
    pub event: PlayerEventType,
    /// v76 (C3a-fix-1) — the window event an `ArmourWorn` is
    /// ([`InventoryGrantPacket::window_event`]); 0 for every other event.
    #[serde(default)]
    pub window_event: u32,
}

// ─── Discovery (LAN broadcast) ───

/// Broadcast by the server on UDP port 7705 every second for LAN discovery.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerAnnouncePacket {
    /// Magic bytes for identification
    pub magic: [u8; 4], // b"AXNS"
    /// Protocol version
    pub protocol_version: u32,
    /// Server port (QUIC)
    pub port: u16,
    /// World/server name
    pub server_name: String,
    /// Current player count
    pub player_count: u8,
    /// Max player count
    pub max_players: u8,
    /// Game mode (legacy projection — kept for consumers that only branch on
    /// creative; new consumers read `play_mode` below).
    pub is_creative: bool,
    /// Full play mode (Spec 05 §8). `is_creative` above is the derived
    /// projection kept for back-compat with consumers that only branch on
    /// creative; new consumers read `play_mode`.
    pub play_mode: crate::play_mode::PlayMode,
}

// ─── Serialization helpers ───

/// Current protocol version. Increment when packet formats change.
///
/// Version history:
/// - v1: initial client-authoritative position relay.
/// - v2 (2026-04-18): `StateUpdatePacket` gains entity spawns/updates/despawns
///   and `last_acked_input` for prediction reconciliation. Adding
///   `EntitySpawn`/`EntityUpdate` structs. See
///   `docs/superpowers/specs/2026-04-18-server-authority-extraction.md`.
/// - v3 (2026-05-03): `JoinRequestPacket` gains `auth_event` and
///   `handle_credential` (Option<SignetAuth*Wire>); new `ChallengePacket`
///   (tag 50) lands on connect. Bincode is positional, so even
///   `Some`/`None`-only adds need a version bump. Phase 3 of
///   `docs/foundations/2026-04-20-engine-signet-auth.md`. (Phase 4 / v48 turned
///   these fields on: a present `auth_event` is verified, absent is rejected on
///   a sign-in-required host.)
/// - v8 (2026-05-13): Wave 25 adds new BlockIds (PURE_DEEPSLATE +
///   3 deepslate ore variants + SATORI_BLOCK = ids 25..=29). Existing
///   clients without these registered would render unknown ids as AIR
///   (the registry fallback), producing voids in chunks that contain
///   deepslate. Bumping forces clean rejection of stale clients.
/// - v9 (2026-05-17): Spec 16 — `StateUpdatePacket` gains the Deepslate
///   Reserve snapshot fields (`reserve_richness: f32`,
///   `reserve_target_sats: u64`, `reserve_current_sats: u64`).
///   Bincode is positional so even appended fields need a version bump.
///   See `docs/foundations/2026-05-17-deepslate-reserve.md`.
/// - v10 (2026-05-17): Wave 26 farming begins — adds TILLED_SOIL = id 30.
///   Stale clients without the registry entry would render tilled soil
///   as AIR; bump forces clean rejection. See foundation
///   `2026-05-14-farming-system.md` Phase 2.
/// - v11 (2026-05-17): Wave 26 farming — adds 12 crop-stage blocks
///   (WHEAT_STAGE_0..3 + CARROT_STAGE_0..3 + POTATO_STAGE_0..3 =
///   ids 31..=42). Same registry-fallback rationale.
/// - v12 (2026-05-18): Wave 27 campfire — adds CAMPFIRE (id 43) and
///   CAMPFIRE_UNLIT (id 44). Same registry-fallback rationale. See
///   foundation `2026-05-18-campfire.md`.
/// - v13 (2026-05-18): Wave 28 campfire extensions — adds CAMPFIRE_SMOKE
///   (id 45) for the friend-signal pillar plus four corn-stage blocks
///   (CORN_STAGE_0..3 = ids 46..=49). See foundation
///   `2026-05-18-campfire-extensions.md`.
/// - v14 (2026-05-18): Spec 19 phase 2 — three new `EntityKind`
///   discriminants appended: `Villager = 9`, `IronGolem = 10`,
///   `WanderingVillager = 11`. Existing discriminants unchanged
///   (wire-stable promise). Also adds VILLAGE_BELL (id 50).
/// - v15 (2026-05-19): Wave 29 log seasoning — adds the DRYING_RACK
///   block (id 51), three new log materials (GreenLog / SeasonedLog /
///   KilnDriedLog appended to MaterialId), and the OAK_LOG mine-drop
///   switch from block to GreenLog material. The Drying Rack workstation
///   matures green logs into seasoned logs over ~5 real minutes per slot
///   when the block above is AIR. Per-rack state lives in a new
///   `WorldSave.drying_racks` field, serde-defaulted for back-compat.
///   See foundation `2026-05-19-log-seasoning.md`.
/// - v16 (2026-05-19): Spec 23 Papyrus Reed — adds 4 new block ids
///   (PAPYRUS_STAGE_0..3 = 52..=55), 2 new materials (PapyrusReed,
///   PapyrusSheet) appended to MaterialId, and the `is_paperish_slot`
///   crafting predicate. Same registry-fallback rationale as previous
///   block-id bumps. Foundation A of Build Schematics — unblocks Spec
///   24 (Blueprint Paper recipe consumes PapyrusSheet via the new predicate).
///   See foundation `2026-05-19-papyrus-reed.md`.
/// - v17 (2026-05-19): Spec 24 Build Schematics Core (Foundation B,
///   partial — Phases 2-4 + 13-14 delivered; UI Phases 5-12 deferred).
///   Adds 3 new block ids (BLUEPRINT_PAPER = 56, CONSTRUCTION_ANCHOR = 57,
///   ARCHITECT_PLAQUE = 58), a 4th Item variant (`Item::Plan(PlanData)`),
///   2 new SavedSlot variants in inventory persistence (Plan), and 2
///   new WorldSave fields (`construction_anchors`, `architect_plaques`).
///   Bincode is positional; appending the Item enum variant plus the
///   SavedSlot variant requires a wire bump. See foundation
///   `2026-05-19-build-schematics-core.md`.
/// - v18 (2026-05-20): Spec 24 Phases 5+7+9+10+11+12 + 2026-05-20
///   paper-economy & creative-vs-survival amendments. PlanData gains
///   `authored_in: String` (#[serde(default)] → "survival" for legacy
///   plans). ConstructionAnchorData gains `is_creative_build: bool`
///   (#[serde(default)] → false). ArchitectPlaqueData replaces the
///   `PlaqueChain = Vec<DerivationLink>` type alias with a struct
///   carrying `{ chain, authored_in }`. Blueprint Paper recipe yield 4 → 9
///   per PapyrusSheet (per-grade split per the amendment). WorldMeta
///   gains `has_seen_license_onboarding: bool` (#[serde(default)] →
///   false). All field additions are #[serde(default)] at-EOF so
///   pre-amendment saves load with conservative defaults — but the
///   wire bump signals the new shape for any peer that sees it.
/// - v19 (2026-05-21): Spec 28d chunk 3 — two new `EntityKind`
///   discriminants appended: `Horse = 13`, `Rabbit = 14`. Existing
///   discriminants unchanged (wire-stable promise). Also adds three
///   new MaterialId variants (RawRabbit, CookedRabbit, RabbitHide)
///   appended bincode-positionally. Live spawn deferred; the wire
///   shape is locked so future ECS spawn work doesn't need another
///   bump.
/// - v20 (2026-05-21): Spec 28d chunk 4 — `EntityKind::Goat = 15`
///   appended. Goat AI module ships with the same deferred-spawn
///   posture as horse/rabbit.
/// - v21 (2026-05-21): Spec 28d chunk 5 — `EntityKind::Bee = 16`
///   appended; also two new MaterialId variants (HoneyBottle, BeeStinger)
///   appended bincode-positionally. Bee AI is flight-bobbed wander
///   with sting-on-attack.
/// - v22 (2026-05-21): Spec 28d chunk 6 — `EntityKind::Squid = 17`
///   appended. Squid AI is aquatic drift (in WATER blocks) with
///   suffocation-on-air timer. Drops 1-3 InkSac (material already
///   reserved by 28c).
/// - v23 (2026-05-21): Spec 28d chunk 7 — a retired-roster EntityKind
///   (discriminant 18) + its drop MaterialId appended (both later excised
///   in v39); a ranged-attack AI module shipped alongside.
/// - v24 (2026-05-21): Spec 28d chunk 8 — `BEE_HIVE = 111` block ID +
///   new `BlockEntityData::Hive(HiveData)` variant + new `SavedHive`
///   WorldSave Vec. WorldSave format extends with `hives` Vec
///   (#[serde(default)] for legacy saves).
/// - v25 (2026-05-22): HP-2 — `EntityKind::Bear = 20` + `EntityKind::Hyena
///   = 21` appended (wire-stable). `CHEST = 112` block ID + new
///   `BlockEntityData::Chest(ChestData)` variant + `SavedChest` Vec
///   on WorldSave (#[serde(default)]).
/// - v26 (2026-05-23): HP-3 — three new `EntityKind` discriminants
///   appended (Brigand = 22, Marauder = 23, Berserker = 24). New
///   `BRIGAND_HIDEOUT_BANNER = 113` block ID (decorative gate marker
///   for worldgen-placed Brigand Hideouts; no block-entity). New
///   `WorldSave.brigand_hideouts: Vec<SavedHideout>` field
///   (#[serde(default)]). Wire-stable promise preserved — no renumber.
/// - v27 (2026-05-23): HP-4 — `EntityKind::Knight = 25` appended.
///   Knight reuses `AiState::GolemGuard` + a generalised
///   `tick_golem_combat`; no new state-tag discriminants. Ships
///   alongside the Iron Golem (HP-6 retires the golem).
/// - v28 (2026-05-23): HP-3 v2 — `TROPHY_WALL = 114` BlockId. Pure
///   decoration (no block-entity); crafted from a BrigandChieftainTrophy
///   plus 2 oak planks vertical. Bump forces clean rejection of stale
///   clients that wouldn't know the new id.
/// - v29 (2026-05-23): Salt feature — 5 new BlockIds (ROCK_SALT,
///   SALT_LICK, SALT_LAMP, SALT_BLOCK, SALT_PATH) at ids 115-119 +
///   Salt + 15 Cured/Seasoned MaterialIds appended. Save shape
///   unchanged (positional `u16` enum append for both blocks and
///   materials).
/// - v30 (2026-05-23): Rubber feature — 4 new BlockIds (RUBBER_LOG,
///   RUBBER_PLANKS, RUBBER_LEAVES, RUBBER_LOG_TAPPED) at ids 120-123
///   + Rubber/RubberSapling/RubberBall/CopperCable MaterialIds +
///     WoodSpecies::Rubber + ToolType::Slingshot + ToolType::Eraser +
///     ArmourMaterial::Rubber appended. All positional enum appends;
///     save shape unchanged.
///     v31 (2026-05-23): Mob Bounty Board (Spec 33) — BOUNTY_BOARD = 124,
///     MaterialId::BountyBoardItem, PayoutKind::BountyClaim, 3 new
///     WorldSave fields (bounties Vec, bounty_next_id, bounty_last_refresh_tick).
///     PlayerSlot.bounties_claimed map. Positional enum append.
///     v32 (2026-05-23): Tip Jar (Spec 34) — TIP_JAR = 125,
///     MaterialId::TipJarItem, PayoutKind::Tip,
///     BlockEntityData::TipJar(TipJarData) variant + WorldSave.tip_jars
///     Vec. PlayerSlot.open_tip_jar field. Positional enum appends.
///     v33 (2026-05-23): Tool Repair / Repair Bench (Spec 35) —
///     REPAIR_BENCH = 126, MaterialId::RepairBenchItem,
///     PayoutKind::RepairTax (the first sat SINK). Stateless block —
///     no block-entity / WorldSave field. Positional enum appends.
///     v34 (2026-05-23): Plot Ownership v1 (Spec 36) — PLOT_MARKER = 127,
///     MaterialId::PlotMarkerItem, WorldSave.plots Vec<PlotData>.
///     Also fixes a latent bug: BountyBoard/TipJar/RepairBench/
///     PlotMarker items are now in material_as_placeable_block (they
///     were uncraftable-into-placement before). Positional appends.
///     v35 (2026-05-23): Market Hubs v1 (Spec 37) — MARKET_BELL = 128,
///     MaterialId::MarketBellItem, WorldSave.market_hubs
///     Vec<MarketHubData>. Positional appends.
///     v36 (2026-05-23): Auctions v1 (Spec 38) — AUCTION_BLOCK = 129,
///     MaterialId::AuctionBlockItem, BlockEntityData::Auction(AuctionData)
///   + WorldSave.auctions Vec<SavedAuction>. Positional appends.
///     v37 (2026-05-23): Server Bazaar v1 (Spec 39) — BAZAAR_BLOCK = 130,
///     MaterialId::BazaarBlockItem, PayoutKind::BazaarSale. Stateless
///     (no block-entity / WorldSave field). Positional appends.
///     v38 (2026-05-24): Player avatars + viewmodel Phase 1 — PlayerState's
///     block-only `held_item: u16` replaced by tool-capable `held_kind: u8` +
///     `held_id: u16` (see `ItemRef`/`item_kind`), plus `anim_state: u8` and
///     `flags: u8` (see `player_flags`) for animated remote avatars.
///     v39 (2026-05-24): fantasy roster excised for open-source cleanup — removed
///     the 7 retired hostile-mob EntityKinds + 5 retired drop MaterialIds
///     (BREAKING, pre-launch). See docs/foundations/2026-05-24-fantasy-roster-excision.md.
///     v40 (2026-05-28): Spec 38 (Blueprint / Cyanotype) — `PlanData` gains
///     `develop_state: DevelopState` (Latent / Developed) with
///     `#[serde(default)]` returning `Developed` for forward-compat sketch
///     (bincode v1 cannot honour that on missing trailing bytes — old saves
///     containing captured plans won't load; accepted pre-launch). New
///     `LATENT_PRINT = 158` BlockId + `BlockEntityData::LatentPrint`
///     variant + `WorldSave.latent_prints: Vec<SavedLatentPrint>`. The
///     retired `Stick + PapyrusSheet → 9 Plan Tiles` recipe is replaced
///     by `Papyrus Sheet + Iron + Salt → 3 Blueprint Paper` (vertical
///     column). All positional enum appends. See
///     docs/foundations/2026-05-27-blueprint-cyanotype.md.
///     v41 (2026-05-28): Spec 40 (Bulk Vendor) — `VendorMode::Bulk` variant
///     appended after `SellPlanLicence`; `VendorData.lot_size: u32` field
///     added with `#[serde(default)]` returning 1. New
///     `VendorUiOutcome::OwnerSetLotSize(u32)`. The inline Sell/Buy/Barter
///     buy logic in `game_loop.rs` has been extracted into the pure
///     `vendor::try_buy(data, mode, policy, charter, buyer_inventory) ->
///   Result<BuyOutcome, BuyRefusal>` helper; `preview_refusal` moved
///     from `vendor_ui` to `vendor` (re-exported for back-compat). All
///     positional enum / field appends. See
///     docs/foundations/2026-05-23-bulk-vendor.md.
///     v42 (2026-06-02): player cosmetics Phase 3 — `JoinRequestPacket.skin_key: u64`
///     (client announces its skin reference on join) and `PlayerState.skin_key: u64`
///     (server broadcasts each player's skin reference every tick). Both appended
///     last; a `u64` content hash of the player's `CosmeticDescriptor` (0 = default).
///     Foundation for per-player skin delivery (the skin BYTES aren't networked yet;
///     renderers key off the per-player texture-array layer, not skin_key). All
///     positional trailing appends. See
///     docs/superpowers/plans/2026-06-02-player-cosmetics-phase-3.md.
///     v43 (2026-06-04): Blueprint column-capture — Drafting Stamp tool.
///     ToolType::DraftingStamp appended last (positional enum append, wire-stable).
///     No new BlockIds, MaterialIds, or WorldSave fields. Old saves load fine
///     because the new variant is never present in existing tool slots.
///     v44 (2026-06-09): `JoinAcceptPacket` and `ServerAnnouncePacket` gain
///     `play_mode: PlayMode` alongside the existing `is_creative: bool`.
///     `is_creative` is kept as a derived projection for back-compat; new
///     consumers read `play_mode` directly. Bincode is positional so the field
///     append requires a version bump.
///     v45 (2026-06-10): Rail freight Phase 1 — `EntityKind::Cart = 26` appended
///     (wire-stable; existing variants keep their discriminants). Carts are
///     track-driven entities, not mobs; the hosted server broadcasts them via
///     the same `EntitySpawn`/`EntityUpdate` path as mobs (lerped Position +
///     `CartData.facing` yaw, neutral health 0). Positional enum append.
///     v46 (2026-06-10): Craftable armoured carts CA1 — cart HULL tier. `CartData`
///     gains a trailing `hull: Hull` field (`Wood`/`Iron`/`Diamond`, default
///     `Wood`). This is a SAVE-SHAPE change only — `SavedCart` (nested in
///     `WorldSave.carts: Vec<SavedCart>`) changes its bincode layout, so the
///     version bumps to flag it. The WIRE is UNCHANGED: no consumer renders
///     broadcast carts yet, so `EntitySpawn`/`EntityUpdate` do NOT gain a hull
///     field. Pre-hull saves that already contain carts load with `carts == []`
///     (the `carts` tail decode now defaults on error — see
///     `save::deserialize_world_save_tolerant`); a fresh/parked cart is a wood
///     cart via `#[serde(default)]`. CA4 will read `Hull::hardness` for breaching.
///     v47: `JoinAccept` now carries the world's REAL seed instead of a hardcoded
///     42 (dedicated-server work, gap G3) — joiners previously generated mismatched
///     terrain. Wire layout is unchanged; the bump enforces the lockstep client
///     update via the existing protocol-version check.
///     v48: Phase 4 verified identity — `ChallengePacket` gains a trailing `origin`
///     (the value the client signs into its kind-21236 auth event); authenticated
///     clients now WAIT for the challenge, sign `{nonce, origin}`, then send a
///     JoinRequest carrying the signed `auth_event`. The server verifies a present
///     auth_event and rejects an absent one on a sign-in-required host.
///     `player_name` is a display fallback only — never a trusted identity source.
///     v49: Phase 4 inspect view — `PlayerEventType::Joined` gains a trailing
///     `npub` (the joiner's full NIP-19 npub, or `""` for a guest) so the client
///     can show a copyable inspect view to verify a specific person.
///     v50: Server-identity join proof (Track 3) — the join handshake carries the
///     server's identity proof so a pinned client verifies the operator.
///     v51: Spec 48 Electricity — `BlockChange` gains `meta: u8` + the chunk stream
///     carries sparse per-block metadata.
///     v52: Operator Console (Spec B) — `PacketType::OperatorSnapshot = 51` +
///     `OperatorSnapshotPacket { snapshot_json }`, server → an authenticated
///     operator-player only. Append-only; no existing packet shape changed.
///     v53: Texture packs (Spec 03 §11.6) — `PacketType::ResourcePackSuggest = 52`
///   + `ResourcePackSuggestPacket { name, url, sha256, size_bytes, required }`,
///     server → client after join when a pack is configured. Append-only; no
///     existing packet shape changed.
///     v54: Creator Gallery (Spec 2026-06-19 §9) — `JoinAcceptPacket` gains a final
///     `exhibits: Vec<Exhibit>` so a joining client renders the world's authored 2D
///     art (it's authored, not procedural like the gallery, so it must travel the
///     wire). Append-LAST + `#[serde(default)]`; no existing packet shape changed.
///     v55: Explosives (Spec 49) — new BlockIds (Brimstone/Nitre/Composter/Blasting
///     Keg/Plunger Detonator, 293..=297), MaterialIds (Saltpetre/BlackPowder,
///     156..=157), a Composter block-entity + a BlastingKeg PowerDevice kind. No
///     existing packet shape changed; the version gate rejects pre-explosives peers.
///     v56: Pets-debt-water wave (2026-07-06) — `EntityKind` gained `Crab = 37`
///     (and, over the same wave, Fish/Fox/Cat/Donkey/Mule earlier), broadcast by
///     `StateUpdatePacket.entity_spawns`. No existing packet shape changed; bumped
///     so an old client that doesn't know a new `EntityKind` discriminant gets a
///     clean version-gate rejection at join instead of a bincode decode error on
///     first broadcast of the new mob.
///     v57: Death-drops phase 2 (2026-07-11) — `EntityKind::Item = 38` + the
///     `item_kind`/`item_id`/`item_count` stack payload on `EntitySpawn`, so
///     server-side drops are visible to remote clients. Packet shape CHANGED
///     (`EntitySpawn` widened), hence the bump.
///     v58: Death-drops phase 2b (2026-07-11) — `PacketType::InventoryGrant = 53`
///     and `InventoryGrantPacket`, server → the picking-up client only,
///     delivering the stack a server-side pickup granted. Append-only; bumped
///     so an old client never receives an unknown packet tag.
///     v59: Weather sync (P9) — `StateUpdatePacket` gains a trailing
///     `rain_ticks_left: u32` + `storm_ticks_left: u32` (from
///     `Weather::ticks_left`) so a joined client's rain/lightning window
///     matches what the server (and its fire-dousing) actually simulates,
///     instead of every peer rolling its own private weather. Append-LAST +
///     `#[serde(default)]`; a v58 peer's packet still decodes (reads as
///     permanently clear — an acceptable degrade).
/// - v60 (2026-09-05): World chat Phase 2 — `PacketType::ChatSay = 54`
///   (client → server, the typed text only) and `PacketType::ChatDeliver = 55`
///   (server → client, one line already permitted for this recipient by the
///   world-chat tier rule: `from_pubkey`, server-chosen `from_name`, `text`,
///   `ChatWireKind`). Two types rather than one symmetric packet, because a
///   client must never be able to assert its own attribution. Append-only —
///   no existing packet shape changed; bumped so a pre-chat peer never
///   receives an unknown tag. See `docs/foundations/2026-09-05-world-chat.md`.
/// - v61 (2026-09-06): Death-drops phase 3, full-fidelity item wire — the new
///   `WireItem` enum rides as a trailing `full_item` on `EntitySpawn` and on
///   `InventoryGrantPacket`, carrying tool type/material/durability and armour
///   slot/material/durability. Server-side tool and armour drops are now
///   granted to (and rendered for) server-simulated players instead of sitting
///   on the floor until lifetime expiry. Packet shape CHANGED (two structs
///   widened), hence the bump. Plans stay floor-bound by design.
///   See `docs/foundations/2026-07-12-full-fidelity-item-wire.md`.
/// - v62 (2026-09-07): Wind, Copper & Electricity wave — `PacketType::
///   DeviceInteract = 56` + `DeviceInteractPacket { pos }`, client → server.
///   Until now nothing on the wire carried a device interaction, so a joiner's
///   lever/button/crank/mirror flipped only their own copy of the world while
///   the host — the authority for the power sim — never heard about it. The
///   host validates (joined, in reach, a toggle-class `PowerDevice` there),
///   applies the interaction with the same `power::interact_device` the
///   single-player client uses, and the resulting flips ride the existing
///   block-change broadcast. Append-only; bumped so a pre-v62 host never
///   receives an unknown tag.
/// - v63 (2026-09-27): join channel binding (audit fix B) — `ChallengePacket`
///   LOSES its `origin` field (packet shape CHANGED). The joiner signs an origin
///   it builds from its own transport (`signet::join_origin`: the QUIC TLS
///   exporter, or `axenstax-join:unbound`); the host recomputes it from its own
///   transport and requires an exact match.
/// - v64 (2026-09-28): QUIC game-packet framing (audit wave 1). Every game
///   packet now rides ONE reliable, ordered bidirectional stream per
///   connection as `u32 LE length + payload` (max 16 MiB), opened by the
///   client with a zero-length hello — no more QUIC datagrams. The server
///   closes a client with more than 8 MiB queued ("connection too slow").
///   No packet shape changed; the transport framing did, so a v63 peer can't
///   talk to a v64 one.
/// - (2026-10-06, NO bump — still v64) bounded StateUpdates (gap-audit T1-5,
///   T2-12): a tick's deltas may now span several `StateUpdate`s (each
///   repeats the snapshot fields; a remote client gets at most 48 KiB a tick,
///   the rest on later ticks), and the frame cap drops from 16 MiB to
///   `MAX_WIRE_PACKET_LEN` (tag + 64 KiB). No packet shape changed, and a v64
///   client already accumulated deltas across StateUpdates; frames between
///   the two caps could never decode anyway. See `state_outbox`.
/// - v65 (2026-10-06): join world flags (gap-audit T2-9). `JoinAcceptPacket`
///   gains trailing `world_rules: WorldRules` (world type, flat ground, water
///   depth, Workshop void, time lock, mobs, explosives, fire spread, keep
///   inventory) + `worldgen_version: u32`; `JoinRequestPacket` gains trailing
///   `worldgen_version: u32`. The joiner now waits for `JoinAccept` and builds
///   its world meta from it (seed + rules + spawn) before generating terrain.
///   A server that can't decode a JoinRequest still reads its leading
///   `protocol_version` ([`peek_protocol_version`]) so an older client gets
///   the mismatch reason instead of silence.
/// - v66 (2026-10-06): WebSocket join origin (Spec 08 §9.0.1 T-JOIN-RELAY,
///   WebSocket residual). `JoinRequestPacket` gains trailing `ws_host: String`
///   (the normalised `host[:port]` a WS joiner dialled). A WS join now signs
///   `axenstax-join:ws-host:<ws_host>` instead of `axenstax-join:unbound`; a
///   dedicated server with `--public-host` refuses any other host, and signs
///   its `JoinAccept` identity proof over the same origin. QUIC and in-process
///   joins are unchanged. Bumped because a v65 WS client signs `unbound`,
///   which a v66 server refuses — the version reason is clearer.
/// - v67 (2026-10-06, MP-A3): server-held death + server projectiles.
///   `PacketType::Respawn = 57` (C→S, empty), `PlayerEventType::Died` and
///   `PlayerEventType::Respawned { x, y, z }` (S→C), and
///   `EntityKind::Projectile = 39`, all appended. A dead joiner stays dead on
///   the server until it asks to respawn (the 40-tick revive BRIDGE is gone);
///   a dedicated server's dispenser arrows fly, hit and reach joiners.
/// - v68 (2026-10-07, MP-D2a): joiners see the server's mobs and are hurt by
///   them. `EntityUpdate` gains trailing `vx`/`vy`/`vz` + `flags`
///   ([`entity_flags`]: hurt flash, baby, tamed, Satoshi) and is sent
///   changed-only; entity events are filtered per client by an interest
///   radius around a joiner's body (`entity_broadcast`). `InputPacket` gains
///   trailing `armour_points` + `health_delta`: a joiner's health is the
///   server's (it lands mob and lava/fire hits server-side), and the client
///   reports only the changes it still owns (eating, regen, poison,
///   starvation). Packet shapes CHANGED, hence the bump.
/// - v69 (2026-10-07, Phase B2a): the server pushes chunks to joiners.
///   `ChunkDataPacket` (tag 3, until now decoded but never sent) gains its
///   side data (`meta`, render-visible `entities`, face `attachments`) and
///   continuation packets; `JoinRequestPacket` gains trailing
///   `render_distance: u8`; `InputPacket` gains trailing `chunk_ack: u32`
///   (the push's credit window), `chunk_drops: Vec<ChunkDrop>` (columns the
///   client let go of) and `render_distance: u8` (its current one). Server
///   block changes reach a joiner only for chunks it has been sent. See
///   `chunk_push` and Spec 04 §4.1.
/// - v70 (2026-10-07, MP-D2b): joiners act on the server's mobs. Appended:
///   `PacketType::EntityAttack = 58` and `EntityInteract = 59` (C→S: a swing
///   or a one-shot right-click — feed, tame, shear, milk, lead on/off, sit —
///   on an entity named by its `ProtocolId`, or `InteractKind::LeadToPost`, a
///   Lead on a fence post; the held item is the client's word),
///   `InteractOutcome = 60` (S→C, to the asker: accepted / items consumed / a
///   note code; the weapon wears only on an accepted swing) and `KillEvent =
///   61` (S→C, to the killer: species, why credited — `kill_reason` —,
///   position, flags); `PlayerEventType::DiedOf { cause }` (the death
///   screen's real cause), `ArmourWorn { hits }` (server-landed hits wear the
///   joiner's armour) and `Bred { offspring }` (a baby of an animal this
///   joiner fed); `entity_flags::TETHERED`. Kills and breeds a joiner makes
///   credit that joiner, never a host's player nor a later joiner in its slot.
/// - v71 (2026-10-07, Phase B2b): touched columns. A joiner whose terrain
///   generator matches the host's is pushed only the columns that differ from
///   generation; for every other column within its push radius the server
///   sends `PacketType::ColumnLocal = 4` ([`ColumnLocalPacket`]
///   `{ cx, cz, hash }`, the hash of the server's live column) in the same
///   ordered, numbered chunk stream, and the joiner generates it itself and
///   checks the hash. `JoinAcceptPacket` gains trailing `chunk_note_radius:
///   u8` (the server's push limit when it sends notes, `0` when it pushes
///   everything); `InputPacket` gains trailing `column_mismatch:
///   Option<ColumnMismatch>` (a sticky "push me everything" switch, set when
///   a local column's generation did not hash as the note said).
///   `--chunk-sync touched` is the default. See `chunk_verdict`, `chunk_push`
///   and Spec 04 §4.1.
/// - v72 (2026-10-07, C1): the server yields a joiner's breaks.
///   `InputPacket` gains trailing `mined: Vec<MinedBlock>` (after B2b's
///   `column_mismatch`: the cells its survival break arm mined, each with the
///   tool it mined with). The server computes the drop — crop, tool-tier mine
///   drop + bonus, Satori on the world's secret — and grants it by
///   `InventoryGrant`; a joined client no longer grants itself break drops.
///   Packet shape CHANGED, hence the bump.
/// - v73 (2026-10-07, C2a): a joiner's hunger, eating and sleep
///   are the server's. Appended: `PacketType::ItemAction = 62` (C→S,
///   [`ItemActionPacket`]: `Eat` with the held-food claim, or `Sleep` at a
///   bed) and `ItemActionOutcome = 63` (S→C, to the asker: accepted / items
///   consumed / a note code); `StateUpdatePacket` gains trailing
///   `own_hunger: u8` (the addressed client's hunger as the server holds
///   it). The server runs every joiner's metabolism and ignores a reported
///   heal (`InputPacket.health_delta` counts losses only).
///   FU3 (2026-10-07, NO bump — no shape change): `entity_flags::
///   PRODUCT_NOT_READY` (bit 32) on a mirrored cow or sheep whose milk or
///   wool isn't ready; 0 means ready or unknown, so either side may be older.
/// - v74 (2026-10-07, C2b): a joiner's crafting and Q-drops are mirrored on
///   the server.
///   `ItemAction` appends `Craft { grid, table }` (= 2) and `Drop` (= 3,
///   the held claim of `Eat`); both fire-and-forget (no outcome). The server
///   mirrors a craft on its shadow of the joiner's inventory and spawns a
///   Q-drop as a real ground item; a grant that doesn't fit the shadow
///   spills at the joiner's feet.
/// - v75 (2026-10-08, C3a-2a): the server mirrors a joiner's inventory window, click for
///   click. Appended: `PacketType::WindowOp = 64` (C→S, [`WindowOpPacket`]
///   `{ op_seq, op: WireWindowOp, digest }`: `Click(window::WindowClick)`,
///   `OpenPlayer`, `OpenTable { cell }`, `SetAutoRefill { on }`), never
///   answered. `window::WindowClick`, `window::WindowSlot` and
///   `crafting::CraftSlot` become wire data (append-only); a drag's slot
///   list is bounded at 45. The server applies the same `window::apply` to
///   its copy of the joiner's window (36 slots, armour, cursor, grid,
///   station), behind the client's edits, and tallies digest mismatches
///   (log-only). `ItemAction::Craft` (= 2) is unused: the craft is the
///   result click; a v75 server ignores it and tallies it.
/// - v76 (2026-10-08, C3a-fix-1): a joiner's window stays in lockstep. The server numbers every
///   change it makes to a joiner's window — a grant (a pickup's too), an
///   accepted request's owed take, an armour-wear hit, a swing's weapon wear —
///   as a window event (1, 2, 3… per connection), queues it, and applies it
///   to its copy only up to the count the client reports having applied.
///   S→C carriers gain trailing `window_event: u32` (0 = changes nothing):
///   `InventoryGrantPacket`, `InteractOutcomePacket`,
///   `ItemActionOutcomePacket`, `PlayerEventPacket` (for `ArmourWorn`). C→S
///   packets the server judges against the window gain trailing
///   `events_applied: u32`: `InputPacket` (then `edit_hands:
///   Vec<EditHand>`, each edit's own hotbar slot and hand),
///   `WindowOpPacket`, `ItemActionPacket`, `EntityInteractPacket` and
///   `EntityAttackPacket` (after a new `hotbar_slot: u8`, the swing's slot).
///   `ItemAction` appends `GrantUnfit { event, count }` (= 4): the part of a
///   grant that didn't fit the client comes back as a real ground item the
///   server spawns, never a client-local spill.
/// - v77 (2026-10-08, C3b-1): shared chests, dispensers and furnaces for
///   joiners. `WireWindowOp` appends `OpenContainer { cell }` (= 4) and
///   `Container(container_window::ContainerClick)` (= 5; Withdraw, Deposit,
///   Sort, DumpMatching, Restock, TakeAll, Furnace, append-only);
///   `WindowOpPacket` appends, after v76's `events_applied`, `touched:
///   Vec<WireWindowSlot>` (≤ 122, the slots a container op changed on the
///   client) and `claims: Vec<(WireWindowSlot, WireSlot)>` (≤ 122, the
///   client's pre-op values of the player slots a container op acts on).
///   Appended S→C: `ContainerOpened = 65` ([`ContainerOpenedPacket`] `{ cell,
///   kind, slots ≤ 72, furnace, refused }`) and `WindowSlotSet = 66`
///   ([`WindowSlotSetPacket`] `{ op_seq_applied, reason, sets ≤ 122,
///   furnace, window_event }`: a per-slot correction of a container op, or a
///   push of what changed in the open container — never a whole window; a
///   set that changes player slots is a numbered window event,
///   `window_event` ≠ 0). `item_kind::PLAN = 4` is reserved for a Plan
///   placeholder in a [`WireStack`]. A container op's window digest covers
///   the container.
pub const PROTOCOL_VERSION: u32 = 77;

/// The `protocol_version` of a JoinRequest payload that doesn't decode as this
/// build's `JoinRequestPacket` (an older or newer client's shape). It is the
/// packet's first field, a fixint `u32` LE, in every version. `None` when the
/// payload is shorter than that.
pub fn peek_protocol_version(payload: &[u8]) -> Option<u32> {
    payload.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The refusal a joiner on another protocol version is shown.
pub fn protocol_mismatch_reason(client: u32) -> String {
    format!("Protocol mismatch: client v{client}, server v{PROTOCOL_VERSION}")
}

/// Magic bytes for LAN discovery packets. `discovery.rs` hardcodes the same
/// `*b"AXNS"` literal directly (both writing and checking it) rather than
/// referencing this named constant, so this one has no consumer.
#[allow(dead_code)]
pub const ANNOUNCE_MAGIC: [u8; 4] = *b"AXNS";

/// LAN discovery broadcast port.
pub const DISCOVERY_PORT: u16 = 7705;

/// Game server QUIC port.
pub const SERVER_PORT: u16 = 7700;

/// Serialize a packet with a type tag prefix.
pub fn serialize_packet<T: Serialize>(packet_type: PacketType, payload: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(packet_type as u8);
    let payload_bytes = bincode::serialize(payload).expect("serialization failed");
    buf.extend_from_slice(&payload_bytes);
    buf
}

/// Deserialize a packet, returning the type tag and remaining bytes.
pub fn deserialize_header(data: &[u8]) -> Option<(PacketType, &[u8])> {
    if data.is_empty() {
        return None;
    }
    let tag = match data[0] {
        1 => PacketType::ClientInput,
        2 => PacketType::StateUpdate,
        3 => PacketType::ChunkData,
        4 => PacketType::ColumnLocal,
        10 => PacketType::JoinRequest,
        11 => PacketType::JoinAccept,
        12 => PacketType::JoinReject,
        20 => PacketType::Disconnect,
        21 => PacketType::PlayerEvent,
        30 => PacketType::Ping,
        31 => PacketType::Pong,
        40 => PacketType::ServerAnnounce,
        50 => PacketType::Challenge,
        51 => PacketType::OperatorSnapshot,
        52 => PacketType::ResourcePackSuggest,
        53 => PacketType::InventoryGrant,
        54 => PacketType::ChatSay,
        55 => PacketType::ChatDeliver,
        56 => PacketType::DeviceInteract,
        57 => PacketType::Respawn,
        58 => PacketType::EntityAttack,
        59 => PacketType::EntityInteract,
        60 => PacketType::InteractOutcome,
        61 => PacketType::KillEvent,
        62 => PacketType::ItemAction,
        63 => PacketType::ItemActionOutcome,
        64 => PacketType::WindowOp,
        65 => PacketType::ContainerOpened,
        66 => PacketType::WindowSlotSet,
        _ => return None,
    };
    Some((tag, &data[1..]))
}

/// Maximum network packet payload size (64 KB). Bincode deserialization of
/// untrusted data is capped at this limit to prevent OOM from crafted length
/// prefixes on Vec/String fields.
pub const MAX_PACKET_SIZE: u64 = 65_536;

/// Largest whole game packet on the wire: the 1-byte type tag plus a payload
/// of at most [`MAX_PACKET_SIZE`]. Anything bigger can never decode (the
/// payload trips `safe_deserialize`'s limit), so it is also the transport
/// frame cap on both ends (`network::MAX_FRAME_LEN`, the WebSocket accept
/// config) — one number, so a receiver never buffers a frame it is bound to
/// throw away. Senders stay under it: `StateUpdate`s are split by
/// `state_outbox` (with headroom), and `RemoteClient::send_input` moves the
/// block changes that don't fit an input packet into the next one.
pub const MAX_WIRE_PACKET_LEN: usize = 1 + MAX_PACKET_SIZE as usize;

/// Safely deserialize a network packet payload with a size limit.
/// Prevents OOM attacks from malicious bincode length prefixes.
pub fn safe_deserialize<'a, T: Deserialize<'a>>(payload: &'a [u8]) -> Result<T, bincode::Error> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_PACKET_SIZE)
        .deserialize(payload)
}

/// Compress block data with LZ4 for chunk transmission (the server's chunk
/// push, `chunk_push`). Measured over real saves and fresh terrain
/// (2026-10-07): a terrain chunk is about 2.3 KB (max about 4.4 KB), an
/// all-air one about 50 bytes.
pub fn compress_chunk(block_data: &[u8]) -> Vec<u8> {
    lz4_flex::compress_prepend_size(block_data)
}

/// Maximum expected uncompressed chunk size.
/// A 16x16x16 chunk is 8192 bytes of u16 block IDs + a 512-byte player-placed
/// mask = 8704 bytes (`Chunk::as_bytes`). 16 KiB keeps ~2x headroom.
const MAX_CHUNK_DECOMPRESSED: usize = 16_384;

/// Decompress LZ4 chunk data with size limit to prevent decompression bombs.
/// Rejects payloads whose prepended size exceeds MAX_CHUNK_DECOMPRESSED.
pub fn decompress_chunk(compressed: &[u8]) -> Result<Vec<u8>, String> {
    // The prepended size is a little-endian u32 in the first 4 bytes.
    if compressed.len() >= 4 {
        let claimed_size = u32::from_le_bytes([
            compressed[0], compressed[1], compressed[2], compressed[3],
        ]) as usize;
        if claimed_size > MAX_CHUNK_DECOMPRESSED {
            return Err(format!(
                "LZ4 decompression bomb: claimed {claimed_size} > {MAX_CHUNK_DECOMPRESSED}"
            ));
        }
    }
    lz4_flex::decompress_size_prepended(compressed)
        .map_err(|e| format!("LZ4 decompress: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_ref_roundtrip() {
        use super::{ItemRef, item_kind};
        for r in [ItemRef::Empty, ItemRef::Block(42), ItemRef::Tool(2), ItemRef::Material(7)] {
            let (k, id) = r.to_wire();
            assert_eq!(ItemRef::from_wire(k, id), r);
        }
        assert_eq!(ItemRef::Empty.to_wire(), (0, 0));
        assert_eq!(item_kind::EMPTY, 0);
    }

    #[test]
    fn player_state_extended_roundtrip() {
        let p = PlayerState {
            player_index: 1,
            x: 1.0, y: 2.0, z: 3.0,
            yaw: 0.5, pitch: -0.2,
            health: 18.0,
            held_kind: item_kind::TOOL,
            held_id: 2,
            anim_state: 1,
            flags: 0b101,
            skin_key: 0xDEAD_BEEF_CAFE,
        };
        let bytes = bincode::serialize(&p).unwrap();
        let back: PlayerState = safe_deserialize(&bytes).unwrap();
        assert_eq!(back.held_kind, item_kind::TOOL);
        assert_eq!(back.held_id, 2);
        assert_ne!(back.flags & player_flags::SWINGING, 0);
        assert_ne!(back.flags & player_flags::ON_GROUND, 0);
        assert_eq!(back.flags & player_flags::CROUCHING, 0); // not set in 0b101
        assert_eq!(back.anim_state, 1);
        assert_eq!(back.skin_key, 0xDEAD_BEEF_CAFE, "skin_key survives the bincode round-trip");
    }

    #[test]
    fn protocol_version_bumped() {
        // C3b-1 — v77.
        assert_eq!(super::PROTOCOL_VERSION, 77);
    }

    #[test]
    fn inventory_grant_roundtrip() {
        // Death-drops phase 2b — server → client stack delivery when a
        // server-simulated player picks up a dropped item.
        let pkt = InventoryGrantPacket {
            item_kind: item_kind::MATERIAL,
            item_id: 4,
            count: 3,
            full_item: WireItem::None,
            window_event: 0x0102_0304,
        };
        let bytes = serialize_packet(PacketType::InventoryGrant, &pkt);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::InventoryGrant, "header tag 53 maps back");
        let back: InventoryGrantPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.item_kind, item_kind::MATERIAL);
        assert_eq!(back.item_id, 4);
        assert_eq!(back.count, 3);
        assert_eq!(back.full_item, WireItem::None);
        assert_eq!(back.window_event, 0x0102_0304, "v76 — the grant's window event survives");
    }

    #[test]
    fn wire_item_roundtrips_every_variant() {
        // Death-drops phase 3 — the full-fidelity payload must survive the
        // same bincode path the packets take, on both carriers.
        for w in [
            WireItem::None,
            WireItem::Tool { tool_type: 3, material: 2, durability: 37 },
            WireItem::Armour { slot: 1, material: 4, durability: 165 },
        ] {
            let pkt = InventoryGrantPacket {
                item_kind: item_kind::EMPTY,
                item_id: 0,
                count: 1,
                full_item: w,
                window_event: 1,
            };
            let bytes = serialize_packet(PacketType::InventoryGrant, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            let back: InventoryGrantPacket = safe_deserialize(payload).unwrap();
            assert_eq!(back.full_item, w, "grant carries {w:?} intact");

            let spawn = EntitySpawn {
                id: 9,
                kind: EntityKind::Item,
                x: 1.0,
                y: 2.0,
                z: 3.0,
                yaw: 0.0,
                health: 0,
                item_kind: item_kind::EMPTY,
                item_id: 0,
                item_count: 1,
                full_item: w,
            };
            let bytes = serialize_packet(PacketType::StateUpdate, &spawn);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            let back: EntitySpawn = safe_deserialize(payload).unwrap();
            assert_eq!(back.full_item, w, "spawn carries {w:?} intact");
        }
    }

    #[test]
    fn wire_item_unknown_discriminant_is_an_error_not_a_panic() {
        // A NEWER peer's appended variant (e.g. a future `Plan`) must fail the
        // decode cleanly. The join-time version gate is what actually keeps
        // such a peer out; this pins that the decoder never panics if one
        // slips through.
        let pkt = InventoryGrantPacket {
            item_kind: item_kind::EMPTY,
            item_id: 0,
            count: 1,
            full_item: WireItem::None,
            window_event: 1,
        };
        let bytes = serialize_packet(PacketType::InventoryGrant, &pkt);
        let (_, payload) = deserialize_header(&bytes).unwrap();
        let mut payload = payload.to_vec();
        // Overwrite the first byte of the trailing (fixint u32) variant index
        // with an unassigned discriminant.
        // v76 — `window_event` (a u32) follows `full_item` now.
        let n = payload.len();
        payload[n - 8] = 9;
        let back: Result<InventoryGrantPacket, _> = safe_deserialize(&payload);
        assert!(back.is_err(), "unknown WireItem discriminant must not decode");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn chat_say_roundtrip() {
        let pkt = ChatSayPacket { text: "hello world".to_string() };
        let bytes = serialize_packet(PacketType::ChatSay, &pkt);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::ChatSay, "header tag 54 maps back");
        let back: ChatSayPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.text, "hello world");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn chat_deliver_roundtrip() {
        let pkt = ChatDeliverPacket {
            from_pubkey: Some([0x42; 32]),
            from_name: "Axolittle".to_string(),
            text: "welcome!".to_string(),
            kind: ChatWireKind::Player,
        };
        let bytes = serialize_packet(PacketType::ChatDeliver, &pkt);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::ChatDeliver, "header tag 55 maps back");
        let back: ChatDeliverPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.from_pubkey, Some([0x42; 32]));
        assert_eq!(back.from_name, "Axolittle");
        assert_eq!(back.text, "welcome!");
        assert_eq!(back.kind, ChatWireKind::Player);
    }

    /// A `System` line carries no speaker — `from_pubkey` must round-trip as
    /// `None` rather than some sentinel value.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn chat_deliver_system_line_has_no_speaker() {
        let pkt = ChatDeliverPacket {
            from_pubkey: None,
            from_name: "System".to_string(),
            text: "Chat needs a verified sign-in.".to_string(),
            kind: ChatWireKind::System,
        };
        let bytes = serialize_packet(PacketType::ChatDeliver, &pkt);
        let (_ptype, payload) = deserialize_header(&bytes).unwrap();
        let back: ChatDeliverPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.from_pubkey, None);
        assert_eq!(back.kind, ChatWireKind::System);
    }

    #[test]
    fn device_interact_roundtrip() {
        // Negative and large coordinates both: `pos` is signed world space, and
        // a lever at x = -3000 must not come back as a lever somewhere else.
        for pos in [(0, 64, 0), (-3, 70, 12), (-3000, 5, 2_000_000)] {
            let pkt = DeviceInteractPacket { pos };
            let bytes = serialize_packet(PacketType::DeviceInteract, &pkt);
            assert_eq!(bytes.len(), 1 + 12, "tag + three i32s, nothing else");
            let (ptype, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(ptype, PacketType::DeviceInteract, "header tag 56 maps back");
            let back: DeviceInteractPacket = safe_deserialize(payload).unwrap();
            assert_eq!(back.pos, pos);
        }
    }

    #[test]
    fn device_interact_rejects_a_truncated_or_overlong_payload() {
        // Same posture as every other packet: a malformed body decodes to an
        // error the dispatch arm drops, never a partial interaction.
        let bytes = serialize_packet(
            PacketType::DeviceInteract,
            &DeviceInteractPacket { pos: (1, 2, 3) },
        );
        let (_, payload) = deserialize_header(&bytes).unwrap();
        assert!(
            safe_deserialize::<DeviceInteractPacket>(&payload[..payload.len() - 1]).is_err(),
            "a truncated body must not decode"
        );
        // Trailing junk is refused too — bincode's fixint decode is exact-size.
        let mut long = payload.to_vec();
        long.push(0xFF);
        assert!(
            safe_deserialize::<DeviceInteractPacket>(&long).is_err(),
            "trailing bytes must not decode"
        );
    }

    #[test]
    fn input_packet_roundtrip() {
        let pkt = InputPacket {
            tick: 42,
            x: 1.5, y: 80.0, z: -3.25,
            yaw: 0.5, pitch: -0.1,
            health: 18.0,
            held_item: 7,
            held_kind: item_kind::TOOL,
            held_id: 2,
            move_forward: 0.8, move_right: -0.3,
            sprint: true, sneak: false, jump: true, toggle_flight: false,
            break_block: false, place_block: true, toggle_inventory: false,
            drop_item: false,
            hotbar_slot: Some(4),
            block_changes: vec![BlockChange { x: 0, y: 64, z: 0, new_block: 3, meta: 0 }],
            armour_points: 11,
            health_delta: -1.5,
            chunk_ack: 77,
            chunk_drops: vec![ChunkDrop { cx: -3, cz: 9, as_of: 70 }],
            render_distance: 6,
            column_mismatch: Some(ColumnMismatch { cx: 1, cz: 2, server_hash: 3, client_hash: 4 }),
            mined: vec![MinedBlock {
                x: 0,
                y: 64,
                z: 0,
                tool: WireItem::Tool { tool_type: 0, material: 3, durability: 100 },
            }],
            events_applied: 12,
            edit_hands: vec![(4, item_kind::BLOCK, 3)],
        };
        let bytes = bincode::serialize(&pkt).unwrap();
        let back: InputPacket = safe_deserialize(&bytes).unwrap();
        assert_eq!(back.column_mismatch, pkt.column_mismatch);
        // C3a-fix-1 (v76) — the window-event count and each edit's hand.
        assert_eq!(back.events_applied, 12);
        assert_eq!(back.edit_hands, pkt.edit_hands);
        // C1 (v72) — the mined cells and their tools survive the round-trip.
        assert_eq!(back.mined, pkt.mined);
        assert_eq!(back.chunk_ack, 77);
        assert_eq!(back.render_distance, 6);
        assert_eq!(back.chunk_drops, pkt.chunk_drops);
        assert_eq!(back.tick, pkt.tick);
        // MP-D2a (v68) — the trailing vitals survive the round-trip.
        assert_eq!(back.armour_points, 11);
        assert_eq!(back.health_delta, -1.5);
        assert_eq!(back.hotbar_slot, pkt.hotbar_slot);
        // Tool-capable held ref survives the round-trip.
        assert_eq!(back.held_kind, item_kind::TOOL);
        assert_eq!(back.held_id, 2);
        // Legacy block-only field still preserved alongside the new pair.
        assert_eq!(back.held_item, 7);
        assert_eq!(back.block_changes.len(), 1);
        assert_eq!(back.block_changes[0].new_block, 3);
    }

    /// bincode 1 is positional, so `InputPacket`'s trailing fields must sit in
    /// the order each protocol bump appended them: D2a's vitals (v68), then
    /// B2a's chunk-push feedback (v69), then B2b's column-mismatch switch
    /// (v71), then C1's mined cells (v72). Pinned on the wire bytes, so a
    /// merge that reorders them fails here rather than on a live join.
    #[test]
    fn input_packet_trailing_fields_are_in_append_order() {
        let head = InputPacket { block_changes: Vec::new(), ..Default::default() };
        let pkt = InputPacket {
            armour_points: 0xA5,
            health_delta: 2.5,
            chunk_ack: 0x0102_0304,
            chunk_drops: vec![ChunkDrop { cx: -1, cz: 7, as_of: 9 }],
            render_distance: 0x5C,
            column_mismatch: Some(ColumnMismatch {
                cx: 3,
                cz: -4,
                server_hash: 0x1122_3344,
                client_hash: 0x5566_7788,
            }),
            mined: vec![MinedBlock { x: 3, y: -4, z: 5, tool: WireItem::None }],
            events_applied: 0x0A0B_0C0D,
            edit_hands: vec![(2, 0x11, 0x0304)],
            ..head.clone()
        };
        let base = bincode::serialize(&head).unwrap();
        let bytes = bincode::serialize(&pkt).unwrap();
        // v68 (MP-D2a): armour_points u8, health_delta f32.
        let mut tail = vec![0xA5];
        tail.extend_from_slice(&2.5f32.to_le_bytes());
        // v69 (B2a): chunk_ack u32, chunk_drops (u64 length + entries),
        // render_distance u8.
        tail.extend_from_slice(&0x0102_0304u32.to_le_bytes());
        tail.extend_from_slice(&1u64.to_le_bytes());
        for v in [-1i32, 7] {
            tail.extend_from_slice(&v.to_le_bytes());
        }
        tail.extend_from_slice(&9u32.to_le_bytes());
        tail.push(0x5C);
        // v71 (B2b): column_mismatch (`Some` tag + cx, cz, server_hash,
        // client_hash).
        tail.push(1);
        for v in [3i32, -4] {
            tail.extend_from_slice(&v.to_le_bytes());
        }
        tail.extend_from_slice(&0x1122_3344u32.to_le_bytes());
        tail.extend_from_slice(&0x5566_7788u32.to_le_bytes());
        // v72 (C1): mined (u64 length + entries: x, y, z i32, then the
        // WireItem's u32 variant tag — None = 0).
        tail.extend_from_slice(&1u64.to_le_bytes());
        for v in [3i32, -4, 5] {
            tail.extend_from_slice(&v.to_le_bytes());
        }
        tail.extend_from_slice(&0u32.to_le_bytes());
        // v76 (C3a-fix-1): events_applied u32, then edit_hands (u64 length +
        // entries: slot u8, held_kind u8, held_id u16).
        tail.extend_from_slice(&0x0A0B_0C0Du32.to_le_bytes());
        tail.extend_from_slice(&1u64.to_le_bytes());
        tail.extend_from_slice(&[2, 0x11]);
        tail.extend_from_slice(&0x0304u16.to_le_bytes());
        // Everything before the appended fields is unchanged, and the
        // appended fields close the packet in append order (`None` is one
        // `0` byte).
        let prefix = bytes.len() - tail.len();
        assert_eq!(&bytes[prefix..], &tail[..]);
        assert_eq!(&bytes[..prefix], &base[..base.len() - (1 + 4 + 4 + 8 + 1 + 1 + 8 + 4 + 8)]);
    }

    #[test]
    fn state_update_roundtrip_with_v2_fields() {
        let pkt = StateUpdatePacket {
            tick: 100,
            players: vec![],
            block_changes: vec![],
            world_time: 6000,
            last_acked_input: 42,
            entity_spawns: vec![EntitySpawn {
                id: 7,
                kind: EntityKind::Brigand,
                x: 10.0, y: 64.0, z: 20.0,
                yaw: 0.0, health: 20,
                item_kind: 0, item_id: 0, item_count: 0,
                full_item: WireItem::None,
            }],
            entity_updates: vec![],
            entity_despawns: vec![3],
            reserve_richness: 0.75,
            reserve_target_sats: 60_000,
            reserve_current_sats: 45_230,
            rain_ticks_left: 900,
            storm_ticks_left: 300,
            own_hunger: 0,
        };
        let bytes = bincode::serialize(&pkt).unwrap();
        let back: StateUpdatePacket = safe_deserialize(&bytes).unwrap();
        assert_eq!(back.last_acked_input, 42);
        assert_eq!(back.entity_spawns.len(), 1);
        assert_eq!(back.entity_spawns[0].id, 7);
        assert!(matches!(back.entity_spawns[0].kind, EntityKind::Brigand));
        assert_eq!(back.entity_despawns, vec![3]);
        assert!((back.reserve_richness - 0.75).abs() < 1e-6);
        assert_eq!(back.reserve_target_sats, 60_000);
        assert_eq!(back.reserve_current_sats, 45_230);
        assert_eq!(back.rain_ticks_left, 900);
        assert_eq!(back.storm_ticks_left, 300);
    }

    #[test]
    fn state_update_roundtrip_with_cart_entity() {
        // Rail freight Phase 1 (v45): a cart broadcasts as EntityKind::Cart with
        // its lerped Position + facing yaw, neutral health (0 — not a combat
        // entity). The spawn + a same-tick update must bincode-round-trip equal.
        let pkt = StateUpdatePacket {
            tick: 200,
            players: vec![],
            block_changes: vec![],
            world_time: 12_000,
            last_acked_input: 0,
            entity_spawns: vec![EntitySpawn {
                id: 9,
                kind: EntityKind::Cart,
                x: 5.5, y: 65.0, z: 12.5,
                yaw: 1.5707964, health: 0,
                item_kind: 0, item_id: 0, item_count: 0,
                full_item: WireItem::None,
            }],
            entity_updates: vec![EntityUpdate {
                id: 9,
                x: 6.5, y: 65.0, z: 12.5,
                yaw: 1.5707964, state: 0,
                ..Default::default()
            }],
            entity_despawns: vec![],
            reserve_richness: 0.0,
            reserve_target_sats: 0,
            reserve_current_sats: 0,
            rain_ticks_left: 0,
            storm_ticks_left: 0,
            own_hunger: 0,
        };
        let bytes = bincode::serialize(&pkt).unwrap();
        let back: StateUpdatePacket = safe_deserialize(&bytes).unwrap();
        assert_eq!(back.entity_spawns.len(), 1);
        assert_eq!(back.entity_spawns[0].id, 9);
        assert!(matches!(back.entity_spawns[0].kind, EntityKind::Cart));
        assert_eq!(back.entity_spawns[0].health, 0);
        assert!((back.entity_spawns[0].yaw - 1.5707964).abs() < 1e-6);
        assert_eq!(back.entity_updates.len(), 1);
        assert_eq!(back.entity_updates[0].id, 9);
        assert!((back.entity_updates[0].x - 6.5).abs() < 1e-6);
    }

    #[test]
    fn entity_update_carries_velocity_and_flags() {
        // MP-D2a (v68) — what a joiner's mirror needs to draw a mob: its
        // velocity (walk cycle, smoothing) and the render flags.
        let upd = EntityUpdate {
            id: 4,
            x: 1.0,
            y: 65.0,
            z: -3.0,
            yaw: 0.5,
            state: 2,
            vx: 0.12,
            vy: -0.08,
            vz: 0.0,
            flags: entity_flags::HURT | entity_flags::BABY,
        };
        let bytes = bincode::serialize(&upd).unwrap();
        // id + 4 f32 + state + 3 f32 + flags, fixint.
        assert_eq!(bytes.len(), 4 + 16 + 1 + 12 + 1);
        let back: EntityUpdate = safe_deserialize(&bytes).unwrap();
        assert_eq!(back, upd);
    }

    #[test]
    fn state_update_roundtrip_with_weather_fields() {
        // v59 — weather sync (P9). `rain_ticks_left`/`storm_ticks_left` are
        // ticks-REMAINING (a duration), not the server's absolute
        // `rain_until`/`storm_until`, and must survive the wire untouched.
        let pkt = StateUpdatePacket {
            tick: 5_000,
            players: vec![],
            block_changes: vec![],
            world_time: 6000,
            last_acked_input: 0,
            entity_spawns: vec![],
            entity_updates: vec![],
            entity_despawns: vec![],
            reserve_richness: 0.0,
            reserve_target_sats: 0,
            reserve_current_sats: 0,
            rain_ticks_left: 1_800,
            storm_ticks_left: 600,
            own_hunger: 0,
        };
        let bytes = bincode::serialize(&pkt).unwrap();
        let back: StateUpdatePacket = safe_deserialize(&bytes).unwrap();
        assert_eq!(back.rain_ticks_left, 1_800);
        assert_eq!(back.storm_ticks_left, 600);
    }

    /// Pins the REAL (if surprising) behaviour of a v58 peer's shorter
    /// `StateUpdatePacket` (no `rain_ticks_left`/`storm_ticks_left` on the
    /// wire): it does NOT gracefully decode with zeros. `#[serde(default)]`
    /// is well-known to work for a self-describing format (JSON: a missing
    /// key is just absent from the map), but bincode 1 is positional —
    /// `Deserializer::deserialize_struct` hands the derived impl a
    /// `SeqAccess` whose `len` is the CURRENT struct's field count (13 here),
    /// not the byte stream's actual length. It always attempts to read all
    /// 13 slots and propagates the inner decode's `UnexpectedEof` the moment
    /// bytes run out, rather than reporting "no more elements" so the
    /// `#[serde(default)]` fallback can kick in. `save.rs`'s `read_tail` /
    /// `deserialize_world_save_tolerant` documents this EXACT gotcha for
    /// `WorldSave` ("`#[serde(default)]`, which is inert on this format") and
    /// works around it with a manual, EOF-tolerant field-by-field decode.
    ///
    /// Doing the same for `StateUpdatePacket` here would mean reimplementing
    /// `safe_deserialize`'s `MAX_PACKET_SIZE` guard by hand for a manual
    /// per-field cursor decode (that guard is exactly what stands between
    /// untrusted network bytes and a length-prefix OOM) — real, security-
    /// sensitive work, and not proportionate: `hosted_server.rs`'s
    /// `protocol_version` check (`req.protocol_version != PROTOCOL_VERSION`)
    /// rejects a mismatched JoinRequest before a `StateUpdatePacket` is EVER
    /// exchanged, so a v58-shaped one reaching a v59 decoder is unreachable
    /// in production today. `#[serde(default)]` is kept on the two fields
    /// anyway — consistent with every prior append (v53's `exhibits`, v49's
    /// `JoinRequestPacket` options, …), all of which carry this identical,
    /// pre-existing gap — and stands ready for a real tolerant-decode path
    /// (`read_tail`-style) if the version gate is ever relaxed.
    #[test]
    fn state_update_from_a_shorter_older_peer_is_rejected_not_defaulted() {
        #[derive(serde::Serialize)]
        struct OldStateUpdatePacket {
            tick: u64,
            players: Vec<PlayerState>,
            block_changes: Vec<BlockChange>,
            world_time: u32,
            last_acked_input: u64,
            entity_spawns: Vec<EntitySpawn>,
            entity_updates: Vec<EntityUpdate>,
            entity_despawns: Vec<u32>,
            reserve_richness: f32,
            reserve_target_sats: u64,
            reserve_current_sats: u64,
        }
        let old = OldStateUpdatePacket {
            tick: 42,
            players: vec![],
            block_changes: vec![],
            world_time: 9000,
            last_acked_input: 7,
            entity_spawns: vec![],
            entity_updates: vec![],
            entity_despawns: vec![],
            reserve_richness: 0.5,
            reserve_target_sats: 1000,
            reserve_current_sats: 500,
        };
        let bytes = bincode::serialize(&old).expect("encode v58 shape");
        let result: Result<StateUpdatePacket, _> = safe_deserialize(&bytes);
        assert!(
            result.is_err(),
            "a genuinely shorter v58 stream does NOT decode with zeros — it errors \
             (see doc comment: this is the same bincode limitation save.rs's \
             read_tail works around, and is unreachable in practice because the \
             protocol_version join-time gate rejects the mismatch first)"
        );
    }

    #[test]
    fn cart_entity_kind_wire_encoding_is_stable() {
        // FOOTGUN: bincode encodes an enum by its *ordinal index* (its position
        // in the `enum` body), NOT by the `#[repr(u8)]` discriminant value. The
        // retired-fantasy-mob gaps mean Cart's repr value (26) differs from its
        // wire byte. Cart is the LAST variant (the 20th counting from Cow), so
        // its bincode ordinal is 19. Appending keeps every earlier variant's
        // ordinal fixed — that is the wire-stable property we actually rely on.
        // Pin both ends so a re-order (which WOULD break the wire) is a
        // test-visible event.
        let cow = bincode::serialize(&EntityKind::Cow).unwrap();
        assert_eq!(cow[0], 0, "Cow is the first variant — bincode ordinal 0");
        let cart = bincode::serialize(&EntityKind::Cart).unwrap();
        assert_eq!(cart[0], 19, "Cart is the last variant — bincode ordinal 19");
        // And it round-trips back to Cart (identity, the real guarantee).
        let back: EntityKind = bincode::deserialize(&cart).unwrap();
        assert_eq!(back, EntityKind::Cart);
    }

    #[test]
    fn safe_deserialize_rejects_truncated() {
        // Deliberately a 2-byte payload — nowhere near a full InputPacket.
        let result: Result<InputPacket, _> = safe_deserialize(&[0, 1]);
        assert!(result.is_err(), "expected decode error on truncated payload");
    }

    #[test]
    fn safe_deserialize_rejects_oversized_string() {
        // Craft a bincode payload with an enormous length prefix for a Vec/String
        // field. `safe_deserialize` caps at MAX_PACKET_SIZE; this should refuse.
        // InputPacket has `block_changes: Vec<BlockChange>` — the first 8 bytes
        // after the fixed fields are the Vec length (u64). We send all bytes as
        // 0xFF — effectively a length of u64::MAX, which exceeds the limit.
        let mut payload = vec![0u8; 8 * 20]; // enough for the fixed fields
        payload.extend_from_slice(&u64::MAX.to_le_bytes());
        let result: Result<InputPacket, _> = safe_deserialize(&payload);
        assert!(result.is_err(), "expected error on oversized length prefix");
    }

    #[test]
    fn decompress_chunk_rejects_bomb() {
        // Claimed size = 1 GB. Our cap is 16 KiB. Must reject without allocating.
        let mut bomb = (1_000_000_000u32).to_le_bytes().to_vec();
        bomb.extend_from_slice(&[0u8; 4]);
        let err = decompress_chunk(&bomb).unwrap_err();
        assert!(err.contains("bomb"), "error should mention bomb: {err}");
    }

    #[test]
    fn chunk_roundtrip() {
        let original = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let compressed = compress_chunk(&original);
        let back = decompress_chunk(&compressed).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn protocol_version_pinned() {
        // Pinning the version so a casual bump is a test-visible event — force
        // the bumper to update this test and think about compat.
        // v51 (2026-06-17): Spec 48 Electricity — BlockChange gains `meta: u8`
        //   and the chunk stream carries sparse per-block metadata.
        // v52 (2026-06-17): Operator Console (Spec B) — PacketType::OperatorSnapshot
        //   = 51 + OperatorSnapshotPacket; server → operator-player only.
        // v53 (2026-06-19): Texture packs — ResourcePackSuggest = 52 + packet.
        // v54 (2026-06-19): Creator Gallery — JoinAcceptPacket gains a final
        //   `exhibits: Vec<Exhibit>` so a joining client renders authored 2D art.
        // v55 (2026-06-20): Explosives (Spec 49) — new block/material ids +
        //   Composter block-entity + BlastingKeg PowerDevice kind.
        // v56 (2026-07-06): Pets-debt-water wave — EntityKind gained Crab (and
        //   earlier Fish/Fox/Cat/Donkey/Mule); no packet shape changed.
        // v57 (2026-07-11): death-drops phase 2 — EntityKind::Item + the
        //   item_kind/item_id/item_count stack payload on EntitySpawn
        //   (packet shape CHANGED).
        // v58 (2026-07-11): death-drops phase 2b — PacketType::InventoryGrant
        //   (53) + InventoryGrantPacket, server → client stack delivery on
        //   remote pickup. Additive (no existing shape changed); bumped so an
        //   old client never sees an unknown packet tag.
        // v59 (2026-09-03): weather sync (P9) — StateUpdatePacket gains a
        //   trailing rain_ticks_left/storm_ticks_left pair so a joined
        //   client's rain window matches the server's. Additive
        //   (#[serde(default)], no existing shape changed).
        // v60 (2026-09-05): World chat Phase 2 — PacketType::ChatSay (54) +
        //   PacketType::ChatDeliver (55), plus their payload structs.
        //   Additive; no existing packet shape changed.
        // v61 (2026-09-06): death-drops phase 3 — WireItem full_item appended
        //   to EntitySpawn and InventoryGrantPacket, so tool/armour drops
        //   reach remote players with durability intact (packet shape
        //   CHANGED). Plans stay floor-bound.
        // v62 (2026-09-07): wind/copper/electricity — PacketType::
        //   DeviceInteract (56) + DeviceInteractPacket { pos }, client →
        //   server, so a joiner's lever/button/crank/mirror reaches the host
        //   that owns the power sim. Additive; no existing shape changed.
        // v63 (2026-09-27): join channel binding (audit fix B) —
        //   ChallengePacket LOSES its `origin` field (packet shape CHANGED).
        //   The joiner signs an origin it builds from its own transport's TLS
        //   exporter; the server recomputes it from its transport.
        // v64 (2026-09-28): QUIC game packets ride one reliable framed stream
        //   (u32 LE length + payload) instead of datagrams; 8 MiB outbound
        //   queue cap per QUIC client. No packet shape changed.
        // v65 (2026-10-06): JoinAccept gains world_rules + worldgen_version,
        //   JoinRequest gains worldgen_version (gap-audit T2-9).
        // v66 (2026-10-06): JoinRequest gains ws_host; a WS join signs
        //   `axenstax-join:ws-host:<host>` (T-JOIN-RELAY WebSocket residual).
        // v67 (2026-10-06, MP-A3): `PacketType::Respawn = 57` (C→S),
        //   `PlayerEventType::{Died, Respawned}` and `EntityKind::Projectile =
        //   39`, all appended — a dead joiner stays dead on the server until it
        //   asks, and server-side arrows reach joiners.
        // v68 (2026-10-07, MP-D2a): `EntityUpdate` gains velocity + flags and
        //   goes changed-only behind a per-client interest radius;
        //   `InputPacket` gains `armour_points` + `health_delta` (a joiner's
        //   health is the server's).
        // v69 (2026-10-07, B2a): chunk push — ChunkData side data +
        //   continuations, JoinRequest.render_distance, InputPacket.chunk_ack
        //   + chunk_drops + render_distance.
        // v70 (2026-10-07, MP-D2b): `EntityAttack = 58`, `EntityInteract =
        //   59` (`InteractKind::LeadToPost` included), `InteractOutcome = 60`,
        //   `KillEvent = 61`, `PlayerEventType::{DiedOf, ArmourWorn, Bred}`,
        //   `entity_flags::TETHERED` — joiners act on the server's mobs.
        // v71 (2026-10-07, B2b): touched columns — PacketType::ColumnLocal
        //   (tag 4, with the column's hash), JoinAccept.chunk_note_radius,
        //   InputPacket.column_mismatch.
        // v72 (2026-10-07, C1): `InputPacket.mined` — the server yields a
        //   joiner's breaks.
        // v73 (2026-10-07, C2a): `ItemAction = 62` (Eat, Sleep),
        //   `ItemActionOutcome = 63`, `StateUpdatePacket.own_hunger` — a
        //   joiner's hunger, eating and sleep are the server's.
        // v74 (2026-10-07, C2b):
        //   `ItemAction::Craft` (= 2) and `ItemAction::Drop` (= 3), both
        //   unanswered — a joiner's crafting and Q-drops are mirrored on the
        //   server.
        // v75 (2026-10-08, C3a-2a):
        //   `WindowOp = 64` (Click, OpenPlayer, OpenTable, SetAutoRefill) —
        //   the server mirrors a joiner's inventory window; `ItemAction::Craft`
        //   is unused.
        // v76 (2026-10-08, C3a-fix-1):
        //   trailing `window_event` on the four S→C carriers, `events_applied`
        //   on the five C→S packets judged against the window,
        //   `InputPacket.edit_hands`, `EntityAttackPacket.hotbar_slot`,
        //   `ItemAction::GrantUnfit` (= 4) — ordered server window events.
        // v77 (2026-10-08, C3b-1):
        //   `WireWindowOp::OpenContainer` (= 4) and `Container` (= 5),
        //   `WindowOpPacket.touched` and `.claims` (after `events_applied`),
        //   `ContainerOpened = 65`, `WindowSlotSet = 66` (trailing
        //   `window_event`) — shared chests, dispensers and furnaces.
        assert_eq!(PROTOCOL_VERSION, 77);
    }

    fn sample_accept() -> JoinAcceptPacket {
        JoinAcceptPacket {
            player_index: 2,
            seed: 777,
            spawn_x: 1.5,
            spawn_y: 70.0,
            spawn_z: -3.5,
            world_time: 1234,
            is_creative: false,
            play_mode: crate::play_mode::PlayMode::Survival,
            difficulty: "hard".into(),
            server_identity: None,
            exhibits: Vec::new(),
            world_rules: WorldRules {
                world_type: "flat".into(),
                ground: "sand".into(),
                water_depth: 5,
                is_workshop: false,
                time_lock: "day".into(),
                mobs_enabled: false,
                explosives_enabled: false,
                fire_spread_enabled: false,
                keep_inventory: true,
            },
            worldgen_version: 9,
            chunk_note_radius: 0,
        }
    }

    #[test]
    fn join_accept_carries_world_rules_and_worldgen_version() {
        let acc = sample_accept();
        let bytes = serialize_packet(PacketType::JoinAccept, &acc);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::JoinAccept);
        let back: JoinAcceptPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.world_rules, acc.world_rules, "every rule flag survives the wire");
        assert_eq!(back.worldgen_version, 9);
        assert_eq!(back.seed, 777);
    }

    #[test]
    fn join_request_carries_worldgen_version() {
        let mut req = crate::remote_client::build_join_request_guest("Stax", 0);
        assert_eq!(
            req.worldgen_version,
            crate::world::worldgen_fingerprint(),
            "a joiner announces its own generator version"
        );
        req.worldgen_version = 41;
        let bytes = serialize_packet(PacketType::JoinRequest, &req);
        let (_ptype, payload) = deserialize_header(&bytes).unwrap();
        let back: JoinRequestPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.worldgen_version, 41);
    }

    #[test]
    fn an_older_join_request_still_yields_its_protocol_version() {
        // A v64 client's JoinRequest is one field shorter, so it no longer
        // decodes; the server must still read its version to explain why.
        #[derive(serde::Serialize)]
        struct V64JoinRequest {
            protocol_version: u32,
            player_name: String,
            auth_event: Option<crate::signet::SignetAuthEventWire>,
            handle_credential: Option<crate::signet::SignetCredentialWire>,
            skin_key: u64,
            client_nonce_hex: String,
        }
        let old = V64JoinRequest {
            protocol_version: 64,
            player_name: "Old".into(),
            auth_event: None,
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
        };
        let bytes = serialize_packet(PacketType::JoinRequest, &old);
        let (_ptype, payload) = deserialize_header(&bytes).unwrap();
        assert!(
            safe_deserialize::<JoinRequestPacket>(payload).is_err(),
            "the shorter shape does not decode (bincode is positional)"
        );
        assert_eq!(peek_protocol_version(payload), Some(64));
        assert_eq!(peek_protocol_version(&[1, 2]), None);
        assert_eq!(
            protocol_mismatch_reason(64),
            format!("Protocol mismatch: client v64, server v{PROTOCOL_VERSION}")
        );
    }

    #[test]
    fn world_rules_default_is_a_fresh_worlds_rules() {
        let r = WorldRules::default();
        assert_eq!(r.world_type, "normal");
        assert_eq!(r.time_lock, "cycle");
        assert!(r.mobs_enabled && r.explosives_enabled && r.fire_spread_enabled);
        assert!(!r.keep_inventory && !r.is_workshop);
    }

    #[test]
    fn world_rules_round_trip_through_a_world_meta() {
        let rules = sample_accept().world_rules;
        let mut meta = crate::save::WorldMeta::new("joined");
        rules.apply_to_meta(&mut meta);
        assert_eq!(WorldRules::from_meta(&meta), rules);
    }

    /// MP-D2b (v70) — the four appended packets keep their tags and shapes.
    #[test]
    fn joiner_action_packets_round_trip() {
        let attack = EntityAttackPacket {
            seq: 7,
            entity: 42,
            held_kind: item_kind::TOOL,
            held_id: 3,
            held_full: WireItem::Tool { tool_type: 1, material: 2, durability: 99 },
            sprint: true,
            sneak: false,
            hotbar_slot: 6,
            events_applied: 0x0102_0304,
        };
        let bytes = serialize_packet(PacketType::EntityAttack, &attack);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::EntityAttack);
        assert_eq!(safe_deserialize::<EntityAttackPacket>(payload).unwrap(), attack);
        // v76 (C3a-fix-1) — appended in order: hotbar_slot u8, then
        // events_applied u32, closing the packet.
        assert_eq!(&payload[payload.len() - 5..], &[6, 4, 3, 2, 1]);

        let interact = EntityInteractPacket {
            seq: 8,
            entity: 43,
            kind: InteractKind::SitToggle,
            held_kind: item_kind::EMPTY,
            held_id: 0,
            held_full: WireItem::None,
            hotbar_slot: 4,
            sneak: true,
            events_applied: 9,
        };
        let bytes = serialize_packet(PacketType::EntityInteract, &interact);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::EntityInteract);
        assert_eq!(safe_deserialize::<EntityInteractPacket>(payload).unwrap(), interact);
        // Review D2b B3 — the fence-post transfer carries its post.
        let to_post = EntityInteractPacket {
            kind: InteractKind::LeadToPost { post: [-3, 64, 1_000_000] },
            entity: 0,
            ..interact.clone()
        };
        let bytes = serialize_packet(PacketType::EntityInteract, &to_post);
        let (_, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(safe_deserialize::<EntityInteractPacket>(payload).unwrap(), to_post);

        let outcome = InteractOutcomePacket {
            seq: 8,
            entity: 43,
            kind: Some(InteractKind::Milk),
            accepted: true,
            consume_held: 1,
            note: 2,
            window_event: 0x0A0B_0C0D,
        };
        let bytes = serialize_packet(PacketType::InteractOutcome, &outcome);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::InteractOutcome);
        assert_eq!(safe_deserialize::<InteractOutcomePacket>(payload).unwrap(), outcome);
        assert_eq!(&payload[payload.len() - 4..], &0x0A0B_0C0Du32.to_le_bytes(), "v76 window_event closes it");
        assert_eq!(safe_deserialize::<EntityInteractPacket>(
            deserialize_header(&serialize_packet(PacketType::EntityInteract, &interact)).unwrap().1
        ).unwrap().events_applied, 9);

        let kill = KillEventPacket {
            victim: EntityKind::Nostrich,
            reason: kill_reason::LAST_HIT,
            x: 1.0,
            y: 2.0,
            z: 3.0,
            victim_flags: entity_flags::TAMED,
        };
        let bytes = serialize_packet(PacketType::KillEvent, &kill);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::KillEvent);
        assert_eq!(safe_deserialize::<KillEventPacket>(payload).unwrap(), kill);
        for (tag, t) in [
            (58u8, PacketType::EntityAttack),
            (59, PacketType::EntityInteract),
            (60, PacketType::InteractOutcome),
            (61, PacketType::KillEvent),
        ] {
            assert_eq!(t as u8, tag, "wire-stable tag");
        }
    }

    /// C2a (v73) — the item-action request and its outcome keep their tags
    /// and shapes, and the `ItemAction` variants their wire order (append
    /// only: Eat = 0, Sleep = 1, C2b's Craft = 2 and Drop = 3, v76's
    /// GrantUnfit = 4).
    #[test]
    fn item_action_packets_round_trip() {
        let eat = ItemActionPacket {
            seq: 11,
            action: ItemAction::Eat {
                hotbar_slot: 3,
                held_kind: item_kind::MATERIAL,
                held_id: 17,
                held_full: WireItem::None,
            },
            events_applied: 0x0102_0304,
        };
        let bytes = serialize_packet(PacketType::ItemAction, &eat);
        assert_eq!(bytes[0], 62, "wire-stable tag");
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::ItemAction);
        assert_eq!(safe_deserialize::<ItemActionPacket>(payload).unwrap(), eat);
        // The enum's variant index leads the action: Eat = 0.
        assert_eq!(&payload[4..8], &0u32.to_le_bytes());
        // v76 — events_applied closes the packet.
        assert_eq!(&payload[payload.len() - 4..], &0x0102_0304u32.to_le_bytes());

        let sleep =
            ItemActionPacket { seq: 12, action: ItemAction::Sleep { bed: [-5, 70, 1_000_000] }, events_applied: 0 };
        let bytes = serialize_packet(PacketType::ItemAction, &sleep);
        let (_, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(safe_deserialize::<ItemActionPacket>(payload).unwrap(), sleep);
        assert_eq!(&payload[4..8], &1u32.to_le_bytes(), "Sleep = 1");

        // C2b (v74) — Craft = 2 and Drop = 3, appended after Sleep.
        let mut grid = [(item_kind::EMPTY, 0u16); 9];
        grid[0] = (item_kind::MATERIAL, 3);
        grid[4] = (item_kind::BLOCK, 5);
        for table in [None, Some([7, 64, -9])] {
            let craft = ItemActionPacket { seq: 13, action: ItemAction::Craft { grid, table }, events_applied: 0 };
            let bytes = serialize_packet(PacketType::ItemAction, &craft);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<ItemActionPacket>(payload).unwrap(), craft);
            assert_eq!(&payload[4..8], &2u32.to_le_bytes(), "Craft = 2");
            assert_eq!(peek_item_action_variant(payload), Some(2));
        }
        let drop = ItemActionPacket {
            seq: 14,
            action: ItemAction::Drop {
                hotbar_slot: 8,
                held_kind: item_kind::TOOL,
                held_id: 2,
                held_full: WireItem::Tool { tool_type: 1, material: 2, durability: 77 },
            },
            events_applied: 0,
        };
        let bytes = serialize_packet(PacketType::ItemAction, &drop);
        assert_eq!(bytes[0], 62, "still the ItemAction tag: no new PacketType");
        let (_, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(safe_deserialize::<ItemActionPacket>(payload).unwrap(), drop);
        assert_eq!(&payload[4..8], &3u32.to_le_bytes(), "Drop = 3");
        assert_eq!(peek_item_action_variant(payload), Some(item_action_variant::DROP));
        assert_eq!(peek_item_action_variant(&payload[..7]), None, "too short to say");
        // v76 (C3a-fix-1) — GrantUnfit = 4, appended after Drop: the grant's
        // window event (u32), then the count (u8).
        let unfit = ItemActionPacket { seq: 15, action: ItemAction::GrantUnfit { event: 0x0506_0708, count: 9 }, events_applied: 3 };
        let bytes = serialize_packet(PacketType::ItemAction, &unfit);
        let (_, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(safe_deserialize::<ItemActionPacket>(payload).unwrap(), unfit);
        assert_eq!(&payload[4..8], &4u32.to_le_bytes(), "GrantUnfit = 4");
        assert_eq!(&payload[8..12], &0x0506_0708u32.to_le_bytes());
        assert_eq!(payload[12], 9);
        assert_ne!(peek_item_action_variant(payload), Some(item_action_variant::DROP), "not paced as a drop");

        let outcome = ItemActionOutcomePacket { seq: 12, accepted: false, consume_held: 0, note: 5, window_event: 0x0102_0304 };
        let bytes = serialize_packet(PacketType::ItemActionOutcome, &outcome);
        assert_eq!(bytes[0], 63, "wire-stable tag");
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::ItemActionOutcome);
        assert_eq!(safe_deserialize::<ItemActionOutcomePacket>(payload).unwrap(), outcome);
        assert_eq!(&payload[payload.len() - 4..], &0x0102_0304u32.to_le_bytes(), "v76 window_event closes it");
        for (tag, t) in [(62u8, PacketType::ItemAction), (63, PacketType::ItemActionOutcome)] {
            assert_eq!(t as u8, tag, "wire-stable tag");
            assert_eq!(deserialize_header(&[tag, 0]).map(|(p, _)| p), Some(t));
        }
    }

    /// C3a-2a (v75) — `WindowOp = 64`: every `WireWindowOp` and every
    /// `WindowClick` round-trips, their variant order is pinned on the wire
    /// bytes (both enums are append-only), and a drag's slot list over 45
    /// doesn't decode.
    #[test]
    fn window_op_packets_round_trip() {
        use crate::crafting::CraftSlot;
        use crate::window::{WindowClick, WindowSlot, MAX_DRAG_SLOTS};
        let mut example = [[CraftSlot::Empty; 3]; 3];
        example[0][0] = CraftSlot::Block(5);
        example[1][1] = CraftSlot::Material(crate::item::MaterialId::Stick);
        let clicks = [
            WindowClick::Slot { slot: 35, right: true },
            WindowClick::Grid { row: 2, col: 1, right: false },
            WindowClick::Armour { slot: 3 },
            WindowClick::Result,
            WindowClick::Trash,
            WindowClick::DragDistribute { slots: vec![WindowSlot::Inv(9), WindowSlot::Grid(1, 2)] },
            WindowClick::DragGather { slots: vec![WindowSlot::Grid(0, 0)] },
            WindowClick::Sort,
            WindowClick::ToggleLock { slot: 4 },
            WindowClick::Autofill { example },
            WindowClick::Close,
        ];
        for (index, click) in clicks.into_iter().enumerate() {
            let pkt = WindowOpPacket {
                op_seq: 7,
                op: WireWindowOp::Click(click),
                digest: 0xDEAD_BEEF,
                events_applied: 0x0102_0304,
                touched: Vec::new(),
                claims: Vec::new(),
            };
            let bytes = serialize_packet(PacketType::WindowOp, &pkt);
            assert_eq!(bytes[0], 64, "wire-stable tag");
            let (ptype, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(ptype, PacketType::WindowOp);
            assert_eq!(safe_deserialize::<WindowOpPacket>(payload).unwrap(), pkt);
            // op_seq (u32), then the op's variant (Click = 0), then the click's.
            assert_eq!(&payload[4..8], &0u32.to_le_bytes(), "Click = 0");
            assert_eq!(&payload[8..12], &(index as u32).to_le_bytes(), "WindowClick variant {index}");
            // v76 — digest, then events_applied; v77 — then `touched` and
            // `claims` (each an empty list: a u64 length of 0) close the
            // packet. Append order: main's fields first.
            let tail = &payload[payload.len() - 24..];
            assert_eq!(&tail[0..4], &0xDEAD_BEEFu32.to_le_bytes(), "digest");
            assert_eq!(&tail[4..8], &0x0102_0304u32.to_le_bytes(), "events_applied (v76)");
            assert_eq!(&tail[8..16], &0u64.to_le_bytes(), "touched (v77)");
            assert_eq!(&tail[16..24], &0u64.to_le_bytes(), "claims (v77)");
        }
        let others = [
            (WireWindowOp::OpenPlayer, 1u32),
            (WireWindowOp::OpenTable { cell: [-3, 64, 1_000_000] }, 2),
            (WireWindowOp::SetAutoRefill { on: false }, 3),
        ];
        for (op, index) in others {
            let pkt = WindowOpPacket { op_seq: u32::MAX, op, digest: 1, events_applied: 0, touched: Vec::new(), claims: Vec::new() };
            let bytes = serialize_packet(PacketType::WindowOp, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<WindowOpPacket>(payload).unwrap(), pkt);
            assert_eq!(&payload[4..8], &index.to_le_bytes(), "WireWindowOp variant {index}");
        }
        // WindowSlot: Inv = 0, Grid = 1.
        let grid = WindowOpPacket {
            op_seq: 1,
            op: WireWindowOp::Click(WindowClick::DragGather { slots: vec![WindowSlot::Grid(2, 0)] }),
            digest: 0,
            events_applied: 0,
            touched: Vec::new(),
            claims: Vec::new(),
        };
        let bytes = serialize_packet(PacketType::WindowOp, &grid);
        // tag, op_seq, Click, DragGather, the list's u64 length, then Grid = 1.
        assert_eq!(&bytes[1 + 12 + 8..1 + 12 + 12], &1u32.to_le_bytes(), "WindowSlot::Grid = 1");
        // The drag bound: 45 decode, 46 don't.
        for (n, decodes) in [(MAX_DRAG_SLOTS, true), (MAX_DRAG_SLOTS + 1, false)] {
            let pkt = WindowOpPacket {
                op_seq: 1,
                op: WireWindowOp::Click(WindowClick::DragDistribute { slots: vec![WindowSlot::Inv(0); n] }),
                digest: 0,
                events_applied: 0,
                touched: Vec::new(),
                claims: Vec::new(),
            };
            let bytes = serialize_packet(PacketType::WindowOp, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<WindowOpPacket>(payload).is_ok(), decodes, "{n} slots");
        }
        assert_eq!(PacketType::WindowOp as u8, 64);
        assert_eq!(deserialize_header(&[64, 0]).map(|(p, _)| p), Some(PacketType::WindowOp));
        // C3b-1 (v77) — 65 and 66 are taken (`container_packets_round_trip`).
        assert_eq!(deserialize_header(&[67, 0]), None, "nothing past 66 yet");
    }

    /// C3b-1 (v77) — shared containers on the wire: `OpenContainer` (= 4)
    /// and `Container` (= 5) ops with every `ContainerClick` (variant order
    /// pinned on the wire bytes), `WindowOpPacket.touched` and its bound,
    /// `ContainerOpened = 65` and `WindowSlotSet = 66` round trips with
    /// their bounds, `WireWindowSlot`, `ContainerKind`, `OpenRefusal`, the
    /// furnace's `SlotKind`/`ClickMode` order, and the Plan placeholder kind.
    #[test]
    fn container_packets_round_trip() {
        fn stone_claim() -> WireSlot {
            Some(WireStack { item_kind: item_kind::BLOCK, item_id: 1, count: 5, full_item: WireItem::None })
        }
        use crate::chest::ChestTier;
        use crate::container_window::{ContainerClick, ContainerKind};
        use crate::furnace::{ClickMode, SlotKind};
        let clicks = [
            ContainerClick::Withdraw { slot: 71, all: true },
            ContainerClick::Deposit { slot: 35, all: false },
            ContainerClick::Sort,
            ContainerClick::DumpMatching,
            ContainerClick::Restock,
            ContainerClick::TakeAll,
            ContainerClick::Furnace { kind: SlotKind::Fuel, mode: ClickMode::Stack, hotbar: 8 },
        ];
        for (index, click) in clicks.into_iter().enumerate() {
            let pkt = WindowOpPacket {
                op_seq: 3,
                op: WireWindowOp::Container(click),
                digest: 9,
                events_applied: 3,
                touched: vec![WireWindowSlot::Inv(4), WireWindowSlot::Container(71)],
                claims: vec![(WireWindowSlot::Inv(4), None)],
            };
            let bytes = serialize_packet(PacketType::WindowOp, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<WindowOpPacket>(payload).unwrap(), pkt);
            assert_eq!(&payload[4..8], &5u32.to_le_bytes(), "Container = 5");
            assert_eq!(&payload[8..12], &(index as u32).to_le_bytes(), "ContainerClick variant {index}");
        }
        let open = WindowOpPacket {
            op_seq: 1,
            op: WireWindowOp::OpenContainer { cell: [-5, 70, 9] },
            digest: 2,
            events_applied: 0,
            touched: Vec::new(),
            claims: Vec::new(),
        };
        let bytes = serialize_packet(PacketType::WindowOp, &open);
        let (_, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(safe_deserialize::<WindowOpPacket>(payload).unwrap(), open);
        assert_eq!(&payload[4..8], &4u32.to_le_bytes(), "OpenContainer = 4");
        // The furnace click's own enums: SlotKind Input/Fuel/Output = 0/1/2,
        // ClickMode Single/Stack = 0/1 (after the click's variant u32).
        let furnace = WindowOpPacket {
            op_seq: 1,
            op: WireWindowOp::Container(ContainerClick::Furnace { kind: SlotKind::Output, mode: ClickMode::Single, hotbar: 0 }),
            digest: 0,
            events_applied: 0,
            touched: Vec::new(),
            claims: Vec::new(),
        };
        let bytes = serialize_packet(PacketType::WindowOp, &furnace);
        assert_eq!(&bytes[1 + 12..1 + 16], &2u32.to_le_bytes(), "SlotKind::Output = 2");
        assert_eq!(&bytes[1 + 16..1 + 20], &0u32.to_le_bytes(), "ClickMode::Single = 0");
        // touched is bounded at 122.
        for (n, decodes) in [(MAX_WINDOW_SLOTS, true), (MAX_WINDOW_SLOTS + 1, false)] {
            let pkt = WindowOpPacket {
                op_seq: 1,
                op: WireWindowOp::Container(ContainerClick::Sort),
                digest: 0,
                events_applied: 0,
                touched: vec![WireWindowSlot::Cursor; n],
                claims: Vec::new(),
            };
            let bytes = serialize_packet(PacketType::WindowOp, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<WindowOpPacket>(payload).is_ok(), decodes, "{n} touched");
        }
        // claims too.
        for (n, decodes) in [(MAX_WINDOW_SLOTS, true), (MAX_WINDOW_SLOTS + 1, false)] {
            let pkt = WindowOpPacket {
                op_seq: 1,
                op: WireWindowOp::Container(ContainerClick::Restock),
                digest: 0,
                events_applied: 0,
                touched: Vec::new(),
                claims: vec![(WireWindowSlot::Inv(0), stone_claim()); n],
            };
            let bytes = serialize_packet(PacketType::WindowOp, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<WindowOpPacket>(payload).is_ok(), decodes, "{n} claims");
        }
        // The trailing order (v77): events_applied (v76), then touched, then
        // claims, each list a u64 length then its items.
        let order = WindowOpPacket {
            op_seq: 1,
            op: WireWindowOp::Container(ContainerClick::Deposit { slot: 2, all: true }),
            digest: 0x0A0B_0C0D,
            events_applied: 0x0102_0304,
            touched: vec![WireWindowSlot::Inv(2)],
            claims: vec![(WireWindowSlot::Inv(2), None)],
        };
        let bytes = bincode::serialize(&order).unwrap();
        let mut tail = Vec::new();
        tail.extend_from_slice(&0x0A0B_0C0Du32.to_le_bytes());
        tail.extend_from_slice(&0x0102_0304u32.to_le_bytes());
        tail.extend_from_slice(&1u64.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes()); // WireWindowSlot::Inv
        tail.push(2);
        tail.extend_from_slice(&1u64.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes()); // WireWindowSlot::Inv
        tail.push(2);
        tail.push(0); // None
        assert_eq!(&bytes[bytes.len() - tail.len()..], &tail[..], "digest, events_applied, touched, claims");
        assert_eq!(MAX_WINDOW_SLOTS, 72 + 36 + 4 + 1 + 9);

        // ContainerOpened = 65: every kind, a refusal, the slot bound.
        let stone = Some(WireStack { item_kind: item_kind::BLOCK, item_id: 1, count: 5, full_item: WireItem::None });
        let pick = Some(WireStack {
            item_kind: item_kind::TOOL,
            item_id: 2,
            count: 1,
            full_item: WireItem::Tool { tool_type: 0, material: 2, durability: 99 },
        });
        let plan = Some(WireStack { item_kind: item_kind::PLAN, item_id: 0, count: 1, full_item: WireItem::None });
        let kinds = [
            (ContainerKind::Chest { tier: ChestTier::Satori }, 0u32),
            (ContainerKind::Dispenser, 1),
            (ContainerKind::Dropper, 2),
            (ContainerKind::Furnace, 3),
        ];
        for (kind, index) in kinds {
            let pkt = ContainerOpenedPacket {
                cell: [1, 2, 3],
                kind,
                slots: vec![stone.clone(), None, pick.clone(), plan.clone()],
                furnace: Some(FurnaceView { smelt_progress: 40, smelt_total: 200, fuel_ticks_remaining: 1500, lit: true }),
                refused: None,
            };
            let bytes = serialize_packet(PacketType::ContainerOpened, &pkt);
            assert_eq!(bytes[0], 65, "wire-stable tag");
            let (ptype, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(ptype, PacketType::ContainerOpened);
            assert_eq!(safe_deserialize::<ContainerOpenedPacket>(payload).unwrap(), pkt);
            assert_eq!(&payload[12..16], &index.to_le_bytes(), "ContainerKind variant {index}");
        }
        for (refusal, index) in [
            (OpenRefusal::OutOfReach, 0u8),
            (OpenRefusal::Protected, 1),
            (OpenRefusal::NotAContainer, 2),
            (OpenRefusal::NotInWorld, 3),
        ] {
            let pkt = ContainerOpenedPacket {
                cell: [0, 0, 0],
                kind: ContainerKind::Dispenser,
                slots: Vec::new(),
                furnace: None,
                refused: Some(refusal),
            };
            let bytes = serialize_packet(PacketType::ContainerOpened, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<ContainerOpenedPacket>(payload).unwrap(), pkt);
            // cell 12, kind 4, slots length 8, furnace None 1, Some 1, then the refusal.
            assert_eq!(&payload[26..30], &u32::from(index).to_le_bytes(), "OpenRefusal variant {index}");
        }
        for (n, decodes) in [(MAX_CONTAINER_SLOTS, true), (MAX_CONTAINER_SLOTS + 1, false)] {
            let pkt = ContainerOpenedPacket {
                cell: [0, 0, 0],
                kind: ContainerKind::Chest { tier: ChestTier::Wood },
                slots: vec![None; n],
                furnace: None,
                refused: None,
            };
            let bytes = serialize_packet(PacketType::ContainerOpened, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<ContainerOpenedPacket>(payload).is_ok(), decodes, "{n} slots");
        }

        // WindowSlotSet = 66: every WireWindowSlot (Inv/Armour/Cursor/Grid/
        // Container = 0..4), both reasons, the bound.
        let slots = [
            (WireWindowSlot::Inv(35), 0u32),
            (WireWindowSlot::Armour(3), 1),
            (WireWindowSlot::Cursor, 2),
            (WireWindowSlot::Grid(2, 1), 3),
            (WireWindowSlot::Container(71), 4),
        ];
        for (at, index) in slots {
            let pkt = WindowSlotSetPacket {
                op_seq_applied: 12,
                reason: slot_set_reason::CORRECTION,
                sets: vec![(at, stone.clone())],
                furnace: None,
                window_event: 0,
            };
            let bytes = serialize_packet(PacketType::WindowSlotSet, &pkt);
            assert_eq!(bytes[0], 66, "wire-stable tag");
            let (ptype, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(ptype, PacketType::WindowSlotSet);
            assert_eq!(safe_deserialize::<WindowSlotSetPacket>(payload).unwrap(), pkt);
            // op_seq_applied 4, reason 1, the list's u64 length 8, then the slot.
            assert_eq!(&payload[13..17], &index.to_le_bytes(), "WireWindowSlot variant {index}");
        }
        assert_eq!((slot_set_reason::CORRECTION, slot_set_reason::CHANGED), (0, 1));
        let push = WindowSlotSetPacket {
            op_seq_applied: 0,
            reason: slot_set_reason::CHANGED,
            sets: vec![(WireWindowSlot::Container(0), None)],
            furnace: Some(FurnaceView::default()),
            window_event: 0,
        };
        let bytes = serialize_packet(PacketType::WindowSlotSet, &push);
        let (_, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(safe_deserialize::<WindowSlotSetPacket>(payload).unwrap(), push);
        for (n, decodes) in [(MAX_WINDOW_SLOTS, true), (MAX_WINDOW_SLOTS + 1, false)] {
            let pkt = WindowSlotSetPacket {
                op_seq_applied: 0,
                reason: slot_set_reason::CHANGED,
                sets: vec![(WireWindowSlot::Cursor, None); n],
                furnace: None,
                window_event: 0,
            };
            let bytes = serialize_packet(PacketType::WindowSlotSet, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            assert_eq!(safe_deserialize::<WindowSlotSetPacket>(payload).is_ok(), decodes, "{n} sets");
        }
        // v77 — a correction that changes player slots is a numbered window
        // event: `window_event` closes the packet.
        let numbered = WindowSlotSetPacket {
            op_seq_applied: 9,
            reason: slot_set_reason::CORRECTION,
            sets: vec![(WireWindowSlot::Inv(3), None)],
            furnace: None,
            window_event: 0x0102_0304,
        };
        let bytes = serialize_packet(PacketType::WindowSlotSet, &numbered);
        let (_, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(safe_deserialize::<WindowSlotSetPacket>(payload).unwrap(), numbered);
        assert_eq!(&payload[payload.len() - 4..], &0x0102_0304u32.to_le_bytes(), "window_event last");
        assert_eq!(item_kind::PLAN, 4, "the reserved Plan placeholder kind");
        assert_eq!((PacketType::ContainerOpened as u8, PacketType::WindowSlotSet as u8), (65, 66));
    }

    /// bincode 1 is positional, so `StateUpdatePacket`'s trailing fields must
    /// sit in the order each bump appended them: P9's weather windows (v59),
    /// then C2a's `own_hunger` (v73). Pinned on the wire bytes.
    #[test]
    fn state_update_trailing_fields_are_in_append_order() {
        let pkt = StateUpdatePacket {
            tick: 1,
            players: vec![],
            block_changes: vec![],
            world_time: 2,
            last_acked_input: 3,
            entity_spawns: vec![],
            entity_updates: vec![],
            entity_despawns: vec![],
            reserve_richness: 0.0,
            reserve_target_sats: 0,
            reserve_current_sats: 0,
            rain_ticks_left: 0x0102_0304,
            storm_ticks_left: 0x0506_0708,
            own_hunger: 0x11,
        };
        let bytes = bincode::serialize(&pkt).unwrap();
        let mut tail = Vec::new();
        // v59 (P9): rain_ticks_left u32, storm_ticks_left u32.
        tail.extend_from_slice(&0x0102_0304u32.to_le_bytes());
        tail.extend_from_slice(&0x0506_0708u32.to_le_bytes());
        // v73 (C2a): own_hunger u8.
        tail.push(0x11);
        assert_eq!(&bytes[bytes.len() - tail.len()..], &tail[..]);
        let back: StateUpdatePacket = safe_deserialize(&bytes).unwrap();
        assert_eq!(back.own_hunger, 0x11);
    }

    #[test]
    fn respawn_request_and_life_events_round_trip() {
        let bytes = serialize_packet(PacketType::Respawn, &());
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::Respawn, "header tag 57 maps back");
        assert!(payload.is_empty(), "a Respawn request asserts nothing but the wish");
        for event in [
            PlayerEventType::Died,
            PlayerEventType::Respawned { x: 1.5, y: 70.0, z: -3.5 },
            PlayerEventType::DiedOf { cause: WireDamageCause::Mob(EntityKind::Shark) },
            PlayerEventType::ArmourWorn { hits: 3 },
            PlayerEventType::Bred { offspring: EntityKind::Mule },
        ] {
            let pkt = PlayerEventPacket { player_index: 4, event, window_event: 0x0102_0304 };
            let bytes = serialize_packet(PacketType::PlayerEvent, &pkt);
            let (_, payload) = deserialize_header(&bytes).unwrap();
            let back: PlayerEventPacket = safe_deserialize(payload).unwrap();
            assert_eq!(back.player_index, 4);
            assert_eq!(back.event, pkt.event);
            // v76 — the window event closes the packet.
            assert_eq!(back.window_event, 0x0102_0304);
            assert_eq!(&payload[payload.len() - 4..], &0x0102_0304u32.to_le_bytes());
        }
    }

    #[test]
    fn projectile_entity_kind_is_appended() {
        // Wire-stable: Item stays 38; Projectile takes the next value.
        assert_eq!(EntityKind::Item as u8, 38);
        assert_eq!(EntityKind::Projectile as u8, 39);
    }

    #[test]
    fn join_request_round_trip_with_no_auth_fields() {
        // v3 default for clients that haven't migrated: both Option<*> are None.
        let req = JoinRequestPacket {
            protocol_version: PROTOCOL_VERSION,
            player_name: "Stax".to_string(),
            auth_event: None,
            handle_credential: None,
            skin_key: 0,
            client_nonce_hex: String::new(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: String::new(),
            render_distance: 0,
        };
        let bytes = serialize_packet(PacketType::JoinRequest, &req);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::JoinRequest);
        let back: JoinRequestPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.protocol_version, PROTOCOL_VERSION);
        assert_eq!(back.player_name, "Stax");
        assert!(back.auth_event.is_none());
        assert!(back.handle_credential.is_none());
        assert_eq!(back.skin_key, 0, "default skin reference round-trips as 0");
    }

    #[test]
    fn resource_pack_suggest_round_trips() {
        let suggest = ResourcePackSuggestPacket {
            name: "Crisp 32".to_string(),
            url: "https://axenstax.app/static/packs/crisp32/pack.json".to_string(),
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
            size_bytes: 4096,
            required: true,
        };
        let bytes = serialize_packet(PacketType::ResourcePackSuggest, &suggest);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::ResourcePackSuggest, "tag 52 decodes");
        let back: ResourcePackSuggestPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back, suggest, "all fields round-trip through bincode");
    }

    #[test]
    fn join_request_round_trip_with_auth_fields() {
        // Populated wire shape — proves the new fields bincode-survive
        // (incl. the Vec<u8> 64-byte signatures).
        use crate::signet::{SignetAuthEventWire, SignetCredentialWire};
        let auth = SignetAuthEventWire {
            pubkey: [0xaau8; 32],
            created_at: 1_700_000_000,
            kind: 21236,
            tags: vec![
                vec!["challenge".into(), "f".repeat(64)],
                vec!["origin".into(), "https://localhost:8094".into()],
            ],
            content: String::new(),
            id: [0xccu8; 32],
            sig: vec![0xdeu8; 64],
            from_np: false,
        };
        let cred = SignetCredentialWire {
            pubkey: [0xaau8; 32],
            created_at: 1_700_000_000,
            kind: 31000,
            tags: vec![vec!["display-name".into(), "Stax".into()]],
            content: String::new(),
            id: [0xddu8; 32],
            sig: vec![0xeeu8; 64],
        };
        let req = JoinRequestPacket {
            protocol_version: PROTOCOL_VERSION,
            player_name: "Stax".to_string(),
            auth_event: Some(auth.clone()),
            handle_credential: Some(cred.clone()),
            skin_key: 0x0123_4567_89AB_CDEF,
            client_nonce_hex: "abc123".to_string(),
            worldgen_version: crate::world::worldgen_fingerprint(),
            ws_host: "play.example.org:6767".to_string(),
            render_distance: 0,
        };
        let bytes = serialize_packet(PacketType::JoinRequest, &req);
        let (_ptype, payload) = deserialize_header(&bytes).unwrap();
        let back: JoinRequestPacket = safe_deserialize(payload).unwrap();
        let back_auth = back.auth_event.unwrap();
        assert_eq!(back_auth.pubkey, auth.pubkey);
        assert_eq!(back_auth.tags, auth.tags);
        assert_eq!(back_auth.sig, auth.sig);
        let back_cred = back.handle_credential.unwrap();
        assert_eq!(back_cred.tags, cred.tags);
        assert_eq!(back_cred.sig, cred.sig);
        assert_eq!(
            back.skin_key, 0x0123_4567_89AB_CDEF,
            "a populated skin reference survives the bincode round-trip"
        );
        assert_eq!(back.ws_host, "play.example.org:6767", "v66 dialled host round-trips");
    }

    #[test]
    fn challenge_packet_round_trip() {
        // v63: no `origin` — the client builds it from its own transport.
        let pkt = ChallengePacket {
            nonce_hex: "f".repeat(64),
        };
        let bytes = serialize_packet(PacketType::Challenge, &pkt);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::Challenge);
        let back: ChallengePacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.nonce_hex.len(), 64);
    }

    #[test]
    fn deserialize_header_recognises_challenge_tag() {
        let payload = [50u8, 1, 2, 3];
        let (ptype, rest) = deserialize_header(&payload).unwrap();
        assert_eq!(ptype, PacketType::Challenge);
        assert_eq!(rest, &[1, 2, 3]);
    }

    #[test]
    fn deserialize_header_empty_returns_none() {
        assert!(deserialize_header(&[]).is_none());
    }

    #[test]
    fn deserialize_header_unknown_tag_returns_none() {
        // 99 is not in the PacketType enum.
        assert!(deserialize_header(&[99, 0, 0]).is_none());
        // 0 is also unmapped — packet types start at 1.
        assert!(deserialize_header(&[0, 1, 2]).is_none());
    }

    #[test]
    fn deserialize_header_strips_tag_and_keeps_payload() {
        let payload = [10u8, 0xde, 0xad, 0xbe, 0xef];
        let (ptype, rest) = deserialize_header(&payload).unwrap();
        assert_eq!(ptype, PacketType::JoinRequest);
        assert_eq!(rest, &[0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn compress_decompress_roundtrip_beyond_4kib() {
        // Synthetic 8 KiB payload exercising the compressor above the 4 KiB
        // spec ask. (A real serialized sub-chunk is now 8704 bytes — 8192 block
        // bytes + the 512-byte placed mask — still well under the 16 KiB cap.)
        let mut payload = Vec::with_capacity(8192);
        for i in 0..4096u32 {
            let b = ((i % 16) as u16).to_le_bytes();
            payload.extend_from_slice(&b);
        }
        assert_eq!(payload.len(), 8192);

        let compressed = compress_chunk(&payload);
        let back = decompress_chunk(&compressed).expect("valid roundtrip");
        assert_eq!(back, payload);
    }
}

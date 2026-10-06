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
}

// ─── Handshake ───────────────────────────────────────────────

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
    /// The joiner's own [`crate::world::WORLDGEN_VERSION`] (v65, gap-audit
    /// T2-9). A joiner regenerates the host's terrain locally, so the host
    /// records this on the player (`ServerPlayer::worldgen_mismatch`) to know
    /// whose terrain may differ from its own. APPEND-ONLY: stays last.
    #[serde(default)]
    pub worldgen_version: u32,
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
    /// The host's [`crate::world::WORLDGEN_VERSION`] (v65). A joiner on a
    /// different version warns its player that terrain may look different.
    #[serde(default)]
    pub worldgen_version: u32,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
/// Phase D: client-authoritative — client sends its position directly.
/// The server relays positions to all clients without running physics.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InputPacket {
    /// Client tick number (for ordering and prediction reconciliation)
    pub tick: u64,
    /// Client-authoritative position (server relays, doesn't simulate)
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Look direction (absolute, not delta — server doesn't accumulate)
    pub yaw: f32,
    pub pitch: f32,
    /// Health (client-authoritative for Phase D)
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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

/// Server → client: an existing entity's position/state changed this tick.
/// Sent every tick for every entity whose state differs from last broadcast.
/// `state` is an AI-state tag for animation (idle=0, wander=1, chase=2, attack=3).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityUpdate {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub state: u8,
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
}

// ─── Chunk data (Server → Client, reliable stream) ───

/// A full chunk sent to a client (initial load or entering new area).
/// The block data is LZ4-compressed before network transmission.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkDataPacket {
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
    /// LZ4-compressed `Chunk::as_bytes()` — 8192 bytes of u16 block IDs plus
    /// the 512-byte player-placed mask = 8704 bytes uncompressed (Spec 6 §2.2).
    /// When the send path is wired, use `chunk.as_bytes()` (which includes the
    /// mask), not a hand-built block-only array. `from_bytes` accepts both the
    /// legacy 8192 and current 8704 lengths.
    pub compressed_blocks: Vec<u8>,
}

// ─── Player events ───

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PlayerEventType {
    /// `name` is the display handle (verified credential name, disambiguated, or
    /// guest fallback). `npub` is the joiner's full verified npub (NIP-19 bech32)
    /// for the client's inspect view, or `""` for a guest / unverified join —
    /// the inspect view lets a specific person be verified beyond the grindable
    /// collision suffix (Phase 4).
    Joined { name: String, npub: String },
    Left,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerEventPacket {
    pub player_index: u32,
    pub event: PlayerEventType,
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
pub const PROTOCOL_VERSION: u32 = 65;

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

/// Compress block data with LZ4 for chunk transmission. `decompress_chunk`
/// (below) IS live — game_loop.rs decompresses inbound chunk packets — but
/// nothing on the send side calls this yet to actually produce
/// `compressed_blocks` for a real remote client; round-trip tested here.
#[cfg_attr(not(test), allow(dead_code))]
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
        assert_eq!(super::PROTOCOL_VERSION, 65);
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
        };
        let bytes = serialize_packet(PacketType::InventoryGrant, &pkt);
        let (ptype, payload) = deserialize_header(&bytes).unwrap();
        assert_eq!(ptype, PacketType::InventoryGrant, "header tag 53 maps back");
        let back: InventoryGrantPacket = safe_deserialize(payload).unwrap();
        assert_eq!(back.item_kind, item_kind::MATERIAL);
        assert_eq!(back.item_id, 4);
        assert_eq!(back.count, 3);
        assert_eq!(back.full_item, WireItem::None);
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
        };
        let bytes = serialize_packet(PacketType::InventoryGrant, &pkt);
        let (_, payload) = deserialize_header(&bytes).unwrap();
        let mut payload = payload.to_vec();
        // Overwrite the first byte of the trailing (fixint u32) variant index
        // with an unassigned discriminant.
        let n = payload.len();
        payload[n - 4] = 9;
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
        };
        let bytes = bincode::serialize(&pkt).unwrap();
        let back: InputPacket = safe_deserialize(&bytes).unwrap();
        assert_eq!(back.tick, pkt.tick);
        assert_eq!(back.hotbar_slot, pkt.hotbar_slot);
        // Tool-capable held ref survives the round-trip.
        assert_eq!(back.held_kind, item_kind::TOOL);
        assert_eq!(back.held_id, 2);
        // Legacy block-only field still preserved alongside the new pair.
        assert_eq!(back.held_item, 7);
        assert_eq!(back.block_changes.len(), 1);
        assert_eq!(back.block_changes[0].new_block, 3);
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
            }],
            entity_despawns: vec![],
            reserve_richness: 0.0,
            reserve_target_sats: 0,
            reserve_current_sats: 0,
            rain_ticks_left: 0,
            storm_ticks_left: 0,
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
        assert_eq!(PROTOCOL_VERSION, 65);
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
            crate::world::WORLDGEN_VERSION,
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
            worldgen_version: crate::world::WORLDGEN_VERSION,
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
            worldgen_version: crate::world::WORLDGEN_VERSION,
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

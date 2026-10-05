//! Game server — authoritative world simulation.
//!
//! Owns all game state: block world, entities, combat, water, leaf decay,
//! mob spawning. Processes player inputs and produces world updates.
//! Runs at 20 TPS.

use glam::Vec3;

use crate::biome::BiomeGenerator;
use crate::block::{self, BlockRegistry};
use crate::chunk::CHUNK_SIZE;
use crate::combat::PlayerCombat;
use crate::entity;
use crate::inventory::Inventory;
use crate::leaf_decay::LeafDecaySystem;
use crate::physics::Player;
use crate::lava::LavaSystem;
use crate::water::WaterSystem;
use crate::world::World;

/// Maximum chunk Y coordinate for world generation.
const MAX_CHUNK_Y: i32 = 5;
// BRIDGE: `REACH_DISTANCE` (server-authoritative reach check) and
// `STREAM_BUDGET` (server-side chunk-stream pacing) are declared but not yet
// enforced here — reach checking and chunk streaming for real clients today
// live in the single-player client path (block_interact.rs / chunk_stream.rs),
// which GameServer doesn't run (see the "Single-player bypasses GameServer"
// note in CLAUDE.md). Wire these in when the dedicated server needs its own
// authoritative reach/stream enforcement.
/// Block reach distance.
#[allow(dead_code)]
const REACH_DISTANCE: f32 = 5.0;
/// Max columns to stream per frame.
#[allow(dead_code)]
const STREAM_BUDGET: usize = 4;

/// Per-player server-side state.
pub struct ServerPlayer {
    pub player: Player,
    pub inventory: Inventory,
    pub combat: PlayerCombat,
    pub hotbar_slot: usize,
    // BRIDGE: server-side crafting isn't wired up (the known `ServerPlayer` vs
    // `PlayerSlot` duplication in CLAUDE.md's tech-debt list) — crafting today
    // is client-authoritative even in the single-player-bypasses-GameServer
    // path. Kept here for when server-authoritative crafting lands.
    #[allow(dead_code)]
    pub crafting_ui: crate::craft_ui::CraftingUi,
    /// Camera yaw (radians) — relayed from client InputPacket for state broadcast.
    pub yaw: f32,
    /// Camera pitch (radians) — relayed from client InputPacket for state broadcast.
    pub pitch: f32,
    /// Currently held item (block ID) — relayed from client for rendering on other clients.
    pub held_item: u16,
    /// Tool-capable held-item wire ref relayed from the client's InputPacket
    /// (`item_kind::*` + id). Authoritative source of the broadcast held item
    /// for server-simulated (remote) players, whose `inventory`/`hotbar_slot`
    /// are not kept live server-side. EMPTY/0 until the first input arrives.
    pub held_kind: u8,
    pub held_id: u16,
    /// Latest intent received from this player (remote clients only; local
    /// players are position-authoritative). Consumed by
    /// `GameServer::tick_player_physics` when present.
    pub pending_intent: Option<crate::player_intent::PlayerIntent>,
    /// Intents that arrived while one was already pending (network jitter
    /// bunching two inputs into one tick). Consumed one per tick after
    /// `pending_intent`, so no movement step is dropped — and a client
    /// flooding inputs still only moves one step a tick. Bounded by
    /// [`MAX_QUEUED_INTENTS`] (oldest dropped). See [`ServerPlayer::queue_intent`].
    pub intent_queue: std::collections::VecDeque<crate::player_intent::PlayerIntent>,
    /// Whether this player is server-simulated (true for remote clients) vs
    /// client-position-trusted (true for local players). Set at connect time.
    pub server_simulated: bool,
    /// Whether the player's transport is still live. `hosted_server.rs` keeps
    /// slot indexes stable on disconnect (its parallel `transports` array), so
    /// the `ServerPlayer` lingers — this flag stops the ghost slot from acting
    /// on the world (e.g. hoovering item drops nobody can receive). Local
    /// players are always connected.
    pub connected: bool,
    /// Highest `InputPacket.tick` consumed from this player. Monotonic; older
    /// packets are dropped as replays. Zero means no input consumed yet.
    pub last_input_tick: u64,
    /// Phase 2 — sneak signal from the player's most recent input, retained
    /// for the per-tick avatar broadcast. `pending_intent` is consumed by
    /// `tick_player_physics` before `broadcast_state` runs, so the broadcast
    /// can't read it directly; we latch sneak/acting here at input time.
    pub last_sneak: bool,
    /// Phase 2 — "acting" (break_block || place_block) from the player's most
    /// recent input. Drives the SWINGING flag on the avatar. Latched at input
    /// time for the same reason as `last_sneak`.
    pub last_acting: bool,
    /// Phase 2 — magnitude of the player's most recent analog move input
    /// (`hypot(move_forward, move_right)`). Used as the walk signal for
    /// position-trusted local players, whose `player.velocity` is never
    /// server-simulated (only `pos` is overwritten from the InputPacket).
    /// Server-simulated players use real `player.velocity` instead.
    pub last_move_mag: f32,
    /// Skin reference for this player: a `u64` content hash
    /// (`CosmeticDescriptor::skin_key`). `0` = default skin (sentinel). Copied
    /// from the client's `JoinRequestPacket.skin_key` at join time and broadcast
    /// verbatim on every `PlayerState` (see `collect_player_state`). The skin
    /// BYTES are delivered out-of-band (gated). Defaults to `0`.
    pub skin_key: u64,
    /// Verified Signet pubkey (x-only, 32 bytes) for an authenticated remote
    /// player, set at join time from the verified `auth_event`. `None` for local
    /// players and for guest joins on an open server. This is the identity source
    /// for economy-block ownership (the `LocalPlayer(pidx)` → `Npub` convergence
    /// the economy specs are waiting on — Phase 4 of the Signet-auth spec).
    pub verified_pubkey: Option<[u8; 32]>,
    /// Final display handle: the verified credential name (disambiguated against
    /// other present players), or the sanitised asserted name for a guest. Empty
    /// for local players. Broadcast in the `Joined` player event.
    pub display_name: String,
    /// World chat (Phase 2) — this player's effective comms level, `min(charter
    /// ceiling, operator policy)`, resolved at join from Charter for a verified
    /// player; see `docs/foundations/2026-09-05-world-chat.md` §3.3. Defaults
    /// to `Blocked` — the most restrictive level — so a guest, a split-screen
    /// seat or an unsigned slot 0 hears nothing (room lines included) unless a
    /// verified player's own Charter/guardian policy raises it (audit
    /// 2026-09-27; §3.5).
    pub comms: crate::comms::Party,
    /// World chat (Phase 2) — this player's own address book: who they've
    /// classified, and as what tier. Phase 4 fills this in from Signet/
    /// Kenspeckle contacts. An empty book is safe by construction — `tier_of`
    /// falls back to `Stranger`, so an `Approved` player simply talks to
    /// nobody until contacts arrive; the gap can never widen who is heard.
    pub contacts: std::collections::HashMap<[u8; 32], crate::comms::Tier>,
    /// World chat (Phase 2) — this player's per-minute chat token bucket.
    /// See `crate::comms::RateLimiter`.
    pub chat_rate: crate::comms::RateLimiter,
}

/// Resolve the [`ItemRef`] a player is currently holding from the item in
/// their active hotbar slot. Empty / out-of-range slots resolve to `Empty`.
/// Correct only when the player's `inventory`/`hotbar_slot` are kept live
/// server-side — i.e. local (position-trusted) players. Remote players read
/// `held_kind`/`held_id` (client-authoritative) instead, since their
/// server-side inventory only reflects save-time state.
pub fn server_player_item_ref(sp: &ServerPlayer) -> crate::protocol::ItemRef {
    match sp.inventory.hotbar_slot(sp.hotbar_slot) {
        Some(stack) => crate::inventory::item_to_ref(&stack.item),
        None => crate::protocol::ItemRef::Empty,
    }
}

/// Resolve the wire `(held_kind, held_id)` pair to broadcast for a player,
/// picking the correct source for the player type:
///   - server-simulated (remote) players → the client-sent `held_kind`/
///     `held_id` relayed each tick (their server-side inventory only reflects
///     save-time state, so resolving from it would broadcast a stale item —
///     the Phase 2 regression this fixes).
///   - local (position-trusted) players → resolve from the host's own live
///     inventory via `server_player_item_ref`.
pub fn broadcast_held_ref(sp: &ServerPlayer) -> (u8, u16) {
    if sp.server_simulated {
        (sp.held_kind, sp.held_id)
    } else {
        server_player_item_ref(sp).to_wire()
    }
}

/// Locomotion state + flag bits from sim/input. Crouch is a flag, not a
/// state, so it can combine with idle/walk.
///
/// `anim_state`: 0 idle, 1 walk, 2 jump (in-air). `flags`: SWINGING iff
/// acting (break or place this tick), CROUCHING iff sneak, ON_GROUND iff
/// on_ground.
pub fn player_anim_fields(speed: f32, on_ground: bool, sneak: bool, acting: bool) -> (u8, u8) {
    use crate::protocol::player_flags as pf;
    const WALK_EPS: f32 = 0.05;
    let anim_state: u8 = if !on_ground {
        2
    } else if speed > WALK_EPS {
        1
    } else {
        0
    };
    let mut flags = 0u8;
    if acting {
        flags |= pf::SWINGING;
    }
    if sneak {
        flags |= pf::CROUCHING;
    }
    if on_ground {
        flags |= pf::ON_GROUND;
    }
    (anim_state, flags)
}

/// Build the broadcast PlayerState for one player. Single source of truth for
/// the per-player state-collection logic used by HostedServer::broadcast_state
/// and exercised directly by integration tests.
///
/// `idx` is the player slot index — `ServerPlayer` carries no index of its own,
/// so the caller (which iterates the players Vec) supplies it.
pub fn collect_player_state(sp: &ServerPlayer, idx: u32) -> crate::protocol::PlayerState {
    // Phase 2 — real held item + locomotion/flags. Source per player type
    // (remote: client-sent ref; local: live inventory) is encapsulated in
    // broadcast_held_ref.
    let (held_kind, held_id) = broadcast_held_ref(sp);
    // Server-simulated (remote) players have real physics: horizontal velocity
    // + on_ground come from the sim. Local position-trusted players don't run
    // server physics (only pos is overwritten from the InputPacket), so
    // velocity/on_ground are stale — fall back to the latched analog move
    // magnitude for the walk signal and assume grounded (the InputPacket
    // carries no on_ground signal). CONCERN: local players never report
    // jump/airborne; acceptable until single-player routes through server-sim
    // (Known debt).
    let (speed, on_ground) = if sp.server_simulated {
        let v = (sp.player.velocity.x.powi(2) + sp.player.velocity.z.powi(2)).sqrt();
        (v, sp.player.on_ground)
    } else {
        (sp.last_move_mag, true)
    };
    let (anim_state, flags) = player_anim_fields(speed, on_ground, sp.last_sneak, sp.last_acting);
    crate::protocol::PlayerState {
        player_index: idx,
        x: sp.player.pos.x,
        y: sp.player.pos.y,
        z: sp.player.pos.z,
        yaw: sp.yaw,
        pitch: sp.pitch,
        health: sp.combat.health,
        held_kind,
        held_id,
        anim_state,
        flags,
        // The one place a non-zero key flows on the server side: the per-player
        // reference announced at join time, rebroadcast every tick.
        skin_key: sp.skin_key,
    }
}

impl ServerPlayer {
    pub fn new(spawn: Vec3) -> Self {
        Self {
            player: Player::new(spawn),
            inventory: Inventory::new(),
            combat: PlayerCombat::new(),
            hotbar_slot: 0,
            crafting_ui: crate::craft_ui::CraftingUi::new(),
            yaw: 0.0,
            pitch: 0.0,
            held_item: 0,
            held_kind: crate::protocol::item_kind::EMPTY,
            held_id: 0,
            pending_intent: None,
            intent_queue: std::collections::VecDeque::new(),
            // Default: position-trusted. Caller flips to true for remote players.
            server_simulated: false,
            connected: true,
            last_input_tick: 0,
            last_sneak: false,
            last_acting: false,
            last_move_mag: 0.0,
            // Default skin until a JoinRequest announces a custom reference.
            skin_key: 0,
            // Identity set at join time (remote authenticated players only).
            verified_pubkey: None,
            display_name: String::new(),
            // Most restrictive until a verified identity's Charter/guardian
            // policy is resolved at join (§3.3, §3.5).
            comms: crate::comms::Party::at(crate::comms::CommsLevel::Blocked),
            // Phase 4 fills this in from Signet/Kenspeckle contacts.
            contacts: std::collections::HashMap::new(),
            chat_rate: crate::comms::RateLimiter::new(),
        }
    }
}

/// Most movement intents a server-simulated player may have waiting (a pending
/// one plus this many queued). At 20 TPS that bounds the backlog to a few
/// ticks of latency.
pub const MAX_QUEUED_INTENTS: usize = 3;

impl ServerPlayer {
    /// Take one client input's movement intent. The first waits in
    /// `pending_intent`; more arriving before the next tick queue behind it
    /// (bounded, oldest dropped). `GameServer::tick_player_physics` consumes
    /// exactly one per tick.
    pub fn queue_intent(&mut self, intent: crate::player_intent::PlayerIntent) {
        if self.pending_intent.is_none() && self.intent_queue.is_empty() {
            self.pending_intent = Some(intent);
            return;
        }
        if self.intent_queue.len() >= MAX_QUEUED_INTENTS {
            self.intent_queue.pop_front();
        }
        self.intent_queue.push_back(intent);
    }

    /// This player's own classification of `other`, from their own address
    /// book — directional and local (§2.1: "tier(A → B) is A's view of B").
    /// An empty book means everyone reads as `Stranger`, which is safe by
    /// construction: an `Approved` player simply talks to nobody until
    /// contacts arrive (Phase 4), never the other way round.
    pub fn tier_of(&self, other: &[u8; 32]) -> crate::comms::Tier {
        self.contacts
            .get(other)
            .copied()
            .unwrap_or(crate::comms::Tier::Stranger)
    }
}

/// Outcome of running one chat line through the world-chat pipeline
/// (`handle_chat_say`). `hosted_server.rs`'s `ChatSay` dispatch is the only
/// production caller; the pipeline is a free function over `&mut
/// [ServerPlayer]` rather than a method so `TestHost` (whose `server.players`
/// is already `pub`) can drive it directly in a test, with no real transport.
///
/// Native-only — the web build carries no chat surface at all
/// (`docs/foundations/2026-09-05-world-chat.md` §6); its only caller
/// (`hosted_server::handle_chat_say_packet`) is native-only too.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, PartialEq)]
pub enum ChatSayOutcome {
    /// No verified key: chat is unavailable to this sender (spec §3.5). An
    /// anonymous chat path would let a child escape their guardian's ceiling
    /// simply by not signing in — the Grokster line this project holds.
    NoVerifiedKey,
    /// Over the per-minute rate limit, checked before any permission
    /// evaluation. `warn` mirrors `RateLimiter::should_warn` — true only the
    /// first time in a run of refusals, so the caller warns once, not on
    /// every dropped line.
    RateLimited { warn: bool },
    /// Failed the sanitiser (empty, too long, or a control character).
    Rejected(crate::comms::ChatReject),
    /// Delivered. `text` is the sanitised line; `recipients` are the OTHER
    /// player indices that pass the tier rule (never the sender — a speaker
    /// always hears their own line, which the caller echoes separately
    /// rather than running the rule against yourself).
    Delivered { text: String, recipients: Vec<usize> },
}

/// Run one chat line from `players[sender]` through the full world-chat
/// pipeline, in the order the spec requires (§7.3-7.6): verified-key gate,
/// rate limit, sanitiser, then the tier rule against every OTHER connected,
/// verified player. Mutates `players[sender].chat_rate`; otherwise pure —
/// no I/O, so it never touches a transport.
/// Decode a signed-in persona pubkey (64 lowercase hex) into the wire form.
///
/// Split out as a pure function so the parsing has tests — the caller reads it
/// from a file on disk, which does not.
///
/// Anything that is not exactly 32 bytes of hex yields `None`, and `None` means
/// "not signed in", which means no chat. Being strict here is the point: a
/// half-parsed identity would be worse than no identity.
pub fn local_identity_pubkey(hex_key: Option<&str>) -> Option<[u8; 32]> {
    let hex_key = hex_key?;
    let bytes = hex::decode(hex_key).ok()?;
    <[u8; 32]>::try_from(bytes.as_slice()).ok()
}

/// Native-only — see `ChatSayOutcome` above.
#[cfg(not(target_arch = "wasm32"))]
pub fn handle_chat_say(
    players: &mut [ServerPlayer],
    sender: usize,
    raw_text: &str,
    now_tick: u64,
) -> ChatSayOutcome {
    let Some(sp) = players.get(sender) else {
        return ChatSayOutcome::NoVerifiedKey;
    };
    if sp.verified_pubkey.is_none() {
        return ChatSayOutcome::NoVerifiedKey;
    }
    // Rate limit before anything else costs a permission evaluation.
    if !players[sender].chat_rate.allow(now_tick) {
        let warn = players[sender].chat_rate.should_warn();
        return ChatSayOutcome::RateLimited { warn };
    }
    let text = match crate::comms::sanitize_chat_text(raw_text) {
        Ok(t) => t,
        Err(reject) => return ChatSayOutcome::Rejected(reject),
    };

    let speaker_pubkey = players[sender]
        .verified_pubkey
        .expect("checked at function entry");
    let speaker_level = players[sender].comms;
    let mut recipients = Vec::new();
    for idx in 0..players.len() {
        if idx == sender || !players[idx].connected {
            continue;
        }
        // No verified key: can't be in anyone's address book and can't
        // participate — same gate as the sender side (spec §3.5).
        let Some(listener_pubkey) = players[idx].verified_pubkey else {
            continue;
        };
        let listener_level = players[idx].comms;
        let speaker_sees_listener = players[sender].tier_of(&listener_pubkey);
        let listener_sees_speaker = players[idx].tier_of(&speaker_pubkey);
        if crate::comms::delivers(
            speaker_level,
            speaker_sees_listener,
            listener_level,
            listener_sees_speaker,
        ) {
            recipients.push(idx);
        }
    }
    ChatSayOutcome::Delivered { text, recipients }
}

// `ClientInput` (an early per-tick input struct) was removed here — zero
// references anywhere. `ServerPlayer.pending_intent: Option<PlayerIntent>`
// (player_intent.rs) is the actual, live input mechanism; this predates it
// and was never wired in.

/// The authoritative game server.
pub struct GameServer {
    pub world_name: String,
    /// The world's real generation seed (threaded from `new`). Source of truth
    /// for the seed in `JoinAccept` (so joiners generate matching terrain) and
    /// in the save path — was previously hardcoded to 42 in both places.
    pub seed: u32,
    pub world: World,
    pub registry: BlockRegistry,
    pub biome_gen: BiomeGenerator,
    pub players: Vec<ServerPlayer>,
    pub ecs: hecs::World,
    pub water: WaterSystem,
    /// Campaign B — lava flow. Ticked alongside `water` in `tick()`; without it
    /// the authoritative server never simulated lava (world-gen pools + placed
    /// buckets sat static forever on hosted/dedicated worlds).
    pub lava: LavaSystem,
    /// Fire spread (2026-07-04) — authoritative burn/spread sim (fire.rs).
    pub fire: crate::fire::FireSystem,
    /// Cache of `WorldMeta.fire_spread_enabled` (loaded with the other meta).
    pub fire_spread_enabled: bool,
    /// Cache of `WorldMeta.explosives_enabled` (loaded with the other meta) —
    /// gates a server-side keg detonation exactly as the client's
    /// `GameState::explosives_enabled` gates its own (Spec 49).
    pub explosives_enabled: bool,
    /// T1-3 (2026-10-05) — does THIS server tick the block machines (hives,
    /// crops + saplings, dispensers, pistons, furnaces, composters, keg fuses,
    /// hoppers — see `block_machines.rs`)? `true` only when no local host
    /// client simulates them: set by `HostedServer::start_inner` iff it has 0
    /// local players (the dedicated server). A LAN / online host's client
    /// already ticks every machine and mirrors its block-entities in, so a
    /// second server-side tick would double every piston push and fight the
    /// mirror. Default `false` (also for `TestHost`, whose
    /// `tick_furnaces` / `tick_pistons` stand-ins would otherwise double up).
    pub simulates_block_machines: bool,
    pub leaf_decay: LeafDecaySystem,
    pub world_time: u32,
    pub loaded_columns: ahash::AHashSet<(i32, i32)>,
    pub falling_tick_counter: u32,
    /// Monotonically-incrementing tick counter used as the clock for
    /// systems that must measure absolute durations (e.g. Rubber tap
    /// cooldowns). `world_time` cycles at 24 000 so it can't be used
    /// directly for any subtract-based age check.
    pub tick_counter: u64,
    /// Server-authoritative weather window (P9 weather sync). Ephemeral —
    /// always starts `CLEAR` on construction, matching `weather.rs`'s
    /// "a world always loads clear" invariant; never persisted. Advanced
    /// every tick in `tick()` and put on the wire (as ticks-remaining, via
    /// `Weather::ticks_left`) on every `StateUpdatePacket` broadcast, so a
    /// hosted world's rain — and its fire-dousing — agree with what every
    /// client sees, instead of each side rolling its own private window.
    pub weather: crate::weather::Weather,
    /// Source of truth for how the player interacts with the world (Spec 05 §8).
    /// `is_creative` below is a single-writer cached projection of this — see
    /// `set_play_mode`. Never write `is_creative` directly; always go through
    /// `set_play_mode` so the cache can't drift.
    pub play_mode: crate::play_mode::PlayMode,
    /// Whether the current world runs in creative mode. Sourced from
    /// `WorldMeta.game_mode` on `initial_load`. Drives the flight gate in
    /// `tick_player_physics` — survival worlds ignore `toggle_flight` and
    /// force-clear any active flying state every tick.
    pub is_creative: bool,
    /// Block changes accumulated this tick from simulation (falling blocks,
    /// eventually player actions). Drained by the snapshot builder in
    /// hosted_server.rs into StateUpdatePacket.block_changes.
    pub pending_block_changes: Vec<crate::protocol::BlockChange>,
    /// Stacks granted by THIS tick's server-side item pickup (death-drops
    /// phase 2b), keyed by player index. Per-tick data: cleared at the top of
    /// each pickup pass, drained by `hosted_server.rs` right after
    /// `server.tick()` into per-connection `InventoryGrantPacket`s. Remote
    /// inventories are client-authoritative until the dual-sim rework, so the
    /// packet is what actually lands the stack in front of the player.
    pub pending_item_grants: Vec<(usize, crate::item::ItemStack)>,
    /// Server-side view radius in chunks (Spec 39 — was the `RENDER_DISTANCE`
    /// const). Defaults to the High preset; in hosted single-player the client
    /// keeps it in sync with the player's render-distance dial.
    pub render_distance: i32,
}

impl GameServer {
    pub fn new(num_players: usize, world_name: String, seed: u32) -> Self {
        let registry = BlockRegistry::new();
        let mut world = World::new();
        // Spec 27 Phase 5 — make engine-bundled curated plans
        // available to village procgen on the server side. Idempotent
        // + cheap (parses ~20 JSON blobs once).
        world.load_bundled_plans();
        // #8 — seed from the caller (the world's real seed), not a hardcoded 42.
        let biome_gen = BiomeGenerator::new(seed);

        let mut players = Vec::new();
        for i in 0..num_players {
            let offset = i as f32 * 2.0;
            players.push(ServerPlayer::new(Vec3::new(0.5 + offset, 80.0, 0.5)));
        }

        Self {
            world_name,
            seed,
            world,
            registry,
            biome_gen,
            players,
            ecs: hecs::World::new(),
            water: WaterSystem::new(),
            lava: LavaSystem::new(),
            fire: crate::fire::FireSystem::new(),
            fire_spread_enabled: true,
            explosives_enabled: true,
            simulates_block_machines: false,
            leaf_decay: LeafDecaySystem::new(),
            world_time: 6000,
            loaded_columns: ahash::AHashSet::new(),
            falling_tick_counter: 0,
            tick_counter: 0,
            weather: crate::weather::Weather::CLEAR,
            play_mode: crate::play_mode::PlayMode::Survival,
            is_creative: false,
            pending_block_changes: Vec::new(),
            pending_item_grants: Vec::new(),
            render_distance: crate::graphics_settings::DEFAULT_RENDER_DISTANCE,
        }
    }

    /// The ONLY sanctioned way to change the world's play mode. Keeps the
    /// `is_creative` projection in lock-step so the ~130 read sites stay valid.
    pub fn set_play_mode(&mut self, mode: crate::play_mode::PlayMode) {
        self.play_mode = mode;
        self.is_creative = mode.is_creative();
    }

    /// Get the number of connected players.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Run initial world load around the first player's position.
    pub fn initial_load(&mut self) {
        // Dedicated server runs with 0 local players — fall back to the world
        // spawn so initial_load doesn't index an empty player list (gap G4).
        let spawn = self
            .players
            .first()
            .map(|p| p.player.pos)
            .unwrap_or(Vec3::new(0.5, 80.0, 0.5));
        let cs = CHUNK_SIZE as i32;
        let pcx = (spawn.x.floor() as i32).div_euclid(cs);
        let pcz = (spawn.z.floor() as i32).div_euclid(cs);
        // Live server render distance (Spec 39 — was the `RENDER_DISTANCE` const).
        let rd = self.render_distance;

        // Pull mode + flat-world config from world meta so the hosted-server
        // sim matches the saved world: physics-flight gate (mode), terrain type
        // (world_type/ground/water_depth), day/night lock, and mob gating.
        // Falls back to defaults for a brand-new world with no meta yet.
        let meta = crate::save::load_world_meta(&self.world_name);
        self.set_play_mode(crate::play_mode::PlayMode::from_meta_str(&meta.game_mode));
        self.world.is_workshop = meta.is_workshop;
        self.world.world_type = meta.world_type;
        self.world.ground = meta.ground;
        self.world.water_depth = meta.water_depth;
        self.world.time_lock = meta.time_lock;
        self.world.mobs_enabled = meta.mobs_enabled;
        self.world.keep_inventory = meta.keep_inventory;
        self.fire_spread_enabled = meta.fire_spread_enabled;
        self.explosives_enabled = meta.explosives_enabled;

        // Check for saved world
        let wname = self.world_name.clone();
        if crate::save::world_exists(&wname) {
            log::info!("Loading saved world '{wname}'...");
            match crate::save::load_world(&wname, &mut self.world) {
                Ok((save_data, chunk_count)) => {
                    // Build per-player restore list: new saves have a `players` Vec;
                    // old saves have an empty Vec — fall back to legacy single-player fields.
                    let player_saves: Vec<crate::save::PlayerSaveData> = if save_data.players.is_empty() {
                        vec![crate::save::PlayerSaveData {
                            x: save_data.player_x,
                            y: save_data.player_y,
                            z: save_data.player_z,
                            yaw: 0.0,
                            pitch: 0.0,
                            health: save_data.player_health,
                            hotbar_slot: save_data.hotbar_slot,
                            inventory: save_data.inventory.clone(),
                            spawn_pos: None, // legacy single-player save
                            hunger: 20,
                            reputation: vec![],
                            tamed_pets: vec![],
                            armour_slots: [None, None, None, None],
                            kill_counter: vec![],
                            bounties_claimed: vec![],
                        }]
                    } else {
                        save_data.players.clone()
                    };

                    // Restore each player from save
                    for (i, p_save) in player_saves.iter().enumerate() {
                        if let Some(slot) = self.players.get_mut(i) {
                            slot.player.pos = Vec3::new(p_save.x, p_save.y, p_save.z);
                            slot.player.velocity = Vec3::ZERO;
                            // Note: ServerPlayer has no camera — yaw/pitch are only
                            // restored in PlayerSlot (game_loop.rs) for the local client.
                            slot.combat.health = p_save.health;
                            slot.combat.hunger = p_save.hunger;
                            slot.hotbar_slot = p_save.hotbar_slot;
                            crate::save::restore_inventory(&mut slot.inventory, &p_save.inventory);
                        }
                    }

                    // Position any extra players (beyond what's in the save) near player 0
                    let p0_pos = self
                        .players
                        .first()
                        .map(|p| p.player.pos)
                        .unwrap_or(Vec3::new(0.5, 80.0, 0.5));
                    for i in player_saves.len()..self.players.len() {
                        self.players[i].player.pos = p0_pos + Vec3::new(i as f32 * 2.0, 0.0, 0.0);
                        self.players[i].player.velocity = Vec3::ZERO;
                    }

                    // Mark loaded columns
                    for (cx, _cy, cz) in self.world.chunk_positions() {
                        self.loaded_columns.insert((cx, cz));
                    }

                    // Rail freight Phase 1 (Task 1.6) — re-spawn persisted carts
                    // into THIS server's ECS (the one `cart::tick_carts` advances
                    // each tick) so a saved in-flight cart resumes rolling on
                    // reload. Carts are ECS entities, not block-entities, so they
                    // bypass `apply_world_save_state`; the ECS handle is in scope
                    // here. Old saves have an empty `carts` Vec via
                    // `#[serde(default)]` → a no-op for pre-rail worlds.
                    for s in &save_data.carts {
                        crate::cart::spawn_cart_from(&mut self.ecs, s.data.clone());
                    }

                    // Spec 30 — light data isn't persisted; recompute
                    // from block state for every loaded column.
                    for &(cx, cz) in &self.loaded_columns.iter().copied().collect::<Vec<_>>() {
                        crate::lighting::run_initial_pass_for_column(
                            &mut self.world, cx, cz, &self.registry,
                        );
                    }

                    // Register water + lava sources
                    for &(cx, cz) in &self.loaded_columns.iter().copied().collect::<Vec<_>>() {
                        self.water.register_column_sources(cx, cz, &self.world);
                        self.lava.register_column_sources(cx, cz, &self.world);
                        self.fire.register_column_fires(cx, cz, &self.world, self.tick_counter);
                    }

                    // Generate missing columns (around p0_pos, computed above —
                    // the world spawn when there are no local players).
                    let pcx = (p0_pos.x.floor() as i32).div_euclid(cs);
                    let pcz = (p0_pos.z.floor() as i32).div_euclid(cs);
                    for dx in -rd..=rd {
                        for dz in -rd..=rd {
                            let cx = pcx + dx;
                            let cz = pcz + dz;
                            if !self.loaded_columns.contains(&(cx, cz)) {
                                self.world.generate_column(cx, cz, &self.biome_gen);
                                crate::lighting::run_initial_pass_for_column(&mut self.world, cx, cz, &self.registry);
                                self.loaded_columns.insert((cx, cz));
                                self.water.register_column_sources(cx, cz, &self.world);
                                self.lava.register_column_sources(cx, cz, &self.world);
                                self.fire.register_column_fires(cx, cz, &self.world, self.tick_counter);
                            }
                        }
                    }

                    // Scatter mobs
                    for &(cx, cz) in &self.loaded_columns.iter().copied().collect::<Vec<_>>() {
                        entity::scatter_mobs_in_column(&mut self.ecs, cx, cz, &self.world, &self.biome_gen);
                    }

                    log::info!("Loaded {chunk_count} saved chunks. {} entities.", self.ecs.len());
                    return;
                }
                Err(e) => log::warn!("Failed to load world: {e}. Generating new."),
            }
        }

        // Fresh world generation
        log::info!("Generating new world (render distance = {rd})...");
        for dx in -rd..=rd {
            for dz in -rd..=rd {
                let cx = pcx + dx;
                let cz = pcz + dz;
                self.world.generate_column(cx, cz, &self.biome_gen);
                                crate::lighting::run_initial_pass_for_column(&mut self.world, cx, cz, &self.registry);
                self.loaded_columns.insert((cx, cz));
            }
        }

        for dx in -rd..=rd {
            for dz in -rd..=rd {
                self.water.register_column_sources(pcx + dx, pcz + dz, &self.world);
                self.lava.register_column_sources(pcx + dx, pcz + dz, &self.world);
                self.fire.register_column_fires(pcx + dx, pcz + dz, &self.world, self.tick_counter);
            }
        }

        for dx in -rd..=rd {
            for dz in -rd..=rd {
                entity::scatter_mobs_in_column(&mut self.ecs, pcx + dx, pcz + dz, &self.world, &self.biome_gen);
            }
        }
        log::info!("Spawned {} entities.", self.ecs.len());

        // Place players on terrain
        for p in &mut self.players {
            let px = p.player.pos.x.floor() as i32;
            let pz = p.player.pos.z.floor() as i32;
            let mut sy = 80;
            while sy > 0 && self.world.get_block(px, sy, pz) == block::AIR {
                sy -= 1;
            }
            p.player.pos.y = sy as f32 + 1.0;
            p.player.velocity = Vec3::ZERO;
        }
    }

    /// Run one server simulation tick (20 TPS).
    ///
    /// Covers: world time, mob spawn/sun-burn (400-tick cycle), falling
    /// blocks, water/lava/fire, leaf decay (applied + broadcast), mob AI,
    /// entity physics, power, carts, combat timers, entity health, dead entity
    /// cleanup — and, when [`Self::simulates_block_machines`] is set (dedicated
    /// server only), the block machines in `block_machines.rs`.
    ///
    /// BRIDGE: HostedServer input still trusts the client's position
    /// (hosted_server.rs:345). Single-player bypasses GameServer entirely;
    /// mob spawning + falling blocks + player sim all run a parallel copy on
    /// the client side in that case (see game_loop.rs). Task 1d routes
    /// single-player through HostedServer so GameServer is the single
    /// authority; then the dual-sim code paths delete.
    pub fn tick(&mut self) {
        // Per-world active-tick clock (Goal 1) — mirrors GameState::tick so
        // hosted / headless (TestHost) worlds accrue the same world-clock stat
        // (total_ticks). Source of truth on disk is WorldMeta.
        self.world.tick_world_clock();
        // Spec 29 — drain legacy meat ejected from v1 furnaces during
        // load (mirror of GameState::tick). Empty after the first
        // post-load tick.
        if !self.world.pending_legacy_meat_drops.is_empty() {
            let drops = std::mem::take(&mut self.world.pending_legacy_meat_drops);
            for (pos, stack) in drops {
                let drop_pos = Vec3::new(
                    pos.0 as f32 + 0.5,
                    pos.1 as f32 + 1.0,
                    pos.2 as f32 + 0.5,
                );
                let seed = pos.0.unsigned_abs() ^ pos.1.unsigned_abs() ^ pos.2.unsigned_abs();
                crate::entity::spawn_item(&mut self.ecs, drop_pos, stack, seed);
            }
        }
        // Advance world time (24000 ticks per day cycle = 20 min @ 20 TPS).
        // Matches GameState::world_time_step's default of 1 (post-playtest
        // rollback from the alpha-fast 4× BRIDGE).
        self.world_time = (self.world_time + 1) % 24000;
        // Monotonic counter — used by anything that needs an absolute
        // age check (Rubber tap cooldown, future replenishers).
        self.tick_counter = self.tick_counter.wrapping_add(1);

        // P9 weather sync — advance the SAME formula the client uses
        // (`weather::advance`), every tick, so a dedicated/hosted server's
        // weather is the single source of truth every client's rain window
        // is derived from (see `weather` field doc + `hosted_server.rs`'s
        // `broadcast_state`, which puts `self.weather.ticks_left(tick)` on
        // every StateUpdate).
        self.weather = crate::weather::advance(
            self.weather,
            self.tick_counter,
            crate::weather::world_has_weather(self.world.is_workshop),
        );

        // Mob spawn + sun-burn cycle (every 400 ticks = 20 s). Shared free
        // function — the single-player client path calls the same function on
        // its own ECS until the dual-ECS state is unified (Task 1d).
        let player_positions: Vec<Vec3> = self.players.iter().map(|sp| sp.player.pos).collect();
        // Blank-canvas time-lock: use the effective time so locked-day worlds
        // never trigger night-mob spawns and locked-night worlds always do.
        let eff_world_time = self.world.effective_world_time(self.world_time);
        if self.world_time.is_multiple_of(400) {
            crate::spawning::tick_mob_spawning(
                &mut self.ecs,
                &self.world,
                eff_world_time,
                &player_positions,
            );
        }

        // Falling blocks + water/leaf decay (every 4th tick = 5 Hz).
        self.falling_tick_counter = self.falling_tick_counter.wrapping_add(1);
        if self.falling_tick_counter.is_multiple_of(4) {
            // Falling blocks — shared pure function, same logic as client path.
            // Returned BlockChange records go into pending_block_changes so
            // hosted_server.rs's snapshot builder drains them to clients.
            let falling_changes = crate::falling_blocks::tick_falling_blocks(
                &mut self.world,
                &self.registry,
                &mut self.water,
                &player_positions,
                MAX_CHUNK_Y,
            );
            self.pending_block_changes.extend(falling_changes);

            let water_dirty = self.water.tick_spread(&mut self.world);
            // A water spread that froze a lava source to obsidian must drop that
            // source from the lava system, or a re-placed lava cell there is
            // silently inert (phantom source). Mirror of the client tick.
            crate::fluids::reconcile_frozen_lava_sources(
                &mut self.lava,
                &self.world,
                &water_dirty,
            );
            let water_retract = self.water.tick_retract(&mut self.world);
            // Campaign B — lava flow (slower + shorter range than water; freezes
            // to obsidian on water contact). Runs on the same 4-tick cadence as
            // water; its own SLOW_FACTOR gate crawls it slower still.
            let lava_spread = self.lava.tick_spread(&mut self.world);
            let lava_retract = self.lava.tick_retract(&mut self.world);

            // Fire (2026-07-04) — server-authoritative burn, gated on the
            // server's OWN weather window (`self.weather`, advanced once per
            // tick above) — not a private re-roll, so a hosted world's
            // fire-dousing rain agrees with the rain every client is shown.
            let raining_now = self.tick_counter < self.weather.rain_until;
            let fire_lit =
                self.fire
                    .ignite_from_lava(&mut self.world, &lava_spread, self.tick_counter);
            let fire_dirty = self.fire.tick(
                &mut self.world,
                self.tick_counter,
                raining_now,
                self.fire_spread_enabled,
            );

            // Broadcast the fluid deltas. Each system reports the cells it
            // touched; read the settled block there (WATER/LAVA/OBSIDIAN/AIR)
            // and queue a BlockChange so remote clients SEE the fluid move —
            // same pattern as falling blocks + power above. Without this the
            // server advanced fluids invisibly until a full chunk resync. The
            // per-tick budgets in water/lava cap this at a few hundred cells.
            for &(x, y, z) in water_dirty
                .iter()
                .chain(water_retract.iter())
                .chain(lava_spread.iter())
                .chain(lava_retract.iter())
                .chain(fire_lit.iter())
                .chain(fire_dirty.iter())
            {
                let nb = self.world.get_block(x, y, z);
                // Carry the meta byte: water depth levels (AUX field) ride the
                // same delta so remote clients render the sloped surface and
                // compute the same flow vector as the host.
                self.pending_block_changes.push(crate::protocol::BlockChange::with_meta(
                    x,
                    y,
                    z,
                    nb,
                    self.world.meta_at(x, y, z),
                ));
            }

            // Leaf decay — APPLY the result (T1-3): it used to be computed and
            // discarded, so a server-side decay cleared the leaf in the
            // server's world while every joiner kept a floating one until a
            // full chunk resync, and its sapling drops vanished. The queue is
            // fed by `on_log_broken` from the block-edit apply in
            // `hosted_server.rs` for REMOTE players' log breaks only, on every
            // host kind (a joiner's client runs no decay; a LAN host's client
            // keeps owning decay of its own breaks). Not flag-gated.
            let leaf_dirty = self.leaf_decay.tick(&mut self.world);
            for &(x, y, z) in &leaf_dirty {
                let nb = self.world.get_block(x, y, z);
                self.pending_block_changes
                    .push(crate::game_loop::broadcast_change(&self.world, x, y, z, nb));
            }
            crate::leaf_decay::spawn_sapling_drops(
                &mut self.ecs,
                self.leaf_decay.take_sapling_drops(),
            );

            // Block machines (T1-3) — only where no host client ticks them.
            if self.simulates_block_machines {
                self.tick_block_machines();
            }
        }

        // Hoppers — own 8-tick cadence on the monotonic counter, outside the
        // 4-tick block, mirroring the client loop. Same flag rule.
        if self.simulates_block_machines {
            self.tick_hoppers();
        }

        // Server-driven player physics for remote (server_simulated) players
        // — Task 1d. Local players skip this branch and continue to have
        // their position set directly from InputPacket by hosted_server.rs.
        self.tick_player_physics();

        // Mob AI — targets nearest player. Rebuild positions — player_physics
        // may have moved server_simulated players.
        let player_positions: Vec<Vec3> = self.players.iter().map(|sp| sp.player.pos).collect();
        // HP-3 — brigand AI pre-pass runs BEFORE the main dispatcher so
        // tier-specific detect-range + flee-gate + day/night chase-gate
        // overrides land first.
        crate::brigand::tick_brigand_overrides(
            &mut self.ecs,
            &player_positions,
            eff_world_time,
        );
        crate::mob_ai::tick_mob_ai(
            &mut self.ecs,
            &self.world,
            &self.registry,
            &player_positions,
        );
        // HP-3 — Brigand Hideout replenisher. Throttled internally on
        // the per-hideout 24 000-tick cooldown; safe to call every tick
        // (the heavy lift only runs when a hideout needs topping up).
        // MUST use `tick_counter` (monotonic), not `world_time` (cyclic
        // 0-23999): `REPLENISH_COOLDOWN_TICKS = 24 000` = day length,
        // so the subtract math wraps wrong every day and the cooldown
        // resets silently.
        crate::brigand_hideout_gen::tick_hideout_spawning(
            &mut self.world,
            &mut self.ecs,
            self.tick_counter,
        );
        // Salt feature — snowfall painter. Internally throttled on
        // SNOWFALL_PERIOD_TICKS (6 000 ticks); cheap to call every
        // tick because the body is a no-op on intermediate ticks.
        // MUST use `tick_counter` — pass_id = tick / 6 000 collapses
        // to {0,1,2,3} with a cyclic 24 000 clock, so the same four
        // cells get snow every day instead of drifting.
        crate::snowfall::tick_snowfall(
            &mut self.world,
            &self.biome_gen,
            self.biome_gen.seed,
            self.tick_counter,
        );
        // Rubber feature — restore tapped rubber logs whose cooldown
        // expired. O(N) scan of the tapped-logs index every tick; index
        // is bounded by player-tapped trees so stays tiny. MUST use
        // `tick_counter` (monotonic), not `world_time` (cyclic 0-23999):
        // a subtract-based age check on a 24 000-tick cooldown never
        // fires with a clock that wraps at 24 000.
        crate::rubber::tick_rubber_cooldowns(&mut self.world, self.tick_counter);
        // Salt feature — Salt Lick aura HP regen for livestock. Self-
        // throttled internally on SALT_LICK_REGEN_INTERVAL_TICKS; safe
        // to call every tick.
        crate::salt_lick::tick_salt_lick_regen(&mut self.ecs, &self.world, self.tick_counter);
        // Spec 33 Mob Bounty Board — daily rotation refresh. Self-
        // throttled internally on BOUNTY_REFRESH_TICKS.
        let _ = crate::bounty::tick_bounty_refresh(
            &mut self.world, self.tick_counter, self.biome_gen.seed,
        );

        // Entity physics
        crate::entity::tick_entities(&mut self.ecs, &self.world, &self.registry);

        // Item-entity housekeeping (death-drops phase 2b) — mirror of the
        // client-sim pass in game_loop.rs: lifetime decay (5-min despawn +
        // pickup_delay countdown; without it server drops are permanently
        // unpickable and accumulate forever), then magnet/pickup for
        // server-simulated players. Local (position-trusted) players are
        // skipped — they pick up from their own client sim, and granting here
        // too would double-count under the single-player parity flag. Only
        // stacks that survive the `ItemRef` wire encoding are granted (a tool
        // collapses to a bare tier on the wire — fabricating a wrong-kind,
        // full-durability tool client-side is worse than leaving the drop).
        // Grants are per-tick data, drained by hosted_server.rs into
        // per-connection InventoryGrantPacket sends right after this tick.
        crate::entity::tick_item_lifetimes(&mut self.ecs);
        self.pending_item_grants.clear();
        {
            let mut eligible_players: Vec<(usize, Vec3, &mut Inventory)> = Vec::new();
            for (idx, sp) in self.players.iter_mut().enumerate() {
                if !sp.server_simulated || !sp.connected || sp.combat.dead {
                    continue;
                }
                eligible_players.push((idx, sp.player.pos, &mut sp.inventory));
            }
            // Death-drops phase 3 (v61) — everything the wire can express is
            // eligible. Blocks/materials ride the lossless `(kind, id)` pair;
            // tools and armour ride `WireItem` on the grant packet. Only
            // `Item::Plan` stays floor-bound: `plan::PlanData` has no wire
            // form, so granting one would mint an empty plan client-side.
            let grants = crate::entity::tick_item_pickups(
                &mut self.ecs,
                &mut eligible_players,
                |item| !matches!(item, crate::item::Item::Plan(_)),
            );
            self.pending_item_grants.extend(grants);
        }

        // Spec 48 (Electricity) — power & logic sim. Runs AFTER entity physics
        // (so pressure plates see settled positions) and BEFORE carts (so a
        // powered rail's speed-up is already computed when the cart steps).
        // Server-authoritative; visual flips ride pending_block_changes.
        {
            let mut power_positions: Vec<(f32, f32, f32)> = self
                .players
                .iter()
                .map(|p| (p.player.pos.x, p.player.pos.y, p.player.pos.z))
                .collect();
            for (_id, pos) in self.ecs.query::<&crate::entity::Position>().iter() {
                power_positions.push((pos.0.x, pos.0.y, pos.0.z));
            }
            // Wind, Copper & Electricity wave §2.3 — ONE sea-level wind sample
            // per tick serves every Windmill in the world; each mill lifts it
            // to its own height with `wind::with_altitude`. Derived from the
            // server's authoritative weather window + the world seed, so a
            // joined client's mills read exactly as the host simulates them
            // without a byte of protocol.
            let wind = crate::wind::sample(
                self.tick_counter,
                self.weather,
                self.seed,
                crate::biome::SEA_LEVEL,
            );
            let power_changes = crate::power::power_tick(
                &mut self.world,
                self.tick_counter,
                &power_positions,
                wind,
                &self.registry,
            );
            // Spec 48 — drive the lighting BFS for any lamp/generator that
            // flipped lit↔unlit (power_tick uses bare set_block, which doesn't
            // relight). Keeps streamed chunk light + the host's own render correct.
            for bc in &power_changes {
                crate::power::relight_after_power_change(&mut self.world, bc, &self.registry);
            }
            self.pending_block_changes.extend(power_changes);
        }

        // Rail freight (Phase 1) — roll carts along the track + unload cargo
        // into the destination depot chest on arrival. Server-authoritative
        // (mirrors the GameState::tick call so hosted / headless worlds advance
        // carts identically). Carts omit OnGround so the physics tick above
        // skips them; this drives their timed cell-to-cell travel instead.
        crate::cart::tick_carts(&mut self.ecs, &mut self.world);

        // Per-player entity collision + combat timers
        for i in 0..self.players.len() {
            let player = &mut self.players[i].player;
            crate::entity::push_player_from_entities(
                &self.ecs,
                &mut player.pos,
                &mut player.velocity,
            );
            self.players[i].combat.tick();
        }

        // Entity health timers
        for (_id, health) in self.ecs.query_mut::<&mut crate::combat::Health>() {
            health.tick();
        }

        // HP-3 — snapshot HomeHideout anchors on dying brigands so the
        // post-despawn population decrement runs against the right
        // hideout.
        let mut dying_brigand_homes: Vec<[i32; 3]> = Vec::new();
        for (_e, (health, kind, home)) in self.ecs.query::<(
            &crate::combat::Health,
            &crate::entity::MobKind,
            &crate::brigand::HomeHideout,
        )>().iter() {
            if health.is_dead() && crate::brigand::Tier::from_mob_type(kind.0).is_some() {
                dying_brigand_homes.push(home.anchor);
            }
        }

        // Wave-hardening backlog (2026-07-11) — this used to DISCARD
        // `despawn_dead`'s return, so a mob killed in the server sim dropped
        // NOTHING (no loot table, no wolf routing, no cargo-pack spill).
        // Snapshot ownership/cargo state first, then run the same
        // `death_drops` routing the client-side sweep uses. (Phase 2 closed
        // the wire: `diff_entities` broadcasts these drops to remote clients,
        // and the pickup pass above grants them via InventoryGrant. Kill
        // attribution / bounty / Vow stay client-side pending the dual-sim
        // rework.)
        let drop_snaps = crate::death_drops::snapshot_before_despawn(&self.ecs);
        let deaths = crate::combat::despawn_dead(&mut self.ecs);
        for (kind, pos, _attacker) in deaths {
            crate::death_drops::spawn_drops_for_death(
                &mut self.ecs,
                &self.world,
                &drop_snaps,
                kind,
                pos,
                self.world_time,
            );
        }

        // HP-3 — apply the population decrements after despawn.
        for home in &dying_brigand_homes {
            crate::brigand_hideout_gen::on_brigand_killed(&mut self.world, *home);
        }
    }

    /// Run `Player::tick` for every player flagged `server_simulated` using
    /// their most-recent `pending_intent` — Task 1d.
    ///
    /// Local (position-trusted) players are skipped: their position is
    /// authoritatively set from each `InputPacket` by `hosted_server.rs`.
    ///
    /// Anti-cheat seed: speed cap. If the intent-driven tick produces a
    /// horizontal displacement that exceeds `MAX_HORIZONTAL_PER_TICK`, the
    /// move is clamped to the cap and logged. Full spec 08 validation is a
    /// later task.
    fn tick_player_physics(&mut self) {
        // 21.78 b/s → 1.089 b/tick at 20 TPS, with 1.5× headroom. NOTE
        // (Task 15 audit, 2026-07-07): 21.78 is FLY_SPRINT_SPEED, not the
        // grounded SPRINT_SPEED (5.612 b/s → 0.2806 b/tick) this cap was
        // originally described as — so the gate is ~3.9× looser than a
        // grounded-sprint cap would be. Deliberately left as-is: tightening
        // it to SPRINT_SPEED-derived (≈0.42 b/tick) is a future
        // anti-cheat-tuning decision, not a typo fix — a corrected value
        // would need to account for the additive sprint-jump boost (bare
        // sprint-jump peaks ≈0.484 b/tick, Rubber-booted ≈0.597 — both over
        // 0.42) and for the Rubber Boots multiplier
        // (`armour::sprint_multiplier`, capped at 1.4 partly for this
        // reason). Creative flight legitimately reaches 1.089 b/tick, which
        // is presumably why the fly-sprint figure was used.
        const MAX_HORIZONTAL_PER_TICK: f32 = 1.089 * 1.5;

        for sp in &mut self.players {
            if !sp.server_simulated {
                continue;
            }
            let Some(intent) = sp.pending_intent.take().or_else(|| sp.intent_queue.pop_front())
            else {
                continue;
            };
            let pre = sp.player.pos;
            // Server has no GPU camera; construct a throwaway one from yaw/pitch.
            // Aspect/FOV only matter for matrices we don't build.
            let mut cam = crate::camera::Camera::new(sp.player.pos, 1.0);
            cam.yaw = sp.yaw;
            cam.pitch = sp.pitch;

            // Task 15 — `sp.player.sprint_boots_mult` stays at its default
            // (1.0, no bonus) here: `ServerPlayer` doesn't track armour at
            // all (same pre-existing BRIDGE as reputation/pets/kill_counter
            // above — armour lives on the client's `PlayerSlot`). A remote
            // player's Rubber Boots bonus isn't applied server-side until
            // that duplication collapses.
            sp.player.tick(&intent, &cam, &self.world, &self.registry, self.play_mode);

            // Speed cap (horizontal only — y is gravity/jump and legitimately
            // exceeds this during a jump).
            let dx = sp.player.pos.x - pre.x;
            let dz = sp.player.pos.z - pre.z;
            let horizontal = (dx * dx + dz * dz).sqrt();
            if horizontal > MAX_HORIZONTAL_PER_TICK {
                let scale = MAX_HORIZONTAL_PER_TICK / horizontal;
                sp.player.pos.x = pre.x + dx * scale;
                sp.player.pos.z = pre.z + dz * scale;
                log::warn!(
                    "speed cap: clamped {horizontal:.3}→{MAX_HORIZONTAL_PER_TICK:.3} b/tick"
                );
            }
        }
    }

    /// Save the world (all players' state).
    /// BRIDGE: ServerPlayer doesn't match PlayerSlot — save uses raw fields.
    /// When GameServer becomes truly authoritative, unify the save path.
    /// Native-only for now; the WASM single-player path still autosaves
    /// through the client's `save::save_world_wasm` (IndexedDB upload). Once
    /// single-player routes through HostedServer on WASM we'll add a WASM
    /// branch here.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self) {
        match self.try_save() {
            Ok(()) => log::info!("Server saved world '{}'", self.world_name),
            Err(e) => log::error!("Server save of world '{}' FAILED: {e}", self.world_name),
        }
    }

    /// [`save`](Self::save), reporting failure instead of only logging it.
    ///
    /// Same write discipline as the client save (`save::write_world_folder`):
    /// every file goes through tmp + rename, so a crash or full disk never
    /// leaves a torn file. Chunks are written FIRST (no per-file fsync, one
    /// directory fsync at the end), then `world.dat`. Every chunk is attempted
    /// and each failure is logged with its path; any failure makes the save
    /// return `Err`. `world.dat` is still written when SOME chunks failed: the
    /// good chunks have already been renamed in, so skipping it would leave
    /// players, inventories and block entities from the last save beside newer
    /// chunks — and a chunk path that fails every time would freeze them for
    /// good (item loss / duplication on restart). Only when EVERY chunk write
    /// failed (likely a dead disk) is `world.dat` skipped, leaving the last
    /// consistent pair untouched. `exhibits.json` is a sidecar for the
    /// Operator Console, written after `world.dat`; its failure is logged but
    /// doesn't fail the save.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn try_save(&self) -> Result<(), String> {
        let wname = self.world_name.clone();
        let dir = crate::save::world_dir(&wname);
        let chunks_dir = dir.join("chunks");
        std::fs::create_dir_all(&chunks_dir)
            .map_err(|e| format!("mkdir {}: {e}", chunks_dir.display()))?;

        let player_saves: Vec<crate::save::PlayerSaveData> = self.players.iter().map(|sp| {
            crate::save::PlayerSaveData {
                x: sp.player.pos.x,
                y: sp.player.pos.y,
                z: sp.player.pos.z,
                yaw: 0.0,
                pitch: 0.0,
                health: sp.combat.health,
                hotbar_slot: sp.hotbar_slot,
                inventory: crate::save::serialize_inventory_raw(&sp.inventory),
                // Server-side ServerPlayer doesn't track bed-spawn yet —
                // BRIDGE in CLAUDE.md (ServerPlayer / PlayerSlot duplication).
                // Defer until that's collapsed; for now, no spawn_pos saved.
                spawn_pos: None,
                hunger: sp.combat.hunger,
                // Server-side ServerPlayer doesn't carry reputation either
                // (it's a PlayerSlot field on the client). Same BRIDGE; the
                // server-authoritative path will own this when the duplication
                // collapses.
                reputation: vec![],
                // Server-side ServerPlayer doesn't track pets either (same
                // BRIDGE — mob save isn't wired); persist empty for now.
                tamed_pets: vec![],
                // Server-side ServerPlayer doesn't track armour either
                // (same BRIDGE — armour lives on PlayerSlot on the
                // client). Persist empty until the duplication is collapsed.
                armour_slots: [None, None, None, None],
                // Server-side ServerPlayer also doesn't track kill_counter
                // or bounties_claimed — same BRIDGE. Persist empty.
                kill_counter: vec![],
                bounties_claimed: vec![],
            }
        }).collect();

        // Legacy single-player fields (`player_x/y/z`, health, hotbar, inventory)
        // default when there are no local players — a dedicated server has none,
        // and the `players` Vec is the real source of truth on load. Without this
        // the 0-player save path panics indexing `player_saves[0]` (gap G4).
        let p0_fallback = crate::save::PlayerSaveData {
            x: 0.5,
            y: 80.0,
            z: 0.5,
            yaw: 0.0,
            pitch: 0.0,
            health: 20.0,
            hotbar_slot: 0,
            inventory: Vec::new(),
            spawn_pos: None,
            hunger: 20,
            reputation: vec![],
            tamed_pets: vec![],
            armour_slots: [None, None, None, None],
            kill_counter: vec![],
            bounties_claimed: vec![],
        };
        let p0 = player_saves.first().unwrap_or(&p0_fallback);
        let campfires: Vec<crate::save::SavedCampfire> = self
            .world
            .iter_campfires()
            .map(|((x, y, z), data)| crate::save::SavedCampfire {
                x, y, z, data: data.clone(),
            })
            .collect();
        let furnaces: Vec<crate::save::SavedFurnace> = self
            .world
            .iter_furnaces()
            .map(|((x, y, z), data)| crate::save::SavedFurnace {
                x, y, z, data: data.clone(),
            })
            .collect();
        let vendors: Vec<crate::save::SavedVendor> = self
            .world
            .iter_vendors()
            .map(|((x, y, z), data)| crate::save::SavedVendor {
                x, y, z, data: data.clone(),
            })
            .collect();
        let drying_racks: Vec<crate::save::SavedDryingRack> = self
            .world
            .drying_racks
            .iter()
            .map(|(&(x, y, z), data)| crate::save::SavedDryingRack {
                x, y, z, data: data.clone(),
            })
            .collect();
        let hives: Vec<crate::save::SavedHive> = self
            .world
            .iter_hives()
            .map(|((x, y, z), data)| crate::save::SavedHive {
                x, y, z, data: *data,
            })
            .collect();
        let dispensers: Vec<crate::save::SavedDispenser> = self
            .world
            .iter_dispensers()
            .map(|((x, y, z), d)| crate::save::SavedDispenser { x, y, z, data: d.clone() })
            .collect();
        let chests: Vec<crate::save::SavedChest> = self
            .world
            .iter_chests()
            .map(|((x, y, z), data)| crate::save::SavedChest {
                x, y, z, data: data.clone(),
            })
            .collect();
        let tip_jars: Vec<crate::save::SavedTipJar> = self
            .world
            .iter_tip_jars()
            .map(|((x, y, z), data)| crate::save::SavedTipJar {
                x, y, z, data: data.clone(),
            })
            .collect();
        let auctions: Vec<crate::save::SavedAuction> = self
            .world
            .iter_auctions()
            .map(|((x, y, z), data)| crate::save::SavedAuction {
                x, y, z, data: data.clone(),
            })
            .collect();
        // Spec 38 (Blueprint / Cyanotype) — Latent Print block-entities,
        // mirroring the auctions pattern.
        let latent_prints: Vec<crate::save::SavedLatentPrint> = self
            .world
            .iter_latent_prints()
            .map(|((x, y, z), data)| crate::save::SavedLatentPrint {
                x, y, z, data: data.clone(),
            })
            .collect();
        let construction_anchors: Vec<crate::save::SavedConstructionAnchor> = self
            .world
            .construction_anchors
            .iter()
            .map(|(&(x, y, z), data)| crate::save::SavedConstructionAnchor {
                x, y, z, data: data.clone(),
            })
            .collect();
        let architect_plaques: Vec<crate::save::SavedArchitectPlaque> = self
            .world
            .architect_plaques
            .iter()
            .map(|(&(x, y, z), chain)| crate::save::SavedArchitectPlaque {
                x, y, z, chain: chain.clone(),
            })
            .collect();
        let save = crate::save::WorldSave {
            seed: self.seed,
            player_x: p0.x,
            player_y: p0.y,
            player_z: p0.z,
            player_health: p0.health,
            hotbar_slot: p0.hotbar_slot,
            inventory: p0.inventory.clone(),
            players: player_saves,
            campfires,
            furnaces,
            vendors,
            drying_racks,
            hives,
            chests,
            tip_jars,
            auctions,
            latent_prints,
            plots: self.world.plots.clone(),
            market_hubs: self.world.market_hubs.clone(),
            construction_anchors,
            architect_plaques,
            village_anchors: self
                .world
                .village_anchors
                .iter()
                .map(|(&(gx, gz), &anchor)| crate::save::SavedVillageAnchor {
                    grid_x: gx, grid_z: gz, anchor,
                })
                .collect(),
            populated_villages: self.world.populated_villages.iter().copied().collect(),
            village_bells: self.world.village_bells.clone(),
            village_treasuries: self.world.village_treasuries.iter().map(|(&k, &v)| (k, v)).collect(),
            active_raids: self.world.active_raids.clone(),
            raid_scheduler: self.world.raid_scheduler.clone(),
            raid_kills: self
                .world
                .raid_kills
                .iter()
                .map(|(&(vid, pk), &c)| (vid, pk, c))
                .collect(),
            brigand_hideouts: self
                .world
                .brigand_hideouts
                .iter()
                .map(|(&(gx, gz), data)| crate::save::SavedHideout {
                    grid_x: gx, grid_z: gz, data: data.clone(),
                })
                .collect(),
            bounties: self.world.bounties.iter().map(|b| crate::save::SavedBounty {
                id: b.id, template_idx: b.template_idx, issued_tick: b.issued_tick,
            }).collect(),
            bounty_next_id: self.world.bounty_next_id,
            bounty_last_refresh_tick: self.world.bounty_last_refresh_tick,
            face_overlays: crate::save::face_overlays_to_saved(&self.world),
            face_blueprints: crate::save::face_blueprints_to_saved(&self.world),
            face_blueprint_blanks: crate::save::face_blueprint_blanks_to_saved(&self.world),
            workshop: self.world.workshop.clone(),
            // Rail freight Phase 1 — snapshot carts from THIS server's ECS (the
            // same one `cart::tick_carts` advances) so a saved cart resumes its
            // exact in-flight state on reload.
            carts: crate::save::carts_to_saved(&self.ecs),
            // Animals Wave 2 — persist tamed pets from this server's ECS.
            saved_mobs: crate::save::tamed_mobs_to_saved(&self.ecs),
            // Satoshi onboarding — per-world guide state.
            satoshi: self.world.satoshi.clone(),
            dispensers,
            graves: self
                .world
                .iter_graves()
                .map(|((x, y, z), data)| crate::save::SavedGrave { x, y, z, data: data.clone() })
                .collect(),
            waypoints: self.world.waypoints.clone(),
            // Spec 48 (Electricity) — persist per-block meta + power-device state
            // from THIS server's world so circuits survive a server save/reload.
            block_meta: self
                .world
                .block_meta
                .iter()
                .map(|(&(x, y, z), &m)| (x, y, z, m))
                .collect(),
            power_devices: self
                .world
                .iter_power_devices()
                .map(|((x, y, z), data)| crate::save::SavedPowerDevice {
                    x,
                    y,
                    z,
                    data: data.clone(),
                })
                .collect(),
            signs: self
                .world
                .iter_signs()
                .map(|((x, y, z), data)| crate::save::SavedSign {
                    x,
                    y,
                    z,
                    data: data.clone(),
                })
                .collect(),
            item_frames: self
                .world
                .iter_item_frames()
                .map(|((x, y, z), data)| crate::save::SavedItemFrame {
                    x,
                    y,
                    z,
                    data: data.clone(),
                })
                .collect(),
            // Locked inventory slots are a client-side UI nicety; the dedicated
            // server has no single "player 0" UI state to persist.
            locked_slots: Vec::new(),
            hostile_acts: self.world.hostile_acts.acts().to_vec(),
            rigs: self.world.rigs.clone(),
            // #19 — index-aligned clip side table (see `WorldSave.rig_clips`).
            rig_clips: self.world.rigs.iter().map(|r| r.clip).collect(),
            exhibits: self.world.exhibits.clone(),
            composters: self
                .world
                .iter_composters()
                .map(|((x, y, z), data)| crate::save::SavedComposter { x, y, z, data: data.clone() })
                .collect(),
        };

        let encoded = bincode::serialize(&save).map_err(|e| format!("serialize world.dat: {e}"))?;

        // Spec 02 §7.5 — loaded + evicted chunks, written before the commit point.
        let mut chunk_attempts = 0usize;
        let mut chunk_failures = 0usize;
        let mut first_failure: Option<String> = None;
        for ((cx, cy, cz), chunk) in self.world.persistable_chunks() {
            if chunk.is_empty() { continue; }
            chunk_attempts += 1;
            let path = chunks_dir.join(format!("{cx}_{cy}_{cz}.chunk"));
            if let Err(e) = crate::save::write_atomic_nosync(&path, &chunk.as_bytes()) {
                log::error!("Server save: chunk write failed ({}): {e}", path.display());
                chunk_failures += 1;
                first_failure.get_or_insert(e);
            }
        }
        crate::save::sync_dir(&chunks_dir);
        if chunk_failures > 0 && chunk_failures == chunk_attempts {
            return Err(format!(
                "all {chunk_failures} chunk write(s) failed, world.dat not updated; first: {}",
                first_failure.unwrap_or_default()
            ));
        }

        let world_dat = dir.join("world.dat");
        crate::save::write_atomic(&world_dat, &encoded)?;

        // Creator-gallery (Spec 2026-06-19 §9) — write an `exhibits.json` sidecar
        // next to `world.dat` so the Operator Console (which can't decode the
        // bincode `world.dat`) can list the world's placed exhibits. Best-effort.
        let exhibits_path = dir.join("exhibits.json");
        match serde_json::to_vec_pretty(&self.world.exhibits) {
            Ok(json) => {
                if let Err(e) = crate::save::write_atomic(&exhibits_path, &json) {
                    log::warn!("exhibits.json sidecar write failed ({}): {e}", exhibits_path.display());
                }
            }
            Err(e) => log::warn!("exhibits.json sidecar serialise failed ({}): {e}", exhibits_path.display()),
        }
        match first_failure {
            Some(first) => Err(format!(
                "{chunk_failures} of {chunk_attempts} chunk write(s) failed (world.dat written); first: {first}"
            )),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// T0-7 — the dedicated-server save round-trips through tmp + rename and
    /// leaves no `.tmp` behind.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn server_save_is_atomic_and_round_trips() {
        let name = "__test_server_atomic_save_round_trip__";
        let dir = crate::save::world_dir(name);
        let _ = std::fs::remove_dir_all(&dir);

        let mut server = GameServer::new(1, name.to_string(), 42);
        server.world.set_block(3, 64, 5, crate::block::BEDROCK);
        server.try_save().expect("save succeeds");

        assert!(dir.join("world.dat").is_file());
        assert!(dir.join("exhibits.json").is_file());
        assert!(dir.join("chunks/0_4_0.chunk").is_file(), "the edited chunk was written");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .chain(std::fs::read_dir(dir.join("chunks")).unwrap())
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .map(|e| e.path())
            .collect();
        assert!(leftovers.is_empty(), "atomic writes leave no .tmp: {leftovers:?}");

        let mut server2 = GameServer::new(1, name.to_string(), 42);
        server2.initial_load();
        assert_eq!(server2.world.get_block(3, 64, 5), crate::block::BEDROCK, "block survives save/load");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T0-7 — a chunk write failure is reported (with the chunk's path), not
    /// swallowed. While other chunks still save, `world.dat` IS written so
    /// player/block-entity state never falls behind chunks already renamed in;
    /// only when every chunk fails is `world.dat` left alone.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn server_save_reports_chunk_failure_and_keeps_world_dat_in_step() {
        let name = "__test_server_atomic_save_chunk_failure__";
        let dir = crate::save::world_dir(name);
        let _ = std::fs::remove_dir_all(&dir);

        let mut server = GameServer::new(1, name.to_string(), 42);
        server.world.set_block(3, 64, 5, crate::block::BEDROCK);
        server.world.set_block(40, 64, 5, crate::block::BEDROCK); // chunk 2_4_0
        server.try_save().expect("first save succeeds");
        assert!(dir.join("chunks/2_4_0.chunk").is_file(), "second chunk written");

        // Inject a failure without permissions (works as root/CI): a directory
        // where a chunk's tmp file must go makes that write fail.
        std::fs::create_dir_all(dir.join("chunks/0_4_0.chunk.tmp")).unwrap();

        // One of several chunks fails: Err names it, world.dat still written.
        std::fs::remove_file(dir.join("world.dat")).unwrap();
        let err = server.try_save().expect_err("a failed chunk write must fail the save");
        assert!(err.contains("0_4_0.chunk"), "error names the chunk path: {err}");
        assert!(dir.join("world.dat").is_file(), "world.dat must keep step with the chunks that saved");

        // Every chunk fails: world.dat is left alone.
        for e in std::fs::read_dir(dir.join("chunks")).unwrap().filter_map(|e| e.ok()) {
            let file = e.file_name().to_string_lossy().into_owned();
            if file.ends_with(".chunk") {
                std::fs::create_dir_all(dir.join("chunks").join(format!("{file}.tmp"))).unwrap();
            }
        }
        std::fs::remove_file(dir.join("world.dat")).unwrap();
        let err = server.try_save().expect_err("all chunks failing must fail the save");
        assert!(err.contains("not updated"), "{err}");
        assert!(!dir.join("world.dat").exists(), "no world.dat when no chunk saved");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #8 — `GameServer::new` must seed its `BiomeGenerator` from the seed it is
    /// given, NOT the old hardcoded 42. Without this the LAN-host server
    /// regenerates every world at seed 42 regardless of the world's real seed,
    /// so hosted terrain silently diverges from the (correctly-seeded) client.
    #[test]
    fn game_server_seeds_biome_gen_from_argument() {
        let server = GameServer::new(1, "seed_arg_test".to_string(), 12345);
        assert_eq!(
            server.biome_gen.seed, 12345,
            "GameServer must use the provided seed, not hardcoded 42"
        );
    }

    /// Finding #1 regression — the authoritative server must simulate lava.
    /// Before this fix `GameServer` had no lava system and never ticked it, so a
    /// lava source (world-gen pool or emptied bucket) sat static forever on any
    /// hosted/dedicated world. Drive real `tick()`s and confirm it flows.
    #[test]
    fn game_server_simulates_lava_flow() {
        let mut server = GameServer::new(1, "lava_tick".to_string(), 42);
        // A stone floor with a lava source floating one block above it, placed
        // straight onto the server world (no world-gen needed for the check).
        for x in -3..=3 {
            for z in -3..=3 {
                server.world.set_block(x, 60, z, crate::block::STONE);
            }
        }
        server.world.set_block(0, 64, 0, crate::block::LAVA);
        server.lava.add_source(0, 64, 0);
        assert_eq!(server.world.get_block(0, 63, 0), crate::block::AIR);

        // tick() runs fluids on a 4-tick cadence and lava crawls on top of that,
        // so give it plenty of server ticks.
        for _ in 0..120 {
            server.tick();
        }

        assert_eq!(
            server.world.get_block(0, 63, 0),
            crate::block::LAVA,
            "lava must pour downward under GameServer::tick (was static before)"
        );
    }

    /// The server must not just advance fluids internally — it must queue the
    /// moved cells as `BlockChange`s so remote clients see the flow. Without the
    /// broadcast the server sim was invisible until a full chunk resync.
    #[test]
    fn game_server_broadcasts_fluid_block_changes() {
        let mut server = GameServer::new(1, "lava_broadcast".to_string(), 42);
        for x in -3..=3 {
            for z in -3..=3 {
                server.world.set_block(x, 60, z, crate::block::STONE);
            }
        }
        server.world.set_block(0, 64, 0, crate::block::LAVA);
        server.lava.add_source(0, 64, 0);

        // Drive ticks; `pending_block_changes` accumulates (a bare GameServer
        // has no HostedServer draining it) so a flow delta must show up.
        let mut saw_lava_delta = false;
        for _ in 0..120 {
            server.tick();
            if server.pending_block_changes.iter().any(|bc| {
                bc.x == 0 && bc.y == 63 && bc.z == 0 && bc.new_block == crate::block::LAVA
            }) {
                saw_lava_delta = true;
                break;
            }
        }
        assert!(
            saw_lava_delta,
            "the lava that flowed down must be queued as a BlockChange for clients",
        );
    }

    /// Water depth levels (meta AUX) must ride the fluid delta broadcast —
    /// remote clients render the sloped surface and compute the same flow
    /// vector as the host only if `BlockChange.meta` carries the level.
    #[test]
    fn game_server_broadcasts_water_depth_meta() {
        let mut server = GameServer::new(1, "water_meta".to_string(), 42);
        for x in -10..=10 {
            for z in -10..=10 {
                server.world.set_block(x, 60, z, crate::block::STONE);
            }
        }
        server.world.set_block(0, 61, 0, crate::block::WATER);
        server.water.add_source(0, 61, 0);

        let mut saw_leveled_delta = false;
        for _ in 0..200 {
            server.tick();
            if server.pending_block_changes.iter().any(|bc| {
                bc.new_block == crate::block::WATER
                    && bc.y == 61
                    && crate::meta::aux(bc.meta) == 2
                    && (bc.x.abs() + bc.z.abs()) == 2
            }) {
                saw_leveled_delta = true;
                break;
            }
        }
        assert!(
            saw_leveled_delta,
            "a spread cell two blocks out must broadcast depth level 2 in BlockChange.meta",
        );
    }

    /// Fire must burn on the authoritative server (the lava lesson, d2f3b02d):
    /// a hosted/dedicated world's fire spreads into fuel and the consumed cells
    /// broadcast as BlockChanges.
    #[test]
    fn game_server_simulates_and_broadcasts_fire() {
        let mut server = GameServer::new(1, "fire_tick".to_string(), 42);
        for x in -3..=3 {
            for z in -3..=3 {
                server.world.set_block(x, 60, z, crate::block::STONE);
            }
        }
        server.world.set_block(1, 61, 0, crate::block::OAK_PLANKS);
        assert!(server.fire.ignite(&mut server.world, 0, 61, 0, 0));

        let mut planks_burned = false;
        let mut saw_fire_delta = false;
        for _ in 0..600 {
            server.tick();
            if server.world.get_block(1, 61, 0) == crate::block::FIRE {
                planks_burned = true;
            }
            if server
                .pending_block_changes
                .iter()
                .any(|bc| bc.new_block == crate::block::FIRE)
            {
                saw_fire_delta = true;
            }
            if planks_burned && saw_fire_delta {
                break;
            }
        }
        assert!(planks_burned, "server fire consumed the adjacent planks");
        assert!(saw_fire_delta, "fire spread broadcast as BlockChange");
    }

    /// `fire_spread_enabled = false` must protect blocks on the server too.
    #[test]
    fn game_server_fire_toggle_off_protects_fuel() {
        let mut server = GameServer::new(1, "fire_toggle".to_string(), 42);
        server.fire_spread_enabled = false;
        for x in -3..=3 {
            for z in -3..=3 {
                server.world.set_block(x, 60, z, crate::block::STONE);
            }
        }
        server.world.set_block(1, 61, 0, crate::block::OAK_PLANKS);
        assert!(server.fire.ignite(&mut server.world, 0, 61, 0, 0));
        for _ in 0..600 {
            server.tick();
        }
        assert_eq!(
            server.world.get_block(1, 61, 0),
            crate::block::OAK_PLANKS,
            "toggle off: fuel untouched"
        );
        assert_eq!(
            server.world.get_block(0, 61, 0),
            crate::block::AIR,
            "the flame still burnt out"
        );
    }

    // ─── Death-drops phase 2b: server-side pickup for server-simulated players ───

    /// Shared scaffolding: a 1-player server with a stone floor and a bone
    /// stack dropped exactly at the player. Caller flips flags then ticks.
    fn pickup_server_with_drop(stack: crate::item::ItemStack) -> GameServer {
        let mut server = GameServer::new(1, "pickup".to_string(), 42);
        for x in -3..=3 {
            for z in -3..=3 {
                server.world.set_block(x, 60, z, crate::block::STONE);
            }
        }
        let pos = Vec3::new(0.5, 61.0, 0.5);
        server.players[0].player.pos = pos;
        crate::entity::spawn_item(&mut server.ecs, pos, stack, 1);
        server
    }

    fn bone_count(server: &GameServer) -> u32 {
        server.players[0]
            .inventory
            .slots_iter()
            .flatten()
            .filter(|s| {
                matches!(
                    s.item,
                    crate::item::Item::Material(crate::item::MaterialId::Bone)
                )
            })
            .map(|s| s.count as u32)
            .sum()
    }

    /// The whole chain: `tick()` decays pickup_delay (tick_item_lifetimes),
    /// then grants the settled stack to a server-simulated player — into the
    /// server-side inventory AND onto `pending_item_grants` for the wire.
    #[test]
    fn server_grants_dropped_item_to_server_simulated_player() {
        let mut server = pickup_server_with_drop(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Bone,
            2,
        ));
        server.players[0].server_simulated = true;

        // Grants are per-tick data (cleared each tick), so watch every tick.
        let mut seen_grant: Option<(usize, crate::item::ItemStack)> = None;
        for _ in 0..(crate::entity::ITEM_PICKUP_DELAY_TICKS + 20) {
            server.tick();
            if let Some(g) = server.pending_item_grants.first() {
                seen_grant = Some(g.clone());
            }
        }

        assert_eq!(bone_count(&server), 2, "stack landed in the server-side inventory");
        let (idx, stack) = seen_grant.expect("a grant must be queued for the wire");
        assert_eq!(idx, 0);
        assert_eq!(stack.count, 2);
        assert_eq!(
            server.ecs.query::<&crate::entity::ItemEntity>().iter().count(),
            0,
            "picked-up item despawned from the server ECS"
        );
    }

    /// Local (position-trusted) players pick up from their own client sim —
    /// the server granting too would double-count under the single-player
    /// parity flag. Their drops stay on the server floor.
    #[test]
    fn server_pickup_skips_local_position_trusted_players() {
        let mut server = pickup_server_with_drop(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Bone,
            2,
        ));
        // players[0] keeps the default server_simulated = false.
        for _ in 0..(crate::entity::ITEM_PICKUP_DELAY_TICKS + 20) {
            server.tick();
            assert!(server.pending_item_grants.is_empty(), "no grant for a local player");
        }
        assert_eq!(bone_count(&server), 0);
        assert_eq!(
            server.ecs.query::<&crate::entity::ItemEntity>().iter().count(),
            1,
            "item stays on the ground for the client sim to handle"
        );
    }

    /// Death-drops phase 3 (v61) — a tool drop IS granted now, at its true
    /// durability. Before the full-fidelity wire the drop sat on the server
    /// floor until lifetime expiry, because the `(kind, id)` pair collapsed it
    /// to a bare material tier.
    #[test]
    fn server_grants_a_tool_drop_with_durability_preserved() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        pick.durability = 37;
        let mut server = pickup_server_with_drop(crate::item::ItemStack::new_tool(pick));
        server.players[0].server_simulated = true;

        let mut seen_grant: Option<(usize, crate::item::ItemStack)> = None;
        for _ in 0..(crate::entity::ITEM_PICKUP_DELAY_TICKS + 20) {
            server.tick();
            if let Some(g) = server.pending_item_grants.first() {
                seen_grant = Some(g.clone());
            }
        }

        let (idx, stack) = seen_grant.expect("a tool grant must be queued for the wire");
        assert_eq!(idx, 0);
        assert_eq!(
            stack.item,
            crate::item::Item::Tool(pick),
            "the half-worn pickaxe is granted as itself, not re-minted"
        );
        assert!(
            server.players[0]
                .inventory
                .slots_iter()
                .flatten()
                .any(|s| s.item == crate::item::Item::Tool(pick)),
            "and it lands in the server-side inventory too"
        );
        assert_eq!(
            server.ecs.query::<&crate::entity::ItemEntity>().iter().count(),
            0,
            "picked-up tool despawned from the server ECS"
        );
    }

    /// Plans have no wire form (`plan::PlanData` is far too heavy for a
    /// per-tick broadcast) and stay floor-bound by design — no grant, no
    /// magnet, no despawn.
    #[test]
    fn server_pickup_never_grants_plans() {
        let mut server = pickup_server_with_drop(crate::item::ItemStack {
            item: crate::item::Item::Plan(crate::plan::PlanData::debug_3x3_stone()),
            count: 1,
        });
        server.players[0].server_simulated = true;
        for _ in 0..(crate::entity::ITEM_PICKUP_DELAY_TICKS + 20) {
            server.tick();
            assert!(server.pending_item_grants.is_empty(), "plan must not be granted");
        }
        assert_eq!(
            server.ecs.query::<&crate::entity::ItemEntity>().iter().count(),
            1,
            "floor-bound drop stays on the ground"
        );
    }

    /// A disconnected slot's `ServerPlayer` lingers (indexes stay stable for
    /// the transports array) — it must not hoover loot nobody can receive.
    #[test]
    fn server_pickup_skips_disconnected_players() {
        let mut server = pickup_server_with_drop(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Bone,
            2,
        ));
        server.players[0].server_simulated = true;
        server.players[0].connected = false;
        for _ in 0..(crate::entity::ITEM_PICKUP_DELAY_TICKS + 20) {
            server.tick();
            assert!(server.pending_item_grants.is_empty());
        }
        assert_eq!(bone_count(&server), 0);
        assert_eq!(
            server.ecs.query::<&crate::entity::ItemEntity>().iter().count(),
            1,
            "ghost slot leaves the drop for connected players"
        );
    }

    /// Server items age out — without `tick_item_lifetimes` in the server
    /// tick, drops in a hosted world accumulate forever.
    #[test]
    fn server_items_age_out_by_lifetime() {
        let mut server = pickup_server_with_drop(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Bone,
            2,
        ));
        // Nobody eligible to pick it up.
        // Shrink the lifetime so the test doesn't tick 5 real minutes.
        for (_id, life) in server.ecs.query_mut::<&mut crate::entity::Lifetime>() {
            life.0 = 3;
        }
        for _ in 0..6 {
            server.tick();
        }
        assert_eq!(
            server.ecs.query::<&crate::entity::ItemEntity>().iter().count(),
            0,
            "expired drop despawns server-side"
        );
    }

    /// Headless benchmark — not a correctness test, a performance tripwire.
    /// Verify `GameServer::tick` can clear the 50 ms / 20 TPS budget on
    /// dev-profile with a single-player world. Threshold is conservative
    /// (well above median) so CI doesn't flake on slow hardware. Regressions
    /// tighten the threshold.
    /// Audit 2026-09-27: `pending_intent` was overwritten per packet, so two
    /// inputs bunched into one tick lost a movement step. Now both are
    /// simulated, one per tick — and a flood still moves one step a tick.
    #[test]
    fn bunched_intents_are_all_simulated_one_per_tick() {
        let mut sp = ServerPlayer::new(Vec3::ZERO);
        let a = crate::player_intent::PlayerIntent { move_forward: 1.0, ..Default::default() };
        let b = crate::player_intent::PlayerIntent { move_forward: -1.0, ..Default::default() };
        sp.queue_intent(a.clone());
        sp.queue_intent(b.clone());
        assert_eq!(sp.pending_intent.as_ref().map(|i| i.move_forward), Some(1.0));
        assert_eq!(sp.intent_queue.len(), 1);
        // A flood is bounded: oldest queued intents drop, the pending one stays.
        for _ in 0..10 {
            sp.queue_intent(b.clone());
        }
        assert_eq!(sp.intent_queue.len(), MAX_QUEUED_INTENTS);
        assert_eq!(sp.pending_intent.as_ref().map(|i| i.move_forward), Some(1.0));

        let mut server = GameServer::new(1, "intent-queue-test".into(), 42);
        server.players[0] = sp;
        server.players[0].server_simulated = true;
        server.tick_player_physics();
        assert!(server.players[0].pending_intent.is_none());
        assert_eq!(server.players[0].intent_queue.len(), MAX_QUEUED_INTENTS);
        server.tick_player_physics();
        assert_eq!(
            server.players[0].intent_queue.len(),
            MAX_QUEUED_INTENTS - 1,
            "exactly one intent is consumed per tick"
        );
    }

    #[test]
    fn a_new_player_hears_nothing_until_a_verified_policy_raises_it() {
        let sp = ServerPlayer::new(Vec3::ZERO);
        assert_eq!(sp.comms.level, crate::comms::CommsLevel::Blocked);
    }

    #[test]
    fn tick_stays_under_budget() {
        // Create a minimal server with 1 player and pre-generated world.
        let mut server = GameServer::new(1, "bench".to_string(), 42);
        server.initial_load();
        let player_count = server.player_count();
        assert_eq!(player_count, 1);

        // Warm-up — first tick pays JIT-style costs (allocator, cache fills).
        for _ in 0..20 {
            server.tick();
        }

        // Bench 200 ticks = 10 seconds of game time at 20 TPS.
        const TICK_COUNT: u32 = 200;
        let start = Instant::now();
        for _ in 0..TICK_COUNT {
            server.tick();
        }
        let total = start.elapsed();
        let per_tick_ms = total.as_micros() as f64 / (TICK_COUNT as f64 * 1000.0);

        eprintln!(
            "GameServer::tick bench — {TICK_COUNT} ticks in {total:?}, mean {per_tick_ms:.2} ms/tick"
        );

        // Threshold: 25 ms/tick mean (half the 50 ms budget). Dev-profile
        // build with opt-level=1 on modest hardware should clear this with
        // plenty of headroom on a 1-player world.
        assert!(
            per_tick_ms < 25.0,
            "mean tick time {per_tick_ms:.2} ms exceeds 25 ms budget"
        );
    }

    // ─── Task 2.1: held ItemRef resolution ───

    #[test]
    fn server_player_item_ref_reads_active_hotbar_slot() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::item::{Item, ItemStack};
        use crate::protocol::ItemRef;
        let mut sp = ServerPlayer::new(Vec3::new(0.0, 0.0, 0.0));
        // Empty slot → Empty.
        assert_eq!(server_player_item_ref(&sp), ItemRef::Empty);
        // Iron tool in slot 3, select slot 3 → Tool(2).
        sp.inventory.set_slot(
            3,
            Some(ItemStack {
                item: Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron)),
                count: 1,
            }),
        );
        sp.hotbar_slot = 3;
        assert_eq!(server_player_item_ref(&sp), ItemRef::Tool(2));
        // Switch to an empty slot 0 → Empty again.
        sp.hotbar_slot = 0;
        assert_eq!(server_player_item_ref(&sp), ItemRef::Empty);
        // Block in slot 0.
        sp.inventory
            .set_slot(0, Some(ItemStack::new_block(42, 5)));
        assert_eq!(server_player_item_ref(&sp), ItemRef::Block(42));
    }

    #[test]
    fn broadcast_held_ref_source_depends_on_player_type() {
        use crate::item::ItemStack;
        use crate::protocol::{item_kind, ItemRef};
        let mut sp = ServerPlayer::new(Vec3::new(0.0, 0.0, 0.0));

        // Save-time inventory: block 42 in the active slot. The client-sent
        // ref carries a *different* live item (a tool) — what a remote player
        // actually holds after switching hotbar slots in-game.
        sp.inventory.set_slot(0, Some(ItemStack::new_block(42, 5)));
        sp.hotbar_slot = 0;
        let (live_kind, live_id) = ItemRef::Tool(2).to_wire();
        sp.held_kind = live_kind;
        sp.held_id = live_id;

        // Remote (server-simulated): trust the client-sent live ref, NOT the
        // stale save-time inventory. This is the regression fix.
        sp.server_simulated = true;
        assert_eq!(broadcast_held_ref(&sp), ItemRef::Tool(2).to_wire());

        // Local (position-trusted): the host's own inventory is live, so
        // resolve from the active hotbar slot (block 42), ignoring held_kind.
        sp.server_simulated = false;
        assert_eq!(broadcast_held_ref(&sp), ItemRef::Block(42).to_wire());

        // A remote player who has never sent input → Empty (EMPTY/0 default).
        let mut fresh = ServerPlayer::new(Vec3::new(0.0, 0.0, 0.0));
        fresh.server_simulated = true;
        assert_eq!(fresh.held_kind, item_kind::EMPTY);
        assert_eq!(broadcast_held_ref(&fresh), ItemRef::Empty.to_wire());
    }

    // ─── Task 2.2: anim_state + flags authoring ───

    #[test]
    fn player_anim_fields_jump_when_airborne() {
        use crate::protocol::player_flags as pf;
        // Airborne wins even if moving fast.
        let (anim, flags) = player_anim_fields(5.0, false, false, false);
        assert_eq!(anim, 2);
        assert_eq!(flags & pf::ON_GROUND, 0);
    }

    #[test]
    fn player_anim_fields_walk_when_moving_on_ground() {
        let (anim, _) = player_anim_fields(1.0, true, false, false);
        assert_eq!(anim, 1);
    }

    #[test]
    fn player_anim_fields_idle_when_still_on_ground() {
        let (anim, _) = player_anim_fields(0.0, true, false, false);
        assert_eq!(anim, 0);
        // Just-below-eps still idle.
        let (anim2, _) = player_anim_fields(0.04, true, false, false);
        assert_eq!(anim2, 0);
    }

    #[test]
    fn player_anim_fields_swinging_iff_acting() {
        use crate::protocol::player_flags as pf;
        let (_, flags_acting) = player_anim_fields(0.0, true, false, true);
        assert_ne!(flags_acting & pf::SWINGING, 0);
        let (_, flags_idle) = player_anim_fields(0.0, true, false, false);
        assert_eq!(flags_idle & pf::SWINGING, 0);
    }

    #[test]
    fn player_anim_fields_crouching_iff_sneak() {
        use crate::protocol::player_flags as pf;
        let (_, flags_sneak) = player_anim_fields(0.0, true, true, false);
        assert_ne!(flags_sneak & pf::CROUCHING, 0);
        let (_, flags_no) = player_anim_fields(0.0, true, false, false);
        assert_eq!(flags_no & pf::CROUCHING, 0);
    }

    #[test]
    fn player_anim_fields_crouch_is_never_an_anim_state() {
        use crate::protocol::player_flags as pf;
        // Idle + sneak: anim_state stays 0 (idle), crouch is a flag only.
        assert_eq!(player_anim_fields(0.0, true, true, false).0, 0);
        // Sneak while walking: anim_state is 1 (walk) AND the CROUCHING flag
        // is set — crouch combines with locomotion, never replaces it.
        let (s, f) = player_anim_fields(1.0, true, true, false);
        assert_eq!(s, 1);
        assert_ne!(f & pf::CROUCHING, 0);
    }

    #[test]
    fn player_anim_fields_on_ground_flag_tracks_on_ground() {
        use crate::protocol::player_flags as pf;
        let (_, grounded) = player_anim_fields(0.0, true, false, false);
        assert_ne!(grounded & pf::ON_GROUND, 0);
        let (_, airborne) = player_anim_fields(0.0, false, false, false);
        assert_eq!(airborne & pf::ON_GROUND, 0);
    }

    #[test]
    fn player_anim_fields_flags_combine() {
        use crate::protocol::player_flags as pf;
        // Walking + crouching + acting + on ground all at once.
        let (anim, flags) = player_anim_fields(1.0, true, true, true);
        assert_eq!(anim, 1);
        assert_ne!(flags & pf::SWINGING, 0);
        assert_ne!(flags & pf::CROUCHING, 0);
        assert_ne!(flags & pf::ON_GROUND, 0);
    }

    // ─── local sign-in identity (world chat §3.5) ───

    #[test]
    fn local_identity_decodes_a_real_pubkey() {
        let hex_key = "a".repeat(64);
        let got = super::local_identity_pubkey(Some(&hex_key)).expect("valid 32-byte hex");
        assert_eq!(got, [0xaa; 32]);
    }

    #[test]
    fn local_identity_is_none_when_not_signed_in() {
        assert!(super::local_identity_pubkey(None).is_none());
    }

    /// Strictness is the point: a half-parsed identity would be worse than no
    /// identity, because it would silently be somebody else's key.
    #[test]
    fn local_identity_rejects_anything_not_exactly_32_bytes_of_hex() {
        assert!(super::local_identity_pubkey(Some("")).is_none());
        assert!(super::local_identity_pubkey(Some(&"a".repeat(62))).is_none());
        assert!(super::local_identity_pubkey(Some(&"a".repeat(66))).is_none());
        assert!(super::local_identity_pubkey(Some(&"z".repeat(64))).is_none());
        assert!(super::local_identity_pubkey(Some("npub1abc")).is_none());
    }
}

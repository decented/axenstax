//! Per-player state bundle — everything that's unique to each player in a multiplayer game.
//!
//! In split screen: 2-4 of these exist in GameState.
//! In host-as-server: the host has local PlayerSlots, remote players have server-side state.

use glam::Vec3;

use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
use crate::camera::Camera;
use crate::combat::PlayerCombat;
use crate::craft_ui::CraftingUi;
use crate::input::InputState;
use crate::inventory::Inventory;
use crate::item::ItemStack;
use crate::physics::Player;

/// Ticks the creative break cooldown holds between block breaks. At 20 TPS,
/// 5 ticks ≈ 0.25 s (~4 breaks/s) — brisk enough for building, far below the
/// effectively-uncapped ~20/s a held button used to give in creative. Tunable;
/// Axolittle playtest will refine the feel. Spec 05.
pub const BLOCK_BREAK_COOLDOWN_TICKS: u32 = 5;

/// Ticks the short "just placed/interacted with a workstation" cooldown
/// holds before the next placement/interaction is allowed (e.g. campfire,
/// drying rack, blueprint stamp). Hoisted from a bare `4` repeated at every
/// `game_loop.rs` call site (2026-09-06 hardening pass) so the tuned value
/// lives in one place. Feel-tunable.
pub const PLACE_COOLDOWN_TICKS: u32 = 4;

/// Unique per-player state.
pub struct PlayerSlot {
    /// Player physics (position, velocity, on_ground, flying).
    pub player: Player,
    /// Camera for this player's viewport.
    pub camera: Camera,
    /// Input state (keyboard/mouse or gamepad). Vestigial — never written; all
    /// real input flows through `GameState.input`/`GameState.gamepad` via
    /// `route_intents` (see the existing note at game_loop.rs ~1372).
    #[allow(dead_code)]
    pub input: InputState,
    /// Inventory (36 slots).
    pub inventory: Inventory,
    /// Combat state (health, attack cooldown, death).
    pub combat: PlayerCombat,
    /// Currently selected hotbar slot.
    pub hotbar_slot: usize,
    /// Earliest tick at which the mouse-wheel may advance the hotbar again.
    /// Quantises scroll so a fast spin / high-DPI wheel doesn't run the
    /// selection away (#11). Not persisted — transient input state.
    pub hotbar_scroll_ready_tick: u64,
    /// C2b — earliest tick at which a joined client may Q-drop again
    /// (`item_actions::DROP_INTERVAL_TICKS` after the last), the rate the
    /// server's drop bucket refills at. Single-player and a host's seats
    /// drop as before. Not persisted — transient input state.
    pub drop_ready_tick: u64,
    /// Sub-notch scroll carried between ticks. One wheel notch (~1.0) selects
    /// exactly one slot; fractional/trackpad deltas accumulate here until a whole
    /// notch is reached, so selection no longer walks at a fixed rate
    /// ("selector too fast" report). Not persisted — transient input state.
    pub hotbar_scroll_accum: f32,
    /// Crafting UI state.
    pub crafting_ui: CraftingUi,
    /// Currently targeted block.
    pub target_block: Option<[i32; 3]>,
    /// Face normal of targeted block.
    pub target_face: [i32; 3],
    /// Whether this player's cursor is captured (for mouse look). Same
    /// vestigial story as `input` above — the live cursor-capture state
    /// lives on `GameState`, not per-slot.
    #[allow(dead_code)]
    pub cursor_captured: bool,
    /// Player index (0 = primary keyboard+mouse, 1+ = gamepad/secondary). Set
    /// at construction; nothing reads it back (slots are indexed by their
    /// position in `GameState.players` instead).
    #[allow(dead_code)]
    pub index: usize,
    /// Block currently being mined (survival mode break timer).
    pub breaking_pos: Option<[i32; 3]>,
    /// Ticks spent mining the current block.
    pub break_progress: u32,
    /// Owner-inbox #1/2/3 — true while the current left-hold has peeled a
    /// wallpaper overlay; gates the block-break until the button is released, so
    /// one strike = one stage (peel, THEN break) even in creative (instant
    /// break). Cleared each frame the break button is up. Transient (not saved).
    pub peel_latch: bool,
    /// Placement cooldown (ticks remaining until next place allowed).
    /// Known debt (C3c-2-fix): it counts FRAMES, not game ticks
    /// ([`Self::tick_place_cooldown`] runs once a frame), so the right-click
    /// arms it paces run faster at a higher frame rate. The bow and the
    /// slingshot no longer use it for their cadence ([`Self::next_shot_tick`]).
    pub place_cooldown: u32,
    /// C3c-2-fix (M1) — the game tick (`GameState::tick_counter`) from which
    /// this player's bow or slingshot may shoot again: ONE tick-based cadence
    /// on every seat ([`crate::shot::SHOT_COOLDOWN_TICKS`]), the cadence the
    /// server holds a joiner's shots to, so an honest held button is never
    /// refused. Transient (not saved).
    pub next_shot_tick: u64,
    /// Eating cooldown, in fixed ticks (C2a verify M1): eating needs it at
    /// zero and arms it to `item_actions::EAT_COOLDOWN_TICKS`. Counted in the
    /// fixed-tick loop, never per frame, so a bite is 0.8 s at any frame rate.
    /// Transient (not saved).
    pub eat_cooldown: u32,
    /// Break cooldown (ticks remaining until the next *creative* block break is
    /// allowed). Survival breaking is already paced by `break_time` (block
    /// hardness via `breaking_pos`/`break_progress`), but creative breaks are
    /// instant, so without this a held button smashes one block every tick
    /// (~20/s). Mirrors `place_cooldown`. Transient (not saved). Spec 05.
    pub break_cooldown: u32,
    /// Where the player respawns on death. A new world sets it to the player's
    /// start ([`Self::start_at`]); bed-settable (P4) by sleeping in a bed;
    /// `/spawnpoint` sets it too. It is the point asked for, not the spot the
    /// body stands on: the respawn settles it on dry ground
    /// (`spawn_safety::safe_respawn_point`) and leaves the saved value alone.
    pub spawn_pos: Vec3,
    /// P6 — an in-progress fishing cast (None = not fishing). Transient: a cast
    /// doesn't survive a save, like other runtime player state.
    pub fishing: Option<crate::fishing::FishingLine>,
    /// Spec 17 Phase 6 — campfire friction-ignition target. Set when the
    /// player starts a stick-rub on an unlit fueled campfire; cleared
    /// each frame the player isn't actively holding right-click on it.
    /// `(position, started_at)`.
    pub friction_target: Option<([i32; 3], web_time::Instant)>,
    /// Spec 19 anti-grief — tick at which this player most recently swung at
    /// a villager. The next swing on a villager within 30 s (600 ticks @ 20 TPS)
    /// actually damages; the first one just toasts a warning. None = never
    /// hit one (or the timer expired and the next hit warns again).
    pub last_villager_warn_tick: Option<u64>,
    /// Spec 19 phase 5 — entity reference for the villager whose dialogue is
    /// currently open in front of this player (None = no dialogue). Transient
    /// — not persisted across save/load.
    pub dialogue_villager: Option<hecs::Entity>,
    /// The Satoshi onboarding guide whose warm dialogue is open for this player
    /// (None = closed). Transient — not persisted (his per-world *progress* lives
    /// on `World.satoshi`). Separate from `dialogue_villager` so his bespoke
    /// scripted panel renders instead of the generic quest dialogue.
    pub dialogue_satoshi: Option<hecs::Entity>,
    /// Per-villager decline cooldown — quest declined → 5-min lockout (Spec 19
    /// phase 5). Keyed by villager `hecs::Entity`; value is the tick at which
    /// the decline was issued so we can compare against the engine's
    /// `tick_counter`. Transient.
    pub villager_decline_until: ahash::AHashMap<hecs::Entity, u64>,
    /// Spec 19 phase 6 — currently-accepted quests keyed by villager entity.
    /// One slot per (player, villager) pair per the spec. Transient on
    /// alpha; Phase 11 may persist.
    pub active_quests: ahash::AHashMap<hecs::Entity, crate::quest::Quest>,
    /// Phase 6 kill-counter — accumulates kills per MobType while a quest is
    /// active. The dialogue's completion check reads this against the active
    /// quest's `Kill` target. Doesn't reset between quests — each new Kill
    /// quest captures a snapshot of the current count and checks delta.
    pub kill_counter: ahash::AHashMap<crate::mob::MobType, u32>,
    /// Identity latched by the imported-world loader; local snapshot capture only.
    pub(crate) personal_binding: Option<crate::progression::GameplayBinding>,
    pub(crate) quest_progress: crate::progression::QuestProgress,
    pub(crate) gameplay_revision: u64,
    pub(crate) gameplay_clock: crate::quest_state::GameplayClock,
    pub(crate) kill_tracking: crate::progression::KillTracking,
    /// A bound mutation was lost in this live holder. Ordinary installation,
    /// saving and a later successful mutation cannot establish completeness.
    gameplay_carry_incomplete: bool,
    /// Snapshot of `kill_counter` at the moment a Kill quest was accepted;
    /// delta = current - snapshot. None for non-Kill quests.
    pub kill_quest_baseline: ahash::AHashMap<hecs::Entity, (crate::mob::MobType, u32)>,
    /// Spec 19 follow-on — per-player Charter sats flag. True = guardian
    /// has enabled Bitcoin-touching gameplay for this player; false = the
    /// kid plays in Barter-only mode regardless of server policy. Defaults
    /// to **false** (audit 2026-09-27): a missing guardian record is not
    /// consent. Only an explicit guardian opt-in (the Charter credential,
    /// when Spec 1 Phase 4 wires it) turns it on. Per
    /// `project_bitcoin_parent_controlled.md`.
    pub charter_allows_sats: bool,
    /// World chat (Phase 3) — this player's Charter comms **ceiling**, per
    /// `docs/foundations/2026-09-05-world-chat.md` §2.5/§3.3. Assigned once,
    /// here, and meant to be passed as an explicit function parameter to any
    /// future consumer rather than read off `self` deep in a helper — the same
    /// discipline as `charter_allows_sats` above, which is why that flag has
    /// never been accidentally bypassed.
    ///
    /// Defaults to `Approved`, never `Anyone`: a missing guardian record is
    /// not consent (§2.6).
    ///
    /// Like `charter_allows_sats` (now default-off), `charter_comms` does not
    /// default to the permissive end. The real enforcement point
    /// is server-side (`ServerPlayer.comms`), resolved at join in
    /// `hosted_server.rs` via `crate::charter::comms_level` for exactly this
    /// player's verified pubkey (§3.3, §2.5). This field is the client-local
    /// echo of that same ceiling for future HUD use (e.g. greying the chat
    /// box) and has no consumer yet — nothing turns on it being read.
    #[allow(dead_code)]
    pub charter_comms: crate::comms::CommsLevel,
    /// Spec 33 Mob Bounty Board — per-player claim history. Maps
    /// `ActiveBounty.id` → kill-count consumed at claim time. Re-claiming
    /// the same bounty id is blocked until the rotation refreshes the
    /// id; the map is intentionally NOT cleared on death (no farming
    /// respawn cycles to drain server treasury).
    pub bounties_claimed: ahash::AHashMap<u32, u32>,
    /// Village cells (grid_x, grid_z) this player has been within discovery
    /// range of at least once. Used to fire a one-shot "You found a village!"
    /// toast on first arrival per village. Transient on alpha.
    pub discovered_villages: ahash::AHashSet<(i32, i32)>,
    /// Spec 19 phase 9 — per-village reputation. Quest payouts and villager
    /// kills mutate it; the dialogue path will read its tier in future
    /// iterations to gate behaviour.
    pub reputation: crate::reputation::Reputation,
    /// Tick at which this player last took a villager-kill reputation hit.
    /// 30 s lockout so rapid kills only cost rep once. Distinct from
    /// `last_villager_warn_tick` which gates damage, not rep.
    pub last_villager_kill_rep_tick: Option<u64>,
    /// Spec 24 Phase 5 — transient state for the Capture egui dialog.
    /// Populated by the right-click-BLUEPRINT_PAPER handler in game_loop;
    /// the dialog renders + mutates this and clears it on
    /// Confirm/Cancel. None = no Capture dialog open for this player.
    pub pending_capture: Option<crate::plan::PendingCapture>,
    /// Spec 24 Phase 5 — sticky per-player default for the licence
    /// picker. Set whenever the player confirms a capture; defaults
    /// to None (the dialog falls back to CC-BY-SA, the platform
    /// default per vision doc §5.5). Transient — not persisted.
    pub last_chosen_license: Option<crate::plan::PlanLicense>,
    /// Spec 24 Phase 7 — hotbar slot of a plan currently being
    /// inspected. Set when the player right-clicks while holding a
    /// Plan item; the dialog renders + mutates this. None = no
    /// Inspect dialog open.
    pub pending_inspect: Option<usize>,
    /// Spec 24 Phase 12 — world-space position of an Architect's
    /// Plaque currently being read. Set when the player right-clicks
    /// the plaque block; dialog reads from `world.architect_plaques`
    /// at this position. None = no Plaque dialog open.
    pub pending_plaque: Option<(i32, i32, i32)>,
    /// Spec 27 Phase 9 — world-space position of a Village Bell the
    /// player is reading. Set when the player right-clicks the bell;
    /// dialog enumerates the village's houses + tip-all pool.
    pub pending_bell: Option<(i32, i32, i32)>,
    /// Spec 24 Phase 8 — placement preview state. Set when the player
    /// confirms Place in the Inspect dialog; cleared on confirm-build,
    /// right-click cancel, or Esc. While `Some`, Q/E rotate the ghost,
    /// the cursor anchors via raycast, and left-click commits the
    /// build. Pure client-local — never serialised, never networked.
    pub ghost_state: Option<crate::plan::GhostState>,
    /// Guided build-along — after confirming a plan's placement with the
    /// materials in hand (or in Creative), the "build it automatically vs build
    /// it myself (guide)" choice is pending. Carries `(plan, anchor, rotations)`.
    /// Transient — never serialised.
    pub pending_build_choice: Option<(crate::plan::PlanData, [i32; 3], u8)>,
    /// Spec 20 Phase 5 — world-space position of a Furnace block the
    /// player has open. None = no Furnace UI is showing. Set on
    /// sneak-right-click of the furnace; cleared on Close / Esc.
    /// Slot mutations (fuel-add, input-add, output-take) operate on
    /// `world.furnace_at_mut(pos)` while this is Some.
    pub open_furnace: Option<(i32, i32, i32)>,
    /// Spec 21 Phase 5 — world-space position of a Vendor Block the
    /// player has open. None = no Vendor UI is showing. Set on
    /// right-click; cleared on Close / Esc / break.
    pub open_vendor: Option<(i32, i32, i32)>,
    /// HP-2 (2026-05-22) — world-space position of a Chest block the
    /// player has open. None = no Chest UI is showing. Set on
    /// right-click; cleared on Close / Esc / break. Mirrors `open_furnace`
    /// / `open_vendor`.
    pub open_chest: Option<(i32, i32, i32)>,
    /// Dispenser/Dropper (2026-07-04) — world-space position of the one the
    /// player has open. Mirrors `open_chest`.
    pub open_dispenser: Option<(i32, i32, i32)>,
    /// C3b-1 — a joined client's mirror of the server's container it has
    /// open (`ContainerOpened`), kept by the server's corrections and pushes.
    /// While it is set, the matching `open_chest` / `open_dispenser` /
    /// `open_furnace` names its cell (so every "a screen is open" check
    /// holds) and the screen draws from it, not from the world. Cleared, and
    /// the close sent as a window op, when that field stops naming it or the
    /// cell leaves reach or changes kind (`GameState::tick_shared_container`).
    pub shared_container: Option<crate::container_window::SharedContainer>,
    /// Task 11 (2026-07-06) — the Donkey/Mule ECS entity whose cargo pack
    /// the player has open, plus its world-space position AT OPEN TIME.
    /// Entity-keyed (not block-position-keyed like the other `open_*`
    /// dialogs) since a pack lives on a mobile mob, not a block; the
    /// position is snapshotted once (rather than re-read live each frame)
    /// so `show_container_dialog`'s title-derived egui window id stays
    /// stable while the steed wanders instead of churning every tick. None
    /// = no Pack UI is showing. Set on sneak + empty-hand right-click;
    /// cleared on Close / Esc / the steed vanishing. Transient — never
    /// serialised.
    pub open_pack: Option<(hecs::Entity, (i32, i32, i32))>,
    /// Particle framework (2026-07-05) — previous frame's in-water flag for
    /// the splash edge detect. Transient, never saved.
    pub was_in_water: bool,
    /// Wave 2c — world-space position of a Sign the player is editing. None =
    /// no sign editor showing. Set on right-click / on placement; cleared on
    /// Close / Esc / break. Mirrors `open_chest`.
    pub open_sign: Option<(i32, i32, i32)>,
    /// Living world Tier 2 S7 — world-space position of a Site Board the
    /// player has open. None = no board window showing. Set on right-click;
    /// cleared on Done / Esc / the block going.
    pub open_site_board: Option<(i32, i32, i32)>,
    /// Spec 33 Mob Bounty Board — world-space position of a
    /// BOUNTY_BOARD the player has open. None = no bounty UI is
    /// showing. Set on right-click; cleared on Close / Esc / break.
    /// Mirrors `open_furnace` / `open_vendor` / `open_chest`.
    pub open_bounty_board: Option<(i32, i32, i32)>,
    /// Spec 34 Tip Jar — world-space position of a TIP_JAR the player
    /// has open. None = no tip UI is showing. Mirrors `open_vendor`.
    pub open_tip_jar: Option<(i32, i32, i32)>,
    /// Spec 35 Repair Bench — world-space position of a REPAIR_BENCH
    /// the player has open. None = no repair UI is showing.
    pub open_repair_bench: Option<(i32, i32, i32)>,
    /// Spec 37 Market Hub — world-space position of a MARKET_BELL the
    /// player has open (directory panel). None = closed.
    pub open_market_hub: Option<(i32, i32, i32)>,
    /// Spec 38 Auction — world-space position of an AUCTION_BLOCK the
    /// player has open. None = closed.
    pub open_auction: Option<(i32, i32, i32)>,
    /// Spec 39 Bazaar — world-space position of a BAZAAR_BLOCK the
    /// player has open. None = closed.
    pub open_bazaar: Option<(i32, i32, i32)>,
    /// Spec 28f — Inventory Explorer overlay state. `Some` while open;
    /// holds the current search query + category filter. `None` when
    /// closed. B toggles; Esc / Close button clears.
    pub explorer_state: Option<crate::inventory_explorer::ExplorerState>,
    /// Spec 28d.nostrich — active "Nostrich's Vow" curse. `Some` while
    /// the player is cursed for killing or eating a Nostrich. Decays
    /// per tick; cleared when a memorial purifies it or it expires.
    /// Blocks vendor trade + zeroes village rep + suppresses sats.
    /// Not persisted on save (alpha-ephemeral; survives to next session
    /// would feel punishing if a v1-tester quits with one active).
    pub nostrich_vow: Option<crate::nostrich_vow::NostrichVow>,
    /// Spec 28e — equipped armour, indexed by `ArmourSlot as usize`
    /// (Helmet=0, Chestplate=1, Leggings=2, Boots=3). `None` slots
    /// contribute zero armour points; broken pieces (durability 0) are
    /// silently auto-unequipped to None by the damage path so they
    /// never linger as visual clutter.
    pub armour_slots: [Option<ArmourItem>; 4],
    /// The off-hand item (Minecraft's second hand). Any item can sit here; it
    /// is not slot 36 of `inventory` but rides beside the armour, like it
    /// (`hands.rs` holds the rules: which hand a right-click uses, the swap).
    /// Stays on the body at death, like armour (`death::split_window` never
    /// sees it). Saved as `WorldSave.player_offhands`. A joined client's copy
    /// on the server is `ServerPlayer.offhand` (saved in the joiner sidecar,
    /// v6; protocol 101).
    pub offhand: Option<ItemStack>,
    /// Off-hand release (slice 5) — consecutive ticks this body's shield has been
    /// raised (`shield::next_raised_ticks`); the shield blocks from
    /// `shield::RAISE_TICKS`. Runtime only.
    pub shield_ticks: u32,
    /// Wearing blocks since the game loop last played the clack: it drains this
    /// ([`Self::take_shield_clacks`]). Runtime only.
    pub shield_clacks: u8,
    /// Spec 26 — Builder villager entity whose Commission dialog is
    /// currently open for this player. `Some` while the dialog is
    /// visible (including during the Pick-build-site sub-flow, in which
    /// case the dialog itself is hidden but the entity stays bound so
    /// returning from Pick re-opens it). Transient — never persisted.
    pub open_commission_villager: Option<hecs::Entity>,
    /// Spec 26 — draft state for an in-progress commission dialog.
    /// Cleared on Confirm (commission moves onto the villager) or
    /// Cancel.
    pub pending_commission: Option<crate::builder::CommissionDraft>,
    /// The mount (a rideable mob or a rail cart) in THIS client's sim ECS
    /// this player is riding, or `None` when on foot. While `Some`, the
    /// tick steers the mount from this player's input and pins the body to
    /// the seat (`riding::steer` / `riding::follow`, MP-D2c-1), movement
    /// physics is skipped, and block break/place is gated ("you're on the
    /// train" — no acting at a distance). Cleared by the dismount rules
    /// (`riding::Dismount`). Transient — not persisted (a session that quits
    /// mid-ride reloads on foot). Always `None` for a joiner, whose mount is
    /// the server's ([`Self::mirror_mount`]).
    pub riding: Option<hecs::Entity>,
    /// The ride's own state (the Nostrich's wind-up speed and drifted
    /// heading). Transient; fresh on each mount.
    pub ride: crate::riding::RideState,
    /// MP-D2c-1 — a joiner's seat: the server's `ProtocolId` of the mount it
    /// rides (`StateUpdatePacket.own_riding`, server-driven, so a forced
    /// dismount reaches it). The body follows the mirrored mount at the
    /// seat and is not predicted; `None` on foot and always for a
    /// non-joined slot.
    pub mirror_mount: Option<u32>,
    /// #7 — WorldEdit region selection corners + clipboard. Transient (not
    /// persisted); single-player power-user editing.
    pub worldedit: crate::worldedit::WorldEditSession,
    /// W2 — where this player's last death left a grave, for the death
    /// screen's "Your items are in a grave at x, y, z" line. Cleared at each
    /// death, set when a grave is placed. Transient.
    pub last_grave: Option<[i32; 3]>,
}

impl PlayerSlot {
    /// Start the player at `pos`, at rest, and make it the respawn point: what a
    /// new world does with the spawn the search chose.
    pub fn start_at(&mut self, pos: Vec3) {
        self.player.pos = pos;
        self.player.velocity = Vec3::ZERO;
        self.player.reset_fall();
        self.spawn_pos = pos;
    }

    pub(crate) fn mark_gameplay_incomplete(&mut self) { self.gameplay_carry_incomplete = true; }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn gameplay_carry_complete(&self) -> bool { !self.gameplay_carry_incomplete }
    /// Only the real world-change boundary establishes a new live holder.
    pub(crate) fn reset_gameplay_holder(&mut self) {
        self.personal_binding = None;
        self.gameplay_carry_incomplete = false;
    }

    /// World handover (CP2 review M-A): a per-screen HUD dialog is open, any
    /// surface whose clicks change the world, this seat's items or its
    /// progression (containers, signs, trades, bounties, crafting, Plans,
    /// the inventory explorer, villager and guide dialogue).
    pub fn hud_dialog_open(&self) -> bool {
        self.crafting_ui.open
            || self.dialogue_villager.is_some()
            || self.dialogue_satoshi.is_some()
            || self.pending_build_choice.is_some()
            || self.open_commission_villager.is_some()
            || self.open_vendor.is_some()
            || self.shared_container.is_some()
            || self.open_furnace.is_some()
            || self.open_dispenser.is_some()
            || self.open_pack.is_some()
            || self.open_chest.is_some()
            || self.open_sign.is_some()
            || self.open_site_board.is_some()
            || self.open_bounty_board.is_some()
            || self.open_tip_jar.is_some()
            || self.open_repair_bench.is_some()
            || self.open_market_hub.is_some()
            || self.open_auction.is_some()
            || self.open_bazaar.is_some()
            || self.pending_plaque.is_some()
            || self.pending_bell.is_some()
            || self.pending_inspect.is_some()
            || self.pending_capture.is_some()
            || self.explorer_state.is_some()
    }

    /// World handover (M-A): close every HUD dialog that holds only a
    /// pointer at the world, as a source freezes (before its capture). The
    /// crafting screen stays open but gated (its grid and cursor items are
    /// part of the captured window; closing it would move them).
    pub fn close_hud_dialogs_for_freeze(&mut self) {
        self.dialogue_villager = None;
        self.dialogue_satoshi = None;
        self.pending_build_choice = None;
        self.open_commission_villager = None;
        self.open_vendor = None;
        self.open_furnace = None;
        self.open_dispenser = None;
        self.open_pack = None;
        self.open_chest = None;
        self.open_sign = None;
        self.open_site_board = None;
        self.open_bounty_board = None;
        self.open_tip_jar = None;
        self.open_repair_bench = None;
        self.open_market_hub = None;
        self.open_auction = None;
        self.open_bazaar = None;
        self.pending_plaque = None;
        self.pending_bell = None;
        self.pending_inspect = None;
        self.pending_capture = None;
        self.explorer_state = None;
    }

    /// On a mount or cart: one in this client's own sim ([`Self::riding`])
    /// or, joined, the server's ([`Self::mirror_mount`]).
    pub fn seated(&self) -> bool {
        self.riding.is_some() || self.mirror_mount.is_some()
    }

    pub fn new(index: usize, spawn: Vec3, aspect: f32) -> Self {
        Self {
            player: Player::new(spawn),
            camera: Camera::new(spawn, aspect),
            input: InputState::new(),
            inventory: Inventory::new_client(),
            combat: PlayerCombat::new(),
            hotbar_slot: 0,
            hotbar_scroll_ready_tick: 0,
            drop_ready_tick: 0,
            hotbar_scroll_accum: 0.0,
            crafting_ui: CraftingUi::new(),
            target_block: None,
            target_face: [0; 3],
            cursor_captured: false,
            index,
            breaking_pos: None,
            break_progress: 0,
            peel_latch: false,
            place_cooldown: 0,
            next_shot_tick: 0,
            eat_cooldown: 0,
            break_cooldown: 0,
            spawn_pos: spawn,
            fishing: None,
            friction_target: None,
            last_villager_warn_tick: None,
            dialogue_villager: None,
            dialogue_satoshi: None,
            villager_decline_until: ahash::AHashMap::new(),
            active_quests: ahash::AHashMap::new(),
            kill_counter: ahash::AHashMap::new(),
            personal_binding: None,
            quest_progress: Default::default(),
            gameplay_revision: 0,
            gameplay_clock: Default::default(),
            kill_tracking: crate::progression::KillTracking::Authoritative,
            gameplay_carry_incomplete: false,
            kill_quest_baseline: ahash::AHashMap::new(),
            charter_allows_sats: false,
            charter_comms: crate::comms::CommsLevel::Approved,
            bounties_claimed: ahash::AHashMap::new(),
            discovered_villages: ahash::AHashSet::new(),
            reputation: crate::reputation::Reputation::default(),
            last_villager_kill_rep_tick: None,
            pending_capture: None,
            last_chosen_license: None,
            pending_inspect: None,
            pending_plaque: None,
            pending_bell: None,
            ghost_state: None,
            pending_build_choice: None,
            open_furnace: None,
            open_vendor: None,
            open_chest: None,
            open_dispenser: None,
            shared_container: None,
            open_pack: None,
            was_in_water: false,
            open_sign: None,
            open_site_board: None,
            open_bounty_board: None,
            open_tip_jar: None,
            open_repair_bench: None,
            open_market_hub: None,
            open_auction: None,
            open_bazaar: None,
            explorer_state: None,
            nostrich_vow: None,
            armour_slots: [None, None, None, None],
            offhand: None,
            shield_ticks: 0,
            shield_clacks: 0,
            open_commission_villager: None,
            pending_commission: None,
            riding: None,
            ride: crate::riding::RideState::default(),
            mirror_mount: None,
            worldedit: crate::worldedit::WorldEditSession::default(),
            last_grave: None,
        }
    }

    /// Whether this player is dead.
    pub fn is_dead(&self) -> bool {
        self.combat.dead
    }

    /// Iterator over the four armour slots in canonical order
    /// (Helmet, Chestplate, Leggings, Boots). Pass directly into
    /// `armour::total_armour_points`.
    pub fn equipped_armour(&self) -> impl Iterator<Item = Option<&ArmourItem>> {
        self.armour_slots.iter().map(|opt| opt.as_ref())
    }

    /// Total non-broken armour points across all four slots.
    pub fn total_armour_points(&self) -> u8 {
        crate::armour::total_armour_points(self.equipped_armour())
    }

    /// Material of the currently-equipped Boots, if any (`None` = bare
    /// feet or a broken/unequipped piece — broken Boots are already
    /// dropped to `None` by `take_damage_with_armour_from`).
    pub fn equipped_boots_material(&self) -> Option<ArmourMaterial> {
        self.armour_slots[ArmourSlot::Boots as usize].map(|item| item.material)
    }

    /// Task 15 — refresh `player.sprint_boots_mult` from the equipped Boots
    /// before the physics tick. Call once per player per tick, right before
    /// `self.player.tick(...)` — mirrors how `mode`/`camera` are read fresh
    /// each tick rather than cached on `Player` itself (which has no armour
    /// concept of its own, only this derived scalar).
    pub fn refresh_sprint_boots_mult(&mut self) {
        self.player.sprint_boots_mult = crate::armour::sprint_multiplier(self.equipped_boots_material());
    }

    /// Apply raw incoming damage to this player's combat state after
    /// running it through the armour reduction formula, then decrement
    /// durability on every non-broken equipped piece by 1 (Minecraft
    /// per-piece wear). Pieces that hit 0 durability are silently
    /// unequipped so they don't keep contributing nor linger as visual
    /// clutter. Returns whether the hit landed (mirrors
    /// `PlayerCombat::take_damage`'s contract — false if the player
    /// was invulnerable or dead). Records `cause` for the death screen (W2)
    /// when the hit lands.
    pub fn take_damage_with_armour_from(
        &mut self,
        raw_damage: f32,
        cause: crate::survival::DamageCause,
    ) -> bool {
        // Dead → nothing to do. i-frames are PlayerCombat's call (MC rule: a
        // larger hit inside the window still applies its excess); armour only
        // wears when the hit actually lands, so a swarm of ignored hits
        // doesn't shred armour invisibly.
        if self.combat.dead {
            return false;
        }
        let total_points = self.total_armour_points();
        let reduced = crate::armour::damage_after_armour(raw_damage, total_points);
        let landed = self.combat.take_damage_from(reduced, cause);
        if landed {
            self.wear_armour();
        }
        landed
    }

    /// One landed hit's wear on the equipped armour: every worn piece loses
    /// one durability, and a piece that breaks is unequipped so it neither
    /// keeps contributing points nor lingers as a zero-durability ghost.
    /// The shared rule (`window::wear_armour`): the server wears its copy
    /// of a joiner's armour with it too (C3a-2a).
    pub fn wear_armour(&mut self) {
        crate::window::wear_armour(&mut self.armour_slots);
    }

    /// True when the creative break cooldown has elapsed, so a block may be
    /// broken this tick. Survival ignores this — it's paced by `break_time`
    /// (block hardness) instead.
    pub fn break_ready(&self) -> bool {
        self.break_cooldown == 0
    }

    /// Re-arm the creative break cooldown after committing a break this tick.
    pub fn arm_break_cooldown(&mut self) {
        self.break_cooldown = BLOCK_BREAK_COOLDOWN_TICKS;
    }

    /// Advance the break cooldown one tick, saturating at 0. Call once per game
    /// tick for every player, exactly like `place_cooldown`.
    pub fn tick_break_cooldown(&mut self) {
        self.break_cooldown = self.break_cooldown.saturating_sub(1);
    }

    /// Advance the eating cooldown one FIXED tick, saturating at 0 (not per
    /// frame: C2a verify M1).
    pub fn tick_eat_cooldown(&mut self) {
        self.eat_cooldown = self.eat_cooldown.saturating_sub(1);
    }

    /// A bite was taken (or, joined, asked for): arm the eating cooldown.
    /// Only PLACING and PLANTING wait on it ([`Self::biting`]; C3a-fix-2 D-L1):
    /// a door, chest, table, bed or mob responds right after a bite, so
    /// `place_cooldown` is left alone. Counted in FIXED ticks (C2b verify M2:
    /// in frames it was 0.27 s at 60 fps and 1.07 s at 15 fps).
    pub fn start_eat_cooldown(&mut self) {
        self.eat_cooldown = crate::item_actions::EAT_COOLDOWN_TICKS;
    }

    /// Is a bite's cooldown still running? The right-click chain's place and
    /// plant arms hold off while it is (a held right-click with food is
    /// swallowed first, `health_sync::eat_click`); every other arm answers.
    pub fn biting(&self) -> bool {
        self.eat_cooldown > 0
    }

    /// C3c-2-fix (M1) — may the bow or slingshot shoot on game tick `now`?
    /// A schedule more than one cooldown ahead is stale (the tick counter
    /// restarted with a new world) and doesn't hold a shot back.
    pub fn shot_ready(&self, now: u64) -> bool {
        now >= self.next_shot_tick || self.next_shot_tick - now > crate::shot::SHOT_COOLDOWN_TICKS
    }

    /// C3c-2-fix (M1) — a shot left (or, joined, was asked for) on game tick
    /// `now`: the next may go [`crate::shot::SHOT_COOLDOWN_TICKS`] later.
    pub fn arm_shot(&mut self, now: u64) {
        self.next_shot_tick = now + crate::shot::SHOT_COOLDOWN_TICKS;
    }

    /// Advance the placement cooldown one tick (frame), saturating at 0. Call
    /// once per game tick for every player, exactly like
    /// `tick_break_cooldown`. Ordinary placement pacing is frame-counted; the
    /// post-bite block is separate ([`Self::biting`]).
    pub fn tick_place_cooldown(&mut self) {
        self.place_cooldown = self.place_cooldown.saturating_sub(1);
    }

    // ── The two hands (`hands.rs`) ──────────────────────────────────────────

    /// The stack in `hand`: the selected hotbar slot, or the off-hand. The one
    /// accessor the right-click chain reads its held item through.
    pub fn held_for_use(&self, hand: crate::hands::Hand) -> Option<&ItemStack> {
        match hand {
            crate::hands::Hand::Main => self.inventory.hotbar_slot(self.hotbar_slot),
            crate::hands::Hand::Off => self.offhand.as_ref(),
        }
    }

    /// Off-hand release (slice 5) — this body's raised shield for the one block
    /// rule (`shield::blocks`): none unless it is alive and holds a shield in
    /// either hand. Whether the shield is *raised* is the count in
    /// `shield_ticks` (the rule wants [`crate::shield::RAISE_TICKS`]).
    pub fn shield_guard(&self) -> Option<crate::shield::Guard> {
        if self.combat.dead {
            return None;
        }
        let (main, off) = self.hands_items();
        crate::shield::guard_hand(main, off)?;
        Some(crate::shield::Guard {
            victim: self.player.pos,
            look: crate::shield::look_from_yaw(self.camera.yaw),
            raised_for: self.shield_ticks,
        })
    }

    /// The shield's side of landing hits on this body.
    pub fn shield_ctx(&self) -> crate::shield::ShieldCtx {
        crate::shield::ShieldCtx::with(self.shield_guard())
    }

    /// Settle what a shield blocked while hits were landed: wear the guarding
    /// shield (`shield::wear`) and queue the clacks.
    pub fn settle_shield(&mut self, tally: crate::shield::BlockTally) {
        if tally.wear > 0 {
            let (main, off) = self.hands_items();
            if let Some(hand) = crate::shield::guard_hand(main, off) {
                let slot = self.hotbar_slot;
                crate::shield::wear(&mut self.inventory, slot, &mut self.offhand, hand, tally.wear);
            }
        }
        self.shield_clacks = self.shield_clacks.saturating_add(tally.clacks);
    }

    /// Drain the clacks to play (one sound however many blocked this tick).
    pub fn take_shield_clacks(&mut self) -> u8 {
        std::mem::take(&mut self.shield_clacks)
    }

    /// A hit from `from` with `raw` damage before armour: stopped by a raised
    /// shield (no damage, no knockback), or landed through the armour. Returns
    /// whether it landed. `knock` is added to the velocity unless blocked.
    pub fn take_hit_from(
        &mut self,
        raw: f32,
        cause: crate::survival::DamageCause,
        from: glam::Vec3,
        knock: glam::Vec3,
    ) -> bool {
        let mut shield = self.shield_ctx();
        if !self.combat.dead && self.combat.shield_blocks(&mut shield, from, raw) {
            self.settle_shield(shield.tally);
            return false;
        }
        let landed = self.take_damage_with_armour_from(raw, cause);
        self.player.velocity += knock;
        landed
    }

    /// The main-hand item and the off-hand item, as [`crate::hands::use_hand`] reads them.
    pub fn hands_items(&self) -> (Option<&crate::item::Item>, Option<&crate::item::Item>) {
        (
            self.inventory.hotbar_slot(self.hotbar_slot).map(|s| &s.item),
            self.offhand.as_ref().map(|s| &s.item),
        )
    }

    /// What a joiner's request claims it holds in `hand` (v101): the selected
    /// hotbar slot (an off-hand use leaves the selection be), the item, and
    /// its wire forms (the `ItemRef` pair, then full fidelity).
    pub fn request_claim(&self, hand: crate::hands::Hand) -> (usize, Option<crate::item::Item>, u8, u16, crate::protocol::WireItem) {
        let held = self.held_for_use(hand).map(|s| s.item.clone());
        let (kind, id) = match &held {
            Some(item) => crate::inventory::item_to_ref(item).to_wire(),
            None => crate::protocol::ItemRef::Empty.to_wire(),
        };
        let full = held.as_ref().map_or(crate::protocol::WireItem::None, crate::inventory::item_to_wire_full);
        (self.hotbar_slot, held, kind, id, full)
    }

    /// An edit's hand on the input wire (v101, [`crate::protocol::EditHand`]):
    /// the selected hotbar slot, the `ItemRef` pair of the item in `hand`,
    /// and the hand itself.
    pub fn edit_hand(&self, hand: crate::hands::Hand) -> crate::protocol::EditHand {
        let (_, _, kind, id, _) = self.request_claim(hand);
        (self.hotbar_slot.min(usize::from(u8::MAX)) as u8, kind, id, hand)
    }

    /// [`Inventory::consume_one_material`] for `hand`.
    pub fn consume_one_material_for(&mut self, hand: crate::hands::Hand, expected: crate::item::MaterialId) -> bool {
        match hand {
            crate::hands::Hand::Main => self.inventory.consume_one_material(self.hotbar_slot, expected),
            crate::hands::Hand::Off => crate::inventory::consume_one_material_cell(&mut self.offhand, expected),
        }
    }

    /// [`Inventory::take_one_from_hotbar`] for `hand`.
    pub fn take_one_for(&mut self, hand: crate::hands::Hand) -> Option<ItemStack> {
        match hand {
            crate::hands::Hand::Main => self.inventory.take_one_from_hotbar(self.hotbar_slot),
            crate::hands::Hand::Off => crate::inventory::take_one_cell(&mut self.offhand),
        }
    }

    /// [`Inventory::try_eat_hotbar`] for `hand`.
    pub fn try_eat_for(&mut self, hand: crate::hands::Hand) -> Option<(f32, u32)> {
        match hand {
            crate::hands::Hand::Main => self.inventory.try_eat_hotbar(self.hotbar_slot),
            crate::hands::Hand::Off => crate::inventory::try_eat_cell(&mut self.offhand),
        }
    }

    /// [`Inventory::use_hotbar_tool`] for `hand`.
    pub fn use_tool_for(&mut self, hand: crate::hands::Hand) -> Option<crate::inventory::ToolUseInfo> {
        match hand {
            crate::hands::Hand::Main => self.inventory.use_hotbar_tool(self.hotbar_slot),
            crate::hands::Hand::Off => crate::inventory::use_tool_cell(&mut self.offhand),
        }
    }

    /// [`Inventory::take_placeable_from_hotbar`] for `hand` (the off-hand does
    /// not auto-refill from the bag).
    pub fn take_placeable_for(&mut self, hand: crate::hands::Hand) -> Option<crate::block::BlockId> {
        match hand {
            crate::hands::Hand::Main => self.inventory.take_placeable_from_hotbar(self.hotbar_slot),
            crate::hands::Hand::Off => crate::inventory::take_placeable_cell(&mut self.offhand).map(|(id, _)| id),
        }
    }

    /// [`Inventory::take_block_from_hotbar`] for `hand`: one of a held block
    /// (not a placeable material), its id.
    pub fn take_block_for(&mut self, hand: crate::hands::Hand) -> Option<crate::block::BlockId> {
        match hand {
            crate::hands::Hand::Main => self.inventory.take_block_from_hotbar(self.hotbar_slot),
            crate::hands::Hand::Off => {
                self.offhand.as_ref()?.item.as_block()?;
                crate::inventory::take_placeable_cell(&mut self.offhand).map(|(id, _)| id)
            }
        }
    }

    /// One round of ammo for a local shot: the off-hand first, then the bag
    /// (`shot::take_ammo`).
    pub fn take_shot_ammo(&mut self, weapon: crate::protocol::ShotWeapon) -> bool {
        crate::shot::take_ammo(&mut self.inventory, &mut self.offhand, weapon)
    }

    /// [`Inventory::hotbar_placeable_id`] for `hand`: what a click would place, without taking it.
    pub fn placeable_id_for(&self, hand: crate::hands::Hand) -> Option<crate::block::BlockId> {
        match self.held_for_use(hand)?.item {
            crate::item::Item::Block(b) => Some(b),
            crate::item::Item::Material(m) => crate::item::material_as_placeable_block(m),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Spec 28e — armour wiring on PlayerSlot. These exercise the
    //! pure helpers without touching combat callers.
    use super::*;
    use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot, max_durability};

    fn fresh_slot() -> PlayerSlot {
        PlayerSlot::new(0, Vec3::new(0.0, 64.0, 0.0), 1.0)
    }

    #[test]
    fn break_cooldown_paces_creative_breaking() {
        let mut slot = fresh_slot();
        assert!(slot.break_ready(), "a fresh player can break immediately");
        slot.arm_break_cooldown();
        assert!(!slot.break_ready(), "arming blocks the very next tick");
        // Tick down to one short of elapsed — still blocked.
        for _ in 0..(BLOCK_BREAK_COOLDOWN_TICKS - 1) {
            slot.tick_break_cooldown();
            assert!(!slot.break_ready(), "still on cooldown before it fully elapses");
        }
        // The final tick clears it.
        slot.tick_break_cooldown();
        assert!(slot.break_ready(), "ready again once the full cooldown elapses");
    }

    #[test]
    fn ticking_break_cooldown_at_zero_does_not_underflow() {
        let mut slot = fresh_slot();
        slot.tick_break_cooldown(); // already 0 — must saturate, not wrap
        assert!(slot.break_ready());
        assert_eq!(slot.break_cooldown, 0);
    }

    #[test]
    fn place_cooldown_reaches_zero_after_exactly_the_configured_ticks() {
        let mut slot = fresh_slot();
        slot.place_cooldown = PLACE_COOLDOWN_TICKS;
        for _ in 0..(PLACE_COOLDOWN_TICKS - 1) {
            slot.tick_place_cooldown();
            assert!(slot.place_cooldown > 0, "still on cooldown before it fully elapses");
        }
        slot.tick_place_cooldown();
        assert_eq!(slot.place_cooldown, 0, "cooldown of 4 should reach 0 after exactly 4 ticks");
    }

    /// C2b verify M2 + C3a-fix-2 D-L1 — the block on placing after a bite
    /// lasts `EAT_COOLDOWN_TICKS` FIXED ticks, however many frames pass
    /// meanwhile, and it blocks only placing: the placement cooldown that
    /// gates doors, chests, tables, beds and mobs is untouched.
    #[test]
    fn the_post_bite_placement_block_is_counted_in_ticks_not_frames() {
        let mut slot = fresh_slot();
        slot.start_eat_cooldown();
        assert!(slot.biting(), "placing is blocked at once");
        assert_eq!(slot.place_cooldown, 0, "D-L1: doors, chests and mobs still answer");
        // 100 frames pass but no fixed tick: still blocked.
        for _ in 0..100 {
            slot.tick_place_cooldown();
        }
        assert!(slot.biting(), "frames alone never end it");
        assert_eq!(slot.place_cooldown, 0);
        // The fixed ticks run it out.
        for _ in 0..crate::item_actions::EAT_COOLDOWN_TICKS {
            assert!(slot.biting());
            slot.tick_eat_cooldown();
        }
        assert!(!slot.biting());
    }

    #[test]
    fn ticking_place_cooldown_at_zero_does_not_underflow() {
        let mut slot = fresh_slot();
        slot.tick_place_cooldown(); // already 0 — must saturate, not wrap
        assert_eq!(slot.place_cooldown, 0);
    }

    #[test]
    fn new_slot_starts_with_no_armour() {
        let slot = fresh_slot();
        assert!(slot.armour_slots.iter().all(|a| a.is_none()));
        assert_eq!(slot.total_armour_points(), 0);
    }

    #[test]
    fn refresh_sprint_boots_mult_reflects_equipped_boots() {
        // Task 15 — the seam between armour and physics: PlayerSlot reads
        // its own Boots slot and stamps the derived multiplier onto Player
        // right before the physics tick would consume it.
        let mut slot = fresh_slot();
        slot.refresh_sprint_boots_mult();
        assert_eq!(slot.player.sprint_boots_mult, 1.0, "bare feet: no bonus");

        slot.armour_slots[ArmourSlot::Boots as usize] =
            Some(ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Rubber));
        slot.refresh_sprint_boots_mult();
        assert_eq!(slot.player.sprint_boots_mult, crate::armour::sprint_multiplier(Some(ArmourMaterial::Rubber)));

        // Unequipping drops it back to 1.0 (not sticky from a prior tick).
        slot.armour_slots[ArmourSlot::Boots as usize] = None;
        slot.refresh_sprint_boots_mult();
        assert_eq!(slot.player.sprint_boots_mult, 1.0, "unequipping boots removes the bonus");
    }

    #[test]
    fn total_armour_points_sums_equipped_pieces() {
        let mut slot = fresh_slot();
        slot.armour_slots[ArmourSlot::Helmet as usize] =
            Some(ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron));
        slot.armour_slots[ArmourSlot::Chestplate as usize] =
            Some(ArmourItem::new(ArmourSlot::Chestplate, ArmourMaterial::Iron));
        // 2 + 6 = 8 points.
        assert_eq!(slot.total_armour_points(), 8);
    }

    #[test]
    fn take_damage_with_armour_reduces_by_40_percent_in_full_iron() {
        let mut slot = fresh_slot();
        for s in [
            ArmourSlot::Helmet, ArmourSlot::Chestplate,
            ArmourSlot::Leggings, ArmourSlot::Boots,
        ] {
            slot.armour_slots[s as usize] = Some(ArmourItem::new(s, ArmourMaterial::Iron));
        }
        let start = slot.combat.health;
        slot.take_damage_with_armour_from(10.0, crate::survival::DamageCause::Generic);
        // 15 points → 60% reduction → 4 hp lands.
        assert!((slot.combat.health - (start - 4.0)).abs() < 1e-3);
    }

    #[test]
    fn take_damage_with_armour_wears_each_piece_by_one() {
        let mut slot = fresh_slot();
        for s in [
            ArmourSlot::Helmet, ArmourSlot::Chestplate,
            ArmourSlot::Leggings, ArmourSlot::Boots,
        ] {
            slot.armour_slots[s as usize] = Some(ArmourItem::new(s, ArmourMaterial::Iron));
        }
        slot.take_damage_with_armour_from(5.0, crate::survival::DamageCause::Generic);
        for s in [
            ArmourSlot::Helmet, ArmourSlot::Chestplate,
            ArmourSlot::Leggings, ArmourSlot::Boots,
        ] {
            let p = slot.armour_slots[s as usize].as_ref().unwrap();
            assert_eq!(p.durability, max_durability(s, ArmourMaterial::Iron) - 1);
        }
    }

    #[test]
    fn take_damage_with_armour_dead_is_no_op() {
        let mut slot = fresh_slot();
        slot.combat.dead = true;
        slot.armour_slots[ArmourSlot::Helmet as usize] =
            Some(ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron));
        let helmet_dur_before = slot.armour_slots[ArmourSlot::Helmet as usize].unwrap().durability;
        let landed = slot.take_damage_with_armour_from(5.0, crate::survival::DamageCause::Generic);
        assert!(!landed);
        // No durability wear on a non-landed hit (parity with combat
        // i-frames so swarms can't shred armour invisibly).
        assert_eq!(
            slot.armour_slots[ArmourSlot::Helmet as usize].unwrap().durability,
            helmet_dur_before,
        );
    }

    /// C3c-2-fix (M1) — the shot cadence is counted in game ticks: a button
    /// held at 60 or at 144 frames a second fires every
    /// `SHOT_COOLDOWN_TICKS` ticks (2.5 a second), where the frame-counted
    /// `place_cooldown` fired 7.5 or 18 a second. A stale schedule (a new
    /// world's tick counter) holds nothing back.
    #[test]
    fn a_held_shot_fires_on_the_tick_cadence_at_any_frame_rate() {
        for fps in [60u32, 144] {
            let mut slot = fresh_slot();
            let (mut tick, mut acc, mut shots) = (0u64, 0u32, 0u32);
            // 20 seconds of frames; ticks run off a 20 TPS accumulator (in
            // 1/(20·fps) s units) before each frame's click, as the game loop's.
            for _ in 0..(20 * fps) {
                acc += 20;
                while acc >= fps {
                    acc -= fps;
                    tick += 1;
                }
                if slot.shot_ready(tick) {
                    slot.arm_shot(tick);
                    shots += 1;
                }
            }
            // A shot on tick 0 and every cooldown after, to the last tick.
            assert_eq!(u64::from(shots), tick / crate::shot::SHOT_COOLDOWN_TICKS + 1, "{fps} fps");
            assert_eq!((tick, shots), (400, 51), "{fps} fps: 2.5 a second over 20 s");
        }
        let mut slot = fresh_slot();
        slot.arm_shot(50_000);
        assert!(!slot.shot_ready(50_004));
        assert!(slot.shot_ready(3), "a restarted tick counter: the old schedule is stale");
    }

    // ── The two hands (`hands.rs`): the routing every right-click arm uses ──

    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::hands::{use_hand, Hand, UseTarget};
    use crate::item::MaterialId;

    /// The hand a click aimed at `target` uses, from this slot's real items.
    fn hand_for(slot: &PlayerSlot, target: &UseTarget) -> Hand {
        let (main, off) = slot.hands_items();
        use_hand(main, off, target)
    }

    fn aimed_at_stone() -> UseTarget {
        UseTarget { block: Some(crate::block::STONE), ..UseTarget::default() }
    }

    #[test]
    fn a_pickaxe_in_hand_places_the_off_hand_torch_from_the_off_hand() {
        let mut slot = fresh_slot();
        let pick = ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron));
        slot.inventory.set_slot(slot.hotbar_slot, Some(pick.clone()));
        slot.offhand = Some(ItemStack::new_block(crate::block::TORCH, 5));

        let hand = hand_for(&slot, &aimed_at_stone());
        assert_eq!(hand, Hand::Off, "the pickaxe has no use here: the off-hand's torch does");
        assert_eq!(slot.held_for_use(hand).map(|s| &s.item), Some(&crate::item::Item::Block(crate::block::TORCH)));
        assert_eq!(slot.placeable_id_for(hand), Some(crate::block::TORCH));
        assert_eq!(slot.take_placeable_for(hand), Some(crate::block::TORCH));
        assert_eq!(slot.offhand, Some(ItemStack::new_block(crate::block::TORCH, 4)), "taken from the off-hand");
        assert_eq!(slot.inventory.hotbar_slot(slot.hotbar_slot), Some(&pick), "the main hand is untouched");
        // The last torch empties the off-hand and nothing refills it from the bag.
        slot.inventory.set_slot(20, Some(ItemStack::new_block(crate::block::TORCH, 9)));
        for _ in 0..4 {
            assert_eq!(slot.take_placeable_for(Hand::Off), Some(crate::block::TORCH));
        }
        assert_eq!(slot.offhand, None);
        assert_eq!(slot.inventory.slot(20), Some(&ItemStack::new_block(crate::block::TORCH, 9)));
        assert_eq!(slot.take_placeable_for(Hand::Off), None);
    }

    #[test]
    fn food_in_the_main_hand_eats_from_the_main_hand() {
        let mut slot = fresh_slot();
        slot.inventory.set_slot(slot.hotbar_slot, Some(ItemStack::new_material(MaterialId::Bread, 3)));
        slot.offhand = Some(ItemStack::new_material(MaterialId::Bread, 7));
        let hungry = UseTarget { wants_to_eat: true, ..aimed_at_stone() };

        let hand = hand_for(&slot, &hungry);
        assert_eq!(hand, Hand::Main);
        assert!(slot.try_eat_for(hand).is_some());
        assert_eq!(slot.inventory.hotbar_slot(slot.hotbar_slot).map(|s| s.count), Some(2), "one bite from the main stack");
        assert_eq!(slot.offhand.as_ref().map(|s| s.count), Some(7), "the off-hand is untouched");
    }

    #[test]
    fn food_in_the_off_hand_is_eaten_when_the_main_hand_has_no_use() {
        let mut slot = fresh_slot();
        slot.inventory.set_slot(slot.hotbar_slot, Some(ItemStack::new_tool(Tool::new(ToolType::Sword, ToolMaterial::Iron))));
        slot.offhand = Some(ItemStack::new_material(MaterialId::Bread, 2));
        let hungry = UseTarget { wants_to_eat: true, ..UseTarget::default() };

        let hand = hand_for(&slot, &hungry);
        assert_eq!(hand, Hand::Off);
        assert!(slot.try_eat_for(hand).is_some());
        assert_eq!(slot.offhand.as_ref().map(|s| s.count), Some(1));
        assert!(slot.try_eat_for(hand).is_some());
        assert_eq!(slot.offhand, None, "the last bite empties the off-hand");
        assert!(slot.inventory.hotbar_slot(slot.hotbar_slot).is_some(), "the sword stays");
    }

    #[test]
    fn a_block_in_the_main_hand_places_from_the_main_hand() {
        let mut slot = fresh_slot();
        slot.inventory.set_slot(slot.hotbar_slot, Some(ItemStack::new_block(crate::block::STONE, 9)));
        slot.offhand = Some(ItemStack::new_block(crate::block::TORCH, 5));

        let hand = hand_for(&slot, &aimed_at_stone());
        assert_eq!(hand, Hand::Main);
        assert_eq!(slot.take_placeable_for(hand), Some(crate::block::STONE));
        assert_eq!(slot.inventory.hotbar_slot(slot.hotbar_slot).map(|s| s.count), Some(8));
        assert_eq!(slot.offhand, Some(ItemStack::new_block(crate::block::TORCH, 5)), "the torches stay");
    }

    /// Review 1 finding 3, as the click plays out: a hoe in the main hand and a
    /// torch in the off-hand, aimed at stone, places the torch from the off-hand;
    /// aimed at the top of grass the hoe keeps the click (it tills).
    #[test]
    fn a_hoe_on_stone_places_the_off_hand_torch_and_on_grass_keeps_the_click() {
        let mut slot = fresh_slot();
        slot.inventory.set_slot(slot.hotbar_slot, Some(ItemStack::new_tool(Tool::new(ToolType::Hoe, ToolMaterial::Iron))));
        slot.offhand = Some(ItemStack::new_block(crate::block::TORCH, 5));
        let grass_top = UseTarget { block: Some(crate::block::GRASS), face: [0, 1, 0], ..UseTarget::default() };
        assert_eq!(hand_for(&slot, &grass_top), Hand::Main, "the hoe tills grass");
        let hand = hand_for(&slot, &aimed_at_stone());
        assert_eq!(hand, Hand::Off, "a hoe has no use on stone");
        assert_eq!(slot.take_placeable_for(hand), Some(crate::block::TORCH), "the torch is placed");
        assert_eq!(slot.offhand, Some(ItemStack::new_block(crate::block::TORCH, 4)));
        assert!(slot.inventory.hotbar_slot(slot.hotbar_slot).is_some(), "the hoe stays");
    }

    #[test]
    fn a_tool_in_the_off_hand_wears_there_and_breaks_there() {
        let mut slot = fresh_slot();
        slot.offhand = Some(ItemStack::new_tool(Tool::new(ToolType::Shears, ToolMaterial::Iron)));
        let first = slot.use_tool_for(Hand::Off).expect("a tool wears");
        assert!(first.after_pct < first.before_pct);
        let mut worn = Tool::new(ToolType::Shears, ToolMaterial::Iron);
        worn.durability = 1;
        slot.offhand = Some(ItemStack::new_tool(worn));
        assert!(slot.use_tool_for(Hand::Off).expect("it wears").just_broke);
        assert_eq!(slot.offhand, None, "a broken off-hand tool is gone");
    }

    /// F / a tap on the off-hand box, as the game does it: the window rule
    /// ([`crate::window::WindowClick::SwapHands`], `window_actions`).
    fn swap_hands(slot: &mut PlayerSlot) -> bool {
        let sel = slot.hotbar_slot;
        let ctx = crate::window::ClickCtx::new(false, crate::window::Station::Player, glam::Vec3::ZERO, |_| crate::block::AIR);
        let mut view = crate::window::WindowMut {
            inv: &mut slot.inventory,
            armour: &mut slot.armour_slots,
            offhand: &mut slot.offhand,
            cursor: &mut slot.crafting_ui.cursor_item,
            grid: &mut slot.crafting_ui.grid,
            container: None,
        };
        crate::window::apply(&mut view, &crate::window::WindowClick::SwapHands { slot: sel }, &ctx).ok()
    }

    /// v101 — what a joined client stamps on an edit and claims in a request:
    /// the off-hand's item with `Hand::Off`, the selection untouched, the
    /// pickaxe in hand named only when the hand is the main one.
    #[test]
    fn the_client_stamps_the_hand_and_claims_the_item_of_that_hand() {
        use crate::hands::Hand;
        let mut slot = fresh_slot();
        let sel = slot.hotbar_slot;
        let pick = ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron));
        slot.inventory.set_slot(sel, Some(pick.clone()));
        slot.offhand = Some(ItemStack::new_block(crate::block::TORCH, 5));
        let torch_ref = crate::inventory::item_to_ref(&crate::item::Item::Block(crate::block::TORCH)).to_wire();
        let pick_ref = crate::inventory::item_to_ref(&pick.item).to_wire();
        assert_eq!(slot.edit_hand(Hand::Off), (sel as u8, torch_ref.0, torch_ref.1, Hand::Off));
        assert_eq!(slot.edit_hand(Hand::Main), (sel as u8, pick_ref.0, pick_ref.1, Hand::Main));
        let (hot, held, kind, id, full) = slot.request_claim(Hand::Off);
        assert_eq!((hot, kind, id), (sel, torch_ref.0, torch_ref.1), "the selection is left be");
        assert_eq!(held, Some(crate::item::Item::Block(crate::block::TORCH)));
        assert_eq!(full, crate::inventory::item_to_wire_full(&crate::item::Item::Block(crate::block::TORCH)));
        // An empty off-hand claims nothing.
        slot.offhand = None;
        let (_, held, kind, id, full) = slot.request_claim(Hand::Off);
        assert_eq!((held, (kind, id), full), (None, crate::protocol::ItemRef::Empty.to_wire(), crate::protocol::WireItem::None));
        // After F the claims follow the items.
        assert!(swap_hands(&mut slot));
        assert_eq!(slot.request_claim(Hand::Off).1, Some(pick.item.clone()));
        assert_eq!(slot.edit_hand(Hand::Off).3, Hand::Off);
    }

    #[test]
    fn swapping_hands_trades_the_selected_slot_and_the_off_hand() {
        let mut slot = fresh_slot();
        let sel = slot.hotbar_slot;
        slot.inventory.set_slot(sel, Some(ItemStack::new_block(crate::block::STONE, 9)));
        slot.offhand = Some(ItemStack::new_block(crate::block::TORCH, 5));
        assert!(swap_hands(&mut slot));
        assert_eq!(slot.inventory.hotbar_slot(sel), Some(&ItemStack::new_block(crate::block::TORCH, 5)));
        assert_eq!(slot.offhand, Some(ItemStack::new_block(crate::block::STONE, 9)));
        assert!(swap_hands(&mut slot));
        assert_eq!(slot.inventory.hotbar_slot(sel), Some(&ItemStack::new_block(crate::block::STONE, 9)));
        assert_eq!(slot.offhand, Some(ItemStack::new_block(crate::block::TORCH, 5)));
        // An empty hand swaps in the off-hand item; two empty hands swap nothing.
        slot.inventory.set_slot(sel, None);
        assert!(swap_hands(&mut slot));
        assert_eq!(slot.inventory.hotbar_slot(sel), Some(&ItemStack::new_block(crate::block::TORCH, 5)));
        assert_eq!(slot.offhand, None);
        slot.inventory.set_slot(sel, None);
        assert!(!swap_hands(&mut slot));
    }
}

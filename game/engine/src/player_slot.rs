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
    pub place_cooldown: u32,
    /// Break cooldown (ticks remaining until the next *creative* block break is
    /// allowed). Survival breaking is already paced by `break_time` (block
    /// hardness via `breaking_pos`/`break_progress`), but creative breaks are
    /// instant, so without this a held button smashes one block every tick
    /// (~20/s). Mirrors `place_cooldown`. Transient (not saved). Spec 05.
    pub break_cooldown: u32,
    /// Where the player respawns on death. Initialised from the world spawn
    /// point at PlayerSlot creation; bed-settable (P4) by sleeping in a bed.
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
    /// Rail freight (Phase 1, Task 1.5) — the cart entity this player is
    /// currently RIDING, or `None` when on foot. While `Some`, the per-tick
    /// follow in `game_loop` pins the player's eye to the cart's seat
    /// (`cart::rider_eye_position`), normal movement physics + input are
    /// suppressed (the cart drives the player), and block break/place is gated
    /// ("you're on the train" — no acting at a distance). Cleared on arrival
    /// (cart parks), on a jump input (hop off), or defensively if the cart
    /// entity vanishes. Transient — not persisted (a session that quits
    /// mid-ride reloads on foot).
    pub riding: Option<hecs::Entity>,
    /// Nostrich ride physics state (see `nostrich_ride`): current speed (b/s) and
    /// drifted heading `(x, z)`. Transient; reset to `0` on mount + dismount. Only
    /// meaningful while `riding` a Nostrich.
    pub ride_speed: f32,
    pub ride_heading: glam::Vec2,
    /// #7 — WorldEdit region selection corners + clipboard. Transient (not
    /// persisted); single-player power-user editing.
    pub worldedit: crate::worldedit::WorldEditSession,
}

impl PlayerSlot {
    pub fn new(index: usize, spawn: Vec3, aspect: f32) -> Self {
        Self {
            player: Player::new(spawn),
            camera: Camera::new(spawn, aspect),
            input: InputState::new(),
            inventory: Inventory::new(),
            combat: PlayerCombat::new(),
            hotbar_slot: 0,
            hotbar_scroll_ready_tick: 0,
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
            open_pack: None,
            was_in_water: false,
            open_sign: None,
            open_bounty_board: None,
            open_tip_jar: None,
            open_repair_bench: None,
            open_market_hub: None,
            open_auction: None,
            open_bazaar: None,
            explorer_state: None,
            nostrich_vow: None,
            armour_slots: [None, None, None, None],
            open_commission_villager: None,
            pending_commission: None,
            riding: None,
            ride_speed: 0.0,
            ride_heading: glam::Vec2::ZERO,
            worldedit: crate::worldedit::WorldEditSession::default(),
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
    /// dropped to `None` by `take_damage_with_armour`).
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
    /// was invulnerable or dead).
    pub fn take_damage_with_armour(&mut self, raw_damage: f32) -> bool {
        // i-frames / dead → don't wear armour either. PlayerCombat is
        // the authority on whether the hit "happened" for the player;
        // this mirrors that gate so a swarm of hits during i-frames
        // doesn't shred armour invisibly.
        if self.combat.invincible_timer > 0 || self.combat.dead {
            return false;
        }
        let total_points = self.total_armour_points();
        let reduced = crate::armour::damage_after_armour(raw_damage, total_points);
        let landed = self.combat.take_damage(reduced);
        if landed {
            for slot in self.armour_slots.iter_mut() {
                if let Some(piece) = slot.as_mut()
                    && !piece.is_broken() {
                        piece.durability = piece.durability.saturating_sub(1);
                    }
                // Drop the piece entirely when it breaks so total
                // points stays accurate and the UI doesn't show a
                // zero-durability ghost.
                if slot.as_ref().is_some_and(|p| p.is_broken()) {
                    *slot = None;
                }
            }
        }
        landed
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

    /// Advance the placement cooldown one tick, saturating at 0. Call once
    /// per game tick for every player, exactly like `tick_break_cooldown`.
    pub fn tick_place_cooldown(&mut self) {
        self.place_cooldown = self.place_cooldown.saturating_sub(1);
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
        slot.take_damage_with_armour(10.0);
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
        slot.take_damage_with_armour(5.0);
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
        let landed = slot.take_damage_with_armour(5.0);
        assert!(!landed);
        // No durability wear on a non-landed hit (parity with combat
        // i-frames so swarms can't shred armour invisibly).
        assert_eq!(
            slot.armour_slots[ArmourSlot::Helmet as usize].unwrap().durability,
            helmet_dur_before,
        );
    }
}

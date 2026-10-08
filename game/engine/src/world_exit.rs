//! Leaving a world — the ONE teardown every exit path goes through.
//!
//! Before this module each way out of a world (Save & Quit, Quit without
//! saving, the Trial "Leave" button, the end cards, the skin-paint hop to the
//! Workshop, the window-close button) did its own subset of the work, and most
//! of them forgot the network: the hosted server, the joined client and online
//! hosting kept running into the next world (audit 2026-09-27, P3). They also
//! disagreed about saving: the close button saved whatever `self.world` held in
//! any mode, and a joined session saved the host's world as a local
//! "remote_game". [`GameState::leave_world`] is now the single exit, and the
//! decisions it makes are the small pure functions below, so they unit-test
//! without a window or a GPU.

use crate::commands::OpLevel;
use crate::scenario::ScenarioDef;
use crate::world::World;

/// What the player asked to happen to the world they are leaving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveChoice {
    /// Write the world (Save & Quit, Trial Leave, end cards, window close).
    Save,
    /// The player chose to throw the session away ("Quit without saving").
    /// Writes nothing and drops the crash-recovery autosave with it.
    Discard,
    /// Write nothing and touch nothing on disk: the window closing with no
    /// savable world live, or a session ending that the player didn't choose.
    /// The crash-recovery autosave stays for the next load to recover.
    Abandon,
}

/// Where the player lands after [`GameState::leave_world`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExitTo {
    /// Back to the lobby: Leave Trial, the end cards, the J-board arena hop,
    /// the skin-paint hop, a dropped connection. In a showcase kiosk this is
    /// the showcase start (its lobby), never the terminal exit screen.
    Lobby,
    /// The player is done: the pause menu's quit buttons and the window
    /// close. A showcase kiosk dead-ends here (spec 2026-06-19 §10).
    Quit,
}

/// Does this exit end on the showcase kiosk's terminal exit screen? Only an
/// explicit quit does; the in-world hops (J board → arena, end cards) keep
/// the visitor inside the kiosk (review W3 S4).
pub(crate) fn exit_dead_ends(to: ExitTo, showcase_dead_end: bool) -> bool {
    to == ExitTo::Quit && showcase_dead_end
}

/// What kind of world is live. Recorded once, at the Loading → Playing
/// hand-off, and cleared by every exit — so "no live world" (the lobby, the
/// splash, the first Loading frame, after Quit without saving) can never be
/// mistaken for a world to save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorldKind {
    /// The player's own world, saved on this machine.
    Local,
    /// A Trial / Experience arena (the Stash Column launch path).
    Arena,
    /// Someone else's world, joined over the network. Never saved here.
    Joined,
}

/// Classify the world that just finished loading.
pub(crate) fn classify_world(joined: bool, arena: bool) -> WorldKind {
    if joined {
        WorldKind::Joined
    } else if arena {
        WorldKind::Arena
    } else {
        WorldKind::Local
    }
}

/// Does closing the window save? Only while the player is actually in (or
/// paused over) a fully loaded world this machine owns: their own world or a
/// Trial arena (Satori Rush is `Resume`, and `KeepNew` arenas are kept as the
/// player's own worlds — review W3 B1). Never from the lobby, never mid-load,
/// never after "Quit without saving", never a joined session.
pub(crate) fn should_save_on_close(in_world: bool, live: Option<WorldKind>) -> bool {
    in_world && matches!(live, Some(WorldKind::Local | WorldKind::Arena))
}

/// What the window close hands to [`GameState::leave_world`]: a save when
/// there is a world to save, otherwise [`SaveChoice::Abandon`] — a close
/// never throws away the crash-recovery autosave. Every close saves, the one
/// after a failed close-save too: it retries once ([`after_close_save`]).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the web has no close-save
pub(crate) fn close_choice(in_world: bool, live: Option<WorldKind>) -> SaveChoice {
    if should_save_on_close(in_world, live) {
        SaveChoice::Save
    } else {
        SaveChoice::Abandon
    }
}

/// What a window close does once [`GameState::leave_world`] has run its
/// [`close_choice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the web has no close-save
pub(crate) enum CloseOutcome {
    /// The player left (the save landed, or there was nothing to save): quit.
    Quit,
    /// The close's save failed: stay in the world, with the toast saying why and
    /// what closing again does ([`close_again_hint`]).
    StayToRetry,
    /// The save failed on the close after a failed close too — it was retried
    /// once: quit anyway, by a [`SaveChoice::Abandon`] that keeps the autosave.
    QuitKeepingAutosave,
}

/// [`CloseOutcome`] for a close that left (`left`) or not, given whether the
/// previous close's save had failed with no save landing since
/// (`SessionSaves::close_save_failed`). A save that keeps failing never traps
/// the player in the world with "Quit without saving" as the only way out
/// (review 2026-10-06), and a close never quits without first trying to save
/// (third review, 2026-10-06: the flag used to make every later close quit
/// without even trying, however much was played since).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the web has no close-save
pub(crate) fn after_close_save(left: bool, close_save_failed: bool) -> CloseOutcome {
    match (left, close_save_failed) {
        (true, _) => CloseOutcome::Quit,
        (false, false) => CloseOutcome::StayToRetry,
        (false, true) => CloseOutcome::QuitKeepingAutosave,
    }
}

/// What this session's saves have left the crash-recovery autosave guarding.
/// Cleared on every world entry and exit, and by any save that lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SessionSaves {
    /// The last save this session tried failed, so the autosave may be the
    /// newest copy of the session there is.
    pub(crate) save_failed: bool,
    /// The world opened from its crash-recovery autosave and no save has
    /// landed since: with a damaged `world.dat` it is the only good copy.
    pub(crate) opened_from_autosave: bool,
    /// A window close tried to save and failed: the next close tries once more
    /// and quits either way, keeping the autosave if that fails too
    /// ([`after_close_save`]). Cleared by any save that lands.
    pub(crate) close_save_failed: bool,
}

impl SessionSaves {
    /// The state a world opens in.
    pub(crate) fn opened(from_autosave: bool) -> Self {
        Self { opened_from_autosave: from_autosave, ..Self::default() }
    }

    /// Record a save of the live world: one that landed supersedes the autosave
    /// (and every reason to guard it); one that failed makes it precious.
    pub(crate) fn note_save(&mut self, landed: bool) {
        if landed {
            *self = Self::default();
        } else {
            self.save_failed = true;
        }
    }

    /// Must "Quit without saving" keep the autosave? Yes after a failed save
    /// (it may hold the session's only copy) or an autosave open (it may be the
    /// world's only good copy).
    pub(crate) fn keeps_autosave(self) -> bool {
        self.save_failed || self.opened_from_autosave
    }
}

/// Is the crash-recovery autosave dropped on leave? Only when it is
/// superseded or unwanted: after a save that actually landed (the loader
/// prefers an autosave, so a stale one would roll the fresh save back), or
/// when the player explicitly chose "Quit without saving" and nothing makes
/// it precious (`keep_autosave`, [`SessionSaves::keeps_autosave`]): after a
/// failed save, or in a session opened from the autosave, a discard keeps it
/// like [`SaveChoice::Abandon`] does (review 2026-10-06 — a save that kept
/// failing funnelled the player into deleting it). Never after a failed save,
/// never on [`SaveChoice::Abandon`] (window close with nothing saved, a dropped
/// connection), never for a joined session (it has none).
pub(crate) fn should_clear_autosave(
    choice: SaveChoice,
    saved: bool,
    live: Option<WorldKind>,
    joined_now: bool,
    keep_autosave: bool,
) -> bool {
    if joined_now || !matches!(live, Some(WorldKind::Local | WorldKind::Arena)) {
        return false;
    }
    match choice {
        SaveChoice::Save => saved,
        SaveChoice::Discard => !keep_autosave,
        SaveChoice::Abandon => false,
    }
}

/// The pause menu's "Quit without saving" button. When that quit keeps the
/// autosave ([`SessionSaves::keeps_autosave`], and one is there — `kept` is its
/// age, e.g. "2 minutes ago"), it says so instead of promising a discard.
pub(crate) fn quit_no_save_label(kept: Option<&str>) -> String {
    match kept {
        Some(age) => format!("Quit — your autosave from {age} is kept"),
        None => "Quit Without Saving".to_string(),
    }
}

/// The "are you sure?" line under that button.
pub(crate) fn quit_no_save_warning(kept: Option<&str>) -> String {
    match kept {
        Some(age) => format!("Anything since your autosave from {age} will be lost."),
        None => "Unsaved progress will be lost!".to_string(),
    }
}

/// Appended to the save-failed toast when a window close's save failed: what
/// closing again does ([`after_close_save`]).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the web has no close-save
pub(crate) fn close_again_hint(kept: Option<&str>) -> String {
    let then = match kept {
        Some(age) => format!("your autosave from {age} is kept."),
        None => "the game quits without saving.".to_string(),
    };
    format!(" Close the window again to try once more and quit — if the save fails again, {then}")
}

/// What the player is told when a save fails — Save, Save & Quit, or any exit
/// that saves. The world on disk is whatever the last save that landed wrote
/// (every writer refuses before it touches anything, or writes tmp + rename), so
/// that copy is safe; the session itself is still in memory, and a failed Save &
/// Quit keeps the player in it to retry (review 2026-10-06: it used to only log).
pub(crate) fn save_failed_toast(why: &str) -> String {
    format!("Couldn't save: {why}. Your last save is safe.")
}

/// How long the save-failure toast stays up.
pub(crate) const SAVE_FAILED_TOAST_SECS: u64 = 10;

/// How long an in-world arena launch waits for the web lobby's world list
/// before it gives up and says so (review W3 S6).
pub(crate) const QUEUED_LAUNCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// What the lobby does with a queued in-world arena launch this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueuedLaunch {
    /// Fire it now, as if the Trials menu's Play had been clicked.
    Fire,
    /// Keep it: the web lobby hasn't listed its worlds yet.
    Wait,
    /// Drop it silently: the player did something else in the lobby first,
    /// so it must never fire into a later, unrelated lobby visit.
    Supersede,
    /// Drop it and tell the player: the lobby list never arrived.
    TimedOut,
}

pub(crate) fn queued_launch_step(
    menu_acted: bool,
    lobby_listed: bool,
    waited: std::time::Duration,
) -> QueuedLaunch {
    if menu_acted {
        QueuedLaunch::Supersede
    } else if lobby_listed {
        QueuedLaunch::Fire
    } else if waited >= QUEUED_LAUNCH_TIMEOUT {
        QueuedLaunch::TimedOut
    } else {
        QueuedLaunch::Wait
    }
}

/// Shown in the lobby when a queued arena launch gave up waiting.
pub(crate) const QUEUED_LAUNCH_TIMED_OUT: &str =
    "That Trial couldn't open — pick it again from Trials.";

/// Does [`GameState::leave_world`] write the world? Only when the player chose
/// Save, a world is live, and it is ours to write — a joined session is
/// discarded on leave, whatever was chosen (`joined_now` covers a session
/// whose kind was never recorded because it never finished loading).
pub(crate) fn should_save_on_leave(
    choice: SaveChoice,
    live: Option<WorldKind>,
    joined_now: bool,
) -> bool {
    choice == SaveChoice::Save
        && !joined_now
        && matches!(live, Some(WorldKind::Local | WorldKind::Arena))
}

/// Where a scenario picked from inside a world (the J board, `/scenario`) runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchMode {
    /// Overlay the objective on the current world (touches no inventory).
    InWorld,
    /// Leave (saving) and launch into the scenario's own arena world — the
    /// same path the Trials menu uses.
    Arena,
    /// Refused: the player is in a multiplayer session, and an arena launch
    /// would silently drop them (or their guests) out of it.
    Refused,
}

/// A scenario that clears or fills the inventory never runs in the player's
/// current world: that is how the J board's "Scavenger" wiped real gear and
/// how a kit Challenge became an infinite item tap (audit 2026-09-27, P3/P7).
/// Those go to their arena. One that leaves the bag alone may still overlay.
pub(crate) fn challenge_launch_mode(def: &ScenarioDef, networked: bool) -> LaunchMode {
    if !crate::scenario::provision_touches_inventory(def) {
        LaunchMode::InWorld
    } else if networked {
        LaunchMode::Refused
    } else {
        LaunchMode::Arena
    }
}

/// Privilege for a command typed into this machine's chat. In someone else's
/// world the local player is an ordinary player (`/help` and the other
/// `OpLevel::None` commands still work); in their own world they are op, and
/// the world's Commands setting still decides whether chat opens at all.
pub(crate) fn local_command_op_level(joined: bool) -> OpLevel {
    if joined {
        OpLevel::None
    } else {
        OpLevel::Op
    }
}

/// What a joiner is told when it tries to teleport itself (a waypoint jump
/// from the map or `/waypoint tp`): in someone else's world the body is the
/// server's (Spec 04 §5.3.1), and the server takes no teleport from a client
/// yet, so a jump would only be put back.
pub(crate) const JOINED_TELEPORT_REFUSED: &str =
    "Teleporting isn't available when you've joined someone else's world yet.";

/// May this machine's player teleport itself? Not in someone else's world
/// (see [`JOINED_TELEPORT_REFUSED`]).
pub(crate) fn self_teleport_allowed(joined: bool) -> bool {
    !joined
}


/// Is the pause menu's "Switch to Creative" locked? During a creative-locked
/// scenario, and always in someone else's world — the host owns the mode, and
/// `/gamemode` is already refused there (review W3 S1).
pub(crate) fn creative_switch_locked(scenario_locks: bool, joined: bool) -> bool {
    scenario_locks || joined
}

/// Only slot 0 is networked, so a second local seat is refused while hosting
/// or joined, until multi-local fan-out exists.
pub(crate) fn split_screen_seat_allowed(networked: bool) -> bool {
    !networked
}

/// Why a joined session has ended, if it has: the connection failed, was
/// refused (e.g. an identity-proof mismatch), or was closed. `None` while it
/// is still connecting or connected.
pub(crate) fn connection_end_reason(
    state: &crate::remote_client::ConnectionState,
) -> Option<String> {
    use crate::remote_client::ConnectionState;
    match state {
        ConnectionState::Failed(reason) => Some(reason.clone()),
        ConnectionState::Disconnected => Some("disconnected".to_string()),
        ConnectionState::Connecting | ConnectionState::Connected { .. } => None,
    }
}

/// [`connection_end_reason`] for a live client.
pub(crate) fn remote_session_end_reason(
    client: &crate::remote_client::RemoteClient,
) -> Option<String> {
    connection_end_reason(&client.state)
}

/// The friendly refusal shown when a pad tries to join mid-session online.
pub(crate) const SPLIT_SCREEN_ONLINE_REFUSAL: &str =
    "Split-screen isn't available in a shared world yet — one player per machine online.";

/// The refusal shown when a Challenge that uses the inventory is picked in a
/// multiplayer session.
pub(crate) const ARENA_ONLINE_REFUSAL: &str =
    "This Trial runs in its own world. Leave the shared world first, then start it from Trials.";

/// Reset the per-world `World` fields that `World::clear` keeps. The registries
/// (`override_registry`, `player_wardrobe`, `plan_registry`, micro models) are
/// engine/player content that must survive a world change; everything here is
/// a property of the world being left, and leaking it made the next world
/// inherit the last one's levers, hidden cells, waypoints, Workshop projects,
/// exhibits, scheduled updates and power floods (audit 2026-09-27, P3).
pub(crate) fn clear_per_world_fields(world: &mut World) {
    world.block_meta.clear();
    world.scheduler = Default::default();
    world.power = Default::default();
    world.render_hidden.clear();
    world.workshop = Default::default();
    world.waypoints.clear();
    world.exhibits.clear();
    world.rigs.clear();
    world.hostile_acts = Default::default();
    world.salt_licks.clear();
    world.tapped_rubber_logs.clear();
    // The chunk files the last world folder held (Spec 02 §8.4).
    world.forget_disk_chunks();
}

/// The wire form of a `/we` region edit: one `BlockChange` per changed cell,
/// carrying the block and meta the host now holds there.
pub(crate) fn region_broadcast(
    world: &World,
    changed: &[[i32; 3]],
) -> Vec<crate::protocol::BlockChange> {
    changed
        .iter()
        .map(|&[x, y, z]| crate::game_loop::broadcast_change(world, x, y, z, world.get_block(x, y, z)))
        .collect()
}

/// Zero every mob's health (the next despawn sweep drops them normally) —
/// `/killall`, applied to whichever sim it is handed.
pub(crate) fn kill_all_mobs(ecs: &mut hecs::World) {
    let mob_ids: Vec<hecs::Entity> = ecs
        .query::<&crate::entity::MobKind>()
        .iter()
        .map(|(id, _)| id)
        .collect();
    for id in mob_ids {
        if let Ok(mut h) = ecs.get::<&mut crate::combat::Health>(id) {
            h.current = 0.0;
        }
    }
}

impl crate::GameState {
    /// Write the live world — every save of the session goes through here:
    /// Save, Save & Quit and every other exit that saves, a resumable
    /// scenario's start, the replay snapshot. A save that lands drops the
    /// crash-recovery autosave it superseded
    /// (`save::save_world_superseding_autosave`); the outcome is recorded in
    /// [`SessionSaves`].
    pub(crate) fn save_live_world(&mut self) -> Result<(), String> {
        let result = crate::save::save_world_superseding_autosave(
            &self.world_name,
            &self.world,
            &self.players,
            self.biome_gen.seed,
            &crate::save::carts_to_saved(&self.ecs),
            &crate::save::tamed_mobs_to_saved(&self.ecs),
        )
        .inspect_err(|e| log::error!("Save failed: {e}"));
        self.session_saves.note_save(result.is_ok());
        result
    }

    /// The age of the crash-recovery autosave a "Quit without saving" would
    /// keep (`None` when that quit drops it, or there is none).
    pub(crate) fn kept_autosave_age(&self) -> Option<String> {
        if self.persists_locally() && self.session_saves.keeps_autosave() {
            crate::save::autosave_age(&self.world_name)
        } else {
            None
        }
    }

    /// Tell the player a save failed (see [`save_failed_toast`]).
    fn show_save_failed(&mut self, why: &str) {
        self.toast = Some((
            save_failed_toast(why),
            web_time::Instant::now() + std::time::Duration::from_secs(SAVE_FAILED_TOAST_SECS),
        ));
    }

    /// The pause menu's Save: write the world in place and stay paused. The
    /// crash-recovery autosave is cleared only once the save landed (it used to
    /// be cleared even after a failed save, leaving neither copy of the
    /// session); a failure says why. A joined session never writes a local save
    /// (no "remote_game" folder) — the host owns that world.
    pub(crate) fn pause_save(&mut self) {
        if self.persists_locally()
            && let Err(why) = self.save_live_world()
        {
            self.show_save_failed(&why);
        }
        // Spec 40 persistence — flush the player wardrobe so the latest pin/edit
        // is never lost (the debounce may not have fired yet). Best-effort;
        // native writes a profile file, WASM the Stash.
        if self.wardrobe_dirty {
            self.wardrobe_dirty = false;
            self.wardrobe_save_counter = 0;
            crate::wardrobe_store::save(self.world.player_wardrobe.set());
        }
    }

    /// Whether this session may write anything under `worlds/<world_name>`. A
    /// joined session never does: no autosave, no close-save, no meta, no
    /// "remote_game" folder.
    pub(crate) fn persists_locally(&self) -> bool {
        self.remote_client.is_none()
    }

    /// The one way out of a world. Saves (when `save` says so and the world is
    /// ours), flushes the global wardrobe, tears down every network role this
    /// machine held — hosted server, joined client, online host/rendezvous —
    /// clears the session overlays (scenario, Trial, board) and anything
    /// queued to fire in the lobby, and goes `to` the lobby or — for a quit in
    /// a showcase kiosk — the dead-end exit screen. The world data itself is
    /// wiped by `reset_for_world_change` on the next entry, which every load
    /// path runs before reading the new world in.
    ///
    /// Returns whether the player left. A save that FAILS ends nothing: the
    /// player stays in the world — still live, nothing torn down — with a toast
    /// saying why, so the session can be saved again, or left with "Quit
    /// without saving" or a second window close (which retries the save once,
    /// [`GameState::close_window`]) — both of which then KEEP the
    /// crash-recovery autosave ([`SessionSaves`]; review 2026-10-06: a failed
    /// Save & Quit used to only log, then leave, losing the session in
    /// silence). Callers that hop somewhere after leaving must not hop on
    /// `false`.
    pub(crate) fn leave_world(&mut self, save: SaveChoice, to: ExitTo) -> bool {
        let live = self.live_world.take();
        let joined = self.remote_client.is_some();
        // Reaching the line after this means any save it asked for landed
        // (and dropped the autosave it superseded).
        let saved = should_save_on_leave(save, live, joined);
        if saved && let Err(why) = self.save_live_world() {
            self.live_world = live;
            self.show_save_failed(&why);
            return false;
        }
        // The crash-recovery copy: dropped only once a save superseded it or
        // the player chose to discard a session whose autosave nothing guards —
        // never by a close or a lost connection.
        if should_clear_autosave(save, saved, live, joined, self.session_saves.keeps_autosave()) {
            crate::save::clear_autosave(&self.world_name);
        }
        self.session_saves = SessionSaves::default();
        // Nothing queued by the session being left may fire later into an
        // unrelated lobby visit (callers that hop re-queue AFTER this).
        self.pending_menu_action = None;
        self.pending_scenario_launch = None;
        self.pending_workshop_for_skin = false;
        self.region_broadcast_queue.clear();
        // Spec 40 — the wardrobe is a GLOBAL asset, flushed whatever happened
        // to the world (the debounce may not have fired yet).
        if self.wardrobe_dirty {
            self.wardrobe_dirty = false;
            self.wardrobe_save_counter = 0;
            crate::wardrobe_store::save(self.world.player_wardrobe.set());
        }

        // Network teardown, both targets (a browser joiner holds a
        // `remote_client` too). Tell the server we are going so it can free the
        // slot, then drop: dropping the HostedServer closes the accept thread
        // and the announce socket.
        if let Some(mut client) = self.remote_client.take()
            && client.is_connected()
        {
            client.disconnect();
        }
        self.hosted_server = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.lan_host = None;
            self.entry_toast.discard();
        }
        // Online play by contact §5.1 step 6 — give the router its port back,
        // close the relay REQs, retire the bearer.
        #[cfg(not(target_arch = "wasm32"))]
        self.stop_online();
        self.remote_items.clear();
        self.remote_projectiles.clear();
        self.remote_mobs.clear();
        self.remote_mobs_clock = None;
        self.own_health.reset();
        self.joiner_actions.clear();
        self.sent_uses.clear();
        self.remote_players.clear();
        self.remote_swing.clear();
        self.pending_block_changes.clear();
        self.pending_mined.clear();
        // C3a-fix-1 — window-event carriers held while edits were unsent
        // belong to the session being left: applied to a later session they
        // would land stale items and push its fresh `events_applied` to this
        // one's numbers.
        self.window_inbox.clear();

        // Session overlays that belong to the world being left.
        self.scenario = None;
        self.active_trial = None;
        self.trial_outcome = None;
        self.challenge_board_open = false;
        self.controls_card_open = false;
        self.build_guide = None;

        if exit_dead_ends(to, crate::showcase::should_dead_end_exit(&self.showcase)) {
            // Showcase: Quit is a one-step dead-end to the terminal exit
            // screen, never back to the lobby (spec §10).
            self.showcase_exited = true;
        } else {
            self.mode = crate::GameMode::Menu(Box::new(crate::menu::MenuState::new()));
        }
        true
    }

    /// The window's close button (native — the browser has no close-save; its
    /// autosave rides IndexedDB). Closing is one more way out of a world, so it
    /// goes through `leave_world` like Save & Quit, and SAVES while the player
    /// is in a loaded world this machine owns — their own or a Trial arena
    /// (never from the lobby, mid-load, after "Quit without saving", or a joined
    /// session). A close-save that FAILS keeps the window open in the world, the
    /// toast saying why (review 2026-10-06), to retry, "Quit without saving", or
    /// close again — which tries the save once more and quits either way,
    /// keeping the autosave if it fails again ([`after_close_save`]). Returns
    /// whether the app should exit.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn close_window(&mut self) -> bool {
        let in_world = matches!(self.mode, crate::GameMode::Playing | crate::GameMode::Paused { .. });
        let choice = close_choice(in_world, self.live_world);
        let left = self.live_world.is_none() || self.leave_world(choice, ExitTo::Quit);
        match after_close_save(left, self.session_saves.close_save_failed) {
            CloseOutcome::Quit => true,
            CloseOutcome::StayToRetry => {
                self.session_saves.close_save_failed = true;
                let hint = close_again_hint(self.kept_autosave_age().as_deref());
                if let Some((msg, _)) = self.toast.as_mut() {
                    msg.push_str(&hint);
                }
                false
            }
            CloseOutcome::QuitKeepingAutosave => {
                // Never saves, never clears the autosave, always leaves.
                self.leave_world(SaveChoice::Abandon, ExitTo::Quit);
                true
            }
        }
    }

    /// Leave a world whose session ended under the player (host quit or
    /// crashed, the connection dropped, a kick), then say why in the lobby.
    pub(crate) fn leave_world_with_notice(&mut self, save: SaveChoice, notice: String) {
        self.leave_world(save, ExitTo::Lobby);
        if let crate::GameMode::Menu(menu) = &mut self.mode {
            menu.notice = Some(notice);
        }
    }

    /// Start a scenario picked from inside a world (J board, `/scenario`).
    /// Returns true if the player stays in this world (overlay or refusal),
    /// false if they left for the arena.
    pub(crate) fn launch_scenario_from_world(&mut self, def: ScenarioDef) -> bool {
        let networked = self.hosted_server.is_some() || self.remote_client.is_some();
        match challenge_launch_mode(&def, networked) {
            LaunchMode::InWorld => {
                self.start_scenario(def);
                true
            }
            LaunchMode::Arena => {
                // Leave first (saving; it cancels anything already queued),
                // then queue the launch: the lobby's first frame fires it
                // exactly as if the Trials menu's Play had been clicked. A save
                // that failed keeps the player here, told why — no hop.
                if !self.leave_world(SaveChoice::Save, ExitTo::Lobby) {
                    return true;
                }
                self.pending_menu_action = Some((
                    crate::menu::MenuAction::PlayScenario { def: Box::new(def) },
                    web_time::Instant::now(),
                ));
                self.release_cursor();
                false
            }
            LaunchMode::Refused => {
                self.toast = Some((
                    ARENA_ONLINE_REFUSAL.to_string(),
                    web_time::Instant::now() + std::time::Duration::from_secs(5),
                ));
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{named_builtin_def, KitItem, ScenarioKind};

    #[test]
    fn close_saves_only_a_live_local_world_in_play() {
        // Lobby / splash / first Loading frame / after Quit-without-saving:
        // nothing is live, so nothing is written (no phantom "default").
        assert!(!should_save_on_close(false, None));
        assert!(!should_save_on_close(true, None));
        // A live own world, in play or paused over it.
        assert!(should_save_on_close(true, Some(WorldKind::Local)));
        // Back on the menu after leaving: the kind was taken on leave, but even
        // a stale one must not save outside the world.
        assert!(!should_save_on_close(false, Some(WorldKind::Local)));
        // Never a joined session.
        assert!(!should_save_on_close(true, Some(WorldKind::Joined)));
    }

    #[test]
    fn closing_inside_an_arena_saves_the_run_and_keeps_its_crash_copy_until_saved() {
        // Review W3 B1: closing mid-Satori-Rush discarded the run AND deleted
        // its 5-minute crash-recovery copy.
        assert!(should_save_on_close(true, Some(WorldKind::Arena)));
        assert_eq!(close_choice(true, Some(WorldKind::Arena)), SaveChoice::Save);
        assert_eq!(close_choice(true, Some(WorldKind::Local)), SaveChoice::Save);
        // A save that failed leaves the crash copy for recovery.
        assert!(!should_clear_autosave(SaveChoice::Save, false, Some(WorldKind::Arena), false, false));
        // A landed save supersedes it (left beside the fresh save, a stale
        // autosave could roll it back).
        assert!(should_clear_autosave(SaveChoice::Save, true, Some(WorldKind::Arena), false, false));
    }

    #[test]
    fn a_close_never_clears_an_autosave_it_did_not_supersede() {
        for live in [None, Some(WorldKind::Local), Some(WorldKind::Arena), Some(WorldKind::Joined)] {
            for in_world in [false, true] {
                let choice = close_choice(in_world, live);
                assert_ne!(choice, SaveChoice::Discard, "a close is never a discard");
                for (joined, keep) in [(false, false), (true, false), (false, true), (true, true)] {
                    // Whatever happens, an unsaved close never clears it.
                    assert!(!should_clear_autosave(choice, false, live, joined, keep));
                }
            }
        }
        // Only the player's explicit "Quit without saving" discards it.
        assert!(should_clear_autosave(SaveChoice::Discard, false, Some(WorldKind::Local), false, false));
        assert!(should_clear_autosave(SaveChoice::Discard, false, Some(WorldKind::Arena), false, false));
        // A joined session has none of its own to clear.
        assert!(!should_clear_autosave(SaveChoice::Discard, false, Some(WorldKind::Joined), true, false));
    }

    /// Review 2026-10-06 — a save that keeps failing left "Quit without saving"
    /// (a Discard) as the only way out, and a Discard deleted the autosave: the
    /// whole session lost, where a failed Save & Quit on main kept it. After a
    /// failed save, or in a session opened FROM the autosave (with a damaged
    /// world.dat, the only good copy), a Discard keeps it.
    #[test]
    fn a_discard_keeps_the_autosave_after_a_failed_save_or_an_autosave_open() {
        let local = Some(WorldKind::Local);
        for (save_failed, opened_from_autosave, keeps) in [
            (false, false, false),
            (true, false, true),
            (false, true, true),
            (true, true, true),
        ] {
            let s = SessionSaves { save_failed, opened_from_autosave, close_save_failed: false };
            assert_eq!(s.keeps_autosave(), keeps, "{s:?}");
            assert_eq!(
                should_clear_autosave(SaveChoice::Discard, false, local, false, s.keeps_autosave()),
                !keeps,
                "{s:?}"
            );
            // A save that landed still supersedes it, whatever came before.
            assert!(should_clear_autosave(SaveChoice::Save, true, local, false, s.keeps_autosave()));
        }
    }

    #[test]
    fn session_saves_follow_the_saves_that_land_and_fail() {
        assert_eq!(SessionSaves::opened(false), SessionSaves::default());
        let mut s = SessionSaves::opened(true);
        assert!(s.keeps_autosave(), "opened from the autosave");
        s.note_save(false);
        assert!(s.save_failed && s.keeps_autosave());
        s.close_save_failed = true;
        // A save that lands writes world.dat and drops the autosave: nothing
        // left to guard, and the next close saves again.
        s.note_save(true);
        assert_eq!(s, SessionSaves::default());
        s.note_save(false);
        assert!(s.keeps_autosave(), "a later failure guards it again");
    }

    /// A close whose save failed keeps the player in the world (to retry). The
    /// SECOND close tries the save once more and quits either way — keeping the
    /// autosave when it fails again — so a save that keeps failing never traps
    /// the player. Third review (2026-10-06): the flag used to make EVERY later
    /// close quit without even trying to save, however much was played since.
    #[test]
    fn a_second_close_retries_the_save_once_then_quits_keeping_the_autosave() {
        for live in [Some(WorldKind::Local), Some(WorldKind::Arena)] {
            // Every close of a live own world saves — after a failed one too.
            assert_eq!(close_choice(true, live), SaveChoice::Save);
            assert!(should_save_on_leave(close_choice(true, live), live, false));
        }
        // The save landed (or there was nothing to save): quit.
        assert_eq!(after_close_save(true, false), CloseOutcome::Quit);
        assert_eq!(after_close_save(true, true), CloseOutcome::Quit);
        // The first failure stays in the world, to retry.
        assert_eq!(after_close_save(false, false), CloseOutcome::StayToRetry);
        // The retry failed too: quit anyway, by a close that keeps the autosave.
        assert_eq!(after_close_save(false, true), CloseOutcome::QuitKeepingAutosave);
        for live in [Some(WorldKind::Local), Some(WorldKind::Arena)] {
            assert!(!should_save_on_leave(SaveChoice::Abandon, live, false), "no third try");
            assert!(!should_clear_autosave(SaveChoice::Abandon, false, live, false, true), "kept");
        }
        // Any save that lands clears the flag: the next close is a first close.
        let mut s = SessionSaves { close_save_failed: true, save_failed: true, ..SessionSaves::default() };
        s.note_save(true);
        assert!(!s.close_save_failed);
        assert_eq!(after_close_save(false, s.close_save_failed), CloseOutcome::StayToRetry);
    }

    #[test]
    fn quit_without_saving_says_when_it_keeps_the_autosave() {
        assert_eq!(quit_no_save_label(None), "Quit Without Saving");
        assert_eq!(
            quit_no_save_label(Some("3 minutes ago")),
            "Quit — your autosave from 3 minutes ago is kept"
        );
        assert_eq!(quit_no_save_warning(None), "Unsaved progress will be lost!");
        assert_eq!(
            quit_no_save_warning(Some("3 minutes ago")),
            "Anything since your autosave from 3 minutes ago will be lost."
        );
        assert_eq!(
            close_again_hint(Some("just now")),
            " Close the window again to try once more and quit — if the save fails again, \
             your autosave from just now is kept."
        );
        assert_eq!(
            close_again_hint(None),
            " Close the window again to try once more and quit — if the save fails again, \
             the game quits without saving."
        );
    }

    #[test]
    fn only_a_quit_dead_ends_a_showcase_kiosk() {
        // Review W3 S4: the J-board arena hop and the end cards dead-ended
        // the kiosk on the terminal exit screen.
        assert!(exit_dead_ends(ExitTo::Quit, true));
        assert!(!exit_dead_ends(ExitTo::Lobby, true));
        assert!(!exit_dead_ends(ExitTo::Quit, false));
        assert!(!exit_dead_ends(ExitTo::Lobby, false));
    }

    #[test]
    fn a_queued_arena_launch_never_fires_into_a_later_lobby_visit() {
        // Review W3 S6.
        use std::time::Duration;
        let t0 = Duration::ZERO;
        assert_eq!(queued_launch_step(false, true, t0), QueuedLaunch::Fire);
        assert_eq!(queued_launch_step(true, true, t0), QueuedLaunch::Supersede);
        assert_eq!(queued_launch_step(true, false, t0), QueuedLaunch::Supersede);
        assert_eq!(queued_launch_step(false, false, t0), QueuedLaunch::Wait);
        assert_eq!(
            queued_launch_step(false, false, QUEUED_LAUNCH_TIMEOUT),
            QueuedLaunch::TimedOut
        );
    }

    #[test]
    fn a_joiner_cannot_switch_to_creative_from_the_pause_menu() {
        // Review W3 S1.
        assert!(creative_switch_locked(false, true));
        assert!(creative_switch_locked(true, false));
        assert!(!creative_switch_locked(false, false));
    }

    #[test]
    fn a_joiner_cannot_teleport_itself() {
        // Its body is the server's (Spec 04 §5.3.1); in its own world, it can.
        assert!(!self_teleport_allowed(true));
        assert!(self_teleport_allowed(false));
        assert!(JOINED_TELEPORT_REFUSED.contains("joined someone else's world"));
    }

    #[test]
    fn leave_never_saves_a_joined_session_or_a_discard() {
        use SaveChoice::*;
        assert!(should_save_on_leave(Save, Some(WorldKind::Local), false));
        assert!(should_save_on_leave(Save, Some(WorldKind::Arena), false));
        assert!(!should_save_on_leave(Save, Some(WorldKind::Joined), true));
        // A join that never finished loading has no kind yet, but is joined.
        assert!(!should_save_on_leave(Save, None, true));
        assert!(!should_save_on_leave(Discard, Some(WorldKind::Local), false));
        assert!(!should_save_on_leave(Save, None, false));
    }

    #[test]
    fn classify_prefers_joined_over_arena() {
        assert_eq!(classify_world(true, true), WorldKind::Joined);
        assert_eq!(classify_world(false, true), WorldKind::Arena);
        assert_eq!(classify_world(false, false), WorldKind::Local);
    }

    #[test]
    fn scavenger_from_the_board_goes_to_its_arena_not_the_current_world() {
        // Audit P3: Scavenger (Timed Challenge) cleared the real inventory.
        let def = named_builtin_def("scavenger").expect("scavenger is bundled");
        assert_eq!(challenge_launch_mode(&def, false), LaunchMode::Arena);
    }

    #[test]
    fn kit_challenge_never_grants_its_kit_in_the_current_world() {
        // Audit P7: a kit Challenge in-world was an infinite item tap.
        let mut def = named_builtin_def("scavenger").unwrap();
        def.objective = crate::scenario::Objective::FreeRoam;
        def.kind = ScenarioKind::Challenge;
        def.kit = vec![KitItem { name: "stone".to_string(), count: 64 }];
        assert_eq!(challenge_launch_mode(&def, false), LaunchMode::Arena);
        // No kit, no clear → may overlay the current world.
        def.kit.clear();
        assert_eq!(challenge_launch_mode(&def, false), LaunchMode::InWorld);
        // An arena launch is refused inside a multiplayer session; an overlay
        // is still fine.
        def.kit = vec![KitItem { name: "stone".to_string(), count: 64 }];
        assert_eq!(challenge_launch_mode(&def, true), LaunchMode::Refused);
        def.kit.clear();
        assert_eq!(challenge_launch_mode(&def, true), LaunchMode::InWorld);
    }

    #[test]
    fn every_bundled_kit_challenge_routes_to_an_arena() {
        for (name, _) in crate::scenario::challenge_listing() {
            let def = named_builtin_def(name).unwrap();
            let touches = !def.kit.is_empty()
                || crate::scenario::provision_clears_inventory(&def);
            let mode = challenge_launch_mode(&def, false);
            assert_eq!(
                mode == LaunchMode::Arena,
                touches,
                "{name}: launch mode {mode:?} disagrees with its inventory use"
            );
        }
    }

    #[test]
    fn joiner_commands_run_without_op() {
        assert_eq!(local_command_op_level(true), OpLevel::None);
        assert_eq!(local_command_op_level(false), OpLevel::Op);
    }

    #[test]
    fn joiner_cannot_give_or_gamemode_but_can_help() {
        let mut registry = crate::commands::CommandRegistry::new();
        crate::commands::builtins::register_all(&mut registry);
        let mut world = World::new();
        let mut players = vec![crate::player_slot::PlayerSlot::new(
            0,
            glam::Vec3::new(0.5, 80.0, 0.5),
            1.0,
        )];
        let (mut wt, mut step, mut creative) = (0u32, 1u32, false);
        let mut mode = crate::play_mode::PlayMode::Survival;
        let (mut c, mut e, mut p) = (false, false, false);
        let mut log = Vec::new();
        let mut ctx = crate::commands::CommandContext {
            world: &mut world,
            world_time: &mut wt,
            world_time_step: &mut step,
            is_creative: &mut creative,
            play_mode: &mut mode,
            seed: 1,
            world_name: "remote_game",
            players: &mut players,
            player_idx: 0,
            op_level: local_command_op_level(true),
            current_tick: 0,
            log: &mut log,
            registry: &registry,
            cheats_used_marker: &mut c,
            ever_creative_marker: &mut e,
            pure_survival_broken_marker: &mut p,
        };
        for cheat in [
            "/give diamond_block 64",
            "/gamemode creative",
            "/scenario scavenger",
            // Review W3 S2: a Race teleports + places beacons; a Challenge
            // can grant a kit. Both are refused to a joiner.
            "/trial sprint",
            "/trial scavenger",
            "/trial",
        ] {
            let r = crate::commands::dispatch(cheat, &mut ctx, &registry);
            assert!(
                matches!(r, crate::commands::CommandResult::Error(ref m) if m.starts_with("permission denied")),
                "{cheat} must be refused to a joiner, got {r:?}"
            );
        }
        let r = crate::commands::dispatch("/help", &mut ctx, &registry);
        assert!(!matches!(r, crate::commands::CommandResult::Error(_)), "/help got {r:?}");
        drop(ctx);
        assert!(!creative, "a refused /gamemode must not flip the mode");
        assert_eq!(players[0].inventory.distinct_item_kinds(), 0, "a refused /give must not grant");
    }

    #[test]
    fn a_failed_or_closed_connection_ends_the_joined_session() {
        use crate::remote_client::ConnectionState;
        assert_eq!(connection_end_reason(&ConnectionState::Connecting), None);
        assert_eq!(
            connection_end_reason(&ConnectionState::Connected { player_index: 1, seed: 7 }),
            None
        );
        assert_eq!(
            connection_end_reason(&ConnectionState::Failed("refused".into())).as_deref(),
            Some("refused")
        );
        assert!(connection_end_reason(&ConnectionState::Disconnected).is_some());
    }

    #[test]
    fn a_hosts_region_edit_reaches_the_server_world_and_its_broadcast() {
        // Audit P7: host-side `/we` never reached the server or joiners.
        use crate::block;
        let mut host = World::new();
        let mut server = World::new();
        server.set_block(0, 4, 0, block::BATTERY);
        server.insert_power_device(
            (0, 4, 0),
            crate::power::PowerDeviceData::new(
                crate::power::PowerDeviceKind::Battery,
                crate::meta::facing(0),
            ),
        );
        host.set_block(0, 4, 0, block::BATTERY);
        let mut changed = Vec::new();
        crate::worldedit::region_set(&mut host, [0, 4, 0], [1, 4, 0], block::STONE, &mut changed);
        let edits = region_broadcast(&host, &changed);
        assert_eq!(edits.len(), 2);
        for bc in &edits {
            server.apply_remote_block_change(bc);
        }
        assert_eq!(server.get_block(0, 4, 0), block::STONE);
        assert_eq!(server.get_block(1, 4, 0), block::STONE);
        assert!(server.power_device_at((0, 4, 0)).is_none(), "no ghost battery on the server");
    }

    #[test]
    fn split_screen_refused_while_networked() {
        assert!(split_screen_seat_allowed(false));
        assert!(!split_screen_seat_allowed(true));
    }

    #[test]
    fn a_failed_save_says_why_and_that_the_last_save_is_safe() {
        assert_eq!(
            save_failed_toast("refusing to write: existing world_meta.json is damaged"),
            "Couldn't save: refusing to write: existing world_meta.json is damaged. \
             Your last save is safe."
        );
    }

    #[test]
    fn clear_per_world_fields_drops_the_last_worlds_state() {
        let mut w = World::new();
        w.block_meta.insert((1, 2, 3), 7);
        w.render_hidden.insert((1, 2, 3));
        w.salt_licks.insert((4, 5, 6));
        w.tapped_rubber_logs.insert((1, 1, 1), 99);
        w.waypoints.push(crate::waypoint::Waypoint {
            id: 1,
            name: "home".to_string(),
            pos: [0, 64, 0],
            colour: [255, 176, 0],
            kind: crate::waypoint::WaypointKind::Manual,
        });
        clear_per_world_fields(&mut w);
        assert!(w.block_meta.is_empty());
        assert!(w.render_hidden.is_empty());
        assert!(w.salt_licks.is_empty());
        assert!(w.tapped_rubber_logs.is_empty());
        assert!(w.waypoints.is_empty());
        assert!(w.exhibits.is_empty());
        assert!(w.rigs.is_empty());
    }
    /// Harness tests for `leave_world` itself (the real GameState, headless).
    /// GPU-gated like every `game_harness` test.
    #[cfg(not(target_arch = "wasm32"))]
    fn isolate_saves() {
        let tmp = std::env::temp_dir().join(format!("axenstax-harness-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // SAFETY: every harness test sets the same pid-scoped value.
        unsafe { std::env::set_var("AXENSTAX_WORLDS_DIR", &tmp) };
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas — run: cargo test -- --ignored game_harness"]
    fn game_harness_leave_world_cancels_a_queued_arena_launch() {
        isolate_saves();
        let mut hg = crate::test_game_harness::HeadlessGame::boot_into_world("exit-cancels-queue");
        let def = named_builtin_def("scavenger").unwrap();
        hg.state.pending_menu_action = Some((
            crate::menu::MenuAction::PlayScenario { def: Box::new(def.clone()) },
            web_time::Instant::now(),
        ));
        hg.state.pending_scenario_launch = None;
        hg.state.leave_world(SaveChoice::Save, ExitTo::Lobby);
        assert!(hg.state.pending_menu_action.is_none(), "leave_world cancels a queued launch");
        assert!(hg.state.live_world.is_none());
        assert!(matches!(hg.state.mode, crate::GameMode::Menu(_)));
        assert!(crate::save::world_exists("exit-cancels-queue"), "Save wrote the world");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas — run: cargo test -- --ignored game_harness"]
    fn game_harness_board_arena_hop_saves_then_queues_the_launch() {
        isolate_saves();
        let mut hg = crate::test_game_harness::HeadlessGame::boot_into_world("exit-board-hop");
        let def = named_builtin_def("scavenger").unwrap();
        assert!(!hg.state.launch_scenario_from_world(def), "left for the arena");
        assert!(crate::save::world_exists("exit-board-hop"), "saved before teardown");
        assert!(hg.state.live_world.is_none());
        assert!(hg.state.scenario.is_none(), "no kit/clear ran in the player's own world");
        assert!(
            matches!(hg.state.pending_menu_action, Some((crate::menu::MenuAction::PlayScenario { .. }, _))),
            "queued AFTER leave_world's cancel"
        );
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas — run: cargo test -- --ignored game_harness"]
    fn game_harness_closing_an_arena_saves_it() {
        isolate_saves();
        let mut hg = crate::test_game_harness::HeadlessGame::boot_into_world("exit-arena-close");
        hg.state.live_world = Some(WorldKind::Arena);
        // The window-close path (main.rs `CloseRequested`).
        assert!(hg.state.close_window(), "the window closes");
        assert!(crate::save::world_exists("exit-arena-close"), "the arena run was saved");
    }

    /// A unique world name per harness test (they share one worlds dir).
    #[cfg(not(target_arch = "wasm32"))]
    fn harness_world(tag: &str) -> String {
        let name = format!("{tag}-{:?}", std::thread::current().id())
            .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-");
        let _ = std::fs::remove_dir_all(crate::save::world_dir(&name));
        name
    }

    /// Damage `name`'s meta so every save of it is refused (`meta_write_blocked`).
    #[cfg(not(target_arch = "wasm32"))]
    fn make_saves_fail(name: &str) {
        let dir = crate::save::world_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
    }

    /// Review 2026-10-06 — a failed Save & Quit used to only log, then leave:
    /// the session was lost in silence. Now the player stays in the world, told
    /// why, and the crash-recovery autosave is kept.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_failed_save_and_quit_keeps_the_player_in_the_world() {
        isolate_saves();
        let name = harness_world("exit-save-fails");
        let mut hg = crate::test_game_harness::HeadlessGame::boot_into_world(&name);
        let s = &hg.state;
        crate::save::autosave_world(&name, &s.world, &s.players, s.biome_gen.seed, &[], &[]).unwrap();
        make_saves_fail(&name);

        hg.state.mode = crate::GameMode::Paused { confirm_quit: false, confirm_creative: false };
        assert!(!hg.state.leave_world(SaveChoice::Save, ExitTo::Quit), "a failed save never leaves");
        assert!(!matches!(hg.state.mode, crate::GameMode::Menu(_)), "still in the world");
        assert_eq!(hg.state.live_world, Some(WorldKind::Local), "still live: a retry can save it");
        let (toast, _) = hg.state.toast.clone().expect("the player is told");
        assert!(toast.starts_with("Couldn't save: "), "{toast}");
        assert!(toast.ends_with("Your last save is safe."), "{toast}");
        assert!(
            crate::save::world_dir(&name).join("autosave/world.dat").is_file(),
            "the crash-recovery autosave is kept"
        );
        // Every other exit that saves stays too: the J-board arena hop...
        let def = named_builtin_def("scavenger").unwrap();
        assert!(hg.state.launch_scenario_from_world(def), "the hop is refused, not taken");
        assert!(hg.state.pending_menu_action.is_none(), "no launch queued");
        // ...and the Trial / end-card exits.
        assert!(!hg.state.leave_world(SaveChoice::Save, ExitTo::Lobby));
        assert_eq!(hg.state.live_world, Some(WorldKind::Local));

        // "Quit without saving" still leaves — and, after the failed save, says
        // and does keep the autosave (review 2026-10-06).
        let age = hg.state.kept_autosave_age().expect("the autosave is kept");
        assert!(quit_no_save_label(Some(&age)).starts_with("Quit — your autosave from "));
        assert!(hg.state.leave_world(SaveChoice::Discard, ExitTo::Quit));
        assert!(hg.state.live_world.is_none());
        assert!(
            crate::save::world_dir(&name).join("autosave/world.dat").is_file(),
            "a discard after a failed save keeps the autosave"
        );
    }

    /// Third review (2026-10-06) — a failed window-close save keeps the player in
    /// the world; the next close tries the save once more and quits either way,
    /// keeping the autosave when that fails too. A save that lands in between
    /// makes the next close a first close again.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_second_close_retries_the_save_then_quits_keeping_the_autosave() {
        isolate_saves();
        let name = harness_world("exit-close-retry");
        let mut hg = crate::test_game_harness::HeadlessGame::boot_into_world(&name);
        let dir = crate::save::world_dir(&name);
        let autosave = |hg: &crate::test_game_harness::HeadlessGame| {
            let s = &hg.state;
            crate::save::autosave_world(&name, &s.world, &s.players, s.biome_gen.seed, &[], &[]).unwrap();
        };
        autosave(&hg);
        let good_meta = std::fs::read(dir.join("world_meta.json")).ok();
        make_saves_fail(&name);

        assert!(!hg.state.close_window(), "a failed close stays in the world");
        let (toast, _) = hg.state.toast.clone().expect("the player is told");
        assert!(toast.contains("Close the window again to try once more and quit"), "{toast}");
        // A save that lands in between: the next close is a first close again.
        match &good_meta {
            Some(bytes) => std::fs::write(dir.join("world_meta.json"), bytes).unwrap(),
            None => std::fs::remove_file(dir.join("world_meta.json")).unwrap(),
        }
        hg.state.pause_save();
        assert!(!hg.state.session_saves.close_save_failed, "a landed save clears it");
        autosave(&hg);
        make_saves_fail(&name);
        assert!(!hg.state.close_window(), "a first close again: stays");
        // The second close retries, fails, and quits keeping the autosave.
        assert!(hg.state.close_window(), "the second close quits");
        assert!(hg.state.live_world.is_none());
        assert!(dir.join("autosave/world.dat").is_file(), "the autosave is kept");
    }

    /// Review 2026-10-06 — the pause menu's Save cleared the crash-recovery
    /// autosave even when the save failed. Now: cleared only once a save
    /// landed; a failure says why.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas — run: cargo test -- --ignored game_harness"]
    fn game_harness_pause_save_keeps_the_autosave_unless_the_save_landed() {
        isolate_saves();
        let name = harness_world("exit-pause-save");
        let mut hg = crate::test_game_harness::HeadlessGame::boot_into_world(&name);
        let dir = crate::save::world_dir(&name);
        let s = &hg.state;
        crate::save::autosave_world(&name, &s.world, &s.players, s.biome_gen.seed, &[], &[]).unwrap();
        let good_meta = std::fs::read(dir.join("world_meta.json")).ok();
        make_saves_fail(&name);

        hg.state.toast = None;
        hg.state.pause_save();
        assert!(dir.join("autosave/world.dat").is_file(), "a failed save keeps the autosave");
        let (toast, _) = hg.state.toast.clone().expect("the player is told");
        assert!(toast.starts_with("Couldn't save: ") && toast.ends_with("Your last save is safe."));

        // Repaired: the save lands and supersedes the autosave.
        match good_meta {
            Some(bytes) => std::fs::write(dir.join("world_meta.json"), bytes).unwrap(),
            None => std::fs::remove_file(dir.join("world_meta.json")).unwrap(),
        }
        hg.state.toast = None;
        hg.state.pause_save();
        assert!(crate::save::world_exists(&name), "the save landed");
        assert!(!dir.join("autosave").exists(), "a landed save clears the autosave");
        assert!(hg.state.toast.is_none(), "no failure toast");
    }

    /// Review 2026-10-06 — opening a world from its crash-recovery autosave
    /// deleted that autosave straight away; with a damaged `world.dat` that was
    /// the only good copy. It is now kept until a save writes `world.dat`.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas — run: cargo test -- --ignored game_harness"]
    fn game_harness_an_autosave_open_keeps_the_autosave_until_a_save_lands() {
        isolate_saves();
        let name = harness_world("exit-autosave-open");
        let dir = crate::save::world_dir(&name);
        let mut w = World::new();
        w.set_block(3, 64, 5, crate::block::BEDROCK);
        crate::save::write_world_folder(
            &name,
            &crate::save::WorldMeta::new(&name),
            &crate::save::minimal_world_save_for_tests(7),
            &w,
        )
        .unwrap();
        let slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(0.5, 80.0, 0.5), 1.0);
        crate::save::autosave_world(&name, &w, std::slice::from_ref(&slot), 7, &[], &[]).unwrap();
        // The last manual save is damaged (no footer, so the open isn't refused).
        std::fs::write(dir.join("world.dat"), b"\x01not a world save").unwrap();

        let mut hg = crate::test_game_harness::HeadlessGame::boot_into_world(&name);
        assert_eq!(hg.state.world.get_block(3, 64, 5), crate::block::BEDROCK, "opened from the autosave");
        assert!(dir.join("autosave/world.dat").is_file(), "the only good copy is kept");

        // A save that lands writes world.dat, and only then is the autosave dropped.
        assert!(hg.state.leave_world(SaveChoice::Save, ExitTo::Lobby));
        assert!(!dir.join("autosave").exists());
        let mut back = World::new();
        crate::save::load_world(&name, &mut back).expect("world.dat is good again");
        assert_eq!(back.get_block(3, 64, 5), crate::block::BEDROCK);
    }
}

//! World management menu and pause menu — rendered via egui.
//!
//! Main menu: world cards with metadata, selection, action bar, dialogs
//! for create/edit/delete/fork. Pause menu: simplified (no delete).

use crate::brand;
use crate::save::{self, WorldEntry, WorldMeta};

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

// Copperline palette (brand/BRAND-GUIDELINES.md). Stone and secondary text
// exceed 4.5:1 on the dark, raised and selected surfaces. Copper headings
// sit on the darker panel backgrounds; selection outlines are not text.
const TITLE_COLOR: egui::Color32 = brand::COPPER;
const SUBTITLE_COLOR: egui::Color32 = brand::MUTED_TEXT;
const TEXT_COLOR: egui::Color32 = brand::STONE;
const DIM_TEXT: egui::Color32 = brand::MUTED_TEXT;
const BG_DARK: egui::Color32 = brand::DEEP_FRONTIER;
const CARD_SELECTED_BORDER: egui::Color32 = brand::COPPER;
const CARD_BORDER: egui::Color32 = brand::BORDER;
const PANEL_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(18, 32, 36, 245);
const BADGE_CREATIVE_BG: egui::Color32 = egui::Color32::from_rgb(30, 58, 90);
const BADGE_CREATIVE_TEXT: egui::Color32 = egui::Color32::from_rgb(106, 176, 199);
const DELETE_RED: egui::Color32 = egui::Color32::from_rgb(236, 104, 104);
/// Sky Blue lightened: pure `#3F7FBF` is only 4.2:1 on Deep Frontier (fails as text).
const ACTION_BLUE: egui::Color32 = egui::Color32::from_rgb(122, 172, 226);

// ---------------------------------------------------------------------------
// Menu State
// ---------------------------------------------------------------------------

/// Dialog state within the menu.
#[derive(Clone, Debug)]
pub enum MenuDialog {
    None,
    Create {
        name: String,
        seed: String,
        creative: bool,
        commands_enabled: bool,
        cloud_save: bool,
        // Blank-canvas world config (Task B3)
        world_type: String,   // "normal" | "flat"
        ground: String,       // "none"|"grass"|"sand"|"stone"|"dirt"|"snow"|"water"
        water_depth: u8,      // 1–8, only used when ground == "water"
        time_lock: String,    // "cycle"|"day"|"night"
        mobs_enabled: bool,
    },
    Edit { world_idx: usize, name: String, description: String },
    Delete { world_idx: usize, confirm_text: String },
    Forking { world_idx: usize },
    JoinDirect { address: String },  // Direct connect dialog
    /// Spec 40 (The Workshop) — confirm wiping the Workshop room back to the void
    /// floor. Keeps the player's saved reskins; only clears what's built.
    ResetWorkshop,
    /// Native Signet sign-in — shows a `nostrconnect://` QR for the phone to scan
    /// (primary) plus a `bunker://` paste fallback. `paste_uri` holds the fallback
    /// field text; `show_paste` whether the fallback section is expanded. Only
    /// constructed on native (the WASM sign-in path is `auth.js`).
    SignIn { paste_uri: String, show_paste: bool },
    /// Online play by contact — paste an invite link or an npub. Native only:
    /// the web build is the anonymous local taster and has no contacts book.
    #[cfg(not(target_arch = "wasm32"))]
    AddFriend { input: String, error: Option<String> },
    /// A pasted Keep link for a Keep that is not in My Keeps: asked before anything is
    /// dialled, so a link from any app cannot make the game connect and show the player's
    /// identity to its maker without a yes. Native only. (`Invite`'s `Debug` hides the secret.)
    #[cfg(not(target_arch = "wasm32"))]
    JoinKeep { invite: crate::keeps::Invite },
    /// Web-only: an explainer popup for a capability that lives in the native
    /// desktop app (Stash / Join / Host). The web build is a local sandbox, so
    /// these are shown as clickable explainers that point at the free app.
    /// Capability framing — see the legal-positioning note in
    /// docs/superpowers/specs/2026-06-27-web-local-sandbox-design.md.
    #[cfg(target_arch = "wasm32")]
    DesktopOnly(AppFeature),
}

/// A capability that exists only in the downloadable native app, surfaced on the
/// web build as a clickable explainer pointing at the desktop app. Cross-platform
/// + the copy is unit-tested so the **legally-careful wording** can't silently
///   drift: capability/tier framing only — never "bypass the rules", never
///   earning-to-minors, never unrestricted stranger contact. (We're describing what
///   the *software* does, which is lawful; we are NOT offering a way around the law.)
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppFeature {
    Stash,
    Join,
    Host,
}

/// Broken-up, readable copy for an [`AppFeature`] explainer popup — a title, a
/// one-line lead, a few short points, and a CTA note. Kept structured (not one
/// dense paragraph) so the popup reads nicely.
#[allow(dead_code)]
struct AppFeatureCopy {
    title: &'static str,
    lead: &'static str,
    points: &'static [&'static str],
    cta_note: &'static str,
}

/// Copy for an [`AppFeature`] explainer popup. The wording is pinned by
/// `app_feature_copy_is_capability_framed` so the legal positioning holds:
/// capability framing only — never "bypass", earning-to-minors, or unrestricted
/// stranger contact.
#[allow(dead_code)]
fn app_feature_copy(feature: AppFeature) -> AppFeatureCopy {
    match feature {
        AppFeature::Stash => AppFeatureCopy {
            title: "☁  Stash — your private cloud",
            lead: "Keep your worlds safe, and take them anywhere.",
            points: &[
                "Back up your worlds and open them on any device.",
                "Share your creations with other players.",
                "Your keys, your storage — held by you, not us.",
            ],
            cta_note: "Stash lives in the free desktop app.",
        },
        AppFeature::Join => AppFeatureCopy {
            title: "🎮  Multiplayer — join a world",
            lead: "Play in other players' worlds.",
            points: &[
                "Hop into worlds other people are running.",
                "The app handles sign-in and safety.",
                "The web version is a solo sandbox.",
            ],
            cta_note: "Multiplayer lives in the free desktop app.",
        },
        AppFeature::Host => AppFeatureCopy {
            title: "🖧  Multiplayer — host a world",
            lead: "Run your own world for others to join.",
            points: &[
                "Host a world and invite other players in.",
                "The app handles sign-in and safety.",
                "The web version is a solo sandbox.",
            ],
            cta_note: "Hosting lives in the free desktop app.",
        },
    }
}

/// Action returned from the menu each frame.
pub enum MenuAction {
    None,
    /// Load a world by folder name + a spawn-location preference. Default
    /// preserves the saved position; NearVillage teleports to the nearest
    /// village; AtOrigin drops at (0, surface, 0).
    LoadWorld(String, crate::spawn_pref::SpawnPref),
    RefreshWorlds,           // re-scan worlds directory
    HostWorld(String),       // Host a world for LAN play (folder_name)
    /// Join by what the Join dialog's field holds: a room code (native, over
    /// Fray) or a dedicated server's address (`ws://`, `wss://`, `axenstax://`).
    JoinGame(String),
    /// Native: join a room picked from "Games on this network" (Fray's LAN beacon).
    #[cfg(not(target_arch = "wasm32"))]
    JoinLanRoom(decented_net::LanRoom),
    /// Spec 40 (The Workshop) — open the player's Workshop: a blank/void authoring
    /// space, created on first entry, then loaded like any saved world.
    EnterWorkshop,
    /// Spec 40 (The Workshop) — enter the Workshop, then wipe the room back to the
    /// void floor (blocks + in-progress projects), KEEPING the reskin catalogue.
    /// The Workshop isn't a lobby world card, so this is its delete/reset path.
    ResetWorkshop,
    /// Stash Column (Prague) — "Play" on a scenario card: create + enter a fresh
    /// per-def arena world (seeded by `def.arena_seed`) and start the scenario.
    /// The def is carried by value (boxed — it's the largest field in this
    /// enum by a wide margin, so boxing keeps every other `MenuAction` variant
    /// from paying for its size) — embedded for the official games, or
    /// downloaded+decoded from a browsed `scenario` Beacon item. The game loop
    /// owns the create-world → load → start lifecycle.
    PlayScenario { def: Box<crate::scenario::ScenarioDef> },
    /// Stash Column (Prague) — "Adopt" on a skin card: the downloaded Workshop
    /// override-set blob bytes, applied to the next world the player enters
    /// (the override registry lives on a World, so the lobby can't apply it now).
    /// Only constructed on wasm32 (`draw_stash_column`'s async-download drain
    /// is web-only), invisible to a native `cargo clippy` run.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    AdoptSkin { bytes: Vec<u8> },
    /// Campaign G — export the personal-best ghost of this race to an
    /// `.axeghost` file (native Save-As / web download).
    ExportGhost { trial_id: String },
    /// Campaign G — pick an `.axeghost` file; the file names its own race, so
    /// one button serves every row. Stored as that race's rival ghost.
    ImportGhost,
    /// CP12 (CP10 review MEDIUM-1): the lobby's Quit. Ends the process only
    /// through the game's one exit function (`GameState::exit_app`).
    #[cfg(not(target_arch = "wasm32"))]
    Quit,
    /// CP12: the self-updater's Restart: the exit function first (its
    /// handover grace included), then the relaunch.
    #[cfg(not(target_arch = "wasm32"))]
    RestartForUpdate,
    /// Online play by contact — host this world for friends in other houses.
    /// Native only.
    #[cfg(not(target_arch = "wasm32"))]
    HostWorldOnline(String),
    /// Call a contact who is hosting. `persona_hex` is internal; the display
    /// name is what any failure message will use.
    #[cfg(not(target_arch = "wasm32"))]
    JoinContact {
        persona_hex: String,
        display_name: String,
    },
    /// A pasted invite link: calls at once and adds nobody.
    #[cfg(not(target_arch = "wasm32"))]
    JoinByInvite(String),
    /// A pasted npub: adds a contact to call later.
    #[cfg(not(target_arch = "wasm32"))]
    AddFriendNpub(String),
    /// Join a Keep by its invite: from the confirm that follows a pasted link, or a row
    /// of My Keeps (no confirm: the Keep is already one the player joined).
    #[cfg(not(target_arch = "wasm32"))]
    JoinKeep(crate::keeps::Invite),
}

/// One typed item in the Stash Column — a browsed (or embedded) Beacon item the
/// player can act on. A `Scenario` is **Played** (launches its arena); a `Skin`
/// is **Adopted**. Embedded official scenarios carry their `def` in hand so they
/// launch with the network off; browsed items download their blob on action.
#[derive(Clone, Debug)]
// On native only the two embedded official games exist; the fields are read by the web look-up.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct StashItem {
    /// Display label (the Beacon item name / def display-name).
    pub name: String,
    /// Author npub (NIP-19) for display, or a friendly label for embedded items.
    pub author_npub: String,
    /// Skin → Adopt, Scenario → Play.
    pub kind: crate::open_stash::StashItemKind,
    /// Blossom blob hash to download on action. Empty for embedded items.
    pub blob_hash: String,
    /// Present for EMBEDDED official scenarios — Play needs no network.
    pub embedded_def: Option<crate::scenario::ScenarioDef>,
}

/// The pinned, always-present "AxeNStax Official" scenarios, sourced from the
/// EMBEDDED defs (zero network — the booth's offline floor). A live browse of
/// the real AxeNStax npub layers over this seed; these are the guaranteed
/// fallback so both core games launch even with the conference wifi down.
pub fn official_stash_items() -> Vec<StashItem> {
    [
        crate::scenario::hash_dash_def(),
        crate::scenario::satori_rush_def(),
    ]
        .into_iter()
        .map(|def| StashItem {
            name: def.display_name.clone(),
            author_npub: "AxeNStax Official".to_string(),
            kind: crate::open_stash::StashItemKind::Scenario,
            blob_hash: String::new(),
            embedded_def: Some(def),
        })
        .collect()
}

/// Which lobby page shows (home page overhaul spec §10). The Trials gallery
/// keeps its own open flag and draws over any page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LobbyPage {
    #[default]
    Home,
    Worlds,
    PlayTogether,
    /// "More to play": the official games (and, on the web, a creator look-up).
    Stash,
}

/// What Escape (the Android Back key) does in the lobby, decided by [`lobby_escape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyEscape {
    /// A bare Home page: quit, as the lobby always has.
    Quit,
    /// A dialog was open and is now closed.
    ClosedDialog,
    /// The page (or the gallery, or an open menu) handles it in its own draw:
    /// back one page, never quit.
    LeaveToPage,
}

/// Escape in the lobby. A dialog closes first; the Trials gallery, an inner
/// page or an open ⋯ / ▾ menu then take it in their own draw (which got the
/// same key through egui), so it steps back and never quits; only a bare Home
/// page quits. A closed dialog marks the key spent, so the page under it does
/// not also step back on the same press.
pub fn lobby_escape(menu: &mut MenuState) -> LobbyEscape {
    if !matches!(menu.dialog, MenuDialog::None) {
        menu.dialog = MenuDialog::None;
        menu.escape_spent = true;
        return LobbyEscape::ClosedDialog;
    }
    if menu.trials_gallery.is_open() || menu.page != LobbyPage::Home || menu.popup_open {
        return LobbyEscape::LeaveToPage;
    }
    LobbyEscape::Quit
}

/// The Create dialog with its defaults: survival, commands on, Stash off,
/// normal terrain on grass, the day cycle and mobs on.
pub(crate) fn new_create_dialog() -> MenuDialog {
    MenuDialog::Create {
        name: String::new(),
        seed: String::new(),
        creative: false,
        commands_enabled: true,
        cloud_save: false,
        world_type: "normal".to_string(),
        ground: "grass".to_string(),
        water_depth: 3,
        time_lock: "cycle".to_string(),
        mobs_enabled: true,
    }
}

/// Persistent menu state held in GameMode::Menu.
pub struct MenuState {
    pub worlds: Vec<WorldEntry>,
    pub selected: Option<usize>,
    pub dialog: MenuDialog,
    /// Spawn-location preference for the next world load. Sits above the
    /// world list as a small dropdown; defaults to `Default` so existing
    /// behaviour (spawn at saved position) is unchanged unless the player
    /// deliberately picks otherwise. Per-Create-dialog override lives in
    /// `MenuDialog::Create` so new worlds carry their own pref.
    pub spawn_pref: crate::spawn_pref::SpawnPref,
    /// WASM: async IndexedDB world-list fetch state.
    #[cfg(target_arch = "wasm32")]
    pub local_fetch: Option<std::rc::Rc<std::cell::RefCell<Option<Result<Vec<crate::wasm_save::LocalWorldEntry>, String>>>>>,
    /// WASM: whether the local world list has been loaded.
    #[cfg(target_arch = "wasm32")]
    pub local_loaded: bool,
    /// WASM: async cloud world-list fetch (Stash manifest). Merged into `worlds`
    /// after the local list loads, so cloud-only worlds (saved on another
    /// machine) appear here too.
    #[cfg(target_arch = "wasm32")]
    pub cloud_fetch: Option<std::rc::Rc<std::cell::RefCell<Option<Result<Vec<crate::wasm_save::CloudWorldEntry>, String>>>>>,
    /// WASM: whether the cloud list has been merged in this session.
    #[cfg(target_arch = "wasm32")]
    pub cloud_loaded: bool,
    /// WASM: blob hashes for cloud-only worlds, keyed by folder_name, so opening
    /// one downloads + decrypts the right blob instead of hitting IndexedDB.
    #[cfg(target_arch = "wasm32")]
    pub cloud_blob_hashes: std::collections::HashMap<String, String>,
    /// WASM: names of ALL worlds present in the player's cloud Stash manifest
    /// (both cloud-only and those also held locally). Drives the world-card
    /// Stash traffic light: a Stash-on world in this set is confirmed stashed
    /// (green); on but absent is unconfirmed (amber). Empty until the cloud
    /// list resolves, and on native (no cloud).
    #[cfg(target_arch = "wasm32")]
    pub cloud_world_names: std::collections::HashSet<String>,
    /// WASM: async world load from IndexedDB. `Ok(Some(bytes))` = found + packed,
    /// `Ok(None)` = no record (new world — fresh terrain), `Err` = IDB failure.
    #[cfg(target_arch = "wasm32")]
    pub local_load: Option<std::rc::Rc<std::cell::RefCell<Option<Result<Option<Vec<u8>>, String>>>>>,
    /// WASM: name of the world being loaded.
    #[cfg(target_arch = "wasm32")]
    pub loading_world_name: Option<String>,
    /// WASM: async "Restore from file" import. `Ok(name)` = imported world's
    /// final stored name; `Err("cancelled")` = picker dismissed (no-op);
    /// other `Err` = a friendly message to show.
    #[cfg(target_arch = "wasm32")]
    pub import_result: Option<std::rc::Rc<std::cell::RefCell<Option<Result<String, String>>>>>,
    /// WASM: status line under the action bar (e.g. "Restored: <name>" or an
    /// import error). Cleared when a new import starts.
    #[cfg(target_arch = "wasm32")]
    pub import_status: Option<String>,
    /// WASM: in-flight "Export everything" (`.axeprofile`) job. `Ok(msg)` = a
    /// bundle was handed to the browser to download; `Err(msg)` = a friendly
    /// reason it couldn't be. Shares the `import_status` line.
    #[cfg(target_arch = "wasm32")]
    pub profile_export: Option<std::rc::Rc<std::cell::RefCell<Option<Result<String, String>>>>>,
    /// Native: the LAN beacon browser behind the Join dialog's "Games on this
    /// network" list (gap-audit T2-10, Fray's beacon). Opened when that dialog
    /// first draws and dropped when it closes, so the beacon port is not held
    /// for the whole session.
    #[cfg(not(target_arch = "wasm32"))]
    pub lan: Option<decented_net::LanBrowser>,
    /// What the player has typed in Play together's room-code field (a room
    /// code, a Keep link or a server address). Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub join_code: String,
    /// Native: status line shown after the Export/Import buttons (last
    /// operation result — success or error). Cleared on the next export/import.
    #[cfg(not(target_arch = "wasm32"))]
    pub transfer_status: Option<String>,
    /// Native (CP11): one plain line per handover import still awaiting its
    /// Committed ("Waiting for <npub> to finish handing this world over").
    /// Not a world card: the import isn't under the worlds folder and can't
    /// be opened until Activated.
    #[cfg(not(target_arch = "wasm32"))]
    pub awaiting_imports: Vec<String>,
    /// CP20 (CP18 review MEDIUM-1): when the lobby promotes again (CP22:
    /// once per activation, and on the retry cadence only while held).
    #[cfg(not(target_arch = "wasm32"))]
    handover_poll: crate::handover::live::promote::LobbyPoll,
    /// Native: in-flight OS file-dialog channel. `Some` while the worker thread
    /// is running (or has a result waiting); cleared by `poll_dialog` once the
    /// result has been consumed or the channel disconnects.
    #[cfg(not(target_arch = "wasm32"))]
    pending_dialog: Option<std::sync::mpsc::Receiver<crate::native_file_dialog::FileDialogResult>>,
    /// 2026-05-21 — was the paginated-grid world list's page index; the grid
    /// was reverted to a single-column vertical scroll the same day (kids
    /// missed the scroll affordance — see CLAUDE.md), so this no longer
    /// drives anything. Kept at 0.
    pub world_page: usize,

    // ─── Stash Column (Prague delivery) ───
    /// The npub (or 64-hex) the player typed to browse a stash. Cross-platform
    /// so the column renders on native (`--shot-lobby`); the live browse itself
    /// is a PWA path.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub stash_input: String,
    /// Typed items returned by the last browse. The pinned "AxeNStax Official"
    /// section is separate + always present (see `official_stash_items`).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub stash_items: Vec<StashItem>,
    /// Status / error line under the input ("Browsing…", "No items", an error,
    /// or the native "live browse is a PWA feature" note).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub stash_status: Option<String>,
    /// WASM: async Beacon browse result for the column (typed items).
    #[cfg(target_arch = "wasm32")]
    pub stash_browse: Option<std::rc::Rc<std::cell::RefCell<Option<Result<Vec<StashItem>, String>>>>>,
    /// WASM: async Beacon download for a Play/Adopt action, tagged with the kind
    /// so the resolved bytes route to the right MenuAction.
    #[cfg(target_arch = "wasm32")]
    pub stash_action_dl: Option<std::rc::Rc<std::cell::RefCell<Option<Result<(crate::open_stash::StashItemKind, Vec<u8>), String>>>>>,
    /// True while a "Sync Stash" batch is in flight. Set/read only by
    /// `poll_cloud_worlds`; no "Sync Stash" button was ever built to drive it
    /// (the start/cancel JS side was removed 2026-07-09 as confirmed dead —
    /// see docs/superpowers/specs/2026-07-09-give-aliases-and-dead-web-stash-code.md).
    #[allow(dead_code)]
    pub stash_syncing: bool,
    /// The latest "Sync Stash" status/result line. No longer shown (the home
    /// page overhaul removed the dead line); still written by the web poll.
    #[allow(dead_code)]
    pub stash_sync_msg: Option<String>,
    /// My Servers (Spec A) — the player's saved server list, loaded once.
    /// Native only: its card sits on the Play together page,
    /// which the web taster never mounts (no multiplayer there).
    #[cfg(not(target_arch = "wasm32"))]
    pub my_servers: crate::my_servers::MyServers,
    /// My Keeps: the Keeps the player has been let into, newest first. Read when the lobby
    /// is built (the lobby is built each time the player comes back to it) and after a Forget.
    #[cfg(not(target_arch = "wasm32"))]
    pub keeps: Vec<crate::keeps::Keep>,
    /// The Keep (its host key) whose Forget has had its first tap.
    #[cfg(not(target_arch = "wasm32"))]
    pub keep_forget_armed: Option<String>,
    /// The compact Join Game card shows all My Keeps (the "More Keeps" button was tapped)
    /// rather than the newest few. Lasts while the dialog is open.
    #[cfg(not(target_arch = "wasm32"))]
    pub keeps_expanded: bool,
    /// When the "Join this Keep?" confirm first drew; Join does nothing until
    /// `keeps::CONFIRM_ARM` has passed, so a tap already under way (a link from another app
    /// brings the game to the front) cannot accept it. Cleared when the confirm is gone.
    #[cfg(not(target_arch = "wasm32"))]
    pub keep_confirm_shown: Option<web_time::Instant>,
    /// Why the player just landed back in the lobby when it wasn't their
    /// choice ("Disconnected from host", a kick, a Trial that couldn't open).
    /// Shown as a banner until dismissed.
    pub notice: Option<String>,

    /// The Trials gallery (full-screen shelves + hero view); closed by default.
    /// Opened by the "Trials" tile in the world list. See `trials_gallery_ui`.
    pub trials_gallery: crate::trials_gallery_ui::GalleryState,
    /// Trials gallery — which trial's "How it works" pop-up is open (`"race:<id>"`
    /// / `"ch:<token>"`), or `None`. A centred modal with the detailed how-to +
    /// Cancel / Play.
    pub trial_info: Option<String>,
    /// Online play by contact — the local address book, loaded once for the
    /// Friends column. Native only. Task 18 refreshes it (and pushes the fresh
    /// book into a running `OnlineHost`) whenever the mirror changes.
    #[cfg(not(target_arch = "wasm32"))]
    pub contacts: Vec<crate::contacts::Contact>,
    /// A copy of the player's "Your relays" list (`GraphicsSettings.online_relays`),
    /// refreshed by the caller every frame before drawing — the sign-in QR and
    /// Signet contacts pairing read it from here. Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub relays: Vec<String>,
    /// The "Relays" window is open (sign-in dialog button). The caller draws it
    /// with `relays_ui::draw_window`. Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub show_relays: bool,
    /// The lobby's Settings panel is open (header "Settings" button). The caller
    /// draws `draw_settings_panel` over the lobby; while set, `draw_main_menu`
    /// paints only the background so nothing underneath takes clicks. All targets.
    pub show_settings: bool,
    /// Which lobby page shows. Home when the lobby is built.
    pub page: LobbyPage,
    /// Set when `lib.rs` spent an Escape closing a dialog or Settings, so the
    /// page drawn on the next frame does not also step back on that press.
    pub escape_spent: bool,
    /// A ⋯ / ▾ menu (or the Start at list) was open at the end of the last
    /// frame: Escape closes it rather than leaving the page or quitting.
    pub popup_open: bool,
}

impl MenuState {
    /// Show `page` (its world detail, if any, stays as it was).
    pub fn go(&mut self, page: LobbyPage) {
        self.page = page;
    }

    /// Back one step: world detail to Your worlds, any inner page to Home.
    pub fn step_back(&mut self) {
        if self.page == LobbyPage::Worlds && self.selected.is_some() {
            self.selected = None;
        } else {
            self.page = LobbyPage::Home;
        }
    }

    /// Native: an OS file dialog is in flight (the import / export entries grey out).
    pub(crate) fn dialog_pending(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pending_dialog.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Native: open an OS file picker (one at a time), saying so on the status line.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_file_dialog(&mut self, req: crate::native_file_dialog::FileDialogRequest) {
        if self.pending_dialog.is_some() {
            return;
        }
        self.transfer_status = Some("Opening the file picker…".to_string());
        self.pending_dialog = Some(crate::native_file_dialog::spawn_dialog(req));
    }

    /// Native: bring in every `.axeworld` in the transfer folder, then re-list.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn import_transfer_folder(&mut self) {
        let msg = native_import_worlds();
        self.worlds = save::list_world_entries();
        self.selected = None;
        self.transfer_status = Some(msg);
    }

    /// Web: "Restore from a file" (one `.axeworld` into this browser).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn start_web_import(&mut self) {
        self.import_status = None;
        self.import_result = Some(kick_off_import());
    }

    /// Web: "Save everything to a file" (every world plus the Trials records).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn start_web_profile_export(&mut self) {
        self.import_status = Some("Packing your profile…".to_string());
        self.profile_export = Some(kick_off_profile_export());
    }

    pub fn new() -> Self {
        // CP18: a handed-over world that has finished arriving moves into
        // the worlds folder first, so it lists below. CP20: the activation
        // count is read before that pass, so one landing during it is
        // still acted on by `poll_handovers`.
        #[cfg(not(target_arch = "wasm32"))]
        let activations_at_entry = crate::handover::live::promote::activations();
        #[cfg(not(target_arch = "wasm32"))]
        let held_at_entry = crate::handover_game::promote_activated_imports().held;
        #[cfg(not(target_arch = "wasm32"))]
        let worlds = save::list_world_entries();
        #[cfg(target_arch = "wasm32")]
        let worlds = Vec::new(); // Populated async below

        // `mut` only on wasm32 — the block below reassigns local_fetch/cloud_fetch
        // there; native never mutates `state` after construction (same pattern as
        // `cloud_buf` above, mirrored cfg direction).
        #[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
        let mut state = Self {
            worlds,
            selected: None,
            dialog: MenuDialog::None,
            spawn_pref: crate::spawn_pref::SpawnPref::Default,
            #[cfg(target_arch = "wasm32")]
            local_fetch: None,
            #[cfg(target_arch = "wasm32")]
            local_loaded: false,
            #[cfg(target_arch = "wasm32")]
            cloud_fetch: None,
            #[cfg(target_arch = "wasm32")]
            cloud_loaded: false,
            #[cfg(target_arch = "wasm32")]
            cloud_blob_hashes: std::collections::HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            cloud_world_names: std::collections::HashSet::new(),
            #[cfg(target_arch = "wasm32")]
            local_load: None,
            #[cfg(target_arch = "wasm32")]
            loading_world_name: None,
            #[cfg(target_arch = "wasm32")]
            import_result: None,
            #[cfg(target_arch = "wasm32")]
            import_status: None,
            #[cfg(target_arch = "wasm32")]
            profile_export: None,
            #[cfg(not(target_arch = "wasm32"))]
            lan: None,
            #[cfg(not(target_arch = "wasm32"))]
            join_code: String::new(),
            #[cfg(not(target_arch = "wasm32"))]
            transfer_status: None,
            #[cfg(not(target_arch = "wasm32"))]
            awaiting_imports: crate::handover_game::awaiting_import_statuses(),
            #[cfg(not(target_arch = "wasm32"))]
            handover_poll: crate::handover::live::promote::LobbyPoll::new(activations_at_entry, held_at_entry, std::time::Instant::now()),
            #[cfg(not(target_arch = "wasm32"))]
            pending_dialog: None,
            world_page: 0,
            stash_input: String::new(),
            stash_items: Vec::new(),
            stash_status: None,
            #[cfg(target_arch = "wasm32")]
            stash_browse: None,
            #[cfg(target_arch = "wasm32")]
            stash_action_dl: None,
            stash_syncing: false,
            stash_sync_msg: None,
            #[cfg(not(target_arch = "wasm32"))]
            my_servers: crate::my_servers::MyServers::load(),
            #[cfg(not(target_arch = "wasm32"))]
            keeps: crate::keeps::load_list(&crate::keeps::path()),
            #[cfg(not(target_arch = "wasm32"))]
            keep_forget_armed: None,
            #[cfg(not(target_arch = "wasm32"))]
            keeps_expanded: false,
            #[cfg(not(target_arch = "wasm32"))]
            keep_confirm_shown: None,
            notice: None,
            trials_gallery: Default::default(),
            trial_info: None,
            #[cfg(not(target_arch = "wasm32"))]
            contacts: crate::contacts::load_local_book(),
            #[cfg(not(target_arch = "wasm32"))]
            relays: crate::graphics_settings::default_online_relays(),
            #[cfg(not(target_arch = "wasm32"))]
            show_relays: false,
            show_settings: false,
            page: LobbyPage::Home,
            escape_spent: false,
            popup_open: false,
        };

        #[cfg(target_arch = "wasm32")]
        {
            state.local_fetch = Some(kick_off_local_fetch());
            state.cloud_fetch = kick_off_cloud_fetch();
        }

        state
    }

    /// CP20 (CP18 review MEDIUM-1, DECIDED): called every lobby frame. When
    /// a recipient import has reached Activated since the lobby last looked,
    /// promote and re-list in place (selection, open dialog and page kept);
    /// while a pass finds an Activated import whose lock is still held (its
    /// holder lets go a moment after Activated), try again twice a second.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll_handovers(&mut self) {
        let now = std::time::Instant::now();
        if !self.handover_poll.due(crate::handover::live::promote::activations(), now) {
            return;
        }
        let promoted = crate::handover_game::promote_activated_imports();
        let released = self.handover_poll.ran(&promoted, now);
        let awaiting = crate::handover_game::awaiting_import_statuses();
        // CP22 (CP21 review LOW-1): re-list whenever the waiting lines
        // changed too (another process may have moved the world in), and
        // CP24 (CP22 review LOW-1) once a held import is held no more.
        if promoted.moved.is_empty() && awaiting == self.awaiting_imports && !released {
            return;
        }
        self.awaiting_imports = awaiting;
        let selected = self.selected.and_then(|i| self.worlds.get(i)).map(|e| e.folder_name.clone());
        self.worlds = save::list_world_entries();
        self.selected = selected.and_then(|f| self.worlds.iter().position(|e| e.folder_name == f));
    }

    pub fn refresh(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::handover_game::promote_activated_imports();
            self.worlds = save::list_world_entries();
            self.awaiting_imports = crate::handover_game::awaiting_import_statuses();
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.local_fetch = Some(kick_off_local_fetch());
            self.local_loaded = false;
            self.cloud_fetch = kick_off_cloud_fetch();
            self.cloud_loaded = false;
            self.cloud_blob_hashes.clear();
            self.cloud_world_names.clear();
            // Drop any in-flight Stash Column fetch so a browse/download kicked
            // before the refresh can't resolve later and inject stale items.
            self.stash_browse = None;
            self.stash_action_dl = None;
        }
        self.selected = None;
        self.dialog = MenuDialog::None;
        self.world_page = 0;
    }

    /// Native: drain the in-flight OS file-dialog channel. Call each frame while
    /// the lobby is shown. Routes [`FileDialogResult`] variants to the
    /// `transfer_status` line and, for world imports, refreshes the world list.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll_dialog(&mut self) {
        use crate::native_file_dialog::FileDialogResult;
        use std::sync::mpsc::TryRecvError;
        let Some(ref rx) = self.pending_dialog else { return; };
        match rx.try_recv() {
            Ok(result) => {
                // WorldToImport needs `&mut self` to refresh the world list —
                // handle it here rather than in the stateless `route_dialog_result`
                // helper.  All other variants delegate to the helper.
                match result {
                    FileDialogResult::WorldToImport(bytes) => {
                        let (status, imported_ok) = import_picked_world(&bytes);
                        self.transfer_status = Some(status);
                        if imported_ok {
                            self.worlds = save::list_world_entries();
                            // The import re-sorts the list (newest first), so any
                            // prior selection index is stale — clear it, matching
                            // the other world-list refresh paths.
                            self.selected = None;
                        }
                    }
                    // "Take your worlds to native" — a whole `.axeprofile`
                    // bundle. Same shape as a world import: refresh the list
                    // when anything landed.
                    FileDialogResult::ProfileToImport(bytes) => {
                        let (status, imported_ok) = import_picked_profile(&bytes);
                        self.transfer_status = Some(status);
                        if imported_ok {
                            self.worlds = save::list_world_entries();
                            self.selected = None;
                        }
                    }
                    other => {
                        route_dialog_result(other, &mut self.transfer_status);
                    }
                }
                self.pending_dialog = None;
            }
            Err(TryRecvError::Disconnected) => {
                // Worker finished without sending (shouldn't happen — it always
                // sends exactly one result) — clear the slot to avoid polling a
                // dead channel forever.
                self.pending_dialog = None;
            }
            Err(TryRecvError::Empty) => {
                // Worker still running — try again next frame.
            }
        }
    }

    /// WASM: poll the async "Restore from file" import. Call each frame.
    /// On success, refreshes the world list and sets a status line; on a
    /// cancelled picker, does nothing; on a bad file, shows a friendly message.
    #[cfg(target_arch = "wasm32")]
    pub fn poll_import(&mut self) {
        self.poll_profile_export();
        let Some(ref slot) = self.import_result else { return; };
        let result = slot.borrow_mut().take();
        if let Some(result) = result {
            self.import_result = None;
            match result {
                Ok(name) => {
                    self.import_status = Some(format!("Restored: {name}"));
                    // Re-fetch the world list so the imported world appears.
                    self.local_fetch = Some(kick_off_local_fetch());
                    self.local_loaded = false;
                }
                Err(msg) if msg == "cancelled" => {
                    // Picker dismissed — no-op, no toast.
                }
                Err(msg) => {
                    self.import_status = Some(msg);
                }
            }
        }
    }

    /// WASM: poll the async "Export everything" job. Call each frame (it is
    /// driven from `poll_import`). Reports into the same status line as the
    /// import flow — one place to look after either action.
    #[cfg(target_arch = "wasm32")]
    fn poll_profile_export(&mut self) {
        let Some(ref slot) = self.profile_export else { return; };
        let Some(result) = slot.borrow_mut().take() else { return; };
        self.profile_export = None;
        self.import_status = Some(match result {
            Ok(msg) => msg,
            Err(msg) => msg,
        });
    }

    /// WASM: poll for async world-list result. Call each frame.
    #[cfg(target_arch = "wasm32")]
    pub fn poll_local_worlds(&mut self) {
        self.poll_import();
        // Cloud poll runs EVERY frame, not just while the local list is
        // loading: it consumes the JS "Sync Stash" terminal status (resets the
        // Cancel button), services mark_cloud_dirty → manifest refetch (the
        // amber→green Stash light), merges late-resolving cloud lists, and
        // refreshes the queued-message count. It was previously below the
        // early-return, so all of that starved the moment local_loaded went
        // true — first observed on the first-ever sync to reach `done`
        // (2026-06-11, stash-sync investigation).
        self.poll_cloud_worlds();
        if self.local_loaded { return; }
        let Some(ref slot) = self.local_fetch else { return; };
        let result = slot.borrow_mut().take();
        if let Some(result) = result {
            match result {
                Ok(entries) => {
                    // Sync Rust-side meta cache so `load_world_meta` returns the
                    // latest version during save rename/edit flows.
                    crate::save::WASM_META_CACHE.with(|c| {
                        let mut cache = c.borrow_mut();
                        for e in &entries {
                            cache.insert(e.name.clone(), save::WorldMeta {
                                display_name: e.display_name.clone(),
                                description: e.description.clone(),
                                game_mode: e.game_mode.clone(),
                                created_at: String::new(),
                                icon: None,
                                pure_survival: e.game_mode == "survival",
                                ever_creative: e.game_mode == "creative",
                                cheats_used: false,
                                difficulty: e.difficulty.clone(),
                                difficulty_history: Vec::new(),
                                forked_from: None,
                                version: 0,
                                genesis_block_found: false,
                                commands_enabled: true,
                                explosives_enabled: true,
            fire_spread_enabled: true,
                                cloud_save: e.cloud_save,
                                has_seen_license_onboarding: false,
                                // Display-only list cache; the real seed comes
                                // from the unpacked blob meta on load.
                                seed: 42,
                                total_work: 0,
                                total_ticks: 0,
                                genesis_found_at_tick: None,
                                scenario_def: None,
                                is_workshop: false,
                                world_override: None,
                                world_type: "normal".to_string(),
                                ground: "grass".to_string(),
                                water_depth: 3,
                                time_lock: "cycle".to_string(),
                                mobs_enabled: true,
                                keep_inventory: false,
                                // Seam B: display-only list cache — the real owner
                                // (if any) comes from the unpacked blob meta on load.
                                owner_pubkey: None,
                                // Display-only cache; real publish state from the blob on load.
                                published_to: None,
                                satoshi_enabled: false,
                                pop_secret: None,
                                // Display-only, like `seed`: the real generator comes from the blob meta.
                                worldgen_version: crate::biome::LEGACY_GENERATOR,
                            });
                        }
                    });

                    self.worlds = entries.into_iter()
                        // The Workshop is a singleton authoring space reached ONLY
                        // via the "The Workshop" button — never a lobby world card.
                        .filter(|e| e.name != crate::workshop::WORKSHOP_FOLDER)
                        // Trial arenas are reached from Trials, never listed as worlds.
                        .filter(|e| !save::is_arena_folder(&e.name))
                        .map(|e| WorldEntry {
                        folder_name: e.name.clone(),
                        meta: save::WorldMeta {
                            display_name: e.display_name.clone(),
                            description: e.description.clone(),
                            game_mode: e.game_mode.clone(),
                            created_at: String::new(),
                            icon: None,
                            pure_survival: e.game_mode == "survival",
                            ever_creative: e.game_mode == "creative",
                            cheats_used: false,
                            difficulty: e.difficulty.clone(),
                            difficulty_history: Vec::new(),
                            forked_from: None,
                            version: 0,
                            genesis_block_found: false,
                            commands_enabled: true,
                            explosives_enabled: true,
            fire_spread_enabled: true,
                            cloud_save: e.cloud_save,
                            has_seen_license_onboarding: false,
                            seed: 42,
                            total_work: 0,
                            total_ticks: 0,
                            genesis_found_at_tick: None,
                            scenario_def: None,
                            is_workshop: false,
                            world_override: None,
                            world_type: "normal".to_string(),
                            ground: "grass".to_string(),
                            water_depth: 3,
                            time_lock: "cycle".to_string(),
                            mobs_enabled: true,
                            keep_inventory: false,
                            // Seam B: display-only list cache (real owner from the blob on load).
                            owner_pubkey: None,
                            published_to: None,
                            satoshi_enabled: false,
                            pop_secret: None,
                            // Display-only, like `seed`: the real generator comes from the blob meta.
                            worldgen_version: crate::biome::LEGACY_GENERATOR,
                        },
                        size_bytes: e.size,
                        cloud_only: false,
                    }).collect();
                    log::info!("Loaded {} local worlds", self.worlds.len());
                }
                Err(e) => {
                    log::warn!("Failed to list local worlds: {e}");
                }
            }
            self.local_loaded = true;
            self.local_fetch = None;
        }
        // (cloud poll moved to the top of this fn — see comment there)
    }

    /// WASM: poll the async cloud world-list and merge cloud-only worlds into
    /// the list. Only runs once the local list is in (so dedup-by-name works).
    /// A world that exists locally wins (local is authoritative); cloud-only
    /// worlds — saved on another machine — are appended and badged via
    /// `cloud_blob_hashes` so opening one downloads + decrypts the blob.
    ///
    /// Inert-by-design as of 2026-07-09: `kick_off_cloud_fetch` short-circuits
    /// to `None` because `cloud_available()` is always `false` on web (no
    /// signer reachable since web login retirement — the push/upload side of
    /// this system was removed as confirmed dead; see
    /// docs/superpowers/specs/2026-07-09-give-aliases-and-dead-web-stash-code.md).
    /// Left in place rather than excavated further: it shares `WorldEntry`
    /// construction with the live local world-list code below, so removing it
    /// risks the load-bearing path for marginal benefit on something already
    /// fully inert.
    #[cfg(target_arch = "wasm32")]
    pub fn poll_cloud_worlds(&mut self) {
        // Poll the JS "Sync Stash" batch (cloud.js owns the wait/upload/flush;
        // menu.rs just renders it). Update the status line, and on a terminal
        // state clear the in-flight flag + (on success) refresh the greens.
        #[derive(serde::Deserialize)]
        struct SyncStatus {
            state: String,
            message: String,
        }
        let raw = crate::wasm_save::sync_stash_status();
        if let Ok(st) = serde_json::from_str::<SyncStatus>(&raw) {
            if st.state != "idle" {
                self.stash_sync_msg = Some(st.message);
            }
            if matches!(st.state.as_str(), "done" | "cancelled" | "error") {
                if self.stash_syncing && st.state == "done" {
                    crate::wasm_save::mark_cloud_dirty();
                }
                self.stash_syncing = false;
            }
        }
        // A just-landed cloud save marks the manifest dirty — re-fetch so the world's
        // Stash light flips amber → green on its own (no manual lobby refresh needed).
        if crate::wasm_save::take_cloud_dirty() {
            self.cloud_loaded = false;
            self.cloud_world_names.clear();
            self.cloud_blob_hashes.clear();
            self.worlds.retain(|w| !w.cloud_only);
            self.cloud_fetch = kick_off_cloud_fetch();
        }
        if self.cloud_loaded || !self.local_loaded {
            return;
        }
        let Some(ref slot) = self.cloud_fetch else { return; };
        let result = slot.borrow_mut().take();
        if let Some(result) = result {
            match result {
                Ok(entries) => {
                    // Cloud manifest entries are keyed by the world's DISPLAY name
                    // (save.rs uploads under `meta.display_name`), so dedup against
                    // local display names — not folder names — or a local world's
                    // cloud copy shows up as a phantom duplicate card.
                    let have: std::collections::HashSet<String> =
                        self.worlds.iter().map(|w| w.meta.display_name.clone()).collect();
                    let mut added = 0;
                    for e in entries {
                        // Every stashed world (local-held or cloud-only) goes in
                        // the names set so the traffic light can show "confirmed
                        // stashed" (green) for local worlds too.
                        self.cloud_world_names.insert(e.name.clone());
                        if have.contains(&e.name) {
                            continue; // local copy wins
                        }
                        self.cloud_blob_hashes.insert(e.name.clone(), e.blob_hash.clone());
                        self.worlds.push(WorldEntry {
                            folder_name: e.name.clone(),
                            meta: save::WorldMeta {
                                display_name: e.name.clone(),
                                description: "📦 In your Stash — open it on any computer".to_string(),
                                game_mode: "survival".to_string(),
                                created_at: String::new(),
                                icon: None,
                                pure_survival: false,
                                ever_creative: false,
                                cheats_used: false,
                                difficulty: "normal".to_string(),
                                difficulty_history: Vec::new(),
                                forked_from: None,
                                version: 0,
                                genesis_block_found: false,
                                commands_enabled: true,
                                explosives_enabled: true,
            fire_spread_enabled: true,
                                cloud_save: true,
                                has_seen_license_onboarding: false,
                                seed: 42,
                                total_work: 0,
                                total_ticks: 0,
                                genesis_found_at_tick: None,
                                scenario_def: None,
                                is_workshop: false,
                                world_override: None,
                                world_type: "normal".to_string(),
                                ground: "grass".to_string(),
                                water_depth: 3,
                                time_lock: "cycle".to_string(),
                                mobs_enabled: true,
                                keep_inventory: false,
                                // Seam B: display-only list cache (real owner from the blob on load).
                                owner_pubkey: None,
                                published_to: None,
                                satoshi_enabled: false,
                                pop_secret: None,
                                // Display-only, like `seed`: the real generator comes from the blob meta.
                                worldgen_version: crate::biome::LEGACY_GENERATOR,
                            },
                            size_bytes: e.size,
                            cloud_only: true,
                        });
                        added += 1;
                    }
                    if added > 0 {
                        log::info!("Merged {added} cloud-only world(s)");
                    }
                }
                Err(e) => log::warn!("Failed to list cloud worlds: {e}"),
            }
            self.cloud_loaded = true;
            self.cloud_fetch = None;
        }
    }
}

/// Kick off `list_worlds_wasm` for the current pubkey. Returns a slot the
/// caller can poll via `poll_local_worlds`.
#[cfg(target_arch = "wasm32")]
fn kick_off_local_fetch() -> std::rc::Rc<std::cell::RefCell<Option<Result<Vec<crate::wasm_save::LocalWorldEntry>, String>>>> {
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let pubkey = crate::save::wasm_storage_key();
    let slot_clone = slot.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = crate::wasm_save::list_worlds_wasm(&pubkey).await;
        *slot_clone.borrow_mut() = Some(result);
    });
    slot
}

/// Kick off a cloud world-list fetch (Stash manifest from the relay), if cloud
/// save is available. Returns the poll slot, or `None` when cloud is off (no
/// capable signer / not configured) — in which case the menu is local-only.
#[cfg(target_arch = "wasm32")]
fn kick_off_cloud_fetch() -> Option<std::rc::Rc<std::cell::RefCell<Option<Result<Vec<crate::wasm_save::CloudWorldEntry>, String>>>>> {
    if !crate::wasm_save::cloud_available() {
        return None;
    }
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot_clone = slot.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = crate::wasm_save::cloud_list_wasm().await;
        *slot_clone.borrow_mut() = Some(result);
    });
    Some(slot)
}

/// Deduplicate a candidate folder slug against already-known folder names —
/// the wasm counterpart of `save::sanitize_folder_name`'s native collision
/// handling (which walks `worlds_root()` on disk; wasm has no filesystem, so
/// this checks the in-memory, already-loaded world list instead). P6 audit:
/// "Create" with an existing name used to silently land in the OLD world
/// rather than making a new one, because the wasm sanitizer returned the
/// same slug unconditionally — same for the auto-name ("New World N")
/// colliding after an earlier world was deleted. Pure, so it's testable
/// without a wasm target.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn dedupe_folder_name(base: &str, existing: &[String]) -> String {
    if !existing.iter().any(|f| f == base) {
        return base.to_string();
    }
    // At most `existing.len()` folders can possibly collide with a
    // `{base}_i` candidate, so trying `existing.len() + 1` of them is
    // guaranteed (pigeonhole) to find a free one — no time-based fallback
    // needed, unlike native's disk-scan version.
    for i in 2..=existing.len() + 2 {
        let candidate = format!("{base}_{i}");
        if !existing.iter().any(|f| f == &candidate) {
            return candidate;
        }
    }
    unreachable!("pigeonhole: exhausted existing.len()+1 candidates against existing.len() collisions")
}

/// Kick off an IndexedDB metadata-only edit (the world "Edit" — rename /
/// re-describe — dialog), then re-list, writing the fresh list into the
/// returned slot (polled via `poll_local_worlds`). P6 audit: the previous
/// `save::save_world_meta` call only updated the in-memory `WASM_META_CACHE`
/// — on wasm the durable record lives in IndexedDB, and the very next
/// `RefreshWorlds` re-listed straight from there, silently reverting the
/// edit before the player's next look at the lobby.
///
/// Re-saves the EXISTING blob unchanged, with the new `meta_json` — safe
/// without decompressing/repacking the world archive, because
/// `world_store.js`'s `list()` only ever reads `game_mode`/`display_name`/
/// `description`/`difficulty`/`cloud_save` back off the record's `meta`
/// field (the lobby card fields); loading the world to actually play it
/// (`axenstax_load_world`) returns the blob alone and never touches this
/// field, so it stays purely a "what the lobby card shows" cache — exactly
/// what `meta` already is going into this call (a clone of the display-only
/// `WorldEntry.meta`, per the same BRIDGE noted where it's built above).
///
/// A world that was never actually saved to IndexedDB yet (edited in the
/// gap between Create and the first autosave) has no blob to re-save
/// against — that's not an error, there's simply nothing to update yet; the
/// in-memory cache (already updated by the caller) covers it until the
/// first real save writes the record.
#[cfg(target_arch = "wasm32")]
fn kick_off_local_meta_edit(
    folder_name: String,
    meta: save::WorldMeta,
) -> std::rc::Rc<std::cell::RefCell<Option<Result<Vec<crate::wasm_save::LocalWorldEntry>, String>>>>
{
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let pubkey = crate::save::wasm_storage_key();
    let slot_clone = slot.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let existing = crate::wasm_save::load_world_wasm(&pubkey, &folder_name).await?;
            if let Some(blob) = existing {
                let meta_json = serde_json::to_string(&meta)
                    .map_err(|e| format!("meta serialise: {e}"))?;
                crate::wasm_save::save_world_wasm(&pubkey, &folder_name, blob, meta_json).await?;
            } else {
                log::info!(
                    "world edit: '{folder_name}' has no IndexedDB record yet — nothing to \
                     re-save; the in-memory cache covers it until the first real save"
                );
            }
            crate::wasm_save::list_worlds_wasm(&pubkey).await
        }
        .await;
        *slot_clone.borrow_mut() = Some(result);
    });
    slot
}

/// Kick off an IndexedDB world delete, then re-list, writing the fresh
/// list into the returned slot (polled via `poll_local_worlds`). Chaining
/// the two awaits guarantees the delete commits before the list reads, so
/// the deleted world can't briefly reappear (2026-05-30 "unable to delete
/// worlds" — the WASM delete was previously a silent no-op).
#[cfg(target_arch = "wasm32")]
fn kick_off_local_delete(folder_name: String) -> std::rc::Rc<std::cell::RefCell<Option<Result<Vec<crate::wasm_save::LocalWorldEntry>, String>>>> {
    // Drop the cached meta up front so a same-named world made later can't
    // inherit the deleted world's metadata.
    crate::save::WASM_META_CACHE.with(|c| {
        c.borrow_mut().remove(&folder_name);
    });
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let pubkey = crate::save::wasm_storage_key();
    let slot_clone = slot.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = match crate::wasm_save::delete_world_wasm(&pubkey, &folder_name).await {
            Ok(()) => crate::wasm_save::list_worlds_wasm(&pubkey).await,
            Err(e) => Err(e),
        };
        *slot_clone.borrow_mut() = Some(result);
    });
    slot
}

/// Kick off `import_world_wasm` (the "Restore from file" flow) for the current
/// pubkey. Returns a slot the caller polls via `poll_import`.
#[cfg(target_arch = "wasm32")]
fn kick_off_import() -> std::rc::Rc<std::cell::RefCell<Option<Result<String, String>>>> {
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let pubkey = crate::save::wasm_storage_key();
    let slot_clone = slot.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = crate::wasm_save::import_world_wasm(pubkey).await;
        *slot_clone.borrow_mut() = Some(result);
    });
    slot
}

/// Kick off "Export everything" — pack every local world plus the Trials
/// records into one `.axeprofile` and hand it to the browser as a download.
/// Returns a slot the caller polls via `poll_profile_export`.
#[cfg(target_arch = "wasm32")]
fn kick_off_profile_export() -> std::rc::Rc<std::cell::RefCell<Option<Result<String, String>>>> {
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let pubkey = crate::save::wasm_storage_key();
    let slot_clone = slot.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = crate::wasm_save::export_profile_wasm(pubkey).await;
        *slot_clone.borrow_mut() = Some(result);
    });
    slot
}

// ---------------------------------------------------------------------------
// Main Menu
// ---------------------------------------------------------------------------

/// Apply a world-card action: mutate menu state (open dialogs, persist the Stash
/// toggle, kick a backup) and return any resulting `MenuAction` (Play/Host/Load),
/// or `MenuAction::None`. Centralised so the responsive grid stays readable.
pub(crate) fn handle_card_action(state: &mut MenuState, idx: usize, card_action: CardAction) -> MenuAction {
    // A world saved by a newer version is never opened or hosted (Spec 02 §8.4):
    // stay in the lobby and say why, before anything reads or writes it.
    // CP15 (CP14 review MEDIUM-1): nor forked (a refused world is never
    // copied into a playable one); the card says why instead.
    // Edit and Stash rewrite world_meta.json: a world from a newer build (unknown
    // terrain generator) must not be renamed or toggled from this build.
    if matches!(
        card_action,
        CardAction::DoubleClick
            | CardAction::Play
            | CardAction::Host
            | CardAction::Fork
            | CardAction::Edit
            | CardAction::ToggleStash
    ) && let Some(why) = save::world_open_refusal(&state.worlds[idx].folder_name)
    {
        state.notice = Some(why);
        return MenuAction::None;
    }
    match card_action {
        CardAction::Select => {
            state.selected = Some(idx);
        }
        CardAction::DoubleClick | CardAction::Play => {
            return MenuAction::LoadWorld(state.worlds[idx].folder_name.clone(), state.spawn_pref);
        }
        CardAction::Host => {
            return MenuAction::HostWorld(state.worlds[idx].folder_name.clone());
        }
        #[cfg(not(target_arch = "wasm32"))]
        CardAction::HostOnline => {
            return MenuAction::HostWorldOnline(state.worlds[idx].folder_name.clone());
        }
        CardAction::Edit => {
            let entry = &state.worlds[idx];
            state.dialog = MenuDialog::Edit {
                world_idx: idx,
                name: entry.meta.display_name.clone(),
                description: entry.meta.description.clone(),
            };
        }
        CardAction::Fork => {
            state.dialog = MenuDialog::Forking { world_idx: idx };
        }
        CardAction::Delete => {
            state.dialog = MenuDialog::Delete { world_idx: idx, confirm_text: String::new() };
        }
        #[cfg(target_arch = "wasm32")]
        CardAction::Backup => {
            let pubkey = crate::save::wasm_storage_key();
            let folder = state.worlds[idx].folder_name.clone();
            wasm_bindgen_futures::spawn_local(crate::wasm_save::export_world_wasm(pubkey, folder));
        }
        #[cfg(not(target_arch = "wasm32"))]
        CardAction::Export => {
            // Primary path: Save-As dialog. Guard against double-open.
            if state.pending_dialog.is_none() {
                let name = state.worlds[idx].folder_name.clone();
                match export_dialog_request(&name) {
                    Ok(req) => {
                        // Immediate feedback before the OS dialog appears, so a
                        // second click sees a busy state rather than nothing.
                        state.transfer_status = Some("Opening the file picker…".to_string());
                        state.pending_dialog =
                            Some(crate::native_file_dialog::spawn_dialog(req));
                    }
                    Err(e) => {
                        state.transfer_status = Some(format!("Export failed: {e}"));
                    }
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        CardAction::ExportToFolder => {
            // Fallback path: write to world-transfer/ folder without a dialog.
            let name = state.worlds[idx].folder_name.clone();
            state.transfer_status = Some(native_export_world(&name));
        }
        CardAction::ToggleStash => {
            // Flip the per-world Stash opt-in and persist. Native-only trigger
            // (see the `stash_button_style`/`stash_hint` button, itself
            // `#[cfg(not(wasm32))]`) — the web push-to-cloud branch that used to
            // live here was removed 2026-07-09 (confirmed dead: this handler was
            // never reachable on a wasm32 build in the first place, since its
            // only trigger button doesn't compile in on web; see
            // docs/superpowers/specs/2026-07-09-give-aliases-and-dead-web-stash-code.md).
            let new_val = !state.worlds[idx].meta.cloud_save;
            state.worlds[idx].meta.cloud_save = new_val;
            let folder = state.worlds[idx].folder_name.clone();
            let mut meta = state.worlds[idx].meta.clone();
            keep_disk_pop_secret(&folder, &mut meta);
            if let Err(e) = save::save_world_meta(&folder, &meta) {
                log::error!("Failed to persist Stash toggle: {e}");
            }
        }
        CardAction::None => {}
    }
    MenuAction::None
}

/// A lobby entry's meta can be older than the world on disk: a legacy world
/// listed, then played (a fresh PoP secret generated and saved), then edited or
/// Stash-toggled from the stale entry would write `pop_secret: None` back and
/// the next load would roll yet another secret (review N4). Keep the one on
/// disk. Native only: the web load path reads the secret from the world blob,
/// never from this meta.
fn keep_disk_pop_secret(folder: &str, meta: &mut crate::save::WorldMeta) {
    #[cfg(not(target_arch = "wasm32"))]
    if meta.pop_secret.is_none() {
        meta.pop_secret = save::load_world_meta(folder).pop_secret;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (folder, meta);
}

/// WASM-only: a simple "loading" veil shown over the lobby while a world's
/// async data fetch (IndexedDB / cloud) is in flight — BEFORE we have the data
/// to enter `GameMode::Loading`. Darkens the screen, swallows clicks (so Play
/// can't be re-fired), and animates an ellipsis. Once the data lands the engine
/// switches to the full animated `loading_screen::draw_loading_screen` for the
/// chunk build. Native loads synchronously and never shows this veil.
#[cfg(target_arch = "wasm32")]
pub(crate) fn draw_loading_overlay(ctx: &egui::Context, world_name: &str) {
    ctx.request_repaint(); // keep the animation ticking each frame
    let t = ctx.input(|i| i.time);
    let dots = ".".repeat(1 + ((t * 2.0) as usize % 3));
    egui::Area::new(egui::Id::new("loading_overlay"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let screen = ui.max_rect();
            ui.painter()
                .rect_filled(screen, 0.0, egui::Color32::from_rgba_premultiplied(8, 10, 16, 235));
            // Eat any click/drag over the whole screen so input can't leak.
            ui.allocate_rect(screen, egui::Sense::click_and_drag());
            ui.painter().text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                format!("Loading {world_name}{dots}"),
                egui::FontId::proportional(26.0),
                TITLE_COLOR,
            );
        });
}

/// My Servers (Spec A) — the player's saved-server quick-join column. A row's
/// "Join" re-resolves the operator npub fresh (native; the web resolve path is
/// an A-8 follow-up). Visual placement/feel = playtest boundary.
///
/// Drawn as a card on the Play together page (`play_together_ui`), which owns
/// the scroll area — hence `_inner`, and hence no `ScrollArea` of its own
/// (nesting two would give it its own scrollbar inside the page's).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_my_servers_column_inner(
    ui: &mut egui::Ui,
    state: &mut MenuState,
    m: &crate::ui_safe_area::Metrics,
) -> MenuAction {
    fn short_npub(s: &str) -> String {
        if s.len() > 18 {
            format!("{}…{}", &s[..12], &s[s.len() - 4..])
        } else {
            s.to_string()
        }
    }

    let mut action = MenuAction::None;
    let mut remove_npub: Option<String> = None;

    ui.label(
        egui::RichText::new("My Servers")
            .size(m.px(20.0))
            .color(TITLE_COLOR)
            .strong(),
    );
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("Servers you've joined. Join by name — the address is looked up fresh.")
            .size(m.body())
            .color(SUBTITLE_COLOR),
    );
    ui.add_space(6.0);
    ui.separator();

    // Snapshot (immutable) so the row loop can stage mutations.
    let rows: Vec<(String, String, String, bool)> = state
        .my_servers
        .sorted()
        .iter()
        .map(|e| {
            (
                e.operator_npub.clone(),
                e.name.clone(),
                e.last_endpoint.clone(),
                e.favourite,
            )
        })
        .collect();

    if rows.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(
                "No saved servers yet. Join one with an axenstax://npub… link and it'll appear here.",
            )
            .size(m.body())
            .color(DIM_TEXT),
        );
    }

    for (npub, name, endpoint, favourite) in &rows {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if *favourite {
                ui.label(egui::RichText::new("★").size(m.body()).color(TITLE_COLOR));
            }
            let title = if name.is_empty() {
                short_npub(npub)
            } else {
                name.clone()
            };
            ui.label(
                egui::RichText::new(title)
                    .size(m.body())
                    .color(TEXT_COLOR)
                    .strong(),
            );
        });
        ui.label(
            egui::RichText::new(short_npub(npub))
                .size(m.body())
                .color(DIM_TEXT),
        );
        if !endpoint.is_empty() {
            ui.label(
                egui::RichText::new(endpoint.as_str())
                    .size(m.body())
                    .color(DIM_TEXT),
            );
        }
        ui.horizontal(|ui| {
            if ui.add(crate::lobby_pages_ui::button(m, "Join", crate::lobby_pages_ui::Kind::Primary).min_size(egui::vec2(m.px(72.0), m.hit()))).clicked() {
                action = MenuAction::JoinGame(format!("axenstax://{npub}"));
            }
            if ui.add(crate::lobby_pages_ui::button(m, "Remove", crate::lobby_pages_ui::Kind::Secondary)).clicked() {
                remove_npub = Some(npub.clone());
            }
        });
        ui.separator();
    }

    if let Some(npub) = remove_npub {
        state.my_servers.remove(&npub);
        state.my_servers.save();
    }
    action
}

/// My Keeps: the Keeps the player has been let into, under My Servers in the friends
/// column. A row's Join dials the Keep with the invite it saved (no confirm: the player
/// already agreed to this Keep); Forget needs a second tap. Hidden while the list is empty.
/// The same block sits under the text box of the Join Game dialog (`draw_my_keeps`), which is
/// the only place a phone shows it. Visual placement/feel = playtest boundary.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_my_keeps_column_inner(
    ui: &mut egui::Ui,
    state: &mut MenuState,
    m: &crate::ui_safe_area::Metrics,
) -> MenuAction {
    draw_my_keeps(ui, &mut state.keeps, &mut state.keep_forget_armed, &crate::keeps::path(), KeepsLayout::Whole, Some(m))
}

/// How the My Keeps rows are laid out.
#[cfg(not(target_arch = "wasm32"))]
enum KeepsLayout<'a> {
    /// Every row inline; the container scrolls as a whole (the friends column).
    Whole,
    /// Every row in a scroll area of this height (the Join Game dialog on a tall window).
    Scroll(f32),
    /// The newest `JOIN_KEEPS_COMPACT_SHOWN` rows inline, with a More Keeps / Fewer Keeps
    /// button under them for the rest, drawn inline too (the compact Join Game card, which
    /// scrolls as a whole: a swipe on the rows must never be caught by a scroll area of
    /// their own).
    Folded(&'a mut bool),
}

/// A My Keeps button: the dialog's small button, or the lobby page's hit-sized one.
#[cfg(not(target_arch = "wasm32"))]
fn keeps_button(
    ui: &mut egui::Ui,
    m: Option<&crate::ui_safe_area::Metrics>,
    label: &str,
    kind: crate::lobby_pages_ui::Kind,
) -> egui::Response {
    match m {
        Some(m) => ui.add(crate::lobby_pages_ui::button(m, label, kind).min_size(egui::vec2(m.px(72.0), m.hit()))),
        None => ui.button(egui::RichText::new(label).size(12.0)),
    }
}

/// The My Keeps block: a heading, then a row per Keep (name, age, Join, Forget). Shared by the
/// friends column and the Join Game dialog; `layout` says how the rows sit (see `KeepsLayout`).
/// Returns `JoinKeep` when a Join was tapped, else `None`. A Forget that completes removes the
/// Keep from `file` and from `keeps`.
#[cfg(not(target_arch = "wasm32"))]
fn draw_my_keeps(
    ui: &mut egui::Ui,
    keeps: &mut Vec<crate::keeps::Keep>,
    forget_armed: &mut Option<String>,
    file: &std::path::Path,
    layout: KeepsLayout<'_>,
    m: Option<&crate::ui_safe_area::Metrics>,
) -> MenuAction {
    // `m` is `None` in the Join Game dialog (its own sizes) and the lobby's
    // metrics on the Play together page (body text, hit-sized buttons).
    let text = |dialog: f32| m.map_or(dialog, |m| m.body());
    let mut action = MenuAction::None;
    if keeps.is_empty() {
        return action;
    }
    let now = crate::keeps::unix_now();
    let rows = crate::keeps::rows(keeps, now);

    ui.label(egui::RichText::new(crate::keeps::MY_KEEPS).size(m.map_or(18.0, |m| m.px(20.0))).color(TITLE_COLOR).strong());
    ui.add_space(6.0);
    ui.separator();

    let mut forget: Option<String> = None;
    let shown = match &layout {
        KeepsLayout::Folded(expanded) if !**expanded => JOIN_KEEPS_COMPACT_SHOWN,
        _ => rows.len(),
    };
    let mut draw_rows = |ui: &mut egui::Ui| {
        for row in rows.iter().take(shown) {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(row.name.as_str()).size(text(14.0)).color(TEXT_COLOR).strong());
            ui.label(egui::RichText::new(row.when.as_str()).size(text(10.0)).color(DIM_TEXT));
            let armed = forget_armed.as_deref() == Some(row.host_key.as_str());
            ui.horizontal(|ui| {
                if keeps_button(ui, m, "Join", crate::lobby_pages_ui::Kind::Primary).clicked() {
                    let invite = keeps
                        .iter()
                        .find(|k| k.host_key == row.host_key)
                        .and_then(|k| k.keys())
                        .map(|(host_key, secret)| crate::keeps::Invite { host_key, secret });
                    if let Some(invite) = invite {
                        *forget_armed = None;
                        action = MenuAction::JoinKeep(invite);
                    }
                }
                let forget_label = if armed { "Forget for sure" } else { "Forget" };
                if keeps_button(ui, m, forget_label, crate::lobby_pages_ui::Kind::Secondary).clicked()
                    && crate::keeps::tap_forget(forget_armed, &row.host_key)
                {
                    forget = Some(row.host_key.clone());
                }
            });
            // The sentence appears under the buttons the player has just tapped, which can be
            // the bottom edge of a scrolled area: scroll it into view once, when it appears.
            let seen = egui::Id::new(("keep-forget-seen", row.host_key.as_str()));
            if armed {
                let said = ui.label(egui::RichText::new(crate::keeps::FORGET_CONFIRM).size(text(11.0)).color(DIM_TEXT));
                if !ui.data(|d| d.get_temp::<bool>(seen).unwrap_or(false)) {
                    ui.data_mut(|d| d.insert_temp(seen, true));
                    said.scroll_to_me(None);
                }
            } else {
                ui.data_mut(|d| d.remove_temp::<bool>(seen));
            }
            ui.separator();
        }
    };
    match layout {
        KeepsLayout::Scroll(h) => {
            egui::ScrollArea::vertical()
                .id_salt("my_keeps_rows")
                .max_height(h)
                .auto_shrink([false, true])
                .show(ui, draw_rows);
        }
        KeepsLayout::Whole => draw_rows(ui),
        KeepsLayout::Folded(expanded) => {
            draw_rows(ui);
            if rows.len() > JOIN_KEEPS_COMPACT_SHOWN {
                ui.add_space(6.0);
                let label = if *expanded {
                    "Fewer Keeps".to_string()
                } else {
                    format!("More Keeps ({})", rows.len() - JOIN_KEEPS_COMPACT_SHOWN)
                };
                if keeps_button(ui, m, &label, crate::lobby_pages_ui::Kind::Secondary).clicked() {
                    *expanded = !*expanded;
                }
            }
        }
    }

    if let Some(host_hex) = forget
        && let Some(host_key) = fray_keep::parse_hex32(&host_hex)
    {
        crate::keeps::forget(file, &host_key);
        *keeps = crate::keeps::load_list(file);
    }
    action
}

/// Trials — the "What to do?" (ⓘ) pop-up: a centred modal with the detailed
/// how-to for one trial + Cancel / Play. Returns a `PlayScenario` action when
/// Play is pressed (and clears the modal); Cancel or clicking the dimmed
/// backdrop clears it. No-op when no info modal is open.
/// Resolve a crafting trial's recipe hints to their catalogue card display
/// names ("Crafting Table", "Wooden Pickaxe", …) for the "What to do" footer.
fn trial_recipe_card_names(token: &str) -> Vec<String> {
    crate::scenario::trial_recipe_hints(token)
        .iter()
        .filter_map(|n| {
            let stack = crate::commands::builtins::give::resolve_item(n, 1).ok()?;
            let idx = crate::crafting_catalogue::recipe_index_for_output(&stack.item)?;
            crate::crafting_catalogue::all_cards().get(idx).map(|c| c.name.clone())
        })
        .collect()
}

fn draw_trial_info_modal(ctx: &egui::Context, state: &mut MenuState) -> MenuAction {
    let Some(key) = state.trial_info.clone() else {
        return MenuAction::None;
    };
    let ttype = crate::trials::type_for_key(&key);
    // Resolve the row key → (title, task list, recipe-card names, launchable def).
    // The popup is high-level ("what to do") — the control-specific "how" lives
    // in the in-game objective panel.
    let resolved: Option<(String, crate::scenario::TaskList, Vec<String>, crate::scenario::ScenarioDef)> =
        if let Some(id) = key.strip_prefix("race:") {
            crate::trials::CATALOG.iter().find(|d| d.id == id).map(|d| {
                let dist = {
                    let dx = (d.finish_xz[0] - d.start_xz[0]) as f32;
                    let dz = (d.finish_xz[1] - d.start_xz[1]) as f32;
                    (dx * dx + dz * dz).sqrt().round() as i32
                };
                let tasks = crate::scenario::TaskList {
                    ordered: false,
                    lines: vec![
                        format!("Run ~{dist} blocks to the bright finish beacon."),
                        "Beat your best time — you race the ghost of your last run.".to_string(),
                    ],
                    routes: Vec::new(),
                };
                (d.name.to_string(), tasks, Vec::new(), crate::trials::race_scenario_def(d))
            })
        } else if let Some(name) = key.strip_prefix("ch:") {
            crate::scenario::named_builtin_def(name).map(|def| {
                let title = def.display_name.clone();
                let tasks = crate::scenario::objective_tasks(&def, crate::scenario::trial_task_labels(name));
                (title, tasks, trial_recipe_card_names(name), def)
            })
        } else {
            None
        };
    let Some((title, tasks, recipes, def)) = resolved else {
        state.trial_info = None;
        return MenuAction::None;
    };
    let icon_color = ttype.map(|t| {
        let [r, g, b] = t.color_rgb();
        egui::Color32::from_rgb(r, g, b)
    });

    let mut action = MenuAction::None;
    let mut close = false;
    let resp = egui::Modal::new(egui::Id::new("trial_info_modal")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            // Large, colour-tinted type icon.
            if let (Some(t), Some(c)) = (ttype, icon_color) {
                ui.label(egui::RichText::new(t.icon()).size(40.0).color(c));
                ui.add_space(2.0);
            }
            ui.label(egui::RichText::new(&title).size(20.0).color(TITLE_COLOR).strong());
            if let Some(t) = ttype {
                ui.label(
                    egui::RichText::new(format!("{} trial", t.label()))
                        .size(12.0)
                        .color(icon_color.unwrap_or(SUBTITLE_COLOR)),
                );
            }
        });
        ui.add_space(12.0);
        // Task list: numbered when the steps are ordered, bullets otherwise;
        // a trial with several ways reads "Pick a way:" then each route.
        let groups = tasks.groups();
        if groups.iter().any(|g| g.name.is_some()) {
            ui.label(egui::RichText::new("Pick a way:").size(15.0).strong().color(TEXT_COLOR));
            ui.add_space(3.0);
        }
        for group in &groups {
            if let Some(name) = group.name {
                ui.label(
                    egui::RichText::new(name).size(15.0).strong().color(icon_color.unwrap_or(TEXT_COLOR)),
                );
            }
            for (k, (_, line)) in group.lines.iter().enumerate() {
                let prefix = if group.ordered { format!("{}.", k + 1) } else { "•".to_string() };
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(prefix)
                            .size(15.0)
                            .strong()
                            .color(icon_color.unwrap_or(TEXT_COLOR)),
                    );
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(*line).size(15.0).color(TEXT_COLOR));
                });
                ui.add_space(3.0);
            }
        }
        if !recipes.is_empty() {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!(
                    "\u{1F528} You'll craft: {} — the recipes show in your inventory.",
                    recipes.join(", ")
                ))
                .size(12.0)
                .color(SUBTITLE_COLOR),
            );
        }
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("Cancel").size(15.0)).clicked() {
                close = true;
            }
            ui.add_space(10.0);
            if ui.button(egui::RichText::new("Play").size(15.0).strong()).clicked() {
                action = MenuAction::PlayScenario { def: Box::new(def.clone()) };
                close = true;
            }
        });
        ui.add_space(4.0);
    });
    // Clicking the dimmed backdrop (or Esc) dismisses — treat it as Cancel.
    if resp.should_close() {
        close = true;
    }
    if close {
        state.trial_info = None;
    }
    action
}

/// Draw the main menu. Returns a MenuAction if the user did something.
/// The version line under the lobby wordmark: what you are running, and whether
/// anything newer is published.
///
/// Native-only. The web build is always current by construction, so the line
/// would say nothing useful there.
///
/// Uses `ui.hyperlink_to` rather than calling `webbrowser` directly: the
/// `egui-winit` "links" feature (already enabled in Cargo.toml) owns opening
/// URLs, so this needs no new dependency and behaves the same on every platform
/// egui already supports. If the browser cannot be opened the link simply does
/// nothing — an unopened link is a far better failure than a crash.
///
/// Restyled 2026-09 from an 11px dim-grey footnote into a badge: kids notice
/// what version they're running and compare it with friends, so `Current`
/// should read as a small win, not something apologised for in the corner.
/// The four states stay exactly as honestly distinguished as before — only
/// `Available` gets the eye-catching treatment, because it's the only one
/// asking the player for anything. The badge pill is the SAME visual language
/// as `draw_game_mode_badge`/`draw_publish_badge` elsewhere in this file
/// (rounded-rect + centered caps label); the `Current`/`Available` colour
/// pairs are literally `draw_publish_badge`'s `UpToDate`/`UnpublishedChanges`
/// pairs — reusing established meaning ("this is good" / "this wants your
/// attention", not an error) rather than inventing a new hue. `Ahead` reuses
/// the CREATIVE badge's blue: informational, deliberately not a warning
/// colour.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_version_line(ui: &mut egui::Ui, restart: &mut bool) {
    use crate::update_check::{self, UpdateState};

    const VERSION_SIZE: f32 = 15.0;
    const BADGE_FONT: f32 = 10.0;
    const BADGE_H: f32 = 20.0;
    // = draw_publish_badge's PublishBadge::UpToDate pair.
    const LATEST_BG: egui::Color32 = egui::Color32::from_rgb(28, 64, 38);
    const LATEST_TEXT: egui::Color32 = egui::Color32::from_rgb(150, 224, 160);
    // = draw_publish_badge's PublishBadge::UnpublishedChanges pair.
    const AVAILABLE_BG: egui::Color32 = egui::Color32::from_rgb(74, 60, 24);
    const AVAILABLE_TEXT: egui::Color32 = egui::Color32::from_rgb(232, 206, 130);

    let cur = update_check::current_version();

    let pill = |ui: &mut egui::Ui, label: &str, bg: egui::Color32, fg: egui::Color32, w: f32| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, BADGE_H), egui::Sense::hover());
        ui.painter().rect_filled(rect, 10.0, bg);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(BADGE_FONT),
            fg,
        );
    };

    match update_check::state() {
        // In flight, offline, check failed, or opted out. No badge at all —
        // state only what is certain. Bigger and less washed-out than the old
        // 11px/(120,124,140) footnote, but SUBTITLE_COLOR (not TEXT_COLOR) so
        // it still reads as "we don't know", never as a quiet claim of
        // currency.
        UpdateState::Unknown => {
            ui.label(
                egui::RichText::new(format!("AxeNStax v{cur}")).size(VERSION_SIZE).color(SUBTITLE_COLOR),
            );
        }
        // The state a kid wants to see: running the newest published build.
        UpdateState::Current => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    egui::RichText::new(format!("AxeNStax v{cur}"))
                        .size(VERSION_SIZE)
                        .color(TEXT_COLOR)
                        .strong(),
                );
                pill(ui, "★ LATEST", LATEST_BG, LATEST_TEXT, 92.0);
            });
        }
        // A local/CI build newer than anything published. Calling that
        // "latest" would be a quiet lie, so this stays neutral — informational,
        // not a warning.
        UpdateState::Ahead => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(egui::RichText::new(format!("AxeNStax v{cur}")).size(VERSION_SIZE).color(TEXT_COLOR));
                pill(ui, "DEV BUILD", BADGE_CREATIVE_BG, BADGE_CREATIVE_TEXT, 92.0);
            });
        }
        UpdateState::Available { version: latest, appimage } => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(egui::RichText::new(format!("AxeNStax v{cur}")).size(VERSION_SIZE).color(TEXT_COLOR));
                // The eye-catching one — sits right next to the manual
                // download link and (when offered) the in-place "Update now"
                // button, both drawn below unchanged.
                pill(ui, "UPDATE AVAILABLE", AVAILABLE_BG, AVAILABLE_TEXT, 150.0);
                ui.hyperlink_to(
                    egui::RichText::new(format!("(v{latest} available)"))
                        .size(VERSION_SIZE)
                        .color(TITLE_COLOR)
                        .strong(),
                    update_check::DOWNLOAD_PAGE,
                );
                draw_self_update_controls(ui, &latest, appimage, VERSION_SIZE, SUBTITLE_COLOR, restart);
            });
        }
    }
}

/// The in-place "Update now" button + progress line, shown only when this
/// build IS a running AppImage (so there is a file to overwrite) AND the
/// manifest carried an AppImage reference to fetch. Everything else about
/// `Available` — the version text and the manual download link — is drawn by
/// the caller regardless, so a `None` appimage or a non-AppImage build falls
/// straight back to today's link-only behaviour.
///
/// Gated like its caller: `self_update` and `update_check` do not exist on
/// wasm32, and a free fn is NOT covered by its caller's cfg — v0.2.19's first
/// gate run broke the WASM build on exactly that assumption.
#[cfg(not(target_arch = "wasm32"))]
fn draw_self_update_controls(
    ui: &mut egui::Ui,
    latest_version: &str,
    appimage: Option<crate::update_check::AppImageRef>,
    size: f32,
    dim: egui::Color32,
    restart: &mut bool,
) {
    use crate::self_update::{self, Progress};

    let (Some(appimage), Some(running)) = (appimage, self_update::running_appimage()) else {
        return;
    };

    match self_update::progress() {
        Progress::Idle => {
            if ui.small_button("Update now").clicked() {
                self_update::start(running, appimage, latest_version.to_string());
            }
        }
        Progress::Downloading { received, total } => {
            let text = match total {
                Some(total) if total > 0 => {
                    format!("Downloading v{latest_version}... {}%", (received * 100 / total).min(100))
                }
                _ => format!("Downloading v{latest_version}... {:.1} MiB", received as f64 / 1_048_576.0),
            };
            ui.label(egui::RichText::new(text).size(size).color(dim));
        }
        Progress::Verifying => {
            ui.label(egui::RichText::new("Checking the download...").size(size).color(dim));
        }
        Progress::Installed { version } => {
            ui.label(
                egui::RichText::new(format!("Updated to v{version} —"))
                    .size(size)
                    .color(dim),
            );
            // CP12 (CP10 review MEDIUM-1): never a bare `process::exit`: the
            // game's one exit function runs first, then the relaunch
            // (`MenuAction::RestartForUpdate`).
            if ui.small_button("Restart").clicked() {
                *restart = true;
            }
        }
        Progress::Failed(reason) => {
            ui.label(egui::RichText::new(format!("Update failed: {reason}")).size(size).color(dim));
        }
    }
}

pub fn draw_main_menu(ctx: &egui::Context, state: &mut MenuState) -> MenuAction {
    let mut action = MenuAction::None;

    // Online play by contact §5.1 — is "Host online" pressable on a world card,
    // and what should it say? Read once per frame (not per card) because both
    // halves touch the profile directory. Not cached on `MenuState`: signing in
    // and the first attestation both happen from inside the lobby, and a stale
    // greyed button is worse than two small file reads a frame.
    #[cfg(not(target_arch = "wasm32"))]
    let (host_online_enabled, host_online_note) = {
        let signed_in = crate::signet::native_signer::load_identity().npub().is_some();
        let attested = crate::runtime_identity::load_attestation(
            &crate::runtime_identity::attestation_path(),
        )
        .is_some();
        let s = crate::friends_ui::host_online_state(signed_in, attested);
        (s.enabled, s.note)
    };
    #[cfg(target_arch = "wasm32")]
    let (host_online_enabled, host_online_note) = (false, None::<&'static str>);

    // Drain any in-flight OS file-dialog channel so the transfer_status line
    // updates as soon as the worker reports back. Must be before any early
    // returns so it runs every frame the lobby is shown.
    #[cfg(not(target_arch = "wasm32"))]
    state.poll_dialog();

    // #7 — while a world's async load is in flight (WASM), show a
    // non-interactive loading veil instead of the live menu and swallow all
    // input. Before this, the full interactive menu kept painting with zero
    // feedback and Play could be re-fired mid-load. The caller polls the async
    // load and clears `loading_world_name` when it lands; until then we just
    // paint progress and return early so nothing underneath is clickable.
    // WASM-only: `loading_world_name` is set by the async IndexedDB/cloud load
    // path, which doesn't exist on native (native load is synchronous).
    #[cfg(target_arch = "wasm32")]
    if let Some(name) = state.loading_world_name.clone() {
        draw_loading_overlay(ctx, &name);
        return MenuAction::None;
    }

    // Full-screen background
    egui::Area::new(egui::Id::new("menu_bg"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .interactable(false)
        .show(ctx, |ui| {
            let screen = ui.max_rect();
            ui.painter().rect_filled(screen, 0.0, BG_DARK);
        });

    // Why the player is back here, when it wasn't their choice (a dropped or
    // closed connection, a kick). Stays until dismissed.
    if let Some(notice) = state.notice.clone() {
        // Top-centre, inside the TV-safe area.
        let safe = crate::ui_safe_area::rect(ctx);
        egui::Area::new(egui::Id::new("menu_notice"))
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(safe.center().x, safe.min.y))
            .constrain_to(safe)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(safe.width() - 24.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(&notice).strong());
                        // The refusal to host says "press Disconnect": here it is,
                        // at any window width.
                        #[cfg(not(target_arch = "wasm32"))]
                        if crate::signet_contacts::ui::is_host_refusal(&notice)
                            && crate::signet_contacts::view().unreadable
                            && ui.button(crate::signet_contacts::ui::DISCONNECT_SIGNET).clicked()
                        {
                            crate::signet_contacts::disconnect();
                            state.notice = None;
                        }
                        if ui.button("OK").clicked() {
                            state.notice = None;
                        }
                    });
                });
            });
    }

    // The lobby Settings panel is modal (the caller draws it over this): paint
    // nothing else so the lobby beneath can't take clicks, like the dialogs below.
    if state.show_settings {
        return action;
    }

    // The LAN browser only lives while the Join dialog is up (frees the beacon port).
    // (The Play together page lists the same games inline, so it keeps the
    // browser too while nothing is on top of it.)
    #[cfg(not(target_arch = "wasm32"))]
    if !matches!(state.dialog, MenuDialog::JoinDirect { .. })
        && !(state.page == LobbyPage::PlayTogether && matches!(state.dialog, MenuDialog::None))
    {
        state.lan = None;
        state.keeps_expanded = false;
    }

    #[cfg(not(target_arch = "wasm32"))]
    if !matches!(state.dialog, MenuDialog::JoinKeep { .. }) {
        state.keep_confirm_shown = None;
    }

    // Draw any active dialog on top
    match state.dialog.clone() {
        MenuDialog::Create { name, seed, creative, commands_enabled, cloud_save,
                             world_type, ground, water_depth, time_lock, mobs_enabled } => {
            let mut creative_flag = creative;
            let mut commands_flag = commands_enabled;
            let mut cloud_flag = cloud_save;
            let mut world_type_flag = world_type;
            let mut ground_flag = ground;
            let mut water_depth_flag = water_depth;
            let mut time_lock_flag = time_lock;
            let mut mobs_flag = mobs_enabled;
            let result = draw_create_dialog(
                ctx, &name, &seed,
                &mut creative_flag, &mut commands_flag, &mut cloud_flag,
                &mut world_type_flag, &mut ground_flag, &mut water_depth_flag,
                &mut time_lock_flag, &mut mobs_flag,
            );
            match result {
                DialogResult::Update(new_name, new_seed) => {
                    state.dialog = MenuDialog::Create {
                        name: new_name, seed: new_seed,
                        creative: creative_flag, commands_enabled: commands_flag,
                        cloud_save: cloud_flag,
                        world_type: world_type_flag, ground: ground_flag,
                        water_depth: water_depth_flag, time_lock: time_lock_flag,
                        mobs_enabled: mobs_flag,
                    };
                }
                DialogResult::Confirm(name_val, seed_val) => {
                    // Create the world (auto-name if empty for controller-only users)
                    let display_name = if name_val.trim().is_empty() {
                        format!("New World {}", state.worlds.len() + 1)
                    } else {
                        name_val
                    };
                    let folder = save::sanitize_folder_name(&display_name);
                    // P6 audit — on wasm, `sanitize_folder_name` has no
                    // filesystem to dedupe against (native walks disk; the
                    // wasm arm just returns the raw slug), so "Create" with
                    // an existing name — or the auto-name colliding after a
                    // delete — silently landed in the OLD world instead of
                    // making a new one. Dedupe against the already-loaded
                    // world list instead (the wasm equivalent of the disk
                    // scan `sanitize_folder_name` does on native, which
                    // doesn't need this — it already returns a unique slug).
                    #[cfg(target_arch = "wasm32")]
                    let folder = dedupe_folder_name(
                        &folder,
                        &state.worlds.iter().map(|w| w.folder_name.clone()).collect::<Vec<_>>(),
                    );
                    // WorldMeta::new() seeds a fresh random world. If the player
                    // typed a seed, honour it: a bare number is used as-is; any
                    // other text is hashed to a u32 (Minecraft-style text seeds).
                    // Blank → keep the random seed.
                    let mut meta = WorldMeta::new(&display_name);
                    let trimmed = seed_val.trim();
                    if !trimmed.is_empty() {
                        meta.seed = trimmed
                            .parse::<u32>()
                            .unwrap_or_else(|_| save::seed_from_text(trimmed));
                    }
                    if creative_flag {
                        meta.mark_creative();
                    }
                    meta.commands_enabled = commands_flag;
                    meta.cloud_save = cloud_flag;
                    // Blank-canvas world config (Task B3)
                    meta.world_type = world_type_flag.clone();
                    meta.ground = ground_flag;
                    meta.water_depth = water_depth_flag;
                    meta.time_lock = time_lock_flag;
                    meta.mobs_enabled = mobs_flag;
                    // #47 — blank-canvas / parkour worlds keep inventory on death
                    // (matches their day/night-lock + mobs-off "build undisturbed"
                    // profile — you shouldn't lose your kit to a parkour fall).
                    meta.keep_inventory = world_type_flag == "flat";
                    // Satoshi onboarding — host the guide only in a NEW, NORMAL
                    // Survival/Creative world (never flat/gallery, never
                    // Adventure/Spectator). game_mode is survival/creative here
                    // (the create dialog only offers those two).
                    // Test Lab forces the guide on too — Satoshi runs the
                    // playtest missions there (normal terrain, see generate_column).
                    meta.satoshi_enabled = (meta.world_type == "normal"
                        || meta.world_type == "testlab")
                        && (meta.game_mode == "survival" || meta.game_mode == "creative");
                    log::info!("[axe-cloud] create '{folder}': cloud_save={cloud_flag}");
                    if let Err(e) = save::save_world_meta(&folder, &meta) {
                        log::error!("Failed to save world meta: {e}");
                    }
                    let pref = state.spawn_pref;
                    state.dialog = MenuDialog::None;
                    action = MenuAction::LoadWorld(folder, pref);
                }
                DialogResult::Cancel => {
                    state.dialog = MenuDialog::None;
                }
                _ => {}
            }
            return action;
        }
        MenuDialog::Edit { world_idx, name, description } => {
            let result = draw_edit_dialog(ctx, &name, &description);
            match result {
                DialogResult::Update(new_name, new_desc) => {
                    state.dialog = MenuDialog::Edit { world_idx, name: new_name, description: new_desc };
                }
                DialogResult::Confirm(new_name, new_desc) => {
                    if let Some(entry) = state.worlds.get(world_idx) {
                        let folder = entry.folder_name.clone();
                        let mut meta = entry.meta.clone();
                        meta.display_name = new_name;
                        meta.description = new_desc;
                        keep_disk_pop_secret(&folder, &mut meta);
                        if let Err(e) = save::save_world_meta(&folder, &meta) {
                            log::error!("Failed to save meta: {e}");
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            action = MenuAction::RefreshWorlds;
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            // P6 audit — see kick_off_local_meta_edit: a plain
                            // RefreshWorlds would re-list straight from
                            // IndexedDB before this edit ever reached the
                            // record, silently reverting it. Chain the
                            // record update → re-list instead, same pattern
                            // as the delete flow above.
                            state.local_fetch = Some(kick_off_local_meta_edit(folder, meta));
                            state.local_loaded = false;
                        }
                    }
                    state.dialog = MenuDialog::None;
                }
                DialogResult::Cancel => {
                    state.dialog = MenuDialog::None;
                }
                _ => {}
            }
            return action;
        }
        MenuDialog::Delete { world_idx, confirm_text } => {
            let world_name = state.worlds.get(world_idx)
                .map(|e| e.meta.display_name.clone())
                .unwrap_or_default();
            let result = draw_delete_dialog(ctx, &world_name, &confirm_text);
            match result {
                DialogResult::Update(new_text, _) => {
                    state.dialog = MenuDialog::Delete { world_idx, confirm_text: new_text };
                }
                DialogResult::ConfirmSingle => {
                    if let Some(entry) = state.worlds.get(world_idx) {
                        let folder = entry.folder_name.clone();
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            if let Err(e) = save::delete_world(&folder) {
                                log::error!("Delete failed: {e}");
                            }
                            action = MenuAction::RefreshWorlds;
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            // Sequence delete → re-list in a single async
                            // chain so the refreshed world list can't race
                            // ahead of the IndexedDB delete commit (a plain
                            // RefreshWorlds kicks an independent list that
                            // might read the world back before it's gone).
                            state.local_fetch = Some(kick_off_local_delete(folder));
                            state.local_loaded = false;
                            state.selected = None;
                        }
                    }
                    state.dialog = MenuDialog::None;
                }
                DialogResult::Cancel => {
                    state.dialog = MenuDialog::None;
                }
                _ => {}
            }
            return action;
        }
        MenuDialog::Forking { world_idx } => {
            // Fork happens synchronously for now
            if let Some(entry) = state.worlds.get(world_idx) {
                match save::fork_world(&entry.folder_name, &entry.meta.display_name) {
                    Ok(_new_folder) => {
                        log::info!("Fork complete");
                    }
                    Err(e) => {
                        log::error!("Fork failed: {e}");
                        state.notice = Some(e);
                    }
                }
            }
            state.dialog = MenuDialog::None;
            action = MenuAction::RefreshWorlds;
            return action;
        }
        MenuDialog::JoinDirect { address } => {
            // Native: poll LAN announcements for the "Games on this network" list.
            #[cfg(not(target_arch = "wasm32"))]
            let lan_games = crate::lan_ui::poll_games(&mut state.lan);
            #[cfg(not(target_arch = "wasm32"))]
            let mut picked = None;
            // My Keeps, under the text box: a Join there is the friends column's Join.
            #[cfg(not(target_arch = "wasm32"))]
            let mut keep_join = MenuAction::None;
            #[cfg(not(target_arch = "wasm32"))]
            let keeps_file = crate::keeps::path();
            let result = draw_join_dialog(
                ctx,
                &address,
                #[cfg(not(target_arch = "wasm32"))]
                &lan_games,
                #[cfg(not(target_arch = "wasm32"))]
                &mut picked,
                #[cfg(not(target_arch = "wasm32"))]
                JoinKeepsPane {
                    keeps: &mut state.keeps,
                    forget_armed: &mut state.keep_forget_armed,
                    expanded: &mut state.keeps_expanded,
                    file: &keeps_file,
                    join: &mut keep_join,
                },
            );
            // A click on a heard room joins it straight away.
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(room) = picked {
                state.dialog = MenuDialog::None;
                state.lan = None;
                return MenuAction::JoinLanRoom(room);
            }
            // A Join on a saved Keep closes the dialog and dials it, with no confirm.
            #[cfg(not(target_arch = "wasm32"))]
            if matches!(keep_join, MenuAction::JoinKeep(_)) {
                state.dialog = MenuDialog::None;
                state.lan = None;
                return keep_join;
            }
            match result {
                DialogResult::Update(new_addr, _) => {
                    state.dialog = MenuDialog::JoinDirect { address: new_addr };
                }
                DialogResult::Confirm(addr, _) => {
                    state.dialog = MenuDialog::None;
                    // Free the beacon port now: the menu is not drawn again once a join starts.
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        state.lan = None;
                    }
                    action = MenuAction::JoinGame(addr);
                }
                DialogResult::Cancel => {
                    state.dialog = MenuDialog::None;
                }
                _ => {}
            }
            return action;
        }
        #[cfg(not(target_arch = "wasm32"))]
        MenuDialog::JoinKeep { invite } => {
            let shown = *state.keep_confirm_shown.get_or_insert_with(web_time::Instant::now);
            let armed = crate::keeps::confirm_armed(shown.elapsed());
            egui::Window::new(crate::keeps::CONFIRM_TITLE)
                .max_width(crate::ui_safe_area::rect(ctx).width() - 24.0)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(380.0);
                    ui.label(egui::RichText::new(crate::keeps::label(&invite.host_key)).strong());
                    ui.add_space(6.0);
                    ui.label(crate::keeps::CONFIRM_BODY);
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(armed, egui::Button::new(egui::RichText::new("Join").strong())).clicked() {
                            action = MenuAction::JoinKeep(invite);
                            state.dialog = MenuDialog::None;
                        }
                        if ui.button("Cancel").clicked() {
                            state.dialog = MenuDialog::None;
                        }
                    });
                });
            return action;
        }
        MenuDialog::ResetWorkshop => {
            egui::Window::new("Reset The Workshop?")
                .max_width(crate::ui_safe_area::rect(ctx).width() - 24.0)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(380.0);
                    ui.label(
                        "This clears everything you've built in The Workshop and \
                         returns it to an empty floor.",
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Your saved skins and redesigns are kept.")
                            .color(egui::Color32::from_rgb(160, 220, 160)),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        let reset = egui::Button::new(
                            egui::RichText::new("Reset Workshop").strong()
                                .color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(150, 60, 30));
                        if ui.add(reset).clicked() {
                            action = MenuAction::ResetWorkshop;
                            state.dialog = MenuDialog::None;
                        }
                        if ui.button("Cancel").clicked() {
                            state.dialog = MenuDialog::None;
                        }
                    });
                });
            return action;
        }
        MenuDialog::SignIn { paste_uri, show_paste } => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut paste = paste_uri;
                let mut show = show_paste;
                match draw_signin_dialog(ctx, &mut paste, &mut show, &state.relays, &mut state.show_relays) {
                    SignInDialogResult::Close => {
                        crate::native_signin::reset();
                        state.dialog = MenuDialog::None;
                    }
                    SignInDialogResult::Stay => {
                        state.dialog = MenuDialog::SignIn { paste_uri: paste, show_paste: show };
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (paste_uri, show_paste);
                state.dialog = MenuDialog::None;
            }
            return action;
        }
        #[cfg(target_arch = "wasm32")]
        MenuDialog::DesktopOnly(feature) => {
            // Modal popup that floats OVER the lobby. Deliberately NO early return:
            // the lobby CentralPanel renders below in Background order while these
            // Foreground Areas float on top — so the lobby stays visible (dimmed)
            // behind the card, instead of the popup looking like a whole new screen.
            let copy = app_feature_copy(feature);

            // Dimmed, click-absorbing backdrop (modal; the lobby shows through).
            egui::Area::new(egui::Id::new("desktop_popup_backdrop"))
                .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
                .interactable(true)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    // Full window rect — NOT ui.max_rect(). Because we deliberately
                    // don't early-return, the lobby CentralPanel has already claimed
                    // the central area, so max_rect would be only the leftover space
                    // (the "shadowed box in the top-left corner" bug). screen_rect is
                    // always the whole window, so the dim covers everything.
                    let screen = ui.ctx().content_rect();
                    ui.painter().rect_filled(
                        screen,
                        0.0,
                        egui::Color32::from_rgba_premultiplied(0, 0, 0, 150),
                    );
                    ui.allocate_rect(screen, egui::Sense::click());
                });

            // The card — roomy padding, broken-up content.
            egui::Area::new(egui::Id::new("desktop_popup_card"))
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .interactable(true)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(24, 27, 34))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 110, 70)))
                        .corner_radius(egui::CornerRadius::same(14))
                        .inner_margin(egui::Margin::same(30))
                        .show(ui, |ui| {
                            ui.set_max_width(430.0);
                            ui.set_min_width(380.0);

                            ui.label(
                                egui::RichText::new(copy.title)
                                    .size(21.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(150, 210, 160)),
                            );
                            ui.add_space(14.0);
                            ui.label(
                                egui::RichText::new(copy.lead)
                                    .size(15.5)
                                    .color(egui::Color32::from_rgb(224, 230, 236)),
                            );
                            ui.add_space(16.0);
                            for p in copy.points {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("✦")
                                            .size(14.0)
                                            .color(egui::Color32::from_rgb(120, 195, 135)),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        egui::RichText::new(*p)
                                            .size(14.0)
                                            .color(egui::Color32::from_rgb(202, 208, 214)),
                                    );
                                });
                                ui.add_space(10.0);
                            }
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(copy.cta_note)
                                    .size(13.5)
                                    .italics()
                                    .color(egui::Color32::from_rgb(140, 200, 150)),
                            );
                            ui.add_space(22.0);

                            ui.horizontal(|ui| {
                                let get = egui::Button::new(
                                    egui::RichText::new("⬇  Get the free desktop app")
                                        .size(14.0)
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                )
                                .min_size(egui::vec2(0.0, 38.0))
                                .fill(egui::Color32::from_rgb(45, 107, 45))
                                .corner_radius(egui::CornerRadius::same(6));
                                if ui.add(get).clicked() {
                                    if let Some(win) = web_sys::window() {
                                        let _ = win.open_with_url_and_target("/download", "_blank");
                                    }
                                    state.dialog = MenuDialog::None;
                                }
                                ui.add_space(10.0);
                                let close = egui::Button::new(
                                    egui::RichText::new("Close")
                                        .size(14.0)
                                        .color(egui::Color32::from_rgb(190, 196, 202)),
                                )
                                .min_size(egui::vec2(0.0, 38.0))
                                .fill(egui::Color32::from_rgba_premultiplied(40, 44, 52, 240))
                                .corner_radius(egui::CornerRadius::same(6));
                                if ui.add(close).clicked() {
                                    state.dialog = MenuDialog::None;
                                }
                            });
                        });
                });
            // NO early return — fall through so the lobby renders behind the popup.
        }
        #[cfg(not(target_arch = "wasm32"))]
        MenuDialog::AddFriend { input, error } => {
            // `state.dialog` was cloned above, so these are owned copies: the
            // edited text is written back below rather than mutated in place.
            let mut input = input;
            let mut error = error;
            let mut close = false;
            let mut submit: Option<String> = None;
            egui::Window::new("Add a friend")
                .max_width(crate::ui_safe_area::rect(ctx).width() - 24.0)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    let w = (crate::ui_safe_area::rect(ctx).width() - 40.0).min(420.0);
                    ui.set_max_width(w);
                    ui.label(crate::friends_ui::add_friend_lead());
                    ui.add(egui::TextEdit::singleline(&mut input).desired_width(w));
                    if let Some(e) = error.as_ref() {
                        ui.colored_label(egui::Color32::from_rgb(230, 150, 150), e);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Add").clicked() {
                            submit = Some(input.clone());
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if let Some(text) = submit {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                match crate::friends_ui::parse_add_friend(&text, now) {
                    Ok(crate::friends_ui::AddFriendInput::Invite(_)) => {
                        action = MenuAction::JoinByInvite(text);
                        close = true;
                    }
                    Ok(crate::friends_ui::AddFriendInput::Npub(npub)) => {
                        action = MenuAction::AddFriendNpub(npub);
                        close = true;
                    }
                    Err(e) => error = Some(e),
                }
            }
            state.dialog = if close {
                MenuDialog::None
            } else {
                MenuDialog::AddFriend { input, error }
            };
            // Modal, like JoinDirect: returning here keeps the lobby behind it
            // un-clickable, so a stray click on "Add a friend" underneath can't
            // reset the box the player is typing into.
            return action;
        }
        MenuDialog::None => {}
    }

    // The Trials gallery takes the whole screen while it is open: no world list,
    // no side columns. Back / Escape closes it (inside `draw`). The "How it works"
    // modal is drawn last, over it, as it is over the lobby.
    if state.trials_gallery.is_open() {
        let icons = crate::egui_integration::block_texture_handles(ctx);
        let g = crate::trials_gallery_ui::draw(
            ctx,
            &mut state.trials_gallery,
            &icons,
            state.trial_info.is_some(),
        );
        if !matches!(g.action, MenuAction::None) {
            action = g.action;
        }
        if let Some(key) = g.how_it_works {
            state.trial_info = Some(key);
        }
        let info_action = draw_trial_info_modal(ctx, state);
        if !matches!(info_action, MenuAction::None) {
            action = info_action;
        }
        return action;
    }

    // The lobby's pages (home page overhaul spec): Home, Your worlds, Play
    // together, More to play, each inside the TV-safe area.
    let page_action = crate::lobby_pages_ui::draw(
        ctx,
        state,
        crate::lobby_pages_ui::HostOnline { enabled: host_online_enabled, note: host_online_note },
    );
    if !matches!(page_action, MenuAction::None) {
        action = page_action;
    }

    // Trials "What to do?" pop-up — a centred modal over the whole lobby. Drawn
    // last so it sits on top; its Play takes precedence over the lobby behind it.
    let info_action = draw_trial_info_modal(ctx, state);
    if !matches!(info_action, MenuAction::None) {
        action = info_action;
    }

    action
}

// ---------------------------------------------------------------------------
// World Card
// ---------------------------------------------------------------------------

pub(crate) enum CardAction {
    None,
    Select,
    DoubleClick,
    Play,
    Host,
    Edit,
    Fork,
    /// Flip this world's Stash opt-in (cloud_save) on/off.
    ToggleStash,
    Delete,
    /// WASM only — "Back up to file" (download the packed world blob).
    #[cfg(target_arch = "wasm32")]
    Backup,
    /// Native only — export via Save-As dialog (primary path).
    #[cfg(not(target_arch = "wasm32"))]
    Export,
    /// Native only — export to world-transfer/<name>.axeworld folder (fallback).
    #[cfg(not(target_arch = "wasm32"))]
    ExportToFolder,
    /// Native only — host this world for friends outside the house.
    #[cfg(not(target_arch = "wasm32"))]
    HostOnline,
}

// ---------------------------------------------------------------------------
// Native file-dialog result routing
// ---------------------------------------------------------------------------

/// Route one [`FileDialogResult`] from the OS file-picker worker to the lobby
/// status line (and eventually to phase-specific handlers once they are built).
///
/// Returns `true` when the dialog is fully done (the channel can be cleared);
/// `false` if the result leaves the channel alive for more reads (unused for
/// now — every dialog sends exactly one result — but keeps the API honest).
///
/// Extracted as a free function so the unit tests can call it directly without
/// needing to construct a full [`MenuState`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn route_dialog_result(
    result: crate::native_file_dialog::FileDialogResult,
    transfer_status: &mut Option<String>,
) -> bool {
    use crate::native_file_dialog::FileDialogResult;
    match result {
        FileDialogResult::WorldSaved(path) => {
            *transfer_status = Some(format!("Saved to {}", path.display()));
        }
        FileDialogResult::Cancelled => {
            *transfer_status = Some("Cancelled".to_string());
        }
        FileDialogResult::Err(e) => {
            *transfer_status = Some(format!("Save/open failed: {e}"));
        }
        FileDialogResult::WorldToImport(_bytes) => {
            // Handled upstream in poll_dialog (needs &mut self for world-list refresh).
            // route_dialog_result should never see this variant in normal operation;
            // leave status untouched so it is a clear no-op if called directly in tests.
        }
        FileDialogResult::SkinPng(_bytes) => {
            // Phase 4 handles skin upload at its own poll site
        }
        FileDialogResult::SkinSaved(_path) => {
            // Handled by the dedicated pending_skin_save_dialog poll site (Task A4).
        }
        FileDialogResult::GhostJson(_) | FileDialogResult::GhostSaved(_) => {
            // Campaign G — handled by the dedicated pending_ghost_dialog poll
            // site in game_loop (needs &mut GameState for the rival store).
        }
        FileDialogResult::ProfileToImport(_bytes) => {
            // Handled upstream in poll_dialog (needs &mut self for world-list
            // refresh), exactly like WorldToImport. A clear no-op here.
        }
    }
    true // dialog is done after one result
}

// ---------------------------------------------------------------------------
// Native Save-As export helper
// ---------------------------------------------------------------------------

/// Build a [`FileDialogRequest::SaveWorld`] for the given world.
///
/// Extracted as a free function so unit tests can call it without opening an
/// OS dialog — the packing step is the only logic that needs testing here;
/// the dialog dispatch is covered by the `route_dialog_result` suite.
///
/// Returns `Err(msg)` if the world cannot be packed (e.g. folder missing).
#[cfg(not(target_arch = "wasm32"))]
fn export_dialog_request(
    name: &str,
) -> Result<crate::native_file_dialog::FileDialogRequest, String> {
    let bytes = crate::native_world_io::export_world_native(name)?;
    Ok(crate::native_file_dialog::FileDialogRequest::SaveWorld {
        default_name: format!("{name}.axeworld"),
        bytes,
    })
}

// ---------------------------------------------------------------------------
// Native world import helper (single file picked via OS dialog)
// ---------------------------------------------------------------------------

/// Unpack a `.axeworld` byte blob picked via the OS Open-file dialog.
///
/// Returns a `(status, imported_ok)` pair suitable for assignment to
/// `transfer_status`.  Extracted as a free function so unit tests can call it
/// directly without opening an OS dialog.
#[cfg(not(target_arch = "wasm32"))]
fn import_picked_world(bytes: &[u8]) -> (String, bool) {
    match crate::native_world_io::import_world_native(bytes) {
        Ok(name) => (format!("Imported '{name}' from the file you picked"), true),
        Err(e) => (format!("Import failed: {e}"), false),
    }
}

/// Apply a `.axeprofile` bundle picked via the OS Open-file dialog — the
/// receiving half of the web lobby's "Export everything".
///
/// Returns a `(status, changed)` pair for `transfer_status`; `changed` is true
/// when at least one world landed, so the caller refreshes the world list.
#[cfg(not(target_arch = "wasm32"))]
fn import_picked_profile(bytes: &[u8]) -> (String, bool) {
    match crate::native_world_io::import_profile_native(bytes) {
        Ok(summary) => {
            let changed = summary.imported > 0;
            (summary.message(), changed)
        }
        Err(e) => (format!("Import failed: {e}"), false),
    }
}

// ---------------------------------------------------------------------------
// Native world transfer helpers (Export / Import via world-transfer/ folder)
// ---------------------------------------------------------------------------

/// Export a single world to `world-transfer/<name>.axeworld`.
///
/// Returns a short human-readable status string (success or error) suitable
/// for display in the lobby's transfer-status line.
#[cfg(not(target_arch = "wasm32"))]
fn native_export_world(name: &str) -> String {
    match crate::native_world_io::export_world_native(name) {
        Err(e) => format!("Export failed: {e}"),
        Ok(bytes) => {
            if let Err(e) = std::fs::create_dir_all("world-transfer") {
                return format!("Export failed (mkdir): {e}");
            }
            let path = format!("world-transfer/{name}.axeworld");
            match std::fs::write(&path, &bytes) {
                Ok(()) => format!("Exported to the world-transfer folder: {path}"),
                Err(e) => format!("Export failed (write): {e}"),
            }
        }
    }
}

/// Scan `world-transfer/` for `*.axeworld` files and import each one.
///
/// Returns a short human-readable status string summarising what happened.
/// On success the caller must refresh `state.worlds` so new entries appear.
#[cfg(not(target_arch = "wasm32"))]
fn native_import_worlds() -> String {
    // Collect all *.axeworld paths from world-transfer/.
    let paths = list_axeworld_files("world-transfer");
    if paths.is_empty() {
        return "No .axeworld files in world-transfer/".to_string();
    }

    let mut imported: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for path in &paths {
        match std::fs::read(path) {
            Err(e) => errors.push(format!("{path}: read error — {e}")),
            Ok(bytes) => match crate::native_world_io::import_world_native(&bytes) {
                Ok(name) => imported.push(name),
                Err(e) => errors.push(format!("{path}: {e}")),
            },
        }
    }

    if errors.is_empty() {
        format!(
            "Scanned world-transfer/ — imported {} world(s): {}",
            imported.len(),
            imported.join(", ")
        )
    } else if imported.is_empty() {
        format!("Import failed: {}", errors.join("; "))
    } else {
        format!(
            "Scanned world-transfer/ — imported {}: {}. Errors: {}",
            imported.len(),
            imported.join(", "),
            errors.join("; ")
        )
    }
}

/// List all `*.axeworld` paths inside `dir`.  Returns an empty vec if the
/// directory doesn't exist or can't be read.
#[cfg(not(target_arch = "wasm32"))]
fn list_axeworld_files(dir: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(|e| {
        let e = e.ok()?;
        let path = e.path();
        if path.extension()?.to_str()? == "axeworld" {
            path.to_str().map(|s| s.to_owned())
        } else {
            None
        }
    })
    .collect()
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

enum DialogResult {
    Update(String, String),   // Two field values changed
    Confirm(String, String),  // Two field values confirmed
    ConfirmSingle,            // Single-action confirm (delete)
    Cancel,
}

/// A single-line text field that works on a touchscreen. On a touch device the
/// soft keyboard can't be summoned through winit+egui, so this renders a
/// tappable field that pops the OS keyboard via `window.prompt()` (`prompt_label`
/// is its caption, the current value is pre-filled). On desktop/native it's the
/// normal inline `TextEdit`. Returns the widget `Response` either way so callers
/// can still `request_focus()` / test `lost_focus()`.
fn menu_text_field(
    ui: &mut egui::Ui,
    buf: &mut String,
    prompt_label: &str,
    placeholder: &str,
    width: f32,
    font_size: f32,
) -> egui::Response {
    menu_text_field_tall(ui, buf, prompt_label, placeholder, width, font_size, 0.0)
}

/// [`menu_text_field`] at least `min_height` tall (a lobby page's 44 pt hit target).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn menu_text_field_tall(
    ui: &mut egui::Ui,
    buf: &mut String,
    prompt_label: &str,
    placeholder: &str,
    width: f32,
    font_size: f32,
    min_height: f32,
) -> egui::Response {
    if crate::touch_input::is_touch_device() && crate::touch_input::OS_KEYBOARD_PROMPT {
        let (text, color) = if buf.is_empty() {
            (placeholder.to_string(), egui::Color32::from_rgb(120, 120, 136))
        } else {
            (buf.clone(), egui::Color32::from_rgb(224, 224, 232))
        };
        let resp = ui.add_sized(
            egui::vec2(width, (font_size + 16.0).max(min_height)),
            egui::Button::new(egui::RichText::new(text).size(font_size).color(color))
                .fill(egui::Color32::from_rgb(28, 30, 40)),
        );
        if resp.clicked()
            && let Some(s) = crate::touch_input::os_keyboard_prompt(prompt_label, buf) {
                *buf = s;
            }
        resp
    } else {
        ui.add(
            egui::TextEdit::singleline(buf)
                .desired_width(width)
                .hint_text(if min_height > 0.0 {
                    // A lobby page's field: its hint is body size too, not egui's small default.
                    egui::RichText::new(placeholder).size(font_size)
                } else {
                    egui::RichText::new(placeholder)
                })
                .font(egui::FontId::proportional(font_size))
                .margin(egui::Margin::symmetric(8, (((min_height - font_size * 1.3) / 2.0).max(4.0)) as i8)),
        )
    }
}

/// Whether dialogs draw as the height-capped, scrolling phone card: always on Android, else
/// when the window is under 620 points tall. A test can force it on a window of any size.
fn dialog_is_compact(ctx: &egui::Context) -> bool {
    #[cfg(test)]
    if let Some(forced) = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("test-force-compact-dialog"))) {
        return forced;
    }
    cfg!(target_os = "android") || ctx.content_rect().height() < 620.0
}

fn draw_dialog_frame(ctx: &egui::Context, title: &str, title_color: egui::Color32, border_color: egui::Color32, inner: impl FnOnce(&mut egui::Ui) -> DialogResult) -> DialogResult {
    let mut result = DialogResult::Update(String::new(), String::new());

    // Overlay
    egui::Area::new(egui::Id::new("dialog_overlay"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .interactable(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let screen = ui.max_rect();
            ui.painter().rect_filled(screen, 0.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 200));
        });

    // A phone in landscape gives egui a SHORT viewport: a Pixel 8 is 2400x1080
    // physical at scale factor 2.625, i.e. ~914x411 *points*. Desktop windows
    // are 700-1000 points tall, so dialogs authored there (the create-world form
    // is ~600) overflow past both edges with no way to reach the top field or
    // the confirm button. `compact` switches every dialog routed through here
    // to a height-capped, scrolling card. It is always on for Android and
    // otherwise only when the viewport is too short for a desktop-sized dialog
    // — exactly the case that was unreachable before — so a normal desktop or
    // browser window renders byte-for-byte as it always has.
    //
    // content_rect(), not the deprecated screen_rect(): egui 0.34 split the
    // two, and on a phone the CONTENT area excludes the notch / nav-bar insets.
    let screen = ctx.content_rect();
    // Home page overhaul §2: centred in the TV-safe area, never larger than it.
    let safe = crate::ui_safe_area::rect(ctx);
    let compact = dialog_is_compact(ctx);
    let short_screen = screen.height() < 620.0;
    let margin = if short_screen { 16 } else { 28 };
    // The scrolling body's cap: the safe height less the pinned title and the card's own margins.
    let max_dialog_h = (safe.height() - 2.0 * margin as f32 - 40.0).max(160.0);
    // The -40 nudge is a desktop nicety (optical centring above the midline);
    // on a short screen it just pushes the card's bottom off the display.
    let y_offset = if short_screen { 0.0 } else { -40.0 };

    egui::Area::new(egui::Id::new("dialog_content"))
        .pivot(egui::Align2::CENTER_CENTER)
        .fixed_pos(safe.center() + egui::vec2(0.0, y_offset))
        .interactable(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(26, 29, 37))
                .stroke(egui::Stroke::new(1.0_f32, border_color))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(margin))
                .show(ui, |ui| {
                    let inner_w = safe.width() - 2.0 * margin as f32 - 2.0;
                    ui.set_min_width(360.0_f32.min(inner_w));
                    if compact {
                        // Cap the width too: on a 914-point-wide landscape
                        // phone an unconstrained card sprawls edge to edge.
                        ui.set_max_width((screen.width() - 64.0).clamp(360.0, 620.0).min(inner_w));
                    } else {
                        ui.set_max_width(inner_w);
                    }

                    // Title stays PINNED outside the scroll area — a scrolled-
                    // away title leaves you looking at an anonymous form.
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(title)
                                .size(22.0)
                                .color(title_color)
                                .strong(),
                        );
                    });
                    ui.add_space(if short_screen { 10.0 } else { 20.0 });

                    if compact {
                        // `auto_shrink([false, true])`: keep the full width but
                        // shrink vertically to the content, so a small dialog
                        // stays a small card and only an over-tall one scrolls.
                        // AlwaysVisible: the 2026-05-21 playtest found kids did
                        // not notice a scroll affordance — an always-drawn bar
                        // is the cheapest "there is more below" signal.
                        //
                        // `min_scrolled_height`: an Area's UI may only use the size it had last
                        // frame (the first frame, egui's 600 x 400 default), so without it the
                        // card could never grow past that and a long card (the Join Game
                        // dialog with saved Keeps) was cut off at about 300 points on any screen.
                        egui::ScrollArea::vertical()
                            .max_height(max_dialog_h)
                            .min_scrolled_height(max_dialog_h)
                            .auto_shrink([false, true])
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                            )
                            .show(ui, |ui| {
                                result = inner(ui);
                            });
                    } else {
                        result = inner(ui);
                    }
                });
        });

    result
}

fn draw_create_dialog(
    ctx: &egui::Context,
    name: &str,
    seed: &str,
    creative: &mut bool,
    commands_enabled: &mut bool,
    cloud_save: &mut bool,
    // Blank-canvas world config (Task B3)
    world_type: &mut String,
    ground: &mut String,
    water_depth: &mut u8,
    time_lock: &mut String,
    mobs_enabled: &mut bool,
) -> DialogResult {
    let mut name_buf = name.to_string();
    let mut seed_buf = seed.to_string();
    let mut confirmed = false;
    let mut cancelled = false;
    let mut creative_buf = *creative;
    let mut commands_buf = *commands_enabled;
    // `mut` only on native — the web build gates the Stash toggle out (local
    // sandbox), so cloud_buf is never reassigned there.
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut cloud_buf = *cloud_save;
    let mut world_type_buf = world_type.clone();
    let mut ground_buf = ground.clone();
    let mut water_depth_buf = *water_depth;
    let mut time_lock_buf = time_lock.clone();
    let mut mobs_buf = *mobs_enabled;

    let _result = draw_dialog_frame(ctx, "Create New World", TITLE_COLOR, CARD_BORDER, |ui| {
        // Moonshot Phase A — one muted flavour line seeding the founding-myth
        // setting. Copy only; the form behaves identically.
        ui.label(
            egui::RichText::new("Begin a new world in the Age of Diamonds.")
                .size(12.0)
                .italics()
                .color(egui::Color32::from_rgb(120, 120, 140)),
        );
        ui.add_space(10.0);

        // Name field
        ui.label(egui::RichText::new("WORLD NAME").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
        ui.add_space(4.0);
        let name_response =
            menu_text_field(ui, &mut name_buf, "World name", "My New World", 360.0, 15.0);
        // Auto-focus name field (desktop only — a touch field is a tap-to-type button)
        if name_buf.is_empty() && seed_buf.is_empty() {
            name_response.request_focus();
        }

        ui.add_space(14.0);

        // Seed field
        ui.label(egui::RichText::new("SEED").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("(optional — leave blank for random)").size(11.0).color(egui::Color32::from_rgb(85, 85, 102)));
        });
        ui.add_space(4.0);
        menu_text_field(ui, &mut seed_buf, "Seed (optional)", "e.g. 12345", 360.0, 15.0);

        // Game mode selector
        ui.add_space(14.0);
        ui.label(egui::RichText::new("GAME MODE").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let surv_color = if !creative_buf { egui::Color32::from_rgb(143, 199, 106) } else { egui::Color32::from_rgb(100, 100, 100) };
            let crea_color = if creative_buf { egui::Color32::from_rgb(106, 176, 199) } else { egui::Color32::from_rgb(100, 100, 100) };
            if ui.add(egui::Button::new(egui::RichText::new("Survival").size(14.0).color(surv_color))
                .min_size(egui::vec2(175.0, 36.0))
                .fill(if !creative_buf { egui::Color32::from_rgb(35, 60, 25) } else { egui::Color32::from_rgb(30, 33, 40) })
                .corner_radius(egui::CornerRadius::same(6))
            ).clicked() {
                creative_buf = false;
            }
            if ui.add(egui::Button::new(egui::RichText::new("Creative").size(14.0).color(crea_color))
                .min_size(egui::vec2(175.0, 36.0))
                .fill(if creative_buf { egui::Color32::from_rgb(25, 45, 65) } else { egui::Color32::from_rgb(30, 33, 40) })
                .corner_radius(egui::CornerRadius::same(6))
            ).clicked() {
                creative_buf = true;
            }
        });

        // ── World Type selector (Task B3) ─────────────────────────────────────
        ui.add_space(14.0);
        ui.label(egui::RichText::new("WORLD TYPE").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let norm_color = if world_type_buf == "normal" { egui::Color32::from_rgb(143, 199, 106) } else { egui::Color32::from_rgb(100, 100, 100) };
            let flat_color = if world_type_buf == "flat"   { egui::Color32::from_rgb(255, 213, 120) } else { egui::Color32::from_rgb(100, 100, 100) };
            #[cfg(not(target_arch = "wasm32"))]
            let lab_color  = if world_type_buf == "testlab" { egui::Color32::from_rgb(120, 200, 255) } else { egui::Color32::from_rgb(100, 100, 100) };
            if ui.add(egui::Button::new(egui::RichText::new("Normal").size(14.0).color(norm_color))
                .min_size(egui::vec2(170.0, 36.0))
                .fill(if world_type_buf == "normal" { egui::Color32::from_rgb(35, 60, 25) } else { egui::Color32::from_rgb(30, 33, 40) })
                .corner_radius(egui::CornerRadius::same(6))
            ).clicked() {
                world_type_buf = "normal".to_string();
            }
            if ui.add(egui::Button::new(egui::RichText::new("Blank Canvas").size(14.0).color(flat_color))
                .min_size(egui::vec2(170.0, 36.0))
                .fill(if world_type_buf == "flat" { egui::Color32::from_rgb(60, 50, 20) } else { egui::Color32::from_rgb(30, 33, 40) })
                .corner_radius(egui::CornerRadius::same(6))
            ).clicked() {
                world_type_buf = "flat".to_string();
                // Default blank-canvas to mobs OFF — build undisturbed.
                mobs_buf = false;
            }
            // Test Lab — Satoshi hands out playtest missions; normal terrain.
            // Not offered on web: its verdicts had no transport but the browser
            // lobby mailbox, removed 2026-10-01 (the game has no feedback channel).
            #[cfg(not(target_arch = "wasm32"))]
            if ui.add(egui::Button::new(egui::RichText::new("🧪 Test Lab").size(14.0).color(lab_color))
                .min_size(egui::vec2(170.0, 36.0))
                .fill(if world_type_buf == "testlab" { egui::Color32::from_rgb(20, 45, 65) } else { egui::Color32::from_rgb(30, 33, 40) })
                .corner_radius(egui::CornerRadius::same(6))
            ).clicked() {
                world_type_buf = "testlab".to_string();
            }
        });
        if world_type_buf == "testlab" {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Satoshi will ask you to try new features and tap whether they work. Normal terrain + a starter kit.")
                    .size(11.0)
                    .color(egui::Color32::from_rgb(120, 200, 255)),
            );
        }

        // ── Flat-world options (only shown when Blank Canvas is selected) ─────
        if world_type_buf == "flat" {
            // Ground block
            ui.add_space(14.0);
            ui.label(egui::RichText::new("GROUND").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (label, value) in &[
                    ("None",  "none"),
                    ("Grass", "grass"),
                    ("Sand",  "sand"),
                    ("Stone", "stone"),
                    ("Dirt",  "dirt"),
                    ("Snow",  "snow"),
                    ("Water", "water"),
                ] {
                    let selected = ground_buf == *value;
                    let fg = if selected { egui::Color32::from_rgb(255, 213, 120) } else { egui::Color32::from_rgb(160, 160, 160) };
                    if ui.add(
                        egui::Button::new(egui::RichText::new(*label).size(13.0).color(fg))
                            .min_size(egui::vec2(72.0, 30.0))
                            .fill(if selected { egui::Color32::from_rgb(60, 50, 20) } else { egui::Color32::from_rgb(30, 33, 40) })
                            .corner_radius(egui::CornerRadius::same(5))
                    ).clicked() {
                        ground_buf = value.to_string();
                    }
                }
            });

            // Water depth — only when ground == "water"
            if ground_buf == "water" {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("WATER DEPTH (blocks)").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
                ui.add_space(4.0);
                ui.add(egui::Slider::new(&mut water_depth_buf, 1..=8));
            }

            // Time of day
            ui.add_space(14.0);
            ui.label(egui::RichText::new("TIME").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (label, value) in &[("Cycle", "cycle"), ("Day", "day"), ("Night", "night")] {
                    let selected = time_lock_buf == *value;
                    let fg = if selected { egui::Color32::from_rgb(255, 213, 120) } else { egui::Color32::from_rgb(160, 160, 160) };
                    if ui.add(
                        egui::Button::new(egui::RichText::new(*label).size(13.0).color(fg))
                            .min_size(egui::vec2(112.0, 30.0))
                            .fill(if selected { egui::Color32::from_rgb(60, 50, 20) } else { egui::Color32::from_rgb(30, 33, 40) })
                            .corner_radius(egui::CornerRadius::same(5))
                    ).clicked() {
                        time_lock_buf = value.to_string();
                    }
                }
            });

            // Mobs on/off
            ui.add_space(14.0);
            ui.label(egui::RichText::new("MOBS").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let on_color  = if mobs_buf  { egui::Color32::from_rgb(143, 199, 106) } else { egui::Color32::from_rgb(100, 100, 100) };
                let off_color = if !mobs_buf { egui::Color32::from_rgb(199, 143, 106) } else { egui::Color32::from_rgb(100, 100, 100) };
                if ui.add(egui::Button::new(egui::RichText::new("ON").size(14.0).color(on_color))
                    .min_size(egui::vec2(175.0, 36.0))
                    .fill(if mobs_buf { egui::Color32::from_rgb(35, 60, 25) } else { egui::Color32::from_rgb(30, 33, 40) })
                    .corner_radius(egui::CornerRadius::same(6))
                ).clicked() {
                    mobs_buf = true;
                }
                if ui.add(egui::Button::new(egui::RichText::new("OFF").size(14.0).color(off_color))
                    .min_size(egui::vec2(175.0, 36.0))
                    .fill(if !mobs_buf { egui::Color32::from_rgb(60, 35, 25) } else { egui::Color32::from_rgb(30, 33, 40) })
                    .corner_radius(egui::CornerRadius::same(6))
                ).clicked() {
                    mobs_buf = false;
                }
            });
        }

        // Commands toggle (T / `/` chat overlay enables /give /time /gamemode
        // /tp /seed /clear /help when ON). Default: ON. Per-world setting,
        // stored in WorldMeta.commands_enabled.
        ui.add_space(14.0);
        ui.label(egui::RichText::new("COMMANDS").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let on_color = if commands_buf { egui::Color32::from_rgb(143, 199, 106) } else { egui::Color32::from_rgb(100, 100, 100) };
            let off_color = if !commands_buf { egui::Color32::from_rgb(199, 143, 106) } else { egui::Color32::from_rgb(100, 100, 100) };
            if ui.add(egui::Button::new(egui::RichText::new("ON").size(14.0).color(on_color))
                .min_size(egui::vec2(175.0, 36.0))
                .fill(if commands_buf { egui::Color32::from_rgb(35, 60, 25) } else { egui::Color32::from_rgb(30, 33, 40) })
                .corner_radius(egui::CornerRadius::same(6))
            ).clicked() {
                commands_buf = true;
            }
            if ui.add(egui::Button::new(egui::RichText::new("OFF").size(14.0).color(off_color))
                .min_size(egui::vec2(175.0, 36.0))
                .fill(if !commands_buf { egui::Color32::from_rgb(60, 35, 25) } else { egui::Color32::from_rgb(30, 33, 40) })
                .corner_radius(egui::CornerRadius::same(6))
            ).clicked() {
                commands_buf = false;
            }
        });

        // Stash toggle — per-world cloud save, default OFF (privacy-first).
        // OFF keeps the world on this computer; ON makes it open on any
        // computer (pushed to the player's Stash on save). Stored in
        // WorldMeta.cloud_save. Spec: 2026-06-02-stash-opt-in-toggle-design.md
        // Native-only: web is a login-free local sandbox with no Stash — the
        // lobby nudges to the desktop app instead. On web `cloud_buf` stays at
        // its default (OFF), so web worlds are always local.
        #[cfg(not(target_arch = "wasm32"))]
        {
            ui.add_space(14.0);
            ui.label(egui::RichText::new("SAVE TO YOUR STASH").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(if cloud_buf {
                    "On — open this world on any computer"
                } else {
                    "Off — this world stays on this computer"
                }).size(11.0).color(egui::Color32::from_rgb(85, 85, 102)));
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let on_color = if cloud_buf { egui::Color32::from_rgb(143, 199, 106) } else { egui::Color32::from_rgb(100, 100, 100) };
                let off_color = if !cloud_buf { egui::Color32::from_rgb(199, 143, 106) } else { egui::Color32::from_rgb(100, 100, 100) };
                if ui.add(egui::Button::new(egui::RichText::new("📦 ON").size(14.0).color(on_color))
                    .min_size(egui::vec2(175.0, 36.0))
                    .fill(if cloud_buf { egui::Color32::from_rgb(35, 60, 25) } else { egui::Color32::from_rgb(30, 33, 40) })
                    .corner_radius(egui::CornerRadius::same(6))
                ).clicked() {
                    cloud_buf = true;
                }
                if ui.add(egui::Button::new(egui::RichText::new("OFF").size(14.0).color(off_color))
                    .min_size(egui::vec2(175.0, 36.0))
                    .fill(if !cloud_buf { egui::Color32::from_rgb(60, 35, 25) } else { egui::Color32::from_rgb(30, 33, 40) })
                    .corner_radius(egui::CornerRadius::same(6))
                ).clicked() {
                    cloud_buf = false;
                }
            });
        }

        ui.add_space(20.0);

        // Buttons
        ui.horizontal(|ui| {
            let label = if name_buf.trim().is_empty() { "Create & Play (auto-name)" } else { "Create & Play" };
            let create_btn = egui::Button::new(
                egui::RichText::new(label)
                    .size(15.0)
                    .color(egui::Color32::from_rgb(184, 232, 184))
                    .strong(),
            )
            .min_size(egui::vec2(200.0, 42.0))
            .fill(egui::Color32::from_rgb(45, 107, 45))
            .corner_radius(egui::CornerRadius::same(6));
            if ui.add(create_btn).clicked() {
                confirmed = true;
            }

            if ui.add(
                egui::Button::new(egui::RichText::new("Cancel").size(14.0).color(TEXT_COLOR))
                    .min_size(egui::vec2(0.0, 42.0))
                    .fill(brand::SURFACE)
                    .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
                    .corner_radius(egui::CornerRadius::same(6)),
            ).clicked() {
                cancelled = true;
            }
        });

        // This return is overridden below
        DialogResult::Update(name_buf.clone(), seed_buf.clone())
    });

    // Write back toggles so the caller can persist them in MenuDialog state
    *creative = creative_buf;
    *commands_enabled = commands_buf;
    *cloud_save = cloud_buf;
    *world_type = world_type_buf;
    *ground = ground_buf;
    *water_depth = water_depth_buf;
    *time_lock = time_lock_buf;
    *mobs_enabled = mobs_buf;

    if confirmed {
        DialogResult::Confirm(name_buf, seed_buf)
    } else if cancelled {
        DialogResult::Cancel
    } else {
        DialogResult::Update(name_buf, seed_buf)
    }
}

fn draw_edit_dialog(ctx: &egui::Context, name: &str, description: &str) -> DialogResult {
    let mut name_buf = name.to_string();
    let mut desc_buf = description.to_string();
    let mut confirmed = false;
    let mut cancelled = false;

    draw_dialog_frame(ctx, "Edit World", TITLE_COLOR, CARD_BORDER, |ui| {
        // Name
        ui.label(egui::RichText::new("NAME").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
        ui.add_space(4.0);
        menu_text_field(ui, &mut name_buf, "World name", "", 360.0, 15.0);

        ui.add_space(14.0);

        // Description
        ui.label(egui::RichText::new("DESCRIPTION").size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("(optional)").size(11.0).color(egui::Color32::from_rgb(85, 85, 102)));
        });
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::multiline(&mut desc_buf)
                .desired_width(360.0)
                .desired_rows(3)
                .font(egui::FontId::proportional(14.0)),
        );

        ui.add_space(20.0);

        // Buttons
        ui.horizontal(|ui| {
            let save_btn = egui::Button::new(
                egui::RichText::new("Save").size(15.0).color(egui::Color32::from_rgb(168, 200, 232)).strong(),
            )
            .min_size(egui::vec2(200.0, 42.0))
            .fill(egui::Color32::from_rgb(45, 74, 107))
            .corner_radius(egui::CornerRadius::same(6));
            if ui.add(save_btn).clicked() {
                confirmed = true;
            }

            if ui.add(
                egui::Button::new(egui::RichText::new("Cancel").size(14.0).color(TEXT_COLOR))
                    .min_size(egui::vec2(0.0, 42.0))
                    .fill(brand::SURFACE)
                    .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
                    .corner_radius(egui::CornerRadius::same(6)),
            ).clicked() {
                cancelled = true;
            }
        });

        DialogResult::Update(name_buf.clone(), desc_buf.clone())
    });

    if confirmed {
        DialogResult::Confirm(name_buf, desc_buf)
    } else if cancelled {
        DialogResult::Cancel
    } else {
        DialogResult::Update(name_buf, desc_buf)
    }
}

fn draw_delete_dialog(ctx: &egui::Context, world_name: &str, confirm_text: &str) -> DialogResult {
    let mut text_buf = confirm_text.to_string();
    let mut confirmed = false;
    let mut cancelled = false;
    let border = egui::Color32::from_rgb(90, 40, 40);

    draw_dialog_frame(ctx, "\u{26A0} Delete World", DELETE_RED, border, |ui| {
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(format!("This will permanently delete \"{}\" and all its data.", world_name))
                    .size(14.0)
                    .color(egui::Color32::from_rgb(170, 170, 170)),
            );
            ui.label(
                egui::RichText::new("This cannot be undone.")
                    .size(14.0)
                    .color(egui::Color32::from_rgb(170, 170, 170))
                    .strong(),
            );
        });

        ui.add_space(16.0);

        // Type to confirm
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Type ").size(12.0).color(egui::Color32::from_rgb(136, 136, 153)));
            ui.label(
                egui::RichText::new(world_name)
                    .size(12.0)
                    .color(egui::Color32::from_rgb(232, 160, 160))
                    .background_color(egui::Color32::from_rgb(42, 24, 24))
                    .strong(),
            );
            ui.label(egui::RichText::new(" to confirm").size(12.0).color(egui::Color32::from_rgb(136, 136, 153)));
        });
        ui.add_space(6.0);
        menu_text_field(ui, &mut text_buf, "Type the world name to confirm", "", 360.0, 15.0);

        ui.add_space(20.0);

        let matches = text_buf.trim() == world_name;

        ui.horizontal(|ui| {
            let del_btn = egui::Button::new(
                egui::RichText::new("Delete Forever")
                    .size(15.0)
                    .color(if matches { egui::Color32::from_rgb(255, 180, 180) } else { egui::Color32::from_rgb(100, 68, 68) })
                    .strong(),
            )
            .min_size(egui::vec2(200.0, 42.0))
            .fill(if matches { egui::Color32::from_rgb(140, 30, 30) } else { egui::Color32::from_rgb(58, 24, 24) })
            .corner_radius(egui::CornerRadius::same(6));
            let resp = ui.add(del_btn);
            if resp.clicked() && matches {
                confirmed = true;
            }

            if ui.add(
                egui::Button::new(egui::RichText::new("Cancel").size(14.0).color(TEXT_COLOR))
                    .min_size(egui::vec2(0.0, 42.0))
                    .fill(brand::SURFACE)
                    .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
                    .corner_radius(egui::CornerRadius::same(6)),
            ).clicked() {
                cancelled = true;
            }
        });

        if !matches {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("Button enables when the name matches exactly").size(11.0).color(egui::Color32::from_rgb(85, 68, 68)));
            });
        }

        DialogResult::Update(text_buf.clone(), String::new())
    });

    if confirmed {
        DialogResult::ConfirmSingle
    } else if cancelled {
        DialogResult::Cancel
    } else {
        DialogResult::Update(text_buf, String::new())
    }
}

/// What the Join Game dialog needs to show My Keeps and hand back a Join (native only).
#[cfg(not(target_arch = "wasm32"))]
struct JoinKeepsPane<'a> {
    keeps: &'a mut Vec<crate::keeps::Keep>,
    forget_armed: &'a mut Option<String>,
    /// A compact card shows only the newest Keeps until this is set (More Keeps / Fewer Keeps).
    expanded: &'a mut bool,
    /// The saved-Keeps file a completed Forget removes the Keep from.
    file: &'a std::path::Path,
    /// Set to `JoinKeep` when a row's Join was tapped; the caller closes the dialog and dials.
    join: &'a mut MenuAction,
}

/// The tallest the My Keeps rows grow in the dialog on a window tall enough not to scroll the
/// whole card (under the text box); past it they scroll on their own.
#[cfg(not(target_arch = "wasm32"))]
const JOIN_KEEPS_MAX_HEIGHT: f32 = 190.0;

/// A compact (phone) dialog puts My Keeps first and shows this many of the newest, so twenty
/// saved Keeps never push the room code field off the screen. The rest wait behind a "More
/// Keeps" button; there is no scroll area of their own (a swipe on the rows would be caught
/// by it instead of scrolling the card).
#[cfg(not(target_arch = "wasm32"))]
const JOIN_KEEPS_COMPACT_SHOWN: usize = 3;

fn draw_join_dialog(
    ctx: &egui::Context,
    address: &str,
    #[cfg(not(target_arch = "wasm32"))] lan_games: &crate::lan_ui::LanGames,
    // A click on a heard LAN room: the caller joins it straight away.
    #[cfg(not(target_arch = "wasm32"))] picked: &mut Option<decented_net::LanRoom>,
    #[cfg(not(target_arch = "wasm32"))] keeps_pane: JoinKeepsPane<'_>,
) -> DialogResult {
    let mut addr_buf = address.to_string();
    let mut confirmed = false;
    let mut cancelled = false;

    let _result = draw_dialog_frame(ctx, "Join Game", ACTION_BLUE, CARD_BORDER, |ui| {
        // A phone's card is short: a saved Keep is the quickest way in, so on a compact card
        // My Keeps comes first (the newest few, then a More Keeps button), above the LAN rooms
        // and the code field. A tall window keeps them under the text box.
        #[cfg(not(target_arch = "wasm32"))]
        let compact = dialog_is_compact(ctx);
        #[cfg(not(target_arch = "wasm32"))]
        if compact && !keeps_pane.keeps.is_empty() {
            let action = draw_my_keeps(
                ui,
                keeps_pane.keeps,
                keeps_pane.forget_armed,
                keeps_pane.file,
                KeepsLayout::Folded(&mut *keeps_pane.expanded),
                None,
            );
            if matches!(action, MenuAction::JoinKeep(_)) {
                *keeps_pane.join = action;
            }
            ui.add_space(14.0);
        }
        // LAN games: the common case is a friend on the same network.
        #[cfg(not(target_arch = "wasm32"))]
        {
            *picked = crate::lan_ui::draw_lan_games(ui, lan_games);
        }
        // The room code (native), or a server address.
        #[cfg(not(target_arch = "wasm32"))]
        let (heading, label, hint) =
            (crate::lan_ui::CODE_HEADING, "Room code", crate::lan_ui::CODE_HINT);
        #[cfg(target_arch = "wasm32")]
        let (heading, label, hint) = ("SERVER ADDRESS", "Server address", "192.168.1.10:7700");
        ui.label(egui::RichText::new(heading).size(11.0).color(egui::Color32::from_rgb(136, 136, 153)).strong());
        ui.add_space(4.0);
        let addr_response = menu_text_field(ui, &mut addr_buf, label, hint, 360.0, 15.0);
        // Auto-focus the field
        addr_response.request_focus();
        #[cfg(not(target_arch = "wasm32"))]
        {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(crate::lan_ui::CODE_HELP)
                    .size(12.0)
                    .color(egui::Color32::from_rgb(150, 150, 160)),
            );
            // My Keeps on a tall window: under the text box, rows in a scroll area of their own.
            if !compact && !keeps_pane.keeps.is_empty() {
                ui.add_space(14.0);
                let action = draw_my_keeps(
                    ui,
                    keeps_pane.keeps,
                    keeps_pane.forget_armed,
                    keeps_pane.file,
                    KeepsLayout::Scroll(JOIN_KEEPS_MAX_HEIGHT),
                    None,
                );
                if matches!(action, MenuAction::JoinKeep(_)) {
                    *keeps_pane.join = action;
                }
            }
        }

        ui.add_space(20.0);

        // Buttons
        ui.horizontal(|ui| {
            let connect_btn = egui::Button::new(
                egui::RichText::new("Connect")
                    .size(15.0)
                    .color(egui::Color32::from_rgb(184, 212, 255))
                    .strong(),
            )
            .min_size(egui::vec2(200.0, 42.0))
            .fill(egui::Color32::from_rgb(30, 60, 120))
            .corner_radius(egui::CornerRadius::same(6));
            if ui.add(connect_btn).clicked() {
                confirmed = true;
            }

            if ui.add(
                egui::Button::new(egui::RichText::new("Cancel").size(14.0).color(TEXT_COLOR))
                    .min_size(egui::vec2(0.0, 42.0))
                    .fill(brand::SURFACE)
                    .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
                    .corner_radius(egui::CornerRadius::same(6)),
            ).clicked() {
                cancelled = true;
            }
        });

        DialogResult::Update(addr_buf.clone(), String::new())
    });

    if confirmed {
        DialogResult::Confirm(addr_buf, String::new())
    } else if cancelled {
        DialogResult::Cancel
    } else {
        DialogResult::Update(addr_buf, String::new())
    }
}

// ---------------------------------------------------------------------------
// Pause Menu (simplified — no delete)
// ---------------------------------------------------------------------------

pub const PAUSE_RESUME: i32 = 0;
pub const PAUSE_SAVE: i32 = 1;
pub const PAUSE_SAVE_QUIT: i32 = 2;
pub const PAUSE_QUIT_NO_SAVE: i32 = 3;
pub const PAUSE_DIFFICULTY_CHANGED: i32 = 4;
pub const PAUSE_SWITCH_CREATIVE: i32 = 5;
/// Open the "Your look" skin-customisation panel (cross-platform — native
/// uploads via an OS PNG picker, WASM via the web file dialog + Stash).
pub const PAUSE_CUSTOMISE_SKIN: i32 = 6;
/// The Stash toggle was flipped — caller persists the new `cloud_save` value
/// (the menu mutated it in place) and keeps the pause menu open.
pub const PAUSE_STASH_TOGGLED: i32 = 7;
/// Open the Settings panel (Spec 39; labelled "Graphics" until the panel also held sound, chat and camera). Cross-platform.
pub const PAUSE_OPEN_SETTINGS: i32 = 8;
/// Trial pause menu: restart the active trial/challenge in place (fresh run).
pub const PAUSE_TRY_AGAIN: i32 = 9;
/// Trial pause menu: leave the trial back to the lobby without saving (trials
/// are ephemeral — there is nothing to save).
pub const PAUSE_LEAVE_TRIAL: i32 = 10;
/// Reopen the controls card (`controls.rs`) over the world. Every pause menu
/// (normal and Trial). Cross-platform; the card picks the touch or keyboard table.
pub const PAUSE_OPEN_CONTROLS: i32 = 11;
/// Player leave: value is 100 + player_index (100 = P1 leaves, 101 = P2 leaves)
pub const PAUSE_PLAYER_LEAVE_BASE: i32 = 100;

const PAUSE_LABELS: &[&str] = &["Resume", "Save", "Save and Quit", "Quit Without Saving"];
const PAUSE_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(80, 180, 80),   // Resume — green
    egui::Color32::from_rgb(120, 150, 220),  // Save — blue
    egui::Color32::from_rgb(220, 170, 70),   // Save & Quit — orange
    egui::Color32::from_rgb(180, 80, 80),    // Quit no save — red
];

/// Draw the pause menu overlay. Returns the button index clicked, or -1.
/// `confirm_quit` tracks whether the "are you sure?" prompt is showing.
/// `is_trial` collapses the menu to just Resume / Try Again / Leave — a Trial
/// (coverage Challenge or Race) is an ephemeral, retryable run, so Save,
/// difficulty and creative-switch don't apply. `kept_autosave` is the age of
/// the crash-recovery autosave "Quit Without Saving" would keep (after a failed
/// save, or in a session opened from it — `world_exit::SessionSaves`); the
/// button and its warning then say so rather than promise a discard.
pub fn draw_pause_menu(ctx: &egui::Context, world_name: &str, confirm_quit: &mut bool, confirm_creative: &mut bool, difficulty: &mut String, is_creative: bool, lock_creative: bool, num_players: usize, cloud_save: &mut bool, is_trial: bool, kept_autosave: Option<&str>) -> i32 {
    let mut clicked = -1i32;
    // On a phone (the card rule of the dialogs) the leave buttons stay pinned at the top and
    // the other controls scroll under them, so nothing that leaves the menu is off screen.
    let compact = dialog_is_compact(ctx);
    let confirming = *confirm_quit || *confirm_creative;

    // Semi-transparent overlay
    egui::Area::new(egui::Id::new("pause_overlay"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .interactable(false)
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            let screen = ui.max_rect();
            ui.painter().rect_filled(screen, 0.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 200));
        });

    // Top-level panel — see the `#[allow(deprecated)]` note near line 2492
    // (no non-deprecated top-level entry point for CentralPanel in egui 0.34).
    #[allow(deprecated)]
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(if compact { 8.0 } else { 100.0 });

                ui.label(
                    egui::RichText::new("Paused")
                        .size(if compact { 26.0 } else { 36.0 })
                        .color(egui::Color32::from_rgb(220, 220, 220))
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("World: {world_name}"))
                        .size(14.0)
                        .color(SUBTITLE_COLOR),
                );
                ui.add_space(if compact { 8.0 } else { 16.0 });

                // One body, drawn in two parts on a compact screen: the buttons that leave the
                // pause menu (`Leave`), pinned above, and the rest (`Rest`: difficulty, Stash,
                // Controls, Graphics, Your look, split-screen, creative), which scrolls under
                // them. A tall window draws both parts, in the order they always had (`All`).
                let button_width = 260.0;
                let mut body = |ui: &mut egui::Ui, part: PausePart| {
                let leave = part != PausePart::Rest;
                let rest = part != PausePart::Leave;

                // Difficulty selector — hidden during a Trial (the scenario fixes it).
                if !is_trial && rest {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Difficulty").size(13.0).color(egui::Color32::from_rgb(140, 140, 155)));
                ui.add_space(6.0);
                // The row's own width, centred, on a compact screen: a bare `horizontal` in a
                // centred column starts at the screen's left edge and ran off it on a phone.
                let difficulty_row = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
                    if compact {
                        ui.allocate_ui_with_layout(
                            egui::vec2(PAUSE_DIFFICULTY_ROW_WIDTH, 30.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| add(ui),
                        );
                    } else {
                        ui.horizontal(|ui| add(ui));
                    }
                };
                difficulty_row(ui, &mut |ui| {
                    for &(label, color) in &[
                        ("Peaceful", egui::Color32::from_rgb(120, 200, 120)),
                        ("Easy", egui::Color32::from_rgb(180, 200, 100)),
                        ("Normal", egui::Color32::from_rgb(200, 180, 80)),
                        ("Hard", egui::Color32::from_rgb(220, 100, 80)),
                    ] {
                        let level = label.to_lowercase();
                        let is_current = *difficulty == level;
                        let btn = egui::Button::new(
                            egui::RichText::new(label).size(12.0).color(if is_current { color } else { egui::Color32::from_rgb(90, 90, 90) }),
                        )
                        .min_size(egui::vec2(60.0, 30.0))
                        .fill(if is_current { egui::Color32::from_rgb(35, 38, 50) } else { egui::Color32::from_rgb(22, 25, 32) })
                        .corner_radius(egui::CornerRadius::same(4));
                        if ui.add(btn).clicked() && !is_current {
                            *difficulty = level;
                            clicked = PAUSE_DIFFICULTY_CHANGED;
                        }
                    }
                });
                ui.add_space(16.0);
                }

                if *confirm_quit {
                    if leave {
                    // "Are you sure?" confirmation
                    ui.label(
                        egui::RichText::new(crate::world_exit::quit_no_save_warning(kept_autosave))
                            .size(15.0)
                            .color(egui::Color32::from_rgb(220, 140, 140))
                            .strong(),
                    );
                    ui.add_space(16.0);

                    let yes_btn = egui::Button::new(
                        egui::RichText::new("Yes, Quit")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(255, 120, 120)),
                    )
                    .min_size(egui::vec2(button_width, 42.0))
                    .fill(egui::Color32::from_rgb(80, 25, 25))
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(120, 40, 40)));
                    if ui.add(yes_btn).clicked() {
                        clicked = PAUSE_QUIT_NO_SAVE;
                    }
                    ui.add_space(4.0);

                    let no_btn = egui::Button::new(
                        egui::RichText::new("Go Back")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(150, 150, 150)),
                    )
                    .min_size(egui::vec2(button_width, 42.0))
                    .fill(PANEL_BG)
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 50, 60)));
                    if ui.add(no_btn).clicked() {
                        *confirm_quit = false;
                    }
                    }
                } else if *confirm_creative {
                    if leave {
                    // Creative mode confirmation
                    ui.label(
                        egui::RichText::new("This world will be permanently marked as creative-touched.")
                            .size(15.0)
                            .color(egui::Color32::from_rgb(140, 180, 220))
                            .strong(),
                    );
                    ui.add_space(16.0);

                    let yes_btn = egui::Button::new(
                        egui::RichText::new("Yes, Switch to Creative")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(120, 180, 255)),
                    )
                    .min_size(egui::vec2(button_width, 42.0))
                    .fill(egui::Color32::from_rgb(25, 45, 75))
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(60, 90, 140)));
                    if ui.add(yes_btn).clicked() {
                        clicked = PAUSE_SWITCH_CREATIVE;
                    }
                    ui.add_space(4.0);

                    let no_btn = egui::Button::new(
                        egui::RichText::new("Go Back")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(150, 150, 150)),
                    )
                    .min_size(egui::vec2(button_width, 42.0))
                    .fill(PANEL_BG)
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 50, 60)));
                    if ui.add(no_btn).clicked() {
                        *confirm_creative = false;
                    }
                    }
                } else if is_trial {
                    if leave {
                    // A Trial is an ephemeral, retryable run — there's nothing to
                    // save. Collapse the menu to Resume / Try Again / Leave.
                    let resume_btn = egui::Button::new(
                        egui::RichText::new("Resume")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(80, 180, 80)),
                    )
                    .min_size(egui::vec2(button_width, 42.0))
                    .fill(PANEL_BG)
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 50, 60)));
                    if ui.add(resume_btn).clicked() {
                        clicked = PAUSE_RESUME;
                    }
                    ui.add_space(4.0);

                    let retry_btn = egui::Button::new(
                        egui::RichText::new("\u{21bb} Try Again")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(120, 180, 255)),
                    )
                    .min_size(egui::vec2(button_width, 42.0))
                    .fill(PANEL_BG)
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 60, 80)));
                    if ui.add(retry_btn).clicked() {
                        clicked = PAUSE_TRY_AGAIN;
                    }
                    ui.add_space(4.0);

                    let leave_btn = egui::Button::new(
                        egui::RichText::new("\u{2190} Leave")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(220, 170, 70)),
                    )
                    .min_size(egui::vec2(button_width, 42.0))
                    .fill(PANEL_BG)
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 55, 30)));
                    if ui.add(leave_btn).clicked() {
                        clicked = PAUSE_LEAVE_TRIAL;
                    }
                    ui.add_space(4.0);

                    let controls_btn = egui::Button::new(
                        egui::RichText::new("Controls")
                            .size(16.0)
                            .color(egui::Color32::from_rgb(150, 190, 230)),
                    )
                    .min_size(egui::vec2(button_width, 42.0))
                    .fill(PANEL_BG)
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 60, 80)));
                    if ui.add(controls_btn).clicked() {
                        clicked = PAUSE_OPEN_CONTROLS;
                    }
                    }
                } else {
                    // Stash toggle — flip whether this world also saves online.
                    // Native-only: web is a local sandbox with no Stash (the lobby
                    // nudges to the desktop app). On web the Save buttons below just
                    // save locally and *cloud_save stays false.
                    #[cfg(not(target_arch = "wasm32"))]
                    if rest {
                        let stash_label = if *cloud_save {
                            "📦 Stash: ON — also saved online (tap for off)"
                        } else {
                            "📦 Stash: OFF — only on this computer (tap for on)"
                        };
                        let stash_btn = egui::Button::new(
                            egui::RichText::new(stash_label)
                                .size(14.0)
                                .color(if *cloud_save { egui::Color32::from_rgb(140, 210, 140) } else { egui::Color32::from_rgb(170, 170, 180) }),
                        )
                        .min_size(egui::vec2(button_width, 38.0))
                        .fill(if *cloud_save { egui::Color32::from_rgb(28, 48, 28) } else { PANEL_BG })
                        .corner_radius(egui::CornerRadius::same(4))
                        .stroke(egui::Stroke::new(1.0_f32, if *cloud_save { egui::Color32::from_rgb(60, 110, 60) } else { egui::Color32::from_rgb(50, 50, 60) }));
                        if ui.add(stash_btn).clicked() {
                            *cloud_save = !*cloud_save;
                            clicked = PAUSE_STASH_TOGGLED;
                        }
                        ui.add_space(8.0);
                    }

                    // Normal pause menu buttons. Save / Save and Quit reflect the
                    // Stash state so the player sees exactly what will happen.
                    if leave {
                    for (i, (&base_label, &color)) in PAUSE_LABELS.iter().zip(PAUSE_COLORS.iter()).enumerate() {
                        let label: String = if i as i32 == PAUSE_SAVE {
                            if *cloud_save { "Save & Stash" } else { "Save" }.to_string()
                        } else if i as i32 == PAUSE_SAVE_QUIT {
                            if *cloud_save { "Save, Stash & Quit" } else { "Save and Quit" }.to_string()
                        } else if i as i32 == PAUSE_QUIT_NO_SAVE {
                            crate::world_exit::quit_no_save_label(kept_autosave)
                        } else {
                            base_label.to_string()
                        };
                        let button = egui::Button::new(
                            egui::RichText::new(label)
                                .size(16.0)
                                .color(color),
                        )
                        .min_size(egui::vec2(button_width, 42.0))
                        .fill(PANEL_BG)
                        .corner_radius(egui::CornerRadius::same(4))
                        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 50, 60)));

                        if ui.add(button).clicked() {
                            if i as i32 == PAUSE_QUIT_NO_SAVE {
                                *confirm_quit = true;
                            } else {
                                clicked = i as i32;
                            }
                        }
                        ui.add_space(4.0);
                    }
                    }

                    if rest {
                    // Controls — reopens the controls card (first-spawn card +
                    // H help sheet share one table: `controls.rs`). Cross-platform.
                    {
                        let controls_btn = egui::Button::new(
                            egui::RichText::new("Controls")
                                .size(16.0)
                                .color(egui::Color32::from_rgb(150, 190, 230)),
                        )
                        .min_size(egui::vec2(button_width, 42.0))
                        .fill(PANEL_BG)
                        .corner_radius(egui::CornerRadius::same(4))
                        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 60, 80)));
                        if ui.add(controls_btn).clicked() {
                            clicked = PAUSE_OPEN_CONTROLS;
                        }
                        ui.add_space(4.0);
                    }

                    // Graphics settings (Spec 39) — cross-platform. Opens the
                    // quality-dials panel in place of the pause buttons.
                    {
                        let gfx_btn = egui::Button::new(
                            egui::RichText::new("Settings")
                                .size(16.0)
                                .color(egui::Color32::from_rgb(150, 200, 160)),
                        )
                        .min_size(egui::vec2(button_width, 42.0))
                        .fill(PANEL_BG)
                        .corner_radius(egui::CornerRadius::same(4))
                        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 60, 52)));
                        if ui.add(gfx_btn).clicked() {
                            clicked = PAUSE_OPEN_SETTINGS;
                        }
                        ui.add_space(4.0);
                    }

                    // "Your look" — skin customisation. Cross-platform: WASM
                    // uploads + Stashes (PWA); native uploads via an OS PNG picker
                    // and live-applies (persistence is a native follow-up). The
                    // panel itself (`draw_skin_panel`) is shared.
                    {
                        let look_btn = egui::Button::new(
                            egui::RichText::new("Your look")
                                .size(16.0)
                                .color(egui::Color32::from_rgb(180, 150, 220)),
                        )
                        .min_size(egui::vec2(button_width, 42.0))
                        .fill(PANEL_BG)
                        .corner_radius(egui::CornerRadius::same(4))
                        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 50, 60)));
                        if ui.add(look_btn).clicked() {
                            clicked = PAUSE_CUSTOMISE_SKIN;
                        }
                        ui.add_space(4.0);
                    }

                    // Player leave buttons (only in split-screen)
                    if num_players >= 2 {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Split-Screen")
                                .size(13.0)
                                .color(egui::Color32::from_rgb(140, 140, 155)),
                        );
                        ui.add_space(4.0);
                        for pi in 0..num_players {
                            let label = format!("Player {}: Leave", pi + 1);
                            let btn = egui::Button::new(
                                egui::RichText::new(&label)
                                    .size(14.0)
                                    .color(egui::Color32::from_rgb(200, 160, 100)),
                            )
                            .min_size(egui::vec2(button_width, 36.0))
                            .fill(egui::Color32::from_rgb(30, 28, 22))
                            .corner_radius(egui::CornerRadius::same(4))
                            .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 55, 30)));
                            if ui.add(btn).clicked() {
                                clicked = PAUSE_PLAYER_LEAVE_BASE + pi as i32;
                            }
                            ui.add_space(2.0);
                        }
                    }

                    // Switch to Creative button (only in survival worlds, and
                    // NOT while a scenario that locks creative is active — a
                    // competition can't be cheated into creative mid-run).
                    if !is_creative && !lock_creative {
                        ui.add_space(8.0);
                        let creative_btn = egui::Button::new(
                            egui::RichText::new("Switch to Creative")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(106, 176, 199)),
                        )
                        .min_size(egui::vec2(button_width, 36.0))
                        .fill(egui::Color32::from_rgb(22, 30, 40))
                        .corner_radius(egui::CornerRadius::same(4))
                        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(40, 60, 80)));
                        if ui.add(creative_btn).clicked() {
                            *confirm_creative = true;
                        }
                    }
                    }
                }
                };

                if compact {
                    body(ui, PausePart::Leave);
                    // The rest scrolls in what is left of the screen; a confirm prompt has
                    // nothing under it.
                    if !confirming {
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .max_height((ui.available_height() - 8.0).max(60.0))
                            .auto_shrink([false, true])
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| body(ui, PausePart::Rest));
                            });
                    }
                } else {
                    body(ui, PausePart::All);
                }
            });
        });

    clicked
}

/// Which of the pause menu's two parts to draw (see `draw_pause_menu`).
#[derive(Clone, Copy, PartialEq)]
enum PausePart {
    /// The buttons that leave the pause menu: Resume, Save, Save and Quit, Quit Without
    /// Saving, the confirm prompts, and the Trial's buttons.
    Leave,
    /// Everything else.
    Rest,
    /// Both, in the order a desktop window always drew them.
    All,
}

/// The width of the difficulty row on a compact screen: four buttons and their gaps.
const PAUSE_DIFFICULTY_ROW_WIDTH: f32 = 290.0;

// ---------------------------------------------------------------------------
// "Your look" panel — BYO 64×64 skin (Phase 2 cosmetics)
// ---------------------------------------------------------------------------

/// Lifecycle of the upload flow, surfaced as a status line in the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum SkinStatus {
    Idle,
    Picking,
    /// A new skin (upload or preset) was applied. The bool is whether it was
    /// stashed to the cloud (true) or only saved on this device (false), so the
    /// status line can be honest about where the skin lives.
    Applied(bool),
    Error(String),
}

/// What the game should do this frame in response to the "Your look" wardrobe.
#[derive(Clone, Debug, PartialEq)]
pub enum SkinPanelAction {
    None,
    Back,
    /// Wear (equip) a wardrobe entry.
    Wear(crate::skin_wardrobe::SkinId),
    /// Select (highlight) an entry without wearing it.
    Select(crate::skin_wardrobe::SkinId),
    /// Open the Skin Studio editing this entry (Save writes back to it).
    Edit(crate::skin_wardrobe::SkinId),
    /// Add a blank entry and immediately open it in the Studio.
    NewBlank,
    /// Add a new entry recoloured from a `tinted_default_skin_rgba` variant.
    NewFromColor(u32),
    /// Add a new entry from an uploaded 64×64 PNG (opens the file picker).
    Upload,
    /// Fork an entry into an independent copy.
    Duplicate(crate::skin_wardrobe::SkinId),
    /// Delete an entry (the panel guards against the last/equipped via state).
    Delete(crate::skin_wardrobe::SkinId),
    /// Begin inline-renaming an entry (seed the rename buffer with its name).
    BeginRename(crate::skin_wardrobe::SkinId),
    /// Commit the inline rename to this entry.
    CommitRename(crate::skin_wardrobe::SkinId, String),
    /// Export a wardrobe entry as a Minecraft 64×64 PNG.
    ExportPng(crate::skin_wardrobe::SkinId),
    /// Open the "Bring in a Minecraft skin" dialog.
    ImportFromMinecraft,
    /// Submit the typed username to fetch.
    CommitMcImport(String),
    /// Close the import dialog without fetching.
    CancelMcImport,
    /// Re-pull an imported entry's current skin by its stored UUID.
    Refresh(crate::skin_wardrobe::SkinId),
    /// Switch an entry between the Classic and Slim ("Alex") arm models.
    SetArmModel(crate::skin_wardrobe::SkinId, crate::skin_uv::ArmModel),
    /// Dismiss the in-dialog error (back to the editable username field).
    DismissMcError,
}

/// Build the download filename for an exported skin: `axenstax-<slug>.png`,
/// lowercased, non-alphanumeric runs → single hyphens, trimmed. Empty/blank
/// names fall back to `axenstax-my-skin.png` (the UX "unsaved skin" default).
pub fn export_skin_filename(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true; // suppress leading hyphen
    for ch in name.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug = "my-skin".to_string();
    }
    format!("axenstax-{slug}.png")
}

/// UI state for the "Your look" wardrobe panel. The applied pixels live in the
/// game's `skin_wardrobe`; this only holds the grid's presentation state.
pub struct SkinPanelState {
    pub open: bool,
    pub status: SkinStatus,
    pub picking: bool,
    /// Which entry is highlighted in the grid (defaults to the equipped one,
    /// set by game_loop when the panel opens).
    pub selected: Option<crate::skin_wardrobe::SkinId>,
    /// The entry currently being inline-renamed, if any.
    pub renaming: Option<crate::skin_wardrobe::SkinId>,
    /// Scratch buffer for the inline rename field.
    pub rename_buf: String,
    /// Import-from-Minecraft dialog: open flag, username scratch, in-flight
    /// (spinner) flag, last structured error to show in the dialog, and a
    /// transient toast line shown above the grid.
    pub mc_import_open: bool,
    pub mc_import_buf: String,
    pub mc_import_busy: bool,
    pub mc_import_error: Option<crate::mc_import::McImportError>,
    pub mc_toast: Option<String>,
    /// Wall-clock seconds (`wardrobe_now()`) the toast was set, so the frame loop
    /// can expire it after a few seconds instead of pinning it forever.
    pub mc_toast_at: u64,
}

impl Default for SkinPanelState {
    fn default() -> Self {
        Self {
            open: false,
            status: SkinStatus::Idle,
            picking: false,
            selected: None,
            renaming: None,
            rename_buf: String::new(),
            mc_import_open: false,
            mc_import_buf: String::new(),
            mc_import_busy: false,
            mc_import_error: None,
            mc_toast: None,
            mc_toast_at: 0,
        }
    }
}

/// One grid cell the panel renders: a wardrobe entry + its 3D thumbnail texture.
pub struct WardrobeCell {
    pub id: crate::skin_wardrobe::SkinId,
    pub name: String,
    pub tex: egui::TextureId,
    pub equipped: bool,
    /// Display handle if this skin was imported from Minecraft (drives the
    /// "from Minecraft" subtitle + the refresh icon). None for local skins.
    pub minecraft_handle: Option<String>,
    /// Classic (4-px arms) or Slim ("Alex", 3-px) — drives the Arms toggle and
    /// the export tip, and is what the thumbnail was rendered on.
    pub arm_model: crate::skin_uv::ArmModel,
}

/// The "Your look" wardrobe: a grid of 3D mini-avatar thumbnails (one per skin)
/// with per-selection actions (Wear / Edit / Duplicate / Rename / Delete) and
/// bottom-row global actions (New / colour presets / Upload PNG / Back).
/// Pure egui — returns the action the game performs this frame.
pub fn draw_skin_panel(
    ctx: &egui::Context,
    state: &mut SkinPanelState,
    cells: &[WardrobeCell],
) -> SkinPanelAction {
    let mut action = SkinPanelAction::None;
    let selected = state.selected;
    let selected_is_equipped = cells.iter().any(|c| Some(c.id) == selected && c.equipped);
    let can_delete = cells.len() > 1 && !selected_is_equipped;

    egui::Window::new("Your look")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_max_width(420.0);

            // Transient "imported / refreshed" toast, above the grid.
            if let Some(t) = &state.mc_toast {
                ui.colored_label(egui::Color32::from_rgb(120, 200, 120), t.clone());
                ui.add_space(4.0);
            }

            // --- Grid of skins ---
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for cell in cells {
                        let is_sel = selected == Some(cell.id);
                        ui.allocate_ui(egui::vec2(96.0, 168.0), |ui| {
                            ui.vertical_centered(|ui| {
                                // Thumbnail (click = select).
                                let img = egui::Image::new((cell.tex, egui::vec2(88.0, 132.0)))
                                    .sense(egui::Sense::click());
                                let resp = ui.add(img);
                                if is_sel {
                                    ui.painter().rect_stroke(
                                        resp.rect.expand(2.0),
                                        4.0,
                                        egui::Stroke::new(2.0_f32, ACTION_BLUE),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                                if resp.clicked() {
                                    action = SkinPanelAction::Select(cell.id);
                                }
                                // Name (or inline rename field when renaming this one).
                                if state.renaming == Some(cell.id) {
                                    ui.horizontal(|ui| {
                                        let r = menu_text_field(
                                            ui, &mut state.rename_buf, "Rename skin", "name", 64.0, 12.0,
                                        );
                                        // Desktop: Enter while the field has focus commits.
                                        if r.lost_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                        {
                                            action = SkinPanelAction::CommitRename(
                                                cell.id, state.rename_buf.clone(),
                                            );
                                        }
                                        // Touch: the OS prompt returns immediately with no
                                        // focus to lose, so an explicit ✓ always commits.
                                        if ui
                                            .button(egui::RichText::new("✓").size(12.0).color(ACTION_BLUE))
                                            .clicked()
                                        {
                                            action = SkinPanelAction::CommitRename(
                                                cell.id, state.rename_buf.clone(),
                                            );
                                        }
                                    });
                                } else {
                                    let label = if cell.equipped {
                                        format!("{} ✓", cell.name)
                                    } else {
                                        cell.name.clone()
                                    };
                                    ui.label(egui::RichText::new(label).size(12.0).color(
                                        if cell.equipped { ACTION_BLUE } else { DIM_TEXT },
                                    ));
                                }
                                if let Some(handle) = &cell.minecraft_handle {
                                    let _ = handle;
                                    ui.label(
                                        egui::RichText::new("from Minecraft")
                                            .size(10.0)
                                            .italics()
                                            .color(DIM_TEXT),
                                    );
                                    if ui
                                        .button(egui::RichText::new("🔄").size(12.0))
                                        .on_hover_text(format!("Re-pull {}'s latest skin", cell.name))
                                        .clicked()
                                    {
                                        action = SkinPanelAction::Refresh(cell.id);
                                    }
                                }
                            });
                        });
                    }
                    // Pinned "Bring in a Minecraft skin" card (kid-UX §2a).
                    ui.allocate_ui(egui::vec2(96.0, 168.0), |ui| {
                        ui.vertical_centered(|ui| {
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(88.0, 132.0),
                                egui::Sense::click(),
                            );
                            ui.painter().rect_stroke(
                                rect,
                                6.0,
                                egui::Stroke::new(1.5_f32, DIM_TEXT),
                                egui::StrokeKind::Inside,
                            );
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "⬇",
                                egui::FontId::proportional(28.0),
                                ACTION_BLUE,
                            );
                            if resp.clicked() {
                                action = SkinPanelAction::ImportFromMinecraft;
                            }
                            ui.label(
                                egui::RichText::new("Bring in a\nMinecraft skin")
                                    .size(11.0)
                                    .color(DIM_TEXT),
                            );
                        });
                    });
                });
            });

            ui.add_space(8.0);

            // --- Actions on the selected entry ---
            if let Some(sel) = selected {
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Wear").size(14.0)).clicked() {
                        action = SkinPanelAction::Wear(sel);
                    }
                    if ui.button(egui::RichText::new("Edit ✎").size(14.0)).clicked() {
                        action = SkinPanelAction::Edit(sel);
                    }
                    if ui.button(egui::RichText::new("Duplicate ⧉").size(14.0)).clicked() {
                        action = SkinPanelAction::Duplicate(sel);
                    }
                    if ui.button(egui::RichText::new("Rename").size(14.0)).clicked() {
                        action = SkinPanelAction::BeginRename(sel);
                    }
                    if ui.button(egui::RichText::new("Export ⬇").size(14.0)).clicked() {
                        action = SkinPanelAction::ExportPng(sel);
                    }
                    if ui
                        .add_enabled(can_delete, egui::Button::new(
                            egui::RichText::new("Delete ✕").size(14.0).color(
                                egui::Color32::from_rgb(220, 120, 120),
                            ),
                        ))
                        .clicked()
                    {
                        action = SkinPanelAction::Delete(sel);
                    }
                });

                // Arms: Minecraft's two player models. The same 64×64 PNG works
                // on both — this only picks how wide the arms are drawn.
                let arms = cells
                    .iter()
                    .find(|c| c.id == sel)
                    .map(|c| c.arm_model)
                    .unwrap_or_default();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Arms").size(14.0));
                    if ui
                        .selectable_label(
                            arms == crate::skin_uv::ArmModel::Classic,
                            "Classic",
                        )
                        .clicked()
                    {
                        action = SkinPanelAction::SetArmModel(
                            sel,
                            crate::skin_uv::ArmModel::Classic,
                        );
                    }
                    if ui
                        .selectable_label(arms == crate::skin_uv::ArmModel::Slim, "Slim")
                        .clicked()
                    {
                        action =
                            SkinPanelAction::SetArmModel(sel, crate::skin_uv::ArmModel::Slim);
                    }
                });
                ui.label(
                    egui::RichText::new(match arms {
                        crate::skin_uv::ArmModel::Classic => {
                            "Chunky 4-pixel arms — Minecraft's \"Steve\" shape."
                        }
                        crate::skin_uv::ArmModel::Slim => {
                            "Slim 3-pixel arms — Minecraft's \"Alex\" shape."
                        }
                    })
                    .size(11.0)
                    .color(DIM_TEXT),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Same picture either way — when you export it, pick {} on minecraft.net.",
                        arms.label()
                    ))
                    .size(11.0)
                    .color(DIM_TEXT),
                );
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            // --- New skins ---
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("+ New (blank)").size(14.0).color(ACTION_BLUE)).clicked() {
                    action = SkinPanelAction::NewBlank;
                }
                if ui
                    .add_enabled(
                        !state.picking,
                        egui::Button::new(
                            egui::RichText::new("⬆ Upload PNG").size(14.0).color(ACTION_BLUE),
                        ),
                    )
                    .clicked()
                {
                    action = SkinPanelAction::Upload;
                }
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new("New from a colour:").size(12.0).color(DIM_TEXT));
            const PRESETS: [(u32, &str, egui::Color32); 5] = [
                (1, "Red", egui::Color32::from_rgb(220, 110, 110)),
                (2, "Green", egui::Color32::from_rgb(120, 200, 120)),
                (3, "Blue", egui::Color32::from_rgb(120, 150, 220)),
                (4, "Yellow", egui::Color32::from_rgb(220, 205, 110)),
                (5, "Pink", egui::Color32::from_rgb(220, 140, 220)),
            ];
            ui.horizontal_wrapped(|ui| {
                for (variant, label, colour) in PRESETS {
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new(label).size(13.0).color(colour))
                                .min_size(egui::vec2(56.0, 30.0))
                                .stroke(egui::Stroke::new(1.0_f32, colour)),
                        )
                        .clicked()
                    {
                        action = SkinPanelAction::NewFromColor(variant);
                    }
                }
            });

            ui.add_space(6.0);

            // --- Status + Back ---
            match &state.status {
                SkinStatus::Idle => {}
                SkinStatus::Picking => { ui.label("Choose a 64×64 PNG…"); }
                SkinStatus::Applied(stashed) => {
                    let msg = if *stashed {
                        "Saved ✓"
                    } else {
                        #[cfg(not(target_arch = "wasm32"))]
                        { "Saved on this computer ✓" }
                        #[cfg(target_arch = "wasm32")]
                        { "Saved on this device ✓" }
                    };
                    ui.colored_label(egui::Color32::from_rgb(120, 200, 120), msg);
                }

                SkinStatus::Error(m) => {
                    ui.colored_label(egui::Color32::from_rgb(220, 120, 120), m.clone());
                }
            }
            ui.add_space(8.0);
            if ui.button(egui::RichText::new("Back").size(15.0).color(DIM_TEXT)).clicked() {
                action = SkinPanelAction::Back;
            }
        });

    // --- Bring in a Minecraft skin: a second window over the wardrobe ---
    if state.mc_import_open {
        egui::Window::new("Bring in a Minecraft skin")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                if let Some(err) = state.mc_import_error {
                    // Error state: friendly copy + "Try again"/"OK".
                    ui.label(egui::RichText::new(err.user_message()).size(13.0));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            action = SkinPanelAction::CancelMcImport;
                        }
                        let label = if matches!(err, crate::mc_import::McImportError::RateLimited) {
                            "OK"
                        } else {
                            "Try again"
                        };
                        if ui
                            .button(egui::RichText::new(label).color(ACTION_BLUE))
                            .clicked()
                        {
                            action = SkinPanelAction::DismissMcError;
                        }
                    });
                } else if state.mc_import_busy {
                    ui.label(
                        egui::RichText::new(format!("⟳  Looking up {}…", state.mc_import_buf))
                            .size(14.0),
                    );
                    ui.add_space(8.0);
                    // Always escapable: if the worker dies/hangs, Cancel clears
                    // the busy state and closes the dialog (kid-UX — no dead end).
                    if ui.button("Cancel").clicked() {
                        action = SkinPanelAction::CancelMcImport;
                    }
                } else {
                    ui.label(
                        egui::RichText::new(
                            "🎮 Type a Java Edition username and we'll grab that player's skin for your wardrobe.",
                        )
                        .size(13.0),
                    );
                    ui.add_space(6.0);
                    let r = menu_text_field(
                        ui,
                        &mut state.mc_import_buf,
                        "Minecraft username",
                        "e.g. Notch",
                        240.0,
                        14.0,
                    );
                    let submit =
                        r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.label(
                        egui::RichText::new("Java Edition only (not Xbox / Bedrock names)")
                            .size(10.0)
                            .color(DIM_TEXT),
                    );
                    ui.add_space(4.0);
                    // T0-5 — just-in-time notice: say where the name goes
                    // before the player presses the button.
                    ui.label(
                        egui::RichText::new(crate::mc_import::LOOKUP_NOTICE)
                            .size(11.0)
                            .color(DIM_TEXT),
                    );
                    ui.add_space(8.0);
                    let has_text = !state.mc_import_buf.trim().is_empty();
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            action = SkinPanelAction::CancelMcImport;
                        }
                        let go = ui.add_enabled(
                            has_text,
                            egui::Button::new(
                                egui::RichText::new("Grab their skin!").color(ACTION_BLUE),
                            ),
                        );
                        if (go.clicked() || submit) && has_text {
                            action = SkinPanelAction::CommitMcImport(
                                state.mc_import_buf.trim().to_string(),
                            );
                        }
                    });
                }
            });
    }

    action
}

/// The "your skin is downloaded — here's how to wear it in real Minecraft"
/// panel shown after an export. Non-blocking; returns true when dismissed
/// (Got it! / clicking away). `unsaved` adds the "save first" tip.
pub fn draw_skin_export_help(
    ctx: &egui::Context,
    unsaved: bool,
    arm: crate::skin_uv::ArmModel,
) -> bool {
    let mut dismissed = false;
    egui::Window::new("export_help")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -24.0])
        .show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(egui::RichText::new("✅  Your skin is downloaded!").size(17.0).color(ACTION_BLUE));
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Now put it on in real Minecraft in 3 steps:").size(13.0));
            ui.add_space(4.0);
            ui.label("1.  Go to minecraft.net → Log in → Profile");
            ui.label("2.  Under \"Skin\" tap Browse and pick the file you just saved");
            ui.label(match arm {
                crate::skin_uv::ArmModel::Classic => {
                    "3.  For the arm shape choose Classic — that's what this skin is set to here"
                }
                crate::skin_uv::ArmModel::Slim => {
                    "3.  For the arm shape choose Slim / Alex — that's what this skin is set to here"
                }
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(
                "Using Minecraft on Xbox or mobile? Go to minecraft.net → Profile — same steps",
            ).size(11.0).color(DIM_TEXT));
            if unsaved {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(
                    "Tip: Save your skin first so you can find it again later.",
                ).size(11.0).color(DIM_TEXT));
            }
            ui.add_space(8.0);
            if ui.button(egui::RichText::new("Got it!").size(14.0).color(ACTION_BLUE)).clicked() {
                dismissed = true;
            }
        });
    // Escape also dismisses.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        dismissed = true;
    }
    dismissed
}

// ---------------------------------------------------------------------------
// Graphics settings panel (Spec 39)
// ---------------------------------------------------------------------------

/// What the game should do this frame in response to the settings panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsPanelAction {
    None,
    /// A dial or preset moved — the caller applies the live settings (FOV,
    /// render distance, sensitivity, present mode) and persists them.
    Changed,
    /// The texture-pack selection changed (already activated + persisted) — the
    /// caller rebuilds the block-texture atlas and remeshes loaded chunks (P3c/d).
    TexturePackChanged,
    Back,
}

/// What the lobby does with the settings panel's result. The lobby has no
/// loaded world, so only world-independent effects are applied; the rest
/// (FOV, render distance, mipmaps, texture-pack atlas, hosted-server radius)
/// is pushed on world entry by `sync_graphics_to_engine` /
/// `reapply_overrides`, from the already-persisted settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LobbySettingsPlan {
    /// Persist settings and re-gate the slash commands (tester unlock).
    pub persist: bool,
    /// Push the display-only dials (present mode, render scale) to the renderer.
    pub apply_display: bool,
    /// Drop a stale sign-in QR so it is rebuilt with the new relay list.
    pub reset_qr: bool,
    /// Close the panel.
    pub close: bool,
}

/// Decide the lobby's reaction to a settings-panel frame. `relays_changed` is
/// "the relay list differs from before the frame" (the relay manager saves
/// itself and does not raise `Changed`); `qr_on_screen` is "a sign-in QR or
/// its failure is showing".
pub fn plan_lobby_settings(
    action: SettingsPanelAction,
    relays_changed: bool,
    qr_on_screen: bool,
) -> LobbySettingsPlan {
    let changed = action == SettingsPanelAction::Changed;
    LobbySettingsPlan {
        persist: changed,
        apply_display: changed,
        reset_qr: relays_changed && qr_on_screen,
        close: action == SettingsPanelAction::Back,
    }
}

/// UI state for the Graphics settings panel. The actual values live in the
/// game's `GraphicsSettings`; this tracks whether the panel is open, plus the
/// "Your relays" manager's own UI state (its add-field draft and Check
/// results — the list itself is `gfx.online_relays`).
#[derive(Default)]
pub struct SettingsPanelState {
    pub open: bool,
    /// Set by the game loop each frame the panel is drawn: what the auto-quality
    /// governor is currently holding below the player's values (`None` when it
    /// is not holding anything). Read-only here.
    pub governor_note: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    pub relays: crate::relays_ui::RelaysUiState,
}

/// The plain version line shown at the foot of Settings, e.g. `AxeNStax v0.2.28`.
pub fn version_line() -> String {
    format!("AxeNStax v{}", env!("CARGO_PKG_VERSION"))
}

/// What a compact Settings Window spends on everything but its scroll area: its title bar,
/// frame, the pinned Back button and the margin to the screen edges.
const SETTINGS_COMPACT_CHROME: f32 = 150.0;

/// Draw the Settings panel (graphics, chat, sound, camera). Mutates `gfx` in place and returns
/// `Changed` when any dial/preset moved (caller applies live + persists), or
/// `Back` when the player closes it. Pure egui, mirrors `draw_skin_panel`.
pub fn draw_settings_panel(
    ctx: &egui::Context,
    state: &mut SettingsPanelState,
    gfx: &mut crate::graphics_settings::GraphicsSettings,
) -> SettingsPanelAction {
    use crate::graphics_settings::{
        FrameLimit, GraphicsPreset, ParticleLevel, AUTO_CENTRE_DELAY_MAX, AUTO_CENTRE_DELAY_MIN,
        AUTO_CENTRE_SPEED_MAX, AUTO_CENTRE_SPEED_MIN, FOV_MAX, FOV_MIN, FREELOOK_SENS_MAX,
        FREELOOK_SENS_MIN, RENDER_DISTANCE_MAX, RENDER_DISTANCE_MIN, RENDER_SCALE_MAX,
        RENDER_SCALE_MIN, SENSITIVITY_MAX, SENSITIVITY_MIN, TP_DISTANCE_MAX, TP_DISTANCE_MIN,
        ZOOM_FOV_MAX, ZOOM_FOV_MIN, BRIGHTNESS_MAX, BRIGHTNESS_MIN, MINIMAP_ZOOM_MAX,
        MINIMAP_ZOOM_MIN, MASTER_VOLUME_MAX, MASTER_VOLUME_MIN, CHAT_TEXT_SCALE_MAX,
        CHAT_TEXT_SCALE_MIN, CHAT_TEXT_SCALE_STEP,
    };
    // The relay manager (its only use of `state`) is native-only.
    #[cfg(target_arch = "wasm32")]
    let _ = &state;
    let mut action = SettingsPanelAction::None;
    let mut changed = false;
    let mut pack_changed = false;

    // On a compact (phone) screen the dials sit in a scroll area, capped so the Window with
    // its title and the pinned Back button fits the screen; Back stays outside the scroll
    // area, so it is always reachable. A tall window keeps the panel as it was.
    let compact = dialog_is_compact(ctx);
    // Inside the TV-safe area (home page overhaul §2), so the pinned Back is on screen.
    let settings_scroll_h = (crate::ui_safe_area::rect(ctx).height() - SETTINGS_COMPACT_CHROME).max(120.0);

    egui::Window::new("Settings")
        .max_width(crate::ui_safe_area::rect(ctx).width() - 24.0)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(340.0);

            let mut settings_body = |ui: &mut egui::Ui| {
            // Preset row — highlight the derived current preset.
            let current = gfx.preset();
            ui.label(
                egui::RichText::new(format!("Preset: {}", current.label()))
                    .size(15.0)
                    .color(TITLE_COLOR),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for p in GraphicsPreset::ALL {
                    let selected = current == p;
                    let stroke = if selected {
                        egui::Stroke::new(2.0_f32, CARD_SELECTED_BORDER)
                    } else {
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 50, 60))
                    };
                    let btn = egui::Button::new(
                        egui::RichText::new(p.label())
                            .size(13.0)
                            .color(if selected { ACTION_BLUE } else { TEXT_COLOR }),
                    )
                    .min_size(egui::vec2(58.0, 32.0))
                    .fill(PANEL_BG)
                    .corner_radius(egui::CornerRadius::same(4))
                    .stroke(stroke);
                    // Clicking the preset already shown still applies it: the
                    // pixel budget and Leaves: Fast are set by a pick but are
                    // not part of the label, so an old Low file picks them up here.
                    if ui.add(btn).clicked() {
                        gfx.apply_preset(p);
                        changed = true;
                    }
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            // --- Texture pack picker (P3c) — native; web pack-swap is P4. ---
            #[cfg(not(target_arch = "wasm32"))]
            {
                let root = crate::texture_registry::texturepacks_root();
                let mut options = vec!["Default".to_string()];
                options.extend(crate::texture_registry::discover_packs(&root));
                let mut sel = crate::texture_registry::active_pack_name();
                let before = sel.clone();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Texture pack").size(13.0).color(TEXT_COLOR));
                    egui::ComboBox::from_id_salt("texture_pack_combo")
                        .selected_text(sel.clone())
                        .show_ui(ui, |ui| {
                            for opt in &options {
                                ui.selectable_value(&mut sel, opt.clone(), opt);
                            }
                        });
                });
                if sel != before {
                    crate::texture_registry::select_pack(&sel);
                    pack_changed = true;
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }

            // --- Texture pack picker (P4d) — web: packs are fetched from the game
            // site (`/static/packs/`). Selecting one fetches + decodes + applies it
            // asynchronously, so we DON'T set `pack_changed` here (no atlas to
            // rebuild yet); the game loop rebuilds when the fetch completes. ---
            #[cfg(target_arch = "wasm32")]
            {
                crate::texture_packs_web::ensure_list_fetched();
                let mut options = vec!["Default".to_string()];
                options.extend(crate::texture_packs_web::pack_names());
                let mut sel = crate::texture_packs_web::saved_selection();
                let before = sel.clone();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Texture pack").size(13.0).color(TEXT_COLOR));
                    egui::ComboBox::from_id_salt("texture_pack_combo_web")
                        .selected_text(sel.clone())
                        .show_ui(ui, |ui| {
                            for opt in &options {
                                ui.selectable_value(&mut sel, opt.clone(), opt);
                            }
                        });
                });
                if sel != before {
                    crate::texture_packs_web::select(&sel);
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }

            // Individual dials. Touching any flips the preset to Custom.
            if ui
                .add(
                    egui::Slider::new(
                        &mut gfx.render_distance,
                        RENDER_DISTANCE_MIN..=RENDER_DISTANCE_MAX,
                    )
                    .text("Render distance"),
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .add(
                    egui::Slider::new(&mut gfx.render_scale, RENDER_SCALE_MIN..=RENDER_SCALE_MAX)
                        .fixed_decimals(2)
                        .text("Render scale"),
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .add(
                    egui::Slider::new(&mut gfx.fov_y, FOV_MIN..=FOV_MAX)
                        .fixed_decimals(0)
                        .text("Field of view"),
                )
                .changed()
            {
                changed = true;
            }
            // #44 — hold-to-zoom (C) field of view. Its own narrow range, below
            // the personal-FOV floor.
            if ui
                .add(
                    egui::Slider::new(&mut gfx.zoom_fov, ZOOM_FOV_MIN..=ZOOM_FOV_MAX)
                        .fixed_decimals(0)
                        .text("Zoom field of view (hold C)"),
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .add(
                    egui::Slider::new(
                        &mut gfx.mouse_sensitivity,
                        SENSITIVITY_MIN..=SENSITIVITY_MAX,
                    )
                    .fixed_decimals(4)
                    .text("Mouse sensitivity"),
                )
                .changed()
            {
                changed = true;
            }
            // Brightness — player-side answer to "it is too dark". Neutral at 1.0;
            // higher lifts shadows (gamma), lower darkens. Personal pref (doesn't
            // flip the preset). Applied live via the camera uniform.
            if ui
                .add(
                    egui::Slider::new(&mut gfx.brightness, BRIGHTNESS_MIN..=BRIGHTNESS_MAX)
                        .fixed_decimals(2)
                        .text("Brightness"),
                )
                .changed()
            {
                changed = true;
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Frame limit").size(13.0).color(TEXT_COLOR));
                for (fl, label) in [
                    (FrameLimit::VSync, "VSync"),
                    (FrameLimit::Cap(30), "30"),
                    (FrameLimit::Cap(60), "60"),
                    (FrameLimit::Cap(120), "120"),
                    (FrameLimit::Uncapped, "Max"),
                ] {
                    let selected = gfx.frame_limit == fl;
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        gfx.frame_limit = fl;
                        changed = true;
                    }
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Particles").size(13.0).color(TEXT_COLOR));
                for (pl, label) in [
                    (ParticleLevel::Off, "Off"),
                    (ParticleLevel::Reduced, "Reduced"),
                    (ParticleLevel::Full, "Full"),
                ] {
                    let selected = gfx.particles == pl;
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        gfx.particles = pl;
                        changed = true;
                    }
                }
            });

            ui.add_space(4.0);
            if ui.checkbox(&mut gfx.fog, "Fog").changed() {
                changed = true;
            }
            // Spec 39 A6 — opt-in. Nearest magnification is kept (pixels stay
            // crisp up close); only minified/distant faces take a smoothed mip.
            if ui
                .checkbox(&mut gfx.mipmaps, "Mipmaps")
                .on_hover_text(
                    "Smooths distant textures so far-off ground and walls stop \
                     shimmering. Blocks you are standing next to stay pixel-sharp. \
                     Off by default — turn it on and look at the horizon.",
                )
                .changed()
            {
                changed = true;
            }
            // Perf B2 — "Leaves: Fast": solid-looking leaves that skip the see-through
            // pass and every hidden inner face. A preset dial (Potato and Low on).
            if ui
                .checkbox(&mut gfx.leaves_fast, "Leaves: Fast")
                .on_hover_text(
                    "Draws leaves as solid blocks with no see-through gaps. \
                     Treetops look flatter but the game runs faster, most on \
                     slower machines. The Low and Potato presets turn it on.",
                )
                .changed()
            {
                changed = true;
            }
            // Perf B1 — auto quality (Spec 03 §10.3): lowers distance, then scale,
            // then particles while frames run long, and restores them when they
            // recover. Never changes or saves the values above.
            if ui
                .checkbox(&mut gfx.auto_quality, "Auto quality")
                .on_hover_text(
                    "If the game runs slowly, temporarily lowers render distance, \
                     then render scale, then particles - and puts them back when \
                     it speeds up. Your own settings are never changed.",
                )
                .changed()
            {
                changed = true;
            }
            if let Some(note) = state.governor_note.as_deref() {
                ui.label(egui::RichText::new(note).size(11.0).color(SUBTITLE_COLOR));
            }
            // #44 — make the F3 debug overlay discoverable for players who don't
            // know the key. This is the on-load default; F3 still toggles live.
            if ui
                .checkbox(&mut gfx.show_debug_hud, "Show debug overlay (F3)")
                .changed()
            {
                changed = true;
            }
            // Living world S5 (D16) — the Tally and world bloom, for grown-ups,
            // drawn with the F3 overlay. Off unless someone turns it on.
            if ui
                .checkbox(&mut gfx.grownup_panel, "Grown-up panel (F3)")
                .changed()
            {
                changed = true;
            }
            // #45 — auto-refill an emptied hotbar block from the bag.
            if ui
                .checkbox(&mut gfx.auto_refill, "Auto-refill hotbar from inventory")
                .changed()
            {
                changed = true;
            }
            // #6 — minimap: corner map + zoom. On by default; the zoom slider is
            // world blocks per pixel (lower = more zoomed in).
            if ui
                .checkbox(&mut gfx.minimap_enabled, "Show minimap (top-right)")
                .changed()
            {
                changed = true;
            }
            if gfx.minimap_enabled
                && ui
                    .add(
                        egui::Slider::new(&mut gfx.minimap_zoom, MINIMAP_ZOOM_MIN..=MINIMAP_ZOOM_MAX)
                            .fixed_decimals(1)
                            .text("Minimap zoom (blocks/pixel)"),
                    )
                    .changed()
            {
                changed = true;
            }
            // #24 — accessibility narration (Web Speech TTS; web only for now).
            if ui
                .checkbox(&mut gfx.narration_enabled, "Narrate hotbar (accessibility)")
                .changed()
            {
                changed = true;
            }

            // Chat — text size. A multiplier on the chat overlay's own 13 pt
            // monospace, shown as a percentage (100% to 300% in quarter steps).
            // Applies live (the next chat frame reads it) and persists like
            // Brightness. The preview line is drawn at the chosen size.
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Chat")
                    .size(14.0)
                    .color(TITLE_COLOR),
            );
            ui.add_space(4.0);
            let mut chat_pct = gfx.chat_text_scale * 100.0;
            if ui
                .add(
                    egui::Slider::new(
                        &mut chat_pct,
                        (CHAT_TEXT_SCALE_MIN * 100.0)..=(CHAT_TEXT_SCALE_MAX * 100.0),
                    )
                    .step_by(f64::from(CHAT_TEXT_SCALE_STEP * 100.0))
                    .fixed_decimals(0)
                    .suffix("%")
                    .text("Chat text size"),
                )
                .changed()
            {
                gfx.chat_text_scale = (chat_pct / 100.0).clamp(CHAT_TEXT_SCALE_MIN, CHAT_TEXT_SCALE_MAX);
                changed = true;
            }
            ui.label(
                egui::RichText::new("/give torch 4")
                    .font(egui::FontId::monospace(13.0 * gfx.chat_text_scale))
                    .color(egui::Color32::from_rgb(196, 196, 212)),
            );

            // Sound — master volume + mute. Persisted per-device on both targets
            // and applied live (`AudioEngine::set_master`). The slider is shown
            // as a percentage; the audio engine squares it for a usable curve.
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Sound")
                    .size(14.0)
                    .color(TITLE_COLOR),
            );
            ui.add_space(4.0);
            let mut volume_pct = gfx.master_volume * 100.0;
            if ui
                .add(
                    egui::Slider::new(&mut volume_pct, 0.0..=100.0)
                        .fixed_decimals(0)
                        .suffix("%")
                        .text("Volume"),
                )
                .changed()
            {
                gfx.master_volume = (volume_pct / 100.0)
                    .clamp(MASTER_VOLUME_MIN, MASTER_VOLUME_MAX);
                changed = true;
            }
            if ui.checkbox(&mut gfx.audio_muted, "Mute all sound").changed() {
                changed = true;
            }

            // Phase 6 — third-person camera section. Defaults preserve the
            // approved feel; these are the live dials the Axolittle playtest tunes.
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Third-person camera")
                    .size(14.0)
                    .color(TITLE_COLOR),
            );
            ui.add_space(4.0);
            if ui
                .add(
                    egui::Slider::new(&mut gfx.third_person_distance, TP_DISTANCE_MIN..=TP_DISTANCE_MAX)
                        .fixed_decimals(2)
                        .text("Camera distance"),
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(&mut gfx.avatar_fade, "Fade my avatar when the camera is close")
                .changed()
            {
                changed = true;
            }
            if ui
                .checkbox(&mut gfx.third_person_freelook, "Free-look (hold Alt to look around)")
                .changed()
            {
                changed = true;
            }
            if gfx.third_person_freelook {
                if ui
                    .add(
                        egui::Slider::new(
                            &mut gfx.freelook_sensitivity,
                            FREELOOK_SENS_MIN..=FREELOOK_SENS_MAX,
                        )
                        .fixed_decimals(2)
                        .text("Free-look sensitivity"),
                    )
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .add(
                        egui::Slider::new(
                            &mut gfx.auto_centre_delay,
                            AUTO_CENTRE_DELAY_MIN..=AUTO_CENTRE_DELAY_MAX,
                        )
                        .fixed_decimals(2)
                        .text("Auto-centre delay"),
                    )
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .add(
                        egui::Slider::new(
                            &mut gfx.auto_centre_speed,
                            AUTO_CENTRE_SPEED_MIN..=AUTO_CENTRE_SPEED_MAX,
                        )
                        .fixed_decimals(1)
                        .text("Auto-centre speed"),
                    )
                    .changed()
                {
                    changed = true;
                }
            }

            // Online play by contact (spec §6). Native only — there is no online
            // play on the web taster.
            #[cfg(not(target_arch = "wasm32"))]
            {
                ui.add_space(12.0);
                ui.label(egui::RichText::new("Online").size(15.0).strong());
                ui.label(
                    egui::RichText::new(
                        "The port your computer listens on when you host a world for a friend.",
                    )
                    .size(11.0)
                    .color(DIM_TEXT),
                );
                ui.add_space(4.0);
                let mut port = gfx.online_port.to_string();
                ui.horizontal(|ui| {
                    ui.label("Port");
                    if ui
                        .add(egui::TextEdit::singleline(&mut port).desired_width(70.0))
                        .changed()
                        && let Ok(p) = port.parse::<u16>()
                    {
                        gfx.online_port = p;
                        gfx.save();
                    }
                    ui.label(
                        egui::RichText::new("0 = pick one automatically")
                            .size(10.0)
                            .color(DIM_TEXT),
                    );
                });
            }

            // "Your relays" — its own section (Spec 04 §1.9), not under Online:
            // the list serves sign-in, contacts and updates as well as online
            // play. Native only.
            #[cfg(not(target_arch = "wasm32"))]
            {
                ui.add_space(12.0);
                crate::relays_ui::draw(ui, &mut state.relays, gfx);
            }

            // Version line.
            ui.add_space(12.0);
            ui.label(egui::RichText::new(version_line()).size(11.0).color(DIM_TEXT));
            };
            if compact {
                // `min_scrolled_height`, as in `draw_dialog_frame`: a Window's UI may only use
                // the size it had last frame, so without it the panel never grows past its
                // first, small size.
                egui::ScrollArea::vertical()
                    .max_height(settings_scroll_h)
                    .min_scrolled_height(settings_scroll_h)
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| settings_body(ui));
            } else {
                settings_body(ui);
            }

            ui.add_space(10.0);
            let back = egui::Button::new(egui::RichText::new("Back").size(15.0).color(DIM_TEXT))
                .min_size(egui::vec2(320.0, 36.0))
                .fill(PANEL_BG)
                .corner_radius(egui::CornerRadius::same(4))
                .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(50, 50, 60)));
            if ui.add(back).clicked() {
                action = SettingsPanelAction::Back;
            }
        });

    if changed {
        gfx.clamp();
    }
    // Back (set inside the closure) always wins; otherwise a texture-pack change
    // takes precedence over a graphics-dial change (it needs the atlas rebuild).
    if action == SettingsPanelAction::None {
        if pack_changed {
            action = SettingsPanelAction::TexturePackChanged;
        } else if changed {
            action = SettingsPanelAction::Changed;
        }
    }
    action
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{
        app_feature_copy, plan_lobby_settings, version_line, AppFeature, LobbySettingsPlan,
        SettingsPanelAction,
    };

    #[test]
    fn the_version_line_carries_the_crate_version() {
        let v = version_line();
        assert!(v.starts_with("AxeNStax v"));
        assert!(v.ends_with(env!("CARGO_PKG_VERSION")));
    }

    /// CP15 (CP14 review MEDIUM-1): the lobby's Fork on a refused (retired)
    /// world opens no fork dialog and shows the refusal line.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn handover_lobby_fork_of_a_refused_world_shows_why_and_forks_nothing() {
        let _root = crate::save::WorldsRootGuard::new("lobby-fork-refused");
        let name = "refused";
        crate::save::write_world_folder(name, &crate::save::WorldMeta::new(name), &crate::save::minimal_world_save_for_tests(5), &crate::world::World::new()).unwrap();
        let store = crate::handover::live::authority::store(&crate::save::world_dir(name)).unwrap();
        store.persist(crate::handover::durable::MarkerKind::Retired, [9; 16], b"a retirement record").unwrap();
        let mut state = super::MenuState::new();
        state.worlds = crate::save::list_world_entries();
        let idx = state.worlds.iter().position(|w| w.folder_name == name).expect("the world isn't listed");
        let action = super::handle_card_action(&mut state, idx, super::CardAction::Fork);
        assert!(matches!(action, super::MenuAction::None));
        assert!(!matches!(state.dialog, super::MenuDialog::Forking { .. }), "the lobby opened a fork of a refused world");
        assert!(state.notice.is_some(), "the lobby didn't say why the world can't be forked");
    }

    /// Terrain T1 review S1: a world from a newer build (its meta parses but names
    /// a terrain generator this build lacks) is never rewritten or forked by the
    /// lobby. Rename, Stash and Fork all leave its `world_meta.json` byte for
    /// byte as it was and create nothing; the card says "needs a newer version".
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn lobby_edit_stash_and_fork_leave_a_newer_generators_world_untouched() {
        use crate::save;
        let _root = save::WorldsRootGuard::new("lobby-newer-generator");
        let name = "from-the-future";
        let mut meta = save::WorldMeta::new(name);
        // One past the newest generator this build has, so it stays unknown as
        // generators are added.
        meta.worldgen_version = crate::biome::Generator::ALL.iter().map(|g| g.version()).max().unwrap() + 1;
        meta.seed = 777;
        save::write_world_folder(name, &meta, &save::minimal_world_save_for_tests(5), &crate::world::World::new())
            .unwrap();
        let meta_path = save::world_dir(name).join("world_meta.json");
        let before = std::fs::read(&meta_path).expect("the meta was written");
        let folders = |root: &std::path::Path| std::fs::read_dir(root).unwrap().count();
        let root_dir = save::world_dir(name).parent().unwrap().to_path_buf();
        assert_eq!(folders(&root_dir), 1);

        let mut state = super::MenuState::new();
        state.worlds = save::list_world_entries();
        let idx = state.worlds.iter().position(|w| w.folder_name == name).expect("the world isn't listed");
        let card = &state.worlds[idx].meta;
        assert!(card.display_name.contains("needs a newer version"), "{}", card.display_name);
        assert!(!card.display_name.contains("damaged"), "{}", card.display_name);
        assert_eq!(card.seed, 777, "the card shows the world's real info, not placeholders");

        // Rename (Edit).
        let action = super::handle_card_action(&mut state, idx, super::CardAction::Edit);
        assert!(matches!(action, super::MenuAction::None));
        assert!(!matches!(state.dialog, super::MenuDialog::Edit { .. }), "the lobby opened a rename");
        assert!(state.notice.take().is_some(), "no reason shown for Edit");
        // ...and the write the Edit dialog would make is refused by the saver itself.
        assert!(save::save_world_meta(name, &state.worlds[idx].meta).is_err());

        // Stash toggle.
        let stash_before = state.worlds[idx].meta.cloud_save;
        super::handle_card_action(&mut state, idx, super::CardAction::ToggleStash);
        assert_eq!(state.worlds[idx].meta.cloud_save, stash_before, "the card's Stash flag flipped");
        assert!(state.notice.take().is_some(), "no reason shown for Stash");

        // Fork.
        super::handle_card_action(&mut state, idx, super::CardAction::Fork);
        assert!(!matches!(state.dialog, super::MenuDialog::Forking { .. }), "the lobby opened a fork");
        assert!(state.notice.take().is_some(), "no reason shown for Fork");
        assert!(save::fork_world(name, "From the future").is_err(), "fork_world copied it");

        assert_eq!(std::fs::read(&meta_path).unwrap(), before, "world_meta.json changed");
        assert_eq!(folders(&root_dir), 1, "a fork was created");
    }

    #[test]
    fn lobby_settings_changed_persists_and_applies_display_only() {
        let p = plan_lobby_settings(SettingsPanelAction::Changed, false, false);
        assert_eq!(
            p,
            LobbySettingsPlan { persist: true, apply_display: true, reset_qr: false, close: false }
        );
    }

    #[test]
    fn lobby_settings_back_closes_without_applying() {
        let p = plan_lobby_settings(SettingsPanelAction::Back, false, false);
        assert!(p.close && !p.persist && !p.apply_display);
    }

    #[test]
    fn lobby_settings_idle_and_texture_pack_do_nothing_in_lobby() {
        // A texture-pack pick is already persisted by the picker; the atlas is
        // rebuilt on world entry, so the lobby has nothing to apply.
        for a in [SettingsPanelAction::None, SettingsPanelAction::TexturePackChanged] {
            let p = plan_lobby_settings(a, false, false);
            assert_eq!(
                p,
                LobbySettingsPlan { persist: false, apply_display: false, reset_qr: false, close: false }
            );
        }
    }

    #[test]
    fn lobby_relay_change_drops_qr_only_when_one_is_showing() {
        // The relay manager does not raise `Changed`, so relays alone must still
        // refresh the QR (and need no settings persist — the manager saved).
        let p = plan_lobby_settings(SettingsPanelAction::None, true, true);
        assert!(p.reset_qr && !p.persist);
        assert!(!plan_lobby_settings(SettingsPanelAction::None, true, false).reset_qr);
        assert!(!plan_lobby_settings(SettingsPanelAction::None, false, true).reset_qr);
    }

    /// Legal positioning guard: the desktop-app explainer copy must stay
    /// CAPABILITY-framed — describing what the software does — and must never
    /// drift into "bypass the rules" / earning-to-minors / unrestricted stranger
    /// contact. It must also actually point at the desktop app (the funnel).
    /// See docs/superpowers/specs/2026-06-27-web-local-sandbox-design.md.
    #[test]
    fn app_feature_copy_is_capability_framed() {
        const BANNED: &[&str] = &[
            "bypass", "circumvent", "evade", "unrestricted", "no limit",
            "no restriction", "no sign-in needed",
            "stranger", "no rules", "around the",
        ];
        for f in [AppFeature::Stash, AppFeature::Join, AppFeature::Host] {
            let c = app_feature_copy(f);
            let mut text = format!("{} {} {}", c.title, c.lead, c.cta_note).to_lowercase();
            for p in c.points {
                text.push(' ');
                text.push_str(&p.to_lowercase());
            }
            for banned in BANNED {
                assert!(
                    !text.contains(banned),
                    "feature copy for {f:?} must not contain '{banned}': {text}"
                );
            }
            // Money words: the shared `copy_lint::BANNED_MONEY_WORDS`.
            let money = crate::copy_lint::banned_in(&text, crate::copy_lint::BANNED_MONEY_WORDS);
            assert!(money.is_empty(), "feature copy for {f:?} must not say {money:?}: {text}");
            assert!(
                text.contains("desktop app"),
                "feature copy for {f:?} must point at the desktop app: {text}"
            );
        }
    }

    /// RAII guard for the native world-transfer tests: removes a temp
    /// `worlds/<name>` folder on drop, so it's cleaned up even if an assertion
    /// panics mid-test (a bare `remove_dir_all` at the end of the test would be
    /// skipped on failure, leaking the folder into later runs).
    #[cfg(not(target_arch = "wasm32"))]
    struct Cleanup(std::path::PathBuf);
    #[cfg(not(target_arch = "wasm32"))]
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Cleanup-only guard for the world-transfer tests: on drop, removes every
    /// `worlds/<name>` folder whose name begins with `prefix`. Used to sweep up
    /// deduped import folders whose exact names a test doesn't capture. The
    /// prefix is always a unique per-test nonce stem, so this never touches
    /// another (parallel) test's worlds. Cleanup-only — never asserted on — so a
    /// benign race that misses a folder merely leaves it for the next sweep.
    #[cfg(not(target_arch = "wasm32"))]
    struct PrefixCleanup(String);
    #[cfg(not(target_arch = "wasm32"))]
    impl Drop for PrefixCleanup {
        fn drop(&mut self) {
            for entry in crate::save::list_world_entries() {
                if entry.folder_name.starts_with(&self.0) {
                    let _ = std::fs::remove_dir_all(crate::save::world_dir(&entry.folder_name));
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn list_axeworld_files_empty_on_missing_dir() {
        // A path that is guaranteed not to exist.
        let result = super::list_axeworld_files("__nonexistent_transfer_dir_z9q8w7__");
        assert!(result.is_empty(), "missing dir must yield empty list, got: {result:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn list_axeworld_files_finds_axeworld_only() {
        // Build a temporary directory with mixed files.
        let tmp = std::env::temp_dir().join("__menu_axeworld_test_5f3a2b__");
        let _ = std::fs::remove_dir_all(&tmp); // clean up any prior run
        std::fs::create_dir_all(&tmp).expect("tmp dir");
        std::fs::write(tmp.join("world1.axeworld"), b"x").unwrap();
        std::fs::write(tmp.join("world2.axeworld"), b"y").unwrap();
        std::fs::write(tmp.join("readme.txt"), b"ignore me").unwrap();
        std::fs::write(tmp.join("other.zip"), b"ignore too").unwrap();

        let result = super::list_axeworld_files(tmp.to_str().unwrap());

        assert_eq!(result.len(), 2, "must find exactly 2 .axeworld files, got: {result:?}");
        assert!(result.iter().all(|p| p.ends_with(".axeworld")), "all results must end with .axeworld");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── route_dialog_result tests ────────────────────────────────────────────
    // These inject results directly through an mpsc channel, bypassing the real
    // OS dialog (which can't open headlessly). They test the routing logic that
    // lives entirely in Rust and has no GUI dependencies.

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn route_dialog_result_cancelled_sets_status_and_returns_done() {
        use crate::native_file_dialog::FileDialogResult;
        let mut status: Option<String> = None;
        let done = super::route_dialog_result(FileDialogResult::Cancelled, &mut status);
        assert!(done, "Cancelled must signal done");
        assert_eq!(status.as_deref(), Some("Cancelled"),
                   "Cancelled must set status to 'Cancelled'");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn route_dialog_result_world_saved_contains_path() {
        use crate::native_file_dialog::FileDialogResult;
        let mut status: Option<String> = None;
        let path = std::path::PathBuf::from("/tmp/foo.axeworld");
        let done = super::route_dialog_result(FileDialogResult::WorldSaved(path), &mut status);
        assert!(done, "WorldSaved must signal done");
        let s = status.unwrap();
        assert!(s.contains("Saved to"), "status must mention 'Saved to', got: {s:?}");
        assert!(s.contains("/tmp/foo.axeworld"), "status must contain the path, got: {s:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn route_dialog_result_err_mentions_failed_and_message() {
        use crate::native_file_dialog::FileDialogResult;
        let mut status: Option<String> = None;
        let done = super::route_dialog_result(
            FileDialogResult::Err("disk full".to_string()),
            &mut status,
        );
        assert!(done, "Err must signal done");
        let s = status.unwrap();
        assert!(s.contains("failed"), "status must contain 'failed', got: {s:?}");
        assert!(s.contains("disk full"), "status must contain the error message, got: {s:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn poll_dialog_via_channel_cancelled() {
        use crate::native_file_dialog::FileDialogResult;
        // Build the channel directly — no OS dialog needed.
        let (tx, rx) = std::sync::mpsc::channel::<FileDialogResult>();
        // We can't call MenuState::new() in tests (heavy deps), so test the
        // underlying helper instead.
        tx.send(FileDialogResult::Cancelled).unwrap();
        drop(tx);
        let mut status: Option<String> = None;
        // Drain manually (mirrors what poll_dialog does).
        while let Ok(result) = rx.try_recv() {
            super::route_dialog_result(result, &mut status);
        }
        assert_eq!(status.as_deref(), Some("Cancelled"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn route_dialog_result_no_op_for_world_to_import() {
        use crate::native_file_dialog::FileDialogResult;
        // WorldToImport is handled upstream in poll_dialog (which has &mut self
        // and can refresh the world list). route_dialog_result must remain a
        // no-op for this variant — it should not crash or touch transfer_status.
        let mut status: Option<String> = None;
        let done = super::route_dialog_result(
            FileDialogResult::WorldToImport(b"some bytes".to_vec()),
            &mut status,
        );
        assert!(done, "WorldToImport must signal done (channel cleared)");
        assert!(status.is_none(), "route_dialog_result must leave status untouched for WorldToImport (handled in poll_dialog)");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn route_dialog_result_no_op_for_skin_png() {
        use crate::native_file_dialog::FileDialogResult;
        // Phase 4 placeholder — routing SkinPng must NOT crash or set status.
        let mut status: Option<String> = None;
        let done = super::route_dialog_result(
            FileDialogResult::SkinPng(b"\x89PNG".to_vec()),
            &mut status,
        );
        assert!(done, "SkinPng must signal done (channel cleared)");
        assert!(status.is_none(), "Phase 4 placeholder must leave status untouched");
    }

    // ── export_dialog_request tests (Phase 2) ────────────────────────────────

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn export_dialog_request_returns_save_world_with_correct_name_and_bytes() {
        use crate::native_file_dialog::FileDialogRequest;
        use crate::save::{world_dir, WorldMeta, write_world_folder};
        use crate::world::World;
        use crate::block;

        // Fixed nonce to avoid collisions with parallel test runs.
        let world_name = "__menu_export_req_p2_7e2a9b";

        // Build a minimal world and write it to disk.
        let mut world = World::new();
        world.set_block(0, 0, 0, block::STONE);
        world.set_block(1, 0, 0, block::DIRT);
        let meta = WorldMeta::new(world_name);
        let save = crate::save::minimal_world_save_for_tests(99);
        write_world_folder(world_name, &meta, &save, &world)
            .expect("write_world_folder failed in test setup");
        // Removed on drop even if an assertion below panics.
        let _guard = Cleanup(world_dir(world_name));

        // Call the helper under test.
        let req = super::export_dialog_request(world_name)
            .expect("export_dialog_request must succeed for a valid world");

        // Assert: the request is a SaveWorld with the right default_name.
        let (default_name, bytes) = match req {
            FileDialogRequest::SaveWorld { default_name, bytes } => (default_name, bytes),
            _ => panic!("expected SaveWorld variant"),
        };
        assert_eq!(
            default_name,
            format!("{world_name}.axeworld"),
            "default_name must be '<name>.axeworld'"
        );

        // Assert: the bytes match what export_world_native produces directly.
        let expected = crate::native_world_io::export_world_native(world_name)
            .expect("export_world_native must succeed");
        assert_eq!(bytes, expected, "bytes must equal the packed world output");
        assert!(!bytes.is_empty(), "packed bytes must be non-empty");
        // Temp folder removed by `_guard` on drop.
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn export_dialog_request_fails_for_nonexistent_world() {
        let result = super::export_dialog_request("__menu_export_req_nonexistent_world_p2");
        assert!(result.is_err(), "must return Err for a world that does not exist");
    }

    // ── import_picked_world tests (Phase 3) ──────────────────────────────────

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn stale_lobby_meta_keeps_the_disk_pop_secret() {
        let _root = crate::save::WorldsRootGuard::new("n4-secret");
        let meta = crate::save::WorldMeta::new("n4");
        let disk = meta.pop_secret.expect("new world has a secret");
        crate::save::save_world_meta("n4", &meta).expect("write meta");
        let mut stale = meta.clone();
        stale.pop_secret = None; // the pre-play lobby entry of a legacy world
        super::keep_disk_pop_secret("n4", &mut stale);
        assert_eq!(stale.pop_secret, Some(disk));
    }

    /// Happy-path: build a world on disk, export it to bytes, then call the
    /// lobby helper `import_picked_world`. Asserts success via the helper's
    /// BOOLEAN (never its status string, so this survives any copy rewording —
    /// Phase 5d) and verifies the deduped folder landed on disk. The folder is
    /// located by its slug (derived from the unique per-test source name), so
    /// the check is robust under parallel test runs — it never trips over world
    /// folders created by other tests sharing the same `worlds/` directory.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn import_picked_world_valid_bytes_succeeds() {
        use crate::save::{world_dir, WorldMeta, write_world_folder};
        use crate::world::World;
        use crate::block;

        // Isolate the worlds dir to a private per-thread temp dir: keeps this
        // test's source + imported folders out of the shared `worlds/` (whose
        // pollution is what made the sibling garbage-import count test flaky)
        // and auto-cleans on drop. Declared first so it drops LAST (after the
        // per-folder cleanups below).
        let _root = crate::save::WorldsRootGuard::new("import-valid");

        // Fixed nonce to avoid collisions with parallel test runs. The imported
        // folder's slug is derived from this name by `sanitize_folder_name`
        // (lowercase + trim leading/trailing `_`), so deduped import folders
        // share this stem — used only for cleanup, never for assertions.
        let src_name = "__menu_import_p3_valid_c9d2e1";
        let _import_sweep = PrefixCleanup("menu_import_p3_valid_c9d2e1".to_string());

        // Build a minimal world and write it to disk.
        let mut world = World::new();
        world.set_block(0, 0, 0, block::STONE);
        world.set_block(2, 0, 0, block::DIRT);
        let meta = WorldMeta::new(src_name);
        let save = crate::save::minimal_world_save_for_tests(42);
        write_world_folder(src_name, &meta, &save, &world)
            .expect("write_world_folder (source) failed in test setup");
        let _src_guard = Cleanup(world_dir(src_name));

        // Export the source world to bytes.
        let bytes = crate::native_world_io::export_world_native(src_name)
            .expect("export_world_native failed in test setup");

        // PART 1 — the lobby helper under test reports success via its BOOLEAN,
        // never its status string (so this survives status-copy rewording).
        let (_status, imported_ok) = super::import_picked_world(&bytes);
        assert!(
            imported_ok,
            "import_picked_world must return imported_ok=true for valid bytes"
        );

        // PART 2 — drive `import_world_native` directly so we get the deduped
        // folder NAME back as a return value (race-free, no status parsing, no
        // global folder diff). This is the robust disk-state assertion.
        let imported_folder = crate::native_world_io::import_world_native(&bytes)
            .expect("import_world_native must succeed for valid bytes");

        assert_ne!(
            imported_folder, src_name,
            "imported world must land in a different (deduped) folder, not overwrite the source"
        );
        assert!(
            world_dir(&imported_folder).exists(),
            "imported world folder must exist on disk: {imported_folder}"
        );
    }

    /// Unhappy-path: garbage bytes must not panic, must return imported_ok=false,
    /// and must not create any new world folder.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn import_picked_world_garbage_bytes_fails_gracefully() {
        use crate::save::list_world_entries;

        // Isolate the worlds dir to a private per-thread temp dir so the exact
        // count assertion below can't race other FS tests creating/removing
        // folders in the shared `worlds/` dir under cargo's parallel runner.
        let _root = crate::save::WorldsRootGuard::new("import-garbage");

        // Count existing worlds before the call so we can verify nothing new appeared.
        let count_before = list_world_entries().len();

        let (status, imported_ok) = super::import_picked_world(b"not a real archive");

        assert!(!imported_ok, "import_picked_world must return imported_ok=false for garbage bytes");
        assert!(
            status.to_lowercase().contains("failed") || status.to_lowercase().contains("import"),
            "status must convey failure, got: {status:?}"
        );

        // No new world folder should have been created.
        let count_after = list_world_entries().len();
        assert_eq!(
            count_after, count_before,
            "garbage import must not create any new world folder (before={count_before}, after={count_after})"
        );
    }

    // ── Status-copy distinguishability (Phase 5a) ────────────────────────────
    // The lobby's single `transfer_status` line is shared by all four
    // world-transfer paths. These guard that the picker paths read clearly
    // differently from the folder-fallback paths, so a future copy regression
    // that blurs them is caught. They exercise the REAL status producers — no
    // mocks — but only the string-shaping branches that need no disk I/O.

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_as_picker_status_distinct_from_folder_export() {
        use crate::native_file_dialog::FileDialogResult;

        // Save-As picker success → "Saved to <path>".
        let mut status: Option<String> = None;
        let path = std::path::PathBuf::from("/somewhere/My World.axeworld");
        super::route_dialog_result(FileDialogResult::WorldSaved(path), &mut status);
        let picker = status.expect("WorldSaved must set a status");

        assert!(
            picker.contains("Saved to"),
            "picker-export copy must say 'Saved to', got: {picker:?}"
        );
        // It must NOT read like the folder fallback, which names the folder.
        assert!(
            !picker.contains("world-transfer"),
            "the Save-As picker path must not mention the world-transfer folder, got: {picker:?}"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn import_picker_status_distinct_from_folder_scan() {
        use crate::save::{world_dir, WorldMeta, write_world_folder};
        use crate::world::World;
        use crate::block;

        // Build a real world, export it, and import via the picker helper so we
        // assert the ACTUAL returned status copy (not a re-derived literal).
        // Unique nonce so this test's folders never overlap another test's; the
        // sweep prefix is the sanitized stem (lowercase, leading `_` trimmed).
        let src_name = "__menu_import_copy_5a_b41f7c";
        let _import_sweep = PrefixCleanup("menu_import_copy_5a_b41f7c".to_string());
        let mut world = World::new();
        world.set_block(0, 0, 0, block::STONE);
        let meta = WorldMeta::new(src_name);
        let save = crate::save::minimal_world_save_for_tests(7);
        write_world_folder(src_name, &meta, &save, &world)
            .expect("write_world_folder failed in test setup");
        let _src_guard = Cleanup(world_dir(src_name));

        let bytes = crate::native_world_io::export_world_native(src_name)
            .expect("export_world_native failed in test setup");

        let (picker_status, ok) = super::import_picked_world(&bytes);
        assert!(ok, "valid bytes must import");
        // Picker copy must name the picked file — the distinguishing phrase.
        assert!(
            picker_status.contains("from the file you picked"),
            "import-picker copy must say 'from the file you picked', got: {picker_status:?}"
        );

        // Folder-scan copy must read as a scan of the folder, not a single file.
        // With an empty world-transfer/ it short-circuits, but still references
        // the folder, which is the bit we need to be distinguishable.
        let scan_status = super::native_import_worlds();
        assert!(
            scan_status.contains("world-transfer"),
            "folder-scan copy must reference world-transfer/, got: {scan_status:?}"
        );
        assert!(
            !scan_status.contains("from the file you picked"),
            "folder-scan copy must not read like the single-file picker, got: {scan_status:?}"
        );
    }

    // ── export_skin_filename tests (Task A2) ─────────────────────────────────

    #[test]
    fn export_filename_is_kid_safe_slug() {
        assert_eq!(super::export_skin_filename("My Cool Skin"), "axenstax-my-cool-skin.png");
        assert_eq!(super::export_skin_filename("Knight!!"), "axenstax-knight.png");
        assert_eq!(super::export_skin_filename(""), "axenstax-my-skin.png");
        assert_eq!(super::export_skin_filename("   "), "axenstax-my-skin.png");
        // Collapses runs and trims hyphens.
        assert_eq!(super::export_skin_filename("a   b"), "axenstax-a-b.png");
    }


    // ── P6 audit: wasm "Create" folder-name dedup (dedupe_folder_name) ───────

    #[test]
    fn dedupe_folder_name_passes_through_a_unique_name() {
        let existing = vec!["some_other_world".to_string()];
        assert_eq!(super::dedupe_folder_name("my_world", &existing), "my_world");
    }

    #[test]
    fn dedupe_folder_name_suffixes_on_a_single_collision() {
        // The audit's literal repro: "Create" with a name that collides with
        // an already-loaded world must NOT return the same folder (which
        // would make the caller land back in the old world).
        let existing = vec!["my_world".to_string()];
        let got = super::dedupe_folder_name("my_world", &existing);
        assert_ne!(got, "my_world");
        assert_eq!(got, "my_world_2");
    }

    #[test]
    fn dedupe_folder_name_skips_already_taken_suffixes() {
        let existing =
            vec!["new_world_3".to_string(), "new_world_3_2".to_string(), "new_world_3_3".to_string()];
        let got = super::dedupe_folder_name("new_world_3", &existing);
        assert_eq!(got, "new_world_3_4", "must skip every suffix already in use, got {got}");
        assert!(!existing.contains(&got));
    }

    #[test]
    fn dedupe_folder_name_handles_the_delete_then_recreate_scenario() {
        // "New World 1/2/3", #1 deleted, then Create with a blank name
        // auto-names "New World 3" again — colliding with the surviving #3.
        let existing = vec!["new_world_2".to_string(), "new_world_3".to_string()];
        let got = super::dedupe_folder_name("new_world_3", &existing);
        assert_ne!(got, "new_world_3", "must not silently reuse the surviving world's folder");
    }

    #[test]
    fn dedupe_folder_name_never_loops_forever_under_many_collisions() {
        let existing: Vec<String> =
            std::iter::once("w".to_string()).chain((2..50).map(|i| format!("w_{i}"))).collect();
        let got = super::dedupe_folder_name("w", &existing);
        assert!(!existing.contains(&got), "got {got}, which is already taken");
    }
}

// ── Native Signet sign-in dialog ─────────────────────────────────────────────
// The QR (nostrconnect://) + paste (bunker://) sign-in screen, driven by the
// off-thread `native_signin` manager. Native-only — the WASM sign-in path is
// `auth.js`.

/// Whether the sign-in dialog should stay open or close.
#[cfg(not(target_arch = "wasm32"))]
enum SignInDialogResult {
    Stay,
    Close,
}

/// Render the sign-in dialog. Auto-starts the QR flow on open, shows live status,
/// and offers a paste-`bunker://` fallback.
#[cfg(not(target_arch = "wasm32"))]
fn draw_signin_dialog(
    ctx: &egui::Context,
    paste_uri: &mut String,
    show_paste: &mut bool,
    relays: &[String],
    show_relays: &mut bool,
) -> SignInDialogResult {
    use crate::native_signin::{self, SignInStatus};

    native_signin::poll();
    // The moment the dialog opens (Idle), generate the QR and begin listening
    // on the player's own relays. Editing the relays resets this to Idle (the
    // caller does that), so the QR is rebuilt with the new list.
    if matches!(native_signin::status(), SignInStatus::Idle) {
        // A sign-in can commit an instant before a "Try again" / relay-edit
        // reset (native_signin's guard keeps it, by design). Don't then offer
        // a fresh QR to a player who is already signed in — show that instead.
        match crate::signet::native_signer::current_owner_pubkey() {
            Some(pubkey_hex) => native_signin::show_signed_in(pubkey_hex),
            None => native_signin::start_qr(relays),
        }
    }
    let status = native_signin::status();

    let mut result = SignInDialogResult::Stay;
    egui::Window::new("Sign in with Signet")
        .max_width(crate::ui_safe_area::rect(ctx).width() - 24.0)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(360.0);
            match &status {
                SignInStatus::AwaitingScan { uri } => {
                    ui.label("Scan this with the Signet app on your phone, then approve.");
                    ui.add_space(10.0);
                    // 280 px: the URI carries up to three relays, so the code
                    // has more modules than the old one-relay QR.
                    ui.vertical_centered(|ui| draw_qr(ui, uri, 280.0));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Waiting for your phone…");
                    });
                    ctx.request_repaint();
                }
                SignInStatus::AwaitingApproval => {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Connected — approve the sign-in in Signet on your phone.");
                    });
                    ctx.request_repaint();
                }
                SignInStatus::Success { npub } => {
                    ui.colored_label(
                        egui::Color32::from_rgb(150, 220, 150),
                        format!("✓ Signed in as {}", crate::plan::short_npub(npub, 28)),
                    );
                    ui.add_space(10.0);
                    if ui.button("Done").clicked() {
                        result = SignInDialogResult::Close;
                    }
                }
                SignInStatus::Failed { message } => {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 150, 150),
                        format!("Sign-in didn't complete: {message}"),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Try again").clicked() {
                            // Back to Idle → the QR auto-restarts next frame.
                            native_signin::reset();
                        }
                        if ui.button("Close").clicked() {
                            result = SignInDialogResult::Close;
                        }
                    });
                }
                SignInStatus::Idle => {
                    ui.label("Starting…");
                    ctx.request_repaint();
                }
            }

            ui.add_space(12.0);
            ui.separator();
            let header = if *show_paste {
                "▾ Paste a bunker:// link instead"
            } else {
                "▸ Trouble scanning? Paste a bunker:// link"
            };
            if ui.selectable_label(false, header).clicked() {
                *show_paste = !*show_paste;
            }
            if *show_paste {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Paste a bunker:// link from Signet, then Connect.")
                        .size(11.0)
                        .color(SUBTITLE_COLOR),
                );
                ui.text_edit_singleline(paste_uri);
                if ui.button("Connect with link").clicked() {
                    native_signin::start_paste(paste_uri.as_str());
                }
            }

            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(format!(
                    "Signing in through: {}",
                    signin_relay_hosts(&native_signin::signin_relays(relays))
                ))
                .size(11.0)
                .color(SUBTITLE_COLOR),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    result = SignInDialogResult::Close;
                }
                if ui
                    .button("Relays")
                    .on_hover_text("Choose the relays used to sign in, before you scan")
                    .clicked()
                {
                    *show_relays = true;
                }
            });
        });
    result
}

/// `wss://relay.damus.io` → `relay.damus.io`, joined for the sign-in dialog.
#[cfg(not(target_arch = "wasm32"))]
fn signin_relay_hosts(relays: &[String]) -> String {
    relays
        .iter()
        .map(|r| r.trim_start_matches("wss://").trim_end_matches('/'))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Draw a QR code for `data` as a `size_px`-square grid of black/white cells in
/// egui (no texture upload — small module count, cheap). Includes a 2-module
/// quiet zone so scanners lock on.
///
/// `pub(crate)` so `friends_ui` can reuse it rather than growing a second QR
/// renderer.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_qr(ui: &mut egui::Ui, data: &str, size_px: f32) {
    let code = match qrcode::QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(_) => {
            ui.colored_label(
                egui::Color32::from_rgb(230, 150, 150),
                "Couldn't render the QR — use the paste link below.",
            );
            return;
        }
    };
    let colors = code.to_colors();
    let n = code.width();
    let quiet = 2usize;
    let total = (n + quiet * 2) as f32;
    let cell = size_px / total;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size_px, size_px), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, egui::Color32::WHITE);
    for y in 0..n {
        for x in 0..n {
            if matches!(colors[y * n + x], qrcode::Color::Dark) {
                let min = rect.min
                    + egui::vec2((x + quiet) as f32 * cell, (y + quiet) as f32 * cell);
                // ceil the cell so adjacent modules don't leave hairline gaps.
                painter.rect_filled(
                    egui::Rect::from_min_size(min, egui::vec2(cell.ceil(), cell.ceil())),
                    0.0,
                    egui::Color32::BLACK,
                );
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "menu_keeps_tests.rs"]
mod keeps_dialog_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "menu_phone_panels_tests.rs"]
mod phone_panels_tests;

//! Axe'n'Stax — Prototype build.
//!
//! Step 19: Splash screen + world management menu.
//!
//! Controls:
//!   WASD        — Move
//!   Mouse       — Look around
//!   Left click  — Break block
//!   Right click — Place block
//!   1-9         — Select block type
//!   Scroll      — Cycle block selection
//!   Space       — Jump (double-tap = toggle creative flight)
//!   Shift       — Sneak (descend in flight)
//!   Ctrl        — Sprint
//!   Escape      — Release mouse / quit
//!   F3          — Toggle debug overlay (position display)
//!   F7          — Toggle spawn-proof overlay (#8: red = mobs spawn here at night)
//!   M           — Open/close the full-screen map (#6)
//!   P           — (Workshop) Pin the blown-up block onto every instance
//!   K           — (Workshop) open the Wardrobe (design gallery)

// Phase 4b (2026-07-06): the two style lints we accept rather than fight —
// long-standing wide signatures in game_loop/renderer wiring, and egui/wgpu
// generic soup. Everything else is fixed, and check.sh now runs -D warnings.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

// Cross-platform modules
mod audio;
mod biome;
mod block;
mod camera;
mod chunk;
mod meta;
mod block_shape;
mod block_update;
mod power;
mod beam;
mod graphics_settings;
mod input;
mod item;
mod inventory;
mod mesh;
mod micro_model;
mod micro_model_assets;
mod micro_model_registry;
mod mipmap;
mod npub;
mod official_overrides;
mod override_registry;
mod workshop;
mod workshop_painter;
mod physics;
mod placement;
mod play_mode;
mod rail;
mod cart;
mod player_intent;
mod raycast;
mod renderer;
mod resource_pack;
mod texture_anim;
mod texture_gen;
mod texture_packs_web;
mod texture_registry;
mod world;
mod campfire_ui;
mod power_ui;
mod leaf_decay;
mod lighting;
mod nostrich;
mod nostrich_ride;
mod nostrich_vow;
mod water;
mod lava;
mod fire;
mod weather;
// Wind (Wind/Copper/Electricity wave §2.1) — a derived, deterministic breeze
// (tick + weather + seed + altitude). Nothing saved, nothing synced.
mod wind;
mod dispenser;
mod particles;
mod fluids;
mod entity;
mod mob;
mod mob_ai;
mod combat;
mod death_drops;
mod game_window;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod test_game_harness;
mod crafting;
mod crafting_catalogue;
mod scenario;
mod showcase;
mod showcase_ui;
mod open_stash;
mod gamestr;
mod craft_ui;
mod recipe_book_ui;
mod egui_integration;
mod menu;
// Online play by contact — the lobby's people surface (your address, friends).
// Native only: there is no online play on the web taster.
#[cfg(not(target_arch = "wasm32"))]
mod friends_ui;
mod hud_ui;
mod minimap;
mod map_ui;
mod waypoint;
mod worldedit;
mod world_exit;
mod spawn_overlay;
mod build_guide;
mod build_steps;
mod nbt;
mod schematic;
mod narration;
mod test_board;
mod my_servers;
mod server_resolve;
#[cfg(not(target_arch = "wasm32"))]
mod server_resolve_native;
#[cfg(target_arch = "wasm32")]
mod server_resolve_web;
mod admission;
// Online play by contact — WHO may join (identity), as distinct from `admission`
// above, which is WHETHER there is room (capacity). Native only.
#[cfg(not(target_arch = "wasm32"))]
mod online_admission;
mod access_policy;
mod comms;
mod charter;
mod contacts;
mod guardian_copy;
// Online play by contact (2026-09-06) — the invite link/QR format. Native only:
// the web build is the anonymous local sandbox and carries no online surface.
#[cfg(not(target_arch = "wasm32"))]
mod invite;
// Online play by contact — the per-install runtime key + its persona
// attestation. Native only (bunker signing, profile/ files).
#[cfg(not(target_arch = "wasm32"))]
mod runtime_identity;
// Online play by contact — the encrypted setup handshake over public relays.
// Native only (tokio websockets, NIP-44, bunker-adjacent identity).
#[cfg(not(target_arch = "wasm32"))]
mod rendezvous;
// Online play by contact — NAT traversal (STUN, candidates, UPnP, punching).
#[cfg(not(target_arch = "wasm32"))]
mod nat;
// Online play by contact — host-side orchestration.
#[cfg(not(target_arch = "wasm32"))]
mod online_host;
// Online play by contact — joiner-side orchestration + the failure copy.
#[cfg(not(target_arch = "wasm32"))]
mod online_join;
// Online play by contact — the phone tap, the socket and the router, off-loop.
#[cfg(not(target_arch = "wasm32"))]
mod online_prep;
mod world_room;
#[cfg(not(target_arch = "wasm32"))]
mod kithmoot_keeper;
mod kick;
mod privacy;
#[cfg(not(target_arch = "wasm32"))]
mod server_card_publish;
#[cfg(not(target_arch = "wasm32"))]
mod console_settings;
#[cfg(not(target_arch = "wasm32"))]
mod console_telemetry;
#[cfg(not(target_arch = "wasm32"))]
mod console_snapshot;
#[cfg(not(target_arch = "wasm32"))]
mod operator_panel;
#[cfg(not(target_arch = "wasm32"))]
mod console_auth;
#[cfg(not(target_arch = "wasm32"))]
mod native_mailbox;
// Hidden tester-feedback unlock (Settings: tap the version line 7 times).
#[cfg(not(target_arch = "wasm32"))]
mod tester_gate;
// Signet contacts sync (native only) — docs/foundations/2026-10-01-signet-contacts-sync.md.
#[cfg(not(target_arch = "wasm32"))]
mod signet_contacts;
mod held_item_model;
mod splash_ui;
mod loading_screen;
mod exhibit;
mod entity_model;
mod viewmodel;
mod game_loop;
mod chunk_stream;
mod block_interact;
mod falling_blocks;
mod growth;
mod campfire;
mod composter; // Spec 49 (Explosives) — the farming nitre-bed (Compost → Saltpetre).
mod explosion; // Spec 49 (Explosives) — Blasting Keg blast resolution (pure helpers + resolve_blast).
mod furnace;
mod furnace_ui;
mod wardrobe_store;
mod wardrobe_ui;
mod vendor;
mod vendor_ui;
mod chest;
mod chest_ui;
mod sign;
mod sign_ui;
mod item_frame;
mod challenge_board_ui;
// #19 dynamic/animated asset authoring — Phase A skeleton data model + Phase B
// animation-set evaluator, now consumed by the Rig Studio (rig_studio_ui) +
// entity_model::build_rigged_vertices.
mod skeleton;
mod anim_set;
mod rig_studio_ui;
mod hostile_acts;
mod grave;
mod bear_ai;
mod hyena_ai;
mod brigand;
mod brigand_hideout_gen;
mod salt_lick;
mod snowfall;
mod rubber;
mod bucket;
mod slingshot;
mod bounty;
mod bounty_ui;
mod tip_jar;
mod tip_jar_ui;
mod repair;
mod repair_ui;
mod plot;
mod market_hub;
mod market_hub_ui;
mod auction;
mod auction_ui;
mod bazaar;
mod bazaar_ui;
mod inventory_explorer;
mod wolf;
mod horse_ai;
mod rabbit_ai;
mod goat_ai;
mod bee_ai;
mod squid_ai;
mod bee_hive;
mod species_ai;
mod pure_helpers;
mod worldgen_helpers;
mod armour;
mod tree_shapes;
mod crop_growth;
mod animal_products;
mod server_economy;
mod sapling;
mod tameable;
mod pet_bed;
mod workstation;
mod drying_rack;
mod papyrus;
mod plan;
mod plan_registry;
mod plan_ui;
mod latent_print;
mod blueprint_attach;
mod tether;
mod breeding;
mod genetics;
mod companion;
mod fishing;
mod hopper;
mod piston;
mod builder;
mod commission_ui;
mod village_bell_ui;
mod villager;
mod village_gen;
mod satoshi;
mod profile_bundle;
mod trials;
mod ravine_gen;
mod mineshaft_gen;
mod villager_ui;
mod quest;
mod reputation;
mod economy;
mod raid;
mod spawn_pref;
mod parity_check;
mod spawning;
mod screen;
mod player_slot;
mod protocol;
mod signet;
mod skin_grid;
mod skin_hit;
mod skin_layers;
mod skin_paint;
mod skin_pose;
mod skin_uv;
mod skin_wardrobe;
mod skin_wardrobe_store;
mod commands;
mod cosmetics;
mod mc_import;
mod chat_ui;
mod reserve;
// Proof of Play — the per-strike HMAC-SHA256 hash + Satori vein algorithm
// (Spec 6 §2, §2.2c). Pure, deterministic, server-side.
mod proof_of_play;

// Gamepad + local-join coordination — cross-platform after spec 15.
// Native uses gilrs; WASM uses `navigator.getGamepads()`.
mod gamepad;
mod local_join;

// Native-only modules
mod save;
mod data_dir;
// Transport, server sim, and HostedServer are cross-platform so single-player
// (alpha PWA target) can route through a local in-process HostedServer. The
// QUIC/LAN discovery layer below stays native-only because it depends on
// tokio + quinn, which don't run in wasm32-unknown-unknown.
mod transport;
mod server;
mod hosted_server;
#[cfg(not(target_arch = "wasm32"))]
mod network;
#[cfg(not(target_arch = "wasm32"))]
mod discovery;
mod remote_client;
mod remote_entities;
// WebSocket transport — the dedicated-server pipe that BOTH the browser PWA and
// the native client speak (browsers can't do QUIC). Native half here; the
// browser half is `ws_transport_web` (wasm32-only).
#[cfg(not(target_arch = "wasm32"))]
mod ws_transport;
#[cfg(target_arch = "wasm32")]
mod ws_transport_web;
// Headless dedicated-server entry point (`--server`). Native-only — it never
// creates a window/renderer, so it runs on a GPU-less Docker host.
#[cfg(not(target_arch = "wasm32"))]
mod server_main;
// Heartwood-backed server identity: delegated runtime key authorised by an
// operator's NIP-46 bunker. Native-only (uses `nostr`/`signet-nip46-client`).
#[cfg(not(target_arch = "wasm32"))]
mod server_identity;
// Native Signet sign-in driver (off-thread NIP-46 pairing for the lobby dialog).
#[cfg(not(target_arch = "wasm32"))]
mod native_signin;
// "Your relays" — the one relay list's editor (Spec 04 §1.9): settings panel,
// sign-in dialog and lobby. Native only; the web taster uses no relays.
#[cfg(not(target_arch = "wasm32"))]
mod relays_ui;
// Native OS file-dialog seam (world Export/Import + skin upload). Off-thread,
// mirrors native_signin's channel-report pattern. Never compiled on WASM.
#[cfg(not(target_arch = "wasm32"))]
mod native_file_dialog;
// Version + update-available indicator. Native-only: the web build is always
// current by construction, so the indicator would be meaningless there.
#[cfg(not(target_arch = "wasm32"))]
mod update_check;
// In-place AppImage self-updater — downloads + sha256-verifies + installs the
// newest Linux build over the running one when `update_check` reports an
// `Available` update with an AppImage reference. Native-only, same reason as
// `update_check`; further gated at the call site to a build that IS a
// running AppImage (see `self_update::running_appimage`).
#[cfg(not(target_arch = "wasm32"))]
mod self_update;
// Second update source for `update_check`: a kind-30063 Nostr release event,
// so an update can be discovered with no web server involved (the HTTP
// `latest.json` path is a single host and has gone dark before). Native-only,
// same reason as `update_check`/`self_update`.
#[cfg(not(target_arch = "wasm32"))]
mod nostr_release;
// Anti-X-ray chunk-stream obfuscation (Spec 8 §5.2.2) — buried ore → host rock
// in the data sent to clients. Pure + cross-platform (the server send-path and a
// future WASM spectator both need it); the foundation the networked spectator
// (design Phase 3) builds on. Currently unwired (no remote chunk send-path yet).
mod anti_xray;
// Cinematic Director (Phase 1) — a render-only camera rig detached from the
// avatar (six modes + keyframe paths). Native-only so the web bundle + the
// spectator/anti-X-ray surface are untouched. The replay Director (Phase 2)
// reuses these.
#[cfg(not(target_arch = "wasm32"))]
mod camera_path;
#[cfg(not(target_arch = "wasm32"))]
mod director;
// Cinematic replay (Phase 2) — the `.axereplay` format + recorder + native
// store, and the playback engine. Native-only. The Director (Phase 1) is the
// playback viewpoint.
#[cfg(not(target_arch = "wasm32"))]
mod replay;
#[cfg(not(target_arch = "wasm32"))]
mod replay_player;

// Test-only modules — integration harness (Spec 3 Layer B) plus the
// integration test suites that use it (Layer C). Both live inside the bin
// under `#[cfg(test)]` because the engine is a bin-only crate; if a
// `src/lib.rs` is ever added, these move to `tests/` with no test-code
// changes beyond import paths.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod test_harness;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod test_integration;

// `touch_input` is cross-platform: the menu's text fields call `is_touch_device()`
// / `os_keyboard_prompt()` on every target (native gets cheap no-op stubs), so the
// module must always compile. The on-screen overlay drawing + touch-event handling
// is WASM-only (its callers are wasm-gated), hence dead_code is allowed on native
// non-test builds where only the lightweight detection/keyboard API is reached.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
mod touch_input;

// WASM-only modules
#[cfg(target_arch = "wasm32")]
mod web_main;
#[cfg(target_arch = "wasm32")]
mod wasm_auth;
// Cross-platform archive helpers (pack/unpack + dedupe_world_name).
// No cfg gate — compiles on native and wasm32.
mod world_archive;
// Native-only world export / import: bridges world_archive <-> on-disk folders.
// Uses std::fs, so wasm32 must not compile it.
#[cfg(not(target_arch = "wasm32"))]
mod native_world_io;
// Cross-platform: pure helpers are now in `world_archive`; wasm_save re-exports
// them and adds the IndexedDB/file-picker glue (wasm-only).
mod wasm_save;
#[cfg(target_arch = "wasm32")]
mod web_logger;

use std::sync::Arc;
use web_time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, KeyEvent, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::block::BlockRegistry;
use crate::camera::CameraUniform;
use crate::input::InputState;
use crate::mesh::build_chunk_meshes;
use crate::renderer::Renderer;
use crate::world::World;
use crate::water::WaterSystem;
use crate::leaf_decay::LeafDecaySystem;

/// Tick rate: 20 TPS (Spec 05 Section 1.8)
const TICK_DURATION: Duration = Duration::from_millis(50);

/// Maximum chunk Y coordinate for world generation
const MAX_CHUNK_Y: i32 = 5;

/// Block reach distance (Spec 05 Section 2.5: 4.5 blocks survival)
const REACH_DISTANCE: f32 = 5.0;

/// Pets wave Task 13 — extra block/entity interaction reach granted while a
/// Reach Claw is the active hotbar item. See `GameState::effective_reach`.
pub const REACH_CLAW_BONUS: f32 = 2.0;

/// Max columns to generate + mesh per frame during streaming (after initial load)
const STREAM_BUDGET: usize = 4;

// `WORLD_NAME` and `HOTBAR_BLOCKS` (early-prototype defaults, predating the
// real world-management menu and item/inventory system) were removed here —
// zero references anywhere.

pub(crate) enum GameMode {
    Splash(crate::splash_ui::SplashState),
    /// Boxed — `MenuState` is by far the largest variant's payload (488 bytes),
    /// so boxing keeps the other variants (notably the frequently-matched
    /// `Playing`/`Paused`) from paying for its size.
    Menu(Box<crate::menu::MenuState>),
    /// Live, animated world-load state (native + WASM). Each frame drives
    /// `begin_load` (first frame) then `step_load` (a budget of columns) while
    /// painting `loading_screen::draw_loading_screen` with real progress + a
    /// rotating tip card — so the load never freezes the thread. Transitions to
    /// `Playing` once the queue is drained AND the minimum display time elapsed.
    Loading(crate::loading_screen::LoadingState),
    Playing,
    Paused { confirm_quit: bool, confirm_creative: bool },
}

/// Where the last painted texel landed, for Shift+click straight lines. The
/// fractional hit is kept alongside the resolved texel so the MIRRORED side of
/// a symmetric stroke can rule its own line: the stored hit is put back through
/// `skin_uv::mirror_hit` to get that side's start point, rather than the
/// mirrored half getting a lone dot while the primary half gets a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LastPaint {
    pub part: usize,
    pub face: usize,
    pub layer: crate::skin_uv::SkinLayer,
    pub frac_u: f32,
    pub frac_v: f32,
}

/// In-world avatar-paint session (replaces the retired Skin Studio panel state).
/// Lives only while a Workshop avatar mannequin is blown up + being painted.
pub(crate) struct SkinPaintSession {
    /// The Avatar WorkshopProject this session paints.
    pub project_id: u32,
    /// The wardrobe entry Pin writes back to (equipped by default; the chosen
    /// entry when arrived via "Your look → Edit").
    pub editing_id: crate::skin_wardrobe::SkinId,
    /// The working 64×64×4 RGBA buffer (seeded from `editing_id`'s skin).
    pub buffer: Vec<u8>,
    /// Are the clothes (overlay) shown AND selected for painting? One flag drives
    /// drawing, hit-testing and the panel caption together, so painting a pixel
    /// you cannot see is not expressible. Defaults from the skin itself — see
    /// [`crate::skin_layers::clothes_layer_has_content`].
    pub clothes_on: bool,
    /// Target pose: are the limbs separated so every face can be reached?
    pub limbs_apart: bool,
    /// Eased separation factor, 0 = together, 1 = apart. Lerps toward
    /// `limbs_apart`. Handed to BOTH the renderer and the hit-test.
    pub limbs_t: f32,
    /// Which tool the next click applies (Skindex parity, 2026-09-06). Replaces
    /// the old `erase: bool` — the eraser is a TOOL, and so are fill and the
    /// three shading brushes, so no two of them can be "on" at once. The
    /// eraser is still distinct from "no dye in hand", which cannot be
    /// expressed as a palette swatch but erases just the same.
    pub tool: crate::skin_paint::PaintTool,
    /// Texels already modified by a shading brush during the CURRENT stroke.
    /// Cleared on the mouse-down edge ([`SkinPaintSession::begin_stroke`]) and
    /// consulted by Lighten/Darken/Noise, so holding the button on one spot
    /// can't compound to white/black and a mirrored hit that folds onto the
    /// same texel can't apply twice.
    pub touched: crate::skin_paint::TouchedSet,
    /// Where the previous click landed, so Shift+click can rule a straight line
    /// from there to the new texel. `None` until the first paint of the session.
    pub last_paint: Option<LastPaint>,
    /// Recently-chosen colours, most-recent-first, capped at
    /// [`crate::skin_paint::RECENT_CAP`]. Fed by the wheel, the hex box, the dye
    /// swatches and the eyedropper alike.
    pub recent: Vec<[u8; 4]>,
    /// Edit buffer for the panel's `#rrggbb` box. Kept as raw text so a
    /// half-typed value ("#ff88") isn't fought over mid-keystroke — it only
    /// commits to `picked_color` when it parses.
    pub hex_input: String,
    /// Seeded RNG for the Noise brush. A plain LCG, not the `rand` crate: this
    /// has to build for wasm32 with no extra dependency.
    pub rng: crate::skin_paint::Lcg,
    /// Brush size for the whole brush family (Brush/Eraser/Lighten/Darken/
    /// Noise). Fill ignores it.
    pub brush: u32,
    /// Left/right symmetry (M toggles).
    pub mirror: bool,
    /// Eyedropper override: takes precedence over the held dye until cleared.
    pub picked_color: Option<[u8; 4]>,
    /// Which hotbar slot was held when `picked_color` was set (cleared on change).
    pub picked_slot: usize,
    /// Per-stroke undo snapshots (cap [`SkinPaintSession::HISTORY_CAP`]).
    pub undo: Vec<Vec<u8>>,
    /// Campaign S (2026-07-05) — redo snapshots (undone strokes). A new
    /// stroke clears this branch; same cap as `undo`.
    pub redo: Vec<Vec<u8>>,
    /// Which arm shape the mannequin (and therefore the hit-test, the grid and
    /// the footprint) is built on. Seeded from the entry being edited, changed
    /// live by the panel's Arms row, and written back to the entry on Pin —
    /// so what you paint on is what you end up wearing.
    pub arm_model: crate::skin_uv::ArmModel,
    /// Precision aids (2026-09-06) — the texel grid drawn over the mannequin
    /// and the memo that keeps it from being rebuilt every frame. On by
    /// default; L (or the panel row) toggles it. See [`crate::skin_grid`].
    pub grid: crate::skin_grid::GridAid,
}

impl SkinPaintSession {
    /// Cap on undo/redo history (per-stroke full-buffer snapshots). 64 × 16 KiB
    /// = 1 MiB per stack — cheap enough that a long painting session can be
    /// walked all the way back.
    pub const HISTORY_CAP: usize = 64;

    /// Which skin layer clicks land on. Clothes visible IS clothes selected.
    pub fn layer(&self) -> crate::skin_uv::SkinLayer {
        if self.clothes_on {
            crate::skin_uv::SkinLayer::Overlay
        } else {
            crate::skin_uv::SkinLayer::Base
        }
    }

    /// Ease the separation factor toward the target. Call once per frame with
    /// the frame delta; ~0.2 s to travel end to end so the limbs read as
    /// floating apart rather than teleporting.
    pub fn tick_limbs(&mut self, dt: f32) {
        const RATE: f32 = 5.0; // 1.0 / 0.2s
        let target = if self.limbs_apart { 1.0 } else { 0.0 };
        let step = RATE * dt;
        if (target - self.limbs_t).abs() <= step {
            self.limbs_t = target;
        } else if target > self.limbs_t {
            self.limbs_t += step;
        } else {
            self.limbs_t -= step;
        }
    }

    /// The player chose a colour (wheel, hex box, dye swatch, recent swatch or
    /// eyedropper). Records it as recent, refreshes the hex box, and puts the
    /// ERASER down — reaching for a colour means "paint with it". A shading
    /// brush is left selected: it ignores the colour, so swapping the tool out
    /// from under the player would be the surprise, not the courtesy.
    pub fn set_color(&mut self, c: [u8; 4]) {
        self.set_color_keep_hex(c);
        self.hex_input = crate::skin_paint::hex_of(c);
    }

    /// As [`SkinPaintSession::set_color`], but leaves the hex box's text alone.
    /// For the hex box itself: rewriting "#FF8800" to "#ff8800" under the
    /// player's cursor mid-keystroke is exactly the fight the edit buffer
    /// exists to avoid.
    pub fn set_color_keep_hex(&mut self, c: [u8; 4]) {
        self.picked_color = Some(c);
        if self.tool == crate::skin_paint::PaintTool::Eraser {
            self.tool = crate::skin_paint::PaintTool::Brush;
        }
        crate::skin_paint::push_recent(&mut self.recent, c);
    }

    /// B — walk the shading brushes (Brush → Lighten → Darken → Noise → Brush).
    /// Returns the newly-selected tool so the caller can announce it.
    pub fn cycle_tool(&mut self) -> crate::skin_paint::PaintTool {
        self.tool = self.tool.cycle_shading();
        self.tool
    }

    /// The mouse-down edge of a new stroke: reset the once-per-texel shading
    /// guard and snapshot for undo. `last_paint` deliberately SURVIVES — it is
    /// the anchor a Shift+click rules its line from, and that anchor is the
    /// previous stroke by definition.
    pub fn begin_stroke(&mut self) {
        self.touched.clear();
        self.snapshot_for_stroke();
    }

    /// A new stroke begins: snapshot the buffer for undo and drop any redo
    /// branch (painting after an undo forks history, like every editor).
    pub fn snapshot_for_stroke(&mut self) {
        self.undo.push(self.buffer.clone());
        if self.undo.len() > Self::HISTORY_CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Step back one stroke. `true` if a snapshot was restored.
    pub fn undo_step(&mut self) -> bool {
        let Some(prev) = self.undo.pop().filter(|p| p.len() == self.buffer.len()) else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.buffer, prev));
        if self.redo.len() > Self::HISTORY_CAP {
            self.redo.remove(0);
        }
        true
    }

    /// Step forward again after an undo. `true` if a snapshot was restored.
    pub fn redo_step(&mut self) -> bool {
        let Some(next) = self.redo.pop().filter(|p| p.len() == self.buffer.len()) else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.buffer, next));
        if self.undo.len() > Self::HISTORY_CAP {
            self.undo.remove(0);
        }
        true
    }
}

#[cfg(test)]
mod skin_paint_session_tests {
    use super::SkinPaintSession;

    fn session(fill: u8) -> SkinPaintSession {
        SkinPaintSession {
            project_id: 0,
            editing_id: Default::default(),
            buffer: vec![fill; 16], // any consistent size — the methods only
            // compare lengths, so tests don't need a full 64×64×4 buffer
            clothes_on: false,
            limbs_apart: false,
            limbs_t: 0.0,
            tool: crate::skin_paint::PaintTool::Brush,
            touched: crate::skin_paint::TouchedSet::default(),
            last_paint: None,
            recent: Vec::new(),
            hex_input: String::new(),
            rng: crate::skin_paint::Lcg::new(1),
            brush: 1,
            mirror: false,
            picked_color: None,
            picked_slot: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            arm_model: crate::skin_uv::ArmModel::Classic,
            grid: Default::default(),
        }
    }

    #[test]
    fn clothes_on_selects_the_overlay_layer() {
        let mut s = session(0);
        assert_eq!(s.layer(), crate::skin_uv::SkinLayer::Base);
        s.clothes_on = true;
        assert_eq!(s.layer(), crate::skin_uv::SkinLayer::Overlay);
    }

    #[test]
    fn limbs_ease_to_the_target_and_stop() {
        let mut s = session(0);
        s.limbs_apart = true;
        for _ in 0..100 {
            s.tick_limbs(1.0 / 60.0);
        }
        assert_eq!(s.limbs_t, 1.0, "settles exactly on the target");
        s.limbs_apart = false;
        for _ in 0..100 {
            s.tick_limbs(1.0 / 60.0);
        }
        assert_eq!(s.limbs_t, 0.0);
    }

    #[test]
    fn undo_then_redo_round_trips_and_new_stroke_clears_redo() {
        let mut s = session(0);
        // Stroke 1: snapshot, then paint the buffer to 1s.
        s.snapshot_for_stroke();
        s.buffer = vec![1; 16];
        // Undo → back to 0s; redo → forward to 1s.
        assert!(s.undo_step());
        assert_eq!(s.buffer, vec![0; 16]);
        assert!(s.redo_step());
        assert_eq!(s.buffer, vec![1; 16]);
        // Undo again, then a NEW stroke forks history: redo branch is gone.
        assert!(s.undo_step());
        s.snapshot_for_stroke();
        s.buffer = vec![2; 16];
        assert!(s.redo.is_empty(), "a new stroke clears the redo branch");
        assert!(!s.redo_step(), "nothing to redo after the fork");
        // And undo still walks back through the fork.
        assert!(s.undo_step());
        assert_eq!(s.buffer, vec![0; 16]);
    }

    #[test]
    fn history_caps_hold() {
        let mut s = session(0);
        for i in 0..(SkinPaintSession::HISTORY_CAP as u8 + 16) {
            s.snapshot_for_stroke();
            s.buffer = vec![i; 16];
        }
        assert_eq!(s.undo.len(), SkinPaintSession::HISTORY_CAP);
        let mut undone = 0;
        while s.undo_step() {
            undone += 1;
        }
        assert_eq!(undone, SkinPaintSession::HISTORY_CAP);
        assert_eq!(s.redo.len(), SkinPaintSession::HISTORY_CAP);
    }

    // ── Skindex-parity tool set (2026-09-06) ──────────────────────────────

    #[test]
    fn choosing_a_colour_records_it_and_fills_the_hex_box() {
        let mut s = session(0);
        s.set_color([255, 136, 0, 255]);
        assert_eq!(s.picked_color, Some([255, 136, 0, 255]));
        assert_eq!(s.recent, vec![[255, 136, 0, 255]]);
        assert_eq!(s.hex_input, "#ff8800");
    }

    #[test]
    fn choosing_a_colour_puts_the_eraser_down() {
        // Reaching for a colour means "paint with it" — the same rule the
        // `erase` bool carried before the tool enum existed.
        let mut s = session(0);
        s.tool = crate::skin_paint::PaintTool::Eraser;
        s.set_color([1, 2, 3, 255]);
        assert_eq!(s.tool, crate::skin_paint::PaintTool::Brush);
    }

    #[test]
    fn choosing_a_colour_leaves_a_shading_brush_selected() {
        // Lighten/Darken/Noise don't use the colour, but picking one while
        // they're up must not silently swap the tool out from under the kid.
        let mut s = session(0);
        s.tool = crate::skin_paint::PaintTool::Darken;
        s.set_color([1, 2, 3, 255]);
        assert_eq!(s.tool, crate::skin_paint::PaintTool::Darken);
    }

    #[test]
    fn b_walks_the_shading_brushes_on_the_session() {
        let mut s = session(0);
        assert_eq!(s.cycle_tool(), crate::skin_paint::PaintTool::Lighten);
        assert_eq!(s.cycle_tool(), crate::skin_paint::PaintTool::Darken);
        assert_eq!(s.cycle_tool(), crate::skin_paint::PaintTool::Noise);
        assert_eq!(s.cycle_tool(), crate::skin_paint::PaintTool::Brush);
        assert_eq!(s.tool, crate::skin_paint::PaintTool::Brush);
    }

    #[test]
    fn a_new_stroke_clears_the_per_stroke_shading_guard() {
        // Each press starts a fresh once-per-texel budget, so a second click on
        // the same spot darkens it again — but holding the button does not.
        let mut s = session(0);
        s.touched.insert((3, 4));
        s.begin_stroke();
        assert!(s.touched.is_empty(), "the touched set resets per stroke");
        assert_eq!(s.undo.len(), 1, "and the stroke still snapshots for undo");
    }

    #[test]
    fn empty_stacks_are_noops() {
        let mut s = session(7);
        assert!(!s.undo_step());
        assert!(!s.redo_step());
        assert_eq!(s.buffer, vec![7; 16], "buffer untouched by no-op steps");
    }
}

pub(crate) struct GameState {
    /// Name of the currently loaded world.
    pub(crate) world_name: String,
    pub(crate) renderer: Renderer,
    pub(crate) window: crate::game_window::GameWindow,
    pub(crate) world: World,
    pub(crate) registry: BlockRegistry,
    pub(crate) biome_gen: crate::biome::BiomeGenerator,
    pub(crate) input: InputState,
    pub(crate) audio: crate::audio::AudioEngine,
    pub(crate) last_tick: Instant,
    pub(crate) tick_accumulator: Duration,
    /// Wall-clock time the last frame was paced (Spec 39 A3 frame cap). Native
    /// only; WASM is paced by requestAnimationFrame.
    pub(crate) frame_pace_instant: Instant,
    pub(crate) show_debug: bool,
    /// #6 — minimap render state (explored-map cache + last baked texture +
    /// refresh throttle). Empty/None until the first frame paints it.
    pub(crate) minimap: crate::minimap::MinimapView,
    /// #6 — full-screen map overlay (M outside the Workshop). Shares the
    /// minimap's explored-tile cache; has its own centre/zoom/texture.
    pub(crate) map_screen: crate::minimap::MapScreen,
    /// #8 — spawn-proof overlay toggle (F7). When on, red wireframe markers
    /// paint surface cells where a mob can spawn at night.
    pub(crate) spawn_overlay: bool,
    /// Campaign S — the skin-paint colour picker window (C toggles it while an
    /// avatar paint session is open). Counts as a UI modal so the cursor frees.
    pub(crate) skin_picker_open: bool,
    /// Paint-panel buttons resolved on the next tick. The panel borrows the
    /// session mutably to draw, so the actions it triggers cannot run inline.
    pub(crate) pending_skin_undo: bool,
    pub(crate) pending_skin_redo: bool,
    pub(crate) pending_skin_pin: bool,
    /// #9 — active blueprint build-guide (a plan projected at an origin),
    /// rendered as status-coloured ghosts. `None` = no guide active.
    pub(crate) build_guide: Option<crate::build_guide::BuildGuide>,
    /// #24 — accessibility narration throttle (speaks the hotbar selection on
    /// change when `graphics.narration_enabled`).
    pub(crate) narrator: crate::narration::Narrator,
    /// Loaded column tracking for chunk streaming
    pub(crate) loaded_columns: ahash::AHashSet<(i32, i32)>,
    /// Test Lab: index into the ranked `test_board` registry of the mission
    /// Satoshi is currently offering. Session-only (not persisted) — a verdict
    /// goes to the mailbox immediately, so resuming just restarts the list.
    pub(crate) mission_idx: usize,
    /// Test Lab: the optional free-text note bound to the mission verdict field.
    pub(crate) mission_note: String,
    /// Test Lab: the ranked mission list, loaded once per world (lazily) from the
    /// embedded `test_board` registry. Empty outside a Test Lab world.
    pub(crate) mission_registry: Vec<crate::test_board::TestItem>,
    /// Trials (⚡ Race): the live attempt, `Some` while a trial is running.
    pub(crate) active_trial: Option<crate::trials::ActiveTrial>,
    /// Trials: a finished trial's result — `Some` shows the "Well done!" panel
    /// (Back to Trials / Try again), cleared when the player picks one.
    pub(crate) trial_outcome: Option<crate::trials::TrialOutcome>,
    /// Trials: personal bests per trial id (+ the ghost of each best run).
    /// Cross-world within a session; persisted to the local trials store.
    pub(crate) trial_bests: crate::trials::TrialBests,
    /// Columns still to generate+mesh during a `GameMode::Loading` world entry.
    /// Built by `chunk_stream::begin_load`, drained by `step_load` over frames
    /// so the load screen animates instead of freezing. Empty outside loading.
    pub(crate) load_queue: std::collections::VecDeque<(i32, i32)>,
    /// Chunk meshes queued for rebuild, drained at a per-frame budget (Spec 39
    /// A4) so a lighting/boundary edit storm can't stall a frame. The edited
    /// block's own chunk still rebuilds immediately for instant feedback.
    pub(crate) dirty_mesh_chunks: ahash::AHashSet<(i32, i32, i32)>,
    /// Counter for falling block ticks (runs every 4 ticks at 20 TPS = 5 Hz)
    pub(crate) falling_tick_counter: u32,
    /// Water flow system
    pub(crate) water: WaterSystem,
    /// Campaign B — lava flow (mirrors `water`; shorter range, slower, freezes
    /// to obsidian on water contact).
    pub(crate) lava: crate::lava::LavaSystem,
    /// Fire spread (2026-07-04 gap-fill wave) — scheduled burn/spread events;
    /// rain douses, `fire_spread_enabled` gates fuel consumption.
    pub(crate) fire: crate::fire::FireSystem,
    /// Particle framework (2026-07-05) — client-side visual pool (never
    /// touches save/protocol/server). Simulated per-FRAME with real dt.
    pub(crate) particles: crate::particles::ParticleSystem,
    /// Scratch instance vec reused every frame (no steady-state allocs).
    pub(crate) particle_instances: Vec<crate::mesh::ParticleInstance>,
    /// Frame clock for the particle sim's dt.
    pub(crate) last_particle_frame: Instant,
    /// Ambient-emitter pulse accumulator (rain/snow/fire ambience fire every
    /// ~0.15 s, not every frame).
    pub(crate) particle_ambient_accum: f32,
    /// Thunderstorm tier (2026-07-05): while `tick_counter < this`, the rain
    /// is a storm — lightning strikes roll. Transient like the rain window.
    pub(crate) weather_storm_until: u64,
    /// Lightning flash intensity 0..1, decays per frame; briefly boosts sky
    /// brightness + sky colour so the whole world blinks white.
    pub(crate) lightning_flash: f32,
    /// Previous-frame in-water flags for ECS entities (mob/item splash edge
    /// detect). Client-visual bookkeeping only; rebuilt every frame.
    pub(crate) entity_was_in_water: ahash::AHashMap<hecs::Entity, bool>,
    /// Leaf decay system
    pub(crate) leaf_decay: LeafDecaySystem,
    /// ECS world for mobs and other entities
    pub(crate) ecs: hecs::World,
    /// World time in ticks (0-23999, wraps). 0=sunrise, 6000=noon, 12000=sunset, 18000=midnight.
    pub(crate) world_time: u32,
    /// World-time advance per tick. 1 = standard 20-min day; 4 = 5-min alpha day.
    /// Configurable at runtime via `/time speed <n>`.
    pub(crate) world_time_step: u32,
    /// Monotonic tick counter — never wraps. Used for UI timestamps where
    /// `world_time` (5-min wrap on alpha) would alias.
    pub(crate) tick_counter: u64,
    /// Proof-of-Play server secret (32 bytes). Derived from the world seed
    /// at world creation for alpha; production loads from env var per Spec
    /// 6 §1.3. Drives the per-strike HMAC + Orange-Bitcoin-Gem vein
    /// algorithm (Spec 6 §2 / §2.2c).
    pub(crate) pop_server_secret: [u8; 32],
    /// Proof-of-Play epoch id. Always 0 in alpha; rotates per Spec 6 §2.6
    /// when seasonal commitment scheme is wired.
    pub(crate) pop_epoch_id: u32,
    /// Sparse "block-was-exposed-at-tick" map for pure-deepslate. Populated
    /// when a player mines a block face-adjacent to pure deepslate (Spec 6
    /// §2.2c.3). Absent entries are treated as "naturally exposed → fully
    /// decayed" so cave-found gems return nothing; players must mine into
    /// solid deepslate to refresh the exposure clock.
    pub(crate) pop_exposure_map: ahash::AHashMap<(i32, i32, i32), u64>,
    /// Chat overlay state (input field, log, history). Single-player only in v1.
    pub(crate) chat: chat_ui::ChatState,
    /// Spec 40 — the Workshop face-painter panel (the 16×16 paint-grid UI). Opened
    /// via `/ws edit <asset>` in the Workshop; single editor (player 0) in v1.
    pub(crate) workshop_painter: workshop_painter::WorkshopPainterUi,
    /// Spec 40 — the mannequin "play" toggle: when true, mob mannequins in the
    /// Workshop animate their walk cycle (preview); when false they stand still
    /// (the default — a posable mannequin). Toggled by `/ws play`.
    pub(crate) workshop_play: bool,
    /// Workshop Phase 5 Task 6 — Wardrobe panel open flag. `true` while the K-key
    /// panel is visible; cursor is released while open.
    pub(crate) wardrobe_open: bool,
    /// Wave 6 — challenge board open flag (feature-coverage Phase 6). `true`
    /// while the J-key board is visible; cursor released + gameplay gated.
    pub(crate) challenge_board_open: bool,
    /// #19 Rig Studio open flag (Y key). Cursor released + gameplay gated while open.
    pub(crate) rig_studio_open: bool,
    /// Trials — the in-game objective / help pop-up is showing (H key). A
    /// NON-blocking overlay (the game + any race clock keep running behind it),
    /// so you can glance at "what to do + progress" and dismiss with H.
    pub(crate) show_objective: bool,
    /// Trials — Satoshi's spoken intro card for the trial that just started:
    /// `(title, intro, expiry)`. Shown as a non-blocking Satoshi speech card at
    /// the top of the screen for a few seconds on trial start (his `intro` from
    /// `scenario::trial_satoshi`), then auto-clears at `expiry`. Pressing H (the
    /// objective pop-up) or starting a new trial clears it early. `None` = no card.
    pub(crate) satoshi_brief: Option<(String, String, Instant)>,
    /// #19 Rig Studio authoring state (selected skeleton + per-part block assignments).
    pub(crate) rig_studio: rig_studio_ui::RigStudioState,
    /// Workshop Phase 5 Task 6 — the block selected in the Wardrobe's left column.
    /// `None` = nothing selected (right pane is empty).
    pub(crate) wardrobe_block: Option<crate::block::BlockId>,
    /// Workshop Phase 5 Task 6 — in-progress rename edit buffer: `(block, design_id, name)`.
    /// `Some` while the rename text field is active for a specific design; committed or
    /// cancelled by the panel.
    pub(crate) wardrobe_rename: Option<(crate::block::BlockId, u32, String)>,
    /// Spec 40 persistence — the player wardrobe changed this session and needs a
    /// debounced save (mirrors the world autosave dirty cadence).
    pub(crate) wardrobe_dirty: bool,
    /// Debounce counter for the wardrobe save (ticks since last flush).
    pub(crate) wardrobe_save_counter: u32,
    /// Async load slot for the player-global wardrobe blob (WASM Stash). Outer
    /// Option = "a load completed"; inner Option = the bytes (None = nothing stored).
    /// Drained at frame top (wasm32-only drain in game_loop.rs, invisible to a
    /// native `cargo clippy` run).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) wardrobe_load_slot: std::rc::Rc<std::cell::RefCell<Option<Option<Vec<u8>>>>>,
    /// The player edited the wardrobe this session — a late async load must NOT
    /// clobber it (mirrors `skin_user_acted`).
    pub(crate) wardrobe_user_acted: bool,
    /// "Remember my designs on entry" preference (default true). When false a world
    /// is entered at standard (stock + official); the saved wardrobe is untouched.
    pub(crate) wardrobe_remember: bool,
    /// Cinematic Director (Phase 1) — a render-only camera rig detached from the
    /// avatar, on the primary viewport (slot 0). Transient; never serialized.
    /// Native-only (the web bundle is untouched).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) director: crate::director::DirectorCamera,
    /// Cinematic replay (Phase 2c) — `Some` while a single-player session is being
    /// recorded to a `.axereplay`. One frame is teed per sim tick from
    /// `network_send_input`. Transient; never serialized. Native-only.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) replay_recorder: Option<crate::replay::ReplayRecorder>,
    /// Cinematic replay (Phase 2c) — `Some` while a recorded `.axereplay` is being
    /// played back by the Director (render-only re-application of recorded state).
    /// Transient. Native-only.
    // BRIDGE: never read yet — no UI drives replay playback (see replay.rs's
    // list_replays/load_replay, also unwired). Set to None and never touched.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(dead_code)]
    pub(crate) replay_player: Option<crate::replay_player::ReplayPlayer>,
    /// Registered slash commands (built-ins + future game-specific).
    pub(crate) cmd_registry: commands::CommandRegistry,
    /// Current game mode (menu or playing)
    pub(crate) mode: GameMode,
    /// Ticks since last autosave (autosave every 5 minutes = 6000 ticks at 20 TPS)
    pub(crate) autosave_counter: u32,
    /// P8 — weather. The `tick_counter` value at which the current rain ends;
    /// it's raining while `tick_counter < weather_rain_until`. Ephemeral (not
    /// persisted) — a fresh session starts clear.
    pub(crate) weather_rain_until: u64,
    /// Wind, Copper & Electricity wave §4 — the `weather_lock` applied on the
    /// PREVIOUS tick, so the loop can tell a lock being held from one being
    /// released. A pinned window is re-stamped a full in-game minute ahead every
    /// tick; without this the residue would sit there for up to 60 s after the
    /// trial ended (`weather::apply_lock`). Ephemeral, like the window itself.
    pub(crate) weather_lock_applied: Option<String>,
    /// Wind, Copper & Electricity wave §2.3 — the sea-level wind sample for the
    /// current tick, taken once in the power step and read back by the render
    /// path (the Windmill hover label + the F3 read-out) so all three agree.
    /// Derived, never saved, never synced: `wind::sample(tick, weather, seed)`.
    pub(crate) wind: crate::wind::WindSample,
    /// Per-device graphics / quality settings (Spec 39). Single source of truth
    /// for render distance, render scale, FOV, sensitivity, frame limit, fog.
    /// Loaded from persistent storage at startup; default = High (today's look).
    pub(crate) graphics: crate::graphics_settings::GraphicsSettings,
    /// Source of truth for how the player interacts with the world (Spec 05 §8).
    /// `is_creative` below is a single-writer cached projection of this — see
    /// `set_play_mode`. Never write `is_creative` directly; always go through
    /// `set_play_mode` so the cache can't drift.
    pub play_mode: crate::play_mode::PlayMode,
    /// True when current world is in creative mode
    pub(crate) is_creative: bool,
    /// Per-world flag — when false, the T / `/` chat overlay is gated
    /// off and slash-commands are unavailable. Set from `WorldMeta.commands_enabled`
    /// on world load. Defaults to true (matches behaviour before the flag
    /// existed).
    pub(crate) is_commands_enabled: bool,
    /// Spec 49 (Explosives) — runtime cache of `WorldMeta.explosives_enabled`.
    /// When false, Blasting Keg detonations no-op (hand-lit AND electrical) and
    /// break no blocks. Set from meta on world load; defaults to true.
    pub(crate) explosives_enabled: bool,
    /// Runtime cache of `WorldMeta.fire_spread_enabled` (2026-07-04). When
    /// false, fire never consumes flammable blocks (still ignites + burns out).
    pub(crate) fire_spread_enabled: bool,
    /// Spec 24 Phase 5 — runtime cache of `WorldMeta.has_seen_license_onboarding`.
    /// Set on world load; flipped to `true` the first time any player
    /// in this world confirms a Plan capture, with the meta written
    /// back to disk in the same step.
    pub(crate) has_seen_license_onboarding: bool,
    /// Difficulty level from world metadata ("peaceful", "easy", "normal", "hard")
    pub(crate) difficulty: String,
    /// Gamepad input system — native uses gilrs, WASM uses the browser's
    /// `navigator.getGamepads()` polling API.
    pub(crate) gamepad: crate::gamepad::GamepadSystem,
    /// Per-player state (split-screen ready: Vec holds 1-4 PlayerSlots)
    pub(crate) players: Vec<crate::player_slot::PlayerSlot>,
    /// Screen layout (viewports mapped to players)
    pub(crate) screens: Vec<crate::screen::Screen>,
    /// If Player 1 is using a gamepad (controller-only / console mode),
    /// this holds the gamepad index. None = Player 1 is on keyboard+mouse.
    /// Set when the user presses Play: mouse click = None, gamepad A = Some(idx).
    pub(crate) p1_gamepad: Option<usize>,
    /// Toast notification message + expiry time.
    pub(crate) toast: Option<(String, Instant)>,
    /// UX polish sweep Task 2 (2026-07-07) — session-only first-encounter
    /// hint tracker. Deliberately a plain runtime field: NOT in `Save` or
    /// `WorldMeta`, never serialized, resets to empty on every process start.
    /// See `game_loop::Hint` / `GameState::hint_once`.
    pub(crate) hints_shown: std::collections::HashSet<crate::game_loop::Hint>,
    /// Phase 7 — first-person viewmodel swing window, per LOCAL player index.
    /// Counts down `SWING_TICKS..=0`; armed to `SWING_TICKS` on the rising edge
    /// of the player's own break/place intent so the held tool swings exactly
    /// like the remote-avatar arm (mirrors `remote_swing`, but for self).
    pub(crate) local_swing: std::collections::HashMap<usize, u32>,
    /// Phase 3 (cosmetics) — live sneak signal per LOCAL player index, captured
    /// each tick from the post-gate intent. Read by `local_player_state` to drive
    /// a split-screen peer's CROUCHING flag. `PlayerSlot.input` is vestigial in
    /// this engine (all real input flows through `self.input`/`self.gamepad`
    /// merged by `route_intents`), so the slot can't be read for sneak — this
    /// captured signal is the real one. Lifecycle mirrors `local_swing`.
    pub(crate) local_sneak: std::collections::HashMap<usize, bool>,
    /// Monotonic per-send sequence for the outbound InputPacket.tick. Strictly
    /// increasing across the whole session so the server's replay filter
    /// (`hosted_server.rs` `input.tick <= sp.last_input_tick`) never drops our
    /// input. Must NOT be world_time (which is cyclic 0..23999 and would stall
    /// input for ~20 min after each day-cycle wrap).
    pub(crate) net_send_seq: u64,
    /// Player 0's final (post-gate) intent this tick, stashed by `tick()` for
    /// `network_send_input` to read after the tick completes — so the intent the
    /// client SENDS matches the intent it SIMULATED locally this tick.
    /// Single-local-player-per-machine only (split-screen+LAN multi-local input
    /// fan-out is a separate follow-up).
    pub(crate) net_local_intent: crate::player_intent::PlayerIntent,
    /// Hosted server (when this machine is hosting a LAN game). Cross-platform:
    /// always `None` on wasm (browsers can't host), but the field's presence lets
    /// the shared network methods reference it without per-line cfg gating.
    pub(crate) hosted_server: Option<crate::hosted_server::HostedServer>,
    /// Remote client (when this machine has JOINED a server). Cross-platform now
    /// — the browser joins the dedicated WebSocket server through this too.
    pub(crate) remote_client: Option<crate::remote_client::RemoteClient>,
    /// Online play by contact — the host-side rendezvous, while hosting online.
    /// `Some` only between "Host online" succeeding and leaving the world.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) online_host: Option<crate::online_host::OnlineHost>,
    /// A join in progress, with the instant it started (the 8 s deadline is
    /// measured from there, not from a wall clock).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) online_join: Option<(crate::online_join::OnlineJoin, Instant)>,
    /// The socket/router/attestation preparation running on a worker thread.
    /// Nothing about playing online blocks a frame; this is where the waiting
    /// happens.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) online_prep: Option<crate::online_prep::PendingPrep>,
    /// The router mapping, renewed hourly and released when hosting stops.
    /// Every call into the router is a SOAP round-trip, so the keeper owns a
    /// worker thread and the tick only decides *whether* it is time.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) upnp_lease: Option<crate::online_prep::LeaseKeeper>,
    /// Whether `/online` has the in-game Online panel open.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) show_online_panel: bool,
    /// A world the online paths want entered on the next lobby frame — the
    /// worker finishes outside the menu's borrow, so the transition is staged
    /// here rather than driven from `poll_online`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pending_online_world: Option<String>,
    /// Set when the contacts mirror has been rewritten, so the lobby re-reads
    /// the book instead of showing a stale Friends column.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) contacts_dirty: bool,
    /// Text to put on the clipboard at the next egui pass. Copying needs the
    /// egui context, which the dispatch sites do not have.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pending_clipboard: Option<String>,
    /// Latest Operator Console snapshot received from a server we operate (Spec B
    /// task 7). Set only for the verified operator; drives the in-game Operator
    /// panel. Native — the console read-model + panel are native-only.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) operator_snapshot: Option<crate::console_snapshot::ConsoleSnapshot>,
    /// Native custom-skin upload (Phase 4) — the in-flight OS PNG picker.
    /// `Some` while an Open-PNG dialog is showing/reading on its worker thread;
    /// `update_and_render` drains its `FileDialogResult` each frame (decode →
    /// new wardrobe entry + equip) and clears it. Mirrors the WASM `skin_pick_slot`,
    /// and the double-open guard (only spawn when `None`) keeps it to one dialog
    /// at a time.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pending_skin_dialog:
        Option<std::sync::mpsc::Receiver<crate::native_file_dialog::FileDialogResult>>,
    /// Native skin EXPORT — the in-flight OS Save-PNG dialog. Separate from
    /// `pending_skin_dialog` (which is the upload/open picker) so a save result
    /// can't be mistaken for an open result. Drained in `update_and_render`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pending_skin_save_dialog:
        Option<std::sync::mpsc::Receiver<crate::native_file_dialog::FileDialogResult>>,
    /// Campaign G — the in-flight OS ghost export/import dialog (`.axeghost`).
    /// One slot for both directions: the result variants (`GhostSaved` vs
    /// `GhostJson`) disambiguate. Drained in `update_and_render`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pending_ghost_dialog:
        Option<std::sync::mpsc::Receiver<crate::native_file_dialog::FileDialogResult>>,
    /// Native Minecraft import — the in-flight worker (3 blocking Mojang GETs).
    /// `Some` while a username/UUID lookup is running on its worker thread;
    /// `update_and_render` drains the single `McImportOutcome` each frame and
    /// clears it. Mirrors the WASM `skin_mc_import_slot`. Native-only — the web
    /// path goes through the `/mc-skin` proxy fetch (`skin_mc_import_slot`).
    ///
    /// The tuple carries `(mc_import_token at kick, target, receiver)`: the
    /// `target` (None = new import, Some(id) = refresh that entry) travels WITH
    /// the request rather than in a shared field, so an interleaved import +
    /// refresh can never write a downloaded skin onto the wrong entry. The token
    /// is checked at drain — a stale (superseded/cancelled) result is discarded.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pending_mc_import: Option<(
        u32,
        Option<crate::skin_wardrobe::SkinId>,
        std::sync::mpsc::Receiver<crate::mc_import::McImportOutcome>,
    )>,
    /// Remote player positions received from the server (avatars to render).
    /// Cross-platform — the browser renders other players when joined.
    pub(crate) remote_players: Vec<crate::protocol::PlayerState>,
    /// Swing-window ticks remaining per remote player (player_index ->
    /// remaining). Drives the right-arm mine/place swing on remote avatars;
    /// set to SWING_TICKS on a rising SWINGING-flag edge, decremented toward 0.
    pub(crate) remote_swing: std::collections::HashMap<u32, u32>,
    /// Server-broadcast dropped items (death-drops phase 2b), keyed by wire
    /// ProtocolId. Render-only ghosts fed from StateUpdate entity diffs when
    /// joined to a server; pickup is server-authoritative (InventoryGrant).
    /// Cross-platform — the browser renders server loot when joined.
    pub(crate) remote_items: crate::remote_entities::RemoteItems,
    /// Block changes made this tick, to be sent to the server. Cross-platform so
    /// `network_send_input` can drain it on both targets. NOTE: the ~30 push
    /// sites stay native-gated for now, so a browser joiner does not yet
    /// propagate its OWN edits to the server (v1 limitation L-web-edit).
    pub(crate) pending_block_changes: Vec<crate::protocol::BlockChange>,
    /// Touch input for WASM (virtual joystick + action buttons).
    #[cfg(target_arch = "wasm32")]
    pub(crate) touch: crate::touch_input::TouchInput,
    /// One-shot guard for the dedicated-server auto-join. When the Docker-served
    /// page sets `window.AXENSTAX_DEDICATED_WS`, the menu auto-joins that server
    /// exactly once instead of waiting for a manual click.
    #[cfg(target_arch = "wasm32")]
    pub(crate) dedicated_autojoin_done: bool,
    /// Per-frame render durations (seconds). Rolling ~5-second window — fed
    /// by `update_and_render`, consumed by the #44 debug-HUD FPS line (native +
    /// WASM).
    pub(crate) frame_samples: std::collections::VecDeque<f32>,
    /// Last observed frame start timestamp, used to compute the next frame
    /// delta. None on the first frame.
    pub(crate) last_frame_instant: Option<Instant>,
    /// Last rendered menu/pause frame — throttles the lightweight lobby screens
    /// to ~20 fps so an idle lobby doesn't spin the GPU at full display rate.
    pub(crate) last_menu_frame: Option<Instant>,
    /// Per-tick simulation durations (seconds). Rolling ~5-second window at
    /// 20 TPS — fed by `tick`, consumed by the debug-HUD perf line.
    pub(crate) tick_samples: std::collections::VecDeque<f32>,
    /// Deepslate Reserve snapshot (Spec 16). Defaults to
    /// `synthetic_default` and updates from incoming `StateUpdatePacket`
    /// frames so the client always has a current value for HUD + future
    /// chunk-gen variant selection. Set on every network state-update tick.
    pub(crate) reserve: crate::reserve::ReserveState,
    /// Fund-the-Reserve dialog state (Spec 16 Phase 5). `Some` while the
    /// dialog is open; `None` when closed. Opened by clicking the
    /// "Fund the Reserve" button in the F3 gauge.
    pub(crate) fund_dialog: Option<crate::hud_ui::FundDialogState>,
    /// Server-wide sats-flow policy (Spec 19 follow-on). Single instance on
    /// alpha; per-server config when multi-server lands. All sats payouts
    /// route through `economy::apply_sats_payout` with this policy +
    /// per-player Charter flag. See `docs/vision/sat-flow-and-economy-loops.md`.
    pub(crate) sats_policy: crate::economy::ServerSatsPolicy,
    /// Spawn-location preference for the next world load. Set by the menu
    /// when the player picks Load/Create; applied AFTER
    /// `chunk_stream::initial_load` runs so the override sees the saved
    /// player position as its reference. Cleared back to `Default` once
    /// applied so a later in-game reload doesn't re-teleport.
    pub(crate) pending_spawn_pref: crate::spawn_pref::SpawnPref,
    /// One-shot: the next `begin_load` is a RESUME whose world + players were
    /// already restored (the WASM poll branch unpacks the IndexedDB blob and
    /// restores everything before entering `GameMode::Loading`). Gates
    /// `begin_load`'s FRESH-world setup — without it the WASM resume fell into
    /// FRESH (the sync `save::load_world` is a stub Err on wasm32) and had its
    /// restored position reset + Workshop/Test-Lab kit re-dumped on every PWA
    /// reload. `take()`-consumed by `begin_load`; never set on native (the
    /// sync load succeeds there, so the SAVE branch runs regardless).
    pub(crate) world_preloaded: bool,
    /// Spec 40 (The Workshop) — one-shot: set when the Lobby "Reset Workshop"
    /// button is confirmed. After the Workshop world finishes loading, the room
    /// is wiped back to the void floor (blocks + in-progress projects) while the
    /// in-memory reskin/reshape catalogue is KEPT (it is not in world.dat), then an
    /// autosave persists the cleared room. Cleared once applied.
    pub(crate) pending_workshop_reset: bool,
    /// Stash Column (Prague) — one-shot: armed when "Play" is hit on a scenario
    /// card. The PlayScenario handler creates + enters a fresh per-def arena
    /// world; this carries the def so it's installed (provision + arm the runner)
    /// on the FIRST play tick, once the arena world is fully loaded + players
    /// spawned (same hook as `pending_workshop_reset`). `take()`-consumed once.
    pub(crate) pending_scenario_launch: Option<crate::scenario::PendingScenarioLaunch>,
    /// Stash Column (Prague) — one-shot: the bytes of a skin override-set adopted
    /// from the lobby (the override registry lives on a World, so it can't apply
    /// in the menu). Applied to the next world the player enters on the first play
    /// tick, then the textures rebuild + chunks re-mesh. `take()`-consumed once.
    pub(crate) pending_skin_adopt: Option<Vec<u8>>,
    /// F1: the live "my look" model. The local player's avatar (and first-person
    /// hand) is rendered from THIS — apply = write_avatar_skin(skin_rgba()).
    /// Upload sets Rgba64; Reset sets default(). Cross-platform so the descriptor
    /// + apply path stay identical on native and WASM.
    pub(crate) local_cosmetic: crate::cosmetics::CosmeticDescriptor,
    /// Phase 1c: the player's avatar-skin wardrobe (many named skins). The
    /// equipped entry derives `local_cosmetic` via `active_descriptor()`, so the
    /// renderer + wire stay unchanged. Per-identity, like cosmetics (no WorldSave).
    pub(crate) skin_wardrobe: crate::skin_wardrobe::SkinWardrobe,
    /// True once the wardrobe has been loaded for the current session/world
    /// (native: disk; web: async). Prevents re-loading over session edits.
    pub(crate) wardrobe_loaded: bool,
    /// UI state for the "Your look" panel (preview thumbnail + status). Holds
    /// presentation only; the applied pixels live in `local_cosmetic`.
    pub(crate) skin_panel: crate::menu::SkinPanelState,
    /// In-world avatar paint session — `Some` only while a Workshop mannequin is
    /// blown up + being painted (Skin painting in the Workshop). Transient; never
    /// serialized.
    pub(crate) skin_paint: Option<SkinPaintSession>,
    /// "Your look → Edit/New" hand-off: the wardrobe entry + its seeded 64×64×4
    /// RGBA the Workshop painter should open on. Consumed on the first Workshop
    /// frame (auto-inflates the mannequin locked on this entry). Transient.
    pub(crate) pending_skin_paint: Option<(crate::skin_wardrobe::SkinId, Vec<u8>)>,
    /// One-shot: the pause-overlay "Your look → Edit/New" routed us to the menu to
    /// re-enter the Workshop. The menu handler fires `EnterWorkshop` once, then
    /// clears this, reusing the world-load machinery for both native + wasm.
    pub(crate) pending_workshop_for_skin: bool,
    /// The kind of world that is live right now: set at the Loading → Playing
    /// hand-off, taken by `leave_world`. `None` = no world (lobby, splash,
    /// mid-load, after a discard) — which is what stops the close button
    /// saving a phantom or discarded world. See `world_exit`.
    pub(crate) live_world: Option<crate::world_exit::WorldKind>,
    /// One-shot menu action fired on the lobby's next frame — an in-world
    /// arena launch (J board / `/scenario`) leaves the world and then runs the
    /// Trials menu's own `PlayScenario` path through this. Stamped with when
    /// it was queued, so a web lobby whose list never loads gives up on it
    /// (`world_exit::queued_launch_step`).
    pub(crate) pending_menu_action: Option<(crate::menu::MenuAction, web_time::Instant)>,
    /// Host-side `/we` cells still to broadcast, drained into successive
    /// StateUpdates (`worldedit::REGION_BROADCAST_BATCH` per tick).
    pub(crate) region_broadcast_queue: crate::worldedit::RegionBroadcastQueue,
    /// True for each frame the Workshop avatar-paint left-click is held — so a
    /// stroke snapshots undo exactly once on its rising edge.
    pub(crate) workshop_paint_stroke_active: bool,
    /// Export help panel ("how to wear this in Minecraft") visibility + whether
    /// the exported skin was unsaved (adds the "save first" tip). Cross-platform.
    pub(crate) export_help_open: bool,
    pub(crate) export_help_unsaved: bool,
    /// Arm model of the skin that was just exported, so step 3 of the help can
    /// name the ONE button to press on minecraft.net instead of offering both.
    pub(crate) export_help_arm: crate::skin_uv::ArmModel,
    /// UI state for the Graphics settings panel (Spec 39). Open/closed only;
    /// the dial values live in `graphics`.
    pub(crate) settings_panel: crate::menu::SettingsPanelState,
    /// The lobby's "Relays" window (opened from the lobby header or the
    /// sign-in dialog). The list itself lives in `graphics.online_relays`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) lobby_relays_ui: crate::relays_ui::RelaysUiState,
    /// Async hand-off slot for the skin file picker. The picker future writes
    /// its result here, tagged with the `skin_pick_token` generation it was
    /// started under; the main loop drains it each frame and DISCARDS a stale
    /// result (one whose token no longer matches), so a late-resolving pick
    /// can't clobber a Reset/Back the player did while it was in flight (WASM
    /// only — upload + Stash are a PWA feature, mirroring cloud-save).
    #[cfg(target_arch = "wasm32")]
    pub(crate) skin_pick_slot:
        std::rc::Rc<std::cell::RefCell<Option<(u32, Result<Vec<u8>, String>)>>>,
    /// Campaign G — the in-flight browser `.axeghost` pick (WASM). No token:
    /// the only consumer is the rival store, and a late import landing is
    /// harmless (it just sets the rival). Drained in `update_and_render`.
    #[cfg(target_arch = "wasm32")]
    pub(crate) ghost_pick_slot:
        std::rc::Rc<std::cell::RefCell<Option<Result<Vec<u8>, String>>>>,
    /// Generation counter for the in-flight skin pick (FIX 1). Bumped when a
    /// pick is started AND whenever the player does something else (Reset/Back)
    /// — so any pick still resolving against an old token is ignored on drain.
    #[cfg(target_arch = "wasm32")]
    pub(crate) skin_pick_token: u32,
    /// Async hand-off for the on-enter skin-wardrobe load (WASM). Outer Option =
    /// "a load completed this frame"; inner = the payload (blob, or a legacy
    /// single-skin PNG to migrate), or None if nothing stored.
    #[cfg(target_arch = "wasm32")]
    pub(crate) skin_wb_load_slot:
        std::rc::Rc<std::cell::RefCell<Option<Option<crate::game_loop::SkinWbPayload>>>>,
    /// Async hand-off for a Minecraft skin import (WASM). The fetch task writes
    /// `(mc_import_token at kick, target, outcome)` here; the frame-top drain
    /// consumes it. `target` (None = new import, Some(id) = refresh of that
    /// entry) travels WITH the result so a late import can't land on the wrong
    /// entry; the token is checked so a stale (superseded/cancelled) result is
    /// dropped. Mirrors the native `pending_mc_import` tuple.
    #[cfg(target_arch = "wasm32")]
    pub(crate) skin_mc_import_slot: std::rc::Rc<
        std::cell::RefCell<
            Option<(
                u32,
                Option<crate::skin_wardrobe::SkinId>,
                crate::mc_import::McImportOutcome,
            )>,
        >,
    >,
    /// Generation counter for the in-flight Minecraft import/refresh. Bumped on
    /// EVERY kick, and whenever an in-flight request is invalidated (Cancel /
    /// panel close). The kick-time value rides inside the async payload; at drain
    /// a result whose token != this is DISCARDED, so it can't overwrite (and
    /// persist) the wrong wardrobe entry. Cross-platform (native + web share the
    /// discard rule). Mirrors the `skin_pick_token` pattern.
    ///
    /// An `Rc<Cell<u32>>` rather than a plain `u32` so the WASM proxy-fetch
    /// future can hold a clone (the "live token") and read it back when it
    /// resolves: a superseded fetch (its captured kick-token no longer equals
    /// the live value) becomes a NO-OP write instead of clobbering a newer
    /// kick's result. This mirrors native's drop-on-supersede (where Cancel /
    /// reopen drops the worker `Receiver`), closing the web-only race where a
    /// slow fetch overwrote a fast one and hung the spinner. Native only ever
    /// reads `.get()` (the worker thread carries a copied `u32` in its tuple).
    pub(crate) mc_import_token: std::rc::Rc<std::cell::Cell<u32>>,
    /// Exhibits (Creator Gallery) — async hand-off for decoded exhibit images,
    /// shape `(tex id, width, height, RGBA)`. Native
    /// uploads directly (stays empty); WASM fetch tasks push here, drained each
    /// frame into `Renderer::upload_painting_image`.
    pub(crate) exhibit_art_slot:
        std::rc::Rc<std::cell::RefCell<Vec<(u64, u32, u32, Vec<u8>)>>>,
    /// True once the current world's exhibit quads have been built + images
    /// requested, so the loader fires exactly once per world visit. Cleared when a
    /// world with no exhibits loads.
    pub(crate) exhibit_art_requested: bool,
    /// Exhibit images that travelled inside the loaded `.axeworld` archive
    /// (#127). The WASM load path captures `unpack_world`'s images here; WASM
    /// `request_exhibit_art` decodes them in preference to the same-origin
    /// `/exhibits/<ref>` fetch, so a file-imported / locally-saved gallery world
    /// renders its art without a server route. Native uses the on-disk
    /// `exhibits/` folder, so this is WASM-only.
    #[cfg(target_arch = "wasm32")]
    pub(crate) pending_exhibit_images: Vec<crate::world_archive::ExhibitImage>,
    /// Real decoded image aspect (pixel width / height) per exhibit texture id,
    /// learned as each image decodes. `build_exhibit_quads` consults it so art is
    /// hung at the image's TRUE shape (fit within the artist's size box), never
    /// stretched to a hand-typed width×height. Empty until the first decode;
    /// cleared on world change. Native fills it synchronously; WASM as fetches land.
    pub(crate) exhibit_tex_aspect: std::collections::HashMap<u64, f32>,
    /// Spec 40 Phase D3 — async Beacon browse/following/adopt results, drained
    /// each frame in update_and_render (PWA-only), mirroring the skin slots.
    /// Browse: numbered redesign rows for `world.beacon_browse_cache`.
    #[cfg(target_arch = "wasm32")]
    pub(crate) beacon_browse_slot:
        std::rc::Rc<std::cell::RefCell<Option<Result<Vec<crate::world::BrowseEntry>, String>>>>,
    /// Spec 40 Phase D3 — async result of `/ws following` (npub strings, display-ready).
    #[cfg(target_arch = "wasm32")]
    pub(crate) beacon_following_slot:
        std::rc::Rc<std::cell::RefCell<Option<Result<Vec<String>, String>>>>,
    /// Spec 40 Phase D3 — async result of `/ws adopt <n>` download (raw blob bytes).
    #[cfg(target_arch = "wasm32")]
    pub(crate) beacon_adopt_slot:
        std::rc::Rc<std::cell::RefCell<Option<Result<Vec<u8>, String>>>>,
    /// True once the player has uploaded or reset their skin this session, so
    /// a slow in-flight Stash load (kicked off at world entry) can't land late
    /// and silently revert their choice. Reset per page-load (a fresh session
    /// re-reads the saved skin).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) skin_user_acted: bool,
    /// Phase 3 (cosmetics) — true once the per-player tint variants have been
    /// seeded into skin-array layers 1.. (layer 0 is the local/panel player's
    /// own skin, owned by Phase 2). One-time guard so split-screen players each
    /// wear a distinct tinted-default skin (player 1 → layer 1, player 2 →
    /// layer 2, …). Native-only: split-screen doesn't exist on WASM, where
    /// layers 1.. are never sampled.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) skin_tints_seeded: bool,

    /// Active challenge scenario (Goal 1 scenario runner). `Some` ⇒ a scenario
    /// is running; drives the timer/score HUD + end screen, gates gameplay
    /// input when ended, and is cleared on world change. NOT a `GameMode`
    /// variant by design (avoids exhaustive-match surgery). See `scenario.rs`.
    pub(crate) scenario: Option<crate::scenario::ScenarioState>,
    /// Kiosk / showcase config (Spec 2026-06-19 §8 Phase 2). Disabled in normal
    /// play; on the web kiosk it is read from `ws_transport_web::showcase_config`.
    pub(crate) showcase: crate::showcase::ShowcaseConfig,
    /// The visiting guest's personal basket — EPHEMERAL (never saved).
    pub(crate) basket: crate::showcase::Basket,
    /// True once the visitor has exited inside a showcase: the terminal exit
    /// screen is up and there is no path back to the lobby (Spec §10). A flag, not
    /// a `GameMode` variant (avoids exhaustive-match surgery; same as `scenario`).
    pub(crate) showcase_exited: bool,
    /// One-shot: a kiosk left-click this frame requests a collect, consumed in the
    /// per-player raycast block (which has the eye + look ray). Cleared when taken.
    pub(crate) collect_requested: bool,
}

impl GameState {
    /// The ONLY sanctioned way to change the world's play mode. Keeps the
    /// `is_creative` projection in lock-step so the ~130 read sites stay valid.
    pub fn set_play_mode(&mut self, mode: crate::play_mode::PlayMode) {
        self.play_mode = mode;
        self.is_creative = mode.is_creative();
    }

    #[cfg(target_arch = "wasm32")]
    fn set_status(msg: &str) {
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("status"))
        {
            el.set_text_content(Some(msg));
        }
    }

    async fn new(window: Arc<Window>) -> Self {
        Self::new_inner(Some(window), (0, 0)).await
    }

    /// Headless GameState harness (2026-07-11) — the full game (world, chunks,
    /// ticks, input dispatch, ECS, egui UI logic) with no display server. The
    /// renderer is `Renderer::new_headless` (real GPU preferred; needs an
    /// adapter that can hold the 506-layer block atlas), the window is
    /// `GameWindow::Headless`, and every surface paint is skipped. Used by
    /// `test_game_harness.rs`; native-only. (Test-gated with it — widen the
    /// cfg when a non-test flow, e.g. screenshot tooling, adopts it.)
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) async fn new_headless(width: u32, height: u32) -> Self {
        Self::new_inner(None, (width, height)).await
    }

    async fn new_inner(window: Option<Arc<Window>>, headless_size: (u32, u32)) -> Self {
        let size = match &window {
            Some(w) => w.inner_size(),
            None => winit::dpi::PhysicalSize::new(headless_size.0, headless_size.1),
        };
        // Clamp to 1 on both axes — on WASM, winit's initial inner_size() can be
        // 0x0 before the ResizeObserver fires, which would make aspect = 0.0
        // and produce an infinite view_proj[0][0]. The actual aspect is set
        // on the first resize event and in the post-init resync in resumed().
        let aspect = (size.width.max(1) as f32) / (size.height.max(1) as f32);

        #[cfg(target_arch = "wasm32")]
        Self::set_status("Creating world...");

        let registry = BlockRegistry::new();
        let mut world = World::new();
        // Spec 27 Phase 5 — engine-bundled curated plans for village
        // procgen. Empty registry = village_gen falls back to the
        // existing hardcoded shape (gameplay-neutral).
        world.load_bundled_plans();
        // Owner-inbox #18 — register built-in micro-model overrides (flowers etc.)
        // so registered blocks render their baked sub-voxel shell. Empty until a
        // block is bound (Phase C); the renderer uploads the per-type geometry via
        // `sync_micro_models` on first chunk load.
        world.load_bundled_micro_models(&registry);
        let biome_gen = crate::biome::BiomeGenerator::new(42);

        #[cfg(target_arch = "wasm32")]
        Self::set_status("Generating textures...");

        // Activate the persisted texture-pack selection before the atlas is built,
        // so the saved pack is in effect on the first frame (P3c).
        #[cfg(not(target_arch = "wasm32"))]
        texture_registry::init_active_pack_from_disk();
        // Procedural set + any active disk texture pack's named overrides (P2).
        let textures = texture_registry::base_textures();

        #[cfg(target_arch = "wasm32")]
        Self::set_status("Initializing GPU renderer...");

        let renderer = match &window {
            Some(w) => Renderer::new(w.clone(), &textures).await,
            None => Renderer::new_headless(size.width, size.height, &textures).await,
        };

        #[cfg(target_arch = "wasm32")]
        Self::set_status("Starting game...");

        let input = InputState::new();
        let audio = crate::audio::AudioEngine::new();

        // Create a single PlayerSlot for player 0
        let spawn = glam::Vec3::new(0.5, 80.0, 0.5);
        let mut player0 = crate::player_slot::PlayerSlot::new(0, spawn, aspect);
        // Debug shortcut: pre-open the crafting UI so headless inspection has
        // something to render. Paired with `AXENSTAX_AUTO_INVENTORY=1` →
        // `GameMode::Playing` below. Used by the dev-side screenshot capture
        // path when no input-injection tool (xdotool) is available.
        if std::env::var("AXENSTAX_AUTO_INVENTORY").ok().as_deref() == Some("1") {
            player0.crafting_ui.open_player_crafting();
            log::info!("AXENSTAX_AUTO_INVENTORY=1 — booting into Playing with crafting UI open");
        }
        let screens = crate::screen::compute_screen_layout(1, renderer.width, renderer.height);

        Self {
            world_name: "default".to_string(),
            renderer,
            window: match window {
                Some(w) => crate::game_window::GameWindow::Real(w),
                None => crate::game_window::GameWindow::Headless {
                    width: size.width,
                    height: size.height,
                },
            },
            world,
            registry,
            // Capture the seed before moving biome_gen into the struct so we
            // can derive the Proof-of-Play secret below.
            // Placeholder until a world loads (`apply_world_seed` installs the
            // world's own secret); random, never seed-derived.
            pop_server_secret: crate::proof_of_play::gen_world_secret(),
            biome_gen,
            input,
            audio,
            last_tick: Instant::now(),
            tick_accumulator: Duration::ZERO,
            frame_pace_instant: Instant::now(),
            show_debug: false,
            minimap: crate::minimap::MinimapView::default(),
            map_screen: crate::minimap::MapScreen::default(),
            spawn_overlay: false,
            skin_picker_open: false,
            pending_skin_undo: false,
            pending_skin_redo: false,
            pending_skin_pin: false,
            build_guide: None,
            narrator: crate::narration::Narrator::default(),
            loaded_columns: ahash::AHashSet::new(),
            mission_idx: 0,
            mission_note: String::new(),
            mission_registry: Vec::new(),
            active_trial: None,
            trial_outcome: None,
            trial_bests: crate::trials::TrialBests::load(),
            load_queue: std::collections::VecDeque::new(),
            dirty_mesh_chunks: ahash::AHashSet::new(),
            falling_tick_counter: 0,
            water: WaterSystem::new(),
            lava: crate::lava::LavaSystem::new(),
            fire: crate::fire::FireSystem::new(),
            particles: crate::particles::ParticleSystem::new(),
            particle_instances: Vec::new(),
            last_particle_frame: Instant::now(),
            particle_ambient_accum: 0.0,
            weather_storm_until: 0,
            lightning_flash: 0.0,
            entity_was_in_water: ahash::AHashMap::new(),
            leaf_decay: LeafDecaySystem::new(),
            ecs: hecs::World::new(),
            // Spawn into the morning. `compute_sun` mapping: 0=midnight,
            // 6000=sunrise (still dim), 12000=noon. 7500 sits ~3/4 of the
            // way from dawn to noon — sun fully risen, brightness ≈ 0.75,
            // a clear "morning" feel without being noon-bright.
            // `reset_for_world_change` mirrors this on every menu → Playing
            // transition so loaded + new worlds both come up at morning.
            world_time: 7500,
            // 20-minute real-world day: 24000 ticks / (1 step * 20 TPS) = 1200 s = 20 min.
            // The previous `step: 4` (5-min day) was the alpha-fast BRIDGE
            // documented in CLAUDE.md; rolled back per playtest feedback that
            // "time is moving too fast." Runtime override remains available
            // via `/time speed <n>` for testing.
            world_time_step: 1,
            tick_counter: 0,
            pop_epoch_id: 0,
            pop_exposure_map: ahash::AHashMap::new(),
            chat: chat_ui::ChatState::new(),
            challenge_board_open: false,
            show_objective: false,
            satoshi_brief: None,
            rig_studio_open: false,
            rig_studio: rig_studio_ui::RigStudioState::default(),
            workshop_painter: workshop_painter::WorkshopPainterUi::new(),
            workshop_play: false,
            wardrobe_open: false,
            wardrobe_block: None,
            wardrobe_rename: None,
            wardrobe_dirty: false,
            wardrobe_save_counter: 0,
            wardrobe_load_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
            wardrobe_user_acted: false,
            wardrobe_remember: true,
            #[cfg(not(target_arch = "wasm32"))]
            director: crate::director::DirectorCamera::default(),
            #[cfg(not(target_arch = "wasm32"))]
            replay_recorder: None,
            #[cfg(not(target_arch = "wasm32"))]
            replay_player: None,
            cmd_registry: {
                let mut r = commands::CommandRegistry::new();
                commands::builtins::register_all(&mut r);
                // Alpha-tester gate: /bug, /idea, /mailbox stay hidden unless
                // the player unlocked them in Settings (native only).
                #[cfg(not(target_arch = "wasm32"))]
                crate::native_mailbox::apply_gate(
                    &mut r,
                    &crate::graphics_settings::GraphicsSettings::load(),
                );
                r
            },
            autosave_counter: 0,
            weather_rain_until: 0,
            weather_lock_applied: None,
            wind: crate::wind::WindSample::CALM,
            graphics: crate::graphics_settings::GraphicsSettings::load(),
            play_mode: crate::play_mode::PlayMode::Survival,
            is_creative: false,
            is_commands_enabled: true,
            explosives_enabled: true,
            fire_spread_enabled: true,
            has_seen_license_onboarding: false,
            difficulty: "normal".to_string(),
            gamepad: crate::gamepad::GamepadSystem::new(),
            // Debug shortcut: AXENSTAX_AUTO_INVENTORY=1 boots straight into
            // Playing mode so headless inspection can capture the crafting UI
            // without driving the menu. Used to debug interaction-only bugs
            // when no xdotool / input-injection tool is available.
            mode: if std::env::var("AXENSTAX_AUTO_INVENTORY").ok().as_deref() == Some("1") {
                GameMode::Playing
            } else {
                // Web: the branded HTML bundle-loader (`index.html` #loading — voxel
                // hero + rotating cards) already covers the launch brand moment while
                // the WASM downloads, so the in-engine cosmetic splash would be a
                // SECOND, plainer brand screen back-to-back — what reads as "both the
                // new and the old loading screen". Skip straight to the lobby on web.
                // Native has no HTML loader, so it keeps the splash as its launch beat.
                #[cfg(target_arch = "wasm32")]
                {
                    GameMode::Menu(Box::new(crate::menu::MenuState::new()))
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    GameMode::Splash(crate::splash_ui::SplashState::new())
                }
            },
            players: vec![player0],
            screens,
            p1_gamepad: None,
            toast: None,
            hints_shown: std::collections::HashSet::new(),
            local_swing: std::collections::HashMap::new(),
            local_sneak: std::collections::HashMap::new(),
            net_send_seq: 0,
            net_local_intent: crate::player_intent::PlayerIntent::default(),
            hosted_server: None,
            remote_client: None,
            #[cfg(not(target_arch = "wasm32"))]
            online_host: None,
            #[cfg(not(target_arch = "wasm32"))]
            online_join: None,
            #[cfg(not(target_arch = "wasm32"))]
            online_prep: None,
            #[cfg(not(target_arch = "wasm32"))]
            upnp_lease: None,
            #[cfg(not(target_arch = "wasm32"))]
            show_online_panel: false,
            #[cfg(not(target_arch = "wasm32"))]
            pending_online_world: None,
            #[cfg(not(target_arch = "wasm32"))]
            contacts_dirty: false,
            #[cfg(not(target_arch = "wasm32"))]
            pending_clipboard: None,
            #[cfg(not(target_arch = "wasm32"))]
            operator_snapshot: None,
            #[cfg(not(target_arch = "wasm32"))]
            pending_skin_dialog: None,
            #[cfg(not(target_arch = "wasm32"))]
            pending_skin_save_dialog: None,
            #[cfg(not(target_arch = "wasm32"))]
            pending_ghost_dialog: None,
            #[cfg(not(target_arch = "wasm32"))]
            pending_mc_import: None,
            remote_players: Vec::new(),
            remote_swing: std::collections::HashMap::new(),
            remote_items: crate::remote_entities::RemoteItems::default(),
            pending_block_changes: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            touch: crate::touch_input::TouchInput::new(),
            #[cfg(target_arch = "wasm32")]
            dedicated_autojoin_done: false,
            frame_samples: std::collections::VecDeque::with_capacity(300),
            last_frame_instant: None,
            last_menu_frame: None,
            tick_samples: std::collections::VecDeque::with_capacity(100),
            reserve: crate::reserve::ReserveState::synthetic_default(),
            fund_dialog: None,
            sats_policy: crate::economy::ServerSatsPolicy::default(),
            pending_spawn_pref: crate::spawn_pref::SpawnPref::Default,
            world_preloaded: false,
            pending_workshop_reset: false,
            pending_scenario_launch: None,
            live_world: None,
            pending_menu_action: None,
            region_broadcast_queue: Default::default(),
            pending_skin_adopt: None,
            local_cosmetic: crate::cosmetics::CosmeticDescriptor::default(),
            skin_wardrobe: crate::skin_wardrobe::SkinWardrobe::new(),
            wardrobe_loaded: false,
            skin_panel: crate::menu::SkinPanelState::default(),
            skin_paint: None,
            pending_skin_paint: None,
            pending_workshop_for_skin: false,
            workshop_paint_stroke_active: false,
            export_help_open: false,
            export_help_unsaved: false,
            export_help_arm: crate::skin_uv::ArmModel::Classic,
            settings_panel: crate::menu::SettingsPanelState::default(),
            #[cfg(not(target_arch = "wasm32"))]
            lobby_relays_ui: crate::relays_ui::RelaysUiState::default(),
            #[cfg(target_arch = "wasm32")]
            skin_pick_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(target_arch = "wasm32")]
            ghost_pick_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(target_arch = "wasm32")]
            skin_pick_token: 0,
            #[cfg(target_arch = "wasm32")]
            skin_wb_load_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(target_arch = "wasm32")]
            skin_mc_import_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
            mc_import_token: std::rc::Rc::new(std::cell::Cell::new(0)),
            exhibit_art_slot: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            exhibit_art_requested: false,
            #[cfg(target_arch = "wasm32")]
            pending_exhibit_images: Vec::new(),
            exhibit_tex_aspect: std::collections::HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            beacon_browse_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(target_arch = "wasm32")]
            beacon_following_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(target_arch = "wasm32")]
            beacon_adopt_slot: std::rc::Rc::new(std::cell::RefCell::new(None)),
            skin_user_acted: false,
            #[cfg(not(target_arch = "wasm32"))]
            skin_tints_seeded: false,
            scenario: None,
            showcase: crate::showcase::ShowcaseConfig::default(),
            basket: crate::showcase::Basket::new(),
            showcase_exited: false,
            collect_requested: false,
        }
    }

    fn capture_cursor(&mut self) {
        let window = &self.window;
        let _ = window
            .set_cursor_grab(CursorGrabMode::Confined)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Locked));
        window.set_cursor_visible(false);
        self.input.cursor_captured = true;
    }

    fn release_cursor(&mut self) {
        let _ = self.window.set_cursor_grab(CursorGrabMode::None);
        self.window.set_cursor_visible(true);
        self.input.cursor_captured = false;
    }

    // Call sites (e.g. the resize handler) currently inline
    // `self.window.inner_size().width.max(1)` themselves rather than calling
    // these — a reuse opportunity, not touched here (out of scope for a
    // clippy pass).
    #[allow(dead_code)]
    fn width(&self) -> u32 {
        self.window.inner_size().width.max(1)
    }

    #[allow(dead_code)]
    fn height(&self) -> u32 {
        self.window.inner_size().height.max(1)
    }

    /// Surface a toast when a tool either just broke or just dropped below
    /// 10 % durability. No-op when the tool didn't cross the threshold this
    /// use. Called from every site that invokes `use_hotbar_tool` so the
    /// kid is never surprised by a snapped pickaxe.
    pub(crate) fn handle_tool_use(&mut self, result: Option<crate::inventory::ToolUseInfo>) {
        const LOW_THRESHOLD: f32 = 0.10;
        let Some(info) = result else { return; };
        if info.just_broke {
            self.toast = Some((
                format!("Your {} broke!", info.display_name),
                Instant::now() + Duration::from_secs(3),
            ));
            return;
        }
        if info.before_pct > LOW_THRESHOLD && info.after_pct <= LOW_THRESHOLD {
            self.toast = Some((
                format!("Your {} is almost broken — repair or replace soon.", info.display_name),
                Instant::now() + Duration::from_secs(3),
            ));
        }
    }
}

struct App {
    state: Option<GameState>,
    #[cfg(target_arch = "wasm32")]
    pending_state: std::rc::Rc<std::cell::RefCell<Option<GameState>>>,
}

impl App {
    fn new() -> Self {
        Self {
            state: None,
            #[cfg(target_arch = "wasm32")]
            pending_state: std::rc::Rc::new(std::cell::RefCell::new(None)),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let window_attrs = Window::default_attributes()
            .with_title("Axe'n'Stax")
            .with_inner_size(PhysicalSize::new(1280, 720));

        // Native launches in BORDERLESS FULLSCREEN on the current monitor (no
        // resolution switch — multi-monitor safe). The 1280×720 above is the
        // windowed fallback size when the player toggles out with F11. WASM keeps
        // its canvas sizing path (browser owns fullscreen).
        #[cfg(not(target_arch = "wasm32"))]
        let window_attrs =
            window_attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));

        let window = Arc::new(event_loop.create_window(window_attrs).unwrap());

        #[cfg(not(target_arch = "wasm32"))]
        {
            let state = pollster::block_on(GameState::new(window));
            self.state = Some(state);
        }

        #[cfg(target_arch = "wasm32")]
        {
            // Insert the winit canvas into the DOM
            use winit::platform::web::WindowExtWebSys;
            let canvas = window.canvas().expect("winit should create a canvas on WASM");
            canvas.style().set_css_text("width: 100%; height: 100%;");
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.body())
                .expect("document body")
                .append_child(&canvas)
                .expect("append canvas");

            // Size the canvas backing store to the browser viewport. CSS width:100% alone
            // leaves the canvas's width/height DOM attributes at 1x1 (winit's default),
            // so wgpu renders into a 1-pixel surface that's stretched invisibly by CSS.
            //
            // The backing store MUST be in *physical* pixels (CSS px × devicePixelRatio).
            // winit reports inner_size()/scale_factor() in physical px, the wgpu surface is
            // configured from inner_size(), and egui's pixels_per_point is the DPR — so a
            // logical-px backing store (DPR ignored) leaves the surface/viewport at 1/DPR
            // scale and the scene renders into the top-left corner. On desktop (DPR=1)
            // logical==physical so it's invisible; on mobile (DPR=2+) it's a fraction of
            // the screen (quarter at DPR=2). Always multiply CSS px by the DPR.
            let web_window = web_sys::window().expect("web window");
            let dpr = web_window.device_pixel_ratio().max(1.0);
            let vw = web_window.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1280.0);
            let vh = web_window.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(720.0);
            let pw = ((vw * dpr).round() as u32).max(1);
            let ph = ((vh * dpr).round() as u32).max(1);
            canvas.set_width(pw);
            canvas.set_height(ph);
            let _ = window.request_inner_size(PhysicalSize::new(pw, ph));
            log::info!("WASM: initial canvas sized to {}x{} physical (dpr {})", pw, ph, dpr);

            // Re-size the canvas on browser resize so the surface tracks the viewport.
            {
                use wasm_bindgen::JsCast;
                let window_for_resize = window.clone();
                let canvas_for_resize = canvas.clone();
                let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                    let w = web_sys::window().expect("web window");
                    // Physical px = CSS px × devicePixelRatio (see initial-sizing note above).
                    let dpr = w.device_pixel_ratio().max(1.0);
                    let vw = w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1280.0);
                    let vh = w.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(720.0);
                    let pw = ((vw * dpr).round() as u32).max(1);
                    let ph = ((vh * dpr).round() as u32).max(1);
                    canvas_for_resize.set_width(pw);
                    canvas_for_resize.set_height(ph);
                    let _ = window_for_resize.request_inner_size(PhysicalSize::new(pw, ph));
                    window_for_resize.request_redraw();
                });
                let _ = web_window.add_event_listener_with_callback(
                    "resize",
                    closure.as_ref().unchecked_ref(),
                );
                closure.forget();
            }

            // Don't hide loading screen yet — keep it visible until the game renders.
            // It will be hidden when GameState init completes.

            // On WASM, we can't block on async. Spawn the init and store the
            // result in a shared cell that we check each frame.
            let pending = self.pending_state.clone();
            let window_clone = window.clone();
            wasm_bindgen_futures::spawn_local(async move {
                log::info!("WASM: starting GameState init...");
                let state = GameState::new(window).await;
                log::info!("WASM: GameState init complete!");
                *pending.borrow_mut() = Some(state);
                // Trigger a redraw so the event loop picks up the new state
                window_clone.request_redraw();
            });
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // On WASM, check if async init has completed
        #[cfg(target_arch = "wasm32")]
        if self.state.is_none() {
            let ready = self.pending_state.borrow().is_some();
            if ready {
                self.state = self.pending_state.borrow_mut().take();
                log::info!("GameState ready (WASM async init complete)");
                // Hide loading screen now that the game is ready
                if let Some(loading) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id("loading"))
                {
                    let _ = loading.set_attribute("style", "display:none");
                }
                // winit's ResizeObserver fires its initial Resized event during
                // async init, before self.state exists — so the early-return in
                // this handler drops it and the wgpu surface stays at its 1x1
                // default. Force a one-time resize to the current viewport here.
                if let Some(state) = self.state.as_mut() {
                    use winit::platform::web::WindowExtWebSys;
                    let web_window = web_sys::window().expect("web window");
                    // Physical px = CSS px × devicePixelRatio (see initial-sizing note above).
                    let dpr = web_window.device_pixel_ratio().max(1.0);
                    let vw = web_window.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1280.0);
                    let vh = web_window.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(720.0);
                    let w = ((vw * dpr).round() as u32).max(1);
                    let h = ((vh * dpr).round() as u32).max(1);
                    if let Some(canvas) = state.window.canvas() {
                        canvas.set_width(w);
                        canvas.set_height(h);
                    }
                    state.renderer.resize(w, h);
                    state.screens = crate::screen::compute_screen_layout(
                        state.players.len(),
                        w,
                        h,
                    );
                    // Sync camera aspect alongside renderer — the initial
                    // PlayerSlot::new aspect was computed from winit's 0x0
                    // inner_size, so view_proj would otherwise keep an
                    // infinite [0][0] until the first window resize fires.
                    for screen in &state.screens {
                        let pidx = match &screen.content {
                            crate::screen::ScreenContent::LocalPlayer(idx) => *idx,
                        };
                        if let Some(p) = state.players.get_mut(pidx) {
                            p.camera.aspect = screen.viewport.aspect();
                        }
                    }
                    log::info!("WASM: post-init resync renderer to {}x{}", w, h);
                }
            }
        }

        let Some(state) = &mut self.state else {
            return;
        };

        // Pass events to egui first. (Window events only arrive with a real
        // window, so the headless arm of the gate is unreachable in practice.)
        // While the cursor is captured for 3D play, pointer events are withheld
        // from egui: no clickable UI is up then, and an overlay left
        // interactable by mistake would otherwise re-show the cursor every
        // frame (egui's `set_cursor_visible(true)`). Gameplay reads the mouse
        // through its own input path, so nothing is lost. Ported from the
        // 2026-06-26 native-polish branch as a systemic guard behind the
        // per-panel `interactable(false)` fixes.
        let suppress_egui_pointer = state.input.cursor_captured
            && matches!(
                event,
                WindowEvent::CursorMoved { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::CursorLeft { .. }
            );
        let egui_consumed = !suppress_egui_pointer
            && state
                .window
                .winit_arc()
                .is_some_and(|w| state.renderer.egui.on_window_event(w, &event));

        match event {
            WindowEvent::CloseRequested => {
                // Native only — the browser has no close-save (its autosave
                // rides IndexedDB). Closing is one more way out of a world, so
                // it goes through the same `leave_world` Save & Quit uses; it
                // SAVES while the player is in a loaded world this machine
                // owns — their own or a Trial arena (never from the lobby,
                // mid-load, after "Quit without saving", or a joined session).
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // Give the router its port back and retire the bearer with
                    // the bounded, on-thread variant first, so `leave_world`'s
                    // own `stop_online` finds nothing left to wait on.
                    state.stop_online_on_exit();
                    let in_world = matches!(state.mode, GameMode::Playing | GameMode::Paused { .. });
                    // A close never throws away a crash-recovery autosave:
                    // it saves a live own world or arena, or leaves disk alone.
                    let choice = crate::world_exit::close_choice(in_world, state.live_world);
                    if state.live_world.is_some() {
                        state.leave_world(choice, crate::world_exit::ExitTo::Quit);
                    }
                }
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                #[cfg(target_arch = "wasm32")]
                {
                    use winit::platform::web::WindowExtWebSys;
                    if let Some(canvas) = state.window.canvas() {
                        // winit's ResizeObserver may reset canvas backing to 1x1 if
                        // CSS layout hadn't settled when the observer first ticked.
                        // Force the backing store to match the Resized event dims so
                        // the wgpu surface always renders at the right resolution.
                        canvas.set_width(size.width.max(1));
                        canvas.set_height(size.height.max(1));
                    }
                    log::info!("WASM: Resized to {}x{}", size.width, size.height);
                }
                state.renderer.resize(size.width, size.height);
                state.screens = crate::screen::compute_screen_layout(
                    state.players.len(),
                    size.width,
                    size.height,
                );
                for screen in &state.screens {
                    let pidx = match &screen.content {
                        crate::screen::ScreenContent::LocalPlayer(idx) => *idx,
                    };
                    if let Some(p) = state.players.get_mut(pidx) {
                        p.camera.aspect = screen.viewport.aspect();
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    state.touch.set_screen_size(size.width as f32, size.height as f32);
                    // Keep the touch hotbar hit-band aligned with the drawn
                    // hotbar across DPR changes (e.g. moving a window between
                    // monitors, or the browser zoom). Same ppp egui draws with.
                    state
                        .touch
                        .set_pixels_per_point(state.renderer.egui.ctx.pixels_per_point());
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                // Crafting UI mouse tracking (for cursor item follow)
                if state.players[0].crafting_ui.open {
                    let w = state.window.inner_size().width.max(1) as f32;
                    let h = state.window.inner_size().height.max(1) as f32;
                    let ndc_x = (position.x as f32 / w) * 2.0 - 1.0;
                    let ndc_y = 1.0 - (position.y as f32 / h) * 2.0;
                    state.players[0].crafting_ui.mouse_ndc = [ndc_x, ndc_y];
                }
            }

            WindowEvent::Focused(focused) => {
                if !focused {
                    // Focus lost (alt-tab / switching windows or tabs). The OS/browser
                    // silently drops the pointer grab and stops sending key-up events, so:
                    //   • held movement keys would stay latched (stuck WASD on return)
                    //   • cursor_captured would stay stale-true → the next click hits the
                    //     break/place path instead of re-capturing, and on web no
                    //     MouseMotion deltas arrive without pointer lock, so the view
                    //     won't turn ("trackpad is off").
                    state.input.release_all_inputs();
                    if state.input.cursor_captured {
                        state.release_cursor();
                    }
                } else {
                    // Returning: discard any stray accumulated look-delta so the camera
                    // doesn't snap. Re-capture happens on the next click — that's the only
                    // way to re-acquire pointer lock on web anyway.
                    state.input.mouse_dx = 0.0;
                    state.input.mouse_dy = 0.0;
                }
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state: key_state,
                        repeat,
                        ..
                    },
                ..
            } => {
                // Suppress all engine key handling while a DOM overlay (the
                // voice-feedback modal) has focus. feedback.js handles F6,
                // Space, and Escape itself while open.
                if crate::input::dom_overlay_blocks_input() {
                    return;
                }
                // Chat overlay swallows keyboard while open. egui (via
                // on_window_event above) already handled the keystroke for
                // the text field, history, Enter, and Esc — we just need to
                // not double-process it as gameplay input.
                //
                // CRITICAL: still let key releases through. If the user was
                // holding W (or any movement key) when they pressed T to
                // open chat, dropping releases here would leave keys_held
                // stuck — the player walks forward after chat closes
                // without W being held. Releases never trigger gameplay
                // actions on their own; they only un-set held state.
                if state.chat.open {
                    if matches!(key_state, ElementState::Released) {
                        state.input.key_released(key);
                    }
                    return;
                }
                // If an egui widget already consumed this key, it has
                // keyboard focus — the inventory-explorer search box being
                // the prime case. Don't ALSO run it as gameplay input, or
                // typing 'e'/'t'/'b' into the search box toggles the
                // inventory, opens chat, and closes the menu (the
                // 2026-05-30 playtest keyboard-trap bug). Same stuck-key
                // caveat as the chat guard above: always let releases
                // through so a movement key held when a widget grabbed
                // focus doesn't latch on.
                if egui_consumed {
                    if matches!(key_state, ElementState::Released) {
                        state.input.key_released(key);
                    }
                    return;
                }
                // T or / opens the chat overlay during play. Slash pre-fills
                // the input so the user can keep typing the command.
                // Gated by `is_commands_enabled` — set per-world via the menu's
                // Create dialog. Worlds created with Commands: OFF can't open
                // chat at all (no /give, no /time, no /tp).
                if matches!(key_state, ElementState::Pressed)
                    && !repeat
                    && matches!(state.mode, GameMode::Playing)
                    && state.is_commands_enabled
                    && !state.players.first().map(|p| p.crafting_ui.open).unwrap_or(false)
                {
                    if key == KeyCode::KeyT {
                        state.chat.open_with_prefix("");
                        return;
                    }
                    if key == KeyCode::Slash {
                        state.chat.open_with_prefix("/");
                        return;
                    }
                }
                match key_state {
                ElementState::Pressed if !repeat => {
                    match key {
                        KeyCode::Escape => {
                            // Determine action first, then apply (avoids borrow conflict)
                            enum EscAction { SkipSplash, CloseDialog, Quit, Resume, CloseExplorer, CloseBook, CloseCraft, CloseVillager, CloseBuildChoice, ClosePanel, Pause, Ignore }
                            // Computed BEFORE the `&mut state.mode` match below —
                            // `p0_ui_modal_open(&self)` borrows all of `state`,
                            // which would conflict with that match's mutable
                            // borrow of the `mode` field if called from inside it.
                            let any_panel_open = state.p0_ui_modal_open();
                            let esc_action = match &mut state.mode {
                                GameMode::Splash(_) => EscAction::SkipSplash,
                                // #7 — Loading is a single transient frame; ESC does nothing.
                                GameMode::Loading(_) => EscAction::Ignore,
                                GameMode::Menu(menu_state) => {
                                    if matches!(menu_state.dialog, crate::menu::MenuDialog::None) {
                                        EscAction::Quit
                                    } else {
                                        menu_state.dialog = crate::menu::MenuDialog::None;
                                        EscAction::CloseDialog
                                    }
                                }
                                GameMode::Paused { .. } => EscAction::Resume,
                                GameMode::Playing => {
                                    // Layered close: Esc closes the item-search
                                    // Explorer first (leaving the inventory open);
                                    // a second Esc then closes the inventory.
                                    if state.players[0].explorer_state.is_some() {
                                        EscAction::CloseExplorer
                                    } else if state.players[0].crafting_ui.open
                                        && state.players[0].crafting_ui.book_open
                                    {
                                        // Recipe book is a layer inside the inventory:
                                        // Esc closes the book back to the grid, a second
                                        // Esc then closes the inventory.
                                        EscAction::CloseBook
                                    } else if state.players[0].crafting_ui.open {
                                        EscAction::CloseCraft
                                    } else if state.players[0].dialogue_villager.is_some() {
                                        EscAction::CloseVillager
                                    } else if state.players[0].pending_build_choice.is_some() {
                                        // Guided build-along — Esc backs out of the
                                        // "how do you want this built?" dialog
                                        // without laying anything (the plan stays
                                        // in the bag), rather than pausing behind it.
                                        EscAction::CloseBuildChoice
                                    } else if any_panel_open {
                                        // P6 audit fix — Wardrobe, Rig Studio,
                                        // Challenge Board, chest, vendor, furnace,
                                        // sign, Satoshi dialogue, Workshop painter
                                        // and the trial-outcome card all used to
                                        // fall through to Pause here: their own
                                        // Esc handlers only run on a live (non-
                                        // Paused) egui frame, and going straight
                                        // to Paused meant that frame never ran, so
                                        // the panel stayed open behind a locked,
                                        // hidden cursor. Close it instead.
                                        EscAction::ClosePanel
                                    } else {
                                        EscAction::Pause
                                    }
                                }
                            };
                            match esc_action {
                                EscAction::Ignore => {}
                                EscAction::SkipSplash => {
                                    // Skip the cosmetic splash straight to the lobby — web too.
                                    // The splash only renders AFTER auth completes (auth.js calls
                                    // __axenstax_start post sign-in), so skipping is safe there;
                                    // this is also the manual escape hatch if the timed transition
                                    // ever stalls.
                                    state.mode = GameMode::Menu(Box::new(crate::menu::MenuState::new()));
                                }
                                EscAction::Quit => {
                                    // In a showcase, exit is a one-step dead-end to the
                                    // terminal screen — never a bare lobby (spec §10).
                                    if crate::showcase::should_dead_end_exit(&state.showcase) {
                                        state.showcase_exited = true;
                                        state.release_cursor();
                                    } else {
                                        event_loop.exit();
                                    }
                                    return;
                                }
                                EscAction::Resume => {
                                    // ESC while paused resumes; close the "Your
                                    // look" panel so it doesn't linger open.
                                    state.skin_panel.open = false;
                                    state.mode = GameMode::Playing;
                                    // P6 audit: a panel left open behind the
                                    // pause screen (see close_topmost_ui_panel)
                                    // must keep the cursor free — capturing
                                    // unconditionally here locked it over a
                                    // still-open Wardrobe/chest/etc.
                                    if !state.p0_ui_modal_open() {
                                        state.capture_cursor();
                                    }
                                }
                                EscAction::CloseExplorer => {
                                    state.players[0].explorer_state = None;
                                    // Keep the cursor released if the inventory (or
                                    // another panel) is still open behind it.
                                    if !state.p0_ui_modal_open() {
                                        state.capture_cursor();
                                    }
                                }
                                EscAction::CloseBook => {
                                    // Close the recipe book back to the crafting grid;
                                    // the inventory stays open (cursor stays released).
                                    state.players[0].crafting_ui.book_open = false;
                                }
                                EscAction::CloseCraft => {
                                    let p = &mut state.players[0];
                                    p.crafting_ui.close(&mut p.inventory);
                                    state.capture_cursor();
                                }
                                EscAction::CloseVillager => {
                                    state.players[0].dialogue_villager = None;
                                    state.capture_cursor();
                                }
                                EscAction::CloseBuildChoice => {
                                    state.players[0].pending_build_choice = None;
                                    if !state.p0_ui_modal_open() {
                                        state.capture_cursor();
                                    }
                                }
                                EscAction::ClosePanel => {
                                    state.close_topmost_ui_panel();
                                    if !state.p0_ui_modal_open() {
                                        state.capture_cursor();
                                    }
                                }
                                EscAction::Pause => {
                                    state.release_cursor();
                                    // Fresh pause always shows the pause buttons,
                                    // never a stale "Your look" panel.
                                    state.skin_panel.open = false;
                                    state.mode = GameMode::Paused { confirm_quit: false, confirm_creative: false };
                                }
                                EscAction::CloseDialog => {} // Already handled above
                            }
                        }
                        KeyCode::F3 => {
                            state.show_debug = !state.show_debug;
                        }
                        // #8 — F7 toggles the spawn-proof overlay (red markers on
                        // surface cells where a mob can spawn at night).
                        KeyCode::F7 => {
                            state.spawn_overlay = !state.spawn_overlay;
                        }
                        // F11 toggles borderless fullscreen (native only; the
                        // browser owns fullscreen on WASM).
                        #[cfg(not(target_arch = "wasm32"))]
                        KeyCode::F11 => {
                            let next = if state.window.fullscreen().is_some() {
                                None
                            } else {
                                Some(winit::window::Fullscreen::Borderless(None))
                            };
                            state.window.set_fullscreen(next);
                        }
                        // Menu keyboard navigation: egui handles its own keyboard nav
                        // Arrow/Enter keys are passed to egui via on_window_event above
                        _ => {}
                    }
                    state.input.key_pressed(key);
                }
                ElementState::Pressed => {} // key repeat — ignore
                ElementState::Released => {
                    state.input.key_released(key);
                }
                }
            }

            WindowEvent::MouseInput {
                state: btn_state,
                button,
                ..
            } => {
                // Chat eats mouse buttons too — otherwise a click during typing
                // (or a stuck press across the chat-close transition) registers
                // as block break/place on the next tick.
                //
                // Same stuck-state caveat as the keyboard handler: a held
                // press at chat-open time would leave left_held / right_held
                // true forever if releases were dropped here. Let releases
                // through; suppress only presses.
                if state.chat.open {
                    if matches!(btn_state, ElementState::Released) {
                        state.input.mouse_button_released(button);
                    }
                    return;
                }
                if btn_state == ElementState::Pressed {
                    match &state.mode {
                        GameMode::Splash(_) => {
                            // Click skips the cosmetic splash to the lobby — web included
                            // (auth is already complete by the time the splash renders).
                            state.mode = GameMode::Menu(Box::new(crate::menu::MenuState::new()));
                        }
                        // Menu and Pause: egui handles click detection via draw functions
                        // in update_and_render(). No manual hit-testing needed.
                        // Loading (#7) is a transient one-frame screen — ignore clicks.
                        GameMode::Menu(_) | GameMode::Paused { .. } | GameMode::Loading(_) => {
                            // egui already got this event via on_window_event above
                        }
                        GameMode::Playing => {
                            if egui_consumed {
                                // egui handled this click (e.g., crafting UI button)
                            } else if state.players[0].crafting_ui.open {
                                // Crafting UI is open but click wasn't on an egui widget
                                // (click outside = no action, egui handles slot clicks)
                            } else if state
                                .scenario
                                .as_ref()
                                .is_some_and(|s| s.shows_blocking_end_card())
                            {
                                // Scenario end-card modal is up — egui owns its
                                // "Back to Menu" button and the gamestr opt-in
                                // overlay sits on top. A stray click must NOT
                                // re-capture the cursor (which would re-lock the
                                // pointer and make both unclickable again).
                            } else if !state.input.cursor_captured {
                                state.capture_cursor();
                            } else if state.showcase.enabled
                                && matches!(button, winit::event::MouseButton::Left)
                            {
                                // Kiosk: a left-click collects the exhibit you're
                                // looking at (no block break / attack). The actual
                                // pick runs in the per-player raycast block, which
                                // has the eye + look ray.
                                state.collect_requested = true;
                            } else {
                                state.input.mouse_button_pressed(button);
                            }
                        }
                    }
                } else {
                    // ElementState::Released
                    state.input.mouse_button_released(button);
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                if state.chat.open {
                    return;
                }
                if state.input.cursor_captured {
                    match delta {
                        MouseScrollDelta::LineDelta(_, y) => {
                            state.input.scroll(y);
                        }
                        MouseScrollDelta::PixelDelta(pos) => {
                            state.input.scroll(pos.y as f32 / 100.0);
                        }
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                state.update_and_render();
            }

            WindowEvent::Touch(_touch) => {
                #[cfg(target_arch = "wasm32")]
                {
                    let x = _touch.location.x as f32;
                    let y = _touch.location.y as f32;
                    let id = _touch.id;
                    match _touch.phase {
                        winit::event::TouchPhase::Started => {
                            state.touch.on_touch_start(id, x, y);
                        }
                        winit::event::TouchPhase::Moved => {
                            state.touch.on_touch_move(id, x, y);
                        }
                        winit::event::TouchPhase::Ended
                        | winit::event::TouchPhase::Cancelled => {
                            state.touch.on_touch_end(id);
                        }
                    }
                }
            }

            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        let Some(state) = &mut self.state else {
            return;
        };

        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event
            && state.input.cursor_captured {
                state.input.mouse_moved(dx, dy);
            }
    }

    /// Drive a continuous render loop under `ControlFlow::Poll`. winit only emits
    /// `RedrawRequested` when something calls `request_redraw()`; relying on each
    /// rendered frame to re-request is fragile on the **web** backend — an
    /// input-less screen (the launch splash) stalls because no event re-kicks the
    /// chain, so its timed transition to the lobby never fires (gameplay survives
    /// only because the mouse keeps generating events). Requesting a redraw every
    /// `about_to_wait` guarantees frames keep flowing; frame pacing in
    /// `update_and_render` (native) and `requestAnimationFrame` (web) cap the rate.
    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    // Offline-first identity (login contract, Decision 2): load the cached Signet
    // identity with ZERO network, so who-you-are is known before any connection.
    // This is NOT a gate — a guest plays the whole game; sign-in is an opt-in menu
    // action layered on top. It also publishes the owner pubkey for Seam B saves.
    // First resolve the per-user data dir (and copy any legacy CWD state into
    // it) — BEFORE anything reads native state (audit 2026-09-27).
    data_dir::init();
    let identity = signet::native_signer::init_identity();
    match identity.npub() {
        Some(npub) => log::info!("Signed in as {npub} (cached identity, offline)"),
        None => log::info!("Playing as guest — sign-in is optional"),
    }

    // Native mailbox: start the background worker now — it retires the old
    // device key/inbox files, flushes any queued report, and reads the public
    // feedback status board off the frame thread. `profile_dir()` is the same
    // data-dir `profile/` the Signet identity above uses (session_path() in
    // signet/native_signer.rs) — consistent by design.
    #[cfg(not(target_arch = "wasm32"))]
    crate::native_mailbox::init(&crate::data_dir::profile_dir());

    // Signet contacts sync: boot fetch + the 15-minute loop, on its own
    // worker thread (never the frame thread).
    #[cfg(not(target_arch = "wasm32"))]
    crate::signet_contacts::init();

    let args: Vec<String> = std::env::args().collect();

    // Headless dedicated server (Docker). Runs the authoritative simulation +
    // WebSocket accept loop with NO window/renderer, then returns. Everything
    // below (event loop, GPU) is skipped. See server_main.rs.
    // Server-identity provisioning subcommands (one-shot, native-only). Each
    // builds its own async runtime, does its work, and returns. See server_main.
    if args.iter().any(|a| a == "--pair-server") {
        server_main::run_pair(&args);
        return;
    }
    if args.iter().any(|a| a == "--refresh-delegation") {
        server_main::run_refresh(&args);
        return;
    }
    if args.iter().any(|a| a == "--show-connect") {
        server_main::run_show_connect(&args);
        return;
    }
    if args.iter().any(|a| a == "--admin") {
        server_main::run_admin(&args);
        return;
    }
    if args.iter().any(|a| a == "--admin-publish") {
        server_main::run_admin_publish(&args);
        return;
    }
    if args.iter().any(|a| a == "--admin-sign") {
        server_main::run_admin_sign(&args);
        return;
    }

    if args.iter().any(|a| a == "--server") {
        server_main::run(&args);
        return;
    }

    if args.iter().any(|a| a == "--screenshot") {
        run_screenshot(&args);
        return;
    }

    if args.iter().any(|a| a == "--shot-lobby") {
        run_shot_lobby(&args);
        return;
    }

    if args.iter().any(|a| a == "--shot-signin") {
        run_shot_signin(&args);
        return;
    }

    if args.iter().any(|a| a == "--shot-painter") {
        run_shot_painter(&args);
        return;
    }

    if args.iter().any(|a| a == "--shot-workshop") {
        run_shot_workshop(&args);
        return;
    }

    if args.iter().any(|a| a == "--shot-3p") {
        run_shot_3p(&args);
        return;
    }

    if args.iter().any(|a| a == "--dump-textures") {
        run_dump_textures(&args);
        return;
    }

    log::info!("Axe'n'Stax -- Prototype (Step 19: Splash + World Management)");
    log::info!("Controls: WASD=move, Mouse=look, LClick=break, RClick=place");
    log::info!("          1-9 or Scroll=select block, Space=jump, Ctrl=sprint, Shift=sneak");
    log::info!("          Double-tap Space=toggle flight, F3=debug, Escape=quit");

    // Kick the version check off before the window exists, so the answer is
    // usually ready by the time the lobby first draws. Deliberately placed
    // AFTER the `--server` / `--admin-*` / `--screenshot` dispatch above: a
    // dedicated server or a CI screenshot run has no lobby to show it in and
    // should not be making the request at all.
    //
    // Non-blocking, and a no-op when AXENSTAX_NO_UPDATE_CHECK is set. The signed
    // release feed is read from the player's own relay list.
    update_check::start_once(crate::graphics_settings::GraphicsSettings::load().online_relays);

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = App::new();
    event_loop.run_app(&mut app).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // WASM entry is handled by web_main.rs via #[wasm_bindgen(start)]
    // This main() exists only to satisfy the Rust compiler.
}


/// Build a deterministic sample lobby (world list) for screenshot iteration.
#[cfg(not(target_arch = "wasm32"))]
fn sample_menu_state() -> crate::menu::MenuState {
    use crate::save::{WorldEntry, WorldMeta};
    let mk = |name: &str, desc: &str, mode: &str, version: u32, cloud: bool, secs_ago: u64, size: u64| {
        let mut meta = WorldMeta::new(name);
        meta.description = desc.to_string();
        meta.game_mode = mode.to_string();
        meta.pure_survival = mode == "survival";
        meta.ever_creative = mode == "creative";
        meta.version = version;
        meta.cloud_save = cloud;
        WorldEntry {
            folder_name: crate::save::sanitize_folder_name(name),
            meta,
            last_played: std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago),
            size_bytes: size,
            cloud_only: false,
        }
    };
    let mut state = crate::menu::MenuState::new();
    state.worlds = vec![
        mk("My Castle", "A big base by the river", "survival", 12, true, 7_200, 3_400_000),
        mk("Sky City", "Floating islands build", "creative", 3, false, 86_400, 1_100_000),
        mk("Cave Base", "Deep diamond mine", "survival", 47, true, 600, 8_900_000),
        mk("Redstone Lab", "", "creative", 8, false, 3_600, 2_200_000),
        mk("Jungle Hideout", "New world", "survival", 1, true, 120, 540_000),
    ];
    state.selected = Some(0);
    state.world_page = 0;
    state
}

/// Headless lobby screenshot mode: render the world list at several screen sizes
/// to PNGs, for UX iteration without opening a window. `--shot-lobby [out_dir]`.
#[cfg(not(target_arch = "wasm32"))]
fn run_shot_lobby(args: &[String]) {
    let out_dir = args
        .iter()
        .position(|a| a == "--shot-lobby")
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "/tmp/lobby-shots".to_string());
    std::fs::create_dir_all(&out_dir).ok();

    let textures = crate::texture_gen::generate_textures();
    let sizes = [("desktop", 1280u32, 800u32), ("tablet", 834, 1112), ("phone", 390, 844)];
    for (name, w, h) in sizes {
        let mut renderer = pollster::block_on(Renderer::new_headless(w, h, &textures));
        let mut menu = sample_menu_state();
        // Campaign G — pre-expand the first race so the shots exercise the
        // expanded body (best/rival lines + Play / Export ghost / Import ghost).
        menu.expanded_trial = Some("race:sprint".to_string());
        let path = format!("{out_dir}/lobby-{name}.png");
        renderer.render_menu_to_png(&path, |ctx| {
            let _ = crate::menu::draw_main_menu(ctx, &mut menu);
        });
    }
    log::info!("Lobby shots written to {out_dir}");
}

/// Dump the procedural default pack to editable PNGs, one per texture key.
/// `--dump-textures [out_dir]` (default `/tmp/axenstax-default-pack`). This is
/// the canonical default-pack producer for the texture-pack pipeline (P1): edit
/// a dumped PNG and a later pack loader (P2) overrides that layer by name.
#[cfg(not(target_arch = "wasm32"))]
fn run_dump_textures(args: &[String]) {
    let out_dir = args
        .iter()
        .position(|a| a == "--dump-textures")
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "/tmp/axenstax-default-pack".to_string());
    match crate::texture_registry::dump_textures(std::path::Path::new(&out_dir)) {
        Ok(n) => log::info!("Dumped {n} default-pack PNGs to {out_dir}"),
        Err(e) => log::error!("texture dump failed: {e}"),
    }
}

/// Headless render of the native Signet sign-in dialog (the QR + paste screen),
/// for UX iteration without a window. `--shot-signin [out_path]`. Injects a
/// sample `AwaitingScan` status so the QR renders with **no network** (the live
/// handshake is the device boundary).
#[cfg(not(target_arch = "wasm32"))]
fn run_shot_signin(args: &[String]) {
    let out_path = args
        .iter()
        .position(|a| a == "--shot-signin")
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "/tmp/signin-dialog.png".to_string());

    // A realistic-length nostrconnect:// URI (random app pubkey) so the QR's
    // module density matches a live one.
    let sample_uri = "nostrconnect://b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3?relay=wss%3A%2F%2Frelay.trotters.cc&metadata=%7B%22name%22%3A%22Axe%27n%27Stax%22%7D".to_string();
    crate::native_signin::preview_awaiting_scan(sample_uri);

    let textures = crate::texture_gen::generate_textures();
    let mut renderer = pollster::block_on(Renderer::new_headless(1280, 800, &textures));
    let mut menu = sample_menu_state();
    menu.dialog = crate::menu::MenuDialog::SignIn {
        paste_uri: String::new(),
        show_paste: true,
    };
    // egui auto-sizes a Window over two frames (frame 1 measures, frame 2
    // positions). The renderer's Context persists between calls, so render twice
    // — the second capture has the dialog correctly sized + centred.
    for _ in 0..2 {
        renderer.render_menu_to_png(&out_path, |ctx| {
            let _ = crate::menu::draw_main_menu(ctx, &mut menu);
        });
    }
    log::info!("Sign-in dialog shot written to {out_path}");
}

/// Spec 40 — headless render of the Workshop face-painter panel, for UX iteration
/// without a window. `--shot-painter [out_path]`. Seeds the painter from a flower's
/// current texture and paints a few cells so the grid + palette + faces are visible.
#[cfg(not(target_arch = "wasm32"))]
fn run_shot_painter(args: &[String]) {
    let out_path = args
        .iter()
        .position(|a| a == "--shot-painter")
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "/tmp/workshop-painter.png".to_string());

    let registry = BlockRegistry::new();
    let textures = crate::texture_gen::generate_textures();
    let mut renderer = pollster::block_on(Renderer::new_headless(1280, 800, &textures));

    let mut painter = crate::workshop_painter::WorkshopPainterUi::new();
    painter.open_for(
        crate::workshop_painter::PaintTarget::Block(crate::block::CORNFLOWER),
        &textures,
        &registry,
    );
    // Paint a small motif on the Top face so the grid clearly shows editing.
    painter.selected = 5; // yellow
    for x in 5..11 {
        painter.paint_cell(0, x, 5);
        painter.paint_cell(0, x, 10);
    }
    painter.selected = 9; // blue
    for y in 6..10 {
        painter.paint_cell(0, 5, y);
        painter.paint_cell(0, 10, y);
    }

    let viewport = crate::screen::ViewportRect { x: 0, y: 0, width: 1280, height: 800 };
    renderer.render_menu_to_png(&out_path, |ctx| {
        let _ = crate::workshop_painter::draw_workshop_painter(ctx, &viewport, &mut painter);
    });
    log::info!("Workshop painter shot written to {out_path}");
}

/// Spec 40 — headless render of the in-world Workshop: the void floor with a couple
/// of INFLATED mannequins (a block + a mob) placed on it, proving the inflated
/// working-copy render. `--shot-workshop [out_path]`.
#[cfg(not(target_arch = "wasm32"))]
fn run_shot_workshop(args: &[String]) {
    let out_path = args
        .iter()
        .position(|a| a == "--shot-workshop")
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "/tmp/workshop-inworld.png".to_string());

    let registry = BlockRegistry::new();
    let mut world = World::new();
    world.is_workshop = true;
    let textures = crate::texture_gen::generate_textures();
    let mut renderer = pollster::block_on(Renderer::new_headless(1280, 720, &textures));

    // Void floor around origin.
    let biome_gen = crate::biome::BiomeGenerator::new(1);
    for dx in -3..=3 {
        for dz in -3..=3 {
            world.generate_column(dx, dz, &biome_gen);
            crate::lighting::run_initial_pass_for_column(&mut world, dx, dz, &registry);
        }
    }
    let floor = crate::workshop::WORKSHOP_FLOOR_Y;

    // Two inflated projects on the floor: a flower block and a cow mob.
    let b = world.workshop.add(
        crate::workshop::WorkshopTarget::Block(crate::block::CORNFLOWER),
        crate::workshop::WorkshopMode::Reskin,
        [0, floor, 0],
    );
    for _ in 0..4 {
        world.workshop.get_mut(b).unwrap().pump();
    }
    // Phase 3 — lock the balloon at ×4 and paint/sculpt it so the headless shot
    // shows the in-world editor's output (not just an inflated cube).
    if let Some(proj) = world.workshop.get_mut(b) {
        proj.blow_up = Some(crate::workshop::BlowUp {
            phase: crate::workshop::BlowUpPhase::Locked,
            charge: crate::workshop::BLOW_UP_CHARGE_TICKS,
            collapse_left: 0,
            collapse_from: 4.0,
            corner: [0, floor, 0],
            held_this_tick: false,
        });
        // A solid copy of the source block, then bold strokes that read clearly
        // from any angle: a red band wrapped around all four sides (AllSides
        // rotation), a yellow top cap, and a carved-out top corner (sculpt).
        use crate::block::{WALLPAPER_RED, WALLPAPER_YELLOW};
        use crate::workshop::{EditOp, EditSymmetry};
        let mut buf = crate::workshop::EditBuffer::solid(crate::block::CORNFLOWER);
        // Red band at mid-height around every side (one stroke per column, the
        // 4-fold symmetry carries it to all four faces).
        for x in 0..16 {
            for y in 6..10 {
                buf.apply([x, y, 0], EditOp::Paint(WALLPAPER_RED), EditSymmetry::AllSides);
            }
        }
        // Yellow top cap.
        for x in 0..16 {
            for z in 0..16 {
                buf.apply([x, 15, z], EditOp::Paint(WALLPAPER_YELLOW), EditSymmetry::Off);
            }
        }
        // Carve a 4×4×4 notch out of the top-front-left corner (visible sculpt).
        for x in 0..4 {
            for y in 12..16 {
                for z in 0..4 {
                    buf.apply([x, y, z], EditOp::Carve, EditSymmetry::Off);
                }
            }
        }
        proj.edit = Some(buf);
    }
    let m = world.workshop.add(
        crate::workshop::WorkshopTarget::Mob(crate::mob::MobType::Cow),
        crate::workshop::WorkshopMode::Reskin,
        [3, floor, 0],
    );
    for _ in 0..2 {
        world.workshop.get_mut(m).unwrap().pump();
    }

    // Mesh the floor chunks.
    for (cx, cy, cz) in world.chunk_positions().collect::<Vec<_>>() {
        let meshes = build_chunk_meshes(cx, cy, cz, &world, &registry);
        renderer.upload_chunk((cx, cy, cz), &meshes);
    }
    // Build + upload the in-world mannequins (the entity pass draws them).
    let mannequins = crate::entity_model::build_workshop_inworld_vertices(&world, &registry, 0.0, false);
    renderer.upload_entity_vertices(0, &mannequins);
    log::info!("Workshop in-world: {} mannequin vertices", mannequins.len());

    // Camera: stand back from the two projects and look at the floor.
    let eye = glam::Vec3::new(-2.0, floor as f32 + 4.5, 7.0);
    let target = glam::Vec3::new(1.5, floor as f32 + 2.0, 0.0);
    let proj = glam::Mat4::perspective_rh(70.0_f32.to_radians(), 1280.0 / 720.0, 0.1, 300.0);
    let view = glam::Mat4::look_at_rh(eye, target, glam::Vec3::Y);
    let camera_uniform = CameraUniform {
        view_proj: (proj * view).to_cols_array_2d(),
        camera_pos: [eye.x, eye.y, eye.z, 0.0],
        sun_dir: [0.3, 1.0, 0.5, 1.0],
        fog: crate::camera::default_fog(),
        params: [1.0, 0.0, 0.0, 0.0],
        cam_right: [1.0, 0.0, 0.0, 0.0],
        cam_up: [0.0, 1.0, 0.0, 0.0],
    };
    renderer.update_camera(0, &camera_uniform);
    renderer.render_to_png(&out_path);
    log::info!("Workshop in-world shot written to {out_path}");
}

/// Headless screenshot mode: generate world, render one frame, save PNG, exit.
#[cfg(not(target_arch = "wasm32"))]
fn run_screenshot(args: &[String]) {
    // Parse optional output path: --screenshot [path]
    let default_path = "screenshot.png".to_string();
    let output_path = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
        .unwrap_or(&default_path);

    log::info!("Axe'n'Stax -- Headless screenshot mode");
    log::info!("Output: {output_path}");

    let _registry = BlockRegistry::new();
    let mut world = World::new();
    let biome_gen = crate::biome::BiomeGenerator::new(42);

    let textures = crate::texture_gen::generate_textures();
    let mut renderer = pollster::block_on(Renderer::new_headless(1280, 720, &textures));

    // Generate terrain around origin
    let render_dist = 6; // smaller for speed
    log::info!("Generating terrain ({} chunk radius)...", render_dist);
    let registry = crate::block::BlockRegistry::new();
    for dx in -render_dist..=render_dist {
        for dz in -render_dist..=render_dist {
            world.generate_column(dx, dz, &biome_gen);
            crate::lighting::run_initial_pass_for_column(&mut world, dx, dz, &registry);
        }
    }

    // Owner-inbox #18 — register the built-in flower micro-models so the
    // headless screenshot is faithful to the in-game micro pass (natural flowers
    // render as 3D shells via `render_to_png`'s micro pass + `sync_micro_models`).
    world.load_bundled_micro_models(&registry);

    // Mesh all chunks
    let positions: Vec<_> = world.chunk_positions().collect();
    let mut meshed = 0;
    for (cx, cy, cz) in positions {
        let meshes = build_chunk_meshes(cx, cy, cz, &world, &registry);
        if !meshes.opaque.vertices.is_empty() {
            meshed += 1;
        }
        renderer.upload_chunk((cx, cy, cz), &meshes);
    }
    log::info!("Meshed {meshed} chunks.");

    // Find spawn height
    let mut spawn_y = 80;
    while spawn_y > 0 && world.get_block(0, spawn_y, 0) == block::AIR {
        spawn_y -= 1;
    }
    let spawn_y = spawn_y as f32 + 1.0;

    // Position camera looking across the terrain at a slight angle
    let eye = glam::Vec3::new(8.0, spawn_y + 12.0, 8.0);
    let target = glam::Vec3::new(0.0, spawn_y - 2.0, -20.0);
    let aspect = 1280.0 / 720.0;

    let view = glam::Mat4::look_at_rh(eye, target, glam::Vec3::Y);
    let proj = glam::Mat4::perspective_rh(70.0_f32.to_radians(), aspect, 0.1, 300.0);
    let view_proj = proj * view;

    let camera_uniform = CameraUniform {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [eye.x, eye.y, eye.z, 0.0],
        sun_dir: [0.3, 1.0, 0.5, 1.0],
        fog: crate::camera::default_fog(),
        params: [1.0, 0.0, 0.0, 0.0],
        cam_right: [1.0, 0.0, 0.0, 0.0],
        cam_up: [0.0, 1.0, 0.0, 0.0],
    };
    renderer.update_camera(0, &camera_uniform);

    // Owner-inbox #18 — upload the per-type baked micro-model shells so the
    // headless screenshot's micro pass can draw them.
    renderer.sync_micro_models(&world.micro_registry);

    // Render and save (headless: no egui HUD, just world + crosshair)
    renderer.render_to_png(output_path);
    log::info!("Done.");
}

/// L4 headless smoke for the third-person camera (Phase 1). Poses a player on
/// real terrain, sets the camera to over-the-shoulder, renders ONE frame to PNG,
/// and asserts the frame is non-blank AND the player's own avatar appears in the
/// lower-centre — the catastrophic-failure catch (black screen / avatar missing /
/// crash) plus a human-glanceable artifact. The avatar wears a solid magenta test
/// skin so the assertion can find it unambiguously against terrain/sky.
/// Spec: `docs/foundations/2026-06-09-third-person-camera.md`.
#[cfg(not(target_arch = "wasm32"))]
fn run_shot_3p(args: &[String]) {
    let default_path = "tools/smoke/out/play-3p.png".to_string();
    let output_path = args
        .iter()
        .position(|a| a == "--shot-3p")
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
        .unwrap_or(&default_path)
        .clone();
    if let Some(dir) = std::path::Path::new(&output_path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    log::info!("Axe'n'Stax -- Headless third-person (Phase 1) smoke → {output_path}");

    let registry = crate::block::BlockRegistry::new();
    let mut world = World::new();
    let biome_gen = crate::biome::BiomeGenerator::new(42);
    let textures = crate::texture_gen::generate_textures();
    let mut renderer = pollster::block_on(Renderer::new_headless(1280, 720, &textures));

    let render_dist = 6;
    for dx in -render_dist..=render_dist {
        for dz in -render_dist..=render_dist {
            world.generate_column(dx, dz, &biome_gen);
            crate::lighting::run_initial_pass_for_column(&mut world, dx, dz, &registry);
        }
    }
    world.load_bundled_micro_models(&registry);
    for (cx, cy, cz) in world.chunk_positions().collect::<Vec<_>>() {
        let meshes = build_chunk_meshes(cx, cy, cz, &world, &registry);
        renderer.upload_chunk((cx, cy, cz), &meshes);
    }
    renderer.sync_micro_models(&world.micro_registry);

    // Stand the player on the terrain at the origin.
    let mut ground = 80;
    while ground > 0 && world.get_block(0, ground, 0) == block::AIR {
        ground -= 1;
    }
    let foot = glam::Vec3::new(0.5, ground as f32 + 1.0, 0.5);

    // Solid magenta test skin (layer 0) so the avatar is unmistakable in the PNG.
    let skin: Vec<u8> = std::iter::repeat_n([255u8, 0, 255, 255], 64 * 64).flatten().collect();
    renderer.write_avatar_skin(&skin);

    // The player faces -Z; build their own avatar (over-the-shoulder).
    let ps = crate::protocol::PlayerState {
        player_index: 0,
        x: foot.x,
        y: foot.y,
        z: foot.z,
        yaw: 0.0,
        pitch: 0.0,
        health: 20.0,
        held_kind: crate::protocol::item_kind::EMPTY,
        held_id: 0,
        anim_state: 0, // idle
        flags: 0,
        skin_key: 0,
    };
    let eye = foot + glam::Vec3::new(0.0, 1.62, 0.0);

    // Re-open a rendered PNG and measure (distinct-colour count → non-blank;
    // magenta-ish pixels in the lower-centre → the self-avatar is on screen).
    fn assess(path: &str) -> (usize, u32) {
        let img = image::open(path).expect("re-open rendered PNG").to_rgba8();
        let (w, h) = img.dimensions();
        let mut seen: std::collections::HashSet<[u8; 3]> = std::collections::HashSet::new();
        for p in img.pixels() {
            seen.insert([p[0] >> 4, p[1] >> 4, p[2] >> 4]); // quantise to ignore dither
            if seen.len() > 12 {
                break;
            }
        }
        let (x0, x1) = ((w as f32 * 0.2) as u32, (w as f32 * 0.8) as u32);
        let (y0, y1) = ((h as f32 * 0.35) as u32, (h as f32 * 0.98) as u32);
        let mut magenta = 0u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = img.get_pixel(x, y).0;
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                if r > 60 && b > 60 && g < 60 && (r - g) > 25 && (b - g) > 25 {
                    magenta += 1;
                }
            }
        }
        (seen.len(), magenta)
    }

    // Sibling paths next to the primary artifact (over-the-shoulder = the canonical
    // play-3p.png; first-person + orbit-behind alongside it for comparison).
    let parent = std::path::Path::new(&output_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let sib = |name: &str| parent.join(name).to_string_lossy().into_owned();
    let first_path = sib("play-3p-first.png");
    let orbit_path = sib("play-3p-orbit.png");

    // Render one frame per camera mode. First-person draws NO self-avatar (the
    // viewmodel hand would, but it's off in headless) → expect ~0 magenta;
    // the two third-person modes draw the body → expect lots of magenta.
    let modes = [
        (crate::camera::CameraMode::FirstPerson, "first-person", first_path.as_str(), false),
        (crate::camera::CameraMode::OverShoulder, "over-the-shoulder", output_path.as_str(), true),
        (crate::camera::CameraMode::OrbitBehind, "orbit-behind", orbit_path.as_str(), true),
    ];
    let mut all_ok = true;
    let mut ps = ps;
    for (mode, label, path, expect_avatar) in modes {
        let mut camera = crate::camera::Camera::new(eye, 1280.0 / 720.0);
        camera.yaw = 0.0;
        camera.pitch = -0.18; // look slightly down so the body frames lower-centre
        camera.mode = mode;

        // Face the avatar where the camera looks (mirrors the game's self-avatar
        // path) so you see the back of the head — the 2026-06-09 facing fix.
        let fwd = camera.forward();
        ps.yaw = crate::entity_model::yaw_facing(fwd.x, fwd.z);

        let (skin_v, _held) = crate::entity_model::self_avatar_vertices(mode, &ps, 0.0, 0, 0, 1.0, &registry, crate::skin_uv::ArmModel::Classic);
        renderer.upload_avatar_vertices(0, &skin_v); // empty (first-person) clears the buffer

        renderer.update_camera(0, &CameraUniform::from_camera(&camera));
        renderer.render_to_png(path);

        let (colours, magenta) = assess(path);
        let non_blank = colours > 12;
        let avatar_ok = if expect_avatar { magenta > 200 } else { magenta < 50 };
        let ok = non_blank && avatar_ok;
        all_ok &= ok;
        log::info!(
            "{label:>17}: {} | non_blank={non_blank} ({colours} colours) avatar_px={magenta} (expect_avatar={expect_avatar}) → {}",
            path,
            if ok { "PASS" } else { "FAIL" }
        );
    }

    // ── Campaign N (2026-07-05) — night dimming smoke ────────────────────────
    // Same over-the-shoulder scene at midnight (sun.w ≈ 0.05): the self-avatar
    // carries a sampled combined light in `sky_light` and the terrain/plants
    // read `sky * sun.w`, so the whole frame must go dark and the magenta body
    // must stop reading as bright magenta. Guards the entity/avatar/plant
    // night-light plumbing end-to-end (bake → attribute → shader).
    {
        let night_path = sib("play-3p-night.png");
        let mut camera = crate::camera::Camera::new(eye, 1280.0 / 720.0);
        camera.yaw = 0.0;
        camera.pitch = -0.18;
        camera.mode = crate::camera::CameraMode::OverShoulder;
        let fwd = camera.forward();
        ps.yaw = crate::entity_model::yaw_facing(fwd.x, fwd.z);

        let night_brightness = 0.05_f32;
        let (mut skin_v, _held) =
            crate::entity_model::self_avatar_vertices(camera.mode, &ps, 0.0, 0, 0, 1.0, &registry, crate::skin_uv::ArmModel::Classic);
        let ch = world.light_channels_at(foot.x, foot.y, foot.z);
        crate::entity_model::set_avatar_light(
            &mut skin_v,
            crate::entity_model::combined_light(ch, night_brightness),
        );
        renderer.upload_avatar_vertices(0, &skin_v);
        let mut cu = CameraUniform::from_camera(&camera);
        cu.sun_dir[3] = night_brightness;
        renderer.update_camera(0, &cu);
        renderer.render_to_png(&night_path);

        // Mean luminance of the day over-shoulder frame vs the night frame.
        let mean_luma = |path: &str| -> f32 {
            let img = image::open(path).expect("re-open rendered PNG").to_rgba8();
            let mut sum = 0.0_f64;
            let mut n = 0u64;
            for p in img.pixels() {
                sum += 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
                n += 1;
            }
            (sum / n as f64) as f32
        };
        let day_luma = mean_luma(&output_path);
        let night_luma = mean_luma(&night_path);
        let (_, night_magenta) = assess(&night_path);
        let dark_enough = night_luma < day_luma * 0.6;
        let avatar_dimmed = night_magenta < 50;
        let ok = dark_enough && avatar_dimmed;
        all_ok &= ok;
        log::info!(
            "       night-dim: {} | day_luma={day_luma:.1} night_luma={night_luma:.1} avatar_px={night_magenta} → {}",
            night_path,
            if ok { "PASS" } else { "FAIL" }
        );
    }

    // ── Phase 2 — no-snap collision smoke (orbit-behind, wall right behind) ──
    // Drop a STONE wall into the camera's pull-back path. It only needs to exist
    // in the world for the collision raycast (it ends up behind the clamped
    // camera, so no mesh upload is needed). Assert the camera CLAMPED (fraction <
    // 1) and that the frame still renders the avatar (proves the camera sat in
    // FRONT of the wall, not inside it — which would black out / occlude).
    {
        let walled_path = sib("play-3p-walled.png");
        let mut camera = crate::camera::Camera::new(eye, 1280.0 / 720.0);
        camera.yaw = 0.0;
        camera.pitch = -0.18;
        camera.mode = crate::camera::CameraMode::OrbitBehind;

        // Wall in the camera's pull-back path (orbit pulls toward +Z + up).
        let wz = eye.z.floor() as i32 + 2;
        let wy = eye.y.floor() as i32;
        for x in -1..=1 {
            for y in (wy - 1)..=(wy + 2) {
                world.set_block(x, y, wz, block::STONE);
            }
        }

        let frac = crate::raycast::camera_collision_fraction(&camera, &world, &registry);
        camera.collision_frac = frac;

        let fwd = camera.forward();
        ps.yaw = crate::entity_model::yaw_facing(fwd.x, fwd.z);
        // Opaque body (fade 1.0) — proves the camera clamped in front of the wall.
        let (skin_v, _held) =
            crate::entity_model::self_avatar_vertices(camera.mode, &ps, 0.0, 0, 0, 1.0, &registry, crate::skin_uv::ArmModel::Classic);
        renderer.upload_avatar_vertices(0, &skin_v);
        renderer.update_camera(0, &CameraUniform::from_camera(&camera));
        renderer.render_to_png(&walled_path);

        let (colours, magenta) = assess(&walled_path);
        let clamped = frac < 1.0; // the wall actually engaged the collision clamp
        let non_blank = colours > 12;
        let avatar_ok = magenta > 200; // body still framed → camera in front of the wall
        let ok = clamped && non_blank && avatar_ok;
        all_ok &= ok;
        log::info!(
            "    walled-orbit: {} | clamped={clamped} (frac={frac:.3}) non_blank={non_blank} ({colours} colours) avatar_px={magenta} → {}",
            walled_path,
            if ok { "PASS" } else { "FAIL" }
        );

        // ── Phase 3 — avatar fade-on-occlusion (same pulled-in scene) ────────
        // The camera is clamped close (frac<1) → the body would hide the aim, so
        // the self-avatar fades to a screen-door stipple. Render the SAME frame
        // with the computed fade and assert the avatar is partly see-through:
        // fewer body pixels than the opaque render, but still visible.
        let fade_path = sib("play-3p-fade.png");
        let fade = crate::camera::avatar_fade_alpha(frac);
        let (skin_fv, _h) =
            crate::entity_model::self_avatar_vertices(camera.mode, &ps, 0.0, 0, 0, fade, &registry, crate::skin_uv::ArmModel::Classic);
        renderer.upload_avatar_vertices(0, &skin_fv);
        renderer.update_camera(0, &CameraUniform::from_camera(&camera));
        renderer.render_to_png(&fade_path);

        let (fcolours, fmagenta) = assess(&fade_path);
        let faded = fade < 1.0; // the curve engaged a fade at this pull-in
        let thinned = fmagenta < magenta; // dither dropped some body pixels
        let still_visible = fmagenta > 200; // but the guy is still there
        let fok = faded && fcolours > 12 && thinned && still_visible;
        all_ok &= fok;
        log::info!(
            "     faded-orbit: {} | faded={faded} (alpha={fade:.3}) thinned={thinned} ({fmagenta} vs opaque {magenta}) still_visible={still_visible} → {}",
            fade_path,
            if fok { "PASS" } else { "FAIL" }
        );
    }

    if all_ok {
        log::info!("PASS — all camera modes rendered correctly (avatar shown only in third-person; no-snap collision clamps before a wall; fade thins the body when pulled in).");
    } else {
        log::error!("FAIL — see the per-mode lines above.");
        std::process::exit(1);
    }
}

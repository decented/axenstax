//! Engine-generic graphics / quality settings — the single source of truth for
//! render distance, render scale, FOV, mouse sensitivity, frame limit, and fog.
//!
//! Spec 39 (`docs/foundations/2026-06-02-gpu-performance-and-graphics-settings.md`)
//! + Spec 03 §"Graphics Settings". This replaces the hardcoded `RENDER_DISTANCE`
//!   consts (previously duplicated in `main.rs` + `server.rs`) and the hardcoded
//!   FOV / sensitivity / present-mode / fog literals with one persisted struct.
//!
//! Persistence is **per-device** (not per-world):
//!   - WASM: `localStorage` key `axenstax_gfx` (JSON).
//!   - Native: `settings.json` in the working directory, next to `worlds/`.
//!
//! Nothing here is AxeNStax-specific — it lifts to any Decented voxel game.
//!
//! ## Presets vs personal prefs
//!
//! **Quality** dials (render distance/scale, fog, mipmaps, particles, frame
//! limit) define the GPU load and therefore the *preset*. FOV
//! and mouse sensitivity are **personal preferences** that don't change GPU
//! load, so they are deliberately excluded from preset detection — changing
//! your FOV does not knock you off "High". The `preset()` label is *derived*
//! from the quality dials (no stored field to drift): if they exactly match a
//! named preset you're on that preset, otherwise `Custom`.

use serde::{Deserialize, Serialize};

/// Named quality presets. `Custom` is never stored — it is what `preset()`
/// returns when the quality dials match no named preset.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsPreset {
    Potato,
    Low,
    Medium,
    High,
    Ultra,
    Custom,
}

impl GraphicsPreset {
    /// The five selectable presets, in increasing-quality order (excludes Custom).
    pub const ALL: [GraphicsPreset; 5] = [
        GraphicsPreset::Potato,
        GraphicsPreset::Low,
        GraphicsPreset::Medium,
        GraphicsPreset::High,
        GraphicsPreset::Ultra,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GraphicsPreset::Potato => "Potato",
            GraphicsPreset::Low => "Low",
            GraphicsPreset::Medium => "Medium",
            GraphicsPreset::High => "High",
            GraphicsPreset::Ultra => "Ultra",
            GraphicsPreset::Custom => "Custom",
        }
    }
}

/// Frame-rate limiter. `VSync` ties to the display refresh (today's behaviour);
/// `Uncapped` presents as fast as the GPU allows; `Cap(n)` paces to n FPS.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameLimit {
    VSync,
    Uncapped,
    Cap(u16),
}

impl FrameLimit {
    /// FPS ceiling for the frame pacer, or `None` when the limiter is vsync /
    /// uncapped (no explicit software cap). The frame pacer that would use
    /// this is Phase 6 (see the note in game_loop.rs: "The software FPS cap
    /// (Cap(n)) is applied by the frame pacer (Phase 6)") — not built yet.
    #[allow(dead_code)]
    pub fn fps_cap(self) -> Option<u16> {
        match self {
            FrameLimit::Cap(n) => Some(n),
            FrameLimit::VSync | FrameLimit::Uncapped => None,
        }
    }

    /// No settings-UI dropdown lists `FrameLimit` options yet.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            FrameLimit::VSync => "VSync",
            FrameLimit::Uncapped => "Uncapped",
            FrameLimit::Cap(30) => "30 FPS",
            FrameLimit::Cap(60) => "60 FPS",
            FrameLimit::Cap(120) => "120 FPS",
            FrameLimit::Cap(_) => "Capped",
        }
    }

    /// Software frame-cap interval for the game-loop pacer (Spec 39 A3). `Some`
    /// only for an explicit FPS cap; `None` for VSync (paced by the swapchain
    /// `present`) and Uncapped (deliberately unlimited). The native loop sleeps
    /// off any unused budget, which also yields the CPU and stops the
    /// `ControlFlow::Poll` busy-spin.
    pub fn software_cap_interval(self) -> Option<std::time::Duration> {
        match self {
            FrameLimit::Cap(n) => Some(std::time::Duration::from_secs_f32(1.0 / n.max(1) as f32)),
            FrameLimit::VSync | FrameLimit::Uncapped => None,
        }
    }
}

/// Particle density. `Off`/`Reduced` are the weak-GPU savings; `Full` is today.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleLevel {
    Off,
    Reduced,
    Full,
}

// ---------------------------------------------------------------------------
// Tunable ranges (clamped on load + by the UI sliders)
// ---------------------------------------------------------------------------

pub const RENDER_DISTANCE_MIN: i32 = 4;
pub const RENDER_DISTANCE_MAX: i32 = 16;
pub const RENDER_SCALE_MIN: f32 = 0.25;
pub const RENDER_SCALE_MAX: f32 = 1.0;
pub const FOV_MIN: f32 = 60.0;
pub const FOV_MAX: f32 = 100.0;
/// Hold-to-zoom FOV range (#44). Narrower than `FOV_MIN` on purpose — zoom is a
/// transient aiming aid that bypasses the personal-FOV clamp.
pub const ZOOM_FOV_MIN: f32 = 15.0;
pub const ZOOM_FOV_MAX: f32 = 45.0;
/// Default hold-to-zoom FOV (≈ `DEFAULT_FOV / 3.5`, OptiFine-ish narrow view).
pub const DEFAULT_ZOOM_FOV: f32 = 20.0;
pub const SENSITIVITY_MIN: f32 = 0.0005;
pub const SENSITIVITY_MAX: f32 = 0.01;

/// Brightness control range (graphics-menu slider). 1.0 = neutral; >1 brightens
/// (lifts shadows — the player-side answer to "it is too dark"), <1 darkens. The
/// shader applies it as `pow(colour, 1/brightness)`. A personal preference, like
/// FOV/sensitivity — it does NOT change GPU load, so it never flips the preset.
pub const BRIGHTNESS_MIN: f32 = 0.6;
pub const BRIGHTNESS_MAX: f32 = 2.0;
pub const DEFAULT_BRIGHTNESS: f32 = 1.0;
/// Master volume slider range (`0.0` = silent, `1.0` = sounds as authored).
/// The audio engine squares it (`audio::effective_gain`) so the lower half of
/// the slider is usable.
pub const MASTER_VOLUME_MIN: f32 = 0.0;
pub const MASTER_VOLUME_MAX: f32 = 1.0;
pub const DEFAULT_MASTER_VOLUME: f32 = 1.0;

/// Minimap zoom = world blocks per map pixel. Lower = more zoomed in (fewer
/// blocks per pixel). A personal preference like brightness — it never flips the
/// preset. Default shows ~the texture size in blocks across (≈10 chunks). #6.
pub const MINIMAP_ZOOM_MIN: f32 = 0.5;
pub const MINIMAP_ZOOM_MAX: f32 = 4.0;
pub const DEFAULT_MINIMAP_ZOOM: f32 = 1.0;

/// The default mouse-look multiplier. Lowered 0.003 → 0.002 (2026-06-16 playtest:
/// Axolittle "look around too fast") for a calmer default turn rate; players who
/// want faster can raise it on the Settings slider (`SENSITIVITY_MIN..=MAX`).
/// Feel-tunable.
pub const DEFAULT_SENSITIVITY: f32 = 0.002;

/// The mouse-sensitivity value shipped before the 2026-06-16 playtest fix
/// (see `DEFAULT_SENSITIVITY`'s history). Used by [`migrate`] to detect a
/// persisted profile that predates the change.
const LEGACY_SENSITIVITY_DEFAULT: f32 = 0.003;

/// Default vertical FOV in degrees (Minecraft default, matches `Camera::new`).
pub const DEFAULT_FOV: f32 = 70.0;

// Phase 6 — third-person camera feel ranges (live-tunable sliders). Defaults are
// the flagged feel guesses; the Axolittle playtest tunes them in-game.
/// Third-person pull-back zoom multiplier (1.0 = the built-in distance).
pub const TP_DISTANCE_MIN: f32 = 0.5;
pub const TP_DISTANCE_MAX: f32 = 2.0;
/// Free-look mouse-sensitivity multiplier (on top of the base look sensitivity).
pub const FREELOOK_SENS_MIN: f32 = 0.25;
pub const FREELOOK_SENS_MAX: f32 = 3.0;
/// Auto-centre post-input delay (seconds) before the camera re-centres behind you.
pub const AUTO_CENTRE_DELAY_MIN: f32 = 0.0;
pub const AUTO_CENTRE_DELAY_MAX: f32 = 3.0;
/// Auto-centre ease speed (orbit radians/second eased back behind).
pub const AUTO_CENTRE_SPEED_MIN: f32 = 0.5;
pub const AUTO_CENTRE_SPEED_MAX: f32 = 8.0;

/// The canonical default render distance (High preset). Single source of truth
/// for the value that used to live as `const RENDER_DISTANCE = 10` in both
/// `main.rs` and `server.rs`.
pub const DEFAULT_RENDER_DISTANCE: i32 = 10;

/// Pixel budgets (physical pixels of the 3D world) the presets carry. High and
/// Ultra carry none (uncapped). 0.9 M is roughly 1280x720; 1.3 M a little
/// under 1600x900; 2.1 M about 1920x1080.
pub const POTATO_PIXEL_BUDGET: u32 = 900_000;
pub const LOW_PIXEL_BUDGET: u32 = 1_300_000;
pub const MEDIUM_PIXEL_BUDGET: u32 = 2_100_000;
/// Floor for a hand-edited budget: below this even a 0.25 scale of a small
/// window would be pointless, and `0` would divide by nothing.
pub const PIXEL_BUDGET_MIN: u32 = 100_000;

/// Pure: the render scale actually used for the 3D world.
/// `min(render_scale, sqrt(budget / physical_pixels))`, clamped into
/// `RENDER_SCALE_MIN..=RENDER_SCALE_MAX`. `physical_pixels` is the surface size
/// in device pixels (so a DPR-3 phone counts 9x its CSS area). `None` budget =
/// the user's scale unchanged (clamped).
pub fn budgeted_scale(render_scale: f32, pixel_budget: Option<u32>, width: u32, height: u32) -> f32 {
    let wanted = render_scale.clamp(RENDER_SCALE_MIN, RENDER_SCALE_MAX);
    let Some(budget) = pixel_budget else {
        return wanted;
    };
    let pixels = (width.max(1) as f64) * (height.max(1) as f64);
    let cap = ((budget as f64) / pixels).sqrt() as f32;
    wanted.min(cap).clamp(RENDER_SCALE_MIN, RENDER_SCALE_MAX)
}

/// Pure: the preset a first run starts on, from what the platform reveals.
/// Order matters: a software rasteriser is Potato whatever else is true; a touch
/// device (Android, or a browser reporting touch points) is Low; an integrated
/// GPU is Low in a browser and Medium natively; a discrete GPU is High; anything
/// unreported (a browser usually hides the adapter class, so `Other`) or a
/// virtual GPU is Medium.
pub fn recommend_preset(device_type: wgpu::DeviceType, is_touch: bool, is_web: bool) -> GraphicsPreset {
    use wgpu::DeviceType;
    if device_type == DeviceType::Cpu {
        return GraphicsPreset::Potato;
    }
    if is_touch {
        return GraphicsPreset::Low;
    }
    match device_type {
        DeviceType::IntegratedGpu if is_web => GraphicsPreset::Low,
        DeviceType::IntegratedGpu => GraphicsPreset::Medium,
        DeviceType::DiscreteGpu => GraphicsPreset::High,
        _ => GraphicsPreset::Medium,
    }
}

/// Web frame-cap slack: rAF fires on the display's own cadence, so a frame that
/// arrives a hair early (rAF jitter) still counts as due. Without it a 60 FPS
/// cap on a 60 Hz display would skip every other frame.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const WEB_CAP_SLACK: std::time::Duration = std::time::Duration::from_millis(2);

/// Pure: should a render that arrives `elapsed` after the previous one go ahead
/// under a software cap of `interval`? `false` = skip it and ask for another
/// redraw. The slack is `min(WEB_CAP_SLACK, interval / 8)` so tiny intervals
/// keep a proportionate margin.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn cap_frame_due(elapsed: std::time::Duration, interval: std::time::Duration) -> bool {
    let slack = WEB_CAP_SLACK.min(interval / 8);
    elapsed + slack >= interval
}

/// Per-device graphics settings. The single source of truth for all quality
/// dials. Default = the **High** preset, i.e. today's look, so a fresh install
/// or an old/partial settings file behaves exactly as before.
///
/// Not `Copy` — `online_relays: Vec<String>` (spec §6) prevents it. Every
/// existing caller already held this by reference or moved it once, so
/// dropping `Copy` needed no call-site changes (verified 2026-09-07).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct GraphicsSettings {
    /// View radius in chunks (4..=16). Drives chunk streaming + fog.
    pub render_distance: i32,
    /// Internal-resolution multiplier (0.25..=1.0). Renders the world to an
    /// offscreen target at this scale then blits up — the biggest weak-GPU lever.
    pub render_scale: f32,
    /// Vertical field of view in degrees (60..=100). Personal pref.
    pub fov_y: f32,
    /// Mouse-look sensitivity multiplier. Personal pref.
    pub mouse_sensitivity: f32,
    /// Frame-rate limiter (vsync / uncapped / capped).
    pub frame_limit: FrameLimit,
    /// Distance fog on/off. When on, fog distances are *derived from render
    /// distance* (so the two always track — fixes the pop-in bug). Off = clip
    /// at the far plane with no fog.
    pub fog: bool,
    /// Spec 39 A6 — mip chain + mip filtering on the block texture array
    /// (smooths distant/grazing block faces; costs a CPU mip build on toggle and
    /// ~33% more atlas VRAM). **Opt-in — false in every preset, High included**,
    /// so the shipped look is unchanged until a human decides the distant look is
    /// better with it on. See `docs/spec/03-rendering.md` §9.4b.
    pub mipmaps: bool,
    /// Particle density.
    pub particles: ParticleLevel,
    /// Cap on the 3D world's rendered pixel count (physical pixels, so it
    /// includes the display scale factor). `None` = uncapped, exactly today's
    /// behaviour, which is what an old settings file (no key) deserialises to.
    /// The effective render scale is `min(render_scale, sqrt(budget / physical
    /// pixels))` (see [`budgeted_scale`]); a quality dial set by the presets
    /// (Potato 0.9 M, Low 1.3 M, Medium 2.1 M, High and Ultra none). A file
    /// with no such key takes the budget of the preset its other dials match
    /// (`parse_persisted`).
    #[serde(default)]
    pub pixel_budget: Option<u32>,
    /// "Leaves: Fast" (Perf B2): leaves render as opaque cubes, with interior
    /// faces culled and their cutout holes filled with a darkened leaf colour,
    /// instead of the see-through cutout pass. A quality dial set by the presets
    /// (Potato and Low on; Medium, High and Ultra off). A file with no such key
    /// takes the value of the preset its other dials match, else off
    /// (`parse_persisted`). Toggling remeshes the loaded world.
    #[serde(default)]
    pub leaves_fast: bool,
    /// Auto quality (Spec 03 §10.3): a runtime governor may hold render
    /// distance, render scale and particles temporarily BELOW the values above
    /// when frames run long. It never raises past them and never writes its
    /// offset to the file. Off for any existing player (an old file has no key);
    /// switched on when first-run detection picks a device-aware default.
    #[serde(default)]
    pub auto_quality: bool,
    /// Phase 3 (third-person camera) — fade the self-avatar to a screen-door
    /// stipple when the camera is pulled in close, so the body never hides the
    /// crosshair target. On by default; a comfort toggle (not a quality dial), so
    /// it's preset-independent. The Phase-6 settings panel exposes the switch.
    pub avatar_fade: bool,
    /// Phase 6 — the perspective a freshly-loaded world starts in (the F5 toggle
    /// updates this so your last view is remembered across reloads/sessions).
    pub default_camera_mode: crate::camera::CameraMode,
    /// Phase 6 — third-person pull-back zoom multiplier (`TP_DISTANCE_*` range).
    pub third_person_distance: f32,
    /// Phase 6 — enable the decoupled free-look modifier (hold Alt to look around
    /// without re-aiming) + velocity-gated auto-recentre. Off preserves the
    /// approved coupled-camera feel.
    pub third_person_freelook: bool,
    /// Phase 6 — free-look mouse-sensitivity multiplier (`FREELOOK_SENS_*`).
    pub freelook_sensitivity: f32,
    /// Phase 6 — auto-centre post-input delay in seconds (`AUTO_CENTRE_DELAY_*`).
    pub auto_centre_delay: f32,
    /// Phase 6 — auto-centre ease speed (`AUTO_CENTRE_SPEED_*`).
    pub auto_centre_speed: f32,
    /// #44 — persisted default for the F3 debug overlay. Off by default; ticking
    /// this in Settings makes the overlay discoverable without knowing F3, and
    /// it becomes the on-load default. F3 still toggles it live in-session.
    pub show_debug_hud: bool,
    /// #44 — transient hold-to-zoom field of view in degrees. Deliberately
    /// outside the `FOV_MIN..=FOV_MAX` personal-FOV clamp (zoom is an aiming aid,
    /// not a comfort dial) — clamped to `ZOOM_FOV_MIN..=ZOOM_FOV_MAX`.
    pub zoom_fov: f32,
    /// #45 P4 — auto-refill an exhausted hotbar block/stack from the bag. On by
    /// default (it never surprises: it only refills a slot you just emptied, with
    /// the same item). Synced to each player's `Inventory::auto_refill`.
    pub auto_refill: bool,
    /// Brightness control (graphics-menu slider). 1.0 = neutral; >1 brightens,
    /// <1 darkens. Fed into the camera uniform (`params.x`) and applied in the
    /// shader as `pow(colour, 1/brightness)`. Personal pref — excluded from
    /// preset detection. Clamped to `BRIGHTNESS_MIN..=BRIGHTNESS_MAX` on load.
    pub brightness: f32,
    /// #6 — show the corner minimap. On by default (discoverable). Personal
    /// pref, excluded from preset detection.
    pub minimap_enabled: bool,
    /// #6 — minimap zoom in world blocks per map pixel. Personal pref. Clamped
    /// to `MINIMAP_ZOOM_MIN..=MINIMAP_ZOOM_MAX` on load.
    pub minimap_zoom: f32,
    /// #24 — accessibility narration (Web Speech TTS for hotbar/status). Off by
    /// default (opt-in). Personal pref, excluded from preset detection.
    pub narration_enabled: bool,
    /// Build Schematics — whether this device has seen the one-time CC-BY-SA
    /// licence onboarding modal that precedes the first blueprint capture. Lives
    /// here (per-device, cross-platform persisted) rather than on per-world
    /// `WorldMeta` so it survives a web reload: world meta isn't persisted with
    /// the flag on WASM, which made the modal re-block the capture dialog every
    /// session ("i can't save any blueprints", #blueprint-save). Off by default.
    pub has_seen_license_onboarding: bool,
    /// "Your relays" — the player's ONE relay list (Spec 04 §1.9). Everything
    /// the native app connects to on the player's behalf reads it: the
    /// sign-in QR (first 3), online-play setup (rendezvous), Server Card
    /// discovery, Signet contacts pairing (first one) and the signed release
    /// feed. **Setup only**: no relay ever carries a byte of game traffic
    /// (CLAUDE.md red line 2). Feedback is NOT on this list — it goes to the
    /// project's fixed inbox (`native_mailbox::FEEDBACK_INBOX_RELAYS`).
    /// Edited through `relays_ui` (reachable before sign-in) so a household
    /// can choose relays it trusts. `serde(default)` so an existing
    /// settings.json still loads.
    #[serde(default = "default_online_relays")]
    pub online_relays: Vec<String>,
    /// UDP port to bind when hosting online. `0` = let the OS choose an
    /// ephemeral port, which is fine (the port travels inside the candidates)
    /// but makes a manual router forward impossible, so the default is the
    /// familiar `protocol::SERVER_PORT`.
    #[serde(default = "default_online_port")]
    pub online_port: u16,
    /// Alpha-tester feedback (`/bug`, `/idea`, `/mailbox`) — OFF by default.
    /// Flipped on by the hidden unlock in the Settings panel (tap the version
    /// line 7 times) and off again by its checkbox. Read ONLY through
    /// `native_mailbox::feedback_enabled`, never directly, so a future
    /// age/guardian gate has one place to land. Native only in effect (the web
    /// build has no feedback channel); the field itself is cross-platform so
    /// the settings file shape is identical. `serde(default)` = an existing
    /// settings.json loads with feedback off.
    #[serde(default)]
    pub tester_feedback: bool,
    /// Master volume for every sound effect, `MASTER_VOLUME_MIN..=MASTER_VOLUME_MAX`.
    /// Per-device (native `settings.json`, web localStorage) and applied on
    /// BOTH targets via `AudioEngine::set_master`. Personal pref, excluded from
    /// preset detection. Clamped on load.
    pub master_volume: f32,
    /// Mute switch — silences all sound without losing the volume setting.
    pub audio_muted: bool,
    /// Whether this device has already been shown the one-time controls card
    /// (first spawn). Per-device like `has_seen_license_onboarding`, so it
    /// survives a web reload. The pause menu's "Controls" button reopens the
    /// card any time regardless. Off by default.
    pub controls_card_seen: bool,
}

/// The shipped relay set for the rendezvous handshake: public third-party
/// relays only (spec §1 red line 2: we operate no relay that a group's setup
/// depends on). Players can still add any relay, ours included.
pub fn default_online_relays() -> Vec<String> {
    crate::server_resolve::public_default_relays()
}

/// The pre-launch default, which listed our own `relay.trotters.cc` first. A
/// saved list that still equals it exactly was never customised, so it is
/// migrated to the public default on load; any edited list is left alone.
const PRE_LAUNCH_ONLINE_RELAYS: [&str; 4] = [
    "wss://relay.trotters.cc",
    "wss://nos.lol",
    "wss://relay.damus.io",
    "wss://relay.primal.net",
];

/// Same as `protocol::SERVER_PORT` — the port a LAN host already uses, so a
/// household that has forwarded it once has forwarded it for both.
pub const DEFAULT_ONLINE_PORT: u16 = 7700;

fn default_online_port() -> u16 {
    DEFAULT_ONLINE_PORT
}

/// The most relays "Your relays" holds.
pub const MAX_RELAYS: usize = 8;

/// Trim, drop anything that isn't `wss://`, de-duplicate, and cap at
/// [`MAX_RELAYS`] (the same ceiling `invite::MAX_RELAYS` enforces on a pasted
/// link, so a host can never mint an invite its own settings would reject).
pub fn sanitise_relays(relays: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in relays {
        let r = r.trim().to_string();
        if !r.starts_with("wss://") || out.contains(&r) {
            continue;
        }
        out.push(r);
        if out.len() == MAX_RELAYS {
            break;
        }
    }
    out
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        // Default = High preset == today's look (unchanged behaviour).
        Self::from_preset(GraphicsPreset::High)
    }
}

impl GraphicsSettings {
    /// Build the settings for a named preset. `Custom` maps to `High` (there is
    /// no canonical "custom" — the caller keeps the user's existing dials).
    ///
    /// FOV + sensitivity are personal prefs and are set to their defaults here;
    /// callers applying a preset to live settings should preserve the user's
    /// current FOV/sensitivity (see [`apply_preset`]). This pure function exists
    /// so preset detection has a canonical comparison target.
    pub fn from_preset(preset: GraphicsPreset) -> Self {
        let (render_distance, render_scale, frame_limit, fog, mipmaps, particles, pixel_budget, leaves_fast) =
            match preset {
                GraphicsPreset::Potato => (
                    4,
                    0.5,
                    FrameLimit::Cap(30),
                    false,
                    false,
                    ParticleLevel::Off,
                    Some(POTATO_PIXEL_BUDGET),
                    true,
                ),
                GraphicsPreset::Low => (
                    5,
                    0.75,
                    FrameLimit::Cap(60),
                    true,
                    false,
                    ParticleLevel::Reduced,
                    Some(LOW_PIXEL_BUDGET),
                    true,
                ),
                GraphicsPreset::Medium => (
                    7,
                    1.0,
                    FrameLimit::Cap(60),
                    true,
                    false,
                    ParticleLevel::Full,
                    Some(MEDIUM_PIXEL_BUDGET),
                    false,
                ),
                // High == today's values → default look is unchanged.
                GraphicsPreset::High | GraphicsPreset::Custom => (
                    DEFAULT_RENDER_DISTANCE,
                    1.0,
                    FrameLimit::VSync,
                    true,
                    false,
                    ParticleLevel::Full,
                    None,
                    false,
                ),
                GraphicsPreset::Ultra => (
                    14,
                    1.0,
                    FrameLimit::Uncapped,
                    true,
                    false,
                    ParticleLevel::Full,
                    None,
                    false,
                ),
            };
        Self {
            render_distance,
            render_scale,
            fov_y: DEFAULT_FOV,
            mouse_sensitivity: DEFAULT_SENSITIVITY,
            frame_limit,
            fog,
            mipmaps,
            particles,
            pixel_budget,
            leaves_fast,
            auto_quality: false,
            // Comfort toggle — on for every preset (Phase-6 panel flips it).
            avatar_fade: true,
            // Phase 6 — camera prefs are preset-independent; defaults preserve
            // the approved feel (first-person, no zoom, free-look off). The
            // auto-centre defaults mirror the camera consts (feel guesses).
            default_camera_mode: crate::camera::CameraMode::FirstPerson,
            third_person_distance: 1.0,
            third_person_freelook: false,
            freelook_sensitivity: 1.0,
            auto_centre_delay: crate::camera::RECENTER_COOLDOWN,
            auto_centre_speed: crate::camera::RECENTER_SPEED,
            // #44 — debug overlay off by default; zoom FOV at the narrow default.
            // Both are preset-independent personal prefs.
            show_debug_hud: false,
            zoom_fov: DEFAULT_ZOOM_FOV,
            // #45 — auto-refill on by default (only ever refills a just-emptied
            // slot with the same item, so it can't surprise the player).
            auto_refill: true,
            // Neutral brightness — the look is unchanged until the player slides it.
            brightness: DEFAULT_BRIGHTNESS,
            // #6 — minimap on by default; default zoom shows ~10 chunks across.
            minimap_enabled: true,
            minimap_zoom: DEFAULT_MINIMAP_ZOOM,
            // #24 — narration off by default (opt-in accessibility aid).
            narration_enabled: false,
            has_seen_license_onboarding: false,
            online_relays: default_online_relays(),
            online_port: DEFAULT_ONLINE_PORT,
            tester_feedback: false,
            master_volume: DEFAULT_MASTER_VOLUME,
            audio_muted: false,
            controls_card_seen: false,
        }
    }

    /// Apply a preset to live settings. Only the quality dials compared by
    /// `quality_eq` change; everything else (FOV, sensitivity, relays, port,
    /// the tester unlock, accessibility toggles) is the player's own and stays.
    pub fn apply_preset(&mut self, preset: GraphicsPreset) {
        let p = Self::from_preset(preset);
        self.render_distance = p.render_distance;
        self.render_scale = p.render_scale;
        self.frame_limit = p.frame_limit;
        self.fog = p.fog;
        self.mipmaps = p.mipmaps;
        self.particles = p.particles;
        self.pixel_budget = p.pixel_budget;
        self.leaves_fast = p.leaves_fast;
    }

    /// Derive the preset label from the **quality** dials. Returns `Custom` when
    /// no named preset matches. FOV + sensitivity are excluded (personal prefs),
    /// so changing them never flips the preset.
    pub fn preset(&self) -> GraphicsPreset {
        for p in GraphicsPreset::ALL {
            if self.quality_eq(&Self::from_preset(p)) {
                return p;
            }
        }
        GraphicsPreset::Custom
    }

    /// Compare only the quality-load fields (everything except FOV + sensitivity).
    /// The pixel budget and "Leaves: Fast" are among them: a preset means its
    /// budget and its leaves too.
    fn quality_eq(&self, other: &Self) -> bool {
        self.dials_eq(other)
            && self.pixel_budget == other.pixel_budget
            && self.leaves_fast == other.leaves_fast
    }

    /// The quality dials a settings file written before `pixel_budget` /
    /// `leaves_fast` existed can have: everything `quality_eq` compares except
    /// those two.
    fn dials_eq(&self, other: &Self) -> bool {
        self.render_distance == other.render_distance
            && (self.render_scale - other.render_scale).abs() < 1e-4
            && self.frame_limit == other.frame_limit
            && self.fog == other.fog
            && self.mipmaps == other.mipmaps
            && self.particles == other.particles
    }

    /// Clamp every dial into its valid range. Run on load so a hand-edited or
    /// corrupt settings file can never push the engine into a bad state.
    pub fn clamp(&mut self) {
        self.render_distance = self
            .render_distance
            .clamp(RENDER_DISTANCE_MIN, RENDER_DISTANCE_MAX);
        self.render_scale = self.render_scale.clamp(RENDER_SCALE_MIN, RENDER_SCALE_MAX);
        self.pixel_budget = self.pixel_budget.map(|b| b.max(PIXEL_BUDGET_MIN));
        self.fov_y = self.fov_y.clamp(FOV_MIN, FOV_MAX);
        self.mouse_sensitivity = self
            .mouse_sensitivity
            .clamp(SENSITIVITY_MIN, SENSITIVITY_MAX);
        // Phase 6 — third-person camera dials.
        self.third_person_distance = self.third_person_distance.clamp(TP_DISTANCE_MIN, TP_DISTANCE_MAX);
        self.freelook_sensitivity = self.freelook_sensitivity.clamp(FREELOOK_SENS_MIN, FREELOOK_SENS_MAX);
        self.auto_centre_delay = self.auto_centre_delay.clamp(AUTO_CENTRE_DELAY_MIN, AUTO_CENTRE_DELAY_MAX);
        self.auto_centre_speed = self.auto_centre_speed.clamp(AUTO_CENTRE_SPEED_MIN, AUTO_CENTRE_SPEED_MAX);
        // #44 — zoom FOV has its own (narrower) range, separate from `fov_y`.
        self.zoom_fov = self.zoom_fov.clamp(ZOOM_FOV_MIN, ZOOM_FOV_MAX);
        // Brightness slider — clamp so a hand-edited file can't blow out / black out.
        self.brightness = self.brightness.clamp(BRIGHTNESS_MIN, BRIGHTNESS_MAX);
        // #6 — minimap zoom (blocks per pixel).
        self.minimap_zoom = self.minimap_zoom.clamp(MINIMAP_ZOOM_MIN, MINIMAP_ZOOM_MAX);
        // Master volume — a hand-edited file can't amplify or go negative.
        self.master_volume = self.master_volume.clamp(MASTER_VOLUME_MIN, MASTER_VOLUME_MAX);
        // An empty or all-rubbish relay list would make online play silently
        // impossible, so repair rather than accept it.
        self.online_relays = sanitise_relays(std::mem::take(&mut self.online_relays));
        if self.online_relays.iter().map(String::as_str).eq(PRE_LAUNCH_ONLINE_RELAYS) {
            self.online_relays = default_online_relays();
        }
        if self.online_relays.is_empty() {
            self.online_relays = default_online_relays();
        }
    }

    // -- Derived render parameters ------------------------------------------

    /// Fog start/end distances **in blocks**, derived from render distance so
    /// fog always fades out just before the chunk-load edge (no pop-in). At the
    /// historical default (render distance 10) this returns `(128.0, 160.0)` —
    /// exactly the old hardcoded `shader.wgsl` literals, so the look is unchanged.
    ///
    /// When `fog` is off the caller should instead push a very-far pair (or a
    /// flag) so nothing fades; this helper only describes the *on* curve.
    /// The live caller (camera.rs) calls the free function `fog_distances`
    /// directly with `render_distance` rather than through this convenience
    /// method, so this one has no caller.
    #[allow(dead_code)]
    pub fn fog_distances(&self) -> (f32, f32) {
        fog_distances(self.render_distance)
    }
}

/// Pure: fog (start, end) in blocks for a given render distance. `end` is the
/// load edge (render_distance × chunk-width-16); `start` is two chunks closer.
/// render_distance 10 → (128, 160), matching the legacy shader constants.
pub fn fog_distances(render_distance: i32) -> (f32, f32) {
    let end = (render_distance as f32) * 16.0;
    let start = ((render_distance - 2).max(1) as f32) * 16.0;
    (start, end)
}

// ---------------------------------------------------------------------------
// Persistence — per-device. Native: settings.json. WASM: localStorage.
// ---------------------------------------------------------------------------

impl GraphicsSettings {
    /// Load persisted settings, falling back to the default (High) when absent
    /// or unparseable. Migrated (see [`migrate`]), then always clamped.
    pub fn load() -> Self {
        Self::load_or_first_run(None)
    }

    /// Load persisted settings; when none have ever been stored (no file on
    /// native, no `localStorage` key on web) and a `first_run` preset is given,
    /// start from that preset with `auto_quality` on instead of High. An
    /// existing file, even an unparseable one, is never re-defaulted.
    pub fn load_or_first_run(first_run: Option<GraphicsPreset>) -> Self {
        let raw = load_raw();
        let mut s = match (&raw, first_run) {
            (None, Some(preset)) => {
                let mut s = Self::from_preset(preset);
                s.auto_quality = true;
                s
            }
            _ => raw
                .and_then(|json| parse_persisted(&json))
                .unwrap_or_default(),
        };
        migrate(&mut s);
        s.clamp();
        s
    }

    /// True when nothing has been persisted yet (a first run).
    pub fn is_first_run() -> bool {
        load_raw().is_none()
    }

    /// Persist these settings (best-effort — persistence failure never breaks
    /// the game; the in-memory settings still apply this session).
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            save_raw(&json);
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const STORAGE_KEY: &str = "axenstax_gfx";

#[cfg(not(target_arch = "wasm32"))]
fn settings_path() -> std::path::PathBuf {
    crate::data_dir::data_root().join("settings.json")
}

#[cfg(not(target_arch = "wasm32"))]
fn load_raw() -> Option<String> {
    std::fs::read_to_string(settings_path()).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn save_raw(json: &str) {
    let _ = std::fs::write(settings_path(), json);
}

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
fn load_raw() -> Option<String> {
    local_storage()?.get_item(STORAGE_KEY).ok()?
}

#[cfg(target_arch = "wasm32")]
fn save_raw(json: &str) {
    if let Some(store) = local_storage() {
        let _ = store.set_item(STORAGE_KEY, json);
    }
}

/// One-shot migration for a value carried over from an old persisted profile.
///
/// **Sensitivity migration (2026-09-06)**: `DEFAULT_SENSITIVITY` was lowered
/// 0.003 → 0.002 on 2026-06-16 (Axolittle playtest: "look around too fast"),
/// but a profile already saved to native `settings.json` or web localStorage
/// (`axenstax_gfx`) keeps whatever value it was written with — new-install
/// players got the calmer default, existing players stayed on the old feel
/// forever. If the persisted `mouse_sensitivity` still equals the legacy
/// default (within a small float tolerance), move it onto the current
/// default. A value that merely happens to differ from both is a deliberate
/// player choice and is left untouched. Pure, so it's testable without
/// touching the filesystem; `load()` calls it after parse and before clamp.
fn migrate(settings: &mut GraphicsSettings) {
    if (settings.mouse_sensitivity - LEGACY_SENSITIVITY_DEFAULT).abs() < 1e-6 {
        settings.mouse_sensitivity = DEFAULT_SENSITIVITY;
    }
}

/// Parse a persisted settings blob. A file written before `pixel_budget` or
/// `leaves_fast` existed (no such key) takes that dial from the preset its
/// other dials match (an old Low file is Low, 1.3 M and fast leaves included, so
/// its label does not change), or off / no budget when it matches none. A file
/// that has the key is taken as written.
///
/// A file that has `pixel_budget` but not `leaves_fast` (written between the two
/// dials) matches a preset on its dials AND its budget, so a budget the player
/// chose by hand does not borrow a preset's leaves.
fn parse_persisted(json: &str) -> Option<GraphicsSettings> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let had_budget = value.get("pixel_budget").is_some();
    let had_leaves = value.get("leaves_fast").is_some();
    let mut s: GraphicsSettings = serde_json::from_value(value).ok()?;
    let matching = |s: &GraphicsSettings, with_budget: bool| {
        GraphicsPreset::ALL
            .into_iter()
            .map(GraphicsSettings::from_preset)
            .find(|p| s.dials_eq(p) && (!with_budget || s.pixel_budget == p.pixel_budget))
    };
    if !had_budget {
        s.pixel_budget = matching(&s, false).and_then(|p| p.pixel_budget);
    }
    if !had_leaves {
        s.leaves_fast = matching(&s, had_budget).is_some_and(|p| p.leaves_fast);
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_high_and_matches_today() {
        let s = GraphicsSettings::default();
        assert_eq!(s.preset(), GraphicsPreset::High);
        // Today's hardcoded values must be unchanged at the default.
        assert_eq!(s.render_distance, 10);
        assert_eq!(s.render_scale, 1.0);
        assert_eq!(s.fov_y, 70.0);
        // Lowered 0.003 → 0.002 (2026-06-16 playtest: "look around too fast").
        assert_eq!(s.mouse_sensitivity, 0.002);
        assert_eq!(s.frame_limit, FrameLimit::VSync);
        assert!(s.fog);
    }

    #[test]
    fn mipmaps_are_opt_in_on_every_preset() {
        // Spec 39 A6 — the dial is opt-in: no preset (High/Ultra included) may
        // turn it on, so the default look stays exactly what shipped, and a
        // fresh profile still detects as High.
        for p in GraphicsPreset::ALL {
            assert!(
                !GraphicsSettings::from_preset(p).mipmaps,
                "{p:?} must ship with mipmaps off"
            );
        }
        assert!(!GraphicsSettings::default().mipmaps);
        assert_eq!(GraphicsSettings::default().preset(), GraphicsPreset::High);
    }

    #[test]
    fn turning_mipmaps_on_is_a_custom_profile() {
        // It is a quality dial, so it must move the preset label off High.
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::High);
        s.mipmaps = true;
        assert_eq!(s.preset(), GraphicsPreset::Custom);
    }

    #[test]
    fn every_preset_round_trips_through_detection() {
        for p in GraphicsPreset::ALL {
            let s = GraphicsSettings::from_preset(p);
            assert_eq!(s.preset(), p, "{:?} should detect as itself", p);
        }
    }

    #[test]
    fn presets_are_distinct_in_quality() {
        // No two presets share the same quality fingerprint (else detection
        // would be ambiguous and the earlier-listed preset would always win).
        let all: Vec<_> = GraphicsPreset::ALL
            .iter()
            .map(|&p| GraphicsSettings::from_preset(p))
            .collect();
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i != j {
                    assert!(!a.quality_eq(b), "presets {i} and {j} are quality-identical");
                }
            }
        }
    }

    #[test]
    fn touching_a_quality_dial_flips_to_custom() {
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::High);
        s.render_distance = 8; // not any preset's distance
        assert_eq!(s.preset(), GraphicsPreset::Custom);
    }

    #[test]
    fn changing_fov_or_sensitivity_does_not_change_preset() {
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::Medium);
        s.fov_y = 95.0;
        s.mouse_sensitivity = 0.007;
        assert_eq!(
            s.preset(),
            GraphicsPreset::Medium,
            "personal prefs must not affect preset"
        );
    }

    #[test]
    fn brightness_defaults_neutral() {
        // Fresh settings must be brightness-neutral so the deployed look is
        // unchanged until the player moves the slider.
        assert_eq!(GraphicsSettings::default().brightness, 1.0);
        assert_eq!(DEFAULT_BRIGHTNESS, 1.0);
    }

    #[test]
    fn brightness_clamps_into_range() {
        let mut s = GraphicsSettings::default();
        s.brightness = 99.0;
        s.clamp();
        assert_eq!(s.brightness, BRIGHTNESS_MAX);
        s.brightness = -5.0;
        s.clamp();
        assert_eq!(s.brightness, BRIGHTNESS_MIN);
    }

    #[test]
    fn changing_brightness_does_not_change_preset() {
        // Brightness is a personal pref (like FOV/sensitivity) — it must not flip
        // the quality preset to Custom.
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::High);
        s.brightness = 1.6;
        assert_eq!(s.preset(), GraphicsPreset::High);
    }

    #[test]
    fn migrate_replaces_legacy_sensitivity_default() {
        // A profile persisted before 2026-06-16 still carries the old
        // shipped default (0.003) — migrate it onto the new default (0.002)
        // rather than leaving players stuck on the old feel forever.
        let mut s = GraphicsSettings::default();
        s.mouse_sensitivity = LEGACY_SENSITIVITY_DEFAULT;
        migrate(&mut s);
        assert_eq!(s.mouse_sensitivity, DEFAULT_SENSITIVITY);
    }

    #[test]
    fn migrate_leaves_a_deliberately_chosen_value_alone() {
        // A value that just happens to differ from both defaults must be a
        // player's real choice — migration must never touch it.
        let mut s = GraphicsSettings::default();
        s.mouse_sensitivity = 0.0045;
        migrate(&mut s);
        assert_eq!(s.mouse_sensitivity, 0.0045);
    }

    #[test]
    fn migrate_leaves_the_current_default_alone() {
        let mut s = GraphicsSettings::default();
        s.mouse_sensitivity = DEFAULT_SENSITIVITY;
        migrate(&mut s);
        assert_eq!(s.mouse_sensitivity, DEFAULT_SENSITIVITY);
    }

    #[test]
    fn load_migrates_a_legacy_persisted_blob() {
        // Simulate what `load()` does with a JSON blob carrying the legacy
        // default, without touching the filesystem.
        let json = r#"{ "mouse_sensitivity": 0.003 }"#;
        let mut s: GraphicsSettings = serde_json::from_str(json).unwrap();
        migrate(&mut s);
        s.clamp();
        assert_eq!(s.mouse_sensitivity, DEFAULT_SENSITIVITY);
    }

    #[test]
    fn old_settings_file_loads_brightness_at_default() {
        // A pre-brightness settings file must load with brightness neutral via
        // #[serde(default)], leaving existing players unaffected.
        let json = r#"{ "render_distance": 8 }"#;
        let s: GraphicsSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.brightness, 1.0);
    }

    #[test]
    fn apply_preset_preserves_personal_prefs() {
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::High);
        s.fov_y = 90.0;
        s.mouse_sensitivity = 0.006;
        s.apply_preset(GraphicsPreset::Potato);
        assert_eq!(s.preset(), GraphicsPreset::Potato);
        assert_eq!(s.fov_y, 90.0, "FOV preserved across preset change");
        assert_eq!(s.mouse_sensitivity, 0.006, "sensitivity preserved");
    }

    #[test]
    fn apply_preset_keeps_non_quality_settings() {
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::High);
        s.online_relays = vec!["wss://relay.example.com".into()];
        s.online_port = 4242;
        s.tester_feedback = true;
        s.narration_enabled = !s.narration_enabled;
        let narration = s.narration_enabled;
        s.master_volume = 0.3;
        s.audio_muted = true;
        s.controls_card_seen = true;
        s.apply_preset(GraphicsPreset::Potato);
        assert_eq!(s.preset(), GraphicsPreset::Potato);
        assert_eq!(s.online_relays, vec!["wss://relay.example.com".to_string()]);
        assert_eq!(s.online_port, 4242);
        assert!(s.tester_feedback);
        assert_eq!(s.narration_enabled, narration);
        assert_eq!(s.master_volume, 0.3, "a preset click must not touch the volume");
        assert!(s.audio_muted);
        assert!(s.controls_card_seen);
    }

    #[test]
    fn audio_and_controls_card_defaults_and_old_file_compat() {
        let d = GraphicsSettings::default();
        assert_eq!(d.master_volume, DEFAULT_MASTER_VOLUME);
        assert!(!d.audio_muted);
        assert!(!d.controls_card_seen, "a new device must see the controls card once");
        // An old settings file (none of the three keys) loads with the defaults.
        let old: GraphicsSettings = serde_json::from_str(r#"{ "render_distance": 8 }"#).unwrap();
        assert_eq!(old.master_volume, DEFAULT_MASTER_VOLUME);
        assert!(!old.audio_muted);
        assert!(!old.controls_card_seen);
        // Round-trip.
        let s = GraphicsSettings { master_volume: 0.4, audio_muted: true, controls_card_seen: true, ..Default::default() };
        let back: GraphicsSettings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.master_volume, 0.4);
        assert!(back.audio_muted);
        assert!(back.controls_card_seen);
    }

    #[test]
    fn clamp_pins_master_volume_into_range() {
        let mut s = GraphicsSettings { master_volume: 9.0, ..Default::default() };
        s.clamp();
        assert_eq!(s.master_volume, MASTER_VOLUME_MAX);
        s.master_volume = -1.0;
        s.clamp();
        assert_eq!(s.master_volume, MASTER_VOLUME_MIN);
    }

    #[test]
    fn serde_round_trips() {
        let s = GraphicsSettings::from_preset(GraphicsPreset::Low);
        let json = serde_json::to_string(&s).unwrap();
        let back: GraphicsSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn missing_fields_fall_back_to_high_default() {
        // An old/partial settings file (only render_distance present) must load
        // cleanly with every other field at the High default.
        let json = r#"{ "render_distance": 6 }"#;
        let s: GraphicsSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.render_distance, 6);
        assert_eq!(s.render_scale, 1.0); // High default
        assert_eq!(s.frame_limit, FrameLimit::VSync);
    }

    // ── Phase 6 — third-person camera settings (persist + live-tunable) ──────

    #[test]
    fn camera_settings_have_sane_defaults() {
        let g = GraphicsSettings::default();
        assert_eq!(g.default_camera_mode, crate::camera::CameraMode::FirstPerson);
        assert_eq!(g.third_person_distance, 1.0); // no zoom
        assert!(!g.third_person_freelook); // off — preserves the approved feel
        assert_eq!(g.freelook_sensitivity, 1.0);
        // Auto-centre defaults mirror the camera consts (the flagged feel guesses).
        assert_eq!(g.auto_centre_delay, crate::camera::RECENTER_COOLDOWN);
        assert_eq!(g.auto_centre_speed, crate::camera::RECENTER_SPEED);
    }

    #[test]
    fn camera_settings_clamp_into_range() {
        let mut g = GraphicsSettings::default();
        g.third_person_distance = 99.0;
        g.freelook_sensitivity = -5.0;
        g.auto_centre_delay = 99.0;
        g.auto_centre_speed = 0.0;
        g.clamp();
        assert!(g.third_person_distance <= TP_DISTANCE_MAX && g.third_person_distance >= TP_DISTANCE_MIN);
        assert!(g.freelook_sensitivity >= FREELOOK_SENS_MIN);
        assert!(g.auto_centre_delay <= AUTO_CENTRE_DELAY_MAX);
        assert!(g.auto_centre_speed >= AUTO_CENTRE_SPEED_MIN);
    }

    #[test]
    fn camera_settings_survive_serde_round_trip() {
        let mut g = GraphicsSettings::default();
        g.default_camera_mode = crate::camera::CameraMode::OverShoulder;
        g.third_person_distance = 1.4;
        g.third_person_freelook = true;
        g.auto_centre_delay = 0.5;
        let json = serde_json::to_string(&g).unwrap();
        let back: GraphicsSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.default_camera_mode, crate::camera::CameraMode::OverShoulder);
        assert_eq!(back.third_person_distance, 1.4);
        assert!(back.third_person_freelook);
        assert_eq!(back.auto_centre_delay, 0.5);
    }

    #[test]
    fn license_onboarding_flag_persists_and_defaults_false() {
        // Blueprint capture is gated behind a one-time CC-BY-SA modal. The
        // "seen" flag rides graphics-settings so it persists per-device on web
        // (localStorage) too, not just native — otherwise the modal re-blocks
        // the capture dialog every web session ("i can't save any blueprints").
        let mut g = GraphicsSettings::default();
        assert!(!g.has_seen_license_onboarding, "default: not yet seen");
        g.has_seen_license_onboarding = true;
        let json = serde_json::to_string(&g).unwrap();
        let back: GraphicsSettings = serde_json::from_str(&json).unwrap();
        assert!(back.has_seen_license_onboarding, "flag survives round-trip");
        // Legacy settings JSON without the field → defaults false (back-compat).
        let legacy: GraphicsSettings =
            serde_json::from_str(r#"{ "render_distance": 8 }"#).unwrap();
        assert!(!legacy.has_seen_license_onboarding, "absent field defaults false");
    }

    #[test]
    fn old_settings_file_loads_camera_fields_at_default() {
        // An old file (pre-Phase-6) has no camera fields → they fall back to
        // the defaults via #[serde(default)], so existing players are unaffected.
        let json = r#"{ "render_distance": 8 }"#;
        let s: GraphicsSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.default_camera_mode, crate::camera::CameraMode::FirstPerson);
        assert_eq!(s.third_person_distance, 1.0);
        assert!(!s.third_person_freelook);
    }

    #[test]
    fn clamp_pulls_out_of_range_values_in() {
        let mut s = GraphicsSettings::default();
        s.render_distance = 999;
        s.render_scale = 0.1;
        s.fov_y = 5.0;
        s.mouse_sensitivity = 100.0;
        s.clamp();
        assert_eq!(s.render_distance, RENDER_DISTANCE_MAX);
        assert_eq!(s.render_scale, RENDER_SCALE_MIN);
        assert_eq!(s.fov_y, FOV_MIN);
        assert_eq!(s.mouse_sensitivity, SENSITIVITY_MAX);
    }

    #[test]
    fn software_cap_interval_only_for_explicit_caps() {
        assert_eq!(FrameLimit::VSync.software_cap_interval(), None);
        assert_eq!(FrameLimit::Uncapped.software_cap_interval(), None);
        let c60 = FrameLimit::Cap(60).software_cap_interval().unwrap();
        let c30 = FrameLimit::Cap(30).software_cap_interval().unwrap();
        assert!(c30 > c60, "30 FPS = longer frame budget than 60 FPS");
        // 60 FPS ≈ 16.67 ms.
        assert!((c60.as_secs_f32() - 1.0 / 60.0).abs() < 1e-6);
    }

    #[test]
    fn fog_matches_legacy_constants_at_default_distance() {
        // The old shader literals were fog_start=128, fog_end=160 at RD=10.
        assert_eq!(fog_distances(10), (128.0, 160.0));
    }

    #[test]
    fn fog_tracks_render_distance() {
        let (s4, e4) = fog_distances(4);
        let (s14, e14) = fog_distances(14);
        assert!(e4 < e14, "smaller render distance → nearer fog edge");
        assert!(s4 < e4 && s14 < e14, "fog starts before it ends");
        assert_eq!(e4, 64.0);
        assert_eq!(e14, 224.0);
    }

    // ─── Online play by contact (spec §6) ───

    #[test]
    fn default_relays_are_public_only() {
        let d = default_online_relays();
        assert_eq!(
            d,
            vec![
                "wss://relay.damus.io".to_string(),
                "wss://nos.lol".to_string(),
                "wss://relay.primal.net".to_string(),
            ]
        );
        assert!(d.iter().all(|r| !r.contains("trotters")), "red line 2: no AxeNStax relay in the defaults");
    }

    #[test]
    fn an_uncustomised_pre_launch_relay_list_migrates_to_the_public_default() {
        let mut s = GraphicsSettings::default();
        s.online_relays = PRE_LAUNCH_ONLINE_RELAYS.iter().map(|r| r.to_string()).collect();
        s.clamp();
        assert_eq!(s.online_relays, default_online_relays());
    }

    #[test]
    fn a_customised_relay_list_that_includes_trotters_is_kept() {
        let mut s = GraphicsSettings::default();
        let mine = vec!["wss://relay.trotters.cc".to_string(), "wss://my.relay.example".to_string()];
        s.online_relays = mine.clone();
        s.clamp();
        assert_eq!(s.online_relays, mine);
    }

    #[test]
    fn a_fresh_settings_object_carries_the_online_defaults() {
        let s = GraphicsSettings::default();
        assert_eq!(s.online_relays, default_online_relays());
        assert_eq!(s.online_port, DEFAULT_ONLINE_PORT);
    }

    #[test]
    fn sanitise_drops_non_wss_blank_and_duplicate_relays_and_caps_at_eight() {
        let got = sanitise_relays(vec![
            "  wss://a.example  ".to_string(),
            "ws://insecure.example".to_string(),
            "".to_string(),
            "https://not-a-relay.example".to_string(),
            "wss://a.example".to_string(),
            "wss://b.example".to_string(),
        ]);
        assert_eq!(
            got,
            vec!["wss://a.example".to_string(), "wss://b.example".to_string()]
        );

        let many: Vec<String> = (0..12).map(|i| format!("wss://r{i}.example")).collect();
        assert_eq!(sanitise_relays(many).len(), 8, "capped at the invite's MAX_RELAYS");
    }

    #[test]
    fn clamp_repairs_an_empty_relay_list_but_keeps_a_custom_one() {
        let mut s = GraphicsSettings { online_relays: vec![], ..Default::default() };
        s.clamp();
        assert_eq!(
            s.online_relays,
            default_online_relays(),
            "an empty list would make online play silently impossible"
        );

        let mut custom = GraphicsSettings {
            online_relays: vec!["wss://mine.example".to_string()],
            ..Default::default()
        };
        custom.clamp();
        assert_eq!(custom.online_relays, vec!["wss://mine.example".to_string()]);
    }

    #[test]
    fn an_old_settings_file_without_the_online_fields_still_loads() {
        // `serde(default)` is what keeps an existing settings.json readable —
        // pinned here because losing it would reset every player's graphics
        // preferences, not just their relays. The "old" file is built by
        // serialising today's settings and DELETING the two new keys, so this
        // test cannot rot as other fields are added.
        let mut v: serde_json::Value =
            serde_json::to_value(GraphicsSettings::default()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("online_relays");
        obj.remove("online_port");
        assert!(!obj.contains_key("online_relays"));

        let s: GraphicsSettings = serde_json::from_value(v).unwrap();
        assert_eq!(
            s.render_distance,
            GraphicsSettings::default().render_distance,
            "the old fields survive"
        );
        assert_eq!(s.online_relays, default_online_relays());
        assert_eq!(s.online_port, DEFAULT_ONLINE_PORT);
    }

    #[test]
    fn tester_feedback_is_off_by_default() {
        assert!(!GraphicsSettings::default().tester_feedback);
    }

    #[test]
    fn an_old_settings_file_without_tester_feedback_loads_with_it_off() {
        // Built by deleting the key from today's serialised settings, so the
        // test cannot rot as other fields are added.
        let mut v: serde_json::Value =
            serde_json::to_value(GraphicsSettings { tester_feedback: true, ..Default::default() })
                .unwrap();
        assert!(v.as_object_mut().unwrap().remove("tester_feedback").is_some());
        let s: GraphicsSettings = serde_json::from_value(v).unwrap();
        assert!(!s.tester_feedback, "an old file must never switch feedback on");
        // And a file that only holds other keys loads the same way.
        let s: GraphicsSettings = serde_json::from_str(r#"{ "render_distance": 8 }"#).unwrap();
        assert!(!s.tester_feedback);
    }

    #[test]
    fn tester_feedback_round_trips_and_survives_a_preset_click() {
        let mut s = GraphicsSettings { tester_feedback: true, ..Default::default() };
        let back: GraphicsSettings =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(back.tester_feedback);
        s.apply_preset(GraphicsPreset::Potato);
        assert!(s.tester_feedback, "choosing a quality preset must not clear the unlock");
        assert!(s.clone().tester_feedback);
    }

    #[test]
    fn online_settings_are_not_part_of_preset_detection() {
        // Relays and a port are not GPU load. Editing them must not knock the
        // player off "High" — the same rule FOV and sensitivity follow.
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::High);
        s.online_relays = vec!["wss://mine.example".to_string()];
        s.online_port = 0;
        assert_eq!(s.preset(), GraphicsPreset::High);
    }

    // -- Perf B1: device-aware defaults, pixel budget, auto quality ---------

    #[test]
    fn recommend_preset_table() {
        use wgpu::DeviceType as D;
        use GraphicsPreset as P;
        // (device, touch, web) -> preset
        let table = [
            // Software rasterisers are Potato whatever else is true.
            (D::Cpu, false, false, P::Potato),
            (D::Cpu, false, true, P::Potato),
            (D::Cpu, true, true, P::Potato),
            // Touch / Android / mobile web are Low.
            (D::IntegratedGpu, true, false, P::Low),
            (D::IntegratedGpu, true, true, P::Low),
            (D::DiscreteGpu, true, true, P::Low),
            (D::Other, true, true, P::Low),
            // Integrated: Low on web, Medium native.
            (D::IntegratedGpu, false, true, P::Low),
            (D::IntegratedGpu, false, false, P::Medium),
            // Unknown class (typical in a browser) and virtual GPUs: Medium.
            (D::Other, false, true, P::Medium),
            (D::Other, false, false, P::Medium),
            (D::VirtualGpu, false, false, P::Medium),
            (D::VirtualGpu, false, true, P::Medium),
            // Discrete: High.
            (D::DiscreteGpu, false, false, P::High),
            (D::DiscreteGpu, false, true, P::High),
        ];
        for (d, touch, web, want) in table {
            assert_eq!(
                recommend_preset(d, touch, web),
                want,
                "device {d:?} touch {touch} web {web}"
            );
        }
    }

    #[test]
    fn pixel_budget_scale_maths() {
        // No budget: the user's scale, clamped.
        assert_eq!(budgeted_scale(1.0, None, 3840, 2160), 1.0);
        assert_eq!(budgeted_scale(0.1, None, 1920, 1080), RENDER_SCALE_MIN);
        // 1920x1080 = 2.0736 M pixels: a 2.1 M budget does not bind.
        assert_eq!(budgeted_scale(1.0, Some(MEDIUM_PIXEL_BUDGET), 1920, 1080), 1.0);
        // 4K (8.2944 M): sqrt(2.1 / 8.2944) = 0.5032.
        let s = budgeted_scale(1.0, Some(MEDIUM_PIXEL_BUDGET), 3840, 2160);
        assert!((s - 0.5032).abs() < 1e-3, "{s}");
        // The result times the surface never exceeds the budget (within rounding).
        let px = (3840.0 * s) * (2160.0 * s);
        assert!(px <= MEDIUM_PIXEL_BUDGET as f32 * 1.0001, "{px}");
        // A DPR-3 phone: 390x844 CSS = 1170x2532 = 2.96 M physical.
        let s = budgeted_scale(0.75, Some(LOW_PIXEL_BUDGET), 1170, 2532);
        assert!((s - (1.3e6_f32 / 2_962_440.0).sqrt()).abs() < 1e-3, "{s}");
        // Never raises above the user's scale.
        assert_eq!(budgeted_scale(0.5, Some(MEDIUM_PIXEL_BUDGET), 640, 480), 0.5);
        // Never drops below the floor, however large the surface.
        assert_eq!(
            budgeted_scale(1.0, Some(POTATO_PIXEL_BUDGET), 16000, 9000),
            RENDER_SCALE_MIN
        );
        // A zero-size surface cannot divide by zero.
        assert!(budgeted_scale(1.0, Some(LOW_PIXEL_BUDGET), 0, 0).is_finite());
    }

    #[test]
    fn preset_budgets_match_the_decision() {
        let b = |p| GraphicsSettings::from_preset(p).pixel_budget;
        assert_eq!(b(GraphicsPreset::Potato), Some(900_000));
        assert_eq!(b(GraphicsPreset::Low), Some(1_300_000));
        assert_eq!(b(GraphicsPreset::Medium), Some(2_100_000));
        assert_eq!(b(GraphicsPreset::High), None);
        assert_eq!(b(GraphicsPreset::Ultra), None);
        // The default is still the High preset with nothing new switched on.
        let d = GraphicsSettings::default();
        assert_eq!(d.pixel_budget, None);
        assert!(!d.auto_quality);
    }

    #[test]
    fn render_scale_floor_is_a_quarter() {
        assert_eq!(RENDER_SCALE_MIN, 0.25);
        let mut s = GraphicsSettings::default();
        s.render_scale = 0.3;
        s.clamp();
        assert_eq!(s.render_scale, 0.3);
    }

    /// Parse through the loader's own path (`parse_persisted`) and clamp.
    fn load_json(json: &str) -> GraphicsSettings {
        let mut s = parse_persisted(json).expect("parses");
        s.clamp();
        s
    }

    #[test]
    fn an_old_low_file_is_still_low_and_takes_lows_budget() {
        // Written before pixel_budget / auto_quality existed, and while
        // smooth_lighting was still a (dead) key.
        let s = load_json(
            r#"{
            "render_distance": 5,
            "render_scale": 0.75,
            "frame_limit": { "Cap": 60 },
            "fog": true,
            "mipmaps": false,
            "particles": "Reduced",
            "smooth_lighting": false,
            "fov_y": 85.0
        }"#,
        );
        assert_eq!(s.preset(), GraphicsPreset::Low, "label must not become Custom");
        assert_eq!(s.pixel_budget, Some(LOW_PIXEL_BUDGET));
        assert!(!s.auto_quality, "no key = governor off");
        assert_eq!(s.fov_y, 85.0, "personal prefs untouched");
    }

    #[test]
    fn every_old_preset_file_keeps_its_label_and_its_presets_budget() {
        for p in GraphicsPreset::ALL {
            let mut new = GraphicsSettings::from_preset(p);
            new.auto_quality = false;
            // An old file: today's JSON minus the two new keys.
            let mut v = serde_json::to_value(&new).unwrap();
            v.as_object_mut().unwrap().remove("pixel_budget");
            v.as_object_mut().unwrap().remove("auto_quality");
            let s = load_json(&v.to_string());
            assert_eq!(s.preset(), p, "{p:?}");
            assert_eq!(s.pixel_budget, new.pixel_budget, "{p:?}");
            assert!(!s.auto_quality);
        }
    }

    #[test]
    fn leaves_fast_is_on_for_potato_and_low_only() {
        for (p, want) in [
            (GraphicsPreset::Potato, true),
            (GraphicsPreset::Low, true),
            (GraphicsPreset::Medium, false),
            (GraphicsPreset::High, false),
            (GraphicsPreset::Ultra, false),
        ] {
            assert_eq!(GraphicsSettings::from_preset(p).leaves_fast, want, "{p:?}");
        }
        assert!(!GraphicsSettings::default().leaves_fast, "the default look is unchanged");
    }

    #[test]
    fn leaves_fast_is_part_of_the_preset() {
        // Flipping it takes a preset to Custom; flipping back restores the label.
        for p in GraphicsPreset::ALL {
            if p == GraphicsPreset::Custom {
                continue;
            }
            let mut s = GraphicsSettings::from_preset(p);
            s.leaves_fast = !s.leaves_fast;
            assert_eq!(s.preset(), GraphicsPreset::Custom, "{p:?}");
            s.leaves_fast = !s.leaves_fast;
            assert_eq!(s.preset(), p, "{p:?}");
        }
        // Applying a preset sets it, and an unrelated dial change does not.
        let mut s = GraphicsSettings::default();
        s.apply_preset(GraphicsPreset::Low);
        assert!(s.leaves_fast);
        s.apply_preset(GraphicsPreset::Medium);
        assert!(!s.leaves_fast);
        s.fov_y = 95.0;
        assert_eq!(s.preset(), GraphicsPreset::Medium);
    }

    #[test]
    fn an_old_file_takes_leaves_fast_from_the_preset_its_dials_match() {
        // Today's JSON minus every key this and the last wave added.
        for p in GraphicsPreset::ALL {
            let new = GraphicsSettings::from_preset(p);
            let mut v = serde_json::to_value(&new).unwrap();
            for k in ["pixel_budget", "leaves_fast", "auto_quality"] {
                v.as_object_mut().unwrap().remove(k);
            }
            let s = load_json(&v.to_string());
            assert_eq!(s.leaves_fast, new.leaves_fast, "{p:?}");
            assert_eq!(s.preset(), p, "an old {p:?} file keeps its label");
        }
        // Dials that match no preset: off.
        let s = load_json(r#"{ "render_distance": 9, "render_scale": 0.6, "particles": "Reduced" }"#);
        assert!(!s.leaves_fast);
        assert_eq!(s.preset(), GraphicsPreset::Custom);
        // The bare file is High: off.
        assert!(!load_json("{}").leaves_fast);
    }

    #[test]
    fn a_stored_leaves_fast_key_is_taken_as_written() {
        // Low dials with the key written false: the player turned it off.
        let low = GraphicsSettings::from_preset(GraphicsPreset::Low);
        let mut v = serde_json::to_value(&low).unwrap();
        v["leaves_fast"] = serde_json::Value::Bool(false);
        let s = load_json(&v.to_string());
        assert!(!s.leaves_fast);
        assert_eq!(s.preset(), GraphicsPreset::Custom);
        // And true on High dials stays true.
        let mut v = serde_json::to_value(GraphicsSettings::default()).unwrap();
        v["leaves_fast"] = serde_json::Value::Bool(true);
        let s = load_json(&v.to_string());
        assert!(s.leaves_fast);
        assert_eq!(s.preset(), GraphicsPreset::Custom);
    }

    #[test]
    fn a_file_with_a_hand_picked_budget_does_not_borrow_a_presets_leaves() {
        // Written between the budget wave and this one: has pixel_budget, no
        // leaves_fast. Low dials but a budget of the player's own: Custom, off.
        let low = GraphicsSettings::from_preset(GraphicsPreset::Low);
        let mut v = serde_json::to_value(&low).unwrap();
        v["pixel_budget"] = serde_json::json!(1_700_000u32);
        v.as_object_mut().unwrap().remove("leaves_fast");
        let s = load_json(&v.to_string());
        assert!(!s.leaves_fast);
        // The preset's own budget does match.
        let mut v = serde_json::to_value(&low).unwrap();
        v.as_object_mut().unwrap().remove("leaves_fast");
        let s = load_json(&v.to_string());
        assert!(s.leaves_fast);
        assert_eq!(s.preset(), GraphicsPreset::Low);
    }

    #[test]
    fn an_old_custom_file_stays_custom_with_no_budget() {
        let s = load_json(
            r#"{ "render_distance": 9, "render_scale": 0.6, "particles": "Reduced" }"#,
        );
        assert_eq!(s.preset(), GraphicsPreset::Custom);
        assert_eq!(s.pixel_budget, None);
        assert!(!s.auto_quality);
        // The bare minimum file is the High default, uncapped.
        let s = load_json("{}");
        assert_eq!(s.preset(), GraphicsPreset::High);
        assert_eq!(s.pixel_budget, None);
    }

    #[test]
    fn a_stored_budget_key_is_taken_as_written() {
        // Low dials with the key present and null: the player's file says no
        // budget, so it is not re-derived (and reads as Custom, as saved).
        let s = load_json(
            r#"{ "render_distance": 5, "render_scale": 0.75, "frame_limit": { "Cap": 60 },
                 "particles": "Reduced", "pixel_budget": null }"#,
        );
        assert_eq!(s.pixel_budget, None);
        assert_eq!(s.preset(), GraphicsPreset::Custom);
        let s = load_json(r#"{ "pixel_budget": 1500000 }"#);
        assert_eq!(s.pixel_budget, Some(1_500_000));
    }

    #[test]
    fn a_saved_new_file_round_trips_through_the_loader() {
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::Medium);
        s.auto_quality = true;
        let back = load_json(&serde_json::to_string(&s).unwrap());
        assert_eq!(back, s);
        // High has no budget: saved as null, loaded as None, still High.
        let high = GraphicsSettings::default();
        let back = load_json(&serde_json::to_string(&high).unwrap());
        assert_eq!(back.preset(), GraphicsPreset::High);
        assert_eq!(back.pixel_budget, None);
    }

    #[test]
    fn new_fields_round_trip_and_hand_edits_clamp() {
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::Medium);
        s.auto_quality = true;
        let back: GraphicsSettings =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.pixel_budget, Some(MEDIUM_PIXEL_BUDGET));
        assert!(back.auto_quality);
        let mut tiny = back;
        tiny.pixel_budget = Some(1);
        tiny.clamp();
        assert_eq!(tiny.pixel_budget, Some(PIXEL_BUDGET_MIN));
    }

    #[test]
    fn web_cap_interval_decision() {
        use std::time::Duration as D;
        let i60 = FrameLimit::Cap(60).software_cap_interval().unwrap();
        let i30 = FrameLimit::Cap(30).software_cap_interval().unwrap();
        // 60 Hz display, 60 FPS cap: a frame 16.4 ms after the last (rAF
        // jitter) is still due; one at 8 ms is not.
        assert!(cap_frame_due(D::from_micros(16_400), i60));
        assert!(!cap_frame_due(D::from_millis(8), i60));
        // 60 Hz display, 30 FPS cap: alternate rAF ticks are skipped.
        assert!(!cap_frame_due(D::from_micros(16_700), i30));
        assert!(cap_frame_due(D::from_micros(33_300), i30));
        // 120 Hz display, 60 FPS cap: every second tick (8.3 ms) is skipped.
        assert!(!cap_frame_due(D::from_micros(8_300), i60));
        assert!(cap_frame_due(D::from_micros(16_700), i60));
        // Slack is bounded: a frame 5 ms early under a 33 ms cap is not due.
        assert!(!cap_frame_due(D::from_millis(28), i30));
        // A zero elapsed time is never due.
        assert!(!cap_frame_due(D::ZERO, i60));
    }

    #[test]
    fn first_run_preset_enables_auto_quality_only_through_the_loader() {
        // The pure builder never switches the governor on by itself.
        for p in GraphicsPreset::ALL {
            assert!(!GraphicsSettings::from_preset(p).auto_quality);
        }
    }
}

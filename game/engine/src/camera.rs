//! FPS camera with view/projection matrices.
//!
//! Spec 03 Section 1.5: Camera uniform contains view, projection, view_proj, camera_pos, time.
//! Spec 05 Section 1.1: Creative flight speed 10.89 b/s, sprint 21.78 b/s.

use glam::{Mat4, Vec3, Vec4};

/// A view frustum as 6 inward-pointing plane equations `ax + by + cz + d = 0`
/// (stored as `Vec4{x:a, y:b, z:c, w:d}`). A point is inside the frustum when it
/// is on the positive side of all 6 planes. Spec 03 §5.2.
///
/// Extracted from a view-projection matrix via the Gribb–Hartmann method. This
/// engine renders with `glam::camera::rh::proj::directx::perspective` (wgpu/D3D clip volume: `0 ≤ z ≤ w`),
/// so the near plane is `row2` (not `row3 + row2`, which is the OpenGL form).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    /// left, right, bottom, top, near, far — inward normals.
    pub planes: [Vec4; 6],
}

impl Frustum {
    /// Extract the 6 frustum planes from a view-projection matrix.
    pub fn from_view_proj(m: Mat4) -> Self {
        // glam `Mat4::row(i)` gives the i-th row as a Vec4, which is exactly
        // what Gribb–Hartmann needs (the method is matrix-layout agnostic once
        // you have the rows).
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        Self {
            planes: [
                r3 + r0, // left
                r3 - r0, // right
                r3 + r1, // bottom
                r3 - r1, // top
                r2,      // near  (wgpu/D3D 0≤z≤w convention)
                r3 - r2, // far
            ],
        }
    }

    /// True if the axis-aligned box `[min, max]` is at least partially inside
    /// the frustum. Conservative: may keep a box that is just outside a corner,
    /// but never culls a box that is actually visible.
    pub fn contains_aabb(&self, min: Vec3, max: Vec3) -> bool {
        frustum_contains_aabb(&self.planes, min, max)
    }
}

/// Pure frustum-vs-AABB test (Spec 03 §5.2). Returns `false` only when the box
/// is *fully* outside at least one plane. Uses the "p-vertex" optimisation: for
/// each plane it tests the single AABB corner furthest along the plane normal —
/// if even that corner is behind the plane, the whole box is outside.
pub fn frustum_contains_aabb(planes: &[Vec4; 6], min: Vec3, max: Vec3) -> bool {
    for p in planes {
        let n = Vec3::new(p.x, p.y, p.z);
        // The AABB corner furthest in the +normal direction.
        let pv = Vec3::new(
            if n.x >= 0.0 { max.x } else { min.x },
            if n.y >= 0.0 { max.y } else { min.y },
            if n.z >= 0.0 { max.z } else { min.z },
        );
        if n.dot(pv) + p.w < 0.0 {
            return false; // fully outside this plane → outside the frustum
        }
    }
    true
}

/// Which perspective the camera renders from. **Aim stays eye-anchored in every
/// mode** — only the *render* origin moves (see [`render_eye`]); gameplay rays
/// (mining/placing/combat) always originate from the true eye + look direction.
/// Engine-generic (cross-game). UK English. Defaults to first-person.
/// Spec: `docs/foundations/2026-06-09-third-person-camera.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CameraMode {
    /// The eye is the camera (classic FPS). The render origin == the true eye.
    #[default]
    FirstPerson,
    /// Over-the-shoulder third-person — the proven default (Shoulder Surfing,
    /// 56.5M+ downloads). Render origin pulled back along -forward, offset to the
    /// shoulder side, lifted slightly.
    OverShoulder,
    /// Centred-behind third-person at a fixed orbit distance (MC's ~4-block view).
    OrbitBehind,
}

impl CameraMode {
    /// True when the eye IS the camera. The first-person viewmodel (hand + held
    /// item) draws only in this mode; third-person draws the self-avatar instead.
    pub fn shows_viewmodel(self) -> bool {
        matches!(self, CameraMode::FirstPerson)
    }

    /// True when the render origin is pulled back off the body — the modes that
    /// draw the player's own avatar and hide the first-person viewmodel.
    pub fn is_third_person(self) -> bool {
        !self.shows_viewmodel()
    }

    /// The next mode in the perspective-toggle cycle (F5 / gamepad / touch):
    /// first-person → over-the-shoulder → orbit-behind → first-person. A closed
    /// 3-cycle so repeated taps always return home.
    pub fn next(self) -> Self {
        match self {
            CameraMode::FirstPerson => CameraMode::OverShoulder,
            CameraMode::OverShoulder => CameraMode::OrbitBehind,
            CameraMode::OrbitBehind => CameraMode::FirstPerson,
        }
    }
}

// Tuning consts — *feel* numbers. Phase 6's Axolittle playtest tunes these; the
// directions they encode (behind / to-the-side / lifted) are the invariant, not
// the magnitudes. All in world units (blocks).
/// Over-the-shoulder: metres the render eye pulls back along -forward.
pub const OVER_SHOULDER_BACK: f32 = 2.5;
/// Over-the-shoulder: lateral shoulder offset along +right.
pub const OVER_SHOULDER_RIGHT: f32 = 0.65;
/// Over-the-shoulder: vertical lift above the eye.
pub const OVER_SHOULDER_UP: f32 = 0.25;
/// Orbit-behind: centred distance back along -forward (MC default ≈ 4).
pub const ORBIT_BEHIND_DISTANCE: f32 = 4.0;
/// Orbit-behind: vertical lift above the eye.
pub const ORBIT_BEHIND_UP: f32 = 0.4;

// Phase 2 — no-snap camera collision. Feel numbers (Phase 6 playtest tunes);
// the *behaviour* they encode (clamp before solid, retract fast, ease out slow)
// is the invariant, not the magnitudes. All render-only — aim is never affected.
/// World units of air kept between the render eye and the surface it would clip.
pub const COLLISION_MARGIN: f32 = 0.25;
/// Retract speed when an obstruction appears (fraction-of-offset per second).
/// Fast, so the camera is never left sitting inside geometry.
pub const COLLISION_PULL_IN_SPEED: f32 = 12.0;
/// Ease-out speed once the path clears (fraction-of-offset per second). Slow, so
/// recovery is a smooth lerp — the documented "punch in, never return" fix.
pub const COLLISION_EASE_OUT_SPEED: f32 = 3.0;

// Phase 3 — avatar fade-on-occlusion. When Phase-2 collision pulls the camera in
// close to the body, the body occludes the crosshair, so the self-avatar fades
// toward transparent (screen-door dither in `fs_avatar`). Feel numbers (Phase 6).
/// At/above this collision fraction the avatar is fully opaque (camera far enough
/// back that the body doesn't block the aim).
pub const AVATAR_FADE_START: f32 = 0.6;
/// The most-transparent the avatar gets when the camera is fully pulled in — kept
/// faintly visible (not fully invisible) so you still see your guy.
pub const AVATAR_FADE_MIN_ALPHA: f32 = 0.12;

// Phase 5 — feel. Every lever defaults to NEUTRAL so the approved Phase-1 feel is
// unchanged; profiles + free-look + auto-recenter are mechanisms the Phase-6
// playtest tunes/enables. All magnitudes are feel GUESSES, flagged for that
// playtest. Decoupling is render-only — aim (yaw/pitch/forward) is never touched.
/// Clamp on the pitch→distance scale (how far the look-down/up curve may push).
pub const PITCH_DISTANCE_MIN: f32 = 0.6;
pub const PITCH_DISTANCE_MAX: f32 = 1.5;
/// Clamp on the pitch→FOV offset in degrees.
pub const PITCH_FOV_MIN: f32 = -6.0;
pub const PITCH_FOV_MAX: f32 = 8.0;
// Auto-recenter / free-look tuning. Live as of Phase 6 (free-look input bound to
// Alt; auto-recenter ticked in the game loop). The DEFAULT values double as the
// `GraphicsSettings.auto_centre_*` defaults — the flagged feel guesses the
// Axolittle playtest tunes via the in-game sliders.
/// Default auto-recenter ease speed: orbit radians/second back toward behind.
pub const RECENTER_SPEED: f32 = 2.5;
/// Default post-input cooldown (seconds) after free-look before recenter resumes.
pub const RECENTER_COOLDOWN: f32 = 1.0;
/// Minimum player speed (blocks/s) for velocity-gated auto-recenter to engage.
pub const RECENTER_MIN_SPEED: f32 = 0.5;
/// Clamp on the free-look vertical orbit offset (radians).
pub const ORBIT_PITCH_LIMIT: f32 = 1.2;

/// Phase 5 — a per-context camera *feel* profile (data-driven). Build sits
/// further back + wider for overview (Workshop/blueprints/farming/villages);
/// Combat sits closer + tighter (Hash Dash/Satori Rush/raids). The engine picks
/// by context (Phase 6); Neutral is the no-op default that preserves Phase-1 feel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraProfile {
    /// Multiplies the base third-person pull-back distance.
    pub distance_mul: f32,
    /// Added to the base FOV (degrees).
    pub fov_offset: f32,
    /// Pitch→distance curve strength (0 = no curve). Look down → pull back.
    pub pitch_distance_k: f32,
    /// Pitch→FOV curve strength (0 = no curve). Look down → widen.
    pub pitch_fov_k: f32,
}

/// The no-op default — identical render to Phase 1–4 (the approved feel).
pub const NEUTRAL_PROFILE: CameraProfile = CameraProfile {
    distance_mul: 1.0,
    fov_offset: 0.0,
    pitch_distance_k: 0.0,
    pitch_fov_k: 0.0,
};
/// Build: further back, wider, look-down overview bias. Feel guesses (Phase 6).
/// Selected by context in Phase 6 (the selector is the deferred enablement).
#[allow(dead_code)]
pub const BUILD_PROFILE: CameraProfile = CameraProfile {
    distance_mul: 1.25,
    fov_offset: 6.0,
    pitch_distance_k: 0.30,
    pitch_fov_k: 5.0,
};
/// Combat: closer, tighter framing. Feel guesses (Phase 6).
#[allow(dead_code)]
pub const COMBAT_PROFILE: CameraProfile = CameraProfile {
    distance_mul: 0.85,
    fov_offset: -4.0,
    pitch_distance_k: 0.10,
    pitch_fov_k: 2.0,
};

/// The render-camera origin for a given mode, with the Phase-5 profile/pitch
/// `dist_scale` multiplying **only the longitudinal pull-back distance**
/// (`-forward`). First-person returns the eye unchanged; the over-the-shoulder
/// lateral and vertical offsets stay fixed — a further-back or closer rig must
/// not drift sideways/up as you change profile or pitch. `dist_scale == 1.0` is
/// the un-scaled (Phase-1) render eye. `forward`/`right` are expected unit vectors.
///
/// **This is render-only.** No gameplay raycast may originate here — the
/// eye-anchored-aim invariant requires mining/placing/combat to keep raying from
/// the true eye + look direction.
pub fn render_eye_scaled(eye: Vec3, forward: Vec3, right: Vec3, mode: CameraMode, dist_scale: f32) -> Vec3 {
    match mode {
        CameraMode::FirstPerson => eye,
        CameraMode::OverShoulder => {
            eye - forward * (OVER_SHOULDER_BACK * dist_scale)
                + right * OVER_SHOULDER_RIGHT
                + Vec3::Y * OVER_SHOULDER_UP
        }
        CameraMode::OrbitBehind => {
            eye - forward * (ORBIT_BEHIND_DISTANCE * dist_scale) + Vec3::Y * ORBIT_BEHIND_UP
        }
    }
}

/// The fraction `[0,1]` of the desired third-person pull-back the render eye may
/// extend to without entering solid geometry. `offset_len` is the full desired
/// distance from the eye to the un-clamped render eye; `hit_dist` is the distance
/// to the first solid block along that ray (`None` = clear line of sight). Keeps
/// `margin` world units of air between the camera and the surface. Render-only —
/// this only moves where the frame is viewed from, never the aim ray.
pub fn collision_target_fraction(offset_len: f32, hit_dist: Option<f32>, margin: f32) -> f32 {
    if offset_len <= 1e-5 {
        return 1.0; // first-person / no pull-back → nothing to clamp
    }
    match hit_dist {
        None => 1.0, // clear path to the desired render eye
        Some(d) => ((d - margin) / offset_len).clamp(0.0, 1.0),
    }
}

/// Advance a collision fraction toward `target` (both in `[0,1]`) without
/// snapping: retracting (`target < current` — an obstruction appeared) uses the
/// fast `in_speed`; easing back out (`target > current` — the path cleared) uses
/// the slow `out_speed`. Speeds are fraction-per-second; `dt` in seconds. A step
/// never overshoots the target (a sub-step delta lands exactly on it).
pub fn approach_fraction(current: f32, target: f32, dt: f32, in_speed: f32, out_speed: f32) -> f32 {
    let speed = if target < current { in_speed } else { out_speed };
    let max_step = (speed * dt).max(0.0);
    let delta = target - current;
    if delta.abs() <= max_step {
        target
    } else {
        current + delta.signum() * max_step
    }
}

/// Phase 3 — the self-avatar's fade alpha `[AVATAR_FADE_MIN_ALPHA, 1.0]` for a
/// given Phase-2 collision fraction. Above [`AVATAR_FADE_START`] the camera is
/// far enough back that the body doesn't occlude the crosshair → fully opaque
/// (`1.0`); below, it ramps linearly down to [`AVATAR_FADE_MIN_ALPHA`] as the
/// camera pulls in. The renderer feeds this into a screen-door dither so the
/// aim target stays visible through the fading body. **Render-only.**
pub fn avatar_fade_alpha(collision_frac: f32) -> f32 {
    let c = collision_frac.clamp(0.0, 1.0);
    if c >= AVATAR_FADE_START {
        1.0
    } else {
        AVATAR_FADE_MIN_ALPHA + (1.0 - AVATAR_FADE_MIN_ALPHA) * (c / AVATAR_FADE_START)
    }
}

/// Phase 5 — pitch→distance scale for the third-person pull-back. `pitch` is the
/// look pitch (radians; >0 = up, <0 = down). Looking down (`pitch<0`) pulls the
/// camera **back** for a build overview; looking up brings it **closer** for a
/// combat framing. `k` is the per-profile strength (0 = no curve). Clamped to
/// `[PITCH_DISTANCE_MIN, PITCH_DISTANCE_MAX]`. Render-only.
pub fn pitch_distance_scale(pitch: f32, k: f32) -> f32 {
    (1.0 - pitch * k).clamp(PITCH_DISTANCE_MIN, PITCH_DISTANCE_MAX)
}

/// Phase 5 — pitch→FOV offset (degrees) for third-person. Looking down widens
/// (overview); looking up narrows. `k` is the per-profile strength (0 = none).
/// Clamped to `[PITCH_FOV_MIN, PITCH_FOV_MAX]`. Render-only.
pub fn pitch_fov_offset(pitch: f32, k: f32) -> f32 {
    (-pitch * k).clamp(PITCH_FOV_MIN, PITCH_FOV_MAX)
}

/// Phase 5 — velocity-gated auto-recenter predicate. The camera blends its orbit
/// back behind the player only when the player is **not** steering (no deliberate
/// orbit input), **is moving** (`speed >= RECENTER_MIN_SPEED`), and the
/// post-input `cooldown` has elapsed (`<= 0`). Player input always wins (it arms
/// the cooldown via [`Camera::apply_free_look`]).
pub fn should_auto_recenter(steering: bool, speed: f32, cooldown: f32) -> bool {
    !steering && cooldown <= 0.0 && speed >= RECENTER_MIN_SPEED
}

pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,   // Radians, 0 = looking along -Z (Minecraft convention)
    pub pitch: f32, // Radians, clamped to [-89°, 89°]
    pub fov_y: f32, // Vertical FOV in degrees
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
    /// Perspective mode (per-player — each `PlayerSlot` owns its `Camera`).
    /// First-person by default. In third-person only the *render* origin moves;
    /// `position`/`forward()` stay the eye/look that gameplay rays read.
    pub mode: CameraMode,
    /// Phase 2 — live no-snap collision fraction `[0,1]`: how far along the
    /// desired third-person pull-back the render eye currently sits (`1.0` =
    /// fully extended / unobstructed). Smoothed each fixed tick by
    /// [`Self::update_collision`] from a world raycast. First-person ignores it
    /// (no offset to scale). Render-only — never affects aim.
    pub collision_frac: f32,
    /// Phase 5 — the active feel profile (Neutral by default = the approved
    /// Phase-1 feel). Scales the pull-back distance + FOV + pitch curves.
    pub profile: CameraProfile,
    /// Phase 5 — decoupled free-look horizontal orbit offset (radians) added to
    /// the *render* azimuth only; `yaw` (aim) is untouched. Auto-recentres to 0.
    pub orbit_yaw_offset: f32,
    /// Phase 5 — decoupled free-look vertical orbit offset (radians), render-only.
    pub orbit_pitch_offset: f32,
    /// Phase 5 — seconds of post-input cooldown remaining before velocity-gated
    /// auto-recenter resumes (armed by [`Self::apply_free_look`]).
    pub recenter_cooldown: f32,
    /// Phase 6 — live (settings-driven) third-person zoom multiplier on the
    /// pull-back distance (1.0 = the built-in distance). Pushed each tick from
    /// `GraphicsSettings.third_person_distance`.
    pub user_distance: f32,
    /// Phase 6 — live auto-centre ease speed (orbit rad/s), from
    /// `GraphicsSettings.auto_centre_speed`. Defaults to [`RECENTER_SPEED`].
    pub recenter_speed: f32,
    /// Phase 6 — live auto-centre post-input delay (seconds), from
    /// `GraphicsSettings.auto_centre_delay`. Defaults to [`RECENTER_COOLDOWN`].
    pub recenter_delay: f32,
    /// #44 — hold-to-zoom render FOV (degrees), pushed each tick from
    /// `GraphicsSettings.zoom_fov`. Used only while [`Self::zooming`] is set;
    /// deliberately outside the personal-FOV clamp.
    pub zoom_fov: f32,
    /// #44 — live zoom state, set each frame from the C-key/touch hold. Render-
    /// only: it swaps the base FOV in [`Self::effective_fov_y`] without touching
    /// the stored `fov_y`, so releasing restores the configured FOV exactly.
    pub zooming: bool,
}

impl Camera {
    pub fn new(position: Vec3, aspect: f32) -> Self {
        Self {
            position,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 70.0, // Minecraft default FOV
            aspect,
            near: 0.1,
            far: 1000.0,
            mode: CameraMode::FirstPerson,
            collision_frac: 1.0,
            profile: NEUTRAL_PROFILE,
            orbit_yaw_offset: 0.0,
            orbit_pitch_offset: 0.0,
            recenter_cooldown: 0.0,
            user_distance: 1.0,
            recenter_speed: RECENTER_SPEED,
            recenter_delay: RECENTER_COOLDOWN,
            zoom_fov: crate::graphics_settings::DEFAULT_ZOOM_FOV,
            zooming: false,
        }
    }

    /// #44 — set the transient hold-to-zoom state. Render-only; does not touch
    /// the stored `fov_y`, so releasing restores the configured FOV exactly.
    pub fn set_zoom(&mut self, active: bool) {
        self.zooming = active;
    }

    /// The render-camera origin for this camera given its [`CameraMode`], with the
    /// live Phase-2 collision clamp applied. Equals `position` (the true eye) in
    /// first-person; in third-person it is the desired pull-back scaled by
    /// [`Self::collision_frac`] (`1.0` = fully extended). **Render-only** — never
    /// an aim-ray origin (eye-anchored-aim invariant).
    pub fn render_eye(&self) -> Vec3 {
        let desired = self.desired_render_eye();
        // Lerp from the eye toward the desired pull-back by the collision
        // fraction. First-person desired == position, so this is a no-op there
        // regardless of the fraction.
        self.position + (desired - self.position) * self.collision_frac
    }

    /// The *un-clamped* render-camera origin — the full desired pull-back for the
    /// current mode, before any Phase-2 collision clamp. This is the point a
    /// collision raycast aims toward (eye → here). Render-only.
    ///
    /// Phase 5: the pull-back runs along the **orbit** direction (yaw/pitch + the
    /// free-look offset), and its length is scaled by the active profile's
    /// `distance_mul` and the pitch→distance curve. Under the Neutral default
    /// (offsets 0, mul 1, curve k 0) this is identical to the Phase-1–4 behaviour.
    pub fn desired_render_eye(&self) -> Vec3 {
        // Profile + pitch scale only the pull-back DISTANCE (not the shoulder/up
        // offset). The Phase-2 collision_frac (applied in `render_eye`) is what
        // uniformly collapses the whole rig toward the eye against a wall.
        let dist_scale = self.profile.distance_mul
            * pitch_distance_scale(self.pitch, self.profile.pitch_distance_k)
            * self.user_distance;
        render_eye_scaled(self.position, self.orbit_forward(), self.orbit_right(), self.mode, dist_scale)
    }

    /// Phase 5 — the render look/orbit direction: `yaw`+`orbit_yaw_offset`,
    /// `pitch`+`orbit_pitch_offset`. Equals [`Self::forward`] (the aim direction)
    /// when there's no free-look offset. **Aim never reads this** — it's the
    /// decoupled *render* azimuth only.
    pub fn orbit_forward(&self) -> Vec3 {
        let y = self.yaw + self.orbit_yaw_offset;
        // Clamp the combined orbit pitch like the aim pitch (±89°) so a free-look
        // offset can never drive the render look to ±Y, which would degenerate the
        // `look_at_rh` view matrix (up == Vec3::Y). No-op at the default (offset 0,
        // since `pitch` is already clamped).
        let max_pitch = 89.0_f32.to_radians();
        let p = (self.pitch + self.orbit_pitch_offset).clamp(-max_pitch, max_pitch);
        Vec3::new(-y.sin() * p.cos(), p.sin(), -y.cos() * p.cos())
    }

    /// Phase 5 — horizontal right vector for the orbit azimuth (free-look aware).
    pub fn orbit_right(&self) -> Vec3 {
        let y = self.yaw + self.orbit_yaw_offset;
        Vec3::new(y.cos(), 0.0, -y.sin())
    }

    /// Phase 5 — the effective vertical FOV (degrees): base + the active profile's
    /// `fov_offset` + the pitch→FOV curve, in third-person only. First-person and
    /// the Neutral profile return the base `fov_y` unchanged.
    pub fn effective_fov_y(&self) -> f32 {
        // #44 — hold-to-zoom swaps the base FOV transiently. It bypasses the
        // personal-FOV clamp (the zoom value has its own narrower range) and
        // never mutates `fov_y`. Third-person profile/pitch offsets still apply
        // on top so zoom works from any perspective.
        let base = if self.zooming { self.zoom_fov } else { self.fov_y };
        if self.mode.is_third_person() {
            base + self.profile.fov_offset + pitch_fov_offset(self.pitch, self.profile.pitch_fov_k)
        } else {
            base
        }
    }

    /// Phase 5 — switch the active feel profile (Build / Combat / Neutral). The
    /// engine selects by context (Phase 6); render-only.
    /// Phase-6 enablement API — tested now; called once the context selector lands.
    #[allow(dead_code)]
    pub fn set_profile(&mut self, profile: CameraProfile) {
        self.profile = profile;
    }

    /// Phase 5 — apply a free-look delta (radians) to the decoupled *render* orbit
    /// without touching the aim (`yaw`/`pitch`). Arms the post-input cooldown so
    /// auto-recenter waits — **player input always wins**. Vertical orbit is
    /// clamped to `±ORBIT_PITCH_LIMIT`. Render-only.
    pub fn apply_free_look(&mut self, d_yaw: f32, d_pitch: f32) {
        self.orbit_yaw_offset += d_yaw;
        self.orbit_pitch_offset =
            (self.orbit_pitch_offset + d_pitch).clamp(-ORBIT_PITCH_LIMIT, ORBIT_PITCH_LIMIT);
        self.recenter_cooldown = self.recenter_delay;
    }

    /// Phase 5 — advance the velocity-gated auto-recenter one fixed-tick step.
    /// Counts down the post-input cooldown; then, only when the player isn't
    /// `steering` and is moving (`speed >= RECENTER_MIN_SPEED`), eases the
    /// free-look orbit offsets back toward 0 (behind the player). Player input
    /// always wins (it re-arms the cooldown). Render-only — aim is untouched.
    pub fn tick_auto_recenter(&mut self, steering: bool, speed: f32, dt: f32) {
        self.recenter_cooldown = (self.recenter_cooldown - dt).max(0.0);
        if should_auto_recenter(steering, speed, self.recenter_cooldown) {
            let s = self.recenter_speed;
            self.orbit_yaw_offset = approach_fraction(self.orbit_yaw_offset, 0.0, dt, s, s);
            self.orbit_pitch_offset = approach_fraction(self.orbit_pitch_offset, 0.0, dt, s, s);
        }
    }

    /// Phase 2 — advance the no-snap collision smoothing one fixed-tick step
    /// toward `target_frac` (the clear fraction from a world raycast, computed by
    /// the caller). Retract fast, ease out slow. Render-only.
    pub fn update_collision(&mut self, target_frac: f32, dt: f32) {
        self.collision_frac = approach_fraction(
            self.collision_frac,
            target_frac,
            dt,
            COLLISION_PULL_IN_SPEED,
            COLLISION_EASE_OUT_SPEED,
        );
    }

    /// Direction the camera is looking (unit vector).
    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            -self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }

    /// Right vector (horizontal).
    pub fn right(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin())
    }

    /// Horizontal forward (for movement — ignores pitch).
    pub fn horizontal_forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos())
    }

    pub fn view_matrix(&self) -> Mat4 {
        // The RENDER origin moves in third-person; Phase 5 lets the render LOOK
        // direction decouple from aim via the free-look orbit offset. With no
        // offset (the default) `orbit_forward() == forward()`, so first-person →
        // render_eye() == position and this is byte-identical to the old
        // `look_at_rh(position, position + forward, Y)`. Aim (forward()) is never
        // moved by this — only where the frame is viewed from / toward.
        let origin = self.render_eye();
        let target = origin + self.orbit_forward();
        glam::camera::rh::view::look_at_mat4(origin, target, Vec3::Y)
    }

    pub fn projection_matrix(&self) -> Mat4 {
        // Phase 5 — third-person may widen/narrow FOV by profile + pitch curve;
        // first-person / Neutral → effective_fov_y() == fov_y (unchanged).
        glam::camera::rh::proj::directx::perspective(
            self.effective_fov_y().to_radians(),
            self.aspect,
            self.near,
            self.far,
        )
    }

    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Rotate camera from mouse delta.
    pub fn rotate(&mut self, dx: f32, dy: f32, sensitivity: f32) {
        self.yaw -= dx * sensitivity;
        self.pitch -= dy * sensitivity; // Inverted Y for natural feel
        // Clamp pitch to prevent flipping
        let max_pitch = 89.0_f32.to_radians();
        self.pitch = self.pitch.clamp(-max_pitch, max_pitch);
    }
}

/// Camera uniform buffer data (matches Spec 03 bind group 0, slot 0).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4], // xyz = position, w = underwater flag
    pub sun_dir: [f32; 4],    // xyz = sun direction (normalized), w = sky brightness 0-1
    /// Spec 39 — distance fog, derived from the live render distance.
    /// xy = terrain fog (start, end); zw = water fog (start, end).
    pub fog: [f32; 4],
    /// User-facing render tuning. x = brightness control (graphics setting):
    /// the shader applies `pow(color, 1/x)`, so 1.0 = neutral, >1 brightens,
    /// <1 darkens. The shader treats x ≤ 0 as neutral, so a zeroed uniform can
    /// never black-screen. yzw reserved for future scalars. The live game loop
    /// sets x from `graphics.brightness`; other paths leave the 1.0 default.
    pub params: [f32; 4],
    /// Particle billboarding (2026-07-05): the camera's world-space RIGHT
    /// vector (xyz; w unused). Per-player, so split-screen billboards face
    /// each eye correctly — nothing else in the engine had true camera-facing
    /// quads (exhibits bake corners CPU-side for player 0 only).
    pub cam_right: [f32; 4],
    /// Camera world-space UP vector (xyz; w unused).
    pub cam_up: [f32; 4],
}

impl CameraUniform {
    pub fn from_camera(camera: &Camera) -> Self {
        // Shader fog/underwater is computed relative to where the frame is
        // actually viewed FROM — the render eye. First-person → render_eye() ==
        // position, so this is unchanged; third-person → fog tracks the pulled-
        // back camera. The underwater w-flag is overwritten by the caller from
        // the true eye (the body's eye decides "am I submerged"), see game_loop.
        let re = camera.render_eye();
        // Screen basis for particle billboards: pitch-aware, derived from the
        // RENDER look direction so third-person/free-look billboards face the
        // actual frame, not the aim.
        let fwd = camera.orbit_forward();
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let right = if right.length_squared() < 1e-6 { Vec3::X } else { right };
        let up = right.cross(fwd).normalize_or_zero();
        Self {
            view_proj: camera.view_projection_matrix().to_cols_array_2d(),
            camera_pos: [re.x, re.y, re.z, 0.0],
            sun_dir: [0.3, 1.0, 0.5, 1.0], // Default: noon
            // Default = the High-preset fog (terrain 128/160, water 96/140) so any
            // path that doesn't override it (headless screenshots) looks unchanged.
            // The live game loop overrides this per frame from `graphics`.
            fog: default_fog(),
            // Neutral brightness by default; the live game loop overrides x from
            // `graphics.brightness`. Menus/loading/screenshots stay neutral.
            params: [1.0, 0.0, 0.0, 0.0],
            cam_right: [right.x, right.y, right.z, 0.0],
            cam_up: [up.x, up.y, up.z, 0.0],
        }
    }
}

/// Fog (terrain_start, terrain_end, water_start, water_end) for a render
/// distance. Terrain fog comes straight from `graphics_settings::fog_distances`;
/// water fog is proportionally nearer (×0.75 / ×0.875), preserving the legacy
/// 96/140 pair at the default render distance of 10.
pub fn fog_vec(render_distance: i32) -> [f32; 4] {
    let (ts, te) = crate::graphics_settings::fog_distances(render_distance);
    [ts, te, ts * 0.75, te * 0.875]
}

/// The High-preset / default fog vec (render distance 10 → [128, 160, 96, 140]).
pub fn default_fog() -> [f32; 4] {
    fog_vec(crate::graphics_settings::DEFAULT_RENDER_DISTANCE)
}

/// Fog vec with everything pushed far away — used when the fog dial is off.
pub fn fog_disabled() -> [f32; 4] {
    [1.0e9, 1.0e9, 1.0e9, 1.0e9]
}

/// Compute sun direction and sky brightness from world time.
///
/// World time: 0 = midnight, ~6000 = sunrise (elevation 0, dim), 12000 = noon
/// (peak brightness), ~18000 = sunset, full cycle = 24000 ticks. Confirmed by
/// the math: `elevation = sin((t/24000)·τ − π/2)`, so t=0 → −1 (deep night)
/// and t=12000 → +1 (peak day). The earlier docstring's "0 = sunrise" was
/// wrong; spawning::tests pin the actual semantics (NIGHT=0, DAY=12000).
pub fn compute_sun(world_time: u32) -> (Vec3, f32) {
    let t = (world_time as f32 / 24000.0) * std::f32::consts::TAU;
    // Sun elevation: peaks at noon (6000 ticks), below horizon at night
    let elevation = (t - std::f32::consts::FRAC_PI_2).sin(); // noon=1, midnight=-1
    let azimuth = t.cos();

    let sun_dir = Vec3::new(azimuth * 0.5, elevation.max(0.05), 0.3).normalize();

    // Sky brightness: 1.0 at noon, transitions to 0.15 at night
    let brightness = if elevation > 0.0 {
        0.15 + 0.85 * elevation
    } else {
        0.15 + 0.15 * elevation.max(-1.0) // Dim moonlight
    };

    (sun_dir, brightness.clamp(0.05, 1.0))
}

/// Compute sky clear color from brightness.
pub fn sky_color(brightness: f32) -> (f64, f64, f64) {
    // Day: (0.53, 0.72, 0.90), Night: (0.01, 0.01, 0.05)
    let r = 0.01 + 0.52 * brightness as f64;
    let g = 0.01 + 0.71 * brightness as f64;
    let b = 0.05 + 0.85 * brightness as f64;
    (r, g, b)
}

/// Owner-inbox #18 Phase D — micro-model distance LOD selection. A chunk whose
/// centre projects within `lod_dist` (view-space forward distance in world units,
/// = clip-w under a perspective `view_proj`) draws its full 3D micro-model shell;
/// beyond that it falls back to the cheap billboard. Behind-camera chunks (w ≤ 0)
/// read as "near" but are frustum-culled before this matters. Pure + tested so the
/// LOD threshold is a regression-guarded selection, not just a magic comparison.
pub fn micro_chunk_is_near(view_proj: &Mat4, cx: i32, cy: i32, cz: i32, lod_dist: f32) -> bool {
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let half = cs / 2;
    let centre = Vec3::new(
        (cx * cs + half) as f32,
        (cy * cs + half) as f32,
        (cz * cs + half) as f32,
    );
    let w = (*view_proj * centre.extend(1.0)).w;
    w <= lod_dist
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_overrides_render_fov_without_touching_fov_y() {
        // #44 P3 — hold-to-zoom is a transient *render* FOV. It must narrow the
        // effective FOV while held and restore the configured FOV *exactly* on
        // release, never mutating the stored personal `fov_y`.
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.fov_y = 70.0;
        cam.zoom_fov = 20.0;
        assert_eq!(cam.effective_fov_y(), 70.0, "no zoom → configured FOV");
        cam.set_zoom(true);
        assert_eq!(cam.effective_fov_y(), 20.0, "zooming → narrow zoom FOV");
        assert_eq!(cam.fov_y, 70.0, "zoom must not mutate stored FOV");
        cam.set_zoom(false);
        assert_eq!(cam.effective_fov_y(), 70.0, "release restores configured FOV exactly");
        assert_eq!(cam.fov_y, 70.0);
    }

    #[test]
    fn zoom_bypasses_the_personal_fov_clamp() {
        // The zoom FOV (≈20°) is well below FOV_MIN (60°); zoom must reach it
        // without being pulled back into the 60–100° personal-FOV range.
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.fov_y = 90.0;
        cam.zoom_fov = 15.0;
        cam.set_zoom(true);
        assert!(cam.effective_fov_y() < crate::graphics_settings::FOV_MIN);
    }

    #[test]
    fn micro_lod_near_far_by_distance() {
        // Camera at (8,8,8) looking down -Z (rh perspective).
        let view = glam::camera::rh::view::look_at_mat4(
            Vec3::new(8.0, 8.0, 8.0),
            Vec3::new(8.0, 8.0, -100.0),
            Vec3::Y,
        );
        let proj = glam::camera::rh::proj::directx::perspective(70f32.to_radians(), 1.6, 0.1, 500.0);
        let vp = proj * view;
        // chunk (0,0,-1) centre ≈ (8,8,-8) → ~16 ahead; (0,0,-5) ≈ (8,8,-72) → ~80 ahead.
        assert!(micro_chunk_is_near(&vp, 0, 0, -1, 64.0), "16 ahead is near");
        assert!(!micro_chunk_is_near(&vp, 0, 0, -5, 64.0), "80 ahead is far");
        // A huge threshold keeps everything near (LOD effectively off).
        assert!(micro_chunk_is_near(&vp, 0, 0, -5, f32::MAX));
    }

    /// A camera at the origin looking along −Z (Minecraft default), 90° FOV,
    /// square aspect, near 0.1, far 200.
    fn test_view_proj() -> Mat4 {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.fov_y = 90.0;
        cam.far = 200.0;
        cam.view_projection_matrix()
    }

    /// AABB centred at `c` with half-extent 1.
    fn box_at(c: Vec3) -> (Vec3, Vec3) {
        (c - Vec3::ONE, c + Vec3::ONE)
    }

    #[test]
    fn box_directly_in_front_is_visible() {
        let f = Frustum::from_view_proj(test_view_proj());
        let (min, max) = box_at(Vec3::new(0.0, 0.0, -10.0)); // 10 ahead
        assert!(f.contains_aabb(min, max));
    }

    #[test]
    fn box_directly_behind_is_culled() {
        let f = Frustum::from_view_proj(test_view_proj());
        let (min, max) = box_at(Vec3::new(0.0, 0.0, 10.0)); // 10 behind
        assert!(!f.contains_aabb(min, max));
    }

    #[test]
    fn box_far_to_the_side_is_culled() {
        let f = Frustum::from_view_proj(test_view_proj());
        // Far off to the right but at the same depth — outside a 90° FOV.
        let (min, max) = box_at(Vec3::new(1000.0, 0.0, -10.0));
        assert!(!f.contains_aabb(min, max));
    }

    #[test]
    fn box_beyond_far_plane_is_culled() {
        let f = Frustum::from_view_proj(test_view_proj());
        let (min, max) = box_at(Vec3::new(0.0, 0.0, -500.0)); // past far=200
        assert!(!f.contains_aabb(min, max));
    }

    #[test]
    fn box_straddling_the_near_edge_is_kept() {
        // A box spanning the camera (partly in front, partly behind) must be
        // kept — it is partially visible, and the test is conservative.
        let f = Frustum::from_view_proj(test_view_proj());
        let (min, max) = box_at(Vec3::new(0.0, 0.0, 0.0));
        assert!(f.contains_aabb(min, max));
    }

    #[test]
    fn fog_vec_matches_legacy_at_default_distance() {
        // Legacy literals: terrain 128/160, water 96/140 at render distance 10.
        let f = fog_vec(10);
        assert_eq!(f, [128.0, 160.0, 96.0, 140.0]);
    }

    #[test]
    fn fog_vec_water_is_nearer_than_terrain_and_tracks_distance() {
        for rd in [4, 8, 14, 16] {
            let f = fog_vec(rd);
            assert!(f[0] < f[1], "terrain start before end");
            assert!(f[2] < f[3], "water start before end");
            assert!(f[2] < f[0], "water fog nearer than terrain (rd={rd})");
            assert!(f[3] < f[1], "water fog ends nearer than terrain (rd={rd})");
        }
        assert!(fog_vec(4)[1] < fog_vec(16)[1], "bigger render distance → farther fog");
    }

    #[test]
    fn fog_disabled_is_effectively_infinite() {
        let f = fog_disabled();
        assert!(f.iter().all(|&d| d >= 1.0e9));
    }

    #[test]
    fn camera_uniform_default_fog_is_high_preset() {
        let u = CameraUniform::from_camera(&Camera::new(Vec3::ZERO, 1.0));
        assert_eq!(u.fog, [128.0, 160.0, 96.0, 140.0]);
    }

    #[test]
    fn camera_uniform_brightness_defaults_neutral() {
        // params.x must default to 1.0 (neutral) so any path that doesn't set it
        // from the graphics setting renders unchanged — never black/over-bright.
        let u = CameraUniform::from_camera(&Camera::new(Vec3::ZERO, 1.0));
        assert_eq!(u.params[0], 1.0, "brightness must default neutral");
    }

    #[test]
    fn rotating_the_camera_changes_what_is_visible() {
        // Looking along −Z, a box at +Z (behind) is culled. Turn 180° (yaw=π)
        // to look along +Z and the same box becomes visible.
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.fov_y = 90.0;
        cam.far = 200.0;
        let (min, max) = box_at(Vec3::new(0.0, 0.0, 10.0));

        let f_fwd = Frustum::from_view_proj(cam.view_projection_matrix());
        assert!(!f_fwd.contains_aabb(min, max), "behind → culled");

        cam.yaw = std::f32::consts::PI; // face +Z
        let f_back = Frustum::from_view_proj(cam.view_projection_matrix());
        assert!(f_back.contains_aabb(min, max), "after turning → visible");
    }

    // ── Third-person camera — Phase 1, Task 1 (L1: pure render_eye) ──────────
    // The contract these guard: aim stays eye-anchored (render_eye is render-only),
    // first-person is identity, third-person pulls the render origin BEHIND the eye
    // (along -forward), to the shoulder side, and slightly up. Asserts structural
    // direction (behind / to-the-side / lifted), NOT exact const values — the
    // offsets are feel numbers tuned in Phase 6, the directions are the invariant.

    /// eye=origin, looking along -Z (yaw=0): forward=-Z, right=+X.
    fn fp_basis() -> (Vec3, Vec3, Vec3) {
        (Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 0.0))
    }

    #[test]
    fn camera_mode_defaults_to_first_person() {
        assert_eq!(CameraMode::default(), CameraMode::FirstPerson);
    }

    #[test]
    fn render_eye_first_person_is_the_eye_unchanged() {
        let (eye, fwd, right) = fp_basis();
        assert_eq!(render_eye_scaled(eye, fwd, right, CameraMode::FirstPerson, 1.0), eye);
    }

    #[test]
    fn render_eye_over_shoulder_pulls_behind_to_the_side_and_up() {
        let (eye, fwd, right) = fp_basis();
        let r = render_eye_scaled(eye, fwd, right, CameraMode::OverShoulder, 1.0);
        let off = r - eye;
        assert!(off.dot(fwd) < 0.0, "render eye is BEHIND the true eye");
        assert!(off.dot(right) > 0.0, "offset to the shoulder (right) side");
        assert!(off.dot(Vec3::Y) > 0.0, "lifted slightly above the eye");
        assert!(off.length() > 0.0, "third-person render eye actually moved");
    }

    #[test]
    fn render_eye_orbit_behind_pulls_straight_back_centred() {
        let (eye, fwd, right) = fp_basis();
        let r = render_eye_scaled(eye, fwd, right, CameraMode::OrbitBehind, 1.0);
        let off = r - eye;
        assert!(off.dot(fwd) < 0.0, "render eye is BEHIND the true eye");
        assert!(off.dot(right).abs() < 1e-4, "centred behind — no lateral shoulder offset");
        assert!(off.length() > 0.0, "third-person render eye actually moved");
    }

    #[test]
    fn orbit_behind_sits_farther_back_than_over_shoulder() {
        // Orbit (centred, MC-style 4-block) reads farther back than the closer
        // over-the-shoulder default — the two third-person modes are distinct.
        let (eye, fwd, right) = fp_basis();
        let os = (render_eye_scaled(eye, fwd, right, CameraMode::OverShoulder, 1.0) - eye).dot(-fwd);
        let ob = (render_eye_scaled(eye, fwd, right, CameraMode::OrbitBehind, 1.0) - eye).dot(-fwd);
        assert!(ob > os, "orbit-behind ({ob}) is farther back than over-shoulder ({os})");
    }

    // ── Third-person camera — Phase 1, Task 2 (Camera carries the mode; the
    //    view origin and uniform track render_eye; first-person is unchanged) ──

    #[test]
    fn camera_render_eye_is_position_in_first_person() {
        let cam = Camera::new(Vec3::new(1.0, 2.0, 3.0), 1.0); // default FirstPerson
        assert_eq!(cam.render_eye(), cam.position);
    }

    #[test]
    fn camera_render_eye_pulls_back_in_third_person() {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OverShoulder;
        let re = cam.render_eye();
        assert_ne!(re, cam.position, "third-person render eye left the body");
        assert!((re - cam.position).dot(cam.forward()) < 0.0, "behind the eye");
    }

    #[test]
    fn view_matrix_origin_is_the_render_eye_in_third_person() {
        // `look_at_rh` maps the camera origin to the view-space origin. In
        // third-person that origin must be the render eye, NOT the true eye.
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OrbitBehind;
        let v = cam.view_matrix();
        let render_mapped = (v * cam.render_eye().extend(1.0)).truncate();
        assert!(render_mapped.length() < 1e-4, "render eye maps to the view origin");
        let eye_mapped = (v * cam.position.extend(1.0)).truncate();
        assert!(eye_mapped.length() > 0.5, "the true eye is no longer the view origin");
    }

    #[test]
    fn camera_uniform_pos_tracks_render_eye_in_third_person() {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OverShoulder;
        let u = CameraUniform::from_camera(&cam);
        let re = cam.render_eye();
        assert!((u.camera_pos[0] - re.x).abs() < 1e-4);
        assert!((u.camera_pos[1] - re.y).abs() < 1e-4);
        assert!((u.camera_pos[2] - re.z).abs() < 1e-4);
    }

    #[test]
    fn first_person_render_and_uniform_are_unchanged() {
        // Regression guard: first-person (the default) keeps the exact old
        // behaviour — render origin == eye, uniform camera_pos == eye.
        let cam = Camera::new(Vec3::new(5.0, 64.0, -3.0), 1.6);
        assert_eq!(cam.render_eye(), cam.position);
        let u = CameraUniform::from_camera(&cam);
        assert_eq!(
            [u.camera_pos[0], u.camera_pos[1], u.camera_pos[2]],
            [cam.position.x, cam.position.y, cam.position.z]
        );
    }

    // ── Third-person camera — Phase 1, Task 4 (viewmodel/avatar gating) ──────

    #[test]
    fn first_person_shows_viewmodel_third_person_hides_it() {
        assert!(CameraMode::FirstPerson.shows_viewmodel());
        assert!(!CameraMode::OverShoulder.shows_viewmodel());
        assert!(!CameraMode::OrbitBehind.shows_viewmodel());
    }

    #[test]
    fn is_third_person_only_for_non_first_person() {
        assert!(!CameraMode::FirstPerson.is_third_person());
        assert!(CameraMode::OverShoulder.is_third_person());
        assert!(CameraMode::OrbitBehind.is_third_person());
    }

    // ── Third-person camera — Phase 1, Task 5 (perspective toggle cycle) ─────

    #[test]
    fn next_cycles_first_over_shoulder_orbit_and_back() {
        assert_eq!(CameraMode::FirstPerson.next(), CameraMode::OverShoulder);
        assert_eq!(CameraMode::OverShoulder.next(), CameraMode::OrbitBehind);
        assert_eq!(CameraMode::OrbitBehind.next(), CameraMode::FirstPerson);
        // Three taps return to the start — a closed 3-cycle.
        let mut m = CameraMode::FirstPerson;
        for _ in 0..3 {
            m = m.next();
        }
        assert_eq!(m, CameraMode::FirstPerson);
    }

    // ── Third-person camera — Phase 1, Task 6 (L3: you can see yourself) ─────

    #[test]
    fn you_see_yourself_in_third_person_but_not_first() {
        // A player standing at origin: eye +1.62, body a 0.6-wide × 1.8-tall box.
        let foot = Vec3::new(0.0, 64.0, 0.0);
        let eye = foot + Vec3::new(0.0, 1.62, 0.0);
        let body_min = foot - Vec3::new(0.3, 0.0, 0.3);
        let body_max = foot + Vec3::new(0.3, 1.8, 0.3);
        let body_center = (body_min + body_max) * 0.5;

        let mut cam = Camera::new(eye, 1.6);
        cam.fov_y = 70.0; // yaw/pitch default 0 → looking along -Z, level.

        // First-person: the render eye sits inside the body, so the body has
        // ~zero forward depth — you're in your own head, you can't see yourself.
        cam.mode = CameraMode::FirstPerson;
        let depth_fp = (body_center - cam.render_eye()).dot(cam.forward());
        assert!(depth_fp.abs() < 0.5, "first-person: body sits at the camera (depth {depth_fp})");

        // Third-person: the render camera is pulled BEHIND the body, so the body
        // is well in front (positive depth) AND its AABB is inside the frustum.
        for mode in [CameraMode::OverShoulder, CameraMode::OrbitBehind] {
            cam.mode = mode;
            let depth = (body_center - cam.render_eye()).dot(cam.forward());
            assert!(depth > 1.5, "{mode:?}: camera sits behind the body (depth {depth})");
            let frustum = Frustum::from_view_proj(cam.view_projection_matrix());
            assert!(
                frustum.contains_aabb(body_min, body_max),
                "{mode:?}: your own body must be inside the view frustum"
            );
        }
    }

    // ── Third-person camera — Phase 2 (L1: no-snap collision math) ───────────
    // The contract: the render eye clamps to BEFORE a solid surface (never sits
    // inside geometry), keeping a `margin` of air; recovery EASES out slowly
    // while retraction is FAST (the "punch in, never return" failure mode is the
    // slow-out; a hard snap is avoided by rate-limiting both directions). All
    // render-only — aim is untouched. Magnitudes are feel numbers (Phase 6).

    #[test]
    fn collision_fraction_clear_path_is_full_extension() {
        // No solid hit along the desired offset → camera extends fully (1.0).
        assert_eq!(collision_target_fraction(2.5, None, COLLISION_MARGIN), 1.0);
    }

    #[test]
    fn collision_fraction_zero_offset_is_full() {
        // First-person (no pull-back): nothing to clamp even with a hit at 0.
        assert_eq!(collision_target_fraction(0.0, Some(0.05), COLLISION_MARGIN), 1.0);
    }

    #[test]
    fn collision_fraction_clamps_before_a_near_wall() {
        // Desired pull-back 2.5; a wall 1.0 ahead; margin 0.25 → the camera may
        // extend (1.0-0.25)/2.5 = 0.3 of the way before it would clip.
        let f = collision_target_fraction(2.5, Some(1.0), 0.25);
        assert!((f - 0.3).abs() < 1e-4, "expected 0.3, got {f}");
    }

    #[test]
    fn collision_fraction_wall_inside_margin_pins_to_eye() {
        // A wall closer than the margin → fraction 0: the camera cannot leave the
        // eye (it would otherwise sit inside/through the wall).
        assert_eq!(collision_target_fraction(2.5, Some(0.1), 0.25), 0.0);
    }

    #[test]
    fn collision_fraction_far_wall_never_exceeds_full_extension() {
        // A hit beyond the desired offset must not push the fraction above 1.0.
        assert_eq!(collision_target_fraction(2.5, Some(10.0), 0.25), 1.0);
    }

    #[test]
    fn approach_pulls_in_faster_than_it_eases_out() {
        let dt = 0.05;
        // Retract from full toward 0 (obstruction appeared): uses the fast speed.
        let retract = approach_fraction(1.0, 0.0, dt, COLLISION_PULL_IN_SPEED, COLLISION_EASE_OUT_SPEED);
        // Ease out from 0 toward full (path cleared): uses the slow speed.
        let ease = approach_fraction(0.0, 1.0, dt, COLLISION_PULL_IN_SPEED, COLLISION_EASE_OUT_SPEED);
        let pull_in_step = 1.0 - retract; // how far it moved inward
        let ease_out_step = ease; // how far it moved outward from 0
        assert!(pull_in_step > 0.0 && ease_out_step > 0.0, "both move toward target");
        assert!(
            pull_in_step > ease_out_step,
            "pull-in ({pull_in_step}) must be faster than ease-out ({ease_out_step})"
        );
    }

    #[test]
    fn approach_snaps_to_target_without_overshoot() {
        // A remaining delta smaller than one step lands exactly on the target —
        // no oscillation past it (either direction).
        assert_eq!(approach_fraction(0.5, 0.5001, 1.0, COLLISION_PULL_IN_SPEED, COLLISION_EASE_OUT_SPEED), 0.5001);
        assert_eq!(approach_fraction(0.5, 0.4999, 1.0, COLLISION_PULL_IN_SPEED, COLLISION_EASE_OUT_SPEED), 0.4999);
    }

    #[test]
    fn approach_eventually_fully_recovers() {
        // Repeated ease-out from a clamped fraction converges to full extension —
        // the camera always comes back (no permanent punch-in).
        let mut f = 0.0;
        for _ in 0..400 {
            f = approach_fraction(f, 1.0, 0.05, COLLISION_PULL_IN_SPEED, COLLISION_EASE_OUT_SPEED);
        }
        assert!((f - 1.0).abs() < 1e-6, "did not fully recover, got {f}");
    }

    #[test]
    fn camera_render_eye_scales_by_collision_fraction() {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OrbitBehind;
        let full = cam.render_eye(); // default collision_frac == 1.0 → full pull-back
        cam.collision_frac = 0.5;
        let half = cam.render_eye();
        // Half the fraction → render eye sits halfway between the eye and full.
        let expected = cam.position + (full - cam.position) * 0.5;
        assert!((half - expected).length() < 1e-5, "half={half}, expected={expected}");
        assert!(
            (half - cam.position).length() < (full - cam.position).length(),
            "clamped render eye is closer to the body than full extension"
        );
    }

    #[test]
    fn first_person_render_eye_ignores_collision_fraction() {
        // No offset in first-person, so the fraction can never move the render eye.
        let mut cam = Camera::new(Vec3::new(1.0, 2.0, 3.0), 1.0);
        cam.collision_frac = 0.3;
        assert_eq!(cam.render_eye(), cam.position);
    }

    #[test]
    fn camera_collision_frac_defaults_to_full_extension() {
        // A fresh camera is never pre-clamped — Phase 1 behaviour is unchanged.
        assert_eq!(Camera::new(Vec3::ZERO, 1.0).collision_frac, 1.0);
    }

    #[test]
    fn update_collision_smooths_toward_target() {
        // The Camera method drives the same no-snap smoothing the game loop uses.
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OrbitBehind;
        cam.update_collision(0.0, 0.05); // obstruction → start retracting
        assert!(cam.collision_frac < 1.0 && cam.collision_frac >= 0.0);
        // Ease back out once clear: converges to full.
        for _ in 0..400 {
            cam.update_collision(1.0, 0.05);
        }
        assert!((cam.collision_frac - 1.0).abs() < 1e-6);
    }

    // ── Third-person camera — Phase 3 (L1: avatar fade-on-occlusion curve) ───
    // When Phase-2 collision pulls the camera in close to the body (small
    // collision_frac), the body would occlude the crosshair, so the self-avatar
    // fades toward transparent. Above the start threshold it stays fully opaque.

    #[test]
    fn avatar_fade_is_opaque_when_camera_is_extended() {
        assert_eq!(avatar_fade_alpha(1.0), 1.0);
        assert_eq!(avatar_fade_alpha(AVATAR_FADE_START), 1.0);
        assert_eq!(avatar_fade_alpha(0.95), 1.0);
    }

    #[test]
    fn avatar_fade_ramps_down_as_camera_pulls_in() {
        let mid = avatar_fade_alpha(AVATAR_FADE_START * 0.5);
        assert!(mid > AVATAR_FADE_MIN_ALPHA && mid < 1.0, "mid-pull is partially faded, got {mid}");
        assert!(
            (avatar_fade_alpha(0.0) - AVATAR_FADE_MIN_ALPHA).abs() < 1e-6,
            "fully pulled in → the minimum alpha (still faintly visible)"
        );
    }

    #[test]
    fn avatar_fade_is_monotonic_and_bounded() {
        let mut prev = avatar_fade_alpha(0.0);
        let mut c = 0.0;
        while c <= 1.0 {
            let a = avatar_fade_alpha(c);
            assert!((AVATAR_FADE_MIN_ALPHA..=1.0).contains(&a), "alpha {a} out of range at frac {c}");
            assert!(a >= prev - 1e-6, "fade must not decrease as the camera extends (frac {c})");
            prev = a;
            c += 0.05;
        }
    }

    // ── Third-person camera — Phase 5 (L1: feel — profiles, curves, recenter) ─
    // Every Phase-5 lever defaults to NEUTRAL so the *approved Phase-1 feel is
    // unchanged*; Build/Combat profiles + free-look + auto-recenter are mechanisms
    // whose values + enablement are tuned in the Phase-6 playtest. All decoupling
    // is render-only — aim (forward()/yaw/pitch) is never touched, so the
    // eye-anchored invariant holds (also guarded by the TestHost L3 tests).

    #[test]
    fn neutral_profile_is_a_no_op_preserving_phase1_feel() {
        assert_eq!(NEUTRAL_PROFILE.distance_mul, 1.0);
        assert_eq!(NEUTRAL_PROFILE.fov_offset, 0.0);
        // Zero curve coefficients → the pitch curves are identity under Neutral.
        assert_eq!(pitch_distance_scale(-1.0, NEUTRAL_PROFILE.pitch_distance_k), 1.0);
        assert_eq!(pitch_fov_offset(-1.0, NEUTRAL_PROFILE.pitch_fov_k), 0.0);
    }

    #[test]
    fn build_profile_is_further_and_wider_than_combat() {
        assert!(BUILD_PROFILE.distance_mul > COMBAT_PROFILE.distance_mul, "build sits further back");
        assert!(BUILD_PROFILE.fov_offset > COMBAT_PROFILE.fov_offset, "build wider; combat tighter");
    }

    #[test]
    fn pitch_distance_scale_pulls_back_looking_down_closer_looking_up() {
        let k = BUILD_PROFILE.pitch_distance_k;
        let down = pitch_distance_scale(-1.2, k); // looking down (pitch<0)
        let level = pitch_distance_scale(0.0, k);
        let up = pitch_distance_scale(1.2, k); // looking up (pitch>0)
        assert!((level - 1.0).abs() < 1e-6, "level look is unscaled");
        assert!(down > 1.0, "looking down pulls the camera back ({down})");
        assert!(up < 1.0, "looking up brings it closer ({up})");
        assert!((PITCH_DISTANCE_MIN..=PITCH_DISTANCE_MAX).contains(&pitch_distance_scale(-10.0, k)));
        assert!((PITCH_DISTANCE_MIN..=PITCH_DISTANCE_MAX).contains(&pitch_distance_scale(10.0, k)));
    }

    #[test]
    fn pitch_fov_offset_widens_looking_down() {
        let k = BUILD_PROFILE.pitch_fov_k;
        assert!(pitch_fov_offset(-1.0, k) > 0.0, "looking down widens FOV (overview)");
        assert!(pitch_fov_offset(1.0, k) < 0.0, "looking up narrows FOV");
        assert_eq!(pitch_fov_offset(0.0, k), 0.0);
    }

    #[test]
    fn camera_profile_scales_the_render_pull_back() {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OrbitBehind;
        let neutral = (cam.desired_render_eye() - cam.position).length();
        cam.set_profile(BUILD_PROFILE);
        let build = (cam.desired_render_eye() - cam.position).length();
        cam.set_profile(COMBAT_PROFILE);
        let combat = (cam.desired_render_eye() - cam.position).length();
        assert!(build > neutral, "build profile pulls further back ({build} vs {neutral})");
        assert!(combat < neutral, "combat profile sits closer ({combat} vs {neutral})");
    }

    // ── Phase 6 — the camera reads live (settings-driven) feel values ────────

    #[test]
    fn user_distance_zooms_the_third_person_pullback() {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OrbitBehind;
        // Measure the longitudinal pull-back (along -orbit_forward); the fixed
        // vertical lift is intentionally NOT scaled, so total length isn't linear.
        let back = |c: &Camera| (c.desired_render_eye() - c.position).dot(-c.orbit_forward());
        let base = back(&cam);
        cam.user_distance = 1.5;
        let zoomed = back(&cam);
        assert!((zoomed - base * 1.5).abs() < 1e-4, "user distance multiplies pull-back ({zoomed} vs {base})");
    }

    #[test]
    fn apply_free_look_arms_to_the_live_recenter_delay() {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.recenter_delay = 0.4; // settings-driven
        cam.apply_free_look(0.2, 0.0);
        assert_eq!(cam.recenter_cooldown, 0.4, "cooldown armed to the live delay, not a const");
    }

    #[test]
    fn auto_recenter_eases_at_the_live_speed() {
        // A higher live recenter speed eases the offset back further per step.
        let step = |speed: f32| {
            let mut c = Camera::new(Vec3::ZERO, 1.0);
            c.mode = CameraMode::OrbitBehind;
            c.recenter_speed = speed;
            c.apply_free_look(0.9, 0.0);
            c.recenter_cooldown = 0.0; // past the cooldown
            let before = c.orbit_yaw_offset;
            c.tick_auto_recenter(false, 5.0, 0.05);
            before - c.orbit_yaw_offset
        };
        assert!(step(5.0) > step(1.0), "faster live recenter speed → bigger ease-back step");
    }

    #[test]
    fn profile_scales_pullback_distance_not_the_shoulder_offset() {
        // A wider/closer profile must change only the pull-back DISTANCE, not the
        // over-the-shoulder lateral + vertical offset (else the rig drifts
        // sideways/up as you change profile or pitch). yaw 0 → forward -Z, right +X.
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OverShoulder;
        let neutral = cam.desired_render_eye() - cam.position;
        cam.set_profile(BUILD_PROFILE);
        let build = cam.desired_render_eye() - cam.position;
        // Back distance (along +Z = -forward) scales up with the profile…
        assert!(build.z > neutral.z * 1.2, "build pulls further back ({} vs {})", build.z, neutral.z);
        // …but the lateral shoulder (x) and lift (y) are UNCHANGED.
        assert!((build.x - neutral.x).abs() < 1e-5, "shoulder offset must not scale ({} vs {})", build.x, neutral.x);
        assert!((build.y - neutral.y).abs() < 1e-5, "vertical lift must not scale ({} vs {})", build.y, neutral.y);
    }

    #[test]
    fn default_camera_uses_the_neutral_profile() {
        // A fresh camera must render exactly as Phase 1–4 (approved feel).
        let cam = Camera::new(Vec3::new(1.0, 2.0, 3.0), 1.6);
        assert_eq!(cam.profile, NEUTRAL_PROFILE);
        assert_eq!(cam.orbit_yaw_offset, 0.0);
        assert_eq!(cam.orbit_pitch_offset, 0.0);
    }

    #[test]
    fn free_look_moves_the_render_camera_but_never_the_aim() {
        // The decoupled orbit offset moves where the frame is viewed from, but the
        // aim ray (forward() from yaw/pitch) is untouched — the invariant under
        // free-look. (The TestHost crosshair_target tests guard the full path.)
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OrbitBehind;
        let aim_before = cam.forward();
        let eye_before = cam.desired_render_eye();
        cam.apply_free_look(0.6, 0.2);
        assert_eq!(cam.forward(), aim_before, "free-look must NOT move the aim ray");
        assert_ne!(cam.desired_render_eye(), eye_before, "free-look orbits the render camera");
    }

    #[test]
    fn apply_free_look_arms_the_cooldown_input_always_wins() {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.apply_free_look(0.3, 0.0);
        assert!(cam.recenter_cooldown > 0.0, "input arms the post-input cooldown");
        // Even moving and not steering, recenter is suppressed during cooldown.
        assert!(!should_auto_recenter(false, 5.0, cam.recenter_cooldown));
    }

    #[test]
    fn should_auto_recenter_only_when_idle_moving_and_off_cooldown() {
        assert!(should_auto_recenter(false, 5.0, 0.0), "moving, not steering, no cooldown → recenter");
        assert!(!should_auto_recenter(true, 5.0, 0.0), "steering wins — no auto-recenter");
        assert!(!should_auto_recenter(false, 0.0, 0.0), "standing still → don't recenter");
        assert!(!should_auto_recenter(false, 5.0, 0.5), "still in post-input cooldown → wait");
    }

    #[test]
    fn auto_recenter_eases_orbit_offset_back_to_zero_only_when_idle() {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.mode = CameraMode::OrbitBehind;
        cam.apply_free_look(0.8, 0.3);
        let off0 = cam.orbit_yaw_offset;
        // Steering: the offset must NOT recenter (deliberate orbiting wins).
        cam.tick_auto_recenter(true, 5.0, 0.05);
        assert_eq!(cam.orbit_yaw_offset, off0, "deliberate steering is never overridden");
        // Cooldown elapses, then idle-moving eases the orbit back toward behind.
        for _ in 0..40 {
            cam.tick_auto_recenter(false, 5.0, 0.05);
        }
        assert!(cam.orbit_yaw_offset.abs() < off0.abs(), "idle movement recenters the orbit");
        // Given time it fully recenters behind the player.
        for _ in 0..400 {
            cam.tick_auto_recenter(false, 5.0, 0.05);
        }
        assert!(
            cam.orbit_yaw_offset.abs() < 1e-3 && cam.orbit_pitch_offset.abs() < 1e-3,
            "fully returns behind the player"
        );
    }
}

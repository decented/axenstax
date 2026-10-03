//! The cinematic Director rig (Phase 1) — a render-only camera detached from the
//! avatar, with six modes. It is **never** the gameplay camera: the gameplay
//! `slot.camera` is untouched (so third-person, eye-anchored aim and body-keyed
//! chunk streaming stay correct), and the body is frozen via the existing modal
//! input-gate while the Director is active. `as_render_camera()` produces a
//! first-person `Camera` (so `render_eye() == position`, no third-person
//! pull-back leak) used only to render the primary viewport.
//!
//! All the per-mode math is pure (`update` takes `dt` + inputs + target
//! snapshots), so it unit-tests in isolation. Native-only (the web bundle and
//! the spectator/anti-X-ray surface are untouched); the replay Director (Phase 2)
//! reuses the same rig over recorded `TargetSnapshot`s.

use glam::Vec3;

use crate::camera_path::{CameraPath, CameraPose};

/// Matches `physics::FLY_SPEED` (private there); kept in sync deliberately.
const DIRECTOR_FLY_SPEED: f32 = 10.89;
const MAX_PITCH: f32 = 1.553_343; // 89° in radians

/// The six cinematic modes, cycled by `next()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DirectorMode {
    /// Manual noclip flight (the freecam).
    #[default]
    FreeFly,
    /// Play a keyframed camera path (dolly).
    Path,
    /// Locked position; aim only (a fixed wide shot).
    Tripod,
    /// Chase a target at a yaw-rotated offset; free aim.
    Follow,
    /// Orbit a target, aim auto-locked to it.
    LookAt,
    /// Render from a target player's eye (first-person POV).
    Pov,
}

impl DirectorMode {
    /// Closed six-cycle (for the F7 mode key).
    pub fn next(self) -> Self {
        match self {
            DirectorMode::FreeFly => DirectorMode::Path,
            DirectorMode::Path => DirectorMode::Tripod,
            DirectorMode::Tripod => DirectorMode::Follow,
            DirectorMode::Follow => DirectorMode::LookAt,
            DirectorMode::LookAt => DirectorMode::Pov,
            DirectorMode::Pov => DirectorMode::FreeFly,
        }
    }

    /// No caller — no HUD currently displays the Director's current mode
    /// name.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            DirectorMode::FreeFly => "Free-fly",
            DirectorMode::Path => "Path",
            DirectorMode::Tripod => "Tripod",
            DirectorMode::Follow => "Follow",
            DirectorMode::LookAt => "Look-at",
            DirectorMode::Pov => "POV",
        }
    }
}

/// A target the Director can follow / look-at / POV — a player's eye + look
/// angles, snapshotted each frame (live) or read from a recording (Phase 2).
#[derive(Clone, Copy, Debug)]
pub struct TargetSnapshot {
    pub eye: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

/// Per-frame Director input (mouse already scaled by sensitivity; move axes in
/// `[-1, 1]`).
#[derive(Clone, Copy, Debug, Default)]
pub struct DirectorInputs {
    pub look_dx: f32,
    pub look_dy: f32,
    pub fwd: f32,
    pub right: f32,
    pub up: f32,
    pub sprint: bool,
}

/// The Director camera rig + its authoring state. Transient — never serialized.
#[derive(Clone, Debug)]
pub struct DirectorCamera {
    pub active: bool,
    pub mode: DirectorMode,
    pub pose: CameraPose,
    pub fly_speed: f32,
    pub sprint_mult: f32,
    pub target_index: usize,
    pub follow_offset: Vec3,
    pub orbit_radius: f32,
    pub orbit_yaw: f32,
    pub orbit_pitch: f32,
    pub path: CameraPath,
    /// `Some(t)` while a path is playing; `None` when idle/authoring.
    pub playback: Option<f32>,
    pub playback_speed: f32,
    pub loop_path: bool,
    pub hide_hud: bool,
    /// Wall-clock of the last update, for per-frame dt (managed by the caller).
    pub last_update: Option<web_time::Instant>,
}

impl Default for DirectorCamera {
    fn default() -> Self {
        Self {
            active: false,
            mode: DirectorMode::FreeFly,
            pose: CameraPose { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0, fov: 70.0 },
            fly_speed: DIRECTOR_FLY_SPEED,
            sprint_mult: 2.0,
            target_index: 0,
            follow_offset: Vec3::new(0.0, 2.0, 6.0),
            orbit_radius: 6.0,
            orbit_yaw: 0.0,
            orbit_pitch: 0.3,
            path: CameraPath::new(),
            playback: None,
            playback_speed: 1.0,
            loop_path: false,
            hide_hud: true,
            last_update: None,
        }
    }
}

impl DirectorCamera {
    /// Advance one frame. Pure: all state is in `self` + the arguments.
    pub fn update(&mut self, dt: f32, inp: &DirectorInputs, targets: &[TargetSnapshot]) {
        match self.mode {
            DirectorMode::FreeFly => {
                self.apply_look(inp);
                let dir = forward(self.pose.yaw, self.pose.pitch) * inp.fwd
                    + right(self.pose.yaw) * inp.right
                    + Vec3::Y * inp.up;
                if dir.length_squared() > 1e-9 {
                    let speed = self.fly_speed * if inp.sprint { self.sprint_mult } else { 1.0 };
                    self.pose.position += dir.normalize() * speed * dt;
                }
            }
            DirectorMode::Tripod => self.apply_look(inp),
            DirectorMode::Path => {
                if let Some(mut p) = self.playback {
                    let dur = self.path.duration();
                    p += dt * self.playback_speed;
                    p = if self.loop_path && dur > 0.0 { p.rem_euclid(dur) } else { p.clamp(0.0, dur) };
                    self.playback = Some(p);
                    if let Some(sampled) = self.path.sample(p) {
                        self.pose = sampled;
                    }
                }
            }
            DirectorMode::Follow => {
                if let Some(tg) = targets.get(self.target_index) {
                    self.pose.position = tg.eye + rotate_offset(self.follow_offset, tg.yaw);
                }
                self.apply_look(inp);
            }
            DirectorMode::LookAt => {
                self.orbit_yaw -= inp.look_dx;
                self.orbit_pitch = (self.orbit_pitch - inp.look_dy).clamp(-MAX_PITCH, MAX_PITCH);
                if inp.fwd != 0.0 {
                    self.orbit_radius =
                        (self.orbit_radius - inp.fwd * self.fly_speed * dt).clamp(1.0, 256.0);
                }
                if let Some(tg) = targets.get(self.target_index) {
                    self.pose.position =
                        tg.eye + orbit_vec(self.orbit_radius, self.orbit_yaw, self.orbit_pitch);
                    let look = (tg.eye - self.pose.position).normalize_or_zero();
                    if look.length_squared() > 1e-9 {
                        self.pose.yaw = (-look.x).atan2(-look.z);
                        self.pose.pitch = look.y.clamp(-1.0, 1.0).asin();
                    }
                }
            }
            DirectorMode::Pov => {
                if let Some(tg) = targets.get(self.target_index) {
                    self.pose.position = tg.eye;
                    self.pose.yaw = tg.yaw;
                    self.pose.pitch = tg.pitch;
                }
            }
        }
    }

    fn apply_look(&mut self, inp: &DirectorInputs) {
        self.pose.yaw -= inp.look_dx;
        self.pose.pitch = (self.pose.pitch - inp.look_dy).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Build the first-person render camera for this pose. `mode = FirstPerson`
    /// guarantees `render_eye() == position` (no third-person pull-back leak).
    pub fn as_render_camera(&self, aspect: f32, fallback_fov: f32) -> crate::camera::Camera {
        let mut cam = crate::camera::Camera::new(self.pose.position, aspect);
        cam.yaw = self.pose.yaw;
        cam.pitch = self.pose.pitch;
        cam.fov_y = if self.pose.fov > 1.0 { self.pose.fov } else { fallback_fov };
        cam.mode = crate::camera::CameraMode::FirstPerson;
        cam
    }

    /// Append a keyframe at the current pose at time `t`.
    pub fn drop_keyframe(&mut self, t: f32) {
        self.path.push_at(t, self.pose);
    }

    /// Seamlessly start the Director from the gameplay camera's current view.
    pub fn seed_from(&mut self, cam: &crate::camera::Camera) {
        self.pose = CameraPose {
            position: cam.position,
            yaw: cam.yaw,
            pitch: cam.pitch,
            fov: cam.fov_y,
        };
    }
}

fn forward(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos())
}

fn right(yaw: f32) -> Vec3 {
    Vec3::new(yaw.cos(), 0.0, -yaw.sin())
}

/// Horizontal "behind" basis (−horizontal_forward).
fn back(yaw: f32) -> Vec3 {
    Vec3::new(yaw.sin(), 0.0, yaw.cos())
}

/// Place a local follow offset (`x`=right, `y`=up, `z`=behind) in world space by
/// the target's yaw.
fn rotate_offset(offset: Vec3, yaw: f32) -> Vec3 {
    right(yaw) * offset.x + Vec3::Y * offset.y + back(yaw) * offset.z
}

fn orbit_vec(radius: f32, yaw: f32, pitch: f32) -> Vec3 {
    radius * Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> DirectorInputs {
        DirectorInputs::default()
    }

    // (9) FreeFly fwd=1 → moves fly_speed*dt along forward().
    #[test]
    fn freefly_forward_moves_along_view() {
        let mut d = DirectorCamera::default();
        let inp = DirectorInputs { fwd: 1.0, ..inputs() };
        d.update(0.1, &inp, &[]);
        let expected = forward(0.0, 0.0) * d.fly_speed * 0.1;
        assert!((d.pose.position - expected).length() < 1e-4, "{:?} vs {:?}", d.pose.position, expected);
    }

    // (10) up = ±1 → ±Y only.
    #[test]
    fn freefly_up_is_vertical_only() {
        let mut d = DirectorCamera::default();
        d.update(0.1, &DirectorInputs { up: 1.0, ..inputs() }, &[]);
        assert!(d.pose.position.x.abs() < 1e-5 && d.pose.position.z.abs() < 1e-5);
        assert!((d.pose.position.y - d.fly_speed * 0.1).abs() < 1e-4);
        let mut d2 = DirectorCamera::default();
        d2.update(0.1, &DirectorInputs { up: -1.0, ..inputs() }, &[]);
        assert!(d2.pose.position.y < 0.0);
    }

    // (11) sprint scales speed by sprint_mult.
    #[test]
    fn freefly_sprint_scales_speed() {
        let mut slow = DirectorCamera::default();
        slow.update(0.1, &DirectorInputs { fwd: 1.0, ..inputs() }, &[]);
        let mut fast = DirectorCamera::default();
        fast.update(0.1, &DirectorInputs { fwd: 1.0, sprint: true, ..inputs() }, &[]);
        let ratio = fast.pose.position.length() / slow.pose.position.length();
        assert!((ratio - d_default_sprint()).abs() < 1e-3, "ratio={ratio}");
    }
    fn d_default_sprint() -> f32 {
        DirectorCamera::default().sprint_mult
    }

    // (12) LookAt: the camera aims at the target.
    #[test]
    fn lookat_aims_at_target() {
        let mut d = DirectorCamera::default();
        d.mode = DirectorMode::LookAt;
        let tg = [TargetSnapshot { eye: Vec3::new(10.0, 1.62, 10.0), yaw: 0.0, pitch: 0.0 }];
        d.update(0.016, &inputs(), &tg);
        let look = (tg[0].eye - d.pose.position).normalize();
        let fwd = forward(d.pose.yaw, d.pose.pitch);
        assert!(fwd.dot(look) > 0.999, "dot={}", fwd.dot(look));
    }

    // (13) Pov == the target snapshot.
    #[test]
    fn pov_matches_target() {
        let mut d = DirectorCamera::default();
        d.mode = DirectorMode::Pov;
        let tg = [TargetSnapshot { eye: Vec3::new(5.0, 2.0, 5.0), yaw: 1.0, pitch: 0.3 }];
        d.update(0.016, &inputs(), &tg);
        assert_eq!(d.pose.position, Vec3::new(5.0, 2.0, 5.0));
        assert_eq!(d.pose.yaw, 1.0);
        assert_eq!(d.pose.pitch, 0.3);
    }

    // (14) Follow holds the yaw-rotated offset behind the target.
    #[test]
    fn follow_holds_rotated_offset() {
        let mut d = DirectorCamera::default();
        d.mode = DirectorMode::Follow;
        d.follow_offset = Vec3::new(0.0, 2.0, 6.0);
        let eye = Vec3::new(0.0, 1.62, 0.0);
        d.update(0.016, &inputs(), &[TargetSnapshot { eye, yaw: 0.0, pitch: 0.0 }]);
        assert!((d.pose.position - (eye + Vec3::new(0.0, 2.0, 6.0))).length() < 1e-4);
        // yaw 90° rotates the offset around Y.
        let mut d2 = DirectorCamera::default();
        d2.mode = DirectorMode::Follow;
        d2.follow_offset = Vec3::new(0.0, 2.0, 6.0);
        d2.update(0.016, &inputs(), &[TargetSnapshot { eye, yaw: std::f32::consts::FRAC_PI_2, pitch: 0.0 }]);
        assert!((d2.pose.position - (eye + Vec3::new(6.0, 2.0, 0.0))).length() < 1e-3, "{:?}", d2.pose.position);
    }

    // (15) mode.next() cycles all six and wraps.
    #[test]
    fn mode_cycle_visits_all_six() {
        let mut m = DirectorMode::FreeFly;
        let mut seen = vec![m];
        for _ in 0..5 {
            m = m.next();
            seen.push(m);
        }
        assert_eq!(m.next(), DirectorMode::FreeFly, "wraps");
        seen.sort_by_key(|x| *x as u8);
        seen.dedup();
        assert_eq!(seen.len(), 6, "all six distinct");
    }

    // (16) drop_keyframe appends at increasing t.
    #[test]
    fn drop_keyframe_appends() {
        let mut d = DirectorCamera::default();
        d.drop_keyframe(0.0);
        d.pose.position = Vec3::new(1.0, 0.0, 0.0);
        d.drop_keyframe(1.0);
        assert_eq!(d.path.len(), 2);
        let ts: Vec<f32> = d.path.keys().iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 1.0]);
    }

    // (17) as_render_camera renders from the pose position (no third-person pull-back).
    #[test]
    fn render_camera_is_eye_anchored() {
        let mut d = DirectorCamera::default();
        d.pose.position = Vec3::new(3.0, 4.0, 5.0);
        let cam = d.as_render_camera(1.6, 70.0);
        assert!((cam.render_eye() - Vec3::new(3.0, 4.0, 5.0)).length() < 1e-4);
        assert_eq!(cam.mode, crate::camera::CameraMode::FirstPerson);
    }
}

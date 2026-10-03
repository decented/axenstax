//! Pure keyframe camera-path interpolation for the cinematic Director (Phase 1).
//!
//! A `CameraPath` is an ordered list of `Keyframe`s (a `CameraPose` at a time
//! `t` in seconds). `sample(t)` returns the smoothly-interpolated pose:
//! **uniform Catmull-Rom** for position + FOV (the curve passes through every
//! control point), and **shortest-arc angle interpolation** for yaw/pitch (so a
//! 350°→10° move travels +20°, not −340°). End segments clamp their phantom
//! neighbours (`p0 = p1`, `p3 = p2`), which makes a 2-key path a clean straight
//! segment.
//!
//! No egui / winit / wgpu — just `glam`, so it unit-tests in isolation. The
//! Director (`director.rs`) drives it; the replay Director (Phase 2) reuses it.
//! Centripetal Catmull-Rom + arc-length reparameterisation are a documented
//! Phase-2 refinement.

use glam::Vec3;
use std::f32::consts::{PI, TAU};

/// A camera state at a point on a path. `fov` is vertical FOV in **degrees**
/// (matching `Camera::fov_y`); `yaw`/`pitch` are radians (Minecraft convention).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
}

/// A pose pinned to a time `t` (seconds from the path start).
#[derive(Clone, Copy, Debug)]
pub struct Keyframe {
    pub t: f32,
    pub pose: CameraPose,
}

/// An ordered keyframe path. Invariant: `keys` is sorted ascending by `t`.
#[derive(Clone, Debug, Default)]
pub struct CameraPath {
    keys: Vec<Keyframe>,
}

impl CameraPath {
    pub fn new() -> Self {
        Self { keys: Vec::new() }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn keys(&self) -> &[Keyframe] {
        &self.keys
    }

    pub fn clear(&mut self) {
        self.keys.clear();
    }

    /// Insert a keyframe, keeping `keys` sorted by `t` (stable for equal `t`).
    pub fn push_at(&mut self, t: f32, pose: CameraPose) {
        let idx = self.keys.partition_point(|k| k.t <= t);
        self.keys.insert(idx, Keyframe { t, pose });
    }

    /// Total path length in seconds (the last keyframe's `t`); `0.0` if empty.
    pub fn duration(&self) -> f32 {
        self.keys.last().map(|k| k.t).unwrap_or(0.0)
    }

    /// Interpolated pose at time `t` (clamped to `[0, duration]`).
    /// `None` only for an empty path; a single-key path is constant.
    pub fn sample(&self, t: f32) -> Option<CameraPose> {
        match self.keys.len() {
            0 => None,
            1 => Some(self.keys[0].pose),
            n => {
                let t = t.clamp(0.0, self.duration());
                // Segment [i, i+1] with keys[i].t <= t <= keys[i+1].t.
                let mut i = 0;
                while i + 1 < n && self.keys[i + 1].t < t {
                    i += 1;
                }
                let k1 = self.keys[i];
                let k2 = self.keys[i + 1];
                let seg = (k2.t - k1.t).max(1e-6);
                let u = ((t - k1.t) / seg).clamp(0.0, 1.0);
                // Phantom neighbours clamp at the ends.
                let p0 = self.keys[i.saturating_sub(1)].pose;
                let p3 = self.keys[(i + 2).min(n - 1)].pose;
                Some(CameraPose {
                    position: catmull_rom_vec(p0.position, k1.pose.position, k2.pose.position, p3.position, u),
                    yaw: angle_lerp(k1.pose.yaw, k2.pose.yaw, u),
                    pitch: angle_lerp(k1.pose.pitch, k2.pose.pitch, u),
                    fov: catmull_rom_f32(p0.fov, k1.pose.fov, k2.pose.fov, p3.fov, u),
                })
            }
        }
    }
}

/// Uniform Catmull-Rom basis on `[0,1]` between `p1` and `p2`.
fn catmull_rom_f32(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (2.0 * p1
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

fn catmull_rom_vec(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    Vec3::new(
        catmull_rom_f32(p0.x, p1.x, p2.x, p3.x, t),
        catmull_rom_f32(p0.y, p1.y, p2.y, p3.y, t),
        catmull_rom_f32(p0.z, p1.z, p2.z, p3.z, t),
    )
}

/// Shortest-arc interpolation between two angles (radians): wrap `b - a` into
/// `(-π, π]` so the camera always takes the short way round. Public so replay
/// playback (`replay_player`) reuses the same arc convention.
pub fn angle_lerp(a: f32, b: f32, t: f32) -> f32 {
    let delta = (b - a + PI).rem_euclid(TAU) - PI;
    a + delta * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(p: Vec3, yaw: f32, pitch: f32) -> CameraPose {
        CameraPose { position: p, yaw, pitch, fov: 70.0 }
    }

    // (1) sampling at a keyframe time passes through that control point.
    #[test]
    fn sample_at_keyframe_passes_through_control_point() {
        let mut path = CameraPath::new();
        path.push_at(0.0, pose(Vec3::ZERO, 0.0, 0.0));
        path.push_at(1.0, pose(Vec3::new(10.0, 0.0, 0.0), 0.0, 0.0));
        path.push_at(2.0, pose(Vec3::new(10.0, 5.0, 0.0), 0.0, 0.0));
        let s = path.sample(1.0).unwrap();
        assert!((s.position - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-4, "{:?}", s.position);
    }

    // (2) midpoint of a straight 2-key path lies on the segment and is finite.
    #[test]
    fn straight_two_key_midpoint_on_segment() {
        let mut path = CameraPath::new();
        path.push_at(0.0, pose(Vec3::ZERO, 0.0, 0.0));
        path.push_at(2.0, pose(Vec3::new(4.0, 0.0, 0.0), 0.0, 0.0));
        let s = path.sample(1.0).unwrap();
        assert!(s.position.is_finite());
        assert!((s.position - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-4, "{:?}", s.position);
    }

    // (3) a single-key path is constant.
    #[test]
    fn single_key_is_constant() {
        let mut path = CameraPath::new();
        path.push_at(0.5, pose(Vec3::new(1.0, 2.0, 3.0), 0.7, -0.2));
        let s = path.sample(99.0).unwrap();
        assert_eq!(s.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(s.yaw, 0.7);
    }

    // (4) empty path samples to None.
    #[test]
    fn empty_is_none() {
        assert!(CameraPath::new().sample(0.0).is_none());
    }

    // (5) yaw 350°→10° crosses +20° (short arc), not −340°.
    #[test]
    fn yaw_takes_short_arc() {
        let mut path = CameraPath::new();
        path.push_at(0.0, pose(Vec3::ZERO, 350f32.to_radians(), 0.0));
        path.push_at(1.0, pose(Vec3::ZERO, 10f32.to_radians(), 0.0));
        let end = path.sample(1.0).unwrap().yaw;
        // 350° + 20° = 370° ≡ 10°.
        assert!((end - 370f32.to_radians()).abs() < 1e-3, "end={}", end.to_degrees());
        let mid = path.sample(0.5).unwrap().yaw; // 350 + 10 = 360 ≡ 0
        assert!((mid - 360f32.to_radians()).abs() < 1e-3, "mid={}", mid.to_degrees());
    }

    // (6) duration == last key's t.
    #[test]
    fn duration_is_last_t() {
        let mut path = CameraPath::new();
        path.push_at(0.0, pose(Vec3::ZERO, 0.0, 0.0));
        path.push_at(3.5, pose(Vec3::ONE, 0.0, 0.0));
        assert_eq!(path.duration(), 3.5);
    }

    // (7) clamp outside range: t<0 → first pose, t>dur → last pose.
    #[test]
    fn clamps_outside_range() {
        let mut path = CameraPath::new();
        path.push_at(0.0, pose(Vec3::ZERO, 0.0, 0.0));
        path.push_at(1.0, pose(Vec3::new(9.0, 0.0, 0.0), 0.0, 0.0));
        assert!((path.sample(-5.0).unwrap().position - Vec3::ZERO).length() < 1e-4);
        assert!((path.sample(5.0).unwrap().position - Vec3::new(9.0, 0.0, 0.0)).length() < 1e-4);
    }

    // (8) dense sweep: no NaN; just-before vs just-after an interior key are close.
    #[test]
    fn dense_sweep_is_smooth_and_finite() {
        let mut path = CameraPath::new();
        path.push_at(0.0, pose(Vec3::ZERO, 0.0, 0.0));
        path.push_at(1.0, pose(Vec3::new(5.0, 2.0, -1.0), 1.0, 0.3));
        path.push_at(2.0, pose(Vec3::new(2.0, 4.0, 3.0), -0.5, -0.4));
        let mut prev: Option<Vec3> = None;
        let mut s = 0.0;
        while s <= 2.0 {
            let p = path.sample(s).unwrap().position;
            assert!(p.is_finite());
            if let Some(pv) = prev {
                assert!((p - pv).length() < 1.0, "jump at t={s}: {pv:?}->{p:?}");
            }
            prev = Some(p);
            s += 0.01;
        }
    }
}

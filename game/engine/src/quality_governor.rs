//! Auto-quality governor (Spec 03 §10.3, as built).
//!
//! A small, pure state machine that watches frame times and, when frames run
//! long, holds render distance, render scale and particles temporarily BELOW
//! the values the player chose. It is an offset, not a setting:
//!
//! * it never raises anything above the player's values;
//! * it is never written to the settings file (a restart starts clean);
//! * changing a preset or a slider resets it ([`Governor::reset`]).
//!
//! ## Steps
//!
//! The offset is a *level*: how many steps of a per-player plan are applied.
//! The plan is built from the player's own values, in this order:
//!
//! 1. render distance, 2 chunks per step, down to `RENDER_DISTANCE_MIN`;
//! 2. render scale, ×0.8 / ×0.64 / ×0.5 of the player's scale, never below
//!    `min(scale, 0.5)`;
//! 3. particles, `Full → Reduced → Off`.
//!
//! A step that would change nothing (already at the floor) is not in the plan.
//!
//! ## Rules
//!
//! The frame time the governor judges is the 90th percentile of the last
//! second of frames, evaluated every [`EVAL_INTERVAL`] seconds.
//!
//! * Step DOWN when that p90 stays above [`DOWN_RATIO`] × the target frame time
//!   for [`DOWN_HOLD`] seconds.
//! * Step UP when it stays below [`UP_RATIO`] × the target for [`UP_HOLD`]
//!   seconds (the gap between the two ratios is the hysteresis band: inside it
//!   nothing happens and both timers restart).
//! * At most one step per [`MIN_STEP_INTERVAL`] second, and after any step the
//!   one-second window must refill before the next judgement.
//! * Anti-flap back-off: a step down within [`BACKOFF_WINDOW`] seconds of a step
//!   up doubles the hold needed for the next step up (capped at [`UP_HOLD_MAX`]),
//!   so a machine that can run level N-1 but not level N settles instead of
//!   bouncing every few seconds.
//! * The first [`WARMUP`] seconds after a reset are ignored (world load hitches).
//!
//! A frame-capped (vsync) machine cannot show frames faster than its refresh
//! interval, so on a 60 Hz display the up rule (below 0.7 × the 60 fps target)
//! can never fire: the offset holds until a slider/preset change or a restart.
//! That is the spec's rule, kept as written.

use crate::graphics_settings::{GraphicsSettings, ParticleLevel, RENDER_DISTANCE_MIN};
use std::collections::VecDeque;

/// Step down when the p90 frame time exceeds this multiple of the target.
pub const DOWN_RATIO: f32 = 1.25;
/// Step up when the p90 frame time is below this multiple of the target.
pub const UP_RATIO: f32 = 0.7;
/// Seconds the "too slow" condition must hold before a step down.
pub const DOWN_HOLD: f32 = 3.0;
/// Seconds the "fast" condition must hold before a step up (first time).
pub const UP_HOLD: f32 = 10.0;
/// Longest step-up hold the back-off can reach.
pub const UP_HOLD_MAX: f32 = 160.0;
/// A step down this soon after a step up counts as a bounce and doubles the hold.
pub const BACKOFF_WINDOW: f32 = 30.0;
/// Minimum seconds between any two steps.
pub const MIN_STEP_INTERVAL: f32 = 1.0;
/// Seconds of frames the percentile is taken over.
pub const WINDOW: f32 = 1.0;
/// Seconds between judgements.
pub const EVAL_INTERVAL: f32 = 0.25;
/// Seconds after a reset during which frames are ignored.
pub const WARMUP: f32 = 5.0;
/// Frame time the governor aims at when the frame limit names none (60 fps).
pub const DEFAULT_TARGET: f32 = 1.0 / 60.0;
/// Most samples kept in the window (bounds the sort at very high frame rates).
const MAX_WINDOW_SAMPLES: usize = 240;
/// Longest plan: 6 distance + 3 scale + 2 particle steps, with room to spare.
const MAX_STEPS: usize = 12;
/// Render-scale multipliers of the player's own scale, in step order.
const SCALE_STEPS: [f32; 3] = [0.8, 0.64, 0.5];
/// The governor never takes the scale below this (or below the player's own
/// value if that is already lower).
const SCALE_FLOOR: f32 = 0.5;
/// Render distance removed by each distance step (Spec 03 §10.3).
const DISTANCE_STEP: i32 = 2;

/// One step of the plan, holding the value it sets.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Distance(i32),
    Scale(f32),
    Particles(ParticleLevel),
}

/// The three dials the governor may hold below the player's values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effective {
    pub render_distance: i32,
    pub render_scale: f32,
    pub particles: ParticleLevel,
}

impl Effective {
    /// The player's own values (level 0).
    pub fn of(user: &GraphicsSettings) -> Self {
        Self {
            render_distance: user.render_distance,
            render_scale: user.render_scale,
            particles: user.particles,
        }
    }
}

fn plan(user: &GraphicsSettings, shrink_distance: bool) -> ([Step; MAX_STEPS], usize) {
    let mut steps = [Step::Distance(0); MAX_STEPS];
    let mut n = 0;
    let mut push = |s: Step, n: &mut usize| {
        if *n < MAX_STEPS {
            steps[*n] = s;
            *n += 1;
        }
    };
    let mut rd = user.render_distance;
    while shrink_distance && rd > RENDER_DISTANCE_MIN {
        rd = (rd - DISTANCE_STEP).max(RENDER_DISTANCE_MIN);
        push(Step::Distance(rd), &mut n);
    }
    let floor = user.render_scale.min(SCALE_FLOOR);
    let mut cur = user.render_scale;
    for m in SCALE_STEPS {
        let next = (user.render_scale * m).max(floor);
        if next < cur - 1e-3 {
            push(Step::Scale(next), &mut n);
            cur = next;
        }
    }
    match user.particles {
        ParticleLevel::Full => {
            push(Step::Particles(ParticleLevel::Reduced), &mut n);
            push(Step::Particles(ParticleLevel::Off), &mut n);
        }
        ParticleLevel::Reduced => push(Step::Particles(ParticleLevel::Off), &mut n),
        ParticleLevel::Off => {}
    }
    (steps, n)
}

/// Pure: the deepest level the player's own values allow. `shrink_distance` is
/// false where lowering the streamed distance would strand data (a joiner holds
/// only what its server pushed, and never reloads a column it let go): the plan
/// then starts at render scale.
pub fn max_level(user: &GraphicsSettings, shrink_distance: bool) -> usize {
    plan(user, shrink_distance).1
}

/// Pure: the dials at `level` (clamped to the plan), derived from the player's
/// values. Level 0 is exactly the player's values, and no level ever exceeds
/// them.
pub fn effective(user: &GraphicsSettings, level: usize, shrink_distance: bool) -> Effective {
    let (steps, n) = plan(user, shrink_distance);
    let mut e = Effective::of(user);
    for s in &steps[..level.min(n)] {
        match *s {
            Step::Distance(d) => e.render_distance = d,
            Step::Scale(v) => e.render_scale = v,
            Step::Particles(p) => e.particles = p,
        }
    }
    e
}

/// Pure: the frame time (seconds) the governor aims at for a frame limit.
pub fn target_frame_time(limit: crate::graphics_settings::FrameLimit) -> f32 {
    match limit.software_cap_interval() {
        Some(i) => i.as_secs_f32(),
        None => DEFAULT_TARGET,
    }
}

/// A change the caller must apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Down,
    Up,
}

/// The player's values the offset was built from. A change to any of them is
/// the player taking control back ([`Governor::rebase`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Basis {
    render_distance: i32,
    render_scale: f32,
    particles: ParticleLevel,
    frame_limit: crate::graphics_settings::FrameLimit,
    pixel_budget: Option<u32>,
    auto_quality: bool,
}

impl Basis {
    fn of(u: &GraphicsSettings) -> Self {
        Self {
            render_distance: u.render_distance,
            render_scale: u.render_scale,
            particles: u.particles,
            frame_limit: u.frame_limit,
            pixel_budget: u.pixel_budget,
            auto_quality: u.auto_quality,
        }
    }
}

/// The state machine. Feed it every in-world frame with [`Governor::update`].
#[derive(Clone, Debug)]
pub struct Governor {
    level: usize,
    window: VecDeque<f32>,
    window_sum: f32,
    since_eval: f32,
    slow_for: f32,
    fast_for: f32,
    age: f32,
    since_step: f32,
    since_up: f32,
    up_hold: f32,
    basis: Option<Basis>,
}

impl Default for Governor {
    fn default() -> Self {
        Self::new()
    }
}

impl Governor {
    pub fn new() -> Self {
        Self {
            level: 0,
            window: VecDeque::with_capacity(MAX_WINDOW_SAMPLES),
            window_sum: 0.0,
            since_eval: 0.0,
            slow_for: 0.0,
            fast_for: 0.0,
            age: 0.0,
            since_step: f32::INFINITY,
            since_up: f32::INFINITY,
            up_hold: UP_HOLD,
            basis: None,
        }
    }

    /// Current level (0 = the player's own values).
    pub fn level(&self) -> usize {
        self.level
    }

    /// Back to the player's values with a clean history. Called when the player
    /// moves a slider or picks a preset, and on entering a world.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Call whenever the player's settings may have changed. If any dial the
    /// offset is built on differs from last time, the offset is dropped (a
    /// preset or slider change hands control back to the player) and `true` is
    /// returned; an unrelated change (volume, FOV...) leaves it alone.
    pub fn rebase(&mut self, user: &GraphicsSettings) -> bool {
        let now = Basis::of(user);
        match self.basis {
            Some(b) if b == now => false,
            Some(_) => {
                self.reset();
                self.basis = Some(now);
                true
            }
            None => {
                self.basis = Some(now);
                false
            }
        }
    }

    fn clear_window(&mut self) {
        self.window.clear();
        self.window_sum = 0.0;
        self.since_eval = 0.0;
        self.slow_for = 0.0;
        self.fast_for = 0.0;
    }

    /// 90th percentile of the window (seconds), or `None` until it holds most
    /// of a full window.
    fn p90(&self) -> Option<f32> {
        if self.window_sum < WINDOW * 0.75 || self.window.is_empty() {
            return None;
        }
        let mut v: Vec<f32> = self.window.iter().copied().collect();
        v.sort_by(|a, b| a.total_cmp(b));
        let idx = ((v.len() as f32 * 0.9).ceil() as usize).clamp(1, v.len()) - 1;
        Some(v[idx])
    }

    /// Feed one frame (`dt` seconds since the previous frame). `target` is the
    /// frame time aimed at ([`target_frame_time`]); `max_level` the deepest
    /// level the player's values allow ([`max_level`]). Returns the step to
    /// apply, if any.
    pub fn update(&mut self, dt: f32, target: f32, max_level: usize) -> Option<Change> {
        if !dt.is_finite() || dt <= 0.0 || !target.is_finite() || target <= 0.0 {
            return None;
        }
        self.age += dt;
        self.since_step += dt;
        self.since_up += dt;
        if self.age < WARMUP {
            return None;
        }
        self.window.push_back(dt);
        self.window_sum += dt;
        while self.window_sum - self.window.front().copied().unwrap_or(0.0) >= WINDOW
            || self.window.len() > MAX_WINDOW_SAMPLES
        {
            let Some(old) = self.window.pop_front() else { break };
            self.window_sum -= old;
        }
        self.since_eval += dt;
        if self.since_eval < EVAL_INTERVAL {
            return None;
        }
        let elapsed = self.since_eval;
        self.since_eval = 0.0;
        let p90 = self.p90()?;

        if p90 > DOWN_RATIO * target {
            self.slow_for += elapsed;
            self.fast_for = 0.0;
        } else if p90 < UP_RATIO * target {
            self.fast_for += elapsed;
            self.slow_for = 0.0;
        } else {
            self.slow_for = 0.0;
            self.fast_for = 0.0;
        }
        if self.since_step < MIN_STEP_INTERVAL {
            return None;
        }
        if self.slow_for >= DOWN_HOLD && self.level < max_level {
            if self.since_up < BACKOFF_WINDOW {
                self.up_hold = (self.up_hold * 2.0).min(UP_HOLD_MAX);
            }
            self.level += 1;
            self.after_step();
            return Some(Change::Down);
        }
        if self.fast_for >= self.up_hold && self.level > 0 {
            self.level -= 1;
            self.since_up = 0.0;
            self.after_step();
            return Some(Change::Up);
        }
        None
    }

    fn after_step(&mut self) {
        self.since_step = 0.0;
        self.clear_window();
    }

    /// One line for the settings panel when the governor is holding below the
    /// player's values, else `None`.
    pub fn note(&self, user: &GraphicsSettings, shrink_distance: bool) -> Option<String> {
        if self.level == 0 {
            return None;
        }
        let e = effective(user, self.level, shrink_distance);
        let mut parts = Vec::new();
        if e.render_distance != user.render_distance {
            parts.push(format!(
                "render distance {} (set {})",
                e.render_distance, user.render_distance
            ));
        }
        if (e.render_scale - user.render_scale).abs() > 1e-3 {
            parts.push(format!(
                "render scale {:.2} (set {:.2})",
                e.render_scale, user.render_scale
            ));
        }
        if e.particles != user.particles {
            parts.push("fewer particles".to_string());
        }
        if parts.is_empty() {
            return None;
        }
        Some(format!("Auto quality is holding {}.", parts.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics_settings::FrameLimit;

    const T60: f32 = 1.0 / 60.0;

    fn user() -> GraphicsSettings {
        GraphicsSettings::default() // rd 10, scale 1.0, particles Full
    }

    /// Run `secs` seconds of frames; `frame(level, t)` gives the frame time.
    /// Returns (time, change) for every step.
    fn run(
        g: &mut Governor,
        u: &GraphicsSettings,
        t0: f32,
        secs: f32,
        mut frame: impl FnMut(usize, f32) -> f32,
    ) -> (f32, Vec<(f32, Change)>) {
        let mut t = t0;
        let mut out = Vec::new();
        while t < t0 + secs {
            let dt = frame(g.level(), t);
            t += dt;
            if let Some(c) = g.update(dt, T60, max_level(u, true)) {
                out.push((t, c));
            }
        }
        (t, out)
    }

    #[test]
    fn plan_is_distance_then_scale_then_particles_and_never_raises() {
        let u = user();
        assert_eq!(max_level(&u, true), 3 + 3 + 2);
        let mut prev = effective(&u, 0, true);
        assert_eq!(prev, Effective::of(&u));
        let kinds: Vec<_> = (1..=max_level(&u, true))
            .map(|l| {
                let e = effective(&u, l, true);
                assert!(e.render_distance <= prev.render_distance);
                assert!(e.render_scale <= prev.render_scale);
                assert!(e.render_distance <= u.render_distance);
                assert!(e.render_scale <= u.render_scale + 1e-6);
                let k = if e.render_distance != prev.render_distance {
                    0
                } else if (e.render_scale - prev.render_scale).abs() > 1e-6 {
                    1
                } else {
                    assert_ne!(e.particles, prev.particles);
                    2
                };
                prev = e;
                k
            })
            .collect();
        assert!(kinds.windows(2).all(|w| w[0] <= w[1]), "{kinds:?}");
        let last = effective(&u, 99, true);
        assert_eq!(last.render_distance, RENDER_DISTANCE_MIN);
        assert_eq!(last.render_scale, 0.5);
        assert_eq!(last.particles, ParticleLevel::Off);
    }

    #[test]
    fn plan_respects_floors_already_reached() {
        let mut u = user();
        u.render_distance = RENDER_DISTANCE_MIN;
        u.render_scale = 0.4;
        u.particles = ParticleLevel::Off;
        assert_eq!(max_level(&u, true), 0);
        assert_eq!(effective(&u, 5, true), Effective::of(&u));
        u.render_scale = 0.75;
        u.particles = ParticleLevel::Reduced;
        // 0.75: ×0.8 = 0.6, then the floor 0.5 → 2 steps; plus 1 particle step.
        assert_eq!(max_level(&u, true), 3);
        assert!((effective(&u, 1, true).render_scale - 0.6).abs() < 1e-6);
        assert!((effective(&u, 2, true).render_scale - 0.5).abs() < 1e-6);
        assert_eq!(effective(&u, 3, true).particles, ParticleLevel::Off);
        // An odd distance steps by two and lands on the minimum.
        u.render_distance = 5;
        assert_eq!(effective(&u, 1, true).render_distance, 4);
    }

    #[test]
    fn a_joiner_plan_never_shrinks_the_streamed_distance() {
        let u = user();
        assert_eq!(max_level(&u, false), 3 + 2);
        for l in 0..=max_level(&u, false) {
            assert_eq!(effective(&u, l, false).render_distance, u.render_distance);
        }
        assert!(effective(&u, 1, false).render_scale < u.render_scale);
    }

    #[test]
    fn target_comes_from_the_frame_limit() {
        assert!((target_frame_time(FrameLimit::Cap(30)) - 1.0 / 30.0).abs() < 1e-6);
        assert!((target_frame_time(FrameLimit::Cap(60)) - T60).abs() < 1e-6);
        assert!((target_frame_time(FrameLimit::VSync) - T60).abs() < 1e-6);
        assert!((target_frame_time(FrameLimit::Uncapped) - T60).abs() < 1e-6);
    }

    #[test]
    fn steady_slow_steps_down_at_a_bounded_rate_then_holds() {
        let u = user();
        let mut g = Governor::new();
        let (_, steps) = run(&mut g, &u, 0.0, 120.0, |_, _| 0.050); // 20 fps
        assert!(steps.iter().all(|(_, c)| *c == Change::Down));
        assert_eq!(steps.len(), max_level(&u, true), "reaches the floor and stops");
        assert_eq!(g.level(), max_level(&u, true));
        // Nothing before the warm-up, and never faster than the hold allows.
        assert!(steps[0].0 >= WARMUP + DOWN_HOLD - 0.3, "{:?}", steps[0]);
        for w in steps.windows(2) {
            let gap = w[1].0 - w[0].0;
            assert!(gap >= MIN_STEP_INTERVAL, "gap {gap}");
            assert!(gap >= DOWN_HOLD - 0.3, "gap {gap}");
        }
    }

    #[test]
    fn steady_fast_recovers_to_zero_and_never_goes_below() {
        let u = user();
        let mut g = Governor::new();
        let (t, down) = run(&mut g, &u, 0.0, 20.0, |_, _| 0.050);
        assert!(!down.is_empty());
        let start = g.level();
        assert!(start >= 2);
        let (_, ups) = run(&mut g, &u, t, 200.0, |_, _| 0.006); // 166 fps
        assert!(ups.iter().all(|(_, c)| *c == Change::Up));
        assert_eq!(ups.len(), start);
        assert_eq!(g.level(), 0);
        for w in ups.windows(2) {
            assert!(w[1].0 - w[0].0 >= UP_HOLD - 0.3, "up gap {}", w[1].0 - w[0].0);
        }
        // Still fast at level 0: nothing happens.
        let (_, more) = run(&mut g, &u, t + 200.0, 60.0, |_, _| 0.006);
        assert!(more.is_empty());
    }

    #[test]
    fn frames_inside_the_hysteresis_band_change_nothing() {
        let u = user();
        for ft in [T60 * 0.75, T60, T60 * 1.2] {
            let mut g = Governor::new();
            let (_, steps) = run(&mut g, &u, 0.0, 120.0, |_, _| ft);
            assert!(steps.is_empty(), "ft {ft}: {steps:?}");
        }
        // Inside the band after a drop: it holds, neither up nor down.
        let mut g = Governor::new();
        let (t, _) = run(&mut g, &u, 0.0, 10.0, |_, _| 0.050);
        let held = g.level();
        let (_, steps) = run(&mut g, &u, t, 120.0, |_, _| T60 * 0.9);
        assert!(steps.is_empty());
        assert_eq!(g.level(), held);
    }

    #[test]
    fn a_single_hitch_in_a_smooth_stream_is_ignored() {
        let u = user();
        let mut g = Governor::new();
        let mut hitched = false;
        let (_, steps) = run(&mut g, &u, 0.0, 120.0, |_, t| {
            if !hitched && t > 30.0 {
                hitched = true;
                0.4
            } else {
                T60
            }
        });
        assert!(steps.is_empty(), "{steps:?}");
    }

    #[test]
    fn a_noisy_stream_judged_by_p90_not_the_mean() {
        // 80% smooth 60 fps, 20% 60 ms stutters: mean ≈ 25 ms (1.5x) but the
        // p90 is the stutter, so this machine IS slow and must step down.
        let u = user();
        let mut g = Governor::new();
        let mut n = 0u32;
        let (_, steps) = run(&mut g, &u, 0.0, 60.0, |_, _| {
            n += 1;
            if n % 5 == 0 { 0.060 } else { T60 }
        });
        assert!(!steps.is_empty());
        // 5% stutters leave the p90 smooth: no step.
        let mut g = Governor::new();
        let mut n = 0u32;
        let (_, steps) = run(&mut g, &u, 0.0, 60.0, |_, _| {
            n += 1;
            if n % 20 == 0 { 0.060 } else { T60 }
        });
        assert!(steps.is_empty(), "{steps:?}");
    }

    #[test]
    fn a_machine_that_copes_one_level_down_does_not_flap() {
        // 40 ms frames at level 0, 10 ms at any lower quality: the fast result
        // after each step down invites a step up that fails again.
        let u = user();
        let mut g = Governor::new();
        let (_, steps) = run(&mut g, &u, 0.0, 900.0, |lvl, _| if lvl == 0 { 0.040 } else { 0.010 });
        assert!(steps.len() <= 20, "flapped {} times", steps.len());
        // Each bounce takes longer than the last (the back-off).
        let downs: Vec<f32> = steps
            .iter()
            .filter(|(_, c)| *c == Change::Down)
            .map(|(t, _)| *t)
            .collect();
        let gaps: Vec<f32> = downs.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.windows(2).all(|w| w[1] >= w[0] - 1.0), "{gaps:?}");
        // And the last 300 s see at most a handful of steps.
        let late = steps.iter().filter(|(t, _)| *t > 600.0).count();
        assert!(late <= 5, "{late} late steps");
    }

    #[test]
    fn oscillating_load_never_steps_faster_than_once_per_second() {
        // 10 s slow / 10 s fast, repeating.
        let u = user();
        let mut g = Governor::new();
        let (_, steps) = run(&mut g, &u, 0.0, 600.0, |_, t| {
            if (t / 10.0) as u32 % 2 == 0 { 0.045 } else { 0.008 }
        });
        for w in steps.windows(2) {
            assert!(w[1].0 - w[0].0 >= MIN_STEP_INTERVAL);
        }
        assert!(steps.len() < 40, "{} steps in 10 min", steps.len());
    }

    #[test]
    fn reset_returns_to_the_players_values() {
        let u = user();
        let mut g = Governor::new();
        run(&mut g, &u, 0.0, 30.0, |_, _| 0.050);
        assert!(g.level() > 0);
        assert!(g.note(&u, true).is_some());
        g.reset();
        assert_eq!(g.level(), 0);
        assert_eq!(effective(&u, g.level(), true), Effective::of(&u));
        assert!(g.note(&u, true).is_none());
        // And it has to warm up again.
        let (_, steps) = run(&mut g, &u, 0.0, WARMUP - 0.5, |_, _| 0.050);
        assert!(steps.is_empty());
    }

    #[test]
    fn only_quality_dials_reset_the_offset() {
        let mut u = user();
        let mut g = Governor::new();
        assert!(!g.rebase(&u), "first sight just records");
        run(&mut g, &u, 0.0, 30.0, |_, _| 0.050);
        let held = g.level();
        assert!(held > 0);
        // Volume, FOV, brightness: not quality dials.
        u.master_volume = 0.3;
        u.fov_y = 90.0;
        u.brightness = 1.4;
        assert!(!g.rebase(&u));
        assert_eq!(g.level(), held);
        // A slider the governor works on, a preset, the toggle: all reset.
        u.render_distance = 8;
        assert!(g.rebase(&u));
        assert_eq!(g.level(), 0);
        run(&mut g, &u, 0.0, 30.0, |_, _| 0.050);
        assert!(g.level() > 0);
        u.apply_preset(crate::graphics_settings::GraphicsPreset::Low);
        assert!(g.rebase(&u));
        assert_eq!(g.level(), 0);
    }

    #[test]
    fn note_names_what_it_is_holding() {
        let u = user();
        let mut g = Governor::new();
        run(&mut g, &u, 0.0, 14.0, |_, _| 0.050);
        let n = g.note(&u, true).expect("holding below");
        assert!(n.contains("render distance"), "{n}");
        assert!(n.contains("(set 10)"), "{n}");
    }

    #[test]
    fn bad_input_is_ignored() {
        let mut g = Governor::new();
        assert_eq!(g.update(f32::NAN, T60, 5), None);
        assert_eq!(g.update(-1.0, T60, 5), None);
        assert_eq!(g.update(0.0, T60, 5), None);
        assert_eq!(g.update(0.016, 0.0, 5), None);
    }
}

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
//!    `min(scale, 0.5)`, skipping any step the pixel budget would swallow (when
//!    the budget already holds the scale below the step, the step changes
//!    nothing on screen);
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
//! * Step UP when it stays at or below [`UP_RATIO`] × the target for
//!   [`UP_HOLD`] seconds (the gap between the two ratios is the hysteresis band:
//!   inside it nothing happens and both timers restart). The up ratio is just
//!   above 1.0 so a vsync-pinned display (frame time can never fall below the
//!   refresh interval) still recovers after a stall; the back-off below is
//!   what stops that from oscillating.
//! * At most one step per [`MIN_STEP_INTERVAL`] second, and after any step the
//!   one-second window must refill before the next judgement.
//! * Anti-flap back-off: a step down within [`BACKOFF_WINDOW`] seconds of a step
//!   up doubles the hold needed for the next step up (capped at [`UP_HOLD_MAX`]),
//!   so a machine that can run level N-1 but not level N settles instead of
//!   bouncing every few seconds.
//! * Ineffective-step rule: a step down must improve the p90 by at least
//!   [`INEFFECTIVE_GAIN`] (10%) once the window has refilled. If it does not,
//!   the machine is not limited by what the step lowers (a 30 or 40 Hz
//!   display, a throttled browser, a CPU-bound frame), so the step is reverted
//!   ([`Change::Reverted`]) and further steps down are held off for the
//!   anti-flap back-off period ([`BACKOFF_WINDOW`], doubling per revert up to
//!   [`UP_HOLD_MAX`]). Where the display rate is known (native, from winit) the
//!   target is also at least the display's period ([`target_frame_time_for`]).
//! * The first [`WARMUP`] seconds after a reset are ignored (world load hitches).
//!
//! A frame-capped (vsync) machine cannot show frames faster than its refresh
//! interval; with the up ratio at 1.05 a 60 Hz display pinned at 16.7 ms still
//! steps back up after a stall, and a machine that is genuinely too slow one
//! notch higher bounces at most once per (doubling) hold.

use crate::graphics_settings::{GraphicsSettings, ParticleLevel, RENDER_DISTANCE_MIN};
use std::collections::VecDeque;

/// Step down when the p90 frame time exceeds this multiple of the target.
pub const DOWN_RATIO: f32 = 1.25;
/// Step up when the p90 frame time is at or below this multiple of the target.
pub const UP_RATIO: f32 = 1.05;
/// Seconds the "too slow" condition must hold before a step down.
pub const DOWN_HOLD: f32 = 3.0;
/// Seconds the "fast" condition must hold before a step up (first time).
pub const UP_HOLD: f32 = 10.0;
/// Longest step-up hold the back-off can reach.
pub const UP_HOLD_MAX: f32 = 160.0;
/// A step down this soon after a step up counts as a bounce and doubles the hold.
pub const BACKOFF_WINDOW: f32 = 30.0;
/// A step down must lower the p90 by at least this fraction, or it is reverted.
pub const INEFFECTIVE_GAIN: f32 = 0.10;
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

/// `scale_cap` value for "the pixel budget holds the scale nowhere" (tests).
#[cfg(test)]
pub const NO_SCALE_CAP: f32 = f32::INFINITY;

fn plan(user: &GraphicsSettings, shrink_distance: bool, scale_cap: f32) -> ([Step; MAX_STEPS], usize) {
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
    // What the renderer actually draws at is min(step, cap): a step whose value
    // is no lower than the cap, from a value already at or above it, changes
    // nothing (it would only cost three seconds and mislabel the settings note).
    let shown = |v: f32| v.min(scale_cap);
    for m in SCALE_STEPS {
        let next = (user.render_scale * m).max(floor);
        if next < cur - 1e-3 {
            if shown(next) < shown(cur) - 1e-3 {
                push(Step::Scale(next), &mut n);
            }
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

/// Pure: may the governor shorten the streamed distance? Only a client that
/// streams its own world. ANY joined client keeps scale and particle steps
/// only: it announces the player's render distance in every input, so the
/// server's push radius stays the player's, and a client that unloaded the
/// pushed ring between its lower distance and that radius would be re-sent it
/// (or told to regenerate it) every few seconds, on the machine that is
/// already too slow. (Not just a push-only joiner: a joiner that can generate
/// its own ground is in the same loop.)
pub fn may_shrink_distance(joined: bool) -> bool {
    !joined
}

/// Pure: the deepest level the player's own values allow. `shrink_distance` is
/// false where lowering the streamed distance would strand data (a joined
/// client: [`may_shrink_distance`]): the plan then starts at render scale.
/// `scale_cap` is the largest scale the pixel budget allows on this surface
/// (`f32::INFINITY` for none).
pub fn max_level(user: &GraphicsSettings, shrink_distance: bool, scale_cap: f32) -> usize {
    plan(user, shrink_distance, scale_cap).1
}

/// Pure: the dials at `level` (clamped to the plan), derived from the player's
/// values. Level 0 is exactly the player's values, and no level ever exceeds
/// them.
pub fn effective(user: &GraphicsSettings, level: usize, shrink_distance: bool, scale_cap: f32) -> Effective {
    let (steps, n) = plan(user, shrink_distance, scale_cap);
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

/// Pure: [`target_frame_time`], raised to the display's own period where the
/// platform reports the refresh rate (native: winit's `refresh_rate_millihertz`).
/// A 30 or 40 Hz panel can never show frames faster than that, so judging it
/// against 16.7 ms would send the governor to its floor for nothing. A
/// missing or absurd rate (outside 20..1000 Hz) leaves the target alone.
pub fn target_frame_time_for(
    limit: crate::graphics_settings::FrameLimit,
    display_millihertz: Option<u32>,
) -> f32 {
    let target = target_frame_time(limit);
    match display_millihertz {
        Some(mhz) if (20_000..=1_000_000).contains(&mhz) => target.max(1000.0 / mhz as f32),
        _ => target,
    }
}

/// A change the caller must apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Down,
    Up,
    /// A step down that did not help was undone (the level went back up by
    /// one); further steps down are held off for a while.
    Reverted,
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
    /// The p90 the last step down was taken at, until the window after it has
    /// refilled and the step has been judged.
    probe: Option<f32>,
    /// Seconds left before another step down is allowed (after a revert).
    down_off: f32,
    /// The next revert's hold-off length: [`BACKOFF_WINDOW`], doubling.
    hold_off: f32,
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
            probe: None,
            down_off: 0.0,
            hold_off: BACKOFF_WINDOW,
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
    /// of a full window. A window at its sample cap is full however short it
    /// is in time: above ~320 fps 240 samples cover less than 0.75 s, and
    /// waiting for the time would freeze the governor for good.
    fn p90(&self) -> Option<f32> {
        let full = self.window.len() >= MAX_WINDOW_SAMPLES;
        if (self.window_sum < WINDOW * 0.75 && !full) || self.window.is_empty() {
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
        self.down_off = (self.down_off - dt).max(0.0);
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

        // Judge the last step down once its window has refilled: it must have
        // bought a real gain, or it is undone and the machine left alone.
        // (Not inside the minimum gap between steps: the one-step-per-second
        // rule covers an undo as much as a step.)
        if self.since_step >= MIN_STEP_INTERVAL
            && let Some(before) = self.probe.take()
            && p90 > before * (1.0 - INEFFECTIVE_GAIN)
            && self.level > 0
        {
            self.level -= 1;
            self.down_off = self.hold_off;
            self.hold_off = (self.hold_off * 2.0).min(UP_HOLD_MAX);
            self.after_step();
            return Some(Change::Reverted);
        }

        if p90 > DOWN_RATIO * target {
            self.slow_for += elapsed;
            self.fast_for = 0.0;
        } else if p90 <= UP_RATIO * target {
            self.fast_for += elapsed;
            self.slow_for = 0.0;
        } else {
            self.slow_for = 0.0;
            self.fast_for = 0.0;
        }
        if self.since_step < MIN_STEP_INTERVAL {
            return None;
        }
        if self.slow_for >= DOWN_HOLD && self.level < max_level && self.down_off <= 0.0 {
            if self.since_up < BACKOFF_WINDOW {
                self.up_hold = (self.up_hold * 2.0).min(UP_HOLD_MAX);
            }
            self.level += 1;
            self.probe = Some(p90);
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
    pub fn note(&self, user: &GraphicsSettings, shrink_distance: bool, scale_cap: f32) -> Option<String> {
        if self.level == 0 {
            return None;
        }
        let e = effective(user, self.level, shrink_distance, scale_cap);
        let mut parts = Vec::new();
        if e.render_distance != user.render_distance {
            parts.push(format!(
                "render distance {} (set {})",
                e.render_distance, user.render_distance
            ));
        }
        // The scale actually drawn: the pixel budget may already hold it lower
        // than the step ("render scale 0.80 (set 1.00)" while it renders at 0.50).
        let (shown, was) = (e.render_scale.min(scale_cap), user.render_scale.min(scale_cap));
        if (shown - was).abs() > 1e-3 {
            parts.push(format!("render scale {shown:.2} (was {was:.2})"));
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

    /// A genuinely GPU-bound machine: 200 ms at the player's values, each step
    /// down buys 20%, so every step clears the ineffective-step test. (A flat
    /// "slow whatever the level" series is the NOT-GPU-bound case, below.)
    fn gpu_bound(level: usize) -> f32 {
        0.200 * 0.8f32.powi(level as i32)
    }

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
            if let Some(c) = g.update(dt, T60, max_level(u, true, NO_SCALE_CAP)) {
                out.push((t, c));
            }
        }
        (t, out)
    }

    #[test]
    fn plan_is_distance_then_scale_then_particles_and_never_raises() {
        let u = user();
        assert_eq!(max_level(&u, true, NO_SCALE_CAP), 3 + 3 + 2);
        let mut prev = effective(&u, 0, true, NO_SCALE_CAP);
        assert_eq!(prev, Effective::of(&u));
        let kinds: Vec<_> = (1..=max_level(&u, true, NO_SCALE_CAP))
            .map(|l| {
                let e = effective(&u, l, true, NO_SCALE_CAP);
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
        let last = effective(&u, 99, true, NO_SCALE_CAP);
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
        assert_eq!(max_level(&u, true, NO_SCALE_CAP), 0);
        assert_eq!(effective(&u, 5, true, NO_SCALE_CAP), Effective::of(&u));
        u.render_scale = 0.75;
        u.particles = ParticleLevel::Reduced;
        // 0.75: ×0.8 = 0.6, then the floor 0.5 → 2 steps; plus 1 particle step.
        assert_eq!(max_level(&u, true, NO_SCALE_CAP), 3);
        assert!((effective(&u, 1, true, NO_SCALE_CAP).render_scale - 0.6).abs() < 1e-6);
        assert!((effective(&u, 2, true, NO_SCALE_CAP).render_scale - 0.5).abs() < 1e-6);
        assert_eq!(effective(&u, 3, true, NO_SCALE_CAP).particles, ParticleLevel::Off);
        // An odd distance steps by two and lands on the minimum.
        u.render_distance = 5;
        assert_eq!(effective(&u, 1, true, NO_SCALE_CAP).render_distance, 4);
    }

    #[test]
    fn a_joiner_plan_never_shrinks_the_streamed_distance() {
        let u = user();
        assert_eq!(max_level(&u, false, NO_SCALE_CAP), 3 + 2);
        for l in 0..=max_level(&u, false, NO_SCALE_CAP) {
            assert_eq!(effective(&u, l, false, NO_SCALE_CAP).render_distance, u.render_distance);
        }
        assert!(effective(&u, 1, false, NO_SCALE_CAP).render_scale < u.render_scale);
    }

    #[test]
    fn only_a_client_streaming_its_own_world_may_shrink_the_distance() {
        assert!(may_shrink_distance(false));
        assert!(!may_shrink_distance(true), "a joined client keeps the player's distance");
        // The plan a joined client gets keeps the distance at every level,
        // for every distance the player can set, however far the governor goes.
        for rd in crate::graphics_settings::RENDER_DISTANCE_MIN..=crate::graphics_settings::RENDER_DISTANCE_MAX {
            let mut u = user();
            u.render_distance = rd;
            let shrink = may_shrink_distance(true);
            for l in 0..=max_level(&u, shrink, NO_SCALE_CAP) + 2 {
                assert_eq!(effective(&u, l, shrink, NO_SCALE_CAP).render_distance, rd, "rd {rd} level {l}");
            }
        }
        // And driven by a machine too slow to ever recover, a joined client
        // still steps down (scale, particles) but its distance never moves.
        let u = user();
        let shrink = may_shrink_distance(true);
        let mut g = Governor::new();
        let mut t = 0.0;
        while t < 120.0 {
            let dt = gpu_bound(g.level());
            g.update(dt, T60, max_level(&u, shrink, NO_SCALE_CAP));
            t += dt;
            assert_eq!(effective(&u, g.level(), shrink, NO_SCALE_CAP).render_distance, u.render_distance);
        }
        assert_eq!(g.level(), max_level(&u, shrink, NO_SCALE_CAP));
        assert!(effective(&u, g.level(), shrink, NO_SCALE_CAP).render_scale < u.render_scale);
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
        let (_, steps) = run(&mut g, &u, 0.0, 120.0, |l, _| gpu_bound(l)); // 20 fps
        assert!(steps.iter().all(|(_, c)| *c == Change::Down));
        assert_eq!(steps.len(), max_level(&u, true, NO_SCALE_CAP), "reaches the floor and stops");
        assert_eq!(g.level(), max_level(&u, true, NO_SCALE_CAP));
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
        let (t, down) = run(&mut g, &u, 0.0, 20.0, |l, _| gpu_bound(l));
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
        // At level 0 there is nothing to step up to, so fast and in-band both hold.
        for ft in [T60 * 0.75, T60, T60 * 1.2] {
            let mut g = Governor::new();
            let (_, steps) = run(&mut g, &u, 0.0, 120.0, |_, _| ft);
            assert!(steps.is_empty(), "ft {ft}: {steps:?}");
        }
        // Inside the band after a drop: it holds, neither up nor down.
        let mut g = Governor::new();
        let (t, _) = run(&mut g, &u, 0.0, 10.0, |l, _| gpu_bound(l));
        let held = g.level();
        let (_, steps) = run(&mut g, &u, t, 120.0, |_, _| T60 * 1.15);
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

    /// Frame times from the series, driven for `secs`; returns every change with
    /// its time and the deepest level reached.
    fn drive(secs: f32, u: &GraphicsSettings, target: f32, mut frame: impl FnMut(usize) -> f32) -> (Vec<(f32, Change)>, usize) {
        let mut g = Governor::new();
        let (mut t, mut deepest, mut out) = (0.0f32, 0usize, Vec::new());
        while t < secs {
            let dt = frame(g.level());
            t += dt;
            if let Some(c) = g.update(dt, target, max_level(u, true, NO_SCALE_CAP)) {
                out.push((t, c));
            }
            deepest = deepest.max(g.level());
        }
        (out, deepest)
    }

    #[test]
    fn a_display_pinned_series_never_walks_to_the_floor() {
        // A 30 Hz and a 40 Hz panel (and a throttled 24 fps browser): the frame
        // time is the display's whatever the quality, so a step down buys
        // nothing. It is tried, undone, and not tried again for a while.
        let u = user();
        for hz in [30.0f32, 40.0, 24.0] {
            let (changes, deepest) = drive(1200.0, &u, T60, |_| 1.0 / hz);
            assert!(deepest <= 1, "{hz} Hz walked to level {deepest}");
            let downs = changes.iter().filter(|(_, c)| *c == Change::Down).count();
            let reverts = changes.iter().filter(|(_, c)| *c == Change::Reverted).count();
            assert!(downs >= 1, "{hz} Hz: it should have tried a step");
            assert_eq!(downs, reverts, "{hz} Hz: every step tried is undone");
            // Back-off: the tries get further apart (30, 60, 120, then 160 s).
            assert!(downs <= 12, "{hz} Hz: {downs} tries in 20 min");
            let tries: Vec<f32> = changes.iter().filter(|(_, c)| *c == Change::Down).map(|(t, _)| *t).collect();
            let gaps: Vec<f32> = tries.windows(2).map(|w| w[1] - w[0]).collect();
            assert!(gaps.windows(2).all(|w| w[1] >= w[0] - 1.0), "{hz} Hz: {gaps:?}");
            assert!(gaps.first().copied().unwrap_or(BACKOFF_WINDOW) >= BACKOFF_WINDOW - 0.5, "{hz} Hz: {gaps:?}");
        }
    }

    #[test]
    fn a_genuinely_gpu_bound_series_still_steps_down() {
        let u = user();
        // 200 ms at the player's values, 20% faster per step: every step counts.
        let (changes, deepest) = drive(120.0, &u, T60, gpu_bound);
        assert_eq!(deepest, max_level(&u, true, NO_SCALE_CAP), "reaches the floor");
        assert!(changes.iter().all(|(_, c)| *c == Change::Down), "{changes:?}");
        // A step that buys 11% is kept; one that buys 9% is not.
        let (changes, deepest) = drive(120.0, &u, T60, |l| 0.200 * 0.89f32.powi(l as i32));
        assert_eq!(deepest, max_level(&u, true, NO_SCALE_CAP));
        assert!(changes.iter().all(|(_, c)| *c == Change::Down));
        let (changes, deepest) = drive(120.0, &u, T60, |l| 0.200 * 0.91f32.powi(l as i32));
        assert_eq!(deepest, 1, "9% is not enough");
        assert!(changes.iter().any(|(_, c)| *c == Change::Reverted));
    }

    #[test]
    fn a_reverted_step_holds_off_the_next_one() {
        let u = user();
        let (changes, _) = drive(400.0, &u, T60, |_| 1.0 / 30.0);
        let mut last_revert = None;
        for (t, c) in &changes {
            match c {
                Change::Reverted => last_revert = Some(*t),
                Change::Down => {
                    if let Some(r) = last_revert {
                        assert!(*t - r >= BACKOFF_WINDOW - 0.5, "stepped down {}s after a revert", t - r);
                    }
                }
                Change::Up => {}
            }
        }
        assert!(last_revert.is_some());
    }

    #[test]
    fn the_target_is_at_least_the_displays_period() {
        use crate::graphics_settings::FrameLimit;
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        // A 30 Hz and a 40 Hz panel raise a 60 fps target.
        assert!(near(target_frame_time_for(FrameLimit::Cap(60), Some(30_000)), 1.0 / 30.0));
        assert!(near(target_frame_time_for(FrameLimit::VSync, Some(40_000)), 1.0 / 40.0));
        // A 60 or 144 Hz panel leaves it (a vsync 144 Hz machine is still judged at 60).
        assert!(near(target_frame_time_for(FrameLimit::VSync, Some(59_940)), 1.0 / 59.94));
        assert!(near(target_frame_time_for(FrameLimit::VSync, Some(144_000)), T60));
        // A slower cap wins over a faster panel.
        assert!(near(target_frame_time_for(FrameLimit::Cap(30), Some(144_000)), 1.0 / 30.0));
        // Unknown or absurd rates change nothing.
        for mhz in [None, Some(0), Some(1), Some(5_000_000)] {
            assert!(near(target_frame_time_for(FrameLimit::VSync, mhz), T60), "{mhz:?}");
        }
        // And a governor given that target leaves a 30 Hz-pinned panel alone.
        let u = user();
        let target = target_frame_time_for(FrameLimit::VSync, Some(30_000));
        let (changes, deepest) = drive(300.0, &u, target, |_| 1.0 / 30.0);
        assert!(changes.is_empty() && deepest == 0, "{changes:?}");
    }

    #[test]
    fn scale_steps_the_pixel_budget_would_swallow_are_skipped() {
        let mut u = user();
        u.particles = ParticleLevel::Off;
        u.render_distance = RENDER_DISTANCE_MIN;
        // Scale 1.0: steps 0.8, 0.64, 0.5. With nothing capping it: three.
        assert_eq!(max_level(&u, true, NO_SCALE_CAP), 3);
        // A budget holding the scale at 0.5 swallows all three.
        assert_eq!(max_level(&u, true, 0.5), 0);
        assert_eq!(effective(&u, 3, true, 0.5), Effective::of(&u));
        // At 0.7: the 0.8 step changes nothing (still 0.7); 0.64 and 0.5 do.
        assert_eq!(max_level(&u, true, 0.7), 2);
        assert!((effective(&u, 1, true, 0.7).render_scale - 0.64).abs() < 1e-6);
        assert!((effective(&u, 2, true, 0.7).render_scale - 0.5).abs() < 1e-6);
        // The note shows what is drawn: 0.64 against 0.70, never "set 1.00".
        let mut g = Governor::new();
        run_capped(&mut g, &u, 0.7);
        assert_eq!(g.level(), 2);
        let n = g.note(&u, true, 0.7).expect("holding below");
        assert!(n.contains("render scale 0.50 (was 0.70)"), "{n}");
        assert!(!n.contains("1.00"), "{n}");
    }

    fn run_capped(g: &mut Governor, u: &GraphicsSettings, cap: f32) {
        let mut t = 0.0;
        while t < 60.0 {
            let dt = gpu_bound(g.level());
            t += dt;
            g.update(dt, T60, max_level(u, true, cap));
        }
    }

    #[test]
    fn a_very_high_frame_rate_still_steps_up_after_a_step_down() {
        // 400 fps: 240 samples cover 0.6 s, under the 0.75 s the window used
        // to need, so the governor froze wherever it was.
        let u = user();
        let mut g = Governor::new();
        let (t, down) = run(&mut g, &u, 0.0, 20.0, |l, _| gpu_bound(l));
        assert!(!down.is_empty());
        let start = g.level();
        assert!(start >= 2);
        let (t, ups) = run(&mut g, &u, t, 300.0, |_, _| 1.0 / 400.0);
        assert!(ups.iter().all(|(_, c)| *c == Change::Up), "{ups:?}");
        assert_eq!(ups.len(), start, "steps all the way back up");
        assert_eq!(g.level(), 0);
        // And it can still step down at that rate: 400 fps is judged, not skipped.
        let (_, down) = run(&mut g, &u, t, 60.0, |l, _| gpu_bound(l));
        assert!(!down.is_empty());
        // A uniformly slow 1200 fps-class series (the cap fills in 0.2 s) judges too.
        let mut g = Governor::new();
        let (_, steps) = run(&mut g, &u, 0.0, 60.0, |_, _| 1.0 / 1200.0);
        assert!(steps.is_empty(), "fast stays put at level 0");
    }

    #[test]
    fn a_vsync_pinned_display_recovers_after_a_stall() {
        // 60 Hz vsync: the frame time can never beat 16.7 ms. After a stall the
        // offset must still come back down to 0, one notch per UP_HOLD.
        let u = user();
        let mut g = Governor::new();
        let (t, down) = run(&mut g, &u, 0.0, 20.0, |l, _| gpu_bound(l));
        assert!(!down.is_empty());
        let start = g.level();
        assert!(start >= 2);
        let (_, ups) = run(&mut g, &u, t, 300.0, |_, _| T60);
        assert!(ups.iter().all(|(_, c)| *c == Change::Up), "{ups:?}");
        assert_eq!(ups.len(), start);
        assert_eq!(g.level(), 0);
        for w in ups.windows(2) {
            assert!(w[1].0 - w[0].0 >= UP_HOLD - 0.3, "up gap {}", w[1].0 - w[0].0);
        }
        // 1.04x of the target still counts as fast; 1.10x is inside the band.
        let mut g = Governor::new();
        let (t, _) = run(&mut g, &u, 0.0, 20.0, |l, _| gpu_bound(l));
        let held = g.level();
        let (_, ups) = run(&mut g, &u, t, 120.0, |_, _| T60 * 1.10);
        assert!(ups.is_empty());
        assert_eq!(g.level(), held);
    }

    #[test]
    fn a_machine_too_slow_at_the_higher_notch_settles_on_a_vsync_display() {
        // Level 0 runs at 25 fps, every lower quality holds the vsync-pinned
        // 16.7 ms: each step up is tried and fails again. The back-off must
        // stretch the gaps (10, 20, 40, 80, then 160 s) so it settles.
        let u = user();
        let mut g = Governor::new();
        let (_, steps) = run(&mut g, &u, 0.0, 1200.0, |lvl, _| if lvl == 0 { 0.040 } else { T60 });
        let ups: Vec<f32> = steps
            .iter()
            .filter(|(_, c)| *c == Change::Up)
            .map(|(t, _)| *t)
            .collect();
        // 10, 20, 40, 80, then 160 s apart: 10 attempts; without the back-off it
        // would be ~90 (one every ~13 s).
        assert!(ups.len() <= 11, "{} step-up attempts in 20 min", ups.len());
        let gaps: Vec<f32> = ups.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.windows(2).all(|w| w[1] >= w[0] - 1.0), "{gaps:?}");
        assert!(
            gaps.last().copied().unwrap_or(UP_HOLD_MAX) >= 100.0,
            "the hold should have grown: {gaps:?}"
        );
        // Settled below level 0 at the end.
        assert!(g.level() <= 1);
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
        run(&mut g, &u, 0.0, 30.0, |l, _| gpu_bound(l));
        assert!(g.level() > 0);
        assert!(g.note(&u, true, NO_SCALE_CAP).is_some());
        g.reset();
        assert_eq!(g.level(), 0);
        assert_eq!(effective(&u, g.level(), true, NO_SCALE_CAP), Effective::of(&u));
        assert!(g.note(&u, true, NO_SCALE_CAP).is_none());
        // And it has to warm up again.
        let (_, steps) = run(&mut g, &u, 0.0, WARMUP - 0.5, |l, _| gpu_bound(l));
        assert!(steps.is_empty());
    }

    #[test]
    fn only_quality_dials_reset_the_offset() {
        let mut u = user();
        let mut g = Governor::new();
        assert!(!g.rebase(&u), "first sight just records");
        run(&mut g, &u, 0.0, 30.0, |l, _| gpu_bound(l));
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
        run(&mut g, &u, 0.0, 30.0, |l, _| gpu_bound(l));
        assert!(g.level() > 0);
        u.apply_preset(crate::graphics_settings::GraphicsPreset::Low);
        assert!(g.rebase(&u));
        assert_eq!(g.level(), 0);
    }

    #[test]
    fn note_names_what_it_is_holding() {
        let u = user();
        let mut g = Governor::new();
        run(&mut g, &u, 0.0, 14.0, |l, _| gpu_bound(l));
        let n = g.note(&u, true, NO_SCALE_CAP).expect("holding below");
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

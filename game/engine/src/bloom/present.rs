//! Bloom, made visible and audible (spec 2026-11-08 §2). Presentation only.
//!
//! - **Colour.** [`fill_uniform`] packs the field's 16 x 16 level grid into the
//!   camera uniform; `shader.wgsl` grades every lit surface by it. [`grade`],
//!   [`sample_level`] and [`sway_amplitude`] are the CPU twins of the shader
//!   functions of the same names, kept here so the curves are tested.
//! - **Sound.** [`BloomPresent`] schedules birdsong by day and insects by night
//!   for each local player, from the local level.
//! - **Life.** [`pollen_per_sec`] and [`firefly_per_sec`] give the spawn rates;
//!   `GameState::bloom_ambient_life` spawns the motes round each local player.
//!
//! Nothing here writes to the world, the wire or a save.

use glam::Vec3;

use super::field::{BloomField, GRID_SIDE};
use crate::camera::CameraUniform;
use crate::GameState;

/// Cells in the grid.
pub const CELLS: usize = GRID_SIDE * GRID_SIDE;
/// Blocks per grid cell (one region).
pub const CELL_SIZE: f32 = 16.0;
/// The level that grades nothing (and what an unknown place reads).
pub const NEUTRAL: f32 = 0.6;

// ── Colour ───────────────────────────────────────────────────────────

/// Pack the 256 levels four to a vec4, in order (cell `k` is lane `k % 4` of
/// vec4 `k / 4`), the layout `bloom_grid` has in the uniform.
pub fn pack_grid(grid: &[f32; CELLS]) -> [[f32; 4]; CELLS / 4] {
    let mut out = [[0.0; 4]; CELLS / 4];
    for (k, level) in grid.iter().enumerate() {
        out[k / 4][k % 4] = *level;
    }
    out
}

/// Fill a player's camera uniform from the field. The grid is centred on the
/// camera (`camera_pos` x and z), so the shader's camera-cell fallback is the
/// cell the viewer stands in. `clock` is the shader clock in seconds.
pub fn fill_uniform(u: &mut CameraUniform, bloom: &BloomField, clock: f32) {
    let (grid, ox, oz) = bloom.grid_around(u.camera_pos[0], u.camera_pos[2]);
    u.bloom_origin = [ox, oz, CELL_SIZE, clock];
    u.bloom_grid = pack_grid(&grid);
}

/// Test hook (the `--bloom <level>` screenshot flag): every cell at `level`.
pub fn force_flat(u: &mut CameraUniform, level: f32, clock: f32) {
    let grid = [level.clamp(0.0, 1.0); CELLS];
    u.bloom_origin = [
        u.camera_pos[0] - CELL_SIZE * (GRID_SIDE / 2) as f32,
        u.camera_pos[2] - CELL_SIZE * (GRID_SIDE / 2) as f32,
        CELL_SIZE,
        clock,
    ];
    u.bloom_grid = pack_grid(&grid);
}

fn cell(grid: &[f32; CELLS], i: i32, j: i32) -> f32 {
    let n = GRID_SIDE as i32 - 1;
    grid[(j.clamp(0, n) as usize) * GRID_SIDE + i.clamp(0, n) as usize]
}

/// CPU twin of the shader's `bloom_level_at`: the level at world `(x, z)`,
/// bilinear between cell centres, and the camera cell's value outside the grid.
pub fn sample_level(grid: &[f32; CELLS], ox: f32, oz: f32, cam_x: f32, cam_z: f32, x: f32, z: f32) -> f32 {
    let side = GRID_SIDE as f32;
    let (gx, gz) = ((x - ox) / CELL_SIZE, (z - oz) / CELL_SIZE);
    if gx < 0.0 || gz < 0.0 || gx >= side || gz >= side {
        let (cx, cz) = (((cam_x - ox) / CELL_SIZE).floor() as i32, ((cam_z - oz) / CELL_SIZE).floor() as i32);
        return cell(grid, cx, cz);
    }
    let (ux, uz) = (gx - 0.5, gz - 0.5);
    let (fx, fz) = (ux.floor(), uz.floor());
    let (tx, tz) = (ux - fx, uz - fz);
    let (i, j) = (fx as i32, fz as i32);
    let a = cell(grid, i, j) * (1.0 - tx) + cell(grid, i + 1, j) * tx;
    let b = cell(grid, i, j + 1) * (1.0 - tx) + cell(grid, i + 1, j + 1) * tx;
    a * (1.0 - tz) + b * tz
}

/// Level below which a place starts to lose colour, and above which it
/// brightens. Between them the colour is untouched.
pub const GREY_BELOW: f32 = 0.45;
pub const VIVID_ABOVE: f32 = 0.75;
/// Saturation left at level 0.
pub const GREY_MIN_SATURATION: f32 = 0.15;
/// Extra saturation at level 1.
pub const VIVID_EXTRA_SATURATION: f32 = 0.15;

/// CPU twin of the shader's `bloom_grade_level`.
pub fn grade(rgb: [f32; 3], level: f32) -> [f32; 3] {
    let lum = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    let (sat, cast) = if level < GREY_BELOW {
        let t = (1.0 - level / GREY_BELOW).clamp(0.0, 1.0);
        (1.0 - (1.0 - GREY_MIN_SATURATION) * t, lerp3([1.0; 3], [0.95, 0.98, 1.04], t))
    } else if level > VIVID_ABOVE {
        let t = ((level - VIVID_ABOVE) / (1.0 - VIVID_ABOVE)).clamp(0.0, 1.0);
        (1.0 + VIVID_EXTRA_SATURATION * t, lerp3([1.0; 3], [1.04, 1.015, 0.95], t))
    } else {
        return rgb;
    };
    let mut out = [0.0; 3];
    for k in 0..3 {
        out[k] = ((lum + (rgb[k] - lum) * sat) * cast[k]).max(0.0);
    }
    out
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// CPU twin of the shader's `bloom_sway_amplitude`: how far a plant's tip
/// leans, in blocks. None at or below 0.3, gentle (0.035) at 0.6, a bit more
/// (0.06) at 1.0.
pub fn sway_amplitude(level: f32) -> f32 {
    0.035 * smoothstep(0.3, 0.6, level) + 0.025 * smoothstep(0.6, 1.0, level)
}

// ── Time of day ──────────────────────────────────────────────────────

/// What the sky is doing, for ambience.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DayPhase {
    Day,
    /// Dawn and dusk: neither birds nor crickets.
    Twilight,
    Night,
}

impl DayPhase {
    /// From the sky brightness (`camera::compute_sun`, 0..=1).
    pub fn from_brightness(b: f32) -> Self {
        if b >= 0.5 {
            Self::Day
        } else if b <= 0.2 {
            Self::Night
        } else {
            Self::Twilight
        }
    }
}

// ── Sound ────────────────────────────────────────────────────────────

/// Below this level a place is still: no birdsong, no insects.
pub const SOUND_MIN_LEVEL: f32 = 0.3;

/// Seconds between ambient one-shots at `level`, or `None` when the place is
/// still. 60 s at 0.3, 35 s at 0.45, 20 s at 1.0 (piecewise linear). It was
/// 20, 12 and 3 s until the v0.2.31 playtest: in Creative, whose floor keeps
/// every place vivid, a chirp every 2 to 4 s was the complaint.
pub fn sound_interval(level: f32) -> Option<f32> {
    if !level.is_finite() || level < SOUND_MIN_LEVEL {
        return None;
    }
    let l = level.min(1.0);
    Some(if l < 0.45 {
        60.0 + (l - 0.3) / 0.15 * (35.0 - 60.0)
    } else {
        35.0 + (l - 0.45) / 0.55 * (20.0 - 35.0)
    })
}

/// One thing the ambience wants played.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AmbientSound {
    /// A bird; the seed picks the chirp.
    Birdsong(u64),
    Insects,
}

/// Presentation state: the shader clock and each local player's next-sound timer.
#[derive(Default)]
pub struct BloomPresent {
    clock: f32,
    timers: Vec<f32>,
    counter: u64,
}

impl BloomPresent {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The shader clock in seconds.
    pub fn clock(&self) -> f32 {
        self.clock
    }

    /// Advance the shader clock (per frame; clamp `dt` upstream).
    pub fn advance_clock(&mut self, dt: f32) {
        // Keep f32 precision for the sway: a multiple of every sway period is
        // not needed, a rare phase jump after about 11 h of play is.
        self.clock = (self.clock + dt) % 40_000.0;
    }

    fn roll(&mut self, lane: u64) -> f32 {
        self.counter = self.counter.wrapping_add(1);
        (crate::particles::ambient_jitter(self.counter.wrapping_mul(0x2545_F491), lane) + 1.0) * 0.5
    }

    /// Count the timers down by `dt` and return what is due. `levels` holds
    /// one entry per local player: the bloom level where they stand. `calm`
    /// is false in rain. Birdsong by day, insects by night, nothing at dusk,
    /// nothing below [`SOUND_MIN_LEVEL`]; an interval that shrinks as the
    /// level rises, jittered by -30% to +30%.
    pub fn due_sounds(&mut self, dt: f32, levels: &[f32], phase: DayPhase, calm: bool) -> Vec<AmbientSound> {
        while self.timers.len() < levels.len() {
            // A first sound a few seconds after arriving, not on the spot.
            let first = 3.0 + self.roll(11) * 6.0;
            self.timers.push(first);
        }
        self.timers.truncate(levels.len());
        let mut due = Vec::new();
        for (k, &level) in levels.iter().enumerate() {
            self.timers[k] -= dt;
            if self.timers[k] > 0.0 {
                continue;
            }
            let interval = if calm { sound_interval(level) } else { None };
            match (interval, phase) {
                (Some(iv), DayPhase::Day) => {
                    let seed = self.counter.wrapping_add(k as u64 * 977);
                    due.push(AmbientSound::Birdsong(seed));
                    self.timers[k] = iv * (0.7 + 0.6 * self.roll(12));
                }
                (Some(iv), DayPhase::Night) => {
                    due.push(AmbientSound::Insects);
                    self.timers[k] = iv * (0.7 + 0.6 * self.roll(13));
                }
                // Still, wet or the wrong hour: look again in a second.
                _ => self.timers[k] = 1.0,
            }
        }
        due
    }
}

// ── Life ─────────────────────────────────────────────────────────────

/// Pollen motes per second near a player at `level`: none below 0.45, a few
/// in the middle band, more above 0.75.
pub fn pollen_per_sec(level: f32) -> f32 {
    life_rate(level, 0.6, 2.5)
}

/// Fireflies per second near a player at `level` (same shape, fewer).
pub fn firefly_per_sec(level: f32) -> f32 {
    life_rate(level, 0.4, 1.6)
}

fn life_rate(level: f32, at_vivid: f32, at_full: f32) -> f32 {
    if !level.is_finite() || level < GREY_BELOW {
        0.0
    } else if level < VIVID_ABOVE {
        (level - GREY_BELOW) / (VIVID_ABOVE - GREY_BELOW) * at_vivid
    } else {
        at_vivid + (level.min(1.0) - VIVID_ABOVE) / (1.0 - VIVID_ABOVE) * (at_full - at_vivid)
    }
}

/// How many motes to spawn this pulse: the whole part of `expected`, plus one
/// more when `roll` (0..1) falls under the fraction.
pub fn spawn_count(expected: f32, roll: f32) -> u32 {
    if !expected.is_finite() || expected <= 0.0 {
        return 0;
    }
    let whole = expected.floor();
    whole as u32 + u32::from(roll < expected - whole)
}

/// Horizontal reach of the ambient motes round a player, in blocks.
const LIFE_RADIUS: f32 = 14.0;

// ── The one place this meets `GameState` ─────────────────────────────

impl GameState {
    /// The sky phase and whether it is calm, as the ambience reads them.
    fn bloom_sky(&self) -> (DayPhase, bool) {
        let t = self.world.effective_world_time(self.world_time);
        let phase = DayPhase::from_brightness(crate::camera::compute_sun(t).1);
        (phase, self.tick_counter >= self.weather_rain_until)
    }

    /// Per frame: run the shader clock and play the birdsong or insects that
    /// are due, from the level under each local player. The audio engine
    /// applies the master volume and mute.
    pub(crate) fn bloom_ambient_sound(&mut self, dt: f32) {
        self.bloom_present.advance_clock(dt);
        let (phase, calm) = self.bloom_sky();
        let levels: Vec<f32> = self
            .players
            .iter()
            .map(|p| self.bloom.level_at(p.player.pos.x, p.player.pos.z))
            .collect();
        for s in self.bloom_present.due_sounds(dt, &levels, phase, calm) {
            match s {
                AmbientSound::Birdsong(seed) => self.audio.play_birdsong(seed),
                AmbientSound::Insects => self.audio.play_insects(),
            }
        }
    }

    /// Per ambient pulse (`pulse_secs` apart): pollen by day and fireflies at
    /// night round each local player, at a rate set by the local level.
    /// Halved on the Reduced particle tier; the pool's cap does the rest.
    pub(crate) fn bloom_ambient_life(&mut self, seed_base: u64, pulse_secs: f32, reduced: bool) {
        let (phase, calm) = self.bloom_sky();
        if !calm || phase == DayPhase::Twilight {
            return;
        }
        let tier = if reduced { 0.5 } else { 1.0 };
        for pidx in 0..self.players.len() {
            let eye = self.players[pidx].player.eye_pos();
            let level = self.bloom.level_at(eye.x, eye.z);
            let per_sec = if phase == DayPhase::Day { pollen_per_sec(level) } else { firefly_per_sec(level) };
            let sd0 = seed_base.wrapping_mul(0x9E37_79B9).wrapping_add((pidx as u64) << 44);
            let roll = (crate::particles::ambient_jitter(sd0, 90) + 1.0) * 0.5;
            let n = spawn_count(per_sec * pulse_secs * tier, roll);
            for i in 0..u64::from(n) {
                let sd = sd0.wrapping_add(i * 1637);
                let px = eye.x + crate::particles::ambient_jitter(sd, 91) * LIFE_RADIUS;
                let pz = eye.z + crate::particles::ambient_jitter(sd, 92) * LIFE_RADIUS;
                let ground = crate::particles::surface_kill_y(&self.world, px, eye.y + 3.0, pz);
                if ground < eye.y - 40.0 {
                    continue; // open air all the way down: nothing to hover over
                }
                let height = 0.4 + (crate::particles::ambient_jitter(sd, 93) + 1.0) * 0.5 * 1.6;
                let at = Vec3::new(px, ground + height, pz);
                if phase == DayPhase::Day {
                    self.particles.pollen_mote(at, sd);
                } else {
                    self.particles.firefly(at, sd);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn packing_keeps_cell_order_four_to_a_vec4() {
        let mut g = [0.0_f32; CELLS];
        for (k, v) in g.iter_mut().enumerate() {
            *v = k as f32 / 255.0;
        }
        let p = pack_grid(&g);
        assert_eq!(p.len(), 64);
        assert!(close(p[0][0], 0.0));
        assert!(close(p[0][3], 3.0 / 255.0));
        assert!(close(p[1][0], 4.0 / 255.0));
        assert!(close(p[63][3], 1.0));
    }

    #[test]
    fn an_empty_field_fills_the_uniform_neutral_and_enabled() {
        let bloom = BloomField::new();
        let mut u = CameraUniform::from_camera(&crate::camera::Camera::new(Vec3::new(40.0, 70.0, -9.0), 1.0));
        // Off until filled.
        assert_eq!(u.bloom_origin[2], 0.0);
        fill_uniform(&mut u, &bloom, 12.5);
        assert_eq!(u.bloom_origin[2], CELL_SIZE);
        assert_eq!(u.bloom_origin[3], 12.5);
        // The grid is centred on the camera: its min corner is 8 cells back.
        assert_eq!(u.bloom_origin[0], (40.0_f32 / 16.0).floor() * 16.0 - 128.0);
        assert_eq!(u.bloom_origin[1], (-9.0_f32 / 16.0).floor() * 16.0 - 128.0);
        assert!(u.bloom_grid.iter().flatten().all(|&l| close(l, NEUTRAL)));
    }

    #[test]
    fn the_neutral_uniform_is_off_and_flat() {
        let u = CameraUniform::from_camera(&crate::camera::Camera::new(Vec3::ZERO, 1.0));
        assert_eq!(u.bloom_origin, crate::camera::BLOOM_ORIGIN_OFF);
        assert!(u.bloom_grid.iter().flatten().all(|&l| close(l, NEUTRAL)));
        // And the uniform is a whole number of vec4s (WGSL layout).
        assert_eq!(std::mem::size_of::<CameraUniform>() % 16, 0);
        assert_eq!(std::mem::size_of::<CameraUniform>(), 64 + 16 * 6 + 16 + 1024);
    }

    #[test]
    fn force_flat_covers_the_camera() {
        let mut u = CameraUniform::from_camera(&crate::camera::Camera::new(Vec3::new(5.0, 70.0, 5.0), 1.0));
        force_flat(&mut u, 0.0, 0.0);
        assert!(u.bloom_grid.iter().flatten().all(|&l| l == 0.0));
        assert!(u.bloom_origin[0] < 5.0 && u.bloom_origin[0] + 256.0 > 5.0);
    }

    #[test]
    fn the_grade_leaves_the_middle_band_alone() {
        let c = [0.3, 0.6, 0.2];
        for l in [0.45, 0.5, 0.6, 0.75] {
            assert_eq!(grade(c, l), c, "level {l}");
        }
    }

    #[test]
    fn the_grade_is_continuous_at_both_edges() {
        let c = [0.3, 0.6, 0.2];
        for (l, edge) in [(0.449, 0.45), (0.751, 0.75)] {
            let (a, b) = (grade(c, l), grade(c, edge));
            for k in 0..3 {
                assert!((a[k] - b[k]).abs() < 0.01, "level {l} channel {k}");
            }
        }
    }

    fn saturation(c: [f32; 3]) -> f32 {
        let lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        ((c[0] - lum).abs() + (c[1] - lum).abs() + (c[2] - lum).abs()) / 3.0
    }

    #[test]
    fn a_dead_place_keeps_fifteen_percent_saturation_and_a_cool_cast() {
        let c = [0.3, 0.6, 0.2];
        let g = grade(c, 0.0);
        // The cast shifts the channels a few percent, so compare to the cast
        // grey, not the raw luminance: well under a third of the colour is left.
        assert!(saturation(g) < saturation(c) * 0.25, "{g:?}");
        assert!(saturation(g) > 0.0);
        assert!(g[2] / g[0] > c[2] / c[0], "cooler: blue gains on red");
        // Monotone: the lower the level, the less colour.
        let mid = saturation(grade(c, 0.2));
        assert!(saturation(g) < mid && mid < saturation(c));
    }

    #[test]
    fn a_vivid_place_gains_saturation_and_a_warm_lift() {
        let c = [0.3, 0.6, 0.2];
        let g = grade(c, 1.0);
        assert!(saturation(g) > saturation(c) * 1.1, "{g:?}");
        assert!(g[0] / g[2] > c[0] / c[2], "warmer: red gains on blue");
        assert!(grade([0.0; 3], 1.0).iter().all(|&v| v == 0.0), "black stays black");
        assert!(grade([1.0; 3], 0.0).iter().all(|&v| v.is_finite() && v >= 0.0));
    }

    #[test]
    fn sampling_is_bilinear_and_falls_back_to_the_camera_cell() {
        let mut g = [0.6_f32; CELLS];
        // Cells (8, 8) = 0.0 and (9, 8) = 1.0 along x, origin (0, 0).
        g[8 * GRID_SIDE + 8] = 0.0;
        g[8 * GRID_SIDE + 9] = 1.0;
        // Cell centres: x = 8.5 * 16 = 136 and 152; z = 136.
        assert!(close(sample_level(&g, 0.0, 0.0, 130.0, 130.0, 136.0, 136.0), 0.0));
        assert!(close(sample_level(&g, 0.0, 0.0, 130.0, 130.0, 152.0, 136.0), 1.0));
        assert!(close(sample_level(&g, 0.0, 0.0, 130.0, 130.0, 144.0, 136.0), 0.5));
        // Off the grid: the camera cell (8, 8) = 0.0.
        assert!(close(sample_level(&g, 0.0, 0.0, 130.0, 130.0, -500.0, 900.0), 0.0));
        // Edges clamp rather than read out of bounds.
        assert!(close(sample_level(&g, 0.0, 0.0, 130.0, 130.0, 0.5, 0.5), 0.6));
        assert!(close(sample_level(&g, 0.0, 0.0, 130.0, 130.0, 255.9, 255.9), 0.6));
    }

    #[test]
    fn grass_sways_not_at_all_when_grey_and_more_when_vivid() {
        assert_eq!(sway_amplitude(0.0), 0.0);
        assert_eq!(sway_amplitude(0.3), 0.0);
        assert!(close(sway_amplitude(0.6), 0.035));
        assert!(close(sway_amplitude(1.0), 0.06));
        assert!(sway_amplitude(0.45) > 0.0 && sway_amplitude(0.45) < sway_amplitude(0.6));
        assert!(sway_amplitude(1.0) < 0.1, "subtle");
    }

    #[test]
    fn day_phase_splits_the_sky_brightness() {
        assert_eq!(DayPhase::from_brightness(1.0), DayPhase::Day);
        assert_eq!(DayPhase::from_brightness(0.5), DayPhase::Day);
        assert_eq!(DayPhase::from_brightness(0.35), DayPhase::Twilight);
        assert_eq!(DayPhase::from_brightness(0.15), DayPhase::Night);
    }

    #[test]
    fn the_sound_interval_shrinks_with_level_and_is_none_when_still() {
        assert_eq!(sound_interval(0.0), None);
        assert_eq!(sound_interval(0.29), None);
        assert!(close(sound_interval(0.3).unwrap(), 60.0));
        assert!(close(sound_interval(0.45).unwrap(), 35.0));
        assert!(close(sound_interval(1.0).unwrap(), 20.0));
        assert!(close(sound_interval(7.0).unwrap(), 20.0), "clamped");
        let mut last = f32::MAX;
        for k in 30..=100 {
            let iv = sound_interval(k as f32 / 100.0).unwrap();
            assert!(iv <= last + 1e-4, "never lengthens as the level rises");
            last = iv;
        }
        assert_eq!(sound_interval(f32::NAN), None);
    }

    fn run(p: &mut BloomPresent, secs: f32, levels: &[f32], phase: DayPhase, calm: bool) -> Vec<AmbientSound> {
        let mut all = Vec::new();
        let mut t = 0.0;
        while t < secs {
            all.extend(p.due_sounds(0.1, levels, phase, calm));
            t += 0.1;
        }
        all
    }

    #[test]
    fn birds_by_day_insects_by_night_nothing_in_a_grey_place() {
        let mut p = BloomPresent::default();
        let day = run(&mut p, 120.0, &[1.0], DayPhase::Day, true);
        assert!(day.len() >= 4, "a lively day: {}", day.len());
        assert!(day.iter().all(|s| matches!(s, AmbientSound::Birdsong(_))));
        let mut p = BloomPresent::default();
        let night = run(&mut p, 120.0, &[1.0], DayPhase::Night, true);
        assert!(night.len() >= 4);
        assert!(night.iter().all(|s| *s == AmbientSound::Insects));
        let mut p = BloomPresent::default();
        assert!(run(&mut p, 600.0, &[0.2], DayPhase::Day, true).is_empty(), "grey is still");
        let mut p = BloomPresent::default();
        assert!(run(&mut p, 600.0, &[1.0], DayPhase::Twilight, true).is_empty(), "dusk is quiet");
        let mut p = BloomPresent::default();
        assert!(run(&mut p, 600.0, &[1.0], DayPhase::Day, false).is_empty(), "rain is quiet");
    }

    #[test]
    fn a_livelier_place_sings_more_often() {
        let count = |level| {
            let mut p = BloomPresent::default();
            run(&mut p, 600.0, &[level], DayPhase::Day, true).len()
        };
        let (low, mid, high) = (count(0.4), count(0.6), count(1.0));
        assert!(low < mid && mid < high, "{low} {mid} {high}");
        // Around 35 s at 0.45 and 20 s at 1.0, jitter averaging out.
        assert!((12..=24).contains(&count(0.45)), "{}", count(0.45));
        assert!((22..=40).contains(&high), "{high}");
    }

    #[test]
    fn each_local_player_has_their_own_timer_and_the_first_sound_is_not_instant() {
        let mut p = BloomPresent::default();
        assert!(p.due_sounds(0.05, &[1.0, 1.0], DayPhase::Day, true).is_empty());
        // Only the player standing somewhere lively sings.
        let mut p = BloomPresent::default();
        let s = run(&mut p, 120.0, &[0.1, 1.0], DayPhase::Day, true);
        assert!(!s.is_empty());
        // A player leaving drops their timer.
        p.due_sounds(0.1, &[1.0], DayPhase::Day, true);
        assert_eq!(p.timers.len(), 1);
    }

    #[test]
    fn the_shader_clock_advances_and_resets() {
        let mut p = BloomPresent::default();
        p.advance_clock(0.5);
        p.advance_clock(0.25);
        assert!(close(p.clock(), 0.75));
        p.reset();
        assert_eq!(p.clock(), 0.0);
    }

    #[test]
    fn motes_are_none_below_forty_five_and_grow_above_seventy_five() {
        for f in [pollen_per_sec, firefly_per_sec] {
            assert_eq!(f(0.0), 0.0);
            assert_eq!(f(0.44), 0.0);
            assert_eq!(f(f32::NAN), 0.0);
            assert!(f(0.6) > 0.0);
            assert!(f(0.9) > f(0.75) && f(1.0) > f(0.9));
            assert!(f(1.0) < 3.0, "gentle");
            let mut last = 0.0;
            for k in 0..=100 {
                let r = f(k as f32 / 100.0);
                assert!(r >= last - 1e-5, "monotone");
                last = r;
            }
        }
        assert!(pollen_per_sec(1.0) > firefly_per_sec(1.0));
    }

    #[test]
    fn spawn_count_is_the_whole_part_plus_a_chance_of_one_more() {
        assert_eq!(spawn_count(0.0, 0.0), 0);
        assert_eq!(spawn_count(-1.0, 0.0), 0);
        assert_eq!(spawn_count(f32::NAN, 0.0), 0);
        assert_eq!(spawn_count(0.4, 0.3), 1);
        assert_eq!(spawn_count(0.4, 0.5), 0);
        assert_eq!(spawn_count(2.5, 0.9), 2);
        assert_eq!(spawn_count(2.5, 0.1), 3);
        // The long-run mean matches the expectation.
        let total: u32 = (0..1000).map(|k| spawn_count(0.45, k as f32 / 1000.0)).sum();
        assert!((total as f32 - 450.0).abs() <= 1.0);
    }
}

//! The 2D climate fields of the v3 terrain (§3): continentalness C, erosion
//! E, weirdness W, temperature T, humidity H, the river field R, and the
//! escarpment field S (added while tuning, so §11's cliff share holds: see
//! `lattice::SCARP_HEIGHT`).
//!
//! Each is hand-summed fBm over one `noise::OpenSimplex` (lacunarity 2, gain
//! 0.5), scaled by a constant so its sampled std is about 0.35, then clamped to
//! [-1, 1]. The scale constants were measured once by
//! `field_spread_is_about_0_35` (below) and are hard-coded: they depend only on
//! the octave count, not on the seed or the wavelength.

use noise::{NoiseFn, OpenSimplex};

/// Per-octave sample offsets, so the octaves of one field never all pass
/// through the same lattice point (every OpenSimplex octave is 0 at the origin).
const OCTAVE_OFFSET: [(f64, f64); 4] = [(0.0, 0.0), (37.17, -11.29), (-71.53, 53.81), (19.37, 97.13)];

/// Scale that brings a hand-summed fBm of 1, 2, 3 or 4 octaves to std ≈ 0.35
/// (`field_spread_is_about_0_35` measured the raw spread over 1M samples).
pub const NORM: [f64; 4] = [1.539, 1.378, 1.345, 1.336];

/// One fBm field.
pub struct Fbm {
    noise: OpenSimplex,
    octaves: usize,
    /// Frequency of the first octave: 1 / base wavelength (blocks).
    freq: f64,
}

impl Fbm {
    pub fn new(seed: u32, octaves: usize, wavelength: f64) -> Self {
        Self { noise: OpenSimplex::new(seed), octaves, freq: 1.0 / wavelength }
    }

    /// The unscaled sum `Σ 0.5^i · n(x·f·2^i, z·f·2^i)`.
    pub fn raw(&self, x: f64, z: f64) -> f64 {
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut f = self.freq;
        for &(ox, oz) in OCTAVE_OFFSET.iter().take(self.octaves) {
            sum += amp * self.noise.get([x * f + ox, z * f + oz]);
            amp *= 0.5;
            f *= 2.0;
        }
        sum
    }

    /// The field: std ≈ 0.35, in [-1, 1].
    pub fn get(&self, x: f64, z: f64) -> f64 {
        (self.raw(x, z) * NORM[self.octaves - 1]).clamp(-1.0, 1.0)
    }
}

/// The six fields at one point.
#[derive(Clone, Copy, Debug)]
pub struct Climate {
    pub c: f64,
    pub e: f64,
    pub w: f64,
    pub t: f64,
    pub h: f64,
    pub r: f64,
    pub s: f64,
}

/// The fields of one world.
pub struct Fields {
    pub c: Fbm,
    pub e: Fbm,
    pub w: Fbm,
    pub t: Fbm,
    pub h: Fbm,
    pub r: Fbm,
    pub s: Fbm,
}

impl Fields {
    /// Salts sit far from the v2 generator's (0..=4000) so no v3 field shares
    /// a permutation table with a v2 one.
    pub fn new(seed: u32) -> Self {
        let s = |salt: u32| seed.wrapping_add(salt);
        Self {
            c: Fbm::new(s(31_001), 4, 2048.0),
            e: Fbm::new(s(31_002), 3, 1024.0),
            w: Fbm::new(s(31_003), 3, 512.0),
            t: Fbm::new(s(31_004), 2, 1536.0),
            h: Fbm::new(s(31_005), 2, 1536.0),
            r: Fbm::new(s(31_006), 2, 768.0),
            s: Fbm::new(s(31_008), 2, 256.0),
        }
    }

    pub fn sample(&self, x: i32, z: i32) -> Climate {
        let (x, z) = (x as f64, z as f64);
        Climate {
            c: self.c.get(x, z),
            e: self.e.get(x, z),
            w: self.w.get(x, z),
            t: self.t.get(x, z),
            h: self.h.get(x, z),
            r: self.r.get(x, z),
            s: self.s.get(x, z),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Measures the raw fBm spread per octave count and checks the hard-coded
    /// `NORM` brings it to 0.35 (±0.02). Run with `--nocapture` to re-measure.
    #[test]
    fn field_spread_is_about_0_35() {
        for octaves in 1..=4 {
            let f = Fbm::new(12_345, octaves, 64.0);
            let (mut n, mut sum, mut sq) = (0.0, 0.0, 0.0);
            // 256×256 samples 7 blocks apart: ~28 wavelengths each way.
            for i in 0..256 {
                for k in 0..256 {
                    let v = f.raw(i as f64 * 7.0, k as f64 * 7.0);
                    n += 1.0;
                    sum += v;
                    sq += v * v;
                }
            }
            let mean = sum / n;
            let std = (sq / n - mean * mean).sqrt();
            println!("octaves {octaves}: raw std {std:.4} -> norm {:.3}", 0.35 / std);
            let scaled = std * NORM[octaves - 1];
            assert!((scaled - 0.35).abs() < 0.02, "octaves {octaves}: scaled std {scaled:.3}");
        }
    }
}

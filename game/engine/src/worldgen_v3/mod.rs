//! Terrain generator v3, "dramatic terrain"
//! (docs/foundations/2026-10-23-terrain-overhaul.md; as-built: Spec 02 §5.3.8).
//!
//! Continuous climate fields (C, E, W, T, H, R) on a 4-block lattice give a
//! 2D height (`height2d`) through piecewise-linear splines; a 3D density noise
//! on a 4×8×4 lattice adds overhangs and cliffs; surface rules paint the top;
//! biomes are labels read off the same fields. Selected per world by
//! `biome::Generator::V3`; v2 worlds never reach this code.
//!
//! Determinism (§0.2): f64; only `+ - * /`, `sqrt`, `abs`, `floor`/`ceil`,
//! `min`/`max` and comparisons; column-clipped and order-independent; the
//! thread-local memos are lookup-only.
//!
//! Frozen as shipped (2026-10-24, `biome::V3_REVISION` 1): shape, surface and
//! labels are v3's own; caves are v2's flooded noise caves (`fill::solid_block`)
//! and trees, vegetation and structures v2's decoration (`World::generate_column`).
//! Do not change v3 output: the dry caves and v3 trees go in the next generator.

pub mod climate;
pub mod density;
pub mod fill;
pub mod labels;
pub mod lattice;
pub mod spline;
pub mod surface;

#[cfg(test)]
mod metrics;
#[cfg(test)]
mod tests;

use crate::biome::Biome;
use density::Density;
use lattice::{Lattice, Sample};
use noise::{NoiseFn, OpenSimplex};

/// The v3 terrain of one world.
pub struct Terrain {
    pub lattice: Lattice,
    pub density: Density,
    detail: OpenSimplex,
}

impl Terrain {
    pub fn new(seed: u32) -> Self {
        Self {
            lattice: Lattice::new(seed),
            density: Density::new(seed),
            detail: OpenSimplex::new(seed.wrapping_add(31_201)),
        }
    }

    pub fn seed(&self) -> u32 {
        self.lattice.seed
    }

    /// Every 2D quantity at column `(x, z)`.
    pub fn sample(&self, x: i32, z: i32) -> Sample {
        self.lattice.sample(x, z)
    }

    /// The 2D surface height (§4), before the 3D noise.
    pub fn height2d(&self, x: i32, z: i32) -> f64 {
        self.lattice.height2d(x, z)
    }

    /// The v3 `terrain_height`: the top solid y of `height2d` alone.
    pub fn terrain_height(&self, x: i32, z: i32) -> i32 {
        lattice::top_of(self.height2d(x, z))
    }

    /// The v3 biome label (§8).
    pub fn biome_at(&self, x: i32, z: i32) -> Biome {
        let s = self.sample(x, z);
        labels::biome(&s, lattice::top_of(s.h))
    }

    /// Whether `(x, y, z)` is solid terrain (before caves). Pure, and equal to
    /// the column fill.
    pub fn solid_at(&self, x: i32, y: i32, z: i32) -> bool {
        if y == 0 {
            return true;
        }
        let s = self.sample(x, z);
        y > 0 && self.density.solid_at(s.h, s.j, x, y, z)
    }

    /// The real top solid y of column `(x, z)` with the 3D noise (overhangs
    /// included; caves not), where `terrain_height` is the 2D estimate.
    pub fn surface_y(&self, x: i32, z: i32) -> i32 {
        let s = self.sample(x, z);
        let (lo, hi) = density::noisy_band(s.h, s.j);
        let lo = lo.max(1);
        for y in (lo..=hi.min(density::HEIGHT_CAP)).rev() {
            if self.density.solid_at(s.h, s.j, x, y, z) {
                return y;
            }
        }
        lo - 1
    }

    /// A small 2D detail noise in [-1, 1] at `wavelength`, offset by `salt`
    /// (snow line, gravel patches).
    pub fn detail_noise(&self, x: i32, z: i32, wavelength: f64, salt: f64) -> f64 {
        let f = 1.0 / wavelength;
        (self.detail.get([x as f64 * f + salt, z as f64 * f - salt]) * 2.6).clamp(-1.0, 1.0)
    }
}

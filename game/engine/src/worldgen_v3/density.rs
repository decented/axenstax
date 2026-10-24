//! 3D shape of the v3 terrain (§5): overhangs and cliffs.
//!
//! `density(x, y, z) = (h − y) + J · n3(x/32, y/24, z/32)`, solid where
//! `density > 0`. `n3` is one OpenSimplex 3D noise, scaled to std ≈
//! [`N3_SCALE`]·0.19 and clamped to [-1, 1], sampled on a lattice every
//! [`XZ`] blocks in x/z and [`Y`] blocks in y (globally aligned) and
//! trilinearly interpolated. Because the interpolated value is always within
//! [-1, 1], a cell with `y < h − J` is solid and one with `y ≥ h + J` is air
//! without sampling the noise; [`is_solid`] is the one decision both the column
//! fill and the pure [`Density::solid_at`] make.

use noise::{NoiseFn, OpenSimplex};
use std::cell::RefCell;

/// Lattice spacing in x/z and in y.
pub const XZ: i32 = 4;
pub const Y: i32 = 8;
/// Noise frequencies: x/32, y/24, z/32 (§5).
const FX: f64 = 1.0 / 32.0;
const FY: f64 = 1.0 / 24.0;
/// Scale on the raw 3D OpenSimplex (raw std ≈ 0.19) before the clamp.
pub const N3_SCALE: f64 = 2.6;
/// Nothing generates above this y (§4's height cap): `h` is clamped to it and
/// the noise may not lift a cell past it.
pub const HEIGHT_CAP: i32 = 240;

/// Solid or not, for a column of height `h` and jaggedness `j` at `y`; `n`
/// yields the interpolated noise (only called inside the noisy band).
#[inline]
pub fn is_solid(h: f64, j: f64, y: i32, n: impl FnOnce() -> f64) -> bool {
    if y > HEIGHT_CAP {
        return false;
    }
    let yf = y as f64;
    if yf < h - j {
        return true;
    }
    if yf >= h + j {
        return false;
    }
    (h - yf) + j * n() > 0.0
}

/// The lowest and highest y whose solidity depends on the noise, for a column
/// of height `h` and jaggedness `j` (every y below is solid, every y above air).
pub fn noisy_band(h: f64, j: f64) -> (i32, i32) {
    ((h - j).floor() as i32, (h + j).ceil() as i32)
}

/// Trilinear blend of the 8 lattice corners `c[dx + 2·dy + 4·dz]`.
#[inline]
pub fn trilerp(c: &[f64; 8], fx: f64, fy: f64, fz: f64) -> f64 {
    let l = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = l(c[0], c[1], fx);
    let x10 = l(c[2], c[3], fx);
    let x01 = l(c[4], c[5], fx);
    let x11 = l(c[6], c[7], fx);
    l(l(x00, x10, fy), l(x01, x11, fy), fz)
}

/// Fractions of `(x, y, z)` inside its lattice cell.
#[inline]
pub fn fractions(x: i32, y: i32, z: i32) -> (f64, f64, f64) {
    (
        x.rem_euclid(XZ) as f64 / XZ as f64,
        y.rem_euclid(Y) as f64 / Y as f64,
        z.rem_euclid(XZ) as f64 / XZ as f64,
    )
}

const MEMO_BITS: u32 = 13;

thread_local! {
    static MEMO: RefCell<Vec<(u32, i32, i32, i32, f64, bool)>> =
        RefCell::new(vec![(0, 0, 0, 0, 0.0, false); 1 << MEMO_BITS]);
}

fn slot_index(seed: u32, ix: i32, iy: i32, iz: i32) -> usize {
    let h = (ix as u32).wrapping_mul(0x9E37_79B1)
        ^ (iy as u32).wrapping_mul(0x27D4_EB2F)
        ^ (iz as u32).wrapping_mul(0x85EB_CA77)
        ^ seed.wrapping_mul(0xC2B2_AE3D);
    ((h ^ (h >> 15)) & ((1 << MEMO_BITS) - 1)) as usize
}

/// The 3D density noise of one world.
pub struct Density {
    seed: u32,
    noise: OpenSimplex,
}

impl Density {
    pub fn new(seed: u32) -> Self {
        Self { seed, noise: OpenSimplex::new(seed.wrapping_add(31_101)) }
    }

    /// The noise at lattice point `(ix·XZ, iy·Y, iz·XZ)`, in [-1, 1].
    pub fn compute(&self, ix: i32, iy: i32, iz: i32) -> f64 {
        let v = self.noise.get([
            (ix * XZ) as f64 * FX,
            (iy * Y) as f64 * FY,
            (iz * XZ) as f64 * FX,
        ]);
        (v * N3_SCALE).clamp(-1.0, 1.0)
    }

    /// [`Self::compute`], memoised (thread-local, direct-mapped).
    pub fn point(&self, ix: i32, iy: i32, iz: i32) -> f64 {
        let i = slot_index(self.seed, ix, iy, iz);
        let hit = MEMO.with(|m| {
            let e = m.borrow()[i];
            (e.5 && e.0 == self.seed && e.1 == ix && e.2 == iy && e.3 == iz).then_some(e.4)
        });
        if let Some(v) = hit {
            return v;
        }
        let v = self.compute(ix, iy, iz);
        MEMO.with(|m| m.borrow_mut()[i] = (self.seed, ix, iy, iz, v, true));
        v
    }

    /// The 8 lattice corners around `(x, y, z)`.
    pub fn corners(&self, x: i32, y: i32, z: i32) -> [f64; 8] {
        let (ix, iy, iz) = (x.div_euclid(XZ), y.div_euclid(Y), z.div_euclid(XZ));
        let mut c = [0.0; 8];
        for (k, v) in c.iter_mut().enumerate() {
            let k = k as i32;
            *v = self.point(ix + (k & 1), iy + ((k >> 1) & 1), iz + ((k >> 2) & 1));
        }
        c
    }

    /// The interpolated noise at `(x, y, z)`.
    pub fn n3(&self, x: i32, y: i32, z: i32) -> f64 {
        let (fx, fy, fz) = fractions(x, y, z);
        trilerp(&self.corners(x, y, z), fx, fy, fz)
    }

    /// Pure solidity of `(x, y, z)` for a column of height `h`, jaggedness `j`.
    pub fn solid_at(&self, h: f64, j: f64, x: i32, y: i32, z: i32) -> bool {
        is_solid(h, j, y, || self.n3(x, y, z))
    }
}

/// The density-noise lattice a column fill needs, sampled once: lattice x/z
/// points `ix0..=ix0+4`, `iz0..=iz0+4` and y points `iy0..=iy1`. Holds exactly
/// [`Density::point`]'s values, so [`ColumnNoise::n3`] equals
/// [`Density::n3`] bit-for-bit.
pub struct ColumnNoise {
    ix0: i32,
    iy0: i32,
    iz0: i32,
    ny: usize,
    v: Vec<f64>,
}

impl ColumnNoise {
    /// Lattice for chunk column `(cx, cz)` covering the y-band `lo..=hi`.
    pub fn new(d: &Density, cx: i32, cz: i32, lo: i32, hi: i32) -> Self {
        let ix0 = (cx * 16).div_euclid(XZ);
        let iz0 = (cz * 16).div_euclid(XZ);
        let n = (16 / XZ + 1) as usize;
        let (iy0, iy1) = (lo.div_euclid(Y), hi.div_euclid(Y) + 1);
        let ny = (iy1 - iy0 + 1).max(0) as usize;
        let mut v = Vec::with_capacity(n * n * ny);
        for kz in 0..n as i32 {
            for kx in 0..n as i32 {
                for ky in 0..ny as i32 {
                    v.push(d.compute(ix0 + kx, iy0 + ky, iz0 + kz));
                }
            }
        }
        Self { ix0, iy0, iz0, ny, v }
    }

    fn at(&self, ix: i32, iy: i32, iz: i32) -> f64 {
        let n = (16 / XZ + 1) as usize;
        let (kx, ky, kz) = ((ix - self.ix0) as usize, (iy - self.iy0) as usize, (iz - self.iz0) as usize);
        self.v[(kz * n + kx) * self.ny + ky]
    }

    /// The interpolated noise at `(x, y, z)` (inside this column and band).
    pub fn n3(&self, x: i32, y: i32, z: i32) -> f64 {
        let (ix, iy, iz) = (x.div_euclid(XZ), y.div_euclid(Y), z.div_euclid(XZ));
        let mut c = [0.0; 8];
        for (k, v) in c.iter_mut().enumerate() {
            let k = k as i32;
            *v = self.at(ix + (k & 1), iy + ((k >> 1) & 1), iz + ((k >> 2) & 1));
        }
        let (fx, fy, fz) = fractions(x, y, z);
        trilerp(&c, fx, fy, fz)
    }
}

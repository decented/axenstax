//! The 2D lattice of the v3 terrain (§3, §4): every climate field and the
//! derived height, relief and jaggedness are computed at x, z multiples of
//! [`STEP`] (globally aligned) and bilinearly interpolated in between.
//!
//! [`Lattice::sample`] is the one pure function both the column fill and every
//! pure query (`height2d`, `biome_at`, `terrain_block_at`) go through, so they
//! agree bit-for-bit by construction. A small thread-local direct-mapped memo
//! holds recent lattice points (lookup only, never iterated; a collision just
//! recomputes), keyed by the seed so generators of different seeds never mix.

use super::climate::Fields;
use super::spline::{clamp01, lerp, smoothstep, Spline};
use std::cell::RefCell;

/// Lattice spacing in x and z (blocks).
pub const STEP: i32 = 4;

/// Sea level (the v2 generator's, unchanged: `biome::SEA_LEVEL`).
const SEA: f64 = 62.0;

/// `base(C)`: the continental floor/plain height (§4).
pub static BASE: Spline = Spline(&[
    (-1.0, 28.0),
    (-0.6, 32.0),
    (-0.5, 37.0),
    (-0.45, 44.0),
    (-0.25, 52.0),
    (-0.15, 58.0),
    (-0.08, 62.0),
    (-0.02, 65.0),
    (0.2, 67.0),
    (0.6, 72.0),
    (1.0, 78.0),
]);

/// `A(E)`: relief amplitude in blocks; low erosion = mountainous (§4).
pub static RELIEF: Spline = Spline(&[
    (-1.0, 160.0),
    (-0.6, 140.0),
    (-0.3, 70.0),
    (0.0, 28.0),
    (0.4, 9.0),
    (1.0, 4.0),
]);

/// Escarpments (added in tuning; §11 asks ≥ 1.5 % of land at slope ≥ 6, which
/// smooth splines on a 4-block lattice never reach): along the zero contour of
/// the S field the land steps up by up to `SCARP_HEIGHT` blocks over a band of
/// ±`SCARP_BAND` in S (~2 blocks, smeared to one lattice cell), scaled by
/// `clamp01((SCARP_E0 − E) / SCARP_EW)` so flat, eroded plains keep none.
pub const SCARP_HEIGHT: f64 = 44.0;
pub const SCARP_BAND: f64 = 0.008;
pub const SCARP_E0: f64 = 0.3;
pub const SCARP_EW: f64 = 0.5;

/// River half-width in |R| units at full width (h ≤ [`RIVER_FULL_BELOW`]).
pub const RIVER_WIDTH: f64 = 0.06;
/// River bed height (water fills to sea level).
pub const RIVER_BED: f64 = 57.0;
/// Rivers run full width below this height…
pub const RIVER_FULL_BELOW: f64 = 95.0;
/// …and shrink to nothing at this one, so they never saw through mountains.
pub const RIVER_NONE_ABOVE: f64 = 110.0;
/// Height clamp (§4).
pub const H_MIN: f64 = 6.0;
pub const H_MAX: f64 = 240.0;
/// Jaggedness (blocks) in full mountains, and everywhere inland (§5).
pub const J_MOUNTAIN: f64 = 18.0;
pub const J_BASE: f64 = 2.0;

/// Every 2D quantity of the terrain at one point. At a lattice point these
/// are computed; between, each is the bilinear blend of the four corners.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// Continentalness.
    pub c: f64,
    /// Erosion.
    pub e: f64,
    /// Peaks-and-valleys, `1 − |3·|W| − 2|`.
    pub pv: f64,
    /// Temperature.
    pub t: f64,
    /// Humidity.
    pub hum: f64,
    /// 0 at sea, 1 inland.
    pub inland: f64,
    /// Jaggedness: amplitude (blocks) of the 3D density noise.
    pub j: f64,
    /// Surface height (blocks), rivers carved.
    pub h: f64,
    /// River mask: > 0 inside a river channel, else ≤ 0.
    pub river: f64,
}

/// `shape(PV)`: PV ≥ 0 → `PV·√PV` (peaks), PV < 0 → `0.35·PV` (valleys).
pub fn shape(pv: f64) -> f64 {
    if pv >= 0.0 { pv * pv.sqrt() } else { 0.35 * pv }
}

/// The terrain at lattice point `(ix·STEP, iz·STEP)`.
pub fn compute(fields: &Fields, ix: i32, iz: i32) -> Sample {
    let cl = fields.sample(ix * STEP, iz * STEP);
    let pv = 1.0 - (3.0 * cl.w.abs() - 2.0).abs();
    let inland = smoothstep(-0.12, 0.05, cl.c);
    let scarp = SCARP_HEIGHT
        * clamp01((SCARP_E0 - cl.e) / SCARP_EW)
        * smoothstep(-SCARP_BAND, SCARP_BAND, cl.s);
    let relief = RELIEF.eval(cl.e) * shape(pv) + scarp;
    let mut h = (BASE.eval(cl.c) + inland * relief).clamp(H_MIN, H_MAX);
    let mut j = inland * (J_MOUNTAIN * clamp01((-cl.e - 0.2) / 0.5) * clamp01(pv) + J_BASE);
    // Rivers (§4): a channel where |R| is small, narrowing to nothing between
    // RIVER_FULL_BELOW and RIVER_NONE_ABOVE. The channel never raises ground
    // (a lake bed below 57 stays), and the 3D noise fades out in it so the bed
    // is clean.
    let r = cl.r.abs();
    let width = RIVER_WIDTH * clamp01((RIVER_NONE_ABOVE - h) / (RIVER_NONE_ABOVE - RIVER_FULL_BELOW));
    let river = if inland > 0.0 { width - r } else { -1.0 };
    if river > 0.0 {
        let s = smoothstep(0.0, 1.0, r / width);
        h = h.min(lerp(RIVER_BED, h, s));
        j *= s;
    }
    Sample { c: cl.c, e: cl.e, pv, t: cl.t, hum: cl.h, inland, j, h, river }
}

/// Bilinear blend of four corner samples at fractions `fx`, `fz` in [0, 1).
fn bilerp(a: &Sample, b: &Sample, c: &Sample, d: &Sample, fx: f64, fz: f64) -> Sample {
    let f = |p: f64, q: f64, r: f64, s: f64| lerp(lerp(p, q, fx), lerp(r, s, fx), fz);
    Sample {
        c: f(a.c, b.c, c.c, d.c),
        e: f(a.e, b.e, c.e, d.e),
        pv: f(a.pv, b.pv, c.pv, d.pv),
        t: f(a.t, b.t, c.t, d.t),
        hum: f(a.hum, b.hum, c.hum, d.hum),
        inland: f(a.inland, b.inland, c.inland, d.inland),
        j: f(a.j, b.j, c.j, d.j),
        h: f(a.h, b.h, c.h, d.h),
        river: f(a.river, b.river, c.river, d.river),
    }
}

const MEMO_BITS: u32 = 12;

#[derive(Clone, Copy)]
struct Slot {
    seed: u32,
    ix: i32,
    iz: i32,
    used: bool,
    s: Sample,
}

thread_local! {
    static MEMO: RefCell<Vec<Slot>> = RefCell::new(vec![
        Slot { seed: 0, ix: 0, iz: 0, used: false, s: Sample::default() };
        1 << MEMO_BITS
    ]);
}

fn slot_index(seed: u32, ix: i32, iz: i32) -> usize {
    let h = (ix as u32).wrapping_mul(0x9E37_79B1)
        ^ (iz as u32).wrapping_mul(0x85EB_CA77)
        ^ seed.wrapping_mul(0xC2B2_AE3D);
    ((h ^ (h >> 15)) & ((1 << MEMO_BITS) - 1)) as usize
}

/// The 2D lattice of one world.
pub struct Lattice {
    pub seed: u32,
    pub fields: Fields,
}

impl Lattice {
    pub fn new(seed: u32) -> Self {
        Self { seed, fields: Fields::new(seed) }
    }

    /// The lattice point `(ix, iz)` (memoised).
    pub fn point(&self, ix: i32, iz: i32) -> Sample {
        let i = slot_index(self.seed, ix, iz);
        let hit = MEMO.with(|m| {
            let e = m.borrow()[i];
            (e.used && e.seed == self.seed && e.ix == ix && e.iz == iz).then_some(e.s)
        });
        if let Some(s) = hit {
            return s;
        }
        let s = compute(&self.fields, ix, iz);
        MEMO.with(|m| m.borrow_mut()[i] = Slot { seed: self.seed, ix, iz, used: true, s });
        s
    }

    /// The terrain at block column `(x, z)`.
    pub fn sample(&self, x: i32, z: i32) -> Sample {
        let (ix, iz) = (x.div_euclid(STEP), z.div_euclid(STEP));
        let fx = x.rem_euclid(STEP) as f64 / STEP as f64;
        let fz = z.rem_euclid(STEP) as f64 / STEP as f64;
        let a = self.point(ix, iz);
        let b = self.point(ix + 1, iz);
        let c = self.point(ix, iz + 1);
        let d = self.point(ix + 1, iz + 1);
        bilerp(&a, &b, &c, &d, fx, fz)
    }

    /// `height2d(x, z)` (§4), the surface height before the 3D noise.
    pub fn height2d(&self, x: i32, z: i32) -> f64 {
        self.sample(x, z).h
    }

    /// Slope (§6): the largest height step to a 4-neighbour.
    pub fn slope(&self, x: i32, z: i32, h: f64) -> f64 {
        let d = |dx: i32, dz: i32| (self.height2d(x + dx, z + dz) - h).abs();
        d(1, 0).max(d(-1, 0)).max(d(0, 1)).max(d(0, -1))
    }
}

/// The top solid y of a column of height `h` with no 3D noise: the highest
/// integer `y < h`. This is the v3 `terrain_height`.
pub fn top_of(h: f64) -> i32 {
    h.ceil() as i32 - 1
}

/// Whether a column is underwater (its top below the sea's surface block).
pub fn is_wet(top: i32) -> bool {
    (top as f64) < SEA
}

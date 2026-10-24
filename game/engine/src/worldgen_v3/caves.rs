//! Caves of the **v4** terrain (§7; v4 = v3 + these dry caves, v3 stays frozen): dry cheese caverns and spaghetti tunnels.
//!
//! Three noise fields (cheese `c`, tunnel pair `a`, `b`) live on a 4×4×4
//! lattice (globally aligned) and are trilinearly interpolated, exactly the
//! `density` pattern. A *solid* cell is carved where `c > CHEESE_MIN`
//! (caverns, y ≥ [`CHEESE_FLOOR`], ≥ [`CHEESE_ROOF`] below the 2D top) or
//! `a² + b² < SPAG_R2` (tunnels, y ≥ [`SPAG_FLOOR`], reaching the surface
//! only on dry land with a top ≥ [`MOUTH_TOP`] where the mouth mask allows). A carved cell at y ≤
//! [`LAVA_Y`] is lava, otherwise air. Caves are never flooded: [`CaveColumn`]'s
//! `cap` keeps every cell at y ≤ 62 clear of any water the column or a
//! 4-neighbour can hold (see [`water_cap`]).
//!
//! Only a v4 world builds any of this ([`Terrain::caves`] is `None` for v3);
//! the v3 fill keeps v2's flooded noise caves.
//!
//! Both the column fill ([`ColumnCaves`]) and the pure query ([`CaveNoise`])
//! hand the same eight corners to [`carve`], so they agree bit for bit.

use super::density::{noisy_band, trilerp};
use super::lattice::{is_wet, Sample};
use super::surface::Column;
use super::Terrain;
use crate::block::{self, BlockId};
use noise::{NoiseFn, OpenSimplex};
use std::cell::RefCell;

/// Lattice spacing (x, y and z).
pub const STEP: i32 = 4;
/// Raw 3D OpenSimplex (std ≈ 0.19) → ≈ 0.5 before the clamp, as `density`.
const SCALE: f64 = 2.6;
/// Cheese: `fbm3(x/96, y/64, z/96)`, two octaves (the second at twice the
/// frequency, half the weight), carved above this.
///
/// Tuned: design §7 said 0.62 (and 0.012 for the tunnels), which measured 4.63 %
/// of the underground carved over seeds 1, 42 and 1337 (9 areas each), under
/// the 5 % floor; 0.58 and 0.014 measure 5.86 % (`cave_metrics`).
pub const CHEESE_MIN: f64 = 0.58;
const CHEESE_F: (f64, f64) = (1.0 / 96.0, 1.0 / 64.0);
/// Spaghetti: two fields at (x/48, y/32, z/48), carved inside this radius².
pub const SPAG_R2: f64 = 0.014;
const SPAG_F: (f64, f64) = (1.0 / 48.0, 1.0 / 32.0);
/// A field whose eight corners all lie beyond this (> √SPAG_R2, with slack
/// for rounding) cannot dip inside the tunnel radius: the interpolation is a
/// convex blend.
pub const SPAG_REJECT: f64 = 0.125;
/// Slack on the cheese corner test (rounding of the trilinear blend).
const EPS: f64 = 1e-9;
/// Lowest carved y for caverns and tunnels.
pub const CHEESE_FLOOR: i32 = 8;
pub const SPAG_FLOOR: i32 = 6;
/// Caverns stop this far under the 2D top, and never above [`CHEESE_CEIL`].
pub const CHEESE_ROOF: i32 = 10;
pub const CHEESE_CEIL: i32 = 120;
/// A tunnel may break the surface (a cave mouth) only on a column topping at
/// least this that the 2D mouth mask allows ([`MOUTH_MASK`]); elsewhere it
/// stays [`ROOF`] under the top (dry land) or [`SEABED`] (top < [`MOUTH_TOP`]).
pub const MOUTH_TOP: i32 = 65;
/// Roof a tunnel leaves under a dry column's top where no mouth is allowed.
pub const ROOF: i32 = 4;
/// Mouth mask: a 2D noise (x/64, z/64, ×2.6, clamped) above this lets tunnels
/// open to the sky; it keeps mouths to a few per area instead of a surface
/// of holes (unmasked, 3 % of columns were open).
pub const MOUTH_MASK: f64 = 0.8;
const MOUTH_F: f64 = 1.0 / 64.0;
/// Carved cells at or below this are lava.
pub const LAVA_Y: i32 = 10;
/// §7: no carving within 6 blocks below the top of a wet column.
pub const SEABED: i32 = 7;
/// Water exists only at or below this y (sea level).
const WATER_Y: i32 = 62;

/// The cave noise of one world.
pub struct Caves {
    seed: u32,
    cheese: [OpenSimplex; 2],
    tunnel: [OpenSimplex; 2],
    mouth: OpenSimplex,
}

type Point = [f64; 3];

const MEMO_BITS: u32 = 13;

thread_local! {
    static MEMO: RefCell<Vec<(u32, i32, i32, i32, Point, bool)>> =
        RefCell::new(vec![(0, 0, 0, 0, [0.0; 3], false); 1 << MEMO_BITS]);
}

fn slot_index(seed: u32, ix: i32, iy: i32, iz: i32) -> usize {
    let h = (ix as u32).wrapping_mul(0x9E37_79B1)
        ^ (iy as u32).wrapping_mul(0x27D4_EB2F)
        ^ (iz as u32).wrapping_mul(0x85EB_CA77)
        ^ seed.wrapping_mul(0x1656_67B1);
    ((h ^ (h >> 15)) & ((1 << MEMO_BITS) - 1)) as usize
}

impl Caves {
    pub fn new(seed: u32) -> Self {
        let n = |k: u32| OpenSimplex::new(seed.wrapping_add(31_300 + k));
        Self { seed, cheese: [n(1), n(2)], tunnel: [n(3), n(4)], mouth: n(5) }
    }

    /// `[cheese, a, b]` at lattice point `(ix·4, iy·4, iz·4)`, each in [-1, 1].
    pub fn compute(&self, ix: i32, iy: i32, iz: i32) -> Point {
        let (x, y, z) = ((ix * STEP) as f64, (iy * STEP) as f64, (iz * STEP) as f64);
        let f = |n: &OpenSimplex, fr: (f64, f64), m: f64| {
            (n.get([x * fr.0 * m, y * fr.1 * m, z * fr.0 * m]) * SCALE).clamp(-1.0, 1.0)
        };
        let cheese = (f(&self.cheese[0], CHEESE_F, 1.0) + 0.5 * f(&self.cheese[1], CHEESE_F, 2.0)) / 1.5;
        [cheese, f(&self.tunnel[0], SPAG_F, 1.0), f(&self.tunnel[1], SPAG_F, 1.0)]
    }

    /// Whether tunnels may open to the sky at column `(x, z)` (the mouth mask).
    pub fn mouth_at(&self, x: i32, z: i32) -> bool {
        (self.mouth.get([x as f64 * MOUTH_F, z as f64 * MOUTH_F]) * SCALE).clamp(-1.0, 1.0) > MOUTH_MASK
    }

    /// [`Self::compute`], memoised (thread-local, direct-mapped, lookup only).
    fn point(&self, ix: i32, iy: i32, iz: i32) -> Point {
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
}

/// Where the eight lattice corners around a cell come from.
pub trait CaveNoise {
    /// The corners `c[dx + 2·dy + 4·dz]` around `(x, y, z)`.
    fn corners(&self, x: i32, y: i32, z: i32) -> [Point; 8];
}

impl CaveNoise for Caves {
    fn corners(&self, x: i32, y: i32, z: i32) -> [Point; 8] {
        let (ix, iy, iz) = (x.div_euclid(STEP), y.div_euclid(STEP), z.div_euclid(STEP));
        let mut c = [[0.0; 3]; 8];
        for (k, v) in c.iter_mut().enumerate() {
            let k = k as i32;
            *v = self.point(ix + (k & 1), iy + ((k >> 1) & 1), iz + ((k >> 2) & 1));
        }
        c
    }
}

/// The lattice a chunk-column fill needs, sampled once; holds exactly
/// [`Caves::compute`]'s values.
pub struct ColumnCaves {
    ix0: i32,
    iz0: i32,
    ny: usize,
    v: Vec<Point>,
}

impl ColumnCaves {
    /// Lattice for chunk column `(cx, cz)` covering y `0..=max_y`.
    pub fn new(c: &Caves, cx: i32, cz: i32, max_y: i32) -> Self {
        let (ix0, iz0) = ((cx * 16).div_euclid(STEP), (cz * 16).div_euclid(STEP));
        let n = (16 / STEP + 1) as usize;
        let ny = (max_y.max(0).div_euclid(STEP) + 2) as usize;
        let mut v = Vec::with_capacity(n * n * ny);
        for kz in 0..n as i32 {
            for kx in 0..n as i32 {
                for ky in 0..ny as i32 {
                    v.push(c.compute(ix0 + kx, ky, iz0 + kz));
                }
            }
        }
        Self { ix0, iz0, ny, v }
    }
}

impl CaveNoise for ColumnCaves {
    fn corners(&self, x: i32, y: i32, z: i32) -> [Point; 8] {
        let n = (16 / STEP + 1) as usize;
        let (ix, iy, iz) = (x.div_euclid(STEP), y.div_euclid(STEP), z.div_euclid(STEP));
        let mut c = [[0.0; 3]; 8];
        for (k, v) in c.iter_mut().enumerate() {
            let k = k as i32;
            let (kx, ky, kz) = (ix + (k & 1) - self.ix0, iy + ((k >> 1) & 1), iz + ((k >> 2) & 1) - self.iz0);
            *v = self.v[(kz as usize * n + kx as usize) * self.ny + ky as usize];
        }
        c
    }
}

/// The y ≤ 62 ceiling of [`CaveColumn::cap`] for column `(x, z)` with sample
/// `s` and 2D top `top`: water can only sit in a cell the density leaves empty
/// (y ≥ `noisy_band(h, j).0`), and only at y ≤ 62, so a cell at y ≤ 62 stays
/// two below that floor in its own column and in each 4-neighbour (a carved
/// cell next to water would be drained into at runtime). A wet column also
/// keeps [`SEABED`] under its top (§7).
pub fn water_cap(t: &Terrain, x: i32, z: i32, s: &Sample, top: i32) -> i32 {
    let floor = |s: &Sample| noisy_band(s.h, s.j).0 - 2;
    let mut cap = floor(s);
    if is_wet(top) {
        cap = cap.min(top - SEABED);
    }
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        cap = cap.min(floor(&t.lattice.sample(x + dx, z + dz)));
    }
    cap
}

/// The 2D, per-column facts the cave pass needs beyond [`Column`]. Built
/// (identically) only on the v4 path, so a v3 column pays nothing for caves.
#[derive(Clone, Copy, Debug)]
pub struct CaveColumn {
    /// Highest y ≤ 62 a cave may carve here without touching water
    /// ([`water_cap`]).
    pub cap: i32,
    /// Tunnels may open to the sky here ([`Caves::mouth_at`]).
    pub mouth: bool,
}

impl CaveColumn {
    pub fn new(t: &Terrain, caves: &Caves, x: i32, z: i32, col: &Column) -> Self {
        Self { cap: water_cap(t, x, z, &col.s, col.top), mouth: caves.mouth_at(x, z) }
    }
}

/// What a *solid* cell `(x, y, z)` of column `col` becomes when a cave takes
/// it: lava at y ≤ [`LAVA_Y`], else air; `None` when it stays rock.
pub fn carve<N: CaveNoise>(col: &Column, cc: &CaveColumn, n: &N, x: i32, y: i32, z: i32) -> Option<BlockId> {
    let spag_top = if col.top < MOUTH_TOP {
        col.top - SEABED
    } else if cc.mouth {
        col.top
    } else {
        col.top - ROOF
    };
    if y < SPAG_FLOOR || y > spag_top || (y <= WATER_Y && y > cc.cap) {
        return None;
    }
    let spag = y <= spag_top;
    let cheese = y >= CHEESE_FLOOR && y <= (col.top - CHEESE_ROOF).min(CHEESE_CEIL);
    if !spag && !cheese {
        return None;
    }
    let c = n.corners(x, y, z);
    // Convex blend: a field whose corners all clear a bound cannot cross it.
    let (mut cmax, mut amin, mut amax, mut bmin, mut bmax) = (-2.0f64, 2.0f64, -2.0f64, 2.0f64, -2.0f64);
    for p in &c {
        cmax = cmax.max(p[0]);
        amin = amin.min(p[1]);
        amax = amax.max(p[1]);
        bmin = bmin.min(p[2]);
        bmax = bmax.max(p[2]);
    }
    let cheese = cheese && cmax > CHEESE_MIN - EPS;
    let spag = spag
        && amin < SPAG_REJECT
        && amax > -SPAG_REJECT
        && bmin < SPAG_REJECT
        && bmax > -SPAG_REJECT;
    if !cheese && !spag {
        return None;
    }
    let (fx, fy, fz) = (
        x.rem_euclid(STEP) as f64 / STEP as f64,
        y.rem_euclid(STEP) as f64 / STEP as f64,
        z.rem_euclid(STEP) as f64 / STEP as f64,
    );
    let field = |k: usize| trilerp(&[c[0][k], c[1][k], c[2][k], c[3][k], c[4][k], c[5][k], c[6][k], c[7][k]], fx, fy, fz);
    let inside = (cheese && field(0) > CHEESE_MIN) || (spag && {
        let (a, b) = (field(1), field(2));
        a * a + b * b < SPAG_R2
    });
    inside.then_some(if y <= LAVA_Y { block::LAVA } else { block::AIR })
}

/// Whether the cave pass carves `(x, y, z)`: the cell is density-solid and
/// [`carve`] takes it. Pure; equals the column fill. (A cell the density leaves
/// empty, such as the air above a mouth, is not "carved".) `false` for a
/// terrain without caves (v3).
pub fn is_carved(t: &Terrain, x: i32, y: i32, z: i32) -> bool {
    let Some(caves) = t.caves() else { return false };
    if y <= 0 || y >= super::fill::HEIGHT as i32 {
        return false;
    }
    let s = t.sample(x, z);
    if !t.density.solid_at(s.h, s.j, x, y, z) {
        return false;
    }
    let col = Column::new(t, x, z, s);
    carve(&col, &CaveColumn::new(t, caves, x, z, &col), caves, x, y, z).is_some()
}

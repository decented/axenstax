//! v4 cave acceptance metrics (§11) over real chunk-filled regions.
//!
//! The fast tests assert the bounds on a few 16×16-chunk areas (256×256
//! blocks) so a regression fails `check.sh`; the `#[ignore]` one measures
//! more areas and prints the table:
//!
//! ```text
//! cargo test --release --lib worldgen_v3::cave_metrics -- --ignored --nocapture
//! ```
//!
//! Underground = the cells from y 1 up to the 2D top. A cell is *carved* when
//! the density leaves it solid and the block is air or lava. A *leak* is a
//! carved cell with a water neighbour (so "0 water cells inside caves" also
//! covers seepage at runtime). A *mouth* is a 4-connected cluster of columns
//! whose real surface cell (`Terrain::surface_y`, density only) is carved,
//! at y ≥ 63.

use super::fill::{column_blocks, HEIGHT};
use super::lattice::{is_wet, top_of};
use super::Terrain;
use crate::biome::{BiomeGenerator, Generator};
use crate::block::{self, BlockId};
use std::collections::{HashMap, HashSet};

const SEEDS: [u32; 3] = [1, 42, 1337];
/// An area is 16×16 chunks.
const AREA: i32 = 16;

/// A filled rectangle of chunk columns.
struct Region {
    cx0: i32,
    cz0: i32,
    n: i32,
    cols: HashMap<(i32, i32), Vec<BlockId>>,
}

impl Region {
    fn fill(bg: &BiomeGenerator, t: &Terrain, cx0: i32, cz0: i32, n: i32) -> Self {
        let mut cols = HashMap::new();
        for cx in cx0..cx0 + n {
            for cz in cz0..cz0 + n {
                cols.insert((cx, cz), column_blocks(bg, t, cx, cz));
            }
        }
        Self { cx0, cz0, n, cols }
    }

    fn block(&self, x: i32, y: i32, z: i32) -> Option<BlockId> {
        if !(0..HEIGHT as i32).contains(&y) {
            return None;
        }
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        let col = self.cols.get(&(cx, cz))?;
        Some(col[((z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize) * HEIGHT + y as usize])
    }

    /// Block coordinate range `[lo, hi)` of the region in x (and z).
    fn span(&self) -> ((i32, i32), (i32, i32)) {
        ((self.cx0 * 16, (self.cx0 + self.n) * 16), (self.cz0 * 16, (self.cz0 + self.n) * 16))
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct CaveStats {
    pub underground: u64,
    pub carved: u64,
    pub lava: u64,
    pub leaks: u64,
    pub mouths: u64,
    pub areas: u64,
}

impl CaveStats {
    fn add(&mut self, o: &CaveStats) {
        self.underground += o.underground;
        self.carved += o.carved;
        self.lava += o.lava;
        self.leaks += o.leaks;
        self.mouths += o.mouths;
        self.areas += o.areas;
    }

    pub fn carved_pct(&self) -> f64 {
        100.0 * self.carved as f64 / self.underground.max(1) as f64
    }

    pub fn mouths_per_area(&self) -> f64 {
        self.mouths as f64 / self.areas.max(1) as f64
    }

    fn row(&self, label: &str) -> String {
        format!(
            "{label}: carved {:.2}% of {} underground | water leaks {} | lava {} | mouths {} in {} areas = {:.2}/area",
            self.carved_pct(),
            self.underground,
            self.leaks,
            self.lava,
            self.mouths,
            self.areas,
            self.mouths_per_area(),
        )
    }
}

/// Measure `r`, leaving a 1-block rim out of the leak test (no neighbours).
fn measure(t: &Terrain, r: &Region) -> CaveStats {
    let mut s = CaveStats { areas: (r.n as u64).pow(2) / (AREA as u64).pow(2).max(1), ..Default::default() };
    if s.areas == 0 {
        s.areas = 1;
    }
    let ((x0, x1), (z0, z1)) = r.span();
    let mut mouth_cols: HashSet<(i32, i32)> = HashSet::new();
    for x in x0..x1 {
        for z in z0..z1 {
            let smp = t.sample(x, z);
            let top = top_of(smp.h);
            let (lo, _) = super::density::noisy_band(smp.h, smp.j);
            for y in 1..=top.min(HEIGHT as i32 - 1) {
                s.underground += 1;
                let b = r.block(x, y, z).unwrap();
                if !matches!(b, block::AIR | block::LAVA) {
                    continue;
                }
                // Carved = the density holds it solid (below the noisy band it
                // always does).
                if y >= lo && !t.density.solid_at(smp.h, smp.j, x, y, z) {
                    continue;
                }
                s.carved += 1;
                if b == block::LAVA {
                    s.lava += 1;
                }
                let leak = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]
                    .iter()
                    .any(|&(dx, dy, dz)| r.block(x + dx, y + dy, z + dz) == Some(block::WATER));
                if leak {
                    s.leaks += 1;
                }
            }
            let ys = t.surface_y(x, z);
            if ys >= 63 && matches!(r.block(x, ys, z), Some(block::AIR | block::LAVA)) {
                mouth_cols.insert((x, z));
            }
        }
    }
    // 4-connected clusters of mouth columns.
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut starts: Vec<(i32, i32)> = mouth_cols.iter().copied().collect();
    starts.sort_unstable();
    for st in starts {
        if !seen.insert(st) {
            continue;
        }
        s.mouths += 1;
        let mut stack = vec![st];
        while let Some((x, z)) = stack.pop() {
            for n in [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)] {
                if mouth_cols.contains(&n) && seen.insert(n) {
                    stack.push(n);
                }
            }
        }
    }
    s
}

/// `areas` areas per seed, laid out in a row along x from the origin and
/// offset in z by seed, pooled over the seeds.
fn run(areas: &[(i32, i32)], tag: &str, print: bool) -> CaveStats {
    let mut pooled = CaveStats::default();
    for seed in SEEDS {
        let bg = BiomeGenerator::new(seed, Generator::V4);
        let t = bg.v3().unwrap();
        let mut seed_stats = CaveStats::default();
        for &(ax, az) in areas {
            let r = Region::fill(&bg, t, ax * AREA, az * AREA, AREA);
            let m = measure(t, &r);
            if print {
                println!("  {}", m.row(&format!("seed {seed} area ({ax},{az})")));
            }
            seed_stats.add(&m);
        }
        if print {
            println!("{}", seed_stats.row(&format!("seed {seed}")));
        }
        // A seed's volume needs enough areas to average out the terrain.
        assert!(
            areas.len() < 9 || (5.0..=8.0).contains(&seed_stats.carved_pct()),
            "seed {seed}: carved volume outside 5..8 %: {}",
            seed_stats.row("seed")
        );
        pooled.add(&seed_stats);
    }
    if print {
        println!("{}", pooled.row(tag));
    }
    pooled
}

fn assert_bounds(m: &CaveStats, what: &str) {
    let report = m.row(what);
    assert_eq!(m.leaks, 0, "water inside caves: {report}");
    assert!((5.0..=8.0).contains(&m.carved_pct()), "carved volume outside 5..8 %: {report}");
    assert!(m.mouths_per_area() >= 1.0, "fewer than one mouth per area: {report}");
    assert!(m.lava > 0, "no lava at the bottom: {report}");
}

#[test]
fn caves_meet_the_acceptance_bounds_on_a_small_region() {
    // A few areas per seed, away from the origin so they include mountains,
    // coast and plains: the full table comes from the ignored test.
    let m = run(&[(0, 0), (1, -1)], "small region, 3 seeds", false);
    assert_bounds(&m, "small region, 3 seeds");
}

#[test]
#[ignore]
fn cave_metrics_full_region() {
    let areas: Vec<(i32, i32)> = (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))).collect();
    let m = run(&areas, "pooled, 9 areas per seed", true);
    assert_bounds(&m, "pooled");
}

#[test]
fn a_coast_gets_caves_but_no_seepage() {
    // Chunk columns that hold both sea and dry land (found on the 2D height),
    // filled for real: no carved cell may touch water, yet the rule must not
    // be vacuous (carved cells at y <= 62 near the sea exist).
    let (mut checked, mut near_sea) = (0u64, 0u64);
    for seed in SEEDS {
        let bg = BiomeGenerator::new(seed, Generator::V4);
        let t = bg.v3().unwrap();
        let mut coast = Vec::new();
        'search: for cx in -60..60 {
            for cz in -60..60 {
                let (mut wet, mut dry) = (false, false);
                for (dx, dz) in [(0, 0), (15, 0), (0, 15), (15, 15), (8, 8)] {
                    let top = top_of(t.height2d(cx * 16 + dx, cz * 16 + dz));
                    wet |= is_wet(top);
                    dry |= top >= 68;
                }
                if wet && dry {
                    coast.push((cx, cz));
                    if coast.len() == 12 {
                        break 'search;
                    }
                }
            }
        }
        assert!(coast.len() >= 6, "seed {seed}: only {} coastal chunks", coast.len());
        for &(cx, cz) in &coast {
            let r = Region::fill(&bg, t, cx - 1, cz - 1, 3);
            let m = measure(t, &r);
            assert_eq!(m.leaks, 0, "seed {seed} chunk ({cx},{cz}): carved cells touch water");
            checked += m.carved;
            // Carved y <= 62 cells (air or lava there is always a cave: pockets
            // under overhangs fill with water) within 12 blocks of water.
            let ((x0, x1), (z0, z1)) = r.span();
            for x in x0..x1 {
                for z in z0..z1 {
                    if is_wet(top_of(t.height2d(x, z))) {
                        continue;
                    }
                    let near = (-12..=12).step_by(4).any(|d| {
                        is_wet(top_of(t.height2d(x + d, z))) || is_wet(top_of(t.height2d(x, z + d)))
                    });
                    if near {
                        near_sea += (1..=62)
                            .filter(|&y| matches!(r.block(x, y, z), Some(block::AIR | block::LAVA)))
                            .count() as u64;
                    }
                }
            }
        }
    }
    assert!(checked > 0, "no carved cells at all in the coastal chunks");
    assert!(near_sea > 0, "no carved cell at y <= 62 within 12 blocks of the sea: the seepage test is vacuous");
}

#[test]
fn caves_leave_the_seabed_alone() {
    // §7: nothing carved within 6 blocks below the top of a wet column.
    let bg = BiomeGenerator::new(42, Generator::V4);
    let t = bg.v3().unwrap();
    let mut wet_checked = 0;
    for x in (-600..600).step_by(7) {
        for z in (-600..600).step_by(7) {
            let top = top_of(t.height2d(x, z));
            if !is_wet(top) {
                continue;
            }
            wet_checked += 1;
            for y in (top - 6).max(1)..=top + 1 {
                assert!(!super::caves::is_carved(t, x, y, z), "carved seabed at ({x},{y},{z}) top {top}");
            }
        }
    }
    assert!(wet_checked > 1500, "only {wet_checked} wet columns sampled");
}

#[test]
fn only_lava_below_ten_and_air_above() {
    // Every carved cell is lava at y <= 10 and air above (sampled through the
    // pure query), and lava does appear in the filled blocks.
    let bg = BiomeGenerator::new(1337, Generator::V4);
    let t = bg.v3().unwrap();
    let (mut lava, mut air, mut checked) = (0u64, 0u64, 0u64);
    for cx in -3..3 {
        for cz in -3..3 {
            let blocks = column_blocks(&bg, t, cx, cz);
            for (i, &b) in blocks.iter().enumerate() {
                let y = (i % HEIGHT) as i32;
                lava += (b == block::LAVA) as u64;
                if i % 5 != 0 || y > 110 {
                    continue;
                }
                let c = (i / HEIGHT) as i32;
                let (x, z) = (cx * 16 + c % 16, cz * 16 + c / 16);
                checked += 1;
                if super::caves::is_carved(t, x, y, z) {
                    if y <= super::caves::LAVA_Y {
                        assert_eq!(b, block::LAVA, "({x},{y},{z})");
                    } else {
                        assert_eq!(b, block::AIR, "({x},{y},{z})");
                        air += 1;
                    }
                } else {
                    assert_ne!(b, block::LAVA, "uncarved lava at ({x},{y},{z})");
                }
            }
        }
    }
    assert!(air > 0 && lava > 0, "air {air} lava {lava} (checked {checked})");
}

#[test]
fn the_reject_bound_covers_the_tunnel_radius() {
    use super::caves::{SPAG_R2, SPAG_REJECT};
    assert!(SPAG_REJECT * SPAG_REJECT > SPAG_R2 + 1e-6);
}

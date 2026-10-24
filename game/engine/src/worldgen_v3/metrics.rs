//! v3 terrain acceptance metrics (§11), the map tool and the bench.
//!
//! Fast tests assert the §11 bounds on a sparse sample of a smaller region, so
//! a regression fails `check.sh`. The `#[ignore]` ones measure the full region
//! (≥ 4096×4096 blocks every 8 blocks, seeds 1, 42, 1337), draw the maps and
//! time a column:
//!
//! ```text
//! cargo test --lib worldgen_v3::metrics -- --ignored --nocapture
//! cargo test --release --lib worldgen_v3::metrics::bench -- --ignored --nocapture
//! ```

use super::labels;
use super::lattice::top_of;
use super::Terrain;
use crate::biome::{Biome, SEA_LEVEL};

pub const SEEDS: [u32; 3] = [1, 42, 1337];

/// §8 target share (% of land excluding mountains) per climate biome.
pub const TARGETS: [(Biome, f64); 8] = [
    (Biome::Plains, 22.0),
    (Biome::Forest, 18.0),
    (Biome::Savanna, 12.0),
    (Biome::Desert, 10.0),
    (Biome::Taiga, 10.0),
    (Biome::SnowyTundra, 7.0),
    (Biome::BirchForest, 6.0),
    (Biome::Jungle, 6.0),
];

#[derive(Default)]
pub struct Metrics {
    pub samples: usize,
    pub dry: Vec<i32>,
    pub ocean: usize,
    pub deep_floor: Vec<i32>,
    pub steep: usize,
    pub biomes: [usize; 13],
}

fn idx(b: Biome) -> usize {
    Biome::ALL.iter().position(|&x| x == b).unwrap()
}

impl Metrics {
    /// Sample a `2·half` square centred on the origin every `step` blocks.
    pub fn measure(&mut self, t: &Terrain, half: i32, step: usize) {
        for x in (-half..half).step_by(step) {
            for z in (-half..half).step_by(step) {
                let s = t.sample(x, z);
                let top = top_of(s.h);
                let b = labels::biome(&s, top);
                self.samples += 1;
                self.biomes[idx(b)] += 1;
                match b {
                    Biome::Ocean => self.ocean += 1,
                    Biome::DeepOcean => {
                        self.ocean += 1;
                        self.deep_floor.push(top);
                    }
                    _ => {}
                }
                if top >= SEA_LEVEL {
                    self.dry.push(top);
                    if t.lattice.slope(x, z, s.h) >= 6.0 {
                        self.steep += 1;
                    }
                }
            }
        }
    }

    fn pct(v: &mut [i32], p: f64) -> i32 {
        v.sort_unstable();
        v[((v.len() - 1) as f64 * p) as usize]
    }

    pub fn ocean_share(&self) -> f64 {
        100.0 * self.ocean as f64 / self.samples as f64
    }

    pub fn steep_share(&self) -> f64 {
        100.0 * self.steep as f64 / self.dry.len() as f64
    }

    /// Share (%) of `b` in land excluding oceans and mountains.
    pub fn land_share(&self, b: Biome) -> f64 {
        let land: usize = self.samples
            - self.ocean
            - self.biomes[idx(Biome::Mountains)];
        100.0 * self.biomes[idx(b)] as f64 / land as f64
    }

    pub fn report(&mut self, label: &str) -> String {
        let (p50, p99, max) = (
            Self::pct(&mut self.dry, 0.5),
            Self::pct(&mut self.dry, 0.99),
            *self.dry.iter().max().unwrap(),
        );
        let deep90 = Self::pct(&mut self.deep_floor, 0.9);
        let mut out = format!(
            "{label}: samples {} | dry p50 {p50} p99 {p99} max {max} | ocean+deep {:.1}% | deep floor p90 \
             {deep90} | steep {:.2}% | mountains {:.1}% of land\n  ",
            self.samples,
            self.ocean_share(),
            self.steep_share(),
            100.0 * self.biomes[idx(Biome::Mountains)] as f64 / (self.samples - self.ocean) as f64,
        );
        for b in Biome::ALL {
            if !matches!(b, Biome::Ocean | Biome::DeepOcean | Biome::Mountains) {
                out += &format!("{} {:.1}% ", b.name(), self.land_share(b));
            }
        }
        out
    }

    /// Every §11 bound this module measures; the list of failures.
    pub fn check(&mut self) -> Vec<String> {
        let mut bad = Vec::new();
        let p50 = Self::pct(&mut self.dry, 0.5);
        let p99 = Self::pct(&mut self.dry, 0.99);
        let max = *self.dry.iter().max().unwrap();
        let deep90 = Self::pct(&mut self.deep_floor, 0.9);
        if !(66..=78).contains(&p50) {
            bad.push(format!("dry p50 {p50} not in 66..=78"));
        }
        if p99 < 150 {
            bad.push(format!("dry p99 {p99} < 150"));
        }
        if max < 180 {
            bad.push(format!("dry max {max} < 180"));
        }
        if self.ocean_share() < 25.0 {
            bad.push(format!("ocean {:.1}% < 25%", self.ocean_share()));
        }
        if deep90 > 40 {
            bad.push(format!("deep floor p90 {deep90} > 40"));
        }
        if self.steep_share() < 1.5 {
            bad.push(format!("steep {:.2}% < 1.5%", self.steep_share()));
        }
        for (b, target) in TARGETS {
            let s = self.land_share(b);
            if (s - target).abs() > 5.0 {
                bad.push(format!("{} {s:.1}% not within 5 of {target}", b.name()));
            }
        }
        for b in Biome::ALL {
            if !matches!(b, Biome::Ocean | Biome::DeepOcean | Biome::Mountains) && self.land_share(b) > 30.0 {
                bad.push(format!("{} {:.1}% > 30%", b.name(), self.land_share(b)));
            }
        }
        let mountains = 100.0 * self.biomes[idx(Biome::Mountains)] as f64 / (self.samples - self.ocean) as f64;
        if mountains > 30.0 {
            bad.push(format!("Mountains {mountains:.1}% of land > 30%"));
        }
        bad
    }
}

fn all_seeds(half: i32, step: usize) -> Metrics {
    let mut m = Metrics::default();
    for seed in SEEDS {
        m.measure(&Terrain::new(seed), half, step);
    }
    m
}

#[test]
fn v3_terrain_meets_the_acceptance_bounds_on_a_sparse_sample() {
    // 8192² blocks every 32 (3 × 65k samples): sparse but wide, so it sees
    // several continents per seed. The full-resolution numbers come from the
    // ignored `metrics_full_region`.
    let mut m = all_seeds(4096, 32);
    let report = m.report("sparse, 3 seeds");
    let bad = m.check();
    assert!(bad.is_empty(), "{report}\n§11 bounds missed: {bad:?}");
}

#[test]
#[ignore]
fn metrics_full_region() {
    // §11 over seeds 1, 42, 1337: 8192×8192 blocks around the origin, every 8.
    let mut pooled = Metrics::default();
    for seed in SEEDS {
        let t = Terrain::new(seed);
        let mut m = Metrics::default();
        m.measure(&t, 4096, 8);
        println!("{}", m.report(&format!("seed {seed}")));
        pooled.measure(&t, 4096, 8);
    }
    println!("{}", pooled.report("pooled"));
    let bad = pooled.check();
    assert!(bad.is_empty(), "§11 bounds missed: {bad:?}");
}

/// Map colours per biome (index order of `Biome::ALL`).
const BIOME_RGB: [[u8; 3]; 13] = [
    [141, 179, 96],  // Plains
    [5, 102, 33],    // Forest
    [250, 148, 24],  // Desert
    [130, 130, 130], // Mountains
    [0, 0, 160],     // Ocean
    [48, 116, 68],   // BirchForest
    [11, 102, 89],   // Taiga
    [83, 123, 9],    // Jungle
    [189, 178, 95],  // Savanna
    [240, 250, 250], // SnowyTundra
    [0, 0, 80],      // DeepOcean
    [60, 110, 255],  // River
    [250, 222, 85],  // Beach
];

fn height_rgb(top: i32) -> [u8; 3] {
    if top < SEA_LEVEL {
        let d = ((SEA_LEVEL - top) * 4).min(200) as u8;
        return [0, 40, 255 - d];
    }
    let v = top - SEA_LEVEL;
    match v {
        0..=20 => [40 + v as u8 * 4, 140 + v as u8 * 3, 40],
        21..=80 => [120 + (v - 20) as u8, 200 - (v - 20) as u8 * 2, 40 + (v - 20) as u8],
        _ => {
            let w = (130 + (v - 80) * 2).min(255) as u8;
            [w, w, w]
        }
    }
}

#[test]
#[ignore]
fn map_tool_writes_height_and_biome_pngs() {
    // 2048×2048 blocks around the origin, 1 px = 4 blocks (a lattice point).
    // The repo root's `.claude/overnight/maps` (the main checkout's when run
    // from a linked worktree under `.claude/worktrees/`).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap();
    let root = root.to_str().unwrap();
    let root = root.split("/.claude/worktrees/").next().unwrap();
    let dir = std::path::Path::new(root).join(".claude/overnight/maps");
    std::fs::create_dir_all(&dir).unwrap();
    for seed in SEEDS {
        let t = Terrain::new(seed);
        let n = 512u32;
        let mut height = image::RgbImage::new(n, n);
        let mut biome = image::RgbImage::new(n, n);
        for px in 0..n {
            for pz in 0..n {
                let (x, z) = ((px as i32 - 256) * 4, (pz as i32 - 256) * 4);
                let s = t.sample(x, z);
                let top = top_of(s.h);
                height.put_pixel(px, pz, image::Rgb(height_rgb(top)));
                biome.put_pixel(px, pz, image::Rgb(BIOME_RGB[idx(labels::biome(&s, top))]));
            }
        }
        height.save(dir.join(format!("seed-{seed}-height.png"))).unwrap();
        biome.save(dir.join(format!("seed-{seed}-biome.png"))).unwrap();
    }
    println!("maps written to {}", dir.display());
}

#[test]
#[ignore]
fn bench_v3_column_vs_v2() {
    use crate::biome::{BiomeGenerator, Generator};
    use crate::world::World;
    // An 8×8 block of columns each (the memo warms as neighbours share
    // lattice points, as in streaming), full `generate_column` and fill only.
    for generator in [Generator::V2, Generator::V3, Generator::V4] {
        let bg = BiomeGenerator::new(42, generator);
        // A mountain region for v3's worst case, the origin for the average.
        let origins = match generator {
            Generator::V2 => vec![(0, 0)],
            Generator::V3 | Generator::V4 => vec![(0, 0), super::tests::find_column_above(bg.v3().unwrap(), 150.0)],
        };
        for (ox, oz) in origins {
            let mut w = World::new();
            let start = std::time::Instant::now();
            for cx in ox..ox + 8 {
                for cz in oz..oz + 8 {
                    w.generate_column(cx, cz, &bg);
                }
            }
            let full = start.elapsed().as_secs_f64() * 1000.0 / 64.0;
            let fill = if let Some(t) = bg.v3() {
                let start = std::time::Instant::now();
                for cx in ox + 8..ox + 16 {
                    for cz in oz..oz + 8 {
                        std::hint::black_box(super::fill::column_blocks(&bg, t, cx, cz));
                    }
                }
                start.elapsed().as_secs_f64() * 1000.0 / 64.0
            } else {
                f64::NAN
            };
            println!("{generator:?} at chunk ({ox},{oz}): generate_column {full:.3} ms/column, v3 fill only {fill:.3} ms");
        }
    }
}

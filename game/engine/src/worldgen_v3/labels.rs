//! Biome labels of the v3 terrain (§8). A biome is a label read off the same
//! fields the height came from, never a shape: there are no walls at borders.

use super::lattice::Sample;
use crate::biome::Biome;

/// §8.1: continentalness below which the sea is deep.
pub const DEEP_OCEAN_C: f64 = -0.5;
/// §8.1: continentalness below which the land is sea.
pub const OCEAN_C: f64 = -0.15;
/// §6.3 / §8.3: the beach band.
pub const BEACH_C: (f64, f64) = (-0.12, -0.08);
pub const BEACH_TOP: (i32, i32) = (59, 65);
/// §8.4: a column higher than this is Mountains whatever its climate.
pub const MOUNTAIN_TOP: i32 = 120;

/// The rugged-highland test (§8.4): low erosion and high peaks-and-valleys.
pub fn mountainous(s: &Sample) -> bool {
    s.inland > 0.0 && s.e < -0.35 && s.pv > 0.4
}

/// The beach band (§6.3, §8.3) for a column whose top solid is `top`.
pub fn beach_band(s: &Sample, top: i32) -> bool {
    (BEACH_C.0..=BEACH_C.1).contains(&s.c)
        && (BEACH_TOP.0..=BEACH_TOP.1).contains(&top)
        && !mountainous(s)
}

/// The label of a column with 2D terrain `s` and top `top` (§8, in order).
pub fn biome(s: &Sample, top: i32) -> Biome {
    if s.c < DEEP_OCEAN_C {
        return Biome::DeepOcean;
    }
    if s.c < OCEAN_C {
        return Biome::Ocean;
    }
    if s.river > 0.0 && top < 63 {
        return Biome::River;
    }
    if beach_band(s, top) {
        return Biome::Beach;
    }
    if top > MOUNTAIN_TOP || mountainous(s) {
        return Biome::Mountains;
    }
    whittaker(s.t, s.hum)
}

/// Temperature band edges (§8.5), tuned so land splits roughly as the design
/// asks (measured by `worldgen_v3::metrics`).
pub const T_EDGES: [f64; 3] = [-0.32, 0.02, 0.27];

/// The v3 climate table (§8.5). Each temperature band splits on humidity:
///
/// | T band | humidity → biome |
/// |---|---|
/// | cold | SnowyTundra < −0.20 ≤ Taiga |
/// | cool | Plains < −0.13 ≤ Forest < 0.22 ≤ BirchForest |
/// | warm | Savanna < −0.25 ≤ Plains < 0.07 ≤ Forest < 0.40 ≤ Jungle |
/// | hot  | Desert < 0.0 ≤ Savanna < 0.30 ≤ Jungle |
pub fn whittaker(t: f64, hum: f64) -> Biome {
    if t < T_EDGES[0] {
        if hum < -0.20 { Biome::SnowyTundra } else { Biome::Taiga }
    } else if t < T_EDGES[1] {
        if hum < -0.13 {
            Biome::Plains
        } else if hum < 0.22 {
            Biome::Forest
        } else {
            Biome::BirchForest
        }
    } else if t < T_EDGES[2] {
        if hum < -0.25 {
            Biome::Savanna
        } else if hum < 0.07 {
            Biome::Plains
        } else if hum < 0.40 {
            Biome::Forest
        } else {
            Biome::Jungle
        }
    } else if hum < 0.0 {
        Biome::Desert
    } else if hum < 0.30 {
        Biome::Savanna
    } else {
        Biome::Jungle
    }
}

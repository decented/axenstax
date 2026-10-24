//! Surface rules of the v3 terrain (§6). A solid cell is a *surface* cell when
//! the cell above it is not solid; `depth` counts the solid cells between a
//! cell and its local surface (0 = the surface cell itself), so a ledge under
//! an overhang gets its own grass. Below [`MAX_DEPTH`] everything is rock
//! (ore / stone / deepslate, unchanged from v2).

use super::labels;
use super::lattice::{is_wet, Sample};
use super::Terrain;
use crate::biome::Biome;
use crate::block::{self, BlockId};

/// The deepest surface layer any rule paints (sand ×3 over sandstone ×3).
pub const MAX_DEPTH: i32 = 6;
/// §6.1: snow line base and its noise amplitude.
pub const SNOW_LINE: f64 = 165.0;
pub const SNOW_LINE_NOISE: f64 = 8.0;
/// §6.2: a surface this high, or this steep, is bare rock.
pub const ROCK_TOP: i32 = 140;
pub const CLIFF_SLOPE: f64 = 3.0;
/// §6.3: a beach colder than this is gravel.
pub const COLD_BEACH_T: f64 = -0.45;

/// Everything the surface rules need about one column, all pure 2D.
pub struct Column {
    pub s: Sample,
    /// The 2D top (`terrain_height`).
    pub top: i32,
    pub biome: Biome,
    pub slope: f64,
    pub snow_line: f64,
    /// A gravel patch (rock faces, sea floors).
    pub gravel: bool,
    /// Dirt layers under grass (3 or 4).
    pub dirt: i32,
}

impl Column {
    pub fn new(t: &Terrain, x: i32, z: i32, s: Sample) -> Self {
        let top = super::lattice::top_of(s.h);
        Self {
            s,
            top,
            biome: labels::biome(&s, top),
            slope: t.lattice.slope(x, z, s.h),
            snow_line: SNOW_LINE + SNOW_LINE_NOISE * t.detail_noise(x, z, 48.0, 0.0),
            gravel: t.detail_noise(x, z, 12.0, 300.0) > 0.4,
            dirt: 3 + (cell_hash(x, z, t.seed()) & 1) as i32,
        }
    }
}

fn cell_hash(x: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27D4_EB2F) ^ (z as u32).wrapping_mul(0x1656_67B1) ^ seed;
    h = (h ^ (h >> 15)).wrapping_mul(0x85EB_CA77);
    h ^ (h >> 13)
}

/// The block a surface rule paints on a solid cell `depth` (< [`MAX_DEPTH`])
/// below its local surface at `top`, or `None` for rock.
pub fn layer(col: &Column, depth: i32, top: i32) -> Option<BlockId> {
    let sand_over_sandstone = || match depth {
        0..=2 => Some(block::SAND),
        _ => Some(block::SANDSTONE),
    };
    // 1. Snow-capped peaks.
    if top as f64 >= col.snow_line {
        return (depth == 0).then_some(block::SNOW);
    }
    // 2. High or steep: bare rock (with gravel patches).
    if top >= ROCK_TOP || col.slope >= CLIFF_SLOPE {
        return (depth == 0 && col.gravel).then_some(block::GRAVEL);
    }
    // 3. Beaches.
    if labels::beach_band(&col.s, top) {
        if col.s.t < COLD_BEACH_T {
            return (depth <= 2).then_some(block::GRAVEL);
        }
        return sand_over_sandstone();
    }
    // 4. Sea, river and lake floors.
    if is_wet(top) {
        return (depth <= 2).then_some(if col.gravel { block::GRAVEL } else { block::SAND });
    }
    // 5. The biome's own ground.
    match col.biome {
        Biome::Desert | Biome::Beach | Biome::Ocean | Biome::DeepOcean | Biome::River => {
            sand_over_sandstone()
        }
        Biome::SnowyTundra => match depth {
            0 => Some(block::SNOW),
            d if d <= col.dirt => Some(block::DIRT),
            _ => None,
        },
        _ => match depth {
            0 => Some(block::GRASS),
            d if d <= col.dirt => Some(block::DIRT),
            _ => None,
        },
    }
}

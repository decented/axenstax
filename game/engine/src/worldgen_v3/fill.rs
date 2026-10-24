//! The v3 terrain fill: one chunk column at a time ([`column_blocks`]) and the
//! pure per-cell answer ([`block_at`]) that `world::terrain_block_at` gives for
//! a v3 world. Both make every decision through the same functions
//! (`Lattice::sample`, `density::is_solid` with bit-identical noise,
//! `surface::layer`, `solid_block`), so they agree cell for cell.

use super::caves::{self, CaveColumn, CaveNoise, ColumnCaves};
use super::density::{self, ColumnNoise};
use super::surface::{self, Column, MAX_DEPTH};
use super::Terrain;
use crate::biome::{BiomeGenerator, LAVA_CAVE_LEVEL, SEA_LEVEL};
use crate::block::{self, BlockId};

/// World height in blocks (y 0..=255).
pub const HEIGHT: usize = crate::world::WORLD_HEIGHT as usize;

/// What an empty (not solid) cell holds: water up to sea level, else air.
#[inline]
fn fluid(y: i32) -> BlockId {
    if y <= SEA_LEVEL { block::WATER } else { block::AIR }
}

/// A solid cell `depth` (capped at [`MAX_DEPTH`]) solid cells below its local
/// surface.
fn solid_block(bg: &BiomeGenerator, col: &Column, x: i32, y: i32, z: i32, depth: i32) -> BlockId {
    // v3 is frozen with v2's flooded noise caves (`BiomeGenerator::is_cave`,
    // y 2..=54) and v2's lava/water fill; design §7's dry caves are v4's
    // (`solid_block_v4`).
    if y > 1 && y < col.top - 1 && bg.is_cave(x, y, z) {
        return if y <= LAVA_CAVE_LEVEL {
            block::LAVA
        } else if y <= SEA_LEVEL {
            block::WATER
        } else {
            block::AIR
        };
    }
    rock_block(bg, col, x, y, z, depth)
}

/// A solid cell the caves left alone: the surface layer, else ore or rock.
fn rock_block(bg: &BiomeGenerator, col: &Column, x: i32, y: i32, z: i32, depth: i32) -> BlockId {
    if depth < MAX_DEPTH
        && let Some(b) = surface::layer(col, depth, y + depth)
    {
        return b;
    }
    bg.ore_at(x, y, z).unwrap_or_else(|| bg.base_rock_with(x, y, z, col.biome, col.top))
}

/// What the v4 cave pass needs for one column: its 2D facts and the cave
/// noise corners (a column lattice in the fill, the memoised pure lookup in
/// [`block_at`]).
struct V4Caves<'a, N: CaveNoise> {
    column: CaveColumn,
    noise: &'a N,
}

/// [`solid_block`] for v4: the dry caves of §7 (air, or lava low down; never
/// water) instead of v2's flooded noise caves.
fn solid_block_v4<N: CaveNoise>(
    bg: &BiomeGenerator,
    col: &Column,
    cave: &V4Caves<N>,
    (x, y, z): (i32, i32, i32),
    depth: i32,
) -> BlockId {
    if let Some(b) = caves::carve(col, &cave.column, cave.noise, x, y, z) {
        return b;
    }
    rock_block(bg, col, x, y, z, depth)
}

/// The block v3 terrain generation puts at `(x, y, z)` (before trees,
/// vegetation and structures). Pure.
pub fn block_at(bg: &BiomeGenerator, t: &Terrain, x: i32, y: i32, z: i32) -> BlockId {
    if y == 0 {
        return block::BEDROCK;
    }
    if !(0..HEIGHT as i32).contains(&y) {
        return block::AIR;
    }
    let s = t.lattice.sample(x, z);
    let solid = |y: i32| t.density.solid_at(s.h, s.j, x, y, z);
    if !solid(y) {
        return fluid(y);
    }
    let mut depth = 0;
    while depth < MAX_DEPTH && solid(y + depth + 1) {
        depth += 1;
    }
    let col = Column::new(t, x, z, s);
    match t.caves() {
        None => solid_block(bg, &col, x, y, z, depth),
        Some(c) => {
            let cave = V4Caves { column: CaveColumn::new(t, c, x, z, &col), noise: c };
            solid_block_v4(bg, &col, &cave, (x, y, z), depth)
        }
    }
}

/// Every cell of chunk column `(cx, cz)`, indexed `(lz * 16 + lx) * HEIGHT + y`.
pub fn column_blocks(bg: &BiomeGenerator, t: &Terrain, cx: i32, cz: i32) -> Vec<BlockId> {
    let cs = 16;
    let samples: Vec<_> = (0..cs * cs)
        .map(|i| t.lattice.sample(cx * cs + i % cs, cz * cs + i / cs))
        .collect();
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for s in &samples {
        let (l, h) = density::noisy_band(s.h, s.j);
        lo = lo.min(l);
        hi = hi.max(h);
    }
    let noise = ColumnNoise::new(&t.density, cx, cz, lo, hi.min(density::HEIGHT_CAP));
    // v4 only: the cave lattice of this column (v3 builds none).
    let cave_noise = t.caves().map(|c| {
        let cave_top = samples.iter().map(|s| super::lattice::top_of(s.h)).max().unwrap_or(0);
        (c, ColumnCaves::new(c, cx, cz, cave_top))
    });
    let mut out = vec![block::AIR; (cs * cs) as usize * HEIGHT];
    for (i, s) in samples.into_iter().enumerate() {
        let (lx, lz) = (i as i32 % cs, i as i32 / cs);
        let (x, z) = (cx * cs + lx, cz * cs + lz);
        let col = Column::new(t, x, z, s);
        let cave = cave_noise
            .as_ref()
            .map(|(c, lattice)| V4Caves { column: CaveColumn::new(t, c, x, z, &col), noise: lattice });
        let base = i * HEIGHT;
        let top = density::noisy_band(s.h, s.j).1.min(density::HEIGHT_CAP);
        for y in (top + 1).max(1)..HEIGHT as i32 {
            out[base + y as usize] = fluid(y);
        }
        let mut run = 0;
        for y in (1..=top).rev() {
            if density::is_solid(s.h, s.j, y, || noise.n3(x, y, z)) {
                let depth = run.min(MAX_DEPTH);
                out[base + y as usize] = match &cave {
                    None => solid_block(bg, &col, x, y, z, depth),
                    Some(cave) => solid_block_v4(bg, &col, cave, (x, y, z), depth),
                };
                run += 1;
            } else {
                out[base + y as usize] = fluid(y);
                run = 0;
            }
        }
        out[base] = block::BEDROCK;
    }
    out
}

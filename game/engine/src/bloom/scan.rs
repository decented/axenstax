//! Reading a region's columns straight from its chunk sections.
//!
//! One region is one chunk column: sixteen sections stacked in y. The scan
//! fetches the (at most) sixteen sections once and then reads cells directly,
//! so a column costs what it holds, not a hash lookup per cell.

use std::ops::RangeInclusive;

use super::eco::{classify, EcoTally};
use super::kinds::{crop_family, living_kind};
use super::natural::NaturalRegion;
use super::RegionKey;
use crate::block::{self, BlockId, BlockRegistry};
use crate::chunk::{Chunk, CHUNK_SIZE};
use crate::world::{World, MAX_CHUNK_Y};

/// Sections per chunk column.
const SECTIONS: usize = MAX_CHUNK_Y as usize + 1;
/// How far below its top a column is walked for living kinds.
const WALK_LIMIT: i32 = 48;

/// The loaded sections of one region.
pub struct RegionChunks<'a> {
    sections: [Option<&'a Chunk>; SECTIONS],
}

impl<'a> RegionChunks<'a> {
    pub fn of(world: &'a World, key: RegionKey) -> Self {
        let mut sections = [None; SECTIONS];
        for (cy, slot) in sections.iter_mut().enumerate() {
            *slot = world.get_chunk(key.0, cy as i32, key.1).filter(|c| !c.is_empty());
        }
        Self { sections }
    }

    /// Does the region hold any block at all?
    pub fn is_loaded(&self) -> bool {
        self.sections.iter().any(Option::is_some)
    }

    /// Has any section been edited or loaded from a save since worldgen? If not,
    /// the region is exactly what worldgen made.
    pub fn is_touched(&self) -> bool {
        self.sections.iter().flatten().any(|c| c.persist())
    }

    fn block(&self, lx: usize, y: i32, lz: usize) -> BlockId {
        if y < 0 {
            return block::AIR;
        }
        self.sections
            .get(y as usize / CHUNK_SIZE)
            .copied()
            .flatten()
            .map_or(block::AIR, |c| c.get(lx, y as usize % CHUNK_SIZE, lz))
    }

    fn placed(&self, lx: usize, y: i32, lz: usize) -> bool {
        self.sections
            .get(y as usize / CHUNK_SIZE)
            .copied()
            .flatten()
            .is_some_and(|c| c.is_placed(lx, y as usize % CHUNK_SIZE, lz))
    }

    /// The highest non-air cell of column `(lx, lz)`: its height and block.
    pub fn top(&self, lx: usize, lz: usize) -> Option<(i32, BlockId)> {
        for cy in (0..SECTIONS).rev() {
            let Some(c) = self.sections[cy] else { continue };
            for ly in (0..CHUNK_SIZE).rev() {
                let b = c.get(lx, ly, lz);
                if b != block::AIR {
                    return Some(((cy * CHUNK_SIZE + ly) as i32, b));
                }
            }
        }
        None
    }
}

/// Weather covers: not a wound, wherever they lie.
fn is_weather_cover(id: BlockId) -> bool {
    id == block::SNOW || id == block::SNOW_LAYER
}

/// Add the columns `lxs` x `lzs` of a region to `tally`. `natural` is the
/// region as worldgen left it, or `None` when the region is untouched (then
/// every non-placed block is natural).
pub fn scan_columns(
    chunks: &RegionChunks<'_>,
    natural: Option<&NaturalRegion>,
    registry: &BlockRegistry,
    lxs: RangeInclusive<usize>,
    lzs: RangeInclusive<usize>,
    tally: &mut EcoTally,
) {
    for lz in lzs {
        for lx in lxs.clone() {
            let Some((top_y, top)) = chunks.top(lx, lz) else { continue };
            let living = living_kind(top).is_some();
            let placed = chunks.placed(lx, top_y, lz);
            let is_natural = is_weather_cover(top)
                || natural.is_none_or(|n| n.matches(lx, lz, top_y, top));
            let soil = top == block::DIRT || top == block::TILLED_SOIL;
            tally.add_column(classify(living, placed, is_natural, soil));
            if let Some(f) = crop_family(top) {
                tally.add_crop_cell(f);
            }
            walk_kinds(chunks, registry, lx, lz, top_y, tally);
        }
    }
}

/// Walk down from the top through living and non-solid cells to the first solid
/// non-living one, collecting the living kinds on the way: a tree's leaves and
/// trunk both count.
fn walk_kinds(
    chunks: &RegionChunks<'_>,
    registry: &BlockRegistry,
    lx: usize,
    lz: usize,
    top_y: i32,
    tally: &mut EcoTally,
) {
    let floor = (top_y - WALK_LIMIT).max(0);
    let mut y = top_y;
    while y >= floor {
        let b = chunks.block(lx, y, lz);
        match living_kind(b) {
            Some(k) => tally.add_kind(k),
            None if registry.is_solid(b) => break,
            None => {}
        }
        y -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> BlockRegistry {
        BlockRegistry::new()
    }

    #[test]
    fn top_finds_the_highest_block_across_sections() {
        let mut w = World::new();
        w.set_block(3, 5, 4, block::STONE);
        w.set_block(3, 40, 4, block::DIRT);
        let rc = RegionChunks::of(&w, (0, 0));
        assert!(rc.is_loaded());
        assert_eq!(rc.top(3, 4), Some((40, block::DIRT)));
        assert_eq!(rc.top(0, 0), None);
        assert!(!RegionChunks::of(&w, (5, 5)).is_loaded());
    }

    #[test]
    fn a_walk_collects_leaves_and_trunk_but_stops_at_ground() {
        let mut w = World::new();
        w.set_block(1, 10, 1, block::STONE);
        w.set_block(1, 11, 1, block::OAK_LOG);
        w.set_block(1, 12, 1, block::OAK_LOG);
        w.set_block(1, 13, 1, block::BIRCH_LEAVES);
        // Below the stone is more life that the walk must not reach.
        w.set_block(1, 5, 1, block::WHEAT_STAGE_0);
        let rc = RegionChunks::of(&w, (0, 0));
        let mut t = EcoTally::default();
        scan_columns(&rc, None, &reg(), 1..=1, 1..=1, &mut t);
        assert_eq!(t.kinds.count_ones(), 2, "oak log, birch leaf: {:b}", t.kinds);
        assert_eq!(t.columns, 1);
    }

    #[test]
    fn a_placed_non_living_top_is_built_and_skipped_in_the_average() {
        let mut w = World::new();
        w.place_player_block(2, 20, 2, block::COBBLESTONE);
        w.set_block(3, 20, 2, block::STONE);
        let rc = RegionChunks::of(&w, (0, 0));
        let mut t = EcoTally::default();
        scan_columns(&rc, None, &reg(), 2..=3, 2..=2, &mut t);
        assert_eq!((t.columns, t.built), (2, 1));
    }

    #[test]
    fn a_planted_crop_is_living_even_though_it_was_placed() {
        let mut w = World::new();
        w.place_player_block(2, 20, 2, block::WHEAT_STAGE_0);
        let rc = RegionChunks::of(&w, (0, 0));
        let mut t = EcoTally::default();
        scan_columns(&rc, None, &reg(), 2..=2, 2..=2, &mut t);
        assert_eq!((t.built, t.crop_cells[0]), (0, 1));
        assert_eq!(t.value_sum, 1.0);
    }
}

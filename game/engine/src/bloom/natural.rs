//! What worldgen left in a region: the "natural top" of every column.
//!
//! A column is wounded when its top block is not alive, not placed by a player,
//! and not what the ground naturally was. So a natural ravine, a cliff or a
//! rocky bank never reads grey, while a stripped quarry does.
//!
//! Method: the region's column is regenerated into a throwaway twin of the
//! world (`World::generation_twin`, the same trick `chunk_verdict` uses) and
//! its top blocks are kept: 256 heights and blocks, about 1.3 KB per region.
//! That is exact for every generator (trees, villages, ravines, flat floors
//! included). It is needed only for a *touched* region, one with a section
//! edited or loaded from a save; a region fresh from worldgen is its own natural
//! top. It is cached per region for the session and generated at most once per
//! tick (see `field.rs`).

use std::collections::HashMap;

use super::scan::RegionChunks;
use super::RegionKey;
use crate::biome::BiomeGenerator;
use crate::block::BlockId;
use crate::chunk::CHUNK_SIZE;
use crate::world::World;

const COLUMNS: usize = CHUNK_SIZE * CHUNK_SIZE;

/// The natural top of every column of one region.
pub struct NaturalRegion {
    /// Height of the top block, or -1 for an empty column. Index `lz * 16 + lx`.
    top_y: [i16; COLUMNS],
    block: [BlockId; COLUMNS],
}

impl NaturalRegion {
    /// Is `block` at height `y` what worldgen left on top of column `(lx, lz)`?
    pub fn matches(&self, lx: usize, lz: usize, y: i32, block: BlockId) -> bool {
        let i = lz * CHUNK_SIZE + lx;
        i32::from(self.top_y[i]) == y && self.block[i] == block
    }

    /// Regenerate region `key` into a twin of `world` and keep its tops.
    pub fn generate(world: &World, biome_gen: &BiomeGenerator, key: RegionKey) -> Self {
        let mut twin = world.generation_twin();
        twin.generate_column(key.0, key.1, biome_gen);
        let chunks = RegionChunks::of(&twin, key);
        let mut out = Self { top_y: [-1; COLUMNS], block: [0; COLUMNS] };
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                if let Some((y, b)) = chunks.top(lx, lz) {
                    out.top_y[lz * CHUNK_SIZE + lx] = y as i16;
                    out.block[lz * CHUNK_SIZE + lx] = b;
                }
            }
        }
        out
    }
}

/// Natural tops per region, for the session.
#[derive(Default)]
pub struct NaturalCache {
    map: HashMap<RegionKey, Box<NaturalRegion>>,
}

impl NaturalCache {
    pub fn get(&self, key: RegionKey) -> Option<&NaturalRegion> {
        self.map.get(&key).map(|b| &**b)
    }

    pub fn contains(&self, key: RegionKey) -> bool {
        self.map.contains_key(&key)
    }

    /// Generate and keep region `key`'s natural top.
    pub fn ensure(&mut self, world: &World, biome_gen: &BiomeGenerator, key: RegionKey) {
        self.map
            .entry(key)
            .or_insert_with(|| Box::new(NaturalRegion::generate(world, biome_gen, key)));
    }

    /// Forget regions for which `keep` is false.
    pub fn retain(&mut self, mut keep: impl FnMut(RegionKey) -> bool) {
        self.map.retain(|k, _| keep(*k));
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

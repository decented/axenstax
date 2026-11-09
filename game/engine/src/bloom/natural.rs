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
//! top. It is cached per region for the session and generated at most once
//! every 4 ticks (see `field.rs`; a fully touched field fills in about a
//! minute). A client that cannot regenerate the terrain its host sent (a
//! push-only joiner, whose worldgen differs) never builds one and reads every
//! non-placed block as natural.

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

/// Natural tops per region, for the session, least recently used dropped
/// past [`NATURAL_CACHE_MAX`].
///
/// Until the v0.2.31 playtest the cache was trimmed to the tracked regions,
/// so a Creative player flying back and forth re-ran worldgen for every
/// touched region each pass, on the main thread (and "touched" grows with play:
/// any edit outside worldgen, grass spread and leaf decay included, and every
/// chunk loaded from a save). Kept now, it costs at most about 2.7 MB.
#[derive(Default)]
pub struct NaturalCache {
    map: HashMap<RegionKey, (Box<NaturalRegion>, u64)>,
}

/// Regions kept in a [`NaturalCache`] (about 1.3 KB each).
pub const NATURAL_CACHE_MAX: usize = 2048;

/// The cap for a field tracking `tracked` regions: never below
/// [`NATURAL_CACHE_MAX`], and always with room for every tracked region and a
/// quarter more. A dedicated server with many players far apart tracks more
/// than 2,048 regions at once; trimming under that would drop regions still in
/// use and regenerate them (a worldgen column each) for ever.
pub fn cache_cap(tracked: usize) -> usize {
    NATURAL_CACHE_MAX.max(tracked * 5 / 4)
}

impl NaturalCache {
    pub fn get(&self, key: RegionKey) -> Option<&NaturalRegion> {
        self.map.get(&key).map(|(b, _)| &**b)
    }

    pub fn contains(&self, key: RegionKey) -> bool {
        self.map.contains_key(&key)
    }

    /// Generate and keep region `key`'s natural top, stamped as used at `now`.
    pub fn ensure(&mut self, world: &World, biome_gen: &BiomeGenerator, key: RegionKey, now: u64) {
        let entry = self
            .map
            .entry(key)
            .or_insert_with(|| (Box::new(NaturalRegion::generate(world, biome_gen, key)), now));
        entry.1 = now;
    }

    /// Stamp region `key` as used at `now` (no-op when absent).
    pub fn touch(&mut self, key: RegionKey, now: u64) {
        if let Some(e) = self.map.get_mut(&key) {
            e.1 = now;
        }
    }

    /// Past `max` regions, drop the least recently used down to `max * 7 / 8`
    /// (so the sort is paid rarely, not on every new region).
    pub fn trim(&mut self, max: usize) {
        if self.map.len() <= max {
            return;
        }
        let keep = max * 7 / 8;
        let mut stamps: Vec<u64> = self.map.values().map(|(_, t)| *t).collect();
        stamps.sort_unstable();
        let cut = stamps[self.map.len() - keep];
        self.map.retain(|_, (_, t)| *t >= cut);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Box<NaturalRegion> {
        Box::new(NaturalRegion { top_y: [-1; COLUMNS], block: [0; COLUMNS] })
    }

    #[test]
    fn the_cache_keeps_recently_used_regions_and_drops_the_oldest_past_its_cap() {
        let mut c = NaturalCache::default();
        for i in 0..20i32 {
            c.map.insert((i, 0), (blank(), i as u64));
        }
        // Region 0 is old by insertion but used just now: it stays.
        c.touch((0, 0), 100);
        c.trim(16);
        assert_eq!(c.len(), 14, "trimmed to 7/8 of the cap");
        assert!(c.contains((0, 0)), "recently used survives");
        assert!(!c.contains((1, 0)) && !c.contains((6, 0)), "the least recently used go");
        assert!(c.contains((19, 0)));
        // Under the cap nothing moves.
        c.trim(16);
        assert_eq!(c.len(), 14);
    }

    #[test]
    fn the_cap_always_has_room_for_every_tracked_region() {
        assert_eq!(cache_cap(289), NATURAL_CACHE_MAX, "one player: the usual cap");
        assert_eq!(cache_cap(2048), 2560);
        // Eight players far apart track 2,312 regions: more than the old cap.
        assert!(cache_cap(8 * 289) >= 8 * 289 * 5 / 4);
    }
}

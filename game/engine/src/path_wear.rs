//! Living world Tier 2 S3 — Worn Paths (spec `docs/superpowers/specs/2026-11-09-living-world-tier2.md` §3, call T7).
//!
//! Grass walked over often becomes a [`block::WORN_PATH`]. A **footfall** is a
//! body on the ground, standing on a GRASS block, entering a cell it was not in
//! on its last grounded sample. A cell that takes [`FOOTFALLS`] footfalls turns
//! to a Worn Path. Standing still makes no footfall; jumping on the spot makes
//! none either (a body's cell only changes on a grounded sample, and an
//! airborne sample changes nothing).
//!
//! * **Runtime only.** The partial counts live in [`PathWear`], a `World`
//!   field that is never saved (the block itself persists). It is bounded at
//!   [`MAX_ENTRIES`] cells; the least recently stepped one is forgotten first.
//! * **Not in creative** (a creative lawn round a build must not wear away).
//! * **One rule, every path.** [`tick_wear`] is called by the authority's
//!   `SimSystem::UseRecord` phase (a dedicated server, a lending host, joiners'
//!   bodies and the host's own seats alike) and by the standalone seat loop. It
//!   writes the world and returns the `BlockChange`s the authority broadcasts.
//! * **Grounded without `on_ground`.** A lending host's own seats are
//!   position-trusted and never report `on_ground`, so a body counts as
//!   standing when its feet rest on the top face of the block under it
//!   ([`ground_cell`]), and a caller that cannot say passes `on_ground: true`.
//!
//! A Worn Path is not "placed" (it carries no author), breaks like dirt, and
//! never turns back to grass.

use std::collections::HashMap;

use glam::Vec3;

use crate::block;
use crate::protocol::BlockChange;
use crate::world::World;

/// Footfalls that wear a grass cell into a path.
pub const FOOTFALLS: u16 = 24;
/// Most cells with a partial count; past it the least recently stepped is forgotten.
pub const MAX_ENTRIES: usize = 16_384;

/// A cell, `[x, y, z]`.
pub type Cell = [i32; 3];

/// A body to sample this tick.
#[derive(Clone, Copy, Debug)]
pub struct Foot {
    /// Who it is on this machine: the seat index (standalone), or the player
    /// index on the authority. Only used to remember the last cell it stood in.
    pub key: u32,
    /// Its foot position (bottom centre of the hitbox).
    pub pos: Vec3,
    /// The body's own word that it is not in the air. The caller passes `true`
    /// for a position-trusted body, which reports none.
    pub on_ground: bool,
}

/// The block cell a body whose feet are at `pos` is standing on, if its feet
/// rest on that block's top face (within a tenth of a block: a sunk step or a
/// body a hair above). `None` in the air.
pub fn ground_cell(pos: Vec3) -> Option<Cell> {
    let below = (pos.y - 0.05).floor() as i32;
    if pos.y - (below + 1) as f32 <= -0.1 {
        return None;
    }
    Some([pos.x.floor() as i32, below, pos.z.floor() as i32])
}

/// Partial footfall counts and where each body last stood.
#[derive(Clone, Debug, Default)]
pub struct PathWear {
    /// Cell → (footfalls so far, tick of the latest one).
    counts: HashMap<Cell, (u16, u64)>,
    /// Body key → the ground cell of its latest grounded sample.
    last: HashMap<u32, Cell>,
    /// Footfalls recorded so far: orders the cells by how recently they were stepped.
    seq: u64,
}

impl PathWear {
    /// Forget everything (a world reset).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Cells with a partial count.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// Nothing counted.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// The footfalls so far on `cell`.
    #[cfg(test)]
    pub fn count(&self, cell: Cell) -> u16 {
        self.counts.get(&cell).map_or(0, |c| c.0)
    }

    /// Sample `feet` against `world`: the cells that reached [`FOOTFALLS`]
    /// this sample, in body order (each at most once). Does not write the world.
    pub fn step(&mut self, world: &World, feet: &[Foot], creative: bool) -> Vec<Cell> {
        let mut worn = Vec::new();
        if creative {
            return worn;
        }
        for f in feet {
            if !f.on_ground {
                continue;
            }
            let Some(cell) = ground_cell(f.pos) else { continue };
            if self.last.insert(f.key, cell) == Some(cell) {
                continue;
            }
            if world.get_block(cell[0], cell[1], cell[2]) != block::GRASS {
                continue;
            }
            self.seq += 1;
            let entry = self.counts.entry(cell).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = self.seq;
            if entry.0 >= FOOTFALLS {
                self.counts.remove(&cell);
                if !worn.contains(&cell) {
                    worn.push(cell);
                }
            }
        }
        self.bound();
        worn
    }

    /// Keep the map within [`MAX_ENTRIES`]: drop the least recently stepped
    /// eighth in one go (so the sort is amortised).
    fn bound(&mut self) {
        if self.counts.len() <= MAX_ENTRIES {
            return;
        }
        let mut by_age: Vec<(u64, Cell)> = self.counts.iter().map(|(c, v)| (v.1, *c)).collect();
        by_age.sort_unstable();
        let drop = self.counts.len() - (MAX_ENTRIES - MAX_ENTRIES / 8);
        for (_, cell) in by_age.into_iter().take(drop) {
            self.counts.remove(&cell);
        }
    }
}

/// Wear the world's grass for `feet` this tick and return the changes made
/// (`GRASS → WORN_PATH`), for the authority to broadcast. A cell that is no
/// longer grass by now is left alone. The cell is not "placed" afterwards, so
/// a player-laid lawn that wears through stops being anyone's.
pub fn tick_wear(world: &mut World, feet: &[Foot], creative: bool) -> Vec<BlockChange> {
    if creative || feet.is_empty() {
        return Vec::new();
    }
    // The map lives on the world; take it out so the sample can read the world.
    let mut wear = std::mem::take(&mut world.path_wear);
    let worn = wear.step(world, feet, creative);
    world.path_wear = wear;
    let mut changes = Vec::with_capacity(worn.len());
    for [x, y, z] in worn {
        if world.get_block(x, y, z) != block::GRASS {
            continue;
        }
        world.set_block(x, y, z, block::WORN_PATH);
        if world.is_placed(x, y, z) {
            world.set_placed(x, y, z, false);
        }
        changes.push(BlockChange::with_meta(x, y, z, block::WORN_PATH, 0));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lawn() -> World {
        let mut w = World::new();
        for x in -4..8 {
            for z in -4..8 {
                w.set_block(x, 39, z, block::GRASS);
            }
        }
        w
    }

    fn at(key: u32, x: f32, z: f32) -> Foot {
        Foot { key, pos: Vec3::new(x, 40.0, z), on_ground: true }
    }

    /// Walk a body between two cells `n` times, returning the first sample
    /// index at which cell (1, 39, 1) turned to a path.
    fn pace(w: &mut World, n: usize) -> Option<usize> {
        for i in 0..n {
            // Alternate between the target cell and its neighbour: each
            // sample enters a cell the body was not in.
            let f = if i % 2 == 0 { at(0, 1.5, 1.5) } else { at(0, 2.5, 1.5) };
            let changes = tick_wear(w, &[f], false);
            if changes.iter().any(|c| (c.x, c.y, c.z) == (1, 39, 1)) {
                return Some(i);
            }
        }
        None
    }

    #[test]
    fn twenty_three_footfalls_leave_grass_and_the_twenty_fourth_makes_a_path() {
        let mut w = lawn();
        // Footfalls on (1,39,1) are the even samples: 1st at i = 0, 24th at i = 46.
        for i in 0..46 {
            let f = if i % 2 == 0 { at(0, 1.5, 1.5) } else { at(0, 2.5, 1.5) };
            assert!(tick_wear(&mut w, &[f], false).iter().all(|c| (c.x, c.y, c.z) != (1, 39, 1)));
        }
        assert_eq!(w.get_block(1, 39, 1), block::GRASS, "23 footfalls");
        assert_eq!(w.path_wear.count([1, 39, 1]), 23);
        let changes = tick_wear(&mut w, &[at(0, 1.5, 1.5)], false);
        assert_eq!(w.get_block(1, 39, 1), block::WORN_PATH, "the 24th");
        assert_eq!(changes, vec![BlockChange::with_meta(1, 39, 1, block::WORN_PATH, 0)]);
        assert_eq!(w.path_wear.count([1, 39, 1]), 0, "a finished cell keeps no count");
        assert!(pace(&mut w, 4).is_none(), "an already worn cell is not worn again");
        assert!(!w.is_placed(1, 39, 1), "a path is nobody's");
    }

    #[test]
    fn standing_still_or_jumping_on_the_spot_makes_no_footfall() {
        let mut w = lawn();
        for _ in 0..200 {
            tick_wear(&mut w, &[at(0, 1.5, 1.5)], false);
        }
        assert_eq!(w.path_wear.count([1, 39, 1]), 1, "the first step onto the cell, then nothing");
        // Hop: feet 0.6 above the floor are airborne; landing on the same cell is no new cell.
        for _ in 0..200 {
            let up = Foot { key: 0, pos: Vec3::new(1.5, 40.6, 1.5), on_ground: false };
            tick_wear(&mut w, &[up], false);
            tick_wear(&mut w, &[at(0, 1.5, 1.5)], false);
        }
        assert_eq!(w.path_wear.count([1, 39, 1]), 1);
        assert_eq!(w.get_block(1, 39, 1), block::GRASS);
    }

    #[test]
    fn creative_wears_nothing_and_neither_does_the_air() {
        let mut w = lawn();
        for i in 0..200 {
            let f = if i % 2 == 0 { at(0, 1.5, 1.5) } else { at(0, 2.5, 1.5) };
            assert!(tick_wear(&mut w, &[f], true).is_empty());
        }
        assert!(w.path_wear.is_empty());
        // Falling past the grass: the flag says airborne, and the feet are off the face either way.
        for i in 0..200 {
            let x = if i % 2 == 0 { 1.5 } else { 2.5 };
            let falling = Foot { key: 0, pos: Vec3::new(x, 41.3, 1.5), on_ground: false };
            let flagged_but_high = Foot { key: 0, pos: Vec3::new(x, 41.3, 1.5), on_ground: true };
            assert!(tick_wear(&mut w, &[falling, flagged_but_high], false).is_empty());
        }
        assert!(w.path_wear.is_empty());
    }

    #[test]
    fn only_grass_wears_and_a_stone_step_between_is_not_a_footfall_on_grass() {
        let mut w = lawn();
        w.set_block(1, 39, 1, block::STONE);
        w.set_block(5, 39, 5, block::DIRT);
        for i in 0..100 {
            let f = if i % 2 == 0 { at(0, 1.5, 1.5) } else { at(0, 5.5, 5.5) };
            tick_wear(&mut w, &[f], false);
        }
        assert_eq!(w.get_block(1, 39, 1), block::STONE);
        assert_eq!(w.get_block(5, 39, 5), block::DIRT);
        assert!(w.path_wear.is_empty());
    }

    #[test]
    fn each_body_has_its_own_last_cell() {
        let mut w = lawn();
        // Two bodies on the same two cells, swapping each sample: both enter a new cell every time.
        for i in 0..4 {
            let (a, b) = if i % 2 == 0 { (1.5, 2.5) } else { (2.5, 1.5) };
            tick_wear(&mut w, &[at(0, a, 1.5), at(1, b, 1.5)], false);
        }
        assert_eq!(w.path_wear.count([1, 39, 1]), 4);
        assert_eq!(w.path_wear.count([2, 39, 1]), 4);
    }

    #[test]
    fn the_map_is_bounded_and_forgets_the_least_recently_stepped() {
        let mut wear = PathWear::default();
        let mut w = World::new();
        let n = MAX_ENTRIES as i32 + 50;
        for x in 0..n {
            w.set_block(x, 39, 0, block::GRASS);
        }
        // One body, one new grass cell per sample.
        for x in 0..n {
            wear.step(&w, &[Foot { key: 0, pos: Vec3::new(x as f32 + 0.5, 40.0, 0.5), on_ground: true }], false);
            assert!(wear.len() <= MAX_ENTRIES, "{}", wear.len());
        }
        assert!(wear.len() > MAX_ENTRIES / 2, "it trims an eighth, not everything");
        assert_eq!(wear.count([0, 39, 0]), 0, "the oldest is gone");
        assert_eq!(wear.count([n - 1, 39, 0]), 1, "the newest is kept");
    }

    #[test]
    fn ground_cell_is_the_top_face_under_the_feet() {
        assert_eq!(ground_cell(Vec3::new(1.5, 40.0, 1.5)), Some([1, 39, 1]));
        assert_eq!(ground_cell(Vec3::new(1.5, 39.97, -0.5)), Some([1, 39, -1]), "sunk a hair");
        assert_eq!(ground_cell(Vec3::new(1.5, 40.04, 1.5)), Some([1, 39, 1]), "a hair above");
        assert_eq!(ground_cell(Vec3::new(1.5, 40.5, 1.5)), None, "half a block up is the air");
        assert_eq!(ground_cell(Vec3::new(1.5, 41.0, 1.5)), Some([1, 40, 1]), "the next block's top face");
    }

    #[test]
    fn a_player_laid_lawn_stops_being_anyones_when_it_wears_through() {
        let mut w = lawn();
        w.place_player_block(1, 39, 1, block::GRASS);
        assert!(w.is_placed(1, 39, 1));
        for i in 0..48 {
            let f = if i % 2 == 0 { at(0, 1.5, 1.5) } else { at(0, 2.5, 1.5) };
            tick_wear(&mut w, &[f], false);
        }
        assert_eq!(w.get_block(1, 39, 1), block::WORN_PATH);
        assert!(!w.is_placed(1, 39, 1));
    }
}

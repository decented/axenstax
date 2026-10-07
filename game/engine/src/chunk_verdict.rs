//! Touched columns (Phase B2b; Spec 04 §4.1 "Touched columns").
//!
//! A joiner whose terrain generator matches the host's
//! (`world::worldgen_fingerprint`) can build a column itself when the column
//! is still exactly what generation makes from the world's seed and flags.
//! The server then sends a "this column is local" note
//! (`protocol::ColumnLocalPacket`) instead of pushing six chunks
//! (`chunk_push`). This module decides which columns qualify.
//!
//! - **The verdict is per column** ([`column_verdict`]). A column is
//!   [`Verdict::Touched`] if any of its chunks differs from a scratch
//!   regeneration of that one column. The comparison covers blocks, the
//!   player-placed bits, and, on the block cells, `block_meta`, block entities
//!   (type, position and contents: a looted worldgen chest is touched) and
//!   face attachments. Light is excluded. The scratch is a fresh `World`
//!   with the server world's generation inputs (`World::generation_twin`:
//!   the Workshop void, the world type, the flat floor, its water depth) and
//!   the server's own `BiomeGenerator` (the seed). Generation is
//!   order-independent and clipped to its column (Phase B0, `world.rs`
//!   `WORLDGEN_VERSION`), so a fresh column matches. If a feature ever spills
//!   across columns, the scratch differs and the column reads as a false
//!   `Touched`. That is safe: it costs bandwidth, never a wrong world.
//! - **The cache is shared and monotonic** ([`Verdicts`]). Every joiner
//!   reads the same verdicts. A non-worldgen write to a column marks it
//!   `Touched` for good. The World records the columns it edits
//!   (`World::track_edited_columns`, hooked where its setters change what a
//!   joiner would see: blocks, metadata, placed bits, block entities including
//!   any `&mut` handed out, and face attachments). The hosted server drains
//!   that set into the cache every tick ([`Verdicts::touch`]). A column loaded
//!   from a save starts with no verdict and is compared once. A save restore
//!   is not an edit (`World::without_edit_tracking`). A verdict survives the
//!   column's unload, because its saved content cannot change while it is
//!   unloaded (a write-through to an evicted column still marks it). Verdicts
//!   are not persisted across restarts.
//! - **The budget** ([`VERDICT_BUDGET`]). Verdicts are computed lazily on
//!   the server tick, nearest first round each joiner's server body, at most
//!   [`VERDICTS_PER_TICK`] a tick. The server stops starting new ones once
//!   [`VERDICT_TIME_PER_TICK`] has gone, but always does at least one. It
//!   runs inside a lending host's own frame, so the budget stays small. A
//!   column with no verdict yet is neither pushed nor declared local.
//!
//! Untouched columns come from the joiner's own generation, so the anti-X-ray
//! obfuscation that `chunk_push::build_chunk_packets` could apply to a push
//! (`anti_xray.rs`, not wired) can never reach them. The seed still ships.
// Reached only through a hosted server with remote players, which the web
// build never has.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::time::Duration;

use crate::biome::BiomeGenerator;
use crate::chunk::Chunk;
use crate::chunk_push::block_entries_in;
use crate::world::{World, MAX_CHUNK_Y};

/// Is a column still what generation makes?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every chunk is exactly the scratch regeneration: the joiner generates it.
    Untouched,
    /// Something differs (or did once this session): the server pushes it.
    Touched,
}

/// Most verdicts a server computes in one tick (all joiners together).
pub const VERDICTS_PER_TICK: usize = 4;

/// After this much of a tick has gone on verdicts, no new one starts. At
/// least one always runs, so a slow machine still makes progress.
pub const VERDICT_TIME_PER_TICK: Duration = Duration::from_millis(3);

/// A per-tick verdict budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerdictBudget {
    /// Most verdicts computed in one call.
    pub count: usize,
    /// No new verdict starts once this much time has gone (the first always runs).
    pub time: Duration,
}

/// The server's per-tick budget: [`VERDICTS_PER_TICK`] and [`VERDICT_TIME_PER_TICK`].
pub const VERDICT_BUDGET: VerdictBudget =
    VerdictBudget { count: VERDICTS_PER_TICK, time: VERDICT_TIME_PER_TICK };

/// Compare column `col` of `world` against a scratch regeneration of it (see
/// the module docs). `biome_gen` must be the generator `world` was generated
/// with.
pub fn column_verdict(world: &World, biome_gen: &BiomeGenerator, col: (i32, i32)) -> Verdict {
    let mut scratch = world.generation_twin();
    scratch.generate_column(col.0, col.1, biome_gen);
    if column_matches(world, &scratch, col) { Verdict::Untouched } else { Verdict::Touched }
}

/// Does column `col` of `live` hold what it holds in `pristine`: the same
/// blocks and placed bits, and on the block cells the same metadata, block
/// entities (type and contents) and face attachments? An absent chunk equals
/// one that is all air with no placed bit.
pub(crate) fn column_matches(live: &World, pristine: &World, col: (i32, i32)) -> bool {
    let (cx, cz) = col;
    for cy in 0..=MAX_CHUNK_Y {
        let same = match (live.get_chunk(cx, cy, cz), pristine.get_chunk(cx, cy, cz)) {
            (None, None) => true,
            (Some(c), None) | (None, Some(c)) => c.is_bare(),
            (Some(a), Some(b)) => a.same_content(b),
        };
        if !same {
            return false;
        }
    }
    // The blocks match, so both hold the same block cells: compare what sits
    // on them. (Side data on an air cell belongs to no block and is never
    // pushed either.)
    for cy in 0..=MAX_CHUNK_Y {
        let coord = (cx, cy, cz);
        let (Some(a), Some(b)) = (live.get_chunk(cx, cy, cz), pristine.get_chunk(cx, cy, cz)) else {
            continue;
        };
        if a.is_empty() {
            continue;
        }
        if side_meta(live, a, coord) != side_meta(pristine, b, coord) {
            return false;
        }
        match (side_entities(live, a, coord), side_entities(pristine, b, coord)) {
            (Some(x), Some(y)) if x == y => {}
            _ => return false,
        }
        if block_entries_in(&live.face_attachments, coord, a)
            != block_entries_in(&pristine.face_attachments, coord, b)
        {
            return false;
        }
    }
    true
}

/// The chunk's non-zero metadata on its block cells.
fn side_meta(world: &World, chunk: &Chunk, coord: (i32, i32, i32)) -> Vec<(u16, u8)> {
    block_entries_in(&world.block_meta, coord, chunk)
        .into_iter()
        .filter(|(_, m)| **m != 0)
        .map(|(i, m)| (i, *m))
        .collect()
}

/// The chunk's block entities on its block cells, each as its serialized
/// bytes (variant and contents; `BlockEntityData` has no `PartialEq`).
/// `None` if one does not serialize (then nothing is assumed equal).
fn side_entities(world: &World, chunk: &Chunk, coord: (i32, i32, i32)) -> Option<Vec<(u16, Vec<u8>)>> {
    block_entries_in(&world.block_entities, coord, chunk)
        .into_iter()
        .map(|(i, e)| bincode::serialize(e).ok().map(|bytes| (i, bytes)))
        .collect()
}

/// The server's shared verdict cache. See the module docs.
#[derive(Debug, Default)]
pub struct Verdicts {
    map: ahash::AHashMap<(i32, i32), Verdict>,
}

impl Verdicts {
    /// The verdict on column `col`, if it has one.
    pub fn get(&self, col: (i32, i32)) -> Option<Verdict> {
        self.map.get(&col).copied()
    }

    /// Column `col` was edited: touched for good.
    pub fn touch(&mut self, col: (i32, i32)) {
        self.map.insert(col, Verdict::Touched);
    }

    /// Decide the columns in `candidates` (nearest first) that have no
    /// verdict yet, in order, within `budget`. Returns how many it decided.
    pub fn decide(
        &mut self,
        world: &World,
        biome_gen: &BiomeGenerator,
        candidates: &[(i32, i32)],
        budget: VerdictBudget,
    ) -> usize {
        let start = web_time::Instant::now();
        let mut decided = 0;
        for &col in candidates {
            if decided >= budget.count || (decided > 0 && start.elapsed() >= budget.time) {
                break;
            }
            if self.map.contains_key(&col) {
                continue;
            }
            self.map.insert(col, column_verdict(world, biome_gen, col));
            decided += 1;
        }
        decided
    }

    /// Set a verdict outright. Test-only.
    #[cfg(test)]
    pub fn set_for_test(&mut self, col: (i32, i32), verdict: Verdict) {
        self.map.insert(col, verdict);
    }

    /// How many columns have a verdict. Test-only.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// How many verdicts are `Touched`. Test-only.
    #[cfg(test)]
    pub fn touched(&self) -> usize {
        self.map.values().filter(|&&v| v == Verdict::Touched).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::chunk::CHUNK_SIZE;

    const SEED: u32 = 42;

    /// A world with the `r`-ring round `centre` generated, nearest first.
    fn generated(biome: &BiomeGenerator, centre: (i32, i32), r: i32) -> World {
        let mut world = World::new();
        let mut cols: Vec<(i32, i32)> = Vec::new();
        for dx in -r..=r {
            for dz in -r..=r {
                cols.push((centre.0 + dx, centre.1 + dz));
            }
        }
        cols.sort_by_key(|&(x, z)| ((x - centre.0).pow(2) + (z - centre.1).pow(2), x, z));
        for (x, z) in cols {
            world.generate_column(x, z, biome);
        }
        world
    }

    /// A column holding a mineshaft's worldgen chest, and the chest's cell.
    fn mineshaft_chest(biome: &BiomeGenerator) -> ((i32, i32), (i32, i32, i32)) {
        for g in 0..64 {
            let (gx, gz) = (g % 8 - 4, g / 8 - 4);
            if let Some(layout) =
                crate::mineshaft_gen::layout_for_mineshaft_cell(SEED, gx, gz, biome)
                && let Some(cell) = crate::mineshaft_gen::chest_site(&layout, 0)
            {
                let cs = CHUNK_SIZE as i32;
                return ((cell.0.div_euclid(cs), cell.2.div_euclid(cs)), cell);
            }
        }
        panic!("no mineshaft near the origin for seed {SEED}");
    }

    #[test]
    fn a_fresh_column_is_untouched_whatever_its_neighbours() {
        let biome = BiomeGenerator::new(SEED);
        let world = generated(&biome, (0, 0), 2);
        for dx in -1..=1 {
            for dz in -1..=1 {
                assert_eq!(column_verdict(&world, &biome, (dx, dz)), Verdict::Untouched, "({dx}, {dz})");
            }
        }
    }

    #[test]
    fn one_edit_makes_a_column_touched_and_is_tracked() {
        let biome = BiomeGenerator::new(SEED);
        let mut world = generated(&biome, (0, 0), 1);
        world.track_edited_columns();
        // World-gen writes are never edits.
        world.generate_column(5, 5, &biome);
        assert!(world.take_edited_columns().is_empty(), "generation is no edit");
        world.set_block(3, 90, 3, block::GLASS);
        assert_eq!(column_verdict(&world, &biome, (0, 0)), Verdict::Touched);
        assert_eq!(world.take_edited_columns(), vec![(0, 0)]);
        // Metadata alone is an edit too, and so is a placed bit.
        world.set_meta((20, 40, 4), 3);
        world.set_placed(-3, 40, 4, true);
        let mut cols = world.take_edited_columns();
        cols.sort_unstable();
        assert_eq!(cols, vec![(-1, 0), (1, 0)]);
        assert_eq!(column_verdict(&world, &biome, (1, 0)), Verdict::Touched, "meta alone");
        assert_eq!(column_verdict(&world, &biome, (-1, 0)), Verdict::Touched, "a placed bit alone");
    }

    #[test]
    fn a_looted_worldgen_chest_is_touched() {
        let biome = BiomeGenerator::new(SEED);
        let (col, cell) = mineshaft_chest(&biome);
        let mut world = generated(&biome, col, 1);
        assert!(world.chest_at(cell).is_some_and(|c| c.slots.iter().any(Option::is_some)), "a loot chest");
        assert_eq!(column_verdict(&world, &biome, col), Verdict::Untouched, "as generated");
        world.track_edited_columns();
        // Take the loot: the block stays a chest, only its contents change.
        let chest = world.chest_at_mut(cell).expect("chest");
        for slot in &mut chest.slots {
            *slot = None;
        }
        assert_eq!(world.take_edited_columns(), vec![col], "handing out the chest counts as an edit");
        assert_eq!(column_verdict(&world, &biome, col), Verdict::Touched, "its contents are compared");
    }

    #[test]
    fn an_unedited_column_saved_and_loaded_is_untouched() {
        let _guard = crate::save::WorldsRootGuard::new("verdict-save-load");
        let biome = BiomeGenerator::new(SEED);
        let (col, cell) = mineshaft_chest(&biome);
        let world = generated(&biome, col, 1);
        let name = format!("verdict-save-load-{}", std::process::id());
        let player = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(0.0, 90.0, 0.0), 1.0);
        crate::save::save_world(&name, &world, &[player], SEED, &[], &[]).expect("saved");
        let mut loaded = World::new();
        loaded.track_edited_columns();
        crate::save::load_world(&name, &mut loaded).expect("loaded");
        assert!(loaded.chest_at(cell).is_some(), "the chest came back");
        assert!(loaded.take_edited_columns().is_empty(), "a restore is no edit");
        assert_eq!(column_verdict(&loaded, &biome, col), Verdict::Untouched);
        let _ = crate::save::delete_world(&name);
    }

    #[test]
    fn decide_never_passes_its_count_and_skips_what_it_knows() {
        let biome = BiomeGenerator::new(SEED);
        let world = generated(&biome, (0, 0), 2);
        let mut v = Verdicts::default();
        v.touch((0, 0));
        let cands = [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1)];
        let budget = VerdictBudget { count: 2, time: Duration::from_secs(60) };
        assert_eq!(v.decide(&world, &biome, &cands, budget), 2);
        assert_eq!(v.get((0, 0)), Some(Verdict::Touched), "an edit is never re-decided");
        assert_eq!(v.get((1, 0)), Some(Verdict::Untouched));
        assert_eq!(v.get((0, 1)), Some(Verdict::Untouched));
        assert_eq!(v.get((-1, 0)), None, "past the count");
        // A zero time budget still decides one.
        let rushed = VerdictBudget { count: 4, time: Duration::ZERO };
        assert_eq!(v.decide(&world, &biome, &cands, rushed), 1);
        assert_eq!(v.get((-1, 0)), Some(Verdict::Untouched));
    }

    /// Measurement (Phase B2b): the false-touched rate on fresh worlds — every
    /// column of the inner 9×9 of an 11×11 area generated nearest first,
    /// five seeds — and the cost of one verdict. Run with
    /// `cargo test --release --lib measure_verdicts -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement"]
    fn measure_verdicts() {
        let mut total = 0;
        let mut touched = Vec::new();
        let mut times = Vec::new();
        for seed in [1u32, 42, 1234, 99_999, 7_777_777] {
            let biome = BiomeGenerator::new(seed);
            let centre = ((seed % 97) as i32 - 48, (seed % 89) as i32 - 44);
            let world = generated(&biome, centre, 5);
            for dx in -4..=4 {
                for dz in -4..=4 {
                    let col = (centre.0 + dx, centre.1 + dz);
                    let t = web_time::Instant::now();
                    let v = column_verdict(&world, &biome, col);
                    times.push(t.elapsed());
                    total += 1;
                    if v == Verdict::Touched {
                        touched.push((seed, col));
                    }
                }
            }
        }
        times.sort_unstable();
        let mean = times.iter().sum::<Duration>() / times.len() as u32;
        println!(
            "verdicts: {total} columns, {} touched ({:.2}%): {touched:?}",
            touched.len(),
            100.0 * touched.len() as f64 / f64::from(total)
        );
        println!(
            "ms per verdict: mean {:.2}, median {:.2}, p95 {:.2}, max {:.2}",
            mean.as_secs_f64() * 1e3,
            times[times.len() / 2].as_secs_f64() * 1e3,
            times[times.len() * 95 / 100].as_secs_f64() * 1e3,
            times[times.len() - 1].as_secs_f64() * 1e3,
        );
    }
}

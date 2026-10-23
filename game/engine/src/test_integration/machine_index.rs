//! Oracle test for the per-chunk piston / hopper counters that let the
//! per-tick machine passes skip chunks without any (perf audit D.1).
//!
//! The pass used to read every cell of every loaded chunk. Now it reads only
//! chunks whose maintained counter is non-zero, so a counter that drifted from
//! the cells would make pistons or hoppers silently stop working. This test
//! drives every way blocks reach a `World` — single writes (into loaded,
//! unloaded and evicted cells), whole-chunk inserts, column generation, evict /
//! restore / discard, `clear` — and after each step checks the indexed passes
//! return exactly what a full scan of every loaded chunk finds.

use crate::biome::BiomeGenerator;
use crate::block::{self, BlockId};
use crate::chunk::{Chunk, CHUNK_SIZE};
use crate::world::{World, MAX_CHUNK_Y};

const COLS: i32 = 4; // columns 0..COLS on each horizontal axis

fn next(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// The oracle: every cell of every loaded chunk, no index.
fn scan_positions(world: &World, wanted: impl Fn(BlockId) -> bool) -> Vec<(i32, i32, i32)> {
    let cs = CHUNK_SIZE as i32;
    let mut out = Vec::new();
    for ((cx, cy, cz), chunk) in world.iter_chunks() {
        for ly in 0..CHUNK_SIZE {
            for lz in 0..CHUNK_SIZE {
                for lx in 0..CHUNK_SIZE {
                    if wanted(chunk.get(lx, ly, lz)) {
                        out.push((cx * cs + lx as i32, cy * cs + ly as i32, cz * cs + lz as i32));
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out
}

fn indexed_positions<'a>(
    chunks: impl Iterator<Item = ((i32, i32, i32), &'a Chunk)>,
    wanted: impl Fn(BlockId) -> bool,
) -> Vec<(i32, i32, i32)> {
    let cs = CHUNK_SIZE as i32;
    let mut out = Vec::new();
    for ((cx, cy, cz), chunk) in chunks {
        for ly in 0..CHUNK_SIZE {
            for lz in 0..CHUNK_SIZE {
                for lx in 0..CHUNK_SIZE {
                    if wanted(chunk.get(lx, ly, lz)) {
                        out.push((cx * cs + lx as i32, cy * cs + ly as i32, cz * cs + lz as i32));
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out
}

fn is_piston(b: BlockId) -> bool {
    b == block::PISTON || b == block::STICKY_PISTON
}

fn is_hopper(b: BlockId) -> bool {
    b == block::HOPPER
}

#[track_caller]
fn assert_index_matches_scan(world: &World, what: &str) {
    assert_eq!(
        crate::piston::collect_piston_positions(world).into_iter().collect::<std::collections::BTreeSet<_>>(),
        scan_positions(world, is_piston).into_iter().collect(),
        "{what}: piston index != full scan"
    );
    assert_eq!(
        indexed_positions(world.chunks_with_hoppers(), is_hopper),
        scan_positions(world, is_hopper),
        "{what}: hopper index != full scan"
    );
    // The counters' chunk filter itself: no chunk with a machine is skipped.
    let skipped_pistons = world.iter_chunks().filter(|(_, c)| !c.has_pistons()).any(|(_, c)| c.recount_machines().0 > 0);
    let skipped_hoppers = world.iter_chunks().filter(|(_, c)| !c.has_hoppers()).any(|(_, c)| c.recount_machines().1 > 0);
    assert!(!skipped_pistons && !skipped_hoppers, "{what}: a chunk holding a machine is filtered out");
}

fn machine_chunk(rng: &mut u64) -> Chunk {
    let palette = [block::AIR, block::STONE, block::PISTON, block::STICKY_PISTON, block::HOPPER];
    let mut c = Chunk::new();
    for _ in 0..40 {
        let r = next(rng);
        c.set((r & 15) as usize, ((r >> 4) & 15) as usize, ((r >> 8) & 15) as usize, palette[((r >> 16) % 5) as usize]);
    }
    c
}

#[test]
fn the_machine_index_equals_a_full_scan_through_edits_loads_and_evictions() {
    let palette = [block::AIR, block::STONE, block::PISTON, block::STICKY_PISTON, block::HOPPER, block::PISTON_HEAD];
    let mut world = World::new();
    let mut rng = 0xA5A5_1234_9876_ABCDu64;
    let span = COLS * CHUNK_SIZE as i32;
    let height = (MAX_CHUNK_Y + 1) * CHUNK_SIZE as i32;
    assert_index_matches_scan(&world, "empty world");

    // Natural terrain first (seed 42): generation writes through the same setters.
    let biome_gen = BiomeGenerator::new(42);
    for cx in 0..2 {
        for cz in 0..2 {
            world.generate_column(cx, cz, &biome_gen);
        }
    }
    assert_index_matches_scan(&world, "after generate_column");

    for round in 0..25 {
        // Random single-block edits, including into columns that were never loaded
        // (set_block conjures the chunk).
        for _ in 0..150 {
            let r = next(&mut rng);
            let x = (r % span as u64) as i32;
            let z = ((r >> 12) % span as u64) as i32;
            let y = ((r >> 24) % height as u64) as i32;
            world.set_block(x, y, z, palette[((r >> 40) % palette.len() as u64) as usize]);
        }
        assert_index_matches_scan(&world, &format!("round {round}: set_block"));

        // A whole chunk arriving (a pushed / loaded chunk) over whatever is there.
        let r = next(&mut rng);
        let (cx, cy, cz) = ((r % COLS as u64) as i32, ((r >> 8) % (MAX_CHUNK_Y as u64 + 1)) as i32, ((r >> 16) % COLS as u64) as i32);
        let chunk = machine_chunk(&mut rng);
        world.insert_chunk(cx, cy, cz, chunk);
        assert_index_matches_scan(&world, &format!("round {round}: insert_chunk"));

        // Bytes round trip: how a saved or pushed chunk is rebuilt.
        let r = next(&mut rng);
        let (cx, cy, cz) = ((r % COLS as u64) as i32, ((r >> 8) % (MAX_CHUNK_Y as u64 + 1)) as i32, ((r >> 16) % COLS as u64) as i32);
        if let Some(bytes) = world.get_chunk(cx, cy, cz).map(|c| c.as_bytes()) {
            world.insert_chunk(cx, cy, cz, Chunk::from_bytes(&bytes).expect("round trip"));
            assert_index_matches_scan(&world, &format!("round {round}: from_bytes"));
        }

        // Eviction (kept: an edited column moves to the evicted store), an edit
        // that writes through to the evicted copy, restore, and discard.
        let r = next(&mut rng);
        let (cx, cz) = ((r % COLS as u64) as i32, ((r >> 8) % COLS as u64) as i32);
        match (r >> 20) % 3 {
            0 => {
                world.evict_column(cx, cz);
                assert_index_matches_scan(&world, &format!("round {round}: evict"));
                let (x, z) = (cx * CHUNK_SIZE as i32 + 3, cz * CHUNK_SIZE as i32 + 5);
                world.set_block(x, 20, z, block::PISTON); // write-through if evicted
                assert_index_matches_scan(&world, &format!("round {round}: edit while evicted"));
                world.restore_column(cx, cz);
                assert_index_matches_scan(&world, &format!("round {round}: restore"));
            }
            1 => {
                world.discard_column(cx, cz);
                assert_index_matches_scan(&world, &format!("round {round}: discard"));
            }
            _ => {}
        }
    }
    // Survivors must be a real test: the run ended with machines present.
    assert!(!scan_positions(&world, is_piston).is_empty(), "the run left pistons to find");
    assert!(!scan_positions(&world, is_hopper).is_empty(), "the run left hoppers to find");

    world.clear();
    assert_index_matches_scan(&world, "after clear");
    assert!(crate::piston::collect_piston_positions(&world).is_empty());
}

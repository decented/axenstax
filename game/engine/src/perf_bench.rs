//! CPU-only benchmark for the world-side hot paths (no GPU, no window).
//!
//! `#[ignore]`d: run it in release mode and read the table.
//!
//! ```text
//! cargo test --release --lib perf_bench -- --ignored --nocapture
//! ```
//!
//! It times, for a seed-42 world at render distance 4, 10 and 16 (so
//! `(2r+1)^2` columns): `World::generate_column`, the initial light pass per
//! column, `build_chunk_meshes` per section (non-empty and all-air apart), and
//! the per-tick machine scans (`tick_pistons`, `tick_hoppers`) over the loaded
//! area. Numbers vary with the machine; the point is the before/after ratio on
//! one machine. Baselines live in `.claude/overnight/perf-baseline.md`.

use std::time::{Duration, Instant};

use crate::biome::BiomeGenerator;
use crate::block::{self, BlockRegistry};
use crate::chunk::CHUNK_SIZE;
use crate::mesh::build_chunk_meshes;
use crate::world::{World, MAX_CHUNK_Y};

const SEED: u32 = 42;
const RENDER_DISTANCES: [i32; 3] = [4, 10, 16];
/// How many columns round the origin are meshed per world (the full world at
/// render distance 4; a central square beyond that, to bound the run).
const MESH_SAMPLE_RADIUS: i32 = 3;
/// Repeats of each per-tick scan, averaged.
const SCAN_REPEATS: u32 = 20;

fn per(total: Duration, n: usize) -> f64 {
    if n == 0 {
        0.0
    } else {
        total.as_secs_f64() * 1e6 / n as f64
    }
}

struct Row {
    rd: i32,
    columns: usize,
    gen_us: f64,
    light_us: f64,
    mesh_us: f64,
    mesh_air_us: f64,
    mesh_sections: usize,
    mesh_air_sections: usize,
    piston_us: f64,
    hopper_us: f64,
    collect_pistons_us: f64,
    /// The pre-index full-cell scan, timed in the same process for comparison.
    legacy_scan_us: f64,
    chunks: usize,
}

/// The piston / hopper scan as it was before the per-chunk counters: read
/// every cell of every loaded chunk. Kept here only to time against.
fn legacy_full_scan(world: &World) -> usize {
    let mut found = 0;
    for (_, chunk) in world.iter_chunks() {
        for ly in 0..CHUNK_SIZE {
            for lz in 0..CHUNK_SIZE {
                for lx in 0..CHUNK_SIZE {
                    let b = chunk.get(lx, ly, lz);
                    if b == block::PISTON || b == block::STICKY_PISTON || b == block::HOPPER {
                        found += 1;
                    }
                }
            }
        }
    }
    found
}

fn run(rd: i32) -> Row {
    let registry = BlockRegistry::new();
    let biome_gen = BiomeGenerator::new(SEED, crate::biome::Generator::V2);
    let mut world = World::new();

    let mut cols = Vec::new();
    for cx in -rd..=rd {
        for cz in -rd..=rd {
            cols.push((cx, cz));
        }
    }
    // Generation and lighting interleaved per column, as the streamer does,
    // each timed on its own.
    let (mut gen_t, mut light_t) = (Duration::ZERO, Duration::ZERO);
    for &(cx, cz) in &cols {
        let t = Instant::now();
        world.generate_column(cx, cz, &biome_gen);
        gen_t += t.elapsed();
        let t = Instant::now();
        crate::lighting::run_initial_pass_for_column(&mut world, cx, cz, &registry);
        light_t += t.elapsed();
    }

    // A few machines so the scans have something to find (a world with none
    // still pays the scan; these keep the oracle honest).
    for i in 0..4 {
        world.set_block(8 + i * 3, 70, 8, block::PISTON);
        world.set_block(8 + i * 3, 70, 12, block::HOPPER);
    }

    // Meshing: every section of the central square, non-empty vs all-air.
    let r = MESH_SAMPLE_RADIUS.min(rd);
    let (mut mesh_t, mut air_t) = (Duration::ZERO, Duration::ZERO);
    let (mut mesh_n, mut air_n) = (0usize, 0usize);
    for cx in -r..=r {
        for cz in -r..=r {
            for cy in 0..=MAX_CHUNK_Y {
                let Some(chunk) = world.get_chunk(cx, cy, cz) else { continue };
                let empty = chunk.is_empty();
                let t = Instant::now();
                let m = build_chunk_meshes(cx, cy, cz, &world, &registry);
                let dt = t.elapsed();
                std::hint::black_box(&m);
                if empty {
                    air_t += dt;
                    air_n += 1;
                } else {
                    mesh_t += dt;
                    mesh_n += 1;
                }
            }
        }
    }

    // Per-tick scans over the whole loaded area.
    let t = Instant::now();
    for _ in 0..SCAN_REPEATS {
        std::hint::black_box(crate::piston::collect_piston_positions(&world));
    }
    let collect = t.elapsed() / SCAN_REPEATS;
    let t = Instant::now();
    for _ in 0..SCAN_REPEATS {
        std::hint::black_box(legacy_full_scan(&world));
    }
    let legacy = t.elapsed() / SCAN_REPEATS;
    let t = Instant::now();
    for _ in 0..SCAN_REPEATS {
        std::hint::black_box(crate::piston::tick_pistons(&mut world));
    }
    let pistons = t.elapsed() / SCAN_REPEATS;
    let t = Instant::now();
    for _ in 0..SCAN_REPEATS {
        // Tick 0 is on the hopper cadence.
        crate::hopper::tick_hoppers(&mut world, 0);
    }
    let hoppers = t.elapsed() / SCAN_REPEATS;

    Row {
        rd,
        columns: cols.len(),
        gen_us: per(gen_t, cols.len()),
        light_us: per(light_t, cols.len()),
        mesh_us: per(mesh_t, mesh_n),
        mesh_air_us: per(air_t, air_n),
        mesh_sections: mesh_n,
        mesh_air_sections: air_n,
        piston_us: pistons.as_secs_f64() * 1e6,
        hopper_us: hoppers.as_secs_f64() * 1e6,
        collect_pistons_us: collect.as_secs_f64() * 1e6,
        legacy_scan_us: legacy.as_secs_f64() * 1e6,
        chunks: world.iter_chunks().count(),
    }
}

#[test]
#[ignore = "benchmark: cargo test --release --lib perf_bench -- --ignored --nocapture"]
fn world_cpu_bench() {
    let rows: Vec<Row> = RENDER_DISTANCES.iter().map(|&rd| run(rd)).collect();
    let profile = if cfg!(debug_assertions) { "DEBUG (numbers not comparable)" } else { "release" };
    eprintln!("\nperf_bench  seed {SEED}  profile {profile}  chunk {CHUNK_SIZE}^3  sections/column {}", MAX_CHUNK_Y + 1);
    eprintln!(
        "{:>3} {:>6} {:>7} | {:>10} {:>10} | {:>10} {:>5} {:>10} {:>5} | {:>12} {:>12} {:>12}",
        "rd", "cols", "chunks", "gen us/col", "light us/c", "mesh us/sec", "n", "air us/sec", "n",
        "collect_pis", "tick_pistons", "tick_hopper"
    );
    eprintln!("    (legacy full-cell scan of the same world, for comparison, in the last table)");
    for r in &rows {
        eprintln!(
            "{:>3} {:>6} {:>7} | {:>10.0} {:>10.0} | {:>10.0} {:>5} {:>10.1} {:>5} | {:>10.0}us {:>10.0}us {:>10.0}us",
            r.rd, r.columns, r.chunks, r.gen_us, r.light_us, r.mesh_us, r.mesh_sections, r.mesh_air_us,
            r.mesh_air_sections, r.collect_pistons_us, r.piston_us, r.hopper_us
        );
    }
    eprintln!("\nfull-cell scan (the pre-counter piston/hopper pass) vs indexed pass, same process:");
    for r in &rows {
        eprintln!(
            "rd {:>2}: legacy scan {:>9.0}us   indexed collect_piston_positions {:>7.0}us   ({:.0}x)",
            r.rd, r.legacy_scan_us, r.collect_pistons_us, r.legacy_scan_us / r.collect_pistons_us.max(0.1)
        );
    }
    stream_batch_report();
}

/// Mesh work for a 2x2 batch of columns streaming in next to a loaded ring:
/// the old per-column algorithm against the batch plan, both timed here.
fn stream_batch_report() {
    use crate::stream_mesh::{build_sections, plan_stream_meshes, section_kind, MeshPolicy, SectionKind};
    let registry = BlockRegistry::new();
    let biome_gen = BiomeGenerator::new(SEED, crate::biome::Generator::V2);
    let mut world = World::new();
    let area: Vec<(i32, i32)> = (0..4).flat_map(|x| (0..4).map(move |z| (x, z))).collect();
    let batch = [(1, 1), (2, 1), (1, 2), (2, 2)];
    for &(cx, cz) in &area {
        world.generate_column(cx, cz, &biome_gen);
        crate::lighting::run_initial_pass_for_column(&mut world, cx, cz, &registry);
    }
    let preloaded = |x: i32, z: i32| area.contains(&(x, z)) && !batch.contains(&(x, z));

    // Old: per column, mesh every present section, then every present section
    // of each loaded 4-neighbour (a batch column counts once it has been meshed).
    let t = Instant::now();
    let mut legacy_builds = 0usize;
    let mut done: Vec<(i32, i32)> = Vec::new();
    for &(cx, cz) in &batch {
        done.push((cx, cz));
        for cy in 0..=MAX_CHUNK_Y {
            if world.has_chunk(cx, cy, cz) {
                std::hint::black_box(build_chunk_meshes(cx, cy, cz, &world, &registry));
                legacy_builds += 1;
            }
        }
        for (nx, nz) in [(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)] {
            if done.contains(&(nx, nz)) || preloaded(nx, nz) {
                for cy in 0..=MAX_CHUNK_Y {
                    if world.has_chunk(nx, cy, nz) {
                        std::hint::black_box(build_chunk_meshes(nx, cy, nz, &world, &registry));
                        legacy_builds += 1;
                    }
                }
            }
        }
    }
    let legacy_t = t.elapsed();

    let kind = |p| section_kind(&world, p);
    let t = Instant::now();
    let plan = plan_stream_meshes(&batch, MeshPolicy::STREAMING, preloaded, kind);
    let mut sink = 0usize;
    let inline = build_sections(&world, &registry, &plan.now, |_, m| sink += m.opaque.indices.len());
    let inline_t = t.elapsed();
    let queued = build_sections(&world, &registry, &plan.later, |_, m| sink += m.opaque.indices.len());
    let total_t = t.elapsed();
    std::hint::black_box(sink);
    let empty = (0..=MAX_CHUNK_Y).filter(|&cy| batch.iter().all(|&(cx, cz)| kind((cx, cy, cz)) != SectionKind::Solid)).count();
    eprintln!(
        "\nstream batch (4 columns beside a loaded ring), mesh builds and CPU time:\n  \
         per-column algorithm: {legacy_builds} builds, {:.1} ms (all inline, one frame)\n  \
         batched plan:         {} builds ({inline} inline = {:.1} ms; {queued} queued for the 8/frame dirty pass), {:.1} ms total  [{empty} all-air levels]",
        legacy_t.as_secs_f64() * 1e3,
        inline + queued,
        inline_t.as_secs_f64() * 1e3,
        total_t.as_secs_f64() * 1e3,
    );
}

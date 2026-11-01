//! Keep slice 1 — the tier-L tick measurement (Keep handoff §8). `#[ignore]`:
//! run it by hand in the release profile, once per machine you want a number
//! for, and record the output in the handoff's §8 "Measured":
//!
//! ```text
//! cargo test --release --lib tick_bench -- --ignored --nocapture
//! ```
//!
//! For 0, 2 and 8 synthetic joiners (server-simulated bodies walking and
//! turning, spread on a ring round spawn) on a dedicated-style server at sim
//! distance 8, with the twin-run machine fixture (fire, furnaces, hoppers,
//! campfires, water) and the scattered + placed mobs present: `GameServer::tick`
//! wall time p50 / p99 / max over [`MEASURED`] ticks, after [`WARMUP`] ticks
//! in which the streamer loads the whole sim distance. Then a scripted build
//! (blocks placed by every body, items in every joiner's window), a save, and
//! the bytes on disk of the touched-chunk files and the joiner sidecars.
//! `GameServer::tick` only: no transport, admission or per-client diff
//! (`HostedServer::tick`), so this is the simulation's share of a tick. Three
//! ticks in four skip the fluid and machine pass, so p50 reads that cheap path;
//! read p99 and the mean for the load. The fire field has burnt out by the end
//! of the warm-up, so the machines are mostly idle scans here.
//!
//! Lives under `#[cfg(test)]`, so its `Instant` is outside the tick lint.

use std::time::Instant;

use glam::Vec3;

use super::twin_run::{add_joiner, drive_joiners, twin_loaded, BASE_Y};
use crate::save::WorldsRootGuard;

const SIM_DISTANCE: i32 = 8;
const WARMUP: u64 = 600;
const MEASURED: usize = 600;

fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    let i = ((sorted_ms.len() as f64 - 1.0) * p).round() as usize;
    sorted_ms[i]
}

/// Bytes and file count under `dir` (recursive); `(0, 0)` if absent.
fn dir_bytes(dir: &std::path::Path) -> (u64, usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return (0, 0) };
    let mut total = (0, 0);
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            let (b, n) = dir_bytes(&e.path());
            total.0 += b;
            total.1 += n;
        } else {
            total.0 += meta.len();
            total.1 += 1;
        }
    }
    total
}

#[test]
#[ignore]
fn tick_bench_joiners_at_sim_distance_8() {
    eprintln!(
        "tick_bench: debug_assertions={} (run with --release for the recorded numbers)",
        cfg!(debug_assertions)
    );
    for joiners in [0usize, 2, 8] {
        let name = format!("keep-s1-bench-{joiners}");
        let _root = WorldsRootGuard::new(&name);
        let mut h = twin_loaded(4242, &name, 0, SIM_DISTANCE, SIM_DISTANCE);
        for j in 0..joiners {
            let a = j as f32 / joiners as f32 * std::f32::consts::TAU;
            let pos = Vec3::new(8.5 + 40.0 * a.cos(), (BASE_Y + 1) as f32, 8.5 + 40.0 * a.sin());
            let slot = add_joiner(&mut h, pos);
            let mut pk = [0u8; 32];
            pk[0] = 0x02;
            pk[31] = slot as u8;
            h.server.players[slot].verified_pubkey = Some(pk);
        }
        while h.server.tick_counter < WARMUP {
            let next = h.server.tick_counter + 1;
            drive_joiners(&mut h, next);
            h.server.tick();
        }
        let mut ms = Vec::with_capacity(MEASURED);
        for _ in 0..MEASURED {
            let next = h.server.tick_counter + 1;
            drive_joiners(&mut h, next);
            let t = Instant::now();
            h.server.tick();
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let mean = ms.iter().sum::<f64>() / ms.len() as f64;
        ms.sort_by(|a, b| a.total_cmp(b));
        eprintln!(
            "tick_bench joiners={joiners} sim={SIM_DISTANCE}: p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms, mean {:.2} ms \
             over {MEASURED} ticks; columns {} entities {}",
            percentile(&ms, 0.50),
            percentile(&ms, 0.99),
            ms[ms.len() - 1],
            mean,
            h.server.loaded_columns.len(),
            h.server.ecs.len(),
        );

        // Scripted build: a 24×24 four-course wall-and-floor block by the
        // spawn body (2,304 blocks over four columns), 8×8×2 by each joiner
        // where it stands, and a window of items for each joiner's sidecar.
        for x in 0..24 {
            for z in 30..54 {
                for y in 0..4 {
                    let b = if (x + z + y) % 3 == 0 { crate::block::COBBLESTONE } else { crate::block::OAK_PLANKS };
                    h.server.world.place_player_block(x, BASE_Y + 1 + y, z, b);
                }
            }
        }
        for slot in 0..h.server.players.len() {
            if !h.server.players[slot].server_simulated {
                continue;
            }
            let at = h.server.players[slot].player.pos;
            let (bx, by, bz) = (at.x.floor() as i32, at.y.floor() as i32 + 2, at.z.floor() as i32);
            for dx in 0..8 {
                for dz in 0..8 {
                    for dy in 0..2 {
                        h.server.world.place_player_block(bx + dx, by + dy, bz + dz, crate::block::STONE);
                    }
                }
            }
            for i in 0..20u8 {
                let stack = crate::item::ItemStack::new_block(crate::block::COBBLESTONE, 1 + i);
                let _ = h.server.players[slot].inventory.add_item(stack);
            }
        }
        // A server save writes every non-empty chunk it holds, touched or not
        // (`GameServer::try_save`), so `chunks/` is not the touched set: count
        // that separately, by the flag eviction keeps a column for.
        let touched = h
            .server
            .world
            .persistable_chunks()
            .filter(|(_, c)| c.persist() && !c.is_empty())
            .count();
        let touched_bytes = touched * crate::chunk::Chunk::new().as_bytes().len();
        h.server.try_save().expect("bench world saves");
        let dir = crate::save::world_dir(&name);
        let (chunk_bytes, chunk_files) = dir_bytes(&dir.join("chunks"));
        let (sidecar_bytes, sidecars) = dir_bytes(&dir.join("joiners"));
        let (all_bytes, all_files) = dir_bytes(&dir);
        eprintln!(
            "tick_bench joiners={joiners} save: touched {touched} chunks {touched_bytes} B; \
             chunks/ {chunk_files} files {chunk_bytes} B (all non-empty); \
             joiners/ {sidecars} sidecars {sidecar_bytes} B; world dir {all_files} files {all_bytes} B"
        );
    }
}

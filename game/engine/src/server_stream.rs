//! Dedicated-server column streaming (Phase B1, 2026-10-06; Spec 01 §4.1.2).
//!
//! A dedicated server (`HostedServer` with 0 local players — `server_main`)
//! has no host client streaming terrain for it, so `GameServer::initial_load`'s
//! region around the world spawn used to be all the server ever held: a joiner
//! who walked out of it fell through server-side air, and every edit there was
//! refused as `Unloaded`. Here the server loads and unloads columns itself,
//! around every connected player plus the world spawn, with the SAME policy the
//! client streamer uses (`chunk_stream::plan_stream_step`) and the same
//! per-column steps (`chunk_stream::ColumnSims`): restore-else-generate, light,
//! fluid/fire rescan and mob scatter in; mob reclaim and evict-or-drop out.
//!
//! Unload never writes to disk and never deletes a file: an edited or saved
//! column moves to the World's in-memory evicted store, and every server save
//! (`GameServer::try_save`, via `World::persistable_chunks`) still writes it
//! (Spec 02 §7.5.1). The streamer never reads the disk either — the whole save
//! is in memory from boot — so there is no "column that failed to load" for it
//! to write back.
//!
//! LAN / online hosts (≥ 1 local player) do not use this: their host client
//! streams, and the server keeps its `initial_load` region (a later step lends
//! the host client's world to the server).

use crate::chunk_stream::{column_of, is_void_column, plan_stream_step, ColumnSims};
use crate::server::GameServer;

/// Columns a dedicated server streams in per tick. Each is a restore or a
/// full `generate_column` plus a column light pass, so this caps the tick-time
/// spike (the client streams 4 per frame; the server runs 20 ticks a second).
pub const SERVER_STREAM_BUDGET: usize = 2;

/// Default `--sim-distance`: the radius, in columns (Chebyshev), the server
/// keeps loaded around each connected player and the world spawn.
pub const DEFAULT_SIM_DISTANCE: i32 = 8;
/// `--sim-distance` floor: reach and mob AI need the neighbouring columns.
// The `--sim-distance` items are reached only from native `server_main`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const MIN_SIM_DISTANCE: i32 = 2;
/// `--sim-distance` ceiling: the client's own render-distance maximum (a
/// radius of 16 is 1,089 columns per isolated player).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const MAX_SIM_DISTANCE: i32 = 16;

/// Clamp an operator-supplied sim distance into the supported range.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn clamp_sim_distance(columns: i32) -> i32 {
    columns.clamp(MIN_SIM_DISTANCE, MAX_SIM_DISTANCE)
}

/// The dedicated server's streaming state. Present on a `GameServer` only when
/// it runs as the dedicated server (`HostedServer::start_inner`, 0 local
/// players); `None` everywhere else, including `TestHost`.
#[derive(Debug)]
pub struct ColumnStreamer {
    sim_distance: i32,
    budget: usize,
    /// The anchor columns of the last pass that left nothing waiting. While
    /// every anchor stays in the same column there is no new work, so the pass
    /// is skipped. Cleared by a sim-distance change. A void column is therefore
    /// only re-checked when an anchor changes column.
    settled_for: Option<Vec<(i32, i32)>>,
}

impl Default for ColumnStreamer {
    fn default() -> Self {
        Self { sim_distance: DEFAULT_SIM_DISTANCE, budget: SERVER_STREAM_BUDGET, settled_for: None }
    }
}

impl ColumnStreamer {
    /// The radius in force (after clamping). Read by tests today.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sim_distance(&self) -> i32 {
        self.sim_distance
    }
}

impl GameServer {
    /// Set the dedicated server's streaming radius (`--sim-distance`), clamped
    /// to `MIN_SIM_DISTANCE..=MAX_SIM_DISTANCE`. Takes effect on the next tick:
    /// columns beyond the new radius + hysteresis unload, missing ones stream in
    /// at the per-tick budget. No-op on a server that doesn't stream (a host).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn set_sim_distance(&mut self, columns: i32) {
        if let Some(s) = self.column_streamer.as_mut() {
            s.sim_distance = clamp_sim_distance(columns);
            s.settled_for = None;
        }
    }

    /// The columns the server keeps loaded around: every connected player's
    /// column plus the world spawn's (so the spawn area stays warm for the
    /// next joiner). Ghost slots (disconnected, kept for index stability) are
    /// skipped. Sorted and deduplicated, so it doubles as the settled key.
    pub(crate) fn stream_anchors(&self) -> Vec<(i32, i32)> {
        let mut anchors: Vec<(i32, i32)> = self
            .players
            .iter()
            .filter(|sp| sp.connected)
            .map(|sp| column_of(sp.player.pos))
            .chain(std::iter::once(self.spawn_column))
            .collect();
        anchors.sort_unstable();
        anchors.dedup();
        anchors
    }

    /// One streaming pass (called at the top of `tick`, before anything reads
    /// the world). Unloads columns outside every anchor's radius + hysteresis,
    /// then streams in up to the budget, each anchor's own column first.
    /// Returns how many columns were streamed in. No-op unless this server
    /// streams (the dedicated server).
    pub(crate) fn stream_columns(&mut self) -> usize {
        let Some(streamer) = self.column_streamer.as_ref() else {
            return 0;
        };
        let anchors = self.stream_anchors();
        if streamer.settled_for.as_ref() == Some(&anchors) {
            return 0;
        }
        let world = &self.world;
        let step = plan_stream_step(
            &anchors,
            &anchors,
            streamer.sim_distance,
            streamer.budget,
            &self.loaded_columns,
            |cx, cz| is_void_column(world, cx, cz),
        );
        if step.healed > 0 {
            log::warn!("server stream self-heal: re-generating {} void column(s)", step.healed);
        }
        for &(cx, cz) in &step.unload {
            self.column_sims().stream_out(cx, cz);
        }
        for &(cx, cz) in &step.load {
            self.column_sims().stream_in(cx, cz);
        }
        // Everything wanted fits this pass: settled until an anchor moves.
        let settled = step.pending <= step.load.len();
        if let Some(s) = self.column_streamer.as_mut() {
            s.settled_for = settled.then_some(anchors);
        }
        step.load.len()
    }

    fn column_sims(&mut self) -> ColumnSims<'_> {
        ColumnSims {
            world: &mut self.world,
            loaded: &mut self.loaded_columns,
            registry: &self.registry,
            biome_gen: &self.biome_gen,
            water: &mut self.water,
            lava: &mut self.lava,
            fire: &mut self.fire,
            ecs: &mut self.ecs,
            tick: self.tick_counter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_distance_is_clamped_to_the_supported_range() {
        assert_eq!(clamp_sim_distance(0), MIN_SIM_DISTANCE);
        assert_eq!(clamp_sim_distance(-5), MIN_SIM_DISTANCE);
        assert_eq!(clamp_sim_distance(8), 8);
        assert_eq!(clamp_sim_distance(1_000), MAX_SIM_DISTANCE);
    }

    #[test]
    fn a_server_without_a_streamer_never_streams() {
        let mut server = GameServer::new(0, "server-stream-none".into(), 42);
        assert!(server.column_streamer.is_none(), "off by default (hosts, TestHost)");
        server.set_sim_distance(4);
        assert_eq!(server.stream_columns(), 0);
        assert!(server.loaded_columns.is_empty());
    }

    /// Streaming from nothing: the anchor's own column first, then the budget
    /// per pass until the square is loaded, then the pass settles.
    #[test]
    fn streams_the_square_around_spawn_within_budget_then_settles() {
        let mut server = GameServer::new(0, "server-stream-square".into(), 42);
        server.column_streamer = Some(ColumnStreamer::default());
        server.set_sim_distance(MIN_SIM_DISTANCE);
        let side = (2 * MIN_SIM_DISTANCE + 1) as usize;
        let mut passes = 0;
        loop {
            let before = server.loaded_columns.len();
            let n = server.stream_columns();
            assert!(n <= SERVER_STREAM_BUDGET);
            assert_eq!(server.loaded_columns.len(), before + n);
            if passes == 0 {
                assert!(server.loaded_columns.contains(&(0, 0)), "spawn column first");
            }
            passes += 1;
            if n == 0 {
                break;
            }
            assert!(passes < 100, "streaming must converge");
        }
        assert_eq!(server.loaded_columns.len(), side * side);
        assert!(server.column_streamer.as_ref().unwrap().settled_for.is_some());
    }
}

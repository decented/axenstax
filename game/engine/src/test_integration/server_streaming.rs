//! Phase B1 — a dedicated server streams world columns around every connected
//! player (`server_stream.rs`, Spec 01 §4.1.2).
//!
//! Drives REAL joins and REAL movement intents through `HostedServer::tick`
//! over the in-process channel transport. Before B1 the dedicated server only
//! ever held `initial_load`'s region around spawn: a joiner who left it fell
//! through server air and every edit out there was refused as `Unloaded`.
//!
//! The joiner travels by creative flight above the build height: a grounded
//! walk over real terrain stalls at the first two-block step, and the streamer
//! only sees the server-simulated position, however it moves.

use std::time::{Duration, Instant};

use super::joiner_authority::{cell_beside, join_guest, send_edits};
use crate::block;
use crate::chunk_stream::column_of;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol;
use crate::save::WorldsRootGuard;
use crate::server_stream::SERVER_STREAM_BUDGET;
use crate::transport::{ChannelClientTransport, ClientTransport};

/// A small radius keeps the walk cheap: 5 new columns per column crossed.
const SIM: i32 = 2;
/// Far enough that the destination is ~19 columns past the boot region's
/// edge (radius 10 around spawn), so only streaming can have loaded it.
const FAR_X: f32 = 300.0;
/// Cruising height: above the build height (6 chunks × 16 = 96), so no
/// terrain is in the way.
const CRUISE_Y: f32 = 110.0;

/// A 0-local-player server — what `server_main` runs — in creative (flight,
/// no fall damage), streaming at `SIM`.
fn start_dedicated(world: &str) -> HostedServer {
    let mut hs = HostedServer::start(
        0,
        world.to_string(),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts");
    hs.server.set_play_mode(crate::play_mode::PlayMode::Creative);
    hs.server.set_sim_distance(SIM);
    hs
}

/// One movement input from the joiner. Yaw 0: `move_right` = +x.
fn send_move(client: &ChannelClientTransport, tick: u64, right: f32, jump: bool, toggle_flight: bool) {
    let input = protocol::InputPacket {
        tick,
        move_right: right,
        sprint: true,
        jump,
        toggle_flight,
        health: 20.0,
        ..Default::default()
    };
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
}

/// Running state for one joiner's journey.
struct Journey<'a> {
    hs: &'a mut HostedServer,
    client: &'a ChannelClientTransport,
    slot: usize,
    tick: u64,
}

impl Journey<'_> {
    fn pos(&self) -> glam::Vec3 {
        self.hs.server.players[self.slot].player.pos
    }

    /// Send one input and tick once, asserting the B1 invariants: no more
    /// than the budget streamed in this tick, the player's own column is
    /// loaded, and the player is never below the world floor.
    fn step(&mut self, right: f32, jump: bool, toggle_flight: bool) {
        self.tick += 1;
        send_move(self.client, self.tick, right, jump, toggle_flight);
        let before = self.hs.server.loaded_columns.clone();
        self.hs.tick();
        let streamed_in = self.hs.server.loaded_columns.difference(&before).count();
        assert!(
            streamed_in <= SERVER_STREAM_BUDGET,
            "tick {}: {streamed_in} columns streamed in (budget {SERVER_STREAM_BUDGET})",
            self.tick
        );
        let at = self.pos();
        assert!(
            self.hs.server.loaded_columns.contains(&column_of(at)),
            "tick {}: the joiner's column {:?} is not loaded (at {at:?})",
            self.tick,
            column_of(at)
        );
        assert!(at.y > 0.0, "tick {}: fell through the world (at {at:?})", self.tick);
    }

    /// Fly (creative, sprinting) at `CRUISE_Y` until x reaches `to_x`.
    fn fly_to_x(&mut self, to_x: f32) {
        self.step(0.0, false, true); // toggle flight on
        for _ in 0..200 {
            if self.pos().y >= CRUISE_Y {
                break;
            }
            self.step(0.0, true, false);
        }
        assert!(self.pos().y >= CRUISE_Y, "climbed to cruise height: {:?}", self.pos());
        let dir = if to_x > self.pos().x { 1.0 } else { -1.0 };
        for _ in 0..2_000 {
            if (to_x - self.pos().x) * dir <= 0.0 {
                return;
            }
            self.step(dir, false, false);
        }
        panic!("never reached x {to_x}: at {:?}", self.pos());
    }

    /// Stop flying and drop to the ground; return once standing.
    fn land(&mut self) {
        self.step(0.0, false, true); // toggle flight off
        for _ in 0..400 {
            if self.hs.server.players[self.slot].player.on_ground {
                return;
            }
            self.step(0.0, false, false);
        }
        panic!("never landed: at {:?}", self.pos());
    }
}

/// An air cell the joiner can reach: beside the head, else above it.
fn air_cell_in_reach(hs: &HostedServer, slot: usize) -> (i32, i32, i32) {
    [(1, 1, 0), (0, 1, 1), (-1, 1, 0), (0, 1, -1), (1, 2, 0), (0, 2, 0)]
        .into_iter()
        .map(|(dx, dy, dz)| cell_beside(hs, slot, dx, dy, dz))
        .find(|&(x, y, z)| hs.server.world.get_block(x, y, z) == block::AIR)
        .expect("an air cell next to a standing player")
}

/// The full B1 acceptance path: walk 300 blocks out, stand, build, walk home
/// (the far column unloads — kept because edited, pristine ones dropped),
/// save, restart, walk out again: the edit is back from the save.
#[test]
fn a_joiner_far_from_spawn_stands_builds_and_their_edit_survives_unload_and_restart() {
    let _g = WorldsRootGuard::new("server_streaming_far");
    let name = format!("server-streaming-far-{}", std::process::id());

    let mut hs = start_dedicated(&name);
    let (client, slot) = join_guest(&mut hs, "Wanderer");
    let mut j = Journey { hs: &mut hs, client: &client, slot, tick: 0 };

    // ── Out: 300 blocks east, then stand on the server's ground there. ──
    j.fly_to_x(FAR_X);
    j.land();
    let at = j.pos();
    let feet = (at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
    let ground = j.hs.server.world.get_block(feet.0, (at.y - 0.05).floor() as i32, feet.2);
    assert_ne!(ground, block::AIR, "standing on generated ground at {at:?}");
    let far_col = column_of(at);
    assert!(far_col.0 >= 18, "really far from spawn: {far_col:?}");

    // ── Build there: the edit is accepted (pre-B1: refused as Unloaded). ──
    let cell = air_cell_in_reach(j.hs, slot);
    j.tick += 1;
    send_edits(j.hs, &client, slot, j.tick, &[(cell, block::GLASS)]);
    j.hs.tick();
    assert_eq!(j.hs.server.world.get_block(cell.0, cell.1, cell.2), block::GLASS, "far edit accepted");
    let edit_col = column_of(glam::Vec3::new(cell.0 as f32, 0.0, cell.2 as f32));
    let edit_chunk = (edit_col.0, cell.1.div_euclid(16), edit_col.1);

    // ── Home: the far columns unload. ──
    j.fly_to_x(0.5);
    j.land();
    let server = &j.hs.server;
    assert!(!server.loaded_columns.contains(&edit_col), "the far column unloaded");
    assert!(server.world.is_column_evicted(edit_col.0, edit_col.1), "the edited column is kept");
    let dropped_somewhere = (12..=17).flat_map(|cx| (-SIM..=SIM).map(move |cz| (cx, cz))).any(|(cx, cz)| {
        !server.loaded_columns.contains(&(cx, cz))
            && !server.world.is_column_evicted(cx, cz)
            && !(0..=5).any(|cy| server.world.has_chunk(cx, cy, cz))
    });
    assert!(dropped_somewhere, "pristine far columns are dropped, not kept");

    // ── Save: the existing save path writes the evicted column. ──
    j.hs.server.try_save().expect("server save");
    let chunk_file = crate::save::world_dir(&name)
        .join("chunks")
        .join(format!("{}_{}_{}.chunk", edit_chunk.0, edit_chunk.1, edit_chunk.2));
    assert!(chunk_file.exists(), "the unloaded edit is on disk: {}", chunk_file.display());
    drop(j);
    drop(client);
    drop(hs);

    // ── Restart from the save: the far column boots evicted, then streams
    //    back in from the store with the edit when a joiner walks out. ──
    let mut hs = start_dedicated(&name);
    hs.tick();
    assert!(hs.server.world.is_column_evicted(edit_col.0, edit_col.1), "boot evicts far saved columns");
    let (client, slot) = join_guest(&mut hs, "Returner");
    let mut j = Journey { hs: &mut hs, client: &client, slot, tick: 0 };
    j.fly_to_x(cell.0 as f32 + 0.5);
    let server = &j.hs.server;
    assert!(server.loaded_columns.contains(&edit_col), "the column streamed back in");
    assert!(!server.world.is_column_evicted(edit_col.0, edit_col.1), "restored to the live chunks");
    assert!(server.world.has_chunk(edit_chunk.0, edit_chunk.1, edit_chunk.2));
    assert_eq!(server.world.get_block(cell.0, cell.1, cell.2), block::GLASS, "the edit came back");
}

/// Per-pass cost of the dedicated streamer, printed for the B1 report (run
/// with `--nocapture`). Asserts only the budget and that a settled pass does
/// no work; timings vary with the machine and profile.
#[test]
fn stream_pass_cost_is_bounded_and_a_settled_pass_is_free() {
    let _g = WorldsRootGuard::new("server_streaming_cost");
    let mut hs = start_dedicated(&format!("server-streaming-cost-{}", std::process::id()));
    let (_client, slot) = join_guest(&mut hs, "Teleporter");

    let idle_start = Instant::now();
    for _ in 0..100 {
        assert_eq!(hs.server.stream_columns(), 0, "settled: nothing to do");
    }
    let idle = idle_start.elapsed() / 100;

    // Jump the joiner 40 columns away: a whole new square to stream.
    hs.server.players[slot].player.pos = glam::Vec3::new(40.0 * 16.0 + 8.0, CRUISE_Y, 8.0);
    let mut busy: Vec<Duration> = Vec::new();
    for _ in 0..100 {
        let t = Instant::now();
        let n = hs.server.stream_columns();
        let dt = t.elapsed();
        assert!(n <= SERVER_STREAM_BUDGET);
        if n == 0 {
            break;
        }
        busy.push(dt);
    }
    let side = (2 * SIM + 1) as usize;
    assert_eq!(busy.len(), (side * side).div_ceil(SERVER_STREAM_BUDGET), "budget-paced");
    let max = busy.iter().max().copied().unwrap_or_default();
    let mean = busy.iter().sum::<Duration>() / busy.len() as u32;
    eprintln!(
        "B1 stream pass: settled {idle:?}; streaming {SERVER_STREAM_BUDGET} cols/pass mean {mean:?}, max {max:?} \
         ({} passes)",
        busy.len()
    );
}

/// One column-loading story per mode: only the dedicated server streams, and
/// only a LAN / online host refills round its joiners — never both.
#[test]
fn only_the_dedicated_server_gets_a_column_streamer() {
    let _g = WorldsRootGuard::new("server_streaming_flag");
    let start = |local: usize, tag: &str| {
        HostedServer::start(
            local,
            format!("server-streaming-flag-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("server starts")
    };
    let dedicated = start(0, "dedicated");
    let streamer = dedicated.server.column_streamer.as_ref().expect("dedicated streams");
    assert_eq!(streamer.sim_distance(), crate::server_stream::DEFAULT_SIM_DISTANCE);
    assert_eq!(dedicated.server.column_refill_per_tick, 0, "the dedicated server does not also refill");
    // `start` is an owning server: with a host client, the `--no-lend` host.
    let host = start(1, "host");
    assert!(host.server.column_streamer.is_none(), "a host client streams for its server");
    assert_eq!(
        host.server.column_refill_per_tick,
        crate::server::HOST_COLUMN_REFILL_PER_TICK,
        "an owning host refills round its joiners instead"
    );
    // D1 — a host that lends its world loads no column on the server side at
    // all: its host client's streamer anchors on every joiner
    // (`chunk_stream::client_stream_anchors`).
    let lent = HostedServer::start_host(
        1,
        format!("server-streaming-flag-lent-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
        crate::hosted_server::HostWorld::Lent,
    )
    .expect("lending host starts");
    assert!(lent.server.column_streamer.is_none(), "a lending host's server does not stream");
    assert_eq!(lent.server.column_refill_per_tick, 0, "nor refill");
}

/// The streamer's spawn anchor follows the computed world spawn
/// (`GameServer::world_spawn`, which replaced the fixed BRIDGE point): set at
/// boot, the column a joiner is placed in, and re-recorded whenever the spawn
/// is computed again.
#[test]
fn the_spawn_anchor_follows_the_computed_world_spawn() {
    let _g = WorldsRootGuard::new("server_streaming_spawn");
    let mut hs = start_dedicated(&format!("server-streaming-spawn-{}", std::process::id()));
    let boot_anchor = hs.server.spawn_column;
    let spawn = hs.server.world_spawn();
    assert_eq!(boot_anchor, column_of(spawn), "the anchor is the computed spawn's column from boot");
    let (_client, slot) = join_guest(&mut hs, "Newcomer");
    let placed = column_of(hs.server.players[slot].spawn_pos);
    assert_eq!(placed, hs.server.spawn_column, "joiners are placed in the anchored column");
    assert!(hs.server.stream_anchors().contains(&placed));
    // A stale anchor is corrected by the next computation.
    hs.server.spawn_column = (7, 7);
    let again = hs.server.world_spawn();
    assert_eq!(hs.server.spawn_column, column_of(again));
}

/// A disconnected slot (kept for index stability) no longer pins columns.
#[test]
fn a_disconnected_ghost_slot_is_not_a_streaming_anchor() {
    let _g = WorldsRootGuard::new("server_streaming_ghost");
    let mut hs = start_dedicated(&format!("server-streaming-ghost-{}", std::process::id()));
    let (_client, slot) = join_guest(&mut hs, "Ghost");
    hs.server.players[slot].player.pos = glam::Vec3::new(30.0 * 16.0, CRUISE_Y, 0.0);
    assert!(hs.server.stream_anchors().contains(&(30, 0)));
    hs.server.players[slot].connected = false;
    assert_eq!(hs.server.stream_anchors(), vec![hs.server.spawn_column]);
}

/// Spec 02 §8.4 — an on-disk column is never generated over or touched by the
/// streamer. It has no disk path at all: boot loads the whole save through
/// `world_open` (all or nothing; a torn chunk file is kept aside there), and
/// streaming restores from the in-memory evicted store else generates. So a
/// `.chunk` file the session never read (here: one planted after boot, under a
/// column the streamer then loads and unloads, and a save then passes) stays
/// byte-for-byte as it was; streaming itself writes and deletes no file.
#[test]
fn streaming_a_column_in_and_out_never_touches_a_chunk_file_on_disk() {
    let _g = WorldsRootGuard::new("server_streaming_disk");
    let name = format!("server-streaming-disk-{}", std::process::id());
    let mut hs = start_dedicated(&name);

    let target = (21, 0);
    let chunks_dir = crate::save::world_dir(&name).join("chunks");
    std::fs::create_dir_all(&chunks_dir).expect("chunks dir");
    let planted = chunks_dir.join(format!("{}_4_{}.chunk", target.0, target.1));
    let planted_bytes = b"not a chunk: a column file the streamer must never read or replace".to_vec();
    std::fs::write(&planted, &planted_bytes).expect("plant a chunk file");
    let snapshot = || -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(&chunks_dir)
            .expect("list chunks dir")
            .map(|e| {
                let e = e.expect("dir entry");
                (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).expect("read"))
            })
            .collect();
        files.sort();
        files
    };
    let before = snapshot();

    let (client, slot) = join_guest(&mut hs, "Walker");
    let mut j = Journey { hs: &mut hs, client: &client, slot, tick: 0 };
    j.fly_to_x(target.0 as f32 * 16.0 + 8.0);
    assert!(j.hs.server.loaded_columns.contains(&target), "the column streamed in");
    j.land();
    j.fly_to_x(0.5);
    assert!(!j.hs.server.loaded_columns.contains(&target), "and streamed back out");
    assert_eq!(snapshot(), before, "streaming wrote and deleted nothing");

    // A save writes the world it holds (its own chunk files appear), but the
    // planted column was generated, never edited: nothing of it is persisted,
    // so its file is neither rewritten nor removed.
    j.hs.server.try_save().expect("server save");
    assert_eq!(std::fs::read(&planted).expect("still there"), planted_bytes);
}

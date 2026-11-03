//! Keep slice 3: a checkpoint that couldn't be restored is an error at
//! `checkpoint`, never a written image, by the constants the restore uses. Plus
//! the two bounds a run now holds as it goes, and a walk over what the fixture
//! can reach (Spec 01 §4.1.5).

use glam::Vec3;

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, SEED};
use super::authority_checkpoint_refusals::saved;
use super::twin_run::{add_joiner, run_to, twin, BASE_Y};
use crate::authority_checkpoint::{refuse_unrestorable, seal, section_offsets, CheckpointError, MAX_CHECKPOINT_BYTES};
use crate::commands::builtins::tp::MAX_COORD;
use crate::save::WorldsRootGuard;
use crate::server::{GameServer, ServerPlayer};
use crate::test_harness::TestHost;

const COUNT: usize = 4;
/// Bits 7..=24 are the sections a column can hold.
const FULL_COLUMN: u32 = 0x01FF_FF80;

fn host(tag: &str) -> (WorldsRootGuard, TestHost) {
    let root = WorldsRootGuard::new(tag);
    let h = fixture(tag, 1);
    (root, h)
}

#[track_caller]
fn assert_unrestorable(server: &GameServer, what: &str) {
    match server.checkpoint() {
        Err(CheckpointError::Unrestorable(_)) => {}
        other => panic!("{what}: expected Unrestorable, got {:?}", other.map(|b| b.len())),
    }
    assert!(server.state_hash().is_err(), "{what}: a hash of it");
}

// ---- Bounds on a body ----

#[test]
fn a_body_past_the_limit_is_an_error_at_checkpoint() {
    let (_root, mut h) = host("keep-s3-body-far");
    h.server.players[0].player.pos.x = MAX_COORD * 1.01;
    assert_unrestorable(&h.server, "a body past the limit");
    h.server.players[0].player.pos.x = 0.5;
    h.server.players[0].spawn_pos.z = -MAX_COORD * 1.01;
    assert_unrestorable(&h.server, "a spawn past the limit");
}

#[test]
fn a_body_that_is_not_finite_is_an_error_at_checkpoint() {
    let (_root, mut h) = host("keep-s3-body-nan");
    h.server.players[0].player.velocity.y = f32::NAN;
    assert_unrestorable(&h.server, "a velocity");
    h.server.players[0].player.velocity.y = 0.0;
    h.server.players[0].combat.health = f32::INFINITY;
    assert_unrestorable(&h.server, "a health");
}

/// At the limit it is fine, both ways.
#[test]
fn a_body_at_the_limit_checkpoints_and_restores() {
    let (_root, mut h) = host("keep-s3-body-at");
    run_to(&mut h, 5);
    h.server.players[0].player.pos = Vec3::new(MAX_COORD, 100.0, -MAX_COORD);
    h.server.players[0].player.velocity = Vec3::ZERO;
    h.server.players[0].spawn_pos = Vec3::new(-MAX_COORD, 100.0, MAX_COORD);
    let bytes = persist_and_checkpoint(&mut h);
    let back = restore(&h, &bytes).expect("restores");
    assert_eq!(back.players[0].player.pos, h.server.players[0].player.pos);
}

/// Live: nothing walks, rides or falls past the limit, and a tick that finds a
/// body there holds it at the wall.
#[test]
fn a_tick_holds_a_body_at_the_limit() {
    let (_root, mut h) = host("keep-s3-body-live");
    {
        let sp = &mut h.server.players[0];
        sp.player.pos = Vec3::new(MAX_COORD + 64.0, 100.0, -MAX_COORD - 64.0);
        sp.player.velocity = Vec3::new(5.0, 0.0, -5.0);
        sp.spawn_pos = Vec3::new(MAX_COORD + 64.0, 100.0, 0.5);
    }
    h.server.tick();
    let sp = &h.server.players[0];
    assert!(sp.player.pos.x.abs() <= MAX_COORD && sp.player.pos.z.abs() <= MAX_COORD, "{:?}", sp.player.pos);
    assert!(sp.spawn_pos.x.abs() <= MAX_COORD, "{:?}", sp.spawn_pos);
    assert_eq!((sp.player.velocity.x, sp.player.velocity.z), (0.0, 0.0), "stopped at the wall");
    h.server.checkpoint().expect("a held body checkpoints");
}

// ---- Bounds on a section ----

fn evicted_column(h: &mut TestHost) -> (i32, i32, i32) {
    let (cx, cy, cz) = (40, 6, 40);
    h.server.world.set_block(cx * 16, cy * 16, cz * 16, crate::block::STONE);
    assert!(h.server.world.evict_column(cx, cz), "the column holds something to keep");
    (cx, cy, cz)
}

#[test]
fn a_section_both_live_and_evicted_is_an_error_at_checkpoint() {
    let (_root, mut h) = host("keep-s3-both");
    let (cx, cy, cz) = evicted_column(&mut h);
    h.server.world.force_live_chunk(cx, cy, cz, crate::chunk::Chunk::new());
    assert_unrestorable(&h.server, "a section both live and evicted");
}

/// Live: a section has one home, and the evicted copy is the one that counts
/// (`World::persistable_chunks`).
#[test]
fn a_chunk_inserted_into_an_evicted_section_goes_nowhere() {
    let (_root, mut h) = host("keep-s3-both-live");
    let (cx, cy, cz) = evicted_column(&mut h);
    h.server.world.insert_chunk(cx, cy, cz, crate::chunk::Chunk::new());
    assert!(!h.server.world.has_chunk(cx, cy, cz), "no live copy");
    assert!(!h.server.world.persistable_chunk(cx, cy, cz).unwrap().is_empty(), "the evicted copy stands");
    h.server.checkpoint().expect("a section in one place checkpoints");
}

#[test]
fn a_column_past_the_range_is_an_error_at_checkpoint() {
    let (_root, mut h) = host("keep-s3-column-far");
    let far = MAX_COORD as i32 + 16 * 64 + 16;
    h.server.world.set_block(far, 100, 0, crate::block::STONE);
    assert_unrestorable(&h.server, "a chunk past the playable range");
}

/// The section cap, on bytes: a world at it holds 840 MB of chunks, too much to
/// build in a test on a shared machine, so this puts the entries through the same
/// gate `checkpoint` puts its own bytes through.
#[test]
fn the_section_cap_is_the_gates_at_exactly_the_restores_number() {
    let (_root, h, bytes) = saved("keep-s3-section-cap");
    let at = section_offsets(&h.server).unwrap();
    let with_sections = |full_columns: i32, last: u32| {
        let mut layout = Vec::new();
        layout.extend_from_slice(&((full_columns + 1) as u32).to_le_bytes());
        for x in 0..=full_columns {
            layout.extend_from_slice(&x.to_le_bytes());
            layout.extend_from_slice(&0i32.to_le_bytes());
            layout.push(0);
            layout.extend_from_slice(&(if x == full_columns { last } else { FULL_COLUMN }).to_le_bytes());
            layout.extend_from_slice(&0u32.to_le_bytes());
        }
        let mut out = bytes[..at.layout].to_vec();
        out.extend_from_slice(&layout);
        out.extend_from_slice(&bytes[at.sims..]);
        seal(&mut out);
        out
    };
    // 3,640 full columns are 65,520 sections: 16 more is the cap, 17 past it.
    let sixteen = FULL_COLUMN & (FULL_COLUMN >> 2);
    assert_eq!(sixteen.count_ones(), 16);
    let seventeen = FULL_COLUMN & (FULL_COLUMN >> 1);
    assert_eq!(seventeen.count_ones(), 17);
    refuse_unrestorable(&with_sections(3_640, sixteen)).expect("at the cap");
    match refuse_unrestorable(&with_sections(3_640, seventeen)) {
        Err(CheckpointError::Unrestorable(why)) => assert!(why.contains("more sections"), "{why}"),
        other => panic!("past the cap: {other:?}"),
    }
}

// ---- Every other bound the restore has ----

#[test]
fn the_restores_other_limits_are_errors_at_checkpoint() {
    let (_root, mut h) = host("keep-s3-others");
    let (distance, render, refill) = (
        h.server.render_distance,
        h.server.column_refill_per_tick,
        h.server.spawn_column,
    );
    h.server.render_distance = crate::graphics_settings::RENDER_DISTANCE_MAX + 1;
    assert_unrestorable(&h.server, "render distance");
    h.server.render_distance = distance;
    h.server.column_refill_per_tick = crate::server::HOST_COLUMN_REFILL_PER_TICK + 1;
    assert_unrestorable(&h.server, "column refill");
    h.server.column_refill_per_tick = render;
    h.server.spawn_column = (i32::MAX, 0);
    assert_unrestorable(&h.server, "spawn column");
    h.server.spawn_column = refill;
    let streamer = h.server.column_streamer.take();
    h.server.column_streamer = Some(crate::server_stream::ColumnStreamer::from_parts(i32::MAX, crate::server_stream::SERVER_STREAM_BUDGET, None));
    assert_unrestorable(&h.server, "sim distance");
    h.server.column_streamer = streamer;
    h.server.fire.restore_cells(&[((i32::MAX, 100, 0), 10, 0)]);
    assert_unrestorable(&h.server, "a burning cell past the range");
    h.server.fire.restore_cells(&[]);
    h.server.checkpoint().expect("all put back");
    let wide = u8::MAX as usize + 3;
    let players = std::mem::take(&mut h.server.players);
    h.server.players = (0..wide).map(|_| ServerPlayer::new(Vec3::new(0.5, 100.0, 0.5))).collect();
    assert_unrestorable(&h.server, "more slots than a server holds");
    h.server.players = players;
}

// ---- The size cap, to the byte ----

/// Filled to exactly the cap a checkpoint is written and restored; a byte past
/// it is an error. Fire cells for the bulk and a name for the last few bytes.
#[test]
fn a_checkpoint_of_exactly_the_cap_restores_and_one_byte_more_is_too_large() {
    let _root = WorldsRootGuard::new("keep-s3-exact-cap");
    let mut h = twin(SEED, "ckpt-exact-cap", 1, 2);
    run_to(&mut h, 2);
    h.server.fire.restore_cells(&[]);
    h.server.try_save().expect("saves");
    let room = MAX_CHECKPOINT_BYTES - h.server.checkpoint().unwrap().len();
    const FIRE_CELL: usize = 12 + 8 + 1;
    let cells: Vec<_> = (0..(room / FIRE_CELL) as i32).map(|i| ((i, 100, 0), 1_000_000u64, 0u8)).collect();
    h.server.fire.restore_cells(&cells);
    h.server.players[0].display_name.push_str(&"x".repeat(room % FIRE_CELL));
    let bytes = h.server.checkpoint().expect("exactly the cap");
    assert_eq!(bytes.len(), MAX_CHECKPOINT_BYTES);
    let back = restore(&h, &bytes).expect("restores");
    assert_eq!(back.checkpoint().expect("again"), bytes);
    h.server.players[0].display_name.push('x');
    match h.server.checkpoint() {
        Err(CheckpointError::TooLarge { bytes }) => assert_eq!(bytes, MAX_CHECKPOINT_BYTES + 1),
        other => panic!("one byte past the cap: {:?}", other.map(|b| b.len())),
    }
}

/// The manifest reaches the cap the same way: an error, not a shorter list.
#[test]
fn a_manifest_past_the_cap_is_too_large() {
    let _root = WorldsRootGuard::new("keep-s3-manifest-cap");
    let h = twin(SEED, "ckpt-manifest-cap", 0, 2);
    let name = |i: usize| std::path::PathBuf::from(format!("npub{i:0>60}.dat"));
    for i in 0..(MAX_CHECKPOINT_BYTES / (COUNT + 1 + 68 + 32) + 1) {
        h.server.stored_records.note_joiner(&name(i), b"a record");
    }
    assert!(matches!(h.server.checkpoint(), Err(CheckpointError::TooLarge { .. })));
}

// ---- Anything the fixture can reach ----

/// A small deterministic generator: no new dependency, and a failure names its seed.
struct Walk(u64);
impl Walk {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self.0.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(23) ^ 0x2545_F491_4F6C_DD1D;
        (self.0 >> 16) % below
    }
}

/// One thing a world's life can do, chosen by `walk`.
fn act(h: &mut TestHost, walk: &mut Walk) {
    let (x, z) = (walk.next(40) as i32 - 20, walk.next(40) as i32 - 20);
    let y = BASE_Y + 1 + walk.next(6) as i32;
    let tick = h.server.tick_counter;
    match walk.next(10) {
        0 => h.set_block(x, y, z, crate::block::STONE),
        1 => h.set_block(x, y, z, crate::block::AIR),
        2 => { let _ = h.server.fire.ignite(&mut h.server.world, x, y, z, tick); }
        3 => h.set_block(x, y, z, crate::block::WATER),
        4 => h.set_block(x, y, z, crate::block::LAVA),
        5 => {
            let joiner = h.server.players.iter().position(|sp| sp.server_simulated);
            if let Some(j) = joiner {
                let far = [MAX_COORD * 1.5, MAX_COORD, -MAX_COORD, 300.0][walk.next(4) as usize];
                h.teleport_player(j, Vec3::new(far, 120.0, 40.5));
            }
        }
        6 => {
            let kept = h.server.world.persistable_chunks().next().map(|(k, _)| k);
            if let Some((cx, _, cz)) = kept {
                h.server.world.evict_column(cx, cz);
            }
        }
        7 => {
            let at = Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5);
            let cow = crate::entity::spawn_mob(&mut h.server.ecs, crate::mob::MobType::Cow, at);
            if walk.next(2) == 0 {
                h.server.ecs.get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
            }
        }
        8 => { add_joiner(h, Vec3::new(x as f32 + 0.5, 110.0, z as f32 + 0.5)); }
        _ => {}
    }
    let to = h.server.tick_counter + 1 + walk.next(8);
    run_to(h, to);
}

/// Whatever the walk reached: a checkpoint (dead mobs and all), and after a
/// persist, one that restores to its own bytes.
#[test]
fn any_state_the_fixtures_actions_reach_checkpoints_and_restores() {
    for seed in 1..=4u64 {
        let tag = format!("keep-s3-walk-{seed}");
        let _root = WorldsRootGuard::new(&tag);
        let mut h = fixture(&tag, 1);
        let mut walk = Walk(seed);
        for step in 0..16 {
            act(&mut h, &mut walk);
            h.server.checkpoint().unwrap_or_else(|e| panic!("seed {seed} step {step}: {e:?}"));
            if step % 4 == 3 {
                let bytes = persist_and_checkpoint(&mut h);
                let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("seed {seed} step {step}: {e:?}"));
                assert_eq!(back.checkpoint().expect("again"), bytes, "seed {seed} step {step}");
            }
        }
    }
}

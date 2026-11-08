//! Keep slice 2b — bytes changed by someone who also fixed the seal. A seal is a
//! checksum, so these pass it: each is refused by what reads the bytes, and most
//! before the world is touched. Those tests delete the world's folder first: a
//! layout that got as far as the load would allocate what it names, and there is
//! no load without a folder, so the refusal's wording shows which step refused.

use glam::Vec3;

use super::authority_checkpoint::{restore, SEED};
use super::authority_checkpoint_refusals::{assert_invalid_checkpoint, saved};
use super::twin_run::twin;
use crate::authority_checkpoint::{layout_record, rebind_layout, rebind_state, seal, section_offsets, state_record, RestoreError, CHECKPOINT_MAGIC};
use crate::save::WorldsRootGuard;
use crate::server::ServerPlayer;
use crate::test_harness::TestHost;

const COUNT: usize = 4;
/// One layout entry: column x and z, flags, then the two masks.
const COLUMN_ENTRY: usize = 17;
const LIVE_MASK: usize = 9;
const EVICTED_MASK: usize = 13;
/// Bits 7..=24 are the sections a column can hold (`-1..=MAX_CHUNK_Y + 1`).
const COLUMN_SECTIONS: u32 = 0x01FF_FF80;

/// Within the modes section (`volatile::write_modes`).
const REFILL: usize = 3;
const RENDER_DISTANCE: usize = 9;
const PLAY_MODE: usize = 13;
/// Within the streamer section, past its option tag (`volatile::write_streamer`).
const SIM_DISTANCE: usize = 1;
const BUDGET: usize = 5;
/// Within the players record, in the first body (`players::write_body`), which
/// has no landing pending.
const BODY_POS: usize = COUNT;
const BODY_VELOCITY: usize = COUNT + 12;
const BODY_SPAWN: usize = COUNT + 12 + 12 + 3 + 4 + 4 + 1 + 4 + 4;

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn delete_the_world(h: &TestHost) {
    std::fs::remove_dir_all(crate::save::world_dir(&h.server.world_name)).unwrap();
}

/// Reseal, and require a refusal whose reason contains `needle`.
fn assert_refused_for(h: &TestHost, mut bytes: Vec<u8>, what: &str, needle: &str) {
    seal(&mut bytes);
    match restore(h, &bytes).err() {
        Some(RestoreError::InvalidCheckpoint(why)) => {
            assert!(why.contains(needle), "{what}: refused, but for this: {why}");
        }
        other => panic!("{what}: expected InvalidCheckpoint, got {other:?}"),
    }
}

/// The checkpoint with its layout record replaced by `columns` entries, all live in
/// `live`, and bound: the digests all vouch for it, so what refuses it is its reader.
fn with_layout(h: &TestHost, bytes: &[u8], columns: i32, live: u32) -> Vec<u8> {
    let mut layout = Vec::with_capacity(COUNT + columns as usize * COLUMN_ENTRY);
    layout.extend_from_slice(&(columns as u32).to_le_bytes());
    for x in 0..columns {
        layout.extend_from_slice(&x.to_le_bytes());
        layout.extend_from_slice(&0i32.to_le_bytes());
        layout.push(0);
        layout.extend_from_slice(&live.to_le_bytes());
        layout.extend_from_slice(&0u32.to_le_bytes());
    }
    rebind_layout(&h.server, bytes, layout)
}

/// This server's layout record with `edit` made to it, and bound.
fn with_edited_layout(h: &TestHost, bytes: &[u8], edit: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut layout = layout_record(&h.server);
    edit(&mut layout);
    rebind_layout(&h.server, bytes, layout)
}

#[test]
fn a_section_no_column_holds_is_refused_before_the_load() {
    let (_root, h, bytes) = saved("keep-s2b-layout-bits");
    let first = COUNT;
    for bit in [0u32, 6, 25, 31] {
        for mask in [LIVE_MASK, EVICTED_MASK] {
            let hostile = with_edited_layout(&h, &bytes, |layout| {
                let was = read_u32(layout, first + mask);
                put(layout, first + mask, &(was | 1 << bit).to_le_bytes());
            });
            assert_refused_for(&h, hostile, &format!("bit {bit} of mask {mask}"), "outside a column");
        }
    }
}

/// The review's scenario: every bit legal, and tens of gigabytes of chunks if they were
/// built. Refused by the reader, before the load allocates any.
#[test]
fn a_layout_past_the_section_cap_is_refused_before_the_load() {
    let (_root, h, bytes) = saved("keep-s2b-layout-cap");
    assert_eq!(COLUMN_SECTIONS.count_ones(), 18);
    // Just past the section cap, and far past it.
    for columns in [14_565, 100_000] {
        let hostile = with_layout(&h, &bytes, columns, COLUMN_SECTIONS);
        assert_refused_for(&h, hostile, &format!("{columns} full columns"), "more sections");
    }
}

/// The save keeps one copy of a section, so a checkpoint can't name two.
#[test]
fn a_section_both_live_and_evicted_is_refused_before_the_load() {
    let (_root, h, bytes) = saved("keep-s2b-layout-both");
    let first = COUNT;
    let hostile = with_edited_layout(&h, &bytes, |layout| {
        let live = read_u32(layout, first + LIVE_MASK);
        assert_ne!(live, 0, "the fixture's first column holds a section");
        put(layout, first + EVICTED_MASK, &live.to_le_bytes());
    });
    assert_refused_for(&h, hostile, "both masks", "both live and evicted");
}

/// Far enough out, a column's first cell overflows an `i32`.
#[test]
fn a_column_past_the_playable_range_is_refused_before_the_load() {
    let (_root, h, bytes) = saved("keep-s2b-layout-far");
    let hostile = with_edited_layout(&h, &bytes, |layout| put(layout, COUNT, &i32::MIN.to_le_bytes()));
    assert_refused_for(&h, hostile, "column x", "column outside");
}

/// `is_creative` follows the body's play mode, so the body has to agree with the
/// binding in the header.
#[test]
fn an_edited_play_mode_under_a_survival_config_is_refused() {
    let (_root, h, bytes) = saved("keep-s2b-play-mode");
    assert!(!h.server.is_creative);
    let at = section_offsets(&h.server).unwrap().modes + PLAY_MODE + COUNT;
    assert_eq!(&bytes[at..at + 8], b"survival");
    let mut edited = bytes.clone();
    put(&mut edited, at, b"creative");
    assert_refused_for(&h, edited, "creative body, survival config", "canonical form");
}

/// A flipped bit is damage whichever field it lands in. A newer build's sealed
/// bytes are still named as such.
#[test]
fn the_seal_is_read_before_the_version_fields() {
    let (_root, h, bytes) = saved("keep-s2b-versions");
    let protocol = CHECKPOINT_MAGIC.len() + 2;
    for field in [protocol, protocol + 4] {
        let mut newer = bytes.clone();
        newer[field + 3] ^= 0x40;
        assert_invalid_checkpoint(&h, &newer, "a flipped bit in a version field");
        seal(&mut newer);
        match restore(&h, &newer).err() {
            Some(RestoreError::UnsupportedVersion(_)) => {}
            other => panic!("sealed bytes from a newer build: got {other:?}"),
        }
    }
}

#[test]
fn a_sim_distance_budget_or_refill_past_its_limit_is_refused() {
    let (_root, h, bytes) = saved("keep-s2b-streamer");
    let at = section_offsets(&h.server).unwrap();
    assert!(h.server.column_streamer.is_some());
    delete_the_world(&h);
    for value in [i32::MAX, i32::MIN, 1, 17] {
        let mut hostile = bytes.clone();
        put(&mut hostile, at.streamer + SIM_DISTANCE, &value.to_le_bytes());
        assert_refused_for(&h, hostile, &format!("sim distance {value}"), "sim distance");
    }
    for value in [0u32, 3, u32::MAX] {
        let mut hostile = bytes.clone();
        put(&mut hostile, at.streamer + BUDGET, &value.to_le_bytes());
        assert_refused_for(&h, hostile, &format!("budget {value}"), "budget");
    }
    for value in [3u32, u32::MAX] {
        let mut hostile = bytes.clone();
        put(&mut hostile, at.modes + REFILL, &value.to_le_bytes());
        assert_refused_for(&h, hostile, &format!("refill {value}"), "refill");
    }
}

#[test]
fn a_render_distance_or_spawn_column_past_its_limit_is_refused() {
    let (_root, h, bytes) = saved("keep-s2b-modes");
    let at = section_offsets(&h.server).unwrap();
    delete_the_world(&h);
    for value in [-1, 17, i32::MAX] {
        let mut hostile = bytes.clone();
        put(&mut hostile, at.modes + RENDER_DISTANCE, &value.to_le_bytes());
        assert_refused_for(&h, hostile, &format!("render distance {value}"), "render distance");
    }
    let spawn = at.modes + PLAY_MODE + COUNT + b"survival".len();
    for (axis, value) in [(0, i32::MAX), (0, i32::MIN), (4, i32::MAX), (4, 1 << 20)] {
        let mut hostile = bytes.clone();
        put(&mut hostile, spawn + axis, &value.to_le_bytes());
        assert_refused_for(&h, hostile, &format!("spawn column axis {axis}: {value}"), "spawn column");
    }
}

#[test]
fn a_body_that_is_not_finite_or_out_of_range_is_refused() {
    let (_root, h, bytes) = saved("keep-s2b-body");
    assert!(h.server.players[0].player.pending_landing.is_none());
    let cases = [
        ("position x", BODY_POS, f32::NAN),
        ("position y", BODY_POS + 4, f32::INFINITY),
        ("position z", BODY_POS + 8, 1_000_001.0),
        ("velocity x", BODY_VELOCITY, f32::NAN),
        ("spawn x", BODY_SPAWN, f32::NEG_INFINITY),
        ("spawn z", BODY_SPAWN + 8, -1_000_001.0),
    ];
    for (what, field, value) in cases {
        let mut players = state_record(&h.server, b'u');
        put(&mut players, field, &value.to_le_bytes());
        assert_refused_for(&h, rebind_state(&h.server, &bytes, b'u', players), what, "a body");
    }
}

/// Not reachable by editing a count: the slots have to be there to pass the
/// reader's own length check. So these are real slots, past what a server holds.
#[test]
fn more_players_than_a_server_holds_are_refused() {
    let _root = WorldsRootGuard::new("keep-s2b-players");
    let mut h = twin(SEED, "ckpt-players", 0, 2);
    while h.server.players.len() < 258 {
        let mut sp = ServerPlayer::new(Vec3::new(0.5, 110.0, 0.5));
        sp.server_simulated = true;
        h.server.players.push(sp);
    }
    h.server.persist_authority().expect("the slots are stored");
    let bytes = h.server.checkpoint_ungated().expect("the image holds no slot");
    assert_refused_for(&h, bytes, "258 slots", "more players");
}

/// What a first tick would do arithmetic on: a settled anchor and a queued cell
/// near the edge of an `i32`, and a lava phase past its cycle.
#[test]
fn values_the_first_tick_would_overflow_on_are_refused() {
    let _root = WorldsRootGuard::new("keep-s2b-overflow");
    let mut h = twin(SEED, "ckpt-overflow", 0, 2);

    let streamer = h.server.column_streamer.take();
    h.server.column_streamer = Some(crate::server_stream::ColumnStreamer::from_parts(2, 2, Some(vec![(i32::MAX, 0)])));
    let bytes = h.server.checkpoint_ungated().unwrap();
    assert_refused_for(&h, bytes, "a settled anchor", "anchor");
    h.server.column_streamer = streamer;

    h.server.fire.restore_cells(&[((i32::MAX, 110, 0), 1_000_000, 0)]);
    h.server.persist_authority().expect("the cell is stored");
    let bytes = h.server.checkpoint_ungated().unwrap();
    assert_refused_for(&h, bytes, "a burning cell", "cell");
    h.server.fire.restore_cells(&[]);

    h.server.lava.set_slow_for_test(255);
    let bytes = h.server.checkpoint_ungated().unwrap();
    assert_refused_for(&h, bytes, "lava's phase", "lava");
}

/// These three were refused already; the first paths past the seal to pin it.
#[test]
fn a_resealed_trailing_byte_or_cut_is_invalid() {
    let (_root, h, bytes) = saved("keep-s2b-resealed-shape");
    let end = bytes.len() - 32;
    let mut longer = bytes.clone();
    longer.insert(end, 0);
    assert_refused_for(&h, longer, "one byte more", "left over");
    let mut shorter = bytes.clone();
    shorter.remove(end - 1);
    assert_refused_for(&h, shorter, "one byte less", "end early");
    let mut half = bytes[..bytes.len() / 2].to_vec();
    half.extend_from_slice(&[0; 32]);
    seal(&mut half);
    assert_invalid_checkpoint(&h, &half, "cut in half");
}

#[test]
fn a_resealed_minus_zero_is_not_canonical() {
    let _root = WorldsRootGuard::new("keep-s2b-resealed-zero");
    let mut h = twin(SEED, "ckpt-zero-reseal", 1, 2);
    super::authority_checkpoint::sign_in(&mut h);
    h.server.players[0].player.velocity = Vec3::ZERO;
    h.server.persist_authority().unwrap();
    let bytes = h.server.checkpoint().unwrap();
    let mut players = state_record(&h.server, b'u');
    assert_eq!(players[BODY_VELOCITY..BODY_VELOCITY + 4], [0; 4]);
    put(&mut players, BODY_VELOCITY, &(-0.0f32).to_le_bytes());
    assert_refused_for(&h, rebind_state(&h.server, &bytes, b'u', players), "-0.0", "canonical form");
}

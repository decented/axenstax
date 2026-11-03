//! Keep slice 3, ruling 1: the lava queue's records don't shift as it drains, so what a
//! tick changes of them is small and a `last` batch fits at any queue length (Tier-L
//! rulings, section 4 item 3).

use std::collections::{BTreeMap, BTreeSet};

use fray_session::authority::{validate_persist_batch, PersistBatch, StoredLimits, StoredRecord, MAX_PERSIST_BATCH_BYTES, MAX_PERSIST_BATCH_ENTRIES, MAX_STORED_KEY_BYTES};

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, settle, SEED};
use super::twin_run::{run_to, twin_loaded};
use crate::authority_checkpoint::{rebind_entries, record_file, seal, section_offsets, RecordSet, RestoreError, MAX_CHECKPOINT_BYTES};
use crate::block::{AIR, LAVA};
use crate::save::{world_dir, WorldsRootGuard};
use crate::test_harness::TestHost;

const PIECE_ENTRIES: usize = 1024;
const SPREAD_ENTRY_BYTES: usize = 13;
const RETRACT_ENTRY_BYTES: usize = 12;
const N: u64 = PIECE_ENTRIES as u64;
const LIMITS: StoredLimits = StoredLimits { key_bytes: MAX_STORED_KEY_BYTES, batch_bytes: MAX_PERSIST_BATCH_BYTES, batch_entries: MAX_PERSIST_BATCH_ENTRIES };

/// Spread entries for cells that hold nothing, so each drains without flowing.
fn wakes(n: i32, y: i32) -> Vec<(i32, i32, i32, u8)> {
    (0..n).map(|i| (i, y, i % 7, 1u8)).collect()
}

/// `n` queued wakes of cells that hold nothing.
fn queue_wakes(h: &mut TestHost, n: i32) {
    h.server.lava.queue_for_test(&wakes(n, 300), &[]);
}

fn store_of(set: &RecordSet) -> BTreeMap<Vec<u8>, Vec<u8>> {
    set.records.iter().map(|r| (r.key.clone(), r.value.clone())).collect()
}

/// What of `set` the store doesn't hold yet.
fn dirty<'a>(set: &'a RecordSet, store: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<&'a StoredRecord> {
    set.records.iter().filter(|r| store.get(&r.key) != Some(&r.value)).collect()
}

fn is_lava(key: &[u8]) -> bool {
    matches!(key[0], b'q' | b'r' | b'x')
}

fn family_bytes(records: &[&StoredRecord], family: u8) -> usize {
    records.iter().filter(|r| r.key[0] == family).map(|r| r.value.len()).sum()
}

fn piece_key(family: u8, number: u32) -> Vec<u8> {
    let mut key = vec![family];
    key.extend_from_slice(&number.to_be_bytes());
    key
}

fn section_key((cx, cy, cz): (i32, i32, i32)) -> Vec<u8> {
    let mut key = vec![b'x'];
    for v in [cx, cy, cz] {
        key.extend_from_slice(&v.to_be_bytes());
    }
    key
}

fn step(h: &mut TestHost) {
    let next = h.server.tick_counter + 1;
    run_to(h, next);
}

/// How many entries the spread and the retract queue have held since they last stood empty.
fn pushed(h: &TestHost) -> (u64, u64) {
    let (spread, retract) = (h.server.lava.spread_queue(), h.server.lava.retract_queue());
    (spread.head() + spread.iter().len() as u64, retract.head() + retract.iter().len() as u64)
}

/// A long queue of real lava, its cursor `head` entries in: each entry that drains flows
/// down a cell and queues that cell, so the queue drains and grows as lava really does.
fn flowing_world(h: &mut TestHost, head: u64) -> Vec<(i32, i32, i32)> {
    let mut columns: Vec<_> = h.server.loaded_columns.iter().copied().collect();
    columns.sort_unstable();
    let cells: Vec<(i32, i32, i32)> = columns
        .iter()
        .flat_map(|&(cx, cz)| (0..4).flat_map(move |a| (0..4).map(move |b| (cx * 16 + 4 * a, 130, cz * 16 + 4 * b))))
        .filter(|&(x, y, z)| h.server.world.is_column_present_at(x, z) && h.server.world.get_block(x, y, z) == AIR && h.server.world.get_block(x, y - 1, z) == AIR)
        .take(400)
        .collect();
    assert!(cells.len() >= 200, "{} cells", cells.len());
    for &(x, y, z) in &cells {
        h.set_block(x, y, z, LAVA);
        h.server.lava.add_source(x, y, z);
    }
    // These alone, not the world's own wakes before them, so they flow in the ticks a test runs.
    let queued: Vec<_> = cells.iter().map(|&(x, y, z)| (x, y, z, 0u8)).collect();
    h.server.lava.queue_at_head_for_test(head, (0, 300, 0, 1), &queued);
    cells
}

/// One tick's dirty lava is its last piece, whatever the queue holds.
#[test]
fn a_tick_dirties_one_piece_of_a_long_queue() {
    let _root = WorldsRootGuard::new("keep-s3e-dirty");
    let mut h = fixture("ckpt-lava-dirty", 2);
    run_to(&mut h, 5);
    queue_wakes(&mut h, 100_000);
    let mut set = h.server.build_record_set(None, &Default::default()).expect("builds");
    // Fluids run one tick in four and lava moves on one of their three calls, so 24 ticks
    // hold two drains.
    for _ in 0..24 {
        let store = store_of(&set);
        let next = h.server.tick_counter + 1;
        run_to(&mut h, next);
        set = h.server.build_record_set(None, &set.keys()).expect("builds");
        let bytes: usize = dirty(&set, &store).iter().filter(|r| is_lava(&r.key)).map(|r| r.value.len()).sum();
        assert!(bytes <= PIECE_ENTRIES * SPREAD_ENTRY_BYTES, "tick {next}: {bytes} B of lava dirty");
    }
    assert!(h.server.lava.digest_view().1.len() < 100_000, "the queue drained, so the comparison wasn't idle");
}

/// After the first commit of a worst-case load, each tick's dirty set is a legal batch, so
/// lava never has `last` withheld, and the load still stays under the cap and restores.
#[test]
fn a_worst_case_queue_leaves_a_tick_a_legal_batch() {
    let _root = WorldsRootGuard::new("keep-s3e-worst");
    let mut h = fixture("ckpt-lava-worst", 8);
    run_to(&mut h, 5);
    queue_wakes(&mut h, 540_000);
    let mut set = h.server.build_record_set(None, &Default::default()).expect("builds");
    for _ in 0..24 {
        let store = store_of(&set);
        let next = h.server.tick_counter + 1;
        run_to(&mut h, next);
        set = h.server.build_record_set(None, &set.keys()).expect("builds");
        let batch = PersistBatch { records: dirty(&set, &store).into_iter().cloned().collect(), removed: set.removed.clone(), last: true };
        validate_persist_batch(&batch, LIMITS).unwrap_or_else(|e| panic!("tick {next}: {e:?}"));
    }
    assert!(h.server.lava.digest_view().1.len() < 540_000, "the queue drained, so the comparison wasn't idle");
    let bytes = persist_and_checkpoint(&mut h);
    assert!(bytes.len() < MAX_CHECKPOINT_BYTES / 4, "{} B", bytes.len());
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
}

/// A real commit is 1,200 ticks apart. Over one, the queue drains by a few pieces: those
/// are removed, and the rest of a worst-case queue isn't in the batch at all.
#[test]
fn a_commit_interval_of_a_long_drain_only_removes_the_pieces_it_passed() {
    let _root = WorldsRootGuard::new("keep-s3e-interval");
    let mut h = fixture("ckpt-lava-interval", 2);
    run_to(&mut h, 5);
    queue_wakes(&mut h, 540_000);
    settle(&mut h);
    let set = h.server.build_record_set(None, &Default::default()).expect("builds");
    let store = store_of(&set);
    let before = pushed(&h);
    let to = h.server.tick_counter + 1200;
    run_to(&mut h, to);
    settle(&mut h);
    let appended = pushed(&h).0 - before.0;
    let next = h.server.build_record_set(None, &set.keys()).expect("builds");
    let changed = dirty(&next, &store);
    let (q, r, x) = (family_bytes(&changed, b'q'), family_bytes(&changed, b'r'), family_bytes(&changed, b'x'));
    eprintln!("interval: {appended} appended; dirty q {q} B, r {r} B, x {x} B");
    assert!(q <= PIECE_ENTRIES * SPREAD_ENTRY_BYTES + appended as usize * SPREAD_ENTRY_BYTES, "{q} B of the spread queue, {appended} appended");
    let head = h.server.lava.spread_queue().head();
    assert!(head >= N * 2, "the queue drained past whole pieces: its cursor is {head}");
    let expected: Vec<Vec<u8>> = (0..(head / N) as u32).map(|i| piece_key(b'q', i)).collect();
    let removed: Vec<Vec<u8>> = next.removed.iter().filter(|key| key[0] == b'q').cloned().collect();
    assert_eq!(removed, expected, "the pieces wholly behind the cursor, and no others");
}

/// Over many ticks of lava really flowing, with a queue that crosses a piece edge and a
/// retraction: what is dirty of each queue is at most one piece and what it appended.
#[test]
fn over_many_ticks_lava_dirties_one_piece_and_what_it_appended() {
    let _root = WorldsRootGuard::new("keep-s3e-flow");
    let mut h = fixture("ckpt-lava-flow", 2);
    run_to(&mut h, 5);
    let cells = flowing_world(&mut h, N - 100);
    settle(&mut h);
    let mut set = h.server.build_record_set(None, &Default::default()).expect("builds");
    let (mut appended_total, mut edges, mut most_sources, mut retracted) = (0u64, 0usize, 0usize, 0usize);
    for at in 0..240 {
        // Dig some of the sources, so what they fed retracts.
        if at == 120 {
            for &(x, y, z) in cells.iter().take(100) {
                h.set_block(x, y, z, AIR);
                h.server.lava.remove_source(x, y, z);
            }
        }
        let store = store_of(&set);
        let before = pushed(&h);
        step(&mut h);
        settle(&mut h);
        let after = pushed(&h);
        set = h.server.build_record_set(None, &set.keys()).expect("builds");
        let changed = dirty(&set, &store);
        let appended = (after.0.saturating_sub(before.0), after.1.saturating_sub(before.1));
        appended_total += appended.0;
        let (q, r, x) = (family_bytes(&changed, b'q'), family_bytes(&changed, b'r'), family_bytes(&changed, b'x'));
        assert!(q <= PIECE_ENTRIES * SPREAD_ENTRY_BYTES + appended.0 as usize * SPREAD_ENTRY_BYTES, "tick {at}: {q} B of the spread queue, {} appended", appended.0);
        assert!(r <= PIECE_ENTRIES * RETRACT_ENTRY_BYTES + appended.1 as usize * RETRACT_ENTRY_BYTES, "tick {at}: {r} B of the retract queue, {} appended", appended.1);
        most_sources = most_sources.max(x);
        retracted += r;
        edges += set.removed.iter().filter(|key| key[0] == b'q').count();
    }
    eprintln!("flow: {appended_total} appended, {edges} pieces removed, at most {most_sources} B of sources dirty a tick, {retracted} B of retract queue written");
    assert!(most_sources <= 32 * 1024, "{most_sources} B of sources dirty in a tick");
    assert!(most_sources > 0 && retracted > 0, "lava flowed and retracted, so the comparison wasn't idle");
    assert!(appended_total > 100, "{appended_total} entries appended");
    assert!(edges >= 1, "the cursor crossed a piece's edge");
}

/// The drained pieces leave: `removed` names the key, a persist takes the file away, and the
/// pieces after it are the bytes they were.
#[test]
fn a_piece_the_cursor_has_passed_is_removed_and_the_rest_stay_as_they_were() {
    let _root = WorldsRootGuard::new("keep-s3e-edge");
    let mut h = fixture("ckpt-lava-edge", 1);
    run_to(&mut h, 5);
    h.server.lava.queue_at_head_for_test(N - 40, (0, 300, 0, 1), &wakes(N as i32 * 2 + 50, 300));
    settle(&mut h);
    let first = h.server.persist_authority_set().expect("persists");
    let dir = world_dir("ckpt-lava-edge");
    let stored = store_of(&first);
    assert!(record_file(&dir, &piece_key(b'q', 0)).exists());
    while h.server.lava.spread_queue().head() < N {
        step(&mut h);
    }
    settle(&mut h);
    let next = h.server.persist_authority_set().expect("persists");
    assert!(next.removed.contains(&piece_key(b'q', 0)), "{:?}", next.removed);
    assert!(!record_file(&dir, &piece_key(b'q', 0)).exists());
    let again = store_of(&next);
    assert_eq!(again[&piece_key(b'q', 1)], stored[&piece_key(b'q', 1)], "a full piece doesn't change");
    let bytes = h.server.checkpoint().expect("checkpoints");
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
}

/// Everything of lava a record set holds, by key.
fn lava_records(h: &TestHost) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let set = h.server.build_record_set(None, &Default::default()).expect("builds");
    set.records.into_iter().filter(|r| is_lava(&r.key)).map(|r| (r.key, r.value)).collect()
}

/// Ruling 2: restore-then-continue writes the same bytes an uninterrupted run writes,
/// across a piece's edge and a retraction, cursor included.
#[test]
fn a_restored_run_goes_on_writing_the_bytes_the_uninterrupted_run_writes() {
    let _root = WorldsRootGuard::new("keep-s3e-continue");
    let mut a = fixture("ckpt-lava-continue", 2);
    run_to(&mut a, 5);
    let cells = flowing_world(&mut a, N - 250);
    run_to(&mut a, 40);
    let bytes = persist_and_checkpoint(&mut a);
    let mut b = TestHost::adopt(restore(&a, &bytes).expect("restores"));
    assert_eq!(lava_records(&a), lava_records(&b), "straight after the restore");
    let head = a.server.lava.spread_queue().head();
    assert!(head < N, "still in its first piece: {head}");
    for (i, at) in [40u64, 80, 120, 200, 280].into_iter().enumerate() {
        let to = a.server.tick_counter + at;
        run_to(&mut a, to);
        run_to(&mut b, to);
        settle(&mut a);
        settle(&mut b);
        assert_eq!(a.server.tick_counter, b.server.tick_counter);
        assert_eq!(a.server.lava.spread_queue(), b.server.lava.spread_queue(), "the spread queue, tick {to}");
        assert_eq!(a.server.lava.retract_queue(), b.server.lava.retract_queue(), "the retract queue, tick {to}");
        assert_eq!(lava_records(&a), lava_records(&b), "lava's records, tick {to}");
        if i == 1 {
            for host in [&mut a, &mut b] {
                for &(x, y, z) in cells.iter().take(60) {
                    host.set_block(x, y, z, AIR);
                    host.server.lava.remove_source(x, y, z);
                }
            }
        }
    }
    let after = a.server.lava.spread_queue().head();
    assert!(after > N, "the comparison crossed a piece's edge: {head} to {after}");
    let records = lava_records(&a);
    assert!(records.keys().any(|key| key[0] == b'x'), "lava flowed, so the sources differ from a rescan");
}

/// Two authorities of one world, fed the same, write the same (ruling 4 item 4).
#[test]
fn twins_write_the_same_lava_records() {
    let _root = WorldsRootGuard::new("keep-s3e-twins");
    let mut a = fixture("ckpt-lava-twin-a", 1);
    let mut b = fixture("ckpt-lava-twin-b", 1);
    for h in [&mut a, &mut b] {
        run_to(h, 5);
        flowing_world(h, N - 60);
        run_to(h, 150);
        settle(h);
    }
    assert_eq!(a.server.tick_counter, b.server.tick_counter);
    assert!(a.server.lava.spread_queue().head() >= N, "crossed a piece's edge");
    assert_eq!(lava_records(&a), lava_records(&b));
    assert_eq!(a.server.checkpoint().expect("a"), b.server.checkpoint().expect("b"));
}

struct Mutable {
    key: Vec<u8>,
    good: Vec<u8>,
    older: Vec<u8>,
    swapped: Vec<u8>,
}

/// Each lava family, each of the four things a store can hold in place of a record: all
/// `invalid-checkpoint`, from a real restore, before anything reads them.
#[test]
fn a_lava_record_flipped_absent_swapped_or_older_is_refused() {
    let _root = WorldsRootGuard::new("keep-s3e-mutate");
    let mut h = fixture("ckpt-lava-mutate", 1);
    run_to(&mut h, 5);
    let retract = |y: i32| (0..N as i32 * 2 + 5).map(|i| (i, y, 0)).collect::<Vec<_>>();
    let (a, b) = ((1, 300, 1), (41, 300, 41));
    h.server.lava.add_source(a.0, a.1, a.2);
    h.server.lava.add_source(b.0, b.1, b.2);
    h.server.lava.queue_for_test(&wakes(N as i32 * 3 + 10, 300), &retract(300));
    settle(&mut h);
    h.server.persist_authority().expect("persists");
    let dir = world_dir("ckpt-lava-mutate");
    let read = |key: &[u8]| std::fs::read(record_file(&dir, key)).unwrap_or_else(|e| panic!("{}: {e}", hex::encode(key)));
    let keys = [piece_key(b'q', 0), piece_key(b'q', 1), piece_key(b'q', 3), piece_key(b'r', 0), piece_key(b'r', 2), section_key((0, 18, 0)), section_key((2, 18, 2))];
    let older: Vec<Vec<u8>> = keys.iter().map(|key| read(key)).collect();

    h.server.lava.add_source(a.0 + 1, a.1, a.2);
    h.server.lava.add_source(b.0 + 1, b.1, b.2);
    h.server.lava.queue_for_test(&wakes(N as i32 * 3 + 10, 301), &retract(301));
    let bytes = persist_and_checkpoint(&mut h);
    let cases: Vec<Mutable> = keys
        .iter()
        .zip(older)
        .enumerate()
        .map(|(i, (key, older))| {
            // The next record of the same family: another key's valid bytes.
            let other = &keys[if i + 1 < keys.len() && keys[i + 1][0] == key[0] { i + 1 } else { i - 1 }];
            Mutable { key: key.clone(), good: read(key), older, swapped: read(other) }
        })
        .collect();
    let mut wrong = BTreeMap::new();
    for case in &cases {
        let path = record_file(&dir, &case.key);
        assert_ne!(case.older, case.good, "{}: the store never held another value", hex::encode(&case.key));
        assert_ne!(case.swapped, case.good, "{}: the other key held these bytes", hex::encode(&case.key));
        let mut flipped = case.good.clone();
        flipped[2] ^= 1;
        for (name, there) in [("flipped", Some(&flipped)), ("absent", None), ("another key's", Some(&case.swapped)), ("older", Some(&case.older))] {
            match there {
                Some(held) => std::fs::write(&path, held).unwrap(),
                None => std::fs::remove_file(&path).unwrap(),
            }
            match restore(&h, &bytes).err() {
                Some(RestoreError::InvalidCheckpoint(_)) => {}
                other => {
                    wrong.insert(format!("{} {name}", hex::encode(&case.key)), format!("{other:?}"));
                }
            }
        }
        std::fs::write(&path, &case.good).unwrap();
    }
    assert!(wrong.is_empty(), "{} mutations not refused: {wrong:#?}", wrong.len());
    restore(&h, &bytes).unwrap_or_else(|e| panic!("all put back, it restores: {e:?}"));
}

/// A cursor the pieces don't agree with, resealed so the seal says nothing: refused by the
/// reader, as no run writes it. The first piece here is number 1, so piece 0 was removed.
#[test]
fn a_cursor_past_the_end_or_in_a_removed_piece_is_refused() {
    let _root = WorldsRootGuard::new("keep-s3e-cursor");
    let mut h = fixture("ckpt-lava-cursor", 1);
    run_to(&mut h, 5);
    let retract: Vec<_> = (0..2000).map(|i| (i, 300, 0)).collect();
    h.server.lava.queue_for_test(&[], &retract);
    let live = N as i32 * 2 + 10;
    h.server.lava.queue_at_head_for_test(N + 5, (0, 300, 0, 1), &wakes(live, 300));
    let bytes = persist_and_checkpoint(&mut h);
    let spread_at = section_offsets(&h.server).unwrap().sims + 1 + 8;
    let retract_at = spread_at + 8;
    let end = N + 5 + live as u64;
    assert_eq!(bytes[spread_at..spread_at + 8], (N + 5).to_le_bytes(), "the cursor is where this test thinks it is");
    assert_eq!(bytes[retract_at..retract_at + 8], 0u64.to_le_bytes());
    let cases: [(&str, usize, u64, &str); 7] = [
        ("past the end", spread_at, end + 1, "past the end"),
        ("on the end", spread_at, end, "past the end"),
        ("far past the end", spread_at, u64::MAX, "past the end"),
        ("in a removed piece", spread_at, 5, "isn't bound"),
        ("at the start of a removed piece", spread_at, 0, "isn't bound"),
        ("leaving a piece behind it", spread_at, N * 2 + 1, "wholly behind"),
        ("past the retract queue's end", retract_at, 2000, "past the end"),
    ];
    for (what, at, cursor, words) in cases {
        let mut crafted = bytes.clone();
        crafted[at..at + 8].copy_from_slice(&cursor.to_le_bytes());
        seal(&mut crafted);
        match restore(&h, &crafted).err() {
            Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains(words), "{what}: refused, but for this: {why}"),
            other => panic!("{what}: {other:?}"),
        }
    }
    restore(&h, &bytes).unwrap_or_else(|e| panic!("untouched, it restores: {e:?}"));

    // Pieces the tree binds with a hole in them.
    for (what, key, words) in [
        ("a gap", piece_key(b'q', 2), "piece by piece"),
        ("no first piece", piece_key(b'q', 1), "isn't bound"),
        ("no last piece", piece_key(b'q', 3), ""),
        ("no retract piece", piece_key(b'r', 0), ""),
    ] {
        let crafted = rebind_entries(&h.server, &bytes, |flat| flat.retain(|(k, _)| *k != key));
        match restore(&h, &crafted).err() {
            Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains(words), "{what}: refused, but for this: {why}"),
            other => panic!("{what}: {other:?}"),
        }
        h.server.write_bound_records().expect("the tree goes back");
    }
    restore(&h, &bytes).unwrap_or_else(|e| panic!("all put back, it restores: {e:?}"));
}

/// Plain greedy cutting of a first commit's records into batches that each pass the limits.
/// Not the batcher, which is the adapter's.
fn staged_batches(records: &[StoredRecord]) -> Vec<PersistBatch> {
    let mut batches = Vec::new();
    let mut batch = PersistBatch { records: Vec::new(), removed: Vec::new(), last: false };
    let mut bytes = 0usize;
    for record in records {
        let size = record.key.len() + record.value.len();
        if bytes + size > MAX_PERSIST_BATCH_BYTES || batch.records.len() == MAX_PERSIST_BATCH_ENTRIES {
            batches.push(std::mem::replace(&mut batch, PersistBatch { records: Vec::new(), removed: Vec::new(), last: false }));
            bytes = 0;
        }
        bytes += size;
        batch.records.push(record.clone());
    }
    batches.push(batch);
    batches
}

/// A world with a worst-case queue at its first commit: too large for a batch, so it goes
/// over staged batches a tick apart. The pieces don't change while that happens, so at the
/// `last` the store holds all of them but the one the queue's end touches, the set binds what
/// the store holds, and it restores for real.
#[test]
fn the_first_commit_of_a_huge_queue_spreads_over_staged_batches_and_binds_at_the_last() {
    let _root = WorldsRootGuard::new("keep-s3e-spread");
    let mut h = fixture("ckpt-lava-spread", 2);
    run_to(&mut h, 5);
    h.server.lava.queue_at_head_for_test(N - 40, (0, 300, 0, 1), &wakes(540_000, 300));
    settle(&mut h);
    let first = h.server.build_record_set(None, &BTreeSet::new()).expect("builds");
    let batches = staged_batches(&first.records);
    let lava_batches = batches.iter().filter(|b| b.records.iter().any(|r| is_lava(&r.key))).count();
    assert!(lava_batches >= 6, "{lava_batches} batches hold lava's pieces");
    let mut store: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    for batch in &batches {
        validate_persist_batch(batch, LIMITS).unwrap_or_else(|e| panic!("a staged batch: {e:?}"));
        store.extend(batch.records.iter().map(|r| (r.key.clone(), r.value.clone())));
        step(&mut h);
    }
    // Past the first piece while they went in, so a piece already staged has to come out.
    while h.server.lava.spread_queue().head() < N {
        step(&mut h);
    }
    settle(&mut h);
    let keys: BTreeSet<Vec<u8>> = store.keys().cloned().collect();
    let set = h.server.build_record_set(None, &keys).expect("builds");
    let changed = dirty(&set, &store);
    let lava_again = changed.iter().filter(|r| r.key[0] == b'q').count();
    assert!(lava_again <= 1, "the staged pieces stood, but for the one the queue's end touches: {lava_again}");
    assert!(set.removed.contains(&piece_key(b'q', 0)), "{:?}", set.removed);
    let last = PersistBatch { records: changed.into_iter().cloned().collect(), removed: set.removed.clone(), last: true };
    validate_persist_batch(&last, LIMITS).unwrap_or_else(|e| panic!("the last batch: {e:?}"));
    store.extend(last.records.iter().map(|r| (r.key.clone(), r.value.clone())));
    for key in &last.removed {
        store.remove(key);
    }
    assert_eq!(store, store_of(&set), "the store equals what the set binds");
    for (key, digest) in set.flat.iter().filter(|(key, _)| is_lava(key)) {
        assert_eq!(crate::authority_checkpoint::record_digest(&store[key]), *digest, "{}", hex::encode(key));
    }

    let persisted = h.server.persist_authority_set().expect("persists");
    assert_eq!(persisted.tree.top, set.tree.top, "the same state, the same root");
    let dir = world_dir("ckpt-lava-spread");
    for (key, value) in store.iter().filter(|(key, _)| is_lava(key)) {
        std::fs::write(record_file(&dir, key), value).unwrap();
    }
    let bytes = h.server.checkpoint().expect("checkpoints");
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores from the staged pieces: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
}

/// The most of lava a tick may dirty at the 8-joiner bench, in a batch of 1 MiB: a piece of
/// each queue, and a little for what a drain appends and the sources that moved.
const LAVA_SHARE_OF_A_TICK: usize = 32 * 1024;

/// What a tick dirties at the 8-joiner bench, family by family, and at a commit interval.
/// Run by name with `--ignored --nocapture`.
#[test]
#[ignore]
fn what_a_tick_dirties_at_the_eight_joiner_bench() {
    let name = "keep-s3e-bench";
    let _root = WorldsRootGuard::new(name);
    let mut h = twin_loaded(SEED, name, 0, super::tick_bench::SIM_DISTANCE, super::tick_bench::SIM_DISTANCE);
    super::tick_bench::add_bench_joiners(&mut h, 8);
    super::tick_bench::warm_up(&mut h);
    super::tick_bench::scripted_build(&mut h);
    settle(&mut h);
    let mut set = h.server.build_record_set(None, &BTreeSet::new()).expect("builds");
    let mut sizes: BTreeMap<char, (usize, usize)> = BTreeMap::new();
    for r in &set.records {
        let size = sizes.entry(r.key[0] as char).or_default();
        size.0 += 1;
        size.1 += r.value.len();
    }
    eprintln!("bench whole set, family: (records, bytes): {sizes:?}");
    let spread = h.server.lava.spread_queue();
    eprintln!("bench queue at the start: cursor {}, {} entries", spread.head(), spread.iter().len());
    let first_set = store_of(&set);
    let first_keys = set.keys();
    // (ticks dirtied, most in a tick, all of them).
    let mut tally: BTreeMap<char, (usize, usize, usize)> = BTreeMap::new();
    let (mut most_lava, mut most_batch) = (0usize, 0usize);
    const TICKS: usize = 48;
    for _ in 0..TICKS {
        let store = store_of(&set);
        step(&mut h);
        settle(&mut h);
        set = h.server.build_record_set(None, &set.keys()).expect("builds");
        let changed = dirty(&set, &store);
        let mut this: BTreeMap<char, usize> = BTreeMap::new();
        for r in &changed {
            *this.entry(r.key[0] as char).or_default() += r.value.len();
        }
        for (family, bytes) in this {
            let t = tally.entry(family).or_default();
            t.0 += 1;
            t.1 = t.1.max(bytes);
            t.2 += bytes;
        }
        let lava: usize = changed.iter().filter(|r| is_lava(&r.key)).map(|r| r.value.len()).sum();
        assert!(lava <= LAVA_SHARE_OF_A_TICK, "{lava} B of lava in a tick");
        most_lava = most_lava.max(lava);
        most_batch = most_batch.max(changed.iter().map(|r| r.key.len() + r.value.len()).sum());
        let batch = PersistBatch { records: changed.into_iter().cloned().collect(), removed: set.removed.clone(), last: true };
        validate_persist_batch(&batch, LIMITS).unwrap_or_else(|e| panic!("a tick's batch: {e:?}"));
    }
    eprintln!("bench {TICKS} ticks, family: (ticks dirtied, most in a tick, total): {tally:?}");
    eprintln!("bench most lava in a tick {most_lava} B; most of any tick's batch {most_batch} B, of {MAX_PERSIST_BATCH_BYTES}");
    let to = h.server.tick_counter + 1200;
    run_to(&mut h, to);
    settle(&mut h);
    let next = h.server.build_record_set(None, &first_keys).expect("builds");
    let changed = dirty(&next, &first_set);
    let mut this: BTreeMap<char, (usize, usize)> = BTreeMap::new();
    for r in &changed {
        let t = this.entry(r.key[0] as char).or_default();
        t.0 += 1;
        t.1 += r.value.len();
    }
    let bytes: usize = changed.iter().map(|r| r.key.len() + r.value.len()).sum();
    let removed = next.removed.iter().map(|k| k[0] as char).collect::<String>();
    let spread = h.server.lava.spread_queue();
    eprintln!("bench interval of {} ticks, family: (records, bytes): {this:?}; {bytes} B in all; removed {removed:?}; cursor {}, {} entries", TICKS + 1200, spread.head(), spread.iter().len());
}

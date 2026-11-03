//! Keep slice 3: `world.dat` is bound one digest a fixed piece of its stored bytes (the
//! Tier-L rulings' addendum, section 2). The file stays whole where a save writes it.
//! Every piece is verified before anything reads the file, and each case here is
//! `invalid-checkpoint` from a real restore, naming the piece.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore};
use super::authority_checkpoint_records::fixture_with_meta;
use super::twin_run::run_to;
use crate::authority_checkpoint::{manifest_entries, rebind_entries, record_file, CheckpointError, RestoreError, FILE_PIECE_BYTES};
use crate::save::{read_world_save, world_dir, SavedSign, WorldSave, WorldsRootGuard};
use crate::save_format::encode_world_save;
use crate::server::GameServer;
use crate::test_harness::TestHost;

const PIECE: usize = FILE_PIECE_BYTES;

/// A mutated file, and what its refusal must say: (case, the file's bytes, the words).
type Case = (String, Vec<u8>, String);

/// Every way a file can differ from its bound pieces. `older` is the file as it once was,
/// one row different, and the same length.
fn piece_mutations(good: &[u8], older: &[u8]) -> Vec<Case> {
    let n = good.len().div_ceil(PIECE);
    assert!(n >= 3, "{} B is too few pieces to swap and cut", good.len());
    assert_eq!(good.len(), older.len(), "an older value of the same length");
    let piece = |bytes: &[u8], i: usize| bytes[i * PIECE..((i + 1) * PIECE).min(bytes.len())].to_vec();
    let of = |what: String| format!("world.dat piece {what}");
    let flipped = |at: usize| {
        let mut bytes = good.to_vec();
        bytes[at] ^= 1;
        bytes
    };
    let stale_piece = (0..n).find(|&i| piece(good, i) != piece(older, i)).expect("the older file differs");
    assert!(stale_piece >= 1, "the older value lies in the first piece");
    let mut stale = good.to_vec();
    stale[stale_piece * PIECE..(stale_piece * PIECE) + piece(older, stale_piece).len()].copy_from_slice(&piece(older, stale_piece));
    assert_eq!(stale, older, "the older value differs in one piece only");
    let mut swapped = piece(good, 1);
    swapped.extend_from_slice(&piece(good, 0));
    swapped.extend_from_slice(&good[2 * PIECE..]);
    assert_eq!(swapped.len(), good.len());
    let longer = |by: usize| {
        let mut bytes = good.to_vec();
        bytes.resize(good.len() + by, 0);
        bytes
    };
    let past_the_end = if good.len().is_multiple_of(PIECE) { of(format!("{n} is not in the manifest")) } else { of(format!("{} doesn't match", n - 1)) };
    vec![
        ("a flipped first piece".into(), flipped(2), of("0 doesn't match".into())),
        ("a flipped middle piece".into(), flipped(PIECE + 2), of("1 doesn't match".into())),
        ("a flipped last piece".into(), flipped(good.len() - 1), of(format!("{} doesn't match", n - 1))),
        ("short by exactly one piece".into(), good[..(n - 1) * PIECE].to_vec(), of(format!("{} is missing", n - 1))),
        ("short by two pieces".into(), good[..(n - 2) * PIECE].to_vec(), of(format!("{} is missing", n - 2))),
        ("short by a byte".into(), good[..good.len() - 1].to_vec(), of(format!("{} doesn't match", n - 1))),
        ("cut inside a middle piece".into(), good[..PIECE + 5].to_vec(), of("1 doesn't match".into())),
        ("emptied".into(), Vec::new(), of("0 doesn't match".into())),
        ("two pieces swapped".into(), swapped, of("0 doesn't match".into())),
        ("a stale piece".into(), stale, of(format!("{stale_piece} doesn't match"))),
        ("longer by a byte".into(), longer(1), past_the_end.clone()),
        ("longer by a piece".into(), longer(PIECE), past_the_end),
    ]
}

/// Each case, written in place of the file, is refused by `restore` for its own reason.
fn assert_every_case_is_refused(cases: &[Case], file: &Path, good: &[u8], restore: impl Fn() -> Result<GameServer, RestoreError>) {
    let mut wrong = BTreeMap::new();
    for (case, bytes, words) in cases {
        std::fs::write(file, bytes).unwrap();
        match restore().err() {
            Some(RestoreError::InvalidCheckpoint(why)) if why.contains(words) => {}
            other => {
                wrong.insert(case.clone(), format!("wanted \"{words}\", got {other:?}"));
            }
        }
    }
    std::fs::write(file, good).unwrap();
    assert!(wrong.is_empty(), "{} of {} cases wrong: {wrong:#?}", wrong.len(), cases.len());
}

fn sign(i: usize, chars: usize) -> SavedSign {
    SavedSign { x: i as i32, y: 300, z: 5, data: crate::sign::SignData { text: "x".repeat(chars) } }
}

/// A sign row past its text: its place and the text's length.
const SIGN_ROW: usize = 12 + 8;

/// Real rows up to exactly `pieces` full pieces, so that a piece past the end is a piece
/// of its own and two can be swapped whole.
fn save_of_whole_pieces(mut save: WorldSave, pieces: usize) -> WorldSave {
    let full = crate::sign::SIGN_MAX_CHARS + SIGN_ROW;
    let target = pieces * PIECE;
    let mut next = 0;
    loop {
        let len = encode_world_save(&save).expect("encodes").len();
        assert!(len <= target, "the base save is already past {target} B: {len} B");
        let room = target - len;
        if room == 0 {
            return save;
        }
        assert!(room >= SIGN_ROW, "{room} B is too little for a row");
        if room >= 2 * full {
            for _ in 0..room / full - 1 {
                save.signs.push(sign(next, crate::sign::SIGN_MAX_CHARS));
                next += 1;
            }
        } else if room > full {
            // Two rows' worth, each with room for a row: the next pass ends it.
            save.signs.push(sign(next, room / 2 - SIGN_ROW));
        } else {
            save.signs.push(sign(next, room - SIGN_ROW));
        }
        next += 1;
    }
}

/// A fixture whose `world.dat` is exactly `pieces` pieces, bound and restoring.
fn world_of_pieces(name: &str, pieces: usize) -> (TestHost, Vec<u8>, Vec<u8>) {
    let mut h = fixture(name, 1);
    run_to(&mut h, 20);
    persist_and_checkpoint(&mut h);
    let file = world_dir(name).join("world.dat");
    let save = save_of_whole_pieces(read_world_save(&std::fs::read(&file).unwrap()).expect("a save"), pieces);
    let good = encode_world_save(&save).expect("encodes");
    assert_eq!(good.len(), pieces * PIECE);
    std::fs::write(&file, &good).unwrap();
    h.server.stored_records.note_world_dat(name);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("checkpoints");
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("untouched, it restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
    (h, bytes, good)
}

/// `world.dat` as it once was: one sign's text, in the piece after the first.
fn older_world_dat(good: &[u8]) -> Vec<u8> {
    let mut save = read_world_save(good).expect("reads");
    let row = PIECE / (crate::sign::SIGN_MAX_CHARS + SIGN_ROW) + 40;
    let was = &mut save.signs[row].data.text;
    *was = "y".repeat(was.len());
    encode_world_save(&save).expect("encodes")
}

#[test]
fn each_piece_of_world_dat_is_bound_by_its_own_digest() {
    let _root = WorldsRootGuard::new("keep-s3d-bound");
    let (h, _, good) = world_of_pieces("ckpt-pieces-bound", 3);
    let entries = manifest_entries(&h.server).unwrap();
    let bound: Vec<_> = entries.iter().filter(|(key, _)| key[0] == b'w').collect();
    assert_eq!(bound.len(), 3);
    for (i, (key, digest)) in bound.iter().map(|e| (&e.0, &e.1)).enumerate() {
        assert_eq!(key.len(), 5, "{}", hex::encode(key));
        assert_eq!(key[1..], (i as u32).to_be_bytes());
        let expected: [u8; 32] = Sha256::digest(&good[i * PIECE..(i + 1) * PIECE]).into();
        assert_eq!(*digest, expected, "piece {i}");
    }
    let held = std::fs::read_dir(world_dir("ckpt-pieces-bound").join("authority")).unwrap();
    assert!(held.flatten().all(|f| !f.file_name().to_string_lossy().starts_with("77")), "no second copy of the file");
}

#[test]
fn every_way_world_dat_can_differ_from_its_pieces_is_refused() {
    let _root = WorldsRootGuard::new("keep-s3d-mutations");
    let (h, bytes, good) = world_of_pieces("ckpt-pieces", 4);
    let file = world_dir("ckpt-pieces").join("world.dat");
    let cases = piece_mutations(&good, &older_world_dat(&good));
    assert_every_case_is_refused(&cases, &file, &good, || restore(&h, &bytes));
    std::fs::remove_file(&file).unwrap();
    match restore(&h, &bytes).err() {
        Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains("world.dat piece 0 is missing"), "{why}"),
        other => panic!("the file absent: {other:?}"),
    }
    std::fs::write(&file, &good).unwrap();
    restore(&h, &bytes).unwrap_or_else(|e| panic!("all put back, it restores: {e:?}"));
}

/// The defined form of an empty file is one empty piece, so it isn't the same as no file.
#[test]
fn an_empty_world_dat_binds_one_empty_piece() {
    let _root = WorldsRootGuard::new("keep-s3d-empty");
    let mut h = fixture("ckpt-pieces-empty", 1);
    persist_and_checkpoint(&mut h);
    let file = world_dir("ckpt-pieces-empty").join("world.dat");
    std::fs::write(&file, b"").unwrap();
    h.server.stored_records.note_world_dat("ckpt-pieces-empty");
    let entries = manifest_entries(&h.server).unwrap();
    let bound: Vec<_> = entries.iter().filter(|(key, _)| key[0] == b'w').collect();
    let empty: [u8; 32] = Sha256::digest(b"").into();
    assert_eq!(bound.len(), 1);
    assert_eq!((bound[0].0.as_slice(), bound[0].1), (&b"w\0\0\0\0"[..], empty));
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("checkpoints");
    std::fs::remove_file(&file).unwrap();
    match restore(&h, &bytes).err() {
        Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains("world.dat piece 0 is missing"), "{why}"),
        other => panic!("the file gone: {other:?}"),
    }
}

/// A file this size is one key and digest in the record tree a piece, so the checkpoint
/// and its tree stay as they are in size, and the normal save is what wrote it.
#[test]
fn a_normal_save_writes_world_dat_whole_and_nowhere_else() {
    let _root = WorldsRootGuard::new("keep-s3d-whole");
    let mut h = fixture("ckpt-pieces-whole", 1);
    run_to(&mut h, 5);
    h.server.try_save().expect("saves");
    let dir = world_dir("ckpt-pieces-whole");
    let saved = std::fs::read(dir.join("world.dat")).unwrap();
    read_world_save(&saved).expect("whole and readable");
    assert!(!dir.join("authority").exists(), "a save writes no piece");
}

// ---- A checkpoint that can't be restored is never written ----

fn assert_cant_bind(h: &TestHost, what: &str) {
    assert!(matches!(h.server.checkpoint(), Err(CheckpointError::Unrestorable(_))), "{what}: {:?}", h.server.checkpoint().map(|b| b.len()));
    assert!(h.server.state_hash().is_err(), "{what}: a hash of it");
    assert!(h.server.write_bound_records().is_err(), "{what}: a persist of it");
}

/// In the file's place: a folder, then a link to itself, neither of which reads.
fn assert_each_failure_is_an_error(h: &TestHost, file: &Path, note: impl Fn()) {
    let good = std::fs::read(file).unwrap();
    std::fs::remove_file(file).unwrap();
    std::fs::create_dir(file).unwrap();
    note();
    assert_cant_bind(h, "a folder");
    std::fs::remove_dir(file).unwrap();
    std::os::unix::fs::symlink(file, file).unwrap();
    note();
    assert_cant_bind(h, "a link to itself");
    std::fs::remove_file(file).unwrap();
    std::fs::write(file, good).unwrap();
    note();
    h.server.checkpoint().expect("put right, it checkpoints");
}

#[test]
fn a_world_dat_that_cant_be_read_is_an_error_at_checkpoint() {
    let _root = WorldsRootGuard::new("keep-s3d-note-dat");
    let mut h = fixture("ckpt-note-dat", 1);
    persist_and_checkpoint(&mut h);
    let file = world_dir("ckpt-note-dat").join("world.dat");
    assert_each_failure_is_an_error(&h, &file, || h.server.stored_records.note_world_dat("ckpt-note-dat"));
}

#[test]
fn a_meta_that_cant_be_read_is_an_error_at_checkpoint() {
    let _root = WorldsRootGuard::new("keep-s3d-note-meta");
    let mut h = fixture_with_meta("ckpt-note-meta");
    persist_and_checkpoint(&mut h);
    let file = world_dir("ckpt-note-meta").join("world_meta.json");
    assert_each_failure_is_an_error(&h, &file, || h.server.stored_records.note_meta("ckpt-note-meta"));
}

/// Where a record is legally absent, its absence is no failure: no meta, no `handover/`,
/// and no `world.dat` before the first persist, which a restore then refuses as such.
#[test]
fn a_file_that_is_legally_absent_is_no_failure() {
    let _root = WorldsRootGuard::new("keep-s3d-note-absent");
    let h = fixture("ckpt-note-absent", 1);
    let dir = world_dir("ckpt-note-absent");
    assert!(!dir.join("handover").exists() && !dir.join("world_meta.json").exists() && !dir.join("world.dat").exists());
    h.server.stored_records.note_meta("ckpt-note-absent");
    h.server.stored_records.note_world_dat("ckpt-note-absent");
    h.server.stored_records.note_handover("ckpt-note-absent");
    h.server.checkpoint().expect("nothing failed");
}

// ---- Pieces of the state records ----

/// A key of a state family.
fn state_key(family: u8, i: u32) -> Vec<u8> {
    let mut key = vec![family];
    key.extend_from_slice(&i.to_be_bytes());
    key
}

/// Every state family at three pieces or more: the reader's bytes are real, from real
/// state, at a small cost on disk.
fn world_of_long_records(name: &str) -> (TestHost, Vec<u8>) {
    let mut h = fixture(name, 1);
    run_to(&mut h, 5);
    const N: i32 = 15_000;
    for i in 0..N {
        // 17 B a column, 25 B a water source and its front, 20 B a clock, 13 B a powered cell.
        h.server.loaded_columns.insert((i + 1000, 0));
        h.server.water.add_source(i, 300, 0);
        h.server.lava.add_source(i, 301, 0);
        h.server.world.pop_exposure.insert((i, 302, 0), 9);
        h.server.world.power.energised.insert((i, 303, 0), 3);
    }
    let burning: Vec<_> = (0..N).map(|i| ((i, 305, 0), 1_000_000u64, 0u8)).collect();
    h.server.fire.restore_cells(&burning);
    let mut plan: crate::plan::PlanData = bincode::deserialize(&[0u8; 1 << 12]).expect("zeros decode as a plan");
    plan.cells = vec![bincode::deserialize(&[0u8; 1 << 12]).expect("zeros decode as a cell"); 32_768];
    let mut carrier = crate::server::ServerPlayer::new(glam::Vec3::new(0.5, 110.0, 0.5));
    carrier.server_simulated = true;
    carrier.inventory.set_slot(35, Some(crate::item::ItemStack { item: crate::item::Item::Plan(plan), count: 1 }));
    h.server.players.push(carrier);
    h.server.persist_authority().expect("persists");
    for i in 0..6_000 {
        let cow = crate::entity::spawn_mob(&mut h.server.ecs, crate::mob::MobType::Cow, glam::Vec3::new(i as f32, 304.0, 0.5));
        h.server.ecs.get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
    }
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("checkpoints");
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("untouched, it restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
    (h, bytes)
}

/// Each family, an absent middle piece and pieces the tree binds with a gap: the digests
/// can't speak for these, as a gap's list is one a persist never writes. All refused.
#[test]
fn every_state_family_refuses_an_absent_middle_piece_and_non_contiguous_pieces() {
    let _root = WorldsRootGuard::new("keep-s3d-families");
    let (h, bytes) = world_of_long_records("ckpt-families");
    let dir = world_dir("ckpt-families");
    let entries = manifest_entries(&h.server).unwrap();
    let expect = |crafted: &[u8], case: String, words: &str, wrong: &mut BTreeMap<String, String>| match restore(&h, crafted).err() {
        Some(RestoreError::InvalidCheckpoint(why)) if why.contains(words) => {}
        other => {
            wrong.insert(case, format!("wanted \"{words}\", got {other:?}"));
        }
    };
    let mut wrong = BTreeMap::new();
    for (family, name) in [(b'l', "layout"), (b'q', "lava"), (b'p', "exposure"), (b'f', "water"), (b's', "cell sims"), (b'u', "players"), (b'k', "corpses")] {
        let pieces = entries.iter().filter(|(key, _)| key.len() == 5 && key[0] == family).count() as u32;
        assert!(pieces >= 3, "{name}: {pieces} pieces");

        let middle = record_file(&dir, &state_key(family, 1));
        let good = std::fs::read(&middle).unwrap();
        std::fs::remove_file(&middle).unwrap();
        expect(&bytes, format!("{name}: an absent middle piece"), "piece 1 is missing", &mut wrong);
        std::fs::write(&middle, &good).unwrap();

        let first = record_file(&dir, &state_key(family, 0));
        let held = std::fs::read(&first).unwrap();
        std::fs::write(&first, &good).unwrap();
        std::fs::write(&middle, &held).unwrap();
        expect(&bytes, format!("{name}: two pieces swapped"), "piece 0 doesn't match", &mut wrong);
        std::fs::write(&first, &held).unwrap();
        std::fs::write(&middle, &good).unwrap();

        let without = |gone: u32| rebind_entries(&h.server, &bytes, |flat| flat.retain(|(key, _)| *key != state_key(family, gone)));
        expect(&without(1), format!("{name}: a gap in the middle"), "piece by piece", &mut wrong);
        expect(&without(0), format!("{name}: no first piece"), "piece by piece", &mut wrong);
        expect(&without(pieces - 1), format!("{name}: no last piece"), "", &mut wrong);
        let renumbered = rebind_entries(&h.server, &bytes, |flat| {
            flat.retain(|(key, _)| *key != state_key(family, 1));
            let digest = flat.iter().find(|(key, _)| *key == state_key(family, 2)).expect("a third piece").1;
            flat.push((state_key(family, pieces + 5), digest));
            flat.sort_unstable();
        });
        expect(&renumbered, format!("{name}: a piece past the end"), "piece by piece", &mut wrong);
        h.server.write_bound_records().expect("the tree goes back");
    }
    assert!(wrong.is_empty(), "{} cases wrong: {wrong:#?}", wrong.len());
    restore(&h, &bytes).unwrap_or_else(|e| panic!("all put back, it restores: {e:?}"));
}

/// A gap in the pieces bound of `world.dat` isn't a tree a persist writes either: the file
/// has a piece the manifest doesn't name.
#[test]
fn a_gap_in_world_dats_bound_pieces_is_refused() {
    let _root = WorldsRootGuard::new("keep-s3d-w-gap");
    let (h, bytes, _) = world_of_pieces("ckpt-w-gap", 3);
    let crafted = rebind_entries(&h.server, &bytes, |flat| flat.retain(|(key, _)| *key != state_key(b'w', 1)));
    match restore(&h, &crafted).err() {
        Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains("world.dat piece 1 is not in the manifest"), "{why}"),
        other => panic!("a gap: {other:?}"),
    }
    let crafted = rebind_entries(&h.server, &bytes, |flat| flat.retain(|(key, _)| key[0] != b'w'));
    match restore(&h, &crafted).err() {
        Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains("no world.dat is bound"), "{why}"),
        other => panic!("no pieces: {other:?}"),
    }
}

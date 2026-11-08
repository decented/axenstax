//! Keep slice 3, ruling 1: the record set a persist makes is built from memory by one pure
//! function (`GameServer::build_record_set`), and the save and the authority's persist
//! both go through the same encoding. These tests hold the builder to the disk: what a
//! persist writes to `authority/`, to `world.dat`, to the chunk files and to the joiners'
//! sidecars is what it returns, and a save writes what the plain encoder gives.

use std::collections::BTreeSet;

use super::authority_checkpoint::{fixture, persist_and_checkpoint};
use super::twin_run::run_to;
use crate::authority_checkpoint::{manifest_entries, record_file, RecordSet, FILE_PIECE_BYTES};
use crate::save::{world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::test_harness::TestHost;

fn plain_world_dat(server: &GameServer) -> Vec<u8> {
    let mobs = crate::save::tamed_mobs_to_saved(&server.ecs).expect("no death pending");
    server.encode_world_dat(server.selected_local_personal.as_ref(), mobs).expect("encodes")
}

fn value<'a>(set: &'a RecordSet, key: &[u8]) -> Option<&'a [u8]> {
    set.records.iter().find(|r| r.key == key).map(|r| r.value.as_slice())
}

fn chunk_key(cx: i32, cy: i32, cz: i32) -> Vec<u8> {
    let mut key = vec![b'c'];
    for n in [cx, cy, cz] {
        key.extend_from_slice(&n.to_le_bytes());
    }
    key
}

fn seed_lava_records(h: &mut TestHost) {
    // A source differing from a rescan and a pending retraction exercise both families.
    h.server.lava.add_source(1, 300, 1);
    h.server.lava.queue_for_test(&[(1, 300, 1, 0)], &[(2, 300, 2)]);
}

/// A save writes `world.dat` and every chunk file as the plain encoder gives them, the
/// first time and after.
#[test]
fn a_save_writes_what_the_plain_encoder_gives() {
    let _root = WorldsRootGuard::new("keep-s3d-plain");
    let mut h = fixture("ckpt-plain", 1);
    for tick in [5, 12] {
        run_to(&mut h, tick);
        while crate::sim_death::has_pending(&h.server.ecs) {
            let next = h.server.tick_counter + 1;
            run_to(&mut h, next);
        }
        h.server.try_save().expect("saves");
        let dir = world_dir("ckpt-plain");
        assert_eq!(std::fs::read(dir.join("world.dat")).unwrap(), plain_world_dat(&h.server), "world.dat at tick {tick}");
        let mut seen = 0;
        for ((cx, cy, cz), chunk) in h.server.world.persistable_chunks().filter(|(_, c)| !c.is_empty()) {
            let file = std::fs::read(dir.join("chunks").join(format!("{cx}_{cy}_{cz}.chunk"))).unwrap();
            assert_eq!(file, chunk.as_bytes(), "chunk ({cx}, {cy}, {cz}) at tick {tick}");
            seen += 1;
        }
        assert!(seen > 0);
    }
}

/// After a persist the builder's set is the disk: `world.dat` in its pieces, each chunk,
/// each present sidecar, the records in `authority/`, and the list the checkpoint binds.
#[test]
fn the_builders_set_is_what_a_persist_writes() {
    let _root = WorldsRootGuard::new("keep-s3d-equal");
    let mut h = fixture("ckpt-equal", 2);
    for tick in [5, 12] {
        run_to(&mut h, tick);
        seed_lava_records(&mut h);
        persist_and_checkpoint(&mut h);
        let dir = world_dir("ckpt-equal");
        let set = h.server.build_record_set(None, &h.server.stored_records.bound_keys()).expect("builds");
        // `world.dat`
        let pieces: Vec<&[u8]> = set.records.iter().filter(|r| r.key.len() == 5 && r.key[0] == b'w').map(|r| r.value.as_slice()).collect();
        assert_eq!(pieces.concat(), std::fs::read(dir.join("world.dat")).unwrap(), "world.dat at tick {tick}");
        assert!(pieces.iter().all(|p| p.len() <= FILE_PIECE_BYTES));
        // chunks
        let mut chunks = 0;
        for ((cx, cy, cz), chunk) in h.server.world.persistable_chunks().filter(|(_, c)| !c.is_empty()) {
            assert_eq!(value(&set, &chunk_key(cx, cy, cz)), Some(std::fs::read(dir.join("chunks").join(format!("{cx}_{cy}_{cz}.chunk"))).unwrap().as_slice()));
            chunks += 1;
        }
        assert_eq!(set.records.iter().filter(|r| r.key.first() == Some(&b'c')).count(), chunks);
        // sidecars
        let joiners: Vec<_> = set.records.iter().filter(|r| r.key.first() == Some(&b'j')).collect();
        assert!(!joiners.is_empty(), "the fixture's joiners are signed in");
        for r in &joiners {
            let name = String::from_utf8(r.key[1..].to_vec()).unwrap();
            assert_eq!(std::fs::read(dir.join("joiners").join(name)).unwrap(), r.value);
        }
        // authority/: exactly the state pieces and the tree
        let held: BTreeSet<_> = std::fs::read_dir(dir.join("authority")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        let stored: Vec<_> = set.records.iter().filter(|r| matches!(r.key[0], b'l' | b'q' | b'r' | b'x' | b'p' | b'f' | b's' | b'u' | b'k' | b't')).collect();
        for family in [b'r', b'x'] {
            assert!(stored.iter().any(|r| r.key[0] == family && !r.value.is_empty()), "the fixture exercises {}", family as char);
        }
        assert_eq!(held, stored.iter().map(|r| hex::encode(&r.key)).collect::<BTreeSet<_>>(), "the files are the set");
        for r in stored {
            assert_eq!(std::fs::read(record_file(&dir, &r.key)).unwrap(), r.value);
        }
        // what the checkpoint binds
        assert_eq!(set.flat, manifest_entries(&h.server).unwrap(), "the same list the checkpoint binds");
        assert!(set.removed.is_empty(), "{:?}", set.removed);
    }
}

fn grow_signs(h: &mut TestHost, rows: usize) {
    for i in 0..rows {
        let data = crate::sign::SignData { text: "x".repeat(crate::sign::SIGN_MAX_CHARS) };
        h.server.world.insert_sign((i as i32, 300, 5), data);
    }
}

/// Keys leave the store when a file shrinks, a chunk goes and a sidecar is no longer bound.
#[test]
fn removed_is_the_previous_bound_set_less_this_one() {
    let _root = WorldsRootGuard::new("keep-s3d-removed");
    let mut h = fixture("ckpt-removed", 1);
    run_to(&mut h, 5);
    grow_signs(&mut h, 4_000);
    persist_and_checkpoint(&mut h);
    let before = h.server.stored_records.bound_keys();
    let old_pieces = before.iter().filter(|k| k.len() == 5 && k[0] == b'w').count() as u32;
    assert!(old_pieces >= 3, "{old_pieces} pieces is too few to shrink");

    // A shrink of `w`, a dropped chunk and a sidecar nothing binds now.
    h.server.world.block_entities.clear();
    let dropped = before.iter().find(|k| k.first() == Some(&b'c') && k.len() == 13).cloned().expect("a chunk is bound");
    let at = |i: usize| i32::from_le_bytes([dropped[i], dropped[i + 1], dropped[i + 2], dropped[i + 3]]);
    h.server.world.remove_chunk(at(1), at(5), at(9));
    let mut previous = before.clone();
    let gone = b"jnpub1gone.dat".to_vec();
    previous.insert(gone.clone());

    let set = h.server.build_record_set(None, &previous).expect("builds");
    let new_world_dat = plain_world_dat(&h.server);
    let shrunk = crate::authority_checkpoint::file_piece_batch(Some(&new_world_dat), old_pieces);
    assert!(!shrunk.removes.is_empty(), "the file shrank");
    for key in &shrunk.removes {
        assert!(set.removed.contains(key), "{key:?} should go");
    }
    assert!(set.removed.contains(&dropped), "the dropped chunk goes");
    assert!(set.removed.contains(&gone), "the unbound sidecar goes");
    let bound = set.keys();
    let expected: Vec<_> = previous.difference(&bound).cloned().collect();
    assert_eq!(set.removed, expected, "exactly the previous keys this set doesn't bind");
    assert!(set.removed.iter().all(|k| !bound.contains(k)));
}

/// No key but a sidecar's and a manifest record's carries a persona, and every key is of a
/// family the schema names.
#[test]
fn the_keys_are_of_known_families_and_only_two_carry_a_persona() {
    let _root = WorldsRootGuard::new("keep-s3d-keys");
    let mut h = fixture("ckpt-keys", 2);
    run_to(&mut h, 5);
    seed_lava_records(&mut h);
    persist_and_checkpoint(&mut h);
    let set = h.server.build_record_set(None, &BTreeSet::new()).expect("builds");
    let personas: Vec<[u8; 32]> = h.server.players.iter().filter_map(|sp| sp.verified_pubkey).collect();
    assert!(!personas.is_empty());
    for key in set.keys() {
        assert!(b"cdhjlmqrxpfstuwk".contains(&key[0]), "{key:?}");
        if matches!(key[0], b'j' | b't') {
            continue;
        }
        let text = String::from_utf8_lossy(&key);
        assert!(!text.contains("npub"), "{key:?} names a persona");
        for pk in &personas {
            assert!(!key.windows(32).any(|w| w == pk), "{key:?} holds a persona's key");
        }
    }
}

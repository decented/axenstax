//! Keep slice 3: the checkpoint's size doesn't follow the world's. The manifest, the
//! layout, the lava queue and each cell's exposure clock are bound stored records, so
//! each of them can grow past what the 1 MiB image could hold (Spec 01 §4.1.5).

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, SEED};
use super::twin_run::{run_to, twin};
use crate::authority_checkpoint::{commit_and_read_bound, layout_sections_read, section_sizes, tree_shape, MAX_CHECKPOINT_BYTES, MAX_RECORD_BYTES};
use crate::fluids::FluidCheckpoint;
use crate::save::{world_dir, WorldsRootGuard};
use crate::world::LAYOUT_COLUMN_SECTIONS;

/// The manifest is one digest in the image however many records it binds. The sidecars
/// are digests only, as the size is what this is about: a restore needs their files.
#[test]
fn the_manifest_is_one_digest_whatever_the_world_holds() {
    let _root = WorldsRootGuard::new("keep-s3c-manifest");
    let h = twin(SEED, "ckpt-manifest", 0, 2);
    let small = h.server.checkpoint().expect("a small world checkpoints").len();
    let name = |i: usize| std::path::PathBuf::from(format!("npub{i:0>60}.dat"));
    // Past the old ceiling of about 21,000 entries.
    for i in 0..25_000 {
        h.server.stored_records.note_joiner(&name(i), b"a record");
    }
    let large = h.server.checkpoint().expect("past what an inline manifest could hold").len();
    assert_eq!(large, small, "the image didn't change size");
}

/// Worst lava inside one drain: a queue far past the largest the bench saw (about
/// 54,000 entries, at eight joiners on a 511-column world). The fixture has eight
/// joiners; a real 511-column world is the `#[ignore]` size bench in
/// `authority_checkpoint.rs`, which asserts the same thing at the real sizes.
#[test]
fn a_worst_case_lava_load_with_eight_joiners_stays_under_the_cap() {
    let _root = WorldsRootGuard::new("keep-s3c-lava");
    let mut h = fixture("ckpt-lava", 8);
    run_to(&mut h, 5);
    let spread = (0..500_000).map(|i| (i, 300, i % 7, 1u8)).collect();
    let queued = FluidCheckpoint { extra: Vec::new(), missing: Vec::new(), spread, retract: Vec::new() };
    let (world, loaded) = (&h.server.world, &h.server.loaded_columns);
    h.server.lava.restore_checkpoint(world, loaded, &queued, 0);
    let bytes = persist_and_checkpoint(&mut h);
    assert!(bytes.len() < MAX_CHECKPOINT_BYTES / 4, "{} B", bytes.len());
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
}

/// Past the 50,000-odd cells the image had room for, its values untouched.
#[test]
fn exposure_clocks_past_the_old_ceiling_commit_and_restore() {
    let _root = WorldsRootGuard::new("keep-s3c-exposure");
    let mut h = fixture("ckpt-exposure-big", 1);
    run_to(&mut h, 5);
    for i in 0..100_000 {
        h.server.world.pop_exposure.insert((i, 20, 0), u64::from(i as u32) * 3);
    }
    let bytes = persist_and_checkpoint(&mut h);
    assert!(bytes.len() < MAX_CHECKPOINT_BYTES / 4, "{} B", bytes.len());
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.world.pop_exposure, h.server.world.pop_exposure);
    assert_eq!(back.checkpoint().expect("again"), bytes);
}

/// A real world past 65,536 sections holds about 840 MB of chunks, built twice (the
/// run's and the restore's), too much for a test on a shared machine, and its files
/// would be about 600 MB of disk. So this runs a layout in the writer's form and a
/// manifest entry per section, without chunk bodies, through the real pieces, tree,
/// commit and walk, and the real reader that `checkpoint` and `restore` share. The
/// end-to-end path is the same code at the smaller sizes of the other tests.
#[test]
fn a_world_past_65536_sections_commits_and_restores_its_layout_and_manifest() {
    let _root = WorldsRootGuard::new("keep-s3c-sections");
    let dir = world_dir("ckpt-sections");
    std::fs::create_dir_all(&dir).unwrap();
    let columns = 3_900i32;
    assert_eq!(LAYOUT_COLUMN_SECTIONS.count_ones(), 18);
    let mut layout = Vec::new();
    layout.extend_from_slice(&(columns as u32).to_le_bytes());
    for x in 0..columns {
        layout.extend_from_slice(&x.to_le_bytes());
        layout.extend_from_slice(&0i32.to_le_bytes());
        layout.push(0);
        layout.extend_from_slice(&LAYOUT_COLUMN_SECTIONS.to_le_bytes());
        layout.extend_from_slice(&0u32.to_le_bytes());
    }
    assert_eq!(layout_sections_read(&layout), Ok(70_200));

    let mut chunks = Vec::new();
    for x in 0..columns {
        for bit in 0..u32::BITS {
            if LAYOUT_COLUMN_SECTIONS & (1 << bit) != 0 {
                let mut key = vec![b'c'];
                key.extend_from_slice(&x.to_le_bytes());
                key.extend_from_slice(&(bit as i32).to_le_bytes());
                key.extend_from_slice(&0i32.to_le_bytes());
                chunks.push((key, [bit as u8; 32]));
            }
        }
    }
    let (sections, walked, tree) = commit_and_read_bound(&dir, layout, chunks.clone()).expect("commits and reads back");
    assert_eq!(sections, 70_200);
    chunks.sort_unstable();
    let bound_chunks: Vec<_> = walked.iter().filter(|(key, _)| key[0] == b'c').cloned().collect();
    assert_eq!(bound_chunks, chunks, "the tree binds every section, in sorted order");
    let (records, longest) = tree_shape(&tree);
    assert!(longest <= MAX_RECORD_BYTES, "{longest} B");
    assert!(records < 262_144 / 8, "{records} manifest records");
    assert_eq!(hex::encode(&tree.top.0), "74", "the checkpoint names one digest whatever the size");
}

/// The form a fixture may use, which the module never writes.
#[test]
fn an_inline_manifest_restores_in_a_fixture() {
    let _root = WorldsRootGuard::new("keep-s3c-inline");
    let mut h = fixture("ckpt-inline", 1);
    run_to(&mut h, 5);
    let tree = persist_and_checkpoint(&mut h);
    let inline = h.server.checkpoint_inline().expect("inline");
    assert!(inline.len() > tree.len());
    let back = restore(&h, &inline).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), tree);
}

/// A persist rewrites what changed, and removes what is no longer bound.
#[test]
fn a_persist_leaves_only_what_it_binds() {
    let _root = WorldsRootGuard::new("keep-s3c-stale");
    let mut h = fixture("ckpt-stale", 1);
    run_to(&mut h, 5);
    for i in 0..10_000 {
        h.server.world.pop_exposure.insert((i, 20, 0), 1);
    }
    persist_and_checkpoint(&mut h);
    let dir = world_dir("ckpt-stale").join("authority");
    let held = || std::fs::read_dir(&dir).unwrap().count();
    let many = held();
    h.server.world.pop_exposure.clear();
    let bytes = persist_and_checkpoint(&mut h);
    assert_eq!(held() + 3, many, "three pieces of the clocks went");
    restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    let files = crate::authority_checkpoint::bound_record_files(&h.server).unwrap();
    assert_eq!(held(), files.len());
    assert!(files.iter().all(|(_, bytes)| bytes.len() <= MAX_RECORD_BYTES));
}

/// A persist that isn't the authority's writes nothing new.
#[test]
fn a_world_never_run_as_an_authority_gains_no_new_file() {
    let _root = WorldsRootGuard::new("keep-s3c-no-file");
    let mut h = fixture("ckpt-no-file", 1);
    run_to(&mut h, 5);
    h.server.try_save().expect("saves");
    assert!(!world_dir("ckpt-no-file").join("authority").exists());
}

/// The authority's persist writes the records it binds.
#[test]
fn the_authoritys_persist_writes_the_records() {
    let _root = WorldsRootGuard::new("keep-s3c-writes");
    let mut h = fixture("ckpt-writes", 1);
    run_to(&mut h, 5);
    persist_and_checkpoint(&mut h);
    let held = std::fs::read_dir(world_dir("ckpt-writes").join("authority")).map(Iterator::count);
    assert!(matches!(held, Ok(n) if n > 3), "{held:?}");
    let sizes = section_sizes(&h.server).expect("sizes");
    assert!(sizes.iter().any(|(name, _)| name.starts_with("record:")));
}

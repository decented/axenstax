//! Keep slice 3: the checkpoint's size doesn't follow the world's. The manifest, the
//! layout, lava, water, each cell's exposure clock, the cell sims, the player slots and
//! the corpses are bound stored records, so each of them can grow past what the 1 MiB
//! image could hold (Spec 01 §4.1.5).

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, SEED};
use super::twin_run::{run_to, twin};
use crate::authority_checkpoint::{
    commit_and_read_bound, layout_record, layout_sections_read, rebind_layout, section_sizes, tree_shape, MAX_CHECKPOINT_BYTES, MAX_RECORD_BYTES,
};
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
    let spread: Vec<_> = (0..500_000).map(|i| (i, 300, i % 7, 1u8)).collect();
    h.server.lava.queue_for_test(&spread, &[]);
    let bytes = persist_and_checkpoint(&mut h);
    assert!(bytes.len() < MAX_CHECKPOINT_BYTES / 4, "{} B", bytes.len());
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
}

/// A very large water load: about 87,000 flowing cells' sources used to reach the cap.
/// Each here is a source (12 B) and a queued front (13 B), at a small cost on disk.
#[test]
fn a_very_large_water_load_stays_under_the_cap_and_restores() {
    let _root = WorldsRootGuard::new("keep-s3d-water");
    let mut h = fixture("ckpt-water", 8);
    run_to(&mut h, 5);
    for i in 0..400_000 {
        h.server.water.add_source(i, 300, i % 7);
    }
    let bytes = persist_and_checkpoint(&mut h);
    assert!(bytes.len() < MAX_CHECKPOINT_BYTES / 4, "{} B", bytes.len());
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
    let files = crate::authority_checkpoint::bound_record_files(&h.server).unwrap();
    assert!(files.iter().all(|(_, record)| record.len() <= MAX_RECORD_BYTES));
    assert!(files.iter().filter(|(key, _)| key[0] == b'f').count() > 100, "the load is in its record");
}

fn zeros<T: serde::de::DeserializeOwned>() -> T {
    bincode::deserialize(&[0u8; 1 << 12]).expect("zeros decode as this")
}

/// Every list that can grow with what the world holds, past what the image could carry,
/// and the image is the size it was: only the settled anchors have a length in it, and
/// they are one a player.
#[test]
fn the_image_is_one_size_whatever_the_world_holds() {
    let _root = WorldsRootGuard::new("keep-s3d-flood");
    let mut h = fixture("ckpt-flood", 2);
    run_to(&mut h, 5);
    let small = h.server.checkpoint().expect("checkpoints").len();
    const N: i32 = 120_000;
    for i in 0..N {
        h.server.water.add_source(i, 300, 0);
        h.server.world.power.energised.insert((i, 301, 0), 3);
        h.server.world.power.mains.insert((i, 302, 0));
        h.server.world.tapped_rubber_logs.insert((i, 303, 0), 7);
    }
    let cell = |y: i32| (0..N).map(move |i| (i, y, 0));
    // A load re-adopts every fire block a column kept, past what a run's ignitions reach.
    let burning: Vec<_> = cell(308).map(|pos| (pos, 1_000_000u64, 0u8)).collect();
    h.server.fire.restore_cells(&burning);
    let tick = h.server.leaf_decay.digest_view().2;
    h.server.leaf_decay.restore_view((cell(304).collect(), cell(305).map(|(x, y, z)| (x, y, z, tick + 400)).collect(), tick));
    let due = cell(306).map(|pos| (1_000_000, crate::block_update::ScheduledUpdate { pos, kind: crate::block_update::ScheduleKind::GateSettle })).collect();
    h.server.world.scheduler.restore_view(cell(307).collect(), due);
    // Plans in a slot's inventory: a plan holds a whole captured building's cells.
    let mut plan: crate::plan::PlanData = zeros();
    plan.cells = vec![zeros(); 32_768];
    let mut carrier = crate::server::ServerPlayer::new(glam::Vec3::new(0.5, 110.0, 0.5));
    carrier.server_simulated = true;
    for slot in 0..36 {
        carrier.inventory.set_slot(slot, Some(crate::item::ItemStack { item: crate::item::Item::Plan(plan.clone()), count: 1 }));
    }
    h.server.players.push(carrier);
    h.server.persist_authority().expect("persists");
    // A save waits for the sweep, so these come after it, with the record that holds them.
    for i in 0..3_000 {
        let cow = crate::entity::spawn_mob(&mut h.server.ecs, crate::mob::MobType::Cow, glam::Vec3::new(i as f32, 300.0, 0.5));
        h.server.ecs.get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
    }
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("checkpoints");
    assert_eq!(bytes.len(), small, "the image grew");
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
    assert!(crate::authority_checkpoint::bound_record_files(&h.server).unwrap().iter().all(|(_, record)| record.len() <= MAX_RECORD_BYTES));
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

/// A real restore of a world whose layout names 65,537 sections that hold nothing, past
/// the old limit of 65,536: a small fixture's own layout with 3,641 columns of empty
/// live sections added, bound by the real tree, then restored for real. Each empty
/// section is conjured on restore (about 12.8 KB), so this holds about 840 MB; it is
/// ignored so a gate on a shared machine doesn't build it, and run by name with
/// `--ignored` where there are 16 GB of disk and 5 GB of memory to spare.
#[test]
#[ignore]
fn a_real_restore_past_65536_empty_sections() {
    let _root = WorldsRootGuard::new("keep-s3a-65537");
    let mut h = fixture("ckpt-65537", 0);
    run_to(&mut h, 5);
    let bytes = persist_and_checkpoint(&mut h);
    let mut layout = layout_record(&h.server);
    let base = u32::from_le_bytes(layout[..4].try_into().unwrap());
    let (columns, first) = (3_641u32, 10_000i32);
    layout[..4].copy_from_slice(&(base + columns).to_le_bytes());
    let mut named = 0u32;
    for i in 0..columns {
        // The last column holds one section fewer: 3,640 of 18 and one of 17 is 65,537.
        let live = if i + 1 == columns { LAYOUT_COLUMN_SECTIONS & (LAYOUT_COLUMN_SECTIONS - 1) } else { LAYOUT_COLUMN_SECTIONS };
        named += live.count_ones();
        layout.extend_from_slice(&(first + i as i32).to_le_bytes());
        layout.extend_from_slice(&0i32.to_le_bytes());
        layout.push(0);
        layout.extend_from_slice(&live.to_le_bytes());
        layout.extend_from_slice(&0u32.to_le_bytes());
    }
    assert_eq!(named, 65_537);
    assert!(layout_sections_read(&layout).unwrap() >= 65_537, "the layout reads past the old limit");
    let crafted = rebind_layout(&h.server, &bytes, layout);
    let back = restore(&h, &crafted).unwrap_or_else(|e| panic!("restores past 65,536 sections: {e:?}"));
    let held: u32 = back.world.chunk_layout().unwrap().iter().map(|c| c.live_sections.count_ones() + c.evicted_sections.count_ones()).sum();
    assert!(held >= 65_537, "{held} sections");
    assert_eq!(back.checkpoint().expect("again"), crafted);
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

/// A fork is a new world: it takes the source's chunks and tables and none of the
/// records that bind them, which name the source's players.
#[test]
fn a_fork_does_not_carry_the_authoritys_records() {
    let _root = WorldsRootGuard::new("keep-s3a-fork");
    let mut h = fixture("ckpt-fork", 1);
    run_to(&mut h, 5);
    persist_and_checkpoint(&mut h);
    assert!(world_dir("ckpt-fork").join("authority").is_dir(), "the source has its records");
    let forked = crate::save::fork_world("ckpt-fork", "ckpt-fork").expect("forks");
    let copy = world_dir(&forked);
    assert!(copy.join("world.dat").is_file(), "the fork has the tables");
    assert!(!copy.join("authority").exists(), "the fork copied the authority's records");
    assert!(world_dir("ckpt-fork").join("authority").is_dir(), "the source keeps them");
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

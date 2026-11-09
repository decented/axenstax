//! Keep slice 3: every field of `WorldSave` is bound by the checkpoint. The list is
//! driven from the type: a field added to the save doesn't compile until it has a
//! row here, and a row is a real restore that must come back `InvalidCheckpoint`.
//! "Makes the hash differ" is not what these check (Spec 01 §4.1.5).
//!
//! One stored file, `world.dat`, holds them all, bound in fixed pieces, so each field's
//! row asks the question of that field's own bytes, and is refused by the piece they lie
//! in. Piece-level cases (missing, swapped, stale, longer) are in
//! `authority_checkpoint_pieces.rs`; the file-wide ones (absent, swapped from another
//! world, an older value) are in `every_record_refuses_every_mutation`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore};
use super::authority_checkpoint_records::{fixture_with_meta, folder_bytes};
use super::authority_checkpoint_refusals::assert_invalid_checkpoint;
use super::twin_run::run_to;
use crate::authority_checkpoint::{layout_record, rebind_layout, record_file, CheckpointError, RestoreError};
use crate::entity::spawn_mob;
use crate::save::{read_world_save, read_world_save_reporting, world_dir, WorldSave, WorldsRootGuard};
use crate::save_format::{encode_world_save, footer_bytes};
use crate::test_harness::TestHost;

/// A valid row of any saved type: what zeros decode to.
fn zero<T: DeserializeOwned>() -> T {
    bincode::deserialize(&[0u8; 1 << 16]).expect("zeros decode as this row")
}

type Mutate = fn(&mut WorldSave);

/// One field of the save. A table is given a row in the baseline, so that it can
/// also be emptied; a value is only ever changed.
struct Field {
    name: &'static str,
    /// Adds a row to the baseline, a table only.
    populate: Option<Mutate>,
    /// Makes the table absent, a table only.
    absent: Option<Mutate>,
    /// Another valid value for the field.
    changed: Mutate,
}

macro_rules! save_fields {
    (tables: [$($table:ident),* $(,)?], values: [$($value:ident: |$s:ident| $change:expr),* $(,)?] $(,)?) => {
        /// Never called. It has no `..`, so a new field of `WorldSave` fails to
        /// compile here until the list below gives it a row.
        #[allow(dead_code, unused_variables)]
        fn every_field_has_a_row(save: WorldSave) {
            let WorldSave { $($table,)* $($value,)* } = save;
        }

        fn save_fields() -> Vec<Field> {
            vec![
                $(Field {
                    name: stringify!($table),
                    populate: Some(|s| s.$table.push(zero())),
                    absent: Some(|s| s.$table.clear()),
                    changed: |s| s.$table.push(zero()),
                },)*
                $(Field {
                    name: stringify!($value),
                    populate: None,
                    absent: None,
                    changed: |$s: &mut WorldSave| { $change; },
                },)*
            ]
        }
    };
}

save_fields! {
    tables: [
        inventory, players, campfires, furnaces, vendors, drying_racks, hives, chests, tip_jars,
        plots, market_hubs, auctions, latent_prints, construction_anchors, architect_plaques,
        village_anchors, populated_villages, village_bells, village_treasuries, active_raids,
        raid_kills, brigand_hideouts, bounties, face_overlays, face_blueprints,
        face_blueprint_blanks, carts, graves, waypoints, block_meta, power_devices, signs,
        item_frames, locked_slots, hostile_acts, rigs, exhibits, composters, saved_mobs,
        dispensers, rig_clips,
    ],
    values: [
        seed: |s| s.seed ^= 1,
        player_x: |s| s.player_x += 1.0,
        player_y: |s| s.player_y += 1.0,
        player_z: |s| s.player_z += 1.0,
        player_health: |s| s.player_health -= 1.0,
        hotbar_slot: |s| s.hotbar_slot += 1,
        raid_scheduler: |s| s.raid_scheduler.last_rolled_day += 1,
        bounty_next_id: |s| s.bounty_next_id += 1,
        bounty_last_refresh_tick: |s| s.bounty_last_refresh_tick += 1,
        workshop: |s| s.workshop = { let mut b = [0u8; 12]; b[8] = 1; bincode::deserialize(&b).expect("no projects, next id 1") },
        satoshi: |s| s.satoshi.has_greeted ^= true,
        local_personal: |s| s.local_personal = Some(crate::personal_value::LocalPersonalSave(vec![3, 9, 8, 7])),
        quest_world: |s| s.quest_world = Some(crate::quest_npcs::QuestWorldSave { scope: [7; 16], npcs: Vec::new() }),
        worldgen_version: |s| s.worldgen_version = s.worldgen_version.map(|v| v + 1),
        authority_clock: |s| s.authority_clock.as_mut().expect("a server's save has a clock").tick_counter += 1,
        block_authors: |s| s.block_authors = Some(crate::authorship::SavedAuthorship {
            keys: vec![crate::authorship::AuthorKey::Npub("npub1mutation".into())],
            chunks: vec![(0, 4, 0, vec![(7, 0)])],
            starters: Vec::new(),
        }),
        region_use: |s| s.region_use = Some(crate::use_record::SavedRegionUse {
            lived_day: 3,
            last_world_day: Some(9),
            regions: vec![((0, 0), crate::use_record::RegionUse { days: 2, last_day: 3, nights: 1, last_night: Some(8) })],
        }),
        plan_builds: |s| s.plan_builds = Some(crate::plan_builds::SavedPlanBuilds { rows: vec![([4; 32], 2, 1)] }),
        plot_succession: |s| s.plot_succession = Some(crate::succession::SavedSuccession { last_seen: vec![("npub1owner".into(), 1_800_000_000)], passed_on: vec![(4, 64, 4)] }),
        site_boards: |s| s.site_boards = Some(crate::site_board::SavedSiteBoards { boards: vec![((4, 64, 4), vec![(2, true)])] }),
    ],
}

fn encoded(save: &WorldSave) -> Vec<u8> {
    encode_world_save(save).expect("encodes")
}

/// How many of a row reach the record limit, from what `add` adds to the save.
fn rows_to_the_limit(base: &WorldSave, add: impl Fn(&mut WorldSave)) -> (usize, usize) {
    const ROWS: usize = 1_000;
    let empty = encoded(base).len();
    let mut filled = read_world_save(&encoded(base)).expect("reads");
    for _ in 0..ROWS {
        add(&mut filled);
    }
    let per_row = (encoded(&filled).len() - empty) / ROWS;
    (per_row, ((1usize << 20).saturating_sub(empty)) / per_row.max(1))
}

/// `world.dat` against the runner's record limit (1 MiB less headers; 1 MiB is used here,
/// so these are slightly generous): the handoff's table. Zero rows are each table's
/// smallest row, what zeros decode to; the "real" rows are a played world's. Run by name.
#[test]
#[ignore]
fn world_dat_rows_to_the_record_limit() {
    let _root = WorldsRootGuard::new("keep-s3c-world-dat");
    let mut h = fixture("ckpt-world-dat", 2);
    run_to(&mut h, 20);
    persist_and_checkpoint(&mut h);
    let base = read_world_save(&std::fs::read(world_dir("ckpt-world-dat").join("world.dat")).unwrap()).expect("a save");
    eprintln!("world_dat base: {} B", encoded(&base).len());
    for field in save_fields() {
        if let Some(populate) = field.populate {
            let (per_row, rows) = rows_to_the_limit(&base, populate);
            eprintln!("world_dat zero row, {}: {per_row} B, {rows} rows", field.name);
        }
    }
    let stack = crate::item::ItemStack::new_block(crate::block::COBBLESTONE, 64);
    let chest = crate::chest::ChestData { slots: vec![Some(stack); 27], tier: Default::default() };
    let (per_row, rows) = rows_to_the_limit(&base, |s| {
        s.chests.push(crate::save::SavedChest { x: 1, y: 2, z: 3, data: chest.clone() });
    });
    eprintln!("world_dat real row, a full wooden chest: {per_row} B, {rows} rows");
    let text = "x".repeat(crate::sign::SIGN_MAX_CHARS);
    let (per_row, rows) = rows_to_the_limit(&base, |s| {
        s.signs.push(crate::save::SavedSign { x: 1, y: 2, z: 3, data: crate::sign::SignData { text: text.clone() } });
    });
    eprintln!("world_dat real row, a full sign: {per_row} B, {rows} rows");
}

/// `None` if the restore refused `record` as it should, else what happened.
fn wrong_outcome(h: &TestHost, bytes: &[u8], file: &Path, record: &[u8], names: &str) -> Option<String> {
    std::fs::write(file, record).expect("write the record");
    match restore(h, bytes).err() {
        Some(RestoreError::InvalidCheckpoint(why)) if why.contains(names) => None,
        Some(other) => Some(format!("refused otherwise: {other:?}")),
        None => Some("restored".to_string()),
    }
}

/// The type's list, run for real.
#[test]
fn every_field_of_the_save_is_bound() {
    let _root = WorldsRootGuard::new("keep-s3-fields");
    let mut h = fixture("ckpt-fields", 1);
    run_to(&mut h, 20);
    persist_and_checkpoint(&mut h);
    let name = h.server.world_name.clone();
    let dir = world_dir(&name);
    let file = dir.join("world.dat");

    // Every table holds a row, as a world that has used them all would.
    let mut rich = read_world_save(&std::fs::read(&file).unwrap()).expect("a save");
    let fields = save_fields();
    for field in &fields {
        if let Some(populate) = field.populate {
            populate(&mut rich);
        }
    }
    let baseline = encoded(&rich);
    std::fs::write(&file, &baseline).unwrap();
    h.server.stored_records.note_world_dat(&name);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("checkpoints");
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("every table holding a row restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
    let before = folder_bytes(&dir);

    let mut wrong = BTreeMap::new();
    for field in &fields {
        let mut save = read_world_save(&baseline).expect("the baseline reads");
        (field.changed)(&mut save);
        let changed = encoded(&save);
        assert_ne!(changed, baseline, "{}: its change changed nothing", field.name);
        let (_, damaged) = read_world_save_reporting(&changed).unwrap_or_else(|e| panic!("{}: not a valid record: {e:?}", field.name));
        assert!(!damaged, "{}: not a whole record", field.name);
        if let Some(why) = wrong_outcome(&h, &bytes, &file, &changed, "world.dat piece") {
            wrong.insert(format!("{}, another valid value", field.name), why);
        }

        // A byte of this field's own: the first where the two records part.
        let at = baseline.iter().zip(&changed).position(|(a, b)| a != b).unwrap_or(baseline.len() - 1);
        let mut flipped = baseline.clone();
        flipped[at] ^= 1;
        if let Some(why) = wrong_outcome(&h, &bytes, &file, &flipped, "world.dat piece") {
            wrong.insert(format!("{}, a flipped byte", field.name), why);
        }

        if let Some(absent) = field.absent {
            let mut save = read_world_save(&baseline).expect("the baseline reads");
            absent(&mut save);
            let emptied = encoded(&save);
            assert_ne!(emptied, baseline, "{}: it held no row", field.name);
            if let Some(why) = wrong_outcome(&h, &bytes, &file, &emptied, "world.dat piece") {
                wrong.insert(format!("{}, its rows gone", field.name), why);
            }
        }
    }
    assert!(wrong.is_empty(), "{} of the save's mutations weren't refused: {wrong:#?}", wrong.len());

    std::fs::write(&file, &baseline).unwrap();
    assert_eq!(folder_bytes(&dir), before, "a refused restore changed the folder");
    restore(&h, &bytes).unwrap_or_else(|e| panic!("put back, it restores: {e:?}"));
}

struct Record {
    name: String,
    path: PathBuf,
    /// Another key's valid bytes.
    swapped: Vec<u8>,
    /// This key's bytes at the last persist.
    older: Vec<u8>,
}

/// Each kind of bound record: a flipped byte, absent, a valid record from another key,
/// and an older valid value of its own key.
#[test]
fn every_record_refuses_every_mutation() {
    let _root = WorldsRootGuard::new("keep-s3-records");
    let mut other = fixture_with_meta("ckpt-records-other");
    run_to(&mut other, 7);
    other.server.try_save().expect("saves");
    let other_dir = world_dir("ckpt-records-other");

    let mut h = fixture_with_meta("ckpt-records");
    run_to(&mut h, 20);
    h.server.persist_authority().expect("the first persist");
    let dir = world_dir("ckpt-records");
    let older_authority = folder_bytes(&dir.join("authority"));
    let chunk = dir.join("chunks/0_6_0.chunk");
    let sidecar = h.server.joiner_path(h.server.players[1].verified_pubkey.unwrap());
    let sidecar_b = h.server.joiner_path(h.server.players[2].verified_pubkey.unwrap());
    let older = |path: &Path| std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (older_dat, older_chunk, older_sidecar, older_meta) =
        (older(&dir.join("world.dat")), older(&chunk), older(&sidecar), older(&dir.join("world_meta.json")));

    // Everything moves before the second persist.
    h.set_block(1, 100, 1, crate::block::COBBLESTONE);
    h.server.players[1].player.pos.x += 1.0;
    let mut meta = crate::save::load_world_meta("ckpt-records");
    meta.description = "later".into();
    crate::save::save_world_meta("ckpt-records", &meta).expect("write meta");
    run_to(&mut h, 30);
    // And the state in records: a new column, a woken lava cell, a clock, a water source
    // and a powered cell. The slots have moved already.
    h.set_block(200, 100, 200, crate::block::COBBLESTONE);
    h.server.lava.add_source(3, 120, 3);
    h.server.world.pop_exposure.insert((3, 20, 3), 9);
    h.server.water.add_source(4, 120, 4);
    h.server.world.power.energised.insert((5, 120, 5), 3);
    h.server.persist_authority().expect("the second persist");
    // A save waits for the sweep, so a corpse comes after it, in its own record.
    let cow = spawn_mob(&mut h.server.ecs, crate::mob::MobType::Cow, glam::Vec3::new(6.5, 120.0, 6.5));
    h.server.ecs.get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("checkpoints");
    restore(&h, &bytes).unwrap_or_else(|e| panic!("untouched, it restores: {e:?}"));

    let records = [
        Record { name: "world.dat".into(), path: dir.join("world.dat"), swapped: older(&other_dir.join("world.dat")), older: older_dat },
        Record { name: "chunk".into(), path: chunk.clone(), swapped: older(&dir.join("chunks/1_6_1.chunk")), older: older_chunk },
        Record { name: "sidecar".into(), path: sidecar.clone(), swapped: older(&sidecar_b), older: older_sidecar },
        Record { name: "world meta".into(), path: dir.join("world_meta.json"), swapped: older(&other_dir.join("world_meta.json")), older: older_meta },
    ];
    // The records only an authority has: each state record, the manifest's root, and
    // one of the tree's other records. Each has an older value of its own key, and any
    // of them is another's valid bytes.
    let authority = |key: &[u8]| record_file(&dir, key);
    let older_of = |key: &[u8]| older_authority.get(&authority(key)).unwrap_or_else(|| panic!("no older {}", hex::encode(key))).clone();
    let files = crate::authority_checkpoint::bound_record_files(&h.server).unwrap();
    let node = files
        .iter()
        .map(|(key, _)| key)
        .find(|key| key.len() > 1 && key[0] == b't' && older_authority.get(&authority(key)).is_some_and(|was| *was != older(&authority(key))))
        .expect("a persist rewrites a record below the root");
    let piece = |family: u8| {
        let mut key = vec![family];
        key.extend_from_slice(&0u32.to_be_bytes());
        key
    };
    // One row for each state family, from the list the records module keeps, so a family
    // added there is mutated here. Each takes the next family's piece as another key's
    // valid bytes.
    let families = crate::authority_checkpoint::state_families();
    let mut authority_records: Vec<Record> = families
        .iter()
        .enumerate()
        .map(|(i, (family, name))| {
            let (key, next) = (piece(*family), piece(families[(i + 1) % families.len()].0));
            Record { name: format!("{name} piece"), path: authority(&key), swapped: older(&authority(&next)), older: older_of(&key) }
        })
        .collect();
    authority_records.push(Record { name: "manifest root".into(), path: authority(b"t"), swapped: older(&authority(node)), older: older_of(b"t") });
    authority_records.push(Record { name: "manifest record".into(), path: authority(node), swapped: older(&authority(b"t")), older: older_of(node) });
    let mut wrong = BTreeMap::new();
    for record in records.iter().chain(&authority_records) {
        let good = older(&record.path);
        assert_ne!(record.older, good, "{}: the persists left it as it was", record.name);
        assert_ne!(record.swapped, good, "{}: another key held these bytes", record.name);
        let mut flipped = good.clone();
        flipped[2] ^= 1;
        for (case, bytes_there) in [("flipped", Some(&flipped)), ("absent", None), ("swapped", Some(&record.swapped)), ("older", Some(&record.older))] {
            match bytes_there {
                Some(b) => std::fs::write(&record.path, b).unwrap(),
                None => std::fs::remove_file(&record.path).unwrap(),
            }
            match restore(&h, &bytes).err() {
                Some(RestoreError::InvalidCheckpoint(_)) => {}
                other => {
                    wrong.insert(format!("{}, {case}", record.name), format!("{other:?}"));
                }
            }
        }
        std::fs::write(&record.path, &good).unwrap();
    }
    assert!(wrong.is_empty(), "records' mutations not refused: {wrong:#?}");
    restore(&h, &bytes).unwrap_or_else(|e| panic!("all put back, it restores: {e:?}"));
}

/// The volatile image: any flipped byte is a seal mismatch.
#[test]
fn a_flipped_byte_anywhere_in_the_image_is_refused() {
    let _root = WorldsRootGuard::new("keep-s3-image");
    let mut h = fixture("ckpt-image", 1);
    run_to(&mut h, 5);
    let bytes = persist_and_checkpoint(&mut h);
    for at in (crate::authority_checkpoint::CHECKPOINT_MAGIC.len() + 2..bytes.len()).step_by(97) {
        let mut flipped = bytes.clone();
        flipped[at] ^= 1;
        assert_invalid_checkpoint(&h, &flipped, &format!("byte {at}"));
    }
}

/// Twins write the same `world.dat`, whatever order their hash maps iterate in.
#[test]
fn twins_write_the_same_world_dat() {
    let _root = WorldsRootGuard::new("keep-s3-twin-dat");
    let mut a = fixture("ckpt-dat-a", 2);
    let mut b = fixture("ckpt-dat-b", 2);
    run_to(&mut a, 200);
    run_to(&mut b, 200);
    let (ca, cb) = (persist_and_checkpoint(&mut a), persist_and_checkpoint(&mut b));
    let dat = |name: &str| std::fs::read(world_dir(name).join("world.dat")).unwrap();
    let (da, db) = (dat("ckpt-dat-a"), dat("ckpt-dat-b"));
    if da != db {
        let at = da.iter().zip(&db).position(|(x, y)| x != y);
        panic!("twins' world.dat differ, first at {at:?} of {} and {} bytes", da.len(), db.len());
    }
    assert_eq!(ca, cb);
}

/// A table's change moves the checkpoint at the persist that stores it, and the
/// digest named is exactly the file's.
#[test]
fn a_persist_moves_the_checkpoint_with_the_tables() {
    use sha2::{Digest, Sha256};
    let _root = WorldsRootGuard::new("keep-s3-persist-moves");
    let mut h = fixture("ckpt-persist-moves", 1);
    run_to(&mut h, 5);
    let first = persist_and_checkpoint(&mut h);
    h.server.world.village_bells.push([1, 2, 3]);
    let second = persist_and_checkpoint(&mut h);
    assert_ne!(first, second, "the world's tables moved");
    let file = std::fs::read(world_dir("ckpt-persist-moves").join("world.dat")).unwrap();
    let digest: [u8; 32] = Sha256::digest(&file).into();
    let entries = crate::authority_checkpoint::manifest_entries(&h.server).unwrap();
    let pieces: Vec<_> = entries.iter().filter(|(key, _)| key[0] == b'w').collect();
    assert_eq!((pieces.len(), pieces[0].0.as_slice(), pieces[0].1), (1, &b"w\0\0\0\0"[..], digest));
    assert!(!entries.iter().any(|(key, _)| key == b"b" || key == b"w"), "the tables have stored bytes, in pieces");
}

/// A record written before this build's last field is unchanged, and doesn't read as
/// changed: the digest is of its bytes, not of what this build would write.
#[test]
fn a_record_from_an_older_build_is_no_mismatch() {
    let _root = WorldsRootGuard::new("keep-s3-older-build");
    let mut h = fixture("ckpt-older-build", 1);
    run_to(&mut h, 5);
    persist_and_checkpoint(&mut h);
    let name = h.server.world_name.clone();
    let file = world_dir(&name).join("world.dat");
    let mut save = read_world_save(&std::fs::read(&file).unwrap()).unwrap();
    save.authority_clock = None;
    save.block_authors = None;
    save.region_use = None;
    save.plan_builds = None;
    save.plot_succession = None;
    save.site_boards = None;
    let mut body = bincode::serialize(&save).unwrap();
    body.pop(); // The site boards' `None`: a format 59 writer had no such field,
    body.pop(); // nor the succession table's,
    body.pop(); // nor the Plan build counts',
    body.pop(); // nor the use record's,
    body.pop(); // nor the authorship's,
    body.pop(); // nor the clock's.
    body.extend_from_slice(&footer_bytes(59));
    std::fs::write(&file, &body).unwrap();
    h.server.stored_records.note_world_dat(&name);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("checkpoints");
    let back = restore(&h, &bytes).unwrap_or_else(|e| panic!("an older record restores: {e:?}"));
    assert_eq!(back.checkpoint().expect("again"), bytes);
    assert_eq!(std::fs::read(&file).unwrap(), body, "and it is as it was");
}

/// A checkpoint of a later tick than the persist it names isn't the state those
/// records hold: its digests can all match while the tables are stale.
#[test]
fn a_checkpoint_later_than_its_persist_is_refused() {
    let _root = WorldsRootGuard::new("keep-s3-late");
    let mut h = fixture("ckpt-late", 1);
    run_to(&mut h, 5);
    let good = persist_and_checkpoint(&mut h);
    h.server.tick_counter += 1;
    let late = h.server.checkpoint().expect("checkpoints");
    assert_ne!(good, late);
    assert_invalid_checkpoint(&h, &late, "a later tick than the persist");
    h.server.tick_counter -= 1;
    restore(&h, &good).unwrap_or_else(|e| panic!("the good one restores: {e:?}"));
}

/// `pop_exposure` is in no save: the checkpoint binds it, in a record of its own.
#[test]
fn a_cells_exposure_clock_comes_back() {
    let _root = WorldsRootGuard::new("keep-s3-exposure");
    let mut h = fixture("ckpt-exposure", 1);
    run_to(&mut h, 5);
    let without = persist_and_checkpoint(&mut h);
    h.server.world.pop_exposure.insert((3, 70, 1), 9);
    h.server.world.pop_exposure.insert((-3, 70, 1), 12);
    let with = h.server.checkpoint().expect("checkpoints");
    assert_ne!(without, with, "the clock is bound");
    assert_invalid_checkpoint(&h, &with, "a clock no persist has stored");
    h.server.persist_authority().expect("persists");
    let back = restore(&h, &with).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(back.world.pop_exposure, h.server.world.pop_exposure);
    assert_eq!(back.checkpoint().expect("again"), with);

    h.server.world.pop_exposure.insert((i32::MAX - 1, 70, 1), 9);
    assert!(matches!(h.server.checkpoint(), Err(CheckpointError::Unrestorable(_))), "a cell no tick can reach");
}

/// A restore that passes the digests writes nothing: the load's recoveries are for a
/// normal open, and what they would act on isn't a record this checkpoint binds.
#[test]
fn a_restore_writes_nothing_whatever_else_the_folder_holds() {
    let _root = WorldsRootGuard::new("keep-s3-writes-nothing");
    let mut h = fixture("ckpt-writes-nothing", 1);
    run_to(&mut h, 5);
    let bytes = persist_and_checkpoint(&mut h);
    let dir = world_dir("ckpt-writes-nothing");
    // A joiner stage for a commit that never happened.
    let stage = dir.join(crate::joiner_stage::STAGED_JOINERS);
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("world-dat.sha256"), "00".repeat(32)).unwrap();
    std::fs::write(stage.join("npub1stale.dat"), b"stale").unwrap();
    // A first save's stage, and a torn chunk file no record binds.
    let staged_chunks = dir.join(crate::save::STAGED_CHUNKS);
    std::fs::create_dir_all(&staged_chunks).unwrap();
    std::fs::write(staged_chunks.join("90_6_90.chunk"), [0u8; 8704]).unwrap();
    std::fs::write(dir.join("chunks/99_6_99.chunk"), b"torn").unwrap();
    let before = folder_bytes(&dir);

    let mut back = restore(&h, &bytes).unwrap_or_else(|e| panic!("restores: {e:?}"));
    assert_eq!(folder_bytes(&dir).keys().collect::<Vec<_>>(), before.keys().collect::<Vec<_>>(), "files came or went");
    assert_eq!(folder_bytes(&dir), before, "a restore wrote");
    assert!(!back.world.has_chunk(99, 6, 99), "the torn file isn't restored from");
    assert_eq!(back.checkpoint().expect("again"), bytes);
    back.try_save().expect("the restored world saves");
    assert!(!stage.exists() && !staged_chunks.exists(), "and its persist settles what was left over");
}

/// A bound `world.dat` this build only part-decodes defaults what it couldn't read,
/// which isn't the state it binds: refused, and no copy is kept aside.
#[test]
fn a_partly_decoded_world_dat_is_refused_and_nothing_is_written() {
    let _root = WorldsRootGuard::new("keep-s3-partial");
    let mut h = fixture("ckpt-partial", 1);
    run_to(&mut h, 5);
    persist_and_checkpoint(&mut h);
    let name = h.server.world_name.clone();
    let dir = world_dir(&name);
    let file = dir.join("world.dat");
    let save = read_world_save(&std::fs::read(&file).unwrap()).unwrap();
    // No footer, as the oldest saves, and cut inside a field: the five newest
    // (authorship, use record, Plan build counts, succession, site boards) are one
    // `None` byte each, so six bytes reaches into the clock before them.
    let mut body = bincode::serialize(&save).unwrap();
    body.truncate(body.len() - 6);
    assert!(read_world_save_reporting(&body).expect("reads").1, "the fixture is a partly decoded record");
    std::fs::write(&file, &body).unwrap();
    h.server.stored_records.note_world_dat(&name);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("checkpoints");
    let before = folder_bytes(&dir);
    assert_invalid_checkpoint(&h, &bytes, "a partly decoded world.dat");
    assert_eq!(folder_bytes(&dir), before, "a copy was kept aside");
}

/// The shared reader: evicted sections only in a column flagged evicted.
#[test]
fn evicted_sections_in_a_column_not_flagged_evicted_are_refused() {
    let _root = WorldsRootGuard::new("keep-s3-evicted-flag");
    let mut h = fixture("ckpt-evicted-flag", 1);
    run_to(&mut h, 5);
    let bytes = persist_and_checkpoint(&mut h);
    // The first column: x, z, flags, live, evicted.
    let entry = 4;
    let mut layout = layout_record(&h.server);
    assert_eq!(layout[entry + 8] & 2, 0, "the fixture's first column isn't evicted");
    let live = u32::from_le_bytes(layout[entry + 9..entry + 13].try_into().unwrap());
    let free = !live & crate::world::LAYOUT_COLUMN_SECTIONS;
    layout[entry + 13..entry + 17].copy_from_slice(&(1u32 << free.trailing_zeros()).to_le_bytes());
    let crafted = rebind_layout(&h.server, &bytes, layout);
    match restore(&h, &crafted).err() {
        Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains("not flagged evicted"), "{why}"),
        other => panic!("evicted sections, no flag: {other:?}"),
    }
}

/// The shared reader: a corpse past the playable range isn't anywhere a tick can
/// reach, and `checkpoint` refuses what a restore would.
#[test]
fn a_corpse_past_the_range_is_an_error_at_checkpoint() {
    let _root = WorldsRootGuard::new("keep-s3-corpse-range");
    let mut h = fixture("ckpt-corpse-range", 1);
    run_to(&mut h, 5);
    persist_and_checkpoint(&mut h);
    let cow = spawn_mob(&mut h.server.ecs, crate::mob::MobType::Cow, glam::Vec3::new(2.0e7, 100.0, 0.0));
    h.server.ecs.get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
    assert!(matches!(h.server.checkpoint(), Err(CheckpointError::Unrestorable(_))));
}

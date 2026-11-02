//! Keep slice 2b — a restore and the world's stored records: a joiner's sidecar
//! stays the truth until it re-attaches, the world's meta is round-tripped and
//! bound, a refusal after the load leaves the folder as it was, and the load
//! generates nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, SEED};
use super::authority_checkpoint_refusals::assert_invalid_checkpoint;
use super::twin_run::run_to;
use crate::authority_checkpoint::{seal, section_offsets, RestoreError};
use crate::container_window::ItemDelta;
use crate::item::ItemStack;
use crate::joiner_save::{BoundPersonal, JoinerSave};
use crate::progression::{GameplayBinding, WorldPlayer};
use crate::save::WorldsRootGuard;
use crate::test_harness::{TestConfig, TestHost};

/// Every file under `dir`, with its bytes.
fn folder_bytes(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir).expect("a folder").flatten() {
        if entry.metadata().expect("metadata").is_dir() {
            out.extend(folder_bytes(&entry.path()));
        } else {
            out.insert(entry.path(), std::fs::read(entry.path()).expect("a file"));
        }
    }
    out
}

/// Joiner `slot` is bound to this world and owes 12 cobblestone it doesn't hold.
fn bind_and_owe(h: &mut TestHost, slot: usize) {
    let scope = [19; 16];
    let key = h.server.players[slot].verified_pubkey.expect("signed in");
    h.server.world.quest_scope = Some(scope);
    let sp = &mut h.server.players[slot];
    sp.personal = Some(BoundPersonal::fresh(GameplayBinding::WorldLocal { scope, player: WorldPlayer::Npub(key) }).unwrap());
    let stack = ItemStack::new_block(crate::block::COBBLESTONE, 12);
    let take = ItemDelta { take: vec![(0, stack)], give: vec![] };
    sp.window_events.debt.apply(&mut sp.inventory, &mut sp.craft_grid, &mut sp.cursor, &mut sp.armour, &take);
}

/// The record is the truth until the joiner comes back: an autosave, a shutdown
/// save and a save of that one slot all leave it byte for byte, debt and binding
/// included, and nothing reads the slot as owing nothing meanwhile.
#[test]
fn a_restored_joiners_debt_stays_in_its_record() {
    let _root = WorldsRootGuard::new("keep-s2b-debt");
    let mut a = fixture("ckpt-debt", 1);
    bind_and_owe(&mut a, 1);
    run_to(&mut a, 20);
    let bytes = persist_and_checkpoint(&mut a);
    let path = a.server.joiner_path(a.server.players[1].verified_pubkey.unwrap());
    let record = std::fs::read(&path).expect("the save wrote a sidecar");
    let saved = JoinerSave::decode(&record).expect("a record");
    assert!(!saved.debt().unwrap().is_empty(), "the fixture owes something");
    assert!(saved.personal.is_some(), "and is bound");

    let mut b = TestHost::adopt(restore(&a, &bytes).expect("restores"));
    assert!(b.server.players[1].personal.is_some(), "the binding came back from the sidecar");
    assert!(!b.server.players[1].obligations_proven_zero(), "a restored debtor doesn't read as clean");
    b.server.try_save().expect("the restored world saves");
    assert_eq!(std::fs::read(&path).unwrap(), record, "a save rewrote the record");
    b.server.save_joiner(1).expect("a slot's own save does nothing");
    assert_eq!(std::fs::read(&path).unwrap(), record, "a slot's save rewrote the record");
    let end = b.server.tick_counter + 20;
    run_to(&mut b, end);
    b.server.save_joiners().expect("the autosave's pass does nothing");
    assert_eq!(std::fs::read(&path).unwrap(), record, "ticks and an autosave rewrote the record");

    b.server.players[1].enter_world();
    b.server.save_joiner(1).unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), record, "back in the world, its saves are its own again");
}

/// A world with a saved meta, as every real one has.
fn fixture_with_meta(name: &str) -> TestHost {
    let mut meta = crate::save::WorldMeta::new(name);
    meta.seed = SEED;
    meta.worldgen_version = crate::biome::Generator::V2.version();
    crate::save::save_world_meta(name, &meta).expect("write meta");
    fixture(name, 2)
}

#[test]
fn a_world_with_a_meta_round_trips_it_and_refuses_a_changed_one() {
    let _root = WorldsRootGuard::new("keep-s2b-meta-round");
    let mut h = fixture_with_meta("ckpt-meta-round");
    run_to(&mut h, 20);
    let bytes = persist_and_checkpoint(&mut h);
    assert!(h.server.stored_records.meta_digest().is_some(), "the meta is a record");
    let restored = TestHost::adopt(restore(&h, &bytes).expect("restores"));
    assert_eq!(restored.server.checkpoint().expect("checkpoints again"), bytes);

    let file = crate::save::world_dir("ckpt-meta-round").join("world_meta.json");
    let original = std::fs::read(&file).unwrap();
    let mut meta = crate::save::load_world_meta("ckpt-meta-round");
    meta.description = "something else".into();
    crate::save::save_world_meta("ckpt-meta-round", &meta).expect("write meta");
    match restore(&h, &bytes).err() {
        Some(RestoreError::InvalidCheckpoint(why)) => assert!(why.contains("world meta"), "{why}"),
        other => panic!("a changed meta: got {other:?}"),
    }
    std::fs::write(&file, &original).unwrap();
    restore(&h, &bytes).expect("the meta put back, it restores");
}

/// Refused after the load, with the world opened and the layout and the volatile
/// state applied to a server nobody gets: the folder is as it was, and the real
/// checkpoint still restores.
#[test]
fn a_refusal_after_the_load_leaves_the_world_able_to_restore() {
    let _root = WorldsRootGuard::new("keep-s2b-after-load");
    let mut h = fixture("ckpt-after-load", 2);
    h.server.players[0].player.velocity = glam::Vec3::ZERO;
    run_to(&mut h, 20);
    h.server.players[0].player.velocity = glam::Vec3::ZERO;
    let bytes = persist_and_checkpoint(&mut h);
    let dir = crate::save::world_dir("ckpt-after-load");
    let before = folder_bytes(&dir);

    // The manifest's last digest: refused where the records are compared.
    let mut tampered = bytes.clone();
    let at = tampered.len() - 33;
    tampered[at] ^= 1;
    seal(&mut tampered);
    assert_invalid_checkpoint(&h, &tampered, "a resealed manifest");
    assert_eq!(folder_bytes(&dir), before, "the refused restore changed the folder");

    // A `-0.0`: refused at the very end, by the re-encode.
    let velocity = section_offsets(&h.server).unwrap().players + 4 + 12;
    let mut minus_zero = bytes.clone();
    assert_eq!(minus_zero[velocity..velocity + 4], [0; 4]);
    minus_zero[velocity..velocity + 4].copy_from_slice(&(-0.0f32).to_le_bytes());
    seal(&mut minus_zero);
    assert_invalid_checkpoint(&h, &minus_zero, "a resealed -0.0");
    assert_eq!(folder_bytes(&dir), before, "the refused restore changed the folder");

    let restored = TestHost::adopt(restore(&h, &bytes).expect("the real one still restores"));
    assert_eq!(restored.server.checkpoint().unwrap(), bytes);
    assert_eq!(folder_bytes(&dir), before, "the restore changed the folder");
}

/// A centre column the run let go before its first save isn't on disk. Generating
/// it for the load could add to a table the manifest binds, so it isn't generated.
#[test]
fn a_restore_generates_no_column_the_folder_lacks() {
    let _root = WorldsRootGuard::new("keep-s2b-centre");
    let config = TestConfig { seed: SEED, world_name: "ckpt-centre".into(), do_initial_load: true, ..TestConfig::default() };
    let mut h = TestHost::start_seeded(config, 0);
    let biome = crate::biome::BiomeGenerator::new(SEED, h.server.world.generator);
    h.server.world.generate_column(30, 30, &biome);
    h.server.loaded_columns.insert((30, 30));
    h.server.world.discard_column(0, 0);
    h.server.loaded_columns.remove(&(0, 0));
    h.server.water.forget_column(0, 0);
    h.server.lava.forget_column(0, 0);
    h.server.try_save().expect("saves");
    let chunks = crate::save::world_dir("ckpt-centre").join("chunks");
    assert!(
        !std::fs::read_dir(&chunks).unwrap().flatten().any(|f| f.file_name().to_string_lossy().starts_with("0_")),
        "column (0, 0) isn't on disk"
    );
    let bytes = h.server.checkpoint().expect("checkpoints");

    let generated = || crate::world::COLUMNS_GENERATED.with(std::cell::Cell::get);
    let before = generated();
    let restored = restore(&h, &bytes).expect("restores");
    assert_eq!(generated(), before, "the restore's load generated a column");
    assert!(!restored.loaded_columns.contains(&(0, 0)));
    assert!(!restored.world.has_chunk(0, 0, 0));
}

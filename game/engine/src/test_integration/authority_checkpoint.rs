//! Keep slice 2b — the authority checkpoint and restore (Spec 01 §4.1.5): the
//! same bytes from twin worlds, the same bytes after a round trip, and a
//! restored world that carries on as the world it came from, for the sections
//! the restore promises (the refusals are in `authority_checkpoint_refusals.rs`).
//!
//! The fixture is slice 1's (`twin_run.rs`), its joiners signed in so that a
//! save writes their sidecars.

use glam::Vec3;

use super::twin_run::{add_joiner, run_to, twin, twin_loaded, BASE_Y};
use crate::authority_checkpoint::{RestoreError, CHECKPOINT_MAGIC, MAX_CHECKPOINT_BYTES};
use crate::save::WorldsRootGuard;
use crate::server::GameServer;
use crate::test_harness::TestHost;

pub(super) const SEED: u32 = 4242;

/// What a restored world must keep equal to the uninterrupted one, by digest
/// section (Keep slice 2 spec, item 6).
const REQUIRED: [&str; 4] = ["chunks", "block_entities", "clocks", "players"];

/// Not one of the four required, but the checkpoint carries it, so it holds too.
const ALSO_CARRIED: [&str; 1] = ["fire_fluid_queues"];

/// The sections a restore does not bring back yet: the restoration ledger
/// (handoff §7). Printed by every comparison; excluded from its assertion.
const NOT_YET_RESTORED: [&str; 6] = [
    // Recomputed by the load's column pass, which needn't match the increments
    // a run made block by block.
    "light",
    // Wild mobs, night hostiles, golems and raiders: re-scattered, and hecs ids
    // change on restore. Mob AI rolls hash the id (`mob_ai::simple_hash`).
    "wild_mobs",
    // Tamed pets, carts and quest givers: from the save, with new hecs ids.
    "tamed_entities",
    // On the ground when the checkpoint was taken: gone. Hecs ids change on restore.
    "item_drops",
    // In flight when the checkpoint was taken: gone. Hecs ids change on restore.
    "projectiles",
    // Equal in this fixture, so printed and not promised: hideout and raid counts
    // follow the mobs. The tables themselves read back from `world.dat`, which the
    // manifest binds, and `pop_exposure` is in the volatile sections.
    "world_tables",
];

pub(super) fn fixture(name: &str, joiners: usize) -> TestHost {
    let mut h = twin(SEED, name, joiners, 2);
    sign_in(&mut h);
    h
}

/// Give every joiner a persona, so a save writes a sidecar for it.
pub(super) fn sign_in(h: &mut TestHost) {
    for (slot, sp) in h.server.players.iter_mut().enumerate() {
        if sp.server_simulated {
            let mut pk = [0u8; 32];
            pk[0] = 0x02;
            pk[31] = slot as u8;
            sp.verified_pubkey = Some(pk);
        }
    }
}

/// Run on until no mob death is waiting: a save refuses one (the runner withholds
/// its `last` while one is pending).
fn settle(h: &mut TestHost) {
    while crate::sim_death::has_pending(&h.server.ecs) {
        let next = h.server.tick_counter + 1;
        run_to(h, next);
    }
}

/// Persist with the existing save, then checkpoint.
pub(super) fn persist_and_checkpoint(h: &mut TestHost) -> Vec<u8> {
    settle(h);
    h.server.try_save().expect("the fixture saves");
    h.server.checkpoint().expect("the fixture checkpoints")
}

pub(super) fn restore(h: &TestHost, bytes: &[u8]) -> Result<GameServer, RestoreError> {
    GameServer::restore_from_checkpoint(
        &h.server.authority_config(),
        bytes,
        &crate::save::world_dir(&h.server.world_name),
    )
}

/// The sections that differ between two servers.
fn differing(a: &GameServer, b: &GameServer) -> Vec<&'static str> {
    let (sa, sb) = (a.sim_digest_sections(), b.sim_digest_sections());
    sa.iter().filter(|(name, d)| sb.get(*name) != Some(*d)).map(|(name, _)| *name).collect()
}

/// Print what differs and fail on anything the restore promises.
fn assert_restored(label: &str, a: &GameServer, b: &GameServer, promised: &[&str]) {
    let differ = differing(a, b);
    let ledger: Vec<_> = differ.iter().filter(|s| NOT_YET_RESTORED.contains(s)).collect();
    eprintln!("{label}: differs {differ:?}; of the ledger {NOT_YET_RESTORED:?}, {ledger:?}");
    let broken: Vec<_> = differ.iter().filter(|s| promised.contains(s)).collect();
    assert!(broken.is_empty(), "{label}: a restored world differs in {broken:?}");
}

#[test]
fn every_digest_section_is_classified() {
    let _root = WorldsRootGuard::new("keep-s2b-classified");
    let h = TestHost::start_seeded(crate::test_harness::TestConfig::default(), 0);
    let mut classified: Vec<&str> = REQUIRED.iter().chain(&ALSO_CARRIED).chain(&NOT_YET_RESTORED).copied().collect();
    classified.sort_unstable();
    let names: Vec<&str> = h.server.sim_digest_sections().into_keys().collect();
    assert_eq!(classified, names, "a new digest section needs a place in these lists");
}

/// Twin worlds, not hash order or process state, decide the bytes: every map
/// here has its own random hash seed.
#[test]
fn twins_give_identical_checkpoint_bytes() {
    let _root = WorldsRootGuard::new("keep-s2b-twins");
    let mut a = fixture("ckpt-twin-a", 2);
    let mut b = fixture("ckpt-twin-b", 2);
    for at in [1u64, 200, 700] {
        run_to(&mut a, at);
        run_to(&mut b, at);
        settle(&mut a);
        settle(&mut b);
        assert_eq!(a.server.tick_counter, b.server.tick_counter);
        let (ca, cb) = (a.server.checkpoint().expect("a"), b.server.checkpoint().expect("b"));
        assert_eq!(ca, cb, "twins' checkpoints differ at tick {at}");
        assert_eq!(a.server.state_hash(), b.server.state_hash());
    }
    // A save writes the same records on both, so the manifests still agree.
    assert_eq!(persist_and_checkpoint(&mut a), persist_and_checkpoint(&mut b), "after a save");
}

#[test]
fn another_seed_gives_other_bytes() {
    let _root = WorldsRootGuard::new("keep-s2b-seed");
    let a = twin(SEED, "ckpt-seed-a", 0, 2);
    let b = twin(SEED + 1, "ckpt-seed-b", 0, 2);
    assert_ne!(a.server.checkpoint().unwrap(), b.server.checkpoint().unwrap());
}

/// The checkpoint moves with the state: a tick, a block, a joiner's step.
#[test]
fn the_bytes_follow_the_state() {
    let _root = WorldsRootGuard::new("keep-s2b-follow");
    let mut h = fixture("ckpt-follow", 1);
    let start = h.server.checkpoint().unwrap();
    h.set_block(9, BASE_Y + 3, 9, crate::block::STONE);
    let after_block = h.server.checkpoint().unwrap();
    assert_ne!(start, after_block);
    h.server.players[1].player.pos.x += 1.0;
    let after_step = h.server.checkpoint().unwrap();
    assert_ne!(after_block, after_step);
    run_to(&mut h, 1);
    assert_ne!(after_step, h.server.checkpoint().unwrap());
}

#[test]
fn minus_zero_gives_the_same_bytes() {
    let _root = WorldsRootGuard::new("keep-s2b-zero");
    let mut h = fixture("ckpt-zero", 1);
    h.server.players[1].yaw = 0.0;
    let plus = h.server.checkpoint().unwrap();
    h.server.players[1].yaw = -0.0;
    assert_eq!(plus, h.server.checkpoint().unwrap());
}

/// `state_hash` is exactly this, and the header holds what the spec lists.
#[test]
fn the_header_and_the_hash_are_as_specified() {
    use sha2::{Digest, Sha256};
    let _root = WorldsRootGuard::new("keep-s2b-header");
    let h = fixture("ckpt-header", 1);
    let bytes = h.server.checkpoint().unwrap();

    let mut hash = Sha256::new();
    hash.update(b"fray/authority-state/v1\0");
    hash.update(&bytes);
    let expected: [u8; 32] = hash.finalize().into();
    assert_eq!(h.server.state_hash().unwrap(), expected);

    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        &bytes[at - n..at]
    };
    assert_eq!(CHECKPOINT_MAGIC, b"axenstax/authority-checkpoint\0");
    assert_eq!(take(CHECKPOINT_MAGIC.len()), CHECKPOINT_MAGIC);
    assert_eq!(take(2), 3u16.to_le_bytes(), "schema");
    assert_eq!(take(4), crate::protocol::PROTOCOL_VERSION.to_le_bytes());
    assert_eq!(take(4), crate::save_format::SAVE_FORMAT_VERSION.to_le_bytes());
    assert_eq!(take(4), h.server.world.generator.version().to_le_bytes());
    assert_eq!(take(8), u64::from(SEED).to_le_bytes(), "the seed, widened");

    let mut binding = Sha256::new();
    binding.update(u64::from(SEED).to_le_bytes());
    binding.update(h.server.world.generator.version().to_le_bytes());
    binding.update([u8::from(h.server.is_creative), h.server.difficulty as u8]);
    let binding: [u8; 32] = binding.finalize().into();
    assert_eq!(take(32), binding, "config binding");

    let mut key_id = Sha256::new();
    key_id.update(b"axenstax/pop-key-id/v1\0");
    key_id.update(h.server.pop_secret);
    let key_id: [u8; 32] = key_id.finalize().into();
    assert_eq!(take(16), &key_id[..16], "pop key id");
    assert_eq!(take(8), h.server.tick_counter.to_le_bytes(), "the completed tick");
}

/// The secret never appears, and neither does the sub-key rolls are made with.
#[test]
fn the_pop_secret_is_never_in_the_bytes() {
    let _root = WorldsRootGuard::new("keep-s2b-secret");
    let mut h = fixture("ckpt-secret", 2);
    run_to(&mut h, 40);
    let bytes = persist_and_checkpoint(&mut h);
    let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(!contains(&h.server.pop_secret), "the secret is in the checkpoint");
    assert!(!contains(&crate::server::use_seed_key(&h.server.pop_secret)), "the use-roll key is in the checkpoint");
}

/// A meta carrying the secret doesn't put it in the bytes, and the digest of
/// the record doesn't move when only the secret does.
#[test]
fn the_meta_record_leaves_the_secret_out() {
    let _root = WorldsRootGuard::new("keep-s2b-meta");
    let mut meta = crate::save::WorldMeta::new("ckpt-meta");
    meta.pop_secret = Some([0x5a; 32]);
    crate::save::save_world_meta("ckpt-meta", &meta).expect("write meta");
    let open = || {
        let mut s = GameServer::new(0, "ckpt-meta".into(), meta.seed);
        s.render_distance = 0;
        s.initial_load().expect("load");
        s
    };
    let first = open();
    assert_eq!(first.pop_secret, [0x5a; 32], "the meta's secret was read");
    let bytes = first.checkpoint().expect("checkpoint");
    assert!(!bytes.windows(32).any(|w| w == [0x5a; 32]));
    let digest = first.stored_records.meta_digest();
    assert!(digest.is_some(), "a meta on disk is a record");

    let mut other = meta.clone();
    other.pop_secret = Some([0x77; 32]);
    crate::save::save_world_meta("ckpt-meta", &other).expect("write meta");
    assert_eq!(open().stored_records.meta_digest(), digest, "only the secret changed");
    other.description = "something else".into();
    crate::save::save_world_meta("ckpt-meta", &other).expect("write meta");
    assert_ne!(open().stored_records.meta_digest(), digest, "the record changed");
}

/// Reading a checkpoint writes nothing and changes nothing.
#[test]
fn a_checkpoint_is_observational() {
    let _root = WorldsRootGuard::new("keep-s2b-observe");
    let mut h = fixture("ckpt-observe", 2);
    run_to(&mut h, 10);
    let dir = crate::save::world_dir("ckpt-observe");
    let files = folder_listing(&dir);
    let digest = h.server.sim_digest();
    let bytes = h.server.checkpoint().unwrap();
    assert_eq!(digest, h.server.sim_digest(), "the state moved");
    assert_eq!(bytes, h.server.checkpoint().unwrap(), "the bytes moved");
    assert_eq!(files, folder_listing(&dir), "a checkpoint wrote to the world's folder");
    assert!(!h.server.world.knows_disk_chunk((0, 0, 0)), "a checkpoint touched the record of chunk files");

    h.server.try_save().unwrap();
    let known = |h: &TestHost| {
        h.server.world.persistable_chunks().filter(|(k, _)| h.server.world.knows_disk_chunk(*k)).count()
    };
    let before = known(&h);
    let files = folder_listing(&dir);
    h.server.checkpoint().unwrap();
    assert_eq!(known(&h), before);
    assert_eq!(files, folder_listing(&dir), "a checkpoint wrote to the world's folder");
}

/// Every file under `dir`, with its size and modification time.
fn folder_listing(dir: &std::path::Path) -> Vec<(std::path::PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let meta = entry.metadata().expect("metadata");
        if meta.is_dir() {
            out.extend(folder_listing(&entry.path()));
        } else {
            out.push((entry.path(), meta.len(), meta.modified().expect("mtime")));
        }
    }
    out.sort();
    out
}

/// Checkpoint, restore, checkpoint again: the same bytes, at more than one
/// point in the run (the later ones have columns evicted and dropped).
#[test]
fn a_round_trip_gives_identical_bytes() {
    let _root = WorldsRootGuard::new("keep-s2b-round-trip");
    let mut h = fixture("ckpt-round", 2);
    for at in [0u64, 200, 1100] {
        if at == 1100 {
            // An edit in the column the joiner is teleported to at tick 300 and
            // away from at 1000, so that column is evicted, not dropped.
            run_to(&mut h, 400);
            let at = h.player_pos(1);
            h.place_block(at.x as i32, BASE_Y, at.z as i32, crate::block::STONE);
        }
        run_to(&mut h, at);
        let bytes = persist_and_checkpoint(&mut h);
        assert!(bytes.len() < MAX_CHECKPOINT_BYTES);
        let restored = TestHost::adopt(restore(&h, &bytes).expect("restores"));
        let again = restored.server.checkpoint().expect("checkpoints again");
        assert_eq!(again, bytes, "the round trip at tick {at} changed the bytes");
        assert_eq!(restored.server.world.chunk_layout(), h.server.world.chunk_layout());
    }
    assert!(
        h.server.world.chunk_layout().unwrap().iter().any(|c| c.evicted_column),
        "the teleports evicted a column, so the layout really was tested"
    );
}

#[derive(Clone, Copy, PartialEq)]
enum Mobs {
    /// Both worlds keep the entities they have. They aren't the same entities,
    /// so a section a mob can touch may part.
    AsTheyAre,
    /// Both start again from an empty ECS at the checkpoint, so only what the
    /// restore carries can part them, and mobs that spawn from there on do too.
    FromEmpty,
}

/// Run to `at`, persist, checkpoint and restore, then feed the original and the
/// restored world the same script for 200 ticks and compare.
fn restore_and_continue(at: u64, mobs: Mobs, promised: &[&str]) {
    let name = format!("ckpt-continue-{at}-{}", mobs as u8);
    let _root = WorldsRootGuard::new(&name);
    let mut a = fixture(&name, 2);
    run_to(&mut a, at);
    let bytes = persist_and_checkpoint(&mut a);
    eprintln!("tick {at}: checkpoint {} B", bytes.len());
    let mut b = TestHost::adopt(restore(&a, &bytes).expect("restores"));
    if mobs == Mobs::FromEmpty {
        a.server.ecs = hecs::World::new();
        b.server.ecs = hecs::World::new();
    }
    assert_restored(&format!("tick {at}, straight after the restore"), &a.server, &b.server, promised);
    let (before, end) = (a.server.sim_digest_sections(), a.server.tick_counter + 200);
    run_to(&mut a, end);
    run_to(&mut b, end);
    assert_restored(&format!("tick {at} + 200"), &a.server, &b.server, promised);

    // The comparison isn't idle: these moved in those 200 ticks.
    let after = a.server.sim_digest_sections();
    let moved: Vec<_> = REQUIRED.iter().filter(|s| before[*s] != after[*s]).collect();
    eprintln!("tick {at} + 200: the original's moved sections {moved:?}");
    for section in ["block_entities", "clocks", "players"] {
        assert_ne!(before[section], after[section], "{section} never moved, so its comparison proved nothing");
    }
    if at < 2000 {
        assert_ne!(before["chunks"], after["chunks"], "chunks never moved, so their comparison proved nothing");
    }
}

/// What holds whatever the mobs do in this fixture. The joiners never meet a
/// mob in these 200 ticks; when one does, `players` parts
/// (`a_mob_on_a_joiner_parts_the_players_section`).
fn promised() -> Vec<&'static str> {
    REQUIRED.iter().chain(&ALSO_CARRIED).copied().collect()
}

/// From an empty ECS the two worlds are the same world, so all that may part
/// them is light, which a load recomputes. Entities spawned after the boundary
/// agree too: same state, same rolls, same ids.
fn promised_from_empty() -> Vec<&'static str> {
    let mut all = promised();
    all.extend(NOT_YET_RESTORED.iter().copied().filter(|s| *s != "light"));
    all
}

#[test]
fn restore_and_continue_from_tick_1_with_its_mobs() {
    restore_and_continue(1, Mobs::AsTheyAre, &promised());
}
#[test]
fn restore_and_continue_from_tick_200_with_its_mobs() {
    restore_and_continue(200, Mobs::AsTheyAre, &promised());
}
#[test]
fn restore_and_continue_from_tick_2000_with_its_mobs() {
    restore_and_continue(2000, Mobs::AsTheyAre, &promised());
}
/// A source is drained at tick 900, and its flowing water must drain the same:
/// a rescan alone would have made every flowing cell a source of its own.
#[test]
fn restore_and_continue_across_a_drained_source() {
    restore_and_continue(800, Mobs::AsTheyAre, &promised());
}
#[test]
fn restore_and_continue_from_tick_1_from_empty() {
    restore_and_continue(1, Mobs::FromEmpty, &promised_from_empty());
}
#[test]
fn restore_and_continue_from_tick_200_from_empty() {
    restore_and_continue(200, Mobs::FromEmpty, &promised_from_empty());
}
#[test]
fn restore_and_continue_from_tick_2000_from_empty() {
    restore_and_continue(2000, Mobs::FromEmpty, &promised_from_empty());
}

/// The limit of `players` (handoff §7): a mob on a joiner moves it, and a
/// restored world's mobs aren't the original's, so this section holds only
/// while no joiner meets one. Until entities are restored, this stands.
#[test]
fn a_mob_on_a_joiner_parts_the_players_section() {
    let _root = WorldsRootGuard::new("keep-s2b-mob-parts");
    let mut a = fixture("ckpt-mob-parts", 1);
    a.server.ecs = hecs::World::new();
    run_to(&mut a, 2);
    let bytes = persist_and_checkpoint(&mut a);
    let mut b = TestHost::adopt(restore(&a, &bytes).expect("restores"));
    b.server.ecs = hecs::World::new();
    let at = a.player_pos(1);
    crate::entity::spawn_mob(&mut a.server.ecs, crate::mob::MobType::Cow, at);
    let end = a.server.tick_counter + 5;
    run_to(&mut a, end);
    run_to(&mut b, end);
    assert!(differing(&a.server, &b.server).contains(&"players"), "a cow on a joiner didn't move it");
}

/// A column the run evicted comes back from the store as it left, and a column
/// the run dropped stays dropped though an earlier save left its files on disk.
#[test]
fn evicted_and_dropped_columns_stay_so() {
    let _root = WorldsRootGuard::new("keep-s2b-columns");
    let mut a = fixture("ckpt-columns", 1);
    let (near, far, home) = (
        Vec3::new(200.5, 120.0, 40.5),
        Vec3::new(-150.5, 120.0, -60.5),
        Vec3::new(6.5, (BASE_Y + 1) as f32, 20.5),
    );
    let step = |h: &mut TestHost, ticks: u64| {
        let end = h.server.tick_counter + ticks;
        while h.server.tick_counter < end {
            h.server.tick();
        }
    };
    // Edited, so evicted when the joiner leaves.
    a.teleport_player(1, near);
    step(&mut a, 60);
    a.place_block(200, BASE_Y, 40, crate::block::STONE);
    // Pristine and saved, so dropped when the joiner leaves, its files stale.
    a.teleport_player(1, far);
    step(&mut a, 60);
    a.server.try_save().expect("saves with the far column held");
    a.teleport_player(1, home);
    step(&mut a, 60);
    settle(&mut a);
    a.server.try_save().expect("saves");
    let (cx, cz) = (200i32.div_euclid(16), 40i32.div_euclid(16));
    let (fx, fz) = ((-151i32).div_euclid(16), (-61i32).div_euclid(16));
    assert!(a.server.world.is_column_evicted(cx, cz), "the edited column was evicted");
    assert!(!a.server.loaded_columns.contains(&(fx, fz)), "the pristine column was let go");
    assert!(!a.server.world.is_column_evicted(fx, fz), "and dropped, not evicted");
    assert!(
        std::fs::read_dir(crate::save::world_dir("ckpt-columns").join("chunks"))
            .unwrap()
            .flatten()
            .any(|f| f.file_name().to_string_lossy().starts_with(&format!("{fx}_"))),
        "the dropped column left files for the load to read"
    );

    let bytes = a.server.checkpoint().expect("checkpoints");
    let mut b = TestHost::adopt(restore(&a, &bytes).expect("restores"));
    assert!(b.server.world.is_column_evicted(cx, cz));
    assert!(!b.server.loaded_columns.contains(&(fx, fz)));
    a.server.ecs = hecs::World::new();
    b.server.ecs = hecs::World::new();
    assert_restored("columns, restored", &a.server, &b.server, &promised());
    // The joiner goes back to both columns, and leaves again.
    for (to, ticks) in [(near, 60), (far, 60), (home, 60)] {
        a.teleport_player(1, to);
        b.teleport_player(1, to);
        step(&mut a, ticks);
        step(&mut b, ticks);
        assert_restored("columns, walked", &a.server, &b.server, &promised());
    }
}

/// A restored joiner has what its sidecar and the checkpoint carry, and a save
/// before it re-attaches leaves the record it was restored from.
#[test]
fn a_restored_joiner_saves_the_record_it_came_from() {
    let _root = WorldsRootGuard::new("keep-s2b-sidecar");
    let mut a = fixture("ckpt-sidecar", 1);
    let _ = a.server.players[1].inventory.add_item(crate::item::ItemStack::new_block(crate::block::COBBLESTONE, 9));
    run_to(&mut a, 50);
    let bytes = persist_and_checkpoint(&mut a);
    let path = a.server.joiner_path(a.server.players[1].verified_pubkey.unwrap());
    let record = std::fs::read(&path).expect("the save wrote a sidecar");
    let b = TestHost::adopt(restore(&a, &bytes).expect("restores"));
    assert_eq!(b.server.players[1].player.pos, a.server.players[1].player.pos);
    assert_eq!(b.server.players[1].inventory.slot(0).map(|s| s.count), Some(9));
    b.server.try_save().expect("the restored world saves");
    assert_eq!(std::fs::read(&path).unwrap(), record, "the restored joiner's save changed its record");
}

/// The cap in numbers (handoff §6, Fray list): the checkpoint's bytes at the
/// world sizes of the slice 1 bench. Run by name with `--ignored`.
#[test]
#[ignore]
fn checkpoint_size_at_the_bench_world_sizes() {
    for joiners in [0usize, 2, 8] {
        let name = format!("keep-s2b-size-{joiners}");
        let _root = WorldsRootGuard::new(&name);
        let mut h = twin_loaded(SEED, &name, 0, super::tick_bench::SIM_DISTANCE, super::tick_bench::SIM_DISTANCE);
        super::tick_bench::add_bench_joiners(&mut h, joiners);
        super::tick_bench::warm_up(&mut h);
        super::tick_bench::scripted_build(&mut h);
        settle(&mut h);
        h.server.try_save().expect("the bench world saves");
        let manifest = crate::authority_checkpoint::manifest_len(&h.server).expect("a manifest");
        let chunks = h.server.world.persistable_chunks().filter(|(_, c)| !c.is_empty()).count();
        let per_chunk = manifest as f64 / chunks.max(1) as f64;
        let sections = crate::authority_checkpoint::section_sizes(&h.server).expect("the sections");
        let volatile: usize = sections.iter().filter(|(n, _)| !n.starts_with(' ')).map(|(_, b)| b).sum();
        let result = h.server.checkpoint().map(|b| b.len());
        eprintln!(
            "checkpoint_size joiners={joiners}: checkpoint() {result:?}; manifest {manifest} B for {chunks} non-empty \
             chunks ({per_chunk:.1} B each); volatile {volatile} B {sections:?}; columns {}, entities {}; \
             with no volatile the cap is near {:.0} chunks",
            h.server.loaded_columns.len(),
            h.server.ecs.len(),
            MAX_CHECKPOINT_BYTES as f64 / per_chunk,
        );
    }
}

/// A joiner stands at a spot the restore then loads a body for.
#[test]
fn a_restored_world_keeps_each_slot_in_its_place() {
    let _root = WorldsRootGuard::new("keep-s2b-slots");
    let mut a = fixture("ckpt-slots", 3);
    let late = add_joiner(&mut a, Vec3::new(10.5, (BASE_Y + 1) as f32, 20.5));
    a.server.players[late].connected = false;
    run_to(&mut a, 30);
    let bytes = persist_and_checkpoint(&mut a);
    let b = TestHost::adopt(restore(&a, &bytes).expect("restores"));
    assert_eq!(b.server.players.len(), a.server.players.len());
    for (i, (pa, pb)) in a.server.players.iter().zip(&b.server.players).enumerate() {
        assert_eq!(pa.player.pos, pb.player.pos, "slot {i}");
        assert_eq!(pa.connected, pb.connected, "slot {i}");
        assert_eq!(pa.server_simulated, pb.server_simulated, "slot {i}");
        assert_eq!(pa.verified_pubkey, pb.verified_pubkey, "slot {i}");
    }
}

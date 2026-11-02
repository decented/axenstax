//! Keep slice 2b — what a checkpoint and a restore refuse, each under the code
//! the contract gives it (Spec 01 §4.1.5).

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, SEED};
use super::twin_run::{run_to, twin};
use crate::authority_checkpoint::{seal, CheckpointError, RestoreError, CHECKPOINT_MAGIC, MAX_CHECKPOINT_BYTES};
use crate::save::WorldsRootGuard;
use crate::server::GameServer;
use crate::test_harness::TestHost;

fn saved(tag: &str) -> (WorldsRootGuard, TestHost, Vec<u8>) {
    let root = WorldsRootGuard::new(tag);
    let mut h = fixture(tag, 2);
    run_to(&mut h, 20);
    let bytes = persist_and_checkpoint(&mut h);
    (root, h, bytes)
}

fn assert_invalid_checkpoint(h: &TestHost, bytes: &[u8], what: &str) {
    match restore(h, bytes).err() {
        Some(RestoreError::InvalidCheckpoint(_)) => {}
        other => panic!("{what}: expected InvalidCheckpoint, got {other:?}"),
    }
}

fn assert_invalid_config(h: &TestHost, config: &crate::authority_checkpoint::AuthorityConfig, bytes: &[u8], what: &str) {
    let dir = crate::save::world_dir(&h.server.world_name);
    match GameServer::restore_from_checkpoint(config, bytes, &dir).err() {
        Some(RestoreError::InvalidConfig(_)) => {}
        other => panic!("{what}: expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn a_newer_schema_is_unsupported() {
    let (_root, h, mut bytes) = saved("keep-s2b-newer");
    let at = CHECKPOINT_MAGIC.len();
    bytes[at..at + 2].copy_from_slice(&2u16.to_le_bytes());
    match restore(&h, &bytes).err() {
        Some(RestoreError::UnsupportedVersion(_)) => {}
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

/// Schema 0 never existed: a bad checkpoint, not a future one.
#[test]
fn a_schema_that_never_was_is_invalid() {
    let (_root, h, mut bytes) = saved("keep-s2b-schema-0");
    let at = CHECKPOINT_MAGIC.len();
    bytes[at..at + 2].copy_from_slice(&0u16.to_le_bytes());
    assert_invalid_checkpoint(&h, &bytes, "schema 0");
}

#[test]
fn bad_magic_is_invalid() {
    let (_root, h, mut bytes) = saved("keep-s2b-magic");
    bytes[0] ^= 1;
    assert_invalid_checkpoint(&h, &bytes, "magic");
}

/// Cut anywhere, a checkpoint is refused, never read as far as it goes.
#[test]
fn truncated_bytes_are_invalid() {
    let (_root, h, bytes) = saved("keep-s2b-truncated");
    let n = bytes.len();
    for cut in [0, 10, CHECKPOINT_MAGIC.len(), CHECKPOINT_MAGIC.len() + 1, 60, 100, n / 2, n - 33, n - 32, n - 1] {
        assert_invalid_checkpoint(&h, &bytes[..cut], &format!("cut at {cut} of {n}"));
    }
}

#[test]
fn trailing_bytes_are_invalid() {
    let (_root, h, mut bytes) = saved("keep-s2b-trailing");
    bytes.push(0);
    assert_invalid_checkpoint(&h, &bytes, "one byte more");
}

/// A flipped bit anywhere in the body, without resealing, fails the seal.
#[test]
fn a_flipped_bit_is_invalid() {
    let (_root, h, mut bytes) = saved("keep-s2b-flip");
    let at = bytes.len() / 2;
    bytes[at] ^= 1;
    assert_invalid_checkpoint(&h, &bytes, "flipped bit");
}

/// A manifest record changed by someone who also fixes the seal: the record
/// doesn't match what the load read.
#[test]
fn a_tampered_manifest_record_is_invalid() {
    let (_root, h, mut bytes) = saved("keep-s2b-manifest");
    // The last manifest entry's digest ends just before the seal.
    let at = bytes.len() - 33;
    bytes[at] ^= 1;
    seal(&mut bytes);
    assert_invalid_checkpoint(&h, &bytes, "resealed manifest");
}

/// The header's tick, changed and resealed, disagrees with the clock section.
#[test]
fn a_header_that_disagrees_with_the_clock_is_invalid() {
    let (_root, h, mut bytes) = saved("keep-s2b-reseal");
    let at = CHECKPOINT_MAGIC.len() + 2 + 4 + 4 + 4 + 8 + 32 + 16;
    bytes[at] ^= 1;
    seal(&mut bytes);
    assert_invalid_checkpoint(&h, &bytes, "resealed tick");
}

#[test]
fn a_changed_chunk_file_is_invalid() {
    let (_root, h, bytes) = saved("keep-s2b-chunk-file");
    let dir = crate::save::world_dir(&h.server.world_name).join("chunks");
    let file = std::fs::read_dir(&dir).unwrap().flatten().next().expect("a chunk file").path();
    let mut chunk = std::fs::read(&file).unwrap();
    chunk[0] ^= 1;
    std::fs::write(&file, &chunk).unwrap();
    assert_invalid_checkpoint(&h, &bytes, "changed chunk file");
}

#[test]
fn a_missing_chunk_file_is_invalid() {
    let (_root, h, bytes) = saved("keep-s2b-chunk-gone");
    let dir = crate::save::world_dir(&h.server.world_name).join("chunks");
    let file = std::fs::read_dir(&dir).unwrap().flatten().next().expect("a chunk file").path();
    std::fs::remove_file(&file).unwrap();
    assert_invalid_checkpoint(&h, &bytes, "missing chunk file");
}

#[test]
fn a_changed_sidecar_is_invalid() {
    let (_root, h, bytes) = saved("keep-s2b-sidecar-file");
    let dir = crate::save::world_dir(&h.server.world_name).join("joiners");
    let file = std::fs::read_dir(&dir).unwrap().flatten().next().expect("a sidecar").path();
    let mut record = std::fs::read(&file).unwrap();
    let at = record.len() - 1;
    record[at] ^= 1;
    std::fs::write(&file, &record).unwrap();
    assert_invalid_checkpoint(&h, &bytes, "changed sidecar");
}

#[test]
fn a_sidecar_the_manifest_does_not_name_is_invalid() {
    let (_root, h, bytes) = saved("keep-s2b-sidecar-extra");
    let dir = crate::save::world_dir(&h.server.world_name).join("joiners");
    let file = std::fs::read_dir(&dir).unwrap().flatten().next().expect("a sidecar").path();
    let extra = dir.join(format!("{}.dat", crate::hosted_server::pubkey_to_npub(&[9; 32])));
    std::fs::copy(&file, &extra).unwrap();
    assert_invalid_checkpoint(&h, &bytes, "extra sidecar");
}

#[test]
fn a_world_with_no_records_is_invalid() {
    let (_root, h, bytes) = saved("keep-s2b-gone");
    std::fs::remove_dir_all(crate::save::world_dir(&h.server.world_name)).unwrap();
    assert_invalid_checkpoint(&h, &bytes, "world folder gone");
}

#[test]
fn a_wrong_pop_key_is_invalid_config() {
    let (_root, h, bytes) = saved("keep-s2b-key");
    let mut config = h.server.authority_config();
    config.pop_secret[0] ^= 1;
    assert_invalid_config(&h, &config, &bytes, "wrong key");
}

#[test]
fn another_binding_is_invalid_config() {
    let (_root, h, bytes) = saved("keep-s2b-binding");
    let good = h.server.authority_config();
    let mut seed = good.clone();
    seed.seed += 1;
    assert_invalid_config(&h, &seed, &bytes, "seed");
    let mut wide = good.clone();
    wide.seed += 1 << 32;
    assert_invalid_config(&h, &wide, &bytes, "a seed past u32");
    let mut version = good.clone();
    version.worldgen_version += 1;
    assert_invalid_config(&h, &version, &bytes, "worldgen version");
    let mut creative = good.clone();
    creative.creative = !creative.creative;
    assert_invalid_config(&h, &creative, &bytes, "creative");
    let mut difficulty = good;
    difficulty.difficulty = crate::survival::Difficulty::Hard;
    assert_invalid_config(&h, &difficulty, &bytes, "difficulty");
}

#[test]
fn oversize_bytes_are_invalid_on_restore() {
    let (_root, h, mut bytes) = saved("keep-s2b-oversize-in");
    bytes.resize(MAX_CHECKPOINT_BYTES + 1, 0);
    assert_invalid_checkpoint(&h, &bytes, "over the cap");
}

/// Past the cap a checkpoint is an error, never a shorter checkpoint.
#[test]
fn oversize_state_is_an_error_not_a_truncation() {
    let _root = WorldsRootGuard::new("keep-s2b-oversize");
    let mut h = twin(SEED, "ckpt-oversize", 0, 2);
    let cells: Vec<_> = (0..60_000).map(|i| ((i, 200, 0), 1_000_000u64, 0u8)).collect();
    h.server.fire.restore_cells(&cells);
    match h.server.checkpoint() {
        Err(CheckpointError::TooLarge { bytes }) => assert!(bytes > MAX_CHECKPOINT_BYTES, "{bytes}"),
        other => panic!("expected TooLarge, got {:?}", other.map(|b| b.len())),
    }
    assert!(matches!(h.server.state_hash(), Err(CheckpointError::TooLarge { .. })));
}

#[test]
fn pending_deaths_are_refused_until_the_next_tick() {
    let _root = WorldsRootGuard::new("keep-s2b-deaths");
    let mut h = twin(SEED, "ckpt-deaths", 0, 2);
    let at = glam::Vec3::new(4.5, 101.0, 20.5);
    let cow = crate::entity::spawn_mob(&mut h.server.ecs, crate::mob::MobType::Cow, at);
    h.server.ecs.get::<&mut crate::combat::Health>(cow).unwrap().current = 0.0;
    assert_eq!(h.server.checkpoint(), Err(CheckpointError::DeathsPending));
    assert_eq!(h.server.state_hash(), Err(CheckpointError::DeathsPending));
    let next = h.server.tick_counter + 1;
    run_to(&mut h, next);
    h.server.checkpoint().expect("the death was swept at the tick boundary");
}

#[test]
fn a_lent_server_has_no_world_to_checkpoint() {
    let _root = WorldsRootGuard::new("keep-s2b-lent");
    let mut h = twin(SEED, "ckpt-lent", 0, 2);
    h.server.lent = true;
    assert_eq!(h.server.checkpoint(), Err(CheckpointError::Lent));
}

/// Refused checkpoints do no harm: the world still restores from the real one.
#[test]
fn a_refusal_leaves_the_world_able_to_restore() {
    let (_root, h, bytes) = saved("keep-s2b-after");
    let mut bad = bytes.clone();
    bad[0] ^= 1;
    assert_invalid_checkpoint(&h, &bad, "bad magic");
    let mut config = h.server.authority_config();
    config.pop_secret[0] ^= 1;
    assert_invalid_config(&h, &config, &bytes, "wrong key");
    restore(&h, &bytes).expect("the real one still restores");
}

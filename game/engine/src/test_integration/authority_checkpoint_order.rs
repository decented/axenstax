//! Keep slice 3: the order a restore checks things in and the code each answer
//! gets, digests over a record's stored bytes, and what a damaged meta does
//! (Spec 01 §4.1.5).

use sha2::{Digest, Sha256};

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, SEED};
use super::authority_checkpoint_records::{fixture_with_meta, folder_bytes};
use super::authority_checkpoint_refusals::{assert_invalid_checkpoint, assert_invalid_config, saved};
use super::twin_run::run_to;
use crate::authority_checkpoint::{config_binding, manifest_entries, seal, RestoreError, CHECKPOINT_MAGIC, MAX_CHECKPOINT_BYTES, SCHEMA};
use crate::save::{world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::test_harness::TestHost;

const SCHEMA_AT: usize = CHECKPOINT_MAGIC.len();
const PROTOCOL_AT: usize = SCHEMA_AT + 2;
const SAVE_FORMAT_AT: usize = PROTOCOL_AT + 4;
const SEED_AT: usize = SAVE_FORMAT_AT + 4 + 4;
const BINDING_AT: usize = SEED_AT + 8;
const TICK_AT: usize = BINDING_AT + 32 + 16;

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn with_schema(mut bytes: Vec<u8>, schema: u16) -> Vec<u8> {
    bytes[SCHEMA_AT..SCHEMA_AT + 2].copy_from_slice(&schema.to_le_bytes());
    bytes
}

fn assert_unsupported(result: Result<GameServer, RestoreError>, what: &str) {
    match result.err() {
        Some(RestoreError::UnsupportedVersion(_)) => {}
        other => panic!("{what}: expected UnsupportedVersion, got {other:?}"),
    }
}

// ---- 1. The order ----

/// The schema is read before anything else, the cap included: a later schema's
/// cap isn't this one's to judge.
#[test]
fn a_newer_schema_past_the_cap_is_unsupported() {
    let (_root, h, bytes) = saved("keep-s3-schema-cap");
    let mut big = with_schema(bytes, SCHEMA + 1);
    big.resize(MAX_CHECKPOINT_BYTES + 1, 0);
    assert_unsupported(restore(&h, &big), "a newer schema over the cap");
}

/// Any schema but this one is a schema this build can't read, older ones too.
#[test]
fn any_schema_this_build_cannot_read_is_unsupported() {
    let (_root, h, bytes) = saved("keep-s3-schema-any");
    for schema in (0..SCHEMA).chain([SCHEMA + 1, u16::MAX]) {
        assert_unsupported(restore(&h, &with_schema(bytes.clone(), schema)), &format!("schema {schema}"));
    }
}

/// Only the first fields matter to that answer: nothing after the schema is read.
#[test]
fn a_newer_schema_is_named_whatever_follows_it() {
    let (_root, h, bytes) = saved("keep-s3-schema-short");
    let newer = with_schema(bytes, SCHEMA + 1);
    assert_unsupported(restore(&h, &newer[..SCHEMA_AT + 2]), "cut straight after the schema");
    let mut unsealed = newer.clone();
    let last = unsealed.len() - 1;
    unsealed[last] ^= 1;
    assert_unsupported(restore(&h, &unsealed), "a bad seal behind it");
}

/// The seal before the versions, the versions before the config.
#[test]
fn versions_are_read_after_the_seal_and_before_the_config() {
    let (_root, h, bytes) = saved("keep-s3-versions");
    let mut wrong_key = h.server.authority_config();
    wrong_key.pop_secret[0] ^= 1;
    for at in [PROTOCOL_AT, SAVE_FORMAT_AT] {
        let mut newer = bytes.clone();
        newer[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        // No fresh seal: damage, whichever field it landed in.
        assert_invalid_checkpoint(&h, &newer, "a version field under a bad seal");
        seal(&mut newer);
        assert_unsupported(restore(&h, &newer), "a newer version");
        let dir = world_dir(&h.server.world_name);
        assert_unsupported(GameServer::restore_from_checkpoint(&wrong_key, &newer, &dir), "a newer version and another config");
    }
}

/// The config before the records.
#[test]
fn a_wrong_config_is_named_before_a_changed_record() {
    let (_root, h, bytes) = saved("keep-s3-config-first");
    let dir = world_dir(&h.server.world_name).join("chunks");
    let file = std::fs::read_dir(&dir).unwrap().flatten().next().expect("a chunk file").path();
    let mut chunk = std::fs::read(&file).unwrap();
    chunk[0] ^= 1;
    std::fs::write(&file, &chunk).unwrap();
    let mut config = h.server.authority_config();
    config.pop_secret[0] ^= 1;
    assert_invalid_config(&h, &config, &bytes, "another config and a changed record");
}

/// A seed no server can run is the config's answer, whatever follows the header.
#[test]
fn a_seed_past_a_servers_range_is_invalid_config_before_the_body() {
    let (_root, h, bytes) = saved("keep-s3-config-seed");
    let mut config = h.server.authority_config();
    config.seed += 1 << 33;
    let mut hostile = bytes[..TICK_AT + 8].to_vec();
    hostile[SEED_AT..SEED_AT + 8].copy_from_slice(&config.seed.to_le_bytes());
    let binding = config_binding(config.seed, config.worldgen_version, config.creative, config.difficulty);
    hostile[BINDING_AT..BINDING_AT + 32].copy_from_slice(&binding);
    hostile.extend_from_slice(&[0xff; 64]);
    seal(&mut hostile);
    assert_invalid_config(&h, &config, &hostile, "a seed past u32");
}

/// The bytes of the record decide, before any load has parsed them: a meta that
/// names another generator is a changed record, not another config.
#[test]
fn a_meta_naming_another_generator_is_a_record_mismatch() {
    let _root = WorldsRootGuard::new("keep-s3-meta-gen");
    let mut h = fixture_with_meta("ckpt-meta-gen");
    run_to(&mut h, 20);
    let bytes = persist_and_checkpoint(&mut h);
    let mut meta = crate::save::load_world_meta("ckpt-meta-gen");
    meta.worldgen_version = crate::biome::Generator::V3.version();
    crate::save::save_world_meta("ckpt-meta-gen", &meta).expect("write meta");
    assert_invalid_checkpoint(&h, &bytes, "a meta naming another generator");
}

// ---- 2. Digests over stored bytes ----

/// A meta's stored bytes with its secret's value cut, by a route that shares
/// nothing with the engine's: the test's own meta files are written so that
/// `"pop_secret": [` is unique and its value holds no `]`.
fn stored_without_secret(stored: &[u8]) -> Vec<u8> {
    let text = std::str::from_utf8(stored).unwrap();
    let Some(key) = text.find("\"pop_secret\"") else { return stored.to_vec() };
    let open = key + text[key..].find('[').unwrap();
    let close = open + text[open..].find(']').unwrap() + 1;
    [&stored[..open], &stored[close..]].concat()
}

/// Every record the manifest names is digested as it lies on disk.
#[test]
fn the_manifest_binds_each_record_by_its_stored_bytes() {
    let _root = WorldsRootGuard::new("keep-s3-stored");
    let mut h = fixture_with_meta("ckpt-stored");
    run_to(&mut h, 20);
    h.server.persist_authority().expect("persists");
    let dir = world_dir("ckpt-stored");
    let (mut chunks, mut sidecars, mut metas) = (0, 0, 0);
    for (key, digest) in manifest_entries(&h.server).expect("entries") {
        match key[0] {
            b'c' => {
                let at = |i: usize| i32::from_le_bytes(key[i..i + 4].try_into().unwrap());
                let file = dir.join("chunks").join(format!("{}_{}_{}.chunk", at(1), at(5), at(9)));
                assert_eq!(digest, sha256(&std::fs::read(&file).unwrap()), "{}", file.display());
                chunks += 1;
            }
            b'j' => {
                let file = dir.join("joiners").join(std::str::from_utf8(&key[1..]).unwrap());
                assert_eq!(digest, sha256(&std::fs::read(&file).unwrap()), "{}", file.display());
                sidecars += 1;
            }
            b'm' => {
                let stored = std::fs::read(dir.join("world_meta.json")).unwrap();
                assert_eq!(digest, sha256(&stored_without_secret(&stored)), "the meta");
                metas += 1;
            }
            _ => {}
        }
    }
    assert!(chunks > 0 && sidecars > 0, "{chunks} chunks, {sidecars} sidecars");
    assert_eq!(metas, 1);
}

/// A meta this build would write differently (an older build's: fields missing,
/// its own spacing; a newer one's: fields this one doesn't know) is unchanged as
/// long as its bytes are. Re-encoding the parsed struct would move the digest
/// the moment a build added a field.
#[test]
fn a_meta_this_build_would_write_differently_keeps_its_digest_and_restores() {
    let _root = WorldsRootGuard::new("keep-s3-old-meta");
    let name = "ckpt-old-meta";
    let mut meta = crate::save::WorldMeta::new(name);
    meta.seed = SEED;
    meta.worldgen_version = crate::biome::Generator::V2.version();
    meta.pop_secret = Some([7; 32]);
    let mut json: serde_json::Value = serde_json::to_value(&meta).unwrap();
    let object = json.as_object_mut().unwrap();
    assert!(object.remove("world_type").is_some(), "a field an older build wouldn't have written");
    object.insert("a_field_from_another_build".into(), 1.into());
    let stored = serde_json::to_vec(&json).unwrap();
    std::fs::create_dir_all(world_dir(name)).unwrap();
    std::fs::write(world_dir(name).join("world_meta.json"), &stored).unwrap();

    let mut h = fixture(name, 1);
    run_to(&mut h, 20);
    let bytes = persist_and_checkpoint(&mut h);
    assert_eq!(std::fs::read(world_dir(name).join("world_meta.json")).unwrap(), stored, "a save leaves the meta alone");
    assert_eq!(h.server.stored_records.meta_digest(), Some(sha256(&stored_without_secret(&stored))));
    let back = TestHost::adopt(restore(&h, &bytes).expect("restores"));
    assert_eq!(back.server.checkpoint().expect("again"), bytes);
}

/// Every byte is the record, a space that no parser would notice included.
#[test]
fn a_change_a_parser_would_not_notice_is_still_a_mismatch() {
    let _root = WorldsRootGuard::new("keep-s3-meta-space");
    let mut h = fixture_with_meta("ckpt-meta-space");
    run_to(&mut h, 20);
    let bytes = persist_and_checkpoint(&mut h);
    let file = world_dir("ckpt-meta-space").join("world_meta.json");
    let mut stored = std::fs::read(&file).unwrap();
    stored.push(b'\n');
    std::fs::write(&file, &stored).unwrap();
    assert_invalid_checkpoint(&h, &bytes, "a trailing newline in the meta");
}

// ---- 5. A damaged meta ----

/// From a good root a torn record doesn't happen, so this is a refusal: nothing
/// is repaired, kept aside or rebuilt on the way to it.
#[test]
fn a_torn_meta_is_refused_and_left_exactly_as_it_was() {
    let _root = WorldsRootGuard::new("keep-s3-meta-torn");
    let mut h = fixture_with_meta("ckpt-meta-torn");
    run_to(&mut h, 20);
    let bytes = persist_and_checkpoint(&mut h);
    let dir = world_dir("ckpt-meta-torn");
    std::fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
    let before = folder_bytes(&dir);
    assert_invalid_checkpoint(&h, &bytes, "a torn meta");
    assert_eq!(folder_bytes(&dir), before, "the refused restore changed the folder");
}

/// A meta lost beside a kept-aside copy is what a load would rebuild from.
#[test]
fn a_meta_the_load_would_rebuild_is_refused_and_left_exactly_as_it_was() {
    let (_root, h, bytes) = saved("keep-s3-meta-aside");
    let dir = world_dir(&h.server.world_name);
    std::fs::write(dir.join("world_meta.json.corrupt-1"), b"{ torn").unwrap();
    let before = folder_bytes(&dir);
    assert_invalid_checkpoint(&h, &bytes, "a meta to rebuild");
    assert_eq!(folder_bytes(&dir), before, "the refused restore changed the folder");
}

/// A server opened over a torn meta keeps the recovery a normal open has, and
/// its digest is of the file that recovery wrote, not of the defaults it read
/// before: the run's checkpoints still restore.
#[test]
fn a_run_opened_over_a_torn_meta_binds_the_repaired_file() {
    let _root = WorldsRootGuard::new("keep-s3-meta-repair");
    let name = "ckpt-meta-repair";
    let mut first = fixture_with_meta(name);
    run_to(&mut first, 20);
    first.server.try_save().expect("saves");
    let dir = world_dir(name);
    std::fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();

    let mut server = GameServer::new(0, name.into(), SEED);
    server.render_distance = 0;
    server.initial_load().expect("the recovery still opens the world");
    let repaired = std::fs::read(dir.join("world_meta.json")).unwrap();
    assert_ne!(repaired, b"{ torn");
    assert_eq!(server.stored_records.meta_digest(), Some(sha256(&stored_without_secret(&repaired))));
    server.persist_authority().expect("persists");
    let bytes = server.checkpoint().expect("checkpoints");
    let back = GameServer::restore_from_checkpoint(&server.authority_config(), &bytes, &dir).expect("restores");
    assert_eq!(back.checkpoint().expect("again"), bytes);
}

/// However the file came to change, a persist binds what it then holds.
#[test]
fn a_meta_written_during_a_run_is_bound_by_the_next_persist() {
    let _root = WorldsRootGuard::new("keep-s3-meta-run");
    let mut h = fixture_with_meta("ckpt-meta-run");
    run_to(&mut h, 20);
    let mut meta = crate::save::load_world_meta("ckpt-meta-run");
    meta.description = "written during the run".into();
    crate::save::save_world_meta("ckpt-meta-run", &meta).expect("write meta");
    let bytes = persist_and_checkpoint(&mut h);
    restore(&h, &bytes).expect("restores");
}

/// A dedicated server writes back a secret its world's meta lacked, after the
/// load. Its digest follows at once, with no persist between.
#[test]
fn a_secret_settled_after_the_load_keeps_the_digest_true() {
    let _root = WorldsRootGuard::new("keep-s3-meta-settle");
    let name = "ckpt-meta-settle";
    let mut meta = crate::save::WorldMeta::new(name);
    meta.seed = SEED;
    meta.worldgen_version = crate::biome::Generator::V2.version();
    meta.pop_secret = None;
    crate::save::save_world_meta(name, &meta).expect("write meta");
    let mut server = GameServer::new(0, name.into(), SEED);
    server.render_distance = 0;
    server.initial_load().expect("load");
    crate::server_main::settle_pop_secret(name, &mut server);
    let stored = std::fs::read(world_dir(name).join("world_meta.json")).unwrap();
    assert!(serde_json::from_slice::<crate::save::WorldMeta>(&stored).unwrap().pop_secret.is_some());
    assert_eq!(server.stored_records.meta_digest(), Some(sha256(&stored_without_secret(&stored))));
}

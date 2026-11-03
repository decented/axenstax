//! Keep slices 2b and 3 — the authority's canonical checkpoint and its restore
//! (Spec 01 §4.1.5). A pure, bounded byte form of what a save doesn't hold, plus a
//! digest manifest of what it does (handoff §6, choice 1). An API with tests: nothing
//! in `--keep-world` calls it yet, and nothing here touches the pipe or the wire.
//!
//! ## Layout, in order
//!
//! ```text
//! magic         b"axenstax/authority-checkpoint\0"
//! schema        u16 (2)
//! protocol      u32   PROTOCOL_VERSION
//! save format   u32   SAVE_FORMAT_VERSION
//! worldgen      u32   the world's generator version
//! seed          u64   widened
//! binding       [32]  SHA-256 of seed, worldgen, creative and difficulty
//! pop key id    [16]  SHA256(b"axenstax/pop-key-id/v1\0" || pop_secret)[..16]
//! tick          u64   the completed tick
//! volatile            `volatile.rs`, then `players.rs`, then `deaths.rs`
//! manifest            `manifest.rs`: sorted (record key, SHA-256)
//! seal          [32]  SHA-256 of everything before it
//! ```
//!
//! Fixed-width little-endian integers, `u32` length prefixes, every set in sorted
//! order, every queue in its queue order (which is state), floats as bits with
//! `-0.0` folded onto `0.0` (`codec.rs`). The hard cap is 1 MiB: past it a
//! checkpoint is an error, never a shorter checkpoint.
//!
//! The seal isn't in the slice 2 spec's list. It lets a restore see damage to the
//! volatile sections, which nothing else on disk can check. It is a checksum, not
//! a signature, and nothing here tells one kind of damage from another: whatever
//! doesn't match is `InvalidCheckpoint`.
//!
//! The secret itself is never in the bytes: only the key id, and the meta record
//! is hashed with the value of `pop_secret` cut from it.
//!
//! ## A checkpoint that can't be restored is never written
//!
//! `checkpoint` puts its own bytes through the decoder and range checks `restore`
//! uses (`decode_body`), so each limit is enforced at both ends by one piece of
//! code, and a limit added later is enforced at both by being added once.
//!
//! ## Observational
//!
//! `checkpoint` takes `&self`, does no I/O and calls no save path (nothing touches
//! `World::disk_chunks`). What it needs of the disk, `StoredRecords` learnt when
//! the records were read or written. It reads no clock and no per-run value:
//! `run_id` and the reputation clock's owner are left out. `calendar_id` is in, as
//! part of the 2a clock: it is random on a real server but persisted since 2a, so
//! two authorities opened from one saved world agree (two separate fresh worlds
//! aren't the same world and needn't).
//!
//! `state_hash` is exactly `SHA256(b"fray/authority-state/v1\0" || checkpoint())`.

#![cfg_attr(not(test), allow(dead_code))] // The API, not wired into `--keep-world` yet.

mod codec;
mod deaths;
mod manifest;
mod players;
mod restore;
mod volatile;

use sha2::{Digest, Sha256};

use crate::server::GameServer;

pub(crate) use manifest::StoredRecords;

pub(crate) const CHECKPOINT_MAGIC: &[u8] = b"axenstax/authority-checkpoint\0";
/// 2: pending deaths, and the meta digested as stored. Nothing relies on 1.
pub(crate) const SCHEMA: u16 = 2;
/// The session-image cap of migration v0.1 (handoff §6, choice 1).
pub(crate) const MAX_CHECKPOINT_BYTES: usize = 1 << 20;

const POP_KEY_ID_LABEL: &[u8] = b"axenstax/pop-key-id/v1\0";
const STATE_HASH_LABEL: &[u8] = b"fray/authority-state/v1\0";
const SCHEMA_BYTES: usize = 2;
/// Past the magic, the schema and the protocol and save-format versions.
const AFTER_VERSIONS: usize = CHECKPOINT_MAGIC.len() + SCHEMA_BYTES + 4 + 4;
const HEADER_BYTES: usize = AFTER_VERSIONS + 4 + 8 + 32 + 16 + 8;
const SEAL_BYTES: usize = 32;

/// Why no checkpoint is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CheckpointError {
    /// The state doesn't fit the cap: nothing is cut to make it.
    TooLarge { bytes: usize },
    /// A lent server's world is its host client's, not its own.
    Lent,
    /// The state breaks a limit `restore` enforces, so its restore would refuse:
    /// no image is written (Spec 01 §4.1.5).
    Unrestorable(String),
    Encode(String),
}

/// What a restore is told by the host shell. The secret is never in a
/// checkpoint (handoff §6, choice 6), so it comes from here.
#[derive(Clone)]
pub(crate) struct AuthorityConfig {
    pub seed: u64,
    pub worldgen_version: u32,
    pub creative: bool,
    pub difficulty: crate::survival::Difficulty,
    pub pop_secret: [u8; 32],
}

/// Why a restore was refused, by the contract code each maps onto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RestoreError {
    /// The binding or the key id doesn't match the config.
    InvalidConfig(String),
    /// Damaged bytes, or records that don't match the manifest.
    InvalidCheckpoint(String),
    /// A schema this build can't read, or a newer build's versions.
    UnsupportedVersion(String),
    /// Anything else: the load itself failed.
    Fault(String),
}

/// SHA-256 over the canonical bytes of what a restore must agree on.
pub(crate) fn config_binding(seed: u64, worldgen_version: u32, creative: bool, difficulty: crate::survival::Difficulty) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(seed.to_le_bytes());
    h.update(worldgen_version.to_le_bytes());
    h.update([u8::from(creative), difficulty as u8]);
    h.finalize().into()
}

fn pop_key_id(secret: &[u8; 32]) -> [u8; 16] {
    let mut h = Sha256::new();
    h.update(POP_KEY_ID_LABEL);
    h.update(secret);
    let digest: [u8; 32] = h.finalize().into();
    let mut id = [0; 16];
    id.copy_from_slice(&digest[..16]);
    id
}

/// Set the last [`SEAL_BYTES`] to the digest of everything before them.
pub(crate) fn seal(bytes: &mut [u8]) {
    let body = bytes.len() - SEAL_BYTES;
    let digest: [u8; 32] = Sha256::digest(&bytes[..body]).into();
    bytes[body..].copy_from_slice(&digest);
}

fn check_seal(bytes: &[u8]) -> Result<(), RestoreError> {
    if bytes.len() < HEADER_BYTES + SEAL_BYTES {
        return Err(RestoreError::InvalidCheckpoint("the bytes end early".into()));
    }
    let body = bytes.len() - SEAL_BYTES;
    let digest: [u8; 32] = Sha256::digest(&bytes[..body]).into();
    if digest != bytes[body..] {
        return Err(RestoreError::InvalidCheckpoint("the seal doesn't match the bytes".into()));
    }
    Ok(())
}

/// The gate `checkpoint` puts its own bytes through: the decoder and the range
/// checks a restore runs on them, so nothing they would refuse is written.
pub(crate) fn refuse_unrestorable(bytes: &[u8]) -> Result<(), CheckpointError> {
    let refused = |why: String| CheckpointError::Unrestorable(why);
    let tick = Header::read(bytes).map_err(|e| refused(format!("{e:?}")))?.tick;
    restore::decode_body(&bytes[HEADER_BYTES..bytes.len() - SEAL_BYTES], tick).map_err(|e| refused(e.0.to_string()))?;
    Ok(())
}

/// The fields a checkpoint opens with.
struct Header {
    protocol: u32,
    save_format: u32,
    worldgen_version: u32,
    seed: u64,
    binding: [u8; 32],
    key_id: [u8; 16],
    tick: u64,
}

/// The first thing a restore reads, whatever follows it: any schema but this
/// build's, older or newer, is one whose layout, seal and cap aren't ours to judge.
fn check_schema(bytes: &[u8]) -> Result<(), RestoreError> {
    let mut r = codec::Reader::new(bytes);
    if r.take(CHECKPOINT_MAGIC.len())? != CHECKPOINT_MAGIC {
        return Err(RestoreError::InvalidCheckpoint("the magic is wrong".into()));
    }
    match r.u16()? {
        SCHEMA => Ok(()),
        other => Err(RestoreError::UnsupportedVersion(format!("schema {other}, and this build reads {SCHEMA}"))),
    }
}

impl Header {
    /// The fields after the schema, which [`check_schema`] has read.
    fn read(bytes: &[u8]) -> Result<Self, RestoreError> {
        let mut r = codec::Reader::new(bytes);
        r.take(CHECKPOINT_MAGIC.len() + SCHEMA_BYTES)?;
        Ok(Self {
            protocol: r.u32()?,
            save_format: r.u32()?,
            worldgen_version: r.u32()?,
            seed: r.u64()?,
            binding: r.bytes32()?,
            key_id: r.bytes16()?,
            tick: r.u64()?,
        })
    }

    fn check_versions(&self) -> Result<(), RestoreError> {
        if self.protocol > crate::protocol::PROTOCOL_VERSION {
            return Err(RestoreError::UnsupportedVersion(format!("protocol {}", self.protocol)));
        }
        if self.save_format > crate::save_format::SAVE_FORMAT_VERSION {
            return Err(RestoreError::UnsupportedVersion(format!("save format {}", self.save_format)));
        }
        Ok(())
    }

    fn check_config(&self, config: &AuthorityConfig) -> Result<(), RestoreError> {
        let binding = config_binding(config.seed, config.worldgen_version, config.creative, config.difficulty);
        if binding != self.binding || config.seed != self.seed || config.worldgen_version != self.worldgen_version {
            return Err(RestoreError::InvalidConfig("this config isn't the one the checkpoint is bound to".into()));
        }
        if pop_key_id(&config.pop_secret) != self.key_id {
            return Err(RestoreError::InvalidConfig("this config holds another Proof-of-Play key".into()));
        }
        if u32::try_from(config.seed).is_err() {
            return Err(RestoreError::InvalidConfig("a seed past u32".into()));
        }
        Ok(())
    }
}

/// The manifest's encoded size (the size bench).
#[cfg(test)]
pub(crate) fn manifest_len(server: &GameServer) -> Result<usize, String> {
    let mut w = codec::Writer::new();
    manifest::write(&mut w, &manifest::entries(server)?);
    Ok(w.len())
}

/// The manifest as `checkpoint` would write it (the stored-bytes tests).
#[cfg(test)]
pub(crate) fn manifest_entries(server: &GameServer) -> Result<Vec<(Vec<u8>, [u8; 32])>, String> {
    manifest::entries(server)
}

/// Every part's encoded size, past the cap too (the size bench).
#[cfg(test)]
pub(crate) fn section_sizes(server: &GameServer) -> Result<Vec<(&'static str, usize)>, String> {
    volatile::section_sizes(server)
}

/// Where each section of this server's checkpoint starts (the resealed tests).
#[cfg(test)]
pub(crate) fn section_offsets(server: &GameServer) -> Result<volatile::SectionOffsets, String> {
    volatile::section_offsets(server, HEADER_BYTES)
}

impl GameServer {
    /// The config this authority is running under, as a shell holds it.
    pub(crate) fn authority_config(&self) -> AuthorityConfig {
        AuthorityConfig {
            seed: u64::from(self.seed),
            worldgen_version: self.world.generator.version(),
            creative: self.is_creative,
            difficulty: self.difficulty,
            pop_secret: self.pop_secret,
        }
    }

    /// This authority's state, in canonical bytes (see the module docs for the
    /// layout). Observational: no I/O, no mutation. Its manifest is true of the
    /// disk only straight after a save, so a runner persists, then checkpoints.
    /// `TooLarge` is the state not fitting here and now; `Unrestorable` is a state
    /// no restore would take. Neither ever leaves an image behind.
    pub(crate) fn checkpoint(&self) -> Result<Vec<u8>, CheckpointError> {
        let mut bytes = self.image()?;
        refuse_unrestorable(&bytes)?;
        seal(&mut bytes);
        Ok(bytes)
    }

    /// The bytes, its seal still to set. Past the gate only in `checkpoint`.
    fn image(&self) -> Result<Vec<u8>, CheckpointError> {
        if self.lent {
            return Err(CheckpointError::Lent);
        }
        let config = self.authority_config();
        let mut w = codec::Writer::new();
        w.raw(CHECKPOINT_MAGIC);
        w.u16(SCHEMA);
        w.u32(crate::protocol::PROTOCOL_VERSION);
        w.u32(crate::save_format::SAVE_FORMAT_VERSION);
        w.u32(config.worldgen_version);
        w.u64(config.seed);
        w.raw(&config_binding(config.seed, config.worldgen_version, config.creative, config.difficulty));
        w.raw(&pop_key_id(&self.pop_secret));
        w.u64(self.tick_counter);
        debug_assert_eq!(w.len(), HEADER_BYTES);
        volatile::write(&mut w, self).map_err(CheckpointError::Encode)?;
        if w.over_cap() {
            return Err(CheckpointError::TooLarge { bytes: w.len() });
        }
        manifest::write(&mut w, &manifest::entries(self).map_err(CheckpointError::Encode)?);
        w.raw(&[0; SEAL_BYTES]);
        w.finish().map_err(|bytes| CheckpointError::TooLarge { bytes })
    }

    /// What `checkpoint` won't hand over, for the tests that a restore also refuses
    /// it whoever wrote it.
    #[cfg(test)]
    pub(crate) fn checkpoint_ungated(&self) -> Result<Vec<u8>, CheckpointError> {
        let mut bytes = self.image()?;
        seal(&mut bytes);
        Ok(bytes)
    }

    /// Exactly `SHA256(b"fray/authority-state/v1\0" || checkpoint())`.
    pub(crate) fn state_hash(&self) -> Result<[u8; 32], CheckpointError> {
        let mut h = Sha256::new();
        h.update(STATE_HASH_LABEL);
        h.update(self.checkpoint()?);
        Ok(h.finalize().into())
    }
}

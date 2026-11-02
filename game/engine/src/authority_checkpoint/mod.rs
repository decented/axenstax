//! Keep slice 2b — the authority's canonical checkpoint and its restore (Spec 01
//! §4.1.5). A pure, bounded byte form of what a save doesn't hold, plus a digest
//! manifest of what it does (handoff §6, choice 1). An API with tests: nothing in
//! `--keep-world` calls it yet, and nothing here touches the pipe or the wire.
//!
//! ## Layout, in order
//!
//! ```text
//! magic         b"axenstax/authority-checkpoint\0"
//! schema        u16 (1)
//! protocol      u32   PROTOCOL_VERSION
//! save format   u32   SAVE_FORMAT_VERSION
//! worldgen      u32   the world's generator version
//! seed          u64   widened
//! binding       [32]  SHA-256 of seed, worldgen, creative and difficulty
//! pop key id    [16]  SHA256(b"axenstax/pop-key-id/v1\0" || pop_secret)[..16]
//! tick          u64   the completed tick
//! volatile            `volatile.rs`, then `players.rs`
//! manifest            `manifest.rs`: sorted (record key, SHA-256)
//! seal          [32]  SHA-256 of everything before it
//! ```
//!
//! Fixed-width little-endian integers, `u32` length prefixes, every set in sorted
//! order, every queue in its queue order (which is state), floats as bits with
//! `-0.0` folded onto `0.0` (`codec.rs`). The hard cap is 1 MiB: past it a
//! checkpoint is an error, never a shorter checkpoint.
//!
//! The seal isn't in the slice 2 spec's list. It makes "bad or tampered bytes"
//! true of the volatile sections, which nothing else on disk can check. It is a
//! checksum, not a signature: whoever can rewrite a checkpoint can reseal it.
//!
//! The secret itself is never in the bytes: only the key id, and the meta record
//! is hashed with `pop_secret` removed.
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
mod manifest;
mod players;
mod restore;
mod volatile;

use sha2::{Digest, Sha256};

use crate::server::GameServer;

pub(crate) use manifest::StoredRecords;

pub(crate) const CHECKPOINT_MAGIC: &[u8] = b"axenstax/authority-checkpoint\0";
pub(crate) const SCHEMA: u16 = 1;
/// The session-image cap of migration v0.1 (handoff §6, choice 1).
pub(crate) const MAX_CHECKPOINT_BYTES: usize = 1 << 20;

const POP_KEY_ID_LABEL: &[u8] = b"axenstax/pop-key-id/v1\0";
const STATE_HASH_LABEL: &[u8] = b"fray/authority-state/v1\0";
const SCHEMA_BYTES: usize = 2;
const HEADER_BYTES: usize = CHECKPOINT_MAGIC.len() + SCHEMA_BYTES + 4 + 4 + 4 + 8 + 32 + 16 + 8;
const SEAL_BYTES: usize = 32;

/// Why no checkpoint is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CheckpointError {
    /// A mob's death is waiting to be swept. Retry at the next tick boundary
    /// (choice 3). The contract has no "not now" code yet: handoff §6.
    DeathsPending,
    /// The state doesn't fit the cap: nothing is cut to make it.
    TooLarge { bytes: usize },
    /// A lent server's world is its host client's, not its own.
    Lent,
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
    /// Bad or tampered bytes, or records that don't match the manifest.
    InvalidCheckpoint(String),
    /// A newer schema or build than this one.
    UnsupportedVersion(String),
    /// Anything else: the load itself failed.
    Fault(String),
}

/// SHA-256 over the canonical bytes of what a restore must agree on.
fn config_binding(seed: u64, worldgen_version: u32, creative: bool, difficulty: crate::survival::Difficulty) -> [u8; 32] {
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

impl Header {
    /// The magic, then the schema, then the rest: a newer schema is named as
    /// such whatever follows it.
    fn read(bytes: &[u8]) -> Result<Self, RestoreError> {
        let mut r = codec::Reader::new(bytes);
        if r.take(CHECKPOINT_MAGIC.len())? != CHECKPOINT_MAGIC {
            return Err(RestoreError::InvalidCheckpoint("the magic is wrong".into()));
        }
        match r.u16()? {
            SCHEMA => {}
            0 => return Err(RestoreError::InvalidCheckpoint("schema 0 never existed".into())),
            newer => return Err(RestoreError::UnsupportedVersion(format!("schema {newer}, and this build reads {SCHEMA}"))),
        }
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

/// Every part's encoded size, past the cap too (the size bench).
#[cfg(test)]
pub(crate) fn section_sizes(server: &GameServer) -> Result<Vec<(&'static str, usize)>, String> {
    volatile::section_sizes(server)
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
    pub(crate) fn checkpoint(&self) -> Result<Vec<u8>, CheckpointError> {
        if self.lent {
            return Err(CheckpointError::Lent);
        }
        if crate::sim_death::has_pending(&self.ecs) {
            return Err(CheckpointError::DeathsPending);
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
        let mut bytes = w.finish().map_err(|bytes| CheckpointError::TooLarge { bytes })?;
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

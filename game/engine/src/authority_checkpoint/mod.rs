//! Keep slices 2b and 3 — the authority's canonical checkpoint and its restore
//! (Spec 01 §4.1.5). A pure, bounded byte form of what a save doesn't hold, plus the
//! digest of a tree of stored records naming what it does (handoff §6, choice 1). An
//! API with tests: nothing in `--keep-world` calls it yet, and nothing here touches the
//! pipe or the wire.
//!
//! ## Layout, in order
//!
//! ```text
//! magic         b"axenstax/authority-checkpoint\0"
//! schema        u16 (5)
//! protocol      u32   PROTOCOL_VERSION
//! save format   u32   SAVE_FORMAT_VERSION
//! worldgen      u32   the world's generator version
//! seed          u64   widened
//! binding       [32]  SHA-256 of seed, worldgen, creative and difficulty
//! pop key id    [16]  SHA256(b"axenstax/pop-key-id/v1\0" || pop_secret)[..16]
//! tick          u64   the completed tick
//! volatile            `volatile.rs`: the clock, the modes, the streamer and two phases
//! manifest            `records.rs`: the root of the tree, (key, SHA-256)
//! seal          [32]  SHA-256 of everything before it
//! ```
//!
//! Fixed-width little-endian integers, `u32` length prefixes, every set in sorted
//! order, every queue in its queue order (which is state), floats as bits with
//! `-0.0` folded onto `0.0` (`codec.rs`). The hard cap is 1 MiB: past it a
//! checkpoint is an error, never a shorter checkpoint. What grows with the world is
//! not in these bytes: the manifest, the layout, lava, water, the exposure clocks, the cell
//! sims, the player slots and the corpses are stored records, and `world.dat` is bound in
//! fixed pieces, each by its digest (`records.rs`).
//!
//! The seal isn't in the slice 2 spec's list. It lets a restore see damage to the
//! volatile sections, which nothing else on disk can check. It is a checksum, not
//! a signature, and nothing here tells one kind of damage from another: whatever
//! doesn't match is `InvalidCheckpoint`.
//!
//! The secret itself is never in the bytes: only the key id, and the meta record
//! is hashed with the value of `pop_secret` cut from it.
//!
//! ## An image its own bytes or state records would be refused for is never written
//!
//! `checkpoint` puts its own bytes and the state records it binds through the decoder
//! and range checks `restore` uses (`decode_body`, `take_state`), so each limit is
//! enforced at both ends by one piece of code, and a limit added later is enforced at
//! both by being added once. That is all it checks: a root is restorable only straight
//! after a persist. With no persist yet, at a later tick than the persist, over a
//! `world.dat` this build only part-decodes, or after the state changed since the
//! persist, `checkpoint` still returns an image and the restore refuses it. A runner
//! checks the restore at each commit.
//!
//! ## Observational
//!
//! `checkpoint` takes `&self`, does no I/O and calls no save path (nothing touches
//! `World::disk_chunks`). What it needs of the disk, `StoredRecords` learnt when
//! the records were read or written. The tree and the state records are functions of
//! what it holds, so it computes them from memory, as `persist_authority` wrote them
//! if nothing has changed since; if something has, the digests it names aren't the
//! store's, and a restore refuses. It reads no clock and no per-run value:
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
mod record_set;
mod lava_records;
mod records;
mod restore;
mod volatile;

use sha2::{Digest, Sha256};

use crate::server::GameServer;

pub(crate) use manifest::StoredRecords;
pub(crate) use record_set::RecordSet;
#[cfg(test)]
pub(crate) use records::file_piece_batch;
pub(crate) use records::RECORDS_DIR;
#[cfg(test)]
pub(crate) use records::{FILE_PIECE_BYTES, MAX_RECORD_BYTES, Tree};

pub(crate) const CHECKPOINT_MAGIC: &[u8] = b"axenstax/authority-checkpoint\0";
/// 5: `world.dat` bound in fixed pieces, and all that grows with the world in stored
/// records. Nothing relies on an earlier one.
pub(crate) const SCHEMA: u16 = 6;
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

/// The gate `checkpoint` puts its own bytes and state records through: the decoders
/// and the range checks a restore runs on them, so nothing they would refuse is written.
fn refuse_unrestorable(bytes: &[u8], state: &records::StateRecords) -> Result<(), CheckpointError> {
    let refused = |why: String| CheckpointError::Unrestorable(why);
    let tick = Header::read(bytes).map_err(|e| refused(format!("{e:?}")))?.tick;
    let (mut volatile, _) = restore::decode_body(&bytes[HEADER_BYTES..bytes.len() - SEAL_BYTES], tick).map_err(|e| refused(e.0.to_string()))?;
    volatile.take_state(state).map_err(|e| refused(e.0.to_string()))?;
    Ok(())
}

/// What a checkpoint binds, all from memory: each is exactly what a persist writes.
struct Bound {
    state: records::StateRecords,
    /// The state records as stored.
    pieces: Vec<records::Stored>,
    /// Every record, the one sorted list.
    flat: Vec<manifest::Entry>,
    tree: records::Tree,
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

/// How many sections a layout's bytes name, through the reader `checkpoint` and
/// `restore` share.
#[cfg(test)]
pub(crate) fn layout_sections_read(bytes: &[u8]) -> Result<u32, String> {
    volatile::layout_sections_read(bytes)
}

/// What the manifest's tree comes to (the size bench): its records, their bytes, the
/// largest, and what the same list would be inline.
#[cfg(test)]
pub(crate) fn manifest_sizes(server: &GameServer) -> Result<(usize, usize, usize, usize), String> {
    let bound = server.bound_records()?;
    let mut inline = codec::Writer::new();
    manifest::write(&mut inline, &bound.flat);
    let sizes = bound.tree.nodes.iter().map(|(_, bytes)| bytes.len());
    Ok((bound.tree.nodes.len(), sizes.clone().sum(), sizes.max().unwrap_or(0), inline.len()))
}

/// Every record this server's checkpoint binds, the tree walked (the stored-bytes tests).
#[cfg(test)]
pub(crate) fn manifest_entries(server: &GameServer) -> Result<Vec<(Vec<u8>, [u8; 32])>, String> {
    Ok(server.bound_records()?.flat)
}

/// Every family of state record, by its key letter and name: what the tests that go
/// through each family are driven from.
#[cfg(test)]
pub(crate) fn state_families() -> Vec<(u8, &'static str)> {
    records::FAMILIES.to_vec()
}

/// A record's digest as a manifest binds it.
#[cfg(test)]
pub(crate) fn record_digest(bytes: &[u8]) -> [u8; 32] {
    records::sha256(bytes)
}

/// The key of every manifest record and state piece it binds, and each one's bytes.
#[cfg(test)]
pub(crate) fn bound_record_files(server: &GameServer) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
    let bound = server.bound_records()?;
    Ok(bound.pieces.into_iter().chain(bound.tree.nodes).collect())
}

/// The file a record lies in.
#[cfg(test)]
pub(crate) fn record_file(world_dir: &std::path::Path, key: &[u8]) -> std::path::PathBuf {
    records::record_path(world_dir, key)
}

/// The world past what a test can build: real sorted chunk keys and a real layout's
/// bytes through the real pieces, tree, commit and walk, the store a folder holding
/// nothing else, and the same read and range check a restore does. Returns the sections
/// the layout names, and what the tree gave back.
#[cfg(test)]
pub(crate) fn commit_and_read_bound(
    world_dir: &std::path::Path,
    layout: Vec<u8>,
    chunks: Vec<(Vec<u8>, [u8; 32])>,
) -> Result<(u32, Vec<(Vec<u8>, [u8; 32])>, records::Tree), RestoreError> {
    let counts = |n: usize| {
        let mut w = codec::Writer::uncapped();
        for _ in 0..n {
            w.count(0);
        }
        w.into_bytes()
    };
    // What each record's reader takes for nothing there.
    let state = records::StateRecords {
        layout,
        lava_spread: Vec::new(),
        lava_retract: Vec::new(),
        lava_sources: Vec::new(),
        exposure: counts(1),
        water: counts(4),
        cells: counts(8),
        players: counts(1),
        corpses: counts(1),
    };
    let pieces = state.pieces();
    let mut flat = chunks;
    flat.extend(pieces.iter().map(|(key, bytes)| (key.clone(), records::sha256(bytes))));
    flat.sort_unstable();
    let tree = records::build_tree(&flat).map_err(RestoreError::Fault)?;
    records::commit(world_dir, &pieces, &tree.nodes).map_err(RestoreError::Fault)?;
    let walked = records::walk(std::slice::from_ref(&tree.top), world_dir)?;
    let (read, _) = records::read_state(&walked, world_dir)?;
    let sections = volatile::layout_sections_read(&read.layout).map_err(RestoreError::InvalidCheckpoint)?;
    Ok((sections, walked, tree))
}

/// `tree`'s size in the terms of the size tests: how many records and the longest.
#[cfg(test)]
pub(crate) fn tree_shape(tree: &records::Tree) -> (usize, usize) {
    (tree.nodes.len(), tree.nodes.iter().map(|(_, bytes)| bytes.len()).max().unwrap_or(0))
}

/// This server's layout record, whole.
#[cfg(test)]
pub(crate) fn layout_record(server: &GameServer) -> Vec<u8> {
    server.bound_records().expect("the state encodes").state.layout
}

/// This server's record of a state family, whole.
#[cfg(test)]
pub(crate) fn state_record(server: &GameServer, family: u8) -> Vec<u8> {
    server.bound_records().expect("the state encodes").state.family(family).clone()
}

/// `pieces` and a tree of `flat` written, and the checkpoint's digest of the tree replaced
/// and resealed.
#[cfg(test)]
fn rebound(server: &GameServer, bytes: &[u8], old_top: [u8; 32], pieces: &[records::Stored], flat: &[manifest::Entry]) -> Vec<u8> {
    let tree = records::build_tree(flat).expect("a tree");
    let dir = crate::save::world_dir(&server.world_name);
    records::commit(&dir, pieces, &tree.nodes).expect("the records write");
    // The manifest section is the last thing before the seal, and its root is `t`.
    let mut out = bytes.to_vec();
    let digest_at = out.len() - SEAL_BYTES - 32;
    assert_eq!(old_top, out[digest_at..digest_at + 32], "the section is where it was");
    out[digest_at..digest_at + 32].copy_from_slice(&tree.top.1);
    seal(&mut out);
    out
}

/// A checkpoint with one state record's bytes replaced, its pieces and tree written and
/// the digest in the image resealed: for the tests of what a restore does with bytes
/// the digests vouch for.
#[cfg(test)]
pub(crate) fn rebind_state(server: &GameServer, bytes: &[u8], family: u8, record: Vec<u8>) -> Vec<u8> {
    let mut bound = server.bound_records().expect("the state encodes");
    *bound.state.family(family) = record;
    let pieces = bound.state.pieces();
    let mut flat: Vec<_> = bound.flat.into_iter().filter(|(key, _)| !records::is_state_key(key)).collect();
    flat.extend(pieces.iter().map(|(key, piece)| (key.clone(), records::sha256(piece))));
    flat.sort_unstable();
    rebound(server, bytes, bound.tree.top.1, &pieces, &flat)
}

/// The same for the layout.
#[cfg(test)]
pub(crate) fn rebind_layout(server: &GameServer, bytes: &[u8], layout: Vec<u8>) -> Vec<u8> {
    rebind_state(server, bytes, records::LAYOUT, layout)
}

/// A checkpoint whose tree binds `edit`'s list instead, every piece left in the store:
/// for what a restore does with a list that isn't one a persist would write.
#[cfg(test)]
pub(crate) fn rebind_entries(server: &GameServer, bytes: &[u8], edit: impl FnOnce(&mut Vec<manifest::Entry>)) -> Vec<u8> {
    let mut bound = server.bound_records().expect("the state encodes");
    edit(&mut bound.flat);
    rebound(server, bytes, bound.tree.top.1, &bound.pieces, &bound.flat)
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

    /// The authority's persist: a save, then the records only an authority has, which
    /// bind it. A world that is never run as one gets no new file, as nothing else calls
    /// this: `try_save` is a normal server's save, and writes what it always did. Not
    /// called by `--keep-world` yet; a runner's `last` step will. `Err`: this persist's
    /// records aren't all there, so no checkpoint can be restored from it.
    pub(crate) fn persist_authority(&self) -> Result<(), String> {
        self.persist_authority_set().map(|_| ())
    }

    /// [`Self::persist_authority`], returning the record set it bound: the save's own
    /// `world.dat` and sidecars cut into records by [`Self::build_record_set`]'s code, the
    /// state records and the tree written to `authority/`, and the bound keys kept for the
    /// next set's `removed`.
    pub(crate) fn persist_authority_set(&self) -> Result<RecordSet, String> {
        let (world_dat, rows) = self.try_save_returning(self.selected_local_personal.as_ref())?;
        let set = self.record_set_of(&world_dat, rows, &self.stored_records.bound_keys())?;
        records::commit(&crate::save::world_dir(&self.world_name), set.state_pieces(), &set.tree.nodes)?;
        self.stored_records.set_bound_keys(set.keys());
        Ok(set)
    }

    /// What a checkpoint now binds, written: for the tests that change a record after a persist.
    pub(crate) fn write_bound_records(&self) -> Result<(), String> {
        let bound = self.bound_records()?;
        records::commit(&crate::save::world_dir(&self.world_name), &bound.pieces, &bound.tree.nodes)?;
        let keys = bound.flat.iter().map(|(key, _)| key.clone()).chain(bound.tree.nodes.iter().map(|(key, _)| key.clone())).collect();
        self.stored_records.set_bound_keys(keys);
        Ok(())
    }

    fn bound_records(&self) -> Result<Bound, String> {
        let state = volatile::state_records(self)?;
        let pieces = state.pieces();
        let mut flat = manifest::entries(self)?;
        flat.extend(pieces.iter().map(|(key, bytes)| (key.clone(), records::sha256(bytes))));
        flat.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let tree = records::build_tree(&flat)?;
        Ok(Bound { state, pieces, flat, tree })
    }

    /// This authority's state, in canonical bytes (see the module docs for the
    /// layout). Observational: no I/O, no mutation. Its manifest is true of the
    /// disk only straight after a save, so a runner persists, then checkpoints.
    /// `TooLarge` is the state not fitting here and now; `Unrestorable` is a state
    /// no restore would take. Neither ever leaves an image behind.
    pub(crate) fn checkpoint(&self) -> Result<Vec<u8>, CheckpointError> {
        let bound = self.bound_records_for_checkpoint()?;
        let mut bytes = self.image(std::slice::from_ref(&bound.tree.top))?;
        refuse_unrestorable(&bytes, &bound.state)?;
        seal(&mut bytes);
        Ok(bytes)
    }

    fn bound_records_for_checkpoint(&self) -> Result<Bound, CheckpointError> {
        if self.lent {
            return Err(CheckpointError::Lent);
        }
        if let Some(why) = self.stored_records.why_unbindable() {
            return Err(CheckpointError::Unrestorable(why));
        }
        self.bound_records().map_err(CheckpointError::Encode)
    }

    /// A manifest listed in the image: for a fixture. The shipped module only ever
    /// writes the tree, and a restore outside the tests refuses this as not canonical.
    #[cfg(test)]
    pub(crate) fn checkpoint_inline(&self) -> Result<Vec<u8>, CheckpointError> {
        let bound = self.bound_records_for_checkpoint()?;
        let mut bytes = self.image(&bound.flat)?;
        refuse_unrestorable(&bytes, &bound.state)?;
        seal(&mut bytes);
        Ok(bytes)
    }

    /// The bytes, its seal still to set. Past the gate only in `checkpoint`.
    fn image(&self, manifest_section: &[manifest::Entry]) -> Result<Vec<u8>, CheckpointError> {
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
        manifest::write(&mut w, manifest_section);
        w.raw(&[0; SEAL_BYTES]);
        w.finish().map_err(|bytes| CheckpointError::TooLarge { bytes })
    }

    /// What `checkpoint` won't hand over, for the tests that a restore also refuses
    /// it whoever wrote it.
    #[cfg(test)]
    pub(crate) fn checkpoint_ungated(&self) -> Result<Vec<u8>, CheckpointError> {
        let bound = self.bound_records_for_checkpoint()?;
        let mut bytes = self.image(std::slice::from_ref(&bound.tree.top))?;
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

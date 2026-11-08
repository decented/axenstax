//! The record set one persist makes, built from memory (Tier-L ruling 1, slice 3): every
//! record a checkpoint binds that this server holds the bytes of, the digest-only entries
//! for the rest, the manifest tree over all of them, and the keys the previous set bound
//! that this one doesn't. No file is read or written here; `persist_authority` writes
//! what it needs of the result.
//!
//! | Key | Value in `records` | Where the bytes come from |
//! |---|---|---|
//! | `w` + piece | a piece of `world.dat` | [`GameServer::encode_world_dat`], cut by `records::file_piece_batch` |
//! | `c` + `(cx, cy, cz)` | a non-empty chunk | `Chunk::as_bytes`, what a save writes |
//! | `j` + file name | a present joiner's sidecar | its capture, encoded (`joiner_rows_in_memory`) |
//! | `l p f s u k` + piece | the state records | `volatile::state_records` |
//! | `q r` + piece | lava's spread and retract queues, at stable positions | `lava_records` |
//! | `x` + section | lava's source differences, by 16-cube section | `lava_records` |
//! | `t` + prefix | a manifest record | the tree over every entry below |
//!
//! `held` lists the entries this server has only a digest of: the sidecars of joiners not
//! present, the meta (`m`) and a handed-over world's records (`h`). Their bytes are the
//! meta writer's and the joiners' own saves', not this save's, so they stay digest-only
//! and a runner that needs them reads the files, as a save leaves them. They are bound
//! all the same: the tree has to equal the one a checkpoint computes.
//!
//! The type is this module's own, not Fray's `PersistBatch`: a set has digest-only
//! entries and a tree root, and no `last` flag or size split, which belong to the
//! batcher. Its `records` are Fray's `StoredRecord`s, ready for one.

use std::collections::BTreeSet;

use fray_session::authority::StoredRecord;

use super::manifest::{self, Entry};
use super::records::{self, sha256, Stored, Tree};
use super::volatile;
use crate::server::GameServer;

/// What a persist binds, and what it takes out of the store.
pub(crate) struct RecordSet {
    /// Every bound record this server holds the bytes of, sorted by key.
    pub records: Vec<StoredRecord>,
    /// Bound, digest only (see the module docs).
    pub held: Vec<Entry>,
    /// The one sorted list the tree binds: every entry of `records` but the manifest
    /// records, and `held`.
    pub flat: Vec<Entry>,
    pub tree: Tree,
    /// The keys the previous bound set had and this one hasn't, sorted.
    pub removed: Vec<Vec<u8>>,
    state_pieces: Vec<Stored>,
}

impl RecordSet {
    /// Every key this set binds: the entries and the manifest records over them.
    pub(crate) fn keys(&self) -> BTreeSet<Vec<u8>> {
        self.flat.iter().map(|(key, _)| key.clone()).chain(self.tree.nodes.iter().map(|(key, _)| key.clone())).collect()
    }

    /// The state records as stored, which a persist writes to `authority/`.
    pub(super) fn state_pieces(&self) -> &[Stored] {
        &self.state_pieces
    }
}

impl GameServer {
    /// The record set of this server as it stands, from memory only: it encodes the
    /// `world.dat` a save would write, so a caller that has just saved passes those bytes
    /// to [`Self::record_set_of`] instead. `previous` is the key set the last persist
    /// bound. Errs as a save does for what it can't encode, and as a checkpoint does for
    /// records it can't bind.
    pub(crate) fn build_record_set(
        &self,
        local_personal: Option<&crate::personal_value::LocalPersonalSave>,
        previous: &BTreeSet<Vec<u8>>,
    ) -> Result<RecordSet, String> {
        let saved_mobs = crate::save::tamed_mobs_to_saved(&self.ecs)?;
        let world_dat = self.encode_world_dat(local_personal, saved_mobs)?;
        let rows = self.joiner_rows_in_memory()?;
        self.record_set_of(&world_dat, rows, previous)
    }

    /// The set for `world_dat` and the present joiners' `rows`, both as a save made them.
    pub(super) fn record_set_of(
        &self,
        world_dat: &[u8],
        rows: Vec<crate::joiner_stage::Row>,
        previous: &BTreeSet<Vec<u8>>,
    ) -> Result<RecordSet, String> {
        let sidecar_key = |name: &str| {
            let mut key = vec![b'j'];
            key.extend_from_slice(name.as_bytes());
            key
        };
        let present: BTreeSet<Vec<u8>> = rows.iter().map(|row| sidecar_key(&row.name)).collect();
        // What the notes hold of the disk, less what this set holds the bytes of.
        let held: Vec<Entry> = manifest::held_entries(self)?
            .into_iter()
            .filter(|(key, _)| !records::is_world_dat_piece(key) && !present.contains(key))
            .collect();
        let mut flat = held.clone();
        let mut stored: Vec<StoredRecord> = Vec::new();
        let mut hold = |key: Vec<u8>, value: Vec<u8>, flat: &mut Vec<Entry>| {
            flat.push((key.clone(), sha256(&value)));
            stored.push(StoredRecord { key, value });
        };

        let old_pieces = previous.iter().filter(|key| records::is_world_dat_piece(key)).count() as u32;
        for (key, piece) in records::file_piece_batch(Some(world_dat), old_pieces).writes {
            hold(key, piece.to_vec(), &mut flat);
        }
        for row in rows {
            hold(sidecar_key(&row.name), row.bytes, &mut flat);
        }
        for ((cx, cy, cz), chunk) in self.world.persistable_chunks() {
            if !chunk.is_empty() {
                hold(manifest::chunk_key(cx, cy, cz), chunk.as_bytes(), &mut flat);
            }
        }
        let state_pieces = volatile::state_records(self)?.pieces();
        for (key, piece) in &state_pieces {
            hold(key.clone(), piece.clone(), &mut flat);
        }
        flat.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let tree = records::build_tree(&flat)?;
        stored.extend(tree.nodes.iter().map(|(key, value)| StoredRecord { key: key.clone(), value: value.clone() }));
        stored.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        let mut set = RecordSet { records: stored, held, flat, tree, removed: Vec::new(), state_pieces };
        set.removed = previous.difference(&set.keys()).cloned().collect();
        Ok(set)
    }
}

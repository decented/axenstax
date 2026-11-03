//! The records only the authority's own persist writes (`GameServer::persist_authority`):
//! the manifest tree, and what follows the world's size and is in no save. A world
//! never run as an authority has none of them. Each is a file named by its key in
//! `authority/`, standing in for a runner's store.
//!
//! | Key | Record |
//! |---|---|
//! | `t` + prefix | a manifest record: `(key, digest)` in sorted order, of records with that prefix |
//! | `l` + `u32` BE | a piece of the layout |
//! | `q` + `u32` BE | a piece of the lava queue |
//! | `p` + `u32` BE | a piece of the exposure clocks |
//! | `f` + `u32` BE | a piece of water: sources as the difference from a rescan, its queues |
//! | `s` + `u32` BE | a piece of the cell sims: leaf decay, the scheduler, power, rubber taps |
//! | `u` + `u32` BE | a piece of the player slots |
//! | `k` + `u32` BE | a piece of the corpses waiting for a death pass |
//!
//! One more family is bound the same way and has no file here, as the file it is pieces
//! of stays whole where a save writes it: `w` + `u32` BE, a piece of `world.dat`.
//! `manifest.rs` reads it.
//!
//! **The tree.** A checkpoint's manifest section lists `(key, digest)`, and a key
//! starting `t` binds a further manifest record the same way (ruling 2). The tree is a
//! function of the entries alone: at most [`LEAF_ENTRIES`] entries is one record; more
//! are shared out by the byte after their common prefix, each share a record of its
//! own, recursively. So walking it gives the one sorted list, a checkpoint computes its
//! root from memory with no I/O, and changing one entry rewrites one path.
//!
//! **Pieces.** A state record is cut at [`MAX_RECORD_BYTES`], so that no value passes
//! the runner's record limit whatever the world holds. A queue's order is its state, so
//! a piece is by position: an entry added near the start rewrites the pieces after it.
//!
//! **`world.dat`'s pieces.** One key and digest for each [`FILE_PIECE_BYTES`] of the stored
//! bytes, so the file can pass the runner's record-value limit (the Tier-L rulings' addendum
//! to section 2). Nothing is parsed to make them, so a build with another field doesn't
//! see an unchanged file as changed. One changed row rewrites its piece, and every later
//! piece if it changes the row's length. Per-table keys can replace that if a measured
//! commit shows it matters.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::codec::{Malformed, Reader, Writer};
use super::manifest::{self, Entry};
use super::RestoreError;

/// Where these records lie in a world's folder.
pub(crate) const RECORDS_DIR: &str = "authority";
/// The runner's key limit (Tier-L rulings, the limits table).
pub(super) const MAX_RECORD_KEY: usize = 128;
/// No record here is longer: the game's number, a sixteenth of the runner's record-value
/// limit (1 MiB less headers). It isn't derived from that limit or the batch limit behind
/// it, which is an unmeasured placeholder. A manifest record can't reach it: 257 entries
/// of at most `4 + 128 + 32` bytes.
pub(crate) const MAX_RECORD_BYTES: usize = 64 * 1024;
const LEAF_ENTRIES: usize = 64;

/// The size of a piece of `world.dat`, part of this schema: changing it is a new schema.
/// Not derived from the batch limit, an unmeasured placeholder. Well under the runner's
/// record-value limit, so that limit can come down without a change here. Why this number:
/// a piece costs the file one manifest entry of 41 B, so even a 1 GiB file (the runner's
/// total) is 16,384 keys of its 262,144; and it is what one changed row of a fixed length
/// rewrites, where a piece at the limit would make that 1 MiB. A smaller one saves nothing
/// once a row's length changes, as every later piece is then rewritten whatever their
/// size. It is also the one limit a read of any record in this schema already has.
pub(crate) const FILE_PIECE_BYTES: usize = 64 * 1024;
const _: () = assert!(FILE_PIECE_BYTES <= MAX_RECORD_BYTES);

pub(super) const MANIFEST: u8 = b't';
pub(super) const LAYOUT: u8 = b'l';
pub(super) const LAVA: u8 = b'q';
pub(super) const EXPOSURE: u8 = b'p';
pub(super) const WATER: u8 = b'f';
pub(super) const CELLS: u8 = b's';
pub(super) const PLAYERS: u8 = b'u';
pub(super) const CORPSES: u8 = b'k';
pub(super) const WORLD_DAT: u8 = b'w';
const FAMILIES: [(u8, &str); 7] = [
    (LAYOUT, "layout"),
    (LAVA, "lava queue"),
    (EXPOSURE, "exposure clocks"),
    (WATER, "water"),
    (CELLS, "cell sims"),
    (PLAYERS, "player slots"),
    (CORPSES, "corpses"),
];

pub(crate) type Stored = (Vec<u8>, Vec<u8>);

pub(super) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The file holding a record.
pub(crate) fn record_path(world_dir: &Path, key: &[u8]) -> PathBuf {
    world_dir.join(RECORDS_DIR).join(hex::encode(key))
}

/// A record cut to fit: is this key one of those a state's pieces use?
pub(super) fn is_state_key(key: &[u8]) -> bool {
    key.len() == 5 && FAMILIES.iter().any(|(family, _)| *family == key[0])
}

/// Is this key a piece of `world.dat`?
pub(super) fn is_world_dat_piece(key: &[u8]) -> bool {
    key.len() == 5 && key[0] == WORLD_DAT
}

/// What a family's key says, for a refusal.
pub(super) fn describe(key: &[u8]) -> Option<String> {
    FAMILIES.iter().chain(&[(WORLD_DAT, "world.dat")]).find(|(family, _)| key.len() == 5 && *family == key[0]).map(|(_, name)| {
        format!("{name} piece {}", u32::from_be_bytes([key[1], key[2], key[3], key[4]]))
    })
}

fn piece_key(family: u8, i: u32) -> Vec<u8> {
    let mut key = vec![family];
    key.extend_from_slice(&i.to_be_bytes());
    key
}

/// The bytes of the state records, whole.
#[derive(Default)]
pub(super) struct StateRecords {
    pub layout: Vec<u8>,
    pub lava: Vec<u8>,
    pub exposure: Vec<u8>,
    pub water: Vec<u8>,
    pub cells: Vec<u8>,
    pub players: Vec<u8>,
    pub corpses: Vec<u8>,
}

impl StateRecords {
    /// The record of a family in [`FAMILIES`].
    pub(super) fn family(&mut self, family: u8) -> &mut Vec<u8> {
        match family {
            LAYOUT => &mut self.layout,
            LAVA => &mut self.lava,
            EXPOSURE => &mut self.exposure,
            WATER => &mut self.water,
            CELLS => &mut self.cells,
            PLAYERS => &mut self.players,
            CORPSES => &mut self.corpses,
            other => unreachable!("no state record is family {other}"),
        }
    }

    /// As they are stored: at least one piece each, in key order.
    pub(super) fn pieces(&self) -> Vec<Stored> {
        let mut out = Vec::new();
        let parts = [
            (LAYOUT, &self.layout),
            (LAVA, &self.lava),
            (EXPOSURE, &self.exposure),
            (WATER, &self.water),
            (CELLS, &self.cells),
            (PLAYERS, &self.players),
            (CORPSES, &self.corpses),
        ];
        for (family, bytes) in parts {
            for (i, piece) in bytes.chunks(MAX_RECORD_BYTES).enumerate() {
                let mut key = vec![family];
                key.extend_from_slice(&(i as u32).to_be_bytes());
                out.push((key, piece.to_vec()));
            }
        }
        out
    }
}

/// The manifest tree of some sorted entries.
pub(crate) struct Tree {
    /// What the checkpoint names.
    pub top: (Vec<u8>, [u8; 32]),
    /// Every manifest record, by key.
    pub nodes: Vec<Stored>,
}

pub(super) fn build_tree(entries: &[Entry]) -> Result<Tree, String> {
    if let Some((key, _)) = entries.iter().find(|(key, _)| key.len() >= MAX_RECORD_KEY) {
        return Err(format!("a record key of {} bytes", key.len()));
    }
    let mut nodes = Vec::new();
    let top = build_node(entries, &mut nodes);
    nodes.sort_unstable();
    Ok(Tree { top, nodes })
}

fn common_prefix(entries: &[Entry]) -> &[u8] {
    let (Some((first, _)), Some((last, _))) = (entries.first(), entries.last()) else { return &[] };
    // Sorted, so the first and last differ wherever any of them do.
    let shared = first.iter().zip(last).take_while(|(a, b)| a == b).count();
    &first[..shared]
}

fn build_node(entries: &[Entry], nodes: &mut Vec<Stored>) -> Entry {
    let prefix = common_prefix(entries);
    let mut list: Vec<Entry> = Vec::new();
    if entries.len() <= LEAF_ENTRIES {
        list.extend_from_slice(entries);
    } else {
        let mut rest = entries;
        // A key that is the prefix itself sorts first and belongs to no share.
        if rest[0].0.len() == prefix.len() {
            list.push(rest[0].clone());
            rest = &rest[1..];
        }
        while let Some((first, _)) = rest.first() {
            let byte = first[prefix.len()];
            let end = rest.partition_point(|(key, _)| key[prefix.len()] <= byte);
            list.push(build_node(&rest[..end], nodes));
            rest = &rest[end..];
        }
    }
    let mut w = Writer::uncapped();
    manifest::write(&mut w, &list);
    let bytes = w.into_bytes();
    let mut key = vec![MANIFEST];
    key.extend_from_slice(prefix);
    let digest = sha256(&bytes);
    nodes.push((key.clone(), bytes));
    (key, digest)
}

/// `world.dat`'s stored bytes cut into its pieces. An empty file is one empty piece, so that
/// it is bound and isn't the same as no file; a file of a whole number of pieces has no
/// empty one after them.
pub(crate) fn file_pieces(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return vec![bytes];
    }
    bytes.chunks(FILE_PIECE_BYTES).collect()
}

/// The key and digest of each piece of `world.dat`.
pub(super) fn file_entries(bytes: &[u8]) -> Vec<Entry> {
    file_pieces(bytes).into_iter().enumerate().map(|(i, piece)| (piece_key(WORLD_DAT, i as u32), sha256(piece))).collect()
}

/// What a batch must do to leave the store equal to the record set a checkpoint binds.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PieceWrites<'a> {
    pub writes: Vec<(Vec<u8>, &'a [u8])>,
    pub removes: Vec<Vec<u8>>,
}

/// The pieces of `world.dat`'s new bytes (`None`: the file has gone), and the keys to
/// remove given how many pieces the store held, so a file that shrank leaves none past its
/// new end. Pure, and not called yet: the file stays whole on disk here, and `PersistBatch`
/// (ruling 1) will call this to make its batch of a persist.
pub(crate) fn file_piece_batch(new_bytes: Option<&[u8]>, old_count: u32) -> PieceWrites<'_> {
    let pieces = new_bytes.map(file_pieces).unwrap_or_default();
    let new_count = pieces.len() as u32;
    PieceWrites {
        writes: pieces.into_iter().enumerate().map(|(i, piece)| (piece_key(WORLD_DAT, i as u32), piece)).collect(),
        removes: (new_count..old_count.max(new_count)).map(|i| piece_key(WORLD_DAT, i)).collect(),
    }
}

/// What the store holds under `key`. `None`: nothing. A record past the limit comes back
/// cut, and so doesn't match its digest.
fn read_record(world_dir: &Path, key: &[u8]) -> Result<Option<Vec<u8>>, RestoreError> {
    let path = record_path(world_dir, key);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RestoreError::Fault(format!("{} can't be read ({e})", path.display()))),
    };
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| RestoreError::Fault(format!("{} can't be read ({e})", path.display())))?;
    Ok(Some(bytes))
}

/// The one sorted list a manifest section binds, each manifest record checked against
/// its digest before it is parsed. Keys starting `t` are read; any other is an entry.
pub(super) fn walk(top: &[Entry], world_dir: &Path) -> Result<Vec<Entry>, RestoreError> {
    let mut flat = Vec::new();
    walk_list(top, None, world_dir, &mut flat)?;
    if !flat.windows(2).all(|pair| pair[0].0 < pair[1].0) {
        return Err(Malformed("a manifest tree that isn't in its one sorted order").into());
    }
    Ok(flat)
}

fn walk_list(list: &[Entry], within: Option<&[u8]>, world_dir: &Path, flat: &mut Vec<Entry>) -> Result<(), RestoreError> {
    let inside = within.unwrap_or_default();
    for (key, digest) in list {
        if key.len() > MAX_RECORD_KEY {
            return Err(Malformed("a record key past the limit").into());
        }
        if key.first() != Some(&MANIFEST) {
            if !key.starts_with(inside) {
                return Err(Malformed("a manifest record naming a key outside its prefix").into());
            }
            flat.push((key.clone(), *digest));
            continue;
        }
        let prefix = &key[1..];
        // A longer prefix each time, so the walk ends whatever the records say.
        if !prefix.starts_with(inside) || (within.is_some() && prefix.len() <= inside.len()) {
            return Err(Malformed("a manifest record that doesn't narrow its parent's prefix").into());
        }
        let named = hex::encode(key);
        let Some(stored) = read_record(world_dir, key)? else {
            return Err(RestoreError::InvalidCheckpoint(format!("the manifest record {named} is missing")));
        };
        if sha256(&stored) != *digest {
            return Err(RestoreError::InvalidCheckpoint(format!("the manifest record {named} doesn't match")));
        }
        let mut r = Reader::new(&stored);
        let inner = manifest::read(&mut r)?;
        if !r.is_empty() {
            return Err(Malformed("bytes left over after a manifest record").into());
        }
        walk_list(&inner, Some(prefix), world_dir, flat)?;
    }
    Ok(())
}

/// Each state record the sorted `wanted` names, whole, and the digest of each piece as it
/// lies on disk for the caller to compare. A piece that isn't there is left for the
/// comparison to name.
pub(super) fn read_state(wanted: &[Entry], world_dir: &Path) -> Result<(StateRecords, Vec<Entry>), RestoreError> {
    let mut found = Vec::new();
    let mut state = StateRecords::default();
    for (family, name) in FAMILIES {
        let mut bytes = Vec::new();
        let mut next = 0u32;
        for (key, _) in wanted.iter().filter(|(key, _)| is_state_key(key) && key[0] == family) {
            if key[1..] != next.to_be_bytes() {
                return Err(RestoreError::InvalidCheckpoint(format!("the {name} isn't bound piece by piece from its first")));
            }
            next += 1;
            if let Some(piece) = read_record(world_dir, key)? {
                found.push((key.clone(), sha256(&piece)));
                bytes.extend_from_slice(&piece);
            }
        }
        if next == 0 {
            return Err(RestoreError::InvalidCheckpoint(format!("no {name} is bound")));
        }
        *state.family(family) = bytes;
    }
    Ok((state, found))
}

/// Write `records` and the manifest `nodes` that bind them: anything that changed, the
/// tree's root last, then whatever these records no longer include is removed. Nothing
/// reads such a leftover, as a restore only reads what the tree names.
pub(super) fn commit(world_dir: &Path, records: &[Stored], nodes: &[Stored]) -> Result<(), String> {
    let dir = world_dir.join(RECORDS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} can't be made ({e})", dir.display()))?;
    // In reverse key order a parent's prefix comes after its children's.
    for (key, bytes) in records.iter().chain(nodes.iter().rev()) {
        let path = record_path(world_dir, key);
        if std::fs::read(&path).is_ok_and(|held| held == *bytes) {
            continue;
        }
        crate::save::write_atomic_nosync(&path, bytes)?;
    }
    crate::fsync::sync_dir(&dir).map_err(|e| e.to_string())?;
    crate::fsync::sync_dir(world_dir).map_err(|e| e.to_string())?;
    let bound: std::collections::BTreeSet<String> = records.iter().chain(nodes).map(|(key, _)| hex::encode(key)).collect();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{} can't be listed ({e})", dir.display()))?;
    for entry in entries.flatten() {
        if entry.file_name().to_str().is_none_or(|name| !bound.contains(name))
            && let Err(e) = std::fs::remove_file(entry.path())
        {
            log::warn!("{}: a record nothing binds now wasn't removed: {e}", entry.path().display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(keys: &[Vec<u8>]) -> Vec<Entry> {
        let mut list: Vec<Entry> = keys.iter().map(|key| (key.clone(), sha256(key))).collect();
        list.sort_unstable();
        list
    }

    fn chunk_key(cx: i32, cy: i32, cz: i32) -> Vec<u8> {
        let mut key = vec![b'c'];
        key.extend_from_slice(&cx.to_le_bytes());
        key.extend_from_slice(&cy.to_le_bytes());
        key.extend_from_slice(&cz.to_le_bytes());
        key
    }

    fn world(n: i32) -> Vec<Vec<u8>> {
        let mut keys: Vec<Vec<u8>> = (0..n).flat_map(|x| (0..6).map(move |y| chunk_key(x, y, x % 9))).collect();
        keys.push(b"m".to_vec());
        keys.push(b"w".to_vec());
        keys
    }

    fn in_memory(tree: &Tree) -> Vec<Entry> {
        fn descend(list: &[Entry], nodes: &[Stored], flat: &mut Vec<Entry>) {
            for (key, digest) in list {
                if key[0] != MANIFEST {
                    flat.push((key.clone(), *digest));
                    continue;
                }
                let (_, bytes) = nodes.iter().find(|(k, _)| k == key).expect("every child is a node");
                assert_eq!(sha256(bytes), *digest);
                assert!(bytes.len() <= MAX_RECORD_BYTES, "{} B", bytes.len());
                let mut r = Reader::new(bytes);
                descend(&manifest::read(&mut r).expect("reads"), nodes, flat);
            }
        }
        let mut flat = Vec::new();
        descend(std::slice::from_ref(&tree.top), &tree.nodes, &mut flat);
        flat
    }

    #[test]
    fn walking_a_tree_gives_back_the_sorted_list() {
        for n in [0, 1, 10, 11, 500, 5_000] {
            let list = entries(&world(n));
            assert_eq!(in_memory(&build_tree(&list).unwrap()), list, "{n} columns");
        }
    }

    #[test]
    fn a_key_that_is_a_prefix_of_others_has_its_place() {
        let mut keys: Vec<Vec<u8>> = (0..200u8).map(|i| vec![b'j', b'a', i]).collect();
        keys.push(b"ja".to_vec());
        let list = entries(&keys);
        assert_eq!(in_memory(&build_tree(&list).unwrap()), list);
    }

    #[test]
    fn one_changed_entry_rewrites_one_path() {
        let mut list = entries(&world(1_000));
        let before = build_tree(&list).unwrap();
        list[1_234].1 = [7; 32];
        let after = build_tree(&list).unwrap();
        assert_eq!(before.nodes.len(), after.nodes.len());
        let changed = before.nodes.iter().zip(&after.nodes).filter(|(a, b)| a != b).count();
        assert!(changed <= 5, "{changed} of {} records changed", before.nodes.len());
        assert_ne!(before.top, after.top);
    }

    #[test]
    fn one_added_entry_rewrites_one_path() {
        let keys = world(1_000);
        let before = build_tree(&entries(&keys)).unwrap();
        let mut more = keys.clone();
        more.push(chunk_key(500, 7, 500 % 9));
        let after = build_tree(&entries(&more)).unwrap();
        let held: std::collections::BTreeSet<&Stored> = before.nodes.iter().collect();
        let changed = after.nodes.iter().filter(|node| !held.contains(node)).count();
        assert!(changed <= 5, "{changed} of {} records changed", after.nodes.len());
    }

    #[test]
    fn a_key_too_long_for_the_runner_is_an_error() {
        let list = entries(&[vec![b'j'; MAX_RECORD_KEY]]);
        assert!(build_tree(&list).is_err());
        let list = entries(&[vec![b'j'; MAX_RECORD_KEY - 1]]);
        assert!(build_tree(&list).is_ok());
    }

    use super::piece_key as key;

    /// The schema's number: a change is a new schema, not a tuning. Under the runner's
    /// record-value limit of 1 MiB less headers.
    #[test]
    fn a_files_piece_size_is_the_schemas_number() {
        assert_eq!(FILE_PIECE_BYTES, 65_536);
        assert!(FILE_PIECE_BYTES < (1 << 20) - 4096);
    }

    #[test]
    fn an_empty_file_is_one_empty_piece_and_a_whole_one_has_no_empty_tail() {
        assert_eq!(file_pieces(&[]), vec![&[] as &[u8]]);
        let exact = vec![1u8; FILE_PIECE_BYTES * 2];
        assert_eq!(file_pieces(&exact).len(), 2);
        let over = vec![1u8; FILE_PIECE_BYTES * 2 + 1];
        let pieces = file_pieces(&over);
        assert_eq!(pieces.len(), 3);
        assert_eq!(pieces[2].len(), 1);
        assert_eq!(pieces.concat(), over);
    }

    /// After the batch, the store is the bound set: every piece of the new bytes, and
    /// nothing past them, whatever the old count.
    #[test]
    fn a_batch_of_a_files_pieces_leaves_the_store_equal_to_the_bound_set() {
        let piece = FILE_PIECE_BYTES;
        for (new_len, pieces) in [(piece * 3, 3u32), (piece * 3 + 1, 4), (piece, 1), (1, 1), (0, 1)] {
            let bytes: Vec<u8> = (0..new_len).map(|i| (i % 251) as u8).collect();
            for old in [0u32, 1, 2, 3, 4, 5, 40] {
                let batch = file_piece_batch(Some(&bytes), old);
                let keys: Vec<_> = batch.writes.iter().map(|(k, _)| k.clone()).collect();
                assert_eq!(keys, (0..pieces).map(|i| key(b'w', i)).collect::<Vec<_>>(), "{new_len} B, {old} before");
                assert_eq!(batch.writes.iter().map(|(_, b)| b.to_vec()).collect::<Vec<_>>().concat(), bytes);
                let gone: Vec<_> = (pieces..old.max(pieces)).map(|i| key(b'w', i)).collect();
                assert_eq!(batch.removes, gone, "{new_len} B, {old} before");
            }
        }
    }

    #[test]
    fn a_shrink_by_exactly_one_piece_removes_exactly_the_last_key() {
        let bytes = vec![0u8; FILE_PIECE_BYTES * 2];
        assert_eq!(file_piece_batch(Some(&bytes), 3).removes, vec![key(b'w', 2)]);
    }

    /// A file that has gone leaves no piece behind.
    #[test]
    fn a_batch_of_a_file_that_has_gone_removes_every_piece() {
        let batch = file_piece_batch(None, 2);
        assert!(batch.writes.is_empty());
        assert_eq!(batch.removes, vec![key(b'w', 0), key(b'w', 1)]);
    }

    #[test]
    fn every_piece_is_within_the_limit_and_they_join_back() {
        let state = StateRecords { layout: vec![1; 3], lava: vec![2; MAX_RECORD_BYTES * 2 + 1], exposure: vec![3; MAX_RECORD_BYTES], ..StateRecords::default() };
        let pieces = state.pieces();
        assert_eq!(pieces.len(), 1 + 3 + 1);
        assert!(pieces.iter().all(|(key, bytes)| is_state_key(key) && bytes.len() <= MAX_RECORD_BYTES));
    }
}

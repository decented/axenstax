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

use std::collections::BTreeMap;
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
/// No record here is longer, well inside the runner's limit of one batch. A manifest
/// record can't reach it: 257 entries of at most `4 + 128 + 32` bytes.
pub(crate) const MAX_RECORD_BYTES: usize = 64 * 1024;
const LEAF_ENTRIES: usize = 64;

pub(super) const MANIFEST: u8 = b't';
pub(super) const LAYOUT: u8 = b'l';
pub(super) const LAVA: u8 = b'q';
pub(super) const EXPOSURE: u8 = b'p';
const FAMILIES: [(u8, &str); 3] = [(LAYOUT, "layout"), (LAVA, "lava queue"), (EXPOSURE, "exposure clocks")];

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

/// What a family's key says, for a refusal.
pub(super) fn describe(key: &[u8]) -> Option<String> {
    FAMILIES.iter().find(|(family, _)| key.len() == 5 && *family == key[0]).map(|(_, name)| {
        format!("{name} piece {}", u32::from_be_bytes([key[1], key[2], key[3], key[4]]))
    })
}

/// The bytes of the three state records, whole.
#[derive(Default)]
pub(super) struct StateRecords {
    pub layout: Vec<u8>,
    pub lava: Vec<u8>,
    pub exposure: Vec<u8>,
}

impl StateRecords {
    /// As they are stored: at least one piece each, in key order.
    pub(super) fn pieces(&self) -> Vec<Stored> {
        let mut out = Vec::new();
        for (family, bytes) in [(LAYOUT, &self.layout), (LAVA, &self.lava), (EXPOSURE, &self.exposure)] {
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
    let mut whole: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
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
        whole.insert(family, bytes);
    }
    let mut take = |family| whole.remove(&family).unwrap_or_default();
    Ok((StateRecords { layout: take(LAYOUT), lava: take(LAVA), exposure: take(EXPOSURE) }, found))
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

    #[test]
    fn every_piece_is_within_the_limit_and_they_join_back() {
        let state = StateRecords { layout: vec![1; 3], lava: vec![2; MAX_RECORD_BYTES * 2 + 1], exposure: vec![3; MAX_RECORD_BYTES] };
        let pieces = state.pieces();
        assert_eq!(pieces.len(), 1 + 3 + 1);
        assert!(pieces.iter().all(|(key, bytes)| is_state_key(key) && bytes.len() <= MAX_RECORD_BYTES));
    }
}

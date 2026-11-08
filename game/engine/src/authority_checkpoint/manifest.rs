//! The manifest: a sorted list of `(record key, SHA-256 of the record's bytes)`
//! naming what the world's stored records hold. `state_hash` covers the world
//! through it (handoff §6, choice 1). The checkpoint names its root record and the
//! list lies in a tree of stored records (`records.rs`), so its size isn't the image's.
//!
//! | Key | Record | Read from |
//! |---|---|---|
//! | `b"c"` + `(cx, cy, cz)` | a non-empty chunk, the 8,704 B a save writes | memory |
//! | `b"h"` + path | a handed-over world's `handover/` or `participants/` file, as stored | [`StoredRecords`] |
//! | `b"j"` + file name | a joiner's sidecar, as stored | [`StoredRecords`] |
//! | `b"l"`, `b"p"`, `b"f"`, `b"s"`, `b"u"`, `b"k"` + piece | the layout, exposure clocks, water, cell sims, player slots and corpses | memory |
//! | `b"q"`, `b"r"` + piece | lava's spread and retract queues, at stable positions | memory |
//! | `b"x"` + section | lava's source differences, by 16-cube section | memory |
//! | `b"t"` + prefix | a record of the manifest tree itself, which names others the same way | memory |
//! | `b"m"` | the world's meta as stored, the value of `pop_secret` cut | [`StoredRecords`] |
//! | `b"w"` + piece | `world.dat` as stored, in fixed pieces: every table of `WorldSave` | [`StoredRecords`] |
//!
//! A server save writes every non-empty chunk it holds, so that is what is listed.
//!
//! `world.dat` is bound in fixed pieces (`records.rs`), so it can pass the runner's
//! record-value limit whatever the world's tables reach, and a field added to the save is
//! bound by being in the file. The file stays whole where a save writes it: a restore
//! cuts what is on disk, so a file longer than the bound set has a piece the manifest
//! doesn't name, and one shorter lacks one it does. A server writes its rows in one order
//! (`WorldSave::sort_unordered_rows`), so the same tables are the same bytes.
//!
//! The manifest is only true of the disk straight after a save. A record that has
//! since changed doesn't match, and a checkpoint of a later tick than the save is
//! refused by the clock `world.dat` holds, as an unsaved change to a table doesn't
//! move the digest until the save stores it. The runner persists, then checkpoints.
//!
//! A digest is over a record's stored bytes ([`stored_mismatch`] reads them), never
//! a parsed struct encoded again, so a build with another field doesn't see an
//! unchanged record as changed. A chunk's digest is taken from memory, so that a
//! checkpoint moves with an unsaved block; its `as_bytes` is exactly what a save
//! writes, not an encoding of parsed fields.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::codec::{Malformed, Reader, Writer};
use super::records::{self, sha256, StateRecords, FILE_PIECE_BYTES, MANIFEST, MAX_RECORD_KEY};
use super::RestoreError;
use crate::server::GameServer;

pub(super) type Entry = (Vec<u8>, [u8; 32]);

const HANDOVER: u8 = b'h';
/// Where a handed-over world keeps what a restore reads of its authority: every file
/// but a lock or a temporary one, which nothing reads as a record.
const HANDOVER_FOLDERS: [&str; 2] = ["handover", "participants"];
/// More than the `handover/` scan's own limit of 256, and than a manifest's agreements.
const MAX_HANDOVER_RECORDS: usize = 4096;

/// What this server knows of the records it holds no copy of, so that a
/// checkpoint reads no file: written where those records are read or written,
/// and only ever read by the checkpoint.
#[derive(Default)]
pub(crate) struct StoredRecords {
    /// The digest of the world's stored meta, without its secret. `None`: no meta
    /// file. A `Mutex`, as a save takes `&self`.
    meta: Mutex<Option<[u8; 32]>>,
    /// The key and digest of each piece of the stored `world.dat`. Empty: no such file yet.
    world_dat: Mutex<Vec<Entry>>,
    /// Every sidecar in the world's `joiners/`, by file name. A `Mutex`, as a
    /// save takes `&self` (as `World::disk_chunks` does).
    joiners: Mutex<BTreeMap<String, [u8; 32]>>,
    /// Every record of a handed-over world's authority, by path in its folder.
    handover: Mutex<BTreeMap<String, [u8; 32]>>,
    /// What couldn't be read when it was last noted. A record left out can't be bound,
    /// and a restore would refuse the checkpoint that left it out, so none is written
    /// while this holds anything. Which of them: the `note_*` that failed.
    unreadable: Mutex<BTreeMap<&'static str, String>>,
    /// The keys the last record set bound, so that the next knows which to remove
    /// (`record_set`). Keys only; empty until a persist.
    bound: Mutex<std::collections::BTreeSet<Vec<u8>>>,
}

/// The sidecars are one file per persona ever seen. A name too long for a record key
/// (and the manifest record over it) is no persona's, and nothing reads it.
fn is_sidecar_name(name: &str) -> bool {
    name.starts_with("npub") && name.ends_with(".dat") && name.len() + 2 < MAX_RECORD_KEY
}

impl StoredRecords {
    /// What a read came to. A normal save goes on as it did, with the error logged; it
    /// is only a checkpoint or an authority's persist that can't go on without it.
    fn noted<T: Default>(&self, world: &str, what: &'static str, read: Result<T, String>) -> T {
        let mut unreadable = self.unreadable.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match read {
            Ok(value) => {
                unreadable.remove(what);
                value
            }
            Err(e) => {
                // A normal save goes on regardless; only a checkpoint or an authority's
                // persist stops, and it says why (`why_unbindable`).
                log::warn!("world '{world}': the checkpoint can't bind {what}: {e}");
                unreadable.insert(what, e);
                T::default()
            }
        }
    }

    /// Why no checkpoint can bind the records as they were last noted, if so.
    pub(super) fn why_unbindable(&self) -> Option<String> {
        let unreadable = self.unreadable.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        unreadable.iter().next().map(|(what, why)| format!("{what} can't be read, so it can't be bound: {why}"))
    }

    /// Learn the world's meta from the disk: at the load, after anything that can
    /// rewrite it, and at each persist, so the digest is of the bytes that are there.
    /// A world with no file has no record: the defaults `load_world_meta` falls back
    /// to hold a random seed and today's date, which are nothing to bind to.
    pub(crate) fn note_meta(&self, world: &str) {
        let digest = self.noted(world, "the meta", stored_meta_digest(&crate::save::world_dir(world)));
        *self.meta.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = digest;
    }

    /// Learn `world.dat` from the disk: at the load and at each persist, whichever
    /// way its commit went, so the digests are of the bytes that are there.
    pub(crate) fn note_world_dat(&self, world: &str) {
        let pieces = self.noted(world, "world.dat", stored_world_dat_pieces(&crate::save::world_dir(world), u64::MAX));
        *self.world_dat.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = pieces;
    }

    /// Learn a handed-over world's authority records from the disk: at the load and at
    /// each persist. A world with no `handover/` has none.
    pub(crate) fn note_handover(&self, world: &str) {
        let records = self.noted(world, "its handover records", stored_handover(&crate::save::world_dir(world)));
        *self.handover.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = records;
    }

    /// Whatever sidecars the world folder holds: at load, including the records
    /// of joiners not connected now.
    pub(crate) fn scan_joiners(&mut self, world_dir: &std::path::Path) {
        let world = world_dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let joiners = self.noted(world, "its sidecars", stored_sidecars(world_dir));
        *self.joiners.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner) = joiners;
    }

    /// A sidecar this server just wrote.
    pub(crate) fn note_joiner(&self, path: &std::path::Path, bytes: &[u8]) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return };
        self.joiners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name.to_string(), sha256(bytes));
    }

    /// The keys the last persist bound.
    pub(crate) fn bound_keys(&self) -> std::collections::BTreeSet<Vec<u8>> {
        self.bound.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub(super) fn set_bound_keys(&self, keys: std::collections::BTreeSet<Vec<u8>>) {
        *self.bound.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = keys;
    }

    #[cfg(test)]
    pub(crate) fn joiner_digest(&self, name: &str) -> Option<[u8; 32]> {
        self.joiners.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(name).copied()
    }

    #[cfg(test)]
    pub(crate) fn meta_digest(&self) -> Option<[u8; 32]> {
        *self.meta.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Every sidecar in a world's `joiners/`, as its file holds it, by file name.
/// `Err`: one that can't be read, which a restore would then report as not in the
/// manifest though nothing is wrong with it.
fn stored_sidecars(world_dir: &Path) -> Result<BTreeMap<String, [u8; 32]>, String> {
    let mut joiners = BTreeMap::new();
    let entries = match std::fs::read_dir(world_dir.join("joiners")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(joiners),
        Err(e) => return Err(format!("joiners/ can't be listed ({e})")),
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else { continue };
        if !is_sidecar_name(&name) {
            continue;
        }
        let bytes = std::fs::File::open(entry.path())
            .map_err(|e| e.to_string())
            .and_then(|file| crate::save::read_bounded(file, crate::joiner_save::MAX_JOINER_SAVE_BYTES))
            .map_err(|e| format!("sidecar {name}: {e}"))?;
        joiners.insert(name, sha256(&bytes));
    }
    Ok(joiners)
}

/// Every record a restore of a handed-over world reads: all `open_guard` and `proof`
/// scan in `handover/`, and the progress `load_or_bootstrap` reads in `participants/`.
/// `Err`: one that isn't a readable regular file, which isn't what any persist bound.
fn stored_handover(world_dir: &Path) -> Result<BTreeMap<String, [u8; 32]>, String> {
    let mut records = BTreeMap::new();
    for folder in HANDOVER_FOLDERS {
        let entries = match std::fs::read_dir(world_dir.join(folder)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{folder}/ can't be listed ({e})")),
        };
        for entry in entries {
            let entry = entry.map_err(|e| format!("{folder}/ can't be listed ({e})"))?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| format!("a name in {folder}/ that isn't text"))?;
            if name.starts_with('.') || name.contains(".tmp-") {
                continue;
            }
            if records.len() >= MAX_HANDOVER_RECORDS {
                return Err(format!("more than {MAX_HANDOVER_RECORDS} handover records"));
            }
            let bytes = crate::handover::live::personal_adapter::read_regular(&entry.path(), crate::handover::manifest::MANIFEST_LIMIT as u64)
                .map_err(|e| format!("{folder}/{name}: {e}"))?;
            records.insert(format!("{folder}/{name}"), sha256(&bytes));
        }
    }
    Ok(records)
}

/// The key and digest of each piece of the stored `world.dat`, of at most its first `most`
/// bytes, as a restore has no use for more than one piece past what it binds. Empty: no
/// such file.
fn stored_world_dat_pieces(world_dir: &Path, most: u64) -> Result<Vec<Entry>, String> {
    let file = match std::fs::File::open(world_dir.join("world.dat")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("world.dat can't be read ({e})")),
    };
    let mut stored = Vec::new();
    file.take(most).read_to_end(&mut stored).map_err(|e| format!("world.dat can't be read ({e})"))?;
    Ok(records::file_entries(&stored))
}

fn stored_meta_digest(world_dir: &Path) -> Result<Option<[u8; 32]>, String> {
    match std::fs::read(world_dir.join("world_meta.json")) {
        Ok(stored) => Ok(Some(meta_record_digest(&stored))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("world_meta.json can't be read ({e})")),
    }
}

/// Every record the sorted `wanted` binds, read as it lies on disk before any load has
/// parsed it and checked against its digest. What comes back is the state that lies in
/// records, whole. A disk that can't be read is a `Fault`.
///
/// A chunk file the manifest doesn't name is no mismatch: it is a column the run
/// dropped, and the layout keeps it out of the restored world. A sidecar, meta, handover
/// record or piece of `world.dat` it doesn't name is one, as each of those is read whole
/// into the world. A manifest with no `world.dat` is a checkpoint from before any persist.
/// `world.dat` is only digested here, piece by piece: its readers come after.
pub(super) fn verify_stored(wanted: &[Entry], world_dir: &Path) -> Result<StateRecords, RestoreError> {
    let disagree = |why: String| RestoreError::InvalidCheckpoint(format!("the stored records and the manifest disagree: {why}"));
    let world_dat_pieces = wanted.iter().filter(|(key, _)| records::is_world_dat_piece(key)).count() as u64;
    if world_dat_pieces == 0 {
        return Err(disagree("no world.dat is bound".to_string()));
    }
    let (state, mut found) = records::read_state(wanted, world_dir)?;
    for (key, _) in wanted.iter().filter(|(key, _)| key.first() == Some(&b'c') && key.len() == 13) {
        let at = |i: usize| i32::from_le_bytes([key[i], key[i + 1], key[i + 2], key[i + 3]]);
        let file = world_dir.join("chunks").join(format!("{}_{}_{}.chunk", at(1), at(5), at(9)));
        match std::fs::read(&file) {
            Ok(stored) => found.push((key.clone(), sha256(&stored))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(RestoreError::Fault(format!("{} can't be read ({e})", file.display()))),
        }
    }
    for (name, digest) in stored_sidecars(world_dir).map_err(RestoreError::Fault)? {
        let mut key = vec![b'j'];
        key.extend_from_slice(name.as_bytes());
        found.push((key, digest));
    }
    for (path, digest) in stored_handover(world_dir).map_err(&disagree)? {
        found.push((handover_key(&path), digest));
    }
    // Enough to show a piece past the bound set, if there is one.
    let most = (world_dat_pieces + 1).saturating_mul(FILE_PIECE_BYTES as u64);
    found.extend(stored_world_dat_pieces(world_dir, most).map_err(RestoreError::Fault)?);
    match stored_meta_digest(world_dir).map_err(RestoreError::Fault)? {
        Some(digest) => found.push((b"m".to_vec(), digest)),
        // The load would rebuild a meta from this: a write, of a record nobody bound.
        None if crate::save::has_quarantined_sibling(world_dir, "world_meta.json") => {
            return Err(disagree("the world meta was kept aside as damaged".to_string()));
        }
        None => {}
    }
    found.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    match first_mismatch(wanted, &found) {
        Some(why) => Err(disagree(why)),
        None => Ok(state),
    }
}

fn handover_key(path: &str) -> Vec<u8> {
    let mut key = vec![HANDOVER];
    key.extend_from_slice(path.as_bytes());
    key
}

/// The digest of a stored meta: all of its bytes but the value of `pop_secret`,
/// which the header's key id binds instead. Nothing is parsed into a struct or
/// encoded again, so the digest of an unchanged file holds for any build.
fn meta_record_digest(stored: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    match secret_value_span(stored) {
        Some(span) => {
            h.update(&stored[..span.start]);
            h.update(&stored[span.end..]);
        }
        None => h.update(stored),
    }
    h.finalize().into()
}

/// Where the value of the top-level `"pop_secret"` member lies in a JSON object.
/// Bytes that aren't such an object have none, and are digested whole: a torn file
/// matches nothing anyway.
fn secret_value_span(json: &[u8]) -> Option<std::ops::Range<usize>> {
    const KEY: &[u8] = b"\"pop_secret\"";
    // The closing quote of the string opening at `from`.
    let string_end = |from: usize| {
        let mut i = from + 1;
        while i < json.len() {
            match json[i] {
                b'\\' => i += 2,
                b'"' => return Some(i),
                _ => i += 1,
            }
        }
        None
    };
    let skip_space = |mut i: usize| {
        while json.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        i
    };
    let (mut depth, mut i) = (0usize, 0usize);
    while i < json.len() {
        match json[i] {
            b'"' => {
                let end = string_end(i)?;
                let colon = skip_space(end + 1);
                if depth == 1 && &json[i..=end] == KEY && json.get(colon) == Some(&b':') {
                    let start = skip_space(colon + 1);
                    let (mut inner, mut at) = (0usize, start);
                    loop {
                        match *json.get(at)? {
                            b'"' => at = string_end(at)?,
                            b'{' | b'[' => inner += 1,
                            b'}' | b']' if inner == 0 => break,
                            b'}' | b']' => inner -= 1,
                            b',' if inner == 0 => break,
                            _ => {}
                        }
                        at += 1;
                    }
                    let mut stop = at;
                    while stop > start && json[stop - 1].is_ascii_whitespace() {
                        stop -= 1;
                    }
                    return Some(start..stop);
                }
                i = end;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The key of a chunk's record.
pub(super) fn chunk_key(cx: i32, cy: i32, cz: i32) -> Vec<u8> {
    let mut key = Vec::with_capacity(13);
    key.push(b'c');
    key.extend_from_slice(&cx.to_le_bytes());
    key.extend_from_slice(&cy.to_le_bytes());
    key.extend_from_slice(&cz.to_le_bytes());
    key
}

/// The records this server holds a digest of and no bytes: the sidecars of every joiner
/// the world has seen, a handed-over world's records, the meta and `world.dat`'s pieces, as
/// last noted from the disk. Unsorted.
pub(super) fn held_entries(s: &GameServer) -> Result<Vec<Entry>, String> {
    if let Some(why) = s.stored_records.why_unbindable() {
        return Err(why);
    }
    let mut list: Vec<Entry> = Vec::new();
    let joiners = s.stored_records.joiners.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for (name, digest) in joiners.iter() {
        let mut key = Vec::with_capacity(1 + name.len());
        key.push(b'j');
        key.extend_from_slice(name.as_bytes());
        list.push((key, *digest));
    }
    drop(joiners);
    let handover = s.stored_records.handover.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for (path, digest) in handover.iter() {
        list.push((handover_key(path), *digest));
    }
    drop(handover);
    if let Some(digest) = *s.stored_records.meta.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        list.push((b"m".to_vec(), digest));
    }
    list.extend_from_slice(&s.stored_records.world_dat.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    Ok(list)
}

/// Every record but the state that lies in records, which is added once it is encoded
/// (`GameServer::bound_records`). Sorted by key.
pub(super) fn entries(s: &GameServer) -> Result<Vec<Entry>, String> {
    let mut list = held_entries(s)?;
    for ((cx, cy, cz), chunk) in s.world.persistable_chunks() {
        if !chunk.is_empty() {
            list.push((chunk_key(cx, cy, cz), sha256(&chunk.as_bytes())));
        }
    }
    list.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(list)
}

pub(super) fn write(w: &mut Writer, entries: &[Entry]) {
    w.count(entries.len());
    for (key, digest) in entries {
        w.blob(key);
        w.raw(digest);
    }
}

pub(super) fn read(r: &mut Reader<'_>) -> Result<Vec<Entry>, Malformed> {
    let n = r.count(4 + 32)?;
    let mut list: Vec<Entry> = Vec::with_capacity(n);
    for _ in 0..n {
        let key = r.blob()?.to_vec();
        let digest = r.bytes32()?;
        if list.last().is_some_and(|(last, _): &Entry| sort_name(last) >= sort_name(&key)) {
            return Err(Malformed("a manifest that isn't in its one sorted order"));
        }
        list.push((key, digest));
    }
    Ok(list)
}

/// Where a key sorts: a manifest record by the prefix it holds, so that a list reads in
/// the order of what it binds, at every level.
fn sort_name(key: &[u8]) -> &[u8] {
    if key.first() == Some(&MANIFEST) { &key[1..] } else { key }
}

/// What differs between the checkpoint's manifest and what the load read, named.
pub(super) fn first_mismatch(wanted: &[Entry], found: &[Entry]) -> Option<String> {
    let describe = |key: &[u8]| match key.first() {
        Some(b'c') if key.len() == 13 => {
            let at = |i: usize| i32::from_le_bytes([key[i], key[i + 1], key[i + 2], key[i + 3]]);
            format!("chunk ({}, {}, {})", at(1), at(5), at(9))
        }
        Some(b'j') => format!("sidecar {}", String::from_utf8_lossy(&key[1..])),
        Some(&HANDOVER) => format!("handover record {}", String::from_utf8_lossy(&key[1..])),
        Some(b'm') => "the world meta".to_string(),
        _ => records::describe(key).unwrap_or_else(|| "an unknown record".to_string()),
    };
    let (mut a, mut b) = (wanted.iter().peekable(), found.iter().peekable());
    loop {
        match (a.peek(), b.peek()) {
            (None, None) => return None,
            (Some((key, _)), None) => return Some(format!("{} is missing", describe(key))),
            (None, Some((key, _))) => return Some(format!("{} is not in the manifest", describe(key))),
            (Some((ka, da)), Some((kb, db))) => match ka.cmp(kb) {
                std::cmp::Ordering::Less => return Some(format!("{} is missing", describe(ka))),
                std::cmp::Ordering::Greater => return Some(format!("{} is not in the manifest", describe(kb))),
                std::cmp::Ordering::Equal if da != db => return Some(format!("{} doesn't match", describe(ka))),
                std::cmp::Ordering::Equal => {
                    a.next();
                    b.next();
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(json: &str) -> Option<&str> {
        secret_value_span(json.as_bytes()).map(|span| &json[span])
    }

    #[test]
    fn only_the_top_level_secrets_value_is_cut() {
        assert_eq!(cut(r#"{"a": 1, "pop_secret": [1, 2], "b": 2}"#), Some("[1, 2]"));
        assert_eq!(cut(r#"{"pop_secret":[1]}"#), Some("[1]"));
        assert_eq!(cut("{\n  \"a\": 1,\n  \"pop_secret\": [\n    1\n  ]\n}"), Some("[\n    1\n  ]"));
        assert_eq!(cut(r#"{"pop_secret": null, "b": 2}"#), Some("null"));
    }

    #[test]
    fn a_secret_that_is_only_named_is_not_cut() {
        assert_eq!(cut(r#"{"a": 1}"#), None);
        assert_eq!(cut(r#"{"a": {"pop_secret": [1]}}"#), None, "nested");
        assert_eq!(cut(r#"{"a": "pop_secret", "b": 2}"#), None, "a value");
        assert_eq!(cut(r#"{"a": "\"pop_secret\": [1]"}"#), None, "inside a string");
        assert_eq!(cut(r#"["pop_secret"]"#), None, "not an object");
        assert_eq!(cut("{ torn"), None);
        assert_eq!(cut(r#"{"pop_secret": [1"#), None, "torn inside the value");
    }

    #[test]
    fn every_other_byte_of_a_meta_moves_its_digest() {
        let meta = br#"{"a": 1, "pop_secret": [1, 2], "b": 2}"#;
        let same_secret_gone = br#"{"a": 1, "pop_secret": [9, 9, 9], "b": 2}"#;
        assert_eq!(meta_record_digest(meta), meta_record_digest(same_secret_gone));
        for at in (0..meta.len()).filter(|at| !(23..29).contains(at)) {
            let mut changed = meta.to_vec();
            changed[at] ^= 1;
            assert_ne!(meta_record_digest(&changed), meta_record_digest(meta), "byte {at}");
        }
    }
}

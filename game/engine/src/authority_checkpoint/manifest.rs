//! The manifest: a sorted list of `(record key, SHA-256 of the record's bytes)`
//! naming what the world's stored records hold. `state_hash` covers the world
//! through it (handoff §6, choice 1).
//!
//! | Key | Record | Read from |
//! |---|---|---|
//! | `b"c"` + `(cx, cy, cz)` | a non-empty chunk, the 8,704 B a save writes | memory |
//! | `b"j"` + file name | a joiner's sidecar, as stored | [`StoredRecords`] |
//! | `b"m"` | the world's meta as stored, the value of `pop_secret` cut | [`StoredRecords`] |
//! | `b"b"` | block entities, drying racks, face attachments and block meta | memory |
//!
//! A server save writes every non-empty chunk it holds, so that is what is
//! listed, and the list is what meets the 1 MiB cap first.
//!
//! The `b"b"` record is not named in the slice 2 spec's list. Without it a
//! checkpoint's hash would not move for a furnace's contents, because a block
//! entity sits in `world.dat`, which isn't canonical. It is the same canonical
//! view the `block_entities` digest section takes, and costs one entry.
//!
//! The manifest is only true of the disk straight after a save: a restore from a
//! checkpoint taken any later finds a record that doesn't match and refuses. The
//! runner persists, then checkpoints.
//!
//! A digest is over a record's stored bytes ([`stored_mismatch`] reads them), never
//! a parsed struct encoded again, so a build with another `WorldMeta` field doesn't
//! see an unchanged record as changed. A chunk's digest is taken from memory, so
//! that a checkpoint moves with an unsaved block; its `as_bytes` is exactly what a
//! save writes, not an encoding of parsed fields. The one exception is `b"b"`, which
//! has no stored bytes of its own while those tables sit in `world.dat`: it is
//! compared after the load, and a new block-entity field still moves it, until those
//! tables become records of their own (handoff §6, slice 3).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::codec::{Malformed, Reader, Writer};
use crate::server::GameServer;

pub(super) type Entry = (Vec<u8>, [u8; 32]);

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// What this server knows of the records it holds no copy of, so that a
/// checkpoint reads no file: written where those records are read or written,
/// and only ever read by the checkpoint.
#[derive(Default)]
pub(crate) struct StoredRecords {
    /// The digest of the world's stored meta, without its secret. `None`: no meta
    /// file. A `Mutex`, as a save takes `&self`.
    meta: Mutex<Option<[u8; 32]>>,
    /// Every sidecar in the world's `joiners/`, by file name. A `Mutex`, as a
    /// save takes `&self` (as `World::disk_chunks` does).
    joiners: Mutex<BTreeMap<String, [u8; 32]>>,
}

/// The sidecars are one file per persona ever seen.
fn is_sidecar_name(name: &str) -> bool {
    name.starts_with("npub") && name.ends_with(".dat")
}

impl StoredRecords {
    /// Learn the world's meta from the disk: at the load, after anything that can
    /// rewrite it, and at each persist, so the digest is of the bytes that are there.
    /// A world with no file has no record: the defaults `load_world_meta` falls back
    /// to hold a random seed and today's date, which are nothing to bind to.
    pub(crate) fn note_meta(&self, world: &str) {
        let digest = match stored_meta_digest(&crate::save::world_dir(world)) {
            Ok(digest) => digest,
            Err(e) => {
                log::error!("world '{world}': the checkpoint can't bind the meta: {e}");
                None
            }
        };
        *self.meta.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = digest;
    }

    /// Whatever sidecars the world folder holds: at load, including the records
    /// of joiners not connected now.
    pub(crate) fn scan_joiners(&mut self, world_dir: &std::path::Path) {
        let joiners = stored_sidecars(world_dir);
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
fn stored_sidecars(world_dir: &Path) -> BTreeMap<String, [u8; 32]> {
    let mut joiners = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(world_dir.join("joiners")) {
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            if !is_sidecar_name(&name) {
                continue;
            }
            let read = std::fs::File::open(entry.path())
                .map_err(|e| e.to_string())
                .and_then(|file| crate::save::read_bounded(file, crate::joiner_save::MAX_JOINER_SAVE_BYTES));
            match read {
                Ok(bytes) => {
                    joiners.insert(name, sha256(&bytes));
                }
                Err(e) => log::warn!("Sidecar {name} left out of the checkpoint's records: {e}"),
            }
        }
    }
    joiners
}

fn stored_meta_digest(world_dir: &Path) -> Result<Option<[u8; 32]>, String> {
    match std::fs::read(world_dir.join("world_meta.json")) {
        Ok(stored) => Ok(Some(meta_record_digest(&stored))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("world_meta.json can't be read ({e})")),
    }
}

/// What, if anything, differs between the manifest and the records as they lie on
/// disk, read before any load has parsed them. `Err` is a disk that can't be read.
///
/// A chunk file the manifest doesn't name is no mismatch: it is a column the run
/// dropped, and the layout keeps it out of the restored world. A sidecar or meta
/// it doesn't name is one, as each of those is read whole into the world.
pub(super) fn stored_mismatch(wanted: &[Entry], world_dir: &Path) -> Result<Option<String>, String> {
    let mut found: Vec<Entry> = Vec::new();
    for (key, digest) in wanted {
        match key.first() {
            Some(b'c') if key.len() == 13 => {
                let at = |i: usize| i32::from_le_bytes([key[i], key[i + 1], key[i + 2], key[i + 3]]);
                let file = world_dir.join("chunks").join(format!("{}_{}_{}.chunk", at(1), at(5), at(9)));
                match std::fs::read(&file) {
                    Ok(stored) => found.push((key.clone(), sha256(&stored))),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("{} can't be read ({e})", file.display())),
                }
            }
            // Has no stored bytes of its own yet: compared after the load.
            Some(b'b') => found.push((key.clone(), *digest)),
            _ => {}
        }
    }
    for (name, digest) in stored_sidecars(world_dir) {
        let mut key = vec![b'j'];
        key.extend_from_slice(name.as_bytes());
        found.push((key, digest));
    }
    match stored_meta_digest(world_dir)? {
        Some(digest) => found.push((b"m".to_vec(), digest)),
        // The load would rebuild a meta from this: a write, of a record nobody bound.
        None if crate::save::has_quarantined_sibling(world_dir, "world_meta.json") => {
            return Ok(Some("the world meta was kept aside as damaged".to_string()));
        }
        None => {}
    }
    found.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(first_mismatch(wanted, &found))
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

/// The block side tables a save keeps in `world.dat`, in one canonical view.
fn block_tables_digest(s: &GameServer) -> Result<[u8; 32], String> {
    fn enc(value: &impl serde::Serialize) -> Result<Vec<u8>, String> {
        bincode::serialize(value).map_err(|e| e.to_string())
    }
    fn table<V: serde::Serialize>(
        h: &mut Sha256,
        map: &ahash::AHashMap<(i32, i32, i32), V>,
    ) -> Result<(), String> {
        let mut rows: Vec<_> = map.iter().collect();
        rows.sort_unstable_by_key(|(pos, _)| **pos);
        h.update((rows.len() as u32).to_le_bytes());
        for (pos, value) in rows {
            h.update(pos.0.to_le_bytes());
            h.update(pos.1.to_le_bytes());
            h.update(pos.2.to_le_bytes());
            let bytes = enc(value)?;
            h.update((bytes.len() as u32).to_le_bytes());
            h.update(&bytes);
        }
        Ok(())
    }
    let w = &s.world;
    let mut h = Sha256::new();
    h.update(b"block-tables\0");
    table(&mut h, &w.block_entities)?;
    table(&mut h, &w.drying_racks)?;
    table(&mut h, &w.face_attachments)?;
    table(&mut h, &w.block_meta)?;
    Ok(h.finalize().into())
}

/// Every record, sorted by key.
pub(super) fn entries(s: &GameServer) -> Result<Vec<Entry>, String> {
    let mut list: Vec<Entry> = Vec::new();
    for ((cx, cy, cz), chunk) in s.world.persistable_chunks() {
        if chunk.is_empty() {
            continue;
        }
        let mut key = Vec::with_capacity(13);
        key.push(b'c');
        key.extend_from_slice(&cx.to_le_bytes());
        key.extend_from_slice(&cy.to_le_bytes());
        key.extend_from_slice(&cz.to_le_bytes());
        list.push((key, sha256(&chunk.as_bytes())));
    }
    let joiners = s.stored_records.joiners.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for (name, digest) in joiners.iter() {
        let mut key = Vec::with_capacity(1 + name.len());
        key.push(b'j');
        key.extend_from_slice(name.as_bytes());
        list.push((key, *digest));
    }
    drop(joiners);
    if let Some(digest) = *s.stored_records.meta.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        list.push((b"m".to_vec(), digest));
    }
    list.push((b"b".to_vec(), block_tables_digest(s)?));
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
        if list.last().is_some_and(|(last, _)| *last >= key) {
            return Err(Malformed("a manifest that isn't in its one sorted order"));
        }
        list.push((key, digest));
    }
    Ok(list)
}

/// What differs between the checkpoint's manifest and what the load read, named.
pub(super) fn first_mismatch(wanted: &[Entry], found: &[Entry]) -> Option<String> {
    let describe = |key: &[u8]| match key.first() {
        Some(b'c') if key.len() == 13 => {
            let at = |i: usize| i32::from_le_bytes([key[i], key[i + 1], key[i + 2], key[i + 3]]);
            format!("chunk ({}, {}, {})", at(1), at(5), at(9))
        }
        Some(b'j') => format!("sidecar {}", String::from_utf8_lossy(&key[1..])),
        Some(b'm') => "the world meta".to_string(),
        Some(b'b') => "the block tables".to_string(),
        _ => "an unknown record".to_string(),
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

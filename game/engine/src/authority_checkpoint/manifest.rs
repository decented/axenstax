//! The manifest: a sorted list of `(record key, SHA-256 of the record's bytes)`
//! naming what the world's stored records hold. `state_hash` covers the world
//! through it (handoff §6, choice 1).
//!
//! | Key | Record | Read from |
//! |---|---|---|
//! | `b"c"` + `(cx, cy, cz)` | a non-empty chunk, its 8,704 B form | memory |
//! | `b"j"` + file name | a joiner's sidecar, as written | [`StoredRecords`] |
//! | `b"m"` | the world's meta, `pop_secret` removed | [`StoredRecords`] |
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
//! The manifest describes durable records from memory, so it is only true of
//! the disk straight after a save: a restore from a checkpoint taken any later
//! is refused as tampered. The runner persists, then checkpoints.

use std::collections::BTreeMap;
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
    /// The digest of the world's meta, without its secret. `None`: no meta file.
    meta: Option<[u8; 32]>,
    /// Every sidecar in the world's `joiners/`, by file name. A `Mutex`, as a
    /// save takes `&self` (as `World::disk_chunks` does).
    joiners: Mutex<BTreeMap<String, [u8; 32]>>,
}

/// The sidecars are one file per persona ever seen.
fn is_sidecar_name(name: &str) -> bool {
    name.starts_with("npub") && name.ends_with(".dat")
}

impl StoredRecords {
    /// The meta a server just loaded. A world with no file has no record: the
    /// defaults `load_world_meta` falls back to hold a random seed and today's
    /// date, which are nothing to bind to.
    pub(crate) fn note_meta(&mut self, world: &str, meta: &crate::save::WorldMeta) {
        let on_disk = crate::save::world_dir(world).join("world_meta.json").exists();
        self.meta = on_disk.then(|| meta_record_digest(meta));
    }

    /// Whatever sidecars the world folder holds: at load, including the records
    /// of joiners not connected now.
    pub(crate) fn scan_joiners(&mut self, world_dir: &std::path::Path) {
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
        self.meta
    }
}

/// The meta's canonical bytes are its JSON with `pop_secret` removed: its fields
/// are in struct order and it holds no hash container.
fn meta_record_digest(meta: &crate::save::WorldMeta) -> [u8; 32] {
    let mut meta = meta.clone();
    meta.pop_secret = None;
    sha256(&serde_json::to_vec(&meta).expect("a world meta encodes"))
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
    if let Some(digest) = s.stored_records.meta {
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

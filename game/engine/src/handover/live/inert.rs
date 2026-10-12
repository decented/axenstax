//! P3: a recipient's import is inert, and this machine's to discard, until
//! Prepared. At import start the session writes `inert-<transfer>.dat` next
//! to the import (in `<data>/handover/imports`); Prepared clears it. An abort
//! or timeout before Prepared removes ONLY that transfer's import (its final
//! `handover-<world>-<transfer>` directory and any
//! `.handover-receive-<transfer>-*` scratch), its Ready marker, then the inert
//! marker itself (CP10: the Ready marker lives inside the import, in its
//! working world's `handover/`). After Prepared nothing here deletes: a failure quarantines,
//! and nothing Activated is ever removed. A bounded sweep (before a new
//! recipient session and at startup) discards marked imports that have no
//! live session, using the same markers. An unmarked directory is never
//! touched.
use crate::handover::{Error, Id, Result, durable::MarkerStore};
use std::{fs, io::Read, io::Write, path::Path};

const DOMAIN: &[u8] = b"AxeNStax/handover/inert-import/v1";
/// Entries examined per sweep (the imports directory is this machine's own).
pub const SCAN_LIMIT: usize = 256;

fn name(transfer: Id) -> String {
    format!("inert-{}.dat", hex::encode(transfer))
}
fn record(transfer: Id, world: Id) -> Vec<u8> {
    let mut v = Vec::new();
    crate::handover::codec::frame(&mut v, DOMAIN);
    v.extend_from_slice(&transfer);
    v.extend_from_slice(&world);
    v
}
fn sync(dir: &Path) -> Result<()> {
    // Windows: no directory fsync (see `crate::fsync`).
    crate::fsync::sync_dir(dir).map_err(|_| Error::Io)
}

/// Mark `transfer`'s import (of snapshot `world`) inert, durably, before any
/// of its data is written.
pub fn mark(imports: &Path, transfer: Id, world: Id) -> Result<()> {
    let path = imports.join(name(transfer));
    if fs::symlink_metadata(&path).is_ok() {
        return Err(Error::State("inert marker exists"));
    }
    let temp = imports.join(format!(".{}.tmp", name(transfer)));
    let _ = fs::remove_file(&temp);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = crate::fsync::open_written(&mut opts, &temp).map_err(|_| Error::Io)?;
    f.write_all(&record(transfer, world)).and_then(|()| crate::fsync::sync_written(&f)).map_err(|_| Error::Io)?;
    fs::rename(&temp, &path).map_err(|_| Error::Io)?;
    sync(imports)
}

/// The snapshot world a marked transfer imports (`None`: not marked).
pub fn marked(imports: &Path, transfer: Id) -> Result<Option<Id>> {
    let path = imports.join(name(transfer));
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::Io),
    };
    if !meta.is_file() {
        return Err(Error::Invalid("inert marker type"));
    }
    let mut bytes = Vec::new();
    fs::File::open(&path).map_err(|_| Error::Io)?.take(128).read_to_end(&mut bytes).map_err(|_| Error::Io)?;
    let mut r = crate::handover::codec::Reader::new(&bytes, 128)?;
    r.label(std::str::from_utf8(DOMAIN).map_err(|_| Error::Invalid("inert domain"))?)?;
    let t: Id = r.array()?;
    let world: Id = r.array()?;
    r.end()?;
    if t != transfer {
        return Err(Error::Invalid("inert marker binding"));
    }
    Ok(Some(world))
}

/// Prepared: the import is no longer this machine's to discard.
pub fn clear(imports: &Path, transfer: Id) -> Result<()> {
    match fs::remove_file(imports.join(name(transfer))) {
        Ok(()) => sync(imports),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::Io),
    }
}

/// A file or folder inside an import being discarded (a symlink is
/// unlinked, never followed).
fn remove_entry(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path).map_err(|_| Error::Io),
        Ok(_) => fs::remove_file(path).map_err(|_| Error::Io),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::Io),
    }
}

fn remove_dir(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => Err(Error::Invalid("inert import type")),
        Ok(_) => fs::remove_dir_all(path).map_err(|_| Error::Io),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::Io),
    }
}

/// Review LOW-7: a crash while packing leaves `.handover-pack-*` under the
/// source's scratch folder. Called only when no source session runs (at
/// start-up and before a new one), so every such directory is an orphan.
/// Bounded scan; anything else (and a symlink) is left alone.
pub fn sweep_pack_scratch(scratch: &Path) -> Result<usize> {
    let mut removed = 0;
    for (seen, entry) in fs::read_dir(scratch).map_err(|_| Error::Io)?.enumerate() {
        if seen >= SCAN_LIMIT {
            break;
        }
        let entry = entry.map_err(|_| Error::Io)?;
        if entry.file_name().to_str().is_some_and(|n| n.starts_with(".handover-pack-")) && remove_dir(&entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Remove `transfer`'s marked inert import, its Ready marker, then the inert
/// marker. Unmarked: nothing is touched. A Prepared/Activated marker for the
/// transfer refuses (that import is no longer inert). CP10: the recipient's
/// records live in the import's own working world `handover/`
/// (`authority::dir(<import>/working)`), so they go with the import.
pub fn discard(imports: &Path, transfer: Id) -> Result<()> {
    let Some(world) = marked(imports, transfer)? else { return Ok(()) };
    let import = imports.join(format!("handover-{}-{}", hex::encode(world), hex::encode(transfer)));
    let working = import.join("working");
    let records = super::authority::dir(&working);
    if fs::symlink_metadata(&records).is_ok() {
        // CP18 (CP17 review MEDIUM-2): the import's handover lock is taken
        // and the state re-checked under it, so another copy of the game
        // whose receipt is landing (it holds the lock from before the
        // receipt) is never deleted from under; held there: skipped. The
        // lock is held while everything but `working/handover/` goes, then
        // let go (Windows can't remove a folder with the handle open).
        let Some(lock) = super::world_lock::unless_elsewhere(&working, "the inert discard") else {
            return Err(Error::State("the import is in use by another copy of the game"));
        };
        MarkerStore::new(records)?.discard_inert_ready(transfer)?;
        let entries = fs::read_dir(&import).map_err(|_| Error::Io)?;
        for entry in entries.take(SCAN_LIMIT).flatten() {
            if entry.file_name() == "working" {
                for child in fs::read_dir(&working).map_err(|_| Error::Io)?.take(SCAN_LIMIT).flatten() {
                    if child.file_name() != "handover" {
                        remove_entry(&child.path())?;
                    }
                }
            } else {
                remove_entry(&entry.path())?;
            }
        }
        drop(lock);
    }
    remove_dir(&import)?;
    let scratch = format!(".handover-receive-{}-", hex::encode(transfer));
    let mut seen = 0;
    for entry in fs::read_dir(imports).map_err(|_| Error::Io)? {
        seen += 1;
        if seen > SCAN_LIMIT {
            return Err(Error::Limit("imports directory entries"));
        }
        let entry = entry.map_err(|_| Error::Io)?;
        if entry.file_name().to_str().is_some_and(|n| n.starts_with(&scratch)) {
            remove_dir(&entry.path())?;
        }
    }
    sync(imports)?;
    clear(imports, transfer)
}

/// Discard every marked inert import except `live`'s. Bounded; returns how
/// many it removed. A marker that can't be read is left (and reported).
pub fn sweep(imports: &Path, live: Option<Id>) -> Result<usize> {
    let mut transfers = Vec::new();
    let mut seen = 0;
    for entry in fs::read_dir(imports).map_err(|_| Error::Io)? {
        seen += 1;
        if seen > SCAN_LIMIT {
            break;
        }
        let entry = entry.map_err(|_| Error::Io)?;
        let Some(hexed) = entry.file_name().to_str().and_then(|n| n.strip_prefix("inert-")?.strip_suffix(".dat").map(str::to_owned))
        else {
            continue;
        };
        let Ok(Ok(t)) = hex::decode(&hexed).map(<Id>::try_from) else { continue };
        if Some(t) != live {
            transfers.push(t);
        }
    }
    let mut removed = 0;
    let mut first_err = None;
    for t in transfers {
        match discard(imports, t) {
            Ok(()) => removed += 1,
            Err(e) => first_err = first_err.or(Some(e)),
        }
    }
    match first_err {
        Some(e) if removed == 0 => Err(e),
        _ => Ok(removed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CP18 (CP17 review MEDIUM-2): an inert import whose handover lock
    /// another copy of the game holds (its receipt may be landing) is never
    /// discarded by this one's sweep; once that copy has gone, it is.
    #[test]
    fn handover_inert_sweep_leaves_an_import_another_process_holds() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let imports = std::env::temp_dir().join(format!("axenstax-inert-held-{}", hex::encode(r)));
        let (t, w) = ([0x61; 16], [0x62; 16]);
        let import = imports.join(format!("handover-{}-{}", hex::encode(w), hex::encode(t)));
        let working = import.join("working");
        std::fs::create_dir_all(working.join("chunks")).unwrap();
        std::fs::write(working.join("world.dat"), b"w").unwrap();
        std::fs::write(import.join("archive.tar.gz"), b"a").unwrap();
        mark(&imports, t, w).unwrap();
        super::super::authority::store(&working).unwrap();
        let child = super::super::world_lock::tests::ChildHolder::hold(&working);
        assert!(sweep(&imports, None).is_err(), "the held import's discard reported success");
        assert!(working.join("world.dat").exists() && import.join("archive.tar.gz").exists(), "an import another process holds was discarded");
        child.kill();
        assert_eq!(sweep(&imports, None).unwrap(), 1);
        assert!(!import.exists(), "the import outlived its discard");
        let _ = std::fs::remove_dir_all(&imports);
    }
}

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

/// The import folder name of `transfer`'s import of the snapshot `world`
/// (`handover-<world>-<transfer>`, as `checkpoint::import` names it).
pub fn import_folder(world: Id, transfer: Id) -> String {
    format!("handover-{}-{}", hex::encode(world), hex::encode(transfer))
}

#[cfg(test)]
thread_local! {
    /// Tests: run inside `discard` right after its lock step (another
    /// copy of the game's recipient trying to take the lock there).
    pub(crate) static DISCARD_PROBE: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
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
    let folder = import_folder(world, transfer);
    let import = imports.join(&folder);
    let working = import.join("working");
    // CP23 (DECIDED, coordinator 2026-10-13): the import's lock (the
    // sibling file `<folder>.lock`) before anything of the import is
    // touched. CP24 (CP23 review MEDIUM-1 + LOW-3, DECIDED): the recipient
    // takes it before it marks the import inert and holds it to Activated
    // or quarantine, so the discard ALWAYS takes it (making the file if it
    // isn't there; it is removed at the end) and never touches the import
    // or its scratch without it. Held: skipped (another copy of the game,
    // or this one's own worker). Any other lock error: skipped too, left
    // for the next sweep. Only a file system with no file locking at all
    // goes on unlocked: no recipient can hold one there (it refuses).
    let lock = match super::world_lock::acquire_import(imports, &folder, true) {
        Ok(l) => Some(l),
        Err(super::world_lock::LockError::Unsupported) => None,
        Err(super::world_lock::LockError::HeldElsewhere) => {
            super::world_lock::log_skipped(&import, "the inert discard");
            return Err(Error::State("the import is in use by another copy of the game"));
        }
        Err(e) => {
            log::warn!("handover: {}'s import lock couldn't be taken ({e:?}); its discard is left for the next sweep", import.display());
            return Err(Error::Io);
        }
    };
    #[cfg(test)]
    DISCARD_PROBE.with(|p| {
        if let Some(probe) = p.borrow_mut().as_mut() {
            probe();
        }
    });
    // Re-read under the lock: whatever let go of it may have moved on.
    let exists = fs::symlink_metadata(&import).is_ok();
    if exists && super::world_lock::held_elsewhere(&working) {
        // An older copy of the game holding only the world's own lock.
        super::world_lock::log_skipped(&import, "the inert discard");
        return Err(Error::State("the import is in use by another copy of the game"));
    }
    let records = super::authority::dir(&working);
    if exists && fs::symlink_metadata(&records).is_ok() {
        // CP23 (CP22 review P-2): read and removed as it is; nothing made.
        MarkerStore::open_existing(records)?.discard_inert_ready(transfer)?;
    }
    if exists {
        remove_dir(&import)?;
    }
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
    clear(imports, transfer)?;
    // The lock file goes last, still held, once the import has gone.
    if let Some(lock) = lock {
        lock.remove_lock_file();
    }
    Ok(())
}

/// CP23: a lock file `handover-<w>-<t>.lock` whose import folder is gone
/// (its holder died between the two removals, or it was taken for an
/// import that never appeared) is removed once its lock is free; a held
/// one (a recipient about to rename its import into place) is left.
/// CP24 (CP23 review MEDIUM-2): on Unix only; elsewhere import lock files
/// are never deleted (`LiveLock::remove_lock_file`).
pub fn sweep_stale_locks(imports: &Path) -> usize {
    if cfg!(not(unix)) {
        return 0;
    }
    let Ok(entries) = fs::read_dir(imports) else { return 0 };
    let mut removed = 0;
    for entry in entries.take(SCAN_LIMIT).flatten() {
        let Some(folder) = entry.file_name().to_str().and_then(|n| n.strip_suffix(".lock")).filter(|f| f.starts_with("handover-")).map(str::to_owned)
        else {
            continue;
        };
        if fs::symlink_metadata(imports.join(&folder)).is_ok() {
            continue;
        }
        let Ok(lock) = super::world_lock::acquire_import(imports, &folder, false) else { continue };
        if fs::symlink_metadata(imports.join(&folder)).is_err() {
            lock.remove_lock_file();
            removed += 1;
        }
    }
    removed
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
    sweep_stale_locks(imports);
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

    /// CP23 (DECIDED): the import's lock is a sibling file. A discard of a
    /// marked import whose lock is held (here: this process's own worker)
    /// is skipped; once let go, the discard removes the import, then the
    /// lock file. A lock file whose import is gone is swept when free,
    /// never while held (a recipient about to rename its import in).
    #[test]
    fn handover_inert_import_lock_is_a_sibling_file_removed_after_its_import() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let imports = std::env::temp_dir().join(format!("axenstax-inert-sibling-{}", hex::encode(r)));
        let (t, w) = ([0x71; 16], [0x72; 16]);
        let folder = format!("handover-{}-{}", hex::encode(w), hex::encode(t));
        let import = imports.join(&folder);
        let working = import.join("working");
        std::fs::create_dir_all(working.join("chunks")).unwrap();
        std::fs::write(working.join("world.dat"), b"w").unwrap();
        mark(&imports, t, w).unwrap();
        let lock = super::super::world_lock::acquire_import(&imports, &folder, true).unwrap();
        let lock_file = imports.join(format!("{folder}.lock"));
        assert!(lock_file.is_file(), "the import's lock isn't the sibling file");
        assert!(discard(&imports, t).is_err(), "a held import's discard reported success");
        assert!(working.join("world.dat").exists(), "an import whose lock is held was discarded");
        drop(lock);
        discard(&imports, t).unwrap();
        assert!(!import.exists(), "the import outlived its discard");
        assert!(!lock_file.exists(), "the lock file outlived its import");
        let stale = "handover-aa-bb";
        let held = super::super::world_lock::acquire_import(&imports, stale, true).unwrap();
        assert_eq!(sweep_stale_locks(&imports), 0);
        assert!(imports.join(format!("{stale}.lock")).exists(), "a held lock file was swept");
        drop(held);
        assert_eq!(sweep_stale_locks(&imports), 1);
        assert!(!imports.join(format!("{stale}.lock")).exists(), "a free lock file with no import stayed");
        let _ = std::fs::remove_dir_all(&imports);
    }

    /// CP24 (CP23 review MEDIUM-1 + LOW-3, DECIDED): the discard always
    /// takes the import's lock, even with no lock file and no import yet
    /// (only a scratch). Another copy of the game's recipient trying to
    /// take that lock during the discard (to rename its import in) finds it
    /// held, and nothing a lock holder renamed in is ever deleted.
    #[test]
    fn handover_inert_discard_racing_a_just_renamed_import_deletes_nothing() {
        use std::{cell::RefCell, rc::Rc};
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let imports = std::env::temp_dir().join(format!("axenstax-inert-race-{}", hex::encode(r)));
        let (t, w) = ([0x81; 16], [0x82; 16]);
        let folder = import_folder(w, t);
        let import = imports.join(&folder);
        std::fs::create_dir_all(&imports).unwrap();
        mark(&imports, t, w).unwrap();
        let scratch = imports.join(format!(".handover-receive-{}-x", hex::encode(t)));
        std::fs::create_dir_all(scratch.join("chunks")).unwrap();
        let lock_file = imports.join(format!("{folder}.lock"));
        assert!(!lock_file.exists() && !import.exists());
        // The recipient: takes the lock if it can, then renames its import in.
        let got: Rc<RefCell<Option<super::super::world_lock::LiveLock>>> = Rc::default();
        let (probe_got, probe_imports, probe_folder, probe_import) = (got.clone(), imports.clone(), folder.clone(), import.clone());
        DISCARD_PROBE.with(|p| {
            *p.borrow_mut() = Some(Box::new(move || {
                if let Ok(l) = super::super::world_lock::acquire_import(&probe_imports, &probe_folder, true) {
                    std::fs::create_dir_all(probe_import.join("working")).unwrap();
                    std::fs::write(probe_import.join("working").join("world.dat"), b"w").unwrap();
                    *probe_got.borrow_mut() = Some(l);
                }
            }))
        });
        let r = discard(&imports, t);
        DISCARD_PROBE.with(|p| *p.borrow_mut() = None);
        if got.borrow().is_some() {
            assert!(
                import.join("working").join("world.dat").exists(),
                "the discard deleted an import renamed in by the holder of its lock"
            );
        }
        assert!(got.borrow().is_none(), "a recipient took the import's lock during its discard");
        r.unwrap();
        assert!(!scratch.exists(), "the scratch outlived its discard");
        assert!(!lock_file.exists(), "the discard left the lock file it made");
        assert!(marked(&imports, t).unwrap().is_none(), "the inert marker outlived its discard");
        let _ = std::fs::remove_dir_all(&imports);
    }

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

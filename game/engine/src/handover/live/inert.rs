//! P3: a recipient's import is inert, and this machine's to discard, until
//! Prepared. At import start the session writes `inert-<transfer>.dat` next
//! to the import (in `<data>/handover/imports`); Prepared clears it. An abort
//! or timeout before Prepared removes ONLY that transfer's import (its final
//! `handover-<world>-<transfer>` directory and any
//! `.handover-receive-<transfer>-*` scratch), its Ready marker, then the inert
//! marker itself. After Prepared nothing here deletes: a failure quarantines,
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
    // Windows: no directory fsync (see `durable::sync_directory`). Not compiled here.
    #[cfg(windows)]
    {
        let _ = dir;
        Ok(())
    }
    #[cfg(not(windows))]
    fs::File::open(dir).and_then(|f| f.sync_all()).map_err(|_| Error::Io)
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
    let mut f = opts.open(&temp).map_err(|_| Error::Io)?;
    f.write_all(&record(transfer, world)).and_then(|()| f.sync_all()).map_err(|_| Error::Io)?;
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
/// transfer refuses (that import is no longer inert).
pub fn discard(imports: &Path, markers: &MarkerStore, transfer: Id) -> Result<()> {
    let Some(world) = marked(imports, transfer)? else { return Ok(()) };
    markers.discard_inert_ready(transfer)?;
    remove_dir(&imports.join(format!("handover-{}-{}", hex::encode(world), hex::encode(transfer))))?;
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
pub fn sweep(imports: &Path, markers: &MarkerStore, live: Option<Id>) -> Result<usize> {
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
        match discard(imports, markers, t) {
            Ok(()) => removed += 1,
            Err(e) => first_err = first_err.or(Some(e)),
        }
    }
    match first_err {
        Some(e) if removed == 0 => Err(e),
        _ => Ok(removed),
    }
}

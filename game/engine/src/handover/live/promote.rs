//! CP18 (DECIDED, coordinator 2026-10-12): the recipient opens the
//! handed-over world as its own.
//!
//! A handover's import is received into
//! `<data>/handover/imports/handover-<world>-<transfer>/working`, which the
//! world list never scans. Once the import is Activated (and only then),
//! [`promote_activated_imports`] moves that `working` folder into the worlds
//! folder with one `rename`, so it gets an ordinary card and Play, Host and
//! Fork work on it exactly as on a world made here.
//!
//! - Main thread only (boot and every lobby refresh), never the resumer's
//!   thread: the live recipient session and the resumer's wait read the
//!   import at its old path, and both hold the import's handover lock
//!   (`world_lock`) until they have seen Activated. An import whose lock is
//!   held here or by another process is left for the next refresh.
//! - Idempotent. The rename is atomic, so the import's `working` is either
//!   still there or already moved; every import scan reads
//!   `working/handover/`, so a moved import vanishes from all of them. An
//!   import whose Activated transfer already has a world in the worlds
//!   folder is never moved a second time.
//! - The moved world keeps its `handover/` with the Activated record (the
//!   open guard passes it; fork and export leave `handover/` behind).
//! - A worlds folder on a different drive (`AXENSTAX_WORLDS_DIR` elsewhere:
//!   the rename fails with `CrossesDevices`): no copy path. The import stays
//!   where it is and the lobby says why, with what to change.
//! - CP19 (DECIDED, coordinator 2026-10-12): once the move is durable (the
//!   rename, then both parent folders fsynced), the rest of the import
//!   folder (`received/`, the archive, the manifest and admission) is
//!   removed: the moved world keeps its own `handover/` with the Activated
//!   record and the install ledger lives elsewhere, so nothing there is
//!   needed. A crash between the move and the removal leaves an import
//!   folder with no `working`; the next pass's leftover sweep finds a world
//!   in the worlds folder carrying that import's Activated transfer, fsyncs
//!   both folders and removes it. An import folder is removed only then:
//!   never one that isn't Activated and moved (an import still receiving
//!   has no `working` either, and no world carries its transfer).
use crate::handover::Id;
use std::path::{Path, PathBuf};

/// What one promotion pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Promoted {
    /// Folder names (under the worlds folder) of the worlds moved in.
    pub moved: Vec<String>,
    /// Plain lobby lines for imports that couldn't be moved (and why).
    pub lines: Vec<String>,
}

/// The rename boundary (injectable so a test can force a different drive).
pub type Rename = fn(&Path, &Path) -> std::io::Result<()>;
/// The directory fsync (injectable so a test can make the move not durable).
pub type SyncDir = fn(&Path) -> std::io::Result<()>;

/// Move every Activated, unquarantined, unheld import under `imports` into
/// `worlds_root`. See the module docs.
pub fn promote_activated_imports(imports: &Path, worlds_root: &Path) -> Promoted {
    promote_with(imports, worlds_root, |from, to| std::fs::rename(from, to))
}

pub fn promote_with(imports: &Path, worlds_root: &Path, rename: Rename) -> Promoted {
    promote_with_io(imports, worlds_root, rename, crate::fsync::sync_dir)
}

pub fn promote_with_io(imports: &Path, worlds_root: &Path, rename: Rename, sync: SyncDir) -> Promoted {
    let mut out = Promoted::default();
    remove_moved_leftovers(imports, worlds_root, sync);
    let mut seen = std::collections::BTreeSet::new();
    for (working, transfer) in super::resumer::import_records(imports, "activated-") {
        if !seen.insert(working.clone()) {
            continue;
        }
        match promote_one(&working, transfer, worlds_root, rename, sync) {
            Ok(Some(folder)) => out.moved.push(folder),
            Ok(None) => {}
            Err(line) => out.lines.push(line),
        }
    }
    out
}

/// `Ok(Some(folder))` moved; `Ok(None)` not (yet) movable, try again later;
/// `Err(line)` stays put with a line for the lobby.
fn promote_one(working: &Path, transfer: Id, worlds_root: &Path, rename: Rename, sync: SyncDir) -> Result<Option<String>, String> {
    let records = super::authority::dir(working);
    if std::fs::symlink_metadata(records.join(format!("quarantined-{}.dat", hex::encode(transfer)))).is_ok() {
        return Ok(None);
    }
    // The session or the resumer's wait in this process, or another game
    // process, still works on the import at this path.
    if super::world_lock::held_here(working) || super::world_lock::held_elsewhere(working) {
        return Ok(None);
    }
    // Every transfer it holds is Activated, every Activated record parses
    // and binds its receipt, and nothing is quarantined.
    if super::authority::open_guard(working).is_err() {
        if super::resumer::log_due("promote: refused", transfer) {
            log::warn!("handover: an activated import under {} is refused by the open guard; it isn't moved", working.display());
        }
        return Ok(None);
    }
    if let Some(world) = already_moved(worlds_root, transfer) {
        if super::resumer::log_due("promote: already moved", transfer) {
            log::warn!(
                "handover: an activated import is already in the worlds folder as {}; the copy under {} is left alone",
                world.display(),
                working.display()
            );
        }
        return Ok(None);
    }
    let name = display_name(working);
    let folder = free_folder(worlds_root, &name);
    let target = worlds_root.join(&folder);
    if std::fs::create_dir_all(worlds_root).is_err() {
        return Ok(None);
    }
    match rename(working, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            if super::resumer::log_due("promote: other drive", transfer) {
                log::warn!("handover: {} and {} are on different drives; the world isn't moved", working.display(), worlds_root.display());
            }
            return Err(super::copy::import_on_other_drive(&name));
        }
        Err(e) => {
            if super::resumer::log_due("promote: rename", transfer) {
                log::warn!("handover: the handed-over world couldn't be moved into the worlds folder ({e}); it is tried again");
            }
            return Ok(None);
        }
    }
    log::info!("handover: the handed-over world is now in the worlds folder as {folder}");
    if let Some(import) = working.parent() {
        remove_once_durable(import, worlds_root, sync);
    }
    Ok(Some(folder))
}

/// CP19: fsync the import folder (the rename's source parent) and the
/// worlds folder (its target parent); only when both succeed is the move
/// durable, and only then is the rest of the import folder removed (and
/// `imports` fsynced). A failed sync leaves it for the next pass's leftover
/// sweep, which syncs again first.
fn remove_once_durable(import: &Path, worlds_root: &Path, sync: SyncDir) -> bool {
    for dir in [import, worlds_root] {
        if let Err(e) = sync(dir) {
            log::warn!("handover: syncing {} after moving the world in failed ({e}); the import folder is kept for now", dir.display());
            return false;
        }
    }
    if let Err(e) = std::fs::remove_dir_all(import) {
        log::warn!("handover: removing the moved import's folder {} failed ({e}); it is tried again", import.display());
        return false;
    }
    if let Some(imports) = import.parent() {
        let _ = sync(imports);
    }
    true
}

/// CP19: the boot pass's already-moved branch for a crash between the move
/// and the removal. An import folder (`handover-<world>-<transfer>`) with no
/// `working` whose transfer's Activated record a world in the worlds folder
/// carries (and that world passes the open guard) is a moved import's
/// leftovers: removed once durable. Anything else is left alone.
fn remove_moved_leftovers(imports: &Path, worlds_root: &Path, sync: SyncDir) {
    let Ok(entries) = std::fs::read_dir(imports) else { return };
    for entry in entries.take(super::inert::SCAN_LIMIT).flatten() {
        let import = entry.path();
        let Some(transfer) = entry.file_name().to_str().and_then(import_transfer) else { continue };
        if !std::fs::symlink_metadata(&import).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        if !matches!(std::fs::symlink_metadata(import.join("working")), Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
            continue;
        }
        let Some(world) = already_moved(worlds_root, transfer) else { continue };
        if super::authority::open_guard(&world).is_err() {
            continue;
        }
        if remove_once_durable(&import, worlds_root, sync) {
            log::info!("handover: removed the leftovers of an import already moved in as {}", world.display());
        }
    }
}

/// The transfer id from an import folder name `handover-<world>-<transfer>`.
fn import_transfer(name: &str) -> Option<Id> {
    let rest = name.strip_prefix("handover-")?;
    let (_, t) = rest.rsplit_once('-')?;
    Id::try_from(hex::decode(t).ok()?).ok()
}

/// A world under `worlds_root` already carrying `transfer`'s Activated
/// record (bounded; read-only).
fn already_moved(worlds_root: &Path, transfer: Id) -> Option<PathBuf> {
    let name = format!("activated-{}.dat", hex::encode(transfer));
    let entries = std::fs::read_dir(worlds_root).ok()?;
    entries
        .take(super::inert::SCAN_LIMIT)
        .flatten()
        .map(|e| e.path())
        .find(|w| std::fs::symlink_metadata(super::authority::dir(w).join(&name)).is_ok())
}

/// The world's display name from its `world_meta.json` (bounded read), or
/// "world".
fn display_name(working: &Path) -> String {
    use std::io::Read;
    const LIMIT: u64 = 2 * 1024 * 1024;
    let mut bytes = Vec::new();
    let read = std::fs::File::open(working.join("world_meta.json")).and_then(|f| f.take(LIMIT + 1).read_to_end(&mut bytes));
    if read.is_err() || bytes.len() as u64 > LIMIT {
        return "world".to_owned();
    }
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("display_name")?.as_str().map(str::to_owned))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "world".to_owned())
}

/// The lobby's new-world folder naming (`save::slugify_folder_name`) with
/// its `_2`, `_3`, … collision rule, applied against `worlds_root` (not the
/// global root) and to the "world" fallback too, which
/// `save::sanitize_folder_name` returns without a collision check.
fn free_folder(worlds_root: &Path, display: &str) -> String {
    let base = crate::save::slugify_folder_name(display);
    let free = |c: &str| matches!(std::fs::symlink_metadata(worlds_root.join(c)), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
    if free(&base) {
        return base;
    }
    (2..100u32)
        .map(|i| format!("{base}_{i}"))
        .find(|c| free(c))
        .unwrap_or_else(|| format!("{base}_{}", std::time::UNIX_EPOCH.elapsed().unwrap_or_default().as_secs()))
}

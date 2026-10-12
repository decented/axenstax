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
//! - The import's other contents (`received/`, the archive) stay where they
//!   are; the import folder itself is never listed (no `world.dat` at its
//!   top level).
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

/// Move every Activated, unquarantined, unheld import under `imports` into
/// `worlds_root`. See the module docs.
pub fn promote_activated_imports(imports: &Path, worlds_root: &Path) -> Promoted {
    promote_with(imports, worlds_root, |from, to| std::fs::rename(from, to))
}

pub fn promote_with(imports: &Path, worlds_root: &Path, rename: Rename) -> Promoted {
    let mut out = Promoted::default();
    let mut seen = std::collections::BTreeSet::new();
    for (working, transfer) in super::resumer::import_records(imports, "activated-") {
        if !seen.insert(working.clone()) {
            continue;
        }
        match promote_one(&working, transfer, worlds_root, rename) {
            Ok(Some(folder)) => out.moved.push(folder),
            Ok(None) => {}
            Err(line) => out.lines.push(line),
        }
    }
    out
}

/// `Ok(Some(folder))` moved; `Ok(None)` not (yet) movable, try again later;
/// `Err(line)` stays put with a line for the lobby.
fn promote_one(working: &Path, transfer: Id, worlds_root: &Path, rename: Rename) -> Result<Option<String>, String> {
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
    for dir in [working.parent(), Some(worlds_root)].into_iter().flatten() {
        if let Err(e) = crate::fsync::sync_dir(dir) {
            log::warn!("handover: syncing {} after moving the world in failed ({e})", dir.display());
        }
    }
    log::info!("handover: the handed-over world is now in the worlds folder as {folder}");
    Ok(Some(folder))
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

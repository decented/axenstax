//! Joiner sidecars published with the `world.dat` they belong to (Spec 02 §8.2.1).
//!
//! A sidecar holds what a joiner carries and `world.dat` holds the chests. A
//! world save that moved them separately could be cut short between the two,
//! and the world would reopen with an item in both places or in neither.
//!
//! 1. [`stage`]: this save's sidecars into `joiners.new/`, fsynced, with a
//!    marker naming the SHA-256 of the `world.dat` about to commit.
//! 2. The caller commits `world.dat`, as it always has.
//! 3. [`settle`]: rename the staged sidecars into `joiners/`, then remove
//!    `joiners.new/`.
//!
//! [`settle`] also runs before any sidecar is read. A marker that matches the
//! `world.dat` on disk means the commit happened, so the publish is finished;
//! anything else means it didn't, so the stage is dropped. Both can be cut short
//! and run again.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write as _;
use std::path::Path;

/// Where a save stages its joiners' sidecars, beside `joiners/`.
pub(crate) const STAGED_JOINERS: &str = "joiners.new";
/// Names the commit a stage belongs to: the SHA-256, in hex, of that `world.dat`.
/// Written last, so a stage with a marker is whole.
const MARKER: &str = "world-dat.sha256";

/// Which commit a stage may belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Commit {
    /// `world_open::open_world` opened `world.dat`: a stage for the autosave is
    /// for a commit that isn't the world now open.
    WorldDat,
    /// `open_world` opened `autosave/world.dat`.
    Autosave,
    /// A running session, whose last publish failed: whichever it committed.
    Either,
}

/// One sidecar for [`stage`]: its file name in `joiners/`, and its bytes.
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

/// Test hook: where a save stops on this thread, as if the process were killed.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    /// Staged, before anything of the commit.
    BeforeCommit,
    /// `world.dat` committed, nothing published.
    AfterCommit,
    /// This many sidecars published.
    DuringPublish(usize),
    /// Every sidecar published, the staging directory still there.
    BeforeRemoval,
}

#[cfg(test)]
thread_local! {
    pub(crate) static CUT: std::cell::Cell<Option<Cut>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn cut(at: Cut) -> Result<(), String> {
    if CUT.with(|c| c.get()) == Some(at) { Err(format!("save cut short at {at:?} (test)")) } else { Ok(()) }
}

/// Stage `rows` for the commit of `committing` (the exact `world.dat` bytes), and
/// make that durable before the commit can be. A stage already there is dropped:
/// a save settles one before it starts, so it can only be its own failed attempt.
pub(crate) fn stage(world: &Path, rows: &[Row], committing: &[u8]) -> Result<(), String> {
    let staged = write_stage(world, rows, committing);
    if staged.is_err() {
        let _ = remove_staging(world);
    }
    staged?;
    #[cfg(test)]
    cut(Cut::BeforeCommit)?;
    Ok(())
}

fn write_stage(world: &Path, rows: &[Row], committing: &[u8]) -> Result<(), String> {
    let staging = world.join(STAGED_JOINERS);
    crate::save_fence::check(&staging)?;
    remove_staging(world)?;
    fs::create_dir_all(world).map_err(|e| format!("mkdir {}: {e}", world.display()))?;
    create_private_dir(&staging)?;
    for row in rows {
        let path = staging.join(&row.name);
        let mut options = fs::OpenOptions::new();
        options.create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = crate::fsync::open_written(&mut options, &path).map_err(|e| format!("stage {}: {e}", path.display()))?;
        file.write_all(&row.bytes).map_err(|e| format!("stage {}: {e}", path.display()))?;
        crate::fsync::sync_written(&file).map_err(|e| format!("fsync {}: {e}", path.display()))?;
    }
    sync_dir(&staging)?;
    // Last: a stage with a marker is a whole stage.
    crate::save::write_atomic(&staging.join(MARKER), hex::encode(sha256(committing)).as_bytes())?;
    sync_dir(&staging)?;
    sync_dir(world)
}

/// Finish or drop whatever stage is beside `world`'s sidecars. See the module
/// comment. `note` hears each sidecar as it lands in `joiners/`, so what a server
/// says of its records never runs ahead of the disk (`StoredRecords::note_joiner`).
///
/// `Err` leaves the stage for the next call, except that a stage this can't
/// read a commit file to judge is left untouched: a read that fails isn't a
/// commit that never happened.
pub(crate) fn settle(world: &Path, commit: Commit, note: &mut dyn FnMut(&Path, &[u8])) -> Result<(), String> {
    let staging = world.join(STAGED_JOINERS);
    match fs::symlink_metadata(&staging) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{STAGED_JOINERS}/ can't be checked ({e})")),
        Ok(_) => {}
    }
    // M-1: never written into a retired world.
    crate::handover::live::authority::open_guard(world).map_err(|r| r.0.to_string())?;
    if is_committed(world, &staging, commit)? {
        log::warn!("{}: publishing joiners' records saved with this world.dat", world.display());
        publish(world, &staging, note)
    } else {
        log::warn!("{}: dropping joiners' records staged for a save that never committed", world.display());
        remove_staging(world)
    }
}

fn is_committed(world: &Path, staging: &Path, commit: Commit) -> Result<bool, String> {
    let Some(marker) = read_marker(staging)? else { return Ok(false) };
    let files: &[&str] = match commit {
        Commit::WorldDat => &["world.dat"],
        Commit::Autosave => &["autosave/world.dat"],
        Commit::Either => &["world.dat", "autosave/world.dat"],
    };
    for file in files {
        match fs::read(world.join(file)) {
            Ok(bytes) if sha256(&bytes) == marker => return Ok(true),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{file} can't be read to settle {STAGED_JOINERS}/ ({e})")),
        }
    }
    Ok(false)
}

/// The digest the marker names. `None`: no marker, or not one, so a stage cut
/// short before its marker landed.
fn read_marker(staging: &Path) -> Result<Option<[u8; 32]>, String> {
    let file = match fs::File::open(staging.join(MARKER)) {
        Ok(file) => file,
        // Not a directory: a stray file or link where the stage would be.
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => return Ok(None),
        Err(e) => return Err(format!("{STAGED_JOINERS}/{MARKER} can't be read ({e})")),
    };
    let Ok(bytes) = crate::save::read_bounded(file, 64) else { return Ok(None) };
    Ok(hex::decode(bytes).ok().and_then(|digest| digest.try_into().ok()))
}

fn publish(world: &Path, staging: &Path, note: &mut dyn FnMut(&Path, &[u8])) -> Result<(), String> {
    #[cfg(test)]
    cut(Cut::AfterCommit)?;
    let joiners = world.join("joiners");
    crate::handover::live::personal_adapter::ensure_directory(&joiners)?;
    let entries = fs::read_dir(staging).map_err(|e| format!("{STAGED_JOINERS}/ can't be listed ({e})"))?;
    let mut published = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{STAGED_JOINERS}/ can't be listed ({e})"))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str().filter(|name| is_sidecar_name(name)) else { continue };
        let path = entry.path();
        let regular = fs::symlink_metadata(&path).is_ok_and(|m| m.is_file());
        if !regular {
            log::warn!("{}: not a joiner's record, not published", path.display());
            continue;
        }
        let file = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = crate::save::read_bounded(file, crate::joiner_save::MAX_JOINER_SAVE_BYTES)?;
        let target = joiners.join(name);
        crate::save_fence::commit(&target, || {
            fs::rename(&path, &target).map_err(|e| format!("publish {name}: {e}"))
        })?;
        note(&target, &bytes);
        published += 1;
        #[cfg(test)]
        cut(Cut::DuringPublish(published))?;
    }
    sync_dir(&joiners)?;
    sync_dir(world)?;
    log::debug!("{}: {published} joiner record(s) published", world.display());
    #[cfg(test)]
    cut(Cut::BeforeRemoval)?;
    remove_staging(world)
}

/// Remove whatever is at the stage's path, without following a link. Every
/// sidecar has left a stage that was published, so what goes here is what was
/// never committed.
fn remove_staging(world: &Path) -> Result<(), String> {
    let staging = world.join(STAGED_JOINERS);
    let metadata = match fs::symlink_metadata(&staging) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{STAGED_JOINERS}/ can't be checked ({e})")),
    };
    crate::save_fence::commit(&staging, || {
        if metadata.is_dir() { fs::remove_dir_all(&staging) } else { fs::remove_file(&staging) }
            .map_err(|e| format!("{STAGED_JOINERS}/ can't be removed ({e})"))
    })?;
    sync_dir(world)
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|e| format!("mkdir {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

fn sync_dir(path: &Path) -> Result<(), String> {
    crate::fsync::sync_dir(path).map_err(|e| format!("fsync {}: {e}", path.display()))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The file names `joiners/` holds (`joiner_save`'s `joiner_path`).
fn is_sidecar_name(name: &str) -> bool {
    name.starts_with("npub1") && name.ends_with(".dat")
}

#[cfg(test)]
#[path = "joiner_stage_tests.rs"]
mod tests;

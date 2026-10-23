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
//! - CP20 (CP18 review MEDIUM-1, DECIDED): every recipient import that
//!   reaches Activated (the live session or the resumer's wait, any thread)
//!   bumps [`activations`]; the lobby compares it each frame and, when it
//!   moves, promotes and re-lists, retrying while a pass reports an import
//!   still [`Promoted::held`] (the holder lets go of the import's lock a
//!   moment after Activated). Promotion also runs on lobby entry.
use crate::handover::Id;
use std::path::{Path, PathBuf};

static ACTIVATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    /// Tests: the activations noted on this thread (the process-wide count
    /// also moves with every other test's activations).
    static THREAD_ACTIVATIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// CP20: a recipient import reached Activated. CP22 (CP21 review MEDIUM-1,
/// DECIDED): called only on `Receiver::activate`'s real transition, never
/// when a restart or a status read finds an import already Activated (boot
/// and lobby entry promote those anyway), so the lobby's pass runs once per
/// activation, not every frame.
pub fn note_activated() {
    ACTIVATIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    #[cfg(test)]
    THREAD_ACTIVATIONS.with(|c| c.set(c.get() + 1));
}

/// CP20: how many imports have reached Activated in this process.
pub fn activations() -> u64 {
    ACTIVATIONS.load(std::sync::atomic::Ordering::SeqCst)
}

/// Tests: `activations` as noted on this thread only.
#[cfg(test)]
pub fn activations_on_this_thread() -> u64 {
    THREAD_ACTIVATIONS.with(|c| c.get())
}

/// CP22 (CP21 review MEDIUM-1, DECIDED): when the lobby runs a promotion
/// pass. Due once per new activation, then again on the 500 ms cadence only
/// while the last pass found an Activated import whose lock is still held
/// (its holder lets go a moment after Activated). An import that stays
/// unmoved for any other reason (a twin, another drive, a failed move)
/// costs nothing per frame: it is tried again at the next lobby entry,
/// refresh or activation.
#[derive(Debug)]
pub struct LobbyPoll {
    seen: u64,
    retry: Option<std::time::Instant>,
}

impl LobbyPoll {
    /// The retry cadence while an import's lock is held.
    pub const RETRY: std::time::Duration = std::time::Duration::from_millis(500);

    /// `seen`: the activation count read before the lobby's entry pass;
    /// `held`: that pass found a held import.
    pub fn new(seen: u64, held: bool, now: std::time::Instant) -> Self {
        Self { seen, retry: held.then_some(now) }
    }

    /// Whether a pass is due now, given the current activation count.
    pub fn due(&mut self, activations: u64, now: std::time::Instant) -> bool {
        if activations != self.seen {
            self.seen = activations;
            return true;
        }
        self.retry.is_some_and(|t| now.duration_since(t) >= Self::RETRY)
    }

    /// Record a pass's outcome. Returns whether the previous pass found a
    /// held import and this one found none: CP24 (CP22 review LOW-1) the
    /// lobby re-lists then, since another copy of the game may have moved
    /// the import in meanwhile (this pass then moves nothing and no waiting
    /// line changes).
    pub fn ran(&mut self, promoted: &Promoted, now: std::time::Instant) -> bool {
        let was_held = self.retry.is_some();
        self.retry = promoted.held.then_some(now);
        was_held && !promoted.held
    }
}

/// What one promotion pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Promoted {
    /// Folder names (under the worlds folder) of the worlds moved in.
    pub moved: Vec<String>,
    /// Plain lobby lines for imports that couldn't be moved (and why).
    pub lines: Vec<String>,
    /// CP20: an Activated import was skipped because its lock is still
    /// held (here or by another process); a later pass may move it.
    pub held: bool,
}

/// One import's outcome.
enum Step {
    Moved(String),
    /// Its lock is held; try again shortly.
    Held,
    /// Nothing to do (quarantined, or a hand-made twin of a moved world).
    Skip,
    /// Stays put; the lobby shows this line.
    Line(String),
}

/// The rename boundary (injectable so a test can force a different drive).
pub type Rename = fn(&Path, &Path) -> std::io::Result<()>;
/// The directory fsync (injectable so a test can make the move not durable).
pub type SyncDir = fn(&Path) -> std::io::Result<()>;

/// Move every Activated, unquarantined, unheld import under `imports` into
/// `worlds_root`. See the module docs.
pub fn promote_activated_imports(imports: &Path, worlds_root: &Path) -> Promoted {
    promote_with(imports, worlds_root, rename_no_replace)
}

/// CP19 (CP18 review MEDIUM-3 / LOW-1): the move never replaces a folder
/// at the target, even an empty one someone else just made there. CP21:
/// the platform rules and the fallback for a filesystem without the
/// no-replace flag live in `crate::handover::rename`.
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    crate::handover::rename::no_replace(from, to)
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
            Step::Moved(folder) => out.moved.push(folder),
            Step::Held => out.held = true,
            Step::Skip => {}
            Step::Line(line) => out.lines.push(line),
        }
    }
    out
}

/// CP20 (CP18 review LOW-2): an Activated import that stays put says why
/// in the lobby (a damaged record, or a move that failed and is retried),
/// except a hand-made twin of a world already moved in (step 3, DECIDED:
/// logged once, never moved twice, never deleted) and a quarantined import
/// (the resumer's notices carry that).
fn promote_one(working: &Path, transfer: Id, worlds_root: &Path, rename: Rename, sync: SyncDir) -> Step {
    let records = super::authority::dir(working);
    if std::fs::symlink_metadata(records.join(format!("quarantined-{}.dat", hex::encode(transfer)))).is_ok() {
        return Step::Skip;
    }
    // The session or the resumer's wait in this process, or another game
    // process, still works on the import at this path.
    if super::world_lock::held_here(working) || super::world_lock::held_elsewhere(working) {
        return Step::Held;
    }
    // CP23: and the import's own lock (the sibling `<import>.lock`, which
    // the session holds to Activated): held anywhere, this process too, is
    // held. Taken while the world moves out; its file goes after the
    // import folder. No lock file (an import from before it existed) or no
    // file locking: as before it.
    let import_lock = match import_lock_of(working) {
        Some(Err(super::world_lock::LockError::HeldElsewhere)) => return Step::Held,
        Some(Ok(l)) => Some(l),
        _ => None,
    };
    // Every transfer it holds is Activated, every Activated record parses
    // and binds its receipt, and nothing is quarantined.
    if super::authority::open_guard(working).is_err() {
        if super::resumer::log_due("promote: refused", transfer) {
            log::warn!("handover: an activated import under {} is refused by the open guard; it isn't moved", working.display());
        }
        return Step::Line(super::copy::import_record_damaged(&display_name(working)));
    }
    if let Some(world) = already_moved(worlds_root, transfer) {
        if super::resumer::log_due("promote: already moved", transfer) {
            log::warn!(
                "handover: an activated import is already in the worlds folder as {}; the copy under {} is left alone",
                world.display(),
                working.display()
            );
        }
        return Step::Skip;
    }
    let name = display_name(working);
    let folder = free_folder(worlds_root, &name);
    let target = worlds_root.join(&folder);
    if std::fs::create_dir_all(worlds_root).is_err() {
        return Step::Line(super::copy::import_not_moved_yet(&name));
    }
    match rename(working, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            if super::resumer::log_due("promote: other drive", transfer) {
                log::warn!("handover: {} and {} are on different drives; the world isn't moved", working.display(), worlds_root.display());
            }
            return Step::Line(super::copy::import_on_other_drive(&name));
        }
        Err(e) => {
            if super::resumer::log_due("promote: rename", transfer) {
                log::warn!("handover: the handed-over world couldn't be moved into the worlds folder ({e}); it is tried again");
            }
            return Step::Line(super::copy::import_not_moved_yet(&name));
        }
    }
    log::info!("handover: the handed-over world is now in the worlds folder as {folder}");
    if let Some(import) = working.parent()
        && remove_once_durable(import, worlds_root, sync)
        && let Some(l) = &import_lock
    {
        l.remove_lock_file();
    }
    Step::Moved(folder)
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
        // CP23: under the import's lock (held: skipped); its file after.
        let lock = match import_lock_of(&import.join("working")) {
            Some(Err(super::world_lock::LockError::HeldElsewhere)) => continue,
            Some(Ok(l)) => Some(l),
            _ => None,
        };
        if remove_once_durable(&import, worlds_root, sync) {
            log::info!("handover: removed the leftovers of an import already moved in as {}", world.display());
            if let Some(l) = &lock {
                l.remove_lock_file();
            }
        }
    }
}

/// CP23: the import lock of the import folder holding `working` (never
/// creating its file); `None` when the path has no import folder parent.
fn import_lock_of(working: &Path) -> Option<Result<super::world_lock::LiveLock, super::world_lock::LockError>> {
    let import = working.parent()?;
    let folder = import.file_name()?.to_str()?;
    // CP24 (CP23 review LOW-1): one try; the lobby's main thread never
    // waits for another process's lock (held: the 500 ms held retry).
    Some(super::world_lock::try_import_once(import.parent()?, folder))
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
///
/// CP19 (CP18 review MEDIUM-3, LOW-1): a reserved name
/// (`save::is_reserved_folder`: the Workshop, the joined-world folder,
/// Windows device names) counts as taken, and the fallback past `_99` is
/// checked too (time-stamped, then counted).
fn free_folder(worlds_root: &Path, display: &str) -> String {
    let base = crate::save::off_arena_folder(crate::save::slugify_folder_name(display));
    let free = |c: &str| !crate::save::is_reserved_folder(c) && matches!(std::fs::symlink_metadata(worlds_root.join(c)), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
    if free(&base) {
        return base;
    }
    let stamp = std::time::UNIX_EPOCH.elapsed().unwrap_or_default().as_secs();
    (2..100u64)
        .map(|i| format!("{base}_{i}"))
        .chain((0..1000u64).map(|i| format!("{base}_{}", stamp + i)))
        .find(|c| free(c))
        .unwrap_or_else(|| format!("world_{stamp}_{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    /// CP24 (CP22 review LOW-1 + LOW-4): the lobby's poll. An activation
    /// (on another thread) makes a pass due on the next frame; while a pass
    /// finds a held import, passes repeat no sooner than every 500 ms; the
    /// first pass that finds none after a held one asks for a re-list.
    #[test]
    fn handover_promote_lobby_poll_relists_once_a_held_import_is_let_go() {
        use super::{LobbyPoll, Promoted};
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut poll = LobbyPoll::new(7, false, start);
        assert!(!poll.due(7, start), "a pass ran with nothing new");
        let seen = std::thread::spawn(|| 8u64).join().unwrap();
        assert!(poll.due(seen, start), "another thread's activation wasn't seen on the next frame");
        let held = Promoted { held: true, ..Default::default() };
        assert!(!poll.ran(&held, start), "a held pass asked for a re-list");
        assert!(!poll.due(8, start + Duration::from_millis(499)), "the held retry came before 500 ms");
        assert!(poll.due(8, start + LobbyPoll::RETRY), "the held retry never came");
        assert!(!poll.ran(&held, start + LobbyPoll::RETRY), "a second held pass asked for a re-list");
        assert!(
            poll.ran(&Promoted::default(), start + LobbyPoll::RETRY * 2),
            "the pass after a held import was let go (moved elsewhere) didn't ask for a re-list"
        );
        assert!(!poll.due(8, start + LobbyPoll::RETRY * 4), "a pass ran with nothing held or new");
        assert!(!poll.ran(&Promoted::default(), start + LobbyPoll::RETRY * 4), "an idle pass asked for a re-list");
    }

    fn root(tag: &str) -> crate::handover::durable::tests::TestDir {
        let d = crate::handover::durable::tests::TestDir::random(&format!("axenstax-promote-{tag}"));
        std::fs::create_dir_all(&*d).unwrap();
        d
    }

    /// CP19 (CP18 review MEDIUM-3): reserved names are taken even when the
    /// folder doesn't exist, so "The Workshop" never lands on the Workshop
    /// and "CON"/"Lpt1.txt" never on a Windows device name.
    #[test]
    fn handover_promote_never_names_a_reserved_folder() {
        let d = root("reserved");
        for (display, want) in [("The Workshop", "the_workshop_2"), ("the workshop!", "the_workshop_2"), ("CON", "con_2"), ("Lpt1", "lpt1_2"), ("Remote Game", "remote_game_2"), ("Comet", "comet")] {
            assert_eq!(super::free_folder(&d, display), want, "{display}");
        }
        assert!(crate::save::is_reserved_folder("nul.txt") && crate::save::is_reserved_folder("COM9") && !crate::save::is_reserved_folder("com10"));
        assert!(!d.join("the_workshop").exists(), "the Workshop folder was made");
        // CP21 (CP19 review HIGH-1): never a Trial arena's folder, any case;
        // the world still gets a readable, non-`exp-` folder.
        for (display, want) in [("Exp-Hash-Dash", "exp_hash-dash"), ("exp-satori-rush", "exp_satori-rush"), ("Expedition", "expedition")] {
            let got = super::free_folder(&d, display);
            assert_eq!(got, want, "{display}");
            assert!(!crate::save::is_arena_folder(&got), "{display} landed on a Trial arena folder: {got}");
        }
        assert!(crate::save::is_reserved_folder("EXP-hash-dash") && crate::save::is_reserved_folder("exp-hash-dash-0000beef"));
    }

    /// CP19 (CP18 review LOW-1): the move never replaces an existing folder
    /// at the target, even an empty one.
    #[test]
    fn handover_promote_rename_never_replaces_a_folder() {
        let d = root("noreplace");
        std::fs::create_dir_all(d.join("from")).unwrap();
        std::fs::write(d.join("from").join("world.dat"), b"w").unwrap();
        std::fs::create_dir_all(d.join("to")).unwrap();
        assert!(super::rename_no_replace(&d.join("from"), &d.join("to")).is_err(), "the move replaced an empty folder");
        assert!(d.join("from").join("world.dat").exists());
        super::rename_no_replace(&d.join("from"), &d.join("free")).unwrap();
        assert!(d.join("free").join("world.dat").exists());
    }
}

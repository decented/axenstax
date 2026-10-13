//! CP17 (CP15 review LOW-3, DECIDED): the per-world handover lock.
//!
//! A live handover holds an exclusive advisory lock on `handover/.live` in
//! the world's folder (the source: from before its pending abort record is
//! written until its terminal record is durable; the recipient: from before
//! its Prepared receipt is written until the import is Activated or
//! quarantined). Another game process on the same computer then:
//! - skips that world at start-up (no pending-abort promotion, no
//!   publication, no wait taken over), logging it once;
//! - can't open, host, fork or write it (`authority::open_guard` refuses
//!   with `copy::HANDOVER_ELSEWHERE`), and can't start a handover of it.
//!
//! The lock is the standard library's (`File::try_lock`: `flock` on Unix,
//! `LockFileEx` on Windows). The OS drops it when the handle closes or the
//! process dies, so a crash never leaves a stale lock. `.live` itself is
//! never deleted (deleting a lock file under a holder lets two processes
//! lock two different files); an empty `.live` with no holder is normal and
//! counts as one entry toward the folder's 256-entry bound.
//!
//! Within one process the lock is shared: `acquire` hands back the same
//! guard (an `Arc`) while any holder in this process keeps it, so the
//! session and the resumer job that takes over its wait hold it together,
//! and the OS lock is released when the last of them drops. The OS lock
//! can't tell this process from another (a second open of the file in the
//! same process conflicts too), so `held_elsewhere` first looks at this
//! process's own holders.
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

/// The lock file's name inside `handover/`.
pub const LOCK_FILE: &str = ".live";

/// This process's holders, by the lock file's path (`lock_path`). A
/// present entry whose guard is gone is a holder still releasing (CP17
/// review LOW-4): it counts as this process, never as another.
static HELD: Mutex<BTreeMap<PathBuf, Weak<Held>>> = Mutex::new(BTreeMap::new());

struct Held {
    key: PathBuf,
    file: Mutex<Option<File>>,
}
impl Drop for Held {
    fn drop(&mut self) {
        // CP17 review LOW-4: the file (and its OS lock) goes first, then
        // the entry; until then an `acquire` here sees a releasing holder
        // and retries, never another process.
        self.file.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        // CP23 (CP22 review MEDIUM-1): never `upgrade` under `HELD`. The
        // temporary `Arc` could be the last one (its other holder dropping
        // meanwhile), and its `Drop` would lock `HELD` again on this thread.
        if held.get(&self.key).is_some_and(|w| w.strong_count() == 0) {
            held.remove(&self.key);
        }
    }
}

/// A held per-world handover lock. Dropping the last clone in this process
/// releases it.
#[derive(Clone)]
pub struct LiveLock(#[allow(dead_code)] Arc<Held>);

impl LiveLock {
    /// CP23: the import's lock file goes, once its import folder has gone,
    /// while it is still held (so nobody locks it believing the import is
    /// there; whoever opened it meanwhile sees it renamed away and starts
    /// over). Only the file this guard holds is removed.
    ///
    /// CP24 (CP23 review MEDIUM-2, DECIDED): Unix only. Elsewhere the
    /// "still the same file" check is by name, not by file identity, so a
    /// lock file deleted under its holder could let two processes hold two
    /// different files at the same name; there the file is never deleted
    /// (it is empty, and costs one entry of the imports scan bound) and a
    /// later holder takes the same file. Not run on Windows here.
    pub fn remove_lock_file(&self) {
        #[cfg(unix)]
        self.remove_lock_file_unix();
    }
    #[cfg(unix)]
    fn remove_lock_file_unix(&self) {
        let key = &self.0.key;
        let g = self.0.file.lock().unwrap_or_else(|e| e.into_inner());
        if g.as_ref().is_some_and(|f| still_named(f, key))
            && let Err(e) = std::fs::remove_file(key)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("handover: the import's lock file {} couldn't be removed ({e}); a later sweep removes it", key.display());
        }
    }
    /// CP19 (CP18 review MEDIUM-2): the held lock file is still the one at
    /// `world_dir`'s lock path (Unix: same device and inode; elsewhere: it
    /// still exists there), i.e. nobody removed the folder under the lock.
    pub fn still_at(&self, world_dir: &Path) -> bool {
        let Ok(on_disk) = std::fs::symlink_metadata(super::authority::dir(world_dir).join(LOCK_FILE)) else { return false };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let g = self.0.file.lock().unwrap_or_else(|e| e.into_inner());
            let Some(held) = g.as_ref().and_then(|f| f.metadata().ok()) else { return false };
            held.dev() == on_disk.dev() && held.ino() == on_disk.ino()
        }
        #[cfg(not(unix))]
        on_disk.is_file()
    }
}

/// Why the lock couldn't be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockError {
    /// Another process on this computer holds it.
    HeldElsewhere,
    /// The lock file couldn't be made or opened (or is not a regular file).
    Io,
    /// CP19 (CP18 review MEDIUM-2): the world folder is gone (never
    /// recreated by `acquire_existing`).
    Missing,
    /// CP17 review MEDIUM-1: this folder's file system gives no file lock
    /// (any lock error other than "held"). Opening, saving and hosting go
    /// on as before the lock; a new handover refuses.
    Unsupported,
}

#[cfg(test)]
thread_local! {
    /// Tests: every lock call on this thread fails as on a file system
    /// without file locking.
    pub(crate) static NO_LOCKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `try_lock` / `try_lock_shared`, with the test hook.
fn try_lock(file: &File, shared: bool) -> Result<(), TryLockError> {
    #[cfg(test)]
    if NO_LOCKING.with(|c| c.get()) {
        return Err(TryLockError::Error(std::io::Error::from(std::io::ErrorKind::Unsupported)));
    }
    if shared { file.try_lock_shared() } else { file.try_lock() }
}

/// Log once per lock file that its folder gives no file lock.
fn log_unsupported(key: &Path, e: &std::io::Error) {
    static LOGGED: Mutex<std::collections::BTreeSet<PathBuf>> = Mutex::new(std::collections::BTreeSet::new());
    if LOGGED.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_path_buf()) {
        log::warn!("handover: {} can't be locked ({e}); this folder has no file locking, so no handover can start here", key.display());
    }
}

/// CP17 review LOW-4 (c): the canonical `handover/` (symlinks, `..` and
/// other spellings resolved) when it exists, else the absolute path.
fn lock_path(world_dir: &Path) -> PathBuf {
    let d = super::authority::dir(world_dir);
    let d = std::fs::canonicalize(&d).or_else(|_| std::path::absolute(&d)).unwrap_or(d);
    d.join(LOCK_FILE)
}

/// Take the world's handover lock (creating `handover/` and `.live` when
/// needed). Already held in this process: the same guard. A brief retry
/// covers another process's momentary probe (`held_elsewhere`).
pub fn acquire(world_dir: &Path) -> Result<LiveLock, LockError> {
    super::authority::store(world_dir).map_err(|_| LockError::Io)?;
    take(world_dir)
}

/// CP19 (CP18 review MEDIUM-2, DECIDED): the recipient's lock on its import.
/// Never creates a missing world folder: `world_dir` must already be a real
/// directory, and only `handover/` inside it is made (one `create_dir`,
/// never `create_dir_all`), so a folder another process removed stays
/// removed and the caller refuses (`Missing`).
pub fn acquire_existing(world_dir: &Path) -> Result<LiveLock, LockError> {
    match std::fs::symlink_metadata(world_dir) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(LockError::Io),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LockError::Missing),
        Err(_) => return Err(LockError::Io),
    }
    let d = super::authority::dir(world_dir);
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    match b.create(&d) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LockError::Missing),
        Err(_) => return Err(LockError::Io),
    }
    if !std::fs::symlink_metadata(&d).is_ok_and(|m| m.is_dir()) {
        return Err(LockError::Io);
    }
    take(world_dir)
}

fn take(world_dir: &Path) -> Result<LiveLock, LockError> {
    take_key(lock_path(world_dir), InProcess::Share, true)
}

/// Whether a holder already in this process is shared or counts as held.
#[derive(Clone, Copy, PartialEq, Eq)]
enum InProcess {
    /// A world's `.live`: the session and the resumer share one guard.
    Share,
    /// An import's lock (CP23): only the guard's own clones share it, so a
    /// sweep in this process never "takes" the lock its own import worker
    /// holds.
    Refuse,
}

/// The lock file opened for a take: never followed, 0600, created only
/// when `create`.
fn open_lock(key: &Path, create: bool) -> Result<File, LockError> {
    match std::fs::symlink_metadata(key) {
        Ok(m) if !m.is_file() => return Err(LockError::Io),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !create => return Err(LockError::Missing),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(LockError::Io),
    }
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(create).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    opts.open(key).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { LockError::Missing } else { LockError::Io })
}

/// The open file is still the one at `key` (CP23: an import's lock file is
/// removed by its holder once the import is gone; whoever locked the
/// unlinked file meanwhile takes the new one instead).
fn still_named(file: &File, key: &Path) -> bool {
    let Ok(on_disk) = std::fs::symlink_metadata(key) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        file.metadata().is_ok_and(|m| m.dev() == on_disk.dev() && m.ino() == on_disk.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        on_disk.is_file()
    }
}

/// CP23 (CP22 review P-1): the registry (`HELD`) is held only for the map
/// check, the try-lock and the insert; the stat and the open (which on an
/// encrypted home folder writes the file's header) happen outside it.
fn take_key(key: PathBuf, in_process: InProcess, create: bool) -> Result<LiveLock, LockError> {
    take_key_passes(key, in_process, create, 80)
}

/// `take_key` in at most `passes` passes (5 ms apart), then `HeldElsewhere`.
fn take_key_passes(key: PathBuf, in_process: InProcess, create: bool, passes: u32) -> Result<LiveLock, LockError> {
    // CP17 review LOW-4 (b): never sleep while holding the registry.
    for pass in 0..passes {
        let releasing = {
            let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
            match held.get(&key) {
                Some(w) if in_process == InProcess::Share => {
                    if let Some(h) = w.upgrade() {
                        return Ok(LiveLock(h));
                    }
                    true
                }
                // An import's lock held in this process is held.
                Some(w) if w.strong_count() > 0 => return Err(LockError::HeldElsewhere),
                Some(_) => true,
                None => false,
            }
        };
        if !releasing {
            let file = open_lock(&key, create)?;
            let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
            match held.get(&key) {
                Some(w) if in_process == InProcess::Share => {
                    if let Some(h) = w.upgrade() {
                        drop(held);
                        return Ok(LiveLock(h));
                    }
                }
                Some(w) if w.strong_count() > 0 => return Err(LockError::HeldElsewhere),
                Some(_) => {}
                None => match try_lock(&file, false) {
                    Ok(()) if still_named(&file, &key) => {
                        let h = Arc::new(Held { key: key.clone(), file: Mutex::new(Some(file)) });
                        held.insert(key, Arc::downgrade(&h));
                        return Ok(LiveLock(h));
                    }
                    // Its holder removed it meanwhile: the next pass opens
                    // the file now at `key` (or none, without `create`).
                    Ok(()) => {}
                    // Another process's holder or momentary probe.
                    Err(TryLockError::WouldBlock) => {}
                    Err(TryLockError::Error(e)) => {
                        log_unsupported(&key, &e);
                        return Err(LockError::Unsupported);
                    }
                },
            }
            drop(held);
            drop(file);
        }
        if pass + 1 < passes {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    Err(LockError::HeldElsewhere)
}

/// CP23 (DECIDED, coordinator 2026-10-13): the lock file of the import
/// folder `<imports>/<folder>`, a sibling file `<folder>.lock` and never
/// inside a folder that is renamed while it is held (Windows can refuse
/// to rename a folder with a file open inside it).
pub fn import_lock_path(imports: &Path, folder: &str) -> PathBuf {
    let d = std::fs::canonicalize(imports).or_else(|_| std::path::absolute(imports)).unwrap_or_else(|_| imports.to_path_buf());
    d.join(format!("{folder}.lock"))
}

/// CP23: take the import's lock. `create`: make the lock file if it
/// isn't there (the recipient, before the rename that makes its import
/// visible; a discard of an import that exists); otherwise a missing lock
/// file is `Missing` and nothing is created. Never creates the import
/// folder. Held anywhere else, in this process too: `HeldElsewhere` (only
/// the guard's own clones share it).
pub fn acquire_import(imports: &Path, folder: &str, create: bool) -> Result<LiveLock, LockError> {
    take_key(import_lock_path(imports, folder), InProcess::Refuse, create)
}

/// CP24 (CP23 review LOW-1): `acquire_import` for the lobby's main thread
/// (promotion and its leftover sweep): ONE try, never creating the file.
/// Held by another process (or its probe, or a holder in this process
/// letting go this instant) is `HeldElsewhere` at once, never a ~400 ms
/// wait; the caller's 500 ms held retry tries again.
pub fn try_import_once(imports: &Path, folder: &str) -> Result<LiveLock, LockError> {
    take_key_passes(import_lock_path(imports, folder), InProcess::Refuse, false, 1)
}

/// CP18: whether a holder in THIS process (a live session, the resumer's
/// wait) still holds the world's handover lock. Read-only.
///
/// CP23 (CP22 review MEDIUM-1): counts the holders without upgrading. A
/// temporary `Arc` made under `HELD` can be the last one when the holder
/// lets go meanwhile (the lobby's held-retry polls exactly then), and its
/// `Drop` locks `HELD` again on this thread: a self-deadlock.
pub fn held_here(world_dir: &Path) -> bool {
    let key = lock_path(world_dir);
    let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    held.get(&key).is_some_and(|w| {
        let n = w.strong_count();
        #[cfg(test)]
        UNDER_HELD.with(|h| {
            if let Some(f) = h.borrow_mut().as_mut() {
                f();
            }
        });
        n > 0
    })
}

#[cfg(test)]
thread_local! {
    /// Tests: run inside `held_here` while it holds `HELD` (another
    /// thread's holder letting go at that moment).
    static UNDER_HELD: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
}

/// Whether another process on this computer holds the world's handover
/// lock. Read-only: no `handover/` or `.live` is ever created (none means
/// no holder). This process's own holders never count. A lock file that
/// can't be opened or checked counts as held (fail closed: the world may
/// be mid-handover).
pub fn held_elsewhere(world_dir: &Path) -> bool {
    let key = lock_path(world_dir);
    // This process's holder, live or still releasing (LOW-4 (a)).
    if HELD.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&key) {
        return false;
    }
    // CP23 (CP22 review P-1): the stat and the open outside the registry.
    match std::fs::symlink_metadata(&key) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Ok(m) if m.is_file() => {}
        _ => return true,
    }
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    let file = match opts.open(&key) {
        Ok(f) => f,
        Err(e) => {
            // CP17 review MEDIUM-1: an existing regular `.live` that can't be
            // opened is no proof of a holder; the world stays usable.
            log_unsupported(&key, &e);
            return false;
        }
    };
    // Probed (and let go) under the registry's lock, so this process's own
    // `acquire` never meets the probe. CP17 review LOW-3: a shared lock, so
    // two probes never collide; another process's exclusive holder still
    // blocks it. CP17 review MEDIUM-1: only "would block" is held.
    let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if held.contains_key(&key) {
        return false;
    }
    let r = match try_lock(&file, true) {
        Ok(()) => false,
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(e)) => {
            log_unsupported(&key, &e);
            false
        }
    };
    drop(file);
    drop(held);
    r
}

/// The resumer's and the start-up promotion's take: `None` when another
/// process holds the lock (skipped, logged once); otherwise the guard to
/// hold while working, or `Some(None)` when the lock file can't be made
/// here but no other process is seen holding one (work goes on as it did
/// before the lock existed, with a warning).
pub(crate) fn unless_elsewhere(world_dir: &Path, what: &str) -> Option<Option<LiveLock>> {
    match acquire(world_dir) {
        Ok(lock) => Some(Some(lock)),
        Err(LockError::HeldElsewhere) => {
            log_skipped(world_dir, what);
            None
        }
        Err(LockError::Io) if held_elsewhere(world_dir) => {
            log_skipped(world_dir, what);
            None
        }
        Err(LockError::Io | LockError::Unsupported | LockError::Missing) => {
            log::warn!("handover: {}'s handover lock couldn't be taken; {what} goes on without it", world_dir.display());
            Some(None)
        }
    }
}

/// Start-up: log once per world per process that it was skipped because
/// another process holds its lock.
pub(crate) fn log_skipped(world_dir: &Path, what: &str) {
    static LOGGED: Mutex<std::collections::BTreeSet<PathBuf>> = Mutex::new(std::collections::BTreeSet::new());
    if LOGGED.lock().unwrap_or_else(|e| e.into_inner()).insert(lock_path(world_dir)) {
        log::info!("handover: {} is in a handover in another copy of the game; {what} is skipped here", world_dir.display());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The env var naming the world folder the helper child locks.
    pub(crate) const CHILD_ENV: &str = "AXENSTAX_HANDOVER_LOCK_CHILD";

    /// Not a test on its own: the second process. Run by `ChildHolder`
    /// (re-executing this test binary); without the env var it does nothing.
    /// CP19 (CP18 review MEDIUM-2): the recipient's lock never recreates a
    /// world folder another process removed; on one that exists it makes
    /// only `handover/`, and notices the folder removed under it.
    #[test]
    fn handover_lock_on_an_import_never_recreates_a_vanished_folder() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let base = std::env::temp_dir().join(format!("axenstax-lock-vanish-{}", hex::encode(r)));
        let gone = base.join("import").join("working");
        assert!(matches!(acquire_existing(&gone), Err(LockError::Missing)));
        assert!(!base.join("import").exists(), "taking the lock recreated a vanished import folder");
        std::fs::create_dir_all(&gone).unwrap();
        let lock = acquire_existing(&gone).expect("the lock on an existing import");
        assert!(lock.still_at(&gone));
        std::fs::remove_dir_all(base.join("import")).unwrap();
        assert!(!lock.still_at(&gone), "a folder removed under the lock still reads as there");
        drop(lock);
        let _ = std::fs::remove_dir_all(&base);
    }

    const SWEEP_ENV: &str = "AXENSTAX_HANDOVER_SWEEP_CHILD";
    #[test]
    fn handover_sweep_child_process() {
        let Some(dir) = std::env::var_os(SWEEP_ENV) else { return };
        let r = super::super::inert::sweep(Path::new(&dir), None);
        println!("SWEEP {}", if r.is_ok() { "Ok" } else { "Err" });
    }

    /// Another game process running the inert sweep over `imports` once;
    /// its "SWEEP Ok" / "SWEEP Err" line.
    pub(crate) fn child_sweep(imports: &Path) -> String {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "handover::live::world_lock::tests::handover_sweep_child_process", "--nocapture", "--test-threads=1"])
            .env(SWEEP_ENV, imports)
            .stderr(std::process::Stdio::null())
            .output()
            .expect("run the second process");
        let text = String::from_utf8_lossy(&out.stdout);
        ["SWEEP Ok", "SWEEP Err"].into_iter().find(|w| text.contains(w)).unwrap_or("no sweep line").to_owned()
    }

    #[test]
    fn handover_lock_child_process() {
        let Some(dir) = std::env::var_os(CHILD_ENV) else { return };
        let _lock = acquire(Path::new(&dir)).expect("the child takes the lock");
        println!("LOCKED");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    /// CP24: the env var naming the import folder whose sibling lock the
    /// helper child takes.
    const IMPORT_CHILD_ENV: &str = "AXENSTAX_HANDOVER_IMPORT_LOCK_CHILD";

    /// Not a test on its own: a second process holding an import's lock
    /// (`<imports>/<import>.lock`), as its live session would.
    #[test]
    fn handover_import_lock_child_process() {
        let Some(dir) = std::env::var_os(IMPORT_CHILD_ENV) else { return };
        let import = PathBuf::from(dir);
        let folder = import.file_name().and_then(|n| n.to_str()).unwrap().to_owned();
        let _lock = acquire_import(import.parent().unwrap(), &folder, true).expect("the child takes the import lock");
        println!("LOCKED");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    /// A second game process holding `world`'s handover lock, until killed.
    pub(crate) struct ChildHolder(std::process::Child);
    impl ChildHolder {
        pub(crate) fn hold(world: &Path) -> Self {
            Self::spawn("handover_lock_child_process", CHILD_ENV, world)
        }
        /// CP24: a second game process holding the import lock of the
        /// import folder `import` (`<imports>/<import>.lock`), until killed.
        pub(crate) fn hold_import(import: &Path) -> Self {
            Self::spawn("handover_import_lock_child_process", IMPORT_CHILD_ENV, import)
        }
        fn spawn(test: &str, env: &str, path: &Path) -> Self {
            use std::io::BufRead;
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &format!("handover::live::world_lock::tests::{test}"), "--nocapture", "--test-threads=1"])
                .env(env, path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn the second process");
            let out = child.stdout.take().unwrap();
            let mut lines = std::io::BufReader::new(out).lines();
            assert!(
                lines.any(|l| l.is_ok_and(|l| l.contains("LOCKED"))),
                "the second process never took the lock"
            );
            Self(child)
        }
        /// The process dies without releasing anything itself.
        pub(crate) fn kill(mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    impl Drop for ChildHolder {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// CP23 (CP22 review MEDIUM-1): the last other holder lets go while
    /// `held_here` looks at the registry. Before, `held_here`'s temporary
    /// `Arc` was then the last one, and its drop locked `HELD` again on the
    /// same thread: the call never returned and every later lock user in
    /// the process hung. Now it returns, and the lock is released.
    #[test]
    fn handover_lock_held_here_survives_the_holder_letting_go_meanwhile() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let world = std::env::temp_dir().join(format!("axenstax-lock-heldhere-{}", hex::encode(r)));
        std::fs::create_dir_all(&world).unwrap();
        let lock = acquire(&world).expect("the lock");
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let probe_world = world.clone();
        std::thread::spawn(move || {
            let mut holder = Some(lock);
            UNDER_HELD.with(|h| {
                *h.borrow_mut() = Some(Box::new(move || {
                    // The holder (another thread in the game) lets go now.
                    if let Some(l) = holder.take() {
                        let t = std::thread::spawn(move || drop(l));
                        // Its release waits for `HELD` (held here) unless
                        // nothing else of it remains; give it a moment.
                        let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
                        while !t.is_finished() && std::time::Instant::now() < until {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                    }
                }))
            });
            let r = held_here(&probe_world);
            UNDER_HELD.with(|h| h.borrow_mut().take());
            let _ = done_tx.send(r);
        });
        let r = done_rx.recv_timeout(std::time::Duration::from_secs(10));
        assert!(r.is_ok(), "held_here never returned: the thread deadlocked on its own registry lock");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while held_here(&world) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let f = File::open(lock_path(&world)).unwrap();
        assert!(f.try_lock().is_ok(), "the released lock stayed held");
        drop(f);
        let _ = std::fs::remove_dir_all(&world);
    }

    #[test]
    fn handover_lock_another_process_holds_until_it_dies() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let tmp = std::env::temp_dir().join(format!("axenstax-lock-{}", hex::encode(r)));
        let world = tmp.join("w");
        std::fs::create_dir_all(&world).unwrap();
        assert!(!held_elsewhere(&world), "no lock file is no holder");
        assert!(!super::super::authority::dir(&world).exists(), "the probe created handover/");
        let child = ChildHolder::hold(&world);
        assert!(held_elsewhere(&world), "a second process's lock wasn't seen");
        assert_eq!(acquire(&world).err(), Some(LockError::HeldElsewhere));
        child.kill();
        assert!(!held_elsewhere(&world), "a dead process's lock stayed held");
        let lock = acquire(&world).expect("taken once the holder died");
        assert!(!held_elsewhere(&world), "this process's own lock counted as another's");
        let again = acquire(&world).expect("this process shares its own lock");
        drop(lock);
        drop(again);
        // Released by drop: a raw handle (as another process would) takes it.
        let f = File::open(lock_path(&world)).unwrap();
        assert!(f.try_lock().is_ok(), "dropping the last guard didn't release the lock");
        drop(f);
        let _ = std::fs::remove_dir_all(&tmp);
    }
    /// CP18 (CP17 review MEDIUM-1): on a file system with no file locking
    /// (any lock error but "would block") a world that has a `.live` still
    /// opens, and the start-up work goes on without the lock; only a new
    /// handover is refused (`acquire` says `Unsupported`).
    #[test]
    fn handover_lock_no_file_locking_never_makes_a_world_unopenable() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let world = std::env::temp_dir().join(format!("axenstax-nolock-{}", hex::encode(r)));
        std::fs::create_dir_all(&world).unwrap();
        drop(acquire(&world).expect("an ordinary lock"));
        assert!(world.join("handover").join(LOCK_FILE).exists());
        NO_LOCKING.with(|c| c.set(true));
        assert!(!held_elsewhere(&world), "a lock error read as another process's handover");
        assert_eq!(super::super::authority::open_guard(&world).map_err(|r| r.0), Ok(()), "a world with no file locking was refused");
        assert_eq!(acquire(&world).err(), Some(LockError::Unsupported), "a handover could start with no file lock");
        assert!(matches!(unless_elsewhere(&world, "test"), Some(None)), "the start-up work was skipped");
        NO_LOCKING.with(|c| c.set(false));
        let _ = std::fs::remove_dir_all(&world);
    }
}

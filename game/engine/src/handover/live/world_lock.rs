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

/// This process's holders, by the lock file's absolute path.
static HELD: Mutex<BTreeMap<PathBuf, Weak<Held>>> = Mutex::new(BTreeMap::new());

struct Held {
    key: PathBuf,
    file: Mutex<Option<File>>,
}
impl Drop for Held {
    fn drop(&mut self) {
        // Released under the registry's lock, so an `acquire` in this
        // process never meets this handle's OS lock half-released.
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        self.file.lock().unwrap_or_else(|e| e.into_inner()).take();
        if held.get(&self.key).is_some_and(|w| w.upgrade().is_none()) {
            held.remove(&self.key);
        }
    }
}

/// A held per-world handover lock. Dropping the last clone in this process
/// releases it.
#[derive(Clone)]
pub struct LiveLock(#[allow(dead_code)] Arc<Held>);

/// Why the lock couldn't be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockError {
    /// Another process on this computer holds it.
    HeldElsewhere,
    /// The lock file couldn't be made or opened (or is not a regular file).
    Io,
}

fn lock_path(world_dir: &Path) -> PathBuf {
    let p = super::authority::dir(world_dir).join(LOCK_FILE);
    std::path::absolute(&p).unwrap_or(p)
}

/// Take the world's handover lock (creating `handover/` and `.live` when
/// needed). Already held in this process: the same guard. A brief retry
/// covers another process's momentary probe (`held_elsewhere`).
pub fn acquire(world_dir: &Path) -> Result<LiveLock, LockError> {
    let key = lock_path(world_dir);
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = held.get(&key).and_then(Weak::upgrade) {
        return Ok(LiveLock(h));
    }
    super::authority::store(world_dir).map_err(|_| LockError::Io)?;
    match std::fs::symlink_metadata(&key) {
        Ok(m) if !m.is_file() => return Err(LockError::Io),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(LockError::Io),
    }
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = opts.open(&key).map_err(|_| LockError::Io)?;
    let mut tries = 0;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if tries < 40 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(TryLockError::WouldBlock) => return Err(LockError::HeldElsewhere),
            Err(TryLockError::Error(e)) => {
                log::warn!("handover: the world's handover lock couldn't be taken ({e})");
                return Err(LockError::Io);
            }
        }
    }
    let h = Arc::new(Held { key: key.clone(), file: Mutex::new(Some(file)) });
    held.insert(key, Arc::downgrade(&h));
    Ok(LiveLock(h))
}

/// Whether another process on this computer holds the world's handover
/// lock. Read-only: no `handover/` or `.live` is ever created (none means
/// no holder). This process's own holders never count. A lock file that
/// can't be opened or checked counts as held (fail closed: the world may
/// be mid-handover).
pub fn held_elsewhere(world_dir: &Path) -> bool {
    let key = lock_path(world_dir);
    let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if held.get(&key).is_some_and(|w| w.upgrade().is_some()) {
        return false;
    }
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
    let Ok(file) = opts.open(&key) else { return true };
    // Probed under the registry's lock, so this process's own `acquire`
    // never meets the probe.
    match file.try_lock() {
        Ok(()) => false,
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(e)) => {
            log::warn!("handover: the world's handover lock can't be checked ({e}); the world is treated as in use");
            true
        }
    }
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
        Err(LockError::Io) => {
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

    /// A second game process holding `world`'s handover lock, until killed.
    pub(crate) struct ChildHolder(std::process::Child);
    impl ChildHolder {
        pub(crate) fn hold(world: &Path) -> Self {
            use std::io::BufRead;
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "handover::live::world_lock::tests::handover_lock_child_process", "--nocapture", "--test-threads=1"])
                .env(CHILD_ENV, world)
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
}

//! The late-save fence (handover CP14; CP9 step 4).
//!
//! A source handover freezes its world, then captures it. Every writer into
//! a world's folder runs synchronously on its caller's thread today (see
//! the list in Spec 04 "The late-save fence"), so no save job is in flight
//! on another thread at the freeze. The fence makes that structural rather
//! than incidental:
//! - the freeze raises a per-world fence ([`raise`]). Raising takes the
//!   commit lock, so a commit already under way on any thread finishes
//!   first and the capture reflects the last save that landed;
//! - every commit into a world's folder (the rename or delete that makes a
//!   write visible) checks the fence under that same lock immediately
//!   before it commits ([`commit`]), and a fenced commit discards itself:
//!   its temp file is removed and it logs one line;
//! - the fence lifts only on an abort that unfreezes ([`lift`], from
//!   `HostedServer::handover_release` before retirement). Once the world is
//!   retired ([`retire`]) it never lifts in this process, and the retirement
//!   record keeps the world refused across restarts
//!   (`handover::live::authority::open_guard`).
//!
//! No save writes under a world's `handover/`: the handover's own records go
//! through `handover::durable`, never through here, and a save commit there
//! is refused.
//!
//! Fences are keyed by the folder path as `save::world_dir` spells it; the
//! source session's `world_dir` is built by that same function.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// A late save's commit, discarded because a handover froze its world.
pub(crate) const DISCARDED: &str = "a handover froze this world; a late save was discarded";
/// A save commit aimed under a world's `handover/` folder.
pub(crate) const UNDER_HANDOVER: &str = "a save never writes under a world's handover/ folder";

struct Fence {
    dir: PathBuf,
    retired: bool,
}

/// Every raised fence. Also the commit lock: a commit runs while holding it.
static FENCES: Mutex<Vec<Fence>> = Mutex::new(Vec::new());

fn fences() -> MutexGuard<'static, Vec<Fence>> {
    FENCES.lock().unwrap_or_else(|e| e.into_inner())
}

fn covers(list: &[Fence], path: &Path) -> bool {
    list.iter().any(|f| path.starts_with(&f.dir))
}

/// Raise the fence over `dir`. Waits for a commit under way (the lock).
pub(crate) fn raise(dir: &Path) {
    let mut list = fences();
    if !list.iter().any(|f| f.dir == dir) {
        list.push(Fence { dir: dir.to_path_buf(), retired: false });
    }
}

/// `dir`'s world was retired (or may have been): its fence never lifts.
pub(crate) fn retire(dir: &Path) {
    let mut list = fences();
    match list.iter_mut().find(|f| f.dir == dir) {
        Some(f) => f.retired = true,
        None => list.push(Fence { dir: dir.to_path_buf(), retired: true }),
    }
}

/// An abort that unfroze `dir` before retirement: saves land again. A
/// retired world's fence stays up.
pub(crate) fn lift(dir: &Path) {
    fences().retain(|f| f.dir != dir || f.retired);
}

/// Is `path` inside a fenced world folder?
pub(crate) fn is_up(path: &Path) -> bool {
    covers(&fences(), path)
}

/// A writer's early refusal, before any temp file or folder is made.
pub(crate) fn check(dir: &Path) -> Result<(), String> {
    if is_up(dir) {
        log::warn!("{}: {DISCARDED}", dir.display());
        return Err(DISCARDED.to_string());
    }
    Ok(())
}

/// Did [`commit`] refuse (as opposed to the commit itself failing)?
pub(crate) fn refused(e: &str) -> bool {
    e == DISCARDED || e == UNDER_HANDOVER
}

/// Is `path` under a world folder's `handover/`? A world folder here is a
/// fenced folder, a direct child of the worlds root, or a folder that holds
/// a world's `world.dat` or `world_meta.json`.
fn under_handover(list: &[Fence], path: &Path) -> bool {
    path.ancestors().skip(1).any(|a| {
        a.file_name().is_some_and(|n| n == "handover")
            && a.parent().is_some_and(|w| {
                list.iter().any(|f| f.dir == w)
                    || is_worlds_root_child(w)
                    || w.join("world.dat").exists()
                    || w.join("world_meta.json").exists()
            })
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn is_worlds_root_child(w: &Path) -> bool {
    w.parent() == Some(crate::save::worlds_root().as_path())
}
#[cfg(target_arch = "wasm32")]
fn is_worlds_root_child(_: &Path) -> bool {
    false
}

/// The commit point of a save into `path`: run `op` (the rename or delete
/// that makes the write visible) only when no fence covers `path`, holding
/// the fence lock so a freeze can't land between the check and the commit.
/// A fenced commit logs one line and returns [`DISCARDED`].
pub(crate) fn commit<T>(path: &Path, op: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    commit_inner(path, true, op)
}

/// [`commit`] without the log line, for the deletes a save makes before its
/// first write (that write logs the discard).
pub(crate) fn commit_quiet<T>(path: &Path, op: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    commit_inner(path, false, op)
}

fn commit_inner<T>(path: &Path, loud: bool, op: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    #[cfg(test)]
    hold::arrive(path);
    let list = fences();
    if under_handover(&list, path) {
        log::error!("{}: {UNDER_HANDOVER}", path.display());
        return Err(UNDER_HANDOVER.to_string());
    }
    if covers(&list, path) {
        if loud {
            log::warn!("{}: {DISCARDED}", path.display());
        }
        return Err(DISCARDED.to_string());
    }
    op()
}

/// Test seam: hold the next commit under a folder at its commit point
/// (before the fence lock), so a test can freeze and retire in between.
#[cfg(test)]
pub(crate) mod hold {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Condvar, Mutex};

    #[derive(Default)]
    struct State {
        arrived: bool,
        released: bool,
    }
    pub(crate) struct Hold {
        dir: PathBuf,
        state: Mutex<State>,
        cv: Condvar,
    }
    static HOLDS: Mutex<Vec<Arc<Hold>>> = Mutex::new(Vec::new());

    /// Hold the next commit under `dir`.
    pub(crate) fn hold(dir: &Path) -> Arc<Hold> {
        let h = Arc::new(Hold { dir: dir.to_path_buf(), state: Mutex::default(), cv: Condvar::new() });
        HOLDS.lock().unwrap().push(h.clone());
        h
    }

    impl Hold {
        /// Wait (bounded) until a commit is held here.
        pub(crate) fn wait_arrived(&self) -> bool {
            let s = self.state.lock().unwrap();
            let (s, _) = self
                .cv
                .wait_timeout_while(s, std::time::Duration::from_secs(30), |s| !s.arrived)
                .unwrap();
            s.arrived
        }
        /// Let the held commit go on to its fence check.
        pub(crate) fn release(&self) {
            self.state.lock().unwrap().released = true;
            self.cv.notify_all();
        }
    }

    pub(super) fn arrive(path: &Path) {
        let h = {
            let mut holds = HOLDS.lock().unwrap();
            let Some(i) = holds.iter().position(|h| path.starts_with(&h.dir)) else { return };
            holds.remove(i)
        };
        let mut s = h.state.lock().unwrap();
        s.arrived = true;
        h.cv.notify_all();
        while !s.released {
            s = h.cv.wait(s).unwrap();
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("save-fence-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("handover")).unwrap();
        std::fs::write(p.join("world_meta.json"), b"{}").unwrap();
        p
    }

    /// No save path writes under a world's `handover/`: the save's atomic
    /// writers refuse there, leave nothing behind, and write elsewhere.
    #[test]
    fn a_save_never_writes_under_the_worlds_handover_folder() {
        let w = dir("handover");
        let target = w.join("handover").join("retired-00.dat");
        assert_eq!(crate::save::write_atomic(&target, b"x").unwrap_err(), UNDER_HANDOVER);
        assert_eq!(crate::save::write_atomic_nosync(&target, b"x").unwrap_err(), UNDER_HANDOVER);
        let left: Vec<_> = std::fs::read_dir(w.join("handover")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(left.is_empty(), "a save wrote under handover/: {left:?}");
        crate::save::write_atomic(&w.join("world.dat"), b"ok").unwrap();
        let _ = std::fs::remove_dir_all(&w);
    }

    /// Raised: every commit under the folder is discarded (temp removed);
    /// lifted: it lands. A retired fence never lifts.
    #[test]
    fn a_fence_discards_until_lifted_and_a_retired_one_never_lifts() {
        let w = dir("lift");
        raise(&w);
        assert_eq!(crate::save::write_atomic(&w.join("world.dat"), b"late").unwrap_err(), DISCARDED);
        assert!(!w.join("world.dat").exists() && !w.join("world.dat.tmp").exists());
        lift(&w);
        crate::save::write_atomic(&w.join("world.dat"), b"lands").unwrap();
        retire(&w);
        lift(&w);
        assert_eq!(crate::save::write_atomic(&w.join("world.dat"), b"late").unwrap_err(), DISCARDED);
        assert_eq!(std::fs::read(w.join("world.dat")).unwrap(), b"lands");
        let _ = std::fs::remove_dir_all(&w);
    }
}

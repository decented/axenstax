//! A folder move that never replaces what is already at the target.
//!
//! Used by the import's own rename (`checkpoint::files::rename_new`) and by
//! the move of an Activated import into the world list
//! (`live::promote::rename_no_replace`).
//!
//! Linux and Android: `renameat2(RENAME_NOREPLACE)`, atomic (EEXIST if
//! anything is there). macOS: `renamex_np(RENAME_EXCL)`, atomic.
//!
//! CP21 (CP19 review MEDIUM-2, DECIDED): a filesystem without the
//! no-replace flag (NFS, FUSE without rename2, ecryptfs home folders —
//! seen on the dev machine: `renameat2` returns EINVAL there) gets the
//! check-then-rename path every other platform has: the target is checked
//! absent just before a plain `rename`. A residual race remains there:
//! `rename(2)` would replace an EMPTY folder someone made in between (never
//! a non-empty one, never a file). Only EINVAL and ENOSYS fall back; every
//! other error is returned as it is.

use std::io;
use std::path::Path;

/// The atomic attempt (injectable so a test can stand in an unsupporting
/// filesystem).
pub type Atomic = fn(&Path, &Path) -> io::Result<()>;

/// Move `from` to `to`, never replacing anything at `to`.
pub fn no_replace(from: &Path, to: &Path) -> io::Result<()> {
    no_replace_with(from, to, atomic)
}

/// `no_replace` with the atomic attempt supplied.
pub fn no_replace_with(from: &Path, to: &Path, attempt: Atomic) -> io::Result<()> {
    match attempt(from, to) {
        Err(e) if flag_unsupported(&e) => {
            log::warn!(
                "handover: this filesystem can't move a folder without replacing ({e}); checking the target first instead"
            );
            check_then_rename(from, to)
        }
        r => r,
    }
}

/// The filesystem (or kernel) doesn't support the no-replace flag.
fn flag_unsupported(e: &io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS))
    }
    #[cfg(not(unix))]
    {
        let _ = e;
        false
    }
}

fn check_then_rename(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    std::fs::rename(from, to)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn atomic(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let f = std::ffi::CString::new(from.as_os_str().as_bytes())?;
    let t = std::ffi::CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: two valid NUL-terminated paths that outlive the call; the
    // flag forbids replacing the target.
    #[cfg(target_os = "linux")]
    let r = unsafe { libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE) };
    // Bionic's renameat2 symbol needs API 30 and the app supports API 24, so
    // the raw syscall (an old kernel answers ENOSYS and falls back).
    #[cfg(target_os = "android")]
    let r = unsafe {
        libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE)
            as libc::c_int
    };
    #[cfg(target_os = "macos")]
    let r = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(target_os = "windows")]
fn atomic(from: &Path, to: &Path) -> io::Result<()> {
    // BRIDGE: std's rename falls back to a replacing rename for a directory
    // target — replace with a direct MoveFileExW(from, to, 0) (CP19 review
    // MEDIUM-3, next commit).
    check_then_rename(from, to)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "windows")))]
fn atomic(from: &Path, to: &Path) -> io::Result<()> {
    check_then_rename(from, to)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn einval(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::EINVAL))
    }
    fn eacces(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::EACCES))
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("axenstax-rename-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("from")).unwrap();
        std::fs::write(d.join("from/world.dat"), b"w").unwrap();
        d
    }

    #[test]
    fn handover_rename_without_the_flag_falls_back_and_still_never_replaces() {
        let d = dir("einval");
        // A filesystem that rejects the flag still gets the move ...
        no_replace_with(&d.join("from"), &d.join("to"), einval).expect("an EINVAL filesystem never moved the folder");
        assert_eq!(std::fs::read(d.join("to/world.dat")).unwrap(), b"w");
        // ... and still refuses an occupied target, leaving both alone.
        std::fs::create_dir(d.join("taken")).unwrap();
        let e = no_replace_with(&d.join("to"), &d.join("taken"), einval).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(d.join("to/world.dat").exists() && d.join("taken").exists());
        // Any other error is the answer: no fallback move.
        assert!(no_replace_with(&d.join("to"), &d.join("free"), eacces).is_err());
        assert!(d.join("to/world.dat").exists() && !d.join("free").exists());
        // The real call on this filesystem never replaces an empty folder.
        assert!(no_replace(&d.join("to"), &d.join("taken")).is_err());
        no_replace(&d.join("to"), &d.join("free")).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }
}

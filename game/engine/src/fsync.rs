//! CP15 (CP14 review HIGH-1): the ONE place that re-syncs an existing file
//! or a directory by path. Every handover record, import and save-fence path
//! goes through these, never `File::open(..)` + `sync_all` of its own
//! (`tests::no_file_open_sync_outside_this_module` pins that). CP16 (CP15
//! review HIGH-1): no `sync_all`/`sync_data` call exists anywhere in the
//! engine's source but here (`tests::no_sync_call_outside_this_module`), so
//! a read-only handle opened in one file can't be flushed in another; a
//! handle opened for writing is synced through `sync_written`.
//!
//! Windows: `FlushFileBuffers` refuses a handle without write access, so a
//! read-only `File::open` + `sync_all` of a file always fails there (every
//! re-persist of an identical record failed, so nothing was ever published).
//! The file is opened with write access there (no create, no truncate), and
//! read-only elsewhere. A directory can't be opened for a flush on Windows:
//! its sync is skipped there; the rename before it is NTFS-journaled, as
//! the world saves rely on.
use std::path::Path;

/// fsync the existing file at `path` (never creates or truncates it).
pub fn sync_file(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    #[cfg(not(windows))]
    let f = std::fs::File::open(path)?;
    f.sync_all()
}

/// CP17 (CP16 review MEDIUM-1): a file handle opened WITH write access.
/// Only this module makes one (`open_written` forces `write(true)` onto
/// the caller's options), so `sync_written` can't be handed a read-only
/// handle: `sync_written(&File::open(p)?)` doesn't compile. It reads,
/// writes and seeks as the `File` it wraps. CP19 (CP17 review LOW-2): no
/// `DerefMut`, so the inner handle can't be swapped for a read-only one
/// (`*w = File::open(p)?`); `&File` (metadata, `set_len`) is all `Deref`
/// gives. Windows: a caller's `access_mode` could still narrow the access
/// `open_written` asks for; no caller sets one.
pub struct Written(std::fs::File);
impl std::ops::Deref for Written {
    type Target = std::fs::File;
    fn deref(&self) -> &std::fs::File {
        &self.0
    }
}
impl std::io::Write for Written {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl std::io::Read for Written {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl std::io::Seek for Written {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

/// Open `path` with `options` AND write access (forced here, whatever the
/// caller set). CP19 (CP17 review LOW-1): a caller must NOT set `append`:
/// std's Windows mapping drops the write access whenever `append` is set
/// (FILE_APPEND_DATA without FILE_WRITE_DATA), so the handle would be the
/// append-only one `FlushFileBuffers` documents against. The ledger opens
/// read + write with no `append` and seeks to the end before each write.
pub fn open_written(options: &mut std::fs::OpenOptions, path: &Path) -> std::io::Result<Written> {
    options.write(true).open(path).map(Written)
}

/// CP16 (CP15 review HIGH-1): fsync a handle this code itself opened with
/// write access (`open_written`). Every other sync goes through
/// `sync_file` by path, so no read-only handle is ever flushed.
pub fn sync_written(f: &Written) -> std::io::Result<()> {
    f.0.sync_all()
}

/// CP17 (CP16 review LOW-6): fsync the existing REGULAR file at `path`,
/// and only if the file opened is the one checked: never through a symlink
/// (`O_NOFOLLOW` on Unix), and on Unix the opened handle's device and inode
/// must be the checked path's. Write access on Windows, as `sync_file`.
/// CP19 (CP17 review LOW-9): on Unix the open is also `O_NONBLOCK` (a FIFO
/// swapped in between the check and the open can't hang the caller; the
/// flag is harmless on a regular file) and the handle is refused by fstat
/// unless it is a regular file. Only the last path component is protected
/// from a symlink. Windows: no identity check (std has no stable file-index
/// API), so a link planted between the check and the open is followed; the
/// check-then-open race is closed on Unix only.
pub fn sync_regular(path: &Path) -> std::io::Result<()> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidInput, "not the regular file checked");
    let before = std::fs::symlink_metadata(path)?;
    if !before.is_file() {
        return Err(bad());
    }
    #[cfg(test)]
    tests::between_check_and_open(path);
    let mut options = std::fs::OpenOptions::new();
    #[cfg(windows)]
    options.write(true);
    #[cfg(not(windows))]
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let f = options.open(path)?;
    let after = f.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(bad());
        }
    }
    if !after.is_file() {
        return Err(bad());
    }
    f.sync_all()
}

/// fsync the directory at `path`, making the renames done in it durable
/// (a no-op on Windows, see the module comment).
pub fn sync_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
    #[cfg(not(windows))]
    std::fs::File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    thread_local! {
        static SWAP: std::cell::Cell<Option<fn(&std::path::Path)>> = const { std::cell::Cell::new(None) };
    }
    /// Test hook: runs between `sync_regular`'s check and its open.
    pub(super) fn between_check_and_open(path: &std::path::Path) {
        if let Some(f) = SWAP.with(|s| s.take()) {
            f(path);
        }
    }

    /// CP19 (CP17 review LOW-9): a file swapped between the check and the
    /// open is refused: another regular file (a different inode) by the
    /// identity compare, a FIFO promptly (never a hang) by `O_NONBLOCK` and
    /// the fstat.
    #[cfg(unix)]
    #[test]
    fn sync_regular_refuses_a_file_swapped_after_the_check() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let d = std::env::temp_dir().join(format!("axenstax-fsync-swap-{}", hex::encode(r)));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("member");
        std::fs::write(&p, b"x").unwrap();
        SWAP.with(|s| s.set(Some(|p| {
            let side = p.with_extension("new");
            std::fs::write(&side, b"y").unwrap();
            std::fs::rename(&side, p).unwrap();
        })));
        assert!(super::sync_regular(&p).is_err(), "a regular file swapped in after the check was synced");
        let (tx, rx) = std::sync::mpsc::channel();
        let q = d.join("member");
        std::thread::spawn(move || {
            SWAP.with(|s| s.set(Some(|p| {
                std::fs::remove_file(p).unwrap();
                let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
            })));
            let _ = tx.send(super::sync_regular(&q).is_err());
        });
        let refused = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("a FIFO swapped in after the check hung the sync");
        assert!(refused, "a FIFO swapped in after the check was synced");
        std::fs::remove_dir_all(d).unwrap();
    }
    /// No `File::open(..)` followed (within the statement's next lines) by
    /// `sync_all`/`sync_data` anywhere in the engine's source but here.
    #[test]
    fn no_file_open_sync_outside_this_module() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root.clone()];
        let mut offenders = Vec::new();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") || p == root.join("fsync.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap();
                let lines: Vec<&str> = text.lines().collect();
                for (i, l) in lines.iter().enumerate() {
                    if !l.contains("File::open(") {
                        continue;
                    }
                    let window = lines[i..(i + 6).min(lines.len())].join("\n");
                    if window.contains("sync_all") || window.contains("sync_data") {
                        offenders.push(format!("{}:{}", p.display(), i + 1));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "File::open + sync outside crate::fsync (read-only handles can't be flushed on Windows): {offenders:?}");
    }

    /// CP16 (CP15 review HIGH-1): the guard above can be dodged by opening a
    /// file in one place and syncing it in another (the import's receipt did
    /// exactly that). So: no `sync_all(`/`sync_data(` call anywhere in the
    /// engine's source outside this module.
    #[test]
    fn no_sync_call_outside_this_module() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        // CP17 (CP16 review MEDIUM-1): exempt by full path, not basename.
        let this = root.join("fsync.rs");
        let mut stack = vec![root];
        let mut offenders = Vec::new();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") || p == this {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap();
                offenders.extend(sync_identifiers(&text).into_iter().map(|i| format!("{}:{}", p.display(), i + 1)));
            }
        }
        assert!(offenders.is_empty(), "a file sync outside crate::fsync (use sync_file by path, or sync_written for a handle opened for writing): {offenders:?}");
    }

    /// CP17 (CP16 review MEDIUM-1): the lines naming `sync_all`/`sync_data`
    /// as an identifier, however spaced or referenced (`.sync_all (`, a
    /// split line, `File::sync_all` with no call), not inside a longer
    /// identifier.
    fn sync_identifiers(text: &str) -> Vec<usize> {
        let words = [concat!("sync_", "all"), concat!("sync_", "data")];
        let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
        let mut out = Vec::new();
        for (i, l) in text.lines().enumerate() {
            let hit = words.iter().any(|w| {
                l.match_indices(w).any(|(at, _)| {
                    !l[..at].chars().next_back().is_some_and(ident) && !l[at + w.len()..].chars().next().is_some_and(ident)
                })
            });
            if hit {
                out.push(i);
            }
        }
        out
    }

    /// CP17: the scan catches the dodges the CP16 review listed, and not
    /// a longer identifier.
    #[test]
    fn sync_scan_catches_spaced_and_referenced_syncs() {
        let all = concat!("sync_", "all");
        for dodge in [format!("f.{all} ()"), format!("f.{all}\n    ()"), format!(".map(std::fs::File::{all})")] {
            assert_eq!(sync_identifiers(&dodge), vec![0], "{dodge}");
        }
        assert!(sync_identifiers(&format!("fn my_{all}_helper() {{}}")).is_empty());
    }

    /// CP17 (CP16 review LOW-6): `sync_regular` syncs the regular file
    /// checked, never a symlink's target.
    #[cfg(unix)]
    #[test]
    fn sync_regular_refuses_a_symlink() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let d = std::env::temp_dir().join(format!("axenstax-fsync-{}", hex::encode(r)));
        std::fs::create_dir_all(&d).unwrap();
        let real = d.join("member");
        std::fs::write(&real, b"x").unwrap();
        assert!(super::sync_regular(&real).is_ok());
        let link = d.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(super::sync_regular(&link).is_err(), "a symlink's target was synced");
        std::fs::remove_dir_all(d).unwrap();
    }
}

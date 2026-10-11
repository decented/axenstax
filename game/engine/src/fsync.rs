//! CP15 (CP14 review HIGH-1): the ONE place that re-syncs an existing file
//! or a directory by path. Every handover record, import and save-fence path
//! goes through these, never `File::open(..)` + `sync_all` of its own
//! (`tests::no_file_open_sync_outside_this_module` pins that).
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
    /// No `File::open(..)` followed (within the statement's next lines) by
    /// `sync_all`/`sync_data` anywhere in the engine's source but here.
    #[test]
    fn no_file_open_sync_outside_this_module() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root];
        let mut offenders = Vec::new();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") || p.file_name().is_some_and(|n| n == "fsync.rs") {
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
}

//! Bounded file streams; no archive-sized retained buffer.
use super::super::{Error, Hash, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
pub(super) const BUFFER: usize = 16 * 1024;
/// CP16: the regular-file check `open` makes, without opening.
pub(super) fn regular(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Ok(()),
        Ok(_) => Err(Error::Invalid("unsafe source file")),
        Err(_) => Err(Error::Io),
    }
}
pub(super) fn open(path: &Path) -> Result<File> {
    let before = fs::symlink_metadata(path).map_err(|_| Error::Io)?;
    if !before.is_file() {
        return Err(Error::Invalid("unsafe source file"));
    }
    let file = File::open(path).map_err(|_| Error::Io)?;
    let after = file.metadata().map_err(|_| Error::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(Error::Invalid("source file changed"));
        }
    }
    if !after.is_file() {
        return Err(Error::Invalid("source file type"));
    }
    Ok(file)
}
pub(super) fn new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|_| Error::Io)
}
pub(super) fn owned_dir(parent: &Path, prefix: &str) -> Result<PathBuf> {
    if !fs::symlink_metadata(parent)
        .map_err(|_| Error::Io)?
        .is_dir()
    {
        return Err(Error::Invalid("staging parent"));
    }
    for _ in 0..32 {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|_| Error::Io)?;
        let path = parent.join(format!(".{prefix}-{}", hex::encode(random)));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(Error::Io),
        }
    }
    Err(Error::Io)
}
pub(super) fn digest(mut reader: impl Read, limit: u64) -> Result<(u64, Hash)> {
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buf = [0u8; BUFFER];
    loop {
        let n = reader.read(&mut buf).map_err(|_| Error::Io)?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .ok_or(Error::Limit("file length"))?;
        if size > limit {
            return Err(Error::Limit("file bytes"));
        }
        hash.update(&buf[..n]);
    }
    Ok((size, hash.finalize().into()))
}
pub(super) struct Count<W> {
    pub inner: W,
    pub bytes: u64,
    pub limit: u64,
}
impl<W: Write> Write for Count<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if b.len() as u64 > self.limit.saturating_sub(self.bytes) {
            return Err(io::Error::other("checkpoint byte cap"));
        }
        let n = self.inner.write(b)?;
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
pub(super) fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut f = open(path)?;
    let len = usize::try_from(f.metadata().map_err(|_| Error::Io)?.len())
        .map_err(|_| Error::Limit("file length"))?;
    if len > limit {
        return Err(Error::Limit("member bytes"));
    }
    let mut bytes = vec![0; len];
    f.read_exact(&mut bytes).map_err(|_| Error::Io)?;
    let mut tail = [0; 1];
    if f.read(&mut tail).map_err(|_| Error::Io)? != 0 {
        return Err(Error::Invalid("file grew"));
    }
    Ok(bytes)
}
pub(super) fn sync_dir(path: &Path) -> Result<()> {
    crate::fsync::sync_dir(path).map_err(|_| Error::Io)
}

/// Atomic no-replace is necessary even for two imports racing within one host.
pub(super) fn rename_new(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|_| Error::Io)?;
        let to =
            std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(|_| Error::Io)?;
        // SAFETY: both NUL-terminated paths remain alive; flags forbid replacement.
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        // Bionic's renameat2 symbol requires API30, but the app supports API24.
        // The existing syscall symbol avoids a new dynamic API dependency;
        // older kernels return an error and no overwriting fallback is used.
        #[cfg(target_os = "android")]
        let result = unsafe {
            // SAFETY: kernel receives live terminated paths and no-replace flag.
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            return Ok(());
        }
        Err(Error::Io)
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|_| Error::Io)?;
        let to =
            std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(|_| Error::Io)?;
        // SAFETY: live terminated paths; RENAME_EXCL forbids existing targets.
        if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
            return Ok(());
        }
        Err(Error::Io)
    }
    #[cfg(target_os = "windows")]
    {
        // Directory rename on Windows refuses an already-existing directory/file.
        fs::rename(source, destination).map_err(|_| Error::Io)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "windows"
    )))]
    {
        let _ = (source, destination);
        Err(Error::State("atomic no-replace unavailable"))
    }
}

/// Hash the bytes actually handed to tar, so a source-file change between its
/// preliminary length check and stream read cannot finalise a false manifest.
pub(super) struct HashRead<R> {
    pub inner: R,
    pub bytes: u64,
    pub limit: u64,
    pub hash: Sha256,
}
impl<R: Read> Read for HashRead<R> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(b)?;
        self.bytes = self
            .bytes
            .checked_add(n as u64)
            .ok_or_else(|| io::Error::other("member length"))?;
        if self.bytes > self.limit {
            return Err(io::Error::other("member grew"));
        }
        self.hash.update(&b[..n]);
        Ok(n)
    }
}
impl<R> HashRead<R> {
    pub fn finish(self) -> (u64, Hash) {
        (self.bytes, self.hash.finalize().into())
    }
}

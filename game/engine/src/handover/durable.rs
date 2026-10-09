//! Narrow durable marker storage: no archive extraction or arbitrary filenames.
use super::{Error, Id, Result};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub const MARKER_LIMIT: usize = 64 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Retired,
    Ready,
    Activated,
    Prepared,
    Prefix,
}
impl MarkerKind {
    fn label(self) -> &'static str {
        match self {
            Self::Retired => "retired",
            Self::Ready => "ready",
            Self::Activated => "activated",
            Self::Prepared => "prepared",
            Self::Prefix => "prefix",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteBoundary {
    TempSynced,
    Renamed,
    DirectorySynced,
}
/// Injection observes real file operations; an error after rename is uncertain.
pub trait BoundaryObserver {
    fn reached(&mut self, boundary: WriteBoundary) -> std::io::Result<()>;
}
pub struct NoFault;
impl BoundaryObserver for NoFault {
    fn reached(&mut self, _: WriteBoundary) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct MarkerStore {
    directory: PathBuf,
}
impl MarkerStore {
    /// A dedicated runtime-local directory must not be included in transfer
    /// archives. Retirement is independent of world folders and autosave loads.
    pub fn new(directory: PathBuf) -> Result<Self> {
        if let Ok(meta) = fs::symlink_metadata(&directory)
            && (meta.file_type().is_symlink() || !meta.is_dir())
        {
            return Err(Error::Invalid("marker directory"));
        }
        fs::create_dir_all(&directory).map_err(|_| Error::Io)?;
        sync_directory(&directory)?;
        if let Some(parent) = directory.parent() {
            sync_directory(parent)?;
        }
        Ok(Self { directory })
    }
    fn path(&self, kind: MarkerKind, id: Id) -> PathBuf {
        self.directory
            .join(format!("{}-{}.dat", kind.label(), hex::encode(id)))
    }
    pub fn read(&self, kind: MarkerKind, id: Id) -> Result<Option<Vec<u8>>> {
        read_bounded(&self.path(kind, id))
    }
    pub fn persist(&self, kind: MarkerKind, id: Id, bytes: &[u8]) -> Result<()> {
        self.persist_observed(kind, id, bytes, &mut NoFault)
    }
    pub fn persist_observed(
        &self,
        kind: MarkerKind,
        id: Id,
        bytes: &[u8],
        observer: &mut impl BoundaryObserver,
    ) -> Result<()> {
        if id == [0; 16] || bytes.is_empty() || bytes.len() > MARKER_LIMIT {
            return Err(Error::Limit("marker record"));
        }
        let path = self.path(kind, id);
        if let Some(old) = read_bounded(&path)? {
            if old == bytes {
                fs::File::open(&path)
                    .and_then(|f| f.sync_all())
                    .map_err(|_| Error::UncertainDurability)?;
                sync_directory(&self.directory).map_err(|_| Error::UncertainDurability)?;
                observer
                    .reached(WriteBoundary::DirectorySynced)
                    .map_err(|_| Error::UncertainDurability)?;
                return Ok(());
            }
            if kind != MarkerKind::Prefix {
                return Err(Error::State("immutable marker conflict"));
            }
            let old = super::certificate::Prefix::restore(&old)?;
            let next = super::certificate::Prefix::restore(bytes)?;
            if !old.can_advance_to(&next) {
                return Err(Error::State("prefix rollback/conflict"));
            }
        } else {
            self.check_capacity(kind)?;
        }
        if kind == MarkerKind::Retired && self.retirements()?.len() >= 64 {
            return Err(Error::Limit("retired lineage records"));
        }
        let temp = self
            .directory
            .join(format!("{}.tmp-{}", kind.label(), hex::encode(id)));
        // Stale temporary contents are reused only if identical, never followed
        // through a symlink or rewritten to another commit under the same id.
        if let Some(old) = read_bounded(&temp)? {
            if old != bytes {
                return Err(Error::State("temporary marker conflict"));
            }
            fs::File::open(&temp)
                .and_then(|f| f.sync_all())
                .map_err(|_| Error::Io)?;
        } else {
            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&temp).map_err(|_| Error::Io)?;
            f.write_all(bytes)
                .and_then(|()| f.sync_all())
                .map_err(|_| Error::Io)?;
        }
        observer
            .reached(WriteBoundary::TempSynced)
            .map_err(|_| Error::Io)?;
        fs::rename(&temp, &path).map_err(|_| Error::Io)?;
        observer
            .reached(WriteBoundary::Renamed)
            .map_err(|_| Error::UncertainDurability)?;
        sync_directory(&self.directory).map_err(|_| Error::UncertainDurability)?;
        observer
            .reached(WriteBoundary::DirectorySynced)
            .map_err(|_| Error::UncertainDurability)?;
        Ok(())
    }
    fn check_capacity(&self, kind: MarkerKind) -> Result<()> {
        let mut total = 0usize;
        let mut same = 0usize;
        for entry in fs::read_dir(&self.directory).map_err(|_| Error::Io)? {
            total += 1;
            if total >= 256 {
                return Err(Error::Limit("marker directory capacity"));
            }
            let name = entry.map_err(|_| Error::Io)?.file_name();
            let name = name.to_str().ok_or(Error::Invalid("marker filename"))?;
            if name.starts_with(&format!("{}-", kind.label())) && name.ends_with(".dat") {
                same += 1;
            }
            if same >= 64 {
                return Err(Error::Limit("marker kind capacity"));
            }
        }
        Ok(())
    }
    /// Bounded startup scan. Corrupt/unreadable records fail closed; callers must
    /// validate payloads before opening any old lineage, including autosaves.
    pub fn retirements(&self) -> Result<Vec<(Id, Vec<u8>)>> {
        let mut out = Vec::new();
        let mut scanned = 0usize;
        for entry in fs::read_dir(&self.directory).map_err(|_| Error::Io)? {
            scanned += 1;
            if scanned > 256 {
                return Err(Error::Limit("marker directory entries"));
            }
            let entry = entry.map_err(|_| Error::Io)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(Error::Invalid("marker filename"))?;
            let Some(value) = name
                .strip_prefix("retired-")
                .and_then(|s| s.strip_suffix(".dat"))
            else {
                continue;
            };
            if out.len() >= 64 {
                return Err(Error::Limit("retired lineage records"));
            }
            let id: Id = hex::decode(value)
                .map_err(|_| Error::Invalid("retirement filename"))?
                .try_into()
                .map_err(|_| Error::Invalid("retirement filename"))?;
            if value != hex::encode(id) {
                return Err(Error::Invalid("noncanonical retirement filename"));
            }
            let bytes = read_bounded(&entry.path())?.ok_or(Error::Io)?;
            out.push((id, bytes));
        }
        Ok(out)
    }
}
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| Error::Io)
}
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::Io),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(Error::Invalid("marker file type"));
    }
    if meta.len() > MARKER_LIMIT as u64 {
        return Err(Error::Limit("marker file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| Error::Io)?
        .take(MARKER_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
    if bytes.is_empty() || bytes.len() > MARKER_LIMIT {
        return Err(Error::Invalid("marker size"));
    }
    Ok(Some(bytes))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn store(tag: &str) -> (MarkerStore, PathBuf) {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let p = std::env::temp_dir().join(format!("axe_handover_{tag}_{}", hex::encode(random)));
        (MarkerStore::new(p.clone()).unwrap(), p)
    }
    struct Fault(WriteBoundary);
    impl BoundaryObserver for Fault {
        fn reached(&mut self, b: WriteBoundary) -> std::io::Result<()> {
            if b == self.0 {
                Err(std::io::Error::other("injected"))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn handover_durable_before_after_rename_failure_and_exact_retry() {
        for boundary in [
            WriteBoundary::TempSynced,
            WriteBoundary::Renamed,
            WriteBoundary::DirectorySynced,
        ] {
            let (s, p) = store("fault");
            let err = s
                .persist_observed(
                    MarkerKind::Retired,
                    [1; 16],
                    b"exact Commit",
                    &mut Fault(boundary),
                )
                .unwrap_err();
            if boundary == WriteBoundary::TempSynced {
                assert_eq!(err, Error::Io);
                assert_eq!(s.read(MarkerKind::Retired, [1; 16]).unwrap(), None);
            } else {
                assert_eq!(err, Error::UncertainDurability);
                assert_eq!(
                    s.read(MarkerKind::Retired, [1; 16]).unwrap().unwrap(),
                    b"exact Commit"
                );
            }
            s.persist(MarkerKind::Retired, [1; 16], b"exact Commit")
                .unwrap();
            assert!(
                s.persist(MarkerKind::Retired, [1; 16], b"different")
                    .is_err()
            );
            let reopened = MarkerStore::new(p.clone()).unwrap();
            assert_eq!(reopened.retirements().unwrap().len(), 1);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn handover_durable_cap_refuses_new_record_without_eviction() {
        let (s, p) = store("cap");
        for n in 1..=64u8 {
            s.persist(MarkerKind::Retired, [n; 16], b"unchanged old record")
                .unwrap();
        }
        assert!(
            s.persist(MarkerKind::Retired, [65; 16], b"new record")
                .is_err()
        );
        assert_eq!(s.retirements().unwrap().len(), 64);
        assert_eq!(
            s.read(MarkerKind::Retired, [1; 16]).unwrap().unwrap(),
            b"unchanged old record"
        );
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn handover_durable_read_distinguishes_missing_empty_oversize() {
        let (s, p) = store("read");
        assert_eq!(s.read(MarkerKind::Retired, [1; 16]).unwrap(), None);
        fs::write(s.path(MarkerKind::Retired, [1; 16]), []).unwrap();
        assert!(s.read(MarkerKind::Retired, [1; 16]).is_err());
        fs::write(
            s.path(MarkerKind::Retired, [1; 16]),
            vec![0; MARKER_LIMIT + 1],
        )
        .unwrap();
        assert!(s.retirements().is_err());
        fs::remove_dir_all(p).unwrap();
    }
}

//! Narrow durable marker storage: no archive extraction or arbitrary filenames.
use super::{
    Error, Id, Result,
    codec::{Reader, frame},
};
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
    /// CP9: a retired source's "Committed published" record.
    Published,
    /// CP10: a recipient past its PreparedAck heard the source's
    /// authenticated pre-retirement Abort: no Committed will ever exist, so
    /// the wait for it ends (the import stays, quarantined, never opened).
    Quarantined,
    /// CP13b (D2): a source's abort record after it disclosed the bearer:
    /// the exact signed Aborted and the root's relays (`phase::AbortRecord`).
    /// Exclusive with `Retired` for one transfer (`phase` writes either).
    Aborted,
}
impl MarkerKind {
    fn number(self) -> u8 {
        match self {
            Self::Retired => 0,
            Self::Ready => 1,
            Self::Activated => 2,
            Self::Prepared => 3,
            Self::Prefix => 4,
            Self::Published => 5,
            Self::Quarantined => 6,
            Self::Aborted => 7,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Retired => "retired",
            Self::Ready => "ready",
            Self::Activated => "activated",
            Self::Prepared => "prepared",
            Self::Prefix => "prefix",
            Self::Published => "published",
            Self::Quarantined => "quarantined",
            Self::Aborted => "aborted",
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
        create_private_dir(&directory)?;
        sync_directory(&directory)?;
        if let Some(parent) = directory.parent() {
            sync_directory(parent)?;
        }
        Ok(Self { directory })
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    fn path(&self, kind: MarkerKind, id: Id) -> PathBuf {
        self.directory
            .join(format!("{}-{}.dat", kind.label(), hex::encode(id)))
    }
    /// CP14 (CP10 review LOW-4): the payload is zeroed on drop (a receipt
    /// or Activated record holds the bearer), and so is the raw file buffer.
    pub fn read(&self, kind: MarkerKind, id: Id) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>> {
        read_bounded(&self.path(kind, id))?
            .map(|bytes| decode_marker(&bytes, kind, id).map(zeroize::Zeroizing::new))
            .transpose()
    }
    /// CP14 (CP10 review LOW-4): whether a record of `kind` exists, without
    /// reading or decoding it (a symlink or a non-file counts, failing
    /// closed). For checks that don't need the content.
    pub fn exists(&self, kind: MarkerKind, id: Id) -> Result<bool> {
        match fs::symlink_metadata(self.path(kind, id)) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(Error::Io),
        }
    }
    /// CP13: remove one record (a missing one is fine), then fsync the
    /// directory so the removal is durable.
    pub fn remove(&self, kind: MarkerKind, id: Id) -> Result<()> {
        match fs::remove_file(self.path(kind, id)) {
            Ok(()) => sync_directory(&self.directory),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(Error::Io),
        }
    }
    pub fn persist(&self, kind: MarkerKind, id: Id, bytes: &[u8]) -> Result<()> {
        #[cfg(test)]
        if let Some(at) = fault::take(kind) {
            return self.persist_observed(kind, id, bytes, &mut fault::At(at));
        }
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
        let payload = bytes;
        let encoded = zeroize::Zeroizing::new(encode_marker(kind, id, payload)?);
        let bytes = encoded.as_slice();
        let path = self.path(kind, id);
        let existing = match read_bounded(&path) {
            // CP11: an undecodable "Committed published" record (empty or
            // over the size limit, so it can't even be compared) is replaced
            // like a mismatched one, by the one write below.
            Err(Error::Invalid("marker size") | Error::Limit("marker file")) if kind == MarkerKind::Published => {
                fs::remove_file(&path).map_err(|_| Error::Io)?;
                None
            }
            r => r?,
        };
        if let Some(old) = existing {
            if old.as_slice() == bytes {
                fs::File::open(&path)
                    .and_then(|f| f.sync_all())
                    .map_err(|_| Error::UncertainDurability)?;
                sync_directory(&self.directory).map_err(|_| Error::UncertainDurability)?;
                observer
                    .reached(WriteBoundary::DirectorySynced)
                    .map_err(|_| Error::UncertainDurability)?;
                return Ok(());
            }
            // CP9 review LOW-2: "Committed published" is a derived fact, not
            // authority: a record that doesn't match (or doesn't decode) is
            // rewritten with the exact deterministic bytes. CP11: by ONE
            // fresh write in this call (no retry or recursion here); a failed
            // write returns, and the caller's attempt backoff paces the next.
            if kind == MarkerKind::Published {
                fs::remove_file(&path).map_err(|_| Error::Io)?;
                self.check_capacity(kind)?;
            } else if kind != MarkerKind::Prefix {
                return Err(Error::State("immutable marker conflict"));
            } else {
                let old_payload = decode_marker(&old, kind, id)?;
                let old = super::certificate::Prefix::restore(&old_payload)?;
                let next = super::certificate::Prefix::restore(payload)?;
                if !old.can_advance_to(&next) {
                    return Err(Error::State("prefix rollback/conflict"));
                }
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
        // Stale temporary contents are reused only if identical and never
        // followed through a symlink. CP9 review LOW-3 (coordinator: any
        // kind): a stale regular temp file with other (or partial, or empty)
        // bytes never jams a write; it was never renamed, so nothing durable
        // claims it, and it is replaced. A temp path that isn't a regular file
        // still refuses.
        // CP11: the comparison read is bounded by the record size limit, and
        // only a temp of exactly this record's length is read at all.
        let stale = match fs::symlink_metadata(&temp) {
            Ok(m) if m.is_file() => Some(m.len()),
            Ok(_) => return Err(Error::Invalid("marker file type")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(Error::Io),
        };
        let reuse = stale == Some(bytes.len() as u64) && read_stale_temp(&temp).is_some_and(|old| old == bytes);
        let stale = stale.is_some();
        if stale && !reuse {
            fs::remove_file(&temp).map_err(|_| Error::Io)?;
        }
        if reuse {
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
    /// P3: an inert import's Ready marker goes with the import it describes
    /// (pre-Prepared abort, timeout or sweep). Refused once a Prepared or
    /// Activated marker exists for the same transfer: those are never
    /// discarded. Missing is fine (idempotent).
    pub fn discard_inert_ready(&self, id: Id) -> Result<()> {
        if read_bounded(&self.path(MarkerKind::Prepared, id))?.is_some()
            || read_bounded(&self.path(MarkerKind::Activated, id))?.is_some()
        {
            return Err(Error::State("marker is no longer inert"));
        }
        match fs::remove_file(self.path(MarkerKind::Ready, id)) {
            Ok(()) => sync_directory(&self.directory),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(Error::Io),
        }
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
        self.records(MarkerKind::Retired)
    }
    /// CP13b (D2): every abort record (bounded like `retirements`).
    pub fn aborts(&self) -> Result<Vec<(Id, Vec<u8>)>> {
        self.records(MarkerKind::Aborted)
    }
    fn records(&self, kind: MarkerKind) -> Result<Vec<(Id, Vec<u8>)>> {
        let prefix = format!("{}-", kind.label());

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
                .strip_prefix(prefix.as_str())
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
            let mut bytes = self.read(kind, id)?.ok_or(Error::Io)?;
            out.push((id, std::mem::take(&mut *bytes)));
        }
        Ok(out)
    }
}
/// Read-only open guard (CP6 review LOW-5): the payload of a retirement
/// marker's bytes, read by the caller without a `MarkerStore`.
pub fn retirement_payload(bytes: &[u8], id: Id) -> Result<Vec<u8>> {
    decode_marker(bytes, MarkerKind::Retired, id)
}
/// CP10: the same read-only decode for any marker kind (the open guard's
/// Activated check).
pub fn marker_payload(bytes: &[u8], kind: MarkerKind, id: Id) -> Result<Vec<u8>> {
    decode_marker(bytes, kind, id)
}
const MARKER_DOMAIN: &str = "AxeNStax/handover/marker/v1";
const MARKER_OVERHEAD: usize = 4 + MARKER_DOMAIN.len() + 4 + 1 + 16 + 4 + 32;
/// Unkeyed accidental-corruption checksum, not a machineowner authentication
/// boundary. Full encoded record, including all framing and checksum, is64KiB.
fn encode_marker(kind: MarkerKind, id: Id, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.is_empty() || payload.len() > MARKER_LIMIT - MARKER_OVERHEAD {
        return Err(Error::Limit("full marker record"));
    }
    let mut out = Vec::with_capacity(MARKER_OVERHEAD + payload.len());
    frame(&mut out, MARKER_DOMAIN.as_bytes());
    out.extend_from_slice(&1u32.to_be_bytes());
    out.push(kind.number());
    out.extend_from_slice(&id);
    frame(&mut out, payload);
    let checksum = super::hash(&out);
    out.extend_from_slice(&checksum);
    Ok(out)
}
fn decode_marker(bytes: &[u8], kind: MarkerKind, id: Id) -> Result<Vec<u8>> {
    if bytes.len() > MARKER_LIMIT || bytes.len() < MARKER_OVERHEAD {
        return Err(Error::Invalid("marker envelope size"));
    }
    let (body, checksum) = bytes.split_at(bytes.len() - 32);
    if super::hash(body).as_slice() != checksum {
        return Err(Error::Invalid("marker checksum"));
    }
    let mut r = Reader::new(body, MARKER_LIMIT)?;
    r.label(MARKER_DOMAIN)?;
    if r.u32()? != 1 || r.u8()? != kind.number() || r.array::<16>()? != id {
        return Err(Error::Invalid("marker envelope binding"));
    }
    let payload = r.framed(MARKER_LIMIT - MARKER_OVERHEAD)?;
    r.end()?;
    if payload.is_empty() {
        return Err(Error::Invalid("marker payload"));
    }
    Ok(payload.to_vec())
}

pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    // Windows can't open a directory to fsync it; the rename before this is
    // NTFS-journaled, as main's saves rely on (C3d). Not compiled here.
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
    #[cfg(not(windows))]
    fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| Error::Io)
}
/// A stale temp's bytes, never more than `MARKER_LIMIT` (+1 to tell an
/// over-long one apart), or `None` if it can't be read.
fn read_stale_temp(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path).ok()?.take(MARKER_LIMIT as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= MARKER_LIMIT).then_some(bytes)
}
fn read_bounded(path: &Path) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>> {
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
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
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

/// CP14 (CP10 review LOW-4): a record folder is owner-only (0700), so
/// other accounts can't list the record names (the records are 0600). Its
/// parents are created as usual; an existing folder is tightened.
fn create_private_dir(dir: &Path) -> Result<()> {
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent).map_err(|_| Error::Io)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        match fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(Error::Io),
        }
        let meta = fs::symlink_metadata(dir).map_err(|_| Error::Io)?;
        if !meta.is_dir() {
            return Err(Error::Invalid("marker directory"));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Io)?;
        }
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir).map_err(|_| Error::Io)?;
    Ok(())
}

/// Test seam: fail the next [`MarkerStore::persist`] of a kind on THIS
/// thread at a write boundary (before the rename: `Io`; after: uncertain).
#[cfg(test)]
pub(crate) mod fault {
    use super::{MarkerKind, WriteBoundary};
    use std::cell::Cell;
    thread_local! {
        static NEXT: Cell<Option<(MarkerKind, WriteBoundary)>> = const { Cell::new(None) };
    }
    pub(crate) fn arm(kind: MarkerKind, at: WriteBoundary) {
        NEXT.with(|n| n.set(Some((kind, at))));
    }
    pub(super) fn take(kind: MarkerKind) -> Option<WriteBoundary> {
        NEXT.with(|n| match n.get() {
            Some((k, at)) if k == kind => {
                n.set(None);
                Some(at)
            }
            _ => None,
        })
    }
    pub(super) struct At(pub(super) WriteBoundary);
    impl super::BoundaryObserver for At {
        fn reached(&mut self, b: WriteBoundary) -> std::io::Result<()> {
            if b == self.0 { Err(std::io::Error::other("injected")) } else { Ok(()) }
        }
    }
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
                    s.read(MarkerKind::Retired, [1; 16]).unwrap().unwrap().as_slice(),
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
            s.read(MarkerKind::Retired, [1; 16]).unwrap().unwrap().as_slice(),
            b"unchanged old record"
        );
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn handover_marker_full_size_kind_id_version_trailing_and_corruption() {
        let payload = vec![1; MARKER_LIMIT - MARKER_OVERHEAD];
        let encoded = encode_marker(MarkerKind::Activated, [1; 16], &payload).unwrap();
        assert_eq!(encoded.len(), MARKER_LIMIT);
        assert_eq!(encoded.capacity(), MARKER_LIMIT);
        assert_eq!(
            decode_marker(&encoded, MarkerKind::Activated, [1; 16]).unwrap(),
            payload
        );
        assert!(
            encode_marker(MarkerKind::Activated, [1; 16], &vec![1; payload.len() + 1]).is_err()
        );
        assert!(decode_marker(&encoded, MarkerKind::Retired, [1; 16]).is_err());
        assert!(decode_marker(&encoded, MarkerKind::Activated, [2; 16]).is_err());
        let mut trailing = encode_marker(MarkerKind::Activated, [1; 16], b"payload").unwrap();
        trailing.push(0);
        assert!(decode_marker(&trailing, MarkerKind::Activated, [1; 16]).is_err());
        let mut future = encode_marker(MarkerKind::Activated, [1; 16], b"payload").unwrap();
        let version_at = 4 + MARKER_DOMAIN.len();
        future[version_at + 3] = 2;
        let end = future.len() - 32;
        let checksum = super::super::hash(&future[..end]);
        future[end..].copy_from_slice(&checksum);
        assert!(decode_marker(&future, MarkerKind::Activated, [1; 16]).is_err());
        let (s, p) = store("all-kind-corruption");
        for kind in [
            MarkerKind::Retired,
            MarkerKind::Ready,
            MarkerKind::Activated,
            MarkerKind::Prepared,
            MarkerKind::Prefix,
            MarkerKind::Published,
            MarkerKind::Quarantined,
            MarkerKind::Aborted,
        ] {
            let id = [kind.number() + 1; 16];
            s.persist(kind, id, b"exact phase binding").unwrap();
            let path = s.path(kind, id);
            let mut bytes = fs::read(&path).unwrap();
            let offset = bytes.len() - 33;
            bytes[offset] ^= 1;
            fs::write(&path, bytes).unwrap();
            assert!(s.read(kind, id).is_err());
        }
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

    /// CP11: an undecodable "Committed published" record (garbage, empty or
    /// over the size limit) is replaced by exactly ONE fresh write per call
    /// (the caller's attempt backoff paces any next one); a directory at its
    /// path still refuses.
    #[test]
    fn handover_undecodable_published_record_is_replaced_by_one_write() {
        struct Count(usize);
        impl BoundaryObserver for Count {
            fn reached(&mut self, b: WriteBoundary) -> std::io::Result<()> {
                if b == WriteBoundary::TempSynced {
                    self.0 += 1;
                }
                Ok(())
            }
        }
        let (s, p) = store("published-replace");
        let id = [7; 16];
        let payload = b"committed published".to_vec();
        for planted in [b"garbage".to_vec(), Vec::new(), vec![0xAB; MARKER_LIMIT + 1]] {
            fs::write(s.path(MarkerKind::Published, id), &planted).unwrap();
            let mut count = Count(0);
            s.persist_observed(MarkerKind::Published, id, &payload, &mut count)
                .unwrap_or_else(|e| panic!("a {}-byte undecodable published record wasn't replaced: {e:?}", planted.len()));
            assert_eq!(count.0, 1, "a {}-byte undecodable published record took {} writes", planted.len(), count.0);
            assert_eq!(s.read(MarkerKind::Published, id).unwrap().unwrap().as_slice(), payload);
        }
        fs::remove_file(s.path(MarkerKind::Published, id)).unwrap();
        fs::create_dir(s.path(MarkerKind::Published, id)).unwrap();
        assert!(s.persist(MarkerKind::Published, id, &payload).is_err(), "a directory at the record's path was replaced");
        fs::remove_dir_all(p).unwrap();
    }

    /// CP11: a stale temp larger than any record is never compared (its
    /// length differs) and is replaced; the record lands.
    #[test]
    fn handover_oversized_stale_temp_is_replaced_and_the_record_lands() {
        let (s, p) = store("stale-oversized");
        let id = [8; 16];
        let temp = s.directory.join(format!("{}.tmp-{}", MarkerKind::Ready.label(), hex::encode(id)));
        fs::write(&temp, vec![0xCD; MARKER_LIMIT * 4]).unwrap();
        s.persist(MarkerKind::Ready, id, b"ready record").unwrap();
        assert_eq!(s.read(MarkerKind::Ready, id).unwrap().unwrap().as_slice(), b"ready record");
        assert!(!temp.exists(), "the stale temp outlived the write");
        fs::remove_dir_all(p).unwrap();
    }
}

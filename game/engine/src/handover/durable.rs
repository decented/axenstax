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
    /// CP15: the source's PENDING abort record, written (its ledger line
    /// first) before the bearer leaves: the same `phase::AbortRecord`
    /// bytes, sealed with the Prepared. Removed once a terminal record of
    /// the transfer is durable; promoted to `Aborted` at the next start-up
    /// when none is (`phase::promote_pending_aborts`). Its label is no
    /// prefix of another kind's (the scans split names on `<label>-`).
    PendingAbort,
    /// An inactive source-retained copy; never admits gameplay.
    RetainedInert,
    /// Signed personal provenance for a distinct source-retained copy.
    RetainedCopy,
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
            Self::PendingAbort => 8,
            Self::RetainedInert => 9,
            Self::RetainedCopy => 10,
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
            Self::PendingAbort => "pending-abort",
            Self::RetainedInert => "retained-inert",
            Self::RetainedCopy => "retained-copy",
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

/// CP17: the entries one handover's records take in its world's folder
/// (pending abort, a terminal record, published).
pub const HANDOVER_ENTRIES: usize = 3;

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
    /// CP23 (CP22 review P-2): an existing marker directory as it is: no
    /// directory is made and nothing is synced (an inert discard reads and
    /// removes; it never creates what it is about to delete).
    pub fn open_existing(directory: PathBuf) -> Result<Self> {
        match fs::symlink_metadata(&directory) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(Self { directory }),
            Ok(_) => Err(Error::Invalid("marker directory")),
            Err(_) => Err(Error::Io),
        }
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
                crate::fsync::sync_file(&path).map_err(|_| Error::UncertainDurability)?;
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
        let reuse = stale == Some(bytes.len() as u64) && read_stale_temp(&temp, bytes.len() as u64).is_some_and(|old| old.as_slice() == bytes);
        let stale = stale.is_some();
        if stale && !reuse {
            fs::remove_file(&temp).map_err(|_| Error::Io)?;
        }
        // CP16 (CP14 review LOW-2): from here until the rename lands, a
        // certain failure removes the temp (best effort, then a directory
        // fsync): a never-renamed record (a retirement's signed Commit, a
        // Prepared's bearer) must not linger beside a world that carries on.
        let written = self.write_temp(&temp, bytes, reuse).and_then(|()| {
            observer.reached(WriteBoundary::TempSynced).map_err(|_| Error::Io)?;
            fs::rename(&temp, &path).map_err(|_| Error::Io)
        });
        if let Err(e) = written {
            if fs::remove_file(&temp).is_ok() {
                let _ = sync_directory(&self.directory);
            }
            return Err(e);
        }
        observer
            .reached(WriteBoundary::Renamed)
            .map_err(|_| Error::UncertainDurability)?;
        sync_directory(&self.directory).map_err(|_| Error::UncertainDurability)?;
        observer
            .reached(WriteBoundary::DirectorySynced)
            .map_err(|_| Error::UncertainDurability)?;
        Ok(())
    }
    /// The temp of `persist`: an identical stale one fsynced again, or a
    /// fresh owner-only file written and fsynced.
    fn write_temp(&self, temp: &Path, bytes: &[u8], reuse: bool) -> Result<()> {
        if reuse {
            crate::fsync::sync_file(temp).map_err(|_| Error::Io)?;
        } else {
            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = crate::fsync::open_written(&mut opts, temp).map_err(|_| Error::Io)?;
            f.write_all(bytes)
                .and_then(|()| crate::fsync::sync_written(&f))
                .map_err(|_| Error::Io)?;
        }
        Ok(())
    }
    /// CP16 (CP14 review LOW-2): the start-up sweep. Every `*.tmp-*` here
    /// was never renamed (so nothing durable claims it), and at start-up no
    /// session can be writing one: each regular one is removed (never
    /// through a symlink; anything that isn't a regular file is left alone).
    /// Bounded like the other scans. Returns how many were removed.
    pub fn sweep_temps(&self) -> usize {
        let Ok(entries) = fs::read_dir(&self.directory) else { return 0 };
        let mut removed = 0;
        for entry in entries.take(super::live::inert::SCAN_LIMIT).flatten() {
            if !entry.file_name().to_str().is_some_and(|n| n.contains(".tmp-")) {
                continue;
            }
            let path = entry.path();
            if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        if removed > 0 {
            let _ = sync_directory(&self.directory);
            log::info!("handover: removed {removed} unfinished record(s) under {}", self.directory.display());
        }
        removed
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
    /// CP16 (CP15 review MEDIUM-1): whether this folder can still take
    /// one record of each kind in `kinds` (by `check_capacity`'s own rule:
    /// fewer than 64 of a kind, fewer than 256 entries before each write)
    /// and `files` more entries in all. Checked before the bearer leaves, so
    /// a disclosed transfer can always write the records it may still need.
    pub fn has_room(&self, kinds: &[MarkerKind], files: usize) -> Result<bool> {
        let mut total = 0usize;
        let mut same = vec![0usize; kinds.len()];
        for entry in fs::read_dir(&self.directory).map_err(|_| Error::Io)? {
            total += 1;
            if total + files > 256 {
                return Ok(false);
            }
            let name = entry.map_err(|_| Error::Io)?.file_name();
            let name = name.to_str().ok_or(Error::Invalid("marker filename"))?;
            for (n, kind) in same.iter_mut().zip(kinds) {
                if name.starts_with(&format!("{}-", kind.label())) && name.ends_with(".dat") {
                    *n += 1;
                }
            }
        }
        Ok(kinds.iter().zip(&same).all(|(k, n)| n + kinds.iter().filter(|x| *x == k).count() <= 64))
    }
    /// CP17 (CP16 review LOW-1, LOW-5): room for one more handover in this
    /// world, the ONE rule both the session start and the disclosure use.
    /// The new transfer needs a PendingAbort, a terminal record of either
    /// kind (Retired or Aborted) and a Published: `HANDOVER_ENTRIES` (3)
    /// entries. Every pending abort record already here (an earlier
    /// transfer's, not yet promoted: a promotion that failed at start-up)
    /// is a future Aborted and Published too, so each reserves one more of
    /// each kind and two more entries: a later promotion never meets the cap.
    pub fn has_room_for_handover(&self) -> Result<bool> {
        use MarkerKind::{Aborted, PendingAbort, Published, Retired};
        let pending = self.count(PendingAbort)?;
        let mut kinds = vec![PendingAbort, Retired];
        for _ in 0..=pending {
            kinds.extend([Aborted, Published]);
        }
        self.has_room(&kinds, HANDOVER_ENTRIES + 2 * pending)
    }
    /// How many `<kind>-*.dat` names this folder holds (bounded like the
    /// other scans: more than 256 entries is a `Limit`).
    fn count(&self, kind: MarkerKind) -> Result<usize> {
        let mut n = 0;
        for (seen, entry) in fs::read_dir(&self.directory).map_err(|_| Error::Io)?.enumerate() {
            if seen >= 256 {
                return Err(Error::Limit("marker directory entries"));
            }
            let name = entry.map_err(|_| Error::Io)?.file_name();
            let name = name.to_str().ok_or(Error::Invalid("marker filename"))?;
            if name.starts_with(&format!("{}-", kind.label())) && name.ends_with(".dat") {
                n += 1;
            }
        }
        Ok(n)
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
    /// CP15: every pending abort record (bounded like `retirements`).
    pub fn pending_aborts(&self) -> Result<Vec<(Id, Vec<u8>)>> {
        self.records(MarkerKind::PendingAbort)
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
    // Windows can't open a directory to fsync it (a no-op there, see
    // `crate::fsync`); the rename before this is NTFS-journaled.
    crate::fsync::sync_dir(path).map_err(|_| Error::Io)
}
/// A stale temp's bytes, never more than `MARKER_LIMIT` (+1 to tell an
/// over-long one apart), or `None` if it can't be read.
/// CP16 (CP14 review LOW-3): zeroed on drop (a stale Prepared or Activated
/// temp holds the bearer).
fn read_stale_temp(path: &Path, len: u64) -> Option<zeroize::Zeroizing<Vec<u8>>> {
    let bytes = read_zeroized(path, len).ok()?;
    (bytes.len() as u64 == len && bytes.len() <= MARKER_LIMIT).then_some(bytes)
}
/// CP16 (CP14 review LOW-3): a file `len` bytes long (by its metadata),
/// read into one `Zeroizing` buffer allocated before the read: at most
/// `len + 1` bytes are read into `len + 1` of capacity, so `read_to_end`
/// never reallocates (each reallocation would free an unzeroed copy of a
/// prefix), and a read that fails part-way drops a zeroed buffer. A length
/// of `len + 1` tells the caller the file grew under the read.
pub(crate) fn read_zeroized(path: &Path, len: u64) -> std::io::Result<zeroize::Zeroizing<Vec<u8>>> {
    let cap = usize::try_from(len).ok().and_then(|n| n.checked_add(1)).ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(cap));
    fs::File::open(path)?.take(len + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
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
    let bytes = read_zeroized(path, meta.len()).map_err(|_| Error::Io)?;
    if bytes.is_empty() || bytes.len() > MARKER_LIMIT || bytes.len() as u64 != meta.len() {
        return Err(Error::Invalid("marker size"));
    }
    Ok(Some(bytes))
}

/// CP14 (CP10 review LOW-4): a record folder is owner-only (0700), so
/// other accounts can't list the record names (the records are 0600). Its
/// parents are created as usual; an existing folder is tightened.
pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
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

    /// CP17 (CP16 review LOW-5): the handover room rule's parts, each at
    /// its edge: 63 of any of the four kinds leaves room, 64 doesn't; 253
    /// other entries leave room for the 3, 254 don't; and every pending
    /// record already here reserves an Aborted and a Published of its own.
    #[test]
    fn handover_room_rule_pins_every_kind_and_the_entry_reservation() {
        let fill = |s: &MarkerStore, prefix: &str, n: usize| {
            for i in 0..n {
                std::fs::write(s.directory().join(format!("{prefix}-{i:032x}.dat")), b"").unwrap();
            }
        };
        for kind in [MarkerKind::PendingAbort, MarkerKind::Retired, MarkerKind::Aborted, MarkerKind::Published] {
            let (s, p) = store("room-kind");
            fill(&s, kind.label(), 63);
            assert_eq!(s.has_room_for_handover(), Ok(true), "63 {} left no room", kind.label());
            fill(&s, &format!("{}-x", kind.label()), 1);
            assert_eq!(s.has_room_for_handover(), Ok(false), "64 {} still left room", kind.label());
            std::fs::remove_dir_all(p).unwrap();
        }
        let (s, p) = store("room-entries");
        fill(&s, "other", 253);
        assert_eq!(s.has_room_for_handover(), Ok(true), "253 entries left no room for 3 more");
        fill(&s, "another", 1);
        assert_eq!(s.has_room_for_handover(), Ok(false), "254 entries still left room for 3 more");
        std::fs::remove_dir_all(p).unwrap();
        // LOW-1: 62 Aborted + one earlier pending record: its promotion
        // takes the 63rd, this handover's abort the 64th: room. 63 Aborted
        // + one pending: no room (the promotion would meet the cap).
        let (s, p) = store("room-pending");
        fill(&s, MarkerKind::Aborted.label(), 62);
        fill(&s, MarkerKind::PendingAbort.label(), 1);
        assert_eq!(s.has_room_for_handover(), Ok(true));
        fill(&s, "aborted-x", 1);
        assert_eq!(s.has_room_for_handover(), Ok(false), "an earlier pending record's Aborted wasn't reserved");
        std::fs::remove_dir_all(p).unwrap();
    }
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
                // CP16 (CP14 review LOW-2): a certain failure leaves no temp.
                let temp = s.directory.join(format!("{}.tmp-{}", MarkerKind::Retired.label(), hex::encode([1u8; 16])));
                assert!(!temp.exists(), "a certain failure left the record's temp behind");
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
            MarkerKind::PendingAbort,
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

    /// CP16 (CP14 review LOW-3): the bounded reads fill one buffer sized
    /// before the read (never reallocated); a stale temp is read zeroised.
    #[test]
    fn handover_bounded_reads_never_reallocate() {
        let (s, p) = store("zeroized");
        let id = [10; 16];
        s.persist(MarkerKind::Ready, id, b"ready record").unwrap();
        let path = s.path(MarkerKind::Ready, id);
        let len = fs::metadata(&path).unwrap().len() as usize;
        let read = read_bounded(&path).unwrap().unwrap();
        assert_eq!(read.capacity(), len + 1, "the bounded read reallocated its buffer");
        let temp = s.directory.join("ready.tmp-x");
        fs::write(&temp, b"bearer-ish").unwrap();
        let stale: zeroize::Zeroizing<Vec<u8>> = read_stale_temp(&temp, 10).unwrap();
        assert_eq!(stale.capacity(), 11, "the stale-temp read reallocated its buffer");
        assert!(read_stale_temp(&temp, 9).is_none(), "a temp longer than its metadata said was accepted");
        fs::remove_dir_all(p).unwrap();
    }

    /// CP16 (CP14 review LOW-2): the start-up sweep removes every regular
    /// `*.tmp-*` and leaves records, and anything that isn't a regular file.
    #[test]
    fn handover_start_up_sweep_removes_stale_temps_only() {
        let (s, p) = store("sweep");
        let id = [9; 16];
        s.persist(MarkerKind::Ready, id, b"ready record").unwrap();
        let retired = s.directory.join(format!("{}.tmp-{}", MarkerKind::Retired.label(), hex::encode(id)));
        let prepared = s.directory.join(format!("{}.tmp-{}", MarkerKind::Prepared.label(), hex::encode(id)));
        fs::write(&retired, b"a signed Commit").unwrap();
        fs::write(&prepared, b"a bearer").unwrap();
        let odd = s.directory.join(format!("{}.tmp-{}", MarkerKind::Aborted.label(), hex::encode(id)));
        fs::create_dir(&odd).unwrap();
        assert_eq!(s.sweep_temps(), 2);
        assert!(!retired.exists() && !prepared.exists(), "a stale temp survived the sweep");
        assert!(odd.is_dir(), "the sweep removed something that isn't a regular file");
        assert_eq!(s.read(MarkerKind::Ready, id).unwrap().unwrap().as_slice(), b"ready record");
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

//! CP13 (CP10 review MEDIUM-2 + LOW-1): the install-local handover ledger.
//!
//! One append-only file (0600) in the install's data dir, never inside a world
//! or an import folder, so a world zip can never carry it. Each line lists one
//! durable handover record this install wrote: its transfer id, the role
//! (source or recipient), the record kind (retirement, abort or receipt) and
//! the SHA-256 of the record's exact payload bytes (what `MarkerStore::persist`
//! is handed, and what `MarkerStore::read` returns). The entry is written
//! durably (append + fsync, plus a directory fsync when the file is created)
//! BEFORE the record it authorises; the resumers act only on a record whose
//! exact digest is listed for that transfer and role. It replaces "is this
//! record signed by my runtime key?" as the adoption gate: a re-minted key
//! strands nothing, and a planted record (even one wrapping this install's
//! own signed event) is never acted on.
//!
//! Each line carries its own checksum: a torn or damaged line is skipped and
//! the rest are kept. Reads are bounded (entries and bytes) and fail closed
//! past the bound (nothing listed).
use super::{Error, Hash, Id, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The most entries the ledger holds; past it, reads and appends fail closed.
pub const MAX_ENTRIES: usize = 8192;
/// One line: `v1 <transfer 32 hex> <s|r> <ret|abt|rcp> <digest 64 hex> <check 16 hex>\n`.
const LINE: usize = 3 + 32 + 1 + 1 + 1 + 3 + 1 + 64 + 1 + 16 + 1;
/// The byte bound: every entry whole, plus one torn fragment's worth.
pub const MAX_BYTES: u64 = ((MAX_ENTRIES + 1) * LINE) as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Source,
    Recipient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    Retirement,
    Abort,
    Receipt,
}

/// One listed record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub transfer: Id,
    pub role: Role,
    pub kind: RecordKind,
    pub digest: Hash,
}

impl Entry {
    /// The entry for a record whose exact payload is `record`.
    pub fn of(transfer: Id, role: Role, kind: RecordKind, record: &[u8]) -> Self {
        Self { transfer, role, kind, digest: super::hash(record) }
    }
    fn body(&self) -> String {
        let role = match self.role {
            Role::Source => "s",
            Role::Recipient => "r",
        };
        let kind = match self.kind {
            RecordKind::Retirement => "ret",
            RecordKind::Abort => "abt",
            RecordKind::Receipt => "rcp",
        };
        format!("v1 {} {role} {kind} {}", hex::encode(self.transfer), hex::encode(self.digest))
    }
    fn line(&self) -> String {
        let body = self.body();
        let check = hex::encode(&super::hash(body.as_bytes())[..8]);
        format!("{body} {check}\n")
    }
    /// A whole, checksummed line, or `None` (torn, damaged or unknown).
    fn parse(line: &[u8]) -> Option<Self> {
        if line.len() != LINE - 1 {
            return None;
        }
        let line = std::str::from_utf8(line).ok()?;
        let mut parts = line.split(' ');
        let (v, transfer, role, kind, digest, check) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if v != "v1" || parts.next().is_some() {
            return None;
        }
        let entry = Self {
            transfer: hex::decode(transfer).ok()?.try_into().ok()?,
            role: match role {
                "s" => Role::Source,
                "r" => Role::Recipient,
                _ => return None,
            },
            kind: match kind {
                "ret" => RecordKind::Retirement,
                "abt" => RecordKind::Abort,
                "rcp" => RecordKind::Receipt,
                _ => return None,
            },
            digest: hex::decode(digest).ok()?.try_into().ok()?,
        };
        // Canonical lowercase hex only: the re-rendered line must match.
        (entry.line().trim_end_matches('\n') == line && check.len() == 16).then_some(entry)
    }
}

/// One process-wide lock: appends from several sessions (and parallel tests)
/// never interleave inside a line.
static LOCK: Mutex<()> = Mutex::new(());

/// The ledger file. `install()` is this install's; tests use `at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }
    /// `<data root>/handover/ledger`: beside (never inside) the imports
    /// folder, and outside every world.
    pub fn install() -> Self {
        Self::at(crate::data_dir::data_root().join("handover").join("ledger"))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append `entry` durably: one whole line, fsynced, and the directory
    /// fsynced when this append created the file. A torn last line gets its
    /// own end first, so this entry stays whole. Past the bound: refused.
    pub fn append(&self, entry: Entry) -> Result<()> {
        let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.path.parent().ok_or(Error::Io)?;
        std::fs::create_dir_all(dir).map_err(|_| Error::Io)?;
        let created = match std::fs::symlink_metadata(&self.path) {
            Ok(m) if m.is_file() => false,
            Ok(_) => return Err(Error::Invalid("handover ledger file")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => return Err(Error::Io),
        };
        let mut options = std::fs::OpenOptions::new();
        options.read(true).append(true).create(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        // CP17 (CP16 review LOW-4): opened with write access too (writes
        // still go to the end), so the flush below never meets an
        // append-only handle on Windows. Unconfirmed on Windows itself.
        let mut file = crate::fsync::open_written(&mut options, &self.path).map_err(|_| Error::Io)?;
        let len = file.metadata().map_err(|_| Error::Io)?.len();
        // CP13b (CP13 review MEDIUM-1): the read bound, at append time. The
        // file after this append (a torn tail's newline included) must still
        // read back whole, so entry 8,193 is refused here, before its record.
        if len + 1 + LINE as u64 > MAX_BYTES || whole_entries(&mut file, len)? >= MAX_ENTRIES {
            return Err(Error::Limit("handover ledger"));
        }
        let mut out = Vec::with_capacity(LINE + 1);
        if len > 0 {
            let mut last = [0u8; 1];
            file.seek(SeekFrom::Start(len - 1)).map_err(|_| Error::Io)?;
            file.read_exact(&mut last).map_err(|_| Error::Io)?;
            if last[0] != b'\n' {
                out.push(b'\n');
            }
        }
        out.extend_from_slice(entry.line().as_bytes());
        file.write_all(&out).map_err(|_| Error::Io)?;
        crate::fsync::sync_written(&file).map_err(|_| Error::Io)?;
        if created {
            super::durable::sync_directory(dir)?;
        }
        Ok(())
    }

    /// CP13b (CP13 review MEDIUM-1): whether the ledger reads back and has
    /// room for `n` more entries (checked before the bearer is disclosed and
    /// before a receipt, so a handover never starts that can't be listed).
    pub fn has_room(&self, n: usize) -> bool {
        let len = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        self.entries().is_ok_and(|e| e.len() + n <= MAX_ENTRIES) && len + n as u64 * (LINE as u64 + 1) <= MAX_BYTES
    }

    /// Every whole entry, in order. A missing file is empty. A torn or
    /// damaged line is skipped; past the bound (bytes or entries): `Err`.
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(Error::Io),
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|_| Error::Io)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Error::Limit("handover ledger"));
        }
        let entries: Vec<Entry> = bytes.split(|b| *b == b'\n').filter_map(Entry::parse).collect();
        if entries.len() > MAX_ENTRIES {
            return Err(Error::Limit("handover ledger"));
        }
        Ok(entries)
    }

    /// Whether `entry` is listed. Anything unreadable fails closed (`false`).
    pub fn lists(&self, entry: &Entry) -> bool {
        match self.entries() {
            Ok(entries) => entries.contains(entry),
            Err(e) => {
                log::warn!("handover: the install's handover ledger can't be read ({e}); no record is acted on");
                false
            }
        }
    }
}

/// The whole entries in the first `len` bytes of `file` (under the lock).
fn whole_entries(file: &mut std::fs::File, len: u64) -> Result<usize> {
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(0)).map_err(|_| Error::Io)?;
    (&mut *file).take(len).read_to_end(&mut bytes).map_err(|_| Error::Io)?;
    Ok(bytes.split(|b| *b == b'\n').filter_map(Entry::parse).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CP13b (CP13 review MEDIUM-1): the append bound is the read bound.
    #[test]
    fn handover_ledger_refuses_entry_8193_before_its_record_and_reports_headroom() {
        let l = temp("full");
        std::fs::create_dir_all(l.path().parent().unwrap()).unwrap();
        std::fs::write(l.path(), entry(1, RecordKind::Retirement).line().repeat(MAX_ENTRIES - 2)).unwrap();
        assert!(l.has_room(2) && !l.has_room(3), "the headroom check is off");
        l.append(entry(2, RecordKind::Abort)).unwrap();
        l.append(entry(3, RecordKind::Abort)).unwrap();
        assert!(!l.has_room(1));
        assert!(l.append(entry(4, RecordKind::Abort)).is_err(), "entry 8,193 was appended");
        assert_eq!(l.entries().unwrap().len(), MAX_ENTRIES, "the full ledger no longer reads back");
        assert!(l.lists(&entry(3, RecordKind::Abort)));
    }
    fn temp(tag: &str) -> Ledger {
        let dir = std::env::temp_dir().join(format!("axenstax-ledger-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Ledger::at(dir.join("handover").join("ledger"))
    }
    fn entry(n: u8, kind: RecordKind) -> Entry {
        Entry::of([n; 16], Role::Source, kind, &[n; 7])
    }

    /// A damaged line skips only that entry; a torn tail is skipped and the
    /// next append stays whole; the file is 0600.
    #[test]
    fn handover_ledger_a_damaged_line_skips_only_that_entry() {
        let l = temp("damaged");
        for n in 1..=3 {
            l.append(entry(n, RecordKind::Retirement)).unwrap();
        }
        assert_eq!(l.entries().unwrap().len(), 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(l.path()).unwrap().permissions().mode() & 0o777, 0o600, "the ledger isn't 0600");
        }
        // Damage the second line's digest (one character), and tear a fourth.
        let mut bytes = std::fs::read(l.path()).unwrap();
        bytes[LINE + 60] = if bytes[LINE + 60] == b'0' { b'1' } else { b'0' };
        bytes.extend_from_slice(&entry(4, RecordKind::Receipt).line().as_bytes()[..50]);
        std::fs::write(l.path(), &bytes).unwrap();
        assert_eq!(l.entries().unwrap(), vec![entry(1, RecordKind::Retirement), entry(3, RecordKind::Retirement)], "a damaged line took more than itself");
        assert!(!l.lists(&entry(2, RecordKind::Retirement)), "a damaged entry is still listed");
        l.append(entry(5, RecordKind::Abort)).unwrap();
        assert!(l.lists(&entry(5, RecordKind::Abort)), "an append after a torn line was lost");
        assert!(l.lists(&entry(1, RecordKind::Retirement)) && l.lists(&entry(3, RecordKind::Retirement)));
        // Kind and role are part of the listing.
        assert!(!l.lists(&entry(1, RecordKind::Abort)));
        assert!(!l.lists(&Entry { role: Role::Recipient, ..entry(1, RecordKind::Retirement) }));
    }

    /// Past the bound the ledger fails closed: nothing is listed, and an
    /// append is refused.
    #[test]
    fn handover_ledger_past_its_bound_fails_closed() {
        let l = temp("bound");
        l.append(entry(1, RecordKind::Retirement)).unwrap();
        let mut bytes = std::fs::read(l.path()).unwrap();
        bytes.resize(MAX_BYTES as usize + 1, b'\n');
        std::fs::write(l.path(), &bytes).unwrap();
        assert!(l.entries().is_err());
        assert!(!l.lists(&entry(1, RecordKind::Retirement)), "an over-bound ledger still lists an entry");
        assert!(l.append(entry(2, RecordKind::Retirement)).is_err(), "an over-bound ledger took an append");
    }
}

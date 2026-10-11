//! D7: where a world's handover authority state lives. Each world folder's
//! `handover/` directory is a `MarkerStore` (temp file 0600, sync, rename,
//! directory sync; injectable boundary failures): the source's
//! `retired-<transfer>.dat` (`phase::Retirement`: the certificate core with
//! lineage, epoch and root context, the Prepared binding, the exact signed
//! Commit bytes and the retired root capabilities) is written there by
//! `phase::Source::retire`. No `WorldSave` field is added, so an old save
//! loaded from anywhere still finds the marker beside it.
//!
//! The open guard runs before a world's authority is exposed (load, open,
//! autosave, a late save job): ANY retirement record in the folder refuses,
//! fail closed. It does not filter by lineage or epoch, because a new host
//! session re-mints its invitation (and so the lineage it would derive), and
//! an unreadable or invalid record must never read as "no record".
use super::eligibility::Refusal;
use crate::handover::durable::MarkerStore;
use std::path::{Path, PathBuf};

/// The world's authority directory.
pub fn dir(world_dir: &Path) -> PathBuf {
    world_dir.join("handover")
}

/// The world's authority store (created on first use).
pub fn store(world_dir: &Path) -> crate::handover::Result<MarkerStore> {
    MarkerStore::new(dir(world_dir))
}

/// Most entries the guard looks at in `handover/` before failing closed.
const SCAN_LIMIT: usize = 256;

/// Refuses to expose a world whose authority was retired by a handover, or
/// whose authority state can't be read. A folder with no `handover/` is an
/// ordinary world. CP6 review LOW-5: read-only (`symlink_metadata`,
/// `read_dir`, a bounded read and a decode): it never creates the
/// directory, never writes and never syncs. ANY `retired-*.dat` refuses,
/// whether it decodes (`RETIRED`) or not (`AUTHORITY_UNREADABLE`).
///
/// CP10 (HIGH-2): a handover's imported world keeps its recipient records
/// here too (Ready, the Prepared receipt with the bearer, Activated). Until
/// the Activated record exists, whatever the inert marker says, the import
/// is refused (`NOT_ACTIVATED`): any `ready-*`, `prepared-*` or
/// `quarantined-*` record of a transfer with no `activated-<same>.dat`
/// refuses, and an Activated record that doesn't decode refuses
/// (`AUTHORITY_UNREADABLE`).
pub fn open_guard(world_dir: &Path) -> Result<(), Refusal> {
    let unreadable = Refusal(super::copy::AUTHORITY_UNREADABLE);
    let d = dir(world_dir);
    match std::fs::symlink_metadata(&d) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(unreadable),
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return Err(unreadable),
        Ok(_) => {}
    }
    let entries = std::fs::read_dir(&d).map_err(|_| unreadable.clone())?;
    let mut retired = None;
    let (mut pending, mut activated) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for (seen, entry) in entries.enumerate() {
        if seen >= SCAN_LIMIT {
            return Err(unreadable);
        }
        let name = entry.map_err(|_| unreadable.clone())?.file_name();
        let Some(name) = name.to_str() else { return Err(unreadable) };
        let id = |prefix: &str| {
            name.strip_prefix(prefix)
                .and_then(|n| n.strip_suffix(".dat"))
                .map(|v| hex::decode(v).ok().and_then(|v| <crate::handover::Id>::try_from(v).ok()))
        };
        if let Some(id) = id("retired-") {
            retired = Some((d.join(name), id));
            break;
        }
        for prefix in ["ready-", "prepared-", "quarantined-"] {
            if let Some(t) = id(prefix) {
                pending.insert(t.ok_or(unreadable.clone())?);
            }
        }
        if let Some(t) = id("activated-") {
            activated.insert(t.ok_or(unreadable.clone())?);
        }
    }
    let Some((path, id)) = retired else { return recipient_guard(&d, &pending, &activated) };
    let record = id.zip(read_bounded(&path)).and_then(|(id, b)| crate::handover::durable::retirement_payload(&b, id).ok());
    match record.and_then(|b| crate::handover::phase::Retirement::decode(&b).ok()) {
        Some(_) => Err(Refusal(super::copy::RETIRED)),
        None => Err(unreadable),
    }
}

/// CP10: an import is openable only once each transfer it holds records of
/// is Activated (a quarantined one never is), and every Activated record
/// decodes. Read-only.
fn recipient_guard(
    d: &Path,
    pending: &std::collections::BTreeSet<crate::handover::Id>,
    activated: &std::collections::BTreeSet<crate::handover::Id>,
) -> Result<(), Refusal> {
    use crate::handover::durable::{MarkerKind, marker_payload};
    for t in pending {
        let quarantined = d.join(format!("quarantined-{}.dat", hex::encode(t)));
        if !activated.contains(t) || std::fs::symlink_metadata(quarantined).is_ok() {
            return Err(Refusal(super::copy::NOT_ACTIVATED));
        }
    }
    for t in activated {
        let path = d.join(format!("activated-{}.dat", hex::encode(t)));
        let decoded = read_bounded(&path)
            .and_then(|b| marker_payload(&b, MarkerKind::Activated, *t).ok())
            .is_some_and(|p| crate::handover::codec::Reader::new(&p, crate::handover::durable::MARKER_LIMIT).and_then(|mut r| r.label("AxeNStax/handover/activated/v2")).is_ok());
        if !decoded {
            return Err(Refusal(super::copy::AUTHORITY_UNREADABLE));
        }
    }
    Ok(())
}

/// A regular file of at most `MARKER_LIMIT` bytes, read without following
/// a symlink.
fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > crate::handover::durable::MARKER_LIMIT as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(crate::handover::durable::MARKER_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= crate::handover::durable::MARKER_LIMIT).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handover::durable::MarkerKind;

    fn world(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("handover-authority-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// The guard: an ordinary world opens; any retirement record, valid or
    /// not, refuses; an unreadable authority directory refuses (fail closed).
    #[test]
    fn handover_open_guard_refuses_any_retirement_and_fails_closed() {
        let w = world("guard");
        assert!(open_guard(&w).is_ok(), "an ordinary world refused");
        let s = store(&w).unwrap();
        assert!(open_guard(&w).is_ok(), "an empty authority store refused");
        // CP10: a recipient's records refuse until the transfer is Activated.
        s.persist(MarkerKind::Ready, [3; 16], b"an import still arriving").unwrap();
        assert_eq!(open_guard(&w).unwrap_err().0, crate::handover::live::copy::NOT_ACTIVATED, "an unactivated import opened");
        s.persist(MarkerKind::Activated, [3; 16], b"not an Activated record").unwrap();
        assert_eq!(open_guard(&w).unwrap_err().0, crate::handover::live::copy::AUTHORITY_UNREADABLE, "an undecodable Activated opened");
        let a = store(&world("guard-activated")).unwrap();
        let mut v = Vec::new();
        crate::handover::codec::frame(&mut v, b"AxeNStax/handover/activated/v2");
        a.persist(MarkerKind::Prepared, [5; 16], b"receipt").unwrap();
        a.persist(MarkerKind::Activated, [5; 16], &v).unwrap();
        assert!(open_guard(a.directory().parent().unwrap()).is_ok(), "an Activated import refused");
        a.persist(MarkerKind::Quarantined, [5; 16], b"quarantined").unwrap();
        assert_eq!(open_guard(a.directory().parent().unwrap()).unwrap_err().0, crate::handover::live::copy::NOT_ACTIVATED);
        let _ = std::fs::remove_dir_all(a.directory().parent().unwrap());
        s.persist(MarkerKind::Retired, [4; 16], b"any retirement, even one that doesn't decode").unwrap();
        assert_eq!(open_guard(&w).unwrap_err().0, crate::handover::live::copy::AUTHORITY_UNREADABLE, "a retired world opened");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(dir(&w).join(format!("retired-{}.dat", hex::encode([4u8; 16])))).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600, "the authority record isn't 0600");
        }
        // LOW-5: read-only: a world with no handover/ stays without one.
        let r = world("guard-ro");
        assert!(open_guard(&r).is_ok());
        assert!(!dir(&r).exists(), "the guard created the authority directory");
        let _ = std::fs::remove_dir_all(&r);
        let v = world("guard-file");
        std::fs::write(dir(&v), b"not a directory").unwrap();
        assert_eq!(open_guard(&v).unwrap_err().0, crate::handover::live::copy::AUTHORITY_UNREADABLE);
        let _ = std::fs::remove_dir_all(&w);
        let _ = std::fs::remove_dir_all(&v);
    }
}

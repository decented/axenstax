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

/// Refuses to expose a world whose authority was retired by a handover, or
/// whose authority state can't be read. A folder with no `handover/` is an
/// ordinary world.
pub fn open_guard(world_dir: &Path) -> Result<(), Refusal> {
    let d = dir(world_dir);
    match std::fs::symlink_metadata(&d) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(Refusal(super::copy::AUTHORITY_UNREADABLE)),
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return Err(Refusal(super::copy::AUTHORITY_UNREADABLE)),
        Ok(_) => {}
    }
    let store = MarkerStore::new(d).map_err(|_| Refusal(super::copy::AUTHORITY_UNREADABLE))?;
    match store.retirements() {
        Ok(records) if records.is_empty() => Ok(()),
        Ok(_) => Err(Refusal(super::copy::RETIRED)),
        Err(_) => Err(Refusal(super::copy::AUTHORITY_UNREADABLE)),
    }
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
        s.persist(MarkerKind::Ready, [3; 16], b"not authority").unwrap();
        assert!(open_guard(&w).is_ok());
        s.persist(MarkerKind::Retired, [4; 16], b"any retirement, even one that doesn't decode").unwrap();
        assert_eq!(open_guard(&w).unwrap_err().0, crate::handover::live::copy::RETIRED, "a retired world opened");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(dir(&w).join(format!("retired-{}.dat", hex::encode([4u8; 16])))).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600, "the authority record isn't 0600");
        }
        let v = world("guard-file");
        std::fs::write(dir(&v), b"not a directory").unwrap();
        assert_eq!(open_guard(&v).unwrap_err().0, crate::handover::live::copy::AUTHORITY_UNREADABLE);
        let _ = std::fs::remove_dir_all(&w);
        let _ = std::fs::remove_dir_all(&v);
    }
}

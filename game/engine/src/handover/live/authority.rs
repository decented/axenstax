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
    let Some((path, id)) = retired else {
        match super::retained::read(world_dir) {
            Err(crate::handover::Error::State("inactive retained copy")) => return Err(Refusal(super::copy::NOT_ACTIVATED)),
            Err(_) => return Err(unreadable),
            Ok(_) => {}
        }
        recipient_guard(&d, &pending, &activated)?;
        // CP17 (CP15 review LOW-3): last, so a retired or still-arriving
        // world says so first. Read-only: the probe opens an existing
        // `.live` and never creates one.
        if super::world_lock::held_elsewhere(world_dir) {
            return Err(Refusal(super::copy::HANDOVER_ELSEWHERE));
        }
        return Ok(());
    };
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
        // CP14 (CP10 review LOW-3): the whole record parses, read-only, and
        // names the Prepared of the transfer's receipt when that is readable
        // (a receipt that exists but doesn't decode refuses). Note:
        // `Receiver::activate` doesn't look for a quarantine record; this
        // guard fails closed on one instead (the loop above).
        let receipt = match read_bounded(&d.join(format!("prepared-{}.dat", hex::encode(t)))) {
            None => None,
            Some(b) => match marker_payload(&b, MarkerKind::Prepared, *t) {
                Ok(p) => Some(zeroize::Zeroizing::new(p)),
                Err(_) => return Err(Refusal(super::copy::AUTHORITY_UNREADABLE)),
            },
        };
        let decoded = read_bounded(&path)
            .and_then(|b| marker_payload(&b, MarkerKind::Activated, *t).ok())
            .map(zeroize::Zeroizing::new)
            .is_some_and(|p| crate::handover::phase::activated_record_checks(&p, *t, receipt.as_deref().map(Vec::as_slice)));
        if !decoded {
            return Err(Refusal(super::copy::AUTHORITY_UNREADABLE));
        }
    }
    Ok(())
}

/// A regular file of at most `MARKER_LIMIT` bytes, read without following
/// a symlink. Zeroed on drop (CP10 review LOW-4: it may hold a bearer).
fn read_bounded(path: &Path) -> Option<zeroize::Zeroizing<Vec<u8>>> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > crate::handover::durable::MARKER_LIMIT as u64 {
        return None;
    }
    // CP16 (CP14 review LOW-3): one zeroised buffer from before the read
    // (never reallocated; a part-way failure drops it zeroed).
    let bytes = crate::handover::durable::read_zeroized(path, meta.len()).ok()?;
    (bytes.len() as u64 == meta.len()).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handover::durable::{tests::TestDir, MarkerKind};

    fn world(tag: &str) -> TestDir {
        let p = TestDir::new(std::env::temp_dir().join(format!("handover-authority-{tag}-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&*p);
        std::fs::create_dir_all(&*p).unwrap();
        p
    }

    /// Generic Fork refuses both retired and imported authority: it cannot
    /// preserve their personal proofs. Ordinary unbound worlds still fork
    /// without carrying an authority directory or invitation bearer.
    #[test]
    fn handover_fork_carries_no_handover_folder_and_no_bearer() {
        const BEARER: [u8; 16] = [0xC3; 16];
        let _root = crate::save::WorldsRootGuard::new("handover-fork");
        let (_, core) = crate::handover::certificate::tests::fixture();
        let mut activated = Vec::new();
        crate::handover::codec::frame(&mut activated, b"AxeNStax/handover/activated/v2");
        crate::handover::codec::frame(&mut activated, &core.encode());
        activated.extend_from_slice(&[1; 32]);
        activated.extend_from_slice(&[2; 32]);
        activated.extend_from_slice(&BEARER);
        let retirement = b"a retirement record".to_vec();
        for (name, kind, id, payload) in [
            ("retired", MarkerKind::Retired, [7; 16], &retirement),
            ("activated", MarkerKind::Activated, core.transfer, &activated),
        ] {
            let w = crate::save::world_dir(name);
            crate::save::write_world_folder(
                name,
                &crate::save::WorldMeta::new(name),
                &crate::save::minimal_world_save_for_tests(77),
                &crate::world::World::new(),
            )
            .unwrap();
            store(&w).unwrap().persist(kind, id, payload).unwrap();
            let data_before = std::fs::read(w.join("world.dat")).unwrap();
            let meta_before = std::fs::read(w.join("world_meta.json")).unwrap();
            let records_before: std::collections::BTreeMap<_, _> = std::fs::read_dir(dir(&w))
                .unwrap()
                .flatten()
                .map(|e| (e.path(), std::fs::read(e.path()).unwrap()))
                .collect();
            let before: Vec<_> = std::fs::read_dir(crate::save::worlds_root()).unwrap().flatten().map(|e| e.path()).collect();
            let forked = crate::save::fork_world(name, name);
            let made: Vec<_> = std::fs::read_dir(crate::save::worlds_root())
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| !before.contains(p))
                .collect();
            assert!(forked.is_err(), "a bound {name} world was forked: {forked:?}");
            assert!(made.is_empty(), "the refused {name} fork made a folder");
            assert_eq!(std::fs::read(w.join("world.dat")).unwrap(), data_before);
            assert_eq!(std::fs::read(w.join("world_meta.json")).unwrap(), meta_before);
            let records_after: std::collections::BTreeMap<_, _> = std::fs::read_dir(dir(&w))
                .unwrap()
                .flatten()
                .map(|e| (e.path(), std::fs::read(e.path()).unwrap()))
                .collect();
            assert_eq!(records_after, records_before, "the refused fork changed authority bytes");
            if name == "activated" {
                assert!(open_guard(&w).is_ok(), "the Activated marker guard changed");
            }
        }
        crate::save::write_world_folder(
            "ordinary",
            &crate::save::WorldMeta::new("ordinary"),
            &crate::save::minimal_world_save_for_tests(77),
            &crate::world::World::new(),
        )
        .unwrap();
        let fork = crate::save::fork_world("ordinary", "ordinary").expect("an unbound world still forks");
        let copied = crate::save::world_dir(&fork);
        assert!(copied.join("world.dat").is_file(), "the ordinary fork lost the world");
        assert!(!dir(&copied).exists(), "the ordinary fork carried handover/");
        let mut stack = vec![copied];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    let bytes = std::fs::read(e.path()).unwrap();
                    assert!(!bytes.windows(16).any(|x| x == BEARER), "the ordinary fork carried the bearer");
                }
            }
        }
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
        // CP14 (CP10 review LOW-3): the right label over a truncated
        // payload refuses (the full parse, not the label, decides). A whole
        // Activated record opening is pinned end to end in handover_live.
        let a_world = world("guard-activated");
        let a = store(&a_world).unwrap();
        let mut v = Vec::new();
        crate::handover::codec::frame(&mut v, b"AxeNStax/handover/activated/v2");
        a.persist(MarkerKind::Activated, [5; 16], &v).unwrap();
        assert_eq!(
            open_guard(a.directory().parent().unwrap()).map_err(|r| r.0),
            Err(crate::handover::live::copy::AUTHORITY_UNREADABLE),
            "an Activated record with the right label and a truncated payload opened"
        );
        a.persist(MarkerKind::Quarantined, [5; 16], b"quarantined").unwrap();
        assert_eq!(open_guard(a.directory().parent().unwrap()).unwrap_err().0, crate::handover::live::copy::NOT_ACTIVATED);
        s.persist(MarkerKind::Retired, [4; 16], b"any retirement, even one that doesn't decode").unwrap();
        assert_eq!(open_guard(&w).unwrap_err().0, crate::handover::live::copy::AUTHORITY_UNREADABLE, "a retired world opened");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(dir(&w).join(format!("retired-{}.dat", hex::encode([4u8; 16])))).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600, "the authority record isn't 0600");
            // CP14 (CP10 review LOW-4): the folder itself is owner-only.
            let meta = std::fs::metadata(dir(&w)).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o700, "handover/ isn't 0700");
        }
        // LOW-5: read-only: a world with no handover/ stays without one.
        let r = world("guard-ro");
        assert!(open_guard(&r).is_ok());
        assert!(!dir(&r).exists(), "the guard created the authority directory");
        let v = world("guard-file");
        std::fs::write(dir(&v), b"not a directory").unwrap();
        assert_eq!(open_guard(&v).unwrap_err().0, crate::handover::live::copy::AUTHORITY_UNREADABLE);
    }
}

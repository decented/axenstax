//! Durable receipt binds immutable received members and explicit transformv1.
use super::super::{Error, Hash, Result, manifest::MemberKind};
use super::import::{DurableCheckpoint, ImportBoundary, ImportObserver, observe, sync_existing};
use super::{AdmittedManifest, files};
use std::{fs, path::Path};
pub(super) fn receipt_bytes(a: &AdmittedManifest, meta: Hash, admission: Hash) -> Result<Vec<u8>> {
    let mut bytes = b"AxeNStax/handover/import-receipt/v1\0".to_vec();
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&a.manifest().digest()?);
    bytes.extend_from_slice(&a.core().digest());
    bytes.extend_from_slice(&a.manifest().world);
    bytes.extend_from_slice(&a.manifest().transfer);
    bytes.extend_from_slice(&meta);
    bytes.extend_from_slice(&admission);
    bytes.extend_from_slice(&super::super::hash(&bytes));
    Ok(bytes)
}
pub(super) fn verify_receipt(directory: &Path, a: &AdmittedManifest) -> Result<DurableCheckpoint> {
    if !fs::symlink_metadata(directory)
        .map_err(|_| Error::Io)?
        .is_dir()
    {
        return Err(Error::Invalid("import directory"));
    }
    let evidence = files::read(&directory.join("admission.dat"), 32 * 1024)?;
    super::evidence::verify_stored(&evidence, a)?;
    let saved = files::read(
        &directory.join("manifest.dat"),
        super::super::manifest::MANIFEST_LIMIT,
    )?;
    if saved != a.manifest().encode()? {
        return Err(Error::Invalid("receipt manifest"));
    }
    // Immutable raw member evidence survives authorised metadata/player changes
    // in the working copy. This milestone's transform only changes metadata.
    for m in &a.manifest().members {
        let raw = files::digest(
            files::open(&directory.join("received").join(&m.path))?,
            m.length,
        )?;
        if raw != (m.length, m.digest) {
            return Err(Error::Invalid("received evidence"));
        }
        if m.kind != MemberKind::WorldMeta {
            let working = files::digest(
                files::open(&directory.join("working").join(&m.path))?,
                m.length,
            )?;
            if working != (m.length, m.digest) {
                return Err(Error::Invalid("working evidence"));
            }
        }
    }
    let working = files::read(&directory.join("working/world_meta.json"), 2 * 1024 * 1024)?;
    let meta = super::bounded_decode::metadata(&working)?;
    if meta.pop_secret.is_none()
        || meta.cloud_save
        || meta.published_to.is_some()
        || meta.owner_pubkey != Some(hex::encode(a.core().successor_persona))
    {
        return Err(Error::Invalid("imported operator metadata"));
    }
    let hash = super::super::hash(&working);
    if files::read(&directory.join("receipt.dat"), 1024)?
        != receipt_bytes(a, hash, super::super::hash(&evidence))?
    {
        return Err(Error::Invalid("import receipt checksum/binding"));
    }
    Ok(DurableCheckpoint {
        directory: directory.to_owned(),
        manifest: a.manifest().digest()?,
        core: a.core().digest(),
        world: a.manifest().world,
        transfer: a.manifest().transfer,
        working_meta: hash,
    })
}
pub(super) fn durable_tree(
    directory: &Path,
    a: &AdmittedManifest,
    o: &mut impl ImportObserver,
) -> Result<()> {
    for prefix in ["received", "working"] {
        let root = directory.join(prefix);
        for m in &a.manifest().members {
            let path = root.join(&m.path);
            sync_existing(&path, o)?;
        }
        for child in ["chunks", "exhibits", "joiners", "participants"] {
            let p = root.join(child);
            match fs::symlink_metadata(&p) {
                Ok(meta) => {
                    if !meta.is_dir() {
                        return Err(Error::Invalid("import directory"));
                    }
                    observe(o, ImportBoundary::DirectorySync, &p)?;
                    files::sync_dir(&p)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::Io),
            }
        }
        observe(o, ImportBoundary::DirectorySync, &root)?;
        files::sync_dir(&root)?;
    }
    for name in [
        "archive.tar.gz",
        "manifest.dat",
        "receipt.dat",
        "admission.dat",
    ] {
        let p = directory.join(name);
        sync_existing(&p, o)?;
    }
    observe(o, ImportBoundary::DirectorySync, directory)?;
    files::sync_dir(directory)
}

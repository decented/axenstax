//! A source's distinct, private retained world. Its signed personal provenance
//! admits only identity-selected saved state, never original routing authority.
use crate::handover::{
    Error, Id, Result,
    certificate::Core,
    checkpoint::{
        AdmittedManifest, DurableCheckpoint, Import, ImportBoundary, ImportObserver,
        PackedCheckpoint,
    },
    codec::{Reader, frame},
    durable::{MARKER_LIMIT, MarkerKind, MarkerStore},
    manifest::Manifest,
    phase::Retirement,
    stream::{CHUNK_LIMIT, Chunk},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Timestamp};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const LABEL: &str = "AxeNStax/handover/retained-personal/v1";

fn content(retirement: &[u8], core: &Core, copy: Id, attestation: &[u8]) -> String {
    let mut bytes = Vec::new();
    frame(&mut bytes, LABEL.as_bytes());
    bytes.extend_from_slice(&crate::handover::hash(retirement));
    bytes.extend_from_slice(&crate::handover::hash(&core.encode()));
    bytes.extend_from_slice(&core.manifest);
    bytes.extend_from_slice(&copy);
    bytes.extend_from_slice(&crate::handover::hash(attestation));
    STANDARD.encode(bytes)
}

pub(crate) struct Info {
    pub retirement: Retirement,
    pub manifest: Manifest,
}
impl Info {
    pub(crate) fn core(&self) -> &Core {
        self.retirement.core()
    }
}

fn manifest_matches(manifest: &Manifest, core: &Core) -> Result<()> {
    if manifest.digest()? != core.manifest
        || manifest.lineage != core.lineage
        || manifest.generation != core.generation
        || manifest.transfer != core.transfer
        || manifest.world != core.snapshot_world
        || manifest.source != core.source_persona
        || manifest.receiver != core.successor_persona
    {
        return Err(Error::Invalid("retained personal manifest binding"));
    }
    Ok(())
}

fn decode(payload: &[u8], copy: Id, manifest: Manifest) -> Result<Info> {
    let mut reader = Reader::new(payload, MARKER_LIMIT)?;
    reader.label(LABEL)?;
    let record = reader.framed(MARKER_LIMIT)?;
    let retirement = Retirement::decode(record)?;
    let attestation_bytes = reader.framed(crate::handover::PROOF_LIMIT)?;
    let attestation = Event::from_json(attestation_bytes)
        .map_err(|_| Error::Invalid("retained source attestation"))?;
    let signature = Event::from_json(reader.framed(crate::handover::PROOF_LIMIT)?)
        .map_err(|_| Error::Invalid("retained personal signature"))?;
    reader.end()?;
    let core = retirement.core();
    core.validate(&retirement.personal_root()?, core.issued_at)?;
    let runtime = PublicKey::from_slice(&core.source_runtime)
        .map_err(|_| Error::Invalid("retained source runtime"))?;
    for time in [core.issued_at, core.expires_at] {
        let owner = crate::runtime_identity::verify_player_attestation(
            &attestation,
            &runtime,
            Timestamp::from(time),
        )
        .map_err(|_| Error::Invalid("retained source attestation"))?;
        if owner.to_bytes() != core.source_persona {
            return Err(Error::Invalid("retained source persona"));
        }
    }
    if retirement.retained_copy() != copy
        || signature.verify().is_err()
        || signature.pubkey.to_bytes() != core.source_runtime
        || signature.kind != Kind::Custom(crate::handover::STORED_KIND)
        || signature.created_at.as_secs() != core.issued_at
        || !signature.tags.is_empty()
        || signature.content != content(record, core, copy, attestation_bytes)
    {
        return Err(Error::Invalid("retained personal provenance"));
    }
    manifest_matches(&manifest, core)?;
    Ok(Info {
        retirement,
        manifest,
    })
}

/// Read only; the inherited recipient markers/bearer are never part of a
/// retained copy. An inactive marker always wins over a prepared signature.
pub(crate) fn read(world: &Path) -> Result<Option<Info>> {
    let directory = super::authority::dir(world);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::Io),
    };
    let mut copy = None;
    let mut recipient = false;
    for (seen, entry) in entries.enumerate() {
        if seen >= 256 {
            return Err(Error::Limit("retained handover records"));
        }
        let name = entry.map_err(|_| Error::Io)?.file_name();
        let name = name
            .to_str()
            .ok_or(Error::Invalid("retained record filename"))?;
        if name.starts_with("retained-inert-") {
            return Err(Error::State("inactive retained copy"));
        }
        recipient |= ["ready-", "prepared-", "activated-", "quarantined-"]
            .iter()
            .any(|prefix| name.starts_with(prefix));
        if let Some(id) = name
            .strip_prefix("retained-copy-")
            .and_then(|s| s.strip_suffix(".dat"))
        {
            let id = hex::decode(id)
                .ok()
                .and_then(|bytes| <Id>::try_from(bytes).ok())
                .ok_or(Error::Invalid("retained copy filename"))?;
            if copy.replace(id).is_some() {
                return Err(Error::Invalid("multiple retained copies"));
            }
        }
    }
    let Some(copy) = copy else { return Ok(None) };
    if recipient {
        return Err(Error::Invalid(
            "retained copy has inherited routing receipts",
        ));
    }
    let store = MarkerStore::open_existing(directory.clone())?;
    let payload = store
        .read(MarkerKind::RetainedCopy, copy)?
        .ok_or(Error::Invalid("missing retained personal proof"))?;
    let manifest_bytes = super::personal_adapter::read_regular(
        &directory.join("personal-manifest.dat"),
        crate::handover::manifest::MANIFEST_LIMIT as u64,
    )
    .map_err(|_| Error::Io)?;
    decode(&payload, copy, Manifest::decode(&manifest_bytes)?).map(Some)
}

fn staging(worlds: &Path, copy: Id) -> PathBuf {
    worlds.join(".handover-retained").join(hex::encode(copy))
}
fn folder(copy: Id) -> String {
    format!("retained-{}", hex::encode(copy))
}

/// All members and selected personal state are durable before this is returned.
/// Before retirement Drop discards only this new, inactive scratch copy.
pub(crate) struct PreparedCopy {
    checkpoint: DurableCheckpoint,
    parent: PathBuf,
    copy: Id,
    keep: bool,
}
impl Drop for PreparedCopy {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.parent);
        }
    }
}
struct CancelCopy(Arc<AtomicBool>);
impl ImportObserver for CancelCopy {
    fn reached(&mut self, _: ImportBoundary, _: &Path) -> std::io::Result<()> {
        if self.0.load(Ordering::Relaxed) {
            Err(std::io::Error::other("retained copy cancelled"))
        } else {
            Ok(())
        }
    }
}

pub(crate) fn prepare(
    packed: &PackedCheckpoint,
    admitted: AdmittedManifest,
    copy: Id,
    worlds: PathBuf,
    now: u64,
    cancel: Arc<AtomicBool>,
) -> Result<PreparedCopy> {
    let parent = staging(&worlds, copy);
    if parent.exists() {
        return Err(Error::Invalid("retained copy collision"));
    }
    let root = parent.parent().ok_or(Error::Io)?;
    crate::handover::durable::create_private_dir(root)?;
    for (seen, entry) in fs::read_dir(root).map_err(|_| Error::Io)?.enumerate() {
        entry.map_err(|_| Error::Io)?;
        if seen >= 63 {
            return Err(Error::Limit("inactive retained copies"));
        }
    }
    let mut directory = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(&parent).map_err(|_| Error::Io)?;
    crate::fsync::sync_dir(root).map_err(|_| Error::Io)?;
    let result = (|| {
        let proof = super::personal_adapter::Proof::retained_checkpoint(&admitted, copy);
        let mut import = Import::new(admitted, &parent, now)?;
        let mut archive = fs::File::open(packed.archive()).map_err(|_| Error::Io)?;
        let mut buffer = [0; CHUNK_LIMIT];
        let mut offset = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let length = archive.read(&mut buffer).map_err(|_| Error::Io)?;
            if length == 0 {
                break;
            }
            import.accept(
                &Chunk {
                    transfer: proof.core.transfer,
                    offset,
                    bytes: buffer[..length].to_vec(),
                },
                now,
            )?;
            offset += length as u64;
        }
        let checkpoint = import.complete(now, &mut CancelCopy(cancel))?;
        let working = checkpoint.working_world();
        let metadata_bytes = super::personal_adapter::read_regular(
            &working.join("world_meta.json"),
            2 * 1024 * 1024,
        )
        .map_err(|_| Error::Io)?;
        let mut metadata: crate::save::WorldMeta = serde_json::from_slice(&metadata_bytes)
            .map_err(|_| Error::Invalid("retained metadata"))?;
        metadata.owner_pubkey = Some(hex::encode(proof.core.source_persona));
        metadata.display_name.push_str(" (retained copy)");
        let metadata_bytes =
            serde_json::to_vec(&metadata).map_err(|_| Error::Invalid("retained metadata"))?;
        if metadata_bytes.len() > 2 * 1024 * 1024 {
            return Err(Error::Limit("retained metadata"));
        }
        crate::save::write_atomic(&working.join("world_meta.json"), &metadata_bytes)
            .map_err(|_| Error::UncertainDurability)?;
        let mut selected = None;
        for member in &proof.manifest().members {
            if member.kind != crate::handover::manifest::MemberKind::Sidecar {
                continue;
            }
            let npub = member
                .path
                .strip_prefix("joiners/")
                .and_then(|s| s.strip_suffix(".dat"))
                .ok_or(Error::Invalid("retained participant path"))?;
            let key = crate::handover::context::canonical_npub(npub)?;
            let saved = super::personal_adapter::load_or_bootstrap(&working, key, &proof)
                .map_err(|_| Error::Invalid("retained personal bootstrap"))?
                .ok_or(Error::Invalid("retained participant missing"))?;
            if key == proof.core.source_persona {
                selected = Some(crate::personal_value::LocalPersonalSave(
                    saved
                        .encode()
                        .map_err(|_| Error::Invalid("retained selected state"))?,
                ));
            }
        }
        let selected = selected.ok_or(Error::Invalid("retained source record missing"))?;
        let data_member = proof
            .manifest()
            .members
            .iter()
            .find(|member| member.kind == crate::handover::manifest::MemberKind::WorldData)
            .ok_or(Error::Invalid("retained world member"))?;
        let bytes =
            super::personal_adapter::read_regular(&working.join("world.dat"), data_member.length)
                .map_err(|_| Error::Io)?;
        let (mut save, defaulted) = crate::save::read_world_save_reporting(&bytes)
            .map_err(|_| Error::Invalid("retained world snapshot"))?;
        if defaulted {
            return Err(Error::Invalid("retained world snapshot defaulted"));
        }
        save.local_personal = Some(selected);
        super::personal_adapter::write_world_snapshot(&working, &working, &save)
            .map_err(|_| Error::UncertainDurability)?;
        let store = MarkerStore::new(super::authority::dir(&working))?;
        let mut inert = Vec::new();
        frame(&mut inert, b"AxeNStax/handover/retained-inert/v1");
        inert.extend_from_slice(&copy);
        inert.extend_from_slice(&proof.core.transfer);
        inert.extend_from_slice(&crate::handover::hash(&proof.core.encode()));
        store.persist(MarkerKind::RetainedInert, copy, &inert)?;
        crate::fsync::sync_dir(&parent).map_err(|_| Error::UncertainDurability)?;
        Ok(PreparedCopy {
            checkpoint,
            parent: parent.clone(),
            copy,
            keep: false,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&parent);
    }
    result
}

impl PreparedCopy {
    /// Prepare the exact public proof before attempting retirement. Inert
    /// remains present until durable retirement and folder promotion succeed.
    pub(crate) fn seal(
        &mut self,
        retirement: &Retirement,
        source: &Keys,
        attestation: &Event,
    ) -> Result<()> {
        if retirement.retained_copy() != self.copy
            || retirement.core().digest() != self.checkpoint.core_hash()
            || retirement.core().manifest != self.checkpoint.manifest_hash()
            || retirement.core().transfer != self.checkpoint.transfer()
        {
            return Err(Error::Invalid("retained copy reservation"));
        }
        let record = retirement.encode()?;
        let attestation = attestation.as_json();
        let core = retirement.core();
        let signature = pollster::block_on(
            EventBuilder::new(
                Kind::Custom(crate::handover::STORED_KIND),
                content(&record, core, self.copy, attestation.as_bytes()),
            )
            .custom_created_at(Timestamp::from(core.issued_at))
            .sign(source),
        )
        .map_err(|_| Error::Invalid("retained personal signing"))?
        .as_json();
        let size = LABEL.len() + record.len() + attestation.len() + signature.len() + 16;
        if size > MARKER_LIMIT - 256 {
            return Err(Error::Limit("retained personal proof"));
        }
        let mut payload = Vec::with_capacity(size);
        frame(&mut payload, LABEL.as_bytes());
        frame(&mut payload, &record);
        frame(&mut payload, attestation.as_bytes());
        frame(&mut payload, signature.as_bytes());
        let working = self.checkpoint.working_world();
        let manifest = Manifest::decode(
            &super::personal_adapter::read_regular(
                &working.join("handover/personal-manifest.dat"),
                crate::handover::manifest::MANIFEST_LIMIT as u64,
            )
            .map_err(|_| Error::Io)?,
        )?;
        decode(&payload, self.copy, manifest)?;
        // Preserve prepared bytes if a later retirement write is uncertain.
        self.keep = true;
        MarkerStore::open_existing(super::authority::dir(&working))?.persist(
            MarkerKind::RetainedCopy,
            self.copy,
            &payload,
        )
    }
}

/// The installation ledger authenticates the exact local retirement bytes;
/// freshly confirm their durability before releasing any prepared copy.
pub(crate) fn finish_retired(
    world: &Path,
    retired: &crate::handover::phase::RetiredCommit,
    ledger: &crate::handover::ledger::Ledger,
) -> Result<String> {
    if retired.is_abort() || !ledger.lists(&retired.ledger_entry()) {
        return Err(Error::Invalid("retained source retirement ownership"));
    }
    let store = MarkerStore::open_existing(super::authority::dir(world))?;
    let payload = store
        .read(MarkerKind::Retired, retired.transfer())?
        .ok_or(Error::Invalid("retained source retirement missing"))?;
    if crate::handover::hash(&payload) != retired.ledger_entry().digest {
        return Err(Error::Invalid("retained source retirement changed"));
    }
    let retirement = Retirement::decode(&payload)?;
    retired.persist_again(&store)?;
    finish(&retirement)
}

/// Called only after exact source retirement ownership and durability.
/// Idempotent promotion never replaces a destination, including a played copy.
fn finish(retirement: &Retirement) -> Result<String> {
    let copy = retirement.retained_copy();
    let parent = staging(&crate::save::worlds_root(), copy);
    let destination = crate::save::world_dir(&folder(copy));
    let working = match fs::read_dir(&parent) {
        Ok(entries) => {
            let mut candidates = Vec::new();
            for (seen, entry) in entries.enumerate() {
                if seen >= 8 {
                    return Err(Error::Limit("retained staging entries"));
                }
                let path = entry.map_err(|_| Error::Io)?.path().join("working");
                if path.is_dir() {
                    candidates.push(path);
                }
            }
            if candidates.is_empty() && destination.is_dir() {
                destination.clone()
            } else if candidates.len() == 1 {
                candidates.remove(0)
            } else {
                return Err(Error::Invalid("retained staging world"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && destination.is_dir() => {
            destination.clone()
        }
        Err(_) => return Err(Error::Io),
    };
    let store = MarkerStore::open_existing(super::authority::dir(&working))?;
    let payload = store
        .read(MarkerKind::RetainedCopy, copy)?
        .ok_or(Error::Invalid("retained provenance missing"))?;
    let manifest = Manifest::decode(
        &super::personal_adapter::read_regular(
            &working.join("handover/personal-manifest.dat"),
            crate::handover::manifest::MANIFEST_LIMIT as u64,
        )
        .map_err(|_| Error::Io)?,
    )?;
    let info = decode(&payload, copy, manifest)?;
    if info.retirement.encode()? != retirement.encode()? {
        return Err(Error::Invalid("retained source retirement differs"));
    }
    if working != destination {
        super::promote::rename_no_replace(&working, &destination)
            .map_err(|_| Error::UncertainDurability)?;
        crate::fsync::sync_dir(working.parent().ok_or(Error::Io)?)
            .map_err(|_| Error::UncertainDurability)?;
    }
    crate::fsync::sync_dir(&crate::save::worlds_root()).map_err(|_| Error::UncertainDurability)?;
    let store = MarkerStore::open_existing(super::authority::dir(&destination))?;
    store.persist(MarkerKind::RetainedCopy, copy, &payload)?;
    store.remove(MarkerKind::RetainedInert, copy)?;
    crate::fsync::sync_dir(store.directory()).map_err(|_| Error::UncertainDurability)?;
    crate::fsync::sync_dir(&destination).map_err(|_| Error::UncertainDurability)?;
    if parent.exists() {
        let _ = fs::remove_dir_all(&parent);
    }
    Ok(folder(copy))
}

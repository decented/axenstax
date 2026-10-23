//! Inactive import under an owned, non-world wrapper directory. Raw verified
//! members remain immutable evidence; authorised receiver metadata is separate.
use super::super::{
    Error, Hash, Id, Result,
    manifest::{Manifest, Member, MemberKind},
    stream::{Chunk, Receiver},
};
use super::receipt::{durable_tree, receipt_bytes, verify_receipt};
use super::{AdmittedManifest, files, sync_pool::SyncPool, validate_metadata};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportBoundary {
    Write,
    FileSync,
    DirectorySync,
    BeforeRename,
    Renamed,
    ParentSynced,
}
pub trait ImportObserver {
    fn reached(&mut self, boundary: ImportBoundary, path: &Path) -> std::io::Result<()>;
}
pub struct NoFault;
impl ImportObserver for NoFault {
    fn reached(&mut self, _: ImportBoundary, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
}
/// Private receipt: exact received manifest plus explicit local transformv1.
/// It is never an Activated proof and exposes no gameplay admission operation.
pub struct DurableCheckpoint {
    pub(super) directory: PathBuf,
    pub(super) manifest: Hash,
    pub(super) core: Hash,
    pub(super) world: Id,
    pub(super) transfer: Id,
    pub(super) working_meta: Hash,
    /// CP22 (CP19 review MEDIUM-1): the manifest's member paths.
    pub(super) members: Vec<String>,
}
impl DurableCheckpoint {
    /// CP22 (CP19 review MEDIUM-1, DECIDED): the working world's own files
    /// (`world.dat` and every manifest member) are all still there as
    /// regular files. The recipient checks this under its lock before the
    /// ack, so a discard that emptied the import is never acknowledged into.
    pub fn world_files_present(&self) -> bool {
        let working = self.working_world();
        std::iter::once("world.dat")
            .chain(self.members.iter().map(String::as_str))
            .all(|p| fs::symlink_metadata(working.join(p)).is_ok_and(|m| m.is_file()))
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn working_world(&self) -> PathBuf {
        self.directory.join("working")
    }
    pub fn received_world(&self) -> PathBuf {
        self.directory.join("received")
    }
    pub fn manifest_hash(&self) -> Hash {
        self.manifest
    }
    pub fn core_hash(&self) -> Hash {
        self.core
    }
    pub fn world(&self) -> Id {
        self.world
    }
    pub fn transfer(&self) -> Id {
        self.transfer
    }
    pub fn working_metadata_hash(&self) -> Hash {
        self.working_meta
    }
    pub fn transform_version(&self) -> u32 {
        1
    }
}
pub struct Import {
    admitted: AdmittedManifest,
    parent: PathBuf,
    directory: PathBuf,
    final_path: PathBuf,
    receiver: Option<Receiver<crate::fsync::Written>>,
    verified: bool,
    renamed: bool,
    limits: super::Limits,
}
impl Drop for Import {
    fn drop(&mut self) {
        // A renamed import might already be adopted by a later controller.
        // Never delete it on late cancellation, including ambiguous durability.
        if !self.renamed {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
impl Import {
    pub fn new(admitted: AdmittedManifest, parent: &Path, now: u64) -> Result<Self> {
        Self::new_limited(admitted, parent, now, super::Limits::default())
    }
    pub fn new_limited(
        admitted: AdmittedManifest,
        parent: &Path,
        now: u64,
        limits: super::Limits,
    ) -> Result<Self> {
        limits.validate()?;
        let manifest = admitted.manifest();
        if manifest.stream_length > limits.compressed
            || manifest.members.len() > limits.members
            || manifest
                .members
                .iter()
                .try_fold(0u64, |sum, m| sum.checked_add(m.length))
                .is_none_or(|n| n > limits.logical)
        {
            return Err(Error::Limit("admitted checkpoint limits"));
        }
        let final_path = destination(parent, &admitted);
        collision(&final_path)?;
        // Named by transfer so the inert-import sweep (live::inert) can find a
        // crash-left scratch of exactly that transfer and nothing else.
        let directory = files::owned_dir(
            parent,
            &format!("handover-receive-{}", hex::encode(admitted.manifest().transfer)),
        )?;
        let file = match files::new_written(&directory.join("archive.tar.gz")) {
            Ok(f) => f,
            Err(e) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(e);
            }
        };
        let m = admitted.manifest();
        let receiver = Receiver::new(file, m.transfer, m.stream_length, m.stream_hash, now)?;
        Ok(Self {
            admitted,
            parent: parent.to_owned(),
            directory,
            final_path,
            receiver: Some(receiver),
            verified: false,
            renamed: false,
            limits,
        })
    }
    /// CP23: the import folder's name (`handover-<world>-<transfer>`),
    /// which names its lock file.
    pub fn folder_name(&self) -> String {
        self.final_path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned()
    }
    pub fn accept(&mut self, chunk: &Chunk, now: u64) -> Result<bool> {
        self.receiver
            .as_mut()
            .ok_or(Error::State("import stream closed"))?
            .accept(chunk, now)
    }
    /// Same object retries uncertain post-rename durability without regeneration
    /// or deletion. Process restart uses resume with a newly authenticated offer.
    pub fn complete(
        &mut self,
        now: u64,
        observer: &mut impl ImportObserver,
    ) -> Result<DurableCheckpoint> {
        if self.renamed {
            return self.finish_existing(observer);
        }
        if !self.verified {
            let file = self
                .receiver
                .take()
                .ok_or(Error::State("failed import stream"))?
                .finish(now)?;
            sync_written(&file, &self.directory.join("archive.tar.gz"), observer)?;
            extract(&self.directory, &self.admitted, self.limits, observer)?;
            self.verified = true;
        }
        collision(&self.final_path)?;
        durable_tree(&self.directory, &self.admitted, observer)?;
        observe(observer, ImportBoundary::BeforeRename, &self.directory)?;
        collision(&self.final_path)?;
        files::rename_new(&self.directory, &self.final_path)?;
        self.directory = self.final_path.clone();
        self.renamed = true;
        observer
            .reached(ImportBoundary::Renamed, &self.directory)
            .map_err(|_| Error::UncertainDurability)?;
        self.finish_existing(observer)
    }
    fn finish_existing(&self, observer: &mut impl ImportObserver) -> Result<DurableCheckpoint> {
        let receipt = verify_receipt(&self.directory, &self.admitted)?;
        durable_tree(&self.directory, &self.admitted, observer)
            .map_err(|_| Error::UncertainDurability)?;
        files::sync_dir(&self.parent).map_err(|_| Error::UncertainDurability)?;
        observer
            .reached(ImportBoundary::ParentSynced, &self.parent)
            .map_err(|_| Error::UncertainDurability)?;
        Ok(receipt)
    }
    pub fn resume(admitted: AdmittedManifest, parent: &Path) -> Result<Self> {
        let final_path = destination(parent, &admitted);
        verify_receipt(&final_path, &admitted)?;
        Ok(Self {
            admitted,
            parent: parent.to_owned(),
            directory: final_path.clone(),
            final_path,
            receiver: None,
            verified: true,
            renamed: true,
            limits: super::Limits::default(),
        })
    }
}
fn destination(parent: &Path, a: &AdmittedManifest) -> PathBuf {
    // No world.dat or world_meta.json at this top level: ordinary world scanning
    // cannot accidentally discover the received or working child as a world.
    parent.join(format!(
        "handover-{}-{}",
        hex::encode(a.manifest().world),
        hex::encode(a.manifest().transfer)
    ))
}
fn collision(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(Error::State("destination exists")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::Io),
    }
}
pub(super) fn observe(o: &mut impl ImportObserver, b: ImportBoundary, p: &Path) -> Result<()> {
    o.reached(b, p).map_err(|_| Error::Io)
}
/// A handle this import opened for writing (the archive it received, a
/// member it extracted).
pub(super) fn sync_written(file: &crate::fsync::Written, path: &Path, o: &mut impl ImportObserver) -> Result<()> {
    observe(o, ImportBoundary::FileSync, path)?;
    crate::fsync::sync_written(file).map_err(|_| Error::Io)
}
/// CP16 (CP15 review HIGH-1): an existing import file, re-synced by path
/// through `crate::fsync::sync_file` (write access on Windows, where a
/// read-only handle can't be flushed), after the same regular-file check
/// `files::open` makes.
pub(super) fn sync_existing(path: &Path, o: &mut impl ImportObserver) -> Result<()> {
    observe(o, ImportBoundary::FileSync, path)?;
    // CP17 (CP16 review LOW-6): the file synced is the regular file
    // checked (no symlink followed; same device and inode on Unix).
    crate::fsync::sync_regular(path).map_err(|_| Error::Io)
}
fn write_file(
    path: &Path,
    mut reader: impl Read,
    length: u64,
    o: &mut impl ImportObserver,
    pool: Option<&mut SyncPool>,
) -> Result<Hash> {
    observe(o, ImportBoundary::Write, path)?;
    let mut file = files::new_written(path)?;
    let mut hash = Sha256::new();
    let mut left = length;
    let mut buf = [0u8; files::BUFFER];
    while left > 0 {
        let want = usize::try_from(left.min(buf.len() as u64)).map_err(|_| Error::Io)?;
        let n = reader.read(&mut buf[..want]).map_err(|_| Error::Io)?;
        if n == 0 {
            return Err(Error::Invalid("short member"));
        }
        file.write_all(&buf[..n]).map_err(|_| Error::Io)?;
        hash.update(&buf[..n]);
        left -= n as u64;
    }
    let mut tail = [0; 1];
    if reader.read(&mut tail).map_err(|_| Error::Io)? != 0 {
        return Err(Error::Invalid("long member"));
    }
    if let Some(pool) = pool {
        observe(o, ImportBoundary::FileSync, path)?;
        pool.submit(file)?;
    } else {
        sync_written(&file, path, o)?;
    }
    Ok(hash.finalize().into())
}
struct Limited<R> {
    inner: R,
    left: u64,
}
impl<R: Read> Read for Limited<R> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        if b.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let mut tail = [0];
            if self.inner.read(&mut tail)? != 0 {
                return Err(std::io::Error::other("decompression cap"));
            }
            return Ok(0);
        }
        let len = b.len().min(self.left as usize);
        let n = self.inner.read(&mut b[..len])?;
        self.left -= n as u64;
        Ok(n)
    }
}
fn extract(
    directory: &Path,
    a: &AdmittedManifest,
    limits: super::Limits,
    o: &mut impl ImportObserver,
) -> Result<()> {
    let raw = directory.join("received");
    let working = directory.join("working");
    fs::create_dir(&raw).map_err(|_| Error::Io)?;
    fs::create_dir(&working).map_err(|_| Error::Io)?;
    let mut syncs = SyncPool::new()?;
    // Physical tar overhead is separately bounded, in addition to manifest's
    // logical512MiB sum. Raw entries prohibit hidden GNU/PAX long-name bodies.
    let cap = limits.logical + (a.manifest().members.len() as u64) * 1024 + 1024;
    let gz = flate2::bufread::GzDecoder::new(std::io::BufReader::with_capacity(
        files::BUFFER,
        files::open(&directory.join("archive.tar.gz"))?,
    ));
    let mut archive = tar::Archive::new(Limited {
        inner: gz,
        left: cap,
    });
    let mut entries = archive
        .entries()
        .map_err(|_| Error::Invalid("tar stream"))?
        .raw(true);
    for expected in &a.manifest().members {
        let mut entry = entries
            .next()
            .ok_or(Error::Invalid("missing archive member"))?
            .map_err(|_| Error::Invalid("tar member"))?;
        let path = entry.path_bytes();
        if path.as_ref() != expected.path.as_bytes()
            || !entry.header().entry_type().is_file()
            || entry
                .header()
                .size()
                .map_err(|_| Error::Invalid("member header"))?
                != expected.length
        {
            return Err(Error::Invalid("member path/type/length/order"));
        }
        let dest = raw.join(&expected.path);
        create_parent(&raw, &dest)?;
        let hash = write_file(&dest, &mut entry, expected.length, o, Some(&mut syncs))?;
        if hash != expected.digest {
            return Err(Error::Invalid("member hash"));
        }
        validate_member(&dest, expected, a.manifest())?;
    }
    if entries.next().is_some() {
        return Err(Error::Invalid("extra archive member"));
    }
    // Drain all gzip bytes: CRC/truncation and hidden decompressed suffix fail.
    let mut rest = archive.into_inner();
    let mut buf = [0; files::BUFFER];
    loop {
        let n = rest
            .read(&mut buf)
            .map_err(|_| Error::Invalid("gzip tail"))?;
        if n == 0 {
            break;
        }
        if buf[..n].iter().any(|b| *b != 0) {
            return Err(Error::Invalid("tar trailing bytes"));
        }
    }
    validate_references(&raw, a)?;
    let mut compressed_tail = rest.inner.into_inner();
    let mut trailing = [0u8; 1];
    if compressed_tail.read(&mut trailing).map_err(|_| Error::Io)? != 0 {
        return Err(Error::Invalid("compressed trailing bytes"));
    }
    for member in &a.manifest().members {
        let from = raw.join(&member.path);
        let to = working.join(&member.path);
        create_parent(&working, &to)?;
        if member.kind == MemberKind::WorldMeta {
            let bytes = files::read(&from, 2 * 1024 * 1024)?;
            let mut meta = super::bounded_decode::metadata(&bytes)?;
            validate_metadata(&meta)?;
            meta.pop_secret = Some(crate::proof_of_play::gen_world_secret());
            // Receiver's persona is established by the signed offer; runtime
            // keys never enter metadata. Legacy owner field stores hex internally.
            meta.owner_pubkey = Some(hex::encode(a.core().successor_persona));
            let mut encoded = files::Count {
                inner: Vec::new(),
                bytes: 0,
                limit: 2 * 1024 * 1024,
            };
            serde_json::to_writer(&mut encoded, &meta)
                .map_err(|_| Error::Limit("receiver metadata bytes"))?;
            if encoded.inner.capacity() > 2 * 1024 * 1024 {
                return Err(Error::Limit("receiver metadata allocation"));
            }
            let json = encoded.inner;
            write_file(&to, json.as_slice(), json.len() as u64, o, Some(&mut syncs))?;
        } else {
            write_file(&to, files::open(&from)?, member.length, o, Some(&mut syncs))?;
        }
    }
    let evidence = a.evidence();
    write_file(
        &directory.join("admission.dat"),
        evidence,
        evidence.len() as u64,
        o,
        Some(&mut syncs),
    )?;
    let manifest = a.manifest().encode()?;
    write_file(
        &directory.join("manifest.dat"),
        manifest.as_slice(),
        manifest.len() as u64,
        o,
        Some(&mut syncs),
    )?;
    // Every received and working file is durable and every worker has joined
    // before the receipt is created, directory syncing or a rename can begin.
    syncs.finish()?;
    // Promotion removes the enclosing import. Keep the exact admitted
    // manifest inside working so later identity-selected bootstrap can
    // verify the immutable participant members against Activated authority.
    let proof_dir = working.join("handover");
    fs::create_dir(&proof_dir).map_err(|_| Error::Io)?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&proof_dir, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Io)?;
    }
    write_file(&proof_dir.join("personal-manifest.dat"), manifest.as_slice(), manifest.len() as u64, o, None)?;
    let meta = files::digest(
        files::open(&working.join("world_meta.json"))?,
        2 * 1024 * 1024,
    )?
    .1;
    let receipt = receipt_bytes(a, meta, super::super::hash(a.evidence()))?;
    write_file(
        &directory.join("receipt.dat"),
        receipt.as_slice(),
        receipt.len() as u64,
        o,
        None,
    )?;
    Ok(())
}
fn validate_member(path: &Path, member: &Member, manifest: &Manifest) -> Result<()> {
    let (kind, schema) = (member.kind, member.schema);
    match kind {
        MemberKind::WorldData => {
            if schema != crate::save_format::SAVE_FORMAT_VERSION {
                return Err(Error::Invalid("save schema"));
            }
            super::bounded_decode::world(&files::read(path, super::bounded_decode::WORLD_BYTES)?)?;
        }
        MemberKind::WorldMeta => validate_metadata(&super::bounded_decode::metadata(
            &files::read(path, 2 * 1024 * 1024)?,
        )?)?,
        MemberKind::Chunk => {
            let bytes = files::read(path, 8704)?;
            if crate::chunk::Chunk::from_bytes(&bytes).is_none() {
                return Err(Error::Invalid("chunk payload"));
            }
        }
        MemberKind::Sidecar => super::bounded_decode::sidecar(&files::read(path, 65536)?)?,
        MemberKind::Progression => {
            if ![1, 2].contains(&schema) {
                return Err(Error::Invalid("participant schema"));
            }
            let npub = member
                .path
                .strip_prefix("participants/")
                .and_then(|n| n.strip_suffix(".dat"))
                .ok_or(Error::Invalid("participant path"))?;
            let context = super::super::personal::RecordContext {
                participant: super::super::context::canonical_npub(npub)?,
                lineage: manifest.lineage,
                generation: manifest.generation,
                transfer: manifest.transfer,
            };
            let bytes = files::read(path, super::super::personal::RECORD_LIMIT)?;
            if super::super::personal::PersonalRecord::declared_schema(&bytes)? != schema { return Err(Error::Invalid("participant payload schema")); }
            super::super::personal::PersonalRecord::decode(&bytes, &context)?;
        }
        _ => {}
    }
    Ok(())
}
fn create_parent(root: &Path, path: &Path) -> Result<()> {
    let parent = path.parent().ok_or(Error::Io)?;
    if parent != root {
        match fs::create_dir(parent) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if !fs::symlink_metadata(parent)
                    .map_err(|_| Error::Io)?
                    .is_dir()
                {
                    return Err(Error::Invalid("member directory"));
                }
            }
            Err(_) => return Err(Error::Io),
        }
    }
    Ok(())
}

fn validate_references(raw: &Path, a: &AdmittedManifest) -> Result<()> {
    let save = super::bounded_decode::world(&files::read(
        &raw.join("world.dat"),
        super::bounded_decode::WORLD_BYTES,
    )?)?;
    if save.exhibits.len() > 1024 {
        return Err(Error::Limit("world exhibit count"));
    }
    for exhibit in &save.exhibits {
        let name = &exhibit.image_ref;
        if name.is_empty()
            || crate::save::sanitize_image_ref(name) != *name
            || !a.manifest().members.iter().any(|m| {
                m.kind == MemberKind::Exhibit
                    && m.path.strip_prefix("exhibits/") == Some(name.as_str())
            })
        {
            return Err(Error::Invalid("required exhibit member absent"));
        }
    }
    for m in &a.manifest().members {
        if m.kind == MemberKind::Exhibit
            && !save
                .exhibits
                .iter()
                .any(|e| m.path.strip_prefix("exhibits/") == Some(e.image_ref.as_str()))
        {
            return Err(Error::Invalid("unreferenced exhibit member"));
        }
    }
    Ok(())
}

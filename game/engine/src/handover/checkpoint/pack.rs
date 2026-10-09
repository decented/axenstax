//! File-backed packer over the actual persisted chunk/save representation.
use super::super::{
    Error, Hash, Id, Result,
    manifest::{
        Agreement, COMPRESSED_LIMIT, DECOMPRESSED_LIMIT, Manifest, Member, MemberKind,
        validate_path,
    },
};
use super::{files, validate_metadata, validate_neutral};
use crate::{
    save::{WorldMeta, WorldSave},
    world::World,
};
use flate2::{Compression, write::GzEncoder};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
/// Explicitly admitted by the future agreement/personal adapter. This utility
/// cannot infer complete missing progression or private Plan ownership.
pub struct ParticipantMember<'a> {
    pub npub: &'a str,
    pub kind: MemberKind,
    pub schema: u32,
    pub bytes: &'a [u8],
}
pub struct CheckpointSource<'a> {
    pub meta: &'a WorldMeta,
    pub save: &'a WorldSave,
    pub world: &'a World,
    pub disk_chunks: Option<&'a Path>,
    pub exhibit_dir: Option<&'a Path>,
    pub participants: &'a [ParticipantMember<'a>],
}
pub struct PackBinding {
    pub source: Hash,
    pub receiver: Hash,
    pub world: Id,
    pub lineage: Id,
    pub transfer: Id,
    pub generation: u64,
    pub agreements: Vec<Agreement>,
}
/// Owns only a compressed file and bounded manifest. Dropping removes its own
/// scratch directory; it never removes a source or received world.
pub struct PackedCheckpoint {
    directory: PathBuf,
    archive: PathBuf,
    manifest: Manifest,
}
impl PackedCheckpoint {
    pub fn archive(&self) -> &Path {
        &self.archive
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
}
impl Drop for PackedCheckpoint {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
/// The disk index holds paths only, never chunks. Current persistable content
/// (including evicted-over-spill precedence) overwrites old disk coordinates;
/// current all-air content suppresses a stale disk member.
pub fn pack(
    source: CheckpointSource<'_>,
    binding: PackBinding,
    scratch_parent: &Path,
) -> Result<PackedCheckpoint> {
    pack_limited(source, binding, scratch_parent, super::Limits::default())
}
pub fn pack_limited(
    source: CheckpointSource<'_>,
    binding: PackBinding,
    scratch_parent: &Path,
    limits: super::Limits,
) -> Result<PackedCheckpoint> {
    limits.validate()?;
    validate_metadata(source.meta)?;
    validate_neutral(source.save)?;
    if source.participants.len() > 8192 || source.save.exhibits.len() > 1024 {
        return Err(Error::Limit("supplied member count"));
    }
    if binding.agreements.len() > 4096
        || binding.agreements.capacity() * std::mem::size_of::<Agreement>() > 8 * 1024 * 1024
    {
        return Err(Error::Limit("agreement allocation"));
    }
    let scratch = Scratch(files::owned_dir(scratch_parent, "handover-pack")?);
    let mut inputs = Index::new();
    let meta = scratch.0.join("meta");
    let dat = scratch.0.join("save");
    write_codec(&meta, 2 * 1024 * 1024, |w| {
        serde_json::to_writer(w, source.meta).map_err(|_| Error::Invalid("metadata encode"))
    })?;
    write_codec(&dat, super::bounded_decode::WORLD_BYTES as u64, |w| {
        bincode::serialize_into(&mut *w, source.save).map_err(|_| Error::Invalid("save encode"))?;
        w.write_all(&crate::save_format::footer_bytes(
            crate::save_format::SAVE_FORMAT_VERSION,
        ))
        .map_err(|_| Error::Io)
    })?;
    inputs.insert(
        "world_meta.json".into(),
        Input::File(meta, MemberKind::WorldMeta, 1),
    )?;
    inputs.insert(
        "world.dat".into(),
        Input::File(
            dat,
            MemberKind::WorldData,
            crate::save_format::SAVE_FORMAT_VERSION,
        ),
    )?;
    if let Some(dir) = source.disk_chunks {
        if !fs::symlink_metadata(dir).map_err(|_| Error::Io)?.is_dir() {
            return Err(Error::Invalid("chunk directory"));
        }
        for (n, entry) in fs::read_dir(dir).map_err(|_| Error::Io)?.enumerate() {
            if n >= 65534 {
                return Err(Error::Limit("chunk directory entries"));
            }
            let entry = entry.map_err(|_| Error::Io)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::Invalid("chunk filename"))?;
            let path = format!("chunks/{name}");
            let file = files::open(&entry.path())?;
            let length = file.metadata().map_err(|_| Error::Io)?.len();
            check_member(&path, MemberKind::Chunk, length, 1)?;
            inputs.insert(path, Input::File(entry.path(), MemberKind::Chunk, 1))?;
        }
    }
    for (n, ((x, y, z), chunk)) in source.world.persistable_chunks().enumerate() {
        if n >= 65534 {
            return Err(Error::Limit("snapshot chunks"));
        }
        let path = format!("chunks/{x}_{y}_{z}.chunk");
        if chunk.is_empty() {
            inputs.remove(&path);
        } else {
            inputs.insert(path, Input::Chunk(chunk))?;
        }
    }
    for row in source.participants {
        if row.kind == MemberKind::Sidecar {
            super::bounded_decode::sidecar(row.bytes)?;
        }
        let prefix = match row.kind {
            MemberKind::Sidecar => "joiners",
            MemberKind::Progression => "participants",
            _ => return Err(Error::Invalid("participant kind")),
        };
        let path = format!("{prefix}/{}.dat", row.npub);
        check_member(&path, row.kind, row.bytes.len() as u64, row.schema)?;
        if inputs
            .insert(path, Input::Bytes(row.bytes, row.kind, row.schema))?
            .is_some()
        {
            return Err(Error::Invalid("duplicate participant"));
        }
    }
    for exhibit in &source.save.exhibits {
        let name = &exhibit.image_ref;
        if name.is_empty() || crate::save::sanitize_image_ref(name) != *name {
            return Err(Error::Invalid("exhibit reference"));
        }
        let path = format!("exhibits/{name}");
        if inputs.contains_key(&path) {
            continue;
        }
        let dir = source
            .exhibit_dir
            .ok_or(Error::Invalid("missing exhibits"))?;
        let file_path = dir.join(name);
        let file = files::open(&file_path)?;
        check_member(
            &path,
            MemberKind::Exhibit,
            file.metadata().map_err(|_| Error::Io)?.len(),
            1,
        )?;
        inputs.insert(path, Input::File(file_path, MemberKind::Exhibit, 1))?;
    }
    if inputs.rows.len() > limits.members {
        return Err(Error::Limit("member count"));
    }
    let archive = scratch.0.join("checkpoint.tar.gz");
    let output = files::new_file(&archive)?;
    let count = files::Count {
        inner: output,
        bytes: 0,
        limit: limits.compressed,
    };
    let mut gz = GzEncoder::new(count, Compression::default());
    let mut members = Vec::with_capacity(inputs.rows.len());
    let mut total = 0u64;
    {
        let mut ar = tar::Builder::new(&mut gz);
        for (path, input) in inputs.rows {
            let (kind, schema) = input.kind_schema();
            let (length, digest) = match &input {
                Input::File(p, _, _) => files::digest(files::open(p)?, DECOMPRESSED_LIMIT)?,
                Input::Bytes(b, _, _) => (b.len() as u64, super::super::hash(b)),
                Input::Chunk(c) => {
                    let mut bytes = Vec::with_capacity(8704);
                    c.feed_bytes(|b| bytes.extend_from_slice(b));
                    (bytes.len() as u64, super::super::hash(&bytes))
                }
            };
            check_member(&path, kind, length, schema)?;
            total = total
                .checked_add(length)
                .ok_or(Error::Limit("logical members"))?;
            if total > limits.logical {
                return Err(Error::Limit("logical members"));
            }
            let mut header = tar::Header::new_ustar();
            header.set_size(length);
            header.set_mode(0o600);
            header.set_cksum();
            match input {
                Input::File(p, _, _) => {
                    let mut reader = files::HashRead {
                        inner: files::open(&p)?,
                        bytes: 0,
                        limit: length,
                        hash: sha2::Sha256::default(),
                    };
                    ar.append_data(&mut header, &path, &mut reader)
                        .map_err(|_| Error::Io)?;
                    if reader.finish() != (length, digest) {
                        return Err(Error::Invalid("source changed during packing"));
                    }
                    Ok(())
                }
                Input::Bytes(b, _, _) => ar.append_data(&mut header, &path, Cursor::new(b)),
                Input::Chunk(c) => ar.append_data(&mut header, &path, Cursor::new(c.as_bytes())),
            }
            .map_err(|_| Error::Io)?;
            members.push(Member {
                path,
                kind,
                length,
                digest,
                schema,
            });
        }
        ar.finish().map_err(|_| Error::Io)?;
    }
    let mut output = gz.finish().map_err(|_| Error::Io)?.inner;
    output.flush().map_err(|_| Error::Io)?;
    output.seek(SeekFrom::Start(0)).map_err(|_| Error::Io)?;
    let (stream_length, stream_hash) = files::digest(output, COMPRESSED_LIMIT)?;
    let manifest = Manifest {
        source: binding.source,
        receiver: binding.receiver,
        world: binding.world,
        lineage: binding.lineage,
        transfer: binding.transfer,
        generation: binding.generation,
        agreements: binding.agreements,
        members,
        stream_length,
        stream_hash,
    };
    manifest.validate()?;
    let directory = scratch.0.clone();
    std::mem::forget(scratch);
    Ok(PackedCheckpoint {
        directory,
        archive,
        manifest,
    })
}
fn write_codec(
    path: &Path,
    limit: u64,
    encode: impl FnOnce(&mut files::Count<std::fs::File>) -> Result<()>,
) -> Result<()> {
    let mut writer = files::Count {
        inner: files::new_file(path)?,
        bytes: 0,
        limit,
    };
    encode(&mut writer)?;
    writer.flush().map_err(|_| Error::Io)
}
enum Input<'a> {
    File(PathBuf, MemberKind, u32),
    Bytes(&'a [u8], MemberKind, u32),
    Chunk(&'a crate::chunk::Chunk),
}
impl Input<'_> {
    fn kind_schema(&self) -> (MemberKind, u32) {
        match self {
            Self::File(_, k, s) | Self::Bytes(_, k, s) => (*k, *s),
            Self::Chunk(_) => (MemberKind::Chunk, 1),
        }
    }
}
fn check_member(path: &str, kind: MemberKind, length: u64, schema: u32) -> Result<()> {
    validate_path(&Member {
        path: path.to_owned(),
        kind,
        length,
        digest: [1; 32],
        schema,
    })?;
    match kind {
        MemberKind::Chunk if schema != 1 || ![8192, 8704].contains(&length) => {
            Err(Error::Invalid("chunk size/schema"))
        }
        MemberKind::Sidecar | MemberKind::Progression if schema != 1 || length > 65536 => {
            Err(Error::Invalid("participant size/schema"))
        }
        MemberKind::Exhibit if length > 8 * 1024 * 1024 => Err(Error::Limit("exhibit bytes")),
        _ => Ok(()),
    }
}

const INDEX_LIMIT: usize = 8 * 1024 * 1024;
struct Index<'a> {
    rows: BTreeMap<String, Input<'a>>,
    allocated: usize,
}
fn allocation(path: &String, input: &Input<'_>) -> usize {
    let extra = match input {
        Input::File(p, _, _) => p.capacity(),
        _ => 0,
    };
    path.capacity() + extra + std::mem::size_of::<Input<'_>>() + 160
}
impl<'a> Index<'a> {
    fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
            allocated: 0,
        }
    }
    fn insert(&mut self, path: String, input: Input<'a>) -> Result<Option<Input<'a>>> {
        let old = self
            .rows
            .get_key_value(&path)
            .map(|(p, i)| allocation(p, i))
            .unwrap_or(0);
        let next = self
            .allocated
            .checked_sub(old)
            .and_then(|n| n.checked_add(allocation(&path, &input)))
            .ok_or(Error::Limit("member index allocation"))?;
        if next > INDEX_LIMIT || (self.rows.len() >= 65536 && old == 0) {
            return Err(Error::Limit("member index allocation"));
        }
        self.allocated = next;
        Ok(self.rows.insert(path, input))
    }
    fn remove(&mut self, path: &str) {
        if let Some((p, i)) = self.rows.remove_entry(path) {
            self.allocated -= allocation(&p, &i);
        }
    }
    fn contains_key(&self, path: &str) -> bool {
        self.rows.contains_key(path)
    }
}

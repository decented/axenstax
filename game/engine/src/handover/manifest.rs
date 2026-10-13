//! Known-member checkpoint manifests. This does not extract an archive.
use super::{
    Error, Hash, Id, Result,
    codec::{Reader, frame},
    context::canonical_npub,
};
use std::collections::BTreeSet;
pub const MANIFEST_LIMIT: usize = 2 * 1024 * 1024;
pub const MANIFEST_RESIDENT_LIMIT: usize = 8 * 1024 * 1024;
pub const COMPRESSED_LIMIT: u64 = 128 * 1024 * 1024;
pub const DECOMPRESSED_LIMIT: u64 = 512 * 1024 * 1024;
const DOMAIN: &str = "AxeNStax/handover/manifest/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    WorldMeta,
    WorldData,
    Chunk,
    Sidecar,
    Progression,
    Exhibit,
}
impl MemberKind {
    fn number(self) -> u8 {
        match self {
            Self::WorldMeta => 0,
            Self::WorldData => 1,
            Self::Chunk => 2,
            Self::Sidecar => 3,
            Self::Progression => 4,
            Self::Exhibit => 5,
        }
    }
    fn from_number(n: u8) -> Result<Self> {
        match n {
            0 => Ok(Self::WorldMeta),
            1 => Ok(Self::WorldData),
            2 => Ok(Self::Chunk),
            3 => Ok(Self::Sidecar),
            4 => Ok(Self::Progression),
            5 => Ok(Self::Exhibit),
            _ => Err(Error::Invalid("member type")),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub path: String,
    pub kind: MemberKind,
    pub length: u64,
    pub digest: Hash,
    pub schema: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agreement {
    pub participant: Hash,
    pub proof: Hash,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub source: Hash,
    pub receiver: Hash,
    pub world: Id,
    pub lineage: Id,
    pub transfer: Id,
    pub generation: u64,
    pub stream_length: u64,
    pub stream_hash: Hash,
    pub agreements: Vec<Agreement>,
    pub members: Vec<Member>,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.members.len() > 65536 || self.agreements.len() > 4096 {
            return Err(Error::Limit("manifest entries"));
        }
        if self.world == [0; 16]
            || self.lineage == [0; 16]
            || self.transfer == [0; 16]
            || self.stream_hash == [0; 32]
            || self.stream_length == 0
            || self.stream_length > COMPRESSED_LIMIT
        {
            return Err(Error::Invalid("manifest binding"));
        }
        for key in [self.source, self.receiver] {
            nostr::PublicKey::from_slice(&key)
                .and_then(|key| {
                    key.xonly()?;
                    Ok(key)
                })
                .map_err(|_| Error::Invalid("manifest identity"))?;
        }
        let mut agreed = BTreeSet::new();
        let mut previous = None;
        for a in &self.agreements {
            nostr::PublicKey::from_slice(&a.participant)
                .and_then(|key| {
                    key.xonly()?;
                    Ok(key)
                })
                .map_err(|_| Error::Invalid("agreement identity"))?;
            if a.proof == [0; 32]
                || previous.is_some_and(|p| p >= a.participant)
                || !agreed.insert(a.participant)
            {
                return Err(Error::Invalid("agreement order/proof"));
            }
            previous = Some(a.participant);
        }
        let mut prior: Option<&str> = None;
        let mut total = 0u64;
        let mut images = 0;
        let mut meta = false;
        let mut data = false;
        let mut participants = BTreeSet::new();
        let mut allocated = self
            .members
            .capacity()
            .checked_mul(std::mem::size_of::<Member>())
            .ok_or(Error::Limit("manifest allocation"))?
            + self.agreements.capacity() * std::mem::size_of::<Agreement>();
        for m in &self.members {
            if prior.is_some_and(|p| p >= m.path.as_str()) {
                return Err(Error::Invalid("member order/duplicate"));
            }
            prior = Some(&m.path);
            validate_path(m)?;
            allocated = allocated
                .checked_add(m.path.capacity())
                .ok_or(Error::Limit("manifest allocation"))?;
            if allocated > MANIFEST_RESIDENT_LIMIT {
                return Err(Error::Limit("manifest allocation"));
            }
            if m.digest == [0; 32] || m.schema == 0 {
                return Err(Error::Invalid("member hash/schema"));
            }
            total = total
                .checked_add(m.length)
                .ok_or(Error::Limit("decompressed members"))?;
            if total > DECOMPRESSED_LIMIT {
                return Err(Error::Limit("decompressed members"));
            }
            match m.kind {
                MemberKind::WorldMeta => {
                    meta = true;
                    if m.schema != 1 {
                        return Err(Error::Invalid("metadata schema"));
                    }
                }
                MemberKind::WorldData => {
                    data = true;
                    if m.schema > crate::save_format::SAVE_FORMAT_VERSION {
                        return Err(Error::Invalid("newer world schema"));
                    }
                }
                MemberKind::Chunk => {
                    if m.schema != 1 || ![8192, 8704].contains(&m.length) {
                        return Err(Error::Invalid("chunk schema/size"));
                    }
                }
                MemberKind::Sidecar | MemberKind::Progression => {
                    if m.length > 65536 || m.schema != 1 {
                        return Err(Error::Invalid("participant schema/size"));
                    }
                    let name = m
                        .path
                        .rsplit('/')
                        .next()
                        .ok_or(Error::Invalid("participant path"))?;
                    let npub = name
                        .strip_suffix(".dat")
                        .ok_or(Error::Invalid("participant filename"))?;
                    let pk = canonical_npub(npub)?;
                    participants.insert(pk);
                    if !agreed.contains(&pk) {
                        return Err(Error::Invalid("participant agreement absent"));
                    }
                }
                MemberKind::Exhibit => {
                    images += 1;
                    if m.schema != 1 || m.length > 8 * 1024 * 1024 || images > 1024 {
                        return Err(Error::Limit("exhibit images"));
                    }
                }
            }
        }
        if participants.len() > 4096 {
            return Err(Error::Limit("participants"));
        }
        if !meta || !data {
            return Err(Error::Invalid("missing required world members"));
        }
        if self.encoded_size()? > MANIFEST_LIMIT {
            return Err(Error::Limit("encoded manifest"));
        }
        Ok(())
    }
    fn encoded_size(&self) -> Result<usize> {
        let base = 4 + DOMAIN.len() + 4 + 64 + 48 + 16 + 32 + 4 + 4;
        let mut n = base + self.agreements.len() * 64;
        for m in &self.members {
            n = n
                .checked_add(4 + m.path.len() + 1 + 8 + 32 + 4)
                .ok_or(Error::Limit("manifest bytes"))?;
        }
        Ok(n)
    }
    fn encode_unchecked(&self) -> Vec<u8> {
        let mut v = Vec::new();
        frame(&mut v, DOMAIN.as_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        for a in [self.source, self.receiver] {
            v.extend_from_slice(&a);
        }
        for a in [self.world, self.lineage, self.transfer] {
            v.extend_from_slice(&a);
        }
        for n in [self.generation, self.stream_length] {
            v.extend_from_slice(&n.to_be_bytes());
        }
        v.extend_from_slice(&self.stream_hash);
        v.extend_from_slice(&(self.agreements.len() as u32).to_be_bytes());
        for a in &self.agreements {
            v.extend_from_slice(&a.participant);
            v.extend_from_slice(&a.proof);
        }
        v.extend_from_slice(&(self.members.len() as u32).to_be_bytes());
        for m in &self.members {
            frame(&mut v, m.path.as_bytes());
            v.push(m.kind.number());
            v.extend_from_slice(&m.length.to_be_bytes());
            v.extend_from_slice(&m.digest);
            v.extend_from_slice(&m.schema.to_be_bytes());
        }
        v
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(self.encode_unchecked())
    }
    pub fn digest(&self) -> Result<Hash> {
        Ok(super::hash(&self.encode()?))
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, MANIFEST_LIMIT)?;
        r.label(DOMAIN)?;
        if r.u32()? != 1 {
            return Err(Error::Invalid("manifest version"));
        }
        let source = r.array()?;
        let receiver = r.array()?;
        let world = r.array()?;
        let lineage = r.array()?;
        let transfer = r.array()?;
        let generation = r.u64()?;
        let stream_length = r.u64()?;
        let stream_hash = r.array()?;
        let count = r.u32()? as usize;
        if count > 4096 {
            return Err(Error::Limit("agreements"));
        }
        // Ensure bytes exist before reserving an attacker-provided count.
        if count.checked_mul(64).is_none_or(|n| n > bytes.len()) {
            return Err(Error::Invalid("agreement count"));
        }
        let mut agreements = Vec::with_capacity(count);
        for _ in 0..count {
            agreements.push(Agreement {
                participant: r.array()?,
                proof: r.array()?,
            });
        }
        let count = r.u32()? as usize;
        if count > 65536 || count.checked_mul(49).is_none_or(|n| n > bytes.len()) {
            return Err(Error::Limit("members"));
        }
        let mut members = Vec::with_capacity(count);
        for _ in 0..count {
            let path = std::str::from_utf8(r.framed(240)?)
                .map_err(|_| Error::Invalid("member UTF8"))?
                .to_owned();
            members.push(Member {
                path,
                kind: MemberKind::from_number(r.u8()?)?,
                length: r.u64()?,
                digest: r.array()?,
                schema: r.u32()?,
            });
        }
        r.end()?;
        let m = Self {
            source,
            receiver,
            world,
            lineage,
            transfer,
            generation,
            stream_length,
            stream_hash,
            agreements,
            members,
        };
        m.validate()?;
        Ok(m)
    }
}
pub fn validate_path(m: &Member) -> Result<()> {
    let path = &m.path;
    if path.is_empty()
        || path.len() > 240
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(Error::Invalid("member path"));
    }
    let valid = match m.kind {
        MemberKind::WorldMeta => path == "world_meta.json",
        MemberKind::WorldData => path == "world.dat",
        MemberKind::Chunk => path
            .strip_prefix("chunks/")
            .and_then(|s| s.strip_suffix(".chunk"))
            .is_some_and(|s| {
                let parts: Vec<_> = s.split('_').collect();
                parts.len() == 3
                    && parts
                        .iter()
                        .all(|p| p.parse::<i32>().is_ok_and(|n| n.to_string() == *p))
            }),
        MemberKind::Sidecar | MemberKind::Progression => {
            let prefix = if m.kind == MemberKind::Sidecar {
                "joiners/"
            } else {
                "participants/"
            };
            path.strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(".dat"))
                .is_some_and(|s| canonical_npub(s).is_ok())
        }
        MemberKind::Exhibit => path.strip_prefix("exhibits/").is_some_and(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_- .".contains(&c))
                && !s.contains('/')
                && !s.starts_with('.')
        }),
    };
    if !valid {
        return Err(Error::Invalid("member allowlist"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Manifest {
        let k = nostr::Keys::generate().public_key().to_bytes();
        Manifest {
            source: k,
            receiver: k,
            world: [1; 16],
            lineage: [2; 16],
            transfer: [3; 16],
            generation: 1,
            stream_length: 12,
            stream_hash: [4; 32],
            agreements: vec![],
            members: vec![
                Member {
                    path: "world.dat".into(),
                    kind: MemberKind::WorldData,
                    length: 12,
                    digest: [5; 32],
                    schema: 52,
                },
                Member {
                    path: "world_meta.json".into(),
                    kind: MemberKind::WorldMeta,
                    length: 12,
                    digest: [6; 32],
                    schema: 1,
                },
            ],
        }
    }
    #[test]
    fn handover_manifest_roundtrip_paths_missing_and_duplicates() {
        let m = fixture();
        let b = m.encode().unwrap();
        assert_eq!(Manifest::decode(&b).unwrap(), m);
        let mut bad = b.clone();
        bad.push(0);
        assert!(Manifest::decode(&bad).is_err());
        let mut m = fixture();
        m.members.pop();
        assert!(m.validate().is_err());
        let mut m = fixture();
        m.members.insert(0, m.members[0].clone());
        assert!(m.validate().is_err());
        for path in [
            "../world.dat",
            "/world.dat",
            "chunks/0_0_0.chunk/x",
            "chunks/-0_0_0.chunk",
            "profile/runtime_key.json",
        ] {
            let mut m = fixture();
            m.members[0].path = path.into();
            assert!(m.validate().is_err());
        }
    }
    #[test]
    fn handover_manifest_requires_consent_and_checks_memory_and_schema() {
        use nostr::ToBech32;
        let k = nostr::Keys::generate().public_key();
        let mut m = fixture();
        m.members.push(Member {
            path: format!("joiners/{}.dat", k.to_bech32().unwrap()),
            kind: MemberKind::Sidecar,
            length: 65536,
            digest: [7; 32],
            schema: 1,
        });
        m.members.sort_by(|a, b| a.path.cmp(&b.path));
        assert!(m.validate().is_err());
        m.agreements.push(Agreement {
            participant: k.to_bytes(),
            proof: [8; 32],
        });
        assert!(m.validate().is_ok());
        m.members
            .iter_mut()
            .find(|x| x.kind == MemberKind::Sidecar)
            .unwrap()
            .length = 65537;
        assert!(m.validate().is_err());
        let mut m = fixture();
        m.members[0].schema = u32::MAX;
        assert!(m.validate().is_err());
        let mut m = fixture();
        m.members
            .reserve(MANIFEST_RESIDENT_LIMIT / std::mem::size_of::<Member>() + 1);
        assert!(m.validate().is_err());
    }
}

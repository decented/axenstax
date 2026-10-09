//! Canonical certificates and a bounded, conflict-aware committed prefix.
use super::{
    Error, Hash, Id, Result,
    codec::{Reader, frame},
    context::RootContext,
};
const DOMAIN: &str = "AxeNStax/handover/certificate/v1";
pub const CORE_LIMIT: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Core {
    pub context: Hash,
    pub root_runtime: Hash,
    pub root_persona: Hash,
    pub root_expiry: u64,
    pub lineage: Id,
    pub snapshot_world: Id,
    pub manifest: Hash,
    pub generation: u64,
    pub epoch: u32,
    pub predecessor: Hash,
    pub transfer: Id,
    pub source_runtime: Hash,
    pub source_persona: Hash,
    pub successor_runtime: Hash,
    pub successor_persona: Hash,
    pub issued_at: u64,
    pub not_before: u64,
    pub expires_at: u64,
    pub remaining_hops: u8,
}
impl Core {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        frame(&mut v, DOMAIN.as_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        for a in [self.context, self.root_runtime, self.root_persona] {
            v.extend_from_slice(&a);
        }
        v.extend_from_slice(&self.root_expiry.to_be_bytes());
        v.extend_from_slice(&self.lineage);
        v.extend_from_slice(&self.snapshot_world);
        v.extend_from_slice(&self.manifest);
        v.extend_from_slice(&self.generation.to_be_bytes());
        v.extend_from_slice(&self.epoch.to_be_bytes());
        v.extend_from_slice(&self.predecessor);
        v.extend_from_slice(&self.transfer);
        for a in [
            self.source_runtime,
            self.source_persona,
            self.successor_runtime,
            self.successor_persona,
        ] {
            v.extend_from_slice(&a);
        }
        for n in [self.issued_at, self.not_before, self.expires_at] {
            v.extend_from_slice(&n.to_be_bytes());
        }
        v.push(self.remaining_hops);
        v.push(0);
        v
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, CORE_LIMIT)?;
        r.label(DOMAIN)?;
        if r.u32()? != 1 {
            return Err(Error::Invalid("certificate version"));
        }
        let core = Self {
            context: r.array()?,
            root_runtime: r.array()?,
            root_persona: r.array()?,
            root_expiry: r.u64()?,
            lineage: r.array()?,
            snapshot_world: r.array()?,
            manifest: r.array()?,
            generation: r.u64()?,
            epoch: r.u32()?,
            predecessor: r.array()?,
            transfer: r.array()?,
            source_runtime: r.array()?,
            source_persona: r.array()?,
            successor_runtime: r.array()?,
            successor_persona: r.array()?,
            issued_at: r.u64()?,
            not_before: r.u64()?,
            expires_at: r.u64()?,
            remaining_hops: r.u8()?,
        };
        if r.u8()? != 0 {
            return Err(Error::Invalid("certificate scope"));
        }
        r.end()?;
        Ok(core)
    }
    pub fn digest(&self) -> Hash {
        super::hash(&self.encode())
    }
    pub fn validate(&self, root: &RootContext, now: u64) -> Result<()> {
        if self.context != root.digest()
            || self.root_runtime != root.runtime()
            || self.root_persona != root.persona()
            || self.root_expiry != root.expiry()
        {
            return Err(Error::Invalid("root binding"));
        }
        if self.epoch == 0 || self.epoch > 4 || self.remaining_hops != (4 - self.epoch) as u8 {
            return Err(Error::Invalid("delegation depth"));
        }
        if self.issued_at > now.saturating_add(120)
            || self.not_before > now
            || self.issued_at > self.expires_at
            || self.not_before > self.expires_at
            || now > self.expires_at
            || self.expires_at > root.expiry()
        {
            return Err(Error::Invalid("certificate time"));
        }
        if self.lineage == [0; 16]
            || self.snapshot_world == [0; 16]
            || self.transfer == [0; 16]
            || self.manifest == [0; 32]
        {
            return Err(Error::Invalid("empty certificate binding"));
        }
        for key in [
            self.source_runtime,
            self.source_persona,
            self.successor_runtime,
            self.successor_persona,
        ] {
            nostr::PublicKey::from_slice(&key).map_err(|_| Error::Invalid("certificate key"))?;
        }
        Ok(())
    }
}

/// Only crypto-verified Committed envelopes produce this value.
#[derive(Clone, Debug)]
pub struct VerifiedCommit {
    pub(crate) core: Core,
    pub(crate) event_hash: Hash,
    pub(crate) prepared_hash: Hash,
}
impl VerifiedCommit {
    pub fn core(&self) -> &Core {
        &self.core
    }
    pub fn event_hash(&self) -> Hash {
        self.event_hash
    }
    pub fn prepared_hash(&self) -> Hash {
        self.prepared_hash
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefix {
    entries: Vec<(Hash, Core)>,
    conflict: bool,
    verified: bool,
}
impl Default for Prefix {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            conflict: false,
            verified: true,
        }
    }
}
impl Prefix {
    pub fn is_verified(&self) -> bool {
        self.verified
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn latest_hash(&self) -> Option<Hash> {
        self.entries.last().map(|e| e.0)
    }
    pub fn is_conflicted(&self) -> bool {
        self.conflict
    }
    pub fn accept(&mut self, proof: &VerifiedCommit, root: &RootContext, now: u64) -> Result<bool> {
        if !self.verified {
            return Err(Error::State("cached prefix is not routing proof"));
        }
        if self.conflict {
            return Err(Error::State("conflicting committed prefix"));
        }
        proof.core.validate(root, now)?;
        if let Some((hash, old)) = self
            .entries
            .iter()
            .find(|(_, c)| c.epoch == proof.core.epoch)
        {
            if hash == &proof.event_hash && old == &proof.core {
                return Ok(false);
            }
            self.conflict = true;
            return Err(Error::Invalid("conflicting committed records"));
        }
        if self.entries.len() >= 4 {
            return Err(Error::Limit("committed depth"));
        }
        let c = &proof.core;
        if let Some((prev_hash, prev)) = self.entries.last() {
            if c.epoch != prev.epoch + 1
                || c.predecessor != *prev_hash
                || c.source_runtime != prev.successor_runtime
                || c.source_persona != prev.successor_persona
                || c.lineage != prev.lineage
                || c.expires_at > prev.expires_at
                || c.generation <= prev.generation
                || self.entries.iter().any(|(h, _)| *h == proof.event_hash)
            {
                return Err(Error::Invalid("committed predecessor"));
            }
        } else if c.epoch != 1
            || c.predecessor != [0; 32]
            || c.source_runtime != root.runtime()
            || c.source_persona != root.persona()
        {
            return Err(Error::Invalid("root successor"));
        }
        self.entries.push((proof.event_hash, c.clone()));
        Ok(true)
    }
    pub(crate) fn can_advance_to(&self, next: &Self) -> bool {
        (!self.conflict || next.conflict)
            && next.entries.len() >= self.entries.len()
            && next.entries[..self.entries.len()] == self.entries
    }
    pub fn persist(&self, store: &super::durable::MarkerStore, cache_id: Id) -> Result<()> {
        if !self.verified {
            return Err(Error::State("unverified prefix persistence"));
        }
        store.persist(
            super::durable::MarkerKind::Prefix,
            cache_id,
            &self.checkpoint(),
        )
    }
    /// A restored digest cache is not a routing proof. First reconstruct a
    /// crypto-verified chain from the root; it must include this known prefix.
    pub fn confirmed_by(&self, verified_chain: &Self) -> Result<()> {
        if !verified_chain.verified
            || self.conflict
            || verified_chain.conflict
            || !self.can_advance_to(verified_chain)
        {
            return Err(Error::Invalid("known committed rollback/conflict"));
        }
        Ok(())
    }
    /// Persist canonical already-verified prefix, then reverify all envelopes on
    /// restart before using it for routing. This digest is rollback context only.
    pub fn checkpoint(&self) -> Vec<u8> {
        let mut out = Vec::new();
        frame(&mut out, b"AxeNStax/handover/prefix/v1");
        out.push(self.entries.len() as u8);
        out.push(u8::from(self.conflict));
        for (h, c) in &self.entries {
            out.extend_from_slice(h);
            frame(&mut out, &c.encode());
        }
        out
    }
    pub fn restore(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, 8192)?;
        r.label("AxeNStax/handover/prefix/v1")?;
        let n = r.u8()?;
        if n > 4 {
            return Err(Error::Limit("prefix"));
        }
        let conflict = match r.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::Invalid("prefix flag")),
        };
        let mut entries = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let h = r.array()?;
            let c = Core::decode(r.framed(CORE_LIMIT)?)?;
            entries.push((h, c));
        }
        r.end()?;
        for (i, (_, c)) in entries.iter().enumerate() {
            if c.epoch != i as u32 + 1 || (i > 0 && c.predecessor != entries[i - 1].0) {
                return Err(Error::Invalid("stored prefix"));
            }
        }
        Ok(Self {
            entries,
            conflict,
            verified: false,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture() -> (RootContext, Core) {
        use nostr::ToBech32;
        let a = nostr::Keys::generate();
        let b = nostr::Keys::generate();
        let root = RootContext::new(
            a.public_key().to_bytes(),
            &a.public_key().to_bech32().unwrap(),
            900,
            &["wss://relay.example".into()],
            100,
        )
        .unwrap();
        let c = Core {
            context: root.digest(),
            root_runtime: root.runtime(),
            root_persona: root.persona(),
            root_expiry: 900,
            lineage: [1; 16],
            snapshot_world: [2; 16],
            manifest: [3; 32],
            generation: 1,
            epoch: 1,
            predecessor: [0; 32],
            transfer: [4; 16],
            source_runtime: root.runtime(),
            source_persona: root.persona(),
            successor_runtime: b.public_key().to_bytes(),
            successor_persona: b.public_key().to_bytes(),
            issued_at: 100,
            not_before: 100,
            expires_at: 800,
            remaining_hops: 3,
        };
        (root, c)
    }
    #[test]
    fn handover_core_rejects_truncation_trailing_scope_and_version() {
        let (_, c) = fixture();
        let bytes = c.encode();
        assert_eq!(Core::decode(&bytes).unwrap(), c);
        for i in 0..bytes.len() {
            assert!(Core::decode(&bytes[..i]).is_err());
        }
        let mut bad = bytes.clone();
        bad.push(0);
        assert!(Core::decode(&bad).is_err());
        let mut bad = bytes;
        *bad.last_mut().unwrap() = 1;
        assert!(Core::decode(&bad).is_err());
    }
    #[test]
    fn handover_prefix_persistence_advances_and_rejects_rollback() {
        let (root, c) = fixture();
        let mut p = Prefix::default();
        p.accept(
            &VerifiedCommit {
                core: c.clone(),
                event_hash: [11; 32],
                prepared_hash: [9; 32],
            },
            &root,
            100,
        )
        .unwrap();
        let (store, dir) = crate::handover::durable::tests::store("prefix");
        p.persist(&store, [1; 16]).unwrap();
        let old = p.clone();
        let mut next = c;
        next.epoch = 2;
        next.remaining_hops = 2;
        next.predecessor = [11; 32];
        next.generation = 2;
        next.source_runtime = next.successor_runtime;
        next.source_persona = next.successor_persona;
        p.accept(
            &VerifiedCommit {
                core: next,
                event_hash: [12; 32],
                prepared_hash: [10; 32],
            },
            &root,
            100,
        )
        .unwrap();
        p.persist(&store, [1; 16]).unwrap();
        assert!(old.persist(&store, [1; 16]).is_err());
        assert!(old.confirmed_by(&p).is_ok());
        assert!(p.confirmed_by(&old).is_err());
        let bytes = store
            .read(crate::handover::durable::MarkerKind::Prefix, [1; 16])
            .unwrap()
            .unwrap();
        assert_eq!(
            Prefix::restore(&bytes).unwrap().checkpoint(),
            p.checkpoint()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn handover_core_checked_time_and_binding_boundaries() {
        let (root, c) = fixture();
        assert!(c.validate(&root, 100).is_ok());
        for field in [0, 1, 2, 3, 4] {
            let mut bad = c.clone();
            match field {
                0 => bad.expires_at = 901,
                1 => bad.issued_at = u64::MAX,
                2 => bad.not_before = 101,
                3 => bad.context = [0; 32],
                _ => bad.remaining_hops = 4,
            }
            assert!(bad.validate(&root, 100).is_err());
        }
        assert!(c.validate(&root, 801).is_err());
    }
    #[test]
    fn handover_prefix_conflict_rollback_handback_and_depth() {
        let (root, c) = fixture();
        let mut p = Prefix::default();
        let first = VerifiedCommit {
            core: c.clone(),
            event_hash: [11; 32],
            prepared_hash: [10; 32],
        };
        assert!(p.accept(&first, &root, 100).unwrap());
        assert!(!p.accept(&first, &root, 100).unwrap());
        let mut next = c.clone();
        next.epoch = 2;
        next.remaining_hops = 2;
        next.predecessor = first.event_hash;
        next.source_runtime = c.successor_runtime;
        next.source_persona = c.successor_persona;
        next.successor_runtime = c.source_runtime;
        next.successor_persona = c.source_persona;
        next.generation = 2;
        assert!(
            p.accept(
                &VerifiedCommit {
                    core: next,
                    event_hash: [12; 32],
                    prepared_hash: [10; 32]
                },
                &root,
                100
            )
            .unwrap()
        );
        assert_eq!(
            Prefix::restore(&p.checkpoint()).unwrap().checkpoint(),
            p.checkpoint()
        );
        let mut evil = first;
        evil.event_hash = [13; 32];
        assert!(p.accept(&evil, &root, 100).is_err());
        assert!(p.is_conflicted());
        let mut deep = c;
        deep.epoch = 5;
        assert!(deep.validate(&root, 100).is_err());
    }
}

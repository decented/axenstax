//! Once-only commit model. Durable source retirement creates publication power.
use super::{
    Error, Hash, Id, Result,
    certificate::{Core, VerifiedCommit},
    codec::{Reader, frame},
    crypto::VerifiedPrepared,
    durable::{MarkerKind, MarkerStore},
    stored::StoredProof,
};
use nostr::{Event, JsonUtil};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetiredRoot {
    pub capability: Hash,
    pub expiry: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retirement {
    pub core: Core,
    pub prepared: Hash,
    pub committed: Hash,
    pub exact_event: Vec<u8>,
    pub retained_copy: Id,
    pub active_capability: Hash,
    pub roots: Vec<RetiredRoot>,
}
impl Retirement {
    pub fn new(
        commit: &VerifiedCommit,
        prepared: &VerifiedPrepared,
        exact_event: Vec<u8>,
        retained_copy: Id,
        active_capability: Hash,
        roots: Vec<RetiredRoot>,
    ) -> Result<Self> {
        if commit.core() != prepared.core() || commit.prepared_hash() != prepared.event_hash() {
            return Err(Error::Invalid("retirement Prepared"));
        }
        let r = Self {
            core: commit.core().clone(),
            prepared: prepared.event_hash(),
            committed: commit.event_hash(),
            exact_event,
            retained_copy,
            active_capability,
            roots,
        };
        r.validate()?;
        Ok(r)
    }
    fn validate(&self) -> Result<()> {
        if self.retained_copy == [0; 16]
            || self.retained_copy == self.core.snapshot_world
            || self.retained_copy == self.core.lineage
            || self.roots.is_empty()
            || self.roots.len() > 64
            || self.prepared == [0; 32]
        {
            return Err(Error::Invalid("retirement context"));
        }
        if self.exact_event.len() > super::EVENT_LIMIT
            || self.exact_event.capacity() > super::EVENT_LIMIT
        {
            return Err(Error::Limit("retirement event"));
        }
        let ev =
            Event::from_json(&self.exact_event).map_err(|_| Error::Invalid("retirement event"))?;
        if ev.verify().is_err()
            || ev.id.to_bytes() != self.committed
            || ev.pubkey.to_bytes() != self.core.source_runtime
            || ev.kind != nostr::Kind::Custom(super::STORED_KIND)
        {
            return Err(Error::Invalid("retirement signature"));
        }
        let mut previous = None;
        let mut bound = false;
        for root in &self.roots {
            if root.capability == [0; 32] || previous.is_some_and(|p| p >= root.capability) {
                return Err(Error::Invalid("retired root order"));
            }
            previous = Some(root.capability);
            if root.capability == self.active_capability && root.expiry == self.core.root_expiry {
                bound = true;
            }
        }
        if !bound {
            return Err(Error::Invalid("retired active root missing"));
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut v = Vec::new();
        frame(&mut v, b"AxeNStax/handover/retirement/v1");
        frame(&mut v, &self.core.encode());
        v.extend_from_slice(&self.prepared);
        v.extend_from_slice(&self.committed);
        frame(&mut v, &self.exact_event);
        v.extend_from_slice(&self.retained_copy);
        v.extend_from_slice(&self.active_capability);
        v.extend_from_slice(&(self.roots.len() as u32).to_be_bytes());
        for root in &self.roots {
            v.extend_from_slice(&root.capability);
            v.extend_from_slice(&root.expiry.to_be_bytes());
        }
        Ok(v)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, super::durable::MARKER_LIMIT)?;
        r.label("AxeNStax/handover/retirement/v1")?;
        let core = Core::decode(r.framed(super::certificate::CORE_LIMIT)?)?;
        let prepared = r.array()?;
        let committed = r.array()?;
        let exact_event = r.framed(super::EVENT_LIMIT)?.to_vec();
        let retained_copy = r.array()?;
        let active_capability = r.array()?;
        let n = r.u32()?;
        if n == 0 || n > 64 {
            return Err(Error::Limit("retired roots"));
        }
        let mut roots = Vec::with_capacity(n as usize);
        for _ in 0..n {
            roots.push(RetiredRoot {
                capability: r.array()?,
                expiry: r.u64()?,
            });
        }
        r.end()?;
        let value = Self {
            core,
            prepared,
            committed,
            exact_event,
            retained_copy,
            active_capability,
            roots,
        };
        value.validate()?;
        Ok(value)
    }
    /// Check before contact/bearer shortcuts, including an expired old link:
    /// legacy contact admission ignores bearer expiry. Bound exhaustion fails
    /// closed rather than evicting retired capabilities or lineage retirement.
    pub fn refuses_root(&self, context: Hash, _now: u64) -> bool {
        self.roots.iter().any(|r| r.capability == context)
    }
    pub fn retained_admission(
        &self,
        expected_world: Id,
        offered_context: Option<Hash>,
        already_contact: bool,
        blocked: bool,
    ) -> bool {
        !blocked
            && expected_world == self.retained_copy
            && already_contact
            && !offered_context.is_some_and(|context| self.refuses_root(context, 0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStage {
    Frozen,
    Prepared,
    RetirementUncertain,
    Retired,
    Published,
    Aborted,
}
/// Verified local authority context, independent of folder/autosave and player
/// slot. Root starts at0; committed successor epoch1 is a distinct authority.
#[derive(Clone, Debug)]
pub struct Authority {
    lineage: Id,
    epoch: u32,
    committed: Hash,
    runtime: Hash,
    persona: Hash,
}
impl Authority {
    pub fn root(root: &super::context::RootContext, lineage: Id) -> Result<Self> {
        if lineage == [0; 16] {
            return Err(Error::Invalid("root lineage"));
        }
        Ok(Self {
            lineage,
            epoch: 0,
            committed: [0; 32],
            runtime: root.runtime(),
            persona: root.persona(),
        })
    }
    pub fn successor(proof: &VerifiedCommit, prefix: &super::certificate::Prefix) -> Result<Self> {
        if !prefix.is_verified()
            || prefix.is_conflicted()
            || prefix.latest_hash() != Some(proof.event_hash())
            || prefix.len() != proof.core().epoch as usize
        {
            return Err(Error::Invalid("successor authority prefix"));
        }
        Ok(Self {
            lineage: proof.core().lineage,
            epoch: proof.core().epoch,
            committed: proof.event_hash(),
            runtime: proof.core().successor_runtime,
            persona: proof.core().successor_persona,
        })
    }
    fn validates_source(&self, core: &Core) -> bool {
        self.epoch.checked_add(1) == Some(core.epoch)
            && self.lineage == core.lineage
            && self.committed == core.predecessor
            && self.runtime == core.source_runtime
            && self.persona == core.source_persona
    }
}
pub struct Source {
    core: Core,
    stage: SourceStage,
    prepared: Option<Hash>,
    retirement: Option<Retirement>,
}
/// Not constructible by remote data or a Prepared certificate. Its exact event
/// is the locally persisted retirement's event, never a newly encrypted retry.
pub struct PublicationPermit {
    bytes: Vec<u8>,
    hash: Hash,
}
impl PublicationPermit {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn event_hash(&self) -> Hash {
        self.hash
    }
}
impl Source {
    pub fn frozen(core: Core, authority: &Authority) -> Result<Self> {
        if !authority.validates_source(&core) {
            return Err(Error::Invalid("source authority boundary"));
        }
        Ok(Self {
            core,
            stage: SourceStage::Frozen,
            prepared: None,
            retirement: None,
        })
    }
    pub fn stage(&self) -> SourceStage {
        self.stage
    }
    pub fn gameplay_admitted(&self) -> bool {
        self.stage == SourceStage::Aborted
    }
    pub fn prepared(&mut self, proof: &VerifiedPrepared, stored: &StoredProof) -> Result<()> {
        if self.stage != SourceStage::Frozen
            || proof.core() != &self.core
            || stored.event_hash() != proof.event_hash()
        {
            return Err(Error::State("source Prepared boundary"));
        }
        self.prepared = Some(proof.event_hash());
        self.stage = SourceStage::Prepared;
        Ok(())
    }
    pub fn abort(&mut self) -> Result<()> {
        if !matches!(self.stage, SourceStage::Frozen | SourceStage::Prepared) {
            return Err(Error::State("source cannot resume"));
        }
        self.stage = SourceStage::Aborted;
        Ok(())
    }
    pub fn retire(&mut self, store: &MarkerStore, record: Retirement) -> Result<PublicationPermit> {
        if !matches!(
            self.stage,
            SourceStage::Prepared | SourceStage::RetirementUncertain
        ) || record.core != self.core
            || Some(record.prepared) != self.prepared
        {
            return Err(Error::State("source retirement boundary"));
        }
        if self.retirement.as_ref().is_some_and(|old| old != &record) {
            return Err(Error::State("retirement retry changed"));
        }
        let bytes = record.encode()?;
        // Pessimistically quarantine on ANY persistence error. This also covers
        // unreadable existing retirement, unlike treating every IO as absence.
        self.stage = SourceStage::RetirementUncertain;
        self.retirement = Some(record.clone());
        store.persist(MarkerKind::Retired, self.core.transfer, &bytes)?;
        self.stage = SourceStage::Retired;
        Ok(PublicationPermit {
            bytes: record.exact_event,
            hash: record.committed,
        })
    }
    pub fn published(&mut self, stored: &StoredProof) -> Result<()> {
        if !matches!(self.stage, SourceStage::Retired | SourceStage::Published)
            || self
                .retirement
                .as_ref()
                .is_none_or(|r| r.committed != stored.event_hash())
        {
            return Err(Error::State("source publication boundary"));
        }
        self.stage = SourceStage::Published;
        Ok(())
    }
    /// Called before normal open/autosave/hosting, not after a world begins.
    pub fn restart(store: &MarkerStore, authority: &Authority) -> Result<Option<Self>> {
        let mut found = None;
        for (id, bytes) in store.retirements()? {
            let record = Retirement::decode(&bytes)?;
            if record.core.transfer != id {
                return Err(Error::Invalid("retirement filename binding"));
            }
            if record.core.lineage != authority.lineage
                || authority.epoch.checked_add(1) != Some(record.core.epoch)
            {
                continue;
            }
            if !authority.validates_source(&record.core) {
                return Err(Error::Invalid("retired source authority binding"));
            }
            if found.is_some() {
                return Err(Error::Invalid("conflicting source retirements"));
            }
            store.persist(MarkerKind::Retired, id, &bytes)?;
            found = Some(Self {
                core: record.core.clone(),
                stage: SourceStage::Retired,
                prepared: Some(record.prepared),
                retirement: Some(record),
            });
        }
        // A legitimate hand-back is a new verified authority epoch. Never let
        // caller-selected epoch bypass this scan: world-open integration must
        // derive it from its durable verified Activated/committed binding.
        Ok(found)
    }
    pub fn retry_publication(&self) -> Result<PublicationPermit> {
        if !matches!(self.stage, SourceStage::Retired | SourceStage::Published) {
            return Err(Error::State("source publication forbidden"));
        }
        let r = self
            .retirement
            .as_ref()
            .ok_or(Error::State("missing retirement"))?;
        Ok(PublicationPermit {
            bytes: r.exact_event.clone(),
            hash: r.committed,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverStage {
    Ready,
    Prepared,
    Activated,
    Aborted,
}
pub struct Receiver {
    core: Core,
    prepared: Hash,
    stage: ReceiverStage,
    committed: Option<Hash>,
}
impl Receiver {
    /// Next integration adapter proves full imported checkpoint durability and
    /// latched listener readiness first. This persists Ready control evidence
    /// BEFORE it can be sent; the source has not published Prepared yet.
    pub fn checkpoint_ready(
        store: &MarkerStore,
        core: &Core,
        ready: &Event,
        challenge: Id,
        root: &super::context::RootContext,
        now: u64,
    ) -> Result<Self> {
        core.validate(root, now)?;
        super::crypto::verify_ready(ready, core, Some(challenge))?;
        let mut v = Vec::new();
        frame(&mut v, b"AxeNStax/handover/receiver-ready/v1");
        frame(&mut v, &core.encode());
        frame(&mut v, ready.as_json().as_bytes());
        v.extend_from_slice(&challenge);
        store.persist(MarkerKind::Ready, core.transfer, &v)?;
        Ok(Self {
            core: core.clone(),
            prepared: [0; 32],
            stage: ReceiverStage::Ready,
            committed: None,
        })
    }
    pub fn prepared(&mut self, store: &MarkerStore, proof: &VerifiedPrepared) -> Result<()> {
        if !matches!(self.stage, ReceiverStage::Ready | ReceiverStage::Prepared)
            || proof.core() != &self.core
            || (self.prepared != [0; 32] && self.prepared != proof.event_hash())
        {
            return Err(Error::State("receiver Prepared boundary"));
        }
        let mut v = Vec::new();
        frame(&mut v, b"AxeNStax/handover/receiver-prepared/v1");
        frame(&mut v, &self.core.encode());
        v.extend_from_slice(&proof.event_hash());
        store.persist(MarkerKind::Prepared, self.core.transfer, &v)?;
        self.prepared = proof.event_hash();
        self.stage = ReceiverStage::Prepared;
        Ok(())
    }
    pub fn gameplay_admitted(&self) -> bool {
        self.stage == ReceiverStage::Activated
    }
    pub fn abort(&mut self) -> Result<bool> {
        if self.stage == ReceiverStage::Activated {
            return Ok(false);
        }
        self.stage = ReceiverStage::Aborted;
        Ok(true)
    }
    pub fn activate(
        &mut self,
        store: &MarkerStore,
        proof: &VerifiedCommit,
        stored: &StoredProof,
    ) -> Result<bool> {
        if !matches!(
            self.stage,
            ReceiverStage::Prepared | ReceiverStage::Activated
        ) || proof.core() != &self.core
            || proof.prepared_hash() != self.prepared
            || proof.event_hash() != stored.event_hash()
        {
            return Err(Error::State("receiver activation boundary"));
        }
        if self.stage == ReceiverStage::Activated {
            return if self.committed == Some(proof.event_hash()) {
                Ok(false)
            } else {
                Err(Error::State("activation conflict"))
            };
        }
        let mut v = Vec::new();
        frame(&mut v, b"AxeNStax/handover/activated/v1");
        frame(&mut v, &self.core.encode());
        v.extend_from_slice(&self.prepared);
        v.extend_from_slice(&proof.event_hash());
        store.persist(MarkerKind::Activated, self.core.transfer, &v)?;
        self.committed = Some(proof.event_hash());
        self.stage = ReceiverStage::Activated;
        Ok(true)
    }
    pub fn restart(store: &MarkerStore, transfer: Id) -> Result<Option<Self>> {
        let Some(bytes) = store.read(MarkerKind::Ready, transfer)? else {
            if store.read(MarkerKind::Prepared, transfer)?.is_some()
                || store.read(MarkerKind::Activated, transfer)?.is_some()
            {
                return Err(Error::Invalid("receiver missing Ready"));
            }
            return Ok(None);
        };
        let mut r = Reader::new(&bytes, 8192)?;
        r.label("AxeNStax/handover/receiver-ready/v1")?;
        let core = Core::decode(r.framed(super::certificate::CORE_LIMIT)?)?;
        let ready = Event::from_json(r.framed(super::PROOF_LIMIT)?)
            .map_err(|_| Error::Invalid("stored Ready proof"))?;
        let challenge = r.array()?;
        r.end()?;
        super::crypto::verify_ready(&ready, &core, Some(challenge))?;
        if core.transfer != transfer {
            return Err(Error::Invalid("receiver filename binding"));
        }
        let mut receiver = Self {
            core,
            prepared: [0; 32],
            stage: ReceiverStage::Ready,
            committed: None,
        };
        if let Some(bytes) = store.read(MarkerKind::Prepared, transfer)? {
            let mut r = Reader::new(&bytes, 8192)?;
            r.label("AxeNStax/handover/receiver-prepared/v1")?;
            let c = Core::decode(r.framed(super::certificate::CORE_LIMIT)?)?;
            let p = r.array()?;
            r.end()?;
            if c != receiver.core || p == [0; 32] {
                return Err(Error::Invalid("receiver Prepared binding"));
            }
            receiver.prepared = p;
            receiver.stage = ReceiverStage::Prepared;
        }
        if let Some(bytes) = store.read(MarkerKind::Activated, transfer)? {
            let mut r = Reader::new(&bytes, 8192)?;
            r.label("AxeNStax/handover/activated/v1")?;
            let c = Core::decode(r.framed(super::certificate::CORE_LIMIT)?)?;
            let p = r.array::<32>()?;
            let h = r.array()?;
            r.end()?;
            if c != receiver.core || p != receiver.prepared || p == [0; 32] || h == [0; 32] {
                return Err(Error::Invalid("activation binding"));
            }
            store.persist(MarkerKind::Activated, transfer, &bytes)?;
            receiver.stage = ReceiverStage::Activated;
            receiver.committed = Some(h);
        }
        Ok(Some(receiver))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handover_phase_durable_cut_restart_activation_and_late_abort() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use crate::handover::crypto::{self, Seal, Verified};
            use nostr::{Keys, Timestamp, ToBech32};
            let a = Keys::generate();
            let b = Keys::generate();
            let root = crate::handover::context::RootContext::new(
                a.public_key().to_bytes(),
                &a.public_key().to_bech32().unwrap(),
                900,
                &["wss://relay.example".into()],
                100,
            )
            .unwrap();
            let (_, mut c) = crate::handover::certificate::tests::fixture();
            c.context = root.digest();
            c.root_runtime = root.runtime();
            c.root_persona = root.persona();
            c.source_runtime = root.runtime();
            c.source_persona = root.persona();
            c.successor_runtime = b.public_key().to_bytes();
            c.successor_persona = b.public_key().to_bytes();
            let aa = crate::runtime_identity::mint_player_attestation(
                &a,
                &a.public_key(),
                Timestamp::from(100),
                1,
            )
            .await
            .unwrap();
            let ab = crate::runtime_identity::mint_player_attestation(
                &b,
                &b.public_key(),
                Timestamp::from(100),
                1,
            )
            .await
            .unwrap();
            let keys = crypto::recipient(&root, &[1; 16], None).unwrap();
            let ready = crypto::sign_ready(&b, &c, [8; 16]).await.unwrap();
            let p_event = crypto::seal(
                Seal {
                    source: &a,
                    recipient: &keys,
                    core: &c,
                    ready: &ready,
                    challenge: [8; 16],
                    source_attestation: &aa,
                    successor_attestation: &ab,
                    prepared: None,
                },
                &root,
                100,
            )
            .await
            .unwrap();
            let Verified::Prepared(p) =
                crypto::open(&p_event, &keys, root.runtime(), &root, 100, None).unwrap()
            else {
                panic!("Prepared")
            };
            let event = crypto::seal(
                Seal {
                    source: &a,
                    recipient: &keys,
                    core: &c,
                    ready: &ready,
                    challenge: [8; 16],
                    source_attestation: &aa,
                    successor_attestation: &ab,
                    prepared: Some(&p),
                },
                &root,
                100,
            )
            .await
            .unwrap();
            let Verified::Committed(proof) =
                crypto::open(&event, &keys, root.runtime(), &root, 100, Some(&p)).unwrap()
            else {
                panic!("Commit")
            };
            let (store, dir) = crate::handover::durable::tests::store("source");
            let (receiver_store, receiver_dir) = crate::handover::durable::tests::store("receiver");
            let mut source =
                Source::frozen(c.clone(), &Authority::root(&root, c.lineage).unwrap()).unwrap();
            assert!(!source.gameplay_admitted());
            assert!(source.retry_publication().is_err());
            assert!(
                crate::handover::stored::publication_allowed(
                    &event,
                    &Verified::Committed(proof.clone()),
                    None
                )
                .is_err()
            );
            source
                .prepared(
                    &p,
                    &StoredProof {
                        hash: p.event_hash(),
                    },
                )
                .unwrap();
            let mut receiver =
                Receiver::checkpoint_ready(&receiver_store, &c, &ready, [8; 16], &root, 100)
                    .unwrap();
            receiver.prepared(&receiver_store, &p).unwrap();
            assert!(!receiver.gameplay_admitted());
            assert!(
                !Receiver::restart(&receiver_store, c.transfer)
                    .unwrap()
                    .unwrap()
                    .gameplay_admitted()
            );
            let retirement = Retirement::new(
                &proof,
                &p,
                event.as_json().into_bytes(),
                [9; 16],
                crypto::root_capability(&root, &[1; 16]).unwrap(),
                vec![RetiredRoot {
                    capability: crypto::root_capability(&root, &[1; 16]).unwrap(),
                    expiry: root.expiry(),
                }],
            )
            .unwrap();
            assert!(
                retirement.refuses_root(crypto::root_capability(&root, &[1; 16]).unwrap(), 100)
            );
            assert!(
                retirement.refuses_root(crypto::root_capability(&root, &[1; 16]).unwrap(), 901)
            );
            assert!(!retirement.retained_admission(
                [9; 16],
                Some(crypto::root_capability(&root, &[1; 16]).unwrap()),
                true,
                false
            ));
            assert!(!retirement.retained_admission(c.snapshot_world, None, true, false));
            assert!(retirement.retained_admission([9; 16], None, true, false));
            assert!(!retirement.refuses_root([99; 32], 100));
            assert_eq!(
                Retirement::decode(&retirement.encode().unwrap()).unwrap(),
                retirement
            );
            let permit = source.retire(&store, retirement).unwrap();
            assert_eq!(permit.bytes(), event.as_json().as_bytes());
            assert!(source.abort().is_err());
            assert!(
                crate::handover::stored::publication_allowed(
                    &event,
                    &Verified::Committed(proof.clone()),
                    Some(&permit)
                )
                .is_ok()
            );
            let restarted = Source::restart(&store, &Authority::root(&root, c.lineage).unwrap())
                .unwrap()
                .unwrap();
            assert!(!restarted.gameplay_admitted());
            assert_eq!(
                restarted.retry_publication().unwrap().bytes(),
                permit.bytes()
            );
            assert!(
                receiver
                    .activate(&receiver_store, &proof, &StoredProof { hash: [0; 32] })
                    .is_err()
            );
            assert!(!receiver.gameplay_admitted());
            let stored = StoredProof {
                hash: proof.event_hash(),
            };
            source.published(&stored).unwrap();
            assert!(receiver.activate(&receiver_store, &proof, &stored).unwrap());
            assert!(!receiver.activate(&receiver_store, &proof, &stored).unwrap());
            assert!(!receiver.abort().unwrap());
            assert!(
                Receiver::restart(&receiver_store, c.transfer)
                    .unwrap()
                    .unwrap()
                    .gameplay_admitted()
            );
            std::fs::remove_dir_all(dir).unwrap();
            std::fs::remove_dir_all(receiver_dir).unwrap();
        });
    }
    #[test]
    fn handover_retirement_matches_source_epoch_and_authenticated_handback() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use crate::handover::crypto::{self, Seal, Verified};
            use nostr::{Keys, Timestamp, ToBech32};
            async fn pair(
                a: &Keys,
                b: &Keys,
                aa: &Event,
                ab: &Event,
                core: &Core,
                root: &crate::handover::context::RootContext,
            ) -> (VerifiedPrepared, VerifiedCommit, Event) {
                let keys = crypto::recipient(
                    root,
                    &[1; 16],
                    if core.epoch == 1 {
                        None
                    } else {
                        Some(core.predecessor)
                    },
                )
                .unwrap();
                let ready = crypto::sign_ready(b, core, [8; 16]).await.unwrap();
                let event = crypto::seal(
                    Seal {
                        source: a,
                        recipient: &keys,
                        core,
                        ready: &ready,
                        challenge: [8; 16],
                        source_attestation: aa,
                        successor_attestation: ab,
                        prepared: None,
                    },
                    root,
                    100,
                )
                .await
                .unwrap();
                let Verified::Prepared(p) =
                    crypto::open(&event, &keys, a.public_key().to_bytes(), root, 100, None)
                        .unwrap()
                else {
                    panic!("Prepared")
                };
                let event = crypto::seal(
                    Seal {
                        source: a,
                        recipient: &keys,
                        core,
                        ready: &ready,
                        challenge: [8; 16],
                        source_attestation: aa,
                        successor_attestation: ab,
                        prepared: Some(&p),
                    },
                    root,
                    100,
                )
                .await
                .unwrap();
                let Verified::Committed(c) = crypto::open(
                    &event,
                    &keys,
                    a.public_key().to_bytes(),
                    root,
                    100,
                    Some(&p),
                )
                .unwrap() else {
                    panic!("Commit")
                };
                (p, c, event)
            }
            let a = Keys::generate();
            let b = Keys::generate();
            let root = crate::handover::context::RootContext::new(
                a.public_key().to_bytes(),
                &a.public_key().to_bech32().unwrap(),
                900,
                &["wss://relay.example".into()],
                100,
            )
            .unwrap();
            let (_, mut c) = crate::handover::certificate::tests::fixture();
            c.context = root.digest();
            c.root_runtime = root.runtime();
            c.root_persona = root.persona();
            c.source_runtime = root.runtime();
            c.source_persona = root.persona();
            c.successor_runtime = b.public_key().to_bytes();
            c.successor_persona = b.public_key().to_bytes();
            let aa = crate::runtime_identity::mint_player_attestation(
                &a,
                &a.public_key(),
                Timestamp::from(100),
                1,
            )
            .await
            .unwrap();
            let ab = crate::runtime_identity::mint_player_attestation(
                &b,
                &b.public_key(),
                Timestamp::from(100),
                1,
            )
            .await
            .unwrap();
            let (store, dir) = crate::handover::durable::tests::store("handback");
            let capability = crypto::root_capability(&root, &[1; 16]).unwrap();
            let old = Authority::root(&root, c.lineage).unwrap();
            let (p1, proof1, event1) = pair(&a, &b, &aa, &ab, &c, &root).await;
            let mut source = Source::frozen(c.clone(), &old).unwrap();
            source
                .prepared(
                    &p1,
                    &StoredProof {
                        hash: p1.event_hash(),
                    },
                )
                .unwrap();
            source
                .retire(
                    &store,
                    Retirement::new(
                        &proof1,
                        &p1,
                        event1.as_json().into_bytes(),
                        [19; 16],
                        capability,
                        vec![RetiredRoot {
                            capability,
                            expiry: 900,
                        }],
                    )
                    .unwrap(),
                )
                .unwrap();
            assert!(Source::restart(&store, &old).unwrap().is_some());
            let mut prefix = crate::handover::certificate::Prefix::default();
            prefix.accept(&proof1, &root, 100).unwrap();
            let mut second = c.clone();
            second.epoch = 2;
            second.remaining_hops = 2;
            second.generation = 2;
            second.predecessor = proof1.event_hash();
            second.transfer = [5; 16];
            second.snapshot_world = [5; 16];
            second.source_runtime = b.public_key().to_bytes();
            second.source_persona = second.source_runtime;
            second.successor_runtime = a.public_key().to_bytes();
            second.successor_persona = second.successor_runtime;
            let (_, proof2, _) = pair(&b, &a, &ab, &aa, &second, &root).await;
            prefix.accept(&proof2, &root, 100).unwrap();
            let handback = Authority::successor(&proof2, &prefix).unwrap();
            let cached =
                crate::handover::certificate::Prefix::restore(&prefix.checkpoint()).unwrap();
            assert!(Authority::successor(&proof2, &cached).is_err());
            assert!(cached.confirmed_by(&prefix).is_ok());
            assert!(Source::restart(&store, &handback).unwrap().is_none());
            let mut third = c;
            third.epoch = 3;
            third.remaining_hops = 1;
            third.generation = 3;
            third.predecessor = proof2.event_hash();
            third.transfer = [6; 16];
            third.snapshot_world = [6; 16];
            let (p3, proof3, event3) = pair(&a, &b, &aa, &ab, &third, &root).await;
            let mut source = Source::frozen(third, &handback).unwrap();
            source
                .prepared(
                    &p3,
                    &StoredProof {
                        hash: p3.event_hash(),
                    },
                )
                .unwrap();
            source
                .retire(
                    &store,
                    Retirement::new(
                        &proof3,
                        &p3,
                        event3.as_json().into_bytes(),
                        [20; 16],
                        capability,
                        vec![RetiredRoot {
                            capability,
                            expiry: 900,
                        }],
                    )
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(
                Source::restart(&store, &handback).unwrap().unwrap().stage(),
                SourceStage::Retired
            );
            let mut wrong = handback;
            wrong.committed = [99; 32];
            assert!(Source::restart(&store, &wrong).is_err());
            std::fs::remove_dir_all(dir).unwrap();
        });
    }
    #[test]
    fn handover_phase_abort_only_before_retirement_and_corrupt_marker_fails_closed() {
        let (root, c) = crate::handover::certificate::tests::fixture();
        let mut source =
            Source::frozen(c.clone(), &Authority::root(&root, c.lineage).unwrap()).unwrap();
        source.abort().unwrap();
        assert!(source.gameplay_admitted());
        let (store, dir) = crate::handover::durable::tests::store("corrupt");
        store
            .persist(MarkerKind::Retired, c.lineage, b"broken")
            .unwrap();
        assert!(Source::restart(&store, &Authority::root(&root, c.lineage).unwrap()).is_err());
        assert!(store.retirements().is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

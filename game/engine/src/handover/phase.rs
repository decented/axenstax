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
    core: Core,
    prepared: Hash,
    committed: Hash,
    exact_event: Vec<u8>,
    retained_copy: Id,
    active_capability: Hash,
    roots: Vec<RetiredRoot>,
    /// CP9: the root's relay URLs (canonical order), so a restarted source
    /// can publish the exact Commit without the bearer. Bound by `validate`:
    /// with the core's root fields they must hash to `core.context`.
    relays: Vec<String>,
    verified: bool,
}
impl Retirement {
    pub fn core(&self) -> &Core {
        &self.core
    }
    pub fn committed_hash(&self) -> Hash {
        self.committed
    }
    pub fn retained_copy(&self) -> Id {
        self.retained_copy
    }
    pub fn new(
        commit: &VerifiedCommit,
        prepared: &VerifiedPrepared,
        exact_event: Vec<u8>,
        retained_copy: Id,
        active_capability: Hash,
        roots: Vec<RetiredRoot>,
        relays: Vec<String>,
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
            relays,
            verified: true,
        };
        r.validate()?;
        Ok(r)
    }
    /// The root context this retirement's core names, rebuilt from the
    /// stored relays and the core's root fields; its digest must be the
    /// core's context. `now = 0`: the root may have expired by a restart,
    /// and the Commit was sealed (and its route bounded) before that.
    fn root(&self) -> Result<super::context::RootContext> {
        use nostr::ToBech32;
        let npub = nostr::PublicKey::from_slice(&self.core.root_persona)
            .ok()
            .and_then(|k| k.to_bech32().ok())
            .ok_or(Error::Invalid("retirement root persona"))?;
        let root = super::context::RootContext::new(
            self.core.root_runtime,
            &npub,
            self.core.root_expiry,
            &self.relays,
            0,
        )?;
        if root.digest() != self.core.context || root.relays() != self.relays.as_slice() {
            return Err(Error::Invalid("retirement root relays"));
        }
        Ok(root)
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
        self.root()?;
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut v = Vec::new();
        frame(&mut v, b"AxeNStax/handover/retirement/v2");
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
        v.extend_from_slice(&(self.relays.len() as u32).to_be_bytes());
        for relay in &self.relays {
            frame(&mut v, relay.as_bytes());
        }
        Ok(v)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, super::durable::MARKER_LIMIT)?;
        // CP9: v2 adds the root's relays. A v1 record (unreleased) no
        // longer decodes, so it reads as unreadable: still refused.
        r.label("AxeNStax/handover/retirement/v2")?;
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
        let n = r.u32()?;
        if n == 0 || n > 8 {
            return Err(Error::Limit("retirement relays"));
        }
        let mut relays = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let relay = std::str::from_utf8(r.framed(2048)?).map_err(|_| Error::Invalid("retirement relay"))?;
            relays.push(relay.to_owned());
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
            relays,
            verified: false,
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

/// CP9: a retired source's Committed publication, rebuilt from its durable
/// retirement record ALONE. A restarted source holds no bearer, so it can't
/// rebuild a `VerifiedCommit`; this is the permit it publishes with instead.
/// The only constructors read the record through the world's `MarkerStore`
/// (`read`, `scan`): never loose bytes, never a remote frame. It carries
/// nothing secret: the exact signed Commit bytes, the core and the root's
/// public context (relays included), and the recipient key from the
/// Commit's own signed `p` tag.
pub struct RetiredCommit {
    core: Core,
    root: super::context::RootContext,
    event: Event,
    exact_event: Vec<u8>,
    hash: Hash,
    recipient: Hash,
}
impl RetiredCommit {
    /// The retirement record of `transfer` in `store`, if there is one.
    pub fn read(store: &MarkerStore, transfer: Id) -> Result<Option<Self>> {
        let Some(bytes) = store.read(MarkerKind::Retired, transfer)? else {
            return Ok(None);
        };
        Self::from_record(Retirement::decode(&bytes)?, transfer).map(Some)
    }
    /// Every retirement record in `store` (bounded by `retirements`). Any
    /// record that doesn't decode fails the scan (fail closed).
    pub fn scan(store: &MarkerStore) -> Result<Vec<Self>> {
        store
            .retirements()?
            .into_iter()
            .map(|(id, bytes)| Self::from_record(Retirement::decode(&bytes)?, id))
            .collect()
    }
    fn from_record(r: Retirement, transfer: Id) -> Result<Self> {
        if r.core.transfer != transfer {
            return Err(Error::Invalid("retirement filename binding"));
        }
        let root = r.root()?;
        let event = Event::from_json(&r.exact_event).map_err(|_| Error::Invalid("retirement event"))?;
        // What is published is exactly what was persisted.
        if event.as_json().as_bytes() != r.exact_event.as_slice() {
            return Err(Error::Invalid("retirement event bytes"));
        }
        let tags: Vec<_> = event.tags.iter().collect();
        let recipient: Hash = match tags.as_slice() {
            [t] if t.as_slice().len() == 2 && t.as_slice()[0] == "p" => hex::decode(&t.as_slice()[1])
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or(Error::Invalid("retirement event route"))?,
            _ => return Err(Error::Invalid("retirement event route")),
        };
        Ok(Self {
            core: r.core,
            root,
            event,
            exact_event: r.exact_event,
            hash: r.committed,
            recipient,
        })
    }
    pub fn transfer(&self) -> Id {
        self.core.transfer
    }
    pub fn core(&self) -> &Core {
        &self.core
    }
    pub fn root(&self) -> &super::context::RootContext {
        &self.root
    }
    pub fn event(&self) -> &Event {
        &self.event
    }
    /// The exact signed Commit bytes the retirement record holds.
    pub fn exact_bytes(&self) -> &[u8] {
        &self.exact_event
    }
    pub fn event_hash(&self) -> Hash {
        self.hash
    }
    /// The stored route: the source runtime that signed it, the recipient
    /// its `p` tag names, bounded by the root's expiry.
    pub fn route(&self) -> super::stored::Route {
        super::stored::Route {
            author: self.core.source_runtime,
            recipient: self.recipient,
            until: self.root.expiry(),
        }
    }
    /// "Committed published": written once a relay of the root witnessed
    /// this exact event (`stored::publish_retired`'s readback). The payload
    /// is deterministic (an exact retry after a failed write lands the same
    /// bytes): transfer and Commit hash, nothing else.
    pub fn record_published(&self, store: &MarkerStore, proof: &StoredProof) -> Result<()> {
        if proof.event_hash() != self.hash {
            return Err(Error::State("published proof of another event"));
        }
        store.persist(MarkerKind::Published, self.core.transfer, &self.published_payload())
    }
    /// Whether the "Committed published" record of this Commit exists. A
    /// record that names another Commit is an error (fail closed).
    pub fn is_published(&self, store: &MarkerStore) -> Result<bool> {
        match store.read(MarkerKind::Published, self.core.transfer)? {
            None => Ok(false),
            Some(b) if b == self.published_payload() => Ok(true),
            Some(_) => Err(Error::Invalid("published record binding")),
        }
    }
    fn published_payload(&self) -> Vec<u8> {
        let mut v = Vec::new();
        frame(&mut v, b"AxeNStax/handover/committed-published/v1");
        v.extend_from_slice(&self.core.transfer);
        v.extend_from_slice(&self.hash);
        v
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
    /// The authority epoch this source holds (root is 0); its certificate
    /// names epoch + 1.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }
    /// The committed certificate hash this authority came from (zero at root).
    pub fn committed(&self) -> Hash {
        self.committed
    }
    pub fn runtime(&self) -> Hash {
        self.runtime
    }
    /// The persona this authority's runtime acts for (root: the invitation's
    /// own persona; a successor: the certificate's successor persona).
    pub fn persona(&self) -> Hash {
        self.persona
    }
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
    pub(crate) fn validates_source(&self, core: &Core) -> bool {
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
        if !record.verified {
            return Err(Error::Invalid(
                "restored record is not a new retirement proof",
            ));
        }
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
    pub fn stage(&self) -> ReceiverStage {
        self.stage
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
    use crate::handover::crypto::{self, Seal, Verified};
    use nostr::Keys;
    async fn pair(
        a: &Keys,
        b: &Keys,
        aa: &Event,
        ab: &Event,
        core: &Core,
        root: &crate::handover::context::RootContext,
    ) -> (VerifiedPrepared, VerifiedCommit, Event, Event) {
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
            crypto::open(&event, &keys, a.public_key().to_bytes(), root, 100, None).unwrap()
        else {
            panic!("Prepared")
        };
        let prepared_event = event;
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
        (p, c, event, prepared_event)
    }

    /// CP6 ruling 3: tests mint a `StoredProof` only the way production
    /// does, through the loopback relay's publish, OK and fresh readback.
    async fn witness(
        relay: &crate::handover::relay_fixture::Relay,
        root: &crate::handover::context::RootContext,
        event: &Event,
        verified: Verified,
        permit: Option<&PublicationPermit>,
    ) -> StoredProof {
        use crate::handover::stored::{Budget, Publication, Route, publish_readback};
        let tag = event.tags.iter().next().unwrap().as_slice()[1].clone();
        let mut recipient = [0u8; 32];
        hex::decode_to_slice(&tag, &mut recipient).unwrap();
        let route = Route { author: event.pubkey.to_bytes(), recipient, until: root.expiry() };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        publish_readback(&relay.url(), root, Publication { event, verified: &verified, permit, route }, &mut Budget::new(), &cancel)
            .await
            .unwrap()
    }

    /// P1: the whole stored path through the in-process loopback relay: the
    /// real publish, matching OK and fresh-subscription readback mint the
    /// only `StoredProof`s; a withheld readback, a relay outside the
    /// invitation and a Commit before retirement never count as stored; the
    /// receiver witnesses the Committed by its own readback before Activated.
    #[test]
    fn handover_stored_phases_run_through_the_loopback_relay() {
        use crate::handover::{
            relay_fixture::Relay,
            stored::{Budget, Publication, Route, publish_readback, readback},
        };
        use std::sync::atomic::AtomicBool;
        let relay = Relay::start();
        let other = Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use nostr::{Timestamp, ToBech32};
            let a = Keys::generate();
            let b = Keys::generate();
            let root = crate::handover::context::RootContext::new(
                a.public_key().to_bytes(),
                &a.public_key().to_bech32().unwrap(),
                900,
                &[relay.url()],
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
            let aa = crate::runtime_identity::mint_player_attestation(&a, &a.public_key(), Timestamp::from(100), 1)
                .await
                .unwrap();
            let ab = crate::runtime_identity::mint_player_attestation(&b, &b.public_key(), Timestamp::from(100), 1)
                .await
                .unwrap();
            let keys = crypto::recipient(&root, &[1; 16], None).unwrap();
            let ready = crypto::sign_ready(&b, &c, [8; 16]).await.unwrap();
            let seal = |prepared| Seal {
                source: &a,
                recipient: &keys,
                core: &c,
                ready: &ready,
                challenge: [8; 16],
                source_attestation: &aa,
                successor_attestation: &ab,
                prepared,
            };
            let pe = crypto::seal(seal(None), &root, 100).await.unwrap();
            let Verified::Prepared(p) = crypto::open(&pe, &keys, root.runtime(), &root, 100, None).unwrap() else {
                panic!("Prepared")
            };
            let route = Route { author: root.runtime(), recipient: keys.public_key().to_bytes(), until: root.expiry() };
            let cancel = AtomicBool::new(false);
            let vp = Verified::Prepared(p.clone());
            let (root_ref, cancel_ref) = (&root, &cancel);
            let publish = move |url: String, event, verified, permit| async move {
                publish_readback(&url, root_ref, Publication { event, verified, permit, route }, &mut Budget::new(), cancel_ref)
                    .await
            };
            relay.withhold(true);
            assert!(publish(relay.url(), &pe, &vp, None).await.is_err(), "an OK without readback counted as stored");
            relay.withhold(false);
            assert!(publish(other.url(), &pe, &vp, None).await.is_err());
            assert_eq!(other.received(), 0, "a relay outside the invitation was contacted");
            // CP6 review LOW-2: a certificate is never published under
            // another root (checked before any socket), and one operation's
            // Budget, once bound to a root, refuses a second root.
            let root2 = crate::handover::context::RootContext::new(
                b.public_key().to_bytes(),
                &b.public_key().to_bech32().unwrap(),
                900,
                &[relay.url(), other.url()],
                100,
            )
            .unwrap();
            let p2 = Publication { event: &pe, verified: &vp, permit: None, route };
            let e = publish_readback(&other.url(), &root2, p2, &mut Budget::new(), &cancel).await.err();
            assert!(matches!(e, Some(crate::handover::Error::Invalid("publication of another root's certificate"))), "{e:?}");
            assert_eq!(other.received(), 0, "a certificate was published under another root");
            let mut one = Budget::new();
            let p1 = Publication { event: &pe, verified: &vp, permit: None, route };
            publish_readback(&relay.url(), &root, p1, &mut one, &cancel).await.unwrap();
            let p2 = Publication { event: &pe, verified: &vp, permit: None, route };
            let e = publish_readback(&relay.url(), &root2, p2, &mut one, &cancel).await.err();
            assert!(matches!(e, Some(crate::handover::Error::Invalid("relay context"))), "the operation Budget took a second root: {e:?}");
            let queries = relay.queries();
            let stored = publish(relay.url(), &pe, &vp, None).await.unwrap();
            assert_eq!(stored.event_hash(), p.event_hash());
            assert_eq!(relay.queries(), queries + 1, "no independent readback subscription");
            let (store, dir) = crate::handover::durable::tests::store("fixture-source");
            let (receiver_store, receiver_dir) = crate::handover::durable::tests::store("fixture-receiver");
            let mut source = Source::frozen(c.clone(), &Authority::root(&root, c.lineage).unwrap()).unwrap();
            source.prepared(&p, &stored).unwrap();
            let mut receiver = Receiver::checkpoint_ready(&receiver_store, &c, &ready, [8; 16], &root, 100).unwrap();
            receiver.prepared(&receiver_store, &p).unwrap();
            let ce = crypto::seal(seal(Some(&p)), &root, 100).await.unwrap();
            let Verified::Committed(cv) = crypto::open(&ce, &keys, root.runtime(), &root, 100, Some(&p)).unwrap() else {
                panic!("Commit")
            };
            let vc = Verified::Committed(cv.clone());
            let before = relay.received();
            assert!(publish(relay.url(), &ce, &vc, None).await.is_err());
            assert_eq!(relay.received(), before, "a Commit reached the relay before retirement");
            assert!(
                readback(&relay.url(), &root, &ce, &vc, route, &mut Budget::new(), &cancel).await.is_err(),
                "an unpublished Commit read back"
            );
            let capability = crypto::root_capability(&root, &[1; 16]).unwrap();
            let retirement = Retirement::new(
                &cv,
                &p,
                ce.as_json().into_bytes(),
                [9; 16],
                capability,
                vec![RetiredRoot { capability, expiry: root.expiry() }],
                root.relays().to_vec(),
            )
            .unwrap();
            let permit = source.retire(&store, retirement).unwrap();
            let committed = publish(relay.url(), &ce, &vc, Some(&permit)).await.unwrap();
            source.published(&committed).unwrap();
            assert_eq!(source.stage(), SourceStage::Published);
            // The receiver's own fresh readback witnesses the Committed.
            let witnessed = readback(&relay.url(), &root, &ce, &vc, route, &mut Budget::new(), &cancel).await.unwrap();
            assert!(receiver.activate(&receiver_store, &cv, &witnessed).unwrap());
            assert!(receiver.gameplay_admitted());
            assert_eq!(relay.stored().len(), 2, "exactly Prepared and Committed are stored");
            std::fs::remove_dir_all(dir).unwrap();
            std::fs::remove_dir_all(receiver_dir).unwrap();
        });
    }

    /// The recipient's readback fixture: a root naming `relay`, the
    /// verified Prepared/Committed pair and their route.
    async fn readback_chain(
        relay: &crate::handover::relay_fixture::Relay,
    ) -> (crate::handover::context::RootContext, VerifiedPrepared, VerifiedCommit, Event, Event, crate::handover::stored::Route) {
        use nostr::{Timestamp, ToBech32};
        let a = Keys::generate();
        let b = Keys::generate();
        let root = crate::handover::context::RootContext::new(
            a.public_key().to_bytes(),
            &a.public_key().to_bech32().unwrap(),
            900,
            &[relay.url()],
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
        let aa = crate::runtime_identity::mint_player_attestation(&a, &a.public_key(), Timestamp::from(100), 1).await.unwrap();
        let ab = crate::runtime_identity::mint_player_attestation(&b, &b.public_key(), Timestamp::from(100), 1).await.unwrap();
        let (p, cv, ce, pe) = pair(&a, &b, &aa, &ab, &c, &root).await;
        let keys = crypto::recipient(&root, &[1; 16], None).unwrap();
        let route = crate::handover::stored::Route { author: root.runtime(), recipient: keys.public_key().to_bytes(), until: root.expiry() };
        (root, p, cv, ce, pe, route)
    }

    /// CP6 rulings 1 and 4a: `readback` witnesses only a verified Committed,
    /// by exact event id; a Prepared (even stored) and a different stored
    /// event from the same author on the same route are never a witness.
    #[test]
    fn handover_readback_refuses_a_prepared_and_a_different_event_from_the_same_author() {
        use crate::handover::stored::{Budget, readback};
        let relay = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (root, p, cv, ce, pe, route) = readback_chain(&relay).await;
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let (vp, vc) = (Verified::Prepared(p), Verified::Committed(cv));
            // The relay holds only the Prepared: the same author and route.
            relay.insert(pe.clone());
            assert_eq!(pe.pubkey, ce.pubkey);
            assert!(route.matches(&pe) && route.matches(&ce));
            assert!(
                readback(&relay.url(), &root, &pe, &vp, route, &mut Budget::new(), &cancel).await.is_err(),
                "readback witnessed a Prepared"
            );
            assert!(
                readback(&relay.url(), &root, &pe, &vc, route, &mut Budget::new(), &cancel).await.is_err(),
                "readback accepted bytes that aren't the verified Committed"
            );
            assert!(
                readback(&relay.url(), &root, &ce, &vc, route, &mut Budget::new(), &cancel).await.is_err(),
                "readback accepted a different event with the same author"
            );
        });
    }

    /// CP6 rulings 2 and 4b: a witness is a NEW subscription on a relay the
    /// root names, charged to the caller's budget; a relay the root doesn't
    /// name is never contacted; and an event the same budget already saw is
    /// no witness when the relay no longer returns it (no relay EVENT, no
    /// proof; never a cached copy).
    #[test]
    fn handover_readback_is_a_fresh_charged_subscription_and_needs_a_relay_event() {
        use crate::handover::stored::{Budget, lookup, readback};
        let relay = crate::handover::relay_fixture::Relay::start();
        let other = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (root, _, cv, ce, _, route) = readback_chain(&relay).await;
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let vc = Verified::Committed(cv.clone());
            relay.insert(ce.clone());
            other.insert(ce.clone());
            let mut budget = Budget::new();
            let (queries, bytes) = (relay.queries(), budget.bytes_received());
            let witnessed = readback(&relay.url(), &root, &ce, &vc, route, &mut budget, &cancel).await.unwrap();
            assert_eq!(witnessed.event_hash(), cv.event_hash());
            assert_eq!(relay.queries(), queries + 1, "the readback opened no fresh subscription");
            assert!(budget.bytes_received() > bytes, "the readback wasn't charged to the operation's budget");
            assert!(readback(&other.url(), &root, &ce, &vc, route, &mut Budget::new(), &cancel).await.is_err());
            assert_eq!(other.queries(), 0, "a relay the invitation doesn't name was asked");
            // The same budget has now seen the Committed; the relay stops returning it.
            assert_eq!(lookup(&relay.url(), &root, route, &mut budget, &cancel).await.unwrap().len(), 0, "dedup");
            relay.withhold(true);
            let queries = relay.queries();
            assert!(
                readback(&relay.url(), &root, &ce, &vc, route, &mut budget, &cancel).await.is_err(),
                "a readback without a relay EVENT counted as stored"
            );
            assert_eq!(relay.queries(), queries + 1);
        });
    }

    /// CP5 review MEDIUM-1: a Committed verified under ANOTHER root context
    /// (stored on a relay this root also names) is no witness for this root.
    #[test]
    fn handover_readback_refuses_a_verified_commit_of_another_root() {
        use crate::handover::stored::{Budget, readback};
        let relay = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (root, ..) = readback_chain(&relay).await;
            let (other, _, cv, ce, _, route) = readback_chain(&relay).await;
            assert_ne!(other.digest(), root.digest());
            relay.insert(ce.clone());
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let vc = Verified::Committed(cv);
            assert!(readback(&relay.url(), &other, &ce, &vc, route, &mut Budget::new(), &cancel).await.is_ok());
            assert!(
                readback(&relay.url(), &root, &ce, &vc, route, &mut Budget::new(), &cancel).await.is_err(),
                "readback witnessed another root's Committed"
            );
        });
    }

    #[test]
    fn handover_phase_durable_cut_restart_activation_and_late_abort() {
        let relay = crate::handover::relay_fixture::Relay::start();
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
                &[relay.url()],
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
            let prepared_witness = witness(&relay, &root, &p_event, Verified::Prepared(p.clone()), None).await;
            source.prepared(&p, &prepared_witness).unwrap();
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
                root.relays().to_vec(),
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
                Retirement::decode(&retirement.encode().unwrap())
                    .unwrap()
                    .encode()
                    .unwrap(),
                retirement.encode().unwrap()
            );
            let decoded = Retirement::decode(&retirement.encode().unwrap()).unwrap();
            assert!(source.retire(&store, decoded).is_err());
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
                    .activate(&receiver_store, &proof, &prepared_witness)
                    .is_err()
            );
            assert!(!receiver.gameplay_admitted());
            let stored = witness(&relay, &root, &event, Verified::Committed(proof.clone()), Some(&permit)).await;
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
        let relay = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use crate::handover::crypto;
            use nostr::{Keys, Timestamp, ToBech32};
            let a = Keys::generate();
            let b = Keys::generate();
            let root = crate::handover::context::RootContext::new(
                a.public_key().to_bytes(),
                &a.public_key().to_bech32().unwrap(),
                900,
                &[relay.url()],
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
            let (p1, proof1, event1, pe1) = pair(&a, &b, &aa, &ab, &c, &root).await;
            let mut source = Source::frozen(c.clone(), &old).unwrap();
            source
                .prepared(&p1, &witness(&relay, &root, &pe1, crypto::Verified::Prepared(p1.clone()), None).await)
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
                        root.relays().to_vec(),
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
            let (_, proof2, _, _) = pair(&b, &a, &ab, &aa, &second, &root).await;
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
            let (p3, proof3, event3, pe3) = pair(&a, &b, &aa, &ab, &third, &root).await;
            let mut source = Source::frozen(third, &handback).unwrap();
            source
                .prepared(&p3, &witness(&relay, &root, &pe3, crypto::Verified::Prepared(p3.clone()), None).await)
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
                        root.relays().to_vec(),
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
    fn handover_persisted_lineage_mutation_cannot_reopen_source() {
        let relay = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use nostr::{Timestamp, ToBech32};
            let a = Keys::generate();
            let b = Keys::generate();
            let root = crate::handover::context::RootContext::new(
                a.public_key().to_bytes(),
                &a.public_key().to_bech32().unwrap(),
                900,
                &[relay.url()],
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
            let (p, proof, event, pe) = pair(&a, &b, &aa, &ab, &c, &root).await;
            let (store, dir) = crate::handover::durable::tests::store("mutated-lineage");
            let authority = Authority::root(&root, c.lineage).unwrap();
            let mut source = Source::frozen(c.clone(), &authority).unwrap();
            source
                .prepared(&p, &witness(&relay, &root, &pe, crypto::Verified::Prepared(p.clone()), None).await)
                .unwrap();
            let capability = crypto::root_capability(&root, &[1; 16]).unwrap();
            source
                .retire(
                    &store,
                    Retirement::new(
                        &proof,
                        &p,
                        event.as_json().into_bytes(),
                        [19; 16],
                        capability,
                        vec![RetiredRoot {
                            capability,
                            expiry: 900,
                        }],
                        root.relays().to_vec(),
                    )
                    .unwrap(),
                )
                .unwrap();
            let path = dir.join(format!("retired-{}.dat", hex::encode(c.transfer)));
            let mut bytes = std::fs::read(&path).unwrap();
            let offset = bytes
                .windows(c.lineage.len())
                .position(|w| w == c.lineage)
                .unwrap();
            bytes[offset] ^= 1;
            std::fs::write(&path, &bytes).unwrap();
            assert!(
                Source::restart(&store, &authority).is_err(),
                "corrupted retirement must fail closed, never look absent"
            );
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

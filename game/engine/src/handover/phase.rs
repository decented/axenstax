//! Once-only commit model. Durable source retirement creates publication power.
use super::{
    Error, Hash, Id, Result,
    certificate::{Core, VerifiedCommit},
    codec::{Reader, frame},
    crypto::{VerifiedAbort, VerifiedPrepared},
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
    /// Test only (CP10 review LOW-1): the same record re-encoded to name
    /// `relays`, with `core.context` recomputed so it still decodes: the
    /// planted record a ledger must refuse.
    #[cfg(test)]
    pub fn replanted(bytes: &[u8], relays: &[String]) -> Vec<u8> {
        use nostr::ToBech32;
        let mut r = Self::decode(bytes).unwrap();
        let npub = nostr::PublicKey::from_slice(&r.core.root_persona).unwrap().to_bech32().unwrap();
        let other = super::context::RootContext::new(r.core.root_runtime, &npub, r.core.root_expiry, relays, 0).unwrap();
        r.core.context = other.digest();
        r.relays = other.relays().to_vec();
        r.encode().unwrap()
    }
    pub fn committed_hash(&self) -> Hash {
        self.committed
    }
    pub fn retained_copy(&self) -> Id {
        self.retained_copy
    }
    pub(crate) fn personal_root(&self) -> Result<super::context::RootContext> {
        self.root()
    }
    pub(crate) fn retired_roots(&self) -> &[RetiredRoot] { &self.roots }
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

/// CP13b (D2): a source's abort record, written once it aborted after
/// disclosing the bearer. It mirrors retirement record v2: the exact signed
/// Aborted bytes, the core, the Prepared hash it answers and the root's
/// relays, and nothing secret. It authorises one thing: publishing that
/// Aborted (`RetiredCommit::read_aborted`), so a recipient past its ack
/// quarantines. Written only through `write_terminal`, exclusive with the
/// retirement record of the same transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbortRecord {
    core: Core,
    prepared: Hash,
    aborted: Hash,
    exact_event: Vec<u8>,
    relays: Vec<String>,
    verified: bool,
}
const ABORT_V1: &str = "AxeNStax/handover/abort/v1";
impl AbortRecord {
    pub fn new(abort: &VerifiedAbort, prepared: &VerifiedPrepared, exact_event: Vec<u8>, relays: Vec<String>) -> Result<Self> {
        if abort.core() != prepared.core() || abort.prepared_hash() != prepared.event_hash() {
            return Err(Error::Invalid("abort record Prepared"));
        }
        let r = Self {
            core: abort.core().clone(),
            prepared: prepared.event_hash(),
            aborted: abort.event_hash(),
            exact_event,
            relays,
            verified: true,
        };
        r.validate()?;
        Ok(r)
    }
    pub(crate) fn core(&self) -> &Core { &self.core }
    /// The exact signed Aborted bytes this record holds.
    pub fn exact_event(&self) -> &[u8] {
        &self.exact_event
    }
    /// The id of the signed Aborted this record holds.
    pub fn aborted_hash(&self) -> Hash {
        self.aborted
    }
    fn validate(&self) -> Result<()> {
        if self.prepared == [0; 32] || self.relays.is_empty() || self.relays.len() > 8 {
            return Err(Error::Invalid("abort record context"));
        }
        if self.exact_event.len() > super::EVENT_LIMIT {
            return Err(Error::Limit("abort record event"));
        }
        let ev = Event::from_json(&self.exact_event).map_err(|_| Error::Invalid("abort record event"))?;
        if ev.verify().is_err()
            || ev.id.to_bytes() != self.aborted
            || ev.pubkey.to_bytes() != self.core.source_runtime
            || ev.kind != nostr::Kind::Custom(super::STORED_KIND)
        {
            return Err(Error::Invalid("abort record signature"));
        }
        stored_root(&self.core, &self.relays).map(|_| ())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut v = Vec::new();
        frame(&mut v, ABORT_V1.as_bytes());
        frame(&mut v, &self.core.encode());
        v.extend_from_slice(&self.prepared);
        v.extend_from_slice(&self.aborted);
        frame(&mut v, &self.exact_event);
        v.extend_from_slice(&(self.relays.len() as u32).to_be_bytes());
        for relay in &self.relays {
            frame(&mut v, relay.as_bytes());
        }
        Ok(v)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, super::durable::MARKER_LIMIT)?;
        r.label(ABORT_V1)?;
        let core = Core::decode(r.framed(super::certificate::CORE_LIMIT)?)?;
        let prepared = r.array()?;
        let aborted = r.array()?;
        let exact_event = r.framed(super::EVENT_LIMIT)?.to_vec();
        let n = r.u32()?;
        if n == 0 || n > 8 {
            return Err(Error::Limit("abort record relays"));
        }
        let mut relays = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let relay = std::str::from_utf8(r.framed(2048)?).map_err(|_| Error::Invalid("abort record relay"))?;
            relays.push(relay.to_owned());
        }
        r.end()?;
        let value = Self { core, prepared, aborted, exact_event, relays, verified: false };
        value.validate()?;
        Ok(value)
    }
}

/// CP13b (D2): the refusals of the one choke point.
pub const ABORT_RECORD_EXISTS: &str = "abort record exists";
pub const RETIREMENT_RECORD_EXISTS: &str = "retirement record exists";
/// CP13b (CP13 review MEDIUM-1): the ledger entry couldn't be written, so
/// the record certainly wasn't: a certain non-retirement (an abort).
pub const LEDGER_UNWRITTEN: &str = "handover ledger unwritten";
static TERMINAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// CP13b (D2): the ONE function that writes a source's terminal record for
/// `transfer`: the retirement record (`Retired`) or the abort record
/// (`Aborted`). Whichever is durable first wins: under one process-wide lock
/// it refuses when the other kind exists (or can't be read: fail closed),
/// then appends and fsyncs the ledger entry, then the record.
fn write_terminal(store: &MarkerStore, ledger: &super::ledger::Ledger, transfer: Id, kind: MarkerKind, bytes: &[u8]) -> Result<()> {
    let _serialised = TERMINAL.lock().unwrap_or_else(|e| e.into_inner());
    write_terminal_locked(store, ledger, transfer, kind, bytes)
}
fn write_terminal_locked(store: &MarkerStore, ledger: &super::ledger::Ledger, transfer: Id, kind: MarkerKind, bytes: &[u8]) -> Result<()> {
    use super::ledger::{Entry, RecordKind, Role};
    let (other, entry, refusal) = match kind {
        MarkerKind::Retired => (MarkerKind::Aborted, RecordKind::Retirement, ABORT_RECORD_EXISTS),
        MarkerKind::Aborted => (MarkerKind::Retired, RecordKind::Abort, RETIREMENT_RECORD_EXISTS),
        _ => return Err(Error::State("not a terminal record")),
    };
    if store.read(other, transfer)?.is_some() {
        return Err(Error::State(refusal));
    }
    // CP15: the abort record's bytes are the pending abort record's bytes,
    // whose ledger line (kind Abort, same digest) was written before the
    // bearer left. One line per record keeps the ledger simple: the
    // headroom checked at disclosure (`has_room(2)`: this Abort line and the
    // Retirement line) still covers every handover, and a record listed
    // already is never listed twice.
    let entry = Entry::of(transfer, Role::Source, entry, bytes);
    if !(kind == MarkerKind::Aborted && ledger.lists(&entry)) {
        ledger.append(entry).map_err(|_| Error::State(LEDGER_UNWRITTEN))?;
    }
    store.persist(kind, transfer, bytes)?;
    // CP15: a durable terminal record supersedes the pending abort record,
    // which is REMOVED rather than kept beside it: the store caps each kind
    // at 64 records per world, so kept pending records would end every
    // later handover of the world, and a removal needs no ledger line (the
    // ledger lists what may be published; a removed file publishes
    // nothing). Only after an Ok persist (file and directory fsynced): an
    // uncertain write keeps the pending record as the start-up backstop.
    if let Err(e) = store.remove(MarkerKind::PendingAbort, transfer) {
        log::warn!("handover: a pending abort record couldn't be removed ({e}); the next start-up removes it");
    }
    Ok(())
}

/// CP15 (CP13b review LOW-3, coordinator 2026-10-11): the pending abort
/// record, written before the bearer leaves the source (the caller sets
/// `disclosed` only after an Ok). Its ledger line first (kind Abort, the
/// digest of these exact bytes, which are the abort record the promotion
/// or a clean abort writes later), then the record (file and directory
/// fsynced). Any error: nothing may be disclosed.
pub fn write_pending_abort(store: &MarkerStore, ledger: &super::ledger::Ledger, record: &AbortRecord) -> Result<()> {
    use super::ledger::{Entry, RecordKind, Role};
    if !record.verified {
        return Err(Error::Invalid("pending abort record is not a new seal"));
    }
    let bytes = record.encode()?;
    let transfer = record.core.transfer;
    ledger
        .append(Entry::of(transfer, Role::Source, RecordKind::Abort, &bytes))
        .map_err(|_| Error::State(LEDGER_UNWRITTEN))?;
    store.persist(MarkerKind::PendingAbort, transfer, &bytes)
}

/// CP15: what the start-up promotion did with `world`'s pending abort records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Promotion {
    /// Promoted to abort records (published by the scan that follows).
    pub promoted: usize,
    /// Discarded because a retirement record of the transfer exists.
    pub retired_won: usize,
}

/// CP15 (coordinator 2026-10-11): at start-up, before any world opens, a
/// pending abort record with no terminal record is promoted to the abort
/// record through the one choke point (`write_terminal`: retire/abort
/// exclusivity holds; no retire can race a process that died; CP17: a
/// second live process is fenced by `live::world_lock`, which the caller
/// holds while promoting, skipping a world another process holds). The scan
/// that follows publishes it (`Resumer::adopt`, which re-persists it
/// first), and the recipient past its ack quarantines on it instead of
/// waiting forever. The restarted process holds no bearer: invitations are
/// never persisted, so the old one is already rotated.
///
/// Safe only because a Committed is never published without a durable
/// retirement record (CP14/CP14b: the session hands a Committed over only
/// through `adopt`'s `persist_again`, and the scan publishes only records
/// it read and re-persisted). No retirement record here means no Committed
/// left this machine, so an Aborted can't contradict one.
///
/// A pending record and a retirement record: the retirement wins, the
/// pending record is discarded and that is logged. A pending record whose
/// abort record already exists is just removed. Each record is decoded on
/// its own (one bad record is logged and skipped, never gating another),
/// and only a record whose exact digest `ledger` lists is promoted.
pub fn promote_pending_aborts(store: &MarkerStore, ledger: &super::ledger::Ledger) -> Promotion {
    use super::ledger::{Entry, RecordKind, Role};
    let mut out = Promotion::default();
    let pending = match store.pending_aborts() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("handover: pending abort records under {} can't be listed: {e}", store.directory().display());
            return out;
        }
    };
    let _serialised = TERMINAL.lock().unwrap_or_else(|e| e.into_inner());
    for (transfer, bytes) in pending {
        if let Err(e) = AbortRecord::decode(&bytes).and_then(|r| {
            if r.core.transfer == transfer { Ok(()) } else { Err(Error::Invalid("pending abort filename binding")) }
        }) {
            log::warn!("handover: a pending abort record can't be read ({e}); it is ignored");
            continue;
        }
        if !ledger.lists(&Entry::of(transfer, Role::Source, RecordKind::Abort, &bytes)) {
            log::warn!("handover: a pending abort record isn't in this install's ledger; it is ignored");
            continue;
        }
        match store.read(MarkerKind::Aborted, transfer) {
            Ok(Some(_)) => {
                let _ = store.remove(MarkerKind::PendingAbort, transfer);
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("handover: an abort record can't be read ({e}); its pending record stays");
                continue;
            }
        }
        match write_terminal_locked(store, ledger, transfer, MarkerKind::Aborted, &bytes) {
            Ok(()) => out.promoted += 1,
            Err(Error::State(RETIREMENT_RECORD_EXISTS)) => {
                log::warn!("handover: a pending abort record lost to the transfer's retirement record; the retirement stands and the pending record is discarded");
                if let Err(e) = store.remove(MarkerKind::PendingAbort, transfer) {
                    log::warn!("handover: the discarded pending abort record couldn't be removed ({e})");
                }
                out.retired_won += 1;
            }
            Err(e) => log::error!("handover: a pending abort record couldn't be promoted ({e}); the next start-up tries again"),
        }
    }
    out
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
    /// CP13: the SHA-256 of the record's exact payload bytes (its ledger digest).
    record: Hash,
    /// CP13b (D2): read from an abort record: the event is the Aborted.
    aborted: bool,
}
impl RetiredCommit {
    /// CP13b (D2): the abort record of `transfer` in `store`, if any, as the
    /// same publication permit (its event is the exact signed Aborted).
    pub fn read_aborted(store: &MarkerStore, transfer: Id) -> Result<Option<Self>> {
        let Some(bytes) = store.read(MarkerKind::Aborted, transfer)? else {
            return Ok(None);
        };
        Self::from_abort(AbortRecord::decode(&bytes)?, transfer, super::hash(&bytes)).map(Some)
    }
    fn from_abort(r: AbortRecord, transfer: Id, record: Hash) -> Result<Self> {
        let relays = r.relays.clone();
        let mut out = Self::from_parts(r.core, &relays, r.exact_event, r.aborted, transfer, record)?;
        out.aborted = true;
        Ok(out)
    }
    /// Whether this permit publishes an Aborted (an abort record).
    pub fn is_abort(&self) -> bool {
        self.aborted
    }
    /// CP14b (CP10 review LOW-8): make the record this permit was read from
    /// durable before anything publishes it. A record that reads back is
    /// only VISIBLE: a crash (or a failed directory fsync) after its rename
    /// can leave it unsynced. This re-reads the exact payload, requires its
    /// digest to be the one the permit was built from, and persists the
    /// identical bytes, which returns Ok only after the file and the
    /// directory are fsynced again.
    pub fn persist_again(&self, store: &MarkerStore) -> Result<()> {
        let kind = if self.aborted { MarkerKind::Aborted } else { MarkerKind::Retired };
        let Some(bytes) = store.read(kind, self.core.transfer)? else {
            return Err(Error::State("terminal record gone"));
        };
        if super::hash(&bytes) != self.record {
            return Err(Error::State("terminal record changed"));
        }
        store.persist(kind, self.core.transfer, &bytes)
    }
    /// The retirement record of `transfer` in `store`, if there is one.
    pub fn read(store: &MarkerStore, transfer: Id) -> Result<Option<Self>> {
        let Some(bytes) = store.read(MarkerKind::Retired, transfer)? else {
            return Ok(None);
        };
        Self::from_record(Retirement::decode(&bytes)?, transfer, super::hash(&bytes)).map(Some)
    }
    /// Every retirement record in `store` (bounded by `retirements`), then
    /// (CP13b) every abort record (bounded by `aborts`). A retirement record
    /// that doesn't decode fails the scan (fail closed: the world stays
    /// refused). CP15 (CP13b review LOW-6): abort records are decoded one at
    /// a time; one that can't be listed or decoded is logged and skipped
    /// (`unreadable_aborts` gives the lobby its line), so an old damaged
    /// abort record never stops a later retirement's publication.
    pub fn scan(store: &MarkerStore) -> Result<Vec<Self>> {
        let mut out: Vec<Self> = store
            .retirements()?
            .into_iter()
            .map(|(id, bytes)| Self::from_record(Retirement::decode(&bytes)?, id, super::hash(&bytes)))
            .collect::<Result<_>>()?;
        match store.aborts() {
            Ok(aborts) => {
                for (id, bytes) in aborts {
                    match AbortRecord::decode(&bytes).and_then(|r| Self::from_abort(r, id, super::hash(&bytes))) {
                        Ok(r) => out.push(r),
                        Err(e) => log::warn!("handover: an abort record can't be read ({e}); it is skipped"),
                    }
                }
            }
            Err(e) => log::warn!("handover: the abort records can't be listed ({e}); they are skipped"),
        }
        Ok(out)
    }
    /// CP15 (CP13b review LOW-6): how many of `store`'s abort records the
    /// scan skips (they can't be listed or decoded).
    pub fn unreadable_aborts(store: &MarkerStore) -> usize {
        match store.aborts() {
            Ok(aborts) => aborts
                .iter()
                .filter(|(id, bytes)| AbortRecord::decode(bytes).and_then(|r| Self::from_abort(r, *id, super::hash(bytes))).is_err())
                .count(),
            Err(_) => 1,
        }
    }
    fn from_record(r: Retirement, transfer: Id, record: Hash) -> Result<Self> {
        r.root()?;
        let relays = r.relays.clone();
        Self::from_parts(r.core, &relays, r.exact_event, r.committed, transfer, record)
    }
    fn from_parts(core: Core, relays: &[String], exact_event: Vec<u8>, hash: Hash, transfer: Id, record: Hash) -> Result<Self> {
        if core.transfer != transfer {
            return Err(Error::Invalid("retirement filename binding"));
        }
        let root = stored_root(&core, relays)?;
        let event = Event::from_json(&exact_event).map_err(|_| Error::Invalid("retirement event"))?;
        // What is published is exactly what was persisted.
        if event.as_json().as_bytes() != exact_event.as_slice() {
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
        Ok(Self { core, root, event, exact_event, hash, recipient, record, aborted: false })
    }
    pub fn transfer(&self) -> Id {
        self.core.transfer
    }
    /// CP13: the ledger entry that must list this record before any resumer
    /// publishes it.
    pub fn ledger_entry(&self) -> super::ledger::Entry {
        use super::ledger::{Entry, RecordKind, Role};
        let kind = if self.aborted { RecordKind::Abort } else { RecordKind::Retirement };
        Entry { transfer: self.core.transfer, role: Role::Source, kind, digest: self.record }
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
    /// CP9 review LOW-2, one rule: only a record that reads back as this
    /// exact Commit's counts. Unreadable, missing or naming another Commit
    /// reads as not published (logged): re-publishing identical bytes is
    /// safe, and the record write replaces a mismatched one.
    pub fn is_published(&self, store: &MarkerStore) -> bool {
        match store.read(MarkerKind::Published, self.core.transfer) {
            Ok(Some(b)) if *b == self.published_payload() => true,
            Ok(None) => false,
            Ok(Some(_)) => {
                log::warn!("handover: a Committed published record names another Commit; publishing again");
                false
            }
            Err(e) => {
                log::warn!("handover: the Committed published record can't be read ({e}); publishing again");
                false
            }
        }
    }
    fn published_payload(&self) -> Vec<u8> {
        let mut v = Vec::new();
        // CP13b: an Aborted's "published" record has its own label.
        let label: &[u8] = if self.aborted { b"AxeNStax/handover/aborted-published/v1" } else { b"AxeNStax/handover/committed-published/v1" };
        frame(&mut v, label);
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authority {
    lineage: Id,
    epoch: u32,
    committed: Hash,
    runtime: Hash,
    persona: Hash,
    expiry: u64,
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
    pub(crate) fn expiry(&self) -> u64 {
        self.expiry
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
            expiry: root.expiry(),
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
            expiry: proof.core().expires_at,
        })
    }
    pub(crate) fn validates_source(&self, core: &Core) -> bool {
        self.epoch.checked_add(1) == Some(core.epoch)
            && self.lineage == core.lineage
            && self.committed == core.predecessor
            && self.runtime == core.source_runtime
            && self.persona == core.source_persona
            && core.expires_at <= self.expiry
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
    /// CP13: the ledger entry (this install's handover ledger) is written
    /// durably BEFORE the retirement record it authorises; if it fails, the
    /// record isn't written and the step fails as a failed persist does.
    pub fn retire(&mut self, store: &MarkerStore, ledger: &super::ledger::Ledger, record: Retirement) -> Result<PublicationPermit> {
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
        let was = self.stage;
        self.stage = SourceStage::RetirementUncertain;
        self.retirement = Some(record.clone());
        // CP13b (D2): the one choke point; an abort record of this
        // transfer already won (nothing was written here): stays aborted.
        match write_terminal(store, ledger, self.core.transfer, MarkerKind::Retired, &bytes) {
            Err(Error::State(ABORT_RECORD_EXISTS)) => {
                self.stage = SourceStage::Aborted;
                self.retirement = None;
                return Err(Error::State(ABORT_RECORD_EXISTS));
            }
            // Nothing written this time: back to where it was (Prepared
            // aborts; an earlier uncertain write stays uncertain).
            Err(Error::State(LEDGER_UNWRITTEN)) => {
                self.stage = was;
                if was == SourceStage::Prepared {
                    self.retirement = None;
                }
                return Err(Error::State(LEDGER_UNWRITTEN));
            }
            // CP14b (CP13b review LOW-3): `persist` returns `Io` or `Limit`
            // only BEFORE its rename (temp create, write or fsync; a full
            // disk is the likely one), and `write_terminal`'s read of the
            // abort record fails before anything is written: the record
            // certainly isn't on disk. Treated as LEDGER_UNWRITTEN: back to
            // Prepared, and the caller aborts (the live Abort and the
            // abort-record rule). Only `UncertainDurability` (after the
            // rename) and an existing or unreadable record (`State`,
            // `Invalid`: fail closed) freeze as uncertain.
            Err(e @ (Error::Io | Error::Limit(_))) if was == SourceStage::Prepared => {
                self.stage = was;
                self.retirement = None;
                return Err(e);
            }
            r => r?,
        }
        self.stage = SourceStage::Retired;
        Ok(PublicationPermit {
            bytes: record.exact_event,
            hash: record.committed,
        })
    }
    /// CP13b (D2): the abort after the bearer was disclosed. The abort
    /// record (its ledger entry first) is written through the one choke
    /// point BEFORE the caller unfreezes. A retirement record of this
    /// transfer already won: refused, and this source stays retired
    /// (`RetirementUncertain`: frozen, published from its record). Any other
    /// write failure still ends Aborted (the world is the source's) and is
    /// returned so the caller shows a status line.
    pub fn abort_disclosed(&mut self, store: &MarkerStore, ledger: &super::ledger::Ledger, record: AbortRecord) -> Result<()> {
        if !matches!(self.stage, SourceStage::Frozen | SourceStage::Prepared) {
            return Err(Error::State("source cannot resume"));
        }
        if !record.verified || record.core != self.core || self.prepared.is_some_and(|p| p != record.prepared) {
            return Err(Error::State("source abort record boundary"));
        }
        let bytes = record.encode()?;
        match write_terminal(store, ledger, self.core.transfer, MarkerKind::Aborted, &bytes) {
            Err(Error::State(RETIREMENT_RECORD_EXISTS)) => {
                self.stage = SourceStage::RetirementUncertain;
                Err(Error::State(RETIREMENT_RECORD_EXISTS))
            }
            r => {
                self.stage = SourceStage::Aborted;
                r
            }
        }
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

/// CP10 (HIGH-2): what the recipient persists with its Prepared receipt,
/// BEFORE its PreparedAck leaves: the exact Prepared event, the invitation
/// root it is sealed under (its relays: `Core` carries only the digest) and
/// the disclosed bearer. With these a restarted recipient finds, opens and
/// witnesses the Committed without the session (`AwaitingCommit`). H3: the
/// bearer is stored only in the imported world's `handover/` records: this
/// Prepared receipt, then the Activated record.
pub struct PreparedReceipt<'a> {
    pub event: &'a Event,
    pub root: &'a super::context::RootContext,
    pub bearer: &'a super::live::bearer::Bearer,
}

/// What a Prepared receiver holds of its receipt.
#[derive(Clone)]
struct Held {
    bearer: super::live::bearer::Bearer,
    event: Event,
    root: super::context::RootContext,
}

pub struct Receiver {
    core: Core,
    prepared: Hash,
    stage: ReceiverStage,
    committed: Option<Hash>,
    held: Option<Held>,
}

const PREPARED_V2: &str = "AxeNStax/handover/receiver-prepared/v2";
const ACTIVATED_V2: &str = "AxeNStax/handover/activated/v2";
const QUARANTINED_V1: &str = "AxeNStax/handover/receiver-quarantined/v1";

/// CP14 (CP10 review LOW-3): the open guard's read-only full parse of an
/// Activated record's `payload` for `transfer`: the label, the core (bound
/// to the transfer), the Prepared hash, the Commit hash, the 16-byte bearer
/// and nothing after (`r.end()`). When the transfer's Prepared receipt is
/// readable (`receipt`, its marker payload), the record must name that
/// receipt's Prepared. Writes nothing; the bearer copy is zeroed.
pub(crate) fn activated_record_checks(payload: &[u8], transfer: Id, receipt: Option<&[u8]>) -> bool {
    activated_core(payload, transfer, receipt).is_ok()
}

/// The same full, receipt-bound decoder used by the open guard, exposing
/// only certificate context. The bearer is zeroed before returning.
pub(crate) fn activated_core(payload: &[u8], transfer: Id, receipt: Option<&[u8]>) -> Result<Core> {
        let mut r = Reader::new(payload, super::durable::MARKER_LIMIT)?;
        r.label(ACTIVATED_V2)?;
        let core = Core::decode(r.framed(super::certificate::CORE_LIMIT)?)?;
        let prepared: Hash = r.array()?;
        let committed: Hash = r.array()?;
        let mut bearer = r.array::<16>()?;
        zeroize::Zeroize::zeroize(&mut bearer);
        r.end()?;
        if core.transfer != transfer || prepared == [0; 32] || committed == [0; 32] {
            return Err(Error::Invalid("activation record"));
        }
        if let Some(receipt) = receipt {
            let (receipt_prepared, _held) = Receiver::read_prepared(&core, receipt)?;
            if receipt_prepared != prepared {
                return Err(Error::Invalid("activation binding"));
            }
        }
        Ok(core)
}

/// The root a stored receipt names, rebuilt from its relays (`now = 0`: the
/// root may have expired by a restart) and bound to the core's digest.
fn stored_root(core: &Core, relays: &[String]) -> Result<super::context::RootContext> {
    use nostr::ToBech32;
    let npub = nostr::PublicKey::from_slice(&core.root_persona)
        .ok()
        .and_then(|k| k.to_bech32().ok())
        .ok_or(Error::Invalid("receipt root persona"))?;
    let root = super::context::RootContext::new(core.root_runtime, &npub, core.root_expiry, relays, 0)?;
    if root.digest() != core.context || root.relays() != relays {
        return Err(Error::Invalid("receipt root relays"));
    }
    Ok(root)
}

/// The route the source seals to this bearer under `root`.
fn bearer_route(core: &Core, root: &super::context::RootContext, bearer: &super::live::bearer::Bearer) -> Result<(nostr::Keys, super::stored::Route)> {
    let keys = super::crypto::recipient(root, bearer.expose(), (core.epoch > 1).then_some(core.predecessor))?;
    let route = super::stored::Route { author: core.source_runtime, recipient: keys.public_key().to_bytes(), until: root.expiry() };
    Ok((keys, route))
}

/// Checked installed receipt metadata. This is not a routing proof: a live
/// continuation must reconstruct the committed prefix from the original relays
/// and match its final verified certificate to this exact installation.
pub(crate) struct Installed {
    pub core: Core,
    pub committed: Hash,
    pub root: super::context::RootContext,
    pub bearer: super::live::bearer::Bearer,
}
impl Installed {
    pub(crate) fn read(store: &MarkerStore, transfer: Id) -> Result<Option<Self>> {
        if store.exists(MarkerKind::Quarantined, transfer)? {
            return Err(Error::State("quarantined installation"));
        }
        let Some(mut receiver) = Receiver::restart_with(store, transfer, false)? else {
            return Ok(None);
        };
        if receiver.stage != ReceiverStage::Activated {
            return Err(Error::State("installation is not activated"));
        }
        let held = receiver.held.take().ok_or(Error::State("installation has no receipt"))?;
        Ok(Some(Self {
            core: receiver.core,
            committed: receiver.committed.ok_or(Error::State("installation has no commit"))?,
            root: held.root,
            bearer: held.bearer,
        }))
    }
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
            held: None,
        })
    }
    /// CP10 (HIGH-2): the Prepared receipt WITH the bearer, the exact
    /// Prepared event and the root's relays, durably (0600 MarkerStore:
    /// temp, fsync, rename, directory fsync). The caller sends its
    /// PreparedAck only after this returns Ok. The bearer must derive the key
    /// the event is routed to, so the stored bearer always opens it.
    /// CP13: its ledger entry is written durably first (as `retire`).
    pub fn prepared(
        &mut self,
        store: &MarkerStore,
        ledger: &super::ledger::Ledger,
        proof: &VerifiedPrepared,
        receipt: PreparedReceipt<'_>,
    ) -> Result<()> {
        if !matches!(self.stage, ReceiverStage::Ready | ReceiverStage::Prepared)
            || proof.core() != &self.core
            || (self.prepared != [0; 32] && self.prepared != proof.event_hash())
            || receipt.event.id.to_bytes() != proof.event_hash()
            || receipt.root.digest() != self.core.context
        {
            return Err(Error::State("receiver Prepared boundary"));
        }
        let (_, route) = bearer_route(&self.core, receipt.root, receipt.bearer)?;
        if !route.matches(receipt.event) {
            return Err(Error::Invalid("receipt bearer route"));
        }
        let mut v = Vec::new();
        frame(&mut v, PREPARED_V2.as_bytes());
        frame(&mut v, &self.core.encode());
        v.extend_from_slice(&proof.event_hash());
        frame(&mut v, receipt.event.as_json().as_bytes());
        let relays = receipt.root.relays();
        v.extend_from_slice(&(relays.len() as u32).to_be_bytes());
        for relay in relays {
            frame(&mut v, relay.as_bytes());
        }
        v.extend_from_slice(receipt.bearer.expose());
        use super::ledger::{Entry, RecordKind, Role};
        let r = ledger
            .append(Entry::of(self.core.transfer, Role::Recipient, RecordKind::Receipt, &v))
            .and_then(|()| store.persist(MarkerKind::Prepared, self.core.transfer, &v));
        zeroize::Zeroize::zeroize(&mut v);
        r?;
        self.prepared = proof.event_hash();
        self.stage = ReceiverStage::Prepared;
        self.held = Some(Held { bearer: receipt.bearer.clone(), event: receipt.event.clone(), root: receipt.root.clone() });
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
    /// CP10: the source's authenticated pre-retirement Abort reached a
    /// recipient past its ack: no Committed will ever exist, so the durable
    /// "quarantined" record ends the wait (the resumer and the start-up scan
    /// skip it). The import and every other record stay.
    pub fn quarantine(&mut self, store: &MarkerStore) -> Result<()> {
        if self.stage == ReceiverStage::Activated {
            return Err(Error::State("activated receiver"));
        }
        let mut v = Vec::new();
        frame(&mut v, QUARANTINED_V1.as_bytes());
        frame(&mut v, &self.core.encode());
        v.extend_from_slice(&self.prepared);
        store.persist(MarkerKind::Quarantined, self.core.transfer, &v)?;
        self.stage = ReceiverStage::Aborted;
        Ok(())
    }
    /// The Activated record carries the bearer forward (H3).
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
        let held = self.held.as_ref().ok_or(Error::State("receiver bearer"))?;
        let mut v = Vec::new();
        frame(&mut v, ACTIVATED_V2.as_bytes());
        frame(&mut v, &self.core.encode());
        v.extend_from_slice(&self.prepared);
        v.extend_from_slice(&proof.event_hash());
        v.extend_from_slice(held.bearer.expose());
        let r = store.persist(MarkerKind::Activated, self.core.transfer, &v);
        zeroize::Zeroize::zeroize(&mut v);
        r?;
        self.committed = Some(proof.event_hash());
        self.stage = ReceiverStage::Activated;
        // CP20 (CP18 review MEDIUM-1): the lobby lists the world without a restart.
        crate::handover::live::promote::note_activated();
        Ok(true)
    }
    pub fn restart(store: &MarkerStore, transfer: Id) -> Result<Option<Self>> {
        Self::restart_with(store, transfer, true)
    }
    /// `restart`, with `persist` false a read only: an Activated record is
    /// checked but never written again (CP24, CP22 review LOW-3).
    fn restart_with(store: &MarkerStore, transfer: Id, persist: bool) -> Result<Option<Self>> {
        let Some(bytes) = store.read(MarkerKind::Ready, transfer)? else {
            if store.exists(MarkerKind::Prepared, transfer)? || store.exists(MarkerKind::Activated, transfer)?
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
            held: None,
        };
        if let Some(mut bytes) = store.read(MarkerKind::Prepared, transfer)? {
            let held = Self::read_prepared(&receiver.core, &bytes);
            zeroize::Zeroize::zeroize(&mut bytes);
            let (p, held) = held?;
            receiver.prepared = p;
            receiver.held = Some(held);
            receiver.stage = ReceiverStage::Prepared;
        }
        if let Some(mut bytes) = store.read(MarkerKind::Activated, transfer)? {
            let checked = (|| {
                let mut r = Reader::new(&bytes, 8192)?;
                r.label(ACTIVATED_V2)?;
                let c = Core::decode(r.framed(super::certificate::CORE_LIMIT)?)?;
                let p = r.array::<32>()?;
                let h = r.array()?;
                let b = r.array::<16>()?;
                r.end()?;
                let same_bearer = receiver.held.as_ref().is_some_and(|held| held.bearer.expose() == &b);
                if c != receiver.core || p != receiver.prepared || p == [0; 32] || h == [0; 32] || !same_bearer {
                    return Err(Error::Invalid("activation binding"));
                }
                Ok(h)
            })();
            let persisted = (persist && checked.is_ok()).then(|| store.persist(MarkerKind::Activated, transfer, &bytes));
            zeroize::Zeroize::zeroize(&mut bytes);
            let h = checked?;
            persisted.transpose()?;
            receiver.stage = ReceiverStage::Activated;
            receiver.committed = Some(h);
            // CP22 (CP21 review MEDIUM-1): no `note_activated` here. A
            // restart finds an import already Activated (every lobby status
            // read restarts it); boot and lobby entry promote those.
        }
        Ok(Some(receiver))
    }
    /// A v2 Prepared receipt: bound to `core`, its exact event hashing to
    /// the stored Prepared hash, its root rebuilt from the stored relays,
    /// and its bearer deriving the key the event is routed to. A v1 receipt
    /// (no bearer, unreleased) no longer decodes: it reads as unreadable.
    fn read_prepared(core: &Core, bytes: &[u8]) -> Result<(Hash, Held)> {
        let mut r = Reader::new(bytes, super::durable::MARKER_LIMIT)?;
        r.label(PREPARED_V2)?;
        let c = Core::decode(r.framed(super::certificate::CORE_LIMIT)?)?;
        let p: Hash = r.array()?;
        let event = Event::from_json(r.framed(super::EVENT_LIMIT)?).map_err(|_| Error::Invalid("stored Prepared event"))?;
        let n = r.u32()?;
        if n == 0 || n > 8 {
            return Err(Error::Limit("receipt relays"));
        }
        let mut relays = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let relay = std::str::from_utf8(r.framed(2048)?).map_err(|_| Error::Invalid("receipt relay"))?;
            relays.push(relay.to_owned());
        }
        let bearer = super::live::bearer::Bearer::new(r.array()?);
        r.end()?;
        if &c != core || p == [0; 32] || event.id.to_bytes() != p || event.verify().is_err() {
            return Err(Error::Invalid("receiver Prepared binding"));
        }
        let root = stored_root(core, &relays)?;
        let (_, route) = bearer_route(core, &root, &bearer)?;
        if !route.matches(&event) {
            return Err(Error::Invalid("receipt bearer route"));
        }
        Ok((p, Held { bearer, event, root }))
    }
}

/// CP10 (HIGH-2): a recipient past its PreparedAck whose Activated record
/// isn't written yet: the persisted "awaiting Committed" state. Built only
/// by reading the import's own `handover/` through its `MarkerStore`
/// (`read`), never from a live session's memory, so a restart resumes
/// exactly what was durable before the ack. `None` once Activated or
/// quarantined (the source's authenticated pre-retirement Abort).
pub struct AwaitingCommit {
    receiver: Receiver,
    /// CP13: the SHA-256 of the Prepared receipt's exact payload bytes.
    receipt: Hash,
}
impl AwaitingCommit {
    pub fn read(store: &MarkerStore, transfer: Id) -> Result<Option<Self>> {
        if store.exists(MarkerKind::Quarantined, transfer)? {
            return Ok(None);
        }
        // CP24 (CP22 review LOW-3): read only. An Activated import has
        // nothing to wait for (`None` below), so re-persisting its record
        // here (a signature check and two fsyncs per import, on every lobby
        // status read and held retry) bought nothing.
        let Some(receiver) = Receiver::restart_with(store, transfer, false)? else { return Ok(None) };
        if receiver.stage != ReceiverStage::Prepared || receiver.held.is_none() {
            return Ok(None);
        }
        let mut bytes = store.read(MarkerKind::Prepared, transfer)?.ok_or(Error::Io)?;
        let receipt = super::hash(&bytes);
        zeroize::Zeroize::zeroize(&mut bytes);
        Ok(Some(Self { receiver, receipt }))
    }
    pub fn transfer(&self) -> Id {
        self.receiver.core.transfer
    }
    /// CP13: the ledger entry that must list this receipt before any resumer
    /// waits on it.
    pub fn ledger_entry(&self) -> super::ledger::Entry {
        use super::ledger::{Entry, RecordKind, Role};
        Entry { transfer: self.transfer(), role: Role::Recipient, kind: RecordKind::Receipt, digest: self.receipt }
    }
    /// The runtime that signed this recipient's Ready (the successor).
    pub fn recipient_runtime(&self) -> Hash {
        self.receiver.core.successor_runtime
    }
    fn held(&self) -> &Held {
        self.receiver.held.as_ref().expect("an AwaitingCommit always holds its receipt")
    }
    pub fn root(&self) -> &super::context::RootContext {
        &self.held().root
    }
    /// The route the Committed is sealed to (the bearer's key under the root).
    pub fn route(&self) -> Result<super::stored::Route> {
        bearer_route(&self.receiver.core, self.root(), &self.held().bearer).map(|(_, r)| r)
    }
    /// The source persona this import is arriving from, as an npub (never
    /// hex): the world list's "Waiting for <npub> ..." line.
    pub fn source_npub(&self) -> Option<String> {
        use nostr::ToBech32;
        nostr::PublicKey::from_slice(&self.receiver.core.source_persona).ok()?.to_bech32().ok()
    }
    /// This transfer's Committed among `events` (a root relay's answers):
    /// the stored Prepared is re-opened under the bearer's key, then each
    /// candidate must open as the Committed bound to it and to the core.
    ///
    /// CP11 (CP10 review HIGH-1, coordinator): both open at `core.issued_at`,
    /// the time the receipt captured, NEVER `today` (the caller's injectable
    /// clock). The source authorised the transfer before expiry and this
    /// recipient already acked, so a recipient resumed days after the
    /// certificate, the root or the attestations expired still activates.
    /// At `issued_at` every time check passes exactly as at receipt:
    /// `not_before == issued_at` (offer), both events' `created_at` is
    /// `issued_at`, `issued_at <= expires_at <= root expiry`, and each
    /// attestation holds `valid_from <= issued_at <= expires_at <=
    /// valid_until`. The receipt (v2) already holds the core, so there is no
    /// v3. `today` only feeds a log line.
    pub fn committed(&self, events: &[Event], today: u64) -> Option<(Event, VerifiedCommit)> {
        let held = self.held();
        let (keys, _) = bearer_route(&self.receiver.core, &held.root, &held.bearer).ok()?;
        let source = self.receiver.core.source_runtime;
        let now = self.receiver.core.issued_at;
        let Ok(super::crypto::Verified::Prepared(vp)) = super::crypto::open(&held.event, &keys, source, &held.root, now, None) else {
            return None;
        };
        if vp.event_hash() != self.receiver.prepared || vp.core() != &self.receiver.core {
            return None;
        }
        events.iter().take(128).find_map(|ev| match super::crypto::open(ev, &keys, source, &held.root, now, Some(&vp)) {
            Ok(super::crypto::Verified::Committed(c))
                if c.core() == &self.receiver.core && c.prepared_hash() == self.receiver.prepared =>
            {
                if today > self.receiver.core.expires_at {
                    log::info!("handover: the Committed arrived after the certificate expired; it is checked as of the receipt");
                }
                Some((ev.clone(), c))
            }
            _ => None,
        })
    }
    /// CP13b (D2): THE one verification of a source's Aborted, shared by the
    /// live `Frame::Abort` arm and the published Aborted: the same opening as
    /// `committed` (at `core.issued_at`, under the bearer's key, against the
    /// stored Prepared), bound to this transfer and context (the exact core)
    /// and to the stored Prepared hash.
    pub fn aborted(&self, events: &[Event]) -> Option<(Event, VerifiedAbort)> {
        let held = self.held();
        let (keys, _) = bearer_route(&self.receiver.core, &held.root, &held.bearer).ok()?;
        let source = self.receiver.core.source_runtime;
        let now = self.receiver.core.issued_at;
        let Ok(super::crypto::Verified::Prepared(vp)) = super::crypto::open(&held.event, &keys, source, &held.root, now, None) else {
            return None;
        };
        if vp.event_hash() != self.receiver.prepared || vp.core() != &self.receiver.core {
            return None;
        }
        events.iter().take(128).find_map(|ev| match super::crypto::open(ev, &keys, source, &held.root, now, Some(&vp)) {
            Ok(super::crypto::Verified::Aborted(a)) if a.core() == &self.receiver.core && a.prepared_hash() == self.receiver.prepared => {
                Some((ev.clone(), a))
            }
            _ => None,
        })
    }
    /// CP13b (D2): a verified Aborted ends the wait: the durable quarantine
    /// record, then the receipt (and its bearer) is removed.
    pub fn end_aborted(&mut self, store: &MarkerStore, proof: &VerifiedAbort) -> Result<()> {
        if proof.core() != &self.receiver.core || proof.prepared_hash() != self.receiver.prepared {
            return Err(Error::State("Aborted of another receipt"));
        }
        self.receiver.quarantine(store)?;
        store.remove(MarkerKind::Prepared, self.transfer())
    }
    /// The durable Activated record (with the bearer, H3).
    pub fn activate(&mut self, store: &MarkerStore, proof: &VerifiedCommit, stored: &StoredProof) -> Result<bool> {
        self.receiver.activate(store, proof, stored)
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
            let bearer = crate::handover::live::bearer::Bearer::new([1; 16]);
            receiver.prepared(&receiver_store, &crate::handover::ledger::Ledger::install(), &p, PreparedReceipt { event: &pe, root: &root, bearer: &bearer }).unwrap();
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
            let permit = source.retire(&store, &crate::handover::ledger::Ledger::install(), retirement).unwrap();
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

    /// CP11 (coordinator, expiry): a recipient resumed after the root and
    /// both attestations expired still activates. Everything here was minted
    /// at t = 100 (root expiry 900, attestations valid one day), so today's
    /// clock is decades past every expiry: the recipient verifies the
    /// Committed against what its Prepared receipt captured, never today's
    /// clock. Its import shows the world list's awaiting line until then.
    #[test]
    fn handover_recipient_resumed_after_the_root_and_attestations_expire_still_activates() {
        use crate::handover::live::resumer::{Backoff, Resumer, awaiting_statuses};
        use nostr::{Timestamp, ToBech32};
        let relay = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (root, c, _b, ready, pe, ce, p, source_npub) = rt.block_on(async {
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
            let ready = crypto::sign_ready(&b, &c, [8; 16]).await.unwrap();
            let (p, _, ce, pe) = pair(&a, &b, &aa, &ab, &c, &root).await;
            (root, c, b, ready, pe, ce, p, a.public_key().to_bech32().unwrap())
        });
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let base = std::env::temp_dir().join(format!("axe_handover_expired_{}", hex::encode(random)));
        let imports = base.join("imports");
        let working = imports.join(format!("handover-w-{}", hex::encode(c.transfer))).join("working");
        std::fs::create_dir_all(&working).unwrap();
        let store = crate::handover::live::authority::store(&working).unwrap();
        let mut receiver = Receiver::checkpoint_ready(&store, &c, &ready, [8; 16], &root, 100).unwrap();
        let bearer = crate::handover::live::bearer::Bearer::new([1; 16]);
        receiver.prepared(&store, &crate::handover::ledger::Ledger::install(), &p, PreparedReceipt { event: &pe, root: &root, bearer: &bearer }).unwrap();
        relay.insert(ce.clone());
        let today = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert!(today > root.expiry() && today > 100 + 86_400, "the fixture's root and attestations haven't expired");
        let awaiting = AwaitingCommit::read(&store, c.transfer).unwrap().expect("awaiting Committed");
        assert!(
            awaiting.committed(std::slice::from_ref(&ce), today + 86_400).is_some(),
            "a recipient resumed after the root and attestations expired refused its Committed"
        );
        assert_eq!(
            awaiting_statuses(&imports.join("no-worlds"), &imports, &crate::handover::ledger::Ledger::install()),
            vec![format!("Waiting for {source_npub} to finish handing this world over")],
            "the world list doesn't show the awaiting import"
        );
        let resumer = Resumer::new(Backoff { first: std::time::Duration::from_millis(50), ceiling: std::time::Duration::from_millis(200) });
        assert_eq!(resumer.resume_imports(&imports, &crate::handover::ledger::Ledger::install()), 1, "the expired import wasn't resumed");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while store.read(MarkerKind::Activated, c.transfer).unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "the resumed recipient never activated after the root and attestations expired");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(Receiver::restart(&store, c.transfer).unwrap().unwrap().gameplay_admitted());
        assert!(awaiting_statuses(&imports.join("no-worlds"), &imports, &crate::handover::ledger::Ledger::install()).is_empty(), "an activated import still shows as awaiting");
        std::fs::remove_dir_all(base).unwrap();
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
            let bearer = crate::handover::live::bearer::Bearer::new([1; 16]);
            // CP13: the ledger entry comes BEFORE the record it authorises. A
            // ledger that can't be written leaves no record and fails the step.
            let blocker = std::env::temp_dir().join(format!("axenstax-ledger-blocked-{}", std::process::id()));
            std::fs::write(&blocker, b"a file, not a folder").unwrap();
            let blocked = crate::handover::ledger::Ledger::at(blocker.join("ledger"));
            assert!(receiver.prepared(&receiver_store, &blocked, &p, PreparedReceipt { event: &p_event, root: &root, bearer: &bearer }).is_err(), "a failed ledger write still stored the receipt");
            assert_eq!(receiver_store.read(MarkerKind::Prepared, c.transfer).unwrap(), None, "the receipt was written before its ledger entry");
            receiver.prepared(&receiver_store, &crate::handover::ledger::Ledger::install(), &p, PreparedReceipt { event: &p_event, root: &root, bearer: &bearer }).unwrap();
            assert!(
                AwaitingCommit::read(&receiver_store, c.transfer).unwrap().is_some_and(|a| crate::handover::ledger::Ledger::install().lists(&a.ledger_entry())),
                "the ledger doesn't list the receipt's exact bytes"
            );
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
            assert!(source.retire(&store, &crate::handover::ledger::Ledger::install(), decoded).is_err());
            assert!(source.retire(&store, &blocked, retirement.clone()).is_err(), "a failed ledger write still retired");
            // CP13b (CP13 review MEDIUM-1): a certain non-retirement, so the
            // source can abort; never the RetirementUncertain freeze.
            assert_eq!(source.stage(), SourceStage::Prepared, "a ledger error at retire froze the source");
            assert_eq!(store.read(MarkerKind::Retired, c.transfer).unwrap(), None, "the retirement record was written before its ledger entry");
            let permit = source.retire(&store, &crate::handover::ledger::Ledger::install(), retirement).unwrap();
            assert!(
                RetiredCommit::read(&store, c.transfer).unwrap().is_some_and(|r| crate::handover::ledger::Ledger::install().lists(&r.ledger_entry())),
                "the ledger doesn't list the retirement record's exact bytes"
            );
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
                    &crate::handover::ledger::Ledger::install(),
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
                    &crate::handover::ledger::Ledger::install(),
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
                    &crate::handover::ledger::Ledger::install(),
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

    /// CP13b fixture: a root on `relay`, the core, and its verified
    /// Prepared, Committed and Aborted (over the same Prepared), plus an
    /// Aborted over another Prepared of the same core and one of another
    /// transfer, all sealed to the bearer `[1; 16]`.
    struct Ends {
        root: crate::handover::context::RootContext,
        c: Core,
        ready: Event,
        p: VerifiedPrepared,
        pe: Event,
        cv: VerifiedCommit,
        ce: Event,
        av: VerifiedAbort,
        ae: Event,
        other_prepared: Event,
        other_transfer: Event,
    }
    async fn ends(relay: &crate::handover::relay_fixture::Relay) -> Ends {
        use nostr::{Timestamp, ToBech32};
        let a = Keys::generate();
        let b = Keys::generate();
        let root = crate::handover::context::RootContext::new(a.public_key().to_bytes(), &a.public_key().to_bech32().unwrap(), 900, &[relay.url()], 100).unwrap();
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
        let keys = crypto::recipient(&root, &[1; 16], if c.epoch == 1 { None } else { Some(c.predecessor) }).unwrap();
        let aborted = |core: Core, p: VerifiedPrepared| {
            let (a, b, aa, ab, root, keys) = (a.clone(), b.clone(), aa.clone(), ab.clone(), root.clone(), keys.clone());
            async move {
                let ready = crypto::sign_ready(&b, &core, [8; 16]).await.unwrap();
                let seal = Seal { source: &a, recipient: &keys, core: &core, ready: &ready, challenge: [8; 16], source_attestation: &aa, successor_attestation: &ab, prepared: Some(&p) };
                crypto::seal_aborted(seal, &root, 100).await.unwrap()
            }
        };
        let ready = crypto::sign_ready(&b, &c, [8; 16]).await.unwrap();
        let (p, cv, ce, pe) = pair(&a, &b, &aa, &ab, &c, &root).await;
        let ae = aborted(c.clone(), p.clone()).await;
        let Verified::Aborted(av) = crypto::open(&ae, &keys, root.runtime(), &root, c.issued_at, Some(&p)).unwrap() else { panic!("Aborted") };
        let (p2, ..) = pair(&a, &b, &aa, &ab, &c, &root).await;
        let other_prepared = aborted(c.clone(), p2).await;
        let mut c2 = c.clone();
        c2.transfer = [0x42; 16];
        let (p3, ..) = pair(&a, &b, &aa, &ab, &c2, &root).await;
        let other_transfer = aborted(c2, p3).await;
        Ends { root, c, ready, p, pe, cv, ce, av, ae, other_prepared, other_transfer }
    }

    /// CP13b (D2): retire and the abort after disclosure race to the one
    /// choke point: exactly one durable terminal record, and it is the only
    /// one any publication can read. In sequence, each refuses the other.
    #[test]
    fn handover_retire_and_abort_are_exclusive_at_one_choke_point() {
        use std::sync::{Arc, Barrier};
        let relay = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let e = rt.block_on(ends(&relay));
        let stored = rt.block_on(witness(&relay, &e.root, &e.pe, Verified::Prepared(e.p.clone()), None));
        let capability = crypto::root_capability(&e.root, &[1; 16]).unwrap();
        let retirement = || {
            Retirement::new(&e.cv, &e.p, e.ce.as_json().into_bytes(), [9; 16], capability, vec![RetiredRoot { capability, expiry: e.root.expiry() }], e.root.relays().to_vec()).unwrap()
        };
        let abort = || AbortRecord::new(&e.av, &e.p, e.ae.as_json().into_bytes(), e.root.relays().to_vec()).unwrap();
        let source = || {
            let mut s = Source::frozen(e.c.clone(), &Authority::root(&e.root, e.c.lineage).unwrap()).unwrap();
            s.prepared(&e.p, &stored).unwrap();
            s
        };
        for round in 0..6 {
            let (store, _dir) = crate::handover::durable::tests::store(&format!("terminal-race-{round}"));
            let store = Arc::new(store);
            let barrier = Arc::new(Barrier::new(2));
            let (mut retiring, mut aborting) = (source(), source());
            let (s1, b1, r) = (store.clone(), barrier.clone(), retirement());
            let retire = std::thread::spawn(move || {
                b1.wait();
                retiring.retire(&s1, &crate::handover::ledger::Ledger::install(), r).is_ok()
            });
            let (s2, b2, a) = (store.clone(), barrier.clone(), abort());
            let abort_t = std::thread::spawn(move || {
                b2.wait();
                aborting.abort_disclosed(&s2, &crate::handover::ledger::Ledger::install(), a).is_ok()
            });
            let (retired, aborted) = (retire.join().unwrap(), abort_t.join().unwrap());
            let records = RetiredCommit::scan(&store).unwrap();
            assert_eq!(records.len(), 1, "round {round}: retire {retired}, abort {aborted}: {} durable terminal records", records.len());
            assert!(retired != aborted, "round {round}: retire {retired}, abort {aborted}");
            assert_eq!(records[0].is_abort(), aborted);
            assert_eq!(records[0].event_hash(), if aborted { e.av.event_hash() } else { e.cv.event_hash() }, "round {round}: the record holds the loser's event");
            assert!(crate::handover::ledger::Ledger::install().lists(&records[0].ledger_entry()));
        }
        let (store, _dir) = crate::handover::durable::tests::store("terminal-abort-first");
        let mut s = source();
        s.abort_disclosed(&store, &crate::handover::ledger::Ledger::install(), abort()).unwrap();
        assert_eq!(s.stage(), SourceStage::Aborted);
        let mut late = source();
        assert_eq!(late.retire(&store, &crate::handover::ledger::Ledger::install(), retirement()).err(), Some(Error::State(ABORT_RECORD_EXISTS)));
        assert_eq!(late.stage(), SourceStage::Aborted, "a retire refused by an abort record didn't stay aborted");
        assert_eq!(store.read(MarkerKind::Retired, e.c.transfer).unwrap(), None);
        let (store, _dir) = crate::handover::durable::tests::store("terminal-retire-first");
        source().retire(&store, &crate::handover::ledger::Ledger::install(), retirement()).unwrap();
        let mut late = source();
        assert_eq!(late.abort_disclosed(&store, &crate::handover::ledger::Ledger::install(), abort()).err(), Some(Error::State(RETIREMENT_RECORD_EXISTS)));
        assert_eq!(late.stage(), SourceStage::RetirementUncertain, "an abort refused by a retirement record didn't stay retired");
        assert_eq!(store.read(MarkerKind::Aborted, e.c.transfer).unwrap(), None);
    }

    /// CP13b (D2): the recipient's one Aborted verification binds the
    /// transfer, the context and the Prepared hash (an Aborted of another
    /// transfer or over another Prepared, and a Committed, are ignored; an
    /// Aborted never reads as Committed). With both a valid Committed and a
    /// valid Aborted on the relay, Committed wins, with the player's line.
    #[test]
    fn handover_the_aborted_binds_its_prepared_and_committed_wins_over_it() {
        use crate::handover::live::resumer::{Backoff, Resumer};
        let relay = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let e = rt.block_on(ends(&relay));
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let base = std::env::temp_dir().join(format!("axe_handover_both_{}", hex::encode(random)));
        let imports = base.join("imports");
        let working = imports.join(format!("handover-w-{}", hex::encode(e.c.transfer))).join("working");
        std::fs::create_dir_all(&working).unwrap();
        let store = crate::handover::live::authority::store(&working).unwrap();
        let mut receiver = Receiver::checkpoint_ready(&store, &e.c, &e.ready, [8; 16], &e.root, 100).unwrap();
        let bearer = crate::handover::live::bearer::Bearer::new([1; 16]);
        receiver.prepared(&store, &crate::handover::ledger::Ledger::install(), &e.p, PreparedReceipt { event: &e.pe, root: &e.root, bearer: &bearer }).unwrap();
        let awaiting = AwaitingCommit::read(&store, e.c.transfer).unwrap().unwrap();
        assert!(
            awaiting.aborted(&[e.other_prepared.clone(), e.other_transfer.clone(), e.ce.clone()]).is_none(),
            "an Aborted over another Prepared, of another transfer, or a Committed was accepted as this receipt's Aborted"
        );
        assert!(awaiting.aborted(std::slice::from_ref(&e.ae)).is_some(), "the valid Aborted didn't verify");
        assert!(awaiting.committed(std::slice::from_ref(&e.ae), 0).is_none(), "an Aborted read as Committed");
        relay.insert(e.ae.clone());
        relay.insert(e.ce.clone());
        let resumer = Resumer::new(Backoff { first: std::time::Duration::from_millis(50), ceiling: std::time::Duration::from_millis(200) });
        assert_eq!(resumer.resume_imports(&imports, &crate::handover::ledger::Ledger::install()), 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while store.read(MarkerKind::Activated, e.c.transfer).unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "with both on the relay, Committed didn't win");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(store.read(MarkerKind::Quarantined, e.c.transfer).unwrap(), None, "the Aborted quarantined beside the Committed");
        assert!(resumer.notices().contains(&crate::handover::live::copy::WORLD_IN_TWO_PLACES.to_owned()), "no player-visible line: {:?}", resumer.notices());
        std::fs::remove_dir_all(base).unwrap();
    }

    /// CP15 (CP13b review LOW-6): a damaged abort record of another
    /// transfer never blinds the scan: the retirement record is still
    /// listed (and so published), and the lobby count names the skipped one.
    #[test]
    fn handover_a_damaged_abort_record_never_gates_a_retirement() {
        let relay = crate::handover::relay_fixture::Relay::start();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let e = rt.block_on(ends(&relay));
        let capability = crypto::root_capability(&e.root, &[1; 16]).unwrap();
        let retirement = Retirement::new(&e.cv, &e.p, e.ce.as_json().into_bytes(), [9; 16], capability, vec![RetiredRoot { capability, expiry: e.root.expiry() }], e.root.relays().to_vec()).unwrap();
        let (store, _dir) = crate::handover::durable::tests::store("damaged-abort");
        store.persist(MarkerKind::Retired, e.c.transfer, &retirement.encode().unwrap()).unwrap();
        store.persist(MarkerKind::Aborted, [0x55; 16], b"an old abort record, damaged").unwrap();
        let records = RetiredCommit::scan(&store).expect("a damaged abort record failed the whole scan");
        assert_eq!(records.len(), 1);
        assert!(!records[0].is_abort() && records[0].transfer() == e.c.transfer);
        assert_eq!(RetiredCommit::unreadable_aborts(&store), 1);
    }

    // ── CP18: the recipient's activated import moves into the world list ──

    /// An import past its PreparedAck whose working world is a real saved
    /// world named `name` (written under the test's worlds root, then moved
    /// into the import); its Committed waits on `relay`.
    fn prepared_world_import(tag: &str, name: &str, relay: &crate::handover::relay_fixture::Relay) -> (std::path::PathBuf, std::path::PathBuf, crate::handover::Id) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let e = rt.block_on(ends(relay));
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let imports = std::env::temp_dir().join(format!("axe_handover_promote_{tag}_{}", hex::encode(random))).join("imports");
        let import = imports.join(format!("handover-w-{}", hex::encode(e.c.transfer)));
        std::fs::create_dir_all(&import).unwrap();
        crate::save::write_world_folder("arriving", &crate::save::WorldMeta::new(name), &crate::save::minimal_world_save_for_tests(7), &crate::world::World::new()).unwrap();
        let working = import.join("working");
        std::fs::rename(crate::save::world_dir("arriving"), &working).unwrap();
        let store = crate::handover::live::authority::store(&working).unwrap();
        let mut receiver = Receiver::checkpoint_ready(&store, &e.c, &e.ready, [8; 16], &e.root, 100).unwrap();
        let bearer = crate::handover::live::bearer::Bearer::new([1; 16]);
        receiver.prepared(&store, &crate::handover::ledger::Ledger::install(), &e.p, PreparedReceipt { event: &e.pe, root: &e.root, bearer: &bearer }).unwrap();
        relay.insert(e.ce.clone());
        (imports, working, e.c.transfer)
    }

    /// The wait for Committed, run to Activated.
    fn activate_import(imports: &std::path::Path, working: &std::path::Path, t: crate::handover::Id) {
        use crate::handover::live::resumer::{Backoff, Resumer};
        let resumer = Resumer::new(Backoff { first: std::time::Duration::from_millis(50), ceiling: std::time::Duration::from_millis(200) });
        assert_eq!(resumer.resume_imports(imports, &crate::handover::ledger::Ledger::install()), 1, "the import's wait wasn't taken");
        let store = crate::handover::live::authority::store(working).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while store.read(MarkerKind::Activated, t).unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "the import never activated");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Promote until something moves (the resumer's wait lets go of the
    /// import's lock a moment after Activated), or the deadline.
    fn promote_until_moved(imports: &std::path::Path) -> crate::handover::live::promote::Promoted {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let p = crate::handover::live::promote::promote_activated_imports(imports, &crate::save::worlds_root());
            if !p.moved.is_empty() || std::time::Instant::now() > deadline {
                return p;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// CP18: not before Activated, not while the import's handover lock is
    /// held in this process; then once, into the world list, openable, its
    /// Activated record beside it. A second pass (the next start, or a
    /// crash that left a twin of the import) never moves it again, and a
    /// fork of the moved world carries no `handover/`.
    #[test]
    fn handover_promote_moves_an_activated_import_into_the_world_list_once() {
        use crate::handover::live::{promote::promote_activated_imports, world_lock};
        let _root = crate::save::WorldsRootGuard::new("promote-once");
        let relay = crate::handover::relay_fixture::Relay::start();
        let (imports, working, t) = prepared_world_import("once", "Harbour Town", &relay);
        let root = crate::save::worlds_root();
        let before = promote_activated_imports(&imports, &root);
        assert!(before.moved.is_empty() && working.exists(), "an import was moved before it was Activated");
        assert!(crate::save::list_world_entries().is_empty(), "an import listed before it was Activated");
        let held = world_lock::acquire(&working).unwrap();
        activate_import(&imports, &working, t);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let while_held = promote_activated_imports(&imports, &root);
        assert!(while_held.moved.is_empty() && working.exists(), "an import was moved while its handover lock was held here");
        drop(held);
        let moved = promote_until_moved(&imports);
        assert_eq!(moved, crate::handover::live::promote::Promoted { moved: vec!["harbour_town".into()], lines: vec![], held: false });
        assert!(!working.exists(), "the import's working world is still there");
        let listed = crate::save::list_world_entries();
        assert_eq!(listed.len(), 1, "the world list: {:?}", listed.iter().map(|e| &e.folder_name).collect::<Vec<_>>());
        assert_eq!((listed[0].folder_name.as_str(), listed[0].meta.display_name.as_str()), ("harbour_town", "Harbour Town"));
        assert_eq!(crate::save::world_open_refusal("harbour_town"), None, "the moved world is refused");
        let moved_records = crate::handover::live::authority::dir(&root.join("harbour_town"));
        assert!(moved_records.join(format!("activated-{}.dat", hex::encode(t))).exists(), "the Activated record didn't move with the world");
        // The next start: nothing left to move, still one world.
        assert_eq!(promote_activated_imports(&imports, &root), Default::default());
        // A twin of the import beside the moved world (e.g. a copy put back
        // by hand, or a move a crash repeated): never moved in a second time.
        // CP20 (step 3, DECIDED): we can't prove it is a leftover rather
        // than the player's copy, so it is never deleted either, and the
        // lobby shows no line for it; the log says so once (a minute).
        crate::save::copy_dir_recursive(&root.join("harbour_town"), &working, None).unwrap();
        for _ in 0..2 {
            assert_eq!(promote_activated_imports(&imports, &root), Default::default(), "a world was moved in twice");
            assert!(working.join("handover").exists(), "the hand-made twin was deleted");
        }
        assert!(
            !crate::handover::live::resumer::log_due("promote: already moved", t),
            "the twin wasn't logged (or was logged on every pass)"
        );
        assert_eq!(crate::save::list_world_entries().len(), 1, "the handed-over world is listed twice");
        let fork = crate::save::fork_world("harbour_town", "Harbour Town").expect("the moved world forks");
        assert!(!crate::save::world_dir(&fork).join("handover").exists(), "the fork carried handover/");
        let _ = std::fs::remove_dir_all(imports.parent().unwrap());
    }

    /// CP22 (CP21 review MEDIUM-1, DECIDED): an Activated import that stays
    /// unmoved (here a hand-made twin of a world already moved in) costs
    /// nothing per lobby frame: reading the lobby's status lines (which
    /// restarts every awaiting import) notes no activation, so after the
    /// lobby's entry pass no frame runs another promotion.
    #[test]
    fn handover_promote_a_stuck_import_runs_no_pass_per_lobby_frame() {
        use crate::handover::live::promote::{activations_on_this_thread, promote_activated_imports, LobbyPoll};
        let _root = crate::save::WorldsRootGuard::new("promote-stuck");
        let relay = crate::handover::relay_fixture::Relay::start();
        let (imports, working, t) = prepared_world_import("stuck", "Still Pond", &relay);
        activate_import(&imports, &working, t);
        assert_eq!(promote_until_moved(&imports).moved, vec!["still_pond".to_owned()]);
        let root = crate::save::worlds_root();
        crate::save::copy_dir_recursive(&root.join("still_pond"), &working, None).unwrap();
        let ledger = crate::handover::ledger::Ledger::install();
        // Lobby entry: the count first, then the entry pass and the lines.
        let now = std::time::Instant::now();
        let entry = promote_activated_imports(&imports, &root);
        let mut poll = LobbyPoll::new(activations_on_this_thread(), entry.held, now);
        let _ = crate::handover::live::resumer::awaiting_statuses(&root, &imports, &ledger);
        let mut passes = 0;
        for _ in 0..30 {
            let now = std::time::Instant::now();
            if poll.due(activations_on_this_thread(), now) {
                passes += 1;
                let p = promote_activated_imports(&imports, &root);
                poll.ran(&p, now);
                let _ = crate::handover::live::resumer::awaiting_statuses(&root, &imports, &ledger);
            }
        }
        assert_eq!(passes, 0, "a stuck Activated import ran a promotion pass on {passes} of 30 lobby frames");
        assert!(working.join("handover").exists(), "the twin was touched");
        let _ = std::fs::remove_dir_all(imports.parent().unwrap());
    }

    /// CP18: a worlds folder on another drive (the rename fails with
    /// `CrossesDevices`) leaves the import where it is, with the lobby line;
    /// another process holding the import's lock, and a quarantine record,
    /// each keep it there with no line; with neither, it moves.
    /// CP24 (CP22 review LOW-3): the lobby's status read (and the
    /// resumer's) never writes an Activated record again.
    #[test]
    fn handover_awaiting_read_never_rewrites_an_activated_record() {
        let _root = crate::save::WorldsRootGuard::new("awaiting-read-only");
        let relay = crate::handover::relay_fixture::Relay::start();
        let (imports, working, t) = prepared_world_import("read-only", "Quiet Bay", &relay);
        activate_import(&imports, &working, t);
        let record = crate::handover::live::authority::dir(&working).join(format!("activated-{}.dat", hex::encode(t)));
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        std::fs::File::options().write(true).open(&record).unwrap().set_modified(old).unwrap();
        let store = crate::handover::live::authority::store(&working).unwrap();
        assert!(AwaitingCommit::read(&store, t).unwrap().is_none(), "an Activated import reads as awaiting");
        assert_eq!(std::fs::metadata(&record).unwrap().modified().unwrap(), old, "a status read wrote the Activated record again");

        let worlds = crate::save::worlds_root();
        // Exercise the world's abort/retirement scans as well as the import.
        crate::handover::live::authority::store(&worlds.join("marker-only")).unwrap();
        let (lines, syncs) = crate::fsync::tests::count_syncs(|| {
            crate::handover::live::resumer::awaiting_statuses(&worlds, &imports, &crate::handover::ledger::Ledger::install())
        });
        assert!(lines.is_empty(), "an Activated import left a waiting line");
        assert_eq!(syncs, 0, "a lobby status traversal synced existing directories or records");
        assert_eq!(std::fs::metadata(&record).unwrap().modified().unwrap(), old, "the status traversal rewrote Activated");
    }

    #[test]
    fn handover_promote_refuses_another_drive_a_held_lock_and_a_quarantine() {
        use crate::handover::live::{copy, promote::{promote_activated_imports, promote_with}, world_lock};
        let _root = crate::save::WorldsRootGuard::new("promote-refused");
        let relay = crate::handover::relay_fixture::Relay::start();
        let (imports, working, t) = prepared_world_import("refused", "Far Field", &relay);
        activate_import(&imports, &working, t);
        let root = crate::save::worlds_root();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let other_drive = loop {
            let p = promote_with(&imports, &root, |_, _| Err(std::io::Error::from(std::io::ErrorKind::CrossesDevices)));
            if !p.lines.is_empty() || std::time::Instant::now() > deadline {
                break p;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(other_drive, crate::handover::live::promote::Promoted { moved: vec![], lines: vec![copy::import_on_other_drive("Far Field")], held: false });
        assert!(working.exists() && crate::save::list_world_entries().is_empty(), "something moved across drives");
        let child = world_lock::tests::ChildHolder::hold(&working);
        // CP20: held, so the lobby retries shortly; no line.
        assert_eq!(
            promote_activated_imports(&imports, &root),
            crate::handover::live::promote::Promoted { held: true, ..Default::default() },
            "an import another process holds was moved"
        );
        child.kill();
        // CP24 (CP23 review LOW-1): another process holding only the
        // import's own lock: held at once; the lobby's main thread never
        // waits (~400 ms before) for another process's lock.
        let child = world_lock::tests::ChildHolder::hold_import(working.parent().unwrap());
        let started = std::time::Instant::now();
        let pass = promote_activated_imports(&imports, &root);
        let took = started.elapsed();
        assert_eq!(
            pass,
            crate::handover::live::promote::Promoted { held: true, ..Default::default() },
            "an import whose own lock another process holds was moved"
        );
        assert!(took < std::time::Duration::from_millis(300), "a promotion pass waited {took:?} for another process's import lock");
        child.kill();
        let quarantined = crate::handover::live::authority::dir(&working).join(format!("quarantined-{}.dat", hex::encode(t)));
        std::fs::write(&quarantined, b"q").unwrap();
        assert_eq!(promote_activated_imports(&imports, &root), Default::default(), "a quarantined import was moved");
        assert!(crate::save::list_world_entries().is_empty(), "a quarantined import is listed");
        std::fs::remove_file(&quarantined).unwrap();
        // CP20 (CP18 review LOW-2): an Activated import that stays put for
        // another reason says so in the lobby: a move that failed (tried
        // again), and a damaged Activated record (the open guard refuses it).
        let failed = promote_with(&imports, &root, |_, _| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
        assert_eq!(failed.lines, vec![copy::import_not_moved_yet("Far Field")], "a failed move showed no line");
        let record = crate::handover::live::authority::dir(&working).join(format!("activated-{}.dat", hex::encode(t)));
        let intact = std::fs::read(&record).unwrap();
        std::fs::write(&record, b"damaged").unwrap();
        let damaged = promote_activated_imports(&imports, &root);
        assert_eq!(damaged.lines, vec![copy::import_record_damaged("Far Field")], "a damaged record showed no line");
        assert!(damaged.moved.is_empty() && working.exists(), "an import with a damaged record moved");
        std::fs::write(&record, &intact).unwrap();
        assert_eq!(promote_until_moved(&imports).moved, vec!["far_field".to_owned()]);
        let _ = std::fs::remove_dir_all(imports.parent().unwrap());
    }

    /// Promote through the given boundaries until something leaves
    /// `working` (the resumer's wait lets go of the lock a moment after
    /// Activated), or the deadline.
    fn promote_io_until_gone(imports: &std::path::Path, working: &std::path::Path, rename: crate::handover::live::promote::Rename, sync: crate::handover::live::promote::SyncDir) -> crate::handover::live::promote::Promoted {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let p = crate::handover::live::promote::promote_with_io(imports, &crate::save::worlds_root(), rename, sync);
            if !working.exists() || std::time::Instant::now() > deadline {
                return p;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// The import folder's other contents, as a received import has them.
    fn leftovers(import: &std::path::Path) {
        std::fs::create_dir_all(import.join("received")).unwrap();
        std::fs::write(import.join("received").join("world.dat"), b"received").unwrap();
        std::fs::write(import.join("pack.axeworld"), b"archive").unwrap();
    }

    /// CP19: the rest of the import folder goes only once the move is
    /// durable. A failed fsync after the rename keeps it (the world is
    /// already listed); the next pass syncs again and removes it, and never
    /// moves or lists the world twice.
    #[test]
    fn handover_promote_removes_the_import_folder_only_once_the_move_is_durable() {
        let _root = crate::save::WorldsRootGuard::new("promote-durable");
        let relay = crate::handover::relay_fixture::Relay::start();
        let (imports, working, t) = prepared_world_import("durable", "Quay Side", &relay);
        let import = working.parent().unwrap().to_path_buf();
        leftovers(&import);
        activate_import(&imports, &working, t);
        let not_durable = promote_io_until_gone(&imports, &working, |a, b| std::fs::rename(a, b), |_| Err(std::io::Error::other("injected sync failure")));
        assert_eq!(not_durable.moved, vec!["quay_side".to_owned()]);
        assert!(import.join("received").join("world.dat").exists() && import.join("pack.axeworld").exists(), "the import folder was removed before the move was durable");
        assert_eq!(crate::save::list_world_entries().len(), 1);
        let next = crate::handover::live::promote::promote_activated_imports(&imports, &crate::save::worlds_root());
        assert_eq!(next, Default::default(), "the next pass moved something");
        assert!(!import.exists(), "the moved import's leftovers stayed once the move was durable");
        assert_eq!(crate::save::list_world_entries().len(), 1, "the handed-over world is listed twice");
        let _ = std::fs::remove_dir_all(imports.parent().unwrap());
    }

    /// CP19: a crash between the rename and the removal (the rename landed,
    /// nothing after it ran): the next pass's leftover sweep removes the
    /// import folder without moving or listing anything twice. An import
    /// folder with no `working` that no moved world accounts for (one still
    /// receiving) is never removed.
    #[test]
    fn handover_promote_removes_a_crashed_moves_leftovers_and_nothing_else() {
        let _root = crate::save::WorldsRootGuard::new("promote-crash");
        let relay = crate::handover::relay_fixture::Relay::start();
        let (imports, working, t) = prepared_world_import("crash", "Mill Pond", &relay);
        let import = working.parent().unwrap().to_path_buf();
        leftovers(&import);
        let receiving = imports.join(format!("handover-w-{}", hex::encode([0x5a_u8; 16])));
        leftovers(&receiving);
        activate_import(&imports, &working, t);
        // The rename lands, then the process "dies": nothing after it runs.
        let crashed = promote_io_until_gone(&imports, &working, |a, b| std::fs::rename(a, b).and(Err(std::io::Error::other("crash after rename"))), crate::fsync::sync_dir);
        assert!(crashed.moved.is_empty() && !working.exists() && import.join("received").exists(), "the crash state wasn't reached");
        let boot = crate::handover::live::promote::promote_activated_imports(&imports, &crate::save::worlds_root());
        assert_eq!(boot, Default::default(), "the boot pass moved something");
        assert!(!import.exists(), "the crashed move's leftovers stayed");
        assert!(receiving.join("received").join("world.dat").exists(), "an import nobody moved was removed");
        let listed = crate::save::list_world_entries();
        assert_eq!(listed.len(), 1, "the world list: {:?}", listed.iter().map(|e| &e.folder_name).collect::<Vec<_>>());
        assert_eq!(crate::save::world_open_refusal(&listed[0].folder_name), None);
        let _ = std::fs::remove_dir_all(imports.parent().unwrap());
    }

    /// CP19 (CP18 review MEDIUM-3): a handed-over world called "The
    /// Workshop" lands in its own listed folder, never the Workshop's (which
    /// the recipient hasn't made yet, and still hasn't afterwards).
    #[test]
    fn handover_promote_a_world_named_the_workshop_gets_its_own_folder() {
        let _root = crate::save::WorldsRootGuard::new("promote-workshop");
        let relay = crate::handover::relay_fixture::Relay::start();
        let (imports, working, t) = prepared_world_import("workshop", "The Workshop", &relay);
        activate_import(&imports, &working, t);
        assert_eq!(promote_until_moved(&imports).moved, vec!["the_workshop_2".to_owned()]);
        let root = crate::save::worlds_root();
        assert!(!root.join(crate::workshop::WORKSHOP_FOLDER).exists(), "the handed-over world landed on the Workshop folder");
        let listed = crate::save::list_world_entries();
        assert_eq!(listed.iter().map(|e| e.folder_name.as_str()).collect::<Vec<_>>(), vec!["the_workshop_2"]);
        let _ = std::fs::remove_dir_all(imports.parent().unwrap());
    }
}

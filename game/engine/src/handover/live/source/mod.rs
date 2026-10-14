//! Source session: the host's explicit action, the recipient's verified
//! consent, the frozen capture/pack, the bounded direct transfer and the
//! verified Ready. Everything before retirement is recoverable: `abort`
//! releases the freeze and the original world is exactly as it was.
//! After Ready (Slice A, CP7): the bearer is disclosed, the inert Prepared
//! is published and read back on a fenced worker, the recipient's signed
//! acknowledgement of its durable Prepared is verified, and only then is
//! the source retired with the exact signed Commit bytes. From Retired on
//! nothing (no deadline, no Abort, no Cancel) resumes the original world.
use super::bearer::Bearer;
use super::eligibility::{self, Refusal, Seat};
use super::wire::{Frame, PART_LIMIT, acceptance_content, prepared_ack_content, prepared_ack_proof, source_agreement_content};
use crate::handover::{
    Hash, Id,
    certificate::Core,
    crypto::{self, Seal, Verified, VerifiedPrepared},
    phase::{AbortRecord, PublicationPermit, RetiredRoot, Retirement},
    stored::{Budget, Publication, Route, StoredProof},
    checkpoint::{self, PackBinding, PackedCheckpoint, ParticipantMember},
    context::RootContext,
    manifest::{Agreement, MemberKind},
    personal::RecordContext,
    phase::Authority,
    stream::{CHUNK_LIMIT, FLIGHT_LIMIT, Sender},
};
use crate::hosted_server::HostedServer;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Timestamp, ToBech32};
use std::io::Read;
use std::path::PathBuf;

mod offer;
mod publish;
mod retire;

/// The host's explicit choice. Only the host-action caller constructs it.
pub struct HostAction {
    pub seat: Seat,
    pub source_key: [u8; 32],
    pub world_name: String,
}

pub struct SourceConfig {
    pub runtime: Keys,
    pub root: RootContext,
    pub authority: Authority,
    pub lineage: Id,
    pub world: Id,
    pub generation: u64,
    pub scratch: PathBuf,
    /// Certificate lifetime, clipped to the invitation's expiry.
    pub validity: u64,
    /// The world's own folder, whose `joiners/` the session reads itself at
    /// every boundary (`None`: a world never saved holds no records, and
    /// can't be retired: the handover stops before the bearer is disclosed).
    pub world_dir: Option<PathBuf>,
    /// H1: the bearer of the online host's LIVE invitation (the one
    /// `online_admission`'s `Accept(ByInvite)` checks), never a new kind.
    /// Disclosed to the recipient after Ready; Prepared and Commit are
    /// sealed to the key it derives (`crypto::recipient`).
    pub bearer: Bearer,
    /// This runtime's player attestation for the source persona, sealed
    /// into Prepared and Commit.
    pub attestation: Event,
}

/// What the caller re-reads from the live game at EVERY boundary (host
/// action, consent/freeze, frozen capture), never the values captured when
/// the host chose: a sign-in change or a second local player since then
/// refuses before anything irreversible.
#[derive(Clone, Copy, Debug)]
pub struct Live {
    pub signed_in: Option<[u8; 32]>,
    pub local_seats: usize,
}

/// How long the recipient may take to answer the proposal (the world is
/// not frozen yet) and, once frozen, how long the transfer may go without
/// any message from the recipient before the source gives up and resumes.
pub const CONSENT_SECS: u64 = 300;
pub const STALL_SECS: u64 = 60;
/// While the pack runs on its worker the source tells the recipient it is
/// still there (the recipient's own stall deadline is `STALL_SECS`).
pub const HEARTBEAT_SECS: u64 = 5;
/// P4: the whole operation, from the recipient's consent to its Activated,
/// on both sides. Per-phase stall timers plus heartbeats could otherwise
/// hold a player parked indefinitely. Before Retired hitting it is an
/// abort that restores play; after Retired only exact retry or quarantine.
pub const OPERATION_SECS: u64 = 15 * 60;

/// What a pack worker hands back: its generation, the transfer it packed
/// for, and the packed copy (whose scratch is removed when it is dropped).
type PackResult = (u64, Id, crate::handover::Result<PackProduct>);
enum PackProduct {
    Archive(PackedCheckpoint),
    Retained(PackedCheckpoint, Box<super::retained::PreparedCopy>),
}
/// What a publish worker hands back: its generation, the transfer, the one
/// operation `Budget` (moved back for the next publication) and the stored
/// witness (or why not).
type PublishResult = (u64, Id, Budget, crate::handover::Result<StoredProof>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStage {
    Proposed,
    /// Consent verified and the source frozen: the caller captures now.
    Frozen,
    /// The frozen copy is being packed on a worker thread; the frame, the
    /// control pump and Cancel keep running.
    Packing,
    Streaming,
    /// Recipient durably holds the checkpoint; inert until Prepared/Commit.
    ReadyVerified,
    /// The bearer is disclosed; the inert Prepared is being published and
    /// read back on a worker. An abort from here rotates the invitation.
    Publishing,
    /// Prepared is stored and sent; waiting for the recipient's signed
    /// acknowledgement that it persisted it.
    Prepared,
    /// Durably retired with the exact signed Commit bytes. Terminal for
    /// this world: no abort, no deadline, no resume.
    Retired,
    /// The retirement write failed in a way that may have reached the disk:
    /// quarantined (frozen, never resumed, never published).
    RetirementUncertain,
    Aborted(&'static str),
}

/// What the frozen caller supplies: the sanitised metadata copy, the RAW
/// save built from the frozen real seats, carts and kept mobs (the session
/// neutralises it itself), and the source's own records (`capture::local0_*`).
pub struct Capture<'a> {
    pub meta: &'a crate::save::WorldMeta,
    pub save: crate::save::WorldSave,
    pub world: &'a crate::world::World,
    pub disk_chunks: Option<&'a std::path::Path>,
    pub exhibit_dir: Option<&'a std::path::Path>,
    pub source_sidecar: &'a [u8],
    pub source_progress: &'a [u8],
}

pub struct SourceSession {
    stage: SourceStage,
    action: HostAction,
    cfg: SourceConfig,
    transfer: Id,
    challenge: Id,
    recipient_runtime: Option<[u8; 32]>,
    recipient_challenge: Id,
    acceptance: Option<String>,
    packed: Option<PackedCheckpoint>,
    retained: Option<super::retained::PreparedCopy>,
    retained_offer: Option<(String, String)>,
    retained_folder: Option<String>,
    core: Option<Core>,
    file: Option<std::fs::File>,
    sender: Option<Sender>,
    ready: Option<Event>,
    /// Shared with the bound link: dropping the session releases it.
    _lease: std::sync::Arc<()>,
    last_heard: u64,
    last_beat: u64,
    /// P4: when the recipient's verified consent froze this world.
    consented_at: Option<u64>,
    /// The pack worker's generation fence: only a result carrying this
    /// generation and transfer, arriving while `Packing`, is ever used.
    pack_generation: u64,
    pack_rx: Option<std::sync::mpsc::Receiver<PackResult>>,
    pack_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pack_worker: Option<std::thread::JoinHandle<()>>,
    source_agreement: Option<String>,
    /// H2: the recipient attestation `verify_accept` verified (sealed into
    /// Prepared/Commit as the successor's).
    recipient_attestation: Option<Event>,
    /// The core phase model, from the bearer's disclosure on.
    phase: Option<crate::handover::phase::Source>,
    /// The sealed Prepared and its verification under the bearer's key.
    prepared: Option<(Event, VerifiedPrepared)>,
    /// P2: the bearer has left this machine (abort now rotates it).
    disclosed: bool,
    rotate_invite: bool,
    /// The one operation `Budget` for every stored publication (Prepared,
    /// then Committed), moved to and back from the publish worker.
    budget: Option<Budget>,
    publish_generation: u64,
    publish_rx: Option<std::sync::mpsc::Receiver<PublishResult>>,
    publish_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    publish_worker: Option<std::thread::JoinHandle<()>>,
    /// The retained copy's fresh world id, fixed before any final bytes
    /// escape (it is in the retirement record).
    retained_copy: Id,
    /// After Retired: the exact signed Commit and the permit to publish it.
    commit: Option<Event>,
    permit: Option<PublicationPermit>,
    /// CP9: the Committed publication went to the background resumer.
    handed: bool,
    /// CP9 review LOW-1: the last hand-off try (retried at most once a second).
    hand_tried: Option<std::time::Instant>,
    /// CP13b (D2): this session's abort record is durable (its Aborted goes
    /// to the resumer like a Committed), or it couldn't be written.
    abort_recorded: bool,
    abort_unrecorded: bool,
    /// CP16 (CP14 review LOW-1): the sealed Aborted went out on the link
    /// (the send returned Ok). What `end_line` may claim about the other
    /// player's wait depends on it.
    abort_sent: bool,
    /// CP15: the Aborted sealed with the Prepared before the bearer left,
    /// and its abort record (durable as the pending abort record). Every
    /// abort after disclosure writes exactly this record.
    pending_abort: Option<(Event, AbortRecord)>,
    /// CP17 (CP15 review LOW-3): the world's handover lock, taken before the
    /// pending abort record is written and dropped once a terminal record
    /// is durable (or with the session; the OS drops it if the process dies).
    live_lock: Option<super::world_lock::LiveLock>,
    /// CP13b test hook: the abort record's write fails.
    #[cfg(test)]
    pub test_fail_abort_record: bool,
    #[cfg(test)]
    pack_hold: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
}

/// CP13b (D2): what `record_abort` did.
enum AbortRecorded {
    /// No bearer disclosed or no Prepared sealed: nothing to answer.
    Nothing,
    Recorded(Event),
    /// Sealed (when `Some`) but not durably recorded.
    Unrecorded(Option<Event>),
    /// A retirement record of this transfer already won.
    Retired,
}

fn random_id() -> Id {
    let mut id = [0; 16];
    getrandom::fill(&mut id).expect("randomness unavailable");
    id
}
fn npub(key: [u8; 32]) -> String {
    PublicKey::from_slice(&key).ok().and_then(|k| k.to_bech32().ok()).unwrap_or_default()
}

impl SourceSession {
    /// The host action: refusals first, then bind the exact seat and propose.
    pub fn begin(hs: &mut HostedServer, action: HostAction, cfg: SourceConfig, live: Live, now: u64) -> Result<Self, Refusal> {
        check(hs, &action, &cfg, live)?;
        if cfg.authority.epoch() >= 4 || now >= cfg.authority.expiry() {
            return Err(Refusal("The original invitation has expired or has no handover hops left."));
        }
        // CP17 (CP16 review LOW-2): a world with no room for this handover's
        // records is refused now, before it freezes and streams, not after
        // the whole world has been copied (disclosure checks again).
        if let Some(dir) = cfg.world_dir.as_deref()
            && super::authority::dir(dir).is_dir()
            && !matches!(super::authority::store(dir).and_then(|s| s.has_room_for_handover()), Ok(true))
        {
            return Err(Refusal(super::copy::WORLD_RECORDS_FULL));
        }
        // H1: no live invitation capability, no handover.
        if *cfg.bearer.expose() == [0; 16] {
            return Err(Refusal(super::copy::NO_LIVE_INVITE));
        }
        if let Some(directory) = &cfg.world_dir
            && let Some(info) = super::retained::read(directory).map_err(|_| Refusal(super::copy::AUTHORITY_UNREADABLE))?
        {
            let capability = crypto::root_capability(&cfg.root, cfg.bearer.expose()).map_err(|_| Refusal(super::copy::NO_LIVE_INVITE))?;
            if info.retirement.retired_roots().len() >= 64 && !info.retirement.refuses_root(capability, now) {
                return Err(Refusal("This world's retired invitation records are full. Nothing was changed."));
            }
        }
        let context = crate::handover::personal::RecordContext { participant: action.seat.key, lineage: cfg.lineage, generation: cfg.generation, transfer: [1; 16] };
        let save = crate::joiner_save::JoinerSave::capture(&hs.server.players[action.seat.slot], hs.server.run_id);
        let seed = save.encode_seed().map_err(|_| Refusal("The other player's inventory couldn't be recorded."))?;
        let progress = if let Some(personal) = &save.personal {
            let record = personal.record().map_err(|_| Refusal("The other player's progress couldn't be recorded."))?;
            crate::handover::personal::PersonalRecord::from_gameplay(context, &record).and_then(|record| record.encode()).map_err(|_| Refusal("The other player's progress couldn't be recorded."))?
        } else { super::capture::default_progress(context) };
        super::personal_adapter::preflight(&seed, &progress, context)?;
        let transfer = random_id();
        let s = action.seat;
        let lease = hs.handover_bind(s.slot, s.generation, s.key, transfer).map_err(Refusal)?;
        let mut session = Self {
            stage: SourceStage::Proposed,
            transfer,
            challenge: random_id(),
            recipient_runtime: None,
            recipient_challenge: [0; 16],
            acceptance: None,
            packed: None,
            retained: None,
            retained_offer: None,
            retained_folder: None,
            core: None,
            file: None,
            sender: None,
            ready: None,
            _lease: lease,
            last_heard: now,
            last_beat: now,
            consented_at: None,
            pack_generation: 0,
            pack_rx: None,
            pack_cancel: Default::default(),
            pack_worker: None,
            source_agreement: None,
            recipient_attestation: None,
            phase: None,
            prepared: None,
            disclosed: false,
            rotate_invite: false,
            budget: None,
            publish_generation: 0,
            publish_rx: None,
            publish_cancel: Default::default(),
            publish_worker: None,
            retained_copy: random_id(),
            commit: None,
            permit: None,
            handed: false,
            hand_tried: None,
            abort_recorded: false,
            abort_unrecorded: false,
            abort_sent: false,
            pending_abort: None,
            live_lock: None,
            #[cfg(test)]
            test_fail_abort_record: false,
            #[cfg(test)]
            pack_hold: None,
            action,
            cfg,
        };
        let propose = Frame::Propose {
            transfer: session.transfer,
            world_name: session.action.world_name.clone(),
            source_npub: npub(session.action.source_key),
            challenge: session.challenge,
            root_runtime: session.cfg.root.runtime(),
            root_persona: npub(session.cfg.root.persona()),
            root_expiry: session.cfg.root.expiry(),
            root_relays: session.cfg.root.relays().to_vec(),
            lineage: session.cfg.lineage,
        };
        if let Err(r) = session.send(hs, &propose) {
            hs.handover_release();
            return Err(r);
        }
        Ok(session)
    }

    /// The persona handing the world over (the host action's source key).
    pub fn source_key(&self) -> [u8; 32] {
        self.action.source_key
    }

    /// The context a participant record of this transfer is bound to.
    pub fn record_context(&self, participant: [u8; 32]) -> RecordContext {
        RecordContext {
            participant,
            lineage: self.cfg.lineage,
            generation: self.cfg.generation,
            transfer: self.transfer,
        }
    }

    /// The recipient's npub, for the host's status line.
    pub fn recipient_npub(&self) -> String {
        npub(self.action.seat.key)
    }

    pub fn stage(&self) -> SourceStage {
        self.stage
    }
    #[cfg_attr(not(test), expect(dead_code, reason = "Retained handover proof utility; the live controller uses its checked adapter"))]
    pub fn transfer(&self) -> Id {
        self.transfer
    }
    #[cfg_attr(not(test), expect(dead_code, reason = "Retained handover proof utility; the live controller uses its checked adapter"))]
    pub fn core(&self) -> Option<&Core> {
        self.core.as_ref()
    }
    #[cfg_attr(not(test), expect(dead_code, reason = "Retained handover proof utility; the live controller uses its checked adapter"))]
    pub fn ready(&self) -> Option<&Event> {
        self.ready.as_ref()
    }
    /// P2: the bearer has been sent to the recipient.
    #[cfg_attr(not(test), expect(dead_code, reason = "Retained handover proof utility; the live controller uses its checked adapter"))]
    pub fn bearer_disclosed(&self) -> bool {
        self.disclosed
    }
    /// P2: this session aborted after disclosing the bearer, so the caller
    /// must mint a fresh invitation (old invite strings stop admitting).
    pub fn needs_invite_rotation(&self) -> bool {
        self.rotate_invite
    }
    /// The retained copy's world id (in the retirement record).
    #[cfg_attr(not(test), expect(dead_code, reason = "Retained handover proof utility; the live controller uses its checked adapter"))]
    pub fn retained_copy(&self) -> Id {
        self.retained_copy
    }
    pub(crate) fn retained_folder(&self) -> Option<&str> { self.retained_folder.as_deref() }
    /// After Retired: the exact signed Commit bytes this source persisted.
    #[cfg_attr(not(test), expect(dead_code, reason = "Retained handover proof utility; the live controller uses its checked adapter"))]
    pub fn commit_event(&self) -> Option<&Event> {
        self.commit.as_ref()
    }
    /// CP9: at Retired, hand the Committed publication to `resumer`, which
    /// outlives this session, its world and `GameState`. It reads the
    /// record back through the world's store (`RetiredCommit`), the same
    /// path the start-up scan takes, so the exact persisted bytes are what
    /// is published. Once per session; false until handed.
    /// CP9 review LOW-1/LOW-4: handed only when the resumer took it (or
    /// already holds it); a failure is logged and retried at most once a
    /// second (the start-up scan stays the backstop). A
    /// `RetirementUncertain` session whose record reads back is handed too.
    /// CP13b (D2): an aborted session whose abort record is durable hands
    /// its Aborted over the same way (`RetiredCommit::read_aborted`).
    pub fn hand_publication(&mut self, resumer: &super::resumer::Resumer) -> bool {
        let aborted = matches!(self.stage, SourceStage::Aborted(_)) && self.abort_recorded;
        if self.handed || !(aborted || matches!(self.stage, SourceStage::Retired | SourceStage::RetirementUncertain)) {
            return self.handed;
        }
        if self.hand_tried.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1)) {
            return false;
        }
        let first = self.hand_tried.is_none();
        self.hand_tried = Some(std::time::Instant::now());
        let uncertain = self.stage == SourceStage::RetirementUncertain;
        let Some(dir) = self.cfg.world_dir.as_ref() else { return false };
        let t = self.transfer;
        let read = crate::handover::live::authority::store(dir).and_then(|s| {
            let retired = if aborted {
                crate::handover::phase::RetiredCommit::read_aborted(&s, t)?
            } else {
                // CP14 (CP10 review LOW-8): an uncertain retirement whose
                // record reads back is only VISIBLE, not proven durable (its
                // directory fsync failed). `Resumer::adopt` re-persists the
                // identical record (`RetiredCommit::persist_again`: Ok only
                // after the file and the directory are fsynced again) before
                // it takes the publication, so the Committed is handed only
                // once the retirement is durable (a lost rename after
                // publication would fork the world).
                crate::handover::phase::RetiredCommit::read(&s, t)?
            };
            Ok(retired.map(|r| (r.is_published(&s), r)))
        });
        match read {
            // Already proven and recorded: nothing left to hand.
            Ok(Some((true, _))) => self.handed = true,
            Ok(Some((false, retired))) => {
                let ledger = crate::handover::ledger::Ledger::install();
                self.handed = resumer.adopt(dir, retired, &ledger) || resumer.holds(t);
                // CP16 (CP14 review LOW-6): tried once a second; logged at
                // most once a minute.
                if !self.handed && super::resumer::log_due("hand-publication", t) {
                    log::error!("handover: the resumer couldn't take the publication; retrying (logged at most once a minute)");
                }
            }
            // An uncertain retirement whose record never landed: nothing to
            // publish (logged once, not every second).
            Ok(None) if uncertain => {
                if first {
                    log::warn!("handover: the retirement record never landed; nothing to publish");
                }
            }
            Ok(None) if super::resumer::log_due("hand-missing", t) => log::error!("handover: retired, but no retirement record to publish"),
            Err(e) if super::resumer::log_due("hand-unreadable", t) => log::error!("handover: the retirement record can't be read back: {e}"),
            Ok(None) | Err(_) => {}
        }
        self.handed
    }
    pub fn publication_handed(&self) -> bool {
        self.handed
    }
    /// CP14b (CP13b review LOW-5): the line `end_source_handover` shows when
    /// this session ends with `msg`: plus the re-issued invitation when the
    /// bearer had left, plus `copy::ABORT_UNRECORDED` when the abort record
    /// couldn't be written.
    pub fn end_line(&self, msg: &str) -> String {
        let mut line = msg.to_string();
        if self.needs_invite_rotation() {
            line = format!("{line} {}", super::copy::INVITE_REISSUED);
        }
        if self.abort_unrecorded {
            // CP16 (CP14 review LOW-1): say only what is known. A sealed
            // Aborted that went out ends a recipient's wait if it arrived
            // (before its ack it just ends; past it, it quarantines).
            let why = if self.abort_sent { super::copy::ABORT_UNRECORDED_SENT } else { super::copy::ABORT_UNRECORDED };
            line = format!("{line} {why}");
        }
        line
    }
    /// CP13b (D2): the abort record (the Aborted to publish) is durable.
    pub fn abort_recorded(&self) -> bool {
        self.abort_recorded
    }
    /// CP13b (D2): aborted after disclosure, but the abort record couldn't
    /// be written: play resumed anyway, and the caller shows
    /// `copy::ABORT_UNRECORDED`.
    #[cfg_attr(not(test), expect(dead_code, reason = "Retained handover proof utility; the live controller uses its checked adapter"))]
    pub fn abort_unrecorded(&self) -> bool {
        self.abort_unrecorded
    }

    /// CP13b (D2): every abort after the bearer was disclosed (this
    /// session's `abort`, its liveness aborts, and a session dropped without
    /// one) writes the abort record (ledger entry first) through the phase's
    /// one choke point BEFORE anything unfreezes. CP15: the record is the
    /// one sealed with the Prepared before the bearer left (`disclose`, the
    /// pending abort record), so the Aborted a crash leaves for the next
    /// start-up and the one a live abort writes are the same signed bytes.
    fn record_abort(&mut self) -> AbortRecorded {
        if !self.disclosed || self.abort_recorded || self.abort_unrecorded {
            return AbortRecorded::Nothing;
        }
        let Some((ev, record)) = self.pending_abort.clone() else {
            // Unreachable: `disclose` sets `disclosed` only after the
            // pending abort record is durable.
            log::error!("handover: disclosed without a pending abort record; play resumes without an abort record");
            self.abort_unrecorded = true;
            return AbortRecorded::Unrecorded(None);
        };
        #[cfg(test)]
        let fail = self.test_fail_abort_record;
        #[cfg(not(test))]
        let fail = false;
        let store = match self.cfg.world_dir.as_ref().map(|d| crate::handover::live::authority::store(d)) {
            Some(Ok(store)) if !fail => store,
            _ => {
                log::error!("handover: the abort record couldn't be written; play resumes anyway");
                self.abort_unrecorded = true;
                return AbortRecorded::Unrecorded(Some(ev));
            }
        };
        let Some(phase) = self.phase.as_mut() else { return AbortRecorded::Nothing };
        match phase.abort_disclosed(&store, &crate::handover::ledger::Ledger::install(), record) {
            Ok(()) => {
                self.abort_recorded = true;
                self.live_lock = None;
                AbortRecorded::Recorded(ev)
            }
            Err(crate::handover::Error::State(crate::handover::phase::RETIREMENT_RECORD_EXISTS)) => {
                self.live_lock = None;
                AbortRecorded::Retired
            }
            Err(e) => {
                log::error!("handover: the abort record couldn't be written ({e}); play resumes anyway");
                self.abort_unrecorded = true;
                AbortRecorded::Unrecorded(Some(ev))
            }
        }
    }
    /// CP17 test hook: whether this session holds the world's handover
    /// lock, and dropping it (a "kill" that `mem::forget`s the session
    /// would otherwise leak this process's guard).
    #[cfg(test)]
    pub fn test_holds_live_lock(&self) -> bool {
        self.live_lock.is_some()
    }
    #[cfg(test)]
    pub fn test_drop_live_lock(&mut self) {
        self.live_lock = None;
    }
    /// Retired or quarantined: the original world never resumes.
    pub fn past_retirement(&self) -> bool {
        matches!(self.stage, SourceStage::Retired | SourceStage::RetirementUncertain)
    }

    fn send(&mut self, hs: &mut HostedServer, f: &Frame) -> Result<(), Refusal> {
        let mut bytes = f.encode().map_err(|_| Refusal("A handover message was too large."))?;
        let r = hs.handover_send(&bytes).map_err(Refusal);
        // CP7 review LOW-1: the encode buffer we own is wiped (the bearer
        // frame's copy; transport-owned copies are documented in Spec 04).
        if matches!(f, Frame::Bearer { .. }) {
            zeroize::Zeroize::zeroize(&mut bytes);
        }
        r
    }

    /// Pre-retirement abort: tell the recipient, release the freeze, delete
    /// the scratch archive. The original world was never changed.
    /// From Retired on this does nothing: no deadline, Cancel or late Abort
    /// resumes a retired world (only exact retry or quarantine).
    pub fn abort(&mut self, hs: &mut HostedServer, why: &'static str) {
        self.abort_inner(Some(hs), why);
    }

    /// CP14b (CP13b review LOW-1): the same abort when the hosted server
    /// (and the link with it) has gone: the abort record is written first
    /// (a retirement record that won keeps the world retired and its fence
    /// up), nothing can be sent, and the world's late-save fence lifts as
    /// `handover_release` would have lifted it.
    pub fn abort_without_server(&mut self, why: &'static str) {
        self.abort_inner(None, why);
    }

    fn abort_inner(&mut self, hs: Option<&mut HostedServer>, why: &'static str) {
        if matches!(self.stage, SourceStage::Aborted(_)) || self.past_retirement() {
            return;
        }
        // CP13b (D2): durable before the release below. A retirement record
        // that already won keeps this world retired (frozen).
        let aborted = match self.record_abort() {
            AbortRecorded::Retired => {
                self.stage = SourceStage::RetirementUncertain;
                match hs {
                    Some(hs) => hs.handover_mark_retired(),
                    None => {
                        if let Some(dir) = self.cfg.world_dir.as_ref() {
                            crate::save_fence::retire(dir);
                        }
                    }
                }
                return;
            }
            AbortRecorded::Recorded(ev) | AbortRecorded::Unrecorded(Some(ev)) => Some(ev.as_json()),
            AbortRecorded::Unrecorded(None) | AbortRecorded::Nothing => None,
        };
        match hs {
            Some(hs) => {
                let carried = aborted.is_some();
                let sent = self.send(hs, &Frame::Abort { transfer: self.transfer, aborted }).is_ok();
                self.abort_sent = carried && sent;
                hs.handover_release();
            }
            None => {
                if let Some(dir) = self.cfg.world_dir.as_ref() {
                    crate::save_fence::lift(dir);
                }
            }
        }
        // M-B: a running pack is cancelled and fenced off; its result, if it
        // still arrives, is dropped (and its scratch with it), never sent.
        self.pack_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        self.pack_generation += 1;
        self.sender = None;
        self.file = None;
        self.packed = None;
        // A running publication is cancelled and fenced off the same way.
        self.publish_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        self.publish_generation += 1;
        self.publish_rx = None;
        if let Some(p) = self.phase.as_mut() {
            let _ = p.abort();
        }
        // P2: the old bearer has left this machine.
        self.rotate_invite = self.disclosed;
        self.stage = SourceStage::Aborted(why);
    }

    fn fail(&mut self, hs: &mut HostedServer, r: Refusal) -> Refusal {
        self.abort(hs, r.0);
        r
    }

    /// Drive control: read the bound seat's frames, keep the transfer
    /// window full. Any refusal aborts (pre-retirement) and is returned.
    /// After Ready the source is still frozen and still listening: a
    /// recipient Abort releases it.
    pub fn pump(&mut self, hs: &mut HostedServer, live: Live, now: u64) -> Result<(), Refusal> {
        // Disclosure waits for the pump after the one that verified Ready (a
        // recipient Abort or a stall already waiting is heard first).
        let was_ready = self.stage == SourceStage::ReadyVerified;
        // A finished pack is drained first, even after an abort: a cancelled
        // or superseded result is dropped here, never sent.
        let r = match self.take_pack(false) {
            Ok(Some(msg)) => self.on_pack(hs, live, msg, now),
            Ok(None) => Ok(()),
            Err(r) => Err(r),
        };
        if let Err(r) = r {
            return Err(self.fail(hs, r));
        }
        if matches!(self.stage, SourceStage::Aborted(_) | SourceStage::RetirementUncertain) {
            return Ok(());
        }
        let r = match self.take_publish() {
            Ok(Some(stored)) => self.on_stored(hs, stored, now),
            Ok(None) => Ok(()),
            Err(r) => Err(r),
        };
        if let Err(r) = r {
            return Err(self.fail(hs, r));
        }
        if matches!(self.stage, SourceStage::Aborted(_)) {
            return Ok(());
        }
        let frames = match hs.handover_take_frames() {
            Ok(f) => f,
            Err(why) => return Err(self.fail(hs, Refusal(why))),
        };
        for bytes in frames {
            let frame = match Frame::decode(&bytes) {
                Ok(f) if f.transfer() == self.transfer => f,
                _ => return Err(self.fail(hs, Refusal("The recipient sent a malformed handover message."))),
            };
            // Review LOW-3: while streaming a repeated Ack is not progress and
            // doesn't feed the stall clock (a new Ack does, in `on_frame`).
            // CP5 review LOW-5: a Notice feeds it only once every chunk is
            // acknowledged (the recipient is importing).
            let complete = self.sender.as_ref().is_some_and(|s| s.complete());
            let no_progress = matches!(frame, Frame::Ack { .. }) || (matches!(frame, Frame::Notice { .. }) && !complete);
            if !(self.stage == SourceStage::Streaming && no_progress) {
                self.last_heard = now;
            }
            if let Err(r) = self.on_frame(hs, live, frame, now) {
                return Err(self.fail(hs, r));
            }
        }
        let quiet = now.saturating_sub(self.last_heard);
        // L2: a silent recipient times out while streaming and after Ready
        // (until Prepared advances it); the pack is this machine's own work.
        // The publish worker is this machine's own work (its Budget bounds
        // it); after Retired no deadline fires (CP5 LOW-7).
        let stalled = match self.stage {
            SourceStage::Proposed => quiet > CONSENT_SECS,
            SourceStage::Streaming | SourceStage::ReadyVerified | SourceStage::Prepared => quiet > STALL_SECS,
            _ => false,
        };
        if stalled {
            return Err(self.fail(hs, Refusal("The other player stopped responding. Play has resumed.")));
        }
        if !self.past_retirement() && self.consented_at.is_some_and(|t| now.saturating_sub(t) > OPERATION_SECS) {
            return Err(self.fail(hs, Refusal(super::copy::TOO_LONG_SOURCE)));
        }
        // M-2: heartbeats while packing and from Ready until the recipient
        // has heard the end (its own stall clock waits on the source).
        let beats = matches!(
            self.stage,
            SourceStage::Packing | SourceStage::ReadyVerified | SourceStage::Publishing | SourceStage::Prepared | SourceStage::Retired
        );
        if beats && now.saturating_sub(self.last_beat) >= HEARTBEAT_SECS {
            self.last_beat = now;
            if let Err(r) = self.send(hs, &Frame::Notice { transfer: self.transfer, paused: true }) {
                return Err(self.fail(hs, r));
            }
        }
        if self.stage == SourceStage::Streaming
            && let Err(r) = self.fill(hs)
        {
            return Err(self.fail(hs, r));
        }
        // Ready verified on an earlier pump: disclose and publish Prepared.
        if was_ready
            && self.stage == SourceStage::ReadyVerified
            && let Err(r) = self.disclose(hs, live, now)
        {
            return Err(self.fail(hs, r));
        }
        Ok(())
    }

    /// Every boundary re-reads identity, seats, live seat state and the
    /// world's saved records.
    fn check(&self, hs: &HostedServer, live: Live) -> Result<(), Refusal> {
        check(hs, &self.action, &self.cfg, live)
    }

    fn on_frame(&mut self, hs: &mut HostedServer, live: Live, frame: Frame, now: u64) -> Result<(), Refusal> {
        match (self.stage, frame) {
            // Nothing the recipient sends resumes a retired world (a late
            // Abort included); only exact retry or quarantine remain.
            (SourceStage::Retired | SourceStage::RetirementUncertain, _) => Ok(()),
            (_, Frame::Abort { .. }) => Err(Refusal("The other player cancelled the handover.")),
            (SourceStage::Proposed, Frame::Decline { reason, .. }) => Err(Refusal(super::copy::host_declined(reason))),
            (SourceStage::Proposed, Frame::Accept { runtime, attestation, acceptance, challenge, .. }) => {
                let attestation = self.verify_accept(runtime, &attestation, &acceptance, challenge, now)?;
                // Re-authenticated: the same identity, seat and records,
                // still eligible, then freeze.
                self.check(hs, live)?;
                hs.handover_freeze(self.cfg.world_dir.as_deref()).map_err(Refusal)?;
                self.recipient_runtime = Some(runtime);
                self.recipient_challenge = challenge;
                self.acceptance = Some(acceptance);
                self.recipient_attestation = Some(attestation);
                self.consented_at = Some(now);
                self.stage = SourceStage::Frozen;
                Ok(())
            }
            // M1: the recipient's heartbeat while its import runs on a worker.
            (SourceStage::Streaming, Frame::Notice { .. }) => Ok(()),
            (SourceStage::Streaming, Frame::Ack { transfer, through }) => {
                let sender = self.sender.as_mut().ok_or(Refusal("No transfer is running."))?;
                if sender
                    .acknowledge(transfer, through)
                    .map_err(|_| Refusal("The recipient acknowledged data it was never sent."))?
                {
                    self.last_heard = now;
                }
                Ok(())
            }
            (SourceStage::Streaming, Frame::Ready { event, .. }) => {
                let done = self.sender.as_ref().is_some_and(|s| s.complete());
                let core = self.core.as_ref().ok_or(Refusal("No certificate."))?;
                let ev = Event::from_json(&event).map_err(|_| Refusal("The recipient's Ready was malformed."))?;
                if !done || crate::handover::crypto::verify_ready(&ev, core, Some(self.challenge)).is_err() {
                    return Err(Refusal("The recipient's Ready didn't verify."));
                }
                self.ready = Some(ev);
                self.sender = None;
                self.file = None;
                self.stage = SourceStage::ReadyVerified;
                Ok(())
            }
            (SourceStage::Prepared, Frame::PreparedAck { event, .. }) => self.on_prepared_ack(hs, live, &event, now),
            _ => Err(Refusal("The recipient sent a handover message out of order.")),
        }
    }

}

/// Review LOW-7: a session dropped without `abort` still stops its pack
/// worker at the next member (its result is dropped with the receiver).
impl Drop for SourceSession {
    fn drop(&mut self) {
        // CP13b (D2): dropped after disclosure without `abort` (the link's
        // reap then releases the world): the abort record still comes first.
        if !matches!(self.stage, SourceStage::Aborted(_)) && !self.past_retirement() {
            // CP14b (CP13b review LOW-1): a durable abort record is handed
            // to the process's resumer at once, so the Aborted is published
            // in this process (not only by the next start-up scan) and a
            // recipient past its ack stops waiting.
            if let AbortRecorded::Recorded(_) = self.record_abort() {
                self.stage = SourceStage::Aborted("The handover ended.");
                if !self.hand_publication(super::resumer::Resumer::global()) {
                    log::warn!("handover: a dropped session's Aborted wasn't handed over; it is published at the next start");
                }
            }
        }
        self.pack_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        self.publish_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The refusals every boundary re-runs. The source persona must be the
/// signed-in identity now AND the persona of the authority this machine
/// holds (a sign-in or profile change can't remap the world's seat 0).
fn check(hs: &HostedServer, action: &HostAction, cfg: &SourceConfig, live: Live) -> Result<(), Refusal> {
    eligibility::local_seats(live.local_seats)?;
    eligibility::source_identity(action.source_key, live.signed_in)?;
    // L3: only a host that lends its one world can capture what its joiners
    // put into it; a `--no-lend` host's server copy diverges from it.
    if !hs.lends_host_world() {
        return Err(Refusal(super::copy::NO_LEND));
    }
    if cfg.authority.runtime() != cfg.runtime.public_key().to_bytes() {
        return Err(Refusal("This machine doesn't hold this world's invitation."));
    }
    if cfg.authority.persona() != action.source_key {
        return Err(Refusal("This world's invitation belongs to a different identity than the one signed in."));
    }
    eligibility::seats(&hs.server, action.seat)?;
    // CP17 (CP15 review LOW-3): another process is handing this world over.
    if cfg.world_dir.as_deref().is_some_and(super::world_lock::held_elsewhere) {
        return Err(Refusal(super::copy::HANDOVER_ELSEWHERE));
    }
    let saved = match &cfg.world_dir {
        Some(dir) => eligibility::saved_record_keys(dir)?,
        None => Vec::new(),
    };
    eligibility::saved_records(&saved, action.source_key, action.seat.key)
}

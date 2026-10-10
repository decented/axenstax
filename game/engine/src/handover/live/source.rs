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
use super::wire::{Frame, PART_LIMIT, acceptance_content, prepared_ack_content, source_agreement_content};
use crate::handover::{
    Hash, Id,
    certificate::Core,
    crypto::{self, Seal, Verified, VerifiedPrepared},
    phase::{PublicationPermit, RetiredRoot, Retirement},
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
type PackResult = (u64, Id, crate::handover::Result<PackedCheckpoint>);
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
    #[cfg(test)]
    pack_hold: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
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
        // H1: no live invitation capability, no handover.
        if *cfg.bearer.expose() == [0; 16] {
            return Err(Refusal(super::copy::NO_LIVE_INVITE));
        }
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
    pub fn transfer(&self) -> Id {
        self.transfer
    }
    pub fn core(&self) -> Option<&Core> {
        self.core.as_ref()
    }
    pub fn ready(&self) -> Option<&Event> {
        self.ready.as_ref()
    }
    /// P2: the bearer has been sent to the recipient.
    pub fn bearer_disclosed(&self) -> bool {
        self.disclosed
    }
    /// P2: this session aborted after disclosing the bearer, so the caller
    /// must mint a fresh invitation (old invite strings stop admitting).
    pub fn needs_invite_rotation(&self) -> bool {
        self.rotate_invite
    }
    /// The retained copy's world id (in the retirement record).
    pub fn retained_copy(&self) -> Id {
        self.retained_copy
    }
    /// After Retired: the exact signed Commit bytes this source persisted.
    pub fn commit_event(&self) -> Option<&Event> {
        self.commit.as_ref()
    }
    /// Retired or quarantined: the original world never resumes.
    pub fn past_retirement(&self) -> bool {
        matches!(self.stage, SourceStage::Retired | SourceStage::RetirementUncertain)
    }

    fn send(&mut self, hs: &mut HostedServer, f: &Frame) -> Result<(), Refusal> {
        let bytes = f.encode().map_err(|_| Refusal("A handover message was too large."))?;
        hs.handover_send(&bytes).map_err(Refusal)
    }

    /// Pre-retirement abort: tell the recipient, release the freeze, delete
    /// the scratch archive. The original world was never changed.
    /// From Retired on this does nothing: no deadline, Cancel or late Abort
    /// resumes a retired world (only exact retry or quarantine).
    pub fn abort(&mut self, hs: &mut HostedServer, why: &'static str) {
        if matches!(self.stage, SourceStage::Aborted(_)) || self.past_retirement() {
            return;
        }
        let _ = self.send(hs, &Frame::Abort { transfer: self.transfer });
        hs.handover_release();
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
            && let Err(r) = self.disclose(hs, now)
        {
            return Err(self.fail(hs, r));
        }
        Ok(())
    }

    /// P2 + M-2: seal the inert Prepared to the key the bearer derives,
    /// disclose the bearer (the point of no return for it), then publish and
    /// read Prepared back on a fenced worker.
    fn disclose(&mut self, hs: &mut HostedServer, now: u64) -> Result<(), Refusal> {
        hs.handover_check().map_err(Refusal)?;
        if self.cfg.world_dir.is_none() {
            return Err(Refusal(super::copy::NEVER_SAVED));
        }
        let unsealed = Refusal("The handover record couldn't be sealed. Play has resumed.");
        let core = self.core.clone().ok_or(Refusal("No certificate."))?;
        let ready = self.ready.clone().ok_or(Refusal("No Ready."))?;
        let successor = self.recipient_attestation.clone().ok_or(Refusal("No recipient attestation."))?;
        let keys = crypto::recipient(&self.cfg.root, self.cfg.bearer.expose(), None).map_err(|_| unsealed.clone())?;
        let seal = Seal {
            source: &self.cfg.runtime,
            recipient: &keys,
            core: &core,
            ready: &ready,
            challenge: self.challenge,
            source_attestation: &self.cfg.attestation,
            successor_attestation: &successor,
            prepared: None,
        };
        let event = pollster::block_on(crypto::seal(seal, &self.cfg.root, now)).map_err(|_| unsealed.clone())?;
        let Ok(Verified::Prepared(vp)) = crypto::open(&event, &keys, core.source_runtime, &self.cfg.root, now, None) else {
            return Err(unsealed);
        };
        let phase = crate::handover::phase::Source::frozen(core, &self.cfg.authority).map_err(|_| unsealed)?;
        // Set before the send: a partly written frame may have left.
        self.disclosed = true;
        self.send(hs, &Frame::Bearer { transfer: self.transfer, bearer: self.cfg.bearer.clone() })?;
        self.phase = Some(phase);
        self.spawn_publish(event.clone(), Verified::Prepared(vp.clone()), None, keys.public_key().to_bytes())?;
        self.prepared = Some((event, vp));
        self.stage = SourceStage::Publishing;
        Ok(())
    }

    /// One stored publication (publish, matching OK, fresh readback) on a
    /// worker with its own reactor, charged to the operation's one Budget.
    /// Relays are tried in the invitation's order until one stores it.
    fn spawn_publish(&mut self, event: Event, verified: Verified, permit: Option<PublicationPermit>, recipient: Hash) -> Result<(), Refusal> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (generation, transfer) = (self.publish_generation, self.transfer);
        let root = self.cfg.root.clone();
        let author = self.cfg.runtime.public_key().to_bytes();
        let mut budget = self.budget.take().unwrap_or_default();
        let cancel = self.publish_cancel.clone();
        let worker = std::thread::Builder::new()
            .name("handover-publish".into())
            .spawn(move || {
                let route = Route { author, recipient, until: root.expiry() };
                let r = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Err(_) => Err(crate::handover::Error::Io),
                    Ok(rt) => rt.block_on(async {
                        let mut last = Err(crate::handover::Error::Invalid("no relay"));
                        for url in root.relays() {
                            let p = Publication { event: &event, verified: &verified, permit: permit.as_ref(), route };
                            match crate::handover::stored::publish_readback(url, &root, p, &mut budget, &cancel).await {
                                Ok(stored) => return Ok(stored),
                                Err(e) => last = Err(e),
                            }
                        }
                        last
                    }),
                };
                let _ = tx.send((generation, transfer, budget, r));
            })
            .map_err(|_| Refusal("The handover record couldn't be published. Play has resumed."))?;
        self.publish_rx = Some(rx);
        self.publish_worker = Some(worker);
        Ok(())
    }

    /// The publish worker's result for this generation, if it has arrived.
    /// A worker that ended without one refuses while it is awaited.
    fn take_publish(&mut self) -> Result<Option<crate::handover::Result<StoredProof>>, Refusal> {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = self.publish_rx.as_ref() else { return Ok(None) };
        let msg = match rx.try_recv() {
            Ok(m) => m,
            Err(TryRecvError::Empty) => return Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.publish_rx = None;
                return if self.stage == SourceStage::Publishing {
                    Err(Refusal(super::copy::PREPARED_UNSTORED))
                } else {
                    Ok(None)
                };
            }
        };
        self.publish_rx = None;
        let (generation, transfer, budget, r) = msg;
        if generation != self.publish_generation || transfer != self.transfer {
            return Ok(None);
        }
        self.budget = Some(budget);
        Ok(Some(r))
    }

    /// Test builds: wait for the publish worker to finish.
    #[cfg(test)]
    pub fn join_publish_worker(&mut self) {
        if let Some(w) = self.publish_worker.take() {
            w.join().expect("publish worker");
        }
    }

    /// Prepared is stored (witnessed by the readback): the phase model moves
    /// to Prepared and the recipient gets the exact event.
    fn on_stored(&mut self, hs: &mut HostedServer, stored: crate::handover::Result<StoredProof>, now: u64) -> Result<(), Refusal> {
        if self.stage != SourceStage::Publishing {
            return Ok(());
        }
        let stored = stored.map_err(|_| Refusal(super::copy::PREPARED_UNSTORED))?;
        let (event, vp) = self.prepared.as_ref().ok_or(Refusal("No Prepared."))?;
        let event = event.as_json();
        self.phase
            .as_mut()
            .ok_or(Refusal("No Prepared."))?
            .prepared(vp, &stored)
            .map_err(|_| Refusal(super::copy::PREPARED_UNSTORED))?;
        self.send(hs, &Frame::Prepared { transfer: self.transfer, event })?;
        // The recipient's clock for persisting it and answering starts now.
        self.last_heard = now;
        self.stage = SourceStage::Prepared;
        Ok(())
    }

    /// M-2: the recipient's signed acknowledgement of its durable Prepared,
    /// bound to this transfer, the Prepared event hash and the capability
    /// the disclosed bearer derives. Only then: seal the Commit locally and
    /// durably retire with its exact bytes, BEFORE any of them escape.
    fn on_prepared_ack(&mut self, ack: &str, now: u64) -> Result<(), Refusal> {
        let bad = Refusal("The recipient's acknowledgement didn't verify. Play has resumed.");
        let (_, vp) = self.prepared.clone().ok_or(Refusal("No Prepared."))?;
        let capability = crypto::root_capability(&self.cfg.root, self.cfg.bearer.expose()).map_err(|_| bad.clone())?;
        let rt = self.recipient_runtime.ok_or(bad.clone())?;
        let ev = Event::from_json(ack).map_err(|_| bad.clone())?;
        if ev.pubkey.to_bytes() != rt
            || ev.verify().is_err()
            || ev.kind != Kind::Custom(crate::handover::STORED_KIND)
            || ev.content != prepared_ack_content(self.transfer, vp.event_hash(), capability)
            || !ev.tags.is_empty()
        {
            return Err(bad);
        }
        self.retire(&vp, capability, now)
    }

    fn retire(&mut self, vp: &VerifiedPrepared, capability: Hash, now: u64) -> Result<(), Refusal> {
        let unsealed = Refusal("The final handover record couldn't be sealed. Play has resumed.");
        let core = self.core.clone().ok_or(Refusal("No certificate."))?;
        let ready = self.ready.clone().ok_or(Refusal("No Ready."))?;
        let successor = self.recipient_attestation.clone().ok_or(Refusal("No recipient attestation."))?;
        let keys = crypto::recipient(&self.cfg.root, self.cfg.bearer.expose(), None).map_err(|_| unsealed.clone())?;
        let seal = Seal {
            source: &self.cfg.runtime,
            recipient: &keys,
            core: &core,
            ready: &ready,
            challenge: self.challenge,
            source_attestation: &self.cfg.attestation,
            successor_attestation: &successor,
            prepared: Some(vp),
        };
        let commit = pollster::block_on(crypto::seal(seal, &self.cfg.root, now)).map_err(|_| unsealed.clone())?;
        let Ok(Verified::Committed(vc)) = crypto::open(&commit, &keys, core.source_runtime, &self.cfg.root, now, Some(vp)) else {
            return Err(unsealed);
        };
        let world_dir = self.cfg.world_dir.as_ref().ok_or(Refusal(super::copy::NEVER_SAVED))?;
        let store = super::authority::store(world_dir).map_err(|_| Refusal(super::copy::AUTHORITY_UNREADABLE))?;
        let record = Retirement::new(
            &vc,
            vp,
            commit.as_json().into_bytes(),
            self.retained_copy,
            capability,
            vec![RetiredRoot { capability, expiry: self.cfg.root.expiry() }],
        )
        .map_err(|_| unsealed)?;
        let phase = self.phase.as_mut().ok_or(Refusal("No Prepared."))?;
        match phase.retire(&store, record) {
            Ok(permit) => {
                self.permit = Some(permit);
                self.commit = Some(commit);
                self.stage = SourceStage::Retired;
                Ok(())
            }
            // A write that may have reached the disk: quarantined, frozen.
            Err(_) if phase.stage() == crate::handover::phase::SourceStage::RetirementUncertain => {
                self.stage = SourceStage::RetirementUncertain;
                Err(Refusal(super::copy::RETIREMENT_UNCERTAIN))
            }
            Err(_) => Err(Refusal("This world couldn't be retired safely. Play has resumed.")),
        }
    }

    /// The worker's message, if it has sent one. A worker that ended without
    /// sending (it panicked) refuses while the session still waits for it,
    /// so the source never sits frozen in `Packing` (it has no stall deadline).
    fn take_pack(&mut self, block: bool) -> Result<Option<PackResult>, Refusal> {
        use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
        let Some(rx) = self.pack_rx.as_ref() else { return Ok(None) };
        let died = Refusal("The world couldn't be packed for the handover. Play has resumed.");
        let msg = if block {
            match rx.recv_timeout(std::time::Duration::from_secs(120)) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    self.pack_rx = None;
                    return if self.stage == SourceStage::Packing { Err(died) } else { Ok(None) };
                }
            }
        } else {
            match rx.try_recv() {
                Ok(m) => Some(m),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    self.pack_rx = None;
                    return if self.stage == SourceStage::Packing { Err(died) } else { Ok(None) };
                }
            }
        };
        if msg.is_some() {
            self.pack_rx = None;
        }
        Ok(msg)
    }

    /// Test hook: the pack worker ended without a result (as a panic would).
    #[cfg(test)]
    pub fn test_lose_pack_worker(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel::<PackResult>();
        drop(tx);
        self.pack_rx = Some(rx);
    }

    /// The generation fence. Anything but this session's live pack, still
    /// awaited, is dropped (its `PackedCheckpoint` removes its scratch).
    fn on_pack(&mut self, hs: &mut HostedServer, live: Live, msg: PackResult, now: u64) -> Result<(), Refusal> {
        let (generation, transfer, result) = msg;
        if generation != self.pack_generation || transfer != self.transfer || self.stage != SourceStage::Packing {
            drop(result);
            return Ok(());
        }
        let packed = result.map_err(|_| {
            Refusal("The world couldn't be packed for the handover (too large, or something in it can't move yet).")
        })?;
        self.after_pack(hs, live, packed, now)
    }

    /// Test hook: hold the pack worker once its archive is written. The
    /// first receiver hears when it got there; it waits for the sender.
    #[cfg(test)]
    pub fn hold_pack(&mut self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (reached_tx, reached_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        self.pack_hold = Some((reached_tx, release_rx));
        (reached_rx, release_tx)
    }
    /// Test hook: the pack worker's cancel flag.
    #[cfg(test)]
    pub fn pack_cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.pack_cancel.clone()
    }
    /// Test hook: wait for the pack worker thread to end.
    #[cfg(test)]
    pub fn join_pack_worker(&mut self) {
        if let Some(w) = self.pack_worker.take() {
            w.join().expect("pack worker");
        }
    }
    /// Test hook: wait for the pack and use it, as a later pump would.
    #[cfg(test)]
    pub fn settle_pack(&mut self, hs: &mut HostedServer, live: Live, now: u64) -> Result<(), Refusal> {
        let r = match self.take_pack(true) {
            Ok(Some(msg)) => self.on_pack(hs, live, msg, now),
            Ok(None) => Ok(()),
            Err(r) => Err(r),
        };
        r.map_err(|r| self.fail(hs, r))
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
                hs.handover_freeze().map_err(Refusal)?;
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
            (SourceStage::Prepared, Frame::PreparedAck { event, .. }) => self.on_prepared_ack(&event, now),
            _ => Err(Refusal("The recipient sent a handover message out of order.")),
        }
    }

    /// H2: returns the recipient attestation it verified (Prepared and
    /// Commit seal it as the successor's).
    fn verify_accept(&self, runtime: [u8; 32], attestation: &str, acceptance: &str, challenge: Id, now: u64) -> Result<Event, Refusal> {
        let bad = Refusal("The other player's consent couldn't be verified.");
        if challenge == [0; 16] {
            return Err(bad);
        }
        let rt = PublicKey::from_slice(&runtime).map_err(|_| bad.clone())?;
        let att = Event::from_json(attestation).map_err(|_| bad.clone())?;
        let persona = crate::runtime_identity::verify_player_attestation(&att, &rt, Timestamp::from(now))
            .map_err(|_| bad.clone())?;
        if persona.to_bytes() != self.action.seat.key {
            return Err(Refusal("The consent came from a different identity than the chosen player."));
        }
        let ev = Event::from_json(acceptance).map_err(|_| bad.clone())?;
        let expected = acceptance_content(self.transfer, self.challenge, &self.action.world_name, &npub(self.action.source_key));
        if ev.pubkey != rt
            || ev.verify().is_err()
            || ev.kind != nostr::Kind::Custom(crate::handover::STORED_KIND)
            || ev.content != expected
            || !ev.tags.is_empty()
        {
            return Err(bad);
        }
        Ok(att)
    }

    /// The frozen capture: re-check, pack the sanitised copies with both
    /// players' own records, certify and offer. Called once, while frozen.
    pub fn offer(&mut self, hs: &mut HostedServer, live: Live, capture: Capture<'_>, now: u64) -> Result<(), Refusal> {
        if self.stage != SourceStage::Frozen || !hs.handover_frozen() {
            return Err(Refusal("The world isn't frozen for a handover."));
        }
        let r = self.offer_inner(hs, live, capture, now);
        r.map_err(|r| self.fail(hs, r))
    }

    /// Is the session waiting for the frozen capture?
    pub fn wants_capture(&self) -> bool {
        self.stage == SourceStage::Frozen
    }

    fn offer_inner(&mut self, hs: &mut HostedServer, live: Live, c: Capture<'_>, now: u64) -> Result<(), Refusal> {
        hs.handover_check().map_err(Refusal)?;
        self.check(hs, live)?;
        let seat = self.action.seat;
        let mut save = c.save;
        eligibility::neutralise(&mut save, &npub(self.action.source_key))?;
        let recipient_record = crate::joiner_save::JoinerSave::capture(&hs.server.players[seat.slot], hs.server.run_id);
        eligibility::recipient_record(Some(&recipient_record))?;
        let recipient_sidecar =
            recipient_record.encode().map_err(|_| Refusal("The other player's inventory couldn't be recorded."))?;
        let context = |participant| RecordContext {
            participant,
            lineage: self.cfg.lineage,
            generation: self.cfg.generation,
            transfer: self.transfer,
        };
        let recipient_progress = super::capture::default_progress(context(seat.key));
        let (src_npub, dst_npub) = (npub(self.action.source_key), npub(seat.key));
        let rows = [
            ParticipantMember { npub: &src_npub, kind: MemberKind::Sidecar, schema: 1, bytes: c.source_sidecar },
            ParticipantMember { npub: &src_npub, kind: MemberKind::Progression, schema: 1, bytes: c.source_progress },
            ParticipantMember { npub: &dst_npub, kind: MemberKind::Sidecar, schema: 1, bytes: &recipient_sidecar },
            ParticipantMember { npub: &dst_npub, kind: MemberKind::Progression, schema: 1, bytes: &recipient_progress },
        ];
        // Source consent: its runtime-signed agreement (the runtime the
        // certificate names), sent with the offer; the recipient's is its
        // runtime-signed acceptance. The manifest binds both hashes.
        let source_agreement = pollster::block_on(
            EventBuilder::new(
                Kind::Custom(crate::handover::STORED_KIND),
                source_agreement_content(self.transfer, self.challenge, &self.action.world_name, &dst_npub),
            )
            .custom_created_at(Timestamp::from(now))
            .sign(&self.cfg.runtime),
        )
        .map_err(|_| Refusal("Your agreement couldn't be signed."))?
        .as_json();
        let source_agreement_hash = crate::handover::hash(source_agreement.as_bytes());
        self.source_agreement = Some(source_agreement);
        let mut agreements = vec![
            Agreement { participant: self.action.source_key, proof: source_agreement_hash },
            Agreement {
                participant: seat.key,
                proof: crate::handover::hash(self.acceptance.as_deref().unwrap_or_default().as_bytes()),
            },
        ];
        agreements.sort_by_key(|a| a.participant);
        // M-B: copy the frozen cut, then pack (disk reads, hashing, gzip) on
        // a worker; the frame, the control pump and Cancel keep running.
        let limits = checkpoint::Limits::default();
        let snapshot = checkpoint::Snapshot::take(
            save,
            checkpoint::SnapshotSource {
                meta: c.meta,
                world: c.world,
                disk_chunks: c.disk_chunks,
                exhibit_dir: c.exhibit_dir,
                participants: &rows,
            },
            limits,
        )
        .map_err(|_| Refusal("The world couldn't be packed for the handover (too large, or something in it can't move yet)."))?;
        let binding = PackBinding {
            source: self.action.source_key,
            receiver: seat.key,
            world: self.cfg.world,
            lineage: self.cfg.lineage,
            transfer: self.transfer,
            generation: self.cfg.generation,
            agreements,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (generation, transfer, scratch) = (self.pack_generation, self.transfer, self.cfg.scratch.clone());
        let control = checkpoint::PackControl {
            cancel: self.pack_cancel.clone(),
            #[cfg(test)]
            hold: self.pack_hold.take(),
        };
        let worker = std::thread::Builder::new()
            .name("handover-pack".into())
            .spawn(move || {
                let r = checkpoint::pack_snapshot(&snapshot, binding, &scratch, limits, &control);
                // A dropped session's receiver is gone: the result (and its
                // scratch) is dropped right here.
                let _ = tx.send((generation, transfer, r));
            })
            .map_err(|_| Refusal("The world couldn't be packed for the handover."))?;
        self.pack_rx = Some(rx);
        self.pack_worker = Some(worker);
        self.last_beat = now;
        self.stage = SourceStage::Packing;
        Ok(())
    }

    /// The pack is back (fenced): re-check, certify, offer, stream.
    fn after_pack(&mut self, hs: &mut HostedServer, live: Live, packed: PackedCheckpoint, now: u64) -> Result<(), Refusal> {
        hs.handover_check().map_err(Refusal)?;
        self.check(hs, live)?;
        let seat = self.action.seat;
        let source_agreement = self.source_agreement.take().ok_or(Refusal("No agreement."))?;
        let manifest = packed.manifest().encode().map_err(|_| Refusal("The world's manifest is too large."))?;
        let epoch = self.cfg.authority.epoch() + 1;
        let core = Core {
            context: self.cfg.root.digest(),
            root_runtime: self.cfg.root.runtime(),
            root_persona: self.cfg.root.persona(),
            root_expiry: self.cfg.root.expiry(),
            lineage: self.cfg.lineage,
            snapshot_world: self.cfg.world,
            manifest: crate::handover::hash(&manifest),
            generation: self.cfg.generation,
            epoch,
            predecessor: self.cfg.authority.committed(),
            transfer: self.transfer,
            source_runtime: self.cfg.runtime.public_key().to_bytes(),
            source_persona: self.action.source_key,
            successor_runtime: self.recipient_runtime.ok_or(Refusal("No recipient runtime."))?,
            successor_persona: seat.key,
            issued_at: now,
            not_before: now,
            expires_at: now.saturating_add(self.cfg.validity).min(self.cfg.root.expiry()),
            remaining_hops: 4u32.saturating_sub(epoch) as u8,
        };
        core.validate(&self.cfg.root, now).map_err(|_| Refusal("The invitation has expired or can't carry another handover."))?;
        let offer = pollster::block_on(checkpoint::sign_offer(&self.cfg.runtime, &core, self.recipient_challenge))
            .map_err(|_| Refusal("The checkpoint offer couldn't be signed."))?
            .as_json();
        let total = manifest.len() as u32;
        for (i, part) in manifest.chunks(PART_LIMIT).enumerate() {
            let f = Frame::ManifestPart { transfer: self.transfer, total, offset: (i * PART_LIMIT) as u32, bytes: part.to_vec() };
            self.send(hs, &f)?;
        }
        self.send(hs, &Frame::Offer { transfer: self.transfer, core: core.encode(), offer, agreement: source_agreement })?;
        let archive = packed.archive();
        let len = std::fs::metadata(archive).map_err(|_| Refusal("The packed world went missing."))?.len();
        self.file = Some(std::fs::File::open(archive).map_err(|_| Refusal("The packed world went missing."))?);
        self.sender = Some(Sender::new(self.transfer, len).map_err(|_| Refusal("The packed world is too large."))?);
        self.packed = Some(packed);
        self.core = Some(core);
        self.stage = SourceStage::Streaming;
        // CP6 review LOW-4: the recipient's silence counts from here, not
        // from consent (the pack was this machine's own time).
        self.last_heard = now;
        self.fill(hs)
    }

    /// Keep at most FLIGHT_LIMIT unacknowledged 16KiB chunks in flight.
    fn fill(&mut self, hs: &mut HostedServer) -> Result<(), Refusal> {
        let mut out = Vec::new();
        {
            let (Some(sender), Some(file)) = (self.sender.as_mut(), self.file.as_mut()) else { return Ok(()) };
            let mut buf = vec![0u8; CHUNK_LIMIT];
            while sender.allocated_chunk_bytes() < FLIGHT_LIMIT * CHUNK_LIMIT && !sender.complete() {
                let n = file.read(&mut buf).map_err(|_| Refusal("The packed world couldn't be read."))?;
                if n == 0 {
                    break;
                }
                let chunk = sender.enqueue(buf[..n].to_vec()).map_err(|_| Refusal("The transfer window is full."))?;
                out.push(Frame::Chunk { transfer: chunk.transfer, offset: chunk.offset, bytes: chunk.bytes.clone() });
            }
        }
        for f in out {
            self.send(hs, &f)?;
        }
        Ok(())
    }
}

/// Review LOW-7: a session dropped without `abort` still stops its pack
/// worker at the next member (its result is dropped with the receiver).
impl Drop for SourceSession {
    fn drop(&mut self) {
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
    let saved = match &cfg.world_dir {
        Some(dir) => eligibility::saved_record_keys(dir)?,
        None => Vec::new(),
    };
    eligibility::saved_records(&saved, action.source_key, action.seat.key)
}

//! Source session: the host's explicit action, the recipient's verified
//! consent, the frozen capture/pack, the bounded direct transfer and the
//! verified Ready. Everything before retirement is recoverable: `abort`
//! releases the freeze and the original world is exactly as it was.
//! Prepared/Commit/retirement are later boundaries, not reachable here.
use super::eligibility::{self, Refusal, Seat};
use super::wire::{Frame, PART_LIMIT, acceptance_content, source_agreement_content};
use crate::handover::{
    Id,
    certificate::Core,
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
    /// every boundary (`None`: a world never saved holds no records).
    pub world_dir: Option<PathBuf>,
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

    fn send(&mut self, hs: &mut HostedServer, f: &Frame) -> Result<(), Refusal> {
        let bytes = f.encode().map_err(|_| Refusal("A handover message was too large."))?;
        hs.handover_send(&bytes).map_err(Refusal)
    }

    /// Pre-retirement abort: tell the recipient, release the freeze, delete
    /// the scratch archive. The original world was never changed.
    pub fn abort(&mut self, hs: &mut HostedServer, why: &'static str) {
        if matches!(self.stage, SourceStage::Aborted(_)) {
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
            self.last_heard = now;
            if let Err(r) = self.on_frame(hs, live, frame, now) {
                return Err(self.fail(hs, r));
            }
        }
        let quiet = now.saturating_sub(self.last_heard);
        // L2: a silent recipient times out while streaming and after Ready
        // (until Prepared advances it); the pack is this machine's own work.
        let stalled = match self.stage {
            SourceStage::Proposed => quiet > CONSENT_SECS,
            SourceStage::Streaming | SourceStage::ReadyVerified => quiet > STALL_SECS,
            _ => false,
        };
        if stalled {
            return Err(self.fail(hs, Refusal("The other player stopped responding. Play has resumed.")));
        }
        if self.consented_at.is_some_and(|t| now.saturating_sub(t) > OPERATION_SECS) {
            return Err(self.fail(hs, Refusal(super::copy::TOO_LONG_SOURCE)));
        }
        if self.stage == SourceStage::Packing && now.saturating_sub(self.last_beat) >= HEARTBEAT_SECS {
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
        Ok(())
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
            (_, Frame::Abort { .. }) => Err(Refusal("The other player cancelled the handover.")),
            (SourceStage::Proposed, Frame::Decline { reason, .. }) => Err(Refusal(super::copy::host_declined(reason))),
            (SourceStage::Proposed, Frame::Accept { runtime, attestation, acceptance, challenge, .. }) => {
                self.verify_accept(runtime, &attestation, &acceptance, challenge, now)?;
                // Re-authenticated: the same identity, seat and records,
                // still eligible, then freeze.
                self.check(hs, live)?;
                hs.handover_freeze().map_err(Refusal)?;
                self.recipient_runtime = Some(runtime);
                self.recipient_challenge = challenge;
                self.acceptance = Some(acceptance);
                self.consented_at = Some(now);
                self.stage = SourceStage::Frozen;
                Ok(())
            }
            (SourceStage::Streaming, Frame::Ack { transfer, through }) => {
                let sender = self.sender.as_mut().ok_or(Refusal("No transfer is running."))?;
                sender
                    .acknowledge(transfer, through)
                    .map_err(|_| Refusal("The recipient acknowledged data it was never sent."))?;
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
            _ => Err(Refusal("The recipient sent a handover message out of order.")),
        }
    }

    fn verify_accept(&self, runtime: [u8; 32], attestation: &str, acceptance: &str, challenge: Id, now: u64) -> Result<(), Refusal> {
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
        Ok(())
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

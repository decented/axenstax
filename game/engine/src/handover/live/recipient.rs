//! Recipient session. A Propose read from the joined connection is checked
//! against the host this client VERIFIED when it joined (never against what
//! the frame claims) before any consent is shown; the joined `RemoteClient`
//! keeps playing meanwhile, and a Decline goes back over it. On explicit
//! consent the session takes the connection out of the `RemoteClient`
//! (parked, not closed), so leaving the joined world cannot touch it, then
//! verifies the signed offer against its own fresh challenge, imports into
//! a fresh non-overwriting inactive directory, makes it durable, binds a
//! latched listener, and only then persists and sends Ready. Before Ready an
//! abort hands the connection (with any gameplay that arrived meanwhile)
//! back, so play continues. No gameplay admission or advertisement here.
use super::bearer::Bearer;
use super::eligibility::Refusal;
use super::listener::LatchedListener;
use super::wire::{Frame, acceptance_content, prepared_ack_content, prepared_ack_proof, root_lineage, source_agreement_content};
use crate::handover::{
    Id,
    certificate::Core,
    crypto::{self, Verified, VerifiedPrepared},
    stored::Route,
    checkpoint::{AdmittedManifest, DurableCheckpoint, Import, ImportBoundary, ImportObserver, OfferContext},
    context::RootContext,
    durable::MarkerStore,
    phase::{Authority, Receiver},
    stream::Chunk,
};
use crate::transport::ClientTransport;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Timestamp, ToBech32};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

/// M1: what the import worker hands back: its generation, the transfer it
/// imported, and the durable checkpoint (or why not).
type ImportResult = (u64, Id, crate::handover::Result<DurableCheckpoint>);

/// M1: who cleans up a cancelled import. The worker decides under this lock:
/// a worker that sees `cancelled` discards its own inert import; once it has
/// set `finished` (or died) the session discards on abort instead.
#[derive(Default)]
struct ImportFence {
    cancelled: bool,
    finished: bool,
}
/// Stops the import at its next file boundary once cancelled.
struct CancelImport(Arc<Mutex<ImportFence>>);
impl ImportObserver for CancelImport {
    fn reached(&mut self, _: ImportBoundary, _: &Path) -> std::io::Result<()> {
        if self.0.lock().map(|g| g.cancelled).unwrap_or(true) {
            Err(std::io::Error::other("handover import cancelled"))
        } else {
            Ok(())
        }
    }
}

/// The host this client verified when it joined: the persona from its
/// verified answer and the runtime it connected to (online play's
/// invitation `k=` or the contact's attested runtime). A join that verified
/// no host identity cannot receive a handover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinedHost {
    pub persona: [u8; 32],
    pub runtime: [u8; 32],
}

/// What the player is asked to accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub transfer: Id,
    pub world_name: String,
    /// Equal to the npub of the host this client verified (checked).
    pub source_npub: String,
    challenge: Id,
    root: RootContext,
    lineage: Id,
}

/// The player's explicit consent to exactly this proposal. Only the consent
/// caller constructs it; nothing accepts on a timer or by default.
pub struct RecipientConsent {
    pub transfer: Id,
    pub world_name: String,
    pub source_npub: String,
}

pub struct RecipientIdentity {
    pub runtime: Keys,
    pub attestation: Event,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipientStage {
    AwaitingConsent,
    Accepted,
    Receiving,
    /// M1: every byte arrived; the import (extract, sync, verify, rename)
    /// runs on a worker while the frame, Cancel and the heartbeat go on.
    Importing,
    /// Durable checkpoint, latched listener and persisted Ready: inert.
    Ready,
    /// The disclosed bearer verified against the source's Prepared, which
    /// is now durable here (inert marker cleared) and acknowledged. From
    /// here the import is never deleted.
    Prepared,
    Aborted(&'static str),
    /// Ended after Prepared: the import, its markers and the received
    /// world are kept untouched (M-2), never deleted, never opened.
    Quarantined(&'static str),
}

/// Gameplay packets parked while the session holds the connection, handed
/// back to the joined client on a pre-Ready abort. Past this the joined
/// session can't be resumed faithfully and the player rejoins instead.
pub const PARKED_BYTES: usize = 4 * 1024 * 1024;

pub struct RecipientSession {
    link: Option<Box<dyn ClientTransport>>,
    parked: VecDeque<Vec<u8>>,
    parked_bytes: usize,
    parked_overflow: bool,
    stage: RecipientStage,
    identity: RecipientIdentity,
    persona: [u8; 32],
    imports: PathBuf,
    markers: MarkerStore,
    proposal: Proposal,
    challenge: Id,
    acceptance: String,
    socket: Option<std::net::UdpSocket>,
    manifest: Vec<u8>,
    import: Option<Import>,
    received: u64,
    expected: u64,
    core: Option<Core>,
    checkpoint: Option<DurableCheckpoint>,
    listener: Option<LatchedListener>,
    receiver: Option<Receiver>,
    /// L2: when the host was last heard from (consent onwards).
    last_heard: u64,
    /// M1: the import worker's fence, result channel and thread.
    import_generation: u64,
    import_fence: Option<Arc<Mutex<ImportFence>>>,
    import_rx: Option<mpsc::Receiver<ImportResult>>,
    import_worker: Option<std::thread::JoinHandle<()>>,
    last_beat: u64,
    #[cfg(test)]
    import_hold: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    /// P4: when this player consented; the whole operation must reach
    /// Activated within `source::OPERATION_SECS` of it.
    consented_at: u64,
    /// L5: the joined session's router lease (UPnP), moved in at consent so
    /// leaving the joined world can't release a mapping this link may use.
    lease: Option<Box<dyn HeldLease>>,
    /// P2: the bearer the source disclosed after Ready (held only here and,
    /// once activated, in the world's Activated record).
    bearer: Option<Bearer>,
    /// The verified Prepared (exact event) this machine persisted.
    prepared: Option<(Event, VerifiedPrepared)>,
}

/// L5: a router mapping the joined session asked for (native: the UPnP
/// `LeaseKeeper`). The session renews it while it holds the link and gives
/// it back to the router when the session ends without returning it.
pub trait HeldLease {
    fn renew(&mut self);
    fn give_back(self: Box<Self>);
    /// Review LOW-7: at process exit a detached release would not outlive
    /// `main`; give it back within `grace` on this thread instead.
    fn give_back_within(self: Box<Self>, grace: std::time::Duration);
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
}

impl Drop for RecipientSession {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            lease.give_back();
        }
    }
}

fn npub(key: [u8; 32]) -> String {
    PublicKey::from_slice(&key).ok().and_then(|k| k.to_bech32().ok()).unwrap_or_default()
}

/// The Handover payload of a packet (`None`: some other packet).
pub fn handover_payload(packet: &[u8]) -> Option<&[u8]> {
    match crate::protocol::deserialize_header(packet) {
        Some((crate::protocol::PacketType::Handover, bytes)) => Some(bytes),
        _ => None,
    }
}

fn packet(f: &Frame) -> Option<Vec<u8>> {
    let bytes = f.encode().ok()?;
    let mut p = vec![crate::protocol::PacketType::Handover as u8];
    p.extend_from_slice(&bytes);
    Some(p)
}

impl RecipientSession {
    /// A Propose read from the joined connection. Checked before any consent
    /// is shown: the invitation it names must be the host this client
    /// verified (runtime and persona), the npub it shows must be that
    /// persona's, and its lineage the one that invitation derives.
    pub fn propose(
        payload: &[u8],
        host: JoinedHost,
        identity: RecipientIdentity,
        imports: PathBuf,
        markers: MarkerStore,
        now: u64,
    ) -> Result<Self, Refusal> {
        let Ok(Frame::Propose {
            transfer, world_name, source_npub, challenge, root_runtime, root_persona, root_expiry, root_relays, lineage,
        }) = Frame::decode(payload)
        else {
            return Err(Refusal("The host's handover proposal was malformed."));
        };
        let root = RootContext::new(root_runtime, &root_persona, root_expiry, &root_relays, now)
            .map_err(|_| Refusal("The host's invitation is not valid."))?;
        if challenge == [0; 16] || transfer == [0; 16] {
            return Err(Refusal("The host's proposal was malformed."));
        }
        if root.runtime() != host.runtime || root.persona() != host.persona || source_npub != npub(host.persona) {
            return Err(Refusal("This handover doesn't come from the host you joined."));
        }
        if lineage != root_lineage(&root) {
            return Err(Refusal("The host's proposal names the wrong world history."));
        }
        let persona = crate::runtime_identity::verify_player_attestation(
            &identity.attestation,
            &identity.runtime.public_key(),
            Timestamp::from(now),
        )
        .map_err(|_| Refusal("Your device's sign-in link has expired."))?
        .to_bytes();
        Ok(Self {
            link: None,
            parked: VecDeque::new(),
            parked_bytes: 0,
            parked_overflow: false,
            stage: RecipientStage::AwaitingConsent,
            identity,
            persona,
            imports,
            markers,
            proposal: Proposal { transfer, world_name, source_npub, challenge, root, lineage },
            challenge: [0; 16],
            acceptance: String::new(),
            socket: None,
            manifest: Vec::new(),
            import: None,
            received: 0,
            expected: 0,
            core: None,
            checkpoint: None,
            listener: None,
            receiver: None,
            last_heard: now,
            import_generation: 0,
            import_fence: None,
            import_rx: None,
            import_worker: None,
            last_beat: now,
            #[cfg(test)]
            import_hold: None,
            consented_at: now,
            lease: None,
            bearer: None,
            prepared: None,
        })
    }

    pub fn stage(&self) -> RecipientStage {
        self.stage
    }
    pub fn proposal(&self) -> &Proposal {
        &self.proposal
    }
    pub fn checkpoint(&self) -> Option<&DurableCheckpoint> {
        self.checkpoint.as_ref()
    }
    pub fn listener(&self) -> Option<&LatchedListener> {
        self.listener.as_ref()
    }
    /// No gameplay before Activated, a later boundary.
    pub fn gameplay_admitted(&self) -> bool {
        self.receiver.as_ref().is_some_and(|r| r.gameplay_admitted())
    }

    fn send(&self, f: &Frame) {
        if let (Some(link), Some(p)) = (&self.link, packet(f)) {
            link.send_to_server(&p);
        }
    }
    fn transfer(&self) -> Id {
        self.proposal.transfer
    }

    /// P3: before Prepared the import is inert and this session's own: an
    /// abort drops the import (its Drop removes pre-rename scratch) and
    /// discards exactly this transfer's marked import, its Ready marker and
    /// the inert marker (`inert::discard`). From Prepared on nothing is
    /// deleted (quarantine); Activated is never touched. The connection
    /// stays with the session for `return_link`.
    pub fn abort(&mut self, why: &'static str) {
        if matches!(self.stage, RecipientStage::Aborted(_) | RecipientStage::Quarantined(_)) {
            return;
        }
        self.send(&Frame::Abort { transfer: self.transfer() });
        self.import = None;
        self.listener = None;
        self.socket = None;
        // M1: fence off a running import worker; if it hasn't finished it
        // discards its own import when it sees the cancel.
        self.import_generation += 1;
        self.import_rx = None;
        let worker_cleans = self.import_fence.take().is_some_and(|f| {
            let mut g = f.lock().unwrap_or_else(|e| e.into_inner());
            g.cancelled = true;
            !g.finished
        });
        let inert = self.receiver.as_ref().is_none_or(|r| r.stage() == crate::handover::phase::ReceiverStage::Ready);
        if inert && !worker_cleans {
            self.checkpoint = None;
            self.receiver = None;
            // An unmarked import is never touched; a failed discard is left
            // for the next sweep (the marker still names it).
            let _ = super::inert::discard(&self.imports, &self.markers, self.transfer());
        }
        // M-2: after Prepared nothing is deleted: the session ends quarantined.
        self.stage = if inert { RecipientStage::Aborted(why) } else { RecipientStage::Quarantined(why) };
    }
    fn fail(&mut self, r: Refusal) -> Refusal {
        self.abort(r.0);
        r
    }

    /// After an abort: the connection and the gameplay that arrived while
    /// the session held it, oldest first, for the joined client to resume
    /// with. `None` if gameplay overflowed the parked bound (the player
    /// rejoins instead) or the connection already closed.
    /// L5: hold the joined session's router lease for as long as this
    /// session holds the link.
    pub fn hold_lease(&mut self, lease: Box<dyn HeldLease>) {
        self.lease = Some(lease);
    }
    pub fn holds_lease(&self) -> bool {
        self.lease.is_some()
    }
    /// Review LOW-7: at exit, give the held lease back within `grace`.
    pub fn release_lease_within(&mut self, grace: std::time::Duration) {
        if let Some(lease) = self.lease.take() {
            lease.give_back_within(grace);
        }
    }
    /// Test hook: send a control frame over the held link.
    #[cfg(test)]
    pub fn test_send(&self, f: &Frame) {
        self.send(f);
    }
    /// The lease back, for a joined session that gets its link back.
    pub fn take_lease(&mut self) -> Option<Box<dyn HeldLease>> {
        self.lease.take()
    }

    pub fn return_link(&mut self) -> Option<(Box<dyn ClientTransport>, VecDeque<Vec<u8>>)> {
        if !matches!(self.stage, RecipientStage::Aborted(_) | RecipientStage::Quarantined(_)) {
            return None;
        }
        let link = self.link.take()?;
        if self.parked_overflow || link.is_closed() {
            return None;
        }
        self.parked_bytes = 0;
        Some((link, std::mem::take(&mut self.parked)))
    }

    /// Explicit consent. `link` is the joined connection, taken out of the
    /// `RemoteClient` now; `listen` is the socket the latched listener binds.
    pub fn consent(
        &mut self,
        c: RecipientConsent,
        listen: std::net::UdpSocket,
        link: Box<dyn ClientTransport>,
        now: u64,
    ) -> Result<(), Refusal> {
        let p = &self.proposal;
        if self.stage != RecipientStage::AwaitingConsent
            || c.transfer != p.transfer
            || c.world_name != p.world_name
            || c.source_npub != p.source_npub
        {
            return Err(Refusal("That isn't the handover you were offered."));
        }
        let content = acceptance_content(p.transfer, p.challenge, &p.world_name, &p.source_npub);
        let acceptance = pollster::block_on(
            EventBuilder::new(Kind::Custom(crate::handover::STORED_KIND), content)
                .custom_created_at(Timestamp::from(now))
                .sign(&self.identity.runtime),
        )
        .map_err(|_| Refusal("Your consent couldn't be signed."))?
        .as_json();
        getrandom::fill(&mut self.challenge).map_err(|_| Refusal("No randomness."))?;
        let accept = Frame::Accept {
            transfer: p.transfer,
            runtime: self.identity.runtime.public_key().to_bytes(),
            attestation: self.identity.attestation.as_json(),
            acceptance: acceptance.clone(),
            challenge: self.challenge,
        };
        self.acceptance = acceptance;
        self.last_heard = now;
        self.consented_at = now;
        self.socket = Some(listen);
        self.link = Some(link);
        self.send(&accept);
        self.stage = RecipientStage::Accepted;
        Ok(())
    }

    /// Decline while still joined: the packet the caller sends over its
    /// joined connection. Only an open proposal can be declined; anything
    /// later is an abort, with its cleanup.
    pub fn decline(&mut self) -> Option<Vec<u8>> {
        if self.stage != RecipientStage::AwaitingConsent {
            self.abort("cancelled");
            return None;
        }
        self.stage = RecipientStage::Aborted("declined");
        packet(&Frame::Decline { transfer: self.transfer(), reason: super::wire::DeclineReason::Player })
    }

    pub fn pump(&mut self, now: u64) -> Result<(), Refusal> {
        if matches!(self.stage, RecipientStage::Aborted(_) | RecipientStage::Quarantined(_) | RecipientStage::AwaitingConsent) {
            return Ok(());
        }
        if let Some(lease) = self.lease.as_mut() {
            lease.renew();
        }
        match self.take_import() {
            Ok(Some(r)) => {
                if let Err(r) = r.map_err(|_| Refusal("The world couldn't be stored safely on this device.")).and_then(|c| self.finish(c, now)) {
                    return Err(self.fail(r));
                }
            }
            Ok(None) => {}
            Err(r) => return Err(self.fail(r)),
        }
        let Some(link) = &self.link else { return Ok(()) };
        if link.is_closed() {
            return Err(self.fail(Refusal("The connection to the host closed.")));
        }
        self.read_link(now)?;
        if matches!(self.stage, RecipientStage::Aborted(_) | RecipientStage::Quarantined(_)) {
            return Ok(());
        }
        // L2: mirrors the source's `STALL_SECS` in every stage that waits on
        // the host (it sends a heartbeat while it packs). Checked after the
        // link is read, so a frame already waiting is never ignored.
        if self.stage != RecipientStage::Importing && now.saturating_sub(self.last_heard) > super::source::STALL_SECS {
            return Err(self.fail(Refusal("The host stopped responding, so the handover was cancelled.")));
        }
        // P4: heartbeats keep the stall deadlines fed, so one total deadline
        // from consent to Activated bounds the whole operation.
        if !self.gameplay_admitted() && now.saturating_sub(self.consented_at) > super::source::OPERATION_SECS {
            return Err(self.fail(Refusal(super::copy::TOO_LONG)));
        }
        // M1: the import runs on a worker; the source's stall clock hears this.
        if self.stage == RecipientStage::Importing && now.saturating_sub(self.last_beat) >= super::source::HEARTBEAT_SECS {
            self.last_beat = now;
            self.send(&Frame::Notice { transfer: self.transfer(), paused: true });
        }
        Ok(())
    }

    fn read_link(&mut self, now: u64) -> Result<(), Refusal> {
        while let Some(pkt) = self.link.as_ref().and_then(|l| l.try_recv_from_server()) {
            self.last_heard = now;
            let Some(bytes) = handover_payload(&pkt) else {
                // Gameplay sent before the host froze: kept for a resume.
                self.parked_bytes += pkt.len();
                if self.parked_bytes > PARKED_BYTES {
                    self.parked_overflow = true;
                    self.parked.clear();
                } else if !self.parked_overflow {
                    self.parked.push_back(pkt);
                }
                continue;
            };
            let decoded = Frame::decode(bytes);
            // CP7 review LOW-1: the packet buffer we own that carried the
            // bearer is wiped (the decoded `Bearer` wipes itself on drop).
            let mut pkt = pkt;
            if matches!(decoded, Ok(Frame::Bearer { .. })) {
                zeroize::Zeroize::zeroize(&mut pkt);
            }
            let r = decoded
                .map_err(|_| Refusal("The host sent a malformed handover message."))
                .and_then(|f| self.on_frame(f, now));
            if let Err(r) = r {
                return Err(self.fail(r));
            }
            if matches!(self.stage, RecipientStage::Aborted(_) | RecipientStage::Quarantined(_)) {
                break;
            }
        }
        Ok(())
    }

    fn on_frame(&mut self, f: Frame, now: u64) -> Result<(), Refusal> {
        if f.transfer() != self.transfer() {
            return Err(Refusal("The host mixed up two handovers."));
        }
        match (self.stage, f) {
            (_, Frame::Abort { .. }) => Err(Refusal("The host cancelled the handover.")),
            (_, Frame::Notice { .. }) => Ok(()),
            (RecipientStage::Accepted, Frame::ManifestPart { total, offset, bytes, .. }) => {
                if total as usize > crate::handover::manifest::MANIFEST_LIMIT
                    || offset as usize != self.manifest.len()
                    || self.manifest.len() + bytes.len() > total as usize
                {
                    return Err(Refusal("The host's manifest was malformed."));
                }
                self.manifest.extend_from_slice(&bytes);
                Ok(())
            }
            (RecipientStage::Accepted, Frame::Offer { core, offer, agreement, .. }) => {
                self.on_offer(&core, offer.as_bytes(), &agreement, now)
            }
            (RecipientStage::Receiving, Frame::Chunk { transfer, offset, bytes }) => {
                let import = self.import.as_mut().ok_or(Refusal("No import is running."))?;
                let len = bytes.len() as u64;
                if import
                    .accept(&Chunk { transfer, offset, bytes }, now)
                    .map_err(|_| Refusal("The world data didn't check out."))?
                {
                    self.received += len;
                }
                self.send(&Frame::Ack { transfer, through: self.received });
                if self.received == self.expected {
                    self.start_import(now)?;
                }
                Ok(())
            }
            (RecipientStage::Ready, Frame::Bearer { bearer, .. }) if self.bearer.is_none() => {
                self.bearer = Some(bearer);
                Ok(())
            }
            (RecipientStage::Ready, Frame::Prepared { event, .. }) => self.on_prepared(&event, now),
            _ => Err(Refusal("The host sent a handover message out of order.")),
        }
    }

    /// P2 + M-2: the disclosed bearer must derive the key the source's
    /// Prepared is sealed to (its `p` route, then decryption and the exact
    /// core). Then Prepared is persisted, the inert marker cleared, and only
    /// then the signed acknowledgement goes back. A wrong bearer stops here
    /// with a typed reason, while the import is still inert.
    fn on_prepared(&mut self, event: &str, now: u64) -> Result<(), Refusal> {
        let wrong = Refusal(super::copy::WRONG_BEARER);
        let p = &self.proposal;
        let core = self.core.as_ref().ok_or(Refusal("No certificate."))?;
        let bearer = self.bearer.as_ref().ok_or(Refusal("The host sent its handover record before its invitation."))?;
        let keys = crypto::recipient(&p.root, bearer.expose(), None).map_err(|_| wrong.clone())?;
        let ev = Event::from_json(event).map_err(|_| Refusal("The host's handover record was malformed."))?;
        let route = Route { author: core.source_runtime, recipient: keys.public_key().to_bytes(), until: p.root.expiry() };
        if !route.matches(&ev) {
            return Err(wrong);
        }
        let Ok(Verified::Prepared(vp)) = crypto::open(&ev, &keys, core.source_runtime, &p.root, now, None) else {
            return Err(wrong);
        };
        if vp.core() != core {
            return Err(Refusal("The host's handover record names a different checkpoint."));
        }
        let (transfer, runtime) = (p.transfer, &self.identity.runtime);
        let content = prepared_ack_content(transfer, vp.event_hash(), prepared_ack_proof(&p.root, bearer.expose(), vp.event_hash()));
        let ack = pollster::block_on(
            EventBuilder::new(Kind::Custom(crate::handover::STORED_KIND), content)
                .custom_created_at(Timestamp::from(now))
                .sign(runtime),
        )
        .map_err(|_| Refusal("Your acknowledgement couldn't be signed."))?
        .as_json();
        let receiver = self.receiver.as_mut().ok_or(Refusal("No Ready."))?;
        receiver
            .prepared(&self.markers, &vp)
            .map_err(|_| Refusal("The handover record couldn't be stored safely on this device."))?;
        // From here the import is never deleted (a Prepared marker refuses
        // every inert discard); a failed clear quarantines it.
        super::inert::clear(&self.imports, transfer).map_err(|_| Refusal("The handover record couldn't be stored safely on this device."))?;
        self.prepared = Some((ev, vp));
        self.stage = RecipientStage::Prepared;
        self.send(&Frame::PreparedAck { transfer, event: ack });
        Ok(())
    }

    fn on_offer(&mut self, core: &[u8], offer: &[u8], agreement: &str, now: u64) -> Result<(), Refusal> {
        let p = &self.proposal;
        let core = Core::decode(core).map_err(|_| Refusal("The host's certificate was malformed."))?;
        // First hop only at this base: the invitation's own runtime and
        // persona (already checked against the host this client verified)
        // offer. Later hops need the verified chain (not wired yet).
        if core.epoch != 1 || core.source_runtime != p.root.runtime() || core.source_persona != p.root.persona() {
            return Err(Refusal("This handover needs a verified invitation chain, which this version can't follow yet."));
        }
        let authority = Authority::root(&p.root, p.lineage).map_err(|_| Refusal("The world's lineage is missing."))?;
        let me = self.identity.runtime.public_key().to_bytes();
        let admitted = AdmittedManifest::verify(
            &self.manifest,
            offer,
            OfferContext {
                core: core.clone(),
                root: &p.root,
                authority: &authority,
                challenge: self.challenge,
                recipient_runtime: me,
                recipient_persona: self.persona,
                now,
            },
        )
        .map_err(|_| Refusal("The host's checkpoint offer didn't verify."))?;
        // Both agreements the manifest binds: the source's, signed by the
        // runtime this certificate names, and this player's own acceptance.
        let bad = Refusal("The host's agreement didn't verify.");
        let ev = Event::from_json(agreement).map_err(|_| bad.clone())?;
        let expected = source_agreement_content(p.transfer, p.challenge, &p.world_name, &npub(self.persona));
        if ev.pubkey.to_bytes() != core.source_runtime
            || ev.verify().is_err()
            || ev.kind != Kind::Custom(crate::handover::STORED_KIND)
            || ev.content != expected
            || !ev.tags.is_empty()
        {
            return Err(bad);
        }
        let mut want = vec![
            (core.source_persona, crate::handover::hash(agreement.as_bytes())),
            (self.persona, crate::handover::hash(self.acceptance.as_bytes())),
        ];
        want.sort();
        let got: Vec<_> = admitted.manifest().agreements.iter().map(|a| (a.participant, a.proof)).collect();
        if got != want {
            return Err(Refusal("The world's agreements don't match what you and the host agreed."));
        }
        self.expected = admitted.manifest().stream_length;
        // P3: marked inert BEFORE any of the import is written.
        super::inert::mark(&self.imports, p.transfer, admitted.manifest().world)
            .map_err(|_| Refusal("The world couldn't be stored on this device."))?;
        self.import = Some(Import::new(admitted, &self.imports, now).map_err(|_| Refusal("The world couldn't be stored on this device."))?);
        self.core = Some(core);
        self.manifest = Vec::new();
        self.stage = RecipientStage::Receiving;
        Ok(())
    }

    /// M1: every byte arrived. The import (stream finish, extract, tree
    /// sync, receipt verify, rename, parent sync) runs on a worker under a
    /// generation fence: the frame, Cancel and the heartbeat keep going, and
    /// a cancelled or superseded import never reaches Ready.
    fn start_import(&mut self, now: u64) -> Result<(), Refusal> {
        let mut import = self.import.take().ok_or(Refusal("No import is running."))?;
        let fence: Arc<Mutex<ImportFence>> = Default::default();
        let (tx, rx) = mpsc::channel();
        let (generation, transfer) = (self.import_generation, self.transfer());
        let (imports, markers) = (self.imports.clone(), self.markers.directory().to_owned());
        let worker_fence = fence.clone();
        #[cfg(test)]
        let hold = self.import_hold.take();
        let worker = std::thread::Builder::new()
            .name("handover-import".into())
            .spawn(move || {
                let r = import.complete(now, &mut CancelImport(worker_fence.clone()));
                // A failed pre-rename import removes its own scratch here.
                drop(import);
                #[cfg(test)]
                if let Some((reached, release)) = hold {
                    let _ = reached.send(());
                    let _ = release.recv();
                }
                let mut g = worker_fence.lock().unwrap_or_else(|e| e.into_inner());
                if g.cancelled {
                    drop(g);
                    if let Ok(m) = MarkerStore::new(markers) {
                        let _ = super::inert::discard(&imports, &m, transfer);
                    }
                    return;
                }
                g.finished = true;
                drop(g);
                let _ = tx.send((generation, transfer, r));
            })
            .map_err(|_| Refusal("The world couldn't be stored on this device."))?;
        self.import_fence = Some(fence);
        self.import_rx = Some(rx);
        self.import_worker = Some(worker);
        self.last_beat = now;
        self.stage = RecipientStage::Importing;
        self.send(&Frame::Notice { transfer, paused: true });
        Ok(())
    }

    /// The import worker's result, if one is here for this generation. A
    /// worker that ended without one (a panic) refuses while it is awaited.
    fn take_import(&mut self) -> Result<Option<crate::handover::Result<DurableCheckpoint>>, Refusal> {
        let Some(rx) = self.import_rx.as_ref() else { return Ok(None) };
        let msg = match rx.try_recv() {
            Ok(m) => m,
            Err(mpsc::TryRecvError::Empty) => return Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.import_rx = None;
                if self.stage != RecipientStage::Importing {
                    return Ok(None);
                }
                // The worker is gone: its files are the session's to discard.
                if let Some(f) = &self.import_fence {
                    f.lock().unwrap_or_else(|e| e.into_inner()).finished = true;
                }
                return Err(Refusal("The world couldn't be stored on this device, so the handover was cancelled."));
            }
        };
        self.import_rx = None;
        let (generation, transfer, result) = msg;
        if generation != self.import_generation || transfer != self.transfer() || self.stage != RecipientStage::Importing {
            return Ok(None);
        }
        Ok(Some(result))
    }

    #[cfg(test)]
    pub fn hold_import(&mut self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (reached_tx, reached_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        self.import_hold = Some((reached_tx, release_rx));
        (reached_rx, release_tx)
    }
    #[cfg(test)]
    pub fn join_import_worker(&mut self) {
        if let Some(w) = self.import_worker.take() {
            w.join().expect("import worker");
        }
    }
    #[cfg(test)]
    pub fn test_lose_import_worker(&mut self) {
        let (tx, rx) = mpsc::channel::<ImportResult>();
        drop(tx);
        self.import_rx = Some(rx);
    }

    /// Durable checkpoint, then the latched listener, then the persisted
    /// Ready receipt, and only then Ready is sent.
    fn finish(&mut self, checkpoint: DurableCheckpoint, now: u64) -> Result<(), Refusal> {
        self.import_fence = None;
        // Waiting on the host starts again now (the import was this side's).
        self.last_heard = now;
        self.checkpoint = Some(checkpoint);
        let socket = self.socket.take().ok_or(Refusal("No listening socket."))?;
        self.listener = Some(LatchedListener::bind(socket).map_err(|_| Refusal("This device couldn't open its listener."))?);
        let p = &self.proposal;
        let core = self.core.as_ref().ok_or(Refusal("No certificate."))?;
        let ready = pollster::block_on(crate::handover::crypto::sign_ready(&self.identity.runtime, core, p.challenge))
            .map_err(|_| Refusal("Ready couldn't be signed."))?;
        let receiver = Receiver::checkpoint_ready(&self.markers, core, &ready, p.challenge, &p.root, now)
            .map_err(|_| Refusal("Ready couldn't be recorded safely."))?;
        self.receiver = Some(receiver);
        self.stage = RecipientStage::Ready;
        self.send(&Frame::Ready { transfer: p.transfer, event: ready.as_json() });
        Ok(())
    }
}

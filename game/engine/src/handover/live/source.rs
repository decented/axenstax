//! Source session: the host's explicit action, the recipient's verified
//! consent, the frozen capture/pack, the bounded direct transfer and the
//! verified Ready. Everything before retirement is recoverable: `abort`
//! releases the freeze and the original world is exactly as it was.
//! Prepared/Commit/retirement are later boundaries, not reachable here.
use super::eligibility::{self, Refusal, Seat};
use super::wire::{Frame, PART_LIMIT, acceptance_content};
use crate::handover::{
    Id,
    certificate::Core,
    checkpoint::{self, CheckpointSource, PackBinding, PackedCheckpoint, ParticipantMember},
    context::RootContext,
    manifest::{Agreement, MemberKind},
    personal::RecordContext,
    phase::Authority,
    stream::{CHUNK_LIMIT, FLIGHT_LIMIT, Sender},
};
use crate::hosted_server::HostedServer;
use nostr::{Event, JsonUtil, Keys, PublicKey, Timestamp, ToBech32};
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStage {
    Proposed,
    /// Consent verified and the source frozen: the caller captures now.
    Frozen,
    Streaming,
    /// Recipient durably holds the checkpoint; inert until Prepared/Commit.
    ReadyVerified,
    Aborted(&'static str),
}

/// What the frozen caller supplies: the sanitised transfer copies and the
/// source's own records (`capture::local0_*`).
pub struct Capture<'a> {
    pub meta: &'a crate::save::WorldMeta,
    pub save: &'a crate::save::WorldSave,
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
    pub fn begin(
        hs: &mut HostedServer,
        action: HostAction,
        cfg: SourceConfig,
        signed_in: Option<[u8; 32]>,
        local_seats: usize,
        saved: &[[u8; 32]],
    ) -> Result<Self, Refusal> {
        eligibility::local_seats(local_seats)?;
        eligibility::source_identity(action.source_key, signed_in)?;
        if cfg.authority.runtime() != cfg.runtime.public_key().to_bytes() {
            return Err(Refusal("This machine doesn't hold this world's invitation."));
        }
        eligibility::seats(&hs.server, action.seat)?;
        eligibility::saved_records(saved, action.source_key, action.seat.key)?;
        let s = action.seat;
        hs.handover_bind(s.slot, s.generation, s.key).map_err(Refusal)?;
        let session = Self {
            stage: SourceStage::Proposed,
            transfer: random_id(),
            challenge: random_id(),
            recipient_runtime: None,
            recipient_challenge: [0; 16],
            acceptance: None,
            packed: None,
            core: None,
            file: None,
            sender: None,
            ready: None,
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
        let mut session = session;
        if let Err(r) = session.send(hs, &propose) {
            hs.handover_release();
            return Err(r);
        }
        Ok(session)
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
    pub fn pump(&mut self, hs: &mut HostedServer, now: u64) -> Result<(), Refusal> {
        if matches!(self.stage, SourceStage::Aborted(_) | SourceStage::ReadyVerified) {
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
            if let Err(r) = self.on_frame(hs, frame, now) {
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

    fn on_frame(&mut self, hs: &mut HostedServer, frame: Frame, now: u64) -> Result<(), Refusal> {
        match (self.stage, frame) {
            (_, Frame::Abort { .. }) | (SourceStage::Proposed, Frame::Decline { .. }) => {
                Err(Refusal("The other player cancelled the handover."))
            }
            (SourceStage::Proposed, Frame::Accept { runtime, attestation, acceptance, challenge, .. }) => {
                self.verify_accept(runtime, &attestation, &acceptance, challenge, now)?;
                // Re-authenticated: the same seat, still eligible, then freeze.
                eligibility::seats(&hs.server, self.action.seat)?;
                hs.handover_freeze().map_err(Refusal)?;
                self.recipient_runtime = Some(runtime);
                self.recipient_challenge = challenge;
                self.acceptance = Some(acceptance);
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
    pub fn offer(&mut self, hs: &mut HostedServer, capture: Capture<'_>, now: u64) -> Result<(), Refusal> {
        if self.stage != SourceStage::Frozen || !hs.handover_frozen() {
            return Err(Refusal("The world isn't frozen for a handover."));
        }
        let r = self.offer_inner(hs, capture, now);
        r.map_err(|r| self.fail(hs, r))
    }

    fn offer_inner(&mut self, hs: &mut HostedServer, c: Capture<'_>, now: u64) -> Result<(), Refusal> {
        hs.handover_check().map_err(Refusal)?;
        eligibility::seats(&hs.server, self.action.seat)?;
        let seat = self.action.seat;
        let recipient_sidecar = crate::joiner_save::JoinerSave::capture(&hs.server.players[seat.slot], hs.server.run_id)
            .encode()
            .map_err(|_| Refusal("The other player's inventory couldn't be recorded."))?;
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
        // Source consent is its host action (authenticated by the offer's
        // runtime signature over this manifest); the recipient's is its
        // runtime-signed acceptance, whose hash the manifest binds.
        let mut agreements = vec![
            Agreement {
                participant: self.action.source_key,
                proof: crate::handover::hash(
                    acceptance_content(self.transfer, self.challenge, &self.action.world_name, &dst_npub).as_bytes(),
                ),
            },
            Agreement {
                participant: seat.key,
                proof: crate::handover::hash(self.acceptance.as_deref().unwrap_or_default().as_bytes()),
            },
        ];
        agreements.sort_by_key(|a| a.participant);
        let packed = checkpoint::pack(
            CheckpointSource {
                meta: c.meta,
                save: c.save,
                world: c.world,
                disk_chunks: c.disk_chunks,
                exhibit_dir: c.exhibit_dir,
                participants: &rows,
            },
            PackBinding {
                source: self.action.source_key,
                receiver: seat.key,
                world: self.cfg.world,
                lineage: self.cfg.lineage,
                transfer: self.transfer,
                generation: self.cfg.generation,
                agreements,
            },
            &self.cfg.scratch,
        )
        .map_err(|_| Refusal("The world couldn't be packed for the handover (too large, or something in it can't move yet)."))?;
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
        self.send(hs, &Frame::Offer { transfer: self.transfer, core: core.encode(), offer })?;
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

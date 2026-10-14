//! Consent verification, the frozen capture, the pack worker, the
//! certified offer and the bounded transfer window.
use super::*;

impl SourceSession {
    /// The worker's message, if it has sent one. A worker that ended without
    /// sending (it panicked) refuses while the session still waits for it,
    /// so the source never sits frozen in `Packing` (it has no stall deadline).
    pub(super) fn take_pack(&mut self, block: bool) -> Result<Option<PackResult>, Refusal> {
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
    pub(super) fn on_pack(&mut self, hs: &mut HostedServer, live: Live, msg: PackResult, now: u64) -> Result<(), Refusal> {
        let (generation, transfer, result) = msg;
        if let Some(worker) = self.pack_worker.take() {
            worker.join().map_err(|_| Refusal("The copy worker failed. Play has resumed."))?;
        }
        if generation != self.pack_generation || transfer != self.transfer || self.stage != SourceStage::Packing {
            drop(result);
            return Ok(());
        }
        let product = result.map_err(|_| {
            Refusal("The world couldn't be packed for the handover (too large, or something in it can't move yet).")
        })?;
        match product {
            PackProduct::Archive(packed) => self.after_pack(hs, live, packed, now),
            PackProduct::Retained(packed, retained) => {
                self.retained = Some(*retained);
                self.send_offer(hs, live, packed, now)
            }
        }
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
        loop {
            let r = match self.take_pack(true) {
                Ok(Some(msg)) => self.on_pack(hs, live, msg, now),
                Ok(None) => return Ok(()),
                Err(r) => Err(r),
            };
            r.map_err(|r| self.fail(hs, r))?;
            if self.stage != SourceStage::Packing { return Ok(()); }
        }
    }

    /// H2: returns the recipient attestation it verified (Prepared and
    /// Commit seal it as the successor's).
    pub(super) fn verify_accept(&self, runtime: [u8; 32], attestation: &str, acceptance: &str, challenge: Id, now: u64) -> Result<Event, Refusal> {
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

    pub(super) fn offer_inner(&mut self, hs: &mut HostedServer, live: Live, c: Capture<'_>, now: u64) -> Result<(), Refusal> {
        hs.handover_check().map_err(Refusal)?;
        self.check(hs, live)?;
        let seat = self.action.seat;
        let mut save = c.save;
        eligibility::neutralise(&mut save, &npub(self.action.source_key))?;
        let recipient_record = crate::joiner_save::JoinerSave::capture(&hs.server.players[seat.slot], hs.server.run_id);
        eligibility::recipient_record(Some(&recipient_record))?;
        let recipient_sidecar =
            recipient_record.encode_seed().map_err(|_| Refusal("The other player's inventory couldn't be recorded."))?;
        let context = |participant| RecordContext {
            participant,
            lineage: self.cfg.lineage,
            generation: self.cfg.generation,
            transfer: self.transfer,
        };
        let recipient_progress = if let Some(personal) = &recipient_record.personal {
            let record = personal.record().map_err(|_| Refusal("The other player's progress couldn't be recorded."))?;
            crate::handover::personal::PersonalRecord::from_gameplay(context(seat.key), &record).and_then(|record| record.encode()).map_err(|_| Refusal("The other player's progress couldn't be recorded."))?
        } else { crate::handover::live::capture::default_progress(context(seat.key)) };
        super::super::personal_adapter::preflight(c.source_sidecar, c.source_progress, context(self.action.source_key))?;
        super::super::personal_adapter::preflight(&recipient_sidecar, &recipient_progress, context(seat.key))?;
        let (src_npub, dst_npub) = (npub(self.action.source_key), npub(seat.key));
        let rows = [
            ParticipantMember { npub: &src_npub, kind: MemberKind::Sidecar, schema: 1, bytes: c.source_sidecar },
            ParticipantMember { npub: &src_npub, kind: MemberKind::Progression, schema: crate::handover::personal::PersonalRecord::declared_schema(c.source_progress).map_err(|_| Refusal("Your progression record has an unsupported schema. Nothing was changed."))?, bytes: c.source_progress },
            ParticipantMember { npub: &dst_npub, kind: MemberKind::Sidecar, schema: 1, bytes: &recipient_sidecar },
            ParticipantMember { npub: &dst_npub, kind: MemberKind::Progression, schema: crate::handover::personal::SCHEMA, bytes: &recipient_progress },
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
                let r = checkpoint::pack_snapshot(&snapshot, binding, &scratch, limits, &control).map(PackProduct::Archive);
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
    pub(super) fn after_pack(&mut self, hs: &mut HostedServer, live: Live, packed: PackedCheckpoint, now: u64) -> Result<(), Refusal> {
        hs.handover_check().map_err(Refusal)?;
        self.check(hs, live)?;
        let seat = self.action.seat;
        let source_agreement = self.source_agreement.take().ok_or(Refusal("No agreement."))?;
        let manifest = packed.manifest().encode().map_err(|_| Refusal("The world's manifest is too large."))?;
        let epoch = self.cfg.authority.epoch().checked_add(1).ok_or(Refusal("The invitation has no handover hops left."))?;
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
            expires_at: now.saturating_add(self.cfg.validity).min(self.cfg.root.expiry()).min(self.cfg.authority.expiry()),
            remaining_hops: 4u32.saturating_sub(epoch) as u8,
        };
        core.validate(&self.cfg.root, now).map_err(|_| Refusal("The invitation has expired or can't carry another handover."))?;
        let offer = pollster::block_on(checkpoint::sign_offer(&self.cfg.runtime, &core, self.recipient_challenge))
            .map_err(|_| Refusal("The checkpoint offer couldn't be signed."))?
            .as_json();
        let admitted = checkpoint::AdmittedManifest::verify(&manifest, offer.as_bytes(), checkpoint::OfferContext {
            core: core.clone(), root: &self.cfg.root, authority: &self.cfg.authority,
            challenge: self.recipient_challenge, recipient_runtime: core.successor_runtime,
            recipient_persona: core.successor_persona, now,
        }).map_err(|_| Refusal("The retained checkpoint offer didn't verify. Play has resumed."))?;
        let (tx, rx) = std::sync::mpsc::channel();
        let (generation, transfer, copy, cancel) = (self.pack_generation, self.transfer, self.retained_copy, self.pack_cancel.clone());
        let worlds = crate::save::worlds_root();
        let worker = std::thread::Builder::new().name("handover-retained-copy".into()).spawn(move || {
            let result = super::super::retained::prepare(&packed, admitted, copy, worlds, now, cancel)
                .map(|retained| PackProduct::Retained(packed, Box::new(retained)));
            let _ = tx.send((generation, transfer, result));
        }).map_err(|_| Refusal("The retained copy couldn't start. Play has resumed."))?;
        self.core = Some(core);
        self.retained_offer = Some((offer, source_agreement));
        self.pack_rx = Some(rx);
        self.pack_worker = Some(worker);
        self.last_beat = now;
        Ok(())
    }

    /// The distinct source copy is durable before the recipient sees an offer
    /// or can send Ready. The ordinary bounded stream starts only now.
    fn send_offer(&mut self, hs: &mut HostedServer, live: Live, packed: PackedCheckpoint, now: u64) -> Result<(), Refusal> {
        hs.handover_check().map_err(Refusal)?;
        self.check(hs, live)?;
        let core = self.core.clone().ok_or(Refusal("No certificate."))?;
        core.validate(&self.cfg.root, now).map_err(|_| Refusal("The invitation expired while the safe copy was being prepared. Play has resumed."))?;
        let (offer, source_agreement) = self.retained_offer.take().ok_or(Refusal("No retained offer."))?;
        let manifest = packed.manifest().encode().map_err(|_| Refusal("The world's manifest is too large."))?;
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
    pub(super) fn fill(&mut self, hs: &mut HostedServer) -> Result<(), Refusal> {
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

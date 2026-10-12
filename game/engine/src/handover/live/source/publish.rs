//! Disclosure and the stored publication of Prepared: the bearer frame,
//! the fenced publish worker and its witness.
use super::*;

impl SourceSession {
    /// P2 + M-2: seal the inert Prepared to the key the bearer derives,
    /// disclose the bearer (the point of no return for it), then publish and
    /// read Prepared back on a fenced worker.
    pub(super) fn disclose(&mut self, hs: &mut HostedServer, live: Live, now: u64) -> Result<(), Refusal> {
        hs.handover_check().map_err(Refusal)?;
        // CP7 review LOW-3: the bearer's point of no return re-runs every
        // boundary check (identity, seats, authority persona, records).
        self.check(hs, live)?;
        if self.cfg.world_dir.is_none() {
            return Err(Refusal(crate::handover::live::copy::NEVER_SAVED));
        }
        // CP13b (CP13 review MEDIUM-1): room for this handover's records in
        // the install's ledger before the bearer can leave.
        // CP15: two lines per handover still: the Abort line (written with
        // the pending abort record, reused by the abort record) and the
        // Retirement line.
        if !crate::handover::ledger::Ledger::install().has_room(2) {
            return Err(Refusal(crate::handover::live::copy::LEDGER_FULL));
        }
        // CP16 (CP15 review MEDIUM-1): room in this world's `handover/` for
        // every record a disclosed transfer may still need: its pending
        // abort record, a terminal record of EITHER kind (Retired or
        // Aborted) and its "published" record. Three more entries at most
        // (the pending record is removed once a terminal record lands; one
        // spare covers a removal that failed). Without it a 65th abort could
        // never be recorded or promoted, and the recipient would wait for
        // ever. No room, or the folder can't be read: nothing is disclosed.
        {
            use crate::handover::durable::MarkerKind::{Aborted, PendingAbort, Published, Retired};
            let dir = self.cfg.world_dir.as_ref().ok_or(Refusal(crate::handover::live::copy::NEVER_SAVED))?;
            // CP17 (CP15 review LOW-3): the world's handover lock, from
            // before the pending abort record until a terminal record is
            // durable: no other game process on this computer promotes,
            // opens or hands over this world meanwhile. Taken before the
            // room check, so `.live` is counted. Not taken: nothing is
            // disclosed.
            match crate::handover::live::world_lock::acquire(dir) {
                Ok(lock) => self.live_lock = Some(lock),
                Err(crate::handover::live::world_lock::LockError::HeldElsewhere) => {
                    return Err(Refusal(crate::handover::live::copy::HANDOVER_ELSEWHERE));
                }
                Err(crate::handover::live::world_lock::LockError::Io) => {
                    log::warn!("handover: this world's handover lock couldn't be taken; nothing was disclosed");
                    return Err(Refusal(crate::handover::live::copy::PENDING_ABORT_UNWRITTEN));
                }
            }
            let room = crate::handover::live::authority::store(dir).and_then(|s| s.has_room(&[PendingAbort, Retired, Aborted, Published], 3));
            if !matches!(room, Ok(true)) {
                log::warn!("handover: this world's handover records have no room for another handover ({room:?}); nothing was disclosed");
                return Err(Refusal(crate::handover::live::copy::WORLD_RECORDS_FULL));
            }
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
        // CP15 (coordinator 2026-10-11): the Aborted is sealed together with
        // the Prepared, and its pending abort record is durable (ledger line,
        // then file and directory fsync) BEFORE the bearer can leave. A
        // source that dies past this point leaves that record, and the next
        // start-up promotes and publishes it (`phase::promote_pending_aborts`),
        // so a recipient past its ack never waits without end. It can't be
        // written: nothing is disclosed and the handover stops.
        let pending = Seal {
            source: &self.cfg.runtime,
            recipient: &keys,
            core: &core,
            ready: &ready,
            challenge: self.challenge,
            source_attestation: &self.cfg.attestation,
            successor_attestation: &successor,
            prepared: Some(&vp),
        };
        let unwritten = Refusal(crate::handover::live::copy::PENDING_ABORT_UNWRITTEN);
        let ev = pollster::block_on(crypto::seal_aborted(pending, &self.cfg.root, core.issued_at)).map_err(|_| unsealed.clone())?;
        let Ok(Verified::Aborted(va)) = crypto::open(&ev, &keys, core.source_runtime, &self.cfg.root, core.issued_at, Some(&vp)) else {
            return Err(unsealed);
        };
        let record = AbortRecord::new(&va, &vp, ev.as_json().into_bytes(), self.cfg.root.relays().to_vec()).map_err(|_| unsealed.clone())?;
        // CP16 (CP15 review LOW-1): the phase is built BEFORE the pending
        // record is written, so nothing can fail between that write and the
        // disclosure and leave an orphan record for a transfer that never
        // disclosed (the next start-up would publish its Aborted).
        let phase = crate::handover::phase::Source::frozen(core, &self.cfg.authority).map_err(|_| unsealed)?;
        let store = self.cfg.world_dir.as_ref().map(|d| crate::handover::live::authority::store(d));
        let Some(Ok(store)) = store else { return Err(unwritten) };
        if let Err(e) = crate::handover::phase::write_pending_abort(&store, &crate::handover::ledger::Ledger::install(), &record) {
            log::error!("handover: the pending abort record couldn't be written ({e}); nothing was disclosed");
            // Nothing left this machine: a record that did land (its
            // directory fsync failed) is withdrawn so no start-up publishes it.
            let _ = store.remove(crate::handover::durable::MarkerKind::PendingAbort, self.transfer);
            return Err(unwritten);
        }
        self.pending_abort = Some((ev, record));
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
    pub(super) fn spawn_publish(&mut self, event: Event, verified: Verified, permit: Option<PublicationPermit>, recipient: Hash) -> Result<(), Refusal> {
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
    pub(super) fn take_publish(&mut self) -> Result<Option<crate::handover::Result<StoredProof>>, Refusal> {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = self.publish_rx.as_ref() else { return Ok(None) };
        let msg = match rx.try_recv() {
            Ok(m) => m,
            Err(TryRecvError::Empty) => return Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.publish_rx = None;
                return if self.stage == SourceStage::Publishing {
                    Err(Refusal(crate::handover::live::copy::PREPARED_UNSTORED))
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
    pub(super) fn on_stored(&mut self, hs: &mut HostedServer, stored: crate::handover::Result<StoredProof>, now: u64) -> Result<(), Refusal> {
        if self.stage != SourceStage::Publishing {
            return Ok(());
        }
        let stored = stored.map_err(|_| Refusal(crate::handover::live::copy::PREPARED_UNSTORED))?;
        let (event, vp) = self.prepared.as_ref().ok_or(Refusal("No Prepared."))?;
        let event = event.as_json();
        self.phase
            .as_mut()
            .ok_or(Refusal("No Prepared."))?
            .prepared(vp, &stored)
            .map_err(|_| Refusal(crate::handover::live::copy::PREPARED_UNSTORED))?;
        self.send(hs, &Frame::Prepared { transfer: self.transfer, event })?;
        // The recipient's clock for persisting it and answering starts now.
        self.last_heard = now;
        self.stage = SourceStage::Prepared;
        Ok(())
    }
}

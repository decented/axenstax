//! The recipient's verified acknowledgement and the durable retirement
//! with the exact signed Commit bytes.
use super::*;

impl SourceSession {
    /// M-2: the recipient's signed acknowledgement of its durable Prepared,
    /// bound to this transfer, the Prepared event hash and a bearer
    /// possession proof (CP7 review LOW-2). Only then: seal the Commit locally and
    /// durably retire with its exact bytes, BEFORE any of them escape.
    pub(super) fn on_prepared_ack(&mut self, hs: &mut HostedServer, live: Live, ack: &str, now: u64) -> Result<(), Refusal> {
        let bad = Refusal("The recipient's acknowledgement didn't verify. Play has resumed.");
        let (_, vp) = self.prepared.clone().ok_or(Refusal("No Prepared."))?;
        let capability = crypto::root_capability(&self.cfg.root, self.cfg.bearer.expose()).map_err(|_| bad.clone())?;
        let rt = self.recipient_runtime.ok_or(bad.clone())?;
        let ev = Event::from_json(ack).map_err(|_| bad.clone())?;
        // CP8 review LOW-2: a plain (not constant-time) String comparison is
        // not an oracle here: only the consenting recipient, which already
        // holds the bearer, reaches it, after its runtime's signature
        // verified, and the first mismatch aborts the session for good.
        if ev.pubkey.to_bytes() != rt
            || ev.verify().is_err()
            || ev.kind != Kind::Custom(crate::handover::STORED_KIND)
            || ev.content
                != prepared_ack_content(
                    self.transfer,
                    vp.event_hash(),
                    prepared_ack_proof(&self.cfg.root, self.cfg.bearer.expose(), vp.event_hash()),
                )
            || !ev.tags.is_empty()
        {
            return Err(bad);
        }
        // CP7 review LOW-3: retirement re-runs every boundary check first.
        self.check(hs, live)?;
        let r = self.retire(&vp, capability, now);
        // CP7 review HIGH-1: from here the link itself refuses to unfreeze
        // (release and reap do nothing), whatever happens to this session.
        if self.past_retirement() {
            hs.handover_mark_retired();
        }
        r
    }

    pub(super) fn retire(&mut self, vp: &VerifiedPrepared, capability: Hash, now: u64) -> Result<(), Refusal> {
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
        let world_dir = self.cfg.world_dir.as_ref().ok_or(Refusal(crate::handover::live::copy::NEVER_SAVED))?;
        let store = crate::handover::live::authority::store(world_dir).map_err(|_| Refusal(crate::handover::live::copy::AUTHORITY_UNREADABLE))?;
        let record = Retirement::new(
            &vc,
            vp,
            commit.as_json().into_bytes(),
            self.retained_copy,
            capability,
            vec![RetiredRoot { capability, expiry: self.cfg.root.expiry() }],
            self.cfg.root.relays().to_vec(),
        )
        .map_err(|_| unsealed)?;
        let phase = self.phase.as_mut().ok_or(Refusal("No Prepared."))?;
        match phase.retire(&store, &crate::handover::ledger::Ledger::install(), record) {
            Ok(permit) => {
                // CP17: the terminal record is durable; the lock goes.
                self.live_lock = None;
                self.permit = Some(permit);
                self.commit = Some(commit);
                self.stage = SourceStage::Retired;
                Ok(())
            }
            // A write that may have reached the disk: quarantined, frozen.
            Err(_) if phase.stage() == crate::handover::phase::SourceStage::RetirementUncertain => {
                self.stage = SourceStage::RetirementUncertain;
                Err(Refusal(crate::handover::live::copy::RETIREMENT_UNCERTAIN))
            }
            Err(_) => Err(Refusal("This world couldn't be retired safely. Play has resumed.")),
        }
    }
}

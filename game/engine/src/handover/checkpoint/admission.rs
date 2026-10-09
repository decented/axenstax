//! A fresh direct-control offer precedes Ready; no Prepared circularity.
use super::super::{
    Error, Id, Result, certificate::Core, codec::frame, context::RootContext, manifest::Manifest,
    phase::Authority,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Timestamp};
const OFFER_LIMIT: usize = 4096;
fn content(core: &Core, challenge: Id) -> String {
    let mut v = Vec::new();
    frame(&mut v, b"AxeNStax/handover/checkpoint-offer/v1");
    frame(&mut v, &core.encode());
    v.extend_from_slice(&challenge);
    STANDARD.encode(v)
}
pub async fn sign_offer(source: &Keys, core: &Core, challenge: Id) -> Result<Event> {
    if source.public_key().to_bytes() != core.source_runtime || challenge == [0; 16] {
        return Err(Error::Invalid("offer source/challenge"));
    }
    EventBuilder::new(
        Kind::Custom(super::super::STORED_KIND),
        content(core, challenge),
    )
    .custom_created_at(Timestamp::from(core.issued_at))
    .sign(source)
    .await
    .map_err(|_| Error::Invalid("offer signing"))
}
/// Private provenance: a digest, raw Manifest or caller boolean cannot admit an
/// archive. The expected challenge is held by the authenticated recipient session.
pub struct AdmittedManifest {
    manifest: Manifest,
    core: Core,
    evidence: Vec<u8>,
    pub(super) authority: Authority,
}
pub struct OfferContext<'a> {
    pub core: Core,
    pub root: &'a RootContext,
    pub authority: &'a Authority,
    pub challenge: Id,
    pub recipient_runtime: super::super::Hash,
    pub recipient_persona: super::super::Hash,
    pub now: u64,
}
impl AdmittedManifest {
    pub fn verify(bytes: &[u8], offer: &[u8], expected: OfferContext<'_>) -> Result<Self> {
        let OfferContext {
            core,
            root,
            authority,
            challenge,
            recipient_runtime,
            recipient_persona,
            now,
        } = expected;
        if offer.len() > OFFER_LIMIT {
            return Err(Error::Limit("checkpoint offer"));
        }
        core.validate(root, now)?;
        if !authority.validates_source(&core)
            || challenge == [0; 16]
            || recipient_runtime != core.successor_runtime
            || recipient_persona != core.successor_persona
        {
            return Err(Error::Invalid("offer authority/recipient"));
        }
        let event = Event::from_json(offer).map_err(|_| Error::Invalid("checkpoint offer"))?;
        if event.verify().is_err()
            || event.pubkey.to_bytes() != core.source_runtime
            || event.kind != Kind::Custom(super::super::STORED_KIND)
            || !event.tags.is_empty()
            || event.created_at.as_secs() != core.issued_at
            || event.content != content(&core, challenge)
        {
            return Err(Error::Invalid("offer signature/binding"));
        }
        let manifest = Manifest::decode(bytes)?;
        if manifest.digest()? != core.manifest
            || manifest.source != core.source_persona
            || manifest.receiver != core.successor_persona
            || manifest.world != core.snapshot_world
            || manifest.lineage != core.lineage
            || manifest.transfer != core.transfer
            || manifest.generation != core.generation
        {
            return Err(Error::Invalid("offered manifest binding"));
        }
        let evidence = super::evidence::encode(root, &core, offer, challenge, now)?;
        Ok(Self {
            manifest,
            core,
            evidence,
            authority: authority.clone(),
        })
    }
    pub(super) fn evidence(&self) -> &[u8] {
        &self.evidence
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn core(&self) -> &Core {
        &self.core
    }
}

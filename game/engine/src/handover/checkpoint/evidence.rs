//! Offline evidence recovery cannot create a fresh Ready/Activated capability.
//! Successor authority requires a genuinely reconstructed verified chain supplied
//! by the later controller; an unsigned stored prefix is never promoted here.
use super::super::{
    Error, Hash, Id, Result,
    certificate::Core,
    codec::{Reader, frame},
    context::RootContext,
    phase::Authority,
};
use super::{AdmittedManifest, OfferContext, files};
use nostr::ToBech32;
use std::{
    fs,
    path::{Path, PathBuf},
};
const LIMIT: usize = 32 * 1024;
const DOMAIN: &str = "AxeNStax/handover/checkpoint-admission/v1";
pub struct OfflineRecovery<'a> {
    pub expected_context: Hash,
    pub recipient_runtime: Hash,
    pub recipient_persona: Hash,
    /// None supports only the independently signed root-authority first hop.
    /// Later hops fail closed until the controller reconstructs a verified chain.
    pub source_authority: Option<&'a Authority>,
}
/// This exposes file evidence only, never AdmittedManifest or DurableCheckpoint.
/// Expired admission evidence is usable for inspection/local fsync retry, not a
/// new transfer, Ready, publication or activation. Live controller rechecks them.
pub struct RecoveredCheckpoint {
    directory: PathBuf,
    manifest: Hash,
    transfer: Id,
}
impl RecoveredCheckpoint {
    pub fn received_world(&self) -> PathBuf {
        self.directory.join("received")
    }
    pub fn working_world(&self) -> PathBuf {
        self.directory.join("working")
    }
    pub fn manifest_hash(&self) -> Hash {
        self.manifest
    }
    pub fn transfer(&self) -> Id {
        self.transfer
    }
    pub fn recover(
        parent: &Path,
        world: Id,
        transfer: Id,
        expected: OfflineRecovery<'_>,
    ) -> Result<Self> {
        if world == [0; 16] || transfer == [0; 16] {
            return Err(Error::Invalid("offline import id"));
        }
        let directory = parent.join(format!(
            "handover-{}-{}",
            hex::encode(world),
            hex::encode(transfer)
        ));
        if !fs::symlink_metadata(&directory)
            .map_err(|_| Error::Io)?
            .is_dir()
        {
            return Err(Error::Invalid("offline import folder"));
        }
        let manifest = files::read(
            &directory.join("manifest.dat"),
            super::super::manifest::MANIFEST_LIMIT,
        )?;
        let bytes = files::read(&directory.join("admission.dat"), LIMIT)?;
        let a = restore(&bytes, &manifest, expected)?;
        if a.manifest().world != world || a.manifest().transfer != transfer {
            return Err(Error::Invalid("offline import destination"));
        }
        let receipt = super::receipt::verify_receipt(&directory, &a)?;
        // Retry real local durability, but no lifecycle permission is returned.
        super::receipt::durable_tree(&directory, &a, &mut super::import::NoFault)?;
        files::sync_dir(parent)?;
        Ok(Self {
            directory,
            manifest: receipt.manifest_hash(),
            transfer,
        })
    }
}
pub(super) fn encode(
    root: &RootContext,
    core: &Core,
    offer: &[u8],
    challenge: Id,
    admitted_at: u64,
) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    frame(&mut v, DOMAIN.as_bytes());
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&root.runtime());
    v.extend_from_slice(&root.persona());
    v.extend_from_slice(&root.expiry().to_be_bytes());
    v.extend_from_slice(&(root.relays().len() as u32).to_be_bytes());
    for relay in root.relays() {
        frame(&mut v, relay.as_bytes());
    }
    frame(&mut v, &core.encode());
    frame(&mut v, offer);
    v.extend_from_slice(&challenge);
    v.extend_from_slice(&admitted_at.to_be_bytes());
    let digest = super::super::hash(&v);
    v.extend_from_slice(&digest);
    if v.capacity() > LIMIT || v.len() > LIMIT {
        return Err(Error::Limit("admission evidence"));
    }
    Ok(v)
}
fn restore(
    bytes: &[u8],
    manifest: &[u8],
    expected: OfflineRecovery<'_>,
) -> Result<AdmittedManifest> {
    if bytes.len() < 32 || bytes.len() > LIMIT {
        return Err(Error::Invalid("admission evidence"));
    }
    let split = bytes.len() - 32;
    if super::super::hash(&bytes[..split]).as_slice() != &bytes[split..] {
        return Err(Error::Invalid("admission integrity"));
    }
    let mut r = Reader::new(&bytes[..split], LIMIT)?;
    r.label(DOMAIN)?;
    if r.u32()? != 1 {
        return Err(Error::Invalid("admission version"));
    }
    let runtime: Hash = r.array()?;
    let persona: Hash = r.array()?;
    let expiry = r.u64()?;
    let count = r.u32()?;
    if count == 0 || count > 8 {
        return Err(Error::Limit("evidence relay count"));
    }
    let mut relays = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let b = r.framed(2048)?;
        relays.push(
            std::str::from_utf8(b)
                .map_err(|_| Error::Invalid("evidence relay"))?
                .to_owned(),
        );
    }
    let core = Core::decode(r.framed(super::super::certificate::CORE_LIMIT)?)?;
    let offer = r.framed(4096)?;
    let challenge = r.array()?;
    let admitted_at = r.u64()?;
    r.end()?;
    let npub = nostr::PublicKey::from_slice(&persona)
        .map_err(|_| Error::Invalid("evidence persona"))?
        .to_bech32()
        .map_err(|_| Error::Invalid("evidence npub"))?;
    let root = RootContext::new(runtime, &npub, expiry, &relays, admitted_at)?;
    if root.digest() != expected.expected_context {
        return Err(Error::Invalid("offline root context"));
    }
    let root_authority;
    let authority = if let Some(a) = expected.source_authority {
        a
    } else {
        if core.epoch != 1 {
            return Err(Error::State("offline successor chain unavailable"));
        }
        root_authority = Authority::root(&root, core.lineage)?;
        &root_authority
    };
    AdmittedManifest::verify(
        manifest,
        offer,
        OfferContext {
            core,
            root: &root,
            authority,
            challenge,
            recipient_runtime: expected.recipient_runtime,
            recipient_persona: expected.recipient_persona,
            now: admitted_at,
        },
    )
}

pub(super) fn verify_stored(bytes: &[u8], a: &AdmittedManifest) -> Result<()> {
    let saved = restore(
        bytes,
        &a.manifest().encode()?,
        OfflineRecovery {
            expected_context: a.core().context,
            recipient_runtime: a.core().successor_runtime,
            recipient_persona: a.core().successor_persona,
            source_authority: Some(&a.authority),
        },
    )?;
    if saved.core() != a.core() {
        return Err(Error::Invalid("stored admission core"));
    }
    Ok(())
}

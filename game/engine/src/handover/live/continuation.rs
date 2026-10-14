//! Original-invitation routing. One bounded stored operation precedes every
//! capability join; contacts remain a separate explicit admission path.
use super::{bearer::Bearer, wire::root_lineage};
use crate::handover::{
    Error, Result,
    certificate::Prefix,
    context::RootContext,
    crypto::{self, Verified, VerifiedCommit},
    phase::{Authority, Installed},
    stored::{self, Budget, Route},
};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    pub root: RootContext,
    pub bearer: Bearer,
    pub prefix: Prefix,
    latest: Option<VerifiedCommit>,
}
impl Resolved {
    pub(crate) fn admission(&self) -> Result<crate::rendezvous::payload::HandoverContext> {
        let authority = self.authority()?;
        Ok(crate::rendezvous::payload::HandoverContext {
            root: self.root.digest(),
            capability: crypto::root_capability(&self.root, self.bearer.expose())?,
            epoch: authority.epoch(),
            committed: authority.committed(),
        })
    }
    pub(crate) fn authority(&self) -> Result<Authority> {
        match &self.latest {
            Some(proof) => Authority::successor(proof, &self.prefix),
            None => Authority::root(&self.root, root_lineage(&self.root)),
        }
    }
    pub(crate) fn expiry(&self) -> u64 {
        self.latest
            .as_ref()
            .map_or(self.root.expiry(), |p| p.core().expires_at)
    }
    pub(crate) fn next_generation(&self) -> Result<u64> {
        self.latest.as_ref().map_or(Ok(1), |p| {
            p.core()
                .generation
                .checked_add(1)
                .ok_or(Error::Limit("checkpoint generation"))
        })
    }
    /// A later source must possess the exact installed authority, not merely
    /// a cached digest or an online route to a different, later successor.
    pub(crate) fn matches_installation(&self, installed: &Installed) -> Result<()> {
        let latest = self
            .latest
            .as_ref()
            .ok_or(Error::Invalid("missing installed prefix"))?;
        if latest.event_hash() != installed.committed
            || latest.core() != &installed.core
            || self.root != installed.root
            || self.bearer != installed.bearer
        {
            return Err(Error::Invalid("prefix differs from installed authority"));
        }
        Ok(())
    }
}

/// Only digests and public certificate cores are cached, never the bearer.
/// Restored prefixes remain rollback context until every envelope is verified.
pub(crate) async fn cached_lookup(
    root: RootContext,
    bearer: Bearer,
    now: u64,
    cancel: &AtomicBool,
) -> Result<Resolved> {
    use crate::handover::durable::{MarkerKind, MarkerStore};
    let capability = crypto::root_capability(&root, bearer.expose())?;
    let id = capability[..16]
        .try_into()
        .map_err(|_| Error::Invalid("capability cache key"))?;
    let store = MarkerStore::new(crate::data_dir::profile_dir().join("handover-routes"))?;
    let known = store
        .read(MarkerKind::Prefix, id)?
        .map(|bytes| Prefix::restore(&bytes))
        .transpose()?;
    let resolved = lookup(root, bearer, known.as_ref(), now, cancel).await?;
    // Do not hold a filesystem lock across relay I/O. Re-read the latest
    // cache under one cross-process lock before its monotonic replacement.
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let lock = options
        .open(store.directory().join(".prefix.lock"))
        .map_err(|_| Error::Io)?;
    if !lock.metadata().map_err(|_| Error::Io)?.is_file() {
        return Err(Error::Invalid("prefix lock file"));
    }
    lock.try_lock().map_err(|_| Error::Io)?;
    if let Some(bytes) = store.read(MarkerKind::Prefix, id)? {
        Prefix::restore(&bytes)?.confirmed_by(&resolved.prefix)?;
    }
    resolved.prefix.persist(&store, id)?;
    Ok(resolved)
}

/// Check only this world's source aborts against its currently installed
/// predecessor. The install-local ledger authenticates the exact plaintext
/// binding; a copied or corrupt abort can never become a routing authority.
fn check_capability_available(
    store: &crate::handover::durable::MarkerStore,
    installed: &Installed,
    ledger: &crate::handover::ledger::Ledger,
) -> Result<()> {
    use crate::handover::{
        ledger::{Entry, RecordKind, Role},
        phase::AbortRecord,
    };
    for (id, bytes) in store.aborts()?.into_iter().chain(store.pending_aborts()?) {
        let abort = AbortRecord::decode(&bytes)?;
        let core = abort.core();
        if core.transfer != id {
            return Err(Error::Invalid("abort filename binding"));
        }
        if core.context == installed.root.digest()
            && core.lineage == installed.core.lineage
            && core.epoch
                == installed
                    .core
                    .epoch
                    .checked_add(1)
                    .ok_or(Error::Limit("authority epoch"))?
            && core.predecessor == installed.committed
            && core.source_runtime == installed.core.successor_runtime
            && core.source_persona == installed.core.successor_persona
        {
            if !ledger.lists(&Entry::of(id, Role::Source, RecordKind::Abort, &bytes)) {
                return Err(Error::Invalid("unowned current capability abort"));
            }
            return Err(Error::State(
                "original capability is unavailable after disclosed abort",
            ));
        }
    }
    Ok(())
}

/// Re-read the exact selected folder at the last pre-publication boundary.
/// An asynchronous preparation never authorises a deleted/replaced world.
pub(crate) fn check_host_start(
    world: &std::path::Path,
    prepared: Option<&Resolved>,
    runtime: [u8; 32],
    persona: [u8; 32],
    now: u64,
) -> Result<()> {
    let proof = super::personal_adapter::proof(world)
        .map_err(|_| Error::Invalid("current host world proof"))?;
    match (proof, prepared) {
        (None, None) => Ok(()),
        (Some(proof), None) if proof.is_retained() => {
            if proof.local_owner() != persona {
                return Err(Error::Invalid("retained host persona"));
            }
            Ok(())
        }
        (Some(proof), Some(resolved)) if !proof.is_retained() => {
            let store =
                crate::handover::durable::MarkerStore::open_existing(super::authority::dir(world))?;
            let installed = Installed::read(&store, proof.core.transfer)?
                .ok_or(Error::Invalid("missing current installed receipt"))?;
            resolved.matches_installation(&installed)?;
            check_capability_available(
                &store,
                &installed,
                &crate::handover::ledger::Ledger::install(),
            )?;
            let authority = resolved.authority()?;
            if authority.runtime() != runtime
                || authority.persona() != persona
                || now > resolved.expiry()
            {
                return Err(Error::Invalid("current host authority differs"));
            }
            Ok(())
        }
        _ => Err(Error::Invalid(
            "host preparation differs from current folder authority",
        )),
    }
}

pub(crate) async fn installed_world(
    world: &std::path::Path,
    now: u64,
    cancel: &AtomicBool,
) -> Result<Option<Resolved>> {
    let proof = super::personal_adapter::proof(world)
        .map_err(|_| Error::Invalid("installed world proof"))?;
    let Some(proof) = proof else { return Ok(None) };
    if proof.is_retained() {
        return Ok(None);
    }
    let store = crate::handover::durable::MarkerStore::open_existing(super::authority::dir(world))?;
    let installed = Installed::read(&store, proof.core.transfer)?
        .ok_or(Error::Invalid("missing installed receipt"))?;
    check_capability_available(
        &store,
        &installed,
        &crate::handover::ledger::Ledger::install(),
    )?;
    let resolved = cached_lookup(
        installed.root.clone(),
        installed.bearer.clone(),
        now,
        cancel,
    )
    .await?;
    resolved.matches_installation(&installed)?;
    Ok(Some(resolved))
}

/// No publication, no gameplay and no automatic contact fallback. All hops
/// share the same deadline, byte/candidate counters and original relay set.
pub(crate) async fn lookup(
    root: RootContext,
    bearer: Bearer,
    known: Option<&Prefix>,
    now: u64,
    cancel: &AtomicBool,
) -> Result<Resolved> {
    if now > root.expiry() {
        return Err(Error::Invalid("expired original invitation"));
    }
    let mut budget = Budget::new();
    let mut prefix = Prefix::default();
    let mut latest: Option<VerifiedCommit> = None;
    for _ in 0..4 {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        budget.hop()?;
        let predecessor = latest.as_ref().map(VerifiedCommit::event_hash);
        let keys = crypto::recipient(&root, bearer.expose(), predecessor)?;
        let source = latest
            .as_ref()
            .map_or(root.runtime(), |p| p.core().successor_runtime);
        let route = Route {
            author: source,
            recipient: keys.public_key().to_bytes(),
            until: root.expiry(),
        };
        let mut events = Vec::new();
        let mut heard = false;
        for (index, relay) in root.relays().iter().enumerate() {
            budget.check()?;
            // A stalled first relay cannot consume the later relays' share.
            // This never extends or replaces the whole-operation deadline.
            let remaining = budget
                .deadline()
                .saturating_duration_since(tokio::time::Instant::now());
            let share = remaining / (root.relays().len() - index) as u32;
            let result = tokio::time::timeout(
                share,
                stored::lookup(relay, &root, route, &mut budget, cancel),
            )
            .await;
            match result {
                Ok(Ok(found)) => {
                    heard = true;
                    events.extend(found);
                }
                Ok(Err(Error::Io)) | Err(_) => {}
                Ok(Err(error)) => return Err(error),
            }
        }
        if !heard {
            return Err(Error::Io);
        }
        let candidates = stored::verify_candidates(&events, &keys, source, &root, now, &budget)?;
        let mut next = None;
        for candidate in candidates {
            if let Verified::Committed(proof) = candidate {
                if proof.core().lineage != root_lineage(&root) {
                    return Err(Error::Invalid("original invitation lineage"));
                }
                // Multiple distinct commits at one epoch make Prefix
                // conflicted; relay ordering never chooses an authority.
                prefix.accept(&proof, &root, now)?;
                next = Some(proof);
            }
        }
        let Some(proof) = next else { break };
        latest = Some(proof);
    }
    if let Some(known) = known {
        known.confirmed_by(&prefix)?;
    }
    Ok(Resolved {
        root,
        bearer,
        prefix,
        latest,
    })
}

#[cfg(test)]
#[path = "continuation_tests.rs"]
mod tests;

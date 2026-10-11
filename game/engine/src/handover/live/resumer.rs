//! CP9: the one background owner of a retired source's Committed
//! publication (Spec 04 "World handover", "Committed publication").
//!
//! - Owned by the process, not by a world: `Resumer::global()`. Leaving the
//!   world, closing the window or dropping `GameState` never stops it (a
//!   detached thread doesn't outlive `main`, so the start-up scan,
//!   `resume_all`, is what carries a publication across a restart).
//! - The live source hands its publication over at Retired; start-up
//!   resumes every world whose `handover/` holds a retirement record and no
//!   "Committed published" record. Both read the record through the world's
//!   `MarkerStore` (`phase::RetiredCommit`): the exact persisted bytes are
//!   what is published, never a re-seal, and no bearer is needed.
//! - It never opens the world: it reads `handover/` only, and the open guard
//!   keeps refusing the world for ever (retired is forever).
//! - Each attempt has a fresh `Budget` bound to the one root. The first
//!   relay that witnesses the exact event by readback is the proof; in the
//!   same attempt the other relays of the root get a best-effort copy (no
//!   readback) and the count is logged. Between attempts it backs off (5 s
//!   doubling to a 300 s ceiling) and never gives up.
//! - Once proven, the "Committed published" record is written durably
//!   (`MarkerStore`: 0600, temp + fsync + rename + directory fsync) in the
//!   same `handover/`, and the resumer drops that world.
//!
//! CP10 (HIGH-2), the recipient flavour: a recipient past its PreparedAck
//! waits for Committed here, not in its session. The live session hands it
//! over at Prepared (`adopt_import`); start-up resumes every import whose
//! working world's `handover/` holds a Prepared receipt and no Activated or
//! quarantine record (`resume_imports`). Both read the durable receipt
//! (`phase::AwaitingCommit`: the bearer, the exact Prepared event, the root's
//! relays). Each attempt has a fresh `Budget`: relays in the root's order,
//! a lookup of the bearer's route, the Committed opened against the stored
//! Prepared, then an independent readback (the proof); then the durable
//! Activated record (with the bearer, H3). Same backoff, never gives up;
//! only the quarantine record (the source's authenticated pre-retirement
//! Abort) ends the wait.
use crate::handover::{
    Id,
    durable::{MarkerKind, MarkerStore},
    phase::{AwaitingCommit, RetiredCommit},
    stored::{self, Budget},
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

/// Bounded backoff between attempts: `first`, doubling, capped at `ceiling`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub first: Duration,
    pub ceiling: Duration,
}
impl Backoff {
    pub const PRODUCTION: Backoff = Backoff { first: Duration::from_secs(5), ceiling: Duration::from_secs(300) };
    /// The wait after the `failures`-th failed attempt in a row (1-based).
    pub fn delay(&self, failures: u32) -> Duration {
        let doubled = self.first.saturating_mul(1u32 << failures.saturating_sub(1).min(16));
        doubled.min(self.ceiling)
    }
}

/// CP9 review MEDIUM-1: attempt `attempt` (0-based) tries the root's relays
/// starting at index `attempt mod n`, so a relay that stalls in first place
/// can't hold every attempt.
pub fn relay_order(n: usize, attempt: u32) -> Vec<usize> {
    (0..n).map(|i| (attempt as usize % n.max(1) + i) % n.max(1)).collect()
}
/// Each relay's own share of an attempt's 25 s: 25 s / n, at least 5 s.
pub fn relay_share(n: usize) -> Duration {
    (Duration::from_secs(25) / n.max(1) as u32).max(Duration::from_secs(5))
}

/// CP9 review LOW-6: this install's runtime key (the one its handovers sign
/// with), `None` when it has none. The start-up scans publish or await only
/// records bound to it; a planted record never contacts a relay.
pub fn install_runtime() -> Option<[u8; 32]> {
    crate::runtime_identity::RuntimeIdentity::load().ok().flatten().map(|i| i.keys().public_key().to_bytes())
}

/// What a test sees of one attempt.
#[cfg(test)]
#[derive(Clone, Debug)]
pub struct Attempt {
    pub transfer: Id,
    pub started: Instant,
    /// The attempt's own Budget as it started (a fresh one: nothing
    /// received yet, a deadline of its own).
    pub budget_deadline: tokio::time::Instant,
    pub budget_bytes_at_start: usize,
    pub proven: bool,
    pub copies: usize,
    /// CP10: a recipient's Committed lookup (not a source's publication).
    pub import: bool,
    /// The relay index this attempt started at (MEDIUM-1 rotation).
    pub first_relay: usize,
}

/// Which side a job serves: one process can be both (tests are).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Flavour {
    Source,
    Import,
}

struct Shared {
    backoff: Backoff,
    /// CP11 (CP10 review HIGH-1): the injectable clock (Unix seconds). It
    /// is never a validity bound for an awaiting import's Committed.
    clock: fn() -> u64,
    /// Transfers this resumer is publishing (source) or awaiting (import)
    /// now, one thread each.
    jobs: Mutex<BTreeSet<(Flavour, Id)>>,
    /// CP9 review MEDIUM-2: a wake-up for threads sleeping out a backoff
    /// (the exit grace makes them attempt at once).
    wake: (Mutex<u64>, Condvar),
    #[cfg(test)]
    log: Mutex<Vec<Attempt>>,
}

/// Cheap to clone: every clone is the same resumer.
#[derive(Clone)]
pub struct Resumer(Arc<Shared>);

/// Never set: the resumer never aborts a publication.
static NEVER: AtomicBool = AtomicBool::new(false);
/// Most entries the start-up scan looks at under the worlds root.
const WORLDS_SCAN_LIMIT: usize = 4096;

impl Resumer {
    pub fn new(backoff: Backoff) -> Self {
        Self::with_clock(backoff, unix_now)
    }
    /// A resumer reading `clock` for today (tests: a day past every expiry).
    pub fn with_clock(backoff: Backoff, clock: fn() -> u64) -> Self {
        Self(Arc::new(Shared {
            backoff,
            clock,
            jobs: Mutex::new(BTreeSet::new()),
            wake: (Mutex::new(0), Condvar::new()),
            #[cfg(test)]
            log: Mutex::new(Vec::new()),
        }))
    }
    /// The process's one resumer (production backoff).
    pub fn global() -> &'static Resumer {
        static GLOBAL: OnceLock<Resumer> = OnceLock::new();
        GLOBAL.get_or_init(|| Resumer::new(Backoff::PRODUCTION))
    }
    /// Whether this resumer is publishing `transfer` now.
    pub fn holds(&self, transfer: Id) -> bool {
        self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).contains(&(Flavour::Source, transfer))
    }
    /// CP10: whether this resumer is awaiting `transfer`'s Committed.
    pub fn holds_import(&self, transfer: Id) -> bool {
        self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).contains(&(Flavour::Import, transfer))
    }
    #[cfg(test)]
    pub fn attempts(&self) -> Vec<Attempt> {
        self.0.log.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// CP9 review MEDIUM-2 + CP12 (CP10 review MEDIUM-1): the exit grace
    /// (`GameState::exit_app`, which every way out goes through). Only a
    /// SOURCE publication in flight is waited for: with none it returns at
    /// once, and a recipient's wait for Committed never holds the exit (it
    /// can't finish before its source publishes, and resumes at the next
    /// start). Otherwise every thread sleeping out a backoff is woken to
    /// attempt at once, and the publications get at most `grace` (never
    /// longer). What isn't done resumes at the next launch from the durable
    /// records.
    pub fn finish_within(&self, grace: Duration) -> bool {
        if !self.publishing() {
            return true;
        }
        log::info!("handover: finishing a Committed publication before quitting (at most {grace:?})");
        let deadline = std::time::Instant::now() + grace;
        {
            let (generation, cv) = &self.0.wake;
            *generation.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            cv.notify_all();
        }
        loop {
            if !self.publishing() {
                return true;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            std::thread::sleep((deadline - now).min(Duration::from_millis(20)));
        }
    }

    /// Whether a source publication is in flight (an import's wait is not).
    fn publishing(&self) -> bool {
        self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(f, _)| *f == Flavour::Source)
    }
    /// Tests: hold a job slot (a publication if `source`, else an import's
    /// wait) until the guard drops, with no thread behind it.
    #[cfg(test)]
    pub(crate) fn test_hold(&self, source: bool, transfer: Id) -> JobGuard {
        let job = (if source { Flavour::Source } else { Flavour::Import }, transfer);
        self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(job);
        JobGuard(self.0.clone(), job)
    }

    /// Take over the Committed publication of the world at `world_dir`
    /// (its `handover/` holds `retired`'s record). Idempotent per transfer;
    /// nothing to do once its "Committed published" record exists. CP9
    /// review LOW-6: only a Commit signed by `install` (this install's
    /// runtime key) is ever published; anything else is logged and never
    /// contacts a relay.
    pub fn adopt(&self, world_dir: &Path, retired: RetiredCommit, install: [u8; 32]) -> bool {
        let transfer = retired.transfer();
        if retired.event().pubkey.to_bytes() != install {
            log::warn!("handover: a retirement record under {} isn't this install's; it is not published", world_dir.display());
            return false;
        }
        let Ok(store) = existing_store(world_dir) else { return false };
        if retired.is_published(&store) {
            return false;
        }
        if !self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert((Flavour::Source, transfer)) {
            return false;
        }
        let shared = self.0.clone();
        let dir = world_dir.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("handover-committed".into())
            .spawn(move || run(shared, dir, retired));
        if spawned.is_err() {
            self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).remove(&(Flavour::Source, transfer));
            log::error!("handover: couldn't start the Committed publication; it resumes at the next start");
            return false;
        }
        true
    }

    /// The start-up step `native_boot_services` runs (CP9 review LOW-5):
    /// every retired world's Committed and every import's wait, bound to
    /// `install`. Returns (publications, waits) taken over.
    pub fn boot(&self, worlds_root: &Path, imports: &Path, install: Option<[u8; 32]>) -> (usize, usize) {
        (self.resume_all(worlds_root, install), self.resume_imports(imports, install))
    }

    /// CP10: take over the wait for `transfer`'s Committed of the import
    /// whose working world is `world_dir` (its `handover/` holds the Prepared
    /// receipt). Idempotent per transfer; nothing to do once Activated or
    /// quarantined. It never opens the world and never creates `handover/`.
    pub fn adopt_import(&self, world_dir: &Path, transfer: Id, install: [u8; 32]) -> bool {
        let Ok(store) = existing_store(world_dir) else { return false };
        match AwaitingCommit::read(&store, transfer) {
            Ok(Some(a)) if a.recipient_runtime() != install => {
                log::warn!("handover: an import's receipt under {} isn't this install's; it is not resumed", world_dir.display());
                return false;
            }
            Ok(Some(_)) => {}
            Ok(None) => return false,
            Err(e) => {
                log::warn!("handover: an import's Prepared receipt can't be read: {e}");
                return false;
            }
        }
        if !self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert((Flavour::Import, transfer)) {
            return false;
        }
        let shared = self.0.clone();
        let dir = world_dir.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("handover-activate".into())
            .spawn(move || run_import(shared, dir, transfer));
        if spawned.is_err() {
            self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).remove(&(Flavour::Import, transfer));
            log::error!("handover: couldn't start waiting for Committed; it resumes at the next start");
            return false;
        }
        true
    }

    /// CP10 start-up: every import under `imports` (`handover-*`) whose
    /// working world's `handover/` holds a Prepared receipt with no Activated
    /// or quarantine record. Bounded; creates nothing. Returns how many waits
    /// it took over.
    pub fn resume_imports(&self, imports: &Path, install: Option<[u8; 32]>) -> usize {
        let Some(install) = install else { return 0 };
        import_receipts(imports).into_iter().filter(|(working, t)| self.adopt_import(working, *t, install)).count()
    }

    /// Start-up: every world under `worlds_root` whose `handover/` holds a
    /// retirement record and no "Committed published" record. Bounded, and
    /// it creates nothing (a world without `handover/` is skipped). Returns
    /// how many publications it took over.
    pub fn resume_all(&self, worlds_root: &Path, install: Option<[u8; 32]>) -> usize {
        let Some(install) = install else { return 0 };
        let Ok(entries) = std::fs::read_dir(worlds_root) else { return 0 };
        let mut adopted = 0;
        for (seen, entry) in entries.enumerate() {
            if seen >= WORLDS_SCAN_LIMIT {
                log::warn!("handover: start-up scan stopped at {WORLDS_SCAN_LIMIT} entries");
                break;
            }
            let Ok(entry) = entry else { continue };
            let world = entry.path();
            // CP9 review LOW-5: worlds as the world list sees them (a
            // symlinked world folder included); `handover/` itself must be a
            // real directory, as the open guard requires.
            if !world.is_dir() || !is_real_dir(&super::authority::dir(&world)) {
                continue;
            }
            let Ok(store) = existing_store(&world) else { continue };
            match RetiredCommit::scan(&store) {
                Ok(records) => {
                    for retired in records {
                        if self.adopt(&world, retired, install) {
                            adopted += 1;
                        }
                    }
                }
                // Unreadable: the world stays refused; nothing to publish.
                Err(e) => log::warn!("handover: a retirement record under {} can't be read: {e}", world.display()),
            }
        }
        adopted
    }
}

fn is_real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// The world's `handover/` store, only if it already exists (never created
/// here: a world deleted under the resumer stays deleted).
fn existing_store(world_dir: &Path) -> crate::handover::Result<MarkerStore> {
    if !is_real_dir(&super::authority::dir(world_dir)) {
        return Err(crate::handover::Error::Io);
    }
    super::authority::store(world_dir)
}

/// CP9 review LOW-1: the job leaves `jobs` however its thread ends (a
/// panic included), so the transfer can be taken over again.
pub(crate) struct JobGuard(Arc<Shared>, (Flavour, Id));
impl Drop for JobGuard {
    fn drop(&mut self) {
        self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.1);
    }
}

/// Sleep out a backoff, or less if the exit grace wakes the resumer.
fn pause(shared: &Shared, wait: Duration) {
    let (generation, cv) = &shared.wake;
    let g = generation.lock().unwrap_or_else(|e| e.into_inner());
    let start = *g;
    let _ = cv.wait_timeout_while(g, wait, |g| *g == start);
}

/// One publication, until proven and recorded. Never aborts.
/// Every (working world, transfer) under `imports` (`handover-*`) whose
/// working world's real `handover/` holds a `prepared-<transfer>.dat`.
/// Bounded (the inert scan limit, 256 records a world); creates nothing and
/// follows no symlinked folder.
fn import_receipts(imports: &Path) -> Vec<(PathBuf, Id)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(imports) else { return out };
    for (seen, entry) in entries.enumerate() {
        if seen >= super::inert::SCAN_LIMIT {
            log::warn!("handover: the imports scan stopped at {} entries", super::inert::SCAN_LIMIT);
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_name().to_str().is_some_and(|n| n.starts_with("handover-")) {
            continue;
        }
        let working = entry.path().join("working");
        let records = super::authority::dir(&working);
        if !is_real_dir(&entry.path()) || !is_real_dir(&working) || !is_real_dir(&records) {
            continue;
        }
        let Ok(names) = std::fs::read_dir(&records) else { continue };
        for name in names.take(256).flatten() {
            let name = name.file_name();
            if let Some(t) = name
                .to_str()
                .and_then(|n| n.strip_prefix("prepared-")?.strip_suffix(".dat"))
                .and_then(|v| hex::decode(v).ok())
                .and_then(|v| Id::try_from(v).ok())
            {
                out.push((working.clone(), t));
            }
        }
    }
    out
}

/// CP11 (coordinator): the world list's plain status line for every import
/// still awaiting its Committed (a Prepared receipt with no Activated or
/// quarantine record): "Waiting for <npub> to finish handing this world
/// over". Read-only and bounded like the start-up scan; there is no "take
/// over anyway" (it risks two live copies).
pub fn awaiting_statuses(imports: &Path) -> Vec<String> {
    import_receipts(imports)
        .into_iter()
        .filter_map(|(working, t)| {
            let store = existing_store(&working).ok()?;
            let npub = AwaitingCommit::read(&store, t).ok()??.source_npub()?;
            Some(super::copy::awaiting_handover(&npub))
        })
        .collect()
}

fn run(shared: Arc<Shared>, world: PathBuf, retired: RetiredCommit) {
    let transfer = retired.transfer();
    let _job = JobGuard(shared.clone(), (Flavour::Source, transfer));
    let mut failures = 0u32;
    let proof = loop {
        match attempt(&shared, &retired, failures) {
            Ok((proof, copies)) => {
                log::info!("handover: Committed published and read back; {copies} other relay(s) took a copy");
                break proof;
            }
            Err(e) => {
                failures = failures.saturating_add(1);
                let wait = shared.backoff.delay(failures);
                log::warn!("handover: Committed publication attempt {failures} failed ({e}); retrying in {wait:?}");
                pause(&shared, wait);
            }
        }
    };
    // The durable "Committed published" record, retried the same way. A
    // world whose `handover/` has gone (deleted) has nowhere to record it.
    let mut failures = 0u32;
    loop {
        let Ok(store) = existing_store(&world) else {
            log::warn!("handover: the retired world's folder is gone; its Committed publication isn't recorded");
            break;
        };
        match retired.record_published(&store, &proof) {
            Ok(()) => break,
            Err(_) if retired.is_published(&store) => break,
            Err(e) => {
                failures = failures.saturating_add(1);
                let wait = shared.backoff.delay(failures);
                log::warn!("handover: the Committed published record failed ({e}); retrying in {wait:?}");
                pause(&shared, wait);
            }
        }
    }
}

/// CP10: one import's wait for Committed, until Activated (or quarantined).
/// Never aborts on its own.
fn run_import(shared: Arc<Shared>, world: PathBuf, transfer: Id) {
    let _job = JobGuard(shared.clone(), (Flavour::Import, transfer));
    let mut failures = 0u32;
    loop {
        // Re-read the durable state each attempt: a quarantine record (the
        // source's authenticated Abort) or a deleted import ends the wait.
        let awaiting = match existing_store(&world).map(|s| (AwaitingCommit::read(&s, transfer), s)) {
            Ok((Ok(Some(a)), store)) => Some((a, store)),
            Ok((Ok(None), _)) => None,
            Ok((Err(e), _)) => {
                log::warn!("handover: an import's Prepared receipt can't be read ({e}); it stays quarantined");
                None
            }
            Err(_) => {
                log::warn!("handover: an import's folder is gone; nothing to activate");
                None
            }
        };
        let Some((mut awaiting, store)) = awaiting else { break };
        let outcome = attempt_import(&shared, &awaiting, failures).and_then(|(proof, stored)| awaiting.activate(&store, &proof, &stored));
        match outcome {
            Ok(_) => {
                log::info!("handover: Committed found and read back; the import is activated");
                break;
            }
            Err(e) => {
                if store.read(MarkerKind::Activated, transfer).is_ok_and(|r| r.is_some()) {
                    break;
                }
                failures = failures.saturating_add(1);
                let wait = shared.backoff.delay(failures);
                log::warn!("handover: waiting for Committed, attempt {failures} ({e}); retrying in {wait:?}");
                pause(&shared, wait);
            }
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// CP10: one recipient attempt with a FRESH Budget bound to the one root:
/// relays in the root's order; on each, a lookup of the bearer's route,
/// the Committed opened against the stored Prepared, then an independent
/// readback of that exact event (the proof).
fn attempt_import(
    shared: &Shared,
    awaiting: &AwaitingCommit,
    number: u32,
) -> crate::handover::Result<(crate::handover::crypto::VerifiedCommit, stored::StoredProof)> {
    let mut budget = Budget::new();
    #[cfg(test)]
    let (started, budget_deadline, budget_bytes_at_start) = (Instant::now(), budget.deadline(), budget.bytes_received());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| crate::handover::Error::Io)?;
    let r = rt.block_on(async {
        let root = awaiting.root();
        let route = awaiting.route()?;
        let mut last = Err(crate::handover::Error::Invalid("no relay"));
        let relays = root.relays();
        for i in relay_order(relays.len(), number) {
            let url = &relays[i];
            budget.share_deadline(relay_share(relays.len()));
            let events = match stored::lookup(url, root, route, &mut budget, &NEVER).await {
                Ok(e) => e,
                Err(e) => {
                    last = Err(e);
                    continue;
                }
            };
            let Some((event, commit)) = awaiting.committed(&events, (shared.clock)()) else {
                last = Err(crate::handover::Error::Invalid("no Committed yet"));
                continue;
            };
            let verified = crate::handover::crypto::Verified::Committed(commit.clone());
            match stored::readback(url, root, &event, &verified, route, &mut budget, &NEVER).await {
                Ok(proof) => return Ok((commit, proof)),
                Err(e) => last = Err(e),
            }
        }
        last
    });
    #[cfg(test)]
    shared.log.lock().unwrap_or_else(|e| e.into_inner()).push(Attempt {
        transfer: awaiting.transfer(),
        started,
        budget_deadline,
        budget_bytes_at_start,
        proven: r.is_ok(),
        copies: 0,
        import: true,
        first_relay: relay_order(awaiting.root().relays().len(), number)[0],
    });
    r
}

/// One attempt with a FRESH Budget bound to the one root: relays in the
/// root's order until one witnesses the exact event (the proof), then a
/// best-effort copy to each other relay of the root. Returns the proof and
/// how many copies a relay took.
fn attempt(shared: &Shared, retired: &RetiredCommit, number: u32) -> crate::handover::Result<(stored::StoredProof, usize)> {
    let mut budget = Budget::new();
    #[cfg(test)]
    let (started, budget_deadline, budget_bytes_at_start) = (Instant::now(), budget.deadline(), budget.bytes_received());
    #[cfg(not(test))]
    let _ = shared;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| crate::handover::Error::Io)?;
    let r = rt.block_on(async {
        let relays = retired.root().relays();
        let mut last = Err(crate::handover::Error::Invalid("no relay"));
        let mut proven = None;
        // MEDIUM-1: rotate the first relay per attempt, and give each relay
        // its own share of the attempt (a copy never counts as proof).
        for i in relay_order(relays.len(), number) {
            let url = &relays[i];
            budget.share_deadline(relay_share(relays.len()));
            match stored::publish_retired(url, retired, &mut budget, &NEVER).await {
                Ok(p) => {
                    proven = Some((i, p));
                    break;
                }
                Err(e) => last = Err(e),
            }
        }
        let Some((i, proof)) = proven else { return last };
        let mut copies = 0;
        for (j, url) in relays.iter().enumerate() {
            if j == i {
                continue;
            }
            budget.share_deadline(relay_share(relays.len()));
            if stored::copy_retired(url, retired, &mut budget, &NEVER).await.is_ok() {
                copies += 1;
            }
        }
        Ok((proof, copies))
    });
    #[cfg(test)]
    shared.log.lock().unwrap_or_else(|e| e.into_inner()).push(Attempt {
        transfer: retired.transfer(),
        started,
        budget_deadline,
        budget_bytes_at_start,
        proven: r.is_ok(),
        copies: r.as_ref().map_or(0, |(_, c)| *c),
        import: false,
        first_relay: relay_order(retired.root().relays().len(), number)[0],
    });
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The production schedule: 5 s doubling to a 300 s ceiling, for ever.
    /// CP9 review MEDIUM-1: attempt k starts at relay k mod n; each relay
    /// gets 25 s / n of the attempt, at least 5 s.
    #[test]
    fn handover_resumer_rotates_the_first_relay_and_shares_the_deadline() {
        assert_eq!(relay_order(3, 0), vec![0, 1, 2]);
        assert_eq!(relay_order(3, 1), vec![1, 2, 0], "attempt 1 didn't start at the next relay");
        assert_eq!(relay_order(3, 5), vec![2, 0, 1]);
        assert_eq!(relay_order(1, 7), vec![0]);
        assert_eq!(relay_share(1), Duration::from_secs(25));
        assert_eq!(relay_share(2), Duration::from_millis(12_500));
        assert_eq!(relay_share(8), Duration::from_secs(5), "the floor");
    }
    /// CP12 (CP10 review MEDIUM-1): the exit grace waits only while a
    /// source publication is in flight; a recipient's wait never holds it.
    #[test]
    fn handover_exit_grace_waits_for_a_source_publication_never_for_a_recipients_wait() {
        let r = Resumer::new(Backoff::PRODUCTION);
        let wait = r.test_hold(false, [1; 16]);
        let start = std::time::Instant::now();
        assert!(r.finish_within(Duration::from_secs(5)), "the exit grace reported a recipient's wait unfinished");
        assert!(start.elapsed() < Duration::from_secs(1), "the exit grace waited for a recipient's wait");
        let publishing = r.test_hold(true, [2; 16]);
        let start = std::time::Instant::now();
        assert!(!r.finish_within(Duration::from_millis(200)), "the exit grace didn't wait for a source publication in flight");
        assert!(start.elapsed() >= Duration::from_millis(200), "the exit grace gave up early");
        let done = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(publishing);
        });
        assert!(r.finish_within(Duration::from_secs(5)), "the grace didn't end when the publication finished");
        done.join().unwrap();
        drop(wait);
    }

    #[test]
    fn handover_resumer_backoff_doubles_to_its_ceiling_and_stays_there() {
        let b = Backoff::PRODUCTION;
        let secs: Vec<u64> = (1..=9).map(|n| b.delay(n).as_secs()).collect();
        assert_eq!(secs, vec![5, 10, 20, 40, 80, 160, 300, 300, 300]);
        assert_eq!(b.delay(u32::MAX), Duration::from_secs(300));
    }
}

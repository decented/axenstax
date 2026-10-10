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
use crate::handover::{
    Id,
    durable::MarkerStore,
    phase::RetiredCommit,
    stored::{self, Budget},
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
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
}

struct Shared {
    backoff: Backoff,
    /// Transfers this resumer is publishing now (one thread each).
    jobs: Mutex<BTreeSet<Id>>,
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
        Self(Arc::new(Shared {
            backoff,
            jobs: Mutex::new(BTreeSet::new()),
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
        self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).contains(&transfer)
    }
    #[cfg(test)]
    pub fn attempts(&self) -> Vec<Attempt> {
        self.0.log.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Take over the Committed publication of the world at `world_dir`
    /// (its `handover/` holds `retired`'s record). Idempotent per transfer;
    /// nothing to do once its "Committed published" record exists.
    pub fn adopt(&self, world_dir: &Path, retired: RetiredCommit) -> bool {
        let transfer = retired.transfer();
        let Ok(store) = existing_store(world_dir) else { return false };
        if retired.is_published(&store).unwrap_or(true) {
            return false;
        }
        if !self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(transfer) {
            return false;
        }
        let shared = self.0.clone();
        let dir = world_dir.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("handover-committed".into())
            .spawn(move || run(shared, dir, retired));
        if spawned.is_err() {
            self.0.jobs.lock().unwrap_or_else(|e| e.into_inner()).remove(&transfer);
            log::error!("handover: couldn't start the Committed publication; it resumes at the next start");
            return false;
        }
        true
    }

    /// Start-up: every world under `worlds_root` whose `handover/` holds a
    /// retirement record and no "Committed published" record. Bounded, and
    /// it creates nothing (a world without `handover/` is skipped). Returns
    /// how many publications it took over.
    pub fn resume_all(&self, worlds_root: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(worlds_root) else { return 0 };
        let mut adopted = 0;
        for (seen, entry) in entries.enumerate() {
            if seen >= WORLDS_SCAN_LIMIT {
                log::warn!("handover: start-up scan stopped at {WORLDS_SCAN_LIMIT} entries");
                break;
            }
            let Ok(entry) = entry else { continue };
            let world = entry.path();
            if !is_real_dir(&world) || !is_real_dir(&super::authority::dir(&world)) {
                continue;
            }
            let Ok(store) = existing_store(&world) else { continue };
            match RetiredCommit::scan(&store) {
                Ok(records) => {
                    for retired in records {
                        if self.adopt(&world, retired) {
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

/// One publication, until proven and recorded. Never aborts.
fn run(shared: Arc<Shared>, world: PathBuf, retired: RetiredCommit) {
    let transfer = retired.transfer();
    let mut failures = 0u32;
    let proof = loop {
        match attempt(&shared, &retired) {
            Ok((proof, copies)) => {
                log::info!("handover: Committed published and read back; {copies} other relay(s) took a copy");
                break proof;
            }
            Err(e) => {
                failures = failures.saturating_add(1);
                let wait = shared.backoff.delay(failures);
                log::warn!("handover: Committed publication attempt {failures} failed ({e}); retrying in {wait:?}");
                std::thread::sleep(wait);
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
            Err(_) if retired.is_published(&store).unwrap_or(false) => break,
            Err(e) => {
                failures = failures.saturating_add(1);
                let wait = shared.backoff.delay(failures);
                log::warn!("handover: the Committed published record failed ({e}); retrying in {wait:?}");
                std::thread::sleep(wait);
            }
        }
    }
    shared.jobs.lock().unwrap_or_else(|e| e.into_inner()).remove(&transfer);
}

/// One attempt with a FRESH Budget bound to the one root: relays in the
/// root's order until one witnesses the exact event (the proof), then a
/// best-effort copy to each other relay of the root. Returns the proof and
/// how many copies a relay took.
fn attempt(shared: &Shared, retired: &RetiredCommit) -> crate::handover::Result<(stored::StoredProof, usize)> {
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
        for (i, url) in relays.iter().enumerate() {
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
            if j != i && stored::copy_retired(url, retired, &mut budget, &NEVER).await.is_ok() {
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
    });
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The production schedule: 5 s doubling to a 300 s ceiling, for ever.
    #[test]
    fn handover_resumer_backoff_doubles_to_its_ceiling_and_stays_there() {
        let b = Backoff::PRODUCTION;
        let secs: Vec<u64> = (1..=9).map(|n| b.delay(n).as_secs()).collect();
        assert_eq!(secs, vec![5, 10, 20, 40, 80, 160, 300, 300, 300]);
        assert_eq!(b.delay(u32::MAX), Duration::from_secs(300));
    }
}

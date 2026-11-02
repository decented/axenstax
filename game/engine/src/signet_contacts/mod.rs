//! Signet contacts sync (native) — the player's own Signet hands this game an
//! encrypted, read-only snapshot of their contacts (tiers and blocks), so Kin
//! and Kith from Signet count for online play and world chat without a manual
//! import. Spec: `docs/foundations/2026-10-01-signet-contacts-sync.md`; wire:
//! `signet/contacts_wire/` (pure port of `forgesworn/signet-contacts` v2).
//!
//! Red lines: the data goes from the player's own Signet, over a relay, to the
//! player's own disk. AxeNStax runs nothing in the path and keeps no copy.
//! Pairing rides the first relay of the player's own "Your relays" list (D4):
//! the request names it and Signet publishes its ack there. Every fetch after
//! that goes to the relay the ack names.
//!
//! Threads: one worker owns the 15-minute sync loop and every snapshot write;
//! a pairing attempt gets its own waiter thread. The frame thread reads a
//! status snapshot and sends commands — it never blocks on a relay. The one
//! exception is Disconnect (D12): it deletes both files on the calling thread
//! at once, so it never waits behind a sync stuck on a relay, and bumps a
//! generation so that sync's result is thrown away when it lands.
#![cfg(not(target_arch = "wasm32"))]

pub mod ack_wait;
pub mod book;
pub mod pairing_flow;
pub mod relay;
pub mod store;
pub mod sync;
pub mod ui;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::signet::contacts_wire::constants::ACK_DEFAULT_TIMEOUT_SECONDS;
use crate::signet::contacts_wire::Capability;

use pairing_flow::{PairingFlow, Phase, Request};
use store::{Grant, StorePaths};
use sync::SyncResult;

/// D1 — exactly these, nothing else.
pub const REQUESTED_CAPS: [Capability; 3] =
    [Capability::ReadDirectory, Capability::ReadTier, Capability::BlocksRead];
/// D4 — the pairing relay: the FIRST relay of the player's list. The contacts
/// v2 pairing request carries exactly one `relay=` (Signet reads it with
/// `params.get('relay')` in `signet-contacts/src/wire/pairing.ts` and
/// publishes its ack to that `rendezvousRelay`), so only one can be named.
/// Falls back to the first public default if the list is somehow empty.
pub fn pairing_relay(player_relays: &[String]) -> String {
    player_relays
        .first()
        .cloned()
        .unwrap_or_else(|| crate::server_resolve::PUBLIC_DEFAULT_RELAYS[0].to_string())
}
/// D3 — the one place the Signet web host is named.
pub const SIGNET_WEB_HOST: &str = "mysignet.app";
/// The name Signet shows on its consent screen.
pub const APP_NAME: &str = "Axe'n'Stax";
/// D11 — periodic sync while running.
pub const SYNC_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// D11 — the Friends-column-open trigger's debounce.
pub const COLUMN_DEBOUNCE_SECS: u64 = 60;
/// D10 — how often the worker re-checks staleness between syncs. Expiry is
/// time-driven: a snapshot can cross `expiresAt` with no fetch at all, and the
/// book (and a hosted world's allowlist) must learn it promptly.
pub const STALENESS_TICK: Duration = Duration::from_secs(60);

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What the Friends row needs to draw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub phase: Option<Phase>,
    pub connected: bool,
    pub people: usize,
    pub truncated: bool,
    pub stale: bool,
    /// The stored files can't be read; Disconnect is how the player gets out.
    pub unreadable: bool,
    /// Unix seconds of the last successful check with the relay.
    pub last_synced: Option<u64>,
    /// A one-off line (e.g. "Signet disconnected this game").
    pub notice: Option<&'static str>,
}

#[derive(Default)]
struct Shared {
    flow: PairingFlow,
    cancel: Option<Arc<AtomicBool>>,
    last_synced: Option<u64>,
    last_attempt: Option<u64>,
    notice: Option<&'static str>,
    book_changed: bool,
    column_last_drawn: Option<Instant>,
    /// Bumped by Disconnect. A sync that started under an older generation
    /// has its result discarded.
    generation: u64,
    /// Snapshot facts, refreshed from disk by the worker.
    connected: bool,
    people: usize,
    truncated: bool,
    stale: bool,
    unreadable: bool,
}

enum Cmd {
    SyncNow,
    Persist(Box<Grant>),
    Disconnect,
}

struct Service {
    shared: Arc<Mutex<Shared>>,
    tx: Mutex<Sender<Cmd>>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

/// What reading the stored Signet state found. "No Signet here" and "Signet is
/// here but couldn't be read" are different answers: the first has no blocks,
/// and the second has blocks nobody can see.
#[derive(Debug, PartialEq, Eq)]
pub enum SignetRead<T> {
    /// Not connected, or disconnected or revoked (D12).
    Absent,
    Value(T),
    /// A file couldn't be read, or was damaged and set aside. Says which.
    Unreadable(String),
}

impl<T> SignetRead<T> {
    pub fn value(self) -> Option<T> {
        match self {
            SignetRead::Value(t) => Some(t),
            _ => None,
        }
    }
}

fn set_aside<T>(path: &std::path::Path) -> SignetRead<T> {
    SignetRead::Unreadable(format!("{}: damaged, and set aside", path.display()))
}

/// The stored snapshot, but only when it belongs to the grant stored beside
/// it. A snapshot with no grant, or one left over from an older grant (a
/// failed delete, a sync that landed after Disconnect), counts for nothing.
fn read_current_snapshot(paths: &StorePaths) -> SignetRead<store::Snapshot> {
    use store::Loaded;
    let grant = match store::load_grant(&paths.grant) {
        Loaded::Ok(grant) => grant,
        Loaded::Absent => return SignetRead::Absent,
        Loaded::Quarantined => return set_aside(&paths.grant),
        Loaded::Unreadable(why) => return SignetRead::Unreadable(why),
    };
    let snap = match store::load_snapshot(&paths.snapshot) {
        Loaded::Ok(snap) => snap,
        Loaded::Absent => return SignetRead::Absent,
        Loaded::Quarantined => return set_aside(&paths.snapshot),
        Loaded::Unreadable(why) => return SignetRead::Unreadable(why),
    };
    if snap.projection.grant_id == grant.grant_id {
        SignetRead::Value(snap)
    } else {
        SignetRead::Absent
    }
}

fn current_snapshot(paths: &StorePaths) -> Option<store::Snapshot> {
    read_current_snapshot(paths).value()
}

/// The snapshot's contribution to the book at `now` (D8–D10). A revoked snapshot
/// contributes nothing and lifts its blocks, as Disconnect does (D12).
pub fn read_book_part_at(paths: &StorePaths, now: u64) -> SignetRead<crate::contacts::SignetPart> {
    match read_current_snapshot(paths) {
        SignetRead::Value(s) => match book::book_part(&s, now) {
            Some(part) => SignetRead::Value(part),
            None => SignetRead::Absent,
        },
        SignetRead::Absent => SignetRead::Absent,
        SignetRead::Unreadable(why) => SignetRead::Unreadable(why),
    }
}

/// The snapshot's contribution to the book right now (read by
/// `contacts::load_local_book`). Reads the files directly so it works before
/// (or without) `init`.
pub fn read_book_part_now() -> SignetRead<crate::contacts::SignetPart> {
    let read = read_book_part_at(&StorePaths::default_paths(), unix_now());
    note_read_problem(&read);
    read
}

/// Whether `problem` is news: a different one from the last, which it replaces.
/// No problem clears the last, so one that comes back is warned about again.
fn is_new_problem(last: &mut Option<String>, problem: Option<&str>) -> bool {
    let new = problem.is_some() && last.as_deref() != problem;
    *last = problem.map(str::to_owned);
    new
}

/// Warn once per distinct problem: this file is read every minute, and more.
fn note_read_problem<T>(read: &SignetRead<T>) {
    static LAST: Mutex<Option<String>> = Mutex::new(None);
    let problem = match read {
        SignetRead::Unreadable(why) => Some(why.as_str()),
        _ => None,
    };
    let Ok(mut last) = LAST.lock() else { return };
    if is_new_problem(&mut last, problem)
        && let Some(why) = problem
    {
        log::warn!(
            "[signet-contacts] {why}. A world being hosted keeps the blocks it has, and no new one \
             can be hosted, until Signet is reconnected."
        );
    }
}

/// Re-read the row facts from disk. A flip in staleness changes the book
/// (D10: stale adds nobody), so it raises `book_changed` — that is what makes
/// expired Kin/Kith leave a hosted world's allowlist without a fetch. Going
/// unreadable or coming back does too, for the same reason.
fn refresh_from_disk(sh: &mut Shared, paths: &StorePaths, now: u64) {
    let was_stale = sh.stale;
    let was_unreadable = sh.unreadable;
    sh.connected = store::load_grant(&paths.grant).ok().is_some();
    let read = read_current_snapshot(paths);
    note_read_problem(&read);
    sh.unreadable = matches!(read, SignetRead::Unreadable(_));
    match read {
        SignetRead::Value(s) => {
            sh.people = book::people_count(&s, now);
            sh.truncated = s.projection.truncated;
            sh.stale = book::is_stale(&s, now);
            sh.last_synced = sh.last_synced.max(Some(s.fetched_at));
        }
        SignetRead::Absent | SignetRead::Unreadable(_) => {
            sh.people = 0;
            sh.truncated = false;
            sh.stale = false;
        }
    }
    if sh.stale != was_stale || sh.unreadable != was_unreadable {
        sh.book_changed = true;
    }
}

fn generation(shared: &Arc<Mutex<Shared>>) -> Option<u64> {
    shared.lock().ok().map(|sh| sh.generation)
}

fn run_sync(shared: &Arc<Mutex<Shared>>, paths: &StorePaths, rt: &tokio::runtime::Runtime) {
    let now = unix_now();
    let started_gen = {
        let Ok(mut sh) = shared.lock() else { return };
        sh.last_attempt = Some(now);
        sh.generation
    };
    let result = sync::sync_once(paths, now, &mut |url, filter| {
        let r = rt.block_on(relay::fetch_events(url, filter));
        // Disconnected while the relay was answering: nothing may be written
        // for a grant that is gone.
        if generation(shared) != Some(started_gen) {
            return Err("disconnected during the fetch".into());
        }
        r
    });
    let Ok(mut sh) = shared.lock() else { return };
    if sh.generation != started_gen {
        refresh_from_disk(&mut sh, paths, unix_now());
        return;
    }
    match result {
        SyncResult::NoGrant | SyncResult::Failed => {}
        SyncResult::Unchanged => sh.last_synced = Some(now),
        SyncResult::Updated => {
            sh.last_synced = Some(now);
            sh.book_changed = true;
        }
        SyncResult::Revoked => {
            sh.last_synced = None;
            sh.notice = Some(ui::REVOKED_NOTICE);
            sh.book_changed = true;
        }
    }
    refresh_from_disk(&mut sh, paths, unix_now());
}

/// Start the worker: one sync now (boot, D11), then every 15 minutes or on a
/// command. No-op when already running.
pub fn init() {
    if SERVICE.get().is_some() {
        return;
    }
    let paths = StorePaths::default_paths();
    let shared = Arc::new(Mutex::new(Shared::default()));
    if let Ok(mut sh) = shared.lock() {
        refresh_from_disk(&mut sh, &paths, unix_now());
    }
    let (tx, rx) = channel::<Cmd>();
    let worker_shared = shared.clone();
    let spawned = std::thread::Builder::new().name("signet-contacts".into()).spawn(move || {
        let Some(rt) = relay::worker_runtime() else {
            log::warn!("[signet-contacts] no runtime; sync disabled");
            return;
        };
        run_sync(&worker_shared, &paths, &rt);
        let mut next_sync = Instant::now() + SYNC_INTERVAL;
        loop {
            let wait = next_sync.saturating_duration_since(Instant::now()).min(STALENESS_TICK);
            match rx.recv_timeout(wait) {
                Err(RecvTimeoutError::Timeout) if Instant::now() < next_sync => {
                    // No fetch due — but expiry is time-driven (D10).
                    if let Ok(mut sh) = worker_shared.lock() {
                        refresh_from_disk(&mut sh, &paths, unix_now());
                    }
                }
                Ok(Cmd::SyncNow) | Err(RecvTimeoutError::Timeout) => {
                    run_sync(&worker_shared, &paths, &rt);
                    next_sync = Instant::now() + SYNC_INTERVAL;
                }
                Ok(Cmd::Persist(grant)) => {
                    match store::save_grant(&paths.grant, &grant) {
                        // The old snapshot belonged to an older grant, if any.
                        // A leftover is ignored anyway (grant_id mismatch).
                        Ok(()) => store::discard_snapshot(&paths.snapshot),
                        Err(e) => log::warn!("[signet-contacts] could not save the grant: {e}"),
                    }
                    if let Ok(mut sh) = worker_shared.lock() {
                        // A new grant has never synced; an old grant's time
                        // must not show against it.
                        sh.last_synced = None;
                        sh.book_changed = true;
                    }
                    run_sync(&worker_shared, &paths, &rt);
                    next_sync = Instant::now() + SYNC_INTERVAL;
                }
                Ok(Cmd::Disconnect) => {
                    // `disconnect()` already deleted both files; this pass
                    // catches a snapshot an in-flight sync wrote meanwhile.
                    if let Err(e) = store::delete_all(&paths) {
                        log::warn!("[signet-contacts] {e}");
                    }
                    if let Ok(mut sh) = worker_shared.lock() {
                        sh.last_synced = None;
                        sh.book_changed = true;
                        refresh_from_disk(&mut sh, &paths, unix_now());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("[signet-contacts] worker did not start: {e}");
        return;
    }
    let _ = SERVICE.set(Service { shared, tx: Mutex::new(tx) });
}

fn send(cmd: Cmd) {
    if let Some(s) = SERVICE.get()
        && let Ok(tx) = s.tx.lock()
    {
        let _ = tx.send(cmd);
    }
}

fn with_shared<T>(f: impl FnOnce(&mut Shared) -> T) -> Option<T> {
    let s = SERVICE.get()?;
    let mut sh = s.shared.lock().ok()?;
    Some(f(&mut sh))
}

/// The Friends row's view.
pub fn view() -> View {
    with_shared(|sh| View {
        phase: Some(sh.flow.phase().clone()),
        connected: sh.connected,
        people: sh.people,
        truncated: sh.truncated,
        stale: sh.stale,
        unreadable: sh.unreadable,
        last_synced: sh.last_synced,
        notice: sh.notice,
    })
    .unwrap_or_default()
}

/// The frame loop drains this: `true` once after the book's Signet part
/// changed, so the column re-reads it and a hosted world learns it.
pub fn take_book_changed() -> bool {
    with_shared(|sh| std::mem::take(&mut sh.book_changed)).unwrap_or(false)
}

/// Called every frame the Friends column is drawn. A draw after a gap is the
/// column opening (D11); it syncs when the last attempt is ≥ 60 s old.
pub fn note_column_drawn() {
    let due = with_shared(|sh| {
        let opened = sh.column_last_drawn.is_none_or(|t| t.elapsed() > Duration::from_secs(1));
        sh.column_last_drawn = Some(Instant::now());
        opened && sh.connected && column_sync_due(sh.last_attempt, unix_now())
    });
    if due == Some(true) {
        send(Cmd::SyncNow);
    }
}

/// The ≥ 60 s debounce for the column-open trigger.
pub fn column_sync_due(last_attempt: Option<u64>, now: u64) -> bool {
    last_attempt.is_none_or(|t| now.saturating_sub(t) >= COLUMN_DEBOUNCE_SECS)
}

/// "Connect Signet contacts": a fresh app key and challenge (D5), the QR, and
/// a waiter thread on the pairing relay (the first of `player_relays`).
pub fn start_pairing(player_relays: &[String]) {
    let relay = pairing_relay(player_relays);
    let Some(challenge) = pairing_flow::fresh_challenge() else {
        log::warn!("[signet-contacts] no randomness for a challenge");
        return;
    };
    let app_keys = nostr::Keys::generate();
    let req = Request { app_name: APP_NAME, caps: &REQUESTED_CAPS, relay: &relay, web_host: SIGNET_WEB_HOST };
    let Some(ticket) = with_shared(|sh| {
        if let Some(c) = sh.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        sh.notice = None;
        let t = sh.flow.start(&req, app_keys, challenge, unix_now());
        if t.is_ok() {
            let flag = Arc::new(AtomicBool::new(false));
            sh.cancel = Some(flag.clone());
            t.ok().map(|t| (t, flag))
        } else {
            None
        }
    })
    .flatten() else {
        return;
    };
    let shared = SERVICE.get().map(|s| s.shared.clone());
    let (ticket, cancel) = ticket;
    let _ = std::thread::Builder::new().name("signet-contacts-pair".into()).spawn(move || {
        let Some(rt) = relay::worker_runtime() else { return };
        let mut live = relay::LiveAckRelay { rt: &rt, url: ticket.relay.clone() };
        let deadline = unix_now() + ACK_DEFAULT_TIMEOUT_SECONDS;
        let out = ack_wait::await_ack(
            &mut live,
            &ticket.app_keys,
            &ticket.challenge,
            &REQUESTED_CAPS,
            deadline,
            &unix_now,
            &cancel,
        );
        let Some(shared) = shared else { return };
        let Ok(mut sh) = shared.lock() else { return };
        match out {
            ack_wait::AckWait::Found(ack) => {
                sh.flow.on_ack(ticket.generation, ack);
            }
            ack_wait::AckWait::TimedOut => sh.flow.on_timeout(ticket.generation),
            ack_wait::AckWait::Cancelled => {}
        }
    });
}

/// Continue on the code screen: the only path that persists a grant (D6).
pub fn continue_pairing() {
    let grant = with_shared(|sh| {
        sh.cancel = None;
        sh.flow.continue_pressed(unix_now())
    })
    .flatten();
    if let Some(g) = grant {
        if let Some(Ok(mut sh)) = SERVICE.get().map(|s| s.shared.lock()) {
            sh.connected = true;
        }
        send(Cmd::Persist(Box::new(g)));
    }
}

/// Cancel at any step: discard key, challenge and ack.
pub fn cancel_pairing() {
    with_shared(|sh| {
        if let Some(c) = sh.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        sh.flow.cancel();
    });
}

pub fn sync_now() {
    send(Cmd::SyncNow);
}

/// D12 — forget the grant and the snapshot on this machine. Takes effect here
/// and now, not when the worker gets to it: the worker may be stuck behind a
/// relay, and the Signet rows must stop counting the moment the player asks.
pub fn disconnect() {
    let paths = StorePaths::default_paths();
    if let Err(e) = store::delete_all(&paths) {
        log::warn!("[signet-contacts] {e}");
    }
    with_shared(|sh| {
        sh.generation = sh.generation.wrapping_add(1);
        sh.notice = None;
        sh.last_synced = None;
        sh.book_changed = true;
        refresh_from_disk(sh, &paths, unix_now());
    });
    send(Cmd::Disconnect);
}

pub fn dismiss_notice() {
    with_shared(|sh| sh.notice = None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signet::contacts_wire::projection::{Frontier, Identity, ProjectedContact, Tier as WireTier};
    use crate::signet::contacts_wire::Projection;
    use store::{Grant, Snapshot, GRANT_VERSION};

    fn tmp(name: &str) -> StorePaths {
        let dir = std::env::temp_dir().join(format!("axenstax-signet-mod-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        StorePaths::in_profile(&dir)
    }

    fn grant(grant_id: &str) -> Grant {
        Grant {
            v: GRANT_VERSION,
            app_secret_hex: nostr::Keys::generate().secret_key().to_secret_hex(),
            grant_id: grant_id.into(),
            rail_pubkey: "b".repeat(64),
            projection_tag: "c".repeat(32),
            relay: "wss://relay.example.com".into(),
            granted_capabilities: REQUESTED_CAPS.to_vec(),
            max_staleness_seconds: 21_600,
            paired_at: 1,
        }
    }

    fn person(n: u8, tier: WireTier, blocked: bool) -> ProjectedContact {
        ProjectedContact {
            contact_id: format!("{n:032x}"),
            identities: Some(vec![Identity { pubkey: hex::encode([n; 32]), verification: None }]),
            display_name: None,
            effective_tier: Some(tier),
            tier_source: None,
            roles: None,
            contact_methods: None,
            blocked: Some(blocked),
            checks: None,
        }
    }

    fn book_part_at(paths: &StorePaths, now: u64) -> Option<crate::contacts::SignetPart> {
        read_book_part_at(paths, now).value()
    }

    fn snapshot(grant_id: &str, expires_at: u64) -> Snapshot {
        let p = Projection {
            v: 2,
            grant_id: grant_id.into(),
            scopes: REQUESTED_CAPS.to_vec(),
            frontier: Frontier { max_clock: 1, op_count: 1, published_at: 1, device_id: "d".repeat(32) },
            issued_at: 0,
            expires_at,
            contacts: vec![person(1, WireTier::Kin, false), person(2, WireTier::Kith, true)],
            revoked: false,
            truncated: false,
        };
        Snapshot::replacing(None, p, 50)
    }

    #[test]
    fn crossing_expires_at_flips_book_changed_and_leaves_only_blocks() {
        let paths = tmp("stale");
        store::save_grant(&paths.grant, &grant(&"a".repeat(32))).unwrap();
        store::save_snapshot(&paths.snapshot, &snapshot(&"a".repeat(32), 1000)).unwrap();
        let mut sh = Shared::default();
        refresh_from_disk(&mut sh, &paths, 900);
        sh.book_changed = false;
        // Fresh: Kin admitted, the block applies.
        let part = book_part_at(&paths, 900).unwrap();
        assert_eq!(part.contacts.len(), 1);
        assert_eq!(part.blocked, vec![[2; 32]]);

        // No fetch at all — only time passes beyond expiresAt.
        refresh_from_disk(&mut sh, &paths, 1001);
        assert!(sh.stale);
        assert!(sh.book_changed, "a stale flip must re-push the book");
        let part = book_part_at(&paths, 1001).unwrap();
        assert!(part.contacts.is_empty(), "stale admits nobody (D10)");
        assert_eq!(part.blocked, vec![[2; 32]], "blocks still apply when stale (D10)");

        // Steady state does not re-raise it.
        sh.book_changed = false;
        refresh_from_disk(&mut sh, &paths, 1002);
        assert!(!sh.book_changed);
    }

    /// The blocks a host would get from what's on disk, as `contacts` reads them.
    fn blocks_at(paths: &StorePaths, now: u64) -> crate::contacts::SignetBlocks {
        crate::contacts::book_with_blocks(Vec::new(), Vec::new(), read_book_part_at(paths, now)).1
    }

    #[test]
    fn damaged_or_unreadable_files_read_as_unreadable_every_time() {
        use crate::contacts::SignetBlocks;
        let valid_grant = |paths: &StorePaths| store::save_grant(&paths.grant, &grant(&"a".repeat(32))).unwrap();

        // Each way a store can go wrong, with blocks known a moment before.
        let cases: [(&str, fn(&StorePaths)); 4] = [
            ("corrupt-grant", |p| std::fs::write(&p.grant, b"{not json").unwrap()),
            ("corrupt-snapshot", |p| std::fs::write(&p.snapshot, b"[]").unwrap()),
            ("grant-read-fails", |p| {
                std::fs::remove_file(&p.grant).unwrap();
                std::fs::create_dir(&p.grant).unwrap();
            }),
            ("snapshot-read-fails", |p| {
                std::fs::remove_file(&p.snapshot).unwrap();
                std::fs::create_dir(&p.snapshot).unwrap();
            }),
        ];
        for (name, damage) in cases {
            let paths = tmp(name);
            valid_grant(&paths);
            store::save_snapshot(&paths.snapshot, &snapshot(&"a".repeat(32), 1000)).unwrap();
            assert_eq!(blocks_at(&paths, 900), SignetBlocks::Known(vec![[2; 32]]), "{name}");

            damage(&paths);
            // The read after the file was moved aside must say the same as the
            // read that moved it, and so must one after a restart.
            for attempt in 0..3 {
                assert_eq!(blocks_at(&paths, 900), SignetBlocks::Unreadable, "{name}, read {attempt}");
                assert!(
                    blocks_at(&paths, 900).for_new_host().is_err_and(|why| why.contains("Signet")),
                    "{name}: no new host on an empty list"
                );
            }

            // And Disconnect gets the player out, whatever went wrong.
            store::delete_all(&paths).unwrap();
            assert_eq!(blocks_at(&paths, 900), SignetBlocks::Known(Vec::new()), "{name}: disconnected");
            let mut sh = Shared::default();
            refresh_from_disk(&mut sh, &paths, 900);
            assert!(!sh.unreadable && !sh.connected, "{name}: the row offers Connect again");
        }
    }

    /// Decision D12, on purpose: asking Signet to go away takes its blocks with it.
    #[test]
    fn disconnect_and_revocation_still_lift_blocks() {
        use crate::contacts::SignetBlocks;
        let paths = tmp("d12");
        store::save_grant(&paths.grant, &grant(&"a".repeat(32))).unwrap();
        store::save_snapshot(&paths.snapshot, &snapshot(&"a".repeat(32), 1000)).unwrap();
        assert_eq!(blocks_at(&paths, 900), SignetBlocks::Known(vec![[2; 32]]));

        let mut revoked = snapshot(&"a".repeat(32), 1000);
        revoked.projection.revoked = true;
        store::save_snapshot(&paths.snapshot, &revoked).unwrap();
        assert_eq!(blocks_at(&paths, 900), SignetBlocks::Known(Vec::new()), "a revoked snapshot");

        store::save_snapshot(&paths.snapshot, &snapshot(&"a".repeat(32), 1000)).unwrap();
        store::delete_all(&paths).unwrap();
        assert_eq!(blocks_at(&paths, 900), SignetBlocks::Known(Vec::new()), "a disconnect");

        // And nothing connected at all is simply no blocks, not a problem.
        assert_eq!(read_book_part_at(&tmp("never"), 900), SignetRead::Absent);
    }

    #[test]
    fn going_unreadable_changes_the_book_once_and_offers_disconnect() {
        let paths = tmp("flip");
        store::save_grant(&paths.grant, &grant(&"a".repeat(32))).unwrap();
        store::save_snapshot(&paths.snapshot, &snapshot(&"a".repeat(32), 1000)).unwrap();
        let mut sh = Shared::default();
        refresh_from_disk(&mut sh, &paths, 900);
        sh.book_changed = false;

        std::fs::write(&paths.grant, b"{not json").unwrap();
        refresh_from_disk(&mut sh, &paths, 900);
        assert!(sh.unreadable && sh.book_changed);
        assert!(ui::offers_disconnect(sh.connected, sh.unreadable), "even with the grant gone");

        sh.book_changed = false;
        refresh_from_disk(&mut sh, &paths, 900);
        assert!(sh.unreadable, "still, after the file was moved aside");
        assert!(!sh.book_changed, "steady state does not re-raise it");
        assert!(sh.people == 0 && !sh.stale);
    }

    #[test]
    fn a_problem_is_warned_about_once_until_it_changes_or_clears() {
        let mut last = None;
        assert!(is_new_problem(&mut last, Some("a")));
        assert!(!is_new_problem(&mut last, Some("a")));
        assert!(is_new_problem(&mut last, Some("b")));
        assert!(!is_new_problem(&mut last, None));
        assert!(is_new_problem(&mut last, Some("b")), "one that comes back is news");
    }

    #[test]
    fn a_snapshot_from_another_grant_is_ignored() {
        let paths = tmp("mismatch");
        store::save_grant(&paths.grant, &grant(&"e".repeat(32))).unwrap();
        store::save_snapshot(&paths.snapshot, &snapshot(&"a".repeat(32), 1000)).unwrap();
        assert_eq!(book_part_at(&paths, 900), None);
        let mut sh = Shared::default();
        refresh_from_disk(&mut sh, &paths, 900);
        assert!(sh.connected);
        assert_eq!(sh.people, 0, "the row must agree with the book");
        // And with no grant at all, an orphan snapshot counts for nothing.
        std::fs::remove_file(&paths.grant).unwrap();
        assert_eq!(book_part_at(&paths, 900), None);
    }
}

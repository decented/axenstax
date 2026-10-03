//! Native lobby-mailbox — send /bug and /idea reports anonymously, and read the
//! public status board for them. Spec:
//! docs/foundations/2026-10-01-feedback-status-board.md (supersedes the reply
//! half of docs/superpowers/specs/2026-07-24-native-mailbox-design.md): nobody
//! is ever messaged. A report is sealed with a one-time burner key and carries
//! a random ticket; the makers publish a signed board of scrambled ticket
//! hashes, and `/mailbox` shows each local ticket's status.
pub mod board;
pub mod outbox;
pub mod relay;
pub mod tickets;
pub mod wire;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use nostr::prelude::*;

/// The project's feedback inbox: public third-party relays, fixed in the
/// build. This is the RECIPIENT's inbox, not a player preference — a player
/// who customises "Your relays" (`GraphicsSettings.online_relays`) still
/// reaches the project, so the mailbox deliberately ignores that list.
/// Reports are published to every relay here and the status board is read
/// from every relay here (S4). Chosen 2026-10-02 by a read-only probe
/// (`REQ {"kinds":[1059],"limit":1}`): each of these returned kind-1059
/// events with no NIP-42 auth; `relay.damus.io` closed the same request with
/// `auth-required`, so it is NOT an inbox relay (the reader could never see a
/// report there). `tools/feedback-reader/` reads the same set by default.
/// No AxeNStax-operated relay is a default anywhere (CLAUDE.md red line 2).
pub(crate) const FEEDBACK_INBOX_RELAYS: [&str; 3] =
    ["wss://nos.lol", "wss://relay.primal.net", "wss://offchain.pub"];

/// Fewest seconds between two board fetches triggered by `/mailbox` (S7).
/// The slash commands the tester gate controls (primary names).
pub const FEEDBACK_COMMANDS: [&str; 3] = ["bug", "idea", "mailbox"];

/// THE gate for tester feedback (`/bug`, `/idea`, `/mailbox`, the "Suggestion
/// Box" trial, and every hint that names them). Every entry point asks this
/// function — never `settings.tester_feedback` directly — so the rule lives in
/// exactly one place.
///
/// Today the rule is the player's own hidden unlock (Settings: tap the version
/// line 7 times; off by default). **This is where a future Signet adult/age
/// boolean gets ANDed in** (age-gate later, not now): e.g.
/// `settings.tester_feedback && signet_is_adult()`. The boolean must come from
/// third-party Signet, never a birth date we hold (CLAUDE.md red line 3).
pub fn feedback_enabled(settings: &crate::graphics_settings::GraphicsSettings) -> bool {
    settings.tester_feedback
}

/// Show or hide the feedback commands on a registry according to the gate.
/// Call at registry construction and whenever settings change. Hidden commands
/// are indistinguishable from unknown ones (see `CommandRegistry::set_hidden`).
pub fn apply_gate(
    registry: &mut crate::commands::CommandRegistry,
    settings: &crate::graphics_settings::GraphicsSettings,
) {
    let hidden = !feedback_enabled(settings);
    for name in FEEDBACK_COMMANDS {
        registry.set_hidden(name, hidden);
    }
}

const REFRESH_DEBOUNCE: Duration = Duration::from_secs(60);
/// How often the worker re-reads the board on its own (S7).
const REFRESH_EVERY: Duration = Duration::from_secs(3600);
/// How often the worker retries a queued report that could not be sent.
const FLUSH_RETRY: Duration = Duration::from_secs(300);

/// Pre-status-board files: the long-lived device key (S6: no longer used for
/// anything — deleted on launch) and the reply inbox it received into.
const LEGACY_FILES: [&str; 2] = ["mailbox_key.json", "mailbox_inbox.json"];

/// The two on-disk stores for one profile directory. Cheap to clone — the
/// worker thread keeps its own copy so it never needs to touch [`SERVICE`].
#[derive(Clone)]
pub struct Paths {
    pub outbox: PathBuf,
    pub tickets: PathBuf,
}

impl Paths {
    pub fn in_profile(dir: &Path) -> Self {
        Self {
            outbox: dir.join("mailbox_outbox.json"),
            tickets: dir.join("mailbox_tickets.json"),
        }
    }
}

/// S6: delete the old device key and reply inbox. Best-effort — a failure is
/// logged and retried on the next launch; a missing file is the normal case.
fn remove_legacy_files(dir: &Path) {
    for name in LEGACY_FILES {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => log::info!("[mailbox] removed retired {name}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("[mailbox] could not remove retired {name}: {e}"),
        }
    }
}

/// Reports an older build already sent were kept (full body) in the outbox.
/// Keep each as a sent ticket (its id may be on the board) and drop the body.
fn migrate_legacy_sent(paths: &Paths) {
    for r in outbox::drain_legacy_sent(&paths.outbox) {
        if let Err(e) =
            tickets::record_sent(&paths.tickets, &r.id, &r.kind, &r.body, r.created_at, r.created_at)
        {
            log::warn!("[mailbox] could not keep legacy ticket: {e}");
        }
    }
    // Reports an older build queued but never sent still go out under the new
    // burner-key path; give each a ticket now so /mailbox can show it
    // (record_new is idempotent, so this is safe on every launch).
    for r in outbox::queued(&paths.outbox) {
        if let Err(e) = tickets::record_new(&paths.tickets, &r.id, &r.kind, &r.body, r.created_at) {
            log::warn!("[mailbox] could not ticket a queued report: {e}");
        }
    }
}

/// Serialises ALL outbox/tickets file access — both the game thread (enqueue,
/// ticket view) and the worker thread (flush, refresh) touch the same JSON
/// files. Held only for the duration of one store call, never across relay
/// I/O or an `.await` point.
static FILE_LOCK: Mutex<()> = Mutex::new(());

/// One flush cycle: send every queued report, each sealed with its OWN fresh
/// burner key (S5 — no two reports share a seal pubkey, none carries a
/// persona). `publish` is injected so tests never touch a socket.
pub async fn flush_once(
    paths: &Paths,
    official_hex: &str,
    publish: &mut dyn FnMut(&Event) -> Result<(), String>,
) {
    let Ok(official) = PublicKey::from_hex(official_hex) else { return };

    // A poisoned FILE_LOCK degrades to "nothing queued this cycle" rather than
    // panicking the caller (game thread for enqueue_and_flush, worker thread
    // for the background flush) — see native_signin.rs / feedback_log.rs for
    // the same graceful-degradation convention on shared background state.
    let queued = match FILE_LOCK.lock() {
        Ok(_g) => outbox::queued(&paths.outbox),
        Err(_) => {
            log::warn!("[mailbox] file lock poisoned, skipping outbox flush this cycle");
            return;
        }
    };
    // Oldest first; a failure leaves the item queued (retry later).
    for report in queued {
        let burner = Keys::generate();
        let rumor = wire::build_rumor(burner.public_key(), &report);
        let wrap = match wire::wrap_report(&burner, &official, rumor).await {
            Ok(w) => w,
            Err(e) => {
                log::warn!("[mailbox] wrap failed: {e}");
                continue;
            }
        };
        if publish(&wrap).is_err() {
            break; // relay down — stop hammering, everything stays queued
        }
        match FILE_LOCK.lock() {
            Ok(_g) => {
                if let Err(e) = outbox::mark_sent(&paths.outbox, &report.id) {
                    log::warn!("[mailbox] could not mark report sent: {e}");
                }
                if let Err(e) = tickets::mark_sent(&paths.tickets, &report.id, outbox::now_secs()) {
                    log::warn!("[mailbox] could not mark ticket sent: {e}");
                }
            }
            Err(_) => log::warn!("[mailbox] file lock poisoned, could not mark report sent"),
        }
    }
}

/// One board refresh (S7): fetch, verify, match against the local tickets.
/// `fetch` is injected so tests never touch a socket. `None` = skipped because
/// there are no local tickets (nothing to look up, so no network at all);
/// `Some(n)` = fetched, `n` tickets changed status.
pub fn refresh_board(
    paths: &Paths,
    official_hex: &str,
    now: u64,
    fetch: &mut dyn FnMut() -> Vec<Event>,
) -> Option<usize> {
    let none_yet = match FILE_LOCK.lock() {
        Ok(_g) => tickets::is_empty(&paths.tickets),
        Err(_) => return None,
    };
    if none_yet {
        return None;
    }
    let events = fetch();
    let Some(board) = board::parse_board(&events, official_hex) else { return Some(0) };
    match FILE_LOCK.lock() {
        Ok(_g) => match tickets::apply_board(&paths.tickets, &board, now) {
            Ok(n) => Some(n),
            Err(e) => {
                log::warn!("[mailbox] could not store board status: {e}");
                Some(0)
            }
        },
        Err(_) => Some(0),
    }
}

/// Run a `Send + 'static` future to completion on a brand-new OS thread that
/// owns its own current-thread tokio runtime, blocking the caller via
/// `JoinHandle::join` (a plain std primitive) rather than `Runtime::block_on`.
///
/// Why not just call `.block_on` again on some shared runtime from inside the
/// worker's closures? Because `flush_once`'s `publish` closure runs
/// SYNCHRONOUSLY from inside the worker's own `rt.block_on(flush_once(...))`
/// — i.e. the calling OS thread is already "entered" into a tokio runtime.
/// Tokio's entered-context guard is a per-OS-thread flag, not a per-`Runtime`-
/// instance one, so calling `block_on` again on that same thread panics
/// ("Cannot start a runtime from within a runtime") even against a second,
/// independent `Runtime`. Spawning a fresh thread sidesteps the problem
/// entirely: that thread has never entered any runtime, so its `block_on` is
/// the first and only one, and the worker thread's wait for the result is a
/// plain (non-tokio) blocking join — legal from inside another runtime's poll,
/// and harmless here because the worker runtime has nothing else to schedule
/// while it waits. Mirrors the "spawn a thread, build a current-thread
/// runtime, block_on" idiom `game_loop::native_join_sign_driver` already uses
/// for the same sync-call-site/async-body shape.
///
/// Both call sites produce a `Result<_, String>`, so `T` is pinned to that
/// shape here: if the spawned IO thread itself panics, `join()` fails and we
/// degrade to `Err(..)` (logging a warning) rather than re-panicking — a bad
/// relay round-trip should drop this cycle, not permanently kill the worker.
fn run_on_worker_thread<F, T>(fut: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let joined = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("mailbox io runtime: {e}"))?;
        rt.block_on(fut)
    })
    .join();
    match joined {
        Ok(result) => result,
        Err(_) => {
            log::warn!("[mailbox] relay io thread panicked");
            Err("relay io thread panicked".to_string())
        }
    }
}

/// Every relay the board is read from (S4): the project's inbox relays.
fn board_relays() -> Vec<String> {
    FEEDBACK_INBOX_RELAYS.iter().map(|s| s.to_string()).collect()
}

/// Publish `ev` to every inbox relay at once. `Ok` when at least one relay
/// accepted it — the report reached the project; it stays queued (retried
/// later) only when every inbox relay refused or was unreachable.
async fn publish_to_inbox(ev: &Event) -> Result<(), String> {
    let results = futures_util::future::join_all(
        FEEDBACK_INBOX_RELAYS.iter().map(|u| relay::publish(u, ev)),
    )
    .await;
    if results.iter().any(Result::is_ok) {
        return Ok(());
    }
    Err(results
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>()
        .join("; "))
}

/// Query every board relay at once; a relay that fails or times out simply
/// contributes nothing.
fn fetch_board_events() -> Vec<Event> {
    let filter = board::board_filter(wire::OFFICIAL_AXENSTAX_PUBKEY_HEX);
    let urls = board_relays();
    let fetched = run_on_worker_thread(async move {
        let results = futures_util::future::join_all(
            urls.iter().map(|u| relay::fetch_events(u, &filter)),
        )
        .await;
        Ok(results.into_iter().filter_map(Result::ok).flatten().collect::<Vec<Event>>())
    });
    fetched.unwrap_or_default()
}

/// One flush cycle against the real relay. Runs on the worker's persistent
/// current-thread runtime.
fn run_flush_cycle(rt: &tokio::runtime::Runtime, paths: &Paths) {
    let mut publish = |ev: &Event| -> Result<(), String> {
        let ev = ev.clone();
        run_on_worker_thread(async move { publish_to_inbox(&ev).await })
    };
    rt.block_on(flush_once(paths, wire::OFFICIAL_AXENSTAX_PUBKEY_HEX, &mut publish));
}

/// Whether enough time has passed since the last fetch (`None` = never).
fn due(since_last: Option<Duration>, min: Duration) -> bool {
    since_last.is_none_or(|d| d >= min)
}

/// What wakes the worker early.
enum Msg {
    /// A report was just queued.
    Flush,
    /// `/mailbox` opened: re-read the board (debounced).
    Refresh,
}

/// The running service: where the stores are, and a channel to wake the worker.
struct Service {
    paths: Paths,
    tx: Sender<Msg>,
}

static SERVICE: RwLock<Option<Service>> = RwLock::new(None);

fn worker_loop(paths: Paths, rx: Receiver<Msg>) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        log::warn!("[mailbox] worker runtime failed to start");
        return;
    };
    let mut last_fetch: Option<Instant> = None;
    let refresh = |last_fetch: &mut Option<Instant>| {
        let now = outbox::now_secs();
        if refresh_board(&paths, wire::OFFICIAL_AXENSTAX_PUBKEY_HEX, now, &mut fetch_board_events)
            .is_some()
        {
            *last_fetch = Some(Instant::now());
        }
    };
    // Boot: catch up on anything queued, then read the board.
    run_flush_cycle(&rt, &paths);
    refresh(&mut last_fetch);
    loop {
        let msg = rx.recv_timeout(FLUSH_RETRY);
        let since = last_fetch.map(|t| t.elapsed());
        match msg {
            Ok(Msg::Flush) => run_flush_cycle(&rt, &paths),
            Ok(Msg::Refresh) => {
                if due(since, REFRESH_DEBOUNCE) {
                    refresh(&mut last_fetch);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                run_flush_cycle(&rt, &paths);
                if due(since, REFRESH_EVERY) {
                    refresh(&mut last_fetch);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Start the background worker for `profile_dir`: a dedicated thread with its
/// own current-thread tokio runtime, mirroring
/// `game_loop::native_join_sign_driver`. Retires the old device key and reply
/// inbox (S6), then flushes + reads the board once, and thereafter wakes on a
/// new report, on `/mailbox`, or every 5 minutes (board: hourly). No-op if a
/// service is already running.
pub fn init(profile_dir: &Path) {
    let mut guard = match SERVICE.write() {
        Ok(g) => g,
        Err(_) => {
            log::warn!("[mailbox] service lock poisoned, skipping init");
            return;
        }
    };
    if guard.is_some() {
        return;
    }
    remove_legacy_files(profile_dir);
    let paths = Paths::in_profile(profile_dir);
    match FILE_LOCK.lock() {
        Ok(_g) => {
            migrate_legacy_sent(&paths);
            tickets::prune(&paths.tickets, outbox::now_secs());
        }
        Err(_) => log::warn!("[mailbox] file lock poisoned, skipping legacy migration"),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    let worker_paths = paths.clone();
    std::thread::spawn(move || worker_loop(worker_paths, rx));
    *guard = Some(Service { paths, tx });
}

/// Queue a report (and its local ticket) and nudge the worker to flush right
/// away (best-effort — a dropped wake just means the next 5-minute retry picks
/// it up).
pub fn enqueue_and_flush(kind: &str, body: &str) -> Result<(), String> {
    let guard = SERVICE
        .read()
        .map_err(|_| "mailbox service lock poisoned".to_string())?;
    let service = guard.as_ref().ok_or("mailbox service not initialised")?;
    {
        let _g = FILE_LOCK
            .lock()
            .map_err(|_| "mailbox file lock poisoned".to_string())?;
        let rec = outbox::enqueue(&service.paths.outbox, kind, body)?;
        tickets::record_new(&service.paths.tickets, &rec.id, kind, body, rec.created_at)?;
    }
    let _ = service.tx.send(Msg::Flush);
    Ok(())
}

/// `/mailbox` opened: ask the worker to re-read the board (it debounces to one
/// fetch per 60 s). Never blocks.
pub fn request_refresh() {
    let Ok(guard) = SERVICE.read() else { return };
    if let Some(service) = guard.as_ref() {
        let _ = service.tx.send(Msg::Refresh);
    }
}

/// The `/mailbox` view: one chat-ready line per local ticket, newest first.
pub fn ticket_lines() -> Vec<String> {
    let Ok(guard) = SERVICE.read() else { return Vec::new() };
    let Some(service) = guard.as_ref() else { return Vec::new() };
    let Ok(_g) = FILE_LOCK.lock() else { return Vec::new() };
    tickets::view_lines(&service.paths.tickets)
}

/// Serialises every test that touches the [`SERVICE`] global. `reset_for_test`
/// acquires this and hands the guard back to the caller, who must hold it for
/// the test's ENTIRE body (bind it to `_guard`, not `_`) — otherwise two tests
/// racing on separate threads can each reset `SERVICE` to their own temp dir
/// mid-flight and read/write each other's store. Poison-tolerant: one
/// panicking test must not cascade-fail every other test that shares this
/// lock.
#[cfg(test)]
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Test-only: re-point the global service at a temp profile dir WITHOUT
/// starting a worker thread (the dummy `tx`'s receiver is dropped, so
/// `enqueue_and_flush`'s `send` fails silently — exactly what a test wants).
///
/// Returns a guard on [`TEST_LOCK`] that the caller must keep alive (`let
/// _guard = reset_for_test(&dir);`) for as long as the test relies on
/// `SERVICE` pointing at `profile_dir` — see the lock's doc comment for why.
#[cfg(test)]
pub fn reset_for_test(profile_dir: &Path) -> std::sync::MutexGuard<'static, ()> {
    let guard = TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let paths = Paths::in_profile(profile_dir);
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    drop(rx);
    let mut service_guard = SERVICE.write().unwrap();
    *service_guard = Some(Service { paths, tx });
    guard
}

#[cfg(test)]
mod tests {
    // `nostr::prelude::*` reaches this module already via `super::*` (mod.rs's
    // own top-level `use nostr::prelude::*;`).
    use super::*;
    use board::ticket_key;

    fn tmp_paths(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("axemb-svc-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Paths::in_profile(&dir)
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    /// Queue a report the way `enqueue_and_flush` does (store + ticket).
    fn queue(paths: &Paths, kind: &str, body: &str) -> outbox::QueuedReport {
        let rec = outbox::enqueue(&paths.outbox, kind, body).unwrap();
        tickets::record_new(&paths.tickets, &rec.id, kind, body, rec.created_at).unwrap();
        rec
    }

    async fn board_event(keys: &Keys, content: String, at: u64) -> Event {
        EventBuilder::new(Kind::from(30078u16), content)
            .tags([Tag::identifier(board::BOARD_D)])
            .custom_created_at(Timestamp::from_secs(at))
            .sign(keys)
            .await
            .unwrap()
    }

    // ─── Tester gate (2026-10-03) ───

    fn gated_registry(on: bool) -> crate::commands::CommandRegistry {
        let mut r = crate::commands::CommandRegistry::new();
        crate::commands::builtins::register_all(&mut r);
        let settings = crate::graphics_settings::GraphicsSettings {
            tester_feedback: on,
            ..Default::default()
        };
        apply_gate(&mut r, &settings);
        r
    }

    /// Run one chat line through the real dispatcher; return its log lines.
    fn chat(registry: &crate::commands::CommandRegistry, line: &str) -> Vec<String> {
        use crate::commands::{dispatch, CommandContext, OpLevel};
        let mut world = crate::world::World::new();
        let (mut t, mut s, mut creative) = (0u32, 4u32, false);
        let mut mode = crate::play_mode::PlayMode::Survival;
        let mut players = vec![crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let (mut ch, mut ev, mut ps) = (false, false, false);
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative,
            play_mode: &mut mode,
            seed: 1,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::None,
            current_tick: 0,
            log: &mut log,
            registry,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        dispatch(line, &mut ctx, registry);
        log.iter().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn the_gate_follows_the_setting_and_is_off_by_default() {
        let mut s = crate::graphics_settings::GraphicsSettings::default();
        assert!(!feedback_enabled(&s), "off by default");
        s.tester_feedback = true;
        assert!(feedback_enabled(&s));
    }

    #[test]
    fn a_fresh_registry_with_no_gate_applied_shows_no_feedback_commands() {
        let mut r = crate::commands::CommandRegistry::new();
        crate::commands::builtins::register_all(&mut r);
        for name in FEEDBACK_COMMANDS {
            assert!(r.lookup(name).is_none(), "/{name} must fail closed");
        }
    }

    #[test]
    fn off_the_feedback_commands_answer_exactly_like_unknown_commands() {
        let r = gated_registry(false);
        for name in FEEDBACK_COMMANDS {
            let got = chat(&r, &format!("/{name} something broke"));
            let unknown = chat(&r, "/zzznotacommand something broke");
            let swap = |v: Vec<String>, n: &str| -> Vec<String> {
                v.into_iter().map(|l| l.replace(n, "NAME")).collect()
            };
            assert_eq!(
                swap(got, name),
                swap(unknown, "zzznotacommand"),
                "/{name} must be indistinguishable from an unknown command"
            );
            assert!(chat(&r, &format!("/{name}")).iter().any(|l| l.contains("unknown command")));
            // `/help <name>` agrees.
            assert!(chat(&r, &format!("/help {name}")).iter().any(|l| l.contains("unknown command")));
        }
    }

    #[test]
    fn off_help_does_not_list_the_feedback_commands() {
        let help = chat(&gated_registry(false), "/help").join("\n");
        assert!(help.contains("Available commands"));
        for name in FEEDBACK_COMMANDS {
            assert!(!help.contains(&format!("/{name} ")), "/help must not list /{name}");
        }
        assert!(!help.contains("makers"), "no feedback wording in /help: {help}");
    }

    #[test]
    fn on_the_feedback_commands_are_listed_and_work() {
        let dir = std::env::temp_dir().join(format!("axemb-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = reset_for_test(&dir);
        let r = gated_registry(true);
        let help = chat(&r, "/help").join("\n");
        for name in FEEDBACK_COMMANDS {
            assert!(help.contains(&format!("/{name} ")), "/help lists /{name}: {help}");
        }
        let out = chat(&r, "/bug the sky is square");
        assert!(out.iter().any(|l| l.contains("Queued")), "{out:?}");
        assert!(!outbox::queued(&Paths::in_profile(&dir).outbox).is_empty());
    }

    #[test]
    fn off_nothing_new_is_queued() {
        let dir = std::env::temp_dir().join(format!("axemb-gate-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = reset_for_test(&dir);
        let r = gated_registry(false);
        chat(&r, "/bug should not be queued");
        chat(&r, "/idea nor this");
        assert!(outbox::queued(&Paths::in_profile(&dir).outbox).is_empty());
    }

    #[test]
    fn each_report_is_sealed_with_its_own_burner_and_no_persona() {
        rt().block_on(async {
            let paths = tmp_paths("burner");
            let maker = Keys::generate(); // stands in for the official key
            queue(&paths, "bug", "report one");
            queue(&paths, "idea", "report two");

            let mut published: Vec<Event> = Vec::new();
            flush_once(&paths, &maker.public_key().to_hex(), &mut |ev| {
                published.push(ev.clone());
                Ok(())
            })
            .await;

            assert_eq!(published.len(), 2, "both queued reports published");
            assert!(published.iter().all(|w| w.kind.as_u16() == 1059));
            assert!(outbox::queued(&paths.outbox).is_empty(), "both marked sent");

            let mut seal_authors = Vec::new();
            for wrap in &published {
                let u = nostr::nips::nip59::extract_rumor(&maker, wrap).await.unwrap();
                assert!(
                    !u.rumor.tags.iter().any(|t| {
                        matches!(t.as_slice().first().map(String::as_str), Some("persona" | "handle"))
                    }),
                    "no identity tag on the rumor"
                );
                seal_authors.push(u.sender);
            }
            assert_ne!(seal_authors[0], seal_authors[1], "two reports, two different seal pubkeys");

            // Tickets are now marked sent, and the sent bodies are not retained.
            let lines = tickets::view_lines(&paths.tickets);
            assert_eq!(lines.len(), 2);
            assert!(lines.iter().all(|l| l.ends_with("Sent")), "got {lines:?}");
            let outbox_text = std::fs::read_to_string(&paths.outbox).unwrap_or_default();
            assert!(!outbox_text.contains("report one"));
        });
    }

    #[test]
    fn publish_failure_leaves_report_queued_and_ticket_unsent() {
        rt().block_on(async {
            let paths = tmp_paths("fail");
            queue(&paths, "idea", "keep me");
            flush_once(&paths, wire::OFFICIAL_AXENSTAX_PUBKEY_HEX, &mut |_| Err("relay down".into()))
                .await;
            assert_eq!(outbox::queued(&paths.outbox).len(), 1, "failure stays queued");
            let lines = tickets::view_lines(&paths.tickets);
            assert!(lines[0].ends_with("Waiting to send"), "got {lines:?}");
        });
    }

    #[test]
    fn refresh_applies_the_official_board_and_ignores_a_stranger() {
        rt().block_on(async {
            let paths = tmp_paths("refresh");
            let official = Keys::generate();
            let stranger = Keys::generate();
            let hex = official.public_key().to_hex();
            let mine = queue(&paths, "bug", "doors too tall");
            let key = ticket_key(&mine.id);
            let body = |s: &str, v: &str| format!(r#"{{"v":1,"t":{{"{key}":{{"s":"{s}","v":"{v}"}}}}}}"#);

            // A stranger's newer "fixed" board is ignored; no change.
            let evil = board_event(&stranger, body("fixed", "9.9.9"), 5000).await;
            assert_eq!(refresh_board(&paths, &hex, 6000, &mut || vec![evil.clone()]), Some(0));
            assert!(tickets::view_lines(&paths.tickets)[0].ends_with("Waiting to send"));

            // The official board lands.
            let real = board_event(&official, body("fixed", "0.2.28"), 4000).await;
            assert_eq!(refresh_board(&paths, &hex, 6000, &mut || vec![evil.clone(), real.clone()]), Some(1));
            let lines = tickets::view_lines(&paths.tickets);
            assert!(lines[0].ends_with("Fixed in v0.2.28"), "got {lines:?}");
        });
    }

    #[test]
    fn refresh_with_no_tickets_does_no_network() {
        let paths = tmp_paths("noticket");
        let got = refresh_board(&paths, wire::OFFICIAL_AXENSTAX_PUBKEY_HEX, 1, &mut || {
            panic!("must not fetch when there is nothing to look up")
        });
        assert_eq!(got, None);
    }

    #[test]
    fn retired_device_key_and_inbox_are_deleted_and_old_sent_reports_kept_as_tickets() {
        let paths = tmp_paths("legacy");
        let dir = paths.outbox.parent().unwrap().to_path_buf();
        for name in LEGACY_FILES {
            std::fs::write(dir.join(name), b"{\"secret_hex\":\"00\"}").unwrap();
        }
        std::fs::write(
            &paths.outbox,
            br#"[{"id":"aa","kind":"bug","body":"old bug","persona_hex":"cd","created_at":5,"status":"sent","event_id":"ee"}]"#,
        )
        .unwrap();

        remove_legacy_files(&dir);
        remove_legacy_files(&dir); // idempotent: second launch finds nothing
        for name in LEGACY_FILES {
            assert!(!dir.join(name).exists(), "{name} deleted");
        }

        migrate_legacy_sent(&paths);
        let lines = tickets::view_lines(&paths.tickets);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("old bug") && lines[0].ends_with("Sent"), "got {lines:?}");
        assert!(!std::fs::read_to_string(&paths.outbox).unwrap().contains("old bug"));
    }

    #[test]
    fn refresh_is_debounced_to_one_per_minute() {
        assert!(due(None, REFRESH_DEBOUNCE), "never fetched ⇒ due");
        assert!(!due(Some(Duration::from_secs(59)), REFRESH_DEBOUNCE));
        assert!(due(Some(Duration::from_secs(60)), REFRESH_DEBOUNCE));
        assert!(!due(Some(Duration::from_secs(3599)), REFRESH_EVERY));
        assert!(due(Some(Duration::from_secs(3600)), REFRESH_EVERY));
    }
}

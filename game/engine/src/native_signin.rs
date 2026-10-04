//! Native Signet sign-in **driver** — runs the NIP-46 pairing handshake off the
//! UI thread and exposes a simple status the lobby dialog polls each frame.
//!
//! NATIVE-ONLY. The egui menu loop is synchronous; the bunker handshake is async
//! (relay I/O + a human approving on their phone). So a sign-in attempt spawns a
//! worker thread with its own current-thread tokio runtime, drives the handshake
//! to completion, and reports progress back through a channel. The menu calls
//! [`poll`] every frame and renders [`status`].
//!
//! Two flows (the user picked "both"):
//!   - **QR** ([`start_qr`]): the desktop generates a `nostrconnect://` URI, shows
//!     it as a QR, and the phone scans it — the smooth phone→desktop path.
//!   - **Paste** ([`start_paste`]): the user pastes a `bunker://` link from Signet.
//!
//! Producing a signature is the **online** step (and the live handshake needs a
//! real phone — the owner/device boundary). Local play never reaches this code.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use signet_nip46_client::{BunkerSession, SessionOptions};

use crate::signet::native_signer::{self, JoinAuthError, NativeIdentity};

/// How many of the player's relays go into the `nostrconnect://` URI. Each
/// relay adds ~40 characters; three keeps the QR comfortably scannable while
/// still giving the phone failover candidates. mySignet reads every `relay=`
/// param in order and answers on the first one it can reach
/// (`signet-app/src/lib/nip46.ts` parseNostrConnectURI → `relayUrls`), and the
/// handshake here listens on all of them.
pub(crate) const SIGNIN_URI_MAX_RELAYS: usize = 3;
/// Shown to the signer (phone) during approval.
const APP_NAME: &str = "Axe'n'Stax";

/// What the sign-in flow is doing right now — drives the dialog UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInStatus {
    /// Nothing in progress.
    Idle,
    /// QR shown; waiting for the phone to scan + connect. Carries the
    /// `nostrconnect://` URI to render as a QR.
    AwaitingScan { uri: String },
    /// Connected / paste submitted; waiting for the signer to approve.
    AwaitingApproval,
    /// Signed in. Carries the npub for display.
    Success { npub: String },
    /// Failed — carries a short human message.
    Failed { message: String },
}

/// Progress events sent from the worker thread to [`poll`]. Each carries the
/// generation of the attempt that produced it, so [`poll`] can drop a result
/// from an attempt the player has since cancelled or replaced.
enum SignInEvent {
    Done {
        generation: u64,
        result: Result<String /* pubkey hex */, JoinAuthError>,
    },
}

struct Manager {
    status: SignInStatus,
    rx: Option<Receiver<SignInEvent>>,
    /// Which attempt `status`/`rx` belong to. Bumped by EVERY state change that
    /// abandons the current attempt — [`reset`] (Cancel, "Try again", relay
    /// edit, sign-out), a new worker, a synchronous failure, the preview hook.
    /// A worker captures the value it was started under and may only persist a
    /// sign-in while it is still current (see [`commit_if_current`]).
    generation: u64,
}

static MANAGER: Mutex<Manager> = Mutex::new(Manager {
    status: SignInStatus::Idle,
    rx: None,
    generation: 0,
});

/// How often a worker blocked in the handshake re-checks whether it has been
/// cancelled. Bounds how long a cancelled QR keeps listening on the relays.
const CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// The current sign-in status (clone — cheap, drives the dialog).
pub fn status() -> SignInStatus {
    MANAGER.lock().map(|m| m.status.clone()).unwrap_or(SignInStatus::Idle)
}

/// Reset to idle (the dialog closed / cancelled / "Try again"). Bumps the
/// generation, so a worker still in flight for the abandoned attempt stops
/// listening within [`CANCEL_POLL`] and can no longer persist a sign-in.
pub fn reset() {
    if let Ok(mut m) = MANAGER.lock() {
        m.generation = m.generation.wrapping_add(1);
        m.status = SignInStatus::Idle;
        m.rx = None;
    }
}

/// True while `generation` is still the live attempt. A poisoned lock counts as
/// stale (fail closed: never persist on doubt).
fn is_current(generation: u64) -> bool {
    MANAGER.lock().map(|m| m.generation == generation).unwrap_or(false)
}

/// Resolves once `generation` has been superseded (polled every
/// [`CANCEL_POLL`]). Raced against the handshake so a cancelled attempt drops
/// its relay subscription instead of waiting out the handshake timeout.
async fn superseded(generation: u64) {
    while is_current(generation) {
        tokio::time::sleep(CANCEL_POLL).await;
    }
}

/// Run `commit` (the disk write that makes the sign-in real) only if
/// `generation` is still the live attempt, holding the manager lock across
/// the check AND the commit. [`reset`] / a new attempt take the same lock to
/// bump the generation, so the two are totally ordered: either the commit
/// finishes first (the player approved on their phone before cancelling — the
/// sign-in stands), or the bump comes first and nothing is persisted. Returns
/// `None` when stale. `commit` must be synchronous: no `.await` may run while
/// the guard is held.
fn commit_if_current<F>(generation: u64, commit: F) -> Option<Result<(), String>>
where
    F: FnOnce() -> Result<(), String>,
{
    let m = MANAGER.lock().ok()?;
    if m.generation != generation {
        return None;
    }
    let out = commit();
    drop(m);
    Some(out)
}

/// The worker's last step once the handshake has produced a pubkey: commit the
/// sign-in if this attempt is still current, then report. A stale attempt
/// persists nothing and reports nothing. Split out so the guard is testable
/// without a live bunker.
fn conclude<F>(generation: u64, tx: &Sender<SignInEvent>, pubkey_hex: String, commit: F)
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let result = match commit_if_current(generation, || commit(&pubkey_hex)) {
        None => return,
        Some(Ok(())) => Ok(pubkey_hex),
        Some(Err(e)) => Err(JoinAuthError::Other(e)),
    };
    let _ = tx.send(SignInEvent::Done { generation, result });
}

/// The relays the sign-in QR advertises: the first [`SIGNIN_URI_MAX_RELAYS`] of
/// the player's "Your relays" list, in order. Falls back to the public
/// defaults only if the list is somehow empty (settings keep it non-empty).
pub(crate) fn signin_relays(player_relays: &[String]) -> Vec<String> {
    let src: Vec<String> = if player_relays.is_empty() {
        crate::server_resolve::public_default_relays()
    } else {
        player_relays.to_vec()
    };
    src.into_iter().take(SIGNIN_URI_MAX_RELAYS).collect()
}

/// Build the `nostrconnect://` URI + pending session for `player_relays`. No
/// network: the handshake only starts when the worker drives the session.
fn build_nostrconnect(
    player_relays: &[String],
) -> Result<(String, BunkerSession), signet_nip46_client::Error> {
    BunkerSession::pair_nostrconnect(signin_relays(player_relays), APP_NAME, SessionOptions::default())
}

/// Begin the **QR** (client-initiated) flow, advertising the player's own
/// relays (`GraphicsSettings.online_relays`).
pub fn start_qr(player_relays: &[String]) {
    let built = build_nostrconnect(player_relays);
    match built {
        Ok((uri, session)) => spawn_worker(session, SignInStatus::AwaitingScan { uri }),
        Err(e) => set_failed(native_signer::map_signer_error(&e)),
    }
}

/// Begin the **paste** (`bunker://`) flow with a user-supplied URI.
pub fn start_paste(bunker_uri: &str) {
    let uri = bunker_uri.trim();
    if uri.is_empty() {
        set_failed(JoinAuthError::Other("paste a bunker:// link first".to_string()));
        return;
    }
    match native_signer::pair(uri) {
        Ok(session) => spawn_worker(session, SignInStatus::AwaitingApproval),
        Err(e) => set_failed(e),
    }
}

/// Drain the worker channel and advance `status`. Call every menu frame.
pub fn poll() {
    let mut m = match MANAGER.lock() {
        Ok(m) => m,
        Err(_) => return,
    };
    let mut events = Vec::new();
    if let Some(rx) = &m.rx {
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
    }
    let current = m.generation;
    for ev in events {
        let SignInEvent::Done { generation, result } = ev;
        if generation != current {
            // From an attempt that was cancelled or replaced — never let it
            // flip the dialog (the worker's commit guard already kept it off
            // disk).
            continue;
        }
        match result {
            Ok(pubkey_hex) => {
                let npub = NativeIdentity::SignedIn { pubkey_hex }
                    .npub()
                    .unwrap_or_default();
                m.status = SignInStatus::Success { npub };
                m.rx = None;
            }
            Err(e) => {
                m.status = SignInStatus::Failed { message: e.to_string() };
                m.rx = None;
            }
        }
    }
}

/// Inject an `AwaitingScan` status (with a sample URI) for a headless
/// screenshot / UX preview of the QR dialog, WITHOUT starting a live handshake
/// or any network. Dev tooling only (`--shot-signin`).
pub fn preview_awaiting_scan(uri: String) {
    if let Ok(mut m) = MANAGER.lock() {
        m.generation = m.generation.wrapping_add(1);
        m.status = SignInStatus::AwaitingScan { uri };
        m.rx = None;
    }
}

fn set_failed(e: JoinAuthError) {
    if let Ok(mut m) = MANAGER.lock() {
        m.generation = m.generation.wrapping_add(1);
        m.status = SignInStatus::Failed { message: e.to_string() };
        m.rx = None;
    }
}

/// Start a new attempt: bump the generation (orphaning any older worker),
/// show `initial`, and hand back the generation + the worker's sender.
fn begin_attempt(initial: SignInStatus) -> Option<(u64, Sender<SignInEvent>)> {
    let (tx, rx): (Sender<SignInEvent>, Receiver<SignInEvent>) = channel();
    let mut m = MANAGER.lock().ok()?;
    m.generation = m.generation.wrapping_add(1);
    m.status = initial;
    m.rx = Some(rx);
    Some((m.generation, tx))
}

/// Move the constructed session onto a worker thread (own current-thread tokio
/// runtime) and drive `connect → approve → persist` to completion, reporting
/// through the channel.
///
/// Cancellation (T0-8): the worker is tied to the generation it started under.
/// It races the handshake against [`superseded`] and bails within
/// [`CANCEL_POLL`] of a reset, and its disk write goes through
/// [`commit_if_current`], so a cancelled or replaced QR can never sign the
/// player in. What remains: (a) if the commit wins the lock an instant before
/// Cancel, the sign-in the player approved on their phone stands (the lobby
/// reads identity from disk, so it shows as signed in; only the dialog's
/// "Signed in" confirmation is skipped); (b) for up to one [`CANCEL_POLL`]
/// after Cancel the old QR's relay subscription is still open, so a scan in
/// that window can still raise an approval prompt on the phone — approving it
/// persists nothing.
fn spawn_worker(session: BunkerSession, initial: SignInStatus) {
    let Some((generation, tx)) = begin_attempt(initial) else { return };
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let result = Err(JoinAuthError::Other(e.to_string()));
                let _ = tx.send(SignInEvent::Done { generation, result });
                return;
            }
        };
        rt.block_on(async move {
            // Keep whatever was shown (the QR for the scan flow, "approve on your
            // phone" for the paste flow) on screen UNTIL the handshake genuinely
            // finishes. `user_public_key()` drives the whole live step — connect +
            // phone approval — so there is no real "now connected, awaiting
            // approval" signal to surface mid-way. Previously an optimistic
            // `Approving` was sent right here, which flipped the QR screen to
            // "Connected — approve on your phone" within milliseconds — before any
            // phone had scanned — so the QR was never actually usable (the
            // "no QR, only paste" + "stuck on approve" reports).

            // Resolving the persona pubkey completes the relay handshake and
            // triggers the phone approval prompt (the live step). Raced against
            // cancellation; the session is shut down AFTER the select so the
            // borrow held by the handshake future has ended.
            let handshake = tokio::select! {
                r = session.user_public_key() => Some(r),
                () = superseded(generation) => None,
            };
            let pubkey = match handshake {
                None => {
                    session.shutdown().await;
                    return;
                }
                Some(Ok(pk)) => pk,
                Some(Err(e)) => {
                    let result = Err(native_signer::map_signer_error(&e));
                    let _ = tx.send(SignInEvent::Done { generation, result });
                    return;
                }
            };
            // Build the session record for silent reconnection on later
            // launches (no disk I/O here — the write is in the commit below).
            let persisted = match session.persist().await {
                Ok(p) => p,
                Err(e) => {
                    let result = Err(native_signer::map_signer_error(&e));
                    let _ = tx.send(SignInEvent::Done { generation, result });
                    return;
                }
            };
            conclude(generation, &tx, pubkey.to_hex(), |hex| {
                native_signer::complete_sign_in(&persisted, hex).map(|_| ())
            });
            session.shutdown().await;
        });
    });
}

/// Sign out: clear the stored session + cached identity + owner, and reset the
/// dialog state.
pub fn sign_out() {
    let _ = native_signer::sign_out();
    reset();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Serialises the tests that drive the process-global [`MANAGER`] (cargo
    /// runs a module's tests in parallel).
    static MANAGER_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn manager_lock() -> std::sync::MutexGuard<'static, ()> {
        MANAGER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// x-only secp256k1 generator — a valid pubkey for the npub render.
    const PK_HEX: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    // Exercises the synchronous status transitions with NO network: bad/empty
    // input fails before any worker thread spawns, and reset/poll are inert at
    // Idle. (The live handshake itself is the owner/device boundary.)
    #[test]
    fn status_lifecycle_without_network() {
        let _g = manager_lock();
        reset();
        assert_eq!(status(), SignInStatus::Idle);

        // Empty paste fails immediately (no thread, no relay).
        start_paste("");
        assert!(matches!(status(), SignInStatus::Failed { .. }));

        reset();
        assert_eq!(status(), SignInStatus::Idle);

        // A malformed bunker URI fails at construction, still synchronously.
        start_paste("not-a-bunker-uri");
        assert!(matches!(status(), SignInStatus::Failed { .. }));

        reset();
        assert_eq!(status(), SignInStatus::Idle);

        // poll() with no in-flight worker is a no-op.
        poll();
        assert_eq!(status(), SignInStatus::Idle);
    }

    /// T0-8 — Cancel / "Try again" after a QR is shown: the old worker's
    /// handshake later completes, but it must persist nothing and report
    /// nothing.
    #[test]
    fn stale_worker_after_reset_persists_nothing_and_is_discarded() {
        let _g = manager_lock();
        let (generation, tx) =
            begin_attempt(SignInStatus::AwaitingScan { uri: "nostrconnect://x".into() }).unwrap();
        assert!(is_current(generation));
        reset(); // the player cancelled
        assert!(!is_current(generation));

        let persisted = AtomicBool::new(false);
        conclude(generation, &tx, PK_HEX.to_string(), |_| {
            persisted.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(!persisted.load(Ordering::SeqCst), "a cancelled attempt must not persist");
        poll();
        assert_eq!(status(), SignInStatus::Idle, "a cancelled attempt must not sign in");

        // "Try again" started a NEW attempt; the old worker finishing late must
        // neither persist nor flip the new attempt's dialog.
        let (old_gen, _old_tx) = begin_attempt(SignInStatus::AwaitingApproval).unwrap();
        let (new_gen, new_tx) =
            begin_attempt(SignInStatus::AwaitingScan { uri: "nostrconnect://y".into() }).unwrap();
        assert_ne!(old_gen, new_gen);
        conclude(old_gen, &new_tx, PK_HEX.to_string(), |_| {
            persisted.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(!persisted.load(Ordering::SeqCst));
        // Consumer side: even a stale-tagged event that reaches the live
        // channel is dropped.
        let _ = new_tx.send(SignInEvent::Done { generation: old_gen, result: Ok(PK_HEX.into()) });
        poll();
        assert!(
            matches!(status(), SignInStatus::AwaitingScan { .. }),
            "stale result must not touch the current attempt: {:?}",
            status()
        );
        reset();
    }

    /// The guard must not break the happy path: the current attempt commits
    /// and the dialog shows success.
    #[test]
    fn current_attempt_still_commits_and_succeeds() {
        let _g = manager_lock();
        let (generation, tx) =
            begin_attempt(SignInStatus::AwaitingScan { uri: "nostrconnect://z".into() }).unwrap();
        let persisted = AtomicBool::new(false);
        conclude(generation, &tx, PK_HEX.to_string(), |hex| {
            assert_eq!(hex, PK_HEX);
            persisted.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(persisted.load(Ordering::SeqCst));
        poll();
        assert!(matches!(status(), SignInStatus::Success { .. }), "{:?}", status());

        // A failing commit on the current attempt surfaces as Failed.
        let (generation, tx) = begin_attempt(SignInStatus::AwaitingApproval).unwrap();
        conclude(generation, &tx, PK_HEX.to_string(), |_| Err("disk full".into()));
        poll();
        assert!(matches!(status(), SignInStatus::Failed { .. }), "{:?}", status());
        reset();
    }

    /// The cancellation watcher resolves once the attempt is superseded.
    #[test]
    fn superseded_resolves_after_reset() {
        let _g = manager_lock();
        let (generation, _tx) = begin_attempt(SignInStatus::AwaitingApproval).unwrap();
        reset();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), superseded(generation))
                .await
                .expect("superseded() must resolve for a reset attempt");
        });
    }

    /// Every `relay=` value in a built URI, still percent-encoded.
    fn raw_relay_params(uri: &str) -> Vec<String> {
        let query = uri.split_once('?').map(|(_, q)| q).unwrap_or("");
        query
            .split('&')
            .filter_map(|kv| kv.strip_prefix("relay="))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn nostrconnect_uri_with_one_relay_carries_exactly_it() {
        let (uri, _session) = build_nostrconnect(&["wss://relay.example.com".to_string()]).unwrap();
        assert!(uri.starts_with("nostrconnect://"), "{uri}");
        assert_eq!(raw_relay_params(&uri), vec!["wss%3A%2F%2Frelay.example.com".to_string()]);
    }

    #[test]
    fn nostrconnect_uri_takes_the_first_three_of_five_relays_percent_encoded() {
        let five: Vec<String> =
            (1..=5).map(|i| format!("wss://r{i}.example.com")).collect();
        let (uri, _session) = build_nostrconnect(&five).unwrap();
        assert_eq!(
            raw_relay_params(&uri),
            vec![
                "wss%3A%2F%2Fr1.example.com".to_string(),
                "wss%3A%2F%2Fr2.example.com".to_string(),
                "wss%3A%2F%2Fr3.example.com".to_string(),
            ],
            "first three, in order, form-urlencoded: {uri}"
        );
        assert!(!uri.contains("r4.example.com") && !uri.contains("r5.example.com"));
    }

    #[test]
    fn signin_relays_never_comes_back_empty() {
        let got = signin_relays(&[]);
        assert!(!got.is_empty());
        assert!(got.len() <= SIGNIN_URI_MAX_RELAYS);
    }
}

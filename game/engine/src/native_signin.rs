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

/// Progress events sent from the worker thread to [`poll`].
enum SignInEvent {
    Done(Result<String /* pubkey hex */, JoinAuthError>),
}

struct Manager {
    status: SignInStatus,
    rx: Option<Receiver<SignInEvent>>,
}

static MANAGER: Mutex<Manager> = Mutex::new(Manager {
    status: SignInStatus::Idle,
    rx: None,
});

/// The current sign-in status (clone — cheap, drives the dialog).
pub fn status() -> SignInStatus {
    MANAGER.lock().map(|m| m.status.clone()).unwrap_or(SignInStatus::Idle)
}

/// Reset to idle (the dialog closed / cancelled). The worker thread, if any,
/// keeps running until its handshake times out; its final send is dropped.
pub fn reset() {
    if let Ok(mut m) = MANAGER.lock() {
        m.status = SignInStatus::Idle;
        m.rx = None;
    }
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
    for ev in events {
        match ev {
            SignInEvent::Done(Ok(pubkey_hex)) => {
                let npub = NativeIdentity::SignedIn { pubkey_hex }
                    .npub()
                    .unwrap_or_default();
                m.status = SignInStatus::Success { npub };
                m.rx = None;
            }
            SignInEvent::Done(Err(e)) => {
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
        m.status = SignInStatus::AwaitingScan { uri };
        m.rx = None;
    }
}

fn set_failed(e: JoinAuthError) {
    if let Ok(mut m) = MANAGER.lock() {
        m.status = SignInStatus::Failed { message: e.to_string() };
        m.rx = None;
    }
}

/// Move the constructed session onto a worker thread (own current-thread tokio
/// runtime) and drive `connect → approve → persist` to completion, reporting
/// through the channel.
fn spawn_worker(session: BunkerSession, initial: SignInStatus) {
    let (tx, rx): (Sender<SignInEvent>, Receiver<SignInEvent>) = channel();
    {
        let Ok(mut m) = MANAGER.lock() else { return };
        m.status = initial;
        m.rx = Some(rx);
    }
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(SignInEvent::Done(Err(JoinAuthError::Other(e.to_string()))));
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
            // triggers the phone approval prompt (the live step).
            let pubkey = match session.user_public_key().await {
                Ok(pk) => pk,
                Err(e) => {
                    let _ = tx.send(SignInEvent::Done(Err(native_signer::map_signer_error(&e))));
                    return;
                }
            };
            // Persist the session for silent reconnection on later launches.
            let persisted = match session.persist().await {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(SignInEvent::Done(Err(native_signer::map_signer_error(&e))));
                    return;
                }
            };
            let pubkey_hex = pubkey.to_hex();
            if let Err(e) = native_signer::complete_sign_in(&persisted, &pubkey_hex) {
                let _ = tx.send(SignInEvent::Done(Err(JoinAuthError::Other(e))));
                return;
            }
            session.shutdown().await;
            let _ = tx.send(SignInEvent::Done(Ok(pubkey_hex)));
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

    // Exercises the synchronous status transitions with NO network: bad/empty
    // input fails before any worker thread spawns, and reset/poll are inert at
    // Idle. (The live handshake itself is the owner/device boundary.) Kept as a
    // single test because it drives the process-global manager.
    #[test]
    fn status_lifecycle_without_network() {
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

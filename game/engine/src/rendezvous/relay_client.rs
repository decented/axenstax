//! Talking to relays — behind a trait, so every state machine above it is
//! testable with no network at all.
//!
//! Two implementations: [`MultiRelay`] (a worker thread holding one websocket
//! per relay, reconnecting with backoff) and [`FakeRelayHub`]/[`FakeRelay`] (an
//! in-memory bus two clients share). The fake is not a stub: it applies the same
//! `kind` + `#p` filter a real relay applies, so a test that passes against it
//! is testing the filter as well as the flow.
//!
//! Frames use the raw REQ/EVENT array shape that `tools/feedback-reader/live.mjs`
//! and `native_mailbox::relay` already use successfully against
//! `wss://relay.trotters.cc` — nostr-tools' framing of the same filter is
//! rejected there, so the field order is load-bearing.
//!
//! A relay only ever sees the sealed setup events (CLAUDE.md red line 2): the
//! game's own traffic never touches this path.
//!
//! **Handoff to the consumers (Tasks 15/16).** This layer is a pipe, not a
//! gate. Two things follow from that and neither is done here:
//!
//! 1. **Dedupe by event id.** The same event arrives once per relay that
//!    delivers it, so a four-relay set routinely yields four copies of one
//!    offer. `verify`'s replay guard is keyed on the session, not the event, so
//!    it is not a substitute.
//! 2. **Verify everything.** `parse_frame` deserialises whatever the socket
//!    said; it checks no signature, no kind and no `#p`. A relay can hand back
//!    an event addressed to somebody else, of the wrong kind, or forged
//!    outright. Nothing that comes out of `try_recv` is trustworthy until
//!    `verify_offer` / `verify_answer` has passed on it.
#![cfg(not(target_arch = "wasm32"))]

#[cfg(test)]
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::Event;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use crate::native_mailbox::relay::{event_frame, parse_frame, Frame};

/// What the rendezvous needs from a relay set. Deliberately tiny: publish one
/// event, subscribe to one (kind, recipient) filter, drain what arrived.
pub trait RendezvousRelay: Send {
    fn publish(&self, ev: &Event) -> Result<(), String>;
    /// One standing subscription per client: calling this again **replaces**
    /// the previous filter rather than adding to it (both share a subscription
    /// id, and a relay treats a repeated id as an overwrite). A host that wants
    /// offers and answers at once needs two clients, not two calls.
    fn subscribe(&self, kind: u16, p_tag_hex: &str) -> Result<(), String>;
    /// Non-blocking: the next event that matched a subscription, if any.
    ///
    /// Unfiltered and undeduplicated on purpose — see the module docs. The
    /// same event arrives once per delivering relay, and nothing about it has
    /// been checked beyond "it parsed as an event", so the caller must dedupe
    /// by id and run it through `verify` before believing a word of it.
    fn try_recv(&self) -> Option<Event>;
    /// `(connected, total)` — shown on the Online panel so a host can see at a
    /// glance whether they are actually reachable.
    fn connected(&self) -> (usize, usize);
    /// Events the relay layer had to throw away because this consumer was
    /// behind. Zero for an implementation that cannot drop (the in-memory
    /// fake); non-zero means a flood or a stalled game loop, and it is shown on
    /// the Online panel rather than losing offers in silence.
    fn dropped(&self) -> usize {
        0
    }
}

/// The REQ frame subscribing to `kind` events addressed to `p_tag_hex`.
///
/// `since` rather than `limit`: these kinds are ephemeral so there is nothing
/// stored to page through, and `limit: 0` is refused by some relays.
pub fn req_frame(sub_id: &str, kind: u16, p_tag_hex: &str, since: u64) -> String {
    json!(["REQ", sub_id, {
        "kinds": [kind],
        "#p": [p_tag_hex],
        "since": since,
    }])
    .to_string()
}

/// `since` for a subscription: back off by the same clock-skew window
/// `verify` allows, so an offer sent a moment ago on a slightly slow clock is
/// still inside the filter. Relays that apply `since` to live events (strfry
/// does) would otherwise silently drop it.
fn sub_since() -> u64 {
    unix_now().saturating_sub(crate::rendezvous::verify::CLOCK_SKEW_SECS as u64)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ─── The in-memory fake ─────────────────────────────────────────────────────
//
// Test-only: nothing in a shipped build talks to a hub in memory, and leaving
// it un-gated is what a module-wide `allow(dead_code)` was hiding.

#[cfg(test)]
struct HubInner {
    /// `(client_index, kind, p_tag_hex)`.
    subs: Vec<(usize, u16, String)>,
    inboxes: Vec<VecDeque<Event>>,
    delivered: usize,
}

#[cfg(test)]
/// An in-memory stand-in for a relay set, shared by every [`FakeRelay`] it
/// hands out. Filtering is real, so the tests above prove the filter too.
pub struct FakeRelayHub {
    inner: Arc<Mutex<HubInner>>,
}

#[cfg(test)]
impl Default for FakeRelayHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl FakeRelayHub {
    pub fn new() -> Self {
        FakeRelayHub {
            inner: Arc::new(Mutex::new(HubInner {
                subs: Vec::new(),
                inboxes: Vec::new(),
                delivered: 0,
            })),
        }
    }

    /// A new client attached to this hub.
    pub fn client(&self) -> FakeRelay {
        let mut g = self.inner.lock().expect("hub mutex");
        g.inboxes.push(VecDeque::new());
        let id = g.inboxes.len() - 1;
        FakeRelay { inner: Arc::clone(&self.inner), id }
    }

    /// How many events the hub has routed to at least one subscriber. Lets a
    /// test assert "the offer actually went somewhere" without polling.
    pub fn delivered(&self) -> usize {
        self.inner.lock().expect("hub mutex").delivered
    }
}

#[cfg(test)]
pub struct FakeRelay {
    inner: Arc<Mutex<HubInner>>,
    id: usize,
}

#[cfg(test)]
impl RendezvousRelay for FakeRelay {
    fn publish(&self, ev: &Event) -> Result<(), String> {
        let recipient = crate::rendezvous::payload::recipient_of(ev).map(|p| p.to_hex());
        let mut g = self.inner.lock().map_err(|_| "hub mutex".to_string())?;
        let targets: Vec<usize> = g
            .subs
            .iter()
            .filter(|(_, kind, p)| {
                ev.kind == nostr::Kind::Custom(*kind) && recipient.as_deref() == Some(p.as_str())
            })
            .map(|(client, _, _)| *client)
            .collect();
        if !targets.is_empty() {
            g.delivered += 1;
        }
        for t in targets {
            if let Some(inbox) = g.inboxes.get_mut(t) {
                inbox.push_back(ev.clone());
            }
        }
        Ok(())
    }

    fn subscribe(&self, kind: u16, p_tag_hex: &str) -> Result<(), String> {
        let mut g = self.inner.lock().map_err(|_| "hub mutex".to_string())?;
        g.subs.push((self.id, kind, p_tag_hex.to_string()));
        Ok(())
    }

    fn try_recv(&self) -> Option<Event> {
        let mut g = self.inner.lock().ok()?;
        g.inboxes.get_mut(self.id)?.pop_front()
    }

    fn connected(&self) -> (usize, usize) {
        (1, 1)
    }
}

// ─── The live worker ────────────────────────────────────────────────────────

enum Cmd {
    Publish(Box<Event>),
    Subscribe { kind: u16, p: String },
}

/// A finished dial, reported back to the worker loop by its own task.
type DialResult = (usize, Result<RelaySocket, String>);

/// What `connect_async` hands back for a `ws://` or `wss://` relay.
type RelaySocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// How many publishes are held at once. The handshake only ever has one offer
/// or one answer in flight, so this is a safety valve rather than a queue
/// anybody is meant to fill.
const MAX_PENDING_PUBLISH: usize = 8;

/// A dial is abandoned after this. A relay that accepts TCP and then says
/// nothing (a tarpit, a black hole, a half-open NAT entry) would otherwise hold
/// the whole worker for the OS timeout.
const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

/// Backoff ceiling for a relay that will not come up.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// A socket that dies sooner than this after connecting was flapping, not
/// working: keep growing its backoff instead of resetting it.
const FLAP_WINDOW: Duration = Duration::from_secs(1);

/// How long a socket is polled before moving on to the next one.
const READ_POLL: Duration = Duration::from_millis(30);

/// Reads taken from one socket per turn, so a chatty relay cannot starve the
/// rest or the shutdown check.
const MAX_READS_PER_TURN: usize = 32;

/// How long `Drop` waits for the worker to put its sockets down before giving
/// up and detaching. A dial in flight is the only thing that can take longer.
const STOP_GRACE: Duration = Duration::from_millis(500);

/// How many parsed events are held for the consumer before new ones are
/// dropped.
///
/// The queue is **bounded**, and that is the point. The worker parses whatever
/// the relays send and the consumer drains at most
/// [`crate::online_host::MAX_EVENTS_PER_POLL`] per frame, so an unbounded
/// channel lets anybody who can reach a relay grow this process's memory
/// without limit simply by publishing. 256 is sixteen frames of backlog at the
/// consumer's rate — far more than an honest handshake produces, and a hard
/// ceiling on what a flood can cost.
pub const EVENT_QUEUE: usize = 256;

/// Hand one parsed event to the consumer, or drop it and count it.
///
/// Dropping is the right answer here: these events are a rendezvous handshake,
/// so a peer whose offer is dropped retries, and blocking the worker instead
/// would stop it reading (and reconnecting) every other relay. `try_send` also
/// fails once the consumer is gone, which is a shutdown in progress — counted
/// rather than special-cased, because the count is only ever diagnostic.
fn forward_event(tx: &mpsc::SyncSender<Event>, dropped: &AtomicUsize, ev: Event) {
    if tx.try_send(ev).is_err() {
        let n = dropped.fetch_add(1, Ordering::Relaxed) + 1;
        // Loud once, then rarely: a flood must not become a log flood.
        if n == 1 || n.is_multiple_of(EVENT_QUEUE) {
            log::warn!("[rendezvous] event queue full — {n} events dropped so far");
        }
    }
}

/// One publish, and which relays have had it. A relay that connects a moment
/// after the publish still gets its copy — the offer is one-shot, so "we sent
/// it to whoever happened to be up" loses handshakes.
struct PendingPublish {
    frame: String,
    /// Indexed like `urls`.
    sent: Vec<bool>,
    expires: std::time::Instant,
}

impl PendingPublish {
    /// Nothing left to do: every relay has had it, or it is older than the
    /// freshness window `verify` would accept anyway.
    fn is_done(&self, now: std::time::Instant) -> bool {
        self.sent.iter().all(|&s| s) || now >= self.expires
    }
}

/// One worker thread holding a websocket per relay. Commands go in on a
/// channel, matching events come back on another.
///
/// OWNER BOUNDARY: the socket half is verified against a live relay, not in CI.
/// The frame shapes it sends are unit-tested above and in `native_mailbox`; the
/// socket behaviour (REQ/EVENT/EOSE/OK, reconnect) is covered by the loopback
/// relay in this module's tests.
pub struct MultiRelay {
    cmd_tx: mpsc::Sender<Cmd>,
    ev_rx: Mutex<mpsc::Receiver<Event>>,
    connected: Arc<AtomicUsize>,
    /// Events the worker had to throw away because the consumer was behind.
    /// Non-zero means either a flood or a stalled game loop; either way it is
    /// worth being able to say so rather than losing offers silently.
    dropped: Arc<AtomicUsize>,
    total: usize,
    shutdown: Arc<AtomicBool>,
    /// Taken in `Drop`, which waits for the worker to close its sockets.
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MultiRelay {
    /// Start the worker. Returns immediately; connections come up in the
    /// background and `connected()` reports progress.
    pub fn start(urls: Vec<String>) -> MultiRelay {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let (ev_tx, ev_rx) = mpsc::sync_channel::<Event>(EVENT_QUEUE);
        let connected = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let total = urls.len();

        let worker_connected = Arc::clone(&connected);
        let worker_dropped = Arc::clone(&dropped);
        let worker_shutdown = Arc::clone(&shutdown);
        let handle = std::thread::Builder::new()
            .name("rendezvous-relay".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::error!("[rendezvous] no tokio runtime: {e}");
                        return;
                    }
                };
                rt.block_on(relay_worker(
                    urls,
                    cmd_rx,
                    ev_tx,
                    worker_connected,
                    worker_dropped,
                    worker_shutdown,
                ));
            })
            .expect("spawn rendezvous relay thread");

        MultiRelay {
            cmd_tx,
            ev_rx: Mutex::new(ev_rx),
            connected,
            dropped,
            total,
            shutdown,
            handle: Some(handle),
        }
    }

    /// How many events have been thrown away because the queue was full. Zero
    /// on any honest session; the Online panel and the log use it to say why an
    /// offer never arrived.
    pub fn dropped_events(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for MultiRelay {
    /// Tell the worker to stop and wait for it, so the sockets are actually
    /// down — and the pending offer actually cancelled — before this returns.
    /// Bounded: a dial already in flight can outlast the grace period, and
    /// hanging the caller (often the game loop) would be worse than detaching.
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let Some(handle) = self.handle.take() else { return };
        let deadline = std::time::Instant::now() + STOP_GRACE;
        while !handle.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            log::warn!("[rendezvous] relay worker still stopping; detaching");
        }
    }
}

impl RendezvousRelay for MultiRelay {
    fn publish(&self, ev: &Event) -> Result<(), String> {
        self.cmd_tx
            .send(Cmd::Publish(Box::new(ev.clone())))
            .map_err(|_| "relay worker is gone".to_string())
    }

    fn subscribe(&self, kind: u16, p_tag_hex: &str) -> Result<(), String> {
        self.cmd_tx
            .send(Cmd::Subscribe { kind, p: p_tag_hex.to_string() })
            .map_err(|_| "relay worker is gone".to_string())
    }

    fn try_recv(&self) -> Option<Event> {
        self.ev_rx.lock().ok()?.try_recv().ok()
    }

    fn connected(&self) -> (usize, usize) {
        (self.connected.load(Ordering::Relaxed), self.total)
    }

    fn dropped(&self) -> usize {
        self.dropped_events()
    }
}

/// Connect to every relay, replay the standing subscription to each as it comes
/// up, forward matching events, and reconnect with capped backoff.
///
/// Nothing here is allowed to wait on one relay: dials for every due relay run
/// concurrently, commands are polled rather than awaited, and each socket is
/// read with a short poll and a per-turn cap. A single black hole in the relay
/// list must not eat the join deadline.
async fn relay_worker(
    urls: Vec<String>,
    cmd_rx: mpsc::Receiver<Cmd>,
    ev_tx: mpsc::SyncSender<Event>,
    connected: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
) {
    const SUB_ID: &str = "rz";
    let n = urls.len();
    let mut sockets: Vec<Option<RelaySocket>> = (0..n).map(|_| None).collect();
    // A dial already out on its own task; do not start a second one.
    let mut dialing = vec![false; n];
    let (dial_tx, mut dial_rx) = tokio::sync::mpsc::unbounded_channel::<DialResult>();
    let mut next_try = vec![std::time::Instant::now(); n];
    let mut backoff = vec![Duration::from_secs(1); n];
    // When each live socket came up, for the flap guard.
    let mut connected_at: Vec<Option<std::time::Instant>> = vec![None; n];
    let mut standing: Option<(u16, String)> = None;
    let mut pending: Vec<PendingPublish> = Vec::new();
    let publish_ttl = Duration::from_secs(crate::rendezvous::verify::CLOCK_SKEW_SECS as u64);

    while !shutdown.load(Ordering::Relaxed) {
        // 1. (Re)connect everything that is down and due. Each dial goes out on
        //    its own task and reports back on a channel, so a relay that is up
        //    is installed the moment it answers rather than when the slowest
        //    dial in the batch finishes. Waiting on a batch is what would let
        //    one black hole eat DIAL_TIMEOUT of the join deadline.
        let now = std::time::Instant::now();
        for i in 0..n {
            if sockets[i].is_some() || dialing[i] || now < next_try[i] {
                continue;
            }
            dialing[i] = true;
            let url = urls[i].clone();
            let tx = dial_tx.clone();
            tokio::spawn(async move {
                let r = tokio::time::timeout(DIAL_TIMEOUT, tokio_tungstenite::connect_async(url))
                    .await
                    .map_err(|_| "timed out".to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                    .map(|(ws, _)| ws);
                let _ = tx.send((i, r));
            });
        }
        while let Ok((i, dial)) = dial_rx.try_recv() {
            dialing[i] = false;
            match dial {
                Ok(mut ws) => {
                    log::info!("[rendezvous] connected {}", urls[i]);
                    if let Some((kind, p)) = standing.clone() {
                        let _ =
                            ws.send(Message::Text(req_frame(SUB_ID, kind, &p, sub_since()))).await;
                    }
                    sockets[i] = Some(ws);
                    connected_at[i] = Some(std::time::Instant::now());
                    backoff[i] = Duration::from_secs(1);
                }
                Err(why) => {
                    log::warn!("[rendezvous] {} unreachable: {why}", urls[i]);
                    next_try[i] = std::time::Instant::now() + backoff[i];
                    backoff[i] = (backoff[i] * 2).min(MAX_BACKOFF);
                }
            }
        }
        connected.store(sockets.iter().filter(|s| s.is_some()).count(), Ordering::Relaxed);
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // 2. Apply any pending commands. A publish is queued rather than sent:
        //    step 2b decides who still needs it.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Subscribe { kind, p } => {
                    standing = Some((kind, p.clone()));
                    let frame = req_frame(SUB_ID, kind, &p, sub_since());
                    for ws in sockets.iter_mut().flatten() {
                        let _ = ws.send(Message::Text(frame.clone())).await;
                    }
                }
                Cmd::Publish(ev) => {
                    pending.push(PendingPublish {
                        frame: event_frame(&ev),
                        sent: vec![false; n],
                        expires: std::time::Instant::now() + publish_ttl,
                    });
                    if pending.len() > MAX_PENDING_PUBLISH {
                        pending.remove(0);
                    }
                }
            }
        }

        // 2b. Each pending publish goes to every connected relay that has not
        //     had it yet — including one that only came up just now. An offer
        //     is one-shot: sending it to whoever happened to be up at the time
        //     loses handshakes.
        if !pending.is_empty() {
            for p in pending.iter_mut() {
                for (i, socket) in sockets.iter_mut().enumerate() {
                    if p.sent[i] {
                        continue;
                    }
                    let Some(ws) = socket.as_mut() else { continue };
                    if ws.send(Message::Text(p.frame.clone())).await.is_ok() {
                        p.sent[i] = true;
                    }
                }
            }
            let now = std::time::Instant::now();
            pending.retain(|p| !p.is_done(now));
        }

        // 3. Drain whatever arrived. Each socket is emptied rather than read
        //    once, so a burst does not take a turn per frame.
        let mut read_any = false;
        for i in 0..n {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            for _ in 0..MAX_READS_PER_TURN {
                let Some(ws) = sockets[i].as_mut() else { break };
                match tokio::time::timeout(READ_POLL, ws.next()).await {
                    Ok(Some(Ok(Message::Text(t)))) => {
                        read_any = true;
                        match parse_frame(&t, SUB_ID) {
                            Frame::Wrap(ev) => forward_event(&ev_tx, &dropped, *ev),
                            // A rejected publish is the one relay answer worth
                            // saying out loud — it is why the other side never
                            // hears from us.
                            Frame::Ok { id, accepted: false } => {
                                log::warn!("[rendezvous] {} rejected event {id}", urls[i]);
                            }
                            Frame::Ok { .. } | Frame::Eose | Frame::Other => {}
                        }
                    }
                    // Closed or errored: drop it so step 1 reconnects it.
                    Ok(Some(Err(_)) | None) => {
                        log::info!("[rendezvous] {} closed", urls[i]);
                        // Flap guard: a socket that died almost as soon as it
                        // opened is not a working relay, so keep backing off
                        // instead of hammering it once a second for ever.
                        let flapped = connected_at[i]
                            .map(|t| t.elapsed() < FLAP_WINDOW)
                            .unwrap_or(false);
                        if flapped {
                            backoff[i] = (backoff[i] * 2).min(MAX_BACKOFF);
                        }
                        sockets[i] = None;
                        connected_at[i] = None;
                        next_try[i] = std::time::Instant::now() + backoff[i];
                        break;
                    }
                    // Ping/pong/binary: keep draining.
                    Ok(Some(Ok(_))) => read_any = true,
                    // Quiet — move on to the next relay.
                    Err(_) => break,
                }
            }
        }
        // Only idle when there was genuinely nothing to read.
        if !read_any {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    for ws in sockets.iter_mut().flatten() {
        let _ = ws.close(None).await;
    }
    log::info!("[rendezvous] relay worker stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    async fn addressed_to(sender: &Keys, recipient: &nostr::PublicKey, kind: u16) -> nostr::Event {
        EventBuilder::new(Kind::Custom(kind), "ciphertext")
            .tags([Tag::public_key(*recipient)])
            .sign(sender)
            .await
            .unwrap()
    }

    /// REGRESSION (whole-branch review, IMPORTANT 3). The worker used to
    /// forward parsed events into an UNBOUNDED channel, so anybody who could
    /// publish to a relay this client was subscribed to could grow the process
    /// without limit — the consumer only drains sixteen per frame.
    #[test]
    fn a_flood_fills_the_queue_and_the_overflow_is_dropped_and_counted() {
        let (tx, rx) = mpsc::sync_channel::<Event>(EVENT_QUEUE);
        let dropped = AtomicUsize::new(0);
        let ev = rt().block_on(async {
            let sender = Keys::generate();
            let recipient = Keys::generate();
            addressed_to(&sender, &recipient.public_key(), 20900).await
        });
        for _ in 0..300 {
            forward_event(&tx, &dropped, ev.clone());
        }
        let queued = std::iter::from_fn(|| rx.try_recv().ok()).count();
        assert!(queued <= EVENT_QUEUE, "{queued} queued — the bound did not hold");
        assert_eq!(queued, EVENT_QUEUE, "the queue should be full, not short");
        assert!(dropped.load(Ordering::Relaxed) > 0, "the overflow must be counted");
        assert_eq!(dropped.load(Ordering::Relaxed), 300 - EVENT_QUEUE);
    }

    #[test]
    fn nothing_is_dropped_while_the_consumer_keeps_up() {
        let (tx, rx) = mpsc::sync_channel::<Event>(EVENT_QUEUE);
        let dropped = AtomicUsize::new(0);
        let ev = rt().block_on(async {
            let sender = Keys::generate();
            let recipient = Keys::generate();
            addressed_to(&sender, &recipient.public_key(), 20900).await
        });
        for _ in 0..1000 {
            forward_event(&tx, &dropped, ev.clone());
            assert!(rx.try_recv().is_ok());
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn req_frame_matches_the_known_good_relay_shape() {
        // Same field order as tools/feedback-reader/live.mjs and
        // native_mailbox::relay::req_frame — trotters rejects nostr-tools'
        // framing of the same filter, so the order is load-bearing.
        let f = req_frame("rz", 20900, &"ab".repeat(32), 500);
        let v: serde_json::Value = serde_json::from_str(&f).unwrap();
        assert_eq!(v[0], "REQ");
        assert_eq!(v[1], "rz");
        assert_eq!(v[2]["kinds"][0], 20900);
        assert_eq!(v[2]["#p"][0], "ab".repeat(32));
        assert_eq!(v[2]["since"], 500);
    }

    #[test]
    fn the_fake_delivers_only_to_a_matching_subscriber() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = hub.client();
            let joiner = hub.client();
            let host_key = Keys::generate();
            let joiner_key = Keys::generate();

            host.subscribe(20900, &host_key.public_key().to_hex()).unwrap();
            joiner.subscribe(20901, &joiner_key.public_key().to_hex()).unwrap();

            // Offer → host.
            let offer = addressed_to(&joiner_key, &host_key.public_key(), 20900).await;
            joiner.publish(&offer).unwrap();
            assert_eq!(host.try_recv().map(|e| e.id), Some(offer.id));
            assert!(joiner.try_recv().is_none(), "not addressed to the joiner");

            // Answer → joiner.
            let answer = addressed_to(&host_key, &joiner_key.public_key(), 20901).await;
            host.publish(&answer).unwrap();
            assert_eq!(joiner.try_recv().map(|e| e.id), Some(answer.id));
            assert!(host.try_recv().is_none());
        });
    }

    #[test]
    fn the_fake_drops_an_event_of_the_wrong_kind() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = hub.client();
            let other = hub.client();
            let host_key = Keys::generate();
            host.subscribe(20900, &host_key.public_key().to_hex()).unwrap();
            // Right recipient, wrong kind.
            let ev = addressed_to(&Keys::generate(), &host_key.public_key(), 20901).await;
            other.publish(&ev).unwrap();
            assert!(host.try_recv().is_none());
        });
    }

    #[test]
    fn the_fake_drops_an_event_addressed_to_somebody_else() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = hub.client();
            let other = hub.client();
            let host_key = Keys::generate();
            host.subscribe(20900, &host_key.public_key().to_hex()).unwrap();
            let ev = addressed_to(&Keys::generate(), &Keys::generate().public_key(), 20900).await;
            other.publish(&ev).unwrap();
            assert!(host.try_recv().is_none());
        });
    }

    #[test]
    fn the_fake_delivers_in_order_and_reports_a_delivery_count() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = hub.client();
            let joiner = hub.client();
            let host_key = Keys::generate();
            host.subscribe(20900, &host_key.public_key().to_hex()).unwrap();
            let a = addressed_to(&Keys::generate(), &host_key.public_key(), 20900).await;
            let b = addressed_to(&Keys::generate(), &host_key.public_key(), 20900).await;
            joiner.publish(&a).unwrap();
            joiner.publish(&b).unwrap();
            assert_eq!(hub.delivered(), 2);
            assert_eq!(host.try_recv().map(|e| e.id), Some(a.id));
            assert_eq!(host.try_recv().map(|e| e.id), Some(b.id));
            assert!(host.try_recv().is_none());
        });
    }

    #[test]
    fn the_fake_reports_itself_as_one_of_one_connected() {
        let hub = FakeRelayHub::new();
        assert_eq!(hub.client().connected(), (1, 1));
    }

    // ─── Loopback relay: the worker's socket half, with no network ──────────
    //
    // A real websocket server on 127.0.0.1 speaking the relay frames the worker
    // expects. It is the only way to prove REQ shape on the wire, EVENT/EOSE/OK
    // handling and reconnect-after-close without reaching a live relay.

    /// What the loopback server saw and did, shared with the test.
    #[derive(Default)]
    struct LoopbackState {
        /// Every client → server frame, in order.
        seen: Vec<String>,
        /// Raw event JSON the server pushes to the next subscriber.
        to_push: Vec<String>,
    }

    struct Loopback {
        port: u16,
        state: Arc<Mutex<LoopbackState>>,
        /// Sockets accepted so far — 2 means the worker reconnected.
        accepted: Arc<AtomicUsize>,
        /// Set to hang up on the live socket (cleared once the server has).
        hang_up: Arc<AtomicBool>,
        /// Connections the server has seen end, however they ended.
        closed: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
    }

    impl Drop for Loopback {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    impl Loopback {
        fn start() -> Loopback {
            Loopback::start_on(0)
        }

        /// Bind a specific port so a test can have a relay that is *not there*
        /// yet and then bring it up at that same address.
        fn start_on(port: u16) -> Loopback {
            let state = Arc::new(Mutex::new(LoopbackState::default()));
            let accepted = Arc::new(AtomicUsize::new(0));
            let closed = Arc::new(AtomicUsize::new(0));
            let hang_up = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let (port_tx, port_rx) = mpsc::channel::<u16>();

            let (s, a, c, h, st) = (
                Arc::clone(&state),
                Arc::clone(&accepted),
                Arc::clone(&closed),
                Arc::clone(&hang_up),
                Arc::clone(&stop),
            );
            std::thread::spawn(move || {
                rt().block_on(async move {
                    let listener =
                        tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
                    port_tx.send(listener.local_addr().unwrap().port()).unwrap();
                    while !st.load(Ordering::Relaxed) {
                        let Ok(Ok((stream, _))) =
                            tokio::time::timeout(Duration::from_millis(50), listener.accept()).await
                        else {
                            continue;
                        };
                        let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                            continue;
                        };
                        a.fetch_add(1, Ordering::Relaxed);
                        loop {
                            if st.load(Ordering::Relaxed) {
                                break;
                            }
                            if h.swap(false, Ordering::Relaxed) {
                                let _ = ws.close(None).await;
                                break;
                            }
                            let msg =
                                tokio::time::timeout(Duration::from_millis(20), ws.next()).await;
                            let Ok(Some(Ok(Message::Text(t)))) = msg else {
                                // Closed or errored ends this connection; a
                                // timeout just means "nothing this turn".
                                if matches!(msg, Ok(Some(Err(_)) | None)) {
                                    break;
                                }
                                continue;
                            };
                            let v: serde_json::Value =
                                serde_json::from_str(&t).unwrap_or(serde_json::Value::Null);
                            // Short lock scope: nothing is awaited holding it.
                            let push = {
                                let mut g = s.lock().unwrap();
                                g.seen.push(t);
                                match v[0].as_str() {
                                    Some("REQ") => std::mem::take(&mut g.to_push),
                                    _ => Vec::new(),
                                }
                            };
                            match v[0].as_str() {
                                Some("REQ") => {
                                    let sub = v[1].as_str().unwrap_or("").to_string();
                                    let _ = ws
                                        .send(Message::Text(format!("[\"EOSE\",\"{sub}\"]")))
                                        .await;
                                    for raw in push {
                                        let _ = ws
                                            .send(Message::Text(format!(
                                                "[\"EVENT\",\"{sub}\",{raw}]"
                                            )))
                                            .await;
                                    }
                                }
                                Some("EVENT") => {
                                    let id = v[1]["id"].as_str().unwrap_or("").to_string();
                                    let _ = ws
                                        .send(Message::Text(format!("[\"OK\",\"{id}\",true,\"\"]")))
                                        .await;
                                }
                                _ => {}
                            }
                        }
                        c.fetch_add(1, Ordering::Relaxed);
                    }
                });
            });

            Loopback {
                port: port_rx.recv().expect("loopback bound"),
                state,
                accepted,
                closed,
                hang_up,
                stop,
            }
        }

        fn url(&self) -> String {
            format!("ws://127.0.0.1:{}", self.port)
        }

        fn queue_push(&self, ev: &Event) {
            self.state.lock().unwrap().to_push.push(serde_json::to_string(ev).unwrap());
        }

        /// Every client → server frame whose first element is `verb`.
        fn frames(&self, verb: &str) -> Vec<serde_json::Value> {
            self.state
                .lock()
                .unwrap()
                .seen
                .iter()
                .filter_map(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                .filter(|v| v[0].as_str() == Some(verb))
                .collect()
        }
    }

    /// Poll `f` until it is true, or fail after 10s. Nothing here sleeps for a
    /// fixed length of time — the worker's own cadence decides how long it takes.
    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    /// A port nothing is listening on — bound and released, so a relay can be
    /// "not there yet" at a known address and turn up later.
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    /// A relay that accepts the TCP connection and then says nothing at all,
    /// so `connect_async` hangs until [`DIAL_TIMEOUT`]. A refused port would
    /// not do — it fails instantly and would prove nothing about concurrency.
    struct Tarpit {
        port: u16,
        stop: Arc<AtomicBool>,
    }

    impl Drop for Tarpit {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    impl Tarpit {
        fn start() -> Tarpit {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let st = Arc::clone(&stop);
            std::thread::spawn(move || {
                // Hold every accepted socket open and never answer the upgrade.
                let mut held = Vec::new();
                while !st.load(Ordering::Relaxed) {
                    if let Ok((stream, _)) = listener.accept() {
                        held.push(stream);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            Tarpit { port, stop }
        }

        fn url(&self) -> String {
            format!("ws://127.0.0.1:{}", self.port)
        }
    }

    #[test]
    fn the_worker_sends_the_req_shape_and_forwards_a_matching_event() {
        let relay = Loopback::start();
        let host_key = Keys::generate();
        let ev = rt()
            .block_on(async { addressed_to(&Keys::generate(), &host_key.public_key(), 20900).await });
        relay.queue_push(&ev);

        let m = MultiRelay::start(vec![relay.url()]);
        m.subscribe(20900, &host_key.public_key().to_hex()).unwrap();

        wait_for("a REQ frame", || !relay.frames("REQ").is_empty());
        let req = relay.frames("REQ").remove(0);
        assert_eq!(req[1], "rz");
        assert_eq!(req[2]["kinds"][0], 20900);
        assert_eq!(req[2]["#p"][0], host_key.public_key().to_hex());
        assert!(req[2]["since"].as_u64().unwrap() > 0, "since is a real timestamp");

        // EOSE is swallowed; the EVENT after it comes through.
        let mut got = None;
        wait_for("the pushed event", || {
            got = m.try_recv();
            got.is_some()
        });
        assert_eq!(got.map(|e| e.id), Some(ev.id));
        assert_eq!(m.connected(), (1, 1));
    }

    #[test]
    fn the_worker_publishes_even_when_asked_before_the_socket_is_up() {
        let relay = Loopback::start();
        let ev = rt().block_on(async {
            addressed_to(&Keys::generate(), &Keys::generate().public_key(), 20901).await
        });

        // `start()` returns before the connection completes, so this publish
        // lands while there is nothing to send it on. It must not be lost.
        let m = MultiRelay::start(vec![relay.url()]);
        m.publish(&ev).unwrap();

        wait_for("an EVENT frame", || !relay.frames("EVENT").is_empty());
        assert_eq!(relay.frames("EVENT").remove(0)[1]["id"], ev.id.to_hex());
        // The relay's OK is handled and does not disturb the worker.
        assert_eq!(m.connected(), (1, 1));
    }

    #[test]
    fn the_worker_reconnects_and_replays_the_subscription_after_a_close() {
        let relay = Loopback::start();
        let m = MultiRelay::start(vec![relay.url()]);
        m.subscribe(20900, &"ab".repeat(32)).unwrap();
        wait_for("the first REQ", || !relay.frames("REQ").is_empty());

        relay.hang_up.store(true, Ordering::Relaxed);
        wait_for("a second connection", || relay.accepted.load(Ordering::Relaxed) >= 2);
        // The standing subscription is replayed on the new socket unasked.
        wait_for("the replayed REQ", || relay.frames("REQ").len() >= 2);
        let replayed = relay.frames("REQ").remove(1);
        assert_eq!(replayed[2]["kinds"][0], 20900);
        assert_eq!(replayed[2]["#p"][0], "ab".repeat(32));
        wait_for("the relay count to recover", || m.connected() == (1, 1));
    }

    #[test]
    fn a_relay_that_is_not_there_leaves_the_worker_alive_and_zero_connected() {
        // Port 1 on loopback refuses instantly: the worker must back off, not
        // spin or die, and must keep reporting an honest (0, 1).
        let m = MultiRelay::start(vec!["ws://127.0.0.1:1".to_string()]);
        m.subscribe(20900, &"ab".repeat(32)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(m.connected(), (0, 1));
        assert!(m.publish(&rt().block_on(async {
            addressed_to(&Keys::generate(), &Keys::generate().public_key(), 20901).await
        }))
        .is_ok());
        assert!(m.try_recv().is_none());
    }

    #[test]
    fn a_dead_relay_does_not_delay_a_live_one() {
        // The tarpit is listed FIRST and never answers, so a serial dial would
        // hold the live relay for the whole DIAL_TIMEOUT before it even gets
        // asked. Two of those would blow the 8s join deadline on their own.
        let tarpit = Tarpit::start();
        let relay = Loopback::start();
        let ev = rt().block_on(async {
            addressed_to(&Keys::generate(), &Keys::generate().public_key(), 20900).await
        });

        let started = std::time::Instant::now();
        let m = MultiRelay::start(vec![tarpit.url(), relay.url()]);
        m.subscribe(20900, &"ab".repeat(32)).unwrap();
        m.publish(&ev).unwrap();

        wait_for("the REQ and the EVENT on the live relay", || {
            !relay.frames("REQ").is_empty() && !relay.frames("EVENT").is_empty()
        });
        let took = started.elapsed();
        assert!(
            took < DIAL_TIMEOUT,
            "the live relay waited {took:?} on a dead one — dials are not concurrent"
        );
        // And the dead one is still counted honestly.
        assert_eq!(m.connected().1, 2);
    }

    #[test]
    fn a_pending_publish_reaches_a_relay_that_connects_afterwards() {
        // An offer is one-shot. Sending it only to whoever happened to be up
        // at that instant loses the handshake to a relay 200ms behind.
        let up = Loopback::start();
        let later_port = free_port();
        let ev = rt().block_on(async {
            addressed_to(&Keys::generate(), &Keys::generate().public_key(), 20900).await
        });

        let m = MultiRelay::start(vec![up.url(), format!("ws://127.0.0.1:{later_port}")]);
        m.publish(&ev).unwrap();
        wait_for("the relay that was already up", || !up.frames("EVENT").is_empty());

        // Now bring the second relay up at the address that was refusing.
        let late = Loopback::start_on(later_port);
        wait_for("the relay that arrived late", || !late.frames("EVENT").is_empty());
        assert_eq!(late.frames("EVENT").remove(0)[1]["id"], ev.id.to_hex());
    }

    #[test]
    fn a_pending_publish_is_dropped_once_every_relay_has_it_or_it_goes_stale() {
        let now = std::time::Instant::now();
        let ttl = Duration::from_secs(crate::rendezvous::verify::CLOCK_SKEW_SECS as u64);
        let mut p = PendingPublish {
            frame: "{}".to_string(),
            sent: vec![false, false],
            expires: now + ttl,
        };
        assert!(!p.is_done(now), "still owed to both relays");
        p.sent[0] = true;
        assert!(!p.is_done(now), "still owed to the second relay");
        // Past the freshness window `verify` would accept, it is dead weight
        // whether or not everybody got it.
        assert!(p.is_done(now + ttl), "retained past its TTL");
        p.sent[1] = true;
        assert!(p.is_done(now), "every relay has it");
    }

    #[test]
    fn dropping_the_relay_closes_the_socket_promptly_and_sends_nothing_more() {
        let relay = Loopback::start();
        let m = MultiRelay::start(vec![relay.url()]);
        m.subscribe(20900, &"ab".repeat(32)).unwrap();
        wait_for("the first REQ", || !relay.frames("REQ").is_empty());
        let frames_before = relay.state.lock().unwrap().seen.len();

        let dropped_at = std::time::Instant::now();
        drop(m);
        wait_for("the socket to close", || relay.closed.load(Ordering::Relaxed) >= 1);
        let took = dropped_at.elapsed();
        assert!(took < Duration::from_secs(2), "socket took {took:?} to go down after drop");

        // Nothing else goes out, and nothing redials.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(relay.state.lock().unwrap().seen.len(), frames_before);
        assert_eq!(relay.accepted.load(Ordering::Relaxed), 1);
    }

    /// OWNER BOUNDARY (live relay) — needs network, not run in CI.
    /// Run manually: `cargo test --bin axenstax-engine relay_client -- --ignored`
    #[test]
    #[ignore]
    fn live_multi_relay_connects_and_subscribes() {
        let m = MultiRelay::start(vec![crate::server_resolve::PUBLIC_DEFAULT_RELAYS[0].to_string()]);
        m.subscribe(20900, &"ab".repeat(32)).unwrap();
        // Give the worker a moment to complete the TLS + REQ round-trip.
        std::thread::sleep(std::time::Duration::from_secs(4));
        assert_eq!(m.connected(), (1, 1), "the worker should have one live relay");
    }
}

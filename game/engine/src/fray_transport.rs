//! Fray (`decented-net`, peer-to-peer QUIC) as AxeNStax's [`ServerTransport`] and
//! [`ClientTransport`]. Native only, like `network.rs`, which it stands beside.
//!
//! The game's own protocol, join challenge and admission stay exactly as they
//! are: this module only carries the game's packets, one reliable message per
//! packet, between a Fray host and its joiners. Nothing here is wired into
//! hosting or joining yet (slice B-2 does that), so the module carries
//! `#[allow(dead_code)]` at its declaration until then.
//!
//! # Host side
//!
//! [`FrayHost::start`] takes a running Fray [`HostHandle`] and a
//! `mpsc::Sender<Box<dyn ServerTransport>>` (the type of `HostedServer`'s
//! `remote_rx` channel) and spawns a pump thread, the counterpart of today's
//! `quic-accept` thread plus the per-connection bridge threads. The pump
//! drains [`HostHandle::poll`] and
//!
//! * turns each `PeerJoined` into one [`FrayServerTransport`] and sends it down
//!   the channel;
//! * routes each `Reliable` message to its peer's transport;
//! * on `PeerLeft` delivers the synthetic `Disconnect` packet that
//!   `network::bridge_server_connection` delivers, then sets the transport's
//!   closed flag (`Release`, read with `Acquire`): a transport that reports
//!   closed has already handed over every packet the peer sent first, which is
//!   what `HostedServer::process_inbound_packets` relies on.
//!
//! # Back-pressure
//!
//! Each peer's waiting packets are charged exactly as `transport::InboundQueue`
//! charges them (their length plus `INBOUND_ENTRY_OVERHEAD`) against
//! `MAX_INBOUND_BYTES`. The packet that crosses it is still handed over and
//! what the peer sends after it is dropped: the server's own `InboundQueue`
//! then overflows on its next read and disconnects the peer the game's way,
//! with its reason and its queue cleared, as with `network.rs`. The charge is
//! released as the game reads the packet. Fray's own queue (the same 8 MiB,
//! counted in payload bytes with a message bound that cannot bind first) only
//! fills if the pump thread itself stalls, so a flood is held by three pools (the
//! adapter's channel, the server's `InboundQueue` and Fray's queue: about 24 MiB
//! at most per peer, charged as above). The
//! outbound side is Fray's `outbound_queue_bytes` (8 MiB, as
//! `network::MAX_OUTBOUND_QUEUE_BYTES`): a peer that stops reading is closed as
//! too slow.
//!
//! # Over-long packets
//!
//! Fray drops a send above its cap and logs it. `network.rs` closes the
//! connection instead (a silent drop could leave a joiner waiting forever on a
//! `JoinAccept`). Both transports here do what `network.rs` does: a packet
//! above `MAX_FRAME_LEN` is not sent, and the link is closed.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::Duration;

use decented_net::{
    Bytes, ClientEvent, ClientHandle, DisconnectReason, FailReason, HostEvent, HostHandle, JoinStage, NetConfig,
    PeerId, QueueBound,
};

use crate::network::{Liveness, MAX_FRAME_LEN, MAX_OUTBOUND_QUEUE_BYTES};
use crate::transport::{
    ClientTransport, Packet, ProvenHost, ServerTransport, INBOUND_ENTRY_OVERHEAD, MAX_INBOUND_BYTES,
};

/// The app tag that namespaces AxeNStax's Fray room codes, QUIC ALPN and LAN
/// beacon.
pub const APP_TAG: &str = "axenstax";

/// The TLS exporter label of the join channel binding. The same string as
/// `network::JOIN_EXPORTER_LABEL` (private there), so the binding a Fray
/// connection gives `signet::join_origin` has the same shape as a plain QUIC
/// connection's.
pub const JOIN_EXPORTER_LABEL: &str = "EXPORTER-axenstax-join-v1";

/// How long the pump sleeps when [`HostHandle::poll`] has nothing for it. The
/// server ticks every 50 ms, so a few milliseconds more latency is invisible,
/// and a phone host is not woken a thousand times a second.
const PUMP_IDLE: Duration = Duration::from_millis(5);

/// What a joiner lets wait for its frame loop before Fray closes the link as a
/// flood. `network.rs`'s client side was unbounded; a joiner whose frame loop
/// stops (a loading screen, a minimised window) while chunks stream in at the
/// host's ~1 MiB/s must survive about a minute, and its memory stays bounded.
pub const CLIENT_INBOUND_BYTES: usize = 64 * 1024 * 1024;

/// Fray's network settings with AxeNStax's numbers: one reliable message per
/// game packet (up to `MAX_FRAME_LEN`), 8 MiB of waiting inbound (64 MiB on a
/// joiner) and queued outbound bytes per peer, the 30 s / 5 s liveness of `Liveness::DEFAULT`, the
/// join exporter label and the `axenstax` app tag. Relays, STUN and the beacon
/// stay at Fray's defaults: the caller sets the relay list (the player's own
/// "Your relays") and `relay_fallback` before hosting or joining.
pub fn net_config(data_dir: impl Into<PathBuf>) -> NetConfig {
    let mut cfg = NetConfig::new(data_dir, APP_TAG);
    cfg.max_reliable_len = MAX_FRAME_LEN;
    // Fray counts payload bytes only, so a bound on messages as large as the
    // byte bound can never be reached first.
    cfg.host_inbound = QueueBound { messages: MAX_INBOUND_BYTES, bytes: MAX_INBOUND_BYTES };
    cfg.client_inbound = QueueBound { messages: CLIENT_INBOUND_BYTES, bytes: CLIENT_INBOUND_BYTES };
    cfg.outbound_queue_bytes = MAX_OUTBOUND_QUEUE_BYTES;
    cfg.idle_timeout = Liveness::DEFAULT.idle;
    cfg.keep_alive = Liveness::DEFAULT.keep_alive.expect("the default liveness has a keep-alive");
    cfg.app_binding_label = Some(JOIN_EXPORTER_LABEL.to_string());
    cfg
}

/// The packet `network::bridge_server_connection` delivers when a connection
/// ends, so `HostedServer` sees the leaver the way it always has.
fn synthetic_disconnect() -> Packet {
    crate::protocol::serialize_packet(crate::protocol::PacketType::Disconnect, &())
}

/// What a waiting packet is charged, as `InboundQueue` charges it.
fn charge(len: usize) -> usize {
    len + INBOUND_ENTRY_OVERHEAD
}

// --- Host side ---

/// The pump's side of one peer's transport.
struct PeerSlot {
    tx: mpsc::Sender<Packet>,
    closed: Arc<AtomicBool>,
    /// What the waiting packets are charged (shared with the transport, which
    /// releases a packet's charge as it hands it to the game).
    charged: Arc<AtomicUsize>,
    /// The peer crossed the inbound bound: the game's `InboundQueue` will
    /// disconnect it on its next read, and whatever it still sends is dropped.
    over: bool,
}

/// A new peer's transport and the pump's side of it.
fn peer_pair(peer: PeerId, handle: Weak<HostHandle>, binding: Option<[u8; 32]>) -> (PeerSlot, FrayServerTransport) {
    let (tx, rx) = mpsc::channel();
    let closed = Arc::new(AtomicBool::new(false));
    let charged = Arc::new(AtomicUsize::new(0));
    let cell = OnceLock::new();
    if let Some(value) = binding {
        let _ = cell.set(value);
    }
    let transport = FrayServerTransport {
        peer,
        handle,
        rx,
        closed: closed.clone(),
        charged: charged.clone(),
        binding: cell,
    };
    (PeerSlot { tx, closed, charged, over: false }, transport)
}

impl PeerSlot {
    /// Hand one message from the peer to its transport, charged as
    /// `InboundQueue` would charge it. The message that takes the charge past
    /// `cap` is still handed over, so the server's `InboundQueue` (whose bound
    /// is the same `MAX_INBOUND_BYTES`) overflows on its next read and
    /// disconnects the peer with the game's reason; everything after it is
    /// dropped. Memory stays within `cap` plus one message.
    fn deliver(&mut self, peer: PeerId, bytes: &[u8], cap: usize) {
        if self.over || bytes.is_empty() {
            // An empty message is network.rs's stream hello: skipped.
            return;
        }
        let cost = charge(bytes.len());
        let crossing = self.charged.load(Ordering::Relaxed) + cost > cap;
        self.charged.fetch_add(cost, Ordering::Relaxed);
        if self.tx.send(bytes.to_vec()).is_err() {
            // The server dropped this transport (its Drop closes the link).
            self.charged.fetch_sub(cost, Ordering::Relaxed);
            return;
        }
        if crossing {
            log::warn!("Fray: {peer} has too many packets waiting; dropping the rest until the server reads");
            self.over = true;
        }
    }

    /// The connection ended: deliver the synthetic `Disconnect`, then mark the
    /// transport closed (so closed implies every earlier packet is already
    /// readable).
    fn finish(self) {
        let packet = synthetic_disconnect();
        self.charged.fetch_add(charge(packet.len()), Ordering::Relaxed);
        let _ = self.tx.send(packet);
        self.closed.store(true, Ordering::Release);
    }
}

/// One joined peer, as the game's server sees it.
pub struct FrayServerTransport {
    peer: PeerId,
    /// Weak, so that the room lives exactly as long as its [`FrayHost`]: the
    /// transports a server still holds never keep a stopped room open.
    handle: Weak<HostHandle>,
    rx: mpsc::Receiver<Packet>,
    closed: Arc<AtomicBool>,
    charged: Arc<AtomicUsize>,
    binding: OnceLock<[u8; 32]>,
}

impl FrayServerTransport {
    /// Close the link to this peer. Fray gives what was already sent 250 ms to
    /// flush before the close (a fixed wait, not an acknowledgement).
    fn close_link(&self) {
        if let Some(handle) = self.handle.upgrade() {
            handle.disconnect(self.peer);
        }
    }
}

impl ServerTransport for FrayServerTransport {
    fn send_to_client(&self, data: &[u8]) {
        if self.closed.load(Ordering::Relaxed) || data.is_empty() {
            return;
        }
        let Some(handle) = self.handle.upgrade() else {
            // The room is gone; nothing will ever arrive.
            self.closed.store(true, Ordering::Release);
            return;
        };
        if data.len() > MAX_FRAME_LEN {
            // As network.rs's writer: close rather than drop, so the peer sees
            // the end instead of waiting for a packet that never comes. The
            // close arrives back as this peer's `PeerLeft`.
            log::warn!(
                "Fray: a {}-byte packet is over the frame cap; closing {}",
                data.len(),
                self.peer
            );
            handle.disconnect(self.peer);
            return;
        }
        handle.send_reliable(self.peer, Bytes::copy_from_slice(data));
    }

    fn try_recv_from_client(&self) -> Option<Packet> {
        match self.rx.try_recv() {
            Ok(packet) => {
                self.charged.fetch_sub(charge(packet.len()), Ordering::Relaxed);
                Some(packet)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                // The pump ended without a `PeerLeft` for this peer.
                self.closed.store(true, Ordering::Release);
                None
            }
        }
    }

    fn channel_binding(&self) -> Option<[u8; 32]> {
        if let Some(binding) = self.binding.get() {
            return Some(*binding);
        }
        let binding = self.handle.upgrade()?.app_binding(self.peer)?;
        let _ = self.binding.set(binding);
        Some(binding)
    }

    fn is_closed(&self) -> bool {
        // Acquire pairs with the pump's Release: closed implies the peer's last
        // packet (and the synthetic `Disconnect`) is already in `rx`.
        self.closed.load(Ordering::Acquire)
    }
}

impl Drop for FrayServerTransport {
    /// The slot was freed: close the link, as dropping a `QuicServerTransport`
    /// makes its bridge flush and close. Fray gives a queued `JoinReject` 250 ms
    /// to get out first (`network.rs` waited up to 2 s for the acknowledgement),
    /// so on a slow path the reason can be lost and the joiner sees the link end.
    fn drop(&mut self) {
        if !self.closed.load(Ordering::Relaxed) {
            self.close_link();
        }
    }
}

/// What the pump thread reports about the room itself.
#[derive(Default)]
struct HostState {
    ready: AtomicBool,
    failure: Mutex<Option<String>>,
}

/// A Fray room and the pump thread that turns its peers into
/// [`FrayServerTransport`]s. Dropping it (or [`FrayHost::shutdown`]) stops the
/// pump and closes the room; transports still held by a server then report
/// closed.
pub struct FrayHost {
    handle: Option<Arc<HostHandle>>,
    state: Arc<HostState>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FrayHost {
    /// Start pumping `handle`'s peers into `remote_tx`.
    pub fn start(handle: HostHandle, remote_tx: mpsc::Sender<Box<dyn ServerTransport>>) -> FrayHost {
        Self::start_with_cap(handle, remote_tx, MAX_INBOUND_BYTES)
    }

    /// As [`FrayHost::start`], with the per-peer inbound bound given (tests
    /// drive it small; the game always uses `MAX_INBOUND_BYTES`).
    fn start_with_cap(
        handle: HostHandle,
        remote_tx: mpsc::Sender<Box<dyn ServerTransport>>,
        inbound_cap: usize,
    ) -> FrayHost {
        let handle = Arc::new(handle);
        let state = Arc::new(HostState::default());
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (handle, state, stop) = (handle.clone(), state.clone(), stop.clone());
            thread::Builder::new()
                .name("fray-pump".into())
                .spawn(move || pump(handle, state, stop, remote_tx, inbound_cap))
                .expect("spawn the Fray pump thread")
        };
        FrayHost { handle: Some(handle), state, stop, thread: Some(thread) }
    }

    /// The room (code, LAN token, address, `set_open`, peer count, ...).
    pub fn handle(&self) -> &HostHandle {
        self.handle.as_ref().expect("the handle is present until shutdown")
    }

    /// Fray has reported the room live (the code can be shown).
    pub fn is_ready(&self) -> bool {
        self.state.ready.load(Ordering::Acquire)
    }

    /// Why hosting could not start, if it could not.
    pub fn failure(&self) -> Option<String> {
        self.state.failure.lock().ok()?.clone()
    }

    /// Stop the pump and close the room, letting the close reach the peers.
    pub fn shutdown(mut self) {
        self.stop_pump();
        if let Some(handle) = self.handle.take() {
            // Sole owner once the pump has ended, unless a transport is
            // mid-send on another thread; then the last holder's drop closes it.
            if let Ok(handle) = Arc::try_unwrap(handle) {
                handle.shutdown();
            }
        }
    }

    fn stop_pump(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for FrayHost {
    fn drop(&mut self) {
        self.stop_pump();
    }
}

/// Whether a joined peer may be handed to the server: only one whose connection
/// has an exporter binding, so its Signet join is checked against that
/// connection and never against the unbound origin.
fn may_seat(binding: Option<&[u8; 32]>) -> bool {
    binding.is_some()
}

/// The pump thread: drain Fray's events and route them (see the module docs).
fn pump(
    handle: Arc<HostHandle>,
    state: Arc<HostState>,
    stop: Arc<AtomicBool>,
    remote_tx: mpsc::Sender<Box<dyn ServerTransport>>,
    inbound_cap: usize,
) {
    let mut slots: HashMap<PeerId, PeerSlot> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let mut busy = false;
        while !stop.load(Ordering::Relaxed)
            && let Some(event) = handle.poll()
        {
            busy = true;
            match event {
                HostEvent::Ready { .. } => state.ready.store(true, Ordering::Release),
                HostEvent::Failed { reason } => {
                    log::warn!("Fray: hosting failed: {reason}");
                    if let Ok(mut failure) = state.failure.lock() {
                        *failure = Some(reason);
                    }
                }
                HostEvent::PeerJoined { peer, .. } => {
                    let binding = handle.app_binding(peer);
                    if !may_seat(binding.as_ref()) {
                        // Without the connection's exporter the server would
                        // check the join against the unbound origin, the one
                        // the binding exists to rule out. Fray always has it
                        // by now; if it ever does not, refuse rather than seat.
                        log::warn!("Fray: {peer} has no channel binding; closing it unseated");
                        handle.disconnect(peer);
                        continue;
                    }
                    let (slot, transport) = peer_pair(peer, Arc::downgrade(&handle), binding);
                    // If the server is gone the transport is dropped here,
                    // which closes the link.
                    if remote_tx.send(Box::new(transport)).is_ok() {
                        slots.insert(peer, slot);
                    }
                }
                HostEvent::PeerLeft { peer, .. } => {
                    if let Some(slot) = slots.remove(&peer) {
                        slot.finish();
                    }
                }
                HostEvent::Reliable { peer, bytes } => {
                    if let Some(slot) = slots.get_mut(&peer) {
                        slot.deliver(peer, &bytes, inbound_cap);
                    }
                }
                // The game sends and reads reliable packets only.
                HostEvent::Lossy { .. } => {}
            }
        }
        if !busy {
            thread::sleep(PUMP_IDLE);
        }
    }
    // The room is stopping: tell every transport still held.
    for (_, slot) in slots.drain() {
        slot.finish();
    }
}

// --- Client side ---

struct ClientInner {
    /// `None` once the link has been closed.
    handle: Option<ClientHandle>,
    /// Fray reported `Connected`: sends go out.
    connected: bool,
    /// Packets sent before `Connected`, in order, and their bytes.
    held: VecDeque<Packet>,
    held_bytes: usize,
    failure: Option<(JoinStage, FailReason)>,
    closed: bool,
    /// What the host said when it disconnected us (`DisconnectReason::KickedWith`). Boxed so
    /// the transport stays under clippy's size gap with the other `JoinPoll` variants.
    close_text: Option<Box<str>>,
    /// The host sent the disconnect (`Kicked`, with words or without), as against a network
    /// error, a timeout or the host leaving.
    host_closed: bool,
}

impl ClientInner {
    /// End the link: hand the handle back to Fray to close and mark closed.
    fn close(&mut self) {
        self.closed = true;
        self.held.clear();
        self.held_bytes = 0;
        if let Some(handle) = self.handle.take() {
            handle.shutdown();
        }
    }
}

/// A joiner's link to the host.
///
/// Built over a [`ClientHandle`] (from `join`, `join_lan` or `join_addr`) that
/// may still be joining: packets sent before Fray reports `Connected` wait,
/// bounded by `MAX_OUTBOUND_QUEUE_BYTES`, and go out in order once it does.
/// Fray's events are read in [`ClientTransport::try_recv_from_server`], one at
/// a time, so the packets waiting to be read stay in Fray's own bounded queue.
pub struct FrayClientTransport {
    inner: Mutex<ClientInner>,
    binding: OnceLock<[u8; 32]>,
    /// The host Fray proved when it reported `Connected`: set by
    /// [`FrayClientTransport::already_connected_to`] or, for a join still in
    /// progress, when that event is read.
    proven: OnceLock<Box<ProvenHost>>,
}

impl FrayClientTransport {
    /// Wrap a join in progress.
    pub fn new(handle: ClientHandle) -> FrayClientTransport {
        FrayClientTransport {
            inner: Mutex::new(ClientInner {
                handle: Some(handle),
                connected: false,
                held: VecDeque::new(),
                held_bytes: 0,
                failure: None,
                closed: false,
                close_text: None,
                host_closed: false,
            }),
            binding: OnceLock::new(),
            proven: OnceLock::new(),
        }
    }

    /// Wrap a handle whose `Connected` event the caller has already read.
    pub fn already_connected(handle: ClientHandle) -> FrayClientTransport {
        let transport = Self::new(handle);
        if let Ok(mut inner) = transport.inner.lock() {
            inner.connected = true;
        }
        transport
    }

    /// [`already_connected`](FrayClientTransport::already_connected) for a
    /// caller that read `Connected` and keeps the host it proved.
    pub fn already_connected_to(handle: ClientHandle, host: ProvenHost) -> FrayClientTransport {
        let transport = Self::already_connected(handle);
        let _ = transport.proven.set(Box::new(host));
        transport
    }

    /// Fray has reported the host joined (seen while reading packets).
    #[cfg_attr(not(test), expect(dead_code, reason = "Join Game waits for Connected before wrapping (fray_play)"))]
    pub fn is_connected(&self) -> bool {
        self.inner.lock().is_ok_and(|inner| inner.connected && !inner.closed)
    }

    /// Why the join failed, once Fray has said so (seen while reading packets).
    #[expect(dead_code, reason = "Join Game reads the failure before wrapping (fray_play)")]
    pub fn failure(&self) -> Option<(JoinStage, FailReason)> {
        self.inner.lock().ok()?.failure.clone()
    }
}

impl ClientTransport for FrayClientTransport {
    fn send_to_server(&self, data: &[u8]) {
        let Ok(mut inner) = self.inner.lock() else { return };
        if inner.closed || data.is_empty() {
            return;
        }
        if data.len() > MAX_FRAME_LEN {
            // As network.rs's writer: close rather than drop.
            log::warn!("Fray: a {}-byte packet is over the frame cap; closing", data.len());
            inner.close();
            return;
        }
        if !inner.connected {
            if inner.held_bytes + data.len() > MAX_OUTBOUND_QUEUE_BYTES {
                log::warn!("Fray: too many bytes held before the host answered; closing");
                inner.close();
                return;
            }
            inner.held_bytes += data.len();
            inner.held.push_back(data.to_vec());
            return;
        }
        if let Some(handle) = &inner.handle {
            handle.send_reliable(Bytes::copy_from_slice(data));
        }
    }

    fn try_recv_from_server(&self) -> Option<Packet> {
        let mut guard = self.inner.lock().ok()?;
        let inner = &mut *guard;
        loop {
            let event = inner.handle.as_ref()?.poll()?;
            match event {
                ClientEvent::Progress { .. } | ClientEvent::Lossy(_) => {}
                ClientEvent::Connected { host_pubkey, host_name, .. } => {
                    let _ = self.proven.set(Box::new(ProvenHost { key: host_pubkey.to_bytes(), name: host_name }));
                    inner.connected = true;
                    inner.held_bytes = 0;
                    if let Some(handle) = &inner.handle {
                        for packet in inner.held.drain(..) {
                            handle.send_reliable(Bytes::from(packet));
                        }
                    }
                }
                ClientEvent::Reliable(bytes) => {
                    // An empty message is network.rs's stream hello: skipped.
                    if !bytes.is_empty() {
                        return Some(bytes.to_vec());
                    }
                }
                ClientEvent::Failed { stage, reason } => {
                    log::warn!("Fray: join failed at {stage:?}: {reason}");
                    inner.failure = Some((stage, reason));
                    inner.close();
                    return None;
                }
                ClientEvent::Disconnected { reason } => {
                    log::info!("Fray: disconnected: {reason:?}");
                    match reason {
                        DisconnectReason::KickedWith(text) => {
                            inner.close_text = Some(text.into_boxed_str());
                            inner.host_closed = true;
                        }
                        DisconnectReason::Kicked => inner.host_closed = true,
                        _ => {}
                    }
                    inner.close();
                    return None;
                }
            }
        }
    }

    fn channel_binding(&self) -> Option<[u8; 32]> {
        if let Some(binding) = self.binding.get() {
            return Some(*binding);
        }
        let binding = self.inner.lock().ok()?.handle.as_ref()?.app_binding()?;
        let _ = self.binding.set(binding);
        Some(binding)
    }

    /// True once the host ended the link, the join failed, or this end closed
    /// it. Set only after every packet the host sent before has been read, as
    /// `RemoteClient::poll` expects (it reads this before it drains).
    fn is_closed(&self) -> bool {
        self.inner.lock().map_or(true, |inner| inner.closed)
    }

    /// Set with the close, so it is there whenever [`Self::is_closed`] is.
    fn close_text(&self) -> Option<String> {
        self.inner.lock().ok()?.close_text.as_deref().map(String::from)
    }

    /// Set with the close, like [`Self::close_text`].
    fn host_closed_link(&self) -> bool {
        self.inner.lock().is_ok_and(|inner| inner.host_closed)
    }

    fn proven_host(&self) -> Option<ProvenHost> {
        self.proven.get().map(|host| (**host).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decented_net::{host, join_addr, Identity, RoomInfo};
    use std::net::{Ipv4Addr, SocketAddr};
    use std::time::Instant;

    const WAIT: Duration = Duration::from_secs(15);

    /// AxeNStax's config on loopback: no relays or STUN, and a private beacon
    /// port per test so the tests never hear each other.
    fn test_cfg() -> NetConfig {
        let mut r = [0u8; 2];
        getrandom::fill(&mut r).expect("OS RNG");
        let mut cfg = net_config(std::env::temp_dir()).lan_only();
        cfg.lan_port = 30_000 + (u16::from_le_bytes(r) % 20_000);
        cfg
    }

    fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if done() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("not seen within {WAIT:?}: {what}");
    }

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    struct Rig {
        host: FrayHost,
        remote_rx: mpsc::Receiver<Box<dyn ServerTransport>>,
        cfg: NetConfig,
        addr: SocketAddr,
    }

    fn start_rig(inbound_cap: usize) -> Rig {
        let cfg = test_cfg();
        let (tx, remote_rx) = mpsc::channel();
        let handle = host(&cfg, &Identity::ephemeral(), RoomInfo { name: "Test room".into(), max_players: 4 });
        let host = FrayHost::start_with_cap(handle, tx, inbound_cap);
        eventually("the room is ready", || host.is_ready());
        let port = host.handle().local_addr().expect("listening").port();
        Rig { host, remote_rx, cfg, addr: SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port) }
    }

    impl Rig {
        /// Join one more client; returns it with the host's transport for it.
        fn join(&self) -> (FrayClientTransport, Box<dyn ServerTransport>) {
            let handle = join_addr(&self.cfg, &Identity::ephemeral(), self.addr, &self.host.handle().lan_token());
            let client = FrayClientTransport::new(handle);
            let server = self.remote_rx.recv_timeout(WAIT).expect("the host hands over a transport");
            eventually("the client is connected", || {
                let _ = client.try_recv_from_server();
                client.is_connected()
            });
            (client, server)
        }
    }

    fn server_recv(server: &dyn ServerTransport) -> Packet {
        let mut got = None;
        eventually("a packet reaches the host", || {
            got = server.try_recv_from_client();
            got.is_some()
        });
        got.unwrap()
    }

    fn client_recv(client: &FrayClientTransport) -> Packet {
        let mut got = None;
        eventually("a packet reaches the client", || {
            got = client.try_recv_from_server();
            got.is_some()
        });
        got.unwrap()
    }

    /// A host that says why it let a player go: the words arrive with the close, on the transport
    /// the game reads; a plain disconnect has none.
    #[test]
    fn the_hosts_reason_for_a_disconnect_is_held_with_the_close() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, _server) = rig.join();
        assert_eq!(client.close_text(), None, "nothing said while the link is up");
        rig.host
            .handle()
            .disconnect_with_reason(PeerId(1), "The world restarted; join again with the same code.");
        eventually("the link to close", || {
            let _ = client.try_recv_from_server();
            client.is_closed()
        });
        assert_eq!(client.close_text().as_deref(), Some("The world restarted; join again with the same code."));

        let (plain, _server) = rig.join();
        rig.host.handle().disconnect(PeerId(2));
        eventually("the link to close", || {
            let _ = plain.try_recv_from_server();
            plain.is_closed()
        });
        assert_eq!(plain.close_text(), None, "a plain disconnect has no words");
        // Either way it was the host that ended the link, which the signer wording relies on.
        assert!(client.host_closed_link() && plain.host_closed_link());
    }

    /// A host that goes away is not a host that turned the player away: the transport says the
    /// link closed, but not that the host closed it.
    #[test]
    fn a_link_that_ends_because_the_host_shut_down_is_not_a_host_kick() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, _server) = rig.join();
        assert!(!client.host_closed_link(), "nothing closed yet");
        rig.host.shutdown();
        eventually("the link to close", || {
            let _ = client.try_recv_from_server();
            client.is_closed()
        });
        assert!(!client.host_closed_link() && client.close_text().is_none());
    }

    /// A peer with no exporter binding is not seated: the server would check
    /// its join against `axenstax-join:unbound` (T-JOIN-RELAY).
    #[test]
    fn a_peer_without_a_channel_binding_is_not_seated() {
        assert!(!may_seat(None));
        assert!(may_seat(Some(&[7u8; 32])));
    }

    #[test]
    fn config_carries_axenstax_numbers() {
        let cfg = net_config("data");
        assert_eq!(cfg.app_tag, "axenstax");
        assert_eq!(cfg.max_reliable_len, 65_537);
        assert_eq!(cfg.max_reliable_len, MAX_FRAME_LEN);
        assert_eq!(cfg.host_inbound.bytes, 8 * 1024 * 1024);
        assert_eq!(cfg.client_inbound.bytes, 64 * 1024 * 1024);
        for bound in [cfg.host_inbound, cfg.client_inbound] {
            assert!(bound.messages >= bound.bytes, "the message count cannot bind before the bytes");
        }
        assert_eq!(cfg.outbound_queue_bytes, 8 * 1024 * 1024);
        assert_eq!(cfg.idle_timeout, Duration::from_secs(30));
        assert_eq!(cfg.keep_alive, Duration::from_secs(5));
        assert_eq!(cfg.app_binding_label.as_deref(), Some("EXPORTER-axenstax-join-v1"));
    }

    #[test]
    fn frames_up_to_the_cap_arrive_whole_in_both_directions() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, server) = rig.join();
        let sizes = [1, 48 * 1024, MAX_FRAME_LEN, 7, MAX_FRAME_LEN];
        let up: Vec<Vec<u8>> = sizes.iter().enumerate().map(|(i, &n)| pattern(n, i as u8)).collect();
        let down: Vec<Vec<u8>> = sizes.iter().enumerate().map(|(i, &n)| pattern(n, 100 + i as u8)).collect();
        for packet in &up {
            client.send_to_server(packet);
        }
        for packet in &down {
            server.send_to_client(packet);
        }
        for packet in &up {
            assert!(server_recv(server.as_ref()) == *packet, "a {}-byte packet arrived whole and in order", packet.len());
        }
        for packet in &down {
            assert!(client_recv(&client) == *packet, "a {}-byte packet arrived whole and in order", packet.len());
        }
        assert!(!client.is_closed() && !server.is_closed());
    }

    #[test]
    fn a_client_send_over_the_cap_closes_the_link_and_is_not_delivered() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, server) = rig.join();
        // (Read first: the sender's close, here as in network.rs, does not
        // wait for what it sent a moment ago to be acknowledged.)
        client.send_to_server(b"first");
        assert_eq!(server_recv(server.as_ref()), b"first");
        client.send_to_server(&vec![9u8; MAX_FRAME_LEN + 1]);
        assert!(client.is_closed(), "the sender closes at once, as network.rs's writer does");
        // Then the synthetic Disconnect; the 65 538-byte packet never arrives.
        assert_eq!(server_recv(server.as_ref()), synthetic_disconnect());
        eventually("the host's transport reports closed", || server.is_closed());
        assert!(server.try_recv_from_client().is_none());
    }

    #[test]
    fn a_host_send_over_the_cap_closes_the_link_and_is_not_delivered() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, server) = rig.join();
        server.send_to_client(b"first");
        server.send_to_client(&vec![9u8; MAX_FRAME_LEN + 1]);
        assert_eq!(client_recv(&client), b"first");
        eventually("the client sees the link end", || {
            assert!(client.try_recv_from_server().is_none(), "the long packet is never delivered");
            client.is_closed()
        });
        eventually("the host's transport reports closed", || server.is_closed());
        assert_eq!(server_recv(server.as_ref()), synthetic_disconnect());
    }

    #[test]
    fn a_peer_leaving_closes_its_server_transport_after_its_last_packet() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, server) = rig.join();
        let (other_client, other_server) = rig.join();
        // Fray closes a leaver's connection at once without flushing what it
        // has not yet had acknowledged (network.rs's bridge waited up to 2 s),
        // so the last packet is only certain to arrive if it was read first.
        client.send_to_server(b"bye");
        assert_eq!(server_recv(server.as_ref()), b"bye");
        client.send_to_server(b"unread");
        drop(client);
        eventually("the host's transport reports closed", || server.is_closed());
        // Closed implies everything that arrived before, and the synthetic
        // Disconnect, is already readable.
        let mut rest = Vec::new();
        while let Some(packet) = server.try_recv_from_client() {
            rest.push(packet);
        }
        assert_eq!(rest.last(), Some(&synthetic_disconnect()));
        assert!(rest.len() <= 2);
        // The other peer is untouched and still talks.
        assert!(!other_server.is_closed());
        other_client.send_to_server(b"still here");
        assert_eq!(server_recv(other_server.as_ref()), b"still here");
    }

    #[test]
    fn dropping_a_server_transport_closes_the_link_to_that_client() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, server) = rig.join();
        server.send_to_client(b"rejected");
        drop(server);
        assert_eq!(client_recv(&client), b"rejected", "what was queued is flushed first");
        eventually("the client sees the link end", || {
            let _ = client.try_recv_from_server();
            client.is_closed()
        });
    }

    #[test]
    fn stopping_the_room_closes_every_transport() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, server) = rig.join();
        rig.host.shutdown();
        eventually("the host's transport reports closed", || server.is_closed());
        eventually("the client sees the host leave", || {
            let _ = client.try_recv_from_server();
            client.is_closed()
        });
        server.send_to_client(b"into the void");
    }

    #[test]
    fn channel_binding_is_equal_on_both_ends_and_per_connection() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (client, server) = rig.join();
        let (other_client, other_server) = rig.join();
        let mine = client.channel_binding().expect("the client has a binding once connected");
        assert_eq!(server.channel_binding(), Some(mine), "both ends of one connection agree");
        assert_ne!(mine, [0u8; 32]);
        let theirs = other_client.channel_binding().expect("binding");
        assert_eq!(other_server.channel_binding(), Some(theirs));
        assert_ne!(mine, theirs, "two connections never share one");
        assert!(!server.is_websocket());
    }

    #[test]
    fn a_peer_over_the_inbound_bound_is_left_for_the_server_to_disconnect() {
        // A bound of 8 KiB: a few 1 KiB packets fit (each charged its length
        // plus the entry overhead), a flood does not.
        let cap = 8 * 1024;
        let rig = start_rig(cap);
        let (client, server) = rig.join();
        let (calm_client, calm_server) = rig.join();
        for _ in 0..64 {
            client.send_to_server(&[5u8; 1024]);
        }
        // What fits, then the packet that crosses the bound, then nothing.
        let expected = cap / charge(1024) + 1;
        let mut read = 0;
        eventually("the packets up to the crossing one arrive", || {
            while server.try_recv_from_client().is_some() {
                read += 1;
            }
            read >= expected
        });
        thread::sleep(Duration::from_millis(300));
        while server.try_recv_from_client().is_some() {
            read += 1;
        }
        assert_eq!(read, expected, "the rest of the flood is dropped");
        // The adapter does not cut the peer off itself: the server's
        // InboundQueue does, with its reason (see the deliver tests).
        assert!(!server.is_closed());
        // A calm peer on the same room is not affected.
        calm_client.send_to_server(b"calm");
        assert_eq!(server_recv(calm_server.as_ref()), b"calm");
    }

    #[test]
    fn two_peers_sending_at_once_lose_and_reorder_nothing() {
        let rig = start_rig(MAX_INBOUND_BYTES);
        let (a, server_a) = rig.join();
        let (b, server_b) = rig.join();
        thread::scope(|s| {
            for client in [&a, &b] {
                s.spawn(move || {
                    for i in 0..500u32 {
                        client.send_to_server(&i.to_le_bytes());
                    }
                });
            }
        });
        for server in [&server_a, &server_b] {
            for i in 0..500u32 {
                assert_eq!(server_recv(server.as_ref()), i.to_le_bytes(), "packet {i} in order");
            }
        }
    }

    /// A peer's slot and transport with no network behind them.
    fn bare_pair() -> (PeerSlot, FrayServerTransport) {
        peer_pair(PeerId(1), Weak::new(), None)
    }

    fn drain(transport: &FrayServerTransport) -> Vec<Packet> {
        std::iter::from_fn(|| transport.try_recv_from_client()).collect()
    }

    #[test]
    fn deliver_admits_exactly_the_bound_and_hands_over_the_crossing_packet() {
        let (mut slot, transport) = bare_pair();
        let cap = 3 * charge(100);
        for i in 0..3u8 {
            slot.deliver(PeerId(1), &[i; 100], cap);
        }
        assert!(!slot.over, "a charge equal to the bound is admitted");
        assert_eq!(transport.charged.load(Ordering::Relaxed), cap);
        slot.deliver(PeerId(1), &[3], cap);
        assert!(slot.over, "one byte more crosses it");
        slot.deliver(PeerId(1), &[4; 100], cap);
        let got = drain(&transport);
        assert_eq!(got.len(), 4, "the crossing packet is handed over, the next is dropped");
        assert_eq!(got[3], vec![3]);
        assert_eq!(transport.charged.load(Ordering::Relaxed), 0);
        slot.finish();
        assert!(transport.is_closed());
        assert_eq!(drain(&transport), vec![synthetic_disconnect()]);
        assert_eq!(transport.charged.load(Ordering::Relaxed), 0, "nothing is left charged");
    }

    #[test]
    fn deliver_and_read_leave_exactly_the_unread_charge() {
        let (mut slot, transport) = bare_pair();
        let mut unread = std::collections::VecDeque::new();
        for i in 0..200usize {
            let len = 1 + (i * 37) % 900;
            slot.deliver(PeerId(1), &vec![i as u8; len], MAX_INBOUND_BYTES);
            unread.push_back(len);
            if i % 3 == 0 {
                let packet = transport.try_recv_from_client().expect("a packet waits");
                assert_eq!(packet.len(), unread.pop_front().unwrap());
            }
            let want: usize = unread.iter().map(|&n| charge(n)).sum();
            assert_eq!(transport.charged.load(Ordering::Relaxed), want, "after packet {i}");
        }
        drain(&transport);
        slot.finish();
        drain(&transport);
        assert_eq!(transport.charged.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_crossing_packet_overflows_the_servers_inbound_queue() {
        let (mut slot, transport) = bare_pair();
        let frame = vec![1u8; MAX_FRAME_LEN];
        while !slot.over {
            slot.deliver(PeerId(1), &frame, MAX_INBOUND_BYTES);
        }
        let mut queue = crate::transport::InboundQueue::default();
        assert!(queue.fill_from(&transport).is_err(), "the server disconnects the peer the game's way");
    }

    #[test]
    fn the_inbound_charge_is_released_as_packets_are_read() {
        // 4 KiB packets are charged 4 KiB + overhead each: three rounds of
        // three would pass an 16 KiB bound if reading did not release them.
        let rig = start_rig(16 * 1024);
        let (client, server) = rig.join();
        for round in 0..3u8 {
            for _ in 0..3 {
                client.send_to_server(&[round; 4 * 1024]);
            }
            for _ in 0..3 {
                assert_eq!(server_recv(server.as_ref()), vec![round; 4 * 1024]);
            }
        }
        assert!(!server.is_closed(), "a peer that is read is never cut off");
    }
}

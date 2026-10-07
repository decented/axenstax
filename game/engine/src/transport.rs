//! Transport abstraction — connects clients to the server.
//!
//! Implementations:
//! - ChannelTransport: in-process mpsc channels (split screen, host-as-server local)
//! - NetworkTransport: UDP sockets (LAN/online) — future Phase 2

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

/// Serialized packet bytes.
pub type Packet = Vec<u8>;

/// FU1/FU3 — the one hard bound on one client's [`InboundQueue`]: 8 MiB of
/// waiting packets (the outbound queue's 8 MiB,
/// `network::MAX_OUTBOUND_QUEUE_BYTES`), each charged its wire length plus
/// [`INBOUND_ENTRY_OVERHEAD`], so a flood of tiny packets reaches it too (about
/// 120,000 empty ones).
///
/// **Why bytes only (FU3, FU1 verify N1).** The backlog a client leaves is not
/// only its own doing. A lending host's server ticks inside the host's frame,
/// while each joiner's QUIC bridge thread (`network::bridge_loop`) keeps
/// reading — and ACKing, so the connection never idles — and pushes every frame
/// into an unbounded channel. A host whose game thread stops (a long save, a
/// loading screen, a debugger, maybe a minimised window) therefore finds every
/// joiner's whole stall in its channel on the first tick after, and the queue
/// takes it in one fill. An honest joiner sends about 20 inputs a second, so
/// FU1's 1,024-packet bound disconnected every joiner after a 51-second host
/// stall. A bare honest input is charged about 150 bytes (90 on the wire),
/// more with acknowledgements, drops and edits, so 8 MiB is a stall of half an
/// hour or more (or a joiner's own freeze of as long, replayed): only a client
/// that floods gets there. A backlog under it drains fast: past
/// `hosted_server::CATCH_UP_QUEUE_LEN` waiting, the server reads
/// `hosted_server::CATCH_UP_PACKETS_PER_TICK` of a client's packets a tick
/// (not ten), unless that client's edit queue is full (FU4a).
pub const MAX_INBOUND_BYTES: usize = 8 * 1024 * 1024;

/// FU3 — what one waiting packet costs against [`MAX_INBOUND_BYTES`] beyond
/// its own bytes: the queue entry and its allocation, rounded up. Makes the
/// byte bound a count bound too for packets of a few bytes.
pub const INBOUND_ENTRY_OVERHEAD: usize = 64;

/// FU1 — how far over its hard bound a client's [`InboundQueue`] went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InboundOverflow {
    /// Packets waiting when the bound was crossed.
    pub packets: usize,
    /// What they are charged ([`MAX_INBOUND_BYTES`]): their bytes plus
    /// [`INBOUND_ENTRY_OVERHEAD`] each.
    pub bytes: usize,
}

/// FU1 — the packets one client has sent that the server has not processed
/// yet, in arrival order. The server reads a client's packets into it every
/// tick, processes at most its per-tick budget from the front, and leaves the
/// rest for the next tick: a packet past the budget waits, it is never
/// dropped. Bounded by [`MAX_INBOUND_BYTES`] (FU3: bytes only, each packet
/// charged [`INBOUND_ENTRY_OVERHEAD`] more): a client that crosses it is one no
/// honest client can be, and is disconnected
/// (`HostedServer::process_inbound_packets`).
#[derive(Default)]
pub struct InboundQueue {
    packets: VecDeque<Packet>,
    /// What the waiting packets are charged: their bytes plus
    /// [`INBOUND_ENTRY_OVERHEAD`] each.
    bytes: usize,
}

/// What `packet` is charged against [`MAX_INBOUND_BYTES`] while it waits.
fn charge(packet: &Packet) -> usize {
    packet.len() + INBOUND_ENTRY_OVERHEAD
}

impl InboundQueue {
    /// Move everything `transport` has received to the back of the queue;
    /// returns how many packets that was. Stops reading, with the overflow, as
    /// soon as the queue is over its hard bound — so a flood costs at most the
    /// bound's worth of work.
    pub fn fill_from(&mut self, transport: &dyn ServerTransport) -> Result<usize, InboundOverflow> {
        let mut arrived = 0;
        while let Some(packet) = transport.try_recv_from_client() {
            self.push(packet)?;
            arrived += 1;
        }
        Ok(arrived)
    }

    fn push(&mut self, packet: Packet) -> Result<(), InboundOverflow> {
        self.bytes += charge(&packet);
        self.packets.push_back(packet);
        if self.bytes > MAX_INBOUND_BYTES {
            return Err(InboundOverflow { packets: self.packets.len(), bytes: self.bytes });
        }
        Ok(())
    }

    /// The oldest packet waiting.
    pub fn front(&self) -> Option<&Packet> {
        self.packets.front()
    }

    /// The newest `n` packets waiting (all of them if fewer), oldest first.
    pub fn newest(&self, n: usize) -> impl Iterator<Item = &Packet> {
        self.packets.iter().skip(self.packets.len().saturating_sub(n))
    }

    /// Take the oldest packet waiting.
    pub fn pop(&mut self) -> Option<Packet> {
        let packet = self.packets.pop_front()?;
        self.bytes -= charge(&packet);
        Some(packet)
    }

    /// Packets waiting.
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    /// Forget everything waiting (the connection ended).
    pub fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
    }
}

/// `Send` on native, nothing on wasm.
///
/// Native transports cross thread boundaries — the QUIC/WebSocket accept thread
/// builds a `Box<dyn ServerTransport>` and hands it to the main loop over an
/// `mpsc` channel, which requires `Send`. WASM is single-threaded and its
/// transport is backed by `web_sys::WebSocket` + `Rc`, which are legitimately
/// `!Send`. Gating the bound this way lets one trait serve both targets without
/// duplicating the definition. (`Send` is a transitive supertrait here, so
/// `dyn ServerTransport: Send` still holds on native.)
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// Server-side handle to one connected client.
pub trait ServerTransport: MaybeSend {
    fn send_to_client(&self, data: &[u8]);
    fn try_recv_from_client(&self) -> Option<Packet>;
    /// End-to-end channel binding for this connection: the QUIC connection's
    /// TLS keying-material exporter (`network::channel_binding_of`). `None` for
    /// a transport without one (in-process channel; WebSocket, whose TLS ends at
    /// the reverse proxy). Feeds `signet::join_origin`.
    fn channel_binding(&self) -> Option<[u8; 32]> {
        None
    }
    /// Whether this connection is a WebSocket. A WS join has no channel
    /// binding; its origin is the host the joiner declares it dialled
    /// (`JoinRequestPacket::ws_host`), checked against the server's public
    /// hosts (`signet::ws_host::expected_ws_join_origin`, protocol v66).
    fn is_websocket(&self) -> bool {
        false
    }
    /// Whether the underlying connection is gone for good (peer closed, read
    /// error, idle timeout). `HostedServer` polls this every tick and frees
    /// the slot, so a laptop lid closing never leaves a ghost player holding a
    /// seat (audit 2026-09-27, "QUIC transport never notices a dropped
    /// connection"). Default `false` for transports that can't tell.
    fn is_closed(&self) -> bool {
        false
    }
}

/// Client-side handle to the server.
pub trait ClientTransport: MaybeSend {
    fn send_to_server(&self, data: &[u8]);
    fn try_recv_from_server(&self) -> Option<Packet>;
    /// The client's view of the same channel binding as
    /// [`ServerTransport::channel_binding`] — equal on both ends of one QUIC
    /// connection. `None` without one.
    fn channel_binding(&self) -> Option<[u8; 32]> {
        None
    }
    /// For a WebSocket: the normalised `host[:port]` actually dialled
    /// (`signet::ws_host::ws_url_host`), which the joiner declares in its
    /// JoinRequest and signs into its join origin. `None` for other transports.
    fn ws_host(&self) -> Option<String> {
        None
    }
    /// Whether the link to the server is gone for good (the host quit,
    /// crashed, dropped us or never answered). `RemoteClient::poll` turns it
    /// into a failed session so the joiner is returned to the lobby instead
    /// of standing in a frozen world. Default `false` for transports that
    /// can't tell.
    fn is_closed(&self) -> bool {
        false
    }
}

// --- Channel transport (in-process, zero-latency) ---

/// Create a paired channel transport for local communication.
/// Returns (server_side, client_side).
pub fn channel_pair() -> (ChannelServerTransport, ChannelClientTransport) {
    let (client_tx, client_rx) = mpsc::channel();
    let (server_tx, server_rx) = mpsc::channel();

    (
        ChannelServerTransport {
            tx: server_tx,
            rx: client_rx,
            closed: AtomicBool::new(false),
        },
        ChannelClientTransport {
            tx: client_tx,
            rx: server_rx,
        },
    )
}

pub struct ChannelServerTransport {
    tx: mpsc::Sender<Packet>,
    rx: mpsc::Receiver<Packet>,
    /// Latched once the client half has been dropped and the queue drained —
    /// the in-process analogue of a closed socket.
    closed: AtomicBool,
}

impl ServerTransport for ChannelServerTransport {
    fn send_to_client(&self, data: &[u8]) {
        let _ = self.tx.send(data.to_vec());
    }

    fn try_recv_from_client(&self) -> Option<Packet> {
        match self.rx.try_recv() {
            Ok(p) => Some(p),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.closed.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

/// What a freed slot holds once its real transport has been dropped (which is
/// what closes the socket). Sends go nowhere, nothing ever arrives, and it
/// reports closed — so a stale index can never reach a new connection.
pub struct ClosedTransport;

impl ServerTransport for ClosedTransport {
    fn send_to_client(&self, _data: &[u8]) {}
    fn try_recv_from_client(&self) -> Option<Packet> {
        None
    }
    fn is_closed(&self) -> bool {
        true
    }
}

/// The server end of a host client's split-screen seat 2.. (D1 review fix 3,
/// `HostedServer::sync_local_slots`): the host client feeds that slot
/// directly and never reads it, so sends go nowhere (nothing piles up in an
/// unread channel) and nothing ever arrives. Unlike [`ClosedTransport`] it is
/// not closed: the seat is live.
// Reached only from native hosting / the dedicated server.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct NullServerTransport;

impl ServerTransport for NullServerTransport {
    fn send_to_client(&self, _data: &[u8]) {}
    fn try_recv_from_client(&self) -> Option<Packet> {
        None
    }
}

pub struct ChannelClientTransport {
    tx: mpsc::Sender<Packet>,
    rx: mpsc::Receiver<Packet>,
}

impl ClientTransport for ChannelClientTransport {
    fn send_to_server(&self, data: &[u8]) {
        let _ = self.tx.send(data.to_vec());
    }

    fn try_recv_from_server(&self) -> Option<Packet> {
        self.rx.try_recv().ok()
    }
}

/// Test-only: a server transport that reports an injected channel binding, so
/// the join-origin check can be driven without a real QUIC connection.
#[cfg(test)]
pub struct BoundServerTransport {
    pub inner: ChannelServerTransport,
    pub binding: Option<[u8; 32]>,
    /// Report `is_websocket()` (stands in for a WebSocket connection).
    pub websocket: bool,
}

#[cfg(test)]
impl ServerTransport for BoundServerTransport {
    fn send_to_client(&self, data: &[u8]) {
        self.inner.send_to_client(data)
    }
    fn try_recv_from_client(&self) -> Option<Packet> {
        self.inner.try_recv_from_client()
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.binding
    }
    fn is_websocket(&self) -> bool {
        self.websocket
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

/// Test-only (FU3, FU1 verify N6): a server transport whose last frame lands
/// just AFTER a fill has emptied the channel, and whose connection closes
/// right behind it — the race a real bridge thread can make with the server's
/// tick (it hands over every frame before it marks the connection closed).
/// The frame put in `late` lands the next time a fill finds the channel empty,
/// and is read by the fill after that.
#[cfg(test)]
pub struct LateFrameServerTransport {
    inner: ChannelServerTransport,
    late: std::sync::Arc<std::sync::Mutex<Option<Packet>>>,
    landed: std::sync::Mutex<Option<Packet>>,
    closed: AtomicBool,
}

#[cfg(test)]
impl LateFrameServerTransport {
    pub fn new(inner: ChannelServerTransport, late: std::sync::Arc<std::sync::Mutex<Option<Packet>>>) -> Self {
        Self { inner, late, landed: std::sync::Mutex::new(None), closed: AtomicBool::new(false) }
    }
}

#[cfg(test)]
impl ServerTransport for LateFrameServerTransport {
    fn send_to_client(&self, data: &[u8]) {
        self.inner.send_to_client(data)
    }
    fn try_recv_from_client(&self) -> Option<Packet> {
        if let Some(p) = self.landed.lock().unwrap().take() {
            return Some(p);
        }
        if let Some(p) = self.inner.try_recv_from_client() {
            return Some(p);
        }
        // The channel is empty: this fill is done. The late frame lands now —
        // too late for it — and the connection closes behind it.
        if let Some(p) = self.late.lock().unwrap().take() {
            *self.landed.lock().unwrap() = Some(p);
            self.closed.store(true, Ordering::Relaxed);
        }
        None
    }
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbound_queue_keeps_arrival_order_and_counts_its_bytes() {
        let (server, client) = channel_pair();
        for n in 1..=3u8 {
            client.send_to_server(&vec![n; n as usize]);
        }
        let mut q = InboundQueue::default();
        assert_eq!(q.fill_from(&server), Ok(3));
        assert_eq!((q.len(), q.bytes), (3, 6 + 3 * INBOUND_ENTRY_OVERHEAD), "bytes plus the per-entry charge");
        assert_eq!(q.pop(), Some(vec![1]));
        client.send_to_server(&[4]);
        assert_eq!(q.fill_from(&server), Ok(1));
        assert_eq!(q.front(), Some(&vec![2, 2]), "the oldest still first");
        assert_eq!(q.newest(2).collect::<Vec<_>>(), vec![&vec![3, 3, 3], &vec![4]]);
        assert_eq!(q.newest(9).count(), 3, "at most what is there");
        let rest: Vec<_> = std::iter::from_fn(|| q.pop()).collect();
        assert_eq!(rest, vec![vec![2, 2], vec![3, 3, 3], vec![4]]);
        assert_eq!((q.len(), q.bytes), (0, 0));
    }

    /// FU3 (FU1 verify N1) — the one hard bound is bytes, each packet
    /// charged its length plus [`INBOUND_ENTRY_OVERHEAD`]: a flood of empty
    /// packets reaches it (no packet count bound of its own), and so does a
    /// few big ones.
    #[test]
    fn the_inbound_queue_overflows_past_its_byte_bound_and_stops_reading() {
        // Tiny packets: the per-entry charge makes the byte bound a count bound.
        let (server, client) = channel_pair();
        let fits = MAX_INBOUND_BYTES / (1 + INBOUND_ENTRY_OVERHEAD);
        for _ in 0..fits {
            client.send_to_server(&[0]);
        }
        let mut q = InboundQueue::default();
        assert_eq!(q.fill_from(&server), Ok(fits), "exactly under the bound is still honest");
        assert!(fits > 100_000, "far past the old 1,024-packet bound: {fits}");
        client.send_to_server(&[0]);
        client.send_to_server(&[0]);
        let charged = (fits + 1) * (1 + INBOUND_ENTRY_OVERHEAD);
        assert_eq!(q.fill_from(&server), Err(InboundOverflow { packets: fits + 1, bytes: charged }));
        assert!(charged > MAX_INBOUND_BYTES);
        assert!(server.try_recv_from_client().is_some(), "it stopped reading at the bound");

        // Big packets.
        let (server, client) = channel_pair();
        let big = vec![0u8; MAX_INBOUND_BYTES / 2 - INBOUND_ENTRY_OVERHEAD];
        let mut q = InboundQueue::default();
        client.send_to_server(&big);
        client.send_to_server(&big);
        assert_eq!(q.fill_from(&server), Ok(2));
        assert_eq!(q.bytes, MAX_INBOUND_BYTES, "two halves, charged, fill it exactly");
        client.send_to_server(&[]);
        assert_eq!(
            q.fill_from(&server).map_err(|o| o.bytes),
            Err(MAX_INBOUND_BYTES + INBOUND_ENTRY_OVERHEAD),
            "even an empty packet costs its entry"
        );
        q.clear();
        assert!(q.is_empty() && q.bytes == 0);
    }
}

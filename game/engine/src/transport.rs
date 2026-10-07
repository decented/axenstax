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

/// FU1 — the hard bound on one client's [`InboundQueue`], in packets. An
/// honest client sends about 20 a second (one input per tick it runs, plus a
/// few actions), and after a frame hitch it catches up at up to ten a frame
/// (`game_loop` runs at most 10 ticks a frame and banks the rest), while the
/// server reads `hosted_server::MAX_PACKETS_PER_TICK` = 10 of them a tick, 200 a
/// second. Its queue therefore grows only while a burst arrives faster than
/// that: a QUIC stall is at most 30 seconds (quinn's idle timeout ends the
/// connection) — some 600 inputs plus actions — and a game-thread freeze of
/// `T` seconds queues at most the `20·T` inputs it then catches up (fewer: the
/// server reads ten a tick while they arrive). 1024 is a freeze of most of a
/// minute; only a client that floods gets there.
pub const MAX_INBOUND_PACKETS: usize = 1024;

/// FU1 — the hard bound on one client's [`InboundQueue`], in bytes (the
/// outbound queue's 8 MiB, `network::MAX_OUTBOUND_QUEUE_BYTES`). Honest inputs
/// are about a hundred bytes; even a client's largest edit burst
/// (`RemoteClient::send_input`'s carry-over) is a handful of full packets.
pub const MAX_INBOUND_BYTES: usize = 8 * 1024 * 1024;

/// FU1 — how far over its hard bound a client's [`InboundQueue`] went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InboundOverflow {
    /// Packets waiting when the bound was crossed.
    pub packets: usize,
    /// Their bytes.
    pub bytes: usize,
}

/// FU1 — the packets one client has sent that the server has not processed
/// yet, in arrival order. The server reads a client's packets into it every
/// tick, processes at most its per-tick budget from the front, and leaves the
/// rest for the next tick: a packet past the budget waits, it is never
/// dropped. Bounded by [`MAX_INBOUND_PACKETS`] and [`MAX_INBOUND_BYTES`]: a
/// client that crosses either is one no honest client can be, and is
/// disconnected (`HostedServer::process_inbound_packets`).
#[derive(Default)]
pub struct InboundQueue {
    packets: VecDeque<Packet>,
    bytes: usize,
}

impl InboundQueue {
    /// Move everything `transport` has received to the back of the queue;
    /// returns how many packets that was. Stops reading, with the overflow, as
    /// soon as the queue is over either hard bound — so a flood costs at most
    /// the bound's worth of work.
    pub fn fill_from(&mut self, transport: &dyn ServerTransport) -> Result<usize, InboundOverflow> {
        let mut arrived = 0;
        while let Some(packet) = transport.try_recv_from_client() {
            self.push(packet)?;
            arrived += 1;
        }
        Ok(arrived)
    }

    fn push(&mut self, packet: Packet) -> Result<(), InboundOverflow> {
        self.bytes += packet.len();
        self.packets.push_back(packet);
        if self.packets.len() > MAX_INBOUND_PACKETS || self.bytes > MAX_INBOUND_BYTES {
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
        self.bytes -= packet.len();
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
        assert_eq!((q.len(), q.bytes), (3, 6));
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

    #[test]
    fn the_inbound_queue_overflows_past_either_bound_and_stops_reading() {
        let (server, client) = channel_pair();
        for _ in 0..MAX_INBOUND_PACKETS {
            client.send_to_server(&[0]);
        }
        let mut q = InboundQueue::default();
        assert_eq!(q.fill_from(&server), Ok(MAX_INBOUND_PACKETS), "exactly at the bound is still honest");
        client.send_to_server(&[0]);
        client.send_to_server(&[0]);
        assert_eq!(
            q.fill_from(&server),
            Err(InboundOverflow { packets: MAX_INBOUND_PACKETS + 1, bytes: MAX_INBOUND_PACKETS + 1 })
        );
        assert!(server.try_recv_from_client().is_some(), "it stopped reading at the bound");

        let (server, client) = channel_pair();
        let big = vec![0u8; MAX_INBOUND_BYTES / 2];
        let mut q = InboundQueue::default();
        client.send_to_server(&big);
        client.send_to_server(&big);
        assert_eq!(q.fill_from(&server), Ok(2));
        client.send_to_server(&[0]);
        assert_eq!(q.fill_from(&server).map_err(|o| o.bytes), Err(MAX_INBOUND_BYTES + 1));
        q.clear();
        assert!(q.is_empty() && q.bytes == 0);
    }
}

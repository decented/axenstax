//! Transport abstraction — connects clients to the server.
//!
//! Implementations:
//! - ChannelTransport: in-process mpsc channels (split screen, host-as-server local)
//! - NetworkTransport: UDP sockets (LAN/online) — future Phase 2

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

/// Serialized packet bytes.
pub type Packet = Vec<u8>;

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

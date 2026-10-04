//! WebSocket transport (native) — the dedicated-server pipe that BOTH the
//! browser PWA and the native client speak.
//!
//! Plain `ws` only: TLS for the browser is terminated by the Caddy front in the
//! Docker image, so the engine never needs a TLS stack here. The browser uses
//! `web_sys::WebSocket` (see `ws_transport_web.rs`); this module is the native
//! half — the server accept loop plus a native client connector.
//!
//! It mirrors `network.rs` (QUIC): an accept thread runs a tokio runtime,
//! accepts connections, and hands each to `HostedServer` as an opaque
//! `Box<dyn ServerTransport>` over the same mpsc channel the QUIC accept thread
//! uses. The hosted server never learns which transport a client arrived on.
//!
//! WebSocket is reliable + ordered + unbounded-per-message, which is strictly
//! better than the QUIC *datagram* path for `ChunkData` delivery (the datagram
//! path silently caps payloads at the QUIC MTU, ~1200 bytes).

#![cfg(not(target_arch = "wasm32"))]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::transport::{ClientTransport, Packet, ServerTransport};

/// Default plain-WebSocket port the dedicated server listens on. Caddy
/// reverse-proxies `wss://host/ws` here; native clients may also connect
/// directly as `ws://host:6767`. (6767 = the "6-7" meme number — memorable,
/// in the registered/unprivileged range, and clash-free with common services.)
pub const DEFAULT_WS_PORT: u16 = 6767;

// ───────────────────────── server side (one client) ─────────────────────────

/// Server-side handle to one WebSocket-connected client. The reader/writer
/// tasks live on the accept thread's runtime; this struct holds only the
/// channel ends the (synchronous) game loop polls.
pub struct WebSocketServerTransport {
    outbound: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    inbound: mpsc::Receiver<Packet>,
    /// Set by the reader task once the socket is gone.
    closed: Arc<AtomicBool>,
}

impl ServerTransport for WebSocketServerTransport {
    fn send_to_client(&self, data: &[u8]) {
        let _ = self.outbound.send(data.to_vec());
    }
    fn try_recv_from_client(&self) -> Option<Packet> {
        self.inbound.try_recv().ok()
    }
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

/// How long one accepted TCP connection gets to finish its WebSocket upgrade.
/// Each upgrade runs in its own task, so a peer that connects and says nothing
/// only ever holds up itself (audit 2026-09-27: one idle socket used to block
/// every later join for good).
pub const WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

// ─────────────────────── client side (native joiner) ────────────────────────

/// Client-side handle to a remote WebSocket server (native joiner).
pub struct WebSocketClientTransport {
    outbound: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    inbound: mpsc::Receiver<Packet>,
    _thread: Option<thread::JoinHandle<()>>,
}

impl ClientTransport for WebSocketClientTransport {
    fn send_to_server(&self, data: &[u8]) {
        let _ = self.outbound.send(data.to_vec());
    }
    fn try_recv_from_server(&self) -> Option<Packet> {
        self.inbound.try_recv().ok()
    }
    /// The bridge thread returns when the socket closes or errors.
    fn is_closed(&self) -> bool {
        self._thread.as_ref().is_some_and(|h| h.is_finished())
    }
}

// ─────────────────────────────── bridges ────────────────────────────────────

/// Bridge an accepted server-side WebSocket into a `ServerTransport`. Spawns a
/// reader task (socket → inbound mpsc) and a writer task (outbound mpsc →
/// socket) on the CURRENT tokio runtime. On close/error the reader injects a
/// synthetic `Disconnect` packet so `HostedServer` releases the slot.
fn bridge_ws_server<S>(ws: tokio_tungstenite::WebSocketStream<S>) -> WebSocketServerTransport
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (in_tx, in_rx) = mpsc::channel::<Packet>();
    let (mut sink, mut stream) = ws.split();
    let closed = Arc::new(AtomicBool::new(false));
    let closed_reader = closed.clone();

    // Writer: drain the outbound queue onto the socket. When the game side
    // drops the transport (slot freed), flush and send a Close frame.
    tokio::spawn(async move {
        while let Some(data) = out_rx.recv().await {
            // tungstenite 0.26: `Binary` holds `Bytes`; `Vec<u8>` converts losslessly.
            if sink.send(Message::Binary(data.into())).await.is_err() {
                return;
            }
        }
        let _ = sink.close().await;
    });

    // Reader: socket → inbound queue; synthesize a Disconnect on close.
    tokio::spawn(async move {
        while let Some(item) = stream.next().await {
            match item {
                // `.to_vec()` works for Vec<u8> and Bytes payloads alike.
                Ok(Message::Binary(b)) => {
                    if in_tx.send(b.to_vec()).is_err() {
                        break;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {} // text/ping/pong — ignored (constant 20 TPS traffic keeps it warm)
            }
        }
        closed_reader.store(true, Ordering::Relaxed);
        let dc =
            crate::protocol::serialize_packet(crate::protocol::PacketType::Disconnect, &());
        let _ = in_tx.send(dc);
    });

    WebSocketServerTransport { outbound: out_tx, inbound: in_rx, closed }
}

/// Spawn the WebSocket accept thread. Mirrors
/// `hosted_server::spawn_quic_accept_thread`: a tokio runtime accepts
/// connections up to `max_remote_players` and pushes each as a
/// `Box<dyn ServerTransport>` to the hosted server.
pub fn spawn_ws_accept_thread(
    port: u16,
    max_remote_players: usize,
    current_remote: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    remote_tx: mpsc::Sender<Box<dyn ServerTransport>>,
) -> Result<thread::JoinHandle<()>, String> {
    thread::Builder::new()
        .name("ws-accept".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime for ws accept loop");

            rt.block_on(async move {
                let bind_addr: SocketAddr =
                    format!("0.0.0.0:{port}").parse().expect("valid bind address");
                let listener = match TcpListener::bind(bind_addr).await {
                    Ok(l) => {
                        log::info!("WebSocket server listening on {bind_addr}");
                        l
                    }
                    Err(e) => {
                        log::error!("Failed to bind WebSocket port {port}: {e}");
                        return;
                    }
                };

                while !shutdown.load(Ordering::Relaxed) {
                    match tokio::time::timeout(Duration::from_secs(1), listener.accept()).await {
                        Ok(Ok((stream, peer))) => {
                            // Admission seam (Spec A §6) — today HardCap; Queue /
                            // OneInOneOut / BitcoinGated plug in here later. Routed
                            // through the seam so the cap isn't a buried `if`.
                            if let crate::admission::Admission::Reject(reason) =
                                crate::admission::decide(
                                    crate::admission::AdmissionPolicy::HardCap,
                                    current_remote.load(Ordering::Relaxed),
                                    max_remote_players,
                                )
                            {
                                log::info!("Rejecting WS connection from {peer} — {reason}");
                                continue;
                            }
                            // Reserve the seat now so a burst of concurrent
                            // upgrades can't overshoot the cap; handed back if
                            // the upgrade fails or times out.
                            current_remote.fetch_add(1, Ordering::Relaxed);
                            let remote_tx = remote_tx.clone();
                            let current_remote = current_remote.clone();
                            tokio::spawn(async move {
                                let upgraded = tokio::time::timeout(
                                    WS_HANDSHAKE_TIMEOUT,
                                    tokio_tungstenite::accept_async(stream),
                                )
                                .await;
                                let handed_over = match upgraded {
                                    Ok(Ok(ws)) => {
                                        log::info!("WebSocket player connected from {peer}");
                                        let transport = bridge_ws_server(ws);
                                        remote_tx.send(Box::new(transport)).is_ok()
                                    }
                                    Ok(Err(e)) => {
                                        log::warn!("WS handshake failed from {peer}: {e}");
                                        false
                                    }
                                    Err(_) => {
                                        log::warn!("WS handshake from {peer} timed out");
                                        false
                                    }
                                };
                                if !handed_over {
                                    crate::admission::release_seat(&current_remote);
                                }
                            });
                        }
                        Ok(Err(e)) => log::warn!("WS accept error: {e}"),
                        Err(_) => {} // timeout → re-check shutdown flag
                    }
                }
                log::info!("WebSocket accept loop stopped");
            });
        })
        .map_err(|e| format!("Failed to spawn WebSocket accept thread: {e}"))
}

/// Connect (native) to a `ws://` server. Returns immediately; the connection is
/// established on a background thread (mirrors `network::connect_to_server`).
/// Bytes queued via `send_to_server` before the socket opens are flushed once
/// connected. Native joiners use `ws://host:6767`; the browser uses the
/// Caddy-fronted `wss://host:8443/ws`.
pub fn connect_ws(url: &str) -> Result<WebSocketClientTransport, String> {
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (in_tx, in_rx) = mpsc::channel::<Packet>();
    let url = url.to_string();

    let handle = thread::Builder::new()
        .name("ws-client".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime for ws client");

            rt.block_on(async move {
                let (ws, _resp) = match tokio_tungstenite::connect_async(&url).await {
                    Ok(x) => x,
                    Err(e) => {
                        log::error!("WebSocket connect to {url} failed: {e}");
                        return;
                    }
                };
                log::info!("Connected to server at {url}");
                let (mut sink, mut stream) = ws.split();
                loop {
                    tokio::select! {
                        item = stream.next() => match item {
                            Some(Ok(Message::Binary(b))) => { let _ = in_tx.send(b.to_vec()); }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            _ => {}
                        },
                        out = out_rx.recv() => match out {
                            Some(data) => {
                                if sink.send(Message::Binary(data.into())).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
                log::info!("WebSocket client bridge closed");
            });
        })
        .map_err(|e| format!("Failed to spawn ws client thread: {e}"))?;

    Ok(WebSocketClientTransport { outbound: out_tx, inbound: in_rx, _thread: Some(handle) })
}

#[cfg(test)]
mod tests {
    use crate::hosted_server::{HostedServer, RemoteTransport};
    use crate::remote_client::{ConnectionState, RemoteClient};
    use std::time::Duration;

    /// End-to-end native WebSocket join: a dedicated server (0 local players)
    /// accepts a WebSocket client, completes the handshake, and hands back the
    /// world's REAL seed (gap G3 — was a hardcoded 42). Drives the full path:
    /// accept thread → `bridge_ws_server` → `HostedServer` → `JoinAccept` →
    /// `RemoteClient::poll`. Uses real localhost sockets, so it pumps with a
    /// generous timeout rather than asserting on a single tick.
    #[test]
    fn websocket_client_joins_and_receives_real_seed() {
        let _guard = crate::save::WorldsRootGuard::new("ws-join");
        let port = 47137; // fixed, unusual; only one such test runs.
        let seed: u32 = 0x00AB_CDEF;

        let mut hs = HostedServer::start(
            0,
            "ws-join-world".into(),
            seed,
            2,
            RemoteTransport::WebSocket { port },
        )
        .expect("start dedicated WebSocket server");

        // Let the accept thread bind the listener before connecting.
        std::thread::sleep(Duration::from_millis(400));

        let mut client =
            RemoteClient::connect_websocket(&format!("ws://127.0.0.1:{port}"), "Tester", None)
                .expect("connect WebSocket client");

        // Pump server + client until joined (or time out at ~6s).
        let mut joined_seed: Option<u32> = None;
        for _ in 0..300 {
            hs.tick();
            client.poll();
            if let ConnectionState::Connected { seed: s, .. } = &client.state {
                joined_seed = Some(*s);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        hs.shutdown();
        assert_eq!(
            joined_seed,
            Some(seed),
            "WebSocket client must reach Connected with the server's REAL seed (G3)"
        );
    }

    /// Audit 2026-09-27: `accept_async` ran inline in the single accept loop
    /// with no timeout, so one TCP connection that never sent its upgrade
    /// blocked every later join until restart. With the idle socket still
    /// open, a real client must join.
    #[test]
    fn an_idle_tcp_connection_does_not_block_a_real_join() {
        let _guard = crate::save::WorldsRootGuard::new("ws-idle");
        let port = 47139; // fixed, unusual; distinct from the test above.

        let mut hs = HostedServer::start(
            0,
            "ws-idle-world".into(),
            7,
            2,
            RemoteTransport::WebSocket { port },
        )
        .expect("start dedicated WebSocket server");
        std::thread::sleep(Duration::from_millis(400));

        // Connect and say nothing — held open for the whole test.
        let _idle = std::net::TcpStream::connect(("127.0.0.1", port)).expect("idle tcp");
        std::thread::sleep(Duration::from_millis(200));

        let mut client =
            RemoteClient::connect_websocket(&format!("ws://127.0.0.1:{port}"), "Tester", None)
                .expect("connect WebSocket client");
        let mut joined = false;
        for _ in 0..200 {
            hs.tick();
            client.poll();
            if matches!(client.state, ConnectionState::Connected { .. }) {
                joined = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        hs.shutdown();
        assert!(joined, "a real client must join while an idle socket is open");
    }
}

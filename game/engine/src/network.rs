//! QUIC network transport -- connects game clients to servers over the network.
//!
//! Uses quinn (QUIC) for transport with TLS 1.3 encryption.
//! The async QUIC I/O runs on a background tokio thread and bridges
//! to the synchronous game loop via mpsc channels. Every game packet rides one
//! reliable, ordered, length-prefixed bidirectional stream per connection
//! (`u32` LE length + payload, max [`MAX_FRAME_LEN`]); the client opens it.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

use crate::transport::{ClientTransport, Packet, ServerTransport};

// --- Configuration ---

/// Default server port for QUIC connections. No consumer references this
/// named constant — the literal `7700` is used directly wherever needed.
#[allow(dead_code)]
pub const DEFAULT_PORT: u16 = 7700;

/// ALPN protocol identifier.
const ALPN_PROTOCOL: &[u8] = b"axenstax-v1";

/// TLS exporter label for the join channel binding (RFC 5705 / RFC 8446 §7.5).
const JOIN_EXPORTER_LABEL: &[u8] = b"EXPORTER-axenstax-join-v1";

/// The join channel binding of a live QUIC connection: 32 bytes from the TLS
/// keying-material exporter. Both ends of one connection derive the same bytes;
/// two different TLS sessions (a relaying host's two legs) never do — and that
/// holds although certificate verification is skipped. Feeds
/// `signet::join_origin`. An exporter failure yields `None`, which a
/// sign-in-required host then rejects (fail closed).
pub fn channel_binding_of(conn: &quinn::Connection) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    match conn.export_keying_material(&mut out, JOIN_EXPORTER_LABEL, b"") {
        Ok(()) => Some(out),
        Err(e) => {
            log::warn!("QUIC channel binding unavailable: {e:?}");
            None
        }
    }
}

// --- Self-signed certificate generation (LAN only) ---

/// Generate a self-signed certificate for LAN play.
/// Returns (certificate_der, private_key_der).
pub fn generate_self_signed_cert() -> (Vec<u8>, Vec<u8>) {
    let cert = rcgen::generate_simple_self_signed(vec!["axenstax-server".to_string()])
        .expect("certificate generation failed");
    let cert_der = cert.cert.der().to_vec();
    let key_der = cert.key_pair.serialize_der();
    (cert_der, key_der)
}

// --- Stream framing ---

/// Largest game packet either end will put on (or accept from) the wire. A
/// `JoinAccept` carrying a world's exhibits or a busy `StateUpdate` is tens of
/// KiB at most; 16 MiB is a generous ceiling that still stops a peer from
/// making us allocate without bound off a forged length prefix.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// How long a freshly-accepted connection has to open its game stream before
/// the server gives up on it (matches `HostedServer`'s pre-auth timeout).
const STREAM_OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Most bytes the server will hold queued for ONE client. A joiner that stops
/// reading its stream still ACKs at the transport level, so it is never
/// idle-closed; without a bound its queue grows ~20 StateUpdates a second
/// until the host runs out of memory (review S2). Over the bound the client is
/// disconnected ("connection too slow").
pub const MAX_OUTBOUND_QUEUE_BYTES: usize = 8 * 1024 * 1024;

/// One frame on the wire: `u32` little-endian payload length, then the payload.
/// `None` if the payload is over [`MAX_FRAME_LEN`] (never sent).
fn encode_frame(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > MAX_FRAME_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

/// Read one frame. `Ok(None)` is a clean end of stream between frames; a stream
/// that ends mid-frame, an oversized length, or a read error is `Err`.
async fn read_frame(recv: &mut quinn::RecvStream) -> Result<Option<Packet>, String> {
    let mut len_buf = [0u8; 4];
    match recv.read_exact(&mut len_buf).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(format!("frame header: {e}")),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        return Err(format!("frame of {len} bytes exceeds the {MAX_FRAME_LEN}-byte cap"));
    }
    // Grow as bytes arrive rather than allocating `len` up front, so a bare
    // header can't pin 16 MiB per connection.
    let mut buf = Vec::with_capacity(len.min(64 * 1024));
    let mut chunk = [0u8; 8 * 1024];
    while buf.len() < len {
        let want = (len - buf.len()).min(chunk.len());
        recv.read_exact(&mut chunk[..want])
            .await
            .map_err(|e| format!("frame body: {e}"))?;
        buf.extend_from_slice(&chunk[..want]);
    }
    Ok(Some(buf))
}

// --- Server-side QUIC transport ---

/// Server-side QUIC transport for one connected client. The game packets ride
/// ONE reliable, ordered bidirectional stream (length-prefixed frames); the
/// async side lives on the bridge thread and talks to the game loop through
/// these channel ends.
pub struct QuicServerTransport {
    /// Send data to the remote client (game thread -> network thread -> QUIC).
    /// Dropping it (the slot was freed) makes the bridge flush and close.
    tx: tokio::sync::mpsc::UnboundedSender<Packet>,
    /// Receive data from the remote client (QUIC -> network thread -> game thread)
    rx: mpsc::Receiver<Packet>,
    /// TLS-exporter channel binding, computed once at bridge time.
    binding: Option<[u8; 32]>,
    /// Set by the bridge once the connection is gone.
    closed: Arc<AtomicBool>,
    /// Bytes queued for the writer and not yet taken by it.
    queued: Arc<AtomicUsize>,
    /// Held so an over-full queue can close the connection from this thread.
    conn: quinn::Connection,
}

impl ServerTransport for QuicServerTransport {
    fn send_to_client(&self, data: &[u8]) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let queued = self.queued.fetch_add(data.len(), Ordering::Relaxed) + data.len();
        if queued > MAX_OUTBOUND_QUEUE_BYTES {
            log::warn!(
                "QUIC: {queued} bytes queued for a client that isn't reading; disconnecting it"
            );
            self.closed.store(true, Ordering::Relaxed);
            self.conn.close(1u32.into(), b"connection too slow");
            return;
        }
        let _ = self.tx.send(data.to_vec());
    }

    fn try_recv_from_client(&self) -> Option<Packet> {
        self.rx.try_recv().ok()
    }

    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.binding
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

// --- Client-side QUIC transport ---

/// Client-side QUIC transport for connecting to a remote server.
pub struct QuicClientTransport {
    /// Send data to the server (game thread -> network thread -> QUIC)
    tx: tokio::sync::mpsc::UnboundedSender<Packet>,
    /// Receive data from the server (QUIC -> network thread -> game thread)
    rx: mpsc::Receiver<Packet>,
    /// Handle to the network thread (for cleanup)
    _network_thread: Option<std::thread::JoinHandle<()>>,
    /// TLS-exporter channel binding. The transport is handed back before the
    /// connection exists, so the network thread fills this in once the
    /// handshake completes — BEFORE it starts pumping packets, so any packet
    /// the game thread reads (the `Challenge` included) implies it is set.
    binding: Arc<OnceLock<[u8; 32]>>,
}

impl ClientTransport for QuicClientTransport {
    fn send_to_server(&self, data: &[u8]) {
        let _ = self.tx.send(data.to_vec());
    }

    fn try_recv_from_server(&self) -> Option<Packet> {
        self.rx.try_recv().ok()
    }

    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.binding.get().copied()
    }

    /// The network thread returns once the connection ends (read error,
    /// peer close, idle timeout) or never came up.
    fn is_closed(&self) -> bool {
        self._network_thread.as_ref().is_some_and(|h| h.is_finished())
    }
}

// --- Skip server certificate verification (LAN only) ---

/// Accept any server certificate. Only for LAN play where we use self-signed certs.
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

// --- Server endpoint ---

/// The server-side TLS + QUIC config. Lifted out of `create_server_endpoint` so
/// the pre-bound-socket variant is the same configuration, not a second copy of
/// it.
fn server_quinn_config() -> Result<quinn::ServerConfig, Box<dyn std::error::Error>> {
    let (cert_der, key_der) = generate_self_signed_cert();
    let cert = rustls::pki_types::CertificateDer::from(cert_der);
    let key = rustls::pki_types::PrivateKeyDer::try_from(key_der)
        .map_err(|e| format!("invalid key: {e}"))?;
    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    server_crypto.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];
    Ok(quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)?,
    )))
}

/// The client-side QUIC config. Certificate verification stays skipped: hosts
/// are self-signed, and the trust anchor is the persona-signed kind-21236 join
/// event (plus the server-identity proof where the host has one), not the TLS
/// certificate.
fn client_quinn_config() -> Result<quinn::ClientConfig, Box<dyn std::error::Error>> {
    let mut client_crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();
    client_crypto.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];
    Ok(quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)?,
    )))
}

/// Create a QUIC server endpoint bound to the given address.
/// Returns the endpoint which can accept incoming connections.
pub fn create_server_endpoint(
    bind_addr: SocketAddr,
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let endpoint = quinn::Endpoint::server(server_quinn_config()?, bind_addr)?;
    log::info!("QUIC server listening on {bind_addr}");
    Ok(endpoint)
}

/// Create a QUIC server endpoint on an **already-bound** socket.
///
/// This is the online-play path: the socket was bound first, then used to
/// gather candidates (including its STUN reflexive mapping) and to punch
/// through the router, and only then handed here — so the address the peer was
/// told to dial is the address QUIC answers on. Must be called inside a tokio
/// runtime (`default_runtime` needs one).
pub fn create_server_endpoint_on_socket(
    socket: std::net::UdpSocket,
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let local = socket.local_addr()?;
    let runtime = quinn::default_runtime()
        .ok_or("no async runtime for quinn — call this inside a tokio context")?;
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(server_quinn_config()?),
        socket,
        runtime,
    )?;
    log::info!("QUIC server listening on the pre-bound socket at {local}");
    Ok(endpoint)
}

/// Create a QUIC client endpoint (binds to 0.0.0.0:0).
pub fn create_client_endpoint() -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_quinn_config()?);
    Ok(endpoint)
}

/// Create a QUIC client endpoint on an already-bound socket — the joiner half
/// of the handoff described on [`create_server_endpoint_on_socket`].
pub fn create_client_endpoint_on_socket(
    socket: std::net::UdpSocket,
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let runtime = quinn::default_runtime()
        .ok_or("no async runtime for quinn — call this inside a tokio context")?;
    let mut endpoint =
        quinn::Endpoint::new(quinn::EndpointConfig::default(), None, socket, runtime)?;
    endpoint.set_default_client_config(client_quinn_config()?);
    Ok(endpoint)
}

// --- Connection bridge (async QUIC <-> sync game loop via mpsc) ---

/// Pump length-prefixed frames between the game stream of a live QUIC
/// connection and the game thread's channels until either side goes away.
/// Shared by every transport bridge so the read/write loop exists once.
///
/// Every game packet rides this one reliable, ordered stream (audit
/// 2026-09-27: the old datagram path silently dropped anything over the path
/// MTU — every busy `StateUpdate`, most `JoinAccept`s — and lost small packets
/// with no retransmit). Head-of-line blocking is accepted at co-op scale.
///
/// Ends when the peer closes/errors (reader side) or when the game side drops
/// its sender (writer side — the slot was freed); in the latter case queued
/// frames, e.g. a `JoinReject`, are flushed and acknowledged before the
/// connection closes. On exit `closed` is set and `on_close` (the server's
/// synthetic `Disconnect`) is delivered to the game thread.
async fn bridge_loop(
    conn: quinn::Connection,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    net_tx: mpsc::Sender<Packet>,
    mut net_rx: tokio::sync::mpsc::UnboundedReceiver<Packet>,
    closed: Arc<AtomicBool>,
    on_close: Option<Packet>,
    queued: Option<Arc<AtomicUsize>>,
) {
    let reader = async {
        loop {
            match read_frame(&mut recv).await {
                // Zero-length frames are the opener's "hello" (a stream is
                // invisible to the peer until something is written on it).
                Ok(Some(p)) if p.is_empty() => continue,
                Ok(Some(p)) => {
                    if net_tx.send(p).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    log::warn!("QUIC read error: {e}");
                    break;
                }
            }
        }
    };
    let writer = async {
        while let Some(data) = net_rx.recv().await {
            if let Some(q) = &queued {
                q.fetch_sub(data.len(), Ordering::Relaxed);
            }
            let Some(frame) = encode_frame(&data) else {
                // Dropping it silently could leave the peer waiting forever
                // (a JoinAccept): close instead, so both sides see the end.
                log::warn!("QUIC: a {}-byte packet is over the frame cap; closing", data.len());
                return;
            };
            if let Err(e) = send.write_all(&frame).await {
                log::warn!("QUIC send error: {e}");
                return;
            }
        }
        // The game side let go: flush, then wait (bounded) for the peer to
        // acknowledge everything before the connection is torn down.
        let _ = send.finish();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), send.stopped()).await;
    };
    tokio::select! {
        _ = reader => {}
        _ = writer => {}
    }
    conn.close(0u32.into(), b"closed");
    closed.store(true, Ordering::Relaxed);
    if let Some(pkt) = on_close {
        let _ = net_tx.send(pkt);
    }
    log::info!("QUIC connection bridge closed");
}

/// Client half of the stream handshake: open the game stream and write the
/// zero-length hello frame that makes it visible to the server.
async fn open_game_stream(
    conn: &quinn::Connection,
) -> Result<(quinn::SendStream, quinn::RecvStream), String> {
    let (mut send, recv) = conn.open_bi().await.map_err(|e| format!("open stream: {e}"))?;
    send.write_all(&0u32.to_le_bytes())
        .await
        .map_err(|e| format!("stream hello: {e}"))?;
    Ok((send, recv))
}

/// Spawn a background tokio runtime that bridges a QUIC connection
/// to mpsc channels for the synchronous game loop.
///
/// Returns a `QuicServerTransport` implementing `ServerTransport`. Packets the
/// game queues before the client has opened its stream (the join `Challenge`)
/// wait in the channel. A connection that never opens a stream within
/// [`STREAM_OPEN_TIMEOUT`], or that ends for any reason, reports closed and
/// delivers a synthetic `Disconnect`, so `HostedServer` frees the slot.
pub fn bridge_server_connection(connection: quinn::Connection) -> QuicServerTransport {
    // game thread -> network thread (outbound)
    let (game_tx, net_rx) = tokio::sync::mpsc::unbounded_channel::<Packet>();
    // network thread -> game thread (inbound)
    let (net_tx, game_rx) = mpsc::channel::<Packet>();
    // The handshake is complete (the accept loop awaited it), so the exporter
    // is available now.
    let binding = channel_binding_of(&connection);
    let closed = Arc::new(AtomicBool::new(false));
    let closed_net = closed.clone();
    let queued = Arc::new(AtomicUsize::new(0));
    let queued_net = queued.clone();
    let conn = connection.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        rt.block_on(async move {
            let disconnect = crate::protocol::serialize_packet(
                crate::protocol::PacketType::Disconnect,
                &(),
            );
            match tokio::time::timeout(STREAM_OPEN_TIMEOUT, connection.accept_bi()).await {
                Ok(Ok((send, recv))) => {
                    bridge_loop(
                        connection,
                        send,
                        recv,
                        net_tx,
                        net_rx,
                        closed_net,
                        Some(disconnect),
                        Some(queued_net),
                    )
                    .await;
                }
                outcome => {
                    match outcome {
                        Ok(Err(e)) => log::warn!("QUIC: no game stream: {e}"),
                        _ => log::warn!("QUIC: peer never opened its game stream"),
                    }
                    connection.close(0u32.into(), b"no stream");
                    closed_net.store(true, Ordering::Relaxed);
                    let _ = net_tx.send(disconnect);
                }
            }
        });
    });

    QuicServerTransport {
        tx: game_tx,
        rx: game_rx,
        binding,
        closed,
        queued,
        conn,
    }
}

/// Connect to a remote server and return a client transport.
///
/// Spawns a background thread with a tokio runtime that manages the QUIC connection.
pub fn connect_to_server(
    server_addr: SocketAddr,
) -> Result<QuicClientTransport, Box<dyn std::error::Error>> {
    // game thread -> network thread (outbound)
    let (game_tx, net_rx) = tokio::sync::mpsc::unbounded_channel::<Packet>();
    // network thread -> game thread (inbound)
    let (net_tx, game_rx) = mpsc::channel::<Packet>();
    let binding = Arc::new(OnceLock::new());
    let binding_net = binding.clone();

    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        rt.block_on(async move {
            // Create endpoint inside tokio runtime (quinn needs it)
            let endpoint = match create_client_endpoint() {
                Ok(ep) => ep,
                Err(e) => {
                    log::error!("Failed to create client endpoint: {e}");
                    return;
                }
            };

            let conn = match endpoint.connect(server_addr, "axenstax-server") {
                Ok(connecting) => match connecting.await {
                    Ok(conn) => conn,
                    Err(e) => {
                        log::error!("QUIC connect failed: {e}");
                        return;
                    }
                },
                Err(e) => {
                    log::error!("QUIC connect error: {e}");
                    return;
                }
            };

            log::info!("Connected to server at {server_addr}");
            if let Some(b) = channel_binding_of(&conn) {
                let _ = binding_net.set(b);
            }
            let (send, recv) = match open_game_stream(&conn).await {
                Ok(s) => s,
                Err(e) => {
                    log::error!("QUIC {e}");
                    return;
                }
            };

            let closed = Arc::new(AtomicBool::new(false));
            bridge_loop(conn, send, recv, net_tx, net_rx, closed, None, None).await;
        });
    });

    Ok(QuicClientTransport {
        tx: game_tx,
        rx: game_rx,
        _network_thread: Some(handle),
        binding,
    })
}

// --- Online play by contact: punch, then race the candidates ---

/// How long one candidate gets to complete its handshake before the race writes
/// it off. Deliberately a per-attempt budget rather than a shorter idle timeout
/// on `client_quinn_config`, which also governs the WINNING connection once
/// gameplay is running — a dead candidate must lose fast without making a live
/// one fragile. Comfortably under the 8 s
/// [`crate::nat::punch::CONNECT_DEADLINE`], so a single dead candidate ends the
/// race as `AllFailed` rather than dragging it to `TimedOut`.
const ATTEMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// A client transport plus a one-shot report of how the connect race went.
///
/// The transport is returned immediately (like `connect_to_server`) because the
/// game loop wants a handle straight away; `outcome` is how the caller learns
/// whether any candidate actually won, which is what decides between "you're
/// in" and the "couldn't reach" copy.
pub struct OnlineConnect {
    pub transport: QuicClientTransport,
    pub outcome: mpsc::Receiver<Result<SocketAddr, String>>,
}

/// Punch, then race a QUIC connect across every candidate on the pre-bound
/// socket, best candidate first.
///
/// The whole race runs on the worker thread: punching sleeps, and connecting
/// blocks on a handshake, neither of which may happen on the game loop.
pub fn connect_to_server_on_socket(
    socket: std::net::UdpSocket,
    candidates: Vec<SocketAddr>,
    session: String,
) -> OnlineConnect {
    // game thread -> network thread (outbound)
    let (game_tx, net_rx) = tokio::sync::mpsc::unbounded_channel::<Packet>();
    // network thread -> game thread (inbound)
    let (net_tx, game_rx) = mpsc::channel::<Packet>();
    let (outcome_tx, outcome_rx) = mpsc::channel::<Result<SocketAddr, String>>();
    let binding = Arc::new(OnceLock::new());
    let binding_net = binding.clone();

    let handle = std::thread::Builder::new()
        .name("quic-online-connect".into())
        .spawn(move || {
            // Punch BEFORE quinn owns the socket: these datagrams teach the
            // routers that this conversation is wanted, and quinn would drop
            // the replies as not-QUIC anyway.
            crate::nat::punch::send_punches(&socket, &candidates, &session);

            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = outcome_tx.send(Err(format!("no runtime: {e}")));
                    return;
                }
            };
            rt.block_on(async move {
                let endpoint = match create_client_endpoint_on_socket(socket) {
                    Ok(ep) => ep,
                    Err(e) => {
                        let _ = outcome_tx.send(Err(format!("endpoint: {e}")));
                        return;
                    }
                };

                let mut race = crate::nat::punch::ConnectRace::new(candidates.len());
                let started = tokio::time::Instant::now();
                let mut attempts = tokio::task::JoinSet::new();
                // task id -> candidate index. A task that panics or is
                // cancelled yields a `JoinError`, which carries the id but not
                // the payload — without this map that candidate would stay
                // "in flight" for ever and `AllFailed` could never fire.
                let mut task_idx: std::collections::HashMap<tokio::task::Id, usize> =
                    std::collections::HashMap::new();
                let mut winner: Option<(SocketAddr, quinn::Connection)> = None;

                while race.outcome().is_none() {
                    for idx in race.advance(started.elapsed()) {
                        let addr = candidates[idx];
                        match endpoint.connect(addr, "axenstax-server") {
                            Ok(connecting) => {
                                let h = attempts.spawn(async move {
                                    (
                                        idx,
                                        addr,
                                        tokio::time::timeout(ATTEMPT_TIMEOUT, connecting).await,
                                    )
                                });
                                task_idx.insert(h.id(), idx);
                            }
                            Err(e) => {
                                log::debug!("[online] candidate {addr}: {e}");
                                race.on_failed(idx);
                            }
                        }
                    }
                    tokio::select! {
                        Some(joined) = attempts.join_next_with_id(), if !attempts.is_empty() => {
                            match joined {
                                Ok((id, (idx, addr, result))) => {
                                    task_idx.remove(&id);
                                    match result {
                                        Ok(Ok(conn)) => {
                                            // A connection that lands AFTER the
                                            // race is already decided (the
                                            // deadline fired on this same pass)
                                            // must not be adopted — close it
                                            // rather than leak it.
                                            if race.outcome().is_none() {
                                                // Aborting the losers is implicit:
                                                // every other attempt's future is
                                                // dropped by `shutdown()` below.
                                                let _aborted = race.on_connected(idx);
                                                winner = Some((addr, conn));
                                            } else {
                                                conn.close(0u32.into(), b"race already decided");
                                            }
                                        }
                                        Ok(Err(e)) => {
                                            log::debug!("[online] candidate {addr}: {e}");
                                            race.on_failed(idx);
                                        }
                                        Err(_elapsed) => {
                                            log::debug!(
                                                "[online] candidate {addr}: no answer in {}s",
                                                ATTEMPT_TIMEOUT.as_secs()
                                            );
                                            race.on_failed(idx);
                                        }
                                    }
                                }
                                Err(e) => {
                                    // Panicked or cancelled: recover the index
                                    // from the id so the race still counts it.
                                    if let Some(idx) = task_idx.remove(&e.id()) {
                                        log::debug!(
                                            "[online] candidate {}: attempt did not finish: {e}",
                                            candidates[idx]
                                        );
                                        race.on_failed(idx);
                                    }
                                }
                            }
                        }
                        // Tick the race even with nothing to join, so the
                        // stagger and the 8 s deadline both keep advancing.
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(20)) => {}
                    }
                }
                attempts.shutdown().await;

                match (race.outcome(), winner) {
                    (Some(crate::nat::punch::RaceOutcome::Won(_)), Some((addr, conn))) => {
                        log::info!("[online] connected to {addr}");
                        // Online-by-contact ends in the same quinn connection
                        // type, so it gets the same channel binding.
                        if let Some(b) = channel_binding_of(&conn) {
                            let _ = binding_net.set(b);
                        }
                        let (send, recv) = match open_game_stream(&conn).await {
                            Ok(s) => s,
                            Err(e) => {
                                let _ = outcome_tx.send(Err(e));
                                return;
                            }
                        };
                        let _ = outcome_tx.send(Ok(addr));
                        let closed = Arc::new(AtomicBool::new(false));
                        bridge_loop(conn, send, recv, net_tx, net_rx, closed, None, None).await;
                    }
                    (Some(crate::nat::punch::RaceOutcome::TimedOut), _) => {
                        let _ = outcome_tx.send(Err("timed out".to_string()));
                    }
                    _ => {
                        let _ = outcome_tx.send(Err("no candidate answered".to_string()));
                    }
                }
            });
        })
        .expect("spawn online connect thread");

    OnlineConnect {
        transport: QuicClientTransport {
            tx: game_tx,
            rx: game_rx,
            _network_thread: Some(handle),
            binding,
        },
        outcome: outcome_rx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Open one loopback QUIC connection and return (server-side, client-side)
    /// channel bindings.
    async fn loopback_bindings() -> (Option<[u8; 32]>, Option<[u8; 32]>) {
        let server = create_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = server.local_addr().unwrap();
        let client = create_client_endpoint().unwrap();
        let accept = tokio::spawn(async move {
            let conn = server.accept().await.unwrap().await.unwrap();
            let b = channel_binding_of(&conn);
            // Keep the endpoint alive until the binding is read.
            drop(conn);
            b
        });
        let conn = client.connect(addr, "axenstax-server").unwrap().await.unwrap();
        let client_b = channel_binding_of(&conn);
        let server_b = accept.await.unwrap();
        (server_b, client_b)
    }

    /// Poll `f` until it yields `Some` or ten seconds pass.
    fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Some(v) = f() {
                return Some(v);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn frames_carry_a_le_length_prefix_and_refuse_oversize() {
        let f = encode_frame(&[7, 8, 9]).unwrap();
        assert_eq!(f, vec![3, 0, 0, 0, 7, 8, 9]);
        assert!(encode_frame(&vec![0u8; MAX_FRAME_LEN]).is_some());
        assert!(encode_frame(&vec![0u8; MAX_FRAME_LEN + 1]).is_none());
    }

    /// Audit 2026-09-27: every game packet used to be one QUIC datagram, so
    /// anything over ~1200 bytes (a `JoinAccept` with exhibits, a busy
    /// `StateUpdate`) was dropped. Over a REAL loopback quinn connection, a
    /// packet far over that — queued by the server before the client has even
    /// opened its stream, like the join `Challenge` is — arrives intact, both
    /// ways, and a client going away is noticed by the server.
    #[test]
    fn a_large_packet_round_trips_over_a_real_quic_stream_and_close_is_seen() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let server = rt
            .block_on(async { create_server_endpoint("127.0.0.1:0".parse().unwrap()) })
            .unwrap();
        let addr = server.local_addr().unwrap();

        let client = connect_to_server(addr).unwrap();
        let conn = rt.block_on(async { server.accept().await.unwrap().await.unwrap() });
        let server_side = bridge_server_connection(conn);

        let big: Vec<u8> = (0..100 * 1024).map(|i| (i % 251) as u8).collect();
        server_side.send_to_client(&big);
        let got = wait_for(|| client.try_recv_from_server()).expect("client got the frame");
        assert_eq!(got, big, "a 100 KiB server packet arrives whole");

        let up: Vec<u8> = big.iter().rev().copied().collect();
        client.send_to_server(&up);
        let got = wait_for(|| server_side.try_recv_from_client()).expect("server got the frame");
        assert_eq!(got, up, "a 100 KiB client packet arrives whole");
        assert!(!server_side.is_closed());

        drop(client);
        let dc = wait_for(|| server_side.try_recv_from_client()).expect("synthetic Disconnect");
        let (ptype, _) = crate::protocol::deserialize_header(&dc).unwrap();
        assert_eq!(ptype, crate::protocol::PacketType::Disconnect);
        assert!(server_side.is_closed(), "the server transport reports the close");
        drop(server);
    }

    /// Review S2: a joiner that stops reading its stream still ACKs, so it is
    /// never idle-closed, and the host's queue for it used to grow without
    /// bound. Past `MAX_OUTBOUND_QUEUE_BYTES` the server now drops it.
    #[test]
    fn a_client_that_stops_reading_is_disconnected_at_the_queue_bound() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let server = rt
            .block_on(async { create_server_endpoint("127.0.0.1:0".parse().unwrap()) })
            .unwrap();
        let addr = server.local_addr().unwrap();
        let client = rt.block_on(async { create_client_endpoint() }).unwrap();
        let (conn, client_conn) = rt.block_on(async {
            let connecting = client.connect(addr, "axenstax-server").unwrap();
            let s = server.accept().await.unwrap().await.unwrap();
            (s, connecting.await.unwrap())
        });
        let server_side = bridge_server_connection(conn);
        // Open the stream (hello frame) and then never read it.
        let _streams = rt.block_on(open_game_stream(&client_conn)).unwrap();

        let mib = vec![0u8; 1024 * 1024];
        for _ in 0..(MAX_OUTBOUND_QUEUE_BYTES / mib.len() + 4) {
            server_side.send_to_client(&mib);
        }
        assert!(
            wait_for(|| server_side.is_closed().then_some(())).is_some(),
            "a client past the outbound bound is disconnected"
        );
        rt.block_on(async move {
            drop(server_side);
            drop(client_conn);
            drop(client);
            drop(server);
        });
    }

    #[test]
    fn both_ends_of_a_quic_connection_export_the_same_binding() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (s1, c1) = loopback_bindings().await;
            assert!(s1.is_some(), "server side must export a binding");
            assert_eq!(s1, c1, "client and server derive the same exporter");
            // A second connection (what a relaying host would hold) differs.
            let (s2, c2) = loopback_bindings().await;
            assert_eq!(s2, c2);
            assert_ne!(s1, s2, "each TLS session has its own exporter");
        });
    }
}

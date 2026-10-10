//! The recipient's latched listener: a bound QUIC endpoint that refuses every
//! incoming connection. It never advertises (no LAN broadcaster, no relay
//! subscription, no Server Card) and has no admission path at all; it exists
//! so the endpoint is ready before Ready. Activation is a separate, later
//! boundary that replaces it with an ordinary host.
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;

pub struct LatchedListener {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    refused: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl LatchedListener {
    pub fn bind(socket: UdpSocket) -> Result<Self, String> {
        let addr = socket.local_addr().map_err(|e| e.to_string())?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let refused = Arc::new(AtomicUsize::new(0));
        let (stop, count) = (shutdown.clone(), refused.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("handover-latched".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let ep = match crate::network::create_server_endpoint_on_socket(socket) {
                        Ok(ep) => ep,
                        Err(e) => {
                            let _ = tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    let _ = tx.send(Ok(()));
                    while !stop.load(Ordering::Relaxed) {
                        tokio::select! {
                            incoming = ep.accept() => match incoming {
                                Some(i) => {
                                    // Counted before the refusal is sent, so a
                                    // client that sees it also sees the count.
                                    count.fetch_add(1, Ordering::Relaxed);
                                    i.refuse();
                                }
                                None => break,
                            },
                            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
                        }
                    }
                    ep.close(0u32.into(), b"latched");
                });
            })
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())??;
        Ok(Self { addr, shutdown, refused, thread: Some(thread) })
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
    /// Connections refused so far.
    pub fn refused(&self) -> usize {
        self.refused.load(Ordering::Relaxed)
    }
}

impl Drop for LatchedListener {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

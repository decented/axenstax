//! The real relay sockets for Signet contacts sync: raw `REQ`/`EOSE` frames
//! over `tokio-tungstenite`, the shape `native_mailbox::relay` and
//! `nostr_release` already use against trotters (which rejects nostr-tools'
//! framing). Every call here runs on a worker thread that owns a
//! current-thread runtime — never on the frame thread.
//!
//! OWNER BOUNDARY: these need a live relay and are exercised by the owner's
//! phone test; every rule they feed is unit-tested in `ack_wait.rs` /
//! `sync.rs` against fakes.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::Event;
use serde_json::{json, Value};
use tokio::runtime::Runtime;
use tokio_tungstenite::tungstenite::Message;

use crate::native_mailbox::relay::{parse_frame, Frame};

use super::ack_wait::AckRelay;

/// How long to wait for a relay to answer a query (to `EOSE`).
const QUERY_CAP: Duration = Duration::from_secs(8);
/// How long a socket may take to open. A relay that accepts TCP and then
/// never completes the handshake must not hold the worker.
const CONNECT_CAP: Duration = Duration::from_secs(10);
/// Headroom for the `REQ`/`CLOSE` sends, which have no bound of their own.
const SEND_SLACK: Duration = Duration::from_secs(5);

async fn connect(
    url: &str,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, String> {
    match tokio::time::timeout(CONNECT_CAP, tokio_tungstenite::connect_async(url)).await {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(e)) => Err(format!("connect: {e}")),
        Err(_) => Err("connect: timed out".into()),
    }
}

/// A current-thread runtime for one worker thread.
pub fn worker_runtime() -> Option<Runtime> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().ok()
}

/// One-shot query: `REQ` → collect to `EOSE` (or [`QUERY_CAP`]). Signatures
/// are checked here, so nothing unsigned reaches a caller. The whole call is
/// bounded, so a hung relay can never hold the sync worker.
pub async fn fetch_events(url: &str, filter: &Value) -> Result<Vec<Event>, String> {
    tokio::time::timeout(CONNECT_CAP + QUERY_CAP + SEND_SLACK, fetch_events_inner(url, filter))
        .await
        .map_err(|_| "relay timed out".to_string())?
}

async fn fetch_events_inner(url: &str, filter: &Value) -> Result<Vec<Event>, String> {
    let ws = connect(url).await?;
    let (mut sink, mut stream) = ws.split();
    let sub = "q";
    sink.send(Message::Text(json!(["REQ", sub, filter]).to_string()))
        .await
        .map_err(|e| format!("send: {e}"))?;
    let mut out = Vec::new();
    let _ = tokio::time::timeout(QUERY_CAP, async {
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(t) = msg else { continue };
            match parse_frame(&t, sub) {
                Frame::Wrap(ev) if ev.verify().is_ok() => out.push(*ev),
                Frame::Eose => break,
                _ => {}
            }
        }
    })
    .await;
    let _ = sink.send(Message::Text(json!(["CLOSE", sub]).to_string())).await;
    Ok(out)
}

/// Both ack filters as two subscriptions on one socket, then a live listen.
/// Bounded as a whole, like [`fetch_events`].
async fn poll_ack(url: &str, eph: &Value, stored: &Value, listen: Duration) -> Result<(Vec<Event>, Vec<Event>), String> {
    tokio::time::timeout(CONNECT_CAP + QUERY_CAP + listen + SEND_SLACK, poll_ack_inner(url, eph, stored, listen))
        .await
        .map_err(|_| "relay timed out".to_string())?
}

async fn poll_ack_inner(
    url: &str,
    eph: &Value,
    stored: &Value,
    listen: Duration,
) -> Result<(Vec<Event>, Vec<Event>), String> {
    let ws = connect(url).await?;
    let (mut sink, mut stream) = ws.split();
    for (sub, f) in [("e", eph), ("s", stored)] {
        sink.send(Message::Text(json!(["REQ", sub, f]).to_string()))
            .await
            .map_err(|e| format!("send: {e}"))?;
    }
    let (mut live, mut page) = (Vec::new(), Vec::new());
    let (mut eose_e, mut eose_s) = (false, false);
    // Stored events to EOSE on both subscriptions.
    let _ = tokio::time::timeout(QUERY_CAP, async {
        while !(eose_e && eose_s) {
            let Some(Ok(msg)) = stream.next().await else { break };
            let Message::Text(t) = msg else { continue };
            match (parse_frame(&t, "e"), parse_frame(&t, "s")) {
                (Frame::Wrap(ev), _) => live.push(*ev),
                (_, Frame::Wrap(ev)) => page.push(*ev),
                (Frame::Eose, _) => eose_e = true,
                (_, Frame::Eose) => eose_s = true,
                _ => {}
            }
        }
    })
    .await;
    // Then hold both open: the ephemeral 21237 only ever reaches a LIVE
    // subscriber. Return as soon as something arrives.
    let _ = tokio::time::timeout(listen, async {
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(t) = msg else { continue };
            if let (Frame::Wrap(ev), _) | (_, Frame::Wrap(ev)) = (parse_frame(&t, "e"), parse_frame(&t, "s")) {
                live.push(*ev);
                break;
            }
        }
    })
    .await;
    for sub in ["e", "s"] {
        let _ = sink.send(Message::Text(json!(["CLOSE", sub]).to_string())).await;
    }
    live.retain(|e| e.verify().is_ok());
    page.retain(|e| e.verify().is_ok());
    Ok((live, page))
}

/// [`AckRelay`] over a real socket.
pub struct LiveAckRelay<'a> {
    pub rt: &'a Runtime,
    pub url: String,
}

impl AckRelay for LiveAckRelay<'_> {
    fn poll(&mut self, eph: &Value, stored: &Value, listen: Duration) -> (Vec<Event>, Vec<Event>) {
        let started = std::time::Instant::now();
        let result = match self.rt.block_on(poll_ack(&self.url, eph, stored, listen)) {
            Ok(r) => r,
            Err(e) => {
                log::debug!("[signet-contacts] ack poll failed: {e}");
                (Vec::new(), Vec::new())
            }
        };
        // A pass that found nothing never returns faster than a listen would
        // have taken, so a relay that drops the socket is not hammered.
        if result.0.is_empty() && result.1.is_empty() {
            std::thread::sleep(listen.saturating_sub(started.elapsed()));
        }
        result
    }

    fn fetch(&mut self, filter: &Value) -> Vec<Event> {
        self.rt.block_on(fetch_events(&self.url, filter)).unwrap_or_default()
    }
}

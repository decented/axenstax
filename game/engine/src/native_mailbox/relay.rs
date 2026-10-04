//! Relay frames + raw-WS client for the native mailbox. The frames are the
//! plain NIP-01 REQ/EVENT arrays (`["REQ", id, filter]`, `["EVENT", event]`)
//! that `tools/feedback-reader/live.mjs` also sends — the most conservative
//! shape, accepted by every relay we have tried (some relays rejected
//! nostr-tools' extra fields). The frame builders and
//! parser here are pure and unit-tested; `publish`/`fetch_events` are thin
//! socket wrappers exercised at the owner playtest (no live relay in CI).

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::prelude::*;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

/// A parsed relay → client frame (only the variants the mailbox cares about).
#[derive(Debug)]
pub enum Frame {
    /// `["EVENT", <sub_id>, <event-object>]` for the requested subscription.
    /// (Named `Wrap` for history; it carries whatever event the REQ matched.)
    Wrap(Box<Event>),
    /// `["EOSE", <sub_id>]` for the requested subscription — end of stored
    /// events, nothing left to collect.
    Eose,
    /// `["OK", <event-id>, <accepted>, <message>]` — publish acknowledgement.
    /// The message text isn't surfaced; callers only need accept/reject.
    Ok { id: String, accepted: bool },
    /// Anything else: a different subscription's frame, NOTICE, or garbage.
    Other,
}

/// Build a REQ frame for `filter`. The filter's field order is the caller's
/// (build it with `json!` in the known-good order: `kinds`, `authors`, `#d`…).
pub fn req_frame(sub_id: &str, filter: &serde_json::Value) -> String {
    json!(["REQ", sub_id, filter]).to_string()
}

/// Wrap a signed event in the relay publish frame `["EVENT", <event>]`.
pub fn event_frame(ev: &Event) -> String {
    json!(["EVENT", ev]).to_string()
}

/// Parse a relay → client frame. EVENT/EOSE are only matched when their
/// sub_id equals `sub_id` (anything else, e.g. another subscription, is
/// [`Frame::Other`]); OK carries an event id instead of a sub_id, so it always
/// parses regardless of `sub_id`. Total: anything unexpected → [`Frame::Other`].
pub fn parse_frame(text: &str, sub_id: &str) -> Frame {
    let Ok(serde_json::Value::Array(arr)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return Frame::Other;
    };
    let s = |i: usize| arr.get(i).and_then(|x| x.as_str());
    match arr.first().and_then(|x| x.as_str()) {
        Some("EVENT") if s(1) == Some(sub_id) => match arr.get(2) {
            Some(ev_val) => match serde_json::from_value::<Event>(ev_val.clone()) {
                Ok(ev) => Frame::Wrap(Box::new(ev)),
                Err(_) => Frame::Other,
            },
            None => Frame::Other,
        },
        Some("EOSE") if s(1) == Some(sub_id) => Frame::Eose,
        Some("OK") => Frame::Ok {
            id: s(1).unwrap_or_default().to_string(),
            accepted: arr.get(2).and_then(|x| x.as_bool()).unwrap_or(false),
        },
        _ => Frame::Other,
    }
}

// ─────────────────────────── live websocket round-trips ─────────────────────
// Owner boundary: these need a real relay and are verified by the owner
// playtest, not unit tests. The envelope they build/parse is the tested part
// above.

/// OWNER BOUNDARY (live relay). Publish `ev` to `url` and wait up to 5s for the
/// relay's matching `OK`. A rejection or a timeout both surface as `Err`; many
/// relays are slow or silent, which is why the mailbox fans out to several.
pub async fn publish(url: &str, ev: &Event) -> Result<(), String> {
    let (ws, _) =
        tokio_tungstenite::connect_async(url).await.map_err(|e| format!("connect {url}: {e}"))?;
    let (mut sink, mut stream) = ws.split();
    sink.send(Message::Text(event_frame(ev).into())).await.map_err(|e| format!("send: {e}"))?;

    let id_hex = ev.id.to_hex();
    let wait_for_ok = async {
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(t) = msg else { continue };
            // OK frames aren't tied to a subscription id, so any sub_id works.
            if let Frame::Ok { id, accepted } = parse_frame(&t, "")
                && id == id_hex
            {
                return Some(accepted);
            }
        }
        None
    };
    match tokio::time::timeout(Duration::from_secs(5), wait_for_ok).await {
        Ok(Some(true)) => Ok(()),
        Ok(Some(false)) => Err(format!("relay rejected event {id_hex}")),
        Ok(None) => Err(format!("relay closed before OK for {id_hex}")),
        Err(_) => Err(format!("timed out waiting for OK on {id_hex}")),
    }
}

/// OWNER BOUNDARY (live relay). One-shot query for `filter`: collect matching
/// events until EOSE or a 10s cap (whichever comes first), then CLOSE. The
/// whole call — connect included — is bounded, so a relay that accepts the
/// socket and then goes silent can never hold the mailbox worker. Signatures
/// are checked here; nothing unsigned reaches a caller. Non-matching frames
/// (NOTICE, another subscription, garbage) are ignored, not errors.
/// Most events one fetch keeps (the board is a single addressable event).
const MAX_FETCHED_EVENTS: usize = 20;

pub async fn fetch_events(url: &str, filter: &serde_json::Value) -> Result<Vec<Event>, String> {
    tokio::time::timeout(Duration::from_secs(25), fetch_events_inner(url, filter))
        .await
        .map_err(|_| format!("relay {url} timed out"))?
}

async fn fetch_events_inner(url: &str, filter: &serde_json::Value) -> Result<Vec<Event>, String> {
    let (ws, _) =
        tokio_tungstenite::connect_async(url).await.map_err(|e| format!("connect {url}: {e}"))?;
    let (mut sink, mut stream) = ws.split();
    let sub_id = "mb";
    sink.send(Message::Text(req_frame(sub_id, filter).into()))
        .await
        .map_err(|e| format!("send: {e}"))?;

    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(t) = msg else { continue };
            match parse_frame(&t, sub_id) {
                Frame::Wrap(ev) if ev.verify().is_ok() => {
                    out.push(*ev);
                    // A hostile relay must not be able to flood us.
                    if out.len() >= MAX_FETCHED_EVENTS {
                        break;
                    }
                }
                Frame::Eose => break,
                _ => {}
            }
        }
    })
    .await;
    let _ = sink.send(Message::Text((json!(["CLOSE", sub_id]).to_string()).into())).await;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    // `nostr::prelude::*` reaches this module already via the parent's own
    // `use` (brought in by `use super::*` above) — no need to reimport it.

    #[test]
    fn req_frame_is_the_plain_nip01_shape() {
        // live.mjs's raw REQ is the known-good shape:
        // ["REQ", id, {kinds:[..], ...filter}]
        let filter = crate::native_mailbox::board::board_filter(&"ab".repeat(32));
        let f = req_frame("mb", &filter);
        let v: serde_json::Value = serde_json::from_str(&f).unwrap();
        assert_eq!(v[0], "REQ");
        assert_eq!(v[1], "mb");
        assert_eq!(v[2]["kinds"][0], 30078);
        assert_eq!(v[2]["authors"][0], "ab".repeat(32));
    }

    #[test]
    fn event_frame_and_parse_round_trip() {
        let keys = Keys::generate();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let ev = rt.block_on(async {
            EventBuilder::text_note("x").sign(&keys).await.unwrap()
        });
        let f = event_frame(&ev);
        let v: serde_json::Value = serde_json::from_str(&f).unwrap();
        assert_eq!(v[0], "EVENT");
        assert_eq!(v[1]["id"], ev.id.to_hex());

        let wrap_msg = format!("[\"EVENT\",\"mb\",{}]", serde_json::to_string(&ev).unwrap());
        match parse_frame(&wrap_msg, "mb") {
            Frame::Wrap(got) => assert_eq!(got.id, ev.id),
            other => panic!("expected Wrap, got {other:?}"),
        }
        assert!(matches!(parse_frame("[\"EOSE\",\"mb\"]", "mb"), Frame::Eose));
        assert!(matches!(parse_frame("[\"EOSE\",\"other\"]", "mb"), Frame::Other));
        match parse_frame(&format!("[\"OK\",\"{}\",true,\"\"]", ev.id.to_hex()), "mb") {
            Frame::Ok { id, accepted } => {
                assert_eq!(id, ev.id.to_hex());
                assert!(accepted);
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    /// OWNER BOUNDARY (live relay) — needs network access, not run in CI.
    /// Exercises the actual TLS handshake + REQ/EOSE round-trip against
    /// the first feedback inbox relay: `connect_async("wss://...")` only succeeds when
    /// `tokio-tungstenite` has a TLS backend compiled in (see the
    /// `rustls-tls-webpki-roots` feature on the Cargo.toml dependency). A
    /// throwaway author has published no board, so an empty `Vec` is the
    /// expected (successful) payload — the assertion is on `is_ok()`, i.e.
    /// "did the socket connect, upgrade to TLS, and complete a REQ → EOSE
    /// round-trip", not on the (empty) contents.
    ///
    /// Run manually: `cargo test --bin axenstax-engine native_mailbox::relay
    /// -- --ignored`.
    #[test]
    #[ignore]
    fn live_relay_tls_round_trip() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let filter = crate::native_mailbox::board::board_filter(&"ab".repeat(32));
        let result = rt.block_on(fetch_events(crate::native_mailbox::FEEDBACK_INBOX_RELAYS[0], &filter));
        assert!(result.is_ok(), "fetch_events against live relay failed: {result:?}");
    }
}

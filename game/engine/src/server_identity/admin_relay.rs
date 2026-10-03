#![cfg(not(target_arch = "wasm32"))]
//! Relay delivery of operator admin commands (C2 — the "last hop").
//!
//! Track 5 built the verify + apply core ([`super::admin`]) and the local
//! `--admin` apply path. This module carries a signed command from the operator
//! to a running server over a Nostr relay, so the operator can change live policy
//! from anywhere with no shell access to the box:
//!
//! - **Server** (`--server`): [`subscribe_admin_commands`] opens a relay
//!   subscription for kind-27422 events authored by its operator, and hands each
//!   one to a callback that verifies + applies it. Runs on its own thread for the
//!   server's lifetime, reconnecting on drop. It only ever writes the on-disk
//!   policy files, which the main loop's existing ~5s reload picks up — so there
//!   is no shared state between the relay thread and the game loop.
//! - **Operator** (`--admin-publish` / `--admin-sign`): [`publish_event`] posts a
//!   signed command event to the relay.
//!
//! The relay-protocol envelope (REQ / EVENT / OK / EOSE / NOTICE) is built and
//! parsed by the pure functions below (unit-tested); the websocket round-trips are
//! the owner boundary.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use crate::server_identity::admin::ADMIN_CMD_KIND;

/// A parsed relay → client frame (only the variants we care about).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayFrame {
    /// `["EVENT", <sub_id>, <event-object>]` — `event_json` is the event object
    /// re-serialised, ready for `nostr::Event::from_json`.
    Event { sub_id: String, event_json: String },
    /// `["EOSE", <sub_id>]` — end of stored events; the live tail follows.
    Eose(String),
    /// `["OK", <event-id>, <accepted>, <message>]` — publish acknowledgement.
    Ok { id: String, accepted: bool, message: String },
    /// `["NOTICE", <message>]` — human-readable relay notice.
    Notice(String),
    /// Anything else / unparseable.
    Unknown,
}

/// Build the REQ frame subscribing to this server's operator admin commands:
/// kind 27422 authored by `author_hex`, optionally only events `since` a unix ts
/// (so a long-lived server doesn't re-apply the whole history on reconnect).
pub fn subscribe_req(sub_id: &str, author_hex: &str, since_secs: Option<u64>) -> String {
    let mut filter = serde_json::Map::new();
    filter.insert("kinds".into(), json!([ADMIN_CMD_KIND]));
    filter.insert("authors".into(), json!([author_hex]));
    if let Some(s) = since_secs {
        filter.insert("since".into(), json!(s));
    }
    json!(["REQ", sub_id, filter]).to_string()
}

/// Build a general REQ frame: stored events of any of `kinds` authored by
/// `author_hex`. Used by npub→address resolution (Spec A) to fetch the operator's
/// attestation and the delegate's card. (`subscribe_req` is the admin-command
/// variant pinned to one kind + a `since` cursor.)
pub fn query_req(sub_id: &str, kinds: &[u16], author_hex: &str) -> String {
    let mut filter = serde_json::Map::new();
    filter.insert("kinds".into(), json!(kinds));
    filter.insert("authors".into(), json!([author_hex]));
    json!(["REQ", sub_id, filter]).to_string()
}

/// Wrap a signed event (its JSON) in the relay publish frame `["EVENT", <event>]`.
pub fn publish_frame(event_json: &str) -> Result<String, String> {
    let ev: serde_json::Value = serde_json::from_str(event_json).map_err(|e| e.to_string())?;
    Ok(json!(["EVENT", ev]).to_string())
}

/// Parse a relay → client frame. Total: anything unexpected → [`RelayFrame::Unknown`].
pub fn parse_relay_frame(msg: &str) -> RelayFrame {
    let Ok(serde_json::Value::Array(arr)) = serde_json::from_str::<serde_json::Value>(msg) else {
        return RelayFrame::Unknown;
    };
    let s = |i: usize| arr.get(i).and_then(|x| x.as_str()).unwrap_or("").to_string();
    match arr.first().and_then(|x| x.as_str()) {
        Some("EVENT") => match arr.get(2) {
            Some(ev) => RelayFrame::Event { sub_id: s(1), event_json: ev.to_string() },
            None => RelayFrame::Unknown,
        },
        Some("EOSE") => RelayFrame::Eose(s(1)),
        Some("NOTICE") => RelayFrame::Notice(s(1)),
        Some("OK") => RelayFrame::Ok {
            id: s(1),
            accepted: arr.get(2).and_then(|x| x.as_bool()).unwrap_or(false),
            message: s(3),
        },
        _ => RelayFrame::Unknown,
    }
}

// ─────────────────────────── live websocket round-trips ─────────────────────
// Owner boundary: these need a real relay. The envelope they build/parse is the
// unit-tested part above.

/// Open a subscription for `author_hex`'s admin commands and hand each event's
/// JSON to `on_event`. Loops forever, reconnecting with a short backoff on any
/// drop/error — intended to run on a dedicated thread for the server's lifetime.
pub async fn subscribe_admin_commands<F>(relay_url: &str, author_hex: &str, mut on_event: F)
where
    F: FnMut(&str),
{
    let sub_id = "axe-admin";
    loop {
        // Only ever apply commands newer than ~5 min ago on (re)connect — the
        // verify step also enforces freshness, this just trims the stored replay.
        let since = nostr::Timestamp::now().as_secs().saturating_sub(300);
        match tokio_tungstenite::connect_async(relay_url).await {
            Ok((ws, _)) => {
                let (mut sink, mut stream) = ws.split();
                let req = subscribe_req(sub_id, author_hex, Some(since));
                if sink.send(Message::Text(req)).await.is_err() {
                    log::warn!("admin relay: failed to send REQ to {relay_url}; retrying");
                } else {
                    log::info!("admin relay: subscribed on {relay_url}");
                    while let Some(msg) = stream.next().await {
                        match msg {
                            Ok(Message::Text(t)) => {
                                if let RelayFrame::Event { event_json, .. } = parse_relay_frame(&t) {
                                    on_event(&event_json);
                                }
                            }
                            Ok(Message::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                }
            }
            Err(e) => log::warn!("admin relay: connect to {relay_url} failed: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Publish a signed command event to the relay and wait briefly for the relay's
/// `OK`. Returns `Ok(())` once accepted (or the wait times out — relays vary in
/// whether they `OK`; the post still landed).
pub async fn publish_event(relay_url: &str, event_json: &str) -> Result<(), String> {
    let frame = publish_frame(event_json)?;
    let (ws, _) = tokio_tungstenite::connect_async(relay_url)
        .await
        .map_err(|e| format!("connect {relay_url}: {e}"))?;
    let (mut sink, mut stream) = ws.split();
    sink.send(Message::Text(frame))
        .await
        .map_err(|e| format!("send: {e}"))?;
    // Best-effort: read up to a few frames or ~5s for an OK acknowledgement.
    let deadline = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(Message::Text(t))) = stream.next().await {
            if let RelayFrame::Ok { accepted, message, .. } = parse_relay_frame(&t) {
                return Some((accepted, message));
            }
        }
        None
    })
    .await;
    match deadline {
        Ok(Some((true, _))) => Ok(()),
        Ok(Some((false, msg))) => Err(format!("relay rejected the command: {msg}")),
        // No OK (timeout or stream end) — treat as sent; many relays don't OK.
        Ok(None) | Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn req_carries_kind_author_and_since() {
        let req = subscribe_req("sub1", "abcdef00", Some(1700));
        // Round-trips as the canonical relay REQ frame.
        let v: serde_json::Value = serde_json::from_str(&req).unwrap();
        assert_eq!(v[0], "REQ");
        assert_eq!(v[1], "sub1");
        assert_eq!(v[2]["kinds"][0], ADMIN_CMD_KIND);
        assert_eq!(v[2]["authors"][0], "abcdef00");
        assert_eq!(v[2]["since"], 1700);
    }

    #[test]
    fn req_omits_since_when_absent() {
        let req = subscribe_req("s", "aa", None);
        let v: serde_json::Value = serde_json::from_str(&req).unwrap();
        assert!(v[2].get("since").is_none(), "no since key when not requested");
    }

    #[test]
    fn query_req_carries_multiple_kinds_and_author() {
        let req = query_req("q1", &[30420, 30422], "abcd");
        let v: serde_json::Value = serde_json::from_str(&req).unwrap();
        assert_eq!(v[0], "REQ");
        assert_eq!(v[1], "q1");
        assert_eq!(v[2]["kinds"][0], 30420);
        assert_eq!(v[2]["kinds"][1], 30422);
        assert_eq!(v[2]["authors"][0], "abcd");
        assert!(v[2].get("since").is_none(), "query_req has no since cursor");
    }

    #[test]
    fn parse_event_frame_extracts_the_event_object() {
        let frame = r#"["EVENT","axe-admin",{"id":"deadbeef","kind":27422,"content":""}]"#;
        match parse_relay_frame(frame) {
            RelayFrame::Event { sub_id, event_json } => {
                assert_eq!(sub_id, "axe-admin");
                // The extracted object is valid JSON ready for Event::from_json.
                let ev: serde_json::Value = serde_json::from_str(&event_json).unwrap();
                assert_eq!(ev["kind"], 27422);
                assert_eq!(ev["id"], "deadbeef");
            }
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn parse_eose_ok_notice_and_garbage() {
        assert_eq!(parse_relay_frame(r#"["EOSE","s"]"#), RelayFrame::Eose("s".into()));
        assert_eq!(parse_relay_frame(r#"["NOTICE","hi"]"#), RelayFrame::Notice("hi".into()));
        assert_eq!(
            parse_relay_frame(r#"["OK","id123",true,""]"#),
            RelayFrame::Ok { id: "id123".into(), accepted: true, message: String::new() }
        );
        assert_eq!(
            parse_relay_frame(r#"["OK","id",false,"blocked"]"#),
            RelayFrame::Ok { id: "id".into(), accepted: false, message: "blocked".into() }
        );
        assert_eq!(parse_relay_frame("not json at all"), RelayFrame::Unknown);
        assert_eq!(parse_relay_frame(r#"{"obj":1}"#), RelayFrame::Unknown);
        assert_eq!(parse_relay_frame(r#"["WAT"]"#), RelayFrame::Unknown);
    }

    #[test]
    fn publish_frame_wraps_a_signed_event() {
        let ev = r#"{"id":"x","kind":27422,"sig":"yy"}"#;
        let frame = publish_frame(ev).unwrap();
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v[0], "EVENT");
        assert_eq!(v[1]["kind"], 27422);
        assert_eq!(v[1]["id"], "x");
    }

    #[test]
    fn publish_frame_rejects_non_json() {
        assert!(publish_frame("not json").is_err());
    }
}

//! Plain NIP-01 relay frames shared by the relay sockets that need them
//! (`rendezvous::relay_client`, `signet_contacts::relay`). The builders and the
//! parser are pure and unit-tested; the sockets that use them are exercised at
//! the owner playtest (no live relay in CI).
//!
//! The frames are the raw `["EVENT", event]` / `["REQ", id, filter]` arrays,
//! the most conservative shape, accepted by every relay we have tried (some
//! relays rejected nostr-tools' extra fields).
#![cfg(not(target_arch = "wasm32"))]

use nostr::prelude::*;
use serde_json::json;

/// A parsed relay → client frame (only the variants the callers care about).
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

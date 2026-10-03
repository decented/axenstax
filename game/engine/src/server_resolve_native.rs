#![cfg(not(target_arch = "wasm32"))]
#![allow(dead_code)] // consumed by the join path (Spec A task 7)
//! Native npub→address resolution: fetch the operator's attestation + the
//! delegate's card from relays and feed the cross-platform selection core
//! ([`crate::server_resolve`]).
//!
//! **Owner boundary:** the websocket round-trips ([`query_events`],
//! [`resolve_by_npub`]) need a live relay and are verified by the owner. The
//! pure pieces are tested elsewhere — the REQ builder in
//! [`crate::server_identity::admin_relay`] and the selection logic in
//! [`crate::server_resolve`]. Per design §4, the connect-time join proof is the
//! trust anchor; this resolver only *finds an address to dial*.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use crate::server_identity::admin_relay::{parse_relay_frame, query_req, RelayFrame};
use crate::server_identity::attestation::ATTESTATION_KIND;
use crate::server_identity::server_card::CARD_KIND;
use crate::server_resolve::{
    delegate_runtime_hex, parse_raw_events, resolve_from_raw, RawEvent, ResolvedServer,
};

/// The default relay set (operator-overridable via `--card-relays`).
pub fn default_relays() -> Vec<String> {
    crate::server_resolve::public_default_relays()
}

/// OWNER BOUNDARY (live relay). Open a one-shot subscription for stored events of
/// `kinds` authored by `author_hex`, collecting each event's JSON until EOSE or a
/// ~5s timeout. Returns whatever arrived (empty on connect failure).
pub async fn query_events(relay_url: &str, kinds: &[u16], author_hex: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok((ws, _)) = tokio_tungstenite::connect_async(relay_url).await else {
        return out;
    };
    let (mut sink, mut stream) = ws.split();
    let req = query_req("axe-resolve", kinds, author_hex);
    if sink.send(Message::Text(req)).await.is_err() {
        return out;
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(Message::Text(t))) = stream.next().await {
            match parse_relay_frame(&t) {
                RelayFrame::Event { event_json, .. } => out.push(event_json),
                RelayFrame::Eose(_) => break,
                _ => {}
            }
        }
    })
    .await;
    out
}

/// OWNER BOUNDARY (live relay). Resolve an operator npub to a live server by
/// querying `relays` in order: fetch the attestation, follow it to the delegate
/// runtime key, fetch the card, and select via the cross-platform core. The
/// join proof on connect is the real trust check (design §4).
pub async fn resolve_by_npub(
    operator_npub: &str,
    relays: &[String],
) -> Result<ResolvedServer, String> {
    let op_hex = nostr::PublicKey::parse(operator_npub)
        .map_err(|e| format!("bad operator npub: {e}"))?
        .to_hex();
    for relay in relays {
        let atts: Vec<RawEvent> =
            parse_raw_events(&query_events(relay, &[ATTESTATION_KIND], &op_hex).await);
        let Some(runtime_hex) = delegate_runtime_hex(&atts, ATTESTATION_KIND, &op_hex) else {
            continue; // no anchor on this relay — try the next
        };
        let cards: Vec<RawEvent> =
            parse_raw_events(&query_events(relay, &[CARD_KIND], &runtime_hex).await);
        if let Ok(resolved) =
            resolve_from_raw(&op_hex, operator_npub, ATTESTATION_KIND, CARD_KIND, &atts, &cards)
        {
            return Ok(resolved);
        }
    }
    Err(format!(
        "could not resolve {operator_npub} on any relay"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_discovery_relays_are_public_only() {
        let d = default_relays();
        assert!(!d.is_empty());
        assert!(d.iter().all(|r| !r.contains("trotters")), "red line 2: no AxeNStax relay in discovery defaults");
    }

    #[tokio::test]
    async fn bad_npub_is_rejected_before_any_network() {
        // PublicKey::parse fails before the relay loop — no network needed.
        assert!(resolve_by_npub("not-an-npub", &[]).await.is_err());
    }
}

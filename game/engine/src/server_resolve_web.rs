#![cfg(target_arch = "wasm32")]
#![allow(dead_code)] // consumed by the web join path (Spec A task 7)
//! Web npub→address resolution. Reuses the cross-platform selection core
//! ([`crate::server_resolve`]). The `nostr` crate is native-only, so this path
//! borrows two existing page utilities instead:
//!   * `window.axenstax_npub_to_hex` (npub-decode.js) for npub→hex, and
//!   * `window.axenstax_relay_query` (relay-query.js) for the relay read.
//!
//! **Owner boundary:** the live relay round-trip is verified in a real browser
//! against a real relay. Per design §4 the connect-time join proof is the trust
//! anchor; this resolver only finds an address to dial.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

use crate::server_resolve::{
    delegate_runtime_hex, parse_raw_events, resolve_from_raw, ResolvedServer, ATTESTATION_KIND,
    CARD_KIND,
};

/// The default relay set (operator-overridable in a later pass).
pub fn default_relays() -> Vec<String> {
    crate::server_resolve::public_default_relays()
}

#[wasm_bindgen]
extern "C" {
    /// npub-decode.js — npub bech32 → 64-hex, or a non-string (null) value if
    /// malformed. Synchronous, never throws.
    #[wasm_bindgen(js_name = axenstax_npub_to_hex)]
    fn js_npub_to_hex(npub: String) -> JsValue;

    /// relay-query.js — open a WebSocket to `relay_url`, send the REQ
    /// `filter_json`, collect EVENT event-objects (as JSON strings) until EOSE or
    /// a ~5s timeout, resolve with a JS array of strings. OWNER BOUNDARY.
    #[wasm_bindgen(js_name = axenstax_relay_query, catch)]
    async fn js_relay_query(relay_url: String, filter_json: String)
        -> Result<JsValue, JsValue>;
}

fn filter_json(kinds: &[u16], author_hex: &str) -> String {
    serde_json::json!({ "kinds": kinds, "authors": [author_hex] }).to_string()
}

/// Fetch event-JSON strings for `kinds` authored by `author_hex` via the JS
/// relay bridge. Empty on any failure (the JS resolves with `[]`, never throws
/// in practice; `catch` covers the missing-bridge case).
async fn query_events_web(relay_url: &str, kinds: &[u16], author_hex: &str) -> Vec<String> {
    match js_relay_query(relay_url.to_string(), filter_json(kinds, author_hex)).await {
        Ok(val) => js_sys::Array::from(&val)
            .iter()
            .filter_map(|v| v.as_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Resolve an operator npub to a live server via the JS relay bridge, reusing the
/// cross-platform selection core.
pub async fn resolve_by_npub_web(
    operator_npub: &str,
    relays: &[String],
) -> Result<ResolvedServer, String> {
    let op_hex = js_npub_to_hex(operator_npub.to_string())
        .as_string()
        .ok_or_else(|| format!("bad operator npub: {operator_npub}"))?;
    for relay in relays {
        let atts =
            parse_raw_events(&query_events_web(relay, &[ATTESTATION_KIND], &op_hex).await);
        let Some(runtime_hex) = delegate_runtime_hex(&atts, ATTESTATION_KIND, &op_hex) else {
            continue;
        };
        let cards =
            parse_raw_events(&query_events_web(relay, &[CARD_KIND], &runtime_hex).await);
        if let Ok(resolved) =
            resolve_from_raw(&op_hex, operator_npub, ATTESTATION_KIND, CARD_KIND, &atts, &cards)
        {
            return Ok(resolved);
        }
    }
    Err(format!("could not resolve {operator_npub} on any relay"))
}

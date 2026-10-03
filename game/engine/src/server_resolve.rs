// Consumed by the native + web relay fetch wiring (Spec A tasks 5/6) and the
// join path (task 7).
#![allow(dead_code)]
//! npub→address resolution — the cross-platform selection core (Spec A task 4).
//!
//! Operates on lightweight [`RawEvent`]s parsed with `serde_json`, which is
//! available on **both** native and wasm. The `nostr` crate is native-only (see
//! Cargo.toml), so this core deliberately does NOT depend on it and does NOT
//! verify signatures. Per design §4 the **join proof** is the trust anchor: a
//! forged or substituted relay record can at worst make a join *fail*, never
//! impersonate the operator (the impostor can't produce a proof chaining to the
//! expected npub). Native callers may additionally verify the attestation +
//! card (defence in depth) before trusting — see the native fetch wiring.
//!
//! The core picks the newest attestation authored by the operator, follows its
//! `d` tag to the delegate runtime key, picks the newest card authored by that
//! key, cross-checks the card's `op` tag, and extracts the endpoint + descriptor.

use serde::Deserialize;

/// Nostr kinds, re-declared here as the cross-platform home (the canonical native
/// `attestation::ATTESTATION_KIND` / `server_card::CARD_KIND` live in the
/// native-only `server_identity` module, so the wasm resolver can't import them).
pub const ATTESTATION_KIND: u16 = 30420;
pub const CARD_KIND: u16 = 30422;

/// Default public relays — the shipped "Your relays" list
/// (`GraphicsSettings.online_relays`) that the native app uses for sign-in,
/// online-play setup (rendezvous), Server Card discovery, Signet contacts
/// pairing and the release feed, plus the operator's default pairing relay.
/// Third-party relays only: AxeNStax operates no relay that is a default
/// anywhere (CLAUDE.md red line 2). `relay.trotters.cc` is ours; a player may
/// add it, but it is never shipped as a default. Feedback has its own fixed
/// inbox set (`native_mailbox::FEEDBACK_INBOX_RELAYS`).
pub const PUBLIC_DEFAULT_RELAYS: [&str; 3] = [
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
];

/// Every Server Card / discovery default, as owned strings.
pub fn public_default_relays() -> Vec<String> {
    PUBLIC_DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
}

/// A minimally-parsed Nostr event (`pubkey`/`created_at`/`kind`/`tags`/`content`).
/// Other event fields (`id`, `sig`) are ignored. Parseable from event JSON on
/// any platform.
#[derive(Clone, Debug, Deserialize)]
pub struct RawEvent {
    /// Author public key, hex.
    pub pubkey: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub kind: u16,
    #[serde(default)]
    pub tags: Vec<Vec<String>>,
    #[serde(default)]
    pub content: String,
}

/// The resolved live server, ready to dial.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ResolvedServer {
    /// Dialable endpoints, in preference order.
    pub endpoints: Vec<String>,
    pub name: String,
    pub about: String,
    pub region: String,
    pub players_cur: u16,
    pub players_max: u16,
    pub protocol: u32,
    pub privacy: String,
    /// The operator npub this resolved to (carried through for the join pin).
    pub operator_npub: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// No attestation (anchor) found for the operator.
    NoAnchor,
    /// No card found for the delegate runtime key.
    NoCard,
    /// The card's `op` tag doesn't name the expected operator.
    OperatorMismatch,
    /// The card carries no dialable endpoint.
    NoEndpoint,
}

/// Parse a batch of event-JSON strings into [`RawEvent`]s, dropping unparseable.
pub fn parse_raw_events(events_json: &[String]) -> Vec<RawEvent> {
    events_json
        .iter()
        .filter_map(|j| serde_json::from_str::<RawEvent>(j).ok())
        .collect()
}

fn newest_by_author<'a>(
    events: &'a [RawEvent],
    kind: u16,
    author_hex: &str,
) -> Option<&'a RawEvent> {
    events
        .iter()
        .filter(|e| e.kind == kind && e.pubkey == author_hex)
        .max_by_key(|e| e.created_at)
}

fn tag1<'a>(e: &'a RawEvent, name: &str) -> Option<&'a str> {
    e.tags
        .iter()
        .find(|t| t.len() >= 2 && t[0] == name)
        .map(|t| t[1].as_str())
}

/// Resolve from already-fetched events. `operator_hex` is the operator pubkey in
/// hex (relay queries filter by it); `operator_npub` is carried into the result.
/// `attestation_kind`/`card_kind` are passed in to avoid coupling this
/// cross-platform module to the native-only kind constants.
#[allow(clippy::too_many_arguments)] // each arg is distinct + meaningful (op id/npub, two kinds, two event sets)
pub fn resolve_from_raw(
    operator_hex: &str,
    operator_npub: &str,
    attestation_kind: u16,
    card_kind: u16,
    attestations: &[RawEvent],
    cards: &[RawEvent],
) -> Result<ResolvedServer, ResolveError> {
    let anchor =
        newest_by_author(attestations, attestation_kind, operator_hex).ok_or(ResolveError::NoAnchor)?;
    // The attestation's `d` tag is the delegate runtime pubkey (hex).
    let runtime_hex = tag1(anchor, "d").ok_or(ResolveError::NoAnchor)?;
    let card = newest_by_author(cards, card_kind, runtime_hex).ok_or(ResolveError::NoCard)?;
    if tag1(card, "op") != Some(operator_hex) {
        return Err(ResolveError::OperatorMismatch);
    }
    let endpoints: Vec<String> = card
        .tags
        .iter()
        .filter(|t| t.len() >= 2 && t[0] == "endpoint")
        .map(|t| t[1].clone())
        .collect();
    if endpoints.is_empty() {
        return Err(ResolveError::NoEndpoint);
    }
    let (players_cur, players_max) = card
        .tags
        .iter()
        .find(|t| t.len() >= 3 && t[0] == "players")
        .map(|t| (t[1].parse().unwrap_or(0), t[2].parse().unwrap_or(0)))
        .unwrap_or((0, 0));
    Ok(ResolvedServer {
        endpoints,
        name: tag1(card, "name").unwrap_or_default().to_string(),
        about: tag1(card, "about").unwrap_or_default().to_string(),
        region: tag1(card, "region").unwrap_or_default().to_string(),
        players_cur,
        players_max,
        protocol: tag1(card, "protocol")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        privacy: tag1(card, "privacy").unwrap_or_default().to_string(),
        operator_npub: operator_npub.to_string(),
    })
}

/// The delegate runtime pubkey (hex) named by the newest attestation authored by
/// `operator_hex` — i.e. which key's card to fetch next. `None` if no anchor.
pub fn delegate_runtime_hex(
    attestations: &[RawEvent],
    attestation_kind: u16,
    operator_hex: &str,
) -> Option<String> {
    newest_by_author(attestations, attestation_kind, operator_hex)
        .and_then(|a| tag1(a, "d").map(|s| s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATT: u16 = 30420;
    const CARD: u16 = 30422;
    const OP: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const RT: &str = "0000000000000000000000000000000000000000000000000000000000000002";

    fn ev(pubkey: &str, created_at: u64, kind: u16, tags: &[&[&str]]) -> RawEvent {
        RawEvent {
            pubkey: pubkey.to_string(),
            created_at,
            kind,
            tags: tags
                .iter()
                .map(|t| t.iter().map(|s| s.to_string()).collect())
                .collect(),
            content: String::new(),
        }
    }

    fn anchor(created_at: u64, runtime_hex: &str) -> RawEvent {
        ev(OP, created_at, ATT, &[&["d", runtime_hex], &["name", "S"]])
    }

    fn card(created_at: u64, endpoint: &str) -> RawEvent {
        ev(
            RT,
            created_at,
            CARD,
            &[
                &["d", "server"],
                &["op", OP],
                &["endpoint", endpoint],
                &["name", "Cool SMP"],
                &["players", "3", "20"],
                &["protocol", "50"],
                &["privacy", "none"],
            ],
        )
    }

    #[test]
    fn happy_path_resolves_endpoint_and_descriptor() {
        let atts = vec![anchor(100, RT)];
        let cards = vec![card(200, "wss://play/ws")];
        let r = resolve_from_raw(OP, "npub1op", ATT, CARD, &atts, &cards).unwrap();
        assert_eq!(r.endpoints, vec!["wss://play/ws".to_string()]);
        assert_eq!(r.name, "Cool SMP");
        assert_eq!(r.players_max, 20);
        assert_eq!(r.protocol, 50);
        assert_eq!(r.privacy, "none");
        assert_eq!(r.operator_npub, "npub1op");
    }

    #[test]
    fn missing_attestation_is_no_anchor() {
        let cards = vec![card(200, "wss://play/ws")];
        assert_eq!(
            resolve_from_raw(OP, "npub1op", ATT, CARD, &[], &cards),
            Err(ResolveError::NoAnchor)
        );
    }

    #[test]
    fn missing_card_is_no_card() {
        let atts = vec![anchor(100, RT)];
        assert_eq!(
            resolve_from_raw(OP, "npub1op", ATT, CARD, &atts, &[]),
            Err(ResolveError::NoCard)
        );
    }

    #[test]
    fn card_with_wrong_op_is_mismatch() {
        let atts = vec![anchor(100, RT)];
        let bad = ev(
            RT,
            200,
            CARD,
            &[&["d", "server"], &["op", "deadbeef"], &["endpoint", "wss://x"]],
        );
        assert_eq!(
            resolve_from_raw(OP, "npub1op", ATT, CARD, &atts, &[bad]),
            Err(ResolveError::OperatorMismatch)
        );
    }

    #[test]
    fn card_without_endpoint_is_no_endpoint() {
        let atts = vec![anchor(100, RT)];
        let no_ep = ev(RT, 200, CARD, &[&["d", "server"], &["op", OP]]);
        assert_eq!(
            resolve_from_raw(OP, "npub1op", ATT, CARD, &atts, &[no_ep]),
            Err(ResolveError::NoEndpoint)
        );
    }

    #[test]
    fn newest_attestation_and_card_win() {
        let rt_old = "00000000000000000000000000000000000000000000000000000000000000aa";
        let atts = vec![anchor(100, rt_old), anchor(300, RT)]; // newest names RT
        let cards = vec![card(100, "wss://old/ws"), card(500, "wss://new/ws")];
        let r = resolve_from_raw(OP, "npub1op", ATT, CARD, &atts, &cards).unwrap();
        assert_eq!(r.endpoints, vec!["wss://new/ws".to_string()]);
    }

    #[test]
    fn parses_real_event_json_ignoring_id_and_sig() {
        let json = r#"{"id":"abc","pubkey":"0000000000000000000000000000000000000000000000000000000000000002","created_at":200,"kind":30422,"tags":[["d","server"],["op","0000000000000000000000000000000000000000000000000000000000000001"],["endpoint","wss://play/ws"]],"content":"","sig":"ff"}"#;
        let parsed = parse_raw_events(&[json.to_string()]);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].kind, 30422);
        assert_eq!(parsed[0].pubkey, RT);
    }

    #[test]
    fn delegate_runtime_hex_follows_newest_anchor() {
        let rt_old = "00000000000000000000000000000000000000000000000000000000000000aa";
        let atts = vec![anchor(100, rt_old), anchor(300, RT)];
        assert_eq!(delegate_runtime_hex(&atts, ATT, OP).as_deref(), Some(RT));
        assert!(delegate_runtime_hex(&[], ATT, OP).is_none());
    }
}

#![cfg(not(target_arch = "wasm32"))]
//! The delegation attestation: a parameterized-replaceable Nostr event signed by
//! the operator (via their Heartwood / NIP-46 bunker) authorising a server
//! runtime key. Pure types + verification — no IO, no network.

use nostr::{Event, EventBuilder, Kind, NostrSigner, PublicKey, Tag, Timestamp};

/// Provisional attestation event kind (parameterized-replaceable, 30000–39999).
/// To be registered in `forgesworn/nips` before this leaves alpha.
pub const ATTESTATION_KIND: u16 = 30420;

/// Attestation protocol version (the `v` tag).
pub const ATTESTATION_VERSION: &str = "1";

/// A parsed, verified view of an operator-signed attestation event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attestation {
    /// The raw signed event — republishable verbatim (directory-ready).
    pub event: Event,
    /// The operator identity (= `event.pubkey`), the Heartwood-held key.
    pub operator: PublicKey,
    /// The authorised server runtime pubkey (the `d` tag).
    pub server_pubkey: PublicKey,
    pub valid_from: Timestamp,
    pub valid_until: Timestamp,
    /// Event kinds the server runtime key is authorised to sign.
    pub allowed_kinds: Vec<u16>,
    pub server_name: String,
    pub host_hint: Option<String>,
    /// What the attested key is authorised to be. `Some("player")` marks a
    /// player's per-install runtime key (online play by contact, spec §2);
    /// `None` means a server runtime key, which is the original and still the
    /// implied case. Verification does not act on this — the two consumers
    /// (`server_identity::store` and `runtime_identity`) each check the value
    /// they require, so neither can be handed the other's attestation.
    pub role: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttestationError {
    BadSignature,
    WrongServerKey,
    Expired,
    NotYetValid,
    BadKind,
    BadVersion,
    Malformed(String),
}

/// First value of the first tag named `name` (`["name", value, …]`).
fn first_tag<'a>(ev: &'a Event, name: &str) -> Option<&'a str> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == name).then(|| s[1].as_str())
    })
}

/// Structural verification: signature, kind, version, tag schema, and the
/// `expected_server` match — but NOT the time window. Used at load time so an
/// expired attestation still parses (and `is_verified` can report the expiry).
pub fn verify_structural(
    event: &Event,
    expected_server: &PublicKey,
) -> Result<Attestation, AttestationError> {
    if event.verify().is_err() {
        return Err(AttestationError::BadSignature);
    }
    if event.kind != Kind::Custom(ATTESTATION_KIND) {
        return Err(AttestationError::BadKind);
    }
    if first_tag(event, "v") != Some(ATTESTATION_VERSION) {
        return Err(AttestationError::BadVersion);
    }
    let d = first_tag(event, "d").ok_or_else(|| AttestationError::Malformed("missing d".into()))?;
    let server_pubkey =
        PublicKey::from_hex(d).map_err(|_| AttestationError::Malformed("bad d pubkey".into()))?;
    let valid_from = first_tag(event, "valid_from")
        .and_then(|s| s.parse::<u64>().ok())
        .map(Timestamp::from)
        .ok_or_else(|| AttestationError::Malformed("valid_from".into()))?;
    let valid_until = first_tag(event, "valid_until")
        .and_then(|s| s.parse::<u64>().ok())
        .map(Timestamp::from)
        .ok_or_else(|| AttestationError::Malformed("valid_until".into()))?;
    let allowed_kinds: Vec<u16> = event
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            (s.len() >= 2 && s[0] == "k")
                .then(|| s[1].parse::<u16>().ok())
                .flatten()
        })
        .collect();
    let server_name = first_tag(event, "name").unwrap_or("").to_string();
    let host_hint = first_tag(event, "host").map(str::to_string);
    let role = first_tag(event, "role").map(str::to_string);

    if &server_pubkey != expected_server {
        return Err(AttestationError::WrongServerKey);
    }

    Ok(Attestation {
        operator: event.pubkey,
        event: event.clone(),
        server_pubkey,
        valid_from,
        valid_until,
        allowed_kinds,
        server_name,
        host_hint,
        role,
    })
}

/// The authorised server runtime pubkey from an attestation's `d` tag.
pub fn parse_server_pubkey(event: &Event) -> Option<PublicKey> {
    first_tag(event, "d").and_then(|d| PublicKey::from_hex(d).ok())
}

/// Full verification: [`verify_structural`] plus the `[valid_from, valid_until]`
/// window against `now`.
pub fn verify(
    event: &Event,
    expected_server: &PublicKey,
    now: Timestamp,
) -> Result<Attestation, AttestationError> {
    let att = verify_structural(event, expected_server)?;
    if now < att.valid_from {
        return Err(AttestationError::NotYetValid);
    }
    if now > att.valid_until {
        return Err(AttestationError::Expired);
    }
    Ok(att)
}

/// Build and sign a delegation attestation for `server`, signed by the operator
/// `signer` (a NIP-46 bunker in production; a local `Keys` in tests — both
/// implement [`NostrSigner`]). Producing the signature is the only online step:
/// for a bunker it round-trips to the operator's Heartwood for approval.
pub async fn mint_attestation<S: NostrSigner>(
    operator: &S,
    server: &PublicKey,
    valid_from: Timestamp,
    valid_until: Timestamp,
    allowed_kinds: &[u16],
    name: &str,
    host: Option<&str>,
) -> Result<Event, String> {
    let mut tags = vec![
        Tag::parse(["d", &server.to_hex()]).map_err(|e| e.to_string())?,
        Tag::parse(["p", &server.to_hex()]).map_err(|e| e.to_string())?,
        Tag::parse(["valid_from", &valid_from.as_secs().to_string()]).map_err(|e| e.to_string())?,
        Tag::parse(["valid_until", &valid_until.as_secs().to_string()])
            .map_err(|e| e.to_string())?,
        Tag::parse(["v", ATTESTATION_VERSION]).map_err(|e| e.to_string())?,
        Tag::parse(["name", name]).map_err(|e| e.to_string())?,
    ];
    for k in allowed_kinds {
        tags.push(Tag::parse(["k", &k.to_string()]).map_err(|e| e.to_string())?);
    }
    if let Some(h) = host {
        tags.push(Tag::parse(["host", h]).map_err(|e| e.to_string())?);
    }
    EventBuilder::new(Kind::Custom(ATTESTATION_KIND), "")
        .tags(tags)
        .sign(operator)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    // Build a signed attestation event with a local operator key (stands in for
    // a Heartwood). Async because signing goes through the `NostrSigner` contract,
    // exactly as a real bunker would.
    async fn signed(op: &Keys, server: &PublicKey, from: u64, until: u64, version: &str) -> Event {
        let tags = vec![
            Tag::parse(["d", &server.to_hex()]).unwrap(),
            Tag::parse(["p", &server.to_hex()]).unwrap(),
            Tag::parse(["valid_from", &from.to_string()]).unwrap(),
            Tag::parse(["valid_until", &until.to_string()]).unwrap(),
            Tag::parse(["k", "27420"]).unwrap(),
            Tag::parse(["name", "Test World"]).unwrap(),
            Tag::parse(["v", version]).unwrap(),
        ];
        EventBuilder::new(Kind::Custom(ATTESTATION_KIND), "")
            .tags(tags)
            .sign(op)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn mint_round_trips_through_verify() {
        // The operator is a local Keys standing in for a Heartwood: mint → verify
        // proves the full chain with no network.
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        let ev = mint_attestation(
            &op,
            &server,
            Timestamp::from(10),
            Timestamp::from(99),
            &[27420, 27421],
            "Mint World",
            Some("host:8080"),
        )
        .await
        .unwrap();
        let att = verify(&ev, &server, Timestamp::from(50)).unwrap();
        assert_eq!(att.operator, op.public_key());
        assert_eq!(att.allowed_kinds, vec![27420, 27421]);
        assert_eq!(att.server_name, "Mint World");
        assert_eq!(att.host_hint.as_deref(), Some("host:8080"));
    }

    #[tokio::test]
    async fn valid_attestation_verifies() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        let ev = signed(&op, &server, 1_000, 2_000, ATTESTATION_VERSION).await;
        let att = verify(&ev, &server, Timestamp::from(1_500)).unwrap();
        assert_eq!(att.operator, op.public_key());
        assert_eq!(att.server_pubkey, server);
        assert_eq!(att.allowed_kinds, vec![27420]);
        assert_eq!(att.server_name, "Test World");
    }

    #[tokio::test]
    async fn expired_is_rejected() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        let ev = signed(&op, &server, 1_000, 2_000, ATTESTATION_VERSION).await;
        assert_eq!(
            verify(&ev, &server, Timestamp::from(2_001)),
            Err(AttestationError::Expired)
        );
    }

    #[tokio::test]
    async fn not_yet_valid_is_rejected() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        let ev = signed(&op, &server, 1_000, 2_000, ATTESTATION_VERSION).await;
        assert_eq!(
            verify(&ev, &server, Timestamp::from(999)),
            Err(AttestationError::NotYetValid)
        );
    }

    #[tokio::test]
    async fn attestation_for_other_server_is_rejected() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        let ev = signed(&op, &server, 1_000, 2_000, ATTESTATION_VERSION).await;
        assert_eq!(
            verify(&ev, &other, Timestamp::from(1_500)),
            Err(AttestationError::WrongServerKey)
        );
    }

    #[tokio::test]
    async fn tampered_event_is_rejected() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        let mut ev = signed(&op, &server, 1_000, 2_000, ATTESTATION_VERSION).await;
        // Corrupt the content so the stored signature no longer matches the id.
        ev.content = "tampered".to_string();
        assert_eq!(
            verify(&ev, &server, Timestamp::from(1_500)),
            Err(AttestationError::BadSignature)
        );
    }

    #[tokio::test]
    async fn unknown_version_is_rejected() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        let ev = signed(&op, &server, 1_000, 2_000, "999").await;
        assert_eq!(
            verify(&ev, &server, Timestamp::from(1_500)),
            Err(AttestationError::BadVersion)
        );
    }

    #[tokio::test]
    async fn structural_accepts_expired_but_full_rejects() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        let ev = signed(&op, &server, 1_000, 2_000, ATTESTATION_VERSION).await;
        // Structural verification ignores the time window.
        assert!(verify_structural(&ev, &server).is_ok());
        // Full verification enforces it.
        assert_eq!(
            verify(&ev, &server, Timestamp::from(5_000)),
            Err(AttestationError::Expired)
        );
    }

    #[tokio::test]
    async fn role_tag_is_read_and_absent_means_server() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        // A server attestation carries no `role` tag at all.
        let ev = signed(&op, &server, 1_000, 2_000, ATTESTATION_VERSION).await;
        assert_eq!(verify_structural(&ev, &server).unwrap().role, None);

        // A player attestation carries role=player and still verifies.
        let mut tags = ev.tags.iter().map(|t| t.clone()).collect::<Vec<_>>();
        tags.push(Tag::parse(["role", "player"]).unwrap());
        let player_ev = EventBuilder::new(Kind::Custom(ATTESTATION_KIND), "")
            .tags(tags)
            .sign(&op)
            .await
            .unwrap();
        assert_eq!(
            verify_structural(&player_ev, &server).unwrap().role.as_deref(),
            Some("player")
        );
    }
}

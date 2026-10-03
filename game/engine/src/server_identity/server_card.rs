#![cfg(not(target_arch = "wasm32"))]
// Forward-API: build/verify are consumed by the resolver + publisher (Spec A
// tasks 4–9); fully tested here. Allow dead_code until those land.
#![allow(dead_code)]
//! Address Card (Spec A) — kind-30422.
//!
//! A runtime-signed, addressable Nostr event that carries a server's live
//! endpoint(s) + descriptor + capacity + a privacy tag. It is the
//! frequently-updated half of npub→address resolution: the box signs it locally
//! with its runtime key (no bunker), and a resolver verifies the chain
//! `card signer (runtime key) → attestation → operator npub`, exactly like the
//! signed-claim primitive. Relays are an untrusted hint layer; the real trust
//! comes from the join proof on connect (design §4). This module is the sign +
//! parse + verify primitive.
//!
//! Modelled on `server_identity::claim`. Provisional kind — register `30422` in
//! `forgesworn/nips` when built (alongside `30420`).

use nostr::{Event, EventBuilder, Kind, PublicKey, Tag, Timestamp, ToBech32};

use crate::server_identity::attestation::{parse_server_pubkey, verify};
use crate::server_identity::store::ServerIdentity;

/// Address Card event kind (addressable / parameterized-replaceable). Provisional.
pub const CARD_KIND: u16 = 30422;

/// The decoded contents of an Address Card.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ServerCard {
    /// Dialable ws(s) endpoints, in preference order (failover).
    pub endpoints: Vec<String>,
    /// Server display name.
    pub name: String,
    /// Short blurb.
    pub about: String,
    /// Region hint (e.g. "eu-west").
    pub region: String,
    /// Current player count (best-effort, refreshed on heartbeat).
    pub players_cur: u16,
    /// Max player count.
    pub players_max: u16,
    /// Engine protocol version the server speaks.
    pub protocol: u32,
    /// Privacy posture tag (Spec C defines values; "none"/"unset" here).
    pub privacy: String,
}

/// A verified Address Card with its proven operator + signer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardView {
    /// The runtime key that signed the card.
    pub server_pubkey: PublicKey,
    /// The operator the runtime key chains to (verified).
    pub operator: PublicKey,
    /// The decoded card.
    pub card: ServerCard,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardError {
    Malformed,
    AttestationInvalid,
    OperatorMismatch,
    BadSignature,
    /// The card was signed by a key the attestation doesn't authorise.
    WrongSigner,
}

/// Build + sign an Address Card with the server runtime key. `None` if the
/// server is unprovisioned (an unattested card isn't verifiable, so we don't
/// make one).
pub fn build_card_event(id: &ServerIdentity, card: &ServerCard) -> Option<Event> {
    let op_hex = id.attestation()?.operator.to_hex();
    let mut tags: Vec<Tag> = Vec::new();
    tags.push(Tag::parse(["d", "server"]).ok()?); // replaceable key (one card per runtime key)
    tags.push(Tag::parse(["op", &op_hex]).ok()?); // operator back-reference (cross-checked)
    for ep in &card.endpoints {
        tags.push(Tag::parse(["endpoint", ep]).ok()?);
    }
    tags.push(Tag::parse(["name", &card.name]).ok()?);
    tags.push(Tag::parse(["about", &card.about]).ok()?);
    tags.push(Tag::parse(["region", &card.region]).ok()?);
    tags.push(
        Tag::parse([
            "players",
            &card.players_cur.to_string(),
            &card.players_max.to_string(),
        ])
        .ok()?,
    );
    tags.push(Tag::parse(["protocol", &card.protocol.to_string()]).ok()?);
    tags.push(Tag::parse(["privacy", &card.privacy]).ok()?);
    let unsigned = EventBuilder::new(Kind::Custom(CARD_KIND), "")
        .tags(tags)
        .build(id.runtime_pubkey());
    id.sign_event(unsigned).ok()
}

/// Build + sign a NIP-09 deletion (kind 5) that retracts this server's Address
/// Card (`a` = `30422:<runtime pubkey>:server`). Published when the operator
/// turns announcing off, so the card stops being discoverable instead of lingering
/// on relays. Signed by the runtime key — the same key that signed the card, as
/// NIP-09 requires.
pub fn build_card_delete_event(id: &ServerIdentity) -> Option<Event> {
    let coord = format!("{}:{}:server", CARD_KIND, id.runtime_pubkey().to_hex());
    let tags = vec![
        Tag::parse(["a", &coord]).ok()?,
        Tag::parse(["k", &CARD_KIND.to_string()]).ok()?,
    ];
    let unsigned = EventBuilder::new(Kind::EventDeletion, "server stopped announcing")
        .tags(tags)
        .build(id.runtime_pubkey());
    id.sign_event(unsigned).ok()
}

/// Decode an Address Card event's tags into a [`ServerCard`]. Lenient: unknown
/// tags ignored, missing fields default. Returns `None` only if the event isn't
/// shaped like a card at all (no tags at all is still `Some` with defaults — the
/// caller decides whether an empty card is useful).
pub fn parse_card(event: &Event) -> Option<ServerCard> {
    let mut card = ServerCard::default();
    for t in event.tags.iter() {
        let s = t.as_slice();
        if s.len() < 2 {
            continue;
        }
        match s[0].as_str() {
            "endpoint" => card.endpoints.push(s[1].clone()),
            "name" => card.name = s[1].clone(),
            "about" => card.about = s[1].clone(),
            "region" => card.region = s[1].clone(),
            "players" => {
                card.players_cur = s.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                card.players_max = s.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            "protocol" => card.protocol = s.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            "privacy" => card.privacy = s[1].clone(),
            _ => {}
        }
    }
    Some(card)
}

/// Verify an Address Card against the operator npub the consumer expects. Needs
/// the server's attestation to chain `card signer (runtime key) → operator`.
/// Note: like [`crate::server_identity::claim::verify_claim`], this does not
/// enforce the attestation's `allowed_kinds`; the trust chain holds because the
/// signer must be the attested runtime key and the operator is cross-checked.
pub fn verify_card(
    card_evt: &Event,
    attestation: &Event,
    expected_op_npub: &str,
    now: Timestamp,
) -> Result<CardView, CardError> {
    let server_pubkey = parse_server_pubkey(attestation).ok_or(CardError::Malformed)?;
    let att = verify(attestation, &server_pubkey, now).map_err(|_| CardError::AttestationInvalid)?;
    // A `role=player` event is a persona's delegation to a PLAYER's runtime key
    // (`runtime_identity`), never a server's. `store::store_attestation` already
    // refuses one; so must every consumer that verifies a delegation it did not
    // store, or a player attestation stands in for a server one.
    if att.role.is_some() {
        return Err(CardError::AttestationInvalid);
    }
    let op_npub = att.operator.to_bech32().map_err(|_| CardError::Malformed)?;
    if op_npub != expected_op_npub {
        return Err(CardError::OperatorMismatch);
    }
    if card_evt.verify().is_err() {
        return Err(CardError::BadSignature);
    }
    if card_evt.pubkey != server_pubkey {
        return Err(CardError::WrongSigner);
    }
    if card_evt.kind != Kind::Custom(CARD_KIND) {
        return Err(CardError::Malformed);
    }
    // Defence in depth: the card's own `op` tag must name the attested operator.
    let op_hex = att.operator.to_hex();
    let op_tag_ok = card_evt.tags.iter().any(|t| {
        let s = t.as_slice();
        s.len() >= 2 && s[0] == "op" && s[1] == op_hex
    });
    if !op_tag_ok {
        return Err(CardError::OperatorMismatch);
    }
    let card = parse_card(card_evt).ok_or(CardError::Malformed)?;
    Ok(CardView {
        server_pubkey,
        operator: att.operator,
        card,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_identity::attestation::mint_attestation;
    use crate::server_identity::store::generate_runtime;
    use nostr::{EventBuilder, Keys, Tag, ToBech32};

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("axe_card_{}_{}", tag, std::process::id()))
    }

    async fn provisioned(dir: &std::path::Path, op: &Keys) -> (ServerIdentity, Event) {
        let mut id = generate_runtime(dir).unwrap();
        let att = mint_attestation(
            op,
            &id.runtime_pubkey(),
            Timestamp::from(0),
            Timestamp::from(u64::MAX >> 1),
            &[CARD_KIND],
            "W",
            None,
        )
        .await
        .unwrap();
        id.store_attestation(dir, att.clone()).unwrap();
        (id, att)
    }

    fn sample_card() -> ServerCard {
        ServerCard {
            endpoints: vec![
                "wss://play.example.com:8080/ws".into(),
                "wss://203.0.113.7:8080/ws".into(),
            ],
            name: "Cool SMP".into(),
            about: "no griefing".into(),
            region: "eu-west".into(),
            players_cur: 3,
            players_max: 20,
            protocol: 50,
            privacy: "none".into(),
        }
    }

    #[test]
    fn delete_event_targets_the_card_coordinate() {
        let dir = tmp("del");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        let evt = build_card_delete_event(&id).unwrap();
        assert_eq!(evt.kind, Kind::EventDeletion);
        assert_eq!(evt.pubkey, id.runtime_pubkey());
        let want = format!("{}:{}:server", CARD_KIND, id.runtime_pubkey().to_hex());
        assert!(evt
            .tags
            .iter()
            .any(|t| t.as_slice() == ["a".to_string(), want.clone()]));
        assert!(evt.verify().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn card_round_trips_and_verifies() {
        let dir = tmp("ok");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (id, att) = provisioned(&dir, &op).await;
        let card = sample_card();
        let evt = build_card_event(&id, &card).unwrap();

        let parsed = parse_card(&evt).unwrap();
        assert_eq!(parsed.endpoints, card.endpoints);
        assert_eq!(parsed.name, "Cool SMP");
        assert_eq!(parsed.players_cur, 3);
        assert_eq!(parsed.players_max, 20);
        assert_eq!(parsed.protocol, 50);
        assert_eq!(parsed.privacy, "none");

        let npub = op.public_key().to_bech32().unwrap();
        let view = verify_card(&evt, &att, &npub, Timestamp::from(100)).unwrap();
        assert_eq!(view.operator, op.public_key());
        assert_eq!(view.server_pubkey, id.runtime_pubkey());
        assert_eq!(view.card.endpoints, card.endpoints);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unprovisioned_builds_no_card() {
        let dir = tmp("none");
        let _ = std::fs::remove_dir_all(&dir);
        let id = generate_runtime(&dir).unwrap();
        assert!(build_card_event(&id, &sample_card()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn card_rejected_for_wrong_operator() {
        let dir = tmp("op");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (id, att) = provisioned(&dir, &op).await;
        let evt = build_card_event(&id, &sample_card()).unwrap();
        let other = Keys::generate().public_key().to_bech32().unwrap();
        assert_eq!(
            verify_card(&evt, &att, &other, Timestamp::from(100)),
            Err(CardError::OperatorMismatch)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tampered_card_rejected() {
        let dir = tmp("tamper");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (id, att) = provisioned(&dir, &op).await;
        let mut evt = build_card_event(&id, &sample_card()).unwrap();
        evt.content = "tampered".to_string();
        let npub = op.public_key().to_bech32().unwrap();
        assert_eq!(
            verify_card(&evt, &att, &npub, Timestamp::from(100)),
            Err(CardError::BadSignature)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// REGRESSION (whole-branch review, MINOR 7). `store::store_attestation`
    /// refuses a `role=player` delegation, but `verify_card` verified one it was
    /// handed — so a player's own attestation, which any player can get with one
    /// phone tap, could be presented as an operator's delegation to a server.
    #[tokio::test]
    async fn a_player_delegation_is_not_a_server_one() {
        let persona = Keys::generate();
        // A player's per-install runtime key, and the attestation their one
        // phone tap produces over it.
        let runtime = Keys::generate();
        let player_att = crate::runtime_identity::mint_player_attestation(
            &persona,
            &runtime.public_key(),
            Timestamp::from(0),
            365,
        )
        .await
        .unwrap();
        // A card genuinely signed by the attested key, naming the attesting
        // persona as its operator — every other check in `verify_card` passes.
        let op_hex = persona.public_key().to_hex();
        let evt = EventBuilder::new(Kind::Custom(CARD_KIND), "")
            .tags([
                Tag::parse(["d", "server"]).unwrap(),
                Tag::parse(["op", &op_hex]).unwrap(),
                Tag::parse(["endpoint", "wss://evil/ws"]).unwrap(),
            ])
            .sign_with_keys(&runtime)
            .unwrap();
        let npub = persona.public_key().to_bech32().unwrap();
        assert_eq!(
            verify_card(&evt, &player_att, &npub, Timestamp::from(100)),
            Err(CardError::AttestationInvalid),
            "a role=player delegation must never stand in for a server's"
        );
    }

    #[tokio::test]
    async fn card_from_unauthorised_signer_rejected() {
        let dir = tmp("signer");
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (_id, att) = provisioned(&dir, &op).await;
        // A card validly signed by some OTHER key, presented with the real attestation.
        let imposter = Keys::generate();
        let op_hex = op.public_key().to_hex();
        let evt = EventBuilder::new(Kind::Custom(CARD_KIND), "")
            .tags([
                Tag::parse(["d", "server"]).unwrap(),
                Tag::parse(["op", &op_hex]).unwrap(),
                Tag::parse(["endpoint", "wss://evil/ws"]).unwrap(),
            ])
            .sign_with_keys(&imposter)
            .unwrap();
        let npub = op.public_key().to_bech32().unwrap();
        assert_eq!(
            verify_card(&evt, &att, &npub, Timestamp::from(100)),
            Err(CardError::WrongSigner)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

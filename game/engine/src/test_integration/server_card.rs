//! End-to-end Server Card test (Spec A). Unlike the per-module unit tests (which
//! hand-build `RawEvent`s), this exercises the REAL wire format: mint an operator
//! attestation + sign a Card with the runtime key (task 1), serialise both to the
//! JSON a relay would deliver, then resolve via the cross-platform core (task 4).
//! This guards the Card tag schema and the resolver parsing against drift.
//! Native (`nostr`); reached by `cargo test --lib`.

use nostr::{JsonUtil, Keys, Timestamp, ToBech32};

use crate::server_identity::attestation::{mint_attestation, ATTESTATION_KIND};
use crate::server_identity::server_card::{build_card_event, ServerCard, CARD_KIND};
use crate::server_identity::store::generate_runtime;
use crate::server_resolve::{parse_raw_events, resolve_from_raw, ResolveError};

fn tmp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("axe_e2e_card_{}_{}", tag, std::process::id()))
}

fn sample_card() -> ServerCard {
    ServerCard {
        endpoints: vec!["wss://play.example.com:8080/ws".into()],
        name: "Cool SMP".into(),
        about: "no griefing".into(),
        region: "eu-west".into(),
        players_cur: 3,
        players_max: 20,
        protocol: 50,
        privacy: "none".into(),
    }
}

#[tokio::test]
async fn real_signed_card_resolves_end_to_end() {
    let dir = tmp("ok");
    let _ = std::fs::remove_dir_all(&dir);
    let op = Keys::generate();

    // Provision: runtime key + operator attestation authorising the Card kind.
    let mut id = generate_runtime(&dir).unwrap();
    let att = mint_attestation(
        &op,
        &id.runtime_pubkey(),
        Timestamp::from(0),
        Timestamp::from(u64::MAX >> 1),
        &[CARD_KIND],
        "S",
        None,
    )
    .await
    .unwrap();
    id.store_attestation(&dir, att.clone()).unwrap();

    let card = sample_card();
    let card_evt = build_card_event(&id, &card).unwrap();

    // Serialise to the JSON a relay would deliver, then resolve through the core.
    let atts = parse_raw_events(&[att.as_json()]);
    let cards = parse_raw_events(&[card_evt.as_json()]);
    let op_hex = op.public_key().to_hex();
    let op_npub = op.public_key().to_bech32().unwrap();

    let resolved =
        resolve_from_raw(&op_hex, &op_npub, ATTESTATION_KIND, CARD_KIND, &atts, &cards).unwrap();
    assert_eq!(resolved.endpoints, vec!["wss://play.example.com:8080/ws".to_string()]);
    assert_eq!(resolved.name, "Cool SMP");
    assert_eq!(resolved.about, "no griefing");
    assert_eq!(resolved.region, "eu-west");
    assert_eq!(resolved.players_cur, 3);
    assert_eq!(resolved.players_max, 20);
    assert_eq!(resolved.protocol, 50);
    assert_eq!(resolved.privacy, "none");
    assert_eq!(resolved.operator_npub, op_npub);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn card_from_non_delegated_key_does_not_resolve() {
    let dir = tmp("real");
    let imp_dir = tmp("imp");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&imp_dir);
    let op = Keys::generate();

    // The real, delegated server.
    let mut id = generate_runtime(&dir).unwrap();
    let att = mint_attestation(
        &op,
        &id.runtime_pubkey(),
        Timestamp::from(0),
        Timestamp::from(u64::MAX >> 1),
        &[CARD_KIND],
        "S",
        None,
    )
    .await
    .unwrap();
    id.store_attestation(&dir, att.clone()).unwrap();

    // An impostor server with its OWN (unrelated) operator + runtime key — its
    // card is validly signed but by a key the real operator never delegated to.
    let mut imp_id = generate_runtime(&imp_dir).unwrap();
    let imp_att = mint_attestation(
        &Keys::generate(),
        &imp_id.runtime_pubkey(),
        Timestamp::from(0),
        Timestamp::from(u64::MAX >> 1),
        &[CARD_KIND],
        "X",
        None,
    )
    .await
    .unwrap();
    imp_id.store_attestation(&imp_dir, imp_att).unwrap();
    let imp_card = build_card_event(
        &imp_id,
        &ServerCard {
            endpoints: vec!["wss://evil/ws".into()],
            ..Default::default()
        },
    )
    .unwrap();

    // Present the REAL operator's attestation but only the impostor's card. The
    // resolver follows the attestation to the real delegate key and finds no card
    // for it — so it refuses (the impostor endpoint is never returned).
    let atts = parse_raw_events(&[att.as_json()]);
    let cards = parse_raw_events(&[imp_card.as_json()]);
    let op_hex = op.public_key().to_hex();
    let op_npub = op.public_key().to_bech32().unwrap();

    assert_eq!(
        resolve_from_raw(&op_hex, &op_npub, ATTESTATION_KIND, CARD_KIND, &atts, &cards),
        Err(ResolveError::NoCard)
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&imp_dir);
}

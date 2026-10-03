//! Two players, two loopback sockets, one in-memory relay: the whole rendezvous
//! end to end with no network policy involved.
//!
//! What this covers that the per-module tests do not: that the host's answer is
//! actually openable by the joiner it was addressed to, that the candidates the
//! joiner receives are the ones the host gathered on its real socket, and that
//! the refusal paths behave the same way when both halves are wired together
//! rather than driven by hand.
//!
//! The QUIC half is deliberately NOT driven here — the connect race is covered
//! by `nat::punch`'s pure state machine and by
//! `handshake::connect_to_server_on_socket_*`. What is left is the two-machine
//! live test on the owner's test sheet.
//!
//! A protocol mismatch has no end-to-end test here for the good reason that it
//! cannot be built through the public API: `OnlineJoin::start` stamps
//! `PROTOCOL_VERSION` into every offer it seals, so an in-process joiner can
//! never disagree with an in-process host. Both halves of that path are pinned
//! by hand-driven unit tests instead —
//! `online_host::tests::a_protocol_mismatch_is_explained_rather_than_ignored`
//! and `online_join::tests::a_protocol_mismatch_refusal_is_explained_to_the_player`.
#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use nostr::{JsonUtil, Keys};

use crate::comms::Tier;
use crate::contacts::{AddedVia, Contact};
use crate::online_host::{HostEvent, OnlineHost};
use crate::online_join::{JoinStep, OnlineJoin};
use crate::rendezvous::payload::{npub_of, Candidate, KIND_JOIN_ANSWER, KIND_JOIN_OFFER};
use crate::rendezvous::relay_client::{FakeRelayHub, RendezvousRelay};
use crate::runtime_identity::{mint_player_attestation, RuntimeIdentity};

const NOW: u64 = 1_700_000_000;

/// One frame of the game loop, as far as a join is concerned. Short enough that
/// nothing here can time out by accident — `elapsed` is injected, never slept.
const A_FRAME: Duration = Duration::from_millis(50);

/// A persona key plus the runtime key it has attested — one player's whole
/// identity, both halves, as the rendezvous needs it.
struct Player {
    persona: Keys,
    identity: RuntimeIdentity,
}

impl Player {
    fn new() -> Self {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        // `pollster`, not a tokio runtime: minting is pure CPU and only `async`
        // because `NostrSigner` is, and `Runtime::block_on` panics if a future
        // driver ever calls this from inside a runtime.
        let attestation = pollster::block_on(mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(NOW - 10),
            90,
        ))
        .unwrap();
        Player {
            persona,
            identity: RuntimeIdentity::from_parts(runtime, Some(attestation)),
        }
    }

    fn persona_bytes(&self) -> [u8; 32] {
        self.persona.public_key().to_bytes()
    }

    fn runtime_bytes(&self) -> [u8; 32] {
        self.identity.runtime_pubkey().to_bytes()
    }

    /// A second handle on the same identity, because `OnlineHost::start` and
    /// `OnlineJoin::start` each take one by value and the test still wants to
    /// read the player's keys afterwards.
    fn identity(&self) -> RuntimeIdentity {
        RuntimeIdentity::from_parts(
            self.identity.keys().clone(),
            self.identity.attestation().cloned(),
        )
    }
}

fn lan(addr: &str) -> Candidate {
    Candidate {
        kind: "lan".to_string(),
        addr: addr.to_string(),
    }
}

/// A host on a real loopback socket, wired to the shared hub, with `book` as
/// its contacts.
fn host_for(hub: &FakeRelayHub, host: &Player, book: Vec<Contact>) -> OnlineHost {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    OnlineHost::start(
        host.identity(),
        host.persona_bytes(),
        Box::new(hub.client()),
        sock,
        vec![lan("127.0.0.1:7700")],
        "Ivy's Hollow".to_string(),
        vec!["wss://nos.lol".to_string()],
        book,
        NOW,
    )
    .map_err(|(e, _relay)| e)
    .unwrap()
}

/// A joiner calling `host_runtime`, publishing its offer as it starts.
fn join_for(
    hub: &FakeRelayHub,
    joiner: &Player,
    host_runtime: [u8; 32],
    bearer: Option<[u8; 16]>,
    now: u64,
) -> OnlineJoin {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    OnlineJoin::start(
        joiner.identity(),
        joiner.persona_bytes(),
        Box::new(hub.client()),
        sock,
        vec![lan("127.0.0.1:0")],
        host_runtime,
        bearer,
        "Rowan".to_string(),
        now,
    )
    .map_err(|(e, _relay)| e)
    .unwrap()
}

/// Every `Dropped` reason the host logged, joined — so a test can say which
/// gate turned somebody away and not merely that one of them did.
fn dropped(events: &[HostEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Dropped(why) => Some(why.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn an_invite_admits_a_stranger_and_both_sides_end_up_as_contacts() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let host_player = Player::new();
    let mut host = host_for(&hub, &host_player, vec![]);
    let bearer = host.invite().bearer;

    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), Some(bearer), NOW);

    // The host sees the offer, admits it, and answers.
    let events = host.poll(NOW, 1, 5);
    assert!(
        events.iter().any(|e| matches!(e, HostEvent::ContactAdded(_))),
        "the host records the caller: {events:?}"
    );
    assert!(host.allowlist().contains(&joiner.persona_bytes()));

    // The joiner sees the answer and moves on to connecting.
    let step = join.poll(NOW, A_FRAME);
    assert_eq!(
        step,
        JoinStep::Connecting {
            world_name: "Ivy's Hollow".to_string()
        },
        "an accepted answer must name the world"
    );
    let host_contact = join
        .host_contact()
        .expect("the host becomes the joiner's contact too");
    assert_eq!(host_contact.pubkey, host_player.persona_bytes());
    assert_eq!(host_contact.tier, Tier::Kith);
    assert_eq!(host_contact.added_via, AddedVia::Invite);
}

#[test]
fn a_known_contact_joins_with_no_bearer_at_all() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let book = vec![Contact {
        pubkey: joiner.persona_bytes(),
        display_name: Some("Rowan".to_string()),
        tier: Tier::Kith,
        is_child: false,
        runtime_pubkey: Some(joiner.runtime_bytes()),
        added_via: AddedVia::Invite,
        added_at: NOW - 86_400,
        last_joined: None,
    }];
    let host_player = Player::new();
    let mut host = host_for(&hub, &host_player, book);
    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), None, NOW);

    let events = host.poll(NOW, 1, 5);
    assert!(
        matches!(join.poll(NOW, A_FRAME), JoinStep::Connecting { .. }),
        "a contact needs no invite: {}",
        dropped(&events)
    );
}

#[test]
fn a_stranger_is_met_with_silence_and_then_the_didnt_answer_copy() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let host_player = Player::new();
    let mut host = host_for(&hub, &host_player, vec![]);
    // No bearer, not a contact.
    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), None, NOW);

    let events = host.poll(NOW, 1, 5);
    // The offer did arrive and was turned away on identity — not lost, not
    // stale, not malformed. Without this the silence below would pass for a
    // test that never delivered anything.
    assert!(
        dropped(&events).contains("NotAContact"),
        "turned away as a stranger: {events:?}"
    );
    assert!(host.allowlist().is_empty());
    assert_eq!(join.poll(NOW, A_FRAME), JoinStep::Waiting, "nothing came back");
    assert_eq!(
        join.poll(NOW, crate::online_join::ANSWER_TIMEOUT),
        JoinStep::Failed(crate::online_join::failure_copy(
            crate::online_join::JoinFailure::NoAnswer,
            "Rowan"
        )),
        "silence eventually reads as 'they didn't answer'"
    );
}

#[test]
fn an_expired_bearer_is_also_met_with_silence() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let host_player = Player::new();
    let mut host = host_for(&hub, &host_player, vec![]);
    let bearer = host.invite().bearer;
    let expiry = host.invite().expires_at;
    // The call is placed AFTER the invite lapsed, not before: an offer stamped
    // at `NOW` and read two days later would be thrown out as stale long before
    // the bearer was ever looked at, and this test would prove nothing.
    let lapsed = expiry + 1;
    let mut join = join_for(
        &hub,
        &joiner,
        host_player.runtime_bytes(),
        Some(bearer),
        lapsed,
    );

    let events = host.poll(lapsed, 1, 5);
    assert!(
        dropped(&events).contains("BearerExpired"),
        "the bearer, not the clock, is what turned them away: {events:?}"
    );
    assert!(host.allowlist().is_empty());
    assert_eq!(
        join.poll(lapsed, A_FRAME),
        JoinStep::Waiting,
        "a lapsed bearer must not be told it lapsed — that confirms a host is here"
    );
}

#[test]
fn a_full_world_is_explained_end_to_end() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let book = vec![Contact {
        pubkey: joiner.persona_bytes(),
        display_name: Some("Rowan".to_string()),
        tier: Tier::Kin,
        is_child: false,
        runtime_pubkey: Some(joiner.runtime_bytes()),
        added_via: AddedVia::Kenspeckle,
        added_at: 0,
        last_joined: None,
    }];
    let host_player = Player::new();
    let mut host = host_for(&hub, &host_player, book);
    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), None, NOW);

    host.poll(NOW, 5, 5);
    assert_eq!(
        join.poll(NOW, A_FRAME),
        JoinStep::Failed(crate::online_join::failure_copy(
            crate::online_join::JoinFailure::Full,
            "Rowan"
        ))
    );
    assert!(
        host.allowlist().is_empty(),
        "a refused caller is not let through the door either"
    );
}

#[test]
fn nothing_the_relay_saw_names_a_person_a_world_or_an_address() {
    // The red-line-3 assertion at the level that matters: everything that
    // crossed the fake relay during a real admission.
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let host_player = Player::new();
    let mut host = host_for(&hub, &host_player, vec![]);
    let bearer = host.invite().bearer;
    // Two observers, not one with two filters: a relay treats a repeated
    // subscription id as an overwrite (see `RendezvousRelay::subscribe`), so a
    // single client can only ever hold one of these.
    let offers_seen = hub.client();
    offers_seen
        .subscribe(KIND_JOIN_OFFER, &hex::encode(host_player.runtime_bytes()))
        .unwrap();
    let answers_seen = hub.client();
    answers_seen
        .subscribe(KIND_JOIN_ANSWER, &hex::encode(joiner.runtime_bytes()))
        .unwrap();

    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), Some(bearer), NOW);
    host.poll(NOW, 1, 5);
    let _ = join.poll(NOW, A_FRAME);

    let mut seen = 0;
    while let Some(ev) = offers_seen.try_recv().or_else(|| answers_seen.try_recv()) {
        seen += 1;
        let wire = ev.as_json();
        // Each needle is either long enough that a chance match in base64 is
        // not a real number (a 63-character npub, 32 hex characters of bearer)
        // or carries a character base64 cannot contain (the apostrophe in
        // "Ivy's", the dots in an address). So a hit here is a real leak and
        // never a collision inside the NIP-44 ciphertext — which matters,
        // because a flaky red-line test is a red-line test nobody trusts.
        assert!(!wire.contains("Ivy's"), "world name leaked: {wire}");
        assert!(!wire.contains("127.0.0.1"), "address leaked: {wire}");
        assert!(
            !wire.contains(&npub_of(&host_player.persona.public_key())),
            "host persona leaked"
        );
        assert!(
            !wire.contains(&npub_of(&joiner.persona.public_key())),
            "joiner persona leaked"
        );
        assert!(!wire.contains(&hex::encode(bearer)), "bearer leaked");
    }
    assert_eq!(
        seen, 2,
        "an offer and an answer should both have crossed the relay"
    );
}

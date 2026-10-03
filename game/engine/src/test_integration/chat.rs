//! World chat (Phase 2) — integration tests for `server::handle_chat_say`,
//! the pure pipeline `hosted_server.rs`'s `ChatSay` dispatch runs (verified-
//! key gate, rate limit, sanitiser, tier rule). `TestHost` wraps `GameServer`
//! directly with no transport plumbing (see `test_harness.rs`'s module doc),
//! so — following the idiom in `handshake.rs` — these tests drive the
//! pipeline function directly via `TestHost::server.players` (already `pub`)
//! rather than standing up real transports.
//!
//! Spec: `docs/foundations/2026-09-05-world-chat.md`.

use crate::comms::Tier;
use crate::server::{handle_chat_say, ChatSayOutcome};
use crate::test_harness::{TestConfig, TestHost};

fn two_player_host() -> TestHost {
    TestHost::start_with(TestConfig { num_players: 2, ..Default::default() })
}

/// Mark player `i` as verified with a distinct, deterministic pubkey.
/// Mark slot `i` verified, with the level a verified adult's policy resolves
/// to at join (`Anyone`). `ServerPlayer::new` defaults to `Blocked`; only a
/// verified identity can get there (and, since 2026-09-28, only a future
/// capability boundary — a Charter comms clause or Signet guardian attestation
/// — can raise the Charter ceiling above `Approved`; tests set it directly).
fn verify(host: &mut TestHost, i: usize, byte: u8) {
    host.server.players[i].verified_pubkey = Some([byte; 32]);
    host.server.players[i].comms = crate::comms::Party::at(crate::comms::CommsLevel::Anyone);
}

#[test]
fn two_anyone_players_hear_each_other() {
    let mut host = two_player_host();
    verify(&mut host, 0, 0x01);
    verify(&mut host, 1, 0x02);
    // Both at Anyone (set by `verify`).

    let outcome = handle_chat_say(&mut host.server.players, 0, "hello world", 0);
    match outcome {
        ChatSayOutcome::Delivered { text, recipients } => {
            assert_eq!(text, "hello world");
            assert_eq!(recipients, vec![1]);
        }
        other => panic!("expected Delivered, got {other:?}"),
    }
}

#[test]
fn approved_player_with_empty_contacts_hears_no_stranger_and_reaches_none() {
    let mut host = two_player_host();
    verify(&mut host, 0, 0x01); // the "host" — stays on Anyone
    verify(&mut host, 1, 0x02); // the restricted player — Approved, empty book
    host.server.players[1].comms = crate::comms::Party::at(crate::comms::CommsLevel::Approved);

    // The Anyone player (0) speaks; the Approved player (1) hasn't ken'd them
    // — hear_ok(Approved, Stranger) is false, so it must not land.
    let from_anyone = handle_chat_say(&mut host.server.players, 0, "hi", 0);
    match from_anyone {
        ChatSayOutcome::Delivered { recipients, .. } => {
            assert!(recipients.is_empty(), "stranger's line reached the Approved player");
        }
        other => panic!("expected Delivered (with no recipients), got {other:?}"),
    }

    // The Approved player replies; speak_ok(Approved, Stranger) is false, so
    // their own line doesn't reach the stranger either.
    let from_approved = handle_chat_say(&mut host.server.players, 1, "hi back", 10);
    match from_approved {
        ChatSayOutcome::Delivered { recipients, .. } => {
            assert!(recipients.is_empty(), "Approved player's line reached a stranger");
        }
        other => panic!("expected Delivered (with no recipients), got {other:?}"),
    }
}

#[test]
fn approved_player_with_kin_contact_talks_both_ways() {
    let mut host = two_player_host();
    let parent_pk = [0x01u8; 32];
    verify(&mut host, 0, 0x01); // parent — Anyone
    verify(&mut host, 1, 0x02); // child — Approved
    host.server.players[1].comms = crate::comms::Party::at(crate::comms::CommsLevel::Approved);
    // Only the child's own book needs the entry: `tier_of` is consulted for
    // both roles (speaker's view of listener, listener's view of speaker),
    // so a single Kin entry on the restricted side covers both directions.
    host.server.players[1].contacts.insert(parent_pk, Tier::Kin);

    let parent_to_child = handle_chat_say(&mut host.server.players, 0, "dinner's ready", 0);
    match parent_to_child {
        ChatSayOutcome::Delivered { recipients, .. } => assert_eq!(recipients, vec![1]),
        other => panic!("expected Delivered, got {other:?}"),
    }

    let child_to_parent = handle_chat_say(&mut host.server.players, 1, "coming!", 10);
    match child_to_parent {
        ChatSayOutcome::Delivered { recipients, .. } => assert_eq!(recipients, vec![0]),
        other => panic!("expected Delivered, got {other:?}"),
    }
}

#[test]
fn ken_is_heard_but_the_reply_does_not_go_back() {
    let mut host = two_player_host();
    let host_pk = [0x01u8; 32];
    verify(&mut host, 0, 0x01); // a well-known host — Anyone
    verify(&mut host, 1, 0x02); // a restricted child — Approved, has ken'd the host
    host.server.players[1].comms = crate::comms::Party::at(crate::comms::CommsLevel::Approved);
    host.server.players[1].contacts.insert(host_pk, Tier::Ken);

    // The host speaks to the room; the child hears them (hear_ok(Approved, Ken)).
    let host_speaks = handle_chat_say(&mut host.server.players, 0, "welcome!", 0);
    match host_speaks {
        ChatSayOutcome::Delivered { recipients, .. } => assert_eq!(recipients, vec![1]),
        other => panic!("expected Delivered, got {other:?}"),
    }

    // The child replies; speak_ok(Approved, Ken) is false — ken grants no
    // right to speak back.
    let child_replies = handle_chat_say(&mut host.server.players, 1, "thanks!", 10);
    match child_replies {
        ChatSayOutcome::Delivered { recipients, .. } => {
            assert!(recipients.is_empty(), "kenning someone must not grant a speak-back channel");
        }
        other => panic!("expected Delivered (with no recipients), got {other:?}"),
    }
}

#[test]
fn unverified_sender_is_refused_and_delivers_to_nobody() {
    let mut host = two_player_host();
    // Player 0 never got a verified_pubkey (guest / not signed in).
    verify(&mut host, 1, 0x02);

    let outcome = handle_chat_say(&mut host.server.players, 0, "let me in", 0);
    assert_eq!(outcome, ChatSayOutcome::NoVerifiedKey);
}

#[test]
fn overlong_line_is_rejected_and_delivers_to_nobody() {
    let mut host = two_player_host();
    verify(&mut host, 0, 0x01);
    verify(&mut host, 1, 0x02);

    let too_long = "a".repeat(crate::comms::MAX_CHAT_TEXT_LEN + 1);
    let outcome = handle_chat_say(&mut host.server.players, 0, &too_long, 0);
    assert_eq!(
        outcome,
        ChatSayOutcome::Rejected(crate::comms::ChatReject::TooLong)
    );
}

#[test]
fn thirty_first_line_in_a_minute_is_rate_limited_and_warns_once() {
    let mut host = two_player_host();
    verify(&mut host, 0, 0x01);
    verify(&mut host, 1, 0x02);

    for i in 0..crate::comms::CHAT_RATE_PER_MIN {
        let outcome = handle_chat_say(&mut host.server.players, 0, "spam", 0);
        assert!(
            matches!(outcome, ChatSayOutcome::Delivered { .. }),
            "line {i} within the allowance was refused"
        );
    }
    // The 31st line in the same instant is over the bucket.
    let refused = handle_chat_say(&mut host.server.players, 0, "spam", 0);
    assert_eq!(refused, ChatSayOutcome::RateLimited { warn: true });

    // A further line in the same run of refusals warns only once.
    let refused_again = handle_chat_say(&mut host.server.players, 0, "spam", 0);
    assert_eq!(refused_again, ChatSayOutcome::RateLimited { warn: false });
}

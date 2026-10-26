//! Tests for `launch_links.rs`: a tapped link is taken once, held until the menu can take it, and
//! routed as a paste is. No JNI: the intent reader is a closure.

use super::*;

const HOST: [u8; 32] = [0xab; 32];
const SECRET: [u8; 32] = [0x01; 32];

fn keep() -> String {
    fray_keep::format_invite_link(crate::keeps::LINK_PREFIX, &HOST, &SECRET)
}

fn link(text: &str) -> IntentRead {
    IntentRead::Link { text: text.to_string(), consumed: true }
}

/// The instant of the `n`th poll, a poll interval apart.
fn at(start: Instant, n: u32) -> Instant {
    start + LINK_POLL * n
}

fn hold(text: &str) -> LaunchLinks {
    let mut links = LaunchLinks::new();
    links.poll(Instant::now(), || link(text));
    links
}

fn is_join_game(action: &MenuAction, text: &str) -> bool {
    matches!(action, MenuAction::JoinGame(t) if t == text)
}

#[test]
fn a_link_is_taken_once() {
    let start = Instant::now();
    let mut links = LaunchLinks::new();
    let text = keep();
    links.poll(start, || link(&text));
    assert!(links.has_held());
    // The intent is consumed, so the next read finds nothing: still one held, not two.
    links.poll(at(start, 1), || IntentRead::NoLink);
    let mut dialog = MenuDialog::None;
    assert!(is_join_game(&links.offer(&mut dialog, true), &text));
    assert!(!links.has_held());
    assert!(matches!(links.offer(&mut dialog, true), MenuAction::None));
}

#[test]
fn the_intent_is_read_at_the_poll_cadence_not_every_frame() {
    let start = Instant::now();
    let mut links = LaunchLinks::new();
    let mut reads = 0;
    for frame in 0..10u32 {
        // Frames 10 ms apart: only the first falls due inside one interval.
        links.poll(start + Duration::from_millis(10 * u64::from(frame)), || {
            reads += 1;
            IntentRead::NoLink
        });
    }
    assert_eq!(reads, 1);
    links.poll(at(start, 1), || {
        reads += 1;
        IntentRead::NoLink
    });
    assert_eq!(reads, 2);
}

#[test]
fn a_link_that_could_not_be_consumed_is_not_taken_again_while_it_stays() {
    let start = Instant::now();
    let mut links = LaunchLinks::new();
    let text = keep();
    let stuck = || IntentRead::Link { text: text.clone(), consumed: false };
    links.poll(start, stuck);
    links.poll(at(start, 1), stuck);
    links.poll(at(start, 2), stuck);
    let mut dialog = MenuDialog::None;
    assert!(is_join_game(&links.offer(&mut dialog, true), &text));
    assert!(!links.has_held(), "one link, however many reads saw it");
    // Once the intent is clear, the same link tapped again is new.
    links.poll(at(start, 3), || IntentRead::NoLink);
    links.poll(at(start, 4), stuck);
    assert!(links.has_held());
}

#[test]
fn nothing_is_held_when_the_intent_cannot_be_read() {
    let mut links = LaunchLinks::new();
    links.poll(Instant::now(), || IntentRead::Unavailable);
    assert!(!links.has_held());
}

#[test]
fn a_link_is_held_until_the_player_is_at_the_menu() {
    let text = keep();
    let mut links = hold(&text);
    let mut dialog = MenuDialog::None;
    // In a world, paused, loading, on the splash, or with a join in flight: `idle` is false.
    for _ in 0..5 {
        assert!(matches!(links.offer(&mut dialog, false), MenuAction::None));
        assert!(links.has_held(), "held, not dropped");
    }
    assert!(matches!(dialog, MenuDialog::None), "the dialog is untouched");
    // Back at the menu it is handled, once.
    assert!(is_join_game(&links.offer(&mut dialog, true), &text));
    assert!(!links.has_held());
}

#[test]
fn a_link_waits_while_another_dialog_is_open() {
    let text = keep();
    let mut links = hold(&text);
    let busy = [
        MenuDialog::ResetWorkshop,
        MenuDialog::SignIn { paste_uri: String::new(), show_paste: false },
        MenuDialog::Delete { world_idx: 0, confirm_text: String::new() },
        MenuDialog::JoinKeep { invite: crate::keeps::Invite { host_key: HOST, secret: SECRET } },
    ];
    for mut dialog in busy {
        assert!(matches!(links.offer(&mut dialog, true), MenuAction::None));
        assert!(links.has_held());
    }
    // The paste boxes give way to it.
    for mut dialog in [
        MenuDialog::None,
        MenuDialog::JoinDirect { address: "half typed".into() },
        MenuDialog::AddFriend { input: String::new(), error: None },
    ] {
        let mut links = hold(&text);
        assert!(is_join_game(&links.offer(&mut dialog, true), &text));
    }
}

#[test]
fn held_links_come_out_in_order_one_per_offer() {
    let start = Instant::now();
    let mut links = LaunchLinks::new();
    let first = keep();
    let second = fray_keep::format_invite_link(crate::keeps::LINK_PREFIX, &[0xcd; 32], &SECRET);
    links.poll(start, || link(&first));
    links.poll(at(start, 1), || link(&second));
    // The same link tapped twice is held once, and moves behind the other.
    links.poll(at(start, 2), || link(&first));
    let mut dialog = MenuDialog::None;
    assert!(is_join_game(&links.offer(&mut dialog, true), &second));
    assert!(is_join_game(&links.offer(&mut dialog, true), &first));
    assert!(!links.has_held());
}

#[test]
fn at_most_four_are_held_and_the_oldest_goes() {
    let start = Instant::now();
    let mut links = LaunchLinks::new();
    let keys: Vec<String> = (1..=6u8).map(|i| fray_keep::format_invite_link(crate::keeps::LINK_PREFIX, &[i; 32], &SECRET)).collect();
    for (i, k) in keys.iter().enumerate() {
        links.poll(at(start, i as u32), || link(k));
    }
    let mut dialog = MenuDialog::None;
    for k in &keys[2..] {
        assert!(is_join_game(&links.offer(&mut dialog, true), k));
    }
    assert!(!links.has_held());
}

#[test]
fn a_link_that_is_not_ours_or_is_far_too_long_is_not_held() {
    for text in ["https://example.org/keep", "stampede://keep/x/y", "ws://host:1", "", "axenstax"] {
        assert!(!hold(text).has_held(), "{text:?}");
    }
    let long = format!("axenstax://keep/{}", "a".repeat(MAX_LINK_CHARS));
    assert!(!hold(&long).has_held());
}

#[test]
fn a_tapped_link_is_routed_as_a_paste_is() {
    let keep_link = keep();
    // A Keep link first, alone, in capitals, or cut short (Join Game's own message says so).
    assert_eq!(route(&keep_link), Some(LaunchRoute::Keep));
    assert_eq!(route(&keep_link.to_uppercase()), Some(LaunchRoute::Keep));
    assert_eq!(route("axenstax://keep/ab"), Some(LaunchRoute::Keep));
    assert_eq!(route("axenstax://keep"), Some(LaunchRoute::Keep));
    // A friend's invite, then the server strings.
    assert_eq!(route("axenstax://invite/2?x=1"), Some(LaunchRoute::FriendInvite));
    assert_eq!(route("AXENSTAX://INVITE/2?x=1"), Some(LaunchRoute::FriendInvite));
    assert_eq!(route("axenstax://play.example.com:8080#op=npub1xyz"), Some(LaunchRoute::Server));
    assert_eq!(route("axenstax://npub1abc"), Some(LaunchRoute::Server));
    // A host that merely starts with `keep` is a server, as `keeps::is_keep_text` reads it.
    assert_eq!(route("axenstax://keeper.example.com:8080"), Some(LaunchRoute::Server));
    assert_eq!(route("https://example.org"), None);
    assert_eq!(route("  axenstax://keep/ab  "), Some(LaunchRoute::Keep));
}

#[test]
fn a_keep_link_goes_to_join_games_router_and_the_others_open_their_paste_box() {
    // Keep: the action the game loop's Join Game arm takes, so signed-out, first-time confirm and
    // saved joins are `keeps::route_join`'s, unchanged.
    let text = keep();
    let mut dialog = MenuDialog::None;
    assert!(is_join_game(&hold(&text).offer(&mut dialog, true), &text));
    assert!(matches!(dialog, MenuDialog::None), "the arm, not the offer, opens the confirm");

    let invite = "axenstax://invite/2?x=1";
    let mut dialog = MenuDialog::None;
    assert!(matches!(hold(invite).offer(&mut dialog, true), MenuAction::None));
    assert!(matches!(&dialog, MenuDialog::AddFriend { input, error: None } if input == invite));

    let server = "axenstax://play.example.com:8080";
    let mut dialog = MenuDialog::JoinDirect { address: "old".into() };
    assert!(matches!(hold(server).offer(&mut dialog, true), MenuAction::None));
    assert!(matches!(&dialog, MenuDialog::JoinDirect { address } if address == server));
}

/// The same texts through the real router: a link handed over by `offer` is `route_join`'s.
#[test]
fn an_offered_keep_link_is_what_route_join_reads() {
    let text = keep();
    let mut dialog = MenuDialog::None;
    let MenuAction::JoinGame(offered) = hold(&text).offer(&mut dialog, true) else { panic!("a Keep link joins") };
    assert!(crate::keeps::is_keep_text(&offered));
    let unsaved = |_: &[u8; 32]| false;
    assert!(matches!(crate::keeps::route_join(&offered, false, unsaved), crate::keeps::JoinRoute::SignInFirst));
    assert!(matches!(crate::keeps::route_join(&offered, true, unsaved), crate::keeps::JoinRoute::Confirm(_)));
    assert!(matches!(crate::keeps::route_join(&offered, true, |_: &[u8; 32]| true), crate::keeps::JoinRoute::Join(_)));
}

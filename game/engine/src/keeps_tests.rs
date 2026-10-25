//! Tests for `keeps.rs`: how a Join Game text is routed, the pass rule, saving, and the
//! My Keeps block's small rules.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use super::*;
use crate::protocol::PacketType;
use crate::remote_client::RemoteClient;
use crate::transport::{ClientTransport, Packet, ProvenHost};

const HOST: [u8; 32] = [0xab; 32];
const SECRET: [u8; 32] = [0x01; 32];

fn link() -> String {
    fray_keep::format_invite_link(LINK_PREFIX, &HOST, &SECRET)
}

fn none_saved(_: &[u8; 32]) -> bool {
    false
}

/// A scratch directory removed when it goes out of scope.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("axenstax-keeps-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join(FILE)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pass_payload(host_key: [u8; 32], secret: [u8; 32]) -> Vec<u8> {
    let bytes = protocol::serialize_packet(PacketType::KeepPass, &KeepPass { host_key, secret });
    bytes[1..].to_vec()
}

// ─── Routing: every `axenstax://` form goes where it did, a Keep link to the Keep ──

#[test]
fn a_keep_link_is_routed_to_the_keep_and_nowhere_else() {
    let invite = Invite { host_key: HOST, secret: SECRET };
    assert_eq!(route_join(&link(), true, none_saved), JoinRoute::Confirm(invite));
    // Spaces around it, the prefix in any case, the hex in any case.
    assert_eq!(route_join(&format!("  {}\n", link()), true, none_saved), JoinRoute::Confirm(invite));
    assert_eq!(route_join(&link().to_uppercase(), true, none_saved), JoinRoute::Confirm(invite));
    // Inside a message, as people paste it.
    assert_eq!(route_join(&format!("Join my Keep: {}.", link()), true, none_saved), JoinRoute::Confirm(invite));
    assert_eq!(route_join(&format!("({})", link()), true, none_saved), JoinRoute::Confirm(invite));
}

#[test]
fn a_cut_short_or_damaged_keep_link_is_a_bad_link_not_a_server() {
    let good = link();
    for bad in [
        "axenstax://keep".to_string(),
        "axenstax://keep/".to_string(),
        "axenstax://keep/abc".to_string(),
        good[..good.len() - 1].to_string(),
        format!("{good}/extra"),
        format!("{}zz", good),
    ] {
        assert_eq!(route_join(&bad, true, none_saved), JoinRoute::BadLink, "{bad}");
        assert!(is_keep_text(&bad), "{bad}");
        assert!(!crate::fray_play::is_server_address(&bad), "a Keep link never reaches the WebSocket path: {bad}");
    }
}

#[test]
fn every_other_join_text_is_left_to_its_own_path() {
    let invite_link = "axenstax://invite/2?h=npub1abc&k=00&b=11&t=1&r=wss%3A%2F%2Frelay.example";
    let npub = "axenstax://npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
    for text in [
        invite_link,
        npub,
        "axenstax://play.example.com:8080",
        "axenstax://play.example.com:8080#op=npub1xyz",
        "AXENSTAX://play.example.com",
        "ws://192.0.2.5:6767",
        "wss://play.example.org/ws",
        "KMT RWX",
        "kmtrwx",
        "  KMT-RWX ",
        "",
        // A Keep link after another kind of address is part of that address's text, not a Keep link.
        &format!("ws://192.0.2.5:6767 {}", link()),
        &format!("axenstax://invite/2?x=1 {}", link()),
    ] {
        assert_eq!(route_join(text, true, none_saved), JoinRoute::NotKeep, "{text}");
        assert!(!is_keep_text(text), "{text}");
    }
    // They still read as they did: servers are servers, a code is not.
    assert!(crate::fray_play::is_server_address(npub));
    assert!(crate::fray_play::is_server_address(invite_link));
    assert!(crate::fray_play::is_server_address("axenstax://play.example.com:8080"));
    assert!(crate::fray_play::is_server_address("ws://192.0.2.5:6767"));
    assert!(crate::fray_play::is_server_address(" wss://play.example.org/ws"));
    assert!(!crate::fray_play::is_server_address("KMT RWX"));
    assert!(!crate::fray_play::is_server_address("kmtrwx"));
}

#[test]
fn a_keep_link_never_reaches_the_connect_string_or_the_friend_invite_parsers() {
    use crate::server_identity::connect::{parse_connect_string, ConnectInfo};
    let keep = link();
    let err = parse_connect_string(&keep).unwrap_err();
    assert!(err.contains("Keep"), "{err}");
    assert!(parse_connect_string("axenstax://keep").is_err());
    // The same parser, unchanged for the others.
    assert_eq!(
        parse_connect_string("axenstax://npub1xyz").unwrap(),
        ConnectInfo::Resolve { operator_npub: "npub1xyz".into() }
    );
    assert!(matches!(parse_connect_string("axenstax://play.example.com:8080").unwrap(), ConnectInfo::Direct { .. }));
    assert!(matches!(parse_connect_string("ws://10.0.0.5:8080").unwrap(), ConnectInfo::Direct { .. }));

    // Add a friend says where a Keep link belongs, and takes the friend invite as before.
    let said = crate::friends_ui::parse_add_friend(&keep, 0).unwrap_err();
    assert_eq!(said, NOT_A_FRIEND_LINK);
    assert_eq!(crate::friends_ui::parse_add_friend("axenstax://keep", 0).unwrap_err(), NOT_A_FRIEND_LINK);
    let now = 1_800_000_000;
    let friend_invite = crate::invite::Invite {
        host_persona: "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m".to_string(),
        host_runtime: nostr::Keys::parse(&format!("{:064x}", 1)).unwrap().public_key().to_bytes(),
        relays: vec!["wss://nos.lol".to_string()],
        bearer: [7u8; 16],
        expires_at: now + 3_600,
        world_name: "Ivy's Hollow".to_string(),
    };
    let text = friend_invite.to_link();
    assert!(!is_keep_text(&text));
    assert!(matches!(crate::friends_ui::parse_add_friend(&text, now), Ok(crate::friends_ui::AddFriendInput::Invite(_))));
    let npub = "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m";
    assert!(matches!(crate::friends_ui::parse_add_friend(npub, now), Ok(crate::friends_ui::AddFriendInput::Npub(_))));
}

// ─── Signed out, and the first-time confirm ──────────────────────────────────

#[test]
fn a_signed_out_player_is_told_to_sign_in_and_nothing_is_dialled() {
    // The router has no way to dial: the only routes that dial are Join and Confirm (which
    // asks first), and signed out is neither, whether or not the Keep is saved.
    for saved in [false, true] {
        let route = route_join(&link(), false, |_| saved);
        assert_eq!(route, JoinRoute::SignInFirst);
    }
    assert_eq!(SIGN_IN_FIRST, "Sign in first to join a Keep.");
    // A damaged link is still reported as damaged, signed in or not.
    assert_eq!(route_join("axenstax://keep/zz", false, none_saved), JoinRoute::BadLink);
}

#[test]
fn a_keep_not_in_my_keeps_asks_first_and_a_saved_one_does_not() {
    let invite = Invite { host_key: HOST, secret: SECRET };
    assert_eq!(route_join(&link(), true, none_saved), JoinRoute::Confirm(invite));
    assert_eq!(route_join(&link(), true, |key| *key == HOST), JoinRoute::Join(invite));
    // Another Keep being saved does not count.
    assert_eq!(route_join(&link(), true, |key| *key == [0x77; 32]), JoinRoute::Confirm(invite));
    assert_eq!(CONFIRM_TITLE, "Join this Keep?");
}

// ─── The return pass ────────────────────────────────────────────────────────

#[test]
fn a_pass_is_kept_only_for_the_proven_key() {
    let mut inbox = PassInbox::default();
    let pass = pass_payload(HOST, SECRET);
    assert_eq!(inbox.offer(Some(HOST), &pass), Offer::Kept);
    // None proven: dropped. Another key proven: dropped.
    assert_eq!(inbox.offer(None, &pass_payload([0xcd; 32], SECRET)), Offer::OtherHost);
    assert_eq!(inbox.offer(Some(HOST), &pass_payload([0xcd; 32], SECRET)), Offer::OtherHost);
    assert_eq!(inbox.offer(Some([0xcd; 32]), &pass), Offer::OtherHost);
    let taken = inbox.take();
    assert_eq!(taken.len(), 1);
    assert_eq!((taken[0].host_key, taken[0].secret), (HOST, SECRET));
    assert!(inbox.take().is_empty(), "taking leaves none");
}

#[test]
fn passes_are_capped_at_four_and_a_repeat_is_ignored() {
    let mut inbox = PassInbox::default();
    assert_eq!(inbox.offer(Some(HOST), &pass_payload(HOST, [0; 32])), Offer::Kept);
    assert_eq!(inbox.offer(Some(HOST), &pass_payload(HOST, [0; 32])), Offer::Repeated);
    for n in 1..4u8 {
        assert_eq!(inbox.offer(Some(HOST), &pass_payload(HOST, [n; 32])), Offer::Kept);
    }
    assert_eq!(inbox.offer(Some(HOST), &pass_payload(HOST, [9; 32])), Offer::Full);
    assert_eq!(inbox.offer(Some(HOST), &pass_payload(HOST, [3; 32])), Offer::Repeated, "a held one is still a repeat when full");
    assert_eq!(inbox.take().len(), MAX_HELD_PASSES);
    // A short or empty payload is not a pass.
    assert_eq!(inbox.offer(Some(HOST), &[1, 2, 3]), Offer::Unreadable);
    assert_eq!(inbox.offer(Some(HOST), &[]), Offer::Unreadable);
}

/// A transport that hands a `RemoteClient` the packets it is given and names the host it proved.
struct Proved {
    host: Option<ProvenHost>,
    incoming: Mutex<VecDeque<Packet>>,
}

impl ClientTransport for Proved {
    fn send_to_server(&self, _data: &[u8]) {}
    fn try_recv_from_server(&self) -> Option<Packet> {
        self.incoming.lock().unwrap().pop_front()
    }
    fn proven_host(&self) -> Option<ProvenHost> {
        self.host.clone()
    }
}

fn client(host: Option<ProvenHost>, packets: Vec<Packet>) -> RemoteClient {
    let transport = Proved { host, incoming: Mutex::new(packets.into()) };
    RemoteClient::from_transport(Box::new(transport), crate::remote_client::build_join_request_guest("Player", 0), None)
}

fn pass_packet(host_key: [u8; 32], secret: [u8; 32]) -> Packet {
    protocol::serialize_packet(PacketType::KeepPass, &KeepPass { host_key, secret })
}

#[test]
fn the_client_keeps_a_pass_from_the_host_its_link_proved_and_no_other() {
    let proved = ProvenHost { key: HOST, name: "Ada's Keep".into() };
    let mut rc = client(Some(proved.clone()), vec![pass_packet(HOST, SECRET), pass_packet([0xcd; 32], [7; 32]), pass_packet(HOST, SECRET)]);
    rc.poll();
    let passes = rc.take_keep_passes();
    assert_eq!(passes.len(), 1, "the other Keep's pass was dropped and the repeat ignored");
    assert_eq!((passes[0].host_key, passes[0].secret), (HOST, SECRET));
    assert!(rc.take_keep_passes().is_empty());
    assert_eq!(rc.proven_host(), Some(proved), "the proven key and name are the client's to read");

    // A link that proved no host believes no pass.
    let mut rc = client(None, vec![pass_packet(HOST, SECRET)]);
    rc.poll();
    assert!(rc.take_keep_passes().is_empty());
}

// ─── Saving ─────────────────────────────────────────────────────────────────

#[test]
fn a_pass_and_an_invite_join_are_each_saved_in_my_keeps_with_the_hosts_name() {
    let dir = Dir::new("save");
    let path = dir.file();
    assert!(load_list(&path).is_empty());

    let pass = KeepPass { host_key: HOST, secret: SECRET };
    assert_eq!(save_passes(&path, std::slice::from_ref(&pass), Some("Ada's Keep"), 1_000), 1);
    let list = load_list(&path);
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].name.as_str(), list[0].last_joined), ("Ada's Keep", 1_000));
    assert_eq!(list[0].keys(), Some((HOST, SECRET)));
    // The same pass again changes nothing.
    assert_eq!(save_passes(&path, std::slice::from_ref(&pass), Some("Ada's Keep"), 1_000), 0);
    assert_eq!(save_passes(&path, &[], Some("Ada's Keep"), 1_000), 0);

    // An invite join of another Keep is saved too, newest first, and a Keep with no name is named by its key.
    let other = Invite { host_key: [0xcd; 32], secret: [2; 32] };
    assert!(save_invite_join(&path, &other, None, 2_000));
    let list = load_list(&path);
    assert_eq!(list.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Keep cdcdcd", "Ada's Keep"]);

    // Forgetting one leaves the other; forgetting what is not there says so.
    assert!(forget(&path, &HOST));
    assert!(!forget(&path, &HOST));
    assert_eq!(load_list(&path).len(), 1);
}

#[cfg(unix)]
#[test]
fn the_saved_keeps_file_is_private_and_a_broken_one_is_never_saved_over() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Dir::new("mode");
    let path = dir.file();
    let invite = Invite { host_key: HOST, secret: SECRET };
    assert!(save_invite_join(&path, &invite, Some("Ada's Keep"), 5));
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "the secrets are the way in");

    // A file that cannot be read (here a directory where the file should be) is left alone.
    let blocked = Dir::new("blocked");
    std::fs::create_dir(blocked.file()).unwrap();
    assert!(!save_invite_join(&blocked.file(), &invite, None, 5));
    assert!(blocked.file().is_dir());
}

#[test]
fn a_seated_invite_join_is_saved_once_and_a_failed_one_is_not() {
    let dir = Dir::new("settle");
    let path = dir.file();
    let invite = Invite { host_key: HOST, secret: SECRET };
    let proved = ProvenHost { key: HOST, name: "Ada's Keep".into() };
    let seated = || crate::remote_client::ConnectionState::Connected { player_index: 1, seed: 0 };

    // Still joining: nothing saved, still pending.
    let mut rc = client(Some(proved.clone()), vec![]);
    let mut pending = Some(invite);
    settle(&mut rc, &mut pending, &path, 100);
    assert!(pending.is_some() && load_list(&path).is_empty());

    // Seated: saved with the host's name, no longer pending.
    rc.state = seated();
    settle(&mut rc, &mut pending, &path, 200);
    assert!(pending.is_none());
    let list = load_list(&path);
    assert_eq!((list.len(), list[0].name.as_str(), list[0].last_joined), (1, "Ada's Keep", 200));

    // A join that failed saves nothing and stops waiting.
    let other = Invite { host_key: [0xcd; 32], secret: [2; 32] };
    let mut rc = client(Some(ProvenHost { key: [0xcd; 32], name: "Other".into() }), vec![]);
    rc.state = crate::remote_client::ConnectionState::Failed("not let in".into());
    let mut pending = Some(other);
    settle(&mut rc, &mut pending, &path, 300);
    assert!(pending.is_none());
    assert_eq!(load_list(&path).len(), 1);

    // A seat on a host other than the Keep the link named is not saved under that Keep.
    let mut rc = client(Some(ProvenHost { key: [0xee; 32], name: "Impostor".into() }), vec![]);
    rc.state = seated();
    let mut pending = Some(other);
    settle(&mut rc, &mut pending, &path, 400);
    assert!(pending.is_none());
    assert_eq!(load_list(&path).len(), 1);
}

#[test]
fn a_pass_a_keep_sends_after_a_code_join_is_saved_with_no_invite_pending() {
    let dir = Dir::new("settle-pass");
    let path = dir.file();
    let mut rc = client(Some(ProvenHost { key: HOST, name: "Ada's Keep".into() }), vec![pass_packet(HOST, SECRET), pass_packet([0xcd; 32], [3; 32])]);
    rc.poll();
    let mut pending = None;
    settle(&mut rc, &mut pending, &path, 500);
    let list = load_list(&path);
    assert_eq!(list.len(), 1, "the other Keep's pass was never held");
    assert_eq!((list[0].name.as_str(), list[0].keys()), ("Ada's Keep", Some((HOST, SECRET))));
    // Nothing more to save the next frame.
    settle(&mut rc, &mut pending, &path, 600);
    assert_eq!(load_list(&path)[0].last_joined, 500);
}

// ─── The My Keeps block ─────────────────────────────────────────────────────

#[test]
fn the_rows_read_in_plain_words_newest_first() {
    let list = vec![Keep::new(&HOST, &SECRET, Some("Ada's Keep"), 10_000 - 300), Keep::new(&[0xcd; 32], &[2; 32], None, 10_000 - 7_300)];
    let rows = rows(&list, 10_000);
    assert_eq!(
        rows.iter().map(|r| (r.name.as_str(), r.when.as_str())).collect::<Vec<_>>(),
        [("Ada's Keep", "5 min ago"), ("Keep cdcdcd", "2 h ago")]
    );
    assert_eq!(rows[0].host_key, fray_keep::hex_encode(&HOST));
    assert_eq!(MY_KEEPS, "My Keeps");
    assert_eq!(FORGET_CONFIRM, "Forget this Keep? You'll need a new invite to return.");
}

#[test]
fn forget_needs_a_second_tap_on_the_same_keep() {
    let mut armed = None;
    assert!(!tap_forget(&mut armed, "aa"), "the first tap only arms it");
    assert_eq!(armed.as_deref(), Some("aa"));
    assert!(!tap_forget(&mut armed, "bb"), "a tap on another Keep arms that one instead");
    assert_eq!(armed.as_deref(), Some("bb"));
    assert!(tap_forget(&mut armed, "bb"), "the second tap on the same Keep forgets it");
    assert_eq!(armed, None);
    assert!(!tap_forget(&mut armed, "bb"), "and it has to be armed again");
}

// ─── Words ──────────────────────────────────────────────────────────────────

#[test]
fn a_keep_that_does_not_answer_or_will_not_let_us_in_is_one_plain_sentence() {
    for reason in [FailReason::NoAnswer, FailReason::NotAdmitted, FailReason::NoLinkAnswer] {
        assert_eq!(invite_failure_text(&reason, "Keep AB12CD"), UNREACHABLE, "{reason:?}");
    }
    // Anything else keeps its own words, with the Keep named.
    let said = invite_failure_text(&FailReason::VersionMismatch, "Keep AB12CD");
    assert!(said.starts_with("Couldn't join Keep AB12CD:") && said.contains("different version"), "{said}");
    assert!(!UNREACHABLE.contains("code"), "a Keep is never called a missing room code");
    assert_eq!(label(&HOST), "Keep ABABAB");
}

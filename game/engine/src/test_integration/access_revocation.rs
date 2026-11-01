//! A policy change reaches players already in the world.
//!
//! `HostedServer::set_access_policy` re-applies `access_policy::decide_access`
//! to every seated joiner, on every path that changes the policy, and a home
//! host's contacts sync goes through the same function. Local slots are never
//! touched. Joins ride the in-process channel transport (`join_guest`), and the
//! tests give the seat a verified key the way `joiner_authority` does.

use super::joiner_authority::{got_reject, join_guest, start_open_server};
use super::lent_world::{join_guest_lent, start_lent};
use crate::access_policy::Allow;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::transport::ChannelClientTransport;

const PLAYER: [u8; 32] = [1; 32];
const FRIEND: [u8; 32] = [2; 32];

/// A dedicated server: no local player, guests open until a policy is set.
fn start_dedicated(tag: &str) -> HostedServer {
    HostedServer::start(
        0,
        format!("access-revocation-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("hosted server starts")
}

/// Seat a joiner and give it the verified key `pk`.
fn seat(hs: &mut HostedServer, pk: [u8; 32]) -> (ChannelClientTransport, usize) {
    let (client, slot) = join_guest(hs, "Player");
    hs.server.players[slot].verified_pubkey = Some(pk);
    (client, slot)
}

#[test]
fn a_block_removes_a_seated_joiner_on_a_dedicated_server() {
    let mut hs = start_dedicated("block");
    let (client, slot) = seat(&mut hs, PLAYER);
    let (friend, friend_slot) = seat(&mut hs, FRIEND);

    hs.set_access_policy(true, Allow::Open, vec![PLAYER]);

    assert!(hs.slot_is_free(slot), "the blocked player is removed");
    assert!(got_reject(&client).is_some_and(|why| why.contains("blocked")), "and told why");
    assert!(!hs.slot_is_free(friend_slot), "nobody else is touched");
    assert!(got_reject(&friend).is_none());
}

#[test]
fn a_delisted_player_is_removed_and_a_listed_one_stays() {
    let mut hs = start_dedicated("delist");
    let (client, slot) = seat(&mut hs, PLAYER);
    let (_friend, friend_slot) = seat(&mut hs, FRIEND);

    hs.set_access_policy(true, Allow::only([FRIEND]), Vec::new());

    assert!(hs.slot_is_free(slot), "off the allowlist means out of the world");
    assert!(got_reject(&client).is_some_and(|why| why.contains("allowlist")));
    assert!(!hs.slot_is_free(friend_slot), "the listed player stays");
}

#[test]
fn requiring_sign_in_removes_a_seated_guest_and_keeps_a_signed_in_player() {
    let mut hs = start_dedicated("signin");
    let (guest, guest_slot) = join_guest(&mut hs, "Guest");
    let (_player, player_slot) = seat(&mut hs, PLAYER);

    hs.set_access_policy(true, Allow::Open, Vec::new());

    assert!(hs.slot_is_free(guest_slot), "a guest cannot stay once sign-in is required");
    assert!(got_reject(&guest).is_some_and(|why| why.contains("sign-in")));
    assert!(!hs.slot_is_free(player_slot), "a signed-in player stays");
}

#[test]
fn a_hosts_own_slot_is_never_removed() {
    let mut hs = start_open_server("access-host-slot");
    hs.server.players[0].verified_pubkey = Some(PLAYER);

    hs.set_access_policy(true, Allow::only([FRIEND]), vec![PLAYER]);

    assert!(!hs.slot_is_free(0), "a local player is not subject to the join policy");
}

#[test]
fn an_unchanged_policy_touches_nobody() {
    let mut hs = start_dedicated("no-op");
    let (client, slot) = seat(&mut hs, FRIEND);
    hs.set_access_policy(true, Allow::Open, vec![PLAYER]);
    assert!(!hs.slot_is_free(slot));

    // Only a real change re-runs the decision: the 5-second reload stays cheap.
    hs.server.players[slot].verified_pubkey = Some(PLAYER);
    hs.set_access_policy(true, Allow::Open, vec![PLAYER]);

    assert!(!hs.slot_is_free(slot), "the same policy again is a no-op");
    assert!(got_reject(&client).is_none());
}

#[test]
fn turning_an_allowlist_on_with_nobody_on_it_removes_everyone_seated() {
    let mut hs = start_dedicated("nobody");
    let (client, slot) = seat(&mut hs, PLAYER);
    hs.set_access_policy(true, Allow::only([]), Vec::new());
    assert!(hs.slot_is_free(slot));
    assert!(got_reject(&client).is_some_and(|why| why.contains("allowlist")));
}

#[test]
fn a_lending_host_removes_the_player_inside_its_next_lent_tick() {
    let (mut hs, mut host) = start_lent("access");
    let (client, slot) = join_guest_lent(&mut hs, &mut host, "Player");
    hs.server.players[slot].verified_pubkey = Some(PLAYER);

    hs.set_access_policy(true, Allow::Open, vec![PLAYER]);
    assert!(!hs.slot_is_free(slot), "the world is not lent here: the release waits for it");

    host.lend_tick(&mut hs);
    assert!(hs.slot_is_free(slot), "removed on the lent world");
    assert!(got_reject(&client).is_some_and(|why| why.contains("blocked")));
}

/// A contact removed from the book, on a host that admits over a direct socket.
#[test]
fn a_contact_removed_from_the_book_is_removed_from_a_home_hosts_world() {
    use crate::comms::Tier;
    use crate::online_host::tests::{a_contact, a_host};

    let hub = crate::rendezvous::relay_client::FakeRelayHub::new();
    let book = vec![a_contact(PLAYER, Tier::Kith), a_contact(FRIEND, Tier::Kith)];
    let (mut online, _) = a_host(&hub, book.clone());
    let mut hs = start_open_server("access-book");
    let (client, slot) = seat(&mut hs, PLAYER);
    let (_friend, friend_slot) = seat(&mut hs, FRIEND);
    crate::game_loop::push_book_to_online_host(Some(&mut online), Some(&mut hs), &book, &[]);
    assert!(!hs.slot_is_free(slot) && !hs.slot_is_free(friend_slot), "both are in the book");

    let fresh = vec![a_contact(FRIEND, Tier::Kith)];
    crate::game_loop::push_book_to_online_host(Some(&mut online), Some(&mut hs), &fresh, &[]);

    assert!(hs.slot_is_free(slot), "a removed contact leaves the world");
    assert!(got_reject(&client).is_some());
    assert!(!hs.slot_is_free(friend_slot), "a remaining contact stays");
}

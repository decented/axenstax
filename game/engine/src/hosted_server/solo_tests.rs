//! D3 — the local-only authority `--solo-hosted` runs through
//! ([`RemoteTransport::None`], [`HostedServer::start_solo`]): no network
//! effect, observed on the server's own state, and seats that follow the
//! client's split-screen seats without an unread channel.

use super::*;

fn solo(tag: &str) -> (crate::save::WorldsRootGuard, HostedServer) {
    let guard = crate::save::WorldsRootGuard::new(&format!("solo-{tag}"));
    let hs = HostedServer::start_solo(format!("solo-{tag}"), 42).expect("a local-only server starts");
    (guard, hs)
}

#[test]
fn a_local_only_server_binds_listens_and_announces_nothing() {
    let (_g, hs) = solo("silent");
    assert!(hs.local_only());
    assert!(hs.lends_host_world(), "solo-hosted lends the client's one world");
    assert_eq!(hs.port, 0, "nothing listens");
    assert_eq!(hs.max_remote_players, 0);
    assert!(hs.broadcaster.is_none(), "no LAN broadcaster");
    assert!(hs.remote_rx.is_none(), "no accept channel");
    assert!(hs.quic_thread.is_none(), "no accept thread or runtime");
    assert!(hs.fray_host.is_none(), "no Fray room");
    assert!(hs.world_room.is_none(), "no world room");
    assert!(!hs.require_signin);
    assert!(!hs.keep_world);
    assert!(hs.identity.is_none());
    assert_eq!(hs.current_remote.load(Ordering::Relaxed), 0);
    assert!(!hs.has_remote_clients());
    assert_eq!(hs.local_transports.len(), 1, "one loopback: seat 0's");
    assert_eq!(hs.num_local_players, 1);
    // No identity, Charter or contacts read for a server nobody can join.
    let sp = &hs.server.players[0];
    assert!(sp.verified_pubkey.is_none());
    assert!(sp.contacts.is_empty());
    // A host client exists: its machines are its own (D3 changes no owner row).
    assert!(!hs.server.simulates_block_machines);
    assert!(hs.server.animal_life_simulated);
    assert!(hs.server.column_streamer.is_none());
    assert_eq!(hs.server.column_refill_per_tick, 0);
}

#[test]
fn a_local_only_server_refuses_remote_capacity_and_a_socket() {
    let _g = crate::save::WorldsRootGuard::new("solo-refuse");
    let capacity = HostedServer::start_inner(
        1,
        "solo-refuse".into(),
        42,
        1,
        RemoteTransport::None,
        None,
        HostWorld::Lent,
    );
    assert!(capacity.is_err(), "remote capacity on a local-only server");
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind a test socket");
    let prebound = HostedServer::start_inner(
        1,
        "solo-refuse".into(),
        42,
        0,
        RemoteTransport::None,
        Some(socket),
        HostWorld::Lent,
    );
    assert!(prebound.is_err(), "a pre-bound socket on a local-only server");
    let no_seat =
        HostedServer::start_inner(0, "solo-refuse".into(), 42, 0, RemoteTransport::None, None, HostWorld::Owned);
    assert!(no_seat.is_err(), "a local-only server with no seat");
}

#[test]
fn split_screen_seats_grow_without_a_loopback_and_keep_their_indices() {
    let (_g, mut hs) = solo("seats");
    let at = |x: f32| (glam::Vec3::new(x, 70.0, 0.5), 0.0, 0.0, 20.0);
    hs.sync_local_slots(&[at(0.5), at(2.5), at(4.5), at(6.5)]);
    assert_eq!(hs.num_local_players, 4);
    assert_eq!(hs.server.players.len(), 4);
    assert_eq!(hs.transports.len(), 4);
    assert_eq!(hs.local_transports.len(), 1, "seats 2-4 have no unread channel");
    assert_eq!(hs.server.players[3].player.pos.x, 6.5);
    hs.sync_local_seats(&[false, true, true, false]);
    assert!(hs.server.players[1].local_seated && hs.server.players[2].local_seated);

    // Seats 3 and 4 leave: their slots stay (indices are stable) but are out
    // of the world, and their ride state is cleared.
    hs.sync_local_slots(&[at(0.5), at(2.5)]);
    hs.sync_local_seats(&[false, true]);
    assert_eq!(hs.server.players.len(), 4, "slot indices are kept");
    assert!(hs.server.players[1].connected);
    assert!(!hs.server.players[2].connected && !hs.server.players[3].connected);
    assert!(hs.disconnected[2] && hs.disconnected[3]);
    assert!(!hs.server.players[2].local_seated, "a departed seat rides nothing");

    // A seat fills slot 3 again.
    hs.sync_local_slots(&[at(0.5), at(2.5), at(8.5)]);
    assert_eq!(hs.server.players.len(), 4);
    assert!(hs.server.players[2].connected);
    assert_eq!(hs.server.players[2].player.pos.x, 8.5);
    assert!(!hs.has_remote_clients());
}

//! "Host Game" over Fray, end to end on this machine: a `HostedServer` started
//! through `start_fray_host_with`, and a joiner that reaches it by the LAN
//! beacon or by room code (over Fray's in-process relay), then goes through the
//! game's own Signet challenge, signed over the Fray connection's binding, to a
//! joined player. The guest and wrong-origin cases pin that a Fray join is
//! admitted no more loosely than a LAN QUIC join was.

use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use decented_net::relay::FakeRelayHub;
use decented_net::{host, host_with_relay, join_invite_with_relay, join_lan, join_with_relay, lan_browse, Identity, Keys};

use super::*;
use crate::hosted_server::{FrayStart, HostWorld, HostedServer};
use crate::remote_client::{build_join_request_guest, ConnectionState, RemoteClient, SignDriverFn, SignedJoin};
use crate::transport::ClientTransport;

const WAIT: Duration = Duration::from_secs(20);

/// AxeNStax's Fray settings on loopback: no relays or STUN, and a private
/// beacon port per test so the tests never hear each other.
fn test_cfg() -> NetConfig {
    let mut r = [0u8; 2];
    getrandom::fill(&mut r).expect("OS RNG");
    let mut cfg = net_config(std::env::temp_dir()).lan_only();
    cfg.lan_port = 30_000 + (u16::from_le_bytes(r) % 20_000);
    cfg
}

/// A world name nothing on disk has, so the owning server generates fresh
/// terrain and writes nothing.
fn world(tag: &str) -> String {
    let mut r = [0u8; 4];
    getrandom::fill(&mut r).expect("OS RNG");
    format!("fray-b2-{tag}-{}-{:08x}", std::process::id(), u32::from_le_bytes(r))
}

fn start_host(open_room: impl FnOnce(decented_net::RoomInfo) -> decented_net::HostHandle) -> HostedServer {
    start_host_blocking(Vec::new(), open_room)
}

/// A host whose player has blocked `blocked` in Signet, listening.
fn start_host_blocking(
    blocked: Vec<[u8; 32]>,
    open_room: impl FnOnce(decented_net::RoomInfo) -> decented_net::HostHandle,
) -> HostedServer {
    let mut server = start_unsettled(blocked, open_room);
    wait_ready(&mut server);
    server
}

/// The room as `start_fray_host_with` returns it: possibly still opening.
fn start_unsettled(
    blocked: Vec<[u8; 32]>,
    open_room: impl FnOnce(decented_net::RoomInfo) -> decented_net::HostHandle,
) -> HostedServer {
    let Ok(server) = HostedServer::start_fray_host_with(1, world("host"), 7, 4, HostWorld::Owned, blocked, open_room) else {
        panic!("the Fray room starts");
    };
    server
}

/// Poll `server`'s room start until it is `Ready`.
fn wait_ready(server: &mut HostedServer) {
    let deadline = Instant::now() + WAIT;
    loop {
        match server.poll_fray_start() {
            FrayStart::Ready => return,
            FrayStart::Pending => {}
            FrayStart::Failed(why) => panic!("the room failed to start: {why}"),
        }
        assert!(Instant::now() < deadline, "the room never reported ready");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Dial `server`'s room straight at its LAN address and token.
fn dial_lan(server: &mut HostedServer, cfg: &NetConfig) -> FrayClientTransport {
    let handle = server.fray_room().unwrap().handle();
    let addr = handle.local_addr().expect("listening");
    let token = handle.lan_token();
    let dial = std::net::SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), addr.port());
    let join = PendingJoin::over(decented_net::join_addr(cfg, &Identity::ephemeral(), dial, &token), "dial".into());
    reach_host(server, join)
}

/// Tick the server while the join finds its host; the transport once it has.
fn reach_host(server: &mut HostedServer, mut join: PendingJoin) -> FrayClientTransport {
    let deadline = Instant::now() + WAIT;
    loop {
        server.tick();
        match join.poll() {
            JoinPoll::Waiting => {}
            JoinPoll::Connected(transport) => return transport,
            JoinPoll::Failed(why) => panic!("the join failed: {why}"),
        }
        assert!(Instant::now() < deadline, "the join never reached the host");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Tick both ends until the client's join is settled (joined or refused).
fn settle(server: &mut HostedServer, client: &mut RemoteClient) {
    let deadline = Instant::now() + WAIT;
    while matches!(client.state, ConnectionState::Connecting) {
        server.tick();
        client.poll();
        assert!(Instant::now() < deadline, "the join never settled");
        thread::sleep(Duration::from_millis(5));
    }
}

/// `ConnectionState` for a failure message (it is not `Debug`).
fn state_name(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Disconnected => "disconnected".into(),
        ConnectionState::Connecting => "connecting".into(),
        ConnectionState::Connected { .. } => "connected".into(),
        ConnectionState::Failed(why) => format!("failed: {why}"),
    }
}

/// Signs the challenge with `keys`, as the bunker would, over the origin the
/// client built (or over `sign_over` instead), and records that origin.
fn driver(keys: Keys, seen: Arc<Mutex<Option<String>>>, sign_over: Option<String>) -> SignDriverFn {
    Box::new(move |nonce: String, origin: String| {
        *seen.lock().unwrap() = Some(origin.clone());
        let over = sign_over.unwrap_or(origin);
        let signed = pollster::block_on(crate::signet::native_signer::build_join_auth_event(&keys, &nonce, &over))
            .map(|auth_event| SignedJoin { auth_event, credential: None })
            .map_err(|e| format!("{e:?}"));
        let (tx, rx) = mpsc::channel();
        tx.send(signed).unwrap();
        rx
    })
}

/// Join over `transport` with a signed challenge and check the host seated
/// the signer, having verified it over this connection's binding.
fn signed_join_is_seated(server: &mut HostedServer, transport: FrayClientTransport) {
    let binding = transport.channel_binding();
    assert!(binding.is_some(), "Fray gives the joiner the app's channel binding");
    let keys = Keys::generate();
    let seen = Arc::new(Mutex::new(None));
    let mut client = RemoteClient::connect_authed_on_transport(
        Box::new(transport),
        "Joiner",
        driver(keys.clone(), seen.clone(), None),
        None,
    );
    settle(server, &mut client);
    assert!(client.is_connected(), "joined: {}", state_name(&client.state));
    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some(crate::signet::join_origin(binding).as_str()),
        "the challenge is signed over the Fray connection's exporter"
    );
    let pubkey = keys.public_key().to_bytes();
    assert!(
        server.server.players.iter().any(|p| p.verified_pubkey == Some(pubkey)),
        "the host verified the joiner's key"
    );
}

#[test]
fn the_play_config_uses_the_players_relays_only() {
    let relays = vec!["wss://relay.example.org".to_string()];
    let cfg = play_config(&relays);
    assert_eq!(cfg.relays, relays);
    assert!(!cfg.relay_fallback, "never Fray's built-in relays (red line 2)");
    assert_eq!(cfg.app_tag, "axenstax");
    let stun: Vec<String> = crate::nat::stun::STUN_SERVERS.iter().map(|s| s.to_string()).collect();
    assert_eq!(cfg.stun, stun);
}

#[test]
fn the_code_is_published_lan_only_or_still_going_out() {
    let s = Duration::from_secs(1);
    assert_eq!(code_status((0, 3), s), CodeStatus::Publishing);
    assert_eq!(code_status((2, 3), s), CodeStatus::Published { up: 2, total: 3 });
    assert_eq!(code_status((0, 0), s), CodeStatus::LanOnly { tried: 0 });
    assert_eq!(code_status((0, 3), PUBLISH_GRACE), CodeStatus::LanOnly { tried: 3 });
    // A relay that comes up late still publishes it.
    assert_eq!(code_status((1, 3), PUBLISH_GRACE * 4), CodeStatus::Published { up: 1, total: 3 });
}

#[test]
fn only_server_addresses_skip_the_code_join() {
    assert!(is_server_address("ws://192.0.2.2:6767"));
    assert!(is_server_address(" wss://play.example.org/ws"));
    assert!(is_server_address("axenstax://npub1xyz"));
    assert!(!is_server_address("KMT RWX"));
    assert!(!is_server_address("192.0.2.10:7700"));
}

#[test]
fn a_room_from_the_lan_list_joins_through_the_signet_challenge() {
    let cfg = test_cfg();
    let mut server = start_host(|room| host(&cfg, &Identity::ephemeral(), room));
    let code = server.fray_room().expect("a Fray room").handle().code().to_string();
    let mut browser = lan_browse(&cfg);
    let deadline = Instant::now() + WAIT;
    let room = loop {
        if let Some(room) = browser.poll().into_iter().find(|r| r.code == code) {
            break room;
        }
        assert!(Instant::now() < deadline, "the room's beacon was never heard");
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(room.max_players, 5, "the host and four joiners");
    assert_eq!(room.name, ROOM_NAME, "the beacon names no world");
    let join = PendingJoin::over(join_lan(&cfg, &Identity::ephemeral(), &room), room.name.clone());
    let transport = reach_host(&mut server, join);
    signed_join_is_seated(&mut server, transport);
}

#[test]
fn a_room_code_joins_through_the_signet_challenge() {
    let hub = FakeRelayHub::new();
    let cfg = test_cfg();
    let mut server = start_host(|room| host_with_relay(&cfg, &Identity::ephemeral(), room, Some(Box::new(hub.client()))));
    let code = server.fray_room().expect("a Fray room").handle().code().to_string();
    let typed = decented_net::room::display_code(&code).to_lowercase();
    let join = PendingJoin::over(join_with_relay(&cfg, &Identity::ephemeral(), &typed, Box::new(hub.client())), typed);
    let transport = reach_host(&mut server, join);
    signed_join_is_seated(&mut server, transport);
}

/// A code holder learns nothing that names the world before the Signet
/// challenge: the room Fray is asked to open, and the name it sends a joiner on
/// connecting, are the neutral one, not the world's folder name.
#[test]
fn a_code_holder_is_told_no_world_name_before_the_challenge() {
    let hub = FakeRelayHub::new();
    let cfg = test_cfg();
    let announced = Arc::new(Mutex::new(None));
    let seen = announced.clone();
    let server = start_host(|room| {
        *seen.lock().unwrap() = Some(room.name.clone());
        host_with_relay(&cfg, &Identity::ephemeral(), room, Some(Box::new(hub.client())))
    });
    let folder = server.server_name.clone();
    assert!(folder.starts_with("fray-b2-"), "the game still knows the world as {folder}");
    assert_eq!(announced.lock().unwrap().as_deref(), Some(ROOM_NAME));

    let mut server = server;
    let code = server.fray_room().expect("a Fray room").handle().code().to_string();
    let typed = decented_net::room::display_code(&code);
    let handle = join_with_relay(&cfg, &Identity::ephemeral(), &typed, Box::new(hub.client()));
    let deadline = Instant::now() + WAIT;
    let told = loop {
        server.tick();
        match handle.poll() {
            Some(ClientEvent::Connected { host_name, .. }) => break host_name,
            Some(ClientEvent::Failed { reason, .. }) => panic!("the join failed: {reason}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "the join never connected");
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(told, ROOM_NAME);
    assert!(!told.contains(&folder), "the world's name must not reach a code holder: {told}");
}

#[test]
fn a_wrong_code_fails_in_the_lobby_with_fray_s_reason() {
    let hub = FakeRelayHub::new();
    let cfg = test_cfg();
    let mut join = PendingJoin::over(join_with_relay(&cfg, &Identity::ephemeral(), "KMTRW1", Box::new(hub.client())), "KMTRW1".into());
    let deadline = Instant::now() + WAIT;
    let why = loop {
        match join.poll() {
            JoinPoll::Failed(why) => break why,
            JoinPoll::Connected(_) => panic!("a bad code cannot connect"),
            JoinPoll::Waiting => {}
        }
        assert!(Instant::now() < deadline, "a bad code never failed");
        thread::sleep(Duration::from_millis(5));
    };
    assert!(why.starts_with("Couldn't join KMTRW1"), "{why}");
    assert!(why.contains("isn't a valid room code"), "{why}");
}

/// Decision 3: a Fray room requires sign-in, as the LAN QUIC host did.
#[test]
fn a_guest_is_refused_by_a_fray_room() {
    let cfg = test_cfg();
    let mut server = start_host(|room| host(&cfg, &Identity::ephemeral(), room));
    let addr = server.fray_room().unwrap().handle().local_addr().expect("listening");
    let token = server.fray_room().unwrap().handle().lan_token();
    let dial = std::net::SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), addr.port());
    let join = PendingJoin::over(decented_net::join_addr(&cfg, &Identity::ephemeral(), dial, &token), "guest".into());
    let transport = reach_host(&mut server, join);
    let mut client = RemoteClient::from_transport(Box::new(transport), build_join_request_guest("Guest", 0), None);
    settle(&mut server, &mut client);
    let ConnectionState::Failed(why) = &client.state else {
        panic!("a guest must be refused: {}", state_name(&client.state));
    };
    assert!(why.contains("sign-in"), "{why}");
}

/// The binding is load-bearing: a challenge signed over another origin (here
/// the unbound one) is refused.
#[test]
fn a_join_signed_over_another_origin_is_refused() {
    let cfg = test_cfg();
    let mut server = start_host(|room| host(&cfg, &Identity::ephemeral(), room));
    let addr = server.fray_room().unwrap().handle().local_addr().expect("listening");
    let token = server.fray_room().unwrap().handle().lan_token();
    let dial = std::net::SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), addr.port());
    let join = PendingJoin::over(decented_net::join_addr(&cfg, &Identity::ephemeral(), dial, &token), "relayed".into());
    let transport = reach_host(&mut server, join);
    let seen = Arc::new(Mutex::new(None));
    let mut client = RemoteClient::connect_authed_on_transport(
        Box::new(transport),
        "Joiner",
        driver(Keys::generate(), seen, Some(crate::signet::join_origin(None))),
        None,
    );
    settle(&mut server, &mut client);
    assert!(matches!(client.state, ConnectionState::Failed(_)), "{}", state_name(&client.state));
    assert!(
        server.server.players.iter().skip(1).all(|p| p.verified_pubkey.is_none()),
        "no joiner was verified"
    );
}

/// S3: a join signed over one connection's binding, presented on another, is
/// refused: the origin is the connection's own exporter, so a signature cannot
/// be carried from one link to the next.
#[test]
fn a_join_signed_over_another_connections_binding_is_refused() {
    let cfg = test_cfg();
    let mut server = start_host(|room| host(&cfg, &Identity::ephemeral(), room));
    let first = dial_lan(&mut server, &cfg);
    let first_binding = first.channel_binding();
    signed_join_is_seated(&mut server, first);

    let second = dial_lan(&mut server, &cfg);
    let second_binding = second.channel_binding();
    assert!(first_binding.is_some() && second_binding.is_some());
    assert_ne!(first_binding, second_binding, "each Fray connection has its own binding");
    let replayer = Keys::generate();
    let mut client = RemoteClient::connect_authed_on_transport(
        Box::new(second),
        "Replayer",
        driver(replayer.clone(), Arc::new(Mutex::new(None)), Some(crate::signet::join_origin(first_binding))),
        None,
    );
    settle(&mut server, &mut client);
    assert!(matches!(client.state, ConnectionState::Failed(_)), "{}", state_name(&client.state));
    let replayer_key = replayer.public_key().to_bytes();
    assert!(
        server.server.players.iter().all(|p| p.verified_pubkey != Some(replayer_key)),
        "the replayed join was never seated"
    );

    // The same link still admits a join signed over its own binding.
    let third = dial_lan(&mut server, &cfg);
    signed_join_is_seated(&mut server, third);
}

/// M1: the player's Signet blocklist applies to Host Game as it does to the
/// online host: a blocked key that signs a valid challenge is refused, over a
/// room code or a LAN join, and an unblocked one is seated. Sign-in stays
/// required.
#[test]
fn a_blocked_key_is_refused_by_a_fray_room_and_an_unblocked_one_is_seated() {
    let blocked = Keys::generate();
    let hub = FakeRelayHub::new();
    let cfg = test_cfg();
    let mut server = start_host_blocking(vec![blocked.public_key().to_bytes()], |room| {
        host_with_relay(&cfg, &Identity::ephemeral(), room, Some(Box::new(hub.client())))
    });
    let code = server.fray_room().expect("a Fray room").handle().code().to_string();
    let typed = decented_net::room::display_code(&code);

    // Over the room code.
    let join = PendingJoin::over(join_with_relay(&cfg, &Identity::ephemeral(), &typed, Box::new(hub.client())), typed.clone());
    let transport = reach_host(&mut server, join);
    let mut client = RemoteClient::connect_authed_on_transport(
        Box::new(transport),
        "Blocked",
        driver(blocked.clone(), Arc::new(Mutex::new(None)), None),
        None,
    );
    settle(&mut server, &mut client);
    let ConnectionState::Failed(why) = &client.state else {
        panic!("a blocked key must be refused: {}", state_name(&client.state));
    };
    assert!(why.contains("blocked"), "{why}");

    // Over the LAN address, with the same key.
    let transport = dial_lan(&mut server, &cfg);
    let mut client = RemoteClient::connect_authed_on_transport(
        Box::new(transport),
        "Blocked",
        driver(blocked.clone(), Arc::new(Mutex::new(None)), None),
        None,
    );
    settle(&mut server, &mut client);
    assert!(matches!(client.state, ConnectionState::Failed(_)), "{}", state_name(&client.state));
    let blocked_bytes = blocked.public_key().to_bytes();
    assert!(
        server.server.players.iter().all(|p| p.verified_pubkey != Some(blocked_bytes)),
        "the blocked key was never seated"
    );

    // Anyone else who signs in is still seated, by code.
    let join = PendingJoin::over(join_with_relay(&cfg, &Identity::ephemeral(), &typed, Box::new(hub.client())), typed);
    let transport = reach_host(&mut server, join);
    signed_join_is_seated(&mut server, transport);
}

/// S1: starting the room does not wait for it. It is `Pending` or `Ready`
/// straight after, and `poll_fray_start` settles it on a later frame.
#[test]
fn the_room_start_returns_at_once_and_settles_on_a_later_poll() {
    let cfg = test_cfg();
    // Time only what follows the room being opened: building the world is
    // slow in a debug build and is not the start's wait.
    let opened = Arc::new(Mutex::new(None));
    let mut server = start_unsettled(Vec::new(), {
        let (cfg, opened) = (cfg.clone(), opened.clone());
        move |room| {
            let handle = host(&cfg, &Identity::ephemeral(), room);
            *opened.lock().unwrap() = Some(Instant::now());
            handle
        }
    });
    let waited = opened.lock().unwrap().expect("the room was opened").elapsed();
    assert!(waited < Duration::from_secs(1), "the start waited {waited:?} for the room");
    assert!(server.fray_room().is_some(), "the room, and its code, exist at once");
    wait_ready(&mut server);
    assert_ne!(server.port, 0, "the listening port is noted once ready");
    assert_eq!(server.poll_fray_start(), FrayStart::Ready);
}

/// S1: a room that cannot start is reported by a later poll (not by the
/// start), and is closed.
#[test]
fn a_room_that_cannot_start_fails_on_a_later_poll() {
    let mut cfg = test_cfg();
    // A liveness pair Fray refuses.
    cfg.idle_timeout = Duration::from_secs(2);
    cfg.keep_alive = Duration::from_secs(3);
    let mut server = start_unsettled(Vec::new(), |room| host(&cfg, &Identity::ephemeral(), room));
    let deadline = Instant::now() + WAIT;
    let why = loop {
        match server.poll_fray_start() {
            FrayStart::Failed(why) => break why,
            FrayStart::Ready => panic!("a refused config cannot be ready"),
            FrayStart::Pending => {}
        }
        assert!(Instant::now() < deadline, "the failure never showed");
        thread::sleep(Duration::from_millis(5));
    };
    assert!(why.contains("keep_alive"), "{why}");
    assert!(server.fray_room().is_none(), "the failed room is closed");
    // The server carries on with nobody able to join.
    for _ in 0..5 {
        server.tick();
    }
}

/// A Keep join by invite carries the host's proof with it: the key the signed welcome was
/// made with is the one the invite named, and the connected client can read it (a return
/// pass is believed only from that key). A code join carries it too.
#[test]
fn an_invite_and_a_code_join_both_hand_the_client_the_hosts_proven_key_and_name() {
    let hub = FakeRelayHub::new();
    let cfg = test_cfg();
    let mut server = start_host(|room| host_with_relay(&cfg, &Identity::ephemeral(), room, Some(Box::new(hub.client()))));
    let secret = [0x5a; 32];
    let (host_key, code) = {
        let handle = server.fray_room().expect("a Fray room").handle();
        handle.add_invite(secret).expect("the room takes an invite");
        (handle.runtime_key().to_bytes(), handle.code().to_string())
    };

    let mut join = PendingJoin::over(
        join_invite_with_relay(&cfg, &Identity::ephemeral(), &secret, host_key, Box::new(hub.client())),
        crate::keeps::label(&host_key),
    );
    join.invite = true;
    let transport = reach_host(&mut server, join);
    let proven = transport.proven_host().expect("the invite join proved the host");
    assert_eq!(proven.key, host_key, "the key the invite named");
    assert_eq!(proven.name, ROOM_NAME, "the name Fray's welcome carries");
    let keys = Keys::generate();
    let mut client = RemoteClient::connect_authed_on_transport(
        Box::new(transport),
        "Joiner",
        driver(keys, Arc::new(Mutex::new(None)), None),
        None,
    );
    settle(&mut server, &mut client);
    assert!(client.is_connected(), "joined: {}", state_name(&client.state));
    assert_eq!(client.proven_host(), Some(proven.clone()), "the client holds what the link proved");

    // By room code the proof is the same, so a pass can arrive after a code join.
    let typed = decented_net::room::display_code(&code);
    let join = PendingJoin::over(join_with_relay(&cfg, &Identity::ephemeral(), &typed, Box::new(hub.client())), typed);
    let transport = reach_host(&mut server, join);
    assert_eq!(transport.proven_host(), Some(proven));
}

/// A Keep that turns an invite join away (a kick, before the join is seated) reads as the one
/// sentence a Keep that never answers does; every other join, and every other reason, keeps
/// its own words.
#[test]
fn a_kicked_invite_join_folds_into_the_one_unreachable_sentence() {
    let host_key = [0x5a; 32];
    let invite = PendingJoin { handle: None, label: crate::keeps::label(&host_key), invite: true };
    assert_eq!(invite.disconnected(DisconnectReason::Kicked), crate::keeps::UNREACHABLE);
    // Another reason on an invite join is not folded.
    assert_eq!(
        invite.disconnected(DisconnectReason::HostLeft),
        crate::lan_host::join_failure_notice("Keep 5a5a5a", "the host left")
    );
    // A code join that is kicked says so in its own words.
    let code = PendingJoin { handle: None, label: "KMT RWX".into(), invite: false };
    assert_eq!(code.disconnected(DisconnectReason::Kicked), crate::lan_host::join_failure_notice("KMT RWX", "the host turned us away"));
    assert_ne!(code.disconnected(DisconnectReason::Kicked), crate::keeps::UNREACHABLE);
}

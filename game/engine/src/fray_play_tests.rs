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
use decented_net::{host, host_with_relay, join_lan, join_with_relay, lan_browse, Identity, Keys};

use super::*;
use crate::hosted_server::{HostWorld, HostedServer};
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
    let Ok(server) = HostedServer::start_fray_host_with(1, world("host"), 7, 4, HostWorld::Owned, open_room) else {
        panic!("the Fray room starts");
    };
    server
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

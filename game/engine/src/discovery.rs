//! LAN discovery — find game servers on the local network via UDP broadcast.
//!
//! Server broadcasts a ServerAnnouncePacket on UDP port 7705 every second.
//! Client listens on that port and collects discovered servers.
//!
//! The broadcaster (`ServerBroadcaster`) runs for every QUIC-hosted world
//! (`hosted_server.rs`). The listener (`ServerListener`) feeds the "Games on
//! this network" list in the Join Game dialog (`lan_ui.rs`, gap-audit T2-10).
//! This is LAN broadcast only: there is no directory, relay or internet
//! discovery here, and there must never be one (red line 1). Anyone on the LAN
//! can broadcast, so every announcement is treated as untrusted text.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Discovery broadcast port.
pub const DISCOVERY_PORT: u16 = crate::protocol::DISCOVERY_PORT;

/// How often the server broadcasts.
const BROADCAST_INTERVAL: Duration = Duration::from_secs(1);

/// How long before a discovered server is considered stale.
const SERVER_TIMEOUT: Duration = Duration::from_secs(5);

/// Most servers one listener will remember — a flooding LAN cannot grow it.
const MAX_DISCOVERED: usize = 100;

/// Most datagrams read per `poll()`. `poll` runs every frame, so this bounds the
/// work (and memory) one frame can spend on a broadcast flood while still being
/// far above real traffic (one announcement per host per second); the rest wait
/// in the OS socket buffer or are dropped by it.
const MAX_PACKETS_PER_POLL: usize = 64;

/// Longest server name (in characters) kept from an announcement.
const MAX_SERVER_NAME_CHARS: usize = 64;

/// Information about a discovered server.
#[derive(Clone, Debug)]
pub struct DiscoveredServer {
    /// Server address (IP from the broadcast source, port from the announce packet)
    pub addr: SocketAddr,
    /// Server/world name
    pub server_name: String,
    /// Current player count
    pub player_count: u8,
    /// Max player count
    pub max_players: u8,
    /// Whether the server is in creative mode
    pub is_creative: bool,
    /// The announcing build's `PROTOCOL_VERSION`. A server on a different
    /// version rejects our join, so the UI marks it rather than offering it.
    pub protocol_version: u32,
    /// When we last heard from this server
    pub last_seen: Instant,
}

// ─── Server-side broadcaster ───────────────────────────────────────────────────

/// Broadcasts server presence on the LAN.
/// Call `tick()` every game tick — it rate-limits internally.
pub struct ServerBroadcaster {
    socket: Option<UdpSocket>,
    last_broadcast: Instant,
    port: u16,
}

impl ServerBroadcaster {
    /// Create a new broadcaster. `game_port` is the QUIC port clients should connect to.
    pub fn new(game_port: u16) -> Self {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .ok()
            .and_then(|s| {
                s.set_broadcast(true).ok()?;
                s.set_nonblocking(true).ok()?;
                Some(s)
            });

        if socket.is_none() {
            log::warn!("Failed to create discovery broadcast socket");
        }

        Self {
            socket,
            // Subtract BROADCAST_INTERVAL so the first tick fires immediately
            last_broadcast: Instant::now() - BROADCAST_INTERVAL,
            port: game_port,
        }
    }

    /// Broadcast server presence if enough time has elapsed.
    /// Call this every game tick — it rate-limits internally.
    pub fn tick(
        &mut self,
        server_name: &str,
        player_count: u8,
        max_players: u8,
        play_mode: crate::play_mode::PlayMode,
    ) {
        if self.last_broadcast.elapsed() < BROADCAST_INTERVAL {
            return;
        }
        self.last_broadcast = Instant::now();

        let Some(socket) = &self.socket else { return };

        let announce = crate::protocol::ServerAnnouncePacket {
            magic: *b"AXNS",
            protocol_version: crate::protocol::PROTOCOL_VERSION,
            port: self.port,
            server_name: server_name.to_string(),
            player_count,
            max_players,
            is_creative: play_mode.is_creative(),
            play_mode,
        };

        let data = crate::protocol::serialize_packet(
            crate::protocol::PacketType::ServerAnnounce,
            &announce,
        );

        let broadcast_addr = SocketAddr::new(Ipv4Addr::BROADCAST.into(), DISCOVERY_PORT);

        if let Err(e) = socket.send_to(&data, broadcast_addr) {
            // Non-fatal — may not be on any network
            log::trace!("Discovery broadcast failed: {e}");
        }
    }
}

// ─── Client-side listener ──────────────────────────────────────────────────────

/// Strip what an announcement must not smuggle into a label: control
/// characters, bidi overrides/isolates/marks and zero-width format characters
/// (which can reorder or hide text to spoof another world's name).
fn clean_server_name(raw: &str) -> String {
    fn hidden(c: char) -> bool {
        c.is_control()
            || matches!(c,
                '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{061C}'
                | '\u{FEFF}')
    }
    let cleaned: String = raw.chars().filter(|c| !hidden(*c)).collect();
    cleaned.trim().chars().take(MAX_SERVER_NAME_CHARS).collect::<String>().trim().to_string()
}

/// Turn one received UDP datagram into a [`DiscoveredServer`], or `None` if it
/// is not a well-formed AXNS announcement. Pure: `src_ip` is the datagram's
/// source address (the game port comes from the packet, the host from the UDP
/// header, so a packet cannot point a joiner at a third machine's IP).
fn parse_announcement(data: &[u8], src_ip: IpAddr) -> Option<DiscoveredServer> {
    // minimum: 1 byte type tag + at least 4 bytes of payload
    if data.len() < 5 {
        return None;
    }
    let (packet_type, payload) = crate::protocol::deserialize_header(data)?;
    if packet_type != crate::protocol::PacketType::ServerAnnounce {
        return None;
    }
    let announce =
        crate::protocol::safe_deserialize::<crate::protocol::ServerAnnouncePacket>(payload).ok()?;
    if &announce.magic != b"AXNS" || announce.port == 0 {
        return None;
    }
    Some(DiscoveredServer {
        addr: SocketAddr::new(src_ip, announce.port),
        server_name: clean_server_name(&announce.server_name),
        player_count: announce.player_count,
        max_players: announce.max_players,
        is_creative: announce.is_creative,
        protocol_version: announce.protocol_version,
        last_seen: Instant::now(),
    })
}

/// Listens for server broadcasts on the LAN.
/// Call `poll()` frequently to check for new/updated servers.
pub struct ServerListener {
    socket: Option<UdpSocket>,
    /// Discovered servers, keyed by address
    servers: Vec<DiscoveredServer>,
}

impl ServerListener {
    /// Create a new listener bound to the discovery port.
    pub fn new() -> Self {
        let socket = UdpSocket::bind(SocketAddr::new(
            Ipv4Addr::UNSPECIFIED.into(),
            DISCOVERY_PORT,
        ))
        .ok()
        .and_then(|s| {
            s.set_nonblocking(true).ok()?;
            Some(s)
        });

        if socket.is_none() {
            log::warn!("Failed to bind discovery listener on port {DISCOVERY_PORT}");
        }

        Self {
            socket,
            servers: Vec::new(),
        }
    }

    /// A listener on an already-bound socket (tests bind an ephemeral loopback
    /// port instead of the fixed discovery port).
    #[cfg(test)]
    fn from_socket(socket: UdpSocket) -> Self {
        socket.set_nonblocking(true).expect("nonblocking");
        Self { socket: Some(socket), servers: Vec::new() }
    }

    /// Whether the discovery port was bound. `false` means the list will stay
    /// empty (another copy of the game on this machine already listens), which
    /// the UI says rather than showing a silently blank list.
    pub fn is_listening(&self) -> bool {
        self.socket.is_some()
    }

    /// Fold one announcement into the list: refresh a known address, or add a
    /// new one while under the cap (flood protection).
    fn record(&mut self, found: DiscoveredServer) {
        if let Some(existing) = self.servers.iter_mut().find(|s| s.addr == found.addr) {
            *existing = found;
        } else if self.servers.len() < MAX_DISCOVERED {
            log::info!("Discovered server: {} at {}", found.server_name, found.addr);
            self.servers.push(found);
        }
    }

    /// Drop servers not heard from within [`SERVER_TIMEOUT`].
    fn prune(&mut self) {
        self.servers.retain(|s| s.last_seen.elapsed() < SERVER_TIMEOUT);
    }

    /// Poll for new server announcements. Non-blocking.
    /// Call this every frame or every few frames.
    pub fn poll(&mut self) {
        if let Some(socket) = &self.socket {
            let mut buf = [0u8; 1024];
            // At most MAX_PACKETS_PER_POLL parsed announcements per call, so the
            // staging Vec is bounded no matter what the LAN throws at us.
            let mut received: Vec<DiscoveredServer> = Vec::with_capacity(MAX_PACKETS_PER_POLL);
            for _ in 0..MAX_PACKETS_PER_POLL {
                let Ok((len, src_addr)) = socket.recv_from(&mut buf) else { break };
                if let Some(found) = parse_announcement(&buf[..len], src_addr.ip()) {
                    received.push(found);
                }
            }
            for found in received {
                self.record(found);
            }
        }
        self.prune();
    }

    /// Get the list of currently discovered servers.
    pub fn servers(&self) -> &[DiscoveredServer] {
        &self.servers
    }
}

impl Default for ServerListener {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23));

    fn announce(name: &str, port: u16) -> crate::protocol::ServerAnnouncePacket {
        crate::protocol::ServerAnnouncePacket {
            magic: *b"AXNS",
            protocol_version: crate::protocol::PROTOCOL_VERSION,
            port,
            server_name: name.to_string(),
            player_count: 2,
            max_players: 5,
            is_creative: true,
            play_mode: crate::play_mode::PlayMode::Creative,
        }
    }

    fn datagram(p: &crate::protocol::ServerAnnouncePacket) -> Vec<u8> {
        crate::protocol::serialize_packet(crate::protocol::PacketType::ServerAnnounce, p)
    }

    fn listener() -> ServerListener {
        ServerListener { socket: None, servers: Vec::new() }
    }

    #[test]
    fn parses_a_well_formed_announcement_using_the_udp_source_ip() {
        let found = parse_announcement(&datagram(&announce("Axo's World", 7700)), SRC)
            .expect("well-formed announcement");
        assert_eq!(found.addr, SocketAddr::new(SRC, 7700), "host from UDP header, port from packet");
        assert_eq!(found.server_name, "Axo's World");
        assert_eq!((found.player_count, found.max_players), (2, 5));
        assert!(found.is_creative);
        assert_eq!(found.protocol_version, crate::protocol::PROTOCOL_VERSION);
    }

    #[test]
    fn rejects_bad_magic_short_wrong_type_and_port_zero() {
        let mut bad = announce("x", 7700);
        bad.magic = *b"NOPE";
        assert!(parse_announcement(&datagram(&bad), SRC).is_none(), "bad magic");
        assert!(parse_announcement(&[1, 2, 3], SRC).is_none(), "too short");
        assert!(parse_announcement(&[], SRC).is_none(), "empty");
        let mut wrong_type = datagram(&announce("x", 7700));
        wrong_type[0] = crate::protocol::PacketType::StateUpdate as u8;
        assert!(parse_announcement(&wrong_type, SRC).is_none(), "wrong packet type");
        assert!(parse_announcement(&datagram(&announce("x", 0)), SRC).is_none(), "port 0 is undialable");
        assert!(parse_announcement(&[10u8; 40], SRC).is_none(), "garbage payload");
    }

    #[test]
    fn server_name_is_cleaned_and_bounded() {
        let nasty = format!("  Evil\u{202E}\u{200B}World\n{}", "z".repeat(200));
        let found = parse_announcement(&datagram(&announce(&nasty, 7700)), SRC).unwrap();
        assert!(!found.server_name.chars().any(|c| c.is_control()));
        assert!(!found.server_name.contains('\u{202E}') && !found.server_name.contains('\u{200B}'));
        assert!(found.server_name.starts_with("EvilWorld"));
        assert_eq!(found.server_name.chars().count(), MAX_SERVER_NAME_CHARS);
    }

    #[test]
    fn a_repeat_announcement_refreshes_rather_than_duplicates() {
        let mut l = listener();
        l.record(parse_announcement(&datagram(&announce("Old", 7700)), SRC).unwrap());
        let mut newer = announce("New", 7700);
        newer.player_count = 4;
        l.record(parse_announcement(&datagram(&newer), SRC).unwrap());
        assert_eq!(l.servers().len(), 1);
        assert_eq!(l.servers()[0].server_name, "New");
        assert_eq!(l.servers()[0].player_count, 4);
        // Same IP, different port is a different server.
        l.record(parse_announcement(&datagram(&announce("Other", 7701)), SRC).unwrap());
        assert_eq!(l.servers().len(), 2);
    }

    #[test]
    fn the_list_is_capped_against_a_broadcast_flood() {
        let mut l = listener();
        for port in 1..=(MAX_DISCOVERED as u16 + 50) {
            l.record(parse_announcement(&datagram(&announce("flood", port)), SRC).unwrap());
        }
        assert_eq!(l.servers().len(), MAX_DISCOVERED);
    }

    #[test]
    fn silent_servers_expire() {
        let mut l = listener();
        let mut stale = parse_announcement(&datagram(&announce("Gone", 7700)), SRC).unwrap();
        if let Some(t) = Instant::now().checked_sub(SERVER_TIMEOUT + Duration::from_secs(1)) {
            stale.last_seen = t;
        }
        l.record(stale);
        l.record(parse_announcement(&datagram(&announce("Here", 7701)), SRC).unwrap());
        l.prune();
        let names: Vec<&str> = l.servers().iter().map(|s| s.server_name.as_str()).collect();
        assert_eq!(names, ["Here"]);
    }

    #[test]
    fn one_poll_reads_a_bounded_number_of_datagrams() {
        // A flood of distinct announcements already queued on the socket: one
        // poll() takes at most MAX_PACKETS_PER_POLL of them, the next poll()
        // takes the next batch. (Both are below MAX_DISCOVERED, so the list cap
        // is not what is being measured.)
        let rx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind rx");
        let rx_addr = rx.local_addr().unwrap();
        let mut l = ServerListener::from_socket(rx);
        assert!(l.is_listening());
        let tx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind tx");
        let total = MAX_PACKETS_PER_POLL + 20;
        assert!(total < MAX_DISCOVERED);
        for port in 1..=total as u16 {
            tx.send_to(&datagram(&announce("flood", port)), rx_addr).expect("send");
        }
        l.poll();
        assert_eq!(l.servers().len(), MAX_PACKETS_PER_POLL, "first poll is capped");
        l.poll();
        assert_eq!(l.servers().len(), total, "the remainder arrives on the next poll");
    }

    #[test]
    fn a_listener_without_a_socket_reports_it() {
        assert!(!listener().is_listening());
    }
}

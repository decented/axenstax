//! LAN discovery — find game servers on the local network via UDP broadcast.
//!
//! Server broadcasts a ServerAnnouncePacket on UDP port 7705 every second.
//! Client listens on that port and collects discovered servers.
//!
//! The broadcaster (`ServerBroadcaster`) is live — `hosted_server.rs` runs one
//! for every QUIC-hosted world. The listener (`ServerListener` + its
//! `DiscoveredServer` results) has no live caller yet — no "servers on my
//! LAN" browser panel exists in the menu to consume it. Untested (no
//! `#[cfg(test)]` module here) — a real client-side gap, not a false positive.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Discovery broadcast port.
pub const DISCOVERY_PORT: u16 = crate::protocol::DISCOVERY_PORT;

/// How often the server broadcasts.
const BROADCAST_INTERVAL: Duration = Duration::from_secs(1);

/// How long before a discovered server is considered stale.
#[allow(dead_code)]
const SERVER_TIMEOUT: Duration = Duration::from_secs(5);

/// Information about a discovered server.
#[derive(Clone, Debug)]
#[allow(dead_code)]
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

/// Listens for server broadcasts on the LAN.
/// Call `poll()` frequently to check for new/updated servers.
#[allow(dead_code)]
pub struct ServerListener {
    socket: Option<UdpSocket>,
    /// Discovered servers, keyed by address
    servers: Vec<DiscoveredServer>,
}

#[allow(dead_code)]
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

    /// Poll for new server announcements. Non-blocking.
    /// Call this every frame or every few frames.
    pub fn poll(&mut self) {
        let Some(socket) = &self.socket else { return };

        let mut buf = [0u8; 1024];
        while let Ok((len, src_addr)) = socket.recv_from(&mut buf) {
            // minimum: 1 byte type tag + at least 4 bytes of payload
            if len < 5 {
                continue;
            }

            let Some((packet_type, payload)) =
                crate::protocol::deserialize_header(&buf[..len])
            else {
                continue;
            };

            if packet_type != crate::protocol::PacketType::ServerAnnounce {
                continue;
            }

            let Ok(announce) =
                crate::protocol::safe_deserialize::<crate::protocol::ServerAnnouncePacket>(payload)
            else {
                continue;
            };

            if &announce.magic != b"AXNS" {
                continue;
            }

            // Source IP comes from the UDP header; game port comes from the packet.
            let server_addr = SocketAddr::new(src_addr.ip(), announce.port);

            // Sanitise server name: truncate to 64 chars, strip control characters
            let server_name: String = announce.server_name
                .chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect();

            if let Some(existing) = self.servers.iter_mut().find(|s| s.addr == server_addr) {
                existing.server_name = server_name;
                existing.player_count = announce.player_count;
                existing.max_players = announce.max_players;
                existing.is_creative = announce.is_creative;
                existing.last_seen = Instant::now();
            } else if self.servers.len() < 100 {
                // Cap discovered servers list to prevent memory exhaustion
                // from broadcast flooding on the LAN.
                self.servers.push(DiscoveredServer {
                    addr: server_addr,
                    server_name: server_name.clone(),
                    player_count: announce.player_count,
                    max_players: announce.max_players,
                    is_creative: announce.is_creative,
                    last_seen: Instant::now(),
                });
                log::info!(
                    "Discovered server: {} at {}",
                    server_name,
                    server_addr
                );
            }
        }

        // Remove servers we haven't heard from recently
        self.servers.retain(|s| s.last_seen.elapsed() < SERVER_TIMEOUT);
    }

    /// Get the list of currently discovered servers.
    pub fn servers(&self) -> &[DiscoveredServer] {
        &self.servers
    }

    /// Clear the discovered server list.
    pub fn clear(&mut self) {
        self.servers.clear();
    }
}

impl Default for ServerListener {
    fn default() -> Self {
        Self::new()
    }
}

//! LAN discovery — announce a game server on the local network via UDP broadcast.
//!
//! The server broadcasts a ServerAnnouncePacket on UDP port 7705 every second.
//!
//! The broadcaster (`ServerBroadcaster`) runs for the online contact host's
//! plain-QUIC world (`hosted_server.rs`). Nothing in the game listens for it
//! any more: "Games on this network" hears Fray's own beacon (`lan_ui.rs`), and
//! "Host Game" announces through Fray. This is LAN broadcast only: there is no
//! directory, relay or internet discovery here, and there must never be one
//! (red line 1).

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Discovery broadcast port.
pub const DISCOVERY_PORT: u16 = crate::protocol::DISCOVERY_PORT;

/// How often the server broadcasts.
const BROADCAST_INTERVAL: Duration = Duration::from_secs(1);

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

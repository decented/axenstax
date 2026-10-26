//! "Host Game" and "Join Game" over Fray (slice B-2): the network settings both
//! use, a join still finding its host, and the room code's status for the host
//! panel. The wording lives in `lan_ui.rs` and `lan_host.rs`.
//!
//! Red line 2: the only relays used are the player's own "Your relays" list
//! (`GraphicsSettings::online_relays`), with Fray's fallback to its built-in
//! relays switched off. Red line 1: a room code is an invite the host hands
//! out, resolved on those relays; the "Games on this network" list is the LAN
//! beacon, heard on this network only.
//!
//! Admission is unchanged: Fray only carries the game's packets, and every
//! joiner (LAN room or code) meets the host's Signet challenge, signed over the
//! connection's TLS exporter under `EXPORTER-axenstax-join-v1`.
#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use decented_net::{ClientEvent, ClientHandle, DisconnectReason, Identity, LanRoom, NetConfig};

use crate::fray_transport::{net_config, FrayClientTransport};
use crate::transport::ProvenHost;

/// How long the host waits for one of its relays before the panel says the code
/// is not out and the room is on this network only. Fray keeps trying after.
pub const PUBLISH_GRACE: Duration = Duration::from_secs(15);

/// The name every Fray room announces, in the LAN beacon and to a joiner on
/// connecting, before the Signet challenge. Fray sends one name for both and
/// cannot send the world's folder name to the LAN only, so a code holder would
/// learn "Emma's world" before proving who they are. The real name reaches a
/// joiner after the challenge, in the game's own handshake.
pub const ROOM_NAME: &str = "AxeNStax world";

/// AxeNStax's Fray settings with the player's relays: the code is published on
/// (and looked up on) `relays` only, never on Fray's defaults. STUN is
/// AxeNStax's own list, the one online play by contact uses.
pub fn play_config(relays: &[String]) -> NetConfig {
    let mut cfg = net_config(crate::data_dir::profile_dir());
    cfg.relays = relays.to_vec();
    cfg.relay_fallback = false;
    cfg.stun = crate::nat::stun::STUN_SERVERS.iter().map(|s| s.to_string()).collect();
    cfg
}

/// The Fray identity for one room or one join: a fresh key each time. Nothing
/// in the game reads it (the Signet challenge decides who joins), and a stable
/// key would follow a player from room to room on the relays.
pub fn session_identity() -> Identity {
    Identity::ephemeral()
}

/// A "Join Game" text that names a dedicated server (the WebSocket path, and
/// `axenstax://` connect strings) rather than a room code. A Keep invite link
/// (`axenstax://keep/...`, in any case, or cut short) is not one: it joins a
/// Keep and never reaches the WebSocket path.
pub fn is_server_address(text: &str) -> bool {
    let t = text.trim();
    if crate::keeps::is_keep_text(t) {
        return false;
    }
    t.starts_with("ws://") || t.starts_with("wss://") || t.starts_with("axenstax://")
}

/// Where the room code stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeStatus {
    /// Reaching the player's relays; the room is already on this network.
    Publishing,
    /// On `up` of the player's `total` relays: the code works from anywhere.
    Published { up: usize, total: usize },
    /// No relay carries the code (none set, or none reached within
    /// [`PUBLISH_GRACE`]): only this network can join. `tried` relays.
    LanOnly { tried: usize },
}

/// The status from Fray's `relays_connected()` and the time since hosting began.
pub fn code_status(relays_connected: (usize, usize), since_start: Duration) -> CodeStatus {
    let (up, total) = relays_connected;
    if up > 0 {
        CodeStatus::Published { up, total }
    } else if total == 0 || since_start >= PUBLISH_GRACE {
        CodeStatus::LanOnly { tried: total }
    } else {
        CodeStatus::Publishing
    }
}

/// What the host panel shows about the room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomView {
    /// The code as players read and type it (`KMT RWX`).
    pub code: String,
    pub status: CodeStatus,
}

/// The room "Host Game" opened, as the game keeps it.
#[derive(Debug)]
pub struct HostedRoom {
    /// This machine's LAN addresses (empty: no network, nobody can join).
    pub access: crate::lan_host::HostAccess,
    pub started: Instant,
    /// The "code is live" toast has been shown.
    pub announced: bool,
    /// The room reported listening (it opens in the background).
    settled: bool,
}

impl HostedRoom {
    pub fn new(access: crate::lan_host::HostAccess) -> HostedRoom {
        HostedRoom { access, started: Instant::now(), announced: false, settled: false }
    }

    /// The room has finished opening.
    pub fn settled(&self) -> bool {
        self.settled
    }

    /// The room is listening. The wait for a relay (see [`PUBLISH_GRACE`])
    /// counts from now: before this the room was still asking the router and a
    /// STUN server, and had not yet reached a relay.
    pub fn settle(&mut self) {
        self.settled = true;
        self.started = Instant::now();
    }

    /// The panel's view of `server`'s room, if it hosts one.
    pub fn view(&self, server: &crate::hosted_server::HostedServer) -> Option<RoomView> {
        let room = server.fray_room()?.handle();
        Some(RoomView {
            code: decented_net::room::display_code(room.code()),
            status: code_status(room.relays_connected(), self.started.elapsed()),
        })
    }

    /// The code, once, the first time it is published.
    pub fn take_published(&mut self, server: &crate::hosted_server::HostedServer) -> Option<String> {
        if self.announced || !self.settled {
            return None;
        }
        let view = self.view(server)?;
        if !matches!(view.status, CodeStatus::Published { .. }) {
            return None;
        }
        self.announced = true;
        Some(view.code)
    }
}

/// What a join in flight did this frame.
pub enum JoinPoll {
    Waiting,
    /// The host is reached: the transport carries the game's handshake next.
    Connected(FrayClientTransport),
    /// The join is over; the message says why, for the lobby.
    Failed(String),
}

/// A "Join Game" through Fray that has not reached its host yet. It is held in
/// the lobby (not in a loading screen) so a failure can say what went wrong:
/// a wrong code, no relay, a full room.
pub struct PendingJoin {
    handle: Option<ClientHandle>,
    /// What the player picked: a room's name, or the code as typed.
    label: String,
    /// A Keep invite join: a Keep says nothing to an invite it does not honour,
    /// so its failures are worded for that (`keeps::invite_failure_text`).
    invite: bool,
}

impl PendingJoin {
    /// Join a room from the "Games on this network" list.
    pub fn lan(room: &LanRoom, relays: &[String]) -> PendingJoin {
        let handle = decented_net::join_lan(&play_config(relays), &session_identity(), room);
        Self::over(handle, crate::lan_ui::room_label(room))
    }

    /// Join by room code over the player's relays.
    pub fn code(code: &str, relays: &[String]) -> PendingJoin {
        let handle = decented_net::join(&play_config(relays), &session_identity(), code);
        Self::over(handle, decented_net::room::display_code(code))
    }

    /// Join a Keep by invite: the Keep's runtime key and the invite's secret
    /// (from an `axenstax://keep/` link or a saved Keep), found over the
    /// player's relays. The host must prove it is that key or the join fails.
    pub fn invite(invite: &fray_keep::Invite, relays: &[String]) -> PendingJoin {
        let handle = decented_net::join_invite(&play_config(relays), &session_identity(), &invite.secret, invite.host_key);
        let mut join = Self::over(handle, crate::keeps::label(&invite.host_key));
        join.invite = true;
        join
    }

    /// Follow a join started elsewhere (tests start theirs on loopback).
    pub fn over(handle: ClientHandle, label: String) -> PendingJoin {
        PendingJoin { handle: Some(handle), label, invite: false }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Word this join's failures as a Keep invite join's (the probe starts its own handle).
    #[cfg(test)]
    pub fn set_invite(&mut self, invite: bool) {
        self.invite = invite;
    }

    /// Read Fray's join events, stopping exactly at `Connected`: the host's
    /// first game packet comes after it, so it stays queued for the transport.
    pub fn poll(&mut self) -> JoinPoll {
        let Some(handle) = &self.handle else {
            return JoinPoll::Failed(self.failed("the join was already over"));
        };
        while let Some(event) = handle.poll() {
            match event {
                ClientEvent::Progress { .. } | ClientEvent::Lossy(_) => {}
                ClientEvent::Connected { host_pubkey, host_name, .. } => {
                    let Some(handle) = self.handle.take() else { break };
                    // The host's proof travels with the link: a return pass is
                    // believed only from this key.
                    let host = ProvenHost { key: host_pubkey.to_bytes(), name: host_name };
                    return JoinPoll::Connected(FrayClientTransport::already_connected_to(handle, host));
                }
                ClientEvent::Failed { stage, reason } => {
                    log::info!("Fray: joining {} failed at {stage:?}: {reason}", self.label);
                    self.handle = None;
                    if self.invite {
                        return JoinPoll::Failed(crate::keeps::invite_failure_text(&reason, &self.label));
                    }
                    return JoinPoll::Failed(self.failed(&reason.to_string()));
                }
                ClientEvent::Disconnected { reason } => {
                    self.handle = None;
                    return JoinPoll::Failed(self.disconnected(reason));
                }
                // The host sends nothing before seating us; never seen here.
                // Dropped if it ever is, so say so.
                ClientEvent::Reliable(_) => {
                    log::warn!("Fray: a packet arrived before Connected; dropped");
                }
            }
        }
        JoinPoll::Waiting
    }

    /// What the lobby says when the host closed the link before the join was seated. A Keep
    /// that turns an invite join away says the same one sentence as a Keep that never answers.
    fn disconnected(&self, reason: DisconnectReason) -> String {
        if self.invite && matches!(reason, DisconnectReason::Kicked) {
            return crate::keeps::UNREACHABLE.to_string();
        }
        let why = match reason {
            DisconnectReason::Kicked => "the host turned us away".to_string(),
            // The host's own sentence (a Keep says why it let the joiner go), already cleaned
            // and cut by Fray; an invite join shows it too, where a plain kick is folded.
            DisconnectReason::KickedWith(text) => text,
            DisconnectReason::Error(e) => e,
            _ => "the host left".to_string(),
        };
        self.failed(&why)
    }

    fn failed(&self, reason: &str) -> String {
        crate::lan_host::join_failure_notice(&self.label, reason)
    }
}

#[cfg(test)]
#[path = "fray_play_tests.rs"]
mod tests;

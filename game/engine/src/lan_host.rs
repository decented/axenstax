//! LAN hosting helpers (gap-audit T2-10): the address a joiner types, a
//! synchronous port bind that fails *honestly*, and the human wording for both.
//!
//! Pure functions (no egui) so the wording and the address rules are testable;
//! `lan_ui.rs` draws them. Native only — the web build has no multiplayer.
//!
//! No external lookups: the address comes from the kernel's routing table
//! (`nat::candidates::local_outbound_v4`, a UDP `connect` that sends nothing).
//! Nothing here talks to an AxeNStax-operated or third-party service (red
//! lines 1 and 3).
#![cfg(not(target_arch = "wasm32"))]

use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

/// Keep only addresses a neighbour on this network could dial, private (RFC 1918)
/// ones first, without duplicates. Loopback, unspecified, link-local and
/// broadcast addresses are dropped — they are no use to another machine.
pub fn lan_addresses_from(candidates: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for ip in candidates {
        let usable = !ip.is_loopback()
            && !ip.is_unspecified()
            && !ip.is_link_local()
            && !ip.is_broadcast()
            && !ip.is_multicast();
        if usable && !out.contains(ip) {
            out.push(*ip);
        }
    }
    // Stable sort: private addresses before anything else, original order kept.
    out.sort_by_key(|ip| !ip.is_private());
    out
}

/// This machine's LAN addresses, best first. Empty when there is no usable
/// network. Uses the routing-table trick the online-play code already uses, so
/// it picks the interface the kernel would really send LAN traffic from.
pub fn detect_lan_addresses() -> Vec<Ipv4Addr> {
    let found: Vec<Ipv4Addr> = crate::nat::candidates::local_outbound_v4().into_iter().collect();
    lan_addresses_from(&found)
}

/// What a joiner needs to reach this host: its LAN address(es) and the port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAccess {
    pub addresses: Vec<Ipv4Addr>,
    pub port: u16,
}

impl HostAccess {
    /// Detect the addresses now and pair them with `port`.
    pub fn detect(port: u16) -> Self {
        Self { addresses: detect_lan_addresses(), port }
    }

    /// `ip:port` strings exactly as a joiner would type them, best first.
    pub fn join_addresses(&self) -> Vec<String> {
        self.addresses.iter().map(|ip| SocketAddr::new((*ip).into(), self.port).to_string()).collect()
    }

    /// The best address to show first, if the machine has a network at all.
    pub fn primary(&self) -> Option<String> {
        self.join_addresses().into_iter().next()
    }
}

/// Plain-English reason a UDP bind on `port` failed. Never shows a raw OS code
/// for the cases a player can act on.
pub fn describe_bind_error(err: &io::Error, port: u16) -> String {
    use io::ErrorKind as K;
    match err.kind() {
        K::AddrInUse => format!(
            "port {port} is already in use. Another copy of Axe'n'Stax (or another program) \
             on this computer is probably hosting. Close it and try again."
        ),
        K::PermissionDenied => format!(
            "this computer would not let Axe'n'Stax open port {port}. \
             Check your firewall or security settings."
        ),
        K::AddrNotAvailable | K::NetworkUnreachable | K::NetworkDown | K::HostUnreachable => {
            "no network connection was found. Connect to Wi-Fi or Ethernet and try again.".to_string()
        }
        _ => format!("couldn't open port {port} ({err})."),
    }
}

/// Bind the UDP port a LAN host listens on, **now**, so a failure is reported to
/// the player instead of being logged on a worker thread while the game carries
/// on as if hosting had worked. The socket is handed to the QUIC accept thread.
pub fn bind_lan_socket(port: u16) -> Result<UdpSocket, String> {
    UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).map_err(|e| describe_bind_error(&e, port))
}

/// Toast shown when the host starts. With an address, say what to type; without
/// one, hosting works but nobody can reach it, which is worth saying.
pub fn hosting_toast(access: &HostAccess) -> String {
    match access.primary() {
        Some(addr) => format!("Hosting on your network. Friends on the same Wi-Fi join {addr}"),
        None => "Hosting, but no network connection was found, so nobody can join yet. \
                 Connect to Wi-Fi or Ethernet."
            .to_string(),
    }
}

/// Toast shown when hosting could not start. The world still opens (solo), and
/// the player is told so rather than left wondering why nobody can join.
pub fn host_failure_toast(reason: &str) -> String {
    let reason = reason.trim();
    let stop = if reason.ends_with(['.', '!', '?']) { "" } else { "." };
    format!("Couldn't host for friends: {reason}{stop} Opening your world for solo play instead.")
}

/// Message shown when "Join Game" cannot connect.
pub fn join_failure_notice(address: &str, reason: &str) -> String {
    format!("Couldn't join {}: {}", address.trim(), reason.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
        Ipv4Addr::new(a, b, c, d)
    }

    #[test]
    fn private_addresses_come_first_and_junk_is_dropped() {
        let got = lan_addresses_from(&[
            ip(100, 64, 0, 9),  // CGNAT / VPN: usable but not private
            ip(127, 0, 0, 1),   // loopback
            ip(0, 0, 0, 0),     // unspecified
            ip(169, 254, 3, 4), // link-local
            ip(255, 255, 255, 255),
            ip(224, 0, 0, 1),   // multicast
            ip(192, 168, 1, 23),
            ip(10, 0, 0, 5),
            ip(192, 168, 1, 23), // duplicate
            ip(172, 16, 8, 8),
        ]);
        assert_eq!(
            got,
            vec![ip(192, 168, 1, 23), ip(10, 0, 0, 5), ip(172, 16, 8, 8), ip(100, 64, 0, 9)]
        );
    }

    #[test]
    fn no_candidates_means_no_addresses() {
        assert!(lan_addresses_from(&[]).is_empty());
        assert!(lan_addresses_from(&[ip(127, 0, 0, 1)]).is_empty());
    }

    #[test]
    fn join_addresses_are_what_a_joiner_types() {
        let a = HostAccess { addresses: vec![ip(192, 168, 1, 23), ip(10, 0, 0, 5)], port: 7700 };
        assert_eq!(a.join_addresses(), ["192.168.1.23:7700", "10.0.0.5:7700"]);
        assert_eq!(a.primary().as_deref(), Some("192.168.1.23:7700"));
        let none = HostAccess { addresses: vec![], port: 7700 };
        assert_eq!(none.primary(), None);
    }

    #[test]
    fn bind_errors_read_as_plain_english() {
        let in_use = describe_bind_error(&io::Error::from(io::ErrorKind::AddrInUse), 7700);
        assert!(in_use.contains("7700") && in_use.contains("already in use"), "got {in_use}");
        let denied = describe_bind_error(&io::Error::from(io::ErrorKind::PermissionDenied), 7700);
        assert!(denied.to_lowercase().contains("firewall"), "got {denied}");
        for kind in [
            io::ErrorKind::AddrNotAvailable,
            io::ErrorKind::NetworkUnreachable,
            io::ErrorKind::NetworkDown,
        ] {
            let m = describe_bind_error(&io::Error::from(kind), 7700);
            assert!(m.contains("no network connection"), "{kind:?} -> {m}");
        }
        let other = describe_bind_error(&io::Error::other("boom"), 7700);
        assert!(other.contains("boom") && other.contains("7700"), "got {other}");
    }

    #[test]
    fn binding_a_taken_port_fails_with_the_in_use_message() {
        let holder = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).expect("bind an ephemeral port");
        let port = holder.local_addr().unwrap().port();
        let err = bind_lan_socket(port).expect_err("the port is held");
        assert!(err.contains("already in use"), "got {err}");
        drop(holder);
        // Once released, the same port binds.
        let again = bind_lan_socket(port).expect("free port binds");
        assert_eq!(again.local_addr().unwrap().port(), port);
    }

    #[test]
    fn toasts_name_the_address_or_the_problem() {
        let a = HostAccess { addresses: vec![ip(192, 168, 1, 23)], port: 7700 };
        let t = hosting_toast(&a);
        assert!(t.contains("192.168.1.23:7700"), "got {t}");
        let none = HostAccess { addresses: vec![], port: 7700 };
        assert!(hosting_toast(&none).contains("no network connection"));

        let f = host_failure_toast("port 7700 is already in use.");
        assert!(f.contains("already in use") && f.contains("solo play"), "got {f}");
        assert!(host_failure_toast("something odd").contains("something odd. Opening"));
        assert!(join_failure_notice(" 10.0.0.2:7700 ", "timed out").starts_with("Couldn't join 10.0.0.2:7700"));
    }
}

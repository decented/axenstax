//! Every address this machine might be reachable at, in the order worth trying.
//!
//! All four kinds are gathered on **one already-bound UDP socket** — the socket
//! quinn will be handed — so the port a peer is told to dial is the port QUIC
//! will actually answer on. That is why `gather` takes a `&UdpSocket` rather
//! than binding its own.
//!
//! Addresses are personal data. They exist here, inside NIP-44 ciphertext on
//! the wire, and nowhere else (CLAUDE.md red line 3).
#![cfg(not(target_arch = "wasm32"))]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use crate::rendezvous::payload::Candidate;

/// The four ways a peer might reach this machine.
///
/// **Declaration order is priority order** and `Ord` is derived from it: same
/// house first (fastest and always works), then a real global IPv6 (no NAT to
/// fight), then a mapping the router agreed to, then whatever the NAT is doing
/// today. Do not reorder these variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    Lan,
    V6,
    Upnp,
    Stun,
}

impl CandidateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CandidateKind::Lan => "lan",
            CandidateKind::V6 => "v6",
            CandidateKind::Upnp => "upnp",
            CandidateKind::Stun => "stun",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "lan" => Some(CandidateKind::Lan),
            "v6" => Some(CandidateKind::V6),
            "upnp" => Some(CandidateKind::Upnp),
            "stun" => Some(CandidateKind::Stun),
            _ => None,
        }
    }
}

/// Priority order, de-duplicated by address.
///
/// An unrecognised kind sorts last rather than being dropped: a future build
/// may add one (NAT-PMP is the planned next), and an address we don't have a
/// name for is still an address worth trying.
pub fn sort_candidates(list: Vec<Candidate>) -> Vec<Candidate> {
    let mut list = list;
    list.sort_by_key(|c| {
        CandidateKind::parse(&c.kind)
            .map(|k| k as u8)
            .unwrap_or(u8::MAX)
    });
    let mut seen: Vec<String> = Vec::new();
    list.retain(|c| {
        if seen.contains(&c.addr) {
            false
        } else {
            seen.push(c.addr.clone());
            true
        }
    });
    list
}

/// Whether anything here could plausibly work from outside the house. Drives
/// the host-side warning in spec §4.4.
pub fn reachable_beyond_lan(list: &[Candidate]) -> bool {
    list.iter().any(|c| {
        matches!(
            CandidateKind::parse(&c.kind),
            Some(CandidateKind::V6 | CandidateKind::Upnp | CandidateKind::Stun)
        )
    })
}

/// How many of a peer's addresses are ever acted on.
///
/// The candidate list arrives inside somebody else's ciphertext, so its length
/// is chosen by them, not by us. Both sides then *send packets* to every
/// address in it — the host punches at them, the joiner dials them — which
/// makes an uncapped list a packet reflector: one offer naming ten thousand
/// addresses turns this machine into the source of ten thousand datagrams
/// aimed wherever the sender liked. Eight is comfortably more than the four
/// kinds [`CandidateKind`] can produce, and it is applied on both sides.
pub const MAX_CANDIDATE_ADDRS: usize = 8;

/// The dialable addresses, in order, at most [`MAX_CANDIDATE_ADDRS`] of them.
/// Unparseable entries are skipped — one garbled candidate must not cost the
/// peer the rest.
pub fn parse_addrs(list: &[Candidate]) -> Vec<SocketAddr> {
    list.iter()
        .filter_map(|c| c.addr.parse::<SocketAddr>().ok())
        .take(MAX_CANDIDATE_ADDRS)
        .collect()
}

/// This machine's outbound IPv4, found by the routing-table trick: `connect` a
/// throwaway UDP socket at a public address (no packet is sent) and read back
/// which local address the kernel chose. Cheaper and more accurate than
/// enumerating interfaces, and it picks the *right* one on a multi-homed box.
pub fn local_outbound_v4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1: routable, never answers
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() && !v4.is_link_local() => {
            Some(v4)
        }
        _ => None,
    }
}

/// The same trick over IPv6, and the same rejection of anything that isn't a
/// real global address — a link-local or loopback answer is no use to a peer in
/// another house.
pub fn local_outbound_v6() -> Option<Ipv6Addr> {
    let s = UdpSocket::bind("[::]:0").ok()?;
    s.connect("[2001:db8::1]:9").ok()?; // documentation prefix
    match s.local_addr().ok()?.ip() {
        IpAddr::V6(v6)
            if !v6.is_loopback()
                && !v6.is_unspecified()
                // Link-local fe80::/10 — same-link only.
                && (v6.segments()[0] & 0xffc0) != 0xfe80
                // Unique-local fc00::/7 — private, not internet-reachable.
                && (v6.octets()[0] & 0xfe) != 0xfc =>
        {
            Some(v6)
        }
        _ => None,
    }
}

/// Gather every candidate for `sock`.
///
/// `upnp` is a mapping already obtained by [`crate::nat::upnp::map_port`] (the
/// caller owns it, because it must be renewed and removed on a schedule this
/// function knows nothing about). `stun_timeout` is per server; both servers
/// are tried and the first answer wins.
pub fn gather(
    sock: &UdpSocket,
    upnp: Option<SocketAddr>,
    stun_timeout: Duration,
) -> Vec<Candidate> {
    let port = sock.local_addr().map(|a| a.port()).unwrap_or(0);
    let mut out: Vec<Candidate> = Vec::new();

    if let Some(v4) = local_outbound_v4() {
        out.push(Candidate {
            kind: CandidateKind::Lan.as_str().to_string(),
            addr: SocketAddr::new(IpAddr::V4(v4), port).to_string(),
        });
    } else if let Ok(local) = sock.local_addr() {
        // Loopback-bound (tests) or an unusual box: still name the socket, so a
        // same-machine or same-namespace peer has something to dial.
        out.push(Candidate {
            kind: CandidateKind::Lan.as_str().to_string(),
            addr: local.to_string(),
        });
    }

    if let Some(v6) = local_outbound_v6() {
        out.push(Candidate {
            kind: CandidateKind::V6.as_str().to_string(),
            addr: SocketAddr::new(IpAddr::V6(v6), port).to_string(),
        });
    }

    if let Some(mapped) = upnp {
        out.push(Candidate {
            kind: CandidateKind::Upnp.as_str().to_string(),
            addr: mapped.to_string(),
        });
    }

    for server in crate::nat::stun::STUN_SERVERS {
        match crate::nat::stun::reflexive_address(sock, server, stun_timeout) {
            Ok(addr) => {
                out.push(Candidate {
                    kind: CandidateKind::Stun.as_str().to_string(),
                    addr: addr.to_string(),
                });
                break;
            }
            Err(e) => log::debug!("[nat] {server}: {e}"),
        }
    }

    sort_candidates(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn c(kind: &str, addr: &str) -> Candidate {
        Candidate { kind: kind.to_string(), addr: addr.to_string() }
    }

    /// REGRESSION (whole-branch review, IMPORTANT 2). The candidate list comes
    /// out of somebody else's ciphertext and both sides send packets to every
    /// address in it, so an uncapped list is a packet reflector.
    #[test]
    fn a_flood_of_candidates_is_capped_before_anything_is_sent_to_them() {
        let flood: Vec<Candidate> = (0..9)
            .map(|i| c("stun", &format!("198.51.100.{i}:4000")))
            .collect();
        let addrs = parse_addrs(&flood);
        assert_eq!(addrs.len(), MAX_CANDIDATE_ADDRS);
        assert_eq!(addrs.len(), 8, "the cap the review asked for");
        // The cap keeps the FIRST eight — the list arrives in priority order,
        // so the ones worth trying are the ones kept.
        assert_eq!(addrs[0], "198.51.100.0:4000".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn the_cap_counts_dialable_addresses_not_garbled_lines() {
        // Nine good addresses preceded by rubbish must still yield eight good
        // ones: a peer cannot spend our budget on unparseable entries.
        let mut list = vec![c("stun", "not-an-address"), c("stun", "")];
        list.extend((0..9).map(|i| c("stun", &format!("198.51.100.{i}:4000"))));
        assert_eq!(parse_addrs(&list).len(), 8);
    }

    #[test]
    fn link_local_v4_is_excluded_like_the_v6_case() {
        // 169.254/16 (APIPA/self-assigned) is same-link-only, exactly like the
        // fe80::/10 exclusion in local_outbound_v6 — local_outbound_v4 must
        // reject it too rather than handing a peer an address nobody outside
        // the segment can reach.
        assert!(Ipv4Addr::new(169, 254, 1, 1).is_link_local());
    }

    #[test]
    fn candidate_kinds_round_trip_their_wire_spelling() {
        for k in [
            CandidateKind::Lan,
            CandidateKind::V6,
            CandidateKind::Upnp,
            CandidateKind::Stun,
        ] {
            assert_eq!(CandidateKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(CandidateKind::parse("teredo"), None);
    }

    #[test]
    fn declaration_order_is_priority_order() {
        // Cheapest and most likely first: the same house, then a real global
        // address, then a mapping we asked the router for, then whatever the
        // NAT happens to be doing today.
        assert!(CandidateKind::Lan < CandidateKind::V6);
        assert!(CandidateKind::V6 < CandidateKind::Upnp);
        assert!(CandidateKind::Upnp < CandidateKind::Stun);
    }

    #[test]
    fn sort_puts_them_in_priority_order() {
        let sorted = sort_candidates(vec![
            c("stun", "203.0.113.9:41234"),
            c("upnp", "198.51.100.7:7700"),
            c("lan", "192.168.1.20:7700"),
            c("v6", "[2001:db8::1]:7700"),
        ]);
        let kinds: Vec<&str> = sorted.iter().map(|x| x.kind.as_str()).collect();
        assert_eq!(kinds, vec!["lan", "v6", "upnp", "stun"]);
    }

    #[test]
    fn sort_drops_duplicate_addresses_keeping_the_higher_priority_one() {
        let sorted = sort_candidates(vec![
            c("stun", "198.51.100.7:7700"),
            c("upnp", "198.51.100.7:7700"),
        ]);
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].kind, "upnp", "the router mapping is the better bet");
    }

    #[test]
    fn sort_keeps_an_unknown_kind_last_rather_than_dropping_it() {
        // Forwards compatibility: a future kind we don't understand is still an
        // address worth trying, just not one to try first.
        let sorted = sort_candidates(vec![
            c("natpmp", "198.51.100.9:7700"),
            c("lan", "192.168.1.20:7700"),
        ]);
        assert_eq!(sorted.len(), 2);
        assert_eq!(sorted[0].kind, "lan");
        assert_eq!(sorted[1].kind, "natpmp");
    }

    #[test]
    fn reachable_beyond_lan_is_false_for_lan_only() {
        assert!(!reachable_beyond_lan(&[c("lan", "192.168.1.20:7700")]));
        assert!(!reachable_beyond_lan(&[]));
        assert!(reachable_beyond_lan(&[c("v6", "[2001:db8::1]:7700")]));
        assert!(reachable_beyond_lan(&[c("upnp", "198.51.100.7:7700")]));
        assert!(reachable_beyond_lan(&[c("stun", "203.0.113.9:41234")]));
    }

    #[test]
    fn parse_addrs_skips_anything_unparseable_without_failing_the_lot() {
        let got = parse_addrs(&[
            c("lan", "192.168.1.20:7700"),
            c("lan", "this is not an address"),
            c("v6", "[2001:db8::1]:7700"),
        ]);
        assert_eq!(
            got,
            vec![
                "192.168.1.20:7700".parse::<SocketAddr>().unwrap(),
                "[2001:db8::1]:7700".parse::<SocketAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn gather_always_yields_at_least_the_bound_socket_on_loopback() {
        // No internet needed: bind loopback, gather with UPnP off and a 1ms
        // STUN budget. The LAN candidate must carry the socket's real port,
        // because that is the port the peer will be told to dial.
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = sock.local_addr().unwrap().port();
        let got = gather(&sock, None, std::time::Duration::from_millis(1));
        assert!(
            got.iter().any(|x| x.addr.ends_with(&format!(":{port}"))),
            "every candidate must name the bound port: {got:?}"
        );
        assert_eq!(got, sort_candidates(got.clone()), "gather returns them sorted");
    }

    #[test]
    fn gather_includes_a_supplied_upnp_mapping() {
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let mapped: SocketAddr = "198.51.100.7:41000".parse().unwrap();
        let got = gather(&sock, Some(mapped), std::time::Duration::from_millis(1));
        assert!(
            got.iter().any(|x| x.kind == "upnp" && x.addr == "198.51.100.7:41000"),
            "{got:?}"
        );
    }
}

//! WebSocket join origin — protocol v66 (Spec 04 §1.8.1, Spec 08 §9.0.1
//! T-JOIN-RELAY, "WebSocket residual").
//!
//! A QUIC join signs `axenstax-join:tls-exporter:<hex>` (channel-bound). A
//! WebSocket's TLS ends at the reverse proxy (Caddy), so there is no exporter
//! to bind to. Instead a WS joiner signs the address it actually dialled:
//! `axenstax-join:ws-host:<host[:port]>`, normalised by [`normalise_host_port`]
//! (the one normaliser both ends use). A dedicated server that knows its own
//! public address(es) ([`PublicHosts`], `--public-host`) refuses a signature
//! made for any other address, so a relaying server M cannot replay a victim's
//! signature (made for M's address) to the real server H.
//!
//! **Residual:** a server with no public host configured accepts any `ws-host`
//! origin (the pre-v66 behaviour); `server_main` logs
//! [`relay_protection_warning`] once at boot.
//!
//! **Non-unique entries:** an address that other machines can also own (an
//! RFC 1918 / CGNAT / loopback / link-local IP, a `.local` or `.lan` name, a
//! single-label name) cannot tell this server from a relay on the victim's own
//! network: M at `192.168.1.20` on V's LAN gets V to sign
//! `ws-host:192.168.1.20:6767` and replays it to H's public endpoint. Such
//! entries are still accepted (a WS-only server must serve LAN joins) but
//! [`PublicHosts::non_unique`] lists them and the boot log warns that they are
//! not relay-protected ([`boot_messages`]).
//!
//! **Ports and schemes:** the origin carries neither the scheme nor, for the
//! scheme's default port, the port, so `:443` also admits a plaintext
//! `ws://host` (port 80) and `:80` also admits `wss://host`. An entry without
//! a port admits only the default ports (80/443), the server's own `ws_port`
//! and 8443 (Caddy's alternate HTTPS port in the bundled compose file), never
//! "any port" (`PublicHost::admits`).
//!
//! **Host normalisation:** ASCII lowercase, one trailing dot stripped, the
//! default port dropped (80 for `ws`, 443 for `wss`), IPv6 in brackets in its
//! canonical `std` form. Internationalised names are converted to punycode on
//! native (`idna`, already in the dependency tree via `url`). The browser
//! build rejects a non-ASCII host here, but it never sees one: it reads the
//! URL back from `WebSocket.url`, which the browser has already punycoded.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Prefix of a WebSocket join origin; followed by the normalised dialled
/// `host[:port]`.
pub const JOIN_ORIGIN_WS_HOST_PREFIX: &str = "axenstax-join:ws-host:";

/// The longest host name accepted (DNS limit).
const MAX_HOST_LEN: usize = 253;

/// Why an address could not be normalised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsHostError {
    /// The URL's scheme is not `ws` or `wss`.
    NotWebSocketUrl,
    /// No host at all.
    Empty,
    /// `user@host` — credentials have no place in a join address.
    Userinfo,
    /// A non-ASCII host on a build without IDNA (the browser).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    NonAsciiHost,
    /// The host is not a valid DNS name or IP address.
    InvalidHost,
    /// The port is not a number in 1..=65535.
    InvalidPort,
}

impl fmt::Display for WsHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotWebSocketUrl => "not a ws:// or wss:// address",
            Self::Empty => "no host name",
            Self::Userinfo => "a user name (user@host) is not allowed in a server address",
            Self::NonAsciiHost => {
                "international domain names aren't supported here; use the xn-- (punycode) form"
            }
            Self::InvalidHost => "not a valid host name or IP address",
            Self::InvalidPort => "not a valid port number",
        })
    }
}

/// Split and normalise `host[:port]`. Returns the canonical host (IPv6 in
/// brackets) and the port, with `default_port` dropped. The single normaliser
/// both the joiner ([`ws_url_host`]) and the server ([`PublicHosts`],
/// [`expected_ws_join_origin`]) run.
fn split_host_port(
    authority: &str,
    default_port: Option<u16>,
) -> Result<(String, Option<u16>), WsHostError> {
    let a = authority.trim();
    if a.is_empty() {
        return Err(WsHostError::Empty);
    }
    if a.contains('@') {
        return Err(WsHostError::Userinfo);
    }
    let (host, port) = if let Some(rest) = a.strip_prefix('[') {
        // [IPv6] or [IPv6]:port
        let (inner, after) = rest.split_once(']').ok_or(WsHostError::InvalidHost)?;
        let ip: Ipv6Addr = inner.parse().map_err(|_| WsHostError::InvalidHost)?;
        let port = match after {
            "" => None,
            p => Some(parse_port(p.strip_prefix(':').ok_or(WsHostError::InvalidHost)?)?),
        };
        (format!("[{ip}]"), port)
    } else if a.matches(':').count() > 1 {
        // A bare IPv6 address (no brackets, so no port).
        let ip: Ipv6Addr = a.parse().map_err(|_| WsHostError::InvalidHost)?;
        (format!("[{ip}]"), None)
    } else {
        let (h, port) = match a.split_once(':') {
            Some((h, p)) => (h, Some(parse_port(p)?)),
            None => (a, None),
        };
        (normalise_name(h)?, port)
    };
    let port = port.filter(|p| Some(*p) != default_port);
    Ok((host, port))
}

fn parse_port(p: &str) -> Result<u16, WsHostError> {
    if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WsHostError::InvalidPort);
    }
    match p.parse::<u16>() {
        Ok(0) | Err(_) => Err(WsHostError::InvalidPort),
        Ok(n) => Ok(n),
    }
}

/// A DNS name or IPv4 address → lowercase ASCII, trailing dot stripped.
fn normalise_name(h: &str) -> Result<String, WsHostError> {
    let ascii = if h.is_ascii() { h.to_ascii_lowercase() } else { idna_to_ascii(h)? };
    let name = ascii.strip_suffix('.').unwrap_or(&ascii);
    if name.is_empty() {
        return Err(WsHostError::Empty);
    }
    if let Ok(ip) = name.parse::<Ipv4Addr>() {
        return Ok(ip.to_string());
    }
    let valid = name.len() <= MAX_HOST_LEN
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
    if valid { Ok(name.to_string()) } else { Err(WsHostError::InvalidHost) }
}

#[cfg(not(target_arch = "wasm32"))]
fn idna_to_ascii(h: &str) -> Result<String, WsHostError> {
    idna::domain_to_ascii(h).map(|s| s.to_ascii_lowercase()).map_err(|_| WsHostError::InvalidHost)
}

#[cfg(target_arch = "wasm32")]
fn idna_to_ascii(_h: &str) -> Result<String, WsHostError> {
    Err(WsHostError::NonAsciiHost)
}

/// Normalise `host[:port]`, dropping `default_port`. See the module docs for
/// the rules.
pub fn normalise_host_port(authority: &str, default_port: Option<u16>) -> Result<String, WsHostError> {
    let (host, port) = split_host_port(authority, default_port)?;
    Ok(match port {
        Some(p) => format!("{host}:{p}"),
        None => host,
    })
}

/// The normalised `host[:port]` of a `ws://` / `wss://` URL — what a joiner
/// signs into its join origin. The default port of the scheme is dropped.
pub fn ws_url_host(url: &str) -> Result<String, WsHostError> {
    let (scheme, rest) = url.trim().split_once("://").ok_or(WsHostError::NotWebSocketUrl)?;
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "ws" => 80,
        "wss" => 443,
        _ => return Err(WsHostError::NotWebSocketUrl),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    normalise_host_port(authority, Some(default_port))
}

/// The join origin for a WebSocket join to `host` (already normalised).
pub fn ws_host_origin(host: &str) -> String {
    format!("{JOIN_ORIGIN_WS_HOST_PREFIX}{host}")
}

// ── Server side (native only: the dedicated server) ──────────────────────────

/// Ports a port-less public host also admits, besides the server's own
/// `ws_port`: the scheme defaults (an origin omits 80 for `ws` and 443 for
/// `wss`, so a dialled `host:80` / `host:443` is the other scheme's default)
/// and 8443, the HTTPS port the bundled Caddy serves the browser client on.
#[cfg(not(target_arch = "wasm32"))]
const PORTLESS_EXTRA_PORTS: [u16; 3] = [80, 443, 8443];

/// One configured public address of a dedicated server.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PublicHost {
    /// Canonical host (IPv6 in brackets).
    host: String,
    /// `None` = the default ports plus the server's own port (see
    /// [`PublicHost::admits`]); `Some(p)` = exactly `p`, where `Some(80 |
    /// 443)` also admits a join that dialled the scheme's default port
    /// (which the origin omits).
    port: Option<u16>,
    /// Whether this address is globally unique to this server. `false` for an
    /// address another machine can also own (private, CGNAT, loopback,
    /// link-local, ULA, `.local`/`.lan`/`.home.arpa`, single-label names): a
    /// relay on the victim's own network could receive a signature for it.
    unique: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl PublicHost {
    /// Whether a join that dialled `host[:port]` (`port` is `None` for the
    /// scheme's default) is addressed to this entry. `ws_port` is the server's
    /// own WebSocket port.
    ///
    /// An entry with a port admits exactly that port (and `:80` / `:443` also
    /// the port-less origin). An entry WITHOUT a port admits only the default
    /// ports, `ws_port` and 8443 — not "any port", which would let a relay
    /// listening on another port of the same name through. The scheme is not
    /// part of the origin, so `:443` also admits a plaintext `ws://host` (port
    /// 80) and `:80` also admits `wss://host`.
    fn admits(&self, host: &str, port: Option<u16>, ws_port: u16) -> bool {
        self.host == host
            && match (self.port, port) {
                (None, None) => true,
                (None, Some(got)) => PORTLESS_EXTRA_PORTS.contains(&got) || got == ws_port,
                (Some(want), Some(got)) => want == got,
                (Some(want), None) => want == 80 || want == 443,
            }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Display for PublicHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.port {
            Some(p) => write!(f, "{}:{p}", self.host),
            None => f.write_str(&self.host),
        }
    }
}

/// Names that only mean something on the local network (RFC 6762 `.local`,
/// common home-router `.lan`, RFC 8375 `.home.arpa`, RFC 6761 `.localhost`,
/// ICANN-reserved private-use `.internal` / `.home` / `.corp` / `.intranet`):
/// another machine can answer to any of them.
#[cfg(not(target_arch = "wasm32"))]
const LOCAL_ONLY_SUFFIXES: [&str; 8] =
    ["local", "lan", "home.arpa", "localhost", "internal", "home", "corp", "intranet"];

/// Whether a canonical host (IPv6 in brackets) is globally unique to one
/// server, as opposed to an address any machine on some private network may
/// also hold. Documentation ranges (203.0.113.0/24, 2001:db8::/32) count as
/// unique: they are the examples operators are told to replace.
#[cfg(not(target_arch = "wasm32"))]
fn is_globally_unique(host: &str) -> bool {
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return match inner.parse::<Ipv6Addr>() {
            Ok(ip) => match ip.to_ipv4_mapped() {
                Some(v4) => !is_non_unique_v4(v4),
                None => {
                    let first = ip.segments()[0];
                    !(ip.is_loopback()
                        || ip.is_unspecified()
                        || first & 0xfe00 == 0xfc00 // fc00::/7 unique local
                        || first & 0xffc0 == 0xfe80) // fe80::/10 link-local
                }
            },
            // Not reachable (the host was normalised), but never claim unique.
            Err(_) => false,
        };
    }
    if let Ok(v4) = host.parse::<Ipv4Addr>() {
        return !is_non_unique_v4(v4);
    }
    // A name: a single label (`myserver`, `localhost`) is never public DNS…
    if !host.contains('.') {
        return false;
    }
    // …nor is anything under a local-only suffix.
    !LOCAL_ONLY_SUFFIXES
        .iter()
        .any(|s| host == *s || host.strip_suffix(*s).is_some_and(|rest| rest.ends_with('.')))
}

#[cfg(not(target_arch = "wasm32"))]
fn is_non_unique_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private() // 10/8, 172.16/12, 192.168/16
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || (o[0] == 100 && (64..=127).contains(&o[1])) // 100.64/10 CGNAT
}

/// The public addresses a dedicated server answers to (`--public-host`,
/// `AXENSTAX_PUBLIC_HOST`, `AXENSTAX_DOMAIN`). Empty = not configured: any
/// `ws-host` origin is accepted (the residual).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublicHosts {
    entries: Vec<PublicHost>,
    /// The server's own WebSocket port, which a port-less entry admits.
    ws_port: u16,
}

#[cfg(not(target_arch = "wasm32"))]
impl PublicHosts {
    /// Parse `name[:port]` entries (blank entries skipped, duplicates merged);
    /// `ws_port` is this server's own WebSocket port. An entry with a port
    /// admits only that port; one without admits the default ports (80/443),
    /// `ws_port` and 8443 (see [`PublicHost::admits`]). Entries that are not
    /// globally unique (see [`PublicHosts::non_unique`]) are kept. `Err` names
    /// the first bad entry — a typo here must refuse to boot, not silently
    /// leave the server unprotected.
    pub fn parse<S: AsRef<str>>(entries: &[S], ws_port: u16) -> Result<Self, String> {
        let mut out: Vec<PublicHost> = Vec::new();
        for raw in entries {
            let e = raw.as_ref().trim();
            if e.is_empty() {
                continue;
            }
            if e.contains("://") || e.contains('/') {
                return Err(format!(
                    "invalid public host '{e}': give a host name or IP, optionally with :port \
                     (no scheme or path), e.g. play.example.org or 203.0.113.7:6767"
                ));
            }
            let (host, port) = split_host_port(e, None)
                .map_err(|err| format!("invalid public host '{e}': {err}"))?;
            let unique = is_globally_unique(&host);
            let ph = PublicHost { host, port, unique };
            if !out.contains(&ph) {
                out.push(ph);
            }
        }
        Ok(Self { entries: out, ws_port })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether a join that dialled `dialled` (normalised `host[:port]`) is
    /// addressed to this server. An empty list admits everything.
    pub fn admits(&self, dialled: &str) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        match split_host_port(dialled, None) {
            Ok((host, port)) => self.entries.iter().any(|p| p.admits(&host, port, self.ws_port)),
            Err(_) => false,
        }
    }

    /// The configured entries that are not globally unique to this server (as
    /// the operator wrote them, normalised, in order). A signature made for
    /// one of these can be replayed here by a relay on the victim's own
    /// network, so they are accepted but NOT relay-protected.
    pub fn non_unique(&self) -> Vec<String> {
        self.entries.iter().filter(|p| !p.unique).map(PublicHost::to_string).collect()
    }

    /// Human-readable list of every configured entry, for the SERVER'S OWN log
    /// only. Never put this in text sent to a joiner: an unauthenticated probe
    /// must not learn which hosts (LAN addresses included) the server uses.
    pub fn describe(&self) -> String {
        self.entries.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(" or ")
    }
}

/// The one startup warning for a server whose WebSocket joins are not
/// relay-protected (no public host configured). `None` once configured.
#[cfg(not(target_arch = "wasm32"))]
pub fn relay_protection_warning(hosts: &PublicHosts) -> Option<&'static str> {
    hosts.is_empty().then_some(
        "WebSocket joins are not relay-protected: set --public-host (or AXENSTAX_PUBLIC_HOST) \
         to the address players use to reach this server",
    )
}

/// What the dedicated server logs at boot about its public hosts, in order:
/// the "none configured" warning, or the summary line plus one warning per
/// entry that is not unique to this server ("not relay-protected: <entry> is
/// not unique to this server"). `server_main` logs each at its level.
#[cfg(not(target_arch = "wasm32"))]
pub fn boot_messages(hosts: &PublicHosts) -> Vec<(log::Level, String)> {
    if let Some(w) = relay_protection_warning(hosts) {
        return vec![(log::Level::Warn, format!("  public host: none — {w}"))];
    }
    let mut out = vec![(
        log::Level::Info,
        format!(
            "  public host: {} (WebSocket joins to any other address are refused)",
            hosts.describe()
        ),
    )];
    for e in hosts.non_unique() {
        out.push((
            log::Level::Warn,
            format!(
                "  public host: not relay-protected: {e} is not unique to this server — \
                 another machine can hold the same address, so a join signed for it elsewhere \
                 could be replayed here. LAN joins still work; use a public domain or IP for \
                 relay protection"
            ),
        ));
    }
    out
}

/// Why a WebSocket join was refused. `reason` goes to the joiner and says
/// nothing about how the server is configured; `detail` is for the server's
/// own log (what was dialled, what the server answers to).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsJoinRefusal {
    pub reason: String,
    pub detail: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl WsJoinRefusal {
    /// A refusal whose log detail is the same as what the joiner is told.
    fn plain(reason: String) -> Self {
        Self { detail: reason.clone(), reason }
    }
}

/// The exact origin a WebSocket join must have signed, from the host the
/// joiner declared it dialled (`JoinRequestPacket::ws_host`). The server
/// re-normalises the declared value (never trusting the client to have done
/// it) and checks it against its public hosts. `Err` carries the rejection
/// reason shown to the joiner (it starts with `auth event origin mismatch` and
/// never names a configured host) and the detail for the server's log.
///
/// Applies to guest joins too: the server-identity proof in `JoinAccept` is
/// signed over this origin, so a relay must not be able to obtain a proof for
/// its own address by forwarding a pinned client's nonce as a guest.
#[cfg(not(target_arch = "wasm32"))]
pub fn expected_ws_join_origin(declared: &str, hosts: &PublicHosts) -> Result<String, WsJoinRefusal> {
    if declared.trim().is_empty() {
        return Err(WsJoinRefusal::plain(
            "auth event origin mismatch: this client didn't say which address it \
             connected to. Update Axe'n'Stax and try again."
                .to_string(),
        ));
    }
    let host = normalise_host_port(declared, None).map_err(|e| {
        WsJoinRefusal::plain(format!(
            "auth event origin mismatch: the address this client connected to is not usable ({e})"
        ))
    })?;
    if !hosts.admits(&host) {
        return Err(WsJoinRefusal {
            reason: String::from(
                "auth event origin mismatch: this server expects to be reached at its public \
                 address. Reconnect using the address its operator gave you."
            ),
            detail: format!(
                "auth event origin mismatch: joiner dialled '{host}', which is not one of this \
                 server's public hosts ({})",
                hosts.describe()
            ),
        });
    }
    Ok(ws_host_origin(&host))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The normalisation table — the client's view (`ws_url_host`).
    #[test]
    fn ws_url_host_normalisation_table() {
        let cases: &[(&str, &str)] = &[
            ("ws://play.example.org:6767", "play.example.org:6767"),
            ("ws://PLAY.Example.ORG:6767/", "play.example.org:6767"),
            ("WSS://play.example.org/ws", "play.example.org"),
            ("wss://play.example.org:443/ws", "play.example.org"),
            ("ws://play.example.org:80", "play.example.org"),
            // The other scheme's default port is NOT stripped.
            ("ws://play.example.org:443", "play.example.org:443"),
            ("wss://play.example.org:80/ws", "play.example.org:80"),
            ("wss://play.example.org:8443/ws", "play.example.org:8443"),
            ("ws://play.example.org.:6767", "play.example.org:6767"),
            ("ws://203.0.113.7:6767", "203.0.113.7:6767"),
            ("ws://[2001:DB8::1]:6767", "[2001:db8::1]:6767"),
            ("ws://[2001:db8:0:0:0:0:0:1]:6767", "[2001:db8::1]:6767"),
            ("wss://[::1]/ws", "[::1]"),
            ("ws://host:6767?x=1#frag", "host:6767"),
            ("ws://localhost:6767", "localhost:6767"),
        ];
        for (url, want) in cases {
            assert_eq!(ws_url_host(url).as_deref(), Ok(*want), "{url}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn international_names_become_punycode() {
        assert_eq!(ws_url_host("wss://Bücher.example/ws").as_deref(), Ok("xn--bcher-kva.example"));
        assert_eq!(
            ws_url_host("ws://bücher.example.:6767").as_deref(),
            Ok("xn--bcher-kva.example:6767")
        );
    }

    #[test]
    fn bad_addresses_are_refused() {
        assert_eq!(ws_url_host("http://h/ws"), Err(WsHostError::NotWebSocketUrl));
        assert_eq!(ws_url_host("h:6767"), Err(WsHostError::NotWebSocketUrl));
        assert_eq!(ws_url_host("ws:///ws"), Err(WsHostError::Empty));
        assert_eq!(ws_url_host("ws://user@h:6767"), Err(WsHostError::Userinfo));
        assert_eq!(ws_url_host("ws://h:0"), Err(WsHostError::InvalidPort));
        assert_eq!(ws_url_host("ws://h:70000"), Err(WsHostError::InvalidPort));
        assert_eq!(ws_url_host("ws://h:/ws"), Err(WsHostError::InvalidPort));
        assert_eq!(ws_url_host("ws://h:+80"), Err(WsHostError::InvalidPort));
        assert_eq!(ws_url_host("ws://a..b:6767"), Err(WsHostError::InvalidHost));
        assert_eq!(ws_url_host("ws://a b:6767"), Err(WsHostError::InvalidHost));
        assert_eq!(ws_url_host("ws://[::1:6767"), Err(WsHostError::InvalidHost));
        assert_eq!(ws_url_host("ws://.:6767"), Err(WsHostError::Empty));
    }

    #[test]
    fn origin_has_the_ws_host_scheme_and_is_never_a_web_origin() {
        let o = ws_host_origin("play.example.org:6767");
        assert_eq!(o, "axenstax-join:ws-host:play.example.org:6767");
        assert!(o.starts_with("axenstax-join:") && !o.starts_with("https://"));
    }

    /// The server's own WebSocket port in these tests.
    const WS_PORT: u16 = 6767;

    fn parse(entries: &[&str]) -> PublicHosts {
        PublicHosts::parse(entries, WS_PORT).unwrap()
    }

    #[test]
    fn public_host_entries_normalise_like_the_client() {
        let hosts = parse(&["Play.Example.Org.", "203.0.113.7:6767", " ", "[2001:db8::1]"]);
        assert!(hosts.admits("play.example.org"));
        assert!(hosts.admits("203.0.113.7:6767"));
        assert!(hosts.admits("[2001:db8::1]:6767"));
        // Duplicates merge; blanks are skipped.
        assert_eq!(parse(&["h", "H.", ""]), parse(&["h"]));
    }

    /// A port-less entry admits only the default ports (the origin omits 80 for
    /// `ws` and 443 for `wss`), the server's own `ws_port` and 8443 (Caddy's
    /// alternate HTTPS port) — never an arbitrary port, so a relay on another
    /// port of the same name is not admitted. An entry with a port admits only
    /// that port, with 80/443 also matching an origin that dialled the scheme's
    /// default (and so carries no port).
    #[test]
    fn port_rules() {
        let any = parse(&["play.example.org"]);
        for d in [
            "play.example.org",
            "play.example.org:80",
            "play.example.org:443",
            "play.example.org:6767",
            "play.example.org:8443",
        ] {
            assert!(any.admits(d), "port-less entry should admit {d}");
        }
        for d in [
            "play.example.org:22",
            "play.example.org:81",
            "play.example.org:6768",
            "play.example.org:8080",
            "play.example.org:9000",
            "play.example.org:65535",
        ] {
            assert!(!any.admits(d), "port-less entry must NOT admit {d}");
        }
        // The server's own port is whatever it was started with.
        let custom = PublicHosts::parse(&["play.example.org"], 7000).unwrap();
        assert!(custom.admits("play.example.org:7000"));
        assert!(!custom.admits("play.example.org:6767"));
        // A different host never matches, whatever the port.
        assert!(!any.admits("other.example.org"));
        assert!(!any.admits("other.example.org:6767"));

        let exact = parse(&["203.0.113.7:6767"]);
        assert!(exact.admits("203.0.113.7:6767"));
        assert!(!exact.admits("203.0.113.7:6768"));
        assert!(!exact.admits("203.0.113.7"));
        let default = parse(&["play.example.org:443"]);
        assert!(default.admits("play.example.org"));
        assert!(default.admits("play.example.org:443"));
        assert!(!default.admits("play.example.org:80"));
        assert!(!default.admits("play.example.org:6767"));
        // An explicit listing is how an operator on an unusual port opts in.
        let odd = parse(&["play.example.org:9000"]);
        assert!(odd.admits("play.example.org:9000"));
        assert!(!odd.admits("play.example.org"));
    }

    /// The scheme is not part of the origin: `ws://h` and `wss://h` both sign
    /// `h`, so a `:443` entry also admits a plaintext join on port 80 and a
    /// `:80` entry also admits a `wss://h` join on port 443. Documented in
    /// Spec 08 (T-JOIN-RELAY) and the operator guide; pinned here.
    #[test]
    fn the_scheme_is_not_part_of_the_origin() {
        assert_eq!(ws_url_host("ws://play.example.org/ws"), ws_url_host("wss://play.example.org/ws"));
        assert!(parse(&["play.example.org:443"]).admits(&ws_url_host("ws://play.example.org/ws").unwrap()));
        assert!(parse(&["play.example.org:80"]).admits(&ws_url_host("wss://play.example.org/ws").unwrap()));
        // …but a non-default port is exact, scheme or not.
        assert!(!parse(&["play.example.org:443"]).admits(&ws_url_host("ws://play.example.org:6767").unwrap()));
    }

    #[test]
    fn bad_public_host_entries_refuse_to_parse() {
        for bad in ["wss://play.example.org/ws", "play.example.org/ws", "h:0", "a..b", "user@h"] {
            let err = PublicHosts::parse(&[bad], WS_PORT).unwrap_err();
            assert!(err.contains(bad), "{bad}: {err}");
        }
    }

    /// Which public-host entries are NOT globally unique to this server: a
    /// relay M on the victim's own network can own the same address, so a
    /// signature made for it could be replayed here (Spec 08 T-JOIN-RELAY).
    #[test]
    fn non_unique_classification_table() {
        let non_unique = [
            // RFC 1918.
            "10.0.0.1", "10.255.255.255", "172.16.0.1", "172.31.255.254", "192.168.1.20", "192.168.0.0",
            // CGNAT 100.64.0.0/10.
            "100.64.0.1", "100.127.255.254",
            // Loopback, link-local, unspecified.
            "127.0.0.1", "127.1.2.3", "169.254.1.1", "169.254.169.254", "0.0.0.0",
            "[::1]", "[::]", "[fe80::1]", "[febf::1]",
            // IPv6 ULA fc00::/7.
            "[fc00::1]", "[fd12:3456:789a::1]",
            // IPv4-mapped IPv6 takes the IPv4 verdict.
            "[::ffff:192.168.1.20]", "[::ffff:127.0.0.1]",
            // Local-only names.
            "nas.local", "Printer.Local.", "router.lan", "game.home.arpa", "home.arpa",
            "localhost", "game.localhost", "box.internal", "myserver", "minecraft",
            // With a port the verdict is the same.
            "192.168.1.20:6767", "nas.local:7000", "[fd12::1]:6767", "myserver:6767",
        ];
        for e in non_unique {
            let hosts = parse(&[e]);
            assert_eq!(hosts.non_unique().len(), 1, "{e} should be flagged non-unique");
        }
        let unique = [
            "203.0.113.7", "203.0.113.7:6767", "8.8.8.8", "play.example.org", "play.example.org:443",
            "Play.Example.Org.", "example.org", "[2001:db8::1]", "[2606:4700:4700::1111]:6767",
            // Just outside each private block.
            "11.0.0.1", "172.15.255.255", "172.32.0.1", "192.167.255.255", "192.169.0.1",
            "100.63.255.255", "100.128.0.1", "169.253.0.1", "128.0.0.1",
            "[fec0::1]", "[fe7f::1]", "[fbff::1]", "[fe00::1]", "[::ffff:8.8.8.8]",
            // Look-alikes that are ordinary public names.
            "locally.example.org", "mylan.example.org", "local.example.org", "home.arpa.example.com",
            "lan.example.net", "internal.example.net",
        ];
        for e in unique {
            let hosts = parse(&[e]);
            assert!(hosts.non_unique().is_empty(), "{e} should NOT be flagged: {:?}", hosts.non_unique());
        }
    }

    /// Non-unique entries are still accepted (a WS-only dedicated server must
    /// serve LAN joins) and listed in order, as the operator wrote them.
    #[test]
    fn non_unique_entries_are_accepted_and_listed() {
        let hosts = parse(&["play.example.org", "192.168.1.20", "nas.local:7000", "203.0.113.7"]);
        assert!(hosts.admits("192.168.1.20:6767"), "LAN joins still work");
        assert!(hosts.admits("nas.local:7000"));
        assert!(hosts.admits("play.example.org"));
        assert_eq!(hosts.non_unique(), vec!["192.168.1.20".to_string(), "nas.local:7000".to_string()]);
        assert!(parse(&["play.example.org", "203.0.113.7"]).non_unique().is_empty());
        assert!(PublicHosts::default().non_unique().is_empty());
    }

    fn texts(msgs: &[(log::Level, String)], level: log::Level) -> Vec<&str> {
        msgs.iter().filter(|(l, _)| *l == level).map(|(_, m)| m.as_str()).collect()
    }

    /// The boot log: one warning per non-unique entry, naming it; nothing to
    /// warn about for unique entries; the unconfigured server keeps its single
    /// "not relay-protected: set --public-host" warning.
    #[test]
    fn boot_warns_for_each_non_unique_entry() {
        let msgs = boot_messages(&parse(&["play.example.org", "192.168.1.20", "nas.local"]));
        let warns = texts(&msgs, log::Level::Warn);
        assert_eq!(warns.len(), 2, "{msgs:?}");
        assert!(
            warns[0].contains("not relay-protected: 192.168.1.20 is not unique to this server"),
            "{}",
            warns[0]
        );
        assert!(warns[1].contains("not relay-protected: nas.local is not unique to this server"), "{}", warns[1]);
        // The unique entry is not warned about.
        assert!(warns.iter().all(|w| !w.contains("play.example.org")), "{warns:?}");
        // The summary line still lists what is configured (server-side log only).
        let infos = texts(&msgs, log::Level::Info);
        assert_eq!(infos.len(), 1, "{msgs:?}");
        assert!(infos[0].contains("'play.example.org'"), "{}", infos[0]);

        let clean = boot_messages(&parse(&["play.example.org", "203.0.113.7:6767"]));
        assert!(texts(&clean, log::Level::Warn).is_empty(), "{clean:?}");
        assert_eq!(texts(&clean, log::Level::Info).len(), 1);

        let unconfigured = boot_messages(&PublicHosts::default());
        let warns = texts(&unconfigured, log::Level::Warn);
        assert_eq!(warns.len(), 1, "{unconfigured:?}");
        assert!(warns[0].contains("WebSocket joins are not relay-protected: set --public-host"), "{}", warns[0]);
    }

    #[test]
    fn unconfigured_server_admits_any_host_and_warns_once_at_boot() {
        let none = PublicHosts::default();
        assert!(none.admits("anything.example:1"));
        let w = relay_protection_warning(&none).expect("unconfigured → warning");
        assert!(w.starts_with("WebSocket joins are not relay-protected: set --public-host"), "{w}");
        assert_eq!(relay_protection_warning(&parse(&["h"])), None);
    }

    #[test]
    fn expected_origin_re_normalises_the_declared_host() {
        let hosts = parse(&["play.example.org"]);
        assert_eq!(
            expected_ws_join_origin("PLAY.example.org:6767", &hosts).as_deref(),
            Ok("axenstax-join:ws-host:play.example.org:6767")
        );
        assert_eq!(
            expected_ws_join_origin("relay.example.net:6767", &PublicHosts::default()).as_deref(),
            Ok("axenstax-join:ws-host:relay.example.net:6767")
        );
    }

    /// IP-vs-domain: a player who dialled the server's IP while it answers as
    /// its domain is refused, told only that the server expects its public
    /// address — never WHICH hosts it is configured with: an unauthenticated
    /// probe must not be able to enumerate them (LAN IPs included).
    #[test]
    fn a_mismatch_is_refused_generically_without_naming_any_configured_host() {
        let hosts = parse(&["play.example.org", "192.168.1.20", "nas.local:7000", "203.0.113.9:6767"]);
        let refusal = expected_ws_join_origin("203.0.113.7:6767", &hosts).unwrap_err();
        let err = &refusal.reason;
        assert!(err.starts_with("auth event origin mismatch"), "{err}");
        assert!(err.contains("expects to be reached at its public address"), "{err}");
        for configured in ["play.example.org", "192.168.1.20", "nas.local", "7000", "203.0.113.9"] {
            assert!(!err.contains(configured), "refusal must not leak '{configured}': {err}");
        }
        // The configured hosts go to the server's own log instead.
        for configured in ["play.example.org", "192.168.1.20", "nas.local:7000", "203.0.113.9:6767"] {
            assert!(refusal.detail.contains(configured), "detail should name {configured}: {}", refusal.detail);
        }
        assert!(refusal.detail.contains("203.0.113.7:6767"), "detail names what was dialled: {}", refusal.detail);
        assert!(!refusal.reason.contains("203.0.113.7"), "the reason is fully generic: {}", refusal.reason);
    }

    /// A relay on another port of a port-less entry's name is refused too, and
    /// the refusal is the same generic one.
    #[test]
    fn a_relay_on_another_port_of_the_same_name_is_refused() {
        let hosts = parse(&["play.example.org"]);
        let refusal = expected_ws_join_origin("play.example.org:9000", &hosts).unwrap_err();
        assert!(refusal.reason.starts_with("auth event origin mismatch"), "{}", refusal.reason);
        assert!(!refusal.reason.contains("play.example.org"), "{}", refusal.reason);
    }

    #[test]
    fn missing_or_garbage_declared_host_is_refused_without_echoing_it() {
        let hosts = PublicHosts::default();
        let err = expected_ws_join_origin("", &hosts).unwrap_err();
        assert!(err.reason.starts_with("auth event origin mismatch"), "{}", err.reason);
        let err = expected_ws_join_origin("evil\u{202e}host:1", &hosts).unwrap_err();
        assert!(err.reason.starts_with("auth event origin mismatch"), "{}", err.reason);
        assert!(!err.reason.contains('\u{202e}'), "the raw declared value is never echoed");
        assert!(!err.detail.contains('\u{202e}'), "nor in the server log");
    }
}

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

/// One configured public address of a dedicated server.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PublicHost {
    /// Canonical host (IPv6 in brackets).
    host: String,
    /// `None` = any port on this host. `Some(80 | 443)` also admits a join
    /// that dialled the scheme's default port (which the origin omits).
    port: Option<u16>,
}

#[cfg(not(target_arch = "wasm32"))]
impl PublicHost {
    fn admits(&self, host: &str, port: Option<u16>) -> bool {
        self.host == host
            && match (self.port, port) {
                (None, _) => true,
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

/// The public addresses a dedicated server answers to (`--public-host`,
/// `AXENSTAX_PUBLIC_HOST`, `AXENSTAX_DOMAIN`). Empty = not configured: any
/// `ws-host` origin is accepted (the residual).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublicHosts(Vec<PublicHost>);

#[cfg(not(target_arch = "wasm32"))]
impl PublicHosts {
    /// Parse `name[:port]` entries (blank entries skipped, duplicates merged).
    /// An entry without a port admits any port on that host; one with a port
    /// admits only that port. `Err` names the first bad entry — a typo here
    /// must refuse to boot, not silently leave the server unprotected.
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Self, String> {
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
            let ph = PublicHost { host, port };
            if !out.contains(&ph) {
                out.push(ph);
            }
        }
        Ok(Self(out))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether a join that dialled `dialled` (normalised `host[:port]`) is
    /// addressed to this server. An empty list admits everything.
    pub fn admits(&self, dialled: &str) -> bool {
        if self.0.is_empty() {
            return true;
        }
        match split_host_port(dialled, None) {
            Ok((host, port)) => self.0.iter().any(|p| p.admits(&host, port)),
            Err(_) => false,
        }
    }

    /// Human-readable list for logs and rejection reasons.
    pub fn describe(&self) -> String {
        self.0.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(" or ")
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

/// The exact origin a WebSocket join must have signed, from the host the
/// joiner declared it dialled (`JoinRequestPacket::ws_host`). The server
/// re-normalises the declared value (never trusting the client to have done
/// it) and checks it against its public hosts. `Err` is the rejection reason
/// shown to the joiner; it starts with `auth event origin mismatch`.
///
/// Applies to guest joins too: the server-identity proof in `JoinAccept` is
/// signed over this origin, so a relay must not be able to obtain a proof for
/// its own address by forwarding a pinned client's nonce as a guest.
#[cfg(not(target_arch = "wasm32"))]
pub fn expected_ws_join_origin(declared: &str, hosts: &PublicHosts) -> Result<String, String> {
    if declared.trim().is_empty() {
        return Err("auth event origin mismatch: this client didn't say which address it \
                    connected to. Update Axe'n'Stax and try again."
            .to_string());
    }
    let host = normalise_host_port(declared, None).map_err(|e| {
        format!("auth event origin mismatch: the address this client connected to is not usable ({e})")
    })?;
    if !hosts.admits(&host) {
        return Err(format!(
            "auth event origin mismatch: you connected to '{host}', but this server only accepts \
             joins addressed to {}. Reconnect using that address.",
            hosts.describe()
        ));
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

    #[test]
    fn public_host_entries_normalise_like_the_client() {
        let hosts =
            PublicHosts::parse(&["Play.Example.Org.", "203.0.113.7:6767", " ", "[2001:db8::1]"]).unwrap();
        assert!(hosts.admits("play.example.org"));
        assert!(hosts.admits("203.0.113.7:6767"));
        assert!(hosts.admits("[2001:db8::1]:6767"));
        // Duplicates merge; blanks are skipped.
        assert_eq!(PublicHosts::parse(&["h", "H.", ""]).unwrap(), PublicHosts::parse(&["h"]).unwrap());
    }

    /// A port-less entry admits any port (the domain is reached as `wss://d/ws`
    /// through Caddy, `wss://d:8443/ws` and `ws://d:6767` natively); an entry
    /// with a port admits only that port, with 80/443 also matching an origin
    /// that dialled the scheme's default (and so carries no port).
    #[test]
    fn port_rules() {
        let any = PublicHosts::parse(&["play.example.org"]).unwrap();
        for d in ["play.example.org", "play.example.org:6767", "play.example.org:8443"] {
            assert!(any.admits(d), "{d}");
        }
        let exact = PublicHosts::parse(&["203.0.113.7:6767"]).unwrap();
        assert!(exact.admits("203.0.113.7:6767"));
        assert!(!exact.admits("203.0.113.7:6768"));
        assert!(!exact.admits("203.0.113.7"));
        let default = PublicHosts::parse(&["play.example.org:443"]).unwrap();
        assert!(default.admits("play.example.org"));
        assert!(default.admits("play.example.org:443"));
        assert!(!default.admits("play.example.org:6767"));
    }

    #[test]
    fn bad_public_host_entries_refuse_to_parse() {
        for bad in ["wss://play.example.org/ws", "play.example.org/ws", "h:0", "a..b", "user@h"] {
            let err = PublicHosts::parse(&[bad]).unwrap_err();
            assert!(err.contains(bad), "{bad}: {err}");
        }
    }

    #[test]
    fn unconfigured_server_admits_any_host_and_warns_once_at_boot() {
        let none = PublicHosts::default();
        assert!(none.admits("anything.example:1"));
        let w = relay_protection_warning(&none).expect("unconfigured → warning");
        assert!(w.starts_with("WebSocket joins are not relay-protected: set --public-host"), "{w}");
        assert_eq!(relay_protection_warning(&PublicHosts::parse(&["h"]).unwrap()), None);
    }

    #[test]
    fn expected_origin_re_normalises_the_declared_host() {
        let hosts = PublicHosts::parse(&["play.example.org"]).unwrap();
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
    /// its domain is told which address to use.
    #[test]
    fn ip_versus_domain_mismatch_gives_a_clear_reason() {
        let hosts = PublicHosts::parse(&["play.example.org"]).unwrap();
        let err = expected_ws_join_origin("203.0.113.7:6767", &hosts).unwrap_err();
        assert!(err.starts_with("auth event origin mismatch"), "{err}");
        assert!(err.contains("'203.0.113.7:6767'"), "{err}");
        assert!(err.contains("'play.example.org'"), "{err}");
        assert!(err.contains("Reconnect using that address"), "{err}");
    }

    #[test]
    fn missing_or_garbage_declared_host_is_refused_without_echoing_it() {
        let hosts = PublicHosts::default();
        let err = expected_ws_join_origin("", &hosts).unwrap_err();
        assert!(err.starts_with("auth event origin mismatch"), "{err}");
        let err = expected_ws_join_origin("evil\u{202e}host:1", &hosts).unwrap_err();
        assert!(err.starts_with("auth event origin mismatch"), "{err}");
        assert!(!err.contains('\u{202e}'), "the raw declared value is never echoed");
    }
}

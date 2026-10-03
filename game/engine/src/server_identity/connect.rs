#![cfg(not(target_arch = "wasm32"))]
//! The connect-string: how a server address + operator npub are shared and
//! parsed. Forms (all coexist):
//!   * `axenstax://<host>:<port>#op=<npub>` — host pinned to an operator; the
//!     `#op=` fragment is ignored by URL parsers so a bare `ws://`/`wss://` URL
//!     still parses (and joins anonymously, preserving the zero-config path).
//!   * `axenstax://<npub>` — **npub-only** (Spec A): resolve the live address by
//!     the operator npub via relays. No host is baked in.

/// A parsed connect-string.
#[allow(dead_code)] // produced by parse_connect_string; consumed by the join path
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectInfo {
    /// A directly-dialable websocket endpoint, optionally pinned to an operator
    /// npub the server's identity proof must chain to.
    Direct {
        /// A websocket URL the client can dial (`ws://…` or `wss://…/ws`).
        url: String,
        /// The operator npub to verify the server against, if pinned.
        operator_npub: Option<String>,
    },
    /// Resolve the live address by the operator npub via relays (Spec A —
    /// the npub is the stable handle; the address is looked up + proof-verified).
    Resolve {
        /// The operator npub to resolve + verify against.
        operator_npub: String,
    },
}

/// Build the canonical shareable host-pinned connect-string.
pub fn build_connect_string(host: &str, port: u16, operator_npub: Option<&str>) -> String {
    match operator_npub {
        Some(np) => format!("axenstax://{host}:{port}#op={np}"),
        None => format!("axenstax://{host}:{port}"),
    }
}

/// Build the npub-only connect-string (resolved via relays at join).
#[allow(dead_code)] // consumed by the My Servers UI (Spec A task 10)
pub fn build_npub_connect_string(operator_npub: &str) -> String {
    format!("axenstax://{operator_npub}")
}

/// Parse a connect-string into a [`ConnectInfo`].
#[allow(dead_code)] // consumed by the client join path; unit-tested
pub fn parse_connect_string(s: &str) -> Result<ConnectInfo, String> {
    // npub-only form: `axenstax://npub1…` with no host separators ⇒ Resolve.
    if let Some(rest) = s.strip_prefix("axenstax://")
        && rest.starts_with("npub1")
            && !rest.contains(':')
            && !rest.contains('/')
            && !rest.contains('.')
            && !rest.contains('#')
        {
            return Ok(ConnectInfo::Resolve {
                operator_npub: rest.to_string(),
            });
        }
    // Otherwise a directly-dialable endpoint. Split the optional "#op=" fragment
    // first so plain ws/wss URLs still parse.
    let (base, operator_npub) = match s.split_once("#op=") {
        Some((b, np)) => (b, Some(np.to_string())),
        None => (s, None),
    };
    let url = if let Some(rest) = base.strip_prefix("axenstax://") {
        // Friendly scheme ⇒ the canonical browser-joinable wss endpoint.
        format!("wss://{rest}/ws")
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_string()
    } else {
        return Err(format!("unrecognised connect string: {base}"));
    };
    Ok(ConnectInfo::Direct {
        url,
        operator_npub,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_string_round_trips() {
        let s = build_connect_string("play.example.com", 8080, Some("npub1xyz"));
        assert_eq!(s, "axenstax://play.example.com:8080#op=npub1xyz");
        let info = parse_connect_string(&s).unwrap();
        assert_eq!(
            info,
            ConnectInfo::Direct {
                url: "wss://play.example.com:8080/ws".into(),
                operator_npub: Some("npub1xyz".into()),
            }
        );
    }

    #[test]
    fn axenstax_without_op_has_no_operator() {
        let info = parse_connect_string("axenstax://play.example.com:8080").unwrap();
        assert_eq!(
            info,
            ConnectInfo::Direct {
                url: "wss://play.example.com:8080/ws".into(),
                operator_npub: None,
            }
        );
    }

    #[test]
    fn bare_ws_passes_through_with_no_operator() {
        let info = parse_connect_string("ws://10.0.0.5:8080").unwrap();
        assert_eq!(
            info,
            ConnectInfo::Direct {
                url: "ws://10.0.0.5:8080".into(),
                operator_npub: None,
            }
        );
    }

    #[test]
    fn bare_wss_with_pinned_operator() {
        let info = parse_connect_string("wss://play.example.com/ws#op=npub1abc").unwrap();
        assert_eq!(
            info,
            ConnectInfo::Direct {
                url: "wss://play.example.com/ws".into(),
                operator_npub: Some("npub1abc".into()),
            }
        );
    }

    #[test]
    fn npub_only_resolves() {
        let info = parse_connect_string("axenstax://npub1xyzabc").unwrap();
        assert_eq!(
            info,
            ConnectInfo::Resolve {
                operator_npub: "npub1xyzabc".into(),
            }
        );
    }

    #[test]
    fn npub_only_round_trips_through_builder() {
        let s = build_npub_connect_string("npub1aaa");
        assert_eq!(s, "axenstax://npub1aaa");
        assert_eq!(
            parse_connect_string(&s).unwrap(),
            ConnectInfo::Resolve {
                operator_npub: "npub1aaa".into()
            }
        );
    }

    #[test]
    fn host_starting_npub1_is_still_direct() {
        // A host that begins "npub1" but carries separators is NOT an npub.
        let info = parse_connect_string("axenstax://npub1host.example:8080").unwrap();
        assert!(matches!(info, ConnectInfo::Direct { .. }));
    }

    #[test]
    fn unknown_scheme_is_rejected() {
        assert!(parse_connect_string("http://nope").is_err());
    }
}

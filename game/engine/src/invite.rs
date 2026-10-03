//! The `axenstax://invite/2` link a host mints for one world, and its QR form.
//!
//! Pure: no I/O, no clock of its own — `parse` is handed `now` so expiry is
//! testable. Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`
//! §3.1.
//!
//! Native-gated even though nothing here needs a native API: the web build is
//! the anonymous local taster (CLAUDE.md red lines 1 and 4), so the whole
//! online-play surface stays out of its bundle. `tools/smoke/forbidden-symbol.mjs`
//! greps the wasm bundle for `axenstax://invite/` to prove it.
#![cfg(not(target_arch = "wasm32"))]

pub const INVITE_VERSION: u32 = 2;
/// A minted invite is good for 48 hours. Minting a fresh one retires the old
/// bearer, so this is a floor on nuisance, not a security boundary.
pub const DEFAULT_INVITE_TTL_SECS: u64 = 48 * 3600;
/// More than this many relays in one link is a sign of a mangled paste, not a
/// well-connected host.
pub const MAX_RELAYS: usize = 8;
pub const BEARER_LEN: usize = 16;

const SCHEME: &str = "axenstax://invite/";

/// One host's standing invitation to one world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    /// The host's Signet persona, as an npub — never hex
    /// ([[feedback_npub_only_display]]).
    pub host_persona: String,
    /// The host's runtime pubkey (x-only). Hex on the wire; this is the key the
    /// joiner NIP-44-seals its offer to.
    pub host_runtime: [u8; 32],
    /// Relays to publish the offer on. Every entry must be `wss://`.
    pub relays: Vec<String>,
    /// The 16-byte bearer that admits one persona, once.
    pub bearer: [u8; BEARER_LEN],
    pub expires_at: u64,
    pub world_name: String,
}

/// Every way a pasted link can be wrong. Distinct variants so the UI can say
/// something specific ("that invite has expired") instead of "bad link".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteError {
    BadScheme(String),
    BadVersion(String),
    MissingField(&'static str),
    BadField(&'static str),
    Expired { expires_at: u64, now: u64 },
    RelayNotWss(String),
    TooManyRelays(usize),
}

impl std::fmt::Display for InviteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InviteError::BadScheme(s) => write!(f, "that isn't an Axe'n'Stax invite link ({s})"),
            InviteError::BadVersion(v) => {
                write!(f, "that invite was made by a different version ({v})")
            }
            InviteError::MissingField(k) => write!(f, "the invite link is missing '{k}'"),
            InviteError::BadField(k) => write!(f, "the invite link's '{k}' is malformed"),
            InviteError::Expired { .. } => {
                write!(f, "that invite has expired — ask for a fresh one")
            }
            InviteError::RelayNotWss(r) => write!(f, "relay '{r}' isn't a wss:// address"),
            InviteError::TooManyRelays(n) => write!(f, "that invite lists too many relays ({n})"),
        }
    }
}

/// Percent-encode everything outside the RFC 3986 unreserved set. Hand-rolled
/// because the tree carries no URL crate and this is a dozen lines.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Inverse of [`pct_encode`]. A malformed escape yields `None` rather than
/// silently dropping bytes.
fn pct_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl Invite {
    /// Render the link. The same string is what `menu::draw_qr` renders as a QR.
    pub fn to_link(&self) -> String {
        let mut s = format!(
            "{SCHEME}{INVITE_VERSION}?h={}&k={}",
            pct_encode(&self.host_persona),
            hex::encode(self.host_runtime),
        );
        for r in &self.relays {
            s.push_str("&r=");
            s.push_str(&pct_encode(r));
        }
        s.push_str(&format!(
            "&b={}&x={}&w={}",
            hex::encode(self.bearer),
            self.expires_at,
            pct_encode(&self.world_name),
        ));
        s
    }

    /// Parse a pasted link. `now` is unix seconds — injected so expiry is a
    /// pure decision the tests can pin.
    pub fn parse(s: &str, now: u64) -> Result<Invite, InviteError> {
        let s = s.trim();
        let rest = s
            .strip_prefix(SCHEME)
            .ok_or_else(|| InviteError::BadScheme(s.to_string()))?;
        let (version, query) = rest
            .split_once('?')
            .ok_or_else(|| InviteError::BadScheme(s.to_string()))?;
        if version != INVITE_VERSION.to_string() {
            return Err(InviteError::BadVersion(version.to_string()));
        }

        let mut host_persona: Option<String> = None;
        let mut runtime_hex: Option<String> = None;
        let mut relays: Vec<String> = Vec::new();
        let mut bearer_hex: Option<String> = None;
        let mut expires_at: Option<u64> = None;
        let mut world_name = String::new();

        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            match k {
                "h" => host_persona = pct_decode(v),
                "k" => runtime_hex = Some(v.to_string()),
                "r" => relays.push(pct_decode(v).ok_or(InviteError::BadField("r"))?),
                "b" => bearer_hex = Some(v.to_string()),
                "x" => expires_at = v.parse::<u64>().ok(),
                "w" => world_name = pct_decode(v).ok_or(InviteError::BadField("w"))?,
                // Unknown parameters are ignored, so a future version can add
                // one without breaking this parser.
                _ => {}
            }
        }

        let host_persona = host_persona.ok_or(InviteError::MissingField("h"))?;
        let runtime_hex = runtime_hex.ok_or(InviteError::MissingField("k"))?;
        let bearer_hex = bearer_hex.ok_or(InviteError::MissingField("b"))?;
        let expires_at = expires_at.ok_or(InviteError::MissingField("x"))?;

        if relays.len() > MAX_RELAYS {
            return Err(InviteError::TooManyRelays(relays.len()));
        }
        if let Some(bad) = relays.iter().find(|r| !r.starts_with("wss://")) {
            return Err(InviteError::RelayNotWss(bad.clone()));
        }

        let host_runtime: [u8; 32] = hex::decode(&runtime_hex)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or(InviteError::BadField("k"))?;
        let bearer: [u8; BEARER_LEN] = hex::decode(&bearer_hex)
            .ok()
            .and_then(|b| <[u8; BEARER_LEN]>::try_from(b.as_slice()).ok())
            .ok_or(InviteError::BadField("b"))?;

        if now > expires_at {
            return Err(InviteError::Expired { expires_at, now });
        }

        Ok(Invite {
            host_persona,
            host_runtime,
            relays,
            bearer,
            expires_at,
            world_name,
        })
    }
}

/// A fresh 16-byte bearer from the OS RNG. Panics only if the OS RNG is
/// unavailable, which is not a condition the game can meaningfully continue in.
pub fn mint_bearer() -> [u8; BEARER_LEN] {
    let mut b = [0u8; BEARER_LEN];
    getrandom::fill(&mut b).expect("OS RNG unavailable");
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Invite {
        Invite {
            host_persona: "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m"
                .to_string(),
            host_runtime: [0xab; 32],
            relays: vec!["wss://nos.lol".to_string(), "wss://relay.damus.io".to_string()],
            bearer: [7u8; 16],
            expires_at: 2_000_000_000,
            world_name: "Ivy's Hollow & Co".to_string(),
        }
    }

    #[test]
    fn round_trips_through_the_link() {
        let inv = sample();
        let link = inv.to_link();
        assert!(link.starts_with("axenstax://invite/2?"), "got {link}");
        let back = Invite::parse(&link, 1_000_000_000).unwrap();
        assert_eq!(back, inv);
    }

    #[test]
    fn world_name_is_percent_encoded_and_decoded() {
        let inv = sample();
        let link = inv.to_link();
        assert!(link.contains("w=Ivy%27s%20Hollow%20%26%20Co"), "got {link}");
        assert_eq!(
            Invite::parse(&link, 1_000_000_000).unwrap().world_name,
            "Ivy's Hollow & Co"
        );
    }

    #[test]
    fn rejects_a_foreign_scheme() {
        assert!(matches!(
            Invite::parse("https://example.com/?h=x", 0),
            Err(InviteError::BadScheme(_))
        ));
    }

    #[test]
    fn rejects_a_different_version() {
        let link = sample().to_link().replace("invite/2?", "invite/9?");
        assert!(matches!(Invite::parse(&link, 0), Err(InviteError::BadVersion(_))));
    }

    #[test]
    fn rejects_each_missing_field() {
        // Drop one required parameter at a time and assert the exact field name.
        for (param, field) in [("h=", "h"), ("k=", "k"), ("b=", "b"), ("x=", "x")] {
            let link = sample().to_link();
            let stripped: String = link
                .split('&')
                .filter(|seg| !seg.contains(param) || seg.starts_with("axenstax"))
                .collect::<Vec<_>>()
                .join("&");
            // The first segment carries the scheme; strip it there too when needed.
            let stripped = stripped
                .split('?')
                .map(|part| {
                    part.split('&')
                        .filter(|seg| !seg.starts_with(param))
                        .collect::<Vec<_>>()
                        .join("&")
                })
                .collect::<Vec<_>>()
                .join("?");
            assert_eq!(
                Invite::parse(&stripped, 1_000_000_000),
                Err(InviteError::MissingField(field)),
                "stripping {param} should name {field}: {stripped}"
            );
        }
    }

    #[test]
    fn rejects_an_expired_invite() {
        let inv = sample();
        assert_eq!(
            Invite::parse(&inv.to_link(), inv.expires_at + 1),
            Err(InviteError::Expired { expires_at: inv.expires_at, now: inv.expires_at + 1 })
        );
        // Exactly at the expiry second is still valid.
        assert!(Invite::parse(&inv.to_link(), inv.expires_at).is_ok());
    }

    #[test]
    fn rejects_a_relay_that_is_not_wss() {
        let link = sample().to_link().replace("wss%3A%2F%2Fnos.lol", "ws%3A%2F%2Fnos.lol");
        assert!(matches!(
            Invite::parse(&link, 1_000_000_000),
            Err(InviteError::RelayNotWss(_))
        ));
    }

    #[test]
    fn rejects_more_than_eight_relays() {
        let mut inv = sample();
        inv.relays = (0..9).map(|i| format!("wss://r{i}.example")).collect();
        assert_eq!(
            Invite::parse(&inv.to_link(), 1_000_000_000),
            Err(InviteError::TooManyRelays(9))
        );
    }

    #[test]
    fn rejects_a_malformed_runtime_key() {
        let link = sample().to_link().replace(&"ab".repeat(32), "notlongenough");
        assert_eq!(Invite::parse(&link, 1_000_000_000), Err(InviteError::BadField("k")));
    }

    #[test]
    fn mint_bearer_is_not_all_zeroes_and_differs_between_calls() {
        let a = mint_bearer();
        let b = mint_bearer();
        assert_ne!(a, [0u8; BEARER_LEN], "a zero bearer would admit by accident");
        assert_ne!(a, b, "each invite gets a fresh bearer");
    }
}
